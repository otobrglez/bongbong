//! The damage a tank wears (docs/effects.md), drawn in the effects
//! language (`pyro.rs`) over the tank sheet's own damaged hulls
//! (`Tank::hull_col`: the damage tiers, the wrecks) rather than stuck on
//! from a sheet of its own.
//!
//! - **Marks**: as the hull loses health its armour gathers soot, bare
//!   metal where the paint was scraped off and dents with a lit lip, each
//!   mark showing up at its own share of the damage. They sit on the hull
//!   and turn with it, under the turret (`game.rs` draws them between the
//!   two), so a mark never floats over a turret that swings across it.
//! - **The wound**: past `WOUND_AT` a hole opens on the engine deck behind
//!   the turret, an ember glowing in it; the particle layer puts smoke up
//!   off it (`fx.rs`), thicker and darker the worse the hull gets.
//! - **Fire**: past `BURNS_AT` (the critical tier), or with afterburn on it,
//!   the deck burns - tongues of flame (`pyro::tongues`) standing on the
//!   wound, leaning with the wind; a wreck burns hard over its whole hulk,
//!   dying down over the end of `wreck_burn_seconds`.
//!
//! Every mark's place, kind and the damage it shows up at hash from the
//! tank's slot and its rolled `damage_variant`, so the same tank always
//! wears the same scars, a replica draws them where the room's round does
//! and nothing draws RNG.

use crate::math::Vec2;
use crate::pyro::{self, Blocks, Shape, BLOCK, CHAR, FIRE, SMOKE};
use crate::tank::Tank;
use crate::tuning::tuning;
use crate::{Position, MAX_DAMAGE, TANK_DAMAGE_TIERS};

/// How many marks a hull can gather.
const MARKS: u32 = 9;

/// The damage a live deck catches fire at: the critical tier, the last of
/// `TANK_DAMAGE_TIERS`, where the art's breaches glow.
pub const BURNS_AT: f32 = TANK_DAMAGE_TIERS[TANK_DAMAGE_TIERS.len() - 1];

/// The share of `MAX_DAMAGE` the wound on the deck opens at.
pub const WOUND_AT: f32 = 0.45;

/// This tank's hashed layout: its slot and its rolled variant.
fn seed(tank: &Tank) -> u32 {
    crate::blast::seed_at(Position::new(tank.owner_slot() as f32 * 32.0, tank.damage_variant as f32 * 32.0), 71)
}

/// The hull's own axes on the field, from its drawn facing: to its right
/// and to its front.
fn axes(tank: &Tank) -> (Vec2, Vec2) {
    let (s, c) = tank.visual_rotation.to_radians().sin_cos();
    (Vec2::new(c, s), Vec2::new(s, -c))
}

/// The point `x` px to the hull's right and `y` px toward its front of
/// its middle.
fn on_hull(tank: &Tank, x: f32, y: f32) -> Position {
    let (right, front) = axes(tank);
    Position::new(tank.position.x + right.x * x + front.x * y, tank.position.y + right.y * x + front.y * y)
}

/// How much of the hull is gone, 0 pristine to 1 a wreck.
fn wear(tank: &Tank) -> f32 {
    (tank.damage / MAX_DAMAGE).clamp(0.0, 1.0)
}

/// Where the damage gathers: a spot on the engine deck behind the turret,
/// hashed across the deck per tank. The wound, the deck fire and the
/// smoke all start here.
pub fn wound(tank: &Tank) -> Position {
    let s = seed(tank);
    let (hw, hh) = tank.hull_half_extents(false);
    on_hull(tank, (pyro::unit(s, 1) - 0.5) * hw * 0.8, -hh * (0.42 + 0.2 * pyro::unit(s, 2)))
}

/// Whether the wound is open: the hull has lost `WOUND_AT` of its health
/// and is not yet a wreck (whose art is all wound).
pub fn wounded(tank: &Tank) -> bool {
    !tank.is_wreck() && wear(tank) >= WOUND_AT
}

/// The marks and the wound, over the hull and under the turret: nothing
/// on a pristine tank or a wreck.
pub fn draw_damage(b: &mut impl Blocks, tank: &Tank, time: f32) {
    let k = wear(tank);
    if k <= 0.0 || tank.is_wreck() {
        return;
    }
    let s = seed(tank);
    let u = |salt: u32| pyro::unit(s, salt);
    let (hw, hh) = tank.hull_half_extents(false);
    let (_, front) = axes(tank);
    let a = tank.alpha();
    for i in 0..MARKS {
        // Spread over the damage so marks come one or two to a hit.
        let shows_at = 0.06 + 0.84 * (i as f32 + u(10 + i)) / MARKS as f32;
        if k < shows_at {
            continue;
        }
        let at = on_hull(tank, (u(20 + i) - 0.5) * 2.0 * hw * 0.72, (u(30 + i) - 0.5) * 2.0 * hh * 0.78);
        match i % 3 {
            // Soot: a dark patch, dense in the middle and dissolving out.
            0 => {
                let r = 3.0 + 2.5 * u(40 + i);
                pyro::dither_disc(b, at, r, pyro::alpha(CHAR[1], 0.7 * a), 0.55);
                pyro::dither_disc(b, at, r * 0.55, pyro::alpha(CHAR[0], 0.85 * a), 0.9);
            }
            // A scrape to bare metal, two or three blocks along the hull
            // or across it.
            1 => {
                let len = BLOCK * (1.0 + (u(40 + i) * 2.0).floor());
                let way = if u(50 + i) < 0.6 { front } else { Vec2::new(-front.y, front.x) };
                let to = Position::new(at.x + way.x * len, at.y + way.y * len);
                pyro::block_line(b, at, to, 1, |_| pyro::alpha(SMOKE[5], 0.9 * a));
            }
            // A dent: a dark pit with its upper lip catching the light.
            _ => {
                pyro::mark(b, at, 2, pyro::alpha(CHAR[0], 0.9 * a));
                pyro::mark(b, Position::new(at.x - BLOCK, at.y - BLOCK), 2, pyro::alpha(SMOKE[6], 0.7 * a));
            }
        }
    }
    if wounded(tank) {
        let w = wound(tank);
        // A scorched ring round a hole, an ember glowing in it that
        // flickers through three steps of the fire ramp.
        pyro::dither_disc(b, w, 5.0, pyro::alpha(CHAR[1], 0.75 * a), 0.6);
        pyro::mark(b, w, 4, pyro::alpha(CHAR[0], a));
        let beat = ((time * 9.0 + u(3) * 9.0) as i32).rem_euclid(4);
        let ember = [FIRE[3], FIRE[5], FIRE[3], FIRE[2]][beat as usize];
        pyro::mark(b, w, 2, pyro::alpha(ember, a));
    }
}

/// How hard the tank burns, 0 (not at all) to 1: a wreck until its fire
/// dies down over the last fifth of `wreck_burn_seconds`; a live hull past
/// `BURNS_AT`, harder toward the end; afterburn from a flamethrower until
/// it runs out.
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

/// Where a burning tank's fire stands: the wound on a live hull, the
/// middle of a wreck.
pub fn fire_at(tank: &Tank) -> Position {
    if tank.is_wreck() {
        tank.position
    } else {
        wound(tank)
    }
}

/// The flames on a burning tank, back to front, `lean` px sideways per px
/// of height (`pyro::smoke_lean`): a couple of tongues on the deck, three
/// big ones over a wreck, and the light they throw (drawn in the additive
/// pass). Empty for a tank that is not burning.
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

    fn marks(t: &Tank) -> Vec<(i32, i32, i32, i32, Color)> {
        let mut r = Rects::default();
        draw_damage(&mut r, t, 0.0);
        r.0
    }

    #[test]
    fn a_pristine_hull_wears_nothing_and_marks_gather_with_damage() {
        assert!(marks(&tank(0.0)).is_empty());
        let light = marks(&tank(25.0)).len();
        let heavy = marks(&tank(70.0)).len();
        assert!(light > 0, "a hull a quarter gone shows it");
        assert!(heavy > light, "more damage, more marks: {light} then {heavy}");
    }

    #[test]
    fn the_marks_turn_with_the_hull() {
        let mut up = tank(60.0);
        let mut right = tank(60.0);
        up.visual_rotation = 0.0;
        right.visual_rotation = 90.0;
        let w_up = wound(&up) - up.position;
        let w_right = wound(&right) - right.position;
        // A quarter turn clockwise takes (x, y) to (-y, x).
        assert!((w_right.x + w_up.y).abs() < 0.01 && (w_right.y - w_up.x).abs() < 0.01, "{w_up:?} turned is {w_right:?}");
        // Behind the turret: toward the rear of a hull facing up.
        assert!(w_up.y > 0.0);
    }

    #[test]
    fn the_same_tank_always_wears_the_same_scars() {
        assert_eq!(marks(&tank(80.0)), marks(&tank(80.0)));
        let mut other = tank(80.0);
        other.damage_variant = 3;
        assert_ne!(marks(&tank(80.0)), marks(&other), "another variant, another layout");
    }

    #[test]
    fn a_hull_burns_from_the_disabled_stage_and_a_wreck_burns_out() {
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
    fn every_mark_is_a_ramp_step() {
        let ramp: Vec<Color> = [&CHAR[..], &SMOKE[..], &FIRE[..]].concat();
        for d in [20.0, 50.0, 95.0] {
            for (.., c) in marks(&tank(d)) {
                assert!(ramp.iter().any(|r| r.r == c.r && r.g == c.g && r.b == c.b), "{c:?}");
            }
        }
    }
}
