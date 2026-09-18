use raylib::core::game_loop;
use raylib::prelude::*;

fn main() {
    let (rl, thread) = init().size(800, 450).build();

    game_loop::run(rl, thread, 60, move |rl, thread| {
        // INPUT
        if rl.is_key_pressed(KeyboardKey::KEY_Q) {
            rl.request_quit();
        }

        // DRAW / RENDER
        let mut d = rl.begin_drawing(thread);
        d.clear_background(Color::RAYWHITE);

        let text = "Hello NTK 2026!";
        let size = 60;
        let width = d.measure_text(text, size);
        d.draw_text(
            text,
            (800 - width) / 2,
            480 / 2 - size / 2,
            size,
            Color::BLACK,
        );

        d.draw_text("Q or Esc to quit", 12, 12, 20, Color::DARKGRAY);
    });
}
