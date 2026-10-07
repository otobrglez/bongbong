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
//! explosions; `engage` hands attacking enemies distinct engagement slots;
//! `nav` builds the nav grid the AI routes by and keeps it across frames.
//!
//! Determinism: all round randomness flows from the one seeded `SmallRng`
//! in `Game::rng` (never `rand::rng()` on the simulation path, never
//! iterate a HashMap/HashSet where the body consumes RNG or spawns) -
//! `determinism_tests` replays a seed twice and bit-compares.

mod combat;
mod command;
mod comms;
mod crates;
pub mod debug;
mod director;
mod engage;
mod field;
mod flame;
mod hits;
mod missiles;
mod grenades;
mod sonic;
mod emp;
mod gauss;
mod fpv;
mod rod;
mod nav;
pub mod portals;
pub mod present;
mod props;
mod towers;
pub mod training;
mod volcano;
pub use props::{FlyingDrum, GroundFire};
pub(crate) use props::tile_rubble;
pub(crate) use gauss::SeatCharge;
pub mod replica;
#[cfg(test)]
mod crate_tests;
#[cfg(test)]
mod flame_tests;
#[cfg(test)]
mod lagcomp_tests;
#[cfg(test)]
mod props_tests;
#[cfg(test)]
mod seat_tests;
#[cfg(test)]
mod emp_tests;
#[cfg(test)]
mod fpv_tests;
#[cfg(test)]
mod gauss_tests;
#[cfg(test)]
mod rod_tests;
#[cfg(test)]
mod sonic_tests;
#[cfg(test)]
mod tower_tests;
mod waves;
#[cfg(test)]
mod weather_tests;
mod weapons;

pub use director::Pacing;
pub use waves::{RollIn, WaveStatus};
pub use weapons::{laser_reach, FlameJet};

/// Which projectile a client's provisional shot is drawn as
/// (`net::predict`): the three the wire also carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProvisionalKind {
    Shell,
    Bullet,
    Plasma,
}

/// Where a seat's hull is: what a client that owns its hull sends every
/// tick and the room puts the seat at (docs/online-coop-prd.md §4.14),
/// and what the room sends back in `Placed` when it moved the hull
/// itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeatPose {
    pub position: Position,
    /// Degrees, 0 = up, one of the four facings.
    pub rotation: f32,
    /// The body's velocity, so the room's ram reads impact speed off it.
    pub velocity: Vec2,
}

/// The least a client-owned hull may move between two poses the room
/// accepts, in ticks of its top speed. An ordinary read covers one tick
/// of the client's driving, and the hull the room compares it with can
/// stand a few ticks off it - a starved tick's guess went on a tick the
/// client did not, a contact held the room's copy back - so one pose may
/// always cover this much without the client having cheated. A read that
/// took a stall's worth of intents at once covers more, and says so
/// (`accept_seat_pose`'s `reach_ticks`).
pub const POSE_REACH_TICKS: f32 = 4.0;

/// Slack on top of the reach, in pixels: the wire's rounding and the
/// solver's own nudge on the room's copy.
pub const POSE_REACH_SLACK_PX: f32 = 8.0;

/// Ticks past a knock's skid (`sonic::knock`) the pose validator still
/// allows a client-owned hull the knock's slide: the client hears of the
/// knock a link's delay after the room put it on, and its skid runs that
/// much later.
pub const POSE_KNOCK_GRACE_TICKS: u64 = 60;

/// What the pose validator allows a client-owned seat past its chassis's
/// reach for the knocks the room put on it (`sonic::knock`,
/// `Game::accept_seat_pose`): each pose may go `speed` px/s further, and
/// all of them together no further than `budget` px - the longest slide
/// the knocks could give, on the slipperiest ground - until frame `until`,
/// the last skid's end and `POSE_KNOCK_GRACE_TICKS` past it. A client can
/// claim no knock the room did not put on it, nor more than one gives.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct SeatKnock {
    speed: f32,
    budget: f32,
    until: u64,
}

impl SeatKnock {
    /// This allowance and a knock of `speed` px/s for `skid` seconds put on
    /// at frame `frame`: the faster of the two speeds while either holds,
    /// the two slides together, until the later end.
    fn with(self, speed: f32, skid: f32, frame: u64) -> SeatKnock {
        let t = tuning();
        let live = if frame <= self.until { self } else { SeatKnock::default() };
        let slide = crate::sonic::slide(&t, speed, t.sonic_skid_grip_floor) + POSE_REACH_SLACK_PX;
        let until = frame + (skid / PHYSICS_FIXED_DT).ceil() as u64 + POSE_KNOCK_GRACE_TICKS;
        SeatKnock { speed: live.speed.max(speed), budget: live.budget + slide, until: until.max(live.until) }
    }

    /// How much further than its reach a pose covering `ticks` ticks may go
    /// at frame `frame` (px).
    fn extra(&self, frame: u64, ticks: f32) -> f32 {
        if frame > self.until {
            return 0.0;
        }
        (self.speed * PHYSICS_FIXED_DT * ticks).min(self.budget)
    }

    /// A pose went `past` px further than its reach: that much of the
    /// budget is spent.
    fn spend(&mut self, past: f32) {
        if past > 0.0 {
            self.budget = (self.budget - past).max(0.0);
        }
    }
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
    /// Its state as the kind's sheet column (`ShellState::col` and the
    /// like) and the time in it: a provisional runs the real projectile's
    /// state machine (`ProvisionalShot::advance`, `simulation::present`),
    /// muzzle frames first, as the room's copy does.
    pub state: i32,
    pub timer: f32,
    /// The impact frames have played out.
    pub done: bool,
}
use waves::WaveState;

use crate::blast::{BlastFx, Scorch};
use crate::decal::Decal;
use crate::tuning::tuning;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use hecs::Entity;
use rand::rngs::SmallRng;
use rand::{RngExt, SeedableRng};
use serde::{Deserialize, Serialize};
use crate::math::Vec2;

use crate::ai::{Ai, AiSnapshot, Intent, Mover, Role, SpecialSense, WallAhead};
use crate::battlefield;
use crate::bullet::Bullet;
use crate::frog::{Facing, Frog, Side};
use crate::laser::{LaserBeam, LaserVariant};
use crate::missile::Missile;
use crate::grenade::Grenade;
use crate::level::{LevelOverrides, Mission, SpawnPlan};
use crate::map::{self, CellObject, MapFile};
use crate::obstacle::{Drum, Material, Obstacle, neighbour_mask};
use crate::pathfind::Grid;
use crate::physics::Physics;
use crate::pickup::{Pickup, PickupKind};
use crate::plasma::{Plasma, PlasmaVariant};
use crate::shell::{Owner, Shell, ShellState};
use crate::shockwave::Shockwave;
use crate::tank::{ActiveWeapon, Dir, Tank, TankKind, Trigger};
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
    TANK_FRAME_SIZE,
    SHOCK_MAX,
    RUBBLE_ROW_TANK,
    SCORCH_MAX,
    MAX_SEATS,
    PATHFIND_CELL_SIZE,
    PHYSICS_FIXED_DT,
    PHYSICS_MAX_CATCHUP_SECONDS,
    Position,
    TANK_SHELL_VARIANT_BY_ROW,
    TANK_TRACK_FRAMES,
    TANK_WRECK_COLS,
};

use combat::{frog_hop_target, ram, HitEffects};
use engage::{EngageCtx, EngageReport, EngageRing, EngageStatus, EngageTank};
use hits::{HitBoxFrame, HitBoxHistory, REWIND_MAX_TICKS, ShellTarget, Terrain};
use weapons::{dispatch_fire, dispatch_fire_from, laser_damage_range, laser_end, tick_queued_shots, PendingLaserShot, Projectile, Rewind, laser_beam_half_width};

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
    /// and `lamp` and every one-shot toggle is either input's. The caller keeps the
    /// input of a rendered frame that ran no step and folds it into the
    /// next frame's, so a tap that lands between two steps still reaches
    /// the simulation as a held fire key or a pressed toggle; directions
    /// are `self`'s alone - a held key is read fresh every frame.
    pub fn or_presses(mut self, pending: Input) -> Input {
        for (seat, carried) in self.seats.iter_mut().zip(pending.seats) {
            seat.fire |= carried.fire;
            seat.lamp |= carried.lamp;
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

/// The beats of a decided round's end (`Game::end_beats`), in seconds: the
/// finale, in which the world plays on with nothing over it; the verdict's
/// fade, in which the end screen eases in; and the countdown to the way on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EndBeats {
    pub finale: f32,
    pub fade: f32,
    pub countdown: f32,
}

impl EndBeats {
    /// The whole end screen, which `restart_timer` counts down from.
    pub fn total(&self) -> f32 {
        self.finale + self.fade + self.countdown
    }

    /// How far the end screen has eased in, 0 to 1, `since` seconds after
    /// the round was decided.
    pub fn verdict(&self, since: f32) -> f32 {
        if since < self.finale {
            0.0
        } else if self.fade <= 0.0 {
            1.0
        } else {
            ((since - self.finale) / self.fade).clamp(0.0, 1.0)
        }
    }
}

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

/// What a round's end screen reports (docs/levels.md): how long it was
/// played and how many enemies went down, and to whom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct RoundStats {
    /// Seconds of play: the round clock (`Game::time`, which stands still
    /// behind the mission banner and while paused) when the round ended,
    /// or now while it runs.
    pub seconds: f32,
    /// Enemy tanks wrecked this round, by anyone or anything.
    pub destroyed: u32,
    /// Every enemy the round brings: the band, or all of its waves.
    pub enemies: u32,
    /// Of `destroyed`, the ones each seat dealt the last damage to
    /// (`Tank::last_hit_by`). A wreck no seat touched - a drum, a fire, a
    /// frog's bite - counts for the team alone.
    pub by_seat: [u32; MAX_SEATS],
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
    /// The room moved a client-owned hull itself - a refused pose, a
    /// portal, a gate - and the client snaps to it
    /// (docs/online-coop-prd.md §4.14).
    Placed { seat: usize, x: f32, y: f32, rotation: f32 },
    /// A laser beam was drawn from the laser module's lens at (`x0`, `y0`)
    /// to where the gun line stopped it at (`x1`, `y1`): an instant hit
    /// leaves nothing in the world for a snapshot to carry, so the beam
    /// itself is the event a replica draws it from (`LaserVariant::name`).
    /// A beam bent by portals is one event a leg (docs/teleporting.md,
    /// "Shots"): `leg` 0 leaves the lens, a later one the portal the last
    /// came out of, and `portal` says the leg ends going into a portal
    /// rather than where the beam stopped.
    LaserBeam { x0: f32, y0: f32, x1: f32, y1: f32, variant: &'static str, seat: u8, leg: u8, portal: bool },
    /// A velocity change the room put on a client-owned hull (knockback, a
    /// blast, a ram, a missile launch's recoil, a sonic hammer's knock);
    /// the owner applies it to its own body, since the room places that
    /// hull wherever the owner says (docs/online-coop-prd.md §4.16).
    /// `skid` is the seconds a knock takes the hull off its tracks
    /// (`Tank::skid`, docs/sonic-hammer.md), 0 for every other shove.
    Shoved { seat: usize, vx: f32, vy: f32, skid: f32 },
    /// A projectile, beam or wave landed on `target` at (`x`, `y`);
    /// `cause` says which kind of thing it was, for what draws it.
    Hit { target: HitTarget, damage: f32, killed: bool, x: f32, y: f32, cause: HitCause },
    /// A sonic hammer fired by `slot` from the pivot (`x`, `y`) along `dir`
    /// (`Dir::name`) (docs/sonic-hammer.md): the wave a replica draws.
    /// Logged right after its `Fired`, in the same tick.
    SonicBlast { slot: usize, x: f32, y: f32, dir: &'static str },
    /// An enemy in owner slot `slot` began the wind-up of `weapon`
    /// (`ActiveWeapon::name`, `tank::Tell`). Not sent: the tell's state
    /// travels in `TankState::tell`.
    TellStarted { slot: usize, weapon: &'static str },
    /// An EMP fired by `slot` from the pivot (`x`, `y`) (docs/emp-burst.md):
    /// the ring a replica draws. Logged right after its `Fired`, in the
    /// same tick.
    EmpPulse { slot: usize, x: f32, y: f32 },
    /// The tank in owner slot `slot`, at (`x`, `y`), was disabled by an
    /// EMP's ring (`Tank::disable`). Not sent: `TankState::disabled` is
    /// what draws.
    Disabled { slot: usize, x: f32, y: f32 },
    /// The tower at (`x`, `y`) went offline under an EMP's ring
    /// (`Tower::disable`). Not sent: the tile's `DISABLED` flag is what
    /// draws.
    TowerDisabled { x: f32, y: f32 },
    /// A missile an EMP killed came down at (`x`, `y`) a dud: no blast.
    MissileDud { x: f32, y: f32 },
    /// One leg of a gauss rail's slug fired by `slot` (docs/gauss-rail.md):
    /// drawn from (`x0`, `y0`) to where it stopped or went into a portal
    /// (`portal`), `leg` 0 first, and every thing it went through, in
    /// order, where it went in. `seat` is `net::wire::NO_SEAT` for an
    /// enemy's. Leg 0 is logged right after its `Fired`, in the same tick.
    RailSlug {
        slot: usize,
        seat: u8,
        leg: u8,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        portal: bool,
        overcharged: bool,
        pierced: Vec<(f32, f32, crate::gauss::Pierced)>,
    },
    /// The tank in owner slot `slot` started a charge of `weapon`
    /// (`ActiveWeapon::name`, `Tank::charge`). Not sent: the charge's state
    /// travels in `TankState::charge`.
    ChargeStarted { slot: usize, weapon: &'static str },
    /// The charge of `weapon` on the tank in owner slot `slot` ended
    /// without firing: let go early, held past its vent, or lost.
    ChargeEnded { slot: usize, weapon: &'static str, end: crate::tank::ChargeEnd },
    /// A drone `id` left the halo of the tank in owner slot `slot` from its
    /// slot's ground point (`x`, `y`) (docs/fpv-swarm.md), locked on the
    /// tank in owner slot `target`, on the players' frog (`frog`), or on
    /// nothing. Logged after its `Fired`, in the same tick.
    DroneLaunched { id: u32, slot: usize, x: f32, y: f32, target: Option<usize>, frog: bool },
    /// The drone `id`, launched by `slot`, burst at (`x`, `y`) - `crown` in
    /// a tree's leaves.
    DroneBurst { id: u32, slot: usize, x: f32, y: f32, crown: bool },
    /// The drone `id` was struck in the air over (`x`, `y`), `height` px
    /// up, by `by` (`air::AirStrike::name`), and falls.
    DroneDowned { id: u32, x: f32, y: f32, height: f32, by: &'static str },
    /// A downed drone `id` reached the ground at (`x`, `y`): a dud.
    DroneCrashed { id: u32, x: f32, y: f32 },
    /// The drone `id` lost its lock (`why`: "wreck", "gone", "canopy",
    /// "teleport") and dives where its target last was. Not sent: the
    /// drone's `lock` is the state.
    DroneLockLost { id: u32, why: &'static str },
    /// The tank in owner slot `slot` called a rod (docs/rod-from-god.md) on
    /// map cell `cell`, landing at round time `land`; `seat` is
    /// `net::wire::NO_SEAT` for an enemy's. Logged after its `Fired`, in the
    /// same tick.
    RodCalled { id: u32, slot: usize, seat: u8, cell: (i32, i32), land: f32 },
    /// The rod `id` landed on map cell `cell`: it left a crater (`crater`)
    /// and set a volcano off (`erupted`). What it crushed, shoved and broke
    /// follows as the ordinary events.
    RodImpact { id: u32, cell: (i32, i32), crater: bool, erupted: bool },
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
    /// `slot` took a `kind` pickup at (`x`, `y`) - where its opening plays
    /// (`fx::CrateOpen`). `spilled`: it was a broken crate's contents lying
    /// loose (`Pickup::loose`), so there was no crate left to open.
    PickupCollected { slot: usize, kind: PickupKind, x: f32, y: f32, spilled: bool },
    /// A `kind` crate at (`x`, `y`) broke under a blast or fire
    /// (`crate_breakable`, `simulation::crates`): `cooked`, its contents
    /// went off in a blast of their own; otherwise they spilled and lie
    /// there loose.
    CrateBroken { kind: PickupKind, x: f32, y: f32, cooked: bool },
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
    /// A field map took the straggler in `slot` off the field at (`x`,
    /// `y`), out of every seat's sight, to roll it in again through a gate
    /// nearer the fight (`Game::reroll_stragglers`); its `TankEntered`
    /// follows when it is through. Not sent: a replica sees the hull
    /// leave and come back through the gate by itself.
    Rerolled { slot: usize, x: f32, y: f32 },
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
    /// A drum launched from (`x`, `y`) toward (`to_x`, `to_y`), where it
    /// will detonate when it lands: a fuel drum another blast set off, or
    /// any drum a sonic hammer's wave threw.
    DrumLaunched { x: f32, y: f32, to_x: f32, to_y: f32, drum: Drum },
    /// Seat `seat` set a lantern down at (`x`, `y`) (docs/volcano.md).
    LanternSet { seat: u8, x: f32, y: f32 },
    /// A blast broke the lantern at (`x`, `y`).
    LanternBroken { x: f32, y: f32 },
    /// An erupting volcano threw a lava bomb from its crater at (`x`, `y`)
    /// toward (`to_x`, `to_y`), where it will burst when it lands
    /// (docs/volcano.md).
    LavaBombLaunched { x: f32, y: f32, to_x: f32, to_y: f32 },
    /// `slot`'s tank entered the portal at (`x`, `y`) and was placed at
    /// (`to_x`, `to_y`), beside another portal (`Game::portal_phase`).
    Teleported { slot: usize, x: f32, y: f32, to_x: f32, to_y: f32 },
    /// A shot or a laser beam went into a portal at (`x`, `y`) - the point
    /// of its path nearest the anchor - and came out of another at
    /// (`to_x`, `to_y`), on the same heading (`simulation::portals`,
    /// docs/teleporting.md). `id` is the projectile's (`Shell::id`),
    /// `None` for a beam, whose legs are its `LaserBeam`s.
    ShotTeleported { id: Option<u32>, x: f32, y: f32, to_x: f32, to_y: f32 },
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
    /// A grenade launched by `slot` went off at (`x`, `y`), its fuse spent.
    GrenadeBlast { slot: usize, x: f32, y: f32 },
    /// A delayed secondary pop from a wreck's ammo cooking off. Purely
    /// cosmetic - it deals no damage - but recorded so tooling and the
    /// presentation layer can see it.
    CookOff { x: f32, y: f32 },
    /// A tesla coil's bolt, from its terminal at (`x0`, `y0`) to the tank
    /// it struck at (`x1`, `y1`); `chained` for a jump on from the first
    /// target (docs/defence-towers-prd.md section 4). The strike itself is
    /// the `Hit` that follows.
    TeslaStrike { x0: f32, y0: f32, x1: f32, y1: f32, chained: bool },
    /// A tower fired from its muzzle at (`x`, `y`) along `heading`: a gun
    /// tower's burst began, or a bio slush lobbed a glob (`kind`:
    /// `gun_tower`, `bio_slush`).
    TowerFired { kind: &'static str, x: f32, y: f32, heading: f32 },
    /// A glob of ooze landed at (`x`, `y`) and splashed.
    GlobSplashed { x: f32, y: f32 },
    /// Tank `slot` was coated in ooze, by a splash or a puddle. A coat on
    /// a tank already coated refreshes it without an event.
    Slimed { slot: usize },
    /// Tank `slot` drove into water and washed its ooze off.
    SlimeWashed { slot: usize },
    /// A tower pack restored the `side` tower at (`x`, `y`) to full health
    /// and put it out.
    TowerRepaired { side: Side, x: f32, y: f32 },
    /// A training beat (1-based) was done (docs/training-stage.md).
    BeatDone { beat: usize },
    /// A training door of beat `beat` opened at (`x`, `y`): its cell is
    /// open ground from now on.
    DoorOpened { beat: usize, x: f32, y: f32 },
    /// Seat `seat` took the training flag at (`x`, `y`).
    FlagTaken { seat: usize, x: f32, y: f32 },
    /// A training round's fallen frog got up again at (`x`, `y`), and the
    /// beat it fell in starts over.
    FrogRevived { x: f32, y: f32 },
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

/// What landed in an `Event::Hit`, so a picture can be chosen by its cause
/// (docs/sonic-hammer.md): every shot, beam and stream draws the burst it
/// always has, and a sonic hammer's wave a flash and dust rather than
/// fire. A weapon that lands its own kind of hit adds its cause here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitCause {
    Shell,
    Bullet,
    Plasma,
    Laser,
    Flame,
    /// A tesla tower's strike.
    Tesla,
    Sonic,
    /// A gauss rail's slug going through.
    Rail,
    /// A rod from god crushing a hull in its circle.
    Rod,
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
    /// The tick of the world each seat's client was drawing when it made
    /// the input this tick applies, in 256ths of a tick: what lag
    /// compensation rewinds the enemies to for that seat's shots
    /// (`set_seat_view`, docs/online-coop-prd.md §4.16). `None` for a seat
    /// with no client view (a local round, a bot).
    seat_view: [Option<(u32, u8)>; MAX_SEATS],
    /// The update a seat's client owns its hull for (`accept_seat_pose`),
    /// by the frame number that update will run as; `drive_player` puts
    /// nothing on it that frame, since the pose already did. One update
    /// only: a seat whose packets stop saying where it is - a client that
    /// dropped, one that went back to stage 2 - is the room's to drive
    /// again on the next tick without anybody having to release it.
    seat_owned: [u64; MAX_SEATS],
    /// The knocks the room put on each client-owned seat (`sonic::knock`)
    /// that `accept_seat_pose` still allows for (`SeatKnock`).
    seat_knock: [SeatKnock; MAX_SEATS],
    /// The sonic hammer's waves running out over the field
    /// (docs/sonic-hammer.md), in the order they were fired: struck by the
    /// rules on the round that simulates them (`tick_sonic_waves`), only
    /// drawn on a replica. Cleared by `init`.
    pub(crate) sonic_waves: Vec<crate::sonic::SonicWave>,
    /// The EMP pulses running out over the field (docs/emp-burst.md), in
    /// the order they were fired: striking on the round that simulates
    /// them (`tick_emp_pulses`), only drawn on a replica. Cleared by `init`.
    pub(crate) emp_pulses: Vec<crate::emp::EmpPulse>,
    /// The gauss rail's slugs as they are drawn (docs/gauss-rail.md), a leg
    /// each, aged in `tick_effects` until their trail has thinned: put on
    /// by `rail_show`. Cleared by `init`.
    pub(crate) rail_slugs: Vec<crate::gauss::RailSlug>,
    /// Charges that ended without firing, as they are drawn - a fizzle, a
    /// vent (`charge_end_show`). Cleared by `init`.
    pub(crate) charge_ends: Vec<crate::gauss::ChargeEndFx>,
    /// A room's count of the ticks each seat's client has held its trigger,
    /// by the update it is for (`set_seat_hold`, docs/gauss-rail.md "The
    /// hold report"): what a charge counts instead of its own, held to it
    /// within `CHARGE_HOLD_SPARE_TICKS`. One update only, like
    /// `seat_owned`.
    seat_hold: [Option<(u64, u32)>; MAX_SEATS],
    /// Seconds every lamp post on the map stays dark after an EMP at night
    /// (docs/emp-burst.md "At 11"), 0 while they are lit
    /// (`lit_lamp_posts`). Counted down in `tick_timers`.
    pub(crate) lamps_out: f32,
    /// Tall-grass cells a sonic wave flattened, with the seconds they hide
    /// nobody for yet: taken out of the cover `Terrain` reads
    /// (`cover_cells`). Ordered, ticked down in `tick_timers`.
    pub(crate) grass_flat: BTreeMap<(i32, i32), f32>,
    /// What stands on the field and ends on the round clock - a rod's call
    /// (`zone.rs`, docs/rod-from-god.md) - sorted by id: what the dangers,
    /// the router, the off-screen arrows, the minimap and the wire read. A
    /// replica's are the room's, from the wire. Cleared by `init`.
    pub(crate) zones: Vec<crate::zone::Zone>,
    /// Seconds ahead of `time` the zones' countdowns are drawn on: 0 in a
    /// local round; a client's lead to its own present online
    /// (`set_zone_lead`). Presentation only.
    pub(crate) zone_lead: f32,
    /// The craters the rods left this round (`rod::Craters`): pits the
    /// footing slows and the router prices, fords while it rains. Cleared by
    /// `init`.
    pub(crate) craters: crate::rod::Craters,
    /// The rods' impacts as they are drawn - the column, the dust, the
    /// debris - aged in `tick_effects` (`rod_show`). Cleared by `init`.
    pub(crate) rod_impacts: Vec<crate::rod::RodImpactFx>,
    /// How each seat has been moving (`SeatStill`, `tick_seat_still`): what
    /// an enemy with a rod calls on a standing seat by.
    seat_still: [crate::rod::SeatStill; MAX_SEATS],
    /// A room's report of the cell each seat's client has its reticle on, by
    /// the update it is for (`set_seat_reticle`, docs/rod-from-god.md "The
    /// reticle report"), one update only, like `seat_hold`.
    seat_reticle: [Option<(u64, (i32, i32))>; MAX_SEATS],
    /// The enemy and frog hit boxes of the last `REWIND_MAX_TICKS` ticks,
    /// recorded at the end of each update: what a seat's shot is swept
    /// against when its client was drawing the past (`seat_rewind`).
    hit_history: HitBoxHistory,
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
    /// The round is fought on a field map - one the camera follows,
    /// `MapFile::class` - and the enemies are bounded the way such a map
    /// needs (`field`: chained alerts and leashes instead of the shared
    /// alert, far enemies thinking less, spawns and gates by their walk to
    /// the fight). Set by `init` from the map; false on every arena, which
    /// plays exactly as it always has.
    pub(crate) field_map: bool,
    /// Blasts and fire break the pickups' crates this round
    /// (`simulation::crates`) - `crate_breakable`, read once by `init`, so
    /// a round keeps the rule it began under.
    pub(crate) crates_breakable: bool,
    /// How many enemies the band plan placed at init. Zero is a sandbox
    /// round (`tanks = 0` / `--enemies 0`): nothing to wreck, so it never
    /// ends by wreck count - only by the player's or the frog's death.
    pub(crate) band_enemy_count: usize,
    /// The wave scheduler (`waves.rs`): idle under the band plan apart
    /// from handing out owner slots.
    wave: WaveState,
    /// The pacing director (`director`): how hard each seat is pressed,
    /// and from that the breather before a field map's next wave. Read
    /// and written by `wave_phase` on a field map's wave round only.
    director: director::Director,
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
    /// The map's lava as the rules see it (docs/volcano.md): what is deep
    /// (static colliders, like deep water), what is a burning ford, which
    /// way it flows and the heat it radiates. Built from the map's cells
    /// beside the water in `init`; empty on a map without lava.
    pub(crate) lava: crate::lava::LavaLayout,
    /// Fading tread marks, oldest first. Kept out of `world`: pure visual
    /// trail data nothing ever queries alongside another component.
    pub(crate) tracks: Vec<Track>,
    /// Seconds since the round started; drives animation. Read by `render`.
    pub(crate) time: f32,
    pub(crate) outcome: Outcome,
    /// Seconds until the automatic restart once the round has ended - or,
    /// on a held end screen, until the caller takes over: the whole of
    /// `end_beats`, the finale and the verdict's fade first.
    pub(crate) restart_timer: f32,
    /// The end screen's countdown runs out into a hold instead of a
    /// restart: `restart_timer` counts `end_beats` down to zero and
    /// stays there until the caller starts the next round (`init`, or the
    /// R key's restart). A level's end screen is held, because where it
    /// goes - the next level or the same one again - is the session's to
    /// decide (docs/levels.md). A setting, kept across restarts.
    pub hold_end_screen: bool,
    /// The round clock when the round ended (`end_round`), for
    /// `round_stats`; `None` while it runs.
    ended_at: Option<f32>,
    /// Enemy wrecks this round, and the ones credited to each seat
    /// (`credit_wreck`).
    enemies_destroyed: u32,
    wrecks_by_seat: [u32; MAX_SEATS],
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
    /// `mark_caps` (`SCORCH_MAX` on an arena).
    pub(crate) scorches: Vec<Scorch>,
    /// Rubble left where a wall tile died this round, oldest first, capped
    /// at `mark_caps` (`DECAL_MAX` on an arena; see `decal.rs`).
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
    /// The volcanoes on the field (docs/volcano.md), one per crater cell
    /// in the map's order (`volcano::build_volcanoes`).
    pub(crate) volcanoes: Vec<crate::volcano::Volcano>,
    /// The last rumble and eruption each volcano's show was staged for
    /// (`Game::eruption_show`), so each plays once.
    pub(crate) eruptions_shown: Vec<(i64, i64)>,
    /// Lava bombs in the air, about to burst where they land
    /// (`volcano::tick_lava_bombs`).
    pub(crate) lava_bombs: Vec<crate::volcano::LavaBomb>,
    /// The lanterns the seats have set down this round (docs/volcano.md,
    /// `lamp::Lantern`), in the order they were set down.
    pub(crate) lanterns: Vec<crate::lamp::Lantern>,
    /// A training map's run (docs/training-stage.md, `training.rs`):
    /// `None` on every other map, which runs none of it.
    pub(crate) training: Option<training::Run>,
    /// The next lantern's id (`lamp::Lantern::id`), counted up per round.
    lantern_next_id: u16,
    /// Lanterns each seat has left to set down this round
    /// (`lamps_per_seat` where `lamps_in_play`, else none).
    pub(crate) lamps_left: [u8; MAX_SEATS],
    /// Whether each seat's lamp key was down last frame: a lantern goes
    /// down on the press, not while it is held.
    player_lamp_held_last_frame: [bool; MAX_SEATS],
    /// Night has fallen on a map with `nightfall` (`Game::fall_night`):
    /// the sky is night's from here to the round's end.
    pub(crate) night_fallen: bool,
    /// The whole-screen flash a kill or a barrel opens with: its age in
    /// seconds while one is playing (`game.rs` fades it out over
    /// `blast_screen_flash_seconds`). Explicit state rather than derived
    /// from `blast_fx`, so a cook-off never drives it and `flash_screen`
    /// can space flashes out.
    pub(crate) screen_flash: Option<f32>,
    /// How strong the flash playing is against a drum's (`flash_screen_with`):
    /// its peak and its length are this many times a drum's.
    pub(crate) screen_flash_strength: f32,
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
    /// The defence towers' weapons (docs/defence-towers-prd.md), keyed by
    /// the cell their `Obstacle` stands on. A `BTreeMap` because the tower
    /// phase walks it and draws RNG, so its order is part of a replay.
    pub(crate) towers: BTreeMap<(i32, i32), crate::tower::Tower>,
    /// Globs of ooze in the air, oldest first (`towers::resolve_globs`).
    pub(crate) globs: Vec<crate::tower::Glob>,
    /// Ooze on the ground, per cell: slimes whatever drives over it until
    /// it dries or a fire takes it. Ordered like `heat`.
    pub(crate) ooze: BTreeMap<(i32, i32), crate::tower::OozePuddle>,
    /// Tesla bolts still in their short display window.
    pub(crate) tesla_bolts: Vec<crate::tower::TeslaBolt>,
    /// What dead towers left on the ground, oldest first.
    pub(crate) tower_ruins: Vec<crate::tower::TowerRuin>,
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
    /// The couch's own seat count while a training round seats one
    /// (docs/training-stage.md): `init` sets it aside on a map with a
    /// training script and puts it back on the next map without one.
    couch_players: Option<PlayerCount>,
    /// `--seed`: pins the round seed, so every restart replays the
    /// identical round - the repro loop for a round the probe flagged.
    pub seed_override: Option<u64>,
    /// The builder's PLAY HERE (`mode::Session::play_here`): the map cell
    /// seat 1 starts on in place of the map's `start`, the map itself left
    /// as it is. Set before `init` and kept across restarts, so a round
    /// tried from here starts here again; PLAY, a level and a map put in
    /// the round's place clear it. With it set, seat 2 takes the fallback
    /// beside seat 1 rather than the map's `start2`. `None` - every round
    /// but the builder's test from a spot - is the map's own start.
    pub start_override: Option<(i32, i32)>,
    /// The seed this round actually ran with (see `round_seed()`).
    round_seed: u64,
    /// The sky this round is fought and drawn under (docs/weather.md),
    /// fixed by `init` from the map's `weather` key, the round seed (a
    /// `random` sky's pick) and, unless `weather_from_map`, the
    /// `weather_override` knob - `Game::weather()`. Never `Random`.
    pub(crate) weather: crate::map::Weather,
    /// `init` reads the map's `weather` key alone, not the window's
    /// `weather_override` knob: set on a room's replica and sandbox and on
    /// the rig's authority, because a room's round is fought under its
    /// map's sky and every client has to draw and drive under that one.
    /// Kept across restarts.
    pub weather_from_map: bool,
    /// The static bodies `init` put on the deep water cells, which a
    /// snowfall mid-round (`change_weather`) takes off as the water ices.
    deep_water_bodies: Vec<rapier2d::prelude::RigidBodyHandle>,
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
    /// `render` skips the ground tileset and its floor shade and leaves
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
    /// Tests only: work every flow field out whole as the frame's routing
    /// grid is built (`Grid::settle_fields`), the yardstick a field worked
    /// out only as far as it is read is held to.
    #[cfg(test)]
    pub(crate) whole_fields: bool,
    /// The nav grid kept across frames (`nav`): its occupancy and labels,
    /// rebuilt only where the terrain changed. Emptied by `init`.
    pub(crate) nav: nav::NavCache,
    /// Tests only: build every frame's routing grid from scratch
    /// (`route_grid`) rather than on the kept one, the yardstick the kept
    /// grid is held to.
    #[cfg(test)]
    pub(crate) scratch_nav: bool,
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
    /// Crates a blast or fire broke this frame (`crate_breakable`), in the
    /// same worklist: a crate cooking off can break another.
    crate_breaks: Vec<Entity>,
    blast_fx: Vec<BlastFx>,
    scorches: Vec<Scorch>,
    decals: Vec<Decal>,
    pending_shells: Vec<Shell>,
    pending_plasmas: Vec<Plasma>,
    pending_bullets: Vec<Bullet>,
    pending_missiles: Vec<Missile>,
    pending_grenades: Vec<Grenade>,
    pending_lasers: Vec<PendingLaserShot>,
    /// Sonic hammer blasts fired this frame, cast into waves by
    /// `resolve_sonic` once the tank loops are done.
    pending_sonic: Vec<sonic::PendingSonic>,
    /// EMP pulses fired this frame, put on the field by `resolve_emp` once
    /// the tank loops are done.
    pending_emp: Vec<emp::PendingEmp>,
    /// Gauss rail slugs released this frame, traced by `resolve_rails` once
    /// the tank loops are done.
    pending_rails: Vec<gauss::PendingRail>,
    /// FPV drones launched this frame, put in the world by `launch_drones`
    /// once the tank loops are done.
    pending_drones: Vec<fpv::PendingDrone>,
    /// Rods called this frame, put on the field as zones by `place_calls`
    /// once the tank loops are done.
    pending_calls: Vec<rod::PendingCall>,
    /// Charges that ended this frame without firing, put on the field by
    /// `resolve_rails`.
    charge_ends: Vec<crate::gauss::ChargeEndFx>,
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
    /// The velocity changes put on client-owned seats this frame, which
    /// `finish_frame` turns into `Event::Shoved`.
    shoves: Shoves,
}

/// The velocity changes the room puts on hulls their clients own
/// (docs/online-coop-prd.md §4.16, "Shoves on owned hulls"): hit
/// knockback, blast shoves (a missile's included), ram pushes and a
/// missile launch's recoil. The room places an owned hull wherever its
/// client says, so a shove applied here alone would be erased by the next
/// pose - each one travels as an `Event::Shoved` for the owner to apply to
/// its own body. The recoil of a shell, bolt or bullet is not among them:
/// the client kicks its hull itself at each launch (`net::predict`).
///
/// `owned` is set once per update from `Game::seat_owned`; everything
/// else is pushed unconditionally and kept only for an owned seat, so a
/// round with no client-owned hull - every local round - keeps nothing
/// and emits nothing.
#[derive(Debug, Default)]
pub(super) struct Shoves {
    owned: [bool; MAX_SEATS],
    log: Vec<(usize, Vec2, f32)>,
    /// The knocks `allow_knock` keeps: the validator's allowance, no event.
    quiet: Vec<(usize, Vec2, f32)>,
}

impl Shoves {
    /// A shove of `dv` px/s on the tank `owner` names, kept only when that
    /// tank is a seat its client owns this update.
    pub(super) fn push(&mut self, owner: Owner, dv: Vec2) {
        self.push_knock(owner, dv, 0.0);
    }

    /// `push`, for a knock that takes the hull off its tracks for `skid`
    /// seconds (`sonic::knock`).
    pub(super) fn push_knock(&mut self, owner: Owner, dv: Vec2, skid: f32) {
        if let Owner::Player(seat) = owner
            && self.owned.get(seat as usize).copied().unwrap_or(false)
        {
            self.log.push((seat as usize, dv, skid));
        }
    }

    /// A knock on an owned seat its client puts on itself - a gauss rail's
    /// recoil, kicked on the release (`net::predict`): not sent, but the
    /// pose validator still allows the hull its speed (`Game::seat_knock`).
    pub(super) fn allow_knock(&mut self, owner: Owner, dv: Vec2, skid: f32) {
        if let Owner::Player(seat) = owner
            && self.owned.get(seat as usize).copied().unwrap_or(false)
        {
            self.quiet.push((seat as usize, dv, skid));
        }
    }

    fn into_events(self) -> impl Iterator<Item = Event> {
        self.log.into_iter().map(|(seat, dv, skid)| Event::Shoved { seat, vx: dv.x, vy: dv.y, skid })
    }
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
            crate_breaks: Vec::new(),
            blast_fx: Vec::new(),
            scorches: Vec::new(),
            decals: Vec::new(),
            pending_shells: Vec::new(),
            pending_plasmas: Vec::new(),
            pending_bullets: Vec::new(),
            pending_missiles: Vec::new(),
            pending_grenades: Vec::new(),
            pending_lasers: Vec::new(),
            pending_sonic: Vec::new(),
            pending_emp: Vec::new(),
            pending_rails: Vec::new(),
            pending_drones: Vec::new(),
            pending_calls: Vec::new(),
            charge_ends: Vec::new(),
            flame_jets: Vec::new(),
            muzzle_flashes: Vec::new(),
            impact_flashes: Vec::new(),
            shocks: Vec::new(),
            physics_stepped: false,
            events: Vec::new(),
            shoves: Shoves::default(),
        }
    }

    /// Take `show`'s cosmetics into the frame's, each list in its own
    /// order, as if the phase had pushed them itself.
    fn stage(&mut self, show: Spectacle) {
        self.blast_fx.extend(show.blast_fx);
        self.scorches.extend(show.scorches);
        self.decals.extend(show.decals);
        self.muzzle_flashes.extend(show.muzzle_flashes);
        self.impact_flashes.extend(show.impact_flashes);
        self.shocks.extend(show.shocks);
    }
}

/// The cosmetics one cause lays down - a kill, a drum's blast, a missile's
/// burst, a cook-off pop - gathered by the cause's `*_show` method, the
/// one place that decides them. A round stages them into its `Frame`; a
/// client replica (docs/online-coop-prd.md section 4.16), which never
/// runs the phases, hands them to `Game::show` off the server's events,
/// so both pictures come from the same code. Hashed from positions, never
/// drawn from the round RNG.
#[derive(Default)]
pub(crate) struct Spectacle {
    pub(crate) blast_fx: Vec<BlastFx>,
    pub(crate) scorches: Vec<Scorch>,
    pub(crate) decals: Vec<Decal>,
    pub(crate) muzzle_flashes: Vec<Shockwave>,
    pub(crate) impact_flashes: Vec<Shockwave>,
    pub(crate) shocks: Vec<Shockwave>,
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
    /// Put `sky` on the round's map - one sky, or several of which the
    /// round's seed picks one, as `init` would - and fight the rest of the
    /// round under it, no restart: the look, sight, grip and gusts change at once (the
    /// override knob still outranks the map's key, as in `init`). Snow
    /// ices the water over where it was open - its deep cells lose their
    /// bodies and the nav grid is built again - but ice does not thaw
    /// back mid-round, since a hull may stand on what would be deep
    /// water; the next round settles the water from its own sky. Draws no
    /// RNG.
    pub fn change_weather(&mut self, sky: crate::map::Skies) {
        self.map.weather = sky;
        let t = tuning();
        self.weather = crate::weather::in_force(sky, self.round_seed(), self.weather_from_map, &t);
        if crate::weather::freezes(self.weather, &t) && !self.water.is_frozen() {
            self.water.freeze();
            for body in std::mem::take(&mut self.deep_water_bodies) {
                self.physics.remove_body(body);
            }
            self.nav.clear();
        }
        // A sky that rains fills every crater a rod has left
        // (docs/rod-from-god.md); one turned to snow froze them above.
        if crate::weather::fills_craters(self.weather, &t) {
            self.fill_craters();
        }
    }

    /// Set up a fresh round: player, map terrain, enemies, frog, pickups,
    /// ground. Also the restart path. `width`/`height` are the battlefield
    /// size in pixels.
    pub fn init(&mut self, width: f32, height: f32) {
        // A training round is played alone: a couch's second seat waits
        // out the course and is back for the next map. No RNG.
        if self.map.training.is_some() {
            if self.players != PlayerCount::ONE {
                self.couch_players = Some(self.players);
                self.players = PlayerCount::ONE;
            }
        } else if let Some(count) = self.couch_players.take() {
            self.players = count;
        }
        // The only `rand::rng()` on the simulation path: it picks the seed,
        // so an unseeded round is replayable once its seed is printed.
        let seed = self.seed_override.unwrap_or_else(|| rand::rng().random());
        self.round_seed = seed;
        let mut rng = SmallRng::seed_from_u64(seed);
        // The sky is the round's from here on: the rules read it (a
        // frozen lake below, sight, grip and gusts every frame) as the
        // renderer does, so it is settled once rather than asked of the
        // knobs frame by frame. A hash of the seed, no RNG.
        self.weather = crate::weather::in_force(self.map.weather, seed, self.weather_from_map, &tuning());
        // Lanterns for every seat where the dark is part of the round.
        let lamps = if self.lamps_in_play() { tuning().lamps_per_seat.clamp(0, u8::MAX as i32) as u8 } else { 0 };
        self.lamps_left = [lamps; MAX_SEATS];

        self.world = hecs::World::new();
        self.tracks.clear();
        self.time = 0.0;
        self.outcome = Outcome::Playing;
        self.restart_timer = 0.0;
        self.ended_at = None;
        self.enemies_destroyed = 0;
        self.wrecks_by_seat = [0; MAX_SEATS];
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
        self.lava_bombs.clear();
        self.lanterns.clear();
        self.lantern_next_id = 0;
        self.player_lamp_held_last_frame = [false; MAX_SEATS];
        self.night_fallen = false;
        self.screen_flash = None;
        self.screen_flash_strength = 1.0;
        self.screen_flash_cooldown = 0.0;
        self.grass_cells.clear();
        self.grass.clear();
        self.laser_beams.clear();
        self.flame_jets.clear();
        self.heat.clear();
        self.flame_contacts.clear();
        self.towers.clear();
        self.globs.clear();
        self.ooze.clear();
        self.tesla_bolts.clear();
        self.tower_ruins.clear();
        self.frame = 0;
        self.hit_history.clear();
        // The water and the portals are the round's: what was kept of the
        // last round's nav grid goes with them.
        self.nav.clear();
        self.seat_view = [None; MAX_SEATS];
        // `frame` starts over, so an update number held from the last
        // round would name one of this round's.
        self.seat_owned = [0; MAX_SEATS];
        self.seat_knock = [SeatKnock::default(); MAX_SEATS];
        self.sonic_waves.clear();
        self.grass_flat.clear();
        self.emp_pulses.clear();
        self.rail_slugs.clear();
        self.charge_ends.clear();
        self.seat_hold = [None; MAX_SEATS];
        self.zones.clear();
        self.craters.clear();
        self.rod_impacts.clear();
        self.seat_still = [crate::rod::SeatStill::default(); MAX_SEATS];
        self.seat_reticle = [None; MAX_SEATS];
        self.lamps_out = 0.0;
        self.next_shot_id = 0;
        self.last_engage.clear();
        self.debug_kills.clear();
        self.debug_detonations.clear();
        self.seats = [None; MAX_SEATS];
        self.frog = None;
        self.enemy_frog = None;
        self.mission = self.level_overrides.resolve_mission(&self.map.mission);
        self.spawn_plan = self.level_overrides.resolve_spawn(&self.map.spawn, self.enemy_count_override);
        self.field_map = self.map.class() == crate::framing::MapClass::Field;
        self.crates_breakable = tuning().crate_breakable;
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
        // Under a snowy sky every lake and ford is ice (docs/weather.md):
        // no deep cell is left for a collider, a spawn roll or the nav
        // grid to keep a hull off, and no current runs.
        if crate::weather::freezes(self.weather, &tuning()) {
            self.water.freeze();
        }
        let deep_cells: HashSet<(i32, i32)> = self.water.deep_grid_cells().collect();
        self.deep_water_bodies.clear();
        for (gx, gy) in self.water.deep_grid_cells() {
            let half = battlefield::tile_hull_half_extent(&deep_cells, gx, gy, OBSTACLE_GRID_SIZE * 0.5);
            let body = self.physics.spawn_static(map::cell_to_world(gx, gy), half);
            self.deep_water_bodies.push(body);
        }

        // --- Lava (docs/volcano.md) ---
        // Water's shape and water's deep cells - a static collider each,
        // nothing in `world`, so every shot flies over - but its own rules:
        // it never freezes, it flows away from the volcano and it burns.
        // An empty map's layout is empty and spawns nothing.
        self.lava = {
            let painted = |pick: fn(&CellObject) -> bool| -> Vec<Position> {
                self.map.iter_cells().filter(|(_, _, o)| pick(o)).map(|(c, r, _)| map::cell_to_world(c, r)).collect()
            };
            crate::lava::LavaLayout::build(
                width,
                height,
                &painted(|o| matches!(o, CellObject::Wall { .. } | CellObject::Road)),
                &painted(|o| matches!(o, CellObject::Lava)),
                &self.map.volcano_cells(),
                tuning().lava_heat_falloff,
            )
        };
        let deep_lava: HashSet<(i32, i32)> = self.lava.deep_grid_cells().collect();
        for &(gx, gy) in &deep_lava {
            let half = battlefield::tile_hull_half_extent(&deep_lava, gx, gy, OBSTACLE_GRID_SIZE * 0.5);
            self.physics.spawn_static(map::cell_to_world(gx, gy), half);
        }

        // --- Player ---
        let row = resolve_player_row(self.player_row_override, tuning().player_tank, self.map.tank, || {
            rng.random_range(0..TANK_VARIANTS)
        });
        // The builder's PLAY HERE cell, else the map's start cell, else the
        // nearest non-wall cell to the center so a wall at the center
        // doesn't spawn the player inside it.
        let start_cell = self.start_override.or_else(|| self.map.start_cell()).unwrap_or_else(|| {
            let (center_col, center_row) = map::world_to_cell(Position::new(width / 2.0, height / 2.0));
            self.map.nearest_free_cell(center_col, center_row)
        });
        let start_cell = dry_cell_near(&self.map, &self.water, &self.lava, start_cell);
        let mut tank = Tank {
            row,
            shell_variant: TANK_SHELL_VARIANT_BY_ROW[row as usize],
            damage_variant: rng.random_range(0..DAMAGE_VARIANTS),
            position: map::cell_to_world(start_cell.0, start_cell.1),
            owner: Owner::Player(0),
            ..Tank::default()
        };
        if rng.random_range(0.0..1.0) < tuning().spawn_shield_chance {
            tank.raise_shield();
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
        // The towers' weapons, one per tower tile. No RNG.
        self.build_towers(width, height);
        // The volcanoes, one per crater. No RNG.
        self.build_volcanoes();
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
        // A tuft is kept off the tiles round its cell (`grass::keep_off`);
        // trees are left out, since every tuft is drawn under them.
        self.grass = {
            let tiles: HashSet<(i32, i32)> =
                self.world.query::<&Obstacle>().iter().filter(|o| !o.material.is_tree()).map(|o| o.cell()).collect();
            let t = tuning();
            self.grass_cells.iter().flat_map(|c| crate::grass::tufts_for_cell(&t, *c, |cell| tiles.contains(&cell))).collect()
        };
        self.grass.sort_by(|a, b| a.base.y.total_cmp(&b.base.y));
        // Deep water counts as terrain for every clearance roll below:
        // no enemy, frog or bonus spawns in a lake.
        let mut obstacle_positions = map_spawn.obstacle_positions;
        obstacle_positions.extend(self.water.deep_cells());
        // And lava of every depth: nothing spawns to burn.
        obstacle_positions.extend(self.lava.cells().map(|(c, r)| map::cell_to_world(c, r)));
        let wall_positions = map_spawn.wall_positions;
        let map_road_cells = map_spawn.road_cells;
        let map_water_cells = map_spawn.water_cells;
        let map_lava_cells = map_spawn.lava_cells;
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
        self.director = director::Director::default();

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
            // a `start2` inside player 1's clearance, or in a round started
            // from the builder's spot, whose `start2` is by the map's start
            // and not by player 1 - takes the nearest usable nav cell to
            // player 1 that keeps a clear tank's width from every seat
            // already down, moved ashore if that lands it in a lake.
            let placed: Vec<Position> = std::iter::once(center).chain(others.iter().copied()).collect();
            let position = (seat == 1 && self.start_override.is_none())
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
                    let dry = dry_cell_near(&self.map, &self.water, &self.lava, cell);
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
                tank.raise_shield();
            }
            roll_track_distortion(&mut tank, &mut rng);
            tank.body = Some(self.physics.spawn_tank(tank.position, tank.move_half_extents(false), tank.mass()));
            self.seats[seat] = Some(self.world.spawn((tank,)));
            others.push(position);
        }
        let clear_of_others = |pos: Position| others.iter().all(|&c| pos.distance_to(c) >= clear);

        // A field map's band is not a ring in from the edges but the cells
        // outside every seat's sight box, those whose walk to the nearest
        // seat is about `field_walk_seconds` where the map has any
        // (`field::spawn_cells`). An
        // arena has none, draws nothing for them and places every enemy in
        // its band exactly as before; a field map whose cells run out falls
        // back to that band too.
        let field_cells = if self.field_map && enemy_count > 0 {
            let seats: Vec<Position> = std::iter::once(center).chain(others.iter().copied()).collect();
            field::spawn_cells(&spawn_grid, &seats, &obstacle_positions)
        } else {
            Vec::new()
        };

        let mut enemy_positions: Vec<Position> = Vec::with_capacity(enemy_count);
        // The field cells clear of the seats, less each enemy's
        // neighbourhood as it goes down (`field::SpawnPool`).
        let mut field_pool = field::SpawnPool::new(&field_cells, |pos| pos.distance_to(center) >= clear && clear_of_others(pos));
        while enemy_positions.len() < enemy_count {
            let field_pick = field_pool.pick(&mut rng, &enemy_positions);
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
            let pos = field_pick.unwrap_or_else(|| {
                battlefield::sample_clear_position(&mut rng, width, height, margin_min, |pos| {
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
                })
            });
            let erow = TANK_SPRITE_ORDER[enemy_positions.len() % TANK_SPRITE_ORDER.len()];
            let mut enemy = roll_enemy_tank(&mut rng, erow, pos, self.first_enemy_slot() + enemy_positions.len());
            // Last of the per-enemy rolls, and skipped outright at a zero
            // share, so a mission without hunters draws exactly what a
            // Destroy round does.
            let role = roll_role(self.mission, &mut rng);
            enemy.body = Some(self.physics.spawn_tank(pos, enemy.move_half_extents(false), enemy.mass()));
            enemy_positions.push(pos);
            field_pool.place(pos, enemy_clear);
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
            spawn_pickup_at(&mut self.world, pos, kind, None);
        }
        // Bonus shields roll only once every slot is placed, so a bonus
        // can't land on a slot that hasn't spawned yet.
        for &(pos, kind) in &self.map_pickup_slots {
            if kind == PickupKind::Health {
                // The frog was created at full health a few lines up, so
                // no frog pack is ever rolled here and round setup draws
                // the RNG it always drew.
                maybe_spawn_health_slot_bonuses(&mut self.world, &self.map, pos, width, height, BonusGates::default(), None, &mut rng);
            }
        }

        // --- Ground: road under every wall tile and explicit road cell
        // (props stand on plain ground) ---
        let mut road_cells = wall_positions.clone();
        road_cells.extend(map_road_cells);
        // Lava runs over dirt: what shows between its banks' rounded
        // corners is a road's floor, never grass.
        road_cells.extend(map_lava_cells);
        // Wall cells go in twice on purpose: folded into the road set they
        // paint dirt underfoot, and passed separately they cast the baked
        // shading that makes a wall look like it is standing on the floor
        // rather than pasted onto it. A round's field also shades its edge.
        let look = crate::ground::Look { theme: self.map.theme, edge_shade: true };
        self.ground = crate::ground::build(width, height, rng.random(), &road_cells, &map_water_cells, &wall_positions, look);

        // --- Training (docs/training-stage.md): the script's run, on the
        // seats and the map just laid out. No RNG. ---
        self.init_training();

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
            stun_timer: 0.0,
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
        self.tick_nightfall();
        self.tick_timers(dt, &mut rng);
        let terrain = Terrain::build(&self.world, width, height, &self.cover_cells(), &self.water);
        // One nav grid for the whole frame - labelled, priced and with a
        // flow field per shared target (`route_grid_on`), on the occupancy
        // and labels the round keeps across frames (`refresh_nav`) -
        // passed to the phases that need it rather than parked on `Frame`:
        // a borrow living there would alias every `&mut Frame` the other
        // phases take.
        self.refresh_nav(width, height);
        let grid = self.route_grid_on(self.nav.base().without_fields(), width, height);
        #[cfg(test)]
        let grid = if self.scratch_nav { self.route_grid(width, height) } else { grid };
        let mut f = Frame::new(dt, width, height, rng, terrain);
        for (owned, &frame) in f.shoves.owned.iter_mut().zip(&self.seat_owned) {
            *owned = frame != 0 && frame == self.frame;
        }
        let eruption = self.eruption_show();
        f.stage(eruption);

        if self.outcome == Outcome::Playing {
            self.apply_debug_kills(&mut f);
            self.apply_debug_detonations(&mut f);
            self.frog_phase(&mut f);
            self.pickup_phase(&mut f);
            self.portal_phase(&mut f, &grid);
            self.player_phase(input, &mut f);
            self.rollin_phase(&mut f);
            self.tick_seat_still(f.dt);
            self.enemy_phase(&mut f, &grid);
            self.wave_phase(&mut f);
            self.tower_phase(&mut f);
            self.volcano_phase(&mut f);
            self.spawn_pending(&mut f);
            self.guide_missiles(&mut f);
            self.guide_drones(&mut f);
            self.resolve_lasers(&mut f);
            self.resolve_rails(&mut f);
            self.resolve_flames(&mut f);
            self.resolve_sonic(&mut f);
            self.tick_sonic_waves(&mut f, true);
            self.resolve_emp(&mut f);
            self.tick_emp_pulses(&mut f, true);
            self.resolve_zones(&mut f, true);
            self.step_world(&mut f, true);
            self.sync_tanks_and_ram(&mut f);
            self.ram_props(&mut f);
            self.roll_grenades(&mut f);
            self.shell_vs_shell(&mut f);
            self.resolve_projectiles::<Shell>(&mut f, true);
            self.resolve_projectiles::<Bullet>(&mut f, true);
            self.resolve_projectiles::<Plasma>(&mut f, true);
            self.resolve_missiles(&mut f, true);
            self.resolve_drones(&mut f, true);
            self.resolve_grenades(&mut f, true);
            self.resolve_globs(&mut f, true);
            self.tick_cookoffs(&mut f);
            self.tick_burns(&mut f);
            self.tick_fires(&mut f, true);
            self.lava_phase(&mut f);
            self.tick_ooze(&mut f);
            self.tick_fuses(&mut f);
            self.tick_launches(&mut f);
            self.tick_lava_bombs(&mut f);
            self.tick_crates(&mut f);
            self.tick_grass(f.dt);
            self.drain_shield_breaks(&mut f);
            self.explosions(&mut f, true);
            self.despawn_wrecks(&mut f);
            self.training_phase(&mut f);
            self.cleanup_done();
            self.check_round_end(&mut f);
        } else {
            // Round over: the scene keeps animating (wrecks burn, in-flight
            // shots land without dealing damage, a barrel cascade finishes
            // without hurting anyone) while the restart counts down.
            // Physics doesn't step, so nothing drifts.
            self.guide_missiles(&mut f);
            self.tick_sonic_waves(&mut f, false);
            self.tick_emp_pulses(&mut f, false);
            self.resolve_zones(&mut f, false);
            self.step_world(&mut f, false);
            self.roll_grenades(&mut f);
            self.resolve_projectiles::<Shell>(&mut f, false);
            self.resolve_projectiles::<Bullet>(&mut f, false);
            self.resolve_projectiles::<Plasma>(&mut f, false);
            self.resolve_missiles(&mut f, false);
            self.resolve_drones(&mut f, false);
            self.resolve_grenades(&mut f, false);
            self.resolve_globs(&mut f, false);
            self.tick_cookoffs(&mut f);
            self.tick_burns(&mut f);
            self.tick_fires(&mut f, false);
            self.tick_fuses(&mut f);
            self.tick_launches(&mut f);
            self.tick_lava_bombs(&mut f);
            self.tick_crates(&mut f);
            self.tick_grass(f.dt);
            self.explosions(&mut f, false);
            self.cleanup_done();
            if self.hold_end_screen {
                self.restart_timer = (self.restart_timer - dt).max(0.0);
            } else {
                self.restart_timer -= dt;
                if self.restart_timer <= 0.0 {
                    self.finish_frame(f);
                    self.init(width, height);
                    return;
                }
            }
        }
        self.hit_history.record(&self.world, self.frame);
        self.finish_frame(f);
    }

    /// Append the frame's effects and events and put the RNG back.
    fn finish_frame(&mut self, f: Frame) {
        let Frame { blast_fx, scorches, decals, muzzle_flashes, impact_flashes, shocks, events, shoves, rng, .. } = f;
        for &(seat, dv, skid) in shoves.log.iter().chain(&shoves.quiet) {
            if skid > 0.0 && seat < MAX_SEATS {
                self.seat_knock[seat] = self.seat_knock[seat].with(dv.length(), skid, self.frame);
            }
        }
        self.show(Spectacle { blast_fx, scorches, decals, muzzle_flashes, impact_flashes, shocks });
        self.events.extend(events);
        self.events.extend(shoves.into_events());
        self.rng = Some(rng);
    }

    /// How many scorches and rubble decals the round keeps: `SCORCH_MAX`
    /// and `DECAL_MAX` on an arena, and on a field map as many per
    /// standard field of its area, so a large map keeps its marks as
    /// densely as an arena keeps its own rather than wearing away the
    /// ones still on screen.
    pub fn mark_caps(&self) -> (usize, usize) {
        mark_caps(&self.map)
    }

    /// Put `show` on screen, each list held to its ceiling: scorches and
    /// decals (`mark_caps`) drop the oldest, `SHOCK_MAX` the weakest.
    /// Every frame of a round ends here, and a replica's spectacle comes
    /// here too.
    pub(crate) fn show(&mut self, show: Spectacle) {
        self.muzzle_flashes.extend(show.muzzle_flashes);
        self.impact_flashes.extend(show.impact_flashes);
        self.blast_fx.extend(show.blast_fx);
        let (scorch_max, decal_max) = self.mark_caps();
        self.scorches.extend(show.scorches);
        if self.scorches.len() > scorch_max {
            let excess = self.scorches.len() - scorch_max;
            self.scorches.drain(..excess);
        }
        self.decals.extend(show.decals);
        if self.decals.len() > decal_max {
            let excess = self.decals.len() - decal_max;
            self.decals.drain(..excess);
        }
        self.shocks.extend(show.shocks);
        if self.shocks.len() > SHOCK_MAX {
            // Evict by punch left, not by age: a cascade's little fuse pops
            // arrive after the tank explosion that set them off, and
            // oldest-first would throw away the one that matters.
            self.shocks.sort_by(|a, b| b.remaining().total_cmp(&a.remaining()));
            self.shocks.truncate(SHOCK_MAX);
        }
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
            if *age >= tuning().blast_screen_flash_seconds * self.screen_flash_strength.max(1.0) {
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
        let t = tuning();
        for slug in &mut self.rail_slugs {
            slug.age += dt;
        }
        self.rail_slugs.retain(|slug| !slug.done(&t));
        for end in &mut self.charge_ends {
            end.age += dt;
        }
        self.charge_ends.retain(|end| !end.done());
        for fx in &mut self.rod_impacts {
            fx.age += dt;
        }
        self.rod_impacts.retain(|fx| !fx.done(&t));
        self.tick_tower_effects(dt);
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
        // A training round that starts its seats with no shells holds
        // their refill until they have opened an ammo crate (`training.rs`).
        let shells_held = self.training.as_ref().is_some_and(|run| run.shells_held);
        for tank in self.world.query::<&mut Tank>().iter() {
            tank.hit_by_seat = None;
            if !(shells_held && tank.owner().is_player()) {
                tank.tick_recharge(dt);
            }
            tank.fire_cooldown = (tank.fire_cooldown - dt).max(0.0);
            tank.ram_cooldown = (tank.ram_cooldown - dt).max(0.0);
            if tank.portal_cooldown > 0.0 && !portals.iter().any(|p| p.distance_to(tank.position) <= trigger_radius) {
                tank.portal_cooldown = (tank.portal_cooldown - dt).max(0.0);
            }
            tank.hit_flash_timer = (tank.hit_flash_timer - dt).max(0.0);
            tank.speed_boost_timer = (tank.speed_boost_timer - dt).max(0.0);
            tank.heat_shield_timer = (tank.heat_shield_timer - dt).max(0.0);
            tank.tick_shield(dt);
            tank.burn_timer = (tank.burn_timer - dt).max(0.0);
            tank.wet_timer = (tank.wet_timer - dt).max(0.0);
            tank.tick_wreck(dt);
            tank.tick_minigun_spin(dt);
            tank.tick_missile_pod();
            tank.tick_recoil(dt);
            tank.skid = (tank.skid - dt).max(0.0);
            tank.spin = (tank.spin - dt).max(0.0);
            tank.tick_disabled(dt);
            if tank.is_wreck() {
                tank.tell = None;
                // A dead hand releases nothing: the charge is lost.
                tank.charge = None;
                tank.spin = 0.0;
                tank.skid = 0.0;
                tank.disabled = 0.0;
                tank.special_offline = 0.0;
            }
            if roll_wreck_col(tank, rng) {
                // The frame a tank becomes a wreck: stop it behaving like
                // an air-hockey puck. See `Physics::settle_wreck`.
                if let Some(body) = tank.body {
                    self.physics.settle_wreck(body, tuning().wreck_linear_damping, tuning().wreck_friction);
                }
            }
        }
        // Flattened grass stands back up and hides again (`cover_cells`).
        self.grass_flat.retain(|_, left| {
            *left -= dt;
            *left > 0.0
        });
        // An EMP's outages run out: the towers come back online, the lamp
        // posts light again.
        for tower in self.towers.values_mut() {
            tower.disabled = (tower.disabled - dt).max(0.0);
        }
        self.lamps_out = (self.lamps_out - dt).max(0.0);
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
            tank.tick_minigun_spin(dt);
            tank.tick_recoil(dt);
            let depth = self.water.depth_at(tank.position);
            if depth.is_wet() {
                tank.wet_timer = wet_seconds;
            } else {
                tank.wet_timer = (tank.wet_timer - dt).max(0.0);
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
    /// implementation that could disagree. The predictor itself steps
    /// `predict_seat_with`, which adds the charge-and-hold trigger.
    #[cfg(test)]
    pub(crate) fn predict_seat(&mut self, seat: usize, intent: Intent, dt: f32) {
        self.predict_seat_with(seat, intent, dt, None);
    }

    /// `predict_seat`, with the seat's charge-and-hold trigger stepped too
    /// (docs/gauss-rail.md) when `trigger` names its press edge and whether
    /// the client's gate would let a press start a charge: in the room's
    /// order - the hull driven, then the trigger, then a release's recoil
    /// (and an overcharge's spin), as `resolve_rails` kicks it before
    /// `step_world`, then the solver's step - so a client's own hull crawls,
    /// recoils and spins on the ticks the room's does. What the trigger
    /// did, for a seat whose trigger is a charge.
    pub(crate) fn predict_seat_with(&mut self, seat: usize, intent: Intent, dt: f32, trigger: Option<(bool, bool)>) -> Option<gauss::SeatCharge> {
        let entity = self.seats.get(seat).copied().flatten()?;
        // Disjoint borrows: `drive_tank` wants the solver and the hull at
        // once, and they are two fields of the same struct.
        let Game { world, physics, water, lava, craters, weather, time, map, .. } = self;
        let Ok(mut tank) = world.get::<&mut Tank>(entity) else { return None };
        if tank.body.is_none() || tank.is_wreck() {
            return None;
        }
        // The sandbox's clock stands at the last snapshot's tick, so a
        // gust reaches the predicted hull when the drawn sand front does.
        let footing = Footing::at(water, lava, craters, *weather, tank.position, *time);
        // The seat's own skid and spin run on the client's ticks, as the
        // room's do in `tick_timers` (`Predictor::shove` starts a skid).
        tank.skid = (tank.skid - dt).max(0.0);
        tank.spin = (tank.spin - dt).max(0.0);
        // The room's order (`drive_player`): a rod's reticle up takes the
        // stick and the hull stands.
        let aiming = tank.charge.and_then(|c| c.weapon.charge_rule()).is_some_and(|r| r.stick == crate::tank::Stick::Aim);
        let drive = if aiming { Intent { move_dir: None, face: None, ..intent } } else { intent };
        drive_tank(physics, &mut tank, drive, dt, footing);
        let mut charged = None;
        if let Some((pressed, open)) = trigger
            && (tank.active_weapon().trigger() == Trigger::Charge || tank.charge.is_some())
        {
            let weapon = tank.charge.map_or_else(|| tank.active_weapon(), |c| c.weapon);
            let dir = Dir::from_rotation(tank.rotation).unwrap_or(Dir::Up).vec();
            let (start, muzzle) = (tank.gun_line_muzzle(dir), crate::gauss::muzzle(&tank));
            let steer = crate::rod::Steer { stick: if aiming { intent.move_dir } else { None }, ..crate::rod::Steer::default() };
            let (edge, reticle) = tank.step_trigger(intent.fire, pressed, dt, open, None, steer, map.field_size());
            if let crate::tank::ChargeEdge::Released(stage) = edge
                && weapon == ActiveWeapon::GaussRail
            {
                gauss::recoil_hull(physics, &mut tank, dir, stage == crate::tank::ChargeStage::Overcharged, footing);
            }
            let hull_cell = crate::map::world_to_cell(tank.position);
            charged = Some(gauss::SeatCharge { edge, weapon, start, muzzle, dir, reticle, hull_cell });
        }
        physics.step();
        // The solver moved the body; the tank's own position is what
        // every reader (and the next tick's `Footing`) goes by.
        if let Some(handle) = tank.body {
            tank.position = physics.position(handle);
        }
        charged
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

    /// Where one seat's hull is, for the room to compare against the
    /// pose it applied and for a client to report its own.
    pub fn seat_pose(&self, seat: usize) -> Option<SeatPose> {
        let (position, rotation, velocity) = self.seat_motion(seat)?;
        Some(SeatPose { position, rotation, velocity })
    }

    /// A client that owns its hull says where it is: put the seat there
    /// if the pose is one the hull could have reached, and mark the seat
    /// as owned so this tick's `drive_player` leaves it alone
    /// (docs/online-coop-prd.md §4.14).
    ///
    /// **Validation, not simulation.** The step from where the room has
    /// the hull is bounded by the chassis's top speed over `reach_ticks`
    /// ticks - never fewer than `POSE_REACH_TICKS` - plus
    /// `POSE_REACH_SLACK_PX`. `reach_ticks` is how much of the client's
    /// driving the pose covers, which the mailbox read that delivered it
    /// measures (`net::mailbox::Mailbox::pose_reach_ticks`): one tick for
    /// an ordinary read, the whole stall for the read that ends one,
    /// since an owned read takes every intent at or before its play
    /// point at once. The pose
    /// must lie inside the field, off a solid tile and out of deep water;
    /// a wreck and a seat still rolling in through a gate own nothing. A
    /// refusal leaves the hull where it was, and the room answers with
    /// `Placed` so the client comes back to it.
    pub fn accept_seat_pose(&mut self, seat: usize, pose: SeatPose, reach_ticks: u32) -> Result<(), &'static str> {
        let Some(entity) = self.seats.get(seat).copied().flatten() else { return Err("no such seat") };
        let (from, reach, knock) = {
            let Ok(tank) = self.world.get::<&Tank>(entity) else { return Err("no tank") };
            if tank.is_wreck() {
                return Err("a wreck");
            }
            if tank.body.is_none() {
                return Err("still entering");
            }
            let ticks = (reach_ticks as f32).max(POSE_REACH_TICKS);
            // The ground's own drift - a current, a gust - carries a hull
            // past its top speed, and the rules put it there.
            let flow = Footing::at(&self.water, &self.lava, &self.craters, self.weather, tank.position, self.time).flow;
            let drift = (flow.x * flow.x + flow.y * flow.y).sqrt();
            let reach = (tank.effective_speed() + drift) * PHYSICS_FIXED_DT * ticks + POSE_REACH_SLACK_PX;
            // A knock carries it past that too, by no more than the knock
            // could slide it in all (`SeatKnock`).
            (tank.position, reach, self.seat_knock[seat].extra(self.frame, ticks))
        };
        let (dx, dy) = (pose.position.x - from.x, pose.position.y - from.y);
        let step = (dx * dx + dy * dy).sqrt();
        if step > reach + knock {
            return Err("further than the hull could have gone");
        }
        let (width, height) = self.map.field_size();
        let inset = OBSTACLE_GRID_SIZE * 0.5;
        if !(inset..=width - inset).contains(&pose.position.x) || !(inset..=height - inset).contains(&pose.position.y) {
            return Err("outside the field");
        }
        let (col, row) = map::world_to_cell(pose.position);
        if self.map.solid_at(col, row) {
            return Err("inside a solid tile");
        }
        if self.water.depth_at(pose.position) == crate::ground::Depth::Deep {
            return Err("in deep water");
        }
        if self.lava.depth_at(pose.position) == crate::ground::Depth::Deep {
            return Err("in deep lava");
        }
        // One tick back along the reported velocity: the coming update's
        // solver step carries the body forward by exactly that much, so
        // it ends the tick on the pose the client reported - with the
        // velocity the contacts and `combat::ram` read - rather than one
        // tick past it.
        let back = Position::new(
            pose.position.x - pose.velocity.x * PHYSICS_FIXED_DT,
            pose.position.y - pose.velocity.y * PHYSICS_FIXED_DT,
        );
        self.seat_knock[seat].spend(step - reach);
        self.place_seat(seat, back, pose.rotation, pose.velocity);
        self.seat_owned[seat] = self.frame + 1;
        Ok(())
    }

    /// The tick of the world a seat's client was drawing when it made the
    /// input this tick applies (`IntentMsg::view_tick`/`view_frac`); set
    /// by the room before the update, read by lag compensation.
    pub fn set_seat_view(&mut self, seat: usize, view_tick: u32, view_frac: u8) {
        if let Some(view) = self.seat_view.get_mut(seat) {
            *view = (view_tick > 0).then_some((view_tick, view_frac));
        }
    }

    /// How many whole ticks behind this update the world a shot's owner
    /// was drawing stands: lag compensation's rewind, "favor the shooter"
    /// (docs/online-coop-prd.md §4.16). The update's frame less the seat's
    /// view (`set_seat_view`, a tick plus 256ths), rounded to the nearest
    /// tick and clamped to `0..=REWIND_MAX_TICKS`. 0 - the present - for
    /// an enemy, and for a seat with no view: every local round, the
    /// probe, a bot seat.
    pub(crate) fn seat_rewind(&self, owner: Owner) -> u8 {
        let Owner::Player(seat) = owner else { return 0 };
        let Some(&Some((tick, frac))) = self.seat_view.get(seat as usize) else { return 0 };
        // In 256ths of a tick, so the rounding is exact.
        let behind = (self.frame as i64) * 256 - ((tick as i64) * 256 + frac as i64);
        ((behind + 128).div_euclid(256)).clamp(0, REWIND_MAX_TICKS as i64) as u8
    }

    /// The enemy and frog boxes `rewind` ticks before this update, or
    /// `None` for the present: a rewind of 0, or a tick the history does
    /// not hold (before the round's first ticks, across a pause).
    fn rewound_boxes(&self, rewind: u8) -> Option<&HitBoxFrame> {
        if rewind == 0 {
            return None;
        }
        self.frame.checked_sub(rewind as u64).and_then(|tick| self.hit_history.at(tick))
    }

    /// The room drives this seat again from the next update: nobody is
    /// reporting its pose.
    pub fn release_seat(&mut self, seat: usize) {
        if let Some(owned) = self.seat_owned.get_mut(seat) {
            *owned = 0;
        }
    }

    /// Whether a client owns this seat's hull for the coming update (or
    /// the one that just ran).
    pub fn seat_is_owned(&self, seat: usize) -> bool {
        self.seat_owned.get(seat).is_some_and(|&f| f != 0 && f >= self.frame)
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
                let mut shell = Shell::at(id, shot.position, shot.prev_position, shot.velocity, shot.rotation, shot.variant, shot.shooter_row, owner);
                shell.state = ShellState::from_col(shot.state).unwrap_or(ShellState::Flying);
                shell.timer = shot.timer;
                self.world.spawn((shell,));
            }
            ProvisionalKind::Bullet => {
                let mut bullet = Bullet::at(id, shot.position, shot.prev_position, shot.velocity, shot.rotation, shot.shooter_row, owner);
                bullet.state = crate::bullet::BulletState::from_col(shot.state).unwrap_or(crate::bullet::BulletState::Flying);
                bullet.timer = shot.timer;
                self.world.spawn((bullet,));
            }
            ProvisionalKind::Plasma => {
                let mut plasma = Plasma::at(id, shot.position, shot.prev_position, shot.velocity, shot.rotation, shot.plasma_variant, shot.shooter_row, owner);
                plasma.state = crate::plasma::PlasmaState::from_col(shot.state).unwrap_or(crate::plasma::PlasmaState::Flying);
                plasma.timer = shot.timer;
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
    /// of the last snapshot. `offline` is the client's own word that its
    /// special is offline (an EMP's press the room has not answered yet):
    /// the shell cannon fires, as the room's offline will have it. `None`
    /// for a wreck or an empty seat.
    pub(crate) fn seat_arms(&self, seat: usize, offline: bool) -> Option<(ActiveWeapon, i32, f32)> {
        let entity = self.seats.get(seat).copied().flatten()?;
        let tank = self.world.get::<&Tank>(entity).ok()?;
        if tank.is_wreck() {
            return None;
        }
        let weapon = if offline { ActiveWeapon::Shell } else { tank.active_weapon() };
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
        // It starts where the room's copy starts: in the kind's first
        // muzzle state.
        let state = match kind {
            ProvisionalKind::Shell => ShellState::Fire0.col(),
            ProvisionalKind::Bullet => crate::bullet::BulletState::Muzzle.col(),
            ProvisionalKind::Plasma => crate::plasma::PlasmaState::Fire0.col(),
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
            state,
            timer: 0.0,
            done: false,
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
        let terrain = Terrain::build(&self.world, width, height, &self.cover_cells(), &self.water);
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
        // Night falls on the round clock, which stands on the room's tick.
        self.tick_nightfall();
        self.tick_wave_banner(dt);
        self.ease_hulls(dt);
        self.fade_tracks(dt);
        self.tick_grass(dt);
        self.tick_burn_frames(dt);
        self.fade_fires(dt);
        self.fade_wrecks(dt);
        // A wreck's fire burns down on the replica's own clock: the wire
        // carries that a tank is a wreck, not how long it has burnt.
        for tank in self.world.query_mut::<&mut Tank>() {
            tank.tick_wreck(dt);
            // A heat shield's ring drains on the replica's own clock; the
            // room's flag going off is what ends it.
            if tank.heat_shield_timer > 0.0 {
                tank.heat_shield_timer = (tank.heat_shield_timer - dt).max(f32::EPSILON);
            }
            // An enemy's tell and a hull's skid run down the same way,
            // until the room's word ends them (`TankState::{tell, skid}`).
            if let Some(tell) = tank.tell.as_mut() {
                tell.left = (tell.left - dt).max(f32::EPSILON);
            }
            if tank.skid > 0.0 {
                tank.skid = (tank.skid - dt).max(f32::EPSILON);
            }
            // An EMP's outages, likewise (`TankState::{disabled, offline}`).
            if tank.disabled > 0.0 {
                tank.disabled = (tank.disabled - dt).max(f32::EPSILON);
            }
            if tank.special_offline > 0.0 {
                tank.special_offline = (tank.special_offline - dt).max(f32::EPSILON);
            }
            // A charge counts up between snapshots, never past its vent:
            // the room's snapshot is what ends it (`TankState::charge`).
            if let Some(charge) = tank.charge.as_mut()
                && let Some(rule) = charge.weapon.charge_rule()
            {
                charge.held = (charge.held + dt).min(rule.vent);
            }
            tank.spin = (tank.spin - dt).max(0.0);
        }
        // An offline tower and the dark lamp posts too, until the room's
        // flag and `RoundState::lamps_out` end them.
        for tower in self.towers.values_mut() {
            if tower.disabled > 0.0 {
                tower.disabled = (tower.disabled - dt).max(f32::EPSILON);
            }
        }
        if self.lamps_out > 0.0 {
            self.lamps_out = (self.lamps_out - dt).max(f32::EPSILON);
        }
        // A drone's rotors and lamp turn on the replica's own clock.
        for drone in self.world.query_mut::<&mut crate::fpv::Drone>() {
            drone.age_by(dt);
        }
        // A stunned frog stays stunned until the room says it is not.
        for frog in self.world.query_mut::<&mut Frog>() {
            if frog.stun_timer > 0.0 {
                frog.stun_timer = (frog.stun_timer - dt).max(f32::EPSILON);
            }
        }
        // The sonic waves run out over the picture, flattening the grass
        // they pass; flattened grass stands back up.
        self.grass_flat.retain(|_, left| {
            *left -= dt;
            *left > 0.0
        });
        self.tick_sonic_pictures(dt);
        self.tick_emp_pictures(dt);
        // A drum that lands blasts where the server says it did, so the
        // landed ones are dropped here and the `Blast` event carries the
        // rest. A lava bomb the same.
        self.age_flying_drums(dt);
        self.age_lava_bombs(dt);
        // A grenade's fuse runs down on the replica's own clock, so its
        // lamp blinks smoothly between snapshots; the room's blast is what
        // ends it.
        for grenade in self.world.query_mut::<&mut Grenade>() {
            grenade.fuse = (grenade.fuse - dt).max(f32::EPSILON);
        }
        // The eruption's ring and flash come off the cycle's clock, which
        // stands on the room's tick.
        let eruption = self.eruption_show();
        self.show(eruption);
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

        // A training frog on its way to its next beat's cell keeps walking
        // rather than shy from the tank it is leading (`training.rs`).
        let walking = Some(frog_entity) == self.frog && self.frog_walking();
        // A frog under a rod's call hops out of it, away from its middle,
        // before it shies from any tank (docs/rod-from-god.md): a stunned
        // or penned one cannot.
        let t = tuning();
        if can_hop
            && !walking
            && let Some(zone) = self.zones.iter().find(|z| z.holds(frog_pos, &t))
        {
            let away = frog_pos - zone.centre;
            let away = if away.length() > 1.0 { away } else { Vec2::new(1.0, 0.0) };
            if let Some(new_pos) = frog_hop_target(&mut f.rng, frog_pos, away, hop_distance, &f.terrain, f.width, f.height) {
                with_frog_mut(&self.world, frog_entity, |fr| fr.start_hop(new_pos));
                return;
            }
        }
        if can_hop
            && !walking
            && let Some((_, tank_pos, dist, _)) = nearest_any
            && dist <= avoid_range
        {
            let away = Vec2::new(frog_pos.x - tank_pos.x, frog_pos.y - tank_pos.y);
            if let Some(new_pos) = frog_hop_target(&mut f.rng, frog_pos, away, hop_distance, &f.terrain, f.width, f.height) {
                with_frog_mut(&self.world, frog_entity, |fr| fr.start_hop(new_pos));
            }
        }
    }

    /// Pickups: any live tank whose hull box, grown by
    /// `pickup_collect_pad_px`, touches a pickup's square collects it
    /// (`Pickup::in_reach` - pure geometry, no physics), then the field is
    /// topped back up to the map's slot count after
    /// `pickup_respawn_seconds`.
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
        let pad = tuning().pickup_collect_pad_px;
        let living_tanks: Vec<(Entity, Position, Position)> = self
            .world
            .query::<(Entity, &Tank)>()
            .without::<&RollIn>()
            .iter()
            .filter(|(_, t)| !t.is_wreck())
            .map(|(e, t)| {
                let (center, half) = t.hull_bbox_world();
                (e, center, half)
            })
            .collect();
        // A frog pack is left where it is for a tank whose own frog is
        // alive and already at full health - the one rule deciding who
        // collects one (docs/frog-health-pack-prd.md). A side whose frog is
        // absent or dead still collects, and wastes it.
        let player_frog_full = self.frog.is_some_and(|e| with_frog(&self.world, e, Frog::at_full_health));
        let enemy_frog_full = self.enemy_frog.is_some_and(|e| with_frog(&self.world, e, Frog::at_full_health));
        let own_frog_full = |e: Entity| if self.is_player(e) { player_frog_full } else { enemy_frog_full };
        // The tower pack's rule, the frog pack's shape: left alone while
        // every standing tower of the collector's side is whole and not
        // burning (docs/defence-towers-prd.md section 9).
        let player_towers_want = self.tower_pack_wanted(Side::Player);
        let enemy_towers_want = self.tower_pack_wanted(Side::Enemy);
        let towers_want = |e: Entity| if self.is_player(e) { player_towers_want } else { enemy_towers_want };
        let collected: Vec<(Entity, Entity, PickupKind, Position, bool)> = self
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
                    .filter(|&&(e, _, _)| with_tank(&self.world, e, |t| t.wants_pickup(pickup.kind)))
                    .filter(|&&(e, _, _)| pickup.kind != PickupKind::FrogHealth || !own_frog_full(e))
                    .filter(|&&(e, _, _)| pickup.kind != PickupKind::TowerPack || towers_want(e))
                    .find(|&&(_, center, half)| pickup.in_reach(center, half, pad))
                    .map(|&(tank_entity, _, _)| (pickup_entity, tank_entity, pickup.kind, pickup.position, pickup.loose.is_some()))
            })
            .collect();
        for (pickup_entity, tank_entity, kind, at, spilled) in collected {
            let slot = {
                let mut q = self.world.query_one::<&mut Tank>(tank_entity);
                let tank = q.get().expect("collector entity always has a Tank");
                // A weapon pickup is the one special the tank carries
                // (`Tank::take_weapon`): another replaces it, the same one
                // refills. Health/Ammo/SpeedUp never touch it.
                match kind {
                    PickupKind::Health => tank.damage = (tank.damage - tuning().pickup_heal_amount).max(0.0),
                    PickupKind::Ammo => tank.shells_ammo += tuning().pickup_ammo_amount,
                    PickupKind::Laser => {
                        tank.take_weapon(ActiveWeapon::Laser);
                        // Rerolled per pickup so a fresh batch can swap the variant.
                        tank.laser_variant = if f.rng.random_range(0.0..1.0) < tuning().laser_blue_pickup_chance {
                            LaserVariant::Blue
                        } else {
                            LaserVariant::Red
                        };
                    }
                    PickupKind::Plasma => {
                        tank.take_weapon(ActiveWeapon::Plasma);
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
                        tank.raise_shield();
                    }
                    // Nothing on the tank: the frog pack heals a frog, and
                    // the frog can't be touched while this `&mut Tank`
                    // borrow is live - see just below.
                    PickupKind::FrogHealth => {}
                    // Nothing on the tank either: it repairs its side's
                    // towers, just below.
                    PickupKind::TowerPack => {}
                    // Refreshes rather than stacks, and puts out a hull
                    // already burning.
                    PickupKind::HeatShield => {
                        tank.heat_shield_timer = tuning().heat_shield_seconds;
                        tank.burn_timer = 0.0;
                    }
                    // Every other weapon crate - the minigun, the missiles,
                    // the flamethrower's fuel (in seconds), the grenades,
                    // the sonic hammer - is its weapon taken up, and no
                    // more (`PickupKind::weapon`).
                    other => {
                        if let Some(weapon) = other.weapon() {
                            tank.take_weapon(weapon);
                        }
                    }
                }
                tank.owner_slot()
            };
            if kind == PickupKind::TowerPack {
                let side = if self.is_player(tank_entity) { Side::Player } else { Side::Enemy };
                self.repair_towers(f, side);
            }
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
            f.events.push(Event::PickupCollected { slot, kind, x: at.x, y: at.y, spilled });
            self.world.despawn(pickup_entity).ok();
        }

        if slot_backed_count(&self.world, &self.map_pickup_slots) < self.map_pickup_slots.len() {
            self.pickup_respawn_timer -= f.dt;
            if self.pickup_respawn_timer <= 0.0 {
                let hurt = BonusGates { frog: self.frog_wants_a_pack(), towers: self.player_tower_hurt() };
                let respawned = respawn_from_slots(
                    &mut self.world,
                    &self.map,
                    &self.map_pickup_slots,
                    f.width,
                    f.height,
                    hurt,
                    Some(self.time),
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
                // A teleport ends a wind-up; `place_tank` ended any skid.
                tank.tell = None;
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

        // A charge whose stick is the weapon's - the rod's reticle
        // (`Stick::Aim`, docs/rod-from-god.md) - takes the stick from the
        // tick after the press: the hull stands, its facing kept.
        let aiming = tank.charge.and_then(|c| c.weapon.charge_rule()).is_some_and(|r| r.stick == crate::tank::Stick::Aim);
        let drive = if aiming { Intent { move_dir: None, face: None, ..intent } } else { intent };
        // A seat whose client owns the hull was put where it is by
        // `accept_seat_pose` before this tick; the stick only fires.
        if self.seat_owned[index] != self.frame {
            let footing = Footing::at(&self.water, &self.lava, &self.craters, self.weather, tank.position, self.time);
            drive_tank(&mut self.physics, tank, drive, f.dt, footing);
        }
        tick_queued_shots(&mut self.physics, f, tank, owner);

        // How the trigger fires is the weapon's (`ActiveWeapon::trigger`):
        // a laser, minigun or missile pod is full-auto while the key is
        // held (still paced by `fire_cooldown` - for the pod, its reload);
        // shells, plasma, grenades, the hammer and the EMP fire once per
        // physical press, so a held key can never re-arm them; the
        // flamethrower is a stream: every held frame emits, with no
        // cooldown between frames at all. A charge weapon's trigger is
        // stepped every tick, pressed or not - its release is the trigger
        // going up - and a charge whose weapon is gone is stepped once
        // more, to lapse it.
        let fire_pressed = intent.fire && !self.player_fire_held_last_frame[index];
        self.player_fire_held_last_frame[index] = intent.fire;
        let weapon = tank.active_weapon();
        if weapon.trigger() == Trigger::Charge || tank.charge.is_some() {
            let report = self.seat_hold_report(index);
            let steer = crate::rod::Steer { stick: if aiming { intent.move_dir } else { None }, aim: None, report: self.seat_reticle_report(index) };
            gauss::charge_trigger(f, entity, tank, owner, intent.fire, fire_pressed, report, steer);
        }
        let should_fire = match weapon.trigger() {
            Trigger::Auto | Trigger::Stream => intent.fire,
            Trigger::Press => fire_pressed,
            Trigger::Charge => false,
        };
        if weapon.trigger() == Trigger::Stream {
            if should_fire {
                dispatch_fire_from(&mut self.physics, f, tank, owner, 0.0, Some(entity));
            } else {
                tank.flame_held = false;
            }
        } else if should_fire && tank.fire_cooldown <= 0.0 {
            tank.flame_held = false;
            dispatch_fire(&mut self.physics, f, tank, owner, 0.0);
        }
        drop(q);
        self.lamp_key(f, index, entity, intent);
    }

    /// One seat's lamp key for one frame: on the press, a lantern set down
    /// behind the hull while it has any left.
    fn lamp_key(&mut self, f: &mut Frame, index: usize, entity: Entity, intent: Intent) {
        let mut q = self.world.query_one::<&mut Tank>(entity);
        let Ok(tank) = q.get() else { return };

        let lamp_pressed = intent.lamp && !self.player_lamp_held_last_frame[index];
        self.player_lamp_held_last_frame[index] = intent.lamp;
        if lamp_pressed && self.lamps_left[index] > 0 {
            let back = Dir::from_rotation(tank.rotation).map_or(Vec2::zero(), |d| d.vec());
            let behind = tank.hull_size() * 0.5 + 8.0;
            let at = Position::new(tank.position.x - back.x * behind, tank.position.y - back.y * behind);
            let inset = OBSTACLE_GRID_SIZE * 0.5;
            let at = Position::new(at.x.clamp(inset, f.width - inset), at.y.clamp(inset, f.height - inset));
            self.lamps_left[index] -= 1;
            let id = self.lantern_next_id;
            self.lantern_next_id = self.lantern_next_id.wrapping_add(1);
            self.lanterns.push(crate::lamp::Lantern { id, position: at, seat: index as u8, lit_at: self.time });
            f.events.push(Event::LanternSet { seat: index as u8, x: at.x, y: at.y });
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
            /// How far an enemy sees this seat (`Game::sight_on`): the
            /// sky's range, or the full one where it stands in the light.
            sight: f32,
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
                PlayerView { entity, pos, wreck, concealed: f.terrain.conceals(pos), entering: entering(entity), sight: self.sight_on(pos) }
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
                // A disabled tank's brain is off: it keeps the seat it had.
                if tank.is_disabled() {
                    continue;
                }
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

        // How far an enemy sees is the sky's say too (docs/weather.md).
        let view_range = self.enemy_sight();
        // The live seats standing on the field, which a field map's alerts,
        // calls and far test measure against (`field`); empty on an arena,
        // which reads none of them.
        let field = self.field_map;
        let live_seats: Vec<Position> =
            if field { players.iter().filter(|p| !p.wreck && !p.entering).map(|p| p.pos).collect() } else { Vec::new() };
        let alert = if field {
            // A field map has no shared alert: each enemy keeps its own,
            // passed only down a chain of neighbours (`field_alerts`).
            let seen: Vec<(Position, f32)> =
                players.iter().filter(|p| !p.concealed && !p.wreck && !p.entering).map(|p| (p.pos, p.sight)).collect();
            self.field_alerts(&seen, &live_seats, view_range, f);
            None
        } else {
            // Shared aggression: any enemy seeing a player refreshes the
            // group's last-known position (the nearest sighting when both
            // players are seen), so the rest converge instead of patrolling
            // blind - and, since concealment gates this too, a player who
            // stays hidden is genuinely *lost* once
            // `enemy_alert_hold_seconds` runs out, rather than merely
            // un-shootable.
            let alert_before = self.alert_position.filter(|_| self.alert_timer > 0.0);
            let mut seen: Option<(f32, Position)> = None;
            // A disabled enemy's senses are dead: it spots nobody.
            let blind: BTreeSet<usize> =
                self.world.query::<(Entity, &Tank)>().with::<&Ai>().iter().filter(|(_, t)| t.is_disabled()).map(|(e, _)| enemy_indices[&e]).collect();
            for p in players.iter().filter(|p| !p.concealed && !p.wreck && !p.entering) {
                for (i, m) in movers.iter().enumerate().skip(players.len()) {
                    if blind.contains(&i) {
                        continue;
                    }
                    let d = m.position.distance_to(p.pos);
                    if d <= p.sight && seen.is_none_or(|(best, _)| d < best) {
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
            alert
        };

        // Role targets: hunters fight the player's frog while it lives
        // (`quarry`), guards hold a leash around their own (`home`); both
        // are `None` once that frog is dead or absent, and the role then
        // behaves as `Role::Player`.
        let live_frog = |frog: Option<Entity>| {
            frog.and_then(|e| with_frog(&self.world, e, |fr| (!fr.is_dead()).then_some((e, fr.position))))
        };
        let quarry = live_frog(self.frog);
        let home = live_frog(self.enemy_frog).map(|(_, p)| p);
        // A hunter carrying a weapon its own rule fires (`ai::generic_fire`
        // false: the sonic hammer, which cannot hurt a frog) fights the
        // seat like everyone else until it is spent.
        let target_of = |ai: &Ai, tank: &Tank| match (ai.role, quarry) {
            (Role::Hunter, Some((frog, pos))) if crate::ai::generic_fire(tank.active_weapon()) => (pos, Some(frog)),
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
            let (target, hunting) = target_of(ai, tank);
            // A seat in the light is seen from further; a frog by the sky.
            let sight = if hunting.is_some() { view_range } else { players[(ai.target_player() as usize).min(players.len() - 1)].sight };
            let status = if guard_holds(ai) { EngageStatus::OutOfRange } else { engage_status(tank, ai, target, sight) };
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
        // A seat's ring keeps its firing slots inside the seat's sight box.
        let sight_box = Some(tuning().sight_box_half_px());
        // A seat a rod's call holds is herded (docs/rod-from-god.md
        // "Herding"): its ring stands round the call's middle, the firing
        // slots out past the call's danger.
        let herd_of = |pos: Position| -> (Position, Option<f32>) {
            let t = tuning();
            match self.zones.iter().find(|z| z.rod().is_some() && z.holds(pos, &t)) {
                Some(z) => (z.centre, Some(t.rod_ai_herd_px.max(z.danger_radius(&t) + t.enemy_danger_clear_px + crate::pyro::BLOCK))),
                None => (pos, None),
            }
        };
        if engaged[0].len() >= 2 {
            let (target_pos, herd) = herd_of(players[0].pos);
            self.engage[0].assign(
                &engaged[0],
                &EngageCtx {
                    target_pos,
                    width: f.width,
                    height: f.height,
                    margin,
                    sight_box,
                    herd,
                    reachable: &reachable,
                    line_of_sight: &line_of_sight,
                },
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
            let (target_pos, herd) = herd_of(players[seat].pos);
            self.engage[seat].assign(
                &engaged[seat],
                &EngageCtx {
                    target_pos,
                    width: f.width,
                    height: f.height,
                    margin,
                    sight_box,
                    herd,
                    reachable: &reachable,
                    line_of_sight: &line_of_sight,
                },
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
                &EngageCtx {
                    target_pos: frog_pos,
                    width: f.width,
                    height: f.height,
                    margin,
                    sight_box: None,
                    herd: None,
                    reachable: &reachable,
                    line_of_sight: &line_of_sight,
                },
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

        // Towers an enemy may hold a grudge against (`Ai::notify_tower_hit`).
        let standing_towers = if self.towers.is_empty() { Vec::new() } else { self.standing_towers() };

        // Field maps: what keeps a tank thinking every tick when it is near
        // - the live seats and the players' frog (`field::mind`).
        let anchors: Vec<Position> =
            if field { live_seats.iter().copied().chain(quarry.map(|(_, p)| p)).collect() } else { Vec::new() };
        let frame = self.frame;

        // What a hammer-carrying enemy would do with a blast each way it
        // could face (docs/sonic-hammer.md "AI"), measured before anyone
        // thinks; only when one carries a hammer, so a round without costs
        // nothing and every `Brain` is handed `SpecialSense::None`.
        let armed = self.world.query::<&Tank>().with::<&Ai>().iter().any(|t| t.active_weapon() == ActiveWeapon::SonicHammer);
        let hammer_senses = if armed {
            let seats: Vec<sonic::HammerSeat> = players
                .iter()
                .enumerate()
                .map(|(i, p)| with_tank(&self.world, p.entity, |t| sonic::HammerSeat::of(i as u8, t, !p.wreck && !p.entering, p.concealed)))
                .collect();
            let alerts: BTreeMap<Entity, Option<Position>> =
                self.world.query::<(Entity, &Ai)>().iter().map(|(e, ai)| (e, if field { ai.field.alert } else { alert })).collect();
            self.hammer_senses(f, &seats, &alerts, grid)
        } else {
            BTreeMap::new()
        };
        // What a pulse of each EMP-carrying enemy's own would get it, and
        // the places every enemy keeps out of because of an EMP
        // (docs/emp-burst.md "AI"): only when a tank on the field carries
        // one, so a round without hands every `Brain` none.
        let (emp_senses, mut dangers) = if self.any_emp() {
            let seats: Vec<emp::EmpSeat> = players
                .iter()
                .enumerate()
                .map(|(i, p)| with_tank(&self.world, p.entity, |t| emp::EmpSeat::of(i as u8, t, !p.wreck && !p.entering, p.concealed)))
                .collect();
            (self.emp_senses(&seats), self.emp_dangers(&seats))
        } else {
            (BTreeMap::new(), Vec::new())
        };
        // A hunter carrying a weapon its own rule fires fights the seat
        // rather than the frog (`target_of`), so on a field map it is woken
        // and leashed as any other tank is (`field::mind`).
        let field_hunting = |ai: &Ai, tank: &Tank| ai.role == Role::Hunter && quarry.is_some() && crate::ai::generic_fire(tank.active_weapon());
        // Whether a tank thinks this tick, as the collect pass below will
        // find: its brain is on and, on a field map, `field::mind` has it
        // think (`field::thinks`, the same rule with none of its
        // bookkeeping).
        let thinks = |tank: &Tank, ai: &Ai| {
            !tank.is_disabled()
                && (!field || field::thinks(ai, tank.position, tank.owner_slot(), &anchors, field_hunting(ai, tank), frame))
        };
        // What a slug of each rail-carrying enemy's own would go through
        // each way it could face, for those that think this tick, and the
        // lanes of every charging rail - places every enemy keeps out of
        // (docs/gauss-rail.md "AI"): only when a tank carries or charges
        // one.
        let gauss_senses = if self.world.query::<&Tank>().with::<&Ai>().iter().any(|t| t.active_weapon() == ActiveWeapon::GaussRail) {
            let seats: Vec<gauss::GaussSeat> = players
                .iter()
                .enumerate()
                .map(|(i, p)| gauss::GaussSeat { seat: i as u8, entity: p.entity, pos: p.pos, live: !p.wreck && !p.entering, concealed: p.concealed, sight: p.sight })
                .collect();
            self.gauss_senses(&f.terrain, &seats, thinks)
        } else {
            BTreeMap::new()
        };
        if self.any_rail_charging() {
            dangers.extend(self.rail_dangers());
        }
        // A rod's call: its circle, which every enemy keeps out of until it
        // lands (docs/rod-from-god.md "Reacting to a call").
        if !self.zones.is_empty() {
            dangers.extend(self.zone_dangers());
        }
        // What each rod-carrying enemy would call (docs/rod-from-god.md
        // "AI"): only when one carries or aims one.
        let rod_senses = if self.any_rod() {
            let seats: Vec<rod::RodSeat> = players
                .iter()
                .enumerate()
                .map(|(i, p)| rod::RodSeat { seat: i as u8, pos: p.pos, live: !p.wreck && !p.entering, concealed: p.concealed, sight: p.sight })
                .collect();
            self.rod_senses(&seats)
        } else {
            BTreeMap::new()
        };
        // What each drone-carrying enemy would launch at and from where,
        // and every enemy a seat's drone has locked (docs/fpv-swarm.md
        // "AI"): only when a tank carries the swarm, or a seat's drone is
        // in the air.
        let fpv_senses = if self.any_fpv() {
            let seats: Vec<fpv::FpvSeat> = players
                .iter()
                .enumerate()
                .map(|(i, p)| fpv::FpvSeat { seat: i as u8, entity: p.entity, pos: p.pos, live: !p.wreck && !p.entering, concealed: p.concealed, sight: p.sight })
                .collect();
            self.fpv_senses(f, &seats, grid, &dangers)
        } else {
            BTreeMap::new()
        };
        let air_threats = self.air_threats(grid);

        // --- collect pass: perception, `think`, aim and fire, exactly as
        // before. Only the impulse is deferred. ---
        let mut pending: Vec<Pending> = Vec::new();
        for (entity, tank, ai) in self.world.query::<(Entity, &mut Tank, &mut Ai)>().iter() {
            let my_index = enemy_indices[&entity];
            // An EMP has its brain off (docs/emp-burst.md): it coasts on
            // its last intent, the trigger released, and does not think -
            // no field map's choice either. The first tick it is back, it
            // reboots.
            if tank.is_disabled() {
                ai.down = true;
                let intent = Intent { fire: false, fire_aim_offset: 0.0, ..ai.last_intent() };
                pending.push(coast_enemy(&mut self.physics, f, entity, tank, intent));
                continue;
            }
            if ai.down {
                ai.reboot();
            }
            // A field map first decides whether this tank thinks this tick
            // (`field::mind`): a far one coasts on its last intent between
            // thinks, its trigger released, and one nothing has woken
            // holds still. A think then covers every tick since the last.
            // A tank that does not think this tick coasts (`coast_enemy`):
            // a branch that takes a tank's thinking away does it here,
            // before the field map's own choice.
            let think_dt = if field {
                match field::mind(ai, tank.position, tank.owner_slot(), &anchors, view_range, field_hunting(ai, tank), frame, f.dt) {
                    field::Mind::Think(dt) => dt,
                    idle => {
                        let intent = match idle {
                            field::Mind::Coast(intent) => intent,
                            _ => Intent::default(),
                        };
                        pending.push(coast_enemy(&mut self.physics, f, entity, tank, intent));
                        continue;
                    }
                }
            } else {
                f.dt
            };
            // On a field map the alert is the tank's own, and a wave tank
            // still called to the fight heads for the nearest live seat.
            let alert = if field {
                let call = if ai.field.called { field::call_target(&live_seats, tank.position) } else { None };
                call.or(ai.field.alert)
            } else {
                alert
            };
            let reach = tank.hull_size() * 0.5 + breach_reach_extra;
            let walls_ahead = Dir::ALL.map(|d| {
                f.terrain
                    .obstacle_ahead(tank.position, d.vec(), reach, breach_pad)
                    .map(|(material, burning)| WallAhead { material, burning })
            });
            let engage_target = self.last_engage.target(entity);
            if let Some(at) = ai.grudge_target() {
                let sight = standing_towers
                    .iter()
                    .find(|&&(p, _)| p == at)
                    .map(|&(_, tile)| f.terrain.line_of_sight_from(tile, tank.position, at));
                ai.set_grudge_sight(sight);
            }
            ai.air_threat = air_threats.get(&entity).copied();
            let (mut target, mut hunting) = target_of(ai, tank);
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
            let fighting_sight = fighting.sight;
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
            debug_assert!(
                tank.active_weapon() != ActiveWeapon::GaussRail || tank.is_wreck() || gauss_senses.contains_key(&entity),
                "a rail tank thinks on a tick `thinks` said it would not"
            );
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
                    think_dt,
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
                    fighting_sight,
                    &hammer_senses
                        .get(&entity)
                        .map(|s| SpecialSense::Hammer(*s))
                        .or_else(|| emp_senses.get(&entity).map(|s| SpecialSense::Emp(*s)))
                        .or_else(|| gauss_senses.get(&entity).map(|s| SpecialSense::Gauss(*s)))
                        .or_else(|| fpv_senses.get(&entity).map(|s| SpecialSense::Fpv(*s)))
                        .or_else(|| rod_senses.get(&entity).map(|s| SpecialSense::Rod(*s)))
                        .unwrap_or(SpecialSense::None),
                    &dangers,
                )
            });
            if let Some(before) = before {
                ai_transition_events(&mut f.events, tank.owner_slot(), &before, &ai.snapshot());
            }
            // Aim now, drive later. `tank.control` sets the hull rotation
            // a shot flies along, so it has to happen before the shot; the
            // impulse is the only part that waits for the commander. A
            // tank in a tell holds its aim whatever it decided.
            let intent = hold_for_tell(intent, tank.tell);
            tank.control(intent.move_dir, intent.face);
            let owner = tank.owner();
            tick_queued_shots(&mut self.physics, f, tank, owner);
            // The AI paces itself with its own fire timer; `fire_cooldown`
            // is the weapon's own minimum (a burst in progress, say). A
            // weapon with a tell winds up first (`enemy_trigger`).
            // What a drone launched now is to lock (`SpecialUse::Launch`).
            tank.fpv_want = ai.air_want;
            enemy_trigger(&mut self.physics, f, entity, tank, owner, intent);
            pending.push(Pending {
                entity,
                slot: tank.owner_slot(),
                intent,
                current,
                facing_before,
                disabled: false,
                charging: tank.charge.is_some(),
                clearing: ai.clearing(),
                dodging: ai.dodging(),
            });
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
                    busy: if p.disabled {
                        Some(command::Busy::Disabled)
                    } else if p.charging {
                        Some(command::Busy::Charging)
                    } else {
                        None
                    },
                    clearing: p.clearing,
                    dodging: p.dodging,
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
            with_tank_mut(&self.world, p.entity, |tank| {
                // The commander's order is never the last word on a tank
                // in a tell: it holds its aim (docs/sonic-hammer.md).
                let intent = commanded_intent(&self.commander, p.slot, p.intent, tank.tell);
                let footing = Footing::at(&self.water, &self.lava, &self.craters, self.weather, tank.position, self.time);
                drive_tank_with(&mut self.physics, tank, intent, f.dt, p.current, p.facing_before, footing);
            });
        }
    }

    /// Insert this frame's fired projectiles - only once no tank query is
    /// active, since hecs can't spawn into a world mid-iteration.
    ///
    /// A seat's shell, bolt or bullet takes its seat's rewind here
    /// (`seat_rewind`, carried as a `Rewind` beside it when it is not
    /// zero) and keeps it for its whole flight: the client that fired it
    /// goes on drawing the enemies that far in the past for as long as the
    /// shot flies.
    fn spawn_pending(&mut self, f: &mut Frame) {
        for mut shell in f.pending_shells.drain(..) {
            shell.set_id(self.take_shot_id());
            let rewind = self.seat_rewind(shell.owner);
            self.spawn_shot(shell, rewind);
        }
        for mut plasma in f.pending_plasmas.drain(..) {
            plasma.set_id(self.take_shot_id());
            let rewind = self.seat_rewind(plasma.owner);
            self.spawn_shot(plasma, rewind);
        }
        for mut bullet in f.pending_bullets.drain(..) {
            bullet.set_id(self.take_shot_id());
            let rewind = self.seat_rewind(bullet.owner);
            self.spawn_shot(bullet, rewind);
        }
        for mut missile in f.pending_missiles.drain(..) {
            missile.set_id(self.take_shot_id());
            self.world.spawn((missile,));
        }
        for mut grenade in f.pending_grenades.drain(..) {
            grenade.id = self.take_shot_id();
            self.world.spawn((grenade,));
        }
        self.launch_drones(f);
        self.place_calls(f);
    }

    /// Put one projectile in the world, with its `Rewind` only when it has
    /// one: a shot judged in the present spawns as a bare projectile. One
    /// fired inside a portal's swirl is leaving that portal
    /// (`portals::ShotPortals`).
    fn spawn_shot<P: Projectile>(&mut self, shot: P, rewind: u8) {
        let leaving = self.fired_inside_portal(shot.position());
        let entity = if rewind > 0 { self.world.spawn((shot, Rewind(rewind))) } else { self.world.spawn((shot,)) };
        if let Some(state) = leaving {
            self.world.insert_one(entity, state).ok();
        }
    }

    /// The next projectile id: one counter for shells, bullets and plasma,
    /// starting at 1 each round, never reused. No RNG.
    fn take_shot_id(&mut self) -> u32 {
        self.next_shot_id += 1;
        self.next_shot_id
    }

    /// Lasers have no travel time: each queued beam is swept over its whole
    /// length right now along the gun line, drawn from the laser's lens up
    /// to where it stopped, and applied.
    ///
    /// A seat's beam is swept against the enemies and frogs its client was
    /// drawing (`seat_rewind`, `rewound_boxes`), like its shells.
    ///
    /// A beam that reaches a portal before anything else goes in at the
    /// point of its line nearest the anchor and carries on out of another
    /// (`simulation::portals`, one RNG draw a pass) on the same heading for
    /// the reach it has left, judged leg by leg, up to
    /// `portal_shot_max_passes` passes. Each leg is drawn and logged as its
    /// own `LaserBeam`, a pass between two as a `ShotTeleported`.
    fn resolve_lasers(&mut self, f: &mut Frame) {
        let players = self.seats_on_field();
        let shots = std::mem::take(&mut f.pending_lasers);
        let reach = laser_reach(self.map.field_size());
        let (portal_radius, max_passes) = (tuning().portal_shot_radius, tuning().portal_shot_max_passes.max(0) as u8);
        for shot in shots {
            let rewind = self.seat_rewind(shot.owner);
            let full = laser_end(&shot, reach);
            let length = (full - shot.start).length().max(f32::EPSILON);
            let dir = (full - shot.start) * (1.0 / length);
            f.muzzle_flashes.push(Shockwave::new(shot.lens));
            // The leg being judged: from `from` to `end`, drawn from `drawn`.
            let (mut from, mut end, mut drawn, mut left) = (shot.start, full, shot.lens, length);
            // A beam from a tank standing on a portal leaves that portal.
            let mut leaving = portals::inside(from, self.shot_portals(), portal_radius);
            let mut leg = 0u8;
            loop {
                let entry = if leg < max_passes { portals::shot_entry(from, end, self.shot_portals(), portal_radius, leaving) } else { None };
                let stop = entry.map_or(end, |(_, at)| at);
                let past = self.rewound_boxes(rewind);
                let hit = f.terrain.sweep_rewound(&self.world, players, shot.owner, from, stop, laser_beam_half_width(), &[], past);
                let (hit_pos, target) = match hit {
                    Some((target, t)) => (from + (stop - from) * t, Some(target)),
                    None => (stop, None),
                };
                let through = target.is_none() && entry.is_some();
                self.laser_beams.push(LaserBeam::new(drawn, hit_pos, shot.variant));
                f.events.push(Event::LaserBeam {
                    x0: drawn.x,
                    y0: drawn.y,
                    x1: hit_pos.x,
                    y1: hit_pos.y,
                    variant: shot.variant.name(),
                    seat: crate::net::encode::owner_seat(shot.owner),
                    leg,
                    portal: through,
                });
                if let (true, Some((entrance, at))) = (through, entry) {
                    let exit = self.draw_shot_exit(f, entrance);
                    let out = portals::exit_point(self.portals[entrance], self.portals[exit], at, dir);
                    f.events.push(Event::ShotTeleported { id: None, x: at.x, y: at.y, to_x: out.x, to_y: out.y });
                    left -= (at - from).length();
                    (from, end, drawn) = (out, out + dir * left, out);
                    leaving = Some(exit);
                    leg += 1;
                    continue;
                }
                let Some(target) = target else { break };
                f.impact_flashes.push(Shockwave::new(hit_pos));
                // No knockback and no frog hop: an instant beam isn't
                // something to be shoved by or to dodge.
                self.apply_hit(f, target, hit_pos, laser_damage_range(&shot), HitEffects::none(HitCause::Laser), shot.owner);
                break;
            }
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
            self.advance_drones(PHYSICS_FIXED_DT, (f.width, f.height));
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
                        ram(e, p, &mut self.physics, &mut f.rng, &mut f.kills, &mut f.shoves, 1.0)
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
                    ram(a, b, &mut self.physics, &mut f.rng, &mut f.kills, &mut f.shoves, factor)
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
        let reach = TANK_FRAME_SIZE * 2.0;
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
                    ram(x, y, &mut self.physics, &mut f.rng, &mut f.kills, &mut f.shoves, 1.0)
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
    ///
    /// A shot whose segment passes through a portal before it meets
    /// anything goes in there and comes out of another
    /// (`simulation::portals`); the sweep is judged only up to the point it
    /// goes in.
    fn resolve_projectiles<P: Projectile>(&mut self, f: &mut Frame, live: bool) {
        let players = self.seats_on_field();
        struct Flight {
            entity: Entity,
            prev: Position,
            pos: Position,
            vel: Vec2,
            owner: Owner,
            dmg: (f32, f32),
            rewind: u8,
            portals: portals::ShotPortals,
        }
        let flying: Vec<Flight> = self
            .world
            .query::<(Entity, &P, Option<&Rewind>, Option<&portals::ShotPortals>)>()
            .iter()
            .filter(|(_, p, _, _)| p.is_flying())
            .map(|(entity, p, rewind, portals)| Flight {
                entity,
                prev: p.prev_position(),
                pos: p.position(),
                vel: p.velocity(),
                owner: p.owner(),
                dmg: p.damage_range(),
                rewind: rewind.map_or(0, |r| r.0),
                portals: portals.copied().unwrap_or_default(),
            })
            .collect();
        let (portal_radius, max_passes) = (tuning().portal_shot_radius, tuning().portal_shot_max_passes.max(0) as u8);
        // What is in the air for a shot that strikes it (`air.rs`): none on
        // the end screen, where a bullet passes a drone by.
        let mut air = if live && P::strikes_air() { self.air_targets() } else { Vec::new() };
        for Flight { entity, prev, pos, vel, owner, dmg, rewind, portals: through } in flying {
            // Into a portal on the way: judged only up to where it goes in.
            let entry = if through.passes < max_passes {
                portals::shot_entry(prev, pos, self.shot_portals(), portal_radius, through.leaving)
            } else {
                None
            };
            if entry.is_none() && through.leaving.is_some() {
                self.note_left_portal(entity, pos);
            }
            let pos = entry.map_or(pos, |(_, at)| at);
            // Lag compensation: a seat's shot meets the enemies and frogs
            // where its client drew them (`Rewind`); 0 is the present.
            let past = self.rewound_boxes(rewind);
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
                let swept = f.terrain.sweep_rewound(&self.world, players, owner, prev, pos, P::hit_half_extent(), &ignored, past);
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
            // An air target met first - or as the ground's hit is met, but
            // for a tank's - stops the shot there and counts a hit on it.
            let in_air = if air.is_empty() { None } else { f.terrain.sweep_air(&air, owner, prev, pos, P::hit_half_extent(), past) };
            let air_first = match (in_air, hit) {
                (Some((_, ta)), Some((ShellTarget::Tank(_), tg))) => ta < tg,
                (Some((_, ta)), Some((_, tg))) => ta <= tg,
                (Some(_), None) => true,
                (None, _) => false,
            };
            if let (true, Some((key, ta))) = (air_first, in_air) {
                let at = prev + (pos - prev) * ta;
                f.impact_flashes.push(Shockwave::new(at));
                {
                    let mut q = self.world.query_one::<&mut P>(entity);
                    let p = q.get().expect("projectile collected this frame still exists");
                    p.set_position(at);
                    p.detonate();
                }
                if self.strike_air(f, key, crate::air::AirStrike::Bullet) {
                    air.retain(|a| a.key != key);
                }
                continue;
            }
            let Some((target, t)) = hit else {
                if let Some((entrance, at)) = entry {
                    self.shot_through::<P>(f, entity, entrance, at, vel);
                }
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
                drop(q);
                // Turned back, the shot is the shield's and is judged in
                // the present.
                if let Ok(mut rewind) = self.world.get::<&mut Rewind>(entity) {
                    rewind.0 = 0;
                }
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
                cause: P::hit_cause(),
            };
            self.apply_hit(f, target, hit_pos, dmg, effects, owner);
        }
    }

    /// Everything a dying tank throws off: its show (`wreck_show`) and a
    /// set of delayed pops.
    ///
    /// Draws no RNG: the parts' landing spots, cells and arcs all come out
    /// of `blast::seed_at` salted per piece, so a spectacular death cannot
    /// shift a seeded replay.
    fn wreck_fx(&mut self, f: &mut Frame, center: Position) {
        let mut show = Spectacle::default();
        self.wreck_show(&mut show, center);
        f.stage(show);

        // Ammo cooking off: a few small pops after the fact, spread over
        // `cookoff_window_seconds`. Queued rather than fired now, and
        // ticked by `tick_cookoffs`; each pop is an `Event::CookOff`, which
        // is how a replica gets it.
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

    /// The show a tank dying at `center` puts on: the kill's shockwave,
    /// the fireball (a mushroom cloud in `wreck_mushroom_chance` of kills,
    /// hashed), the impact flash, the screen flash, a scorch on dry
    /// ground, its last tread marks burnt in and its hull parts thrown.
    /// Everything but the damage and the cook-offs, which is why a replica
    /// can call it off `Event::Wreck`. No RNG.
    pub(crate) fn wreck_show(&mut self, show: &mut Spectacle, center: Position) {
        show.shocks.push(Shockwave::scaled(center, SHOCK_KILL));
        show.blast_fx.push(BlastFx::wreck(center));
        show.impact_flashes.push(Shockwave::new(center));
        self.flash_screen();
        if self.water.depth_at(center) == crate::ground::Depth::Dry {
            show.scorches.push(Scorch::new(center));
        }
        self.scorch_tracks(center);
        // The kill's pressure wave lays the grass round the hull flat, as a
        // blast's does (`blast_show`); it stands back up on the grass's own
        // clock.
        crate::grass::flatten(&mut self.grass, center, tuning().wreck_part_throw_px * tuning().blast_grass_flatten);

        let throw = tuning().wreck_part_throw_px;
        for i in 0..tuning().wreck_parts.max(0) as u32 {
            // Fan the pieces around the hull by hashed angle and distance
            // rather than a fixed rosette, so two wrecks never scatter the
            // same way.
            let h = crate::blast::seed_at(center, 40 + i * 5);
            let angle = (h % 3600) as f32 / 3600.0 * std::f32::consts::TAU;
            let dist = throw * (0.35 + 0.65 * ((h >> 12) % 100) as f32 / 100.0);
            let to = Position::new(center.x + angle.cos() * dist, center.y + angle.sin() * dist);
            show.decals.push(Decal::thrown(RUBBLE_ROW_TANK, center, to, 40 + i * 5));
        }
    }

    /// Burn a wreck's last few tread marks into the ground. Walks back
    /// from the newest mark rather than scanning the whole list, and stops
    /// after `wreck_track_marks`, so the cost is bounded by the number of
    /// marks burnt and not by how long the round has been running.
    fn scorch_tracks(&mut self, center: Position) {
        let reach = crate::TANK_FRAME_SIZE;
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
        self.flash_screen_with(1.0);
    }

    /// `flash_screen` at `strength` times a drum's (its peak and its
    /// length, `screen_flash_strength`): a flash stronger than a drum's -
    /// a rod's impact - is not held back by the gap, and none weaker
    /// replaces a stronger one still fading.
    pub(crate) fn flash_screen_with(&mut self, strength: f32) {
        if self.screen_flash_cooldown > 0.0 && strength <= 1.0 {
            return;
        }
        if self.screen_flash.is_some() && strength < self.screen_flash_strength {
            return;
        }
        self.screen_flash = Some(0.0);
        self.screen_flash_strength = strength;
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
            let mut show = Spectacle::default();
            Self::cookoff_show(&mut show, center);
            f.stage(show);
            f.events.push(Event::CookOff { x: center.x, y: center.y });
        }
    }

    /// One cook-off pop at `center`: a small fireball and a faint ripple,
    /// nothing screen-level. A replica calls it off `Event::CookOff`.
    pub(crate) fn cookoff_show(show: &mut Spectacle, center: Position) {
        show.blast_fx.push(BlastFx::small(center));
        show.shocks.push(Shockwave::scaled(center, SHOCK_COOKOFF));
    }

    /// Every tank killed this frame gets a shockwave and an explosion, and
    /// every barrel detonated this frame its blast; a splash that kills
    /// another tank, or a blast that finishes a barrel's fuse elsewhere, is
    /// appended and handled in turn. Processed in order, so the last ring
    /// shown is the most recent one's. Terminates: a tank can only ever be
    /// pushed once (every push is gated by its own transition into a wreck)
    /// and a barrel dies once. `live` is false on the end screen, where
    /// blasts play out without damage (no kills happen there).
    fn explosions(&mut self, f: &mut Frame, live: bool) {
        let (mut i, mut j, mut k) = (0, 0, 0);
        while i < f.kills.len() || j < f.pending_blasts.len() || k < f.crate_breaks.len() {
            while i < f.kills.len() {
                let (center, victim) = f.kills[i];
                i += 1;
                self.credit_wreck(victim);
                f.events.push(Event::Wreck { slot: victim.slot(), x: center.x, y: center.y });
                self.wreck_fx(f, center);
                self.apply_explosion(f, center, victim);
            }
            if j < f.pending_blasts.len() {
                let blast = f.pending_blasts[j];
                j += 1;
                self.apply_blast(f, blast, live);
            }
            if k < f.crate_breaks.len() {
                let crate_entity = f.crate_breaks[k];
                k += 1;
                self.break_crate(f, crate_entity, live);
            }
        }
    }

    /// Count a wreck for `round_stats`: an enemy's goes to the round's
    /// total, and to the seat that last damaged it, if any did. A tank is
    /// wrecked once (see `explosions`), so it is counted once.
    fn credit_wreck(&mut self, victim: Owner) {
        if victim.is_player() {
            return;
        }
        self.enemies_destroyed += 1;
        let by = self.world.query::<&Tank>().iter().find(|t| t.owner() == victim).and_then(|t| t.last_hit_by);
        if let Some(count) = by.and_then(|seat| self.wrecks_by_seat.get_mut(seat as usize)) {
            *count += 1;
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
        // A training round is won by its last beat and never lost: a
        // wrecked seat and a fallen frog come back (`training_phase`).
        if let Some(run) = &self.training {
            if run.finished() {
                self.end_round(f, Outcome::Won);
            }
            return;
        }
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
        // Nothing charges or fires on the end screen: every charge is lost.
        for tank in self.world.query_mut::<&mut Tank>() {
            tank.lapse_charge();
        }
        self.ended_at = Some(self.time);
        self.restart_timer = self.end_beats().total();
        f.events.push(Event::RoundEnded { outcome });
    }

    /// How this round's end plays out once it is decided: the finale with
    /// nothing over the world, the verdict easing in, then the countdown -
    /// `level_loss_retry_seconds` on a lost level (a held end screen),
    /// `restart_delay` everywhere else.
    pub fn end_beats(&self) -> EndBeats {
        let t = tuning();
        let countdown =
            if self.hold_end_screen && self.outcome == Outcome::Lost { t.level_loss_retry_seconds } else { t.restart_delay };
        EndBeats { finale: t.round_finale_seconds, fade: t.round_verdict_fade_seconds, countdown }
    }

    /// Seconds since the round was decided, read off the end screen's own
    /// clock (`restart_timer`, which a replica is sent), so a room's
    /// clients play the end's beats on the room's clock; `None` while the
    /// round plays.
    pub fn since_end(&self) -> Option<f32> {
        (self.outcome != Outcome::Playing).then(|| (self.end_beats().total() - self.restart_timer).max(0.0))
    }

    /// Whether the end screen's finale is still playing: the round is
    /// decided and nothing is over the world yet.
    pub fn in_finale(&self) -> bool {
        self.since_end().is_some_and(|s| s < self.end_beats().finale)
    }

    /// Cut the finale short: the verdict starts easing in now.
    pub fn skip_finale(&mut self) {
        if self.in_finale() {
            let beats = self.end_beats();
            self.restart_timer = beats.fade + beats.countdown;
        }
    }

    /// Skip the finale and the verdict's fade: the end screen stands whole
    /// now, its countdown at the start.
    pub fn skip_to_verdict(&mut self) {
        if self.outcome != Outcome::Playing {
            self.restart_timer = self.restart_timer.min(self.end_beats().countdown);
        }
    }

    /// Put the end screen's clock at `left` seconds - fewer where it shows
    /// fewer, `left` again where it already stands at zero, waiting: a
    /// press that takes the screen's way sooner, behind a whole fade out.
    pub fn hurry_end(&mut self, left: f32) {
        if self.outcome != Outcome::Playing {
            let left = left.max(0.0);
            self.restart_timer = if self.restart_timer > 0.0 { self.restart_timer.min(left) } else { left };
        }
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

    /// The map cell nearest `near` a tank can be put down on in this
    /// round's terrain (`battlefield::drop_cell` over `grid`, this round's
    /// `nav_grid`): a worst-case tank's box clear of every tile and deep
    /// cell, out of deep water, off every active portal's trigger - a hull
    /// put down on one would be sent through at once - and passing `ok`.
    /// The builder's PLAY HERE spot and the linter's quick fixes ask it.
    /// Reads the round, draws no RNG.
    pub fn drop_cell(&self, grid: &Grid, near: Position, ok: impl Fn((i32, i32), Position) -> bool) -> Option<(i32, i32)> {
        let (width, height) = self.map.field_size();
        let mut obstacles: Vec<Position> = self.world.query::<&Obstacle>().iter().map(|o| o.position).collect();
        obstacles.extend(self.water.deep_cells());
        obstacles.extend(self.lava.cells().map(|(c, r)| map::cell_to_world(c, r)));
        let trigger = tuning().portal_trigger_radius;
        let portals = self.active_portals();
        battlefield::drop_cell(width, height, grid, &obstacles, near, |cell, p| {
            self.water.depth_at(p) != crate::ground::Depth::Deep
                && self.lava.depth_at(p) == crate::ground::Depth::Dry
                && portals.iter().all(|&q| q.distance_to(p) > trigger)
                && ok(cell, p)
        })
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

    /// The round's time and wreck count so far - the end screen's
    /// numbers once `outcome` is decided.
    pub fn round_stats(&self) -> RoundStats {
        let enemies = match self.spawn_plan {
            SpawnPlan::Band { .. } => self.band_enemy_count as u32,
            SpawnPlan::Waves { waves, .. } => (0..waves).map(|i| self.spawn_plan.wave_size(i)).sum(),
        };
        RoundStats {
            seconds: self.ended_at.unwrap_or(self.time),
            destroyed: self.enemies_destroyed,
            enemies,
            by_seat: self.wrecks_by_seat,
        }
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

    /// Every live hull knocked off its tracks (`Tank::skid`): its owner
    /// slot, position and speed (px/s). What `fx.rs` scrapes dust from.
    pub fn skidding(&self) -> Vec<(usize, Position, f32)> {
        let mut out: Vec<(usize, Position, f32)> = self
            .world
            .query::<&Tank>()
            .iter()
            .filter(|t| t.skid > 0.0 && !t.is_wreck())
            .map(|t| (t.owner_slot(), t.position, t.body.map_or(t.skid_speed, |b| self.physics.velocity(b).length())))
            .collect();
        out.sort_by_key(|s| s.0);
        out
    }

    /// Every live hull in a ford this frame: its owner slot, position and
    /// speed (px/s). What `fx.rs` throws spray from.
    pub fn wading(&self) -> Vec<(usize, Position, f32)> {
        let mut out: Vec<(usize, Position, f32)> = self
            .world
            .query::<&Tank>()
            .iter()
            .filter(|t| !t.is_wreck() && self.water.depth_at(t.position).is_wet())
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

    /// Where the players' frog stands while it lives - for tooling (the
    /// probe's `defend` scenario guards it like a seat).
    pub fn frog_position(&self) -> Option<Position> {
        self.frog.and_then(|e| with_frog(&self.world, e, |fr| (!fr.is_dead()).then_some(fr.position)))
    }

    pub fn tank_snapshots(&self) -> Vec<TankSnapshot> {
        // A hunter with the players' frog to hunt is on its way to a fight
        // whatever its alert says (`TankSnapshot::leashed`).
        let quarry = self.frog.is_some_and(|e| self.world.get::<&Frog>(e).is_ok_and(|fr| !fr.is_dead()));
        self.world
            .query::<(Entity, &Tank, Option<&Ai>)>()
            .iter()
            .map(|(entity, tank, ai)| {
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
                    grenade_ammo: tank.grenade_ammo,
                    sonic_ammo: tank.sonic_ammo,
                    emp_charges: tank.emp_charges,
                    gauss_slugs: tank.gauss_slugs,
                    fpv_drones: tank.fpv_drones,
                    fpv_out: tank.fpv_out,
                    rods: tank.rods,
                    charging: tank.charge.is_some(),
                    disabled: tank.is_disabled(),
                    kept_out: ai.is_some_and(Ai::kept_out),
                    air_hold: ai.is_some_and(Ai::air_hold),
                    rod_hold: ai.is_some_and(Ai::rod_hold),
                    tell: tank.tell.is_some(),
                    skidding: tank.skid > 0.0,
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
                    shot_at_seat: ai.and_then(Ai::shot_at_seat),
                    hit_by_seat: tank.hit_by_seat,
                    asleep: self.field_map && ai.is_some_and(|ai| !ai.field.awake),
                    leashed: self.field_map
                        && ai.is_some_and(|ai| {
                            ai.field.alert.is_none()
                                && !ai.field.called
                                && !ai.is_hit_alerted()
                                && !(ai.role == Role::Hunter && quarry && crate::ai::generic_fire(tank.active_weapon()))
                        }),
                    guarding: ai.is_some_and(Ai::holds_beat),
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
    pub grenade_ammo: i32,
    pub sonic_ammo: i32,
    pub emp_charges: i32,
    pub gauss_slugs: i32,
    /// Drones left in its FPV halo, and its drones in the air.
    pub fpv_drones: i32,
    pub fpv_out: u8,
    /// Rods left to call (`Tank::rods`).
    pub rods: i32,
    /// Holding a charge on its trigger (`Tank::charge`, a gauss rail or a
    /// rod's reticle): crawling or standing on purpose.
    pub charging: bool,
    /// Disabled by an EMP (`Tank::disabled`): an enemy coasting with its
    /// brain off, going where it did not ask to.
    pub disabled: bool,
    /// Waiting outside a danger it is kept out of (`Ai::kept_out`):
    /// holding still on purpose.
    pub kept_out: bool,
    /// Standing for the FPV swarm (`Ai::air_hold`): watching its own drone
    /// work, or under a crown while a seat's drone comes at it.
    pub air_hold: bool,
    /// Keeping its distance from a seat with the rod from god
    /// (`Ai::rod_hold`): out of the circle a call on that seat would crush,
    /// on purpose.
    pub rod_hold: bool,
    /// Winding up a special (`Tank::tell`): holding still on purpose.
    pub tell: bool,
    /// Knocked off its tracks (`Tank::skid`): sliding where it did not ask
    /// to go.
    pub skidding: bool,
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
    /// The seat this enemy's last decision to fire was aimed at
    /// (`Ai::shot_at_seat`): the attack on the seat it fights, or a
    /// hunter's snipe. `None` for a seat's own tank, and for an enemy
    /// that fired at a frog, a tower or a tile or did not fire. Paired
    /// with that frame's `Event::Fired`, it is what the probe's
    /// `offbox-fire` check reads.
    pub shot_at_seat: Option<u8>,
    /// The seat that damaged this tank this frame, if one did
    /// (`Tank::hit_by_seat`).
    pub hit_by_seat: Option<u8>,
    /// A field map's enemy nothing has woken yet (`simulation::field`): it
    /// holds still and thinks nothing, by design. Always false on an arena.
    pub asleep: bool,
    /// A field map's enemy with nothing calling it to the fight - no
    /// alert, no call, no recent hit, no frog it hunts - so it keeps to
    /// its home leash rather than heading for a seat. Always false on an
    /// arena.
    pub leashed: bool,
    /// A guard keeping its beat by its frog while the seat it would fight
    /// is far from it (`Role::Guard`, `Ai::holds_beat`): staying put, by
    /// design.
    pub guarding: bool,
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
    /// Its brain is off (an EMP): the commander cannot reach it.
    disabled: bool,
    /// It holds a rail's charge (docs/gauss-rail.md): the commander leaves
    /// it on its lane.
    charging: bool,
    /// The ring its EMP rule asked the commander to clear (`Ai::clearing`).
    clearing: Option<f32>,
    /// Backing out of a danger on its own (`Ai::dodging`).
    dodging: bool,
}

/// What the ground and the sky do to a hull's drive this frame
/// (docs/water.md, docs/weather.md "The rules"): dry ground under a clear
/// sky leaves everything at 1 and the air and the water still.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Footing {
    /// Fraction of the commanded top speed and of `tank_accel_force` the
    /// hull gets.
    pace: f32,
    /// Fraction of `tank_turn_grip_force` the hull keeps.
    grip: f32,
    /// Fraction of `tank_accel_force` its tracks get on top of `pace`:
    /// less than 1 where they spin (ice).
    traction: f32,
    /// Fraction of `tank_decel_curve_rate` it brakes with: less than 1
    /// where it coasts (ice).
    brake: f32,
    /// The frame the hull drives relative to - the water's current, a
    /// sandstorm's gust - so a hull that stops in it drifts with it.
    flow: Position,
    /// In water at all - drops a speed boost and wets the tracks.
    wading: bool,
}

impl Footing {
    pub(crate) const DRY: Footing =
        Footing { pace: 1.0, grip: 1.0, traction: 1.0, brake: 1.0, flow: Position::new(0.0, 0.0), wading: false };

    /// The footing at `pos` at round time `time` under the round's `sky`:
    /// the water's (a ford slows and loosens, ice slides), the lava's (a
    /// burning ford slows and loosens too, with no current and no wet
    /// tracks), a rod's crater's (a dry pit slows), and then the sky's on
    /// top (wet ground loosens every hull, a gust carries it).
    pub(crate) fn at(water: &crate::ground::WaterLayout, lava: &crate::lava::LavaLayout, craters: &crate::rod::Craters, sky: crate::map::Weather, pos: Position, time: f32) -> Footing {
        let t = tuning();
        let mut footing = match water.depth_at(pos) {
            // A dry crater's pit (docs/rod-from-god.md "The crater"); a
            // filled one is a ford, read above as one.
            crate::ground::Depth::Dry if craters.under(pos) => Footing { pace: t.rod_crater_pace, ..Footing::DRY },
            crate::ground::Depth::Dry => Footing::DRY,
            crate::ground::Depth::Ice => Footing {
                grip: t.ice_grip_factor,
                traction: t.ice_traction_factor,
                brake: t.ice_brake_factor,
                ..Footing::DRY
            },
            crate::ground::Depth::Shallow | crate::ground::Depth::Deep => {
                let flow = if water.pushes_south(pos) { t.water_current_speed } else { 0.0 };
                Footing { pace: t.water_speed_factor, grip: t.water_grip_factor, flow: Position::new(0.0, flow), wading: true, ..Footing::DRY }
            }
        };
        if lava.depth_at(pos) != crate::ground::Depth::Dry {
            footing.pace *= t.lava_speed_factor;
            footing.grip *= t.lava_grip_factor;
        }
        let wet = crate::weather::grip_factor(sky, &t);
        if wet != 1.0 {
            footing.grip *= wet;
        }
        let wind = crate::weather::gust_at(sky, pos, time, &t);
        if wind.x != 0.0 || wind.y != 0.0 {
            footing.flow = Position::new(footing.flow.x + wind.x, footing.flow.y + wind.y);
        }
        footing
    }
}

/// One enemy that does not think this tick: a field map's far tank on its
/// last intent, one nothing has woken on none, one an EMP disabled on its
/// last intent (docs/emp-burst.md). It holds a tell's aim, ticks
/// its queued shots and its tell, its trigger released, and is handed back
/// as the `Pending` the apply pass drives.
fn coast_enemy(physics: &mut Physics, f: &mut Frame, entity: Entity, tank: &mut Tank, intent: Intent) -> Pending {
    let intent = hold_for_tell(intent, tank.tell);
    let handle = tank.body.expect("a live enemy always has a body here");
    let current = physics.velocity(handle);
    let facing_before = tank.rotation;
    tank.control(intent.move_dir, intent.face);
    let owner = tank.owner();
    tick_queued_shots(physics, f, tank, owner);
    // The trigger released - but held while a charge runs, so a coast never
    // lets one go: a far tank's charge vents at worst.
    let fire = tank.charge.is_some();
    enemy_trigger(physics, f, entity, tank, owner, Intent { fire, ..intent });
    Pending {
        entity,
        slot: tank.owner_slot(),
        intent,
        current,
        facing_before,
        disabled: tank.is_disabled(),
        charging: tank.charge.is_some(),
        clearing: None,
        dodging: false,
    }
}

/// The intent the apply pass drives `slot` by: the commander's orders over
/// the collect pass's `intent`, then a tell's hold over both - the last
/// word on a tank winding up is its tell's (docs/sonic-hammer.md).
fn commanded_intent(commander: &command::Commander, slot: usize, intent: Intent, tell: Option<crate::tank::Tell>) -> Intent {
    hold_for_tell(commander.apply(slot, intent), tell)
}

/// `intent` for a tank in a tell (docs/sonic-hammer.md "The enemy tell"):
/// no movement, no new trigger, facing the way the tell goes off. The
/// intent as it is for a tank with none.
fn hold_for_tell(intent: Intent, tell: Option<crate::tank::Tell>) -> Intent {
    match tell {
        Some(tell) => Intent { move_dir: None, face: Some(tell.facing), fire: false, ..intent },
        None => intent,
    }
}

/// An enemy's trigger for this frame: a charge weapon's stepped
/// (`gauss::charge_trigger`, its press edge from `Tank::trigger_held`); a
/// tell running is counted down and, at its end, the weapon fires along the
/// facing it held (if the tank is whole, still carries it and its cooldown
/// is out; otherwise the tell lapses); else a pull with the cooldown out
/// starts the weapon's tell (`ActiveWeapon::tell_seconds`,
/// `Event::TellStarted`) or, for a weapon with none, fires it as ever.
fn enemy_trigger(physics: &mut Physics, f: &mut Frame, entity: Entity, tank: &mut Tank, owner: Owner, intent: Intent) {
    let pressed = intent.fire && !tank.trigger_held;
    tank.trigger_held = intent.fire;
    // A charge the AI lets go of without firing (`SpecialUse::Drop`): a
    // rod's target gone, a call standing over it.
    if intent.drop_charge
        && let Some(charge) = tank.charge
        && tank.lapse_charge()
    {
        f.events.push(Event::ChargeEnded { slot: tank.owner_slot(), weapon: charge.weapon.name(), end: crate::tank::ChargeEnd::Lapsed });
        return;
    }
    // A charge weapon's trigger is stepped every tick (its release is the
    // trigger going up), and a charge whose weapon is gone once more, to
    // lapse it (docs/gauss-rail.md).
    if tank.active_weapon().trigger() == Trigger::Charge || tank.charge.is_some() {
        let steer = crate::rod::Steer { aim: intent.aim_cell, ..crate::rod::Steer::default() };
        gauss::charge_trigger(f, entity, tank, owner, intent.fire, pressed, None, steer);
        return;
    }
    if let Some(mut tell) = tank.tell {
        tell.left -= f.dt;
        if tell.left > 0.0 {
            tank.tell = Some(tell);
            return;
        }
        tank.tell = None;
        if !tank.is_wreck() && tank.active_weapon() == tell.weapon && tank.fire_cooldown <= 0.0 {
            dispatch_fire(physics, f, tank, owner, 0.0);
        }
        return;
    }
    if !intent.fire || tank.fire_cooldown > 0.0 {
        return;
    }
    let weapon = tank.active_weapon();
    match weapon.tell_seconds() {
        Some(total) => {
            let facing = Dir::from_rotation(tank.rotation).unwrap_or(Dir::Up);
            tank.tell = Some(crate::tank::Tell { weapon, left: total, total, facing });
            f.events.push(Event::TellStarted { slot: tank.owner_slot(), weapon: weapon.name() });
        }
        None => dispatch_fire(physics, f, tank, owner, intent.fire_aim_offset),
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
    let target = tank.velocity;

    if tank.rotation != facing_before {
        physics.resize_collider(physics.collider_of(handle), tank.move_half_extents(tank.facing_along_x()));
    }

    // Knocked off its tracks (`Tank::skid`, docs/sonic-hammer.md): the
    // drive does nothing - the stick still turns the hull - and its motion
    // against the ground's flow falls by the skid's friction whichever way
    // it faces, so a knock slides a hull as far head-on as broadside. All
    // but still, it drives again at once.
    if tank.skid > 0.0 {
        let t = tuning();
        let speed = current.length();
        if speed < t.tank_decel_snap_px {
            tank.skid = 0.0;
        } else {
            let fall = (crate::sonic::skid_friction(&t, footing.grip) * dt).min(speed);
            let delta = current * (-fall / speed);
            physics.apply_impulse(handle, Position::new(delta.x * tank.mass(), delta.y * tank.mass()));
            return;
        }
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
    // Ooze slows a hull like a ford does (docs/defence-towers-prd.md
    // section 6): the top speed and the drive together.
    let pace = footing.pace * tank.slime_pace();
    // A charging rail crawls (docs/gauss-rail.md): a lower top speed, as
    // `intent.slow` is, not a lazier drive - and the throttle says so, so
    // what was asked of the hull reads as asked. Exactly 1 with no charge.
    let crawl = tank.charge_pace();
    let scale = intent.speed_scale() * crawl * pace;
    tank.throttle = intent.speed_scale() * crawl;
    let (current_on, target_on, current_off) = if along_x {
        (current.x, target.x * scale, current.y)
    } else {
        (current.y, target.y * scale, current.x)
    };

    let want_on = target_on - current_on;
    let speeding_up = want_on * current_on >= 0.0;
    let delta_on = if speeding_up {
        let max_on = tuning().tank_accel_force * pace * footing.traction * tank.speed_factor() / tank.mass() * dt;
        want_on.clamp(-max_on, max_on)
    } else {
        // Close a rate-controlled fraction of the remaining gap each frame
        // (frame-rate independent); snap the last sliver below
        // TANK_DECEL_SNAP_PX rather than trailing the asymptote forever.
        let rate = tuning().tank_decel_curve_rate * tank.speed_factor() / tank.mass() * footing.brake;
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

/// `cell` unless it is deep water or lava, else the nearest map cell
/// (square rings outward, the map's own `nearest_free_cell` order) that is
/// neither solid (a volcano's cone included) nor deep nor lava. A `start` cell can never be water itself (a cell holds
/// one object), so this only matters for the centre fallback of a map
/// without one: a lake over the middle of the field spawns the player on
/// its shore rather than inside the lake's colliders.
fn dry_cell_near(map: &MapFile, water: &crate::ground::WaterLayout, lava: &crate::lava::LavaLayout, cell: (i32, i32)) -> (i32, i32) {
    // Lava of any depth counts: a start on a ford would burn the round's
    // first seconds away.
    let deep = |c: i32, r: i32| {
        let at = map::cell_to_world(c, r);
        water.depth_at(at) == crate::ground::Depth::Deep || lava.depth_at(at) != crate::ground::Depth::Dry
    };
    let solid = |c: i32, r: i32| map.solid_at(c, r);
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
/// is deliberate and not rejection-sampled. `dropped_at` is the round
/// clock a crate comes down at (its air drop, drawn only), `None` for one
/// that stands there from the round's start.
fn spawn_pickup_at(world: &mut hecs::World, pos: Position, kind: PickupKind, dropped_at: Option<f32>) {
    world.spawn((Pickup::dropped(kind, pos, dropped_at),));
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

/// The conditional bonuses a Health slot may roll, each checked before
/// any RNG is drawn for it: the frog pack while the frog is hurt, the
/// tower pack while a player tower is.
#[derive(Clone, Copy, Default)]
struct BonusGates {
    frog: bool,
    towers: bool,
}

/// Top up one pickup at a uniformly random slot not currently occupied,
/// with the health slot's bonus rolls if that slot is a health pack
/// (`hurt` gates the conditional ones). A no-op if every slot is full.
fn respawn_from_slots(
    world: &mut hecs::World,
    map: &MapFile,
    slots: &[(Position, PickupKind)],
    width: f32,
    height: f32,
    hurt: BonusGates,
    dropped_at: Option<f32>,
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
    spawn_pickup_at(world, pos, kind, dropped_at);
    if kind == PickupKind::Health {
        maybe_spawn_health_slot_bonuses(world, map, pos, width, height, hurt, dropped_at, rng);
    }
    Some((pos, kind))
}

/// The un-slotted bonuses that ride along with a Health slot just
/// (re)spawned at `slot`: the rainbow shield always, a frog health pack
/// only while `hurt.frog`, a tower pack only while `hurt.towers`.
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
    hurt: BonusGates,
    dropped_at: Option<f32>,
    rng: &mut SmallRng,
) {
    maybe_spawn_bonus(world, map, slot, PickupKind::Shield, tuning().shield_near_health_chance, width, height, dropped_at, rng);
    if hurt.frog {
        let chance = tuning().frog_pack_near_health_chance;
        maybe_spawn_bonus(world, map, slot, PickupKind::FrogHealth, chance, width, height, dropped_at, rng);
    }
    if hurt.towers {
        let chance = tuning().tower_pack_near_health_chance;
        maybe_spawn_bonus(world, map, slot, PickupKind::TowerPack, chance, width, height, dropped_at, rng);
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
    dropped_at: Option<f32>,
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
        spawn_pickup_at(world, pos, kind, dropped_at);
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
/// weapon (`enemy_special_weapon_chance`, one pickup's worth, swapped by a
/// hash for a BB-36 weapon at its share - `sonic::swap_spawn_special`), a
/// possible starting shield (`spawn_shield_chance`, the player's roll too) and
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
            enemy.take_weapon(ActiveWeapon::Laser);
            enemy.laser_variant = if rng.random_range(0.0..1.0) < tuning().laser_blue_pickup_chance {
                LaserVariant::Blue
            } else {
                LaserVariant::Red
            };
        } else if rng.random_range(0.0..1.0) < tuning().enemy_special_weapon_plasma_share {
            enemy.take_weapon(ActiveWeapon::Plasma);
            enemy.plasma_variant = if rng.random_range(0.0..1.0) < tuning().plasma_purple_pickup_chance {
                PlasmaVariant::Purple
            } else {
                PlasmaVariant::Teal
            };
        } else {
            enemy.take_weapon(ActiveWeapon::Minigun);
        }
        // The BB-36 weapons come in by a hashed swap of the special just
        // drawn, never by a draw: the stream above is untouched.
        sonic::swap_spawn_special(&mut enemy, slot, pos);
    }
    if rng.random_range(0.0..1.0) < tuning().spawn_shield_chance {
        enemy.raise_shield();
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
/// marks. Runs at every live damage tier - a hurt tank still drives on its
/// tracks - and stops once the hull is a wreck. Water takes no mark
/// (`depth`, the water under the hull): the treads still turn, but nothing
/// is pressed into a river bed, and the marks laid while `Tank::wet_timer`
/// runs after wading out are wet ones.
fn lay_tracks(tracks: &mut Vec<Track>, tank: &mut Tank, before: Position, depth: crate::ground::Depth) {
    if tank.is_wreck() {
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
        tank.hull_frame = (tank.hull_frame + 1) % TANK_TRACK_FRAMES;
    }
    // Tracks stop in water; ice takes them like the ground does.
    if depth.is_wet() {
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
/// the enemy fights (the player, or a hunter's frog); `sight` is how far
/// it sees (`Game::enemy_sight`).
fn engage_status(tank: &Tank, ai: &Ai, target: Position, sight: f32) -> EngageStatus {
    if tank.is_wreck() {
        EngageStatus::Wreck
    } else if tank.damage >= tuning().enemy_flee_damage {
        EngageStatus::Fleeing
    } else if tank.active_weapon() == ActiveWeapon::Shell && ai.is_retreating() {
        EngageStatus::Retreating
    } else if tank.position.distance_to(target) <= sight || ai.is_hit_alerted() {
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

/// `Game::mark_caps` for a round on `map`.
fn mark_caps(map: &MapFile) -> (usize, usize) {
    if map.class() == crate::framing::MapClass::Arena {
        return (SCORCH_MAX, DECAL_MAX);
    }
    let (w, h) = map.field_size();
    let fields = (w * h / (crate::DEFAULT_SCREEN_WIDTH as f32 * crate::DEFAULT_SCREEN_HEIGHT as f32)).max(1.0);
    let scaled = |cap: usize| (cap as f32 * fields).ceil() as usize;
    (scaled(SCORCH_MAX), scaled(DECAL_MAX))
}

#[cfg(test)]
mod mark_cap_tests {
    use super::*;

    fn caps(toml: &str) -> (usize, usize) {
        mark_caps(&MapFile::from_toml_str(toml).unwrap())
    }

    #[test]
    fn an_arena_keeps_the_fixed_ceilings_and_a_field_map_scales_them_by_area() {
        assert_eq!(caps("version = 1\n"), (SCORCH_MAX, DECAL_MAX));
        assert_eq!(caps("version = 1\nsize = [36, 18]\n"), (SCORCH_MAX, DECAL_MAX));
        // Large, but shown whole: an arena still.
        assert_eq!(caps("version = 1\nsize = [68, 34]\nview = \"whole\"\n"), (SCORCH_MAX, DECAL_MAX));
        // Four standard fields' worth of ground, four times the marks.
        assert_eq!(caps("version = 1\nsize = [68, 34]\n"), (4 * SCORCH_MAX, 4 * DECAL_MAX));
        // A field map smaller than the standard field keeps no fewer.
        assert_eq!(caps("version = 1\nsize = [30, 15]\nview = \"follow\"\n"), (SCORCH_MAX, DECAL_MAX));
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
            let (w, h) = game.map.field_size();
            game.init(w, h);
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
                    game.update(Input::default(), 1.0 / 60.0, w, h);
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
        // The round is the shipped default map's, on its own 48 x 24 field,
        // so an edit of that map moves these as surely as a change to the
        // draws does: re-baseline with the map only once nothing but the
        // map moved them. The per-seat block runs once per seat after
        // player 1, so a second seat does not disturb the first's stream.
        // Never bump these to go green - work out which change moved them
        // first.
        let (one, two) = (run(1), run(2));
        assert_eq!((one, two), (11_365_739_967_979_466_473, 16_387_833_415_423_339_632), "(one seat, two seats)");
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

    /// The water sandbox's shapes painted in lava instead: a 5 x 5 lake
    /// at columns 14..=18, rows 9..=13 (its 3 x 3 interior deep), a
    /// north-south stream at column 26 and a sideways stream along row 4
    /// from column 8 to column 14.
    fn lava_map(extra: &str) -> String {
        water_map(extra).replace("kind = \"water\"", "kind = \"lava\"")
    }

    fn damage_of(game: &Game) -> f32 {
        with_tank(&game.world, game.player().expect("player"), |t| t.damage)
    }

    #[test]
    fn a_lava_lake_stops_a_hull_but_not_a_shell() {
        let mut game = sandbox(&lava_map(""));
        assert_eq!(game.lava().deep_cells().count(), 9, "the 3 x 3 interior is deep");
        with_tank_mut(&game.world, game.player().expect("player"), |t| t.heat_shield_timer = 100.0);
        teleport_player(&mut game, map::cell_to_world(11, 11));
        drive(&mut game, Some(Dir::Right), 420);
        let p = player_pos(&game);
        assert!(p.x < 464.0, "the hull stopped short of the deep lava: {p:?}");
        assert!(p.x > 400.0, "but it did wade into the edge first: {p:?}");
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
    fn a_lava_ford_slows_and_burns_a_hull_which_keeps_burning_after() {
        let mut dry = sandbox(&lava_map(""));
        teleport_player(&mut dry, map::cell_to_world(6, 16));
        drive(&mut dry, Some(Dir::Right), 120);
        let dry_dist = player_pos(&dry).x - map::cell_to_world(6, 16).x;

        let mut hot = sandbox(&lava_map(""));
        teleport_player(&mut hot, map::cell_to_world(6, 4));
        drive(&mut hot, Some(Dir::Right), 120);
        let hot_dist = player_pos(&hot).x - map::cell_to_world(6, 4).x;
        assert!(hot_dist < dry_dist * 0.8, "wading lava is slower: dry {dry_dist:.0} px, lava {hot_dist:.0} px");
        assert!(damage_of(&hot) > 5.0, "and it burns: {}", damage_of(&hot));
        let burning = with_tank(&hot.world, hot.player().expect("player"), |t| t.burn_timer);
        assert!(burning > 0.0, "the hull leaves the lava alight");
        // Out of the lava and its heat: the afterburn still hurts for a
        // while, then nothing does.
        teleport_player(&mut hot, map::cell_to_world(6, 18));
        let before = damage_of(&hot);
        drive(&mut hot, None, 30);
        let after = damage_of(&hot);
        assert!(after > before, "afterburn: {before} -> {after}");
        drive(&mut hot, None, 240);
        let settled = damage_of(&hot);
        drive(&mut hot, None, 60);
        assert_eq!(damage_of(&hot), settled, "the fire went out");
    }

    #[test]
    fn heat_falls_off_the_banks_and_hurts_only_close_in() {
        let mut game = sandbox(&lava_map(""));
        // Three cells off the north-south stream at column 26: shimmer
        // only, nothing hurts.
        teleport_player(&mut game, map::cell_to_world(23, 16));
        drive(&mut game, None, 120);
        assert_eq!(damage_of(&game), 0.0, "three cells out is safe");
        assert!(game.heat_at(map::cell_to_world(23, 16)) > 0.0, "though it is warm");
        // On the bank, a cell away: hurt, slowly.
        let mut bank = sandbox(&lava_map(""));
        teleport_player(&mut bank, map::cell_to_world(25, 16));
        drive(&mut bank, None, 120);
        let hurt = damage_of(&bank);
        assert!(hurt > 0.0 && hurt < 15.0, "a bank burns a little: {hurt}");
    }

    #[test]
    fn a_heat_shield_carries_a_hull_across_the_lava_unhurt() {
        let mut game = sandbox(&lava_map(""));
        let player = game.player().expect("player");
        with_tank_mut(&game.world, player, |t| t.heat_shield_timer = tuning().heat_shield_seconds);
        teleport_player(&mut game, map::cell_to_world(24, 16));
        drive(&mut game, Some(Dir::Right), 150);
        assert!(player_pos(&game).x > map::cell_to_world(27, 16).x, "across the stream");
        assert_eq!(damage_of(&game), 0.0);
        assert_eq!(with_tank(&game.world, player, |t| t.burn_timer), 0.0, "and not alight");
    }

    #[test]
    fn the_router_walls_off_a_lava_lake_and_prices_a_lava_ford() {
        let game = sandbox(&lava_map(""));
        let grid = game.nav_grid(W, H);
        assert!(!grid.usable(map::cell_to_world(16, 11)), "the lake's deep middle is a wall");
        assert!(game.lava().ford_cells().count() > 20);
    }

    #[test]
    fn lava_never_freezes_under_snow() {
        let mut game = sandbox(&lava_map(""));
        game.change_weather(crate::map::Weather::Snow.into());
        assert_eq!(game.lava().deep_cells().count(), 9);
        assert_eq!(game.lava().depth_at(map::cell_to_world(26, 8)), crate::ground::Depth::Shallow);
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

    /// A destroy round of `enemies` on the open map, two seats.
    fn two_seat_destroy_round(enemies: usize) -> Game {
        let mut game = Game::default();
        game.enemy_count_override = Some(enemies);
        game.seed_override = Some(7);
        game.players = PlayerCount::TWO;
        game.level_overrides.mission = Some(Mission::Destroy);
        game.map = MapFile::from_toml_str(OPEN_MAP).expect("test map parses");
        game.init(W, H);
        game
    }

    fn enemy_slots(game: &Game) -> Vec<usize> {
        let mut slots: Vec<usize> = game.world.query::<&Tank>().with::<&Ai>().iter().map(|t| t.owner_slot()).collect();
        slots.sort_unstable();
        slots
    }

    /// The end screen's numbers: a seat's shell that finishes an enemy is
    /// that seat's wreck, a wreck no seat touched counts for the team
    /// alone, and the round clock stops when the round ends.
    #[test]
    fn round_stats_count_the_wrecks_and_credit_the_seat_that_shot() {
        let mut game = two_seat_destroy_round(2);
        let slots = enemy_slots(&game);
        assert_eq!(game.round_stats(), RoundStats { seconds: 0.0, destroyed: 0, enemies: 2, by_seat: [0; MAX_SEATS] });

        // Player 2's shell, point blank into an enemy one hit finishes.
        let target = slots[0];
        let at = Position::new(400.0, 300.0);
        game.debug_teleport(target, at, Some(0.0)).expect("the enemy exists");
        for tank in game.world.query::<&mut Tank>().with::<&Ai>().iter() {
            if tank.owner_slot() == target {
                tank.damage = MAX_DAMAGE - 0.5;
                tank.shield_hp = 0.0;
            }
        }
        let shooter = Tank { position: Position::new(at.x, at.y - 80.0), rotation: 180.0, ..Tank::default() };
        game.world.spawn((Shell::spawn(&shooter, Owner::Player(1), 0.0, 0.0),));
        let wrecked = |game: &Game| game.world.query::<&Tank>().iter().any(|t| t.owner_slot() == target && t.is_wreck());
        for _ in 0..60 {
            step(&mut game, Input::default());
            if wrecked(&game) {
                break;
            }
        }
        assert!(wrecked(&game), "the shell finished the enemy");
        let stats = game.round_stats();
        assert_eq!((stats.destroyed, stats.by_seat[0], stats.by_seat[1]), (1, 0, 1), "player 2's wreck");
        assert_eq!(game.outcome(), Outcome::Playing);

        // The other goes down to nobody's shot.
        game.debug_kill(slots[1]).expect("the enemy exists");
        step(&mut game, Input::default());
        assert_eq!(game.outcome(), Outcome::Won);
        let stats = game.round_stats();
        assert_eq!((stats.destroyed, stats.enemies, stats.by_seat[0], stats.by_seat[1]), (2, 2, 0, 1));
        assert!(stats.seconds > 0.0);
        for _ in 0..30 {
            step(&mut game, Input::default());
        }
        assert_eq!(game.round_stats().seconds, stats.seconds, "the clock stopped with the round");
    }

    /// A wave round's enemies are every wave's tanks, counted before a
    /// single one has rolled in.
    #[test]
    fn a_wave_rounds_enemy_total_is_every_wave() {
        let mut game = Game::default();
        game.seed_override = Some(7);
        game.level_overrides.spawn = Some(crate::level::SpawnKind::Waves);
        game.level_overrides.waves = Some(3);
        game.level_overrides.wave_size = Some(2);
        game.level_overrides.wave_growth = Some(1);
        game.map = MapFile::from_toml_str(OPEN_MAP).expect("test map parses");
        game.init(W, H);
        assert_eq!(game.round_stats().enemies, 2 + 3 + 4);
    }

    /// A held end screen counts down like any other and then waits at
    /// zero: the round stays over long past `restart_delay` until the
    /// caller starts the next one; not held, it restarts on its own as it
    /// always has.
    #[test]
    fn a_held_end_screen_counts_down_and_waits_for_the_caller() {
        for hold in [false, true] {
            let mut game = two_seat_destroy_round(1);
            game.hold_end_screen = hold;
            let slot = enemy_slots(&game)[0];
            game.debug_kill(slot).expect("the enemy exists");
            step(&mut game, Input::default());
            assert_eq!(game.outcome(), Outcome::Won);
            let full = game.restart_countdown();
            step(&mut game, Input::default());
            assert!(game.restart_countdown() < full, "hold {hold}: the countdown runs");
            let frames = (game.end_beats().total() * 60.0).ceil() as usize + 60;
            for _ in 0..frames {
                step(&mut game, Input::default());
            }
            let expected = if hold { Outcome::Won } else { Outcome::Playing };
            assert_eq!(game.outcome(), expected, "hold {hold}");
            if hold {
                assert_eq!(game.restart_countdown(), 0.0, "held at zero, never below");
            }
            // The R key is the caller too: it starts the next round, and
            // the setting outlives it.
            let mut restart = Input::default();
            restart.restart_pressed = true;
            step(&mut game, restart);
            assert_eq!(game.outcome(), Outcome::Playing);
            assert_eq!(game.round_stats().destroyed, 0, "a new round counts from nothing");
            assert_eq!(game.hold_end_screen, hold);
        }
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

    /// The sight-box tests' field (docs/large-maps-follow-camera.md section
    /// 5): the player near the top middle of an empty map and one enemy, no
    /// frog - nothing but that enemy ever fires, and nothing stands between
    /// the two.
    const SIGHT_BOX_MAP: &str = r#"
version = 1
tanks = 1
cells."20,4" = { kind = "start" }
"#;

    /// The AFK player on `SIGHT_BOX_MAP` and its one enemy, a plain
    /// `Role::Player` attacker, put down at `offset` from the player and
    /// facing it. Returns the round and where the player stands.
    fn duel(offset: Vec2) -> (Game, Position) {
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(7);
        game.player_row_override = Some(0);
        game.level_overrides.mission = Some(Mission::Destroy);
        game.map = MapFile::from_toml_str(SIGHT_BOX_MAP).expect("test map parses");
        game.init(W, H);
        set_enemy_role(&mut game, Role::Player);
        let player = player_pos(&game);
        let at = player + offset;
        game.debug_teleport(1, at, Some(Dir::toward(at, player).rotation())).expect("enemy in slot 1");
        (game, player)
    }

    /// Step until the enemy in slot 1 pulls its trigger, at most `seconds`:
    /// where it stood as that frame began - the position its decision was
    /// made from - and where it stood at the start.
    fn first_shot(game: &mut Game, seconds: f32) -> (Option<Position>, Position) {
        let start = position_of(game, 1);
        for _ in 0..(seconds * 60.0) as u32 {
            let at = position_of(game, 1);
            step(game, Input::default());
            if game.events().iter().any(|e| matches!(e, Event::Fired { slot: 1, .. })) {
                return (Some(at), start);
            }
        }
        (None, start)
    }

    /// An enemy lined up on a seat's column, in attack range but further
    /// out than the seat's sight box reaches, does not fire from there: it
    /// closes in and fires from inside the box.
    #[test]
    fn an_enemy_lined_up_on_a_column_outside_the_sight_box_closes_in_before_it_fires() {
        let (_, half_h) = tuning().sight_box_half_px();
        assert!(300.0 > half_h && 300.0 < tuning().enemy_attack_range, "the defaults this is about");
        let (mut game, player) = duel(Vec2::new(0.0, 300.0));
        let (fired_from, _) = first_shot(&mut game, 4.0);
        let at = fired_from.expect("it never fired");
        assert!((at.y - player.y).abs() <= half_h, "fired from {:.0} px below the seat, outside its box", at.y - player.y);
        assert!(crate::ai::in_sight_box(player, at));
    }

    /// Inside the box the same column is a firing line: the enemy holds
    /// where it stands and fires, and says whom it fired at.
    #[test]
    fn an_enemy_lined_up_on_a_column_inside_the_sight_box_fires_from_where_it_stands() {
        let (_, half_h) = tuning().sight_box_half_px();
        assert!(230.0 < half_h, "the defaults this is about");
        let (mut game, _) = duel(Vec2::new(0.0, 230.0));
        let (fired_from, start) = first_shot(&mut game, 3.0);
        let at = fired_from.expect("it never fired");
        assert!(at.distance_to(start) < 4.0, "it held where it stood, not {:.1} px away", at.distance_to(start));
        let enemy = game.tank_snapshots().into_iter().find(|t| t.slot == 1).expect("enemy");
        assert_eq!(enemy.shot_at_seat, Some(0), "the shot was aimed at the seat");
    }

    /// Sideways the box reaches past the attack range, so a row is a
    /// firing line all the way out, as it always was.
    #[test]
    fn an_enemy_lined_up_on_a_row_fires_from_its_attack_range() {
        let (half_w, _) = tuning().sight_box_half_px();
        assert!(330.0 < tuning().enemy_attack_range && tuning().enemy_attack_range < half_w, "the defaults this is about");
        let (mut game, _) = duel(Vec2::new(330.0, 0.0));
        let (fired_from, start) = first_shot(&mut game, 3.0);
        let at = fired_from.expect("it never fired");
        assert!(at.distance_to(start) < 4.0, "it held where it stood, not {:.1} px away", at.distance_to(start));
    }

    /// The engagement ring's south firing slot stands inside the seat's
    /// sight box, half a cell in from its edge, and the tank holding it
    /// fires from inside the box.
    #[test]
    fn the_rings_south_slot_stands_inside_the_sight_box() {
        let mut game = game_on(OPEN_MAP, 2, Some(0));
        set_role_of(&mut game, 1, Role::Player);
        set_role_of(&mut game, 2, Role::Player);
        let player = player_pos(&game);
        // Two engaged tanks build the ring: one straight below, one east.
        game.debug_teleport(1, player + Vec2::new(0.0, 300.0), Some(Dir::Up.rotation())).expect("slot 1");
        game.debug_teleport(2, player + Vec2::new(300.0, 0.0), Some(Dir::Left.rotation())).expect("slot 2");
        step(&mut game, Input::default());
        let below = game.tank_entity_by_slot(1).expect("slot 1");
        let slot = game.last_engage.slot_of(below).expect("the tank below holds a ring slot");
        assert_eq!((slot.axis_name(), slot.rank), ("down", 0));
        let target = game.last_engage.target(below).expect("its slot's point");
        let (_, half_h) = tuning().sight_box_half_px();
        assert_eq!(target.y - player.y, half_h - OBSTACLE_GRID_SIZE * 0.5, "half a cell inside the box's edge");
        assert!(crate::ai::in_sight_box(player, target));
        let (fired_from, _) = first_shot(&mut game, 4.0);
        let at = fired_from.expect("the tank on the south slot never fired");
        assert!(crate::ai::in_sight_box(player, at), "fired from {:.0} px below the seat", at.y - player.y);
    }

    /// An enemy's seeker missile locks onto a seat only while the tank that
    /// launched it stands inside the seat's sight box - its seek reaches
    /// much further - and otherwise comes down on the launcher's aim point.
    #[test]
    fn an_enemy_missile_locks_onto_a_seat_only_from_inside_its_sight_box() {
        let lock = |offset: Vec2| {
            let (mut game, player) = duel(offset);
            let launcher = player + offset;
            assert!(offset.length() < tuning().missile_seek_range, "the seat is within the seek");
            let mut missile = crate::missile::Missile::spawn(launcher, Vec2::new(0.0, -1.0), Owner::Enemy(1), 0, launcher);
            missile.stage = crate::missile::MissileStage::Seek;
            game.world.spawn((missile,));
            step(&mut game, Input::default());
            game.events()
                .iter()
                .find_map(|e| if let Event::MissileLocked { slot: 1, target, .. } = e { Some(*target) } else { None })
                .expect("the missile looked for a target")
        };
        assert_eq!(lock(Vec2::new(0.0, 300.0)), None, "launched from below the seat's box");
        assert_eq!(lock(Vec2::new(0.0, 200.0)), Some(0), "launched from inside it");
        assert_eq!(lock(Vec2::new(330.0, 0.0)), Some(0), "launched from beside the seat, inside the box");
    }

    /// `TankSnapshot::hit_by_seat` names the seat whose shot landed on an
    /// enemy on the frame it landed, and nobody on the next.
    #[test]
    fn a_snapshot_names_the_seat_that_hit_an_enemy_this_frame() {
        let (mut game, player) = duel(Vec2::new(0.0, 150.0));
        game.debug_teleport(0, player, Some(Dir::Down.rotation())).expect("the seat");
        let fire = Input::single(Intent { fire: true, ..Intent::default() });
        step(&mut game, fire);
        let mut hit = false;
        for _ in 0..60 {
            step(&mut game, Input::default());
            let enemy = game.tank_snapshots().into_iter().find(|t| t.slot == 1).expect("enemy");
            if game.events().iter().any(|e| matches!(e, Event::Hit { target: HitTarget::Enemy { slot: 1 }, .. })) {
                assert_eq!(enemy.hit_by_seat, Some(0), "the shell was the seat's");
                hit = true;
            } else if hit {
                assert_eq!(enemy.hit_by_seat, None, "the next frame nobody hit it");
                return;
            }
        }
        panic!("the seat's shell never landed (hit: {hit})");
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
    /// cell (32 px, well inside the hull's reach) away, so it is collected
    /// on the first frame without moving and nothing outside can interfere.
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

    /// A pickup collects the moment the sprites touch (`Pickup::in_reach`
    /// with `pickup_collect_pad_px`): beside it, in front of it and corner
    /// to corner alike, a px inside the reach takes it and a px outside
    /// leaves it. In front and at the corner are where a disc around the
    /// pickup's centre fell short of the hull's long side.
    #[test]
    fn a_pickup_collects_the_moment_the_sprites_touch() {
        let at = Position::new(640.0, 360.0);
        let try_at = |offset: Position| {
            let mut game = game_on(OPEN_MAP, 1, Some(1));
            spawn_pickup_at(&mut game.world, at, PickupKind::Ammo, None);
            teleport_player(&mut game, Position::new(at.x + offset.x, at.y + offset.y));
            let before = player_ammo(&game);
            step(&mut game, Input::default());
            player_ammo(&game) > before
        };
        let (hx, hy) = {
            let game = game_on(OPEN_MAP, 1, Some(1));
            let player = game.player().expect("player");
            with_tank(&game.world, player, |t| t.hull_half_extents(t.facing_along_x()))
        };
        assert!(hy > hx, "the hull faces up, so its long side is the y axis");
        let reach_x = hx + 16.0 + tuning().pickup_collect_pad_px;
        let reach_y = hy + 16.0 + tuning().pickup_collect_pad_px;
        assert!(try_at(Position::new(reach_x - 1.0, 0.0)), "beside it, touching");
        assert!(!try_at(Position::new(reach_x + 1.0, 0.0)), "beside it, a px short");
        assert!(try_at(Position::new(0.0, reach_y - 1.0)), "in front of it, touching");
        assert!(!try_at(Position::new(0.0, reach_y + 1.0)), "in front of it, a px short");
        assert!(try_at(Position::new(reach_x - 1.0, reach_y - 1.0)), "corner to corner, touching");
        assert!(!try_at(Position::new(reach_x + 1.0, reach_y + 1.0)), "corner to corner, a px short");
    }

    /// A tank carries one special weapon at a time (`Tank::take_weapon`):
    /// a crate for another weapon replaces the one carried, its ammo lost; a
    /// crate for the same weapon refills it to one crate's worth rather than
    /// stacking; spent, the trigger fires shells, whose magazine no weapon
    /// crate touches.
    #[test]
    fn a_weapon_crate_replaces_the_special_carried_and_refills_the_same() {
        let at = Position::new(640.0, 360.0);
        let mut game = game_on(OPEN_MAP, 0, Some(1));
        let player = game.player().expect("player");
        teleport_player(&mut game, at);
        with_tank_mut(&game.world, player, |t| t.take_weapon(ActiveWeapon::Minigun));
        let shells = with_tank(&game.world, player, |t| t.shells_ammo);
        let collect = |game: &mut Game, kind: PickupKind| {
            spawn_pickup_at(&mut game.world, at, kind, None);
            step(game, Input::default());
            assert_eq!(game.world.query::<&Pickup>().iter().count(), 0, "{kind:?} collected");
        };

        collect(&mut game, PickupKind::Laser);
        let t = tuning();
        let held = |game: &Game| with_tank(&game.world, player, |t| (t.active_weapon(), t.minigun_ammo, t.laser_charges, t.shells_ammo));
        assert_eq!(held(&game), (ActiveWeapon::Laser, 0, t.laser_charges_per_pickup, shells), "the laser replaced the minigun");

        with_tank_mut(&game.world, player, |t| t.laser_charges = 1);
        collect(&mut game, PickupKind::Laser);
        assert_eq!(held(&game).2, t.laser_charges_per_pickup, "the same weapon refills to a crate's worth");
        collect(&mut game, PickupKind::Laser);
        assert_eq!(held(&game).2, t.laser_charges_per_pickup, "and never stacks past it");

        with_tank_mut(&game.world, player, |t| t.laser_charges = 0);
        assert_eq!(held(&game), (ActiveWeapon::Shell, 0, 0, shells), "spent, the trigger fires shells");
    }

    /// An enemy takes a weapon crate only while it fires shells
    /// (`Tank::wants_pickup`): one carrying a special drives over it rather
    /// than trading the weapon it has away.
    #[test]
    fn an_armed_enemy_leaves_a_weapon_crate_for_one_on_shells() {
        let at = Position::new(300.0, 560.0);
        let mut game = game_on(OPEN_MAP, 1, Some(1));
        game.debug_teleport(1, at, Some(0.0)).expect("enemy in slot 1");
        let enemy = game.tank_entity_by_slot(1).expect("enemy");
        with_tank_mut(&game.world, enemy, |t| {
            t.disarm();
            t.take_weapon(ActiveWeapon::Minigun);
        });
        spawn_pickup_at(&mut game.world, at, PickupKind::Laser, None);
        step(&mut game, Input::default());
        assert_eq!(game.world.query::<&Pickup>().iter().count(), 1, "an armed enemy drives over it");
        assert_eq!(with_tank(&game.world, enemy, |t| t.active_weapon()), ActiveWeapon::Minigun);

        with_tank_mut(&game.world, enemy, |t| t.disarm());
        step(&mut game, Input::default());
        assert_eq!(game.world.query::<&Pickup>().iter().count(), 0, "one on shells takes it");
        assert_eq!(with_tank(&game.world, enemy, |t| t.active_weapon()), ActiveWeapon::Laser);
    }

    /// The forgiveness a thumb needs (`player_shot_hit_pad_px`): a player's
    /// shot is tested against an enemy's hull grown by the pad, so a shell
    /// passing a few px outside the silhouette still lands; the enemy's
    /// shot at the player is tested against the exact box, and with the
    /// pad at zero so is the player's.
    #[test]
    fn a_players_shot_gets_the_pad_against_an_enemy_and_an_enemys_does_not() {
        let mut game = game_on(OPEN_MAP, 1, Some(1));
        let enemy = game.world.query::<(Entity, &Tank)>().with::<&Ai>().iter().map(|(e, _)| e).next().expect("one enemy");
        game.debug_teleport(1, Position::new(700.0, 400.0), Some(0.0)).expect("enemy in slot 1");
        teleport_player(&mut game, Position::new(300.0, 400.0));
        let pad = tuning().player_shot_hit_pad_px;
        let shell = tuning().shell_hit_half_extent;
        assert!(pad >= 6.0, "the 6 px pass below needs the pad the table ships");
        // A vertical shot `dx` px outside a tank's hull box, past it on
        // both ends.
        let shot_beside = |game: &Game, entity: Entity, dx: f32| {
            let (center, half) = with_tank(&game.world, entity, |t| t.hull_bbox_world());
            let x = center.x + half.x + dx;
            (Position::new(x, center.y + 200.0), Position::new(x, center.y - 200.0))
        };
        let hits_tank = |terrain: &Terrain, shooter: Owner, (p0, p1): (Position, Position)| {
            matches!(terrain.sweep(&game.world, game.players(), shooter, p0, p1, shell), Some((ShellTarget::Tank(_), _)))
        };
        let enemy_slot = with_tank(&game.world, enemy, |t| t.owner_slot());
        let player = game.player().expect("player");
        let terrain = Terrain::build(&game.world, W, H, &game.grass_cells, &game.water);
        assert!(hits_tank(&terrain, Owner::Player(0), shot_beside(&game, enemy, 6.0)), "6 px outside the enemy's hull is inside the pad");
        assert!(!hits_tank(&terrain, Owner::Player(0), shot_beside(&game, enemy, pad + shell + 6.0)), "past the pad is a miss");
        assert!(!hits_tank(&terrain, Owner::Enemy(enemy_slot), shot_beside(&game, player, 6.0)), "the enemy's shot at the player gets no pad");
        let exact = Terrain::build(&game.world, W, H, &game.grass_cells, &game.water).with_player_shot_pad(0.0);
        assert!(!hits_tank(&exact, Owner::Player(0), shot_beside(&game, enemy, 6.0)), "no pad, no forgiveness");
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

    /// Shots through portals (docs/teleporting.md, "Shots"): the player at
    /// (4, 11) facing right, portal A (12, 11) in its line, portal B
    /// (28, 5) on the far side of an iron column at col 20, and a brick
    /// wall at (36, 5) on B's row - in line with A only through the
    /// portal. A frog keeps the round going.
    const SHOT_PORTALS_MAP: &str = r#"
version = 1
size = [40, 22]
tanks = 0
cells."4,11" = { kind = "start" }
cells."4,17" = { kind = "frog" }
cells."12,11" = { kind = "portal" }
cells."28,5" = { kind = "portal" }
cells."36,5" = { kind = "wall", material = "brick" }
"#;

    fn shot_portals_map() -> String {
        let mut map = String::from(SHOT_PORTALS_MAP);
        for row in 0..22 {
            map.push_str(&format!("cells.\"20,{row}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
        }
        map
    }

    fn shot_teleports(game: &Game) -> Vec<(Option<u32>, Position, Position)> {
        game.events()
            .iter()
            .filter_map(|e| match *e {
                Event::ShotTeleported { id, x, y, to_x, to_y } => Some((id, Position::new(x, y), Position::new(to_x, to_y))),
                _ => None,
            })
            .collect()
    }

    /// A shell fired into portal A comes out of portal B at the same
    /// offset, heading and speed kept, and flies on to the brick wall on
    /// B's row - one `ShotTeleported` naming it, and its hit on the far
    /// side of the iron.
    #[test]
    fn a_shell_fired_into_a_portal_comes_out_of_the_other() {
        let mut game = game_on(&shot_portals_map(), 0, None);
        assert!(game.portals_active());
        let (a, b) = (map::cell_to_world(12, 11), map::cell_to_world(28, 5));
        game.debug_teleport(0, map::cell_to_world(4, 11), Some(90.0)).unwrap();
        step(&mut game, Input::single(Intent { fire: true, ..Intent::default() }));
        let shell = game.world.query::<&Shell>().iter().map(|s| (s.id, s.velocity)).next().expect("the shell left");
        let mut passed = None;
        let mut wall_hit = None;
        for _ in 0..240 {
            step(&mut game, Input::default());
            if let Some(&pass) = shot_teleports(&game).first() {
                assert!(passed.is_none(), "one pass");
                passed = Some(pass);
                let moved = game.world.query::<&Shell>().iter().find(|s| s.id == shell.0).map(|s| (s.position, s.velocity)).expect("still flying");
                assert_eq!(moved.1, shell.1, "heading and speed kept");
                assert!(moved.0.distance_to(pass.2) < 1e-3, "placed where the event says");
            }
            wall_hit = game.events().iter().find_map(|e| match *e {
                Event::Hit { target: HitTarget::Obstacle { material: Material::Brick }, x, y, .. } => Some(Position::new(x, y)),
                _ => None,
            });
            if wall_hit.is_some() {
                break;
            }
        }
        let (id, at, to) = passed.expect("the shell went through portal A");
        assert_eq!(id, Some(shell.0));
        assert!(at.distance_to(a) <= tuning().portal_shot_radius, "went in at A: {at:?}");
        assert!((to.x - b.x).abs() < 1.0 && (to.y - b.y).abs() < 1.0, "came out of B's anchor in line: {to:?}");
        let hit = wall_hit.expect("the shell reached the brick wall past B");
        assert!(hit.x > map::cell_to_world(20, 0).x && (hit.y - b.y).abs() < 1.0, "on B's row, past the iron: {hit:?}");
    }

    /// A laser fired into a portal is bent: one leg into A, a pass, a leg
    /// out of B on the same heading - judged along the bent path - that
    /// burns the enemy standing past B, whom no straight line from the
    /// player reaches.
    #[test]
    fn a_laser_fired_into_a_portal_is_bent_and_hits_past_the_exit() {
        let mut game = game_on(&shot_portals_map(), 0, None);
        let (a, b) = (map::cell_to_world(12, 11), map::cell_to_world(28, 5));
        game.debug_teleport(0, map::cell_to_world(4, 11), Some(90.0)).unwrap();
        game.debug_set_tank(0, &crate::simulation::debug::TankPatch { laser_charges: Some(1), ..Default::default() }).unwrap();
        let slot = game.debug_spawn_enemy(map::cell_to_world(33, 5), None, None).expect("an enemy past B");
        step(&mut game, Input::single(Intent { fire: true, ..Intent::default() }));
        let legs: Vec<(u8, bool, Position, Position)> = game
            .events()
            .iter()
            .filter_map(|e| match *e {
                Event::LaserBeam { x0, y0, x1, y1, leg, portal, .. } => Some((leg, portal, Position::new(x0, y0), Position::new(x1, y1))),
                _ => None,
            })
            .collect();
        assert_eq!(legs.len(), 2, "two legs: {legs:?}");
        let (first, second) = (legs[0], legs[1]);
        assert_eq!((first.0, first.1), (0, true), "the first leg ends going into a portal");
        assert!(first.3.distance_to(a) <= tuning().portal_shot_radius, "into A: {first:?}");
        assert_eq!((second.0, second.1), (1, false), "the second leg stops on something");
        assert!((second.2.y - b.y).abs() < 1.0 && (second.2.x - b.x).abs() < 1.0, "out of B: {second:?}");
        assert!(second.3.x > b.x && (second.3.y - b.y).abs() < 1.0, "on the same heading: {second:?}");
        let passes = shot_teleports(&game);
        assert_eq!(passes.len(), 1);
        assert_eq!(passes[0].0, None, "a beam's pass names no projectile");
        let burnt = game.events().iter().any(|e| matches!(*e, Event::Hit { target: HitTarget::Enemy { slot: s }, .. } if s == slot));
        assert!(burnt, "the bent beam hit the enemy past B: {:?}", game.events());
    }

    /// Whether a shot met the iron column at col 20 this frame - a shell
    /// ricochets off iron, so its first contact there is a `Ricochet`.
    fn struck_the_iron(game: &Game) -> bool {
        let wall = map::cell_to_world(20, 0).x;
        game.events().iter().any(|e| match *e {
            Event::Ricochet { x, .. } | Event::Hit { x, .. } => (x - wall).abs() < OBSTACLE_GRID_SIZE,
            _ => false,
        })
    }

    /// The pass cap: a shot that has been through
    /// `portal_shot_max_passes` portals flies over the next as over open
    /// ground - the shell crosses A and meets the iron column.
    #[test]
    fn a_shot_past_its_passes_flies_over_the_portal() {
        let mut game = game_on(&shot_portals_map(), 0, None);
        game.debug_teleport(0, map::cell_to_world(4, 11), Some(90.0)).unwrap();
        step(&mut game, Input::single(Intent { fire: true, ..Intent::default() }));
        let shell = game.world.query::<(Entity, &Shell)>().iter().map(|(e, _)| e).next().expect("the shell left");
        let cap = tuning().portal_shot_max_passes.max(0) as u8;
        game.world.insert_one(shell, portals::ShotPortals { passes: cap, leaving: None }).unwrap();
        let mut iron = false;
        for _ in 0..240 {
            step(&mut game, Input::default());
            assert!(shot_teleports(&game).is_empty(), "no pass past the cap");
            iron = struck_the_iron(&game);
            if iron {
                break;
            }
        }
        assert!(iron, "the shell flew over A and met the iron");
    }

    /// A tank standing on a portal (its cooldown holding it there) fires
    /// out of it: the shell, leaving the muzzle inside the swirl and short
    /// of the anchor, leaves that portal rather than falling into it, and
    /// meets the iron column.
    #[test]
    fn a_shot_fired_on_a_portal_leaves_it() {
        let mut game = game_on(&shot_portals_map(), 0, None);
        let a = map::cell_to_world(12, 11);
        let muzzle = with_tank(&game.world, game.player().unwrap(), |t| tuning().tank_muzzle_forward_offset[t.row as usize] * t.scale);
        game.debug_teleport(0, Position::new(a.x - muzzle - 6.0, a.y), Some(90.0)).unwrap();
        game.debug_set_tank(0, &crate::simulation::debug::TankPatch { portal_cooldown: Some(5.0), ..Default::default() }).unwrap();
        step(&mut game, Input::single(Intent { fire: true, ..Intent::default() }));
        let spawned = game.world.query::<&Shell>().iter().map(|s| s.position).next().expect("the shell left");
        assert!(spawned.x < a.x && spawned.distance_to(a) <= tuning().portal_shot_radius, "fired inside the swirl, short of the anchor: {spawned:?}");
        let mut iron = false;
        for _ in 0..240 {
            step(&mut game, Input::default());
            assert!(shot_teleports(&game).is_empty(), "the shell does not fall into the portal it was fired on");
            iron = struck_the_iron(&game);
            if iron {
                break;
            }
        }
        assert!(iron, "the shell flew out of A and met the iron");
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
                .any(|e| matches!(e, Event::PickupCollected { slot: 0, kind: PickupKind::FrogHealth, .. })),
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
        maybe_spawn_health_slot_bonuses(&mut wa, &map, slot, W, H, BonusGates::default(), None, &mut with_gate);
        let chance = tuning().shield_near_health_chance;
        maybe_spawn_bonus(&mut wb, &map, slot, PickupKind::Shield, chance, W, H, None, &mut shield_only);
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
        let collected = game.events().iter().any(|e| matches!(e, Event::PickupCollected { slot: 0, kind: PickupKind::Ammo, .. }));
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
            let landed = 30.0 * tuning().player_armor_factor;
            assert_eq!(tank.take_damage(30.0, MAX_DAMAGE), landed);
            assert_eq!(tank.damage, landed, "unshielded again once the pool is spent");
        }
    }

    #[test]
    fn a_picked_up_shield_shatters_when_its_clock_runs_out() {
        let mut game = game_on(SEALED_SHIELD_MAP, 1, Some(0));
        let player = game.player().expect("player");
        with_tank_mut(&game.world, player, |t| t.shield_hp = 0.0);
        step(&mut game, Input::default());
        assert!(player_snapshot(&game).shield_hp > 0.0, "picked up");
        // No respawn into the sealed ring to refill it mid-test.
        game.map_pickup_slots.clear();
        let frames = (tuning().shield_seconds / PHYSICS_FIXED_DT).ceil() as usize + 2;
        let mut broke_at = None;
        for frame in 1..=frames {
            step(&mut game, Input::default());
            if game.events().iter().any(|e| matches!(e, Event::ShieldBroken { .. })) {
                broke_at = Some(frame);
                break;
            }
        }
        let broke_at = broke_at.expect("the clock shattered it, untouched");
        assert!(broke_at + 3 >= frames, "not before its time: frame {broke_at} of {frames}");
        assert_eq!(player_snapshot(&game).shield_hp, 0.0);
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
        for _ in 0..(game.end_beats().total() * 60.0) as u32 - 10 {
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

    // --- Field maps: bounded AI and pacing (docs/large-maps-follow-camera.md
    // section 12, `simulation::field`) ---

    /// A long, empty field map - 110 x 20 cells, far past an arena - with
    /// the player at its west end and nobody else placed.
    const FIELD_STRIP: &str = "version = 1\nsize = [110, 20]\ntanks = 0\ncells.\"3,10\" = { kind = \"start\" }\n";

    /// A Destroy round on `map` at the map's own size, seeded, the player's
    /// chassis pinned. A field map's round, which the tests below check.
    fn field_round(map: &str) -> Game {
        let mut game = Game::default();
        game.seed_override = Some(7);
        game.player_row_override = Some(0);
        game.level_overrides.mission = Some(Mission::Destroy);
        game.map = MapFile::from_toml_str(map).expect("test map parses");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        assert!(game.field_map(), "a field map");
        game
    }

    /// One update at the map's own size.
    fn field_step(game: &mut Game, input: Input) {
        let (w, h) = game.map.field_size();
        game.update(input, 1.0 / 60.0, w, h);
    }

    /// The field-map memory of the enemy in owner slot `slot`.
    fn field_mind_of(game: &Game, slot: usize) -> crate::ai::FieldMind {
        game.world
            .query::<(&Tank, &Ai)>()
            .iter()
            .find(|(t, _)| t.owner_slot() == slot)
            .map(|(_, ai)| ai.field)
            .expect("an enemy in that slot")
    }

    #[test]
    fn a_field_maps_alert_travels_down_the_chain_and_stops_at_its_reach() {
        let mut game = field_round(FIELD_STRIP);
        let player = player_pos(&game);
        let (view, chain) = (game.enemy_sight(), tuning().enemy_alert_chain_px);
        // A sees the player; B and C each stand inside the chain's reach of
        // the one before and out of the player's sight; D stands just past
        // C's reach.
        let a = Position::new(player.x + view - 100.0, player.y);
        let b = Position::new(a.x + chain - 80.0, player.y);
        let c = Position::new(b.x + chain - 80.0, player.y);
        let d = Position::new(c.x + chain + 120.0, player.y);
        assert!(b.distance_to(player) > view && d.x < game.map.field_size().0 - 64.0, "the strip is long enough");
        let slots: Vec<usize> =
            [a, b, c, d].iter().map(|&p| game.debug_spawn_enemy(p, Some(1), None).expect("an enemy spawns")).collect();
        field_step(&mut game, Input::default());
        let alerted: Vec<bool> = slots.iter().map(|&s| field_mind_of(&game, s).alert.is_some()).collect();
        assert_eq!(alerted, [true, true, true, false], "the sighting reaches A, B and C down the chain, never D");
        assert_eq!(field_mind_of(&game, slots[2]).alert, Some(player), "C heads for where A saw the player");
        assert!(game.alert_position.is_none(), "a field map keeps no shared alert");
    }

    #[test]
    fn an_arena_keeps_its_shared_alert_and_no_field_memory() {
        // The strip, shown whole: an arena by its `view` key, so the one
        // sighting alerts every enemy on the map and nothing of a field
        // map's memory is ever written.
        let mut game = Game::default();
        game.seed_override = Some(7);
        game.level_overrides.mission = Some(Mission::Destroy);
        game.map = MapFile::from_toml_str(&format!("{FIELD_STRIP}view = \"whole\"\n")).expect("test map parses");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        assert!(!game.field_map());
        let player = player_pos(&game);
        let near = game.debug_spawn_enemy(Position::new(player.x + 600.0, player.y), Some(1), None).unwrap();
        let far = game.debug_spawn_enemy(Position::new(player.x + 3000.0, player.y), Some(1), None).unwrap();
        game.update(Input::default(), 1.0 / 60.0, w, h);
        assert_eq!(game.alert_position, Some(player), "one sighting is the whole map's alert");
        for slot in [near, far] {
            let mind = field_mind_of(&game, slot);
            assert!(mind.alert.is_none() && mind.home.is_none() && !mind.awake && !mind.called, "slot {slot}: {mind:?}");
        }
    }

    #[test]
    fn a_leashed_enemy_with_nothing_to_fight_turns_back_home() {
        let mut game = field_round(FIELD_STRIP);
        let player = player_pos(&game);
        let leash = tuning().enemy_leash_px;
        // Near enough to think every tick, too far to see the player.
        let home = Position::new(player.x + game.enemy_sight() + 200.0, player.y);
        let slot = game.debug_spawn_enemy(home, Some(1), None).expect("an enemy spawns");
        field_step(&mut game, Input::default());
        assert_eq!(field_mind_of(&game, slot).home, Some(home), "home is where it first stood");
        // Carried well past its leash, facing on away from home - where a
        // chase that lost its target leaves a tank.
        game.debug_teleport(slot, Position::new(home.x + leash + 300.0, home.y), Some(90.0)).expect("teleports");
        let mut back = None;
        for frame in 1..=600 {
            field_step(&mut game, Input::default());
            let mind = field_mind_of(&game, slot);
            assert!(mind.alert.is_none() && !mind.called, "nothing calls it anywhere");
            if position_of(&game, slot).distance_to(home) <= leash {
                back = Some(frame);
                break;
            }
        }
        let frame = back.expect("the enemy never came back inside its leash");
        // 300 px past the leash at an enemy's pace: a drive home, not a
        // wander that happened to pass by.
        assert!(frame < 240, "took {frame} frames to come home");
    }

    #[test]
    fn a_far_enemy_thinks_on_its_stagger_and_one_never_woken_sleeps() {
        let mut game = field_round(FIELD_STRIP);
        let player = player_pos(&game);
        let t = tuning();
        let k = t.enemy_far_think_ticks as u64;
        assert!(k > 1, "far thinking skips ticks");
        // Out of the player's sight but within `enemy_far_px`: thinks every
        // tick. Its twin is woken there too, then carried far away.
        let x = player.x + t.enemy_far_px - 150.0;
        assert!(x - player.x > game.enemy_sight());
        let near = game.debug_spawn_enemy(Position::new(x, player.y - 96.0), Some(1), None).unwrap();
        let far = game.debug_spawn_enemy(Position::new(x, player.y + 96.0), Some(1), None).unwrap();
        // Far from the start and never woken by anything.
        let asleep_at = Position::new(player.x + t.enemy_far_px + 900.0, player.y);
        let asleep = game.debug_spawn_enemy(asleep_at, Some(1), None).unwrap();
        field_step(&mut game, Input::default());
        assert!(field_mind_of(&game, far).awake, "a seat within reach wakes it");
        game.debug_teleport(far, Position::new(player.x + t.enemy_far_px + 400.0, player.y + 96.0), None).unwrap();
        let mut beats = 0;
        for _ in 0..3 * k {
            field_step(&mut game, Input::default());
            assert_eq!(field_mind_of(&game, near).think_debt, 0.0, "a near enemy thinks every tick");
            // `mind` runs with the frame number `update` just counted.
            let on_beat = (game.frame() + far as u64).is_multiple_of(k);
            let debt = field_mind_of(&game, far).think_debt;
            assert_eq!(debt == 0.0, on_beat, "frame {}: a far enemy thinks on its beat only (debt {debt})", game.frame());
            beats += on_beat as u32;
            let sleeper = field_mind_of(&game, asleep);
            assert!(!sleeper.awake && sleeper.think_debt == 0.0, "never woken: {sleeper:?}");
        }
        assert_eq!(beats, 3, "once every {k} ticks");
        assert_eq!(field_mind_of(&game, asleep).home, Some(asleep_at));
        assert!(position_of(&game, asleep).distance_to(asleep_at) < 1.0, "a sleeper holds still");
    }

    #[test]
    fn a_field_maps_band_never_spawns_an_enemy_inside_a_seats_sight_box() {
        let map = "version = 1\nsize = [60, 30]\ntanks = 8\ncells.\"30,15\" = { kind = \"start\" }\n";
        for seats in [1, 2] {
            for seed in 0..10u64 {
                let mut game = Game::default();
                game.seed_override = Some(seed);
                game.players = PlayerCount::from_count(seats).expect("one or two");
                game.level_overrides.mission = Some(Mission::Destroy);
                game.map = MapFile::from_toml_str(map).expect("test map parses");
                let (w, h) = game.map.field_size();
                game.init(w, h);
                let tanks = game.tank_snapshots();
                let seat_at: Vec<Position> = tanks.iter().filter(|t| t.is_player).map(|t| t.position).collect();
                let enemies: Vec<Position> = tanks.iter().filter(|t| !t.is_player).map(|t| t.position).collect();
                assert_eq!((seat_at.len(), enemies.len()), (seats, 8));
                for e in &enemies {
                    for s in &seat_at {
                        assert!(!crate::ai::in_sight_box(*s, *e), "seed {seed}, {seats} seats: an enemy at {e:?} inside the box of the seat at {s:?}");
                    }
                }
            }
        }
    }

    /// A 60 x 20 field map under a three-wave plan of one tank a wave, gates
    /// at both ends: the player holds the east end, a stone's throw from the
    /// east gate, and player 2 starts beside it.
    const FIELD_GATES: &str = "version = 1\nsize = [60, 20]\ncells.\"50,10\" = { kind = \"start\" }\ncells.\"50,6\" = { kind = \"start2\" }\ncells.\"0,10\" = { kind = \"gate\" }\ncells.\"59,10\" = { kind = \"gate\" }\n";

    /// A waves round on `FIELD_GATES` with `seats` seats, every one of
    /// them shielded so only the test ends anything.
    fn field_waves(seats: usize) -> Game {
        let mut game = Game::default();
        game.seed_override = Some(7);
        game.player_row_override = Some(0);
        game.players = PlayerCount::from_count(seats).expect("one or two");
        game.level_overrides.mission = Some(Mission::Destroy);
        game.level_overrides.spawn = Some(SpawnKind::Waves);
        game.level_overrides.waves = Some(3);
        game.level_overrides.wave_size = Some(1);
        game.level_overrides.wave_growth = Some(0);
        game.map = MapFile::from_toml_str(FIELD_GATES).expect("test map parses");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        assert!(game.field_map());
        for seat in game.players().into_iter().flatten() {
            with_tank_mut(&game.world, seat, |t| t.shield_hp = 1.0e9);
        }
        game
    }

    /// Step until a tank enters (at most `limit` frames): its slot and
    /// where it came through.
    fn field_step_until_entered(game: &mut Game, limit: u32) -> (usize, Position) {
        for _ in 0..limit {
            field_step(game, Input::default());
            if let Some(&slot) = tank_entered(game).first() {
                return (slot, position_of(game, slot));
            }
        }
        panic!("no tank entered within {limit} frames");
    }

    #[test]
    fn a_field_maps_wave_rolls_in_out_of_sight_and_is_called_to_the_fight() {
        let mut game = field_waves(1);
        let player = player_pos(&game);
        let (slot, arrived) = field_step_until_entered(&mut game, 600);
        // The east gate's lane ends inside the player's sight box, so the
        // wave takes the west one, a walk across the map.
        assert!(!crate::ai::in_sight_box(player, arrived), "arrived at {arrived:?}, in sight of the player at {player:?}");
        assert!(arrived.x < game.map.field_size().0 * 0.5, "came in through the west gate: {arrived:?}");
        assert!(field_mind_of(&game, slot).called, "a field map's wave is called to the fight");
        // Called, it walks straight to the fight rather than wandering: it
        // comes within sight of the player well inside twice its walk.
        let walk = (player.x - arrived.x) / tuning().enemy_speed;
        let mut seen_at = None;
        for frame in 1..=(2.0 * walk * 60.0) as u32 + 120 {
            field_step(&mut game, Input::default());
            if position_of(&game, slot).distance_to(player) <= game.enemy_sight() {
                seen_at = Some(frame);
                break;
            }
        }
        assert!(seen_at.is_some(), "the called tank never reached the fight (walk {walk:.1}s)");
        field_step(&mut game, Input::default());
        assert!(!field_mind_of(&game, slot).called, "the call ends once the fight is in sight");
    }

    #[test]
    fn a_wrecked_seat_comes_back_through_the_gate_nearest_the_living_seats() {
        let mut game = field_waves(2);
        let w = game.map.field_size().0;
        let (wave_one, arrived) = field_step_until_entered(&mut game, 600);
        assert!(arrived.x < w * 0.5, "the wave keeps out of sight, through the west gate");
        game.debug_kill(1).expect("seat 1 is live");
        field_step(&mut game, Input::default());
        let seat1 = game.seat(1).expect("seat 1");
        assert!(with_tank(&game.world, seat1, Tank::is_wreck));
        // Clearing wave 1 calls wave 2, which seat 1 rejoins: through the
        // east gate, a few cells from the seat still fighting there.
        game.debug_kill(wave_one).expect("wave 1's tank");
        let mut started = None;
        for _ in 0..900 {
            field_step(&mut game, Input::default());
            if game.is_entering(seat1) {
                started = Some(with_tank(&game.world, seat1, |t| t.position));
                break;
            }
        }
        let at = started.expect("seat 1 never started back in");
        assert!(at.x > w, "seat 1 comes back through the east gate, beside its team-mate: {at:?}");
    }

    // --- The pacing director (`simulation::director`) ---

    /// `field_waves(1)` with wave 1 called, rolled in and destroyed where
    /// it came through, far from the seat: the frame the breather before
    /// wave 2 starts.
    fn field_breather() -> Game {
        let mut game = field_waves(1);
        let (slot, _) = field_step_until_entered(&mut game, 600);
        game.debug_kill(slot).expect("wave 1's tank");
        for _ in 0..120 {
            field_step(&mut game, Input::default());
            if game.wave_banner().is_some() {
                return game;
            }
        }
        panic!("clearing wave 1 started no breather");
    }

    /// Frames until wave `wave` is called, at most `limit`, with the
    /// banner up on every one of them before it.
    fn frames_until_wave(game: &mut Game, wave: u32, limit: u32) -> u32 {
        for frame in 1..=limit {
            field_step(game, Input::default());
            if wave_started(game) == Some(wave) {
                return frame;
            }
            assert!(game.wave_banner().is_some(), "frame {frame}: the banner went before wave {wave} came");
        }
        panic!("wave {wave} never came within {limit} frames");
    }

    #[test]
    fn a_calm_field_maps_breather_runs_short() {
        let t = tuning();
        let mut game = field_breather();
        assert!(game.pacing().team <= t.director_calm, "nothing has touched the seat: {:?}", game.pacing());
        // Nothing is happening, so the breather runs at the calm rate.
        let frames = frames_until_wave(&mut game, 2, 600) as f32 / 60.0;
        let calm = (t.wave_gap_seconds / t.director_calm_rate).max(t.director_breather_min_seconds);
        assert!(calm < t.wave_gap_seconds, "the case this is about");
        assert!((frames - calm).abs() <= 0.1, "a calm breather of {calm:.2}s took {frames:.2}s");
    }

    #[test]
    fn the_director_holds_a_wave_while_the_team_is_at_its_peak_and_rests_it_after() {
        let t = tuning();
        let mut game = field_breather();
        // Three enemies inside the seat's sight box: the team at its peak.
        let player = player_pos(&game);
        let crowd: Vec<usize> = (0..3)
            .map(|i| {
                let at = Position::new(player.x - 300.0, player.y - 96.0 + 96.0 * i as f32);
                assert!(crate::ai::in_sight_box(player, at));
                game.debug_spawn_enemy(at, Some(1), None).expect("an enemy spawns")
            })
            .collect();
        // Ten seconds of fight - far past the plain breather - and wave 2
        // is held the whole time, the breather owing the rest a peak does.
        for frame in 0..600 {
            field_step(&mut game, Input::default());
            assert_eq!(wave_started(&game), None, "frame {frame}: wave 2 came in the middle of the fight");
            let pacing = game.pacing();
            assert!(pacing.team >= t.director_peak, "frame {frame}: {pacing:?}");
            assert_eq!(game.wave_status().and_then(|w| w.next_in), Some(t.director_relax_seconds), "frame {frame}");
        }
        // The fight ends: the intensity falls under the peak, and from
        // there the team rests `director_relax_seconds` before wave 2.
        for slot in crowd {
            game.debug_kill(slot).expect("a live enemy");
        }
        let seconds = frames_until_wave(&mut game, 2, 60 * 60) as f32 / 60.0;
        let owed = (1.0 - t.director_peak) * t.director_fall_seconds + t.director_relax_seconds;
        assert!((seconds - owed).abs() <= 1.0, "released after {seconds:.2}s, the rest owed was {owed:.2}s");
    }

    // --- Stragglers (`Game::reroll_stragglers`) ---

    /// A 110 x 20 field map under a three-wave plan of one tank a wave: the
    /// player at the west end, a gate at the east end and one on the north
    /// edge half way along.
    const STRAGGLER_MAP: &str = "version = 1\nsize = [110, 20]\ncells.\"10,10\" = { kind = \"start\" }\ncells.\"60,0\" = { kind = \"gate\" }\ncells.\"109,10\" = { kind = \"gate\" }\n";

    /// Write the field-map memory of the enemy in owner slot `slot`.
    fn set_field_mind(game: &mut Game, slot: usize, set: impl FnOnce(&mut crate::ai::FieldMind)) {
        let entity = game.tank_entity_by_slot(slot).expect("a tank in that slot");
        set(&mut game.world.get::<&mut Ai>(entity).expect("an enemy on the field").field);
    }

    /// A wave tank that lost its way - out of every seat's sight, a long
    /// walk from the fight and on no screen - is taken off and rolled in
    /// again through a gate nearer the fight, keeping its role, and comes
    /// back called to it; one lost within sight of the screens is left
    /// where it is.
    #[test]
    fn a_straggler_is_rolled_in_again_through_a_nearer_gate_and_a_tank_near_the_fight_is_not() {
        let mut game = Game::default();
        game.seed_override = Some(7);
        game.player_row_override = Some(0);
        game.level_overrides.mission = Some(Mission::Destroy);
        game.level_overrides.spawn = Some(SpawnKind::Waves);
        game.level_overrides.waves = Some(3);
        game.level_overrides.wave_size = Some(1);
        game.level_overrides.wave_growth = Some(0);
        game.map = MapFile::from_toml_str(STRAGGLER_MAP).expect("test map parses");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        assert!(game.field_map());
        for seat in game.players().into_iter().flatten() {
            with_tank_mut(&game.world, seat, |t| t.shield_hp = 1.0e9);
        }
        let t = tuning();
        let player = player_pos(&game);
        let (straggler, _) = field_step_until_entered(&mut game, 900);
        // Lost at the far east end, its leash there and its call over, a
        // breath short of a straggler's time.
        let far = Position::new(w - 6.0 * PATHFIND_CELL_SIZE, player.y);
        game.debug_teleport(straggler, far, None).expect("teleports");
        set_field_mind(&mut game, straggler, |m| {
            m.home = Some(far);
            m.called = false;
            m.alert = None;
            m.lost = t.field_reroll_after_seconds - 1.0;
        });
        let role = game.world.get::<&Ai>(game.tank_entity_by_slot(straggler).unwrap()).unwrap().role;
        // A wave tank lost as long, out of the seat's sight but on its
        // screen and a short walk away.
        let near = Position::new(player.x + game.enemy_sight() + 100.0, player.y);
        assert!(!field::beyond_every_screen(player, near, &t), "the case this is about");
        let control = game.debug_spawn_enemy(near, Some(1), None).expect("an enemy spawns");
        set_field_mind(&mut game, control, |m| {
            m.home = Some(near);
            m.wave = true;
            m.lost = t.field_reroll_after_seconds - 1.0;
        });
        let mut taken = None;
        for frame in 1..=180 {
            field_step(&mut game, Input::default());
            for e in game.events() {
                if let Event::Rerolled { slot, x, .. } = *e {
                    assert_eq!(slot, straggler, "only the straggler is rolled in again");
                    assert!(x > w * 0.75, "taken off where it was lost: x {x}");
                    taken = Some(frame);
                }
            }
            if taken.is_some() {
                break;
            }
        }
        let frame = taken.expect("the straggler was never rolled in again");
        assert!(frame >= 60, "taken off after {frame} frames, before it was a straggler");
        let entity = game.tank_entity_by_slot(straggler).expect("the same tank, the same slot");
        assert!(game.is_entering(entity), "it rolls in again");
        // Through the north gate, nearer the fight than the east one it
        // would otherwise walk from.
        let (_, gate) = field_step_until_entered(&mut game, 900);
        assert!(gate.y < 5.0 * PATHFIND_CELL_SIZE && (gate.x - 60.5 * PATHFIND_CELL_SIZE).abs() < 2.0 * PATHFIND_CELL_SIZE, "came back at {gate:?}");
        let mind = field_mind_of(&game, straggler);
        assert!(mind.called && mind.wave && mind.lost < 0.1, "back called to the fight, lost no more: {mind:?}");
        assert_eq!(game.world.get::<&Ai>(entity).unwrap().role, role, "it keeps its role");
        assert!(!game.is_entering(game.tank_entity_by_slot(control).unwrap()), "the tank near the fight stays");
    }

    /// The straggler's exemptions: a guard while the frog it keeps lives,
    /// a hull still burning, and one an EMP has coasting with its brain off
    /// (docs/emp-burst.md), are never taken off - though each is lost, a
    /// long walk from the fight and on no screen, as a straggler is.
    #[test]
    fn a_guard_keeping_its_frog_a_burning_hull_and_a_disabled_one_are_never_rolled_in_again() {
        for exempt in ["guard", "burning", "disabled"] {
            let mut game = Game::default();
            game.seed_override = Some(7);
            game.player_row_override = Some(0);
            game.level_overrides.mission = Some(if exempt == "guard" { Mission::Hunt } else { Mission::Destroy });
            game.level_overrides.spawn = Some(SpawnKind::Waves);
            game.level_overrides.waves = Some(3);
            game.level_overrides.wave_size = Some(1);
            game.level_overrides.wave_growth = Some(0);
            game.map = MapFile::from_toml_str(STRAGGLER_MAP).expect("test map parses");
            let (w, h) = game.map.field_size();
            game.init(w, h);
            assert!(game.field_map());
            for seat in game.players().into_iter().flatten() {
                with_tank_mut(&game.world, seat, |t| t.shield_hp = 1.0e9);
            }
            let t = tuning();
            let player = player_pos(&game);
            let (straggler, _) = field_step_until_entered(&mut game, 900);
            let far = Position::new(w - 6.0 * PATHFIND_CELL_SIZE, player.y);
            game.debug_teleport(straggler, far, None).expect("teleports");
            set_field_mind(&mut game, straggler, |m| {
                m.home = Some(far);
                m.called = false;
                m.alert = None;
                // A disabled tank's mind is frozen (`field::mind` is not
                // called), so it is lost long enough already.
                m.lost = t.field_reroll_after_seconds + if exempt == "disabled" { 1.0 } else { -1.0 };
            });
            let entity = game.tank_entity_by_slot(straggler).expect("the straggler");
            if exempt == "guard" {
                let frog = game.enemy_frog.expect("a Hunt round keeps an enemy frog");
                assert!(with_frog(&game.world, frog, |fr| !fr.is_dead()), "its frog lives");
                game.world.get::<&mut Ai>(entity).expect("an enemy").role = Role::Guard;
            } else if exempt == "disabled" {
                with_tank_mut(&game.world, entity, |tank| {
                    tank.disable(10.0);
                });
            } else {
                with_tank_mut(&game.world, entity, |tank| {
                    tank.burn_timer = 10.0;
                    tank.shield_hp = 1.0e9;
                });
            }
            for frame in 1..=180 {
                field_step(&mut game, Input::default());
                assert!(!game.events().iter().any(|e| matches!(e, Event::Rerolled { .. })), "{exempt}: taken off at frame {frame}");
            }
            // It was a straggler in every other way the whole time.
            let mind = field_mind_of(&game, straggler);
            let at = position_of(&game, straggler);
            assert!(mind.lost >= t.field_reroll_after_seconds, "{exempt}: lost long enough: {mind:?}");
            assert!(field::beyond_every_screen(player_pos(&game), at, &t), "{exempt}: on no screen at {at:?}");
            assert!(!game.is_entering(entity), "{exempt}: still on the field");
        }
    }

    /// A wave called to the fight drives at the nearest seat until a seat
    /// is in its sight or it is hit: a hit ends the call there and then,
    /// with every seat still out of its sight.
    #[test]
    fn a_called_wave_tank_is_called_no_more_once_it_is_hit() {
        let mut game = field_round(FIELD_STRIP);
        let player = player_pos(&game);
        let at = Position::new(player.x + game.enemy_sight() + 300.0, player.y);
        let slot = game.debug_spawn_enemy(at, Some(1), None).expect("an enemy spawns");
        set_field_mind(&mut game, slot, |m| {
            m.called = true;
            m.wave = true;
        });
        field_step(&mut game, Input::default());
        assert!(field_mind_of(&game, slot).called, "out of sight and unhurt, it stays called");
        let entity = game.tank_entity_by_slot(slot).expect("the enemy");
        game.world.get::<&mut Ai>(entity).expect("an enemy").notify_hit();
        field_step(&mut game, Input::default());
        let mind = field_mind_of(&game, slot);
        assert!(position_of(&game, slot).distance_to(player) > game.enemy_sight(), "still out of sight");
        assert!(!mind.called, "a hit ends the call: {mind:?}");
    }

    // --- Flow fields worked out as far as they are read (`pathfind`) ---

    /// A flow field worked out only as far as the round reads it routes
    /// every enemy as the whole field does: the same seeded round on
    /// longwater - two seats, so two fields and the frog's, its first wave
    /// rolling in a long walk out and called across the map to the fort -
    /// with every field settled whole as each frame's grid is built and
    /// without, tank for tank and bit for bit.
    #[test]
    fn a_field_read_as_far_as_it_is_needed_routes_every_enemy_as_the_whole_field_does() {
        let run = |whole: bool| {
            let mut game = Game::default();
            game.seed_override = Some(0xB0B5);
            game.players = PlayerCount::from_count(2).expect("two seats");
            game.map = MapFile::from_toml_str(include_str!("../../maps/longwater.toml")).expect("longwater parses");
            game.whole_fields = whole;
            let (w, h) = game.map.field_size();
            game.init(w, h);
            assert!(game.field_map());
            let mut samples = Vec::new();
            for frame in 1..=900u32 {
                game.update(Input::default(), 1.0 / 60.0, w, h);
                if frame % 30 == 0 {
                    let tanks: Vec<(usize, u32, u32, u32, u32)> = game
                        .tank_snapshots()
                        .iter()
                        .map(|t| (t.slot, t.position.x.to_bits(), t.position.y.to_bits(), t.rotation.to_bits(), t.damage.to_bits()))
                        .collect();
                    samples.push(tanks);
                }
            }
            samples
        };
        let (read, whole) = (run(false), run(true));
        assert!(read.last().is_some_and(|tanks| tanks.len() > 4), "the first wave came onto the field");
        for (i, (a, b)) in read.iter().zip(&whole).enumerate() {
            assert_eq!(a, b, "sample {i}: an enemy went another way on the field read as far as it was needed");
        }
    }

    // --- Steering: lanes and turns (docs/large-maps-follow-camera.md
    // section 12) ---

    /// A 40 x 20 field map split by an iron wall along row 10, open only
    /// at a four-tile gap (columns 18 to 21, whose two middle columns are
    /// the nav lanes a hull fits through), the player south of the wall
    /// and nobody else placed: every route from the north half turns
    /// south into the gap, one cell short of it or on it.
    fn gap_map() -> String {
        let mut map = String::from("version = 1\nsize = [40, 20]\ntanks = 0\ncells.\"28,15\" = { kind = \"start\" }\n");
        for c in (0..40).filter(|c| !(18..=21).contains(c)) {
            map.push_str(&format!("cells.\"{c},10\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
        }
        map
    }

    /// An assault (row 1) on row 8 of `gap_map`, `edge` px above row 9:
    /// the steering case of a hull riding the far edge of its lane, or
    /// its centre line. Returns the round and the enemy's slot.
    fn hull_on_row_8(edge: f32) -> (Game, usize) {
        let mut game = field_round(&gap_map());
        let lane_floor = 9.0 * PATHFIND_CELL_SIZE;
        let slot = game.debug_spawn_enemy(Position::new(208.0, lane_floor - edge), Some(1), None).expect("an enemy spawns");
        (game, slot)
    }

    /// Step until the enemy in `slot` is through the wall (its centre past
    /// the wall's south face), at most `limit` frames: the frame it got
    /// there and where it was the frame it first crossed the wall's north
    /// face, or `None` if it never got through.
    fn through_the_gap(game: &mut Game, slot: usize, limit: u32) -> Option<(u32, Position)> {
        let (north, south) = (10.0 * PATHFIND_CELL_SIZE, 11.0 * PATHFIND_CELL_SIZE);
        let mut entered = None;
        for frame in 1..=limit {
            field_step(game, Input::default());
            let at = position_of(game, slot);
            if entered.is_none() && at.y >= north {
                entered = Some(at);
            }
            if at.y >= south {
                return entered.map(|e| (frame, e));
            }
        }
        None
    }

    /// A hull riding the far edge of its lane - its centre 2.5 px above
    /// the next row, nearer that row's centre than
    /// `ai_dir_switch_margin_px` - still takes its route's turn south into
    /// the gap. Held to the margin alone, such a hull can never turn into
    /// the next row: it drives past the gap, back and forth between the two
    /// ends of the wall, for as long as the round lasts.
    #[test]
    fn a_hull_on_the_far_edge_of_its_lane_takes_its_turn() {
        let (mut game, slot) = hull_on_row_8(2.5);
        let start = position_of(&game, slot);
        let next_row_centre = 9.5 * PATHFIND_CELL_SIZE;
        assert!(next_row_centre - start.y < tuning().ai_dir_switch_margin_px, "the case this is about: {start:?}");
        // About 450 px to the turning at an enemy's pace, then through.
        let (frame, _) = through_the_gap(&mut game, slot, 6 * 60).expect("the hull never turned into the gap");
        assert!(frame < 5 * 60, "took {frame} frames to get through");
    }

    /// The turn is taken where the hull's slide through it ends on the
    /// centre line of the lane it turns into: a hull at speed carries on
    /// along its old heading for `v^2 / 2a` (the tracks' grip,
    /// `tank_turn_grip_force` over its mass) after it turns, so it turns
    /// that much early, and comes down the gap on the centre of the gap's
    /// lane rather than past it at the far edge - where the next turn its
    /// way would be the one the margin could never let it take.
    #[test]
    fn a_hull_turns_early_enough_to_slide_onto_its_new_lanes_centre_line() {
        let (mut game, slot) = hull_on_row_8(16.0);
        let (_, entered) = through_the_gap(&mut game, slot, 6 * 60).expect("the hull never turned into the gap");
        // The route turns south on column 20, whose centre is x = 656.
        let lane_centre = 20.5 * PATHFIND_CELL_SIZE;
        assert!((entered.x - lane_centre).abs() <= 6.0, "came down the gap at x = {:.1}, the lane's centre is {lane_centre}", entered.x);
    }

    /// The same hull on the far edge of its lane, sent through the gap by a
    /// route of its own rather than the field every tank shares: the player
    /// waits in the far corner, out of its sight under fog, and it fetches
    /// the laser south of the wall, inside its leash (`Brain::seek`), along
    /// a searched route. The margin could never turn it south into the next
    /// row, so that turn is a lane turn as well (`ai::margin_never_turns`);
    /// held to the margin alone, it drives on past the gap along the row's
    /// edge until it sees the player.
    #[test]
    fn a_hull_on_the_far_edge_of_its_lane_takes_its_own_routes_turn() {
        let map = gap_map().replace("\"28,15\"", "\"38,18\"")
            + "weather = \"fog\"\ncells.\"23,14\" = { kind = \"pickup\", pickup = \"laser\" }\n";
        let mut game = field_round(&map);
        let lane_floor = 9.0 * PATHFIND_CELL_SIZE;
        let slot = game.debug_spawn_enemy(Position::new(208.0, lane_floor - 2.5), Some(1), None).expect("an enemy spawns");
        field_step(&mut game, Input::default());
        let snapshot = game.debug_snapshot(game.map.field_size().0, game.map.field_size().1, debug::Detail::Full);
        let ai = snapshot.tanks.iter().find(|t| t.slot == slot).and_then(|t| t.ai).expect("the enemy thinks");
        assert_eq!(ai.last_action, Some("seek_laser"), "the case this is about: a searched route");
        let (frame, _) = through_the_gap(&mut game, slot, 6 * 60).expect("the hull never turned into the gap");
        assert!(frame < 5 * 60, "took {frame} frames to get through");
    }

    /// The same errand from just past the margin's reach: the hull's
    /// centre 20.2 px across from the centre line of the row its route
    /// turns into, 0.2 px more than `ai_dir_switch_margin_px`. The margin
    /// turns a hull only on a think whose error along its heading, to the
    /// turning's centre, is under what its error across beats the margin
    /// by - here a window 0.4 px wide - and a hull covering 3 px between
    /// two thinks drives across it without a think inside and on past the
    /// gap. On the study map such a hull rode a column to and fro past its
    /// turnings for the rest of the round (docs/large-maps-follow-camera.md
    /// section 12).
    #[test]
    fn a_hull_just_past_the_margins_reach_takes_its_own_routes_turn() {
        let map = gap_map().replace("\"28,15\"", "\"38,18\"")
            + "weather = \"fog\"\ncells.\"23,14\" = { kind = \"pickup\", pickup = \"laser\" }\n";
        let mut game = field_round(&map);
        let next_row_centre = 9.5 * PATHFIND_CELL_SIZE;
        let across = tuning().ai_dir_switch_margin_px + 0.2;
        let slot = game.debug_spawn_enemy(Position::new(208.0, next_row_centre - across), Some(1), None).expect("an enemy spawns");
        field_step(&mut game, Input::default());
        let snapshot = game.debug_snapshot(game.map.field_size().0, game.map.field_size().1, debug::Detail::Full);
        let ai = snapshot.tanks.iter().find(|t| t.slot == slot).and_then(|t| t.ai).expect("the enemy thinks");
        assert_eq!(ai.last_action, Some("seek_laser"), "the case this is about: a searched route");
        let (frame, _) = through_the_gap(&mut game, slot, 6 * 60).expect("the hull never turned into the gap");
        assert!(frame < 5 * 60, "took {frame} frames to get through");
    }
}


