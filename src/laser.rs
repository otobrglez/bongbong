//! The laser: a limited-charge, instant-hit weapon a tank gains from a
//! `pickup::PickupKind::Laser` pickup (see `pickup.rs`, `Tank::laser_charges`,
//! `Tank::laser_variant`). Unlike `Shell` there's no travel time or
//! sprite-sheet animation to animate through - firing resolves the hit the
//! same frame (see `simulation::weapons` and `Game::resolve_lasers`, which
//! reuse `hits::Terrain::sweep`'s segment test), and this type is
//! purely the resulting on-screen flash: a short-lived line from muzzle to
//! whatever it hit, ticked down and dropped once its display window elapses.

use sola_raylib::prelude::*;

use crate::{LASER_BEAM_DISPLAY_SECONDS, LASER_BEAM_WIDTH, LASER_BLUE_DAMAGE_FACTOR, Position};

/// Which of the two laser variants a charge batch is - rolled once per
/// `PickupKind::Laser` pickup (see `LASER_BLUE_PICKUP_CHANCE`) and carried on
/// `Tank::laser_variant` until the next pickup rerolls it. Red is the
/// original, baseline-damage laser; Blue is `LASER_BLUE_DAMAGE_FACTOR` times
/// stronger, and reads as visibly different in flight so the extra power is
/// legible without reading the HUD.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LaserVariant {
    Red,
    Blue,
}

impl LaserVariant {
    /// Multiplier on `LASER_DAMAGE_MIN/MAX` this variant fires at.
    pub fn damage_factor(self) -> f32 {
        match self {
            LaserVariant::Red => 1.0,
            LaserVariant::Blue => LASER_BLUE_DAMAGE_FACTOR,
        }
    }

    /// (glow, core) colors `draw_laser_beam` renders this variant's beam
    /// with, at full alpha - scaled down by the beam's fade-out separately.
    fn colors(self) -> (Color, Color) {
        match self {
            LaserVariant::Red => (Color::new(255, 40, 40, 200), Color::new(255, 220, 220, 255)),
            LaserVariant::Blue => (Color::new(40, 130, 255, 200), Color::new(220, 235, 255, 255)),
        }
    }
}

pub struct LaserBeam {
    pub start: Position,
    pub end: Position,
    pub variant: LaserVariant,
    /// Seconds remaining before this beam is dropped - counts down from
    /// LASER_BEAM_DISPLAY_SECONDS, also used to fade its alpha in `draw_laser_beam`.
    pub timer: f32,
}

impl LaserBeam {
    pub fn new(start: Position, end: Position, variant: LaserVariant) -> Self {
        LaserBeam {
            start,
            end,
            variant,
            timer: LASER_BEAM_DISPLAY_SECONDS,
        }
    }

    /// Age this beam by `dt`; returns true once its display window has
    /// elapsed, so the caller knows to drop it (see `Vec::retain_mut`'s use
    /// in `Game::update`).
    pub fn tick(&mut self, dt: f32) -> bool {
        self.timer -= dt;
        self.timer <= 0.0
    }
}

/// Draw one laser beam as a bright line from muzzle to impact point, fading
/// out over its remaining `timer`. Two overlapping passes - a wider dim
/// glow, a thinner bright core, colored by `beam.variant` (see
/// `LaserVariant::colors`) - rather than a sprite, since an instant beam has
/// no frames to animate through.
pub fn draw_laser_beam(d: &mut impl RaylibDraw, beam: &LaserBeam) {
    let alpha = (beam.timer / LASER_BEAM_DISPLAY_SECONDS).clamp(0.0, 1.0);
    let (glow, core) = beam.variant.colors();
    let glow = Color::new(glow.r, glow.g, glow.b, (glow.a as f32 * alpha) as u8);
    let core = Color::new(core.r, core.g, core.b, (core.a as f32 * alpha) as u8);
    d.draw_line_ex(beam.start, beam.end, LASER_BEAM_WIDTH, glow);
    d.draw_line_ex(beam.start, beam.end, LASER_BEAM_WIDTH * 0.4, core);
}
