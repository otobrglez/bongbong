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
mod engage;
mod hits;
mod weapons;

use crate::tuning::tuning;
use std::collections::{HashMap, HashSet};

use hecs::Entity;
use rand::rngs::SmallRng;
use rand::{RngExt, SeedableRng};
use sola_raylib::core::math::Vector2;

use crate::ai::{Ai, Intent, Mover};
use crate::battlefield;
use crate::bullet::Bullet;
use crate::frog::Frog;
use crate::laser::{LaserBeam, LaserVariant};
use crate::map::{self, MapFile};
use crate::obstacle::Obstacle;
use crate::pathfind::Grid;
use crate::physics::Physics;
use crate::pickup::{Pickup, PickupKind};
use crate::plasma::{Plasma, PlasmaVariant};
use crate::shell::{Owner, Shell, ShellState};
use crate::shockwave::Shockwave;
use crate::tank::{ActiveWeapon, Tank};
use crate::track::Track;
use crate::{
    DAMAGE_VARIANTS,
    FROG_COLLIDER_HALF_EXTENT,
    MAX_DAMAGE,
    OBSTACLE_CLEAR,
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
use engage::{EngageCtx, EngageRing};
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
    /// Toggles the debug inspect overlay - see `Game::inspect_enabled`.
    pub toggle_inspect_pressed: bool,
}

/// The player tank's owner slot (`Tank::owner_slot`); enemies take `n + 1`.
pub(crate) const PLAYER_OWNER_SLOT: usize = 0;

fn enemy_owner_slot(n: usize) -> usize {
    n + 1
}

/// How the current round is going.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub enum Outcome {
    #[default]
    Playing,
    /// Every enemy is a wreck.
    Won,
    /// The player is a wreck, or the frog died.
    Lost,
}

/// scifi_tanks_sheet.png has 12 row-variants, one archetype per row (see
/// docs/SPRITESHEET_SPEC.md §4). Rows 1, 4, 7, 9 and 10 are twin-barrel.
const TANK_VARIANTS: i32 = 12;

/// Enemy spawn order over the 12 variants, alternating twin- and
/// single-barrel chassis so both kinds appear early however few tanks
/// are on the field.
const TANK_SPRITE_ORDER: [i32; 12] = [1, 0, 4, 2, 7, 3, 9, 5, 10, 6, 8, 11];

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
    /// The protect-objective frog's entity - same convention as `player`.
    pub(crate) frog: Option<Entity>,
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
    /// Debug inspect overlay (the "I" key): hit boxes plus a per-tank stat
    /// readout. Survives restarts; off by default.
    pub inspect_enabled: bool,
    /// The rapier world: tank bodies plus wall/obstacle/frog colliders.
    physics: Physics,
    /// Real time not yet consumed by a fixed physics step.
    physics_accumulator: f32,
    /// `--enemies`: pins the enemy count instead of the map's `tanks`
    /// default or a random roll. Set before the first `init`; persists
    /// across restarts.
    pub enemy_count_override: Option<usize>,
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
    /// Tanks destroyed this frame: (position, victim was an enemy).
    kills: Vec<(Position, bool)>,
    pending_shells: Vec<Shell>,
    pending_plasmas: Vec<Plasma>,
    pending_bullets: Vec<Bullet>,
    pending_lasers: Vec<PendingLaserShot>,
    muzzle_flashes: Vec<Shockwave>,
    impact_flashes: Vec<Shockwave>,
    shock: Option<Shockwave>,
    /// Whether at least one fixed physics step ran this frame.
    physics_stepped: bool,
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
        // `--enemies` wins, then the map's `tanks`, then a random roll.
        let enemy_count = self.enemy_count_override.unwrap_or_else(|| {
            self.map
                .tanks
                .map(|n| n as usize)
                .unwrap_or_else(|| rng.random_range(tuning().enemy_count_min..=tuning().enemy_count_max))
        });
        // Both overrides are free-form user input: 0 would be an instantly
        // won round restarting forever, and the cap keeps owner slots small.
        let enemy_count = enemy_count.clamp(1, 31);

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
            // Per-enemy speed within +/- ENEMY_SPEED_VARIANCE, so they don't
            // move in lockstep.
            let factor = 1.0 + rng.random_range(-tuning().enemy_speed_variance..tuning().enemy_speed_variance);
            let owner_slot = enemy_owner_slot(enemy_positions.len());
            let mut enemy = Tank {
                row: erow,
                shell_variant: TANK_SHELL_VARIANT_BY_ROW[erow as usize],
                damage_variant: rng.random_range(0..DAMAGE_VARIANTS),
                position: pos,
                rotation: 180.0, // facing down, toward the player's start
                speed_scale: factor,
                owner_slot,
                ..Tank::default()
            };
            // Some enemies start armed with exactly one pickup's worth of a
            // special weapon (see ENEMY_SPECIAL_WEAPON_CHANCE).
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
            roll_track_distortion(&mut enemy, &mut rng);
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
        let frog_body = self.physics.spawn_static(
            frog_pos,
            Position::new(FROG_COLLIDER_HALF_EXTENT.0, FROG_COLLIDER_HALF_EXTENT.1),
        );
        self.frog = Some(self.world.spawn((Frog {
            position: frog_pos,
            health: tuning().frog_max_health,
            max_health: tuning().frog_max_health,
            variant: rng.random_range(0..crate::frog::FROG_VARIANT_DIRS.len() as i32),
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

        // --- Pickups: every map slot spawns immediately ---
        for &(pos, kind) in &self.map_pickup_slots {
            spawn_pickup_at(&mut self.world, pos, kind);
        }

        // --- Ground: road under every wall tile and explicit road cell ---
        let mut road_cells = obstacle_positions;
        road_cells.extend(map_road_cells);
        self.ground = crate::ground::build(width, height, rng.random(), &road_cells);

        self.rng = Some(rng);
    }

    /// Step the simulation one frame: `input` is this frame's player input,
    /// `dt` its elapsed seconds, `width`/`height` the battlefield size.
    pub fn update(&mut self, input: Input, dt: f32, width: f32, height: f32) {
        debug_assert!(self.rng.is_some(), "Game::rng missing at update entry - init has not run");
        if input.pause_pressed {
            self.paused = !self.paused;
        }
        if input.toggle_shadows_pressed {
            self.shadows_enabled = !self.shadows_enabled;
        }
        if input.toggle_inspect_pressed {
            self.inspect_enabled = !self.inspect_enabled;
        }
        // Debug restart: any time, including paused or on the end screen.
        if input.restart_pressed {
            self.init(width, height);
            return;
        }
        if self.paused {
            return;
        }

        self.tick_effects(dt);
        let mut rng = self.rng.take().expect("rng seeded in init");
        self.time += dt;
        self.tick_timers(dt, &mut rng);
        let terrain = Terrain::build(&self.world, width, height);
        let mut f = Frame::new(dt, width, height, rng, terrain);

        if self.outcome == Outcome::Playing {
            self.frog_phase(&mut f);
            self.pickup_phase(&mut f);
            self.player_phase(input, &mut f);
            self.enemy_phase(&mut f);
            self.spawn_pending(&mut f);
            self.resolve_lasers(&mut f);
            self.step_world(&mut f, true);
            self.sync_tanks_and_ram(&mut f);
            self.shell_vs_shell(&mut f);
            self.resolve_projectiles::<Shell>(&mut f, true);
            self.resolve_projectiles::<Bullet>(&mut f, true);
            self.resolve_projectiles::<Plasma>(&mut f, true);
            self.explosions(&mut f);
            self.cleanup_done();
            self.check_round_end();
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

    /// Append the frame's effects and put the RNG back.
    fn finish_frame(&mut self, f: Frame) {
        self.muzzle_flashes.extend(f.muzzle_flashes);
        self.impact_flashes.extend(f.impact_flashes);
        if f.shock.is_some() {
            self.shock = f.shock;
        }
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
        let frog_entity = self.frog.expect("frog entity spawned in init");
        let (can_attack, can_hop, frog_pos, attack_range, avoid_range, hop_distance) =
            with_frog(&self.world, frog_entity, |fr| {
                (fr.can_attack(), fr.can_hop(), fr.position, fr.attack_range(), fr.avoid_range(), fr.hop_distance())
            });
        let nearest: Option<(Entity, Position, f32)> = self
            .world
            .query::<(Entity, &Tank)>()
            .iter()
            .filter(|(_, t)| !t.is_wreck())
            .map(|(e, t)| (e, t.position, t.position.distance_to(frog_pos)))
            .min_by(|a, b| a.2.total_cmp(&b.2));
        let Some((target, tank_pos, dist)) = nearest else { return };

        if can_attack && dist <= attack_range {
            let dmg = f.rng.random_range(tuning().frog_attack_damage_min..tuning().frog_attack_damage_max);
            let cap = if target == player { MAX_DAMAGE - 1.0 } else { MAX_DAMAGE };
            let (became_wreck, victim_pos) = {
                let mut q = self.world.query_one::<&mut Tank>(target);
                let tank = q.get().expect("attack target always has a Tank");
                tank.damage = (tank.damage + dmg).min(cap);
                tank.mark_hit();
                (tank.is_wreck(), tank.position)
            };
            if became_wreck {
                f.kills.push((victim_pos, target != player));
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
            {
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
                }
            }
            self.world.despawn(pickup_entity).ok();
        }

        if self.world.query::<&Pickup>().iter().count() < self.map_pickup_slots.len() {
            self.pickup_respawn_timer -= f.dt;
            if self.pickup_respawn_timer <= 0.0 {
                respawn_from_slots(&mut self.world, &mut f.rng, &self.map_pickup_slots);
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

        // Engagement slots go to tanks that are really fighting: not
        // wrecked, fleeing or retreating, and either within view range or
        // hit-alerted. Merely alert-following tanks still far out don't
        // claim one - that held approaching packs in loose formation
        // through the same bottleneck (measured via the probe's clustering
        // anomaly); steering at the raw alert point is fine for them.
        let mut engaged: Vec<(Entity, Position)> = Vec::new();
        for (entity, tank, ai) in self.world.query::<(Entity, &Tank, &Ai)>().iter() {
            let excluded = tank.is_wreck()
                || tank.damage >= tuning().enemy_flee_damage
                || (tank.active_weapon() == ActiveWeapon::Shell && ai.is_retreating());
            let near = tank.position.distance_to(player_pos) <= tuning().enemy_view_range || ai.is_hit_alerted();
            if !excluded && near {
                engaged.push((entity, tank.position));
            }
        }
        // Sorted by entity so the greedy claim order is stable frame to frame.
        engaged.sort_by_key(|(e, _)| *e);
        let engage_targets: HashMap<Entity, Position> = if engaged.len() >= 2 {
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
            )
        } else {
            HashMap::new()
        };

        let pickups: Vec<(PickupKind, Position)> = self
            .world
            .query::<&Pickup>()
            .iter()
            .map(|p| (p.kind, p.position))
            .collect();

        for (entity, tank, ai) in self.world.query::<(Entity, &mut Tank, &mut Ai)>().iter() {
            let my_index = enemy_indices[&entity];
            // Real physics speed, so the AI's stuck check can tell
            // "commanded to move" from "actually moved".
            let real_speed = tank
                .body
                .map(|handle| {
                    let v = self.physics.velocity(handle);
                    (v.x * v.x + v.y * v.y).sqrt()
                })
                .unwrap_or(0.0);
            let engage_target = engage_targets.get(&entity).copied();
            let line_of_sight = f.terrain.line_of_sight(tank.position, player_pos);
            // The player lives in a different archetype (no `Ai`), so this
            // shared read never aliases the exclusive borrow above.
            let intent = with_tank(&self.world, player, |player_tank| {
                ai.think(
                    tank,
                    player_tank,
                    f.width,
                    f.height,
                    f.dt,
                    real_speed,
                    &movers,
                    my_index,
                    &grid,
                    &mut f.rng,
                    alert,
                    engage_target,
                    &pickups,
                    line_of_sight,
                )
            });
            if std::env::var_os("BB_AI_TRACE").is_some() { eprintln!("AITRACE idx={} pos=({:.1},{:.1}) rspeed={:.1} mv={:?} face={:?} {}", my_index, tank.position.x, tank.position.y, real_speed, intent.move_dir.map(|d| d.rotation()), intent.face.map(|d| d.rotation()), ai.trace_state()); }
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
            self.apply_hit(f, target, laser_damage_range(&shot), HitEffects::none());
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
                with_two_tanks_mut(&mut self.world, enemy, player, |e, p| {
                    ram(e, true, p, false, &mut self.physics, &mut f.rng, &mut f.kills);
                });
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
            self.apply_hit(f, target, dmg, effects);
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
            let (center, victim_was_enemy) = f.kills[i];
            i += 1;
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

    /// Losing (player or frog dead) takes precedence over winning when
    /// both happen on the same frame.
    fn check_round_end(&mut self) {
        let player = self.player.expect("player entity spawned in init");
        let frog = self.frog.expect("frog entity spawned in init");
        if with_tank(&self.world, player, |t| t.is_wreck()) || with_frog(&self.world, frog, Frog::is_dead) {
            self.end_round(Outcome::Lost);
        } else if self.world.query::<&Tank>().with::<&Ai>().iter().all(|t| t.is_wreck()) {
            self.end_round(Outcome::Won);
        }
    }

    fn end_round(&mut self, outcome: Outcome) {
        self.outcome = outcome;
        self.restart_timer = tuning().restart_delay;
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

    /// Every tank's externally visible state, for headless inspection
    /// (`src/bin/probe.rs`, the tests below) without touching `world`.
    pub fn tank_snapshots(&self) -> Vec<TankSnapshot> {
        let player = self.player.expect("player entity spawned in init");
        self.world
            .query::<(Entity, &Tank)>()
            .iter()
            .map(|(entity, tank)| {
                let body = tank.body.expect("tank should always have a physics body once spawned");
                let contact = self.physics.contact_stats(body);
                TankSnapshot {
                    is_player: entity == player,
                    position: tank.position,
                    rotation: tank.rotation,
                    velocity: self.physics.velocity(body),
                    commanded_velocity: tank.velocity,
                    top_speed: tank.base_speed(),
                    damage: tank.damage,
                    shells_ammo: tank.shells_ammo,
                    minigun_ammo: tank.minigun_ammo,
                    plasma_ammo: tank.plasma_ammo,
                    laser_charges: tank.laser_charges,
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

fn sync_tank_from_physics(physics: &Physics, tank: &mut Tank) {
    let handle = tank.body.expect("tank should always have a physics body once spawned");
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

/// Top up one pickup at a uniformly random slot not currently occupied.
/// A no-op if every slot is full.
fn respawn_from_slots(world: &mut hecs::World, rng: &mut SmallRng, slots: &[(Position, PickupKind)]) {
    let occupied: Vec<Position> = world.query::<&Pickup>().iter().map(|p| p.position).collect();
    let free: Vec<(Position, PickupKind)> = slots
        .iter()
        .copied()
        .filter(|&(pos, _)| occupied.iter().all(|&p| p.distance_to(pos) > 0.5))
        .collect();
    if free.is_empty() {
        return;
    }
    let (pos, kind) = free[rng.random_range(0..free.len())];
    spawn_pickup_at(world, pos, kind);
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

#[cfg(test)]
mod spawn_tests {
    use super::*;
    use crate::OBSTACLE_GRID_SIZE;

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
    fn key(s: &TankSnapshot) -> (bool, [u32; 6], i32, i32, i32, bool) {
        (
            s.is_player,
            [
                s.position.x.to_bits(),
                s.position.y.to_bits(),
                s.velocity.x.to_bits(),
                s.velocity.y.to_bits(),
                s.rotation.to_bits(),
                s.damage.to_bits(),
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
}
