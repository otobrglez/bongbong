use sola_raylib::prelude::*;

/// A 2D screen position in pixels.
pub type Position = Vector2;

// tanks.png is a 256x256 atlas laid out as an 8x8 grid, so each tank is 32x32.
pub const TANK_TEXTURE_SIZE: f32 = 32.0;
// The tank hull only fills part of its 32x32 tile (the rest is transparent
// padding). Collisions use this fraction of the sprite so tanks can nudge close
// together instead of stopping a padding-width apart.
pub const TANK_HULL_FRACTION: f32 = 0.7;
// How far forward (tile px, toward wherever the hull currently faces) a
// shell spawns from the tank's center - i.e. where the turret/barrel tip
// actually sits, not the tank's own center or the edge of its 32x32 tile.
// Measured from the sprite sheet itself: for each of the 8 tank hulls (row 0,
// tanks_candy.png), the topmost opaque pixel in the unrotated (facing "up")
// orientation sits at tile-row ~3-4 (one twin-spike hull tops out later, at
// row 6), all horizontally centered on the tile - averaging to row 3.6, i.e.
// 32/2 - 3.6 = 12.4px above center. See `Shell::spawn`.
pub const TANK_MUZZLE_FORWARD_OFFSET: f32 = 12.5;

// shells.png is a 224x96 sheet: seven 32x32 frames (col, see ShellState)
// indexed across three row-variants (see SHELL_VARIANTS / Tank::shell_variant)
// that reskin the fire and hit frames while keeping the flying frame (col 3)
// pixel-identical across all rows.
pub const SHELL_TEXTURE_SIZE: f32 = 32.0;
// Number of row-variants in shells.png. Each tank rolls one at spawn
// (Tank::shell_variant) and every shell it fires uses that row.
pub const SHELL_VARIANTS: i32 = 3;
pub const SHELL_SPEED: f32 = 500.0;
pub const SHELL_SCALE: f32 = 2.0;
// Half-extent (px) of a shell's own physics sensor, used only to intersect a
// tank's hit sensor (see TANK_HULL_FRACTION/Tank::size and
// physics::Physics::add_hit_sensor). Kept small and near-point-like so the
// intersection test still reads as "did the shell's exact position land
// inside the tank", matching the old point-in-box check, rather than "did
// the shell's box overlap the tank's box".
pub const SHELL_HIT_HALF_EXTENT: f32 = 3.0;

// Drop shadows: a second copy of a tank/shell sprite drawn first, tinted flat
// black (see Game::render / Tank::draw_tank_shadow / shell::draw_shell_shadow),
// offset toward a fixed screen-space direction so it reads as a rotated
// silhouette of that specific sprite sitting slightly behind/below it, not a
// generic blob. See docs/sprite-shadows-design.md. Toggled at runtime by the
// L key and at startup by `--no-shadows` (see Game::shadows_enabled).
// Shared offset direction (down-right, a common top-down-arcade convention) -
// only the *distance* differs per entity type below.
pub const SHADOW_DIR_X: f32 = 0.6;
pub const SHADOW_DIR_Y: f32 = 0.8;

pub const TANK_SHADOW_OFFSET: f32 = 3.0; // px - grounded, stays tight to the hull
pub const TANK_SHADOW_OPACITY: f32 = 0.35;

// Bigger than the tank offset on purpose: the separation between a shell and
// its own shadow is what reads as "airborne" with no real z-axis. Only drawn
// while the shell is actually ShellState::Flying (see Game::render) - the
// fire/impact frames are stationary blast sprites, not airborne objects.
// Rolled once per shell at fire time within this range (Shell::shadow_offset,
// set in Game::update right after Shell::spawn) rather than a flat distance,
// so different shells read as flying at different heights instead of every
// shot looking identical.
pub const SHELL_SHADOW_OFFSET_MIN: f32 = 9.0; // px
pub const SHELL_SHADOW_OFFSET_MAX: f32 = 20.0; // px
pub const SHELL_SHADOW_OPACITY: f32 = 0.30;

// damage.png is a 448x160 overlay sheet: fourteen 32x32 frames per row
// (indexed by column), drawn on top of a tank to show escalating damage. The
// frames are grouped into animated stages (see DamageStage):
//   0 dusty | 1 gray | 2-3 small-smoke | 4-5 more-smoke | 6-7 small-fire
//   8-9 large-fire | 10-12 wrecked (burning) | 13 dead (burnt-out hulk)
pub const DAMAGE_TEXTURE_SIZE: f32 = 32.0;
// Damage overlay column for the burnt-out dead hulk (no fire/smoke) - the
// static frame draw_damage locks onto once Tank::is_dead, layered on top of
// the sprite's own DEAD_TINT_FACTOR gray wash so a dead tank reads clearly
// even at a glance, not just "a bit darker."
pub const DEAD_FRAME: i32 = 13;
// Number of row-variants in damage.png (see SHELL_VARIANTS for the same
// pattern). Each tank rolls one at spawn (Tank::damage_variant) and keeps it
// for its whole life, so a tank's whole damage sequence reads as one
// consistent "flavour" instead of jumping between palettes frame to frame.
pub const DAMAGE_VARIANTS: i32 = 5;
// A dead (burnt-out) tank's own sprite is washed toward gray by this
// fraction (0 = untouched, 1 = flat gray) - see tank::draw_tank, which blends
// a translucent gray pass over the sprite rather than just dimming it, so it
// visually reads as "out of the fight" even before the DEAD_FRAME overlay
// (see draw_damage) is added on top.
pub const DEAD_TINT_FACTOR: f32 = 0.6;

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

// Tank driving: 4-direction movement with real inertia, modeled like a
// tracked vehicle rather than a car. Pressing a direction snaps the *hull
// facing* immediately (rotation is still instant/cosmetic) and the tank's
// velocity along that axis chases the commanded speed via a mass-aware
// acceleration impulse each frame rather than snapping to it - see
// Game::drive_tank, TANK_ACCEL_FORCE/TANK_DECEL_FORCE below. Unlike a car,
// though, a tank's tracks give it almost no lateral grip loss: any velocity
// component *perpendicular* to the commanded direction is scrubbed off hard
// by TANK_TURN_GRIP_FORCE, so turning a corner reads as the hull snapping
// onto the new axis, not sliding through a curve. Momentum only survives
// along the axis actually being driven.
pub const TANK_SPEED: f32 = 220.0; // player top speed (px/s)
pub const ENEMY_SPEED: f32 = 150.0; // baseline enemy top speed (px/s), slower than the player
// Each enemy's speed is randomized within +/- this fraction of ENEMY_SPEED at
// spawn, so some drive faster and some slower instead of all moving in lockstep.
pub const ENEMY_SPEED_VARIANCE: f32 = 0.25;

// A damaged tank slows down: both its top speed and how fast it can reach it
// are scaled by a curve of how hurt it is (0 = pristine, 1 = about to
// wreck; see Tank::speed_factor). The curve stays close to full effect
// through light and moderate damage, then falls off harder as damage climbs
// toward the max - a limp rather than a straight-line taper - bottoming out
// at DAMAGE_SPEED_FLOOR instead of zero (a tank stops moving separately,
// once it's a wreck).
pub const DAMAGE_SPEED_FLOOR: f32 = 0.35;
pub const DAMAGE_SPEED_CURVE: f32 = 2.2;

// Acceleration: how fast a tank's actual velocity can chase its commanded
// target (see Game::drive_tank), expressed as a force so mass genuinely
// matters (F = m*a - a heavier tank, if mass ever varies, ramps slower for
// the same force) rather than a flat px/s^2 every tank shares regardless of
// mass. Both figures are also scaled by Tank::speed_factor, so a damaged
// tank is sluggish to speed up, not just capped at a lower top speed.
// Deceleration is deliberately weaker than acceleration - releasing a
// direction (or getting knocked off course) lets the tank slide for a beat
// rather than stopping dead, which is what actually makes driving more
// challenging, not just "slower everywhere." This same chase-toward-target
// mechanism is also what decays ram/explosion/shell knockback now: a hit
// pushes velocity away from wherever the tank is trying to go, which reads
// as "slow down and correct," so it fades out via the (weak) decel rate
// instead of a separate hand-decayed field.
pub const TANK_ACCEL_FORCE: f32 = 1800.0; // reaches TANK_SPEED in well under a second
pub const TANK_DECEL_FORCE: f32 = 600.0; // noticeably slower to shed speed than to gain it

// Turning grip: how hard a tank's tracks cancel velocity *perpendicular* to
// the currently commanded direction (see Game::drive_tank). Much stronger
// than either accel or decel on purpose - real tank tracks don't slip
// sideways the way wheels do, so a corner should snap onto the new axis
// within a couple of frames rather than sliding through it like a car. Not
// scaled by Tank::speed_factor: track grip is a mechanical property, not an
// engine-power one, so a damaged tank still corners sharply even though it
// accelerates and tops out slower. Only applies while a direction is
// actively held (Intent::move_dir is Some) - a coasting or stationary tank
// has no commanded axis to grip against, so residual/knockback velocity
// there still fades via the (weak) TANK_DECEL_FORCE chase above.
pub const TANK_TURN_GRIP_FORCE: f32 = 6000.0;

// Shell ammo: the player holds up to MAX_SHELLS and recharges one shell every
// SHELL_RECHARGE_SECONDS while below the cap.
pub const MAX_SHELLS: i32 = 7;
pub const SHELL_RECHARGE_SECONDS: f32 = 2.0;

// Ramming: after taking collision damage a tank is immune for this long, so
// continuous touching doesn't drain damage every frame.
pub const RAM_DAMAGE_COOLDOWN: f32 = 0.5;

// A ram also gives both tanks a brief, small knockback shove apart (a real
// physics impulse - see Game::ram), scaled by their closing speed and
// normalized mass (hull area, i.e. scale squared - see Tank::mass) so a
// lighter tank gets shoved further than a heavier one. Wrecks are treated as
// infinite mass in both this and the explosion knockback below -
// already-dead hulks stay put when hit. The push itself decays via the same
// acceleration chase that governs normal driving (see TANK_DECEL_FORCE
// above), not a separate damping constant.
pub const KNOCKBACK_STRENGTH: f32 = 0.2; // fraction of ram closing speed converted to push speed
pub const KNOCKBACK_MAX_SPEED: f32 = 60.0; // px/s cap on any one push - keeps it small

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

// Friendly-fire avoidance: shells can hit any tank except the one that fired
// them (see Game::update), so an enemy lined up on the player with another
// enemy sitting on that same firing line, closer than the player, is about to
// shoot a teammate. When that happens, this is the chance the enemy holds its
// fire instead - not a hard block, so stray friendly fire still happens
// sometimes rather than enemies being perfectly coordinated. See
// Brain::friendly_blocks_shot/act_attack in ai.rs.
pub const ENEMY_FRIENDLY_FIRE_HOLD_CHANCE: f32 = 0.6;

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
// Tightened ~70% from an original 16.0. A tank's real turning arc - the
// stretch where its velocity genuinely has both axes' components at once
// (see TANK_TURN_GRIP_FORCE/Game::drive_tank) - only lasts on the order of
// 30px, so this needs to be small enough for several *real* samples to fall
// within that short a distance; Game::lay_tracks stamps each mark's raw,
// un-smoothed travel heading, so the curve you see is exactly the tank's
// actual path, not an interpolation - the sampling density is what's tuned
// here, not the curviness itself.
pub const TRACK_SPACING: f32 = 5.0; // distance travelled between dropped marks
pub const TRACK_LIFETIME: f32 = 1.0; // seconds for a mark to fully fade away
// Marks are drawn smaller than the tank and faint, so the trail reads as a subtle
// impression in the ground rather than a bold sprite.
pub const TRACK_SCALE_FRACTION: f32 = 0.55; // mark size relative to the tank sprite
pub const TRACK_MAX_OPACITY: f32 = 0.21; // opacity of a fresh mark, before fading (30% lighter than 0.3)

// Per-tank track "distortion": without it, a tank driving in a straight line
// stamps perfectly identical marks in a perfectly straight line, which reads
// as mechanical and homogenous. Each tank rolls its own wobble
// amplitude/wavelength/phase and scale jitter once at spawn (see
// Tank::track_wobble_amp/track_wobble_freq/track_wobble_phase/
// track_scale_jitter) and reuses them for every mark it lays for the whole
// round, rather than randomizing per-mark - so a given tank's whole trail
// reads as one coherent, tank-specific tread pattern (a wavier or straighter
// "signature") instead of per-mark noise. See Game::lay_tracks.
pub const TRACK_WOBBLE_AMP_MIN_DEG: f32 = 1.5;
pub const TRACK_WOBBLE_AMP_MAX_DEG: f32 = 6.0;
// Wavelength range: pixels of travel per full side-to-side wobble cycle.
pub const TRACK_WOBBLE_WAVELENGTH_MIN: f32 = 40.0;
pub const TRACK_WOBBLE_WAVELENGTH_MAX: f32 = 120.0;
// +/- fraction of TRACK_SCALE_FRACTION, so some tanks press slightly wider or
// narrower tread marks than others.
pub const TRACK_SCALE_JITTER: f32 = 0.15;

// Shockwave post-processing effect triggered when a tank is destroyed: a ring
// of radial distortion expands from the hit point over the whole screen and
// fades out. See `shockwave.rs` and `Game::render`.
pub const SHOCKWAVE_DURATION: f32 = 1.0; // seconds the effect plays before clearing
pub const SHOCKWAVE_SPEED: f32 = 0.8; // ring growth speed, UV units/sec
pub const SHOCKWAVE_WIDTH: f32 = 0.08; // thickness of the distorted band, UV units
pub const SHOCKWAVE_STRENGTH: f32 = 0.03; // how hard the ring bends the image, UV units

// A tank's death also deals a small splash of damage to any tank on the
// *opposing* side caught nearby, on top of the shockwave visual. Deliberately
// weak - a chip of damage, not a second kill shot - and never chips a tank's
// own side.
pub const EXPLOSION_RADIUS: f32 = 110.0; // px a wrecked tank's blast reaches
pub const EXPLOSION_DAMAGE_MIN: f32 = 3.0;
pub const EXPLOSION_DAMAGE_MAX: f32 = 8.0;
// Unlike the damage above, the outward shove isn't side-restricted: a real
// shockwave doesn't check allegiance, so it gives *every* live tank it
// reaches (opposing side or the dead tank's own) a shove - same knockback
// mechanism as a ram (Tank::mass, Game::ram), but driven by distance from
// the blast instead of closing speed: full push at the center, tapering
// linearly to nothing at EXPLOSION_RADIUS.
pub const EXPLOSION_KNOCKBACK_SPEED: f32 = 90.0; // px/s push at ground zero

// A shell impact also gives the tank it hits a small shove along the
// shell's travel direction - much weaker than a ram or explosion (deliberately
// a "tap", not a shove), and, like both of those, skipped if this very hit
// just wrecked the tank.
pub const SHELL_IMPACT_KNOCKBACK_SPEED: f32 = 35.0; // px/s push on a shell hit

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
// punch's full reach plus its band width. At the default 720px-tall window
// that reach is (IMPACT_FLASH_SPEED * IMPACT_FLASH_DURATION +
// IMPACT_FLASH_WIDTH) * 720 =~ 125px (the ripple shaders' "UV units" are
// normalized by screen *height*, not width - see the aspect-correction in
// impact.fs) - was previously set to 70, well short of that, which visibly
// clipped the outer edge of the punch.
pub const IMPACT_FLASH_QUAD_RADIUS: f32 = 130.0;

// Physics world: rapier2d integration (see docs/physics-engine-design.md).
// The battlefield boundary is 4 static wall colliders whose inner faces sit
// exactly at the screen edges, matching the old hand-rolled clamp bound;
// this is just how far they extend outward - purely internal, never
// rendered, so the exact value doesn't matter as long as it's comfortably
// more than a tank can move in one physics step.
pub const WALL_THICKNESS: f32 = 100.0;
// The physics world steps at a fixed rate regardless of render frame rate,
// so contact resolution stays consistent; `Game::update` accumulates real
// frame time and drains it in this many fixed chunks. Matches
// rapier2d::prelude::IntegrationParameters::default().dt.
pub const PHYSICS_FIXED_DT: f32 = 1.0 / 60.0;
// Caps how much real time a single frame's accumulator can catch up on, so a
// long stall (window drag, backgrounded tab) doesn't dump a burst of extra
// physics steps once it resumes (the classic fixed-timestep "spiral of
// death").
pub const PHYSICS_MAX_CATCHUP_SECONDS: f32 = 0.25;

// HUD: the SHELLS/HP numbers (Game::render) shift color as that resource
// drops, as a fraction of its max (MAX_SHELLS / MAX_DAMAGE respectively) -
// default gray above HUD_WARN_THRESHOLD, orange between the two, red below
// HUD_CRITICAL_THRESHOLD. Conservative on purpose: only flag real trouble,
// not every routine dip.
pub const HUD_WARN_THRESHOLD: f32 = 0.40;
pub const HUD_CRITICAL_THRESHOLD: f32 = 0.15;

pub mod ai;
pub mod bt;
pub mod damage_stage;
pub mod game;
pub mod physics;
pub mod shell;
pub mod shockwave;
pub mod tank;
pub mod track;
