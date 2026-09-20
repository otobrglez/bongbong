//! Drawing the HUD bar, its buttons and the two dialogs, and the slot
//! tables the bar is laid out from (`hud.rs` owns the model, the shared
//! colours and sizes, and every hit rect).

use sola_raylib::prelude::*;

use crate::hud::{
    leave_dialog_rects, mode_button_rect, players_button_rect, players_dialog_rects, restart_button_rect, weapon_color, HudModel,
    WeaponSlot, BAR_FILL, BUILD_COLOR, DIALOG_W, DIM, HUD_LABEL_SIZE, HUD_TEXT_SIZE, TEXT, WEAPON_SLOTS,
};
use crate::math::{Color, Rectangle};
use crate::render::game::Textures;
use crate::simulation::PlayerCount;
use crate::tank::{ActiveWeapon, TEAM_COLORS};
use crate::{Rect, PICKUP_TEXTURE_SIZE, SHELL_TEXTURE_SIZE};

const HEART: Color = Color::new(230, 60, 70, 255);
const SPEED_COLOR: Color = Color::new(255, 210, 60, 255);
const SHIELD_COLOR: Color = Color::new(170, 120, 255, 255);
const FROG_COLOR: Color = Color::new(120, 220, 90, 255);

// Slot origins along the bar, left to right. Fixed so a wave count or an
// ammo number changing width never nudges what sits after it.
const SLOT_TITLE: i32 = 8;
const BAR_SLOT_W: i32 = 40;
const BAR_W: i32 = 36;
const BAR_H: i32 = 8;
/// The two stacked bars of a two-player round: each this tall, one block
/// apart, the pair ending where the single bar does.
const BAR2_H: i32 = 6;
const BAR2_GAP: i32 = 1;
/// Approximate default-font advance per character at `HUD_TEXT_SIZE`; the
/// pairs are laid out in cells of this width.
pub const CHAR_W: i32 = 12;
/// The small readout font (the two-player weapon pairs) and its cell.
const HUD_SMALL_TEXT_SIZE: i32 = 10;
const CHAR_W_SMALL: i32 = 7;

/// The slot origins one table of readouts is laid out from. One table per
/// player count: the pairs of a two-player round need wider HP, shell and
/// weapon slots, paid for out of the gap after the enemy count. The two
/// width fields are the budget `bar_tests` pins each table against.
#[cfg_attr(not(test), allow(dead_code))]
struct Slots {
    enemies: i32,
    enemy_count: i32,
    heart: i32,
    hp: i32,
    shell: i32,
    shells: i32,
    weapons: i32,
    weapon_slot_w: i32,
    bars: i32,
    /// Width of the HP readout (`100|100` with two players).
    hp_w: i32,
    /// Width of a shell or weapon count (`20|20` with two players).
    count_w: i32,
}

const SLOTS_ONE: Slots = Slots {
    enemies: 176,
    enemy_count: 204,
    heart: 276,
    hp: 292,
    shell: 332,
    shells: 368,
    weapons: 408,
    weapon_slot_w: 72,
    bars: 700,
    hp_w: 3 * CHAR_W,
    count_w: 3 * CHAR_W,
};

/// Two players: every pair - HP, shells and the four weapons - is set in
/// the small font (`CHAR_W_SMALL`) so the whole row still ends before the
/// gauges on the 960 px bar.
const SLOTS_TWO: Slots = Slots {
    enemies: 176,
    enemy_count: 204,
    heart: 272,
    hp: 288,
    shell: 340,
    shells: 374,
    weapons: 412,
    weapon_slot_w: 72,
    bars: 704,
    hp_w: 7 * CHAR_W_SMALL,
    count_w: 5 * CHAR_W_SMALL,
};

impl Slots {
    /// Width of the shells readout: `20|20` in the full font with two
    /// players (the weapon pairs use `count_w`, in the small font).
    #[cfg_attr(not(test), allow(dead_code))]
    const fn shells_pair_w(&self) -> i32 {
        5 * CHAR_W_SMALL
    }

    fn for_players(players: PlayerCount) -> &'static Slots {
        match players {
            PlayerCount::One => &SLOTS_ONE,
            PlayerCount::Two => &SLOTS_TWO,
        }
    }
}

/// The tank glyph's footprint: 7 x 7 blocks of 2 px (`draw_tank_glyph`).
#[cfg_attr(not(test), allow(dead_code))]
const TANK_GLYPH_W: i32 = 14;
const TANK_GLYPH_H: i32 = 14;

/// Draw the bar into `panel` (window space). Everything is placed from
/// the panel's origin, so the same function draws it wherever the layout
/// puts the panel.
pub fn draw_bar(d: &mut impl RaylibDraw, panel: Rect, model: &HudModel, textures: &Textures) {
    let px = panel.x.round() as i32;
    let py = panel.y.round() as i32;
    let pw = panel.w.round() as i32;
    let ph = panel.h.round() as i32;
    d.draw_rectangle(px, py, pw, ph, BAR_FILL);
    let s = Slots::for_players(model.players);
    let p2 = model.p2.as_ref();

    // Text sits in the vertical middle of the bar.
    let text_y = py + (ph - HUD_TEXT_SIZE) / 2;

    d.draw_text(&model.title, px + SLOT_TITLE, text_y, HUD_TEXT_SIZE, TEXT);

    // A tank glyph stands for "enemies": the word does not fit the 960 px
    // bar beside everything else, and the count next to a tank reads.
    draw_tank_glyph(d, px + s.enemies, py + (ph - TANK_GLYPH_H) / 2, DIM);
    let alive = format!("{}", model.enemies_alive);
    d.draw_text(&alive, px + s.enemy_count, text_y, HUD_TEXT_SIZE, TEXT);
    if model.enemies_pending > 0 {
        // Two digits at most before the number stops being a count.
        let x = px + s.enemy_count + (alive.len() as i32).min(2) * 12 + 6;
        d.draw_text(&format!("+{}", model.enemies_pending), x, text_y, HUD_TEXT_SIZE, DIM);
    }

    draw_heart(d, px + s.heart, py + (ph - 12) / 2);
    match p2 {
        None => d.draw_text(&format!("{}", model.p1.hp), px + s.hp, text_y, HUD_TEXT_SIZE, model.p1.hp_color),
        Some(p2) => {
            let small_y = py + (ph - HUD_SMALL_TEXT_SIZE) / 2;
            draw_pair_sized(d, px + s.hp, small_y, 3, (&format!("{}", model.p1.hp), team_tinted(model.p1.hp_color, 0)), (&format!("{}", p2.hp), team_tinted(p2.hp_color, 1)), HUD_SMALL_TEXT_SIZE, CHAR_W_SMALL);
        }
    }

    // The shell sprite's in-flight frame, identical on every row of the
    // sheet, full-bleed at its own 32 px.
    let shell_src = Rectangle::new(3.0 * SHELL_TEXTURE_SIZE, 0.0, SHELL_TEXTURE_SIZE, SHELL_TEXTURE_SIZE);
    let shell_dest = Rectangle::new((px + s.shell) as f32, py as f32, SHELL_TEXTURE_SIZE, SHELL_TEXTURE_SIZE);
    d.draw_texture_pro(textures.shells, shell_src, shell_dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
    match p2 {
        None => {
            if model.p1.shells_active {
                active_outline(d, px + s.shell, py, s.weapons - s.shell - 8, ph, TEXT);
            }
            d.draw_text(&format!("{}", model.p1.shells), px + s.shells, text_y, HUD_TEXT_SIZE, model.p1.shells_color);
        }
        Some(p2) => {
            let small_y = py + (ph - HUD_SMALL_TEXT_SIZE) / 2;
            draw_pair_sized(d, px + s.shells, small_y, 2, (&format!("{}", model.p1.shells), team_tinted(model.p1.shells_color, 0)), (&format!("{}", p2.shells), team_tinted(p2.shells_color, 1)), HUD_SMALL_TEXT_SIZE, CHAR_W_SMALL);
            draw_pair_underlines_sized(d, px + s.shells, py, ph, 2, model.p1.shells_active, p2.shells_active, CHAR_W_SMALL);
        }
    }

    for i in 0..WEAPON_SLOTS {
        let slot = model.p1.weapons[i];
        let x = px + s.weapons + i as i32 * s.weapon_slot_w;
        let texture = match slot.weapon {
            ActiveWeapon::Laser => textures.pickup_laser,
            ActiveWeapon::Plasma => textures.pickup_plasma,
            ActiveWeapon::Minigun => textures.pickup_minigun,
            ActiveWeapon::Flamethrower => textures.pickup_flamethrower,
            ActiveWeapon::Shell => textures.shells,
        };
        let stocked = slot.count > 0 || p2.is_some_and(|p| p.weapons[i].count > 0);
        let src = Rectangle::new(0.0, 0.0, PICKUP_TEXTURE_SIZE, PICKUP_TEXTURE_SIZE);
        let dest = Rectangle::new(x as f32, py as f32, PICKUP_TEXTURE_SIZE, PICKUP_TEXTURE_SIZE);
        let tint = if stocked { Color::WHITE } else { Color::new(255, 255, 255, 70) };
        d.draw_texture_pro(texture, src, dest, Vector2::new(0.0, 0.0), 0.0, tint);
        let count_of = |slot: WeaponSlot| -> (String, Color) {
            if slot.count > 0 {
                (format!("{}", slot.count), weapon_color(slot.weapon))
            } else {
                ("--".to_string(), DIM)
            }
        };
        let count_x = x + PICKUP_TEXTURE_SIZE as i32 + 4;
        match p2 {
            None => {
                let (count, color) = count_of(slot);
                d.draw_text(&count, count_x, text_y, HUD_TEXT_SIZE, color);
                if slot.active {
                    active_outline(d, x, py, s.weapon_slot_w - 8, ph, weapon_color(slot.weapon));
                }
            }
            Some(p2) => {
                // The small font: four pairs have to fit before the gauges.
                let (a, ac) = count_of(slot);
                let (b, bc) = count_of(p2.weapons[i]);
                let small_y = py + (ph - HUD_SMALL_TEXT_SIZE) / 2;
                draw_pair_sized(d, count_x, small_y, 2, (&a, ac), (&b, bc), HUD_SMALL_TEXT_SIZE, CHAR_W_SMALL);
                draw_pair_underlines_sized(d, count_x, py, ph, 2, slot.active, p2.weapons[i].active, CHAR_W_SMALL);
            }
        }
    }

    let bars: [(&str, f32, Option<f32>, Color, bool); 3] = [
        ("SPEED", model.p1.speed, p2.map(|p| p.speed), SPEED_COLOR, true),
        ("SHIELD", model.p1.shield, p2.map(|p| p.shield), SHIELD_COLOR, true),
        ("FROG", model.frog.unwrap_or(0.0), None, FROG_COLOR, model.frog.is_some()),
    ];
    for (i, (label, frac, frac2, color, present)) in bars.iter().enumerate() {
        let x = px + s.bars + i as i32 * BAR_SLOT_W;
        let label_color = if *present { DIM } else { Color::new(60, 60, 66, 255) };
        d.draw_text(label, x, py + 5, HUD_LABEL_SIZE, label_color);
        match frac2 {
            // Two thin bars, player 1 over player 2, ending where the
            // single bar does.
            Some(frac2) => {
                let bottom = py + ph - 4;
                draw_gauge(d, x, bottom - 2 * BAR2_H - BAR2_GAP, BAR2_H, *frac, *color, label_color);
                draw_gauge(d, x, bottom - BAR2_H, BAR2_H, *frac2, *color, label_color);
            }
            None => draw_gauge(d, x, py + ph - 4 - BAR_H, BAR_H, if *present { *frac } else { 0.0 }, *color, label_color),
        }
    }
}

/// One outlined bar of `h` px at (`x`, `y`), filled to `frac` in whole
/// 2 px blocks so it drains in steps like every other gauge in the game.
fn draw_gauge(d: &mut impl RaylibDraw, x: i32, y: i32, h: i32, frac: f32, color: Color, outline: Color) {
    d.draw_rectangle_lines_ex(Rectangle::new(x as f32, y as f32, BAR_W as f32, h as f32), 1.0, outline);
    if frac > 0.0 {
        let fill = ((BAR_W - 4) as f32 * frac.clamp(0.0, 1.0) / 2.0).round() as i32 * 2;
        if fill > 0 {
            d.draw_rectangle(x + 2, y + 2, fill, h - 4, color);
        }
    }
}

/// A two-player readout, `left|right`: the left number right-aligned to
/// the dim separator in a cell `digits` wide, the right one after it, each
/// side in its own colour, at an explicit font size and cell width - the weapon
/// pairs use the small font so four slots fit.
#[allow(clippy::too_many_arguments)]
fn draw_pair_sized(d: &mut impl RaylibDraw, x: i32, text_y: i32, digits: i32, left: (&str, Color), right: (&str, Color), size: i32, ch: i32) {
    let sep_x = x + digits * ch;
    let left_x = sep_x - left.0.len() as i32 * ch;
    d.draw_text(left.0, left_x, text_y, size, left.1);
    d.draw_text("|", sep_x + 2, text_y, size, DIM);
    d.draw_text(right.0, sep_x + ch, text_y, size, right.1);
}

/// The two-player stand-in for `active_outline`: a 2 px underline under
/// whichever side of a pair is what that player's trigger fires, in that
/// player's team colour, at an explicit cell width - for the small-font
/// weapon pairs.
#[allow(clippy::too_many_arguments)]
fn draw_pair_underlines_sized(d: &mut impl RaylibDraw, x: i32, y: i32, h: i32, digits: i32, left: bool, right: bool, ch: i32) {
    let w = digits * ch - 2;
    let uy = y + h - 4;
    if left {
        d.draw_rectangle(x, uy, w, 2, TEAM_COLORS[0]);
    }
    if right {
        d.draw_rectangle(x + (digits + 1) * ch, uy, w, 2, TEAM_COLORS[1]);
    }
}

/// A two-player readout colour: `hud_number_color`'s plain white becomes
/// the player's team colour, so each side of a `60|70` pair is its
/// player's, while the warning and critical colours still win.
fn team_tinted(color: Color, player: usize) -> Color {
    let plain = color.r == TEXT.r && color.g == TEXT.g && color.b == TEXT.b && color.a == TEXT.a;
    if plain { TEAM_COLORS[player & 1] } else { color }
}

/// The outline marking which slot the trigger fires: 2 px, inset one
/// block from the bar's top and bottom edges.
fn active_outline(d: &mut impl RaylibDraw, x: i32, y: i32, w: i32, h: i32, color: Color) {
    d.draw_rectangle_lines_ex(
        Rectangle::new((x - 2) as f32, (y + 2) as f32, (w + 4) as f32, (h - 6) as f32),
        2.0,
        color,
    );
}

/// A pixel heart of 2 px blocks, 14x12, for the HP slot. Drawn rather
/// than loaded: it is the one glyph in the game with no sheet of its own.
fn draw_heart(d: &mut impl RaylibDraw, x: i32, y: i32) {
    const ROWS: [&str; 6] = [".##.##.", "#######", "#######", ".#####.", "..###..", "...#..."];
    for (row, line) in ROWS.iter().enumerate() {
        for (col, c) in line.chars().enumerate() {
            if c == '#' {
                d.draw_rectangle(x + col as i32 * 2, y + row as i32 * 2, 2, 2, HEART);
            }
        }
    }
}

/// A pixel tank seen from above, 7x7 blocks of 2 px (14x14): tracks down
/// both sides, the hull between, the barrel up. The players button shows
/// one or two of these, the play bar one for the enemy count.
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

/// The players button: outlined like the mode button, dim until the
/// dialog it opens is up, its glyphs the tank colours themselves.
pub fn draw_players_button(d: &mut impl RaylibDraw, panel: Rect, players: PlayerCount, open: bool) {
    let r = players_button_rect(panel);
    let outline = if open { TEXT } else { DIM };
    d.draw_rectangle_lines_ex(Rectangle::new(r.x, r.y + 2.0, r.width, r.height - 4.0), 2.0, outline);
    let glyph = 14;
    let gy = (r.y + (r.height - glyph as f32) / 2.0) as i32;
    match players {
        PlayerCount::One => draw_tank_glyph(d, (r.x + (r.width - glyph as f32) / 2.0) as i32, gy, TEAM_COLORS[0]),
        PlayerCount::Two => {
            let gap = 6;
            let x = (r.x + (r.width - (2 * glyph + gap) as f32) / 2.0) as i32;
            draw_tank_glyph(d, x, gy, TEAM_COLORS[0]);
            draw_tank_glyph(d, x + glyph + gap, gy, TEAM_COLORS[1]);
        }
    }
}

/// The RESTART button: the bar's frame around a circular arrow, the one
/// restart glyph a phone player reads without a label, in the bar's text
/// colour so it is neither the builder's amber nor a weapon accent. The
/// press is `tuning::request_restart`, the same path as the dev panel's
/// button, and lands as `Input::restart_pressed` like the R key.
pub fn draw_restart_button(d: &mut impl RaylibDraw, panel: Rect) {
    let r = restart_button_rect(panel);
    d.draw_rectangle_lines_ex(Rectangle::new(r.x, r.y + 2.0, r.width, r.height - 4.0), 2.0, TEXT);
    let center = Vector2::new(r.x + r.width / 2.0, r.y + r.height / 2.0);
    // Three quarters of a ring, the gap at the right, an arrowhead on the
    // end that points on around the circle.
    let (inner, outer) = (6.0, 10.0);
    let (start, end) = (45.0, 315.0);
    d.draw_ring(center, inner, outer, start, end, 24, TEXT);
    let rad = (end as f32).to_radians();
    let mid = (inner + outer) / 2.0;
    let tip_at = Vector2::new(center.x + mid * rad.cos(), center.y + mid * rad.sin());
    let tangent = Vector2::new(-rad.sin(), rad.cos());
    let radial = Vector2::new(rad.cos(), rad.sin());
    let tip = Vector2::new(tip_at.x + tangent.x * 6.0, tip_at.y + tangent.y * 6.0);
    let a = Vector2::new(tip_at.x + radial.x * 5.0, tip_at.y + radial.y * 5.0);
    let b = Vector2::new(tip_at.x - radial.x * 5.0, tip_at.y - radial.y * 5.0);
    d.draw_triangle(a, b, tip, TEXT);
    d.draw_triangle(tip, b, a, TEXT);
}

/// The mode button: an outlined slot with its label centred, in `color`.
pub fn draw_mode_button(d: &mut impl RaylibDraw, panel: Rect, label: &str, color: Color) {
    let r = mode_button_rect(panel);
    d.draw_rectangle_lines_ex(Rectangle::new(r.x, r.y + 2.0, r.width, r.height - 4.0), 2.0, color);
    // The default font runs ~11 px per character at 18 px.
    let text_w = label.len() as i32 * 11;
    d.draw_text(label, (r.x + (r.width - text_w as f32) / 2.0) as i32, (r.y + (r.height - HUD_TEXT_SIZE as f32) / 2.0) as i32, HUD_TEXT_SIZE, color);
}

/// The dialog panel both questions share: shadow, rounded fill, outline,
/// a 28 px title and a 16 px line under it.
fn draw_dialog_panel(d: &mut impl RaylibDraw, panel: Rectangle, title: &str, sub: &str) {
    let shadow = Rectangle::new(panel.x + 4.0, panel.y + 4.0, panel.width, panel.height);
    d.draw_rectangle_rounded(shadow, 0.08, 8, Color::new(0, 0, 0, 90));
    d.draw_rectangle_rounded(panel, 0.08, 8, Color::new(20, 20, 24, 240));
    d.draw_rectangle_rounded_lines_ex(panel, 0.08, 8, 1.5, Color::new(0, 0, 0, 150));
    let title_w = title.len() as i32 * 15;
    d.draw_text(title, (panel.x + (DIALOG_W - title_w as f32) / 2.0) as i32, (panel.y + 22.0) as i32, 28, TEXT);
    let sub_w = sub.len() as i32 * 9;
    d.draw_text(sub, (panel.x + (DIALOG_W - sub_w as f32) / 2.0) as i32, (panel.y + 62.0) as i32, 16, DIM);
}

/// One dialog button: an outlined rounded box with its label centred.
fn draw_dialog_button(d: &mut impl RaylibDraw, rect: Rectangle, label: &str, color: Color, fill: Option<Color>) {
    if let Some(fill) = fill {
        d.draw_rectangle_rounded(rect, 0.2, 8, fill);
    }
    d.draw_rectangle_rounded_lines_ex(rect, 0.2, 8, 2.0, color);
    let w = label.len() as i32 * 11;
    d.draw_text(label, (rect.x + (rect.width - w as f32) / 2.0) as i32, (rect.y + (rect.height - HUD_TEXT_SIZE as f32) / 2.0) as i32, HUD_TEXT_SIZE, color);
}

/// Draw the players dialog over the (already dimmed) field: the live
/// count's button highlighted, the other in the action colour. Field
/// space, like `draw_leave_dialog`.
pub fn draw_players_dialog(d: &mut impl RaylibDraw, field: Rect, players: PlayerCount) {
    let r = players_dialog_rects(field);
    draw_dialog_panel(d, r.panel, "How many players?", "P1 arrows + Space    P2 WASD + L.Shift");
    let live_fill = Some(Color::new(255, 255, 255, 40));
    let one_live = players == PlayerCount::One;
    draw_dialog_button(d, r.one, "1 PLAYER", if one_live { TEXT } else { BUILD_COLOR }, one_live.then_some(live_fill).flatten());
    draw_dialog_button(d, r.two, "2 PLAYERS", if one_live { BUILD_COLOR } else { TEXT }, (!one_live).then_some(live_fill).flatten());
}

/// Draw the leave-round dialog over the (already dimmed) field. Field
/// space: call inside the field camera.
pub fn draw_leave_dialog(d: &mut impl RaylibDraw, field: Rect) {
    let r = leave_dialog_rects(field);
    draw_dialog_panel(d, r.panel, "Leave this round?", "Your progress is lost. The map is kept.");
    draw_dialog_button(d, r.leave, "LEAVE ROUND", BUILD_COLOR, None);
    draw_dialog_button(d, r.stay, "KEEP PLAYING", TEXT, None);
}

#[cfg(test)]
mod bar_tests {
    use super::*;
    use crate::hud::{online_button_rect, PLAYERS_BUTTON_GAP};
    use crate::{PICKUP_TEXTURE_SIZE, SHELL_TEXTURE_SIZE};

    fn default_panel() -> Rect {
        Rect::new(0.0, 0.0, crate::DEFAULT_SCREEN_WIDTH as f32, crate::HUD_BAR_HEIGHT as f32)
    }

    /// The slots must stay inside the default bar and never overlap, in
    /// both tables: the widest thing each can hold is written down here,
    /// so growing a slot fails loudly rather than drawing over its
    /// neighbour.
    #[test]
    fn slots_fit_the_default_bar_without_overlapping() {
        let ch = CHAR_W;
        for (name, s) in [("one", &SLOTS_ONE), ("two", &SLOTS_TWO)] {
            let title_end = SLOT_TITLE + "DESTROY 12/12".len() as i32 * ch;
            assert!(title_end <= s.enemies, "{name}: title runs into the enemies glyph");
            let enemies_end = s.enemies + TANK_GLYPH_W;
            assert!(enemies_end <= s.enemy_count, "{name}");
            let count_end = s.enemy_count + 2 * ch + 6 + 3 * ch;
            assert!(count_end <= s.heart, "{name}");
            assert!(s.heart + 14 <= s.hp, "{name}");
            assert!(s.hp + s.hp_w <= s.shell, "{name}: HP runs into the shell sprite");
            assert!(s.shell + SHELL_TEXTURE_SIZE as i32 <= s.shells, "{name}");
            assert!(s.shells + s.count_w <= s.weapons, "{name}: shells run into the weapons");
            assert!(PICKUP_TEXTURE_SIZE as i32 + 4 + s.count_w <= s.weapon_slot_w, "{name}: a weapon count overflows its slot");
            assert!(s.weapons + WEAPON_SLOTS as i32 * s.weapon_slot_w <= s.bars, "{name}: four weapon slots run into the gauges");
            assert!(BAR_W <= BAR_SLOT_W);
            assert!(s.bars + 3 * BAR_SLOT_W <= crate::DEFAULT_SCREEN_WIDTH);
            // The leftmost of the three buttons at the bar's right end
            // is what the gauges have to clear.
            let button = online_button_rect(default_panel());
            assert!((s.bars + 3 * BAR_SLOT_W) as f32 <= button.x, "{name}: bars run into the ONLINE button");
        }
        // The pairs fit their cells: three digits a side for HP, two for
        // the shells, two in the small font for each weapon.
        assert!(3 * CHAR_W_SMALL + CHAR_W_SMALL + 3 * CHAR_W_SMALL <= SLOTS_TWO.hp_w);
        assert!(2 * CHAR_W_SMALL + CHAR_W_SMALL + 2 * CHAR_W_SMALL <= SLOTS_TWO.shells_pair_w());
        assert!(2 * CHAR_W_SMALL + CHAR_W_SMALL + 2 * CHAR_W_SMALL <= SLOTS_TWO.count_w);
        assert!(SLOTS_TWO.shells + SLOTS_TWO.shells_pair_w() <= SLOTS_TWO.weapons, "two: the shells pair runs into the weapons");
    }

    #[test]
    fn the_three_buttons_sit_between_the_bars_and_the_panels_edge() {
        let panel = default_panel();
        let online = online_button_rect(panel);
        let players = players_button_rect(panel);
        let build = mode_button_rect(panel);
        assert!(players.width >= 48.0 && players.height == panel.h);
        assert!(online.x + online.width + PLAYERS_BUTTON_GAP <= players.x);
        assert!(players.x + players.width + PLAYERS_BUTTON_GAP <= build.x);
        assert!((SLOTS_TWO.bars + 3 * BAR_SLOT_W) as f32 <= online.x);
    }

    #[test]
    fn the_stacked_bars_stay_under_their_label_and_inside_the_bar() {
        let ph = crate::HUD_BAR_HEIGHT;
        let bottom = ph - 4;
        let top = bottom - 2 * BAR2_H - BAR2_GAP;
        assert!(top >= 5 + HUD_LABEL_SIZE, "the top bar overlaps the label");
        assert!(bottom <= ph);
    }
}
