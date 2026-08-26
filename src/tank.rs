use rapier2d::prelude::{ColliderHandle, RigidBodyHandle};
use sola_raylib::prelude::*;

use crate::laser::LaserVariant;
use crate::{
    DAMAGE_SPEED_CURVE, DAMAGE_SPEED_FLOOR, MAX_DAMAGE, MAX_SHELLS, MINIGUN_MOUNT_SCALE,
    MINIGUN_CYCLE_SECONDS, MINIGUN_MOUNT_TEXTURE_SIZE, Position,
    SHADOW_DIR_X, SHADOW_DIR_Y, TANK_BROKEN_TURRET_COL, TANK_CHASSIS_MASS_FACTOR_BY_ROW,
    TANK_HULL_BBOX_BY_ROW, TANK_HULL_DISABLED_COL, TANK_HULL_DISABLED_DAMAGE, TANK_HULL_FRACTION,
    TANK_HULL_LIGHT_COL,
    TANK_HULL_LIGHT_DAMAGE, TANK_HULL_TRACK_COLS, TANK_PIVOT_REAR_FRACTION, TANK_SHADOW_OFFSET,
    TANK_SHADOW_OPACITY,
    TANK_SPEED, TANK_TEXTURE_SIZE, TANK_TURRET_BBOX_BY_ROW, TANK_TURRET_COL,
    TANK_TURRET_VISUAL_TURN_SPEED_DEG,
    TANK_VISUAL_TURN_SPEED_DEG, TANK_WRECK_COLS, WRECK_BURN_SECONDS,
};

/// The four movement/facing directions. rotation 0 == up, clockwise positive,
/// matching the sprite orientation and shell-spawn math.
#[derive(Clone, Copy, PartialEq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    /// Hull rotation in degrees for this direction.
    pub fn rotation(self) -> f32 {
        match self {
            Dir::Up => 0.0,
            Dir::Right => 90.0,
            Dir::Down => 180.0,
            Dir::Left => 270.0,
        }
    }

    /// Unit movement vector (screen space: +x right, +y down).
    pub fn vec(self) -> Vector2 {
        match self {
            Dir::Up => Vector2::new(0.0, -1.0),
            Dir::Down => Vector2::new(0.0, 1.0),
            Dir::Left => Vector2::new(-1.0, 0.0),
            Dir::Right => Vector2::new(1.0, 0.0),
        }
    }

    /// Cardinal direction from `from` toward `to`, choosing the dominant axis.
    pub fn toward(from: Position, to: Position) -> Dir {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        if dx.abs() >= dy.abs() {
            if dx >= 0.0 { Dir::Right } else { Dir::Left }
        } else if dy >= 0.0 {
            Dir::Down
        } else {
            Dir::Up
        }
    }
}

/// A twin-barrel chassis's second shell, waiting to fire a beat after the
/// first - see `Tank::pending_shot`.
#[derive(Clone, Copy)]
pub struct PendingShot {
    /// Seconds remaining until this shell fires.
    pub timer: f32,
    /// Same off-aim deflection as the first shell (see `Shell::spawn`'s
    /// `aim_offset` param) - a misfire skews both rounds identically.
    pub aim_offset: f32,
    /// This barrel's lateral offset (see `Shell::spawn`'s `lateral_offset`
    /// param) - the opposite side from the first shell's barrel.
    pub lateral_offset: f32,
}

/// A minigun burst in progress: `bullets_remaining` more bullets queued to
/// fire at MINIGUN_BULLET_DELAY_SECONDS spacing after the one that just
/// fired - see `Tank::minigun_burst`. Generalizes `PendingShot`'s "one
/// queued extra shot" to "N queued burst bullets" using the identical
/// tick-once-per-frame shape.
#[derive(Clone, Copy)]
pub struct MinigunBurst {
    pub bullets_remaining: u32,
    /// Seconds remaining until the next queued bullet fires.
    pub timer: f32,
    /// Same point-blank misfire skew as `PendingShot::aim_offset` - rolled
    /// once when the burst starts and shared by every bullet in it; each
    /// bullet additionally gets its own fresh MINIGUN_BULLET_SPREAD_DEG
    /// jitter on top of this at the moment it actually fires (see
    /// `simulation::fire_bullet`).
    pub aim_offset: f32,
}

/// Which weapon a tank's next trigger-pull fires - see `Tank::active_weapon`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActiveWeapon {
    Laser,
    Minigun,
    Shell,
}

pub struct Tank {
    /// Which of the 12 tank archetypes in scifi_tanks_sheet.png this tank
    /// draws (see TANK_VARIANTS/TANK_SPRITE_ORDER in simulation.rs). The hull
    /// and turret layers' columns within that row depend on this tank's
    /// current animation/damage state - see `hull_col`/`turret_col`.
    pub row: i32,
    /// Row in shells.png this tank's shells are drawn from (0..SHELL_VARIANTS)
    /// - matched to this tank's chassis (size class, accent colour) via
    /// TANK_SHELL_VARIANT_BY_ROW, set once at spawn (see Game::init) and
    /// fixed for this tank's whole life - every shell it ever fires,
    /// including both independent shots from a twin-barrel chassis (see
    /// `pending_shot`), uses this same single-barrel row.
    pub shell_variant: i32,
    /// A twin-barrel chassis's second shell, queued to fire
    /// TANK_TWIN_SHOT_DELAY_SECONDS after the first so the two rounds read
    /// as one barrel, then the other, rather than appearing in the same
    /// instant. `None` the rest of the time; a single-barrel chassis never
    /// sets this (see `TANK_BARREL_LATERAL_OFFSET_BY_ROW`). Ticked down and
    /// resolved once per frame in `Game::update`, at the same site this
    /// tank's own fire input is handled.
    pub pending_shot: Option<PendingShot>,
    /// A minigun burst in progress, queued the same way `pending_shot`
    /// above queues a twin-barrel chassis's second shell - generalized from
    /// "one queued extra shot" to "N queued burst bullets". `None` the rest
    /// of the time. Ticked/resolved once per frame in `Game::update`, at
    /// the same call sites `pending_shot` is already ticked for player and
    /// enemy.
    pub minigun_burst: Option<MinigunBurst>,
    /// Row in damage.png this tank's damage overlay is drawn from
    /// (0..DAMAGE_VARIANTS). Rolled once at spawn (see Game::init) and fixed
    /// for the tank's whole life, so its damage sequence reads as one
    /// consistent flavour rather than switching palettes between stages.
    pub damage_variant: i32,
    /// Center position on screen (pixels). A read-back mirror of `body`'s
    /// physics transform, synced once per frame after the physics world
    /// steps (see `Game::update`) - nothing else should write this by hand.
    pub position: Position,
    /// Facing angle in degrees. Snaps instantly on a direction change -
    /// physics (`Game::drive_tank`), aiming (`Shell::spawn`) and track
    /// heading all key off this and none of it should lag input. For the
    /// on-screen hull animation, see `visual_rotation` instead.
    pub rotation: f32,
    /// Sprite-only facing angle in degrees, eased toward `rotation` each
    /// frame (see `ease_visual_rotation`/`TANK_VISUAL_TURN_SPEED_DEG`)
    /// instead of snapping with it, so a turn visibly swings the hull over a
    /// few frames. Read only by `draw_tank`/`draw_tank_shadow` - nothing
    /// gameplay-relevant should ever key off this.
    pub visual_rotation: f32,
    /// Turret-only sprite facing angle in degrees, eased toward `rotation`
    /// independently of `visual_rotation` (see
    /// `ease_turret_visual_rotation`/`TANK_TURRET_VISUAL_TURN_SPEED_DEG`) -
    /// faster than the hull's own ease, so the turret visibly leads a turn
    /// while the hull swings around to catch up. Read only by `draw_tank`/
    /// `draw_tank_shadow`.
    pub turret_visual_rotation: f32,
    /// Seconds accumulated toward the minigun barrel-cluster overlay's next
    /// "hot barrel" frame swap (see `draw_minigun_mount`), advanced while
    /// `minigun_burst` is active (see `tick_minigun_spin`) and held in place
    /// - not reset to 0 - the rest of the time, so the mount doesn't
    /// visually snap back to frame 0 between bursts. Wrapped to
    /// `MINIGUN_CYCLE_SECONDS * 3.0` (one full lap of the 3 frames) rather
    /// than growing unbounded.
    ///
    /// This deliberately drives a discrete frame swap, not a continuous
    /// rotation: `minigun_mount.png`'s barrels point along the ground plane
    /// toward the target, so their real rotation axis is edge-on to this
    /// game's top-down camera, not face-on to it - spinning the whole
    /// sprite in the screen plane would read as a helicopter rotor seen
    /// from above, not a side-mounted minigun. Cycling which barrel reads
    /// as freshly-fired fakes the same "rounds cycling through" idea
    /// correctly for this camera angle instead. See
    /// `tools/spritegen/gen_minigun_mount.py`'s module doc comment.
    pub minigun_cycle_timer: f32,
    /// Index into TANK_HULL_TRACK_COLS (0..4) picking which tread-animation
    /// hull frame is currently drawn - see `hull_col`/
    /// TANK_HULL_TRACK_FRAME_DISTANCE. Only consulted while the tank is alive
    /// and below TANK_HULL_LIGHT_DAMAGE; damaged/wrecked hulls hold a fixed
    /// frame instead.
    pub hull_frame: i32,
    /// World px of travel accumulated toward the next `hull_frame` advance -
    /// see `simulation::lay_tracks`. Deliberately separate from
    /// `track_accum` below: that one paces the ground-decal tread marks in
    /// track.rs, an unrelated system with its own spacing: reusing it here
    /// would tie two independently-tuned animations together.
    pub hull_anim_accum: f32,
    /// Which of TANK_WRECK_COLS this tank uses once it becomes a wreck -
    /// `None` until then. Rolled once, the frame `is_wreck()` first becomes
    /// true (see `simulation::Game::update`, right where `tick_wreck` is
    /// called), and kept for the rest of the tank's lifetime rather than
    /// re-rolled each frame, so a field of wrecks shows genuine variety
    /// instead of flickering between variants.
    pub wreck_col: Option<i32>,
    /// How much to scale the 32x32 sprite when drawn.
    pub scale: f32,
    /// Movement speed in pixels per second (player and enemies differ).
    pub speed: f32,
    /// Accumulated damage, 0 (pristine) .. MAX_DAMAGE (destroyed wreck).
    pub damage: f32,
    /// Remaining shells this tank can fire before it must recharge.
    pub shells_ammo: i32,
    /// Remaining laser charges (see `pickup::PickupKind::Laser`,
    /// `laser.rs`). While positive, firing consumes one charge and resolves
    /// an instant beam hit instead of a normal shell (see `simulation.rs`'s
    /// fire dispatch); at zero this tank fires shells as usual.
    pub laser_charges: i32,
    /// Which `laser::LaserVariant` the current charge batch fires as -
    /// rolled fresh on each `PickupKind::Laser` pickup (see
    /// `LASER_BLUE_PICKUP_CHANCE`), meaningless while `laser_charges == 0`.
    pub laser_variant: LaserVariant,
    /// Remaining minigun ammo (see `pickup::PickupKind::Minigun`,
    /// `bullet.rs`). Pickup-only, no passive regen - mirrors `laser_charges`
    /// exactly. While positive and no laser is charged, firing starts a
    /// burst of MINIGUN_BURST_SIZE individually-simulated `bullet::Bullet`s
    /// instead of a normal shell - see `Tank::active_weapon`.
    pub minigun_ammo: i32,
    /// Seconds accumulated toward recharging the next shell.
    pub recharge_timer: f32,
    /// Seconds remaining before this tank may fire again (player only - see
    /// PLAYER_FIRE_INTERVAL; enemies are gated separately by Ai's own
    /// fire_timer). Ticked down alongside ram_cooldown every frame regardless
    /// of owner, since it's harmless/unused idle state for enemies.
    pub fire_cooldown: f32,
    /// Seconds remaining before this tank can take ramming damage again.
    pub ram_cooldown: f32,
    /// Seconds remaining to show this tank's overhead health bar (see
    /// `Game::render`) - reset to HEALTH_BAR_OVERHEAD_SECONDS by `mark_hit`
    /// whenever this tank takes damage (shell, ram, or explosion splash),
    /// ticked down every frame alongside `fire_cooldown`/`ram_cooldown`
    /// (`Game::update`), and faded out over the last
    /// HEALTH_BAR_OVERHEAD_FADE_SECONDS of that window rather than
    /// vanishing abruptly.
    pub hit_flash_timer: f32,
    /// Seconds spent as a wreck. Once it exceeds WRECK_BURN_SECONDS the fire
    /// dies out and the tank becomes a static charred "dead" hulk.
    pub wreck_timer: f32,
    /// Distance travelled (pixels) since the last track mark was dropped.
    pub track_accum: f32,
    /// Number of track marks this tank has laid this round - the phase input
    /// to its track wobble (see `track_wobble_phase`); incremented once per
    /// mark in `Game::lay_tracks`.
    pub track_mark_count: u32,
    /// This tank's track-wobble amplitude in degrees, rolled once at spawn
    /// (see TRACK_WOBBLE_AMP_MIN_DEG/MAX_DEG) and fixed for the round -
    /// how far each mark's rotation swings side to side from the tank's
    /// actual heading.
    pub track_wobble_amp: f32,
    /// This tank's track-wobble angular frequency, in radians per mark
    /// (derived from a randomized wavelength at spawn - see
    /// TRACK_WOBBLE_WAVELENGTH_MIN/MAX) - how tight the wobble's cycles are.
    pub track_wobble_freq: f32,
    /// This tank's track-wobble phase offset in radians, rolled once at
    /// spawn, so tanks that happen to share a similar amplitude/frequency
    /// don't wobble in lockstep.
    pub track_wobble_phase: f32,
    /// Fixed per-tank multiplier on track mark scale (see
    /// TRACK_SCALE_JITTER), rolled once at spawn.
    pub track_scale_jitter: f32,
    /// Commanded velocity this frame (pixels per second): the movement
    /// direction times speed, or zero when not moving. Set by `control` and
    /// read by the AI's predictive collision avoidance, and by
    /// `Game::drive_tank` to derive how much of the physics body's actual
    /// velocity is "ours" versus residual momentum from a ram/explosion
    /// impulse (see that function).
    pub velocity: Vector2,
    /// This tank's rapier rigid body, once spawned into the physics world
    /// (see `Game::init`/`physics::Physics::spawn_tank`).
    pub body: Option<RigidBodyHandle>,
    /// This tank's hull shell-hit sensor - a second collider on the same
    /// `body` (see `physics::Physics::add_hit_sensor`), sized/positioned
    /// exactly like the solid hull collider (`hull_half_extents`) but with
    /// its own owner-exclusion `InteractionGroups` filter, which the solid
    /// collider can't carry without also filtering *movement* collisions
    /// (tank-vs-tank ramming, tank-vs-wall) the same way - see
    /// `find_shell_target`. Paired with `turret_hit_sensor` below for the
    /// turret+barrel portion of the same hit test.
    pub hit_sensor: Option<ColliderHandle>,
    /// This tank's turret+barrel shell-hit sensor - same idea as
    /// `hit_sensor` above, sized/positioned from `turret_bbox_world`
    /// instead of the hull box. Splitting the hit test into these two boxes
    /// (rather than one box covering both, or one oversized box padded to
    /// cover both) is what lets a shot land anywhere on the visible hull
    /// *or* the barrel and register, without also counting the transparent
    /// padding around either shape as a hit - this pair of boxes is exactly
    /// what the "I" key debug inspect overlay (`game.rs::draw_tank_inspect`)
    /// already draws, which is how this shape was chosen and checked
    /// against the art before being wired in here.
    pub turret_hit_sensor: Option<ColliderHandle>,
    /// This tank's collision-group slot (see `physics::owner_group`), set
    /// once at spawn (`Game::init`) - 0 for the player, `n` for the nth
    /// enemy spawned. Reused whenever this tank fires (see `Game::update`)
    /// to rebuild that same group for its shell's shooter-exclusion filter
    /// and to tag the shell's `shell::Owner`, without needing this tank's
    /// position in any spawn-order list.
    pub owner_slot: usize,
}

impl Default for Tank {
    fn default() -> Self {
        Self {
            row: 0,
            shell_variant: 0,
            pending_shot: None,
            minigun_burst: None,
            damage_variant: 0,
            position: Position::default(),
            rotation: 0.0,
            visual_rotation: 0.0,
            turret_visual_rotation: 0.0,
            minigun_cycle_timer: 0.0,
            hull_frame: 0,
            hull_anim_accum: 0.0,
            wreck_col: None,
            scale: 2.0, // 3.0,
            speed: TANK_SPEED,
            damage: 0.0,
            shells_ammo: MAX_SHELLS,
            laser_charges: 0,
            laser_variant: LaserVariant::Red,
            minigun_ammo: 0,
            recharge_timer: 0.0,
            fire_cooldown: 0.0,
            ram_cooldown: 0.0,
            hit_flash_timer: 0.0,
            wreck_timer: 0.0,
            track_accum: 0.0,
            track_mark_count: 0,
            track_wobble_amp: 0.0,
            track_wobble_freq: 0.0,
            track_wobble_phase: 0.0,
            track_scale_jitter: 1.0,
            velocity: Vector2::new(0.0, 0.0),
            body: None,
            hit_sensor: None,
            turret_hit_sensor: None,
            owner_slot: 0,
        }
    }
}

impl Tank {
    /// Side length of this tank on screen (square sprite).
    pub fn size(&self) -> f32 {
        TANK_TEXTURE_SIZE * self.scale
    }

    /// True once the tank has taken maximum damage (a burning wreck).
    pub fn is_wreck(&self) -> bool {
        self.damage >= MAX_DAMAGE
    }

    /// How much damage has hurt this tank's mobility, from 1.0 (pristine) down
    /// to DAMAGE_SPEED_FLOOR (about to wreck). Holds close to 1.0 through
    /// light and moderate damage, then falls off harder as damage nears the
    /// max - a limp rather than a linear taper. Scales both top speed
    /// (`effective_speed`) and how fast the tank can reach it
    /// (`TANK_ACCEL_FORCE`/`TANK_DECEL_CURVE_RATE` in `Game::drive_tank`), so a
    /// damaged tank is sluggish to speed up too, not just capped lower.
    pub fn speed_factor(&self) -> f32 {
        let hurt = (self.damage / MAX_DAMAGE).clamp(0.0, 1.0);
        DAMAGE_SPEED_FLOOR + (1.0 - DAMAGE_SPEED_FLOOR) * (1.0 - hurt.powf(DAMAGE_SPEED_CURVE))
    }

    /// This tank's current top speed, reduced as it takes damage; see
    /// `speed_factor`.
    pub fn effective_speed(&self) -> f32 {
        self.speed * self.speed_factor()
    }

    /// True once a wreck has finished burning and settled into a dead hulk.
    pub fn is_dead(&self) -> bool {
        self.is_wreck() && self.wreck_timer >= WRECK_BURN_SECONDS
    }

    /// Which atlas column to draw this tank's hull from - a four-tier
    /// escalation matching the sheet's own damage ladder: the rolled wreck
    /// variant once it's a wreck (`wreck_col`, falling back to the first
    /// wreck variant on the off chance this is read before that roll
    /// happens), the disabled art once it's taken heavy but non-fatal damage
    /// (TANK_HULL_DISABLED_DAMAGE), the cosmetic "light" art once it's taken
    /// moderate damage (TANK_HULL_LIGHT_DAMAGE) - still fully mobile, so this
    /// tier is static art rather than consulting `hull_frame` - otherwise
    /// whichever tread-animation frame `hull_frame` currently points at.
    pub fn hull_col(&self) -> i32 {
        if self.is_wreck() {
            self.wreck_col.unwrap_or(TANK_WRECK_COLS[0])
        } else if self.damage >= TANK_HULL_DISABLED_DAMAGE {
            TANK_HULL_DISABLED_COL
        } else if self.damage >= TANK_HULL_LIGHT_DAMAGE {
            TANK_HULL_LIGHT_COL
        } else {
            TANK_HULL_TRACK_COLS[self.hull_frame as usize]
        }
    }

    /// Which atlas column to draw this tank's turret from: the severed/
    /// broken turret once it's a wreck, otherwise the intact turret.
    pub fn turret_col(&self) -> i32 {
        if self.is_wreck() {
            TANK_BROKEN_TURRET_COL
        } else {
            TANK_TURRET_COL
        }
    }

    /// Which weapon this tank's next trigger-pull actually fires, by
    /// priority: a charged laser first (most powerful, depleted first),
    /// then minigun ammo, then a traditional shell last. Purely picks the
    /// *tier* - whether that tier's own ammo is actually sufficient to fire
    /// *this instant* is still checked at each dispatch site in
    /// `Game::update` (a twin-barrel chassis needing 2 shells per shot can
    /// still be `ActiveWeapon::Shell` while short of the 2 it needs, exactly
    /// as before this existed).
    pub fn active_weapon(&self) -> ActiveWeapon {
        if self.laser_charges > 0 {
            ActiveWeapon::Laser
        } else if self.minigun_ammo > 0 {
            ActiveWeapon::Minigun
        } else {
            ActiveWeapon::Shell
        }
    }

    /// Advance the minigun's barrel-cycle timer while a burst is active;
    /// hold it otherwise (see `minigun_cycle_timer`'s doc comment). Called
    /// once per frame for every tank alongside `tick_recharge`/`tick_wreck`
    /// in `Game::update`'s unified per-tank timer loop.
    pub fn tick_minigun_spin(&mut self, dt: f32) {
        if self.minigun_burst.is_some() {
            self.minigun_cycle_timer =
                (self.minigun_cycle_timer + dt) % (MINIGUN_CYCLE_SECONDS * 3.0);
        }
    }

    /// Which of `minigun_mount.png`'s 3 "hot barrel" frames to draw right
    /// now - see `minigun_cycle_timer`'s doc comment for why this is a
    /// discrete frame index, not a rotation angle.
    fn minigun_cycle_frame(&self) -> i32 {
        ((self.minigun_cycle_timer / MINIGUN_CYCLE_SECONDS) as i32).clamp(0, 2)
    }

    /// Small phase offset (seconds) derived from screen position so that several
    /// burning tanks don't animate their smoke/fire in perfect lockstep.
    pub fn anim_phase(&self) -> f32 {
        (self.position.x + self.position.y) * 0.01
    }

    /// Collision footprint side length: the visible hull, not the full sprite
    /// tile, so tanks can close the gap left by the sprite's transparent padding.
    /// A uniform-square approximation used by the AI's avoidance radius, the
    /// ground-decal rear-edge offset, and spawn-clearance checks - all of
    /// which only need an approximate footprint. The tank's actual physics
    /// collider is sized more precisely per row - see `hull_half_extents`.
    pub fn hull_size(&self) -> f32 {
        self.size() * TANK_HULL_FRACTION
    }

    /// Physics-collider half-extents (x, y) for this tank's hull, in world
    /// px, oriented for the given facing - `along_x` true when facing
    /// Left/Right (width and height swap from the sprite's own "facing up"
    /// reference frame), matching `simulation::facing_along_x`. Distinct from
    /// `hull_size` above: this is the real per-row rectangle
    /// (TANK_HULL_BBOX_BY_ROW) the physics collider is actually sized from -
    /// see `simulation::drive_tank`/`Physics::resize_collider`.
    pub fn hull_half_extents(&self, along_x: bool) -> (f32, f32) {
        let (w, h) = TANK_HULL_BBOX_BY_ROW[self.row as usize];
        let (w, h) = if along_x { (h, w) } else { (w, h) };
        (w * 0.5 * self.scale, h * 0.5 * self.scale)
    }

    /// True when `rotation` currently faces Left/Right rather than Up/Down -
    /// shared by every method below that needs to know which axis is the
    /// hull's "long" one. `rotation` is always exactly one of
    /// `Dir::rotation()`'s four values (see `Tank::control`), so a plain
    /// `==` match is exact here, no epsilon needed.
    fn facing_along_x(&self) -> bool {
        self.rotation == Dir::Right.rotation() || self.rotation == Dir::Left.rotation()
    }

    /// This tank's hull collider footprint in world space right now -
    /// `position` as the center (the hull box is symmetric front-to-back
    /// and side-to-side, so it never needs an offset) and
    /// `hull_half_extents` oriented for the current facing. A convenience
    /// over calling `hull_half_extents` directly for a caller that doesn't
    /// already have `along_x` in hand (shell-hit testing, see
    /// `simulation::swept_shell_target`) - `simulation::drive_tank`'s own
    /// resize call site still calls `hull_half_extents` directly since it
    /// already knows the along_x it's transitioning *to*.
    pub fn hull_bbox_world(&self) -> (Position, Position) {
        let (hw, hh) = self.hull_half_extents(self.facing_along_x());
        (self.position, Position::new(hw, hh))
    }

    /// World-space center and half-extents of this tank's turret+barrel
    /// bounding box (`TANK_TURRET_BBOX_BY_ROW`) at its current `rotation` -
    /// backs both the "I" key debug inspect overlay
    /// (`game.rs::draw_tank_inspect`) and `Tank::turret_hit_sensor`'s
    /// shape/position (kept in sync on every facing change by
    /// `simulation::drive_tank`, same trigger as the hull collider's own
    /// resize). Unlike `hull_half_extents`'s `along_x` swap (safe because
    /// the hull box is roughly centered on the tank), the turret+barrel box
    /// is off-center - the barrel extends it well past the tile center
    /// toward the front - so this needs a real per-direction rotation of
    /// both the offset and the extents, not just a width/height swap.
    pub fn turret_bbox_world(&self) -> (Position, Position) {
        let (x0, y0, x1, y1) = TANK_TURRET_BBOX_BY_ROW[self.row as usize];
        // Local "facing up" frame, origin at the tile center (16,16) -
        // same convention TANK_HULL_BBOX_BY_ROW's own values are measured
        // in.
        let half = TANK_TEXTURE_SIZE * 0.5;
        let local_cx = (x0 + x1 + 1.0) * 0.5 - half;
        let local_cy = (y0 + y1 + 1.0) * 0.5 - half;
        let local_hw = (x1 - x0 + 1.0) * 0.5;
        let local_hh = (y1 - y0 + 1.0) * 0.5;
        // Clockwise rotation of the local (x,y) offset by `rotation` - the
        // same mapping `Dir::vec` encodes (Up's local "forward", -y, must
        // land on each direction's own forward vector): identity at Up,
        // (x,y)->(-y,x) at Right, negate-both at Down, (x,y)->(y,-x) at
        // Left. Half-extents swap width/height exactly when the offset
        // rotation does (Right/Left), matching `hull_half_extents`'s own
        // along_x swap.
        let (ox, oy, hw, hh) = if self.rotation == Dir::Up.rotation() {
            (local_cx, local_cy, local_hw, local_hh)
        } else if self.rotation == Dir::Right.rotation() {
            (-local_cy, local_cx, local_hh, local_hw)
        } else if self.rotation == Dir::Down.rotation() {
            (-local_cx, -local_cy, local_hw, local_hh)
        } else {
            (local_cy, -local_cx, local_hh, local_hw)
        };
        let center = Position::new(
            self.position.x + ox * self.scale,
            self.position.y + oy * self.scale,
        );
        (center, Position::new(hw * self.scale, hh * self.scale))
    }

    /// A safe circular over-approximation of this tank's real (rectangular,
    /// per-row) physics footprint at its current facing - the true
    /// bounding-circle radius (hypot of both half-extents from
    /// `hull_half_extents`), rather than the uniform `hull_size() * 0.5`
    /// approximation. Used anywhere collision math is circle-based (AI
    /// predictive avoidance - `ai.rs`'s `Mover.radius`/`AvoidCtx.radius`) so
    /// it never assumes a tank is smaller than the collider
    /// `Physics::resize_collider` actually gave it - a mismatch that let the
    /// AI drive tanks (titan/leviathan especially) into obstacles/other
    /// tanks it believed were clear, wedging them until an external impulse
    /// (a shell hit, or the player ramming through) dislodged them.
    pub fn avoidance_radius(&self) -> f32 {
        let (hx, hy) = self.hull_half_extents(self.facing_along_x());
        (hx * hx + hy * hy).sqrt()
    }

    /// A tank's mass, for both collision knockback and (via `Game::drive_tank`,
    /// which divides its accel/decel/turn-grip forces by this) how sluggish it
    /// is to speed up and how much it drifts through a turn. Proportional to
    /// `scale` squared - a genuine area normalization rather than an
    /// arbitrary number - scaled further by this tank's chassis class (see
    /// TANK_CHASSIS_MASS_FACTOR_BY_ROW, indexed by `row`): a `std`-class tank
    /// (assault/warden) has exactly the old flat mass every tank used to
    /// share, `narrow`/`compact` chassis are lighter (quicker to accelerate,
    /// less drift, shoved further in a ram), `long`/`wide` and especially the
    /// two `super_*` chassis are heavier (sluggish, more drift, shove lighter
    /// tanks further than they get shoved back).
    pub fn mass(&self) -> f32 {
        self.scale * self.scale * TANK_CHASSIS_MASS_FACTOR_BY_ROW[self.row as usize]
    }

    /// Recharge ammo over time toward MAX_SHELLS, one shell per interval.
    pub fn tick_recharge(&mut self, dt: f32) {
        if self.shells_ammo >= MAX_SHELLS {
            self.recharge_timer = 0.0;
            return;
        }
        self.recharge_timer += dt;
        while self.recharge_timer >= crate::SHELL_RECHARGE_SECONDS && self.shells_ammo < MAX_SHELLS
        {
            self.recharge_timer -= crate::SHELL_RECHARGE_SECONDS;
            self.shells_ammo += 1;
        }
    }

    /// Age a wreck so its fire burns for WRECK_BURN_SECONDS before going out. The
    /// timer only runs once the tank is a wreck; a live tank keeps it at zero.
    pub fn tick_wreck(&mut self, dt: f32) {
        if self.is_wreck() {
            // Cap the timer so it doesn't grow unbounded once the fire is out.
            self.wreck_timer = (self.wreck_timer + dt).min(WRECK_BURN_SECONDS);
        }
    }

    /// Reset `hit_flash_timer` to full - call whenever this tank takes
    /// damage (shell, ram, or explosion splash) so its overhead health bar
    /// shows/refreshes for another HEALTH_BAR_OVERHEAD_SECONDS.
    pub fn mark_hit(&mut self) {
        self.hit_flash_timer = crate::HEALTH_BAR_OVERHEAD_SECONDS;
    }

    /// Decide this tank's rotation and commanded velocity for one frame.
    /// `move_dir` faces the hull that way and sets `velocity` to its
    /// damage-scaled speed along that axis (classic 4-direction, no momentum;
    /// see `effective_speed`). `face` turns the hull in place without moving
    /// (used when an AI stops to aim). `move_dir` takes precedence. Shared by
    /// the player and the AI so both move identically. Does not touch
    /// `position` - that's the physics body's job once `velocity` is handed
    /// to it; see `Game::drive_tank`.
    pub fn control(&mut self, move_dir: Option<Dir>, face: Option<Dir>) {
        if let Some(dir) = move_dir {
            self.rotation = dir.rotation();
            let step = dir.vec();
            let speed = self.effective_speed();
            self.velocity = Vector2::new(step.x * speed, step.y * speed);
        } else {
            self.velocity = Vector2::new(0.0, 0.0);
            if let Some(dir) = face {
                self.rotation = dir.rotation();
            }
        }
    }

    /// Chase `visual_rotation` toward `rotation` at TANK_VISUAL_TURN_SPEED_DEG
    /// degrees/second, the short way round, so the sprite visibly swings into
    /// a turn instead of popping to the new facing the instant `rotation`
    /// snaps (see the fields' own doc comments). Called once per frame for
    /// every tank from `Game::drive_tank`, right after `control` above sets
    /// this frame's `rotation`.
    pub fn ease_visual_rotation(&mut self, dt: f32) {
        let mut diff = (self.rotation - self.visual_rotation) % 360.0;
        if diff > 180.0 {
            diff -= 360.0;
        } else if diff < -180.0 {
            diff += 360.0;
        }
        let max_step = TANK_VISUAL_TURN_SPEED_DEG * dt;
        self.visual_rotation = (self.visual_rotation + diff.clamp(-max_step, max_step)) % 360.0;
    }

    /// Chase `turret_visual_rotation` toward `rotation` at
    /// TANK_TURRET_VISUAL_TURN_SPEED_DEG degrees/second, the short way round -
    /// same mechanism as `ease_visual_rotation`, just a separate angle and a
    /// faster rate, so the turret visibly gets to the new heading before the
    /// hull does. Called once per frame for every tank, alongside
    /// `ease_visual_rotation`.
    pub fn ease_turret_visual_rotation(&mut self, dt: f32) {
        let mut diff = (self.rotation - self.turret_visual_rotation) % 360.0;
        if diff > 180.0 {
            diff -= 360.0;
        } else if diff < -180.0 {
            diff += 360.0;
        }
        let max_step = TANK_TURRET_VISUAL_TURN_SPEED_DEG * dt;
        self.turret_visual_rotation =
            (self.turret_visual_rotation + diff.clamp(-max_step, max_step)) % 360.0;
    }
}

/// Rotation pivot for a tank sprite of the given on-screen `size`: not the
/// sprite's exact geometric center, but shifted TANK_PIVOT_REAR_FRACTION of
/// its width back toward the rear of the hull (the sprite's "down" edge in
/// its unrotated, facing-up orientation). `draw_texture_pro` rotates around
/// whichever point of the sprite this names while keeping that point pinned
/// to `tank.position`, so the visible hull ends up drawn shifted forward of
/// `position` by the same amount, at every facing - purely a draw-time
/// choice; nothing gameplay-relevant reads this.
fn draw_pivot(size: f32) -> Vector2 {
    Vector2::new(size / 2.0, size / 2.0 + size * TANK_PIVOT_REAR_FRACTION)
}

/// Source rectangle for a fixed representative tank sprite - the Scout
/// chassis's idle hull frame (row 0, col 0) - used by the map editor's
/// start-point palette icon and placed-cell marker, which need one fixed
/// tank sprite rather than any particular round's rolled chassis.
pub fn icon_source_rec() -> Rectangle {
    source_rec(0, 0)
}

/// Source rectangle for the tank at (row, col) inside the atlas.
fn source_rec(row: i32, col: i32) -> Rectangle {
    Rectangle::new(
        col as f32 * TANK_TEXTURE_SIZE,
        row as f32 * TANK_TEXTURE_SIZE,
        TANK_TEXTURE_SIZE,
        TANK_TEXTURE_SIZE,
    )
}

/// Draw a single tank sprite from the atlas at its center position, scaled
/// and rotated. Hull and turret are two separate layers in the atlas (see
/// `hull_col`/`turret_col` for which column each picks, depending on
/// animation/damage state) drawn hull-first-then-turret at the same
/// dest/origin but each at its own eased angle (`visual_rotation` for the
/// hull, `turret_visual_rotation` for the turret) - the turret still just
/// chases the tank's commanded `rotation`, not an independent aim target, but
/// it does so faster than the hull so it visibly leads a turn.
pub fn draw_tank(d: &mut impl RaylibDraw, texture: &Texture2D, tank: &Tank) {
    let hull_src = source_rec(tank.row, tank.hull_col());
    let turret_src = source_rec(tank.row, tank.turret_col());
    let size = tank.size();

    // dest is placed at the tank's position; origin is the rear-shifted
    // pivot (see `draw_pivot`), not the sprite's exact middle.
    let dest = Rectangle::new(tank.position.x, tank.position.y, size, size);
    let origin = draw_pivot(size);

    d.draw_texture_pro(texture, hull_src, dest, origin, tank.visual_rotation, Color::WHITE);
    d.draw_texture_pro(
        texture,
        turret_src,
        dest,
        origin,
        tank.turret_visual_rotation,
        Color::WHITE,
    );
}

/// Draw this tank's drop shadow: the same two layers (each at its own eased
/// angle, matching `draw_tank`), offset toward a fixed screen-space direction
/// and tinted flat black - see docs/sprite-shadows-design.md. Must be called
/// *before* `draw_tank` so the real sprite draws on top of its own shadow. No
/// wreck/dead special-casing needed - a burnt-out hulk is still a solid
/// object sitting on the ground.
pub fn draw_tank_shadow(d: &mut impl RaylibDraw, texture: &Texture2D, tank: &Tank) {
    let hull_src = source_rec(tank.row, tank.hull_col());
    let turret_src = source_rec(tank.row, tank.turret_col());
    let size = tank.size();

    let dest = Rectangle::new(
        tank.position.x + SHADOW_DIR_X * TANK_SHADOW_OFFSET,
        tank.position.y + SHADOW_DIR_Y * TANK_SHADOW_OFFSET,
        size,
        size,
    );
    let origin = draw_pivot(size);
    let shadow = Color::new(0, 0, 0, (255.0 * TANK_SHADOW_OPACITY) as u8);

    d.draw_texture_pro(texture, hull_src, dest, origin, tank.visual_rotation, shadow);
    d.draw_texture_pro(
        texture,
        turret_src,
        dest,
        origin,
        tank.turret_visual_rotation,
        shadow,
    );
}

/// Draw the minigun barrel-cluster overlay on top of this tank's turret, if
/// it currently has the weapon (`minigun_ammo > 0`) and isn't a wreck - the
/// existing broken-turret art (`turret_col`) already communicates
/// "destroyed" on its own, so this simply stops drawing once `is_wreck()`
/// rather than authoring a separate broken-minigun asset. Visible whenever
/// the tank *possesses* the weapon, regardless of whether laser currently
/// outranks it in `active_weapon()` - the mount is a physical object on the
/// turret, not a firing-mode indicator. Independent of hull damage tier
/// (`hull_col`'s light/disabled/wreck ladder): hull and turret art are
/// already fully decoupled layers, and this overlay only checks
/// turret-adjacent state (`is_wreck`), so a hull that's gone
/// light/disabled never hides it.
///
/// Drawn at the exact same `dest`/`origin` as `draw_tank`'s hull/turret
/// layers (same shared pivot, see `draw_pivot`) - the overlay's own art
/// (`tools/spritegen/gen_minigun_mount.py`) is authored around that
/// identical cell-center-ish pivot, with its barrels extending forward from
/// it by a fixed on-canvas distance exactly like `gen_tanks.py` already
/// draws every turret's own barrel rects from that same pivot outward - so
/// it drops into place with no offset math, exactly like the turret layer
/// itself. Positioning it instead via the muzzle-offset formula
/// (`TANK_MUZZLE_FORWARD_OFFSET_BY_ROW`, which uses the tank's instant,
/// snapped `rotation`) would visibly detach the cluster from the turret
/// mid-turn, since this overlay rotates at the eased
/// `turret_visual_rotation` instead - that formula stays reserved for what
/// it's proven for: positioning where `Bullet`s actually spawn.
///
/// Rotated by `turret_visual_rotation` ONLY (to track the turret's own
/// eased aim) - unlike the turret itself, this does NOT add any further
/// spin: `minigun_mount.png` is a 3-column sheet, one "hot barrel" per
/// column (see `minigun_cycle_frame`/`minigun_cycle_timer`), cycled instead
/// of rotated. A top-down camera looks edge-on at a barrel cluster's real
/// rotation axis (the barrels point along the ground plane, toward the
/// target), so spinning this sprite in the screen plane would read as a
/// helicopter rotor seen from above - the wrong axis entirely for this
/// camera angle. See `tools/spritegen/gen_minigun_mount.py`'s module doc
/// comment for the full reasoning. Scaled by the flat `MINIGUN_MOUNT_SCALE`
/// (not indexed by chassis row) layered on the tank's own `scale` - the
/// mount is deliberately the same size on every chassis, a fixed piece of
/// hardware rather than something that scales with the tank it's bolted to.
pub fn draw_minigun_mount(d: &mut impl RaylibDraw, texture: &Texture2D, tank: &Tank) {
    if tank.minigun_ammo <= 0 || tank.is_wreck() {
        return;
    }
    let src = Rectangle::new(
        tank.minigun_cycle_frame() as f32 * MINIGUN_MOUNT_TEXTURE_SIZE,
        0.0,
        MINIGUN_MOUNT_TEXTURE_SIZE,
        MINIGUN_MOUNT_TEXTURE_SIZE,
    );
    let size = MINIGUN_MOUNT_TEXTURE_SIZE * tank.scale * MINIGUN_MOUNT_SCALE;
    let dest = Rectangle::new(tank.position.x, tank.position.y, size, size);
    let origin = draw_pivot(size);
    d.draw_texture_pro(texture, src, dest, origin, tank.turret_visual_rotation, Color::WHITE);
}

/// Shadow pass for `draw_minigun_mount` - same tint/offset convention as
/// `draw_tank_shadow` (`TANK_SHADOW_OFFSET`/`TANK_SHADOW_OPACITY`, not a
/// separate constant): it's rigidly bolted to the turret, so it should read
/// at the exact same height/offset as the turret's own shadow. Call before
/// `draw_minigun_mount` (and after `draw_tank_shadow`), same ordering rule
/// as every other shadow pass.
pub fn draw_minigun_mount_shadow(d: &mut impl RaylibDraw, texture: &Texture2D, tank: &Tank) {
    if tank.minigun_ammo <= 0 || tank.is_wreck() {
        return;
    }
    let src = Rectangle::new(
        tank.minigun_cycle_frame() as f32 * MINIGUN_MOUNT_TEXTURE_SIZE,
        0.0,
        MINIGUN_MOUNT_TEXTURE_SIZE,
        MINIGUN_MOUNT_TEXTURE_SIZE,
    );
    let size = MINIGUN_MOUNT_TEXTURE_SIZE * tank.scale * MINIGUN_MOUNT_SCALE;
    let dest = Rectangle::new(
        tank.position.x + SHADOW_DIR_X * TANK_SHADOW_OFFSET,
        tank.position.y + SHADOW_DIR_Y * TANK_SHADOW_OFFSET,
        size,
        size,
    );
    let origin = draw_pivot(size);
    let shadow = Color::new(0, 0, 0, (255.0 * TANK_SHADOW_OPACITY) as u8);
    d.draw_texture_pro(texture, src, dest, origin, tank.turret_visual_rotation, shadow);
}
