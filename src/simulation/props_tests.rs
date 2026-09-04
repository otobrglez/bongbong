//! Headless scenario tests for the destructible props
//! (docs/sandbags-barrels-fences.md): sandbag pass-over and ram collapse,
//! fence one-shots, barrel blasts, chains and rams, and what blocks sight.
//! Tests cannot touch the global tuning table (the suite runs in parallel
//! threads), so chance rules are checked statistically over seeds against
//! the defaults; each round is seeded, so every run is repeatable.

use super::hits::Terrain;
use super::*;
use crate::ai::Intent;
use crate::map::cell_to_world;
use crate::obstacle::Material;
use crate::tank::Dir;

const W: f32 = 1280.0;
const H: f32 = 720.0;

/// The one enemy every map needs is boxed in iron in the far corner, so it
/// can neither see nor reach anything the test is about.
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

fn game_on(map: &str, seed: u64) -> Game {
    let mut game = Game::default();
    game.enemy_count_override = Some(1);
    game.seed_override = Some(seed);
    game.player_row_override = Some(0);
    game.map = MapFile::from_toml_str(map).expect("test map parses");
    game.init(W, H);
    game.debug_teleport(1, cell_to_world(ENEMY_CELL.0, ENEMY_CELL.1), Some(0.0)).expect("enemy in slot 1");
    game
}

fn step(game: &mut Game, input: Input) {
    game.update(input, 1.0 / 60.0, W, H);
}

fn fire() -> Input {
    Input { player_intent: Intent { fire: true, ..Intent::default() }, ..Input::default() }
}

fn drive(dir: Dir) -> Input {
    Input { player_intent: Intent { move_dir: Some(dir), ..Intent::default() }, ..Input::default() }
}

/// Fire one shell and let it land: `frames` of AFK after the trigger pull.
fn shoot(game: &mut Game, frames: usize) -> Vec<Event> {
    step(game, fire());
    let mut events = game.events().to_vec();
    for _ in 0..frames {
        step(game, Input::default());
        events.extend(game.events().iter().cloned());
    }
    events
}

/// Which grid row an obstacle hit at `y` landed on (hits land on a tile's
/// near face, so nearest centre is the right test).
fn hit_row(y: f32) -> i32 {
    ((y - 16.0) / 32.0).round() as i32
}

fn obstacle_hits(events: &[Event]) -> Vec<(i32, bool)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Hit { target: HitTarget::Obstacle, killed, y, .. } => Some((hit_row(*y), *killed)),
            _ => None,
        })
        .collect()
}

fn player_damage(game: &Game) -> f32 {
    game.tank_snapshots().iter().find(|t| t.is_player).expect("player snapshot").damage
}

fn alive_obstacles(game: &Game) -> usize {
    game.world.query::<&Obstacle>().iter().filter(|o| !o.destroyed).count()
}

fn destroyed_frame(game: &mut Game, input: Input, material: Material, max_frames: usize) -> Option<usize> {
    for frame in 1..=max_frames {
        step(game, input);
        if game.events().iter().any(|e| matches!(e, Event::ObstacleDestroyed { material: m, .. } if *m == material)) {
            return Some(frame);
        }
    }
    None
}

#[test]
fn shells_sometimes_pass_over_a_sandbag_and_hit_what_is_behind() {
    // Player at (20,15) facing up, a sandbag at (20,11), brick behind it at (20,8).
    let map = map_with(
        r#"
cells."20,11" = { kind = "sandbag" }
cells."20,8" = { kind = "wall", material = "brick" }
"#,
    );
    let (mut on_sandbag, mut on_brick) = (0, 0);
    for seed in 1..=60u64 {
        let mut game = game_on(&map, seed);
        game.debug_teleport(0, cell_to_world(20, 15), Some(0.0)).unwrap();
        let hits = obstacle_hits(&shoot(&mut game, 90));
        let rows: Vec<i32> = hits.iter().map(|(row, _)| *row).collect();
        if rows.contains(&11) {
            on_sandbag += 1;
        } else if rows.contains(&8) {
            on_brick += 1;
        } else {
            panic!("seed {seed}: the shell hit neither the sandbag nor the brick: {hits:?}");
        }
    }
    let total = (on_sandbag + on_brick) as f32;
    let pass_fraction = on_brick as f32 / total;
    assert!(on_sandbag > 0 && on_brick > 0, "sandbag {on_sandbag}, brick {on_brick}");
    assert!((0.1..=0.7).contains(&pass_fraction), "pass-over fraction {pass_fraction} is off the 0.35 default");
}

#[test]
fn a_fence_dies_in_at_most_two_hits_and_usually_one() {
    let map = map_with("cells.\"20,11\" = { kind = \"fence\" }\n");
    let (mut one_shot, mut two_shots) = (0, 0);
    for seed in 1..=40u64 {
        let mut game = game_on(&map, seed);
        game.debug_teleport(0, cell_to_world(20, 15), Some(0.0)).unwrap();
        let first: Vec<_> = obstacle_hits(&shoot(&mut game, 90)).into_iter().filter(|(row, _)| *row == 11).collect();
        assert_eq!(first.len(), 1, "seed {seed}: the first shell hit the fence once: {first:?}");
        if first[0].1 {
            one_shot += 1;
            assert_eq!(alive_obstacles(&game), 8, "the fence is gone");
            continue;
        }
        let second: Vec<_> = obstacle_hits(&shoot(&mut game, 90)).into_iter().filter(|(row, _)| *row == 11).collect();
        assert!(second.iter().any(|(_, killed)| *killed), "seed {seed}: a damaged fence always dies to the next hit");
        two_shots += 1;
    }
    assert!(one_shot > two_shots, "70% one-shot: {one_shot} one-shots vs {two_shots} two-shots");
    assert!(two_shots > 0, "some fences take two hits");
}

#[test]
fn a_tank_pushing_into_a_sandbag_flattens_it_and_a_fence_faster() {
    let sandbag = map_with("cells.\"20,13\" = { kind = \"sandbag\" }\n");
    let mut game = game_on(&sandbag, 3);
    game.debug_teleport(0, cell_to_world(20, 15), Some(0.0)).unwrap();
    let before = alive_obstacles(&game);
    let sandbag_frame = destroyed_frame(&mut game, drive(Dir::Up), Material::Sandbag, 240).expect("the sandbag collapses");
    assert!(sandbag_frame >= 24, "it takes sandbag_ram_seconds of pushing, not a touch: frame {sandbag_frame}");
    step(&mut game, Input::default());
    assert_eq!(alive_obstacles(&game), before - 1);

    let fence = map_with("cells.\"20,13\" = { kind = \"fence\" }\n");
    let mut game = game_on(&fence, 3);
    game.debug_teleport(0, cell_to_world(20, 15), Some(0.0)).unwrap();
    let fence_frame = destroyed_frame(&mut game, drive(Dir::Up), Material::Fence, 240).expect("the fence gives way");
    assert!(fence_frame < sandbag_frame, "fence {fence_frame} vs sandbag {sandbag_frame}");
}

#[test]
fn a_shot_barrel_detonates_and_hurts_the_shooter_in_range() {
    let map = map_with("cells.\"20,12\" = { kind = \"barrel\" }\n");
    let mut game = game_on(&map, 5);
    game.debug_teleport(0, cell_to_world(20, 14), Some(0.0)).unwrap();
    assert_eq!(player_damage(&game), 0.0);
    let mut blasted = false;
    for _ in 0..6 {
        let events = shoot(&mut game, 45);
        if events.iter().any(|e| matches!(e, Event::Blast { chained: false, .. })) {
            blasted = true;
            assert!(
                events.iter().any(|e| matches!(e, Event::ObstacleDestroyed { material: Material::Barrel, .. })),
                "the barrel is destroyed by its own blast"
            );
            break;
        }
    }
    assert!(blasted, "a few shells at point-blank range set the barrel off");
    assert!(player_damage(&game) > 0.0, "the shooter stood inside the blast radius");
    step(&mut game, Input::default());
    assert!(!game.blast_fx.is_empty(), "the fireball is playing");
    assert_eq!(game.scorches.len(), 1, "one scorch mark");
}

#[test]
fn barrels_chain_react_on_a_fuse() {
    // (23,11) is out of the first barrel's radius but inside the second's.
    let map = map_with(
        r#"
cells."20,10" = { kind = "barrel" }
cells."21,10" = { kind = "barrel" }
cells."23,11" = { kind = "barrel" }
"#,
    );
    let mut game = game_on(&map, 11);
    game.debug_teleport(0, cell_to_world(20, 14), Some(0.0)).unwrap();
    let mut log: Vec<(usize, bool)> = Vec::new();
    let mut frame = 0;
    'shots: for _ in 0..6 {
        step(&mut game, fire());
        frame += 1;
        for _ in 0..60 {
            step(&mut game, Input::default());
            frame += 1;
            for e in game.events() {
                if let Event::Blast { chained, .. } = e {
                    log.push((frame, *chained));
                }
            }
            if log.len() == 3 {
                break 'shots;
            }
        }
    }
    assert_eq!(log.len(), 3, "three blasts: {log:?}");
    assert!(!log[0].1 && log[1].1 && log[2].1, "the first is the shot, the rest chained: {log:?}");
    // A fuse runs up to 2.5x `barrel_fuse_seconds` at the blast's edge.
    let fuse_frames = (tuning().barrel_fuse_seconds * 2.5 * 60.0).ceil() as usize + 1;
    assert!(log[1].0 > log[0].0 && log[1].0 <= log[0].0 + fuse_frames, "the second waits out its fuse: {log:?}");
    assert!(log[2].0 > log[1].0 && log[2].0 <= log[1].0 + fuse_frames, "the third only chains off the second: {log:?}");
    assert_eq!(game.world.query::<&Obstacle>().iter().filter(|o| o.material == Material::Barrel).count(), 0);
}

#[test]
fn ramming_a_barrel_sets_it_off() {
    let map = map_with("cells.\"20,13\" = { kind = \"barrel\" }\n");
    let mut game = game_on(&map, 8);
    game.debug_teleport(0, cell_to_world(20, 15), Some(0.0)).unwrap();
    let mut blast_frame = None;
    for frame in 1..=180 {
        step(&mut game, drive(Dir::Up));
        if game.events().iter().any(|e| matches!(e, Event::Blast { chained: false, .. })) {
            blast_frame = Some(frame);
            break;
        }
    }
    assert!(blast_frame.is_some(), "pushing into a barrel pops it");
    assert!(player_damage(&game) > 0.0, "and the rammer pays for it");
}

#[test]
fn a_wrecks_splash_sets_off_a_barrel_next_to_it() {
    // The enemy's iron box with one ring tile swapped for a barrel.
    let mut map = map_with("cells.\"36,20\" = { kind = \"barrel\" }\n");
    map = map.replace("cells.\"36,20\" = { kind = \"wall\", material = \"iron\" }\n", "");
    let mut game = game_on(&map, 4);
    game.debug_kill(1).expect("enemy in slot 1");
    let mut saw = Vec::new();
    for _ in 0..30 {
        step(&mut game, Input::default());
        for e in game.events() {
            match e {
                Event::Wreck { slot: 1, .. } => saw.push("wreck"),
                Event::Blast { chained: true, .. } => saw.push("blast"),
                _ => {}
            }
        }
    }
    assert_eq!(saw, vec!["wreck", "blast"], "the wreck's explosion fuses the barrel, which then goes off");
}

#[test]
fn sandbags_and_fences_do_not_block_sight_but_barrels_and_walls_do() {
    let map = map_with(
        r#"
cells."10,5" = { kind = "sandbag" }
cells."10,8" = { kind = "fence" }
cells."10,11" = { kind = "barrel" }
cells."10,14" = { kind = "wall", material = "brick" }
"#,
    );
    let game = game_on(&map, 1);
    let terrain = Terrain::build(&game.world, W, H);
    let across = |row: i32| terrain.line_of_sight(cell_to_world(7, row), cell_to_world(13, row));
    assert!(across(5), "a sandbag is knee-high");
    assert!(across(8), "a fence is see-through");
    assert!(!across(11), "a barrel is a solid block");
    assert!(!across(14), "so is a wall");
}

#[test]
fn a_barrel_occasionally_deflects_or_is_flown_over() {
    let map = map_with("cells.\"20,10\" = { kind = \"barrel\" }\n");
    let (mut landed, mut missed) = (0, 0);
    for seed in 1..=80u64 {
        let mut game = game_on(&map, seed);
        game.debug_teleport(0, cell_to_world(20, 15), Some(0.0)).unwrap();
        let events = shoot(&mut game, 90);
        if obstacle_hits(&events).iter().any(|(row, _)| *row == 10) {
            landed += 1;
        } else {
            missed += 1;
        }
    }
    assert!(landed > missed, "most shots land: {landed} landed, {missed} missed");
    assert!(missed > 0, "at the 8% fly-over and 10% deflect defaults, some of 80 shots miss");
}

#[test]
fn a_map_without_props_spawns_the_same_round_it_always_did() {
    // Prop rules only draw RNG when a prop is involved, so a prop-free
    // round replays exactly (the seeded-replay guard, restated for props).
    let map = map_with("cells.\"20,11\" = { kind = \"wall\", material = \"brick\" }\n");
    let run = |seed: u64| {
        let mut game = game_on(&map, seed);
        game.debug_teleport(0, cell_to_world(20, 15), Some(0.0)).unwrap();
        let mut trace = Vec::new();
        for frame in 0..300 {
            step(&mut game, if frame % 50 == 0 { fire() } else { drive(Dir::Left) });
            trace.push(game.tank_snapshots().iter().map(|t| (t.position, t.damage)).collect::<Vec<_>>());
        }
        trace
    };
    assert!(run(21) == run(21));
}
