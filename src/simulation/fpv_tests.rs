//! Headless scenario tests for the FPV swarm (docs/fpv-swarm.md "Tests"):
//! one per promised rule, on the default 34 x 17 field with the seat at
//! cell (3, 6) facing east and enemies placed by hand. Tests cannot touch
//! the global tuning table, so every number is read off the defaults.

use super::*;
use crate::ai::Role;
use crate::air::{AirKey, AirStrike};
use crate::fpv::{Drone, DroneLock, DroneStage};
use crate::tuning::Tuning;

const W: f32 = 1088.0;
const H: f32 = 544.0;
const DT: f32 = 1.0 / 60.0;

/// The seat's pivot: cell (3, 6).
const SEAT: Position = Position::new(96.0, 192.0);

/// A cell's centre (`map::cell_to_world`: centres on multiples of 32).
fn cell(col: i32, row: i32) -> Position {
    Position::new(col as f32 * 32.0, row as f32 * 32.0)
}

/// A round on an open field plus `extra` cells, `players` seats (the second
/// at cell (3, 12)), nobody shielded, no banner, the first seat at `SEAT`
/// facing east with a crate's worth of drones.
fn round_with(extra: &str, players: usize) -> Game {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.show_intro = false;
    game.level_overrides.mission = Some(Mission::Destroy);
    game.players = PlayerCount::from_count(players).expect("a seat count");
    let map = format!("version = 1\ntanks = 0\ncells.\"3,6\" = {{ kind = \"start\" }}\ncells.\"3,12\" = {{ kind = \"start2\" }}\n{extra}");
    game.map = MapFile::from_toml_str(&map).expect("test map parses");
    game.init(W, H);
    for tank in game.world.query_mut::<&mut Tank>() {
        tank.shield_hp = 0.0;
        tank.shield_timer = 0.0;
    }
    let seat = game.player().expect("a seat");
    game.place_tank(seat, SEAT, Some(90.0)).expect("placed");
    with_tank_mut(&game.world, seat, |t| {
        t.disarm();
        t.fpv_drones = tuning().fpv_drones_per_pickup;
    });
    game
}

fn round(extra: &str) -> Game {
    round_with(extra, 1)
}

/// A parked enemy at `at`: it cannot drive or shoot.
fn parked(game: &mut Game, at: Position) -> Entity {
    let slot = game.debug_spawn_enemy(at, Some(1), Some(Role::Player)).expect("spawns");
    let entity = game.tank_entity_by_slot(slot).expect("exists");
    with_tank_mut(&game.world, entity, |t| {
        t.speed_scale = 0.0;
        t.shells_ammo = 0;
        t.disarm();
        t.shield_hp = 0.0;
        t.shield_timer = 0.0;
    });
    entity
}

/// The seat's trigger for one frame; the frame's events.
fn step(game: &mut Game, fire: bool) -> Vec<Event> {
    let mut input = Input::default();
    input.seats[0].fire = fire;
    game.update(input, DT, W, H);
    game.events().to_vec()
}

/// Press the trigger for one frame, then `frames` idle ones; every event.
fn launch(game: &mut Game, frames: usize) -> Vec<Event> {
    let mut seen = step(game, true);
    for _ in 0..frames {
        seen.extend(step(game, false));
    }
    seen
}

fn tank<R>(game: &Game, entity: Entity, f: impl FnOnce(&Tank) -> R) -> R {
    with_tank(&game.world, entity, f)
}

fn slot_of(game: &Game, entity: Entity) -> usize {
    tank(game, entity, |t| t.owner_slot())
}

fn drones(game: &Game) -> Vec<Drone> {
    game.drones()
}

fn launched(events: &[Event]) -> Vec<(u32, Option<usize>)> {
    events
        .iter()
        .filter_map(|e| match *e {
            Event::DroneLaunched { id, target, .. } => Some((id, target)),
            _ => None,
        })
        .collect()
}

fn bursts(events: &[Event]) -> Vec<(Position, bool)> {
    events
        .iter()
        .filter_map(|e| match *e {
            Event::DroneBurst { x, y, crown, .. } => Some((Position::new(x, y), crown)),
            _ => None,
        })
        .collect()
}

fn downed(events: &[Event]) -> Vec<&'static str> {
    events
        .iter()
        .filter_map(|e| match *e {
            Event::DroneDowned { by, .. } => Some(by),
            _ => None,
        })
        .collect()
}

#[test]
fn an_fpv_crate_arms_the_swarm_and_replaces_the_special_carried() {
    let t = Tuning::DEFAULT;
    let mut tank = Tank::default();
    tank.take_weapon(ActiveWeapon::Laser);
    tank.take_weapon(ActiveWeapon::FpvSwarm);
    assert_eq!((tank.special(), tank.laser_charges, tank.fpv_drones), (Some(ActiveWeapon::FpvSwarm), 0, t.fpv_drones_per_pickup));
    tank.fpv_drones = 2;
    tank.take_weapon(ActiveWeapon::FpvSwarm);
    assert_eq!(tank.fpv_drones, t.fpv_drones_per_pickup, "a second crate refills to a crate's worth");
    tank.take_weapon(ActiveWeapon::Minigun);
    assert_eq!((tank.fpv_drones, tank.special()), (0, Some(ActiveWeapon::Minigun)), "another weapon's crate replaces the halo");
    assert_eq!(ActiveWeapon::FpvSwarm.full_load(), t.fpv_drones_per_pickup);
    assert_eq!(ActiveWeapon::FpvSwarm.trigger(), crate::tank::Trigger::Press);
    assert_eq!(crate::pickup::PickupKind::FpvSwarm.weapon(), Some(ActiveWeapon::FpvSwarm));
}

/// One drone per press: `Fired` then `DroneLaunched` in one tick, the top
/// slot's, one drone spent; none while the trigger is held; the next after
/// the reload.
#[test]
fn a_press_launches_one_drone_and_spends_one() {
    let mut game = round("");
    let seat = game.player().unwrap();
    let t = Tuning::DEFAULT;
    let events = step(&mut game, true);
    let fired = events.iter().position(|e| matches!(e, Event::Fired { slot: 0, weapon: "fpv_swarm" })).expect("fired");
    let up = events.iter().position(|e| matches!(e, Event::DroneLaunched { slot: 0, .. })).expect("launched");
    assert!(fired < up, "Fired first, in the same tick");
    assert_eq!(tank(&game, seat, |tk| tk.fpv_drones), t.fpv_drones_per_pickup - 1);
    let d = drones(&game);
    assert_eq!(d.len(), 1);
    let (slot_point, _) = crate::fpv::halo_slot(SEAT, tank(&game, seat, |tk| tk.sprite_size()), 5, 6);
    assert!(d[0].origin.distance_to(slot_point) < 1.0, "the top slot's drone: {:?} vs {:?}", d[0].origin, slot_point);
    // Held: nothing more.
    let mut seen = Vec::new();
    for _ in 0..60 {
        seen.extend(step(&mut game, true));
    }
    assert!(launched(&seen).is_empty(), "a held trigger launches nothing more");
    // A new press after the reload launches the next.
    step(&mut game, false);
    let events = step(&mut game, true);
    assert_eq!(launched(&events).len(), 1);
    assert_eq!(tank(&game, seat, |tk| tk.fpv_drones), t.fpv_drones_per_pickup - 2);
}

/// The lock: the nearest enemy inside the seat's sight box; one nearer but
/// outside the box is not.
#[test]
fn the_drone_locks_the_nearest_enemy_in_the_seats_sight_box() {
    let mut game = round("");
    let near = parked(&mut game, cell(12, 6));
    let _far = parked(&mut game, cell(16, 9));
    let events = step(&mut game, true);
    assert_eq!(launched(&events)[0].1, Some(slot_of(&game, near)));
    // The box is 11.5 cells either side: an enemy 14 cells off is not
    // locked, even with nothing else there.
    let mut game = round("");
    let _off = parked(&mut game, cell(18, 6));
    let events = step(&mut game, true);
    assert_eq!(launched(&events)[0].1, None, "outside the box: the aim point");
}

#[test]
fn with_no_enemy_in_the_box_it_dives_on_the_aim_point() {
    let mut game = round("");
    let events = launch(&mut game, 300);
    let b = bursts(&events);
    assert_eq!(b.len(), 1, "{events:?}");
    let aim = Position::new(SEAT.x + Tuning::DEFAULT.fpv_aim_px, SEAT.y);
    assert!(b[0].0.distance_to(aim) < 1.0, "{:?} vs {aim:?}", b[0].0);
}

/// Over a wall onto its target: the enemy takes the burst's damage, the
/// wall nothing.
#[test]
fn the_drone_flies_over_walls_and_dives_on_its_target() {
    let wall = "cells.\"8,5\" = { kind = \"wall\", material = \"brick\" }\ncells.\"8,6\" = { kind = \"wall\", material = \"brick\" }\ncells.\"8,7\" = { kind = \"wall\", material = \"brick\" }\n";
    let mut game = round(wall);
    let enemy = parked(&mut game, cell(14, 6));
    let walls = |g: &Game| -> Vec<String> { g.world.query::<&Obstacle>().iter().map(|o| format!("{:?}", o.health)).collect() };
    let before = walls(&game);
    let events = launch(&mut game, 300);
    assert_eq!(bursts(&events).len(), 1);
    let damage = tank(&game, enemy, |t| t.damage);
    let full = Tuning::DEFAULT.fpv_damage;
    assert!(damage > full * 0.9 && damage <= full + 1e-3, "about the burst's damage: {damage}");
    assert_eq!(before, walls(&game), "flew over the wall");
}

/// The side opposing the launcher is hurt; a teammate in the burst is
/// shoved and whole.
#[test]
fn only_the_opposing_side_is_hurt() {
    let mut game = round_with("", 2);
    let mate = game.players()[1].unwrap();
    let enemy = parked(&mut game, cell(12, 6));
    game.place_tank(mate, cell(12, 8), Some(0.0)).unwrap();
    launch(&mut game, 300);
    assert!(tank(&game, enemy, |t| t.damage) > 0.0);
    assert_eq!(tank(&game, mate, |t| t.damage), 0.0, "a teammate takes nothing");
}

/// A tank whose hull touches a tree's crown is never locked.
#[test]
fn a_tank_under_a_tree_is_never_locked() {
    let mut game = round("cells.\"12,5\" = { kind = \"tree\" }\n");
    let hidden = parked(&mut game, cell(12, 8));
    let slot = slot_of(&game, hidden);
    game.debug_teleport(slot, Position::new(cell(12, 6).x, cell(12, 6).y + 6.0), Some(0.0)).expect("moved");
    let events = step(&mut game, true);
    assert_eq!(launched(&events)[0].1, None, "under canopy: nothing to lock");
}

/// A lock is lost the tick its target goes under a tree's crown; the drone
/// dives where it last saw it, and the tank under the tree takes nothing.
#[test]
fn a_drone_loses_its_lock_when_its_target_goes_under_a_tree() {
    let mut game = round("cells.\"14,4\" = { kind = \"tree\" }\n");
    let enemy = parked(&mut game, cell(14, 7));
    step(&mut game, true);
    for _ in 0..10 {
        step(&mut game, false);
    }
    let slot = slot_of(&game, enemy);
    game.debug_teleport(slot, Position::new(cell(14, 5).x, cell(14, 5).y + 14.0), Some(0.0)).expect("moved");
    let events = launch(&mut game, 300);
    assert!(events.iter().any(|e| matches!(e, Event::DroneLockLost { why: "canopy", .. })), "{events:?}");
    assert_eq!(tank(&game, enemy, |t| t.damage), 0.0, "the leaves took it");
}

/// A dive whose point is in a crown bursts in the leaves: the tree takes
/// `fpv_tree_damage`, nothing else anything.
#[test]
fn a_dive_into_a_crown_hurts_only_the_tree() {
    // The aim point six cells east of the seat falls on the tree's cell.
    let mut game = round("cells.\"9,6\" = { kind = \"tree\" }\n");
    let tree_health = |g: &Game| g.world.query::<&Obstacle>().iter().find(|o| o.material.is_tree()).map(|o| o.health);
    let before = tree_health(&game).unwrap();
    let events = launch(&mut game, 300);
    let b = bursts(&events);
    assert_eq!(b.len(), 1);
    assert!(b[0].1, "in the crown");
    let after = tree_health(&game).unwrap_or(0.0);
    assert!((before - after - Tuning::DEFAULT.fpv_tree_damage).abs() < 1e-3, "{before} -> {after}");
}

/// An enemy's drone put straight into the air at `at`, cruising, locked on
/// the first seat.
fn enemy_drone(game: &mut Game, at: Position) -> u32 {
    let seat = game.player().unwrap();
    let enemy = parked(game, Position::new(W - 48.0, at.y));
    let slot = slot_of(game, enemy);
    let mut d = Drone::launch(at, Vec2::new(-1.0, 0.0), 0, Owner::Enemy(slot), DroneLock::Tank { entity: seat, slot: 0 }, SEAT);
    d.stage = DroneStage::Cruise;
    d.height = Tuning::DEFAULT.fpv_cruise_height;
    d.speed = 60.0;
    d.id = game.take_shot_id();
    let id = d.id;
    game.world.spawn((d,));
    id
}

/// The seat's minigun brings an enemy's drone down: `DroneDowned` by a
/// bullet, the drone falling and crashing a dud, nobody hurt.
#[test]
fn a_minigun_bullet_brings_an_opposing_drone_down() {
    let mut game = round("");
    let seat = game.player().unwrap();
    with_tank_mut(&game.world, seat, |t| {
        t.disarm();
        t.minigun_ammo = 60;
    });
    enemy_drone(&mut game, Position::new(SEAT.x + 260.0, SEAT.y));
    let mut seen = Vec::new();
    for _ in 0..240 {
        seen.extend(step(&mut game, true));
    }
    assert_eq!(downed(&seen), vec![AirStrike::Bullet.name()], "{:?}", downed(&seen));
    assert!(seen.iter().any(|e| matches!(e, Event::DroneCrashed { .. })));
    assert!(bursts(&seen).is_empty(), "a downed drone does not burst");
    assert_eq!(tank(&game, seat, |t| t.damage), 0.0);
}

/// Shells pass under a drone.
#[test]
fn shells_pass_under_a_drone() {
    let mut game = round("");
    let seat = game.player().unwrap();
    with_tank_mut(&game.world, seat, |t| {
        t.disarm();
        t.shells_ammo = 20;
    });
    enemy_drone(&mut game, Position::new(SEAT.x + 260.0, SEAT.y));
    let mut seen = Vec::new();
    for i in 0..120 {
        seen.extend(step(&mut game, i % 20 == 0));
    }
    assert!(seen.iter().any(|e| matches!(e, Event::Fired { weapon: "shell", .. })));
    assert!(downed(&seen).is_empty(), "{:?}", downed(&seen));
}

/// A tesla coil arcs a drone in its reach on its own clock, with no charge.
#[test]
fn a_tesla_arcs_a_drone_in_reach_without_charging() {
    // An enemy tesla two cells past the aim point's line.
    let mut game = round("cells.\"7,6\" = { kind = \"tesla\", side = \"enemy\" }\n");
    let events = launch(&mut game, 300);
    assert_eq!(downed(&events), vec![AirStrike::Tesla.name()], "{events:?}");
    assert!(events.iter().any(|e| matches!(e, Event::TeslaStrike { chained: false, .. })));
}

/// A gun tower turns on a drone in reach and brings it down: the seat is
/// out of its reach, the drone flies through it to the enemy beside it.
#[test]
fn a_gun_tower_brings_a_drone_down() {
    let mut game = round("cells.\"14,2\" = { kind = \"gun_tower\", side = \"enemy\" }\n");
    let enemy = parked(&mut game, cell(13, 6));
    let events = launch(&mut game, 300);
    assert_eq!(downed(&events), vec![AirStrike::Bullet.name()], "{events:?}");
    assert_eq!(tank(&game, enemy, |t| t.damage), 0.0, "it never reached its target");
}

/// A seat's own side's towers leave its drones alone.
#[test]
fn a_seats_own_towers_leave_its_drones_alone() {
    let mut game = round("cells.\"7,6\" = { kind = \"tesla\" }\ncells.\"10,9\" = { kind = \"gun_tower\" }\n");
    let events = launch(&mut game, 300);
    assert!(downed(&events).is_empty(), "{events:?}");
    assert_eq!(bursts(&events).len(), 1);
}

/// A disabled tank launches nothing and keeps its drones.
#[test]
fn a_disabled_tank_launches_nothing_and_keeps_its_drones() {
    let mut game = round("");
    let seat = game.player().unwrap();
    with_tank_mut(&game.world, seat, |t| {
        t.disable(3.0);
    });
    let events = launch(&mut game, 10);
    assert!(launched(&events).is_empty());
    assert_eq!(tank(&game, seat, |t| t.fpv_drones), Tuning::DEFAULT.fpv_drones_per_pickup);
}

/// The drones in the air fly out on the end screen and hurt nobody.
#[test]
fn the_drones_fly_out_on_the_end_screen_and_hurt_nobody() {
    let mut game = round("");
    let enemy = parked(&mut game, cell(12, 6));
    step(&mut game, true);
    game.outcome = Outcome::Won;
    game.restart_timer = 100.0;
    let mut seen = Vec::new();
    for _ in 0..300 {
        seen.extend(step(&mut game, false));
    }
    assert_eq!(bursts(&seen).len(), 1, "it still comes down");
    assert_eq!(tank(&game, enemy, |t| t.damage), 0.0, "and hurts nobody");
}

/// The swarm draws nothing from the round's RNG: a launch, a flight and a
/// burst on open ground leave its state where it was.
#[test]
fn the_swarm_draws_no_rng() {
    let run = |fire: bool| {
        let mut game = round("");
        parked(&mut game, cell(12, 6));
        step(&mut game, fire);
        for _ in 0..300 {
            step(&mut game, false);
        }
        let mut rng = game.rng.take().unwrap();
        rand::RngExt::random::<u64>(&mut rng)
    };
    assert_eq!(run(true), run(false));
}

#[test]
fn a_round_with_drones_replays_bit_for_bit() {
    let run = || {
        let mut game = round("cells.\"7,6\" = { kind = \"tesla\", side = \"enemy\" }\n");
        parked(&mut game, cell(12, 6));
        parked(&mut game, cell(14, 9));
        let mut events = Vec::new();
        for i in 0..400 {
            events.extend(step(&mut game, i % 30 == 0));
        }
        let tanks: Vec<(usize, f32, f32, f32)> = game.tank_snapshots().iter().map(|t| (t.slot, t.position.x, t.position.y, t.damage)).collect();
        (format!("{events:?}"), format!("{tanks:?}"))
    };
    assert_eq!(run(), run());
}

/// What `strike_air` takes: a drone downed once, a key no drone has nothing.
#[test]
fn strike_air_downs_a_drone_once() {
    let mut game = round("");
    step(&mut game, true);
    let id = drones(&game)[0].id;
    assert!(game.air_targets().iter().any(|a| a.key == AirKey::Drone(id)));
    let mut f = Frame::new(DT, W, H, game.rng.take().unwrap(), Terrain::build(&game.world, W, H, &game.grass_cells, &game.water));
    assert!(game.strike_air(&mut f, AirKey::Drone(id), AirStrike::Emp, SEAT));
    assert!(!game.strike_air(&mut f, AirKey::Drone(id), AirStrike::Emp, SEAT), "a falling drone is no target");
    assert!(game.air_targets().is_empty());
    assert_eq!(drones(&game)[0].stage, DroneStage::Falling);
    let _ = DroneLock::None;
}

