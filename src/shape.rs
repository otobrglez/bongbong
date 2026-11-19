//! What a tank is to whatever is trying to hit it (`TargetShape`): its hull
//! box and its turret box, and the three questions every weapon asks of
//! them - how far a ring has to reach (`reach_from`), whether a cone takes
//! it in (`in_cone`), and where a moving thing enters it (`swept`). Also
//! the segment-against-box test all of that, and `simulation::hits`,
//! stand on.

use crate::math::Vec2;
use crate::Position;

/// If the segment `p0..p1` passes through the axis-aligned box at `center`
/// with half-extents `half`, the parametric time `t` (`0..=1`) at which it
/// first enters - `None` if it never does. Slab clipping per axis;
/// `t_enter` starts at `0.0`, so a segment that begins inside the box
/// reports an immediate hit rather than a negative time, and a zero-length
/// segment degenerates to a point-in-box test.
pub fn segment_hits_aabb(p0: Position, p1: Position, center: Position, half: Position) -> Option<f32> {
    let d = Position::new(p1.x - p0.x, p1.y - p0.y);
    let mut t_enter = 0.0f32;
    let mut t_exit = 1.0f32;
    for axis in 0..2 {
        let (p0a, da, min_b, max_b) = if axis == 0 {
            (p0.x, d.x, center.x - half.x, center.x + half.x)
        } else {
            (p0.y, d.y, center.y - half.y, center.y + half.y)
        };
        if da.abs() < f32::EPSILON {
            if p0a < min_b || p0a > max_b {
                return None;
            }
        } else {
            let inv_d = 1.0 / da;
            let (mut t1, mut t2) = ((min_b - p0a) * inv_d, (max_b - p0a) * inv_d);
            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }
            t_enter = t_enter.max(t1);
            t_exit = t_exit.min(t2);
            if t_enter > t_exit {
                return None;
            }
        }
    }
    Some(t_enter)
}

/// How far from `origin` the nearest point of the box `center` +- `half`
/// lies: where a ring from `origin` reaches it. 0 inside the box.
pub fn box_reach(origin: Position, center: Position, half: Vec2) -> f32 {
    let dx = ((origin.x - center.x).abs() - half.x).max(0.0);
    let dy = ((origin.y - center.y).abs() - half.y).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

/// One tank as a target (`Tank::target_shape`): its hull box and its
/// turret-and-barrel box, each a centre and half-extents in the world.
///
/// - Something that **travels** - a shell, a bullet, a bolt, a beam, a
///   slug, a flame's ray - hits either box (`swept`).
/// - Something that **spreads from a point** - a blast, a rod's circle, an
///   EMP's ring, a well's collapse - reaches the hull at its nearest point
///   (`reach_from`); the barrel is not a place to be caught by.
/// - A **cone** - a sonic hammer's wave - takes the hull in by its centre
///   and corners (`in_cone`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TargetShape {
    pub hull: (Position, Vec2),
    pub turret: (Position, Vec2),
}

impl TargetShape {
    /// How far a ring from `origin` has to reach to touch the hull: the
    /// distance to its nearest point, 0 from inside it.
    pub fn reach_from(&self, origin: Position) -> f32 {
        box_reach(origin, self.hull.0, self.hull.1)
    }

    /// The hull's centre and four corners: the points a cone takes it by.
    pub fn points(&self) -> [Position; 5] {
        let (c, h) = self.hull;
        [c, Position::new(c.x - h.x, c.y - h.y), Position::new(c.x + h.x, c.y - h.y), Position::new(c.x - h.x, c.y + h.y), Position::new(c.x + h.x, c.y + h.y)]
    }

    /// Whether a cone takes the hull in, and how far out: `reaches` is the
    /// cone's own test of one point (its distance along the cone, `None`
    /// outside it or behind what stops it), and the answer is the nearest
    /// of the hull's `points` it reaches.
    pub fn in_cone(&self, reaches: impl Fn(Position) -> Option<f32>) -> Option<f32> {
        self.points().into_iter().filter_map(reaches).min_by(f32::total_cmp)
    }

    /// Where the segment `p0..p1`, `pad` wide on each side, first enters
    /// the hull or the turret (`segment_hits_aabb`): `t` in `0..=1`.
    pub fn swept(&self, p0: Position, p1: Position, pad: Vec2) -> Option<f32> {
        let hull = segment_hits_aabb(p0, p1, self.hull.0, self.hull.1 + pad);
        let turret = segment_hits_aabb(p0, p1, self.turret.0, self.turret.1 + pad);
        match (hull, turret) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape() -> TargetShape {
        TargetShape { hull: (Position::new(100.0, 100.0), Vec2::new(10.0, 16.0)), turret: (Position::new(100.0, 86.0), Vec2::new(3.0, 14.0)) }
    }

    #[test]
    fn a_ring_reaches_the_nearest_point_of_the_hull_and_never_the_barrel() {
        let s = shape();
        assert_eq!(s.reach_from(Position::new(100.0, 100.0)), 0.0, "inside");
        assert_eq!(s.reach_from(Position::new(140.0, 100.0)), 30.0, "beside");
        assert_eq!(s.reach_from(Position::new(100.0, 50.0)), 34.0, "past the barrel, to the hull");
    }

    #[test]
    fn a_segment_enters_the_turret_before_the_hull_from_the_front() {
        let s = shape();
        let t = s.swept(Position::new(100.0, 0.0), Position::new(100.0, 200.0), Vec2::zero()).expect("it hits");
        assert!((t - 0.36).abs() < 1e-4, "the barrel's tip at y 72: {t}");
        assert_eq!(s.swept(Position::new(0.0, 60.0), Position::new(200.0, 60.0), Vec2::zero()), None, "past the barrel's tip");
        assert!(s.swept(Position::new(0.0, 60.0), Position::new(200.0, 60.0), Vec2::new(0.0, 14.0)).is_some(), "the pad widens it");
    }

    #[test]
    fn a_cone_takes_the_nearest_point_it_reaches() {
        let s = shape();
        // A cone that reaches only what lies right of x = 105, by its x.
        let d = s.in_cone(|p| (p.x > 105.0).then_some(p.x));
        assert_eq!(d, Some(110.0), "a right-hand corner");
        assert_eq!(s.in_cone(|_| None), None);
    }
}
