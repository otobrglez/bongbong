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

/// A gun tower takes a seat's slug and stands - the one tile a slug may
/// leave standing - and the slug flies on, a tower's keep off its damage.
#[test]
fn a_gun_tower_survives_a_seats_slug_and_the_slug_flies_on() {
    let mut game = round("cells.\"8,6\" = { kind = \"gun_tower\", side = \"enemy\" }\n");
    let behind = parked(&mut game, Position::new(700.0, 192.0));
    let events = charge(&mut game, full_ticks());
    let t = Tuning::DEFAULT;
    let hit = events.iter().find_map(|e| match *e {
        Event::Hit { target: HitTarget::Obstacle { material: Material::GunTower }, damage, killed, .. } => Some((damage, killed)),
        _ => None,
    });
    assert_eq!(hit, Some((t.gauss_damage, false)), "the slug's damage, and it stands");
    assert!(!events.iter().any(|e| matches!(e, Event::ObstacleDestroyed { material: Material::GunTower, .. })));
    assert!((damage(&game, behind) - t.gauss_damage * t.gauss_pierce_keep).abs() < 0.5, "{}", damage(&game, behind));
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
    let mut tank = Tank { gauss_slugs: 2, charge: Some(Charge::new(ActiveWeapon::GaussRail, 10.0 * dt)), ..Tank::default() };
    let edge = tank.step_charge(false, false, dt, true, Some(full));
    assert_eq!(edge, crate::tank::ChargeEdge::Ended(ChargeEnd::Fizzled), "a report far past its own count is held to the spare");
}

/// The spare is a press's, never a tick's: a client reporting all it may
/// every tick - past the round's own count - is full `CHARGE_HOLD_SPARE_TICKS`
/// early and no earlier, and one reporting the least holds a charge that
/// many ticks past its vent and no longer.
#[test]
fn the_hold_reports_spare_is_a_presss_never_a_ticks() {
    use crate::tank::{ChargeEdge, ChargeStage};
    let rule = ActiveWeapon::GaussRail.charge_rule().expect("a charge weapon");
    let spare = crate::tank::CHARGE_HOLD_SPARE_TICKS;
    let dt = PHYSICS_FIXED_DT;
    // Held `ticks` ticks with every read reporting `report`, then let go:
    // the edge it ended on.
    let hold = |ticks: u32, report: u32| {
        let mut tank = Tank { gauss_slugs: 2, ..Tank::default() };
        let mut edge = tank.step_charge(true, true, dt, true, Some(report));
        for _ in 1..ticks {
            edge = tank.step_charge(true, false, dt, true, Some(report));
        }
        if tank.charge.is_none() {
            return edge;
        }
        tank.step_charge(false, false, dt, true, Some(report))
    };
    let (full, vent) = (rule.full_ticks(), rule.vent_ticks());
    assert_eq!(hold(full - spare - 1, u32::MAX), ChargeEdge::Ended(ChargeEnd::Fizzled), "no fuller than the spare");
    assert_eq!(hold(full - spare, u32::MAX), ChargeEdge::Released(ChargeStage::Full), "the spare");
    assert!(matches!(hold(vent + spare, 1), ChargeEdge::Released(_)), "held the spare past its vent");
    assert_eq!(hold(vent + spare + 1, 1), ChargeEdge::Ended(ChargeEnd::Vented), "and no longer");
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


// ---- the AI ---------------------------------------------------------------

/// A rail-armed enemy of the player role at `at`, parked.
fn rail_enemy(game: &mut Game, at: Position) -> Entity {
    let entity = parked(game, at);
    with_tank_mut(&game.world, entity, |tk| tk.gauss_slugs = 4);
    entity
}

/// Run the round, the seat idle, until `enemy` starts a charge - its
/// trigger timer runs down first - or 300 ticks pass; then a full charge
/// and half a second more. Every event.
fn watch(game: &mut Game, enemy: Entity) -> Vec<Event> {
    let slot = slot_of(game, enemy);
    let mut seen = Vec::new();
    for _ in 0..300 {
        seen.extend(step(game, false));
        if charged(&seen, slot) {
            seen.extend(idle(game, full_ticks() + 30));
            break;
        }
    }
    seen
}

fn slot_of(game: &Game, entity: Entity) -> usize {
    with_tank(&game.world, entity, |t| t.owner_slot())
}

/// Run the round `ticks` ticks with the seat idle; every event.
fn idle(game: &mut Game, ticks: u32) -> Vec<Event> {
    let mut seen = Vec::new();
    for _ in 0..ticks {
        seen.extend(step(game, false));
    }
    seen
}

fn charged(events: &[Event], slot: usize) -> bool {
    events.iter().any(|e| matches!(*e, Event::ChargeStarted { slot: s, .. } if s == slot))
}

fn fired(events: &[Event], slot: usize) -> bool {
    events.iter().any(|e| matches!(*e, Event::Fired { slot: s, weapon: "gauss_rail" } if s == slot))
}

/// An enemy with the rail lines up on a seat in its lane, inside the
/// seat's sight box, charges and fires at full; the seat is hit.
#[test]
fn the_rail_charges_at_a_seat_in_its_lane_inside_the_sight_box() {
    let mut game = round("");
    let enemy = rail_enemy(&mut game, Position::new(400.0, 192.0));
    let slot = slot_of(&game, enemy);
    let events = watch(&mut game, enemy);
    assert!(charged(&events, slot), "it charges");
    assert!(fired(&events, slot), "and fires at full");
    assert!(damage(&game, seat(&game)) > 0.0, "the seat is hit");
}

/// It fires through brick at a seat it knows is behind it.
#[test]
fn the_rail_fires_through_brick_at_a_seat_it_knows_is_behind_it() {
    let mut game = round("cells.\"7,6\" = { kind = \"wall\", material = \"brick\" }\ncells.\"7,5\" = { kind = \"wall\", material = \"brick\" }\ncells.\"7,7\" = { kind = \"wall\", material = \"brick\" }\n");
    let enemy = rail_enemy(&mut game, Position::new(400.0, 192.0));
    let events = watch(&mut game, enemy);
    assert!(fired(&events, slot_of(&game, enemy)));
    assert!(damage(&game, seat(&game)) > 0.0);
}

/// Iron between: it never charges that way.
#[test]
fn the_rail_never_charges_through_iron() {
    let mut game = round("cells.\"7,6\" = { kind = \"wall\", material = \"iron\" }\n");
    let enemy = rail_enemy(&mut game, Position::new(400.0, 192.0));
    let events = watch(&mut game, enemy);
    assert!(!charged(&events, slot_of(&game, enemy)), "{events:?}");
}

/// A seat hidden in tall grass is not charged at.
#[test]
fn the_rail_does_not_fire_at_a_seat_hidden_in_grass() {
    let mut game = round("cells.\"5,6\" = { kind = \"tall_grass\" }\ncells.\"6,6\" = { kind = \"tall_grass\" }\ncells.\"4,6\" = { kind = \"tall_grass\" }\n");
    let s = seat(&game);
    game.place_tank(s, Position::new(160.0, 192.0), Some(90.0)).unwrap();
    let enemy = rail_enemy(&mut game, Position::new(400.0, 192.0));
    let events = watch(&mut game, enemy);
    assert!(!charged(&events, slot_of(&game, enemy)));
}

/// A seat that steps out of the lane before full wastes the charge: the
/// enemy waits holding it, then vents; one that steps back in before the
/// overcharge is fired at.
#[test]
fn the_rail_waits_at_full_and_wastes_the_charge_when_the_seat_steps_aside() {
    let t = tuning();
    let mut game = round("");
    let enemy = rail_enemy(&mut game, Position::new(400.0, 192.0));
    let slot = slot_of(&game, enemy);
    let mut seen = Vec::new();
    while !charged(&seen, slot) {
        seen.extend(step(&mut game, false));
        assert!(seen.len() < 10_000);
    }
    let s = seat(&game);
    game.place_tank(s, Position::new(96.0, 400.0), Some(90.0)).unwrap();
    let rest = idle(&mut game, crate::tank::ticks_of(t.gauss_charge_seconds + t.gauss_hold_seconds) + 10);
    assert!(!fired(&rest, slot), "never fires at an empty lane");
    assert!(rest.iter().any(|e| matches!(*e, Event::ChargeEnded { slot: s, end: ChargeEnd::Vented, .. } if s == slot)), "it vents");
}

#[test]
fn the_rail_fires_when_the_seat_steps_back_in_before_the_overcharge() {
    let mut game = round("");
    let enemy = rail_enemy(&mut game, Position::new(400.0, 192.0));
    let slot = slot_of(&game, enemy);
    let mut seen = Vec::new();
    while !charged(&seen, slot) {
        seen.extend(step(&mut game, false));
    }
    let s = seat(&game);
    game.place_tank(s, Position::new(96.0, 400.0), Some(90.0)).unwrap();
    let held = idle(&mut game, full_ticks() + 10);
    assert!(!fired(&held, slot), "full, the lane empty: it holds");
    game.place_tank(s, SEAT, Some(90.0)).unwrap();
    let back = idle(&mut game, 10);
    assert!(fired(&back, slot), "back in the lane: fired");
}

/// It never releases overcharged: a seat stepping back in after the
/// overcharge threshold is not fired at.
#[test]
fn the_rail_never_releases_overcharged() {
    let t = tuning();
    let mut game = round("");
    let enemy = rail_enemy(&mut game, Position::new(400.0, 192.0));
    let slot = slot_of(&game, enemy);
    let mut seen = Vec::new();
    while !charged(&seen, slot) {
        seen.extend(step(&mut game, false));
    }
    let s = seat(&game);
    game.place_tank(s, Position::new(96.0, 400.0), Some(90.0)).unwrap();
    idle(&mut game, crate::tank::ticks_of(t.gauss_charge_seconds + t.gauss_overcharge_seconds) + 2);
    game.place_tank(s, SEAT, Some(90.0)).unwrap();
    let back = idle(&mut game, crate::tank::ticks_of(t.gauss_hold_seconds - t.gauss_overcharge_seconds) + 5);
    assert!(!fired(&back, slot), "overcharged, it holds to the vent");
}

/// A fellow enemy in the lane holds the rail.
#[test]
fn the_rail_never_fires_through_a_friend() {
    let mut game = round("");
    let enemy = rail_enemy(&mut game, Position::new(500.0, 192.0));
    let _friend = parked(&mut game, Position::new(300.0, 192.0));
    let events = watch(&mut game, enemy);
    assert!(!charged(&events, slot_of(&game, enemy)));
}

/// Its own side's tower in the lane holds it too; a player's tower adds to
/// a lane's worth.
#[test]
fn the_rail_never_fires_through_its_own_tower() {
    let mut game = round("cells.\"8,6\" = { kind = \"tesla\", side = \"enemy\" }\n");
    let enemy = rail_enemy(&mut game, Position::new(400.0, 192.0));
    let events = watch(&mut game, enemy);
    assert!(!charged(&events, slot_of(&game, enemy)));
}

/// Two seats in one lane outrank one seat in another: from a corner where
/// one seat stands west and two north, it faces north.
#[test]
fn the_rail_prefers_a_lane_with_two_seats() {
    let mut game = round_with("", 2);
    let s1 = seat(&game);
    let s2 = game.seat(1).expect("a second seat");
    // The enemy at (480, 320): seat 1 west of it, seat 2 and a player
    // tower... seat 2 north of it and seat 1 moved north too.
    let enemy = rail_enemy(&mut game, Position::new(480.0, 320.0));
    game.place_tank(s1, Position::new(480.0, 160.0), Some(180.0)).unwrap();
    game.place_tank(s2, Position::new(480.0, 112.0), Some(180.0)).unwrap();
    let lone = parked(&mut game, Position::new(900.0, 320.0));
    with_tank_mut(&game.world, lone, |t| t.damage = MAX_DAMAGE);
    let slot = slot_of(&game, enemy);
    let mut events = Vec::new();
    while !charged(&events, slot) {
        events.extend(step(&mut game, false));
        assert!(events.len() < 100_000, "it charges");
    }
    let facing = with_tank(&game.world, enemy, |t| Dir::from_rotation(t.rotation));
    assert_eq!(facing, Some(Dir::Up), "the lane with both seats");
    let lane = crate::ai::GaussLane { seats: 2, ..Default::default() };
    let one = crate::ai::GaussLane { seats: 1, towers: 1, ..Default::default() };
    assert!(lane.score() > one.score() && one.score() > crate::ai::GaussLane { seats: 1, ..Default::default() }.score());
}

/// A training dummy never charges.
#[test]
fn a_training_dummy_never_charges() {
    let mut game = round("");
    let enemy = rail_enemy(&mut game, Position::new(400.0, 192.0));
    game.world.get::<&mut Ai>(enemy).unwrap().frog_only = true;
    let events = watch(&mut game, enemy);
    assert!(!charged(&events, slot_of(&game, enemy)));
}

/// The generic tiers never fire the rail.
#[test]
fn the_generic_tiers_never_fire_the_rail() {
    assert!(!crate::ai::generic_fire(ActiveWeapon::GaussRail));
}

/// Hurt past fleeing mid-charge, it still holds the trigger - no other
/// tier lets it go - and fires at full.
#[test]
fn a_charge_in_progress_is_never_released_by_another_tier() {
    let mut game = round("");
    let enemy = rail_enemy(&mut game, Position::new(400.0, 192.0));
    let slot = slot_of(&game, enemy);
    let mut seen = Vec::new();
    while !charged(&seen, slot) {
        seen.extend(step(&mut game, false));
    }
    with_tank_mut(&game.world, enemy, |t| t.damage = tuning().enemy_flee_damage + 5.0);
    let rest = idle(&mut game, full_ticks() + 5);
    assert!(!rest.iter().any(|e| matches!(*e, Event::ChargeEnded { slot: s, .. } if s == slot)), "{rest:?}");
    assert!(fired(&rest, slot));
}

/// From outside a seat's sight box it never charges at it - lined up on
/// the seat's column further out than the box reaches.
#[test]
fn an_enemy_never_rails_a_seat_from_outside_its_sight_box() {
    let mut game = round("");
    let s = seat(&game);
    game.place_tank(s, Position::new(480.0, 32.0 * 2.0), Some(180.0)).unwrap();
    let enemy = rail_enemy(&mut game, Position::new(480.0, 64.0 + 300.0));
    let events = watch(&mut game, enemy);
    assert!(!charged(&events, slot_of(&game, enemy)), "300 px down a column, past the box's 240");
}

/// A hunter rails its quarry, the players' frog.
#[test]
fn a_hunter_rails_its_quarry() {
    let mut game = round_as("cells.\"20,12\" = { kind = \"frog\" }\n", 1, Mission::Protect, None);
    let s = seat(&game);
    game.place_tank(s, Position::new(96.0, 448.0), Some(90.0)).unwrap();
    let slot = game.debug_spawn_enemy(Position::new(640.0, 192.0), Some(1), Some(Role::Hunter)).unwrap();
    let enemy = game.tank_entity_by_slot(slot).unwrap();
    with_tank_mut(&game.world, enemy, |t| {
        t.speed_scale = 0.0;
        t.disarm();
        t.gauss_slugs = 4;
    });
    let frog = game.frog.unwrap();
    let before = game.world.get::<&crate::frog::Frog>(frog).unwrap().health;
    let events = watch(&mut game, enemy);
    assert!(fired(&events, slot), "the frog in its column");
    assert!(game.world.get::<&crate::frog::Frog>(frog).unwrap().health < before);
}

/// Enemies step out of a charging seat's lane and do not step back while
/// it charges.
#[test]
fn enemies_step_out_of_a_charging_seats_lane_and_do_not_step_back() {
    let mut game = round("");
    let slot = game.debug_spawn_enemy(Position::new(480.0, 192.0), Some(1), Some(Role::Player)).unwrap();
    let enemy = game.tank_entity_by_slot(slot).unwrap();
    with_tank_mut(&game.world, enemy, |t| {
        t.shells_ammo = 0;
        t.disarm();
    });
    let half = crate::battlefield::max_tank_clearance_half_extent() + tuning().gauss_half_width;
    let in_lane = |game: &Game| (with_tank(&game.world, enemy, |t| t.position).y - 192.0).abs() < half;
    let mut out_at = None;
    for i in 0..full_ticks() + 60 {
        step(&mut game, true);
        if out_at.is_none() && !in_lane(&game) {
            out_at = Some(i);
        }
        if out_at.is_some() {
            assert!(!in_lane(&game), "back in the lane at tick {i}");
        }
        if with_tank(&game.world, seat(&game), |t| t.charge.is_none()) {
            break;
        }
    }
    assert!(out_at.is_some_and(|i| i < 80), "out of the lane: {out_at:?}");
}

/// An enemy charging a rail whose slug would go through the seat - through
/// brick, further than any shot's range - is a lane threat to it; facing
/// away, or with iron between, it is not.
#[test]
fn a_charging_rails_lane_through_cover_warns_the_seat() {
    let mut game = round("cells.\"20,6\" = { kind = \"wall\", material = \"brick\" }\ncells.\"20,12\" = { kind = \"wall\", material = \"iron\" }\n");
    let s = seat(&game);
    let enemy = parked(&mut game, Position::new(1000.0, 192.0));
    let slot = slot_of(&game, enemy);
    let charging = |game: &mut Game, rotation: f32| {
        with_tank_mut(&game.world, enemy, |t| {
            t.rotation = rotation;
            t.gauss_slugs = 4;
            t.charge = Some(Charge::new(ActiveWeapon::GaussRail, 0.6));
        });
        let scene = crate::indicators::Scene::of(game, 0);
        let tv = *scene.tanks.iter().find(|tv| tv.slot == slot).expect("the enemy");
        assert_eq!(tv.windup.map(|(w, _)| w), Some(ActiveWeapon::GaussRail));
        tv.lane
    };
    assert!(charging(&mut game, 270.0), "through the brick, 900 px away");
    assert!(!charging(&mut game, 90.0), "facing away");
    game.place_tank(s, Position::new(96.0, 384.0), Some(90.0)).unwrap();
    game.place_tank(enemy, Position::new(1000.0, 384.0), Some(270.0)).unwrap();
    assert!(!charging(&mut game, 270.0), "iron between");
}

/// Through a portal the slug goes leg by leg: the first leg ends going
/// into the portal (`ShotTeleported` logged), the next leaves the other
/// portal on the same heading, and the enemy past it is hit by the damage
/// the slug has kept.
#[test]
fn the_slug_goes_through_a_portal_leg_by_leg() {
    let mut game = round("cells.\"8,6\" = { kind = \"portal\" }\ncells.\"20,12\" = { kind = \"portal\" }\n");
    let past = parked(&mut game, Position::new(900.0, 400.0));
    let events = charge(&mut game, full_ticks());
    let legs: Vec<(u8, bool, f32)> = events
        .iter()
        .filter_map(|e| match *e {
            Event::RailSlug { leg, portal, x0, .. } => Some((leg, portal, x0)),
            _ => None,
        })
        .collect();
    assert!(legs.len() >= 2, "{legs:?}");
    assert_eq!((legs[0].0, legs[0].1), (0, true), "the first leg ends in the portal");
    assert_eq!(legs[1].0, 1);
    assert!((legs[1].2 - 656.0).abs() < 40.0, "the second leaves the other portal: {legs:?}");
    assert!(events.iter().any(|e| matches!(e, Event::ShotTeleported { .. })));
    assert!(damage(&game, past) > 0.0, "the enemy past the exit is hit");
}

/// A training door stops even an overcharged slug, as iron does a full one.
#[test]
fn a_door_stops_even_an_overcharged_slug() {
    let t = tuning();
    let mut game = round("cells.\"8,6\" = { kind = \"door\", beat = 1 }\n");
    let behind = parked(&mut game, Position::new(500.0, 192.0));
    let over = crate::tank::ticks_of(t.gauss_charge_seconds + t.gauss_overcharge_seconds) + 1;
    let events = charge(&mut game, over);
    assert!(events.iter().any(|e| matches!(*e, Event::RailSlug { overcharged: true, .. })));
    assert_eq!(damage(&game, behind), 0.0);
}

/// A volcano's cone stops even an overcharged slug: the leg ends at the
/// cone's first cell and nothing past it is touched.
#[test]
fn a_volcanos_cone_stops_even_an_overcharged_slug() {
    let t = tuning();
    // The crater at (12, 6): its cone reaches two cells either way along
    // the row, its west face at cell 10.
    let mut game = round("cells.\"12,6\" = { kind = \"volcano\" }\n");
    let behind = parked(&mut game, Position::new(900.0, 192.0));
    let over = crate::tank::ticks_of(t.gauss_charge_seconds + t.gauss_overcharge_seconds) + 1;
    let events = charge(&mut game, over);
    let (end, pierced) = rail_slugs(&events)
        .iter()
        .find_map(|e| match **e {
            Event::RailSlug { x1, overcharged: true, ref pierced, .. } => Some((x1, pierced.len())),
            _ => None,
        })
        .expect("an overcharged slug");
    assert!(end <= 10.0 * 32.0 && end > 9.0 * 32.0, "at the cone's west face: {end}");
    assert_eq!(pierced, 0, "nothing on open ground before it");
    assert_eq!(damage(&game, behind), 0.0);
}

/// A seat's slug through a teammate in its lane hits it as any shot does:
/// `friendly_fire_damage_factor` of it, through the teammate's armour.
#[test]
fn a_seats_slug_through_a_teammate_is_friendly_fire() {
    let mut game = round_with("", 2);
    let mate = game.seat(1).expect("a second seat");
    game.place_tank(mate, Position::new(400.0, 192.0), Some(0.0)).unwrap();
    let before = damage(&game, mate);
    let events = charge(&mut game, full_ticks());
    let t = Tuning::DEFAULT;
    let want = (t.gauss_damage * t.friendly_fire_damage_factor * t.player_armor_factor).min(MAX_DAMAGE);
    assert!((damage(&game, mate) - before - want).abs() < 0.5, "{} against {want}", damage(&game, mate) - before);
    assert!(events.iter().any(|e| matches!(e, Event::Hit { target: HitTarget::Player { player: 1 }, cause: HitCause::Rail, .. })));
}

/// An enemy's slug is used on the seat whose sight box it stands in, and
/// goes on through a second seat further down the lane whose box it stands
/// outside (decision 2): both are hit, the shot is the first's.
#[test]
fn an_enemys_slug_used_on_one_seat_goes_on_through_the_next() {
    let mut game = round_with("", 2);
    let (near, far) = (seat(&game), game.seat(1).expect("a second seat"));
    game.place_tank(near, Position::new(480.0, 192.0), Some(90.0)).unwrap();
    game.place_tank(far, Position::new(96.0, 192.0), Some(90.0)).unwrap();
    let enemy = rail_enemy(&mut game, Position::new(700.0, 192.0));
    let (half_w, _) = tuning().sight_box_half_px();
    assert!(700.0 - 96.0 > half_w && 700.0 - 480.0 < half_w, "outside the far seat's box, inside the near one's");
    let slot = slot_of(&game, enemy);
    let mut seen = Vec::new();
    for _ in 0..400 {
        seen.extend(step(&mut game, false));
        if fired(&seen, slot) {
            break;
        }
    }
    assert!(fired(&seen, slot), "it fires at the near seat");
    assert_eq!(game.world.get::<&Ai>(enemy).unwrap().shot_at_seat(), Some(0), "used on the near seat");
    assert!(damage(&game, near) > 0.0 && damage(&game, far) > 0.0, "and the slug goes on through the far one");
}

/// A sonic hammer's shove mid-charge keeps the charge: the skid carries the
/// hull and the trigger, still down, fires at full.
#[test]
fn a_sonic_shove_mid_charge_keeps_it() {
    let mut game = round("");
    for _ in 0..10 {
        step(&mut game, true);
    }
    let s = seat(&game);
    {
        let Game { world, physics, water, lava, weather, time, .. } = &mut game;
        let footing = Footing::at(water, lava, *weather, Position::new(96.0, 192.0), *time);
        let mut tank = world.get::<&mut Tank>(s).unwrap();
        super::sonic::knock_hull(physics, &mut tank, Vec2::new(0.0, 1.0), 300.0, footing);
    }
    assert!(with_tank(&game.world, s, |t| t.skid > 0.0 && t.charge.is_some()), "skidding, still charging");
    let rest = charge(&mut game, full_ticks());
    assert_eq!(rail_slugs(&rest).len(), 1, "fired at full after the shove");
}

/// The round ending mid-charge clears it, and nothing fires on the end
/// screen.
#[test]
fn the_round_ending_mid_charge_clears_it_and_nothing_fires_on_the_end_screen() {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.show_intro = false;
    game.level_overrides.mission = Some(Mission::Destroy);
    game.map = MapFile::from_toml_str("version = 1\ntanks = 1\ncells.\"3,6\" = { kind = \"start\" }\n").expect("parses");
    game.init(W, H);
    let s = seat(&game);
    with_tank_mut(&game.world, s, |t| {
        t.disarm();
        t.gauss_slugs = 4;
    });
    for _ in 0..full_ticks() {
        step(&mut game, true);
    }
    assert!(with_tank(&game.world, s, |t| t.charge.is_some()));
    game.debug_kill(game.first_enemy_slot()).expect("the one enemy");
    let mut seen = Vec::new();
    for _ in 0..5 {
        seen.extend(step(&mut game, true));
    }
    assert_eq!(game.outcome(), Outcome::Won, "the round ended");
    assert!(with_tank(&game.world, s, |t| t.charge.is_none()));
    seen.extend(step(&mut game, false));
    assert!(rail_slugs(&seen).is_empty(), "{seen:?}");
}

/// An enemy whose way runs across a charging seat's lane waits at the edge
/// (`Ai::kept_out`) rather than walk in: seat 1 charges east along its row,
/// a lane across the field, and the enemy north of it wants seat 2 south
/// of it.
#[test]
fn enemies_wait_at_the_edge_of_a_lane_rather_than_cross_it() {
    let mut game = round_with("", 2);
    let s2 = game.seat(1).expect("a second seat");
    game.place_tank(s2, Position::new(640.0, 448.0), Some(0.0)).unwrap();
    let slot = game.debug_spawn_enemy(Position::new(640.0, 64.0), Some(1), Some(Role::Player)).unwrap();
    let enemy = game.tank_entity_by_slot(slot).unwrap();
    with_tank_mut(&game.world, enemy, |t| {
        t.shells_ammo = 0;
        t.disarm();
    });
    let half = crate::battlefield::max_tank_clearance_half_extent() + tuning().gauss_half_width;
    let mut held = false;
    for _ in 0..full_ticks() + 30 {
        step(&mut game, true);
        let y = with_tank(&game.world, enemy, |t| t.position.y);
        assert!((y - 192.0).abs() >= half - 2.0, "it walked into the lane: {y}");
        held |= game.world.get::<&Ai>(enemy).unwrap().kept_out();
    }
    assert!(held, "it waited at the edge");
}

/// A friend beyond the seat does not hold the rail: two rail tanks either
/// side of a seat on its row each fire at it rather than wait on the other,
/// and the slug flies on into the far one.
#[test]
fn a_friend_beyond_the_seat_does_not_hold_the_rail() {
    let mut game = round("");
    let s = seat(&game);
    game.place_tank(s, Position::new(480.0, 192.0), Some(0.0)).unwrap();
    let east = rail_enemy(&mut game, Position::new(760.0, 192.0));
    let west = rail_enemy(&mut game, Position::new(200.0, 192.0));
    let mut seen = Vec::new();
    for _ in 0..400 {
        seen.extend(step(&mut game, false));
    }
    assert!(fired(&seen, slot_of(&game, east)) || fired(&seen, slot_of(&game, west)), "one of them fires");
    let lane = crate::ai::GaussLane { at_seat: Some((0, 100.0)), seats: 1, friend: Some(300.0), ..Default::default() };
    assert!(lane.counts(), "a friend beyond the seat");
    assert!(!crate::ai::GaussLane { friend: Some(50.0), ..lane }.counts(), "a friend before it");
    assert!(!crate::ai::GaussLane { at_seat: None, seats: 0, towers: 2, ..Default::default() }.counts(), "towers alone are no target");
}

/// A charge starts only on a seat inside its sight box by
/// `gauss_ai_box_margin_px`: at the box's very edge an enemy does not
/// charge, a little further in it does.
#[test]
fn a_charge_starts_only_inside_the_box_by_its_margin() {
    let t = tuning();
    let (half_w, _) = t.sight_box_half_px();
    for (inset, charges) in [(t.gauss_ai_box_margin_px * 0.5, false), (t.gauss_ai_box_margin_px * 2.0, true)] {
        let mut game = round("");
        let enemy = rail_enemy(&mut game, Position::new(SEAT.x + half_w - inset, SEAT.y));
        let events = watch(&mut game, enemy);
        assert_eq!(charged(&events, slot_of(&game, enemy)), charges, "{inset} px inside the box");
    }
}

/// A lane's depth is its half width less the offset across it along the
/// lane and nothing past its ends; its first exit is on the side a tank
/// faces across to - a tank crossing goes on over rather than turning back -
/// and on the near side for one facing along it.
#[test]
fn a_lane_danger_has_depth_across_it_and_exits_ahead_of_a_crossing_tank() {
    use crate::ai::{Danger, DangerShape};
    let lane = Danger {
        shape: DangerShape::Lane { at: Position::new(100.0, 200.0), from: Position::new(126.0, 200.0), dir: Dir::Right, length: 600.0, half_width: 24.0 },
        owner: Some(3),
        slack: 0.0,
    };
    assert_eq!(lane.depth(Position::new(300.0, 200.0)), 24.0);
    assert_eq!(lane.depth(Position::new(300.0, 210.0)), 14.0);
    assert!(lane.depth(Position::new(300.0, 240.0)) < 0.0, "beside it");
    assert!(lane.depth(Position::new(80.0, 200.0)) < 0.0, "behind the muzzle");
    assert!(lane.depth(Position::new(800.0, 200.0)) < 0.0, "past its stop");
    let p = Position::new(300.0, 195.0);
    let down = lane.exits(p, 8.0, p, Dir::Down)[0];
    let up = lane.exits(p, 8.0, p, Dir::Up)[0];
    let along = lane.exits(p, 8.0, p, Dir::Right)[0];
    assert!(down.y > 200.0 + 24.0 && up.y < 200.0 - 24.0, "ahead of the crossing: {down:?} {up:?}");
    assert!(along.y < 200.0 - 24.0, "the near side for one facing along it: {along:?}");
    assert!(lane.exits(p, 8.0, p, Dir::Down).iter().all(|&q| lane.depth(q) < 0.0), "every exit out of it");
}
