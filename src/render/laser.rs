//! Drawing a laser beam (`laser.rs` owns the beam and its variants).

use sola_raylib::prelude::*;

use crate::laser::{LaserBeam, LaserVariant};
use crate::math::Color;
use crate::tuning::tuning;

impl LaserVariant {
    /// (glow, core) colors `draw_laser_beam` renders this variant's beam
    /// with, at full alpha - scaled down by the beam's fade-out separately.
    fn colors(self) -> (Color, Color) {
        match self {
            LaserVariant::Red => (Color::new(255, 40, 40, 200), Color::new(255, 220, 220, 255)),
            LaserVariant::Blue => (Color::new(40, 130, 255, 200), Color::new(220, 235, 255, 255)),
        }
    }
}

/// Draw one laser beam as a bright line from muzzle to impact point, fading
/// out over its remaining `timer`. Two overlapping passes - a wider dim
/// glow, a thinner bright core, colored by `beam.variant` (see
/// `LaserVariant::colors`) - rather than a sprite, since an instant beam has
/// no frames to animate through.
pub fn draw_laser_beam(d: &mut impl RaylibDraw, beam: &LaserBeam) {
    let alpha = (beam.timer / tuning().laser_beam_display_seconds).clamp(0.0, 1.0);
    let (glow, core) = beam.variant.colors();
    let glow = Color::new(glow.r, glow.g, glow.b, (glow.a as f32 * alpha) as u8);
    let core = Color::new(core.r, core.g, core.b, (core.a as f32 * alpha) as u8);
    d.draw_line_ex(beam.start, beam.end, tuning().laser_beam_width, glow);
    d.draw_line_ex(beam.start, beam.end, tuning().laser_beam_width * 0.4, core);
}
