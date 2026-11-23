//! Headless scenario tests for rounds with more than one seat
//! (docs/two-players.md, docs/online-coop-prd.md §4.11).
//!
//! Same shape as `tap_tests`: seeded rounds on tiny inline maps, asserting
//! on `Game::events` and the public snapshot. What is worth locking in is
//! the *contract* a second and further human tank adds - its own slot, its
//! own intent, friendly fire, the every-wreck loss rule, nearest-player
//! targeting and the fallback placement - not any trajectory.

use super::*;
use crate::ai::Intent;
use crate::map::cell_to_world;
use crate::tank::Dir;

const W: f32 = 1280.0;
const H: f32 = 720.0;

/// Player 1 at (20,11), player 2 six cells to the right, the bong in the
/// far corner, one enemy boxed in iron where it can neither see nor reach
/// anything (`tap_tests`'s trick).
fn map_with(extra: &str) -> String {
    let mut text = String::from(
        r#"
version = 1
tanks = 1
cells."2,2" = { kind = "bong" }
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
        q.get().expect("player tank").shield_hp = 0.0;
    }
    game
}

/// A two-player round with its one enemy parked in the iron box.
fn boxed_game(seed: u64) -> Game {
    let mut game = game_on(&map_with(""), 1, PlayerCount::TWO, seed);
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
    Input::two(Intent { move_dir: p1, ..Intent::default() }, Intent { move_dir: p2, ..Intent::default() })
}

fn fire(p1: bool, p2: bool) -> Input {
    Input::two(Intent { fire: p1, ..Intent::default() }, Intent { fire: p2, ..Intent::default() })
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
    let mut game = game_on(&map, 0, PlayerCount::TWO, 9);
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
    let mut game = game_on(&map_with(""), 1, PlayerCount::ONE, 11);
    assert!(game.seat(1).is_none());
    assert_eq!(game.tank_snapshots().iter().filter(|t| t.is_player).count(), 1);
    assert_eq!(game.tank_snapshots().iter().find(|t| !t.is_player).map(|t| t.slot), Some(1), "enemies still count from 1");
    game.debug_kill(0).expect("player slot");
    step(&mut game, Input::default());
    assert_eq!(game.outcome(), Outcome::Lost);
}

#[test]
fn enemies_target_the_nearest_player_and_switch_only_past_the_margin() {
    let mut game = game_on(&map_with(""), 1, PlayerCount::TWO, 13);
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
        let game = game_on(&map, 3, PlayerCount::TWO, seed);
        assert!(game.map.start2_cell().is_none());
        let p1 = snapshot_of(&game, 0).position;
        let p2 = snapshot_of(&game, 1).position;
        let (hx, hy) = with_tank(&game.world, game.seat(1).expect("player 2"), |t| t.move_half_extents(false));
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

/// A bong only ever bites the other side, so it takes a Hunt round's
/// *enemy* bong to bite a player at all - and the bite still never lands
/// the killing blow, on either player.
#[test]
fn a_bong_bite_never_kills_either_player() {
    let map = map_with("mission.kind = \"hunt\"\ncells.\"30,4\" = { kind = \"enemy_bong\" }\n");
    let mut game = game_on(&map, 1, PlayerCount::TWO, 17);
    game.debug_teleport(2, cell_to_world(37, 20), Some(0.0)).expect("enemy in slot 2");
    let enemy_bong = game.enemy_bong.expect("the hunt map places an enemy bong");
    let player2 = game.seat(1).expect("player 2");
    {
        let mut q = game.world.query_one::<&mut Tank>(player2);
        q.get().expect("player 2 tank").damage = MAX_DAMAGE - 0.5;
    }
    let mut bites = 0;
    for _ in 0..300 {
        // Follow the bong: it hops away from whatever crowds it.
        let (pos, range) = with_bong(&game.world, enemy_bong, |fr| (fr.position, fr.attack_range()));
        game.debug_teleport(1, Position::new(pos.x + range * 0.5, pos.y), Some(0.0)).expect("player 2 slot");
        step(&mut game, Input::default());
        bites += game.events().iter().filter(|e| matches!(e, Event::BongBite { slot: 1, .. })).count();
        assert!(!snapshot_of(&game, 1).is_wreck, "a bite finished player 2 off");
    }
    assert!(bites > 0, "the enemy bong never bit player 2 parked beside it");
    assert_eq!(game.outcome(), Outcome::Playing);
}

#[test]
fn a_two_player_round_replays_from_its_seed() {
    let run = || {
        let mut game = game_on(&map_with(""), 3, PlayerCount::TWO, 21);
        for frame in 0..300u32 {
            let input = Input::two(
                Intent { move_dir: Some(if frame % 40 < 20 { Dir::Left } else { Dir::Up }), fire: frame % 30 == 0, ..Intent::default() },
                Intent { move_dir: Some(if frame % 50 < 25 { Dir::Down } else { Dir::Right }), fire: frame % 45 == 0, ..Intent::default() },
            );
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

/// Every seat's intent, seat 0 first: what a room hands `Game::update`.
fn drive_all(dirs: &[Option<Dir>]) -> Input {
    let mut input = Input::default();
    for (seat, &dir) in input.seats.iter_mut().zip(dirs) {
        *seat = Intent { move_dir: dir, ..Intent::default() };
    }
    input
}

/// Four seats on a map that authors two starts: the two it names, then two
/// more placed beside player 1. Each drives on its own intent, each has its
/// own slot, and the enemies count from four.
#[test]
fn four_seats_spawn_clear_of_each_other_and_drive_independently() {
    let mut game = game_on(&map_with(""), 1, PlayerCount::from_count(4).expect("four seats"), 23);
    game.debug_teleport(4, cell_to_world(37, 20), Some(0.0)).expect("enemy in slot 4");

    let snaps = game.tank_snapshots();
    assert_eq!(snaps.iter().filter(|t| t.is_player).map(|t| t.slot).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
    assert_eq!(snaps.iter().filter(|t| !t.is_player).map(|t| t.slot).collect::<Vec<_>>(), vec![4], "enemies count from 4");
    assert_eq!(game.first_enemy_slot(), 4);
    // The authored starts still win for the seats that have one.
    assert_eq!(snapshot_of(&game, 0).position, cell_to_world(20, 11));
    assert_eq!(snapshot_of(&game, 1).position, cell_to_world(26, 11));
    // Nobody stands on anybody, and every seat is somewhere a tank can be.
    let grid = game.nav_grid(W, H);
    let places: Vec<Position> = (0..4).map(|i| snapshot_of(&game, i).position).collect();
    for (i, &a) in places.iter().enumerate() {
        assert!(grid.usable(a), "seat {i} at {a:?} is on an unusable cell");
        for (j, &b) in places.iter().enumerate().skip(i + 1) {
            assert!(a.distance_to(b) >= Tank::default().size() * 2.0 - 1.0, "seats {i} and {j} overlap: {a:?} {b:?}");
        }
    }

    // Four intents, four directions, nobody driven by anybody else's.
    let before = places.clone();
    for _ in 0..30 {
        step(&mut game, drive_all(&[Some(Dir::Up), Some(Dir::Down), Some(Dir::Up), Some(Dir::Down)]));
    }
    for (i, &was) in before.iter().enumerate() {
        let now = snapshot_of(&game, i as u8).position;
        let moved = if i % 2 == 0 { was.y - now.y } else { now.y - was.y };
        assert!(moved > 20.0, "seat {i} did not drive on its own intent: {was:?} -> {now:?}");
    }
}

/// The retarget rule over four seats: the nearest live, visible player
/// wins, the margin still guards a near-tie, and a wreck is never a target.
#[test]
fn enemies_retarget_across_four_seats() {
    let mut game = game_on(&map_with(""), 1, PlayerCount::from_count(4).expect("four seats"), 29);
    // Every seat in a row, the enemy just past the last of them.
    for (seat, col) in [(0usize, 6), (1, 12), (2, 18), (3, 24)] {
        game.debug_teleport(seat, cell_to_world(col, 11), Some(0.0)).expect("a seat");
    }
    game.debug_teleport(4, cell_to_world(28, 11), Some(270.0)).expect("enemy in slot 4");
    step(&mut game, Input::default());
    assert_eq!(enemy_target(&game, 4), 3, "the nearest seat is the target");

    // That seat driven far away: the next-nearest takes over, being nearer
    // by well over the margin.
    game.debug_teleport(3, cell_to_world(2, 11), Some(0.0)).expect("seat 3");
    step(&mut game, Input::default());
    assert_eq!(enemy_target(&game, 4), 2);

    // Seat 1 a little nearer than seat 2, but inside the margin: no switch.
    let enemy = game.tank_snapshots().into_iter().find(|t| t.slot == 4).expect("enemy").position;
    let held = snapshot_of(&game, 2).position.distance_to(enemy);
    let nudge = Position::new(enemy.x - (held - tuning().enemy_target_switch_margin_px * 0.5), enemy.y);
    game.debug_teleport(1, nudge, Some(0.0)).expect("seat 1");
    step(&mut game, Input::default());
    assert_eq!(enemy_target(&game, 4), 2, "a lead inside the margin does not flip the target");

    // The target wrecked: the enemy picks a live seat at once, and the
    // nearest of them.
    game.debug_kill(2).expect("seat 2");
    step(&mut game, Input::default());
    assert_eq!(enemy_target(&game, 4), 1, "a wreck is never a target");
}

/// A ring per seat: put a pack around each of two seats and every enemy
/// in each pack is handed a distinct engagement slot on its own seat's
/// ring. A Destroy round, so no enemy rolls a hunter and competes on the
/// bong's ring instead.
#[test]
fn every_seat_gets_its_own_engagement_ring() {
    let map = map_with("mission.kind = \"destroy\"\n");
    let mut game = game_on(&map, 6, PlayerCount::from_count(4).expect("four seats"), 31);
    // Two seats far apart, the other two parked out of the way; three
    // enemies crowd each of the first pair.
    game.debug_teleport(0, cell_to_world(8, 11), Some(0.0)).expect("seat 0");
    game.debug_teleport(1, cell_to_world(32, 11), Some(0.0)).expect("seat 1");
    game.debug_teleport(2, cell_to_world(8, 20), Some(0.0)).expect("seat 2");
    game.debug_teleport(3, cell_to_world(32, 20), Some(0.0)).expect("seat 3");
    for (i, (col, row)) in [(7, 9), (9, 9), (8, 8), (31, 9), (33, 9), (32, 8)].into_iter().enumerate() {
        game.debug_teleport(4 + i, cell_to_world(col, row), Some(180.0)).expect("an enemy");
    }
    for _ in 0..10 {
        step(&mut game, Input::default());
    }
    let report = game.debug_snapshot(W, H, crate::simulation::debug::Detail::Full);
    let rings_of = |player: u8| -> Vec<Option<u8>> {
        game.world
            .query::<(&Tank, &Ai)>()
            .iter()
            .filter(|(_, ai)| ai.target_player() == player)
            .map(|(t, _)| report.engage.tanks.iter().find(|r| r.slot == t.owner_slot()).and_then(|r| r.ring))
            .collect()
    };
    for player in [0u8, 1] {
        let rings = rings_of(player);
        assert!(rings.len() >= 2, "seat {player} is not being fought by a pack: {rings:?}");
        let mut held: Vec<u8> = rings.iter().copied().flatten().collect();
        assert!(held.len() >= 2, "seat {player}'s own ring handed out no slots: {rings:?}");
        let before = held.len();
        held.sort();
        held.dedup();
        assert_eq!(held.len(), before, "seat {player}'s ring handed the same slot out twice");
    }
}

/// The co-op end rule over four seats: the round runs until every seat is
/// a wreck, and the bong's death ends it whatever the seats are doing.
#[test]
fn four_seats_lose_only_once_every_one_is_wrecked() {
    let mut game = game_on(&map_with(""), 1, PlayerCount::from_count(4).expect("four seats"), 37);
    game.debug_teleport(4, cell_to_world(37, 20), Some(0.0)).expect("enemy in slot 4");
    for seat in 0..3 {
        game.debug_kill(seat).expect("a seat");
        step(&mut game, Input::default());
        assert_eq!(game.outcome(), Outcome::Playing, "{} wrecks of four is not a loss", seat + 1);
    }
    game.debug_kill(3).expect("the last seat");
    step(&mut game, Input::default());
    assert_eq!(game.outcome(), Outcome::Lost, "every seat wrecked loses the round");

    // The bong still ends it on its own, with every seat alive.
    let mut game = game_on(&map_with(""), 1, PlayerCount::from_count(4).expect("four seats"), 41);
    with_bong_mut(&game.world, game.bong.expect("the map places a bong"), |fr| fr.damage(fr.max_health));
    step(&mut game, Input::default());
    assert_eq!(game.outcome(), Outcome::Lost, "the bong's death loses it whoever is alive");
}

/// The seats past the authored starts come off the default map's own
/// terrain: inside the playfield, out of the water, clear of each other -
/// and seats 1 and 2 land exactly where a two-player round puts them.
#[test]
fn n_seats_on_the_shipped_map_stand_where_a_tank_can() {
    let map = MapFile::from_toml_str(map::SHIPPED_MAPS[0].1).expect("the default map parses");
    let (w, h) = map.field_size();
    let round = |count: usize| {
        let mut game = Game::default();
        game.seed_override = Some(0xB0B5);
        game.enemy_count_override = Some(4);
        game.players = PlayerCount::from_count(count).expect("a legal seat count");
        game.map = map.clone();
        game.init(w, h);
        game
    };
    let two = round(2);
    let couch: Vec<Position> = (0..2).map(|i| snapshot_of(&two, i).position).collect();

    for count in 1..=crate::MAX_SEATS {
        let game = round(count);
        let grid = game.nav_grid(w, h);
        let places: Vec<Position> = (0..count).map(|i| snapshot_of(&game, i as u8).position).collect();
        assert_eq!(places.len(), count, "{count} seats hold {count} tanks");
        for (i, &p) in places.iter().enumerate() {
            assert!(p.x > 0.0 && p.x < w && p.y > 0.0 && p.y < h, "{count} seats: seat {i} at {p:?} is off the field");
            assert!(grid.usable(p), "{count} seats: seat {i} at {p:?} is on an unusable cell");
            assert_ne!(
                game.water().depth_at(p),
                crate::ground::Depth::Deep,
                "{count} seats: seat {i} at {p:?} is in a lake"
            );
            for (j, &q) in places.iter().enumerate().skip(i + 1) {
                assert!(
                    p.distance_to(q) >= Tank::default().size() * 2.0 - 1.0,
                    "{count} seats: {i} and {j} overlap at {p:?} {q:?}"
                );
            }
        }
        // Seats 1 and 2 never move as the round grows: the couch round's
        // placement is the first two entries of every larger one.
        for (i, &was) in couch.iter().enumerate().take(count) {
            assert_eq!(places[i], was, "{count} seats: seat {i} moved off the two-player placement");
        }
    }
}

/// A four-seat round replays from its seed like any other: every seat's
/// spawn rolls sit on the one round stream, in seat order.
/// `determinism_tests::the_one_and_two_seat_streams_are_pinned` is the
/// other half - that one and two seats draw what they always drew.
#[test]
fn a_four_seat_round_replays_from_its_seed() {
    let run = || {
        let mut game = game_on(&map_with(""), 4, PlayerCount::from_count(4).expect("four seats"), 43);
        for frame in 0..300u32 {
            let turn = |offset: u32| Some(if (frame + offset) % 40 < 20 { Dir::Left } else { Dir::Up });
            step(&mut game, drive_all(&[turn(0), turn(10), turn(20), turn(30)]));
        }
        game.tank_snapshots()
            .into_iter()
            .map(|t| (t.slot, t.position.x.to_bits(), t.position.y.to_bits(), t.damage.to_bits(), t.shells_ammo))
            .collect::<Vec<_>>()
    };
    let (a, b) = (run(), run());
    assert_eq!(a, b, "two identical four-seat runs diverged");
    assert!(a.len() >= 8, "four seats and four enemies on the field");
}

/// **Stage 3's validator** (docs/online-coop-prd.md §4.14): a pose the
/// hull could have reached is taken and the seat is the client's for the
/// tick - `drive_player` leaves it alone and a shot leaves from it - while
/// a pose out of reach, inside a tile or from a wreck is refused and the
/// hull stays.
#[test]
fn an_owned_pose_is_taken_when_reachable_and_refused_otherwise() {
    use super::{POSE_REACH_SLACK_PX, POSE_REACH_TICKS, SeatPose};
    let mut game = game_on(&map_with(""), 0, PlayerCount::ONE, 7);
    let from = game.seat_pose(0).expect("the seat");
    let reach = game.tank_snapshots()[0].top_speed * PHYSICS_FIXED_DT * POSE_REACH_TICKS + POSE_REACH_SLACK_PX;
    let still = Vec2::new(0.0, 0.0);

    // A step within reach is taken, facing and all.
    let near = SeatPose { position: Position::new(from.position.x + reach * 0.5, from.position.y), rotation: Dir::Right.rotation(), velocity: still, ram: still };
    assert_eq!(game.accept_seat_pose(0, near, 1), Ok(()));
    assert!(game.seat_is_owned(0));
    let at = game.seat_pose(0).expect("the seat");
    assert!((at.position.x - near.position.x).abs() < 0.01 && at.rotation == Dir::Right.rotation(), "{at:?}");

    // The tick drives nothing on an owned seat: a stick held left leaves
    // it where the pose put it.
    let left = Intent { move_dir: Some(Dir::Left), ..Intent::default() };
    game.update(Input::single(left), PHYSICS_FIXED_DT, W, H);
    let after = game.seat_pose(0).expect("the seat");
    assert!((after.position.x - near.position.x).abs() < 1.0, "an owned seat was driven: {after:?}");
    // A moving pose ends the tick on the pose, not a tick past it: the
    // room's copy is the client's, not one step ahead of it.
    let here = game.seat_pose(0).expect("the seat");
    let moving = SeatPose {
        position: Position::new(here.position.x + 3.0, here.position.y),
        rotation: Dir::Right.rotation(),
        velocity: Vec2::new(180.0, 0.0),
        ram: Vec2::new(180.0, 0.0),
    };
    assert_eq!(game.accept_seat_pose(0, moving, 1), Ok(()));
    game.update(Input::default(), PHYSICS_FIXED_DT, W, H);
    let landed = game.seat_pose(0).expect("the seat");
    assert!((landed.position.x - moving.position.x).abs() < 0.5, "the room's hull ended at {landed:?}, not the pose {moving:?}");
    // Ownership lasts one update: a seat nobody reports is the room's
    // again, and the stick drives it, with no release needed.
    for _ in 0..30 {
        game.update(Input::single(left), PHYSICS_FIXED_DT, W, H);
    }
    assert!(game.seat_pose(0).expect("the seat").position.x < near.position.x - 5.0, "a released seat did not drive");

    // Out of reach: refused, the hull stays.
    let here = game.seat_pose(0).expect("the seat");
    let far = SeatPose { position: Position::new(here.position.x + reach * 3.0, here.position.y), ..here };
    assert_eq!(game.accept_seat_pose(0, far, 1), Err("further than the hull could have gone"));
    assert!(!game.seat_is_owned(0));
    assert_eq!(game.seat_pose(0).expect("the seat").position, here.position);
    // Outside the field: refused.
    let outside = SeatPose { position: Position::new(-4.0, here.position.y), ..here };
    assert_eq!(game.accept_seat_pose(0, outside, 1), Err("further than the hull could have gone"));
    // A wreck owns nothing.
    game.debug_kill(0).expect("the seat");
    game.update(Input::default(), PHYSICS_FIXED_DT, W, H);
    assert_eq!(game.accept_seat_pose(0, here, 1), Err("a wreck"));
}

/// A coat of ooze slows a hull, so the validator holds an owned seat's
/// coated hull to the slower pace (`Tank::slime_pace`) - from
/// `POSE_SLIME_GRACE_TICKS` after the coat went on, the client's time to
/// hear of it; until then a pose at the clean pace is the client's honest
/// drive, and is taken.
#[test]
fn an_owned_pose_outrunning_its_coat_is_refused_once_its_client_knows() {
    use super::{POSE_REACH_SLACK_PX, POSE_SLIME_GRACE_TICKS, SeatPose};
    let mut game = game_on(&map_with(""), 0, PlayerCount::ONE, 7);
    let from = game.seat_pose(0).expect("the seat");
    // A puddle under the hull: the next tick coats it.
    game.ooze.insert(crate::map::world_to_cell(from.position), crate::tower::OozePuddle { left: 10.0, total: 10.0 });
    step(&mut game, Input::default());
    assert!(game.events().iter().any(|e| matches!(e, Event::Slimed { slot: 0 })), "the seat was coated");
    // A read that ends a stall: thirty ticks of driving in one pose, a
    // step between what a coated hull and a clean one could cover.
    let ticks = 30;
    let seat = game.players()[0].expect("the seat");
    let between = |game: &Game| {
        let top = with_tank(&game.world, seat, |t| t.effective_speed());
        let clean = top * PHYSICS_FIXED_DT * ticks as f32 + POSE_REACH_SLACK_PX;
        let coated = top * tuning().bio_slime_speed_factor * PHYSICS_FIXED_DT * ticks as f32 + POSE_REACH_SLACK_PX;
        let here = game.seat_pose(0).expect("the seat");
        SeatPose { position: Position::new(here.position.x + (clean + coated) * 0.5, here.position.y), ..here }
    };
    // Straight after the coat: the clean pace is still the client's.
    assert_eq!(game.accept_seat_pose(0, between(&game), ticks), Ok(()));
    for _ in 0..POSE_SLIME_GRACE_TICKS {
        step(&mut game, Input::default());
    }
    assert!(!game.slimed().is_empty(), "the coat still runs");
    let here = game.seat_pose(0).expect("the seat").position;
    assert_eq!(game.accept_seat_pose(0, between(&game), ticks), Err("further than the hull could have gone"));
    assert_eq!(game.seat_pose(0).expect("the seat").position, here, "a refused pose leaves the hull");
    // Washed clean, the same step is a clean hull's.
    with_tank_mut(&game.world, seat, |t| t.slime_timer = 0.0);
    assert_eq!(game.accept_seat_pose(0, between(&game), ticks), Ok(()));
}

/// A pose inside a solid tile is refused even when it is within reach:
/// the client cannot say it is in a wall.
#[test]
fn an_owned_pose_inside_a_tile_is_refused() {
    use super::{POSE_REACH_SLACK_PX, POSE_REACH_TICKS, SeatPose};
    // Where the reach ends, one round on the plain map says; the next
    // round puts an iron cell there.
    let probe = game_on(&map_with(""), 0, PlayerCount::ONE, 7);
    let from = probe.seat_pose(0).expect("the seat");
    let reach = probe.tank_snapshots()[0].top_speed * PHYSICS_FIXED_DT * POSE_REACH_TICKS + POSE_REACH_SLACK_PX;
    let target = Position::new(from.position.x + reach - 1.0, from.position.y);
    let (col, row) = crate::map::world_to_cell(target);
    assert_ne!((col, row), crate::map::world_to_cell(from.position), "the reach has to cross into the next cell for this fixture");
    let mut game = game_on(&map_with(&format!("cells.\"{col},{row}\" = {{ kind = \"wall\", material = \"iron\" }}")), 0, PlayerCount::ONE, 7);
    let from = game.seat_pose(0).expect("the seat");
    let step = SeatPose { position: target, ..from };
    assert_eq!(game.accept_seat_pose(0, step, 1), Err("inside a solid tile"));
    assert_eq!(game.seat_pose(0).expect("the seat").position, from.position);
}

/// The pose validator reads the tiles standing now, not the map they were
/// loaded from: a pose over a wall that has died, or over the cell a loose
/// drum was shoved off, is taken, while one inside a standing wall is still
/// refused.
#[test]
fn an_owned_pose_over_a_dead_tile_or_a_shoved_drums_cell_is_taken() {
    use super::props::DamageCause;
    use super::{POSE_REACH_SLACK_PX, POSE_REACH_TICKS, SeatPose};
    // Where the reach ends, one round on the plain map says; the next
    // rounds put a tile there.
    let probe = game_on(&map_with(""), 0, PlayerCount::ONE, 7);
    let from = probe.seat_pose(0).expect("the seat");
    let reach = probe.tank_snapshots()[0].top_speed * PHYSICS_FIXED_DT * POSE_REACH_TICKS + POSE_REACH_SLACK_PX;
    let target = Position::new(from.position.x + reach - 1.0, from.position.y);
    let (col, row) = crate::map::world_to_cell(target);
    assert_ne!((col, row), crate::map::world_to_cell(from.position), "the reach has to cross into the next cell for this fixture");
    let tile_at = |game: &Game| game.world.query::<(Entity, &Obstacle)>().iter().find(|(_, o)| o.key_cell() == (col, row) && !o.destroyed).map(|(e, _)| e);

    // A brick wall: refused while it stands, taken once it has died.
    let mut game = game_on(&map_with(&format!("cells.\"{col},{row}\" = {{ kind = \"wall\", material = \"brick\" }}")), 0, PlayerCount::ONE, 7);
    let from = game.seat_pose(0).expect("the seat");
    let step = SeatPose { position: target, ..from };
    assert_eq!(game.accept_seat_pose(0, step, 1), Err("inside a solid tile"));
    let wall = tile_at(&game).expect("the brick stands");
    let mut f = Frame::new(PHYSICS_FIXED_DT, W, H, game.rng.take().expect("a seeded round"), Terrain::build(&game.world, W, H, &game.grass_cells, &game.water));
    assert!(game.damage_obstacle(&mut f, wall, f32::MAX, DamageCause::Ram), "the brick died");
    game.rng = Some(f.rng);
    step_round(&mut game);
    assert!(tile_at(&game).is_none(), "the brick is gone");
    let from = game.seat_pose(0).expect("the seat");
    assert_eq!(game.accept_seat_pose(0, SeatPose { position: target, ..from }, 1), Ok(()), "a pose over the dead brick's cell");

    // A loose drum shoved three cells on: its home cell is open ground.
    let mut game = game_on(&map_with(&format!("cells.\"{col},{row}\" = {{ kind = \"barrel\" }}")), 0, PlayerCount::ONE, 7);
    let drum = tile_at(&game).expect("the drum stands");
    let body = game.world.get::<&Obstacle>(drum).expect("the drum").body;
    game.physics.set_position(body, cell_to_world(col + 3, row));
    step_round(&mut game);
    assert_ne!(game.world.get::<&Obstacle>(drum).expect("the drum").cell(), (col, row), "the drum left its cell");
    let from = game.seat_pose(0).expect("the seat");
    assert_eq!(game.accept_seat_pose(0, SeatPose { position: target, ..from }, 1), Ok(()), "a pose over the shoved drum's home cell");
}

/// One step of a round with nobody driving.
fn step_round(game: &mut Game) {
    step(game, Input::default());
}

/// **One `Placed` answers a run of refusals** (docs/online-coop-prd.md
/// §4.14): the room answers the first refused pose with where it has the
/// hull and holds to that for `PLACED_HOLD_TICKS`, sending nothing for the
/// poses refused meanwhile; and a pose within the driving since of that
/// `Placed` is taken however far the room's own copy has gone on since.
#[test]
fn a_refused_pose_is_answered_once_and_the_client_is_taken_back_from_it() {
    use super::{PLACED_HOLD_TICKS, POSE_REACH_SLACK_PX, POSE_REACH_TICKS, SeatPose};
    let mut game = game_on(&map_with(""), 0, PlayerCount::ONE, 7);
    let from = game.seat_pose(0).expect("the seat");
    let speed = game.tank_snapshots()[0].top_speed;
    let reach = speed * PHYSICS_FIXED_DT * POSE_REACH_TICKS + POSE_REACH_SLACK_PX;
    let far = SeatPose { position: Position::new(from.position.x, from.position.y - reach * 3.0), ..from };
    assert_eq!(game.accept_seat_pose(0, far, 1), Err("further than the hull could have gone"));
    let placed = game.place_answer(0).expect("the first refusal is answered");
    assert_eq!(placed.position, from.position, "with where the room has the hull");
    assert_eq!(game.accept_seat_pose(0, far, 1), Err("further than the hull could have gone"));
    assert_eq!(game.place_answer(0), None, "a refusal while the Placed is held is not answered again");

    // The room drives its copy on while the client hears of the Placed:
    // twenty ticks right, further than one pose's reach from where it was.
    let right = Intent { move_dir: Some(Dir::Right), ..Intent::default() };
    for _ in 0..20 {
        step(&mut game, Input::single(right));
    }
    let copy = game.seat_pose(0).expect("the seat");
    let still = Vec2::zero();
    let back = SeatPose { position: Position::new(placed.position.x + speed * PHYSICS_FIXED_DT, placed.position.y), velocity: still, ram: still, ..copy };
    assert!(copy.position.distance_to(back.position) > reach, "the fixture's copy went on past one pose's reach");
    assert_eq!(game.accept_seat_pose(0, back, 1), Ok(()), "the client's pose a tick on from the Placed is taken");
    assert!((game.seat_pose(0).expect("the seat").position.x - back.position.x).abs() < 0.5, "and the hull is put there");

    // Once the hold is over, a refusal is answered again.
    for _ in 0..=PLACED_HOLD_TICKS {
        step_round(&mut game);
    }
    let here = game.seat_pose(0).expect("the seat");
    let far = SeatPose { position: Position::new(here.position.x, here.position.y - reach * 3.0), ..here };
    assert_eq!(game.accept_seat_pose(0, far, 1), Err("further than the hull could have gone"));
    assert!(game.place_answer(0).is_some(), "past the hold a refusal is answered");
}

// --- A seat driving back in through a gate (docs/online-coop-prd.md
// section 4.11): ordnance leaves it alone until it is on the field ---

/// A two-seat waves round of one tank a wave with seat 1 wrecked and
/// already driving back in with wave 2; seat 1's tank.
fn round_with_seat_one_rolling_in() -> (Game, Entity) {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.player_row_override = Some(0);
    game.players = PlayerCount::TWO;
    game.level_overrides.mission = Some(Mission::Destroy);
    game.level_overrides.spawn = Some(crate::level::SpawnKind::Waves);
    game.level_overrides.waves = Some(3);
    game.level_overrides.wave_size = Some(1);
    game.level_overrides.wave_growth = Some(0);
    game.map = MapFile::from_toml_str("version = 1\ntanks = 1\ncells.\"20,11\" = { kind = \"start\" }\ncells.\"30,20\" = { kind = \"bong\" }\n")
        .expect("test map parses");
    game.init(W, H);
    for seat in game.players().into_iter().flatten() {
        with_tank_mut(&game.world, seat, |t| t.shield_hp = 1.0e9);
    }
    let entered = |game: &Game| {
        game.events().iter().find_map(|e| match e {
            Event::TankEntered { slot } => Some(*slot),
            _ => None,
        })
    };
    let mut wave_one = None;
    for _ in 0..600 {
        step(&mut game, Input::default());
        wave_one = entered(&game);
        if wave_one.is_some() {
            break;
        }
    }
    let wave_one = wave_one.expect("wave 1's tank drives in");
    game.debug_kill(1).expect("seat 1 is alive");
    step(&mut game, Input::default());
    game.debug_kill(wave_one).expect("wave 1's tank");
    let seat1 = game.seat(1).expect("seat 1");
    for _ in 0..900 {
        step(&mut game, Input::default());
        if game.is_entering(seat1) {
            return (game, seat1);
        }
    }
    panic!("seat 1 never started back in");
}

/// A missile of `owner` on the ground at `at`, to burst on the next frame.
fn landed_missile(game: &mut Game, at: Position, owner: Owner) {
    let mut missile = crate::missile::Missile::spawn(at, Vec2::new(0.0, 1.0), owner, 0, at);
    missile.locked = true;
    missile.arrived = true;
    game.world.spawn((missile,));
}

#[test]
fn a_blast_beside_a_seat_driving_back_in_neither_hurts_nor_shoves_it() {
    let (mut game, seat1) = round_with_seat_one_rolling_in();
    // An enemy's burst would hurt a seat on the field and a seat's own
    // side's would only shove it; out in the gate lane neither does either.
    for owner in [Owner::Enemy(game.first_enemy_slot()), Owner::Player(0)] {
        assert!(game.is_entering(seat1), "still in the gate lane");
        let before = with_tank(&game.world, seat1, |t| (t.position, t.damage, t.shield_hp, t.hull_points()));
        landed_missile(&mut game, before.0, owner);
        step(&mut game, Input::default());
        assert!(game.events().iter().any(|e| matches!(e, Event::MissileBlast { .. })), "the missile burst: {:?}", game.events());
        let after = with_tank(&game.world, seat1, |t| (t.damage, t.shield_hp, t.hull_points(), t.is_wreck()));
        assert_eq!(after, (before.1, before.2, before.3, false), "{owner:?}'s blast left it whole");
        assert!(!game.events().iter().any(|e| matches!(e, Event::Shoved { seat: 1, .. })), "and unshoved: {:?}", game.events());
    }
}

#[test]
fn an_enemy_missile_does_not_lock_a_seat_driving_back_in() {
    let (mut game, seat1) = round_with_seat_one_rolling_in();
    // A launcher on the field just inside the gate, so the seat in the
    // lane is well inside its sight box, and its missile at the top of the
    // climb right over that seat - the nearest tank to it by far.
    let inside = game.world.get::<&RollIn>(seat1).expect("rolling in").to;
    let launcher = game.debug_spawn_enemy(inside, None, None).expect("spawns");
    let at = with_tank(&game.world, seat1, |t| t.position);
    assert!(crate::ai::in_sight_box(at, inside), "the launcher could see a seat standing there");
    let mut missile = crate::missile::Missile::spawn(at, Vec2::new(0.0, 1.0), Owner::Enemy(launcher), 0, at);
    missile.stage = crate::missile::MissileStage::Seek;
    game.world.spawn((missile,));
    step(&mut game, Input::default());
    let locks: Vec<Option<usize>> = game
        .events()
        .iter()
        .filter_map(|e| match e {
            Event::MissileLocked { slot, target, .. } if *slot == launcher => Some(*target),
            _ => None,
        })
        .collect();
    assert_eq!(locks.len(), 1, "the missile made its pick: {:?}", game.events());
    assert_ne!(locks[0], Some(1), "not the seat in the gate lane");
    assert!(game.world.query::<&crate::missile::Missile>().iter().all(|m| m.target != Some(seat1)));
}
