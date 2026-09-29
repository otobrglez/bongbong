//! The `P1`..`P8` locate label, the one thing drawn on a tank that is text
//! (`tank.rs` owns the entity, the rings and the sprite layers).

use sola_raylib::prelude::*;

use crate::hud::HUD_TEXT_SIZE;
use crate::tank::{player_locate_active, team_color, Tank, BLACK};
use crate::text::{keys, text, width};

/// The locate cue's label, `P1`..`P8` in the seat's team colour just above
/// the hull, drawn over everything so a crowd cannot cover it. Same window
/// as `draw_player_locate`, and the one thing that tells the seats past
/// the second apart from player 1, whose sheet block they share.
pub fn draw_player_label(d: &mut impl RaylibDraw, tank: &Tank, elapsed: f32) {
    let Some(index) = tank.player_index() else { return };
    if tank.is_wreck() || !player_locate_active(elapsed) {
        return;
    }
    let text = text().fmt(keys::SEAT_LABEL, &[("n", (index + 1).into())]);
    let text = text.as_str();
    let size = HUD_TEXT_SIZE;
    // Measured headless (`text::width`): nothing in a draw pass has the
    // handle `MeasureText` needs.
    let w = width(text, size);
    let x = (tank.position.x - w as f32 / 2.0).round() as i32;
    let y = (tank.position.y - tank.size() / 2.0 - size as f32 - 4.0).round() as i32;
    let color = team_color(index);
    d.draw_text(text, x + 1, y + 1, size, BLACK);
    d.draw_text(text, x, y, size, color);
}
