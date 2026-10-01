//! Drawing a laser beam (`laser.rs` owns the beam and its variants).

use sola_raylib::prelude::*;

use crate::laser::{LaserBeam, LaserVariant};
use crate::math::{Color, Vec2};
use crate::pyro;
use crate::render::pyro::Rl;
use crate::render::shot_fx::{fade, glow as soft};
use crate::tuning::tuning;
use crate::Position;

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

/// Draw one laser beam as a bright line of blocks from muzzle to impact
/// point, fading out over its remaining `timer`. Two overlapping passes - a
/// wider dim glow, a one-block bright core, colored by `beam.variant` (see
/// `LaserVariant::colors`) - rather than a sprite, since an instant beam has
/// no frames to animate through.
pub fn draw_laser_beam(d: &mut impl RaylibDraw, beam: &LaserBeam) {
    let alpha = (beam.timer / tuning().laser_beam_display_seconds).clamp(0.0, 1.0);
    let (glow, core) = beam.variant.colors();
    let glow = Color::new(glow.r, glow.g, glow.b, (glow.a as f32 * alpha) as u8);
    let core = Color::new(core.r, core.g, core.b, (core.a as f32 * alpha) as u8);
    let blocks = (tuning().laser_beam_width / pyro::BLOCK).round().max(1.0) as i32;
    pyro::block_line(&mut Rl(d), beam.start, beam.end, blocks, |_| glow);
    pyro::block_line(&mut Rl(d), beam.start, beam.end, 1, |_| core);
}

/// How bright the beam's light is this frame, 0..=1: its fade-out times
/// a fast flicker off its own timer, so a held beam buzzes rather than
/// glowing flat.
fn light(beam: &LaserBeam) -> f32 {
    let alpha = (beam.timer / tuning().laser_beam_display_seconds).clamp(0.0, 1.0);
    let flicker = 0.8 + 0.2 * (beam.timer * tuning().laser_flicker_hz * std::f32::consts::TAU).sin();
    alpha * flicker * tuning().shot_glow_strength
}

/// The bloom around a beam (additive, drawn under it): a wide, faint band
/// in the beam's colour, plus bright packets of light racing from the
/// muzzle to the far end.
pub fn draw_laser_bloom(d: &mut impl RaylibDraw, beam: &LaserBeam) {
    let k = light(beam);
    if k <= 0.0 {
        return;
    }
    let (glow, core) = beam.variant.colors();
    let width = tuning().laser_beam_width;
    let blocks = |w: f32| (w / pyro::BLOCK).round().max(1.0) as i32;
    pyro::block_line(&mut Rl(d), beam.start, beam.end, blocks(width * 3.5), |_| fade(glow, 0.45 * k));
    pyro::block_line(&mut Rl(d), beam.start, beam.end, blocks(width * 1.8), |_| fade(glow, 0.7 * k));
    let span = Vec2::new(beam.end.x - beam.start.x, beam.end.y - beam.start.y);
    let length = span.length();
    if length < 1.0 {
        return;
    }
    // Packets every 40 px, running the beam's length at 900 px/s.
    let offset = (beam.timer * 900.0) % 40.0;
    let mut along = 40.0 - offset;
    while along < length {
        let t = along / length;
        let at = Position::new(beam.start.x + span.x * t, beam.start.y + span.y * t);
        soft(d, at, width * 1.4, fade(core, 0.8 * k));
        along += 40.0;
    }
}

/// The two ends of a beam (additive, drawn over it): a hot lens at the
/// emitter and a bigger, whiter burn where the beam stops, both pulsing
/// with the flicker.
pub fn draw_laser_flares(d: &mut impl RaylibDraw, beam: &LaserBeam) {
    let k = light(beam);
    if k <= 0.0 {
        return;
    }
    let (glow, core) = beam.variant.colors();
    let width = tuning().laser_beam_width;
    soft(d, beam.start, width * 3.0, fade(glow, 0.8 * k));
    soft(d, beam.start, width * 1.4, fade(core, k));
    soft(d, beam.end, width * 5.0, fade(glow, 0.7 * k));
    soft(d, beam.end, width * 2.2, fade(core, k));
    pyro::block_disc(&mut Rl(d), beam.end, width * 0.6, fade(Color::WHITE, k));
}
