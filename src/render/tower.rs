//! The raylib half of `tower.rs` (docs/defence-towers-prd.md section 12):
//! the tesla coil's bolt - three violet filaments twisting round each other
//! with a ring of sparks where they land, the author's L3 pick - and the
//! light the towers throw: a charging coil's lens, the ooze in a mortar's
//! mouth, a glob in the air, a burning tower. Everything here is additive
//! and drawn inside the caller's additive block, except the bolt, which
//! opens its own.

use sola_raylib::prelude::*;

use crate::math::Color;
use crate::render::shot_fx::{fade, glow, ground_light};
use crate::simulation::Game;
use crate::tower::{Glob, TeslaBolt, TowerKind, TowerView, OOZE_LT};
use crate::Position;

/// The bolt's three strands: pale violet, white-hot, deep violet - the
/// plasma cannon's arcane family, so the tesla reads apart from the teal
/// plasma bolt.
const STRAND_PALE: Color = Color::new(0xca, 0xa6, 0xff, 255);
const STRAND_CORE: Color = Color::new(0xff, 0xff, 0xff, 255);
const STRAND_DEEP: Color = Color::new(0x9a, 0x66, 0xff, 255);
const BOLT_GLOW: Color = Color::new(0xb0, 0x78, 0xff, 255);

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

fn polyline(d: &mut impl RaylibDraw, pts: &[Position], width: f32, color: Color) {
    for pair in pts.windows(2) {
        d.draw_line_ex(pair[0], pair[1], width, color);
    }
}

/// One tesla bolt: a glow at both ends, three filaments re-jagged three
/// times over its short life so it crackles, and a ring of 2 px sparks
/// widening round where it struck.
pub fn draw_tesla_bolt(d: &mut impl RaylibDraw, bolt: &TeslaBolt) {
    let alpha = bolt.alpha();
    let flicker = ((1.0 - alpha) * 3.0).floor() as u32;
    let seed = bolt.seed.wrapping_add(flicker.wrapping_mul(977));
    d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
        glow(&mut bd, bolt.start, 18.0, fade(BOLT_GLOW, 0.45 * alpha));
        glow(&mut bd, bolt.end, 24.0, fade(BOLT_GLOW, 0.55 * alpha));
        for (i, (color, width)) in [(STRAND_PALE, 2.0), (STRAND_CORE, 1.5), (STRAND_DEEP, 2.0)].into_iter().enumerate() {
            let pts = jagged(bolt.start, bolt.end, seed.wrapping_add(i as u32 * 11), 6.0);
            polyline(&mut bd, &pts, width, fade(color, alpha));
        }
        let r = 6.0 + (1.0 - alpha) * 12.0;
        let mut a = 0.0f32;
        while a < std::f32::consts::TAU {
            let color = if (a * 10.0) as i32 % 3 == 0 { STRAND_CORE } else { STRAND_PALE };
            let x = ((bolt.end.x + a.cos() * r) / 2.0).round() * 2.0;
            let y = ((bolt.end.y + a.sin() * r * 0.8) / 2.0).round() * 2.0;
            bd.draw_rectangle(x as i32, y as i32, 2, 2, fade(color, alpha));
            a += 0.3;
        }
    });
}

/// The light a standing tower throws (inside an additive block): a
/// charging coil's lens and prong tips brightening toward the strike, the
/// ooze glowing in a mortar's mouth, a burning tower's fire.
pub fn draw_tower_glow(d: &mut impl RaylibDraw, view: &TowerView, time: f32) {
    let at = view.position;
    match view.kind {
        TowerKind::Tesla if view.charge > 0.05 => {
            let k = view.charge * view.charge;
            glow(d, at, 12.0 + 20.0 * k, fade(BOLT_GLOW, 0.5 * k));
            ground_light(d, at, 40.0 + 30.0 * k, BOLT_GLOW, 0.35 * k);
        }
        TowerKind::Bio => {
            let pulse = 0.85 + 0.15 * (time * 3.1 + at.x * 0.01).sin();
            let r = view.heading.to_radians();
            let mouth = Position::new(at.x + r.sin() * 12.0, at.y - r.cos() * 12.0);
            glow(d, mouth, 10.0 + 8.0 * view.charge, fade(OOZE_LT, 0.35 * pulse * (0.3 + 0.7 * view.charge)));
            ground_light(d, at, 36.0, OOZE_LT, 0.12 * pulse);
        }
        _ => {}
    }
    if view.burning {
        let flicker = 0.8 + 0.2 * (time * 23.0 + at.y * 0.1).sin();
        glow(d, Position::new(at.x, at.y - 4.0), 26.0, fade(Color::new(255, 140, 50, 255), 0.35 * flicker));
    }
}

/// A glob's own light, over its sprite (inside an additive block).
pub fn draw_glob_glow(d: &mut impl RaylibDraw, glob: &Glob) {
    let g = glob.ground_pos();
    let at = Position::new(g.x, g.y - glob.height());
    glow(d, at, 14.0, fade(OOZE_LT, 0.45));
    ground_light(d, g, 16.0, OOZE_LT, 0.2);
}

impl Game {
    /// Every standing tower's light (additive block).
    pub(crate) fn draw_towers_light(&self, d: &mut impl RaylibDraw) {
        for view in self.tower_views() {
            draw_tower_glow(d, &view, self.time);
        }
        for glob in &self.globs {
            draw_glob_glow(d, glob);
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
}
