//! Round flow (docs/levels.md "A round that cannot stall"): the round's
//! last enemies are called to the fight, a wave gets onto the field past
//! blocked lanes, and a wrecked seat comes back on the final wave and on
//! a map with no gate.

use super::*;
use crate::tuning::Tuning;

/// `f` with `table` in force on this thread.
fn under<R>(table: Tuning, f: impl FnOnce() -> R) -> R {
    crate::tuning::in_force(Some(&std::sync::Arc::new(table)), f)
}

/// A 110 x 20 strip with the start at its west end and a band of `tanks`.
fn strip(tanks: usize) -> String {
    format!("version = 1\nsize = [110, 20]\ntanks = {tanks}\ncells.\"3,10\" = {{ kind = \"start\" }}\n")
}

/// A seeded Destroy round on `map` at the map's own size, its one seat
/// shielded so only the test ends anything.
fn destroy_round(map: &str) -> Game {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.player_row_override = Some(0);
    game.level_overrides.mission = Some(Mission::Destroy);
    game.map = MapFile::from_toml_str(map).expect("test map parses");
    let (w, h) = game.map.field_size();
    game.init(w, h);
    for seat in game.players().into_iter().flatten() {
        with_tank_mut(&game.world, seat, |t| t.shield_hp = 1.0e9);
    }
    game
}

fn step(game: &mut Game) {
    let (w, h) = game.map.field_size();
    game.update(Input::default(), 1.0 / 60.0, w, h);
}

/// How many tanks the straggler rule called this frame, if it did.
fn called_now(game: &Game) -> Option<usize> {
    game.events().iter().find_map(|e| match e {
        Event::StragglersCalled { count } => Some(*count),
        _ => None,
    })
}

fn enemy_minds(game: &Game) -> Vec<(usize, Position, crate::ai::FieldMind)> {
    let mut minds: Vec<_> =
        game.world.query::<(&Tank, &Ai)>().iter().filter(|(t, _)| !t.is_wreck()).map(|(t, ai)| (t.owner_slot(), t.position, ai.field)).collect();
    minds.sort_by_key(|&(slot, ..)| slot);
    minds
}

/// Step until the rule calls (at most `limit` frames): the frame and the
/// count.
fn step_until_called(game: &mut Game, limit: u32) -> Option<(u32, usize)> {
    for frame in 1..=limit {
        step(game);
        if let Some(count) = called_now(game) {
            return Some((frame, count));
        }
    }
    None
}

#[test]
fn a_field_maps_last_sleeping_band_is_called_to_the_seat_after_a_lull() {
    let mut game = destroy_round(&strip(2));
    assert!(game.field_map());
    let seat = player_pos(&game);
    step(&mut game);
    let before = enemy_minds(&game);
    assert_eq!(before.len(), 2);
    assert!(before.iter().all(|(_, _, m)| !m.awake && !m.called), "the band starts asleep, out of touch: {before:?}");
    let after = tuning().straggler_call_after_seconds;
    let (frame, count) = step_until_called(&mut game, (after * 60.0) as u32 + 5).expect("the lull never called the band");
    assert!(frame as f32 >= after * 60.0 - 2.0, "not before the lull has run: frame {frame}");
    assert_eq!(count, 2);
    let from: Vec<f32> = enemy_minds(&game).iter().map(|(_, p, m)| {
        assert!(m.called, "every one of the last is called");
        p.distance_to(seat)
    }).collect();
    // Called, they wake and come: within sight of the seat, where the
    // call ends, well inside the walk the strip is.
    let sight = game.enemy_sight();
    let mut arrived = false;
    for _ in 0..3600 {
        step(&mut game);
        let minds = enemy_minds(&game);
        assert!(minds.iter().all(|(_, _, m)| m.awake), "a called tank is awake");
        if minds.iter().any(|(_, p, m)| p.distance_to(seat) <= sight && !m.called) {
            arrived = true;
            break;
        }
    }
    assert!(arrived, "no called tank reached the seat's sight from {from:?}");
}

#[test]
fn a_band_larger_than_the_rule_counts_is_left_alone_until_it_is_down_to_its_last() {
    let mut game = destroy_round(&strip(4));
    let after = tuning().straggler_call_after_seconds;
    assert_eq!(tuning().straggler_call_alive, 3, "the test's band is one over it");
    assert_eq!(step_until_called(&mut game, (after * 60.0) as u32 + 120), None, "four alive: nobody is called");
    assert_eq!(game.straggler_lull(), 0.0, "and nothing is counted");
    let slot = enemy_minds(&game)[0].0;
    game.debug_kill(slot).expect("a live enemy");
    let (_, count) = step_until_called(&mut game, (after * 60.0) as u32 + 5).expect("three alive and a lull: called");
    assert_eq!(count, 3);
}

#[test]
fn an_arenas_last_tank_is_called_at_the_seat_it_cannot_see() {
    // The strip shown whole: an arena, its one enemy put far past its
    // sight of the seat with no alert to follow.
    let mut game = destroy_round(&format!("{}view = \"whole\"\n", strip(1)));
    assert!(!game.field_map());
    let seat = player_pos(&game);
    let sight = game.enemy_sight();
    let slot = enemy_minds(&game)[0].0;
    let at = Position::new(seat.x + 3000.0, seat.y);
    game.debug_teleport(slot, at, None).expect("teleports");
    assert!(at.distance_to(seat) > sight, "the test's enemy starts out of sight");
    let after = tuning().straggler_call_after_seconds;
    let (_, count) = step_until_called(&mut game, (after * 60.0) as u32 + 5).expect("the lull never called it");
    assert_eq!(count, 1);
    let mut ended = false;
    for _ in 0..3600 {
        step(&mut game);
        let (_, p, mind) = enemy_minds(&game).into_iter().find(|&(s, ..)| s == slot).expect("still alive");
        if !mind.called {
            assert!(p.distance_to(seat) <= sight + 1.0, "the call ends in sight of the seat: {p:?}");
            ended = true;
            break;
        }
    }
    assert!(ended, "the called tank never reached the seat");
}

#[test]
fn a_sandbox_and_a_round_with_the_rule_off_call_nobody() {
    // No band (`tanks = 0`): nothing to win by wrecking, so the tank a
    // tool put down is nobody's straggler.
    let mut game = destroy_round(&strip(0));
    let seat = player_pos(&game);
    game.debug_spawn_enemy(Position::new(seat.x + 3000.0, seat.y), Some(1), None).expect("an enemy spawns");
    let frames = (tuning().straggler_call_after_seconds * 60.0) as u32 + 120;
    assert_eq!(step_until_called(&mut game, frames), None);
    // `straggler_call_alive` 0 is the rule off.
    under(Tuning { straggler_call_alive: 0, ..Tuning::DEFAULT }, || {
        let mut game = destroy_round(&strip(2));
        assert_eq!(step_until_called(&mut game, frames), None);
        assert_eq!(game.straggler_lull(), 0.0);
    });
}

#[test]
fn a_hit_or_a_seat_in_sight_keeps_the_lull_from_running() {
    let mut game = destroy_round(&strip(1));
    let after = tuning().straggler_call_after_seconds;
    for _ in 0..((after * 30.0) as u32) {
        step(&mut game);
    }
    assert!(game.straggler_lull() > after * 0.4, "half a lull has run: {}", game.straggler_lull());
    // The seat is put down within the enemy's sight: in touch.
    let (_, at, _) = enemy_minds(&game)[0];
    teleport_player(&mut game, Position::new(at.x - 200.0, at.y));
    step(&mut game);
    assert_eq!(game.straggler_lull(), 0.0, "a seat in sight is the fight");
}

/// A one-gate waves map, 40 x 20, the gate at the middle of the west edge.
const ONE_GATE: &str = "version = 1\nsize = [40, 20]\ncells.\"20,10\" = { kind = \"start\" }\ncells.\"20,6\" = { kind = \"start2\" }\ncells.\"0,10\" = { kind = \"gate\" }\n";

/// A 30 x 18 waves map inside an iron ring four cells thick, deeper than
/// a gate's lane reaches in, so it offers no gate at all.
fn walled_map() -> String {
    let mut map = String::from("version = 1\nsize = [30, 18]\ncells.\"15,9\" = { kind = \"start\" }\ncells.\"15,6\" = { kind = \"start2\" }\n");
    for r in 0..18 {
        for c in 0..30 {
            if r < 4 || r >= 14 || c < 4 || c >= 26 {
                map.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
            }
        }
    }
    map
}

/// A seeded Destroy round of `waves` waves of one tank each on `map`, with
/// `seats` seats, every one shielded so only the scheduler ends it.
fn waves_round(map: &str, waves: u32, seats: usize) -> Game {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.player_row_override = Some(0);
    game.players = PlayerCount::from_count(seats).expect("a seat count the round takes");
    game.level_overrides.mission = Some(Mission::Destroy);
    game.level_overrides.spawn = Some(SpawnKind::Waves);
    game.level_overrides.waves = Some(waves);
    game.level_overrides.wave_size = Some(1);
    game.level_overrides.wave_growth = Some(0);
    game.map = MapFile::from_toml_str(map).expect("test map parses");
    let (w, h) = game.map.field_size();
    game.init(w, h);
    for seat in game.players().into_iter().flatten() {
        with_tank_mut(&game.world, seat, |t| t.shield_hp = 1.0e9);
    }
    game
}

fn entered_now(game: &Game) -> Vec<usize> {
    game.events()
        .iter()
        .filter_map(|e| match e {
            Event::TankEntered { slot } => Some(*slot),
            _ => None,
        })
        .collect()
}

#[test]
fn a_seat_parked_on_every_gate_cannot_hold_a_wave_out() {
    let mut game = waves_round(ONE_GATE, 1, 1);
    let (w, h) = game.map.field_size();
    let grid = game.nav_grid(w, h);
    let gates = crate::battlefield::gates_from_cells(&grid, w, h, &game.map.gate_cells(), tuning().wave_gate_inward_cells);
    assert_eq!(gates.len(), 1, "the test's map has one lane");
    // The seat stands on the lane's inside point before the wave is
    // called: every lane is busy, and stays so.
    teleport_player(&mut game, gates[0].inside);
    let wait = tuning().wave_lane_wait_seconds;
    let mut landed = None;
    for frame in 1..=((wait * 60.0) as u32 + 60) {
        step(&mut game);
        assert!(
            game.world.query::<(&Tank, &crate::simulation::waves::RollIn)>().iter().all(|(t, _)| t.is_player()),
            "nothing rolls down the blocked lane"
        );
        if let Some(&slot) = entered_now(&game).first() {
            landed = Some((frame, slot));
            break;
        }
    }
    let (frame, slot) = landed.expect("the wave tank never got onto the field");
    assert!(frame as f32 >= wait * 60.0 - 2.0, "it waited out the lanes first: frame {frame}");
    let entity = game.tank_entity_by_slot(slot).expect("the wave's tank");
    assert!(game.world.get::<&Ai>(entity).is_ok(), "it fights at once");
    let at = with_tank(&game.world, entity, |t| t.position);
    assert!(at.x > 0.0 && at.x < w && at.y > 0.0 && at.y < h, "on the field: {at:?}");
}

#[test]
fn the_final_wave_brings_a_fallen_seat_back() {
    let mut game = waves_round(ONE_GATE, 1, 2);
    let after = tuning().wave_final_return_seconds;
    // The one wave is the final one: its tank in, seat 1 falls.
    for _ in 0..600 {
        step(&mut game);
        if !entered_now(&game).is_empty() {
            break;
        }
    }
    assert_eq!(game.wave_status().expect("a waves round").index, 1);
    game.debug_kill(1).expect("seat 1");
    let seat1 = game.seat(1).expect("seat 1");
    let mut back = None;
    for frame in 1..=((after * 60.0) as u32 + 60) {
        step(&mut game);
        if game.is_entering(seat1) {
            back = Some(frame);
            break;
        }
        assert!(with_tank(&game.world, seat1, Tank::is_wreck), "a wreck until it comes back");
    }
    let frame = back.expect("the final wave never brought seat 1 back");
    assert!(frame as f32 >= after * 60.0 - 2.0, "not before its wait: frame {frame}");
    assert!(with_tank(&game.world, seat1, |t| !t.is_wreck()), "a fresh tank");
    assert_eq!(game.outcome(), Outcome::Playing);

    // 0 keeps it where it fell.
    under(Tuning { wave_final_return_seconds: 0.0, ..Tuning::DEFAULT }, || {
        let mut game = waves_round(ONE_GATE, 1, 2);
        for _ in 0..120 {
            step(&mut game);
        }
        game.debug_kill(1).expect("seat 1");
        let seat1 = game.seat(1).expect("seat 1");
        for _ in 0..((after * 60.0) as u32 + 120) {
            step(&mut game);
            assert!(with_tank(&game.world, seat1, Tank::is_wreck) && !game.is_entering(seat1));
        }
    });
}

#[test]
fn a_map_with_no_gate_drops_a_fallen_seat_back_near_the_living() {
    let map = walled_map();
    let mut game = waves_round(&map, 2, 2);
    let (w, h) = game.map.field_size();
    let grid = game.nav_grid(w, h);
    let inward = tuning().wave_gate_inward_cells;
    assert!(crate::battlefield::gate_candidates(&grid, w, h, &[], 0.0, inward).is_empty(), "the test's map has no lane");
    for _ in 0..600 {
        step(&mut game);
        if !entered_now(&game).is_empty() {
            break;
        }
    }
    let wave_one = enemy_minds(&game).first().map(|&(slot, ..)| slot).expect("wave 1 dropped in");
    game.debug_kill(1).expect("seat 1");
    step(&mut game);
    let seat1 = game.seat(1).expect("seat 1");
    // Clearing wave 1 calls wave 2, which brings seat 1 back: no gate to
    // drive in by, so it lands on the field, its body and the stick at once.
    game.debug_kill(wave_one).expect("wave 1's tank");
    let mut landed = false;
    for _ in 0..900 {
        step(&mut game);
        if entered_now(&game).contains(&1) {
            landed = true;
            break;
        }
    }
    assert!(landed, "seat 1 never came back on a map with no gate");
    let (at, wreck, body) = with_tank(&game.world, seat1, |t| (t.position, t.is_wreck(), t.body.is_some()));
    assert!(!wreck && body && !game.is_entering(seat1), "on the field at once, never through a gate");
    assert!(at.x > 0.0 && at.x < w && at.y > 0.0 && at.y < h, "on the field: {at:?}");
    let seat0 = player_pos(&game);
    assert!(at.distance_to(seat0) < 6.0 * crate::OBSTACLE_GRID_SIZE, "near the living seat: {at:?} by {seat0:?}");
}

#[test]
fn a_seat_coming_back_past_a_blocked_lane_drops_in_near_the_living() {
    let mut game = waves_round(ONE_GATE, 1, 2);
    let (w, h) = game.map.field_size();
    let grid = game.nav_grid(w, h);
    let gates = crate::battlefield::gates_from_cells(&grid, w, h, &game.map.gate_cells(), tuning().wave_gate_inward_cells);
    teleport_player(&mut game, gates[0].inside);
    // The one wave - the final one - is called, then seat 1 falls.
    for _ in 0..120 {
        step(&mut game);
    }
    game.debug_kill(1).expect("seat 1");
    let seat1 = game.seat(1).expect("seat 1");
    let (back, wait) = (tuning().wave_final_return_seconds, tuning().wave_lane_wait_seconds);
    let mut landed = None;
    for frame in 1..=(((back + wait) * 60.0) as u32 + 120) {
        step(&mut game);
        assert!(!game.is_entering(seat1), "it never drives down the blocked lane");
        if entered_now(&game).contains(&1) {
            landed = Some(frame);
            break;
        }
    }
    let frame = landed.expect("seat 1 never came back past the blocked lane");
    assert!(frame as f32 >= (back + wait) * 60.0 - 2.0, "the final wave's wait, then the lane's: frame {frame}");
    let (at, wreck, body) = with_tank(&game.world, seat1, |t| (t.position, t.is_wreck(), t.body.is_some()));
    assert!(!wreck && body);
    assert!(at.distance_to(player_pos(&game)) < 6.0 * crate::OBSTACLE_GRID_SIZE, "near the living seat: {at:?}");
}
