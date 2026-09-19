//! The shadow under a piece of rubble still in the air (`decal.rs` owns
//! the decal and its `Canvas` draw).

use sola_raylib::prelude::*;

use crate::decal::Decal;
use crate::math::Color;
use crate::tuning::tuning;
use crate::{Position, OBSTACLE_TEXTURE_SIZE};

/// The shadow under a piece still in the air, drawn at the point on the
/// ground it is over. Shrinks as the piece rises, which is what actually
/// sells the height in a game with no camera.
pub fn draw_decal_shadow(d: &mut impl RaylibDraw, decal: &Decal) {
    let h = decal.height();
    if h <= 0.0 {
        return;
    }
    let t = decal.flight();
    let ground = Position::new(
        decal.origin.x + (decal.center.x - decal.origin.x) * t,
        decal.origin.y + (decal.center.y - decal.origin.y) * t,
    );
    let lift = (h / tuning().debris_arc_height.max(1e-3)).clamp(0.0, 1.0);
    let r = OBSTACLE_TEXTURE_SIZE * 0.22 * (1.0 - 0.45 * lift);
    let a = (255.0 * tuning().obstacle_shadow_opacity * (1.0 - 0.4 * lift)) as u8;
    d.draw_circle_v(ground, r, Color::new(0, 0, 0, a));
}
