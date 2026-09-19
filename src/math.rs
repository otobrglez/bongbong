//! `Vec2`, the simulation's one vector type: positions, velocities and
//! directions in field pixels (y down). It is the game's own so that
//! nothing under `simulation/` and no entity's state names a raylib type -
//! a headless round, the probe and a future server must build without the
//! drawing crate. The arithmetic is written out component by component in
//! the same order raylib's `Vector2` performs it (`length` is
//! `(x*x + y*y).sqrt()`, `distance_to` squares the differences before the
//! root, nothing goes through a reciprocal), because every seeded replay
//! and every recorded probe fixture depends on those exact bits. Only the
//! operations the round uses are here; add one the same way, never by
//! calling out to another vector library. The render boundary converts
//! with `From` (at the bottom) and nowhere else.

use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

/// A 2D vector of `f32`, `repr(C)` so it lays out like a `(x, y)` pair.
#[repr(C)]
#[derive(Default, Debug, Copy, Clone, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Vec2 {
        Vec2 { x, y }
    }

    pub const fn zero() -> Vec2 {
        Vec2::ZERO
    }

    /// `sqrt(x*x + y*y)`.
    pub fn length(&self) -> f32 {
        ((self.x * self.x) + (self.y * self.y)).sqrt()
    }

    /// `x*x + y*y`, no root.
    pub fn length_sqr(&self) -> f32 {
        (self.x * self.x) + (self.y * self.y)
    }

    pub fn dot(&self, v: Vec2) -> f32 {
        self.x * v.x + self.y * v.y
    }

    /// The distance to `v`: the differences squared, summed, then rooted.
    pub fn distance_to(&self, v: Vec2) -> f32 {
        ((self.x - v.x) * (self.x - v.x) + (self.y - v.y) * (self.y - v.y)).sqrt()
    }
}

impl From<(f32, f32)> for Vec2 {
    fn from((x, y): (f32, f32)) -> Vec2 {
        Vec2 { x, y }
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, v: Vec2) -> Vec2 {
        Vec2 { x: self.x + v.x, y: self.y + v.y }
    }
}

impl AddAssign for Vec2 {
    fn add_assign(&mut self, v: Vec2) {
        *self = *self + v;
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, v: Vec2) -> Vec2 {
        Vec2 { x: self.x - v.x, y: self.y - v.y }
    }
}

impl SubAssign for Vec2 {
    fn sub_assign(&mut self, v: Vec2) {
        *self = *self - v;
    }
}

impl Mul for Vec2 {
    type Output = Vec2;
    fn mul(self, v: Vec2) -> Vec2 {
        Vec2 { x: self.x * v.x, y: self.y * v.y }
    }
}

impl Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, value: f32) -> Vec2 {
        Vec2 { x: self.x * value, y: self.y * value }
    }
}

impl MulAssign<f32> for Vec2 {
    fn mul_assign(&mut self, value: f32) {
        *self = *self * value;
    }
}

impl Div<f32> for Vec2 {
    type Output = Vec2;
    fn div(self, value: f32) -> Vec2 {
        Vec2 { x: self.x / value, y: self.y / value }
    }
}

impl DivAssign<f32> for Vec2 {
    fn div_assign(&mut self, value: f32) {
        *self = *self / value;
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2 { x: -self.x, y: -self.y }
    }
}

// The render boundary: raylib's draw calls take `impl Into<ffi::Vector2>`,
// so a `Vec2` passes straight through, and the presentation code that
// still works in raylib's `Vector2` converts here. Nothing in
// `simulation/` uses these.

impl From<Vec2> for sola_raylib::ffi::Vector2 {
    fn from(v: Vec2) -> Self {
        sola_raylib::ffi::Vector2 { x: v.x, y: v.y }
    }
}

impl From<Vec2> for sola_raylib::core::math::Vector2 {
    fn from(v: Vec2) -> Self {
        sola_raylib::core::math::Vector2::new(v.x, v.y)
    }
}

impl From<sola_raylib::core::math::Vector2> for Vec2 {
    fn from(v: sola_raylib::core::math::Vector2) -> Self {
        Vec2::new(v.x, v.y)
    }
}

#[cfg(test)]
mod tests {
    use super::Vec2;
    use sola_raylib::core::math::Vector2;

    /// Every operation the round uses gives the bits raylib's type gives.
    #[test]
    fn matches_raylib_bit_for_bit() {
        let pairs = [
            (0.1_f32, 0.7_f32, 3.3_f32, -2.9_f32),
            (1e-3, 1e3, -7.25, 0.001),
            (123.456, 789.012, -0.333, 1.0 / 3.0),
            (0.0, 0.0, 1.0, 0.0),
        ];
        for (ax, ay, bx, by) in pairs {
            let a = Vec2::new(ax, ay);
            let b = Vec2::new(bx, by);
            let ra = Vector2::new(ax, ay);
            let rb = Vector2::new(bx, by);
            assert_eq!(a.length().to_bits(), ra.length().to_bits());
            assert_eq!(a.length_sqr().to_bits(), ra.length_sqr().to_bits());
            assert_eq!(a.dot(b).to_bits(), ra.dot(rb).to_bits());
            assert_eq!(a.distance_to(b).to_bits(), ra.distance_to(rb).to_bits());
            let same = |v: Vec2, r: Vector2| v.x.to_bits() == r.x.to_bits() && v.y.to_bits() == r.y.to_bits();
            assert!(same(a + b, ra + rb));
            assert!(same(a - b, ra - rb));
            assert!(same(a * b, ra * rb));
            assert!(same(a * 2.7, ra * 2.7));
            assert!(same(a / 2.7, ra / 2.7));
            assert!(same(-a, -ra));
        }
    }
}
