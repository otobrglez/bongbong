//! The simulation layer: everything that decides what the game is doing
//! this frame - physics, damage, AI, spawning - with no dependency on a
//! window or `RaylibHandle`. `game.rs` (the presentation layer) reads
//! `Game`'s state afterward and draws it; this module never reaches back.
//! `Game::init`/`Game::update` take plain numbers and an `Input` snapshot,
//! so a round can be driven headlessly (see `src/bin/probe.rs` and the
//! tests at the bottom of this file). `crate::math::Vec2` (`Position`) is
//! the one shared vector type, so nothing here names a window or drawing
//! type.
//!
//! Layout: this file owns `Game` (state, `init`, the phased `update`) and
//! the small helpers those phases share; `weapons` fires shots and ticks
//! queued ones; `hits` is the swept projectile hit test over a per-frame
//! terrain snapshot; `combat` applies damage, knockback, rams and
//! explosions; `engage` hands attacking enemies distinct engagement slots.
//!
//! Determinism: all round randomness flows from the one seeded `SmallRng`
//! in `Game::rng` (never `rand::rng()` on the simulation path, never
//! iterate a HashMap/HashSet where the body consumes RNG or spawns) -
//! `determinism_tests` replays a seed twice and bit-compares.

mod combat;
mod command;
mod comms;
pub mod debug;
mod engage;
mod flame;
mod hits;
mod missiles;
mod props;
pub use props::{FlyingDrum, GroundFire};
pub mod replica;
#[cfg(test)]
mod flame_tests;
#[cfg(test)]
mod props_tests;
#[cfg(test)]
mod seat_tests;
mod waves;
mod weapons;

pub use waves::{RollIn, WaveStatus};
pub use weapons::FlameJet;

/// Which projectile a client's provisional shot is drawn as
/// (`net::predict`): the three the wire also carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProvisionalKind {
    Shell,
    Bullet,
    Plasma,
}

/// A client's own shot before the server has confirmed it: the pose and
/// nothing else, since it is drawn and never simulated (`net::predict`).
/// `Game::seat_shot` builds one from a seat's muzzle and
/// `Game::add_provisional_shot` puts it in a replica's world for a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProvisionalShot {
    pub kind: ProvisionalKind,
    /// The seat that fired it, for the shell's owner colouring.
    pub seat: u8,
    pub position: Position,
    pub prev_position: Position,
    pub velocity: Vec2,
    pub rotation: f32,
    /// A shell's row in shells.png; 0 for the other kinds.
    pub variant: i32,
    /// A bolt's variant; `Teal` for the other kinds.
    pub plasma_variant: PlasmaVariant,
    pub shooter_row: i32,
}
use waves::WaveState;

use crate::blast::{BlastFx, Scorch};
use crate::decal::Decal;
use crate::tuning::tuning;
use std::collections::{BTreeMap, HashMap, HashSet};

use hecs::Entity;
use rand::rngs::SmallRng;
use rand::{RngExt, SeedableRng};
use serde::{Deserialize, Serialize};
use crate::math::Vec2;

use crate::ai::{Ai, AiSnapshot, Intent, Mover, Role, WallAhead};
use crate::battlefield;
use crate::bullet::Bullet;
use crate::frog::{Facing, Frog, Side};
use crate::laser::{LaserBeam, LaserVariant};
use crate::missile::Missile;
use crate::level::{LevelOverrides, Mission, SpawnPlan};
use crate::map::{self, CellObject, MapFile};
use crate::obstacle::{Drum, Material, Obstacle, neighbour_mask};
use crate::pathfind::Grid;
use crate::physics::Physics;
use crate::pickup::{Pickup, PickupKind};
use crate::plasma::{Plasma, PlasmaVariant};
use crate::shell::{Owner, Shell, ShellState};
use crate::shockwave::Shockwave;
use crate::tank::{ActiveWeapon, Dir, Tank, TankKind};
use crate::track::Track;
use crate::{
    DAMAGE_VARIANTS,
    FROG_COLLIDER_HALF_EXTENT,
    MAX_DAMAGE,
    OBSTACLE_CLEAR,
    OBSTACLE_GRID_SIZE,
    OBSTACLE_HULL_FRACTION,
    OBSTACLE_SCALE,
    OBSTACLE_TEXTURE_SIZE,
    DECAL_MAX,
    TANK_TEXTURE_SIZE,
    SHOCK_MAX,
    RUBBLE_ROW_TANK,
    SCORCH_MAX,
    MAX_SEATS,
    PATHFIND_CELL_SIZE,
    PHYSICS_FIXED_DT,
    PHYSICS_MAX_CATCHUP_SECONDS,
    Position,
    TANK_HULL_DISABLED_DAMAGE,
    TANK_HULL_TRACK_COLS,
    TANK_SHELL_VARIANT_BY_ROW,
    TANK_WRECK_COLS,
};

use combat::{frog_hop_target, ram, HitEffects};
use engage::{EngageCtx, EngageReport, EngageRing, EngageStatus, EngageTank};
use hits::{ShellTarget, Terrain};
use weapons::{dispatch_fire, dispatch_fire_from, laser_damage_range, tick_queued_shots, PendingLaserShot, Projectile, laser_beam_half_width};

/// One step's player input, gathered by the caller (`app.rs` reading a
/// live `RaylibHandle`, the dev server, a scripted probe) - the entire
/// interface between the simulation and wherever input comes from. A seat's `Intent` is the AI's, so the
/// players and every enemy drive through the identical `drive_tank`/fire
/// path; the four flags below are meta/UI toggles `Intent` has no use for.
#[derive(Default, Clone, Copy)]
pub struct Input {
    /// One raw movement/fire command per seat: seat 0 is player 1, seat 1
    /// player 2, and so on up to `MAX_SEATS`. The round reads the first
    /// `Game::players.count()` seats and nothing else, so a single-player
    /// caller leaves seat 1 at its default (no input) and a seat nobody
    /// drives stands still. `fire` is "is the fire key held this step",
    /// not edge-detected: `update` decides whether that fires (a laser or
    /// minigun is full-auto while held; shells and plasma need a fresh
    /// press), since that depends on the player's current weapon.
    pub seats: [Intent; MAX_SEATS],
    pub pause_pressed: bool,
    pub restart_pressed: bool,
    pub toggle_shadows_pressed: bool,
    /// The I key in a dev build (`--features dev-tools`): cycles
    /// `Game::debug_overlays` through its presets (`Overlays::next_preset`).
    /// Never set in a release build.
    pub cycle_overlays_pressed: bool,
}

impl Input {
    /// Seat 0 driven, every other seat idle, no toggles.
    pub fn single(intent: Intent) -> Input {
        let mut input = Input::default();
        input.seats[0] = intent;
        input
    }

    /// Seats 0 and 1 driven (a two-player round), no toggles.
    pub fn two(player1: Intent, player2: Intent) -> Input {
        let mut input = Input::single(player1);
        input.seats[1] = player2;
        input
    }

    /// Seat `index`'s command; a seat past `MAX_SEATS` reads as no input.
    pub fn seat(&self, index: usize) -> Intent {
        self.seats.get(index).copied().unwrap_or_default()
    }

    /// This input for the second and later steps of one rendered frame:
    /// what is held stays held (directions, fire), the one-shot presses
    /// (pause, restart, shadows, overlays) are spent by the first step, so
    /// a frame that runs two steps toggles pause once, not twice.
    pub fn held_only(mut self) -> Input {
        self.pause_pressed = false;
        self.restart_pressed = false;
        self.toggle_shadows_pressed = false;
        self.cycle_overlays_pressed = false;
        self
    }

    /// This input with `pending`'s presses folded in: every seat's `fire`
    /// and every one-shot toggle is either input's. The caller keeps the
    /// input of a rendered frame that ran no step and folds it into the
    /// next frame's, so a tap that lands between two steps still reaches
    /// the simulation as a held fire key or a pressed toggle; directions
    /// are `self`'s alone - a held key is read fresh every frame.
    pub fn or_presses(mut self, pending: Input) -> Input {
        for (seat, carried) in self.seats.iter_mut().zip(pending.seats) {
            seat.fire |= carried.fire;
        }
        self.pause_pressed |= pending.pause_pressed;
        self.restart_pressed |= pending.restart_pressed;
        self.toggle_shadows_pressed |= pending.toggle_shadows_pressed;
        self.cycle_overlays_pressed |= pending.cycle_overlays_pressed;
        self
    }
}

/// How many seats this round holds, 1 to `MAX_SEATS` - a session setting,
/// chosen from the HUD's players dialog, `--players`, or the size of a
/// room's roster, and kept across restarts like `Game::player_row_override`.
/// A count, not a list of variants: the couch offers one or two, a room up
/// to eight, and everything downstream reads `count()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PlayerCount(u8);

impl PlayerCount {
    /// A single-player round - the default.
    pub const ONE: PlayerCount = PlayerCount(1);
    /// The two-player couch round (docs/two-players.md).
    pub const TWO: PlayerCount = PlayerCount(2);
    /// Every seat the protocol carries.
    pub const MAX: PlayerCount = PlayerCount(MAX_SEATS as u8);

    /// How many seats hold a tank this round.
    pub fn count(self) -> usize {
        self.0 as usize
    }

    /// `n` seats, or `None` outside `1..=MAX_SEATS`.
    pub fn from_count(n: usize) -> Option<PlayerCount> {
        (1..=MAX_SEATS).contains(&n).then_some(PlayerCount(n as u8))
    }
}

impl Default for PlayerCount {
    fn default() -> Self {
        PlayerCount::ONE
    }
}

/// Player 1's owner slot (`Tank::owner_slot`). Each further seat takes the
/// next number; the enemies count up from `Game::first_enemy_slot`.
pub(crate) const PLAYER_OWNER_SLOT: usize = 0;

/// How the current round is going.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    #[default]
    Playing,
    /// Every enemy is a wreck.
    Won,
    /// Every player tank is a wreck, or the frog died.
    Lost,
}

/// One thing that happened during a frame, for tooling (the dev server's
/// event feed, headless tests): appended by the phase that caused it and
/// readable through `Game::events` until the next `update` clears it.
/// Recording never consumes RNG, so it is free for replay determinism.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    RoundStarted { seed: u64, enemies: usize, mission: Mission, spawn: crate::level::SpawnKind },
    RoundEnded { outcome: Outcome },
    /// A trigger pull that launched something. A twin barrel's queued
    /// second shot and a burst's later bullets are part of the same pull.
    Fired { slot: usize, weapon: &'static str },
    /// A projectile or beam landed on `target` at (`x`, `y`).
    Hit { target: HitTarget, damage: f32, killed: bool, x: f32, y: f32 },
    /// A tank became a wreck (any cause) at (`x`, `y`).
    Wreck { slot: usize, x: f32, y: f32 },
    /// Ram contact between the tanks in `slot` and `other_slot`: an enemy and
    /// a player, two enemies, or the two players. The lower-numbered party is
    /// always `slot`.
    ///
    /// `damage` is the **roll**, not necessarily what landed. Both parties are
    /// put through `Tank::take_damage`, so a shielded one absorbs its share
    /// into `shield_hp` and its hull takes nothing. One `f32` cannot express
    /// two different applied amounts, and the two parties genuinely can differ
    /// now that the shield is a pool, so this reports the shared input rather
    /// than pretending to report an outcome. Sum it for "how hard the pack is
    /// shoving itself", never for "damage dealt".
    Ram { slot: usize, other_slot: usize, damage: f32 },
    PickupCollected { slot: usize, kind: PickupKind },
    PickupRespawned { kind: PickupKind, x: f32, y: f32 },
    /// A projectile bounced off `slot`'s rainbow shield at (`x`, `y`).
    Deflected { slot: usize, x: f32, y: f32 },
    /// `slot`'s shot bounced off terrain at (`x`, `y`) - a shell off iron
    /// (`Projectile::try_ricochet`) or a shell or bullet off a barrel
    /// (`Material::deflect_chance`) - and flies on along `heading`
    /// (degrees, 0 = up). Recorded so a client that sees only positions
    /// knows why the shot turned.
    Ricochet { slot: usize, x: f32, y: f32, heading: f32 },
    /// `slot`'s rainbow shield ran out of charge and shattered at
    /// (`x`, `y`) - the frame it crossed zero, emitted once.
    ShieldBroken { slot: usize, x: f32, y: f32 },
    /// Two opposing shells met mid-air and cancelled at (`x`, `y`).
    ShellsCollided { x: f32, y: f32 },
    /// The `side` frog bit the tank in `slot`.
    FrogBite { side: Side, slot: usize, damage: f32, killed: bool },
    /// The tank in `slot` collected a frog health pack and healed the
    /// `side` frog by `amount`, at the frog's own position (`x`, `y`) -
    /// which is nowhere near the pickup, so `fx.rs` needs the coordinates
    /// rather than deriving them from the collector.
    FrogHealed { side: Side, slot: usize, amount: f32, x: f32, y: f32 },
    /// The wave scheduler called wave `wave` (1-based): `size` tanks of
    /// `tier` queued to roll in.
    WaveStarted { wave: u32, size: u32, tier: crate::level::Tier },
    /// A wave tank in `slot` finished rolling in: it now has a body and an
    /// `Ai`, and counts as an enemy on the field.
    TankEntered { slot: usize },
    /// A wave round removed the wreck in `slot` after
    /// `wave_wreck_despawn_seconds`.
    WreckRemoved { slot: usize },
    /// A destructible tile (wall or prop) died at (`x`, `y`), whatever
    /// destroyed it.
    ObstacleDestroyed { material: Material, x: f32, y: f32 },
    /// A barrel detonated at (`x`, `y`); `chained` when another blast's
    /// fuse (or a fire) set it off rather than a shot or a ram; `drum`
    /// says which kind went off.
    Blast { x: f32, y: f32, chained: bool, drum: Drum },
    /// A fuel drum set off by another blast launched from (`x`, `y`)
    /// toward (`to_x`, `to_y`), where it will detonate when it lands.
    DrumLaunched { x: f32, y: f32, to_x: f32, to_y: f32 },
    /// `slot`'s tank entered the portal at (`x`, `y`) and was placed at
    /// (`to_x`, `to_y`), beside another portal (`Game::portal_phase`).
    Teleported { slot: usize, x: f32, y: f32, to_x: f32, to_y: f32 },
    /// A ground cell centred on (`x`, `y`) caught fire: an oil drum's
    /// pool (`pool`) or a lit trail cell.
    FireStarted { x: f32, y: f32, pool: bool },
    /// The flamethrower's stream lit something at (`x`, `y`) after
    /// `flame_ignite_seconds` of exposure (`what`: `ground`, `oil`,
    /// `wood`, `tree`, `drum`), or collapsed a prop under sustained heat
    /// (`sandbag`, `fence`).
    Ignited { x: f32, y: f32, what: &'static str },
    /// A seeker missile fired by `slot` finished its climb and locked on:
    /// onto the tank in `target` slot, or - `None` - onto the ground point
    /// its launcher was aimed at, since nothing was in `missile_seek_range`.
    MissileLocked { slot: usize, target: Option<usize>, x: f32, y: f32 },
    /// A seeker missile fired by `slot` came down and burst at (`x`, `y`).
    MissileBlast { slot: usize, x: f32, y: f32 },
    /// A delayed secondary pop from a wreck's ammo cooking off. Purely
    /// cosmetic - it deals no damage - but recorded so tooling and the
    /// presentation layer can see it.
    CookOff { x: f32, y: f32 },
    /// Rapier quarantined `bodies` bodies and `colliders` colliders on one
    /// fixed step because their state went non-finite (see
    /// `Physics::quarantined`). A solver blow-up, never normal play - the
    /// probe counts it as an `invariant` anomaly.
    PhysicsQuarantine { bodies: usize, colliders: usize },
    // --- AI decisions, recorded only while `Game::trace_ai` is set: each
    // is a transition the enemy phase observed by comparing an enemy's
    // `AiSnapshot` before and after its `think`, so `ai.rs` stays
    // snapshot-only and the recording adds nothing to the simulation. ---
    /// The behaviour tree settled on a different action than last frame.
    AiAction { slot: usize, from: Option<&'static str>, to: Option<&'static str> },
    /// The engagement ring gave `slot` a different slot index (see
    /// `engage::EngageSlot::index`; `None` = steering at its target - the
    /// player, or a hunter's frog - directly).
    EngageSlot { slot: usize, from: Option<u8>, to: Option<u8> },
    /// The stuck escape fired (`escapes` so far this round).
    StuckEscape { slot: usize, escapes: u32 },
    /// A breach started toward `dir`, or ended (`None`).
    Breach { slot: usize, dir: Option<&'static str> },
    /// The ammo retreat latched (`on`) or released.
    Retreat { slot: usize, on: bool },
    /// The shared last-known player position appeared or expired; `x`/`y`
    /// is the position (the last known one when `on` is false).
    Alert { on: bool, x: f32, y: f32 },
    /// Rounds with more than one seat: the enemy in `slot` switched to
    /// fighting `player` (`Ai::target_player`).
    Retarget { slot: usize, player: u8 },
}

/// What an `Event::Hit` landed on.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "what", rename_all = "snake_case")]
pub enum HitTarget {
    /// Human player `player` (0 or 1).
    Player { player: u8 },
    Enemy { slot: usize },
    Frog { side: Side },
    /// A destructible tile. Carries its material because the presentation
    /// layer has to know what a shot just knocked a chip off: masonry,
    /// glass and timber throw very different debris, and `Event::Hit` is
    /// the only signal a *non-destroying* hit produces.
    Obstacle { material: Material },
    Wall,
}

/// Debug overlay switches `render` reads (dev builds only - see `game.rs`),
/// set by the dev server's `overlays` tool or cycled by the I key. Survive
/// restarts; all off by default.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Overlays {
    /// Every tank's hull damage box, turret box and rounded movement
    /// collider (`game.rs`'s `draw_tank_boxes`).
    pub hitboxes: bool,
    /// Every tank's readout card - ammo, weapon, hp, speed, velocity,
    /// collider size, and an enemy's retreat/fire state (`game.rs`'s
    /// `draw_tank_stats`).
    pub stats: bool,
    /// The routing grid: blocked cells, priced cells shaded by their
    /// surcharge, and player 1's flow field as an arrow per cell.
    pub nav_grid: bool,
    /// Each enemy's waypoint, committed heading and last behaviour-tree action.
    pub ai: bool,
    /// Projectile hit boxes and velocity vectors.
    pub projectiles: bool,
    /// Engagement-ring targets.
    pub engage: bool,
    /// Pickup collection radii.
    pub pickups: bool,
}

impl Overlays {
    /// Every overlay off.
    pub const NONE: Overlays = Overlays {
        hitboxes: false,
        stats: false,
        nav_grid: false,
        ai: false,
        projectiles: false,
        engage: false,
        pickups: false,
    };
    /// Both tank layers - hitboxes and the stats card - and nothing else.
    pub const INSPECT: Overlays = Overlays {
        hitboxes: true,
        stats: true,
        ..Overlays::NONE
    };
    /// Every overlay on.
    pub const ALL: Overlays = Overlays {
        hitboxes: true,
        stats: true,
        nav_grid: true,
        ai: true,
        projectiles: true,
        engage: true,
        pickups: true,
    };

    /// Whether any layer is on.
    pub fn any(self) -> bool {
        self != Overlays::NONE
    }

    /// The I key's cycle: `NONE` -> `INSPECT` -> `ALL` -> `NONE`. A hand-set
    /// mix (the dev server's `overlays` tool - one tank layer without the
    /// other counts) snaps to its next step: nothing on -> `INSPECT`,
    /// exactly `INSPECT` -> `ALL`, anything else -> `NONE`.
    pub fn next_preset(self) -> Overlays {
        if !self.any() {
            Overlays::INSPECT
        } else if self == Overlays::INSPECT {
            Overlays::ALL
        } else {
            Overlays::NONE
        }
    }

    /// The name the dev label prints: `None` while nothing is on, otherwise
    /// the preset this mix is exactly (`inspect`, `all`) or `custom`.
    pub fn preset_name(self) -> Option<&'static str> {
        if !self.any() {
            None
        } else if self == Overlays::INSPECT {
            Some("inspect")
        } else if self == Overlays::ALL {
            Some("all")
        } else {
            Some("custom")
        }
    }
}

/// scifi_tanks_sheet.png has 12 row-variants, one archetype per row (see
/// docs/SPRITESHEET_SPEC.md §4). Rows 1, 4, 7, 9 and 10 are twin-barrel.
const TANK_VARIANTS: i32 = 12;

/// Enemy spawn order over the 12 variants, alternating twin- and
/// single-barrel chassis so both kinds appear early however few tanks
/// are on the field.
const TANK_SPRITE_ORDER: [i32; 12] = [1, 0, 4, 2, 7, 3, 9, 5, 10, 6, 8, 11];

/// How long the mission banner fades once the opening freeze ends.
pub const INTRO_FADE_SECONDS: f32 = 0.6;

/// Which chassis row the player spawns in, most specific source first:
/// `--tank` (`Game::player_row_override`), then the `player_tank` tuning
/// knob when it isn't the -1 "unset" default, then the loaded map's own
/// `tank` key, and finally `random` - the round RNG's `0..TANK_VARIANTS`
/// roll, which is only drawn when nothing above set a chassis, so a map or
/// knob choice doesn't shift the seeded RNG stream.
///
/// The knob sits under `--tank` because a flag is typed for one specific
/// run, and over the map because a live drag has to beat a file to be worth
/// dragging - the web build, which has no command line at all, is the
/// panel's home. Whichever source wins is clamped to a real row: every
/// transport already range-checks the knob through `Tuning::set`, and the
/// clamp is the cheap guarantee that nothing here can index the sprite
/// sheet's row tables out of bounds.
fn resolve_player_row(cli: Option<i32>, knob: i32, map: Option<TankKind>, random: impl FnOnce() -> i32) -> i32 {
    let chosen = cli
        .or_else(|| (knob >= 0).then_some(knob))
        .or_else(|| map.map(TankKind::row));
    match chosen {
        Some(row) => row.clamp(0, TANK_VARIANTS - 1),
        None => random(),
    }
}

#[derive(Default)]
pub struct Game {
    /// Every entity: tanks (a `Tank`; enemies also carry an `Ai`, which is
    /// what distinguishes them - the player never does), the `Frog`,
    /// `Obstacle`s, `Pickup`s and in-flight `Shell`/`Bullet`/`Plasma`
    /// projectiles. `pub(crate)` so `render` and the map linter can read it.
    pub(crate) world: hecs::World,
    /// One entity per seat, indexed by owner slot: seat 0 is player 1 and
    /// is `None` only before the first `init`, seats 1.. hold a tank only
    /// while `players` reaches them. Every seat is spawned into the same
    /// archetype - a `Tank` and nothing else - so every query that tells
    /// enemies apart by their `Ai` sees all of them the same way.
    pub(crate) seats: [Option<Entity>; MAX_SEATS],
    /// The player's frog - `None` in a mission without one (`Destroy`),
    /// and before the first `init`.
    pub(crate) frog: Option<Entity>,
    /// The enemy side's frog (`Hunt` mission only), else `None`.
    pub(crate) enemy_frog: Option<Entity>,
    /// What ends this round - resolved by `init` from `level_overrides`
    /// and the map's `[mission]` table (docs/maps-to-levels.md).
    pub mission: Mission,
    /// How enemies arrive this round - resolved by `init` like `mission`.
    pub spawn_plan: SpawnPlan,
    /// How many enemies the band plan placed at init. Zero is a sandbox
    /// round (`tanks = 0` / `--enemies 0`): nothing to wreck, so it never
    /// ends by wreck count - only by the player's or the frog's death.
    pub(crate) band_enemy_count: usize,
    /// The wave scheduler (`waves.rs`): idle under the band plan apart
    /// from handing out owner slots.
    wave: WaveState,
    /// Seconds the round stays frozen behind the opening mission banner.
    /// `update` ticks nothing else while it is positive; a move or fire
    /// input ends it early.
    pub(crate) intro_timer: f32,
    /// Seconds left of the banner's fade-out once the freeze ended; purely
    /// visual (`render`).
    pub(crate) intro_fade: f32,
    /// Whether `init` starts the round behind the mission banner. On for
    /// the windowed game; off for headless callers (probe, tests, the dev
    /// server's `restart` unless asked), whose frame counts must not
    /// include a frozen intro.
    pub show_intro: bool,
    /// Counts down only while fewer pickups are live than the map has
    /// slots; held at PICKUP_RESPAWN_SECONDS while the field is full, so
    /// the first respawn after a collection waits the full delay.
    pickup_respawn_timer: f32,
    /// Decorative grass/dirt/road layer (`ground::build`), rebuilt each
    /// round; drawn first by `render`.
    pub(crate) ground: crate::ground::GroundGrid,
    /// The map's water as the rules see it (docs/water.md): what is deep
    /// (a wall to hulls, spawned as static colliders in `init`), what is a
    /// ford, where the current runs. Built from the map's cells at the top
    /// of `init`, before anything is placed.
    pub(crate) water: crate::ground::WaterLayout,
    /// Fading tread marks, oldest first. Kept out of `world`: pure visual
    /// trail data nothing ever queries alongside another component.
    pub(crate) tracks: Vec<Track>,
    /// Seconds since the round started; drives animation. Read by `render`.
    pub(crate) time: f32,
    pub(crate) outcome: Outcome,
    /// Seconds until the automatic restart once the round has ended.
    pub(crate) restart_timer: f32,
    /// Shared enemy "last known player position", refreshed every frame any
    /// enemy has the player within ENEMY_VIEW_RANGE and cleared once
    /// `alert_timer` runs out - see `ai::Ai::think`'s `alert` parameter.
    alert_position: Option<Position>,
    alert_timer: f32,
    /// Engagement-slot assignment with per-tank memory - see `engage`. One
    /// ring per seat, indexed like `seats`; a ring past the seats this
    /// round holds stays empty.
    engage: [EngageRing; MAX_SEATS],
    /// The second ring, around the player's frog: what hunters with a live
    /// quarry compete on (`enemy_phase`).
    engage_frog: EngageRing,
    /// Kill/blast ripples currently playing, at most `SHOCK_MAX`. A list
    /// rather than one slot: a chained barrel cascade sets off one per
    /// link, and last-write-wins used to mean the tank explosion that
    /// started it simply vanished. They resolve together in a single blit
    /// (see `static/shockwave.fs`).
    pub(crate) shocks: Vec<Shockwave>,
    /// Heat-haze ripples at recently fired muzzles, oldest first.
    pub(crate) muzzle_flashes: Vec<Shockwave>,
    /// Impact ripples where projectiles landed, oldest first.
    pub(crate) impact_flashes: Vec<Shockwave>,
    /// Barrel blasts still playing their fireball animation, oldest first
    /// (see `blast.rs`).
    pub(crate) blast_fx: Vec<BlastFx>,
    /// Burn marks left by barrel blasts this round, oldest first, capped at
    /// `SCORCH_MAX`.
    pub(crate) scorches: Vec<Scorch>,
    /// Rubble left where a wall tile died this round, oldest first, capped
    /// at `DECAL_MAX` (see `decal.rs`).
    pub(crate) decals: Vec<Decal>,
    /// Cells the map marked as tall grass. The concealment query
    /// (`grass::conceals`) runs against these, not against the drawn tufts:
    /// cover is a property of the ground, not of which way the wind blew.
    pub(crate) grass_cells: Vec<Position>,
    /// The tufts those cells scatter, built once per round. Purely drawn -
    /// see `grass.rs` for why this is not an `Obstacle`.
    pub(crate) grass: Vec<crate::grass::GrassTuft>,
    /// Queued ammo cook-offs from tanks that have died (and barrels that
    /// have blown): where each pops and how long until it does. Purely
    /// cosmetic (see `tick_cookoffs`).
    pub(crate) cookoffs: Vec<(Position, f32)>,
    /// Ground cells on fire - an oil drum's pool, a lit trail - with the
    /// time each has left (`props::tick_fires`). Simulation state: they
    /// hurt what drives over them, light what stands beside them and
    /// block the nav grid while they burn.
    pub(crate) fires: Vec<GroundFire>,
    /// The map's oil-trail cells that have not burnt yet, as grid cells
    /// (`CellObject::Oil`). Not solid, no nav effect until lit.
    pub(crate) oil_cells: HashSet<(i32, i32)>,
    /// The map's portal anchors as world positions, in `MapFile::portal_cells`
    /// order (docs/teleporting.md). Plain positions: a portal has no
    /// entity and no body. The network is *active* only with two or more
    /// (`portals_active`); a lone portal is kept here so tooling can list
    /// it, but nothing teleports and the round draws nothing.
    pub(crate) portals: Vec<Position>,
    /// Fuel drums in the air, launched by another blast and about to
    /// detonate where they land (`props::tick_launches`).
    pub(crate) flying_drums: Vec<FlyingDrum>,
    /// The whole-screen flash a kill or a barrel opens with: its age in
    /// seconds while one is playing (`game.rs` fades it out over
    /// `blast_screen_flash_seconds`). Explicit state rather than derived
    /// from `blast_fx`, so a cook-off never drives it and `flash_screen`
    /// can space flashes out.
    pub(crate) screen_flash: Option<f32>,
    /// Seconds until the next whole-screen flash is allowed
    /// (`blast_screen_flash_min_gap_seconds`).
    pub(crate) screen_flash_cooldown: f32,
    /// Laser beams still in their short display window, oldest first.
    pub(crate) laser_beams: Vec<LaserBeam>,
    /// The flame jets resolved this frame, reach capped by terrain -
    /// what `fx.rs` draws the stream from (`Game::flames`). Replaced
    /// every frame, empty when nobody is firing.
    pub(crate) flame_jets: Vec<FlameJet>,
    /// Flame exposure per ground cell, in seconds (`flame.rs`): grows
    /// under a stream, fades at `flame_heat_decay`, and a cell leaves the
    /// map the moment it lights or cools to nothing. Ordered so the
    /// ignition order is fixed frame to frame.
    pub(crate) heat: BTreeMap<(i32, i32), f32>,
    /// Tanks and frogs a stream touched last frame, sorted - the first
    /// frame of contact during a hold is what `Event::Hit` records.
    pub(crate) flame_contacts: Vec<Entity>,
    /// Frozen simulation plus a "PAUSED" overlay. Cleared by `init`.
    pub(crate) paused: bool,
    /// Drop shadows on/off (toggle key, and `--no-shadows` at startup).
    /// Survives restarts; `main.rs` sets the default.
    pub shadows_enabled: bool,
    /// The rapier world: tank bodies plus wall/obstacle/frog colliders.
    physics: Physics,
    /// Real time not yet consumed by a fixed physics step.
    physics_accumulator: f32,
    /// `--enemies`: pins the enemy count instead of the map's `tanks`
    /// default or a random roll. Set before the first `init`; persists
    /// across restarts.
    pub enemy_count_override: Option<usize>,
    /// `--mission`/`--spawn`/wave flags: per-run overrides of the map's
    /// level tables. Same lifetime as `enemy_count_override`.
    pub level_overrides: LevelOverrides,
    /// `--tank`: pins the player's chassis row. Same lifetime as
    /// `enemy_count_override`.
    pub player_row_override: Option<i32>,
    /// `--tank2`: player 2's chassis, over the map's `tank2` key.
    pub player2_row_override: Option<i32>,
    /// How many seats this round holds. Set before `init` and kept across
    /// restarts; `init` spawns exactly `players.count()` tanks.
    pub players: PlayerCount,
    /// `--seed`: pins the round seed, so every restart replays the
    /// identical round - the repro loop for a round the probe flagged.
    pub seed_override: Option<u64>,
    /// The seed this round actually ran with (see `round_seed()`).
    round_seed: u64,
    /// The round's single RNG stream - see the module doc. `None` only
    /// before the first `init`. `update` takes it into the frame context
    /// and puts it back at its end.
    rng: Option<SmallRng>,
    /// The battlefield map this round's static terrain comes from.
    /// `main.rs` sets it before the first `init` (`--map` or
    /// `maps/default.toml`); there is no procedural fallback.
    pub map: MapFile,
    /// This round's pickup slots from the map's `Pickup` cells; `update`
    /// tops the field back up from these same slots.
    map_pickup_slots: Vec<(Position, PickupKind)>,
    /// Last step's raw fire-key state per seat, for edge-detecting a fresh
    /// press.
    player_fire_held_last_frame: [bool; MAX_SEATS],
    /// `update` calls this round (paused frames included); reset by `init`.
    pub(crate) frame: u64,
    /// Projectiles spawned this round (`take_shot_id`); reset by `init`.
    next_shot_id: u32,
    /// What happened during the most recent `update` - see `Event`.
    pub(crate) events: Vec<Event>,
    /// What the last enemy phase's engagement-slot assignment decided (every
    /// enemy's status, slot and target, the slot table), kept for the
    /// `engage` overlay, the debug snapshot and the AI event diff.
    pub(crate) last_engage: EngageReport,
    /// The enemy command & control layer
    /// (docs/enemy-command-and-control-prd.md): sees every enemy's intent for
    /// the frame before any of them moves. Issues nothing until its producers
    /// land; `c2_enabled` is the switch.
    commander: command::Commander,
    /// Owner slots the dev server asked to kill; applied by
    /// `apply_debug_kills` at the top of the next playing frame, so the
    /// kill runs through the normal explosion/round-end path.
    pub(crate) debug_kills: Vec<usize>,
    /// Barrel positions a tool asked to set off (`debug_detonate`); drained
    /// by `apply_debug_detonations` at the top of the next playing frame so
    /// the blast runs through `damage_obstacle` like a direct hit would.
    pub(crate) debug_detonations: Vec<Position>,
    /// `render` skips the ground tileset and its edge vignette and leaves
    /// the field flat white; everything on the ground (decals, scorches,
    /// fires) still draws. For demos that want the effects on a blank sheet.
    pub plain_canvas: bool,
    /// `render` draws no player tank, ring or label. The tank still exists
    /// and simulates; only its presentation is skipped.
    pub hide_players: bool,
    /// Debug overlay switches `render` reads (dev builds only - see
    /// `game.rs`), set by the dev server's `overlays` tool or cycled by the
    /// I key. Survive restarts; all off by default.
    pub debug_overlays: Overlays,
    /// Record the AI-decision `Event`s (`AiAction`, `EngageSlot`, ...).
    /// Tooling-only: the dev server turns it on; the simulation never
    /// reads it, and off by default so a quiet frame has no events.
    pub trace_ai: bool,
}

/// Per-frame scratch state threaded through `Game::update`'s phases: the
/// frame's timing and size, the round RNG (taken out of `Game` for the
/// frame), the terrain snapshot every hit test reads, and everything
/// produced mid-frame that must be applied only once no query is active -
/// projectiles to spawn, laser shots to resolve, kills to explode, effects
/// to append.
struct Frame {
    dt: f32,
    width: f32,
    height: f32,
    rng: SmallRng,
    terrain: Terrain,
    /// Tanks destroyed this frame: (position, who the victim was).
    kills: Vec<(Position, Owner)>,
    /// Barrels detonated this frame, resolved by `explosions` alongside
    /// `kills` as one worklist (a blast that sets off another barrel or
    /// kills a tank appends to it).
    pending_blasts: Vec<props::PendingBlast>,
    blast_fx: Vec<BlastFx>,
    scorches: Vec<Scorch>,
    decals: Vec<Decal>,
    pending_shells: Vec<Shell>,
    pending_plasmas: Vec<Plasma>,
    pending_bullets: Vec<Bullet>,
    pending_missiles: Vec<Missile>,
    pending_lasers: Vec<PendingLaserShot>,
    /// Flame jets emitted this frame, one per firing nozzle
    /// (`resolve_flames` takes them).
    flame_jets: Vec<FlameJet>,
    muzzle_flashes: Vec<Shockwave>,
    impact_flashes: Vec<Shockwave>,
    shocks: Vec<Shockwave>,
    /// Whether at least one fixed physics step ran this frame.
    physics_stepped: bool,
    /// Events this frame's phases recorded; merged onto `Game::events`.
    events: Vec<Event>,
}

impl Frame {
    fn new(dt: f32, width: f32, height: f32, rng: SmallRng, terrain: Terrain) -> Self {
        Frame {
            dt,
            width,
            height,
            rng,
            terrain,
            kills: Vec::new(),
            pending_blasts: Vec::new(),
            blast_fx: Vec::new(),
            scorches: Vec::new(),
            decals: Vec::new(),
            pending_shells: Vec::new(),
            pending_plasmas: Vec::new(),
            pending_bullets: Vec::new(),
            pending_missiles: Vec::new(),
            pending_lasers: Vec::new(),
            flame_jets: Vec::new(),
            muzzle_flashes: Vec::new(),
            impact_flashes: Vec::new(),
            shocks: Vec::new(),
            physics_stepped: false,
            events: Vec::new(),
        }
    }
}

// How hard each thing that shakes the screen shakes it, relative to a tank
// dying. These are ratios between events rather than feel knobs - the
// overall amount is `camera_shake_magnitude` and `shockwave_strength` in
// tuning.rs - so they live here next to the code that decides which is
// which, and a designer turns the whole set up or down with one knob.
pub(crate) const SHOCK_KILL: f32 = 1.0;
pub(crate) const SHOCK_BARREL: f32 = 0.7;
/// A fuel drum: harder than oil, still short of a tank dying.
pub(crate) const SHOCK_FUEL: f32 = 0.9;
pub(crate) const SHOCK_FROG: f32 = 0.6;
/// Ammo cooking off inside a wreck: a faint local ring, and too weak to
/// move the camera once the shake snaps to 2px blocks - a cook-off is a
/// pop next to the hulk, not another explosion.
pub(crate) const SHOCK_COOKOFF: f32 = 0.1;
/// A rainbow shield shattering. Between a cook-off and a barrel: it is a
/// real event the whole field should feel, but it kills nobody, so it stays
/// well under a tank dying. Deliberately no `flash_screen` - the whole-screen
/// flash is reserved for kills and barrels.
pub(crate) const SHOCK_SHIELD_BREAK: f32 = 0.45;
/// A tank leaving or arriving through a portal: one small ring at each
/// end, so the eye is led from where it vanished to where it appeared.
/// Under a shield break - nothing was hurt - and no `flash_screen`.
pub(crate) const SHOCK_TELEPORT: f32 = 0.3;

impl Game {
    /// Set up a fresh round: player, map terrain, enemies, frog, pickups,
    /// ground. Also the restart path. `width`/`height` are the battlefield
    /// size in pixels.
    pub fn init(&mut self, width: f32, height: f32) {
        // The only `rand::rng()` on the simulation path: it picks the seed,
        // so an unseeded round is replayable once its seed is printed.
        let seed = self.seed_override.unwrap_or_else(|| rand::rng().random());
        self.round_seed = seed;
        let mut rng = SmallRng::seed_from_u64(seed);

        self.world = hecs::World::new();
        self.tracks.clear();
        self.time = 0.0;
        self.outcome = Outcome::Playing;
        self.restart_timer = 0.0;
        // The R-key restart is allowed while paused; a new round must not
        // start frozen.
        self.paused = false;
        self.alert_position = None;
        self.alert_timer = 0.0;
        for ring in &mut self.engage {
            ring.clear();
        }
        self.engage_frog.clear();
        self.commander.clear();
        self.pickup_respawn_timer = tuning().pickup_respawn_seconds;
        self.player_fire_held_last_frame = [false; MAX_SEATS];
        self.shocks.clear();
        self.muzzle_flashes.clear();
        self.impact_flashes.clear();
        self.blast_fx.clear();
        self.scorches.clear();
        self.decals.clear();
        self.cookoffs.clear();
        self.fires.clear();
        self.oil_cells.clear();
        self.portals.clear();
        self.flying_drums.clear();
        self.screen_flash = None;
        self.screen_flash_cooldown = 0.0;
        self.grass_cells.clear();
        self.grass.clear();
        self.laser_beams.clear();
        self.flame_jets.clear();
        self.heat.clear();
        self.flame_contacts.clear();
        self.frame = 0;
        self.next_shot_id = 0;
        self.last_engage.clear();
        self.debug_kills.clear();
        self.debug_detonations.clear();
        self.seats = [None; MAX_SEATS];
        self.frog = None;
        self.enemy_frog = None;
        self.mission = self.level_overrides.resolve_mission(&self.map.mission);
        self.spawn_plan = self.level_overrides.resolve_spawn(&self.map.spawn, self.enemy_count_override);
        self.intro_timer = if self.show_intro { tuning().mission_banner_seconds } else { 0.0 };
        self.intro_fade = 0.0;

        self.physics = Physics::new();
        self.physics_accumulator = 0.0;
        battlefield::spawn_walls(&mut self.physics, width, height);

        // --- Water (docs/water.md) ---
        // Read off the map's cells alone, before anything is placed: the
        // player's start, the enemy clearance rolls and the nav grid all
        // need to know where the deep water is. Deep cells get a static
        // collider each, seam-closed against their deep neighbours like a
        // run of wall tiles so a hull slides along a shore without
        // catching; nothing is spawned in `world`, so `hits::Terrain`
        // never sees them and every shot flies over.
        self.water = {
            let painted = |pick: fn(&CellObject) -> bool| -> Vec<Position> {
                self.map.iter_cells().filter(|(_, _, o)| pick(o)).map(|(c, r, _)| map::cell_to_world(c, r)).collect()
            };
            crate::ground::WaterLayout::build(
                width,
                height,
                &painted(|o| matches!(o, CellObject::Wall { .. } | CellObject::Road)),
                &painted(|o| matches!(o, CellObject::Water)),
            )
        };
        let deep_cells: HashSet<(i32, i32)> = self.water.deep_grid_cells().collect();
        for (gx, gy) in self.water.deep_grid_cells() {
            let half = battlefield::tile_hull_half_extent(&deep_cells, gx, gy, OBSTACLE_GRID_SIZE * 0.5);
            self.physics.spawn_static(map::cell_to_world(gx, gy), half);
        }

        // --- Player ---
        let row = resolve_player_row(self.player_row_override, tuning().player_tank, self.map.tank, || {
            rng.random_range(0..TANK_VARIANTS)
        });
        // The map's start cell, else the nearest non-wall cell to the
        // center so a wall at the center doesn't spawn the player inside it.
        let start_cell = self.map.start_cell().unwrap_or_else(|| {
            let (center_col, center_row) = map::world_to_cell(Position::new(width / 2.0, height / 2.0));
            self.map.nearest_free_cell(center_col, center_row)
        });
        let start_cell = dry_cell_near(&self.map, &self.water, start_cell);
        let mut tank = Tank {
            row,
            shell_variant: TANK_SHELL_VARIANT_BY_ROW[row as usize],
            damage_variant: rng.random_range(0..DAMAGE_VARIANTS),
            position: map::cell_to_world(start_cell.0, start_cell.1),
            owner: Owner::Player(0),
            ..Tank::default()
        };
        if rng.random_range(0.0..1.0) < tuning().spawn_shield_chance {
            tank.shield_hp = tuning().shield_capacity;
        }
        roll_track_distortion(&mut tank, &mut rng);
        // Spawn facing up (rotation 0): the Y-axis collider orientation.
        tank.body = Some(self.physics.spawn_tank(tank.position, tank.move_half_extents(false), tank.mass()));
        let center = tank.position;
        // Keeps enemies off the player's spawn point, and enemies off each
        // other so a crowded round doesn't start with tanks ramming.
        let clear = tank.size() * 2.0;
        let enemy_clear = tank.size() * 1.5;
        self.seats[PLAYER_OWNER_SLOT] = Some(self.world.spawn((tank,)));

        // --- Map terrain (walls/road/frog/pickup slots) ---
        // Before enemies, so their clearance check below sees every wall.
        let obstacle_half_extent = OBSTACLE_TEXTURE_SIZE * OBSTACLE_SCALE * OBSTACLE_HULL_FRACTION * 0.5;
        let map_spawn = battlefield::spawn_from_map(&mut self.physics, &mut self.world, &mut rng, &self.map, obstacle_half_extent);
        // The layout is final now, so every wall tile can work out which of
        // its faces are exposed. Only ever recomputed again on destruction.
        self.refresh_edge_masks();
        // Tall grass: whole cells from the map, each scattering a handful
        // of tufts. Hashed from position, so this draws no round RNG.
        self.grass_cells = map_spawn.grass_cells.clone();
        self.oil_cells = map_spawn.oil_cells.iter().copied().collect();
        self.portals = map_spawn.portal_cells.iter().map(|&(c, r)| map::cell_to_world(c, r)).collect();
        // Sorted by where each tuft is *rooted*, once and for all: tufts
        // never move, and `game.rs` merges them against the tanks in this
        // order every frame to get the depth right (a tuft rooted behind a
        // tank has to be drawn behind it). Sorting here keeps the per-frame
        // cost a merge walk instead of a sort of several hundred sprites.
        self.grass = self
            .grass_cells
            .iter()
            .flat_map(|c| crate::grass::tufts_for_cell(*c))
            .collect();
        self.grass.sort_by(|a, b| a.base.y.total_cmp(&b.base.y));
        // Deep water counts as terrain for every clearance roll below:
        // no enemy, frog or bonus spawns in a lake.
        let mut obstacle_positions = map_spawn.obstacle_positions;
        obstacle_positions.extend(self.water.deep_cells());
        let wall_positions = map_spawn.wall_positions;
        let map_road_cells = map_spawn.road_cells;
        let map_water_cells = map_spawn.water_cells;
        let map_frog_pos = map_spawn.frog_pos;
        self.map_pickup_slots = map_spawn.pickup_slots;

        // --- Enemies ---
        // Spawn in a band ENEMY_SPAWN_MARGIN_MIN..MAX of the shorter screen
        // side in from the nearest edge, clear of the player and each other.
        let short_side = width.min(height);
        let margin_min = short_side * tuning().enemy_spawn_margin_min;
        let margin_max = short_side * tuning().enemy_spawn_margin_max;
        // Band plan: `--enemies` wins, then the map's `tanks`, then a random
        // roll. A waves plan places nobody now - its scheduler rolls tanks
        // in through the gates once the intro ends.
        let enemy_count = match self.spawn_plan {
            SpawnPlan::Band { count } => count
                .or(self.map.tanks.map(|n| n as usize))
                .unwrap_or_else(|| rng.random_range(tuning().enemy_count_min..=tuning().enemy_count_max))
                // Free-form user input: the cap keeps live slots small; 0
                // is a sandbox round (see `band_enemy_count`).
                .min(31),
            SpawnPlan::Waves { .. } => 0,
        };
        self.band_enemy_count = enemy_count;
        self.wave = WaveState::new(self.first_enemy_slot() + enemy_count);

        // Terrain legality is the nav grid's `usable` test (see
        // `battlefield::enemy_spawn_legal`); the frog is not down yet, so
        // this grid is walls only - the same grid `relocate_unusable_spawns`
        // audits against below.
        let spawn_grid = self.nav_grid(width, height);

        // --- The other seats ---
        // Every draw here sits inside the loop, after the terrain and
        // before the enemies, so a single-player round's RNG stream is
        // exactly what it was without this block and a two-player one's is
        // exactly what one pass of it drew. Same rolls as player 1, in the
        // same order, one seat at a time.
        let mut others: Vec<Position> = Vec::new();
        for seat in 1..self.players.count() {
            // `--tank2` and the map's `tank2` pin seat 1; the seats past it
            // roll their chassis, which a replica's `init` re-rolls the
            // same way off the same seed.
            let (pin, map_row) = if seat == 1 { (self.player2_row_override, self.map.tank2) } else { (None, None) };
            let row = resolve_player_row(pin, -1, map_row, || rng.random_range(0..TANK_VARIANTS));
            // Seat 1 takes the map's `start2` cell (nudged off a solid
            // tile); every seat without an authored start - and seat 1 with
            // a `start2` inside player 1's clearance - takes the nearest
            // usable nav cell to player 1 that keeps a clear tank's width
            // from every seat already down, moved ashore if that lands it
            // in a lake.
            let placed: Vec<Position> = std::iter::once(center).chain(others.iter().copied()).collect();
            let position = (seat == 1)
                .then(|| self.map.start2_cell())
                .flatten()
                .map(|(col, row)| {
                    let (col, row) = self.map.nearest_free_cell(col, row);
                    map::cell_to_world(col, row)
                })
                .filter(|p| placed.iter().all(|&q| p.distance_to(q) >= clear))
                .unwrap_or_else(|| {
                    // Seat 1 keeps the plain outward walk it has always
                    // had, wall or no wall, because a seeded two-player
                    // replay is that walk's answer. The seats after it
                    // have no authored start to fall back on and no replay
                    // to keep, so they take the walk that only crosses
                    // ground a tank can drive over - `nearest_open`'s
                    // frontier goes through walls, which on a real map can
                    // drop a seat in a sealed pocket it could never leave.
                    let open = if seat == 1 {
                        spawn_grid.nearest_open(center, &placed, clear)
                    } else {
                        let (cols, rows, _) = spawn_grid.dims();
                        let free = |p: Position| placed.iter().all(|&q| p.distance_to(q) >= clear);
                        spawn_grid
                            .nearest_open_reachable(center, cols + rows, free)
                            .unwrap_or_else(|| spawn_grid.nearest_open(center, &placed, clear))
                    };
                    // The nav grid blocks deep water outright, so this only
                    // ever fires for a cell the grid still calls open while
                    // the lake reaches into it - and leaves the grid's own
                    // answer alone when it does not.
                    let cell = map::world_to_cell(open);
                    let dry = dry_cell_near(&self.map, &self.water, cell);
                    if dry == cell { open } else { map::cell_to_world(dry.0, dry.1) }
                });
            let mut tank = Tank {
                row,
                shell_variant: TANK_SHELL_VARIANT_BY_ROW[row as usize],
                damage_variant: rng.random_range(0..DAMAGE_VARIANTS),
                position,
                owner: Owner::Player(seat as u8),
                ..Tank::default()
            };
            if rng.random_range(0.0..1.0) < tuning().spawn_shield_chance {
                tank.shield_hp = tuning().shield_capacity;
            }
            roll_track_distortion(&mut tank, &mut rng);
            tank.body = Some(self.physics.spawn_tank(tank.position, tank.move_half_extents(false), tank.mass()));
            self.seats[seat] = Some(self.world.spawn((tank,)));
            others.push(position);
        }
        let clear_of_others = |pos: Position| others.iter().all(|&c| pos.distance_to(c) >= clear);

        let mut enemy_positions: Vec<Position> = Vec::with_capacity(enemy_count);
        while enemy_positions.len() < enemy_count {
            let legal = |pos: Position| {
                battlefield::enemy_spawn_legal(
                    pos,
                    width,
                    height,
                    margin_min,
                    margin_max,
                    center,
                    clear,
                    &spawn_grid,
                    &obstacle_positions,
                ) && clear_of_others(pos)
            };
            let pos = battlefield::sample_clear_position(&mut rng, width, height, margin_min, |pos| {
                legal(pos) && enemy_positions.iter().all(|&p| pos.distance_to(p) >= enemy_clear)
            })
            .unwrap_or_else(|| {
                // Attempt cap: the band is too crowded for a fully legal
                // spot. Snap one more band sample to the nearest usable
                // cell that keeps its distance from the tanks already
                // down, so the fallback can still never land in a wall.
                let sample = Position::new(
                    rng.random_range(margin_min..(width - margin_min)),
                    rng.random_range(margin_min..(height - margin_min)),
                );
                let mut avoid = enemy_positions.clone();
                avoid.push(center);
                avoid.extend(others.iter().copied());
                spawn_grid.nearest_open(sample, &avoid, enemy_clear)
            });
            let erow = TANK_SPRITE_ORDER[enemy_positions.len() % TANK_SPRITE_ORDER.len()];
            let mut enemy = roll_enemy_tank(&mut rng, erow, pos, self.first_enemy_slot() + enemy_positions.len());
            // Last of the per-enemy rolls, and skipped outright at a zero
            // share, so a mission without hunters draws exactly what a
            // Destroy round does.
            let role = roll_role(self.mission, &mut rng);
            enemy.body = Some(self.physics.spawn_tank(pos, enemy.move_half_extents(false), enemy.mass()));
            enemy_positions.push(pos);
            self.world.spawn((enemy, Ai::with_role(role)));
        }

        // Several individually-fine wall placements can still seal an
        // enemy's spawn cell; relocate those (and any spawn the fallback
        // above still left in a blocked cell), then re-read positions for
        // the frog's clearance check.
        battlefield::relocate_unusable_spawns(&mut self.physics, &mut self.world, width, height);
        enemy_positions = self
            .world
            .query::<&Tank>()
            .with::<&Ai>()
            .iter()
            .map(|t| t.position)
            .collect();

        // --- Frog (protect-objective) ---
        // A map-placed frog cell wins outright; otherwise roll a spot near
        // the player's spawn so defending both is the same early fight.
        // The placement roll runs even in a frog-less mission so the RNG
        // stream, hence every later spawn, matches across missions.
        let frog_clear = FROG_COLLIDER_HALF_EXTENT.0.max(FROG_COLLIDER_HALF_EXTENT.1) + OBSTACLE_CLEAR;
        let frog_pos = map_frog_pos.unwrap_or_else(|| {
            battlefield::sample_clear_position(&mut rng, width, height, margin_min, |pos| {
                let dist = pos.distance_to(center);
                (tuning().frog_spawn_min_dist..=tuning().frog_spawn_max_dist).contains(&dist)
                    && clear_of_others(pos)
                    && enemy_positions.iter().all(|&p| pos.distance_to(p) >= enemy_clear)
                    && obstacle_positions.iter().all(|&p| pos.distance_to(p) >= frog_clear)
            })
            .unwrap_or_else(|| {
                // Attempt cap: snap to the nearest usable nav cell clear of
                // every tank rather than accept a sample inside a wall.
                let sample = Position::new(width * 0.5, height * 0.5);
                let mut avoid = enemy_positions.clone();
                avoid.push(center);
                avoid.extend(others.iter().copied());
                spawn_grid.nearest_open(sample, &avoid, enemy_clear)
            })
        });
        let frog_variant = rng.random_range(0..crate::frog::FROG_VARIANT_DIRS.len() as i32);
        if self.mission.has_player_frog() {
            self.frog = Some(self.spawn_frog(Side::Player, frog_pos, frog_variant));
        }

        // --- Enemy frog (Hunt only) ---
        // A map-placed `enemy_frog` cell wins; otherwise roll a spot in the
        // enemy spawn band, well away from the player's frog and clear of
        // every tank and wall like the player frog's fallback. Draws RNG
        // only in a mission that has one, so every other mission's stream
        // is unchanged by this block.
        if self.mission.has_enemy_frog() {
            let min_dist = tuning().enemy_frog_spawn_min_dist;
            let pos = map_spawn.enemy_frog_pos.unwrap_or_else(|| {
                battlefield::sample_clear_position(&mut rng, width, height, margin_min, |pos| {
                    let border_dist = pos.x.min(width - pos.x).min(pos.y).min(height - pos.y);
                    border_dist <= margin_max
                        && pos.distance_to(frog_pos) >= min_dist
                        && pos.distance_to(center) >= clear
                        && clear_of_others(pos)
                        && spawn_grid.usable(pos)
                        && enemy_positions.iter().all(|&p| pos.distance_to(p) >= enemy_clear)
                        && obstacle_positions.iter().all(|&p| pos.distance_to(p) >= frog_clear)
                })
                .unwrap_or_else(|| {
                    let sample = Position::new(
                        rng.random_range(margin_min..(width - margin_min)),
                        rng.random_range(margin_min..(height - margin_min)),
                    );
                    let mut avoid = enemy_positions.clone();
                    avoid.push(center);
                    avoid.extend(others.iter().copied());
                    avoid.push(frog_pos);
                    spawn_grid.nearest_open(sample, &avoid, enemy_clear)
                })
            });
            let variant = rng.random_range(0..crate::frog::FROG_VARIANT_DIRS.len() as i32);
            self.enemy_frog = Some(self.spawn_frog(Side::Enemy, pos, variant));
        }

        // --- Pickups: every map slot spawns immediately ---
        for &(pos, kind) in &self.map_pickup_slots {
            spawn_pickup_at(&mut self.world, pos, kind);
        }
        // Bonus shields roll only once every slot is placed, so a bonus
        // can't land on a slot that hasn't spawned yet.
        for &(pos, kind) in &self.map_pickup_slots {
            if kind == PickupKind::Health {
                // The frog was created at full health a few lines up, so
                // no frog pack is ever rolled here and round setup draws
                // the RNG it always drew.
                maybe_spawn_health_slot_bonuses(&mut self.world, &self.map, pos, width, height, false, &mut rng);
            }
        }

        // --- Ground: road under every wall tile and explicit road cell
        // (props stand on plain ground) ---
        let mut road_cells = wall_positions.clone();
        road_cells.extend(map_road_cells);
        // Wall cells go in twice on purpose: folded into the road set they
        // paint dirt underfoot, and passed separately they cast the baked
        // shading that makes a wall look like it is standing on the floor
        // rather than pasted onto it.
        self.ground = crate::ground::build(width, height, rng.random(), &road_cells, &map_water_cells, &wall_positions, self.map.theme.drifts());

        self.rng = Some(rng);
        // Not cleared here: a restart mid-`update` (R key, round end) still
        // reports what that frame did before the new round's start.
        self.events.push(Event::RoundStarted {
            seed,
            enemies: enemy_count,
            mission: self.mission,
            spawn: self.spawn_plan.kind(),
        });
    }

    /// Put a fresh, full-health frog of `side` at `pos` with its static
    /// collider, returning its entity.
    pub(crate) fn spawn_frog(&mut self, side: Side, pos: Position, variant: i32) -> Entity {
        let body = self
            .physics
            .spawn_static(pos, Position::new(FROG_COLLIDER_HALF_EXTENT.0, FROG_COLLIDER_HALF_EXTENT.1));
        self.world.spawn((Frog {
            side,
            position: pos,
            health: tuning().frog_max_health,
            max_health: tuning().frog_max_health,
            variant,
            body,
            hurt_timer: 0.0,
            hop_timer: 0.0,
            hop_start: pos,
            hop_end: pos,
            hop_cooldown: 0.0,
            attack_timer: 0.0,
            attack_cooldown: 0.0,
            facing: Facing::Right,
            death_elapsed: None,
        },))
    }

    /// Step the simulation one frame: `input` is this frame's player input,
    /// `dt` its elapsed seconds, `width`/`height` the battlefield size.
    pub fn update(&mut self, input: Input, dt: f32, width: f32, height: f32) {
        debug_assert!(self.rng.is_some(), "Game::rng missing at update entry - init has not run");
        self.frame += 1;
        self.events.clear();
        if input.pause_pressed {
            self.paused = !self.paused;
        }
        if input.toggle_shadows_pressed {
            self.shadows_enabled = !self.shadows_enabled;
        }
        if input.cycle_overlays_pressed {
            self.debug_overlays = self.debug_overlays.next_preset();
        }
        // Debug restart: any time, including paused or on the end screen.
        if input.restart_pressed {
            self.init(width, height);
            return;
        }
        if self.paused {
            return;
        }
        // Opening mission banner: the world stays exactly as `init` left
        // it (no timers, no RNG) until the banner runs out or the player
        // moves/fires. Effects still animate so the screen isn't dead.
        if self.intro_timer > 0.0 {
            self.tick_effects(dt);
            let skip = input.seats[..self.players.count()].iter().any(|seat| seat.move_dir.is_some() || seat.fire);
            self.tick_intro_banner(dt, skip);
            return;
        }
        self.tick_intro_banner(dt, false);

        self.tick_effects(dt);
        let mut rng = self.rng.take().expect("rng seeded in init");
        self.time += dt;
        self.tick_timers(dt, &mut rng);
        let terrain = Terrain::build(&self.world, width, height, &self.grass_cells, &self.water);
        // One nav grid for the whole frame - labelled, priced and with a
        // flow field per shared target (`route_grid`) - passed to the
        // phases that need it rather than parked on `Frame`: a borrow
        // living there would alias every `&mut Frame` the other phases
        // take.
        let grid = self.route_grid(width, height);
        let mut f = Frame::new(dt, width, height, rng, terrain);

        if self.outcome == Outcome::Playing {
            self.apply_debug_kills(&mut f);
            self.apply_debug_detonations(&mut f);
            self.frog_phase(&mut f);
            self.pickup_phase(&mut f);
            self.portal_phase(&mut f, &grid);
            self.player_phase(input, &mut f);
            self.rollin_phase(&mut f);
            self.enemy_phase(&mut f, &grid);
            self.wave_phase(&mut f);
            self.spawn_pending(&mut f);
            self.guide_missiles(&mut f);
            self.resolve_lasers(&mut f);
            self.resolve_flames(&mut f);
            self.step_world(&mut f, true);
            self.sync_tanks_and_ram(&mut f);
            self.ram_props(&mut f);
            self.shell_vs_shell(&mut f);
            self.resolve_projectiles::<Shell>(&mut f, true);
            self.resolve_projectiles::<Bullet>(&mut f, true);
            self.resolve_projectiles::<Plasma>(&mut f, true);
            self.resolve_missiles(&mut f, true);
            self.tick_cookoffs(&mut f);
            self.tick_burns(&mut f);
            self.tick_fires(&mut f, true);
            self.tick_fuses(&mut f);
            self.tick_launches(&mut f);
            self.tick_grass(f.dt);
            self.drain_shield_breaks(&mut f);
            self.explosions(&mut f, true);
            self.despawn_wrecks(&mut f);
            self.cleanup_done();
            self.check_round_end(&mut f);
        } else {
            // Round over: the scene keeps animating (wrecks burn, in-flight
            // shots land without dealing damage, a barrel cascade finishes
            // without hurting anyone) while the restart counts down.
            // Physics doesn't step, so nothing drifts.
            self.guide_missiles(&mut f);
            self.step_world(&mut f, false);
            self.resolve_projectiles::<Shell>(&mut f, false);
            self.resolve_projectiles::<Bullet>(&mut f, false);
            self.resolve_projectiles::<Plasma>(&mut f, false);
            self.resolve_missiles(&mut f, false);
            self.tick_cookoffs(&mut f);
            self.tick_burns(&mut f);
            self.tick_fires(&mut f, false);
            self.tick_fuses(&mut f);
            self.tick_launches(&mut f);
            self.tick_grass(f.dt);
            self.explosions(&mut f, false);
            self.cleanup_done();
            self.restart_timer -= dt;
            if self.restart_timer <= 0.0 {
                self.finish_frame(f);
                self.init(width, height);
                return;
            }
        }
        self.finish_frame(f);
    }

    /// Append the frame's effects and events and put the RNG back.
    fn finish_frame(&mut self, f: Frame) {
        self.muzzle_flashes.extend(f.muzzle_flashes);
        self.impact_flashes.extend(f.impact_flashes);
        self.blast_fx.extend(f.blast_fx);
        self.scorches.extend(f.scorches);
        if self.scorches.len() > SCORCH_MAX {
            let excess = self.scorches.len() - SCORCH_MAX;
            self.scorches.drain(..excess);
        }
        self.decals.extend(f.decals);
        if self.decals.len() > DECAL_MAX {
            let excess = self.decals.len() - DECAL_MAX;
            self.decals.drain(..excess);
        }
        self.shocks.extend(f.shocks);
        if self.shocks.len() > SHOCK_MAX {
            // Evict by punch left, not by age: a cascade's little fuse pops
            // arrive after the tank explosion that set them off, and
            // oldest-first would throw away the one that matters.
            self.shocks.sort_by(|a, b| b.remaining().total_cmp(&a.remaining()));
            self.shocks.truncate(SHOCK_MAX);
        }
        self.events.extend(f.events);
        self.rng = Some(f.rng);
    }

    /// Age the shader effects (shockwave, muzzle/impact flashes, laser
    /// beams) - runs even on the end screen so nothing freezes mid-fade.
    fn tick_effects(&mut self, dt: f32) {
        self.shocks.retain_mut(|shock| {
            shock.time += dt;
            shock.time < tuning().shockwave_duration
        });
        self.muzzle_flashes.retain_mut(|flash| {
            flash.time += dt;
            flash.time < tuning().muzzle_flash_duration
        });
        self.impact_flashes.retain_mut(|flash| {
            flash.time += dt;
            flash.time < tuning().impact_flash_duration
        });
        self.blast_fx.retain_mut(|blast| {
            blast.time += dt;
            !blast.done()
        });
        if let Some(age) = &mut self.screen_flash {
            *age += dt;
            if *age >= tuning().blast_screen_flash_seconds {
                self.screen_flash = None;
            }
        }
        self.screen_flash_cooldown = (self.screen_flash_cooldown - dt).max(0.0);
        for decal in &mut self.decals {
            decal.age += dt;
        }
        for scorch in &mut self.scorches {
            scorch.age += dt;
        }
        self.laser_beams.retain_mut(|beam| !beam.tick(dt));
    }

    /// Per-entity timers: every tank's cooldowns/recharge/wreck burn,
    /// obstacles' burn, the frog's animation and in-flight hop (its static
    /// body follows `position`), and track fade.
    fn tick_timers(&mut self, dt: f32, rng: &mut SmallRng) {
        // The portal cooldown only runs while the tank is off every
        // portal: a tank that stops to fight beside its exit and drifts
        // onto it must not bounce back the moment the timer ends - it has
        // to leave and come back (docs/teleporting.md).
        let portals: &[Position] = if self.portals.len() >= 2 { &self.portals } else { &[] };
        let trigger_radius = tuning().portal_trigger_radius;
        for tank in self.world.query::<&mut Tank>().iter() {
            tank.tick_recharge(dt);
            tank.fire_cooldown = (tank.fire_cooldown - dt).max(0.0);
            tank.ram_cooldown = (tank.ram_cooldown - dt).max(0.0);
            if tank.portal_cooldown > 0.0 && !portals.iter().any(|p| p.distance_to(tank.position) <= trigger_radius) {
                tank.portal_cooldown = (tank.portal_cooldown - dt).max(0.0);
            }
            tank.hit_flash_timer = (tank.hit_flash_timer - dt).max(0.0);
            tank.speed_boost_timer = (tank.speed_boost_timer - dt).max(0.0);
            tank.tick_shield(dt);
            tank.burn_timer = (tank.burn_timer - dt).max(0.0);
            tank.wet_timer = (tank.wet_timer - dt).max(0.0);
            tank.tick_wreck(dt);
            tank.tick_minigun_spin(dt);
            tank.tick_missile_pod();
            if roll_wreck_col(tank, rng) {
                // The frame a tank becomes a wreck: stop it behaving like
                // an air-hockey puck. See `Physics::settle_wreck`.
                if let Some(body) = tank.body {
                    self.physics.settle_wreck(body, tuning().wreck_linear_damping, tuning().wreck_friction);
                }
            }
        }
        for frog in self.world.query::<&mut Frog>().iter() {
            frog.tick(dt);
            self.physics.set_position(frog.body, frog.position);
        }
        self.fade_tracks(dt);
    }

    /// Age every tread mark and drop the ones that have faded out (a
    /// wreck's are scorched and never do).
    fn fade_tracks(&mut self, dt: f32) {
        self.tracks.retain_mut(|t| !t.tick(dt));
    }

    /// The mission banner's own timer: count the freeze down - a seat
    /// moving or firing ends it outright - and hand the fade-out its
    /// seconds the moment it runs out; once the banner is gone, fade it.
    /// Display state either way, which is why a client replica runs it
    /// between the snapshots that pin `intro_timer`
    /// (`tick_presentation`, docs/online-coop-prd.md section 4.5).
    fn tick_intro_banner(&mut self, dt: f32, skip: bool) {
        if self.intro_timer > 0.0 {
            self.intro_timer = if skip { 0.0 } else { (self.intro_timer - dt).max(0.0) };
            if self.intro_timer <= 0.0 {
                self.intro_fade = INTRO_FADE_SECONDS;
            }
        } else {
            self.intro_fade = (self.intro_fade - dt).max(0.0);
        }
    }

    /// Ease every hull's drawn angles, health ring and minigun barrel
    /// toward the state it has been put in, and press the tread marks the
    /// movement since the last presentation tick left behind.
    ///
    /// A round drives these from `drive_tank_with`, `rollin_phase` and
    /// `tick_timers`, each on the tanks it moves; this is the replica's
    /// walk, over every hull the snapshots place, and `Tank::track_from`
    /// is the displacement it lays marks from. The ford rule is
    /// `drive_tank_with`'s: water takes no mark and wets the ones laid on
    /// the far bank.
    fn ease_hulls(&mut self, dt: f32) {
        let wet_seconds = tuning().water_wet_track_seconds;
        for tank in self.world.query::<&mut Tank>().iter() {
            tank.ease_visual_rotation(dt);
            tank.ease_turret_visual_rotation(dt);
            tank.ease_ring_position(dt);
            tank.tick_minigun_spin(dt);
            let depth = self.water.depth_at(tank.position);
            if depth == crate::ground::Depth::Dry {
                tank.wet_timer = (tank.wet_timer - dt).max(0.0);
            } else {
                tank.wet_timer = wet_seconds;
            }
            if let Some(before) = tank.track_from.replace(tank.position) {
                lay_tracks(&mut self.tracks, tank, before, depth);
            }
        }
    }

    /// Flicker every burning tile, without the charring that kills it -
    /// that death is the server's and travels as `ObstacleDestroyed`.
    fn tick_burn_frames(&mut self, dt: f32) {
        for obstacle in self.world.query::<&mut Obstacle>().iter() {
            obstacle.tick_burn_frame(dt);
        }
    }

    /// Stage a decal a replica made for itself, held to the same
    /// `DECAL_MAX` ceiling `finish_frame` holds a round to.
    pub(crate) fn push_decal(&mut self, decal: Decal) {
        self.decals.push(decal);
        if self.decals.len() > DECAL_MAX {
            let excess = self.decals.len() - DECAL_MAX;
            self.decals.drain(..excess);
        }
    }

    /// The cosmetic half of a frame, for a `Game` nothing ever `update`s:
    /// a client replica (docs/online-coop-prd.md section 4.5) applies the
    /// server's snapshots and calls this on every rendered frame between
    /// them, so the picture keeps moving at the client's own rate while
    /// the authority arrives at 20 Hz.
    ///
    /// Every step is one the round runs too, from inside the phase that
    /// owns it, so **a local round never calls this**: `update` does the
    /// same work in its own order and local play is byte for byte what it
    /// was. Nothing here draws RNG, steps physics or decides anything the
    /// server has already decided - hulls, health, tiles, fires, the
    /// round's scalars and every cause all arrive in a snapshot.
    ///
    /// The round clock is the one thing derived rather than sent: a room
    /// server's round runs without the mission banner, so its `time` is
    /// exactly `frame * PHYSICS_FIXED_DT`, which `net::apply` pins at
    /// every snapshot; this carries it forward in between, and holds it
    /// while a banner freezes the round the way `update` does.
    /// One tick of *only* one seat's locomotion, for a client's
    /// prediction sandbox (docs/online-coop-prd.md §4.12).
    ///
    /// This is deliberately not `update`: nothing here fires, ages,
    /// damages, spawns or draws a single number of RNG. It is the hull
    /// and the solver and nothing else, because that is the whole of what
    /// a client may predict - locomotion is a pure function of intent
    /// against a static world, which is what makes a replay land on the
    /// server's answer rather than drift from it.
    ///
    /// The caller owns a `Game` built the way `net::apply::welcome`
    /// builds a replica - same map, same seed - so the walls, the
    /// obstacles and the deep-water boxes this steps against are the
    /// server's own, by construction rather than by a second
    /// implementation that could disagree.
    pub(crate) fn predict_seat(&mut self, seat: usize, intent: Intent, dt: f32) {
        let Some(entity) = self.seats.get(seat).copied().flatten() else { return };
        // Disjoint borrows: `drive_tank` wants the solver and the hull at
        // once, and they are two fields of the same struct.
        let Game { world, physics, water, .. } = self;
        let Ok(mut tank) = world.get::<&mut Tank>(entity) else { return };
        if tank.body.is_none() {
            return;
        }
        let footing = Footing::at(water, tank.position);
        drive_tank(physics, &mut tank, intent, dt, footing);
        physics.step();
        // The solver moved the body; the tank's own position is what
        // every reader (and the next tick's `Footing`) goes by.
        if let Some(handle) = tank.body {
            tank.position = physics.position(handle);
        }
    }

    /// Put one seat's hull where an authority says it is, for the reset a
    /// reconciliation starts from (docs/online-coop-prd.md §4.12).
    ///
    /// Position, facing and velocity together: a replay that starts from
    /// the right place with the wrong momentum diverges within a few
    /// ticks, because the drive model reads the body's velocity at the
    /// top of every step.
    ///
    /// Only the pose. Everything else about a seat - its buffs, its
    /// weapon, what it has collected - is the server's and arrives
    /// through `net::apply`; this is the one thing a client may say
    /// about its own hull ahead of the server.
    pub(crate) fn place_seat(&mut self, seat: usize, position: Position, rotation: f32, velocity: Position) {
        let Some(entity) = self.seats.get(seat).copied().flatten() else { return };
        let Ok(mut tank) = self.world.get::<&mut Tank>(entity) else { return };
        tank.position = position;
        tank.rotation = rotation;
        if let Some(handle) = tank.body {
            self.physics.set_position(handle, position);
            self.physics.set_velocity(handle, velocity);
        }
    }

    /// Put a client's unconfirmed shot in the world under `id`, for
    /// drawing only (`net::predict`).
    ///
    /// It is an ordinary `Shell`, `Bullet` or `Plasma` entity, so every
    /// painter and the dev server's readers see it without knowing it is
    /// provisional - and `net::apply` will despawn it on the next
    /// snapshot along with anything else the server did not list, which
    /// is why the caller puts it back each frame. It never hits
    /// anything: a replica runs no hit test, and a hit is the server's
    /// word.
    pub(crate) fn add_provisional_shot(&mut self, id: u32, shot: &ProvisionalShot) {
        let owner = Owner::Player(shot.seat);
        match shot.kind {
            ProvisionalKind::Shell => {
                let shell = Shell::at(id, shot.position, shot.prev_position, shot.velocity, shot.rotation, shot.variant, shot.shooter_row, owner);
                self.world.spawn((shell,));
            }
            ProvisionalKind::Bullet => {
                let bullet = Bullet::at(id, shot.position, shot.prev_position, shot.velocity, shot.rotation, shot.shooter_row, owner);
                self.world.spawn((bullet,));
            }
            ProvisionalKind::Plasma => {
                let plasma = Plasma::at(id, shot.position, shot.prev_position, shot.velocity, shot.rotation, shot.plasma_variant, shot.shooter_row, owner);
                self.world.spawn((plasma,));
            }
        }
    }

    /// Put one seat under a speed boost, for a test that needs the
    /// server's side of one.
    #[cfg(test)]
    pub(crate) fn give_seat_boost(&mut self, seat: usize) {
        if let Some(entity) = self.seats.get(seat).copied().flatten()
            && let Ok(mut tank) = self.world.get::<&mut Tank>(entity)
        {
            tank.speed_boost_timer = tuning().speed_boost_duration_seconds;
        }
    }

    /// What one seat's trigger would fire right now: the active weapon,
    /// the ammo behind it, and the twin barrel's lateral offset (zero on
    /// a single-barrel chassis). What a client gates its own provisional
    /// shot on (`net::predict`); on a sandbox these are the server's, as
    /// of the last snapshot. `None` for a wreck or an empty seat.
    pub(crate) fn seat_arms(&self, seat: usize) -> Option<(ActiveWeapon, i32, f32)> {
        let entity = self.seats.get(seat).copied().flatten()?;
        let tank = self.world.get::<&Tank>(entity).ok()?;
        if tank.is_wreck() {
            return None;
        }
        let weapon = tank.active_weapon();
        let lateral = tuning().tank_barrel_lateral_offset[tank.row as usize];
        Some((weapon, tank.weapon_ammo(weapon), lateral))
    }

    /// A shot as one seat would fire it right now: from its muzzle,
    /// `lateral` off the centreline, `aim_offset` degrees off the barrel,
    /// at the weapon's speed.
    ///
    /// For a client drawing its own shot on the frame of the press
    /// (`net::predict`). It is `Shell::spawn` (or the bullet's, or the
    /// bolt's) and nothing else - nothing is queued, no recoil is applied
    /// and no RNG is drawn, so a sandbox stays the pure drive model it
    /// is; the shot it hands back is the caller's to carry and to throw
    /// away.
    pub(crate) fn seat_shot(&self, seat: usize, kind: ProvisionalKind, aim_offset: f32, lateral: f32) -> Option<ProvisionalShot> {
        let entity = self.seats.get(seat).copied().flatten()?;
        let tank = self.world.get::<&Tank>(entity).ok()?;
        let owner = Owner::Player(seat as u8);
        let (position, velocity, rotation, variant, plasma_variant) = match kind {
            ProvisionalKind::Shell => {
                let s = Shell::spawn(&tank, owner, aim_offset, lateral);
                (s.position, s.velocity, s.rotation, s.variant, PlasmaVariant::Teal)
            }
            ProvisionalKind::Bullet => {
                let b = Bullet::spawn(&tank, owner, aim_offset);
                (b.position, b.velocity, b.rotation, 0, PlasmaVariant::Teal)
            }
            ProvisionalKind::Plasma => {
                let p = Plasma::spawn(&tank, owner, tank.plasma_variant, aim_offset, lateral);
                (p.position, p.velocity, p.rotation, 0, p.variant)
            }
        };
        Some(ProvisionalShot {
            kind,
            seat: seat as u8,
            position,
            prev_position: position,
            velocity,
            rotation,
            variant,
            plasma_variant,
            shooter_row: tank.row,
        })
    }

    /// Show one seat's flamethrower streaming, for the frames between
    /// the local press and the server's word (`net::predict`): only if
    /// the seat is armed with it and has fuel, which is the server's own
    /// gate. `tick_presentation` draws the jet from the flag.
    pub(crate) fn hold_flame(&mut self, seat: usize) {
        let Some(entity) = self.seats.get(seat).copied().flatten() else { return };
        let Ok(mut tank) = self.world.get::<&mut Tank>(entity) else { return };
        if !tank.is_wreck() && tank.active_weapon() == ActiveWeapon::Flamethrower && tank.flame_fuel > 0.0 {
            tank.flame_held = true;
        }
    }

    /// The flame jets a replica draws, from the `flame_held` flag the
    /// wire carries: one per streaming hull, its reach capped at the
    /// first solid tile as `resolve_flames` caps it, so the cone stops
    /// at the wall here too. A local round never calls this - its jets
    /// are the phase's own.
    fn derive_flame_jets(&mut self) {
        let streaming: Vec<(Entity, Owner)> = self
            .world
            .query::<(Entity, &Tank)>()
            .iter()
            .filter(|(_, t)| t.flame_held && !t.is_wreck())
            .map(|(e, t)| (e, t.owner()))
            .collect();
        self.flame_jets.clear();
        if streaming.is_empty() {
            return;
        }
        let (width, height) = self.map.field_size();
        let terrain = Terrain::build(&self.world, width, height, &self.grass_cells, &self.water);
        for (entity, owner) in streaming {
            let Ok(tank) = self.world.get::<&Tank>(entity) else { continue };
            let jet = weapons::flame_jet(&tank, owner, entity);
            let end = Position::new(jet.origin.x + jet.dir.x * jet.range, jet.origin.y + jet.dir.y * jet.range);
            let reach = terrain.first_solid_along(jet.origin, end).map_or(jet.range, |t| t * jet.range);
            self.flame_jets.push(FlameJet { reach, ..jet });
        }
    }

    /// One seat's hull as the sandbox has it: where it is, which way it
    /// faces, and how fast it is going.
    pub(crate) fn seat_motion(&self, seat: usize) -> Option<(Position, f32, Position)> {
        let entity = self.seats.get(seat).copied().flatten()?;
        let tank = self.world.get::<&Tank>(entity).ok()?;
        let velocity = tank.body.map_or(Position::new(0.0, 0.0), |h| self.physics.velocity(h));
        Some((tank.position, tank.rotation, velocity))
    }

    pub fn tick_presentation(&mut self, dt: f32) {
        self.tick_effects(dt);
        let frozen = self.intro_timer > 0.0;
        self.tick_intro_banner(dt, false);
        if !frozen {
            self.time += dt;
        }
        self.tick_wave_banner(dt);
        self.ease_hulls(dt);
        self.fade_tracks(dt);
        self.tick_grass(dt);
        self.tick_burn_frames(dt);
        self.fade_fires(dt);
        self.fade_wrecks(dt);
        // A drum that lands blasts where the server says it did, so the
        // landed ones are dropped here and the `Blast` event carries the
        // rest.
        self.age_flying_drums(dt);
        // A streaming flamethrower is a flag on the wire; the jet the
        // glow and the particles are drawn from is rebuilt from it.
        self.derive_flame_jets();
    }

    /// Each frog on the field (the player's, then the enemy's) bites the
    /// single nearest live *hostile* tank within its attack range (never the
    /// killing blow on a player - it is a hazard, not a fair fight) and,
    /// independently, hops away from the nearest tank of any side within its
    /// wider avoid range. Each on its own cooldown.
    fn frog_phase(&mut self, f: &mut Frame) {
        for frog_entity in [self.frog, self.enemy_frog].into_iter().flatten() {
            self.frog_reflexes(f, frog_entity);
        }
    }

    /// One frog's bite and hop for this frame - see `frog_phase`.
    ///
    /// The two reflexes pick their own tank: the bite only ever lands on the
    /// other side (`Side::bites` - your own frog is an objective to defend,
    /// not a hazard to park away from), while the hop is indiscriminate, so
    /// a frog still shies away from the tanks that guard it.
    ///
    /// Both set the frog's `facing`, and the hop runs second so it wins a
    /// frame that does both - which is right, because `Frog::anim` draws the
    /// hop clip over the attack clip on exactly those frames.
    fn frog_reflexes(&mut self, f: &mut Frame, frog_entity: Entity) {
        let (side, can_attack, can_hop, frog_pos, attack_range, avoid_range, hop_distance) =
            with_frog(&self.world, frog_entity, |fr| {
                (fr.side, fr.can_attack(), fr.can_hop(), fr.position, fr.attack_range(), fr.avoid_range(), fr.hop_distance())
            });
        let live: Vec<(Entity, Position, f32, Owner)> = self
            .world
            .query::<(Entity, &Tank)>()
            .without::<&RollIn>()
            .iter()
            .filter(|(_, t)| !t.is_wreck())
            .map(|(e, t)| (e, t.position, t.position.distance_to(frog_pos), t.owner()))
            .collect();
        let by_distance = |a: &(Entity, Position, f32, Owner), b: &(Entity, Position, f32, Owner)| a.2.total_cmp(&b.2);
        let nearest_foe = live.iter().filter(|(.., owner)| side.bites(*owner)).min_by(|a, b| by_distance(a, b)).copied();
        let nearest_any = live.iter().min_by(|a, b| by_distance(a, b)).copied();

        if can_attack
            && let Some((target, target_pos, dist, _)) = nearest_foe
            && dist <= attack_range
        {
            let dmg = f.rng.random_range(tuning().frog_attack_damage_min..tuning().frog_attack_damage_max);
            // A bite never finishes a player off, whichever player it is.
            let cap = if self.is_player(target) { MAX_DAMAGE - 1.0 } else { MAX_DAMAGE };
            let (became_wreck, victim_pos, victim) = {
                let mut q = self.world.query_one::<&mut Tank>(target);
                let tank = q.get().expect("attack target always has a Tank");
                tank.take_damage(dmg, cap);
                tank.mark_hit();
                (tank.is_wreck(), tank.position, tank.owner())
            };
            f.events.push(Event::FrogBite { side, slot: victim.slot(), damage: dmg, killed: became_wreck });
            if became_wreck {
                f.kills.push((victim_pos, victim));
            }
            with_frog_mut(&self.world, frog_entity, |fr| fr.start_attack(target_pos));
        }

        if can_hop
            && let Some((_, tank_pos, dist, _)) = nearest_any
            && dist <= avoid_range
        {
            let away = Vec2::new(frog_pos.x - tank_pos.x, frog_pos.y - tank_pos.y);
            if let Some(new_pos) = frog_hop_target(&mut f.rng, frog_pos, away, hop_distance, &f.terrain, f.width, f.height) {
                with_frog_mut(&self.world, frog_entity, |fr| fr.start_hop(new_pos));
            }
        }
    }

    /// Pickups: any live tank within PICKUP_COLLECT_RADIUS collects (pure
    /// proximity, no physics), then the field is topped back up to the
    /// map's slot count after PICKUP_RESPAWN_SECONDS.
    /// Whether the player's frog is hurt enough to be worth dropping a
    /// bonus frog health pack for - the gate on that roll, checked before
    /// any RNG is drawn (docs/frog-health-pack-prd.md section 6). Only the
    /// player's frog counts: the bonus exists to give the player a way back
    /// from a hurt objective, and the packs are neutral once on the ground,
    /// so a Hunt round's enemy frog is welcome to whatever the player
    /// leaves behind but never summons one.
    fn frog_wants_a_pack(&self) -> bool {
        self.frog.is_some_and(|e| {
            with_frog(&self.world, e, |f| !f.is_dead() && f.health_fraction() < tuning().frog_pack_bonus_below)
        })
    }

    fn pickup_phase(&mut self, f: &mut Frame) {
        let living_tanks: Vec<(Entity, Position)> = self
            .world
            .query::<(Entity, &Tank)>()
            .without::<&RollIn>()
            .iter()
            .filter(|(_, t)| !t.is_wreck())
            .map(|(e, t)| (e, t.position))
            .collect();
        // A frog pack is left where it is for a tank whose own frog is
        // alive and already at full health - the one rule deciding who
        // collects one (docs/frog-health-pack-prd.md). A side whose frog is
        // absent or dead still collects, and wastes it.
        let player_frog_full = self.frog.is_some_and(|e| with_frog(&self.world, e, Frog::at_full_health));
        let enemy_frog_full = self.enemy_frog.is_some_and(|e| with_frog(&self.world, e, Frog::at_full_health));
        let own_frog_full = |e: Entity| if self.is_player(e) { player_frog_full } else { enemy_frog_full };
        let collected: Vec<(Entity, Entity, PickupKind)> = self
            .world
            .query::<(Entity, &Pickup)>()
            .iter()
            .filter_map(|(pickup_entity, pickup)| {
                living_tanks
                    .iter()
                    // An enemy only takes what it would actually use: at
                    // full health it drives over a health pack, with a full
                    // magazine over a crate, already stocked over a weapon.
                    // A pack that strips the field of everything the player
                    // needed, while gaining nothing itself, reads as spite
                    // rather than intelligence - see `Tank::wants_pickup`,
                    // which is the same predicate the behaviour tree's
                    // `seek_*` tiers gate on, so a tank can never drive to
                    // something it then refuses to pick up. Players always
                    // collect; taking what you do not strictly need is the
                    // player's call to make.
                    .filter(|&&(e, _)| with_tank(&self.world, e, |t| t.wants_pickup(pickup.kind)))
                    .filter(|&&(e, _)| pickup.kind != PickupKind::FrogHealth || !own_frog_full(e))
                    .find(|(_, pos)| pos.distance_to(pickup.position) <= tuning().pickup_collect_radius)
                    .map(|&(tank_entity, _)| (pickup_entity, tank_entity, pickup.kind))
            })
            .collect();
        for (pickup_entity, tank_entity, kind) in collected {
            let slot = {
                let mut q = self.world.query_one::<&mut Tank>(tank_entity);
                let tank = q.get().expect("collector entity always has a Tank");
                // A weapon pickup queues that weapon (FIFO, see
                // `Tank::weapon_queue`); `enqueue_weapon` must run before
                // the ammo grant. Health/Ammo/SpeedUp never touch the queue.
                match kind {
                    PickupKind::Health => tank.damage = (tank.damage - tuning().pickup_heal_amount).max(0.0),
                    PickupKind::Ammo => tank.shells_ammo += tuning().pickup_ammo_amount,
                    PickupKind::Laser => {
                        tank.enqueue_weapon(ActiveWeapon::Laser);
                        tank.laser_charges += tuning().laser_charges_per_pickup;
                        // Rerolled per pickup so a fresh batch can swap the variant.
                        tank.laser_variant = if f.rng.random_range(0.0..1.0) < tuning().laser_blue_pickup_chance {
                            LaserVariant::Blue
                        } else {
                            LaserVariant::Red
                        };
                    }
                    PickupKind::Minigun => {
                        tank.enqueue_weapon(ActiveWeapon::Minigun);
                        tank.minigun_ammo += tuning().minigun_ammo_per_pickup;
                    }
                    PickupKind::Missiles => {
                        tank.enqueue_weapon(ActiveWeapon::Missiles);
                        tank.missile_ammo += tuning().missile_ammo_per_pickup;
                    }
                    // Fuel in seconds; a second tank stacks.
                    PickupKind::Flamethrower => {
                        tank.enqueue_weapon(ActiveWeapon::Flamethrower);
                        tank.flame_fuel += tuning().flame_fuel_per_pickup;
                    }
                    PickupKind::Plasma => {
                        tank.enqueue_weapon(ActiveWeapon::Plasma);
                        tank.plasma_ammo += tuning().plasma_ammo_per_pickup;
                        tank.plasma_variant = if f.rng.random_range(0.0..1.0) < tuning().plasma_purple_pickup_chance {
                            PlasmaVariant::Purple
                        } else {
                            PlasmaVariant::Teal
                        };
                    }
                    // Refreshes rather than stacks: one boost at a time.
                    PickupKind::SpeedUp => tank.speed_boost_timer = tuning().speed_boost_duration_seconds,
                    // Full heal (damage is the health model, 0 = pristine)
                    // plus a refreshed, never stacked, invulnerability window.
                    PickupKind::Shield => {
                        tank.damage = 0.0;
                        tank.shield_hp = tuning().shield_capacity;
                    }
                    // Nothing on the tank: the frog pack heals a frog, and
                    // the frog can't be touched while this `&mut Tank`
                    // borrow is live - see just below.
                    PickupKind::FrogHealth => {}
                }
                tank.owner_slot()
            };
            if kind == PickupKind::FrogHealth {
                let frog = if self.is_player(tank_entity) { self.frog } else { self.enemy_frog };
                if let Some(frog) = frog {
                    let (side, amount, pos) = with_frog_mut(&self.world, frog, |f| {
                        let before = f.health;
                        f.heal(f.max_health * tuning().frog_pack_heal_fraction);
                        (f.side, f.health - before, f.position)
                    });
                    if amount > 0.0 {
                        f.events.push(Event::FrogHealed { side, slot, amount, x: pos.x, y: pos.y });
                    }
                }
            }
            f.events.push(Event::PickupCollected { slot, kind });
            self.world.despawn(pickup_entity).ok();
        }

        if slot_backed_count(&self.world, &self.map_pickup_slots) < self.map_pickup_slots.len() {
            self.pickup_respawn_timer -= f.dt;
            if self.pickup_respawn_timer <= 0.0 {
                let frog_hurt = self.frog_wants_a_pack();
                let respawned = respawn_from_slots(
                    &mut self.world,
                    &self.map,
                    &self.map_pickup_slots,
                    f.width,
                    f.height,
                    frog_hurt,
                    &mut f.rng,
                );
                if let Some((pos, kind)) = respawned {
                    f.events.push(Event::PickupRespawned { kind, x: pos.x, y: pos.y });
                }
                self.pickup_respawn_timer = tuning().pickup_respawn_seconds;
            }
        } else {
            self.pickup_respawn_timer = tuning().pickup_respawn_seconds;
        }
    }

    /// Portals (docs/teleporting.md): a live tank whose centre comes
    /// within `portal_trigger_radius` of a portal, with its
    /// `portal_cooldown` run out, is placed beside a *different* portal
    /// chosen uniformly at random among those with room - the nearest
    /// open cell to the exit, reached through open cells within
    /// `portal_arrival_max_cells`, outside the exit's trigger radius and
    /// clear of every other tank (wrecks included). Heading is kept,
    /// velocity zeroed (`place_tank`), the cooldown set, and the tank's
    /// AI memory and engagement slot dropped so it plans afresh. No room
    /// anywhere: nothing happens, and no RNG is drawn. Players in index
    /// order first, then enemies by slot, one draw per teleport - a round
    /// on a map with fewer than two portals is byte-identical to one with
    /// none. Runs before `step_world` so the body and `tank.position`
    /// agree when `sync_tanks_and_ram` measures travel: the jump lays no
    /// tread marks.
    fn portal_phase(&mut self, f: &mut Frame, grid: &Grid) {
        if !self.portals_active() {
            return;
        }
        let t = tuning();
        let radius = t.portal_trigger_radius;
        let max_cells = t.portal_arrival_max_cells.max(1) as usize;
        // Two big tanks on neighbouring cell centres (32 px) overlap; two
        // cells apart (64 px) they clear. The pad puts the bar between.
        let clearance = battlefield::max_tank_clearance_half_extent() * 2.0 + PATHFIND_CELL_SIZE * 0.25;
        let mut candidates: Vec<(Entity, usize, Position)> = Vec::new();
        {
            let mut visit = |entity: Entity, tank: &Tank| {
                if tank.is_wreck() || tank.body.is_none() || tank.portal_cooldown > 0.0 {
                    return;
                }
                let entrance = self
                    .portals
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (i, p.distance_to(tank.position)))
                    .filter(|&(_, d)| d <= radius)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(i, _)| i);
                if let Some(i) = entrance {
                    candidates.push((entity, i, tank.position));
                }
            };
            for player in self.seats_on_field().into_iter().flatten() {
                if let Ok(tank) = self.world.get::<&Tank>(player) {
                    visit(player, &tank);
                }
            }
            let mut enemies: Vec<(Entity, usize)> = self
                .world
                .query::<(Entity, &Tank)>()
                .with::<&Ai>()
                .iter()
                .map(|(e, tank)| (e, tank.owner_slot()))
                .collect();
            enemies.sort_by_key(|&(_, slot)| slot);
            for (entity, _) in enemies {
                if let Ok(tank) = self.world.get::<&Tank>(entity) {
                    visit(entity, &tank);
                }
            }
        }
        if candidates.is_empty() {
            return;
        }
        // Everything that occupies ground the arrival must stay clear of;
        // each teleport appends its arrival so two tanks entering on the
        // same frame never share a cell.
        let mut occupied: Vec<(Entity, Position)> = self
            .world
            .query::<(Entity, &Tank)>()
            .iter()
            .filter(|(_, tank)| tank.body.is_some())
            .map(|(e, tank)| (e, tank.position))
            .collect();
        for (entity, entrance, from) in candidates {
            let arrivals: Vec<(usize, Position)> = self
                .portals
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != entrance)
                .filter_map(|(j, &exit)| {
                    let free = |cell: Position| {
                        cell.distance_to(exit) > radius
                            && occupied.iter().all(|&(e, at)| e == entity || at.distance_to(cell) >= clearance)
                    };
                    grid.nearest_open_reachable(exit, max_cells, free).map(|at| (j, at))
                })
                .collect();
            if arrivals.is_empty() {
                continue;
            }
            let (_, to) = arrivals[f.rng.random_range(0..arrivals.len())];
            if self.place_tank(entity, to, None).is_err() {
                continue;
            }
            let slot = {
                let mut q = self.world.query_one::<&mut Tank>(entity);
                let tank = q.get().expect("placed tank exists");
                tank.portal_cooldown = t.portal_cooldown_seconds;
                tank.owner_slot()
            };
            if let Ok(mut ai) = self.world.get::<&mut Ai>(entity) {
                ai.on_teleported();
            }
            for ring in &mut self.engage {
                ring.release(entity);
            }
            self.engage_frog.release(entity);
            if let Some(o) = occupied.iter_mut().find(|(e, _)| *e == entity) {
                o.1 = to;
            }
            f.shocks.push(Shockwave::scaled(from, SHOCK_TELEPORT));
            f.shocks.push(Shockwave::scaled(to, SHOCK_TELEPORT));
            f.events.push(Event::Teleported { slot, x: from.x, y: from.y, to_x: to.x, to_y: to.y });
        }
    }

    /// Drive each seat from its own input and handle its fire key
    /// (docs/two-players.md): seat 0 first, then every further seat in
    /// index order, so their RNG draws keep a fixed order.
    fn player_phase(&mut self, input: Input, f: &mut Frame) {
        // A seat driving back in through a gate is the roll-in's, not the
        // stick's: it has no body until it arrives.
        let seats = self.seats_on_field();
        for index in 0..self.players.count() {
            let Some(entity) = seats[index] else { continue };
            self.drive_player(f, index, entity, input.seat(index));
        }
    }

    /// One seat's tank for one frame: drive it, tick its queued shots and
    /// fire on its own key. A wreck is left alone - a dead player's hulk
    /// outlives the round while a team-mate fights on, and driving it
    /// would keep shoving a corpse around.
    fn drive_player(&mut self, f: &mut Frame, index: usize, entity: Entity, intent: Intent) {
        let owner = Owner::Player(index as u8);
        let mut q = self.world.query_one::<&mut Tank>(entity);
        let tank = q.get().expect("player entity always has a Tank");
        if tank.is_wreck() {
            return;
        }

        let footing = Footing::at(&self.water, tank.position);
        drive_tank(&mut self.physics, tank, intent, f.dt, footing);
        tick_queued_shots(&mut self.physics, f, tank, owner);

        // A laser, minigun or missile pod is full-auto while the key is
        // held (still paced by `fire_cooldown` - for the pod, its reload);
        // shells and plasma fire once per physical press, so a held key can
        // never re-arm them. The
        // flamethrower is a stream: every held frame emits, with no
        // cooldown between frames at all.
        let fire_pressed = intent.fire && !self.player_fire_held_last_frame[index];
        self.player_fire_held_last_frame[index] = intent.fire;
        let weapon = tank.active_weapon();
        let should_fire = match weapon {
            ActiveWeapon::Laser | ActiveWeapon::Minigun | ActiveWeapon::Missiles | ActiveWeapon::Flamethrower => intent.fire,
            ActiveWeapon::Plasma | ActiveWeapon::Shell => fire_pressed,
        };
        if weapon == ActiveWeapon::Flamethrower {
            if should_fire {
                dispatch_fire_from(&mut self.physics, f, tank, owner, 0.0, Some(entity));
            } else {
                tank.flame_held = false;
            }
        } else if should_fire && tank.fire_cooldown <= 0.0 {
            tank.flame_held = false;
            dispatch_fire(&mut self.physics, f, tank, owner, 0.0);
        }
    }

    /// Every enemy perceives (motion snapshot, nav grid, shared alert,
    /// engagement slot, pickups, line of sight), thinks, drives and fires.
    /// With more than one seat each enemy first picks which player it is
    /// fighting (`Ai::target_player`): the nearest live, visible one,
    /// switching only past `enemy_target_switch_margin_px` so two at equal
    /// range do not flip the pack every frame. Everything downstream - the
    /// ring it competes on, its line of sight, what `think` is handed as
    /// "the player" - keys off that choice.
    fn enemy_phase(&mut self, f: &mut Frame, grid: &Grid) {
        let (movers, enemy_indices) = self.motion_snapshot();
        // Every player as the enemies see it this frame; index = player.
        struct PlayerView {
            entity: Entity,
            pos: Position,
            wreck: bool,
            concealed: bool,
            /// Driving back in through a gate, so off the field entirely.
            entering: bool,
        }
        // Built from `players()`, not `seats_on_field()`: the index is the
        // seat number that `Ai::target_player`, the engagement rings and
        // `movers[players.len()..]` below all count on.
        let entering = |entity: Entity| self.world.get::<&RollIn>(entity).is_ok();
        let players: Vec<PlayerView> = self
            .players()
            .into_iter()
            .flatten()
            .map(|entity| {
                let (pos, wreck) = with_tank(&self.world, entity, |t| (t.position, t.is_wreck()));
                PlayerView { entity, pos, wreck, concealed: f.terrain.conceals(pos), entering: entering(entity) }
            })
            .collect();

        // Retarget pass, from two seats up (a single-player round never
        // runs it, so its stream is untouched). A wreck is never a target
        // - every enemy would otherwise stand over a hulk while the rest
        // of the team fought on - and a concealed player is one only for a
        // tank they just shot, the same exemption the attack gates use.
        // The candidate is the nearest eligible seat, ties going to the
        // lower index, and the switch needs the margin only while the
        // current target is still eligible.
        if players.len() >= 2 {
            let margin = tuning().enemy_target_switch_margin_px;
            for (tank, ai) in self.world.query::<(&Tank, &mut Ai)>().iter() {
                let eligible = |p: &PlayerView| !p.wreck && !p.entering && (!p.concealed || ai.is_hit_alerted());
                let current = (ai.target_player() as usize).min(players.len() - 1);
                let mut best: Option<(usize, f32)> = None;
                for (i, p) in players.iter().enumerate().filter(|(_, p)| eligible(p)) {
                    let d = tank.position.distance_to(p.pos);
                    if best.is_none_or(|(_, bd)| d < bd) {
                        best = Some((i, d));
                    }
                }
                let Some((pick, pick_dist)) = best else { continue };
                let switch = if !eligible(&players[current]) {
                    true
                } else {
                    pick != current && pick_dist + margin < tank.position.distance_to(players[current].pos)
                };
                if switch && pick != current {
                    ai.set_target_player(pick as u8);
                    if self.trace_ai {
                        f.events.push(Event::Retarget { slot: tank.owner_slot(), player: pick as u8 });
                    }
                }
            }
        }
        let target_pos_of = |ai: &Ai| players[(ai.target_player() as usize).min(players.len() - 1)].pos;

        // Shared aggression: any enemy seeing a player refreshes the
        // group's last-known position (the nearest sighting when both
        // players are seen), so the rest converge instead of patrolling
        // blind - and, since concealment gates this too, a player who
        // stays hidden is genuinely *lost* once `enemy_alert_hold_seconds`
        // runs out, rather than merely un-shootable.
        let alert_before = self.alert_position.filter(|_| self.alert_timer > 0.0);
        let view_range = tuning().enemy_view_range;
        let mut seen: Option<(f32, Position)> = None;
        for p in players.iter().filter(|p| !p.concealed && !p.wreck && !p.entering) {
            for m in &movers[players.len()..] {
                let d = m.position.distance_to(p.pos);
                if d <= view_range && seen.is_none_or(|(best, _)| d < best) {
                    seen = Some((d, p.pos));
                }
            }
        }
        if let Some((_, pos)) = seen {
            self.alert_position = Some(pos);
            self.alert_timer = tuning().enemy_alert_hold_seconds;
        } else {
            self.alert_timer = (self.alert_timer - f.dt).max(0.0);
            if self.alert_timer <= 0.0 {
                self.alert_position = None;
            }
        }
        let alert = self.alert_position.filter(|_| self.alert_timer > 0.0);
        if self.trace_ai && alert_before.is_some() != alert.is_some() {
            let p = alert.or(alert_before).unwrap_or(players[0].pos);
            f.events.push(Event::Alert { on: alert.is_some(), x: p.x, y: p.y });
        }

        // Role targets: hunters fight the player's frog while it lives
        // (`quarry`), guards hold a leash around their own (`home`); both
        // are `None` once that frog is dead or absent, and the role then
        // behaves as `Role::Player`.
        let live_frog = |frog: Option<Entity>| {
            frog.and_then(|e| with_frog(&self.world, e, |fr| (!fr.is_dead()).then_some((e, fr.position))))
        };
        let quarry = live_frog(self.frog);
        let home = live_frog(self.enemy_frog).map(|(_, p)| p);
        let target_of = |ai: &Ai| match (ai.role, quarry) {
            (Role::Hunter, Some((frog, pos))) => (pos, Some(frog)),
            _ => (target_pos_of(ai), None),
        };
        let guard_holds = |ai: &Ai| {
            ai.role == Role::Guard && home.is_some_and(|h| target_pos_of(ai).distance_to(h) > tuning().guard_leash_px)
        };

        // Engagement slots go to tanks that are really fighting: not
        // wrecked, fleeing or retreating, and either within view range or
        // hit-alerted (`engage_status`). Merely alert-following tanks still
        // far out don't claim one - that held approaching packs in loose
        // formation through the same bottleneck (measured via the probe's
        // clustering anomaly); steering at the raw alert point is fine for
        // them. Hunters with a live quarry compete on the frog's ring;
        // everyone else on the ring around the player it is fighting - one
        // ring per seat, the rest only populated once that seat is filled.
        let mut report = EngageReport::default();
        let mut engaged: Vec<Vec<(Entity, Position)>> = vec![Vec::new(); players.len()];
        let mut engaged_frog: Vec<(Entity, Position)> = Vec::new();
        for (entity, tank, ai) in self.world.query::<(Entity, &Tank, &Ai)>().iter() {
            let (target, hunting) = target_of(ai);
            let status = if guard_holds(ai) { EngageStatus::OutOfRange } else { engage_status(tank, ai, target) };
            report.tanks.push(EngageTank::new(entity, tank.owner_slot(), status));
            if status == EngageStatus::Engaged {
                if hunting.is_some() {
                    engaged_frog.push((entity, tank.position));
                } else {
                    engaged[(ai.target_player() as usize).min(players.len() - 1)].push((entity, tank.position));
                }
            }
        }
        report.tanks.sort_by_key(|t| t.owner);
        // Sorted by entity so the greedy claim order is stable frame to frame.
        for ring in &mut engaged {
            ring.sort_by_key(|(e, _)| *e);
        }
        engaged_frog.sort_by_key(|(e, _)| *e);
        let reachable = |a: Position, b: Position| grid.connected(a, b);
        // One worst-case tank clear of the wall, plus a little.
        let margin = battlefield::max_tank_clearance_half_extent() + 8.0;
        let line_of_sight = |a: Position, b: Position| f.terrain.line_of_sight(a, b);
        if engaged[0].len() >= 2 {
            self.engage[0].assign(
                &engaged[0],
                &EngageCtx { target_pos: players[0].pos, width: f.width, height: f.height, margin, reachable: &reachable, line_of_sight: &line_of_sight },
                &mut report,
            );
        }
        for seat in 1..players.len() {
            if engaged[seat].len() < 2 {
                continue;
            }
            // A seat past the first keeps no slot table either (the
            // snapshot shows player 1's); its per-tank outcomes are merged
            // into the one report, the way the frog ring's are below.
            let mut seat_report = EngageReport {
                tanks: report.tanks.iter().filter(|t| engaged[seat].iter().any(|(e, _)| *e == t.entity)).copied().collect(),
                ..Default::default()
            };
            self.engage[seat].assign(
                &engaged[seat],
                &EngageCtx { target_pos: players[seat].pos, width: f.width, height: f.height, margin, reachable: &reachable, line_of_sight: &line_of_sight },
                &mut seat_report,
            );
            for t in seat_report.tanks {
                if let Some(slot) = report.tanks.iter_mut().find(|r| r.entity == t.entity) {
                    *slot = t;
                }
            }
        }
        // Whether a hunter can drive straight at the frog: a route to the
        // frog's own cell exists (open ground). Without one (a bunker, the
        // frog plugging a corridor) steering at the frog only jitters, so
        // such a hunter needs a ring slot even when it is the only one -
        // unlike the player ring, which a lone attacker never needs.
        let direct_route = |pos: Position, frog_pos: Position| grid.same_cell(pos, frog_pos) || grid.next_step(pos, frog_pos).is_some();
        let frog_ring_wanted = match quarry {
            Some((_, frog_pos)) => engaged_frog.len() >= 2 || engaged_frog.iter().any(|&(_, pos)| !direct_route(pos, frog_pos)),
            None => false,
        };
        if let (true, Some((frog, frog_pos))) = (frog_ring_wanted, quarry) {
            // The frog ring's own report: its slot table is not kept (the
            // snapshot shows the player ring's), its per-tank outcomes are
            // merged into the one report every reader consults.
            // Line of *fire*, not sight: a slot behind a brick wall of the
            // frog's bunker is a slot worth holding, the shells open it.
            let line_of_sight = |a: Position, b: Position| f.terrain.line_of_fire_to_frog(a, b, Some(frog));
            let mut frog_report = EngageReport {
                tanks: report.tanks.iter().filter(|t| engaged_frog.iter().any(|(e, _)| *e == t.entity)).copied().collect(),
                ..Default::default()
            };
            self.engage_frog.assign(
                &engaged_frog,
                &EngageCtx { target_pos: frog_pos, width: f.width, height: f.height, margin, reachable: &reachable, line_of_sight: &line_of_sight },
                &mut frog_report,
            );
            for t in frog_report.tanks {
                if let Some(slot) = report.tanks.iter_mut().find(|r| r.entity == t.entity) {
                    *slot = t;
                }
            }
        }
        let prev = std::mem::replace(&mut self.last_engage, report);
        if self.trace_ai {
            for t in &self.last_engage.tanks {
                let from = prev.slot_of(t.entity).map(|s| s.index() as u8);
                let to = t.slot.map(|s| s.index() as u8);
                if from != to {
                    f.events.push(Event::EngageSlot { slot: t.owner, from, to });
                }
            }
        }

        let pickups: Vec<(PickupKind, Position)> = self
            .world
            .query::<&Pickup>()
            .iter()
            .map(|p| (p.kind, p.position))
            .collect();

        // Breach perception: what a shell fired each way would hit within
        // a tile of the hull - see `Ai::think`'s `walls_ahead`.
        let breach_pad = tuning().shell_hit_half_extent;
        let breach_reach_extra = tuning().enemy_breach_reach_px;

        // --- collect pass: perception, `think`, aim and fire, exactly as
        // before. Only the impulse is deferred. ---
        let mut pending: Vec<Pending> = Vec::new();
        for (entity, tank, ai) in self.world.query::<(Entity, &mut Tank, &mut Ai)>().iter() {
            let my_index = enemy_indices[&entity];
            let reach = tank.hull_size() * 0.5 + breach_reach_extra;
            let walls_ahead = Dir::ALL.map(|d| {
                f.terrain
                    .obstacle_ahead(tank.position, d.vec(), reach, breach_pad)
                    .map(|(material, burning)| WallAhead { material, burning })
            });
            let engage_target = self.last_engage.target(entity);
            let (mut target, mut hunting) = target_of(ai);
            // A hunter that cannot route to the frog and holds no slot on
            // its ring (every slot rejected: off the map, unreachable from
            // its half of the field, or iron in the way) has no way at the
            // frog this frame unless it already stands in range with a line
            // of fire. It fights the player like everyone else instead -
            // breaching walls on the way as any tank does - and is back on
            // the frog the frame a slot opens up.
            if let Some(frog) = hunting {
                let no_slot = self.last_engage.slot_of(entity).is_none() && engage_target.is_none();
                let can_shoot_from_here = tank.position.distance_to(target) <= tuning().enemy_attack_range
                    && f.terrain.line_of_fire_to_frog(tank.position, target, Some(frog));
                if no_slot && !direct_route(tank.position, target) && !can_shoot_from_here {
                    target = target_pos_of(ai);
                    hunting = None;
                }
            }
            let frog_target = match ai.role {
                Role::Hunter => hunting.map(|_| target),
                Role::Guard => home,
                Role::Player => None,
            };
            // Concealment: a player sitting in tall grass is lost, not
            // merely un-shootable. Applied to the *target*, not to the ray -
            // grass hides what is in it rather than blocking sight through
            // it (see `Terrain::conceals`) - and it gates the shared alert
            // above, the attack and chase tiers in `ai::build`, and ram
            // damage in `sync_tanks_and_ram`. All four, because any one left
            // open lets an enemy walk to a player it cannot see: the attack
            // tier's unaligned branch repositions toward the target, so
            // gating only the shot still ends with the pack on top of you.
            //
            // **Except a tank you just shot.** `hit_alert_timer` is the
            // exemption, and it is the whole cost of the mechanic: cover
            // hides you until you use it, and then the tank you hit comes
            // looking. It is also what a range-based reveal was tried for
            // and failed to be - revealing at 96px handed enemies
            // point-blank shots that never miss, and measured *worse* for
            // the player than standing in the open.
            let fighting = &players[(ai.target_player() as usize).min(players.len() - 1)];
            let player_hidden = fighting.concealed && !ai.is_hit_alerted();
            let player_line_of_sight = !player_hidden && f.terrain.line_of_sight(tank.position, fighting.pos);
            // A hunter's fire gate is line of *fire* to the frog: only iron
            // or another frog blocks it, so a walled-in frog gets shot at
            // through its destructible walls until they are gone. Note it
            // filters on `is_permanent`, not `blocks_sight`, so concealment
            // has to be applied here separately or a hunter would keep
            // shooting a target the rest of the AI has lost.
            let line_of_sight = match hunting {
                Some(frog) => f.terrain.line_of_fire_to_frog(tank.position, target, Some(frog)),
                None => player_line_of_sight,
            };
            // Captured before anything this frame touches them, for the
            // deferred `drive_tank_with` below - see its doc comment.
            let handle = tank.body.expect("a live enemy always has a body here");
            let current = self.physics.velocity(handle);
            let facing_before = tank.rotation;
            let before = self.trace_ai.then(|| ai.snapshot());
            // A player lives in a different archetype (no `Ai`), so this
            // shared read never aliases the exclusive borrow above. `think`
            // is handed the player this tank is fighting as "the player".
            let intent = with_tank(&self.world, fighting.entity, |player_tank| {
                ai.think(
                    tank,
                    player_tank,
                    target,
                    frog_target,
                    f.width,
                    f.height,
                    f.dt,
                    &movers,
                    my_index,
                    &grid,
                    &mut f.rng,
                    alert,
                    engage_target,
                    &pickups,
                    line_of_sight,
                    player_line_of_sight,
                    player_hidden,
                    walls_ahead,
                )
            });
            if let Some(before) = before {
                ai_transition_events(&mut f.events, tank.owner_slot(), &before, &ai.snapshot());
            }
            // Aim now, drive later. `tank.control` sets the hull rotation
            // a shot flies along, so it has to happen before the shot; the
            // impulse is the only part that waits for the commander.
            tank.control(intent.move_dir, intent.face);
            let owner = tank.owner();
            tick_queued_shots(&mut self.physics, f, tank, owner);
            // The AI paces itself with its own fire timer; `fire_cooldown`
            // is the weapon's own minimum (a burst in progress, say).
            if intent.fire && tank.fire_cooldown <= 0.0 {
                dispatch_fire(&mut self.physics, f, tank, owner, intent.fire_aim_offset);
            }
            pending.push(Pending { entity, slot: tank.owner_slot(), intent, current, facing_before });
        }

        // --- command pass: no world, no RNG (see `simulation::command`) ---
        // Producers land here. Until they do the commander observes and
        // issues nothing, which is the state the rollout's byte-identical
        // checkpoints are verified in.
        let units: Vec<command::UnitView> = pending
            .iter()
            .map(|p| {
                let (position, velocity, radius, speed, wreck) = with_tank(&self.world, p.entity, |t| {
                    (
                        t.position,
                        self.physics.velocity(t.body.expect("a live enemy has a body")),
                        t.avoidance_radius(),
                        t.effective_speed(),
                        t.is_wreck(),
                    )
                });
                command::UnitView {
                    unit: comms::Unit::Enemy(p.slot),
                    slot: p.slot,
                    position,
                    velocity,
                    radius,
                    speed,
                    intent: p.intent,
                    wreck,
                    ring_rank: self.last_engage.slot_of(p.entity).map(|s| s.rank),
                }
            })
            .collect();
        // `plan` requires its input sorted by owner slot - the caller's
        // contract, as `EngageRing::assign` documents, and what makes its
        // greedy passes deterministic. The collect pass walks raw ECS order,
        // so sort a copy here rather than reordering `pending`, which must
        // keep the order the firing RNG was drawn in.
        let mut sorted = units;
        sorted.sort_by_key(|u| u.slot);
        self.commander.plan(&sorted, &command::CommandCtx {
            dt: f.dt,
            blocked: &|from, dir| grid.blocked_ahead(from, dir.vec()),
        });

        // --- apply pass: the impulse, in the order the collect pass ran ---
        // Walks `pending` rather than re-running the query. Two reasons, and
        // both are load-bearing: `motion_snapshot`'s doc comment records that
        // "a later query over the same archetype has no guaranteed iteration
        // order", and `tick_queued_shots`/`dispatch_fire` above draw from the
        // round RNG, so the order tanks are processed in is part of the
        // seeded stream. Replaying the captured order keeps it exactly what
        // it was before the split; sorting here would look tidier and would
        // shift every existing replay.
        for p in &pending {
            let intent = self.commander.apply(p.slot, p.intent);
            with_tank_mut(&self.world, p.entity, |tank| {
                let footing = Footing::at(&self.water, tank.position);
                drive_tank_with(&mut self.physics, tank, intent, f.dt, p.current, p.facing_before, footing);
            });
        }
    }

    /// Insert this frame's fired projectiles - only once no tank query is
    /// active, since hecs can't spawn into a world mid-iteration.
    fn spawn_pending(&mut self, f: &mut Frame) {
        for mut shell in f.pending_shells.drain(..) {
            shell.set_id(self.take_shot_id());
            self.world.spawn((shell,));
        }
        for mut plasma in f.pending_plasmas.drain(..) {
            plasma.set_id(self.take_shot_id());
            self.world.spawn((plasma,));
        }
        for mut bullet in f.pending_bullets.drain(..) {
            bullet.set_id(self.take_shot_id());
            self.world.spawn((bullet,));
        }
        for mut missile in f.pending_missiles.drain(..) {
            missile.set_id(self.take_shot_id());
            self.world.spawn((missile,));
        }
    }

    /// The next projectile id: one counter for shells, bullets and plasma,
    /// starting at 1 each round, never reused. No RNG.
    fn take_shot_id(&mut self) -> u32 {
        self.next_shot_id += 1;
        self.next_shot_id
    }

    /// Lasers have no travel time: each queued beam is swept over its whole
    /// length right now, drawn up to where it stopped, and applied.
    fn resolve_lasers(&mut self, f: &mut Frame) {
        let players = self.seats_on_field();
        let shots = std::mem::take(&mut f.pending_lasers);
        for shot in shots {
            let hit = f.terrain.sweep(&self.world, players, shot.owner, shot.start, shot.end, laser_beam_half_width());
            let (hit_pos, target) = match hit {
                Some((target, t)) => (shot.start + (shot.end - shot.start) * t, Some(target)),
                None => (shot.end, None),
            };
            f.muzzle_flashes.push(Shockwave::new(shot.start));
            self.laser_beams.push(LaserBeam::new(shot.start, hit_pos, shot.variant));
            let Some(target) = target else { continue };
            f.impact_flashes.push(Shockwave::new(hit_pos));
            // No knockback and no frog hop: an instant beam isn't something
            // to be shoved by or to dodge.
            self.apply_hit(f, target, hit_pos, laser_damage_range(&shot), HitEffects::none(), shot.owner);
        }
    }

    /// Advance the world in fixed PHYSICS_FIXED_DT steps: projectiles are
    /// integrated inside the same loop as the rapier step, so a frame-time
    /// hitch moves them exactly as far as it moves the tanks. Each
    /// projectile's `prev_position` is captured first, giving the hit test
    /// the whole frame's segment. `step_physics` is false on the end screen.
    fn step_world(&mut self, f: &mut Frame, step_physics: bool) {
        self.begin_projectile_frame::<Shell>();
        self.begin_projectile_frame::<Bullet>();
        self.begin_projectile_frame::<Plasma>();
        self.physics_accumulator = (self.physics_accumulator + f.dt).min(PHYSICS_MAX_CATCHUP_SECONDS);
        while self.physics_accumulator >= PHYSICS_FIXED_DT {
            self.advance_projectiles::<Shell>(PHYSICS_FIXED_DT);
            self.advance_projectiles::<Bullet>(PHYSICS_FIXED_DT);
            self.advance_projectiles::<Plasma>(PHYSICS_FIXED_DT);
            for missile in self.world.query::<&mut Missile>().iter() {
                missile.advance(PHYSICS_FIXED_DT);
            }
            if step_physics {
                self.physics.step();
                let (bodies, colliders) = self.physics.quarantined();
                if bodies > 0 || colliders > 0 {
                    f.events.push(Event::PhysicsQuarantine { bodies, colliders });
                }
            }
            self.physics_accumulator -= PHYSICS_FIXED_DT;
            f.physics_stepped = true;
        }
    }

    fn begin_projectile_frame<P: Projectile>(&mut self) {
        for p in self.world.query::<&mut P>().iter() {
            p.begin_frame();
        }
    }

    fn advance_projectiles<P: Projectile>(&mut self, dt: f32) {
        for p in self.world.query::<&mut P>().iter() {
            p.advance(dt);
        }
    }

    /// Read tank positions back from physics, lay tread marks on frames the
    /// physics actually stepped, and resolve ram damage for every enemy
    /// touching a player (`ram` gates on both cooldowns, so a player takes
    /// at most one ram hit per frame), then for the two players against
    /// each other, then enemy against enemy.
    fn sync_tanks_and_ram(&mut self, f: &mut Frame) {
        let players: Vec<(Entity, Position)> = self
            .seats_on_field()
            .into_iter()
            .flatten()
            .map(|p| (p, with_tank(&self.world, p, |t| t.position)))
            .collect();
        let enemies_before: Vec<(Entity, Position)> = self
            .world
            .query::<(Entity, &Tank)>()
            .with::<&Ai>()
            .iter()
            .map(|(e, t)| (e, t.position))
            .collect();
        for tank in self.world.query::<&mut Tank>().iter() {
            sync_tank_from_physics(&self.physics, tank);
        }
        if f.physics_stepped {
            for &(player, before) in &players {
                with_tank_mut(&self.world, player, |t| lay_tracks(&mut self.tracks, t, before, self.water.depth_at(t.position)));
            }
        }
        for (enemy, before) in enemies_before {
            for &(player, _) in &players {
                let (touching, concealed) = with_tank(&self.world, enemy, |e| {
                    with_tank(&self.world, player, |p| (tanks_touching(&self.physics, e, p), f.terrain.conceals(p.position)))
                });
                // A tank that has lost the player in grass does not get to
                // ram them either: without this, hiding traded gunfire for
                // melee and the melee hurt more.
                if touching && !concealed {
                    let rammed = with_two_tanks_mut(&mut self.world, enemy, player, |e, p| {
                        ram(e, p, &mut self.physics, &mut f.rng, &mut f.kills, 1.0)
                            .map(|damage| (p.owner_slot(), e.owner_slot(), damage))
                    });
                    if let Some((slot, other_slot, damage)) = rammed {
                        f.events.push(Event::Ram { slot, other_slot, damage });
                    }
                }
            }
            if f.physics_stepped {
                with_tank_mut(&self.world, enemy, |t| lay_tracks(&mut self.tracks, t, before, self.water.depth_at(t.position)));
            }
        }
        // The two players: friendly fire, at the same factor a shell gets.
        if let [(p1, _), (p2, _)] = players[..] {
            let touching = with_tank(&self.world, p1, |a| with_tank(&self.world, p2, |b| tanks_touching(&self.physics, a, b)));
            if touching {
                let factor = tuning().friendly_fire_damage_factor;
                let rammed = with_two_tanks_mut(&mut self.world, p1, p2, |a, b| {
                    ram(a, b, &mut self.physics, &mut f.rng, &mut f.kills, factor)
                        .map(|damage| (a.owner_slot(), b.owner_slot(), damage))
                });
                if let Some((slot, other_slot, damage)) = rammed {
                    f.events.push(Event::Ram { slot, other_slot, damage });
                }
            }
        }
        self.ram_enemy_pairs(f);
    }

    /// Enemies ram each other too, not just the player.
    ///
    /// This used to be deliberately absent, which meant a six-tank pileup
    /// produced no damage, no event and no feedback at all - the one
    /// collision in the game that did nothing.
    ///
    /// Two things matter here. `ram` draws from the round RNG, so the pair
    /// order has to be fixed: the list is sorted by owner slot and walked
    /// as `i < j`, never in ECS iteration order. And it is O(n^2) in live
    /// enemies, so a cheap distance test comes before the narrow-phase
    /// query, the same way `ram_props` culls.
    fn ram_enemy_pairs(&mut self, f: &mut Frame) {
        let mut enemies: Vec<(Entity, usize, Position)> = self
            .world
            .query::<(Entity, &Tank)>()
            .with::<&Ai>()
            .iter()
            .filter(|(_, t)| !t.is_wreck())
            .map(|(e, t)| (e, t.owner_slot(), t.position))
            .collect();
        enemies.sort_by_key(|(_, slot, _)| *slot);

        // Two tanks can only touch if their centres are within a hull
        // diagonal of each other; anything further apart cannot be in
        // contact and is not worth a narrow-phase query.
        let reach = TANK_TEXTURE_SIZE * 2.0;
        for i in 0..enemies.len() {
            for j in (i + 1)..enemies.len() {
                let (a, _, a_pos) = enemies[i];
                let (b, _, b_pos) = enemies[j];
                if a_pos.distance_to(b_pos) > reach {
                    continue;
                }
                let touching = with_tank(&self.world, a, |x| {
                    with_tank(&self.world, b, |y| tanks_touching(&self.physics, x, y))
                });
                if !touching {
                    continue;
                }
                let rammed = with_two_tanks_mut(&mut self.world, a, b, |x, y| {
                    ram(x, y, &mut self.physics, &mut f.rng, &mut f.kills, 1.0)
                        .map(|damage| (x.owner_slot(), y.owner_slot(), damage))
                });
                if let Some((slot, other_slot, damage)) = rammed {
                    f.events.push(Event::Ram { slot, other_slot, damage });
                }
            }
        }
    }

    /// Two flying shells from opposing sides that meet mid-air detonate
    /// each other. Swept (closest approach over this frame's motion) rather
    /// than an end-of-frame overlap, since at SHELL_SPEED two shells closing
    /// head-on can pass through each other between frames. Same-side pairs
    /// (a twin volley, two different enemies' shells) never cancel.
    fn shell_vs_shell(&mut self, f: &mut Frame) {
        let flying: Vec<(Entity, Position, Vec2, Owner)> = self
            .world
            .query::<(Entity, &Shell)>()
            .iter()
            .filter(|(_, s)| s.state == ShellState::Flying)
            .map(|(e, s)| (e, s.prev_position, s.position - s.prev_position, s.owner))
            .collect();
        let collide_dist = tuning().shell_hit_half_extent * 2.0;
        let mut claimed: HashSet<Entity> = HashSet::new();
        let mut collisions: Vec<(Entity, Entity, Position)> = Vec::new();
        for i in 0..flying.len() {
            let (e1, prev1, disp1, owner1) = flying[i];
            if claimed.contains(&e1) {
                continue;
            }
            for &(e2, prev2, disp2, owner2) in &flying[i + 1..] {
                if owner1.same_side(owner2) || claimed.contains(&e2) {
                    continue;
                }
                // position(t) = prev + disp * t, t in [0, 1]; closest approach.
                let rel_pos = prev1 - prev2;
                let rel_disp = disp1 - disp2;
                let denom = rel_disp.dot(rel_disp);
                let t = if denom > 0.0 {
                    (-rel_pos.dot(rel_disp) / denom).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let c1 = prev1 + disp1 * t;
                let c2 = prev2 + disp2 * t;
                if c1.distance_to(c2) <= collide_dist {
                    collisions.push((e1, e2, Position::new((c1.x + c2.x) * 0.5, (c1.y + c2.y) * 0.5)));
                    claimed.insert(e1);
                    claimed.insert(e2);
                    break;
                }
            }
        }
        for (e1, e2, midpoint) in collisions {
            f.impact_flashes.push(Shockwave::new(midpoint));
            f.events.push(Event::ShellsCollided { x: midpoint.x, y: midpoint.y });
            for e in [e1, e2] {
                let mut q = self.world.query_one::<&mut Shell>(e);
                q.get().expect("shell collected this frame still exists").detonate();
            }
        }
    }

    /// Resolve every flying projectile of one type against the terrain
    /// snapshot and the tanks: sweep its frame segment, flash at the entry
    /// point, ricochet if it can (shells off Iron), otherwise detonate at
    /// that point and - when `live` - apply damage/knockback/frog hop.
    fn resolve_projectiles<P: Projectile>(&mut self, f: &mut Frame, live: bool) {
        let players = self.seats_on_field();
        struct Flight {
            entity: Entity,
            prev: Position,
            pos: Position,
            vel: Vec2,
            owner: Owner,
            dmg: (f32, f32),
        }
        let flying: Vec<Flight> = self
            .world
            .query::<(Entity, &P)>()
            .iter()
            .filter(|(_, p)| p.is_flying())
            .map(|(entity, p)| Flight {
                entity,
                prev: p.prev_position(),
                pos: p.position(),
                vel: p.velocity(),
                owner: p.owner(),
                dmg: p.damage_range(),
            })
            .collect();
        for Flight { entity, prev, pos, vel, owner, dmg } in flying {
            // Sweep, and re-sweep past any prop tile the projectile rolls a
            // pass-over on (`Material::pass_over_chance`) - remembered on
            // the projectile, since a segment ending inside the tile would
            // otherwise re-roll it next frame. Bounded by the props along
            // the segment. A zero chance draws no RNG.
            let mut ignored: Vec<Entity> = {
                let mut q = self.world.query_one::<&P>(entity);
                q.get().expect("projectile collected this frame still exists").passed_over().to_vec()
            };
            let hit = loop {
                let swept = f.terrain.sweep_ignoring(&self.world, players, owner, prev, pos, P::hit_half_extent(), &ignored);
                let Some((target, t)) = swept else { break None };
                if let ShellTarget::Obstacle(e) = target {
                    let chance = f.terrain.obstacle(e).map_or(0.0, |b| b.material.pass_over_chance());
                    if chance > 0.0 && f.rng.random_bool(chance) {
                        ignored.push(e);
                        let mut q = self.world.query_one::<&mut P>(entity);
                        q.get().expect("projectile collected this frame still exists").note_passed_over(e);
                        continue;
                    }
                }
                break Some((target, t));
            };
            let Some((target, t)) = hit else {
                continue;
            };
            let hit_pos = prev + (pos - prev) * t;
            f.impact_flashes.push(Shockwave::new(hit_pos));
            // A shielded tank bounces the projectile away instead of taking
            // the hit - see `Projectile::deflect`. No damage, no knockback,
            // no hit flash on the tank; the impact flash above still shows
            // where the shield was struck.
            //
            // This is the shield's *other* spending seam. A projectile never
            // reaches `Tank::take_damage`, so the charge has to come off
            // here, and it costs `shield_deflect_cost_factor` times the
            // shot's own mid-range damage rather than 1x: bouncing a shell
            // back under your ownership is the strongest thing a shield
            // does, so it should also be the fastest way to spend one.
            // `dmg` is the projectile's `damage_range()` (tuning only), so
            // this draws no RNG and the seeded replay stream is untouched.
            let shield = match target {
                ShellTarget::Tank(e) => shield_deflector(&self.world, e).map(|(c, o)| (e, c, o)),
                _ => None,
            };
            if let Some((shielded, center, new_owner)) = shield {
                let cost = (dmg.0 + dmg.1) * 0.5 * tuning().shield_deflect_cost_factor;
                // The break itself is latched on the tank and announced by
                // `drain_shield_breaks`, along with every other seam's.
                with_tank_mut(&self.world, shielded, |t| t.spend_shield(cost));
                let mut q = self.world.query_one::<&mut P>(entity);
                q.get().expect("projectile collected this frame still exists").deflect(center, new_owner);
                f.events.push(Event::Deflected { slot: new_owner.slot(), x: hit_pos.x, y: hit_pos.y });
                continue;
            }
            let bounced = {
                let mut q = self.world.query_one::<&mut P>(entity);
                let p = q.get().expect("projectile collected this frame still exists");
                let bounced = match target {
                    ShellTarget::Obstacle(e) => f.terrain.obstacle(e).is_some_and(|b| {
                        // The projectile's own rule (shells off Iron), or a
                        // barrel's chance deflection. Zero chance draws no RNG.
                        p.try_ricochet(b) || {
                            let chance = b.material.deflect_chance();
                            P::can_bounce() && chance > 0.0 && f.rng.random_bool(chance) && {
                                p.reflect_off(b);
                                true
                            }
                        }
                    }),
                    _ => false,
                };
                bounced.then(|| p.heading())
            };
            if let Some(heading) = bounced {
                f.events.push(Event::Ricochet { slot: owner.slot(), x: hit_pos.x, y: hit_pos.y, heading });
                continue;
            }
            {
                let mut q = self.world.query_one::<&mut P>(entity);
                let p = q.get().expect("projectile collected this frame still exists");
                p.set_position(hit_pos);
                p.detonate();
            }
            if !live {
                continue;
            }
            let len = (vel.x * vel.x + vel.y * vel.y).sqrt().max(f32::EPSILON);
            let dir = Vec2::new(vel.x / len, vel.y / len);
            let effects = HitEffects {
                knockback: P::knockback_speed().map(|speed| (dir, speed)),
                frog_hop: P::frog_hops().then_some(vel),
                travel: Some(dir),
            };
            self.apply_hit(f, target, hit_pos, dmg, effects, owner);
        }
    }

    /// Every tank killed this frame gets a shockwave and an explosion, and
    /// every barrel detonated this frame its blast; a splash that kills
    /// another tank, or a blast that finishes a barrel's fuse elsewhere, is
    /// appended and handled in turn. Processed in order, so the last ring
    /// shown is the most recent one's. Terminates: a tank can only ever be
    /// pushed once (every push is gated by its own transition into a wreck)
    /// and a barrel dies once. `live` is false on the end screen, where
    /// blasts play out without damage (no kills happen there).
    /// Everything a dying tank throws off: the same fireball, screen flash
    /// and scorch a barrel gets, plus its own wreckage and a set of
    /// delayed pops.
    ///
    /// Draws no RNG: the parts' landing spots, cells and arcs all come out
    /// of `blast::seed_at` salted per piece, so a spectacular death cannot
    /// shift a seeded replay.
    fn wreck_fx(&mut self, f: &mut Frame, center: Position) {
        f.blast_fx.push(BlastFx::wreck(center));
        f.impact_flashes.push(Shockwave::new(center));
        self.flash_screen();
        if self.water.depth_at(center) == crate::ground::Depth::Dry {
            f.scorches.push(Scorch::new(center));
        }
        self.scorch_tracks(center);

        let throw = tuning().wreck_part_throw_px;
        for i in 0..tuning().wreck_parts.max(0) as u32 {
            // Fan the pieces around the hull by hashed angle and distance
            // rather than a fixed rosette, so two wrecks never scatter the
            // same way.
            let h = crate::blast::seed_at(center, 40 + i * 5);
            let angle = (h % 3600) as f32 / 3600.0 * std::f32::consts::TAU;
            let dist = throw * (0.35 + 0.65 * ((h >> 12) % 100) as f32 / 100.0);
            let to = Position::new(center.x + angle.cos() * dist, center.y + angle.sin() * dist);
            f.decals.push(Decal::thrown(RUBBLE_ROW_TANK, center, to, 40 + i * 5));
        }

        // Ammo cooking off: a few small pops after the fact, spread over
        // `cookoff_window_seconds`. Queued rather than fired now, and
        // ticked by `tick_cookoffs`.
        let count = tuning().cookoff_count.max(0) as u32;
        let window = tuning().cookoff_window_seconds;
        for i in 0..count {
            let h = crate::blast::seed_at(center, 90 + i * 7);
            let delay = window * (0.15 + 0.85 * (h % 1000) as f32 / 1000.0);
            let off = ((h >> 10) % 21) as f32 - 10.0;
            let off2 = ((h >> 16) % 21) as f32 - 10.0;
            self.cookoffs.push((Position::new(center.x + off, center.y + off2), delay));
        }
    }

    /// Burn a wreck's last few tread marks into the ground. Walks back
    /// from the newest mark rather than scanning the whole list, and stops
    /// after `wreck_track_marks`, so the cost is bounded by the number of
    /// marks burnt and not by how long the round has been running.
    fn scorch_tracks(&mut self, center: Position) {
        let reach = crate::TANK_TEXTURE_SIZE;
        let mut left = tuning().wreck_track_marks.max(0);
        for track in self.tracks.iter_mut().rev() {
            if left == 0 {
                break;
            }
            if track.position.distance_to(center) <= reach {
                track.scorched = true;
                left -= 1;
            }
        }
    }

    /// Start the whole-screen flash a kill or a barrel opens with, unless
    /// one played within `blast_screen_flash_min_gap_seconds`: a barrel
    /// chain or a multi-kill reads as one flash rather than a strobe.
    /// Cook-offs never call this.
    pub(crate) fn flash_screen(&mut self) {
        if self.screen_flash_cooldown > 0.0 {
            return;
        }
        self.screen_flash = Some(0.0);
        self.screen_flash_cooldown = tuning().blast_screen_flash_min_gap_seconds;
    }

    /// Count down the queued cook-off pops and fire the ones that are due.
    /// Cosmetic only - a secondary never damages anything, because a kill
    /// has already resolved its blast and a second helping of splash would
    /// be a real balance change rather than a detail. It is also local:
    /// a small fireball, a faint ripple and sparks, with none of the
    /// screen-level effects (flash, impact quad, shake) a real kill has.
    fn tick_cookoffs(&mut self, f: &mut Frame) {
        let mut popped = Vec::new();
        self.cookoffs.retain_mut(|(pos, t)| {
            *t -= f.dt;
            if *t > 0.0 {
                return true;
            }
            popped.push(*pos);
            false
        });
        for center in popped {
            f.blast_fx.push(BlastFx::small(center));
            f.shocks.push(Shockwave::scaled(center, SHOCK_COOKOFF));
            f.events.push(Event::CookOff { x: center.x, y: center.y });
        }
    }

    fn explosions(&mut self, f: &mut Frame, live: bool) {
        let (mut i, mut j) = (0, 0);
        while i < f.kills.len() || j < f.pending_blasts.len() {
            while i < f.kills.len() {
                let (center, victim) = f.kills[i];
                i += 1;
                f.events.push(Event::Wreck { slot: victim.slot(), x: center.x, y: center.y });
                f.shocks.push(Shockwave::scaled(center, SHOCK_KILL));
                self.wreck_fx(f, center);
                self.apply_explosion(f, center, victim);
            }
            if j < f.pending_blasts.len() {
                let blast = f.pending_blasts[j];
                j += 1;
                self.apply_blast(f, blast, live);
            }
        }
    }

    /// Despawn projectiles whose impact animation has finished and
    /// obstacles destroyed this frame (their physics body goes too).
    fn cleanup_done(&mut self) {
        self.despawn_done::<Shell>();
        self.despawn_done::<Bullet>();
        self.despawn_done::<Plasma>();
        let destroyed: Vec<_> = self
            .world
            .query::<(Entity, &Obstacle)>()
            .iter()
            .filter(|(_, o)| o.destroyed)
            .map(|(e, o)| (e, o.body))
            .collect();
        let removed = !destroyed.is_empty();
        for (entity, body) in destroyed {
            self.physics.remove_body(body);
            self.world.despawn(entity).ok();
        }
        if removed {
            // A wall run just lost a tile, so its neighbours have newly
            // exposed faces to cap.
            self.refresh_edge_masks();
        }
    }

    /// Recompute every wall tile's cached edge mask.
    ///
    /// Called once after the battlefield is laid out and again whenever a
    /// tile is destroyed, which is the only way a wall layout ever changes
    /// - tiles are never added mid-round. That is why the mask is cached on
    /// `Obstacle` at all: `fence_axis` rebuilds its neighbour set every
    /// frame inside `render`, and doing that for several hundred wall tiles
    /// would be per-frame work for something that changes a handful of
    /// times a round.
    ///
    /// Rebuilds all of them rather than patching the dead tile's eight
    /// neighbours: destruction is rare, the pass is one HashSet build plus
    /// a few lookups per tile, and a full rebuild cannot drift out of sync
    /// the way an incremental update can.
    pub(crate) fn refresh_edge_masks(&mut self) {
        let cells: HashSet<(i32, i32)> = self
            .world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| !o.destroyed && o.material.is_wall())
            .map(|o| o.cell())
            .collect();
        for o in self.world.query::<&mut Obstacle>().iter() {
            if !o.material.is_wall() {
                continue;
            }
            o.edge_mask = neighbour_mask(o.cell(), &cells);
        }
    }

    fn despawn_done<P: Projectile>(&mut self) {
        let done: Vec<Entity> = self
            .world
            .query::<(Entity, &P)>()
            .iter()
            .filter(|(_, p)| p.is_done())
            .map(|(e, _)| e)
            .collect();
        for entity in done {
            self.world.despawn(entity).ok();
        }
    }

    /// Per-mission end rules (docs/maps-to-levels.md). Losing (player or
    /// the player's frog dead) takes precedence over winning when both
    /// happen on the same frame.
    fn check_round_end(&mut self, f: &mut Frame) {
        // Lost once every human tank is a wreck - the one player's in a
        // single-player round, every seat's in a team round - or the frog
        // dies. `players()`, not `seats_on_field()`: a seat driving back
        // in through a gate is alive and the round goes on, and a seat
        // waiting for the next wave to bring it back (decision 7,
        // docs/online-coop-prd.md section 4.11) is a wreck like any other
        // - one wreck of one seat still ends a solo round, and a team that
        // falls together still loses.
        let players_dead = self
            .players()
            .into_iter()
            .flatten()
            .all(|e| with_tank(&self.world, e, |t| t.is_wreck()));
        let frog_dead = |frog: Option<Entity>| frog.is_some_and(|e| with_frog(&self.world, e, Frog::is_dead));
        if players_dead || frog_dead(self.frog) {
            self.end_round(f, Outcome::Lost);
            return;
        }
        // A band round with no enemies at all (a sandbox) has nothing to
        // win by wrecking; it runs until the player or the frog dies.
        let sandbox = matches!(self.spawn_plan, SpawnPlan::Band { .. }) && self.band_enemy_count == 0;
        let won = match self.mission {
            Mission::Hunt => frog_dead(self.enemy_frog),
            Mission::Protect | Mission::Destroy => !sandbox && self.spawn_plan_finished() && self.all_enemies_wrecked(),
        };
        if won {
            self.end_round(f, Outcome::Won);
        }
    }

    /// No enemy is still to come: every wave has rolled in and nobody is
    /// still entering. Always true under the band plan.
    fn spawn_plan_finished(&self) -> bool {
        match self.spawn_plan {
            SpawnPlan::Band { .. } => true,
            SpawnPlan::Waves { .. } => self.waves_finished(),
        }
    }

    fn all_enemies_wrecked(&self) -> bool {
        self.world.query::<&Tank>().with::<&Ai>().iter().all(|t| t.is_wreck())
    }

    fn end_round(&mut self, f: &mut Frame, outcome: Outcome) {
        self.outcome = outcome;
        self.restart_timer = tuning().restart_delay;
        f.events.push(Event::RoundEnded { outcome });
    }

    /// Every tank's motion for the AI's predictive avoidance: the players
    /// first, in index order (so index `i` is player `i`), then the
    /// enemies, plus a map from enemy entity to index (a later query over
    /// the same archetype has no guaranteed iteration order). Wrecks are
    /// included at zero velocity so tanks steer around them as fixed
    /// obstacles.
    fn motion_snapshot(&self) -> (Vec<Mover>, HashMap<Entity, usize>) {
        let to_mover = |t: &Tank| Mover {
            position: t.position,
            velocity: if t.is_wreck() { Position::new(0.0, 0.0) } else { t.velocity },
            radius: t.avoidance_radius(),
            is_player: t.is_player(),
        };
        let mut movers = Vec::new();
        for player in self.players().into_iter().flatten() {
            with_tank(&self.world, player, |t| movers.push(to_mover(t)));
        }
        let mut enemy_indices = HashMap::new();
        for (entity, tank) in self.world.query::<(Entity, &Tank)>().with::<&Ai>().iter() {
            enemy_indices.insert(entity, movers.len());
            movers.push(to_mover(tank));
        }
        (movers, enemy_indices)
    }

    /// The obstacle-occupancy grid the AI routes by this frame (see
    /// `pathfind::Grid`), rebuilt each frame from the current terrain and
    /// built exactly the same way by the map linter, so the two can't
    /// drift. The margin is the worst-case tank in the roster, so no route
    /// is too narrow for a titan. The frog is included: it is a solid
    /// static body that blocks movement exactly like a tile and can move.
    /// The battlefield's own boundary needs no entry here - `Grid::build`
    /// insets its outer edge by the same margin.
    ///
    /// Tiles are taken at the *seam-closed* extent
    /// (`battlefield::tile_half_extent`, the same one `hits::Terrain` and
    /// the physics colliders use), reduced to the larger axis because
    /// `Grid::build` carries one scalar per obstacle. The plain
    /// `hull_size() * 0.5` this used to pass is 12px against a run's real
    /// 16px, i.e. the planner modelling walls as *smaller* than the solver
    /// does - the exact direction of error `maplint::check_planner_physics`
    /// exists to catch, which only stayed silent because the old 48px cell
    /// pitch happened to skip the band where it would have fired.
    pub(crate) fn nav_grid(&self, width: f32, height: f32) -> Grid {
        // Trees are left out of the seam-close set for the same reason
        // `hits::Terrain::build` leaves them out: they never close a seam,
        // here or in physics, so they must not close anybody else's.
        let seam_cells: HashSet<(i32, i32)> = self
            .world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| !o.material.is_tree())
            .map(|o| battlefield::pos_to_cell(o.position))
            .collect();
        // An active portal network joins the grid as one hub the planner
        // may route through (`Grid::with_portals`); with fewer than two
        // portals the grid is exactly the plain one.
        let t = tuning();
        let mut grid = Grid::build(
            width,
            height,
            PATHFIND_CELL_SIZE,
            battlefield::max_tank_clearance_half_extent(),
            self.world
                .query::<&Obstacle>()
                .iter()
                .map(|o| {
                    let (gx, gy) = battlefield::pos_to_cell(o.position);
                    let half = battlefield::tile_half_extent(
                        o.material,
                        &seam_cells,
                        gx,
                        gy,
                        o.hull_size() * 0.5,
                    );
                    (o.position, half.x.max(half.y))
                })
                .chain(self.world.query::<&Frog>().iter().map(|fr| {
                    (fr.position, FROG_COLLIDER_HALF_EXTENT.0.max(FROG_COLLIDER_HALF_EXTENT.1))
                }))
                // A burning cell is a wall for as long as it burns: the AI
                // routes around a pool rather than through it. A tiny
                // half-extent, so only the margin decides how wide the
                // detour is.
                .chain(self.fires.iter().map(|fire| (fire.position(), 1.0)))
                // Deep water is a wall (docs/water.md): the same cells the
                // static colliders stand on, at a cell's half-extent.
                .chain(self.water.deep_cells().map(|p| (p, OBSTACLE_GRID_SIZE * 0.5))),
        )
        .with_portals(self.active_portals(), t.portal_trigger_radius, t.portal_hop_cost);
        // A ford is open but dear: the router wades only when the dry way
        // round costs more.
        grid.weigh(self.water.shallow_cells(), tuning().water_ford_path_cost.max(1) as u32);
        grid
    }

    /// The map's portal anchors, active or not (docs/teleporting.md).
    pub fn portals(&self) -> &[Position] {
        &self.portals
    }

    /// Whether the portal network does anything: two or more portals. A
    /// lone portal neither teleports nor draws.
    pub fn portals_active(&self) -> bool {
        self.portals.len() >= 2
    }

    /// The portals the round acts on: all of them when active, none
    /// otherwise - so the phase, the nav grid, the renderer and the dev
    /// server share one rule.
    fn active_portals(&self) -> &[Position] {
        if self.portals_active() { &self.portals } else { &[] }
    }

    /// The frame's routing grid: `nav_grid` labelled for O(1) reachability,
    /// priced with the tactical surcharges, and carrying one flow field
    /// per target the pack shares - every live player and, while it
    /// lives, the player's frog (a hunter's quarry). Enemies then route
    /// toward those by reading the field; only a target of their own (an
    /// engagement slot, a wander waypoint, a pickup) still costs a search.
    ///
    /// The lane surcharge walks the cells in front of each live player's
    /// barrel (`Tank::rotation` is the axis a shot flies along) out to
    /// `route_lane_cells`, stopping at the first blocked cell - a shell
    /// flies further, but no tank can stand there anyway. The crowd
    /// surcharge is the cell each live enemy stands in. Both are read at
    /// field-build time, so this is the one place they are applied. No
    /// RNG: a surcharge is a pure function of positions, and the field's
    /// ties break on cell index.
    pub(crate) fn route_grid(&self, width: f32, height: f32) -> Grid {
        let mut grid = self.nav_grid(width, height);
        let t = tuning();
        let players: Vec<(Position, f32)> = self
            .seats_on_field()
            .into_iter()
            .flatten()
            .filter_map(|e| with_tank(&self.world, e, |tank| (!tank.is_wreck()).then_some((tank.position, tank.rotation))))
            .collect();
        if t.route_lane_cost > 0 {
            let mut lane = Vec::new();
            for &(pos, rotation) in &players {
                let Some(dir) = Dir::from_rotation(rotation) else {
                    continue;
                };
                let step = dir.vec();
                let mut at = pos;
                for _ in 0..t.route_lane_cells {
                    if grid.blocked_ahead(at, step) {
                        break;
                    }
                    at = Position::new(at.x + step.x * PATHFIND_CELL_SIZE, at.y + step.y * PATHFIND_CELL_SIZE);
                    lane.push(at);
                }
            }
            grid.surcharge(lane.into_iter(), t.route_lane_cost as u32);
        }
        if t.route_crowd_cost > 0 {
            let standing: Vec<Position> =
                self.world.query::<(&Tank, &Ai)>().iter().filter(|(tank, _)| !tank.is_wreck()).map(|(tank, _)| tank.position).collect();
            grid.surcharge(standing.into_iter(), t.route_crowd_cost as u32);
        }
        grid.label();
        for &(pos, _) in &players {
            grid.add_field(pos);
        }
        if let Some(frog) = self.frog.and_then(|e| with_frog(&self.world, e, |fr| (!fr.is_dead()).then_some(fr.position))) {
            grid.add_field(frog);
        }
        grid
    }

    /// The routing cell a world position falls in (`PATHFIND_CELL_SIZE`
    /// pitch) - for tests that reason about the grid.
    #[cfg(test)]
    pub(crate) fn grid_cell_of(&self, p: Position) -> (usize, usize) {
        ((p.x / PATHFIND_CELL_SIZE).max(0.0) as usize, (p.y / PATHFIND_CELL_SIZE).max(0.0) as usize)
    }

    /// Shortest route between two points on this round's nav grid, in
    /// cells (`None`: no route, `Some(0)`: same cell). For external
    /// tooling's path-stretch metric (the probe's `never-arrived` check) -
    /// call once per tank at round start, not per frame.
    pub fn nav_path_cells(&self, from: Position, to: Position, width: f32, height: f32) -> Option<u32> {
        self.nav_grid(width, height).path_cost(from, to)
    }

    /// The seed this round is running with, for replay via `--seed`.
    pub fn round_seed(&self) -> u64 {
        self.round_seed
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    /// `update` calls so far this round (see the `frame` field).
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Everything the most recent `update` recorded - see `Event`. Empty
    /// on a paused frame; holds `RoundStarted` right after `init`.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// The chassis the player is driving this round, whichever of the four
    /// sources `resolve_player_row` picked it from. `None` only before the
    /// first `init`.
    pub fn player_chassis(&self) -> Option<TankKind> {
        self.chassis_of(self.player()?)
    }

    /// Seat 1's chassis this round; `None` in a single-player round or
    /// before the first `init`.
    pub fn player2_chassis(&self) -> Option<TankKind> {
        self.chassis_of(self.seat(1)?)
    }

    fn chassis_of(&self, entity: Entity) -> Option<TankKind> {
        let mut q = self.world.query_one::<&Tank>(entity);
        TankKind::from_row(q.get().ok()?.row)
    }

    /// Every tank's externally visible state, for headless inspection
    /// (`src/bin/probe.rs`, the tests below) without touching `world`.
    /// Flatten the grass under every live hull and let the rest stand back
    /// up (`grass::tick`).
    ///
    /// A simulation phase rather than a draw-time effect because the
    /// recovery has to be frame-rate independent - the trail behind a tank
    /// is a length, and it would change with the frame rate if it were
    /// integrated in `render`. Cosmetic all the same: nothing reads `crush`
    /// but the renderer, and the phase draws no RNG, so a seeded replay is
    /// bit-identical with or without it.
    ///
    /// Velocity comes from the *body*, not `Tank::velocity`, which is the
    /// commanded cardinal vector and reads as zero the instant the driver
    /// lets go while the hull is still rolling.
    fn tick_grass(&mut self, dt: f32) {
        if self.grass.is_empty() {
            return;
        }
        let movers: Vec<crate::grass::Mover> = self
            .world
            .query::<&Tank>()
            .iter()
            .filter(|t| !t.is_wreck())
            .map(|t| crate::grass::Mover {
                position: t.position,
                velocity: t.body.map(|b| self.physics.velocity(b)).unwrap_or(t.velocity),
                half: t.hull_size() * 0.5,
            })
            .collect();
        crate::grass::tick(&mut self.grass, &movers, dt);
    }

    /// The tall-grass cells a live tank is currently moving through - the
    /// source `fx.rs` samples for the leaf specks a hull kicks up. A parked
    /// tank rustles nothing.
    /// The map's water as the rules see it (docs/water.md).
    pub fn water(&self) -> &crate::ground::WaterLayout {
        &self.water
    }

    /// Every live hull in a ford this frame: its owner slot, position and
    /// speed (px/s). What `fx.rs` throws spray from.
    pub fn wading(&self) -> Vec<(usize, Position, f32)> {
        let mut out: Vec<(usize, Position, f32)> = self
            .world
            .query::<&Tank>()
            .iter()
            .filter(|t| !t.is_wreck() && self.water.depth_at(t.position) != crate::ground::Depth::Dry)
            .map(|t| {
                let speed = t.body.map_or(0.0, |b| {
                    let v = self.physics.velocity(b);
                    (v.x * v.x + v.y * v.y).sqrt()
                });
                (t.owner_slot(), t.position, speed)
            })
            .collect();
        out.sort_by_key(|w| w.0);
        out
    }

    pub fn grass_disturbed(&self) -> Vec<Position> {
        let reach = OBSTACLE_GRID_SIZE * 0.5 + tuning().grass_crush_radius * 0.5;
        let movers: Vec<Position> = self
            .world
            .query::<&Tank>()
            .iter()
            .filter(|t| !t.is_wreck())
            .filter(|t| t.body.is_some_and(|b| {
                let v = self.physics.velocity(b);
                v.x * v.x + v.y * v.y > 400.0
            }))
            .map(|t| t.position)
            .collect();
        self.grass_cells
            .iter()
            .copied()
            .filter(|c| movers.iter().any(|m| (m.x - c.x).abs() < reach && (m.y - c.y).abs() < reach))
            .collect()
    }

    /// Every tile currently on fire, with how long it has been burning.
    /// One of three read-only views the presentation particle layer
    /// samples each frame (see `fx::Fx::sample_world`): these are *states*
    /// rather than events - a tile burns for a second and a half, it does
    /// not burn at an instant - so there is nothing in the event log to
    /// drive them from.
    pub fn burning_tiles(&self) -> Vec<(Position, f32)> {
        self.world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| o.burning && !o.destroyed)
            .map(|o| (o.position, o.burn_elapsed))
            .collect()
    }

    /// Every wreck still inside its `wreck_burn_seconds` window, with the
    /// time it has been burning.
    pub fn burning_wrecks(&self) -> Vec<(Position, f32)> {
        self.world
            .query::<&Tank>()
            .iter()
            .filter(|t| t.is_wreck() && !t.is_dead())
            .map(|t| (t.position, t.wreck_timer))
            .collect()
    }

    /// Every tank contact hard enough to be worth showing, as
    /// `(where, how hard, was it another tank)`.
    ///
    /// `Physics::contact_stats` has computed this every frame since the
    /// probe's anomaly work and **nothing in the game has ever read it** -
    /// so driving into a wall, shunting a wreck and grinding through a
    /// six-tank pileup all produced no feedback whatsoever. A contact is a
    /// *state*, not an event (a tank scrapes along a wall for a second, it
    /// does not scrape at an instant), so this is sampled by the
    /// presentation layer rather than pushed as an event - which also means
    /// no new RNG and no simulation change at all.
    pub fn contacts(&self) -> Vec<(Position, f32, bool)> {
        self.world
            .query::<&Tank>()
            .iter()
            .filter(|t| !t.is_dead())
            .filter_map(|t| {
                let stats = self.physics.contact_stats(t.body?);
                let at = stats.contact?;
                (stats.max_impulse > 0.0).then_some((at, stats.max_impulse, stats.touching_tank))
            })
            .collect()
    }

    /// Every prop currently being ground down under a tank's tracks, with
    /// how far into its collapse it is.
    pub fn ramming_tiles(&self) -> Vec<(Position, f32)> {
        self.world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| o.ram_timer > 0.0 && !o.destroyed)
            .map(|o| (o.position, o.ram_timer))
            .collect()
    }

    pub fn tank_snapshots(&self) -> Vec<TankSnapshot> {
        self.world
            .query::<(Entity, &Tank)>()
            .iter()
            .map(|(entity, tank)| {
                // A tank still rolling in has no body: its kinematic
                // velocity stands in and it touches nothing.
                let contact = tank.body.map(|b| self.physics.contact_stats(b)).unwrap_or_default();
                TankSnapshot {
                    slot: tank.owner_slot(),
                    is_player: tank.is_player(),
                    player: self.player_index(entity),
                    entering: tank.body.is_none(),
                    position: tank.position,
                    rotation: tank.rotation,
                    velocity: tank.body.map(|b| self.physics.velocity(b)).unwrap_or(tank.velocity),
                    // Scaled by the throttle: `Tank::velocity` is the
                    // unthrottled cardinal (the AI's avoidance reads it),
                    // but what was actually *asked* of the body is that
                    // times the throttle, and this field is the probe's
                    // intent-vs-outcome signal.
                    commanded_velocity: Position::new(
                        tank.velocity.x * tank.throttle,
                        tank.velocity.y * tank.throttle,
                    ),
                    top_speed: tank.base_speed(),
                    damage: tank.damage,
                    shells_ammo: tank.shells_ammo,
                    minigun_ammo: tank.minigun_ammo,
                    missile_ammo: tank.missile_ammo,
                    plasma_ammo: tank.plasma_ammo,
                    laser_charges: tank.laser_charges,
                    flame_fuel: tank.flame_fuel,
                    burn_timer: tank.burn_timer,
                    shield_hp: tank.shield_hp,
                    shield_recharge_delay: tank.shield_recharge_delay,
                    touching_static: contact.touching_static,
                    touching_tank: contact.touching_tank,
                    contact_impulse: contact.max_impulse,
                    is_wreck: tank.is_wreck(),
                }
            })
            .collect()
    }
}

/// Read-only summary of one tank, from `Game::tank_snapshots`. A
/// non-player tank is always an enemy.
pub struct TankSnapshot {
    /// `Tank::owner_slot`: the players first (0, and 1 in a two-player
    /// round), then the enemies. Slots are never reused, so this
    /// identifies a tank for the whole round even after a wreck despawns
    /// or a wave tank arrives.
    pub slot: usize,
    /// A human player's tank, either of them.
    pub is_player: bool,
    /// Which human player (0 or 1), `None` for an enemy.
    pub player: Option<u8>,
    /// A wave tank still rolling in from outside the battlefield: no
    /// physics body yet, not part of the fight.
    pub entering: bool,
    pub position: Position,
    pub rotation: f32,
    /// Real physics velocity read back from the body.
    pub velocity: Position,
    /// The commanded target velocity `drive_tank` chases (`Tank::velocity`);
    /// its spread from `velocity` is the intent-vs-outcome signal the probe
    /// windows over.
    pub commanded_velocity: Position,
    /// Rolled base top speed, before damage/boost scaling.
    pub top_speed: f32,
    pub damage: f32,
    pub shells_ammo: i32,
    pub minigun_ammo: i32,
    pub missile_ammo: i32,
    pub plasma_ammo: i32,
    pub laser_charges: i32,
    /// Flamethrower fuel left, in seconds of burn.
    pub flame_fuel: f32,
    /// Seconds of flame afterburn left on the hull (0 = not burning).
    pub burn_timer: f32,
    /// Rainbow-shield absorption left, in damage points (0 = unshielded).
    /// Not seconds - the shield is a pool, see `Tank::shield_hp`.
    pub shield_hp: f32,
    /// Seconds before that shield starts refilling. Carried because it is
    /// live simulation state that decides future `shield_hp`, so a replay
    /// that diverged only here would otherwise look identical for up to
    /// `shield_recharge_delay_seconds` of frames.
    pub shield_recharge_delay: f32,
    /// The hull has an active contact with static terrain right now.
    pub touching_static: bool,
    /// The hull has an active contact with *another tank* right now.
    /// Computed by `Physics::contact_stats` since the contact work landed and
    /// surfaced here for the enemy command & control work
    /// (docs/enemy-command-and-control-prd.md section 10): it is the only
    /// signal that tells a three-tank jam apart from a two-tank bump, because
    /// `Event::Ram` saturates - one ram sets the cooldown on *both*
    /// participants, so it caps at roughly one event per tank per
    /// `ram_damage_cooldown` however many neighbours it is grinding against.
    pub touching_tank: bool,
    /// Strongest solver contact impulse on the hull this step.
    pub contact_impulse: f32,
    pub is_wreck: bool,
}

/// Turn an intent into hull rotation plus a mass-aware impulse nudging the
/// tank's body toward its commanded velocity. `Tank::control` sets the
/// target; the axis along the hull chases it with the flat
/// TANK_ACCEL_FORCE when speeding up or the exponential TANK_DECEL_CURVE_RATE
/// curve when slowing/reversing (both scaled by mass and damage), while the
/// perpendicular axis is scrubbed toward zero by TANK_TURN_GRIP_FORCE - weaker
/// than accel, so a corner reads as a drift rather than a snap, and applied
/// whether or not a key is held, since tracks resist sliding all the time.
/// When the facing crosses between the X and Y axes the (non-rotating)
/// collider is reoriented. Shared by the player and every enemy.
/// What `enemy_phase`'s collect pass parks for its apply pass: one tank's
/// decided intent, plus the two values `drive_tank_with` needs as they stood
/// before this frame touched them. Kept in the collect pass's own iteration
/// order, which is the order the firing RNG was drawn in.
struct Pending {
    entity: Entity,
    slot: usize,
    intent: Intent,
    current: Position,
    facing_before: f32,
}

/// What the ground under a hull does to its drive this frame
/// (docs/water.md): dry ground leaves everything at 1 and the water still.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Footing {
    /// Fraction of the commanded top speed and of `tank_accel_force` the
    /// hull gets.
    pace: f32,
    /// Fraction of `tank_turn_grip_force` the hull keeps.
    grip: f32,
    /// The water's own velocity: the frame the hull drives relative to,
    /// so a hull that stops in a current drifts with it.
    flow: Position,
    /// In water at all - drops a speed boost and wets the tracks.
    wading: bool,
}

impl Footing {
    pub(crate) const DRY: Footing = Footing { pace: 1.0, grip: 1.0, flow: Position::new(0.0, 0.0), wading: false };

    pub(crate) fn at(water: &crate::ground::WaterLayout, pos: Position) -> Footing {
        match water.depth_at(pos) {
            crate::ground::Depth::Dry => Footing::DRY,
            _ => {
                let t = tuning();
                let flow = if water.pushes_south(pos) { t.water_current_speed } else { 0.0 };
                Footing { pace: t.water_speed_factor, grip: t.water_grip_factor, flow: Position::new(0.0, flow), wading: true }
            }
        }
    }
}

fn drive_tank(physics: &mut Physics, tank: &mut Tank, intent: Intent, dt: f32, footing: Footing) {
    let handle = tank.body.expect("tank should always have a physics body once spawned");
    drive_tank_with(physics, tank, intent, dt, physics.velocity(handle), tank.rotation, footing)
}

/// `drive_tank`, given the body velocity and hull facing as they stood
/// *before anything this frame touched them*.
///
/// The split exists because `enemy_phase` aims and fires in its collect pass
/// but defers the impulse to its apply pass, so that the enemy command &
/// control layer can see every intent before any tank moves
/// (docs/enemy-command-and-control-prd.md section 4). By the apply pass both
/// values `drive_tank` reads at its top have been disturbed: `current` has
/// picked up the recoil impulse of a shot fired in between (rapier mutates
/// `linvel` immediately), and `facing_before` has already been snapped by the
/// collect pass's own `tank.control`, which would silently defeat the
/// `resize_collider` check below. Capturing both in the collect pass and
/// passing them here is what makes the deferred drive bit-identical to an
/// undeferred one.
///
/// Every other caller goes through `drive_tank` and reads them itself.
///
/// `footing` is the ground's say (docs/water.md): in a ford the top speed,
/// the drive and the grip are scaled down, and the whole model runs in the water's own
/// frame - `current` is taken relative to `footing.flow`, so with no
/// command the hull settles to the water's velocity rather than to rest,
/// and driving upstream nets the difference. Impulses are deltas, so
/// nothing else changes.
#[allow(clippy::too_many_arguments)]
fn drive_tank_with(
    physics: &mut Physics,
    tank: &mut Tank,
    intent: Intent,
    dt: f32,
    current: Position,
    facing_before: f32,
    footing: Footing,
) {
    let handle = tank.body.expect("tank should always have a physics body once spawned");
    let current = Position::new(current.x - footing.flow.x, current.y - footing.flow.y);
    if footing.wading {
        // Water takes the boost: a speed-up ends the moment its hull
        // wades in, and the marks it leaves on the far bank are wet.
        tank.speed_boost_timer = 0.0;
        tank.wet_timer = tuning().water_wet_track_seconds;
    }

    tank.control(intent.move_dir, intent.face);
    tank.ease_visual_rotation(dt);
    tank.ease_turret_visual_rotation(dt);
    tank.ease_ring_position(dt);
    let target = tank.velocity;

    if tank.rotation != facing_before {
        physics.resize_collider(physics.collider_of(handle), tank.move_half_extents(tank.facing_along_x()));
    }

    // The commanded top speed, eased off by whatever `intent.slow` asks for
    // (docs/enemy-command-and-control-prd.md section 5). Scaling the *target*
    // and not the delta makes this a lower top speed rather than a lazier
    // accelerator, and it gives a full stop for free at `slow == 1.0`: the
    // target goes to zero, `speeding_up` is false, and the tank falls onto
    // the existing decel curve instead of needing a separate brake path.
    // Nothing but `simulation::command` ever sets it; the player and every
    // `Ai` leave it at 0, so `scale` is 1.0 and this is a no-op multiply.
    let along_x = tank.facing_along_x();
    let scale = intent.speed_scale() * footing.pace;
    tank.throttle = intent.speed_scale();
    let (current_on, target_on, current_off) = if along_x {
        (current.x, target.x * scale, current.y)
    } else {
        (current.y, target.y * scale, current.x)
    };

    let want_on = target_on - current_on;
    let speeding_up = want_on * current_on >= 0.0;
    let delta_on = if speeding_up {
        let max_on = tuning().tank_accel_force * footing.pace * tank.speed_factor() / tank.mass() * dt;
        want_on.clamp(-max_on, max_on)
    } else {
        // Close a rate-controlled fraction of the remaining gap each frame
        // (frame-rate independent); snap the last sliver below
        // TANK_DECEL_SNAP_PX rather than trailing the asymptote forever.
        let rate = tuning().tank_decel_curve_rate * tank.speed_factor() / tank.mass();
        let remaining_gap = want_on * (-rate * dt).exp();
        if remaining_gap.abs() < tuning().tank_decel_snap_px { want_on } else { want_on - remaining_gap }
    };

    let max_off = tuning().tank_turn_grip_force * footing.grip / tank.mass() * dt;
    let delta_off = (-current_off).clamp(-max_off, max_off);

    let delta = if along_x {
        Position::new(delta_on, delta_off)
    } else {
        Position::new(delta_off, delta_on)
    };
    physics.apply_impulse(handle, Position::new(delta.x * tank.mass(), delta.y * tank.mass()));
}

/// `cell` unless it is deep water, else the nearest map cell (square
/// rings outward, the map's own `nearest_free_cell` order) that is neither
/// solid nor deep. A `start` cell can never be water itself (a cell holds
/// one object), so this only matters for the centre fallback of a map
/// without one: a lake over the middle of the field spawns the player on
/// its shore rather than inside the lake's colliders.
fn dry_cell_near(map: &MapFile, water: &crate::ground::WaterLayout, cell: (i32, i32)) -> (i32, i32) {
    let deep = |c: i32, r: i32| water.depth_at(map::cell_to_world(c, r)) == crate::ground::Depth::Deep;
    let solid = |c: i32, r: i32| map.cell(c, r).is_some_and(CellObject::is_solid);
    if !deep(cell.0, cell.1) {
        return cell;
    }
    for radius in 1..=MapFile::NEAREST_FREE_CELL_MAX_RADIUS {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx.abs() != radius && dy.abs() != radius {
                    continue;
                }
                let (c, r) = (cell.0 + dx, cell.1 + dy);
                if !deep(c, r) && !solid(c, r) {
                    return (c, r);
                }
            }
        }
    }
    cell
}

/// Read a tank's position back from its body; a tank still rolling in has
/// none and keeps the position `rollin_phase` gave it.
fn sync_tank_from_physics(physics: &Physics, tank: &mut Tank) {
    let Some(handle) = tank.body else { return };
    tank.position = physics.position(handle);
}

/// True if the two tanks' bodies currently have an active contact.
fn tanks_touching(physics: &Physics, a: &Tank, b: &Tank) -> bool {
    let a = a.body.expect("tank should always have a physics body once spawned");
    let b = b.body.expect("tank should always have a physics body once spawned");
    physics.touching(a, b)
}

/// Spawn one pickup at a map slot - unconditionally; the map's placement
/// is deliberate and not rejection-sampled.
fn spawn_pickup_at(world: &mut hecs::World, pos: Position, kind: PickupKind) {
    world.spawn((Pickup { kind, position: pos },));
}

impl Game {
    /// Announce every shield that shattered this frame: one
    /// `Event::ShieldBroken` and one `SHOCK_SHIELD_BREAK` ripple per tank,
    /// then clear the latch.
    ///
    /// One drain rather than an emit at each seam. A shield is spent in
    /// eight places - `Tank::take_damage` covers ram (both tanks), blasts,
    /// the flame cone and its afterburn, oil fire and the frog, and the
    /// projectile deflect in `resolve_projectiles` covers the rest - and
    /// `take_damage` has no `Frame` to push onto, so asking each caller to
    /// notice the edge itself is how six of the eight ended up silent.
    /// Walks players in index order then enemies, matching every other
    /// ordered walk in this file so the event order is fixed.
    ///
    /// Only called on the live branch of `update`: an in-flight shell that
    /// lands on the end screen still spends a shield (it is the same hit
    /// loop) but must not shake the camera over the Won/Lost banner.
    fn drain_shield_breaks(&mut self, f: &mut Frame) {
        let mut broken: Vec<(usize, Position)> = Vec::new();
        for player in self.players().into_iter().flatten() {
            with_tank_mut(&self.world, player, |t| {
                if std::mem::take(&mut t.shield_broke) {
                    broken.push((t.owner_slot(), t.position));
                }
            });
        }
        for tank in self.world.query_mut::<&mut Tank>().with::<&Ai>() {
            if std::mem::take(&mut tank.shield_broke) {
                broken.push((tank.owner_slot(), tank.position));
            }
        }
        for (slot, at) in broken {
            f.events.push(Event::ShieldBroken { slot, x: at.x, y: at.y });
            f.shocks.push(Shockwave::scaled(at, SHOCK_SHIELD_BREAK));
        }
    }
}

/// If the tank at `entity` is holding a live rainbow shield, its centre and
/// owner - what a projectile that strikes it bounces off and becomes.
fn shield_deflector(world: &hecs::World, entity: Entity) -> Option<(Position, Owner)> {
    with_tank(world, entity, |t| (t.is_shielded() && !t.is_wreck()).then(|| (t.position, t.owner())))
}

/// Live pickups sitting on a map slot. An un-slotted bonus shield doesn't
/// count, so it can never make the field look full to the respawn timer.
fn slot_backed_count(world: &hecs::World, slots: &[(Position, PickupKind)]) -> usize {
    world
        .query::<&Pickup>()
        .iter()
        .filter(|p| slots.iter().any(|&(pos, _)| pos.distance_to(p.position) <= 0.5))
        .count()
}

/// Top up one pickup at a uniformly random slot not currently occupied,
/// with the health slot's bonus rolls if that slot is a health pack
/// (`frog_hurt` is the frog pack's gate). A no-op if every slot is full.
fn respawn_from_slots(
    world: &mut hecs::World,
    map: &MapFile,
    slots: &[(Position, PickupKind)],
    width: f32,
    height: f32,
    frog_hurt: bool,
    rng: &mut SmallRng,
) -> Option<(Position, PickupKind)> {
    let occupied: Vec<Position> = world.query::<&Pickup>().iter().map(|p| p.position).collect();
    let free: Vec<(Position, PickupKind)> = slots
        .iter()
        .copied()
        .filter(|&(pos, _)| occupied.iter().all(|&p| p.distance_to(pos) > 0.5))
        .collect();
    if free.is_empty() {
        return None;
    }
    let (pos, kind) = free[rng.random_range(0..free.len())];
    spawn_pickup_at(world, pos, kind);
    if kind == PickupKind::Health {
        maybe_spawn_health_slot_bonuses(world, map, pos, width, height, frog_hurt, rng);
    }
    Some((pos, kind))
}

/// The un-slotted bonuses that ride along with a Health slot just
/// (re)spawned at `slot`: the rainbow shield always, and a frog health pack
/// only while `frog_hurt`.
///
/// The frog pack's gate is checked *before* its roll, never after, and that
/// ordering is load-bearing (docs/frog-health-pack-prd.md section 6): a
/// round whose frog is never hurt - `Game::init`'s own spawn pass included,
/// where the frog has just been created at full health - draws exactly the
/// RNG it drew before the pack existed, so every such seeded replay is
/// unchanged. `occupied` is recomputed per roll inside `maybe_spawn_bonus`,
/// so the two bonuses can never land on the same neighbour.
fn maybe_spawn_health_slot_bonuses(
    world: &mut hecs::World,
    map: &MapFile,
    slot: Position,
    width: f32,
    height: f32,
    frog_hurt: bool,
    rng: &mut SmallRng,
) {
    maybe_spawn_bonus(world, map, slot, PickupKind::Shield, tuning().shield_near_health_chance, width, height, rng);
    if frog_hurt {
        let chance = tuning().frog_pack_near_health_chance;
        maybe_spawn_bonus(world, map, slot, PickupKind::FrogHealth, chance, width, height, rng);
    }
}

/// Roll `chance` for the health slot just (re)spawned at `slot` and, on
/// success, drop a `kind` pickup in a free cell next to it. Skipped while
/// one of that kind already sits beside this slot, so repeated health
/// respawns can't pile them up.
fn maybe_spawn_bonus(
    world: &mut hecs::World,
    map: &MapFile,
    slot: Position,
    kind: PickupKind,
    chance: f32,
    width: f32,
    height: f32,
    rng: &mut SmallRng,
) {
    let occupied: Vec<Position> = world.query::<&Pickup>().iter().map(|p| p.position).collect();
    let already_there = world
        .query::<&Pickup>()
        .iter()
        .any(|p| p.kind == kind && p.position.distance_to(slot) <= OBSTACLE_GRID_SIZE * 1.5);
    if already_there || rng.random_range(0.0..1.0) >= chance {
        return;
    }
    if let Some(pos) = bonus_pickup_cell(map, slot, &occupied, width, height, rng) {
        spawn_pickup_at(world, pos, kind);
    }
}

/// A uniformly random free cell touching the slot at `slot`, or `None`
/// when all eight neighbours are taken. Eligible: empty, road or water in
/// the map (walls, the frog, the start cell and other slots are not), centre inside
/// the border walls (their inner faces sit at 0/`width`/0/`height`), and no
/// live pickup already on it. Map walls are checked rather than the live
/// obstacle set: a shot-away wall's cell stays off limits, which is
/// conservative but keeps this independent of the physics world.
fn bonus_pickup_cell(
    map: &MapFile,
    slot: Position,
    occupied: &[Position],
    width: f32,
    height: f32,
    rng: &mut SmallRng,
) -> Option<Position> {
    const NEIGHBOURS: [(i32, i32); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
    let (col, row) = map::world_to_cell(slot);
    let half = OBSTACLE_GRID_SIZE * 0.5;
    let candidates: Vec<Position> = NEIGHBOURS
        .iter()
        .filter_map(|&(dx, dy)| {
            let (c, r) = (col + dx, row + dy);
            let open = matches!(map.cell(c, r), None | Some(CellObject::Road | CellObject::Water));
            let pos = map::cell_to_world(c, r);
            let inside = pos.x >= half && pos.x <= width - half && pos.y >= half && pos.y <= height - half;
            let free = occupied.iter().all(|&p| p.distance_to(pos) > 0.5);
            (open && inside && free).then_some(pos)
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    Some(candidates[rng.random_range(0..candidates.len())])
}

/// Read-only access to one tank. Backed by the dynamically borrow-checked
/// `World::query_one`, so it can run inside another query's iteration as
/// long as the two never touch the same entity. `pub(crate)`: `render`
/// uses it too.
impl Game {
    /// The seats in owner-slot order, empty ones as `None`. Every loop
    /// that has to treat the players before the enemies walks this, so
    /// their RNG draws keep a fixed order whatever the seat count.
    pub(crate) fn players(&self) -> [Option<Entity>; MAX_SEATS] {
        self.seats
    }

    /// Player 1's tank - seat 0, the one every round has.
    pub(crate) fn player(&self) -> Option<Entity> {
        self.seats[PLAYER_OWNER_SLOT]
    }

    /// The seats whose tank stands on the battlefield: like `players()`,
    /// but a seat driving back in through a gate reads as `None`.
    ///
    /// A returning seat (`waves::RollIn`, docs/online-coop-prd.md section
    /// 4.11) is outside the field with no body, so it takes no part in
    /// the frame - it is not shot at, blasted, burnt, rammed, targeted,
    /// routed to or the centre of an engagement ring - the same way an
    /// entering wave tank is left alone by every `.with::<&Ai>()` query.
    /// It is still one of `players()`, which is what keeps the round from
    /// being lost while a seat is on its way back.
    pub(crate) fn seats_on_field(&self) -> [Option<Entity>; MAX_SEATS] {
        let mut seats = self.seats;
        for seat in &mut seats {
            if seat.is_some_and(|e| self.world.get::<&RollIn>(e).is_ok()) {
                *seat = None;
            }
        }
        seats
    }

    /// Seat `index`'s tank, `None` past the seats this round holds.
    pub(crate) fn seat(&self, index: usize) -> Option<Entity> {
        self.seats.get(index).copied().flatten()
    }

    /// Which seat `entity` sits in, or `None` for anything else.
    pub(crate) fn player_index(&self, entity: Entity) -> Option<u8> {
        self.seats.iter().position(|&e| e == Some(entity)).map(|i| i as u8)
    }

    /// True for any seat's tank.
    pub(crate) fn is_player(&self, entity: Entity) -> bool {
        self.player_index(entity).is_some()
    }

    /// The first owner slot an enemy can take: the seats sit below it.
    pub fn first_enemy_slot(&self) -> usize {
        self.players.count()
    }
}

pub(crate) fn with_tank<R>(world: &hecs::World, entity: Entity, f: impl FnOnce(&Tank) -> R) -> R {
    let mut q = world.query_one::<&Tank>(entity);
    f(q.get().expect("entity should have a Tank component"))
}

pub(crate) fn with_frog<R>(world: &hecs::World, entity: Entity, f: impl FnOnce(&Frog) -> R) -> R {
    let mut q = world.query_one::<&Frog>(entity);
    f(q.get().expect("entity should have a Frog component"))
}

fn with_frog_mut<R>(world: &hecs::World, entity: Entity, f: impl FnOnce(&mut Frog) -> R) -> R {
    let mut q = world.query_one::<&mut Frog>(entity);
    f(q.get().expect("entity should have a Frog component"))
}

fn with_tank_mut<R>(world: &hecs::World, entity: Entity, f: impl FnOnce(&mut Tank) -> R) -> R {
    let mut q = world.query_one::<&mut Tank>(entity);
    f(q.get().expect("entity should have a Tank component"))
}

/// Mutable access to two *different* tanks at once (`query_disjoint_mut`).
fn with_two_tanks_mut<R>(world: &mut hecs::World, a: Entity, b: Entity, f: impl FnOnce(&mut Tank, &mut Tank) -> R) -> R {
    let [ta, tb] = world.query_disjoint_mut::<&mut Tank, 2>([a, b]);
    f(
        ta.expect("entity should have a Tank component"),
        tb.expect("entity should have a Tank component"),
    )
}

/// Build one enemy tank of chassis `row` at `pos` in owner slot `slot`,
/// facing down, with every per-tank spawn roll in this order: speed
/// spread (`enemy_speed_variance`), damage variant, a possible special
/// weapon (`enemy_special_weapon_chance`, one pickup's worth), a possible
/// starting shield (`spawn_shield_chance`, the player's roll too) and
/// track wobble. Shared by the band placement in `init` and the wave
/// scheduler, so a wave tank is kitted exactly like a band tank. No
/// physics body: the caller spawns one when the tank is on the field.
fn roll_enemy_tank(rng: &mut SmallRng, row: i32, pos: Position, slot: usize) -> Tank {
    let factor = 1.0 + rng.random_range(-tuning().enemy_speed_variance..tuning().enemy_speed_variance);
    let mut enemy = Tank {
        row,
        shell_variant: TANK_SHELL_VARIANT_BY_ROW[row as usize],
        damage_variant: rng.random_range(0..DAMAGE_VARIANTS),
        position: pos,
        rotation: 180.0,
        speed_scale: factor,
        owner: Owner::Enemy(slot),
        ..Tank::default()
    };
    if rng.random_range(0.0..1.0) < tuning().enemy_special_weapon_chance {
        if rng.random_range(0.0..1.0) < tuning().enemy_special_weapon_laser_share {
            enemy.enqueue_weapon(ActiveWeapon::Laser);
            enemy.laser_charges += tuning().laser_charges_per_pickup;
            enemy.laser_variant = if rng.random_range(0.0..1.0) < tuning().laser_blue_pickup_chance {
                LaserVariant::Blue
            } else {
                LaserVariant::Red
            };
        } else if rng.random_range(0.0..1.0) < tuning().enemy_special_weapon_plasma_share {
            enemy.enqueue_weapon(ActiveWeapon::Plasma);
            enemy.plasma_ammo += tuning().plasma_ammo_per_pickup;
            enemy.plasma_variant = if rng.random_range(0.0..1.0) < tuning().plasma_purple_pickup_chance {
                PlasmaVariant::Purple
            } else {
                PlasmaVariant::Teal
            };
        } else {
            enemy.enqueue_weapon(ActiveWeapon::Minigun);
            enemy.minigun_ammo += tuning().minigun_ammo_per_pickup;
        }
    }
    if rng.random_range(0.0..1.0) < tuning().spawn_shield_chance {
        enemy.shield_hp = tuning().shield_capacity;
    }
    roll_track_distortion(&mut enemy, rng);
    enemy
}

/// Roll a tank's per-tank track-distortion parameters (see
/// TRACK_WOBBLE_AMP_MIN_DEG etc. in lib.rs).
fn roll_track_distortion(tank: &mut Tank, rng: &mut SmallRng) {
    tank.track_wobble_amp = rng.random_range(tuning().track_wobble_amp_min_deg..tuning().track_wobble_amp_max_deg);
    let wavelength = rng.random_range(tuning().track_wobble_wavelength_min..tuning().track_wobble_wavelength_max);
    // Radians per mark: one mark per TRACK_SPACING px, a full cycle per
    // `wavelength` px.
    tank.track_wobble_freq = std::f32::consts::TAU * tuning().track_spacing / wavelength;
    tank.track_wobble_phase = rng.random_range(0.0..std::f32::consts::TAU);
    tank.track_scale_jitter = rng.random_range((1.0 - tuning().track_scale_jitter)..(1.0 + tuning().track_scale_jitter));
}

/// Roll one spawning enemy's `Role` for `mission` (docs/maps-to-levels.md
/// "AI roles"): Protect rolls hunters at `enemy_hunter_share_protect`
/// (the rest fight the player), Hunt at `enemy_hunter_share_hunt` (the rest
/// guard the enemy frog), Destroy has nothing to hunt or guard. A share of
/// zero draws nothing, so it leaves the round's RNG stream untouched.
fn roll_role(mission: Mission, rng: &mut SmallRng) -> Role {
    let mut rolls = |share: f32| share > 0.0 && rng.random_range(0.0..1.0) < share;
    match mission {
        Mission::Protect => {
            if rolls(tuning().enemy_hunter_share_protect) { Role::Hunter } else { Role::Player }
        }
        Mission::Hunt => {
            if rolls(tuning().enemy_hunter_share_hunt) { Role::Hunter } else { Role::Guard }
        }
        Mission::Destroy => Role::Player,
    }
}

/// Roll a tank's wrecked-hull variant the first frame it is a wreck; a
/// no-op every other frame. Uses the round stream, never `rand::rng()`.
/// Returns true on that one frame, which is also the moment the body's
/// damping and friction have to change (`Physics::settle_wreck`) - a wreck
/// is made by damage mid-round, not spawned as one, so there is no builder
/// to set them on.
fn roll_wreck_col(tank: &mut Tank, rng: &mut SmallRng) -> bool {
    if tank.is_wreck() && tank.wreck_col.is_none() {
        tank.wreck_col = Some(TANK_WRECK_COLS[rng.random_range(0..TANK_WRECK_COLS.len())]);
        return true;
    }
    false
}

/// Lay tread marks along the distance a tank travelled this frame, one per
/// TRACK_SPACING px, and advance its tread-animation frame off the same
/// signal. Must only run on frames the physics stepped - otherwise a
/// stationary `before` reads as idle and resets the animation. Marks
/// follow the raw travel heading (not the snapped hull rotation), so a
/// real turn traces its real curve and a sideways shove leaves sideways
/// marks. Stops once the hull is disabled or wrecked. Water takes no
/// mark (`depth`, the water under the hull): the treads still turn, but
/// nothing is pressed into a river bed, and the marks laid while
/// `Tank::wet_timer` runs after wading out are wet ones.
fn lay_tracks(tracks: &mut Vec<Track>, tank: &mut Tank, before: Position, depth: crate::ground::Depth) {
    if tank.is_wreck() || tank.damage >= TANK_HULL_DISABLED_DAMAGE {
        return;
    }
    let moved = tank.position.distance_to(before);
    if moved <= 0.0 {
        tank.hull_frame = 0;
        return;
    }
    tank.hull_anim_accum += moved;
    while tank.hull_anim_accum >= tuning().tank_hull_track_frame_distance {
        tank.hull_anim_accum -= tuning().tank_hull_track_frame_distance;
        tank.hull_frame = (tank.hull_frame + 1) % TANK_HULL_TRACK_COLS.len() as i32;
    }
    if depth != crate::ground::Depth::Dry {
        tank.track_accum = 0.0;
        return;
    }
    // Unit vector pointing back along this frame's travel.
    let back = Vec2::new((before.x - tank.position.x) / moved, (before.y - tank.position.y) / moved);
    let mut heading = (-back.x).atan2(back.y).to_degrees();
    if heading < 0.0 {
        heading += 360.0;
    }
    // Marks start at the rear edge so the trail never pokes ahead of the hull.
    let rear = tank.hull_size() * 0.5;
    let weight_scale = tuning().track_weight_scale[tank.row as usize];
    let scale = tank.scale * tuning().track_scale_fraction * weight_scale * tank.track_scale_jitter;
    let max_opacity = tuning().track_max_opacity * tuning().track_weight_opacity[tank.row as usize];

    tank.track_accum += moved;
    while tank.track_accum >= tuning().track_spacing {
        tank.track_accum -= tuning().track_spacing;
        let dist_back = rear + tank.track_accum;
        // Per-tank wobble so a straight drive doesn't stamp identical marks.
        let wobble = tank.track_wobble_amp
            * (tank.track_mark_count as f32 * tank.track_wobble_freq + tank.track_wobble_phase).sin();
        tracks.push(Track {
            position: Position::new(tank.position.x + back.x * dist_back, tank.position.y + back.y * dist_back),
            rotation: heading + wobble,
            scale,
            max_opacity,
            age: 0.0,
            scorched: false,
            wet: tank.wet_timer > 0.0,
        });
        tank.track_mark_count += 1;
    }
}

/// Whether an enemy competes for an engagement slot this frame, and if
/// not, why - the exclusions win over the range test. `target` is what
/// the enemy fights (the player, or a hunter's frog).
fn engage_status(tank: &Tank, ai: &Ai, target: Position) -> EngageStatus {
    if tank.is_wreck() {
        EngageStatus::Wreck
    } else if tank.damage >= tuning().enemy_flee_damage {
        EngageStatus::Fleeing
    } else if tank.active_weapon() == ActiveWeapon::Shell && ai.is_retreating() {
        EngageStatus::Retreating
    } else if tank.position.distance_to(target) <= tuning().enemy_view_range || ai.is_hit_alerted() {
        EngageStatus::Engaged
    } else {
        EngageStatus::OutOfRange
    }
}

/// The AI-decision events for one enemy's `think`: every transition
/// between its memory before and after (see `Event`'s AI variants).
fn ai_transition_events(events: &mut Vec<Event>, slot: usize, before: &AiSnapshot, after: &AiSnapshot) {
    if before.last_action != after.last_action {
        events.push(Event::AiAction { slot, from: before.last_action, to: after.last_action });
    }
    if before.retreating != after.retreating {
        events.push(Event::Retreat { slot, on: after.retreating });
    }
    if before.breaching != after.breaching {
        events.push(Event::Breach { slot, dir: after.breaching });
    }
    if before.escapes != after.escapes {
        events.push(Event::StuckEscape { slot, escapes: after.escapes });
    }
}

#[cfg(test)]
mod overlay_tests {
    use super::Overlays;

    /// The I key walks NONE -> INSPECT -> ALL -> NONE, and a hand-set mix
    /// (including one tank layer on its own) snaps back to NONE.
    #[test]
    fn presets_cycle_and_mixes_snap() {
        assert_eq!(Overlays::default(), Overlays::NONE);
        assert_eq!(Overlays::NONE.next_preset(), Overlays::INSPECT);
        assert_eq!(Overlays::INSPECT.next_preset(), Overlays::ALL);
        assert_eq!(Overlays::ALL.next_preset(), Overlays::NONE);
        assert!(Overlays::INSPECT.hitboxes && Overlays::INSPECT.stats);
        assert!(Overlays::ALL.hitboxes && Overlays::ALL.stats);
        for mixed in [
            Overlays { nav_grid: true, ..Overlays::NONE },
            Overlays { hitboxes: true, ..Overlays::NONE },
            Overlays { stats: true, ..Overlays::NONE },
        ] {
            assert!(mixed.any(), "{mixed:?}");
            assert_eq!(mixed.next_preset(), Overlays::NONE, "{mixed:?}");
        }
        assert!(!Overlays::NONE.any());
    }

    /// The label names the exact presets and calls everything else custom.
    #[test]
    fn preset_name_reads_inspect_all_custom() {
        assert_eq!(Overlays::NONE.preset_name(), None);
        assert_eq!(Overlays::INSPECT.preset_name(), Some("inspect"));
        assert_eq!(Overlays::ALL.preset_name(), Some("all"));
        assert_eq!(Overlays { hitboxes: true, ..Overlays::NONE }.preset_name(), Some("custom"));
        assert_eq!(Overlays { stats: true, ..Overlays::NONE }.preset_name(), Some("custom"));
        assert_eq!(Overlays { nav_grid: true, ..Overlays::INSPECT }.preset_name(), Some("custom"));
    }
}

#[cfg(test)]
mod spawn_tests {
    use super::*;

    /// No enemy ever starts inside, or touching, a wall on the shipped map.
    /// The default map's border band is dense enough that the spawn
    /// sampler used to hit its attempt cap on every round and hand back a
    /// raw sample, which put roughly one enemy in eleven inside a wall
    /// tile; the sampler now only accepts `battlefield::enemy_spawn_legal`
    /// spots and snaps to `Grid::nearest_open` on a cap, and
    /// `relocate_unusable_spawns` audits the result. Checked as an AABB
    /// test between each enemy's movement collider and every wall tile at
    /// its widest (seam-closed) half-extent, over a spread of seeds and a
    /// crowded enemy count, so a regression in any of the three layers
    /// shows up.
    #[test]
    fn enemies_never_spawn_inside_walls() {
        let wall_half = OBSTACLE_GRID_SIZE * 0.5;
        for seed in 1..=40u64 {
            let mut game = Game::default();
            game.enemy_count_override = Some(8);
            // Band placement is what this test audits, whatever the shipped
            // map's own spawn plan is.
            game.level_overrides.spawn = Some(crate::level::SpawnKind::Band);
            game.seed_override = Some(seed);
            game.map = MapFile::from_toml_str(include_str!("../../maps/default.toml")).expect("embedded default map parses");
            game.init(1280.0, 720.0);
            let walls: Vec<Position> = game.world.query::<&Obstacle>().iter().map(|o| o.position).collect();
            for (tank, _) in game.world.query::<(&Tank, &Ai)>().iter() {
                let (hx, hy) = tank.move_half_extents(false);
                let overlap = walls.iter().find(|w| {
                    (tank.position.x - w.x).abs() < hx + wall_half && (tank.position.y - w.y).abs() < hy + wall_half
                });
                assert!(
                    overlap.is_none(),
                    "seed {seed}: enemy at ({:.1},{:.1}) overlaps wall at {:?}",
                    tank.position.x,
                    tank.position.y,
                    overlap
                );
            }
        }
    }
}

#[cfg(test)]
mod player_chassis_tests {
    use super::*;

    /// The player's chassis has four possible sources; this is the order
    /// they win in, and the RNG is only touched when all three explicit
    /// ones are silent (a map or knob choice must not shift the seeded
    /// stream every later spawn draws from).
    #[test]
    fn chassis_source_precedence() {
        let random = || 3;

        // Nothing set: the round rolls one.
        assert_eq!(resolve_player_row(None, -1, None, random), 3);

        // The map's own chassis, when it names one.
        assert_eq!(resolve_player_row(None, -1, Some(TankKind::Titan), random), 10);

        // The `player_tank` knob outranks the map...
        assert_eq!(resolve_player_row(None, 0, Some(TankKind::Titan), random), 0);

        // ...and `--tank` outranks both.
        assert_eq!(resolve_player_row(Some(5), 0, Some(TankKind::Titan), random), 5);

        // A row from outside the sheet can never reach the sprite tables.
        assert_eq!(resolve_player_row(Some(99), -1, None, random), TANK_VARIANTS - 1);
        assert_eq!(resolve_player_row(None, 99, None, random), TANK_VARIANTS - 1);
    }

    /// A `tank = "titan"` line in a map's TOML is what the player actually
    /// spawns in, and `--tank` (`player_row_override`) still overrides it -
    /// the same map, the same seed, two different chassis.
    #[test]
    fn map_tank_key_spawns_that_chassis() {
        const MAP: &str = r#"
version = 1
tanks = 1
tank = "titan"
cells."20,11" = { kind = "start" }
"#;
        let spawn_row = |cli: Option<i32>| {
            let mut game = Game::default();
            game.enemy_count_override = Some(1);
            game.seed_override = Some(11);
            game.player_row_override = cli;
            game.map = MapFile::from_toml_str(MAP).expect("test map parses");
            game.init(1280.0, 720.0);
            let player = game.player().expect("player entity spawned in init");
            let mut q = game.world.query_one::<&Tank>(player);
            q.get().expect("player has a Tank").row
        };
        assert_eq!(spawn_row(None), TankKind::Titan.row());
        assert_eq!(spawn_row(Some(TankKind::Scout.row())), TankKind::Scout.row());
    }
}

#[cfg(test)]
mod determinism_tests {
    use super::*;

    /// Run one seeded round of `mission` headlessly for `frames` frames of
    /// AFK input at the probe's fixed timestep, sampling `tank_snapshots`
    /// every `sample_every` frames (plus frame 0). `waves` runs it under a
    /// three-wave plan instead of the band.
    fn run_sampled(seed: u64, mission: Mission, frames: u32, sample_every: u32, waves: bool) -> Vec<Vec<TankSnapshot>> {
        let mut game = Game::default();
        game.seed_override = Some(seed);
        game.level_overrides.mission = Some(mission);
        if waves {
            game.level_overrides.spawn = Some(crate::level::SpawnKind::Waves);
            game.level_overrides.waves = Some(3);
            game.level_overrides.wave_size = Some(2);
        } else {
            game.level_overrides.spawn = Some(crate::level::SpawnKind::Band);
            game.enemy_count_override = Some(4);
        }
        game.map = MapFile::from_toml_str(include_str!("../../maps/default.toml")).expect("embedded default map parses");
        game.init(1280.0, 720.0);
        let mut samples = vec![game.tank_snapshots()];
        for frame in 1..=frames {
            game.update(Input::default(), 1.0 / 60.0, 1280.0, 720.0);
            if frame % sample_every == 0 {
                samples.push(game.tank_snapshots());
            }
        }
        samples
    }

    /// Bit-comparable form (`to_bits`), so a mismatch is unambiguous.
    fn key(s: &TankSnapshot) -> (bool, bool, [u32; 8], i32, i32, i32, bool) {
        (
            s.is_player,
            s.entering,
            [
                s.position.x.to_bits(),
                s.position.y.to_bits(),
                s.velocity.x.to_bits(),
                s.velocity.y.to_bits(),
                s.rotation.to_bits(),
                s.damage.to_bits(),
                s.shield_hp.to_bits(),
                s.shield_recharge_delay.to_bits(),
            ],
            s.shells_ammo,
            s.minigun_ammo,
            s.plasma_ammo,
            s.is_wreck,
        )
    }

    /// Two full runs of the same seed must agree bit-for-bit. 600 frames
    /// of an AFK round crosses spawn, patrol, alert sharing, engagement
    /// and firing, so every RNG consumer gets exercised. The Hunt seed adds
    /// the procedural enemy-frog placement, the role rolls, hunters on the
    /// frog ring and guards on their leash.
    #[test]
    fn same_seed_replays_bit_identical() {
        for (seed, mission) in [(0xB0B5_u64, Mission::Protect), (0xC0FFEE_u64, Mission::Protect), (0xF406_u64, Mission::Hunt)] {
            let a = run_sampled(seed, mission, 600, 60, false);
            let b = run_sampled(seed, mission, 600, 60, false);
            assert_eq!(a.len(), b.len(), "seed {seed:#x}: sample counts differ");
            for (i, (sa, sb)) in a.iter().zip(&b).enumerate() {
                assert_eq!(sa.len(), sb.len(), "seed {seed:#x}, sample {i}: tank counts differ");
                for (t, (ta, tb)) in sa.iter().zip(sb).enumerate() {
                    assert_eq!(key(ta), key(tb), "seed {seed:#x}, sample {i}, tank {t}: state diverged");
                }
            }
        }
    }

    /// One and two seats draw the round RNG in exactly the order they drew
    /// it before a round could hold eight, so every seeded replay, probe
    /// fixture and recorded ceiling still describes the same round. The
    /// numbers below are that stream's fingerprint: a change to the order
    /// or the count of the draws `init` and `update` make moves them.
    /// Re-baseline consciously, the way `probe-fixtures` is re-baselined -
    /// never to make this go green.
    #[test]
    fn the_one_and_two_seat_streams_are_pinned() {
        let run = |seats: usize| {
            let mut game = Game::default();
            game.seed_override = Some(0xB0B5);
            game.level_overrides.mission = Some(Mission::Protect);
            game.level_overrides.spawn = Some(crate::level::SpawnKind::Band);
            game.enemy_count_override = Some(4);
            game.players = PlayerCount::from_count(seats).expect("one or two");
            game.map = MapFile::from_toml_str(include_str!("../../maps/default.toml")).expect("embedded default map parses");
            game.init(1280.0, 720.0);
            // FNV-1a over the same bit-exact key the replay tests compare.
            let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
            let mut eat = |bytes: &[u8]| {
                for b in bytes {
                    hash ^= *b as u64;
                    hash = hash.wrapping_mul(0x1000_0000_01b3);
                }
            };
            for frame in 0..=600u32 {
                if frame > 0 {
                    game.update(Input::default(), 1.0 / 60.0, 1280.0, 720.0);
                }
                if frame % 60 != 0 {
                    continue;
                }
                for t in game.tank_snapshots() {
                    eat(&(t.slot as u64).to_le_bytes());
                    let k = key(&t);
                    for word in k.2 {
                        eat(&word.to_le_bytes());
                    }
                    eat(&[k.0 as u8, k.1 as u8, k.6 as u8]);
                    eat(&k.3.to_le_bytes());
                }
            }
            hash
        };
        // **Re-baselined for master's seeker missiles, deliberately.**
        // `PickupKind` gained a variant, so the Health-slot bonus roll
        // draws a different number, and the projectile speeds moved - both
        // shift where a seeded round's tanks end up, and neither has
        // anything to do with the walk order this gate exists to protect.
        // What it protects is unchanged: the per-seat block still runs once
        // per seat after player 1, so a second seat does not disturb the
        // first's stream. Never bump these to go green - work out which
        // change moved them first.
        assert_eq!(run(1), 3_330_505_246_546_623_918, "one seat");
        assert_eq!(run(2), 15_003_608_774_004_553_797, "two seats");
    }

    /// A portal round replays too: the destination draw sits on the round
    /// stream and the arrival placement is a deterministic search. The
    /// fixture puts every enemy on the far side of an iron wall from the
    /// frog, so within 900 frames at least one hops - both runs must show
    /// the same `Teleported` events on the same frames.
    #[test]
    fn same_seed_replays_a_portal_round_bit_identical() {
        let run = |seed: u64| {
            let mut game = Game::default();
            game.seed_override = Some(seed);
            game.map = MapFile::from_toml_str(include_str!("../../maps/test/portals.toml")).expect("fixture parses");
            let (w, h) = game.map.field_size();
            game.init(w, h);
            let mut samples = vec![game.tank_snapshots()];
            let mut events: Vec<String> = Vec::new();
            for frame in 1..=900u32 {
                game.update(Input::default(), 1.0 / 60.0, w, h);
                for e in game.events() {
                    if let Event::Teleported { .. } = e {
                        events.push(format!("{frame}:{e:?}"));
                    }
                }
                if frame % 60 == 0 {
                    samples.push(game.tank_snapshots());
                }
            }
            (samples, events)
        };
        for seed in [0xB0B5_u64, 0xC0FFEE_u64] {
            let (a, ea) = run(seed);
            let (b, eb) = run(seed);
            assert!(!ea.is_empty(), "seed {seed:#x}: nobody came through a portal in 900 frames");
            assert_eq!(ea, eb, "seed {seed:#x}: teleport events diverged");
            for (i, (sa, sb)) in a.iter().zip(&b).enumerate() {
                assert_eq!(sa.len(), sb.len(), "seed {seed:#x}, sample {i}: tank counts differ");
                for (t, (ta, tb)) in sa.iter().zip(sb).enumerate() {
                    assert_eq!(key(ta), key(tb), "seed {seed:#x}, sample {i}, tank {t}: state diverged");
                }
            }
        }
    }

    /// A waves round replays too: gate draws, tier/chassis rolls, the
    /// per-tank spawn rolls and the roll-in all sit on the round stream.
    /// 900 frames covers wave 1 rolling in and engaging.
    #[test]
    fn same_seed_replays_a_waves_round_bit_identical() {
        for seed in [0xB0B5_u64, 0xC0FFEE_u64] {
            let a = run_sampled(seed, Mission::Destroy, 900, 60, true);
            let b = run_sampled(seed, Mission::Destroy, 900, 60, true);
            assert_eq!(a.len(), b.len(), "seed {seed:#x}: sample counts differ");
            assert!(a.last().unwrap().len() > 1, "seed {seed:#x}: wave 1 never arrived");
            for (i, (sa, sb)) in a.iter().zip(&b).enumerate() {
                assert_eq!(sa.len(), sb.len(), "seed {seed:#x}, sample {i}: tank counts differ");
                for (t, (ta, tb)) in sa.iter().zip(sb).enumerate() {
                    assert_eq!(key(ta), key(tb), "seed {seed:#x}, sample {i}, tank {t}: state diverged");
                }
            }
        }
    }
}

#[cfg(test)]
mod mechanics_tests {
    use super::*;

    const W: f32 = 1280.0;
    const H: f32 = 720.0;

    fn game_on(map: &str, enemies: usize, row: Option<i32>) -> Game {
        let mut game = Game::default();
        game.enemy_count_override = Some(enemies);
        game.seed_override = Some(7);
        game.player_row_override = row;
        game.map = MapFile::from_toml_str(map).expect("test map parses");
        game.init(W, H);
        game
    }

    fn step(game: &mut Game, input: Input) {
        game.update(input, 1.0 / 60.0, W, H);
    }

    /// An empty sandbox with `water` cells: the player starts at (5, 11)
    /// facing a 5 x 5 lake whose 3 x 3 interior is deep, a north-south
    /// stream at column 26 from row 3 to row 19, a sideways stream along
    /// row 4 from column 8 to column 14, and a brick wall at (32, 11)
    /// beyond the lake for a shell to hit.
    fn water_map(extra: &str) -> String {
        let mut map = String::from("version = 1\ntanks = 0\ncells.\"5,11\" = { kind = \"start\" }\ncells.\"32,11\" = { kind = \"wall\", material = \"brick\" }\n");
        for c in 14..=18 {
            for r in 9..=13 {
                map.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"water\" }}\n"));
            }
        }
        for r in 3..=19 {
            map.push_str(&format!("cells.\"26,{r}\" = {{ kind = \"water\" }}\n"));
        }
        for c in 8..=14 {
            map.push_str(&format!("cells.\"{c},4\" = {{ kind = \"water\" }}\n"));
        }
        map.push_str(extra);
        map
    }

    fn sandbox(map: &str) -> Game {
        let mut game = Game::default();
        game.seed_override = Some(7);
        game.level_overrides.mission = Some(Mission::Destroy);
        game.map = MapFile::from_toml_str(map).expect("test map parses");
        game.init(W, H);
        game
    }

    fn player_pos(game: &Game) -> Position {
        game.tank_snapshots().iter().find(|t| t.is_player).expect("player").position
    }

    fn teleport_player(game: &mut Game, pos: Position) {
        let player = game.player().expect("player");
        let body = with_tank(&game.world, player, |t| t.body).expect("body");
        game.physics.set_position(body, pos);
        with_tank_mut(&game.world, player, |t| t.position = pos);
    }

    fn drive(game: &mut Game, dir: Option<Dir>, frames: usize) {
        for _ in 0..frames {
            let mut input = Input::default();
            input.seats[0].move_dir = dir;
            step(game, input);
        }
    }

    /// A local round runs its own cosmetics inside `update` and never
    /// calls `tick_presentation`, which is the only reason the two can
    /// share their halves: one `update` advances the round clock and a
    /// hull's drawn angle by exactly one step, and the tread marks come
    /// off the physics step, so `Tank::track_from` - the replica's
    /// displacement record - is never written.
    #[test]
    fn local_play_ticks_its_cosmetics_once() {
        let dt = 1.0 / 60.0;
        let mut game = sandbox("version = 1\ntanks = 0\ncells.\"5,5\" = { kind = \"start\" }\n");
        // Facing up, commanded right: `rotation` snaps, the drawn angle
        // swings across at `tank_visual_turn_speed_deg`.
        let mut input = Input::default();
        input.seats[0].move_dir = Some(Dir::Right);
        step(&mut game, input);
        let player = game.player().expect("player");
        let step_deg = tuning().tank_visual_turn_speed_deg * dt;
        let (visual, rotation, from) =
            with_tank(&game.world, player, |t| (t.visual_rotation, t.rotation, t.track_from));
        assert_eq!(rotation, Dir::Right.rotation(), "the hull's facing snaps");
        assert!((visual - step_deg).abs() < 1e-4, "one update, one ease step: {visual} for {step_deg}");
        assert_eq!(from, None, "a local round lays its marks from the physics step");
        assert!((game.time - dt).abs() < 1e-6, "one update, one dt of round clock: {}", game.time);
        // And the presentation tick a replica runs is a second step on
        // top, which is why `update` must not call it.
        game.tick_presentation(dt);
        let after = with_tank(&game.world, player, |t| t.visual_rotation);
        assert!((after - 2.0 * step_deg).abs() < 1e-4, "the presentation tick eases again: {after}");
    }

    /// `Input::held_only` is the input for a rendered frame's second and
    /// later steps: the held state stays, the one-shot presses are spent.
    #[test]
    fn held_only_spends_the_presses_and_keeps_the_held_state() {
        let mut input = Input::two(Intent { move_dir: Some(Dir::Up), fire: true, ..Intent::default() }, Intent { fire: true, ..Intent::default() });
        input.pause_pressed = true;
        input.restart_pressed = true;
        input.toggle_shadows_pressed = true;
        input.cycle_overlays_pressed = true;
        let later = input.held_only();
        assert_eq!(later.seats[0].move_dir, Some(Dir::Up));
        assert!(later.seats[0].fire && later.seats[1].fire);
        assert!(!later.pause_pressed && !later.restart_pressed && !later.toggle_shadows_pressed && !later.cycle_overlays_pressed);
    }

    /// `Input::or_presses` folds a frame that ran no step into the next:
    /// its fire keys and toggles are still pressed, its directions are
    /// not, and a seat past the array reads as idle.
    #[test]
    fn or_presses_carries_fire_and_toggles_but_never_a_direction() {
        let mut missed = Input::two(Intent { move_dir: Some(Dir::Left), fire: true, ..Intent::default() }, Intent { move_dir: Some(Dir::Down), ..Intent::default() });
        missed.restart_pressed = true;
        let now = Input::single(Intent { move_dir: Some(Dir::Right), ..Intent::default() }).or_presses(missed);
        assert_eq!(now.seats[0].move_dir, Some(Dir::Right));
        assert!(now.seats[0].fire, "the tap between two steps still fires");
        assert_eq!(now.seats[1].move_dir, None);
        assert!(!now.seats[1].fire);
        assert!(now.restart_pressed && !now.pause_pressed);
        assert_eq!(now.seat(MAX_SEATS).move_dir, None);
        assert!(!now.seat(MAX_SEATS + 3).fire);
    }

    /// The round reads the first `players.count()` seats and nothing
    /// else: in a single-player round seat 1 can neither skip the mission
    /// banner nor fire, while seat 0 skips it at once.
    #[test]
    fn a_single_player_round_ignores_the_second_seat() {
        let intro_round = || {
            let mut game = game_on(OPEN_MAP, 1, Some(0));
            game.show_intro = true;
            game.init(W, H);
            assert!(game.intro_timer > 0.0, "the banner is up");
            game
        };
        let mut game = intro_round();
        let mut seat1 = Input::default();
        seat1.seats[1] = Intent { move_dir: Some(Dir::Up), fire: true, ..Intent::default() };
        for _ in 0..3 {
            step(&mut game, seat1);
        }
        assert!(game.intro_timer > 0.0, "seat 1 is not read with one player");
        assert!(!game.events().iter().any(|e| matches!(e, Event::Fired { .. })));
        let mut game = intro_round();
        step(&mut game, Input::single(Intent { fire: true, ..Intent::default() }));
        assert_eq!(game.intro_timer, 0.0, "seat 0 skips the banner");
    }

    /// The frame's routing grid prices the cells down the player's
    /// barrel (`route_lane_cost` on each, out to `route_lane_cells`), so
    /// the field toward the player is dearer straight up the lane than
    /// round the side: from four cells in front the cheapest route
    /// leaves the lane, while four cells to the flank cost the plain
    /// four steps. A lane cell is priced, never blocked.
    #[test]
    fn the_route_grid_prices_the_players_firing_lane() {
        let mut game = sandbox("version = 1\ntanks = 0\ncells.\"5,11\" = { kind = \"start\" }\n");
        teleport_player(&mut game, map::cell_to_world(10, 8));
        // Two frames east: the hull snaps to face right and barely moves.
        drive(&mut game, Some(Dir::Right), 2);
        let player = player_pos(&game);
        assert!(game.grid_cell_of(player) == (10, 8), "still in its cell: {player:?}");
        let grid = game.route_grid(W, H);
        let ahead = map::cell_to_world(14, 8);
        let flank = map::cell_to_world(10, 12);
        assert!(grid.usable(map::cell_to_world(12, 8)), "a lane cell is open");
        assert_eq!(grid.path_cost(flank, player), Some(4), "the flank costs its four steps");
        let up_the_lane = grid.path_cost(ahead, player).expect("routes");
        assert!(up_the_lane > 4, "the lane is dearer than the four steps: {up_the_lane}");
        // The cheapest route out of the lane is round it: one step
        // aside, four along, one back in - six, not three lane cells at
        // four each plus one.
        assert_eq!(up_the_lane, 6);
        let first = grid.next_step(ahead, player).expect("routes");
        assert_ne!(game.grid_cell_of(first), (13, 8), "the first step leaves the lane");
    }

    #[test]
    fn deep_water_stops_a_hull_but_not_a_shell() {
        let mut game = sandbox(&water_map(""));
        assert_eq!(game.water().deep_cells().count(), 9, "the 3 x 3 interior is deep");
        // Drive east into the lake for a long while: the hull stops at
        // the deep edge (the deep cells start at column 15, whose left
        // face is at x = 464) and never gets past it.
        teleport_player(&mut game, map::cell_to_world(11, 11));
        drive(&mut game, Some(Dir::Right), 420);
        let p = player_pos(&game);
        assert!(p.x < 464.0, "the hull stopped short of the deep water: {p:?}");
        assert!(p.x > 400.0, "but it did wade into the shore first: {p:?}");
        assert!((p.y - 352.0).abs() < 8.0, "and slid along nothing: {p:?}");
        // A shell fired from the same spot crosses the whole lake and
        // lands on the wall beyond it.
        let mut input = Input::default();
        input.seats[0].fire = true;
        step(&mut game, input);
        for _ in 0..240 {
            step(&mut game, Input::default());
            if game.events().iter().any(|e| matches!(e, Event::Hit { target: HitTarget::Obstacle { .. }, .. })) {
                return;
            }
        }
        panic!("the shell never reached the wall beyond the lake");
    }

    #[test]
    fn a_ford_slows_a_hull_and_takes_no_tread_marks() {
        // The same drive on dry ground and through the sideways stream
        // (row 4, columns 8..=14): start two cells short of it and drive
        // east for two seconds.
        let mut dry = sandbox(&water_map(""));
        teleport_player(&mut dry, map::cell_to_world(6, 14));
        drive(&mut dry, Some(Dir::Right), 120);
        let dry_dist = player_pos(&dry).x - map::cell_to_world(6, 14).x;

        let mut wet = sandbox(&water_map(""));
        teleport_player(&mut wet, map::cell_to_world(6, 4));
        drive(&mut wet, Some(Dir::Right), 120);
        let wet_dist = player_pos(&wet).x - map::cell_to_world(6, 4).x;
        assert!(wet_dist < dry_dist * 0.8, "wading is slower: dry {dry_dist:.0} px, wet {wet_dist:.0} px");
        assert!(wet_dist > dry_dist * 0.3, "but it is not a wall: dry {dry_dist:.0} px, wet {wet_dist:.0} px");
        assert!(player_pos(&wet).x > map::cell_to_world(8, 4).x, "the hull is in the stream by now");
        // No mark was pressed into the stream bed; the ones on the bank
        // before it are dry, since the hull had not waded yet.
        let stream_left = map::cell_to_world(8, 4).x - 16.0;
        assert!(wet.tracks.iter().all(|t| t.position.x < stream_left), "no tread mark in the water");
        assert!(wet.tracks.iter().all(|t| !t.wet));
        // Wading out again leaves wet marks for a while.
        drive(&mut wet, Some(Dir::Down), 90);
        assert!(wet.tracks.iter().any(|t| t.wet), "the marks on the far bank are wet");
    }

    #[test]
    fn the_current_carries_an_idle_hull_south_on_a_stream_that_runs_that_way() {
        let mut game = sandbox(&water_map(""));
        let start = map::cell_to_world(26, 8);
        teleport_player(&mut game, start);
        drive(&mut game, None, 180);
        let p = player_pos(&game);
        let expect = tuning().water_current_speed * 3.0;
        assert!(p.y > start.y + expect * 0.6, "drifted south with the current: {p:?}, start {start:?}");
        assert!((p.x - start.x).abs() < 4.0, "and only south");
        // The sideways stream has no current.
        let mut game = sandbox(&water_map(""));
        let start = map::cell_to_world(11, 4);
        teleport_player(&mut game, start);
        drive(&mut game, None, 180);
        assert!(player_pos(&game).distance_to(start) < 2.0, "a sideways stream holds still: {:?}", player_pos(&game));
    }

    #[test]
    fn water_puts_a_burning_hull_out_and_takes_no_fire() {
        let mut game = sandbox(&water_map(""));
        let player = game.player().expect("player");
        teleport_player(&mut game, map::cell_to_world(11, 4));
        with_tank_mut(&game.world, player, |t| t.burn_timer = 5.0);
        step(&mut game, Input::default());
        assert_eq!(with_tank(&game.world, player, |t| t.burn_timer), 0.0, "the afterburn went out in the stream");
        // A cell of water refuses to light, a dry one beside it does.
        let (w, h) = (W, H);
        let mut f = Frame::new(1.0 / 60.0, w, h, SmallRng::seed_from_u64(1), Terrain::build(&game.world, w, h, &game.grass_cells, &game.water));
        game.light_cell(&mut f, (10, 4), 3.0, true);
        game.light_cell(&mut f, (10, 6), 3.0, true);
        assert_eq!(game.burning_cells().len(), 1, "only the dry cell lit");
        assert_eq!(game.burning_cells()[0].0, map::cell_to_world(10, 6));
        assert!(f.events.iter().all(|e| !matches!(e, Event::FireStarted { y, .. } if *y == map::cell_to_world(10, 4).y)));
    }

    #[test]
    fn a_start_in_deep_water_spawns_on_the_shore() {
        // No start cell, so the player falls back to the cell nearest the
        // centre - (20, 11) on this field - which a 5 x 5 lake makes deep.
        let mut map = String::from("version = 1\ntanks = 0\n");
        for c in 18..=22 {
            for r in 9..=13 {
                map.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"water\" }}\n"));
            }
        }
        let game = sandbox(&map);
        assert_eq!(game.water().depth_at(map::cell_to_world(20, 11)), crate::ground::Depth::Deep);
        let p = player_pos(&game);
        assert_eq!(game.water().depth_at(p), crate::ground::Depth::Shallow, "moved to the nearest shore cell: {p:?}");
        // The shore ring is two cells out, diagonally at most.
        assert!(p.distance_to(map::cell_to_world(20, 11)) <= 2.0 * OBSTACLE_GRID_SIZE * std::f32::consts::SQRT_2 + 1.0);
    }

    #[test]
    fn the_router_prices_a_ford_and_walls_off_deep_water() {
        // The north-south stream at column 26 spans rows 3..=19 of a
        // 22.5-row field: going round it from (24, 11) to (28, 11) is far
        // longer than the ford's price, so the route wades - three dry
        // steps and one ford.
        let game = sandbox(&water_map(""));
        let ford = tuning().water_ford_path_cost as u32;
        let cost = game.nav_path_cells(map::cell_to_world(24, 11), map::cell_to_world(28, 11), W, H).expect("routes");
        assert_eq!(cost, 3 + ford);
        // Straight across the sideways stream is exactly one ford dearer
        // than a dry run of the same length.
        let dry = game.nav_path_cells(map::cell_to_world(20, 14), map::cell_to_world(20, 18), W, H).expect("routes");
        let crossing = game.nav_path_cells(map::cell_to_world(11, 2), map::cell_to_world(11, 6), W, H).expect("routes");
        assert_eq!(dry, 4);
        assert_eq!(crossing, dry - 1 + ford);
        // The lake's deep middle is not routable at all; its shore is.
        let grid = game.nav_grid(W, H);
        assert!(!grid.usable(map::cell_to_world(16, 11)));
        assert!(grid.next_step(map::cell_to_world(11, 11), map::cell_to_world(16, 11)).is_none() || !grid.usable(map::cell_to_world(16, 11)));
    }

    #[test]
    fn a_destroy_round_has_no_frog_and_ends_on_the_last_wreck() {
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(7);
        game.level_overrides.mission = Some(Mission::Destroy);
        game.map = MapFile::from_toml_str(OPEN_MAP).expect("test map parses");
        game.init(W, H);
        assert_eq!(game.mission, Mission::Destroy);
        assert!(game.frog.is_none() && game.enemy_frog.is_none());
        assert_eq!(game.world.query::<&Frog>().iter().count(), 0);
        assert!(matches!(game.events(), [Event::RoundStarted { mission: Mission::Destroy, .. }]));
        let slot = game.world.query::<&Tank>().with::<&Ai>().iter().map(|t| t.owner_slot()).next().unwrap();
        game.debug_kill(slot).unwrap();
        step(&mut game, Input::default());
        assert_eq!(game.outcome(), Outcome::Won);
    }

    /// `tanks = 0` is a sandbox: no enemy ever spawns and the round does
    /// not end just because there is nobody to wreck.
    #[test]
    fn a_round_with_no_enemies_is_a_sandbox_that_never_ends_on_its_own() {
        let mut game = Game::default();
        game.seed_override = Some(7);
        game.level_overrides.mission = Some(Mission::Destroy);
        game.map = MapFile::from_toml_str("version = 1\ntanks = 0\ncells.\"20,11\" = { kind = \"start\" }\n").expect("parses");
        game.init(W, H);
        assert_eq!(game.world.query::<&Tank>().with::<&Ai>().iter().count(), 0);
        for _ in 0..600 {
            step(&mut game, Input::default());
        }
        assert_eq!(game.outcome(), Outcome::Playing, "an empty round must not end by wreck count");
    }

    #[test]
    fn the_intro_freezes_the_round_and_any_input_skips_it() {
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(7);
        game.show_intro = true;
        game.map = MapFile::from_toml_str(OPEN_MAP).expect("test map parses");
        game.init(W, H);
        let expected = (tuning().mission_banner_seconds * 60.0).round() as u32;
        let mut frozen = 0;
        while game.intro_timer > 0.0 {
            step(&mut game, Input::default());
            assert_eq!(game.time, 0.0, "the world does not advance behind the banner");
            frozen += 1;
            assert!(frozen <= expected + 1, "the intro never ended");
        }
        assert!(frozen >= expected - 1, "froze only {frozen} frames, expected about {expected}");
        step(&mut game, Input::default());
        assert!(game.time > 0.0, "play resumes once the banner runs out");

        game.init(W, H);
        assert!(game.intro_timer > 0.0);
        let mut input = Input::default();
        input.seats[0].fire = true;
        step(&mut game, input);
        assert_eq!(game.intro_timer, 0.0, "fire skips the intro");
        step(&mut game, Input::default());
        assert!(game.time > 0.0);
    }

    fn player_ammo(game: &Game) -> i32 {
        game.tank_snapshots().iter().find(|t| t.is_player).expect("player").shells_ammo
    }

    /// Give the single enemy of a one-enemy round `role` - set directly on
    /// its `Ai`, not rolled, so the test never depends on the share knobs.
    fn set_enemy_role(game: &mut Game, role: Role) {
        let enemies: Vec<Entity> = game.world.query::<(Entity, &Ai)>().iter().map(|(e, _)| e).collect();
        assert_eq!(enemies.len(), 1, "one-enemy round");
        let mut q = game.world.query_one::<&mut Ai>(enemies[0]);
        q.get().expect("enemy has an Ai").role = role;
    }

    /// Give the enemy in owner slot `slot` `role` directly (see
    /// `set_enemy_role`).
    fn set_role_of(game: &mut Game, slot: usize, role: Role) {
        let entity = game
            .world
            .query::<(Entity, &Tank)>()
            .with::<&Ai>()
            .iter()
            .find(|(_, t)| t.owner_slot() == slot)
            .map(|(e, _)| e)
            .expect("enemy in that slot");
        let mut q = game.world.query_one::<&mut Ai>(entity);
        q.get().expect("enemy has an Ai").role = role;
    }

    fn position_of(game: &Game, slot: usize) -> Position {
        game.tank_snapshots().into_iter().find(|t| t.slot == slot).expect("tank in slot").position
    }

    /// A 112 px wide vertical iron corridor (two nav cells): the player at
    /// its top, the frog at its bottom - the default map's lane beside the
    /// player start, where two enemies heading opposite ways jammed.
    const CORRIDOR_MAP: &str = r#"
version = 1
tanks = 2
cells."14,1" = { kind = "start" }
cells."14,22" = { kind = "frog" }
cells."14,20" = { kind = "pickup", pickup = "ammo" }
cells."11,2" = { kind = "wall", material = "iron" }
cells."11,3" = { kind = "wall", material = "iron" }
cells."11,4" = { kind = "wall", material = "iron" }
cells."11,5" = { kind = "wall", material = "iron" }
cells."11,6" = { kind = "wall", material = "iron" }
cells."11,7" = { kind = "wall", material = "iron" }
cells."11,8" = { kind = "wall", material = "iron" }
cells."11,9" = { kind = "wall", material = "iron" }
cells."11,10" = { kind = "wall", material = "iron" }
cells."11,11" = { kind = "wall", material = "iron" }
cells."11,12" = { kind = "wall", material = "iron" }
cells."11,13" = { kind = "wall", material = "iron" }
cells."11,14" = { kind = "wall", material = "iron" }
cells."11,15" = { kind = "wall", material = "iron" }
cells."11,16" = { kind = "wall", material = "iron" }
cells."11,17" = { kind = "wall", material = "iron" }
cells."11,18" = { kind = "wall", material = "iron" }
cells."11,19" = { kind = "wall", material = "iron" }
cells."11,20" = { kind = "wall", material = "iron" }
cells."16,2" = { kind = "wall", material = "iron" }
cells."16,3" = { kind = "wall", material = "iron" }
cells."16,4" = { kind = "wall", material = "iron" }
cells."16,5" = { kind = "wall", material = "iron" }
cells."16,6" = { kind = "wall", material = "iron" }
cells."16,7" = { kind = "wall", material = "iron" }
cells."16,8" = { kind = "wall", material = "iron" }
cells."16,9" = { kind = "wall", material = "iron" }
cells."16,10" = { kind = "wall", material = "iron" }
cells."16,11" = { kind = "wall", material = "iron" }
cells."16,12" = { kind = "wall", material = "iron" }
cells."16,13" = { kind = "wall", material = "iron" }
cells."16,14" = { kind = "wall", material = "iron" }
cells."16,15" = { kind = "wall", material = "iron" }
cells."16,16" = { kind = "wall", material = "iron" }
cells."16,17" = { kind = "wall", material = "iron" }
cells."16,18" = { kind = "wall", material = "iron" }
cells."16,19" = { kind = "wall", material = "iron" }
cells."16,20" = { kind = "wall", material = "iron" }
"#;

    /// Two enemies pressed corner to corner in the corridor, one heading
    /// up to the player and one down to the ammo, half a hull apart in x
    /// so each blocks the other's lane. Neither is moving, but the contact
    /// solver keeps handing each a burst of velocity along its heading;
    /// the stuck escape must still fire and one of them must step aside.
    #[test]
    fn two_enemies_jammed_corner_to_corner_in_a_corridor_break_free() {
        let mut game = game_on(CORRIDOR_MAP, 2, Some(0));
        set_role_of(&mut game, 1, Role::Player);
        set_role_of(&mut game, 2, Role::Player);
        // Player at the very top, out of attack range, so the lower enemy
        // chases up the lane; the upper one is out of shells and retreats
        // down it to the ammo at the bottom. Hulls overlap by half a width.
        game.debug_teleport(0, Position::new(448.0, 24.0), Some(180.0)).expect("player");
        game.debug_set_tank(2, &crate::simulation::debug::TankPatch { shells_ammo: Some(0), ..Default::default() })
            .expect("slot 2");
        let a = Position::new(456.0, 402.0);
        let b = Position::new(440.0, 366.0);
        game.debug_teleport(1, a, Some(0.0)).expect("slot 1");
        game.debug_teleport(2, b, Some(180.0)).expect("slot 2");
        let mut freed_at = None;
        for frame in 1..=(4 * 60) {
            step(&mut game, Input::default());
            let (pa, pb) = (position_of(&game, 1), position_of(&game, 2));
            // Someone stepped aside or got past: the pair is no longer
            // sitting on its starting spots.
            if pa.distance_to(a) > 30.0 || pb.distance_to(b) > 30.0 {
                freed_at = Some(frame);
                break;
            }
        }
        let frame = freed_at.expect("the two enemies never broke out of the jam within 4 s");
        assert!(frame as f32 / 60.0 < 3.0, "took {frame} frames to break the jam");
    }

    fn frog_position(game: &Game, frog: Option<Entity>) -> Position {
        with_frog(&game.world, frog.expect("frog on the field"), |fr| fr.position)
    }

    fn enemy_position(game: &Game) -> Position {
        game.tank_snapshots().into_iter().find(|t| !t.is_player).expect("enemy").position
    }

    /// The player in the top-left corner, its frog two cells over, the
    /// enemy frog in the far bottom-right: a hunter has a long, clear run
    /// to the player's frog and a guard's beat is far from the player.
    const CORNERS_MAP: &str = r#"
version = 1
tanks = 1
cells."3,3" = { kind = "start" }
cells."36,20" = { kind = "frog" }
cells."36,3" = { kind = "enemy_frog" }
"#;

    #[test]
    fn a_hunter_drives_to_the_players_frog() {
        let mut game = game_on(CORNERS_MAP, 1, Some(0));
        set_enemy_role(&mut game, Role::Hunter);
        let frog = frog_position(&game, game.frog);
        // Far left on the frog's row: a straight eastward run along y=656
        // that never lines up on the player in the corner.
        game.debug_teleport(1, Position::new(200.0, frog.y), Some(90.0)).expect("enemy in slot 1");
        let attack_range = tuning().enemy_attack_range;
        let mut last = enemy_position(&game).distance_to(frog);
        assert!(last > 900.0, "starts {last} px from the frog");
        let mut arrived = false;
        for second in 1..=8 {
            for _ in 0..60 {
                step(&mut game, Input::default());
            }
            let now = enemy_position(&game).distance_to(frog_position(&game, game.frog));
            if now <= attack_range {
                arrived = true;
                break;
            }
            assert!(now < last - 60.0, "second {second}: {last:.0} -> {now:.0} px, not closing on the frog");
            last = now;
        }
        assert!(arrived, "never reached attack range of the frog ({last:.0} px)");
        assert_eq!(game.outcome(), Outcome::Playing);
    }

    /// The frog inside a bunker like the default map's: brick above and
    /// below, iron on both sides, one open cell around it. No line of
    /// sight exists from anywhere; the only way in is through the brick.
    const BUNKER_MAP: &str = r#"
version = 1
tanks = 1
cells."3,3" = { kind = "start" }
cells."20,12" = { kind = "frog" }
cells."18,10" = { kind = "wall", material = "brick" }
cells."19,10" = { kind = "wall", material = "brick" }
cells."20,10" = { kind = "wall", material = "brick" }
cells."21,10" = { kind = "wall", material = "brick" }
cells."22,10" = { kind = "wall", material = "brick" }
cells."18,11" = { kind = "wall", material = "iron" }
cells."18,12" = { kind = "wall", material = "iron" }
cells."18,13" = { kind = "wall", material = "iron" }
cells."22,11" = { kind = "wall", material = "iron" }
cells."22,12" = { kind = "wall", material = "iron" }
cells."22,13" = { kind = "wall", material = "iron" }
cells."18,14" = { kind = "wall", material = "brick" }
cells."19,14" = { kind = "wall", material = "brick" }
cells."20,14" = { kind = "wall", material = "brick" }
cells."21,14" = { kind = "wall", material = "brick" }
cells."22,14" = { kind = "wall", material = "brick" }
"#;



    #[test]
    fn a_hunter_shoots_its_way_into_the_frogs_bunker() {
        let mut game = game_on(BUNKER_MAP, 1, Some(0));
        set_enemy_role(&mut game, Role::Hunter);
        let frog = frog_position(&game, game.frog);
        // Straight above the bunker, facing down, inside attack range: the
        // brick roof is all that stands between the hunter and the frog.
        game.debug_teleport(1, Position::new(frog.x, frog.y - tuning().enemy_attack_range * 0.8), Some(180.0))
            .expect("enemy in slot 1");
        let (mut broke_a_tile, mut hit_the_frog) = (false, false);
        for _ in 0..(20 * 60) {
            step(&mut game, Input::default());
            for event in game.events() {
                match event {
                    Event::Hit { target: HitTarget::Obstacle { .. }, .. } => broke_a_tile = true,
                    Event::Hit { target: HitTarget::Frog { side: Side::Player }, .. } => hit_the_frog = true,
                    _ => {}
                }
            }
            if hit_the_frog {
                break;
            }
        }
        assert!(broke_a_tile, "the hunter never fired at the bunker's brick");
        assert!(hit_the_frog, "the hunter never got a shell through to the frog");
    }

    #[test]
    fn a_hunter_in_range_with_line_of_sight_shoots_the_player() {
        let mut game = game_on(OPEN_MAP, 1, Some(0));
        set_enemy_role(&mut game, Role::Hunter);
        let player = player_snapshot(&game).position;
        // Straight above the player, half an attack range away, facing it;
        // the frog is off in the bottom-right.
        game.debug_teleport(1, Position::new(player.x, player.y - tuning().enemy_attack_range * 0.5), Some(180.0))
            .expect("enemy in slot 1");
        let (mut fired, mut hit_player) = (false, false);
        for _ in 0..300 {
            step(&mut game, Input::default());
            for e in game.events() {
                match e {
                    Event::Fired { slot: 1, .. } => fired = true,
                    Event::Hit { target: HitTarget::Player { .. }, .. } => hit_player = true,
                    _ => {}
                }
            }
            if hit_player {
                break;
            }
        }
        assert!(fired && hit_player, "fired: {fired}, hit the player: {hit_player}");
        assert!(player_snapshot(&game).damage > 0.0);
    }

    #[test]
    fn a_guard_never_leaves_its_leash() {
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(7);
        game.player_row_override = Some(0);
        game.level_overrides.mission = Some(Mission::Hunt);
        game.map = MapFile::from_toml_str(CORNERS_MAP).expect("test map parses");
        game.init(W, H);
        assert_eq!(game.mission, Mission::Hunt);
        set_enemy_role(&mut game, Role::Guard);
        let home = frog_position(&game, game.enemy_frog);
        assert_eq!(home, map::cell_to_world(36, 3), "the map's enemy_frog cell places the enemy frog");
        let leash = tuning().guard_leash_px;
        assert!(player_snapshot(&game).position.distance_to(home) > leash * 2.0, "the player starts far from the beat");
        game.debug_teleport(1, Position::new(home.x - 150.0, home.y), Some(270.0)).expect("enemy in slot 1");
        // Slack: waypoints stay inside the leash but the hull turns around
        // a little past one, and the frog hops away from any tank that
        // comes close - its own guard included - which moves the anchor by
        // a hop before the guard turns back.
        let hull = Tank::default().size();
        let hop = with_frog(&game.world, game.enemy_frog.unwrap(), Frog::hop_distance);
        let (mut outside, mut moved) = (0, 0.0);
        let mut prev = enemy_position(&game);
        for frame in 0..900 {
            step(&mut game, Input::default());
            let pos = enemy_position(&game);
            let dist = pos.distance_to(frog_position(&game, game.enemy_frog));
            assert!(dist <= leash + hull + hop, "frame {frame}: guard {dist:.0} px from its frog (leash {leash})");
            outside += (dist > leash + hull) as u32;
            moved += pos.distance_to(prev);
            prev = pos;
        }
        assert!(outside < 90, "outside the leash on {outside} of 900 frames: not turning back");
        assert!(moved > 200.0, "the guard wanders its beat rather than parking (moved {moved:.0} px)");
        assert_eq!(game.outcome(), Outcome::Playing);
    }

    #[test]
    fn a_hunt_round_is_won_when_the_enemy_frog_dies_and_lost_when_the_players_does() {
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(7);
        game.level_overrides.mission = Some(Mission::Hunt);
        game.map = MapFile::from_toml_str(CORNERS_MAP).expect("test map parses");
        game.init(W, H);
        assert_eq!(game.world.query::<&Frog>().iter().count(), 2);
        let sides: Vec<Side> = [game.frog, game.enemy_frog]
            .into_iter()
            .map(|e| with_frog(&game.world, e.expect("both frogs"), |fr| fr.side))
            .collect();
        assert_eq!(sides, [Side::Player, Side::Enemy]);

        with_frog_mut(&game.world, game.enemy_frog.unwrap(), |fr| fr.damage(fr.max_health));
        step(&mut game, Input::default());
        assert_eq!(game.outcome(), Outcome::Won, "the enemy frog's death wins the hunt");
        assert!(game.events().iter().any(|e| matches!(e, Event::RoundEnded { outcome: Outcome::Won })));

        game.init(W, H);
        assert_eq!(game.outcome(), Outcome::Playing);
        with_frog_mut(&game.world, game.frog.unwrap(), |fr| fr.damage(fr.max_health));
        step(&mut game, Input::default());
        assert_eq!(game.outcome(), Outcome::Lost, "the player's frog's death loses the hunt");
    }

    #[test]
    fn a_hunt_map_without_an_enemy_frog_cell_places_one_in_the_band() {
        let mut game = Game::default();
        game.enemy_count_override = Some(2);
        game.seed_override = Some(11);
        game.level_overrides.mission = Some(Mission::Hunt);
        game.map = MapFile::from_toml_str(OPEN_MAP).expect("test map parses");
        game.init(W, H);
        let home = frog_position(&game, game.enemy_frog);
        let frog = frog_position(&game, game.frog);
        assert!(home.distance_to(frog) >= tuning().enemy_frog_spawn_min_dist, "{home:?} is too close to {frog:?}");
        let border = home.x.min(W - home.x).min(home.y).min(H - home.y);
        assert!(border <= W.min(H) * tuning().enemy_spawn_margin_max, "{home:?} is outside the spawn band");
        for t in game.tank_snapshots() {
            assert!(t.position.distance_to(home) > Tank::default().size(), "{:?} spawned on the enemy frog", t.position);
        }
    }

    fn hunt_game() -> Game {
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(7);
        game.player_row_override = Some(0);
        game.level_overrides.mission = Some(Mission::Hunt);
        game.map = MapFile::from_toml_str(CORNERS_MAP).expect("test map parses");
        game.init(W, H);
        game
    }

    fn enemy_slot(game: &Game) -> usize {
        game.world.query::<&Tank>().with::<&Ai>().iter().map(|t| t.owner_slot()).next().expect("one enemy")
    }

    /// Park `slot`'s tank beside `frog`, well inside its bite range. Called
    /// every frame, since the frog hops away from any tank that crowds it -
    /// its own side's included.
    fn park_beside(game: &mut Game, slot: usize, frog: Option<Entity>) {
        let (pos, range) = with_frog(&game.world, frog.expect("frog on the field"), |fr| (fr.position, fr.attack_range()));
        let spot = Position::new(pos.x + range * 0.5, pos.y);
        assert!(spot.distance_to(pos) < range, "the test parks the tank outside bite range");
        game.debug_teleport(slot, spot, Some(0.0)).expect("tank in slot");
    }

    fn bites_while_parked(game: &mut Game, slot: usize, frog: Option<Entity>, frames: u32) -> Vec<(Side, usize)> {
        let mut seen = Vec::new();
        for _ in 0..frames {
            park_beside(game, slot, frog);
            step(game, Input::default());
            seen.extend(game.events().iter().filter_map(|e| match e {
                Event::FrogBite { side, slot, .. } => Some((*side, *slot)),
                _ => None,
            }));
        }
        seen
    }

    /// A frog is its side's objective, not a hazard to it: neither frog
    /// ever bites a tank of its own side, however long one sits on it.
    #[test]
    fn a_frog_never_bites_its_own_side() {
        let mut game = hunt_game();
        let (player_frog, enemy_frog) = (game.frog, game.enemy_frog);
        let enemy = enemy_slot(&game);
        for frame in 0..300 {
            park_beside(&mut game, 0, player_frog);
            park_beside(&mut game, enemy, enemy_frog);
            step(&mut game, Input::default());
            let bite = game.events().iter().find(|e| matches!(e, Event::FrogBite { .. }));
            assert!(bite.is_none(), "frame {frame}: a frog bit its own side: {bite:?}");
        }
    }

    /// A 384 px corridor between two long walls four cells apart, the frog
    /// parked in the middle of it: open ground in every direction along
    /// the corridor, with terrain close by on two sides. A landing test
    /// measured from the tiles' centres rather than their hulls rejects
    /// the whole corridor and the frog never hops at all.
    const HOP_CORRIDOR_MAP: &str = r#"
version = 1
tanks = 0
cells."5,11" = { kind = "start" }
cells."20,11" = { kind = "frog" }
"#;

    fn hop_corridor_map() -> String {
        let mut text = String::from(HOP_CORRIDOR_MAP);
        for col in 14..=26 {
            for row in [9, 13] {
                text.push_str(&format!("cells.\"{col},{row}\" = {{ kind = \"wall\", material = \"brick\" }}\n"));
            }
        }
        text
    }

    /// Every wall/prop tile's centre and hull half-extents, for asserting
    /// the frog never lands inside one.
    fn tile_boxes(game: &Game) -> Vec<(Position, f32)> {
        game.world.query::<&Obstacle>().iter().map(|o| (o.position, o.hull_size() * 0.5)).collect()
    }

    /// Crowd the frog from the west and let it run: it hops clear rather
    /// than sitting there taking it, even with terrain a couple of cells
    /// away on both sides.
    #[test]
    fn a_crowded_frog_hops_clear_of_the_tank_in_built_up_terrain() {
        let mut game = game_on(&hop_corridor_map(), 0, Some(0));
        let frog = game.frog.expect("the map places a frog");
        let (start, avoid, hop) = with_frog(&game.world, frog, |fr| (fr.position, fr.avoid_range(), fr.hop_distance()));
        let boxes = tile_boxes(&game);
        let frog_half = (FROG_COLLIDER_HALF_EXTENT.0, FROG_COLLIDER_HALF_EXTENT.1);
        for frame in 0..180 {
            // Follow it west of wherever it has got to, just inside the
            // avoid ring and outside its bite range.
            let pos = with_frog(&game.world, frog, |fr| fr.position);
            game.debug_teleport(0, Position::new(pos.x - avoid * 0.9, pos.y), Some(0.0)).expect("player slot");
            step(&mut game, Input::default());
            let pos = with_frog(&game.world, frog, |fr| fr.position);
            let inside = boxes.iter().find(|(c, h)| {
                (pos.x - c.x).abs() < h + frog_half.0 && (pos.y - c.y).abs() < h + frog_half.1
            });
            assert!(inside.is_none(), "frame {frame}: the frog landed inside the tile at {inside:?}");
        }
        let moved = with_frog(&game.world, frog, |fr| fr.position).distance_to(start);
        assert!(moved > hop, "the crowded frog moved {moved:.0} px in 3 s - a single hop is {hop:.0} px");
    }

    /// The frog runs *from* the tank: crowded from one side for long
    /// enough, it ends up further away than the tank ever let it be, not
    /// hopping into it.
    #[test]
    fn a_frogs_hops_carry_it_away_from_what_crowds_it() {
        let mut game = game_on(&hop_corridor_map(), 0, Some(0));
        let frog = game.frog.expect("the map places a frog");
        let (start, avoid) = with_frog(&game.world, frog, |fr| (fr.position, fr.avoid_range()));
        // The tank sits still this time, west of the frog: every hop has
        // to take the frog further east than the last.
        let post = Position::new(start.x - avoid * 0.9, start.y);
        let mut nearest = f32::MAX;
        for _ in 0..300 {
            game.debug_teleport(0, post, Some(0.0)).expect("player slot");
            step(&mut game, Input::default());
            let pos = with_frog(&game.world, frog, |fr| fr.position);
            nearest = nearest.min(pos.distance_to(post));
        }
        let pos = with_frog(&game.world, frog, |fr| fr.position);
        assert!(pos.distance_to(post) > avoid, "the frog stayed inside the avoid ring: {pos:?} vs {post:?}");
        assert!(nearest >= avoid * 0.85, "a hop carried the frog to {nearest:.0} px of the tank, closer than it started");
    }

    /// The art is authored facing right and mirrored for the other way
    /// (`frog::Facing`), so a hop has to turn the frog or it moonwalks.
    #[test]
    fn a_frog_faces_the_way_it_hops() {
        for (from_east, want) in [(true, Facing::Left), (false, Facing::Right)] {
            let mut game = game_on(&hop_corridor_map(), 0, Some(0));
            let frog = game.frog.expect("the map places a frog");
            let avoid = with_frog(&game.world, frog, |fr| fr.avoid_range());
            // Start it facing the wrong way, so passing means a hop turned
            // it rather than that it was already right.
            let wrong = if want == Facing::Left { Facing::Right } else { Facing::Left };
            with_frog_mut(&game.world, frog, |fr| fr.facing = wrong);
            for _ in 0..300 {
                let pos = with_frog(&game.world, frog, |fr| fr.position);
                let side = if from_east { avoid * 0.9 } else { -avoid * 0.9 };
                game.debug_teleport(0, Position::new(pos.x + side, pos.y), Some(0.0)).expect("player slot");
                step(&mut game, Input::default());
            }
            let facing = with_frog(&game.world, frog, |fr| fr.facing);
            let side = if from_east { "east" } else { "west" };
            assert_eq!(facing, want, "crowded from the {side}, the frog ended up facing {facing:?}");
        }
    }

    /// The attack clip lashes its tongue to the side the frog faces, so a
    /// bite turns it toward the tank. Checked only on frames where the bite
    /// is what gets drawn: a hop outranks it in `Frog::anim`, and on a frame
    /// that does both the frog rightly faces the way it leapt.
    #[test]
    fn a_frog_faces_the_tank_it_bites() {
        let mut game = hunt_game();
        let enemy_frog = game.enemy_frog.expect("Hunt places an enemy frog");
        with_frog_mut(&game.world, enemy_frog, |fr| fr.facing = Facing::Right);
        let mut checked = false;
        for _ in 0..300 {
            let (pos, range) = with_frog(&game.world, enemy_frog, |fr| (fr.position, fr.attack_range()));
            // Player 1 parked to the frog's *west*, inside its bite range.
            game.debug_teleport(0, Position::new(pos.x - range * 0.5, pos.y), Some(0.0)).expect("player slot");
            step(&mut game, Input::default());
            let bit = game.events().iter().any(|e| matches!(e, Event::FrogBite { side: Side::Enemy, .. }));
            let (facing, hopping) = with_frog(&game.world, enemy_frog, |fr| (fr.facing, fr.hop_timer > 0.0));
            if bit && !hopping {
                assert_eq!(facing, Facing::Left, "the tongue lashed away from the tank it bit");
                checked = true;
                break;
            }
        }
        assert!(checked, "the enemy frog never bit the player parked beside it");
    }

    /// Walled in on every side, there is nowhere to land and the frog
    /// simply stays put - the search reports no spot rather than dropping
    /// it into a tile.
    #[test]
    fn a_boxed_in_frog_stays_put() {
        let mut text = String::from("version = 1\ntanks = 0\ncells.\"5,11\" = { kind = \"start\" }\ncells.\"20,11\" = { kind = \"frog\" }\n");
        for col in 18..=22 {
            for row in 9..=13 {
                if (col, row) != (20, 11) {
                    text.push_str(&format!("cells.\"{col},{row}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
                }
            }
        }
        let mut game = game_on(&text, 0, Some(0));
        let frog = game.frog.expect("the map places a frog");
        let (start, avoid) = with_frog(&game.world, frog, |fr| (fr.position, fr.avoid_range()));
        for _ in 0..120 {
            game.debug_teleport(0, Position::new(start.x - avoid * 0.9, start.y), Some(0.0)).expect("player slot");
            step(&mut game, Input::default());
        }
        let pos = with_frog(&game.world, frog, |fr| fr.position);
        assert_eq!((pos.x, pos.y), (start.x, start.y), "a boxed-in frog hopped into the walls around it");
    }

    /// The other side is still fair game, on both sides - the bite reflex
    /// is gated on who the tank belongs to, not switched off.
    #[test]
    fn a_frog_bites_the_other_side() {
        let mut game = hunt_game();
        let (player_frog, enemy_frog) = (game.frog, game.enemy_frog);
        let enemy = enemy_slot(&game);
        let bites = bites_while_parked(&mut game, enemy, player_frog, 120);
        assert!(
            bites.iter().any(|&(side, slot)| side == Side::Player && slot == enemy),
            "the player's frog never bit the enemy parked on it: {bites:?}"
        );
        let bites = bites_while_parked(&mut game, 0, enemy_frog, 120);
        assert!(
            bites.iter().any(|&(side, slot)| side == Side::Enemy && slot == 0),
            "the enemy frog never bit the player parked on it: {bites:?}"
        );
        assert!(player_snapshot(&game).damage > 0.0, "the enemy frog's bites did no damage");
    }

    /// The role roll is skipped entirely at a zero share, so a Destroy
    /// round and a Protect round with no hunters draw the same stream and
    /// only differ by the frog; every enemy of a Destroy round fights the
    /// player.
    #[test]
    fn destroy_rolls_no_roles_and_hunt_rolls_hunters_or_guards() {
        let mut game = Game::default();
        game.enemy_count_override = Some(6);
        game.seed_override = Some(3);
        game.level_overrides.mission = Some(Mission::Destroy);
        game.map = MapFile::from_toml_str(OPEN_MAP).expect("test map parses");
        game.init(W, H);
        assert!(game.world.query::<&Ai>().iter().all(|ai| ai.role == Role::Player));
        game.level_overrides.mission = Some(Mission::Hunt);
        game.init(W, H);
        assert!(game.world.query::<&Ai>().iter().all(|ai| ai.role != Role::Player));
    }

    /// The player starts inside a sealed Iron ring with an ammo crate one
    /// cell (32px = PICKUP_COLLECT_RADIUS) away, so it is collected on the
    /// first frame without moving and nothing outside can interfere.
    const SEALED_CRATE_MAP: &str = r#"
version = 1
tanks = 1
cells."8,8" = { kind = "wall", material = "iron" }
cells."9,8" = { kind = "wall", material = "iron" }
cells."10,8" = { kind = "wall", material = "iron" }
cells."11,8" = { kind = "wall", material = "iron" }
cells."12,8" = { kind = "wall", material = "iron" }
cells."13,8" = { kind = "wall", material = "iron" }
cells."8,9" = { kind = "wall", material = "iron" }
cells."13,9" = { kind = "wall", material = "iron" }
cells."8,10" = { kind = "wall", material = "iron" }
cells."10,10" = { kind = "start" }
cells."11,10" = { kind = "pickup", pickup = "ammo" }
cells."13,10" = { kind = "wall", material = "iron" }
cells."8,11" = { kind = "wall", material = "iron" }
cells."13,11" = { kind = "wall", material = "iron" }
cells."8,12" = { kind = "wall", material = "iron" }
cells."9,12" = { kind = "wall", material = "iron" }
cells."10,12" = { kind = "wall", material = "iron" }
cells."11,12" = { kind = "wall", material = "iron" }
cells."12,12" = { kind = "wall", material = "iron" }
cells."13,12" = { kind = "wall", material = "iron" }
cells."30,20" = { kind = "frog" }
"#;

    #[test]
    fn a_collected_pickup_respawns_only_after_the_delay() {
        let mut game = game_on(SEALED_CRATE_MAP, 1, Some(0));
        let full = tuning().max_shells;
        let grant = tuning().pickup_ammo_amount;
        assert_eq!(player_ammo(&game), full);
        step(&mut game, Input::default());
        assert_eq!(player_ammo(&game), full + grant, "one crate grants tuning().pickup_ammo_amount once");
        let delay_frames = (tuning().pickup_respawn_seconds * 60.0) as u32;
        for _ in 0..delay_frames - 30 {
            step(&mut game, Input::default());
        }
        assert_eq!(player_ammo(&game), full + grant, "no second grant before tuning().pickup_respawn_seconds");
        for _ in 0..90 {
            step(&mut game, Input::default());
        }
        assert_eq!(game.outcome(), Outcome::Playing);
        assert_eq!(player_ammo(&game), full + 2 * grant, "the crate respawned once the delay elapsed");
    }

    const OPEN_MAP: &str = r#"
version = 1
tanks = 1
cells."20,11" = { kind = "start" }
cells."30,20" = { kind = "frog" }
"#;

    /// Two rooms split by a full-height iron column at col 20, joined only
    /// by portal A (10,11) on the player's side and portal B (30,11) on
    /// the other (docs/teleporting.md).
    fn portal_rooms_map(portal_b: bool, tanks: usize) -> String {
        let mut s = format!(
            "version = 1\ntanks = {tanks}\ncells.\"5,11\" = {{ kind = \"start\" }}\ncells.\"5,8\" = {{ kind = \"frog\" }}\ncells.\"10,11\" = {{ kind = \"portal\" }}\n"
        );
        if portal_b {
            s.push_str("cells.\"30,11\" = { kind = \"portal\" }\n");
        }
        for row in 0..=22 {
            s.push_str(&format!("cells.\"20,{row}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
        }
        s
    }

    fn teleports_of(game: &Game, slot: usize) -> Vec<(Position, Position)> {
        game.events()
            .iter()
            .filter_map(|e| match e {
                Event::Teleported { slot: s, x, y, to_x, to_y } if *s == slot => {
                    Some((Position::new(*x, *y), Position::new(*to_x, *to_y)))
                }
                _ => None,
            })
            .collect()
    }

    /// `Game::portal_phase`: the player drives into portal A and comes
    /// out beside portal B - the arrival is an open cell near B but
    /// outside B's trigger radius, the heading is kept, the velocity is
    /// zeroed, the cooldown is armed, exactly one event says so, and the
    /// AI-less player hops nowhere else while the cooldown runs.
    #[test]
    fn driving_into_a_portal_places_the_tank_beside_another() {
        let map = portal_rooms_map(true, 0);
        let mut game = game_on(&map, 0, None);
        assert!(game.portals_active());
        let a = map::cell_to_world(10, 11);
        let b = map::cell_to_world(30, 11);
        assert_eq!(game.portals(), &[a, b]);
        let right = Input::single(Intent { move_dir: Some(Dir::Right), ..Intent::default() });
        let mut hop = None;
        for _ in 0..600 {
            step(&mut game, right);
            let events = teleports_of(&game, 0);
            if !events.is_empty() {
                assert_eq!(events.len(), 1, "one hop per frame");
                hop = Some(events[0]);
                break;
            }
        }
        let (from, to) = hop.expect("the player reached portal A and hopped");
        let t = tuning();
        assert!(from.distance_to(a) <= t.portal_trigger_radius, "entered through A");
        assert!(to.distance_to(b) > t.portal_trigger_radius, "arrives outside B's trigger radius");
        assert!(to.distance_to(b) <= (t.portal_arrival_max_cells as f32 + 1.0) * OBSTACLE_GRID_SIZE, "but beside B");
        assert!(to.x > map::cell_to_world(20, 0).x, "on the far side of the wall");
        let player = game.player().unwrap();
        {
            let tank = game.world.get::<&Tank>(player).unwrap();
            // The same frame's player phase drives it on from the arrival
            // cell (the hop zeroed its velocity first, so this is under a
            // pixel of fresh acceleration, not carried momentum).
            assert!(tank.position.distance_to(to) < 1.0, "placed at the arrival, got {:?} vs {to:?}", tank.position);
            assert_eq!(tank.rotation, 90.0, "heading kept");
            assert!((tank.portal_cooldown - t.portal_cooldown_seconds).abs() < 1e-3);
        }
        // Rolling on for the cooldown's length never re-triggers: the
        // arrival cell is outside B's radius and the timer is running.
        let frames = (t.portal_cooldown_seconds * 60.0) as usize;
        for _ in 0..frames {
            step(&mut game, right);
            assert!(teleports_of(&game, 0).is_empty(), "no hop while the cooldown runs");
        }
        // Parked on B itself, the cooldown stands still: however long the
        // tank sits there it stays put, and it hops again only after
        // leaving and coming back.
        game.debug_teleport(0, b, Some(90.0)).unwrap();
        game.debug_set_tank(0, &crate::simulation::debug::TankPatch { portal_cooldown: Some(0.5), ..Default::default() }).unwrap();
        for _ in 0..180 {
            step(&mut game, Input::default());
            assert!(teleports_of(&game, 0).is_empty(), "a tank standing on a portal never re-triggers");
        }
        assert!(game.world.get::<&Tank>(player).unwrap().portal_cooldown > 0.0, "the cooldown froze on the portal");
        game.debug_teleport(0, Position::new(b.x, b.y + 3.0 * OBSTACLE_GRID_SIZE), Some(0.0)).unwrap();
        for _ in 0..60 {
            step(&mut game, Input::default());
        }
        assert_eq!(game.world.get::<&Tank>(player).unwrap().portal_cooldown, 0.0, "off the portal the cooldown ran out");
        let up = Input::single(Intent { move_dir: Some(Dir::Up), ..Intent::default() });
        let mut hopped_back = false;
        for _ in 0..240 {
            step(&mut game, up);
            if !teleports_of(&game, 0).is_empty() {
                hopped_back = true;
                break;
            }
        }
        assert!(hopped_back, "coming back onto B after leaving hops again");
    }

    /// A lone portal is an inert marker: nothing teleports, the grid has
    /// no hub, and the round draws no RNG for it.
    #[test]
    fn a_single_portal_does_nothing() {
        let map = portal_rooms_map(false, 0);
        let mut game = game_on(&map, 0, None);
        assert!(!game.portals_active());
        assert_eq!(game.portals().len(), 1);
        assert!(game.nav_grid(W, H).portal_cells().is_empty());
        let right = Input::single(Intent { move_dir: Some(Dir::Right), ..Intent::default() });
        let mut reached = false;
        for _ in 0..600 {
            step(&mut game, right);
            assert!(teleports_of(&game, 0).is_empty());
            let pos = game.world.get::<&Tank>(game.player().unwrap()).unwrap().position;
            reached |= pos.distance_to(map::cell_to_world(10, 11)) <= tuning().portal_trigger_radius;
        }
        assert!(reached, "the player did drive over the lone portal");
    }

    /// The planner routes through the hub: with both portals the enemy's
    /// room connects to the player's, without them it does not - and the
    /// enemy actually makes the hop and ends up on the player's side.
    #[test]
    fn an_enemy_routes_through_the_portals_to_the_player() {
        let with = portal_rooms_map(true, 1);
        let without = portal_rooms_map(false, 1);
        let mut game = game_on(&with, 1, None);
        let slot = enemy_slot(&game);
        let far = map::cell_to_world(28, 11);
        game.debug_teleport(slot, far, Some(270.0)).expect("enemy placed in the far room");
        let player_pos = game.world.get::<&Tank>(game.player().unwrap()).unwrap().position;
        assert!(game.nav_path_cells(far, player_pos, W, H).is_some(), "a route through the hub exists");
        let plain = game_on(&without, 1, None);
        assert!(plain.nav_path_cells(far, player_pos, W, H).is_none(), "without a network the wall is final");

        let mut hopped = None;
        for _ in 0..1200 {
            step(&mut game, Input::default());
            if let Some(&(from, to)) = teleports_of(&game, slot).first() {
                hopped = Some((from, to));
                break;
            }
        }
        let (from, to) = hopped.expect("the enemy drove into portal B within 20 s");
        let wall_x = map::cell_to_world(20, 0).x;
        assert!(from.x > wall_x && to.x < wall_x, "it went from the far room to the player's: {from:?} -> {to:?}");
    }

    /// No room to arrive: portal B ringed by iron with only its own
    /// footprint open. The player drives onto A, nothing happens, and the
    /// round's RNG stream is untouched - a portal-heavy map with a jammed
    /// exit replays exactly like one where nobody tried.
    #[test]
    fn a_portal_with_no_free_arrival_cell_does_not_fire_and_draws_no_rng() {
        let mut map = portal_rooms_map(true, 0);
        for (c, r) in [(28, 9), (29, 9), (30, 9), (31, 9), (32, 9), (28, 10), (32, 10), (28, 11), (32, 11), (28, 12), (32, 12), (28, 13), (29, 13), (30, 13), (31, 13), (32, 13)] {
            map.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
        }
        let mut game = game_on(&map, 0, None);
        assert!(game.portals_active());
        let right = Input::single(Intent { move_dir: Some(Dir::Right), ..Intent::default() });
        let a = map::cell_to_world(10, 11);
        let mut on_portal = false;
        for _ in 0..600 {
            let rng_before = game.rng.clone().expect("rng parked between frames");
            step(&mut game, right);
            assert!(teleports_of(&game, 0).is_empty(), "nowhere to arrive: no hop");
            let pos = game.world.get::<&Tank>(game.player().unwrap()).unwrap().position;
            if pos.distance_to(a) <= tuning().portal_trigger_radius {
                on_portal = true;
                // A frame on the portal with no candidates must draw nothing
                // beyond what the same frame draws elsewhere: with no
                // enemies and no props that is nothing at all.
                let mut before = rng_before;
                let mut after = game.rng.clone().unwrap();
                assert_eq!(before.random::<u64>(), after.random::<u64>(), "RNG advanced on a refused teleport");
            }
        }
        assert!(on_portal, "the player did stand on portal A");
    }

    /// Enemies only take what they would actually use. A pack that hoovers
    /// up every crate it drives past strips the field of what the player
    /// needed and gains nothing itself - see `Tank::wants_pickup`.
    #[test]
    fn an_enemy_leaves_behind_what_it_does_not_need() {
        const MAP: &str = r#"
version = 1
tanks = 1
cells."10,10" = { kind = "start" }
cells."11,10" = { kind = "pickup", pickup = "health" }
cells."30,20" = { kind = "frog" }
"#;
        let mut game = game_on(MAP, 1, Some(0));
        // Park the player far away so only the enemy can reach the pack.
        game.debug_teleport(0, map::cell_to_world(30, 5), None).expect("the player exists");
        let enemy = game.world.query::<&Tank>().with::<&Ai>().iter().map(|t| t.owner_slot()).next().unwrap();
        // Undamaged: it has no use for a health pack.
        let entity = game.tank_entity_by_slot(enemy).expect("the enemy exists");
        with_tank_mut(&game.world, entity, |t| t.damage = 0.0);
        game.debug_teleport(enemy, map::cell_to_world(11, 10), None).expect("the enemy exists");
        step(&mut game, Input::default());
        assert_eq!(
            health_on_field(&game),
            1,
            "a healthy enemy must drive over a health pack and leave it for the player"
        );

        // Hurt: now it wants it, and the same drive-over collects.
        with_tank_mut(&game.world, entity, |t| t.damage = 40.0);
        game.debug_teleport(enemy, map::cell_to_world(11, 10), None).expect("the enemy exists");
        step(&mut game, Input::default());
        assert_eq!(health_on_field(&game), 0, "a hurt enemy still takes it");
    }

    /// The player is deliberately exempt: taking something you do not
    /// strictly need is a decision the person at the controls gets to make.
    #[test]
    fn a_player_still_collects_what_it_does_not_need() {
        const MAP: &str = r#"
version = 1
tanks = 0
cells."10,10" = { kind = "start" }
cells."11,10" = { kind = "pickup", pickup = "health" }
cells."30,20" = { kind = "frog" }
"#;
        let mut game = game_on(MAP, 0, Some(0));
        let player = game.player().expect("a player exists");
        with_tank_mut(&game.world, player, |t| t.damage = 0.0);
        step(&mut game, Input::default());
        assert_eq!(health_on_field(&game), 0, "an undamaged player still collects");
    }

    // --- The frog health pack (docs/frog-health-pack-prd.md) ---

    /// The player starts one cell from a frog pack, so the collect test is
    /// one `step`. The frog is far enough away that nothing else reaches
    /// it. `tanks = 1` so there is an enemy to teleport onto the pack.
    const FROG_PACK_MAP: &str = r#"
version = 1
tanks = 1
cells."10,10" = { kind = "start" }
cells."11,10" = { kind = "pickup", pickup = "frog_health" }
cells."30,20" = { kind = "frog" }
"#;

    /// Same, as a Hunt round: the enemy has a frog of its own to heal.
    const FROG_PACK_HUNT_MAP: &str = r#"
version = 1
tanks = 1
mission.kind = "hunt"
cells."10,10" = { kind = "start" }
cells."11,10" = { kind = "pickup", pickup = "frog_health" }
cells."30,20" = { kind = "frog" }
cells."4,4" = { kind = "enemy_frog" }
"#;

    fn hurt_frog(game: &Game, entity: Entity, amount: f32) {
        with_frog_mut(&game.world, entity, |f| f.damage(amount));
    }

    fn frog_health(game: &Game, entity: Entity) -> f32 {
        with_frog(&game.world, entity, |f| f.health)
    }

    /// Health packs specifically. A Health slot also rolls a bonus shield
    /// (`maybe_spawn_health_slot_bonuses`), so counting every live pickup
    /// would count that too.
    fn health_on_field(game: &Game) -> usize {
        game.world.query::<&Pickup>().iter().filter(|p| p.kind == PickupKind::Health).count()
    }

    fn packs_on_field(game: &Game) -> usize {
        game.world.query::<&Pickup>().iter().filter(|p| p.kind == PickupKind::FrogHealth).count()
    }

    /// Drive `slot`'s tank onto the pack's cell and advance one frame.
    fn collect_with(game: &mut Game, slot: usize) {
        game.debug_teleport(slot, map::cell_to_world(11, 10), None).expect("slot exists");
        step(game, Input::default());
    }

    #[test]
    fn a_frog_pack_restores_the_players_hurt_frog_to_full() {
        let mut game = game_on(FROG_PACK_MAP, 1, Some(0));
        let frog = game.frog.expect("a Protect round has a frog");
        // Init already collected nothing: the frog is pristine, so the
        // pack must still be there.
        assert_eq!(packs_on_field(&game), 1);
        hurt_frog(&game, frog, 25.0);
        let hurt = frog_health(&game, frog);
        assert!(hurt < tuning().frog_max_health);
        step(&mut game, Input::default());
        assert_eq!(frog_health(&game, frog), tuning().frog_max_health, "one pack is a full heal");
        assert_eq!(packs_on_field(&game), 0, "the pack was consumed");
        assert!(
            game.events()
                .iter()
                .any(|e| matches!(e, Event::PickupCollected { slot: 0, kind: PickupKind::FrogHealth })),
            "{:?}",
            game.events()
        );
        let healed = game
            .events()
            .iter()
            .find_map(|e| match e {
                Event::FrogHealed { side, amount, .. } => Some((*side, *amount)),
                _ => None,
            })
            .expect("a heal reports itself");
        assert_eq!(healed.0, Side::Player);
        assert!((healed.1 - (tuning().frog_max_health - hurt)).abs() < 0.01, "{healed:?}");
    }

    #[test]
    fn a_frog_pack_is_left_on_the_field_while_the_frog_is_at_full_health() {
        let mut game = game_on(FROG_PACK_MAP, 1, Some(0));
        for _ in 0..30 {
            step(&mut game, Input::default());
        }
        assert_eq!(packs_on_field(&game), 1, "a pristine frog does not spend the pack");
        assert!(!game.events().iter().any(|e| matches!(e, Event::PickupCollected { .. })));
    }

    /// The pack is the frog's, not the collector's: a tank at full health
    /// still picks one up for a hurt frog, and takes nothing from it.
    #[test]
    fn a_frog_pack_heals_the_frog_and_never_the_tank_that_took_it() {
        let mut game = game_on(FROG_PACK_MAP, 1, Some(0));
        let frog = game.frog.expect("a Protect round has a frog");
        hurt_frog(&game, frog, 25.0);
        let player = game.player().expect("a player exists");
        with_tank_mut(&game.world, player, |t| t.damage = 30.0);
        step(&mut game, Input::default());
        assert_eq!(frog_health(&game, frog), tuning().frog_max_health);
        assert_eq!(with_tank(&game.world, player, |t| t.damage), 30.0, "the tank's own health is untouched");
    }

    /// Protect gives the enemies no frog, so an enemy that drives over a
    /// pack consumes it for nothing - the denial pressure that makes the
    /// pack worth racing for.
    #[test]
    fn an_enemy_with_no_frog_of_its_own_wastes_the_pack() {
        let mut game = game_on(FROG_PACK_MAP, 1, Some(0));
        let frog = game.frog.expect("a Protect round has a frog");
        hurt_frog(&game, frog, 25.0);
        let hurt = frog_health(&game, frog);
        // Park the player far away so only the enemy can reach the pack.
        game.debug_teleport(0, map::cell_to_world(30, 5), None).expect("the player exists");
        let enemy = game.world.query::<&Tank>().with::<&Ai>().iter().map(|t| t.owner_slot()).next().unwrap();
        collect_with(&mut game, enemy);
        assert_eq!(packs_on_field(&game), 0, "the enemy took it");
        assert_eq!(frog_health(&game, frog), hurt, "and healed nothing");
        assert!(!game.events().iter().any(|e| matches!(e, Event::FrogHealed { .. })));
    }

    /// Hunt gives both sides a frog, and each heals only its own.
    #[test]
    fn an_enemy_in_a_hunt_round_heals_the_enemy_frog_and_not_the_players() {
        let mut game = game_on(FROG_PACK_HUNT_MAP, 1, Some(0));
        let frog = game.frog.expect("Hunt has a player frog");
        let enemy_frog = game.enemy_frog.expect("Hunt has an enemy frog");
        hurt_frog(&game, frog, 25.0);
        hurt_frog(&game, enemy_frog, 25.0);
        let player_hurt = frog_health(&game, frog);
        game.debug_teleport(0, map::cell_to_world(30, 5), None).expect("the player exists");
        let enemy = game.world.query::<&Tank>().with::<&Ai>().iter().map(|t| t.owner_slot()).next().unwrap();
        collect_with(&mut game, enemy);
        assert_eq!(frog_health(&game, enemy_frog), tuning().frog_max_health, "the enemy healed its own frog");
        assert_eq!(frog_health(&game, frog), player_hurt, "and left the player's alone");
        assert!(
            game.events().iter().any(|e| matches!(e, Event::FrogHealed { side: Side::Enemy, .. })),
            "{:?}",
            game.events()
        );
    }

    #[test]
    fn a_frog_pack_does_not_revive_a_dead_frog() {
        let mut game = game_on(FROG_PACK_MAP, 1, Some(0));
        let frog = game.frog.expect("a Protect round has a frog");
        hurt_frog(&game, frog, tuning().frog_max_health);
        assert!(with_frog(&game.world, frog, Frog::is_dead));
        step(&mut game, Input::default());
        assert_eq!(frog_health(&game, frog), 0.0, "a dead frog stays dead");
        assert!(with_frog(&game.world, frog, Frog::is_dead));
        // A dead frog is not "full": the pack is taken like any other side
        // with nothing to heal, and wasted.
        assert_eq!(packs_on_field(&game), 0);
        assert!(!game.events().iter().any(|e| matches!(e, Event::FrogHealed { .. })));
    }

    /// The bonus drop: a hurt frog can put a pack beside a respawning
    /// health slot, a pristine one never does. The player is parked next
    /// to the health slot so it is collected and respawns over and over,
    /// giving the roll many chances.
    #[test]
    fn the_bonus_pack_rolls_only_while_the_frog_is_hurt() {
        const MAP: &str = r#"
version = 1
tanks = 0
cells."10,10" = { kind = "start" }
cells."11,10" = { kind = "pickup", pickup = "health" }
cells."30,20" = { kind = "frog" }
"#;
        let frames = (tuning().pickup_respawn_seconds * 60.0) as u32 * 8 + 120;

        let mut hurt = game_on(MAP, 0, Some(0));
        let frog = hurt.frog.expect("a Protect round has a frog");
        let mut saw_pack = false;
        for _ in 0..frames {
            // Held hurt on purpose: a pack the player picks up would
            // otherwise close the gate after the very first drop.
            with_frog_mut(&hurt.world, frog, |f| f.health = f.max_health * 0.5);
            step(&mut hurt, Input::default());
            saw_pack |= packs_on_field(&hurt) > 0;
        }
        assert!(saw_pack, "a hurt frog eventually draws a bonus pack");

        let mut pristine = game_on(MAP, 0, Some(0));
        for _ in 0..frames {
            step(&mut pristine, Input::default());
            assert_eq!(packs_on_field(&pristine), 0, "a pristine frog never gets one");
        }
    }

    /// The gate sits *before* the roll, which is what keeps every round
    /// whose frog is never hurt byte-identical to the same seed before
    /// this pickup existed (docs/frog-health-pack-prd.md section 6). Tested
    /// as it is stated: a pristine frog's health slot must leave the RNG
    /// exactly where the shield's own roll alone leaves it.
    #[test]
    fn a_pristine_frogs_health_slot_draws_no_more_rng_than_the_shield_roll_alone() {
        let map = MapFile::from_toml_str("version = 1\ncells.\"10,10\" = { kind = \"start\" }\n").expect("parses");
        let slot = map::cell_to_world(11, 10);
        let (mut with_gate, mut shield_only) = (SmallRng::seed_from_u64(99), SmallRng::seed_from_u64(99));
        let (mut wa, mut wb) = (hecs::World::new(), hecs::World::new());
        maybe_spawn_health_slot_bonuses(&mut wa, &map, slot, W, H, false, &mut with_gate);
        let chance = tuning().shield_near_health_chance;
        maybe_spawn_bonus(&mut wb, &map, slot, PickupKind::Shield, chance, W, H, &mut shield_only);
        assert_eq!(
            with_gate.random::<u64>(),
            shield_only.random::<u64>(),
            "a pristine frog must add no draw of its own"
        );
    }

    fn fire_once(row: i32) -> i32 {
        let mut game = game_on(OPEN_MAP, 1, Some(row));
        let fire = Input::single(Intent { fire: true, ..Intent::default() });
        step(&mut game, fire);
        for _ in 0..10 {
            step(&mut game, Input::default());
        }
        player_ammo(&game)
    }

    /// `Game::events` reports a collected pickup and a trigger pull on the
    /// frame they happen, and nothing on a frame where nothing happened.
    #[test]
    fn events_record_pickups_and_shots_on_their_frame() {
        let mut game = game_on(SEALED_CRATE_MAP, 1, Some(0));
        assert!(matches!(game.events(), [Event::RoundStarted { enemies: 1, .. }]));
        step(&mut game, Input::default());
        let collected = game.events().iter().any(|e| matches!(e, Event::PickupCollected { slot: 0, kind: PickupKind::Ammo }));
        assert!(collected, "{:?}", game.events());
        step(&mut game, Input::default());
        assert!(game.events().is_empty(), "{:?}", game.events());

        let mut game = game_on(OPEN_MAP, 1, Some(0));
        let fire = Input::single(Intent { fire: true, ..Intent::default() });
        step(&mut game, fire);
        assert!(
            game.events().iter().any(|e| matches!(e, Event::Fired { slot: 0, weapon: "shell" })),
            "{:?}",
            game.events()
        );
    }

    /// An enemy shut inside a brick box with the player out of its line of
    /// fire has no route anywhere; it shoots the brick down and leaves
    /// (see `ai::Brain::wants_breach`).
    const BRICK_BOX_MAP: &str = r#"
version = 1
tanks = 1
cells."8,8" = { kind = "wall", material = "brick" }
cells."9,8" = { kind = "wall", material = "brick" }
cells."10,8" = { kind = "wall", material = "brick" }
cells."11,8" = { kind = "wall", material = "brick" }
cells."12,8" = { kind = "wall", material = "brick" }
cells."8,9" = { kind = "wall", material = "brick" }
cells."12,9" = { kind = "wall", material = "brick" }
cells."8,10" = { kind = "wall", material = "brick" }
cells."12,10" = { kind = "wall", material = "brick" }
cells."8,11" = { kind = "wall", material = "brick" }
cells."12,11" = { kind = "wall", material = "brick" }
cells."8,12" = { kind = "wall", material = "brick" }
cells."9,12" = { kind = "wall", material = "brick" }
cells."10,12" = { kind = "wall", material = "brick" }
cells."11,12" = { kind = "wall", material = "brick" }
cells."12,12" = { kind = "wall", material = "brick" }
cells."24,14" = { kind = "start" }
cells."30,20" = { kind = "frog" }
"#;

    #[test]
    fn a_walled_in_enemy_shoots_its_way_out() {
        let mut game = game_on(BRICK_BOX_MAP, 1, Some(0));
        let center = map::cell_to_world(10, 10);
        game.debug_teleport(1, center, Some(180.0)).expect("enemy in slot 1");
        let (mut fired, mut broke) = (0, false);
        for _ in 0..900 {
            step(&mut game, Input::default());
            for e in game.events() {
                match e {
                    Event::Fired { slot: 1, .. } => fired += 1,
                    Event::Hit { target: HitTarget::Obstacle { .. }, killed: true, .. } => broke = true,
                    _ => {}
                }
            }
            if broke {
                break;
            }
        }
        assert!(fired >= 1 && broke, "fired {fired} shots, broke a tile: {broke}");
        for _ in 0..600 {
            step(&mut game, Input::default());
        }
        let enemy = game.tank_snapshots().into_iter().find(|t| !t.is_player).expect("enemy");
        let out = (enemy.position.x - center.x).abs() > OBSTACLE_GRID_SIZE * 1.5
            || (enemy.position.y - center.y).abs() > OBSTACLE_GRID_SIZE * 1.5;
        assert!(out, "enemy still inside the box at ({:.0},{:.0})", enemy.position.x, enemy.position.y);
    }

    /// Three iron tiles across the player's line of fire, two cells up.
    const IRON_BAR_MAP: &str = r#"
version = 1
tanks = 0
cells."20,11" = { kind = "start" }
cells."19,8" = { kind = "wall", material = "iron" }
cells."20,8" = { kind = "wall", material = "iron" }
cells."21,8" = { kind = "wall", material = "iron" }
"#;

    /// A shell that bounces off iron says so once - `Event::Ricochet` with
    /// the heading it flies on along - and keeps flying until it lands
    /// somewhere else.
    #[test]
    fn a_shell_off_iron_ricochets_once_and_flies_on() {
        let mut game = game_on(IRON_BAR_MAP, 0, Some(0));
        let player = game.player().expect("player");
        with_tank_mut(&game.world, player, |t| t.control(None, Some(Dir::Up)));
        step(&mut game, Input::single(Intent { fire: true, ..Intent::default() }));
        let (mut ricochets, mut headings, mut landed_after) = (0, Vec::new(), false);
        for _ in 0..240 {
            step(&mut game, Input::default());
            for e in game.events() {
                match *e {
                    Event::Ricochet { slot: 0, heading, .. } => {
                        ricochets += 1;
                        headings.push(heading);
                        assert!(game.world.query::<&Shell>().iter().any(|s| s.state == ShellState::Flying), "still flying");
                    }
                    Event::Hit { target: HitTarget::Wall, .. } if ricochets > 0 => landed_after = true,
                    _ => {}
                }
            }
        }
        assert_eq!(ricochets, 1, "one bounce per shell (`shell_ricochet_bounces`)");
        assert!((headings[0] - 180.0).abs() < 1.0, "reflected straight back down, got {}", headings[0]);
        assert!(landed_after, "the shell flew on to the boundary wall");
    }

    #[test]
    fn a_twin_barrel_shot_costs_two_shells_and_a_single_costs_one() {
        let full = tuning().max_shells;
        assert_eq!(fire_once(1), full - 2, "row 1 (assault) is twin-barrel");
        assert_eq!(fire_once(0), full - 1, "row 0 (scout) is single-barrel");
    }

    #[test]
    fn a_held_fire_key_fires_shells_only_once() {
        let mut game = game_on(OPEN_MAP, 1, Some(0));
        let fire = Input::single(Intent { fire: true, ..Intent::default() });
        for _ in 0..30 {
            step(&mut game, fire);
        }
        assert_eq!(player_ammo(&game), tuning().max_shells - 1);
    }

    /// `SEALED_CRATE_MAP` with the crate swapped for a rainbow shield.
    const SEALED_SHIELD_MAP: &str = r#"
version = 1
tanks = 1
cells."8,8" = { kind = "wall", material = "iron" }
cells."9,8" = { kind = "wall", material = "iron" }
cells."10,8" = { kind = "wall", material = "iron" }
cells."11,8" = { kind = "wall", material = "iron" }
cells."12,8" = { kind = "wall", material = "iron" }
cells."13,8" = { kind = "wall", material = "iron" }
cells."8,9" = { kind = "wall", material = "iron" }
cells."13,9" = { kind = "wall", material = "iron" }
cells."8,10" = { kind = "wall", material = "iron" }
cells."10,10" = { kind = "start" }
cells."11,10" = { kind = "pickup", pickup = "shield" }
cells."13,10" = { kind = "wall", material = "iron" }
cells."8,11" = { kind = "wall", material = "iron" }
cells."13,11" = { kind = "wall", material = "iron" }
cells."8,12" = { kind = "wall", material = "iron" }
cells."9,12" = { kind = "wall", material = "iron" }
cells."10,12" = { kind = "wall", material = "iron" }
cells."11,12" = { kind = "wall", material = "iron" }
cells."12,12" = { kind = "wall", material = "iron" }
cells."13,12" = { kind = "wall", material = "iron" }
cells."30,20" = { kind = "frog" }
"#;

    fn player_snapshot(game: &Game) -> TankSnapshot {
        game.tank_snapshots().into_iter().find(|t| t.is_player).expect("player")
    }

    #[test]
    fn a_shield_pickup_heals_to_full_and_absorbs_until_it_is_spent() {
        let mut game = game_on(SEALED_SHIELD_MAP, 1, Some(0));
        let player = game.player().expect("player");
        {
            let mut q = game.world.query_one::<&mut Tank>(player);
            let tank = q.get().expect("player tank");
            tank.damage = 50.0;
            tank.shield_hp = 0.0;
        }
        step(&mut game, Input::default());
        let snap = player_snapshot(&game);
        assert_eq!(snap.damage, 0.0, "the shield pickup is a full heal");
        let capacity = tuning().shield_capacity;
        assert_eq!(snap.shield_hp, capacity, "pool filled to tuning().shield_capacity");
        // Damage through the absorb seam comes off the shield, not the hull,
        // and reports that nothing landed. The slot is cleared first so the
        // crate can't respawn into the sealed ring and refill mid-test.
        game.map_pickup_slots.clear();
        {
            let mut q = game.world.query_one::<&mut Tank>(player);
            let tank = q.get().expect("player tank");
            assert_eq!(tank.take_damage(30.0, MAX_DAMAGE), 0.0, "an absorbed hit lands nothing");
            assert_eq!(tank.damage, 0.0);
            assert_eq!(tank.shield_hp, capacity - 30.0, "the shield paid for it");
        }
        // Spending the rest shatters it, and the hull is exposed again.
        {
            let mut q = game.world.query_one::<&mut Tank>(player);
            let tank = q.get().expect("player tank");
            assert!(tank.spend_shield(capacity), "the blow that empties the pool shatters it");
            assert!(!tank.is_shielded());
            assert_eq!(tank.take_damage(30.0, MAX_DAMAGE), 30.0);
            assert_eq!(tank.damage, 30.0, "unshielded again once the pool is spent");
        }
    }

    /// The pool is what ends a shield, and an oversized hit is absorbed in
    /// full rather than bleeding through to the hull.
    #[test]
    fn an_oversized_hit_is_absorbed_whole_and_then_shatters_the_shield() {
        let mut tank = Tank { shield_hp: 10.0, ..Tank::default() };
        assert_eq!(tank.take_damage(500.0, MAX_DAMAGE), 0.0, "no bleed-through");
        assert_eq!(tank.damage, 0.0);
        assert!(!tank.is_shielded(), "and the shield is gone");
        assert_eq!(tank.shield_hp, 0.0, "never negative");
    }

    /// A damaged shield refills once it is left alone; a shattered one does
    /// not come back, which is what keeps recharge from making shields
    /// permanent. Also pins that a tank hit this frame cannot regen this
    /// frame - `tick_timers` runs before every hit phase.
    #[test]
    fn a_live_shield_recharges_out_of_contact_but_a_broken_one_stays_broken() {
        let capacity = tuning().shield_capacity;
        let delay = tuning().shield_recharge_delay_seconds;
        let dt = 1.0 / 60.0;

        let mut hurt = Tank { shield_hp: capacity * 0.5, ..Tank::default() };
        hurt.spend_shield(1.0);
        let after_hit = hurt.shield_hp;
        hurt.tick_shield(dt);
        assert_eq!(hurt.shield_hp, after_hit, "no regen on the frame it was struck");
        for _ in 0..((delay * 60.0) as u32 + 1) {
            hurt.tick_shield(dt);
        }
        hurt.tick_shield(dt);
        assert!(hurt.shield_hp > after_hit, "it refills once the delay lapses");

        let mut broken = Tank { shield_hp: 0.0, ..Tank::default() };
        for _ in 0..600 {
            broken.tick_shield(dt);
        }
        assert_eq!(broken.shield_hp, 0.0, "a shattered shield never returns on its own");
    }

    /// Drop an enemy-owned shell `above` px above the player, flying
    /// straight down at it, and run `frames` frames. Returns whether a
    /// shell ever ended up owned by the player (i.e. was deflected) and the
    /// player's damage at the end.
    fn shell_from_above(shielded: bool, above: f32, frames: u32) -> (bool, f32) {
        let mut game = game_on(OPEN_MAP, 0, Some(0));
        let player = game.player().expect("player");
        let (pos, row) = {
            let mut q = game.world.query_one::<&mut Tank>(player);
            let tank = q.get().expect("player tank");
            tank.shield_hp = if shielded { tuning().shield_capacity } else { 0.0 };
            (tank.position, tank.row)
        };
        let shooter = Tank { row, position: Position::new(pos.x, pos.y - above), rotation: 180.0, ..Tank::default() };
        let shell = Shell::spawn(&shooter, Owner::Enemy(1), 0.0, 0.0);
        game.world.spawn((shell,));
        let mut deflected = false;
        for _ in 0..frames {
            step(&mut game, Input::default());
            deflected |= game.world.query::<&Shell>().iter().any(|s| s.owner == Owner::Player(0) && s.velocity.y < 0.0);
        }
        (deflected, player_snapshot(&game).damage)
    }

    #[test]
    fn a_shielded_tank_bounces_a_shell_back_at_its_shooter() {
        let (deflected, damage) = shell_from_above(true, 160.0, 240);
        assert!(deflected, "the shell should come back player-owned and travelling up");
        assert_eq!(damage, 0.0, "and the player takes nothing");
        // Same shot without the shield lands, so the setup really does aim
        // at the player.
        let (deflected, damage) = shell_from_above(false, 160.0, 240);
        assert!(!deflected);
        assert!(damage > 0.0, "unshielded control: the shell hits");
    }

    /// Deflecting a shot costs the shield more than absorbing the same
    /// damage would - `shield_deflect_cost_factor`. This is what makes
    /// shooting a shielded tank the fastest way to strip it, and the whole
    /// reason the deflect is a decision rather than a pure punish.
    #[test]
    fn deflecting_a_shell_costs_the_shield_more_than_absorbing_it_would() {
        let mut game = game_on(OPEN_MAP, 0, Some(0));
        let player = game.player().expect("player");
        let capacity = tuning().shield_capacity;
        let (pos, row) = {
            let mut q = game.world.query_one::<&mut Tank>(player);
            let tank = q.get().expect("player tank");
            tank.shield_hp = capacity;
            (tank.position, tank.row)
        };
        let shooter = Tank { row, position: Position::new(pos.x, pos.y - 160.0), rotation: 180.0, ..Tank::default() };
        game.world.spawn((Shell::spawn(&shooter, Owner::Enemy(1), 0.0, 0.0),));
        for _ in 0..240 {
            step(&mut game, Input::default());
        }
        let spent = capacity - player_snapshot(&game).shield_hp;
        assert!(spent > 0.0, "the deflected shell came off the shield");

        // The same shell's own worst-case roll, absorbed instead.
        let worst_absorb = tuning().enemy_damage_max * tuning().tank_damage_factor[row as usize];
        assert!(
            spent > worst_absorb,
            "a deflect ({spent}) must cost more than absorbing even the top of the shell's roll ({worst_absorb})",
        );
    }

    /// The headline rule: a shield is a pool, so enough fire ends it. Fired
    /// through the real projectile path rather than by calling
    /// `spend_shield`, so the deflect seam is what is under test.
    #[test]
    fn sustained_fire_breaks_a_shield_and_says_so() {
        let mut game = game_on(OPEN_MAP, 0, Some(0));
        let player = game.player().expect("player");
        let (pos, row) = {
            let mut q = game.world.query_one::<&mut Tank>(player);
            let tank = q.get().expect("player tank");
            tank.shield_hp = tuning().shield_capacity;
            // Out of the way of the recharge, which would otherwise refill
            // between volleys and make this a test of nothing.
            tank.shield_recharge_delay = 1.0e9;
            (tank.position, tank.row)
        };
        let shooter = Tank { row, position: Position::new(pos.x, pos.y - 160.0), rotation: 180.0, ..Tank::default() };
        let mut broke = false;
        for _ in 0..40 {
            game.world.spawn((Shell::spawn(&shooter, Owner::Enemy(1), 0.0, 0.0),));
            for _ in 0..30 {
                step(&mut game, Input::default());
                broke |= game.events().iter().any(|e| matches!(e, Event::ShieldBroken { .. }));
            }
            if broke {
                break;
            }
        }
        assert!(broke, "enough shells must shatter a shield and emit ShieldBroken");
        let snap = player_snapshot(&game);
        assert_eq!(snap.shield_hp, 0.0, "and the pool is empty afterwards");
    }

    #[test]
    fn a_bonus_shield_lands_on_a_free_neighbouring_cell() {
        let map = MapFile::from_toml_str(OPEN_MAP).expect("map parses");
        let slot = map::cell_to_world(20, 11);
        let mut rng = SmallRng::seed_from_u64(3);
        // The slot itself is occupied by its health pack; every neighbour is
        // free and inside the field.
        let pos = bonus_pickup_cell(&map, slot, &[slot], W, H, &mut rng).expect("an open map has a free neighbour");
        let (c, r) = map::world_to_cell(pos);
        assert!((c - 20).abs() <= 1 && (r - 11).abs() <= 1 && (c, r) != (20, 11), "adjacent, not the slot: {c},{r}");
        assert!(matches!(map.cell(c, r), None | Some(CellObject::Road | CellObject::Water)));
    }

    #[test]
    fn a_walled_in_health_slot_gets_no_bonus_shield() {
        // Cell 11,10 inside the sealed ring: its eight neighbours are the
        // start cell (10,10), walls, or cells already holding a pickup.
        let map = MapFile::from_toml_str(SEALED_CRATE_MAP).expect("map parses");
        let slot = map::cell_to_world(11, 10);
        let mut rng = SmallRng::seed_from_u64(3);
        let open_neighbours: Vec<Position> = [(12, 9), (12, 10), (12, 11), (9, 9), (9, 10), (9, 11), (10, 9), (10, 11), (11, 9), (11, 11)]
            .iter()
            .map(|&(c, r)| map::cell_to_world(c, r))
            .collect();
        let mut occupied = open_neighbours.clone();
        occupied.push(slot);
        assert_eq!(bonus_pickup_cell(&map, slot, &occupied, W, H, &mut rng), None);
    }

    // --- Waves spawn plan (docs/maps-to-levels.md, `waves.rs`) ---

    use crate::level::{SpawnKind, Tier};
    use crate::TANK_TIER_BY_ROW;

    /// A Destroy round on the open map under a waves plan, with the player
    /// shielded for the whole round so the enemies can never end it and the
    /// scheduler's own timing is all that decides what happens.
    fn waves_game(waves: u32, size: u32) -> Game {
        waves_game_seats(waves, size, 1)
    }

    /// `waves_game` for a team: `seats` seats, every one of them shielded
    /// so only the scheduler ends the round.
    fn waves_game_seats(waves: u32, size: u32, seats: usize) -> Game {
        let mut game = Game::default();
        game.seed_override = Some(7);
        game.player_row_override = Some(0);
        game.players = PlayerCount::from_count(seats).expect("a seat count the round takes");
        game.level_overrides.mission = Some(Mission::Destroy);
        game.level_overrides.spawn = Some(SpawnKind::Waves);
        game.level_overrides.waves = Some(waves);
        game.level_overrides.wave_size = Some(size);
        game.level_overrides.wave_growth = Some(0);
        game.map = MapFile::from_toml_str(OPEN_MAP).expect("test map parses");
        game.init(W, H);
        for seat in game.players().into_iter().flatten() {
            with_tank_mut(&game.world, seat, |t| t.shield_hp = 1.0e9);
        }
        game
    }

    fn wave_started(game: &Game) -> Option<u32> {
        game.events().iter().find_map(|e| match e {
            Event::WaveStarted { wave, .. } => Some(*wave),
            _ => None,
        })
    }

    fn tank_entered(game: &Game) -> Vec<usize> {
        game.events()
            .iter()
            .filter_map(|e| match e {
                Event::TankEntered { slot } => Some(*slot),
                _ => None,
            })
            .collect()
    }

    /// Step until a tank enters (at most `limit` frames); the slot.
    fn step_until_entered(game: &mut Game, limit: u32) -> usize {
        for _ in 0..limit {
            step(game, Input::default());
            if let Some(&slot) = tank_entered(game).first() {
                return slot;
            }
        }
        panic!("no tank entered within {limit} frames");
    }

    #[test]
    fn a_waves_round_places_nobody_at_init_and_calls_wave_one_on_the_first_frame() {
        let mut game = waves_game(2, 1);
        assert_eq!(game.tank_snapshots().len(), 1, "only the player at init");
        assert!(matches!(game.events(), [Event::RoundStarted { enemies: 0, spawn: SpawnKind::Waves, .. }]));
        assert_eq!(game.outcome(), Outcome::Playing, "no instant win with nobody on the field");
        step(&mut game, Input::default());
        assert_eq!(wave_started(&game), Some(1), "{:?}", game.events());
        let status = game.wave_status().expect("a waves round reports its status");
        assert_eq!((status.index, status.total, status.alive), (1, 2, 1));
    }

    #[test]
    fn a_rolling_in_tank_has_no_body_until_it_arrives_on_a_usable_cell() {
        let mut game = waves_game(1, 1);
        step(&mut game, Input::default());
        let entering = game.tank_snapshots().into_iter().find(|t| !t.is_player).expect("wave tank spawned");
        assert!(entering.entering);
        let p = entering.position;
        assert!(p.x < 0.0 || p.x > W || p.y < 0.0 || p.y > H, "starts outside the field: ({:.0},{:.0})", p.x, p.y);
        let slot = step_until_entered(&mut game, 600);
        let entity = game.tank_entity_by_slot(slot).expect("entered tank");
        assert!(!game.is_entering(entity));
        let arrived = game.tank_snapshots().into_iter().find(|t| !t.is_player).expect("wave tank");
        assert!(!arrived.entering);
        let p = arrived.position;
        assert!(p.x > 0.0 && p.x < W && p.y > 0.0 && p.y < H, "arrived inside: ({:.0},{:.0})", p.x, p.y);
        assert!(game.nav_grid(W, H).usable(p), "arrived on a usable nav cell");
        assert!(with_tank(&game.world, entity, |t| t.body.is_some()), "has its physics body now");
        assert!(game.world.get::<&Ai>(entity).is_ok(), "and its Ai");
    }

    #[test]
    fn wave_two_spawns_only_after_wave_one_is_cleared() {
        let mut game = waves_game(2, 1);
        let slot = step_until_entered(&mut game, 600);
        for _ in 0..600 {
            step(&mut game, Input::default());
            assert_ne!(wave_started(&game), Some(2), "wave 2 must wait for wave 1 to be cleared");
        }
        game.debug_kill(slot).unwrap();
        step(&mut game, Input::default());
        assert!(game.events().iter().any(|e| matches!(e, Event::Wreck { .. })));
        assert_eq!(game.outcome(), Outcome::Playing, "a wave is still to come");
        let gap_frames = (tuning().wave_gap_seconds * 60.0) as u32;
        let mut called_at = None;
        for frame in 1..=gap_frames + 5 {
            step(&mut game, Input::default());
            if wave_started(&game) == Some(2) {
                called_at = Some(frame);
                break;
            }
        }
        let called_at = called_at.expect("wave 2 called after the breather");
        assert!(called_at >= gap_frames - 1, "called at frame {called_at}, before the {gap_frames}-frame gap");
        assert!(game.wave_banner().is_none(), "the banner goes with the gap");
    }

    #[test]
    fn the_next_wave_joins_after_the_timeout_with_one_still_alive() {
        let mut game = waves_game(2, 1);
        step_until_entered(&mut game, 600);
        // Shield wave 1's tank the way `waves_game` already shields the
        // player: this test is about wave *pacing*, and it needs that tank
        // alive until the timeout. On an open map the tank drives at the
        // player and rams it, and ram damage is mutual - so without this it
        // eventually kills itself against the invulnerable player, live
        // enemies hit `wave_next_when_alive`, and wave 2 arrives on the
        // cleared-the-wave path well before the timeout it is meant to test.
        for tank in game.world.query_mut::<&mut Tank>().with::<&Ai>() {
            tank.shield_hp = 1.0e9;
        }
        let timeout_frames = (tuning().wave_timeout_seconds * 60.0) as u32;
        let gap_frames = (tuning().wave_gap_seconds * 60.0) as u32;
        let mut called_at = None;
        for frame in 1..=timeout_frames + gap_frames + 10 {
            step(&mut game, Input::default());
            if wave_started(&game) == Some(2) {
                called_at = Some(frame);
                break;
            }
        }
        let called_at = called_at.expect("wave 2 joins after the timeout");
        assert!(called_at >= timeout_frames, "called at frame {called_at}, before the {timeout_frames}-frame timeout");
        assert_eq!(game.wave_status().unwrap().alive, 2, "wave 1's tank is still alive alongside wave 2's");
    }

    #[test]
    fn the_live_cap_holds_with_an_oversized_wave() {
        let cap = tuning().wave_max_alive;
        let mut game = waves_game(1, cap as u32 + 4);
        let frames = ((cap as f32 + 4.0) * tuning().wave_stagger_seconds * 60.0) as u32 + 300;
        // The hard claim is the invariant: however oversized the wave, the
        // field never holds more than `wave_max_alive` at once.
        //
        // The surplus leaves the queue only as slots free up, and since
        // enemies ram each other (`ram_enemy_pairs`) a maximal pile now
        // grinds itself down: attrition settles the field a few tanks below
        // the cap rather than pinning it there, so "alive == cap while
        // tanks are still queued" no longer happens and is the wrong thing
        // to assert. What keeps this honest instead is that the field got
        // genuinely crowded while the queue was still full - if the wave
        // never built up, the invariant would hold vacuously.
        let (mut max_alive, mut max_pending) = (0, 0);
        let mut crowded_with_queue = false;
        for _ in 0..frames {
            step(&mut game, Input::default());
            let status = game.wave_status().unwrap();
            assert!(status.alive <= cap, "{} live enemies over the cap of {cap}", status.alive);
            max_alive = max_alive.max(status.alive);
            max_pending = max_pending.max(status.pending);
            crowded_with_queue |= status.alive * 4 >= cap * 3 && status.pending > 0;
        }
        assert!(
            crowded_with_queue,
            "the wave never built up: max alive {max_alive} of {cap}, max pending {max_pending}"
        );
        assert_eq!(game.outcome(), Outcome::Playing);
    }

    #[test]
    fn a_wreck_despawns_after_the_knob_in_a_wave_round() {
        let mut game = waves_game(2, 1);
        let slot = step_until_entered(&mut game, 600);
        game.debug_kill(slot).unwrap();
        step(&mut game, Input::default());
        let despawn_frames = (tuning().wave_wreck_despawn_seconds * 60.0) as u32;
        let mut removed_at = None;
        for frame in 1..=despawn_frames + 5 {
            step(&mut game, Input::default());
            if game.events().iter().any(|e| matches!(e, Event::WreckRemoved { slot: s } if *s == slot)) {
                removed_at = Some(frame);
                break;
            }
            let entity = game.tank_entity_by_slot(slot).expect("wreck still on the field");
            if frame + 65 < despawn_frames {
                assert_eq!(with_tank(&game.world, entity, Tank::alpha), 1.0, "opaque until the last second");
            }
        }
        let removed_at = removed_at.expect("the wreck was removed");
        assert!(removed_at >= despawn_frames - 1, "removed at frame {removed_at}, before {despawn_frames}");
        assert!(game.tank_entity_by_slot(slot).is_none());
        assert!(game.wave_status().unwrap().index >= 2, "wave 2 came meanwhile, so slot {slot} was never reused");
        assert!(game.tank_snapshots().iter().all(|t| t.is_player || !t.is_wreck));
    }

    // --- A wrecked seat re-entering with the next wave
    // (docs/online-coop-prd.md section 4.11, decision 7) ---

    /// Wreck `slot` and run the frame that applies it.
    fn kill_and_step(game: &mut Game, slot: usize) {
        game.debug_kill(slot).expect("a live tank in that slot");
        step(game, Input::default());
    }

    /// Step until seat `seat` is off the field driving back in, at most
    /// `limit` frames; the frame it started on.
    fn step_until_returning(game: &mut Game, seat: usize, limit: u32) -> u32 {
        for frame in 1..=limit {
            step(game, Input::default());
            let entity = game.seat(seat).expect("a seat's tank is never despawned");
            if game.is_entering(entity) {
                return frame;
            }
        }
        panic!("seat {seat} never started back in within {limit} frames");
    }

    #[test]
    fn a_wrecked_seat_drives_back_in_with_the_next_wave_and_plays_again() {
        let mut game = waves_game_seats(3, 1, 2);
        let wave_one = step_until_entered(&mut game, 600);
        kill_and_step(&mut game, 1);
        let seat1 = game.seat(1).expect("seat 1");
        assert!(with_tank(&game.world, seat1, Tank::is_wreck), "seat 1 is a wreck");
        assert_eq!(game.outcome(), Outcome::Playing, "seat 0 is still fighting");
        // It waits where it fell until the wave it belongs to is called.
        for _ in 0..60 {
            step(&mut game, Input::default());
            assert!(!game.is_entering(seat1), "a wreck waits for the next wave, it does not leave early");
        }
        // Clearing wave 1 calls wave 2, and seat 1 comes in with it.
        game.debug_kill(wave_one).expect("wave 1's tank");
        let started = step_until_returning(&mut game, 1, 900);
        assert!(started > 0);
        assert!(game.wave_status().unwrap().index >= 2, "it came with the next wave");
        let out = with_tank(&game.world, seat1, |t| (t.position, t.is_wreck(), t.body.is_some(), t.hull_points()));
        assert!(!out.1, "back at full health, not a wreck");
        assert!(!out.2, "kinematic until it is through the gate");
        assert_eq!(out.3, with_tank(&game.world, game.player().unwrap(), |t| t.hull_points()), "a fresh tank");
        assert!(out.0.x < 0.0 || out.0.x > W || out.0.y < 0.0 || out.0.y > H, "starts outside the field, at a gate");
        // It arrives, takes its body and no `Ai`, and answers its own keys.
        let mut arrived = false;
        for _ in 0..900 {
            step(&mut game, Input::default());
            if !game.is_entering(seat1) {
                arrived = true;
                break;
            }
        }
        assert!(arrived, "seat 1 never finished its roll-in");
        assert!(game.world.get::<&Ai>(seat1).is_err(), "a seat never takes an Ai");
        assert!(with_tank(&game.world, seat1, |t| t.body.is_some()), "it has its physics body again");
        let inside = with_tank(&game.world, seat1, |t| t.position);
        assert!(inside.x > 0.0 && inside.x < W && inside.y > 0.0 && inside.y < H, "arrived inside the field");
        let before = inside;
        let mut input = Input::default();
        input.seats[1] = Intent { move_dir: Some(Dir::Left), ..Intent::default() };
        for _ in 0..30 {
            step(&mut game, input);
        }
        let after = with_tank(&game.world, seat1, |t| t.position);
        assert!(after.distance_to(before) > 8.0, "seat 1 drives again: {before:?} -> {after:?}");
    }

    #[test]
    fn a_seat_driving_back_in_is_not_a_target_and_takes_no_fire() {
        let mut game = waves_game_seats(3, 1, 2);
        let wave_one = step_until_entered(&mut game, 600);
        kill_and_step(&mut game, 1);
        let seat1 = game.seat(1).expect("seat 1");
        game.debug_kill(wave_one).expect("wave 1's tank");
        step_until_returning(&mut game, 1, 900);
        for _ in 0..120 {
            step(&mut game, Input::default());
            if !game.is_entering(seat1) {
                break;
            }
            assert!(!game.seats_on_field().contains(&Some(seat1)), "off the field while it drives in");
            assert!(with_tank(&game.world, seat1, |t| !t.is_wreck()), "nothing out there can touch it");
        }
    }

    #[test]
    fn every_seat_wrecked_at_once_still_loses_the_round() {
        let mut game = waves_game_seats(3, 1, 2);
        step_until_entered(&mut game, 600);
        for seat in [0, 1] {
            game.debug_kill(seat).expect("a live seat");
        }
        step(&mut game, Input::default());
        assert_eq!(game.outcome(), Outcome::Lost, "nobody is left to hold the line");
    }

    #[test]
    fn a_round_lost_while_a_seat_waits_leaves_it_where_it_fell() {
        let mut game = waves_game_seats(3, 1, 2);
        step_until_entered(&mut game, 600);
        kill_and_step(&mut game, 1);
        let seat1 = game.seat(1).expect("seat 1");
        for _ in 0..30 {
            step(&mut game, Input::default());
        }
        // The last seat standing falls while the other is still waiting
        // for the wave that was going to bring it back.
        let player = game.player().expect("player");
        with_tank_mut(&game.world, player, |t| t.shield_hp = 0.0);
        kill_and_step(&mut game, 0);
        assert_eq!(game.outcome(), Outcome::Lost);
        // Up to the restart the end screen plays out; nothing comes back
        // on it, because `wave_phase` only runs while the round does.
        for _ in 0..(tuning().restart_delay * 60.0) as u32 - 10 {
            step(&mut game, Input::default());
            assert!(with_tank(&game.world, seat1, Tank::is_wreck), "the round is over; nobody comes back");
            assert!(!game.is_entering(seat1));
        }
        assert_eq!(game.outcome(), Outcome::Lost);
    }

    #[test]
    fn a_band_round_leaves_a_wrecked_seat_wrecked() {
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(7);
        game.player_row_override = Some(0);
        game.players = PlayerCount::TWO;
        // Destroy, so the round can only end on the tanks: the one enemy
        // and every seat are shielded, so it cannot end at all.
        game.level_overrides.mission = Some(Mission::Destroy);
        game.map = MapFile::from_toml_str(OPEN_MAP).expect("test map parses");
        game.init(W, H);
        for seat in game.players().into_iter().flatten() {
            with_tank_mut(&game.world, seat, |t| t.shield_hp = 1.0e9);
        }
        for tank in game.world.query_mut::<&mut Tank>().with::<&Ai>() {
            tank.shield_hp = 1.0e9;
        }
        kill_and_step(&mut game, 1);
        let seat1 = game.seat(1).expect("seat 1");
        for _ in 0..600 {
            step(&mut game, Input::default());
            assert!(with_tank(&game.world, seat1, Tank::is_wreck), "a band round keeps a wrecked seat wrecked");
            assert!(!game.is_entering(seat1));
        }
        assert_eq!(game.outcome(), Outcome::Playing);
    }

    #[test]
    fn a_single_seat_wave_round_loses_on_its_one_wreck() {
        let mut game = waves_game(3, 1);
        step_until_entered(&mut game, 600);
        let player = game.player().expect("player");
        with_tank_mut(&game.world, player, |t| t.shield_hp = 0.0);
        kill_and_step(&mut game, 0);
        assert_eq!(game.outcome(), Outcome::Lost, "one wreck of one seat is every seat wrecked");
        assert!(with_tank(&game.world, player, Tank::is_wreck));
        for _ in 0..600 {
            step(&mut game, Input::default());
            assert!(!game.is_entering(player), "nothing comes back: the round is over");
        }
    }

    #[test]
    fn wrecks_stay_for_the_whole_round_under_the_band_plan() {
        let mut game = game_on(OPEN_MAP, 2, Some(0));
        let slot = game.tank_snapshots().len() - 1;
        game.debug_kill(slot).unwrap();
        for _ in 0..((tuning().wave_wreck_despawn_seconds * 60.0) as u32 + 60) {
            step(&mut game, Input::default());
            assert!(!game.events().iter().any(|e| matches!(e, Event::WreckRemoved { .. })));
        }
        assert!(game.tank_entity_by_slot(slot).is_some(), "the wreck is still there");
    }

    #[test]
    fn every_tank_of_a_wave_is_in_its_tier_or_one_lower() {
        let mut game = waves_game(4, 2);
        game.level_overrides.wave_growth = Some(1);
        game.level_overrides.tier_start = Some(Tier::Light);
        game.level_overrides.tier_end = Some(Tier::Super);
        game.init(W, H);
        let player = game.player().expect("player");
        with_tank_mut(&game.world, player, |t| t.shield_hp = 1.0e9);
        let plan = game.spawn_plan;
        let mut wave = 0u32;
        let mut seen = 0;
        // Kill each tank the frame it enters, so waves follow each other
        // on the cleared-wave rule alone.
        for _ in 0..6000 {
            step(&mut game, Input::default());
            if let Some(w) = wave_started(&game) {
                wave = w;
            }
            for slot in tank_entered(&game) {
                let entity = game.tank_entity_by_slot(slot).expect("entered");
                let row = with_tank(&game.world, entity, |t| t.row);
                let tier = plan.wave_tier(wave - 1);
                let allowed = [tier, Tier::from_index(tier.index().saturating_sub(1))];
                assert!(
                    allowed.contains(&TANK_TIER_BY_ROW[row as usize]),
                    "wave {wave} ({tier:?}) brought row {row} ({:?})",
                    TANK_TIER_BY_ROW[row as usize]
                );
                seen += 1;
                game.debug_kill(slot).unwrap();
            }
            if game.outcome() != Outcome::Playing {
                break;
            }
        }
        assert_eq!(seen, 2 + 3 + 4 + 5, "every tank of every wave entered");
        assert_eq!(game.outcome(), Outcome::Won, "the round ends once the last wave is wrecked");
    }
}


