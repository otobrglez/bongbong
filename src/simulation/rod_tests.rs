//! Headless scenario tests for the rod from god (docs/rod-from-god.md
//! "Tests"): one per promised rule, on the default 34 x 17 field with the
//! seat at cell (3, 6) facing east and enemies placed by hand. Tests cannot
//! touch the global tuning table, so every number is read off the
//! defaults.

use super::*;
use crate::ai::Role;
use crate::tank::ChargeEnd;
use crate::zone::Zone;

const W: f32 = 1088.0;
const H: f32 = 544.0;
const DT: f32 = 1.0 / 60.0;

/// The seat's pivot: cell (3, 6).
const SEAT: Position = Position::new(96.0, 192.0);

/// A round on an open field plus `extra` cells, on `mission`, nobody
/// shielded, no banner, the seat facing east with the rod.
fn round_as(extra: &str, mission: Mission) -> Game {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.show_intro = false;
    game.level_overrides.mission = Some(mission);
    let map = format!("version = 1\ntanks = 0\ncells.\"3,6\" = {{ kind = \"start\" }}\n{extra}");
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
        t.take_weapon(ActiveWeapon::RodFromGod);
    });
    game
}

fn round(extra: &str) -> Game {
    round_as(extra, Mission::Destroy)
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

fn seat(game: &Game) -> Entity {
    game.player().expect("a seat")
}

fn reticle(game: &Game) -> Option<(i32, i32)> {
    with_tank(&game.world, seat(game), |t| t.reticle.map(|r| r.cell))
}

fn rods(game: &Game) -> i32 {
    with_tank(&game.world, seat(game), |t| t.rods)
}

fn settle_ticks() -> u32 {
    crate::tank::ticks_of(tuning().rod_settle_seconds) + 1
}

fn countdown_ticks() -> u32 {
    crate::tank::ticks_of(tuning().rod_countdown_seconds) + 2
}

/// Press, tap the stick along `taps` (a tick held, a tick at rest each),
/// hold until the charge is full, let go: every event.
fn call(game: &mut Game, taps: &[Dir]) -> Vec<Event> {
    let mut seen = step(game, true);
    for &dir in taps {
        seen.extend(step_with(game, Intent { fire: true, move_dir: Some(dir), ..Intent::default() }));
        seen.extend(step(game, true));
    }
    for _ in 0..settle_ticks() {
        seen.extend(step(game, true));
    }
    seen.extend(step(game, false));
    seen
}

fn called(events: &[Event]) -> Vec<(i32, i32)> {
    events.iter().filter_map(|e| if let Event::RodCalled { cell, .. } = e { Some(*cell) } else { None }).collect()
}

fn impacts(events: &[Event]) -> Vec<(i32, i32)> {
    events.iter().filter_map(|e| if let Event::RodImpact { cell, .. } = e { Some(*cell) } else { None }).collect()
}

fn rod_kills(events: &[Event]) -> usize {
    events.iter().filter(|e| matches!(e, Event::Hit { cause: HitCause::Rod, killed: true, .. })).count()
}

/// A crate arms two calls and replaces the special carried; the rod is a
/// charge weapon whose stick aims.
#[test]
fn a_rod_crate_arms_two_calls_and_replaces_the_special() {
    let t = tuning();
    let mut tank = Tank::default();
    tank.take_weapon(ActiveWeapon::GaussRail);
    tank.take_weapon(ActiveWeapon::RodFromGod);
    assert_eq!((tank.rods, tank.gauss_slugs), (t.rod_per_pickup, 0));
    assert_eq!(tank.active_weapon(), ActiveWeapon::RodFromGod);
    assert_eq!(ActiveWeapon::RodFromGod.trigger(), crate::tank::Trigger::Charge);
    let rule = ActiveWeapon::RodFromGod.charge_rule().expect("a charge rule");
    assert_eq!(rule.stick, crate::tank::Stick::Aim);
    assert!(rule.overcharge.is_none());
    assert_eq!(PickupKind::RodFromGod.weapon(), Some(ActiveWeapon::RodFromGod));
}

/// The press puts the reticle `rod_reticle_start_cells` ahead of the hull;
/// a tap of the stick steps it a cell; the hull stands while it aims.
#[test]
fn the_reticle_starts_ahead_and_the_stick_steps_it_while_the_hull_stands() {
    let mut game = round("");
    let start = tuning().rod_reticle_start_cells;
    step(&mut game, true);
    assert_eq!(reticle(&game), Some((3 + start, 6)));
    let before = with_tank(&game.world, seat(&game), |t| t.position);
    for dir in [Dir::Right, Dir::Right, Dir::Down] {
        step_with(&mut game, Intent { fire: true, move_dir: Some(dir), ..Intent::default() });
        step(&mut game, true);
    }
    assert_eq!(reticle(&game), Some((5 + start, 7)));
    // Held, it steps once at once, again after the delay, then at the repeat.
    for _ in 0..crate::tank::ticks_of(tuning().rod_reticle_delay_seconds) + 1 {
        step_with(&mut game, Intent { fire: true, move_dir: Some(Dir::Up), ..Intent::default() });
    }
    assert_eq!(reticle(&game), Some((5 + start, 5)), "a step at once and one after the delay");
    let after = with_tank(&game.world, seat(&game), |t| t.position);
    assert!(after.distance_to(before) < 0.5, "the hull stands while the stick aims: {before:?} -> {after:?}");
    step(&mut game, false);
    assert_eq!(reticle(&game), None, "the reticle goes with the charge");
}

/// The reticle stays inside the caller's sight box and the field.
#[test]
fn the_reticle_stays_inside_the_sight_box_and_the_field() {
    let mut game = round("");
    step(&mut game, true);
    for _ in 0..120 {
        step_with(&mut game, Intent { fire: true, move_dir: Some(Dir::Left), ..Intent::default() });
    }
    assert_eq!(reticle(&game).map(|c| c.0), Some(0), "held at the field's west edge");
    for _ in 0..120 {
        step_with(&mut game, Intent { fire: true, move_dir: Some(Dir::Right), ..Intent::default() });
    }
    let half = (tuning().sight_box_half_cols - 0.5).floor() as i32;
    assert_eq!(reticle(&game).map(|c| c.0), Some(3 + half), "held inside the sight box");
}

/// A release calls the rod onto the reticle's cell: a call spent, the zone
/// standing for the countdown, then the impact - a hull in the circle
/// crushed, shield and all, one in the shove ring knocked, one past it left.
#[test]
fn a_release_calls_the_rod_and_it_lands_after_the_countdown() {
    let mut game = round("");
    let t = tuning();
    let target = crate::map::cell_to_world(12, 6);
    let crushed = parked(&mut game, target);
    with_tank_mut(&game.world, crushed, |t| t.raise_shield());
    let knocked = parked(&mut game, Position::new(target.x, target.y + 96.0));
    let far = parked(&mut game, Position::new(target.x, target.y + 256.0));
    let start = tuning().rod_reticle_start_cells;
    let taps: Vec<Dir> = std::iter::repeat_n(Dir::Right, (12 - 3 - start) as usize).collect();
    let events = call(&mut game, &taps);
    assert_eq!(called(&events), vec![(12, 6)], "{events:?}");
    assert!(events.iter().any(|e| matches!(e, Event::Fired { weapon: "rod_from_god", .. })));
    assert_eq!(rods(&game), t.rod_per_pickup - 1);
    assert_eq!(game.zones().len(), 1);
    let land = game.zones()[0].until;
    let mut seen = Vec::new();
    while game.time < land - DT * 2.0 {
        seen.extend(step(&mut game, false));
    }
    assert!(impacts(&seen).is_empty(), "nothing lands before the countdown is out");
    assert!(!with_tank(&game.world, crushed, |t| t.is_wreck()));
    let before = with_tank(&game.world, knocked, |t| t.position);
    seen.extend(idle(&mut game, 4));
    assert_eq!(impacts(&seen), vec![(12, 6)]);
    assert!(game.zones().is_empty(), "the call is gone once it lands");
    assert_eq!(rod_kills(&seen), 1, "{seen:?}");
    seen.extend(idle(&mut game, 30));
    assert!(with_tank(&game.world, crushed, |t| t.is_wreck()), "crushed through its shield");
    let after = with_tank(&game.world, knocked, |t| t.position);
    assert!(after.y - before.y > 8.0, "knocked out from the strike: {before:?} -> {after:?}");
    assert!(!with_tank(&game.world, knocked, |t| t.is_wreck()));
    assert!(!with_tank(&game.world, far, |t| t.is_wreck() || t.skid > 0.0), "past the shove ring nothing happens");
}

/// Letting go with the reticle on the caller's own cell is the cancel, and
/// a release before the charge has settled is a fizzle: neither spends a
/// call.
#[test]
fn a_release_on_its_own_cell_or_unsettled_spends_nothing() {
    let mut game = round("");
    let start = tuning().rod_reticle_start_cells as usize;
    let taps: Vec<Dir> = std::iter::repeat_n(Dir::Left, start).collect();
    let events = call(&mut game, &taps);
    assert!(called(&events).is_empty());
    assert!(events.iter().any(|e| matches!(e, Event::ChargeEnded { end: ChargeEnd::Fizzled, .. })), "{events:?}");
    assert_eq!(rods(&game), tuning().rod_per_pickup);
    idle(&mut game, 10);
    let mut events = step(&mut game, true);
    events.extend(step(&mut game, false));
    assert!(called(&events).is_empty(), "unsettled: {events:?}");
    assert_eq!(rods(&game), tuning().rod_per_pickup);
}

/// The impact breaks every breakable tile whose cell reaches into the break
/// radius - a drum going off at once - and leaves iron standing.
#[test]
fn the_impact_breaks_the_tiles_in_reach_and_iron_stands() {
    let mut game = round(
        "cells.\"12,6\" = { kind = \"wall\", material = \"brick\" }\n\
         cells.\"13,6\" = { kind = \"wall\", material = \"iron\" }\n\
         cells.\"12,8\" = { kind = \"sandbag\" }\n\
         cells.\"11,5\" = { kind = \"barrel\", drum = \"oil\" }\n\
         cells.\"12,10\" = { kind = \"wall\", material = \"wood\" }\n",
    );
    game.debug_call_rod(crate::map::cell_to_world(12, 6), false).expect("a call");
    idle(&mut game, countdown_ticks() + 30);
    let standing = |c: (i32, i32)| game.world.query::<&crate::obstacle::Obstacle>().iter().any(|o| o.cell() == c && !o.destroyed);
    assert!(!standing((12, 6)), "brick under the strike");
    assert!(!standing((12, 8)), "a sandbag two cells off");
    assert!(!standing((11, 5)), "the drum");
    assert!(standing((13, 6)), "iron stands");
    assert!(standing((12, 10)), "past the break radius");
}

/// On dry ground the rod leaves a crater: a pit that slows a hull and the
/// router prices; under rain it fills and is a ford.
#[test]
fn a_crater_slows_a_hull_and_fills_under_rain() {
    let mut game = round("");
    game.debug_call_rod(crate::map::cell_to_world(12, 6), false).expect("a call");
    let events = idle(&mut game, countdown_ticks());
    assert!(events.iter().any(|e| matches!(e, Event::RodImpact { crater: true, .. })), "{events:?}");
    assert!(game.craters().holds((12, 6)) && game.craters().holds((13, 6)) && !game.craters().holds((14, 6)));
    let pit = Footing::at(&game.water, &game.lava, &game.craters, game.weather, crate::map::cell_to_world(12, 6), game.time);
    assert!((pit.pace - tuning().rod_crater_pace).abs() < 1e-6, "{pit:?}");
    assert_eq!(game.water.depth_of_cell(12, 6), crate::ground::Depth::Dry);
    game.change_weather(crate::map::Skies::of(crate::map::Weather::Rain));
    assert_eq!(game.water.depth_of_cell(12, 6), crate::ground::Depth::Shallow, "rain fills it");
    assert_eq!(game.water.depth_of_cell(14, 6), crate::ground::Depth::Dry);
}

/// No crater where the strike lands in water.
#[test]
fn no_crater_in_water() {
    let mut game = round("cells.\"12,6\" = { kind = \"water\" }\n");
    game.debug_call_rod(crate::map::cell_to_world(12, 6), false).expect("a call");
    idle(&mut game, countdown_ticks());
    assert!(!game.craters().holds((12, 6)), "the water cell is no crater");
    assert!(game.craters().holds((13, 6)), "the dry ground round it is");
}

/// A frog under a call hops out of it if it can; a stunned one dies.
#[test]
fn a_frog_under_a_call_hops_out_and_a_stunned_one_dies() {
    let mut game = round_as("cells.\"14,12\" = { kind = \"frog\" }\n", Mission::Protect);
    let frog = game.frog.expect("a frog");
    let at = with_frog(&game.world, frog, |f| f.position);
    game.debug_call_rod(at, true).expect("a call");
    idle(&mut game, countdown_ticks());
    let (dead, now) = with_frog(&game.world, frog, |f| (f.is_dead(), f.position));
    assert!(!dead && now.distance_to(at) > tuning().rod_kill_radius_px, "it got out: {at:?} -> {now:?}");

    let mut game = round_as("cells.\"14,12\" = { kind = \"frog\" }\n", Mission::Protect);
    let frog = game.frog.expect("a frog");
    let at = with_frog(&game.world, frog, |f| f.position);
    with_frog_mut(&game.world, frog, |f| f.stun(60.0));
    game.debug_call_rod(at, true).expect("a call");
    let events = idle(&mut game, countdown_ticks());
    assert!(with_frog(&game.world, frog, |f| f.is_dead()), "{events:?}");
}

/// A rod that strikes a volcano's cone sets it off: its eruption begins on
/// the tick after the impact.
#[test]
fn a_rod_on_a_volcano_sets_it_off() {
    let mut game = round("cells.\"20,8\" = { kind = \"volcano\" }\n");
    let t = tuning();
    let erupting = |game: &Game| game.volcanoes[0].phase(game.time, &t).stage == crate::volcano::Stage::Erupt;
    assert!(!erupting(&game));
    game.debug_call_rod(crate::map::cell_to_world(20, 8), false).expect("a call");
    let mut events = Vec::new();
    while !events.iter().any(|e| matches!(e, Event::RodImpact { .. })) {
        events = step(&mut game, false);
    }
    assert!(events.iter().any(|e| matches!(e, Event::RodImpact { erupted: true, .. })), "{events:?}");
    step(&mut game, false);
    assert!(erupting(&game), "erupting on the tick after");
    assert!(!game.craters().holds((20, 8)), "no crater on the cone");
}

/// The same on the shipped `vulkan` level: a call on its crater cell
/// erupts the volcano on the next tick, whatever its phase, and the next
/// eruption comes a period after that one.
#[test]
fn a_rod_on_vulkans_crater_sets_it_off() {
    let text = crate::map::SHIPPED_MAPS.iter().find(|(n, _)| *n == "vulkan").expect("shipped").1;
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.show_intro = false;
    game.map = MapFile::from_toml_str(text).expect("parses");
    let (w, h) = game.map.field_size();
    game.init(w, h);
    let t = tuning();
    assert_eq!(game.volcanoes.len(), 1);
    let crater = crate::map::cell_to_world(26, 15);
    let stage = |game: &Game| game.volcanoes[0].phase(game.time, &t).stage;
    // Wait until it is asleep, so the eruption is the rod's.
    while stage(&game) != crate::volcano::Stage::Asleep {
        step(&mut game, false);
    }
    game.debug_call_rod(crater, false).expect("a call");
    let mut events = Vec::new();
    while !events.iter().any(|e| matches!(e, Event::RodImpact { .. })) {
        events = step(&mut game, false);
    }
    assert!(events.iter().any(|e| matches!(e, Event::RodImpact { erupted: true, .. })), "{events:?}");
    step(&mut game, false);
    assert_eq!(stage(&game), crate::volcano::Stage::Erupt, "erupting on the tick after");
    let from = game.time;
    while stage(&game) == crate::volcano::Stage::Erupt {
        step(&mut game, false);
    }
    assert!(game.time - from < t.volcano_erupt_seconds + 0.1, "an eruption's length");
    assert!(!game.craters().holds((26, 15)));
}

/// Every enemy keeps out of a call: one standing in the circle drives out
/// before it lands.
#[test]
fn an_enemy_leaves_a_call_before_it_lands() {
    let mut game = round("");
    let at = crate::map::cell_to_world(20, 10);
    let slot = game.debug_spawn_enemy(at, Some(1), Some(Role::Player)).expect("spawns");
    let enemy = game.tank_entity_by_slot(slot).expect("exists");
    with_tank_mut(&game.world, enemy, |t| {
        t.shells_ammo = 0;
        t.disarm();
    });
    game.debug_call_rod(at, false).expect("a call");
    let events = idle(&mut game, countdown_ticks() + 10);
    assert_eq!(impacts(&events).len(), 1);
    assert!(!with_tank(&game.world, enemy, |t| t.is_wreck()), "it got out of the circle");
}

/// An enemy with the rod calls one on a seat that stands still - from
/// inside that seat's sight box, never outside it - and the seat that does
/// not leave is crushed.
#[test]
fn an_enemy_calls_a_rod_on_a_seat_standing_still() {
    let mut game = round("");
    with_tank_mut(&game.world, seat(&game), |t| t.disarm());
    let slot = game.debug_spawn_enemy(crate::map::cell_to_world(11, 6), Some(1), Some(Role::Player)).expect("spawns");
    let enemy = game.tank_entity_by_slot(slot).expect("exists");
    with_tank_mut(&game.world, enemy, |t| {
        t.shells_ammo = 0;
        t.disarm();
        t.take_weapon(ActiveWeapon::RodFromGod);
        t.speed_scale = 0.0;
    });
    let (half_w, half_h) = tuning().sight_box_half_px();
    let mut seen = Vec::new();
    for _ in 0..60 * 20 {
        let events = step(&mut game, false);
        for e in &events {
            if let Event::RodCalled { slot: by, cell, .. } = e {
                assert_eq!(*by, slot);
                let caller = with_tank(&game.world, enemy, |t| t.position);
                let seat_at = with_tank(&game.world, seat(&game), |t| t.position);
                assert!((caller.x - seat_at.x).abs() <= half_w && (caller.y - seat_at.y).abs() <= half_h, "called from inside the box");
                assert_eq!(*cell, (3, 6), "on the seat's cell");
            }
        }
        seen.extend(events);
        if game.outcome() != Outcome::Playing {
            break;
        }
    }
    assert_eq!(called(&seen).len(), 1, "one call");
    assert!(seen.iter().any(|e| matches!(e, Event::Hit { target: HitTarget::Player { player: 0 }, cause: HitCause::Rod, killed: true, .. })), "the seat stood and was crushed");
}

/// An enemy with the rod beside a seat standing still backs off until its
/// own hull is out of the circle, then calls on it.
#[test]
fn an_enemy_too_near_its_target_backs_off_and_calls() {
    let mut game = round("");
    with_tank_mut(&game.world, seat(&game), |t| t.disarm());
    let slot = game.debug_spawn_enemy(crate::map::cell_to_world(5, 6), Some(1), Some(Role::Player)).expect("spawns");
    let enemy = game.tank_entity_by_slot(slot).expect("exists");
    with_tank_mut(&game.world, enemy, |t| {
        t.shells_ammo = 0;
        t.disarm();
        t.take_weapon(ActiveWeapon::RodFromGod);
    });
    let mut seen = Vec::new();
    for _ in 0..60 * 20 {
        seen.extend(step(&mut game, false));
        if !called(&seen).is_empty() {
            break;
        }
    }
    let seat_at = with_tank(&game.world, seat(&game), |t| t.position);
    let cells = called(&seen);
    assert_eq!(cells.len(), 1, "it called");
    assert!(crate::map::cell_to_world(cells[0].0, cells[0].1).distance_to(seat_at) <= 64.0, "on the seat: {cells:?} {seat_at:?}");
    let caller = with_tank(&game.world, enemy, |t| t.position);
    assert!(caller.distance_to(seat_at) > tuning().rod_kill_radius_px + 32.0, "from outside the circle: {caller:?}");
}

/// A seat's call on a field nobody stands in never costs the round a
/// replay: two rounds from one seed with one call agree to the event.
#[test]
fn a_call_replays_byte_for_byte() {
    let run = || {
        let mut game = round("cells.\"12,8\" = { kind = \"sandbag\" }\ncells.\"11,5\" = { kind = \"barrel\", drum = \"fuel\" }\n");
        parked(&mut game, crate::map::cell_to_world(12, 7));
        let mut log = format!("{:?}", call(&mut game, &[Dir::Right; 5]));
        log.push_str(&format!("{:?}", idle(&mut game, countdown_ticks() + 60)));
        log.push_str(&format!("{:?}", game.drawable_state()));
        log
    };
    assert_eq!(run(), run());
}

/// A zone's danger is one nobody owns, and the rod's flash outlasts a
/// drum's.
#[test]
fn a_call_is_a_danger_for_everyone_and_its_flash_is_the_strongest() {
    let t = tuning();
    let mut game = round("");
    game.debug_call_rod(crate::map::cell_to_world(12, 6), true).expect("a call");
    let zone: Zone = game.zones()[0];
    let danger = zone.danger(&t).expect("a danger");
    assert_eq!(danger.owner, None);
    game.flash_screen();
    game.flash_screen_with(t.rod_screen_flash);
    assert_eq!(game.screen_flash, Some(0.0), "a rod's flash is not held back by the gap");
    assert!((game.screen_flash_strength - t.rod_screen_flash).abs() < 1e-6);
    game.flash_screen();
    assert!((game.screen_flash_strength - t.rod_screen_flash).abs() < 1e-6, "a weaker one does not replace it");
}

/// A room's report puts the seat's reticle on the client's cell - from
/// the press on, held to the reticle's range - and the release calls there
/// (docs/rod-from-god.md "The reticle report").
#[test]
fn a_rooms_report_puts_the_reticle_where_the_client_has_it() {
    let mut game = round("");
    game.set_seat_reticle(0, Some((9, 8)));
    step(&mut game, true);
    assert_eq!(reticle(&game), Some((9, 8)), "on the reported cell from the press");
    for _ in 0..settle_ticks() {
        game.set_seat_reticle(0, Some((10, 8)));
        step(&mut game, true);
    }
    assert_eq!(reticle(&game), Some((10, 8)));
    game.set_seat_reticle(0, Some((40, 8)));
    step(&mut game, true);
    let half = (tuning().sight_box_half_cols - 0.5).floor() as i32;
    assert_eq!(reticle(&game), Some((3 + half, 8)), "held to the range");
    game.set_seat_reticle(0, Some((11, 7)));
    let events = step(&mut game, false);
    assert_eq!(called(&events), vec![(11, 7)], "the release calls on the reported cell");
}

/// Every hull with any part of its box in the circle is crushed, either
/// side - the caller and a teammate too -, and one whose box stays a
/// pixel outside it is only shoved.
#[test]
fn the_circle_crushes_every_hull_with_any_part_inside_it() {
    let mut game = round("");
    let t = tuning();
    let c = crate::map::cell_to_world(15, 8);
    let inside = parked(&mut game, Position::new(c.x + t.rod_kill_radius_px + 10.0, c.y));
    let (half, _) = with_tank(&game.world, inside, |tk| {
        let (_, half) = tk.hull_bbox_world();
        (half, ())
    });
    let outside = parked(&mut game, Position::new(c.x, c.y - t.rod_kill_radius_px - half.y - 4.0));
    game.place_tank(seat(&game), Position::new(c.x - 30.0, c.y), Some(90.0)).expect("placed");
    game.debug_call_rod(c, false).expect("a call");
    let events = idle(&mut game, countdown_ticks() + 30);
    assert_eq!(rod_kills(&events), 2, "{events:?}");
    assert!(with_tank(&game.world, inside, |tk| tk.is_wreck()), "a part inside is enough");
    assert!(with_tank(&game.world, seat(&game), |tk| tk.is_wreck()), "the caller dies in its own circle");
    assert!(!with_tank(&game.world, outside, |tk| tk.is_wreck()), "a pixel out is not crushed");
}

/// Calls land in the order they were made, each on its own tick.
#[test]
fn several_calls_land_in_id_order() {
    let mut game = round("");
    let a = game.debug_call_rod(crate::map::cell_to_world(20, 4), false).expect("a call");
    idle(&mut game, 10);
    let b = game.debug_call_rod(crate::map::cell_to_world(20, 12), false).expect("a call");
    let events = idle(&mut game, countdown_ticks() + 20);
    let landed: Vec<u32> = events.iter().filter_map(|e| if let Event::RodImpact { id, .. } = e { Some(*id) } else { None }).collect();
    assert_eq!(landed, vec![a, b]);
}

/// On the end screen a call lands as a show and nothing else: no crush,
/// no tile, no crater.
#[test]
fn a_call_lands_harmlessly_on_the_end_screen() {
    let mut game = round("cells.\"20,8\" = { kind = \"wall\", material = \"brick\" }\n");
    let enemy = parked(&mut game, crate::map::cell_to_world(20, 9));
    game.debug_call_rod(crate::map::cell_to_world(20, 8), false).expect("a call");
    game.outcome = Outcome::Won;
    game.hold_end_screen = true;
    let events = idle(&mut game, countdown_ticks() + 10);
    assert!(impacts(&events).len() == 1, "the show plays: {events:?}");
    assert_eq!(rod_kills(&events), 0);
    assert!(!with_tank(&game.world, enemy, |tk| tk.is_wreck()));
    assert!(game.world.query::<&crate::obstacle::Obstacle>().iter().any(|o| o.cell() == (20, 8) && !o.destroyed), "the brick stands");
    assert!(game.craters().is_empty());
}

/// The rod draws nothing from the round's RNG: a call, its impact, a
/// shove and a plain tile crushed leave the stream where the same round
/// without them leaves it (what a crushed hull's wreck sets off rolls as
/// any wreck's does).
#[test]
fn the_rod_draws_no_rng() {
    let run = |call: bool| {
        let mut game = round("cells.\"20,8\" = { kind = \"wall\", material = \"brick\" }\n");
        parked(&mut game, crate::map::cell_to_world(20, 13));
        if call {
            game.debug_call_rod(crate::map::cell_to_world(20, 9), false).expect("a call");
        }
        let events = idle(&mut game, countdown_ticks() + 30);
        let mut rng = game.rng.clone().expect("seeded");
        (impacts(&events).len(), rand::RngExt::random::<u64>(&mut rng))
    };
    let (landed, with) = run(true);
    assert_eq!(landed, 1);
    let (_, without) = run(false);
    assert_eq!(with, without);
}

/// A drone over the circle is downed by the impact (`AirStrike::Rod`).
#[test]
fn a_drone_over_the_circle_is_downed() {
    let mut game = round("");
    let at = crate::map::cell_to_world(16, 8);
    let slot = game.debug_spawn_enemy(crate::map::cell_to_world(30, 8), Some(1), Some(Role::Player)).expect("spawns");
    game.debug_call_rod(at, false).expect("a call");
    idle(&mut game, crate::tank::ticks_of(tuning().rod_countdown_seconds) - 2);
    // A drone of the enemy's cruising over the circle as it lands.
    let mut d = crate::fpv::Drone::launch(at, crate::math::Vec2::new(-1.0, 0.0), 0, crate::shell::Owner::Enemy(slot), crate::fpv::DroneLock::None, crate::map::cell_to_world(2, 8));
    d.stage = crate::fpv::DroneStage::Cruise;
    d.height = tuning().fpv_cruise_height;
    d.id = game.take_shot_id();
    game.world.spawn((d,));
    let events = idle(&mut game, 6);
    assert!(events.iter().any(|e| matches!(e, Event::DroneDowned { by: "rod", .. })), "{events:?}");
}

/// The router prices a crater, and the kept nav grid after one is the one
/// built from scratch.
#[test]
fn the_router_prices_a_crater_and_the_kept_grid_follows() {
    let mut game = round("");
    game.debug_call_rod(crate::map::cell_to_world(16, 8), false).expect("a call");
    idle(&mut game, countdown_ticks());
    assert!(!game.craters().is_empty());
    let (w, h) = game.map.field_size();
    game.refresh_nav(w, h);
    let mut scratch = game.nav_grid(w, h);
    scratch.label();
    assert!(game.nav.base().same_as(&scratch), "the kept grid is the one built from scratch");
    let (a, b) = (crate::map::cell_to_world(16, 5), crate::map::cell_to_world(16, 11));
    let plain = round("").nav_grid(w, h).path_cost(a, b).expect("a way");
    let priced = scratch.path_cost(a, b).expect("a way");
    assert!(priced > plain, "the crater on the line costs more: {priced} vs {plain}");
}

/// The spawn swap hands the rod out by its share, a hash of the spawn and
/// never a draw; a tank that drew no special keeps none.
#[test]
fn the_spawn_swap_hands_out_the_rod_by_its_share() {
    let at = Position::new(400.0, 200.0);
    let mut t = crate::tuning::Tuning::DEFAULT;
    let mut laser = Tank::default();
    laser.take_weapon(ActiveWeapon::Laser);
    super::sonic::swap_spawn_special_with(&t, &mut laser, 5, at);
    assert_eq!(laser.special(), Some(ActiveWeapon::Laser), "nothing at the default share");
    t.enemy_special_weapon_rod_share = 1.0;
    super::sonic::swap_spawn_special_with(&t, &mut laser, 5, at);
    assert_eq!(laser.special(), Some(ActiveWeapon::RodFromGod));
    let mut shells = Tank::default();
    super::sonic::swap_spawn_special_with(&t, &mut shells, 5, at);
    assert_eq!(shells.special(), None);
}

/// An enemy takes the rod's crate only while it carries no special.
#[test]
fn an_enemy_takes_the_crate_only_with_no_special() {
    let mut tk = Tank { owner: crate::shell::Owner::Enemy(4), ..Tank::default() };
    assert!(tk.wants_pickup(PickupKind::RodFromGod));
    tk.take_weapon(ActiveWeapon::Plasma);
    assert!(!tk.wants_pickup(PickupKind::RodFromGod));
}

/// An enemy never calls on a seat whose circle holds an ally - one parked
/// beside the seat keeps the camper safe from the call.
#[test]
fn an_enemy_never_calls_on_a_circle_holding_an_ally() {
    let mut game = round("");
    with_tank_mut(&game.world, seat(&game), |t| t.disarm());
    parked(&mut game, Position::new(SEAT.x + 40.0, SEAT.y + 48.0));
    let slot = game.debug_spawn_enemy(crate::map::cell_to_world(11, 6), Some(1), Some(Role::Player)).expect("spawns");
    let enemy = game.tank_entity_by_slot(slot).expect("exists");
    with_tank_mut(&game.world, enemy, |t| {
        t.shells_ammo = 0;
        t.disarm();
        t.take_weapon(ActiveWeapon::RodFromGod);
        t.speed_scale = 0.0;
    });
    let events = idle(&mut game, 60 * 10);
    assert!(called(&events).iter().all(|&c| c != (3, 6)), "no call on the seat with an ally beside it: {:?}", called(&events));
}

/// The seat's motion record counts how long it stood within
/// `rod_ai_still_px` and averages its speed, and starts again on a wreck.
#[test]
fn the_seat_still_record_counts_still_and_averages_speed() {
    let t = crate::tuning::Tuning::DEFAULT;
    let mut r = crate::rod::SeatStill::default();
    for _ in 0..120 {
        r.step(Some(Position::new(100.0, 100.0)), DT, &t);
    }
    assert!((r.still - 119.0 * DT).abs() < 1e-3, "{}", r.still);
    for i in 0..60 {
        r.step(Some(Position::new(100.0 + i as f32 * 2.0, 100.0)), DT, &t);
    }
    assert!(r.still < 1.0, "{}", r.still);
    assert!(r.speed() > 30.0, "{}", r.speed());
    r.step(None, DT, &t);
    assert_eq!(r, crate::rod::SeatStill::default());
}
