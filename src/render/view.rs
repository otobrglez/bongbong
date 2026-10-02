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
    rl.draw(thread, |mut d| present_into(&mut d, composite, view, backdrop));
}

/// `present` into the frame `d` is drawing, so what stands on the window -
/// the HUD's corners, its dialogs - can follow it onto the same frame.
pub fn present_into(d: &mut impl RaylibDraw, composite: &RenderTexture2D, view: &View, backdrop: Color) {
    // A render texture reads back bottom-up; a negative source height
    // flips it on the way out.
    let source = Rectangle::new(0.0, 0.0, view.bitmap.0, -view.bitmap.1);
    let dest = view.dest();
    d.clear_background(Color::BLACK);
    if !view.is_identity() {
        d.draw_rectangle(0, 0, view.window.0 as i32, view.window.1 as i32, backdrop);
        d.draw_rectangle_lines_ex(Rectangle::new(dest.x - 1.0, dest.y - 1.0, dest.width + 2.0, dest.height + 2.0), 1.0, FRAME);
    }
    d.draw_texture_pro(composite, source, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
}

/// Put a followed view's world on the window (docs/large-maps-follow-camera.md
/// §6): `world` holds the scene target's every texel, from the camera's
/// block-grid `origin`, and the view's own corner is `camera.offset`
/// further on, so the texels from there fill the field area of the bitmap
/// as `view` puts it on the window - the sub-block motion the grid cannot
/// hold, applied here as a shift of whole device pixels
/// (`Camera::following` rounded it so), which keeps every block whole.
/// The window is cleared to `backdrop` first, for the bars of a screen
/// past the aspect clamp. Drawn into the frame `d` is drawing; the bar and
/// what stands over the field come after it, onto the same frame.
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
