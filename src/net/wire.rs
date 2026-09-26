//! The message types of the protocol (docs/online-coop-prd.md §4.3) and
//! the quantisation helpers every position, velocity, heading, health and
//! timer on the wire goes through. Nothing here reads a `Game`: the room
//! server fills these from its round and the client writes them into its
//! replica, and both sides only ever see the same bytes.
//!
//! Every family in a `Snapshot` is a vector sorted by its key (`tanks` by
//! `id`, `shots` by `id`, `frogs` by side, `tiles` and `fires` by `cell`,
//! `bonus_pickups` by cell then kind) with each key at most once;
//! `Snapshot::normalise` establishes that order and `delta` relies on it.

use serde::{Deserialize, Serialize};

use crate::ai::Intent;
use crate::frog::Side;
use crate::level::{LevelOverrides, Mission, SpawnKind, Tier};
use crate::net::MAX_SEATS;
use crate::net::events::WireEvent;
use crate::pickup::PickupKind;
use crate::simulation::Outcome;
use crate::tank::{ActiveWeapon, Dir};

// ---------------------------------------------------------------------------
// Quantisation

/// Position steps per pixel: positions travel in quarter pixels, so an
/// `i16` spans ±`POSITION_MAX_PX` and a round trip lands within an eighth
/// of a pixel (`quantise_pos` rounds to the nearest step).
pub const POSITION_STEPS_PER_PX: f32 = 4.0;

/// The largest magnitude a quantised position can express, in pixels
/// (8191.75); anything beyond saturates.
pub const POSITION_MAX_PX: f32 = i16::MAX as f32 / POSITION_STEPS_PER_PX;

/// Velocity resolution in pixels per second per step: an `i8` spans
/// ±`VELOCITY_MAX_PX_PER_S`, comfortably above the default hull speeds and
/// knockbacks (`tank_speed` 210, `explosion_knockback_speed` 90). The
/// client only extrapolates from it during a gap, so saturation on an
/// extreme knob costs nothing visible.
pub const VELOCITY_STEP_PX_PER_S: f32 = 4.0;

/// The largest speed a quantised velocity component can express (508 px/s).
pub const VELOCITY_MAX_PX_PER_S: f32 = i8::MAX as f32 * VELOCITY_STEP_PX_PER_S;

/// Headings travel as a `u8`: 256 steps around a full turn, 1.40625° each,
/// so a round trip lands within 0.703°.
pub const HEADING_STEPS: f32 = 256.0;

/// Timers travel in tenths of a second and saturate at `TIMER_MAX_SECONDS`.
pub const TIMER_STEPS_PER_SECOND: f32 = 10.0;

/// The longest time a quantised timer can express (25.5 s).
pub const TIMER_MAX_SECONDS: f32 = u8::MAX as f32 / TIMER_STEPS_PER_SECOND;

/// Pixels to quarter pixels, rounded to the nearest step and saturated at
/// ±`POSITION_MAX_PX` (an infinity included). NaN becomes 0.
pub fn quantise_pos(px: f32) -> i16 {
    saturate_i16(px * POSITION_STEPS_PER_PX)
}

/// Quarter pixels back to pixels.
pub fn dequantise_pos(q: i16) -> f32 {
    q as f32 / POSITION_STEPS_PER_PX
}

/// Pixels per second to `VELOCITY_STEP_PX_PER_S` steps, rounded to the
/// nearest step and saturated at ±`VELOCITY_MAX_PX_PER_S` (an infinity
/// included). NaN becomes 0.
pub fn quantise_velocity(px_per_s: f32) -> i8 {
    let v = (px_per_s / VELOCITY_STEP_PX_PER_S).round();
    if v.is_nan() { 0 } else { v.clamp(i8::MIN as f32, i8::MAX as f32) as i8 }
}

/// Velocity steps back to pixels per second.
pub fn dequantise_velocity(q: i8) -> f32 {
    q as f32 * VELOCITY_STEP_PX_PER_S
}

/// Degrees (any range, 0 = up, clockwise positive like `Tank::rotation`)
/// to one of `HEADING_STEPS` steps around the turn, rounded to the
/// nearest. A non-finite angle has no step and becomes 0.
pub fn quantise_heading(degrees: f32) -> u8 {
    if !degrees.is_finite() {
        return 0;
    }
    let turns = degrees.rem_euclid(360.0) / 360.0;
    ((turns * HEADING_STEPS).round() as u32 % HEADING_STEPS as u32) as u8
}

/// A heading step back to degrees in `0.0..360.0`.
pub fn dequantise_heading(q: u8) -> f32 {
    q as f32 * 360.0 / HEADING_STEPS
}

/// Health in whole points, rounded to the nearest and saturated at 0 and
/// 255: enough for a hull (`MAX_DAMAGE` 100), a frog and every wall, the
/// iron one at 220 included. NaN becomes 0.
pub fn quantise_health(hp: f32) -> u8 {
    saturate_u8(hp)
}

/// Whole points back to a float.
pub fn dequantise_health(q: u8) -> f32 {
    q as f32
}

/// Seconds to tenths, rounded to the nearest and saturated at 0 and
/// `TIMER_MAX_SECONDS`. NaN becomes 0.
pub fn quantise_seconds(seconds: f32) -> u8 {
    saturate_u8(seconds * TIMER_STEPS_PER_SECOND)
}

/// Tenths of a second back to seconds.
pub fn dequantise_seconds(q: u8) -> f32 {
    q as f32 / TIMER_STEPS_PER_SECOND
}

fn saturate_i16(v: f32) -> i16 {
    let r = v.round();
    if r.is_nan() { 0 } else { r.clamp(i16::MIN as f32, i16::MAX as f32) as i16 }
}

fn saturate_u8(v: f32) -> u8 {
    let r = v.round();
    if r.is_nan() { 0 } else { r.clamp(0.0, u8::MAX as f32) as u8 }
}

// ---------------------------------------------------------------------------
// Small vocabularies

/// The direction byte of an `IntentMsg`: 0 for none, then `Dir::index` + 1
/// (1 up, 2 down, 3 left, 4 right).
pub fn dir_code(dir: Option<Dir>) -> u8 {
    dir.map_or(0, |d| d.index() as u8 + 1)
}

/// Inverse of `dir_code`; 0 and any code past 4 read as no direction, so a
/// malformed intent stops a tank rather than steering it.
pub fn dir_from_code(code: u8) -> Option<Dir> {
    match code {
        0 => None,
        c => Dir::ALL.get(c as usize - 1).copied(),
    }
}

/// A hull's facing as `Dir::index` (0 up, 1 down, 2 left, 3 right).
pub fn dir_index(dir: Dir) -> u8 {
    dir.index() as u8
}

/// Inverse of `dir_index`; `None` past 3.
pub fn dir_from_index(index: u8) -> Option<Dir> {
    Dir::ALL.get(index as usize).copied()
}

/// A frog's side as one byte: 0 the player's, 1 the enemy's.
pub fn side_code(side: Side) -> u8 {
    match side {
        Side::Player => 0,
        Side::Enemy => 1,
    }
}

/// A tank's weapon on the wire: `Tank::active_weapon`'s value in the
/// snapshot and `Event::Fired`'s name in the events. The variant order is
/// the wire encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WeaponKind {
    #[default]
    Shell,
    Laser,
    Plasma,
    Minigun,
    Missiles,
    Flamethrower,
}

impl WeaponKind {
    /// Every kind, in wire order.
    pub const ALL: [WeaponKind; 6] = [
        WeaponKind::Shell,
        WeaponKind::Laser,
        WeaponKind::Plasma,
        WeaponKind::Minigun,
        WeaponKind::Missiles,
        WeaponKind::Flamethrower,
    ];

    /// The name `ActiveWeapon::name` gives, which is what `Event::Fired`
    /// carries.
    pub fn name(self) -> &'static str {
        ActiveWeapon::from(self).name()
    }

    /// Inverse of `name`; `None` for anything else.
    pub fn parse(name: &str) -> Option<WeaponKind> {
        WeaponKind::ALL.into_iter().find(|k| k.name() == name)
    }
}

impl From<ActiveWeapon> for WeaponKind {
    fn from(w: ActiveWeapon) -> Self {
        match w {
            ActiveWeapon::Shell => WeaponKind::Shell,
            ActiveWeapon::Laser => WeaponKind::Laser,
            ActiveWeapon::Plasma => WeaponKind::Plasma,
            ActiveWeapon::Minigun => WeaponKind::Minigun,
            ActiveWeapon::Missiles => WeaponKind::Missiles,
            ActiveWeapon::Flamethrower => WeaponKind::Flamethrower,
        }
    }
}

impl From<WeaponKind> for ActiveWeapon {
    fn from(w: WeaponKind) -> Self {
        match w {
            WeaponKind::Shell => ActiveWeapon::Shell,
            WeaponKind::Laser => ActiveWeapon::Laser,
            WeaponKind::Plasma => ActiveWeapon::Plasma,
            WeaponKind::Minigun => ActiveWeapon::Minigun,
            WeaponKind::Missiles => ActiveWeapon::Missiles,
            WeaponKind::Flamethrower => ActiveWeapon::Flamethrower,
        }
    }
}

/// Which projectile a `ShotState` is; the variant order is the wire
/// encoding. The laser is an instant beam and travels as `Event::Fired`
/// plus its `Hit`; the flamethrower's cone is on the tank's `FLAME` flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShotKind {
    Shell,
    Bullet,
    Plasma,
}

/// `simulation::Outcome` on the wire (the simulation's own enum only
/// serialises). The spelling is the simulation's, for the one place it
/// travels as JSON rather than as postcard's variant index - the lobby's
/// `Ended`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoundOutcome {
    #[default]
    Playing,
    Won,
    Lost,
}

impl From<Outcome> for RoundOutcome {
    fn from(o: Outcome) -> Self {
        match o {
            Outcome::Playing => RoundOutcome::Playing,
            Outcome::Won => RoundOutcome::Won,
            Outcome::Lost => RoundOutcome::Lost,
        }
    }
}

impl From<RoundOutcome> for Outcome {
    fn from(o: RoundOutcome) -> Self {
        match o {
            RoundOutcome::Playing => Outcome::Playing,
            RoundOutcome::Won => Outcome::Won,
            RoundOutcome::Lost => Outcome::Lost,
        }
    }
}

// ---------------------------------------------------------------------------
// Client -> server

/// A seat's input for one tick: `Intent`'s human-settable fields and
/// nothing else (`fire_aim_offset` and `slow` belong to the AI and never
/// leave the server). Sent every tick; stage 1 samples the newest, stage 2
/// consumes them in `tick` order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentMsg {
    /// The client's tick this input is for (meaningful from stage 2 on).
    pub tick: u32,
    /// `dir_code` of `Intent::move_dir`.
    pub move_dir: u8,
    /// `dir_code` of `Intent::face`.
    pub face: u8,
    /// `Intent::fire`, held for at least two ticks by the client so a short
    /// press survives sampling.
    pub fire: bool,
}

impl IntentMsg {
    /// The wire form of `intent` for `tick`.
    pub fn new(tick: u32, intent: &Intent) -> IntentMsg {
        IntentMsg {
            tick,
            move_dir: dir_code(intent.move_dir),
            face: dir_code(intent.face),
            fire: intent.fire,
        }
    }

    /// The `Intent` this stands for, with the AI-only fields at their
    /// defaults (no aim offset, full throttle).
    pub fn intent(&self) -> Intent {
        Intent {
            move_dir: dir_from_code(self.move_dir),
            face: dir_from_code(self.face),
            fire: self.fire,
            ..Intent::default()
        }
    }
}

impl From<&IntentMsg> for Intent {
    fn from(msg: &IntentMsg) -> Self {
        msg.intent()
    }
}

// ---------------------------------------------------------------------------
// Server -> client: the snapshot

/// Bits of `TankState::flags`.
pub mod tank_flags {
    /// The hull is a wreck.
    pub const WRECK: u8 = 1 << 0;
    /// The rainbow shield is up (`shield` holds its charge).
    pub const SHIELD: u8 = 1 << 1;
    /// The speed boost is running.
    pub const BOOST: u8 = 1 << 2;
    /// The hull is burning (afterburn).
    pub const BURNING: u8 = 1 << 3;
    /// The hit window is running (`Tank::hit_flash_timer`, reset to
    /// `health_ring_hit_seconds` by every hit): what shows an enemy's
    /// health ring. A window rather than "since the previous snapshot"
    /// because an encoder sees one frame, not the previous snapshot.
    pub const HIT: u8 = 1 << 4;
    /// The flamethrower's stream is on.
    pub const FLAME: u8 = 1 << 5;
}


/// One tank, player or enemy, keyed by its owner slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TankState {
    /// `Tank::owner_slot`: 0 and up for the seats, enemies from the first
    /// enemy slot. A wave round's slot counter is monotonic, which is why
    /// this is wider than a byte.
    pub id: u16,
    /// The chassis row (`Tank::row`, `TankKind` in sheet-row order). Never
    /// changes, so a delta never repeats it; carried because a wave tank
    /// arrives mid-round and the roster only names the seats' chassis.
    pub row: u8,
    /// Hull centre, quarter pixels (`quantise_pos`).
    pub x: i16,
    /// Hull centre, quarter pixels (`quantise_pos`).
    pub y: i16,
    /// The body's velocity (`quantise_velocity`), not the commanded one.
    pub vx: i8,
    /// The body's velocity (`quantise_velocity`), not the commanded one.
    pub vy: i8,
    /// Facing, `dir_index`.
    pub dir: u8,
    /// Hull health left, whole points (`quantise_health`).
    pub hp: u8,
    /// Shield charge left, whole points; 0 without a live shield.
    pub shield: u8,
    /// `tank_flags` bits.
    pub flags: u8,
    /// The weapon the next trigger pull fires.
    pub weapon: WeaponKind,
    /// Rounds left for `weapon`, saturated at 255.
    pub ammo: u8,
}

/// One live projectile, keyed by a per-round id the server hands out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShotState {
    pub id: u16,
    pub kind: ShotKind,
    /// Quarter pixels (`quantise_pos`).
    pub x: i16,
    /// Quarter pixels (`quantise_pos`).
    pub y: i16,
    /// `quantise_heading` of the projectile's rotation.
    pub heading: u8,
    /// The choreography state as its sheet column (`ShellState`,
    /// `BulletState`, the plasma's equivalent), so the client picks the
    /// muzzle, flying and impact frames the server is on.
    pub state: u8,
    /// The sprite variant: a shell's row in shells.png (`Shell::variant`,
    /// the shooter's chassis row mapped through `TANK_SHELL_VARIANT_BY_ROW`),
    /// a plasma bolt's `PlasmaVariant` (0 teal, 1 purple), 0 for a bullet.
    /// Never changes, so a delta never repeats it.
    pub variant: u8,
}

/// One seeker missile in flight (`missile.rs`).
///
/// **Only what the drawing needs.** A replica never flies a missile - the
/// seek, the lock, the dive and the burst are all the server's - so the
/// aim point, the target, the stage timers and the speed never travel.
/// What is left is where it is, how high, which way its sprite and its
/// shadow point, and which tube it left: `render/missile.rs` derives the
/// lift, the draw scale and both rotations from exactly these.
///
/// The exhaust flicker is deliberately *not* here. It cycles off `age`,
/// which the replica runs itself in `tick_presentation` - a per-missile
/// flame frame is the kind of cosmetic the wire is allowed to omit, and
/// sending it would cost a byte a missile a snapshot to synchronise a
/// flicker nobody can see out of step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissileState {
    pub id: u16,
    /// Quarter pixels (`quantise_pos`): the point on the ground under it.
    pub x: i16,
    /// Quarter pixels (`quantise_pos`).
    pub y: i16,
    /// Height above the ground in quarter pixels - what `Missile::lift`
    /// turns into the shadow's shrink and the sprite's scale.
    pub height: i16,
    /// `quantise_heading` of `Missile::rotation` - the way the sprite
    /// points, which noses up out of the tube and tips down into the dive.
    pub facing: u8,
    /// `quantise_heading` of `Missile::ground_rotation` - the way the
    /// shadow points, which is the ground heading alone.
    pub heading: u8,
    /// Which tube it left: the flicker salt, fixed for its life.
    ///
    /// No owner rides along, for the reason no shot's does
    /// (`apply::REPLICA_OWNER`): a replica resolves no hits and nothing in
    /// the drawing asks whose missile it is. Whose volley *hit* travels as
    /// the `MissileBlast` event, which is the part anything reads.
    pub tube: u8,
}

/// Bits of `FrogState::state`.
pub mod frog_flags {
    /// The frog is dead.
    pub const DEAD: u8 = 1 << 0;
    /// The frog is mid-hop toward (`hop_x`, `hop_y`).
    pub const HOPPING: u8 = 1 << 1;
    /// The frog was hurt recently (the hurt clip plays).
    pub const HURT: u8 = 1 << 2;
    /// The frog is biting.
    pub const BITING: u8 = 1 << 3;
}

/// One of the round's frogs (the player's, and the Hunt mission's enemy
/// frog), keyed by side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrogState {
    pub side: Side,
    /// Quarter pixels (`quantise_pos`).
    pub x: i16,
    /// Quarter pixels (`quantise_pos`).
    pub y: i16,
    /// Health left, whole points (`quantise_health`).
    pub hp: u8,
    /// `frog_flags` bits.
    pub state: u8,
    /// How far the current hop or bite has come, 0..=255 over its length,
    /// so the client plays the clip in step with the server.
    pub phase: u8,
    /// Where the hop lands, quarter pixels; meaningful while `HOPPING`.
    pub hop_x: i16,
    /// Where the hop lands, quarter pixels; meaningful while `HOPPING`.
    pub hop_y: i16,
}

/// A pickup the Health slot's bonus roll dropped beside a slot
/// (`maybe_spawn_health_slot_bonuses`): a shield or a frog health pack,
/// which have no map slot of their own and so travel as explicit cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BonusPickup {
    /// `row * cols + col` on the map's 32 px grid.
    pub cell: u16,
    pub kind: PickupKind,
}

/// Bits of `TileState::flags`. The upper nibble carries the ram lean.
pub mod tile_flags {
    /// The tile is burning.
    pub const BURNING: u8 = 1 << 0;
    /// A barrel whose fuse is lit.
    pub const FUSED: u8 = 1 << 1;
    /// The tile died on this tick. The simulation despawns a dead tile the
    /// same frame, so `net::encode` writes this entry from the frame's
    /// `ObstacleDestroyed` events (`hp` 0, the cell of the event) and it is
    /// gone from the next snapshot; the event itself is the reliable
    /// channel, the entry lets a client that reads only state see it too.
    pub const DESTROYED: u8 = 1 << 2;
    /// Shift of the ram-lean nibble: the leaning direction in its low two
    /// bits (`dir_index`) and the lean's strength in the high two.
    pub const LEAN_SHIFT: u8 = 4;
    /// Mask of the ram-lean nibble.
    pub const LEAN_MASK: u8 = 0b1111 << LEAN_SHIFT;
}

/// A solid cell whose state differs from the map's fresh one (damaged,
/// burning, fused, sooted, leaning or destroyed), keyed by cell. Untouched
/// tiles never appear: the client has the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileState {
    /// `row * cols + col` on the map's 32 px grid.
    pub cell: u16,
    /// Health left, whole points (`quantise_health`).
    pub hp: u8,
    /// `tile_flags` bits.
    pub flags: u8,
    /// The sooted faces, one bit per side in `Dir::index` order.
    pub faces: u8,
}

/// A burning ground cell (an oil pool or a lit trail cell), keyed by cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FireState {
    /// `row * cols + col` on the map's 32 px grid.
    pub cell: u16,
    /// Time left, tenths of a second (`quantise_seconds`).
    pub left: u8,
}

/// The round's scalar state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundState {
    /// The current wave, 1-based; 0 before the first or in a band round.
    pub wave: u8,
    /// Live enemies on the field.
    pub alive: u8,
    /// Enemies still to roll in this wave.
    pub pending: u8,
    /// Time left on the mission banner, tenths of a second.
    pub intro: u8,
    /// Time left on the breather before the next wave, tenths of a second
    /// - the `WAVE N` banner's timer. Zero when no wave is being
    /// announced, and always zero under the band plan.
    pub next_wave: u8,
    /// Time left on the automatic restart, tenths of a second: the number
    /// the end screen counts down. Zero while the round is playing.
    pub restart: u8,
    pub outcome: RoundOutcome,
}

/// The state of the round at one tick, complete: what a client needs to
/// draw the same picture. Sent in full inside `Welcome`; between those the
/// server sends a `delta::SnapshotDelta` against the previous one.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// The server's tick (`Game::frame`).
    pub tick: u32,
    /// The server's clock in milliseconds, for the client's clock sync.
    pub server_ms: u32,
    /// The last input tick applied per seat: what a client's replay is
    /// measured from (`net::predict`).
    pub acked: [u32; MAX_SEATS],
    /// Each seat's mailbox after this tick's read
    /// (`net::mailbox::Mailbox::wire_state`): the intents still waiting
    /// in the low seven bits, `STARVED_BIT` if the read found none. The
    /// reading the client steers its lead by (docs/online-coop-prd.md
    /// §4.12).
    pub mailbox: [u8; MAX_SEATS],
    pub tanks: Vec<TankState>,
    pub shots: Vec<ShotState>,
    /// Seeker missiles in flight, by `Missile::id`.
    pub missiles: Vec<MissileState>,
    pub frogs: Vec<FrogState>,
    /// One bit per map pickup slot, set while its pickup is on the field.
    pub pickups: u64,
    pub bonus_pickups: Vec<BonusPickup>,
    pub tiles: Vec<TileState>,
    pub fires: Vec<FireState>,
    pub round: RoundState,
    /// What happened on the ticks since the previous snapshot, the AI's
    /// trace left out (`WireEvent::from_event`).
    pub events: Vec<WireEvent>,
}

impl Snapshot {
    /// Sort every family by its key and drop repeated keys (the first
    /// entry wins), the order the delta encoder and every consumer assume.
    pub fn normalise(&mut self) {
        self.tanks.sort_by_key(|t| t.id);
        self.tanks.dedup_by_key(|t| t.id);
        self.shots.sort_by_key(|s| s.id);
        self.shots.dedup_by_key(|s| s.id);
        self.frogs.sort_by_key(|f| side_code(f.side));
        self.frogs.dedup_by_key(|f| side_code(f.side));
        self.bonus_pickups.sort();
        self.bonus_pickups.dedup();
        self.tiles.sort_by_key(|t| t.cell);
        self.tiles.dedup_by_key(|t| t.cell);
        self.fires.sort_by_key(|f| f.cell);
        self.fires.dedup_by_key(|f| f.cell);
    }
}

// ---------------------------------------------------------------------------
// Server -> client: the welcome

/// One seat of the roster.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seat {
    pub seat: u8,
    pub nick: String,
    /// The chassis row (`TankKind` in sheet-row order).
    pub chassis: u8,
}

/// `level::LevelOverrides` on the wire: the room's mission and spawn
/// overrides, resolved by `Game::init` exactly as the CLI's are (room >
/// map > the `waves` tuning group). `None` leaves the map to decide.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireOverrides {
    pub mission: Option<Mission>,
    pub spawn: Option<SpawnKind>,
    pub waves: Option<u32>,
    pub wave_size: Option<u32>,
    pub wave_growth: Option<u32>,
    pub tier_start: Option<Tier>,
    pub tier_end: Option<Tier>,
}

impl From<LevelOverrides> for WireOverrides {
    fn from(o: LevelOverrides) -> Self {
        WireOverrides {
            mission: o.mission,
            spawn: o.spawn,
            waves: o.waves,
            wave_size: o.wave_size,
            wave_growth: o.wave_growth,
            tier_start: o.tier_start,
            tier_end: o.tier_end,
        }
    }
}

impl From<WireOverrides> for LevelOverrides {
    fn from(o: WireOverrides) -> Self {
        LevelOverrides {
            mission: o.mission,
            spawn: o.spawn,
            waves: o.waves,
            wave_size: o.wave_size,
            wave_growth: o.wave_growth,
            tier_start: o.tier_start,
            tier_end: o.tier_end,
        }
    }
}

/// Everything a client needs to build its replica, sent once on join and
/// again on every rejoin: the round's parameters and a full snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Welcome {
    /// `net::PROTOCOL_VERSION` of the server; a client on another version
    /// leaves rather than misread the stream.
    pub protocol: u16,
    /// The server's simulation build, stamped for the logs.
    pub sim_version: String,
    /// The receiving client's seat.
    pub seat: u8,
    pub roster: Vec<Seat>,
    /// The map the round runs on (`map::to_toml_string`).
    pub map_toml: String,
    /// The round's seed: ground, grass and layout come out identical.
    pub seed: u64,
    /// The room's tuning diff, a JSON patch for `tuning::submit_json`.
    pub tuning_json: String,
    pub overrides: WireOverrides,
    /// The room's `--enemies` pin (`Game::enemy_count_override`), which
    /// `LevelOverrides` does not carry: the replica's `init` has to draw
    /// exactly the rolls the server's did, and a pinned count skips one.
    pub enemy_count: Option<u16>,
    /// Oil trail cells (`row * cols + col`) still unburnt at the tick the
    /// welcome was cut; the map's list minus what fire has used up.
    pub oil_cells: Vec<u16>,
    /// Cells (`row * cols + col`) of the map's solid tiles that have died
    /// this round. A snapshot lists only tiles that differ from fresh and
    /// says nothing about one that is gone, so a joiner learns the holes
    /// here and every later death from `ObstacleDestroyed`.
    pub dead_cells: Vec<u16>,
    /// The complete state at the tick the welcome was cut.
    pub snapshot: Snapshot,
}

// ---------------------------------------------------------------------------
// Lobby, both directions

/// One seat as the lobby shows it (`Lobby::Roster`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterSeat {
    pub seat: u8,
    pub nick: String,
    /// The chassis row (`TankKind` in sheet-row order); 0 until the round
    /// starts and the room rolls it.
    pub chassis: u8,
    pub ready: bool,
    /// Somebody is on the socket right now; a seat that dropped keeps its
    /// place while it reconnects.
    pub connected: bool,
}

/// The lobby's vocabulary, JSON on the same socket as the binary messages:
/// `{"type": "join", "nick": ..}`. Creating a room is a lobby message too,
/// so a client needs no HTTP. The first block is what a client says, the
/// second what the room answers; a new optional field or variant here
/// needs no `PROTOCOL_VERSION` bump.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Lobby {
    /// Open a room: `map` names a shipped map, or `map_toml` carries a
    /// builder map instead. `seed` pins the round's seed (a replay or a
    /// test); absent, the room draws one.
    Create {
        nick: String,
        device_token: String,
        map: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        map_toml: Option<String>,
        mission: Mission,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seed: Option<u64>,
    },
    /// Take a seat in the room `code` names; the same `device_token`
    /// reclaims the seat after a disconnect.
    Join { nick: String, device_token: String, code: String },
    Ready,
    Start,
    Leave,
    Kick { seat: u8 },
    Chat { text: String },

    /// The room refused or could not do what was asked; the socket stays
    /// open unless the message says otherwise.
    Error { message: String },
    /// The answer to `Create`: the code to share. A `Welcome` follows.
    RoomCreated { code: String },
    /// Every seat of the room, sent to everyone whenever a seat joins,
    /// leaves, readies, drops or comes back.
    Roster { host: u8, seats: Vec<RosterSeat> },
    /// The host started the round; a fresh `Welcome` with the round's
    /// parameters follows.
    Started,
    /// The round is over, with how it went. The room stops ticking here,
    /// so the end cannot travel in a snapshot - the stream ends with the
    /// one the countdown ran out on. A `Roster` follows, and the room is
    /// back in its lobby: `Start` from the host is the rematch.
    Ended { outcome: RoundOutcome },
    /// A `Chat` relayed to every seat with its sender.
    Said { seat: u8, text: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_round_trip_stays_within_a_quarter_pixel() {
        let mut px = -POSITION_MAX_PX;
        while px <= POSITION_MAX_PX {
            let back = dequantise_pos(quantise_pos(px));
            assert!((back - px).abs() <= 0.125 + 1e-4, "{px} -> {back}");
            px += 0.37;
        }
        for exact in [0.0, 0.25, -0.25, 100.75, 1087.5, -8191.75] {
            assert_eq!(dequantise_pos(quantise_pos(exact)), exact);
        }
    }

    #[test]
    fn position_saturates_and_survives_nan() {
        assert_eq!(quantise_pos(1e9), i16::MAX);
        assert_eq!(quantise_pos(-1e9), i16::MIN);
        assert_eq!(quantise_pos(f32::NAN), 0);
        assert_eq!(quantise_pos(f32::INFINITY), i16::MAX);
        assert_eq!(quantise_pos(f32::NEG_INFINITY), i16::MIN);
    }

    #[test]
    fn velocity_round_trip_within_half_a_step() {
        for v in [-508.0, -210.0, -3.0, 0.0, 1.9, 2.0, 160.0, 210.0, 508.0] {
            let back = dequantise_velocity(quantise_velocity(v));
            assert!((back - v).abs() <= VELOCITY_STEP_PX_PER_S / 2.0, "{v} -> {back}");
        }
        assert_eq!(quantise_velocity(5000.0), i8::MAX);
        assert_eq!(quantise_velocity(-5000.0), i8::MIN);
        assert_eq!(quantise_velocity(f32::NAN), 0);
        assert_eq!(quantise_velocity(f32::INFINITY), i8::MAX);
    }

    #[test]
    fn heading_round_trip_within_half_a_step() {
        let half_step = 360.0 / HEADING_STEPS / 2.0;
        let mut deg = -720.0;
        while deg < 720.0 {
            let back = dequantise_heading(quantise_heading(deg));
            let diff = (back - deg).rem_euclid(360.0);
            let diff = diff.min(360.0 - diff);
            assert!(diff <= half_step + 1e-3, "{deg} -> {back}");
            deg += 1.3;
        }
        assert_eq!(quantise_heading(0.0), 0);
        assert_eq!(quantise_heading(360.0), 0);
        assert_eq!(quantise_heading(90.0), 64);
        assert_eq!(quantise_heading(-90.0), 192);
        assert_eq!(quantise_heading(359.9), 0);
        assert_eq!(quantise_heading(f32::NAN), 0);
        assert_eq!(quantise_heading(f32::INFINITY), 0);
    }

    #[test]
    fn health_and_timers_round_and_saturate() {
        assert_eq!(quantise_health(99.6), 100);
        assert_eq!(quantise_health(220.0), 220);
        assert_eq!(quantise_health(-5.0), 0);
        assert_eq!(quantise_health(1000.0), 255);
        assert_eq!(quantise_health(f32::INFINITY), 255);
        assert_eq!(quantise_health(f32::NAN), 0);
        assert_eq!(dequantise_health(37), 37.0);
        assert_eq!(quantise_seconds(1.26), 13);
        assert_eq!(quantise_seconds(60.0), 255);
        assert_eq!(dequantise_seconds(13), 1.3);
    }

    #[test]
    fn intent_round_trips_every_human_field() {
        let dirs = [None, Some(Dir::Up), Some(Dir::Down), Some(Dir::Left), Some(Dir::Right)];
        for &move_dir in &dirs {
            for &face in &dirs {
                for fire in [false, true] {
                    let intent = Intent { move_dir, face, fire, fire_aim_offset: 12.5, slow: 0.5 };
                    let msg = IntentMsg::new(7, &intent);
                    let back: Intent = (&msg).into();
                    assert_eq!(back.move_dir, move_dir);
                    assert_eq!(back.face, face);
                    assert_eq!(back.fire, fire);
                    assert_eq!(back.fire_aim_offset, 0.0, "AI-only field must not travel");
                    assert_eq!(back.slow, 0.0, "AI-only field must not travel");
                    assert_eq!(msg.tick, 7);
                }
            }
        }
    }

    #[test]
    fn malformed_direction_codes_read_as_none() {
        assert_eq!(dir_from_code(0), None);
        assert_eq!(dir_from_code(4), Some(Dir::Right));
        assert_eq!(dir_from_code(5), None);
        assert_eq!(dir_from_code(255), None);
        for d in Dir::ALL {
            assert_eq!(dir_from_code(dir_code(Some(d))), Some(d));
            assert_eq!(dir_from_index(dir_index(d)), Some(d));
        }
        assert_eq!(dir_from_index(4), None);
    }

    #[test]
    fn weapon_kind_mirrors_active_weapon() {
        for kind in WeaponKind::ALL {
            let active: ActiveWeapon = kind.into();
            assert_eq!(WeaponKind::from(active), kind);
            assert_eq!(WeaponKind::parse(kind.name()), Some(kind));
            assert_eq!(active.name(), kind.name());
        }
        assert_eq!(WeaponKind::parse("catapult"), None);
    }

    #[test]
    fn outcome_and_overrides_mirror_the_simulation() {
        for o in [Outcome::Playing, Outcome::Won, Outcome::Lost] {
            let back: Outcome = RoundOutcome::from(o).into();
            assert_eq!(back, o);
        }
        let overrides = LevelOverrides {
            mission: Some(Mission::Hunt),
            spawn: Some(SpawnKind::Waves),
            waves: Some(3),
            wave_size: Some(4),
            wave_growth: Some(1),
            tier_start: Some(Tier::Light),
            tier_end: Some(Tier::Super),
        };
        let back: LevelOverrides = WireOverrides::from(overrides).into();
        assert_eq!(back, overrides);
    }

    #[test]
    fn normalise_sorts_and_dedups_every_family() {
        let mut s = Snapshot {
            tanks: vec![
                TankState { id: 3, ..Default::default() },
                TankState { id: 1, hp: 9, ..Default::default() },
                TankState { id: 1, hp: 2, ..Default::default() },
            ],
            fires: vec![FireState { cell: 9, left: 1 }, FireState { cell: 2, left: 1 }],
            ..Default::default()
        };
        s.normalise();
        assert_eq!(s.tanks.iter().map(|t| t.id).collect::<Vec<_>>(), vec![1, 3]);
        assert_eq!(s.tanks[0].hp, 9, "the first entry wins");
        assert_eq!(s.fires.iter().map(|f| f.cell).collect::<Vec<_>>(), vec![2, 9]);
    }

    #[test]
    fn lobby_is_tagged_json() {
        let msg = Lobby::Join { nick: "oto".into(), device_token: "tok".into(), code: "AK7QX".into() };
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"join","nick":"oto","device_token":"tok","code":"AK7QX"}"#);
        assert_eq!(serde_json::from_str::<Lobby>(&json).unwrap(), msg);
        assert_eq!(serde_json::to_string(&Lobby::Ready).unwrap(), r#"{"type":"ready"}"#);
        let create = Lobby::Create {
            nick: "oto".into(),
            device_token: "tok".into(),
            map: "default".into(),
            map_toml: None,
            mission: Mission::Protect,
            seed: None,
        };
        let json = serde_json::to_string(&create).unwrap();
        assert!(!json.contains("map_toml"), "an absent builder map is left out: {json}");
        assert!(!json.contains("seed"), "an absent seed is left out: {json}");
        assert_eq!(serde_json::from_str::<Lobby>(&json).unwrap(), create);
        let without_seed = r#"{"type":"create","nick":"oto","device_token":"tok","map":"default","mission":"protect"}"#;
        assert_eq!(serde_json::from_str::<Lobby>(without_seed).unwrap(), create);
    }

    #[test]
    fn lobby_replies_are_tagged_json() {
        let roster = Lobby::Roster {
            host: 0,
            seats: vec![RosterSeat { seat: 0, nick: "oto".into(), chassis: 4, ready: true, connected: true }],
        };
        let json = serde_json::to_string(&roster).unwrap();
        assert_eq!(
            json,
            r#"{"type":"roster","host":0,"seats":[{"seat":0,"nick":"oto","chassis":4,"ready":true,"connected":true}]}"#
        );
        assert_eq!(serde_json::from_str::<Lobby>(&json).unwrap(), roster);
        assert_eq!(
            serde_json::to_string(&Lobby::RoomCreated { code: "AK7QX".into() }).unwrap(),
            r#"{"type":"room_created","code":"AK7QX"}"#
        );
        assert_eq!(serde_json::to_string(&Lobby::Started).unwrap(), r#"{"type":"started"}"#);
        let ended = Lobby::Ended { outcome: RoundOutcome::Lost };
        let json = serde_json::to_string(&ended).unwrap();
        assert_eq!(json, r#"{"type":"ended","outcome":"lost"}"#);
        assert_eq!(serde_json::from_str::<Lobby>(&json).unwrap(), ended);
        assert_eq!(
            serde_json::to_string(&Lobby::Error { message: "full".into() }).unwrap(),
            r#"{"type":"error","message":"full"}"#
        );
    }
}
