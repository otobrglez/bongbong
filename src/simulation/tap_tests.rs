//! Headless scenario tests for tap-to-command (docs/tap-navigation.md).
//!
//! Same shape as `props_tests`: seeded rounds on tiny inline maps, asserting
//! on `Game::events` and the public snapshot rather than on trajectories.
//! What is worth locking in here is the *order lifecycle* - taps resolve to
//! the right thing, orders end when they should, and nothing spins forever -
//! because that is what a player on a touchscreen is entirely at the mercy
//! of.

use super::*;
use crate::ai::Intent;
use crate::map::cell_to_world;
use crate::obstacle::Material;
use crate::tank::Dir;

const W: f32 = 1280.0;
const H: f32 = 720.0;

/// One enemy, boxed in iron in the far corner where it can neither see nor
/// reach anything - the same trick `props_tests` uses.
fn map_with(extra: &str) -> String {
    let mut text = String::from(
        r#"
version = 1
tanks = 1
cells."2,2" = { kind = "frog" }
cells."20,18" = { kind = "start" }
"#,
    );
    for (c, r) in [(36, 19), (37, 19), (38, 19), (36, 20), (38, 20), (36, 21), (37, 21), (38, 21)] {
        text.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
    }
    text.push_str(extra);
    text
}

fn game_on(map: &str, seed: u64) -> Game {
    let mut game = Game::default();
    game.enemy_count_override = Some(1);
    game.seed_override = Some(seed);
    game.player_row_override = Some(0);
    game.map = MapFile::from_toml_str(map).expect("test map parses");
    game.init(W, H);
    game.debug_teleport(1, cell_to_world(37, 20), Some(0.0)).expect("enemy in slot 1");
    game
}

fn step(game: &mut Game, input: Input) {
    game.update(input, 1.0 / 60.0, W, H);
}

fn tap(at: Position) -> Input {
    Input { tap: Some(at), ..Input::default() }
}

fn player_at(game: &Game) -> Position {
    game.tank_snapshots().iter().find(|t| t.is_player).expect("player snapshot").position
}

/// Run until the order clears or `frames` run out; returns the frame the
/// order ended on.
fn run_until_idle(game: &mut Game, frames: usize) -> Option<usize> {
    (0..frames).find(|_| {
        step(game, Input::default());
        !game.orders.is_active()
    })
}

#[test]
fn a_tap_on_open_ground_drives_there_and_stops() {
    let mut game = game_on(&map_with(""), 1);
    let start = cell_to_world(10, 10);
    let goal = cell_to_world(18, 10);
    game.debug_teleport(0, start, Some(0.0)).unwrap();
    step(&mut game, tap(goal));
    assert!(game.orders.is_active(), "a tap on open ground starts a move order");

    let ended = run_until_idle(&mut game, 900).expect("the order finishes");
    let at = player_at(&game);
    assert!(
        at.distance_to(goal) <= tuning().order_arrive_px * 2.0,
        "stopped at the tap (frame {ended}, {:.0}px away)",
        at.distance_to(goal)
    );
}

/// The pathfinding claim: a wall between the player and the tap is routed
/// *around*, not driven into. Measured as arrival plus a route that is
/// visibly longer than the straight line - a tank that ground along the wall
/// and gave up would fail the first half.
#[test]
fn a_tap_past_a_wall_paths_around_it() {
    let mut wall = String::new();
    for r in 6..=14 {
        wall.push_str(&format!("cells.\"14,{r}\" = {{ kind = \"wall\", material = \"brick\" }}\n"));
    }
    let mut game = game_on(&map_with(&wall), 2);
    let start = cell_to_world(10, 10);
    let goal = cell_to_world(18, 10);
    game.debug_teleport(0, start, Some(0.0)).unwrap();
    step(&mut game, tap(goal));

    let mut travelled = 0.0;
    let mut prev = player_at(&game);
    let mut arrived = false;
    for _ in 0..1800 {
        step(&mut game, Input::default());
        let now = player_at(&game);
        travelled += prev.distance_to(now);
        prev = now;
        if !game.orders.is_active() {
            arrived = true;
            break;
        }
    }
    assert!(arrived, "the order finished");
    assert!(
        prev.distance_to(goal) <= tuning().order_arrive_px * 2.0,
        "arrived at the tap, {:.0}px away",
        prev.distance_to(goal)
    );
    let straight = start.distance_to(goal);
    assert!(travelled > straight * 1.2, "went around the wall, not through it ({travelled:.0}px vs {straight:.0}px straight)");
}

/// Target locking, which is the half of this the PRD actually names: a tap
/// on an enemy has to close on it and put shells into it.
///
/// Not "and kills it": the assist drives whatever chassis the player has
/// into a fight the enemy is also fighting, and a row-0 scout with 20 shells
/// duelling a live AI tank frequently loses - measured, the round ended and
/// restarted before the enemy did. That is the player's choice to make when
/// they tap; what this test owns is that the order locks on and engages.
#[test]
fn a_tap_on_an_enemy_locks_on_and_puts_shells_into_it() {
    let mut game = game_on(&map_with(""), 3);
    game.debug_teleport(0, cell_to_world(20, 14), Some(0.0)).unwrap();
    let enemy = cell_to_world(20, 7);
    game.debug_teleport(1, enemy, Some(180.0)).unwrap();
    step(&mut game, tap(enemy));
    assert!(game.orders.is_active(), "a tap on a live enemy starts an engagement");
    assert_eq!(
        game.player_order().map(|o| o.kind),
        Some("engage"),
        "and it is an engagement, not a move to where it was standing"
    );

    let (mut fired, mut landed) = (0, 0);
    for _ in 0..900 {
        step(&mut game, Input::default());
        for e in game.events() {
            match e {
                Event::Fired { .. } => fired += 1,
                Event::Hit { target: HitTarget::Enemy { .. }, .. } => landed += 1,
                _ => {}
            }
        }
    }
    assert!(fired >= 5, "the order actually shoots rather than just driving about ({fired} shots)");
    assert!(landed >= 1, "and lands at least one of them ({landed} of {fired})");
}
#[test]
fn a_tap_on_a_barrel_pops_it() {
    let mut game = game_on(&map_with("cells.\"20,8\" = { kind = \"barrel\" }\n"), 4);
    game.debug_teleport(0, cell_to_world(20, 14), Some(0.0)).unwrap();
    let barrel = cell_to_world(20, 8);
    step(&mut game, tap(barrel));

    let popped = (0..1800).any(|_| {
        step(&mut game, Input::default());
        game.events()
            .iter()
            .any(|e| matches!(e, Event::ObstacleDestroyed { material: Material::Barrel, .. }))
    });
    assert!(popped, "tapping a barrel is an order to shoot it");
}

/// Iron never dies, so tapping it is a move order and the tank parks beside
/// it rather than grinding away at it forever with finite ammo.
#[test]
fn a_tap_on_iron_is_a_move_not_an_attack() {
    let mut game = game_on(&map_with("cells.\"20,8\" = { kind = \"wall\", material = \"iron\" }\n"), 5);
    game.debug_teleport(0, cell_to_world(20, 14), Some(0.0)).unwrap();
    step(&mut game, tap(cell_to_world(20, 8)));

    let shells_before = game.tank_snapshots().iter().find(|t| t.is_player).unwrap().shells_ammo;
    run_until_idle(&mut game, 1800);
    let shells_after = game.tank_snapshots().iter().find(|t| t.is_player).unwrap().shells_ammo;
    assert_eq!(shells_before, shells_after, "no shells spent on something that cannot be destroyed");
}

#[test]
fn an_arrow_key_cancels_the_standing_order() {
    let mut game = game_on(&map_with(""), 6);
    game.debug_teleport(0, cell_to_world(10, 10), Some(0.0)).unwrap();
    step(&mut game, tap(cell_to_world(18, 10)));
    assert!(game.orders.is_active());

    let drive = Input {
        player_intent: Intent { move_dir: Some(Dir::Down), ..Intent::default() },
        ..Input::default()
    };
    step(&mut game, drive);
    assert!(!game.orders.is_active(), "the keyboard always wins, on the frame it is pressed");
}

/// A tap somewhere no route exists has to *end*, not leave the tank shuffling
/// against a wall for the rest of the round.
#[test]
fn an_unreachable_tap_gives_up_instead_of_spinning() {
    // A sealed iron box with nothing but floor inside it.
    let mut box_map = String::new();
    for c in 8..=12 {
        for r in 4..=8 {
            let edge = c == 8 || c == 12 || r == 4 || r == 8;
            if edge {
                box_map.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
            }
        }
    }
    let mut game = game_on(&map_with(&box_map), 7);
    game.debug_teleport(0, cell_to_world(20, 14), Some(0.0)).unwrap();
    step(&mut game, tap(cell_to_world(10, 6)));

    let ended = run_until_idle(&mut game, 1800);
    assert!(ended.is_some(), "an order with no route clears itself");
}

/// The order ring is the player's only feedback that a tap registered, so
/// "there is an order" and "the ring is showing" must not drift apart.
#[test]
fn the_order_ring_tracks_whether_an_order_is_running() {
    let mut game = game_on(&map_with(""), 8);
    game.debug_teleport(0, cell_to_world(10, 10), Some(0.0)).unwrap();
    assert_eq!(game.orders.ring_engage(), None, "no order, no ring");

    step(&mut game, tap(cell_to_world(18, 10)));
    assert_eq!(game.orders.ring_engage(), Some(false), "a move order shows the cool ring");

    let enemy = cell_to_world(20, 7);
    game.debug_teleport(1, enemy, Some(180.0)).unwrap();
    step(&mut game, tap(enemy));
    assert_eq!(game.orders.ring_engage(), Some(true), "an engagement shows the hot ring");
}
