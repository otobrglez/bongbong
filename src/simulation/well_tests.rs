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

#[test]
fn the_well_on_the_end_screen_pulls_nothing_and_its_drums_hurt_nobody() {
    let mut game = round("");
    let enemy = still_enemy(&mut game, MID + Vec2::new(80.0, 0.0), 1, 270.0);
    game.debug_well(MID, false).expect("a well");
    game.outcome = Outcome::Won;
    game.hold_end_screen = true;
    let before = pos(&game, enemy);
    let seen = idle(&mut game, ticks(tuning().well_form_seconds + 1.0));
    assert!(pos(&game, enemy).distance_to(before) < 0.5, "nothing pulled");
    assert!(!seen.iter().any(|e| matches!(e, Event::Hit { .. })));
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
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.show_intro = false;
    game.level_overrides.mission = Some(Mission::Destroy);
    game.players = PlayerCount::from_count(2).expect("two seats");
    let map = "version = 1\ntanks = 0\ntank = \"assault\"\ntank2 = \"assault\"\ncells.\"3,6\" = { kind = \"start\" }\ncells.\"3,12\" = { kind = \"start2\" }\n";
    game.map = MapFile::from_toml_str(map).expect("test map parses");
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
