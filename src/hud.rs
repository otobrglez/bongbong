//! The HUD bar: the player's readouts, drawn in the panel above the
//! battlefield rather than over it (docs/hud-and-builder-layout-design.md,
//! variant A). Presentation only, same category as `game.rs`: `HudModel`
//! is a handful of plain numbers gathered from `Game` between the update
//! and the draw, and `draw_bar` lays them out as a row of fixed slots so a
//! number never shifts its neighbours when it changes width.
//!
//! The bar is one obstacle cell tall (`HUD_BAR_HEIGHT`), so the 32 px
//! pickup icons and the shell sprite sit in it full-bleed and read as a
//! tab strip.

use sola_raylib::prelude::*;

use crate::ai::Ai;
use crate::game::Textures;
use crate::simulation::{with_frog, with_tank, Game};
use crate::tank::{ActiveWeapon, Tank};
use crate::tuning::tuning;
use crate::{Rect, MAX_DAMAGE, PICKUP_TEXTURE_SIZE, SHELL_TEXTURE_SIZE};

/// The bar's number/text size.
pub const HUD_TEXT_SIZE: i32 = 18;
/// The small labels over the timed-buff bars (`SPEED`/`SHIELD`/`FROG`).
pub const HUD_LABEL_SIZE: i32 = 10;
/// The version line near the field's bottom-right corner.
pub const HUD_VERSION_TEXT_SIZE: i32 = 20;
/// How far the version line's right end sits in from the field's right
/// edge: clear of the web page's `Full screen` button, which occupies the
/// corner itself (about 90 px wide, 10 px in), plus a gap.
pub const HUD_VERSION_RIGHT_INSET: i32 = 120;
/// How far the version line's bottom sits up from the field's bottom
/// edge: chosen so its glyphs sit level with the page's `Full screen`
/// button (a ~22 px box ending 10 px up).
pub const HUD_VERSION_BOTTOM_INSET: i32 = 11;
/// The version line's colour: white at 70%, a step below the HUD's
/// readouts so it never competes with the round.
pub const HUD_VERSION_COLOR: Color = Color::new(255, 255, 255, 179);

/// The build stamp drawn in the field's bottom-right corner, e.g.
/// `v0.0.19 @otobrglez`.
pub fn version_line() -> String {
    format!("v{} @otobrglez", env!("CARGO_PKG_VERSION"))
}

/// Accent colours for the three special weapons: their count in the bar
/// always, their slot's outline while that weapon is the live one.
pub const HUD_LASER_COLOR: Color = Color::new(255, 60, 160, 255);
pub const HUD_PLASMA_COLOR: Color = Color::new(60, 220, 200, 255);
pub const HUD_MINIGUN_COLOR: Color = Color::new(190, 205, 215, 255);

/// The bar's fill - the same `#151515` the web page is set in, so the bar
/// and the page read as one surface around the field. The editor's bar
/// shares these.
pub const BAR_FILL: Color = Color::new(21, 21, 21, 255);
pub const TEXT: Color = Color::WHITE;
pub const DIM: Color = Color::new(110, 110, 118, 255);
const HEART: Color = Color::new(230, 60, 70, 255);
const SPEED_COLOR: Color = Color::new(255, 210, 60, 255);
const SHIELD_COLOR: Color = Color::new(170, 120, 255, 255);
const FROG_COLOR: Color = Color::new(120, 220, 90, 255);

// Slot origins along the bar, left to right. Fixed so a wave count or an
// ammo number changing width never nudges what sits after it.
const SLOT_TITLE: i32 = 8;
const SLOT_ENEMIES: i32 = 272;
const SLOT_ENEMY_COUNT: i32 = 364;
const SLOT_HEART: i32 = 570;
const SLOT_HP: i32 = 592;
const SLOT_SHELL: i32 = 640;
const SLOT_SHELLS: i32 = 674;
const SLOT_WEAPONS: i32 = 724;
const WEAPON_SLOT_W: i32 = 80;
const SLOT_BARS: i32 = 968;
const BAR_SLOT_W: i32 = 56;
const BAR_W: i32 = 48;
const BAR_H: i32 = 8;

/// One of the three special-weapon slots.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponSlot {
    pub weapon: ActiveWeapon,
    /// Charges/ammo left; 0 draws the slot empty (`--`).
    pub count: i32,
    /// This is what the trigger fires right now.
    pub active: bool,
}

/// Everything the bar shows, as plain values. Built once per frame by
/// `HudModel::gather`, so the draw pass never queries the world.
#[derive(Clone, Debug, PartialEq)]
pub struct HudModel {
    /// `PROTECT`, or `PROTECT   WAVE 2/5` in a wave round.
    pub title: String,
    /// Live enemies, and the ones still to roll in (shown as a dim `+N`).
    pub enemies_alive: usize,
    pub enemies_pending: usize,
    pub hp: i32,
    pub hp_color: Color,
    pub shells: i32,
    pub shells_color: Color,
    /// The trigger fires plain shells right now.
    pub shells_active: bool,
    pub weapons: [WeaponSlot; 3],
    /// Fraction of a speed boost left, 0 when none is running.
    pub speed: f32,
    /// Fraction of a shield left, 0 when none is running.
    pub shield: f32,
    /// The objective frog's health fraction, `None` in a round without one.
    pub frog: Option<f32>,
}

impl HudModel {
    pub fn gather(game: &Game) -> Self {
        let player = game.player.expect("player entity spawned in init");
        let t = tuning();
        let (hp, shells, active, laser, plasma, minigun, speed, shield) = with_tank(&game.world, player, |tank| {
            let boost = if t.speed_boost_duration_seconds > 0.0 {
                (tank.speed_boost_timer / t.speed_boost_duration_seconds).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (
                (MAX_DAMAGE - tank.damage).max(0.0).round() as i32,
                tank.shells_ammo,
                tank.active_weapon(),
                tank.laser_charges,
                tank.plasma_ammo,
                tank.minigun_ammo,
                boost,
                tank.shield_charge(),
            )
        });
        let mut title = game.mission.name().to_ascii_uppercase();
        let wave = game.wave_status();
        if let Some(w) = &wave {
            title.push_str(&format!("   WAVE {}/{}", w.index, w.total));
        }
        // Live enemies: the ones on the field with a mind of their own. A
        // wave tank still rolling in has no `Ai` yet and counts as pending.
        let enemies_alive = game
            .world
            .query::<(&Tank, &Ai)>()
            .iter()
            .filter(|(tank, _)| !tank.is_wreck())
            .count();
        let enemies_pending = wave.as_ref().map_or(0, |w| w.pending);
        // The objective: the player's own frog, or, in a round where only
        // the other side has one, that frog.
        let frog = game
            .frog
            .or(game.enemy_frog)
            .map(|e| with_frog(&game.world, e, |f| f.health_fraction()));
        HudModel {
            title,
            enemies_alive,
            enemies_pending,
            hp,
            hp_color: hud_number_color(hp as f32, MAX_DAMAGE),
            shells,
            shells_color: hud_number_color(shells as f32, t.max_shells as f32),
            shells_active: active == ActiveWeapon::Shell,
            weapons: [
                WeaponSlot { weapon: ActiveWeapon::Laser, count: laser, active: active == ActiveWeapon::Laser },
                WeaponSlot { weapon: ActiveWeapon::Plasma, count: plasma, active: active == ActiveWeapon::Plasma },
                WeaponSlot { weapon: ActiveWeapon::Minigun, count: minigun, active: active == ActiveWeapon::Minigun },
            ],
            speed,
            shield,
            frog,
        }
    }
}

/// Colour for a HUD number (shells or HP) given its current value and max:
/// white, orange under `hud_warn_threshold`, red under
/// `hud_critical_threshold`. Shared by both since they are the same
/// current/max shape, just different units.
pub fn hud_number_color(current: f32, max: f32) -> Color {
    let frac = if max > 0.0 { current / max } else { 0.0 };
    if frac < tuning().hud_critical_threshold {
        Color::RED
    } else if frac < tuning().hud_warn_threshold {
        Color::ORANGE
    } else {
        TEXT
    }
}

/// The accent a special weapon's count and active outline are drawn in.
pub fn weapon_color(weapon: ActiveWeapon) -> Color {
    match weapon {
        ActiveWeapon::Laser => HUD_LASER_COLOR,
        ActiveWeapon::Plasma => HUD_PLASMA_COLOR,
        ActiveWeapon::Minigun => HUD_MINIGUN_COLOR,
        ActiveWeapon::Shell => TEXT,
    }
}

/// Draw the bar into `panel` (window space). Everything is placed from
/// the panel's origin, so the same function draws it wherever the layout
/// puts the panel.
pub fn draw_bar(d: &mut impl RaylibDraw, panel: Rect, model: &HudModel, textures: &Textures) {
    let px = panel.x.round() as i32;
    let py = panel.y.round() as i32;
    let pw = panel.w.round() as i32;
    let ph = panel.h.round() as i32;
    d.draw_rectangle(px, py, pw, ph, BAR_FILL);

    // Text sits in the vertical middle of the bar.
    let text_y = py + (ph - HUD_TEXT_SIZE) / 2;

    d.draw_text(&model.title, px + SLOT_TITLE, text_y, HUD_TEXT_SIZE, TEXT);

    d.draw_text("ENEMIES", px + SLOT_ENEMIES, text_y, HUD_TEXT_SIZE, DIM);
    let alive = format!("{}", model.enemies_alive);
    d.draw_text(&alive, px + SLOT_ENEMY_COUNT, text_y, HUD_TEXT_SIZE, TEXT);
    if model.enemies_pending > 0 {
        // Two digits at most before the number stops being a count.
        let x = px + SLOT_ENEMY_COUNT + (alive.len() as i32).min(2) * 12 + 6;
        d.draw_text(&format!("+{}", model.enemies_pending), x, text_y, HUD_TEXT_SIZE, DIM);
    }

    draw_heart(d, px + SLOT_HEART, py + (ph - 12) / 2);
    d.draw_text(&format!("{}", model.hp), px + SLOT_HP, text_y, HUD_TEXT_SIZE, model.hp_color);

    // The shell sprite's in-flight frame, identical on every row of the
    // sheet, full-bleed at its own 32 px.
    let shell_src = Rectangle::new(3.0 * SHELL_TEXTURE_SIZE, 0.0, SHELL_TEXTURE_SIZE, SHELL_TEXTURE_SIZE);
    let shell_dest = Rectangle::new((px + SLOT_SHELL) as f32, py as f32, SHELL_TEXTURE_SIZE, SHELL_TEXTURE_SIZE);
    d.draw_texture_pro(textures.shells, shell_src, shell_dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
    if model.shells_active {
        active_outline(d, px + SLOT_SHELL, py, SLOT_WEAPONS - SLOT_SHELL - 8, ph, TEXT);
    }
    d.draw_text(&format!("{}", model.shells), px + SLOT_SHELLS, text_y, HUD_TEXT_SIZE, model.shells_color);

    for (i, slot) in model.weapons.iter().enumerate() {
        let x = px + SLOT_WEAPONS + i as i32 * WEAPON_SLOT_W;
        let texture = match slot.weapon {
            ActiveWeapon::Laser => textures.pickup_laser,
            ActiveWeapon::Plasma => textures.pickup_plasma,
            ActiveWeapon::Minigun => textures.pickup_minigun,
            ActiveWeapon::Shell => textures.shells,
        };
        let stocked = slot.count > 0;
        let src = Rectangle::new(0.0, 0.0, PICKUP_TEXTURE_SIZE, PICKUP_TEXTURE_SIZE);
        let dest = Rectangle::new(x as f32, py as f32, PICKUP_TEXTURE_SIZE, PICKUP_TEXTURE_SIZE);
        let tint = if stocked { Color::WHITE } else { Color::new(255, 255, 255, 70) };
        d.draw_texture_pro(texture, src, dest, Vector2::new(0.0, 0.0), 0.0, tint);
        let (count, color) = if stocked {
            (format!("{}", slot.count), weapon_color(slot.weapon))
        } else {
            ("--".to_string(), DIM)
        };
        d.draw_text(&count, x + PICKUP_TEXTURE_SIZE as i32 + 4, text_y, HUD_TEXT_SIZE, color);
        if slot.active {
            active_outline(d, x, py, WEAPON_SLOT_W - 8, ph, weapon_color(slot.weapon));
        }
    }

    let bars: [(&str, f32, Color, bool); 3] = [
        ("SPEED", model.speed, SPEED_COLOR, true),
        ("SHIELD", model.shield, SHIELD_COLOR, true),
        ("FROG", model.frog.unwrap_or(0.0), FROG_COLOR, model.frog.is_some()),
    ];
    for (i, (label, frac, color, present)) in bars.iter().enumerate() {
        let x = px + SLOT_BARS + i as i32 * BAR_SLOT_W;
        let label_color = if *present { DIM } else { Color::new(60, 60, 66, 255) };
        d.draw_text(label, x, py + 5, HUD_LABEL_SIZE, label_color);
        let bar_y = py + ph - 4 - BAR_H;
        d.draw_rectangle_lines_ex(
            Rectangle::new(x as f32, bar_y as f32, BAR_W as f32, BAR_H as f32),
            1.0,
            label_color,
        );
        if *present && *frac > 0.0 {
            // Whole 2 px blocks, so the bar drains in steps like every
            // other gauge in the game.
            let fill = ((BAR_W - 4) as f32 * frac.clamp(0.0, 1.0) / 2.0).round() as i32 * 2;
            if fill > 0 {
                d.draw_rectangle(x + 2, bar_y + 2, fill, BAR_H - 4, *color);
            }
        }
    }
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

/// The mode button's slot at the bar's right end: `BUILD` in play mode,
/// `PLAY` in build mode (docs/game-editor-fusion.md, sections 6 and 7).
/// Full bar height, so a finger has the most to aim at.
pub const MODE_BUTTON_W: f32 = 72.0;
pub const MODE_BUTTON_RIGHT_INSET: f32 = 8.0;

/// Where the mode button sits in `panel` (window space). Shared by the
/// play bar, the build bar and every hit-test, so a tool's `click` and a
/// finger agree on it.
pub fn mode_button_rect(panel: Rect) -> Rectangle {
    Rectangle::new(panel.x + panel.w - MODE_BUTTON_RIGHT_INSET - MODE_BUTTON_W, panel.y, MODE_BUTTON_W, panel.h)
}

/// The mode button: an outlined slot with its label centred, in `color`.
pub fn draw_mode_button(d: &mut impl RaylibDraw, panel: Rect, label: &str, color: Color) {
    let r = mode_button_rect(panel);
    d.draw_rectangle_lines_ex(Rectangle::new(r.x, r.y + 2.0, r.width, r.height - 4.0), 2.0, color);
    // The default font runs ~11 px per character at 18 px.
    let text_w = label.len() as i32 * 11;
    d.draw_text(label, (r.x + (r.width - text_w as f32) / 2.0) as i32, (r.y + (r.height - HUD_TEXT_SIZE as f32) / 2.0) as i32, HUD_TEXT_SIZE, color);
}

/// The builder's amber, the colour the play bar's `BUILD` button and the
/// build bar's `PLAY` button share.
pub const BUILD_COLOR: Color = Color::new(255, 200, 80, 255);

/// The leave-round dialog's geometry, in field space: the panel and its
/// two buttons (`LEAVE ROUND`, `KEEP PLAYING`), each at least 48 px tall
/// and 160 px wide so a finger cannot miss.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeaveDialogRects {
    pub panel: Rectangle,
    pub leave: Rectangle,
    pub stay: Rectangle,
}

pub const DIALOG_W: f32 = 440.0;
pub const DIALOG_H: f32 = 170.0;
pub const DIALOG_BUTTON_W: f32 = 176.0;
pub const DIALOG_BUTTON_H: f32 = 48.0;

pub fn leave_dialog_rects(field: Rect) -> LeaveDialogRects {
    let x = (field.w - DIALOG_W) / 2.0;
    let y = (field.h - DIALOG_H) / 2.0;
    let panel = Rectangle::new(x, y, DIALOG_W, DIALOG_H);
    let by = y + DIALOG_H - 16.0 - DIALOG_BUTTON_H;
    let gap = 16.0;
    let bx = x + (DIALOG_W - 2.0 * DIALOG_BUTTON_W - gap) / 2.0;
    LeaveDialogRects {
        panel,
        leave: Rectangle::new(bx, by, DIALOG_BUTTON_W, DIALOG_BUTTON_H),
        stay: Rectangle::new(bx + DIALOG_BUTTON_W + gap, by, DIALOG_BUTTON_W, DIALOG_BUTTON_H),
    }
}

/// Draw the leave-round dialog over the (already dimmed) field. Field
/// space: call inside the field camera.
pub fn draw_leave_dialog(d: &mut impl RaylibDraw, field: Rect) {
    let r = leave_dialog_rects(field);
    let shadow = Rectangle::new(r.panel.x + 4.0, r.panel.y + 4.0, r.panel.width, r.panel.height);
    d.draw_rectangle_rounded(shadow, 0.08, 8, Color::new(0, 0, 0, 90));
    d.draw_rectangle_rounded(r.panel, 0.08, 8, Color::new(20, 20, 24, 240));
    d.draw_rectangle_rounded_lines_ex(r.panel, 0.08, 8, 1.5, Color::new(0, 0, 0, 150));
    let title = "Leave this round?";
    let title_size = 28;
    let title_w = title.len() as i32 * 15;
    d.draw_text(title, (r.panel.x + (DIALOG_W - title_w as f32) / 2.0) as i32, (r.panel.y + 22.0) as i32, title_size, TEXT);
    let sub = "Your progress is lost. The map is kept.";
    let sub_w = sub.len() as i32 * 9;
    d.draw_text(sub, (r.panel.x + (DIALOG_W - sub_w as f32) / 2.0) as i32, (r.panel.y + 62.0) as i32, 16, DIM);
    for (rect, label, color) in [(r.leave, "LEAVE ROUND", BUILD_COLOR), (r.stay, "KEEP PLAYING", TEXT)] {
        d.draw_rectangle_rounded_lines_ex(rect, 0.2, 8, 2.0, color);
        let w = label.len() as i32 * 11;
        d.draw_text(label, (rect.x + (rect.width - w as f32) / 2.0) as i32, (rect.y + (rect.height - HUD_TEXT_SIZE as f32) / 2.0) as i32, HUD_TEXT_SIZE, color);
    }
}

/// What play-mode chrome `Game::render` draws besides the readouts: the
/// `BUILD` button in the bar and, while the player is being asked, the
/// leave-round dialog over a dimmed field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayChrome {
    pub build_button: bool,
    pub leave_dialog: bool,
}

#[cfg(test)]
mod hud_tests {
    use super::*;

    /// The slots must stay inside the default bar and never overlap: the
    /// widest thing each can hold is written down here, so growing a slot
    /// fails loudly rather than drawing over its neighbour.
    #[test]
    fn slots_fit_the_default_bar_without_overlapping() {
        // Approximate default-font advance at 20 px.
        let ch = 12;
        let title_end = SLOT_TITLE + "DESTROY   WAVE 12/12".len() as i32 * ch;
        assert!(title_end <= SLOT_ENEMIES, "title runs into ENEMIES");
        let enemies_end = SLOT_ENEMIES + "ENEMIES".len() as i32 * ch;
        assert!(enemies_end <= SLOT_ENEMY_COUNT);
        let count_end = SLOT_ENEMY_COUNT + 2 * ch + 6 + 3 * ch;
        assert!(count_end <= SLOT_HEART);
        assert!(SLOT_HEART + 14 <= SLOT_HP);
        assert!(SLOT_HP + 3 * ch <= SLOT_SHELL);
        assert!(SLOT_SHELL + SHELL_TEXTURE_SIZE as i32 <= SLOT_SHELLS);
        assert!(SLOT_SHELLS + 3 * ch <= SLOT_WEAPONS);
        assert!(PICKUP_TEXTURE_SIZE as i32 + 4 + 3 * ch <= WEAPON_SLOT_W);
        assert!(SLOT_WEAPONS + 3 * WEAPON_SLOT_W <= SLOT_BARS);
        assert!(BAR_W <= BAR_SLOT_W);
        assert!(SLOT_BARS + 3 * BAR_SLOT_W <= crate::DEFAULT_SCREEN_WIDTH);
        let button = mode_button_rect(Rect::new(0.0, 0.0, crate::DEFAULT_SCREEN_WIDTH as f32, crate::HUD_BAR_HEIGHT as f32));
        assert!((SLOT_BARS + 3 * BAR_SLOT_W) as f32 <= button.x, "bars run into the BUILD button");
    }

    #[test]
    fn the_leave_dialog_buttons_are_finger_sized_and_inside_the_field() {
        let field = Rect::new(0.0, 32.0, crate::DEFAULT_SCREEN_WIDTH as f32, crate::DEFAULT_SCREEN_HEIGHT as f32);
        let r = leave_dialog_rects(field);
        for b in [r.leave, r.stay] {
            assert!(b.width >= 160.0 && b.height >= 48.0);
            assert!(b.x >= r.panel.x && b.x + b.width <= r.panel.x + r.panel.width);
            assert!(b.y >= r.panel.y && b.y + b.height <= r.panel.y + r.panel.height);
        }
        assert!(r.leave.x + r.leave.width + 16.0 <= r.stay.x);
        assert!(r.panel.x >= 0.0 && r.panel.x + r.panel.width <= field.w);
    }
}
