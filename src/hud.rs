//! The HUD bar: the player's readouts, drawn in the panel above the
//! battlefield rather than over it (docs/hud-and-builder-layout-design.md,
//! variant A). Presentation only, same category as `game.rs`: `HudModel`
//! is a handful of plain numbers gathered from `Game` between the update
//! and the draw, and `render::hud::draw_bar` lays them out as a row of
//! fixed slots so a number never shifts its neighbours when it changes
//! width. This half is headless - the model, the shared colours and sizes,
//! and every button and dialog rect the hit tests read; the slot tables
//! and the drawing are the `render` half.
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

use crate::math::{Color, Rectangle};

use crate::simulation::{with_frog, with_tank, Game, PlayerCount, RollIn};
use crate::tank::{ActiveWeapon, Tank};
use crate::tuning::tuning;
use crate::{Rect, MAX_DAMAGE};

/// The bar's number/text size.
pub const HUD_TEXT_SIZE: i32 = 18;
/// The small labels over the timed-buff bars (`SPEED`/`SHIELD`/`FROG`).
pub const HUD_LABEL_SIZE: i32 = 10;
/// The version line near the field's bottom-right corner, in the size of
/// the bar's small labels (`SPEED`/`SHIELD`/`FROG`).
pub const HUD_VERSION_TEXT_SIZE: i32 = HUD_LABEL_SIZE;
/// How far the version line's right end sits in from the field's right
/// edge - the same as its bottom inset, so it sits square in the corner.
/// The web page's overlay controls live in the opposite, bottom-left
/// corner (site/src/pages/index.astro `.overlay-controls`).
pub const HUD_VERSION_RIGHT_INSET: i32 = 11;
/// How far the version line's bottom sits up from the field's bottom
/// edge.
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
/// Weapon slots, in bar order. Four of them: laser, plasma, minigun,
/// flamethrower.
pub const WEAPON_SLOTS: usize = 4;

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
    /// Which slot table the bar is laid out from: the one-player table at
    /// one seat, the two-player one from two seats up (the couch layouts
    /// are all the bar has - an online round's own strip is its own lane).
    pub players: PlayerCount,
    pub p1: PlayerHud,
    /// Seat 1's readouts from two seats up; a wreck, or an empty seat,
    /// shows zeros.
    pub p2: Option<PlayerHud>,
    /// The objective frog's health fraction, `None` in a round without one.
    pub frog: Option<f32>,
}

impl HudModel {
    pub fn gather(game: &Game) -> Self {
        let player = game.player().expect("player entity spawned in init");
        let p1 = PlayerHud::gather(game, player);
        let p2 = (game.players.count() >= 2)
            .then(|| game.seat(1).map(|e| PlayerHud::gather(game, e)).unwrap_or_else(PlayerHud::empty));
        let mut title = game.mission.name().to_ascii_uppercase();
        let wave = game.wave_status();
        if let Some(w) = &wave {
            title.push_str(&format!(" {}/{}", w.index, w.total));
        }
        // Live enemies: the ones standing on the field. A wave tank still
        // rolling in is outside it and counts as pending instead - which
        // is what `RollIn` marks, rather than the `Ai` it has not been
        // given yet, because a replica's enemies never have one
        // (docs/online-coop-prd.md §4.5).
        let first_enemy = game.first_enemy_slot();
        let enemies_alive = game
            .world
            .query::<&Tank>()
            .without::<&RollIn>()
            .iter()
            .filter(|tank| !tank.is_wreck() && tank.owner_slot() >= first_enemy)
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

/// The mode button's slot at the bar's right end: `BUILD` in play mode,
/// `PLAY` in build mode (docs/game-editor-fusion.md, sections 6 and 7).
/// Full bar height, so a finger has the most to aim at.
pub const MODE_BUTTON_W: f32 = 72.0;

/// The `ONLINE` button, left of the players button: the way into the
/// lobby (`lobby.rs`, docs/online-coop-prd.md §4.10). Wide enough for
/// its six characters, and the bar's last free slot before the gauges.
/// Not drawn where a build cannot reach a room (`ONLINE_AVAILABLE`).
pub const ONLINE_BUTTON_W: f32 = 80.0;

/// Where the `ONLINE` button sits in `panel` (window space): left of the
/// players button, so all three follow `--resolution` together and every
/// hit test agrees on them.
pub fn online_button_rect(panel: Rect) -> Rectangle {
    let p = players_button_rect(panel);
    Rectangle::new(p.x - PLAYERS_BUTTON_GAP - ONLINE_BUTTON_W, p.y, ONLINE_BUTTON_W, p.height)
}

/// The colour of anything to do with a room: the `ONLINE` button, the
/// lobby's accents and the round's status line. Deliberately not the
/// builder's amber - a room is not an edit.
pub const ONLINE_COLOR: Color = Color::new(120, 220, 255, 255);

/// Where the RESTART button sits: the players button's slot, which is free
/// exactly where this button is drawn (no keyboard means no R key and no
/// second player - `KEYBOARD_AVAILABLE`), so nothing else in the bar moves.
pub fn restart_button_rect(panel: Rect) -> Rectangle {
    players_button_rect(panel)
}

pub const MODE_BUTTON_RIGHT_INSET: f32 = 8.0;

/// Where the mode button sits in `panel` (window space). Shared by the
/// play bar, the build bar and every hit-test, so a tool's `click` and a
/// finger agree on it.
pub fn mode_button_rect(panel: Rect) -> Rectangle {
    Rectangle::new(panel.x + panel.w - MODE_BUTTON_RIGHT_INSET - MODE_BUTTON_W, panel.y, MODE_BUTTON_W, panel.h)
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

/// What play-mode chrome `Game::render` draws besides the readouts: the
/// `BUILD` and players buttons in the bar and, while the player is being
/// asked, the leave-round or players dialog over a dimmed field.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayChrome {
    pub build_button: bool,
    pub players_button: bool,
    /// The `ONLINE` button, which opens the lobby.
    pub online_button: bool,
    /// The RESTART button in the players button's slot, where there is no
    /// keyboard for the R key (`KEYBOARD_AVAILABLE`).
    pub restart_button: bool,
    pub leave_dialog: bool,
    pub players_dialog: bool,
    /// One line along the field's top edge while an online round runs:
    /// the room code, this seat and the snapshot buffer
    /// (`net::round::OnlineRound::status`). `None` in a local round -
    /// and everything before the round is the lobby's, not this line's.
    pub status: Option<String>,
    /// The lobby over a dimmed field (`lobby.rs`), in place of the round
    /// this window is not playing.
    pub lobby: Option<crate::lobby::LobbyView>,
    /// The words before the end screen's countdown. `None` is the local
    /// round's "Restarting in", which is what a local round does; an
    /// online round's counts down to the room's lobby instead.
    pub countdown_label: Option<&'static str>,
}

/// The online status line's text size and how far in from the field's
/// top-left corner it sits: the corner the debug overlay label uses, and
/// free in a release build.
pub const HUD_STATUS_TEXT_SIZE: i32 = 14;
pub const HUD_STATUS_INSET: i32 = 11;

/// The status line's colour: the room blue, so a round somebody else is
/// simulating never reads as one of the HUD's own numbers.
pub const HUD_STATUS_COLOR: Color = ONLINE_COLOR;

#[cfg(test)]
mod hud_tests {
    use super::*;

    /// The three bar buttons sit in a row at the bar's right end, in
    /// their fixed order, all of them full bar height and none of them
    /// on top of another.
    #[test]
    fn the_online_button_sits_left_of_the_players_button() {
        let panel = Rect::new(0.0, 0.0, crate::DEFAULT_SCREEN_WIDTH as f32, crate::HUD_BAR_HEIGHT as f32);
        let online = online_button_rect(panel);
        let players = players_button_rect(panel);
        let mode = mode_button_rect(panel);
        assert_eq!(online.height, panel.h);
        assert!(online.width >= "ONLINE".len() as f32 * 11.0, "the label fits its slot");
        assert!(online.x + online.width + PLAYERS_BUTTON_GAP <= players.x);
        assert!(players.x + players.width + PLAYERS_BUTTON_GAP <= mode.x);
        assert!(mode.x + mode.width <= panel.w);
        assert!(online.x >= 0.0);
        // The RESTART button of a keyboard-less build shares the players
        // slot, so the row is the same three rects either way.
        assert_eq!(restart_button_rect(panel), players);
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
