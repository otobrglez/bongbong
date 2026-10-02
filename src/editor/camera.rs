//! The builder's own camera (docs/large-maps-follow-camera.md §9): how much
//! of the canvas the field area of the bitmap shows, and how large -
//! independent of play's, headless and pure.
//!
//! - **FIT** shows the whole map. On an arena the bitmap is the field and
//!   the bar (`Layout::for_field`, as the round draws it), so FIT is the
//!   field at its own size - `Camera::whole`, the very picture the builder
//!   drew before it had a camera. A field map's bitmap is made to the
//!   window's shape instead (`canvas_frame`), its bar at the size an
//!   arena's has in the same window, and FIT shrinks the map into the
//!   area under it, centred.
//! - **Zoom and pan** go from FIT to a cell of `builder_zoom_max_cell_pt`
//!   points. The view never leaves the field on an axis the map fills,
//!   and centres it on an axis it does not. Where the screen is coarse
//!   (under `view_fine_ppi`, `CanvasScreen::coarse`) a zoom step lands on a
//!   whole-block scale - a multiple of half a device pixel per world pixel,
//!   so every 2 px block covers whole device pixels - and a pinch settles
//!   on the nearest one when the fingers lift; a fine screen zooms
//!   smoothly. The view is drawn through `Camera::following`, so its
//!   corner sits on the block grid and the rest of its position is a shift
//!   of whole device pixels.
//! - Every pointer goes through the camera (`Viewport::to_world` on the
//!   `Camera` `view` gives): what the canvas draws under a point is what a
//!   press there paints.

use crate::math::{Rectangle, Vec2};
use crate::tuning::{tuning, Tuning};
use crate::view::{Camera, ScaleCap, View};
use crate::{DEFAULT_SCREEN_HEIGHT, DEFAULT_SCREEN_WIDTH, Layout, OBSTACLE_GRID_SIZE};

/// The screen the canvas is drawn on, as the builder measures it: the
/// zoom steps are counted in its device pixels and the touch sizes in its
/// points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasScreen {
    /// Device pixels per pixel of the bitmap: the view's scale times the
    /// framebuffer's pixels per window unit.
    pub device_per_px: f32,
    /// Points per pixel of the bitmap: the view's scale over the window's
    /// units per point (one but on Android, whose window is in device
    /// pixels).
    pub points_per_px: f32,
    /// A screen under `view_fine_ppi`, where a device pixel is big enough
    /// to show a 2 px block's edge: the zoom keeps the blocks whole there.
    pub coarse: bool,
}

impl Default for CanvasScreen {
    /// A bitmap pixel a device pixel and a point, on a coarse screen: the
    /// window the size of its bitmap on a desktop monitor.
    fn default() -> Self {
        CanvasScreen { device_per_px: 1.0, points_per_px: 1.0, coarse: true }
    }
}

/// The builder's knobs (the `builder` tuning group), read once a frame and
/// passed in so a test states the rules it runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasRules {
    /// `builder_zoom_step`: how much one wheel notch or key press zooms.
    pub zoom_step: f32,
    /// `builder_zoom_max_cell_pt`: the largest a cell is drawn, points.
    pub max_cell_pt: f32,
    /// `builder_touch_slop_pt`: how far a finger moves before it drags.
    pub slop_pt: f32,
    /// `builder_tap_seconds`: how long a tap may last.
    pub tap_seconds: f32,
    /// `builder_paint_min_cell_mm`: under this a finger does not paint.
    pub paint_min_cell_mm: f32,
    /// `builder_tap_zoom_cell_mm`: the cell a zooming tap goes to.
    pub tap_zoom_cell_mm: f32,
    /// `builder_edge_scroll_pt`: the margin a stroke scrolls the view in.
    pub edge_scroll_pt: f32,
    /// `builder_edge_scroll_pt_per_s`: how fast, at the very edge.
    pub edge_scroll_pt_per_s: f32,
    /// `builder_key_pan_pt_per_s`: how fast a held arrow key pans.
    pub key_pan_pt_per_s: f32,
}

impl CanvasRules {
    /// The rules in the tuning table this frame.
    pub fn current() -> CanvasRules {
        CanvasRules::of(&tuning())
    }

    /// The rules in `t`.
    pub fn of(t: &Tuning) -> CanvasRules {
        CanvasRules {
            zoom_step: t.builder_zoom_step,
            max_cell_pt: t.builder_zoom_max_cell_pt,
            slop_pt: t.builder_touch_slop_pt,
            tap_seconds: t.builder_tap_seconds,
            paint_min_cell_mm: t.builder_paint_min_cell_mm,
            tap_zoom_cell_mm: t.builder_tap_zoom_cell_mm,
            edge_scroll_pt: t.builder_edge_scroll_pt,
            edge_scroll_pt_per_s: t.builder_edge_scroll_pt_per_s,
            key_pan_pt_per_s: t.builder_key_pan_pt_per_s,
        }
    }
}

/// The finest whole-block scale: a 2 px block on one device pixel, in
/// device pixels per world pixel. Below it a block is less than a pixel
/// and the zoom is smooth on every screen.
pub const MIN_WHOLE_BLOCK_SCALE: f32 = 0.5;

/// One whole-block step, in device pixels per world pixel.
const BLOCK_STEP: f32 = 0.5;

/// A comparison's allowance for rounding between scales.
const EPS: f32 = 1e-4;

/// How the builder's bitmap lands on a window of `window` for a field map:
/// the bar at the size it has over the standard field's bitmap in the same
/// window (`View::fit_capped` under the same `cap`), so switching from an
/// arena to a field map never changes the chrome, and the canvas area
/// filling the rest of the window - no letterbox, the bitmap made to the
/// window's shape. Never smaller than the standard field's bitmap, which
/// the bar's slot tables are laid out for.
pub fn canvas_frame(window: (f32, f32), cap: Option<ScaleCap>) -> (Layout, View) {
    let standard = Layout::for_field(DEFAULT_SCREEN_WIDTH as f32, DEFAULT_SCREEN_HEIGHT as f32);
    let (sw, sh) = standard.window_size();
    let (sw, sh) = (sw as f32, sh as f32);
    let scale = View::fit_capped((sw, sh), window, cap).scale;
    let width = (window.0.max(1.0) / scale).floor().max(sw);
    let height = (window.1.max(1.0) / scale).floor().max(sh);
    let layout = Layout::for_field(width, height - crate::HUD_BAR_HEIGHT as f32);
    (layout, View::fill((width, height), window))
}

/// Everything a camera move needs to know: the map's field, the canvas
/// area it is drawn into and the screen under that.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// The map's field, world pixels.
    pub field: (f32, f32),
    /// The canvas area, bitmap pixels: `Layout::field`'s size.
    pub area: (f32, f32),
    pub screen: CanvasScreen,
}

impl Viewport {
    /// The field `field` drawn into `layout`'s field area on `screen`.
    pub fn of(field: (f32, f32), layout: &Layout, screen: CanvasScreen) -> Viewport {
        Viewport { field, area: (layout.field.w, layout.field.h), screen }
    }

    /// The scale that shows the whole field in the area, bitmap pixels per
    /// world pixel.
    pub fn fit_scale(&self) -> f32 {
        let (fw, fh) = (self.field.0.max(1.0), self.field.1.max(1.0));
        let s = (self.area.0 / fw).min(self.area.1 / fh);
        if s.is_finite() && s > 0.0 { s } else { 1.0 }
    }

    /// The largest scale: a cell of `max_cell_pt` points, never less than
    /// FIT (a map too small to need zooming cannot be).
    pub fn max_scale(&self, rules: &CanvasRules) -> f32 {
        let points = positive(self.screen.points_per_px, 1.0);
        (rules.max_cell_pt / (OBSTACLE_GRID_SIZE * points)).max(self.fit_scale())
    }

    /// Device pixels per world pixel at `scale`.
    pub fn device_scale(&self, scale: f32) -> f32 {
        scale * positive(self.screen.device_per_px, 1.0)
    }

    /// A cell's width on the glass at `scale`, millimetres: points over a
    /// touch screen's points to the millimetre (`indicators::POINTS_PER_MM`).
    pub fn cell_mm(&self, scale: f32) -> f32 {
        OBSTACLE_GRID_SIZE * scale * positive(self.screen.points_per_px, 1.0) / crate::indicators::POINTS_PER_MM
    }

    /// The scale at which a cell is `mm` wide on the glass.
    pub fn scale_for_cell_mm(&self, mm: f32) -> f32 {
        mm * crate::indicators::POINTS_PER_MM / (OBSTACLE_GRID_SIZE * positive(self.screen.points_per_px, 1.0))
    }

    /// `points` points as bitmap pixels.
    pub fn px(&self, points: f32) -> f32 {
        points / positive(self.screen.points_per_px, 1.0)
    }
}

/// The builder's camera: FIT, or a scale and the world point in the middle
/// of the canvas area. The scale is in bitmap pixels per world pixel, so it
/// survives the window changing size; the view is worked out against the
/// area each frame (`view`), clamped to the field there.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuilderCamera {
    zoom: Option<Zoom>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Zoom {
    scale: f32,
    center: Vec2,
}

impl BuilderCamera {
    /// Whether the view is FIT: the whole map, whatever the area.
    pub fn is_fit(&self) -> bool {
        self.zoom.is_none()
    }

    /// Back to FIT.
    pub fn fit(&mut self) {
        self.zoom = None;
    }

    /// Bitmap pixels per world pixel in `vp`: FIT's, or the zoom's while it
    /// is above FIT (a window grown under a zoom can make FIT the larger).
    pub fn scale(&self, vp: &Viewport) -> f32 {
        let fit = vp.fit_scale();
        self.zoom.map_or(fit, |z| z.scale.max(fit))
    }

    /// The world point in the middle of the canvas area.
    pub fn center(&self, vp: &Viewport) -> Vec2 {
        let half = Vec2::new(vp.field.0 / 2.0, vp.field.1 / 2.0);
        match self.zoom {
            Some(z) => clamp_center(z.center, self.scale(vp), vp),
            None => half,
        }
    }

    /// The view to draw and hit-test through. FIT on an arena's bitmap -
    /// the area the field itself - is `Camera::whole`, the picture a
    /// builder with no camera drew; everything else is a view of
    /// `area / scale` world pixels round the centre, its corner on the
    /// block grid and the rest a shift of whole device pixels
    /// (`Camera::following`).
    pub fn view(&self, vp: &Viewport) -> Camera {
        let scale = self.scale(vp);
        if self.is_fit() && (scale - 1.0).abs() < EPS && vp.area == vp.field {
            return Camera::whole(vp.field);
        }
        let center = self.center(vp);
        let size = (vp.area.0 / scale, vp.area.1 / scale);
        let corner = Vec2::new(center.x - size.0 / 2.0, center.y - size.1 / 2.0);
        Camera::following(vp.field, corner, size, scale, vp.device_scale(scale))
    }

    /// The world point under `at`, a point of the canvas area, before any
    /// rounding: what a move keeps under the fingers.
    pub fn world_at(&self, at: Vec2, vp: &Viewport) -> Vec2 {
        let scale = self.scale(vp);
        let center = self.center(vp);
        Vec2::new(center.x + (at.x - vp.area.0 / 2.0) / scale, center.y + (at.y - vp.area.1 / 2.0) / scale)
    }

    /// Put `center` in the middle at `scale`, both kept to what the field
    /// and the rules allow; a scale at or under FIT is FIT.
    pub fn set(&mut self, center: Vec2, scale: f32, vp: &Viewport, rules: &CanvasRules) {
        let fit = vp.fit_scale();
        let scale = if scale.is_finite() { scale.min(vp.max_scale(rules)) } else { fit };
        if scale <= fit * (1.0 + EPS) {
            self.zoom = None;
            return;
        }
        self.zoom = Some(Zoom { scale, center: clamp_center(center, scale, vp) });
    }

    /// Move the view so the world point under `from` comes to lie under
    /// `to`, magnified `factor` times (both points of the canvas area): a
    /// drag is `factor` 1, a pinch about its fingers' middle the ratio of
    /// their spread.
    pub fn move_point(&mut self, from: Vec2, to: Vec2, factor: f32, vp: &Viewport, rules: &CanvasRules) {
        let world = self.world_at(from, vp);
        let factor = if factor.is_finite() && factor > 0.0 { factor } else { 1.0 };
        let fit = vp.fit_scale();
        let scale = (self.scale(vp) * factor).clamp(fit, vp.max_scale(rules));
        let center = Vec2::new(world.x - (to.x - vp.area.0 / 2.0) / scale, world.y - (to.y - vp.area.1 / 2.0) / scale);
        self.set(center, scale, vp, rules);
    }

    /// Zoom to `scale` keeping the world point under `at` where it is.
    pub fn zoom_at(&mut self, scale: f32, at: Vec2, vp: &Viewport, rules: &CanvasRules) {
        let now = self.scale(vp);
        self.move_point(at, at, scale / now, vp, rules);
    }

    /// Drag the canvas by `delta` bitmap pixels: the world moves with the
    /// pointer.
    pub fn pan(&mut self, delta: Vec2, vp: &Viewport, rules: &CanvasRules) {
        if self.is_fit() {
            return;
        }
        let middle = Vec2::new(vp.area.0 / 2.0, vp.area.1 / 2.0);
        self.move_point(middle, Vec2::new(middle.x + delta.x, middle.y + delta.y), 1.0, vp, rules);
    }

    /// One zoom step in or out about `at`: by `zoom_step`, onto the next
    /// whole-block scale on a coarse screen.
    pub fn step(&mut self, zoom_in: bool, at: Vec2, vp: &Viewport, rules: &CanvasRules) {
        let now = vp.device_scale(self.scale(vp));
        let ratio = rules.zoom_step.max(1.01);
        let target = if zoom_in { now * ratio } else { now / ratio };
        let next = if vp.screen.coarse { whole_block_step(now, target, zoom_in) } else { target };
        self.zoom_at(next / positive(vp.screen.device_per_px, 1.0), at, vp, rules);
    }

    /// Where a coarse screen's zoom comes to rest after a pinch: the
    /// nearest whole-block scale, about `at`. A fine screen keeps it.
    pub fn settle(&mut self, at: Vec2, vp: &Viewport, rules: &CanvasRules) {
        if !vp.screen.coarse || self.is_fit() {
            return;
        }
        let now = vp.device_scale(self.scale(vp));
        self.zoom_at(nearest_whole_block(now) / positive(vp.screen.device_per_px, 1.0), at, vp, rules);
    }

    /// A zooming tap at `at`: a cell of `tap_zoom_cell_mm` on the glass, on
    /// the nearest whole-block scale on a coarse screen.
    pub fn zoom_for_tap(&mut self, at: Vec2, vp: &Viewport, rules: &CanvasRules) {
        self.zoom_at(tap_scale(vp, rules), at, vp, rules);
    }

    /// Put the world point `at` in the middle of the canvas - the
    /// navigator's press and drag (docs/large-maps-follow-camera.md §9):
    /// at the zoom the view has, or, from FIT, at the zoom a zooming tap
    /// goes to (`zoom_for_tap`). A map too small for that zoom stays at
    /// FIT.
    pub fn navigate(&mut self, at: Vec2, vp: &Viewport, rules: &CanvasRules) {
        let scale = if self.is_fit() { tap_scale(vp, rules) } else { self.scale(vp) };
        self.set(at, scale, vp, rules);
    }

    /// Move the middle of a zoomed view by `delta` world pixels - the map
    /// moved under it (a resize's anchor), and the view goes with it.
    pub fn shift(&mut self, delta: Vec2) {
        if let Some(z) = &mut self.zoom {
            z.center = Vec2::new(z.center.x + delta.x, z.center.y + delta.y);
        }
    }
}

/// Where the canvas lands on the window: `window = (world - corner) *
/// zoom + offset`, clipped to `area`. What `MapEditor::render` draws the
/// canvas through when it draws it straight onto the window, and the
/// inverse of a pointer's path to the world (`View::to_bitmap`,
/// `Layout::to_field`, `Camera::to_world`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowMapping {
    /// The world point at the canvas area's top-left corner: the view's,
    /// sub-block offset included (`Camera::rect`).
    pub corner: Vec2,
    /// Where that corner is on the window, in the window's units.
    pub offset: Vec2,
    /// Window units per world pixel.
    pub zoom: f32,
    /// The canvas area on the window.
    pub area: Rectangle,
}

impl WindowMapping {
    /// A world point on the window.
    pub fn to_window(&self, world: Vec2) -> Vec2 {
        Vec2::new((world.x - self.corner.x) * self.zoom + self.offset.x, (world.y - self.corner.y) * self.zoom + self.offset.y)
    }
}

/// The most texels a side of the builder's scene target holds: the size
/// every GPU the game runs on takes, a WebGL one on an older Android
/// included.
pub const SCENE_MAX_TEXELS: f32 = 4096.0;

/// How a view the canvas does not show whole at its own size is drawn:
/// into a scene target at `zoom` texels per world pixel from the world
/// point `origin`, the way a round draws its world (a texel a world pixel,
/// so every sprite is sampled at the scale its art is made for and never
/// bleeds into its neighbour on the sheet), then `source` - the texels
/// under the view - scaled onto the canvas area. `zoom` is 1, or the
/// largest power of two under it that keeps a view of a big map inside
/// `SCENE_MAX_TEXELS`; `origin` sits on a grid of whole texels and whole
/// 2 px blocks, so a tile's edge is a texel's edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScenePlan {
    pub zoom: f32,
    pub origin: Vec2,
    /// The texels the drawing needs, from the target's top-left corner.
    pub size: (i32, i32),
    /// The view's texels, top-down from the target's top-left corner.
    pub source: Rectangle,
}

/// The scene a view `camera` shows is drawn through (`ScenePlan`).
pub fn scene_plan(camera: &Camera) -> ScenePlan {
    let rect = camera.rect();
    // The texels a side needs are the view's, the part of a block the
    // origin's snap adds in front (under two texels) and one more behind.
    let needs = |side: f32, zoom: f32| (side * zoom).ceil() + 3.0;
    let mut zoom = 1.0f32;
    while zoom > 1.0 / 64.0 && (needs(rect.width, zoom) > SCENE_MAX_TEXELS || needs(rect.height, zoom) > SCENE_MAX_TEXELS) {
        zoom *= 0.5;
    }
    let grid = crate::pyro::BLOCK / zoom;
    let origin = Vec2::new((rect.x / grid).floor() * grid, (rect.y / grid).floor() * grid);
    let source = Rectangle::new((rect.x - origin.x) * zoom, (rect.y - origin.y) * zoom, rect.width * zoom, rect.height * zoom);
    let size = ((source.x + source.width).ceil() as i32 + 1, (source.y + source.height).ceil() as i32 + 1);
    ScenePlan { zoom, origin, size, source }
}

/// The mapping a frame drawn through `camera` into `layout`'s field area,
/// put on the window by `view`, lands the canvas with.
pub fn window_mapping(camera: &Camera, view: &View, layout: &Layout) -> WindowMapping {
    let rect = camera.rect();
    let offset = view.to_window(layout.field_origin());
    WindowMapping {
        corner: Vec2::new(rect.x, rect.y),
        offset,
        zoom: camera.scale * view.scale,
        area: Rectangle::new(offset.x, offset.y, layout.field.w * view.scale, layout.field.h * view.scale),
    }
}

/// The scale a zooming tap goes to: a cell of `tap_zoom_cell_mm` on the
/// glass, on the nearest whole-block scale on a coarse screen.
fn tap_scale(vp: &Viewport, rules: &CanvasRules) -> f32 {
    let scale = vp.scale_for_cell_mm(rules.tap_zoom_cell_mm);
    if vp.screen.coarse { nearest_whole_block(vp.device_scale(scale)) / positive(vp.screen.device_per_px, 1.0) } else { scale }
}

/// Keep a view of `area / scale` round `c` inside the field on an axis the
/// field fills, and centred on one it does not.
fn clamp_center(c: Vec2, scale: f32, vp: &Viewport) -> Vec2 {
    let axis = |c: f32, area: f32, field: f32| {
        let half = area / scale / 2.0;
        if !c.is_finite() || field <= 2.0 * half {
            field / 2.0
        } else {
            c.clamp(half, field - half)
        }
    };
    Vec2::new(axis(c.x, vp.area.0, vp.field.0), axis(c.y, vp.area.1, vp.field.1))
}

/// The next scale from `now` toward `target` (device pixels per world
/// pixel) among the whole-block scales: the multiple of half a pixel
/// nearest the target, and at least one step on from `now`. Below the
/// finest whole-block scale the step is the target itself.
pub fn whole_block_step(now: f32, target: f32, zoom_in: bool) -> f32 {
    if zoom_in {
        if target < MIN_WHOLE_BLOCK_SCALE {
            return target;
        }
        let next = ((now + EPS) / BLOCK_STEP).floor() * BLOCK_STEP + BLOCK_STEP;
        snap(target).max(next).max(MIN_WHOLE_BLOCK_SCALE)
    } else {
        if now <= MIN_WHOLE_BLOCK_SCALE + EPS {
            return target;
        }
        let previous = ((now - EPS) / BLOCK_STEP).ceil() * BLOCK_STEP - BLOCK_STEP;
        let step = snap(target).min(previous);
        if step < MIN_WHOLE_BLOCK_SCALE { target.min(MIN_WHOLE_BLOCK_SCALE) } else { step }
    }
}

/// The whole-block scale nearest `scale` by ratio - the lower one on a tie
/// - or `scale` itself under the finest.
pub fn nearest_whole_block(scale: f32) -> f32 {
    if scale < MIN_WHOLE_BLOCK_SCALE {
        return scale;
    }
    let lo = (scale / BLOCK_STEP).floor() * BLOCK_STEP;
    let hi = (scale / BLOCK_STEP).ceil() * BLOCK_STEP;
    if lo < MIN_WHOLE_BLOCK_SCALE || scale * scale > lo * hi { hi } else { lo }
}

/// The multiple of half a pixel nearest `v`.
fn snap(v: f32) -> f32 {
    (v / BLOCK_STEP).round() * BLOCK_STEP
}

/// `v` if it is a positive number, else `or`.
fn positive(v: f32, or: f32) -> f32 {
    if v.is_finite() && v > 0.0 { v } else { or }
}

#[cfg(test)]
mod camera_tests {
    use super::*;

    const STANDARD: (f32, f32) = (1088.0, 544.0);
    const STUDY: (f32, f32) = (96.0 * 32.0, 54.0 * 32.0);

    fn rules() -> CanvasRules {
        CanvasRules::of(&Tuning::DEFAULT)
    }

    fn vp(field: (f32, f32), area: (f32, f32), device: f32, coarse: bool) -> Viewport {
        Viewport { field, area, screen: CanvasScreen { device_per_px: device, points_per_px: device, coarse } }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    /// FIT on an arena's own bitmap is the camera a builder with no camera
    /// drew through; on a field map's area it is the whole map, centred on
    /// the axis it does not fill.
    #[test]
    fn fit_is_the_whole_field_and_the_arenas_own_picture() {
        let arena = vp(STANDARD, STANDARD, 1.5, true);
        let cam = BuilderCamera::default();
        assert_eq!(cam.view(&arena), Camera::whole(STANDARD));
        let field = vp(STUDY, (1280.0, 688.0), 1.5, true);
        let view = cam.view(&field);
        let fit = 688.0 / STUDY.1;
        assert!(near(view.scale, fit), "{view:?}");
        let r = view.rect();
        assert!(near(r.y, 0.0) && near(r.height, STUDY.1), "the map fills the height: {r:?}");
        // Centred to the device pixel the drawn view is rounded to.
        let half_device_px = 0.5 / field.device_scale(fit);
        assert!(r.x < 0.0 && (r.x + r.width / 2.0 - STUDY.0 / 2.0).abs() <= half_device_px, "centred across: {r:?}");
        assert!(cam.is_fit());
    }

    /// A zoom keeps the world point under the pointer where it is, and
    /// zooming all the way out is FIT again.
    #[test]
    fn a_zoom_at_a_point_keeps_that_point_under_it() {
        let v = vp(STUDY, (1280.0, 688.0), 1.0, false);
        let mut cam = BuilderCamera::default();
        let at = Vec2::new(300.0, 200.0);
        let before = cam.world_at(at, &v);
        cam.zoom_at(2.0, at, &v, &rules());
        assert!(!cam.is_fit() && near(cam.scale(&v), 2.0));
        let after = cam.world_at(at, &v);
        assert!(near(before.x, after.x) && near(before.y, after.y), "{before:?} -> {after:?}");
        // The drawn view agrees with the unrounded one to a device pixel.
        let drawn = cam.view(&v).to_world(at);
        assert!((drawn.x - after.x).abs() <= 1.0 && (drawn.y - after.y).abs() <= 1.0);
        cam.zoom_at(0.01, at, &v, &rules());
        assert!(cam.is_fit());
        // Never past the largest cell.
        cam.zoom_at(1000.0, at, &v, &rules());
        assert!(near(cam.scale(&v), v.max_scale(&rules())));
        assert!(near(32.0 * cam.scale(&v) * v.screen.points_per_px, rules().max_cell_pt));
    }

    /// A pan drags the world with the pointer and stops at the field's edge.
    #[test]
    fn a_pan_follows_the_pointer_and_stops_at_the_edge() {
        let v = vp(STUDY, (1280.0, 688.0), 1.0, false);
        let mut cam = BuilderCamera::default();
        cam.pan(Vec2::new(50.0, 0.0), &v, &rules());
        assert!(cam.is_fit(), "FIT has nothing to pan");
        cam.zoom_at(2.0, Vec2::new(640.0, 344.0), &v, &rules());
        let at = Vec2::new(500.0, 300.0);
        let under = cam.world_at(at, &v);
        cam.pan(Vec2::new(-40.0, 30.0), &v, &rules());
        let moved = cam.world_at(Vec2::new(460.0, 330.0), &v);
        assert!(near(under.x, moved.x) && near(under.y, moved.y), "{under:?} vs {moved:?}");
        cam.pan(Vec2::new(1e6, 1e6), &v, &rules());
        let r = cam.view(&v).rect();
        assert!(r.x.abs() <= 1.0 && r.y.abs() <= 1.0, "clamped at the top-left corner: {r:?}");
        cam.pan(Vec2::new(-1e6, -1e6), &v, &rules());
        let r = cam.view(&v).rect();
        assert!((r.x + r.width - STUDY.0).abs() <= 1.0 && (r.y + r.height - STUDY.1).abs() <= 1.0, "{r:?}");
        cam.fit();
        assert!(cam.is_fit());
    }

    /// A coarse screen's steps land on whole blocks and are each at least
    /// one block step apart; a fine screen's are a steady ratio.
    #[test]
    fn coarse_steps_keep_blocks_whole_and_fine_ones_are_smooth() {
        let mut d = 1.0;
        for want in [1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.5, 8.0] {
            d = whole_block_step(d, d * 1.25, true);
            assert_eq!(d, want);
        }
        for want in [6.5, 5.0, 4.0, 3.0, 2.5, 2.0, 1.5, 1.0, 0.5] {
            d = whole_block_step(d, d / 1.25, false);
            assert_eq!(d, want);
        }
        assert!(near(whole_block_step(0.5, 0.4, false), 0.4), "under a pixel a block, smooth");
        assert_eq!(nearest_whole_block(1.7), 1.5);
        assert_eq!(nearest_whole_block(1.8), 2.0);
        assert_eq!(nearest_whole_block(0.3), 0.3);
        // Through the camera: every coarse step is whole blocks on the glass.
        let v = vp(STUDY, (1280.0, 688.0), 1.5, true);
        let mut cam = BuilderCamera::default();
        let at = Vec2::new(640.0, 344.0);
        for _ in 0..8 {
            cam.step(true, at, &v, &rules());
            let device = v.device_scale(cam.scale(&v));
            assert!(near(device * 2.0, (device * 2.0).round()), "{device}");
        }
        let fine = vp(STUDY, (1280.0, 688.0), 3.0, false);
        let mut cam = BuilderCamera::default();
        cam.step(true, at, &fine, &rules());
        let s = cam.scale(&fine);
        cam.step(true, at, &fine, &rules());
        assert!(near(cam.scale(&fine) / s, rules().zoom_step));
        // Stepping out comes back to FIT and stops there.
        for _ in 0..20 {
            cam.step(false, at, &fine, &rules());
        }
        assert!(cam.is_fit());
    }

    /// A pinch that ends between two whole-block scales settles on the
    /// nearer on a coarse screen, about the fingers.
    #[test]
    fn a_pinch_settles_on_whole_blocks_on_a_coarse_screen() {
        let v = vp(STUDY, (1280.0, 688.0), 2.0, true);
        let mut cam = BuilderCamera::default();
        let at = Vec2::new(400.0, 300.0);
        cam.zoom_at(1.13 / 2.0, at, &v, &rules());
        let under = cam.world_at(at, &v);
        cam.settle(at, &v, &rules());
        assert!(near(v.device_scale(cam.scale(&v)), 1.0));
        let after = cam.world_at(at, &v);
        assert!(near(under.x, after.x) && near(under.y, after.y));
        let mut fine = cam;
        let fv = Viewport { screen: CanvasScreen { coarse: false, ..v.screen }, ..v };
        fine.zoom_at(1.13 / 2.0, at, &fv, &rules());
        fine.settle(at, &fv, &rules());
        assert!(near(fv.device_scale(fine.scale(&fv)), 1.13), "a fine screen keeps the pinch");
    }

    /// The field map's bitmap is the window's shape with the standard
    /// field's bar: no letterbox, and the bar no smaller than an arena's.
    #[test]
    fn a_field_maps_bitmap_fills_the_window_with_an_arenas_bar() {
        let cap = Some(ScaleCap { max_scale: 1.5, snap_half: false });
        for (window, cap) in [((852.0, 393.0), None), ((1180.0, 820.0), None), ((1920.0, 1080.0), cap), ((1088.0, 576.0), cap)] {
            let (layout, view) = canvas_frame(window, cap);
            let arena = View::fit_capped((1088.0, 576.0), window, cap);
            assert!((view.scale - arena.scale).abs() / arena.scale < 0.01, "{window:?}: {view:?} vs {arena:?}");
            assert!(layout.panel.w >= 1088.0 && layout.field.w >= 1088.0, "{window:?}: {layout:?}");
            let d = view.dest();
            assert!((d.width - window.0).abs() <= view.scale + 0.01 && (d.height - window.1).abs() <= view.scale + 0.01, "{window:?}: {d:?}");
            assert_eq!(layout.panel.h, crate::HUD_BAR_HEIGHT as f32);
        }
    }

    /// The scene a view is drawn into holds the view at a texel per world
    /// pixel from a corner on the block grid, and halves its texels for a
    /// view too big for one target.
    #[test]
    fn the_scene_holds_the_view_at_a_texel_a_world_pixel_or_halves() {
        let v = vp(STUDY, (1280.0, 688.0), 1.5, true);
        let mut cam = BuilderCamera::default();
        for zoom_in in [false, true] {
            if zoom_in {
                cam.zoom_at(1.7, Vec2::new(321.0, 123.0), &v, &rules());
            }
            let view = cam.view(&v);
            let plan = scene_plan(&view);
            let r = view.rect();
            assert_eq!(plan.zoom, 1.0);
            assert_eq!((plan.origin.x % 2.0, plan.origin.y % 2.0), (0.0, 0.0), "{plan:?}");
            assert!(near(plan.origin.x + plan.source.x, r.x) && near(plan.origin.y + plan.source.y, r.y), "{plan:?} vs {r:?}");
            assert!(near(plan.source.width, r.width) && near(plan.source.height, r.height));
            assert!(plan.size.0 as f32 >= plan.source.x + plan.source.width && plan.size.1 as f32 >= plan.source.y + plan.source.height);
        }
        // The largest map at FIT is more than one target holds: half a
        // texel a world pixel, on a grid of whole texels and blocks.
        let huge = vp((8000.0, 8000.0), (1280.0, 688.0), 1.5, true);
        let plan = scene_plan(&BuilderCamera::default().view(&huge));
        assert_eq!(plan.zoom, 0.25, "{plan:?}");
        assert!(plan.size.0 as f32 <= SCENE_MAX_TEXELS && plan.size.1 as f32 <= SCENE_MAX_TEXELS, "{plan:?}");
        assert_eq!((plan.origin.x % 8.0, plan.origin.y % 8.0), (0.0, 0.0), "{plan:?}");
        // Never a texel past the limit, whatever the view's size and
        // however its corner falls on the block grid (a quarter-pixel
        // device leaves the corner up to 1.75 world px into its block).
        for side in 4085..4100 {
            for corner in [0.0, 0.3, 1.0, 1.7, 1.99] {
                let view = Camera::following((9000.0, 9000.0), Vec2::new(corner, corner), (side as f32, side as f32), 1.0, 4.0);
                let plan = scene_plan(&view);
                assert!(plan.size.0 as f32 <= SCENE_MAX_TEXELS && plan.size.1 as f32 <= SCENE_MAX_TEXELS, "{side} from {corner}: {plan:?}");
            }
        }
    }

    /// The paint threshold's sizes are millimetres on a touch screen's
    /// points.
    #[test]
    fn a_cell_is_measured_on_the_glass() {
        let v = vp(STUDY, (1249.0, 544.0), 2.04, false);
        let v = Viewport { screen: CanvasScreen { points_per_px: 0.682, ..v.screen }, ..v };
        let fit = v.fit_scale();
        let mm = v.cell_mm(fit);
        assert!(mm > 1.0 && mm < 3.0, "the study map at FIT on a phone: {mm} mm");
        let s = v.scale_for_cell_mm(9.0);
        assert!(near(v.cell_mm(s), 9.0));
        let mut cam = BuilderCamera::default();
        cam.zoom_for_tap(Vec2::new(100.0, 100.0), &v, &rules());
        assert!(near(v.cell_mm(cam.scale(&v)), rules().tap_zoom_cell_mm), "a fine screen goes exactly there");
    }
}
