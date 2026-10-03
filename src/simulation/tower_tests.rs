//! Headless scenario tests for the defence towers (docs/defence-towers-prd.md
//! section 16): one per promised rule, on a 40 x 22.5 field with the one
//! enemy every round needs boxed in iron in the far corner, so the player
//! is the only tank an enemy tower ever sees unless a test brings the enemy
//! out. Tests cannot touch the global tuning table, so every number is read
//! off the defaults.

use rand::RngExt;

use super::debug::TankPatch;
use super::*;
use crate::ai::Intent;
use crate::map::cell_to_world;
use crate::tower::{OozePuddle, Tower, TowerKind};

const W: f32 = 1280.0;
const H: f32 = 720.0;
const DT: f32 = 1.0 / 60.0;

/// The iron box the enemy waits in.
const ENEMY_CELL: (i32, i32) = (37, 20);

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

/// A round on `map` with `players` seats, the enemy in its box, and every
/// seat's spawn shield off.
fn game_on(map: &str, players: PlayerCount, seed: u64) -> Game {
    let mut game = Game::default();
    game.enemy_count_override = Some(1);
    game.seed_override = Some(seed);
    game.player_row_override = Some(0);
    game.player2_row_override = Some(0);
    game.players = players;
    game.map = MapFile::from_toml_str(map).expect("test map parses");
    game.init(W, H);
    for entity in game.players().into_iter().flatten() {
        let mut q = game.world.query_one::<&mut Tank>(entity);
        q.get().expect("player tank").shield_hp = 0.0;
    }
    let enemy = game.first_enemy_slot();
    place(&mut game, enemy, ENEMY_CELL);
    game
}

fn place(game: &mut Game, slot: usize, cell: (i32, i32)) {
    game.debug_teleport(slot, cell_to_world(cell.0, cell.1), Some(0.0)).expect("tank in slot");
}

fn step(game: &mut Game, input: Input) {
    game.update(input, DT, W, H);
}

fn idle(game: &mut Game, frames: usize) -> Vec<Event> {
    let mut events = Vec::new();
    for _ in 0..frames {
        step(game, Input::default());
        events.extend(game.events().iter().cloned());
    }
    events
}

fn snapshot(game: &Game, slot: usize) -> TankSnapshot {
    game.tank_snapshots().into_iter().find(|t| t.slot == slot).expect("tank in slot")
}

fn tower(game: &Game, cell: (i32, i32)) -> Tower {
    game.towers.get(&cell).cloned().expect("a tower stands at the cell")
}

fn player_tank(game: &Game, seat: usize) -> Entity {
    game.players()[seat].expect("seat has a tank")
}

fn slime(game: &Game, seat: usize) -> f32 {
    with_tank(&game.world, player_tank(game, seat), |t| t.slime_timer)
}

fn set_slime(game: &mut Game, seat: usize, seconds: f32) {
    with_tank_mut(&game.world, player_tank(game, seat), |t| t.slime_timer = seconds);
}

/// Set the tower tile at `cell` to `frac` of its health.
fn wound(game: &mut Game, cell: (i32, i32), frac: f32) {
    for o in game.world.query_mut::<&mut Obstacle>() {
        if o.cell() == cell {
            o.health = o.max_health * frac;
        }
    }
}

fn tile_health(game: &Game, cell: (i32, i32)) -> Option<(f32, f32)> {
    game.world.query::<&Obstacle>().iter().find(|o| o.cell() == cell && !o.destroyed).map(|o| (o.health, o.max_health))
}

fn strikes(events: &[Event]) -> Vec<(bool, Position)> {
    events
        .iter()
        .filter_map(|e| match *e {
            Event::TeslaStrike { x1, y1, chained, .. } => Some((chained, Position::new(x1, y1))),
            _ => None,
        })
        .collect()
}

fn tower_fired(events: &[Event], kind: TowerKind) -> usize {
    events.iter().filter(|e| matches!(e, Event::TowerFired { kind: k, .. } if *k == kind.name())).count()
}

fn enemy_tesla(cell: (i32, i32)) -> String {
    format!("cells.\"{},{}\" = {{ kind = \"tesla\", side = \"enemy\" }}\n", cell.0, cell.1)
}

// 1. A tesla charges only while an opposing tank is in reach and strikes it
//    at full charge; a tank that backs out before full charge takes nothing.
#[test]
fn a_tesla_strikes_a_tank_in_reach_at_full_charge() {
    let mut game = game_on(&map_with(&enemy_tesla((20, 12))), PlayerCount::ONE, 1);
    place(&mut game, 0, (20, 15));
    let charge_frames = (tuning().tesla_charge_seconds / DT).ceil() as usize;
    let mut struck = None;
    for frame in 1..=charge_frames + 10 {
        step(&mut game, Input::default());
        if !strikes(game.events()).is_empty() {
            struck = Some(frame);
            break;
        }
    }
    let frame = struck.expect("a tank in reach is struck");
    assert!(frame.abs_diff(charge_frames) <= 2, "struck at frame {frame}, full charge is {charge_frames}");
    let damage = snapshot(&game, 0).damage;
    assert!(damage >= tuning().tesla_damage_min - 1e-3 && damage <= tuning().tesla_damage_max + 1e-3, "{damage}");
    assert_eq!(tower(&game, (20, 12)).charge, 0.0, "a strike spends the charge");
}

#[test]
fn a_tank_that_backs_out_before_full_charge_takes_nothing() {
    let mut game = game_on(&map_with(&enemy_tesla((20, 12))), PlayerCount::ONE, 1);
    place(&mut game, 0, (20, 15));
    let half = (tuning().tesla_charge_seconds * 0.5 / DT) as usize;
    idle(&mut game, half);
    assert!(tower(&game, (20, 12)).charge > 0.3, "charging while the tank is in reach");
    place(&mut game, 0, (20, 20));
    let events = idle(&mut game, 120);
    assert!(strikes(&events).is_empty(), "no strike once the tank is out of reach");
    assert_eq!(snapshot(&game, 0).damage, 0.0);
    assert_eq!(tower(&game, (20, 12)).charge, 0.0, "the charge drains away");
}

// 2. The strike chains once to a second tank within the chain radius, for
//    half the rolled damage, and not to a tank behind a wall.
#[test]
fn a_strike_chains_to_a_second_tank_for_half_the_damage() {
    let mut game = game_on(&map_with(&enemy_tesla((20, 12))), PlayerCount::TWO, 2);
    place(&mut game, 0, (20, 15));
    place(&mut game, 1, (21, 16));
    let events = idle(&mut game, (tuning().tesla_charge_seconds / DT) as usize + 5);
    let hits = strikes(&events);
    assert_eq!(hits.len(), 2, "one strike and one jump: {hits:?}");
    assert!(!hits[0].0 && hits[1].0, "the second is the chained jump");
    let t = tuning();
    let (first, second) = (snapshot(&game, 0).damage, snapshot(&game, 1).damage);
    // Both are seats, so their armour takes its share of each.
    let a = t.player_armor_factor;
    assert!(first >= t.tesla_damage_min * a - 1e-3 && first <= t.tesla_damage_max * a + 1e-3, "{first}");
    let f = t.tesla_chain_factor * a;
    assert!(second >= t.tesla_damage_min * f - 1e-3 && second <= t.tesla_damage_max * f + 1e-3, "{second}");
}

#[test]
fn a_strike_does_not_chain_through_a_wall() {
    let map = map_with(&(enemy_tesla((20, 12)) + "cells.\"21,15\" = { kind = \"wall\", material = \"iron\" }\n"));
    let mut game = game_on(&map, PlayerCount::TWO, 2);
    place(&mut game, 0, (20, 15));
    place(&mut game, 1, (22, 15));
    let events = idle(&mut game, (tuning().tesla_charge_seconds / DT) as usize + 5);
    let hits = strikes(&events);
    assert_eq!(hits.len(), 1, "the wall stops the jump: {hits:?}");
    assert_eq!(snapshot(&game, 1).damage, 0.0);
}

// 3. A tesla never charges at its own side, a frog, or a tank behind a wall.
#[test]
fn a_tesla_ignores_its_own_side_frogs_and_tanks_behind_walls() {
    let map = map_with(
        "cells.\"20,12\" = { kind = \"tesla\" }\n\
         cells.\"4,2\" = { kind = \"tesla\", side = \"enemy\" }\n",
    );
    let mut game = game_on(&map, PlayerCount::ONE, 3);
    place(&mut game, 0, (20, 15));
    let events = idle(&mut game, 150);
    assert!(strikes(&events).is_empty());
    assert_eq!(tower(&game, (20, 12)).charge, 0.0, "its own side's tank");
    assert_eq!(tower(&game, (4, 2)).charge, 0.0, "the frog two cells away");

    let map = map_with(&(enemy_tesla((20, 12)) + "cells.\"20,13\" = { kind = \"wall\", material = \"iron\" }\n"));
    let mut game = game_on(&map, PlayerCount::ONE, 3);
    place(&mut game, 0, (20, 15));
    let events = idle(&mut game, 150);
    assert!(strikes(&events).is_empty());
    assert_eq!(tower(&game, (20, 12)).charge, 0.0, "a tank behind iron");
}

// 4. A gun tower turns toward a target at its turn rate and fires only
//    inside its fire cone; it holds fire while a friend is in the line, and
//    its bullets pass through its own side's tanks.
#[test]
fn a_gun_tower_turns_at_its_rate_and_fires_inside_its_cone() {
    let map = map_with("cells.\"20,8\" = { kind = \"gun_tower\", side = \"enemy\" }\n");
    let mut game = game_on(&map, PlayerCount::ONE, 4);
    place(&mut game, 0, (26, 8));
    let t = tuning();
    let per_frame = t.gun_tower_turn_deg_per_second * DT;
    let off = |h: f32| ((90.0 - h + 540.0f32).rem_euclid(360.0) - 180.0).abs();
    let mut heading = tower(&game, (20, 8)).heading;
    assert!(off(heading) > 45.0, "starts aimed at the middle of the field, well off the tank");
    let mut fired_at = None;
    for frame in 1..=120 {
        step(&mut game, Input::default());
        let now = tower(&game, (20, 8)).heading;
        let turned = ((now - heading + 540.0f32).rem_euclid(360.0) - 180.0).abs();
        assert!(turned <= per_frame + 1e-3, "frame {frame}: turned {turned} > {per_frame}");
        heading = now;
        if tower_fired(game.events(), TowerKind::Gun) > 0 {
            assert!(off(now) <= t.gun_tower_fire_cone_deg + 1e-3, "fired {} off the aim", off(now));
            fired_at = Some(frame);
            break;
        }
    }
    assert!(fired_at.is_some(), "fires once it is on the aim");
}

// A gun tower fires from above the battlefield: its bullets fly over the
// sandbags and fences its aim sees past, and still stop at a wall.
#[test]
fn a_gun_tower_shoots_over_sandbags_and_fences() {
    let map = map_with(
        "cells.\"20,8\" = { kind = \"gun_tower\", side = \"enemy\" }\n\
         cells.\"22,8\" = { kind = \"sandbag\" }\n\
         cells.\"24,8\" = { kind = \"fence\" }\n",
    );
    let mut game = game_on(&map, PlayerCount::ONE, 4);
    place(&mut game, 0, (26, 8));
    let (bag, fence) = (tile_health(&game, (22, 8)), tile_health(&game, (24, 8)));
    assert!(bag.is_some() && fence.is_some(), "the cover stands");
    let before = with_tank(&game.world, player_tank(&game, 0), |t| t.health_fraction());
    let events = idle(&mut game, 240);
    assert!(tower_fired(&events, TowerKind::Gun) > 0, "the tower opened fire");
    let after = with_tank(&game.world, player_tank(&game, 0), |t| t.health_fraction());
    assert!(after < before, "the bullets reached the tank: {before} -> {after}");
    assert_eq!((tile_health(&game, (22, 8)), tile_health(&game, (24, 8))), (bag, fence), "the cover is untouched");
}

// An enemy tower fights a seat only from inside the seat's sight box
// (docs/large-maps-follow-camera.md section 5). The gun tower is the one
// whose reach goes past the box: straight up or down its range is longer
// than the box's half height.
#[test]
fn an_enemy_gun_tower_fires_at_a_seat_only_from_inside_its_sight_box() {
    let t = tuning();
    let (_, half_h) = t.sight_box_half_px();
    assert!(t.gun_tower_range > half_h + 8.0, "the defaults this is about: the gun reaches past the box");
    let map = map_with("cells.\"20,4\" = { kind = \"gun_tower\", side = \"enemy\" }\n");
    let tower_at = cell_to_world(20, 4);
    let bursts_with_the_seat_below = |dy: f32| {
        let mut game = game_on(&map, PlayerCount::ONE, 4);
        game.debug_teleport(0, Position::new(tower_at.x, tower_at.y + dy), Some(0.0)).expect("the seat");
        let events = idle(&mut game, 90);
        tower_fired(&events, TowerKind::Gun)
    };
    assert_eq!(bursts_with_the_seat_below(half_h + 8.0), 0, "in reach, but outside the seat's box");
    assert!(bursts_with_the_seat_below(half_h - 8.0) > 0, "inside the box it opens fire");
}

#[test]
fn a_gun_tower_holds_fire_on_a_friend_and_shoots_through_one() {
    let map = map_with("cells.\"20,8\" = { kind = \"gun_tower\", side = \"enemy\" }\n");
    let mut game = game_on(&map, PlayerCount::ONE, 5);
    let enemy = game.first_enemy_slot();
    place(&mut game, 0, (20, 14));
    let mut events = Vec::new();
    for _ in 0..90 {
        place(&mut game, enemy, (20, 11));
        step(&mut game, Input::default());
        events.extend(game.events().iter().cloned());
    }
    assert_eq!(tower_fired(&events, TowerKind::Gun), 0, "a friend in the line holds its fire");

    // Out of the line it opens up; a friend that steps back in mid-burst
    // is flown through.
    place(&mut game, enemy, ENEMY_CELL);
    let mut fired = false;
    for _ in 0..90 {
        step(&mut game, Input::default());
        if tower_fired(game.events(), TowerKind::Gun) > 0 {
            fired = true;
            break;
        }
    }
    assert!(fired, "fires once the line is clear");
    let friend = cell_to_world(20, 11);
    let mut past = false;
    for _ in 0..30 {
        place(&mut game, enemy, (20, 11));
        step(&mut game, Input::default());
        past |= game
            .world
            .query::<&crate::bullet::Bullet>()
            .iter()
            .any(|b| b.owner.is_tower() && b.position.y > friend.y + OBSTACLE_GRID_SIZE);
    }
    assert!(past, "a tower bullet flew on past its own side's tank");
    assert_eq!(snapshot(&game, enemy).damage, 0.0, "and never hurt it");
}

// 5. A gun tower and a bio tower do not see a tank concealed in tall grass;
//    a tesla does.
#[test]
fn tall_grass_hides_a_tank_from_the_guns_but_not_the_tesla() {
    let map = map_with(
        "cells.\"20,15\" = { kind = \"tall_grass\" }\n\
         cells.\"20,10\" = { kind = \"gun_tower\", side = \"enemy\" }\n\
         cells.\"24,15\" = { kind = \"bio_slush\", side = \"enemy\" }\n\
         cells.\"17,15\" = { kind = \"tesla\", side = \"enemy\" }\n",
    );
    let mut game = game_on(&map, PlayerCount::ONE, 6);
    place(&mut game, 0, (20, 15));
    let events = idle(&mut game, 200);
    assert_eq!(tower_fired(&events, TowerKind::Gun), 0, "the gun tower never sees it");
    assert_eq!(tower_fired(&events, TowerKind::Bio), 0, "nor the mortar");
    assert!(!strikes(&events).is_empty(), "the tesla needs no sight of it");
}

// 6. A bio tower lobs at a tank between its minimum and maximum range and
//    never at one inside the minimum; the glob lands at the aim point and
//    flies over a tile between them.
#[test]
fn a_mortar_never_lobs_inside_its_minimum_range() {
    let map = map_with("cells.\"20,14\" = { kind = \"bio_slush\", side = \"enemy\" }\n");
    let mut game = game_on(&map, PlayerCount::ONE, 7);
    place(&mut game, 0, (21, 15));
    assert!(cell_to_world(21, 15).distance_to(cell_to_world(20, 14)) < tuning().bio_min_range);
    let events = idle(&mut game, 240);
    assert_eq!(tower_fired(&events, TowerKind::Bio), 0);
}

#[test]
fn a_glob_flies_over_a_sandbag_and_lands_on_the_tank() {
    let map = map_with(
        "cells.\"20,12\" = { kind = \"bio_slush\", side = \"enemy\" }\n\
         cells.\"20,14\" = { kind = \"sandbag\" }\n",
    );
    let mut game = game_on(&map, PlayerCount::ONE, 8);
    place(&mut game, 0, (20, 17));
    let at = cell_to_world(20, 17);
    let bag = tile_health(&game, (20, 14)).expect("sandbag");
    let mut splash = None;
    for _ in 0..300 {
        step(&mut game, Input::default());
        if let Some(Event::GlobSplashed { x, y }) = game.events().iter().find(|e| matches!(e, Event::GlobSplashed { .. })) {
            splash = Some(Position::new(*x, *y));
            break;
        }
    }
    let splash = splash.expect("the mortar lobbed and the glob came down");
    assert!(splash.distance_to(at) <= tuning().bio_scatter_px + 1.0, "landed {} px off", splash.distance_to(at));
    assert_eq!(tile_health(&game, (20, 14)), Some(bag), "the sandbag in between is untouched");
    assert!(slime(&game, 0) > 0.0, "the splash coated the tank");
}

// 7. A splash coats opposing tanks and not its own side; a coated tank
//    corrodes at `bio_slime_dps` and drives at `bio_slime_speed_factor`;
//    another coat resets the timer rather than adding to it.
#[test]
fn a_splash_spares_its_own_side() {
    // The enemy waits in water beside the tank the mortar lobs at: a coat
    // would be washed off the same frame with a `SlimeWashed`, and the
    // splash's damage would show.
    let map = map_with(
        "cells.\"20,12\" = { kind = \"bio_slush\", side = \"enemy\" }\n\
         cells.\"21,18\" = { kind = \"water\" }\n",
    );
    let mut game = game_on(&map, PlayerCount::ONE, 9);
    let enemy = game.first_enemy_slot();
    place(&mut game, 0, (20, 17));
    let mut events = Vec::new();
    for _ in 0..300 {
        place(&mut game, enemy, (21, 18));
        step(&mut game, Input::default());
        events.extend(game.events().iter().cloned());
        if events.iter().any(|e| matches!(e, Event::GlobSplashed { .. })) {
            break;
        }
    }
    assert!(events.iter().any(|e| matches!(e, Event::GlobSplashed { .. })), "the glob came down");
    assert!(events.iter().any(|e| matches!(e, Event::Slimed { slot: 0 })), "the player is coated");
    assert!(!events.iter().any(|e| matches!(e, Event::SlimeWashed { slot } if *slot == enemy)), "its own side is not");
    assert_eq!(snapshot(&game, enemy).damage, 0.0);
}

#[test]
fn a_coat_corrodes_slows_and_resets_rather_than_stacks() {
    let mut game = game_on(&map_with(""), PlayerCount::ONE, 10);
    place(&mut game, 0, (20, 15));
    let t = tuning();
    set_slime(&mut game, 0, 1.0);
    let pace = with_tank(&game.world, player_tank(&game, 0), |tank| tank.slime_pace());
    assert!((pace - t.bio_slime_speed_factor).abs() < 1e-6, "a coated tank drives at the slime's pace");
    idle(&mut game, 30);
    let corroded = snapshot(&game, 0).damage;
    let half_second = t.bio_slime_dps * 0.5 * t.player_armor_factor;
    assert!((corroded - half_second).abs() < 0.05, "half a second of corrosion: {corroded}");
    idle(&mut game, 60);
    assert_eq!(slime(&game, 0), 0.0, "the coat wears off");
    let after = snapshot(&game, 0).damage;
    idle(&mut game, 30);
    assert_eq!(snapshot(&game, 0).damage, after, "and stops corroding");
    assert_eq!(with_tank(&game.world, player_tank(&game, 0), |tank| tank.slime_pace()), 1.0);

    // A puddle under the hull coats it every frame: the timer is reset to
    // a full coat, never stacked past one.
    game.ooze.insert((20, 15), OozePuddle { left: 2.0, total: 2.0 });
    for _ in 0..60 {
        step(&mut game, Input::default());
        assert!(slime(&game, 0) <= t.bio_slime_seconds + 1e-4);
    }
    assert!(slime(&game, 0) > t.bio_slime_seconds - 0.05);
}

// 8. A puddle slimes a tank of either side that drives over it, costs the
//    router `bio_puddle_path_cost` without blocking, and dries after its
//    time; a glob landing in water leaves none.
#[test]
fn a_puddle_slimes_either_side_and_dries() {
    let mut game = game_on(&map_with(""), PlayerCount::ONE, 11);
    place(&mut game, 0, (20, 15));
    game.ooze.insert((20, 15), OozePuddle { left: 1.0, total: 1.0 });
    let events = idle(&mut game, 1);
    assert!(events.iter().any(|e| matches!(e, Event::Slimed { slot: 0 })), "any tank over ooze is coated");
    idle(&mut game, 58);
    assert!(game.ooze.contains_key(&(20, 15)), "still wet just short of its time");
    idle(&mut game, 2);
    assert!(game.ooze.is_empty(), "dry after its time");
}

#[test]
fn a_puddle_costs_the_router_without_blocking() {
    let mut game = game_on(&map_with(""), PlayerCount::ONE, 12);
    place(&mut game, 0, (20, 15));
    let (a, b) = (cell_to_world(8, 5), cell_to_world(14, 5));
    let dry = game.route_grid(W, H).path_cost(a, b).expect("open field");
    for row in 0..23 {
        game.ooze.insert((11, row), OozePuddle { left: 5.0, total: 5.0 });
    }
    let wet = game.route_grid(W, H).path_cost(a, b).expect("ooze never blocks");
    // A map cell sits on a routing-cell corner, so a line of puddles is two
    // routing cells deep.
    assert_eq!(wet, dry + 2 * tuning().bio_puddle_path_cost as u32, "the ooze crossed at its surcharge");
}

#[test]
fn a_glob_in_water_leaves_no_puddle() {
    let map = map_with(
        "cells.\"20,12\" = { kind = \"bio_slush\", side = \"enemy\" }\n\
         cells.\"20,17\" = { kind = \"water\" }\n",
    );
    let mut game = game_on(&map, PlayerCount::ONE, 13);
    place(&mut game, 0, (20, 17));
    let mut landed = false;
    for _ in 0..300 {
        step(&mut game, Input::default());
        if game.events().iter().any(|e| matches!(e, Event::GlobSplashed { .. })) {
            landed = true;
            break;
        }
    }
    assert!(landed);
    assert!(game.ooze.is_empty(), "water takes the ooze");
    assert_eq!(slime(&game, 0), 0.0);
}

// 9. A slimed tank that drives into water is clean on the next frame.
#[test]
fn water_washes_a_coat_off() {
    let mut game = game_on(&map_with("cells.\"20,15\" = { kind = \"water\" }\n"), PlayerCount::ONE, 14);
    place(&mut game, 0, (20, 15));
    set_slime(&mut game, 0, 3.0);
    let events = idle(&mut game, 1);
    assert!(events.iter().any(|e| matches!(e, Event::SlimeWashed { slot: 0 })));
    assert_eq!(slime(&game, 0), 0.0);
}

// 10. Damage takes a tower through its four stages; below
//     `tower_burn_below` it burns, losing `tower_burn_dps`, at the burning
//     fire factor.
#[test]
fn a_hurt_tower_shows_its_stages_and_burns_low() {
    let map = map_with("cells.\"10,10\" = { kind = \"gun_tower\" }\n");
    let mut game = game_on(&map, PlayerCount::ONE, 15);
    place(&mut game, 0, (20, 15));
    let stage = |game: &Game| game.tower_views().iter().find(|v| v.kind == TowerKind::Gun).expect("standing").stage;
    assert_eq!(stage(&game), 0);
    for (frac, want) in [(0.7, 1), (0.45, 2), (0.2, 3)] {
        wound(&mut game, (10, 10), frac);
        assert_eq!(stage(&game), want, "at {frac} of its health");
    }
    assert!(!tower(&game, (10, 10)).burning, "above the burn line it only smokes");
    let t = tuning();
    wound(&mut game, (10, 10), t.tower_burn_below * 0.8);
    let before = tile_health(&game, (10, 10)).unwrap().0;
    let events = idle(&mut game, 30);
    assert!(events.iter().any(|e| matches!(e, Event::Ignited { what: "tower", .. })), "it catches");
    let burning = tower(&game, (10, 10));
    assert!(burning.burning);
    assert_eq!(burning.fire_factor(), t.tower_burning_fire_factor);
    let lost = before - tile_health(&game, (10, 10)).unwrap().0;
    assert!((lost - t.tower_burn_dps * 0.5).abs() < 0.05, "half a second of burning: {lost}");
}

// 11. A flamethrower held on a pristine tower lights it.
#[test]
fn a_flamethrower_lights_a_pristine_tower() {
    let map = map_with("cells.\"20,13\" = { kind = \"gun_tower\" }\n");
    let mut game = game_on(&map, PlayerCount::ONE, 16);
    place(&mut game, 0, (20, 15));
    game.debug_set_tank(0, &TankPatch { flame_fuel: Some(10.0), ..TankPatch::default() }).unwrap();
    let fire = Input::single(Intent { fire: true, ..Intent::default() });
    let frames = (tuning().flame_ignite_seconds / DT) as usize + 30;
    let mut lit = false;
    for _ in 0..frames {
        step(&mut game, fire);
        lit |= game.events().iter().any(|e| matches!(e, Event::Ignited { what: "tower", .. }));
    }
    assert!(lit, "the stream's heat lights it");
    assert!(tower(&game, (20, 13)).burning);
}

// 12. A dead tower leaves an open cell, a ruin and no `Tower` entry; each
//     kind goes out its own way.
fn burn_out(game: &mut Game, cell: (i32, i32)) -> Vec<Event> {
    wound(game, cell, 0.0001);
    let events = idle(game, 3);
    assert!(events.iter().any(|e| matches!(e, Event::ObstacleDestroyed { .. })), "the tower burnt out");
    events
}

#[test]
fn a_dead_tesla_discharges_into_what_is_close() {
    let mut game = game_on(&map_with(&enemy_tesla((20, 12))), PlayerCount::ONE, 17);
    place(&mut game, 0, (21, 13));
    let events = burn_out(&mut game, (20, 12));
    assert!(game.towers.is_empty(), "its weapon is gone");
    assert!(tile_health(&game, (20, 12)).is_none(), "its cell is open");
    assert_eq!(game.tower_ruins().len(), 1);
    assert!(strikes(&events).iter().any(|s| s.0), "a discharge bolt");
    assert!(snapshot(&game, 0).damage > 0.0, "that hurt the tank beside it");
}

#[test]
fn a_dead_gun_tower_cooks_off_and_a_dead_mortar_spills() {
    let map = map_with(
        "cells.\"10,10\" = { kind = \"gun_tower\" }\n\
         cells.\"14,10\" = { kind = \"bio_slush\" }\n",
    );
    let mut game = game_on(&map, PlayerCount::ONE, 18);
    place(&mut game, 0, (20, 15));
    wound(&mut game, (10, 10), 0.0001);
    let mut cooked = false;
    for _ in 0..3 {
        step(&mut game, Input::default());
        cooked |= !game.cookoffs.is_empty();
    }
    assert!(cooked, "its ammunition cooks off");
    assert!(!game.towers.contains_key(&(10, 10)));
    burn_out(&mut game, (14, 10));
    assert!(game.ooze.contains_key(&(14, 10)), "the mortar spills where it stood");
    assert_eq!(game.tower_ruins().len(), 2);
}

// 13. A shell sometimes glances off a tower with an `Event::Ricochet`.
#[test]
fn shells_sometimes_glance_off_a_tower() {
    let map = map_with("cells.\"20,12\" = { kind = \"gun_tower\" }\n");
    let fire = Input::single(Intent { fire: true, ..Intent::default() });
    let (mut ricochets, mut hits) = (0, 0);
    for seed in 0..6 {
        let mut game = game_on(&map, PlayerCount::ONE, 100 + seed);
        place(&mut game, 0, (20, 15));
        game.debug_set_tank(0, &TankPatch { shells_ammo: Some(99), ..TankPatch::default() }).unwrap();
        for _ in 0..600 {
            step(&mut game, fire);
            for e in game.events() {
                match e {
                    Event::Ricochet { .. } => ricochets += 1,
                    Event::Hit { target: HitTarget::Obstacle { material: Material::GunTower }, .. } => hits += 1,
                    _ => {}
                }
            }
            if !game.towers.contains_key(&(20, 12)) {
                break;
            }
        }
    }
    assert!(ricochets > 0, "some shells glance off ({hits} hit)");
    assert!(hits > ricochets, "most land ({ricochets} glanced, {hits} hit)");
}

// 14. The tower pack restores every standing tower of the collector's side
//     and puts it out; it is left lying while they are all whole.
#[test]
fn a_tower_pack_mends_and_puts_out_the_players_towers() {
    let map = map_with(
        "cells.\"10,10\" = { kind = \"tesla\" }\n\
         cells.\"12,10\" = { kind = \"gun_tower\" }\n\
         cells.\"30,4\" = { kind = \"gun_tower\", side = \"enemy\" }\n\
         cells.\"20,15\" = { kind = \"pickup\", pickup = \"tower_pack\" }\n",
    );
    let mut game = game_on(&map, PlayerCount::ONE, 19);
    let t = tuning();
    wound(&mut game, (10, 10), 0.6);
    wound(&mut game, (12, 10), t.tower_burn_below * 0.5);
    wound(&mut game, (30, 4), 0.5);
    idle(&mut game, 2);
    assert!(tower(&game, (12, 10)).burning, "set alight before the pack arrives");
    place(&mut game, 0, (20, 15));
    let events = idle(&mut game, 2);
    assert!(events.iter().any(|e| matches!(e, Event::PickupCollected { slot: 0, kind: PickupKind::TowerPack, .. })));
    assert_eq!(events.iter().filter(|e| matches!(e, Event::TowerRepaired { side: Side::Player, .. })).count(), 2);
    for cell in [(10, 10), (12, 10)] {
        let (health, max) = tile_health(&game, cell).unwrap();
        assert!(max - health < 0.5, "{cell:?} back to full: {health}/{max}");
    }
    assert!(!tower(&game, (12, 10)).burning, "put out");
    let (enemy, max) = tile_health(&game, (30, 4)).unwrap();
    assert!(enemy < max * 0.6, "the enemy's tower is not the player's to mend");
}

#[test]
fn a_tower_pack_is_left_lying_while_every_tower_is_whole() {
    let map = map_with(
        "cells.\"10,10\" = { kind = \"tesla\" }\n\
         cells.\"20,15\" = { kind = \"pickup\", pickup = \"tower_pack\" }\n",
    );
    let mut game = game_on(&map, PlayerCount::ONE, 20);
    place(&mut game, 0, (20, 15));
    let events = idle(&mut game, 10);
    assert!(!events.iter().any(|e| matches!(e, Event::PickupCollected { kind: PickupKind::TowerPack, .. })));
    assert!(game.world.query::<&Pickup>().iter().any(|p| p.kind == PickupKind::TowerPack), "still lying there");
}

// 15. Towers that fire nothing draw nothing: a bio tower draws no RNG of its
//     own (its scatter is hashed and its coat rolls nothing), so a round
//     with one lobbing at the player is the same stream as the round with
//     an iron wall in its place.
#[test]
fn a_mortar_draws_no_rng() {
    let run = |tile: &str| {
        let mut game = game_on(&map_with(&format!("cells.\"20,12\" = {tile}\n")), PlayerCount::ONE, 21);
        place(&mut game, 0, (20, 17));
        let events = idle(&mut game, 240);
        let lobbed = tower_fired(&events, TowerKind::Bio);
        let mut rng = game.rng.clone().expect("seeded");
        (lobbed, rng.random::<u64>())
    };
    let (lobbed, with_mortar) = run("{ kind = \"bio_slush\", side = \"enemy\" }");
    assert!(lobbed > 0, "the mortar was busy");
    let (_, with_iron) = run("{ kind = \"wall\", material = \"iron\" }");
    assert_eq!(with_mortar, with_iron);
}

// The enemies route round the player's towers: every cell in a standing
// player tower's reach carries `route_tower_cost`.
#[test]
fn enemies_route_round_the_players_towers() {
    let open = map_with("");
    let guarded = map_with("cells.\"11,3\" = { kind = \"tesla\" }\n");
    let (a, b) = (cell_to_world(8, 5), cell_to_world(14, 5));
    let cost = |map: &str| {
        let mut game = game_on(map, PlayerCount::ONE, 22);
        place(&mut game, 0, (20, 15));
        game.route_grid(W, H).path_cost(a, b).expect("a route")
    };
    let (g, o) = (cost(&guarded), cost(&open));
    assert!(g > o, "the tesla's reach is dearer ground: {g} vs {o}");
}

// An enemy a tower hurt fires back at it when lined up.
#[test]
fn an_enemy_a_tower_hurt_fires_back() {
    let map = map_with("cells.\"20,8\" = { kind = \"tesla\" }\n");
    let mut game = game_on(&map, PlayerCount::ONE, 23);
    let enemy = game.first_enemy_slot();
    place(&mut game, 0, (4, 20));
    let mut shot_back = false;
    for _ in 0..400 {
        game.debug_teleport(enemy, cell_to_world(20, 11), None).unwrap();
        step(&mut game, Input::default());
        shot_back |= game
            .events()
            .iter()
            .any(|e| matches!(e, Event::Hit { target: HitTarget::Obstacle { material: Material::Tesla }, .. }));
        if shot_back {
            break;
        }
    }
    assert!(shot_back, "the struck tank turned on the tesla");
}

