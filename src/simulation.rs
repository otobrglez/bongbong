 //! The simulation layer: everything that decides what the game *is* doing
//! this frame - physics, damage, AI, spawning - with no dependency on a
//! window, a `RaylibHandle`, or anything else that only makes sense once
//! there's a screen to draw to. `game.rs` (the presentation layer) reads
//! `Game`'s state afterward and draws it; this module never reaches back
//! the other way.
//!
//! Concretely: `Game::init`/`Game::update` take plain numbers and an
//! `Input` snapshot, not a `RaylibHandle` - so driving a round forward
//! (`init` a fresh one, `update` it frame by frame) needs nothing but this
//! module and a way to produce `Input`s, which is what makes "cleanly
//! pause, replay, or predict game states without bringing the renderer
//! into it" (see docs/housekeeping-ideas-0.0.2-agy.md) actually true rather
//! than aspirational. One honest caveat: `sola_raylib::core::math::Vector2`
//! (aliased `Position` in lib.rs) is still the math type used throughout -
//! that's a vector-math dependency, not a rendering one, and is imported by
//! name here rather than via `sola_raylib::prelude::*` specifically so this
//! module's `use`s never so much as name a window/drawing type.
//!
//! A handful of `Game`'s fields (documented individually below) are
//! `pub(crate)` rather than fully private, purely so `game.rs::render`
//! can read them - `render` inherently needs to see simulation state to
//! draw it, in any split short of a full accessor API, which isn't
//! warranted at this scale.

use std::collections::{HashMap, HashSet};

use hecs::Entity;
use rand::RngExt;
use rapier2d::prelude::ColliderHandle;
use sola_raylib::core::math::Vector2;

use crate::ai::{Ai, Intent, Mover};
use crate::battlefield;
use crate::frog::Frog;
use crate::obstacle::Obstacle;
use crate::pathfind::Grid;
use crate::physics::{self, Physics};
use crate::pickup::{Pickup, PickupKind};
use crate::shell::{Owner, Shell, ShellState};
use crate::shockwave::Shockwave;
use crate::tank::Tank;
use crate::track::Track;
use crate::{
    DAMAGE_VARIANTS, ENEMY_ALERT_HOLD_SECONDS, ENEMY_COUNT_MAX, ENEMY_COUNT_MIN, ENEMY_DAMAGE_MAX,
    ENEMY_DAMAGE_MIN,
    ENEMY_SPAWN_MARGIN_MAX, ENEMY_SPAWN_MARGIN_MIN, ENEMY_SPEED, ENEMY_SPEED_VARIANCE,
    ENEMY_VIEW_RANGE,
    EXPLOSION_DAMAGE_MAX, EXPLOSION_DAMAGE_MIN, EXPLOSION_KNOCKBACK_SPEED, EXPLOSION_RADIUS,
    FROG_ATTACK_DAMAGE_MAX, FROG_ATTACK_DAMAGE_MIN, FROG_COLLIDER_HALF_EXTENT,
    FROG_HOP_ANGLE_FAN_DEG, FROG_HOP_ANGLE_JITTER_DEG, FROG_HOP_BOUNDS_MARGIN, FROG_MAX_HEALTH,
    FROG_SPAWN_MAX_DIST, FROG_SPAWN_MIN_DIST,
    IMPACT_FLASH_DURATION, KNOCKBACK_MAX_SPEED, KNOCKBACK_STRENGTH, MAX_DAMAGE,
    MUZZLE_FLASH_DURATION,
    OBSTACLE_CLEAR, OBSTACLE_HULL_FRACTION, OBSTACLE_SCALE, OBSTACLE_TEXTURE_SIZE,
    PATHFIND_CELL_SIZE, PHYSICS_FIXED_DT, PHYSICS_MAX_CATCHUP_SECONDS, PICKUP_AMMO_AMOUNT,
    PICKUP_COLLECT_RADIUS, PICKUP_COUNT, PICKUP_HEAL_AMOUNT, PICKUP_RESPAWN_SECONDS,
    PLAYER_DAMAGE_MAX, PLAYER_DAMAGE_MIN, PLAYER_FIRE_INTERVAL, Position,
    RAM_DAMAGE_COOLDOWN, RESTART_DELAY, SHELLS_HARD_CAP, SHELL_HIT_HALF_EXTENT,
    SHELL_IMPACT_KNOCKBACK_SPEED,
    SHELL_SHADOW_OFFSET_MAX, SHELL_SHADOW_OFFSET_MIN, SHELL_SPEED,
    SHOCKWAVE_DURATION, TANK_ACCEL_FORCE, TANK_CHASSIS_DAMAGE_FACTOR_BY_ROW,
    TANK_DECEL_CURVE_RATE, TANK_DECEL_SNAP_PX,
    TANK_HULL_DISABLED_DAMAGE,
    TANK_HULL_TRACK_COLS, TANK_HULL_TRACK_FRAME_DISTANCE, TANK_SHELL_VARIANT_BY_ROW,
    TANK_SHELL_VARIANT_STAGGERED_BY_ROW, TANK_TURN_GRIP_FORCE,
    TANK_WRECK_COLS, TRACK_MAX_OPACITY, TRACK_SCALE_FRACTION, TRACK_SCALE_JITTER, TRACK_SPACING,
    TRACK_WEIGHT_OPACITY_BY_ROW, TRACK_WEIGHT_SCALE_BY_ROW, TRACK_WOBBLE_AMP_MAX_DEG,
    TRACK_WOBBLE_AMP_MIN_DEG, TRACK_WOBBLE_WAVELENGTH_MAX, TRACK_WOBBLE_WAVELENGTH_MIN,
};

/// One frame's worth of player input, gathered by the caller (`main.rs`,
/// reading a live `RaylibHandle`) and handed to `Game::update` - this is the
/// entire interface between "simulation" and "wherever input comes from",
/// so `update` itself never needs to know a real keyboard/window exists.
/// Reuses `Intent` (the same shape the AI already produces for enemies) for
/// the actual movement/fire command, so the player and every enemy are
/// driven through the identical `drive_tank`/fire path; the three extra
/// flags below are meta/UI concerns `Intent` has no use for.
#[derive(Default, Clone, Copy)]
pub struct Input {
    /// This frame's commanded movement/fire, straight from raw key state -
    /// `update` itself decides whether to actually honor `move_dir` (e.g.
    /// a wreck can't move but may still fire), not the caller, so the
    /// caller doesn't need to know anything about simulation state to
    /// produce this.
    pub player_intent: Intent,
    pub pause_pressed: bool,
    pub restart_pressed: bool,
    pub toggle_shadows_pressed: bool,
    /// Toggles the debug inspect overlay (per-tank bounding square + ammo/
    /// health/speed/velocity/AI-state readout) - see `Game::inspect_enabled`.
    pub toggle_inspect_pressed: bool,
}

/// Collision-group slot (see `physics::owner_group`) for the player tank's
/// hit sensor and its own shells' shooter-exclusion filter.
pub(crate) const PLAYER_OWNER_SLOT: usize = 0;

/// Collision-group slot for the nth enemy spawned - offset by one so it
/// never collides with `PLAYER_OWNER_SLOT`. Computed once at spawn time
/// (`Game::init`) and stored on that enemy's `Tank::owner_slot`, so nothing
/// later needs this enemy's position in any spawn-order list to rebuild it.
fn enemy_owner_slot(n: usize) -> usize {
    n + 1
}

/// How the current round is going.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum Outcome {
    #[default]
    Playing,
    Won,  // all enemies destroyed
    Lost, // player destroyed
}

#[derive(Default)]
pub struct Game {
    /// Every tank (player + enemies), shell and obstacle lives here as a
    /// hecs entity - see docs/housekeeping-ideas-0.0.2-agy.md's ECS
    /// proposal. A tank entity always carries a `Tank` component; an enemy
    /// additionally carries an `Ai` (the player never does - that's what
    /// distinguishes the two in queries, no separate marker component
    /// needed). A shell entity carries a `Shell`; an obstacle entity
    /// carries an `Obstacle`. `pub(crate)`: `game.rs::render` queries this
    /// directly to draw everything in it.
    pub(crate) world: hecs::World,
    /// The player's entity in `world`. `None` only before the first `init`
    /// call; `main.rs` always calls `init` immediately after
    /// `Game::default()`, so every other method can treat this as always
    /// `Some` once the game is actually running (see the `.expect()`s at
    /// each use site, matching the existing convention for `Tank::body`).
    /// `pub(crate)`: `render` needs it to find the player among `world`'s
    /// entities.
    pub(crate) player: Option<Entity>,
    /// The protect-objective frog's entity in `world` (see `frog::Frog`) -
    /// same "always `Some` once `init` has run" convention as `player`.
    /// `pub(crate)`: `render` needs it to find the frog among `world`'s
    /// entities.
    pub(crate) frog: Option<Entity>,
    /// Seconds until the next pickup respawn attempt - only counts down
    /// while `world` holds fewer than `PICKUP_COUNT` live `Pickup` entities
    /// (see `Game::update`'s pickup section); reset to `PICKUP_RESPAWN_SECONDS`
    /// every time it fires, whether or not that attempt actually finds room.
    pickup_respawn_timer: f32,
    /// This round's four battlefield-boundary wall colliders (see
    /// `battlefield::spawn_walls`), so a shell's flight can be checked against them the
    /// same way as tanks/obstacles (`find_shell_target`) instead of the old
    /// hand-rolled screen-edge coordinate check. Same `None`-only-before-
    /// first-`init` convention as `player`.
    walls: Option<[ColliderHandle; 4]>,
    /// This round's resolved grass/dirt/road layer (see `ground::build`) -
    /// purely decorative (no physics body, no gameplay effect), rebuilt
    /// fresh each `init` same as everything else here. `pub(crate)`: drawn
    /// by `render`, first, before tracks/obstacles/tanks.
    pub(crate) ground: crate::ground::GroundGrid,
    /// Fading tread marks left behind as tanks drive, oldest first. Not
    /// part of `world`: these are pure, gameplay-inert visual trail data
    /// with no physics body and nothing ever queries "give me all tracks"
    /// alongside another component - moving them into the ECS would add
    /// ceremony without the query-composition payoff that motivated moving
    /// tanks/shells/obstacles there. `pub(crate)`: drawn by `render`.
    pub(crate) tracks: Vec<Track>,
    /// Seconds elapsed since the game started; drives damage-overlay
    /// animation. `pub(crate)`: read by `render` (passed to `draw_damage`).
    pub(crate) time: f32,
    /// Result of the current round. `pub(crate)`: read by `render` to pick
    /// the end-of-round banner.
    pub(crate) outcome: Outcome,
    /// Seconds counting down after the round ends; at zero the game
    /// restarts. `pub(crate)`: read by `render` for the banner's countdown text.
    pub(crate) restart_timer: f32,
    /// Shared enemy "last known player position" - `Some` while
    /// `alert_timer` is still counting down. Set/refreshed every frame any
    /// one enemy has the player within `ENEMY_VIEW_RANGE`, so an enemy
    /// outside its own view range can still converge on a sighting the
    /// group made instead of patrolling blind. See `ai::Ai::think`'s
    /// `alert` parameter and `ai::act_patrol`.
    alert_position: Option<Position>,
    /// Seconds remaining before `alert_position` goes stale and is cleared -
    /// reset to `ENEMY_ALERT_HOLD_SECONDS` every frame the sighting holds,
    /// ticked down otherwise.
    alert_timer: f32,
    /// The screen-distortion ring from the most recent tank kill, if one is
    /// still playing out. `pub(crate)`: driven into the shockwave shader by `render`.
    pub(crate) shock: Option<Shockwave>,
    /// Small heat-haze ripples at the barrel of every shot fired recently,
    /// oldest first. Unlike `shock` there can be several in flight at once
    /// (any tank can fire independently), so these are tracked as a list.
    /// `pub(crate)`: drawn by `render`.
    pub(crate) muzzle_flashes: Vec<Shockwave>,
    /// Small impact-flash ripples at the point a shell lands on a tank,
    /// oldest first. Same list-of-many shape as `muzzle_flashes` - several
    /// shells can land in the same frame or overlap in flight. `pub(crate)`:
    /// drawn by `render`.
    pub(crate) impact_flashes: Vec<Shockwave>,
    /// True while the game is paused (toggled by `Input::pause_pressed`);
    /// simulation is frozen and `render` shows a "PAUSED" overlay.
    /// `pub(crate)`: read by `render`.
    pub(crate) paused: bool,
    /// Whether tank/shell drop shadows are drawn (toggled by
    /// `Input::toggle_shadows_pressed`, and by `--no-shadows` at startup -
    /// see main.rs). Not reset by `init`, so it survives round restarts
    /// like `paused` does. Defaults to `false` via `#[derive(Default)]`;
    /// main.rs sets it to `true` right after constructing `Game` unless
    /// `--no-shadows` was passed, so shadows are on by default in normal
    /// play. Already `pub` (a startup/runtime toggle main.rs sets directly,
    /// not just something `render` reads).
    pub shadows_enabled: bool,
    /// Whether the debug inspect overlay is drawn (toggled by
    /// `Input::toggle_inspect_pressed`, the "I" key - see main.rs). Draws a
    /// bounding square around every tank plus a small stat readout
    /// (ammo/health/speed/velocity, and for enemies, AI retreat/fire-cooldown
    /// state) in `game.rs::render`. Not reset by `init`, so it survives round
    /// restarts like `paused`/`shadows_enabled` do. Off by default (`false`
    /// via `#[derive(Default)]`) - this is a debug-only aid, not something
    /// players should see by default.
    pub inspect_enabled: bool,
    /// The rapier physics world (see docs/physics-engine-design.md). Owns
    /// every tank's rigid body plus the battlefield wall colliders. Stays
    /// private - purely a simulation implementation detail, never read by
    /// `render`.
    physics: Physics,
    /// Leftover real time not yet consumed by a fixed physics step; see
    /// `PHYSICS_FIXED_DT` and the accumulator loop in `update`.
    physics_accumulator: f32,
    /// CLI-provided override for how many enemies to spawn (`--enemies`).
    /// When `None`, `init` rolls a random count in
    /// `ENEMY_COUNT_MIN..=ENEMY_COUNT_MAX` as before. Set once before the
    /// first `init` call and left untouched afterwards, so it persists
    /// across round restarts (`init` is called again on restart).
    pub enemy_count_override: Option<usize>,
    /// Same idea as `enemy_count_override`, for how many obstacle
    /// structures to place (`src/bin/probe.rs`'s `--obstacles`, for
    /// stress-testing navigation at a deliberately high, fixed density
    /// rather than `OBSTACLE_COUNT_MIN..=OBSTACLE_COUNT_MAX`'s random
    /// range). Not currently wired to a real-game CLI flag in `main.rs` -
    /// probe-only for now.
    pub obstacle_count_override: Option<usize>,
}

/// scifi_tanks_sheet.png has 12 row-variants, one named archetype per row
/// (see docs/SPRITESHEET_SPEC.md §4), including the super-heavy `titan`
/// (row 10) and `leviathan` (row 11). Gun count is fixed per row: rows 1
/// (assault), 4 (flak), 7 (ravager), 9 (obelisk), 10 (titan) are twin-barrel,
/// the rest are single-barrel.
const TANK_VARIANTS: i32 = 12;

/// A spawn order over all 12 variants that alternates twin- and single-barrel
/// tanks, so however many tanks are on the field, both kinds appear (and
/// appear early). Interleaves twins (1,4,7,9,10) with singles
/// (0,2,3,5,6,8,11).
const TANK_SPRITE_ORDER: [i32; 12] = [1, 0, 4, 2, 7, 3, 9, 5, 10, 6, 8, 11];

impl Game {
    /// Set up the player and spawn the enemy tanks. Also used to restart a
    /// round. `width`/`height` are the battlefield's dimensions in pixels -
    /// `main.rs` passes the window's current size, but nothing here cares
    /// where the number came from.
    pub fn init(&mut self, width: f32, height: f32) {
        let mut rng = rand::rng();

        // Fresh round state: a new World discards every tank/shell/obstacle
        // entity from the previous round in one shot.
        self.world = hecs::World::new();
        self.tracks.clear();
        self.time = 0.0;
        self.outcome = Outcome::Playing;
        self.restart_timer = 0.0;
        self.alert_position = None;
        self.alert_timer = 0.0;
        self.pickup_respawn_timer = 0.0;
        self.shock = None;
        self.muzzle_flashes.clear();
        self.impact_flashes.clear();

        // Fresh physics world each round rather than trying to reuse/resize
        // the previous one - cheap at this scale, and simplest if the
        // battlefield size ever changes between rounds. See
        // docs/physics-engine-design.md.
        self.physics = Physics::new();
        self.physics_accumulator = 0.0;
        self.walls = Some(battlefield::spawn_walls(&mut self.physics, width, height));

        // --- Player ---
        // row: pick a random hull/turret variant (any of the 12; a mix of
        // single/twin barrel, including the two super-heavy archetypes).
        // shell_variant: matched to that chassis (see TANK_SHELL_VARIANT_BY_ROW).
        // damage_variant: this round's damage.png row-variant (see
        // Tank::damage_variant).
        let row = rng.random_range(0..TANK_VARIANTS);
        // The player spawns inside the fortress's O, offset from the true
        // screen center so the "HELLO" sign as a whole reads centered - see
        // `fortress_center_offset`.
        let mut tank = Tank {
            row,
            shell_variant: TANK_SHELL_VARIANT_BY_ROW[row as usize],
            damage_variant: rng.random_range(0..DAMAGE_VARIANTS),
            position: Position::new(width / 2.0 - battlefield::fortress_center_offset(), height / 2.0),
            owner_slot: PLAYER_OWNER_SLOT,
            ..Tank::default()
        };
        roll_track_distortion(&mut tank, &mut rng);
        // Spawn rotation is 0.0 (facing up, Tank::default) - `along_x: false`
        // matches that, same as `facing_along_x` would report for it.
        tank.body = Some(
            self.physics
                .spawn_tank(tank.position, tank.hull_half_extents(false), tank.mass()),
        );
        tank.hit_sensor = Some(self.physics.add_hit_sensor(
            tank.body.unwrap(),
            tank.size() * 0.5,
            physics::owner_group(PLAYER_OWNER_SLOT),
        ));
        // Clearance figures below are derived from the player's own hull
        // size, so grab them from this local `tank` before it's moved into
        // the world.
        let center = tank.position;
        // Keeps enemies/obstacles off the player's own exact spawn point.
        // Deliberately *not* also bumped up to cover the fortress as a
        // whole (an earlier version maxed this against a
        // furthest-tile-from-`center` radius) - for a word as lopsided as
        // "HELLO" (the `O` is the *last* letter, so the rest of the word
        // hangs far off to one side - see `fortress_center_offset`) that
        // radius ballooned to nearly the size of the whole battlefield,
        // which starved enemy/obstacle placement almost everywhere instead
        // of just near the fortress. The precise per-tile checks below
        // (against every real fortress wall tile) already fully guarantee
        // fortress clearance on their own; the radius was a redundant,
        // shape-blind broad-phase check on top of them, not load-bearing.
        let clear = tank.size() * 2.0;
        // Also keep spawned enemies clear of each other - without this, a
        // crowded high-count round can drop two tanks on top of one another,
        // and they start ramming (and damaging) each other before the round
        // even properly begins.
        let enemy_clear = tank.size() * 1.5;
        self.player = Some(self.world.spawn((tank,)));

        // --- Player fortress ---
        // Built right after the player so enemies/regular obstacles (both
        // spawned below) can be kept clear of it - see `battlefield::spawn_player_fortress`.
        let obstacle_half_extent =
            OBSTACLE_TEXTURE_SIZE * OBSTACLE_SCALE * OBSTACLE_HULL_FRACTION * 0.5;
        let fortress = battlefield::spawn_player_fortress(
            &mut self.physics,
            &mut self.world,
            &mut rng,
            center,
            obstacle_half_extent,
            width,
            height,
        );
        let mut obstacle_positions = fortress.tiles;
        let fortress_road_cells = fortress.road_cells;

        // --- Enemies ---
        // Spawn enemy tanks in a band that's 20%-40% of the shorter screen
        // dimension away from the nearest edge of the battlefield, and away from
        // the player's starting spot in the middle.
        let short_side = width.min(height);
        let margin_min = short_side * ENEMY_SPAWN_MARGIN_MIN;
        let margin_max = short_side * ENEMY_SPAWN_MARGIN_MAX;
        let enemy_count = self
            .enemy_count_override
            .unwrap_or_else(|| rng.random_range(ENEMY_COUNT_MIN..=ENEMY_COUNT_MAX));

        // Positions of enemies placed so far - purely local bookkeeping for
        // the clearance checks below, not the entities' actual storage
        // (that's `self.world`).
        let mut enemy_positions: Vec<Position> = Vec::with_capacity(enemy_count);
        while enemy_positions.len() < enemy_count {
            let pos = battlefield::sample_clear_position(&mut rng, width, height, margin_min, |pos| {
                let border_dist = pos.x.min(width - pos.x).min(pos.y).min(height - pos.y);
                border_dist <= margin_max
                    && pos.distance_to(center) >= clear
                    && enemy_positions
                        .iter()
                        .all(|&p| pos.distance_to(p) >= enemy_clear)
                    // The `clear`-from-`center` check above only guards the
                    // player's own exact spawn point, not the fortress as a
                    // whole - that's this per-tile check instead, against
                    // every real fortress wall tile individually
                    // (obstacle_positions holds only fortress tiles at this
                    // point in init). Same per-tile clearance rule
                    // scatter_obstacles already uses for obstacles vs. the
                    // fortress (battlefield.rs) - see `clear`'s own comment
                    // above for why a single bounding-radius check (tried
                    // first) doesn't work for a word shape this lopsided.
                    && obstacle_positions
                        .iter()
                        .all(|&p| pos.distance_to(p) >= enemy_clear + OBSTACLE_CLEAR)
            });
            // Walk the alternating spawn order so each enemy looks distinct and the
            // group mixes single- and twin-barrel hulls.
            let erow = TANK_SPRITE_ORDER[enemy_positions.len() % TANK_SPRITE_ORDER.len()];
            // Vary speed within +/- ENEMY_SPEED_VARIANCE so enemies don't all move
            // in lockstep; each keeps this speed for the round.
            let factor = 1.0 + rng.random_range(-ENEMY_SPEED_VARIANCE..ENEMY_SPEED_VARIANCE);
            let owner_slot = enemy_owner_slot(enemy_positions.len());
            let mut enemy = Tank {
                row: erow,
                shell_variant: TANK_SHELL_VARIANT_BY_ROW[erow as usize],
                damage_variant: rng.random_range(0..DAMAGE_VARIANTS),
                position: pos,
                rotation: 180.0,             // facing down, toward the player's start
                speed: ENEMY_SPEED * factor, // enemies drive slower than the player
                owner_slot,
                ..Tank::default()
            };
            roll_track_distortion(&mut enemy, &mut rng);
            // Spawn rotation is 180.0 (facing down) - `along_x: false`, same
            // Y-axis case as the player's spawn facing.
            enemy.body = Some(self.physics.spawn_tank(
                pos,
                enemy.hull_half_extents(false),
                enemy.mass(),
            ));
            enemy.hit_sensor = Some(self.physics.add_hit_sensor(
                enemy.body.unwrap(),
                enemy.size() * 0.5,
                physics::owner_group(owner_slot),
            ));
            enemy_positions.push(pos);
            self.world.spawn((enemy, Ai::default()));
        }

        // --- Obstacles ---
        // Static in-arena terrain: scattered after tanks so it can steer
        // clear of both the player's fixed start and every enemy's rolled
        // start, instead of tanks needing to dodge obstacles placed first.
        // See `battlefield::scatter_obstacles` for the placement rules
        // (structure shapes, per-structure material/variant, the
        // cross-structure corridor-clearance margin).
        let structure_count =
            battlefield::roll_structure_count(&mut rng, self.obstacle_count_override);
        battlefield::scatter_obstacles(
            &mut self.physics,
            &mut self.world,
            &mut rng,
            width,
            height,
            margin_min,
            obstacle_half_extent,
            center,
            clear,
            &enemy_positions,
            enemy_clear,
            &mut obstacle_positions,
            structure_count,
        );

        // Now that every obstacle for the round is down, catch any enemy
        // whose rolled spawn point ended up fully boxed in by the
        // *cumulative* effect of several individually-fine obstacle
        // placements (see `relocate_boxed_in_tanks`'s own doc comment) and
        // relocate it - then refresh `enemy_positions` from the
        // now-possibly-moved tanks so the frog's own clearance check below
        // sees where they actually ended up.
        battlefield::relocate_boxed_in_tanks(&mut self.physics, &mut self.world, width, height);
        enemy_positions = self
            .world
            .query::<&Tank>()
            .with::<&Ai>()
            .iter()
            .map(|t| t.position)
            .collect();

        // --- Frog (protect-objective) ---
        // Placed near the player's fortress (FROG_SPAWN_MIN_DIST/MAX_DIST
        // from `center`) rather than anywhere on the map, so defending it
        // and defending the player's own start are the same early fight
        // instead of two unrelated ones. Spawned after obstacles so
        // `obstacle_positions` already holds every fortress + scattered-
        // structure tile to steer clear of; reuses the same rejection-
        // sampling `sample_clear_position` the enemy loop above uses.
        let frog_clear =
            FROG_COLLIDER_HALF_EXTENT.0.max(FROG_COLLIDER_HALF_EXTENT.1) + OBSTACLE_CLEAR;
        let frog_pos = battlefield::sample_clear_position(&mut rng, width, height, margin_min, |pos| {
            let dist = pos.distance_to(center);
            dist >= FROG_SPAWN_MIN_DIST
                && dist <= FROG_SPAWN_MAX_DIST
                && enemy_positions.iter().all(|&p| pos.distance_to(p) >= enemy_clear)
                && obstacle_positions.iter().all(|&p| pos.distance_to(p) >= frog_clear)
        });
        let frog_body = self.physics.spawn_static(
            frog_pos,
            Position::new(FROG_COLLIDER_HALF_EXTENT.0, FROG_COLLIDER_HALF_EXTENT.1),
        );
        self.frog = Some(self.world.spawn((Frog {
            position: frog_pos,
            health: FROG_MAX_HEALTH,
            max_health: FROG_MAX_HEALTH,
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

        // --- Pickups (health/ammo) ---
        // One per corner at round start (PICKUP_COUNT); `Game::update` tops
        // this back up over time as pickups are collected, via this same
        // function (see `spawn_pickup`'s own doc comment). Placed after the
        // frog so pickups steer clear of it too, same "clear of whatever's
        // already down" order obstacles/frog themselves follow - and after
        // every fortress/scattered-obstacle tile, all of which are already
        // `Obstacle` entities in `self.world` by now, so `spawn_pickup`'s
        // own live query picks up fortress clearance for free without
        // needing `center`/`clear` passed in separately the way the frog's
        // placement above does.
        for _ in 0..PICKUP_COUNT {
            spawn_pickup(&mut self.world, &mut rng, width, height);
        }

        // --- Ground ---
        // Built last, once every wall/brick object for the round (fortress +
        // scattered structures, all folded into `obstacle_positions` by now)
        // and the fortress's own B/O interior cells are known - grass
        // everywhere, road painted under each of those and nowhere else. See
        // `ground::build`.
        let mut road_cells = obstacle_positions;
        road_cells.extend(fortress_road_cells);
        self.ground = crate::ground::build(width, height, &mut rng, &road_cells);
    }

    /// Step the simulation one frame. `input` is this frame's player
    /// input (see `Input`); `dt` is the frame's elapsed time in seconds
    /// and `width`/`height` the battlefield's current dimensions - all
    /// plain data, gathered by the caller from wherever it likes (a live
    /// `RaylibHandle` in `main.rs` today).
    pub fn update(&mut self, input: Input, dt: f32, width: f32, height: f32) {
        if input.pause_pressed {
            self.paused = !self.paused;
        }
        if input.toggle_shadows_pressed {
            self.shadows_enabled = !self.shadows_enabled;
        }
        if input.toggle_inspect_pressed {
            self.inspect_enabled = !self.inspect_enabled;
        }
        // Debug convenience: instantly restart the round on demand, the same
        // way `restart_timer` hitting zero already does after a round ends -
        // but available at any time (mid-round, paused, or on the win/lose
        // screen), so this is checked before the pause early-return below.
        if input.restart_pressed {
            self.init(width, height);
            return;
        }
        if self.paused {
            return;
        }

        let player = self.player.expect("player entity spawned in init");

        // Advance any in-flight shockwave regardless of round state, so it
        // finishes fading even if this frame's damage just ended the round.
        if let Some(shock) = &mut self.shock {
            shock.time += dt;
            if shock.time >= SHOCKWAVE_DURATION {
                self.shock = None;
            }
        }
        // Same, but for every in-flight muzzle-flash heat haze.
        self.muzzle_flashes.retain_mut(|flash| {
            flash.time += dt;
            flash.time < MUZZLE_FLASH_DURATION
        });
        // Same, but for every in-flight shell-impact flash.
        self.impact_flashes.retain_mut(|flash| {
            flash.time += dt;
            flash.time < IMPACT_FLASH_DURATION
        });

        // Round is over: count down and restart, but keep animating the scene
        // (burning wrecks etc.) so the end screen stays lively.
        if self.outcome != Outcome::Playing {
            self.time += dt;
            for tank in self.world.query::<&mut Tank>().iter() {
                tank.tick_wreck(dt);
                roll_wreck_col(tank);
            }
            for obstacle in self.world.query::<&mut Obstacle>().iter() {
                obstacle.tick_burn(dt);
            }
            for frog in self.world.query::<&mut Frog>().iter() {
                frog.tick(dt);
                // Keep the physics body in step with `position` while a
                // hop is animating (see `Frog::tick`'s doc comment) - the
                // "round is over" branch still runs this so an in-flight
                // hop finishes visually instead of freezing mid-air.
                self.physics.set_position(frog.body, frog.position);
            }
            self.tracks.retain_mut(|t| !t.tick(dt));
            self.restart_timer -= dt;
            if self.restart_timer <= 0.0 {
                self.init(width, height);
            }
            return;
        }

        // Advance the global animation clock and per-tank timers (every
        // tank - player and enemies alike share the exact same ticking, so
        // one query covers both instead of a separate player statement plus
        // an enemies loop).
        self.time += dt;
        for tank in self.world.query::<&mut Tank>().iter() {
            tank.tick_recharge(dt);
            tank.fire_cooldown = (tank.fire_cooldown - dt).max(0.0);
            tank.ram_cooldown = (tank.ram_cooldown - dt).max(0.0);
            tank.hit_flash_timer = (tank.hit_flash_timer - dt).max(0.0);
            tank.tick_wreck(dt);
            roll_wreck_col(tank);
        }
        for obstacle in self.world.query::<&mut Obstacle>().iter() {
            obstacle.tick_burn(dt);
        }
        for frog in self.world.query::<&mut Frog>().iter() {
            frog.tick(dt);
            // Keep the physics body in step with `position` while a hop is
            // animating (see `Frog::tick`'s doc comment) - a no-op write
            // the rest of the time, since `tick` only moves `position`
            // during an in-flight hop.
            self.physics.set_position(frog.body, frog.position);
        }
        // Age existing marks and drop the ones that have fully faded.
        self.tracks.retain_mut(|t| !t.tick(dt));

        let mut rng = rand::rng();
        // Tanks freshly destroyed this frame (by ramming or by shellfire
        // below), tagged with whether the victim was an enemy; each gets a
        // shockwave and a small splash of explosion damage to nearby tanks on
        // the *opposing* side, processed once all of this frame's movement
        // and shell hits are resolved. A splash chip can itself finish off an
        // already-critical tank - `apply_explosion` feeds that fresh kill
        // back into this same vec (see the while-loop below), so it still
        // gets its own shockwave/knockback instead of dying with no effect.
        let mut kills: Vec<(Position, bool)> = Vec::new();
        // Shells fired this frame. hecs doesn't allow spawning a new entity
        // into `self.world` while a query over it is still active (it can
        // reallocate archetype storage underneath the query's borrow), so
        // every shot fired below is collected here and actually spawned in
        // one batch after the player/enemy sections finish querying tanks.
        let mut pending_shells: Vec<Shell> = Vec::new();

        // --- Frog: bite the nearest tank in range ---
        // Independent of the hop-on-hit reaction handled down in the shell
        // loop below - checked every frame regardless of whether the frog
        // has taken any fire this round, so standing next to it is
        // dangerous even if it's never been shot. Only the single nearest
        // in-range tank (either side) is bitten per attack tick, gated by
        // `Frog::can_attack`/FROG_ATTACK_COOLDOWN_SECONDS the same way a
        // tank's own `fire_cooldown` paces its shots.
        {
            let frog_entity = self.frog.expect("frog entity spawned in init");
            let (can_attack, frog_pos, attack_range) = with_frog(&self.world, frog_entity, |f| {
                (f.can_attack(), f.position, f.attack_range())
            });
            if can_attack {
                let mut nearest: Option<(Entity, f32)> = None;
                for (entity, tank) in self.world.query::<(Entity, &Tank)>().iter() {
                    if tank.is_wreck() {
                        continue;
                    }
                    let d = tank.position.distance_to(frog_pos);
                    let closer_than_current = match nearest {
                        None => true,
                        Some((_, nd)) => d < nd,
                    };
                    if d <= attack_range && closer_than_current {
                        nearest = Some((entity, d));
                    }
                }
                if let Some((target, _)) = nearest {
                    let dmg = rng.random_range(FROG_ATTACK_DAMAGE_MIN..FROG_ATTACK_DAMAGE_MAX);
                    let (became_wreck, victim_pos) = {
                        let mut q = self.world.query_one::<&mut Tank>(target);
                        let tank = q.get().expect("attack target should always have a Tank");
                        tank.damage = (tank.damage + dmg).min(MAX_DAMAGE);
                        tank.mark_hit();
                        (tank.is_wreck(), tank.position)
                    };
                    if became_wreck {
                        kills.push((victim_pos, target != player));
                    }
                    with_frog_mut(&self.world, frog_entity, |f| f.start_attack());
                }
            }
        }

        // --- Frog: hop away from the nearest tank if one's gotten close ---
        // Same "either side" symmetry as the bite above - friendly and
        // enemy tanks both count as too close. Independent of the bite
        // above rather than an alternative to it (see FROG_AVOID_RANGE_FACTOR's
        // own comment): a tank already within bite range is *also* within
        // avoid range (the latter is deliberately the bigger of the two),
        // so both can trigger the same round - the frog bites whatever's
        // adjacent to it this frame, on its own cooldown, while separately
        // trying to open up distance from whichever tank is nearest, on
        // *its* own cooldown.
        {
            let frog_entity = self.frog.expect("frog entity spawned in init");
            let (can_hop, frog_pos, avoid_range, hop_distance) =
                with_frog(&self.world, frog_entity, |f| {
                    (f.can_hop(), f.position, f.avoid_range(), f.hop_distance())
                });
            if can_hop {
                let nearest_tank = self
                    .world
                    .query::<&Tank>()
                    .iter()
                    .filter(|t| !t.is_wreck())
                    .map(|t| (t.position, t.position.distance_to(frog_pos)))
                    .filter(|&(_, d)| d <= avoid_range)
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((tank_pos, _)) = nearest_tank {
                    // Obstacle positions collected fresh here (infrequent -
                    // only when a tank has actually closed within avoid
                    // range and the frog isn't on hop cooldown, not every
                    // frame) rather than threaded through from elsewhere,
                    // same reasoning as the shell-hit hop below.
                    let obstacle_positions: Vec<Position> = self
                        .world
                        .query::<&Obstacle>()
                        .iter()
                        .map(|o| o.position)
                        .collect();
                    let away_from_tank =
                        Position::new(frog_pos.x - tank_pos.x, frog_pos.y - tank_pos.y);
                    if let Some(new_pos) = frog_hop_target(
                        &mut rng,
                        frog_pos,
                        away_from_tank,
                        hop_distance,
                        &obstacle_positions,
                        width,
                        height,
                    ) {
                        with_frog_mut(&self.world, frog_entity, |f| f.start_hop(new_pos));
                    }
                }
            }
        }

        // --- Pickups: collect on touch, keep the field topped up ---
        // Checked every frame against every living tank's current position -
        // pure proximity, no physics body (see pickup.rs's module doc
        // comment for why). Collect-then-apply: snapshot which tank (if any)
        // is in range of each pickup first, then apply effects/despawn
        // after, same reason as the destroyed-obstacle/kill cleanup
        // elsewhere in this function - can't despawn a `world` entity while
        // a query over it is still borrowed.
        {
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
                        .find(|(_, pos)| pos.distance_to(pickup.position) <= PICKUP_COLLECT_RADIUS)
                        .map(|&(tank_entity, _)| (pickup_entity, tank_entity, pickup.kind))
                })
                .collect();
            for (pickup_entity, tank_entity, kind) in collected {
                let mut q = self.world.query_one::<&mut Tank>(tank_entity);
                let tank = q.get().expect("collector entity always has a Tank");
                match kind {
                    PickupKind::Health => tank.damage = (tank.damage - PICKUP_HEAL_AMOUNT).max(0.0),
                    PickupKind::Ammo => {
                        tank.shells_ammo = (tank.shells_ammo + PICKUP_AMMO_AMOUNT).min(SHELLS_HARD_CAP)
                    }
                }
                drop(q);
                self.world.despawn(pickup_entity).ok();
            }

            // Top back up to PICKUP_COUNT after a delay, once collection (or,
            // at round start before the very first spawn_pickup calls in
            // `init`, never - this only ever runs once `update` is already
            // ticking) drops the live count below it. Not tied to *which*
            // corner just emptied, just "how many are live right now".
            if self.world.query::<&Pickup>().iter().count() < PICKUP_COUNT {
                self.pickup_respawn_timer -= dt;
                if self.pickup_respawn_timer <= 0.0 {
                    spawn_pickup(&mut self.world, &mut rng, width, height);
                    self.pickup_respawn_timer = PICKUP_RESPAWN_SECONDS;
                }
            } else {
                self.pickup_respawn_timer = 0.0;
            }
        }

        // --- Player: hand this frame's intent to physics ---
        // A wreck can't move but may still fire - `update` (not the input
        // source) is what decides that, since it's the one thing here that
        // actually knows the player's current state.
        let mut player_intent = input.player_intent;
        {
            let mut q = self.world.query_one::<&mut Tank>(player);
            let player_tank = q.get().expect("player entity always has a Tank");
            if player_tank.is_wreck() {
                player_intent.move_dir = None;
            }

            // Hand the player's commanded velocity to its physics body. Actual
            // movement and collision (walls, tank-vs-tank blocking) happen below
            // when the physics world steps - not here.
            drive_tank(&mut self.physics, player_tank, player_intent, dt);
            if player_intent.fire && player_tank.shells_ammo >= 1 && player_tank.fire_cooldown <= 0.0 {
                player_tank.shells_ammo -= 1;
                player_tank.fire_cooldown = PLAYER_FIRE_INTERVAL;
                // Alternate simultaneous/staggered twin shell art shot-to-shot
                // (no-op for single-barrel chassis - see Tank::alternate_shot).
                player_tank.shell_variant = if player_tank.alternate_shot {
                    TANK_SHELL_VARIANT_STAGGERED_BY_ROW[player_tank.row as usize]
                } else {
                    TANK_SHELL_VARIANT_BY_ROW[player_tank.row as usize]
                };
                player_tank.alternate_shot = !player_tank.alternate_shot;
                // The player always fires straight down the barrel.
                let mut shell = Shell::spawn(player_tank, Owner::Player, 0.0);
                shell.body = Some(self.physics.spawn_shell(
                    shell.position,
                    SHELL_HIT_HALF_EXTENT,
                    physics::owner_group(PLAYER_OWNER_SLOT),
                ));
                shell.shadow_offset =
                    rng.random_range(SHELL_SHADOW_OFFSET_MIN..SHELL_SHADOW_OFFSET_MAX);
                self.muzzle_flashes.push(Shockwave {
                    center: shell.position,
                    time: 0.0,
                });
                pending_shells.push(shell);
            }
        }

        // --- Enemies: each brain decides an intent, then hands it to physics too ---
        // Snapshot every live tank's motion for predictive collision avoidance,
        // plus a map from each enemy's entity to its slot in that snapshot
        // (slot 0 is always the player) - see `motion_snapshot`.
        let (movers, enemy_indices) = self.motion_snapshot();
        // This frame's obstacle occupancy grid (see pathfind::Grid), so
        // `Ai::steer` can route around static obstacles instead of just
        // walking into one and getting physically stuck by its collider.
        // Rebuilt fresh every frame - obstacles are few and the grid small,
        // so this is cheap enough not to need caching. The margin is the
        // *worst-case* tank in the roster (see `max_tank_avoidance_radius`),
        // not just a representative default one, so pathfinding never routes
        // even the biggest tank (titan/leviathan) through a gap too narrow
        // for it to actually fit through.
        //
        // The frog is chained in alongside `Obstacle` for the same reason:
        // it's spawned as a real, solid `Physics::spawn_static` body (see
        // `Game::init`) that blocks tank movement exactly like an obstacle
        // tile does, and it can relocate mid-round (`Frog::start_hop`) - but
        // it isn't an `Obstacle` component, so without this the grid had no
        // idea it existed at all. That's the *literal* version of this
        // comment's own "walking into one and getting physically stuck by
        // its collider" warning: pathfinding treated the frog's cell as
        // open ground, routed tanks straight through it, and physics
        // stopped them cold - a "stuck near the frog" symptom that read
        // like a bug in steering when the grid simply never knew to route
        // around it.
        let grid = Grid::build(
            width,
            height,
            PATHFIND_CELL_SIZE,
            battlefield::max_tank_avoidance_radius(),
            self.world
                .query::<&Obstacle>()
                .iter()
                .map(|o| (o.position, o.hull_size() * 0.5))
                .chain(self.world.query::<&Frog>().iter().map(|f| {
                    (
                        f.position,
                        FROG_COLLIDER_HALF_EXTENT.0.max(FROG_COLLIDER_HALF_EXTENT.1),
                    )
                })),
        );
        // Shared aggression (see ENEMY_ALERT_HOLD_SECONDS): if any enemy
        // currently has the player within ENEMY_VIEW_RANGE, refresh the
        // group's shared "last known player position" so every enemy - even
        // ones with the player well outside their own view range - can
        // converge on it via act_patrol instead of wandering randomly.
        let player_alive = with_tank(&self.world, player, |t| !t.is_wreck());
        let player_pos = movers[0].position;
        let any_enemy_sees_player = player_alive
            && movers[1..]
                .iter()
                .any(|m| m.position.distance_to(player_pos) <= ENEMY_VIEW_RANGE);
        if any_enemy_sees_player {
            self.alert_position = Some(player_pos);
            self.alert_timer = ENEMY_ALERT_HOLD_SECONDS;
        } else {
            self.alert_timer = (self.alert_timer - dt).max(0.0);
            if self.alert_timer <= 0.0 {
                self.alert_position = None;
            }
        }
        let alert = self.alert_position.filter(|_| self.alert_timer > 0.0);

        for (entity, tank, ai) in self.world.query::<(Entity, &mut Tank, &mut Ai)>().iter() {
            let my_index = enemy_indices[&entity];
            // `Ai::think`'s targeting reads the player's tank; grabbed via a
            // separate, shared `query_one` (see `with_tank`) each iteration
            // rather than once up front, since it needs to coexist with the
            // exclusive `(&mut Tank, &mut Ai)` borrow this loop already
            // holds over every *other* archetype - the player's is a
            // different one (no `Ai`), so the two never actually alias.
            // This tank's *actual* physics speed this frame, for `Ai::think`'s
            // stuck-escape check (`Ai::steer`) - `Tank::velocity` is only the
            // commanded target (see its doc comment), which reads the same
            // whether the tank is freely cruising or wedged against an
            // obstacle's collider; `Physics::velocity` doesn't.
            let real_speed = tank
                .body
                .map(|handle| {
                    let v = self.physics.velocity(handle);
                    (v.x * v.x + v.y * v.y).sqrt()
                })
                .unwrap_or(0.0);
            let intent = with_tank(&self.world, player, |player_tank| {
                ai.think(
                    tank,
                    player_tank,
                    width,
                    height,
                    dt,
                    real_speed,
                    &movers,
                    my_index,
                    &grid,
                    &mut rng,
                    alert,
                )
            });
            drive_tank(&mut self.physics, tank, intent, dt);
            if intent.fire && tank.shells_ammo >= 1 {
                tank.shells_ammo -= 1;
                // Alternate simultaneous/staggered twin shell art shot-to-shot
                // (no-op for single-barrel chassis - see Tank::alternate_shot).
                tank.shell_variant = if tank.alternate_shot {
                    TANK_SHELL_VARIANT_STAGGERED_BY_ROW[tank.row as usize]
                } else {
                    TANK_SHELL_VARIANT_BY_ROW[tank.row as usize]
                };
                tank.alternate_shot = !tank.alternate_shot;
                // Point-blank shots may be thrown off-aim (see roll_misfire).
                let mut shell =
                    Shell::spawn(tank, Owner::Enemy(tank.owner_slot), intent.fire_aim_offset);
                shell.body = Some(self.physics.spawn_shell(
                    shell.position,
                    SHELL_HIT_HALF_EXTENT,
                    physics::owner_group(tank.owner_slot),
                ));
                shell.shadow_offset =
                    rng.random_range(SHELL_SHADOW_OFFSET_MIN..SHELL_SHADOW_OFFSET_MAX);
                self.muzzle_flashes.push(Shockwave {
                    center: shell.position,
                    time: 0.0,
                });
                pending_shells.push(shell);
            }
        }
        // Now safe to actually insert this frame's shots - no tank/Ai query
        // is active anymore.
        for shell in pending_shells {
            self.world.spawn((shell,));
        }

        // --- Shells: advance movement/animation, then sync into physics ---
        // A shell's position is still hand-integrated (velocity * dt) rather
        // than physics-driven, matching its existing state machine - but
        // pushing that position into its kinematic sensor here, before the
        // physics step below, is what lets the intersection queries after
        // that step (see further down) reflect this frame's movement. See
        // docs/physics-engine-design.md.
        for shell in self.world.query::<&mut Shell>().iter() {
            shell.update(dt);
            let handle = shell
                .body
                .expect("shell should always have a physics body once spawned");
            self.physics.set_kinematic_position(handle, shell.position);
        }

        // --- Physics: advance the world in fixed steps ---
        // Every tank's body already has this frame's commanded velocity (set
        // above); stepping resolves all of this frame's movement and collision
        // (walls, tank-vs-tank blocking) for every body at once, rather than
        // the old per-tank sequential move-then-revert-if-overlapping dance. A
        // fixed step keeps the contact solver's behavior consistent regardless
        // of the render frame rate. See docs/physics-engine-design.md.
        self.physics_accumulator = (self.physics_accumulator + dt).min(PHYSICS_MAX_CATCHUP_SECONDS);
        while self.physics_accumulator >= PHYSICS_FIXED_DT {
            self.physics.step();
            self.physics_accumulator -= PHYSICS_FIXED_DT;
        }

        // --- Read positions back, then resolve ram damage and lay tracks ---
        let tank_before = with_tank(&self.world, player, |t| t.position);
        with_tank_mut(&self.world, player, |t| {
            sync_tank_from_physics(&self.physics, t)
        });
        let enemies_before: Vec<(Entity, Position)> = self
            .world
            .query::<(Entity, &Tank)>()
            .with::<&Ai>()
            .iter()
            .map(|(e, t)| (e, t.position))
            .collect();
        for tank in self.world.query::<&mut Tank>().with::<&Ai>().iter() {
            sync_tank_from_physics(&self.physics, tank);
        }

        // A tank touching the opposing side takes a cooldown-gated ram-damage
        // hit; the collider contact itself already stopped/redirected their
        // movement during the physics step above, so this only handles the
        // damage roll (see `ram`'s doc comment for why enemy-vs-enemy contact
        // doesn't call this). "Touching" is read straight from rapier's own
        // narrow-phase contact state (see `Physics::touching`), not a
        // hand-rolled geometric re-check.
        for (enemy_entity, _) in &enemies_before {
            let touching = with_tank(&self.world, player, |p| {
                with_tank(&self.world, *enemy_entity, |e| {
                    tanks_touching(&self.physics, p, e)
                })
            });
            if touching {
                with_two_tanks_mut(&mut self.world, player, *enemy_entity, |p, e| {
                    ram(p, false, e, true, &mut self.physics, &mut rng, &mut kills);
                });
                break;
            }
        }
        with_tank_mut(&self.world, player, |t| {
            lay_tracks(&mut self.tracks, t, tank_before)
        });
        for (enemy_entity, before) in &enemies_before {
            let touching = with_tank(&self.world, *enemy_entity, |e| {
                with_tank(&self.world, player, |p| tanks_touching(&self.physics, e, p))
            });
            if touching {
                with_two_tanks_mut(&mut self.world, *enemy_entity, player, |e, p| {
                    ram(e, true, p, false, &mut self.physics, &mut rng, &mut kills);
                });
            }
            with_tank_mut(&self.world, *enemy_entity, |t| {
                lay_tracks(&mut self.tracks, t, *before)
            });
        }

        // --- Shells: mutual detonation when two shells from opposing
        // owners collide mid-air ---
        // Checked before the shell-vs-target loop below, and with a
        // continuous (swept) test rather than an end-of-frame position
        // overlap: at SHELL_SPEED (500px/s) a shell moves ~8px/frame at
        // 60fps against a collision radius of only SHELL_HIT_HALF_EXTENT*2
        // (6px), so two shells closing head-on can tunnel straight through
        // each other between frames without their positions ever
        // overlapping at a frame boundary. Instead this finds, for each
        // pair, the moment of closest approach over the frame's straight-
        // line motion and checks the distance there. Same-owner pairs are
        // skipped - mirrors the shooter-exclusion physics groups already
        // give shell-vs-tank hits (see `find_shell_target`'s doc comment) -
        // so a tank's own shells (e.g. a double-barrel chassis's paired
        // shots) never detonate on each other. No damage/knockback to
        // nearby tanks here, unlike a tank kill's `apply_explosion` splash -
        // this is purely two shells cancelling each other out.
        let flying_shells: Vec<(Entity, Position, Vector2, Owner)> = self
            .world
            .query::<(Entity, &Shell)>()
            .iter()
            .filter(|(_, s)| s.state == ShellState::Flying)
            .map(|(e, s)| {
                let disp = s.velocity * dt;
                (e, s.position - disp, disp, s.owner)
            })
            .collect();

        let shell_collide_dist = SHELL_HIT_HALF_EXTENT * 2.0;
        let mut claimed_shells: HashSet<Entity> = HashSet::new();
        let mut shell_collisions: Vec<(Entity, Entity, Position)> = Vec::new();
        for i in 0..flying_shells.len() {
            let (e1, prev1, disp1, owner1) = flying_shells[i];
            if claimed_shells.contains(&e1) {
                continue;
            }
            for &(e2, prev2, disp2, owner2) in &flying_shells[i + 1..] {
                if owner1 == owner2 || claimed_shells.contains(&e2) {
                    continue;
                }
                // Closest approach of the two shells' straight-line motion
                // this frame: position(t) = prev + disp * t, t in [0, 1].
                let rel_pos = prev1 - prev2;
                let rel_disp = disp1 - disp2;
                let denom = rel_disp.dot(rel_disp);
                let t = if denom > 0.0 {
                    (-rel_pos.dot(rel_disp) / denom).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let closest1 = prev1 + disp1 * t;
                let closest2 = prev2 + disp2 * t;
                if closest1.distance_to(closest2) <= shell_collide_dist {
                    let midpoint = Position::new(
                        (closest1.x + closest2.x) * 0.5,
                        (closest1.y + closest2.y) * 0.5,
                    );
                    shell_collisions.push((e1, e2, midpoint));
                    claimed_shells.insert(e1);
                    claimed_shells.insert(e2);
                    break;
                }
            }
        }
        for (e1, e2, midpoint) in shell_collisions {
            self.impact_flashes.push(Shockwave {
                center: midpoint,
                time: 0.0,
            });
            for e in [e1, e2] {
                let mut q = self.world.query_one::<&mut Shell>(e);
                let shell = q.get().expect("shell collected this frame still exists");
                shell.detonate();
            }
        }

        // --- Shells: damage/detonate against whatever they're intersecting
        // (Flying only) ---
        // A shell can hit any tank except the one that fired it - including
        // other enemies, so enemy fire is a real hazard to the whole field,
        // not just the player. Excluding the shooter's own tank matters: a
        // shell spawns right on its own tank's hit sensor, so without this
        // it would detonate on itself the instant it starts flying. That
        // exclusion is handled by `Physics`'s collision groups now (see
        // `physics::owner_group`, `add_hit_sensor`, `spawn_shell`) - a
        // shooter's own hit sensor never registers as intersecting its own
        // shells, so `find_shell_target` doesn't need to re-check
        // `shell.owner` against the tank it's testing. Damage amount still
        // depends on who fired it (PLAYER_DAMAGE_*/ENEMY_DAMAGE_*), not who
        // it hits. Hit detection reads real physics intersections (a
        // shell's sensor vs. a tank's hit sensor/an obstacle's collider/a
        // wall's collider - see `Physics::intersecting`) rather than a
        // hand-rolled point-in-box or coordinate-bounds check - a shell
        // hits at most one target per frame, checked in priority order
        // (player, then enemies, then obstacles, then walls).
        let walls = self.walls.expect("walls spawned in init");
        for shell in self.world.query::<&mut Shell>().iter() {
            if shell.state != ShellState::Flying {
                continue;
            }
            let shell_handle = shell
                .body
                .expect("shell should always have a physics body once spawned");
            let shell_collider = self.physics.collider_of(shell_handle);
            let (base_dmg_min, base_dmg_max) = if shell.owner == Owner::Player {
                (PLAYER_DAMAGE_MIN, PLAYER_DAMAGE_MAX)
            } else {
                (ENEMY_DAMAGE_MIN, ENEMY_DAMAGE_MAX)
            };
            // Scale by the shooter's own chassis class (see
            // TANK_CHASSIS_DAMAGE_FACTOR_BY_ROW) - a std-class shooter deals
            // exactly the base range above, unchanged from before this
            // table existed.
            let chassis_factor = TANK_CHASSIS_DAMAGE_FACTOR_BY_ROW[shell.shooter_row as usize];
            let dmg_min = base_dmg_min * chassis_factor;
            let dmg_max = base_dmg_max * chassis_factor;

            let Some(target) =
                find_shell_target(&self.world, &self.physics, player, &walls, shell_collider)
            else {
                continue;
            };

            // A hit always flashes and detonates the shell, even against an
            // already-wrecked tank or a Wall - only the damage/kill/
            // knockback below (meaningless against a wreck, and not
            // applicable to a Wall at all) is gated per-target.
            self.impact_flashes.push(Shockwave {
                center: shell.position,
                time: 0.0,
            });
            shell.detonate();

            match target {
                ShellTarget::PlayerTank => {
                    let mut q = self.world.query_one::<&mut Tank>(player);
                    let player_tank = q.get().expect("player entity always has a Tank");
                    if !player_tank.is_wreck() {
                        let dmg = rng.random_range(dmg_min..dmg_max);
                        player_tank.damage = (player_tank.damage + dmg).min(MAX_DAMAGE);
                        player_tank.mark_hit();
                        if player_tank.is_wreck() {
                            kills.push((player_tank.position, false));
                        } else {
                            shell_impact(player_tank, shell, &mut self.physics);
                        }
                    }
                }
                ShellTarget::EnemyTank(entity) => {
                    let mut q = self.world.query_one::<&mut Tank>(entity);
                    let tank = q.get().expect("shell target entity always has a Tank");
                    if !tank.is_wreck() {
                        let dmg = rng.random_range(dmg_min..dmg_max);
                        tank.damage = (tank.damage + dmg).min(MAX_DAMAGE);
                        tank.mark_hit();
                        if tank.is_wreck() {
                            kills.push((tank.position, true));
                        } else {
                            shell_impact(tank, shell, &mut self.physics);
                        }
                    }
                }
                ShellTarget::Frog(entity) => {
                    let (now_dead, frog_pos, can_hop, hop_distance) = {
                        let mut q = self.world.query_one::<&mut Frog>(entity);
                        let frog = q.get().expect("shell target entity always has a Frog");
                        if frog.is_dead() {
                            (true, frog.position, false, 0.0)
                        } else {
                            let dmg = rng.random_range(dmg_min..dmg_max);
                            frog.damage(dmg);
                            (
                                frog.is_dead(),
                                frog.position,
                                frog.can_hop(),
                                frog.hop_distance(),
                            )
                        }
                    };
                    if now_dead {
                        self.shock = Some(Shockwave {
                            center: frog_pos,
                            time: 0.0,
                        });
                    } else if can_hop {
                        // Try to hop away from this shot - see
                        // frog_hop_target's own doc comment for the search.
                        // Obstacle positions collected fresh each hit
                        // (infrequent - only on an actual frog hit, not
                        // every frame) rather than threaded through from
                        // elsewhere, since nothing else in this loop needs
                        // them.
                        let obstacle_positions: Vec<Position> = self
                            .world
                            .query::<&Obstacle>()
                            .iter()
                            .map(|o| o.position)
                            .collect();
                        if let Some(new_pos) = frog_hop_target(
                            &mut rng,
                            frog_pos,
                            shell.velocity,
                            hop_distance,
                            &obstacle_positions,
                            width,
                            height,
                        ) {
                            // start_hop only records where the hop is
                            // headed - it doesn't move `position` itself
                            // (see its own doc comment); the per-frame tick
                            // loop below carries it there smoothly and
                            // keeps the physics body in step.
                            with_frog_mut(&self.world, entity, |f| f.start_hop(new_pos));
                        }
                    }
                }
                ShellTarget::Obstacle(entity) => {
                    let mut q = self.world.query_one::<&mut Obstacle>(entity);
                    let obstacle = q
                        .get()
                        .expect("shell target entity always has an Obstacle");
                    let dmg = rng.random_range(dmg_min..dmg_max);
                    obstacle.damage(dmg);
                }
                ShellTarget::Wall => {}
            }
        }
        // Remove physics bodies for shells finishing their bang animation
        // this frame, then despawn them. Collected first, then applied -
        // hecs doesn't allow despawning while a query over the same world is
        // still active (see the `pending_shells` comment above for why).
        let done_shells: Vec<_> = self
            .world
            .query::<(Entity, &Shell)>()
            .iter()
            .filter(|(_, s)| s.done)
            .map(|(e, s)| (e, s.body))
            .collect();
        for (entity, body) in done_shells {
            if let Some(handle) = body {
                self.physics.remove_body(handle);
            }
            self.world.despawn(entity).ok();
        }

        // Remove physics bodies for any Crate an obstacle broke this frame,
        // then despawn it - same collect-then-apply pattern as the shell
        // cleanup just above.
        let destroyed_obstacles: Vec<_> = self
            .world
            .query::<(Entity, &Obstacle)>()
            .iter()
            .filter(|(_, o)| o.destroyed)
            .map(|(e, o)| (e, o.body))
            .collect();
        for (entity, body) in destroyed_obstacles {
            self.physics.remove_body(body);
            self.world.despawn(entity).ok();
        }

        // Every tank destroyed this frame gets a shockwave (the most recent
        // kill's ring is the one that plays - see the field comment on
        // `shock`) plus a small splash of damage to nearby tanks on the
        // opposing side. A `while let ... pop()` rather than a plain `for`
        // because `apply_explosion` can push a fresh kill onto this same vec
        // (a splash chip finishing off an already-critical tank) - draining
        // it this way picks those up too. Always terminates: a tank can only
        // ever land in `kills` once (every path that pushes to it is gated
        // by `is_wreck()`), so the vec can only shrink toward empty as tanks
        // are used up.
        while let Some((center, victim_was_enemy)) = kills.pop() {
            self.shock = Some(Shockwave { center, time: 0.0 });
            self.apply_explosion(center, victim_was_enemy, &mut rng, &mut kills);
        }

        // Check for a round end. Losing (player destroyed, or the frog
        // dying) takes precedence over winning in case the last enemy and
        // the player/frog die on the same frame.
        let frog = self.frog.expect("frog entity spawned in init");
        if with_tank(&self.world, player, |t| t.is_wreck())
            || with_frog(&self.world, frog, Frog::is_dead)
        {
            self.end_round(Outcome::Lost);
        } else if self
            .world
            .query::<&Tank>()
            .with::<&Ai>()
            .iter()
            .all(|t| t.is_wreck())
        {
            self.end_round(Outcome::Won);
        }
    }

    /// Enter the end-of-round state and start the restart countdown.
    fn end_round(&mut self, outcome: Outcome) {
        self.outcome = outcome;
        self.restart_timer = RESTART_DELAY;
    }

    /// A tank's death deals a small splash of damage, plus an outward shove,
    /// to live tanks within EXPLOSION_RADIUS of `center`. The shove reaches
    /// every live tank regardless of side - a real shockwave doesn't check
    /// allegiance - but the damage stays side-restricted exactly as before:
    /// only the side opposing whoever died takes the chip of extra damage
    /// (an enemy's death can still catch the player nearby, but never chips
    /// other enemies standing next to it, and vice versa). Usually just a
    /// chip and a nudge - but if that chip finishes off an already-critical
    /// tank, `explosion_hit` reports it into `kills` so it still gets its
    /// own shockwave/knockback (see the while-loop in `update`), rather than
    /// dying silently with no effect.
    fn apply_explosion(
        &mut self,
        center: Position,
        victim_was_enemy: bool,
        rng: &mut rand::rngs::ThreadRng,
        kills: &mut Vec<(Position, bool)>,
    ) {
        let player = self.player.expect("player entity spawned in init");
        {
            let mut q = self.world.query_one::<&mut Tank>(player);
            let tank = q.get().expect("player entity always has a Tank");
            explosion_hit(
                tank,
                center,
                victim_was_enemy,
                false,
                &mut self.physics,
                rng,
                kills,
            );
        }
        for (tank, _ai) in self.world.query::<(&mut Tank, &mut Ai)>().iter() {
            explosion_hit(
                tank,
                center,
                !victim_was_enemy,
                true,
                &mut self.physics,
                rng,
                kills,
            );
        }
    }

    /// Snapshot every live tank's motion for the AI's collision avoidance:
    /// slot 0 is the player, slots 1.. are the enemies in this pass's
    /// iteration order. Wrecks are included at zero velocity so tanks still
    /// steer around burning hulks. Also returns a map from each enemy's
    /// entity to its slot in the snapshot: a *later*, separate query over
    /// the same Tank+Ai archetype (the actual AI-think loop in `update`)
    /// has no guaranteed-identical iteration order, so looking a slot up by
    /// entity identity - rather than re-deriving it from that later
    /// iteration's position - is what keeps `Ai::think`'s `my_index`
    /// correct regardless.
    fn motion_snapshot(&self) -> (Vec<Mover>, HashMap<Entity, usize>) {
        let to_mover = |t: &Tank| Mover {
            position: t.position,
            // A wreck can't move; treat it as stationary regardless of the velocity
            // it carried into death so tanks steer around it as a fixed obstacle.
            velocity: if t.is_wreck() {
                Position::new(0.0, 0.0)
            } else {
                t.velocity
            },
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

    /// How the current round is going. Read-only counterpart to `outcome`'s
    /// `pub(crate)` field, for external inspection - e.g. `src/bin/probe.rs`
    /// - that has no reason to see `world`/`Entity` itself, only the result.
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    /// A snapshot of every tank (player + enemies) currently in play, for
    /// external inspection without touching `world`/`Entity` directly - see
    /// `TankSnapshot` and `src/bin/probe.rs`, and this module's doc comment
    /// on driving/observing a round headlessly.
    pub fn tank_snapshots(&self) -> Vec<TankSnapshot> {
        let player = self.player.expect("player entity spawned in init");
        self.world
            .query::<(Entity, &Tank)>()
            .iter()
            .map(|(entity, tank)| TankSnapshot {
                is_player: entity == player,
                position: tank.position,
                rotation: tank.rotation,
                // The tank's *actual* physics velocity (post accel/decel
                // curve), not `tank.velocity` - that field holds the
                // instantly-snapped commanded target `drive_tank` chases
                // toward, not what the body is really doing this frame. See
                // `TANK_DECEL_CURVE_RATE` in lib.rs: this is what you want to
                // watch to verify the braking curve headlessly.
                velocity: self.physics.velocity(
                    tank.body
                        .expect("tank should always have a physics body once spawned"),
                ),
                damage: tank.damage,
                shells_ammo: tank.shells_ammo,
                is_wreck: tank.is_wreck(),
            })
            .collect()
    }
}

/// Read-only summary of one tank's externally-visible state, returned by
/// `Game::tank_snapshots`. A non-player tank is always an enemy - the world
/// field's doc comment on `Ai` is the source of truth for that invariant.
pub struct TankSnapshot {
    pub is_player: bool,
    pub position: Position,
    pub rotation: f32,
    pub velocity: Position,
    pub damage: f32,
    pub shells_ammo: i32,
    pub is_wreck: bool,
}

/// True if `rotation` faces along the x axis (Right/Left) rather than y
/// (Up/Down). `Tank::rotation` is always exactly one of 0/90/180/270 (see
/// `Dir::rotation`), so this is an exact match, not a fuzzy angle check.
fn facing_along_x(rotation: f32) -> bool {
    let r = rotation.rem_euclid(360.0);
    (r - 90.0).abs() < 1.0 || (r - 270.0).abs() < 1.0
}

/// Turn an intent into hull rotation plus a mass-aware acceleration impulse
/// nudging a tank's physics body toward its commanded velocity - not an
/// instant snap, and not a car-like blend either. `Tank::control` still
/// decides the *target* velocity (unchanged). Velocity always splits against
/// the hull's own facing (`Tank::rotation`, updated by `control` above),
/// never against whether a key happens to be held this frame: the axis along
/// the hull (forward/back) chases the target using the flat-force
/// `TANK_ACCEL_FORCE` when speeding up (linear ramp, see its doc comment in
/// `lib.rs`) or the exponential `TANK_DECEL_CURVE_RATE` curve when
/// slowing/reversing/coasting to a stop (see that constant's doc comment for
/// why braking is a curve rather than a matching flat force) - both scaled
/// by `Tank::mass` and `Tank::speed_factor`, so a damaged tank is sluggish
/// both ways. The axis perpendicular to the hull gets scrubbed toward zero by
/// `TANK_TURN_GRIP_FORCE` instead (a flat force like accel, weaker than
/// `TANK_ACCEL_FORCE` so this scrub is deliberately slower than the new
/// axis's own buildup - see its doc comment in `lib.rs` for the
/// drift-through-corners feel this produces), unscaled by mass/damage
/// factors beyond `Tank::mass` itself. Real tank tracks resist lateral
/// sliding mechanically, all the time, not just while the driver is actively
/// steering, so this applies whether or not a direction is currently held: a
/// corner reads as a genuine drift through the turn rather than the hull
/// snapping onto the new axis, and a ram/explosion/shell knockback that
/// shoves a tank sideways to wherever it's currently facing gets scrubbed
/// faster than a voluntary coasting stop would (via the much weaker
/// `TANK_DECEL_CURVE_RATE`), just not the near-instant kill it used to be.
/// (While a direction *is* held, `control` has already set `tank.rotation`
/// to that same direction this frame, so the axis this picks is identical to
/// the driven axis - unchanged from before.) Shared by the player and every
/// enemy so both drive identically; a free function (not a `Game` method) so
/// it can borrow `physics` and one `tank` independently of the rest of
/// `self`.
fn drive_tank(physics: &mut Physics, tank: &mut Tank, intent: Intent, dt: f32) {
    let handle = tank
        .body
        .expect("tank should always have a physics body once spawned");
    let current = physics.velocity(handle);
    let facing_before = tank.rotation;

    tank.control(intent.move_dir, intent.face);
    tank.ease_visual_rotation(dt);
    tank.ease_turret_visual_rotation(dt);
    let target = tank.velocity;

    // `control` above may have just snapped `rotation` to a new cardinal
    // direction (moving a new way, or an AI turning in place to aim) - the
    // physics collider doesn't rotate with the sprite (see
    // `Physics::spawn_tank`'s doc comment), so a non-square hull needs its
    // half-extents reoriented by hand whenever the facing crosses between
    // the X-axis and Y-axis cardinal pair. Only resize on an actual change,
    // not every frame a direction is merely held.
    if tank.rotation != facing_before {
        physics.resize_collider(
            physics.collider_of(handle),
            tank.hull_half_extents(facing_along_x(tank.rotation)),
        );
    }

    let along_x = facing_along_x(tank.rotation);
    let (current_on, target_on, current_off) = if along_x {
        (current.x, target.x, current.y)
    } else {
        (current.y, target.y, current.x)
    };

    let want_on = target_on - current_on;
    let speeding_up = want_on * current_on >= 0.0;
    let delta_on = if speeding_up {
        let max_on = TANK_ACCEL_FORCE * tank.speed_factor() / tank.mass() * dt;
        want_on.clamp(-max_on, max_on)
    } else {
        // Curved brake: close a `rate`-controlled fraction of the remaining
        // on-axis gap each frame (frame-rate independent), not a flat
        // per-frame cap - see TANK_DECEL_CURVE_RATE's doc comment in lib.rs.
        // Snap the last sliver to target once it's below TANK_DECEL_SNAP_PX
        // rather than trailing the exponential's asymptotic tail forever.
        let rate = TANK_DECEL_CURVE_RATE * tank.speed_factor() / tank.mass();
        let remaining_gap = want_on * (-rate * dt).exp();
        if remaining_gap.abs() < TANK_DECEL_SNAP_PX {
            want_on
        } else {
            want_on - remaining_gap
        }
    };

    let max_off = TANK_TURN_GRIP_FORCE / tank.mass() * dt;
    let delta_off = (-current_off).clamp(-max_off, max_off);

    let delta = if along_x {
        Position::new(delta_on, delta_off)
    } else {
        Position::new(delta_off, delta_on)
    };

    physics.apply_impulse(
        handle,
        Position::new(delta.x * tank.mass(), delta.y * tank.mass()),
    );
}

/// Read a tank's position back from its physics body after the world steps.
/// A free function for the same borrow-splitting reason as `drive_tank`.
fn sync_tank_from_physics(physics: &Physics, tank: &mut Tank) {
    let handle = tank
        .body
        .expect("tank should always have a physics body once spawned");
    tank.position = physics.position(handle);
}

/// True if `a` and `b`'s physics bodies currently have an active contact.
/// See `Physics::touching`.
fn tanks_touching(physics: &Physics, a: &Tank, b: &Tank) -> bool {
    let a = a
        .body
        .expect("tank should always have a physics body once spawned");
    let b = b
        .body
        .expect("tank should always have a physics body once spawned");
    physics.touching(a, b)
}

/// Which target (if any) a shell's collider is currently intersecting -
/// read-only, so `Game::update`'s shell-hit loop can decide what to do
/// about it (damage/kill/knockback for a tank, damage for an obstacle,
/// nothing extra for a wall) without this function needing to borrow
/// anything mutably itself.
enum ShellTarget {
    PlayerTank,
    EnemyTank(Entity),
    Frog(Entity),
    Obstacle(Entity),
    Wall,
}

/// Find what (if anything) `shell_collider` is intersecting this frame,
/// checked in priority order: the player's tank, then every enemy tank,
/// then the protect-objective frog, then every obstacle, then the four
/// battlefield walls - a shell hits at most one target, so the first match
/// wins. Replaces what used to be three separate, near-duplicate loops
/// (player/enemies/obstacles) plus a completely separate hand-rolled
/// screen-edge coordinate check standing in for walls (see `Shell::update`'s
/// old doc comment) - walls are real, queryable colliders now (see
/// `battlefield::spawn_walls`/`Game::walls`), so they go through the exact
/// same `Physics::intersecting` check as everything else. The frog is
/// checked ahead of obstacles for the same "living things before terrain"
/// reasoning that already puts tanks first.
fn find_shell_target(
    world: &hecs::World,
    physics: &Physics,
    player: Entity,
    walls: &[ColliderHandle; 4],
    shell_collider: ColliderHandle,
) -> Option<ShellTarget> {
    let player_sensor = with_tank(world, player, |t| {
        t.hit_sensor
            .expect("tank should always have a hit sensor once spawned")
    });
    if physics.intersecting(shell_collider, player_sensor) {
        return Some(ShellTarget::PlayerTank);
    }

    for (entity, tank) in world.query::<(Entity, &Tank)>().with::<&Ai>().iter() {
        let sensor = tank
            .hit_sensor
            .expect("tank should always have a hit sensor once spawned");
        if physics.intersecting(shell_collider, sensor) {
            return Some(ShellTarget::EnemyTank(entity));
        }
    }

    for (entity, frog) in world.query::<(Entity, &Frog)>().iter() {
        let collider = physics.collider_of(frog.body);
        if physics.intersecting(shell_collider, collider) {
            return Some(ShellTarget::Frog(entity));
        }
    }

    for (entity, obstacle) in world.query::<(Entity, &Obstacle)>().iter() {
        let collider = physics.collider_of(obstacle.body);
        if physics.intersecting(shell_collider, collider) {
            return Some(ShellTarget::Obstacle(entity));
        }
    }

    for &wall in walls {
        if physics.intersecting(shell_collider, wall) {
            return Some(ShellTarget::Wall);
        }
    }

    None
}

/// Try to find a landing spot for the frog's evasive hop, roughly
/// `distance` px away, continuing along `away_from_dir`'s direction (only
/// its angle matters, not its magnitude, so callers don't need to
/// normalize it first) - a shell's own velocity when it hit the frog (it
/// was heading *into* the frog, so carrying on that line moves the frog
/// further from whoever fired it), or simply `frog_pos` minus a too-close
/// tank's position (see the frog-avoidance section of `Game::update`) for
/// the other caller. Tries that ideal angle (plus a little random jitter,
/// so hops don't all look mechanically identical) first, then
/// FROG_HOP_ANGLE_FAN_DEG's offsets from it in turn, landing on the first
/// candidate that's both inside the battlefield (FROG_HOP_BOUNDS_MARGIN)
/// and clear of every current obstacle - so a frog backed into a corner or
/// wall still gets a real shot at finding *some* clear spot rather than
/// only ever trying the one exact "dead away" direction. Returns `None` if
/// every candidate is blocked: per the mechanic's own framing ("hop away if
/// it can"), this is a best-effort evasion, not a guaranteed one - the
/// caller just leaves the frog where it is when this happens.
fn frog_hop_target(
    rng: &mut rand::rngs::ThreadRng,
    frog_pos: Position,
    away_from_dir: Position,
    distance: f32,
    obstacle_positions: &[Position],
    width: f32,
    height: f32,
) -> Option<Position> {
    let clear = FROG_COLLIDER_HALF_EXTENT.0.max(FROG_COLLIDER_HALF_EXTENT.1) + OBSTACLE_CLEAR;
    let jitter = rng
        .random_range(-FROG_HOP_ANGLE_JITTER_DEG..FROG_HOP_ANGLE_JITTER_DEG)
        .to_radians();
    let base_angle = away_from_dir.y.atan2(away_from_dir.x) + jitter;
    for offset_deg in FROG_HOP_ANGLE_FAN_DEG {
        let angle = base_angle + offset_deg.to_radians();
        let candidate = Position::new(
            frog_pos.x + angle.cos() * distance,
            frog_pos.y + angle.sin() * distance,
        );
        let in_bounds = candidate.x >= FROG_HOP_BOUNDS_MARGIN
            && candidate.x <= width - FROG_HOP_BOUNDS_MARGIN
            && candidate.y >= FROG_HOP_BOUNDS_MARGIN
            && candidate.y <= height - FROG_HOP_BOUNDS_MARGIN;
        let clear_of_obstacles = obstacle_positions
            .iter()
            .all(|&p| candidate.distance_to(p) >= clear);
        if in_bounds && clear_of_obstacles {
            return Some(candidate);
        }
    }
    None
}

/// Roll a kind and a clear, corner-adjacent position for one new pickup and
/// spawn it into `world` - used identically both at round init (called
/// PICKUP_COUNT times, once every other entity for the round is down) and
/// by `Game::update` to top the field back up once the live count drops
/// below `PICKUP_COUNT`. Gathers clearance data (every obstacle - which
/// already includes every fortress tile, see
/// `battlefield::spawn_player_fortress` - every tank, the frog, and every
/// other live pickup) fresh from `world` on each call rather than taking it
/// as parameters, so the exact same function is correct whether it's round
/// start or five minutes into a round with some obstacles since destroyed
/// and every tank long since moved.
fn spawn_pickup(world: &mut hecs::World, rng: &mut rand::rngs::ThreadRng, width: f32, height: f32) {
    let obstacles: Vec<Position> = world.query::<&Obstacle>().iter().map(|o| o.position).collect();
    let tanks: Vec<Position> = world.query::<&Tank>().iter().map(|t| t.position).collect();
    let frog: Vec<Position> = world.query::<&Frog>().iter().map(|f| f.position).collect();
    let pickups: Vec<Position> = world.query::<&Pickup>().iter().map(|p| p.position).collect();
    let pos = battlefield::sample_corner_position(rng, width, height, |pos| {
        obstacles.iter().all(|&p| pos.distance_to(p) >= OBSTACLE_CLEAR)
            && tanks.iter().all(|&p| pos.distance_to(p) >= OBSTACLE_CLEAR)
            && frog.iter().all(|&p| pos.distance_to(p) >= OBSTACLE_CLEAR)
            && pickups
                .iter()
                .all(|&p| pos.distance_to(p) >= PICKUP_COLLECT_RADIUS * 2.0)
    });
    let kind = if rng.random_bool(0.5) {
        PickupKind::Health
    } else {
        PickupKind::Ammo
    };
    world.spawn((Pickup { kind, position: pos },));
}

/// Run `f` with read-only access to one specific tank entity's `Tank`
/// component. Backed by `World::query_one` - a dynamically borrow-checked,
/// shared (`&World`) query - rather than `query_one_mut`, specifically so
/// this can be called from *inside* code that already holds its own borrow
/// of `world` (e.g. from within a `world.query::<(&mut Tank, &mut
/// Ai)>()` iteration over every other tank): two different entities'
/// components never actually alias in practice here (see call sites), and
/// a shared receiver is what lets both borrows coexist at the Rust level in
/// the first place. `pub(crate)`: `game.rs::render` reuses this too (it only
/// ever needs read-only single-tank access).
pub(crate) fn with_tank<R>(world: &hecs::World, entity: Entity, f: impl FnOnce(&Tank) -> R) -> R {
    let mut q = world.query_one::<&Tank>(entity);
    let tank = q.get().expect("entity should have a Tank component");
    f(tank)
}

/// Same as `with_tank`, for the (always exactly one) protect-objective
/// `Frog`.
pub(crate) fn with_frog<R>(world: &hecs::World, entity: Entity, f: impl FnOnce(&Frog) -> R) -> R {
    let mut q = world.query_one::<&Frog>(entity);
    let frog = q.get().expect("entity should have a Frog component");
    f(frog)
}

/// Same as `with_frog`, but for mutable access.
fn with_frog_mut<R>(world: &hecs::World, entity: Entity, f: impl FnOnce(&mut Frog) -> R) -> R {
    let mut q = world.query_one::<&mut Frog>(entity);
    let frog = q.get().expect("entity should have a Frog component");
    f(frog)
}

/// Same as `with_tank`, but for mutable access to one specific tank.
fn with_tank_mut<R>(world: &hecs::World, entity: Entity, f: impl FnOnce(&mut Tank) -> R) -> R {
    let mut q = world.query_one::<&mut Tank>(entity);
    let tank = q.get().expect("entity should have a Tank component");
    f(tank)
}

/// Run `f` with simultaneous mutable access to two *different* tank
/// entities - e.g. the player and the one enemy currently ramming it, both
/// of which `ram` needs to mutate in the same call. Backed by
/// `World::query_disjoint_mut`, hecs's purpose-built API for exactly this case
/// ("query a fixed number of distinct entities in a uniquely borrowed
/// world... which would otherwise be forbidden by the unique borrow").
fn with_two_tanks_mut<R>(
    world: &mut hecs::World,
    a: Entity,
    b: Entity,
    f: impl FnOnce(&mut Tank, &mut Tank) -> R,
) -> R {
    let [ta, tb] = world.query_disjoint_mut::<&mut Tank, 2>([a, b]);
    f(
        ta.expect("entity should have a Tank component"),
        tb.expect("entity should have a Tank component"),
    )
}

/// Roll a fresh set of per-tank track-distortion parameters (see
/// TRACK_WOBBLE_AMP_MIN_DEG etc. in lib.rs) onto `tank`. Shared by the player
/// and every enemy spawn site in `Game::init` so both roll the same way.
fn roll_track_distortion(tank: &mut Tank, rng: &mut rand::rngs::ThreadRng) {
    tank.track_wobble_amp = rng.random_range(TRACK_WOBBLE_AMP_MIN_DEG..TRACK_WOBBLE_AMP_MAX_DEG);
    let wavelength = rng.random_range(TRACK_WOBBLE_WAVELENGTH_MIN..TRACK_WOBBLE_WAVELENGTH_MAX);
    // Radians per mark: each mark represents TRACK_SPACING px of travel, so a
    // full 2*PI cycle should span `wavelength` px, i.e. wavelength/TRACK_SPACING
    // marks.
    tank.track_wobble_freq = std::f32::consts::TAU * TRACK_SPACING / wavelength;
    tank.track_wobble_phase = rng.random_range(0.0..std::f32::consts::TAU);
    tank.track_scale_jitter =
        rng.random_range((1.0 - TRACK_SCALE_JITTER)..(1.0 + TRACK_SCALE_JITTER));
}

/// Roll this tank's wrecked-hull variant (see `Tank::wreck_col`) the first
/// frame it becomes a wreck, so `hull_col` has something to show - a no-op
/// on every other frame (already rolled, or not a wreck yet).
fn roll_wreck_col(tank: &mut Tank) {
    if tank.is_wreck() && tank.wreck_col.is_none() {
        tank.wreck_col = Some(TANK_WRECK_COLS[rand::rng().random_range(0..TANK_WRECK_COLS.len())]);
    }
}

/// Lay fresh tread marks along the distance a tank travelled this frame, dropping
/// one mark every TRACK_SPACING pixels. `before` is where the tank was at the
/// start of the frame; if it didn't actually move (blocked/idle) nothing is laid.
/// Also drives the tank sprite's own tread-animation frame (`Tank::hull_frame`,
/// see TANK_HULL_TRACK_FRAME_DISTANCE) off the same distance-moved signal -
/// a separate visual (the vehicle's own tread graphics vs. the marks it
/// leaves on the ground) but the same underlying input, so it's cheapest to
/// advance both from the one place that already computes `moved`. Stops
/// entirely - no ground marks, no hull_frame advance - once the hull is
/// disabled (TANK_HULL_DISABLED_DAMAGE) or wrecked; harmless for hull_frame
/// either way since `hull_col` ignores it once damage crosses that
/// threshold, but skipping it here avoids doing the work for nothing.
fn lay_tracks(tracks: &mut Vec<Track>, tank: &mut Tank, before: Position) {
    if tank.is_wreck() || tank.damage >= TANK_HULL_DISABLED_DAMAGE {
        return;
    }
    let moved = tank.position.distance_to(before);
    if moved <= 0.0 {
        // Idle: hold the resting tread frame rather than freeze mid-cycle.
        tank.hull_frame = 0;
        return;
    }
    tank.hull_anim_accum += moved;
    while tank.hull_anim_accum >= TANK_HULL_TRACK_FRAME_DISTANCE {
        tank.hull_anim_accum -= TANK_HULL_TRACK_FRAME_DISTANCE;
        tank.hull_frame = (tank.hull_frame + 1) % TANK_HULL_TRACK_COLS.len() as i32;
    }
    // Unit vector pointing back along the segment the tank just travelled, used
    // to place marks evenly along the path rather than stacking them at the end.
    let back = Vector2::new(
        (before.x - tank.position.x) / moved,
        (before.y - tank.position.y) / moved,
    );

    // Heading of this frame's straight-line travel chord (same 0-degrees-up,
    // clockwise convention as `Dir::rotation`) - not the hull's cosmetic
    // `tank.rotation`, which snaps instantly on a keypress (see
    // Tank::control). This is deliberately the *raw*, un-smoothed heading:
    // an earlier version of this function eased each mark's rotation toward
    // it over several marks to fake a rounder-looking turn, but that
    // fabricated curvature where none physically exists - a straight
    // reversal (e.g. right into left) never leaves its axis, so the real
    // heading jumps directly (90 <-> 270) with no genuine in-between
    // direction, and easing through one anyway visibly rotated marks sitting
    // on a perfectly straight line. A real 90-degree turn, by contrast,
    // *does* have real in-between headings - the tank's velocity has both
    // axes' components at once for a stretch while TANK_TURN_GRIP_FORCE
    // scrubs the old axis out and TANK_ACCEL_FORCE ramps the new one up (see
    // Game::drive_tank) - so sampling this raw heading densely enough (see
    // TRACK_SPACING) is enough on its own to trace that real curve, with
    // nothing invented. This also makes sliding sideways from a ram or
    // explosion lay tracks that follow where the tank is actually going, not
    // where it's pointed.
    let mut heading = (-back.x).atan2(back.y).to_degrees();
    if heading < 0.0 {
        heading += 360.0;
    }

    // Push marks out to the tank's rear edge so the trail comes out from behind
    // the hull and never pokes ahead of it.
    let rear = tank.hull_size() * 0.5;
    // Chassis weight (see TRACK_WEIGHT_SCALE_BY_ROW/TRACK_WEIGHT_OPACITY_BY_ROW)
    // on top of the base fraction/jitter, so a titan visibly presses a
    // bigger, darker mark than a scout instead of every row looking the same.
    let weight_scale = TRACK_WEIGHT_SCALE_BY_ROW[tank.row as usize];
    let scale = tank.scale * TRACK_SCALE_FRACTION * weight_scale * tank.track_scale_jitter;
    let max_opacity = TRACK_MAX_OPACITY * TRACK_WEIGHT_OPACITY_BY_ROW[tank.row as usize];

    tank.track_accum += moved;
    // Step up the path in TRACK_SPACING increments so the spacing stays even
    // regardless of speed or frame rate. `dist_back` is how far behind the current
    // position each mark sits.
    while tank.track_accum >= TRACK_SPACING {
        tank.track_accum -= TRACK_SPACING;
        let dist_back = rear + tank.track_accum;
        // Wobble this mark's rotation around the true heading using this
        // tank's own amplitude/frequency/phase (see roll_track_distortion),
        // so a straight drive doesn't stamp a perfectly repeated mark and
        // each tank's trail reads as its own tread pattern rather than
        // identical to every other tank's.
        let wobble = tank.track_wobble_amp
            * (tank.track_mark_count as f32 * tank.track_wobble_freq + tank.track_wobble_phase)
                .sin();
        tracks.push(Track {
            position: Position::new(
                tank.position.x + back.x * dist_back,
                tank.position.y + back.y * dist_back,
            ),
            rotation: heading + wobble,
            scale,
            max_opacity,
            age: 0.0,
        });
        tank.track_mark_count += 1;
    }
}

/// Give a tank a small shove along the shell's travel direction when it's
/// hit - a "tap", much weaker than a ram or explosion. Only ever called when
/// the tank isn't (and didn't just become) a wreck, matching `ram` and
/// `explosion_hit`'s convention that a fresh wreck doesn't get knocked
/// around; `shell.velocity` is a fixed-magnitude (`SHELL_SPEED`) vector set
/// once at spawn, so dividing by it is a cheap, exact way to get the unit
/// travel direction without a fresh sqrt.
fn shell_impact(tank: &mut Tank, shell: &Shell, physics: &mut Physics) {
    let dir = Vector2::new(
        shell.velocity.x / SHELL_SPEED,
        shell.velocity.y / SHELL_SPEED,
    );
    let handle = tank
        .body
        .expect("tank should always have a physics body once spawned");
    physics.apply_impulse(
        handle,
        Position::new(
            dir.x * SHELL_IMPACT_KNOCKBACK_SPEED * tank.mass(),
            dir.y * SHELL_IMPACT_KNOCKBACK_SPEED * tank.mass(),
        ),
    );
}

/// Apply one-off ramming damage to two touching tanks on opposing sides,
/// debounced by each tank's collision cooldown so continuous contact doesn't
/// drain damage every frame. Only ever called for player-vs-enemy contact -
/// enemies bumping each other are kept apart by the physics engine's own
/// contact response (see `Game::update`) without dealing damage, so this
/// doesn't need to guard against that case. Records the position of either
/// tank freshly killed by the collision, tagged with which side it was on,
/// so the caller can trigger its shockwave and (opposing-side-only)
/// explosion splash.
fn ram(
    a: &mut Tank,
    a_is_enemy: bool,
    b: &mut Tank,
    b_is_enemy: bool,
    physics: &mut Physics,
    rng: &mut rand::rngs::ThreadRng,
    kills: &mut Vec<(Position, bool)>,
) {
    if a.ram_cooldown <= 0.0 && b.ram_cooldown <= 0.0 {
        let a_was_wreck = a.is_wreck();
        let b_was_wreck = b.is_wreck();
        let dmg = rng.random_range(2.0..6.0);
        a.damage = (a.damage + dmg).min(MAX_DAMAGE);
        b.damage = (b.damage + dmg).min(MAX_DAMAGE);
        a.mark_hit();
        b.mark_hit();
        a.ram_cooldown = RAM_DAMAGE_COOLDOWN;
        b.ram_cooldown = RAM_DAMAGE_COOLDOWN;
        if !a_was_wreck && a.is_wreck() {
            kills.push((a.position, a_is_enemy));
        }
        if !b_was_wreck && b.is_wreck() {
            kills.push((b.position, b_is_enemy));
        }

        // Knockback: shove both tanks apart along the line between their
        // centers, harder the faster they were closing (using this frame's
        // `velocity`), split by mass so the lighter tank gets shoved further.
        // A tank that's now a wreck (freshly killed by this very hit, or
        // already one) doesn't move. The desired velocity change per tank is
        // still worked out by hand (same formula as before); what's real now
        // is the *application* - `physics.apply_impulse` on each tank's own
        // body, converting the desired push into `push * that tank's own
        // mass` so the resulting velocity change is exact.
        let dx = a.position.x - b.position.x;
        let dy = a.position.y - b.position.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist > 0.001 {
            let axis = Vector2::new(dx / dist, dy / dist);
            let rel_x = a.velocity.x - b.velocity.x;
            let rel_y = a.velocity.y - b.velocity.y;
            let impact_speed = (rel_x * rel_x + rel_y * rel_y).sqrt();
            let push = (impact_speed * KNOCKBACK_STRENGTH).min(KNOCKBACK_MAX_SPEED);
            let total_mass = a.mass() + b.mass();

            if !a.is_wreck() {
                let a_push = (push * 2.0 * b.mass() / total_mass).min(KNOCKBACK_MAX_SPEED);
                let handle = a
                    .body
                    .expect("tank should always have a physics body once spawned");
                physics.apply_impulse(
                    handle,
                    Position::new(axis.x * a_push * a.mass(), axis.y * a_push * a.mass()),
                );
            }
            if !b.is_wreck() {
                let b_push = (push * 2.0 * a.mass() / total_mass).min(KNOCKBACK_MAX_SPEED);
                let handle = b
                    .body
                    .expect("tank should always have a physics body once spawned");
                physics.apply_impulse(
                    handle,
                    Position::new(-axis.x * b_push * b.mass(), -axis.y * b_push * b.mass()),
                );
            }
        }
    }
}

/// Apply one tank's share of a nearby explosion: an outward knockback shove
/// that fades linearly with distance (full strength at the blast center,
/// nothing at EXPLOSION_RADIUS), reaching every live tank in range regardless
/// of side - a real shockwave doesn't check allegiance - plus, only when
/// `damage` is true (the caller passes this for the side opposing whoever
/// died, never a tank's own side), a small chip of extra damage. No-op on a
/// wreck (immovable, and past caring about a chip of damage) or a tank
/// outside the blast radius. If that chip of damage is what finishes `tank`
/// off, its position (tagged with `is_enemy`, `tank`'s own side) is pushed
/// onto `kills` so the caller gives it a shockwave/knockback of its own too,
/// same as a kill from ramming or a shell.
fn explosion_hit(
    tank: &mut Tank,
    center: Position,
    damage: bool,
    is_enemy: bool,
    physics: &mut Physics,
    rng: &mut rand::rngs::ThreadRng,
    kills: &mut Vec<(Position, bool)>,
) {
    if tank.is_wreck() {
        return;
    }
    let dx = tank.position.x - center.x;
    let dy = tank.position.y - center.y;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist > EXPLOSION_RADIUS {
        return;
    }

    if damage {
        let dmg = rng.random_range(EXPLOSION_DAMAGE_MIN..EXPLOSION_DAMAGE_MAX);
        tank.damage = (tank.damage + dmg).min(MAX_DAMAGE);
        tank.mark_hit();
        if tank.is_wreck() {
            kills.push((tank.position, is_enemy));
        }
    }

    // Push harder the closer the tank was to the blast, and divide by mass
    // relative to this tank's own *chassis-free* baseline (scale^2 alone,
    // the flat mass every tank shared before TANK_CHASSIS_MASS_FACTOR_BY_ROW
    // existed - deliberately not `Tank::default().mass()`, which would key
    // off whichever chassis `Tank::row`'s default (0, scout) happens to be
    // rather than the neutral `std`-class baseline EXPLOSION_KNOCKBACK_SPEED
    // was actually tuned against) so a heavier chassis (see `Tank::mass`)
    // resists the shove more and a lighter one flies further. As in `ram`,
    // the desired push is still worked out by hand; applying it as
    // `physics.apply_impulse` (impulse = push * this tank's own mass) is
    // what makes it a real physics impulse.
    let falloff = 1.0 - dist / EXPLOSION_RADIUS;
    let reference_mass = tank.scale * tank.scale;
    let push = (EXPLOSION_KNOCKBACK_SPEED * falloff * reference_mass / tank.mass())
        .min(KNOCKBACK_MAX_SPEED);
    let axis = if dist > 0.001 {
        Vector2::new(dx / dist, dy / dist)
    } else {
        // Degenerate case: sitting exactly on the blast center - push in an
        // arbitrary direction rather than dividing by zero.
        Vector2::new(1.0, 0.0)
    };
    let handle = tank
        .body
        .expect("tank should always have a physics body once spawned");
    physics.apply_impulse(
        handle,
        Position::new(axis.x * push * tank.mass(), axis.y * push * tank.mass()),
    );
}
