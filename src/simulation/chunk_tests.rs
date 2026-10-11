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
    let game = sandbox();
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
    let game = sandbox();
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

/// A round on `cells` (map TOML lines) with the player at (5, 11) facing
/// right and `enemies` enemies.
fn field(cells: &str, enemies: usize) -> Game {
    let map = format!("version = 1\ntanks = {enemies}\ncells.\"5,11\" = {{ kind = \"start\" }}\n{cells}");
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.enemy_count_override = Some(enemies);
    game.level_overrides.mission = Some(Mission::Destroy);
    game.map = MapFile::from_toml_str(&map).expect("test map parses");
    game.init(W, H);
    game
}

fn wall(material: &str, cells: &[(i32, i32)]) -> String {
    cells.iter().map(|(c, r)| format!("cells.\"{c},{r}\" = {{ kind = \"wall\", material = \"{material}\" }}\n")).collect()
}

/// Fire a shell every `every` frames for `frames` frames, gathering the
/// events.
fn shell_away(game: &mut Game, frames: usize, every: usize) -> Vec<Event> {
    step(game, face_right());
    let mut events = Vec::new();
    for frame in 0..frames {
        let mut input = Input::default();
        input.seats[0].fire = frame % every == 0;
        step(game, input);
        events.extend(game.events().iter().cloned());
    }
    events
}

fn destroyed(events: &[Event], cell: (i32, i32)) -> bool {
    let at = cell_to_world(cell.0, cell.1);
    events.iter().any(|e| matches!(*e, Event::ObstacleDestroyed { x, y, .. } if (x - at.x).abs() < 1.0 && (y - at.y).abs() < 1.0))
}

#[test]
fn a_breach_throws_spall_into_the_tank_sheltering_behind() {
    let mut game = field(&wall("brick", &[(12, 10), (12, 11), (12, 12)]), 1);
    let behind = cell_to_world(14, 11);
    let mut events = Vec::new();
    for _ in 0..4 {
        // Hold the enemy where it shelters, nose to the wall.
        game.debug_teleport(1, behind, Some(270.0)).expect("enemy in slot 1");
        events.extend(shell_away(&mut game, 30, 30));
    }
    let spall: Vec<f32> = events
        .iter()
        .filter_map(|e| match *e {
            Event::Hit { target: HitTarget::Enemy { .. }, damage, cause: HitCause::Spall, .. } => Some(damage),
            _ => None,
        })
        .collect();
    assert!(!spall.is_empty(), "the enemy behind the wall took spall");
    assert!(spall.iter().all(|&d| d > 0.0 && d <= tuning().spall_damage), "{spall:?}");
}

#[test]
fn a_collapse_brings_down_the_weak_tiles_of_its_run_and_stops_at_a_whole_one() {
    let run = [(12, 9), (12, 10), (12, 11), (12, 12), (12, 13)];
    let mut game = field(&wall("brick", &run), 0);
    // (12, 10) and (12, 12) weakened by earlier fire; (12, 9) and (12, 13)
    // whole.
    for cell in [(12, 10), (12, 12)] {
        let entity = tile_at(&game, cell).expect("tile");
        let mut q = game.world.query_one::<&mut Obstacle>(entity);
        let ch = q.get().expect("obstacle").chunks.as_mut().expect("chunked");
        for i in [0, 1, 2, 3, 4, 5] {
            ch.strike(i, 1000.0, 0.0, 0.0);
        }
    }
    let events = shell_away(&mut game, 240, 40);
    assert!(destroyed(&events, (12, 11)), "the struck tile gave way");
    assert!(destroyed(&events, (12, 10)) && destroyed(&events, (12, 12)), "its weak neighbours came down with it");
    assert!(tile_at(&game, (12, 9)).is_some() && tile_at(&game, (12, 13)).is_some(), "the whole ones stand");
    // A beat apart, not on the same frame.
    let frame_of = |cell: (i32, i32)| {
        let mut f = None;
        for (i, e) in events.iter().enumerate() {
            let at = cell_to_world(cell.0, cell.1);
            if matches!(*e, Event::ObstacleDestroyed { x, y, .. } if (x - at.x).abs() < 1.0 && (y - at.y).abs() < 1.0) {
                f = Some(i);
            }
        }
        f
    };
    assert!(frame_of((12, 10)) > frame_of((12, 11)));
}

#[test]
fn a_fallen_tile_leaves_rubble_that_slows_a_hull() {
    let mut game = field(&wall("brick", &[(12, 11)]), 0);
    let events = shell_away(&mut game, 300, 40);
    assert!(destroyed(&events, (12, 11)));
    assert_ne!(game.rubble.level((12, 11)), crate::chunks::RubbleLevel::Clear, "rubble where it stood");
    assert!(game.rubble.units((13, 11)) > 0, "and some blown out behind it");
    let at = cell_to_world(12, 11);
    let footing = Footing::at(&game.water, &game.lava, &game.craters, game.weather, at, game.time).on_rubble(&game.rubble, at);
    assert!(footing.pace < 1.0, "a hull goes slower over it");
    assert_eq!(Footing::at(&game.water, &game.lava, &game.craters, game.weather, at, game.time).pace, 1.0);
}

#[test]
fn a_shattering_pane_takes_its_cracked_neighbour_and_cracks_a_whole_one() {
    let mut game = field(&wall("glass", &[(12, 10), (12, 11), (12, 12)]), 0);
    // (12, 10) already cracked.
    {
        let entity = tile_at(&game, (12, 10)).expect("pane");
        let mut q = game.world.query_one::<&mut Obstacle>(entity);
        let o = q.get().expect("obstacle");
        o.health = o.max_health * 0.5;
    }
    let events = shell_away(&mut game, 60, 1000);
    assert!(destroyed(&events, (12, 11)), "the struck pane shattered");
    assert!(destroyed(&events, (12, 10)), "the cracked one went with it");
    let whole = tile_at(&game, (12, 12)).expect("the whole one stands");
    let o = game.world.get::<&Obstacle>(whole).expect("obstacle");
    assert!(o.health < o.max_health, "but cracked");
}

/// Fire shells right until the tile at `cell` gives way (dies or stands as
/// a cage), returning how many it took. The aim walks up and down the
/// tile's face from shot to shot: one line of fire only opens a loophole,
/// which every later shell flies through.
fn shells_to_give_way(game: &mut Game, cell: (i32, i32)) -> usize {
    step(game, face_right());
    let mut shots = 0;
    let row_y = cell_to_world(cell.0, cell.1).y;
    for frame in 0..1200 {
        let mut input = Input::default();
        if frame % 40 == 0 {
            let dy = [0.0, -10.0, 10.0, -5.0, 5.0][shots % 5];
            game.debug_teleport(0, Position::new(cell_to_world(5, 11).x, row_y + dy), Some(90.0)).expect("player");
            input.seats[0].fire = true;
            shots += 1;
        }
        step(game, input);
        let gone = match tile_at(game, cell) {
            None => true,
            Some(e) => game.world.get::<&Obstacle>(e).map(|o| o.cage.is_some()).unwrap_or(true),
        };
        if gone {
            return shots;
        }
    }
    panic!("the tile at {cell:?} never gave way");
}

#[test]
fn concrete_takes_more_breaching_than_brick() {
    let brick = shells_to_give_way(&mut field(&wall("brick", &[(12, 11)]), 0), (12, 11));
    let concrete = shells_to_give_way(&mut field(&wall("concrete", &[(12, 11)]), 0), (12, 11));
    assert!(concrete > brick, "concrete {concrete} shells against brick {brick}");
}

#[test]
fn broken_concrete_leaves_rebar_that_stops_a_hull_but_not_a_shot() {
    let mut game = field(&wall("concrete", &[(12, 11)]), 0);
    shells_to_give_way(&mut game, (12, 11));
    let entity = tile_at(&game, (12, 11)).expect("the rebar stands");
    assert_eq!(game.world.get::<&Obstacle>(entity).unwrap().cage, Some(tuning().cage_cut_hits as u8));
    // A shot passes.
    let wall_at = cell_to_world(12, 11);
    let terrain = Terrain::build(&game.world, W, H, &[], &game.water);
    let hit = terrain.sweep(&game.world, game.players(), crate::shell::Owner::Player(0), Position::new(wall_at.x - 40.0, wall_at.y), Position::new(wall_at.x + 40.0, wall_at.y), 2.0);
    assert_eq!(hit.map(|(t, _)| t), None, "a shot goes through the cage");
    // A hull does not.
    for _ in 0..240 {
        step(&mut game, face_right());
    }
    let player = game.tank_snapshots().iter().find(|t| t.is_player).expect("player").position;
    assert!(player.x < wall_at.x - 16.0, "the hull stops at the rebar: {} against {}", player.x, wall_at.x);
}

#[test]
fn a_cage_falls_to_its_heavy_blasts_and_nothing_less() {
    let game = field(&wall("concrete", &[(12, 11)]), 0);
    let entity = tile_at(&game, (12, 11)).expect("tile");
    let mut q = game.world.query_one::<&mut Obstacle>(entity);
    let o = q.get().expect("obstacle");
    let (died, broken) = o.strike(f32::MAX, cell_to_world(12, 11), None);
    assert!(!died && broken != 0, "it gives way into a cage, not to nothing");
    let full = tuning().cage_cut_hits as u8;
    assert_eq!(o.cage, Some(full));
    assert!(!o.damage(1000.0), "a shot's damage does not touch it");
    assert!(!o.cut_cage(tuning().cage_cut_damage * 0.5, false), "a light blast does not cut it");
    assert_eq!(o.cage, Some(full));
    for k in 1..full {
        assert!(!o.cut_cage(tuning().cage_cut_damage, false), "blast {k} bends it");
    }
    assert!(o.cut_cage(tuning().cage_cut_damage, false), "the last heavy blast cuts it down");
    assert!(o.destroyed);
}

#[test]
fn wired_glass_keeps_its_mesh_when_it_shatters() {
    let mut game = field(&wall("glass", &[(12, 11)]), 0);
    for o in game.world.query::<&mut Obstacle>().iter() {
        o.variant = 1;
    }
    let events = shell_away(&mut game, 120, 40);
    assert!(events.iter().any(|e| matches!(e, Event::ChunksBroken { material: Material::Glass, collapsed: true, .. })), "the pane shattered");
    let entity = tile_at(&game, (12, 11)).expect("the mesh stands");
    assert!(game.world.get::<&Obstacle>(entity).unwrap().cage.is_some());
    assert!(!destroyed(&events, (12, 11)));
}

// ----- The AI's reading of a chunked wall (BB-85) -----

#[test]
fn sight_passes_a_loophole_a_shell_fits_and_stops_at_what_stands() {
    let mut game = sandbox();
    let wall = cell_to_world(12, 11);
    let line = |game: &Game, y: f32| {
        let terrain = Terrain::build(&game.world, W, H, &[], &game.water);
        terrain.line_of_sight(Position::new(wall.x - 60.0, y), Position::new(wall.x + 60.0, y))
    };
    assert!(!line(&game, wall.y), "a whole tile blocks sight");
    // One row gone: a slot a chunk tall, 8 px, and a shell 6 px wide fits.
    break_rows(&mut game, (12, 11), &[1]);
    assert!(line(&game, wall.y - 4.0), "through the slot's middle");
    assert!(!line(&game, wall.y - 7.0), "too near its edge for a shell");
    assert!(!line(&game, wall.y + 4.0), "into the row below it");
}

#[test]
fn sight_passes_rebar() {
    let mut game = field(&wall("concrete", &[(12, 11)]), 0);
    shells_to_give_way(&mut game, (12, 11));
    let at = cell_to_world(12, 11);
    let terrain = Terrain::build(&game.world, W, H, &[], &game.water);
    assert!(terrain.line_of_sight(Position::new(at.x - 60.0, at.y), Position::new(at.x + 60.0, at.y)));
}

#[test]
fn a_holed_tile_is_in_the_way_only_while_its_body_stands_across_the_hulls_lane() {
    let mut game = sandbox();
    let wall = cell_to_world(12, 11);
    let right = Position::new(1.0, 0.0);
    let ahead = |game: &Game, y: f32| {
        let terrain = Terrain::build(&game.world, W, H, &[], &game.water);
        terrain.obstacle_ahead(Position::new(wall.x - 40.0, y), right, 60.0, 3.0, 10.0)
    };
    assert!(ahead(&game, wall.y - 12.0).is_some(), "a whole tile is in the way");
    // The top half gone: the body is the bottom half, y 0..16 below the
    // centre. A hull 20 px wide riding 14 px above the centre clears it.
    break_rows(&mut game, (12, 11), &[0, 1]);
    assert!(ahead(&game, wall.y - 14.0).is_none(), "the hull's lane is open");
    let seen = ahead(&game, wall.y).expect("the hull's lane meets the bottom half");
    assert_eq!(seen.center, wall);
    assert_eq!(seen.standing.count_ones(), 8);
}

#[test]
fn a_cage_is_never_a_wall_to_breach() {
    let mut game = field(&wall("concrete", &[(12, 11)]), 0);
    shells_to_give_way(&mut game, (12, 11));
    let at = cell_to_world(12, 11);
    let terrain = Terrain::build(&game.world, W, H, &[], &game.water);
    assert!(terrain.obstacle_ahead(Position::new(at.x - 40.0, at.y), Position::new(1.0, 0.0), 60.0, 3.0, 10.0).is_none());
}

/// Where a sapper's shot from `me` facing right meets the tile at `wall`:
/// the chunk its ray reaches first, walking it out in 1 px steps.
fn sapper_hits(me: Position, seen: &crate::ai::WallAhead, lane: f32) -> Option<usize> {
    let deg = crate::ai::sapper_aim(me, crate::tank::Dir::Right, lane, seen);
    let (s, c) = (deg.to_radians().sin(), deg.to_radians().cos());
    (0..120).map(|k| Position::new(me.x + c * k as f32, me.y + s * k as f32)).find_map(|p| {
        crate::chunks::index_at(seen.center, p).filter(|&i| seen.standing & (1 << i) != 0)
    })
}

#[test]
fn a_sapper_shoots_at_the_wall_still_in_its_way_not_through_its_own_hole() {
    let center = cell_to_world(12, 11);
    let me = Position::new(center.x - 40.0, center.y);
    let whole = crate::ai::WallAhead { material: Material::Brick, burning: false, center, standing: crate::chunks::ALL };
    assert_eq!(crate::ai::sapper_aim(me, crate::tank::Dir::Right, 12.0, &whole), 0.0, "straight at a whole tile");
    // The middle of the near column shot out: straight down the barrel now
    // flies through the hole and past the tile.
    let drilled = whole.standing & !(1 << 4) & !(1 << 8);
    let seen = crate::ai::WallAhead { standing: drilled, ..whole };
    let hit = sapper_hits(me, &seen, 12.0).expect("the shot meets the tile");
    assert!(crate::chunks::quadrant_stands(drilled, crate::chunks::quadrant_of(hit)), "chunk {hit} holds up a standing quadrant");
    assert_eq!(hit % CHUNK_SIDE, 0, "on the near column");
    let max = tuning().enemy_sapper_max_aim_deg;
    assert!(crate::ai::sapper_aim(me, crate::tank::Dir::Right, 12.0, &seen).abs() <= max);
    // Once no quadrant stands across the lane there is nothing to aim at.
    let open = crate::ai::WallAhead { standing: 0, ..whole };
    assert_eq!(crate::ai::sapper_aim(me, crate::tank::Dir::Right, 12.0, &open), 0.0);
}

/// A column of brick across the whole field between an enemy and the
/// player: no way round, so the enemy breaches it.
fn walled_off() -> Game {
    let column: Vec<(i32, i32)> = (0..(H / OBSTACLE_GRID_SIZE) as i32 + 1).map(|r| (12, r)).collect();
    let mut game = field(&wall("brick", &column), 1);
    game.debug_teleport(1, cell_to_world(15, 11), Some(270.0)).expect("enemy in slot 1");
    game
}

#[test]
fn an_enemy_cut_off_from_the_player_goes_to_the_wall_and_breaches_it() {
    let mut game = walled_off();
    let wall_x = cell_to_world(12, 11).x;
    let (mut breached, mut fell) = (false, None);
    for frame in 0..60 * 20 {
        step(&mut game, Input::default());
        breached |= game.world.query::<&crate::ai::Ai>().iter().any(|ai| ai.snapshot().breaching.is_some());
        for e in game.events() {
            if let Event::ObstacleDestroyed { x, .. } = *e
                && (x - wall_x).abs() < 1.0
            {
                fell = fell.or(Some(frame));
            }
        }
        if fell.is_some() {
            break;
        }
    }
    // With no route round, the enemy plans its way through the wall
    // (`ai::Sap`), drives up to it and shoots a tile of it down.
    assert!(breached, "the enemy never started a breach");
    assert!(fell.is_some(), "no tile of the wall fell");
}

/// Hold an enemy on the player's row, three cells east of the wall at
/// (12, 11) and in attack range of the player, facing west, for `frames`
/// frames: whether its shells hit the player.
fn shoots_the_player_through(game: &mut Game, frames: usize) -> bool {
    let at = cell_to_world(15, 11);
    let mut hit = false;
    for _ in 0..frames {
        game.debug_teleport(1, at, Some(270.0)).expect("enemy in slot 1");
        step(game, Input::default());
        hit |= game.events().iter().any(|e| matches!(e, Event::Hit { target: HitTarget::Player { .. }, .. }));
    }
    hit
}

#[test]
fn an_enemy_fires_through_a_loophole_and_not_through_a_whole_wall() {
    let mut whole = field(&wall("brick", &[(12, 11)]), 1);
    assert!(!shoots_the_player_through(&mut whole, 240), "a whole tile shields the player");
    let mut holed = field(&wall("brick", &[(12, 11)]), 1);
    // A slot through the middle of the tile, on the line between them.
    break_rows(&mut holed, (12, 11), &[1, 2]);
    assert!(shoots_the_player_through(&mut holed, 240), "the enemy fires through the loophole");
}
