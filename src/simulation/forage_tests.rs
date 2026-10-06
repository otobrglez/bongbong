//! The mushroom hunt's rules (docs/mushroom-hunt-prd.md, `forage.rs`) on
//! tiny inline maps.

use super::*;

const W: f32 = 1280.0;
const H: f32 = 720.0;

/// A map with the player's start at (5, 11), a frog cell, an iron box at
/// the far east for the enemies to start in, and `extra` cells.
fn map_with(extra: &str) -> String {
    let mut text = String::from("version = 1\ncells.\"5,11\" = { kind = \"start\" }\ncells.\"5,15\" = { kind = \"frog\" }\n");
    text.push_str(extra);
    text
}

fn mushrooms(cells: &[(i32, i32)]) -> String {
    cells.iter().map(|(c, r)| format!("cells.\"{c},{r}\" = {{ kind = \"mushroom\" }}\n")).collect()
}

fn game_on(map: &str, mission: Mission, enemies: usize, players: PlayerCount) -> Game {
    let mut game = Game::default();
    game.enemy_count_override = Some(enemies);
    game.seed_override = Some(11);
    game.player_row_override = Some(0);
    game.player2_row_override = Some(0);
    game.players = players;
    game.level_overrides.mission = Some(mission);
    game.map = MapFile::from_toml_str(map).expect("test map parses");
    game.init(W, H);
    game
}

fn step(game: &mut Game) {
    game.update(Input::default(), PHYSICS_FIXED_DT, W, H);
}

fn put_seat(game: &mut Game, seat: usize, pos: Position) {
    let entity = game.seat(seat).expect("seat");
    let body = with_tank(&game.world, entity, |t| t.body).expect("body");
    game.physics.set_position(body, pos);
    with_tank_mut(&game.world, entity, |t| t.position = pos);
}

fn taken(game: &Game) -> Vec<(u8, i16, i16, u16)> {
    game.events()
        .iter()
        .filter_map(|e| match *e {
            Event::MushroomTaken { seat, col, row, left } => Some((seat, col, row, left)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_seat_driving_into_a_mushroom_takes_it() {
    let mut game = game_on(&map_with(&mushrooms(&[(20, 11), (30, 4)])), Mission::Forage, 0, PlayerCount::ONE);
    assert_eq!((game.mushrooms_left(), game.mushrooms().len()), (2, 2));
    // Two cells away: out of reach.
    put_seat(&mut game, 0, crate::map::cell_to_world(18, 11));
    step(&mut game);
    assert!(taken(&game).is_empty(), "taken from two cells away");
    put_seat(&mut game, 0, crate::map::cell_to_world(19, 11));
    step(&mut game);
    assert_eq!(taken(&game), vec![(0, 20, 11, 1)]);
    assert_eq!(game.mushrooms_left(), 1);
    assert_eq!(game.outcome(), Outcome::Playing, "one is still out");
    step(&mut game);
    assert!(taken(&game).is_empty(), "a mushroom is taken once");
}

#[test]
fn the_last_mushroom_wins_a_forage_round_with_enemies_still_standing() {
    let mut game = game_on(&map_with(&mushrooms(&[(20, 11)])), Mission::Forage, 2, PlayerCount::ONE);
    let enemies = game.tank_snapshots().iter().filter(|t| !t.is_player).count();
    assert!(enemies > 0, "the round has enemies");
    put_seat(&mut game, 0, crate::map::cell_to_world(20, 11));
    step(&mut game);
    assert_eq!(game.outcome(), Outcome::Won);
    assert!(game.events().iter().any(|e| matches!(e, Event::RoundEnded { outcome: Outcome::Won })));
}

#[test]
fn a_forage_round_has_no_frog_even_on_a_map_with_one() {
    let game = game_on(&map_with(&mushrooms(&[(20, 11)])), Mission::Forage, 0, PlayerCount::ONE);
    assert!(game.frog.is_none() && game.enemy_frog.is_none(), "a forage round spawned a frog");
    assert_eq!(game.frog_position(), None);
}

#[test]
fn a_forage_map_with_no_mushrooms_is_never_won() {
    let mut game = game_on(&map_with(""), Mission::Forage, 0, PlayerCount::ONE);
    for _ in 0..10 {
        step(&mut game);
    }
    assert_eq!(game.outcome(), Outcome::Playing);
}

#[test]
fn every_seat_wrecked_loses_a_forage_round() {
    let mut game = game_on(&map_with(&mushrooms(&[(20, 11)])), Mission::Forage, 0, PlayerCount::ONE);
    let seat = game.seat(0).expect("seat");
    with_tank_mut(&game.world, seat, |t| t.damage = crate::MAX_DAMAGE);
    step(&mut game);
    assert_eq!(game.outcome(), Outcome::Lost);
}

#[test]
fn in_another_mission_a_mushroom_is_taken_and_counts_for_nothing() {
    let mut game = game_on(&map_with(&mushrooms(&[(20, 11)])), Mission::Destroy, 1, PlayerCount::ONE);
    put_seat(&mut game, 0, crate::map::cell_to_world(20, 11));
    step(&mut game);
    assert_eq!(taken(&game), vec![(0, 20, 11, 0)]);
    assert_eq!(game.outcome(), Outcome::Playing, "a destroy round is not won by mushrooms");
}

#[test]
fn two_seats_on_one_mushroom_give_it_to_the_lower_seat() {
    let mut game = game_on(&map_with(&mushrooms(&[(20, 11)])), Mission::Forage, 0, PlayerCount::TWO);
    put_seat(&mut game, 1, crate::map::cell_to_world(20, 10));
    put_seat(&mut game, 0, crate::map::cell_to_world(20, 12));
    step(&mut game);
    assert_eq!(taken(&game), vec![(0, 20, 11, 0)]);
}

#[test]
fn an_enemy_never_takes_a_mushroom() {
    let mut game = game_on(&map_with(&mushrooms(&[(20, 11)])), Mission::Forage, 1, PlayerCount::ONE);
    let enemy = game.world.query::<(Entity, &Tank)>().with::<&crate::ai::Ai>().iter().map(|(e, _)| e).next().expect("enemy");
    let body = with_tank(&game.world, enemy, |t| t.body).expect("body");
    let at = crate::map::cell_to_world(20, 11);
    game.physics.set_position(body, at);
    with_tank_mut(&game.world, enemy, |t| t.position = at);
    step(&mut game);
    assert!(taken(&game).is_empty());
    assert_eq!(game.mushrooms_left(), 1);
}

#[test]
fn the_taken_mask_round_trips_and_a_restart_puts_them_back() {
    let mut game = game_on(&map_with(&mushrooms(&[(20, 11), (25, 11), (30, 11)])), Mission::Forage, 0, PlayerCount::ONE);
    put_seat(&mut game, 0, crate::map::cell_to_world(25, 11));
    step(&mut game);
    assert_eq!(game.mushrooms_taken_mask(), 0b010, "map order: (20,11), (25,11), (30,11)");
    let mut other = game_on(&map_with(&mushrooms(&[(20, 11), (25, 11), (30, 11)])), Mission::Forage, 0, PlayerCount::ONE);
    other.set_mushrooms_taken(0b010);
    assert_eq!(other.mushrooms_left(), 2);
    assert!(other.mushrooms()[1].taken_at.is_some(), "a mushroom the mask takes pops");
    game.init(W, H);
    assert_eq!(game.mushrooms_left(), 3);
}
