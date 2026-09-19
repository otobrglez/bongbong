//! Drawing a shell and its shadow (`shell.rs` owns the entity).

use sola_raylib::prelude::*;

use crate::math::{Color, Rectangle};
use crate::shell::{Shell, ShellState};
use crate::tuning::tuning;
use crate::{SHELL_SCALE, SHELL_TEXTURE_SIZE};

/// Column of this state in the shells sprite sheet.
fn state_col(state: ShellState) -> i32 {
    state.col()
}

/// Source rectangle for a shell frame (variant row, state column) in shells.png.
fn source_rec(variant: i32, col: i32) -> Rectangle {
    Rectangle::new(
        col as f32 * SHELL_TEXTURE_SIZE,
        variant as f32 * SHELL_TEXTURE_SIZE,
        SHELL_TEXTURE_SIZE,
        SHELL_TEXTURE_SIZE,
    )
}

/// Draw a shell using its current state's frame (from its variant row),
/// centered and rotated to face travel.
pub fn draw_shell(d: &mut impl RaylibDraw, texture: &Texture2D, shell: &Shell) {
    let src = source_rec(shell.variant, state_col(shell.state));
    let size = SHELL_TEXTURE_SIZE * SHELL_SCALE;

    let dest = Rectangle::new(shell.position.x, shell.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);

    d.draw_texture_pro(texture, src, dest, origin, shell.rotation, Color::WHITE);
}

/// Draw this shell's drop shadow: same sprite/rotation, offset further than a
/// tank's shadow so the gap between shell and shadow reads as height - see
/// docs/sprite-shadows-design.md. The offset itself is `shell.shadow_offset`
/// (rolled once per shell at fire time, see the field doc on `Shell`), not a
/// flat constant, so different shells appear to fly at different heights.
/// Caller (`Game::render`) only calls this while `shell.state ==
/// ShellState::Flying`; the fire/impact frames are stationary blast sprites,
/// not airborne objects, so they get no shadow.
pub fn draw_shell_shadow(d: &mut impl RaylibDraw, texture: &Texture2D, shell: &Shell) {
    let src = source_rec(shell.variant, state_col(shell.state));
    let size = SHELL_TEXTURE_SIZE * SHELL_SCALE;

    let dest = Rectangle::new(
        shell.position.x + tuning().shadow_dir_x * shell.shadow_offset,
        shell.position.y + tuning().shadow_dir_y * shell.shadow_offset,
        size,
        size,
    );
    let origin = Vector2::new(size / 2.0, size / 2.0);
    let shadow = Color::new(0, 0, 0, (255.0 * tuning().shell_shadow_opacity) as u8);

    d.draw_texture_pro(texture, src, dest, origin, shell.rotation, shadow);
}
