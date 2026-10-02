//! Putting the composited bitmap on the window (`view.rs` owns the
//! mapping itself), and the raylib cameras a `view::Camera` draws the
//! world through.

use sola_raylib::prelude::*;

use crate::math::{Color, Rectangle, Vec2};
use crate::view::{Camera, View};
use crate::Layout;

impl Camera {
    /// The camera pass 1 draws the world through, into a scene target of
    /// `target_size`: the view's corner at the target's, one texel per
    /// world pixel. The identity for the whole field.
    pub fn in_target(&self) -> Camera2D {
        Camera2D { offset: Vector2::new(0.0, 0.0), target: self.origin.into(), rotation: 0.0, zoom: 1.0 }
    }

    /// The camera that puts what lies in the world onto the field area of
    /// the bitmap at `field_origin`, where the frame shows it: the view's
    /// corner (`rect` - the block-grid origin plus a followed view's
    /// sub-block offset) at the field's, `scale` bitmap pixels to the world
    /// pixel - `to_view`'s mapping. The field camera for the whole field.
    pub fn on_field(&self, field_origin: Vec2) -> Camera2D {
        let corner = self.rect();
        Camera2D { offset: field_origin.into(), target: Vector2::new(corner.x, corner.y), rotation: 0.0, zoom: self.scale }
    }
}

/// Put the composited frame on screen: the margins in `backdrop` (the
/// HUD bar's own colour, so the bar and the margins read as one panel), a
/// one-pixel frame around the bitmap, then the bitmap into `view.dest()`.
/// Nearest filtering keeps the pixel art's blocks whole where the scale is
/// an integer and sharp elsewhere. A window the bitmap's own size gets no
/// margins and no frame.
pub fn present(rl: &mut RaylibHandle, thread: &RaylibThread, composite: &RenderTexture2D, view: &View, backdrop: Color) {
    rl.draw(thread, |mut d| present_into(&mut d, composite, view, backdrop, None));
}

/// `present` into the frame `d` is drawing, so what stands on the window -
/// the HUD's corners, its dialogs - can follow it onto the same frame.
/// With `margins`, the window round the bitmap shows the world past an
/// arena's field (`render::margin`) rather than the bars, on the bitmap's
/// own pixel grid so the two meet block for block. Either way the bitmap
/// lands on the backdrop's colour, which is what shows through where a
/// translucent draw left its alpha under one - so the field looks the same
/// with margins or bars round it, and the margins, put on the same way,
/// match it at the edge.
pub fn present_into(d: &mut impl RaylibDraw, composite: &RenderTexture2D, view: &View, backdrop: Color, margins: Option<&Margins>) {
    // A render texture reads back bottom-up; a negative source height
    // flips it on the way out.
    let source = Rectangle::new(0.0, 0.0, view.bitmap.0, -view.bitmap.1);
    let dest = view.dest();
    d.clear_background(Color::BLACK);
    match margins {
        Some(m) => {
            d.draw_rectangle(0, 0, view.window.0 as i32, view.window.1 as i32, backdrop);
            for part in &m.parts {
                // Read the right way up: a render texture is stored
                // bottom-up.
                let rows = Rectangle::new(part.x - m.rect.x, m.rect.y + m.rect.height - part.y - part.height, part.width, -part.height);
                let at = view.to_window(Vec2::new(part.x, part.y));
                let to = Rectangle::new(at.x, at.y, part.width * view.scale, part.height * view.scale);
                d.draw_texture_pro(m.target, rows, to, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
            }
        }
        None => letterbox(d, view, backdrop),
    }
    d.draw_texture_pro(composite, source, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
}

/// What an arena shows round its field (`render::margin`): the target
/// holding the world past it, drawn through the same steps as the field's
/// bitmap, the rectangle of the bitmap it covers - its world rectangle
/// moved to the field's place in the bitmap - and the parts of that past
/// the field, in the bitmap's pixels, which are all of it that shows.
pub struct Margins<'a> {
    pub target: &'a RenderTexture2D,
    pub rect: Rectangle,
    pub parts: Vec<Rectangle>,
}

/// The window round a bitmap that does not fill it: the margins in
/// `backdrop` and a one-pixel frame round where the bitmap goes. Nothing
/// for a window the bitmap's own size.
pub fn letterbox(d: &mut impl RaylibDraw, view: &View, backdrop: Color) {
    if view.is_identity() {
        return;
    }
    let dest = view.dest();
    d.draw_rectangle(0, 0, view.window.0 as i32, view.window.1 as i32, backdrop);
    d.draw_rectangle_lines_ex(Rectangle::new(dest.x - 1.0, dest.y - 1.0, dest.width + 2.0, dest.height + 2.0), 1.0, FRAME);
}

/// Put a followed view's world on the window (docs/large-maps-follow-camera.md
/// §6): `world` holds the scene target's every texel, from the camera's
/// block-grid `origin`, and the view's own corner is `camera.offset`
/// further on, so the texels from there fill the field area of the bitmap
/// as `view` puts it on the window - the sub-block motion the grid cannot
/// hold, applied here as a shift of whole device pixels
/// (`Camera::following` rounded it so), which keeps every block whole.
/// The window is cleared to `backdrop` first, for the bars of a screen
/// past the aspect clamp. Drawn into the frame `d` is drawing; the chrome
/// comes after it, onto the same frame.
pub fn present_world(d: &mut impl RaylibDraw, world: &RenderTexture2D, camera: &Camera, view: &View, layout: &Layout, backdrop: Color) {
    d.clear_background(backdrop);
    let (_, target_h) = camera.target_size();
    let (w, h) = camera.size;
    // A render texture reads back bottom-up: the texel row the view's top
    // edge starts on is counted from the texture's bottom.
    let source = Rectangle::new(camera.offset.x, target_h as f32 - camera.offset.y - h, w, -h);
    let corner = view.to_window(layout.field_origin());
    let dest = Rectangle::new(corner.x, corner.y, layout.field.w * view.scale, layout.field.h * view.scale);
    d.draw_texture_pro(world, source, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
}

/// The second half of a couch's split screen (`follow::Split`) on the
/// window, over the first already presented by `present_world`: its world
/// - `world` holding its scene target's every texel, from `camera`'s
/// block-grid origin, as `present_world`'s does - shifted by its own
/// sub-block offset and drawn only over the part of the field area past
/// the divider (the line through `at`, square to the unit `normal`, both in
/// the followed bitmap's pixels), so the cut is exact at any angle.
pub fn present_half(_d: &mut impl RaylibDraw, world: &RenderTexture2D, camera: &Camera, view: &View, layout: &Layout, at: Vec2, normal: Vec2) {
    let origin = view.to_window(layout.field_origin());
    let (w, h) = (layout.field.w * view.scale, layout.field.h * view.scale);
    let field = [origin, Vec2::new(origin.x + w, origin.y), Vec2::new(origin.x + w, origin.y + h), Vec2::new(origin.x, origin.y + h)];
    let shown = past_line(&field, Vec2::new(origin.x + at.x * view.scale, origin.y + at.y * view.scale), normal);
    let (tex_w, tex_h) = (world.texture.width.max(1) as f32, world.texture.height.max(1) as f32);
    let (_, target_h) = camera.target_size();
    // A window point: the point of the field area under it, then the texel
    // of the half's world that shows there - a render texture reads back
    // bottom-up, so its rows count from the bottom.
    draw_textured(world.texture.id, &shown, |p| {
        let (x, y) = ((p.x - origin.x) / view.scale, (p.y - origin.y) / view.scale);
        Vec2::new((camera.offset.x + x / camera.scale) / tex_w, (target_h as f32 - camera.offset.y - y / camera.scale) / tex_h)
    });
}

/// The second half of a couch's split screen while the establishing shot
/// zooms into it (`establish::ZoomSplit`): `composite` - the whole field,
/// which `present_into` just put on the window through the first half's
/// view - put on it again through the second half's `view`, over the part
/// of the window past the line through `at` square to the unit `normal`
/// (window units): the margins in `backdrop` and their frame as `letterbox`
/// draws them, then the field. Each half is what `present_into` would show
/// through its own view.
pub fn present_zoom_half(d: &mut impl RaylibDraw, composite: &RenderTexture2D, view: &View, at: Vec2, normal: Vec2, backdrop: Color) {
    let (ww, wh) = view.window;
    let part = past_line(&corners(Rectangle::new(0.0, 0.0, ww, wh)), at, normal);
    if part.len() < 3 {
        return;
    }
    let dest = view.dest();
    if view.is_identity() {
        fill_convex(d, &part, Color::BLACK);
    } else {
        fill_convex(d, &part, backdrop);
        // `draw_rectangle_lines_ex`'s four strips, a pixel thick.
        let (x, y, w, h) = (dest.x - 1.0, dest.y - 1.0, dest.width + 2.0, dest.height + 2.0);
        for strip in [
            Rectangle::new(x, y, w, 1.0),
            Rectangle::new(x, y + h - 1.0, w, 1.0),
            Rectangle::new(x, y + 1.0, 1.0, h - 2.0),
            Rectangle::new(x + w - 1.0, y + 1.0, 1.0, h - 2.0),
        ] {
            fill_convex(d, &past_line(&corners(strip), at, normal), FRAME);
        }
    }
    let shown = past_line(&corners(dest), at, normal);
    let (tex_w, tex_h) = (composite.texture.width.max(1) as f32, composite.texture.height.max(1) as f32);
    // `present_into`'s source rectangle, the bitmap read the right way up.
    draw_textured(composite.texture.id, &shown, |p| {
        let (x, y) = ((p.x - dest.x) / view.scale, (p.y - dest.y) / view.scale);
        Vec2::new(x / tex_w, (view.bitmap.1 - y) / tex_h)
    });
}

/// A rectangle's corners, turning clockwise on the window.
fn corners(r: Rectangle) -> [Vec2; 4] {
    [Vec2::new(r.x, r.y), Vec2::new(r.x + r.width, r.y), Vec2::new(r.x + r.width, r.y + r.height), Vec2::new(r.x, r.y + r.height)]
}

/// The convex polygon `points` (window units) filled with `texture`, each
/// corner showing the texel at `uv` of it (0 to 1), as a fan of triangles
/// straight into raylib's batch. Nothing for fewer than three corners.
fn draw_textured(texture: u32, points: &[Vec2], uv: impl Fn(Vec2) -> Vec2) {
    if points.len() < 3 {
        return;
    }
    let corners: Vec<(Vec2, Vec2)> = points.iter().map(|&p| (p, uv(p))).collect();
    // SAFETY: plain rlgl immediate-mode calls inside the frame being
    // drawn; they append textured vertices to raylib's batch, which flushes
    // itself when full or when the texture changes.
    unsafe {
        sola_raylib::ffi::rlSetTexture(texture);
        sola_raylib::ffi::rlBegin(0x0004); // RL_TRIANGLES
        sola_raylib::ffi::rlColor4ub(255, 255, 255, 255);
        for i in 1..corners.len() - 1 {
            for (p, uv) in counter_clockwise(corners[0], corners[i], corners[i + 1]) {
                sola_raylib::ffi::rlTexCoord2f(uv.x, uv.y);
                sola_raylib::ffi::rlVertex2f(p.x, p.y);
            }
        }
        sola_raylib::ffi::rlEnd();
        sola_raylib::ffi::rlSetTexture(0);
    }
}

/// The convex polygon `points` (window units) filled with `color`.
fn fill_convex(d: &mut impl RaylibDraw, points: &[Vec2], color: Color) {
    for i in 1..points.len().saturating_sub(1) {
        let [a, b, c] = counter_clockwise((points[0], ()), (points[i], ()), (points[i + 1], ()));
        d.draw_triangle(a.0, b.0, c.0, color);
    }
}

/// A triangle's corners in the order raylib draws them, counter-clockwise
/// on the window (whose y grows downward), whatever order they came in.
fn counter_clockwise<T>(a: (Vec2, T), b: (Vec2, T), c: (Vec2, T)) -> [(Vec2, T); 3] {
    let cross = (b.0.x - a.0.x) * (c.0.y - a.0.y) - (b.0.y - a.0.y) * (c.0.x - a.0.x);
    if cross > 0.0 { [a, c, b] } else { [a, b, c] }
}

/// The part of the convex polygon `points` past the line through `at`
/// square to `normal` - where `(p - at) . normal` is positive - as a convex
/// polygon in the same turning order (one clip of Sutherland-Hodgman).
pub fn past_line(points: &[Vec2], at: Vec2, normal: Vec2) -> Vec<Vec2> {
    let side = |p: Vec2| (p.x - at.x) * normal.x + (p.y - at.y) * normal.y;
    let mut out = Vec::with_capacity(points.len() + 1);
    for (i, &p) in points.iter().enumerate() {
        let q = points[(i + 1) % points.len()];
        let (sp, sq) = (side(p), side(q));
        if sp > 0.0 {
            out.push(p);
        }
        if (sp > 0.0) != (sq > 0.0) {
            let t = sp / (sp - sq);
            out.push(Vec2::new(p.x + (q.x - p.x) * t, p.y + (q.y - p.y) * t));
        }
    }
    out
}

/// The divider between a split screen's halves on the window: the line
/// through `at` square to the unit `normal` (the followed bitmap's pixels)
/// across the field area, `DIVIDER_PX` of the bitmap's pixels wide - a
/// block of the art, whatever the scale - in `color`, faded in with how far
/// apart the halves' views stand (`apart`, world px), so a split opening
/// from one picture draws its line in as the pictures part.
pub fn draw_divider(d: &mut impl RaylibDraw, view: &View, layout: &Layout, at: Vec2, normal: Vec2, apart: f32, color: Color) {
    let origin = view.to_window(layout.field_origin());
    let field = Rectangle::new(origin.x, origin.y, layout.field.w * view.scale, layout.field.h * view.scale);
    let at = Vec2::new(origin.x + at.x * view.scale, origin.y + at.y * view.scale);
    line_across(d, field, at, normal, DIVIDER_PX * view.scale, divider_alpha(apart), color);
}

/// The divider while the establishing shot zooms into a split
/// (`establish::ZoomSplit`): the line through `at` square to `normal`
/// (window units) across the window, a block of the art wide at the zoom's
/// `scale` (window units per world pixel), faded in by `alpha` on top of
/// `draw_divider`'s fade with how far `apart` the follow views stand.
#[allow(clippy::too_many_arguments)]
pub fn draw_zoom_divider(d: &mut impl RaylibDraw, window: (f32, f32), at: Vec2, normal: Vec2, scale: f32, alpha: f32, apart: f32, color: Color) {
    line_across(d, Rectangle::new(0.0, 0.0, window.0, window.1), at, normal, DIVIDER_PX * scale, alpha * divider_alpha(apart), color);
}

/// How far a split's divider is drawn in for halves whose views stand
/// `apart` world px apart.
fn divider_alpha(apart: f32) -> f32 {
    (apart / DIVIDER_FADE_PX).clamp(0.0, 1.0)
}

/// The line through `at` square to the unit `normal` across `rect`, `width`
/// wide, in `color` at `alpha` of its own: all in window units.
fn line_across(d: &mut impl RaylibDraw, rect: Rectangle, at: Vec2, normal: Vec2, width: f32, alpha: f32, color: Color) {
    let alpha = if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 0.0 };
    if alpha <= 0.0 {
        return;
    }
    // The line's two ends: where it crosses the rectangle's edges.
    let along = Vec2::new(-normal.y, normal.x);
    let reach = rect.width + rect.height;
    let (from, to) = (at - along * reach, at + along * reach);
    let corner = Vec2::new(rect.x, rect.y);
    let Some((from, to)) = clip_segment(from - corner, to - corner, rect.width, rect.height) else { return };
    let faded = Color::new(color.r, color.g, color.b, (color.a as f32 * alpha).round() as u8);
    d.draw_line_ex(from + corner, to + corner, width, faded);
}

/// The segment from `a` to `b` kept inside the rectangle from the origin
/// to `(w, h)` (Liang-Barsky); `None` where none of it is.
fn clip_segment(a: Vec2, b: Vec2, w: f32, h: f32) -> Option<(Vec2, Vec2)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-d.x, a.x), (d.x, w - a.x), (-d.y, a.y), (d.y, h - a.y)] {
        if p.abs() < 1e-9 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let r = q / p;
        if p < 0.0 {
            t0 = t0.max(r);
        } else {
            t1 = t1.min(r);
        }
    }
    (t0 <= t1).then(|| (a + d * t0, a + d * t1))
}

/// The divider's width in the followed bitmap's pixels: one 2 px block.
const DIVIDER_PX: f32 = 2.0;

/// How far apart, world px, a split's two views stand before its divider
/// is drawn in full.
const DIVIDER_FADE_PX: f32 = 8.0;

/// The frame around the bitmap when it does not fill the window.
const FRAME: Color = Color::new(62, 62, 66, 255);

#[cfg(test)]
mod tests {
    use super::*;

    /// Where raylib's `Camera2D` puts a world point: `(world - target) *
    /// zoom + offset`.
    fn through(c: Camera2D, world: Vec2) -> Vec2 {
        Vec2::new((world.x - c.target.x) * c.zoom + c.offset.x, (world.y - c.target.y) * c.zoom + c.offset.y)
    }

    #[test]
    fn on_field_puts_a_world_point_where_the_frame_shows_it() {
        let field_origin = Vec2::new(0.0, 32.0);
        let cameras = [
            Camera::whole((1088.0, 544.0)),
            Camera::zoomed((1536.0, 768.0), Vec2::new(700.0, 300.0), 2.0),
            // A followed view between blocks: its offset counts.
            Camera::following((1536.0, 768.0), Vec2::new(301.3, 120.9), (1280.0, 688.0), 1.0, 1.5),
            Camera::following((1536.0, 768.0), Vec2::new(101.0, 49.0), (800.0, 520.0), 1.36, 2.0),
        ];
        for camera in cameras {
            for world in [Vec2::new(900.0, 400.0), Vec2::new(camera.rect().x, camera.rect().y)] {
                let drawn = through(camera.on_field(field_origin), world);
                let view = camera.to_view(world);
                let want = Vec2::new(field_origin.x + view.x, field_origin.y + view.y);
                assert!((drawn.x - want.x).abs() < 1e-3 && (drawn.y - want.y).abs() < 1e-3, "{camera:?}: {drawn:?} vs {want:?}");
            }
        }
    }
}
