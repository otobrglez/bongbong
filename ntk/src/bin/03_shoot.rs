use raylib::core::game_loop;
use raylib::prelude::*;

struct Shot { position: Vector2, velocity: Vector2, }

fn main() {
    let (rl, thread) = init().size(800, 450).build();

    let mut position = Vector2::new(350.0, 185.0);
    let size = Vector2::new(100.0, 80.0);
    let speed = 300.0;
    let mut facing = Vector2::new(0.0, -1.0);

    let mut shots: Vec<Shot> = Vec::new();
    let shot_speed = 600.0;

    game_loop::run(rl, thread, 60, move |rl, thread| {
        // INPUT
        let mut direction = Vector2::zero();
        if rl.is_key_down(KeyboardKey::KEY_RIGHT) { direction.x += 1.0; }
        if rl.is_key_down(KeyboardKey::KEY_LEFT) { direction.x -= 1.0; }
        if rl.is_key_down(KeyboardKey::KEY_DOWN) { direction.y += 1.0; }
        if rl.is_key_down(KeyboardKey::KEY_UP) { direction.y -= 1.0; }
        if direction != Vector2::zero() { facing = direction.normalized(); }

        if rl.is_key_pressed(KeyboardKey::KEY_SPACE) {
            shots.push(Shot {
                position: position + size / 2.0,
                velocity: facing * shot_speed,
            });
        }

        // UPDATE
        let dt = rl.get_frame_time();
        position += direction * speed * dt;
        for shot in &mut shots { shot.position += shot.velocity * dt; }

        // DRAW
        let mut d = rl.begin_drawing(thread);
        d.clear_background(Color::RAYWHITE);
        d.draw_rectangle_v(position, size, Color::MAROON);
        for shot in &shots {
            d.draw_circle_v(shot.position, 10.0, Color::LIME);
        }
    });
}
