//! The `P1`/`P2` locate label, the one thing drawn on a tank that is text
//! (`tank.rs` owns the entity, the rings and the sprite layers).

use sola_raylib::prelude::*;

use crate::hud::HUD_TEXT_SIZE;
use crate::render::hud::CHAR_W;
use crate::tank::{player_locate_active, Tank, BLACK, TEAM_COLORS};

/// The locate cue's label, `P1`/`P2` in the team colour just above the
/// hull, drawn over everything so a crowd cannot cover it. Same window as
/// `draw_player_locate`.
pub fn draw_player_label(d: &mut impl RaylibDraw, tank: &Tank, elapsed: f32) {
    let Some(index) = tank.player_index() else { return };
    if tank.is_wreck() || !player_locate_active(elapsed) {
        return;
    }
    let text = if index == 0 { "P1" } else { "P2" };
    let size = HUD_TEXT_SIZE;
    // The HUD's fixed cell width for this size; measuring needs the handle,
    // which nothing in a draw pass has.
    let w = text.len() as i32 * CHAR_W;
    let x = (tank.position.x - w as f32 / 2.0).round() as i32;
    let y = (tank.position.y - tank.size() / 2.0 - size as f32 - 4.0).round() as i32;
    let color = TEAM_COLORS[index as usize & 1];
    d.draw_text(text, x + 1, y + 1, size, BLACK);
    d.draw_text(text, x, y, size, color);
}
