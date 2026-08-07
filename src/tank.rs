use rapier2d::prelude::{ColliderHandle, RigidBodyHandle};
use sola_raylib::prelude::*;

use crate::{
    DAMAGE_SPEED_CURVE, DAMAGE_SPEED_FLOOR, DEAD_TINT_FACTOR, MAX_DAMAGE, MAX_SHELLS, Position,
    SHADOW_DIR_X, SHADOW_DIR_Y, TANK_HULL_FRACTION, TANK_SHADOW_OFFSET, TANK_SHADOW_OPACITY,
    TANK_SPEED, TANK_TEXTURE_SIZE, WRECK_BURN_SECONDS,
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
    /// Which sprite in the atlas to draw.
    pub row: i32,
    pub col: i32,
    /// Row in shells.png this tank's shells are drawn from (0..SHELL_VARIANTS).
    /// Rolled once at spawn (see Game::init) and fixed for the tank's whole
    /// life, so all of its shots read as visually consistent.
    pub shell_variant: i32,
    /// Row in damage.png this tank's damage overlay is drawn from
    /// (0..DAMAGE_VARIANTS). Rolled once at spawn (see Game::init) and fixed
    /// for the tank's whole life, so its damage sequence reads as one
    /// consistent flavour rather than switching palettes between stages.
    pub damage_variant: i32,
    /// Center position on screen (pixels). A read-back mirror of `body`'s
    /// physics transform, synced once per frame after the physics world
    /// steps (see `Game::update`) - nothing else should write this by hand.
    pub position: Position,
    /// Facing angle in degrees.
    pub rotation: f32,
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
}

impl Default for Tank {
    fn default() -> Self {
        Self {
            row: 0,
            col: 0,
            shell_variant: 0,
            damage_variant: 0,
            position: Position::default(),
            rotation: 0.0,
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

    /// Small phase offset (seconds) derived from screen position so that several
    /// burning tanks don't animate their smoke/fire in perfect lockstep.
    pub fn anim_phase(&self) -> f32 {
        (self.position.x + self.position.y) * 0.01
    }

    /// Collision footprint side length: the visible hull, not the full sprite
    /// tile, so tanks can close the gap left by the sprite's transparent padding.
    pub fn hull_size(&self) -> f32 {
        self.size() * TANK_HULL_FRACTION
    }

    /// A tank's mass for collision knockback: proportional to hull area
    /// (scale squared), so it's a genuine normalization rather than an
    /// arbitrary number - two tanks of equal scale split an impact evenly,
    /// and a bigger one (if scale ever varies) resists more and shoves harder.
    pub fn mass(&self) -> f32 {
        self.scale * self.scale
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

/// Draw a single tank sprite from the atlas at its center position, scaled and rotated.
pub fn draw_tank(d: &mut impl RaylibDraw, texture: &Texture2D, tank: &Tank) {
    let src = source_rec(tank.row, tank.col);
    let size = tank.size();

    // dest is placed at the tank's position; origin is half the size so the
    // sprite is centered on `position` and rotates around its own middle.
    let dest = Rectangle::new(tank.position.x, tank.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);

    d.draw_texture_pro(texture, src, dest, origin, tank.rotation, Color::WHITE);

    // A dead (burnt-out) tank is washed toward gray instead of showing a
    // separate dead overlay sprite. A plain multiply tint only dims
    // brightness (hue/saturation survive, so it barely reads as "dead" at
    // pixel-art scale) - drawing the same sprite again with a translucent
    // flat-gray tint blends it toward true gray instead, and reusing the
    // same texture/src means the wash is automatically masked to the tank's
    // own silhouette (transparent padding stays transparent).
    if tank.is_dead() {
        let alpha = (255.0 * DEAD_TINT_FACTOR) as u8;
        let wash = Color::new(120, 120, 120, alpha);
        d.draw_texture_pro(texture, src, dest, origin, tank.rotation, wash);
    }
}

/// Draw this tank's drop shadow: the same sprite, same rotation, offset
/// toward a fixed screen-space direction and tinted flat black - see
/// docs/sprite-shadows-design.md. Must be called *before* `draw_tank` so the
/// real sprite draws on top of its own shadow. No wreck/dead special-casing
/// needed - a burnt-out hulk is still a solid object sitting on the ground.
pub fn draw_tank_shadow(d: &mut impl RaylibDraw, texture: &Texture2D, tank: &Tank) {
    let src = source_rec(tank.row, tank.col);
    let size = tank.size();

    let dest = Rectangle::new(
        tank.position.x + SHADOW_DIR_X * TANK_SHADOW_OFFSET,
        tank.position.y + SHADOW_DIR_Y * TANK_SHADOW_OFFSET,
        size,
        size,
    );
    let origin = Vector2::new(size / 2.0, size / 2.0);
    let shadow = Color::new(0, 0, 0, (255.0 * TANK_SHADOW_OPACITY) as u8);

    d.draw_texture_pro(texture, src, dest, origin, tank.rotation, shadow);
}
