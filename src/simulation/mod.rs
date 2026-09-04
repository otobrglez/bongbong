//! The simulation layer: everything that decides what the game is doing
//! this frame - physics, damage, AI, spawning - with no dependency on a
//! window or `RaylibHandle`. `game.rs` (the presentation layer) reads
//! `Game`'s state afterward and draws it; this module never reaches back.
//! `Game::init`/`Game::update` take plain numbers and an `Input` snapshot,
//! so a round can be driven headlessly (see `src/bin/probe.rs` and the
//! tests at the bottom of this file). `sola_raylib::core::math::Vector2`
//! (`Position`) is the one shared type, imported by name so nothing here
//! names a window or drawing type.
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
pub mod debug;
mod engage;
mod hits;
mod waves;
mod weapons;

pub use waves::{RollIn, WaveStatus};
use waves::WaveState;

use crate::tuning::tuning;
use std::collections::{HashMap, HashSet};

use hecs::Entity;
use rand::rngs::SmallRng;
use rand::{RngExt, SeedableRng};
use serde::{Deserialize, Serialize};
use sola_raylib::core::math::Vector2;

use crate::ai::{Ai, AiSnapshot, Intent, Mover, WallAhead};
use crate::battlefield;
use crate::bullet::Bullet;
use crate::frog::Frog;
use crate::laser::{LaserBeam, LaserVariant};
use crate::level::{LevelOverrides, Mission, SpawnPlan};
use crate::map::{self, CellObject, MapFile};
use crate::obstacle::Obstacle;
use crate::pathfind::Grid;
use crate::physics::Physics;
use crate::pickup::{Pickup, PickupKind};
use crate::plasma::{Plasma, PlasmaVariant};
use crate::shell::{Owner, Shell, ShellState};
use crate::shockwave::Shockwave;
use crate::tank::{ActiveWeapon, Dir, Tank};
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
use weapons::{dispatch_fire, laser_damage_range, tick_queued_shots, PendingLaserShot, Projectile, laser_beam_half_width};

/// One frame's player input, gathered by the caller (`main.rs` reading a
/// live `RaylibHandle`, or a scripted probe) - the entire interface
/// between the simulation and wherever input comes from. `player_intent`
/// reuses the AI's `Intent` so the player and every enemy drive through the
/// identical `drive_tank`/fire path; the four flags below are meta/UI
/// toggles `Intent` has no use for.
#[derive(Default, Clone, Copy)]
pub struct Input {
    /// Raw movement/fire command. `fire` is "is the fire key held this
    /// frame", not edge-detected: `update` decides whether that fires (a
    /// laser or minigun is full-auto while held; shells and plasma need a
    /// fresh press), since that depends on the player's current weapon.
    pub player_intent: Intent,
    pub pause_pressed: bool,
    pub restart_pressed: bool,
    pub toggle_shadows_pressed: bool,
    /// The I key in a dev build (`--features dev-tools`): cycles
    /// `Game::debug_overlays` through its presets (`Overlays::next_preset`).
    /// Never set in a release build.
    pub cycle_overlays_pressed: bool,
}

/// The player tank's owner slot (`Tank::owner_slot`); enemies take `n + 1`.
pub(crate) const PLAYER_OWNER_SLOT: usize = 0;

fn enemy_owner_slot(n: usize) -> usize {
    n + 1
}

/// How the current round is going.
#[derive(Clone, Copy, PartialEq, Default, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    #[default]
    Playing,
    /// Every enemy is a wreck.
    Won,
    /// The player is a wreck, or the frog died.
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
    /// Player/enemy ram contact dealt `damage` to both sides.
    Ram { enemy_slot: usize, damage: f32 },
    PickupCollected { slot: usize, kind: PickupKind },
    PickupRespawned { kind: PickupKind, x: f32, y: f32 },
    /// A projectile bounced off `slot`'s rainbow shield at (`x`, `y`).
    Deflected { slot: usize, x: f32, y: f32 },
    /// Two opposing shells met mid-air and cancelled at (`x`, `y`).
    ShellsCollided { x: f32, y: f32 },
    /// The frog bit the tank in `slot`.
    FrogBite { slot: usize, damage: f32, killed: bool },
    /// The wave scheduler called wave `wave` (1-based): `size` tanks of
    /// `tier` queued to roll in.
    WaveStarted { wave: u32, size: u32, tier: crate::level::Tier },
    /// A wave tank in `slot` finished rolling in: it now has a body and an
    /// `Ai`, and counts as an enemy on the field.
    TankEntered { slot: usize },
    /// A wave round removed the wreck in `slot` after
    /// `wave_wreck_despawn_seconds`.
    WreckRemoved { slot: usize },
    // --- AI decisions, recorded only while `Game::trace_ai` is set: each
    // is a transition the enemy phase observed by comparing an enemy's
    // `AiSnapshot` before and after its `think`, so `ai.rs` stays
    // snapshot-only and the recording adds nothing to the simulation. ---
    /// The behaviour tree settled on a different action than last frame.
    AiAction { slot: usize, from: Option<&'static str>, to: Option<&'static str> },
    /// The engagement ring gave `slot` a different slot index (see
    /// `engage::EngageSlot::index`; `None` = steering at the player).
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
}

/// What an `Event::Hit` landed on.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "what", rename_all = "snake_case")]
pub enum HitTarget {
    Player,
    Enemy { slot: usize },
    Frog,
    Obstacle,
    Wall,
}

/// Debug overlay switches `render` reads (dev builds only - see `game.rs`),
/// set by the dev server's `overlays` tool or cycled by the I key. Survive
/// restarts; all off by default.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Overlays {
    /// The hitbox/collider outlines and per-tank stat readout (`game.rs`'s
    /// `draw_tank_inspect`).
    pub inspect: bool,
    /// Blocked nav-grid cells.
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
        inspect: false,
        nav_grid: false,
        ai: false,
        projectiles: false,
        engage: false,
        pickups: false,
    };
    /// Only the inspect layer.
    pub const INSPECT: Overlays = Overlays {
        inspect: true,
        ..Overlays::NONE
    };
    /// Every overlay on.
    pub const ALL: Overlays = Overlays {
        inspect: true,
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
    /// mix (the dev server's `overlays` tool) snaps to its next step: nothing
    /// on -> `INSPECT`, exactly `INSPECT` -> `ALL`, anything else -> `NONE`.
    pub fn next_preset(self) -> Overlays {
        if !self.any() {
            Overlays::INSPECT
        } else if self == Overlays::INSPECT {
            Overlays::ALL
        } else {
            Overlays::NONE
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

#[derive(Default)]
pub struct Game {
    /// Every entity: tanks (a `Tank`; enemies also carry an `Ai`, which is
    /// what distinguishes them - the player never does), the `Frog`,
    /// `Obstacle`s, `Pickup`s and in-flight `Shell`/`Bullet`/`Plasma`
    /// projectiles. `pub(crate)` so `render` and the map linter can read it.
    pub(crate) world: hecs::World,
    /// The player's entity. `None` only before the first `init`; every
    /// method treats it as `Some` once a round is running.
    pub(crate) player: Option<Entity>,
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
    /// Engagement-slot assignment with per-tank memory - see `engage`.
    engage: EngageRing,
    /// The screen-distortion ring from the most recent kill (tank or
    /// frog), while it plays. Driven into the shockwave shader by `render`.
    pub(crate) shock: Option<Shockwave>,
    /// Heat-haze ripples at recently fired muzzles, oldest first.
    pub(crate) muzzle_flashes: Vec<Shockwave>,
    /// Impact ripples where projectiles landed, oldest first.
    pub(crate) impact_flashes: Vec<Shockwave>,
    /// Laser beams still in their short display window, oldest first.
    pub(crate) laser_beams: Vec<LaserBeam>,
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
    /// Last frame's raw fire-key state, for edge-detecting a fresh press.
    player_fire_held_last_frame: bool,
    /// `update` calls this round (paused frames included); reset by `init`.
    pub(crate) frame: u64,
    /// What happened during the most recent `update` - see `Event`.
    pub(crate) events: Vec<Event>,
    /// What the last enemy phase's engagement-slot assignment decided (every
    /// enemy's status, slot and target, the slot table), kept for the
    /// `engage` overlay, the debug snapshot and the AI event diff.
    pub(crate) last_engage: EngageReport,
    /// Owner slots the dev server asked to kill; applied by
    /// `apply_debug_kills` at the top of the next playing frame, so the
    /// kill runs through the normal explosion/round-end path.
    pub(crate) debug_kills: Vec<usize>,
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
    /// Tanks destroyed this frame: (position, victim was an enemy, owner slot).
    kills: Vec<(Position, bool, usize)>,
    pending_shells: Vec<Shell>,
    pending_plasmas: Vec<Plasma>,
    pending_bullets: Vec<Bullet>,
    pending_lasers: Vec<PendingLaserShot>,
    muzzle_flashes: Vec<Shockwave>,
    impact_flashes: Vec<Shockwave>,
    shock: Option<Shockwave>,
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
            pending_shells: Vec::new(),
            pending_plasmas: Vec::new(),
            pending_bullets: Vec::new(),
            pending_lasers: Vec::new(),
            muzzle_flashes: Vec::new(),
            impact_flashes: Vec::new(),
            shock: None,
            physics_stepped: false,
            events: Vec::new(),
        }
    }
}

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
        self.engage.clear();
        self.pickup_respawn_timer = tuning().pickup_respawn_seconds;
        self.player_fire_held_last_frame = false;
        self.shock = None;
        self.muzzle_flashes.clear();
        self.impact_flashes.clear();
        self.laser_beams.clear();
        self.frame = 0;
        self.last_engage.clear();
        self.debug_kills.clear();
        self.frog = None;
        self.enemy_frog = None;
        self.mission = self.level_overrides.resolve_mission(&self.map.mission);
        self.spawn_plan = self.level_overrides.resolve_spawn(&self.map.spawn, self.enemy_count_override);
        self.intro_timer = if self.show_intro { tuning().mission_banner_seconds } else { 0.0 };
        self.intro_fade = 0.0;

        self.physics = Physics::new();
        self.physics_accumulator = 0.0;
        battlefield::spawn_walls(&mut self.physics, width, height);

        // --- Player ---
        let row = self
            .player_row_override
            .unwrap_or_else(|| rng.random_range(0..TANK_VARIANTS));
        // The map's start cell, else the nearest non-wall cell to the
        // center so a wall at the center doesn't spawn the player inside it.
        let start_cell = self.map.start_cell().unwrap_or_else(|| {
            let (center_col, center_row) = map::world_to_cell(Position::new(width / 2.0, height / 2.0));
            self.map.nearest_free_cell(center_col, center_row)
        });
        let mut tank = Tank {
            row,
            shell_variant: TANK_SHELL_VARIANT_BY_ROW[row as usize],
            damage_variant: rng.random_range(0..DAMAGE_VARIANTS),
            position: map::cell_to_world(start_cell.0, start_cell.1),
            owner_slot: PLAYER_OWNER_SLOT,
            ..Tank::default()
        };
        if rng.random_range(0.0..1.0) < tuning().spawn_shield_chance {
            tank.shield_timer = tuning().shield_duration_seconds;
        }
        roll_track_distortion(&mut tank, &mut rng);
        // Spawn facing up (rotation 0): the Y-axis collider orientation.
        tank.body = Some(self.physics.spawn_tank(tank.position, tank.move_half_extents(false), tank.mass()));
        let center = tank.position;
        // Keeps enemies off the player's spawn point, and enemies off each
        // other so a crowded round doesn't start with tanks ramming.
        let clear = tank.size() * 2.0;
        let enemy_clear = tank.size() * 1.5;
        self.player = Some(self.world.spawn((tank,)));

        // --- Map terrain (walls/road/frog/pickup slots) ---
        // Before enemies, so their clearance check below sees every wall.
        let obstacle_half_extent = OBSTACLE_TEXTURE_SIZE * OBSTACLE_SCALE * OBSTACLE_HULL_FRACTION * 0.5;
        let map_spawn = battlefield::spawn_from_map(&mut self.physics, &mut self.world, &mut rng, &self.map, obstacle_half_extent);
        let obstacle_positions = map_spawn.obstacle_positions;
        let map_road_cells = map_spawn.road_cells;
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
                // Free-form user input: 0 would be an instantly won round
                // restarting forever, and the cap keeps live slots small.
                .clamp(1, 31),
            SpawnPlan::Waves { .. } => 0,
        };
        self.wave = WaveState::new(enemy_owner_slot(enemy_count));

        // Terrain legality is the nav grid's `usable` test (see
        // `battlefield::enemy_spawn_legal`); the frog is not down yet, so
        // this grid is walls only - the same grid `relocate_unusable_spawns`
        // audits against below.
        let spawn_grid = self.nav_grid(width, height);
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
                )
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
                spawn_grid.nearest_open(sample, &avoid, enemy_clear)
            });
            let erow = TANK_SPRITE_ORDER[enemy_positions.len() % TANK_SPRITE_ORDER.len()];
            let mut enemy = roll_enemy_tank(&mut rng, erow, pos, enemy_owner_slot(enemy_positions.len()));
            enemy.body = Some(self.physics.spawn_tank(pos, enemy.move_half_extents(false), enemy.mass()));
            enemy_positions.push(pos);
            self.world.spawn((enemy, Ai::default()));
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
                    && enemy_positions.iter().all(|&p| pos.distance_to(p) >= enemy_clear)
                    && obstacle_positions.iter().all(|&p| pos.distance_to(p) >= frog_clear)
            })
            .unwrap_or_else(|| {
                // Attempt cap: snap to the nearest usable nav cell clear of
                // every tank rather than accept a sample inside a wall.
                let sample = Position::new(width * 0.5, height * 0.5);
                let mut avoid = enemy_positions.clone();
                avoid.push(center);
                spawn_grid.nearest_open(sample, &avoid, enemy_clear)
            })
        });
        let frog_variant = rng.random_range(0..crate::frog::FROG_VARIANT_DIRS.len() as i32);
        if self.mission.has_player_frog() {
            let frog_body = self.physics.spawn_static(
                frog_pos,
                Position::new(FROG_COLLIDER_HALF_EXTENT.0, FROG_COLLIDER_HALF_EXTENT.1),
            );
            self.frog = Some(self.world.spawn((Frog {
                position: frog_pos,
                health: tuning().frog_max_health,
                max_health: tuning().frog_max_health,
                variant: frog_variant,
                body: frog_body,
                hurt_timer: 0.0,
                hit_flash_timer: 0.0,
                hop_timer: 0.0,
                hop_start: frog_pos,
                hop_end: frog_pos,
                hop_cooldown: 0.0,
                attack_timer: 0.0,
                attack_cooldown: 0.0,
                death_elapsed: None,
            },)));
        }

        // --- Pickups: every map slot spawns immediately ---
        for &(pos, kind) in &self.map_pickup_slots {
            spawn_pickup_at(&mut self.world, pos, kind);
        }
        // Bonus shields roll only once every slot is placed, so a bonus
        // can't land on a slot that hasn't spawned yet.
        for &(pos, kind) in &self.map_pickup_slots {
            if kind == PickupKind::Health {
                maybe_spawn_bonus_shield(&mut self.world, &self.map, pos, width, height, &mut rng);
            }
        }

        // --- Ground: road under every wall tile and explicit road cell ---
        let mut road_cells = obstacle_positions;
        road_cells.extend(map_road_cells);
        self.ground = crate::ground::build(width, height, rng.random(), &road_cells);

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
            let skip = input.player_intent.move_dir.is_some() || input.player_intent.fire;
            self.intro_timer = if skip { 0.0 } else { (self.intro_timer - dt).max(0.0) };
            if self.intro_timer <= 0.0 {
                self.intro_fade = INTRO_FADE_SECONDS;
            }
            return;
        }
        self.intro_fade = (self.intro_fade - dt).max(0.0);

        self.tick_effects(dt);
        let mut rng = self.rng.take().expect("rng seeded in init");
        self.time += dt;
        self.tick_timers(dt, &mut rng);
        let terrain = Terrain::build(&self.world, width, height);
        let mut f = Frame::new(dt, width, height, rng, terrain);

        if self.outcome == Outcome::Playing {
            self.apply_debug_kills(&mut f);
            self.frog_phase(&mut f);
            self.pickup_phase(&mut f);
            self.player_phase(input, &mut f);
            self.rollin_phase(&mut f);
            self.enemy_phase(&mut f);
            self.wave_phase(&mut f);
            self.spawn_pending(&mut f);
            self.resolve_lasers(&mut f);
            self.step_world(&mut f, true);
            self.sync_tanks_and_ram(&mut f);
            self.shell_vs_shell(&mut f);
            self.resolve_projectiles::<Shell>(&mut f, true);
            self.resolve_projectiles::<Bullet>(&mut f, true);
            self.resolve_projectiles::<Plasma>(&mut f, true);
            self.explosions(&mut f);
            self.despawn_wrecks(&mut f);
            self.cleanup_done();
            self.check_round_end(&mut f);
        } else {
            // Round over: the scene keeps animating (wrecks burn, in-flight
            // shots land without dealing damage) while the restart counts
            // down. Physics doesn't step, so nothing drifts.
            self.step_world(&mut f, false);
            self.resolve_projectiles::<Shell>(&mut f, false);
            self.resolve_projectiles::<Bullet>(&mut f, false);
            self.resolve_projectiles::<Plasma>(&mut f, false);
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
        if f.shock.is_some() {
            self.shock = f.shock;
        }
        self.events.extend(f.events);
        self.rng = Some(f.rng);
    }

    /// Age the shader effects (shockwave, muzzle/impact flashes, laser
    /// beams) - runs even on the end screen so nothing freezes mid-fade.
    fn tick_effects(&mut self, dt: f32) {
        if let Some(shock) = &mut self.shock {
            shock.time += dt;
            if shock.time >= tuning().shockwave_duration {
                self.shock = None;
            }
        }
        self.muzzle_flashes.retain_mut(|flash| {
            flash.time += dt;
            flash.time < tuning().muzzle_flash_duration
        });
        self.impact_flashes.retain_mut(|flash| {
            flash.time += dt;
            flash.time < tuning().impact_flash_duration
        });
        self.laser_beams.retain_mut(|beam| !beam.tick(dt));
    }

    /// Per-entity timers: every tank's cooldowns/recharge/wreck burn,
    /// obstacles' burn, the frog's animation and in-flight hop (its static
    /// body follows `position`), and track fade.
    fn tick_timers(&mut self, dt: f32, rng: &mut SmallRng) {
        for tank in self.world.query::<&mut Tank>().iter() {
            tank.tick_recharge(dt);
            tank.fire_cooldown = (tank.fire_cooldown - dt).max(0.0);
            tank.ram_cooldown = (tank.ram_cooldown - dt).max(0.0);
            tank.hit_flash_timer = (tank.hit_flash_timer - dt).max(0.0);
            tank.speed_boost_timer = (tank.speed_boost_timer - dt).max(0.0);
            tank.shield_timer = (tank.shield_timer - dt).max(0.0);
            tank.tick_wreck(dt);
            tank.tick_minigun_spin(dt);
            roll_wreck_col(tank, rng);
        }
        for obstacle in self.world.query::<&mut Obstacle>().iter() {
            obstacle.tick_burn(dt);
        }
        for frog in self.world.query::<&mut Frog>().iter() {
            frog.tick(dt);
            self.physics.set_position(frog.body, frog.position);
        }
        self.tracks.retain_mut(|t| !t.tick(dt));
    }

    /// The frog bites the single nearest live tank within its attack range
    /// (either side; never the killing blow on the player - it is a hazard,
    /// not a fair fight) and, independently, hops away from the nearest
    /// tank within its wider avoid range. Each on its own cooldown.
    fn frog_phase(&mut self, f: &mut Frame) {
        let player = self.player.expect("player entity spawned in init");
        let Some(frog_entity) = self.frog else { return };
        let (can_attack, can_hop, frog_pos, attack_range, avoid_range, hop_distance) =
            with_frog(&self.world, frog_entity, |fr| {
                (fr.can_attack(), fr.can_hop(), fr.position, fr.attack_range(), fr.avoid_range(), fr.hop_distance())
            });
        let nearest: Option<(Entity, Position, f32)> = self
            .world
            .query::<(Entity, &Tank)>()
            .without::<&RollIn>()
            .iter()
            .filter(|(_, t)| !t.is_wreck())
            .map(|(e, t)| (e, t.position, t.position.distance_to(frog_pos)))
            .min_by(|a, b| a.2.total_cmp(&b.2));
        let Some((target, tank_pos, dist)) = nearest else { return };

        if can_attack && dist <= attack_range {
            let dmg = f.rng.random_range(tuning().frog_attack_damage_min..tuning().frog_attack_damage_max);
            let cap = if target == player { MAX_DAMAGE - 1.0 } else { MAX_DAMAGE };
            let (became_wreck, victim_pos, victim_slot) = {
                let mut q = self.world.query_one::<&mut Tank>(target);
                let tank = q.get().expect("attack target always has a Tank");
                tank.take_damage(dmg, cap);
                tank.mark_hit();
                (tank.is_wreck(), tank.position, tank.owner_slot)
            };
            f.events.push(Event::FrogBite { slot: victim_slot, damage: dmg, killed: became_wreck });
            if became_wreck {
                f.kills.push((victim_pos, target != player, victim_slot));
            }
            with_frog_mut(&self.world, frog_entity, |fr| fr.start_attack());
        }

        if can_hop && dist <= avoid_range {
            let away = Vector2::new(frog_pos.x - tank_pos.x, frog_pos.y - tank_pos.y);
            let obstacles = f.terrain.obstacle_centers();
            if let Some(new_pos) = frog_hop_target(&mut f.rng, frog_pos, away, hop_distance, &obstacles, f.width, f.height) {
                with_frog_mut(&self.world, frog_entity, |fr| fr.start_hop(new_pos));
            }
        }
    }

    /// Pickups: any live tank within PICKUP_COLLECT_RADIUS collects (pure
    /// proximity, no physics), then the field is topped back up to the
    /// map's slot count after PICKUP_RESPAWN_SECONDS.
    fn pickup_phase(&mut self, f: &mut Frame) {
        let living_tanks: Vec<(Entity, Position)> = self
            .world
            .query::<(Entity, &Tank)>()
            .without::<&RollIn>()
            .iter()
            .filter(|(_, t)| !t.is_wreck())
            .map(|(e, t)| (e, t.position))
            .collect();
        let collected: Vec<(Entity, Entity, PickupKind)> = self
            .world
            .query::<(Entity, &Pickup)>()
            .iter()
            .filter_map(|(pickup_entity, pickup)| {
                living_tanks
                    .iter()
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
                        tank.shield_timer = tuning().shield_duration_seconds;
                    }
                }
                tank.owner_slot
            };
            f.events.push(Event::PickupCollected { slot, kind });
            self.world.despawn(pickup_entity).ok();
        }

        if slot_backed_count(&self.world, &self.map_pickup_slots) < self.map_pickup_slots.len() {
            self.pickup_respawn_timer -= f.dt;
            if self.pickup_respawn_timer <= 0.0 {
                let respawned =
                    respawn_from_slots(&mut self.world, &self.map, &self.map_pickup_slots, f.width, f.height, &mut f.rng);
                if let Some((pos, kind)) = respawned {
                    f.events.push(Event::PickupRespawned { kind, x: pos.x, y: pos.y });
                }
                self.pickup_respawn_timer = tuning().pickup_respawn_seconds;
            }
        } else {
            self.pickup_respawn_timer = tuning().pickup_respawn_seconds;
        }
    }

    /// Drive the player from this frame's input and handle its fire key.
    /// The player is never a wreck here: becoming one ends the round, and
    /// the round-over path never reaches this phase.
    fn player_phase(&mut self, input: Input, f: &mut Frame) {
        let player = self.player.expect("player entity spawned in init");
        let intent = input.player_intent;
        let mut q = self.world.query_one::<&mut Tank>(player);
        let tank = q.get().expect("player entity always has a Tank");

        drive_tank(&mut self.physics, tank, intent, f.dt);
        tick_queued_shots(&mut self.physics, f, tank, Owner::Player);

        // A laser or minigun is full-auto while the key is held (still
        // paced by `fire_cooldown`); shells and plasma fire once per
        // physical press, so a held key can never re-arm them.
        let fire_pressed = intent.fire && !self.player_fire_held_last_frame;
        self.player_fire_held_last_frame = intent.fire;
        let should_fire = match tank.active_weapon() {
            ActiveWeapon::Laser | ActiveWeapon::Minigun => intent.fire,
            ActiveWeapon::Plasma | ActiveWeapon::Shell => fire_pressed,
        };
        if should_fire && tank.fire_cooldown <= 0.0 {
            dispatch_fire(&mut self.physics, f, tank, Owner::Player, 0.0);
        }
    }

    /// Every enemy perceives (motion snapshot, nav grid, shared alert,
    /// engagement slot, pickups, line of sight), thinks, drives and fires.
    fn enemy_phase(&mut self, f: &mut Frame) {
        let player = self.player.expect("player entity spawned in init");
        let (movers, enemy_indices) = self.motion_snapshot();
        let grid = self.nav_grid(f.width, f.height);
        let components = grid.components();
        let player_pos = movers[0].position;

        // Shared aggression: any enemy seeing the player refreshes the
        // group's last-known position, so the rest converge instead of
        // patrolling blind.
        let alert_before = self.alert_position.filter(|_| self.alert_timer > 0.0);
        let any_enemy_sees_player = movers[1..]
            .iter()
            .any(|m| m.position.distance_to(player_pos) <= tuning().enemy_view_range);
        if any_enemy_sees_player {
            self.alert_position = Some(player_pos);
            self.alert_timer = tuning().enemy_alert_hold_seconds;
        } else {
            self.alert_timer = (self.alert_timer - f.dt).max(0.0);
            if self.alert_timer <= 0.0 {
                self.alert_position = None;
            }
        }
        let alert = self.alert_position.filter(|_| self.alert_timer > 0.0);
        if self.trace_ai && alert_before.is_some() != alert.is_some() {
            let p = alert.or(alert_before).unwrap_or(player_pos);
            f.events.push(Event::Alert { on: alert.is_some(), x: p.x, y: p.y });
        }

        // Engagement slots go to tanks that are really fighting: not
        // wrecked, fleeing or retreating, and either within view range or
        // hit-alerted (`engage_status`). Merely alert-following tanks still
        // far out don't claim one - that held approaching packs in loose
        // formation through the same bottleneck (measured via the probe's
        // clustering anomaly); steering at the raw alert point is fine for
        // them.
        let mut report = EngageReport::default();
        let mut engaged: Vec<(Entity, Position)> = Vec::new();
        for (entity, tank, ai) in self.world.query::<(Entity, &Tank, &Ai)>().iter() {
            let status = engage_status(tank, ai, player_pos);
            report.tanks.push(EngageTank::new(entity, tank.owner_slot, status));
            if status == EngageStatus::Engaged {
                engaged.push((entity, tank.position));
            }
        }
        report.tanks.sort_by_key(|t| t.owner);
        // Sorted by entity so the greedy claim order is stable frame to frame.
        engaged.sort_by_key(|(e, _)| *e);
        if engaged.len() >= 2 {
            let reachable = |a: Position, b: Position| components.connected(&grid, a, b);
            let line_of_sight = |a: Position, b: Position| f.terrain.line_of_sight(a, b);
            self.engage.assign(
                &engaged,
                &EngageCtx {
                    player_pos,
                    width: f.width,
                    height: f.height,
                    // One worst-case tank clear of the wall, plus a little.
                    margin: battlefield::max_tank_avoidance_radius() + 8.0,
                    reachable: &reachable,
                    line_of_sight: &line_of_sight,
                },
                &mut report,
            );
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

        for (entity, tank, ai) in self.world.query::<(Entity, &mut Tank, &mut Ai)>().iter() {
            let my_index = enemy_indices[&entity];
            let reach = tank.hull_size() * 0.5 + breach_reach_extra;
            let walls_ahead = Dir::ALL.map(|d| {
                f.terrain
                    .obstacle_ahead(tank.position, d.vec(), reach, breach_pad)
                    .map(|(material, burning)| WallAhead { material, burning })
            });
            // Real physics velocity, so the AI's stuck check can tell
            // "commanded to move" from "actually got somewhere that way".
            let real_velocity = tank
                .body
                .map(|handle| self.physics.velocity(handle))
                .unwrap_or_default();
            let engage_target = self.last_engage.target(entity);
            let line_of_sight = f.terrain.line_of_sight(tank.position, player_pos);
            let before = self.trace_ai.then(|| ai.snapshot());
            // The player lives in a different archetype (no `Ai`), so this
            // shared read never aliases the exclusive borrow above.
            let intent = with_tank(&self.world, player, |player_tank| {
                ai.think(
                    tank,
                    player_tank,
                    f.width,
                    f.height,
                    f.dt,
                    real_velocity,
                    &movers,
                    my_index,
                    &grid,
                    &mut f.rng,
                    alert,
                    engage_target,
                    &pickups,
                    line_of_sight,
                    walls_ahead,
                )
            });
            if let Some(before) = before {
                ai_transition_events(&mut f.events, tank.owner_slot, &before, &ai.snapshot());
            }
            drive_tank(&mut self.physics, tank, intent, f.dt);
            let owner = tank.owner();
            tick_queued_shots(&mut self.physics, f, tank, owner);
            // The AI paces itself with its own fire timer; `fire_cooldown`
            // is the weapon's own minimum (a burst in progress, say).
            if intent.fire && tank.fire_cooldown <= 0.0 {
                dispatch_fire(&mut self.physics, f, tank, owner, intent.fire_aim_offset);
            }
        }
    }

    /// Insert this frame's fired projectiles - only once no tank query is
    /// active, since hecs can't spawn into a world mid-iteration.
    fn spawn_pending(&mut self, f: &mut Frame) {
        for shell in f.pending_shells.drain(..) {
            self.world.spawn((shell,));
        }
        for plasma in f.pending_plasmas.drain(..) {
            self.world.spawn((plasma,));
        }
        for bullet in f.pending_bullets.drain(..) {
            self.world.spawn((bullet,));
        }
    }

    /// Lasers have no travel time: each queued beam is swept over its whole
    /// length right now, drawn up to where it stopped, and applied.
    fn resolve_lasers(&mut self, f: &mut Frame) {
        let player = self.player.expect("player entity spawned in init");
        let shots = std::mem::take(&mut f.pending_lasers);
        for shot in shots {
            let hit = f.terrain.sweep(&self.world, player, shot.owner, shot.start, shot.end, laser_beam_half_width());
            let (hit_pos, target) = match hit {
                Some((target, t)) => (shot.start + (shot.end - shot.start) * t, Some(target)),
                None => (shot.end, None),
            };
            f.muzzle_flashes.push(Shockwave { center: shot.start, time: 0.0 });
            self.laser_beams.push(LaserBeam::new(shot.start, hit_pos, shot.variant));
            let Some(target) = target else { continue };
            f.impact_flashes.push(Shockwave { center: hit_pos, time: 0.0 });
            // No knockback and no frog hop: an instant beam isn't something
            // to be shoved by or to dodge.
            self.apply_hit(f, target, hit_pos, laser_damage_range(&shot), HitEffects::none());
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
            if step_physics {
                self.physics.step();
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
    /// touching the player (`ram` gates on both cooldowns, so the player
    /// takes at most one ram hit per frame).
    fn sync_tanks_and_ram(&mut self, f: &mut Frame) {
        let player = self.player.expect("player entity spawned in init");
        let player_before = with_tank(&self.world, player, |t| t.position);
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
            with_tank_mut(&self.world, player, |t| lay_tracks(&mut self.tracks, t, player_before));
        }
        for (enemy, before) in enemies_before {
            let touching = with_tank(&self.world, enemy, |e| {
                with_tank(&self.world, player, |p| tanks_touching(&self.physics, e, p))
            });
            if touching {
                let rammed = with_two_tanks_mut(&mut self.world, enemy, player, |e, p| {
                    ram(e, true, p, false, &mut self.physics, &mut f.rng, &mut f.kills).map(|damage| (e.owner_slot, damage))
                });
                if let Some((enemy_slot, damage)) = rammed {
                    f.events.push(Event::Ram { enemy_slot, damage });
                }
            }
            if f.physics_stepped {
                with_tank_mut(&self.world, enemy, |t| lay_tracks(&mut self.tracks, t, before));
            }
        }
    }

    /// Two flying shells from opposing sides that meet mid-air detonate
    /// each other. Swept (closest approach over this frame's motion) rather
    /// than an end-of-frame overlap, since at SHELL_SPEED two shells closing
    /// head-on can pass through each other between frames. Same-side pairs
    /// (a twin volley, two different enemies' shells) never cancel.
    fn shell_vs_shell(&mut self, f: &mut Frame) {
        let flying: Vec<(Entity, Position, Vector2, Owner)> = self
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
            f.impact_flashes.push(Shockwave { center: midpoint, time: 0.0 });
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
        let player = self.player.expect("player entity spawned in init");
        struct Flight {
            entity: Entity,
            prev: Position,
            pos: Position,
            vel: Vector2,
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
            let Some((target, t)) = f.terrain.sweep(&self.world, player, owner, prev, pos, P::hit_half_extent()) else {
                continue;
            };
            let hit_pos = prev + (pos - prev) * t;
            f.impact_flashes.push(Shockwave { center: hit_pos, time: 0.0 });
            // A shielded tank bounces the projectile away instead of taking
            // the hit - see `Projectile::deflect`. No damage, no knockback,
            // no hit flash on the tank; the impact flash above still shows
            // where the shield was struck.
            let shield = match target {
                ShellTarget::PlayerTank => shield_deflector(&self.world, player),
                ShellTarget::EnemyTank(e) => shield_deflector(&self.world, e),
                _ => None,
            };
            if let Some((center, new_owner)) = shield {
                let mut q = self.world.query_one::<&mut P>(entity);
                q.get().expect("projectile collected this frame still exists").deflect(center, new_owner);
                f.events.push(Event::Deflected { slot: new_owner.slot(), x: hit_pos.x, y: hit_pos.y });
                continue;
            }
            let bounced = {
                let mut q = self.world.query_one::<&mut P>(entity);
                let p = q.get().expect("projectile collected this frame still exists");
                match target {
                    ShellTarget::Obstacle(e) => f.terrain.obstacle(e).is_some_and(|b| p.try_ricochet(b)),
                    _ => false,
                }
            };
            if bounced {
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
            let dir = Vector2::new(vel.x / len, vel.y / len);
            let effects = HitEffects {
                knockback: P::knockback_speed().map(|speed| (dir, speed)),
                frog_hop: P::frog_hops().then_some(vel),
            };
            self.apply_hit(f, target, hit_pos, dmg, effects);
        }
    }

    /// Every tank killed this frame gets a shockwave and an explosion; a
    /// splash that kills another tank is appended and handled in turn.
    /// Processed in kill order, so the last ring shown is the most recent
    /// kill's. Terminates: a tank can only ever be pushed once (every push
    /// is gated by its own transition into a wreck).
    fn explosions(&mut self, f: &mut Frame) {
        let mut i = 0;
        while i < f.kills.len() {
            let (center, victim_was_enemy, slot) = f.kills[i];
            i += 1;
            f.events.push(Event::Wreck { slot, x: center.x, y: center.y });
            f.shock = Some(Shockwave { center, time: 0.0 });
            self.apply_explosion(f, center, victim_was_enemy);
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
        for (entity, body) in destroyed {
            self.physics.remove_body(body);
            self.world.despawn(entity).ok();
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
        let player = self.player.expect("player entity spawned in init");
        let player_dead = with_tank(&self.world, player, |t| t.is_wreck());
        let frog_dead = |frog: Option<Entity>| frog.is_some_and(|e| with_frog(&self.world, e, Frog::is_dead));
        if player_dead || frog_dead(self.frog) {
            self.end_round(f, Outcome::Lost);
            return;
        }
        let won = match self.mission {
            Mission::Hunt => frog_dead(self.enemy_frog),
            Mission::Protect | Mission::Destroy => self.spawn_plan_finished() && self.all_enemies_wrecked(),
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

    /// Every tank's motion for the AI's predictive avoidance: slot 0 is the
    /// player, then the enemies, plus a map from enemy entity to slot (a
    /// later query over the same archetype has no guaranteed iteration
    /// order). Wrecks are included at zero velocity so tanks steer around
    /// them as fixed obstacles.
    fn motion_snapshot(&self) -> (Vec<Mover>, HashMap<Entity, usize>) {
        let to_mover = |t: &Tank| Mover {
            position: t.position,
            velocity: if t.is_wreck() { Position::new(0.0, 0.0) } else { t.velocity },
            radius: t.avoidance_radius(),
        };
        let player = self.player.expect("player entity spawned in init");
        let mut movers = Vec::new();
        with_tank(&self.world, player, |t| movers.push(to_mover(t)));
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
    pub(crate) fn nav_grid(&self, width: f32, height: f32) -> Grid {
        Grid::build(
            width,
            height,
            PATHFIND_CELL_SIZE,
            battlefield::max_tank_avoidance_radius(),
            self.world
                .query::<&Obstacle>()
                .iter()
                .map(|o| (o.position, o.hull_size() * 0.5))
                .chain(self.world.query::<&Frog>().iter().map(|fr| {
                    (fr.position, FROG_COLLIDER_HALF_EXTENT.0.max(FROG_COLLIDER_HALF_EXTENT.1))
                })),
        )
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

    /// Every tank's externally visible state, for headless inspection
    /// (`src/bin/probe.rs`, the tests below) without touching `world`.
    pub fn tank_snapshots(&self) -> Vec<TankSnapshot> {
        let player = self.player.expect("player entity spawned in init");
        self.world
            .query::<(Entity, &Tank)>()
            .iter()
            .map(|(entity, tank)| {
                // A tank still rolling in has no body: its kinematic
                // velocity stands in and it touches nothing.
                let contact = tank.body.map(|b| self.physics.contact_stats(b)).unwrap_or_default();
                TankSnapshot {
                    is_player: entity == player,
                    entering: tank.body.is_none(),
                    position: tank.position,
                    rotation: tank.rotation,
                    velocity: tank.body.map(|b| self.physics.velocity(b)).unwrap_or(tank.velocity),
                    commanded_velocity: tank.velocity,
                    top_speed: tank.base_speed(),
                    damage: tank.damage,
                    shells_ammo: tank.shells_ammo,
                    minigun_ammo: tank.minigun_ammo,
                    plasma_ammo: tank.plasma_ammo,
                    laser_charges: tank.laser_charges,
                    shield_timer: tank.shield_timer,
                    touching_static: contact.touching_static,
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
    pub is_player: bool,
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
    pub plasma_ammo: i32,
    pub laser_charges: i32,
    /// Seconds of rainbow shield left (0 = unshielded).
    pub shield_timer: f32,
    /// The hull has an active contact with static terrain right now.
    pub touching_static: bool,
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
fn drive_tank(physics: &mut Physics, tank: &mut Tank, intent: Intent, dt: f32) {
    let handle = tank.body.expect("tank should always have a physics body once spawned");
    let current = physics.velocity(handle);
    let facing_before = tank.rotation;

    tank.control(intent.move_dir, intent.face);
    tank.ease_visual_rotation(dt);
    tank.ease_turret_visual_rotation(dt);
    tank.ease_ring_position(dt);
    let target = tank.velocity;

    if tank.rotation != facing_before {
        physics.resize_collider(physics.collider_of(handle), tank.move_half_extents(tank.facing_along_x()));
    }

    let along_x = tank.facing_along_x();
    let (current_on, target_on, current_off) = if along_x {
        (current.x, target.x, current.y)
    } else {
        (current.y, target.y, current.x)
    };

    let want_on = target_on - current_on;
    let speeding_up = want_on * current_on >= 0.0;
    let delta_on = if speeding_up {
        let max_on = tuning().tank_accel_force * tank.speed_factor() / tank.mass() * dt;
        want_on.clamp(-max_on, max_on)
    } else {
        // Close a rate-controlled fraction of the remaining gap each frame
        // (frame-rate independent); snap the last sliver below
        // TANK_DECEL_SNAP_PX rather than trailing the asymptote forever.
        let rate = tuning().tank_decel_curve_rate * tank.speed_factor() / tank.mass();
        let remaining_gap = want_on * (-rate * dt).exp();
        if remaining_gap.abs() < tuning().tank_decel_snap_px { want_on } else { want_on - remaining_gap }
    };

    let max_off = tuning().tank_turn_grip_force / tank.mass() * dt;
    let delta_off = (-current_off).clamp(-max_off, max_off);

    let delta = if along_x {
        Position::new(delta_on, delta_off)
    } else {
        Position::new(delta_off, delta_on)
    };
    physics.apply_impulse(handle, Position::new(delta.x * tank.mass(), delta.y * tank.mass()));
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
/// with a bonus shield roll if that slot is a health pack. A no-op if every
/// slot is full.
fn respawn_from_slots(
    world: &mut hecs::World,
    map: &MapFile,
    slots: &[(Position, PickupKind)],
    width: f32,
    height: f32,
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
        maybe_spawn_bonus_shield(world, map, pos, width, height, rng);
    }
    Some((pos, kind))
}

/// Roll SHIELD_NEAR_HEALTH_CHANCE for the health slot just (re)spawned at
/// `slot` and, on success, drop a `PickupKind::Shield` in a free cell next
/// to it. Skipped while a shield already sits beside this slot, so repeated
/// health respawns can't pile them up.
fn maybe_spawn_bonus_shield(
    world: &mut hecs::World,
    map: &MapFile,
    slot: Position,
    width: f32,
    height: f32,
    rng: &mut SmallRng,
) {
    let occupied: Vec<Position> = world.query::<&Pickup>().iter().map(|p| p.position).collect();
    let already_there = world
        .query::<&Pickup>()
        .iter()
        .any(|p| p.kind == PickupKind::Shield && p.position.distance_to(slot) <= OBSTACLE_GRID_SIZE * 1.5);
    if already_there || rng.random_range(0.0..1.0) >= tuning().shield_near_health_chance {
        return;
    }
    if let Some(pos) = bonus_shield_cell(map, slot, &occupied, width, height, rng) {
        spawn_pickup_at(world, pos, PickupKind::Shield);
    }
}

/// A uniformly random free cell touching the slot at `slot`, or `None`
/// when all eight neighbours are taken. Eligible: empty or road in the map
/// (walls, the frog, the start cell and other slots are not), centre inside
/// the border walls (their inner faces sit at 0/`width`/0/`height`), and no
/// live pickup already on it. Map walls are checked rather than the live
/// obstacle set: a shot-away wall's cell stays off limits, which is
/// conservative but keeps this independent of the physics world.
fn bonus_shield_cell(
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
            let open = matches!(map.cell(c, r), None | Some(CellObject::Road));
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
        owner_slot: slot,
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
        enemy.shield_timer = tuning().shield_duration_seconds;
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

/// Roll a tank's wrecked-hull variant the first frame it is a wreck; a
/// no-op every other frame. Uses the round stream, never `rand::rng()`.
fn roll_wreck_col(tank: &mut Tank, rng: &mut SmallRng) {
    if tank.is_wreck() && tank.wreck_col.is_none() {
        tank.wreck_col = Some(TANK_WRECK_COLS[rng.random_range(0..TANK_WRECK_COLS.len())]);
    }
}

/// Lay tread marks along the distance a tank travelled this frame, one per
/// TRACK_SPACING px, and advance its tread-animation frame off the same
/// signal. Must only run on frames the physics stepped - otherwise a
/// stationary `before` reads as idle and resets the animation. Marks
/// follow the raw travel heading (not the snapped hull rotation), so a
/// real turn traces its real curve and a sideways shove leaves sideways
/// marks. Stops once the hull is disabled or wrecked.
fn lay_tracks(tracks: &mut Vec<Track>, tank: &mut Tank, before: Position) {
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
    // Unit vector pointing back along this frame's travel.
    let back = Vector2::new((before.x - tank.position.x) / moved, (before.y - tank.position.y) / moved);
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
        });
        tank.track_mark_count += 1;
    }
}

/// Whether an enemy competes for an engagement slot this frame, and if
/// not, why - the exclusions win over the range test.
fn engage_status(tank: &Tank, ai: &Ai, player_pos: Position) -> EngageStatus {
    if tank.is_wreck() {
        EngageStatus::Wreck
    } else if tank.damage >= tuning().enemy_flee_damage {
        EngageStatus::Fleeing
    } else if tank.active_weapon() == ActiveWeapon::Shell && ai.is_retreating() {
        EngageStatus::Retreating
    } else if tank.position.distance_to(player_pos) <= tuning().enemy_view_range || ai.is_hit_alerted() {
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
    /// snaps back to NONE.
    #[test]
    fn presets_cycle_and_mixes_snap() {
        assert_eq!(Overlays::default(), Overlays::NONE);
        assert_eq!(Overlays::NONE.next_preset(), Overlays::INSPECT);
        assert_eq!(Overlays::INSPECT.next_preset(), Overlays::ALL);
        assert_eq!(Overlays::ALL.next_preset(), Overlays::NONE);
        let mixed = Overlays {
            nav_grid: true,
            ..Overlays::NONE
        };
        assert!(mixed.any());
        assert_eq!(mixed.next_preset(), Overlays::NONE);
        assert!(!Overlays::NONE.any());
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
mod determinism_tests {
    use super::*;

    /// Run one seeded round headlessly for `frames` frames of AFK input at
    /// the probe's fixed timestep, sampling `tank_snapshots` every
    /// `sample_every` frames (plus frame 0).
    fn run_sampled(seed: u64, frames: u32, sample_every: u32) -> Vec<Vec<TankSnapshot>> {
        let mut game = Game::default();
        game.enemy_count_override = Some(4);
        game.seed_override = Some(seed);
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
    fn key(s: &TankSnapshot) -> (bool, [u32; 7], i32, i32, i32, bool) {
        (
            s.is_player,
            [
                s.position.x.to_bits(),
                s.position.y.to_bits(),
                s.velocity.x.to_bits(),
                s.velocity.y.to_bits(),
                s.rotation.to_bits(),
                s.damage.to_bits(),
                s.shield_timer.to_bits(),
            ],
            s.shells_ammo,
            s.minigun_ammo,
            s.plasma_ammo,
            s.is_wreck,
        )
    }

    /// Two full runs of the same seed must agree bit-for-bit. 600 frames
    /// of an AFK round crosses spawn, patrol, alert sharing, engagement
    /// and firing, so every RNG consumer gets exercised.
    #[test]
    fn same_seed_replays_bit_identical() {
        for seed in [0xB0B5_u64, 0xC0FFEE_u64] {
            let a = run_sampled(seed, 600, 60);
            let b = run_sampled(seed, 600, 60);
            assert_eq!(a.len(), b.len(), "seed {seed:#x}: sample counts differ");
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
        let slot = game.world.query::<&Tank>().with::<&Ai>().iter().map(|t| t.owner_slot).next().unwrap();
        game.debug_kill(slot).unwrap();
        step(&mut game, Input::default());
        assert_eq!(game.outcome(), Outcome::Won);
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
        input.player_intent.fire = true;
        step(&mut game, input);
        assert_eq!(game.intro_timer, 0.0, "fire skips the intro");
        step(&mut game, Input::default());
        assert!(game.time > 0.0);
    }

    fn player_ammo(game: &Game) -> i32 {
        game.tank_snapshots().iter().find(|t| t.is_player).expect("player").shells_ammo
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

    fn fire_once(row: i32) -> i32 {
        let mut game = game_on(OPEN_MAP, 1, Some(row));
        let fire = Input {
            player_intent: Intent { fire: true, ..Intent::default() },
            ..Input::default()
        };
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
        let fire = Input {
            player_intent: Intent { fire: true, ..Intent::default() },
            ..Input::default()
        };
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
                    Event::Hit { target: HitTarget::Obstacle, killed: true, .. } => broke = true,
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

    #[test]
    fn a_twin_barrel_shot_costs_two_shells_and_a_single_costs_one() {
        let full = tuning().max_shells;
        assert_eq!(fire_once(1), full - 2, "row 1 (assault) is twin-barrel");
        assert_eq!(fire_once(0), full - 1, "row 0 (scout) is single-barrel");
    }

    #[test]
    fn a_held_fire_key_fires_shells_only_once() {
        let mut game = game_on(OPEN_MAP, 1, Some(0));
        let fire = Input {
            player_intent: Intent { fire: true, ..Intent::default() },
            ..Input::default()
        };
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
    fn a_shield_pickup_heals_to_full_and_protects_for_its_duration() {
        let mut game = game_on(SEALED_SHIELD_MAP, 1, Some(0));
        let player = game.player.expect("player");
        {
            let mut q = game.world.query_one::<&mut Tank>(player);
            let tank = q.get().expect("player tank");
            tank.damage = 50.0;
            tank.shield_timer = 0.0;
        }
        step(&mut game, Input::default());
        let snap = player_snapshot(&game);
        assert_eq!(snap.damage, 0.0, "the shield pickup is a full heal");
        let duration = tuning().shield_duration_seconds;
        assert!(
            (snap.shield_timer - duration).abs() <= 1.0 / 60.0 + 1e-4,
            "timer set to tuning().shield_duration_seconds, got {}",
            snap.shield_timer
        );
        // Damage applied through the one damage path is swallowed while
        // the timer runs...
        {
            let mut q = game.world.query_one::<&mut Tank>(player);
            let tank = q.get().expect("player tank");
            tank.take_damage(30.0, MAX_DAMAGE);
            assert_eq!(tank.damage, 0.0);
        }
        // ...and the timer counts down to exactly zero and stays there. The
        // slot is cleared first so the crate can't respawn into the sealed
        // ring and refresh the timer mid-countdown.
        game.map_pickup_slots.clear();
        for _ in 0..((duration * 60.0) as u32 + 5) {
            step(&mut game, Input::default());
        }
        assert_eq!(player_snapshot(&game).shield_timer, 0.0);
        {
            let mut q = game.world.query_one::<&mut Tank>(player);
            let tank = q.get().expect("player tank");
            tank.take_damage(30.0, MAX_DAMAGE);
            assert_eq!(tank.damage, 30.0, "unshielded again once the timer runs out");
        }
    }

    /// Drop an enemy-owned shell `above` px above the player, flying
    /// straight down at it, and run `frames` frames. Returns whether a
    /// shell ever ended up owned by the player (i.e. was deflected) and the
    /// player's damage at the end.
    fn shell_from_above(shielded: bool, above: f32, frames: u32) -> (bool, f32) {
        let mut game = game_on(OPEN_MAP, 0, Some(0));
        let player = game.player.expect("player");
        let (pos, row) = {
            let mut q = game.world.query_one::<&mut Tank>(player);
            let tank = q.get().expect("player tank");
            tank.shield_timer = if shielded { 100.0 } else { 0.0 };
            (tank.position, tank.row)
        };
        let shooter = Tank { row, position: Position::new(pos.x, pos.y - above), rotation: 180.0, ..Tank::default() };
        let shell = Shell::spawn(&shooter, Owner::Enemy(0), 0.0, 0.0);
        game.world.spawn((shell,));
        let mut deflected = false;
        for _ in 0..frames {
            step(&mut game, Input::default());
            deflected |= game.world.query::<&Shell>().iter().any(|s| s.owner == Owner::Player && s.velocity.y < 0.0);
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

    #[test]
    fn a_bonus_shield_lands_on_a_free_neighbouring_cell() {
        let map = MapFile::from_toml_str(OPEN_MAP).expect("map parses");
        let slot = map::cell_to_world(20, 11);
        let mut rng = SmallRng::seed_from_u64(3);
        // The slot itself is occupied by its health pack; every neighbour is
        // free and inside the field.
        let pos = bonus_shield_cell(&map, slot, &[slot], W, H, &mut rng).expect("an open map has a free neighbour");
        let (c, r) = map::world_to_cell(pos);
        assert!((c - 20).abs() <= 1 && (r - 11).abs() <= 1 && (c, r) != (20, 11), "adjacent, not the slot: {c},{r}");
        assert!(matches!(map.cell(c, r), None | Some(CellObject::Road)));
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
        assert_eq!(bonus_shield_cell(&map, slot, &occupied, W, H, &mut rng), None);
    }
}
