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
cells."2,2" = { kind = "bong" }
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
    // The blast tossed it too (`toss_crates`): the seat waits where it lands.
    let (at, _) = lies(&game, 11, 10).expect("it lies somewhere");
    game.debug_teleport(0, Position::new(at.x, at.y + 32.0), Some(0.0)).expect("player 1");
    let mut taken = None;
    for _ in 0..40 {
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

/// Where the pickup of slot cell (`c`, `r`) lies now (`Pickup::at`) and
/// how long it has left in the air.
fn lies(game: &Game, c: i32, r: i32) -> Option<(Position, f32)> {
    let at = cell_to_world(c, r);
    game.world.query::<&Pickup>().iter().find(|p| p.position.distance_to(at) < 0.5).map(|p| (p.at(), p.airborne))
}

/// A blast tosses a crate away from it, a hop it cannot be taken in, to
/// where it lands; one past the toss's reach stays put, and the same seed
/// lands it on the same spot.
#[test]
fn a_blast_tosses_a_crate_away_and_it_lands_to_be_taken() {
    let run = || {
        let mut game = round(&[(13, 10, "ammo"), (30, 10, "health")], false);
        let near_from = lies(&game, 13, 10).expect("the near crate").0;
        let far_from = lies(&game, 30, 10).expect("the far crate").0;
        game.debug_detonate(cell_to_world(10, 10)).unwrap();
        step(&mut game);
        let tossed: Vec<(f32, f32, f32, f32)> = game
            .events()
            .iter()
            .filter_map(|e| match *e {
                Event::CrateTossed { x, y, to_x, to_y, .. } => Some((x, y, to_x, to_y)),
                _ => None,
            })
            .collect();
        assert_eq!(tossed.len(), 1, "the near crate is tossed: {tossed:?}");
        let (at, airborne) = lies(&game, 13, 10).expect("it keeps its slot");
        assert!(airborne > 0.0, "it is in the air");
        assert!(at.x - near_from.x > 24.0 && (at.y - near_from.y).abs() < 0.5, "away from the blast: {near_from:?} -> {at:?}");
        assert_eq!(lies(&game, 30, 10).map(|l| l.0), Some(far_from), "past the reach it stays put");
        for _ in 0..60 {
            step(&mut game);
        }
        assert_eq!(lies(&game, 13, 10).map(|l| l.1), Some(0.0), "and lands");
        at
    };
    let (a, b) = (run(), run());
    assert_eq!((a.x.to_bits(), a.y.to_bits()), (b.x.to_bits(), b.y.to_bits()), "the same spot on the same seed");
}

/// A crate in the air cannot be taken; landed, it is taken where it lies.
#[test]
fn a_tossed_crate_is_taken_where_it_lands_and_not_in_the_air() {
    let mut game = round(&[(13, 10, "ammo")], false);
    game.debug_detonate(cell_to_world(10, 10)).unwrap();
    step(&mut game);
    let (at, _) = lies(&game, 13, 10).expect("tossed");
    // The seat parked right on where it lands, the frame after the blast.
    game.debug_teleport(0, at, Some(0.0)).unwrap();
    step(&mut game);
    assert!(lies(&game, 13, 10).is_some(), "nobody takes a crate in the air");
    for _ in 0..60 {
        step(&mut game);
    }
    assert!(lies(&game, 13, 10).is_none(), "landed, it is taken where it lies");
}

/// A toss stops short of a wall, deep water and lava: the crate comes down
/// on the last clear spot.
#[test]
fn a_tossed_crate_never_lands_in_a_wall_water_or_lava() {
    for blocker in ["kind = \"wall\", material = \"iron\"", "kind = \"water\"", "kind = \"lava\""] {
        let mut map = map_with(&[(12, 10, "ammo")]);
        // A block of it two cells east of the crate, too wide to fly over.
        for c in 14..=18 {
            for r in 8..=12 {
                map.push_str(&format!("cells.\"{c},{r}\" = {{ {blocker} }}\n"));
            }
        }
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(0xC4A7E);
        game.player_row_override = Some(0);
        game.show_intro = false;
        game.map = MapFile::from_toml_str(&map).expect("test map parses");
        game.init(W, H);
        game.debug_teleport(1, cell_to_world(ENEMY_CELL.0, ENEMY_CELL.1), Some(0.0)).unwrap();
        game.debug_detonate(cell_to_world(10, 10)).unwrap();
        step(&mut game);
        let (at, _) = lies(&game, 12, 10).expect("the crate");
        assert!(at.x > cell_to_world(12, 10).x, "{blocker}: tossed east");
        // A wall's cell and lava of any depth stop it; water only where it
        // is deep - a ford's shallows are ground a crate may lie on.
        let edge = cell_to_world(14, 10).x - crate::OBSTACLE_GRID_SIZE * 0.5;
        let clear = match blocker {
            b if b.contains("water") => game.water.depth_at(at) != crate::ground::Depth::Deep,
            _ => at.x < edge,
        };
        assert!(clear, "{blocker}: short of it, at {at:?}");
    }
}
