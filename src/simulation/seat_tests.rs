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

/// A frog only ever bites the other side, so it takes a Hunt round's
/// *enemy* frog to bite a player at all - and the bite still never lands
/// the killing blow, on either player.
#[test]
fn a_frog_bite_never_kills_either_player() {
    let map = map_with("mission.kind = \"hunt\"\ncells.\"30,4\" = { kind = \"enemy_frog\" }\n");
    let mut game = game_on(&map, 1, PlayerCount::TWO, 17);
    game.debug_teleport(2, cell_to_world(37, 20), Some(0.0)).expect("enemy in slot 2");
    let enemy_frog = game.enemy_frog.expect("the hunt map places an enemy frog");
    let player2 = game.seat(1).expect("player 2");
    {
        let mut q = game.world.query_one::<&mut Tank>(player2);
        q.get().expect("player 2 tank").damage = MAX_DAMAGE - 0.5;
    }
    let mut bites = 0;
    for _ in 0..300 {
        // Follow the frog: it hops away from whatever crowds it.
        let (pos, range) = with_frog(&game.world, enemy_frog, |fr| (fr.position, fr.attack_range()));
        game.debug_teleport(1, Position::new(pos.x + range * 0.5, pos.y), Some(0.0)).expect("player 2 slot");
        step(&mut game, Input::default());
        bites += game.events().iter().filter(|e| matches!(e, Event::FrogBite { slot: 1, .. })).count();
        assert!(!snapshot_of(&game, 1).is_wreck, "a bite finished player 2 off");
    }
    assert!(bites > 0, "the enemy frog never bit player 2 parked beside it");
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
/// frog's ring instead.
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
/// a wreck, and the frog's death ends it whatever the seats are doing.
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

    // The frog still ends it on its own, with every seat alive.
    let mut game = game_on(&map_with(""), 1, PlayerCount::from_count(4).expect("four seats"), 41);
    with_frog_mut(&game.world, game.frog.expect("the map places a frog"), |fr| fr.damage(fr.max_health));
    step(&mut game, Input::default());
    assert_eq!(game.outcome(), Outcome::Lost, "the frog's death loses it whoever is alive");
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
