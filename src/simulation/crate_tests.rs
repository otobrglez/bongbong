//! Headless scenarios for the breakable crates (docs/CRATES_SPEC.md,
//! `simulation::crates`): what a blast and fire do to a pickup's crate with
//! `crate_breakable` off and on. The knob is latched per round
//! (`Game::crates_breakable`), so a test turns it on for its own round and
//! never touches the global table.

use super::*;
use crate::map::cell_to_world;

const W: f32 = 1280.0;
const H: f32 = 720.0;

/// The one enemy every map needs, boxed in iron in the far corner.
const ENEMY_CELL: (i32, i32) = (37, 20);

/// A drum and the crates beside it: the oil drum at (10,10) and `crates`
/// as (col, row, kind).
fn map_with(crates: &[(i32, i32, &str)]) -> String {
    let mut text = String::from(
        r#"
version = 1
tanks = 1
cells."2,2" = { kind = "frog" }
cells."20,18" = { kind = "start" }
cells."10,10" = { kind = "barrel", drum = "oil" }
"#,
    );
    for (c, r) in [(36, 19), (37, 19), (38, 19), (36, 20), (38, 20), (36, 21), (37, 21), (38, 21)] {
        text.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
    }
    for (c, r, kind) in crates {
        text.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"pickup\", pickup = \"{kind}\" }}\n"));
    }
    text
}

fn round(crates: &[(i32, i32, &str)], breakable: bool) -> Game {
    let mut game = Game::default();
    game.enemy_count_override = Some(1);
    game.seed_override = Some(0xC4A7E);
    game.player_row_override = Some(0);
    game.show_intro = false;
    game.map = MapFile::from_toml_str(&map_with(crates)).expect("test map parses");
    game.init(W, H);
    game.crates_breakable = breakable;
    game.debug_teleport(1, cell_to_world(ENEMY_CELL.0, ENEMY_CELL.1), Some(0.0)).expect("enemy in slot 1");
    game
}

fn step(game: &mut Game) {
    game.update(Input::default(), 1.0 / 60.0, W, H);
}

/// The pickup on cell (`c`, `r`), if any.
fn pickup_at(game: &Game, c: i32, r: i32) -> Option<(PickupKind, f32, Option<f32>, Option<f32>)> {
    let at = cell_to_world(c, r);
    game.world.query::<&Pickup>().iter().find(|p| p.position.distance_to(at) < 0.5).map(|p| (p.kind, p.health, p.burn, p.loose))
}

fn broken(game: &Game) -> Vec<(PickupKind, bool)> {
    game.events()
        .iter()
        .filter_map(|e| match *e {
            Event::CrateBroken { kind, cooked, .. } => Some((kind, cooked)),
            _ => None,
        })
        .collect()
}

#[test]
fn unbreakable_crates_stand_through_a_blast() {
    let mut game = round(&[(11, 10, "health"), (11, 11, "ammo")], false);
    game.debug_detonate(cell_to_world(10, 10)).expect("the drum");
    for _ in 0..30 {
        step(&mut game);
        assert!(broken(&game).is_empty(), "nothing breaks with the knob off");
    }
    let (_, health, burn, loose) = pickup_at(&game, 11, 10).expect("the health crate stands");
    assert_eq!((health, burn, loose), (tuning().crate_hp, None, None), "untouched");
    assert!(pickup_at(&game, 11, 11).is_some(), "the ammo crate stands");
}

#[test]
fn a_blast_breaks_a_crate_and_its_contents_lie_loose_then_go() {
    let mut game = round(&[(11, 10, "health")], true);
    game.debug_detonate(cell_to_world(10, 10)).expect("the drum");
    step(&mut game);
    // (A health slot may roll its bonus shield into a neighbouring cell,
    // which the drum can break too.)
    assert!(broken(&game).contains(&(PickupKind::Health, false)), "the drum broke the crate beside it");
    let (kind, _, _, loose) = pickup_at(&game, 11, 10).expect("its contents are still there");
    assert_eq!(kind, PickupKind::Health);
    assert!(loose.is_some(), "lying loose");
    let spill = tuning().crate_spill_seconds;
    for _ in 0..((spill - 0.5) * 60.0) as i32 {
        step(&mut game);
    }
    assert!(pickup_at(&game, 11, 10).is_some(), "still there just before its clock runs out");
    for _ in 0..60 {
        step(&mut game);
    }
    assert!(pickup_at(&game, 11, 10).is_none(), "gone once it has");
}

#[test]
fn spilled_contents_are_taken_like_a_crate() {
    let mut game = round(&[(11, 10, "health")], true);
    game.debug_detonate(cell_to_world(10, 10)).expect("the drum");
    step(&mut game);
    assert!(pickup_at(&game, 11, 10).is_some_and(|p| p.3.is_some()), "spilled");
    game.debug_teleport(0, cell_to_world(11, 11), Some(0.0)).expect("player 1");
    let mut taken = None;
    for _ in 0..10 {
        step(&mut game);
        taken = taken.or_else(|| {
            game.events().iter().find_map(|e| match *e {
                Event::PickupCollected { kind, spilled, .. } => Some((kind, spilled)),
                _ => None,
            })
        });
    }
    assert_eq!(taken, Some((PickupKind::Health, true)), "taken, and told apart from a whole crate");
    assert!(pickup_at(&game, 11, 10).is_none());
}

#[test]
fn an_ammo_crate_cooks_off_and_breaks_the_crate_beyond_it() {
    // The drum is too far from the shield crate to break it (64 px); the
    // ammo crate between them is not, and its cook-off finishes the job.
    let mut game = round(&[(11, 10, "ammo"), (12, 10, "shield")], true);
    game.debug_detonate(cell_to_world(10, 10)).expect("the drum");
    step(&mut game);
    let broke = broken(&game);
    assert!(broke.contains(&(PickupKind::Ammo, true)), "the ammo cooked off: {broke:?}");
    assert!(broke.contains(&(PickupKind::Shield, false)), "and broke the shield crate beside it: {broke:?}");
    assert!(pickup_at(&game, 11, 10).is_none(), "a crate that cooks off leaves nothing");
    assert!(pickup_at(&game, 12, 10).is_some_and(|p| p.3.is_some()), "the shield lies loose");
}

#[test]
fn the_drum_alone_does_not_reach_two_cells_away() {
    let mut game = round(&[(12, 10, "shield")], true);
    game.debug_detonate(cell_to_world(10, 10)).expect("the drum");
    for _ in 0..5 {
        step(&mut game);
    }
    assert!(broken(&game).is_empty());
    let (_, health, _, loose) = pickup_at(&game, 12, 10).expect("the crate stands");
    assert!(loose.is_none() && health < tuning().crate_hp, "hurt, not broken: {health}");
}

#[test]
fn fire_lights_a_crate_and_it_falls_in() {
    let mut game = round(&[(14, 14, "speedup")], true);
    // The flamethrower's heat on its cell, as the stream would leave it.
    game.heat.insert((14, 14), tuning().crate_ignite_seconds + 1.0);
    step(&mut game);
    assert!(pickup_at(&game, 14, 14).is_some_and(|p| p.2.is_some()), "it caught");
    let mut fell = false;
    for _ in 0..((tuning().crate_burn_seconds + 0.5) * 60.0) as i32 {
        step(&mut game);
        fell |= broken(&game).contains(&(PickupKind::SpeedUp, false));
    }
    assert!(fell, "it fell in once its fire burnt down");
    assert!(pickup_at(&game, 14, 14).is_some_and(|p| p.3.is_some()), "its contents lie loose");
}

#[test]
fn a_crate_round_replays_byte_for_byte() {
    let run = || {
        let mut game = round(&[(11, 10, "ammo"), (12, 10, "missiles"), (11, 11, "health")], true);
        game.debug_detonate(cell_to_world(10, 10)).expect("the drum");
        for _ in 0..120 {
            step(&mut game);
        }
        game.drawable_state()
    };
    assert_eq!(run(), run());
}
