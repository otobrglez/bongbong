//! Drawing the lobby (`lobby.rs` owns the model, the state machine and
//! every rect). One panel over the dimmed field, in field space like the
//! two dialogs, painted from the [`LobbyView`] a frame gathered.
//!
//! The QR goes through `canvas::Canvas::fill_rect` rather than raylib
//! directly, so the same whole-block drawing a test renders headless is
//! the one on screen; the text around it is raylib's, as all text in this
//! game is (`canvas.rs` has none).

use sola_raylib::prelude::*;

use crate::render::canvas::Sheets;
use crate::hud::{BUILD_COLOR, DIM, HUD_LABEL_SIZE, HUD_TEXT_SIZE, ONLINE_COLOR, TEXT};
use crate::lobby::{
    Button, ButtonView, LobbyView, SeatRow, Stage, button_rect, code_box_rect, content_rect, panel_rect, qr_rect,
    seat_row_rect, seats_rect, LOBBY_CODE_BOX, LOBBY_QR_BOX, LOBBY_SEAT_ROWS,
};
use crate::math::{Color, Rectangle};
use crate::render::canvas::GpuCanvas;
use crate::tank::TEAM_COLORS;
use crate::{qr, Rect};

/// The panel's fill and outline, the dialogs' so the three screens read
/// as one family.
const PANEL_FILL: Color = Color::new(20, 20, 24, 244);
const PANEL_SHADOW: Color = Color::new(0, 0, 0, 90);
const PANEL_EDGE: Color = Color::new(0, 0, 0, 150);
/// The dim over the field behind it, the dialogs' again.
const FIELD_DIM: Color = Color::new(0, 0, 0, 150);
/// A seat row's own backing, so the rows read as a list.
const ROW_FILL: Color = Color::new(255, 255, 255, 14);
const ROW_YOU: Color = Color::new(120, 220, 255, 26);
/// A disabled button.
const DEAD: Color = Color::new(70, 70, 78, 255);
/// The size the room code is set in, the one number a player reads out.
const CODE_TEXT_SIZE: i32 = 34;
/// The default font's advance per character, by size: the same
/// approximation the HUD's buttons are centred with.
const CHAR_W_18: i32 = 11;

/// Draw the whole screen over the field. Field space: call inside the
/// field camera, as `draw_leave_dialog` is called.
pub fn draw_lobby<D: RaylibDraw, S: Sheets>(d: &mut D, field: Rect, view: &LobbyView, sheets: &S) {
    d.draw_rectangle(0, 0, field.w.round() as i32, field.h.round() as i32, FIELD_DIM);
    let panel = panel_rect(field);
    d.draw_rectangle_rounded(Rectangle::new(panel.x + 4.0, panel.y + 4.0, panel.width, panel.height), 0.05, 8, PANEL_SHADOW);
    d.draw_rectangle_rounded(panel, 0.05, 8, PANEL_FILL);
    d.draw_rectangle_rounded_lines_ex(panel, 0.05, 8, 1.5, PANEL_EDGE);

    let c = content_rect(field);
    d.draw_text(&view.title, c.x as i32, c.y as i32, 22, ONLINE_COLOR);
    let sub_y = c.y as i32 + 26;
    match view.stage {
        // In the room the seats take the space the subtitle would, so it
        // sits along the bottom instead, beside the buttons.
        Stage::Room => {}
        _ => d.draw_text(&view.sub, c.x as i32, sub_y, HUD_LABEL_SIZE + 2, DIM),
    }

    match view.stage {
        Stage::Start => draw_start(d, field, view),
        Stage::Code => draw_code(d, field, view),
        Stage::Waiting | Stage::Closed => {}
        Stage::Room => draw_room(d, field, view, sheets),
    }
    for button in &view.buttons {
        draw_button(d, button_rect(field, button.button), button);
    }
}

/// The opening face: the two steppers and what they stand on.
fn draw_start<D: RaylibDraw>(d: &mut D, field: Rect, view: &LobbyView) {
    let c = content_rect(field);
    for (row, (label, value)) in [("MAP", view.map.as_str()), ("MISSION", view.mission.name())].into_iter().enumerate() {
        let prev = button_rect(field, if row == 0 { Button::MapPrev } else { Button::MissionPrev });
        let mid_y = (prev.y + (prev.height - HUD_TEXT_SIZE as f32) / 2.0) as i32;
        d.draw_text(label, c.x as i32, mid_y, HUD_TEXT_SIZE, DIM);
        // Left-aligned a fixed step in from the `<` button rather than
        // centred between the two: the default font's advance is only
        // approximated here, and a value that stays put as it changes
        // reads better than one that drifts.
        d.draw_text(&value.to_ascii_uppercase(), (prev.x + prev.width + 24.0) as i32, mid_y, HUD_TEXT_SIZE, TEXT);
    }
}

/// The code entry: five boxes and what has been typed into them.
fn draw_code<D: RaylibDraw>(d: &mut D, field: Rect, view: &LobbyView) {
    let typed: Vec<char> = view.entry.chars().collect();
    for i in 0..crate::net::rooms::CODE_LETTERS {
        let r = code_box_rect(field, i);
        let filled = i < typed.len();
        d.draw_rectangle_rounded(r, 0.15, 8, if filled { ROW_YOU } else { ROW_FILL });
        d.draw_rectangle_rounded_lines_ex(r, 0.15, 8, 2.0, if i == typed.len() { ONLINE_COLOR } else { DIM });
        if let Some(&c) = typed.get(i) {
            let size = 40;
            let w = size * 3 / 5;
            d.draw_text(
                &c.to_string(),
                (r.x + (LOBBY_CODE_BOX - w as f32) / 2.0) as i32,
                (r.y + (LOBBY_CODE_BOX - size as f32) / 2.0) as i32,
                size,
                TEXT,
            );
        }
    }
}

/// The room: the seats on the left, the code and its QR on the right.
fn draw_room<D: RaylibDraw, S: Sheets>(d: &mut D, field: Rect, view: &LobbyView, sheets: &S) {
    let c = content_rect(field);
    let seats = seats_rect(field);
    for row in 0..LOBBY_SEAT_ROWS {
        draw_seat(d, seat_row_rect(field, row), view.seats.get(row));
    }
    if view.more > 0 {
        let y = (seats.y + seats.height + 2.0) as i32;
        d.draw_text(&format!("+{} MORE", view.more), seats.x as i32, y, HUD_LABEL_SIZE, DIM);
    }
    // The line under the title has nowhere to go in this face; it runs
    // along the bottom between the buttons instead.
    let bottom = button_rect(field, Button::Ready);
    let sub_y = (bottom.y - 18.0) as i32;
    d.draw_text(&view.sub, c.x as i32, sub_y, HUD_LABEL_SIZE, DIM);

    let box_rect = qr_rect(field);
    if let Some(code) = &view.qr {
        let scale = code.scale_for(LOBBY_QR_BOX as i32);
        let side = code.padded_size() * scale;
        let x = (box_rect.x + (LOBBY_QR_BOX - side as f32) / 2.0) as i32;
        let y = (box_rect.y + (LOBBY_QR_BOX - side as f32) / 2.0) as i32;
        // One statement: the canvas borrows the draw handle for it.
        qr::draw(&mut GpuCanvas::new(d, sheets), code, x, y, scale, Color::BLACK, Color::WHITE);
    }
    if let Some(code) = &view.code {
        let w = code.len() as i32 * (CODE_TEXT_SIZE * 3 / 5 + 3);
        d.draw_text(
            code,
            (box_rect.x + (LOBBY_QR_BOX - w as f32) / 2.0) as i32,
            (box_rect.y + box_rect.height + 6.0) as i32,
            CODE_TEXT_SIZE,
            TEXT,
        );
    }
    if let Some(url) = &view.join_url {
        // The link is long with a local override; the tail is what
        // differs, so the head is what gets cut.
        let shown: String = match url.len() > 34 {
            true => format!("...{}", &url[url.len() - 31..]),
            false => url.clone(),
        };
        d.draw_text(
            &shown,
            box_rect.x as i32,
            (box_rect.y + box_rect.height + 6.0 + CODE_TEXT_SIZE as f32 + 2.0) as i32,
            HUD_LABEL_SIZE,
            DIM,
        );
    }
}

/// One seat's row, or an empty slot.
fn draw_seat<D: RaylibDraw>(d: &mut D, r: Rectangle, seat: Option<&SeatRow>) {
    let inner = Rectangle::new(r.x, r.y + 3.0, r.width, r.height - 6.0);
    let Some(seat) = seat else {
        d.draw_rectangle_rounded_lines_ex(inner, 0.2, 8, 1.0, Color::new(70, 70, 78, 140));
        d.draw_text("EMPTY", (r.x + 12.0) as i32, (r.y + (r.height - HUD_LABEL_SIZE as f32) / 2.0) as i32, HUD_LABEL_SIZE, DEAD);
        return;
    };
    d.draw_rectangle_rounded(inner, 0.2, 8, if seat.you { ROW_YOU } else { ROW_FILL });
    let text_y = (r.y + (r.height - HUD_TEXT_SIZE as f32) / 2.0) as i32;
    // Seats past the two team blocks share their colours, as their
    // hulls do until `gen_tanks.py` draws more (docs/online-coop-prd.md §4.11).
    let slot_color = TEAM_COLORS[seat.seat as usize % TEAM_COLORS.len()];
    d.draw_text(&seat.slot, (r.x + 10.0) as i32, text_y, HUD_TEXT_SIZE, slot_color);
    d.draw_text(&seat.nick, (r.x + 46.0) as i32, text_y, HUD_TEXT_SIZE, if seat.you { TEXT } else { Color::new(210, 210, 216, 255) });
    d.draw_text(seat.chassis, (r.x + 190.0) as i32, text_y + 4, HUD_LABEL_SIZE, DIM);
    let state_color = match (seat.host, seat.ready) {
        (true, _) => ONLINE_COLOR,
        (_, true) => BUILD_COLOR,
        _ => DIM,
    };
    d.draw_text(seat.state, (r.x + 280.0) as i32, text_y + 4, HUD_LABEL_SIZE, state_color);
}

/// One button: the dialogs' outlined rounded box, dim when dead.
fn draw_button<D: RaylibDraw>(d: &mut D, r: Rectangle, view: &ButtonView) {
    let color = match (view.enabled, view.accent) {
        (false, _) => DEAD,
        (true, true) => ONLINE_COLOR,
        (true, false) => TEXT,
    };
    if view.enabled && view.accent {
        d.draw_rectangle_rounded(r, 0.2, 8, Color::new(120, 220, 255, 30));
    }
    d.draw_rectangle_rounded_lines_ex(r, 0.2, 8, 2.0, color);
    let size = if view.label.chars().count() == 1 { 24 } else { HUD_TEXT_SIZE };
    let w = view.label.len() as i32 * if size == 24 { 14 } else { CHAR_W_18 };
    d.draw_text(
        &view.label,
        (r.x + (r.width - w as f32) / 2.0) as i32,
        (r.y + (r.height - size as f32) / 2.0) as i32,
        size,
        color,
    );
}

/// The bar's `ONLINE` button: the mode button's frame in the room blue,
/// so the three slots at the bar's right end read as one row.
pub fn draw_online_button<D: RaylibDraw>(d: &mut D, panel: Rect) {
    let r = crate::hud::online_button_rect(panel);
    d.draw_rectangle_lines_ex(Rectangle::new(r.x, r.y + 2.0, r.width, r.height - 4.0), 2.0, ONLINE_COLOR);
    let label = "ONLINE";
    let w = label.len() as i32 * CHAR_W_18;
    d.draw_text(
        label,
        (r.x + (r.width - w as f32) / 2.0) as i32,
        (r.y + (r.height - HUD_TEXT_SIZE as f32) / 2.0) as i32,
        HUD_TEXT_SIZE,
        ONLINE_COLOR,
    );
}
