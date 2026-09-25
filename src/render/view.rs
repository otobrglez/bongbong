//! Putting the composited bitmap on the window (`view.rs` owns the
//! mapping itself).

use sola_raylib::prelude::*;

use crate::math::{Color, Rectangle};
use crate::view::View;

/// Put the composited frame on screen: the margins in `backdrop` (the
/// HUD bar's own colour, so the bar and the margins read as one panel), a
/// one-pixel frame around the bitmap, then the bitmap into `view.dest()`.
/// Nearest filtering keeps the pixel art's blocks whole where the scale is
/// an integer and sharp elsewhere. A window the bitmap's own size gets no
/// margins and no frame.
pub fn present(rl: &mut RaylibHandle, thread: &RaylibThread, composite: &RenderTexture2D, view: &View, backdrop: Color) {
    // A render texture reads back bottom-up; a negative source height
    // flips it on the way out.
    let source = Rectangle::new(0.0, 0.0, view.bitmap.0, -view.bitmap.1);
    let dest = view.dest();
    rl.draw(thread, |mut d| {
        d.clear_background(Color::BLACK);
        if !view.is_identity() {
            d.draw_rectangle(0, 0, view.window.0 as i32, view.window.1 as i32, backdrop);
            d.draw_rectangle_lines_ex(
                Rectangle::new(dest.x - 1.0, dest.y - 1.0, dest.width + 2.0, dest.height + 2.0),
                1.0,
                FRAME,
            );
        }
        d.draw_texture_pro(composite, source, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
    });
}

/// The frame around the bitmap when it does not fill the window.
const FRAME: Color = Color::new(62, 62, 66, 255);
