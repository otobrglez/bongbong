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
    round_under(extra, players, Mission::Destroy)
}

/// `round_with` under `mission`.
fn round_under(extra: &str, players: usize, mission: Mission) -> Game {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.show_intro = false;
    game.level_overrides.mission = Some(mission);
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
    assert_eq!(tank.fpv_drones, 2 + t.fpv_drones_per_pickup, "a second crate stacks on what is left");
    tank.take_weapon(ActiveWeapon::FpvSwarm);
    assert_eq!(tank.fpv_drones, t.fpv_drones_max, "up to the halo's limit");
    tank.take_weapon(ActiveWeapon::Minigun);
    assert_eq!((tank.fpv_drones, tank.special()), (0, Some(ActiveWeapon::Minigun)), "another weapon's crate replaces the halo");
    assert_eq!(ActiveWeapon::FpvSwarm.full_load(), t.fpv_drones_max);
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
    let (slot_point, _) = crate::fpv::halo_slot(SEAT, tank(&game, seat, |tk| tk.sprite_size()), 5, crate::fpv::Halo::of(&t));
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

/// A burst on the open ground leaves a hull under a tree's crown out
/// whole, well inside the blast's reach: the seat's drone, with the tank
/// under the tree not locked, dives on the aim point beside it.
#[test]
fn a_ground_burst_spares_a_hull_under_a_crown() {
    // The aim point six cells east of the seat, (9, 6); the tree two cells
    // north of it, the tank between, its hull under the crown.
    let mut game = round("cells.\"9,4\" = { kind = \"tree\" }\n");
    let hidden = parked(&mut game, Position::new(cell(9, 6).x + 12.0, cell(9, 4).y + 36.0));
    let at = tank(&game, hidden, |t| t.position);
    let events = launch(&mut game, 300);
    assert_eq!(launched(&events)[0].1, None, "under canopy: nothing to lock");
    let b = bursts(&events);
    assert_eq!(b.len(), 1);
    assert!(!b[0].1, "on the ground, not in the leaves");
    assert!(b[0].0.distance_to(at) < Tuning::DEFAULT.fpv_blast_radius_px * 0.8, "well inside the blast: {:?} vs {at:?}", b[0].0);
    assert_eq!(tank(&game, hidden, |t| t.damage), 0.0, "the hull under the crown is left out");
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

/// A board on the aim point is worn by the burst's full damage, like any
/// tile in reach, and never lit - a burst is no fire.
#[test]
fn a_ground_burst_wears_a_range_board_and_never_lights_it() {
    // The aim point six cells east of the seat, (9, 6), holds the board.
    let mut game = round("cells.\"9,6\" = { kind = \"target\" }\n");
    let board = |g: &Game| {
        g.world
            .query::<&Obstacle>()
            .iter()
            .find(|o| o.material == crate::obstacle::Material::Target)
            .map(|o| (o.health, o.burning, o.destroyed))
            .expect("the board stands")
    };
    let (before, _, _) = board(&game);
    let events = launch(&mut game, 300);
    let b = bursts(&events);
    assert_eq!(b.len(), 1);
    assert!(!b[0].1, "on the ground: a board is no crown");
    let (after, burning, destroyed) = board(&game);
    assert!((before - after - Tuning::DEFAULT.fpv_damage).abs() < 1e-3, "{before} -> {after}");
    assert!(!burning && !destroyed, "worn, not lit");
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
    assert!(game.strike_air(&mut f, AirKey::Drone(id), AirStrike::Emp));
    assert!(!game.strike_air(&mut f, AirKey::Drone(id), AirStrike::Emp), "a falling drone is no target");
    assert!(game.air_targets().is_empty());
    assert_eq!(drones(&game)[0].stage, DroneStage::Falling);
    let _ = DroneLock::None;
}


// --- the AI ---------------------------------------------------------------

/// A round as `round_with`, under `mission`, the seat unarmed (shells).
fn round_as(extra: &str, mission: Mission) -> Game {
    let game = round_under(extra, 1, mission);
    let seat = game.player().unwrap();
    with_tank_mut(&game.world, seat, |t| t.disarm());
    game
}

/// A thinking enemy of `role` at `at` carrying a crate's worth of drones,
/// held where it stands (`speed_scale` 0) so the test reads its decisions.
fn fpv_enemy(game: &mut Game, at: Position, role: Role, moves: bool) -> Entity {
    let slot = game.debug_spawn_enemy(at, Some(1), Some(role)).expect("spawns");
    let entity = game.tank_entity_by_slot(slot).expect("exists");
    with_tank_mut(&game.world, entity, |t| {
        if !moves {
            t.speed_scale = 0.0;
        }
        t.disarm();
        t.fpv_drones = Tuning::DEFAULT.fpv_drones_per_pickup;
        t.shield_hp = 0.0;
        t.shield_timer = 0.0;
    });
    entity
}

/// Run `frames` ticks with the seat idle; every event.
fn idle(game: &mut Game, frames: usize) -> Vec<Event> {
    let mut seen = Vec::new();
    for _ in 0..frames {
        seen.extend(step(game, false));
    }
    seen
}

fn launches_by(events: &[Event], slot: usize) -> Vec<(u32, Option<usize>, bool)> {
    events
        .iter()
        .filter_map(|e| match *e {
            Event::DroneLaunched { id, slot: s, target, frog, .. } if s == slot => Some((id, target, frog)),
            _ => None,
        })
        .collect()
}

/// An enemy with drones launches at a seat whose box it stands in, with a
/// wall between them - no line of sight needed.
#[test]
fn the_swarm_launches_at_a_seat_in_its_box_without_line_of_sight() {
    let wall = (4..9).map(|r| format!("cells.\"9,{r}\" = {{ kind = \"wall\", material = \"iron\" }}\n")).collect::<String>();
    let mut game = round_as(&wall, Mission::Destroy);
    let enemy = fpv_enemy(&mut game, cell(14, 6), Role::Player, false);
    let slot = slot_of(&game, enemy);
    let events = idle(&mut game, 300);
    let l = launches_by(&events, slot);
    assert!(!l.is_empty(), "it launched");
    assert_eq!(l[0].1, Some(0), "at the seat");
    assert!(tank(&game, game.player().unwrap(), |t| t.damage) > 0.0, "and it landed");
}

/// Outside the seat's box it launches nothing at it.
#[test]
fn an_enemy_never_launches_at_a_seat_from_outside_its_sight_box() {
    let mut game = round_as("", Mission::Destroy);
    // 12.5 cells east: past the box's 11.5.
    let enemy = fpv_enemy(&mut game, Position::new(SEAT.x + 400.0, SEAT.y), Role::Player, false);
    let events = idle(&mut game, 300);
    assert!(launches_by(&events, slot_of(&game, enemy)).is_empty());
}

/// One at a time: no second launch while its first drone is in the air,
/// none within `fpv_enemy_gap_seconds` of the last.
#[test]
fn the_swarm_launches_one_at_a_time_with_its_gap() {
    let mut game = round_as("", Mission::Destroy);
    let enemy = fpv_enemy(&mut game, cell(14, 6), Role::Player, false);
    let slot = slot_of(&game, enemy);
    let mut frames: Vec<u64> = Vec::new();
    let mut out_at_launch = Vec::new();
    for _ in 0..900 {
        let events = step(&mut game, false);
        if !launches_by(&events, slot).is_empty() {
            frames.push(game.frame());
            out_at_launch.push(game.drones().iter().filter(|d| d.owner == Owner::Enemy(slot) && d.in_air()).count());
        }
    }
    assert!(frames.len() >= 2, "{frames:?}");
    let gap = (Tuning::DEFAULT.fpv_enemy_gap_seconds / DT) as u64;
    assert!(frames.windows(2).all(|w| w[1] - w[0] >= gap), "{frames:?}");
    assert!(out_at_launch.iter().all(|&n| n == 1), "only the one just launched in the air: {out_at_launch:?}");
}

/// A hunter sends its drones at the players' frog.
#[test]
fn a_hunter_sends_its_drones_at_the_frog() {
    let mut game = round_as("cells.\"14,12\" = { kind = \"frog\" }\n", Mission::Protect);
    let seat = game.player().unwrap();
    game.place_tank(seat, cell(3, 1), Some(90.0)).unwrap();
    let enemy = fpv_enemy(&mut game, cell(20, 12), Role::Hunter, false);
    let events = idle(&mut game, 300);
    let l = launches_by(&events, slot_of(&game, enemy));
    assert!(l.iter().any(|&(_, _, frog)| frog), "{l:?}");
}

/// A seat hidden in tall grass is not launched at.
#[test]
fn the_swarm_never_launches_at_a_seat_hidden_in_grass() {
    let grass = (6..9).flat_map(|c| (8..11).map(move |r| format!("cells.\"{c},{r}\" = {{ kind = \"tall_grass\" }}\n"))).collect::<String>();
    let mut game = round_as(&grass, Mission::Destroy);
    let seat = game.player().unwrap();
    game.place_tank(seat, cell(7, 9), Some(90.0)).unwrap();
    let enemy = fpv_enemy(&mut game, cell(14, 6), Role::Player, false);
    let events = idle(&mut game, 300);
    assert!(launches_by(&events, slot_of(&game, enemy)).is_empty());
}

#[test]
fn a_training_dummy_never_launches() {
    let mut game = round_as("", Mission::Destroy);
    let enemy = fpv_enemy(&mut game, cell(14, 6), Role::Hunter, false);
    game.world.get::<&mut crate::ai::Ai>(enemy).unwrap().frog_only = true;
    let events = idle(&mut game, 300);
    assert!(launches_by(&events, slot_of(&game, enemy)).is_empty());
}

/// A training dummy with a minigun fires no flak at a seat's drone: it
/// never fires toward a seat, and the drone is the seat's.
#[test]
fn a_training_dummy_fires_no_flak() {
    let mut game = round("");
    let enemy = parked(&mut game, cell(14, 6));
    with_tank_mut(&game.world, enemy, |t| t.minigun_ammo = 60);
    game.world.get::<&mut crate::ai::Ai>(enemy).unwrap().frog_only = true;
    let slot = slot_of(&game, enemy);
    let events = launch(&mut game, 300);
    assert!(!events.iter().any(|e| matches!(e, Event::Fired { slot: s, weapon: "minigun" } if *s == slot)));
    assert!(downed(&events).is_empty());
}

/// The generic tiers never launch: lined up on the seat in range with no
/// sense, an enemy with drones does not pull its trigger.
#[test]
fn the_generic_tiers_never_launch_a_drone() {
    assert!(!crate::ai::generic_fire(ActiveWeapon::FpvSwarm));
}

/// A seat under a tree is launched at through the crown: the drone bursts
/// in the leaves over it.
#[test]
fn the_swarm_breaks_the_crown_over_a_hidden_seat() {
    let mut game = round_as("cells.\"3,5\" = { kind = \"tree\" }\n", Mission::Destroy);
    let seat = game.player().unwrap();
    game.place_tank(seat, Position::new(96.0, 186.0), Some(0.0)).unwrap();
    let enemy = fpv_enemy(&mut game, cell(14, 6), Role::Player, false);
    let events = idle(&mut game, 400);
    let l = launches_by(&events, slot_of(&game, enemy));
    assert!(!l.is_empty() && l[0].1.is_none(), "launched at the crown, no lock: {l:?}");
    assert!(bursts(&events).iter().any(|&(_, crown)| crown), "in the leaves");
    assert_eq!(tank(&game, seat, |t| t.damage), 0.0, "the seat under it untouched");
}

/// An enemy with a minigun shoots down the seat's drone diving at it.
#[test]
fn an_enemy_with_a_minigun_shoots_down_the_drone_diving_at_it() {
    let mut game = round("");
    let enemy = parked(&mut game, cell(14, 6));
    with_tank_mut(&game.world, enemy, |t| t.minigun_ammo = 60);
    let events = launch(&mut game, 300);
    assert_eq!(downed(&events), vec![AirStrike::Bullet.name()], "{:?}", downed(&events));
    assert_eq!(tank(&game, enemy, |t| t.damage), 0.0);
}

/// An enemy a seat's drone has locked breaks for the nearest tree and is
/// under its crown before the dive: it takes nothing.
#[test]
fn an_enemy_breaks_toward_the_nearest_tree() {
    let mut game = round("cells.\"13,3\" = { kind = \"tree\" }\n");
    let slot = game.debug_spawn_enemy(cell(13, 6), Some(1), Some(Role::Player)).unwrap();
    let enemy = game.tank_entity_by_slot(slot).unwrap();
    with_tank_mut(&game.world, enemy, |t| {
        t.disarm();
        t.shield_hp = 0.0;
        t.shield_timer = 0.0;
    });
    let events = launch(&mut game, 360);
    assert!(events.iter().any(|e| matches!(e, Event::DroneLockLost { why: "canopy", .. })), "it got under the tree");
    assert_eq!(tank(&game, enemy, |t| t.damage), 0.0);
}

/// An enemy inside a danger backs out of it before it answers a seat's
/// drone: here the second seat's armed EMP, the enemy beside it, the first
/// seat's drone locked on it. While it is in the danger the `air` tier
/// gives way to `dodge`.
#[test]
fn an_enemy_inside_a_danger_backs_out_before_it_answers_a_drone() {
    let mut game = round_with("", 2);
    let second = game.players()[1].expect("the second seat");
    with_tank_mut(&game.world, second, |t| t.emp_charges = tuning().emp_charges_per_pickup);
    let slot = game.debug_spawn_enemy(cell(6, 12), Some(1), Some(Role::Player)).expect("spawns");
    let enemy = game.tank_entity_by_slot(slot).expect("exists");
    with_tank_mut(&game.world, enemy, |t| {
        t.disarm();
        t.shells_ammo = 0;
        t.shield_hp = 0.0;
        t.shield_timer = 0.0;
    });
    let launched_at = launched(&step(&mut game, true));
    assert_eq!(launched_at[0].1, Some(slot), "the drone is locked on the enemy");
    let (mut dodged_a_drone, mut answered_inside) = (0, 0);
    for _ in 0..90 {
        step(&mut game, false);
        let ai = game.world.get::<&crate::ai::Ai>(enemy).expect("its brain");
        let snap = ai.snapshot();
        if ai.air_threat.is_some() && snap.dodging {
            dodged_a_drone += 1;
            if snap.air.is_some() {
                answered_inside += 1;
            }
        }
    }
    assert!(dodged_a_drone > 0, "the drone came at it while it stood in the danger");
    assert_eq!(answered_inside, 0, "inside the danger it backs out first");
}

/// With no tree and no minigun, it breaks across the drone's line.
#[test]
fn an_enemy_with_no_tree_breaks_across_the_drones_line() {
    let mut game = round("");
    let slot = game.debug_spawn_enemy(cell(14, 6), Some(1), Some(Role::Player)).unwrap();
    let enemy = game.tank_entity_by_slot(slot).unwrap();
    with_tank_mut(&game.world, enemy, |t| {
        t.disarm();
        t.shells_ammo = 0;
    });
    let start = tank(&game, enemy, |t| t.position);
    step(&mut game, true);
    let mut moved_across = false;
    for _ in 0..200 {
        step(&mut game, false);
        let now = tank(&game, enemy, |t| t.position);
        if (now.y - start.y).abs() > 8.0 {
            moved_across = true;
        }
        if game.drones().is_empty() {
            break;
        }
    }
    assert!(moved_across, "it drove across the drone's line");
}



/// An enemy's drone locked on the seat is in that seat's off-screen
/// picture (`indicators::Scene::drones`); the seat's own are not.
#[test]
fn an_enemy_drone_locked_on_the_seat_is_in_its_scene() {
    let mut game = round("");
    launch(&mut game, 1);
    enemy_drone(&mut game, cell(14, 6));
    let scene = crate::indicators::Scene::of(&game, 0);
    assert_eq!(scene.drones.len(), 1, "the enemy's, not the seat's own: {:?}", scene.drones);
    // A view of the seat's corner alone: the drone is off it, and has its
    // arrow.
    let r = crate::math::Rectangle::new(0.0, 0.0, 300.0, 300.0);
    let t = Tuning::DEFAULT;
    let view = crate::indicators::ViewFrame::new(r, 1.0, Vec2::zero(), r, &t);
    let ind = crate::indicators::Awareness::new().frame(&scene, &view, &t);
    assert!(ind.arrows.iter().any(|a| matches!(a.kind, crate::indicators::ArrowKind::Drone { .. })), "{:?}", ind.arrows);
}

/// Cover it cannot get to is no cover: an enemy in the open that makes for
/// cover and does not arrive launches from where it stands once
/// `fpv_ai_cover_seconds` have passed.
#[test]
fn an_enemy_that_cannot_reach_cover_launches_from_the_open() {
    let wall = (4..9).map(|r| format!("cells.\"9,{r}\" = {{ kind = \"wall\", material = \"iron\" }}\n")).collect::<String>();
    let mut game = round_as(&wall, Mission::Destroy);
    // In the open below the wall's end, held where it stands.
    let enemy = fpv_enemy(&mut game, cell(14, 12), Role::Player, false);
    let slot = slot_of(&game, enemy);
    let mut covered = 0;
    let mut launched = None;
    for frame in 0..600 {
        let events = step(&mut game, false);
        if game.world.get::<&crate::ai::Ai>(enemy).unwrap().snapshot().special == Some("to cover") {
            covered += 1;
        }
        if !launches_by(&events, slot).is_empty() {
            launched = Some(frame);
            break;
        }
    }
    assert!(covered > 0, "it made for cover first");
    let launched = launched.expect("and launched from the open");
    assert!(launched as f32 * DT >= Tuning::DEFAULT.fpv_ai_cover_seconds, "after the patience: {launched}");
}

/// The drawn world a client sweeps its own bullets against holds the air
/// targets: a seat's bullet stops at an enemy's drone, never at its own.
#[test]
fn a_seats_drawn_bullet_stops_at_an_enemy_drone_not_its_own() {
    let mut game = round("");
    enemy_drone(&mut game, cell(14, 6));
    launch(&mut game, 1);
    let world = game.present_world();
    let at = world.air_contact(Some(0), cell(10, 6), cell(18, 6), 2.0).expect("the enemy's drone");
    let drone = game.drones().into_iter().find(|d| d.owner != Owner::Player(0)).expect("the enemy's drone");
    let edge = drone.ground.x - Tuning::DEFAULT.fpv_hit_half_px - 2.0;
    assert!((at.x - edge).abs() < 0.5, "at its column's near side: {at:?} vs {edge}");
    let own = game.drones().into_iter().find(|d| d.owner == Owner::Player(0)).expect("the seat's drone");
    let near = own.ground;
    assert!(world.air_contact(Some(0), Position::new(near.x - 40.0, near.y), Position::new(near.x + 40.0, near.y), 2.0).is_none(), "its own");
}

/// A rainbow shield soaks a drone's burst as it soaks any blast.
#[test]
fn a_shield_soaks_the_burst() {
    let mut game = round("");
    let enemy = parked(&mut game, cell(13, 6));
    // A full shield: it recharges to its pool between hits, never past it.
    let full = Tuning::DEFAULT.shield_capacity;
    with_tank_mut(&game.world, enemy, |t| {
        t.shield_hp = full;
        t.shield_timer = 30.0;
    });
    let mut events = step(&mut game, true);
    while bursts(&events).is_empty() && game.frame() < 400 {
        events = step(&mut game, false);
    }
    assert_eq!(bursts(&events).len(), 1, "it burst");
    assert_eq!(tank(&game, enemy, |t| t.damage), 0.0, "the hull whole");
    assert!(tank(&game, enemy, |t| t.shield_hp) < full, "the shield paid");
}

/// An offline tesla (an EMP) arcs at no drone.
#[test]
fn an_offline_tesla_arcs_no_drone() {
    let mut game = round("cells.\"7,6\" = { kind = \"tesla\", side = \"enemy\" }\n");
    for tower in game.towers.values_mut() {
        tower.disabled = 60.0;
    }
    let events = launch(&mut game, 300);
    assert!(downed(&events).is_empty(), "{events:?}");
}

// --- the earlier weapons on drones ----------------------------------------

/// A drone of `owner`'s cruising at `at`, heading east along its row from
/// a standstill: its id.
fn cruising(game: &mut Game, owner: Owner, at: Position) -> u32 {
    let mut d = Drone::launch(at, Vec2::new(1.0, 0.0), 0, owner, DroneLock::None, Position::new(W - 16.0, at.y));
    d.stage = DroneStage::Cruise;
    d.height = Tuning::DEFAULT.fpv_cruise_height;
    d.speed = 0.0;
    d.id = game.take_shot_id();
    let id = d.id;
    game.world.spawn((d,));
    id
}

/// What downed which drone in `events`, by id.
fn downed_by(events: &[Event]) -> Vec<(u32, &'static str)> {
    events
        .iter()
        .filter_map(|e| match *e {
            Event::DroneDowned { id, by, .. } => Some((id, by)),
            _ => None,
        })
        .collect()
}

/// The EMP's ring is blind: every drone its front reaches falls, the
/// seat's own among them; one past its reach flies on.
#[test]
fn the_emp_ring_downs_every_drone_in_reach_whichever_side() {
    let mut game = round("");
    let seat = game.player().unwrap();
    with_tank_mut(&game.world, seat, |t| {
        t.disarm();
        t.emp_charges = Tuning::DEFAULT.emp_charges_per_pickup;
    });
    let own = cruising(&mut game, Owner::Player(0), Position::new(SEAT.x + 80.0, SEAT.y));
    let theirs = cruising(&mut game, Owner::Enemy(9), Position::new(SEAT.x, SEAT.y + 90.0));
    let far = cruising(&mut game, Owner::Enemy(9), Position::new(SEAT.x + 400.0, SEAT.y));
    let events = launch(&mut game, 60);
    let downed = downed_by(&events);
    assert!(downed.contains(&(own, "emp")) && downed.contains(&(theirs, "emp")), "{downed:?}");
    assert!(!downed.iter().any(|&(id, _)| id == far), "past the ring: {downed:?}");
}

/// The sonic hammer's wave knocks down the other side's drones in its cone
/// and leaves its own side's - and anything out of the cone - flying.
#[test]
fn the_sonic_wave_downs_only_the_other_sides_drones_in_its_cone() {
    let mut game = round("");
    let seat = game.player().unwrap();
    with_tank_mut(&game.world, seat, |t| {
        t.disarm();
        t.sonic_ammo = Tuning::DEFAULT.sonic_ammo_per_pickup;
    });
    let own = cruising(&mut game, Owner::Player(0), Position::new(SEAT.x + 100.0, SEAT.y));
    let theirs = cruising(&mut game, Owner::Enemy(9), Position::new(SEAT.x + 100.0, SEAT.y + 16.0));
    let behind = cruising(&mut game, Owner::Enemy(9), Position::new(SEAT.x - 80.0, SEAT.y + 160.0));
    let events = launch(&mut game, 60);
    let downed = downed_by(&events);
    assert!(downed.contains(&(theirs, "sonic")), "the other side's, in the cone: {downed:?}");
    assert!(!downed.iter().any(|&(id, _)| id == own), "its own side's rides it out: {downed:?}");
    assert!(!downed.iter().any(|&(id, _)| id == behind), "out of the cone: {downed:?}");
}

/// The gauss rail's slug pierces every drone in its lane, whoever's, and
/// keeps its damage past them: the second tank down the lane takes what
/// is left after one tank, as with no drone in the way.
#[test]
fn the_rail_slug_pierces_every_drone_in_its_lane_and_keeps_its_damage() {
    let mut game = round("");
    let seat = game.player().unwrap();
    with_tank_mut(&game.world, seat, |t| {
        t.disarm();
        t.gauss_slugs = Tuning::DEFAULT.gauss_slugs_per_pickup;
    });
    let a = parked(&mut game, Position::new(560.0, SEAT.y));
    let b = parked(&mut game, Position::new(720.0, SEAT.y));
    let full = crate::tank::ticks_of(Tuning::DEFAULT.gauss_charge_seconds);
    for _ in 0..full {
        step(&mut game, true);
    }
    // Into the lane only now, so they stand in it when the slug flies.
    let own = cruising(&mut game, Owner::Player(0), Position::new(SEAT.x + 160.0, SEAT.y));
    let theirs = cruising(&mut game, Owner::Enemy(9), Position::new(SEAT.x + 260.0, SEAT.y));
    let events = step(&mut game, false);
    let downed = downed_by(&events);
    assert!(downed.contains(&(own, "rail")) && downed.contains(&(theirs, "rail")), "{downed:?}");
    let drones: usize = events
        .iter()
        .map(|e| match e {
            Event::RailSlug { pierced, .. } => pierced.iter().filter(|p| p.2 == crate::gauss::Pierced::Drone).count(),
            _ => 0,
        })
        .sum();
    assert_eq!(drones, 2, "both in the slug's pierce list");
    let t = Tuning::DEFAULT;
    assert!(tank(&game, a, |t| t.damage) > 0.0, "the first tank is hit");
    let second = (t.gauss_damage * t.gauss_pierce_keep).min(100.0);
    let got = tank(&game, b, |t| t.damage);
    assert!((got - second).abs() < 0.5, "the second takes one tank's keep, none for the drones: {got} against {second}");
}

/// The other side's towers engage a seat's drone only from inside that
/// seat's sight box: a tesla and a gun tower past the box leave a drone of
/// the seat's flying beside them alone; with the seat moved up so they
/// stand in its box, both go for it.
#[test]
fn a_tower_engages_a_seats_drone_only_from_inside_the_seats_sight_box() {
    let towers = "cells.\"22,4\" = { kind = \"tesla\", side = \"enemy\" }\ncells.\"22,9\" = { kind = \"gun_tower\", side = \"enemy\" }\n";
    let downs = |seat_at: Position| {
        let mut game = round(towers);
        let seat = game.player().unwrap();
        game.place_tank(seat, seat_at, Some(0.0)).expect("placed");
        with_tank_mut(&game.world, seat, |t| t.disarm());
        // Beside the tesla, in the gun tower's reach, heading off east.
        let drone = cruising(&mut game, Owner::Player(0), Position::new(cell(22, 6).x, cell(22, 6).y));
        let mut seen = Vec::new();
        for _ in 0..90 {
            seen.extend(step(&mut game, false));
        }
        downed_by(&seen).into_iter().filter(|&(id, _)| id == drone).count()
    };
    let t = Tuning::DEFAULT;
    // 22 cells east of the seat's column: twelve and a half past its box.
    assert!(cell(22, 6).x - SEAT.x > t.sight_box_half_cols * crate::OBSTACLE_GRID_SIZE + 64.0);
    assert_eq!(downs(SEAT), 0, "from outside the seat's box neither tower engages");
    assert_eq!(downs(cell(14, 6)), 1, "with the towers in its box they bring it down");
}

/// An enemy turns its minigun on a seat's drone only from inside that
/// seat's sight box: the seat launches at it and backs off past the box's
/// edge, and the enemy holds its fire at the drone coming at it.
#[test]
fn an_enemy_holds_its_flak_outside_the_seats_sight_box() {
    let mut game = round("");
    let enemy = parked(&mut game, cell(13, 6));
    with_tank_mut(&game.world, enemy, |t| t.minigun_ammo = 60);
    let slot = slot_of(&game, enemy);
    let first = step(&mut game, true);
    assert_eq!(launched(&first)[0].1, Some(slot), "locked from inside the box");
    let seat = game.player().unwrap();
    game.place_tank(seat, cell(1, 6), Some(90.0)).expect("moved back");
    assert!(cell(13, 6).x - cell(1, 6).x > Tuning::DEFAULT.sight_box_half_cols * crate::OBSTACLE_GRID_SIZE, "now past its box");
    let mut seen = Vec::new();
    for _ in 0..240 {
        seen.extend(step(&mut game, false));
    }
    assert!(
        !seen.iter().any(|e| matches!(e, Event::Fired { slot: s, weapon: "minigun" } if *s == slot)),
        "no flak from outside the box"
    );
    assert!(downed(&seen).is_empty(), "{:?}", downed(&seen));
}

/// What an enemy's FPV rule says this tick (`AiSnapshot::special`).
fn special_why(game: &Game, entity: Entity) -> Option<&'static str> {
    game.world.get::<&crate::ai::Ai>(entity).unwrap().snapshot().special
}

/// A seat that sees it from inside `fpv_ai_min_range_px` is backed off
/// from before the enemy launches: drones do not need it close.
#[test]
fn an_enemy_too_close_backs_off_before_it_launches() {
    let mut game = round_as("", Mission::Destroy);
    let enemy = fpv_enemy(&mut game, Position::new(SEAT.x + 96.0, SEAT.y), Role::Player, true);
    let slot = slot_of(&game, enemy);
    let (mut backing, mut first) = (0, None);
    for frame in 0..600 {
        let events = step(&mut game, false);
        if special_why(&game, enemy) == Some("back off") {
            backing += 1;
        }
        if first.is_none() && !launches_by(&events, slot).is_empty() {
            first = Some((frame, tank(&game, enemy, |t| t.position).distance_to(SEAT)));
        }
    }
    assert!(backing > 0, "it backed off");
    let (frame, away) = first.expect("and launched");
    assert!(away >= Tuning::DEFAULT.fpv_ai_min_range_px * 0.9, "from out of the seat's face: {away} px at frame {frame}");
}

/// Exposed to the seat, an enemy with drones makes for the cover of a
/// wall nearby and launches from behind it.
#[test]
fn an_exposed_enemy_moves_behind_a_wall_and_launches_from_cover() {
    let wall = (5..9).map(|r| format!("cells.\"11,{r}\" = {{ kind = \"wall\", material = \"iron\" }}\n")).collect::<String>();
    let mut game = round_as(&wall, Mission::Destroy);
    // In the open south of the wall's end, seen by the seat.
    let enemy = fpv_enemy(&mut game, cell(13, 10), Role::Player, true);
    let slot = slot_of(&game, enemy);
    let (mut covering, mut launched_from) = (0, None);
    for _ in 0..600 {
        let events = step(&mut game, false);
        if special_why(&game, enemy) == Some("to cover") {
            covering += 1;
        }
        if launched_from.is_none() && !launches_by(&events, slot).is_empty() {
            launched_from = Some((special_why(&game, enemy), tank(&game, enemy, |t| t.position)));
        }
    }
    assert!(covering > 0, "it made for cover");
    let (why, at) = launched_from.expect("and launched");
    assert_eq!(why, Some("cover"), "from behind the wall, at {at:?}");
    assert!(!game.present_world().line_of_sight(SEAT, at), "the seat cannot see where it launched from");
}

/// With one of its drones in the air an enemy stands where it launched
/// from and watches it work.
#[test]
fn an_enemy_watches_its_drone_work() {
    let mut game = round_as("", Mission::Destroy);
    let enemy = fpv_enemy(&mut game, cell(14, 6), Role::Player, true);
    let slot = slot_of(&game, enemy);
    let mut launched = false;
    let (mut watching, mut moved) = (0, 0.0f32);
    for _ in 0..600 {
        let before = tank(&game, enemy, |t| t.position);
        let events = step(&mut game, false);
        launched |= !launches_by(&events, slot).is_empty();
        let up = game.drones().iter().any(|d| d.owner == Owner::Enemy(slot) && d.in_air());
        if launched && up && special_why(&game, enemy) == Some("watch") {
            watching += 1;
            moved = moved.max(tank(&game, enemy, |t| t.position).distance_to(before));
        }
    }
    assert!(watching > 30, "it watched its drone: {watching} ticks");
    assert!(moved < 1.0, "standing where it launched from: {moved} px in a tick");
}
