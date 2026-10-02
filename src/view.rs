//! How the game's bitmap lands on the screen.
//!
//! The game renders one bitmap per frame - the battlefield, under the
//! builder's bar in build mode (`Layout::window_size`) - whatever the
//! window or the canvas is. `View` is the uniform scale and the centring
//! offset that put that bitmap on the real screen with its shape kept: the
//! whole battlefield is always visible, letterboxed when the shapes differ.
//! Every pointer on the world or the builder goes back through the same
//! numbers (`to_bitmap`), so the builder hit-tests in bitmap pixels and
//! never learns what the window is; the chrome over play is laid out on the
//! window itself, in points (`hud::UiFrame`).
//!
//! The field size is the map's (`MapFile::field_size`) and every player in
//! a match shares it; the view is the one per-device thing, and it is
//! presentation only - nothing in `simulation/` sees it. The blit onto the
//! window is `render::view::present`.
//!
//! `Camera` is the other half (docs/large-maps-follow-camera.md §6, §11):
//! which part of the world the field area of the bitmap shows, and how
//! large. The renderer draws the world through it - pass 1 into a scene
//! target one texel per world pixel of the view, pass 2 onto the bitmap -
//! and an arena shows the whole field at its own size (`Camera::whole`),
//! which draws exactly what a renderer with no camera would. A field map
//! is followed instead (`MapClass::Field`): its bitmap is the world this
//! window shows, a bitmap pixel per world pixel, made to the window's shape
//! so it fills it (`FollowFrame`); its view is wherever `follow::Follow`
//! puts it, on the block grid with the rest of its position applied when
//! the frame is presented (`Camera::following`). Presentation only as
//! well; `render::view` builds raylib's cameras from it and presents a
//! followed view.

use crate::framing::{frame, Framing, Screen, Seating, SightBox, ViewRules};
use crate::math::{Rectangle, Vec2};
use crate::Layout;

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
    pub offset: Vec2,
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

    /// `fit` for a bitmap made to the window's shape - a followed field
    /// map's, the world framed into the window less the bar
    /// (`FollowFrame`): the same scale, with the centring offset rounded
    /// to the nearest whole point rather than floored, so the rounding
    /// error of a bitmap that fills the window exactly never shifts it a
    /// whole point off the window's edge. Where the shape was clamped, the
    /// bars left over are centred like `fit`'s.
    pub fn fill(bitmap: (f32, f32), window: (f32, f32)) -> Self {
        let fit = Self::fit(bitmap, window);
        let (bw, bh) = fit.bitmap;
        let (ww, wh) = fit.window;
        let centre = |room: f32| (room / 2.0).round().max(0.0);
        View { offset: Vec2::new(centre(ww - bw * fit.scale), centre(wh - bh * fit.scale)), ..fit }
    }

    /// The bitmap centred in the window at `scale`.
    fn at_scale(bitmap: (f32, f32), window: (f32, f32), scale: f32) -> Self {
        let (bw, bh) = bitmap;
        let (ww, wh) = window;
        let offset = Vec2::new(((ww - bw * scale) / 2.0).floor(), ((wh - bh * scale) / 2.0).floor());
        View { bitmap, window, scale, offset }
    }

    /// The window rectangle the bitmap is drawn into.
    pub fn dest(&self) -> Rectangle {
        Rectangle::new(self.offset.x, self.offset.y, self.bitmap.0 * self.scale, self.bitmap.1 * self.scale)
    }

    /// A window position as a bitmap position. Outside the bitmap the
    /// result is out of range rather than clamped, so a press on a
    /// letterbox bar is not a press on the bitmap's edge.
    pub fn to_bitmap(&self, window: Vec2) -> Vec2 {
        Vec2::new((window.x - self.offset.x) / self.scale, (window.y - self.offset.y) / self.scale)
    }

    /// A bitmap position as a window position.
    pub fn to_window(&self, bitmap: Vec2) -> Vec2 {
        Vec2::new(bitmap.x * self.scale + self.offset.x, bitmap.y * self.scale + self.offset.y)
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

/// How far past the view's edge a draw may be anchored and still reach
/// into it. Every one of the many small things a frame draws - a ground
/// cell, a tile with its shadow and flames, a tree's crown, a tank with
/// its rings, a tuft, a tread mark, a scorch, a hit's burst, a particle's
/// puff - spreads well under this from the point it is culled by. The few
/// big ones (a blast, a mushroom cloud, a flamethrower's jet, a beam) are
/// never culled, and a light is culled by its own radius
/// (`weather::lights_in`).
pub const CULL_MARGIN_PX: f32 = 160.0;

/// What part of the world a frame shows, and how large: the field area of
/// the bitmap (`Layout::field`) shows `size` world pixels from
/// `origin + offset`, at `scale` bitmap pixels to the world pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// The world the camera looks at: the field's size, px.
    pub field: (f32, f32),
    /// The world point at the scene target's top-left corner, on the 2 px
    /// block grid (`pyro::BLOCK`): a texel of the scene target is then a
    /// world pixel and a block of the art never straddles two.
    pub origin: Vec2,
    /// How far past `origin` the view's own corner is, under a block on
    /// each axis: the part of a followed view's position the block grid
    /// cannot hold, applied when the frame is presented (`following`).
    /// Zero for every view that keeps to the grid.
    pub offset: Vec2,
    /// How much of the world the view shows, px.
    pub size: (f32, f32),
    /// Bitmap pixels per world pixel.
    pub scale: f32,
    /// World pixels the scene target holds past the view's far edges, so
    /// the picture can be shifted by `offset` and still cover the view:
    /// one block for a followed view, none otherwise.
    pub margin: f32,
}

impl Camera {
    /// The whole field at its own size: what an arena shows.
    pub fn whole(field: (f32, f32)) -> Self {
        Camera { field, origin: Vec2::new(0.0, 0.0), offset: Vec2::new(0.0, 0.0), size: field, scale: 1.0, margin: 0.0 }
    }

    /// A followed view (docs/large-maps-follow-camera.md §6): `size` world
    /// pixels from the world point `corner`, filling the followed bitmap's
    /// field area at `scale` bitmap pixels per world pixel (`FollowFrame`)
    /// and presented at `device_scale` device pixels per world pixel. The
    /// scene target starts at `corner` floored to the block grid and holds
    /// a block more than the view, so the blocks stay whole in it; what the
    /// floor dropped is the `offset` the presented picture is shifted by,
    /// rounded so the shift is whole device pixels (a rounding that
    /// reaches a whole block moves to the next block instead). A corner
    /// that is not a number is the field's own.
    pub fn following(field: (f32, f32), corner: Vec2, size: (f32, f32), scale: f32, device_scale: f32) -> Self {
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let block = crate::pyro::BLOCK;
        let device = if device_scale.is_finite() && device_scale > 0.0 { device_scale } else { 1.0 };
        let axis = |at: f32| {
            let at = if at.is_finite() { at } else { 0.0 };
            let mut grid = (at / block).floor() * block;
            let mut rest = ((at - grid) * device).round() / device;
            if rest >= block {
                grid += block;
                rest -= block;
            }
            (grid, rest.max(0.0))
        };
        let (x, dx) = axis(corner.x);
        let (y, dy) = axis(corner.y);
        Camera { field, origin: Vec2::new(x, y), offset: Vec2::new(dx, dy), size, scale, margin: block }
    }

    /// Whether this is a followed view (`following`), presented with its
    /// sub-block offset rather than through the bitmap.
    pub fn follows(&self) -> bool {
        self.margin > 0.0
    }

    /// The field magnified `zoom` times around `center`: the view shows
    /// `field / zoom` world pixels, kept inside the field and snapped to
    /// the block grid, and fills the field area of the bitmap. A zoom
    /// under 1 is 1, and a centre that is not a number is the field's.
    /// The dev server's `camera` tool pins one for screenshots.
    pub fn zoomed(field: (f32, f32), center: Vec2, zoom: f32) -> Self {
        let zoom = if zoom.is_finite() { zoom.max(1.0) } else { 1.0 };
        let size = (field.0 / zoom, field.1 / zoom);
        let corner = |center: f32, view: f32, world: f32| {
            let center = if center.is_finite() { center } else { world / 2.0 };
            let at = (center - view / 2.0).clamp(0.0, (world - view).max(0.0));
            (at / crate::pyro::BLOCK).floor() * crate::pyro::BLOCK
        };
        let origin = Vec2::new(corner(center.x, size.0, field.0), corner(center.y, size.1, field.1));
        Camera { field, origin, offset: Vec2::new(0.0, 0.0), size, scale: zoom, margin: 0.0 }
    }

    /// Whether the scene target takes in the whole field, which leaves
    /// nothing to cull.
    pub fn shows_whole_field(&self) -> bool {
        let (w, h) = self.target_size();
        self.origin.x <= 0.0 && self.origin.y <= 0.0 && self.origin.x + w as f32 >= self.field.0 && self.origin.y + h as f32 >= self.field.1
    }

    /// The scene target's size: the view and its margin in whole texels,
    /// one per world pixel - the field's own size for the whole field.
    pub fn target_size(&self) -> (i32, i32) {
        (((self.size.0 + self.margin).ceil() as i32).max(1), ((self.size.1 + self.margin).ceil() as i32).max(1))
    }

    /// The world rectangle the view shows.
    pub fn rect(&self) -> Rectangle {
        Rectangle::new(self.origin.x + self.offset.x, self.origin.y + self.offset.y, self.size.0, self.size.1)
    }

    /// The world rectangle the scene target holds: the view and its
    /// margin, rounded out to whole texels.
    pub fn target_rect(&self) -> Rectangle {
        let (w, h) = self.target_size();
        Rectangle::new(self.origin.x, self.origin.y, w as f32, h as f32)
    }

    /// The part of the field the scene target holds, when that is less
    /// than all of it; `None` for the whole field.
    pub fn part(&self) -> Option<Rectangle> {
        (!self.shows_whole_field()).then(|| self.target_rect())
    }

    /// The world rectangle worth drawing: what the scene target holds
    /// (`part`), grown by `CULL_MARGIN_PX` on every side. `None` for a view
    /// of the whole field, which culls nothing - not even a tank still
    /// rolling in from past its edge.
    pub fn cull(&self) -> Option<Rectangle> {
        self.part().map(|r| Rectangle::new(r.x - CULL_MARGIN_PX, r.y - CULL_MARGIN_PX, r.width + 2.0 * CULL_MARGIN_PX, r.height + 2.0 * CULL_MARGIN_PX))
    }

    /// A point in the field area of the bitmap (`Layout::to_field`'s
    /// answer) as a world point: what a pointer over the field touches.
    pub fn to_world(&self, view: Vec2) -> Vec2 {
        let corner = self.rect();
        Vec2::new(corner.x + view.x / self.scale, corner.y + view.y / self.scale)
    }

    /// A world point as a point in the field area of the bitmap.
    pub fn to_view(&self, world: Vec2) -> Vec2 {
        let corner = self.rect();
        Vec2::new((world.x - corner.x) * self.scale, (world.y - corner.y) * self.scale)
    }

    /// Where the scene target lands in the field area of the bitmap, before
    /// the shake: its every texel `scale` bitmap pixels wide, shifted back
    /// by `offset`. A view whose size is not whole texels, or a target that
    /// holds a margin, reaches a little past the field area, where the
    /// bitmap's edge clips it.
    pub fn dest(&self) -> Rectangle {
        let (w, h) = self.target_size();
        Rectangle::new(-self.offset.x * self.scale, -self.offset.y * self.scale, w as f32 * self.scale, h as f32 * self.scale)
    }

    /// The scene target's texture coordinates in the ripple shaders' frame
    /// (`static/shockwave.fs`): a ripple's ring is measured in field UV - x
    /// across the field, y up from its bottom edge, 0 to 1 - and texture
    /// coordinate `t` of the target is field UV `origin + t * size`, which
    /// is `(0, 0)` and `(1, 1)` for the whole field.
    pub fn field_uv(&self) -> (Vec2, Vec2) {
        let (w, h) = self.target_size();
        let (fw, fh) = (self.field.0.max(1.0), self.field.1.max(1.0));
        let origin = Vec2::new(self.origin.x / fw, 1.0 - (self.origin.y + h as f32) / fh);
        (origin, Vec2::new(w as f32 / fw, h as f32 / fh))
    }
}

/// Whether a draw anchored at `at` can be left out: there is a world
/// rectangle worth drawing (`Camera::cull`) and `at` lies outside it.
pub fn culled(cull: Option<Rectangle>, at: Vec2) -> bool {
    cull.is_some_and(|r| !r.contains(at))
}

/// How a followed field map's frame lands on the window
/// (docs/large-maps-follow-camera.md §3, §6): the world the framing rules
/// give this screen (`framing::frame`), a bitmap of exactly that world, a
/// bitmap pixel per world pixel, and the view that fills the window with it
/// - no letterbox for any shape between `view_aspect_min` and
/// `view_aspect_max`, bars across the long axis past them. Play mode draws
/// no bar over it: the HUD stands in the window's corners
/// (`hud::corners`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FollowFrame {
    /// What this screen shows of a field map.
    pub framing: Framing,
    /// Who the round is framed for, and the sight box every view of it
    /// keeps on screen.
    pub seating: Seating,
    pub sight: SightBox,
    /// The bitmap: the field area the view fills, and no bar.
    pub layout: Layout,
    /// The bitmap onto the window.
    pub view: View,
}

impl FollowFrame {
    /// Frame a field map on `screen` (in points) for a window of `window`
    /// (in the window's own units, which `View` maps to).
    pub fn new(screen: Screen, window: (f32, f32), seating: Seating, sight: SightBox, rules: &ViewRules) -> FollowFrame {
        let framing = frame(screen, seating, sight, rules);
        let (w, h) = framing.visible;
        FollowFrame { framing, seating, sight, layout: Layout::bare(w, h), view: View::fill((w, h), window) }
    }

    /// The scene target a view of this frame needs: the view and the
    /// block of margin `Camera::following` gives it, whole texels. It does
    /// not move with the view, so the target lives as long as the window
    /// keeps its size.
    pub fn target_size(&self) -> (i32, i32) {
        Camera::following((1.0, 1.0), Vec2::new(0.0, 0.0), self.framing.visible, 1.0, 1.0).target_size()
    }

    /// Device pixels per world pixel on a window of `framebuffer` device
    /// pixels per point: what the picture is presented at, and so what a
    /// followed view's sub-block shift is rounded to.
    pub fn device_scale(&self, framebuffer: f32) -> f32 {
        self.view.scale * framebuffer
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;

    #[test]
    fn a_window_of_the_bitmaps_size_is_the_identity() {
        let v = View::fit((960.0, 512.0), (960.0, 512.0));
        assert!(v.is_identity());
        assert_eq!(v.to_bitmap(Vec2::new(100.0, 40.0)), Vec2::new(100.0, 40.0));
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
        assert_eq!(v.offset, Vec2::new(240.0, 156.0));
        assert!(!v.is_identity());
        assert!(v.to_bitmap(Vec2::new(10.0, 10.0)).x < 0.0);
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
        let p = Vec2::new(300.0, 200.0);
        let back = v.to_bitmap(v.to_window(p));
        assert!((back.x - p.x).abs() < 1e-4 && (back.y - p.y).abs() < 1e-4);
        // The top bar: above the bitmap.
        assert!(v.to_bitmap(Vec2::new(500.0, 10.0)).y < 0.0);
    }

    /// Every field size the shipped maps use, from the crossplay studies'
    /// 24 x 12 to grand-campaign's 48 x 24.
    const SHIPPED_FIELDS: [(f32, f32); 7] =
        [(768.0, 384.0), (1088.0, 544.0), (1152.0, 576.0), (1280.0, 640.0), (1280.0, 720.0), (1408.0, 704.0), (1536.0, 768.0)];

    #[test]
    fn the_whole_field_is_the_identity() {
        for field in SHIPPED_FIELDS {
            let c = Camera::whole(field);
            assert!(c.shows_whole_field());
            assert_eq!(c.target_size(), (field.0 as i32, field.1 as i32), "one texel per field pixel");
            assert_eq!(c.rect(), Rectangle::new(0.0, 0.0, field.0, field.1));
            assert_eq!(c.dest(), Rectangle::new(0.0, 0.0, field.0, field.1));
            assert_eq!(c.part(), None);
            assert_eq!(c.cull(), None, "the whole field culls nothing");
            assert_eq!(c.field_uv(), (Vec2::new(0.0, 0.0), Vec2::new(1.0, 1.0)), "{field:?}");
            let p = Vec2::new(123.5, 77.25);
            assert_eq!(c.to_world(p), p);
            assert_eq!(c.to_view(p), p);
        }
    }

    #[test]
    fn a_zoom_shows_part_of_the_field_round_its_centre_and_fills_the_field_area() {
        let field = (1088.0, 544.0);
        let c = Camera::zoomed(field, Vec2::new(544.0, 272.0), 2.0);
        assert_eq!((c.origin, c.size, c.scale), (Vec2::new(272.0, 136.0), (544.0, 272.0), 2.0));
        assert_eq!(c.target_size(), (544, 272));
        assert_eq!(c.dest(), Rectangle::new(0.0, 0.0, 1088.0, 544.0));
        assert!(!c.shows_whole_field());
        assert_eq!(c.part(), Some(c.rect()));
        // The middle of the field area is the point the view centres on;
        // its corner is the view's.
        assert_eq!(c.to_world(Vec2::new(544.0, 272.0)), Vec2::new(544.0, 272.0));
        assert_eq!(c.to_world(Vec2::new(0.0, 0.0)), c.origin);
        let w = Vec2::new(300.0, 200.0);
        assert_eq!(c.to_view(w), Vec2::new(56.0, 128.0));
        assert_eq!(c.to_world(c.to_view(w)), w);
        // Zoom 1 round the middle is the whole field.
        assert_eq!(Camera::zoomed(field, Vec2::new(544.0, 272.0), 1.0), Camera::whole(field));
    }

    #[test]
    fn a_zoomed_view_stays_inside_the_field_and_on_the_block_grid() {
        let field = (1088.0, 544.0);
        assert_eq!(Camera::zoomed(field, Vec2::new(5.0, 5.0), 3.0).origin, Vec2::new(0.0, 0.0), "clamped at the top-left");
        let c = Camera::zoomed(field, Vec2::new(5000.0, 5000.0), 3.0);
        assert!(c.origin.x + c.size.0 <= field.0 && c.origin.y + c.size.1 <= field.1, "clamped at the bottom-right: {c:?}");
        assert!(c.origin.x > field.0 - c.size.0 - 2.0 && c.origin.y > field.1 - c.size.1 - 2.0, "{c:?}");
        let c = Camera::zoomed(field, Vec2::new(501.0, 303.0), 2.0);
        assert_eq!((c.origin.x % 2.0, c.origin.y % 2.0), (0.0, 0.0), "whole blocks: {c:?}");
        // A third of the field is no whole number of texels: the target
        // rounds up and covers the field area.
        let c = Camera::zoomed(field, Vec2::new(544.0, 272.0), 3.0);
        assert_eq!(c.target_size(), (363, 182));
        assert!(c.dest().width >= field.0 && c.dest().height >= field.1);
        assert_eq!(c.part(), Some(Rectangle::new(c.origin.x, c.origin.y, 363.0, 182.0)), "what the target holds");
        // A zoom under one, or not a number, shows the whole field, and a
        // centre that is not a number is the field's.
        assert_eq!(Camera::zoomed(field, Vec2::new(544.0, 272.0), 0.5), Camera::whole(field));
        assert_eq!(Camera::zoomed(field, Vec2::new(f32::NAN, 0.0), 1.0), Camera::whole(field));
        assert_eq!(Camera::zoomed(field, Vec2::new(f32::NAN, f32::NAN), f32::NAN), Camera::whole(field));
        let c = Camera::zoomed(field, Vec2::new(f32::NAN, f32::NAN), 2.0);
        assert_eq!(c.origin, Vec2::new(272.0, 136.0), "the field's own centre");
    }

    #[test]
    fn culling_keeps_the_view_and_a_margin_round_it() {
        let c = Camera::zoomed((1088.0, 544.0), Vec2::new(544.0, 272.0), 2.0);
        let cull = c.cull();
        assert!(cull.is_some());
        assert!(!culled(cull, Vec2::new(272.0, 136.0)), "the view's corner");
        assert!(!culled(cull, Vec2::new(272.0 - CULL_MARGIN_PX + 1.0, 136.0)), "inside the margin");
        assert!(culled(cull, Vec2::new(272.0 - CULL_MARGIN_PX - 1.0, 136.0)), "past it");
        assert!(culled(cull, Vec2::new(816.0 + CULL_MARGIN_PX + 1.0, 408.0)), "past the far corner's margin");
        assert!(!culled(None, Vec2::new(-5000.0, 9000.0)), "no rectangle draws everything");
    }

    #[test]
    fn a_followed_view_snaps_to_whole_blocks_and_presents_the_rest() {
        let field = (1536.0, 768.0);
        // 1.5 device pixels per world pixel: a block is three.
        let c = Camera::following(field, Vec2::new(301.3, 120.9), (1280.0, 688.0), 1.0, 1.5);
        assert_eq!(c.origin, Vec2::new(300.0, 120.0), "the corner floored to the block grid");
        assert!(c.offset.x >= 0.0 && c.offset.x < 2.0 && c.offset.y >= 0.0 && c.offset.y < 2.0, "{c:?}");
        for (offset, exact) in [(c.offset.x, 1.3), (c.offset.y, 0.9)] {
            let device = offset * 1.5;
            assert!((device - device.round()).abs() < 1e-4, "a shift of whole device pixels: {device}");
            assert!((offset - exact).abs() <= 0.5 / 1.5 + 1e-4, "the nearest device pixel to {exact}: {offset}");
        }
        assert!(c.follows() && c.scale == 1.0);
        assert_eq!(c.target_size(), (1282, 690), "the view and a block of margin");
        assert_eq!(c.rect(), Rectangle::new(300.0 + c.offset.x, 120.0 + c.offset.y, 1280.0, 688.0));
        // The target covers the view whatever the shift.
        let t = c.target_rect();
        let r = c.rect();
        assert!(t.x <= r.x && t.y <= r.y && t.x + t.width >= r.x + r.width && t.y + t.height >= r.y + r.height);
        // The pointer mapping goes through the shift.
        let w = Vec2::new(900.0, 400.0);
        let back = c.to_world(c.to_view(w));
        assert!((back.x - w.x).abs() < 1e-3 && (back.y - w.y).abs() < 1e-3);
        assert_eq!(c.to_world(Vec2::new(0.0, 0.0)), Vec2::new(r.x, r.y));
        // A remainder that rounds up to a whole block moves to the next.
        let c = Camera::following(field, Vec2::new(3.9, 0.0), (1280.0, 688.0), 1.0, 1.5);
        assert_eq!((c.origin.x, c.offset.x), (4.0, 0.0));
        // On the grid there is nothing to shift; negative corners (a map
        // shorter than the view) floor the same way.
        let c = Camera::following(field, Vec2::new(64.0, -24.0), (1280.0, 816.0), 1.0, 2.0);
        assert_eq!((c.origin, c.offset), (Vec2::new(64.0, -24.0), Vec2::new(0.0, 0.0)));
        let c = Camera::following(field, Vec2::new(-23.5, 0.0), (1280.0, 688.0), 1.0, 1.0);
        assert_eq!((c.origin.x, c.offset.x), (-24.0, 1.0), "half a pixel rounds to one at one pixel a world pixel");
        // A view magnified into a bitmap wider than it: a pointer's world
        // point goes through the scale too.
        let c = Camera::following(field, Vec2::new(100.0, 50.0), (800.0, 520.0), 1.36, 2.0);
        let w = Vec2::new(400.0, 300.0);
        let back = c.to_world(c.to_view(w));
        assert!((back.x - w.x).abs() < 1e-3 && (back.y - w.y).abs() < 1e-3);
        let p = c.to_view(Vec2::new(c.rect().x + 10.0, c.rect().y));
        assert!((p.x - 13.6).abs() < 1e-4 && p.y == 0.0, "{p:?}");
        assert_eq!(c.target_size(), (802, 522), "the target is world pixels, whatever the bitmap's");
        // Nonsense in, the field's corner out.
        let c = Camera::following(field, Vec2::new(f32::NAN, f32::INFINITY), (1280.0, 688.0), 1.0, f32::NAN);
        assert_eq!((c.origin, c.offset), (Vec2::new(0.0, 0.0), Vec2::new(0.0, 0.0)));
        // A followed view inside the field culls; one past it on both axes
        // holds the whole field and culls nothing.
        assert!(Camera::following(field, Vec2::new(100.0, 40.0), (1280.0, 688.0), 1.0, 1.5).cull().is_some());
        assert_eq!(Camera::following((1280.0, 640.0), Vec2::new(-10.0, -24.0), (1300.0, 688.0), 1.0, 1.0).cull(), None);
    }

    #[test]
    fn a_followed_frame_fills_the_window_with_the_world_alone() {
        use crate::framing::{Seating, SightBox, ViewRules};
        let rules = ViewRules::of(&crate::tuning::Tuning::DEFAULT);
        let sight = SightBox::from_cells(11.5, 7.5);
        let windows = [(852.0, 393.0, 3.0, 460.0), (1180.0, 820.0, 2.0, 264.0), (1920.0, 1080.0, 1.0, 92.0), (1600.0, 900.0, 1.0, 96.0), (1088.0, 576.0, 2.0, 192.0)];
        for (w, h, dpr, ppi) in windows {
            for seating in [Seating::Local, Seating::Room] {
                let screen = Screen::new(w, h, dpr, ppi).with_panel_width(w * dpr);
                let f = FollowFrame::new(screen, (w, h), seating, sight, &rules);
                let (vw, vh) = f.framing.visible;
                // The bitmap is the view, a pixel a world pixel, with no
                // bar over it: the world is the whole window.
                assert_eq!((f.layout.field.w, f.layout.field.h), (vw, vh), "{w} x {h} ({seating:?})");
                assert_eq!((f.layout.field.x, f.layout.field.y), (0.0, 0.0));
                assert_eq!(f.layout.panel.h, 0.0, "no bar in play");
                assert_eq!(f.view.offset, Vec2::new(0.0, 0.0), "{w} x {h}: no letterbox");
                assert!((f.view.scale - f.framing.point_scale).abs() < 1e-4, "{w} x {h}: the world is drawn at the framing's scale");
                assert!((f.device_scale(dpr) - f.framing.point_scale * dpr).abs() < 1e-3);
                let d = f.view.dest();
                assert!((d.width - w).abs() < 0.01 && (d.height - h).abs() < 0.01, "{w} x {h}: {d:?}");
                let (tw, th) = f.target_size();
                assert!(tw as f32 >= vw + 2.0 && th as f32 >= vh + 2.0);
            }
        }
        // Past the aspect clamp a 32:9 screen gets side bars, centred.
        let screen = Screen::new(5120.0, 1440.0, 1.0, 109.0).with_panel_width(5120.0);
        let f = FollowFrame::new(screen, (5120.0, 1440.0), Seating::Room, sight, &rules);
        assert!(f.view.offset.x > 0.0 && f.view.offset.y == 0.0, "{:?}", f.view);
        assert!((f.view.offset.x - f.framing.bars.0).abs() <= 0.5, "{:?} vs {:?}", f.view.offset, f.framing.bars);
    }

    #[test]
    fn fill_rounds_a_rounding_error_away_instead_of_flooring_it_to_a_point() {
        // A bitmap that is the window's own shape to the last bit but one.
        let v = View::fill((1167.8, 563.6), (852.0, 411.2));
        assert_eq!(v.offset, Vec2::new(0.0, 0.0));
        assert_eq!(View::fill((960.0, 512.0), (1920.0, 1080.0)).offset, Vec2::new(0.0, 28.0));
    }

    #[test]
    fn field_uv_puts_the_targets_corners_on_the_views() {
        let field = (1088.0, 544.0);
        let c = Camera::zoomed(field, Vec2::new(700.0, 200.0), 2.0);
        let (o, s) = c.field_uv();
        let at = |t: Vec2| Vec2::new(o.x + t.x * s.x, o.y + t.y * s.y);
        let near = |a: Vec2, b: Vec2| (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6;
        // A render texture reads bottom-up: coordinate (0, 1) is the
        // view's top-left corner and (1, 0) its bottom-right.
        let top_left = Vec2::new(c.origin.x / field.0, 1.0 - c.origin.y / field.1);
        let bottom_right = Vec2::new((c.origin.x + c.size.0) / field.0, 1.0 - (c.origin.y + c.size.1) / field.1);
        assert!(near(at(Vec2::new(0.0, 1.0)), top_left), "{:?} vs {top_left:?}", at(Vec2::new(0.0, 1.0)));
        assert!(near(at(Vec2::new(1.0, 0.0)), bottom_right), "{:?} vs {bottom_right:?}", at(Vec2::new(1.0, 0.0)));
    }
}
