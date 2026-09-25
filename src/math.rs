//! The plain value types the simulation and the renderer share: `Vec2`,
//! the round's one vector type for positions, velocities and directions
//! in field pixels (y down), `Color` and `Rectangle`. They are the game's
//! own so that nothing under `simulation/` and no entity's state names a
//! raylib type - a headless round, the probe and the room server build
//! without the drawing crate (`--no-default-features`). `Vec2`'s
//! arithmetic is written out component by component in the same order
//! raylib's `Vector2` performs it (`length` is `(x*x + y*y).sqrt()`,
//! `distance_to` squares the differences before the root, nothing goes
//! through a reciprocal), because every seeded replay and every recorded
//! probe fixture depends on those exact bits. Only the operations the
//! round uses are here; add one the same way, never by calling out to
//! another vector library. The render boundary converts with `From` (at
//! the bottom, `render` only) and nowhere else.

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

/// An RGBA colour, one byte per channel, laid out like raylib's `Color`.
/// The simulation names colours only as data - a team's marker, a health
/// ramp step, a particle tint - and every draw call converts at the render
/// boundary. `alpha` and `color_from_hsv` are the two helpers the round
/// uses, ported byte for byte from raylib's `ColorAlpha`/`ColorFromHSV` so
/// the presentation is unchanged.
#[repr(C)]
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }

    // raylib's named colours, the ones the game uses.
    pub const WHITE: Color = Color::new(255, 255, 255, 255);
    pub const BLACK: Color = Color::new(0, 0, 0, 255);
    pub const RAYWHITE: Color = Color::new(245, 245, 245, 255);
    pub const LIGHTGRAY: Color = Color::new(211, 211, 211, 255);
    pub const GRAY: Color = Color::new(128, 128, 128, 255);
    pub const DARKGRAY: Color = Color::new(169, 169, 169, 255);
    pub const RED: Color = Color::new(255, 0, 0, 255);
    pub const MAROON: Color = Color::new(128, 0, 0, 255);
    pub const ORANGE: Color = Color::new(255, 165, 0, 255);
    pub const GOLD: Color = Color::new(255, 215, 0, 255);
    pub const YELLOW: Color = Color::new(255, 255, 0, 255);
    pub const LIME: Color = Color::new(0, 255, 0, 255);
    pub const DARKGREEN: Color = Color::new(0, 100, 0, 255);
    pub const SKYBLUE: Color = Color::new(135, 206, 235, 255);
    pub const MAGENTA: Color = Color::new(255, 0, 255, 255);

    /// The same colour at `alpha` (0..=1, clamped): `ColorAlpha`, the
    /// channel truncated to a byte the way the C cast does.
    pub fn alpha(&self, alpha: f32) -> Color {
        let alpha = alpha.clamp(0.0, 1.0);
        Color { r: self.r, g: self.g, b: self.b, a: (255.0 * alpha) as u8 }
    }

    /// `ColorFromHSV`: hue in degrees, saturation and value in 0..=1,
    /// opaque. Each channel is one `fmod` fold of the hue, clamped, so
    /// the result is the byte raylib would draw with.
    pub fn color_from_hsv(hue: f32, saturation: f32, value: f32) -> Color {
        let channel = |offset: f32| {
            let mut k = (offset + hue / 60.0) % 6.0;
            let t = 4.0 - k;
            k = if t < k { t } else { k };
            k = if k < 1.0 { k } else { 1.0 };
            k = if k > 0.0 { k } else { 0.0 };
            ((value - value * saturation * k) * 255.0) as u8
        };
        Color { r: channel(5.0), g: channel(3.0), b: channel(1.0), a: 255 }
    }
}

/// An axis-aligned box in pixels, laid out like raylib's `Rectangle`: a
/// sprite's source cell on its sheet, its destination on the field, a
/// button's hit box. `Rect` in lib.rs is the same shape under the names
/// `Layout` hands around; this one is what a blit and a hit test take.
#[repr(C)]
#[derive(Default, Debug, Copy, Clone, PartialEq)]
pub struct Rectangle {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rectangle {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Rectangle {
        Rectangle { x, y, width, height }
    }

    /// Whether `p` is inside: on or right of the left edge and above the
    /// right one, the half-open test `CheckCollisionPointRec` makes.
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x && p.x < self.x + self.width && p.y >= self.y && p.y < self.y + self.height
    }
}

// The render boundary: raylib's draw calls take `impl Into<ffi::Vector2>`,
// `impl Into<ffi::Rectangle>` and `impl Into<ffi::Color>`, so the three
// types pass straight through, and the presentation code that works in
// raylib's own types converts here. Only the `render` feature has raylib
// at all; nothing in `simulation/` uses these.

#[cfg(feature = "render")]
mod raylib {
    use super::{Color, Rectangle, Vec2};

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

    impl From<Color> for sola_raylib::ffi::Color {
        fn from(c: Color) -> Self {
            sola_raylib::ffi::Color { r: c.r, g: c.g, b: c.b, a: c.a }
        }
    }

    impl From<Color> for sola_raylib::core::color::Color {
        fn from(c: Color) -> Self {
            sola_raylib::core::color::Color::new(c.r, c.g, c.b, c.a)
        }
    }

    impl From<sola_raylib::core::color::Color> for Color {
        fn from(c: sola_raylib::core::color::Color) -> Self {
            Color::new(c.r, c.g, c.b, c.a)
        }
    }

    impl From<Rectangle> for sola_raylib::ffi::Rectangle {
        fn from(r: Rectangle) -> Self {
            sola_raylib::ffi::Rectangle { x: r.x, y: r.y, width: r.width, height: r.height }
        }
    }

    impl From<Rectangle> for sola_raylib::core::math::Rectangle {
        fn from(r: Rectangle) -> Self {
            sola_raylib::core::math::Rectangle::new(r.x, r.y, r.width, r.height)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Color, Rectangle, Vec2};

    #[test]
    fn rectangle_contains_is_half_open() {
        let r = Rectangle::new(10.0, 20.0, 30.0, 40.0);
        assert!(r.contains(Vec2::new(10.0, 20.0)));
        assert!(r.contains(Vec2::new(39.9, 59.9)));
        assert!(!r.contains(Vec2::new(40.0, 30.0)));
        assert!(!r.contains(Vec2::new(20.0, 60.0)));
        assert!(!r.contains(Vec2::new(9.9, 30.0)));
    }

    #[test]
    fn alpha_truncates_like_the_c_cast() {
        assert_eq!(Color::WHITE.alpha(0.5).a, 127);
        assert_eq!(Color::WHITE.alpha(2.0).a, 255);
        assert_eq!(Color::WHITE.alpha(-1.0).a, 0);
        assert_eq!(Color::new(1, 2, 3, 4).alpha(1.0), Color::new(1, 2, 3, 255));
    }

    #[test]
    fn hsv_hits_the_primaries() {
        assert_eq!(Color::color_from_hsv(0.0, 1.0, 1.0), Color::new(255, 0, 0, 255));
        assert_eq!(Color::color_from_hsv(120.0, 1.0, 1.0), Color::new(0, 255, 0, 255));
        assert_eq!(Color::color_from_hsv(240.0, 1.0, 1.0), Color::new(0, 0, 255, 255));
        assert_eq!(Color::color_from_hsv(60.0, 0.0, 0.5), Color::new(127, 127, 127, 255));
    }
}

/// With raylib linked, every helper here is checked against the C
/// function it was ported from, bit for bit.
#[cfg(all(test, feature = "render"))]
mod raylib_tests {
    use super::{Color, Vec2};
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

    /// `alpha` and `color_from_hsv` give the bytes raylib's C helpers give,
    /// over a sweep of hues, saturations, values and alphas.
    #[test]
    fn colour_helpers_match_raylib() {
        type RlColor = sola_raylib::core::color::Color;
        let same = |c: Color, r: RlColor| (c.r, c.g, c.b, c.a) == (r.r, r.g, r.b, r.a);
        for i in 0..=100 {
            let a = i as f32 / 100.0 * 1.4 - 0.2;
            assert!(same(Color::new(9, 8, 7, 6).alpha(a), RlColor::new(9, 8, 7, 6).alpha(a)), "alpha {a}");
        }
        for h in (0..720).step_by(7) {
            for (s, v) in [(1.0, 1.0), (0.6, 1.0), (0.85, 1.0), (0.3, 0.5), (0.0, 0.25)] {
                let hue = h as f32 + 0.37;
                assert!(same(Color::color_from_hsv(hue, s, v), RlColor::color_from_hsv(hue, s, v)), "hsv {hue} {s} {v}");
            }
        }
    }
}
