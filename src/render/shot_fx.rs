//! The light shots throw, in the effects language (`pyro.rs`,
//! docs/effects.md): tracer streaks, halos, rays, the pools of light on
//! the ground. Every call here draws inside an additive
//! blend block (`Game::render` opens it), so the colours add up to light
//! on the ground rather than paint over it. Every shape is built from the
//! field's 2 px blocks: a glow is a few flat bands of light with dithered
//! edges (`pyro::glow`, the weather light pass's rule), and streaks and
//! rays are lines of blocks stepping down their colour to their tips.
//!
//! Nothing here holds state. A shot's streak is drawn back from where it
//! is along the way it faces - so a replica, a paused frame and a
//! re-render all draw the same picture. `shot_glow_strength` scales the lot, and
//! `glow_bands` sets how many steps a glow is drawn in (0 draws the
//! smooth gradients, for comparison).

use sola_raylib::prelude::*;

use crate::math::{Color, Vec2};
use crate::pyro;
use crate::render::pyro::Rl;
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

/// How many flat steps a glow is drawn in (`glow_bands`); 0 is smooth.
fn bands() -> u32 {
    tuning().glow_bands.max(0) as u32
}

/// A tapering line from `a` (`wa` px wide, `ca`) to `b` (`wb` wide, `cb`):
/// whole blocks, the width rounded to blocks and the colour stepping from
/// `ca` to `cb` in three flat steps.
pub(crate) fn taper(d: &mut impl RaylibDraw, a: Vec2, wa: f32, ca: Color, b: Vec2, wb: f32, cb: Color) {
    let block = pyro::BLOCK;
    pyro::block_taper(&mut Rl(d), a, b, wa / block, wb / block, |t| pyro::between(ca, cb, t, 3));
}

/// A glow: `color` in the middle falling to nothing at `radius`, in
/// `glow_bands` flat steps with dithered edges.
pub fn glow(d: &mut impl RaylibDraw, at: Position, radius: f32, color: Color) {
    if radius < 0.5 || color.a == 0 {
        return;
    }
    match bands() {
        0 => d.draw_circle_gradient(at.x as i32, at.y as i32, radius, color, Color::new(color.r, color.g, color.b, 0)),
        n => pyro::glow(&mut Rl(d), at, radius, color, n),
    }
}

/// A streak from `head` back along `-dir` (a unit vector) for `length`
/// px: `width` px across at the head narrowing to a block at the tail,
/// stepping from `head_color` to half of `tail_color`.
pub fn streak(d: &mut impl RaylibDraw, head: Position, dir: Vec2, length: f32, width: f32, head_color: Color, tail_color: Color) {
    if length < 0.5 {
        return;
    }
    let tail = Position::new(head.x - dir.x * length, head.y - dir.y * length);
    taper(d, head, width, head_color, tail, 0.0, fade(tail_color, 0.5));
}

/// Four rays out of `center` along `dir` and its three quarter turns,
/// `reach` px long, stepping out to nothing at their tips.
pub(crate) fn star(d: &mut impl RaylibDraw, center: Position, dir: Vec2, reach: f32, width: f32, color: Color) {
    for (x, y) in [(dir.x, dir.y), (-dir.y, dir.x), (-dir.x, -dir.y), (dir.y, -dir.x)] {
        let tip = Position::new(center.x + x * reach, center.y + y * reach);
        taper(d, center, width, color, tip, 0.0, fade(color, 0.0));
    }
}

/// Is `at` a flamethrower nozzle? Its stream pushes a muzzle flash every
/// held frame and already has its own glow (`draw_flame_glow`), so a
/// flare there would only pile a steady white blob onto it.
pub fn at_nozzle(at: Position, nozzles: &[Position]) -> bool {
    nozzles.iter().any(|n| n.distance_to(at) < 6.0)
}

/// A pool of light a shot throws on the ground around it (call inside
/// the additive block): `color` at `strength` in the middle, stepping
/// down to nothing at `radius` (`glow`).
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
