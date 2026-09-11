//! The player's tap feedback: where the standing order is going, and what
//! it means to do when it gets there (docs/tap-navigation.md).
//!
//! On a keyboard you always know what you told the tank, because you are
//! holding the key down. A tap is a fire-and-forget order, and without a
//! mark on the world the player cannot tell a tap that registered from one
//! that missed - which on a phone, where the finger covers the very pixel
//! it just pressed, is most of them.
//!
//! Two marks, deliberately different *shapes* rather than two colours of
//! one shape. The scene is 1280x720 letterboxed into a hand; shape survives
//! that scaling and hue does not, and a colourblind player gets nothing
//! from a blue-versus-orange distinction at all:
//!
//! - a **destination diamond** on the ground, for "the tank is going
//!   there";
//! - **corner brackets** hugging a thing's footprint, for "that one is
//!   selected" - the same mark on a health pack as on an enemy hull, so
//!   collecting and attacking read as the same kind of act.
//!
//! Colour is then free to carry only intent: cool for benign (move,
//! collect), hot for hostile (engage).
//!
//! Everything here snaps to the same 2-screen-pixel block every sprite in
//! the game lands on (`fx.rs`'s `FX_GRID` does this for particles, and
//! `blast::pixel_disc` for explosion glows). Drawn at sub-pixel positions
//! with smooth curves these read as a cleaner, higher-resolution game
//! layered over this one.

use sola_raylib::prelude::*;

use crate::Position;
use crate::tuning::tuning;

/// The block grid every mark lands on - the same 2 screen px as everything
/// else drawn over the battlefield.
const BLOCK: f32 = 2.0;

fn snap(v: f32) -> f32 {
    (v / BLOCK).round() * BLOCK
}

/// A dark halo drawn one block out around every mark. The battlefield runs
/// from near-black scorch to bright sand, and a single flat colour is
/// illegible on one end or the other; an outline makes the mark its own
/// background.
const OUTLINE: Color = Color::new(12, 16, 22, 150);

/// One axis-aligned block-quantised bar, outlined. Every mark below is
/// built from these, so nothing can drift off the grid and nothing can end
/// up unreadable against the ground it happens to land on.
fn bar(d: &mut impl RaylibDraw, x: f32, y: f32, w: f32, h: f32, color: Color) {
    let (x, y, w, h) = (snap(x), snap(y), snap(w).max(BLOCK), snap(h).max(BLOCK));
    let halo = Color::new(OUTLINE.r, OUTLINE.g, OUTLINE.b, (OUTLINE.a as f32 * (color.a as f32 / 255.0)) as u8);
    d.draw_rectangle_v(
        Vector2::new(x - BLOCK, y - BLOCK),
        Vector2::new(w + BLOCK * 2.0, h + BLOCK * 2.0),
        halo,
    );
    d.draw_rectangle_v(Vector2::new(x, y), Vector2::new(w, h), color);
}

/// A slow triangle wave in 0..=1 - the breathing every mark shares, so they
/// pulse together instead of beating against each other.
fn breath(time: f32, period: f32) -> f32 {
    let t = (time / period).fract();
    if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 }
}

/// "The tank is driving here." A diamond of four bars with a pip at its
/// centre, breathing inward - a reticle settling onto the spot rather than
/// a static dot, because a static dot on a busy battlefield reads as
/// scenery.
pub fn draw_destination(d: &mut impl RaylibDraw, at: Position, time: f32, color: Color) {
    let t = tuning();
    let pulse = breath(time, t.order_marker_pulse_seconds);
    let reach = t.order_marker_size_px * (0.82 + 0.18 * pulse);
    let alpha = |a: f32| Color::new(color.r, color.g, color.b, (color.a as f32 * a) as u8);

    // Four arms pointing in at the spot. Each is a short bar set out along
    // one axis, so the whole thing reads as a diamond without needing a
    // single diagonal line - which on this grid would be a staircase.
    let arm = (t.order_marker_size_px * 0.55).max(BLOCK * 2.0);
    let thick = BLOCK * 3.0;
    bar(d, at.x - thick * 0.5, at.y - reach, thick, arm, alpha(1.0));
    bar(d, at.x - thick * 0.5, at.y + reach - arm, thick, arm, alpha(1.0));
    bar(d, at.x - reach, at.y - thick * 0.5, arm, thick, alpha(1.0));
    bar(d, at.x + reach - arm, at.y - thick * 0.5, arm, thick, alpha(1.0));
    // The pip itself, steady, so there is always something exactly on the
    // destination even at the pulse's widest.
    bar(d, at.x - BLOCK * 1.5, at.y - BLOCK * 1.5, BLOCK * 3.0, BLOCK * 3.0, alpha(0.9));
}

/// "That one is selected." Four corner brackets just outside a target's
/// footprint - `half` is its half-extent, so the mark grows with a titan
/// and shrinks onto a health pack without any per-kind tuning.
pub fn draw_target_brackets(d: &mut impl RaylibDraw, at: Position, half: f32, time: f32, color: Color) {
    let t = tuning();
    let pulse = breath(time, t.order_marker_pulse_seconds);
    // Brackets sit *outside* the silhouette and close in as they pulse, so
    // they never cover the thing they are pointing at.
    let reach = half + t.order_bracket_gap_px * (0.6 + 0.4 * pulse);
    let arm = (half * 0.8).clamp(BLOCK * 5.0, t.order_marker_size_px);
    let thick = BLOCK * 2.0;

    for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let cx = at.x + sx * reach;
        let cy = at.y + sy * reach;
        // The horizontal leg runs back toward the centre from the corner,
        // the vertical leg down it - an L per corner.
        let hx = if sx < 0.0 { cx } else { cx - arm };
        let vy = if sy < 0.0 { cy } else { cy - arm };
        bar(d, hx, if sy < 0.0 { cy } else { cy - thick }, arm, thick, color);
        bar(d, if sx < 0.0 { cx } else { cx - thick }, vy, thick, arm, color);
    }
}
