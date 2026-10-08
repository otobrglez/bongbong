//! Headless scenario tests for the gravity well (docs/gravity-well.md
//! "Tests"): one per promised rule, on the default 34 x 17 field with the
//! seat at cell (3, 6) facing east and everything else placed by hand.
//! Tests cannot touch the global tuning table, so every number is read off
//! the defaults.

use super::*;
use crate::ai::Role;
use crate::grenade::Grenade;
use crate::well::{AnchorBy, Swallow, WellStage};
use crate::zone::Zone;

const W: f32 = 1088.0;
const H: f32 = 544.0;
const DT: f32 = 1.0 / 60.0;

/// The seat's pivot: cell (3, 6).
const SEAT: Position = Position::new(96.0, 192.0);

/// The middle of the field, where a well stands clear of everything.
const MID: Position = Position::new(544.0, 272.0);

/// A round on an open field plus `extra` cells, on `mission`, nobody
/// shielded, no banner, the seat an assault facing east with `weapon`.
fn round_with(extra: &str, mission: Mission, weapon: Option<ActiveWeapon>) -> Game {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.show_intro = false;
    game.level_overrides.mission = Some(mission);
    let map = format!("version = 1\ntanks = 0\ntank = \"assault\"\ncells.\"3,6\" = {{ kind = \"start\" }}\n{extra}");
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
        if let Some(w) = weapon {
            t.take_weapon(w);
        }
    });
    game
}

fn round(extra: &str) -> Game {
    round_with(extra, Mission::Destroy, Some(ActiveWeapon::GravityWell))
}

/// An enemy of chassis `row` at `at` facing `rotation`, its brain off for
/// the test (an EMP's outage: it coasts on no intent), unshielded, unarmed.
fn still_enemy(game: &mut Game, at: Position, row: i32, rotation: f32) -> Entity {
    let slot = game.debug_spawn_enemy(at, Some(row), Some(Role::Player)).expect("spawns");
    let entity = game.tank_entity_by_slot(slot).expect("exists");
    game.place_tank(entity, at, Some(rotation)).expect("placed");
    with_tank_mut(&game.world, entity, |t| {
        t.shells_ammo = 0;
        t.disarm();
        t.shield_hp = 0.0;
        t.shield_timer = 0.0;
        t.disable(1000.0);
    });
    entity
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

fn idle(game: &mut Game, ticks: u32) -> Vec<Event> {
    let mut seen = Vec::new();
    for _ in 0..ticks {
        seen.extend(step(game, false));
    }
    seen
}

fn ticks(seconds: f32) -> u32 {
    crate::tank::ticks_of(seconds)
}

fn seat(game: &Game) -> Entity {
    game.player().expect("a seat")
}

fn pos(game: &Game, e: Entity) -> Position {
    with_tank(&game.world, e, |t| t.position)
}

fn wells_left(game: &Game) -> i32 {
    with_tank(&game.world, seat(game), |t| t.wells)
}

fn wells(game: &Game) -> Vec<Zone> {
    game.zones.iter().filter(|z| z.well().is_some()).copied().collect()
}

fn anchored(events: &[Event]) -> Vec<(Position, AnchorBy)> {
    events.iter().filter_map(|e| if let Event::WellAnchored { x, y, by, .. } = *e { Some((Position::new(x, y), by)) } else { None }).collect()
}

fn collapsed(events: &[Event]) -> Vec<(u32, bool)> {
    events.iter().filter_map(|e| if let Event::WellCollapsed { id, early, .. } = *e { Some((id, early)) } else { None }).collect()
}

fn swallowed(events: &[Event]) -> Vec<Swallow> {
    events.iter().filter_map(|e| if let Event::Swallowed { what, .. } = *e { Some(what) } else { None }).collect()
}

/// A well of player 1's at `at`, stepped on until it pulls.
fn pulling_well(game: &mut Game, at: Position) -> u32 {
    let (id, _) = game.debug_well(at, false).expect("a well");
    for _ in 0..ticks(tuning().well_form_seconds) + 2 {
        if game.zones.iter().any(|z| z.id == id && z.well().is_some_and(|w| w.stage == WellStage::Pulling)) {
            return id;
        }
        step(game, false);
    }
    assert!(game.zones.iter().any(|z| z.id == id && z.well().is_some_and(|w| w.stage == WellStage::Pulling)), "the well pulls");
    id
}

// --- the orb --------------------------------------------------------------

#[test]
fn the_press_fires_a_slow_orb_and_spends_one_well() {
    let mut game = round("");
    assert_eq!(wells_left(&game), tuning().well_per_pickup);
    let seen = step(&mut game, true);
    assert!(seen.iter().any(|e| matches!(e, Event::Fired { weapon: "gravity_well", .. })), "the launch is a Fired");
    assert_eq!(game.orbs().len(), 1, "one orb in flight");
    let orb = game.orbs()[0];
    assert!((orb.velocity.length() - tuning().well_orb_speed).abs() < 1e-3);
    assert!(orb.velocity.x > 0.0 && orb.velocity.y.abs() < 1e-3, "along the facing");
    assert!(orb.position.x > SEAT.x, "from the gun line's muzzle");
    assert_eq!(wells_left(&game), tuning().well_per_pickup - 1);
    assert_eq!(with_tank(&game.world, seat(&game), |t| t.orb), Some(orb.id));
}

#[test]
fn a_second_press_anchors_the_orb_where_it_is() {
    let mut game = round("");
    step(&mut game, true);
    idle(&mut game, 30);
    let at = game.orbs()[0].position;
    let seen = step(&mut game, true);
    let a = anchored(&seen);
    assert_eq!(a.len(), 1, "anchored: {seen:?}");
    assert_eq!(a[0].1, AnchorBy::Press);
    assert!(a[0].0.distance_to(at) <= tuning().well_orb_speed * DT * 1.01, "where it stood");
    assert!(game.orbs().is_empty());
    let w = wells(&game);
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].well().unwrap().stage, WellStage::Forming);
    assert_eq!(wells_left(&game), tuning().well_per_pickup - 1, "the anchor spends nothing");
    assert!(!seen.iter().any(|e| matches!(e, Event::Fired { .. })), "and fires nothing");
}

#[test]
fn the_anchor_press_is_not_held_back_by_the_reload() {
    let mut game = round("");
    step(&mut game, true);
    step(&mut game, false);
    assert!(with_tank(&game.world, seat(&game), |t| t.fire_cooldown) > 0.0);
    let seen = step(&mut game, true);
    assert_eq!(anchored(&seen).len(), 1, "anchored through the reload");
}

#[test]
fn an_orb_anchors_on_the_first_wall_it_meets_and_floats_over_sandbags_and_fences() {
    let mut game = round("cells.\"5,6\" = { kind = \"sandbag\" }\ncells.\"6,6\" = { kind = \"fence\" }\ncells.\"9,6\" = { kind = \"wall\", material = \"brick\" }\n");
    step(&mut game, true);
    let seen = idle(&mut game, ticks(2.5));
    let a = anchored(&seen);
    assert_eq!(a.len(), 1, "{seen:?}");
    assert_eq!(a[0].1, AnchorBy::Contact);
    // The wall's box: its cell's centre less the tile's half.
    let face = crate::map::cell_to_world(9, 6).x - 12.0;
    assert!(a[0].0.x < face && a[0].0.x > face - 24.0, "backed off the wall's face: {:?}", a[0].0);
    assert!(game.world.query::<&Obstacle>().iter().filter(|o| !o.destroyed).count() >= 3, "nothing broken");
}

/// A range board (docs/range-target-prd.md) stands as tall as a wall: an
/// orb anchors against it, and neither the pull nor the collapse moves or
/// breaks it.
#[test]
fn an_orb_anchors_on_a_range_board_and_leaves_it_standing() {
    let mut game = round("cells.\"9,6\" = { kind = \"target\" }\n");
    step(&mut game, true);
    let seen = idle(&mut game, ticks(2.5));
    let a = anchored(&seen);
    assert_eq!(a.len(), 1, "{seen:?}");
    assert_eq!(a[0].1, AnchorBy::Contact);
    let face = crate::map::cell_to_world(9, 6).x - 12.0;
    assert!(a[0].0.x < face && a[0].0.x > face - 24.0, "backed off the board's face: {:?}", a[0].0);
    idle(&mut game, ticks(tuning().well_form_seconds + tuning().well_pull_seconds + 1.0));
    let board: Vec<(Position, bool)> =
        game.world.query::<&Obstacle>().iter().filter(|o| o.material == Material::Target).map(|o| (o.position, o.destroyed)).collect();
    assert_eq!(board, vec![(crate::map::cell_to_world(9, 6), false)], "where it stood, whole");
}

#[test]
fn an_orb_anchors_on_a_tank_it_meets() {
    let mut game = round("");
    let enemy = still_enemy(&mut game, Position::new(288.0, SEAT.y), 1, 0.0);
    step(&mut game, true);
    let seen = idle(&mut game, ticks(2.5));
    let a = anchored(&seen);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].1, AnchorBy::Contact);
    assert!(a[0].0.x < pos(&game, enemy).x, "on its near side");
}

#[test]
fn an_orb_never_anchored_anchors_itself_at_its_range() {
    let mut game = round("");
    step(&mut game, true);
    let from = game.orbs()[0].position.x - tuning().well_orb_speed * DT;
    let seen = idle(&mut game, ticks(tuning().well_orb_range_px / tuning().well_orb_speed) + 4);
    let a = anchored(&seen);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].1, AnchorBy::Range);
    assert!((a[0].0.x - from - tuning().well_orb_range_px).abs() < tuning().well_orb_speed * DT * 2.0, "{:?}", a[0].0);
}

#[test]
fn a_press_just_after_an_orb_anchored_itself_fires_nothing() {
    let mut game = round("cells.\"6,6\" = { kind = \"wall\", material = \"brick\" }\n");
    step(&mut game, true);
    let mut seen = Vec::new();
    for _ in 0..ticks(2.0) {
        seen = step(&mut game, false);
        if !anchored(&seen).is_empty() {
            break;
        }
    }
    assert_eq!(anchored(&seen).len(), 1);
    // The reload is long over by the time a slow orb meets a wall far off;
    // here it has not, so wait it out and press inside the grace.
    with_tank_mut(&game.world, seat(&game), |t| t.fire_cooldown = 0.0);
    let left = wells_left(&game);
    let seen = step(&mut game, true);
    assert!(!seen.iter().any(|e| matches!(e, Event::Fired { .. })), "the press meant for the anchor fires nothing");
    assert_eq!(wells_left(&game), left);
    assert!(game.orbs().is_empty());
    // Past the grace the trigger fires again.
    idle(&mut game, ticks(tuning().well_anchor_grace_seconds) + 1);
    with_tank_mut(&game.world, seat(&game), |t| t.fire_cooldown = 0.0);
    let seen = step(&mut game, true);
    assert!(seen.iter().any(|e| matches!(e, Event::Fired { .. })));
}

#[test]
fn an_emped_shooter_cannot_anchor_and_its_orb_anchors_at_range() {
    let mut game = round("");
    with_tank_mut(&game.world, seat(&game), |t| t.shells_ammo = 5);
    step(&mut game, true);
    step(&mut game, false);
    with_tank_mut(&game.world, seat(&game), |t| {
        t.disable(5.0);
        t.fire_cooldown = 0.0;
    });
    let seen = step(&mut game, true);
    assert!(anchored(&seen).is_empty(), "no anchor while the special is down");
    assert_eq!(game.orbs().len(), 1, "the orb flies on");
    let seen = idle(&mut game, ticks(2.5));
    assert_eq!(anchored(&seen).iter().map(|a| a.1).collect::<Vec<_>>(), vec![AnchorBy::Range]);
}

// --- the stages -----------------------------------------------------------

#[test]
fn the_ring_forms_then_the_pull_runs_then_it_collapses() {
    let mut game = round("");
    let enemy = still_enemy(&mut game, MID + Vec2::new(70.0, 0.0), 1, 90.0);
    let before = pos(&game, enemy);
    let (id, until) = game.debug_well(MID, false).expect("a well");
    assert!((until - game.time - tuning().well_form_seconds).abs() < 1e-4);
    idle(&mut game, ticks(tuning().well_form_seconds) - 2);
    assert!(pos(&game, enemy).distance_to(before) < 0.5, "nothing pulled while it forms");
    idle(&mut game, 4);
    let w = wells(&game);
    assert_eq!(w[0].well().unwrap().stage, WellStage::Pulling);
    assert!((w[0].until - until - tuning().well_pull_seconds).abs() < 1e-3, "until is the collapse");
    idle(&mut game, 30);
    assert!(pos(&game, enemy).x < before.x - 5.0, "pulled in: {:?} from {:?}", pos(&game, enemy), before);
    let seen = idle(&mut game, ticks(tuning().well_pull_seconds) + 2);
    assert_eq!(collapsed(&seen), vec![(id, false)]);
    assert!(wells(&game).is_empty(), "the zone is gone at the collapse");
}

// --- hulls ----------------------------------------------------------------

#[test]
fn a_stopped_hull_rolls_with_the_current_along_its_tracks() {
    let mut game = round("");
    // Facing the core (west), east of it: its tracks lie along the pull.
    let enemy = still_enemy(&mut game, MID + Vec2::new(90.0, 0.0), 1, 270.0);
    pulling_well(&mut game, MID);
    let a = pos(&game, enemy);
    idle(&mut game, 30);
    let b = pos(&game, enemy);
    let speed = (a.x - b.x) / (30.0 * DT);
    let s = crate::well::strength(80.0, &tuning());
    assert!(speed > tuning().well_current_speed * s * 0.5, "rolls in near the current's speed: {speed}");
    assert!((b.y - a.y).abs() < 0.5, "straight in");
}

#[test]
fn a_scout_is_dragged_and_a_titan_stands() {
    let mut game = round("");
    // Both broadside to the pull: facing up, east and west of the core.
    let titan = still_enemy(&mut game, MID + Vec2::new(40.0, 0.0), TankKind::Titan.row(), 0.0);
    let scout = still_enemy(&mut game, MID - Vec2::new(40.0, 0.0), TankKind::Scout.row(), 0.0);
    pulling_well(&mut game, MID);
    let (t0, s0) = (pos(&game, titan), pos(&game, scout));
    idle(&mut game, 30);
    assert!(pos(&game, titan).distance_to(t0) < 1.0, "the titan's grip holds: {:?} from {t0:?}", pos(&game, titan));
    assert!(pos(&game, scout).x > s0.x + 4.0, "the scout slides in: {:?} from {s0:?}", pos(&game, scout));
}

/// The doc's escape numbers through the drive at 60 Hz: an assault seat
/// 40 px from the core flooring it straight away is carried in, the same
/// seat turned across is out of the reach.
#[test]
fn deep_in_driving_across_escapes_and_driving_away_does_not() {
    let t = tuning();
    let run = |dir: Dir| {
        let mut game = round_with("", Mission::Destroy, None);
        pulling_well(&mut game, MID);
        let s = seat(&game);
        let rotation = match dir {
            Dir::Right => 90.0,
            _ => 0.0,
        };
        game.place_tank(s, MID + Vec2::new(40.0, 0.0), Some(rotation)).expect("placed");
        for _ in 0..ticks(1.5) {
            step_with(&mut game, Intent { move_dir: Some(dir), ..Intent::default() });
        }
        pos(&game, s).distance_to(MID)
    };
    let away = run(Dir::Right);
    let across = run(Dir::Up);
    assert!(away < 40.0, "driving away it is carried in: {away}");
    assert!(across > t.well_radius_px, "driving across it is out: {across}");
}

#[test]
fn the_pull_reaches_the_shooter_and_never_a_wreck() {
    let mut game = round("");
    let wreck = still_enemy(&mut game, MID + Vec2::new(0.0, 70.0), 1, 270.0);
    with_tank_mut(&game.world, wreck, |t| {
        t.take_damage(MAX_DAMAGE, MAX_DAMAGE);
    });
    idle(&mut game, 2);
    let w0 = pos(&game, wreck);
    // The shooter's own well, beside it.
    game.place_tank(seat(&game), MID - Vec2::new(60.0, 0.0), Some(90.0)).expect("placed");
    pulling_well(&mut game, MID);
    let s0 = pos(&game, seat(&game));
    idle(&mut game, 30);
    assert!(pos(&game, seat(&game)).x > s0.x + 2.0, "the shooter is pulled by its own well");
    assert!(pos(&game, wreck).distance_to(w0) < 0.5, "a wreck is left alone");
}

#[test]
fn ice_loosens_the_bite() {
    let t = tuning();
    // An assault broadside at a strength whose side pull its dry grip holds
    // and its iced grip does not.
    let d = 100.0;
    let s = crate::well::strength(d, &t);
    let side = t.well_side_pull * s;
    let probe = round_with("", Mission::Destroy, None);
    let grip = with_tank(&probe.world, seat(&probe), |tank| t.tank_turn_grip_force / tank.mass());
    assert!(side < grip && side > grip * t.ice_grip_factor, "the case holds at the defaults: {side} against {grip}");
    let run = |footing: Footing| {
        let mut game = round_with("", Mission::Destroy, None);
        pulling_well(&mut game, MID);
        let s = seat(&game);
        game.place_tank(s, MID + Vec2::new(d, 0.0), Some(0.0)).expect("placed");
        let wells = game.well_field.clone();
        let entity = s;
        let x0 = pos(&game, entity).x;
        for _ in 0..30 {
            let Game { world, physics, .. } = &mut game;
            let mut tank = world.get::<&mut Tank>(entity).unwrap();
            let fp = footing.pulled(&wells, &tank);
            drive_tank(physics, &mut tank, Intent::default(), DT, fp);
            physics.step();
            tank.position = physics.position(tank.body.unwrap());
        }
        x0 - pos(&game, entity).x
    };
    let dry = run(Footing::DRY);
    let ice = run(Footing { grip: t.ice_grip_factor, traction: t.ice_traction_factor, brake: t.ice_brake_factor, ..Footing::DRY });
    assert!(dry.abs() < 0.5, "dry, the grip holds: {dry}");
    assert!(ice > 2.0, "on ice it slides in: {ice}");
}

// --- shots ----------------------------------------------------------------

fn shells(game: &Game) -> Vec<(Vec2, f32)> {
    game.world.query::<&Shell>().iter().map(|s| (s.velocity, s.rotation)).collect()
}

#[test]
fn a_shell_bends_toward_the_core_at_its_own_speed() {
    let mut game = round_with("", Mission::Destroy, None);
    // A well a cell and a half south of the shell's line, ahead.
    pulling_well(&mut game, Position::new(352.0, SEAT.y + 48.0));
    with_tank_mut(&game.world, seat(&game), |t| t.fire_cooldown = 0.0);
    step(&mut game, true);
    let first = shells(&game);
    assert!(!first.is_empty());
    let speed = first[0].0.length();
    idle(&mut game, 20);
    let now = shells(&game);
    assert!(!now.is_empty(), "still flying");
    for (v, rotation) in now {
        assert!((v.length() - speed).abs() < speed * 1e-5, "its speed kept");
        assert!(v.y > 0.0, "turned toward the core: {v:?}");
        assert!((rotation - crate::well::heading_deg(v)).abs() < 1e-3, "its sprite turns with it");
    }
}

#[test]
fn a_shot_that_reaches_the_core_is_swallowed() {
    let mut game = round_with("", Mission::Destroy, None);
    pulling_well(&mut game, Position::new(352.0, SEAT.y));
    with_tank_mut(&game.world, seat(&game), |t| t.fire_cooldown = 0.0);
    let mut seen = step(&mut game, true);
    seen.extend(idle(&mut game, 40));
    let gone = swallowed(&seen);
    assert!(!gone.is_empty() && gone.iter().all(|&w| w == Swallow::Shell), "{gone:?}");
    assert!(!seen.iter().any(|e| matches!(e, Event::Hit { .. })), "no hit");
    assert!(shells(&game).is_empty());
}

#[test]
fn an_orb_in_another_wells_core_is_swallowed() {
    let mut game = round("");
    pulling_well(&mut game, Position::new(256.0, SEAT.y));
    let mut seen = step(&mut game, true);
    seen.extend(idle(&mut game, ticks(1.5)));
    assert_eq!(swallowed(&seen), vec![Swallow::Orb]);
    assert!(anchored(&seen).is_empty());
    assert_eq!(with_tank(&game.world, seat(&game), |t| t.orb), None, "its tank has none out");
}

// --- what it gathers ------------------------------------------------------

fn grenade_at(game: &mut Game, at: Position, fuse: f32) -> Entity {
    let mut g = Grenade::launch(at, Vec2::new(1.0, 0.0), Vec2::zero(), Owner::Player(0));
    g.id = game.take_shot_id();
    g.velocity = Vec2::zero();
    g.height = 0.0;
    g.climb = 0.0;
    g.fuse = fuse;
    game.world.spawn((g,))
}

#[test]
fn grenades_roll_in_and_circle_the_ring() {
    let mut game = round("");
    let id = pulling_well(&mut game, MID);
    let g = grenade_at(&mut game, MID + Vec2::new(70.0, 0.0), 30.0);
    idle(&mut game, ticks(2.0));
    let grenade = *game.world.get::<&Grenade>(g).unwrap();
    let orbit = grenade.orbit.expect("captured");
    assert_eq!(orbit.well, id);
    assert!((grenade.position.distance_to(MID) - tuning().well_ring_px).abs() < 0.01, "on the ring");
    let a = grenade.position;
    idle(&mut game, 5);
    let b = game.world.get::<&Grenade>(g).unwrap().position;
    assert!(a.distance_to(b) > 0.5, "circling");
    assert!((b.distance_to(MID) - tuning().well_ring_px).abs() < 0.01);
}

#[test]
fn a_captured_grenade_whose_fuse_ends_bursts_on_the_ring() {
    let mut game = round("");
    pulling_well(&mut game, MID);
    grenade_at(&mut game, MID + Vec2::new(tuning().well_ring_px - 2.0, 0.0), 0.5);
    let seen = idle(&mut game, ticks(1.0));
    let blast = seen.iter().find_map(|e| if let Event::GrenadeBlast { x, y, .. } = *e { Some(Position::new(x, y)) } else { None }).expect("a blast");
    assert!((blast.distance_to(MID) - tuning().well_ring_px).abs() < 0.5, "on the ring: {blast:?}");
}

#[test]
fn the_collapse_throws_the_grenades_out_in_a_ring() {
    let mut game = round("");
    pulling_well(&mut game, MID);
    let g = grenade_at(&mut game, MID + Vec2::new(tuning().well_ring_px - 2.0, 0.0), 30.0);
    idle(&mut game, ticks(tuning().well_pull_seconds) + 2);
    let grenade = *game.world.get::<&Grenade>(g).unwrap();
    assert!(grenade.orbit.is_none());
    let out = grenade.position - MID;
    assert!(grenade.velocity.x * out.x + grenade.velocity.y * out.y > 0.0, "thrown out along its radial");
    assert!(grenade.fuse > 20.0, "its fuse kept");
}

fn barrels(game: &Game) -> usize {
    game.world.query::<&Obstacle>().iter().filter(|o| !o.destroyed && o.material.is_explosive()).count()
}

#[test]
fn drums_in_reach_are_lifted_and_go_off_together_at_the_collapse() {
    // Two drums two cells from the middle (17, 8) and one well out of reach.
    let mut game = round("cells.\"15,8\" = { kind = \"barrel\", drum = \"oil\" }\ncells.\"19,8\" = { kind = \"barrel\", drum = \"oil\" }\ncells.\"30,2\" = { kind = \"barrel\", drum = \"oil\" }\n");
    let centre = crate::map::cell_to_world(17, 8);
    let (_, _) = game.debug_well(centre, false).expect("a well");
    let seen = idle(&mut game, ticks(tuning().well_form_seconds) + 2);
    let lifted = seen.iter().filter(|e| matches!(e, Event::ObstacleDestroyed { material: Material::Barrel, .. })).count();
    assert_eq!(lifted, 2, "the two in reach lifted");
    assert!(!seen.iter().any(|e| matches!(e, Event::Blast { .. })), "with no blast");
    assert_eq!(game.held_drums().len(), 2);
    assert_eq!(barrels(&game), 1, "the far one stands");
    let mut blasts_at = Vec::new();
    for _ in 0..ticks(tuning().well_pull_seconds) + 4 {
        let seen = step(&mut game, false);
        let n = seen.iter().filter(|e| matches!(e, Event::Blast { .. })).count();
        if n > 0 {
            blasts_at.push(n);
        }
    }
    assert_eq!(blasts_at.first(), Some(&2), "both on one tick: {blasts_at:?}");
    assert!(game.held_drums().is_empty());
}

#[test]
fn a_blast_over_a_held_drum_sets_it_off() {
    let mut game = round("cells.\"15,8\" = { kind = \"barrel\", drum = \"oil\" }\ncells.\"19,8\" = { kind = \"barrel\", drum = \"oil\" }\n");
    let centre = crate::map::cell_to_world(17, 8);
    game.debug_well(centre, false).expect("a well");
    idle(&mut game, ticks(tuning().well_form_seconds + tuning().well_capture_seconds) + 4);
    assert_eq!(game.held_drums().len(), 2, "both circling");
    grenade_at(&mut game, centre + Vec2::new(0.0, tuning().well_ring_px - 1.0), 0.05);
    let seen = idle(&mut game, 10);
    assert!(seen.iter().filter(|e| matches!(e, Event::Blast { chained: true, .. })).count() >= 2, "the chain: {seen:?}");
    assert!(game.held_drums().is_empty());
    assert!(!wells(&game).is_empty(), "before the collapse");
}

#[test]
fn crates_slide_in_and_out_and_keep_their_slot() {
    let mut game = round("cells.\"20,8\" = { kind = \"pickup\", pickup = \"health\" }\n");
    let slot = crate::map::cell_to_world(20, 8);
    let centre = crate::map::cell_to_world(17, 8);
    pulling_well(&mut game, centre);
    idle(&mut game, 30);
    let p = game.world.query::<&Pickup>().iter().next().map(|p| (p.position, p.at(), p.slide)).expect("the crate");
    assert_eq!(p.0, slot, "its slot kept");
    assert!(p.1.x < slot.x - 5.0, "drawn in: {:?}", p.1);
    idle(&mut game, ticks(tuning().well_pull_seconds));
    let p = game.world.query::<&Pickup>().iter().next().map(|p| (p.position, p.at(), p.slide)).expect("the crate");
    let out = p.1 - centre;
    assert!(p.2.x * out.x + p.2.y * out.y > 0.0 || p.1.distance_to(centre) > 20.0, "slid out");
}

#[test]
fn the_frog_slides_in_cannot_hop_and_is_flung_out() {
    let mut game = round_with("cells.\"20,8\" = { kind = \"frog\" }\n", Mission::Protect, Some(ActiveWeapon::GravityWell));
    let frog0 = game.world.query::<&Frog>().iter().next().expect("a frog").position;
    let centre = frog0 - Vec2::new(60.0, 0.0);
    pulling_well(&mut game, centre);
    idle(&mut game, 20);
    let frog = game.world.query::<&Frog>().iter().next().map(|f| (f.position, f.pulled, f.can_hop())).unwrap();
    assert!(frog.1, "pinned");
    assert!(!frog.2, "cannot hop");
    assert!(frog.0.x < frog0.x - 2.0, "slid in: {:?}", frog.0);
    idle(&mut game, ticks(tuning().well_pull_seconds) + 2);
    let (pulled, hop_end, position) = game.world.query::<&Frog>().iter().next().map(|f| (f.pulled, f.hop_end, f.position)).unwrap();
    assert!(!pulled);
    assert!(hop_end.distance_to(centre) > position.distance_to(centre) - 1.0, "hopped out");
}

// --- the collapse ---------------------------------------------------------

#[test]
fn the_collapse_flings_hulls_and_hurts_only_the_other_side() {
    let mut game = round("");
    let enemy = still_enemy(&mut game, MID + Vec2::new(60.0, 0.0), 1, 0.0);
    game.place_tank(seat(&game), MID - Vec2::new(60.0, 0.0), Some(0.0)).expect("placed");
    pulling_well(&mut game, MID);
    let (eh, sh) = (with_tank(&game.world, enemy, |t| t.damage), with_tank(&game.world, seat(&game), |t| t.damage));
    let mut seen = Vec::new();
    for _ in 0..ticks(tuning().well_pull_seconds) + 4 {
        seen = step(&mut game, false);
        if !collapsed(&seen).is_empty() {
            break;
        }
    }
    assert_eq!(collapsed(&seen).len(), 1);
    assert!(seen.iter().any(|e| matches!(e, Event::Hit { target: HitTarget::Enemy { .. }, cause: HitCause::Well, .. })));
    assert!(!seen.iter().any(|e| matches!(e, Event::Hit { target: HitTarget::Player { .. }, .. })), "the owner's side unhurt");
    assert!(with_tank(&game.world, enemy, |t| t.damage) > eh);
    assert_eq!(with_tank(&game.world, seat(&game), |t| t.damage), sh);
    idle(&mut game, 6);
    assert!(pos(&game, enemy).distance_to(MID) > 30.0 || pos(&game, seat(&game)).distance_to(MID) > 30.0, "flung out");
}

#[test]
fn an_emp_collapses_an_anchored_well_and_fizzles_an_orb() {
    let mut game = round_with("", Mission::Destroy, Some(ActiveWeapon::Emp));
    let near = SEAT + Vec2::new(96.0, 0.0);
    let (id, _) = game.debug_well(near, true).expect("a well");
    let orb_id = game.take_shot_id();
    game.orbs.push(crate::well::Orb { id: orb_id, ..crate::well::Orb::launch(SEAT + Vec2::new(0.0, 64.0), Vec2::new(0.0, 1.0), Owner::Enemy(MAX_SEATS), &tuning()) });
    let mut seen = step(&mut game, true);
    seen.extend(idle(&mut game, 60));
    assert_eq!(collapsed(&seen), vec![(id, true)], "{seen:?}");
    assert!(seen.iter().any(|e| matches!(e, Event::OrbFizzled { id, .. } if *id == orb_id)));
    assert!(game.orbs().is_empty());
}

/// On the end screen a well forms, swirls and collapses as a show: it pulls
/// no hull, lifts no drum and hurts nobody, and the drums it held when the
/// round ended go off at its collapse as blasts that hurt nobody - they do
/// not vanish.
#[test]
fn the_well_on_the_end_screen_pulls_nothing_and_its_drums_hurt_nobody() {
    let drums = "cells.\"15,8\" = { kind = \"barrel\", drum = \"oil\" }\ncells.\"19,8\" = { kind = \"barrel\", drum = \"oil\" }\n";
    let centre = crate::map::cell_to_world(17, 8);
    // A well that forms on the end screen lifts nothing.
    let mut game = round(drums);
    let enemy = still_enemy(&mut game, centre + Vec2::new(0.0, 80.0), 1, 270.0);
    game.debug_well(centre, false).expect("a well");
    game.outcome = Outcome::Won;
    game.hold_end_screen = true;
    let before = pos(&game, enemy);
    let seen = idle(&mut game, ticks(tuning().well_form_seconds + 1.0));
    assert!(pos(&game, enemy).distance_to(before) < 0.5, "nothing pulled");
    assert!(game.held_drums().is_empty() && barrels(&game) == 2, "nothing lifted");
    assert!(!seen.iter().any(|e| matches!(e, Event::Hit { .. })));
    // One that held its drums when the round ended sets them off.
    let mut game = round(drums);
    let enemy = still_enemy(&mut game, centre + Vec2::new(0.0, 80.0), 1, 270.0);
    pulling_well(&mut game, centre);
    assert_eq!(game.held_drums().len(), 2, "both lifted while the round ran");
    game.outcome = Outcome::Won;
    game.hold_end_screen = true;
    let (before, damage) = (pos(&game, enemy), with_tank(&game.world, enemy, |t| t.damage));
    let seen = idle(&mut game, ticks(tuning().well_pull_seconds) + 4);
    assert_eq!(collapsed(&seen).len(), 1, "it collapses as a show");
    assert_eq!(seen.iter().filter(|e| matches!(e, Event::Blast { chained: true, .. })).count(), 2, "its drums go off: {seen:?}");
    assert!(game.held_drums().is_empty());
    assert!(!seen.iter().any(|e| matches!(e, Event::Hit { .. })), "and hurt nobody");
    assert_eq!(with_tank(&game.world, enemy, |t| t.damage), damage);
    assert!(pos(&game, enemy).distance_to(before) < 0.5, "nobody flung");
}

// --- the drain ------------------------------------------------------------

#[test]
fn tread_marks_swirl_in() {
    let mut game = round_with("", Mission::Destroy, None);
    // Lay a trail by driving east past the middle, then stand a well on it.
    game.place_tank(seat(&game), MID - Vec2::new(150.0, 0.0), Some(90.0)).expect("placed");
    for _ in 0..ticks(1.2) {
        step_with(&mut game, Intent { move_dir: Some(Dir::Right), ..Intent::default() });
    }
    game.place_tank(seat(&game), Position::new(96.0, 96.0), Some(90.0)).expect("placed");
    let near: Vec<Position> = game.tracks.iter().map(|t| t.position).filter(|p| p.distance_to(MID) < 100.0).collect();
    assert!(!near.is_empty(), "a trail to drain");
    let d0: f32 = near.iter().map(|p| p.distance_to(MID)).sum::<f32>() / near.len() as f32;
    pulling_well(&mut game, MID);
    idle(&mut game, 60);
    let after: Vec<Position> = game.tracks.iter().map(|t| t.position).filter(|p| p.distance_to(MID) < 100.0).collect();
    let d1: f32 = after.iter().map(|p| p.distance_to(MID)).sum::<f32>() / after.len().max(1) as f32;
    assert!(d1 < d0, "drawn in: {d1} from {d0}");
}

// --- determinism ----------------------------------------------------------

/// A round's RNG after a well pulls a hull and a shot, and collapses - no
/// drum, frog or grenade to roll for: the same as with no well at all.
#[test]
fn the_well_draws_no_rng() {
    let run = |well: bool| {
        let mut game = round_with("", Mission::Destroy, None);
        let enemy = still_enemy(&mut game, MID + Vec2::new(60.0, 0.0), 1, 0.0);
        with_tank_mut(&game.world, enemy, |t| t.damage = -1000.0);
        if well {
            game.debug_well(MID, false).expect("a well");
        }
        idle(&mut game, ticks(tuning().well_form_seconds + tuning().well_pull_seconds) + 10);
        let mut rng = game.rng.clone().expect("the round's rng");
        rand::RngExt::random::<u64>(&mut rng)
    };
    assert_eq!(run(true), run(false));
}

#[test]
fn a_round_with_the_well_replays_bit_for_bit() {
    let run = || {
        let mut game = round("cells.\"15,8\" = { kind = \"barrel\", drum = \"oil\" }\n");
        still_enemy(&mut game, MID + Vec2::new(60.0, 10.0), 0, 0.0);
        still_enemy(&mut game, MID - Vec2::new(70.0, 20.0), 10, 90.0);
        game.place_tank(seat(&game), MID - Vec2::new(250.0, 0.0), Some(90.0)).expect("placed");
        let mut trace = Vec::new();
        for i in 0..ticks(7.0) {
            let fire = i == 0 || i == 90;
            step(&mut game, fire);
            for t in game.world.query::<&Tank>().iter() {
                trace.push((t.position.x.to_bits(), t.position.y.to_bits(), t.damage.to_bits()));
            }
        }
        (trace, format!("{:?}", game.events()))
    };
    assert_eq!(run(), run());
}

// --- the AI ---------------------------------------------------------------

/// A round with two seats, both assaults with nothing special, seat 0 at
/// `a` and seat 1 at `b`, facing north.
fn two_seats(a: Position, b: Position) -> Game {
    two_seats_on("", a, b)
}

/// `two_seats` on an open field plus `extra` cells.
fn two_seats_on(extra: &str, a: Position, b: Position) -> Game {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.show_intro = false;
    game.level_overrides.mission = Some(Mission::Destroy);
    game.players = PlayerCount::from_count(2).expect("two seats");
    let map = format!("version = 1\ntanks = 0\ntank = \"assault\"\ntank2 = \"assault\"\ncells.\"3,6\" = {{ kind = \"start\" }}\ncells.\"3,12\" = {{ kind = \"start2\" }}\n{extra}");
    game.map = MapFile::from_toml_str(&map).expect("test map parses");
    game.init(W, H);
    for (i, at) in [a, b].into_iter().enumerate() {
        let e = game.seat(i).expect("a seat");
        game.place_tank(e, at, Some(0.0)).expect("placed");
        with_tank_mut(&game.world, e, |t| {
            t.disarm();
            t.shells_ammo = 0;
            t.shield_hp = 0.0;
            t.shield_timer = 0.0;
        });
    }
    game
}

/// An enemy of chassis `row` carrying wells at `at` facing `rotation`,
/// thinking, unshielded.
fn well_enemy(game: &mut Game, at: Position, row: i32, rotation: f32) -> Entity {
    let slot = game.debug_spawn_enemy(at, Some(row), Some(Role::Player)).expect("spawns");
    let entity = game.tank_entity_by_slot(slot).expect("exists");
    game.place_tank(entity, at, Some(rotation)).expect("placed");
    with_tank_mut(&game.world, entity, |t| {
        t.disarm();
        t.take_weapon(ActiveWeapon::GravityWell);
        t.shield_hp = 0.0;
        t.shield_timer = 0.0;
    });
    entity
}

/// An enemy of chassis `row` at `at` facing `rotation`, thinking, with
/// nothing to fire.
fn bare_enemy(game: &mut Game, at: Position, row: i32, rotation: f32) -> Entity {
    let slot = game.debug_spawn_enemy(at, Some(row), Some(Role::Player)).expect("spawns");
    let entity = game.tank_entity_by_slot(slot).expect("exists");
    game.place_tank(entity, at, Some(rotation)).expect("placed");
    with_tank_mut(&game.world, entity, |t| {
        t.disarm();
        t.shells_ammo = 0;
        t.shield_hp = 0.0;
        t.shield_timer = 0.0;
    });
    entity
}

fn slot_of(game: &Game, e: Entity) -> usize {
    with_tank(&game.world, e, |t| t.owner_slot())
}

fn snapshot(game: &Game, e: Entity) -> crate::ai::AiSnapshot {
    game.world.get::<&Ai>(e).expect("an enemy").snapshot()
}

#[test]
fn an_enemy_anchors_where_two_seats_are_pulled_together() {
    let mut game = two_seats(MID + Vec2::new(0.0, -40.0), MID + Vec2::new(0.0, 40.0));
    let enemy = well_enemy(&mut game, MID - Vec2::new(300.0, 0.0), 1, 90.0);
    let slot = slot_of(&game, enemy);
    let seen = idle(&mut game, ticks(4.0));
    let launched = seen.iter().any(|e| matches!(e, Event::Fired { slot: s, weapon: "gravity_well" } if *s == slot));
    assert!(launched, "the enemy launches");
    let mine: Vec<(Position, AnchorBy)> =
        seen.iter().filter_map(|e| if let Event::WellAnchored { slot: s, x, y, by, .. } = *e { (s == slot).then_some((Position::new(x, y), by)) } else { None }).collect();
    assert_eq!(mine.len(), 1, "{seen:?}");
    assert_eq!(mine[0].1, AnchorBy::Press, "its rule anchors it");
    let r = tuning().well_radius_px;
    for i in 0..2 {
        let at = pos(&game, game.seat(i).unwrap());
        assert!(at.distance_to(mine[0].0) <= r + 32.0, "seat {i} in the pull: {at:?} from {:?}", mine[0].0);
    }
}

#[test]
fn the_generic_tiers_never_fire_the_well() {
    let mut game = round_with("", Mission::Destroy, None);
    game.place_tank(seat(&game), MID, Some(0.0)).expect("placed");
    let enemy = well_enemy(&mut game, MID - Vec2::new(200.0, 0.0), 1, 90.0);
    let slot = slot_of(&game, enemy);
    let seen = idle(&mut game, ticks(4.0));
    assert!(!seen.iter().any(|e| matches!(e, Event::Fired { slot: s, .. } if *s == slot)), "a lone seat in the open is no use for a well, and no shell is fired");
    assert_eq!(with_tank(&game.world, enemy, |t| t.wells), tuning().well_per_pickup);
}

/// A tank that launched its last well fires its shells at the generic
/// pace once the orb is down, not after the rest of the well's
/// `well_ai_fire_interval`.
#[test]
fn an_enemy_that_spent_its_last_well_fires_its_shells_at_once() {
    let t = tuning();
    let mut game = round_with("cells.\"19,8\" = { kind = \"barrel\", drum = \"oil\" }\n", Mission::Destroy, None);
    game.place_tank(seat(&game), MID, Some(0.0)).expect("placed");
    let enemy = well_enemy(&mut game, MID - Vec2::new(250.0, 0.0), 1, 90.0);
    with_tank_mut(&game.world, enemy, |tank| {
        tank.wells = 1;
        tank.shells_ammo = t.max_shells;
    });
    let slot = slot_of(&game, enemy);
    let (why, _) = launch(&mut game, enemy, 1.0);
    assert!(why.is_some(), "it launches its last well");
    let seen = idle(&mut game, ticks(t.enemy_fire_interval + t.enemy_aim_settle + 2.0));
    assert!(anchors_of(&seen, slot).len() == 1, "its orb anchors: {seen:?}");
    assert!(
        seen.iter().any(|e| matches!(e, Event::Fired { slot: s, weapon: "shell" } if *s == slot)),
        "lined up on the seat, it fires a shell well inside the well's {} s",
        t.well_ai_fire_interval
    );
}

#[test]
fn a_training_dummy_never_fires_a_well() {
    let mut game = two_seats(MID + Vec2::new(0.0, -40.0), MID + Vec2::new(0.0, 40.0));
    let enemy = well_enemy(&mut game, MID - Vec2::new(300.0, 0.0), 1, 90.0);
    game.world.get::<&mut Ai>(enemy).unwrap().frog_only = true;
    let slot = slot_of(&game, enemy);
    let seen = idle(&mut game, ticks(4.0));
    assert!(!seen.iter().any(|e| matches!(e, Event::Fired { slot: s, .. } if *s == slot)));
}

#[test]
fn an_enemy_never_anchors_where_its_pull_drags_more_allies_than_seats() {
    let mut game = two_seats(MID + Vec2::new(0.0, -40.0), MID + Vec2::new(0.0, 40.0));
    let enemy = well_enemy(&mut game, MID - Vec2::new(300.0, 0.0), 1, 90.0);
    // Three allies standing with the seats, between them and the well tank,
    // brains off.
    for dy in [-90.0, 0.0, 90.0] {
        still_enemy(&mut game, MID + Vec2::new(-50.0, dy), 1, 0.0);
    }
    let slot = slot_of(&game, enemy);
    let seen = idle(&mut game, ticks(3.0));
    assert!(!seen.iter().any(|e| matches!(e, Event::WellAnchored { slot: s, .. } if *s == slot)), "{seen:?}");
}

#[test]
fn an_enemy_in_a_pull_drives_across_on_its_own_side() {
    let mut game = round_with("", Mission::Destroy, None);
    pulling_well(&mut game, MID);
    // East of the core and a little south: across is south, its own side.
    let enemy = bare_enemy(&mut game, MID + Vec2::new(60.0, 10.0), 1, 270.0);
    step(&mut game, false);
    step(&mut game, false);
    let snap = snapshot(&game, enemy);
    assert_eq!(snap.pull, Some("across"), "{snap:?}");
    assert_eq!(snap.intent_move, Some("down"));
    idle(&mut game, ticks(2.0));
    assert!(pos(&game, enemy).distance_to(MID) > 40.0, "not drawn into the core: {:?}", pos(&game, enemy));
}

#[test]
fn a_heavy_enemy_braces_broadside() {
    let mut game = round_with("", Mission::Destroy, None);
    pulling_well(&mut game, MID);
    let titan = bare_enemy(&mut game, MID + Vec2::new(60.0, 0.0), TankKind::Titan.row(), 270.0);
    idle(&mut game, 20);
    let snap = snapshot(&game, titan);
    assert_eq!(snap.pull, Some("brace"), "{snap:?}");
    let facing = with_tank(&game.world, titan, |t| Dir::from_rotation(t.rotation));
    assert!(matches!(facing, Some(Dir::Up) | Some(Dir::Down)), "broadside: {facing:?}");
    let a = pos(&game, titan);
    idle(&mut game, 30);
    assert!(pos(&game, titan).distance_to(a) < 2.0, "it holds: {:?} from {a:?}", pos(&game, titan));
}

/// How far the hull `entity`'s movement box reaches into the field's
/// boundary or a standing tile's box: 0 where it is clear of all of them.
fn wall_overlap(game: &Game, entity: Entity) -> f32 {
    let (pos, (hx, hy)) = with_tank(&game.world, entity, |t| (t.position, t.move_half_extents(t.facing_along_x())));
    let overlap = |c: Position, h: Position| {
        let dx = h.x + hx - (pos.x - c.x).abs();
        let dy = h.y + hy - (pos.y - c.y).abs();
        if dx > 0.0 && dy > 0.0 { dx.min(dy) } else { 0.0 }
    };
    let terrain = hits::Terrain::build(&game.world, W, H, &[], &game.water);
    let tiles: Vec<(Position, Position)> =
        game.world.query::<(Entity, &Obstacle)>().iter().filter_map(|(e, _)| terrain.obstacle(e).map(|b| (b.center, b.half))).collect();
    battlefield::wall_rects(W, H).into_iter().chain(tiles).map(|(c, h)| overlap(c, h)).fold(0.0, f32::max)
}

/// The pull is no knock - a current along the tracks and a side pull
/// against their grip, under the body's speed cap - yet it presses hulls
/// into whatever stands between them and the core: a hull dragged north,
/// along its tracks and broadside, at an iron wall and at the field's top
/// edge from every gap up to a few steps' travel, sinks under a pixel into
/// it on the step it first meets the face (without `pull_look_ahead`, up
/// to four and a half) and lies flush against it once the pull and the
/// collapse are over - it never goes through.
#[test]
fn the_pull_presses_a_hull_against_a_wall_and_the_fields_edge_never_into_it() {
    let iron: String = (14..=20).map(|c| format!("cells.\"{c},4\" = {{ kind = \"wall\", material = \"iron\" }}\n")).collect();
    for extra in [String::new(), iron] {
        let game = round(&extra);
        // The face north of the seat: the iron's south face, or the field's
        // top edge.
        let face = if extra.is_empty() { 0.0 } else { crate::map::cell_to_world(14, 4).y + 12.0 };
        for rotation in [0.0, 90.0] {
            let up = with_tank(&game.world, seat(&game), |t| t.move_half_extents(rotation == 90.0).1);
            for gap in [0, 1, 2, 3, 4, 6, 8, 12, 16, 20, 24, 28, 32, 40, 48, 56] {
                let mut game = round(&extra);
                let s = seat(&game);
                with_tank_mut(&game.world, s, |t| t.disarm());
                let at = Position::new(544.0, face + up + 0.5 + gap as f32);
                game.place_tank(s, at, Some(rotation)).expect("placed");
                // The core beyond the face, the seat inside its reach.
                let core = Position::new(544.0, (face - 40.0).max(4.0));
                game.debug_well(core, true).expect("a well");
                let mut pulled = false;
                let mut overlap = 0.0;
                for _ in 0..ticks(tuning().well_form_seconds + tuning().well_pull_seconds + 1.0) {
                    step(&mut game, false);
                    let p = with_tank(&game.world, s, |t| t.position);
                    pulled |= game.in_a_pull(p);
                    overlap = wall_overlap(&game, s);
                    assert!(overlap <= 1.0, "{} rotation {rotation}: the hull {overlap} px in at {p:?}, from {at:?}", if extra.is_empty() { "the edge" } else { "iron" });
                }
                assert!(pulled, "the well pulls the seat from {at:?}");
                assert!(overlap <= 0.05, "and lies flush at the end, {overlap} px in, from {at:?}");
            }
        }
    }
}

// --- online in the simulation ---------------------------------------------

/// The pose validator allows a client-owned hull the pull a well puts on it
/// and no more (docs/gravity-well.md "Online"): a pose carried toward the
/// core past the chassis's own reach is taken; one as far the other way,
/// against the pull, is refused; one past the solver's speed cap - which no
/// pull carries a hull beyond - is refused, though counting the whole side
/// pull in any direction would have taken it; and away from every well the
/// chassis's own reach holds.
#[test]
fn an_owned_hull_is_allowed_the_pull_and_no_more() {
    let t = tuning();
    let mut game = round_with("", Mission::Destroy, None);
    pulling_well(&mut game, MID);
    let s = seat(&game);
    let start = MID + Vec2::new(40.0, 0.0);
    let n = POSE_REACH_TICKS;
    let (speed, mass_factor, grip) = with_tank(&game.world, s, |tank| (tank.effective_speed(), tank.mass_factor(), t.tank_turn_grip_force / tank.mass()));
    let cap = game.physics.max_speed();
    let own = speed * DT * n + POSE_REACH_SLACK_PX;
    let pull = game.well_field.hull_pull(start, mass_factor, &t);
    assert!(pull.current.length() * DT * n > 8.0, "the case holds at the defaults: {pull:?}");
    assert!(pull.side.length() <= grip, "and the seat's grip holds the side pull: {pull:?} against {grip}");
    let pose = |game: &mut Game, from: Position, by: Vec2| {
        game.place_tank(s, from, Some(270.0)).expect("placed");
        game.accept_seat_pose(0, SeatPose { position: from + by, rotation: 270.0, velocity: Vec2::zero() }, n as u32)
    };
    // Toward the core, past its own reach and within the current's.
    let toward = Vec2::new(-(own + 6.0), 0.0);
    assert!(pose(&mut game, start, toward).is_ok(), "the pull carries it past its own reach");
    assert!(pose(&mut game, start, toward * -1.0).is_err(), "against the pull, its own reach holds");
    assert!(pose(&mut game, start, Vec2::new(0.0, own - 1.0)).is_ok(), "across, its own reach still");
    // Past the speed cap toward the core - where the whole side pull,
    // counted in any direction, allowed it.
    let past_cap = cap * DT * n + POSE_REACH_SLACK_PX + 4.0;
    let whole = (speed + pull.current.length() + pull.side.length() * WELL_SIDE_REACH_SECONDS) * DT * n + POSE_REACH_SLACK_PX;
    assert!(whole > past_cap, "the whole side pull would have taken {past_cap}: {whole}");
    assert!(pose(&mut game, start, Vec2::new(-past_cap, 0.0)).is_err(), "past the speed cap toward the core");
    // Out of every well's reach, the chassis's own.
    let far = MID + Vec2::new(t.well_radius_px + 80.0, 0.0);
    assert!(pose(&mut game, far, toward).is_err(), "no pull, no allowance");
}

// --- the AI's arms and its pull -------------------------------------------

/// Steps `game` up to `seconds` until `enemy` launches an orb, its fire
/// timer run out first so it may launch on its first think: the arm its
/// rule launched it for (`AiSnapshot::special` on that tick), and every
/// event seen.
fn launch(game: &mut Game, enemy: Entity, seconds: f32) -> (Option<&'static str>, Vec<Event>) {
    game.world.get::<&mut Ai>(enemy).unwrap().set_fire_timer(0.0);
    let slot = slot_of(game, enemy);
    let mut seen = Vec::new();
    for _ in 0..ticks(seconds) {
        let events = step(game, false);
        let fired = events.iter().any(|e| matches!(e, Event::Fired { slot: s, weapon: "gravity_well" } if *s == slot));
        seen.extend(events);
        if fired {
            return (snapshot(game, enemy).special, seen);
        }
    }
    (None, seen)
}

fn anchors_of(seen: &[Event], slot: usize) -> Vec<(Position, AnchorBy)> {
    seen.iter().filter_map(|e| if let Event::WellAnchored { slot: s, x, y, by, .. } = *e { (s == slot).then_some((Position::new(x, y), by)) } else { None }).collect()
}

/// Its last orb is still its rule's to anchor: with its wells spent the
/// trigger would fire shells, yet the rule holds it released until the orb
/// has flown its plan and presses then - no shell in between.
#[test]
fn an_enemy_anchors_its_last_orb_by_its_rule() {
    let mut game = two_seats(MID + Vec2::new(0.0, -40.0), MID + Vec2::new(0.0, 40.0));
    let enemy = well_enemy(&mut game, MID - Vec2::new(300.0, 0.0), 1, 90.0);
    with_tank_mut(&game.world, enemy, |t| {
        t.wells = 1;
        t.shells_ammo = tuning().max_shells;
    });
    let slot = slot_of(&game, enemy);
    let (why, mut seen) = launch(&mut game, enemy, 2.0);
    assert_eq!(why, Some("clump"));
    assert_eq!(with_tank(&game.world, enemy, |t| t.wells), 0, "its last");
    let launched = seen.len();
    let mut pressed_by = None;
    for _ in 0..ticks(3.0) {
        let events = step(&mut game, false);
        if events.iter().any(|e| matches!(e, Event::WellAnchored { slot: s, .. } if *s == slot)) {
            pressed_by = Some(snapshot(&game, enemy).special);
        }
        seen.extend(events);
    }
    let mine = anchors_of(&seen, slot);
    assert_eq!(mine.len(), 1, "{seen:?}");
    assert_eq!(mine[0].1, AnchorBy::Press);
    assert_eq!(pressed_by, Some(Some("anchor")), "its rule's press, not a shell's");
    let anchored_at = seen.iter().position(|e| matches!(e, Event::WellAnchored { slot: s, .. } if *s == slot)).unwrap();
    assert!(
        !seen[launched..anchored_at].iter().any(|e| matches!(e, Event::Fired { slot: s, .. } if *s == slot)),
        "no shell between the launch and the anchor"
    );
    let r = tuning().well_radius_px;
    for i in 0..2 {
        let at = pos(&game, game.seat(i).unwrap());
        assert!(at.distance_to(mine[0].0) <= r + 32.0, "seat {i} in the pull: {at:?} from {:?}", mine[0].0);
    }
}

/// A seat pulled into drums is trouble: one seat in the open, a drum beside
/// it, an enemy carrying wells across the field - it launches for it.
#[test]
fn an_enemy_anchors_to_pull_a_seat_into_drums() {
    let mut game = round_with("cells.\"19,8\" = { kind = \"barrel\", drum = \"oil\" }\n", Mission::Destroy, None);
    game.place_tank(seat(&game), MID, Some(0.0)).expect("placed");
    let enemy = well_enemy(&mut game, MID - Vec2::new(300.0, 0.0), 1, 90.0);
    let (why, _) = launch(&mut game, enemy, 2.0);
    assert_eq!(why, Some("trouble"));
}

/// A seat pulled off the frog it guards: the frog west of the seat, the
/// enemy east of it - it anchors past the seat, away from the frog, the
/// frog itself out of the pull.
#[test]
fn an_enemy_anchors_to_pull_a_guard_off_its_frog() {
    let mut game = round_with("cells.\"8,8\" = { kind = \"frog\" }\n", Mission::Protect, None);
    let frog = crate::map::cell_to_world(8, 8);
    let guard = frog + Vec2::new(96.0, 0.0);
    game.place_tank(seat(&game), guard, Some(0.0)).expect("placed");
    let enemy = well_enemy(&mut game, guard + Vec2::new(300.0, 0.0), 1, 270.0);
    let slot = slot_of(&game, enemy);
    let (why, mut seen) = launch(&mut game, enemy, 2.0);
    assert_eq!(why, Some("guard"));
    seen.extend(idle(&mut game, ticks(2.5)));
    let mine = anchors_of(&seen, slot);
    assert_eq!(mine.len(), 1, "{seen:?}");
    assert!(mine[0].0.distance_to(frog) > tuning().well_radius_px + tuning().well_ai_friend_margin_px, "the frog out of the pull");
    assert!(mine[0].0.distance_to(frog) > guard.distance_to(frog), "past the guard");
}

/// Under fire, an enemy puts a well across the line a seat shoots it along,
/// and the seat's next shell falls into its core.
#[test]
fn an_enemy_under_fire_shields_itself_on_the_seats_line() {
    let mut game = round_with("", Mission::Destroy, None);
    // The seat at (3, 6) faces east, lined up on the enemy.
    let enemy = well_enemy(&mut game, SEAT + Vec2::new(340.0, 0.0), 1, 270.0);
    game.world.get::<&mut Ai>(enemy).unwrap().notify_hit();
    let (why, _) = launch(&mut game, enemy, 1.0);
    assert_eq!(why, Some("shield"));
    let slot = slot_of(&game, enemy);
    let mut seen = idle(&mut game, ticks(2.0));
    assert_eq!(anchors_of(&seen, slot).len(), 1, "{seen:?}");
    with_tank_mut(&game.world, seat(&game), |t| {
        t.shells_ammo = tuning().max_shells;
        t.fire_cooldown = 0.0;
    });
    seen.extend(step(&mut game, true));
    seen.extend(idle(&mut game, ticks(1.5)));
    assert!(swallowed(&seen).contains(&Swallow::Shell), "the seat's shell is swallowed: {seen:?}");
    assert!(!seen.iter().any(|e| matches!(e, Event::Hit { target: HitTarget::Enemy { .. }, .. })), "and never reaches it");
}

/// No well is used on a seat from outside its sight box: the clump the
/// rule would take stands in range of the orb, the enemy a little past the
/// seats' boxes.
#[test]
fn an_enemy_never_uses_a_well_on_a_seat_from_outside_its_sight_box() {
    let (half_w, _) = tuning().sight_box_half_px();
    let mut game = two_seats(MID + Vec2::new(0.0, -40.0), MID + Vec2::new(0.0, 40.0));
    let enemy = well_enemy(&mut game, MID - Vec2::new(half_w + 24.0, 0.0), 1, 90.0);
    let slot = slot_of(&game, enemy);
    idle(&mut game, 2);
    assert_eq!(snapshot(&game, enemy).well, None, "no plan from off the box");
    let (why, seen) = launch(&mut game, enemy, 0.5);
    assert!(why.is_none() && !seen.iter().any(|e| matches!(e, Event::Fired { slot: s, weapon: "gravity_well" } if *s == slot)), "{seen:?}");
}

/// A seat hidden in tall grass is not counted while the enemy has not been
/// shot at.
#[test]
fn a_seat_hidden_in_grass_is_not_counted() {
    let grass = "cells.\"17,7\" = { kind = \"tall_grass\" }\ncells.\"17,9\" = { kind = \"tall_grass\" }\n";
    let mut game = two_seats_on(grass, MID + Vec2::new(0.0, -40.0), MID + Vec2::new(0.0, 40.0));
    let enemy = well_enemy(&mut game, MID - Vec2::new(300.0, 0.0), 1, 90.0);
    let (why, _) = launch(&mut game, enemy, 1.0);
    assert_eq!(why, None, "both seats hidden: nothing to pull together");
    game.world.get::<&mut Ai>(enemy).unwrap().notify_hit();
    let (why, _) = launch(&mut game, enemy, 1.0);
    assert_eq!(why, Some("clump"), "shot at, it counts them");
}

/// One well a seat: two enemies carrying wells on the same two seats - the
/// second leaves them to the first's orb in flight, never stacking a well.
#[test]
fn two_well_tanks_never_stack_on_one_seat() {
    let mut game = two_seats(MID + Vec2::new(0.0, -40.0), MID + Vec2::new(0.0, 40.0));
    let a = well_enemy(&mut game, MID - Vec2::new(300.0, 0.0), 1, 90.0);
    let b = well_enemy(&mut game, MID - Vec2::new(300.0, 60.0), 1, 90.0);
    let seen = idle(&mut game, ticks(4.0));
    let launches = |e: Entity| {
        let slot = slot_of(&game, e);
        seen.iter().filter(|ev| matches!(ev, Event::Fired { slot: s, weapon: "gravity_well" } if *s == slot)).count()
    };
    assert_eq!(launches(a) + launches(b), 1, "one well on the pair: {seen:?}");
}

/// A braced heavy is still a gun: broadside in a pull, a seat lined up
/// along its facing, it fires on it as the attack tier would.
#[test]
fn a_braced_heavy_still_fires_on_a_seat_lined_up() {
    let mut game = round_with("", Mission::Destroy, None);
    pulling_well(&mut game, MID);
    // The seat south of the titan, outside the pull, facing it.
    game.place_tank(seat(&game), MID + Vec2::new(60.0, 200.0), Some(0.0)).expect("placed");
    let titan = bare_enemy(&mut game, MID + Vec2::new(60.0, 0.0), TankKind::Titan.row(), 180.0);
    with_tank_mut(&game.world, titan, |t| t.shells_ammo = tuning().max_shells);
    let slot = slot_of(&game, titan);
    let mut braced = false;
    let mut seen = Vec::new();
    for _ in 0..ticks(1.5) {
        seen.extend(step(&mut game, false));
        braced |= snapshot(&game, titan).pull == Some("brace");
    }
    assert!(braced, "it braces");
    assert!(seen.iter().any(|e| matches!(e, Event::Fired { slot: s, weapon: "shell" } if *s == slot)), "and fires: {seen:?}");
}

/// A heavy chassis braces only while its tracks hold: deep enough in that
/// the side pull beats its grip, it drives across like any other.
#[test]
fn a_heavy_enemy_escapes_once_its_tracks_cannot_hold() {
    let t = tuning();
    let breaker = TankKind::Breaker.row();
    let mut game = round_with("", Mission::Destroy, None);
    pulling_well(&mut game, MID);
    let enemy = bare_enemy(&mut game, MID + Vec2::new(90.0, 0.0), breaker, 0.0);
    let (mass_factor, grip) = with_tank(&game.world, enemy, |tank| (tank.mass_factor(), t.tank_turn_grip_force / tank.mass()));
    assert!(mass_factor >= t.well_ai_heavy_mass, "a heavy chassis");
    let field = game.well_field.clone();
    let deep = (t.well_core_px as i32..t.well_radius_px as i32)
        .map(|d| d as f32)
        .find(|&d| !crate::well::holds_broadside(&field, MID + Vec2::new(d, 0.0), mass_factor, grip, &t))
        .expect("somewhere its tracks do not hold");
    let shallow = (deep + 30.0).min(t.well_radius_px - 8.0);
    game.place_tank(enemy, MID + Vec2::new(shallow, 0.0), Some(0.0)).expect("placed");
    step(&mut game, false);
    step(&mut game, false);
    assert_eq!(snapshot(&game, enemy).pull, Some("brace"), "where they hold, it braces");
    game.place_tank(enemy, MID + Vec2::new(deep, 0.0), Some(0.0)).expect("placed");
    step(&mut game, false);
    step(&mut game, false);
    assert_eq!(snapshot(&game, enemy).pull, Some("across"), "deeper in, it escapes");
}

/// A special that is only down keeps its pacing: an EMP enemy just after
/// its pulse, its emitter offline and its rule's interval running, holds
/// its shells through the outage though it stands lined up on a seat.
#[test]
fn an_offline_emp_enemy_holds_its_shells_through_the_outage() {
    let t = tuning();
    let mut game = round_with("", Mission::Destroy, None);
    game.place_tank(seat(&game), MID, Some(0.0)).expect("placed");
    let enemy = bare_enemy(&mut game, MID - Vec2::new(200.0, 0.0), 1, 90.0);
    with_tank_mut(&game.world, enemy, |tank| {
        tank.take_weapon(ActiveWeapon::Emp);
        tank.shells_ammo = t.max_shells;
        tank.special_offline = t.emp_disable_seconds;
    });
    assert!(t.emp_ai_fire_interval > t.emp_disable_seconds, "the case holds at the defaults");
    game.world.get::<&mut Ai>(enemy).unwrap().set_fire_timer(t.emp_ai_fire_interval);
    let slot = slot_of(&game, enemy);
    let seen = idle(&mut game, ticks(t.emp_disable_seconds - 0.1));
    assert!(!seen.iter().any(|e| matches!(e, Event::Fired { slot: s, .. } if *s == slot)), "{seen:?}");
}

/// Routes go round a well: every nav cell inside a forming or pulling
/// well's reach costs `well_ai_route_cost` more, none outside it.
#[test]
fn routes_go_round_a_well() {
    let t = tuning();
    let mut game = round_with("", Mission::Destroy, None);
    let cell = |p: Position| ((p.x / crate::PATHFIND_CELL_SIZE) as usize, (p.y / crate::PATHFIND_CELL_SIZE) as usize);
    let (inside, outside) = (cell(MID + Vec2::new(40.0, 0.0)), cell(MID + Vec2::new(t.well_radius_px + 48.0, 0.0)));
    let before = game.route_grid(W, H);
    game.debug_well(MID, false).expect("a well");
    let after = game.route_grid(W, H);
    assert_eq!(after.cost_at(inside.0, inside.1), before.cost_at(inside.0, inside.1) + t.well_ai_route_cost as u32);
    assert_eq!(after.cost_at(outside.0, outside.1), before.cost_at(outside.0, outside.1));
}


// --- what it bends, swallows, lifts and leaves ----------------------------

fn shell_positions(game: &Game) -> Vec<Position> {
    game.world.query::<&Shell>().iter().filter(|s| s.is_flying()).map(|s| s.position).collect()
}

/// A shell a well bends into a tank hits it: each tick's stretch of the
/// curve is swept. The tank stands on the curve the same shot flies with
/// nobody there, past the well's reach and off the straight line.
#[test]
fn a_shot_bent_into_a_tank_hits_it() {
    let t = tuning();
    let core = Position::new(352.0, SEAT.y + 96.0);
    let fly = |enemy_at: Option<Position>| {
        let mut game = round_with("", Mission::Destroy, None);
        pulling_well(&mut game, core);
        if let Some(at) = enemy_at {
            still_enemy(&mut game, at, 1, 0.0);
        }
        with_tank_mut(&game.world, seat(&game), |tank| tank.fire_cooldown = 0.0);
        let mut seen = step(&mut game, true);
        let mut trail = Vec::new();
        for _ in 0..90 {
            seen.extend(step(&mut game, false));
            trail.extend(shell_positions(&game));
        }
        (trail, seen)
    };
    let (trail, seen) = fly(None);
    assert!(swallowed(&seen).is_empty(), "the shot passes the core");
    let entered = trail.iter().position(|p| p.distance_to(core) < t.well_radius_px).expect("the shot crosses the pull");
    let target = *trail[entered..].iter().find(|p| p.distance_to(core) > t.well_radius_px + 48.0).expect("and leaves it");
    assert!((target.y - SEAT.y).abs() > 20.0, "off the straight line: {target:?}");
    let (_, seen) = fly(Some(target));
    assert!(seen.iter().any(|e| matches!(e, Event::Hit { target: HitTarget::Enemy { .. }, .. })), "the bent shot hits it: {seen:?}");
}

/// A hull a well holds at its core is hit by a shot aimed into the clump:
/// the sweep meets the hull before the core swallows the shot.
#[test]
fn a_clump_at_the_core_is_hit_before_the_core_swallows() {
    let mut game = round_with("", Mission::Destroy, None);
    let core = Position::new(352.0, SEAT.y);
    still_enemy(&mut game, core, 1, 0.0);
    pulling_well(&mut game, core);
    with_tank_mut(&game.world, seat(&game), |tank| tank.fire_cooldown = 0.0);
    let mut seen = step(&mut game, true);
    seen.extend(idle(&mut game, 40));
    assert!(seen.iter().any(|e| matches!(e, Event::Hit { target: HitTarget::Enemy { .. }, .. })), "{seen:?}");
    assert!(swallowed(&seen).is_empty(), "nothing swallowed");
}

/// The laser is an instant trace: a beam past a well runs straight.
#[test]
fn the_laser_is_not_bent() {
    let mut game = round_with("", Mission::Destroy, Some(ActiveWeapon::Laser));
    pulling_well(&mut game, Position::new(352.0, SEAT.y + 40.0));
    let mut seen = Vec::new();
    for _ in 0..30 {
        seen.extend(step(&mut game, true));
    }
    // Drawn from the module's lens, judged along the gun line: its end
    // stands on the gun line's row, far past the well.
    let (y1, x1) = seen.iter().find_map(|e| if let Event::LaserBeam { y1, x1, .. } = *e { Some((y1, x1)) } else { None }).expect("a beam");
    assert!((y1 - SEAT.y).abs() < 1.0, "straight down the gun line: ends at y {y1}");
    assert!(x1 > 352.0 + tuning().well_radius_px, "past the well: {x1}");
}

/// A missile in a pull is dragged toward the core, and one whose ground
/// point reaches the core is swallowed with no blast.
#[test]
fn a_missile_is_dragged_and_one_reaching_the_core_is_swallowed_without_a_blast() {
    let mut game = round_with("", Mission::Destroy, None);
    pulling_well(&mut game, MID);
    let missile = |game: &mut Game, from: Position| {
        let mut m = crate::missile::Missile::spawn(from, Vec2::new(1.0, 0.0), Owner::Enemy(MAX_SEATS), 0, from + Vec2::new(400.0, 0.0));
        m.id = game.take_shot_id();
        game.world.spawn((m,))
    };
    let past = missile(&mut game, MID + Vec2::new(-80.0, 70.0));
    let into = missile(&mut game, MID - Vec2::new(40.0, 0.0));
    step(&mut game, false);
    step(&mut game, false);
    let dir = game.world.get::<&crate::missile::Missile>(past).unwrap().dir;
    assert!(dir.y < 0.0, "turned toward the core: {dir:?}");
    let seen = idle(&mut game, 40);
    assert!(swallowed(&seen).contains(&Swallow::Missile), "{seen:?}");
    assert!(!seen.iter().any(|e| matches!(e, Event::MissileBlast { x, .. } if (*x - MID.x).abs() < 40.0)), "no blast at the core");
    assert!(game.world.get::<&crate::missile::Missile>(into).is_err(), "gone");
}

/// A drone in a pull whose ground point reaches the core is downed there
/// (`AirStrike::Well`), and swallowed.
#[test]
fn a_drone_is_downed_at_the_core() {
    let mut game = round_with("", Mission::Destroy, None);
    pulling_well(&mut game, MID);
    let mut drone = crate::fpv::Drone::launch(MID + Vec2::new(4.0, 0.0), Vec2::new(1.0, 0.0), 0, Owner::Enemy(MAX_SEATS), crate::fpv::DroneLock::None, MID + Vec2::new(300.0, 0.0));
    drone.id = game.take_shot_id();
    let id = drone.id;
    game.world.spawn((drone,));
    let seen = idle(&mut game, 3);
    assert!(seen.iter().any(|e| matches!(e, Event::DroneDowned { id: d, by: "well", .. } if *d == id)), "{seen:?}");
    assert!(swallowed(&seen).contains(&Swallow::Drone));
}

/// A drum already burning is lifted with its fuse, which burns on in the
/// air: it goes off when the fuse ends, before the collapse. Its cell opens
/// to the router.
#[test]
fn a_fused_drum_is_lifted_with_its_fuse_and_its_cell_opens() {
    let mut game = round("cells.\"15,8\" = { kind = \"barrel\", drum = \"fuel\" }\n");
    let drum_at = crate::map::cell_to_world(15, 8);
    let fuse = 1.0;
    for o in game.world.query_mut::<&mut Obstacle>() {
        if o.material.is_explosive() {
            o.fuse = Some(crate::obstacle::Fuse { left: fuse, total: fuse, from: None });
        }
    }
    assert!(!game.nav_grid(W, H).usable(drum_at), "the drum blocks its cell");
    let centre = crate::map::cell_to_world(17, 8);
    game.debug_well(centre, false).expect("a well");
    let mut seen = idle(&mut game, ticks(tuning().well_form_seconds) + 2);
    let held = game.held_drums().first().copied().expect("lifted");
    assert!(held.fuse.is_some_and(|left| left > 0.0 && left < fuse), "with its fuse: {held:?}");
    assert!(game.nav_grid(W, H).usable(drum_at), "its cell open");
    seen.extend(idle(&mut game, ticks(fuse)));
    assert!(seen.iter().any(|e| matches!(e, Event::Blast { chained: true, .. })), "it went off in the air");
    assert!(game.held_drums().is_empty());
    assert!(collapsed(&seen).is_empty(), "before the collapse");
}

/// A crate drawn toward the core stops where its centre would enter a
/// wall's cell, and is taken where it lies.
#[test]
fn a_crate_stops_at_a_wall_and_is_taken_where_it_lies() {
    let mut game = round("cells.\"20,8\" = { kind = \"pickup\", pickup = \"health\" }\ncells.\"18,8\" = { kind = \"wall\", material = \"iron\" }\n");
    let slot = crate::map::cell_to_world(20, 8);
    pulling_well(&mut game, crate::map::cell_to_world(17, 8));
    idle(&mut game, ticks(2.0));
    let at = game.world.query::<&Pickup>().iter().next().map(|p| p.at()).expect("the crate");
    assert!(at.x < slot.x - 8.0, "drawn in: {at:?}");
    let wall_edge = crate::map::cell_to_world(18, 8).x + 16.0;
    assert!(at.x >= wall_edge, "and stopped short of the wall: {at:?}");
    // The seat driven onto where it lies now - not its slot - takes it.
    with_tank_mut(&game.world, seat(&game), |tank| tank.damage = 40.0);
    game.place_tank(seat(&game), at + Vec2::new(0.0, 30.0), Some(0.0)).expect("placed");
    let seen = idle(&mut game, 2);
    assert!(seen.iter().any(|e| matches!(e, Event::PickupCollected { .. })), "taken where it lies: {seen:?}");
}

/// A rainbow shield soaks the collapse's damage; the fling lands whole.
#[test]
fn a_shield_soaks_the_collapse_not_the_fling() {
    let mut game = round("");
    let enemy = still_enemy(&mut game, MID + Vec2::new(40.0, 0.0), 1, 0.0);
    with_tank_mut(&game.world, enemy, |tank| tank.raise_shield());
    pulling_well(&mut game, MID);
    let (damage, shield) = with_tank(&game.world, enemy, |tank| (tank.damage, tank.shield_hp));
    let mut seen = Vec::new();
    for _ in 0..ticks(tuning().well_pull_seconds) + 4 {
        seen = step(&mut game, false);
        if !collapsed(&seen).is_empty() {
            break;
        }
    }
    assert_eq!(collapsed(&seen).len(), 1);
    let (after, left, skid) = with_tank(&game.world, enemy, |tank| (tank.damage, tank.shield_hp, tank.skid));
    assert_eq!(after, damage, "the hull unhurt");
    assert!(left < shield, "the shield took it: {left} from {shield}");
    assert!(skid > 0.0, "and the fling landed");
}

/// What flies in a well's reach when it collapses is turned straight out
/// from its centre at its own speed.
#[test]
fn what_flies_in_the_reach_is_turned_out() {
    let mut game = round("");
    let id = pulling_well(&mut game, MID);
    let until = game.zones.iter().find(|z| z.id == id).unwrap().until;
    while game.time + 3.0 * DT < until {
        step(&mut game, false);
    }
    let from = MID + Vec2::new(60.0, 0.0);
    let velocity = Vec2::new(0.0, -tuning().shell_speed);
    let shot_id = game.take_shot_id();
    let shell = Shell::at(shot_id, from, from, velocity, 0.0, 0, 1, Owner::Enemy(MAX_SEATS));
    let entity = game.world.spawn((shell,));
    let mut seen = Vec::new();
    for _ in 0..6 {
        seen.extend(step(&mut game, false));
    }
    assert_eq!(collapsed(&seen).len(), 1);
    let (position, velocity) = game.world.get::<&Shell>(entity).map(|s| (s.position, s.velocity)).expect("still flying");
    let out = position - MID;
    assert!(velocity.x * out.x + velocity.y * out.y > 0.9 * velocity.length() * out.length(), "straight out: {velocity:?} at {position:?}");
    assert!((velocity.length() - tuning().shell_speed).abs() < 0.5, "at its own speed");
}

/// Two wells: a drum in reach of both when their pulls start goes to the
/// lower id, and stays that well's.
#[test]
fn two_wells_pull_together_and_each_keeps_what_it_captured() {
    let mut game = round("cells.\"17,8\" = { kind = \"barrel\", drum = \"oil\" }\n");
    let (a, _) = game.debug_well(crate::map::cell_to_world(15, 8), false).expect("a well");
    let (b, _) = game.debug_well(crate::map::cell_to_world(19, 8), false).expect("another");
    assert!(a < b);
    idle(&mut game, ticks(tuning().well_form_seconds) + 2);
    let held = game.held_drums().first().copied().expect("lifted");
    assert_eq!(held.well, a, "the lower id's");
    idle(&mut game, 30);
    assert_eq!(game.held_drums().first().map(|d| d.well), Some(a), "and kept");
}

/// Knocked off its tracks, a hull in a pull is carried by the whole
/// current: one broadside that holds on its tracks slides in on a skid.
#[test]
fn a_skidding_hull_is_carried_by_the_whole_current() {
    let t = tuning();
    let run = |skid: f32| {
        let mut game = round_with("", Mission::Destroy, None);
        pulling_well(&mut game, MID);
        let s = seat(&game);
        game.place_tank(s, MID + Vec2::new(100.0, 0.0), Some(0.0)).expect("placed");
        let wells = game.well_field.clone();
        let x0 = pos(&game, s).x;
        for _ in 0..20 {
            let Game { world, physics, .. } = &mut game;
            let mut tank = world.get::<&mut Tank>(s).unwrap();
            tank.skid = skid;
            let fp = Footing::DRY.pulled(&wells, &tank);
            drive_tank(physics, &mut tank, Intent::default(), DT, fp);
            physics.step();
            tank.position = physics.position(tank.body.unwrap());
        }
        x0 - pos(&game, s).x
    };
    assert!(run(0.0).abs() < 0.5, "on its tracks it holds");
    assert!(run(1.0) > 2.0 * t.well_current_speed * crate::well::strength(100.0, &t) * DT, "skidding it is carried in");
}

/// The spawn swap hands the well out by its share, by a hash, never at
/// the default.
#[test]
fn the_spawn_swap_hands_out_the_well_by_its_share() {
    let at = Position::new(400.0, 200.0);
    let mut t = crate::tuning::Tuning::DEFAULT;
    let mut laser = Tank::default();
    laser.take_weapon(ActiveWeapon::Laser);
    super::sonic::swap_spawn_special_with(&t, &mut laser, 5, at);
    assert_eq!(laser.special(), Some(ActiveWeapon::Laser), "nothing at the default share");
    t.enemy_special_weapon_well_share = 1.0;
    super::sonic::swap_spawn_special_with(&t, &mut laser, 5, at);
    assert_eq!(laser.special(), Some(ActiveWeapon::GravityWell));
    assert_eq!(laser.wells, t.well_per_pickup);
    let mut shells = Tank::default();
    super::sonic::swap_spawn_special_with(&t, &mut shells, 5, at);
    assert_eq!(shells.special(), None, "a tank with no special gets none");
}

/// A well crate loads three wells and replaces the special carried; a
/// second refills to three; an enemy takes it only while it carries none.
#[test]
fn a_well_crate_arms_the_well_and_replaces_the_special_carried() {
    let mut tank = Tank::default();
    tank.take_weapon(ActiveWeapon::Missiles);
    tank.take_weapon(ActiveWeapon::GravityWell);
    assert_eq!((tank.special(), tank.wells, tank.missile_ammo), (Some(ActiveWeapon::GravityWell), tuning().well_per_pickup, 0));
    tank.wells = 1;
    tank.take_weapon(ActiveWeapon::GravityWell);
    assert_eq!(tank.wells, tuning().well_per_pickup, "refilled");
    let mut enemy = Tank { owner: Owner::Enemy(4), ..Tank::default() };
    assert!(enemy.wants_pickup(PickupKind::GravityWell));
    enemy.take_weapon(ActiveWeapon::Plasma);
    assert!(!enemy.wants_pickup(PickupKind::GravityWell));
}

/// Whether a seat of chassis `kind` standing `d` px east of a pulling
/// well's core, flooring it along `dir`, is out of the reach within
/// `seconds`.
fn seat_escapes(kind: TankKind, d: f32, dir: Dir, seconds: f32) -> bool {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.show_intro = false;
    game.level_overrides.mission = Some(Mission::Destroy);
    let map = format!("version = 1\ntanks = 0\ntank = \"{}\"\ncells.\"3,6\" = {{ kind = \"start\" }}\n", kind.name());
    game.map = MapFile::from_toml_str(&map).expect("test map parses");
    game.init(W, H);
    pulling_well(&mut game, MID);
    let s = seat(&game);
    game.place_tank(s, MID + Vec2::new(d, 0.0), Some(if dir == Dir::Right { 90.0 } else { 0.0 })).expect("placed");
    (0..ticks(seconds)).any(|_| {
        step_with(&mut game, Intent { move_dir: Some(dir), ..Intent::default() });
        pos(&game, s).distance_to(MID) > tuning().well_radius_px
    })
}

/// "Drive across the pull, not away from it" by chassis class at the
/// defaults (docs/gravity-well.md "What it pulls"): a light chassis deep
/// in is carried in driving away and out driving across; a standard one
/// likewise further in; a heavy one is never caught - it drives out either
/// way, the brace being the AI's choice to stand.
#[test]
fn each_chassis_class_escapes_a_pull_as_the_doc_says() {
    assert!(!seat_escapes(TankKind::Scout, 65.0, Dir::Right, 3.0), "a scout driving away is carried in");
    assert!(seat_escapes(TankKind::Scout, 65.0, Dir::Up, 1.5), "a scout driving across is out");
    assert!(!seat_escapes(TankKind::Scout, 40.0, Dir::Up, 3.0), "deep in, not even across");
    assert!(!seat_escapes(TankKind::Assault, 30.0, Dir::Right, 3.0), "a standard chassis driving away is carried in");
    assert!(seat_escapes(TankKind::Assault, 30.0, Dir::Up, 2.0), "and out driving across");
    assert!(seat_escapes(TankKind::Titan, 15.0, Dir::Right, 1.0) && seat_escapes(TankKind::Titan, 15.0, Dir::Up, 1.0), "a titan drives out either way");
}

// --- what a seat is told --------------------------------------------------

/// Off the screen a seat is shown every well but its own: an enemy's with
/// its seconds once it pulls.
#[test]
fn a_seat_is_shown_every_well_but_its_own() {
    let mut game = round("");
    game.debug_well(MID, false).expect("the seat's own");
    let (enemy, _) = game.debug_well(MID + Vec2::new(300.0, 0.0), true).expect("an enemy's");
    let scene = crate::indicators::Scene::of(&game, 0);
    assert_eq!(scene.wells.len(), 1, "{:?}", scene.wells);
    assert!(scene.wells[0].1.is_none(), "forming: no seconds yet");
    idle(&mut game, ticks(tuning().well_form_seconds) + 2);
    let scene = crate::indicators::Scene::of(&game, 0);
    let centre = game.zones.iter().find(|z| z.id == enemy).unwrap().centre;
    assert_eq!(scene.wells.first().map(|w| w.0), Some(centre));
    assert!(scene.wells[0].1.is_some_and(|left| left > 0.0 && left < tuning().well_pull_seconds), "pulling: its seconds left");
}
