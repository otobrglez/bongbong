use sola_raylib::prelude::*;

/// A 2D screen position in pixels.
pub type Position = Vector2;

// scifi_tanks_sheet.png is a 416x384 atlas: 13 columns x 12 rows of 32x32
// tiles (see docs/SPRITESHEET_SPEC.md for the full authored spec). Each row
// is one complete, independently-styled tank ("scout", "assault", ... plus
// the super-heavy `titan`/`leviathan` at rows 10/11 - TANK_VARIANTS/
// TANK_SPRITE_ORDER in simulation.rs pick which row a given tank uses).
// Columns:
//   0      hull, idle / tread-animation frame 0
//   1      turret (independently drawn, see `tank::draw_tank`)
//   2,3,4  hull, tread-animation frames 1-3 (see TANK_HULL_TRACK_COLS)
//   5      broken turret (severed barrel) - shown once a tank is a wreck
//   6      hull, "light" damage tier - cosmetic, still fully mobile
//   7      hull, "disabled" tier - heavy but non-fatal damage
//   8-11   hull, four interchangeable wreck variants (see TANK_WRECK_COLS) -
//          peers, not a severity sequence: pick one at random per kill
//   12     ground track-mark decal, per-chassis - not used; the game keeps
//          its existing generic ground-decal system (track.rs) instead
// Both hull and turret are authored around the exact same pivot - the cell
// center (16,16) - where the hull has a recessed turret-mount ring and the
// turret is drawn around that same ring center, not its own bounding box.
// Drawing both layers at that literal center (see TANK_PIVOT_REAR_FRACTION
// below) is what keeps the turret visually seated on the ring at every
// angle. Both layers still chase the same commanded heading (no independent
// aim target) but ease toward it at their own rates - see
// `Tank::visual_rotation`/`turret_visual_rotation` and
// TANK_VISUAL_TURN_SPEED_DEG/TANK_TURRET_VISUAL_TURN_SPEED_DEG below - so the
// turret visibly leads the hull into a turn instead of the two rotating in
// lockstep.
pub const TANK_TEXTURE_SIZE: f32 = 32.0;
pub const TANK_HULL_COL: i32 = 0;
pub const TANK_TURRET_COL: i32 = 1;
// Hull tread-animation loop, in atlas-column order - see
// `Tank::hull_frame`/TANK_HULL_TRACK_FRAME_DISTANCE and
// `simulation::lay_tracks`. Always played forward regardless of movement
// direction: this game's 4-direction snap-to-facing movement has no
// continuous "current heading" for a press to be a reversal *relative to*,
// unlike the spec's forward/reverse cycle (meant for engines with continuous
// turning), so there's no meaningful "reverse" state to distinguish here.
pub const TANK_HULL_TRACK_COLS: [i32; 4] = [0, 2, 3, 4];
pub const TANK_BROKEN_TURRET_COL: i32 = 5;
pub const TANK_HULL_LIGHT_COL: i32 = 6;
pub const TANK_HULL_DISABLED_COL: i32 = 7;
// Four interchangeable wrecked-hull variants - see `Tank::wreck_col`, rolled
// once per tank the frame it first becomes a wreck (`simulation::Game::update`)
// and kept for the rest of its lifetime, rather than picked by severity, so a
// field of wrecks doesn't look copy-pasted.
pub const TANK_WRECK_COLS: [i32; 4] = [8, 9, 10, 11];
// Damage level (see Tank::damage) at which a still-alive tank's hull swaps to
// the "light" damage art (TANK_HULL_LIGHT_COL, cosmetic - still fully
// mobile), matching damage_stage.rs's existing "gray" tier. The heavier
// "disabled" tier (TANK_HULL_DISABLED_COL) matches damage_stage.rs's
// existing "large fire" tier, so the hull swaps and the smoke/fire overlay
// escalate together. The wrecked hull/broken turret swap instead keys off
// Tank::is_wreck() (damage >= MAX_DAMAGE), matching the spec's own state
// machine.
pub const TANK_HULL_LIGHT_DAMAGE: f32 = 30.0;
pub const TANK_HULL_DISABLED_DAMAGE: f32 = 75.0;
// World px of travel between hull tread-animation frame advances (see
// `simulation::lay_tracks`, which already tracks per-frame distance moved for
// the separate ground-decal system in track.rs - this reuses that same
// distance, just accumulated into its own field so the two animations - tank
// tread graphics vs. ground tread marks - stay independently tunable).
pub const TANK_HULL_TRACK_FRAME_DISTANCE: f32 = 8.0;
// The tank hull only fills part of its 32x32 tile (the rest is transparent
// padding). This scalar fraction still backs the AI's avoidance-radius math,
// the ground-decal rear-edge offset, and spawn-clearance checks (see
// `Tank::hull_size`) - all of those only need an approximate footprint, so
// they're left alone. The tank's actual physics collider is sized more
// precisely per row instead - see TANK_HULL_BBOX_BY_ROW.
pub const TANK_HULL_FRACTION: f32 = 0.7;
// Per-row hull footprint (width, height) in tile px, in the sprite's own
// "facing up" reference frame - measured bounding boxes from the spec's §9
// table. Feeds the tank's physics collider (`Physics::spawn_tank`/
// `resize_collider`, see `simulation::drive_tank`), which - unlike
// TANK_HULL_FRACTION above - is sized and reoriented per tank so a `titan`
// (nearly filling its cell) and a `flak` (much smaller) don't share a
// one-size-fits-all hitbox. Indexed by `Tank::row`.
pub const TANK_HULL_BBOX_BY_ROW: [(f32, f32); 12] = [
    (14.0, 19.0), // scout
    (16.0, 22.0), // assault
    (18.0, 24.0), // breaker
    (16.0, 26.0), // longbow
    (16.0, 18.0), // flak
    (14.0, 19.0), // wraith
    (16.0, 22.0), // warden
    (18.0, 24.0), // ravager
    (16.0, 17.0), // glacier
    (16.0, 26.0), // obelisk
    (24.0, 26.0), // titan (super-heavy)
    (22.0, 28.0), // leviathan (super-heavy)
];
// The roster groups into 7 named chassis classes by handling weight (a
// coarser grouping than TANK_HULL_BBOX_BY_ROW's precise per-row collision
// measurements, and independent of it - this table drives `Tank::mass`
// only, not the collider):
//   narrow      12x18   scout, wraith
//   compact     14x16   flak, glacier
//   std         14x20   assault, warden
//   long        14x24   longbow, obelisk
//   wide        16x22   breaker, ravager
//   super_heavy 22x24   titan
//   super_long  20x26   leviathan
// Each entry here is that chassis's (width*height) footprint area divided by
// `std`'s (14*20=280), i.e. how much heavier/lighter that class is relative
// to `std` - `std` itself normalizes to exactly 1.0. `Tank::mass` multiplies
// this onto the old flat `scale^2` baseline (see its doc comment), so a
// `std`-class tank's mass is unchanged from before this table existed, and
// every other class scales relative to that same baseline: `narrow`/
// `compact` end up lighter (faster to accelerate, less drift, easier to
// knock around - see `Game::drive_tank`/`ram`/`explosion_hit`, all of which
// already divided by `Tank::mass` and so pick this up automatically), while
// `long`/`wide` and especially `super_heavy`/`super_long` end up
// meaningfully heavier (sluggish to accelerate, more perpendicular drift
// through a turn, and shove lighter tanks further than they get shoved back
// in a ram).
pub const TANK_CHASSIS_MASS_FACTOR_BY_ROW: [f32; 12] = [
    216.0 / 280.0, // scout       (narrow)
    1.0,           // assault     (std)
    352.0 / 280.0, // breaker     (wide)
    336.0 / 280.0, // longbow     (long)
    224.0 / 280.0, // flak        (compact)
    216.0 / 280.0, // wraith      (narrow)
    1.0,           // warden      (std)
    352.0 / 280.0, // ravager     (wide)
    224.0 / 280.0, // glacier     (compact)
    336.0 / 280.0, // obelisk     (long)
    528.0 / 280.0, // titan       (super_heavy)
    520.0 / 280.0, // leviathan   (super_long)
];
// Per-chassis shell damage multiplier, applied on top of
// PLAYER_DAMAGE_MIN/MAX or ENEMY_DAMAGE_MIN/MAX depending on who fired (see
// `Shell::shooter_row`/its use in `Game::update`'s hit-resolution). Same 7
// chassis grouping as TANK_CHASSIS_MASS_FACTOR_BY_ROW (see its doc comment
// for the class table) but tuned by the sheet's own role hints rather than
// mirroring that table's pure footprint-area math - a sniper platform
// (`long`) should hit hard without needing to be the heaviest thing on the
// field. `std` again normalizes to exactly 1.0, so `assault`/`warden` deal
// exactly the damage every tank used to deal before this table existed.
// Pairs with the mass table above by design: `super_heavy`/`super_long` are
// already the slowest, most drift-prone chassis to drive - this gives them
// the payoff (hits hardest) to go with that cost, while `narrow` leans
// further into "fast and evasive, but weak."
pub const TANK_CHASSIS_DAMAGE_FACTOR_BY_ROW: [f32; 12] = [
    0.75, // scout       (narrow)       - fast recon
    1.0,  // assault     (std)          - general purpose
    1.20, // breaker     (wide)         - heavy brawler
    1.35, // longbow     (long)         - artillery/sniper
    0.90, // flak        (compact)      - anti-air/close range
    0.75, // wraith      (narrow)       - stealth
    1.0,  // warden      (std)          - support/defense
    1.20, // ravager     (wide)         - heavy assault
    0.90, // glacier     (compact)      - balanced
    1.35, // obelisk     (long)         - siege
    1.55, // titan       (super_heavy)  - super-heavy assault
    1.60, // leviathan   (super_long)   - super-heavy siege
];
// How far forward (tile px, toward wherever the hull currently faces) a
// shell spawns from the tank's center - i.e. where the turret/barrel tip
// actually sits, not the tank's own center or the edge of its 32x32 tile.
// Varies per row: taken directly from the spec's published turret
// bounding-box y0 per row (§9), converted to an above-center distance
// (16 - y0). Indexed by `Tank::row` - see `Shell::spawn`.
pub const TANK_MUZZLE_FORWARD_OFFSET_BY_ROW: [f32; 12] = [
    14.0, 13.0, 12.0, 16.0, 10.0, 13.0, 14.0, 14.0, 14.0, 16.0, 16.0, 16.0,
];
// Draw-time rotation pivot, as a fraction of the sprite's width shifted back
// toward the rear of the hull (away from the barrel) - see
// `tank::draw_pivot`. Zero for this atlas: the spec's hull/turret pivot is
// the exact cell center (see the atlas comment above), and any rearward
// shift would visibly drift the turret off the hull's mount ring as it
// rotates. Purely cosmetic either way (draw calls only, see `draw_tank`/
// `draw_tank_shadow`) - `Tank::position` itself is still the physics body's
// real center and everything gameplay-relevant (collision, aim, track
// heading) keeps using it unchanged.
pub const TANK_PIVOT_REAR_FRACTION: f32 = 0.0;

// shells.png is a 224x576 sheet: seven 32x32 frames (col, see ShellState)
// indexed across eighteen row-variants (see SHELL_VARIANTS / Tank::shell_variant)
// that reskin the fire and hit frames while keeping the flying frame (col 3)
// pixel-identical across all rows. Rows are grouped class_base + colour:
//   class_base: standard single = 0, standard twin = 3, standard staggered = 12
//               super single    = 6, super twin    = 9, super staggered    = 15
//   colour:     orange = +0, red = +1, blue = +2
// ("staggered" rows depict the same twin-barrel shot with the left round
// running a beat ahead of the right - see TANK_SHELL_VARIANT_STAGGERED_BY_ROW
// - purely a cosmetic alternate to the simultaneous twin rows, still one
// `Shell` entity per shot either way.) See TANK_SHELL_VARIANT_BY_ROW below for
// which row matches which of the 12 tank chassis.
pub const SHELL_TEXTURE_SIZE: f32 = 32.0;
// Row count in shells.png (see the grouping above).
pub const SHELL_VARIANTS: i32 = 18;
// Shell row matched to each tank chassis row's real barrel count (single vs
// twin), size class (standard vs the two super-heavy titan/leviathan
// chassis), and accent colour - see the class_base/colour grouping in the
// atlas comment above. Longbow (green accent) and wraith (purple accent)
// have no matching shell colour family and fall back to blue. Set once at
// `Tank::shell_variant` on spawn (simulation.rs) and re-set to this (or
// TANK_SHELL_VARIANT_STAGGERED_BY_ROW - see Tank::alternate_shot) every time
// that tank fires.
pub const TANK_SHELL_VARIANT_BY_ROW: [i32; 12] = [
    2,  // 0  scout      standard single, blue (cyan accent)
    3,  // 1  assault    standard twin,   orange (amber accent)
    1,  // 2  breaker    standard single, red
    2,  // 3  longbow    standard single, blue (no green family)
    5,  // 4  flak       standard twin,   blue (cyan accent)
    2,  // 5  wraith     standard single, blue (no purple family)
    2,  // 6  warden     standard single, blue (cyan accent)
    3,  // 7  ravager    standard twin,   orange (amber accent)
    2,  // 8  glacier    standard single, blue
    4,  // 9  obelisk    standard twin,   red
    10, // 10 titan      super twin,      red
    8,  // 11 leviathan  super single,    blue (cyan accent)
];
// Staggered-twin counterpart of TANK_SHELL_VARIANT_BY_ROW, for alternating
// fire on twin-barrel chassis (see Tank::alternate_shot). Rows with no twin
// variant (single-barrel chassis) repeat their base value - toggling between
// them is then a harmless no-op.
pub const TANK_SHELL_VARIANT_STAGGERED_BY_ROW: [i32; 12] =
    [2, 12, 1, 2, 14, 2, 2, 12, 2, 13, 16, 8];
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
// Number of row-variants in damage.png (see SHELL_VARIANTS for the same
// pattern). Each tank rolls one at spawn (Tank::damage_variant) and keeps it
// for its whole life, so a tank's whole damage sequence reads as one
// consistent "flavour" instead of jumping between palettes frame to frame.
pub const DAMAGE_VARIANTS: i32 = 5;

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
// Game::drive_tank, TANK_ACCEL_FORCE/TANK_DECEL_CURVE_RATE below. A tank's tracks
// still resist lateral sliding - any velocity component *perpendicular* to
// the commanded direction is scrubbed toward zero by TANK_TURN_GRIP_FORCE -
// but deliberately not as hard as the new axis builds up, so a corner reads
// as a genuine drift through the turn (the old heading's momentum visibly
// carries through) rather than the hull snapping instantly onto the new
// axis. Feedback from actually driving it: full control (snap-to-new-axis,
// no carry-through) read as too clinical, especially cornering hard off a
// straight line (e.g. up into left) - see TANK_TURN_GRIP_FORCE's comment for
// the numbers.
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
// mass. Also scaled by Tank::speed_factor, so a damaged tank is sluggish to
// speed up, not just capped at a lower top speed.
// TANK_ACCEL_FORCE is kept meaningfully *stronger* than TANK_TURN_GRIP_FORCE
// below (see that constant's comment for the drift this produces) so a
// 90-degree turn still doesn't stall: even though the old axis's momentum
// now lingers instead of being scrubbed away immediately, the new axis's
// speed builds up faster than that old-axis component decays, so the
// resultant speed through a corner stays close to (or even briefly above)
// top speed rather than bottoming out near zero for half a second while the
// new axis crawls back up - the failure mode an earlier, far-weaker-than-grip
// accel tuning had. A weaker-than-accel grip is a different regime from that
// old bug, not a reversion to it: the stall came from grip *dominating*
// accel (scrubbing the old axis faster than the new one could build), not
// from grip merely existing.
pub const TANK_ACCEL_FORCE: f32 = 4200.0; // reaches TANK_SPEED in well under a second

// Deceleration (releasing a direction, reversing, or coasting off a
// ram/explosion/shell knockback - see Game::drive_tank) used to chase toward
// zero with the same flat-force model as acceleration above, just at a much
// weaker force: a constant per-frame cap, so speed bled off in a straight
// line over a fixed span of time regardless of how fast the tank was going.
// Playtester feedback (a Slovenian-language note) was that this read as too
// slow to stop and too flat/mechanical rather than a genuine brake - asked
// for a curve instead of linear, and to lean into that curve rather than
// just nudge the old number. Braking now eases current_on toward target_on
// exponentially instead: `1 - exp(-TANK_DECEL_CURVE_RATE * dt)` is the
// fraction of the remaining speed gap closed this frame, frame-rate
// independent. That's a curve by construction - it bites hardest right when
// a direction is released (most of the speed sheds in the first ~1/RATE
// seconds) and tapers as the tank nears a stop, rather than shedding speed
// at a constant rate the whole way down. Both old and new rates divide by
// Tank::mass same as accel (a `std`-class tank at its default scale sits at
// mass 4.0 - see TANK_CHASSIS_MASS_FACTOR_BY_ROW's doc comment - so that's
// the reference point below, not unit mass): the tuned rate below gives that
// baseline tank a full-speed-to-stop that reads as "most of the way stopped"
// in ~150ms, vs. the old flat-600-force linear ramp's TANK_SPEED * mass / 600
// = ~1.5s to fully stop - not just a curve, a genuinely faster one. Verified
// headlessly with `cargo run --bin probe -- --scenario brake` (see
// probe.rs) rather than just by the arithmetic here, since Game::apply_impulse
// feeding rapier's own integration is one more step removed from these
// constants than a plain formula. Still scaled by Tank::speed_factor like
// accel, so a damaged tank is sluggish to brake too, not just to speed up.
// The exponential only approaches zero asymptotically, so TANK_DECEL_SNAP_PX
// is the remaining speed-gap threshold below which Game::drive_tank snaps
// straight to the target instead of trailing an imperceptible tail forever.
// This is the same chase-toward-target mechanism ram/explosion/shell knockback decay
// rides on (a hit pushes velocity away from wherever the tank is trying to
// go, which reads as "slow down and correct"), so that knockback now decays
// on the same curve rather than a separate hand-decayed field.
pub const TANK_DECEL_CURVE_RATE: f32 = 80.0; // 1/s - higher = snappier stop; this *is* "the curve"
pub const TANK_DECEL_SNAP_PX: f32 = 3.0; // px/s gap below which the tail snaps to target

// Turning grip: how hard a tank's tracks cancel velocity *perpendicular* to
// the hull's current facing (`Tank::rotation` - see Game::drive_tank).
// Deliberately *weaker* than TANK_ACCEL_FORCE now (was 4800, kept at/above
// accel) - direct player feedback on the fully-snapped version was that it
// read as too clinically controlled through a corner (e.g. driving up then
// cutting hard into left), with no sense of the tank's own momentum ever
// carrying through a turn. That first drift pass landed at 2000 (vs. accel's
// 4200), giving on the order of half a second for the old axis's velocity to
// fully scrub out through a turn at top speed; a follow-up pass then dialed
// the drift back down ~10% (2000 -> 2200) once it was actually driven, so
// the scrub is correspondingly a bit quicker now (~10% less time carrying
// old-axis momentum) while still comfortably below accel and still a
// genuine, visible drift rather than a snap. Grip and decel govern different
// axes (perpendicular vs. along-facing) on different models now (grip is
// still a flat force, decel is the exponential curve above) and don't need
// to match numerically, but grip's ~100ms scrub-to-zero at top speed is
// still comfortably quicker than decel's ~150ms curve to a near-stop, so a
// sideways ram/knockback still reads as "grip catches it fast" rather than
// sliding as long as a full coasting stop. Not scaled by
// Tank::speed_factor: track grip is a mechanical property, not an
// engine-power one, so a damaged tank still corners with the same drift
// character even though it accelerates and tops out slower. Applies every
// frame regardless of whether a direction is currently held (real tracks
// resist lateral sliding all the time, not just while the driver is actively
// steering - see Game::drive_tank's own doc comment) - so a ram/explosion/
// shell knockback that shoves a tank sideways to wherever it's currently
// facing also gets scrubbed by this (faster than a coasting stop would via
// the much weaker TANK_DECEL_CURVE_RATE, just no longer almost-instantly).
pub const TANK_TURN_GRIP_FORCE: f32 = 2200.0;

// Purely cosmetic hull-turn animation: `Tank::rotation` itself still snaps
// instantly (physics/aim/track-heading math all key off it, and none of that
// should lag a frame behind input - see Game::drive_tank, Shell::spawn).
// `Tank::visual_rotation` is a second angle that only the sprite draw calls
// read (draw_tank/draw_tank_shadow); it chases `rotation` at this many
// degrees per second (shortest way round) instead of snapping with it, so a
// 90-degree corner or a 180-degree reversal visibly swings the hull over a
// few frames rather than popping to the new facing on the spot.
pub const TANK_VISUAL_TURN_SPEED_DEG: f32 = 720.0;

// The turret (scifi_tanks_sheet.png's second column, see TANK_TURRET_COL) eases
// toward the same commanded `rotation` independently of the hull, at its own
// (faster) turn speed - see `Tank::turret_visual_rotation`/
// `Tank::ease_turret_visual_rotation`. This makes the turret visibly lead a
// turn while the heavier hull swings around to catch up, rather than the two
// rotating in lockstep.
pub const TANK_TURRET_VISUAL_TURN_SPEED_DEG: f32 = 1800.0;

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
// deceleration curve that governs normal braking (see TANK_DECEL_CURVE_RATE
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

// Ammo-aware aggression: an enemy that runs low on shells breaks off and backs
// away (without firing) until it has recharged enough to rejoin the fight,
// rather than plinking the player empty. The two thresholds are deliberately
// apart (not a single cutoff) so the enemy doesn't flicker between
// retreating and attacking every frame it hovers near one value - see
// Ai::wants_retreat in ai.rs.
pub const ENEMY_AMMO_LOW: i32 = 2; // retreat once ammo drops to/below this
pub const ENEMY_AMMO_RESUME: i32 = 5; // must recharge back up to this to re-engage
// While retreating, back off only to breathing room outside attack range,
// not all the way to the map edge like the health-based flee does.
pub const ENEMY_RETREAT_RANGE: f32 = ENEMY_ATTACK_RANGE * 1.3;
// The fuller an enemy's magazine, the faster it re-fires: at MAX_SHELLS it
// uses this interval instead of the baseline ENEMY_FIRE_INTERVAL; ammo
// between ENEMY_AMMO_LOW and MAX_SHELLS linearly interpolates between the
// two - see Brain::fire_interval in ai.rs.
pub const ENEMY_FIRE_INTERVAL_AGGRESSIVE: f32 = 1.4;

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
// A committed heading that's about to walk into a known-blocked grid cell
// (Ai::steer's obstacle-ahead override) still needs at least this much dwell
// time before it can be overridden *again* - much shorter than
// AI_DIR_HOLD_SECONDS (a real obstacle should be reacted to fast), but
// without some floor, a coarse grid's routed direction wobbling by a cell
// near a corner can flip the override back and forth every single frame,
// reintroducing the exact jitter AI_DIR_HOLD_SECONDS/AI_DIR_SWITCH_MARGIN_PX
// exist to prevent (found via the probe harness's `--rounds` sweep: jitter
// counts rose sharply without this).
pub const AI_OBSTACLE_OVERRIDE_HOLD_SECONDS: f32 = 0.1;

// AI pathfinding (see pathfind.rs): a coarse grid A* layer so an enemy
// routes around static obstacles (obstacle.rs) instead of just walking into
// one and getting physically stuck by its collider - `Ai::steer` swaps its
// naive straight-line heading for the grid's first step toward the target.
// Rebuilt fresh every frame in Game::update (obstacles are few and the grid
// is small, so this is cheap enough not to need caching/invalidation).
pub const PATHFIND_CELL_SIZE: f32 = 48.0; // px per grid cell

// Stuck-escape: a tank the AI has been commanding to move (see Ai::think's
// `was_moving`/`stuck_timer`) whose real physics speed stays under
// STUCK_SPEED_EPS for STUCK_ESCAPE_SECONDS running is treated as genuinely
// stuck - not just slow-to-turn (see AI_DIR_HOLD_SECONDS above, a much
// shorter window for a different problem) - and Ai::steer forces a hard
// perpendicular-turn reset instead of continuing to retry whatever's been
// failing. Catches both a bad commitment call and a layout with no path
// around an obstacle cluster at all (see pathfind.rs's Grid::next_step
// returning None).
pub const STUCK_SPEED_EPS: f32 = 8.0; // px/s - top speeds are 150-220 (TANK_SPEED/ENEMY_SPEED)
pub const STUCK_ESCAPE_SECONDS: f32 = 0.75;

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
// Tightened ~70% from an original 16.0, back when a tank's real turning arc -
// the stretch where its velocity genuinely has both axes' components at once
// (see TANK_TURN_GRIP_FORCE/Game::drive_tank) - only lasted on the order of
// 30px; TANK_TURN_GRIP_FORCE has since been loosened for more of a drift
// through corners, which stretches that arc out well past 30px, but this
// value needs no retuning for it - a longer arc with the same spacing just
// means more real samples along it, not fewer, which only helps the curve
// read cleanly. Game::lay_tracks stamps each mark's raw, un-smoothed travel
// heading, so the curve you see is exactly the tank's actual path, not an
// interpolation - the sampling density is what's tuned here, not the
// curviness itself.
pub const TRACK_SPACING: f32 = 5.0; // distance travelled between dropped marks
// Trimmed 20% from an original 1.0s - shortens how far a visible trail
// stretches out behind a moving tank (trail length is roughly speed *
// TRACK_LIFETIME at this spacing).
pub const TRACK_LIFETIME: f32 = 0.8; // seconds for a mark to fully fade away
// Marks are drawn smaller than the tank and faint, so the trail reads as a subtle
// impression in the ground rather than a bold sprite.
pub const TRACK_SCALE_FRACTION: f32 = 0.55; // mark size relative to the tank sprite
pub const TRACK_MAX_OPACITY: f32 = 0.21; // opacity of a fresh mark, before fading (30% lighter than 0.3)

// Per-chassis track "weight": without this, every row presses an identically
// sized/shaded mark regardless of how big or heavy that chassis actually is
// (see TANK_HULL_BBOX_BY_ROW - a titan's real footprint is over twice a
// scout's). Multipliers below are taken from docs/SPRITESHEET_SPEC.md §8's
// authored "intensity by chassis" table (narrow/compact/standard/long/wide/
// super-long/super-heavy, mapped to lightest..heaviest) - the same authored
// grouping the sheet's own per-chassis track-mark art would have used, kept
// here instead since the game draws marks from the single generic
// tracks.png tile, not that column. Applied on top of TRACK_SCALE_FRACTION/
// TRACK_MAX_OPACITY and the per-tank jitter/wobble below, so a titan visibly
// presses a bigger, darker mark than a scout, while still varying a little
// tank-to-tank within that.
pub const TRACK_WEIGHT_SCALE_BY_ROW: [f32; 12] = [
    0.75, // scout (narrow - lightest)
    1.00, // assault (standard - medium)
    1.20, // breaker (wide - heavier)
    1.10, // longbow (long - heavy)
    0.85, // flak (compact - light)
    0.75, // wraith (narrow - lightest)
    1.00, // warden (standard - medium)
    1.20, // ravager (wide - heavier)
    0.85, // glacier (compact - light)
    1.10, // obelisk (long - heavy)
    1.45, // titan (super-heavy - heaviest)
    1.35, // leviathan (super-long - very heavy)
];
// Multiplies TRACK_MAX_OPACITY per row, same tiers/ordering as
// TRACK_WEIGHT_SCALE_BY_ROW - a heavier chassis presses a darker mark, not
// just a bigger one.
pub const TRACK_WEIGHT_OPACITY_BY_ROW: [f32; 12] = [
    0.70, 1.00, 1.20, 1.10, 0.82, 0.70, 1.00, 1.20, 0.82, 1.10, 1.50, 1.35,
];

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

// Obstacles: static battlefield terrain (see obstacle.rs), placed once per
// round alongside enemies, built from one of four materials
// (obstacle::Material). walls_sheet.png is a 256x448 sheet (8 cols x 14 rows
// of 32x32 cells) - see docs/WALLS_SPEC.md for the full spec:
//   rows 0-3  Brick  (cols 0-5 valid): 4 bond-pattern variants, 6 decay
//             stages (col 5 = rubble, i.e. destroyed - never actually drawn,
//             see Obstacle::damage, same "vanishes the instant it dies"
//             behaviour the old Crate always had).
//   rows 4-7  Iron   (cols 0-3 valid): 4 surface-treatment variants, never
//             destroyed - the column only tracks a cosmetic rust stage that
//             plateaus once fully weathered (Obstacle::damage never sets
//             `destroyed` for Iron).
//   rows 8-11 Wood   (cols 0-7 valid): 4 board-layout variants sharing one
//             8-state lifecycle: intact/damaged/heavily damaged, then a
//             fork rolled once at spawn (Obstacle::flammable) - either
//             straight to destroyed (col 3) or a 3-frame burning loop (cols
//             4-6, see Obstacle::tick_burn) into charred (col 7).
//   rows 12-13 Glass (cols 0-3 valid): 2 variants (plain pane / reinforced
//             frame - cosmetic only for now), 4 states from intact to
//             shattered/destroyed.
// Cells outside a material's valid column range are empty - never sampled
// (see Material::variants/visible_stages).
pub const OBSTACLE_TEXTURE_SIZE: f32 = 32.0;
// 1.0, not TANK_SCALE-style 2.0 - an obstacle renders at its raw 32px tile
// size, matching OBSTACLE_GRID_SIZE exactly (see below) rather than being
// twice as big as one grid cell.
pub const OBSTACLE_SCALE: f32 = 1.0;
// Per-material toughness: hp absorbed before reaching the terminal state
// (rubble/charred/shattered), or - for Iron - before its rust stage
// plateaus. Ordered fragile to tough: glass snaps almost immediately, wood
// breaks easily, brick holds longer, iron the longest of all on top of
// being permanent.
pub const OBSTACLE_GLASS_MAX_HEALTH: f32 = 20.0;
pub const OBSTACLE_WOOD_MAX_HEALTH: f32 = 35.0;
pub const OBSTACLE_BRICK_MAX_HEALTH: f32 = 70.0;
pub const OBSTACLE_IRON_MAX_HEALTH: f32 = 220.0; // plateaus the rust stage, never destroys
// Fraction of spawned Wood obstacles that catch fire when destroyed instead
// of breaking outright (Obstacle::flammable, rolled once at spawn) -
// "breaks easily" vs "catches fire" is gameplay data layered onto shared
// art, per docs/WALLS_SPEC.md.
pub const OBSTACLE_WOOD_FLAMMABLE_CHANCE: f64 = 0.5;
// Burning wood's 3-frame flicker loop cadence (cols 4-6) - ~7.7 FPS, the
// spec's own suggested reading for a good flicker.
pub const OBSTACLE_WOOD_BURN_FRAME_SECONDS: f32 = 0.13;
// Total time a Wood obstacle spends in the burning loop before charring
// (col 7) and finally being removed - same instant-vanish-once-destroyed
// convention as every other material (see Obstacle::tick_burn).
pub const OBSTACLE_WOOD_BURN_SECONDS: f32 = 2.5;
// Collision footprint: slightly smaller than the full sprite tile, same
// reasoning as TANK_HULL_FRACTION - lets a shell's small hit sensor
// (SHELL_HIT_HALF_EXTENT) register a clean hit near the sprite's visible
// edge instead of stopping short at the tile's transparent padding.
pub const OBSTACLE_HULL_FRACTION: f32 = 0.75;
// Number of structures/clusters placed each round is randomized within this
// range (see structures.rs / OBSTACLE_STRUCTURE_CHANCE) - same pattern as
// ENEMY_COUNT_MIN/MAX. Each one contributes anywhere from 1 tile (a lone
// `structures::SINGLE`) to 5 (e.g. `structures::WALL_LINE_5`), so a round's
// actual total obstacle-tile count varies with which shapes get rolled.
pub const OBSTACLE_COUNT_MIN: usize = 2;
pub const OBSTACLE_COUNT_MAX: usize = 4;
// Chance a placement rolls a real multi-tile shape from `structures::STRUCTURES`
// rather than falling back to `structures::SINGLE` (a lone tile) - keeps
// lone obstacles common alongside deliberate little builds instead of every
// placement being an equally-likely full structure.
pub const OBSTACLE_STRUCTURE_CHANCE: f64 = 0.5;
// Obstacle spawn positions snap to a world-space grid this many px per cell
// (see `sample_structure_positions` in battlefield.rs) so walls - and every
// tile of a multi-tile structure - land visually aligned instead of at
// arbitrary fractional offsets. Exactly `Obstacle::size()`
// (OBSTACLE_TEXTURE_SIZE * OBSTACLE_SCALE) - one grid cell is one obstacle
// tile, so a grid-aligned obstacle's sprite exactly fills its cell with no
// gap or overlap.
pub const OBSTACLE_GRID_SIZE: f32 = OBSTACLE_TEXTURE_SIZE * OBSTACLE_SCALE;
// Minimum clearance (px) an obstacle spawn must keep from the player's
// start position and every enemy's start position - same idea as the
// enemy_clear/clear checks in Game::init, just reused for a third entity
// type. Obstacle-vs-obstacle spacing uses the separate, smaller
// OBSTACLE_CLUSTER_CLEAR instead (see below).
pub const OBSTACLE_CLEAR: f32 = 90.0;
// Minimum center-to-center spacing between any two obstacle tiles
// specifically - checked between every tile of a newly-placed structure and
// every tile placed so far, including tiles from within the *same*
// structure (adjacent `structures::Blueprint` offsets sit exactly one grid
// cell apart, i.e. exactly at this minimum). Exactly one full tile width -
// same value as OBSTACLE_GRID_SIZE now that a grid cell *is* a tile, so any
// two grid-aligned tiles are either exactly touching (edge-to-edge, matching
// walls_sheet.png's seamless-tiling art - see docs/WALLS_SPEC.md) or further
// apart, never overlapping. Kept as its own named constant (rather than
// reusing OBSTACLE_GRID_SIZE directly) since the two mean different things -
// this is a spacing rule, not a placement grid - and OBSTACLE_CLEAR (90px,
// the player/enemy-vs-obstacle spacing) is intentionally larger than either.
pub const OBSTACLE_CLUSTER_CLEAR: f32 = OBSTACLE_TEXTURE_SIZE * OBSTACLE_SCALE;

// Ground/terrain layer (grass base, road painted under every static
// obstacle tile and inside the fortress's B/O glyphs) - see ground.rs for
// the placement/autotile logic, docs/GROUND_SPEC.md for the full design
// writeup. Drawn from static/punyworld/punyworld-overworld-tileset.png, a
// third-party tileset - deliberately NOT on the Resurrect 64 palette every
// other sheet uses (see docs/PALETTE.md and static/punyworld/SOURCE.md), a
// documented exception rather than an oversight.
pub const GROUND_TEXTURE_SIZE: f32 = 16.0; // native tile size in the source sheet
// 2x, not OBSTACLE_SCALE-style 1.0 - the source art is 16px/tile, and
// GROUND_WORLD_TILE below needs to land on OBSTACLE_GRID_SIZE (32px) so the
// ground layer's grid lines up with the obstacle/physics grid (see
// ground.rs's module doc comment for the centered-vs-top-left-aligned
// phase-matching that actually makes this line-up hold in practice).
pub const GROUND_SCALE: f32 = 2.0;
pub const GROUND_WORLD_TILE: f32 = GROUND_TEXTURE_SIZE * GROUND_SCALE; // = OBSTACLE_GRID_SIZE

// The player's starting enclosure (see `battlefield::spawn_player_fortress`):
// the word "BONG!" spelled out in wall tiles, one material per glyph (B is
// Brick, the O the player spawns inside is Glass, N is Wood, G is Iron, and
// `!` reuses Brick - only four materials exist for five glyphs). Sized by
// the pixel font in `spawn_player_fortress`'s `GLYPH_*` constants directly,
// not by a tank-relative scale like the shape this replaced. Glass's low
// max_health (OBSTACLE_GLASS_MAX_HEALTH) is exactly why the O got that
// material: a shot or two from inside cracks an escape route, same role
// this enclosure's now-removed circular predecessor gave its weakest side.

pub const OBSTACLE_SHADOW_OFFSET: f32 = 3.0; // px, same grounded distance as TANK_SHADOW_OFFSET
pub const OBSTACLE_SHADOW_OPACITY: f32 = 0.35;

// HUD: the SHELLS/HP numbers (Game::render) shift color as that resource
// drops, as a fraction of its max (MAX_SHELLS / MAX_DAMAGE respectively) -
// default gray above HUD_WARN_THRESHOLD, orange between the two, red below
// HUD_CRITICAL_THRESHOLD. Conservative on purpose: only flag real trouble,
// not every routine dip.
pub const HUD_WARN_THRESHOLD: f32 = 0.40;
pub const HUD_CRITICAL_THRESHOLD: f32 = 0.15;

pub mod ai;
pub mod battlefield;
pub mod bt;
pub mod damage_stage;
pub mod game;
pub mod ground;
pub mod obstacle;
pub mod pathfind;
pub mod physics;
pub mod shell;
pub mod shockwave;
pub mod simulation;
pub mod structures;
pub mod tank;
pub mod track;
