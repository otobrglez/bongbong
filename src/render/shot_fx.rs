//! The light shots throw: tracer streaks, halos, the muzzle and impact
//! flares with their star rays and lens glare. Every call here draws
//! inside an additive blend block (`Game::render` opens it), so the
//! colours add up to light on the ground rather than paint over it. Every
//! shape is smooth: soft radial glows, and streaks, rays and glare drawn as
//! tapering triangles whose colour fades along their length.
//!
//! Nothing here holds state. A shot's streak is drawn back from where it
//! is along the way it faces, and a flare is a function of its
//! `Shockwave`'s age - so a replica, a paused frame and a re-render all
//! draw the same picture. `shot_glow_strength` scales the lot.

use sola_raylib::prelude::*;

use crate::math::{Color, Vec2};
use crate::shockwave::Shockwave;
use crate::tuning::tuning;
use crate::Position;

/// The unit vector a sprite facing `rotation` degrees points along (the
/// game's convention: 0 = up, 90 = right).
pub fn heading(rotation: f32) -> Vec2 {
    let r = rotation.to_radians();
    Vec2::new(r.sin(), -r.cos())
}

/// `c` with its alpha scaled by `k` (clamped to 0..=1).
pub fn fade(c: Color, k: f32) -> Color {
    Color::new(c.r, c.g, c.b, (c.a as f32 * k.clamp(0.0, 1.0)) as u8)
}

/// One triangle with a colour per corner, straight into raylib's batch
/// (what `DrawTriangle` does, with the colour set per vertex so a shape can
/// fade along its length). The draw handle is only proof we are inside a
/// drawing pass. raylib wants the corners counter-clockwise on screen, so
/// they are put in that order here.
fn tri(_d: &mut impl RaylibDraw, a: (Vec2, Color), b: (Vec2, Color), c: (Vec2, Color)) {
    let cross = (b.0.x - a.0.x) * (c.0.y - a.0.y) - (b.0.y - a.0.y) * (c.0.x - a.0.x);
    let (b, c) = if cross > 0.0 { (c, b) } else { (b, c) };
    // SAFETY: plain rlgl immediate-mode calls, made between the begin and
    // end of a drawing pass (the caller holds a draw handle); they only
    // append vertices to raylib's current batch.
    unsafe {
        sola_raylib::ffi::rlBegin(0x0004); // RL_TRIANGLES
        for (p, col) in [a, b, c] {
            sola_raylib::ffi::rlColor4ub(col.r, col.g, col.b, col.a);
            sola_raylib::ffi::rlVertex2f(p.x, p.y);
        }
        sola_raylib::ffi::rlEnd();
    }
}

/// A tapering quad from `a` (`wa` px wide, `ca`) to `b` (`wb` wide, `cb`),
/// the colour blending smoothly along it.
fn taper(d: &mut impl RaylibDraw, a: Vec2, wa: f32, ca: Color, b: Vec2, wb: f32, cb: Color) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.01 {
        return;
    }
    let n = Vec2::new(-dy / len, dx / len);
    let (a0, a1) = (a + n * (wa * 0.5), a - n * (wa * 0.5));
    let (b0, b1) = (b + n * (wb * 0.5), b - n * (wb * 0.5));
    tri(d, (a0, ca), (a1, ca), (b0, cb));
    tri(d, (a1, ca), (b1, cb), (b0, cb));
}

/// A soft round glow: `color` in the middle falling smoothly to nothing
/// at `radius`.
pub fn glow(d: &mut impl RaylibDraw, at: Position, radius: f32, color: Color) {
    if radius < 0.5 || color.a == 0 {
        return;
    }
    d.draw_circle_gradient(at.x as i32, at.y as i32, radius, color, Color::new(color.r, color.g, color.b, 0));
}

/// A smooth streak from `head` back along `-dir` (a unit vector) for
/// `length` px: `width` px across at the head tapering to a point, shading
/// from `head_color` to `tail_color`, inside a fainter halo three times as
/// wide so it reads as light rather than a painted line.
pub fn streak(d: &mut impl RaylibDraw, head: Position, dir: Vec2, length: f32, width: f32, head_color: Color, tail_color: Color) {
    if length < 0.5 {
        return;
    }
    let tail = Position::new(head.x - dir.x * length, head.y - dir.y * length);
    let clear = Color::new(tail_color.r, tail_color.g, tail_color.b, 0);
    taper(d, head, width * 3.0, fade(head_color, 0.3), tail, 0.0, clear);
    taper(d, head, width, head_color, tail, 0.0, fade(tail_color, 0.5));
}

/// Four tapered rays out of `center` along `dir` and its three quarter
/// turns, `reach` px long, fading out to their tips.
fn star(d: &mut impl RaylibDraw, center: Position, dir: Vec2, reach: f32, width: f32, color: Color) {
    for (x, y) in [(dir.x, dir.y), (-dir.y, dir.x), (-dir.x, -dir.y), (dir.y, -dir.x)] {
        let tip = Position::new(center.x + x * reach, center.y + y * reach);
        taper(d, center, width, color, tip, 0.0, fade(color, 0.0));
    }
}

/// The anamorphic glare across a fresh flash: a thin horizontal sliver,
/// strongest at the centre, `half` px each way.
fn glare(d: &mut impl RaylibDraw, center: Position, half: f32, color: Color) {
    if half < 1.0 {
        return;
    }
    for side in [1.0, -1.0] {
        let tip = Position::new(center.x + half * side, center.y);
        taper(d, center, 2.5, color, tip, 0.0, fade(color, 0.0));
    }
}

/// Is `at` a flamethrower nozzle? Its stream pushes a muzzle flash every
/// held frame and already has its own glow (`draw_flame_glow`), so a
/// flare there would only pile a steady white blob onto it.
pub fn at_nozzle(at: Position, nozzles: &[Position]) -> bool {
    nozzles.iter().any(|n| n.distance_to(at) < 6.0)
}

/// A muzzle flare: a white-hot core in a yellow ball in an orange bloom,
/// a four-point star turned 45 degrees off the axes, and a flat glare for
/// its first half - all shrinking out over `muzzle_flash_duration`, so it
/// reads as a bang rather than a lamp.
pub fn draw_muzzle_flare(d: &mut impl RaylibDraw, flash: &Shockwave) {
    let strength = tuning().shot_glow_strength;
    let duration = tuning().muzzle_flash_duration.max(0.01);
    if strength <= 0.0 || flash.time >= duration {
        return;
    }
    let k = 1.0 - flash.time / duration;
    let r = tuning().muzzle_glow_radius * (0.35 + 0.65 * k);
    let a = strength * k;
    glow(d, flash.center, r * 2.2, fade(Color::new(255, 110, 30, 170), a));
    glow(d, flash.center, r * 1.2, fade(Color::new(255, 200, 80, 240), a));
    d.draw_circle_v(flash.center, r * 0.4, fade(Color::new(255, 255, 235, 255), a));
    let diag = std::f32::consts::FRAC_1_SQRT_2;
    star(d, flash.center, Vec2::new(diag, diag), r * 2.4, 3.0, fade(Color::new(255, 230, 150, 255), a));
    if k > 0.5 {
        glare(d, flash.center, tuning().shot_glare_length * (k - 0.5) * 2.0, fade(Color::new(255, 220, 170, 200), strength));
    }
}

/// An impact flare: a hot core that dies at once inside an orange bloom
/// that swells as it fades, an upright star, and a glare - the "thwack"
/// `impact.fs` bends the picture for, in light.
pub fn draw_impact_flare(d: &mut impl RaylibDraw, flash: &Shockwave) {
    let strength = tuning().shot_glow_strength;
    let duration = tuning().impact_flash_duration.max(0.01);
    if strength <= 0.0 || flash.time >= duration {
        return;
    }
    let k = 1.0 - flash.time / duration;
    let r = tuning().impact_glow_radius;
    let a = strength * k;
    glow(d, flash.center, r * (1.2 + 0.9 * (1.0 - k)), fade(Color::new(255, 90, 20, 190), a));
    glow(d, flash.center, r * 0.8, fade(Color::new(255, 180, 60, 230), a));
    d.draw_circle_v(flash.center, r * 0.3 * k, fade(Color::new(255, 255, 240, 255), a));
    star(d, flash.center, Vec2::new(0.0, -1.0), r * 2.0 * k, 3.0, fade(Color::new(255, 210, 120, 255), a));
    if k > 0.4 {
        glare(d, flash.center, tuning().shot_glare_length * 1.2 * k, fade(Color::new(255, 200, 150, 190), strength));
    }
}

/// A soft pool of light a shot throws on the ground around it (call
/// inside the additive block): a smooth radial falloff from `color` at
/// `strength` in the middle to nothing at `radius`.
pub fn ground_light(d: &mut impl RaylibDraw, at: Position, radius: f32, color: Color, strength: f32) {
    let k = strength * tuning().shot_glow_strength;
    if k <= 0.0 || radius < 1.0 {
        return;
    }
    glow(d, at, radius, fade(color, k));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_follows_the_games_rotation_convention() {
        let up = heading(0.0);
        let right = heading(90.0);
        assert!(up.x.abs() < 1e-6 && (up.y + 1.0).abs() < 1e-6, "0 degrees faces up");
        assert!((right.x - 1.0).abs() < 1e-6 && right.y.abs() < 1e-6, "90 degrees faces right");
    }

    #[test]
    fn a_nozzle_is_only_where_a_jet_starts() {
        let jets = [Position::new(100.0, 100.0)];
        assert!(at_nozzle(Position::new(102.0, 101.0), &jets));
        assert!(!at_nozzle(Position::new(140.0, 100.0), &jets));
    }
}
