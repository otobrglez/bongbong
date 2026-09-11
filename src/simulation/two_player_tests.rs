//! Headless scenario tests for two-player rounds (docs/two-players.md).
//!
//! Same shape as `tap_tests`: seeded rounds on tiny inline maps, asserting
//! on `Game::events` and the public snapshot. What is worth locking in is
//! the *contract* a second human tank adds - its own slot, its own keys,
//! friendly fire, the both-wrecks loss rule, nearest-player targeting and
//! the fallback placement - not any trajectory.

use super::*;
use crate::ai::Intent;
use crate::map::cell_to_world;
use crate::tank::Dir;

const W: f32 = 1280.0;
const H: f32 = 720.0;

/// Player 1 at (20,11), player 2 six cells to the right, the frog in the
/// far corner, one enemy boxed in iron where it can neither see nor reach
/// anything (`tap_tests`'s trick).
fn map_with(extra: &str) -> String {
    let mut text = String::from(
        r#"
version = 1
tanks = 1
cells."2,2" = { kind = "frog" }
cells."20,11" = { kind = "start" }
cells."26,11" = { kind = "start2" }
"#,
    );
    for (c, r) in [(36, 19), (37, 19), (38, 19), (36, 20), (38, 20), (36, 21), (37, 21), (38, 21)] {
        text.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
    }
    text.push_str(extra);
    text
}

fn game_on(map: &str, enemies: usize, players: PlayerCount, seed: u64) -> Game {
    let mut game = Game::default();
    game.enemy_count_override = Some(enemies);
    game.seed_override = Some(seed);
    game.player_row_override = Some(0);
    game.player2_row_override = Some(0);
    game.players = players;
    game.map = MapFile::from_toml_str(map).expect("test map parses");
    game.init(W, H);
    // Nothing here is about the spawn shield.
    for entity in game.players().into_iter().flatten() {
        let mut q = game.world.query_one::<&mut Tank>(entity);
        q.get().expect("player tank").shield_timer = 0.0;
    }
    game
}

/// A two-player round with its one enemy parked in the iron box.
fn boxed_game(seed: u64) -> Game {
    let mut game = game_on(&map_with(""), 1, PlayerCount::Two, seed);
    game.debug_teleport(2, cell_to_world(37, 20), Some(0.0)).expect("enemy in slot 2");
    game
}

fn step(game: &mut Game, input: Input) {
    game.update(input, 1.0 / 60.0, W, H);
}

fn snapshot_of(game: &Game, player: u8) -> TankSnapshot {
    game.tank_snapshots().into_iter().find(|t| t.player == Some(player)).expect("player snapshot")
}

fn drive(p1: Option<Dir>, p2: Option<Dir>) -> Input {
    Input {
        player_intent: Intent { move_dir: p1, ..Intent::default() },
        player2_intent: Intent { move_dir: p2, ..Intent::default() },
        ..Input::default()
    }
}

fn fire(p1: bool, p2: bool) -> Input {
    Input {
        player_intent: Intent { fire: p1, ..Intent::default() },
        player2_intent: Intent { fire: p2, ..Intent::default() },
        ..Input::default()
    }
}

fn enemy_target(game: &Game, slot: usize) -> u8 {
    game.world
        .query::<(&Tank, &Ai)>()
        .iter()
        .find(|(t, _)| t.owner_slot() == slot)
        .map(|(_, ai)| ai.target_player())
        .expect("enemy in that slot")
}

#[test]
fn both_players_spawn_in_their_own_slots_and_drive_independently() {
    let mut game = boxed_game(3);
    let snaps = game.tank_snapshots();
    let slot_of = |player: u8| snaps.iter().find(|t| t.player == Some(player)).map(|t| t.slot);
    assert_eq!(slot_of(0), Some(0));
    assert_eq!(slot_of(1), Some(1));
    assert_eq!(snaps.iter().filter(|t| !t.is_player).map(|t| t.slot).collect::<Vec<_>>(), vec![2]);
    assert!(snaps.iter().filter(|t| t.is_player).count() == 2);
    assert_eq!(snapshot_of(&game, 0).position, cell_to_world(20, 11));
    assert_eq!(snapshot_of(&game, 1).position, cell_to_world(26, 11));

    let (p1_before, p2_before) = (snapshot_of(&game, 0).position, snapshot_of(&game, 1).position);
    for _ in 0..30 {
        step(&mut game, drive(Some(Dir::Up), Some(Dir::Down)));
    }
    let (p1, p2) = (snapshot_of(&game, 0).position, snapshot_of(&game, 1).position);
    assert!(p1.y < p1_before.y - 20.0, "player 1 drove up on its own key: {} -> {}", p1_before.y, p1.y);
    assert!(p2.y > p2_before.y + 20.0, "player 2 drove down on its own key: {} -> {}", p2_before.y, p2.y);
    assert!((p1.x - p1_before.x).abs() < 1.0 && (p2.x - p2_before.x).abs() < 1.0);

    // Only player 2's key held: player 1 coasts to a stop, player 2 keeps going.
    let p1_mid = snapshot_of(&game, 0).position;
    for _ in 0..60 {
        step(&mut game, drive(None, Some(Dir::Right)));
    }
    assert!((snapshot_of(&game, 0).position.y - p1_mid.y).abs() < 40.0, "player 1 did not keep driving on player 2's key");
    assert!(snapshot_of(&game, 1).position.x > p2.x + 40.0);
}

#[test]
fn each_fire_key_is_edge_triggered_per_player() {
    let mut game = boxed_game(5);
    let full = tuning().max_shells;
    for _ in 0..30 {
        step(&mut game, fire(true, false));
    }
    assert_eq!(snapshot_of(&game, 0).shells_ammo, full - 1, "a held key fires player 1's shell once");
    assert_eq!(snapshot_of(&game, 1).shells_ammo, full, "player 1's key never fires player 2");
    for _ in 0..30 {
        step(&mut game, fire(true, true));
    }
    assert_eq!(snapshot_of(&game, 0).shells_ammo, full - 1, "still held: no re-arm for player 1");
    assert_eq!(snapshot_of(&game, 1).shells_ammo, full - 1, "player 2's own press fires once");
    let fired: Vec<usize> = game
        .events()
        .iter()
        .filter_map(|e| if let Event::Fired { slot, .. } = e { Some(*slot) } else { None })
        .collect();
    assert!(fired.is_empty(), "the last held frame fired nothing: {fired:?}");
}

#[test]
fn a_players_shell_hits_the_other_player_as_friendly_fire() {
    // No enemies: player 2 parked six cells straight up the barrel of
    // player 1, who spawns facing up.
    let map = map_with("").replace("cells.\"26,11\" = { kind = \"start2\" }", "cells.\"20,5\" = { kind = \"start2\" }");
    let mut game = game_on(&map, 0, PlayerCount::Two, 9);
    let before = snapshot_of(&game, 1).damage;
    let mut hit = None;
    for frame in 0..120 {
        step(&mut game, fire(frame == 0, false));
        if let Some(Event::Hit { target: HitTarget::Player { player: 1 }, damage, .. }) =
            game.events().iter().find(|e| matches!(e, Event::Hit { target: HitTarget::Player { .. }, .. }))
        {
            hit = Some(*damage);
            break;
        }
    }
    let damage = hit.expect("player 1's shell landed on player 2 within two seconds");
    let (min, max) = (tuning().player_damage_min, tuning().player_damage_max);
    let factor = tuning().tank_damage_factor[0] * tuning().friendly_fire_damage_factor;
    assert!(damage >= min * factor - 1e-3 && damage <= max * factor + 1e-3, "damage {damage} outside {min}..{max} x {factor}");
    assert!(snapshot_of(&game, 1).damage > before, "player 2 took the damage");
    assert_eq!(snapshot_of(&game, 0).damage, 0.0, "player 1 is untouched by its own shell");
}

#[test]
fn one_player_wreck_keeps_the_round_going_and_both_lose_it() {
    let mut game = boxed_game(11);
    game.debug_kill(1).expect("player 2 slot");
    step(&mut game, Input::default());
    assert_eq!(game.outcome(), Outcome::Playing, "one wreck of two is not a loss");
    assert!(snapshot_of(&game, 1).is_wreck);
    // The wreck is left alone: its key drives nothing.
    let parked = snapshot_of(&game, 1).position;
    for _ in 0..30 {
        step(&mut game, drive(None, Some(Dir::Right)));
    }
    assert!(snapshot_of(&game, 1).position.distance_to(parked) < 8.0, "a dead player's key moves nothing");
    game.debug_kill(0).expect("player 1 slot");
    step(&mut game, Input::default());
    assert_eq!(game.outcome(), Outcome::Lost, "both wrecks lose the round");
}

#[test]
fn single_mode_still_loses_on_the_one_wreck() {
    let mut game = game_on(&map_with(""), 1, PlayerCount::One, 11);
    assert!(game.player2.is_none());
    assert_eq!(game.tank_snapshots().iter().filter(|t| t.is_player).count(), 1);
    assert_eq!(game.tank_snapshots().iter().find(|t| !t.is_player).map(|t| t.slot), Some(1), "enemies still count from 1");
    game.debug_kill(0).expect("player slot");
    step(&mut game, Input::default());
    assert_eq!(game.outcome(), Outcome::Lost);
}

#[test]
fn enemies_target_the_nearest_player_and_switch_only_past_the_margin() {
    let mut game = game_on(&map_with(""), 1, PlayerCount::Two, 13);
    // Player 1 at (20,11), player 2 at (26,11): the enemy four cells right
    // of player 2 is much nearer to it.
    game.debug_teleport(2, cell_to_world(30, 11), Some(270.0)).expect("enemy in slot 2");
    step(&mut game, Input::default());
    assert_eq!(enemy_target(&game, 2), 1, "the nearer player is the target");
    // Player 2 far to the left: player 1 is now nearer by well over the margin.
    game.debug_teleport(1, cell_to_world(8, 11), Some(0.0)).expect("player 2 slot");
    step(&mut game, Input::default());
    assert_eq!(enemy_target(&game, 2), 0);
    // Player 2 a little nearer than player 1, but inside the margin: no switch.
    let enemy = game.tank_snapshots().into_iter().find(|t| t.slot == 2).expect("enemy").position;
    let d1 = snapshot_of(&game, 0).position.distance_to(enemy);
    let nudge = Position::new(enemy.x - d1 + tuning().enemy_target_switch_margin_px * 0.5, enemy.y);
    game.debug_teleport(1, nudge, Some(0.0)).expect("player 2 slot");
    step(&mut game, Input::default());
    assert_eq!(enemy_target(&game, 2), 0, "a lead inside the margin does not flip the target");
    // A wreck is never a target, however near.
    game.debug_kill(0).expect("player 1 slot");
    step(&mut game, Input::default());
    assert_eq!(enemy_target(&game, 2), 1, "the surviving player is the target");
}

#[test]
fn a_missing_start2_places_player_two_beside_player_one_without_overlap() {
    // Walls around the start, so the nearest cells are not all open.
    let mut extra = String::new();
    for (c, r) in [(21, 11), (22, 11), (20, 10), (20, 12), (19, 10), (21, 10), (19, 12), (21, 12)] {
        extra.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"wall\", material = \"brick\" }}\n"));
    }
    let map = map_with(&extra).replace("cells.\"26,11\" = { kind = \"start2\" }\n", "");
    let wall_half = OBSTACLE_GRID_SIZE * 0.5;
    for seed in 1..=20u64 {
        let game = game_on(&map, 3, PlayerCount::Two, seed);
        assert!(game.map.start2_cell().is_none());
        let p1 = snapshot_of(&game, 0).position;
        let p2 = snapshot_of(&game, 1).position;
        let (hx, hy) = with_tank(&game.world, game.player2.expect("player 2"), |t| t.move_half_extents(false));
        assert!(p2.distance_to(p1) >= Tank::default().size() * 2.0 - 1.0, "seed {seed}: player 2 at {p2:?} sits on player 1 at {p1:?}");
        let walls: Vec<Position> = game.world.query::<&Obstacle>().iter().map(|o| o.position).collect();
        let overlap = walls.iter().find(|w| (p2.x - w.x).abs() < hx + wall_half && (p2.y - w.y).abs() < hy + wall_half);
        assert!(overlap.is_none(), "seed {seed}: player 2 at {p2:?} overlaps wall at {overlap:?}");
        assert!(game.nav_grid(W, H).usable(p2), "seed {seed}: player 2 placed on an unusable cell");
        for enemy in game.tank_snapshots().iter().filter(|t| !t.is_player) {
            assert!(enemy.position.distance_to(p2) >= Tank::default().size() * 1.5 - 1.0, "seed {seed}: enemy on player 2");
        }
    }
}

#[test]
fn a_frog_bite_never_kills_either_player() {
    let mut game = boxed_game(17);
    let player2 = game.player2.expect("player 2");
    {
        let mut q = game.world.query_one::<&mut Tank>(player2);
        q.get().expect("player 2 tank").damage = MAX_DAMAGE - 0.5;
    }
    let frog = cell_to_world(2, 2);
    game.debug_teleport(1, Position::new(frog.x + 40.0, frog.y), Some(0.0)).expect("player 2 slot");
    let mut bites = 0;
    for _ in 0..300 {
        step(&mut game, Input::default());
        bites += game.events().iter().filter(|e| matches!(e, Event::FrogBite { slot: 1, .. })).count();
        assert!(!snapshot_of(&game, 1).is_wreck, "a bite finished player 2 off");
    }
    assert!(bites > 0, "the frog never bit player 2 parked beside it");
    assert_eq!(game.outcome(), Outcome::Playing);
}

#[test]
fn a_two_player_round_replays_from_its_seed() {
    let run = || {
        let mut game = game_on(&map_with(""), 3, PlayerCount::Two, 21);
        for frame in 0..300u32 {
            let input = Input {
                player_intent: Intent { move_dir: Some(if frame % 40 < 20 { Dir::Left } else { Dir::Up }), fire: frame % 30 == 0, ..Intent::default() },
                player2_intent: Intent { move_dir: Some(if frame % 50 < 25 { Dir::Down } else { Dir::Right }), fire: frame % 45 == 0, ..Intent::default() },
                ..Input::default()
            };
            step(&mut game, input);
        }
        game.tank_snapshots()
            .into_iter()
            .map(|t| (t.slot, t.position.x.to_bits(), t.position.y.to_bits(), t.damage.to_bits(), t.shells_ammo))
            .collect::<Vec<_>>()
    };
    let (a, b) = (run(), run());
    assert_eq!(a, b, "two identical two-player runs diverged");
    assert!(a.len() >= 5, "two players and three enemies on the field");
}
