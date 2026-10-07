//! Headless scenario tests for the gauss rail (docs/gauss-rail.md
//! "Tests"): one per promised rule, on the default 34 x 17 field with the
//! seat at cell (3, 6) facing east and parked enemies placed by hand. Tests
//! cannot touch the global tuning table, so every number is read off the
//! defaults.

use super::*;
use crate::ai::Role;
use crate::gauss::Pierced;
use crate::tank::{Charge, ChargeEnd};
use crate::tuning::Tuning;

const W: f32 = 1088.0;
const H: f32 = 544.0;
const DT: f32 = 1.0 / 60.0;

/// The seat's pivot: cell (3, 6).
const SEAT: Position = Position::new(96.0, 192.0);

/// A round on an open field plus `extra` cells, `players` seats (the second
/// at cell (3, 12)), nobody shielded, no banner, the seat facing east with
/// the rail.
fn round_with(extra: &str, players: usize) -> Game {
    round_as(extra, players, Mission::Destroy, None)
}

/// `round_with`, on `mission`, the seat in chassis `row` (its body's mass
/// is set at spawn).
fn round_as(extra: &str, players: usize, mission: Mission, row: Option<i32>) -> Game {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.show_intro = false;
    game.player_row_override = row;
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
        t.gauss_slugs = tuning().gauss_slugs_per_pickup;
    });
    game
}

fn round(extra: &str) -> Game {
    round_with(extra, 1)
}

/// A parked enemy of chassis `row` at `at`: it cannot drive or shoot.
fn parked_row(game: &mut Game, at: Position, row: i32) -> Entity {
    let slot = game.debug_spawn_enemy(at, Some(row), Some(Role::Player)).expect("spawns");
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

fn parked(game: &mut Game, at: Position) -> Entity {
    parked_row(game, at, 1)
}

fn step_with(game: &mut Game, intent: Intent) -> Vec<Event> {
    let mut input = Input::default();
    input.seats[0] = intent;
    game.update(input, DT, W, H);
    game.events().to_vec()
}

fn step(game: &mut Game, fire: bool) -> Vec<Event> {
    step_with(game, Intent { fire, ..Intent::default() })
}

/// Hold the trigger `ticks` ticks (the press tick included), then let go:
/// every event.
fn charge(game: &mut Game, ticks: u32) -> Vec<Event> {
    let mut seen = Vec::new();
    for _ in 0..ticks {
        seen.extend(step(game, true));
    }
    seen.extend(step(game, false));
    seen
}

fn full_ticks() -> u32 {
    crate::tank::ticks_of(tuning().gauss_charge_seconds)
}

fn seat(game: &Game) -> Entity {
    game.player().expect("a seat")
}

fn slugs(game: &Game) -> i32 {
    with_tank(&game.world, seat(game), |t| t.gauss_slugs)
}

fn damage(game: &Game, entity: Entity) -> f32 {
    with_tank(&game.world, entity, |t| t.damage)
}

fn rail_slugs(events: &[Event]) -> Vec<&Event> {
    events.iter().filter(|e| matches!(e, Event::RailSlug { .. })).collect()
}

/// A crate arms the rail, replaces another special, and a second refills
/// to a crate's worth; an enemy takes one only on shells.
#[test]
fn a_gauss_crate_arms_the_rail_and_replaces_the_special_carried() {
    let t = tuning();
    let mut tank = Tank::default();
    tank.take_weapon(ActiveWeapon::Minigun);
    tank.take_weapon(ActiveWeapon::GaussRail);
    assert_eq!((tank.minigun_ammo, tank.gauss_slugs), (0, t.gauss_slugs_per_pickup));
    assert_eq!(tank.active_weapon(), ActiveWeapon::GaussRail);
    tank.gauss_slugs = 1;
    tank.take_weapon(ActiveWeapon::GaussRail);
    assert_eq!(tank.gauss_slugs, t.gauss_slugs_per_pickup);
    let mut enemy = Tank { owner: Owner::Enemy(3), ..Tank::default() };
    assert!(enemy.wants_pickup(PickupKind::GaussRail));
    enemy.take_weapon(ActiveWeapon::Laser);
    assert!(!enemy.wants_pickup(PickupKind::GaussRail));
}

/// Held, the rail charges; let go at full, it fires one slug: the start on
/// the press, `Fired` and `RailSlug` on the release tick, one slug spent.
#[test]
fn the_rail_charges_while_held_and_fires_on_release_at_full() {
    let mut game = round("");
    let first = step(&mut game, true);
    assert!(first.iter().any(|e| matches!(e, Event::ChargeStarted { slot: 0, weapon: "gauss_rail" })), "{first:?}");
    for _ in 1..full_ticks() {
        let ev = step(&mut game, true);
        assert!(!ev.iter().any(|e| matches!(e, Event::Fired { .. })), "nothing fires while held");
    }
    let released = step(&mut game, false);
    let fired = released.iter().position(|e| matches!(e, Event::Fired { slot: 0, weapon: "gauss_rail" }));
    let slug = released.iter().position(|e| matches!(e, Event::RailSlug { slot: 0, leg: 0, .. }));
    assert!(fired.is_some() && slug.is_some() && fired < slug, "Fired then RailSlug: {released:?}");
    assert_eq!(slugs(&game), tuning().gauss_slugs_per_pickup - 1);
    assert!(with_tank(&game.world, seat(&game), |t| t.charge.is_none()));
}

/// The threshold is a tick: one short of full fizzles, full fires.
#[test]
fn a_release_one_tick_short_of_full_fizzles_and_at_full_fires() {
    let mut game = round("");
    let short = charge(&mut game, full_ticks() - 1);
    assert!(short.iter().any(|e| matches!(e, Event::ChargeEnded { end: ChargeEnd::Fizzled, .. })), "{short:?}");
    assert!(rail_slugs(&short).is_empty());
    assert_eq!(slugs(&game), tuning().gauss_slugs_per_pickup, "a fizzle spends nothing");
    let full = charge(&mut game, full_ticks());
    assert_eq!(rail_slugs(&full).len(), 1);
}

/// Held past its hold, it vents: no slug, nothing spent, and the rail is
/// hot - a press within the vent's cooldown starts nothing, one after it
/// does; a trigger still held after the vent needs a new press.
#[test]
fn a_charge_held_past_its_hold_vents_and_the_rail_cools() {
    let t = tuning();
    let mut game = round("");
    let vent = crate::tank::ticks_of(t.gauss_charge_seconds + t.gauss_hold_seconds);
    let mut seen = Vec::new();
    for _ in 0..vent + 30 {
        seen.extend(step(&mut game, true));
    }
    assert!(seen.iter().any(|e| matches!(e, Event::ChargeEnded { end: ChargeEnd::Vented, .. })));
    assert!(!seen.iter().any(|e| matches!(e, Event::Fired { .. } | Event::RailSlug { .. })), "it never fires by itself");
    assert_eq!(slugs(&game), t.gauss_slugs_per_pickup);
    assert!(with_tank(&game.world, seat(&game), |tk| tk.charge.is_none()), "held after a vent: no new charge");
    step(&mut game, false);
    let early = step(&mut game, true);
    assert!(!early.iter().any(|e| matches!(e, Event::ChargeStarted { .. })), "still hot");
    step(&mut game, false);
    for _ in 0..crate::tank::ticks_of(t.gauss_vent_cooldown_seconds) {
        step(&mut game, false);
    }
    let late = step(&mut game, true);
    assert!(late.iter().any(|e| matches!(e, Event::ChargeStarted { .. })), "cooled");
}

/// A charging hull crawls at `gauss_crawl_pace` of its top speed, and the
/// throttle says so.
#[test]
fn a_charging_hull_crawls() {
    let mut game = round("");
    let drive = |game: &mut Game, fire: bool| {
        let from = with_tank(&game.world, seat(game), |t| t.position);
        for _ in 0..90 {
            step_with(game, Intent { move_dir: Some(Dir::Down), fire, ..Intent::default() });
        }
        with_tank(&game.world, seat(game), |t| t.position).distance_to(from)
    };
    let free = drive(&mut game, false);
    let mut game = round("");
    let crawl = drive(&mut game, true);
    assert!(crawl < free * (tuning().gauss_crawl_pace + 0.1), "crawl {crawl} against {free}");
    assert!((with_tank(&game.world, seat(&game), |t| t.throttle) - tuning().gauss_crawl_pace).abs() < 1e-4);
}

/// The slug goes through brick, wood, glass and every tank in the lane, in
/// order along it, the damage falling by the keep factors, the tiles gone.
#[test]
fn the_slug_goes_through_brick_wood_glass_and_every_tank_in_order() {
    let mut game = round("cells.\"6,6\" = { kind = \"wall\", material = \"brick\" }\ncells.\"9,6\" = { kind = \"wall\", material = \"wood\" }\ncells.\"12,6\" = { kind = \"wall\", material = \"glass\" }\n");
    let a = parked(&mut game, Position::new(480.0, 192.0));
    let b = parked(&mut game, Position::new(640.0, 192.0));
    let c = parked(&mut game, Position::new(800.0, 192.0));
    let events = charge(&mut game, full_ticks());
    let hits: Vec<(f32, HitTarget)> = events
        .iter()
        .filter_map(|e| match *e {
            Event::Hit { target, x, cause: HitCause::Rail, .. } => Some((x, target)),
            _ => None,
        })
        .collect();
    assert!(hits.windows(2).all(|w| w[0].0 <= w[1].0), "in order along the lane: {hits:?}");
    let tiles = hits.iter().filter(|(_, t)| matches!(t, HitTarget::Obstacle { .. })).count();
    assert!(tiles >= 3, "every tile: {hits:?}");
    let t = Tuning::DEFAULT;
    let tile = t.gauss_tile_keep.powi(3);
    assert!(damage(&game, a) >= 100.0, "the first wrecked");
    let second = t.gauss_damage * tile * t.gauss_pierce_keep;
    assert!(damage(&game, b) >= second.min(100.0) - 0.5, "the second takes what is left: {}", damage(&game, b));
    let third = second * t.gauss_pierce_keep;
    assert!((damage(&game, c) - third.min(100.0)).abs() < 0.5, "the third: {} against {third}", damage(&game, c));
    let gone = events.iter().filter(|e| matches!(e, Event::ObstacleDestroyed { .. })).count();
    assert!(gone >= 2, "brick and glass gone, the wood broken or burning");
}

/// Iron stops the slug: the enemy behind it is untouched and the leg ends
/// at its face.
#[test]
fn iron_stops_the_slug() {
    let mut game = round("cells.\"10,6\" = { kind = \"wall\", material = \"iron\" }\n");
    let behind = parked(&mut game, Position::new(600.0, 192.0));
    let events = charge(&mut game, full_ticks());
    assert_eq!(damage(&game, behind), 0.0);
    let end = rail_slugs(&events).iter().find_map(|e| match **e {
        Event::RailSlug { x1, .. } => Some(x1),
        _ => None,
    });
    // The iron's box is its hull's, round its cell's centre (10 x 32),
    // grown by the slug's half width.
    assert!(end.is_some_and(|x| x < 10.0 * 32.0 - 8.0 && x > 10.0 * 32.0 - 24.0), "at the iron's west face: {end:?}");
}

/// The slug crosses the field to its edge on open ground.
#[test]
fn the_slug_crosses_the_field_to_its_edge() {
    let mut game = round("");
    let events = charge(&mut game, full_ticks());
    let end = rail_slugs(&events).iter().find_map(|e| match **e {
        Event::RailSlug { x1, .. } => Some(x1),
        _ => None,
    });
    assert!(end.is_some_and(|x| x > W - 40.0), "{end:?}");
}

/// Overcharged, the slug cuts iron - the iron whole - and the shooter is
/// spun round, its facing reversed and its stick locked while it skids.
#[test]
fn an_overcharged_slug_cuts_iron_and_spins_the_shooter() {
    let t = tuning();
    let mut game = round("cells.\"10,6\" = { kind = \"wall\", material = \"iron\" }\n");
    let behind = parked(&mut game, Position::new(600.0, 192.0));
    let over = crate::tank::ticks_of(t.gauss_charge_seconds + t.gauss_overcharge_seconds) + 1;
    let events = charge(&mut game, over);
    assert!(rail_slugs(&events).iter().any(|e| matches!(e, Event::RailSlug { overcharged: true, .. })));
    assert!(damage(&game, behind) > 0.0, "through the iron");
    assert!(!events.iter().any(|e| matches!(e, Event::ObstacleDestroyed { material: Material::Iron, .. })), "the iron stands");
    let (rotation, spin) = with_tank(&game.world, seat(&game), |tk| (tk.rotation, tk.spin));
    assert_eq!(Dir::from_rotation(rotation), Some(Dir::Left), "spun round to face back");
    assert!(spin > 0.0);
}

/// The recoil slides a standard chassis about a cell back, a scout further
/// and a titan less - the same whichever way the hull faces.
#[test]
fn the_recoil_slides_about_a_cell_and_a_heavy_chassis_less() {
    let slide = |row: i32, facing: f32| {
        let mut game = round_as("", 1, Mission::Destroy, Some(row));
        let s = seat(&game);
        game.place_tank(s, Position::new(544.0, 272.0), Some(facing)).unwrap();
        let from = with_tank(&game.world, s, |t| t.position);
        charge(&mut game, full_ticks());
        for _ in 0..90 {
            step(&mut game, false);
        }
        with_tank(&game.world, s, |t| t.position).distance_to(from)
    };
    let t = Tuning::DEFAULT;
    let mass = t.tank_mass_factor;
    for (row, want) in [(1, 32.0), (0, 32.0 / mass[0]), (10, 32.0 / mass[10])] {
        // The knock's own tick carries the hull before the skid's
        // friction first bites.
        let v = crate::gauss::recoil_speed(&t, mass[row as usize], false, crate::sonic::skid_friction(&t, 1.0));
        let got = slide(row, 90.0);
        assert!((got - want - v * DT).abs() < 3.0, "row {row}: {got} px against {want}");
    }
    assert!((slide(1, 90.0) - slide(1, 0.0)).abs() < 3.0, "the same slide head-on and broadside");
}

/// A drum in the lane goes off and the slug flies on to the enemy behind.
#[test]
fn a_drum_in_the_lane_goes_off_and_the_slug_flies_on() {
    let mut game = round("cells.\"8,6\" = { kind = \"barrel\", drum = \"oil\" }\n");
    let behind = parked(&mut game, Position::new(700.0, 192.0));
    let events = charge(&mut game, full_ticks());
    assert!(events.iter().any(|e| matches!(e, Event::Blast { .. })), "{events:?}");
    assert!(damage(&game, behind) > 0.0);
}

/// A tower in the lane takes the slug's damage, and the slug flies on.
#[test]
fn a_tower_in_the_lane_takes_the_slugs_damage_and_the_slug_flies_on() {
    let mut game = round("cells.\"8,6\" = { kind = \"tesla\", side = \"enemy\" }\n");
    let behind = parked(&mut game, Position::new(700.0, 192.0));
    let events = charge(&mut game, full_ticks());
    let hit = events.iter().find_map(|e| match *e {
        Event::Hit { target: HitTarget::Obstacle { material: Material::Tesla }, damage, .. } => Some(damage),
        _ => None,
    });
    assert_eq!(hit, Some(tuning().gauss_damage));
    assert!(damage(&game, behind) > 0.0);
}

/// Sandbags, fences, trees and lamp posts in the lane go down - no roll.
#[test]
fn sandbags_fences_trees_and_lamp_posts_in_the_lane_go_down() {
    let mut game = round("cells.\"6,6\" = { kind = \"sandbag\" }\ncells.\"8,6\" = { kind = \"fence\" }\ncells.\"10,6\" = { kind = \"lamp\" }\ncells.\"12,6\" = { kind = \"pine\" }\n");
    let events = charge(&mut game, full_ticks());
    let gone: Vec<Material> = events
        .iter()
        .filter_map(|e| match *e {
            Event::ObstacleDestroyed { material, .. } => Some(material),
            _ => None,
        })
        .collect();
    for m in [Material::Sandbag, Material::Fence, Material::Lamp] {
        assert!(gone.contains(&m), "{m:?} down: {gone:?}");
    }
    let pine_hit = events.iter().any(|e| matches!(e, Event::Hit { target: HitTarget::Obstacle { material: Material::Pine }, .. }));
    assert!(pine_hit, "the tree is hit: down, or alight");
}

/// A rainbow shield soaks one slug whole, and the slug flies on.
#[test]
fn a_rainbow_shield_soaks_one_slug_and_the_slug_flies_on() {
    let mut game = round("");
    let shielded = parked(&mut game, Position::new(400.0, 192.0));
    let behind = parked(&mut game, Position::new(700.0, 192.0));
    with_tank_mut(&game.world, shielded, |t| t.raise_shield());
    let events = charge(&mut game, full_ticks());
    assert_eq!(damage(&game, shielded), 0.0, "the hull takes nothing");
    assert!(events.iter().any(|e| matches!(e, Event::ShieldBroken { .. })));
    assert!(damage(&game, behind) > 0.0, "the slug flies on");
    let pierced = rail_slugs(&events).iter().find_map(|e| match **e {
        Event::RailSlug { ref pierced, .. } => Some(pierced.clone()),
        _ => None,
    });
    assert!(pierced.is_some_and(|p| p.iter().any(|&(_, _, w)| w == Pierced::Shield)));
}

/// A frog in the lane takes `gauss_frog_damage` and does not hop.
#[test]
fn a_frog_in_the_lane_takes_frog_damage_and_does_not_hop() {
    let mut game = round_as("cells.\"9,6\" = { kind = \"frog\" }\n", 1, Mission::Protect, None);
    let frog = game.frog.expect("a frog");
    let (before, at) = {
        let f = game.world.get::<&crate::frog::Frog>(frog).unwrap();
        (f.health, f.position)
    };
    charge(&mut game, full_ticks());
    let f = game.world.get::<&crate::frog::Frog>(frog).unwrap();
    assert!((before - f.health - tuning().gauss_frog_damage).abs() < 0.01, "{} -> {}", before, f.health);
    assert!(f.position.distance_to(at) < 1.0, "no hop");
}

/// Crates and wrecks are left alone.
#[test]
fn crates_and_wrecks_are_left_alone() {
    let mut game = round("cells.\"8,6\" = { kind = \"pickup\", pickup = \"health\" }\n");
    let wreck = parked(&mut game, Position::new(400.0, 192.0));
    with_tank_mut(&game.world, wreck, |t| t.damage = MAX_DAMAGE);
    let behind = parked(&mut game, Position::new(700.0, 192.0));
    let events = charge(&mut game, full_ticks());
    assert!(!events.iter().any(|e| matches!(e, Event::CrateBroken { .. })));
    let wreck_slot = with_tank(&game.world, wreck, |t| t.owner_slot());
    assert!(!events.iter().any(|e| matches!(*e, Event::Hit { target: HitTarget::Enemy { slot }, .. } if slot == wreck_slot)));
    assert!(damage(&game, behind) > 0.0, "the wreck is see-through");
}

/// A wreck mid-charge fires nothing: the charge is lost.
#[test]
fn a_wreck_mid_charge_fires_nothing() {
    let mut game = round("");
    for _ in 0..full_ticks() + 5 {
        step(&mut game, true);
    }
    with_tank_mut(&game.world, seat(&game), |t| t.damage = MAX_DAMAGE);
    let mut seen = step(&mut game, true);
    seen.extend(step(&mut game, false));
    assert!(rail_slugs(&seen).is_empty());
}

/// An EMP mid-charge lapses it.
#[test]
fn an_emp_mid_charge_lapses_it() {
    let mut game = round("");
    for _ in 0..full_ticks() + 5 {
        step(&mut game, true);
    }
    with_tank_mut(&game.world, seat(&game), |t| {
        t.disable(3.0);
    });
    let seen = charge(&mut game, 1);
    assert!(rail_slugs(&seen).is_empty());
    assert!(with_tank(&game.world, seat(&game), |t| t.charge.is_none()));
}

/// Another weapon's crate mid-charge lapses it; a rail crate refills and
/// keeps it.
#[test]
fn another_weapons_crate_lapses_a_charge_and_a_rail_crate_keeps_it() {
    let mut game = round("");
    for _ in 0..10 {
        step(&mut game, true);
    }
    with_tank_mut(&game.world, seat(&game), |t| t.take_weapon(ActiveWeapon::GaussRail));
    step(&mut game, true);
    assert!(with_tank(&game.world, seat(&game), |t| t.charge.is_some()), "a rail crate keeps it");
    with_tank_mut(&game.world, seat(&game), |t| t.take_weapon(ActiveWeapon::Laser));
    let seen = step(&mut game, true);
    assert!(seen.iter().any(|e| matches!(e, Event::ChargeEnded { end: ChargeEnd::Lapsed, .. })), "{seen:?}");
}

/// A teleport mid-charge keeps it (the charge is the trigger's).
#[test]
fn a_teleport_mid_charge_keeps_it() {
    let mut game = round("");
    for _ in 0..10 {
        step(&mut game, true);
    }
    game.place_tank(seat(&game), Position::new(500.0, 400.0), Some(90.0)).unwrap();
    step(&mut game, true);
    assert!(with_tank(&game.world, seat(&game), |t| t.charge.is_some()));
}

/// The rail draws no RNG of its own: a slug through tiles and tanks, with
/// no drum and no portal, leaves the round's RNG as it found it.
#[test]
fn the_rail_draws_no_rng() {
    let mut game = round("cells.\"6,6\" = { kind = \"wall\", material = \"brick\" }\n");
    let _ = parked(&mut game, Position::new(400.0, 192.0));
    for _ in 0..full_ticks() {
        step(&mut game, true);
    }
    let mut twin = round("cells.\"6,6\" = { kind = \"wall\", material = \"brick\" }\n");
    let _ = parked(&mut twin, Position::new(400.0, 192.0));
    for _ in 0..full_ticks() {
        step(&mut twin, true);
    }
    // One releases, the other only stands down; their RNG must agree.
    step(&mut game, false);
    with_tank_mut(&twin.world, seat(&twin), |t| t.charge = None);
    step(&mut twin, false);
    let a = format!("{:?}", game.rng.as_ref().map(|r| r.clone().random_range(0..u64::MAX)));
    let b = format!("{:?}", twin.rng.as_ref().map(|r| r.clone().random_range(0..u64::MAX)));
    assert_eq!(a, b);
}

/// The hold report decides the release: a room count one tick short of full
/// with a report at full fires; a report past the spare is held to it.
#[test]
fn the_hold_report_decides_the_release_within_its_spare() {
    let full = full_ticks();
    let mut tank = Tank { gauss_slugs: 2, ..Tank::default() };
    let dt = PHYSICS_FIXED_DT;
    assert_eq!(tank.step_charge(true, true, dt, true, None), crate::tank::ChargeEdge::Started);
    for _ in 1..full - 1 {
        tank.step_charge(true, false, dt, true, None);
    }
    assert_eq!(tank.charge.map(|c| c.ticks()), Some(full - 1));
    let edge = tank.step_charge(false, false, dt, true, Some(full));
    assert!(matches!(edge, crate::tank::ChargeEdge::Released(_)), "the client's count: {edge:?}");
    let mut tank = Tank { gauss_slugs: 2, charge: Some(Charge { weapon: ActiveWeapon::GaussRail, held: 10.0 * dt }), ..Tank::default() };
    let edge = tank.step_charge(false, false, dt, true, Some(full));
    assert_eq!(edge, crate::tank::ChargeEdge::Ended(ChargeEnd::Fizzled), "a report far past its own count is held to the spare");
}

/// A round with the rail in it replays bit for bit.
#[test]
fn a_round_with_the_rail_replays_bit_for_bit() {
    let run = || {
        let mut game = round("cells.\"6,6\" = { kind = \"wall\", material = \"brick\" }\ncells.\"8,4\" = { kind = \"barrel\", drum = \"fuel\" }\n");
        let _ = parked(&mut game, Position::new(400.0, 192.0));
        let mut trace = Vec::new();
        for i in 0..400 {
            step(&mut game, i % 120 < 100);
            trace.push(format!("{:?}", game.drawable_state()));
        }
        trace
    };
    assert_eq!(run(), run());
}

/// The hashed spawn swap hands the rail out by its share.
#[test]
fn the_spawn_swap_hands_out_the_rail_by_its_share() {
    let at = Position::new(400.0, 200.0);
    let mut t = Tuning::DEFAULT;
    let mut laser = Tank::default();
    laser.take_weapon(ActiveWeapon::Laser);
    sonic::swap_spawn_special_with(&t, &mut laser, 5, at);
    assert_eq!(laser.special(), Some(ActiveWeapon::Laser));
    t.enemy_special_weapon_gauss_share = 1.0;
    sonic::swap_spawn_special_with(&t, &mut laser, 5, at);
    assert_eq!(laser.special(), Some(ActiveWeapon::GaussRail));
}

