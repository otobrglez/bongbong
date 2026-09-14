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
        Self::at_scale((bw, bh), (ww, wh), (ww / bw).min(wh / bh))
    }

    /// `fit`, but never drawn larger than `cap` says: on a big screen the
    /// shared standard field would otherwise be blown up to a 35 mm tank on
    /// a 1080p monitor while the phone player sees it at 8 mm. The cap is
    /// how the desktop presents the same map at a sane size and leaves the
    /// rest of the window to a backdrop (`present`). It never binds where
    /// the fit is already smaller - a phone, a small window - and never
    /// goes below 1.0, the bitmap's own pixels. `None` is the plain fit.
    pub fn fit_capped(bitmap: (f32, f32), window: (f32, f32), cap: Option<ScaleCap>) -> Self {
        let fit = Self::fit(bitmap, window);
        let Some(cap) = cap else { return fit };
        let mut max = cap.max_scale.max(1.0);
        if cap.snap_half {
            max = (max * 2.0).floor() / 2.0;
        }
        if max >= fit.scale {
            fit
        } else {
            Self::at_scale(fit.bitmap, fit.window, max)
        }
    }

    /// The bitmap centred in the window at `scale`.
    fn at_scale(bitmap: (f32, f32), window: (f32, f32), scale: f32) -> Self {
        let (bw, bh) = bitmap;
        let (ww, wh) = window;
        let offset = Vector2::new(((ww - bw * scale) / 2.0).floor(), ((wh - bh * scale) / 2.0).floor());
        View { bitmap, window, scale, offset }
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

/// The most a bitmap is scaled up by (`View::fit_capped`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScaleCap {
    /// Screen pixels per bitmap pixel at most; anything under 1.0 is 1.0.
    pub max_scale: f32,
    /// Floor the cap to a half step (1.0, 1.5, 2.0 ...) so an art pixel -
    /// two bitmap pixels - is a whole number of screen pixels.
    pub snap_half: bool,
}

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
    fn a_cap_above_the_fit_changes_nothing() {
        let cap = Some(ScaleCap { max_scale: 1.5, snap_half: false });
        assert_eq!(View::fit_capped((960.0, 512.0), (852.0, 393.0), cap), View::fit((960.0, 512.0), (852.0, 393.0)));
        assert_eq!(View::fit_capped((960.0, 512.0), (960.0, 512.0), cap), View::fit((960.0, 512.0), (960.0, 512.0)));
        assert_eq!(View::fit_capped((960.0, 512.0), (1920.0, 1080.0), None), View::fit((960.0, 512.0), (1920.0, 1080.0)));
    }

    #[test]
    fn a_cap_below_the_fit_centres_the_capped_bitmap() {
        let cap = Some(ScaleCap { max_scale: 1.5, snap_half: false });
        let v = View::fit_capped((960.0, 512.0), (1920.0, 1080.0), cap);
        assert_eq!(v.scale, 1.5);
        assert_eq!(v.offset, Vector2::new(240.0, 156.0));
        assert!(!v.is_identity());
        assert!(v.to_bitmap(Vector2::new(10.0, 10.0)).x < 0.0);
        let d = v.dest();
        assert_eq!((d.width, d.height), (1440.0, 768.0));
    }

    #[test]
    fn snap_floors_the_cap_to_a_half_step_but_never_under_one() {
        let at = |max_scale, snap_half| View::fit_capped((960.0, 512.0), (2560.0, 1440.0), Some(ScaleCap { max_scale, snap_half })).scale;
        assert_eq!(at(1.34, true), 1.0);
        assert_eq!(at(1.7, true), 1.5);
        assert_eq!(at(1.7, false), 1.7);
        assert_eq!(at(0.5, false), 1.0);
        // A bogus cap on a phone window is still the phone's fit.
        let phone = View::fit_capped((960.0, 512.0), (852.0, 393.0), Some(ScaleCap { max_scale: 0.5, snap_half: true }));
        assert_eq!(phone, View::fit((960.0, 512.0), (852.0, 393.0)));
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
