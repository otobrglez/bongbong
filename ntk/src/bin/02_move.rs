use raylib::core::game_loop;
use raylib::prelude::*;

fn main() {
    let (rl, thread) = init().size(800, 450).build();

    // THE STATE
    let mut position = Vector2::new(350.0, 185.0);
    let size = Vector2::new(100.0, 80.0);
    let speed = 300.0; // pixels per second

    game_loop::run(rl, thread, 60, move |rl, thread| {
        let mut direction = Vector2::zero();
        if rl.is_key_down(KeyboardKey::KEY_RIGHT) {
            direction.x += 1.0;
        }
        if rl.is_key_down(KeyboardKey::KEY_LEFT) {
            direction.x -= 1.0;
        }
        if rl.is_key_down(KeyboardKey::KEY_DOWN) {
            direction.y += 1.0;
        }
        if rl.is_key_down(KeyboardKey::KEY_UP) {
            direction.y -= 1.0;
        }

        // UPDATE - one vector expression: direction * speed * frame time
        position += direction * speed * rl.get_frame_time();

        // DRAW
        let mut d = rl.begin_drawing(thread);
        d.clear_background(Color::RAYWHITE);
        d.draw_rectangle_v(position, size, Color::MAROON);
    });
}
