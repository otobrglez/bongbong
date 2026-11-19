//! A shove's numbers (`Knock`) and the one mass rule every shove follows:
//! a shell's impact, a blast, a ram, a firing recoil, a sonic hammer's
//! wave, a rod's ring, a gravity well's fling, a gauss rail's kick. The
//! world half - the impulse, the skid, telling a client-owned hull - is
//! `simulation::combat::knock`.

use crate::math;
use crate::tuning::Tuning;

/// The lightest mass factor the rule divides by, so a mistuned chassis
/// cannot be thrown off the field.
const MASS_FACTOR_FLOOR: f32 = 0.05;

/// One shove: how fast, how a chassis's mass resists it, and whether it
/// takes the hull off its tracks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knock {
    /// The speed it puts on the standard chassis, px/s.
    pub speed: f32,
    /// How mass resists: the speed is divided by the chassis's mass factor
    /// (`Tank::mass_factor`) to this power. 0 shoves every chassis alike,
    /// 1 is momentum's rule, more makes the heavy ones stand firmer.
    pub mass_exponent: f32,
    /// The most it comes to on any chassis, px/s.
    pub max_speed: f32,
    /// The longest it takes the hull off its tracks, in seconds
    /// (`Tank::skid`): the hull slides until the skid's friction stops it
    /// or this runs out. 0 leaves it driving, and the drive takes the
    /// shove back within a frame or two.
    pub skid: f32,
}

impl Knock {
    /// The speed this puts on a chassis of `mass_factor`, px/s: **the one
    /// mass rule**.
    pub fn velocity(&self, mass_factor: f32) -> f32 {
        (self.speed / math::powf(mass_factor.max(MASS_FACTOR_FLOOR), self.mass_exponent)).min(self.max_speed)
    }

    /// A blast's shove at `speed` (already fallen off with distance): a
    /// short skid, so the hull is seen to be thrown.
    pub fn blast(t: &Tuning, speed: f32) -> Self {
        Knock { speed, mass_exponent: t.blast_knock_mass_exponent, max_speed: t.blast_knock_max_speed, skid: t.blast_skid_max_seconds }
    }

    /// A shell's or a bolt's impact: a nudge, no skid.
    pub fn impact(t: &Tuning, speed: f32) -> Self {
        Knock { speed, mass_exponent: t.impact_knock_mass_exponent, max_speed: t.knockback_max_speed, skid: 0.0 }
    }

    /// One hull's share of a ram, already split between the two by their
    /// masses: no further mass rule, no skid.
    pub fn ram(t: &Tuning, speed: f32) -> Self {
        Knock { speed, mass_exponent: 0.0, max_speed: t.knockback_max_speed, skid: 0.0 }
    }

    /// A weapon's recoil on the hull that fired it: momentum's rule, its
    /// own cap, no skid.
    pub fn recoil(speed: f32, max_speed: f32) -> Self {
        Knock { speed, mass_exponent: 1.0, max_speed, skid: 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standard_chassis_takes_the_speed_and_mass_resists_by_the_exponent() {
        let k = Knock { speed: 100.0, mass_exponent: 1.0, max_speed: 500.0, skid: 0.0 };
        assert_eq!(k.velocity(1.0), 100.0);
        assert_eq!(k.velocity(2.0), 50.0);
        assert!(k.velocity(0.5) > 100.0, "a light chassis flies");
        let blind = Knock { mass_exponent: 0.0, ..k };
        assert_eq!(blind.velocity(2.0), 100.0, "exponent 0 shoves every chassis alike");
        let firm = Knock { mass_exponent: 2.0, ..k };
        assert_eq!(firm.velocity(2.0), 25.0);
    }

    #[test]
    fn it_is_capped_and_a_weightless_chassis_is_floored() {
        let k = Knock { speed: 400.0, mass_exponent: 1.0, max_speed: 300.0, skid: 0.0 };
        assert_eq!(k.velocity(1.0), 300.0);
        assert_eq!(k.velocity(0.0), 300.0);
        assert!(Knock { max_speed: f32::INFINITY, ..k }.velocity(0.0).is_finite());
    }
}
