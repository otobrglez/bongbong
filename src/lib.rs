use sola_raylib::prelude::*;

/// A 2D screen position in pixels.
pub type Position = Vector2;

// tanks.png is a 256x256 atlas laid out as an 8x8 grid, so each tank is 32x32.
pub const TANK_TEXTURE_SIZE: f32 = 32.0;

// shells.png is a 160x32 sheet: five 32x32 frames indexed by column.
pub const SHELL_TEXTURE_SIZE: f32 = 32.0;
pub const SHELL_SPEED: f32 = 500.0;
pub const SHELL_SCALE: f32 = 2.0;

// damage.png is a 448x32 overlay sheet: a single row of fourteen 32x32 frames
// indexed by column, drawn on top of a tank to show escalating damage. The
// frames are grouped into animated stages (see DamageStage):
//   0 dusty | 1 gray | 2-3 small-smoke | 4-5 more-smoke | 6-7 small-fire
//   8-9 large-fire | 10-12 wrecked (burning) | 13 dead (burnt-out hulk)
pub const DAMAGE_TEXTURE_SIZE: f32 = 32.0;
// Damage overlay column for the burnt-out dead hulk (no fire/smoke).
pub const DEAD_FRAME: i32 = 13;

pub const ENEMY_COUNT: usize = 3;
pub const MAX_DAMAGE: f32 = 100.0;

// Tank driving dynamics (mimics a real tank: throttle + hull rotation with
// momentum, rather than instant 4-way strafing).
pub const TANK_ACCEL: f32 = 420.0; // forward throttle acceleration (px/s^2)
pub const TANK_REVERSE_ACCEL: f32 = 300.0; // reverse throttle acceleration (px/s^2)
pub const TANK_MAX_SPEED: f32 = 260.0; // top forward speed (px/s)
pub const TANK_MAX_REVERSE: f32 = 120.0; // top reverse speed (px/s)
pub const TANK_FRICTION: f32 = 380.0; // deceleration while coasting (px/s^2)
pub const TANK_TURN_SPEED: f32 = 150.0; // hull rotation rate (deg/s)

// Shell ammo: the player holds up to MAX_SHELLS and recharges one shell every
// SHELL_RECHARGE_SECONDS while below the cap.
pub const MAX_SHELLS: i32 = 7;
pub const SHELL_RECHARGE_SECONDS: f32 = 2.0;

// Ramming: after taking collision damage a tank is immune for this long, so
// continuous touching doesn't drain damage every frame.
pub const RAM_DAMAGE_COOLDOWN: f32 = 0.5;

// A wreck burns for this long, then the fire/smoke dies out and it settles into
// a static charred "dead" hulk.
pub const WRECK_BURN_SECONDS: f32 = 4.0;

pub mod damage_stage;
pub mod game;
pub mod shell;
pub mod tank;
