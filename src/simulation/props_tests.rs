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
            Event::Hit { target: HitTarget::Obstacle { .. }, killed, y, .. } => Some((hit_row(*y), *killed)),
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
    let terrain = Terrain::build(&game.world, W, H, &game.grass_cells);
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

#[test]
fn a_wood_tile_that_burns_out_reports_its_destruction() {
    // Wood is the one material with two death paths: `damage_obstacle` kills
    // it outright when it is not flammable, and `Obstacle::tick_burn` chars
    // it out after `wood_burn_seconds` when it is. Both have to announce the
    // death, or the one material that dies by fire is also the one that
    // silently leaves nothing behind for the debris to hang off.
    // `wood_flammable_chance` is rolled at spawn and tests cannot touch the
    // tuning table, so sweep seeds and assert only on the rounds that
    // actually caught fire.
    let map = map_with("cells.\"20,11\" = { kind = \"wall\", material = \"wood\" }\n");
    let mut burned = 0;
    for seed in 1..=20u64 {
        let mut game = game_on(&map, seed);
        game.debug_teleport(0, cell_to_world(20, 15), Some(0.0)).unwrap();
        step(&mut game, fire());
        // Watch the whole burn through, frame by frame: the tile is
        // despawned the frame it chars out, so a settle loop longer than
        // `wood_burn_seconds` would find nothing left to inspect.
        let mut lit = false;
        let mut reported = false;
        for _ in 0..300 {
            step(&mut game, Input::default());
            lit |= game.world.query::<&Obstacle>().iter().any(|o: &Obstacle| o.burning);
            reported |= game
                .events()
                .iter()
                .any(|e| matches!(e, Event::ObstacleDestroyed { material: Material::Wood, .. }));
        }
        if !lit {
            continue; // not flammable this seed: it died outright
        }
        burned += 1;
        assert!(reported, "seed {seed}: a wood tile burnt out without an ObstacleDestroyed event");
    }
    assert!(burned > 0, "no seed lit a wood tile - the test proved nothing");
}

#[test]
fn a_destroyed_wall_leaves_rubble_where_it_stood() {
    // Brick, wood and glass each have a terminal sheet cell that is never
    // drawn while the tile is alive, so a dead tile leaves it behind at no
    // art cost.
    for material in [Material::Brick, Material::Glass] {
        let kind = if material == Material::Brick { "brick" } else { "glass" };
        let map = map_with(&format!("cells.\"20,11\" = {{ kind = \"wall\", material = \"{kind}\" }}\n"));
        let mut game = game_on(&map, 1);
        game.debug_teleport(0, cell_to_world(20, 15), Some(0.0)).unwrap();
        for _ in 0..8 {
            if alive_obstacles(&game) == 8 {
                break;
            }
            shoot(&mut game, 90);
        }
        assert_eq!(alive_obstacles(&game), 8, "{material:?} tile never died");
        assert_eq!(game.decals.len(), 1, "{material:?} left exactly one piece of rubble");
        let decal = &game.decals[0];
        assert_eq!(Some((decal.sheet, decal.row)), material.rubble_row(false), "the rubble remembers what died");
        assert_eq!(decal.center, cell_to_world(20, 11), "rubble sits where the tile stood");
        assert!((0..crate::RUBBLE_VARIANTS).contains(&decal.col), "picks one of the row's variants");
    }

    // Every destructible material leaves something; Iron is the one that
    // never dies, so it is the only one with no rubble row at all.
    for material in [Material::Sandbag, Material::Barrel, Material::Fence, Material::Tree, Material::Pine] {
        assert!(material.rubble_row(false).is_some(), "{material:?} leaves rubble too");
    }
    assert!(Material::Iron.rubble_row(false).is_none(), "iron never dies, so it never leaves rubble");
}

/// The trap `Material`'s doc comment warns about: `max_health` indexes a
/// `[f32; 4]` by `self as usize` for the four wall materials, so inserting a
/// new variant among them is a silent out-of-bounds read rather than a
/// compile error. Appending is safe; this asks every variant for its health
/// so a wrong insertion panics here instead of in a round.
#[test]
fn every_material_reports_a_sane_max_health() {
    use crate::obstacle::Sheet;
    for material in [
        Material::Brick, Material::Iron, Material::Wood, Material::Glass,
        Material::Sandbag, Material::Barrel, Material::Fence,
        Material::Tree, Material::Pine,
    ] {
        let hp = material.max_health();
        assert!(hp.is_finite() && hp > 0.0, "{material:?} max_health {hp}");
        assert!(material.variants() > 0, "{material:?} has no cosmetic variants");
    }
    // The wall block must stay first, because that is what the array index
    // means. `MATERIALS` is the list the edge-cap rows are keyed by.
    for (i, material) in crate::obstacle::MATERIALS.iter().enumerate() {
        assert_eq!(*material as usize, i, "{material:?} moved out of the wall block");
        assert!(material.is_wall());
    }
    assert!(Material::Tree.is_tree() && Material::Pine.is_tree());
    assert!(!Material::Tree.is_prop() && !Material::Tree.is_wall());
    assert_eq!(Material::Tree.sheet(), Sheet::Trees);
    assert_eq!(Sheet::Trees.cell(), crate::TREE_TEXTURE_SIZE);
}

#[test]
fn a_tree_shot_down_leaves_litter_from_its_own_sheet() {
    use crate::obstacle::Sheet;
    // Both species, and over several seeds because a tree rolls flammable
    // at spawn: one that catches fire dies through `tick_burns` (charred)
    // and one that does not dies through `damage_obstacle`. Either way it
    // must announce itself and leave litter.
    for (kind, material) in [("tree", Material::Tree), ("pine", Material::Pine)] {
        let map = map_with(&format!("cells.\"20,11\" = {{ kind = \"{kind}\" }}\n"));
        let mut charred_seen = 0;
        for seed in 1..=6u64 {
            let mut game = game_on(&map, seed);
            game.debug_teleport(0, cell_to_world(20, 15), Some(0.0)).unwrap();
            for _ in 0..14 {
                if alive_obstacles(&game) == 8 {
                    break;
                }
                shoot(&mut game, 90);
            }
            assert_eq!(alive_obstacles(&game), 8, "seed {seed}: the {kind} never came down");
            assert_eq!(game.decals.len(), 1, "seed {seed}: one piece of litter");
            let decal = &game.decals[0];
            assert_eq!(decal.sheet, Sheet::Trees, "leaf litter is green, so it is not on the walls sheet");
            if Some((decal.sheet, decal.row)) == material.rubble_row(true) {
                charred_seen += 1;
            } else {
                assert_eq!(Some((decal.sheet, decal.row)), material.rubble_row(false));
            }
            assert_eq!(decal.center, cell_to_world(20, 11));
        }
        assert!(charred_seen > 0, "{kind}: no seed rolled a tree that burned out");
    }
}

#[test]
fn a_tank_drives_through_a_tree_about_as_fast_as_through_a_sandbag() {
    // The player starts one cell south of the tile and drives north into
    // it. Trees sit in the brittle bracket with the props, not the wall
    // bracket: pushing one over takes a beat longer than a sandbag and
    // nothing like a stalled advance.
    let frames_for = |kind: &str, material: Material| {
        let map = map_with(&format!("cells.\"20,11\" = {{ kind = \"{kind}\" }}\n"));
        let mut game = game_on(&map, 1);
        game.debug_teleport(0, cell_to_world(20, 12), Some(0.0)).unwrap();
        destroyed_frame(&mut game, drive(Dir::Up), material, 900)
            .unwrap_or_else(|| panic!("{kind} survived a tank pushing into it for 15 seconds"))
    };
    let fence = frames_for("fence", Material::Fence);
    let sandbag = frames_for("sandbag", Material::Sandbag);
    let tree = frames_for("tree", Material::Tree);
    assert!(fence <= sandbag && sandbag <= tree, "fence {fence}, sandbag {sandbag}, tree {tree}");
    assert!(tree < sandbag * 3, "a tree should not read as a wall to drive at: tree {tree}, sandbag {sandbag}");
}

#[test]
fn one_shell_usually_fells_a_tree() {
    // The brittleness claim, on the *weakest* chassis (row 0, damage
    // factor 0.75) - anything heavier fells one every time. Over seeds
    // because a shell's damage is a roll, and because a tree that rolled
    // flammable dies a second later through `tick_burns` rather than on
    // the hit itself.
    for (kind, material) in [("tree", Material::Tree), ("pine", Material::Pine)] {
        let map = map_with(&format!("cells.\"20,11\" = {{ kind = \"{kind}\" }}\n"));
        let mut felled = 0;
        for seed in 1..=20u64 {
            let mut game = game_on(&map, seed);
            game.debug_teleport(0, cell_to_world(20, 15), Some(0.0)).unwrap();
            // One trigger pull, then long enough for a burning tree to char.
            let events = shoot(&mut game, 150);
            if events.iter().any(|e| matches!(e, Event::ObstacleDestroyed { material: m, .. } if *m == material)) {
                felled += 1;
            }
        }
        assert!(felled >= 12, "{kind}: only {felled}/20 single shells brought it down");
    }
}

#[test]
fn rubble_varies_from_tile_to_tile() {
    // The whole point of the rubble rows is that a levelled wall does not
    // read as a grid of clones, so neighbouring tiles must not all land on
    // the same cell and orientation. Both are position-hashed, so this is
    // a property of the hash rather than of any round - check a wall-sized
    // run of cells directly.
    use crate::decal::Decal;
    let forms: std::collections::HashSet<(i32, u32)> = (0..24)
        .map(|i| {
            let d = Decal::new(Material::Brick, cell_to_world(5 + i % 12, 5 + i / 12), false).expect("brick leaves rubble");
            (d.col, d.seed % 8)
        })
        .collect();
    assert!(forms.len() >= 12, "24 adjacent tiles produced only {} distinct rubble forms", forms.len());
}

#[test]
fn a_dying_tank_throws_wreckage_and_cooks_off() {
    // A barrel used to be the more dramatic of the two deaths: it got a
    // fireball, a screen flash and a scorch while a dying tank got only the
    // shockwave. A kill now lays down all three plus its own thrown parts
    // and a set of delayed pops.
    let mut game = game_on(&map_with(""), 5);
    let scorches_before = game.scorches.len();
    game.debug_kill(1).expect("enemy in slot 1 dies");
    step(&mut game, Input::default());

    assert!(!game.blast_fx.is_empty(), "a dying tank throws a fireball");
    assert!(game.scorches.len() > scorches_before, "and leaves a scorch under the hull");
    assert!(!game.impact_flashes.is_empty(), "and flashes");
    assert_eq!(game.decals.len(), tuning().wreck_parts as usize, "and throws its parts");
    assert!(game.decals.iter().all(|d| !d.landed()), "which are still in the air the frame it dies");
    assert_eq!(game.cookoffs.len(), tuning().cookoff_count as usize, "with its ammo queued to cook off");

    // The parts land where they were always going to land, and only then
    // settle onto the ground.
    let targets: Vec<Position> = game.decals.iter().map(|d| d.center).collect();
    for _ in 0..90 {
        step(&mut game, Input::default());
    }
    assert!(game.decals.iter().all(|d| d.landed()), "the parts settle");
    assert_eq!(game.decals.iter().map(|d| d.center).collect::<Vec<_>>(), targets, "landing spots never moved");
    assert!(
        game.events().iter().any(|e| matches!(e, Event::CookOff { .. }))
            || game.cookoffs.len() < tuning().cookoff_count as usize,
        "the secondaries fire after the kill"
    );
}

#[test]
fn overlapping_shocks_are_capped_and_keep_the_hardest_hit() {
    // `Game::shock` used to be one slot, last-write-wins, so a chained
    // barrel cascade showed a single ripple and a tank dying inside it
    // simply vanished. Several now play at once, capped at the shader's
    // array length, and eviction sorts on `remaining` - strength faded by
    // age - so it is the weakest that go rather than the oldest. Evicting
    // by age would throw away the kill that set the cascade off.
    let kill = Shockwave::scaled(Position::new(0.0, 0.0), SHOCK_KILL);
    let pop = Shockwave::scaled(Position::new(0.0, 0.0), SHOCK_COOKOFF);
    assert!(kill.remaining() > pop.remaining(), "a kill outranks a cook-off pop");
    let mut spent = Shockwave::scaled(Position::new(0.0, 0.0), SHOCK_KILL);
    spent.time = tuning().shockwave_duration;
    assert_eq!(spent.remaining(), 0.0, "and a finished one outranks nothing");

    // More simultaneous kills than the shader has slots for.
    let map = map_with("");
    let mut game = Game::default();
    game.enemy_count_override = Some(SHOCK_MAX + 3);
    game.seed_override = Some(9);
    game.player_row_override = Some(0);
    game.map = MapFile::from_toml_str(&map).expect("test map parses");
    game.init(W, H);
    for slot in 1..=SHOCK_MAX + 3 {
        let _ = game.debug_kill(slot);
    }
    step(&mut game, Input::default());
    assert_eq!(game.shocks.len(), SHOCK_MAX, "never more than the shader can take");
    assert!(game.shocks.iter().all(|s| s.strength == SHOCK_KILL), "and they are all real kills");
}

#[test]
fn a_shoved_wreck_settles_instead_of_sliding_forever() {
    // A wreck keeps its full-mass dynamic body but stops being driven, and
    // nothing in the world sets damping, so a shove used to send it gliding
    // until it hit something. It should now travel a little and come to
    // rest.
    //
    // Three enemies, not one: killing the only enemy wins the round, and
    // the end screen deliberately stops stepping physics - the wreck would
    // simply be frozen, which looks like "settled" for all the wrong
    // reasons.
    let mut game = Game::default();
    game.enemy_count_override = Some(3);
    game.seed_override = Some(4);
    game.player_row_override = Some(0);
    game.map = MapFile::from_toml_str(&map_with("")).expect("test map parses");
    game.init(W, H);
    game.debug_teleport(1, Position::new(640.0, 360.0), Some(0.0)).unwrap();
    game.debug_kill(1).expect("enemy in slot 1 dies");
    step(&mut game, Input::default());
    step(&mut game, Input::default());
    assert_eq!(game.outcome(), Outcome::Playing, "the round is still running");

    let body = {
        let mut q = game.world.query::<&Tank>();
        q.iter()
            .find(|t| t.is_wreck())
            .and_then(|t| t.body)
            .expect("the wreck is still in the world with its body")
    };
    game.physics.apply_impulse(body, Position::new(4000.0, 0.0));
    let launched = game.physics.velocity(body).length();
    assert!(launched > 100.0, "the shove actually landed ({launched} px/s)");
    for _ in 0..240 {
        step(&mut game, Input::default());
    }
    let speed = game.physics.velocity(body).length();
    assert!(speed < 3.0, "the wreck should have come to rest, but is at {speed} px/s");
}

#[test]
fn a_wreck_does_not_absorb_shells() {
    // A wreck kept its full hull and turret boxes in the hit sweep while
    // `apply_hit` discarded the damage, so a shot into one was consumed for
    // nothing - and the AI could not see the cover it was getting either.
    // Shots pass through instead.
    //
    // Three enemies again: killing the last one ends the round, and the end
    // screen stops the player firing at all.
    let map = map_with("cells.\"20,8\" = { kind = \"wall\", material = \"glass\" }\n");
    let mut game = Game::default();
    game.enemy_count_override = Some(3);
    game.seed_override = Some(6);
    game.player_row_override = Some(0);
    game.map = MapFile::from_toml_str(&map).expect("test map parses");
    game.init(W, H);
    game.debug_teleport(0, cell_to_world(20, 15), Some(0.0)).unwrap();
    // The spare two go into the iron box in the far corner, out of the way.
    for slot in [2, 3] {
        let _ = game.debug_teleport(slot, cell_to_world(ENEMY_CELL.0, ENEMY_CELL.1), Some(0.0));
    }
    // Park a wreck squarely between the player and the glass.
    game.debug_teleport(1, cell_to_world(20, 11), Some(0.0)).unwrap();
    game.debug_kill(1).expect("enemy in slot 1 dies");
    step(&mut game, Input::default());
    step(&mut game, Input::default());
    assert_eq!(game.outcome(), Outcome::Playing, "the round is still running");
    assert!(
        game.world.query::<&Tank>().iter().any(|t| t.is_wreck()),
        "there is a wreck in the line of fire"
    );

    let events = shoot(&mut game, 90);
    let hit_rows: Vec<i32> = obstacle_hits(&events).into_iter().map(|(row, _)| row).collect();
    assert!(
        hit_rows.contains(&8),
        "the shell reached the glass behind the wreck; obstacle hits were {hit_rows:?}"
    );
}

#[test]
fn two_enemies_ram_each_other() {
    // Enemy-vs-enemy contact used to be the one collision in the game that
    // did nothing: no damage, no event, no feedback, so a pileup looked
    // like ghosts overlapping.
    let mut game = Game::default();
    game.enemy_count_override = Some(3);
    game.seed_override = Some(11);
    game.player_row_override = Some(0);
    game.map = MapFile::from_toml_str(&map_with("")).expect("test map parses");
    game.init(W, H);
    // Two enemies nose to nose, the third parked out of the way.
    let _ = game.debug_teleport(3, cell_to_world(ENEMY_CELL.0, ENEMY_CELL.1), Some(0.0));
    game.debug_teleport(1, Position::new(624.0, 360.0), Some(90.0)).unwrap();
    game.debug_teleport(2, Position::new(640.0, 360.0), Some(270.0)).unwrap();

    let mut rams = Vec::new();
    for _ in 0..120 {
        step(&mut game, Input::default());
        rams.extend(game.events().iter().filter_map(|e| match e {
            Event::Ram { slot, other_slot, damage } if *slot != 0 => Some((*slot, *other_slot, *damage)),
            _ => None,
        }));
    }
    assert!(!rams.is_empty(), "two enemies shoved together traded ram damage");
    assert!(
        rams.iter().all(|(a, b, d)| *a != *b && *d > 0.0),
        "each ram names both parties and deals damage: {rams:?}"
    );
}

#[test]
fn wall_tiles_know_which_faces_are_exposed() {
    // The edge cap is what makes a run of tiles read as one structure. Its
    // mask is cached at spawn and refreshed only on destruction, so both
    // ends of that have to be right.
    use crate::obstacle::neighbour_mask;
    use std::collections::HashSet;

    // A 3x1 horizontal run: the middle has neighbours east and west, the
    // ends have one each.
    let run: HashSet<(i32, i32)> = [(10, 5), (11, 5), (12, 5)].into_iter().collect();
    let (e, w) = (0b0000_0100u8, 0b0100_0000u8);
    assert_eq!(neighbour_mask((11, 5), &run), e | w, "the middle is capped top and bottom only");
    assert_eq!(neighbour_mask((10, 5), &run), e, "the west end is exposed on three sides");
    assert_eq!(neighbour_mask((12, 5), &run), w, "and the east end likewise");
    assert_eq!(neighbour_mask((50, 50), &run), 0, "an isolated tile is exposed all round");

    // A diagonal bit only counts when both its adjacent orthogonals do -
    // the rule that collapses 256 combinations to the blob set's 47.
    let corner: HashSet<(i32, i32)> = [(0, 0), (1, 0), (0, 1), (1, 1)].into_iter().collect();
    let mask = neighbour_mask((0, 0), &corner);
    assert_eq!(mask & 0b0000_0100, 0b0000_0100, "east neighbour");
    assert_eq!(mask & 0b0001_0000, 0b0001_0000, "south neighbour");
    assert_eq!(mask & 0b0000_1000, 0b0000_1000, "and the SE diagonal, since both orthogonals are set");
    let only_diagonal: HashSet<(i32, i32)> = [(0, 0), (1, 1)].into_iter().collect();
    assert_eq!(neighbour_mask((0, 0), &only_diagonal), 0, "a diagonal alone never counts");
}

#[test]
fn destroying_a_tile_re_exposes_its_neighbours() {
    // Masks are cached, so the refresh on destruction is the part that can
    // silently rot: a wall would keep drawing an interior face where a hole
    // has just opened.
    let map = map_with(
        "cells.\"20,10\" = { kind = \"wall\", material = \"glass\" }\n\
         cells.\"20,11\" = { kind = \"wall\", material = \"glass\" }\n\
         cells.\"20,12\" = { kind = \"wall\", material = \"glass\" }\n",
    );
    let mut game = Game::default();
    game.enemy_count_override = Some(3);
    game.seed_override = Some(2);
    game.player_row_override = Some(0);
    game.map = MapFile::from_toml_str(&map).expect("test map parses");
    game.init(W, H);
    for slot in [1, 2, 3] {
        let _ = game.debug_teleport(slot, cell_to_world(ENEMY_CELL.0, ENEMY_CELL.1), Some(0.0));
    }
    game.debug_teleport(0, cell_to_world(20, 15), Some(0.0)).unwrap();

    let mask_of = |g: &Game, cell: (i32, i32)| {
        g.world.query::<&Obstacle>().iter().find(|o| o.cell() == cell).map(|o| o.edge_mask)
    };
    // The player is south of the column, so the tile it knocks out first is
    // the *nearest* one, (20,12) - and the neighbour that gains an exposed
    // face is the one behind it.
    let south_bit = 0b0001_0000u8;
    assert_eq!(
        mask_of(&game, (20, 11)).expect("middle tile is standing") & south_bit,
        south_bit,
        "it starts with a wall to its south"
    );

    for _ in 0..8 {
        if mask_of(&game, (20, 12)).is_none() {
            break;
        }
        shoot(&mut game, 90);
    }
    assert!(mask_of(&game, (20, 12)).is_none(), "the near tile is gone");
    assert_eq!(
        mask_of(&game, (20, 11)).expect("the middle tile is still standing") & south_bit,
        0,
        "its south face is exposed now and gets capped"
    );

}

/// Grass hides you *from the AI*, not just from its trigger.
///
/// Concealment gates four things - the shared alert, the attack tier, the
/// chase tier and ram damage - because any one left open ends with the pack
/// standing on top of a player it cannot see: `act_attack`'s unaligned
/// branch repositions toward the target, so gating only the shot still
/// walks them in.
///
/// **The player is pinned each frame, and that is not a cheat.** Without it
/// the test measures knockback, not concealment: an idle player takes a hit,
/// gets shoved out of a 32px cell, and is then shot in the open. Measured on
/// the previous version of this test, the player was actually concealed for
/// 73 of 900 frames while claiming to be hiding, and even a 5x5 patch only
/// got that to 132. Pinning isolates the AI's decision, which is the thing
/// this change is about.
#[test]
fn tall_grass_hides_the_player_from_the_ai() {
    let mut cells = String::new();
    for c in 19..=21 {
        for r in 10..=12 {
            cells.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"tall_grass\" }}\n"));
        }
    }
    let open = map_with("");
    let grassy = map_with(&cells);

    let hits_taken = |map: &str| {
        let mut game = Game::default();
        game.enemy_count_override = Some(3);
        game.seed_override = Some(3);
        game.player_row_override = Some(0);
        game.map = MapFile::from_toml_str(map).expect("test map parses");
        game.init(W, H);
        let cell = cell_to_world(20, 11);
        for (i, slot) in [1, 2, 3].iter().enumerate() {
            let _ = game.debug_teleport(*slot, cell_to_world(18 + i as i32 * 2, 5), Some(180.0));
        }
        let mut hits = 0;
        for _ in 0..900 {
            game.debug_teleport(0, cell, Some(0.0)).expect("player holds its ground");
            step(&mut game, Input::default());
            hits += game
                .events()
                .iter()
                .filter(|e| matches!(e, Event::Hit { target: HitTarget::Player { .. }, .. }))
                .count();
        }
        hits
    };

    let open_hits = hits_taken(&open);
    let grass_hits = hits_taken(&grassy);
    assert!(open_hits >= 5, "the control has to actually be dangerous, got {open_hits}");
    assert_eq!(grass_hits, 0, "a player who never leaves cover is never hit (open {open_hits})");
}

/// The cost of the mechanic: cover hides you until you use it. A tank the
/// player shoots is `hit_alert`ed, and that exempts it from every
/// concealment gate - it knows something is in there.
#[test]
fn shooting_from_cover_gives_the_player_away() {
    let mut cells = String::new();
    for c in 19..=21 {
        for r in 10..=12 {
            cells.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"tall_grass\" }}\n"));
        }
    }
    let mut game = game_on(&map_with(&cells), 5);
    let cell = cell_to_world(20, 11);
    game.debug_teleport(0, cell, Some(0.0)).unwrap();
    // One enemy straight up the axis, well inside attack range.
    game.debug_teleport(1, cell_to_world(20, 6), Some(180.0)).unwrap();

    // Fire and watch for return fire in the *same* loop: `hit_alert_timer`
    // runs out after `enemy_hit_alert_seconds`, so checking in a second pass
    // after the shooting stops just measures the alert expiring.
    let (mut landed, mut shot_back) = (false, false);
    for frame in 0..1200 {
        game.debug_teleport(0, cell, Some(0.0)).unwrap();
        // Tap the trigger periodically; shells are edge-triggered.
        let firing = frame % 40 == 0;
        step(&mut game, if firing { fire() } else { Input::default() });
        for e in game.events() {
            match e {
                Event::Hit { target: HitTarget::Enemy { .. }, .. } => landed = true,
                Event::Hit { target: HitTarget::Player { .. }, .. } => shot_back = true,
                _ => {}
            }
        }
    }
    assert!(landed, "the player firing from cover has to be able to land a hit at all");
    assert!(shot_back, "an enemy the player shot from cover shoots back into it");
}

#[test]
fn grass_is_sorted_by_where_each_tuft_is_rooted() {
    let map = map_with("cells.\"20,11\" = { kind = \"tall_grass\" }\ncells.\"20,9\" = { kind = \"tall_grass\" }\ncells.\"20,13\" = { kind = \"tall_grass\" }\n");
    let game = game_on(&map, 1);
    assert!(game.grass.len() >= 3, "the map's cells scattered some tufts");
    assert!(
        game.grass.windows(2).all(|w| w[0].base.y <= w[1].base.y),
        "tufts are not in root order: {:?}",
        game.grass.iter().map(|t| t.base.y).collect::<Vec<_>>()
    );
}

/// The trail: grass flattens under a hull and takes `grass_crush_recover_
/// seconds` to stand back up, so a tank leaves a matted path behind it.
#[test]
fn grass_flattens_under_a_tank_and_stands_back_up() {
    let map = map_with("cells.\"20,11\" = { kind = \"tall_grass\" }\n");
    let mut game = game_on(&map, 1);
    let crush = |g: &Game| g.grass.iter().fold(0.0f32, |m, t| m.max(t.crush));
    assert_eq!(crush(&game), 0.0, "nothing has driven through it yet");

    game.debug_teleport(0, cell_to_world(20, 11), Some(0.0)).unwrap();
    step(&mut game, Input::default());
    let flattened = crush(&game);
    assert!(flattened > 0.5, "a hull parked on the cell should mat it down, got {flattened}");

    // Drive off and give it time to recover. Well clear of the cell so the
    // crush radius no longer reaches it.
    game.debug_teleport(0, cell_to_world(20, 18), Some(0.0)).unwrap();
    step(&mut game, Input::default());
    let just_left = crush(&game);
    assert!(just_left > 0.4, "grass should still be flat the frame after the tank leaves, got {just_left}");
    for _ in 0..(60.0 * 5.0) as usize {
        step(&mut game, Input::default());
    }
    assert_eq!(crush(&game), 0.0, "five seconds is past the recovery, it should be standing again");
}

#[test]
fn conceals_is_a_cell_query() {
    // Cover is a property of the ground a tank stands on. Testing against
    // the drawn tufts instead would make being hidden depend on which way
    // the wind was blowing.
    let cell = cell_to_world(10, 7);
    assert!(crate::grass::conceals(&[cell], cell), "dead centre of the cell");
    let half = crate::OBSTACLE_GRID_SIZE / 2.0;
    assert!(
        crate::grass::conceals(&[cell], Position::new(cell.x + half - 1.0, cell.y - half + 1.0)),
        "just inside a corner"
    );
    assert!(
        !crate::grass::conceals(&[cell], Position::new(cell.x + half + 2.0, cell.y)),
        "just outside the edge"
    );
    assert!(!crate::grass::conceals(&[], cell), "a map with no grass conceals nothing");
}

#[test]
fn screen_flashes_are_spaced_out_and_cookoffs_stay_local() {
    // The whole-screen flash is the harshest thing a kill does, so it is
    // explicit state with a minimum gap: a multi-kill or a barrel chain
    // flashes once rather than strobing. A cook-off is a local pop and
    // never flashes the screen or punches an impact quad.
    let map = map_with("");
    let mut game = Game::default();
    game.enemy_count_override = Some(3);
    game.seed_override = Some(9);
    game.player_row_override = Some(0);
    game.map = MapFile::from_toml_str(&map).expect("test map parses");
    game.init(W, H);
    assert!(game.screen_flash.is_none(), "nothing flashes at the start of a round");

    game.debug_kill(1).expect("enemy in slot 1 dies");
    step(&mut game, Input::default());
    let first = game.screen_flash.expect("a kill flashes the screen");
    game.debug_kill(2).expect("enemy in slot 2 dies");
    step(&mut game, Input::default());
    let second = game.screen_flash.expect("the first flash is still fading");
    assert!(second > first, "a second kill inside the gap does not restart the flash");

    let gap_frames = (tuning().blast_screen_flash_min_gap_seconds * 60.0).ceil() as usize + 1;
    for _ in 0..gap_frames {
        step(&mut game, Input::default());
    }
    assert!(game.screen_flash.is_none(), "the flash has faded out");
    game.debug_kill(3).expect("enemy in slot 3 dies");
    step(&mut game, Input::default());
    assert!(game.screen_flash.is_some(), "a kill after the gap flashes again");
    assert!(!game.cookoffs.is_empty(), "with its ammo queued to cook off");

    // Let the kill's own flash and impact quad run out, then watch the
    // cook-offs pop: none of them may bring either back.
    for _ in 0..12 {
        step(&mut game, Input::default());
    }
    assert!(game.impact_flashes.is_empty() && game.screen_flash.is_none(), "the kill's own effects are over");
    let mut saw_cookoff = false;
    let mut frames = 0;
    while !game.cookoffs.is_empty() {
        step(&mut game, Input::default());
        frames += 1;
        assert!(frames < 60 * 12, "cook-offs finish within the window");
        saw_cookoff |= game.events().iter().any(|e| matches!(e, Event::CookOff { .. }));
        assert!(game.screen_flash.is_none(), "a cook-off never flashes the screen");
        assert!(game.impact_flashes.is_empty(), "or punches an impact quad");
        assert!(
            game.blast_fx.iter().filter(|b| b.time <= 1.0 / 60.0 + 1e-4).all(|b| b.secondary),
            "any fireball a cook-off starts is a secondary"
        );
    }
    assert!(saw_cookoff, "the secondaries fired");
}
