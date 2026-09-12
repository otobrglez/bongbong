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
//!
//! A two-player round (docs/two-players.md) uses a second slot table: HP,
//! shells and the weapon counts become pairs (`60|70`, player 1 on the
//! left), the SPEED and SHIELD bars stack into two thin ones, and the
//! whole-slot outline marking the live weapon becomes an underline under
//! whichever side fires it. The single-player table is untouched.

use sola_raylib::prelude::*;

use crate::ai::Ai;
use crate::game::Textures;
use crate::simulation::{with_frog, with_tank, Game, PlayerCount};
use crate::tank::{ActiveWeapon, Tank, TEAM_COLORS};
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
/// The flamethrower's accent: fuel-orange, the fire ramp's middle.
pub const HUD_FLAME_COLOR: Color = Color::new(255, 140, 40, 255);

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
const BAR_SLOT_W: i32 = 56;
const BAR_W: i32 = 48;
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
/// width fields are the budget `hud_tests` pins each table against.
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
    enemies: 272,
    enemy_count: 364,
    heart: 494,
    hp: 516,
    shell: 564,
    shells: 598,
    weapons: 648,
    weapon_slot_w: 80,
    bars: 968,
    hp_w: 3 * CHAR_W,
    count_w: 3 * CHAR_W,
};

/// Two players: the weapon pairs are set in the small font
/// (`CHAR_W_SMALL`) so four slots still end before the gauges.
const SLOTS_TWO: Slots = Slots {
    enemies: 256,
    enemy_count: 344,
    heart: 416,
    hp: 434,
    shell: 524,
    shells: 560,
    weapons: 628,
    weapon_slot_w: 84,
    bars: 968,
    hp_w: 7 * CHAR_W,
    count_w: 5 * CHAR_W_SMALL,
};

/// Weapon slots, in bar order. Four of them: laser, plasma, minigun,
/// flamethrower.
pub const WEAPON_SLOTS: usize = 4;

impl Slots {
    /// Width of the shells readout: `20|20` in the full font with two
    /// players (the weapon pairs use `count_w`, in the small font).
    #[cfg_attr(not(test), allow(dead_code))]
    const fn shells_pair_w(&self) -> i32 {
        5 * CHAR_W
    }

    fn for_players(players: PlayerCount) -> &'static Slots {
        match players {
            PlayerCount::One => &SLOTS_ONE,
            PlayerCount::Two => &SLOTS_TWO,
        }
    }
}

/// One of the three special-weapon slots.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponSlot {
    pub weapon: ActiveWeapon,
    /// Charges/ammo left; 0 draws the slot empty (`--`).
    pub count: i32,
    /// This is what the trigger fires right now.
    pub active: bool,
}

/// One player's readouts.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerHud {
    pub hp: i32,
    pub hp_color: Color,
    pub shells: i32,
    pub shells_color: Color,
    /// The trigger fires plain shells right now.
    pub shells_active: bool,
    pub weapons: [WeaponSlot; WEAPON_SLOTS],
    /// Fraction of a speed boost left, 0 when none is running.
    pub speed: f32,
    /// Fraction of a shield left, 0 when none is running.
    pub shield: f32,
}

impl PlayerHud {
    /// A wreck's readouts: everything at zero, nothing live.
    fn empty() -> Self {
        let slot = |weapon| WeaponSlot { weapon, count: 0, active: false };
        PlayerHud {
            hp: 0,
            hp_color: hud_number_color(0.0, MAX_DAMAGE),
            shells: 0,
            shells_color: hud_number_color(0.0, tuning().max_shells as f32),
            shells_active: false,
            weapons: [slot(ActiveWeapon::Laser), slot(ActiveWeapon::Plasma), slot(ActiveWeapon::Minigun), slot(ActiveWeapon::Flamethrower)],
            speed: 0.0,
            shield: 0.0,
        }
    }

    fn gather(game: &Game, entity: hecs::Entity) -> Self {
        let t = tuning();
        with_tank(&game.world, entity, |tank| {
            if tank.is_wreck() {
                return PlayerHud::empty();
            }
            let boost = if t.speed_boost_duration_seconds > 0.0 {
                (tank.speed_boost_timer / t.speed_boost_duration_seconds).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let hp = (MAX_DAMAGE - tank.damage).max(0.0).round() as i32;
            let active = tank.active_weapon();
            PlayerHud {
                hp,
                hp_color: hud_number_color(hp as f32, MAX_DAMAGE),
                shells: tank.shells_ammo,
                shells_color: hud_number_color(tank.shells_ammo as f32, t.max_shells as f32),
                shells_active: active == ActiveWeapon::Shell,
                weapons: [
                    WeaponSlot { weapon: ActiveWeapon::Laser, count: tank.laser_charges, active: active == ActiveWeapon::Laser },
                    WeaponSlot { weapon: ActiveWeapon::Plasma, count: tank.plasma_ammo, active: active == ActiveWeapon::Plasma },
                    WeaponSlot { weapon: ActiveWeapon::Minigun, count: tank.minigun_ammo, active: active == ActiveWeapon::Minigun },
                    // Fuel in whole seconds, rounded up.
                    WeaponSlot { weapon: ActiveWeapon::Flamethrower, count: tank.flame_fuel_seconds(), active: active == ActiveWeapon::Flamethrower },
                ],
                speed: boost,
                shield: tank.shield_charge(),
            }
        })
    }
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
    /// Which slot table the bar is laid out from.
    pub players: PlayerCount,
    pub p1: PlayerHud,
    /// Player 2's readouts in a two-player round; a wreck shows zeros.
    pub p2: Option<PlayerHud>,
    /// The objective frog's health fraction, `None` in a round without one.
    pub frog: Option<f32>,
}

impl HudModel {
    pub fn gather(game: &Game) -> Self {
        let player = game.player.expect("player entity spawned in init");
        let p1 = PlayerHud::gather(game, player);
        let p2 = (game.players == PlayerCount::Two)
            .then(|| game.player2.map(|e| PlayerHud::gather(game, e)).unwrap_or_else(PlayerHud::empty));
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
        HudModel { title, enemies_alive, enemies_pending, players: game.players, p1, p2, frog }
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
        ActiveWeapon::Flamethrower => HUD_FLAME_COLOR,
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
    let s = Slots::for_players(model.players);
    let p2 = model.p2.as_ref();

    // Text sits in the vertical middle of the bar.
    let text_y = py + (ph - HUD_TEXT_SIZE) / 2;

    d.draw_text(&model.title, px + SLOT_TITLE, text_y, HUD_TEXT_SIZE, TEXT);

    d.draw_text("ENEMIES", px + s.enemies, text_y, HUD_TEXT_SIZE, DIM);
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
        Some(p2) => draw_pair(d, px + s.hp, text_y, 3, (&format!("{}", model.p1.hp), team_tinted(model.p1.hp_color, 0)), (&format!("{}", p2.hp), team_tinted(p2.hp_color, 1))),
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
            draw_pair(d, px + s.shells, text_y, 2, (&format!("{}", model.p1.shells), team_tinted(model.p1.shells_color, 0)), (&format!("{}", p2.shells), team_tinted(p2.shells_color, 1)));
            draw_pair_underlines(d, px + s.shells, py, ph, 2, model.p1.shells_active, p2.shells_active);
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

/// A two-player readout, `left|right`, in `CHAR_W` cells: the left number
/// right-aligned to the dim separator in a cell `digits` wide, the right
/// one after it. Each side in its own colour.
fn draw_pair(d: &mut impl RaylibDraw, x: i32, text_y: i32, digits: i32, left: (&str, Color), right: (&str, Color)) {
    draw_pair_sized(d, x, text_y, digits, left, right, HUD_TEXT_SIZE, CHAR_W);
}

/// `draw_pair` at an explicit font size and cell width - the weapon
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
/// player's team colour.
fn draw_pair_underlines(d: &mut impl RaylibDraw, x: i32, y: i32, h: i32, digits: i32, left: bool, right: bool) {
    draw_pair_underlines_sized(d, x, y, h, digits, left, right, CHAR_W);
}

/// `draw_pair_underlines` at an explicit cell width, for the small-font
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
/// one or two of these.
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

/// The players button, left of the mode button: one tank glyph in single
/// player, two in a two-player round, each in its player's team colour. Full
/// bar height like the mode button. Opens the players dialog
/// (`Session::press_players`).
pub const PLAYERS_BUTTON_W: f32 = 48.0;
pub const PLAYERS_BUTTON_GAP: f32 = 8.0;

/// Where the players button sits in `panel` (window space): derived from
/// the mode button's slot, so it follows `--resolution` the same way and
/// every hit-test agrees on it.
pub fn players_button_rect(panel: Rect) -> Rectangle {
    let m = mode_button_rect(panel);
    Rectangle::new(m.x - PLAYERS_BUTTON_GAP - PLAYERS_BUTTON_W, m.y, PLAYERS_BUTTON_W, m.height)
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

/// A centred `DIALOG_W` x `DIALOG_H` panel with two buttons on a row 16 px
/// up from its bottom, 16 px apart: (panel, left, right). Both dialogs.
fn two_button_dialog(field: Rect) -> (Rectangle, Rectangle, Rectangle) {
    let x = (field.w - DIALOG_W) / 2.0;
    let y = (field.h - DIALOG_H) / 2.0;
    let panel = Rectangle::new(x, y, DIALOG_W, DIALOG_H);
    let by = y + DIALOG_H - 16.0 - DIALOG_BUTTON_H;
    let gap = 16.0;
    let bx = x + (DIALOG_W - 2.0 * DIALOG_BUTTON_W - gap) / 2.0;
    (
        panel,
        Rectangle::new(bx, by, DIALOG_BUTTON_W, DIALOG_BUTTON_H),
        Rectangle::new(bx + DIALOG_BUTTON_W + gap, by, DIALOG_BUTTON_W, DIALOG_BUTTON_H),
    )
}

pub fn leave_dialog_rects(field: Rect) -> LeaveDialogRects {
    let (panel, leave, stay) = two_button_dialog(field);
    LeaveDialogRects { panel, leave, stay }
}

/// The players dialog's geometry (field space): the panel and its `1
/// PLAYER` / `2 PLAYERS` buttons, the leave dialog's shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayersDialogRects {
    pub panel: Rectangle,
    pub one: Rectangle,
    pub two: Rectangle,
}

pub fn players_dialog_rects(field: Rect) -> PlayersDialogRects {
    let (panel, one, two) = two_button_dialog(field);
    PlayersDialogRects { panel, one, two }
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

/// What play-mode chrome `Game::render` draws besides the readouts: the
/// `BUILD` and players buttons in the bar and, while the player is being
/// asked, the leave-round or players dialog over a dimmed field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayChrome {
    pub build_button: bool,
    pub players_button: bool,
    pub leave_dialog: bool,
    pub players_dialog: bool,
}

#[cfg(test)]
mod hud_tests {
    use super::*;

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
            let title_end = SLOT_TITLE + "DESTROY   WAVE 12/12".len() as i32 * ch;
            assert!(title_end <= s.enemies, "{name}: title runs into ENEMIES");
            let enemies_end = s.enemies + "ENEMIES".len() as i32 * ch;
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
            let button = players_button_rect(default_panel());
            assert!((s.bars + 3 * BAR_SLOT_W) as f32 <= button.x, "{name}: bars run into the players button");
        }
        // The pairs fit their cells: three digits a side for HP, two for
        // the shells, two in the small font for each weapon.
        assert!(3 * CHAR_W + CHAR_W + 3 * CHAR_W <= SLOTS_TWO.hp_w);
        assert!(2 * CHAR_W + CHAR_W + 2 * CHAR_W <= SLOTS_TWO.shells_pair_w());
        assert!(2 * CHAR_W_SMALL + CHAR_W_SMALL + 2 * CHAR_W_SMALL <= SLOTS_TWO.count_w);
        assert!(SLOTS_TWO.shells + SLOTS_TWO.shells_pair_w() <= SLOTS_TWO.weapons, "two: the shells pair runs into the weapons");
    }

    #[test]
    fn the_players_button_sits_between_the_bars_and_build() {
        let panel = default_panel();
        let players = players_button_rect(panel);
        let build = mode_button_rect(panel);
        assert!(players.width >= 48.0 && players.height == panel.h);
        assert!(players.x + players.width + PLAYERS_BUTTON_GAP <= build.x);
        assert!((SLOTS_TWO.bars + 3 * BAR_SLOT_W) as f32 <= players.x);
    }

    #[test]
    fn the_stacked_bars_stay_under_their_label_and_inside_the_bar() {
        let ph = crate::HUD_BAR_HEIGHT;
        let bottom = ph - 4;
        let top = bottom - 2 * BAR2_H - BAR2_GAP;
        assert!(top >= 5 + HUD_LABEL_SIZE, "the top bar overlaps the label");
        assert!(bottom <= ph);
    }

    #[test]
    fn both_dialogs_have_finger_sized_buttons_inside_the_field() {
        let field = Rect::new(0.0, 32.0, crate::DEFAULT_SCREEN_WIDTH as f32, crate::DEFAULT_SCREEN_HEIGHT as f32);
        let l = leave_dialog_rects(field);
        let p = players_dialog_rects(field);
        for (panel, left, right) in [(l.panel, l.leave, l.stay), (p.panel, p.one, p.two)] {
            for b in [left, right] {
                assert!(b.width >= 160.0 && b.height >= 48.0);
                assert!(b.x >= panel.x && b.x + b.width <= panel.x + panel.width);
                assert!(b.y >= panel.y && b.y + b.height <= panel.y + panel.height);
            }
            assert!(left.x + left.width + 16.0 <= right.x);
            assert!(panel.x >= 0.0 && panel.x + panel.width <= field.w);
        }
    }
}
