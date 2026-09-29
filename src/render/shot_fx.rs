//! The light shots throw: tracer streaks, halos, the muzzle and impact
//! flares with their star rays and lens glare. Every call here draws
//! inside an additive blend block (`Game::render` opens it), so the
//! colours add up to light on the ground rather than paint over it, and
//! the shapes on the shots themselves are built from whole `BLOCK`s,
//! like `pixel_disc`; the light they throw on the ground (`ground_light`)
//! and the spike rings of an energy burst are smooth.
//!
//! Nothing here holds state. A shot's streak is drawn back from where it
//! is along the way it faces, and a flare is a function of its
//! `Shockwave`'s age - so a replica, a paused frame and a re-render all
//! draw the same picture. `shot_glow_strength` scales the lot.

use sola_raylib::prelude::*;

use crate::math::{Color, Vec2};
use crate::render::blast::pixel_disc;
use crate::shockwave::Shockwave;
use crate::tuning::tuning;
use crate::Position;

/// The block every shape here quantises to - one source pixel of every
/// sprite in the game.
const BLOCK: f32 = 2.0;

/// Steps of transparency a streak or a ray fades through. Four, like the
/// particles: few enough to read as drawn, not airbrushed.
const LEVELS: f32 = 4.0;

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

/// Linear blend of two colours, alpha included.
fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color::new(l(a.r, b.r), l(a.g, b.g), l(a.b, b.b), l(a.a, b.a))
}

/// One square `blocks` blocks wide centred on `at`, snapped to the grid.
fn block(d: &mut impl RaylibDraw, at: Position, blocks: i32, color: Color) {
    if color.a == 0 {
        return;
    }
    let side = blocks.max(1) as f32 * BLOCK;
    let x = ((at.x - side / 2.0) / BLOCK).round() * BLOCK;
    let y = ((at.y - side / 2.0) / BLOCK).round() * BLOCK;
    d.draw_rectangle(x as i32, y as i32, side as i32, side as i32, color);
}

/// A streak of blocks from `head` back along `-dir` for `length` px,
/// `blocks` wide, shading from `head_color` to `tail_color` and stepping
/// down to nothing at the tail. `dir` must be a unit vector.
pub fn pixel_streak(
    d: &mut impl RaylibDraw,
    head: Position,
    dir: Vec2,
    length: f32,
    blocks: i32,
    head_color: Color,
    tail_color: Color,
) {
    let steps = (length / BLOCK).floor() as i32;
    for i in 0..=steps {
        let t = i as f32 / steps.max(1) as f32;
        let step_fade = ((1.0 - t) * LEVELS).ceil() / LEVELS;
        let at = Position::new(head.x - dir.x * i as f32 * BLOCK, head.y - dir.y * i as f32 * BLOCK);
        block(d, at, blocks, fade(mix(head_color, tail_color, t), step_fade));
    }
}

/// Four rays of single blocks out of `center` along `dir` and its three
/// quarter turns, `reach` px long, fading out stepwise.
fn star(d: &mut impl RaylibDraw, center: Position, dir: Vec2, reach: f32, color: Color) {
    for (x, y) in [(dir.x, dir.y), (-dir.y, dir.x), (-dir.x, -dir.y), (dir.y, -dir.x)] {
        let tip = Position::new(center.x + x * reach, center.y + y * reach);
        pixel_streak(d, tip, Vec2::new(x, y), reach, 1, fade(color, 0.25), color);
    }
}

/// The anamorphic glare across a fresh flash: a thin horizontal line,
/// strongest at the centre, `half` px each way.
fn glare(d: &mut impl RaylibDraw, center: Position, half: f32, color: Color) {
    if half < BLOCK {
        return;
    }
    let right = Position::new(center.x + half, center.y);
    let left = Position::new(center.x - half, center.y);
    pixel_streak(d, right, Vec2::new(1.0, 0.0), half, 1, fade(color, 0.2), color);
    pixel_streak(d, left, Vec2::new(-1.0, 0.0), half, 1, fade(color, 0.2), color);
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
    pixel_disc(d, flash.center, r * 1.7, fade(Color::new(255, 110, 30, 150), a));
    pixel_disc(d, flash.center, r, fade(Color::new(255, 200, 80, 230), a));
    pixel_disc(d, flash.center, r * 0.45, fade(Color::new(255, 255, 235, 255), a));
    let diag = std::f32::consts::FRAC_1_SQRT_2;
    star(d, flash.center, Vec2::new(diag, diag), r * 2.2, fade(Color::new(255, 230, 150, 255), a));
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
    pixel_disc(d, flash.center, r * (0.8 + 0.9 * (1.0 - k)), fade(Color::new(255, 90, 20, 170), a));
    pixel_disc(d, flash.center, r * 0.7, fade(Color::new(255, 180, 60, 220), a));
    pixel_disc(d, flash.center, r * 0.35 * k, fade(Color::new(255, 255, 240, 255), a));
    star(d, flash.center, Vec2::new(0.0, -1.0), r * 2.0 * k, fade(Color::new(255, 210, 120, 255), a));
    if k > 0.4 {
        glare(d, flash.center, tuning().shot_glare_length * 1.2 * k, fade(Color::new(255, 200, 150, 190), strength));
    }
}

/// A soft pool of light a shot throws on the ground around it (call
/// inside the additive block): a smooth radial falloff from `color` at
/// `strength` in the middle to nothing at `radius`. The one smooth shape
/// here - light on the floor has no edge to step.
pub fn ground_light(d: &mut impl RaylibDraw, at: Position, radius: f32, color: Color, strength: f32) {
    let k = strength * tuning().shot_glow_strength;
    if k <= 0.0 || radius < 1.0 {
        return;
    }
    d.draw_circle_gradient(at.x as i32, at.y as i32, radius, fade(color, k), Color::new(color.r, color.g, color.b, 0));
}

/// A ring of short radial spikes thrown out from `center` (additive): the
/// burst an energy shot makes where it lands. `progress` 0..1 carries the
/// ring from `from` to `to` px while it fades; `spikes` of them, turned by
/// `turn` radians so two bursts do not line up.
#[allow(clippy::too_many_arguments)]
pub fn spike_ring(d: &mut impl RaylibDraw, center: Position, progress: f32, from: f32, to: f32, spikes: i32, turn: f32, color: Color) {
    let k = tuning().shot_glow_strength * (1.0 - progress);
    if k <= 0.0 {
        return;
    }
    let r = from + (to - from) * (1.0 - (1.0 - progress) * (1.0 - progress));
    let len = 3.0 + 7.0 * (1.0 - progress);
    let c = fade(color, k);
    for i in 0..spikes {
        let a = turn + i as f32 / spikes as f32 * std::f32::consts::TAU;
        let (cs, sn) = (a.cos(), a.sin());
        let inner = Vec2::new(center.x + cs * r, center.y + sn * r);
        let outer = Vec2::new(center.x + cs * (r + len), center.y + sn * (r + len));
        d.draw_line_ex(inner, outer, 2.0, c);
        d.draw_circle_v(outer, 1.5, fade(Color::WHITE, k));
    }
    d.draw_ring(center, (r - 1.0).max(0.0), r + 1.0, 0.0, 360.0, 48, fade(color, k * 0.45));
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
    fn mix_hits_both_ends() {
        let a = Color::new(0, 10, 20, 255);
        let b = Color::new(200, 110, 20, 55);
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
    }

    #[test]
    fn a_nozzle_is_only_where_a_jet_starts() {
        let jets = [Position::new(100.0, 100.0)];
        assert!(at_nozzle(Position::new(102.0, 101.0), &jets));
        assert!(!at_nozzle(Position::new(140.0, 100.0), &jets));
    }
}
