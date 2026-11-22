//! Hard and soft faces (docs/physics-engine-design.md "Hard and soft"): a
//! hard face throws a hull back, a soft one cushions it, glass cracks
//! under a fast ram, lamp posts and range boards go over, a tree's crown
//! shakes, and grass and bushes slow a hull a little - none of it ever
//! touching a hull sliding along a face.

use super::*;
use crate::obstacle::Surface;
use crate::tuning::Tuning;

const ASSAULT: i32 = 1;

fn cell(c: i32, r: i32) -> Position {
    crate::map::cell_to_world(c, r)
}

/// A row of `kind` cells (`material` for a wall) along row 8, columns
/// 8..=16.
fn row_of(kind: &str) -> String {
    (8..=16)
        .map(|c| match kind {
            "iron" | "brick" | "glass" | "concrete" | "wood" => format!("cells.\"{c},8\" = {{ kind = \"wall\", material = \"{kind}\" }}\n"),
            other => format!("cells.\"{c},8\" = {{ kind = \"{other}\" }}\n"),
        })
        .collect()
}

/// A one-seat round on a field holding only `extra`, stepped once so the
/// solver knows the map's colliders.
fn round(extra: &str) -> Game {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.player_row_override = Some(ASSAULT);
    game.level_overrides.mission = Some(Mission::Destroy);
    game.map = MapFile::from_toml_str(&format!("version = 1\ntanks = 0\ncells.\"1,1\" = {{ kind = \"start\" }}\n{extra}")).expect("test map parses");
    game.init(W, H);
    step(&mut game, Input::default());
    game
}

/// The seat driven down at the row from four cells above it for `frames`
/// steps: its y after every step, and the hits it reported.
fn drive_down(game: &mut Game, frames: usize) -> (Vec<f32>, Vec<Event>) {
    let player = game.player().expect("player");
    game.place_tank(player, cell(12, 4), Some(Dir::Down.rotation())).unwrap();
    let mut ys = Vec::new();
    let mut hits = Vec::new();
    for _ in 0..frames {
        drive(game, Some(Dir::Down), 1);
        ys.push(player_pos(game).y);
        hits.extend(game.events().iter().filter(|e| matches!(e, Event::HullStruck { .. })).cloned());
    }
    (ys, hits)
}

/// The steps of a run of positions.
fn steps(ys: &[f32]) -> Vec<f32> {
    ys.windows(2).map(|w| w[1] - w[0]).collect()
}

#[test]
fn every_material_has_a_surface_and_the_hardest_throw_furthest() {
    let t = Tuning::DEFAULT;
    for m in [Material::Tesla, Material::GunTower, Material::BioSlush, Material::Volcano] {
        assert_eq!(m.surface(), Surface::Hardest, "{m:?}");
    }
    for m in [Material::Iron, Material::Concrete, Material::Brick, Material::Door] {
        assert_eq!(m.surface(), Surface::Hard, "{m:?}");
    }
    assert_eq!(Material::Glass.surface(), Surface::Brittle);
    for m in [Material::Wood, Material::Tree, Material::Pine, Material::Spruce, Material::Scots, Material::Fir, Material::Palm] {
        assert_eq!(m.surface(), Surface::Firm, "{m:?}");
    }
    for m in [Material::Birch, Material::Willow, Material::Snag] {
        assert_eq!(m.surface(), Surface::Yielding, "{m:?}");
    }
    for m in [Material::Sandbag, Material::Fence, Material::Lamp, Material::Target] {
        assert_eq!(m.surface(), Surface::Soft, "{m:?}");
    }
    assert!(Surface::Hardest.bounce(&t) > Surface::Hard.bounce(&t) && Surface::Hard.bounce(&t) > 0.0);
    assert!(Surface::Hardest.jolt(&t) > Surface::Hard.jolt(&t) && Surface::Hard.jolt(&t) > Surface::Firm.jolt(&t) && Surface::Firm.jolt(&t) > Surface::Soft.jolt(&t));
    for s in [Surface::Hardest, Surface::Hard, Surface::Brittle, Surface::Plain] {
        assert_eq!(s.cushion(&t), 0.0, "{s:?} stops a hull where it meets it");
    }
    assert!(Surface::Soft.cushion(&t) < Surface::Yielding.cushion(&t) && Surface::Yielding.cushion(&t) < Surface::Firm.cushion(&t), "the softer, the longer the give");
    for s in Surface::ALL {
        assert_eq!(Surface::from_code(s as u128), s);
    }
}

#[test]
fn a_hull_driven_into_iron_is_thrown_back_and_one_driven_into_sandbags_is_cushioned() {
    // Iron: full speed up to the face, then a step back off it.
    let mut game = round(&row_of("iron"));
    let (ys, hits) = drive_down(&mut game, 80);
    // The step it met the face on: the last before it first went back.
    let at = steps(&ys).iter().position(|&d| d < -0.01).expect("it is thrown back");
    let back = ys[at] - ys[at..(at + 12).min(ys.len())].iter().copied().fold(f32::MAX, f32::min);
    assert!(back >= 1.5, "iron throws the hull back: {back} px");
    let before = steps(&ys[..=at]);
    let last = before.iter().rev().take(2).copied().collect::<Vec<_>>();
    assert!(last.iter().all(|&s| s > 3.0), "and it meets it at full speed: {last:?}");
    let bounced: Vec<&Event> = hits.iter().filter(|e| matches!(e, Event::HullStruck { bounced: true, surface: Surface::Hard, material: Some(Material::Iron), .. })).collect();
    assert_eq!(bounced.len(), 1, "one throw for one hit, however long it leans on: {hits:?}");

    // Sandbags: the hull slows over the last few px and is never thrown.
    let mut game = round(&row_of("sandbag"));
    let (ys, hits) = drive_down(&mut game, 45);
    let s = steps(&ys);
    assert!(s.iter().all(|&d| d >= -0.01), "a sandbag throws nothing back: {s:?}");
    let slowing = s.iter().filter(|&&d| d > 0.3 && d < 3.0).count();
    assert!(slowing >= 2, "it slows over a few steps before the face: {s:?}");
    assert!(hits.iter().all(|e| matches!(e, Event::HullStruck { bounced: false, .. })), "{hits:?}");
}

#[test]
fn the_cushion_and_the_bounce_leave_a_hull_sliding_along_a_face_alone() {
    // Driven east along the row's north face, flank against it: as far
    // along a sandbag row or an iron one as across open ground.
    let along = |extra: &str| {
        let mut game = round(extra);
        let player = game.player().expect("player");
        let hy = with_tank(&game.world, player, |t| t.move_half_extents(true).1);
        // The row's north face: the tile's own box, widened along the row.
        let face = cell(12, 8).y - OBSTACLE_GRID_SIZE * crate::OBSTACLE_HULL_FRACTION * 0.5;
        let start = Position::new(cell(8, 8).x, face - hy - 0.2);
        game.place_tank(player, start, Some(Dir::Right.rotation())).unwrap();
        drive(&mut game, Some(Dir::Right), 40);
        let bounced = game.events().iter().any(|e| matches!(e, Event::HullStruck { bounced: true, .. }));
        (player_pos(&game).x - start.x, bounced)
    };
    let (open, _) = along("");
    let (bags, _) = along(&row_of("sandbag"));
    let (iron, bounced) = along(&row_of("iron"));
    assert!((bags - open).abs() < 0.5 && (iron - open).abs() < 0.5, "open {open}, sandbags {bags}, iron {iron}");
    assert!(!bounced);
}

#[test]
fn a_fast_ram_cracks_glass_and_a_slower_one_bounces_off_it() {
    let pane_health = |game: &Game| game.world.query::<&Obstacle>().iter().filter(|o| o.material == Material::Glass).map(|o| o.health).fold(f32::MAX, f32::min);
    let mut game = round(&row_of("glass"));
    let full = pane_health(&game);
    let (_, hits) = drive_down(&mut game, 40);
    assert!(hits.iter().any(|e| matches!(e, Event::HullStruck { surface: Surface::Brittle, bounced: false, .. })), "{hits:?}");
    assert!(pane_health(&game) < full, "the pane cracked");

    // Under the crack speed the pane holds and throws the hull back.
    let slow = Tuning { glass_crack_speed: 400.0, ..Tuning::DEFAULT };
    let (hits, health, full) = crate::tuning::in_force(Some(&std::sync::Arc::new(slow)), || {
        let mut game = round(&row_of("glass"));
        let full = pane_health(&game);
        let (_, hits) = drive_down(&mut game, 40);
        (hits, pane_health(&game), full)
    });
    assert!(hits.iter().any(|e| matches!(e, Event::HullStruck { surface: Surface::Brittle, bounced: true, .. })), "{hits:?}");
    assert_eq!(health, full);
}

#[test]
fn a_rammed_lamp_post_snaps_and_a_rammed_range_board_tips_over() {
    for kind in ["lamp", "target"] {
        let mut game = round(&format!("cells.\"12,8\" = {{ kind = \"{kind}\" }}\n"));
        let standing = |game: &Game| game.world.query::<&Obstacle>().iter().filter(|o| !o.destroyed).count();
        assert_eq!(standing(&game), 1);
        drive_down(&mut game, 90);
        assert_eq!(standing(&game), 0, "a hull pushing into a {kind} puts it over");
    }
}

#[test]
fn a_hull_hitting_a_tree_shakes_its_crown() {
    let mut game = round("cells.\"12,8\" = { kind = \"pine\" }\n");
    let tree = (12, 8);
    let t = Tuning::DEFAULT;
    let mut leaned = 0.0_f32;
    let player = game.player().expect("player");
    game.place_tank(player, cell(12, 4), Some(Dir::Down.rotation())).unwrap();
    let mut struck = false;
    for _ in 0..40 {
        drive(&mut game, Some(Dir::Down), 1);
        struck |= game.events().iter().any(|e| matches!(e, Event::HullStruck { material: Some(Material::Pine), surface: Surface::Firm, .. }));
        leaned = leaned.max(game.tree_shake_lean(tree, &t).abs());
    }
    assert!(struck, "the hit on the trunk is reported");
    assert!(leaned > 1.0, "and its crown swings: {leaned} px");
}

#[test]
fn grass_and_bushes_slow_a_hull_a_little() {
    let field = |kind: &str| -> String { (2..=30).flat_map(|c| (2..=20).map(move |r| (c, r))).map(|(c, r)| format!("cells.\"{c},{r}\" = {{ kind = \"{kind}\" }}\n")).collect() };
    let run = |extra: &str| {
        let mut game = round(extra);
        let player = game.player().expect("player");
        let start = cell(4, 10);
        game.place_tank(player, start, Some(Dir::Right.rotation())).unwrap();
        drive(&mut game, Some(Dir::Right), 90);
        player_pos(&game).x - start.x
    };
    let (open, grass, bush) = (run(""), run(&field("tall_grass")), run(&field("bush")));
    let t = Tuning::DEFAULT;
    assert!(grass < open && grass > open * t.grass_pace - 8.0, "open {open}, grass {grass}");
    assert!(bush < grass && bush > open * t.bush_pace - 8.0, "grass {grass}, bush {bush}");
}

