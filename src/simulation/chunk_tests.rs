//! Headless scenarios for chunked walls (`chunks.rs`, BB-81): a shot
//! breaks the chunk it strikes, a hole lets shots through while it still
//! stops a hull, a tile gives way once too few chunks stand, and a minigun
//! drills rather than fells.

use super::hits::Terrain;
use super::*;
use crate::chunks::CHUNK_SIDE;
use crate::map::cell_to_world;
use crate::obstacle::Material;

const W: f32 = 1280.0;
const H: f32 = 720.0;

/// The player starts at (5, 11) facing right, a brick tile at (12, 11) in
/// its way and another at (20, 11) behind it, with no enemies.
fn sandbox() -> Game {
    let map = "version = 1\ntanks = 0\ncells.\"5,11\" = { kind = \"start\" }\n\
               cells.\"12,11\" = { kind = \"wall\", material = \"brick\" }\n\
               cells.\"20,11\" = { kind = \"wall\", material = \"brick\" }\n";
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.level_overrides.mission = Some(Mission::Destroy);
    game.map = MapFile::from_toml_str(map).expect("test map parses");
    game.init(W, H);
    game
}

fn step(game: &mut Game, input: Input) {
    game.update(input, 1.0 / 60.0, W, H);
}

fn tile_at(game: &Game, cell: (i32, i32)) -> Option<Entity> {
    game.world.query::<(Entity, &Obstacle)>().iter().find(|(_, o)| o.cell() == cell && !o.destroyed).map(|(e, _)| e)
}

/// Break whole rows of the tile at `cell` by hand, as a run of shots would
/// have, and fit its body to what stands.
fn break_rows(game: &mut Game, cell: (i32, i32), rows: &[usize]) {
    let entity = tile_at(game, cell).expect("tile");
    {
        let mut q = game.world.query_one::<&mut Obstacle>(entity);
        let o = q.get().expect("obstacle");
        let ch = o.chunks.as_mut().expect("brick breaks chunk by chunk");
        for &r in rows {
            for c in 0..CHUNK_SIDE {
                ch.strike(r * CHUNK_SIDE + c, 1000.0, 0.0, 0.0);
            }
        }
    }
    game.fit_tile_body(entity);
}

fn face_right() -> Input {
    let mut input = Input::default();
    input.seats[0].move_dir = Some(crate::tank::Dir::Right);
    input
}

#[test]
fn a_shot_passes_a_hole_and_hits_what_stands() {
    let mut game = sandbox();
    let wall = cell_to_world(12, 11);
    // The middle two rows gone: a slot from 8 to 24 px down the tile.
    break_rows(&mut game, (12, 11), &[1, 2]);
    let terrain = Terrain::build(&game.world, W, H, &[], &game.water);
    let players = game.players();
    let across = |y: f32| {
        terrain.sweep(&game.world, players, crate::shell::Owner::Player(0), Position::new(wall.x - 40.0, y), Position::new(wall.x + 40.0, y), 2.0)
    };
    assert_eq!(across(wall.y).map(|(t, _)| t), None, "through the slot");
    let entity = tile_at(&game, (12, 11)).expect("tile");
    assert_eq!(across(wall.y - 10.0).map(|(t, _)| t), Some(hits::ShellTarget::Obstacle(entity)), "into the top row");
    assert_eq!(across(wall.y + 10.0).map(|(t, _)| t), Some(hits::ShellTarget::Obstacle(entity)), "into the bottom row");
}

#[test]
fn a_hole_a_shot_passes_still_stops_a_hull() {
    let mut game = sandbox();
    break_rows(&mut game, (12, 11), &[1, 2]);
    for _ in 0..240 {
        step(&mut game, face_right());
    }
    let player = game.tank_snapshots().iter().find(|t| t.is_player).expect("player").position;
    let wall = cell_to_world(12, 11);
    assert!(player.x < wall.x - 16.0, "the hull stopped at the wall: {} against {}", player.x, wall.x);
    assert!(tile_at(&game, (12, 11)).is_some());
}

#[test]
fn shells_break_a_brick_tile_chunk_by_chunk_until_it_gives_way() {
    let mut game = sandbox();
    // Face the wall, then fire until it goes.
    step(&mut game, face_right());
    let mut shots = 0;
    let mut broke = Vec::new();
    let mut destroyed = false;
    for frame in 0..900 {
        let fire = frame % 45 == 0;
        let mut input = Input::default();
        if fire {
            input.seats[0].fire = true;
            shots += 1;
        }
        step(&mut game, input);
        for e in game.events() {
            match *e {
                Event::ChunksBroken { material: Material::Brick, x, broken, collapsed, .. } if (x - cell_to_world(12, 11).x).abs() < 1.0 => {
                    broke.push((broken, collapsed));
                }
                Event::ObstacleDestroyed { material: Material::Brick, x, .. } if (x - cell_to_world(12, 11).x).abs() < 1.0 => destroyed = true,
                _ => {}
            }
        }
        if destroyed {
            break;
        }
    }
    assert!(destroyed, "the tile gave way");
    assert!(broke.len() >= 2, "it broke in more than one blow: {broke:?}");
    assert!(broke[..broke.len() - 1].iter().all(|&(_, c)| !c), "only the last blow collapses it");
    assert!(broke.last().is_some_and(|&(_, c)| c), "the collapse drops what stood");
    assert!(shots <= 5, "a brick tile still falls to a handful of shells: {shots}");
}

#[test]
fn a_minigun_drills_a_brick_tile_rather_than_felling_it() {
    let mut game = sandbox();
    let entity = tile_at(&game, (12, 11)).expect("tile");
    let wall = cell_to_world(12, 11);
    let mut q = game.world.query_one::<&mut Obstacle>(entity);
    let o = q.get().expect("obstacle");
    // Minigun rounds (3-6) at the middle of the near face, one after
    // another down the same line.
    let mut rounds = 0;
    loop {
        rounds += 1;
        let (died, _) = o.strike(4.5, Position::new(wall.x - 16.0, wall.y + 2.0), Some(Vec2::new(1.0, 0.0)));
        if died || rounds > 200 {
            break;
        }
    }
    assert!((15..=30).contains(&rounds), "about twenty rounds to fell a brick tile: {rounds}");
}

#[test]
fn a_struck_chunk_is_the_first_one_standing_on_the_shots_way() {
    let mut game = sandbox();
    let entity = tile_at(&game, (12, 11)).expect("tile");
    let wall = cell_to_world(12, 11);
    let mut q = game.world.query_one::<&mut Obstacle>(entity);
    let o = q.get().expect("obstacle");
    let (_, broken) = o.strike(30.0, Position::new(wall.x - 16.0, wall.y - 4.0), Some(Vec2::new(1.0, 0.0)));
    // Row 1 from the left: chunk 4 and its side neighbours 0, 5 and 8.
    assert_eq!(broken & 0b1_0011_0001, 0b1_0011_0001, "{broken:016b}");
    assert_eq!(broken & (1 << 3), 0, "the far corner stands");
}

#[test]
fn the_body_shrinks_to_the_half_of_a_tile_that_stands() {
    let stop = |broken: &[usize]| {
        let mut game = sandbox();
        let entity = tile_at(&game, (12, 11)).expect("tile");
        {
            let mut q = game.world.query_one::<&mut Obstacle>(entity);
            let ch = q.get().expect("obstacle").chunks.as_mut().expect("chunked");
            for &i in broken {
                ch.strike(i, 1000.0, 0.0, 0.0);
            }
        }
        game.fit_tile_body(entity);
        for _ in 0..240 {
            step(&mut game, face_right());
        }
        game.tank_snapshots().iter().find(|t| t.is_player).expect("player").position.x
    };
    let whole = stop(&[]);
    // The two left columns of chunks gone: the left quadrants fall, and the
    // hull drives on to the half that stands.
    let half = stop(&[0, 1, 4, 5, 8, 9, 12, 13]);
    assert!(half > whole + 8.0, "the hull stops at what stands: {half} against {whole}");
}
