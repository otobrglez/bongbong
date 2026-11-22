//! The BB-209 props (docs/sandbags-barrels-fences.md "New props"): a tyre
//! stack throws a hull back, a tank trap is the hardest thing on the
//! field, a hay bale is shoved and burns, a wooden crate is shoved and
//! splinters, a concrete barrier hardly moves.

use super::*;
use crate::obstacle::{Surface, NEW_PROPS};
use crate::tuning::Tuning;

const ASSAULT: i32 = 1;

fn cell(c: i32, r: i32) -> Position {
    crate::map::cell_to_world(c, r)
}

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

fn one(kind: &str) -> String {
    format!("cells.\"12,8\" = {{ kind = \"{kind}\" }}\n")
}

fn the(game: &Game, material: Material) -> Option<(Position, bool, f32)> {
    game.world.query::<&Obstacle>().iter().find(|o| o.material == material && !o.destroyed).map(|o| (o.position, o.burning, o.health))
}

/// The seat driven down at the prop from four cells above it: the hits.
fn drive_into(game: &mut Game, frames: usize) -> Vec<Event> {
    let player = game.player().expect("player");
    game.place_tank(player, cell(12, 4), Some(Dir::Down.rotation())).unwrap();
    let mut hits = Vec::new();
    for _ in 0..frames {
        drive(game, Some(Dir::Down), 1);
        hits.extend(game.events().iter().filter(|e| matches!(e, Event::HullStruck { .. })).cloned());
    }
    hits
}

#[test]
fn every_new_prop_is_placed_as_a_cell_and_spawns_its_tile() {
    for m in NEW_PROPS {
        assert!(m.is_prop() && !m.is_wall() && m.sheet() == crate::canvas::Sheet::Props, "{m:?} is a prop on the props sheet");
        let obj = CellObject::prop(m).expect("a cell places it");
        assert_eq!(obj.material(), Some(m));
        let toml = toml::to_string(&obj).expect("serialises");
        let kind = toml.lines().find(|l| l.starts_with("kind")).expect("a kind").split('"').nth(1).expect("quoted").to_string();
        let game = round(&one(&kind));
        assert!(the(&game, m).is_some(), "{kind} spawns a {m:?}");
        assert_eq!(game.world.query::<&Obstacle>().iter().find(|o| o.material == m).map(|o| game.physics.is_loose(o.body)), Some(m.is_loose()), "{m:?}");
    }
    // What the AI sees over has a pass-over chance; what it cannot, none.
    for m in NEW_PROPS {
        assert_eq!(m.blocks_sight(), m.pass_over_chance() == 0.0, "{m:?}");
    }
}

#[test]
fn a_tyre_stack_throws_a_hull_back_and_a_tank_trap_stops_it_hardest() {
    let t = Tuning::DEFAULT;
    for (kind, material, surface) in [("tyres", Material::Tyres, Surface::Springy), ("tank_trap", Material::TankTrap, Surface::Hardest)] {
        let mut game = round(&one(kind));
        let at = the(&game, material).expect("it stands").0;
        let hits = drive_into(&mut game, 60);
        assert!(hits.iter().any(|e| matches!(e, Event::HullStruck { surface: s, bounced: true, .. } if *s == surface)), "{kind}: {hits:?}");
        assert_eq!(the(&game, material).map(|p| p.0), Some(at), "{kind} is fixed");
    }
    assert!(Surface::Springy.bounce(&t) > Surface::Hard.bounce(&t), "tyres throw harder than iron");
    // A fixed prop is a wall to the router; a loose one is not.
    let game = round(&format!("{}{}", one("tank_trap"), "cells.\"20,8\" = { kind = \"hay\" }\n"));
    let (w, h) = game.map.field_size();
    let grid = game.nav_grid(w, h);
    assert!(!grid.usable(cell(12, 8)) && grid.usable(cell(20, 8)));
}

#[test]
fn a_hay_bale_is_shoved_and_a_blast_sets_it_burning_to_ash() {
    let mut game = round(&one("hay"));
    let from = the(&game, Material::Hay).expect("the bale").0;
    drive_into(&mut game, 50);
    let to = the(&game, Material::Hay).expect("still there").0;
    assert!(to.y - from.y > 16.0, "shoved: {from:?} -> {to:?}");
    // A drum beside it: the blast lights it, and it burns out.
    let mut game = round("cells.\"12,8\" = { kind = \"hay\" }\ncells.\"14,8\" = { kind = \"barrel\", drum = \"oil\" }\n");
    game.debug_detonate(cell(14, 8)).unwrap();
    let mut lit = false;
    for _ in 0..30 {
        step(&mut game, Input::default());
        lit |= the(&game, Material::Hay).is_some_and(|b| b.1);
    }
    assert!(lit, "the blast set it burning");
    for _ in 0..60 * 20 {
        step(&mut game, Input::default());
    }
    assert!(the(&game, Material::Hay).is_none(), "and it burnt out");
}

#[test]
fn a_wooden_crate_is_shoved_and_a_shot_or_a_hard_knock_splinters_it() {
    let mut game = round(&one("wood_crate"));
    let from = the(&game, Material::WoodCrate).expect("the crate").0;
    drive_into(&mut game, 40);
    let to = the(&game, Material::WoodCrate).expect("a hull at its own pace only shoves it").0;
    assert!(to.y - from.y > 16.0, "shoved: {from:?} -> {to:?}");
    // Flung at faster than any hull drives, it splinters - no blast.
    let mut game = round(&one("wood_crate"));
    let player = game.player().expect("player");
    game.place_tank(player, cell(12, 6), Some(Dir::Down.rotation())).unwrap();
    let body = with_tank(&game.world, player, |t| t.body).expect("a body");
    let mut blasts = 0;
    for _ in 0..20 {
        game.physics.set_velocity(body, Position::new(0.0, 420.0));
        step(&mut game, Input::default());
        blasts += game.events().iter().filter(|e| matches!(e, Event::Blast { .. })).count();
    }
    assert!(the(&game, Material::WoodCrate).is_none() && blasts == 0, "splintered, nothing went off");
}

#[test]
fn a_concrete_barrier_only_creeps_under_a_sustained_push() {
    let mut game = round(&one("barrier"));
    let from = the(&game, Material::Barrier).expect("the barrier").0;
    drive_into(&mut game, 150);
    let to = the(&game, Material::Barrier).expect("it stands").0;
    let moved = to.y - from.y;
    // Two and a half seconds at a hull's top speed would be 525 px.
    assert!(moved > 0.5 && moved < 60.0, "a hull's push creeps it: {moved} px in two and a half seconds");
    assert!(Material::Barrier.blocks_sight() && Material::Barrier.pass_over_chance() == 0.0, "it stops shots as concrete does");
}
