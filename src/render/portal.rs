//! The additive glow under an active portal (`portal.rs` owns the frames
//! and the `Canvas` draw).

use sola_raylib::prelude::*;

use crate::blast::seed_at;
use crate::math::Color;
use crate::render::blast::pixel_disc;
use crate::tuning::tuning;
use crate::Position;

/// The P1 team blues (`tools/punypalette.py`'s `TEAM_P1`, `tank::TEAM_COLORS[0]`):
/// deliberately off the ground palette, so a hole in the ground reads as
/// not-terrain the way a player's ring does - docs/PALETTE.md.
const PORTAL_MID: Color = Color::new(0x4D, 0x65, 0xB4, 255);
const PORTAL_LIGHT: Color = Color::new(0x8F, 0xD3, 0xFF, 255);

/// The additive glow under an active portal: a wide dim disc and a small
/// bright one, breathing slowly. `portal_glow_strength` 0 draws nothing.
pub fn draw_portal_glow(d: &mut impl RaylibDraw, center: Position, time: f32) {
    let strength = tuning().portal_glow_strength;
    if strength <= 0.0 {
        return;
    }
    let breathe = 0.85 + 0.15 * (time * 0.8 + seed_at(center, 102) as f32 * 0.01).sin();
    let alpha = |base: f32| (base * strength * breathe * 255.0).round().clamp(0.0, 255.0) as u8;
    pixel_disc(d, center, 40.0, Color::new(PORTAL_MID.r, PORTAL_MID.g, PORTAL_MID.b, alpha(0.6)));
    pixel_disc(d, center, 16.0, Color::new(PORTAL_LIGHT.r, PORTAL_LIGHT.g, PORTAL_LIGHT.b, alpha(0.9)));
}
