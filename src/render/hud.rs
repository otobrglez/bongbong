//! Drawing the HUD - the two corner clusters and their buttons, the
//! builder bar's mode button, the banners, the dialogs and the end screen,
//! all in UI points - and the slot tables the clusters' rows are laid out
//! from (`hud.rs` owns the model, the shared colours and sizes, and every
//! rect the hit tests read).

use sola_raylib::prelude::*;

use crate::hud::{
    banner_size, clock_text, leave_dialog_rects, players_dialog_rects, result_layout, weapon_color, Corners, Fade, Hints,
    HudModel, NextLevel, PlayChrome, PlayerHud, ResultButtons, ResultView, SeatHud, BUILD_COLOR, DIALOG_W, DIM,
    HUD_TEXT_SIZE, LEVEL_BUTTON_W, LEVEL_BUTTON_WORD_GAP, LINE_H, ONLINE_COLOR, RESULT_LINE_SIZE,
    RESULT_SEATS_SIZE, RESULT_STATS_GAP, ROW_H, TEXT, UI_SMALL_TEXT, WEAPON_SLOTS,
};
use crate::math::{Color, Rectangle};
use crate::text::{keys, text, width, Key};
use crate::render::game::Textures;
use crate::simulation::PlayerCount;
use crate::tank::{team_color, ActiveWeapon, HealthRamp, TEAM_COLORS};
use crate::{Rect, MAX_DAMAGE, MAX_SEATS, PICKUP_TEXTURE_SIZE, SHELL_TEXTURE_SIZE};

const HEART: Color = Color::new(230, 60, 70, 255);
const SPEED_COLOR: Color = Color::new(255, 210, 60, 255);
const SHIELD_COLOR: Color = Color::new(170, 120, 255, 255);
const FROG_COLOR: Color = Color::new(120, 220, 90, 255);
/// What a slot draws in when there is nothing in it: the FROG gauge of a
/// round without a frog, and a wrecked seat's chip.
const SPENT: Color = Color::new(60, 60, 66, 255);
/// A cluster's plate: the builder bar's dark, mostly opaque, so the
/// readouts read over any ground under any sky. The minimap's plate too.
const PLATE_FILL: Color = Color::new(21, 21, 24, 208);
pub(crate) const PLATE_EDGE: Color = Color::new(0, 0, 0, 150);
/// The dark plate behind each line of text under the left cluster.
const LINE_FILL: Color = Color::new(0, 0, 0, 150);

// The vitals block's slots, from its left edge (`hud::VITALS_W` wide, two
// `hud::ROW_H` rows). Fixed, so a number changing width never nudges what
// sits after it; `corner_tests` pins that nothing overlaps.
const V_HEART: i32 = 0;
const V_HP: i32 = 18;
#[cfg_attr(not(test), allow(dead_code))]
const V_HP_W: i32 = 30;
const V_HEALTH: i32 = 52;
const V_HEALTH_W: i32 = 48;
const V_SHELL: i32 = 106;
const V_SHELLS: i32 = 140;
/// A count: three digits at `HUD_TEXT_SIZE`.
const V_COUNT_W: i32 = 30;
const V_SPEED: i32 = 182;
const V_SHIELD: i32 = 250;
/// A gauge's slot: its label over its bar.
#[cfg_attr(not(test), allow(dead_code))]
const GAUGE_SLOT_W: i32 = 60;
const GAUGE_W: i32 = 56;
const GAUGE_H: i32 = 8;
/// The weapon queue's slots on the second row: the pickup icon and the
/// count beside it.
const V_WEAPON_W: i32 = 62;
const V_WEAPON_ICON: i32 = 28;

// The right cluster's first row (`hud::INFO_W` wide), from its left edge.
/// The mission word and its wave count, or the level button and the count
/// beside it.
#[cfg_attr(not(test), allow(dead_code))]
const I_TITLE_W: i32 = 160;
const LEVEL_WAVE_GAP: i32 = 8;
const I_ENEMIES: i32 = 166;
const I_ENEMY_COUNT: i32 = 184;
/// Two digits of enemies, then the dim `+N` still to come.
const I_PENDING: i32 = 208;
const I_FROG: i32 = 244;

/// The tank glyph's footprint: 7 x 7 blocks of 2 px (`draw_tank_glyph`).
#[cfg_attr(not(test), allow(dead_code))]
const TANK_GLYPH_W: i32 = 14;
const TANK_GLYPH_H: i32 = 14;

/// `c` at a cluster's fade.
fn faded(c: Color, a: f32) -> Color {
    Color::new(c.r, c.g, c.b, (c.a as f32 * a.clamp(0.0, 1.0)).round() as u8)
}

/// One line of text under the left cluster.
pub struct Line {
    pub text: String,
    pub size: i32,
    pub color: Color,
}

/// Draw both corner clusters (`hud::corners`) in UI points - inside the
/// camera that puts a UI point where `UiFrame` puts it - each at its fade:
/// the vitals blocks top-left with `lines` under them, the round's numbers,
/// the buttons and a room's chips top-right, and the minimap under them
/// from `minimap` where the corners hold one.
#[allow(clippy::too_many_arguments)]
pub fn draw_corners(
    d: &mut impl RaylibDraw,
    corners: &Corners,
    model: &HudModel,
    chrome: &PlayChrome,
    players: PlayerCount,
    textures: &Textures,
    fade: Fade,
    lines: &[Line],
    minimap: Option<&crate::render::minimap::MinimapLayer>,
) {
    // The left cluster: the local seat's block, and a couch's player 2's.
    let a = fade.left;
    let first_seat = chrome.seat.unwrap_or(0);
    let seats: [(Option<&PlayerHud>, u8); 2] = [(Some(&model.local), first_seat), (model.second.as_ref(), 1)];
    for (block, (hud, seat)) in corners.blocks.iter().zip(seats) {
        let Some(hud) = hud else { continue };
        draw_plate(d, Corners::plate(*block), Color::new(team_color(seat).r, team_color(seat).g, team_color(seat).b, 150), a);
        draw_vitals(d, *block, hud, seat, textures, a);
        if block.height > crate::hud::VITALS_H {
            draw_lamp_row(d, *block, hud, a);
        }
    }
    let mut y = corners.lines.y;
    for line in lines {
        let w = width(&line.text, line.size);
        let x = corners.lines.x;
        d.draw_rectangle_rounded(Rectangle::new(x, y, (w + 8) as f32, (line.size + 6) as f32), 0.3, 4, faded(LINE_FILL, a));
        d.draw_text(&line.text, x as i32 + 4, y as i32 + 3, line.size, faded(line.color, a));
        y += LINE_H;
    }

    // The right cluster: the round's numbers, the buttons, the chips.
    let a = fade.right;
    draw_plate(d, corners.right, PLATE_EDGE, a);
    let level = chrome.level_button.map(|n| (n, chrome.levels.is_some()));
    draw_info(d, corners.info, model, level, a);
    if let Some(r) = corners.online {
        draw_label_button(d, r, &text().get(keys::BUTTON_ONLINE), ONLINE_COLOR, a);
    }
    if let Some(r) = corners.players {
        draw_players_button(d, r, players, chrome.players_dialog, a);
    }
    if let Some(r) = corners.restart {
        draw_restart_button(d, r, a);
    }
    if let Some(r) = corners.build {
        draw_label_button(d, r, &text().get(keys::BUTTON_BUILD), BUILD_COLOR, a);
    }
    if let Some(r) = corners.leave {
        draw_label_button(d, r, &text().get(keys::BUTTON_LEAVE), ONLINE_COLOR, a);
    }
    for (i, seat) in model.others.iter().take(MAX_SEATS - 1).enumerate() {
        if let Some(r) = corners.chip(i) {
            draw_seat_chip(d, r, seat, a);
        }
    }
    // The minimap, part of the right cluster: it fades with it.
    if let (Some(rect), Some(layer)) = (corners.minimap, minimap) {
        let picture = crate::minimap::picture(layer.marks, rect, layer.field, crate::indicators::label_font(1.0), &crate::tuning::tuning());
        crate::render::minimap::draw_minimap(d, rect, layer.texture, layer.field, &picture, a);
    }
}

/// A cluster's plate: the dark fill and an edge in `edge`.
pub(crate) fn draw_plate(d: &mut impl RaylibDraw, r: Rectangle, edge: Color, a: f32) {
    d.draw_rectangle_rounded(r, 0.12, 6, faded(PLATE_FILL, a));
    d.draw_rectangle_rounded_lines_ex(r, 0.12, 6, 1.5, faded(edge, a));
}

/// One seat's vitals in `block`: the heart, the health number and its
/// gauge in the seat's own ring colours, the shell sprite and its count,
/// the speed and shield gauges; under them the weapon queue, the live
/// weapon outlined in its accent.
fn draw_vitals(d: &mut impl RaylibDraw, block: Rectangle, hud: &PlayerHud, seat: u8, textures: &Textures, a: f32) {
    let (x, y) = (block.x.round() as i32, block.y.round() as i32);
    let row = ROW_H as i32;
    let text_y = y + (row - HUD_TEXT_SIZE) / 2;
    let white = faded(Color::WHITE, a);

    draw_heart(d, x + V_HEART, y + (row - 12) / 2, a);
    d.draw_text(&hud.hp.to_string(), x + V_HP, text_y, HUD_TEXT_SIZE, faded(hud.hp_color, a));
    let health = (hud.hp as f32 / MAX_DAMAGE).clamp(0.0, 1.0);
    let ramp = HealthRamp::player(seat);
    draw_gauge(d, x + V_HEALTH, y + (row - GAUGE_H) / 2, V_HEALTH_W, GAUGE_H, health, faded(ramp.color(health), a), faded(DIM, a));

    // The shell sprite's in-flight frame, identical on every row of the
    // sheet, full-bleed at its own 32 px.
    let shell_src = Rectangle::new(3.0 * SHELL_TEXTURE_SIZE, 0.0, SHELL_TEXTURE_SIZE, SHELL_TEXTURE_SIZE);
    let shell_dest = Rectangle::new((x + V_SHELL) as f32, y as f32, SHELL_TEXTURE_SIZE, SHELL_TEXTURE_SIZE);
    d.draw_texture_pro(textures.shells, shell_src, shell_dest, Vector2::new(0.0, 0.0), 0.0, white);
    d.draw_text(&hud.shells.to_string(), x + V_SHELLS, text_y, HUD_TEXT_SIZE, faded(hud.shells_color, a));
    if hud.shells_active {
        active_outline(d, x + V_SHELL, y, V_SHELLS + V_COUNT_W - V_SHELL, row, faded(TEXT, a));
    }

    let t = text();
    draw_gauge_slot(d, x + V_SPEED, y, row, &t.get(keys::HUD_SPEED), hud.speed, SPEED_COLOR, true, a);
    draw_gauge_slot(d, x + V_SHIELD, y, row, &t.get(keys::HUD_SHIELD), hud.shield, SHIELD_COLOR, true, a);

    let y = y + row;
    let text_y = y + (row - HUD_TEXT_SIZE) / 2;
    for (i, slot) in hud.weapons.iter().enumerate().take(WEAPON_SLOTS) {
        let sx = x + i as i32 * V_WEAPON_W;
        let texture = match slot.weapon {
            ActiveWeapon::Laser => textures.pickup_laser,
            ActiveWeapon::Plasma => textures.pickup_plasma,
            ActiveWeapon::Minigun => textures.pickup_minigun,
            ActiveWeapon::Missiles => textures.pickup_missiles,
            ActiveWeapon::Flamethrower => textures.pickup_flamethrower,
            ActiveWeapon::Shell => textures.shells,
        };
        let src = Rectangle::new(0.0, 0.0, PICKUP_TEXTURE_SIZE, PICKUP_TEXTURE_SIZE);
        let icon = V_WEAPON_ICON as f32;
        let dest = Rectangle::new(sx as f32, (y + (row - V_WEAPON_ICON) / 2) as f32, icon, icon);
        let tint = if slot.count > 0 { white } else { faded(Color::new(255, 255, 255, 70), a) };
        d.draw_texture_pro(texture, src, dest, Vector2::new(0.0, 0.0), 0.0, tint);
        let (count, color) = if slot.count > 0 { (slot.count.to_string(), weapon_color(slot.weapon)) } else { ("--".to_string(), DIM) };
        d.draw_text(&count, sx + V_WEAPON_ICON + 3, text_y, HUD_TEXT_SIZE, faded(color, a));
        if slot.active {
            active_outline(d, sx, y, V_WEAPON_W - 2, row, faded(weapon_color(slot.weapon), a));
        }
    }
}

/// The lamp row under a block's two (`hud::CornerShape::lamp_row`): the
/// lantern with how many are left to set down - the lamp key's button on
/// a touch screen - and, while one is on, the heat shield's gauge in the
/// shield's slot (docs/volcano.md).
fn draw_lamp_row(d: &mut impl RaylibDraw, block: Rectangle, hud: &PlayerHud, a: f32) {
    use crate::pyro::{FIRE, SMOKE};
    let row_h = (block.height - crate::hud::VITALS_H) as i32;
    let (x, y) = (block.x.round() as i32, (block.y + crate::hud::VITALS_H).round() as i32);
    let t = text();
    if let Some(left) = hud.lamps {
        // The lantern, drawn in the effects language's blocks: cap, glass
        // round its flame, base; dark once none is left.
        let (lx, ly) = (x + 6, y + (row_h - 20) / 2);
        let lit = left > 0;
        let flame = if lit { FIRE[5] } else { SMOKE[2] };
        d.draw_rectangle(lx + 4, ly, 8, 2, faded(SMOKE[1], a));
        d.draw_rectangle(lx + 2, ly + 2, 12, 2, faded(SMOKE[0], a));
        d.draw_rectangle(lx + 2, ly + 4, 2, 12, faded(SMOKE[0], a));
        d.draw_rectangle(lx + 12, ly + 4, 2, 12, faded(SMOKE[0], a));
        d.draw_rectangle(lx + 4, ly + 4, 8, 12, faded(if lit { FIRE[4] } else { SMOKE[1] }, a));
        d.draw_rectangle(lx + 6, ly + 6, 4, 6, faded(flame, a));
        if lit {
            d.draw_rectangle(lx + 6, ly + 6, 2, 2, faded(FIRE[6], a));
        }
        d.draw_rectangle(lx + 2, ly + 16, 12, 4, faded(SMOKE[0], a));
        let count_y = y + (row_h - HUD_TEXT_SIZE) / 2;
        let color = if lit { TEXT } else { SPENT };
        d.draw_text(&left.to_string(), x + 26, count_y, HUD_TEXT_SIZE, faded(color, a));
        d.draw_text(&t.get(keys::HUD_LAMPS), x + 48, y + (row_h - UI_SMALL_TEXT) / 2, UI_SMALL_TEXT, faded(DIM, a));
    }
    if hud.heat_shield > 0.0 {
        draw_gauge_slot(d, x + V_SHIELD, y, row_h, &t.get(keys::HUD_HEAT), hud.heat_shield, HEAT_SHIELD_COLOR, true, a);
    }
}

/// The heat shield gauge's colour: the pickup's molten red.
const HEAT_SHIELD_COLOR: Color = crate::pyro::FIRE[3];

/// The right cluster's first row in `info`: the level button and the wave
/// count beside it on a level (`level` is its number and whether the level
/// select it opens is up), else the mission word with its count; the enemy
/// count with the ones still to come; the frog's gauge.
fn draw_info(d: &mut impl RaylibDraw, info: Rectangle, model: &HudModel, level: Option<(usize, bool)>, a: f32) {
    let (x, y, h) = (info.x.round() as i32, info.y.round() as i32, info.height.round() as i32);
    let text_y = y + (h - HUD_TEXT_SIZE) / 2;
    match level {
        Some((number, open)) => {
            draw_level_button(d, Rectangle::new(info.x, info.y, LEVEL_BUTTON_W, info.height), number, open, a);
            if let Some((index, total)) = model.wave {
                let wx = x + LEVEL_BUTTON_W as i32 + LEVEL_WAVE_GAP;
                d.draw_text(&format!("{index}/{total}"), wx, text_y, HUD_TEXT_SIZE, faded(TEXT, a));
            }
        }
        None => d.draw_text(&model.title, x, text_y, HUD_TEXT_SIZE, faded(TEXT, a)),
    }
    // A tank glyph stands for "enemies": the count next to a tank reads.
    draw_tank_glyph(d, x + I_ENEMIES, y + (h - TANK_GLYPH_H) / 2, faded(DIM, a));
    d.draw_text(&model.enemies_alive.to_string(), x + I_ENEMY_COUNT, text_y, HUD_TEXT_SIZE, faded(TEXT, a));
    if model.enemies_pending > 0 {
        d.draw_text(&format!("+{}", model.enemies_pending), x + I_PENDING, text_y, HUD_TEXT_SIZE, faded(DIM, a));
    }
    draw_gauge_slot(d, x + I_FROG, y, h, &text().get(keys::HUD_FROG), model.frog.unwrap_or(0.0), FROG_COLOR, model.frog.is_some(), a);
}

/// A gauge in its slot at `x` of a row from `y`, `h` tall: its label over
/// its bar, both dim when `present` is false (the FROG gauge of a round
/// without one).
#[allow(clippy::too_many_arguments)]
fn draw_gauge_slot(d: &mut impl RaylibDraw, x: i32, y: i32, h: i32, label: &str, frac: f32, color: Color, present: bool, a: f32) {
    let top = y + (h - ROW_H as i32) / 2;
    let label_color = faded(if present { DIM } else { SPENT }, a);
    d.draw_text(label, x, top + 3, UI_SMALL_TEXT, label_color);
    draw_gauge(d, x, top + 19, GAUGE_W, GAUGE_H, if present { frac } else { 0.0 }, faded(color, a), label_color);
}

/// One outlined bar `w` x `h` at (`x`, `y`), filled to `frac` in whole
/// 2 px blocks so it drains in steps like every other gauge in the game.
fn draw_gauge(d: &mut impl RaylibDraw, x: i32, y: i32, w: i32, h: i32, frac: f32, color: Color, outline: Color) {
    d.draw_rectangle_lines_ex(Rectangle::new(x as f32, y as f32, w as f32, h as f32), 1.0, outline);
    if frac > 0.0 {
        let fill = ((w - 4) as f32 * frac.clamp(0.0, 1.0) / 2.0).round() as i32 * 2;
        if fill > 0 {
            d.draw_rectangle(x + 2, y + 2, fill, h - 4, color);
        }
    }
}

/// One seat of the other seats' strip: its number over its health gauge,
/// both in the ring colour that seat wears on the field, so the chip and
/// the tank are read as one. A wreck keeps its place with an empty gauge
/// and both halves in the spent grey - the strip is as long as the round
/// has seats, whoever is still standing.
fn draw_seat_chip(d: &mut impl RaylibDraw, r: Rectangle, seat: &SeatHud, a: f32) {
    let color = faded(if seat.alive { team_color(seat.seat) } else { SPENT }, a);
    let outline = faded(if seat.alive { DIM } else { SPENT }, a);
    let label = format!("{}", seat.seat + 1);
    let (x, y, w, h) = (r.x as i32, r.y as i32, r.width as i32, r.height as i32);
    d.draw_text(&label, x + (w - width(&label, UI_SMALL_TEXT)) / 2, y + 1, UI_SMALL_TEXT, color);
    draw_gauge(d, x, y + h - GAUGE_H - 1, w, GAUGE_H, seat.health, color, outline);
}

/// The outline marking which slot the trigger fires: 2 pt, inset one
/// block from the row's top and bottom edges.
fn active_outline(d: &mut impl RaylibDraw, x: i32, y: i32, w: i32, h: i32, color: Color) {
    d.draw_rectangle_lines_ex(Rectangle::new((x - 2) as f32, (y + 2) as f32, (w + 4) as f32, (h - 4) as f32), 2.0, color);
}

/// A pixel heart of 2 px blocks, 14x12, for the health slot. Drawn rather
/// than loaded: it is the one glyph in the game with no sheet of its own.
fn draw_heart(d: &mut impl RaylibDraw, x: i32, y: i32, a: f32) {
    const ROWS: [&str; 6] = [".##.##.", "#######", "#######", ".#####.", "..###..", "...#..."];
    for (row, line) in ROWS.iter().enumerate() {
        for (col, c) in line.chars().enumerate() {
            if c == '#' {
                d.draw_rectangle(x + col as i32 * 2, y + row as i32 * 2, 2, 2, faded(HEART, a));
            }
        }
    }
}

/// A pixel tank seen from above, 7x7 blocks of 2 px (14x14): tracks down
/// both sides, the hull between, the barrel up. The players button shows
/// one or two of these, the right cluster one for the enemy count.
fn draw_tank_glyph(d: &mut impl RaylibDraw, x: i32, y: i32, color: Color) {
    const ROWS: [&str; 7] = ["...#...", "...#...", "#.###.#", "#.###.#", "#.###.#", "#.###.#", "#.....#"];
    for (row, line) in ROWS.iter().enumerate() {
        for (col, c) in line.chars().enumerate() {
            if c == '#' {
                d.draw_rectangle(x + col as i32 * 2, y + row as i32 * 2, 2, 2, color);
            }
        }
    }
}

/// A button's frame: 2 pt, one block in from the top and bottom of its
/// rect, the shape every button of the corners and the builder's bar share.
fn button_frame(r: Rectangle) -> Rectangle {
    Rectangle::new(r.x, r.y + 2.0, r.width, r.height - 4.0)
}

/// A framed button with its label centred, in `color`.
fn draw_label_button(d: &mut impl RaylibDraw, r: Rectangle, label: &str, color: Color, a: f32) {
    d.draw_rectangle_lines_ex(button_frame(r), 2.0, faded(color, a));
    let text_w = width(label, HUD_TEXT_SIZE);
    let (x, y) = ((r.x + (r.width - text_w as f32) / 2.0) as i32, (r.y + (r.height - HUD_TEXT_SIZE as f32) / 2.0) as i32);
    d.draw_text(label, x, y, HUD_TEXT_SIZE, faded(color, a));
}

/// The players button: dim until the dialog it opens is up, its glyphs
/// the tank colours themselves.
fn draw_players_button(d: &mut impl RaylibDraw, r: Rectangle, players: PlayerCount, open: bool, a: f32) {
    d.draw_rectangle_lines_ex(button_frame(r), 2.0, faded(if open { TEXT } else { DIM }, a));
    let glyph = TANK_GLYPH_H;
    let gy = (r.y + (r.height - glyph as f32) / 2.0) as i32;
    if players.count() < 2 {
        draw_tank_glyph(d, (r.x + (r.width - glyph as f32) / 2.0) as i32, gy, faded(TEAM_COLORS[0], a));
    } else {
        let gap = 6;
        let x = (r.x + (r.width - (2 * glyph + gap) as f32) / 2.0) as i32;
        draw_tank_glyph(d, x, gy, faded(TEAM_COLORS[0], a));
        draw_tank_glyph(d, x + glyph + gap, gy, faded(TEAM_COLORS[1], a));
    }
}

/// The RESTART button: the frame around a circular arrow, the one restart
/// glyph a phone player reads without a label, in the text colour so it
/// is neither the builder's amber nor a weapon accent. The press is
/// `tuning::request_restart`, the same path as the dev panel's button, and
/// lands as `Input::restart_pressed` like the R key.
fn draw_restart_button(d: &mut impl RaylibDraw, r: Rectangle, a: f32) {
    let color = faded(TEXT, a);
    d.draw_rectangle_lines_ex(button_frame(r), 2.0, color);
    let center = Vector2::new(r.x + r.width / 2.0, r.y + r.height / 2.0);
    // Three quarters of a ring, the gap at the right, an arrowhead on the
    // end that points on around the circle.
    let (inner, outer) = (6.0, 10.0);
    let (start, end) = (45.0, 315.0);
    d.draw_ring(center, inner, outer, start, end, 24, color);
    let rad = (end as f32).to_radians();
    let mid = (inner + outer) / 2.0;
    let tip_at = Vector2::new(center.x + mid * rad.cos(), center.y + mid * rad.sin());
    let tangent = Vector2::new(-rad.sin(), rad.cos());
    let radial = Vector2::new(rad.cos(), rad.sin());
    let tip = Vector2::new(tip_at.x + tangent.x * 6.0, tip_at.y + tangent.y * 6.0);
    let p = Vector2::new(tip_at.x + radial.x * 5.0, tip_at.y + radial.y * 5.0);
    let q = Vector2::new(tip_at.x - radial.x * 5.0, tip_at.y - radial.y * 5.0);
    d.draw_triangle(p, q, tip, color);
    d.draw_triangle(tip, q, p, color);
}

/// The level button: its frame in the levels' amber around `LEVEL 3` -
/// the word small, the number in the readouts' size - washed amber while
/// the level select it opens is up.
fn draw_level_button(d: &mut impl RaylibDraw, r: Rectangle, number: usize, open: bool, a: f32) {
    let frame = button_frame(r);
    if open {
        d.draw_rectangle_rec(frame, faded(Color::new(BUILD_COLOR.r, BUILD_COLOR.g, BUILD_COLOR.b, 50), a));
    }
    let amber = faded(BUILD_COLOR, a);
    d.draw_rectangle_lines_ex(frame, 2.0, amber);
    let word = text().get(keys::BAR_LEVEL);
    let number = number.to_string();
    let (word_w, number_w) = (width(&word, UI_SMALL_TEXT), width(&number, HUD_TEXT_SIZE));
    let x = (r.x + (r.width - (word_w + LEVEL_BUTTON_WORD_GAP + number_w) as f32) / 2.0) as i32;
    let number_y = (r.y + (r.height - HUD_TEXT_SIZE as f32) / 2.0) as i32;
    // The word sits on the number's baseline rather than its middle.
    let word_y = number_y + HUD_TEXT_SIZE - UI_SMALL_TEXT - 2;
    d.draw_text(&word, x, word_y, UI_SMALL_TEXT, amber);
    d.draw_text(&number, x + word_w + LEVEL_BUTTON_WORD_GAP, number_y, HUD_TEXT_SIZE, amber);
}

/// An outlined bar slot with its label centred, in `color`: the builder
/// bar's mode button, `PLAY` (`editor::Bar::play`).
pub fn draw_slot_button(d: &mut impl RaylibDraw, r: Rectangle, label: &str, color: Color) {
    d.draw_rectangle_lines_ex(Rectangle::new(r.x, r.y + 2.0, r.width, r.height - 4.0), 2.0, color);
    let text_w = width(label, HUD_TEXT_SIZE);
    d.draw_text(label, (r.x + (r.width - text_w as f32) / 2.0) as i32, (r.y + (r.height - HUD_TEXT_SIZE as f32) / 2.0) as i32, HUD_TEXT_SIZE, color);
}

/// The dialog panel both questions share: shadow, rounded fill, outline,
/// a 28 px title and a 16 px line under it, both centred by `text::width`.
fn draw_dialog_panel(d: &mut impl RaylibDraw, panel: Rectangle, title: &str, sub: &str) {
    let shadow = Rectangle::new(panel.x + 4.0, panel.y + 4.0, panel.width, panel.height);
    d.draw_rectangle_rounded(shadow, 0.08, 8, Color::new(0, 0, 0, 90));
    d.draw_rectangle_rounded(panel, 0.08, 8, Color::new(20, 20, 24, 240));
    d.draw_rectangle_rounded_lines_ex(panel, 0.08, 8, 1.5, Color::new(0, 0, 0, 150));
    let title_w = width(title, 28);
    d.draw_text(title, (panel.x + (DIALOG_W - title_w as f32) / 2.0) as i32, (panel.y + 22.0) as i32, 28, TEXT);
    let sub_w = width(sub, 16);
    d.draw_text(sub, (panel.x + (DIALOG_W - sub_w as f32) / 2.0) as i32, (panel.y + 62.0) as i32, 16, DIM);
}

/// One dialog button: an outlined rounded box with its label centred.
fn draw_dialog_button(d: &mut impl RaylibDraw, rect: Rectangle, label: &str, color: Color, fill: Option<Color>) {
    if let Some(fill) = fill {
        d.draw_rectangle_rounded(rect, 0.2, 8, fill);
    }
    d.draw_rectangle_rounded_lines_ex(rect, 0.2, 8, 2.0, color);
    let w = width(label, HUD_TEXT_SIZE);
    d.draw_text(label, (rect.x + (rect.width - w as f32) / 2.0) as i32, (rect.y + (rect.height - HUD_TEXT_SIZE as f32) / 2.0) as i32, HUD_TEXT_SIZE, color);
}

/// Draw the players dialog over the (already dimmed) window: the live
/// count's button highlighted, the other in the action colour, and its
/// line naming the controls of the input `hints` says is in use. In UI
/// points, centred in the chrome's `area`, like `draw_leave_dialog`.
pub fn draw_players_dialog(d: &mut impl RaylibDraw, area: Rect, players: PlayerCount, hints: Hints) {
    let t = text();
    let r = players_dialog_rects(area);
    draw_dialog_panel(d, r.panel, &t.get(keys::PLAYERS_TITLE), &t.get(hints.pick(keys::PLAYERS_KEYS, keys::PLAYERS_TOUCH)));
    let live_fill = Some(Color::new(255, 255, 255, 40));
    let one_live = players == PlayerCount::ONE;
    draw_dialog_button(d, r.one, &t.get(keys::PLAYERS_ONE), if one_live { TEXT } else { BUILD_COLOR }, one_live.then_some(live_fill).flatten());
    draw_dialog_button(d, r.two, &t.get(keys::PLAYERS_TWO), if one_live { BUILD_COLOR } else { TEXT }, (!one_live).then_some(live_fill).flatten());
}

/// Draw the leave-round dialog over the (already dimmed) window. In UI
/// points: call inside the UI camera, centred in the chrome's `area`.
pub fn draw_leave_dialog(d: &mut impl RaylibDraw, area: Rect) {
    let t = text();
    let r = leave_dialog_rects(area);
    draw_dialog_panel(d, r.panel, &t.get(keys::LEAVE_TITLE), &t.get(keys::LEAVE_SUB));
    draw_dialog_button(d, r.leave, &t.get(keys::LEAVE_CONFIRM), BUILD_COLOR, None);
    draw_dialog_button(d, r.stay, &t.get(keys::LEAVE_STAY), TEXT, None);
}

/// One line of a banner in UI points, centred on the chrome's `area` at
/// `y`: set in `size` where the area has the room and smaller where it has
/// not (`hud::banner_size`). Answers the size it was set in.
pub fn draw_banner(d: &mut impl RaylibDraw, area: Rect, text: &str, size: i32, y: i32, color: Color) -> i32 {
    let size = banner_size(text, size, area);
    let w = width(text, size);
    d.draw_text(text, (area.x + area.w / 2.0).round() as i32 - w / 2, y, size, color);
    size
}

/// Draw the end screen under its outcome (docs/levels.md): every level
/// complete after the last one's win, the round's time and wrecks, the
/// wrecks by seat from two seats, then a level's buttons - the way it is
/// counting down to carrying the count - or free play's `countdown` in
/// their place. In UI points over the dim, centred in the chrome's
/// `area`, at the rows `hud::result_layout` gives, which is what the hit
/// tests read too.
pub fn draw_result(d: &mut impl RaylibDraw, area: Rect, view: &ResultView, countdown: &str) {
    fn centred(d: &mut impl RaylibDraw, middle: i32, line: &str, y: f32, size: i32, color: Color) {
        let w = width(line, size);
        d.draw_text(line, middle - w / 2, y as i32, size, color);
    }
    // Every line's centre: the area's, on a whole point.
    let middle = (area.x + area.w / 2.0).round() as i32;
    let t = text();
    let rows = result_layout(area, view);
    if let (Some(y), Some(ResultButtons { next: Some(NextLevel::FirstAgain { levels }), .. })) = (rows.all_clear_y, view.buttons) {
        centred(d, middle, &t.fmt(keys::RESULT_ALL_CLEAR, &[("count", levels.into())]), y, RESULT_LINE_SIZE, BUILD_COLOR);
    }
    let time = t.fmt(keys::RESULT_TIME, &[("time", clock_text(view.stats.seconds).into())]);
    let wrecks = t.fmt(keys::RESULT_WRECKS, &[("n", view.stats.destroyed.into()), ("total", view.stats.enemies.into())]);
    let (time_w, wrecks_w) = (width(&time, RESULT_LINE_SIZE), width(&wrecks, RESULT_LINE_SIZE));
    let x = middle - (time_w + RESULT_STATS_GAP + wrecks_w) / 2;
    d.draw_text(&time, x, rows.stats_y as i32, RESULT_LINE_SIZE, Color::RAYWHITE);
    d.draw_text(&wrecks, x + time_w + RESULT_STATS_GAP, rows.stats_y as i32, RESULT_LINE_SIZE, Color::RAYWHITE);
    if let Some(y) = rows.seats_y {
        // Each seat's share in the colour its tank wears, `P1 7  P2 5`.
        let seats = view.seats.min(MAX_SEATS);
        let gap = if seats > 4 { RESULT_STATS_GAP / 2 } else { RESULT_STATS_GAP };
        let parts: Vec<String> = (0..seats)
            .map(|seat| format!("{} {}", t.fmt(keys::SEAT_LABEL, &[("n", (seat + 1).into())]), view.stats.by_seat[seat]))
            .collect();
        let total: i32 = parts.iter().map(|p| width(p, RESULT_SEATS_SIZE)).sum::<i32>() + gap * (seats as i32 - 1);
        let mut x = middle - total / 2;
        for (seat, part) in parts.iter().enumerate() {
            d.draw_text(part, x, y as i32, RESULT_SEATS_SIZE, team_color(seat as u8));
            x += width(part, RESULT_SEATS_SIZE) + gap;
        }
    }
    let lit = Some(Color::new(BUILD_COLOR.r, BUILD_COLOR.g, BUILD_COLOR.b, 40));
    if let Some(rects) = rows.buttons {
        draw_dialog_button(d, rects.levels, &t.get(keys::RESULT_LEVELS), TEXT, None);
    }
    // The button the screen is counting down to says so and counts: the
    // press that skips the wait is the one the eye is already on.
    let count = view.buttons.and_then(|b| b.countdown);
    let counting = |counted: Key, plain: Key| match count {
        Some(seconds) => t.fmt(counted, &[("seconds", seconds.into())]),
        None => t.get(plain),
    };
    match (rows.buttons, view.buttons.and_then(|b| b.next)) {
        // The way on is the one to press; PLAY AGAIN stands beside it.
        (Some(rects), Some(next)) => {
            draw_dialog_button(d, rects.again, &t.get(keys::RESULT_AGAIN), TEXT, None);
            let label = match next {
                NextLevel::Next => counting(keys::RESULT_NEXT_IN, keys::RESULT_NEXT),
                NextLevel::FirstAgain { .. } => t.get(keys::RESULT_FIRST),
            };
            if let Some(rect) = rects.next {
                draw_dialog_button(d, rect, &label, BUILD_COLOR, lit);
            }
        }
        (Some(rects), None) => {
            draw_dialog_button(d, rects.again, &counting(keys::RESULT_AGAIN_IN, keys::RESULT_AGAIN), BUILD_COLOR, lit)
        }
        (None, _) => {
            if let Some(y) = rows.countdown_y {
                centred(d, middle, countdown, y, RESULT_LINE_SIZE, Color::RAYWHITE);
            }
        }
    }
}


#[cfg(test)]
mod corner_tests {
    use super::*;
    use crate::hud::{CHIP_H, CHIP_W, HUD_GAUGE_LABEL_MAX_PX, INFO_TITLE_W, INFO_W, VITALS_W};

    /// The vitals block's slots stay inside its width and its two rows and
    /// never overlap: the widest thing each holds is written down here, so
    /// growing a slot fails loudly rather than drawing over its neighbour.
    #[test]
    fn the_vitals_slots_fit_the_block_without_overlapping() {
        let three_digits = width("100", HUD_TEXT_SIZE).max(width("888", HUD_TEXT_SIZE));
        assert!(V_HEART + 14 <= V_HP, "the heart runs into the health number");
        assert!(three_digits <= V_HP_W && V_HP + V_HP_W <= V_HEALTH, "the health number runs into its gauge");
        assert!(V_HEALTH + V_HEALTH_W <= V_SHELL, "the health gauge runs into the shell sprite");
        assert!(V_SHELL + SHELL_TEXTURE_SIZE as i32 <= V_SHELLS);
        assert!(three_digits <= V_COUNT_W && V_SHELLS + V_COUNT_W <= V_SPEED, "the shells run into the speed gauge");
        assert!(V_SPEED + GAUGE_SLOT_W <= V_SHIELD);
        assert_eq!(V_SHIELD + GAUGE_SLOT_W, VITALS_W as i32, "the first row is the block's width");
        assert!(GAUGE_W <= GAUGE_SLOT_W);
        // The gauge labels' budget, which `text_tests` measures every
        // language against, is the slot less a gap.
        assert_eq!(HUD_GAUGE_LABEL_MAX_PX, GAUGE_SLOT_W - 4);
        // A label over its bar, both inside a row.
        assert!(3 + UI_SMALL_TEXT <= 19 && 19 + GAUGE_H <= ROW_H as i32);
        // The weapon queue: five slots across the second row, each an icon
        // no larger than its sheet and three digits beside it.
        assert_eq!(WEAPON_SLOTS as i32 * V_WEAPON_W, VITALS_W as i32);
        assert!(V_WEAPON_ICON <= PICKUP_TEXTURE_SIZE as i32 && V_WEAPON_ICON <= ROW_H as i32);
        assert!(V_WEAPON_ICON + 3 + three_digits <= V_WEAPON_W - 2, "a weapon count overflows its slot");
    }

    /// The right cluster's first row: the widest mission word with its
    /// wave count, or the level button with the widest count beside it,
    /// ends before the enemy count, which ends before the frog's gauge.
    #[test]
    fn the_info_slots_fit_the_row_without_overlapping() {
        assert_eq!(I_TITLE_W, INFO_TITLE_W as i32, "the title's budget is the slot `text_tests` measures");
        assert!(width("DESTROY 12/12", HUD_TEXT_SIZE) <= I_TITLE_W && I_TITLE_W <= I_ENEMIES);
        assert!(LEVEL_BUTTON_W as i32 + LEVEL_WAVE_GAP + width("12/12", HUD_TEXT_SIZE) <= I_ENEMIES, "the wave count runs into the enemies");
        assert!(I_ENEMIES + TANK_GLYPH_W <= I_ENEMY_COUNT);
        assert!(I_ENEMY_COUNT + width("88", HUD_TEXT_SIZE) + 4 <= I_PENDING, "two digits of enemies run into the +N");
        assert!(I_PENDING + width("+88", HUD_TEXT_SIZE) <= I_FROG, "the +N runs into the frog's gauge");
        assert_eq!(I_FROG + GAUGE_SLOT_W, INFO_W as i32, "the row is the cluster's width");
    }

    /// A chip: its number over its gauge, both inside it.
    #[test]
    fn a_chip_holds_its_number_over_its_gauge() {
        assert!(width("8", UI_SMALL_TEXT) <= CHIP_W as i32);
        assert!(1 + UI_SMALL_TEXT < CHIP_H as i32 - GAUGE_H - 1, "the seat number overlaps its gauge");
        assert!(CHIP_W as i32 > 4, "a gauge needs room for its outline and a block of fill");
    }
}
