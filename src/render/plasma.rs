//! Drawing a plasma bolt - its runtime glow halo and the baked sprite -
//! and its shadow (`plasma.rs` owns the entity and the frame arithmetic).

use sola_raylib::prelude::*;

use crate::math::{Color, Rectangle};
use crate::plasma::{Plasma, PlasmaState, PlasmaVariant};
use crate::render::blast::pixel_disc;
use crate::render::shot_fx::{fade, heading, pixel_streak};
use crate::tuning::tuning;
use crate::{Position, PLASMA_SCALE, PLASMA_TEXTURE_SIZE};

impl PlasmaVariant {
    /// Which row of plasma.png this variant draws from - a genuine second
    /// colour pass (see docs/PLASMA_SPEC.md), not a runtime tint over one
    /// shared row: `draw_texture_pro`'s tint is a per-channel multiply, which
    /// can only ever darken/filter a pixel toward the tint colour, never
    /// invert a channel that started at zero - tinting the (zero-red) teal
    /// glow body purple was tried and just produced a darker blue with
    /// purple-tinted white highlights, not a purple bolt.
    fn row(self) -> i32 {
        match self {
            PlasmaVariant::Teal => 0,
            PlasmaVariant::Purple => 1,
        }
    }

    /// (outer, inner) glow-halo colours at full alpha, for
    /// `draw_plasma_light`'s halo and trail - drawn fresh each frame (not sampled from
    /// the sprite), matched to this variant's own baked row
    /// (`tools/spritegen/gen_plasma.py`'s `TEAL`/`PURPLE` palettes) so the
    /// halo and the sprite read as the same colour.
    fn glow_colors(self) -> (Color, Color) {
        match self {
            PlasmaVariant::Teal => (Color::new(40, 220, 200, 255), Color::new(200, 255, 245, 255)),
            PlasmaVariant::Purple => {
                (Color::new(155, 77, 224, 255), Color::new(230, 205, 255, 255))
            }
        }
    }
}

/// Column of this state in plasma.png - never asked for `Flying`, whose
/// four columns `flying_col` cycles through instead.
fn state_col(state: PlasmaState) -> i32 {
    state.col()
}

/// Source rectangle for a plasma frame at sheet column `col`, row
/// `variant.row()` in plasma.png - no chassis variants (see `plasma.rs`'s
/// doc comment), just the two `PlasmaVariant` rows.
fn source_rec(col: i32, variant: PlasmaVariant) -> Rectangle {
    Rectangle::new(
        col as f32 * PLASMA_TEXTURE_SIZE,
        variant.row() as f32 * PLASMA_TEXTURE_SIZE,
        PLASMA_TEXTURE_SIZE,
        PLASMA_TEXTURE_SIZE,
    )
}

/// Which of `Flying`'s 4 baked breathing-cycle columns (3, 4, 5, 6) to draw
/// right now, from `plasma.timer` (see its doc comment) - cycles forward at
/// PLASMA_FLYING_CYCLE_FPS, wrapping every 4 frames. A plain forward cycle
/// (0,1,2,3,0,1,2,...), not a phase-matched sine like the runtime glow's own
/// `glow_pulse` - `gen_plasma.py`'s 4 frames are themselves authored as a
/// dim->bright->dim breathing loop (see docs/PLASMA_SPEC.md), so a steady
/// cycle through them already reads as pulsing without needing to sample a
/// continuous curve.
fn flying_col(timer: f32) -> i32 {
    3 + (timer * tuning().plasma_flying_cycle_fps) as i32 % 4
}

/// The in-flight glow halo's current radius/alpha, derived from `timer`
/// (see its doc comment) - a sine wave over PLASMA_PULSE_HZ cycles/second,
/// remapped from the base sprite's own half-size into
/// PLASMA_PULSE_MIN_SCALE..MAX_SCALE. Shared by `draw_plasma_light`'s halo
/// passes so they always pulse in lockstep. Deliberately a different cycle
/// rate/shape than the baked `flying_col` animation - see
/// PLASMA_FLYING_CYCLE_FPS's doc comment.
fn glow_pulse(plasma: &Plasma) -> (f32, f32) {
    let base_radius = PLASMA_TEXTURE_SIZE * PLASMA_SCALE * 0.5;
    let phase = (plasma.timer * tuning().plasma_pulse_hz * std::f32::consts::TAU).sin() * 0.5 + 0.5;
    let scale = tuning().plasma_pulse_min_scale + (tuning().plasma_pulse_max_scale - tuning().plasma_pulse_min_scale) * phase;
    (base_radius * scale, phase)
}

/// Draw a plasma bolt's sprite from its current frame (`flying_col`
/// while `Flying`, `PlasmaState::col` otherwise) at `plasma.variant`'s own
/// sheet row - centered and rotated to face travel, same as
/// `draw_shell`/`draw_bullet`. Its light is `draw_plasma_light`, drawn
/// first in the additive block so the sprite composites over it.
pub fn draw_plasma(d: &mut impl RaylibDraw, texture: &Texture2D, plasma: &Plasma) {
    let col = if plasma.state == PlasmaState::Flying {
        flying_col(plasma.timer)
    } else {
        state_col(plasma.state)
    };
    let src = source_rec(col, plasma.variant);
    let size = PLASMA_TEXTURE_SIZE * PLASMA_SCALE;
    let dest = Rectangle::new(plasma.position.x, plasma.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);
    d.draw_texture_pro(texture, src, dest, origin, plasma.rotation, Color::WHITE);
}

/// A flying bolt's light (call inside the additive block): the pulsating
/// halo - two concentric block-built discs sized and faded by
/// `glow_pulse`, coloured by `PlasmaVariant::glow_colors` - over a chain
/// of shrinking, dimming afterimages strung back along its path
/// (`plasma_trail_length`), so the bolt streaks like a comet rather than
/// sliding. A runtime effect over the sprite's own baked breathing
/// animation, not a replacement for it.
pub fn draw_plasma_light(d: &mut impl RaylibDraw, plasma: &Plasma) {
    let strength = tuning().shot_glow_strength;
    if plasma.state != PlasmaState::Flying || strength <= 0.0 {
        return;
    }
    let (radius, phase) = glow_pulse(plasma);
    let (glow_outer, glow_inner) = plasma.variant.glow_colors();
    let dir = heading(plasma.rotation);
    let trail = tuning().plasma_trail_length;
    const GHOSTS: i32 = 5;
    for i in (1..=GHOSTS).rev() {
        let t = i as f32 / GHOSTS as f32;
        let at = Position::new(plasma.position.x - dir.x * trail * t, plasma.position.y - dir.y * trail * t);
        pixel_disc(d, at, radius * 0.5 * (1.0 - 0.6 * t), fade(glow_outer, strength * 0.6 * (1.0 - t)));
    }
    pixel_streak(d, plasma.position, dir, trail, 1, fade(glow_inner, strength * 0.9), fade(glow_outer, 0.0));
    let pulse = (90.0 + 90.0 * phase) / 255.0;
    pixel_disc(d, plasma.position, radius * 0.75, fade(glow_outer, strength * pulse * 0.6));
    pixel_disc(d, plasma.position, radius * 0.4, fade(glow_inner, strength * pulse * 0.55));
}

/// Draw this bolt's drop shadow - same tint/offset convention as
/// `draw_shell_shadow`, no glow halo (a shadow is a flat silhouette, not a
/// light source) - still reads from `plasma.variant`'s own row since the two
/// rows aren't pixel-identical (unlike shells.png's Flying column), just
/// drawn as a flat black silhouette regardless of colour, same as every
/// other shadow pass in the game. Caller (`Game::render`) only calls this
/// while `plasma.state == PlasmaState::Flying`.
pub fn draw_plasma_shadow(d: &mut impl RaylibDraw, texture: &Texture2D, plasma: &Plasma) {
    let src = source_rec(flying_col(plasma.timer), plasma.variant);
    let size = PLASMA_TEXTURE_SIZE * PLASMA_SCALE;
    let dest = Rectangle::new(
        plasma.position.x + tuning().shadow_dir_x * plasma.shadow_offset,
        plasma.position.y + tuning().shadow_dir_y * plasma.shadow_offset,
        size,
        size,
    );
    let origin = Vector2::new(size / 2.0, size / 2.0);
    let shadow = Color::new(0, 0, 0, (255.0 * tuning().plasma_shadow_opacity) as u8);
    d.draw_texture_pro(texture, src, dest, origin, plasma.rotation, shadow);
}
