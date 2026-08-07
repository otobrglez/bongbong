use rapier2d::prelude::{ColliderHandle, RigidBodyHandle};
use sola_raylib::prelude::*;

use crate::{
    DAMAGE_SPEED_CURVE, DAMAGE_SPEED_FLOOR, MAX_DAMAGE, MAX_SHELLS, Position,
    SHADOW_DIR_X, SHADOW_DIR_Y, TANK_BROKEN_TURRET_COL, TANK_CHASSIS_MASS_FACTOR_BY_ROW,
    TANK_HULL_BBOX_BY_ROW, TANK_HULL_DISABLED_COL, TANK_HULL_DISABLED_DAMAGE, TANK_HULL_FRACTION,
    TANK_HULL_LIGHT_COL,
    TANK_HULL_LIGHT_DAMAGE, TANK_HULL_TRACK_COLS, TANK_PIVOT_REAR_FRACTION, TANK_SHADOW_OFFSET,
    TANK_SHADOW_OPACITY,
    TANK_SPEED, TANK_TEXTURE_SIZE, TANK_TURRET_COL, TANK_TURRET_VISUAL_TURN_SPEED_DEG,
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

pub struct Tank {
    /// Which of the 12 tank archetypes in scifi_tanks_sheet.png this tank
    /// draws (see TANK_VARIANTS/TANK_SPRITE_ORDER in simulation.rs). The hull
    /// and turret layers' columns within that row depend on this tank's
    /// current animation/damage state - see `hull_col`/`turret_col`.
    pub row: i32,
    /// Row in shells.png this tank's shells are drawn from (0..SHELL_VARIANTS)
    /// - matched to this tank's chassis (barrel count, size class, accent
    /// colour) via TANK_SHELL_VARIANT_BY_ROW, set at spawn (see Game::init)
    /// and refreshed on every shot this tank fires (see `alternate_shot`,
    /// simulation.rs's two fire call sites) rather than fixed for its whole
    /// life.
    pub shell_variant: i32,
    /// Toggled every time this tank fires (simulation.rs), so a twin-barrel
    /// chassis alternates between the simultaneous-twin and staggered-twin
    /// shell art (TANK_SHELL_VARIANT_BY_ROW vs
    /// TANK_SHELL_VARIANT_STAGGERED_BY_ROW) shot to shot instead of always
    /// firing the same way. A no-op for single-barrel chassis, whose
    /// staggered-table entry equals its base entry.
    pub alternate_shot: bool,
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
    /// Seconds accumulated toward recharging the next shell.
    pub recharge_timer: f32,
    /// Seconds remaining before this tank can take ramming damage again.
    pub ram_cooldown: f32,
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
    /// This tank's shell-hit sensor collider - a second, larger collider on
    /// the same `body` (see `physics::Physics::add_hit_sensor`), sized to
    /// the tank's full sprite rather than its solid hull, used only to
    /// detect when a shell hits it.
    pub hit_sensor: Option<ColliderHandle>,
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
            alternate_shot: false,
            damage_variant: 0,
            position: Position::default(),
            rotation: 0.0,
            visual_rotation: 0.0,
            turret_visual_rotation: 0.0,
            hull_frame: 0,
            hull_anim_accum: 0.0,
            wreck_col: None,
            scale: 2.0, // 3.0,
            speed: TANK_SPEED,
            damage: 0.0,
            shells_ammo: MAX_SHELLS,
            recharge_timer: 0.0,
            ram_cooldown: 0.0,
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
    /// (`TANK_ACCEL_FORCE`/`TANK_DECEL_FORCE` in `Game::drive_tank`), so a
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
    /// (a shell hit, or the player ramming through) dislodged them. Exact
    /// equality against `Dir::Right`/`Dir::Left` is safe here (unlike an
    /// epsilon check) because `rotation` only ever holds one of the four
    /// exact `Dir::rotation()` constants - see `Tank::control`.
    pub fn avoidance_radius(&self) -> f32 {
        let along_x = self.rotation == Dir::Right.rotation() || self.rotation == Dir::Left.rotation();
        let (hx, hy) = self.hull_half_extents(along_x);
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
