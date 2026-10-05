//! Drawing the lobby (`lobby.rs` owns the model, the state machine and
//! every rect). One panel over the dimmed window, in UI points like the
//! two dialogs, painted from the [`LobbyView`] a frame gathered.
//!
//! The QR goes through `canvas::Canvas::fill_rect` rather than raylib
//! directly, so the same whole-block drawing a test renders headless is
//! the one on screen; the text around it is raylib's, as all text in this
//! game is (`canvas.rs` has none).

use sola_raylib::prelude::*;

use crate::render::canvas::Sheets;
use crate::hud::{UiFrame, BUILD_COLOR, DIM, HUD_TEXT_SIZE, ONLINE_COLOR, TEXT, UI_SMALL_TEXT};
use crate::lobby::{
    Button, ButtonView, LobbyView, SeatRow, Stage, button_rect, code_box_rect, content_rect, panel_rect, qr_rect,
    seat_row_rect, seats_rect, LOBBY_CODE_BOX, LOBBY_MARGIN, LOBBY_QR_BOX, LOBBY_SEAT_CHASSIS_X, LOBBY_SEAT_NICK_X,
    LOBBY_SEAT_ROWS, LOBBY_SEAT_STATE_X,
};
use crate::math::{Color, Rectangle};
use crate::render::canvas::GpuCanvas;
use crate::tank::TEAM_COLORS;
use crate::text::{keys, mission_title, text, width};
use crate::{qr, Rect};

/// The panel's fill and outline, the dialogs' so the three screens read
/// as one family.
const PANEL_FILL: Color = Color::new(20, 20, 24, 244);
const PANEL_SHADOW: Color = Color::new(0, 0, 0, 90);
const PANEL_EDGE: Color = Color::new(0, 0, 0, 150);
/// The dim over the window behind it, the dialogs' again.
const WINDOW_DIM: Color = Color::new(0, 0, 0, 150);
/// A seat row's own backing, so the rows read as a list.
const ROW_FILL: Color = Color::new(255, 255, 255, 14);
const ROW_YOU: Color = Color::new(120, 220, 255, 26);
/// A disabled button.
const DEAD: Color = Color::new(70, 70, 78, 255);
/// The size the room code is set in, the one number a player reads out.
const CODE_TEXT_SIZE: i32 = 34;

/// Draw the whole screen over the window: the dim over all of it, the
/// panel centred in the chrome's area. In UI points: call inside the UI
/// camera, as `draw_leave_dialog` is called.
pub fn draw_lobby<D: RaylibDraw, S: Sheets>(d: &mut D, ui: &UiFrame, view: &LobbyView, sheets: &S) {
    let (screen, area) = (ui.screen, ui.area);
    d.draw_rectangle(0, 0, screen.w.ceil() as i32, screen.h.ceil() as i32, WINDOW_DIM);
    let panel = panel_rect(area);
    d.draw_rectangle_rounded(Rectangle::new(panel.x + 4.0, panel.y + 4.0, panel.width, panel.height), 0.05, 8, PANEL_SHADOW);
    d.draw_rectangle_rounded(panel, 0.05, 8, PANEL_FILL);
    d.draw_rectangle_rounded_lines_ex(panel, 0.05, 8, 1.5, PANEL_EDGE);

    let c = content_rect(area);
    d.draw_text(&view.title, c.x as i32, c.y as i32, 22, ONLINE_COLOR);
    let sub_y = c.y as i32 + 26;
    match view.stage {
        // In the room the seats take the space the subtitle would, so it
        // sits along the bottom instead, beside the buttons.
        Stage::Room => {}
        _ => d.draw_text(&view.sub, c.x as i32, sub_y, UI_SMALL_TEXT, DIM),
    }

    match view.stage {
        Stage::Start => draw_start(d, area, view),
        Stage::Code => draw_code(d, area, view),
        Stage::Waiting | Stage::Closed => {}
        Stage::Room => draw_room(d, area, view, sheets),
    }
    for button in &view.buttons {
        draw_button(d, button_rect(area, button.button), button);
    }
}

/// The opening face: the two steppers and what they stand on.
fn draw_start<D: RaylibDraw>(d: &mut D, area: Rect, view: &LobbyView) {
    let c = content_rect(area);
    let t = text();
    // The map is its slug, a data name every build spells the same; the
    // mission is the word for it in the language on screen.
    let rows = [(t.get(keys::LOBBY_MAP), view.map.to_ascii_uppercase()), (t.get(keys::LOBBY_MISSION), t.get(mission_title(view.mission)))];
    for (row, (label, value)) in rows.into_iter().enumerate() {
        let prev = button_rect(area, if row == 0 { Button::MapPrev } else { Button::MissionPrev });
        let mid_y = (prev.y + (prev.height - HUD_TEXT_SIZE as f32) / 2.0) as i32;
        d.draw_text(&label, c.x as i32, mid_y, HUD_TEXT_SIZE, DIM);
        // Left-aligned a fixed step in from the `<` button rather than
        // centred between the two: a value that stays put as it changes
        // reads better than one that drifts.
        d.draw_text(&value, (prev.x + prev.width + 24.0) as i32, mid_y, HUD_TEXT_SIZE, TEXT);
    }
}

/// The code entry: five boxes and what has been typed into them.
fn draw_code<D: RaylibDraw>(d: &mut D, area: Rect, view: &LobbyView) {
    let typed: Vec<char> = view.entry.chars().collect();
    for i in 0..crate::net::rooms::CODE_LETTERS {
        let r = code_box_rect(area, i);
        let filled = i < typed.len();
        d.draw_rectangle_rounded(r, 0.15, 8, if filled { ROW_YOU } else { ROW_FILL });
        d.draw_rectangle_rounded_lines_ex(r, 0.15, 8, 2.0, if i == typed.len() { ONLINE_COLOR } else { DIM });
        if let Some(&c) = typed.get(i) {
            let size = 40;
            let w = width(&c.to_string(), size);
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
fn draw_room<D: RaylibDraw, S: Sheets>(d: &mut D, area: Rect, view: &LobbyView, sheets: &S) {
    let c = content_rect(area);
    let seats = seats_rect(area);
    for row in 0..LOBBY_SEAT_ROWS {
        draw_seat(d, seat_row_rect(area, row), view.seats.get(row));
    }
    if view.more > 0 {
        let y = (seats.y + seats.height + 2.0) as i32;
        let more = text().fmt(keys::LOBBY_MORE, &[("n", (view.more as i64).into())]);
        d.draw_text(&more, seats.x as i32, y, UI_SMALL_TEXT, DIM);
    }
    // The line under the title has nowhere to go in this face; it runs
    // along the bottom between the buttons instead.
    let bottom = button_rect(area, Button::Ready);
    let sub_y = (bottom.y - 18.0) as i32;
    d.draw_text(&view.sub, c.x as i32, sub_y, UI_SMALL_TEXT, DIM);

    let box_rect = qr_rect(area);
    if let Some(code) = &view.qr {
        let scale = code.scale_for(LOBBY_QR_BOX as i32);
        let side = code.padded_size() * scale;
        let x = (box_rect.x + (LOBBY_QR_BOX - side as f32) / 2.0) as i32;
        let y = (box_rect.y + (LOBBY_QR_BOX - side as f32) / 2.0) as i32;
        // One statement: the canvas borrows the draw handle for it.
        qr::draw(&mut GpuCanvas::new(d, sheets), code, x, y, scale, Color::BLACK, Color::WHITE);
    }
    if let Some(code) = &view.code {
        let w = width(code, CODE_TEXT_SIZE);
        d.draw_text(
            code,
            (box_rect.x + (LOBBY_QR_BOX - w as f32) / 2.0) as i32,
            (box_rect.y + box_rect.height + 6.0) as i32,
            CODE_TEXT_SIZE,
            TEXT,
        );
    }
    if let Some(url) = &view.join_url {
        // Under the code: centred under the QR where it fits, else up to
        // the content's right edge from just clear of the seat column, cut
        // from the head - the link is long with a local override, and the
        // tail is what differs.
        let right = box_rect.x + LOBBY_QR_BOX;
        let shown = tail_fit(url, (LOBBY_QR_BOX + LOBBY_MARGIN) as i32 - 4, UI_SMALL_TEXT);
        let w = width(&shown, UI_SMALL_TEXT) as f32;
        let x = if w <= LOBBY_QR_BOX { box_rect.x + (LOBBY_QR_BOX - w) / 2.0 } else { right - w };
        d.draw_text(&shown, x as i32, (box_rect.y + box_rect.height + 6.0 + CODE_TEXT_SIZE as f32 + 2.0) as i32, UI_SMALL_TEXT, DIM);
    }
}

/// `text` whole when it is no wider than `max_px` at `size`, else its tail
/// behind `...`, as much of it as fits.
fn tail_fit(text: &str, max_px: i32, size: i32) -> String {
    if width(text, size) <= max_px {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    (1..chars.len())
        .map(|skip| format!("...{}", chars[skip..].iter().collect::<String>()))
        .find(|cut| width(cut, size) <= max_px)
        .unwrap_or_else(|| "...".to_string())
}

/// One seat's row, or an empty slot.
fn draw_seat<D: RaylibDraw>(d: &mut D, r: Rectangle, seat: Option<&SeatRow>) {
    let inner = Rectangle::new(r.x, r.y + 3.0, r.width, r.height - 6.0);
    let Some(seat) = seat else {
        d.draw_rectangle_rounded_lines_ex(inner, 0.2, 8, 1.0, Color::new(70, 70, 78, 140));
        d.draw_text(&text().get(keys::SEAT_EMPTY), (r.x + 12.0) as i32, (r.y + (r.height - UI_SMALL_TEXT as f32) / 2.0) as i32, UI_SMALL_TEXT, DEAD);
        return;
    };
    d.draw_rectangle_rounded(inner, 0.2, 8, if seat.you { ROW_YOU } else { ROW_FILL });
    let text_y = (r.y + (r.height - HUD_TEXT_SIZE as f32) / 2.0) as i32;
    // The seat's number in its own ring colour (`TEAM_COLORS`), the one its
    // tank carries on the field.
    let slot_color = TEAM_COLORS[seat.seat as usize % TEAM_COLORS.len()];
    d.draw_text(&seat.slot, (r.x + 10.0) as i32, text_y, HUD_TEXT_SIZE, slot_color);
    d.draw_text(&seat.nick, (r.x + LOBBY_SEAT_NICK_X) as i32, text_y, HUD_TEXT_SIZE, if seat.you { TEXT } else { Color::new(210, 210, 216, 255) });
    // The small columns sit on the name's middle.
    let small_y = text_y + (HUD_TEXT_SIZE - UI_SMALL_TEXT) / 2;
    d.draw_text(&seat.chassis, (r.x + LOBBY_SEAT_CHASSIS_X) as i32, small_y, UI_SMALL_TEXT, DIM);
    let state_color = match (seat.host, seat.ready) {
        (true, _) => ONLINE_COLOR,
        (_, true) => BUILD_COLOR,
        _ => DIM,
    };
    d.draw_text(&seat.state, (r.x + LOBBY_SEAT_STATE_X) as i32, small_y, UI_SMALL_TEXT, state_color);
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
    let w = width(&view.label, size);
    d.draw_text(
        &view.label,
        (r.x + (r.width - w as f32) / 2.0) as i32,
        (r.y + (r.height - size as f32) / 2.0) as i32,
        size,
        color,
    );
}
