//! The laser: a limited-charge, instant-hit weapon a tank gains from a
//! `pickup::PickupKind::Laser` pickup (see `pickup.rs`, `Tank::laser_charges`,
//! `Tank::laser_variant`). Unlike `Shell` there's no travel time or
//! sprite-sheet animation to animate through - firing resolves the hit the
//! same frame (see `simulation::weapons` and `Game::resolve_lasers`, which
//! reuse `hits::Terrain::sweep`'s segment test), and this type is
//! purely the resulting on-screen flash: a short-lived line from muzzle to
//! whatever it hit, ticked down and dropped once its display window elapses.

use crate::tuning::tuning;

use crate::{Position};

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
            LaserVariant::Blue => tuning().laser_blue_damage_factor,
        }
    }

}

pub struct LaserBeam {
    pub start: Position,
    pub end: Position,
    pub variant: LaserVariant,
    /// Seconds remaining before this beam is dropped - counts down from
    /// LASER_BEAM_DISPLAY_SECONDS, also used to fade its alpha in `render::laser::draw_laser_beam`.
    pub timer: f32,
}

impl LaserBeam {
    pub fn new(start: Position, end: Position, variant: LaserVariant) -> Self {
        LaserBeam {
            start,
            end,
            variant,
            timer: tuning().laser_beam_display_seconds,
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
