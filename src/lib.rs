use sola_raylib::prelude::*;

/// A 2D screen position in pixels.
pub type Position = Vector2;

// tanks.png is a 256x256 atlas laid out as an 8x8 grid, so each tank is 32x32.
pub const TANK_TEXTURE_SIZE: f32 = 32.0;
// The tank hull only fills part of its 32x32 tile (the rest is transparent
// padding). Collisions use this fraction of the sprite so tanks can nudge close
// together instead of stopping a padding-width apart.
pub const TANK_HULL_FRACTION: f32 = 0.7;

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

// When the round ends (player destroyed, or all enemies destroyed) the result
// is shown for this long, then the game restarts.
pub const RESTART_DELAY: f32 = 5.0;

// Tank driving: classic 4-direction, constant-speed movement. Pressing a
// direction snaps the hull to face it and moves at a fixed speed; releasing
// stops instantly. No momentum.
pub const TANK_SPEED: f32 = 220.0; // player movement speed (px/s)
pub const ENEMY_SPEED: f32 = 150.0; // baseline enemy speed (px/s), slower than the player
// Each enemy's speed is randomized within +/- this fraction of ENEMY_SPEED at
// spawn, so some drive faster and some slower instead of all moving in lockstep.
pub const ENEMY_SPEED_VARIANCE: f32 = 0.25;

// Shell ammo: the player holds up to MAX_SHELLS and recharges one shell every
// SHELL_RECHARGE_SECONDS while below the cap.
pub const MAX_SHELLS: i32 = 7;
pub const SHELL_RECHARGE_SECONDS: f32 = 2.0;

// Ramming: after taking collision damage a tank is immune for this long, so
// continuous touching doesn't drain damage every frame.
pub const RAM_DAMAGE_COOLDOWN: f32 = 0.5;

// Enemy AI tuning. Distances are in pixels, times in seconds.
pub const ENEMY_VIEW_RANGE: f32 = 520.0; // start chasing the player within this
pub const ENEMY_ATTACK_RANGE: f32 = 340.0; // stop and fight within this
pub const ENEMY_FIRE_ALIGN_PX: f32 = 24.0; // fire when player is within this of the axis
pub const ENEMY_FIRE_INTERVAL: f32 = 2.4; // min seconds between AI shots (toned down)
pub const ENEMY_AIM_SETTLE: f32 = 0.45; // must be lined up this long before firing
pub const ENEMY_DAMAGE_MIN: f32 = 5.0; // enemy shell damage lower bound (weaker)
pub const ENEMY_DAMAGE_MAX: f32 = 15.0; // enemy shell damage upper bound
pub const ENEMY_FLEE_DAMAGE: f32 = 70.0; // retreat once this hurt
pub const ENEMY_RETARGET_SECONDS: f32 = 3.0; // how often patrol picks a new point

// Point-blank misfires: when an enemy shoots while very close to the player it may
// "misfire", throwing the shot off its aim so it sails wide instead of landing a
// point-blank hit. The chance scales up the closer it is (zero beyond
// ENEMY_MISFIRE_RANGE, up to _CHANCE_MAX right on top of the player), and a
// misfire deflects the shell by a random angle in the given degree range.
pub const ENEMY_MISFIRE_RANGE: f32 = 180.0; // only misfire within this of the player
pub const ENEMY_MISFIRE_CHANCE_MAX: f32 = 0.6; // misfire odds at point-blank range
pub const ENEMY_MISFIRE_ANGLE_MIN: f32 = 12.0; // smallest off-aim deflection (deg)
pub const ENEMY_MISFIRE_ANGLE_MAX: f32 = 35.0; // largest off-aim deflection (deg)

// Predictive collision avoidance: a moving enemy looks ahead along its heading and
// estimates the closest approach to every other tank (other enemies AND the
// player). If a hit looks likely soon, it sidesteps perpendicular for a short
// window, then resumes steering (which pulls it back on course). See
// Ai::avoid_collisions.
pub const AVOID_LOOKAHEAD: f32 = 0.8; // seconds ahead to predict closest approach
pub const AVOID_MARGIN: f32 = 12.0; // extra clearance beyond the two hull radii (px)
pub const AVOID_DODGE_SECONDS: f32 = 0.4; // how long a sidestep is held once triggered
pub const AVOID_MIN_SPEED: f32 = 10.0; // skip prediction when moving slower than this

// Direction commitment: once an AI picks a cardinal heading it holds it for at
// least this long, and only switches to a new heading that beats the current one
// by this margin. Together these stop the frame-to-frame Left/Right/Up/Down
// jitter that occurs near 45-degree diagonals.
pub const AI_DIR_HOLD_SECONDS: f32 = 0.35;
pub const AI_DIR_SWITCH_MARGIN_PX: f32 = 20.0;

// Player shell damage bounds (unchanged behaviour, now named for symmetry).
pub const PLAYER_DAMAGE_MIN: f32 = 10.0;
pub const PLAYER_DAMAGE_MAX: f32 = 30.0;

// A wreck burns for this long, then the fire/smoke dies out and it settles into
// a static charred "dead" hulk.
pub const WRECK_BURN_SECONDS: f32 = 4.0;

// Track marks: tracks.png is a single 32x32 tile of two tread ladders (matching
// the tank sprite orientation). A tank drops a mark every TRACK_SPACING pixels it
// travels, and each mark fades out over TRACK_LIFETIME seconds.
pub const TRACK_TEXTURE_SIZE: f32 = 32.0;
pub const TRACK_SPACING: f32 = 16.0; // distance travelled between dropped marks
pub const TRACK_LIFETIME: f32 = 1.0; // seconds for a mark to fully fade away
// Marks are drawn smaller than the tank and faint, so the trail reads as a subtle
// impression in the ground rather than a bold sprite.
pub const TRACK_SCALE_FRACTION: f32 = 0.55; // mark size relative to the tank sprite
pub const TRACK_MAX_OPACITY: f32 = 0.3; // opacity of a fresh mark, before fading

pub mod ai;
pub mod bt;
pub mod damage_stage;
pub mod game;
pub mod shell;
pub mod tank;
pub mod track;
