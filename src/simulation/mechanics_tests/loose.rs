//! Loose barrels and shovable wrecks (docs/sandbags-barrels-fences.md
//! "Loose barrels", docs/physics-engine-design.md "Loose objects"): a hull
//! shoves a drum, a wall throws it back, a blast throws it, a hard enough
//! knock sets it off, the router drives through it, and a wreck is pushed
//! into place and stays.

use super::*;
use crate::tuning::Tuning;

const ASSAULT: i32 = 1;

fn cell(c: i32, r: i32) -> Position {
    crate::map::cell_to_world(c, r)
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

fn barrel_at(c: i32, r: i32) -> String {
    format!("cells.\"{c},{r}\" = {{ kind = \"barrel\", drum = \"oil\" }}\n")
}

/// The live barrels: where each stands, by the cell it was placed in.
fn barrels(game: &Game) -> Vec<((i32, i32), Position)> {
    let mut out: Vec<_> = game.world.query::<&Obstacle>().iter().filter(|o| o.material == Material::Barrel && !o.destroyed).map(|o| (o.home, o.position)).collect();
    out.sort_by_key(|b| b.0);
    out
}

fn barrel_body(game: &Game, home: (i32, i32)) -> rapier2d::prelude::RigidBodyHandle {
    game.world.query::<&Obstacle>().iter().find(|o| o.home == home).map(|o| o.body).expect("the barrel stands")
}

fn blasts(game: &Game) -> usize {
    game.events().iter().filter(|e| matches!(e, Event::Blast { .. })).count()
}

/// Steps with nobody driving: how many blasts went off.
fn idle(game: &mut Game, frames: usize) -> usize {
    (0..frames).map(|_| {
        step(game, Input::default());
        blasts(game)
    }).sum()
}

#[test]
fn a_hull_shoves_a_barrel_across_open_ground_and_it_comes_to_rest() {
    let mut game = round(&barrel_at(12, 8));
    let player = game.player().expect("player");
    game.place_tank(player, cell(12, 5), Some(Dir::Down.rotation())).unwrap();
    let start = barrels(&game)[0].1;
    let mut went_off = 0;
    for _ in 0..50 {
        drive(&mut game, Some(Dir::Down), 1);
        went_off += blasts(&game);
    }
    went_off += idle(&mut game, 120);
    let after = barrels(&game);
    assert_eq!(went_off, 0, "a hull at its own top speed shoves a drum, it does not set it off");
    assert_eq!(after.len(), 1);
    let moved = after[0].1 - start;
    assert!(moved.y > 48.0 && moved.x.abs() < 16.0, "shoved on down the field: {moved:?}");
    let rest = after[0].1;
    idle(&mut game, 30);
    assert!(barrels(&game)[0].1.distance_to(rest) < 0.5, "and it comes to rest by itself");
    assert_eq!(game.world.query::<&Obstacle>().iter().find(|o| o.material == Material::Barrel).map(|o| o.home), Some((12, 8)), "known by the cell it was placed in");
}

#[test]
fn a_barrel_bounces_off_a_wall_and_one_thrown_hard_at_it_goes_off() {
    let iron: String = (8..=16).map(|c| format!("cells.\"{c},12\" = {{ kind = \"wall\", material = \"iron\" }}\n")).collect();
    let thrown = |speed: f32| {
        let mut game = round(&format!("{iron}{}", barrel_at(12, 10)));
        let body = barrel_body(&game, (12, 10));
        game.physics.set_velocity(body, Position::new(0.0, speed));
        let mut went_off = 0;
        let mut back = false;
        for _ in 0..40 {
            step(&mut game, Input::default());
            went_off += blasts(&game);
            if !game.world.query::<&Obstacle>().iter().any(|o| o.body == body && !o.destroyed) {
                break;
            }
            back |= game.physics.velocity(body).y < -20.0;
        }
        (went_off, back)
    };
    let (went_off, back) = thrown(180.0);
    assert!(back && went_off == 0, "a drum rolled at a wall bounces off it: bounced {back}, {went_off} blasts");
    let (went_off, _) = thrown(420.0);
    assert!(went_off >= 1, "one thrown hard at it goes off");
}

#[test]
fn a_hull_flung_into_a_barrel_sets_it_off() {
    let mut game = round(&barrel_at(12, 8));
    let player = game.player().expect("player");
    game.place_tank(player, cell(12, 6), Some(Dir::Down.rotation())).unwrap();
    let body = with_tank(&game.world, player, |t| t.body).expect("a body");
    let mut went_off = 0;
    for _ in 0..20 {
        game.physics.set_velocity(body, Position::new(0.0, 340.0));
        step(&mut game, Input::default());
        went_off += blasts(&game);
    }
    assert!(went_off >= 1, "a hull going faster than any drives rams a drum off");
}

#[test]
fn a_blast_throws_the_barrels_round_it() {
    let t = Tuning::DEFAULT;
    // One drum to set off, one past its blast's radius but inside its
    // throw, one well out of both.
    let mut game = round(&format!("{}{}{}", barrel_at(10, 10), barrel_at(15, 10), barrel_at(26, 10)));
    let before = barrels(&game);
    assert!(cell(15, 10).distance_to(cell(10, 10)) > t.barrel_blast_radius && cell(15, 10).distance_to(cell(10, 10)) < t.barrel_blast_radius * t.barrel_throw_reach);
    game.debug_detonate(cell(10, 10)).unwrap();
    idle(&mut game, 60);
    let after = barrels(&game);
    let of = |list: &[((i32, i32), Position)], home: (i32, i32)| list.iter().find(|b| b.0 == home).map(|b| b.1);
    assert!(of(&after, (10, 10)).is_none(), "the drum set off is gone");
    let thrown = of(&after, (15, 10)).expect("the one past the radius is thrown, not lit") - of(&before, (15, 10)).unwrap();
    assert!(thrown.x > 8.0, "thrown away from the blast: {thrown:?}");
    assert_eq!(of(&after, (26, 10)), of(&before, (26, 10)), "the far one does not move");
}

#[test]
fn a_barrel_is_no_wall_to_the_router() {
    let mut game = round(&barrel_at(12, 8));
    let (w, h) = game.map.field_size();
    let grid = game.nav_grid(w, h);
    let (c, r, size) = grid.dims();
    let at = cell(12, 8);
    let (gc, gr) = (((at.x / size) as usize).min(c - 1), ((at.y / size) as usize).min(r - 1));
    assert!(!grid.is_blocked(gc, gr), "the drum's cell is open: a hull shoves it out of the way");
    // Shoved elsewhere, it leaves no wall behind and is none where it is.
    let body = barrel_body(&game, (12, 8));
    game.physics.set_velocity(body, Position::new(150.0, 0.0));
    idle(&mut game, 60);
    let grid = game.nav_grid(w, h);
    assert!(!grid.is_blocked(gc, gr));
}

#[test]
fn a_wreck_is_pushed_into_place_and_stays_and_a_blast_moves_it() {
    // A second seat wrecked on the field, pushed by the first.
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.players = PlayerCount::from_count(2).expect("two seats");
    game.player_row_override = Some(ASSAULT);
    game.player2_row_override = Some(ASSAULT);
    game.level_overrides.mission = Some(Mission::Destroy);
    game.map = MapFile::from_toml_str("version = 1\ntanks = 0\ncells.\"1,1\" = { kind = \"start\" }\n").expect("test map parses");
    game.init(W, H);
    step(&mut game, Input::default());
    let (a, b) = (game.seats[0].expect("seat 0"), game.seats[1].expect("seat 1"));
    let at = cell(14, 12);
    game.place_tank(b, at, Some(0.0)).unwrap();
    game.debug_kill(1).unwrap();
    step(&mut game, Input::default());
    game.place_tank(a, Position::new(at.x, at.y + 70.0), Some(Dir::Up.rotation())).unwrap();
    drive(&mut game, Some(Dir::Up), 120);
    let pushed = with_tank(&game.world, b, |t| t.position);
    assert!(at.y - pushed.y > 40.0, "a hull shoves a wreck of its weight along: {} px", at.y - pushed.y);
    drive(&mut game, None, 60);
    let rest = with_tank(&game.world, b, |t| t.position);
    assert!(pushed.distance_to(rest) < 12.0, "let go, it slides a few px and stops: {} px", pushed.distance_to(rest));
    idle(&mut game, 30);
    assert!(rest.distance_to(with_tank(&game.world, b, |t| t.position)) < 0.5, "and stays where it was put");
}
