//! The raylib half of `tower.rs` (docs/defence-towers-prd.md section 12),
//! drawn the way every shot is (`render/shot_fx.rs`): smooth, additive and
//! stateless. The tesla coil's bolt - three violet filaments with forks
//! crackling off them, a bloom round the lot and a flare at each end, the
//! author's L3 pick - and the light the towers throw: a charging coil's
//! lens, the ooze in a mortar's mouth, a glob in the air trailing its
//! drips, the faint glow of the ooze on the ground and on a coated hull, a
//! burning tower. The bolt's hit and a glob's splash are bursts from
//! `static/impact_burst.fs` (`ImpactKind::Tesla`, `ImpactKind::Ooze`),
//! started by `fx.rs`.

use sola_raylib::prelude::*;

use crate::math::{Color, Vec2};
use crate::render::shot_fx::{fade, glow, ground_light, star, streak, taper};
use crate::simulation::Game;
use crate::tower::{Glob, TeslaBolt, TowerKind, TowerView, OOZE_DK, OOZE_HI, OOZE_LT};
use crate::tuning::tuning;
use crate::Position;

/// The bolt's strands: pale violet, white-hot, deep violet - the plasma
/// cannon's arcane family, so the tesla reads apart from the teal plasma
/// bolt.
const STRAND_PALE: Color = Color::new(0xca, 0xa6, 0xff, 255);
const STRAND_CORE: Color = Color::new(0xff, 0xff, 0xff, 255);
const STRAND_DEEP: Color = Color::new(0x9a, 0x66, 0xff, 255);
const BOLT_GLOW: Color = Color::new(0xb0, 0x78, 0xff, 255);
/// The bloom round a bolt: a deeper violet with little green in it, since
/// light added onto the grass washes toward white.
const BOLT_BLOOM: Color = Color::new(0x8c, 0x40, 0xff, 255);
/// A burning tower's light, the fire's own orange.
const FIRE_LIGHT: Color = Color::new(255, 140, 50, 255);

fn hash(a: u32, b: u32) -> f32 {
    let mut h = a.wrapping_mul(0x27d4_eb2d) ^ b.wrapping_mul(0x1656_67b1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h % 10_000) as f32 / 10_000.0
}

/// A jagged path from `a` to `b`: a kink every ~11 px, pushed sideways by
/// up to `jag` px, tapering to nothing at both ends.
fn jagged(a: Position, b: Position, seed: u32, jag: f32) -> Vec<Position> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let n = ((len / 11.0).round() as u32).max(4);
    let (nx, ny) = (-dy / len, dx / len);
    let mut pts = vec![a];
    for i in 1..n {
        let u = i as f32 / n as f32;
        let off = (hash(seed, i) - 0.5) * 2.0 * jag * (u * std::f32::consts::PI).sin();
        pts.push(Position::new(a.x + dx * u + nx * off, a.y + dy * u + ny * off));
    }
    pts.push(b);
    pts
}

/// A smooth band along `pts`, `w0` px wide at the first point and `w1` at
/// the last, in `color`.
fn band(d: &mut impl RaylibDraw, pts: &[Position], w0: f32, w1: f32, color: Color) {
    let n = pts.len().saturating_sub(1).max(1) as f32;
    for (i, pair) in pts.windows(2).enumerate() {
        let (wa, wb) = (w0 + (w1 - w0) * i as f32 / n, w0 + (w1 - w0) * (i + 1) as f32 / n);
        taper(d, Vec2::new(pair[0].x, pair[0].y), wa, color, Vec2::new(pair[1].x, pair[1].y), wb, color);
    }
}

/// How bright a bolt is this frame, 0..=1: its fade-out times a fast
/// flicker off its own clock, so it buzzes rather than dimming flat.
fn bolt_light(bolt: &TeslaBolt) -> f32 {
    let alpha = bolt.alpha();
    let flicker = 0.75 + 0.25 * ((1.0 - alpha) * 40.0 + bolt.seed as f32 * 0.01).sin();
    alpha * flicker
}

/// One tesla bolt, in its own additive block: a violet bloom along the
/// path, a few short forks crackling off it, three filaments re-jagged
/// three times over its life, and a flare at each end - a small one at
/// the terminal, a bigger, whiter one where it struck. The filaments are
/// drawn whatever `shot_glow_strength` says; the bloom and the flares
/// scale with it, like every shot's light.
pub fn draw_tesla_bolt(d: &mut impl RaylibDraw, bolt: &TeslaBolt) {
    let alpha = bolt.alpha();
    if alpha <= 0.0 {
        return;
    }
    let k = bolt_light(bolt);
    let strength = tuning().shot_glow_strength;
    let jolt = ((1.0 - alpha) * 3.0).floor() as u32;
    let seed = bolt.seed.wrapping_add(jolt.wrapping_mul(977));
    let main = jagged(bolt.start, bolt.end, seed, 7.0);
    d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
        if strength > 0.0 {
            band(&mut bd, &main, 14.0, 18.0, fade(BOLT_BLOOM, 0.3 * k * strength));
            band(&mut bd, &main, 6.0, 8.0, fade(BOLT_BLOOM, 0.55 * k * strength));
        }
        // Forks: short crooked twigs off the inner kinks, off to either
        // side of the way the bolt runs.
        if main.len() > 3 {
            for f in 0..3u32 {
                let i = 1 + (hash(seed, 50 + f) * (main.len() - 2) as f32) as usize % (main.len() - 2);
                let (a, b) = (main[i], main[i + 1]);
                let step = b - a;
                let run = if step.length() > 0.01 { step * (1.0 / step.length()) } else { Vec2::new(0.0, -1.0) };
                let side = if hash(seed, 60 + f) < 0.5 { 1.0 } else { -1.0 };
                let turn = (0.5 + 0.6 * hash(seed, 70 + f)) * side;
                let dir = Vec2::new(run.x * turn.cos() - run.y * turn.sin(), run.x * turn.sin() + run.y * turn.cos());
                let tip = a + dir * (8.0 + 12.0 * hash(seed, 80 + f));
                band(&mut bd, &jagged(a, tip, seed.wrapping_add(90 + f), 3.0), 1.6, 0.0, fade(STRAND_PALE, 0.8 * alpha));
            }
        }
        for (i, (color, width, jag)) in [(STRAND_DEEP, 3.0, 7.0), (STRAND_PALE, 2.0, 5.0), (STRAND_CORE, 0.9, 3.0)].into_iter().enumerate() {
            let path = if i == 0 { main.clone() } else { jagged(bolt.start, bolt.end, seed.wrapping_add(i as u32 * 11), jag) };
            band(&mut bd, &path, width, width * 0.7, fade(color, alpha));
        }
        if strength > 0.0 {
            let a = k * strength;
            glow(&mut bd, bolt.start, 14.0, fade(BOLT_GLOW, 0.6 * a));
            glow(&mut bd, bolt.start, 6.0, fade(STRAND_CORE, 0.8 * a));
            glow(&mut bd, bolt.end, 30.0, fade(BOLT_BLOOM, 0.7 * a));
            glow(&mut bd, bolt.end, 11.0, fade(STRAND_PALE, 0.8 * a));
            let diag = std::f32::consts::FRAC_1_SQRT_2;
            star(&mut bd, bolt.end, Vec2::new(diag, diag), 18.0 * k, 2.5, fade(STRAND_PALE, a));
        }
        bd.draw_circle_v(bolt.end, 2.5 * alpha, fade(Color::WHITE, alpha));
    });
}

/// The light a standing tower throws on itself (inside an additive block,
/// over the towers): a charging coil's lens and prong tips brightening
/// toward the strike, the ooze glowing in a mortar's mouth, a burning
/// tower's fire.
pub fn draw_tower_glow(d: &mut impl RaylibDraw, view: &TowerView, time: f32) {
    let strength = tuning().shot_glow_strength;
    let at = view.position;
    match view.kind {
        TowerKind::Tesla if view.charge > 0.05 => {
            let k = view.charge * view.charge * strength;
            glow(d, Position::new(at.x, at.y - 1.0), 12.0 + 20.0 * k, fade(BOLT_GLOW, 0.5 * k));
            glow(d, Position::new(at.x, at.y - 1.0), 5.0 + 4.0 * k, fade(STRAND_PALE, 0.6 * k));
        }
        TowerKind::Bio => {
            let pulse = 0.85 + 0.15 * (time * 3.1 + at.x * 0.01).sin();
            let mouth = at + crate::render::shot_fx::heading(view.heading) * 12.0;
            let loaded = 0.3 + 0.7 * view.charge;
            glow(d, mouth, 10.0 + 8.0 * view.charge, fade(OOZE_LT, 0.35 * pulse * loaded * strength));
        }
        _ => {}
    }
    if view.burning {
        let flicker = 0.8 + 0.2 * (time * 23.0 + at.y * 0.1).sin();
        glow(d, Position::new(at.x, at.y - 4.0), 26.0, fade(FIRE_LIGHT, 0.35 * flicker * strength));
    }
}

/// A glob in the air (inside an additive block, over its sprite): a halo
/// round the blob and a smooth trail of drips back along the arc it is
/// drawn on.
pub fn draw_glob_glow(d: &mut impl RaylibDraw, glob: &Glob) {
    let strength = tuning().shot_glow_strength;
    let air = |g: &Glob| {
        let p = g.ground_pos();
        Position::new(p.x, p.y - g.height())
    };
    let at = air(glob);
    let mut before = glob.clone();
    before.age = (glob.age - 0.06).max(0.0);
    let back = at - air(&before);
    let length = back.length();
    if length > 0.5 {
        let dir = back * (1.0 / length);
        streak(d, at, dir, (length * 2.5).min(26.0), 5.0, fade(OOZE_HI, 0.8 * strength), fade(OOZE_DK, 0.6 * strength));
    }
    glow(d, at, 14.0, fade(OOZE_LT, 0.45 * strength));
}

impl Game {
    /// Every standing tower's light and every glob's (additive block, over
    /// the standing layer).
    pub(crate) fn draw_towers_light(&self, d: &mut impl RaylibDraw) {
        for view in self.tower_views() {
            draw_tower_glow(d, &view, self.time);
        }
        for glob in &self.globs {
            draw_glob_glow(d, glob);
        }
    }

    /// The towers' pools of light on the ground (inside
    /// `draw_ground_light`'s block): violet under a charging coil and
    /// along a bolt, lime round a mortar, under a glob in the air, off
    /// every puddle - brightest while fresh, with a slow breathing - and
    /// round a coated hull; orange round a burning tower.
    pub(crate) fn draw_towers_ground_light(&self, d: &mut impl RaylibDraw) {
        for view in self.tower_views() {
            match view.kind {
                TowerKind::Tesla if view.charge > 0.05 => {
                    let k = view.charge * view.charge;
                    ground_light(d, view.position, 40.0 + 30.0 * k, BOLT_GLOW, 0.35 * k);
                }
                TowerKind::Bio => ground_light(d, view.position, 36.0, OOZE_LT, 0.1),
                _ => {}
            }
            if view.burning {
                ground_light(d, view.position, 44.0, FIRE_LIGHT, 0.3);
            }
        }
        for bolt in &self.tesla_bolts {
            let k = bolt_light(bolt);
            let mid = Position::new((bolt.start.x + bolt.end.x) * 0.5, (bolt.start.y + bolt.end.y) * 0.5);
            ground_light(d, bolt.end, 44.0, BOLT_GLOW, 0.5 * k);
            ground_light(d, mid, 34.0, BOLT_GLOW, 0.2 * k);
            ground_light(d, bolt.start, 26.0, BOLT_GLOW, 0.3 * k);
        }
        for glob in &self.globs {
            ground_light(d, glob.ground_pos(), 18.0, OOZE_LT, 0.22);
        }
        for (&(c, r), puddle) in &self.ooze {
            let at = crate::map::cell_to_world(c, r);
            let breathe = 0.8 + 0.2 * (self.time * 1.7 + (c * 7 + r * 13) as f32 * 0.37).sin();
            ground_light(d, at, 22.0, OOZE_LT, 0.12 * puddle.freshness() * breathe);
        }
        for (_, at) in self.slimed() {
            ground_light(d, at, 26.0, OOZE_LT, 0.14);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bolt_path_starts_and_ends_where_it_was_asked_to() {
        let a = Position::new(10.0, 10.0);
        let b = Position::new(110.0, 40.0);
        let pts = jagged(a, b, 7, 6.0);
        assert_eq!(pts.first(), Some(&a));
        assert_eq!(pts.last(), Some(&b));
        assert!(pts.len() >= 6);
    }

    #[test]
    fn a_bolt_flickers_out_as_it_fades() {
        let mut bolt = TeslaBolt::new(Position::new(0.0, 0.0), Position::new(90.0, 0.0), 3);
        assert!(bolt_light(&bolt) > 0.5, "bright when fresh");
        while !bolt.tick(1.0 / 60.0) {}
        assert!(bolt_light(&bolt) <= 1e-6, "dark once its window is over");
    }
}
