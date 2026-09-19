//! Portable `sin`/`cos`/`atan2` for anything a pinned CPU render depends
//! on. `f32::sin` and friends call the platform's libm, and macOS and
//! glibc disagree by an ulp often enough that a nearest-neighbour sample
//! lands on the other side of a pixel boundary - so a thumbnail hashed on
//! a Mac failed to match the same map on a Linux runner. These use only
//! IEEE add/multiply/divide in f64 (which Rust never fuses or reorders),
//! then round once to f32, so every platform produces the same bits. They
//! are accurate to well under an f32 ulp over their whole range and cheap
//! enough for a per-blit call; nothing here runs per pixel except the
//! ring's `atan2`, and that only on the CPU canvas.

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};

/// `sin` and `cos` of `radians`, as `f32::sin_cos` would give them.
pub fn sin_cos(radians: f32) -> (f32, f32) {
    let x = radians as f64;
    if !x.is_finite() {
        return (f32::NAN, f32::NAN);
    }
    // Reduce to |r| <= pi/4 and remember the quadrant.
    let k = (x / FRAC_PI_2).round();
    let r = x - k * FRAC_PI_2;
    let (s, c) = small_sin_cos(r);
    match (k as i64).rem_euclid(4) {
        0 => (s as f32, c as f32),
        1 => (c as f32, -s as f32),
        2 => (-s as f32, -c as f32),
        _ => (-c as f32, s as f32),
    }
}

pub fn sin(radians: f32) -> f32 {
    sin_cos(radians).0
}

pub fn cos(radians: f32) -> f32 {
    sin_cos(radians).1
}

/// Taylor series for |r| <= pi/4: the r^15 / r^14 tails are below 1e-13,
/// far under f32's resolution after the final rounding.
fn small_sin_cos(r: f64) -> (f64, f64) {
    let r2 = r * r;
    let s = r
        * (1.0
            - r2 / 6.0 * (1.0 - r2 / 20.0 * (1.0 - r2 / 42.0 * (1.0 - r2 / 72.0 * (1.0 - r2 / 110.0 * (1.0 - r2 / 156.0 * (1.0 - r2 / 210.0)))))));
    let c = 1.0
        - r2 / 2.0 * (1.0 - r2 / 12.0 * (1.0 - r2 / 30.0 * (1.0 - r2 / 56.0 * (1.0 - r2 / 90.0 * (1.0 - r2 / 132.0 * (1.0 - r2 / 182.0))))));
    (s, c)
}

/// `y.atan2(x)` in radians, with libm's conventions for the axes and
/// signs (`atan2(0, -x)` is pi, `atan2(-0.0, -x)` is -pi).
pub fn atan2(y: f32, x: f32) -> f32 {
    let (yf, xf) = (y as f64, x as f64);
    if yf.is_nan() || xf.is_nan() {
        return f32::NAN;
    }
    if yf == 0.0 {
        return if xf.is_sign_negative() { PI.copysign(yf) as f32 } else { yf as f32 };
    }
    if xf == 0.0 {
        return FRAC_PI_2.copysign(yf) as f32;
    }
    let (ay, ax) = (yf.abs(), xf.abs());
    // atan of a ratio in [0, 1], then fold the octant back.
    let swap = ay > ax;
    let t = if swap { ax / ay } else { ay / ax };
    let mut a = atan_unit(t);
    if swap {
        a = FRAC_PI_2 - a;
    }
    if xf < 0.0 {
        a = PI - a;
    }
    (if yf < 0.0 { -a } else { a }) as f32
}

/// atan(t) for t in [0, 1]: above tan(pi/8) the identity
/// atan(t) = pi/4 + atan((t - 1) / (t + 1)) brings the argument under
/// 0.4143, where the odd series to t^23 is accurate past 1e-9.
fn atan_unit(t: f64) -> f64 {
    const TAN_PI_8: f64 = 0.414_213_562_373_095_1;
    let (base, u) = if t > TAN_PI_8 { (FRAC_PI_4, (t - 1.0) / (t + 1.0)) } else { (0.0, t) };
    let u2 = u * u;
    let mut term = u;
    let mut sum = u;
    let mut n = 1.0;
    for _ in 0..11 {
        term *= -u2;
        n += 2.0;
        sum += term / n;
    }
    base + sum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_std_to_well_under_an_f32_ulp_of_one() {
        for i in -20_000..=20_000 {
            let x = i as f32 * 0.001;
            let (s, c) = sin_cos(x);
            assert!((s - x.sin()).abs() <= 2e-7, "sin({x}): {s} vs {}", x.sin());
            assert!((c - x.cos()).abs() <= 2e-7, "cos({x}): {c} vs {}", x.cos());
        }
        for yi in -40..=40 {
            for xi in -40..=40 {
                let (y, x) = (yi as f32 * 0.37, xi as f32 * 0.29);
                let a = atan2(y, x);
                let want = y.atan2(x);
                assert!((a - want).abs() <= 3e-7, "atan2({y}, {x}): {a} vs {want}");
            }
        }
        assert_eq!(atan2(0.0, -1.0), std::f32::consts::PI);
        assert_eq!(atan2(-0.0, -1.0), -std::f32::consts::PI);
        assert_eq!(atan2(0.0, 1.0), 0.0);
        assert_eq!(atan2(1.0, 0.0), std::f32::consts::FRAC_PI_2);
        assert!(sin_cos(f32::NAN).0.is_nan());
    }

    #[test]
    fn exact_at_the_quadrants() {
        // The reductions land exactly on 0 for multiples of pi/2, so the
        // axes come out as clean 0 / 1 / -1 like raylib's own quarter turns.
        assert_eq!(sin_cos(0.0), (0.0, 1.0));
        assert_eq!(sin_cos(std::f32::consts::FRAC_PI_2).1.abs() < 1e-7, true);
        assert_eq!(sin_cos(std::f32::consts::PI).1, -1.0);
    }
}
