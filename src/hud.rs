//! The HUD: the player's readouts and the buttons around a round, in two
//! clusters in the window's top corners (docs/large-maps-follow-camera.md
//! §8) - the world fills the window, so play and an online round draw no
//! bar; the builder keeps its bar as its toolbar. Presentation only, same
//! category as `game.rs`: `HudModel` is a handful of plain numbers gathered
//! from `Game` between the update and the draw, and `render::hud` lays them
//! out from fixed slots so a number never shifts its neighbours when it
//! changes width. This half is headless - the model, the shared colours and
//! sizes, the window the chrome is laid out in (`UiFrame`, a UI scale in
//! points, never the world's) and every button and dialog rect the hit
//! tests read - `corners`, the two dialogs, the end screen's
//! `result_layout` and the banners' sizes, all in UI points and centred in
//! the chrome's area, so they keep their size whatever the map; the slot
//! tables and the drawing are the `render` half.
//!
//! The left cluster is the seat's vitals, one row: health as a number and
//! a gauge, what the trigger fires with what it has left - the special
//! weapon carried (a tank holds one at a time, `Tank::take_weapon`), else
//! shells -, and the speed and shield gauges, on a row as tall as the
//! right cluster's row (`Corners::row_h`) and a plate as tall as the
//! right cluster's, so the two corners read along one line and stand one
//! height on a phone as on a monitor; the right cluster is always that one
//! row, and a window too narrow for it draws both corners smaller
//! (`Corners::scale`) rather than wrapping it. A two-player couch round
//! (docs/two-players.md) gives player 2 a block of its own beside player
//! 1's, or under it on a narrow window, each block's health gauge in its
//! player's team colour.
//!
//! Past two seats there is no couch pair, so the HUD goes *compact*
//! (docs/online-coop-prd.md §4.11): one seat - the one this window is
//! playing - keeps a whole block of readouts, and every other seat becomes
//! a chip in a strip in the right cluster's row, before its buttons, its number and its
//! health gauge in the ring colour that seat's tank wears on the field. An
//! online round is compact from two seats up, since the seat this window
//! steers is rarely seat 1 and the local block has to be *this* player's; a
//! couch round keeps the one- and two-block layouts and only goes compact
//! from three.

use crate::math::{Color, Rectangle, Vec2};

use crate::simulation::{with_frog, with_tank, Game, RollIn, RoundStats};
use crate::tank::{ActiveWeapon, Tank};
use crate::tuning::tuning;
use crate::{Rect, MAX_DAMAGE};

/// The number/text size of the readouts, here and in the builder's bar.
pub const HUD_TEXT_SIZE: i32 = 18;
/// The default font's own size, which the indicators' labels are drawn at
/// whole multiples of (`indicators::label_font`).
pub const HUD_LABEL_SIZE: i32 = 10;
/// The build stamp under the left cluster, in the chrome's small size.
pub const HUD_VERSION_TEXT_SIZE: i32 = UI_SMALL_TEXT;
/// The version line's colour: white at 70%, a step below the HUD's
/// readouts so it never competes with the round.
pub const HUD_VERSION_COLOR: Color = Color::new(255, 255, 255, 179);

/// The build stamp drawn under the left cluster, e.g.
/// `v0.0.19 @otobrglez`.
pub fn version_line() -> String {
    format!("v{} @otobrglez", env!("CARGO_PKG_VERSION"))
}

/// Accent colours for the special weapons: the count the vitals show and
/// the pips under the ring while that weapon is carried.
pub const HUD_LASER_COLOR: Color = Color::new(255, 60, 160, 255);
pub const HUD_PLASMA_COLOR: Color = Color::new(60, 220, 200, 255);
pub const HUD_MINIGUN_COLOR: Color = Color::new(190, 205, 215, 255);
/// Seeker missiles: the pickup icon's lime, clear of the speed gauge's
/// yellow and the flamethrower's orange.
pub const HUD_MISSILES_COLOR: Color = Color::new(190, 240, 70, 255);
/// The flamethrower's accent: fuel-orange, the fire ramp's middle.
pub const HUD_FLAME_COLOR: Color = Color::new(255, 140, 40, 255);
/// The grenade launcher's accent: the grenade crate's ink, the lamp that
/// blinks on every grenade.
pub const HUD_GRENADES_COLOR: Color = Color::new(0xD6, 0x56, 0xF5, 255);
/// The sonic hammer's accent: the crate's sky-blue ink.
pub const HUD_SONIC_COLOR: Color = Color::new(0x46, 0xC3, 0xF2, 255);

/// The builder bar's fill - the same `#151515` the web page is set in, so
/// the bar and the page read as one surface around the field - and the
/// margins round a letterboxed field. The corners' plates are its dark too.
pub const BAR_FILL: Color = Color::new(21, 21, 21, 255);
pub const TEXT: Color = Color::WHITE;
pub const DIM: Color = Color::new(110, 110, 118, 255);
/// What a tank's trigger fires, as its vitals and the pips under its ring
/// show it: the special weapon it carries, else shells - one readout,
/// since a tank carries one special at a time and fires it until it is
/// spent (`Tank::take_weapon`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponSlot {
    pub weapon: ActiveWeapon,
    /// Shells, charges, rounds or seconds of fuel left.
    pub count: i32,
    /// What a full stock holds (`ActiveWeapon::full_load`).
    pub full: i32,
    /// The count's colour: shells by how full the magazine is
    /// (`hud_number_color`), a special in its accent (`weapon_color`).
    pub color: Color,
}

impl WeaponSlot {
    /// `tank`'s trigger as the vitals and the ring's pips show it.
    pub fn of(tank: &Tank) -> WeaponSlot {
        let weapon = tank.special().unwrap_or(ActiveWeapon::Shell);
        let (count, full) = (tank.weapon_ammo(weapon), weapon.full_load());
        let color = match weapon {
            ActiveWeapon::Shell => hud_number_color(count as f32, full as f32),
            special => weapon_color(special),
        };
        WeaponSlot { weapon, count, full, color }
    }
}

/// One player's readouts.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerHud {
    pub hp: i32,
    pub hp_color: Color,
    /// What the trigger fires and what it has left.
    pub weapon: WeaponSlot,
    /// Fraction of a speed boost left, 0 when none is running.
    pub speed: f32,
    /// Fraction of a shield left, 0 when none is running.
    pub shield: f32,
    /// Lanterns left to set down, on a round that gives them
    /// (`Game::lamps_in_play`); `None` elsewhere.
    pub lamps: Option<u8>,
    /// Fraction of a heat shield left, 0 when none is on.
    pub heat_shield: f32,
}

impl PlayerHud {
    /// A wreck's readouts: everything at zero, nothing live.
    fn empty() -> Self {
        PlayerHud {
            hp: 0,
            hp_color: hud_number_color(0.0, MAX_DAMAGE),
            weapon: WeaponSlot { weapon: ActiveWeapon::Shell, count: 0, full: tuning().max_shells, color: hud_number_color(0.0, 1.0) },
            speed: 0.0,
            shield: 0.0,
            lamps: None,
            heat_shield: 0.0,
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
            PlayerHud {
                hp,
                hp_color: hud_number_color(hp as f32, MAX_DAMAGE),
                weapon: WeaponSlot::of(tank),
                speed: boost,
                shield: tank.shield_charge(),
                lamps: game.lamps_in_play().then(|| game.player_index(entity).map_or(0, |seat| game.lamps_left(seat as usize))),
                heat_shield: tank.heat_shield_fraction(),
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

/// Which seats the left cluster holds. One layout per shape of round,
/// picked by `HudModel::gather` and read by `corners`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HudLayout {
    /// One seat: one block of vitals.
    One,
    /// Two on one couch: a block each, player 2's beside player 1's.
    Two,
    /// The local seat's readouts, then a chip per other seat
    /// (docs/online-coop-prd.md §4.11).
    Compact,
}

impl HudLayout {
    /// The layout a round of `seats` seats is drawn in. `local_seat` is
    /// the seat this window plays in an online round and `None` on a
    /// couch, which is the whole difference: a couch of two shows both
    /// blocks, a room of two has to put the local seat - seat 1 as often
    /// as seat 0 - in the block and the other in the strip.
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

/// Everything the corners show, as plain values. Built once per frame by
/// `HudModel::gather`, so the draw pass never queries the world.
#[derive(Clone, Debug, PartialEq)]
pub struct HudModel {
    /// `PROTECT`, or `PROTECT 2/5` in a wave round.
    pub title: String,
    /// The wave called and how many the round has, in a wave round: what
    /// stands beside the level button when that takes the mission word's
    /// place.
    pub wave: Option<(u32, u32)>,
    /// Live enemies, and the ones still to roll in (shown as a dim `+N`).
    pub enemies_alive: usize,
    pub enemies_pending: usize,
    /// Which seats the left cluster holds.
    pub layout: HudLayout,
    /// The seat whose readouts fill the block: player 1 on a couch, the
    /// seat this window is playing in a room.
    pub local: PlayerHud,
    /// Seat 1's readouts in a two-player couch round, its own block;
    /// `None` in every other layout. A wreck shows zeros.
    pub second: Option<PlayerHud>,
    /// The other seats, in seat order, in the compact layout; empty in
    /// the couch ones.
    pub others: Vec<SeatHud>,
    /// The objective frog's health fraction, `None` in a round without one.
    pub frog: Option<f32>,
}

impl HudModel {
    /// The HUD's numbers for the round on screen. `local_seat` is the
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
        // The mission as one word, in the language on screen; the data
        // name (`Mission::name`) is never shown. A training round says so
        // whatever its mission.
        let mission = if game.map.training.is_some() { crate::text::keys::MISSION_TRAINING } else { crate::text::mission_title(game.mission) };
        let mut title = crate::text::text().get(mission);
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
        let wave = wave.map(|w| (w.index, w.total));
        HudModel { title, wave, enemies_alive, enemies_pending, layout, local, second, others, frog }
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

/// The accent a special weapon's count and ammo pips are drawn in.
pub fn weapon_color(weapon: ActiveWeapon) -> Color {
    match weapon {
        ActiveWeapon::Laser => HUD_LASER_COLOR,
        ActiveWeapon::Plasma => HUD_PLASMA_COLOR,
        ActiveWeapon::Minigun => HUD_MINIGUN_COLOR,
        ActiveWeapon::Missiles => HUD_MISSILES_COLOR,
        ActiveWeapon::Flamethrower => HUD_FLAME_COLOR,
        ActiveWeapon::Grenades => HUD_GRENADES_COLOR,
        ActiveWeapon::SonicHammer => HUD_SONIC_COLOR,
        ActiveWeapon::Shell => TEXT,
    }
}

/// The pickup whose symbol stands for `weapon` in its slot (its crate's,
/// `pickup::draw_glyph`); the shell has none.
pub fn weapon_pickup(weapon: ActiveWeapon) -> Option<crate::pickup::PickupKind> {
    use crate::pickup::PickupKind;
    match weapon {
        ActiveWeapon::Laser => Some(PickupKind::Laser),
        ActiveWeapon::Plasma => Some(PickupKind::Plasma),
        ActiveWeapon::Minigun => Some(PickupKind::Minigun),
        ActiveWeapon::Missiles => Some(PickupKind::Missiles),
        ActiveWeapon::Flamethrower => Some(PickupKind::Flamethrower),
        ActiveWeapon::Grenades => Some(PickupKind::Grenades),
        ActiveWeapon::SonicHammer => Some(PickupKind::SonicHammer),
        ActiveWeapon::Shell => None,
    }
}

// ---- the window the chrome is laid out in ----------------------------------

/// The least room the chrome keeps inside the window's safe area, on every
/// side, in points: nothing sits flush against the glass's edge, a rounded
/// corner or the strip a notch leaves.
pub const UI_EDGE_PT: f32 = 8.0;

/// The smallest area, in points, the chrome is laid out for: the lobby's
/// and the level select's 704 x 336 panel with `UI_EDGE_PT` round it. A
/// safe area smaller than this - an iPhone SE is 667 points across - draws
/// every piece of chrome smaller by one factor (`UiFrame::new`) rather than
/// off its edge, which keeps a 48 pt button over 44 pt down to a safe area
/// 660 points across.
pub const UI_MIN_W: f32 = crate::lobby::LOBBY_W + 2.0 * UI_EDGE_PT;
pub const UI_MIN_H: f32 = crate::lobby::LOBBY_H + 2.0 * UI_EDGE_PT;

/// The smallest text the chrome sets, in points: over the 11 pt Apple's
/// guidance holds text on a phone to.
pub const UI_SMALL_TEXT: i32 = 12;

/// The least side of a touch target on a touch screen, in points (Apple's
/// 44 pt; Android's 48 dp is what the dialogs' and the panels' 48 pt
/// buttons already are).
pub const UI_TOUCH_PT: f32 = 44.0;

/// Which input the game's hints name (docs/large-maps-follow-camera.md §8,
/// "hints that follow the input last used"): the keys - Space, R, Esc,
/// Enter, the arrows, the mouse's wheel - while a keyboard is in use, a tap
/// while a touch screen is. The last input decides (`follow`): a touch
/// that lands turns the hints to taps and a key press turns them back, so
/// a phone playing the web build - a keyboard build - and a laptop with a
/// touch screen each read the input in their hands, never only the build's.
/// Apart from `UiFrame::touch`, which keeps the buttons a finger's size
/// once a touch is seen: a key press changes the words, never a button's
/// size under a thumb.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hints {
    #[default]
    Keys,
    Touch,
}

impl Hints {
    /// The hints a window opens with: taps where there is no keyboard to
    /// press or the mouse stands in for a finger (`touch_only`), keys
    /// elsewhere.
    pub fn at_start(touch_only: bool) -> Hints {
        if touch_only { Hints::Touch } else { Hints::Keys }
    }

    /// The hints after a frame of input: a key pressed turns them to the
    /// keys, a touch landing to taps, and a frame with neither leaves them
    /// as they were. A key and a touch in one frame read as the key, the
    /// press a player makes on purpose.
    pub fn follow(self, touch_landed: bool, key_pressed: bool) -> Hints {
        if key_pressed {
            Hints::Keys
        } else if touch_landed {
            Hints::Touch
        } else {
            self
        }
    }

    /// The message these hints read: `keys` naming the keys, `touch` a
    /// tap.
    pub fn pick(self, keys: crate::text::Key, touch: crate::text::Key) -> crate::text::Key {
        match self {
            Hints::Keys => keys,
            Hints::Touch => touch,
        }
    }

    /// The hints as `status.ui.hints` spells them.
    pub fn name(self) -> &'static str {
        match self {
            Hints::Keys => "keys",
            Hints::Touch => "touch",
        }
    }
}

/// A window's safe area as its insets from each edge, in the window's own
/// units: the strips a notch, a Dynamic Island, rounded corners and a home
/// indicator take, which no chrome may sit under. Zero where the platform
/// keeps the window clear of them itself.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// What the web page's own controls - Full screen, Subscribe - take of the
/// canvas (site/src/scripts/overlay.ts publishes it as `window.bbOverlay`):
/// the band along the canvas's top the game's chrome keeps out of, the way
/// it keeps out of a safe area, and the controls' own rectangle, which the
/// off-screen arrows keep off. In CSS pixels from the canvas's corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PageOverlay {
    /// How far down from the canvas's top the chrome stays clear.
    pub top: f32,
    /// Where the controls stand, `None` where there are none.
    pub rect: Option<Rectangle>,
}

impl PageOverlay {
    /// The page's words: `top x y width height`, numbers in CSS pixels.
    /// Anything else - nothing published, a page without the controls, a
    /// value that is not a number - takes nothing, and a rectangle with no
    /// area is no rectangle.
    pub fn parse(text: &str) -> PageOverlay {
        let numbers: Vec<f32> = text.split_whitespace().map_while(|word| word.parse::<f32>().ok().filter(|v| v.is_finite())).collect();
        let [top, x, y, w, h] = numbers[..] else { return PageOverlay::default() };
        let rect = (w > 0.0 && h > 0.0).then(|| Rectangle::new(x, y, w, h));
        PageOverlay { top: top.max(0.0), rect }
    }

    /// The band as the window's safe-area insets, in its units -
    /// `units_per_point` of them to the CSS pixel.
    pub fn insets(&self, units_per_point: f32) -> Insets {
        Insets { top: self.top * units_per_point, ..Insets::default() }
    }

    /// The controls' rectangle in the window's units.
    pub fn rect_in(&self, units_per_point: f32) -> Option<Rectangle> {
        let u = units_per_point;
        self.rect.map(|r| Rectangle::new(r.x * u, r.y * u, r.width * u, r.height * u))
    }
}

/// The window as the chrome lays itself out in it
/// (docs/large-maps-follow-camera.md §8): how many of the window's units a
/// UI point is - the UI scale every piece of chrome is drawn at, never the
/// world's -, the window in those points and the area inside its safe area
/// the chrome keeps to. Every rect a chrome painter draws and a hit test
/// reads is in these points; `to_ui` takes a pointer there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiFrame {
    /// Window units per UI point.
    pub scale: f32,
    /// The whole window in UI points, from (0, 0): what a dim covers.
    pub screen: Rect,
    /// Where the chrome lays itself out: the safe area, `UI_EDGE_PT` in
    /// from each of its edges.
    pub area: Rect,
    /// A touch screen is in use (no keyboard, `--touch-from-mouse`, or a
    /// touch seen): every button is at least `UI_TOUCH_PT` on both sides.
    pub touch: bool,
    /// The input the hints name (`Hints`): the last one used, which
    /// `app.rs` sets every frame (`with_hints`); a frame laid out for touch
    /// opens on taps, any other on the keys.
    pub hints: Hints,
}

impl UiFrame {
    /// The frame for a window of `window` units, `units_per_point` of them
    /// to the point (one where the window is laid out in points or CSS
    /// pixels, the screen's density over 160 on Android), its safe area
    /// `insets` in from the edges, drawn `knob` (`ui_scale`) times a point
    /// - and smaller, where `UI_MIN_W` x `UI_MIN_H` of those would not fit
    /// the safe area. A window, a scale or an inset that is not a number is
    /// read as one unit, one, or none.
    pub fn new(window: (f32, f32), units_per_point: f32, knob: f32, insets: Insets, touch: bool) -> UiFrame {
        let positive = |v: f32| if v.is_finite() && v > 0.0 { v } else { 1.0 };
        let inset = |v: f32| if v.is_finite() { v.max(0.0) } else { 0.0 };
        let (w, h) = (positive(window.0), positive(window.1));
        let (left, top) = (inset(insets.left), inset(insets.top));
        let safe_w = (w - left - inset(insets.right)).max(1.0);
        let safe_h = (h - top - inset(insets.bottom)).max(1.0);
        let unit = positive(units_per_point) * positive(knob);
        let fit = (safe_w / (unit * UI_MIN_W)).min(safe_h / (unit * UI_MIN_H)).min(1.0);
        let scale = unit * fit;
        let area = Rect::new(
            left / scale + UI_EDGE_PT,
            top / scale + UI_EDGE_PT,
            (safe_w / scale - 2.0 * UI_EDGE_PT).max(0.0),
            (safe_h / scale - 2.0 * UI_EDGE_PT).max(0.0),
        );
        UiFrame { scale, screen: Rect::new(0.0, 0.0, w / scale, h / scale), area, touch, hints: Hints::at_start(touch) }
    }

    /// The same frame, its hints naming `hints`.
    pub fn with_hints(self, hints: Hints) -> UiFrame {
        UiFrame { hints, ..self }
    }

    /// A window of `size` units, a unit a point, nothing inset and no
    /// touch: what the chrome is laid out in where there is no window - a
    /// dev server running headless, a test.
    pub fn plain(size: (f32, f32)) -> UiFrame {
        UiFrame::new(size, 1.0, 1.0, Insets::default(), false)
    }

    /// A window position (a pointer) in UI points.
    pub fn to_ui(&self, window: Vec2) -> Vec2 {
        Vec2::new(window.x / self.scale, window.y / self.scale)
    }

    /// A UI point's window position.
    pub fn to_window(&self, ui: Vec2) -> Vec2 {
        Vec2::new(ui.x * self.scale, ui.y * self.scale)
    }

    /// A rectangle in UI points as the window's.
    pub fn rect_to_window(&self, r: Rectangle) -> Rectangle {
        Rectangle::new(r.x * self.scale, r.y * self.scale, r.width * self.scale, r.height * self.scale)
    }
}

/// The players button: one tank glyph in single player, two in a
/// two-player round, each in its player's team colour. Opens the players
/// dialog (`Session::press_players`).
pub const PLAYERS_BUTTON_W: f32 = 48.0;
/// Between two buttons of a row, here and in the builder's bar.
pub const PLAYERS_BUTTON_GAP: f32 = 8.0;

/// The mode button: `BUILD` in play mode, `PLAY` in the builder's bar
/// (docs/game-editor-fusion.md, sections 6 and 7; `editor::Bar::play`); an
/// online round's `LEAVE` takes its place.
pub const MODE_BUTTON_W: f32 = 72.0;

/// The pause button: two bars while the round runs, a play triangle while
/// it is paused - the P key's stand-in on a screen with no keyboard, drawn
/// on every local round so one press works the same everywhere. A
/// finger's width, since it carries no label.
pub const PAUSE_BUTTON_W: f32 = UI_TOUCH_PT;

/// The `ONLINE` button: the way into the lobby (`lobby.rs`,
/// docs/online-coop-prd.md §4.10). Wide enough for its six characters.
/// Not drawn where a build cannot reach a room (`ONLINE_AVAILABLE`).
pub const ONLINE_BUTTON_W: f32 = 80.0;

/// The colour of anything to do with a room: the `ONLINE` button, the
/// lobby's accents, the round's status line and its `LEAVE` button.
/// Deliberately not the builder's amber - a room is not an edit.
pub const ONLINE_COLOR: Color = Color::new(120, 220, 255, 255);

/// The level button (docs/levels.md): on a level it takes the mission
/// word's place - `LEVEL 3`, the word small and the number in the
/// readouts' size - and opens the level select (`Session::press_levels`).
/// The wave count of a wave round stays beside it; the mission word is the
/// opening banner's. Wide enough for the longest word a shipped language
/// spells it with and a two-digit number (`text_tests`).
pub const LEVEL_BUTTON_W: f32 = 96.0;
/// The gap between the level button's word and its number.
pub const LEVEL_BUTTON_WORD_GAP: i32 = 5;

/// The builder's amber, the colour the `BUILD` button and the build bar's
/// `PLAY` button share.
pub const BUILD_COLOR: Color = Color::new(255, 200, 80, 255);

// ---- the corner clusters ---------------------------------------------------
//
// Play and an online round draw no bar: the world fills the window, and the
// readouts and buttons stand in two clusters in its top corners, inside the
// safe area (docs/large-maps-follow-camera.md §8) - the seat's vitals
// top-left, the round's numbers and the buttons top-right - so both bottom
// corners stay the thumbs' and the stick's. Laid out in UI points
// (`UiFrame`), each cluster's rows from fixed slot tables (`render::hud`),
// so a number changing width never nudges its neighbour; `corners` is the
// one geometry the painter (`render::hud::draw_corners`) and every hit test
// read.

/// A cluster's plate: its padding round the rows it holds.
pub const PLATE_PAD: f32 = 4.0;
/// Between two blocks of the left cluster, and between the right
/// cluster's first row and its buttons when they share a row.
pub const CLUSTER_GAP: f32 = 8.0;
/// Between a cluster's rows.
pub const ROW_GAP: f32 = 4.0;
/// The least room between the left cluster and the right one.
pub const SIDE_GAP: f32 = 16.0;
/// A row of readouts: 32 pt, room for the crates' 24 pt symbols with a
/// margin.
pub const ROW_H: f32 = 32.0;

/// A button's height: a readouts' row under a mouse, a finger's on a touch
/// screen.
pub fn button_height(touch: bool) -> f32 {
    if touch { UI_TOUCH_PT } else { ROW_H }
}

/// One seat's vitals, one row: health, what the trigger fires (the special
/// carried, else shells) and the speed and shield gauges (`render::hud`'s
/// slot table fills it). The row is `Corners::row_h` tall.
pub const VITALS_W: f32 = 310.0;
/// The right cluster's first row: the mission word or the level button,
/// the wave, the enemy count and the frog's gauge.
pub const INFO_W: f32 = 304.0;
/// The mission word's slot at the head of that row, its wave count
/// included: the budget `text_tests` measures every language's mission
/// word against.
pub const INFO_TITLE_W: f32 = 160.0;
/// The buttons' row: `ONLINE`, the players (or RESTART), the pause button
/// and the mode button, right to left from its right end.
pub const BUTTONS_W: f32 = ONLINE_BUTTON_W + PLAYERS_BUTTON_GAP + PLAYERS_BUTTON_W + PLAYERS_BUTTON_GAP + PAUSE_SPAN;
/// The pause button and the mode button to its right.
const PAUSE_SPAN: f32 = PAUSE_BUTTON_W + PLAYERS_BUTTON_GAP + MODE_BUTTON_W;
/// One chip of the other seats' strip: its number over its health gauge.
pub const CHIP_W: f32 = 20.0;
pub const CHIP_H: f32 = 26.0;
pub const CHIP_GAP: f32 = 4.0;
/// One line of text under the left cluster - an online round's status,
/// the build stamp - each on its own dark plate.
pub const LINE_H: f32 = 20.0;
/// The least height the minimap is shrunk to where the window is too short
/// for its size (`corners`): any smaller and it is left out.
pub const MINIMAP_MIN_PT: f32 = 32.0;

/// What the corners hold, which is all their geometry depends on - so the
/// hit tests at the top of a frame and the painter at its end, both
/// reading `PlayChrome`, lay them out alike.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CornerShape {
    /// Which seats the left cluster holds: one block, a couch pair's two,
    /// or one block and a chip per other seat.
    pub layout: HudLayout,
    /// The other seats' chips, in the compact layout.
    pub chips: usize,
    /// The level button in the title slot: a local round on a level.
    pub level_button: bool,
    pub online: bool,
    pub players: bool,
    pub restart: bool,
    pub build: bool,
    pub leave: bool,
    /// The pause button, left of the mode button.
    pub pause: bool,
    /// Lines of text under the left cluster.
    pub lines: usize,
    /// The minimap's size in points (`minimap::MinimapRules::size_pt`),
    /// where the frame draws one (`PlayChrome::minimap`).
    pub minimap: Option<(f32, f32)>,
    /// A third row under each block: the lanterns and the heat shield
    /// (`PlayChrome::lamp_row`).
    pub lamp_row: bool,
}

impl CornerShape {
    /// The corners a frame draws around a round of `seats` seats, or
    /// `None` when it draws none (`PlayChrome::hud`).
    pub fn of(chrome: &PlayChrome, seats: usize) -> Option<CornerShape> {
        chrome.hud.then(|| {
            let layout = HudLayout::choose(seats, chrome.seat);
            CornerShape {
                layout,
                chips: if layout == HudLayout::Compact { seats.saturating_sub(1) } else { 0 },
                level_button: chrome.level_button.is_some(),
                online: chrome.online_button,
                players: chrome.players_button,
                restart: chrome.restart_button,
                build: chrome.build_button,
                leave: chrome.leave_button,
                pause: chrome.pause_button,
                // The build stamp always; an online round's status over it.
                lines: 1 + usize::from(chrome.status.is_some()),
                minimap: chrome.minimap,
                lamp_row: chrome.lamp_row,
            }
        })
    }
}

/// The buttons in the corners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CornerButton {
    Level,
    Online,
    Players,
    Restart,
    Build,
    Leave,
    /// Pauses the round or takes it on again, as the P key does.
    Pause,
    /// The lamp row's lantern count: a press sets a lantern down, as the
    /// lamp key does (docs/volcano.md).
    Lamp,
}

impl CornerButton {
    /// The name the dev server's `status.ui` gives it.
    pub fn name(self) -> &'static str {
        match self {
            CornerButton::Level => "level",
            CornerButton::Online => "online",
            CornerButton::Players => "players",
            CornerButton::Restart => "restart",
            CornerButton::Build => "build",
            CornerButton::Leave => "leave",
            CornerButton::Pause => "pause",
            CornerButton::Lamp => "lamp",
        }
    }
}

/// Where everything in the two corners is, in UI points (`corners`): laid
/// out at full size and drawn `scale` times as large about `origin`, so on
/// a window too narrow for the one row the whole of both corners is
/// smaller rather than wrapped (`unscaled` is the full-size layout the
/// painter draws under that scale).
#[derive(Clone, Debug, PartialEq)]
pub struct Corners {
    /// The vitals blocks: the local seat's first, a couch's player 2's
    /// after it - beside it where the area has the width, else under it.
    /// Each is as tall as the right cluster's row, so every plate along
    /// the top (`block_plate`, `right`) is one height; its row along its
    /// top, its lamp row under that (`lamp_row`).
    pub blocks: Vec<Rectangle>,
    /// The box the lines under the left cluster are kept to.
    pub lines: Rectangle,
    /// The corners' first row, the vitals' and the right cluster's alike:
    /// a button's height (`button_height`) times `scale`, so the two
    /// corners read along one line on every screen - one top, one height,
    /// every readout centred on it - even where a touch screen grows the
    /// level button to a finger's.
    pub row_h: f32,
    /// Every block has a lamp row (`CornerShape::lamp_row`).
    pub lamp_rows: bool,
    /// The right cluster's plate: one row, whatever the window.
    pub right: Rectangle,
    /// Its round's numbers (`INFO_W` wide at full size).
    pub info: Rectangle,
    pub level_button: Option<Rectangle>,
    pub online: Option<Rectangle>,
    pub players: Option<Rectangle>,
    pub restart: Option<Rectangle>,
    pub build: Option<Rectangle>,
    pub leave: Option<Rectangle>,
    pub pause: Option<Rectangle>,
    /// The other seats' strip, in the right cluster's row between its
    /// numbers and its buttons.
    pub chips: Option<Rectangle>,
    /// The minimap's image (`minimap.rs`), right-aligned under the right
    /// cluster on a plate of its own: part of that cluster, which fades as
    /// one. Not a control - a press on it is the HUD's and does nothing.
    pub minimap: Option<Rectangle>,
    /// The local seat's lantern count in its block's lamp row, a button.
    pub lamp: Option<Rectangle>,
    /// How large the corners are drawn: 1 wherever the one row fits the
    /// area at full size, else the factor that makes it fit exactly.
    pub scale: f32,
    /// The point they are scaled about: the area's top-left corner.
    pub origin: Vec2,
}

impl Corners {
    /// A full-size block's plate: its rows and `PLATE_PAD` round them (the
    /// navigator's, the loupe's and the minimap's plates too).
    pub fn plate(block: Rectangle) -> Rectangle {
        Rectangle::new(block.x - PLATE_PAD, block.y - PLATE_PAD, block.width + 2.0 * PLATE_PAD, block.height + 2.0 * PLATE_PAD)
    }

    /// A block's plate at these corners' scale.
    pub fn block_plate(&self, block: Rectangle) -> Rectangle {
        let pad = PLATE_PAD * self.scale;
        Rectangle::new(block.x - pad, block.y - pad, block.width + 2.0 * pad, block.height + 2.0 * pad)
    }

    /// The left cluster: every block's plate and the lines under them -
    /// what fades as one.
    pub fn left(&self) -> Rectangle {
        self.blocks.iter().fold(self.lines, |r, block| union(r, self.block_plate(*block)))
    }

    /// `block`'s lamp row, where the round has lamp rows: as tall as its
    /// first row and a row gap under it.
    pub fn lamp_row(&self, block: Rectangle) -> Option<Rectangle> {
        self.lamp_rows.then(|| Rectangle::new(block.x, block.y + self.row_h + ROW_GAP * self.scale, block.width, self.row_h))
    }

    /// The minimap's plate, where there is one.
    pub fn minimap_plate(&self) -> Option<Rectangle> {
        self.minimap.map(|r| self.block_plate(r))
    }

    /// What no off-screen arrow may sit on and no touch may steer or fire
    /// from: the two clusters, and the minimap's plate under the right one.
    pub fn keep_out(&self) -> Vec<Rectangle> {
        [Some(self.left()), Some(self.right), self.minimap_plate()].into_iter().flatten().collect()
    }

    /// The `i`th chip of the strip: fixed per position, so a seat dying
    /// never moves the chip beside it.
    pub fn chip(&self, i: usize) -> Option<Rectangle> {
        let s = self.scale;
        self.chips.map(|strip| Rectangle::new(strip.x + i as f32 * (CHIP_W + CHIP_GAP) * s, strip.y, CHIP_W * s, CHIP_H * s))
    }

    /// Every button there is, with what it is.
    pub fn buttons(&self) -> Vec<(CornerButton, Rectangle)> {
        [
            (CornerButton::Level, self.level_button),
            (CornerButton::Online, self.online),
            (CornerButton::Players, self.players),
            (CornerButton::Restart, self.restart),
            (CornerButton::Build, self.build),
            (CornerButton::Leave, self.leave),
            (CornerButton::Pause, self.pause),
            (CornerButton::Lamp, self.lamp),
        ]
        .into_iter()
        .filter_map(|(button, rect)| rect.map(|rect| (button, rect)))
        .collect()
    }

    /// The button under `p` (UI points), if any.
    pub fn hit(&self, p: Vec2) -> Option<CornerButton> {
        self.buttons().into_iter().find(|(_, rect)| rect.contains(p)).map(|(button, _)| button)
    }

    /// The same corners at full size, `scale` 1: what the painter draws
    /// under a transform `scale` times as large about `origin`.
    pub fn unscaled(&self) -> Corners {
        let s = self.scale;
        let mut full = self.map(|r| about(r, self.origin, 1.0 / s));
        full.row_h = self.row_h / s;
        full.scale = 1.0;
        full
    }

    /// Every rectangle put through `f`.
    fn map(&self, f: impl Fn(Rectangle) -> Rectangle) -> Corners {
        let opt = |r: Option<Rectangle>| r.map(&f);
        Corners {
            blocks: self.blocks.iter().map(|b| f(*b)).collect(),
            lines: f(self.lines),
            right: f(self.right),
            info: f(self.info),
            level_button: opt(self.level_button),
            online: opt(self.online),
            players: opt(self.players),
            restart: opt(self.restart),
            build: opt(self.build),
            leave: opt(self.leave),
            pause: opt(self.pause),
            chips: opt(self.chips),
            minimap: opt(self.minimap),
            lamp: opt(self.lamp),
            ..self.clone()
        }
    }
}

/// `r` scaled `s` times about `o`.
fn about(r: Rectangle, o: Vec2, s: f32) -> Rectangle {
    Rectangle::new(o.x + (r.x - o.x) * s, o.y + (r.y - o.y) * s, r.width * s, r.height * s)
}

/// The smallest rectangle holding both.
fn union(a: Rectangle, b: Rectangle) -> Rectangle {
    let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
    let (x1, y1) = ((a.x + a.width).max(b.x + b.width), (a.y + a.height).max(b.y + b.height));
    Rectangle::new(x0, y0, x1 - x0, y1 - y0)
}

/// Lay the two clusters out in `ui`'s area (docs/large-maps-follow-camera.md
/// §8): the vitals top-left, a couch's second block beside the first where
/// the area holds both and the right cluster, else under it; the right
/// cluster in the top-right corner as one row - the round's numbers, a
/// room's chips, then the buttons; the minimap, where the frame draws one,
/// on a plate of its own under the right cluster, flush with its right
/// edge, shrunk to the room left above the area's bottom and left out where
/// that is under `MINIMAP_MIN_PT` or would reach the left cluster. Every
/// plate along the top - each block's and the right cluster's - is one
/// height, a block's rows'. Laid out at full size - every button
/// `button_height` tall, 44 pt on a touch screen - and where the row is
/// wider than the area, all of it drawn smaller by the one factor that fits
/// it (`Corners::scale`), so the corners never wrap and never meet.
pub fn corners(ui: &UiFrame, shape: &CornerShape) -> Corners {
    let area = ui.area;
    let button_h = button_height(ui.touch);
    let block_plate = VITALS_W + 2.0 * PLATE_PAD;
    let chips_w = if shape.chips > 0 { shape.chips as f32 * (CHIP_W + CHIP_GAP) - CHIP_GAP } else { 0.0 };
    // The buttons' row reaches from its right end to the leftmost slot the
    // round draws a button in - the mode slot, the pause slot, the players
    // slot, the online slot - so each keeps its own place whichever are
    // drawn.
    let buttons_w = if shape.online {
        BUTTONS_W
    } else if shape.players || shape.restart {
        PLAYERS_BUTTON_W + PLAYERS_BUTTON_GAP + PAUSE_SPAN
    } else if shape.pause {
        PAUSE_SPAN
    } else if shape.build || shape.leave {
        MODE_BUTTON_W
    } else {
        0.0
    };
    let after = |w: f32| if w > 0.0 { CLUSTER_GAP + w } else { 0.0 };
    let right_w = INFO_W + after(chips_w) + after(buttons_w) + 2.0 * PLATE_PAD;

    let couch_pair = shape.layout == HudLayout::Two;
    let beside = couch_pair && 2.0 * block_plate + CLUSTER_GAP + SIDE_GAP + right_w <= area.w;
    let left_w = if beside { 2.0 * block_plate + CLUSTER_GAP } else { block_plate };
    // The one row at full size, and the area it is laid out in: the real
    // one where it fits, else one as many times larger as the row is too
    // wide, which the scale below brings back down onto the real one.
    let scale = (area.w / (left_w + SIDE_GAP + right_w)).min(1.0);
    let origin = Vec2::new(area.x, area.y);
    let area = Rect { x: area.x, y: area.y, w: area.w / scale, h: area.h / scale };

    // The right cluster's row, from its right end: the buttons, the chips,
    // the round's numbers.
    let row_y = area.y + PLATE_PAD;
    let right_edge = area.x + area.w - PLATE_PAD;
    let buttons_row = Rectangle::new(right_edge - buttons_w, row_y, buttons_w, button_h);
    // A gap before the buttons only where there are buttons.
    let to_buttons = if buttons_w > 0.0 { CLUSTER_GAP } else { 0.0 };
    let chips = (shape.chips > 0).then(|| Rectangle::new(buttons_row.x - to_buttons - chips_w, row_y + (button_h - CHIP_H) / 2.0, chips_w, CHIP_H));
    let info_end = chips.map_or(buttons_row.x - to_buttons, |c| c.x - CLUSTER_GAP);
    let info = Rectangle::new(info_end - INFO_W, row_y, INFO_W, button_h);

    // The vitals' row is the right cluster's row's height, so the two
    // corners share one line, and a round with lanterns or lava gives
    // every block a lamp row a row gap under it; the right cluster's plate
    // is as tall as a block's.
    let block_h = button_h + if shape.lamp_row { ROW_GAP + button_h } else { 0.0 };
    let right = Rectangle::new(area.x + area.w - right_w, area.y, right_w, block_h + 2.0 * PLATE_PAD);
    let first = Rectangle::new(area.x + PLATE_PAD, area.y + PLATE_PAD, VITALS_W, block_h);
    let mut blocks = vec![first];
    if couch_pair {
        blocks.push(if beside {
            Rectangle::new(first.x + block_plate + CLUSTER_GAP, first.y, VITALS_W, block_h)
        } else {
            Rectangle::new(first.x, first.y + block_h + 2.0 * PLATE_PAD + CLUSTER_GAP, VITALS_W, block_h)
        });
    }
    let bottom = blocks.iter().map(|b| b.y + b.height + PLATE_PAD).fold(area.y, f32::max);
    let lines = Rectangle::new(area.x, bottom + ROW_GAP, block_plate, shape.lines as f32 * LINE_H);

    // The buttons from the row's right end - the mode slot, the pause slot,
    // the players slot, the online slot - each always in its own place,
    // whichever of them a round draws.
    let mode = Rectangle::new(buttons_row.x + buttons_row.width - MODE_BUTTON_W, buttons_row.y, MODE_BUTTON_W, button_h);
    let pause = Rectangle::new(mode.x - PLAYERS_BUTTON_GAP - PAUSE_BUTTON_W, mode.y, PAUSE_BUTTON_W, button_h);
    let players = Rectangle::new(pause.x - PLAYERS_BUTTON_GAP - PLAYERS_BUTTON_W, mode.y, PLAYERS_BUTTON_W, button_h);
    let online = Rectangle::new(players.x - PLAYERS_BUTTON_GAP - ONLINE_BUTTON_W, mode.y, ONLINE_BUTTON_W, button_h);

    // The minimap: under the right cluster's plate, its own plate flush
    // with the cluster's right edge.
    let left = blocks.iter().fold(lines, |r, block| union(r, Corners::plate(*block)));
    let minimap = shape.minimap.and_then(|(w, h)| {
        if !(w > 0.0 && h > 0.0) {
            return None;
        }
        let top = right.y + right.height + ROW_GAP + PLATE_PAD;
        let room = area.y + area.h - PLATE_PAD - top;
        let fit = (room / h).min(1.0);
        if fit < 1.0 && h * fit * scale < MINIMAP_MIN_PT {
            return None;
        }
        let (w, h) = (w * fit, h * fit);
        let rect = Rectangle::new(right_edge - w, top, w, h);
        let clear = rect.x - PLATE_PAD >= left.x + left.width + SIDE_GAP || rect.y - PLATE_PAD >= left.y + left.height + ROW_GAP;
        clear.then_some(rect)
    });
    let full = Corners {
        blocks,
        lines,
        row_h: button_h,
        lamp_rows: shape.lamp_row,
        right,
        info,
        level_button: shape.level_button.then(|| Rectangle::new(info.x, info.y, LEVEL_BUTTON_W, button_h)),
        online: shape.online.then_some(online),
        players: shape.players.then_some(players),
        restart: shape.restart.then_some(players),
        build: shape.build.then_some(mode),
        leave: shape.leave.then_some(mode),
        pause: shape.pause.then_some(pause),
        chips,
        minimap,
        lamp: shape.lamp_row.then(|| Rectangle::new(first.x, first.y + button_h + ROW_GAP, LAMP_BUTTON_W, button_h)),
        scale: 1.0,
        origin,
    };
    if scale < 1.0 {
        Corners { row_h: button_h * scale, scale, ..full.map(|r| about(r, origin, scale)) }
    } else {
        full
    }
}

/// The lamp row's button: the lantern, its count and its word.
pub const LAMP_BUTTON_W: f32 = 150.0;

/// The leave-round dialog's geometry, in UI points: the panel and its two
/// buttons (`LEAVE ROUND`, `KEEP PLAYING`), each at least 48 points tall
/// and 160 wide so a finger cannot miss.
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

/// Where `w` x `h` points stand centred in `area`, on whole points - a
/// dialog's, a panel's or the end screen's place in the chrome's area
/// (`UiFrame::area`), so it stays inside the safe area whatever the
/// window. A box larger than the area keeps to its top-left corner.
pub fn centred_in(area: Rect, w: f32, h: f32) -> Rectangle {
    let x = (area.x + ((area.w - w) / 2.0).max(0.0)).round();
    let y = (area.y + ((area.h - h) / 2.0).max(0.0)).round();
    Rectangle::new(x, y, w, h)
}

/// A `DIALOG_W` x `DIALOG_H` panel centred in `area` (UI points) with two
/// buttons on a row 16 points up from its bottom, 16 apart: (panel, left,
/// right). Both dialogs.
fn two_button_dialog(area: Rect) -> (Rectangle, Rectangle, Rectangle) {
    let panel = centred_in(area, DIALOG_W, DIALOG_H);
    let (x, y) = (panel.x, panel.y);
    let by = y + DIALOG_H - 16.0 - DIALOG_BUTTON_H;
    let gap = 16.0;
    let bx = x + (DIALOG_W - 2.0 * DIALOG_BUTTON_W - gap) / 2.0;
    (
        panel,
        Rectangle::new(bx, by, DIALOG_BUTTON_W, DIALOG_BUTTON_H),
        Rectangle::new(bx + DIALOG_BUTTON_W + gap, by, DIALOG_BUTTON_W, DIALOG_BUTTON_H),
    )
}

/// A question about a kept map (`mapstore::Question`): wider than the
/// other dialogs, its lines carrying the map's name.
pub const QUESTION_W: f32 = 600.0;

/// The map question's geometry (UI points): the panel and its two buttons,
/// the one the question is asking for on the left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuestionRects {
    pub panel: Rectangle,
    pub yes: Rectangle,
    pub no: Rectangle,
}

/// The map question centred in the chrome's `area` (UI points): the one
/// geometry its painter and every hit test read, in play and in the
/// builder alike.
pub fn question_rects(area: Rect) -> QuestionRects {
    let panel = centred_in(area, QUESTION_W, DIALOG_H);
    let by = panel.y + DIALOG_H - 16.0 - DIALOG_BUTTON_H;
    let gap = 16.0;
    let bx = panel.x + (QUESTION_W - 2.0 * DIALOG_BUTTON_W - gap) / 2.0;
    QuestionRects {
        panel,
        yes: Rectangle::new(bx, by, DIALOG_BUTTON_W, DIALOG_BUTTON_H),
        no: Rectangle::new(bx + DIALOG_BUTTON_W + gap, by, DIALOG_BUTTON_W, DIALOG_BUTTON_H),
    }
}

/// The leave dialog centred in the chrome's `area` (UI points): the one
/// geometry its painter and every hit test read.
pub fn leave_dialog_rects(area: Rect) -> LeaveDialogRects {
    let (panel, leave, stay) = two_button_dialog(area);
    LeaveDialogRects { panel, leave, stay }
}

/// The players dialog's geometry (UI points): the panel and its `1
/// PLAYER` / `2 PLAYERS` buttons, the leave dialog's shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayersDialogRects {
    pub panel: Rectangle,
    pub one: Rectangle,
    pub two: Rectangle,
}

/// The players dialog centred in the chrome's `area` (UI points).
pub fn players_dialog_rects(area: Rect) -> PlayersDialogRects {
    let (panel, one, two) = two_button_dialog(area);
    PlayersDialogRects { panel, one, two }
}

/// A level's opening lines (docs/levels.md): its number over the mission
/// banner and its title under it, half the banner's size.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelBanner {
    /// Counted from 1.
    pub number: usize,
    pub count: usize,
    /// In the language on screen (`levels::Level::title`).
    pub title: String,
}

/// What the end screen shows under the outcome (docs/levels.md): the
/// round's numbers, and on a level the buttons that stand where free play
/// counts down to its restart.
#[derive(Clone, Debug, PartialEq)]
pub struct ResultView {
    pub stats: RoundStats,
    /// Seats in the round: from two, the wrecks are split by seat.
    pub seats: usize,
    /// `None` in free play.
    pub buttons: Option<ResultButtons>,
}

/// A level's end-screen buttons: `PLAY AGAIN` always, and after a win the
/// way on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResultButtons {
    pub next: Option<NextLevel>,
    /// Whole seconds until the screen takes its way by itself
    /// (`Session::follow_countdown`), counted down in that way's own
    /// button - `NEXT LEVEL IN 3` after a win, `PLAY AGAIN IN 3` after a
    /// loss - so the press that skips the wait is the one the eye is
    /// already on. `None` after the last level's win, which waits for a
    /// button: going round to level 1 is the player's call.
    pub countdown: Option<u32>,
}

/// Where a won level's second button goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NextLevel {
    /// `NEXT LEVEL`.
    Next,
    /// The last of `levels` levels was won: the line says every level is
    /// complete and the button goes back to the first.
    FirstAgain { levels: usize },
}

/// The end screen's buttons, in UI points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResultRects {
    /// `LEVELS`, the level select, on the left.
    pub levels: Rectangle,
    pub again: Rectangle,
    /// Only after a win.
    pub next: Option<Rectangle>,
}

/// Where each line of the end screen goes, in UI points - the one
/// geometry the drawing and every hit test read, so a button that is not
/// drawn cannot be pressed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResultLayout {
    /// The outcome's top edge, at `RESULT_TITLE_SIZE`.
    pub title_y: f32,
    /// Every level complete, after the last level's win.
    pub all_clear_y: Option<f32>,
    /// Time and wrecks, at `RESULT_LINE_SIZE`.
    pub stats_y: f32,
    /// The wrecks by seat, from two seats, at `RESULT_SEATS_SIZE`.
    pub seats_y: Option<f32>,
    /// A level's buttons; free play has none.
    pub buttons: Option<ResultRects>,
    /// Free play's restart countdown, at `RESULT_LINE_SIZE`, in the
    /// buttons' place; a level counts down inside a button instead.
    pub countdown_y: Option<f32>,
}

pub const RESULT_TITLE_SIZE: i32 = 72;
pub const RESULT_LINE_SIZE: i32 = 28;
pub const RESULT_SEATS_SIZE: i32 = 20;
/// The gap between the end screen's two numbers, and between the seats'.
pub const RESULT_STATS_GAP: i32 = 40;
/// A level's number over the mission banner, and its title under it at
/// half the banner's size.
pub const LEVEL_NUMBER_SIZE: i32 = 28;
pub const LEVEL_TITLE_SIZE: i32 = 36;
/// The mission banner and the end screen's outcome, the `WAVE N` banner
/// and PAUSED: the sizes they are set in where the line has the room
/// (`banner_size`).
pub const BANNER_SIZE: i32 = 72;
pub const WAVE_BANNER_SIZE: i32 = 48;
/// The free-play end screen's countdown and the online one's.
pub const BANNER_SUB_SIZE: i32 = 28;
/// The least a banner shrinks to on a narrow window (`banner_size`):
/// `text_tests` holds every language's banners to it in the smallest area.
pub const BANNER_MIN_SIZE: i32 = 48;
/// The room one line of a banner or the end screen has in the smallest
/// area the chrome lays itself out in (`UI_MIN_W` less the edges) less 16
/// points a side - the budget `text_tests` measures every language
/// against.
pub const RESULT_TEXT_PX: i32 = (UI_MIN_W - 2.0 * UI_EDGE_PT) as i32 - 32;

/// The room a banner's line has in `area`: its width less 16 points a
/// side, and never less than `RESULT_TEXT_PX`, which every area holds.
pub fn banner_px(area: Rect) -> i32 {
    ((area.w - 32.0) as i32).max(RESULT_TEXT_PX)
}

/// The size `text` is set in as a banner of `size` in `area`: `size` where
/// the line has the room, else the largest that fits it (`text::fit_size`)
/// - a long mission banner in a narrow language on a phone - but never
/// under `BANNER_MIN_SIZE`.
pub fn banner_size(text: &str, size: i32, area: Rect) -> i32 {
    crate::text::fit_size(text, size, banner_px(area)).max(BANNER_MIN_SIZE.min(size))
}
pub const RESULT_BUTTON_W: f32 = 224.0;
pub const RESULT_BUTTON_H: f32 = 48.0;
pub const RESULT_BUTTON_GAP: f32 = 24.0;
/// `LEVELS` is the quieter way out, and narrower.
pub const RESULT_LEVELS_W: f32 = 160.0;

/// The end screen stacked and centred in the chrome's `area` (UI points):
/// the outcome, then whichever lines `view` carries, then the buttons or
/// the countdown. A level's buttons sit in one centred row - `LEVELS`,
/// `PLAY AGAIN`, and after a win the way on - so the way forward is always
/// on the right; free play's countdown stands where they would.
pub fn result_layout(area: Rect, view: &ResultView) -> ResultLayout {
    let line = RESULT_LINE_SIZE as f32;
    let all_clear = matches!(view.buttons, Some(ResultButtons { next: Some(NextLevel::FirstAgain { .. }), .. }));
    let rows = 16.0
        + if all_clear { line + 12.0 } else { 0.0 }
        + line
        + 10.0
        + if view.seats >= 2 { RESULT_SEATS_SIZE as f32 + 10.0 } else { 0.0 }
        + 14.0
        + if view.buttons.is_some() { RESULT_BUTTON_H } else { line };
    let top = centred_in(area, 0.0, RESULT_TITLE_SIZE as f32 + rows).y;
    let mut y = top + RESULT_TITLE_SIZE as f32 + 16.0;
    let all_clear_y = all_clear.then(|| {
        let at = y;
        y += line + 12.0;
        at
    });
    let stats_y = y;
    y += line + 10.0;
    let seats_y = (view.seats >= 2).then(|| {
        let at = y;
        y += RESULT_SEATS_SIZE as f32 + 10.0;
        at
    });
    y += 14.0;
    let countdown_y = view.buttons.is_none().then_some(y);
    let buttons = view.buttons.map(|b| {
        let (w, h, gap, levels_w) = (RESULT_BUTTON_W, RESULT_BUTTON_H, RESULT_BUTTON_GAP, RESULT_LEVELS_W);
        let row = levels_w + gap + w + if b.next.is_some() { gap + w } else { 0.0 };
        let x = centred_in(area, row, h).x;
        let again = Rectangle::new(x + levels_w + gap, y, w, h);
        ResultRects {
            levels: Rectangle::new(x, y, levels_w, h),
            again,
            next: b.next.map(|_| Rectangle::new(again.x + w + gap, y, w, h)),
        }
    });
    ResultLayout { title_y: top, all_clear_y, stats_y, seats_y, buttons, countdown_y }
}

/// A round's length as the end screen writes it: `m:ss`, whole seconds.
pub fn clock_text(seconds: f32) -> String {
    let whole = seconds.max(0.0) as u32;
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// What chrome `Game::render` draws around the world: the corner clusters
/// and the buttons in them, and, while the player is being asked, a dialog,
/// the end screen, the lobby or the level select over a dim.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayChrome {
    /// The corner clusters (`corners`): play mode and an online round.
    /// Nothing else - the lobby, a demo - draws them.
    pub hud: bool,
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
    /// The pause button left of the mode button: a local round's, since a
    /// room's round is not this window's to stop.
    pub pause_button: bool,
    /// The local round is paused: the pause button shows the play
    /// triangle that takes it on again.
    pub paused: bool,
    /// The seat this window is playing in a room, whose block the left
    /// cluster shows. `None` in a couch round, where the first block is
    /// player 1's and the couch layouts apply.
    pub seat: Option<u8>,
    pub leave_dialog: bool,
    pub players_dialog: bool,
    /// A question about a kept map, over everything but the level select
    /// (`mode::Session::question`).
    pub question: Option<crate::mapstore::Question>,
    /// One line under the left cluster while an online round runs: the
    /// room code, this seat and the snapshot buffer
    /// (`net::round::OnlineRound::status`). `None` in a local round -
    /// and everything before the round is the lobby's, not this line's.
    pub status: Option<String>,
    /// The lobby over a dimmed field (`lobby.rs`), in place of the round
    /// this window is not playing.
    pub lobby: Option<crate::lobby::LobbyView>,
    /// The message the end screen counts down with. `None` is the local
    /// round's `ROUND_RESTARTING`, which is what a local round does; an
    /// online round's counts down to the room's lobby instead.
    pub countdown_label: Option<crate::text::Key>,
    /// The level on the field, for the opening banner's lines (`None` in
    /// free play and online).
    pub level: Option<LevelBanner>,
    /// The end screen's numbers and a level's buttons, once a local
    /// round is decided.
    pub result: Option<ResultView>,
    /// The level button's number (`Session::level_button`), drawn in the
    /// mission word's place on a level.
    pub level_button: Option<usize>,
    /// The level select over a dimmed field (`level_select.rs`).
    pub levels: Option<crate::level_select::LevelSelectView>,
    /// The minimap under the right cluster, its size in points
    /// (`minimap::MinimapRules::size_pt`): a round on a screen that shows
    /// one (not a phone's, `minimap_show`) whose view shows less than the
    /// whole field (`Session::minimap_on`).
    pub minimap: Option<(f32, f32)>,
    /// The lamp row under each block: a round that gives lanterns or has
    /// lava to cross (docs/volcano.md).
    pub lamp_row: bool,
    /// How dark the fade through black between rounds is, 0 to 1
    /// (`Session::curtain`), drawn over everything else.
    pub curtain: f32,
}

/// The online status line's text size: the first line under the left
/// cluster (`Corners::lines`).
pub const HUD_STATUS_TEXT_SIZE: i32 = 14;

/// The status line's colour: the room blue, so a round somebody else is
/// simulating never reads as one of the HUD's own numbers.
pub const HUD_STATUS_COLOR: Color = ONLINE_COLOR;

/// How far each corner cluster has faded (`ui_fade_opacity` up to 1): a
/// cluster drops while the fight is under it, so the HUD never hides a
/// tank, a shot or a blast, and comes back once it has passed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fade {
    pub left: f32,
    pub right: f32,
}

impl Default for Fade {
    fn default() -> Self {
        Fade { left: 1.0, right: 1.0 }
    }
}

impl Fade {
    /// One frame of `dt` seconds: each cluster moves toward `opacity`
    /// while `covered` says something is under it, else back toward 1, at
    /// the pace that goes the whole way in `seconds` (0 snaps).
    pub fn step(&mut self, covered: [bool; 2], dt: f32, opacity: f32, seconds: f32) {
        let opacity = opacity.clamp(0.0, 1.0);
        let pace = if seconds > 0.0 { (1.0 - opacity).max(1e-3) * dt.max(0.0) / seconds } else { f32::INFINITY };
        for (alpha, under) in [&mut self.left, &mut self.right].into_iter().zip(covered) {
            let to = if under { opacity } else { 1.0 };
            *alpha = if *alpha < to { (*alpha + pace).min(to) } else { (*alpha - pace).max(to) };
        }
    }
}

/// What a corner fades for: every tank standing, every shot in flight and
/// every blast still burning, as the world point it is at and the world
/// pixels round it that it covers.
pub fn action_marks(game: &Game) -> Vec<(crate::Position, f32)> {
    let mut marks = Vec::new();
    for tank in game.world.query::<&Tank>().iter().filter(|t| !t.is_wreck()) {
        marks.push((tank.position, tank.size() * 0.5));
    }
    for shell in game.world.query::<&crate::shell::Shell>().iter() {
        marks.push((shell.position, 6.0));
    }
    for bullet in game.world.query::<&crate::bullet::Bullet>().iter() {
        marks.push((bullet.position, 4.0));
    }
    for plasma in game.world.query::<&crate::plasma::Plasma>().iter() {
        marks.push((plasma.position, 10.0));
    }
    for missile in game.world.query::<&crate::missile::Missile>().iter() {
        marks.push((missile.position, 10.0));
    }
    let fireball = tuning().blast_fireball_px;
    for blast in game.blast_fx.iter().filter(|b| !crate::fireball::done(b)) {
        marks.push((blast.center, fireball * blast.scale));
    }
    marks
}

/// Where the world is drawn on the window, as the chrome measures it: a
/// world point through the camera onto the bitmap's field area, the view
/// onto the window and the UI scale into UI points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldOnScreen {
    pub camera: crate::view::Camera,
    /// Where the field area's corner is in the bitmap (`Layout::field`).
    pub field_origin: Vec2,
    pub view: crate::view::View,
    pub ui_scale: f32,
}

impl WorldOnScreen {
    /// A world point in UI points.
    pub fn to_ui(&self, world: crate::Position) -> Vec2 {
        let p = self.camera.to_view(world);
        let window = self.view.to_window(Vec2::new(self.field_origin.x + p.x, self.field_origin.y + p.y));
        Vec2::new(window.x / self.ui_scale, window.y / self.ui_scale)
    }

    /// A world length in UI points.
    pub fn len_to_ui(&self, world: f32) -> f32 {
        world * self.camera.scale * self.view.scale / self.ui_scale
    }
}

/// Which of the two clusters (left, right) has something of `marks` under
/// it - the right one's minimap under it counting as the cluster: a mark
/// is a world point and the world pixels round it it covers, drawn where
/// `screen` puts it. Only what is drawn counts - a mark at least partly on
/// the field and in the camera's view - so a tank rolling in through a
/// gate, outside the field, fades nothing from the letterbox it would
/// project onto.
pub fn covered(corners: &Corners, marks: &[(crate::Position, f32)], screen: &WorldOnScreen) -> [bool; 2] {
    let shown = screen.camera.rect();
    let (field_w, field_h) = screen.camera.field;
    let (x0, y0) = (shown.x.max(0.0), shown.y.max(0.0));
    let (x1, y1) = ((shown.x + shown.width).min(field_w), (shown.y + shown.height).min(field_h));
    let drawn = |at: crate::Position, radius: f32| at.x + radius > x0 && at.x - radius < x1 && at.y + radius > y0 && at.y - radius < y1;
    let under = |r: Rectangle| {
        marks.iter().filter(|&&(at, radius)| drawn(at, radius)).any(|&(at, radius)| {
            let p = screen.to_ui(at);
            let reach = screen.len_to_ui(radius);
            let nx = p.x.clamp(r.x, r.x + r.width);
            let ny = p.y.clamp(r.y, r.y + r.height);
            (p.x - nx).powi(2) + (p.y - ny).powi(2) <= reach * reach
        })
    };
    [under(corners.left()), under(corners.right) || corners.minimap_plate().is_some_and(under)]
}

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

    /// The hints follow the input last used, whatever the build: a touch
    /// landing turns them to taps, a key press back to the keys, a frame
    /// with neither leaves them, and a key beats a touch in the same frame.
    /// A window with no keyboard - or the mouse standing in for a finger -
    /// opens on taps, any other on the keys; a frame laid out for touch
    /// opens on taps until it is told otherwise, and its buttons stay a
    /// finger's size whatever the hints say.
    #[test]
    fn the_hints_follow_the_input_last_used() {
        use crate::text::keys;
        assert_eq!(Hints::at_start(true), Hints::Touch);
        assert_eq!(Hints::at_start(false), Hints::Keys);
        let mut hints = Hints::at_start(false);
        let frames = [
            ((false, false), Hints::Keys),
            ((true, false), Hints::Touch),
            ((false, false), Hints::Touch),
            ((false, true), Hints::Keys),
            ((false, false), Hints::Keys),
            ((true, true), Hints::Keys),
            ((true, false), Hints::Touch),
        ];
        for ((touch, key), want) in frames {
            hints = hints.follow(touch, key);
            assert_eq!(hints, want, "touch={touch} key={key}");
        }
        assert_eq!(Hints::Keys.pick(keys::EDITOR_PAGE, keys::EDITOR_PAGE_TOUCH), keys::EDITOR_PAGE);
        assert_eq!(Hints::Touch.pick(keys::EDITOR_PAGE, keys::EDITOR_PAGE_TOUCH), keys::EDITOR_PAGE_TOUCH);
        let touch = UiFrame::new((852.0, 393.0), 1.0, 1.0, Insets::default(), true);
        assert_eq!(touch.hints, Hints::Touch);
        assert_eq!(UiFrame::plain((1088.0, 576.0)).hints, Hints::Keys);
        let keyed = touch.with_hints(Hints::Keys);
        assert!(keyed.touch && keyed.hints == Hints::Keys, "a key press changes the words, not the buttons");
        assert_eq!((keyed.scale, keyed.screen, keyed.area), (touch.scale, touch.screen, touch.area));
    }

    /// One seat and two on a couch have a table each; everything else - a
    /// third couch seat, and every room of two or more - is the compact
    /// one.
    #[test]
    fn the_couch_keeps_its_tables_and_a_room_is_compact_from_two() {
        assert_eq!(HudLayout::choose(1, None), HudLayout::One);
        assert_eq!(HudLayout::choose(2, None), HudLayout::Two);
        for seats in 3..=MAX_SEATS {
            assert_eq!(HudLayout::choose(seats, None), HudLayout::Compact, "{seats} on a couch");
        }
        // A room: the seat this window plays is rarely seat 0, so even a
        // pair goes compact rather than pairing the blocks up.
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

    /// The vitals show what the trigger fires and nothing else: shells
    /// against the magazine while no special is carried, the special in
    /// its accent against a crate's worth once one is - the same readout
    /// the pips under the ring are drawn from.
    #[test]
    fn the_trigger_readout_is_the_special_carried_else_shells() {
        let t = tuning();
        let mut tank = Tank { shells_ammo: 7, ..Tank::default() };
        let shells = WeaponSlot::of(&tank);
        assert_eq!((shells.weapon, shells.count, shells.full), (ActiveWeapon::Shell, 7, t.max_shells));
        assert_eq!(shells.color, hud_number_color(7.0, t.max_shells as f32));
        tank.take_weapon(ActiveWeapon::Laser);
        tank.laser_charges -= 2;
        let laser = WeaponSlot::of(&tank);
        assert_eq!((laser.weapon, laser.count, laser.full), (ActiveWeapon::Laser, t.laser_charges_per_pickup - 2, t.laser_charges_per_pickup));
        assert_eq!(laser.color, HUD_LASER_COLOR, "no shells while a special is carried");
        tank.laser_charges = 0;
        assert_eq!(WeaponSlot::of(&tank).count, 7, "spent, back to the shells");
        tank.take_weapon(ActiveWeapon::SonicHammer);
        let sonic = WeaponSlot::of(&tank);
        assert_eq!((sonic.weapon, sonic.count, sonic.full), (ActiveWeapon::SonicHammer, t.sonic_ammo_per_pickup, t.sonic_ammo_per_pickup));
        assert_eq!(sonic.color, HUD_SONIC_COLOR);
        assert_eq!(weapon_pickup(ActiveWeapon::SonicHammer), Some(crate::pickup::PickupKind::SonicHammer), "its crate's symbol");
    }

    /// The block is the local seat's, whichever seat that is, and the
    /// chips are all the others - so a four-seat room at seat 2 reads its
    /// own ammo in the block and seats 1, 2 and 4 in the strip.
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
            assert_eq!(model.local.weapon.count, 3 + seat as i32, "seat {seat}'s own block");
            assert_eq!(model.second, None, "the compact layout pairs nothing");
            let listed: Vec<u8> = model.others.iter().map(|s| s.seat).collect();
            assert_eq!(listed, (0..4u8).filter(|&i| i != seat).collect::<Vec<_>>());
            assert!(model.others.iter().all(|s| s.alive && s.health > 0.0));
        }
        // A couch round of four is the same layout, read from seat 0.
        let couch = HudModel::gather(&game, None);
        assert_eq!(couch.layout, HudLayout::Compact);
        assert_eq!(couch.local.weapon.count, 3);
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

    /// The screens the corners are laid out on: phones (an iPhone 15 with
    /// its Dynamic Island and home indicator inset, an iPhone SE, a Pixel
    /// in pixels at 2.625), an iPad, desktop windows from the smallest the
    /// chrome is laid out for up to 1080p, and the arena's own bitmap.
    fn screens() -> Vec<(&'static str, UiFrame)> {
        let island = Insets { left: 59.0, top: 0.0, right: 59.0, bottom: 21.0 };
        vec![
            ("iphone 15", UiFrame::new((852.0, 393.0), 1.0, 1.0, island, true)),
            ("iphone se", UiFrame::new((667.0, 375.0), 1.0, 1.0, Insets::default(), true)),
            ("pixel", UiFrame::new((2400.0, 1080.0), 2.625, 1.0, Insets::default(), true)),
            ("ipad", UiFrame::new((1180.0, 820.0), 1.0, 1.0, Insets::default(), true)),
            ("smallest", UiFrame::new((UI_MIN_W, UI_MIN_H), 1.0, 1.0, Insets::default(), false)),
            ("bitmap", UiFrame::plain((W, H + 32.0))),
            ("1600", UiFrame::plain((1600.0, 900.0))),
            ("1080p", UiFrame::plain((1920.0, 1080.0))),
            ("1080p touch", UiFrame::new((1920.0, 1080.0), 1.0, 1.0, Insets::default(), true)),
        ]
    }

    /// Every shape of round the corners hold: one seat on a level with
    /// every button, free play, a keyboard-less build's RESTART, a couch
    /// pair, a couch of four, and a full room seen from seat 3 - each also
    /// with a minimap of a wide field (160 x 90 pt), the box's height for
    /// a tall one (60 x 120) and a long thin one's (160 x 12).
    fn shapes() -> Vec<(&'static str, CornerShape)> {
        let plain = plain_shapes();
        let mut all = plain.clone();
        for (map, size) in [("wide map", (160.0, 90.0)), ("tall map", (60.0, 120.0)), ("long map", (160.0, 12.0))] {
            for (name, shape) in &plain {
                let name: &'static str = Box::leak(format!("{name} with a {map}").into_boxed_str());
                all.push((name, CornerShape { minimap: Some(size), ..*shape }));
            }
        }
        all
    }

    /// The shapes with no minimap.
    fn plain_shapes() -> Vec<(&'static str, CornerShape)> {
        let play = CornerShape {
            layout: HudLayout::One,
            chips: 0,
            level_button: true,
            online: true,
            players: true,
            restart: false,
            build: true,
            leave: false,
            pause: true,
            lines: 1,
            minimap: None,
            lamp_row: false,
        };
        vec![
            ("one", play),
            ("lamps", CornerShape { lamp_row: true, ..play }),
            ("couch pair with lamps", CornerShape { layout: HudLayout::Two, lamp_row: true, ..play }),
            ("free play", CornerShape { level_button: false, ..play }),
            ("phone", CornerShape { players: false, restart: true, ..play }),
            ("couch pair", CornerShape { layout: HudLayout::Two, ..play }),
            ("couch of four", CornerShape { layout: HudLayout::Compact, chips: 3, ..play }),
            (
                "full room",
                CornerShape {
                    layout: HudLayout::Compact,
                    chips: MAX_SEATS - 1,
                    level_button: false,
                    online: false,
                    players: false,
                    build: false,
                    leave: true,
                    pause: false,
                    lines: 2,
                    ..play
                },
            ),
        ]
    }

    fn inside(r: Rectangle, outer: Rect) -> bool {
        r.x >= outer.x - 1e-3 && r.y >= outer.y - 1e-3 && r.x + r.width <= outer.x + outer.w + 1e-3 && r.y + r.height <= outer.y + outer.h + 1e-3
    }

    fn within(r: Rectangle, outer: Rectangle) -> bool {
        inside(r, Rect::new(outer.x, outer.y, outer.width, outer.height))
    }

    fn apart(a: Rectangle, b: Rectangle) -> bool {
        a.x + a.width <= b.x + 1e-3 || b.x + b.width <= a.x + 1e-3 || a.y + a.height <= b.y + 1e-3 || b.y + b.height <= a.y + 1e-3
    }

    /// On every screen and in every shape the two clusters stay inside the
    /// safe area and apart, every button inside the right one, none on
    /// another or on the first row's readouts, every chip inside it too,
    /// and on a touch screen every button a finger's 44 pt both ways at
    /// the corners' scale.
    #[test]
    fn the_corners_fit_every_screen_inside_its_safe_area() {
        for (screen, ui) in screens() {
            for (shape_name, shape) in shapes() {
                let what = format!("{shape_name} on {screen}");
                let c = corners(&ui, &shape);
                let (left, right) = (c.left(), c.right);
                assert!(inside(left, ui.area) && inside(right, ui.area), "{what}: a cluster leaves the safe area: {left:?} {right:?} in {:?}", ui.area);
                assert!(apart(left, right), "{what}: the clusters meet: {left:?} {right:?}");
                assert!(right.x - (left.x + left.width) >= SIDE_GAP * c.scale - 1e-3 || left.y + left.height <= right.y, "{what}: too close");
                for (i, a) in c.blocks.iter().enumerate() {
                    assert!(within(c.block_plate(*a), left), "{what}");
                    for b in &c.blocks[i + 1..] {
                        assert!(apart(c.block_plate(*a), c.block_plate(*b)), "{what}: two blocks overlap");
                    }
                }
                assert!(within(c.info, right), "{what}: the first row leaves its plate");
                let buttons = c.buttons();
                for (i, (button, r)) in buttons.iter().enumerate() {
                    // The lamp row is the local block's, in the left cluster.
                    let plate = if *button == CornerButton::Lamp { c.block_plate(c.blocks[0]) } else { right };
                    assert!(within(*r, plate), "{what}: {button:?} leaves the plate");
                    if *button != CornerButton::Level {
                        assert!(apart(*r, c.info), "{what}: {button:?} sits on the first row");
                    }
                    if ui.touch {
                        let finger = UI_TOUCH_PT * c.scale - 1e-3;
                        assert!(r.width >= finger && r.height >= finger, "{what}: {button:?} is {r:?}, under a finger at {}", c.scale);
                    }
                    for (other, o) in &buttons[i + 1..] {
                        assert!(apart(*r, *o), "{what}: {button:?} overlaps {other:?}");
                    }
                }
                for i in 0..shape.chips {
                    let chip = c.chip(i).expect("a strip");
                    assert!(within(chip, right), "{what}: chip {i} leaves the plate");
                    assert!(buttons.iter().all(|(_, r)| apart(*r, chip)), "{what}: chip {i} sits on a button");
                }
                assert_eq!(c.chips.is_some(), shape.chips > 0);
                // The minimap: on its own plate inside the safe area, under
                // the right cluster and flush with its right edge, clear of
                // the left cluster, the size it was asked for, and one of
                // the places a touch or an arrow keeps out of.
                assert_eq!(c.minimap.is_some(), shape.minimap.is_some(), "{what}: every screen here has the room");
                if let (Some(map), Some((w, h))) = (c.minimap, shape.minimap) {
                    let plate = c.minimap_plate().expect("a plate");
                    assert!(inside(plate, ui.area), "{what}: the minimap leaves the safe area: {plate:?} in {:?}", ui.area);
                    assert!(apart(plate, left) && apart(plate, right), "{what}: the minimap meets a cluster: {plate:?}");
                    assert!(plate.y >= right.y + right.height + ROW_GAP * c.scale - 1e-3, "{what}: under the right cluster");
                    assert!((plate.x + plate.width - (right.x + right.width)).abs() < 1e-3, "{what}: flush with its right edge");
                    assert!((map.width - w * c.scale).abs() < 1e-3 && (map.height - h * c.scale).abs() < 1e-3, "{what}: {map:?}");
                    assert!(c.buttons().iter().all(|(_, r)| apart(*r, plate)), "{what}: on a button");
                    assert!(c.keep_out().contains(&plate), "{what}: a keep-out");
                    assert_eq!(c.hit(Vec2::new(map.x + map.width / 2.0, map.y + map.height / 2.0)), None, "{what}: not a button");
                }
            }
        }
    }

    /// The minimap stays inside the safe area at every window size, on
    /// every screen a touch or a mouse lays the chrome out for, in every
    /// shape: shrunk to the room under the right cluster on a short window
    /// and left out where that would take it under `MINIMAP_MIN_PT`, never
    /// on the left cluster, and fading with the right cluster - a tank
    /// under it fades that cluster.
    #[test]
    fn the_minimap_slot_fits_every_window_size() {
        let island = Insets { left: 59.0, top: 0.0, right: 59.0, bottom: 21.0 };
        for w in (640..=2560).step_by(48) {
            for h in (300..=1440).step_by(36) {
                for (insets, touch) in [(Insets::default(), false), (island, true)] {
                    let ui = UiFrame::new((w as f32, h as f32), 1.0, 1.0, insets, touch);
                    for (shape_name, shape) in shapes().into_iter().filter(|(_, s)| s.minimap.is_some()) {
                        let what = format!("{shape_name} in {w}x{h} (touch {touch})");
                        let c = corners(&ui, &shape);
                        let Some(map) = c.minimap else { continue };
                        let plate = c.minimap_plate().expect("a plate");
                        assert!(inside(plate, ui.area), "{what}: {plate:?} leaves {:?}", ui.area);
                        assert!(apart(plate, c.left()) && apart(plate, c.right), "{what}: {plate:?}");
                        let (_, want_h) = shape.minimap.expect("asked");
                        assert!(map.height >= MINIMAP_MIN_PT.min(want_h * c.scale) - 1e-3, "{what}: shrunk too far: {map:?}");
                        assert!(map.height <= want_h * c.scale + 1e-3);
                    }
                }
            }
        }
        // An area too short for it leaves it out, and one a little short
        // shrinks it. (`UiFrame::new` never lays the chrome out in an area
        // under `UI_MIN_H` less its edges, which holds it whole.)
        let short = |h: f32| UiFrame { scale: 1.0, screen: Rect::new(0.0, 0.0, 1600.0, h), area: Rect::new(8.0, 8.0, 1584.0, h - 16.0), touch: false, hints: Hints::Keys };
        let wide = shapes().into_iter().find(|(n, _)| *n == "one with a wide map").expect("the shape").1;
        assert_eq!(corners(&short(90.0), &wide).minimap, None);
        let shrunk = corners(&short(140.0), &wide).minimap.expect("room for a smaller one");
        assert!(shrunk.height < 90.0 && shrunk.height >= MINIMAP_MIN_PT, "{shrunk:?}");
        assert!((shrunk.width / shrunk.height - 160.0 / 90.0).abs() < 1e-3, "the map's shape kept");
        // A tank under the minimap fades the right cluster.
        let ui = UiFrame::plain((1600.0, 900.0));
        let c = corners(&ui, &wide);
        let map = c.minimap.expect("room for it");
        let view = crate::view::View::fit((1600.0, 900.0), (1600.0, 900.0));
        let screen = WorldOnScreen { camera: crate::view::Camera::whole((1600.0, 900.0)), field_origin: Vec2::new(0.0, 0.0), view, ui_scale: ui.scale };
        let under = crate::Position::new(map.x + map.width / 2.0, map.y + map.height / 2.0);
        assert_eq!(covered(&c, &[(under, 8.0)], &screen), [false, true]);
        let bare = corners(&ui, &CornerShape { minimap: None, ..wide });
        assert_eq!(covered(&bare, &[(under, 8.0)], &screen), [false, false], "nothing there without one");
    }

    /// A touch that lands on the minimap is the HUD's: it neither steers
    /// nor fires, held or not, as the clusters' own keep-outs do
    /// (`TouchScheme::set_keep_out`).
    #[test]
    fn the_minimap_claims_presses() {
        use crate::touch::{StickRule, TouchPoint, TouchScheme};
        let ui = UiFrame::new((1180.0, 820.0), 1.0, 1.0, Insets::default(), true);
        let shape = shapes().into_iter().find(|(n, _)| *n == "phone with a wide map").expect("the shape").1;
        let c = corners(&ui, &shape);
        let map = c.minimap.expect("a minimap");
        // The touch scheme takes the whole window in UI points, as play's does.
        let area = ui.screen;
        let rule = StickRule { dead_zone_pt: 8.0, follow_radius_pt: 48.0, axis_switch_deg: 30.0 };
        let mut t = TouchScheme::default();
        t.set_keep_out(&c.keep_out());
        let on = TouchPoint { id: 7, pos: Vec2::new(map.x + map.width / 2.0, map.y + map.height / 2.0) };
        for _ in 0..3 {
            let intent = t.update_with(&[on], area, false, 1.0 / 60.0, &rule);
            assert_eq!((intent.move_dir, intent.fire), (None, false), "a touch on the minimap does nothing");
        }
        // Dragged off it, still nobody's until it lifts.
        let dragged = TouchPoint { id: 7, pos: Vec2::new(300.0, 600.0) };
        assert!(!t.update_with(&[dragged], area, false, 1.0 / 60.0, &rule).fire);
        // The same place with no minimap fires.
        let mut bare = TouchScheme::default();
        bare.set_keep_out(&corners(&ui, &CornerShape { minimap: None, ..shape }).keep_out());
        assert!(bare.update_with(&[on], area, false, 1.0 / 60.0, &rule).fire, "the fire half, uncovered");
    }

    /// What `PlayChrome` says is drawn is what the corners hold: play's
    /// buttons and the level button on a level, an online round's LEAVE
    /// alone and its seat chips, and nothing at all where no HUD is drawn.
    #[test]
    fn the_corners_hold_the_buttons_the_chrome_draws() {
        let ui = UiFrame::plain((1600.0, 900.0));
        let play = PlayChrome {
            hud: true,
            build_button: true,
            players_button: true,
            online_button: true,
            pause_button: true,
            level_button: Some(3),
            ..PlayChrome::default()
        };
        let c = corners(&ui, &CornerShape::of(&play, 1).expect("play draws the corners"));
        let names: Vec<CornerButton> = c.buttons().into_iter().map(|(b, _)| b).collect();
        assert_eq!(names, vec![CornerButton::Level, CornerButton::Online, CornerButton::Players, CornerButton::Build, CornerButton::Pause]);
        // The mode slot is at the right end, the pause button left of it,
        // then the players' and ONLINE's, each in its own place.
        let (online, players, pause, build) = (c.online.unwrap(), c.players.unwrap(), c.pause.unwrap(), c.build.unwrap());
        assert!(online.x + online.width + PLAYERS_BUTTON_GAP <= players.x && players.x + players.width + PLAYERS_BUTTON_GAP <= pause.x);
        assert!(pause.x + pause.width + PLAYERS_BUTTON_GAP <= build.x);
        assert!(pause.width >= UI_TOUCH_PT, "an icon a finger can hit");
        assert!((build.x + build.width - (ui.area.x + ui.area.w - PLATE_PAD)).abs() < 1e-3, "BUILD at the right end");
        // Free play: the mission word, no level button.
        let free = PlayChrome { level_button: None, ..play.clone() };
        assert_eq!(corners(&ui, &CornerShape::of(&free, 1).unwrap()).level_button, None);
        // An online round of three at seat 1: LEAVE in the mode slot, two
        // chips, the status line over the build stamp.
        let online_round = PlayChrome { hud: true, leave_button: true, seat: Some(1), status: Some("ROOM".into()), ..PlayChrome::default() };
        let shape = CornerShape::of(&online_round, 3).unwrap();
        assert_eq!((shape.layout, shape.chips, shape.lines), (HudLayout::Compact, 2, 2));
        let c = corners(&ui, &shape);
        assert_eq!(c.buttons().into_iter().map(|(b, _)| b).collect::<Vec<_>>(), vec![CornerButton::Leave]);
        assert_eq!(c.leave, Some(build), "LEAVE takes the mode button's slot");
        // The lobby and the demos draw none.
        assert_eq!(CornerShape::of(&PlayChrome::default(), 1), None);
        // A press lands on the button under it and nowhere else.
        let centre = |r: Rectangle| Vec2::new(r.x + r.width / 2.0, r.y + r.height / 2.0);
        assert_eq!(c.hit(centre(build)), Some(CornerButton::Leave));
        assert_eq!(c.hit(Vec2::new(ui.screen.w / 2.0, ui.screen.h / 2.0)), None);
    }

    /// The two corners read along one line on every screen and in every
    /// shape: every plate along the top - each block's and the right
    /// cluster's - starts at one top and is one height, the first rows are
    /// one height - a finger's on a touch screen, where the level button
    /// grows to one, at the corners' scale - and centred on one line, and
    /// the right cluster is that one row whole: its numbers, a room's chips
    /// and every button.
    #[test]
    fn the_two_corners_read_along_one_line() {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        for (screen, ui) in screens() {
            for (shape_name, shape) in shapes() {
                let what = format!("{shape_name} on {screen}");
                let c = corners(&ui, &shape);
                assert!(near(c.row_h, c.info.height), "{what}: the first rows differ in height");
                assert!(near(c.row_h, button_height(ui.touch) * c.scale), "{what}");
                let first = c.blocks[0];
                assert!(near(first.y, c.info.y), "{what}: the first rows start apart");
                for block in &c.blocks {
                    let plate = c.block_plate(*block);
                    assert!(near(plate.height, c.right.height), "{what}: a block's plate and the right cluster's differ in height");
                    if block.y == first.y {
                        assert!(near(plate.y, c.right.y), "{what}: the plates' tops differ");
                    }
                }
                let centre = |r: Rectangle| r.y + r.height / 2.0;
                for (button, r) in c.buttons().into_iter().filter(|(b, _)| *b != CornerButton::Lamp) {
                    assert!(near(centre(r), centre(c.info)), "{what}: {button:?} is off the row");
                }
                if let Some(chips) = c.chips {
                    assert!(near(centre(chips), centre(c.info)), "{what}: the chips are off the row");
                }
                assert!(near(first.y + c.row_h / 2.0, centre(c.info)), "{what}: the rows' centre lines differ");
                assert_eq!(c.lamp.is_some(), shape.lamp_row, "{what}");
                if let Some(lamp) = c.lamp {
                    let row = c.lamp_row(first).expect("a lamp row");
                    assert!(within(row, first), "{what}: the lamp row leaves its block");
                    assert!(near(lamp.y, row.y) && near(lamp.height, row.height), "{what}: the lamp button is not on its row");
                } else {
                    assert_eq!(c.lamp_row(first), None, "{what}");
                }
            }
        }
    }

    /// The corners are drawn at full size wherever their one row fits the
    /// area, and where it does not, smaller by the one factor that makes it
    /// reach exactly across: the vitals at the area's left edge, the right
    /// cluster at its right, the scaled gap between them.
    #[test]
    fn the_corners_shrink_only_to_fit_the_row() {
        for (screen, ui) in screens() {
            for (shape_name, shape) in shapes() {
                let what = format!("{shape_name} on {screen}");
                let c = corners(&ui, &shape);
                assert!(c.scale > 0.0 && c.scale <= 1.0, "{what}: {}", c.scale);
                if c.scale < 1.0 {
                    // Drawn smaller only by as much as the row needs: it
                    // reaches from edge to edge with the gap between the
                    // clusters no wider than the scaled one.
                    let left = c.left();
                    assert!((left.x - ui.area.x).abs() < 1e-2, "{what}: {left:?}");
                    assert!((c.right.x + c.right.width - (ui.area.x + ui.area.w)).abs() < 1e-2, "{what}: the row does not reach across");
                    let gap = c.right.x - (left.x + left.width);
                    assert!((gap - SIDE_GAP * c.scale).abs() < 1e-2, "{what}: shrunk further than the row needs ({gap} apart)");
                }
                // What the painter draws is the same corners at full size.
                let unscaled = c.unscaled();
                assert_eq!(unscaled.scale, 1.0);
                assert!((unscaled.row_h - button_height(ui.touch)).abs() < 1e-3, "{what}");
            }
        }
    }

    /// A couch pair's two blocks sit side by side where the window has the
    /// room for both and the right cluster, and player 2's under player
    /// 1's where it has not; the right cluster is one row on every window,
    /// drawn smaller on a phone's.
    #[test]
    fn the_corners_arrange_themselves_by_the_windows_width() {
        let pair = shapes().into_iter().find(|(n, _)| *n == "couch pair").unwrap().1;
        let wide = corners(&UiFrame::plain((1600.0, 900.0)), &pair);
        assert_eq!(wide.blocks[0].y, wide.blocks[1].y, "beside on 1600");
        assert!(wide.blocks[1].x > wide.blocks[0].x + VITALS_W);
        let narrow = corners(&UiFrame::plain((900.0, 600.0)), &pair);
        assert_eq!(narrow.blocks[0].x, narrow.blocks[1].x, "under on 900");
        assert!(narrow.blocks[1].y >= narrow.blocks[0].y + narrow.row_h + 2.0 * PLATE_PAD);
        let one = shapes()[0].1;
        let desk = corners(&UiFrame::plain((1600.0, 900.0)), &one);
        assert_eq!(desk.info.y, desk.build.unwrap().y, "one row on a monitor");
        assert_eq!(desk.scale, 1.0, "full size on a monitor");
        let phone = corners(&UiFrame::new((852.0, 393.0), 1.0, 1.0, Insets { left: 59.0, top: 0.0, right: 59.0, bottom: 21.0 }, true), &one);
        assert_eq!(phone.info.y, phone.build.unwrap().y, "one row on a phone");
        assert!(phone.scale < 1.0, "drawn smaller on a phone");
        assert!((phone.info.height - UI_TOUCH_PT * phone.scale).abs() < 1e-3, "the level button a finger's at the corners' scale");
    }

    /// A cluster fades to `ui_fade_opacity` while something is under it,
    /// at the pace that goes the whole way in `ui_fade_seconds`, and comes
    /// back as fast; 0 seconds snaps.
    #[test]
    fn a_cluster_fades_under_the_fight_and_comes_back() {
        let mut f = Fade::default();
        f.step([true, false], 0.125, 0.35, 0.25);
        assert!((f.left - 0.675).abs() < 1e-4 && f.right == 1.0, "{f:?}");
        f.step([true, false], 0.25, 0.35, 0.25);
        assert_eq!(f.left, 0.35, "held at the floor");
        f.step([false, true], 0.1, 0.35, 0.25);
        assert!((f.left - 0.61).abs() < 1e-4 && (f.right - 0.74).abs() < 1e-4, "{f:?}");
        f.step([false, false], 1.0, 0.35, 0.25);
        assert_eq!(f, Fade::default());
        f.step([true, true], 0.0, 0.35, 0.0);
        assert_eq!((f.left, f.right), (0.35, 0.35), "0 seconds snaps");
    }

    /// What a cluster fades for is found where the frame draws it: a
    /// tank's world point through the camera, the view and the UI scale,
    /// with its own footprint round it.
    #[test]
    fn a_tank_under_a_cluster_is_found_where_it_is_drawn() {
        let ui = UiFrame::plain((1600.0, 900.0));
        let c = corners(&ui, &shapes()[0].1);
        // An arena's field fitted into 1600 x 900: 1088 x 544 at 1.47,
        // letterboxed top and bottom.
        let view = crate::view::View::fit((W, H), (1600.0, 900.0));
        let screen = WorldOnScreen { camera: crate::view::Camera::whole((W, H)), field_origin: Vec2::new(0.0, 0.0), view, ui_scale: ui.scale };
        let at = screen.to_ui(crate::Position::new(10.0, 10.0));
        assert!((at.x - view.to_window(Vec2::new(10.0, 10.0)).x).abs() < 1e-3);
        assert!((screen.len_to_ui(32.0) - 32.0 * view.scale).abs() < 1e-3);
        // A tank in the field's top-left corner is under the left cluster,
        // one in its middle under neither, one at the top-right under the
        // right one.
        let tank = |x: f32, y: f32| vec![(crate::Position::new(x, y), 32.0)];
        assert_eq!(covered(&c, &tank(40.0, 30.0), &screen), [true, false]);
        assert_eq!(covered(&c, &tank(W / 2.0, H / 2.0), &screen), [false, false]);
        assert_eq!(covered(&c, &tank(W - 40.0, 30.0), &screen), [false, true]);
        assert_eq!(covered(&c, &[], &screen), [false, false]);
        // A tank rolling in through a gate above the field projects onto
        // the letterbox the right cluster stands in, but nothing is drawn
        // there: it fades nothing until it is on the field.
        let lane = crate::Position::new(W - 40.0, -20.0);
        let right = c.right;
        let p = screen.to_ui(lane);
        assert!(p.y >= right.y && p.y <= right.y + right.height, "the lane projects under the cluster: {p:?} {right:?}");
        assert_eq!(covered(&c, &[(lane, 16.0)], &screen), [false, false]);
    }

    /// The chrome's areas the screens over the round are held to: the
    /// smallest any window lays it out in, an iPhone's inside its safe
    /// area, and a 1080p monitor's.
    fn areas() -> [Rect; 3] {
        [
            UiFrame::plain((UI_MIN_W, UI_MIN_H)).area,
            UiFrame::new((852.0, 393.0), 1.0, 1.0, Insets { left: 59.0, top: 0.0, right: 59.0, bottom: 21.0 }, true).area,
            UiFrame::plain((1920.0, 1080.0)).area,
        ]
    }

    /// Both dialogs, centred in every area: finger-sized buttons in
    /// points, inside their panel and apart, the panel inside the area.
    #[test]
    fn both_dialogs_have_finger_sized_buttons_inside_the_area() {
        for area in areas() {
            let l = leave_dialog_rects(area);
            let p = players_dialog_rects(area);
            for (panel, left, right) in [(l.panel, l.leave, l.stay), (p.panel, p.one, p.two)] {
                for b in [left, right] {
                    assert!(b.width >= 160.0 && b.height >= 48.0 && b.height >= UI_TOUCH_PT);
                    assert!(b.x >= panel.x && b.x + b.width <= panel.x + panel.width);
                    assert!(b.y >= panel.y && b.y + b.height <= panel.y + panel.height);
                }
                assert!(left.x + left.width + 16.0 <= right.x);
                assert!(panel.x >= area.x && panel.x + panel.width <= area.x + area.w, "{area:?}");
                assert!(panel.y >= area.y && panel.y + panel.height <= area.y + area.h, "{area:?}");
                let middle = |r: Rectangle| (r.x + r.width / 2.0, r.y + r.height / 2.0);
                let (mx, my) = middle(panel);
                assert!((mx - (area.x + area.w / 2.0)).abs() <= 0.5 && (my - (area.y + area.h / 2.0)).abs() <= 0.5, "centred");
            }
        }
    }

    /// A banner keeps its size where the area has the room and shrinks to
    /// fit where it has not, never under `BANNER_MIN_SIZE`.
    #[test]
    fn a_banner_shrinks_to_fit_its_area() {
        let [small, _, desktop] = areas();
        assert_eq!(banner_px(small), RESULT_TEXT_PX, "the smallest area is the budget's");
        let banner = "PROTECT THE FROG!";
        assert_eq!(banner_size(banner, BANNER_SIZE, desktop), BANNER_SIZE);
        let fitted = banner_size(banner, BANNER_SIZE, small);
        assert!(fitted < BANNER_SIZE && fitted >= BANNER_MIN_SIZE, "{fitted}");
        assert!(crate::text::width(banner, fitted) <= banner_px(small));
        assert_eq!(banner_size("YOU WIN", BANNER_SIZE, small), BANNER_SIZE);
        assert_eq!(banner_size(&"W".repeat(80), BANNER_SIZE, small), BANNER_MIN_SIZE, "never under the floor");
        assert_eq!(banner_size("LEVEL 3 / 14", LEVEL_NUMBER_SIZE, small), LEVEL_NUMBER_SIZE, "a size under the floor stays");
    }

    /// The end screen fits the smallest area the chrome is laid out in, in
    /// its tallest and widest form - every level complete, eight seats'
    /// shares, three buttons - its buttons finger-sized, apart and in one
    /// row centred on the area, and each form puts its lines top to bottom
    /// without overlapping; a level's countdown is in a button, never a
    /// line.
    #[test]
    fn the_end_screen_fits_the_smallest_area_in_every_form() {
        let fields = areas();
        let forms = [
            None,
            Some(ResultButtons { next: None, countdown: Some(3) }),
            Some(ResultButtons { next: Some(NextLevel::Next), countdown: Some(3) }),
            Some(ResultButtons { next: Some(NextLevel::FirstAgain { levels: 14 }), countdown: None }),
            Some(ResultButtons { next: None, countdown: None }),
            Some(ResultButtons { next: Some(NextLevel::Next), countdown: None }),
        ];
        for field in fields {
            for seats in [1, 2, MAX_SEATS] {
                for buttons in forms {
                    let view = ResultView { stats: RoundStats::default(), seats, buttons };
                    let rows = result_layout(field, &view);
                    let what = format!("{}x{}, {seats} seats, {buttons:?}", field.w, field.h);
                    assert!(rows.title_y >= field.y, "{what}");
                    let mut y = rows.title_y + RESULT_TITLE_SIZE as f32;
                    for (line, size) in [(rows.all_clear_y, RESULT_LINE_SIZE), (Some(rows.stats_y), RESULT_LINE_SIZE), (rows.seats_y, RESULT_SEATS_SIZE)] {
                        if let Some(at) = line {
                            assert!(at >= y, "{what}: a line overlaps the one above");
                            y = at + size as f32;
                        }
                    }
                    assert_eq!(rows.countdown_y.is_some(), buttons.is_none(), "{what}: a line of countdown only in free play");
                    match rows.buttons {
                        Some(r) => {
                            let row: Vec<Rectangle> = [Some(r.levels), Some(r.again), r.next].into_iter().flatten().collect();
                            for b in &row {
                                assert!(b.y >= y && b.y + b.height <= field.y + field.h, "{what}: the buttons leave the area");
                                assert!(b.width >= crate::lobby::LOBBY_TOUCH_MIN && b.height >= crate::lobby::LOBBY_TOUCH_MIN);
                                assert!(b.x >= field.x + 16.0 && b.x + b.width <= field.x + field.w - 16.0, "{what}: a button runs off the side");
                                assert_eq!(b.y, r.again.y, "{what}: one row");
                            }
                            for pair in row.windows(2) {
                                assert!(pair[0].x + pair[0].width + 16.0 <= pair[1].x, "{what}: two buttons touch");
                            }
                            let (left, right) = (row[0].x, row[row.len() - 1].x + row[row.len() - 1].width);
                            assert!(((left + right) / 2.0 - (field.x + field.w / 2.0)).abs() <= 1.0, "{what}: the row is centred");
                            assert_eq!(r.next.is_some(), buttons.is_some_and(|b| b.next.is_some()), "{what}: a way on only after a win");
                        }
                        None => {
                            assert!(buttons.is_none());
                            let at = rows.countdown_y.expect("free play counts down");
                            assert!(at >= y && at + RESULT_LINE_SIZE as f32 <= field.y + field.h, "{what}");
                        }
                    }
                }
            }
        }
    }

    /// The UI scale is the window's points times the knob: a monitor and a
    /// browser draw a point a unit, Android a dp its density in pixels, and
    /// the chrome keeps `UI_EDGE_PT` inside the safe area on every side.
    #[test]
    fn the_ui_frame_is_the_window_in_points_inside_its_safe_area() {
        let desktop = UiFrame::new((1920.0, 1080.0), 1.0, 1.0, Insets::default(), false);
        assert_eq!(desktop.scale, 1.0);
        assert_eq!(desktop.screen, Rect::new(0.0, 0.0, 1920.0, 1080.0));
        assert_eq!(desktop.area, Rect::new(UI_EDGE_PT, UI_EDGE_PT, 1920.0 - 2.0 * UI_EDGE_PT, 1080.0 - 2.0 * UI_EDGE_PT));
        // An iPhone 15 in landscape: the Dynamic Island's strip on one
        // side, the rounded corners' on the other, the home indicator's at
        // the bottom - the chrome stays inside all three.
        let phone = UiFrame::new((852.0, 393.0), 1.0, 1.0, Insets { left: 59.0, top: 0.0, right: 59.0, bottom: 21.0 }, true);
        assert_eq!(phone.scale, 1.0, "a 734 x 372 point safe area holds the chrome at full size");
        assert_eq!(phone.area, Rect::new(59.0 + UI_EDGE_PT, UI_EDGE_PT, 734.0 - 2.0 * UI_EDGE_PT, 372.0 - 2.0 * UI_EDGE_PT));
        // A Pixel at 2.625: its window is in pixels and a point is a dp.
        let android = UiFrame::new((2400.0, 1080.0), 2.625, 1.0, Insets::default(), true);
        assert!((android.scale - 2.625).abs() < 1e-6);
        assert!((android.screen.w - 2400.0 / 2.625).abs() < 1e-3 && (android.screen.h - 1080.0 / 2.625).abs() < 1e-3);
        // The knob multiplies, where the window has the room.
        let big = UiFrame::new((1920.0, 1080.0), 1.0, 1.5, Insets::default(), false);
        assert_eq!(big.scale, 1.5);
        assert_eq!(big.screen, Rect::new(0.0, 0.0, 1280.0, 720.0));
        // A pointer round-trips.
        let p = Vec2::new(300.0, 170.0);
        let back = big.to_window(big.to_ui(p));
        assert!((back.x - p.x).abs() < 1e-4 && (back.y - p.y).abs() < 1e-4);
        assert_eq!(big.rect_to_window(Rectangle::new(10.0, 20.0, 30.0, 40.0)), Rectangle::new(15.0, 30.0, 45.0, 60.0));
    }

    /// A safe area too small for the chrome draws it smaller to fit rather
    /// than off the glass: an iPhone SE's 667 points take it to 0.93, which
    /// leaves the panels' 48 pt buttons over Apple's 44 pt; a knob past
    /// what a phone holds is held back by the same fit.
    #[test]
    fn a_window_too_small_for_the_chrome_draws_it_smaller_to_fit() {
        let se = UiFrame::new((667.0, 375.0), 1.0, 1.0, Insets::default(), true);
        assert!((se.scale - 667.0 / UI_MIN_W).abs() < 1e-6, "{se:?}");
        assert!(se.area.w >= crate::lobby::LOBBY_W - 1e-3 && se.area.h >= crate::lobby::LOBBY_H - 1e-3, "the panels fit: {se:?}");
        assert!(48.0 * se.scale >= UI_TOUCH_PT, "a 48 pt button is {} points", 48.0 * se.scale);
        let zoomed = UiFrame::new((852.0, 393.0), 1.0, 2.0, Insets { left: 59.0, top: 0.0, right: 59.0, bottom: 21.0 }, true);
        assert!(zoomed.area.w >= crate::lobby::LOBBY_W - 1e-3 && zoomed.area.h >= crate::lobby::LOBBY_H - 1e-3, "{zoomed:?}");
        // Nonsense in, a finite frame out.
        let odd = UiFrame::new((f32::NAN, 0.0), f32::INFINITY, -1.0, Insets { left: f32::NAN, top: -5.0, right: 0.0, bottom: 0.0 }, false);
        assert!(odd.scale.is_finite() && odd.scale > 0.0, "{odd:?}");
        assert!(odd.area.w.is_finite() && odd.area.h.is_finite() && odd.area.x.is_finite());
        assert_eq!(UiFrame::plain((1088.0, 544.0)), UiFrame::new((1088.0, 544.0), 1.0, 1.0, Insets::default(), false));
    }

    #[test]
    fn a_rounds_length_reads_as_minutes_and_seconds() {
        assert_eq!(clock_text(0.0), "0:00");
        assert_eq!(clock_text(59.9), "0:59");
        assert_eq!(clock_text(154.2), "2:34");
        assert_eq!(clock_text(3725.0), "62:05");
        assert_eq!(clock_text(-3.0), "0:00");
    }

    /// What the page publishes of its controls is read back to the number,
    /// and anything else takes nothing.
    #[test]
    fn the_page_overlay_reads_what_the_page_published() {
        let o = PageOverlay::parse("30 291.5 3 269 24");
        assert_eq!(o, PageOverlay { top: 30.0, rect: Some(Rectangle::new(291.5, 3.0, 269.0, 24.0)) });
        assert_eq!(o.insets(3.0), Insets { top: 90.0, ..Insets::default() });
        assert_eq!(o.rect_in(2.0), Some(Rectangle::new(583.0, 6.0, 538.0, 48.0)));
        assert_eq!(PageOverlay::parse("0 10 361 254 22"), PageOverlay { top: 0.0, rect: Some(Rectangle::new(10.0, 361.0, 254.0, 22.0)) }, "a desktop's controls take no band");
        assert_eq!(PageOverlay::parse(""), PageOverlay::default(), "nothing published");
        assert_eq!(PageOverlay::parse("30 1 2 3"), PageOverlay::default(), "too few");
        assert_eq!(PageOverlay::parse("30 1 2 3 4 5"), PageOverlay::default(), "too many");
        assert_eq!(PageOverlay::parse("x 1 2 3 4"), PageOverlay::default(), "not a number");
        assert_eq!(PageOverlay::parse("NaN 1 2 3 4"), PageOverlay::default(), "not a number either");
        assert_eq!(PageOverlay::parse("12 5 5 0 0"), PageOverlay { top: 12.0, rect: None }, "no area, no rectangle");
        assert_eq!(PageOverlay::parse("-4 5 5 1 1").top, 0.0, "a band is never negative");
    }

    /// On a touch screen the page lays its controls along the canvas's top,
    /// centred, and publishes that band; kept as a safe area, it puts
    /// every piece of chrome below the controls - the clusters and the
    /// minimap, the lobby's and the level select's panels, both dialogs,
    /// the end screen's buttons and the mission banner - on an iPhone SE's
    /// window too, where the gap between the clusters (74 points) is
    /// narrower than the controls.
    #[test]
    fn the_page_controls_band_keeps_the_chrome_below_them() {
        for (w, h, units) in [(852.0, 393.0, 3.0), (667.0, 375.0, 2.0), (1180.0, 820.0, 2.0), (800.0, 360.0, 2.625), (568.0, 320.0, 2.0)] {
            let controls = Rectangle::new(w / 2.0 - 135.0, 3.0, 270.0, 24.0);
            let overlay = PageOverlay { top: 30.0, rect: Some(controls) };
            let ui = UiFrame::new((w * units, h * units), units, 1.0, overlay.insets(units), true);
            // UI points to CSS pixels: what the page's controls are laid out in.
            let css = |r: Rectangle| {
                let k = ui.scale / units;
                Rectangle::new(r.x * k, r.y * k, r.width * k, r.height * k)
            };
            let clear = |what: &str, r: Rectangle| assert!(apart(css(r), controls), "{w}x{h}: {what} {:?} under the controls", css(r));
            assert!(css(Rectangle::new(ui.area.x, ui.area.y, ui.area.w, ui.area.h)).y >= 30.0, "{w}x{h}: the area starts below the band");
            for (name, shape) in shapes() {
                let c = corners(&ui, &shape);
                clear(&format!("{name}'s left cluster"), c.left());
                clear(&format!("{name}'s right cluster"), c.right);
                if let Some(plate) = c.minimap_plate() {
                    clear(&format!("{name}'s minimap"), plate);
                }
            }
            clear("the lobby", crate::lobby::panel_rect(ui.area));
            clear("the level select", crate::level_select::panel_rect(ui.area));
            clear("the leave dialog", leave_dialog_rects(ui.area).panel);
            clear("the players dialog", players_dialog_rects(ui.area).panel);
            // The end screen at its tallest: a won level of a co-op round,
            // from its title down to its row of buttons.
            let won = ResultView { stats: RoundStats::default(), seats: 2, buttons: Some(ResultButtons { next: Some(NextLevel::FirstAgain { levels: 14 }), countdown: None }) };
            let rows = result_layout(ui.area, &won);
            let buttons = rows.buttons.expect("a level's buttons");
            let bottom = buttons.again.y + buttons.again.height;
            clear("the end screen", Rectangle::new(ui.area.x, rows.title_y, ui.area.w, bottom - rows.title_y));
            // The mission banner, its level number over it at the top.
            let cy = ui.area.y + ui.area.h / 2.0;
            let top = cy - BANNER_SIZE as f32 / 2.0 - 14.0 - LEVEL_NUMBER_SIZE as f32;
            clear("the level banner", Rectangle::new(ui.area.x, top, ui.area.w, cy - top));
        }
    }
}
