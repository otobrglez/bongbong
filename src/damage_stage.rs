//! What a damaged tank gives off (docs/effects.md). The tank sheet carries
//! the wear itself - its damage tiers (`Tank::damage_tier`,
//! `TANK_DAMAGE_TIERS`: scuffed, damaged, critical) and its wrecks
//! (docs/SPRITESHEET_SPEC.md) - and this adds what moves, in the effects
//! language (`pyro.rs`), one step per tier:
//!
//! - **Scuffed**: the art alone.
//! - **Damaged** (`SMOKES_AT`): smoke rises off the engine deck behind the
//!   turret (`smoke`, `engine_deck`; the particle layer puts it up,
//!   `fx.rs`), thicker the worse the hull gets.
//! - **Critical** (`BURNS_AT`), or with afterburn on it: the deck burns -
//!   tongues of flame (`pyro::tongues`) standing on it, leaning with the
//!   wind - and the smoke turns black. A wreck burns hard over its whole
//!   hulk, dying down over the end of `wreck_burn_seconds`.
//!
//! Where on the deck a tank smokes and burns hashes from its slot and its
//! rolled `damage_variant`, so the same tank always burns in the same
//! place, a replica draws it where the room's round does and nothing draws
//! RNG.

use crate::math::Vec2;
use crate::pyro::{self, Shape, FIRE};
use crate::tank::Tank;
use crate::tuning::tuning;
use crate::{Position, MAX_DAMAGE, TANK_DAMAGE_TIERS};

/// The damage a hull starts smoking at: the damaged tier, where the
/// art's first wound sparks.
pub const SMOKES_AT: f32 = TANK_DAMAGE_TIERS[1];

/// The damage a live deck catches fire at: the critical tier, the last of
/// `TANK_DAMAGE_TIERS`, where the art's breaches glow.
pub const BURNS_AT: f32 = TANK_DAMAGE_TIERS[TANK_DAMAGE_TIERS.len() - 1];

/// This tank's hashed spot: its slot and its rolled variant.
fn seed(tank: &Tank) -> u32 {
    crate::blast::seed_at(Position::new(tank.owner_slot() as f32 * 32.0, tank.damage_variant as f32 * 32.0), 71)
}

/// The point `x` px to the hull's right and `y` px toward its front of
/// its middle, by its drawn facing.
fn on_hull(tank: &Tank, x: f32, y: f32) -> Position {
    let (s, c) = tank.visual_rotation.to_radians().sin_cos();
    let (right, front) = (Vec2::new(c, s), Vec2::new(s, -c));
    Position::new(tank.position.x + right.x * x + front.x * y, tank.position.y + right.y * x + front.y * y)
}

/// Where a damaged tank smokes and burns: a spot on the engine deck behind
/// the turret, hashed across the deck per tank.
pub fn engine_deck(tank: &Tank) -> Position {
    let s = seed(tank);
    let (hw, hh) = tank.hull_half_extents(false);
    on_hull(tank, (pyro::unit(s, 1) - 0.5) * hw * 0.8, -hh * (0.42 + 0.2 * pyro::unit(s, 2)))
}

/// How thick the smoke off a live hull is, 0 (a wisp, at the damaged
/// tier) to 1 (a column, a hull on its last point); `None` for a hull not
/// yet damaged enough to smoke, and for a wreck, whose smoke is its fire's.
pub fn smoke(tank: &Tank) -> Option<f32> {
    if tank.is_wreck() || tank.damage < SMOKES_AT {
        return None;
    }
    Some(((tank.damage - SMOKES_AT) / (MAX_DAMAGE - SMOKES_AT).max(1.0)).clamp(0.0, 1.0))
}

/// How hard the tank burns, 0 (not at all) to 1: a wreck until its fire
/// dies down over the last fifth of `wreck_burn_seconds`; a live hull from
/// the critical tier, harder toward the end; afterburn from a flamethrower
/// until it runs out.
pub fn fire(tank: &Tank) -> f32 {
    if tank.is_wreck() {
        let burn = tuning().wreck_burn_seconds;
        if burn <= 0.0 {
            return 0.0;
        }
        let left = 1.0 - tank.wreck_timer / burn;
        return (left / 0.2).clamp(0.0, 1.0);
    }
    let hull = if tank.damage >= BURNS_AT {
        0.55 + 0.45 * ((tank.damage - BURNS_AT) / (MAX_DAMAGE - BURNS_AT).max(1.0)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let after = (tank.burn_timer / 0.6).clamp(0.0, 1.0);
    hull.max(after)
}

/// Where a burning tank's fire stands: the engine deck of a live hull, the
/// middle of a wreck.
pub fn fire_at(tank: &Tank) -> Position {
    if tank.is_wreck() {
        tank.position
    } else {
        engine_deck(tank)
    }
}

/// The flames on a burning tank, back to front, `lean` px sideways per px
/// of height (`pyro::smoke_lean`): a few tongues on the deck, bigger ones
/// over a wreck, and the light they throw (drawn in the additive pass).
/// Empty for a tank that is not burning.
pub fn flames(tank: &Tank, time: f32, lean: f32) -> Vec<Shape> {
    let strength = fire(tank) * tank.alpha();
    let mut out = Vec::new();
    if strength <= 0.0 {
        return out;
    }
    let s = seed(tank);
    let at = fire_at(tank);
    let size = tank.scale / 2.0;
    let (foot, spread, height, count, light) = if tank.is_wreck() {
        (Position::new(at.x, at.y + 6.0 * size), 24.0 * size, 34.0 * size, 4, 44.0)
    } else {
        (Position::new(at.x, at.y + 3.0), 10.0 * size, 22.0 * size, 3, 28.0)
    };
    pyro::tongues(&mut out, foot, spread, height, count, s, time, lean, strength);
    out.push(Shape::Glow { pos: Position::new(foot.x, foot.y - height * 0.35), radius: light * (0.5 + 0.5 * strength), color: pyro::alpha(FIRE[5], 0.22 * strength) });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Color;
    use crate::pyro::Blocks;

    #[derive(Default)]
    struct Rects(Vec<(i32, i32, i32, i32, Color)>);

    impl Blocks for Rects {
        fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Color) {
            self.0.push((x, y, w, h, c));
        }
    }

    fn tank(damage: f32) -> Tank {
        Tank { damage, position: Position::new(200.0, 200.0), ..Tank::default() }
    }

    #[test]
    fn a_hull_smokes_from_the_damaged_tier_and_thickens() {
        assert_eq!(smoke(&tank(TANK_DAMAGE_TIERS[0])), None, "a scuffed hull wears the art alone");
        assert_eq!(smoke(&tank(SMOKES_AT - 1.0)), None);
        assert_eq!(smoke(&tank(SMOKES_AT)), Some(0.0), "a wisp at the damaged tier");
        assert!(smoke(&tank(90.0)) > smoke(&tank(60.0)));
        assert_eq!(smoke(&tank(MAX_DAMAGE)), None, "a wreck's smoke is its fire's");
    }

    #[test]
    fn the_engine_deck_turns_with_the_hull() {
        let mut up = tank(60.0);
        let mut right = tank(60.0);
        up.visual_rotation = 0.0;
        right.visual_rotation = 90.0;
        let d_up = engine_deck(&up) - up.position;
        let d_right = engine_deck(&right) - right.position;
        // A quarter turn clockwise takes (x, y) to (-y, x).
        assert!((d_right.x + d_up.y).abs() < 0.01 && (d_right.y - d_up.x).abs() < 0.01, "{d_up:?} turned is {d_right:?}");
        // Behind the turret: toward the rear of a hull facing up.
        assert!(d_up.y > 0.0);
    }

    #[test]
    fn the_same_tank_always_burns_in_the_same_place() {
        assert_eq!(engine_deck(&tank(80.0)), engine_deck(&tank(80.0)));
        let mut other = tank(80.0);
        other.damage_variant = 3;
        assert_ne!(engine_deck(&tank(80.0)), engine_deck(&other), "another variant, another spot");
    }

    #[test]
    fn a_hull_burns_from_the_critical_tier_and_a_wreck_burns_out() {
        assert_eq!(fire(&tank(BURNS_AT - 1.0)), 0.0);
        assert!(fire(&tank(BURNS_AT)) > 0.0);
        assert!(!flames(&tank(90.0), 0.0, 0.0).is_empty());
        let mut wreck = tank(MAX_DAMAGE);
        assert_eq!(fire(&wreck), 1.0);
        wreck.wreck_timer = tuning().wreck_burn_seconds;
        assert_eq!(fire(&wreck), 0.0);
        assert!(flames(&wreck, 0.0, 0.0).is_empty());
        let mut scorched = tank(10.0);
        scorched.burn_timer = 1.0;
        assert!(fire(&scorched) > 0.0, "afterburn sets any hull alight");
    }

    #[test]
    fn the_flames_stand_on_the_deck_and_lean_with_the_wind() {
        let burning = tank(95.0);
        let paint = |lean| {
            let mut r = Rects::default();
            pyro::draw(&mut r, &flames(&burning, 0.4, lean));
            r.0
        };
        let (still, leaning) = (paint(0.0), paint(0.6));
        assert!(!still.is_empty());
        let mean_x = |rs: &[(i32, i32, i32, i32, Color)]| rs.iter().map(|r| r.0 as f32).sum::<f32>() / rs.len() as f32;
        assert!((mean_x(&still) - engine_deck(&burning).x).abs() < 8.0, "the fire stands on the deck");
        assert!(mean_x(&leaning) > mean_x(&still) + 2.0, "and leans down-wind");
        assert!(still.iter().all(|r| r.0 % 2 == 0 && r.1 % 2 == 0), "on the block grid");
    }
}
