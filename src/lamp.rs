//! Lamps (docs/volcano.md): a lamp post a map places - a tile, one shot
//! puts it out - and the lantern a player sets down in the dark. Both
//! light the ground round them at night (`weather::lights_in`), and an
//! enemy sees whoever stands in that light at full range
//! (`Game::sight_on`). This file is the drawing, generic over
//! `canvas::Canvas`, and the lantern's state.

use crate::canvas::Canvas;
use crate::math::Color;
use crate::pyro::{FIRE, SMOKE};
use crate::Position;

/// A lantern a seat has set down: where it stands, whose it is and the
/// round clock it was lit at (its flame steadies over its first second).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lantern {
    /// One counter per round, the wire's key (`net::wire::LampState`).
    pub id: u16,
    pub position: Position,
    pub seat: u8,
    pub lit_at: f32,
}

/// The flame's steps: gold, bright gold, pale.
const FLAME: [Color; 3] = [FIRE[4], FIRE[5], FIRE[6]];

/// How a lamp's flame flickers at `time`, 0.85..1: four steps, phased by
/// where it stands so two lamps never flicker together.
pub fn flicker(at: Position, time: f32) -> f32 {
    const STEPS: [f32; 4] = [1.0, 0.92, 0.97, 0.86];
    let phase = (crate::blast::seed_at(at, 61) % 4) as i64;
    STEPS[(((time * 7.0) as i64) + phase).rem_euclid(4) as usize]
}

/// A lamp post standing on cell centre `at`: a dark pole and its shadow,
/// a cross-arm and a glass lantern with its flame, lit from the top left.
pub fn draw_post(c: &mut impl Canvas, at: Position, time: f32, shadow_dir: (f32, f32), shadows: bool) {
    let (x, y) = (crate::pyro::snap(at.x), crate::pyro::snap(at.y));
    if shadows {
        let (sx, sy) = ((shadow_dir.0 * 8.0 / 2.0).round() as i32 * 2, (shadow_dir.1 * 8.0 / 2.0).round() as i32 * 2);
        let shade = Color::new(0, 0, 0, 80);
        c.fill_rect(x - 2 + sx, y - 12 + sy, 4, 22, shade);
        c.fill_rect(x - 6 + sx, y - 16 + sy, 12, 8, shade);
    }
    // The foot, the pole and its lit edge.
    c.fill_rect(x - 6, y + 6, 12, 4, SMOKE[0]);
    c.fill_rect(x - 4, y + 4, 8, 4, SMOKE[1]);
    c.fill_rect(x - 2, y - 10, 4, 16, SMOKE[0]);
    c.fill_rect(x - 2, y - 10, 2, 16, SMOKE[2]);
    // The lantern: a cap, glass round the flame, a base.
    c.fill_rect(x - 6, y - 20, 12, 2, SMOKE[0]);
    c.fill_rect(x - 4, y - 22, 8, 2, SMOKE[1]);
    c.fill_rect(x - 6, y - 18, 2, 8, SMOKE[0]);
    c.fill_rect(x + 4, y - 18, 2, 8, SMOKE[0]);
    c.fill_rect(x - 4, y - 18, 8, 8, FLAME[0]);
    let k = flicker(at, time);
    c.fill_rect(x - 2, y - 16, 4, if k > 0.9 { 6 } else { 4 }, FLAME[1]);
    c.fill_rect(x - 2, y - 14, 2, 2, FLAME[2]);
    c.fill_rect(x - 6, y - 10, 12, 2, SMOKE[1]);
}

/// A lantern set down on the ground at `at`: a small squat lamp with a
/// handle, its flame steadying over its first second.
pub fn draw_lantern(c: &mut impl Canvas, lantern: &Lantern, time: f32) {
    let at = lantern.position;
    let (x, y) = (crate::pyro::snap(at.x), crate::pyro::snap(at.y));
    c.fill_rect(x - 6, y + 2, 12, 4, Color::new(0, 0, 0, 70));
    c.fill_rect(x - 4, y - 10, 8, 2, SMOKE[1]);
    c.fill_rect(x - 2, y - 12, 4, 2, SMOKE[0]);
    c.fill_rect(x - 6, y - 8, 2, 8, SMOKE[0]);
    c.fill_rect(x + 4, y - 8, 2, 8, SMOKE[0]);
    c.fill_rect(x - 4, y - 8, 8, 8, FLAME[0]);
    let age = (time - lantern.lit_at).max(0.0);
    let k = if age < 1.0 { flicker(at, time * 3.0) } else { flicker(at, time) };
    c.fill_rect(x - 2, y - 6, 4, if k > 0.9 { 4 } else { 2 }, FLAME[1]);
    c.fill_rect(x - 2, y - 6, 2, 2, FLAME[2]);
    c.fill_rect(x - 6, y, 12, 2, SMOKE[0]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_post_and_a_lantern_draw_on_the_block_grid() {
        let mut canvas = crate::canvas::CpuCanvas::blank(96, 96);
        draw_post(&mut canvas, Position::new(32.0, 48.0), 1.0, (0.6, 0.5), true);
        draw_lantern(&mut canvas, &Lantern { id: 0, position: Position::new(70.0, 60.0), seat: 0, lit_at: 0.0 }, 2.0);
        assert!(canvas.pixels().iter().any(|p| *p == FLAME[0]));
    }

    #[test]
    fn flicker_stays_within_its_steps() {
        for i in 0..40 {
            let k = flicker(Position::new(64.0, 96.0), i as f32 * 0.05);
            assert!((0.85..=1.0).contains(&k));
        }
    }
}
