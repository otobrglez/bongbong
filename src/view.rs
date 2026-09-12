//! How the game's bitmap lands on the screen.
//!
//! The game renders one fixed-size bitmap per frame - the battlefield
//! under its HUD bar, `Layout::window_size` - whatever the window or the
//! canvas is. `View` is the uniform scale and the centring offset that put
//! that bitmap on the real screen with its shape kept: the whole
//! battlefield is always visible, letterboxed when the shapes differ. Every
//! pointer read goes back through the same numbers (`to_bitmap`), so the
//! HUD's buttons, the dialogs and the builder hit-test in bitmap pixels and
//! never learn what the window is.
//!
//! The field size is the map's (`MapFile::field_size`) and every player in
//! a match shares it; the view is the one per-device thing, and it is
//! presentation only - nothing in `simulation/` sees it.

use sola_raylib::prelude::*;

/// The bitmap-to-screen mapping for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// The bitmap's size in its own pixels (field plus bar).
    pub bitmap: (f32, f32),
    /// The window's size in logical pixels.
    pub window: (f32, f32),
    /// Screen pixels per bitmap pixel.
    pub scale: f32,
    /// Where the bitmap's top-left lands in the window.
    pub offset: Vector2,
}

impl View {
    /// Fit `bitmap` into `window` with its shape kept, centred: the largest
    /// uniform scale at which the whole bitmap is on screen. A window the
    /// bitmap's own size gives scale 1 and no offset, so a fixed-size
    /// window (and the web canvas, whose box keeps the bitmap's shape)
    /// draws exactly as before.
    pub fn fit(bitmap: (f32, f32), window: (f32, f32)) -> Self {
        let (bw, bh) = (bitmap.0.max(1.0), bitmap.1.max(1.0));
        let (ww, wh) = (window.0.max(1.0), window.1.max(1.0));
        let scale = (ww / bw).min(wh / bh);
        let offset = Vector2::new(((ww - bw * scale) / 2.0).floor(), ((wh - bh * scale) / 2.0).floor());
        View { bitmap: (bw, bh), window: (ww, wh), scale, offset }
    }

    /// The window rectangle the bitmap is drawn into.
    pub fn dest(&self) -> Rectangle {
        Rectangle::new(self.offset.x, self.offset.y, self.bitmap.0 * self.scale, self.bitmap.1 * self.scale)
    }

    /// A window position as a bitmap position. Outside the bitmap the
    /// result is out of range rather than clamped, so a press on a
    /// letterbox bar is not a press on the bitmap's edge.
    pub fn to_bitmap(&self, window: Vector2) -> Vector2 {
        Vector2::new((window.x - self.offset.x) / self.scale, (window.y - self.offset.y) / self.scale)
    }

    /// A bitmap position as a window position.
    pub fn to_window(&self, bitmap: Vector2) -> Vector2 {
        Vector2::new(bitmap.x * self.scale + self.offset.x, bitmap.y * self.scale + self.offset.y)
    }

    /// Whether the window is the bitmap's own size - the blit is then an
    /// identity and the letterbox draws nothing.
    pub fn is_identity(&self) -> bool {
        (self.scale - 1.0).abs() < 1e-6 && self.offset.x.abs() < 0.5 && self.offset.y.abs() < 0.5
    }
}

/// Put the composited frame on screen: clear the window, draw the bitmap
/// into `view.dest()`. Nearest filtering keeps the pixel art's blocks
/// whole where the scale is an integer and sharp elsewhere; the bars are
/// plain black.
pub fn present(rl: &mut RaylibHandle, thread: &RaylibThread, composite: &RenderTexture2D, view: &View) {
    // A render texture reads back bottom-up; a negative source height
    // flips it on the way out.
    let source = Rectangle::new(0.0, 0.0, view.bitmap.0, -view.bitmap.1);
    rl.draw(thread, |mut d| {
        d.clear_background(Color::BLACK);
        d.draw_texture_pro(composite, source, view.dest(), Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
    });
}

#[cfg(test)]
mod view_tests {
    use super::*;

    #[test]
    fn a_window_of_the_bitmaps_size_is_the_identity() {
        let v = View::fit((960.0, 512.0), (960.0, 512.0));
        assert!(v.is_identity());
        assert_eq!(v.to_bitmap(Vector2::new(100.0, 40.0)), Vector2::new(100.0, 40.0));
    }

    #[test]
    fn a_wider_window_letterboxes_on_the_sides_and_keeps_the_shape() {
        // An iPhone 15 in landscape, in points: 852 x 393 for a 960 x 512 bitmap.
        let v = View::fit((960.0, 512.0), (852.0, 393.0));
        let s = 393.0 / 512.0;
        assert!((v.scale - s).abs() < 1e-5);
        assert_eq!(v.offset.y, 0.0);
        assert!((v.offset.x - ((852.0 - 960.0 * s) / 2.0).floor()).abs() < 1e-5);
        let d = v.dest();
        assert!((d.width / d.height - 960.0 / 512.0).abs() < 1e-4);
    }

    #[test]
    fn a_taller_window_letterboxes_top_and_bottom() {
        // A 16:9 monitor for a 15:8 bitmap.
        let v = View::fit((960.0, 512.0), (1920.0, 1080.0));
        assert!((v.scale - 2.0).abs() < 1e-6);
        assert_eq!(v.offset.x, 0.0);
        assert_eq!(v.offset.y, 28.0);
    }

    #[test]
    fn pointer_mapping_round_trips_and_leaves_the_bars_out_of_range() {
        let v = View::fit((960.0, 512.0), (1920.0, 1080.0));
        let p = Vector2::new(300.0, 200.0);
        let back = v.to_bitmap(v.to_window(p));
        assert!((back.x - p.x).abs() < 1e-4 && (back.y - p.y).abs() < 1e-4);
        // The top bar: above the bitmap.
        assert!(v.to_bitmap(Vector2::new(500.0, 10.0)).y < 0.0);
    }
}
