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

// Number of enemy tanks is randomized within this range each round.
pub const ENEMY_COUNT_MIN: usize = 3;
pub const ENEMY_COUNT_MAX: usize = 10;
pub const MAX_DAMAGE: f32 = 100.0;

// Enemies spawn in a band that's between 20% and 40% of the shorter screen
// dimension away from the nearest edge of the battlefield - close enough to
// feel like they're closing in from the sides, but never right on the edge
// or dropped in the middle.
pub const ENEMY_SPAWN_MARGIN_MIN: f32 = 0.2;
pub const ENEMY_SPAWN_MARGIN_MAX: f32 = 0.4;

// When the round ends (player destroyed, or all enemies destroyed) the result
// is shown for this long, then the game restarts.
pub const RESTART_DELAY: f32 = 3.0;

// Tank driving: classic 4-direction, constant-speed movement. Pressing a
// direction snaps the hull to face it and moves at a fixed speed; releasing
// stops instantly. No momentum.
pub const TANK_SPEED: f32 = 220.0; // player movement speed (px/s)
pub const ENEMY_SPEED: f32 = 150.0; // baseline enemy speed (px/s), slower than the player
// Each enemy's speed is randomized within +/- this fraction of ENEMY_SPEED at
// spawn, so some drive faster and some slower instead of all moving in lockstep.
pub const ENEMY_SPEED_VARIANCE: f32 = 0.25;

// A damaged tank slows down: its speed is scaled by a curve of how hurt it is
// (0 = pristine, 1 = about to wreck). The curve stays close to full speed
// through light and moderate damage, then falls off harder as damage climbs
// toward the max - a limp rather than a straight-line taper - bottoming out at
// DAMAGE_SPEED_FLOOR instead of zero (a tank stops moving separately, once
// it's a wreck).
pub const DAMAGE_SPEED_FLOOR: f32 = 0.35;
pub const DAMAGE_SPEED_CURVE: f32 = 2.2;

// Shell ammo: the player holds up to MAX_SHELLS and recharges one shell every
// SHELL_RECHARGE_SECONDS while below the cap.
pub const MAX_SHELLS: i32 = 7;
pub const SHELL_RECHARGE_SECONDS: f32 = 2.0;

// Ramming: after taking collision damage a tank is immune for this long, so
// continuous touching doesn't drain damage every frame.
pub const RAM_DAMAGE_COOLDOWN: f32 = 0.5;

// A ram also gives both tanks a brief, small knockback shove apart (see
// Tank::apply_knockback), scaled by their closing speed and normalized mass
// (hull area, i.e. scale squared - see Tank::mass) so a lighter tank gets
// shoved further than a heavier one. Wrecks are treated as infinite mass in
// both this and the explosion knockback below - already-dead hulks stay put
// when hit.
pub const KNOCKBACK_STRENGTH: f32 = 0.2; // fraction of ram closing speed converted to push speed
pub const KNOCKBACK_MAX_SPEED: f32 = 60.0; // px/s cap on any one push - keeps it small
pub const KNOCKBACK_DAMPING: f32 = 8.0; // 1/s decay rate; higher = the push dies out faster

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

// Shockwave post-processing effect triggered when a tank is destroyed: a ring
// of radial distortion expands from the hit point over the whole screen and
// fades out. See `shockwave.rs` and `Game::render`.
pub const SHOCKWAVE_DURATION: f32 = 1.0; // seconds the effect plays before clearing
pub const SHOCKWAVE_SPEED: f32 = 0.8; // ring growth speed, UV units/sec
pub const SHOCKWAVE_WIDTH: f32 = 0.08; // thickness of the distorted band, UV units
pub const SHOCKWAVE_STRENGTH: f32 = 0.03; // how hard the ring bends the image, UV units

// A tank's death also deals a small splash of damage to any other tank caught
// nearby, on top of the shockwave visual. Deliberately weak - a chip of
// damage, not a second kill shot.
pub const EXPLOSION_RADIUS: f32 = 110.0; // px a wrecked tank's blast reaches
pub const EXPLOSION_DAMAGE_MIN: f32 = 3.0;
pub const EXPLOSION_DAMAGE_MAX: f32 = 8.0;
// It also gives every live tank it reaches a small outward shove, same
// knockback mechanism as a ram (Tank::apply_knockback / Tank::mass), but
// driven by distance from the blast instead of closing speed: full push at
// the center, tapering linearly to nothing at EXPLOSION_RADIUS.
pub const EXPLOSION_KNOCKBACK_SPEED: f32 = 90.0; // px/s push at ground zero

// Muzzle-flash heat haze: a tiny, split-second effect localized to a small
// patch of screen at the barrel when a tank fires. Unlike the kill shockwave
// (a rolling, symmetric ring - shockwave.fs), this uses its own shader
// (muzzle_flash.fs) that's a single one-sided outward puff with no wobble,
// so it reads as a hot flash rather than a shrunk-down shockwave. It also
// hits full `strength` right at the leading edge (no sine attenuation), so
// this is tuned lower than the old shared-shader value to land at a similar
// visual intensity.
pub const MUZZLE_FLASH_DURATION: f32 = 0.12; // seconds the effect plays before clearing
pub const MUZZLE_FLASH_SPEED: f32 = 0.9; // front growth speed, UV units/sec
pub const MUZZLE_FLASH_WIDTH: f32 = 0.015; // thickness of the pushed band, UV units
pub const MUZZLE_FLASH_STRENGTH: f32 = 0.015; // how hard the puff shoves the image, UV units
// Half-extent (px) of the quad the effect is drawn into. Needs to comfortably
// contain the ring's full reach (MUZZLE_FLASH_SPEED * _DURATION, converted to
// screen pixels) plus its band width, or the ring visibly clips at the edge.
pub const MUZZLE_FLASH_QUAD_RADIUS: f32 = 90.0;

// Shell-impact flash: a tiny, split-second effect at the point a shell lands
// on a tank (hit or kill), localized to a small patch of screen like the
// muzzle flash. Uses its own shader (impact.fs): a one-sided outward punch
// plus a fast-decaying warm spark flash at the center, so a hit reads as a
// sharp "thwack" distinct from both the muzzle's heat-shimmer puff and the
// big rolling kill-shockwave ring.
pub const IMPACT_FLASH_DURATION: f32 = 0.14; // seconds the effect plays before clearing
pub const IMPACT_FLASH_SPEED: f32 = 1.1; // punch growth speed, UV units/sec
pub const IMPACT_FLASH_WIDTH: f32 = 0.02; // thickness of the punched band, UV units
pub const IMPACT_FLASH_STRENGTH: f32 = 0.025; // how hard the punch shoves the image, UV units
// Half-extent (px) of the quad the effect is drawn into; see
// MUZZLE_FLASH_QUAD_RADIUS for why this needs to comfortably contain the
// punch's full reach plus its band width.
pub const IMPACT_FLASH_QUAD_RADIUS: f32 = 70.0;

pub mod ai;
pub mod bt;
pub mod damage_stage;
pub mod game;
pub mod physics;
pub mod shell;
pub mod shockwave;
pub mod tank;
pub mod track;
