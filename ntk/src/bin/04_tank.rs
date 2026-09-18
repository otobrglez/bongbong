use raylib::core::game_loop;
use raylib::prelude::*;

struct Shot { position: Vector2, velocity: Vector2 }

const FRAME: f32 = 32.0;
const FRAMES: usize = 4;
const SCALE: f32 = 3.0;
const SPRITE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/tank.png");

fn main() {
    let (mut rl, thread) = init().size(800, 450).build();

    // LOAD TEXTURE OF TANK
    let tank = rl.load_texture(&thread, SPRITE).expect("tank.png");

    let mut position = Vector2::new(400.0, 225.0);
    let mut facing = Vector2::new(0.0, -1.0);
    let speed = 200.0;

    // Track animation: which frame we show and how far into it we are.
    let mut frame = 0usize;
    let mut frame_clock = 0.0_f32;
    let frame_seconds = 0.08;
    let mut shots: Vec<Shot> = Vec::new();
    let shot_speed = 600.0;

    game_loop::run(rl, thread, 60, move |rl, thread| {
        // INPUT
        let mut direction = Vector2::zero();
        if rl.is_key_down(KeyboardKey::KEY_RIGHT) { direction.x += 1.0; }
        if rl.is_key_down(KeyboardKey::KEY_LEFT) { direction.x -= 1.0; }
        if rl.is_key_down(KeyboardKey::KEY_DOWN) { direction.y += 1.0; }
        if rl.is_key_down(KeyboardKey::KEY_UP) { direction.y -= 1.0; }
        let moving = direction != Vector2::zero();
        if moving { facing = direction.normalized(); }
        if rl.is_key_pressed(KeyboardKey::KEY_SPACE) {
            shots.push(Shot {
                position: position + facing * (FRAME * SCALE / 2.0),
                velocity: facing * shot_speed,
            });
        }

        // UPDATE
        let dt = rl.get_frame_time();
        position += direction * speed * dt;
        if moving {
            frame_clock += dt;
            while frame_clock >= frame_seconds {
                frame_clock -= frame_seconds;
                frame = (frame + 1) % FRAMES;
            }
        }
        for shot in &mut shots { shot.position += shot.velocity * dt; }

        // DRAW
        let mut d = rl.begin_drawing(thread);
        d.clear_background(Color::RAYWHITE);

        // The sprite faces up; rotate it to the facing (degrees, clockwise, y is down).
        let rotation = facing.x.atan2(-facing.y).to_degrees();
        let source = Rectangle::new(frame as f32 * FRAME, 0.0, FRAME, FRAME);
        let dest = Rectangle::new(position.x, position.y, FRAME * SCALE, FRAME * SCALE);
        let origin = Vector2::new(FRAME * SCALE / 2.0, FRAME * SCALE / 2.0);
        d.draw_texture_pro(&tank, source, dest, origin, rotation, Color::WHITE);

        for shot in &shots {
            d.draw_circle_v(shot.position, 8.0, Color::LIME);
        }
    });
}
