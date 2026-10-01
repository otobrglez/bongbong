//! Drawing a plasma bolt from its baked sprite, and its shadow; the orb
//! it flies as is `render/shot_shaders.rs` (`plasma.rs` owns the entity and the frame arithmetic).

use sola_raylib::prelude::*;

use crate::math::{Color, Rectangle};
use crate::plasma::{Plasma, PlasmaState, PlasmaVariant};
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
/// (0,1,2,3,0,1,2,...) - `gen_plasma.py`'s 4 frames are themselves authored as a
/// dim->bright->dim breathing loop (see docs/PLASMA_SPEC.md), so a steady
/// cycle through them already reads as pulsing without needing to sample a
/// continuous curve.
fn flying_col(timer: f32) -> i32 {
    3 + (timer * tuning().plasma_flying_cycle_fps) as i32 % 4
}

impl PlasmaVariant {
    /// The shader orb's four colours - deep, body, bright, hot - in the
    /// same family as this variant's baked row.
    pub(crate) fn orb_colors(self) -> [Color; 4] {
        match self {
            PlasmaVariant::Teal => [
                Color::new(10, 78, 98, 255),
                Color::new(24, 164, 176, 255),
                Color::new(110, 236, 222, 255),
                Color::new(236, 255, 250, 255),
            ],
            PlasmaVariant::Purple => [
                Color::new(58, 22, 104, 255),
                Color::new(128, 60, 204, 255),
                Color::new(196, 144, 255, 255),
                Color::new(248, 234, 255, 255),
            ],
        }
    }
}

/// Draw a plasma bolt from its baked sprite: the current frame
/// (`flying_col` while `Flying`, `PlasmaState::col` otherwise) from
/// `plasma.variant`'s own row of plasma.png, centred and rotated to face
/// travel like `draw_shell`/`draw_bullet`. In flight the game draws the
/// shader orb instead (`render::shot_shaders::ShotShaders::draw_orb`)
/// wherever the shaders loaded; this is the muzzle and impact frames, and
/// the whole flight where they did not.
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

/// Draw this bolt's drop shadow - same tint/offset convention as
/// `draw_shell_shadow`, no glow halo (a shadow is a flat silhouette, not a
/// light source) - still reads from `plasma.variant`'s own row since the two
/// rows aren't pixel-identical (unlike shells.png's Flying column), just
/// drawn as a flat black silhouette regardless of colour, same as every
/// other shadow pass in the game. Caller (`Game::render`) only calls this
/// while `plasma.state == PlasmaState::Flying`.
/// `orb` says the shader orb is what flies, whose shadow is its own round
/// silhouette rather than the sprite's.
pub fn draw_plasma_shadow(d: &mut impl RaylibDraw, texture: &Texture2D, plasma: &Plasma, orb: bool) {
    if orb {
        let at = Position::new(
            plasma.position.x + tuning().shadow_dir_x * plasma.shadow_offset,
            plasma.position.y + tuning().shadow_dir_y * plasma.shadow_offset,
        );
        let a = (255.0 * tuning().plasma_shadow_opacity) as u8;
        crate::pyro::block_disc(&mut crate::render::pyro::Rl(d), at, tuning().plasma_orb_radius * 0.95, Color::new(0, 0, 0, a));
        return;
    }
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
