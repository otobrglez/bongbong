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
//!
//! Past two seats there is no couch table to pair up, so the bar goes
//! *compact* (docs/online-coop-prd.md §4.11): one seat - the one this
//! window is playing - keeps a whole block of readouts, and every other
//! seat becomes a chip in a strip at the bar's right end, its number and
//! its health gauge in the ring colour that seat's tank wears on the
//! field. An online round is compact from two seats up, since the seat
//! this window steers is rarely seat 1 and the local block has to be
//! *this* player's; a couch round keeps the one- and two-player tables
//! and only goes compact from three.

use crate::math::{Color, Rectangle};

use crate::simulation::{with_frog, with_tank, Game, RollIn};
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
/// Seeker missiles: the pickup icon's lime, clear of the speed gauge's
/// yellow and the flamethrower's orange.
pub const HUD_MISSILES_COLOR: Color = Color::new(190, 240, 70, 255);
/// The flamethrower's accent: fuel-orange, the fire ramp's middle.
pub const HUD_FLAME_COLOR: Color = Color::new(255, 140, 40, 255);

/// The bar's fill - the same `#151515` the web page is set in, so the bar
/// and the page read as one surface around the field. The editor's bar
/// shares these.
pub const BAR_FILL: Color = Color::new(21, 21, 21, 255);
pub const TEXT: Color = Color::WHITE;
pub const DIM: Color = Color::new(110, 110, 118, 255);
/// Weapon slots, in bar order. Five of them: laser, plasma, minigun,
/// missiles, flamethrower.
pub const WEAPON_SLOTS: usize = 5;

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
            weapons: [
                slot(ActiveWeapon::Laser),
                slot(ActiveWeapon::Plasma),
                slot(ActiveWeapon::Minigun),
                slot(ActiveWeapon::Missiles),
                slot(ActiveWeapon::Flamethrower),
            ],
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
                    WeaponSlot { weapon: ActiveWeapon::Missiles, count: tank.missile_ammo, active: active == ActiveWeapon::Missiles },
                    // Fuel in whole seconds, rounded up.
                    WeaponSlot { weapon: ActiveWeapon::Flamethrower, count: tank.flame_fuel_seconds(), active: active == ActiveWeapon::Flamethrower },
                ],
                speed: boost,
                shield: tank.shield_charge(),
            }
        })
    }
}

/// One of the other seats, as the compact strip shows it: which seat,
/// how much health, and whether it is still standing. No weapons and no
/// buffs - that is the local seat's block's job, and a chip has to stay
/// narrow enough that seven of them fit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeatHud {
    /// The owner slot, which is both the `P3` label and the ring colour
    /// that tank wears on the field (`tank::team_color`).
    pub seat: u8,
    /// Health as a fraction of full; 0 for a wreck or an unspawned seat.
    pub health: f32,
    /// Still standing.
    pub alive: bool,
}

impl SeatHud {
    fn gather(game: &Game, seat: usize) -> Self {
        let (health, alive) = match game.seat(seat) {
            Some(entity) => with_tank(&game.world, entity, |tank| {
                if tank.is_wreck() {
                    (0.0, false)
                } else {
                    (((MAX_DAMAGE - tank.damage).max(0.0) / MAX_DAMAGE).clamp(0.0, 1.0), true)
                }
            }),
            None => (0.0, false),
        };
        SeatHud { seat: seat as u8, health, alive }
    }
}

/// Which slot table the bar is laid out from. One table per shape of
/// round, picked by `HudModel::gather` and read by `Slots::for_layout`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HudLayout {
    /// One seat: the plain row of readouts.
    One,
    /// Two on one couch: HP, shells and the weapon counts as pairs.
    Two,
    /// The local seat's readouts, then a chip per other seat
    /// (docs/online-coop-prd.md §4.11).
    Compact,
}

impl HudLayout {
    /// The table a round of `seats` seats is drawn from. `local_seat` is
    /// the seat this window plays in an online round and `None` on a
    /// couch, which is the whole difference: a couch of two has a table
    /// that shows both, a room of two has to put the local seat - seat 1
    /// as often as seat 0 - in the block and the other in the strip.
    pub fn choose(seats: usize, local_seat: Option<u8>) -> HudLayout {
        match (seats, local_seat) {
            (0 | 1, _) => HudLayout::One,
            (2, None) => HudLayout::Two,
            _ => HudLayout::Compact,
        }
    }
}

/// The seats the strip lists: every seat of the round but the local one,
/// in seat order. A wrecked seat keeps its place - the strip is built
/// from the round's seat count, never from who is alive, so a death
/// never shifts a chip.
pub fn other_seats(seats: usize, local: usize) -> Vec<usize> {
    (0..seats).filter(|&i| i != local).collect()
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
    pub layout: HudLayout,
    /// The seat whose readouts fill the block: player 1 on a couch, the
    /// seat this window is playing in a room.
    pub local: PlayerHud,
    /// Seat 1's readouts in a two-player couch round, paired with the
    /// block's; `None` in every other layout. A wreck shows zeros.
    pub second: Option<PlayerHud>,
    /// The other seats, in seat order, in the compact layout; empty in
    /// the couch ones.
    pub others: Vec<SeatHud>,
    /// The objective frog's health fraction, `None` in a round without one.
    pub frog: Option<f32>,
}

impl HudModel {
    /// The bar's numbers for the round on screen. `local_seat` is the
    /// seat this window holds in a room and `None` in a couch round; a
    /// seat the round has not spawned yet - the frame before a replica's
    /// `Welcome` builds one - falls back to seat 0, which is the local
    /// round still on screen behind it.
    pub fn gather(game: &Game, local_seat: Option<u8>) -> Self {
        let seats = game.players.count();
        let local_index = local_seat.map_or(0, |s| s as usize).min(seats.saturating_sub(1));
        let layout = HudLayout::choose(seats, local_seat);
        let local = game
            .seat(local_index)
            .map(|e| PlayerHud::gather(game, e))
            .unwrap_or_else(PlayerHud::empty);
        let second = (layout == HudLayout::Two)
            .then(|| game.seat(1).map(|e| PlayerHud::gather(game, e)).unwrap_or_else(PlayerHud::empty));
        let others = match layout {
            HudLayout::Compact => other_seats(seats, local_index).into_iter().map(|i| SeatHud::gather(game, i)).collect(),
            _ => Vec::new(),
        };
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
        HudModel { title, enemies_alive, enemies_pending, layout, local, second, others, frog }
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
        ActiveWeapon::Missiles => HUD_MISSILES_COLOR,
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
/// lobby's accents, the round's status line and its `LEAVE` button.
/// Deliberately not the builder's amber - a room is not an edit.
pub const ONLINE_COLOR: Color = Color::new(120, 220, 255, 255);

/// Where the `LEAVE` button of an online round sits: the mode button's
/// slot, which is free exactly while the round is the room's (a replica
/// has no builder to switch to, so no `BUILD` button is drawn). It is the
/// way out on a build with no keyboard for the Esc key, and the only one
/// anywhere that says so; the press is `Session::leave_online`.
pub fn leave_button_rect(panel: Rect) -> Rectangle {
    mode_button_rect(panel)
}

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
    /// The `LEAVE` button in the mode button's slot: an online round's
    /// way back to the local one without a keyboard.
    pub leave_button: bool,
    /// The seat this window is playing in a room, which is the one the
    /// bar shows in full. `None` in a couch round, where the bar is
    /// player 1's and the couch tables apply.
    pub seat: Option<u8>,
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
    use crate::map::MapFile;
    use crate::simulation::PlayerCount;
    use crate::MAX_SEATS;

    const W: f32 = crate::DEFAULT_SCREEN_WIDTH as f32;
    const H: f32 = crate::DEFAULT_SCREEN_HEIGHT as f32;

    /// A round of `seats` seats on the shipped map, seeded so the tanks
    /// land in the same places every run.
    fn round(seats: usize) -> Game {
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(11);
        game.players = PlayerCount::from_count(seats).expect("a seat count the round holds");
        game.map = MapFile::from_toml_str(include_str!("../maps/default.toml")).expect("the default map parses");
        game.init(W, H);
        game
    }

    fn wreck(game: &mut Game, seat: usize) {
        let entity = game.seat(seat).expect("the seat has a tank");
        game.world.get::<&mut Tank>(entity).expect("a tank").damage = MAX_DAMAGE;
    }

    /// One seat and two on a couch keep the tables they have always had;
    /// everything else - a third couch seat, and every room of two or
    /// more - is the compact one.
    #[test]
    fn the_couch_keeps_its_tables_and_a_room_is_compact_from_two() {
        assert_eq!(HudLayout::choose(1, None), HudLayout::One);
        assert_eq!(HudLayout::choose(2, None), HudLayout::Two);
        for seats in 3..=MAX_SEATS {
            assert_eq!(HudLayout::choose(seats, None), HudLayout::Compact, "{seats} on a couch");
        }
        // A room: the seat this window plays is rarely seat 0, so even a
        // pair goes compact rather than pairing the bar up.
        assert_eq!(HudLayout::choose(1, Some(0)), HudLayout::One, "the rig's one seat");
        for seats in 2..=MAX_SEATS {
            for seat in 0..seats as u8 {
                assert_eq!(HudLayout::choose(seats, Some(seat)), HudLayout::Compact, "{seats} seats at seat {seat}");
            }
        }
    }

    /// The strip lists every seat but the local one, in seat order, at
    /// every count and from every seat.
    #[test]
    fn the_strip_lists_the_other_seats_in_order() {
        for seats in 1..=MAX_SEATS {
            for local in 0..seats {
                let others = other_seats(seats, local);
                assert_eq!(others.len(), seats - 1, "{seats} seats at {local}");
                assert!(!others.contains(&local), "the local seat is the block, not a chip");
                assert!(others.windows(2).all(|w| w[0] < w[1]), "seat order: {others:?}");
                assert!(others.iter().all(|&i| i < seats));
            }
        }
    }

    /// The block is the local seat's, whichever seat that is, and the
    /// chips are all the others - so a four-seat room at seat 2 reads its
    /// own shells in the bar and seats 1, 2 and 4 in the strip.
    #[test]
    fn the_block_is_the_local_seats_and_the_chips_are_the_rest() {
        let game = round(4);
        for (seat, shells) in [(0usize, 3), (1, 4), (2, 5), (3, 6)] {
            let entity = game.seat(seat).expect("the seat has a tank");
            game.world.get::<&mut Tank>(entity).expect("a tank").shells_ammo = shells;
        }
        for seat in 0..4u8 {
            let model = HudModel::gather(&game, Some(seat));
            assert_eq!(model.layout, HudLayout::Compact);
            assert_eq!(model.local.shells, 3 + seat as i32, "seat {seat}'s own block");
            assert_eq!(model.second, None, "the compact layout pairs nothing");
            let listed: Vec<u8> = model.others.iter().map(|s| s.seat).collect();
            assert_eq!(listed, (0..4u8).filter(|&i| i != seat).collect::<Vec<_>>());
            assert!(model.others.iter().all(|s| s.alive && s.health > 0.0));
        }
        // A couch round of four is the same layout, read from seat 0.
        let couch = HudModel::gather(&game, None);
        assert_eq!(couch.layout, HudLayout::Compact);
        assert_eq!(couch.local.shells, 3);
        assert_eq!(couch.others.iter().map(|s| s.seat).collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    /// A seat dying takes nothing away: the same chips in the same order,
    /// the dead one empty and marked. Numbers moving do not shift the
    /// list either.
    #[test]
    fn a_wrecked_seat_keeps_its_chip() {
        let mut game = round(5);
        let before = HudModel::gather(&game, Some(1));
        wreck(&mut game, 3);
        let after = HudModel::gather(&game, Some(1));
        let seats = |m: &HudModel| m.others.iter().map(|s| s.seat).collect::<Vec<_>>();
        assert_eq!(seats(&before), seats(&after), "a death never moves a chip");
        let dead = after.others.iter().find(|s| s.seat == 3).expect("still listed");
        assert!(!dead.alive && dead.health == 0.0);
        assert!(after.others.iter().filter(|s| s.seat != 3).all(|s| s.alive));
        // The local seat's own block is untouched by any of it.
        assert_eq!(after.local, before.local);
    }

    /// A seat the round has not spawned - the frames before a replica's
    /// welcome builds one, when the local round is still on screen -
    /// falls back to seat 0 rather than drawing an empty block.
    #[test]
    fn a_seat_the_round_does_not_hold_falls_back_to_the_first() {
        let game = round(1);
        let model = HudModel::gather(&game, Some(5));
        assert_eq!(model.layout, HudLayout::One);
        assert!(model.others.is_empty());
        assert_eq!(model.local, HudModel::gather(&game, None).local);
    }

    /// The `LEAVE` button of an online round takes the mode button's
    /// slot, which is free because a replica draws no `BUILD`.
    #[test]
    fn the_leave_button_takes_the_mode_buttons_slot() {
        let panel = Rect::new(0.0, 0.0, crate::DEFAULT_SCREEN_WIDTH as f32, crate::HUD_BAR_HEIGHT as f32);
        let leave = leave_button_rect(panel);
        assert_eq!(leave, mode_button_rect(panel));
        assert!(leave.width >= "LEAVE".len() as f32 * 11.0, "the label fits its slot");
        assert!(leave.height >= 32.0, "a finger has the whole bar to aim at");
        assert!(leave.x + leave.width <= panel.w);
    }

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
