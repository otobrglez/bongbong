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

/// **Most taps have to do something.** This is the regression test for the
/// bug that shipped in the first cut: `resolve_tap` snapped a move to the
/// nearest *usable* cell rather than the nearest *reachable* one, so on the
/// shipped map 81% of taps on open ground produced no route and the tank
/// simply stood there. A player on a touchscreen has no other way to move.
///
/// Measured statically - resolve every cell of `maps/default.toml` and ask
/// whether the order is actionable - because stepping the game between taps
/// lets enemies move and pickups get collected, which makes the number
/// drift by twenty points depending on iteration order.
#[test]
fn almost_every_tap_on_the_shipped_map_produces_an_actionable_order() {
    let mut game = Game::default();
    game.enemy_count_override = Some(4);
    game.seed_override = Some(99);
    let text = std::fs::read_to_string("maps/default.toml").expect("the shipped map");
    let map = MapFile::from_toml_str(&text).expect("it parses");
    let solid: std::collections::HashSet<(i32, i32)> =
        map.iter_cells().filter(|(_, _, o)| o.is_solid()).map(|(c, r, _)| (c, r)).collect();
    game.map = map;
    game.init(W, H);
    let me = player_at(&game);
    let grid = game.nav_grid(W, H);
    let comps = grid.components();
    let arrive = tuning().order_arrive_px;

    let mut counts = [[0usize; 2]; 2]; // [is_solid][actionable]
    for gx in 1..40 {
        for gy in 1..22 {
            let at = cell_to_world(gx, gy);
            let actionable = match game.resolve_tap(at, &grid, &comps) {
                None => false,
                // A move is actionable unless we are already standing there.
                Some(Order::Move { to }) => me.distance_to(to) > arrive,
                // Engage and Collect both approach best-effort, so they are
                // actionable whenever they resolve at all.
                Some(_) => true,
            };
            counts[solid.contains(&(gx, gy)) as usize][actionable as usize] += 1;
        }
    }
    let pct = |c: [usize; 2]| 100.0 * c[1] as f32 / (c[0] + c[1]).max(1) as f32;
    let (open, solid_pct) = (pct(counts[0]), pct(counts[1]));
    println!("open {open:.0}% actionable, solid {solid_pct:.0}%");
    assert!(open >= 90.0, "only {open:.0}% of taps on open ground do anything (was 19% before the fix)");
    assert!(solid_pct >= 70.0, "only {solid_pct:.0}% of taps on solid tiles do anything (was 31% before the fix)");
}

/// "Move in that direction", which is what a tap across an impassable
/// barrier has to mean. Requiring a route to the exact point made such a
/// tap do nothing at all; it now heads for the closest reachable point on
/// this side, which is the same gesture from the player's end.
#[test]
fn a_tap_behind_an_impassable_wall_still_moves_you_toward_it() {
    let mut wall = String::new();
    for r in 1..22 {
        wall.push_str(&format!("cells.\"24,{r}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
    }
    let mut game = game_on(&map_with(&wall), 12);
    let start = cell_to_world(10, 11);
    let beyond = cell_to_world(34, 11); // the far side of an unbroken iron wall
    game.debug_teleport(0, start, Some(0.0)).unwrap();
    step(&mut game, tap(beyond));
    assert!(game.orders.is_active(), "the tap is not simply discarded");

    for _ in 0..900 {
        step(&mut game, Input::default());
        if !game.orders.is_active() {
            break;
        }
    }
    let ended = player_at(&game);
    assert!(
        ended.x > start.x + 100.0,
        "drove toward the tap rather than standing still (x {:.0} -> {:.0})",
        start.x,
        ended.x
    );
}

#[test]
fn a_tap_on_a_pickup_goes_and_collects_it() {
    let mut game = game_on(&map_with("cells.\"14,11\" = { kind = \"pickup\", pickup = \"ammo\" }\n"), 13);
    game.debug_teleport(0, cell_to_world(8, 11), Some(0.0)).unwrap();
    step(&mut game, tap(cell_to_world(14, 11)));
    assert_eq!(game.player_order().map(|o| o.kind), Some("collect"), "a tap on a pickup is its own kind of order");

    let collected = (0..900).any(|_| {
        step(&mut game, Input::default());
        game.events()
            .iter()
            .any(|e| matches!(e, Event::PickupCollected { slot: 0, .. }))
    });
    assert!(collected, "the order drives onto the pickup and takes it");
}

/// The fire gate is a *hit* test, not a heading tolerance - see
/// `orders::hit_window`. This is the measurement that says so: park an
/// enemy at a spread of bearings, tap it, and count how many of the shells
/// that go out actually land.
///
/// The number this replaced was 24 px of flat `enemy_fire_align_px` for
/// every chassis against every target, which put half a twin-barrel hull's
/// shells past the target: measured at 51% for an `assault` and 50% for a
/// `titan`, against 79% for the single-barrel `scout` that has no barrel
/// offset to spend. Sizing the window to the target and charging the
/// shooter's barrel offset against it took the two twin-barrel rows to 66%
/// and 69% while spending barely half the ammunition per kill.
///
/// The frog is walled into a corner here so the round cannot end out from
/// under the measurement - a hunter that shoots it wins the round, and the
/// restart that follows resets the very orders under test.
#[test]
fn a_tapped_enemy_is_shot_at_from_a_line_that_can_hit_it() {
    let mut protected = String::new();
    for (c, r) in [(1, 1), (2, 1), (3, 1), (1, 2), (3, 2), (1, 3), (2, 3), (3, 3)] {
        protected.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
    }
    let map = map_with(&protected);

    // One single-barrel chassis and the two extremes of the twin-barrel
    // ones, since the barrel offset is exactly what the old flat window
    // could not account for.
    let mut rates = Vec::new();
    for row in [0i32, 1, 10] {
        let (mut fired, mut landed) = (0, 0);
        for (dc, dr) in [(0i32, -7i32), (1, -7), (2, -6), (3, -5), (5, -4), (6, -2), (7, 0),
                         (6, 2), (5, 4), (3, 5), (2, 6), (1, 7), (0, 7), (-3, -5), (-6, -2), (-5, 4)] {
            let mut game = Game::default();
            game.enemy_count_override = Some(1);
            game.seed_override = Some(3);
            game.player_row_override = Some(row);
            game.map = MapFile::from_toml_str(&map).expect("test map parses");
            game.init(W, H);
            game.debug_teleport(0, cell_to_world(20, 14), Some(0.0)).unwrap();
            let enemy = cell_to_world(20 + dc, 14 + dr);
            game.debug_teleport(1, enemy, Some(180.0)).unwrap();
            step(&mut game, tap(enemy));
            if game.player_order().map(|o| o.kind) != Some("engage") {
                continue;
            }
            let (mut shots, mut hits) = (0, 0);
            for _ in 0..900 {
                step(&mut game, Input::default());
                let mut over = false;
                for e in game.events() {
                    match e {
                        Event::Fired { slot: 0, .. } => shots += 1,
                        Event::Hit { target: HitTarget::Enemy { .. }, .. } => hits += 1,
                        Event::RoundEnded { .. } => over = true,
                        _ => {}
                    }
                }
                if over {
                    break;
                }
            }
            // A twin-barrel chassis puts two shells downrange per shot.
            let barrels = if tuning().tank_barrel_lateral_offset[row as usize] > 0.0 { 2 } else { 1 };
            fired += shots * barrels;
            landed += hits;
        }
        assert!(fired >= 40, "chassis row {row} took {fired} shots - too few to measure, and a gate the tank cannot satisfy is its own bug");
        let rate = 100.0 * landed as f32 / fired as f32;
        println!("MEASURED row {row}: {landed}/{fired} = {rate:.0}%");
        rates.push((row, landed, fired, rate));
    }
    for (row, landed, fired, rate) in rates {
        assert!(
            rate >= 55.0,
            "chassis row {row} landed {landed} of {fired} shells ({rate:.0}%); \
             the flat-window version managed 47-50% on the twin-barrel rows \
             and that is the bug this guards"
        );
    }
}

/// The corridor complaint, as a number: tap every reachable cell on a map
/// from the player's start and count how many move orders actually get
/// there.
///
/// It is worth knowing how this measurement was got wrong first, because
/// the wrong version sent two rounds of work at the wrong bug. Counting
/// *every* order rather than only the moves read 72% on the shipped map,
/// with misses up to 280 px - but an engagement drives to a firing spot,
/// not to the tap, so most of those "misses" were tanks correctly standing
/// somewhere else. Restricted to the orders that really are meant to arrive
/// somewhere, the same sweep reads 99-100%, and the real defect the 280 px
/// came from turned out to be an engagement grinding into a wall (see
/// `orders::commit_dir`).
#[test]
fn move_orders_arrive_even_through_corridors() {
    for name in ["maps/test/maze.toml", "maps/test/tight-corridors.toml", "maps/default.toml"] {
        let mut game = Game::default();
        game.enemy_count_override = Some(0);
        game.seed_override = Some(5);
        game.map = MapFile::from_toml_str(&std::fs::read_to_string(name).expect("fixture")).expect("parses");
        game.init(W, H);
        let start = player_at(&game);
        let grid = game.nav_grid(W, H);
        let comps = grid.components();
        let (mut tried, mut arrived) = (0, 0);
        for gx in (2..38).step_by(3) {
            for gy in (2..21).step_by(3) {
                let goal = cell_to_world(gx, gy);
                let Some(reach) = grid.nearest_reachable(goal, start, &comps) else { continue };
                // Short hops say nothing about routing.
                if start.distance_to(reach) < 120.0 {
                    continue;
                }
                game.debug_teleport(0, start, Some(0.0)).unwrap();
                game.orders.clear();
                step(&mut game, tap(goal));
                if game.player_order().map(|o| o.kind) != Some("move") {
                    continue;
                }
                tried += 1;
                for _ in 0..1200 {
                    step(&mut game, Input::default());
                    if !game.orders.is_active() {
                        break;
                    }
                }
                // Within a tank's own length of the goal counts as arrived.
                if player_at(&game).distance_to(reach) <= 48.0 {
                    arrived += 1;
                }
            }
        }
        assert!(tried >= 20, "{name}: only {tried} move orders sampled");
        let rate = 100.0 * arrived as f32 / tried as f32;
        assert!(rate >= 95.0, "{name}: only {arrived} of {tried} move orders arrived ({rate:.0}%)");
    }
}
