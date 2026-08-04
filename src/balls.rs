// main.rs
use sola_raylib::prelude::*;

type Position = Vector2;

struct Game {
    ball: Ball,
    letters: Vec<Letter>,
}

impl Default for Game {
    fn default() -> Game {
        Game {
            ball: Ball::default(),
            letters: vec![],
        }
    }
}

#[derive(Default)]
struct Ball {
    moving: bool,
    size: f32,
    position: Position,
}
impl Ball {}

struct Letter {
    letter: char,
    position: Position,
}
impl Letter {
    fn new(letter: char, position: Position) -> Letter {
        Letter { letter, position }
    }
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

fn game_init(game: &mut Game, rl: &RaylibHandle) {
    let (width, height) = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);
    game.ball.position = Vector2::new(width / 2.0, height / 2.0);

    for c in "DEMOOTO".chars() {
        let (x, y) = (
            rand::random_range(20.0..=width - 20.0),
            rand::random_range(20.0..=height - 20.0),
        );
        game.letters.push(Letter::new(c, Vector2::new(x, y)))
    }
}

fn game_update(game: &mut Game, rl: &RaylibHandle) {
    game.ball.moving = false;
    if rl.is_key_down(KeyboardKey::KEY_LEFT) {
        game.ball.moving = true;
        game.ball.position.x -= 2.0
    }
    if rl.is_key_down(KeyboardKey::KEY_RIGHT) {
        game.ball.moving = true;
        game.ball.position.x += 2.0
    }
    if rl.is_key_down(KeyboardKey::KEY_UP) {
        game.ball.moving = true;
        game.ball.position.y -= 2.0
    }
    if rl.is_key_down(KeyboardKey::KEY_DOWN) {
        game.ball.moving = true;
        game.ball.position.y += 2.0
    }

    let font_size = 50.0;
    game.letters.retain(|l| {
        let text = l.letter.to_string();
        let width = rl.measure_text(&text, font_size as i32) as f32;

        let center = Vector2::new(l.position.x + width / 2.0, l.position.y + font_size / 2.0);

        if hit_center(&game.ball, center, 10.0) {
            game.ball.size += 2.0;
            false
        } else {
            true
        }
    });
}

fn hit_center(ball: &Ball, center: Position, tolerance: f32) -> bool {
    let dx = ball.position.x - center.x;
    let dy = ball.position.y - center.y;
    dx * dx + dy * dy <= tolerance * tolerance
}
fn game_render(game: &Game, rl: &mut RaylibHandle, thread: &RaylibThread) {
    rl.draw(thread, |mut d| {
        d.clear_background(Color::RAYWHITE);

        // Draw ball.
        if game.ball.moving {
            d.draw_circle_v(game.ball.position, 10.0 + game.ball.size, Color::RED);
        } else {
            d.draw_circle_v(game.ball.position, 10.0 + game.ball.size, Color::PINK);
        }

        // Draw chars
        for l in game.letters.iter() {
            d.draw_text(
                &l.letter.to_string(),
                l.position.x as i32,
                l.position.y as i32,
                50,
                Color::GRAY,
            );
        }
    });
}
