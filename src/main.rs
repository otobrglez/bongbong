use sola_raylib::prelude::*;

type Position = Vector2;

#[derive(Default)]
struct Game {
    grid: Grid,
}

#[derive(Default)]
struct Grid {
    start: Position,
    tile_size: i32,
    tiles: Vec<Tile>,
}

struct Tile {
    x: usize,
    y: usize,
    color: Color,
}

fn main() {
    let (mut rl, thread) = sola_raylib::init().size(640, 480).title("bongbong").build();
    rl.set_target_fps(60);

    let mut game = Game::default();
    game_init(&mut game, &rl);

    while !rl.window_should_close() {
        game_update(&mut game, &rl);
        game_render(&game, &mut rl, &thread);
    }
}

static MAX_TILES: i32 = 10;

fn game_init(game: &mut Game, rl: &RaylibHandle) {
    let (width, height) = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);

    let mut tiles: Vec<Tile> = vec![];
    for x in 0..MAX_TILES {
        for y in 0..MAX_TILES {
            println!("(x,y) = {:?}, {:?}", x, y);

            let tile = Tile {
                x: x as usize,
                y: y as usize,
                color: Color::BLUEVIOLET,
            };
            tiles.push(tile);
        }
    }

    let t_size: i32 = ((height - 50.0) / MAX_TILES as f32) as i32;
    println!("Tile size = {:?}", t_size);

    let start_x = (width / 2.0) - (t_size as f32 * MAX_TILES as f32 / 2.0);
    let start_y = (height / 2.0) - (t_size as f32 * MAX_TILES as f32 / 2.0);

    println!("Start X,Y = {:?},{:?}", start_x, start_y);

    game.grid.start = Vector2::new(start_x, start_y);
    game.grid.tile_size = t_size;
    game.grid.tiles = tiles;
}

fn game_update(game: &mut Game, rl: &RaylibHandle) {
    /*

    if rl.is_key_down(KeyboardKey::KEY_LEFT) {
    }
    if rl.is_key_down(KeyboardKey::KEY_RIGHT) {
    }
    if rl.is_key_down(KeyboardKey::KEY_UP) {
    }
    if rl.is_key_down(KeyboardKey::KEY_DOWN) {
    }
    */
}

fn game_render(game: &Game, rl: &mut RaylibHandle, thread: &RaylibThread) {
    let (width, height) = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);
    rl.draw(thread, |mut d| {
        d.clear_background(Color::RAYWHITE);

        // GRID
        /*
        d.draw_line_v(
            Vector2::new(0.0, 0.0),
            Vector2::new(width, height),
            Color::BROWN,
        );
        d.draw_line_v(
            Vector2::new(width, 0.0),
            Vector2::new(0.0, height),
            Color::BROWN,
        );
        */
        d.draw_pixel_v(game.grid.start, Color::ORANGE);

        d.draw_circle_v(game.grid.start, 5.0, Color::PURPLE);

        for x in 0..MAX_TILES {
            for y in 0..MAX_TILES {
                let t = game
                    .grid
                    .tiles
                    .iter()
                    .find(|t| t.x == x as usize && t.y == y as usize);
                dbg!(t);
            }
        }
    });
}
