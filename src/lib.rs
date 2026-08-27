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
// table. This is the tank's *damage* silhouette: it sizes the hull shell-hit
// sensor (`Tank::hit_sensor`/`Physics::add_hit_sensor`) and the swept-shell
// fallback check directly, and - unlike TANK_HULL_FRACTION above - is
// measured per row so a `titan` (nearly filling its cell) and a `flak`
// (much smaller) don't share a one-size-fits-all hitbox. The *movement*
// collider derives from this same table but deliberately smaller and
// rounder - see TANK_MOVE_BBOX_FRACTION/TANK_MOVE_CORNER_RADIUS just below.
// Indexed by `Tank::row`.
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
// The movement collider (the solid body walls/obstacles/other tanks actually
// block against - `Physics::spawn_tank`/`resize_collider`) is deliberately
// *smaller* than the hull damage box above: the classic arcade split where
// the damage box matches the visible silhouette (hits feel fair) while the
// movement box is forgiving (tanks slide through gaps and past each other
// instead of snagging pixel-perfectly). Each half-extent from
// TANK_HULL_BBOX_BY_ROW is scaled by this before reaching the physics body -
// see `Tank::move_half_extents`. ~10% is invisible in play (sprites don't
// visibly interpenetrate); past ~25% tanks start reading as clipping
// *into* walls, so tune in small steps (the "I"-key inspect overlay draws
// both boxes for exactly this).
pub const TANK_MOVE_BBOX_FRACTION: f32 = 0.9;
// Corner rounding (world px) of the movement collider: the collider is a
// rapier round-cuboid (a box dilated by this radius - `Physics` shrinks the
// core box by the same amount so the overall footprint stays exactly
// `Tank::move_half_extents`), not a sharp cuboid. Sharp box-vs-box corners
// catch on the seams between adjacent wall-cell colliders (the classic
// internal-edge artifact - tanks visibly snagged mid-slide along a flat
// wall run); a rounded corner slides past instead, the same reason
// character controllers are near-universally capsules. Clamped per tank so
// tiny hulls stay valid - see `physics::tank_corner_radius`.
pub const TANK_MOVE_CORNER_RADIUS: f32 = 4.0;
// Per-row turret+barrel footprint, as raw (x0,y0,x1,y1) inclusive pixel
// coordinates in the 32x32 cell (same "facing up" reference frame as
// TANK_HULL_BBOX_BY_ROW) - straight from the "Turret bbox" column of
// docs/SPRITESHEET_SPEC.md §9's measured table. Unlike the hull table, kept
// as raw corners rather than a (width, height) pair: the turret+barrel
// silhouette isn't centered on the tile the way the hull roughly is - the
// barrel extends it well past center toward the front - so `Tank::
// turret_bbox_world` needs both corners to reconstruct that off-center
// footprint.
//
// **Not used for movement collision** - a tank's solid hull collider
// (`Physics::spawn_tank`/`resize_collider`) stays hull-only, so the barrel
// still isn't a physical obstacle other tanks/shells can ram or get blocked
// by. It *is* used for shell/laser hit-testing though: `Tank::
// turret_hit_sensor` (a second sensor collider, sized/positioned from
// `Tank::turret_bbox_world`) and `simulation::swept_shell_target`'s
// hand-rolled fallback both check this box alongside the hull box, so a
// shot landing on the visible barrel registers as a hit - overriding
// docs/SPRITESHEET_SPEC.md §9's original "exclude the barrel from
// collision" note after visually confirming (via `game.rs`'s "I"-key debug
// inspect overlay, which draws this exact box) that it tracks the art
// closely enough to be worth it.
pub const TANK_TURRET_BBOX_BY_ROW: [(f32, f32, f32, f32); 12] = [
    (11.0, 2.0, 20.0, 20.0), // scout
    (10.0, 3.0, 21.0, 21.0), // assault
    (10.0, 4.0, 21.0, 21.0), // breaker
    (11.0, 0.0, 20.0, 20.0), // longbow
    (10.0, 6.0, 21.0, 21.0), // flak
    (10.0, 3.0, 21.0, 20.0), // wraith
    (10.0, 2.0, 21.0, 21.0), // warden
    (10.0, 2.0, 21.0, 21.0), // ravager
    (11.0, 2.0, 20.0, 21.0), // glacier
    (10.0, 0.0, 21.0, 21.0), // obelisk
    (7.0, 0.0, 24.0, 23.0),  // titan (super-heavy)
    (8.0, 0.0, 23.0, 23.0),  // leviathan (super-heavy)
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
// Sideways (tile px, pre-scale) distance from the tank's center to each
// barrel, for the five twin-barrel chassis (rows 1, 4, 7, 9, 10 - assault,
// flak, ravager, obelisk, titan); zero for every single-barrel row, which
// fires from dead center. Symmetrized from docs/SHELLS_SPEC.md §"Projectile
// alignment"'s published tank-barrel column ranges (standard twin: x12-13 and
// x18-19 around a tile center of x16, i.e. -3.5/+2.5 - averaged to a clean
// ±3.0; super twin/`titan`: x9-12 and x19-22, i.e. -5.5/+4.5 - averaged to
// ±5.0) rather than kept asymmetric, since two independently-simulated
// shells read better evenly spaced than reproducing the art's minor
// asymmetry exactly. See `Shell::spawn`'s `lateral_offset` param and
// `Game::update`'s twin-barrel fire handling - a positive offset is the
// right-hand barrel (screen +x at rotation 0/facing up), negative is left.
pub const TANK_BARREL_LATERAL_OFFSET_BY_ROW: [f32; 12] = [
    0.0, 3.0, 0.0, 0.0, 3.0, 0.0, 0.0, 3.0, 0.0, 3.0, 5.0, 0.0,
];
// How long after a twin-barrel chassis's first shell the second one fires
// (see `Tank::pending_shot`) - long enough to read as two distinct shots,
// short enough that it's still clearly one trigger-pull. Comfortably under
// both PLAYER_FIRE_INTERVAL (0.15s) and the enemy AI's fastest fire_interval
// (ENEMY_FIRE_INTERVAL_AGGRESSIVE, 0.7s), so the pending second shell always
// resolves well before that same tank could legally fire again - nothing
// handles a fresh trigger-pull arriving while one is still pending.
pub const TANK_TWIN_SHOT_DELAY_SECONDS: f32 = 0.05;
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
// The twin/staggered rows (3-5, 9-17) depict one shell sprite showing both
// barrels discharging at once (or staggered a beat apart) and were designed
// for a twin-barrel chassis firing a single `Shell` entity. Since twin-barrel
// chassis now fire two independent, genuinely separate shells instead (see
// TANK_BARREL_LATERAL_OFFSET_BY_ROW/TANK_TWIN_SHOT_DELAY_SECONDS and
// `Game::update`'s fire handling), each of those two shells is just a normal
// single-barrel shot in its own right - so no chassis selects the twin/
// staggered rows anymore. They're still valid, still-loaded art, just
// currently unreferenced by any `TANK_SHELL_VARIANT_BY_ROW` entry; kept in
// the sheet rather than pruned in case a future single-`Shell`-entity use
// case wants them back.
pub const SHELL_TEXTURE_SIZE: f32 = 32.0;
// Row count in shells.png (see the grouping above).
pub const SHELL_VARIANTS: i32 = 18;
// Shell row matched to each tank chassis row's size class (standard vs the
// two super-heavy titan/leviathan chassis) and accent colour - see the
// class_base/colour grouping in the atlas comment above. Always a *single*-
// barrel row now, even for the five twin-barrel chassis (1, 4, 7, 9, 10):
// each barrel fires its own independent shell (see
// TANK_BARREL_LATERAL_OFFSET_BY_ROW), so every individual shell is a normal
// single-barrel shot regardless of which chassis fired it. Longbow (green
// accent) and wraith (purple accent) have no matching shell colour family
// and fall back to blue. Set once at `Tank::shell_variant` on spawn
// (simulation.rs) and fixed for that tank's whole life - unlike before this
// no longer needs to change shot to shot, since there's no more twin/
// staggered art alternation to toggle between.
pub const TANK_SHELL_VARIANT_BY_ROW: [i32; 12] = [
    2, // 0  scout      standard single, blue (cyan accent)
    0, // 1  assault    standard single, orange (amber accent) - twin chassis, single-shell art (see above)
    1, // 2  breaker    standard single, red
    2, // 3  longbow    standard single, blue (no green family)
    2, // 4  flak       standard single, blue (cyan accent) - twin chassis, single-shell art (see above)
    2, // 5  wraith     standard single, blue (no purple family)
    2, // 6  warden     standard single, blue (cyan accent)
    0, // 7  ravager    standard single, orange (amber accent) - twin chassis, single-shell art (see above)
    2, // 8  glacier    standard single, blue
    1, // 9  obelisk    standard single, red - twin chassis, single-shell art (see above)
    7, // 10 titan      super single,    red - twin chassis, single-shell art (see above)
    8, // 11 leviathan  super single,    blue (cyan accent)
];
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

// Fraction of enemies that start each round already carrying a special
// weapon (laser, plasma, or minigun, see ENEMY_SPECIAL_WEAPON_LASER_SHARE/
// ENEMY_SPECIAL_WEAPON_PLASMA_SHARE) loaded with a full pickup's worth of
// ammo, rather than the shell-only default every tank otherwise spawns with
// (see Game::init's enemy spawn loop). Rolled independently per enemy, so
// this is an expected fraction across a round, not an exact headcount.
pub const ENEMY_SPECIAL_WEAPON_CHANCE: f32 = 0.4;
// Of an enemy that rolls a special weapon, the odds it's a laser - unchanged
// from before plasma existed, so laser's own odds aren't disturbed by this
// three-way split.
pub const ENEMY_SPECIAL_WEAPON_LASER_SHARE: f32 = 0.5;
// Of the remaining (non-laser) share, the odds it's plasma rather than a
// minigun - the remainder gets a minigun instead. 0.5 means the two split
// the non-laser half evenly (laser 50%, plasma 25%, minigun 25% overall).
pub const ENEMY_SPECIAL_WEAPON_PLASMA_SHARE: f32 = 0.5;

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
pub const MAX_SHELLS: i32 = 10;
pub const SHELL_RECHARGE_SECONDS: f32 = 2.0;

// Player fire rate: minimum seconds between consecutive player shots. Unlike
// the AI (ENEMY_FIRE_INTERVAL/_AGGRESSIVE, gated by Ai's own fire_timer), the
// player's fire was previously gated only by shells_ammo - holding/tapping
// fire could dump the whole MAX_SHELLS magazine in a few frames. This is the
// player-side equivalent of that cooldown (Tank::fire_cooldown).
pub const PLAYER_FIRE_INTERVAL: f32 = 0.15;

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

// Firing recoil: a small backward impulse on the shooter along the shell's
// own travel axis (so a misfire's aim skew skews the kick too), applied in
// `fire_shell`. Mass-normalized the same way as ram/explosion knockback
// above (reference mass = chassis-free scale^2, divided by the shooter's
// real `Tank::mass`) so a heavier chassis visibly recoils less per shot than
// a lighter one - "feels heavier" without needing its own per-chassis table.
// Deliberately much smaller than KNOCKBACK_MAX_SPEED: this is felt, not a
// real shove.
pub const SHELL_RECOIL_SPEED: f32 = 18.0;
pub const SHELL_RECOIL_MAX_SPEED: f32 = 40.0;

// Ricochet: a shell reflects off an indestructible Iron obstacle instead of
// detonating, up to this many times per shell (see `Shell::bounces_left`) -
// every other target (a tank, the frog, the battlefield's outer boundary
// wall, any destructible wall material) still detonates on first contact
// regardless. The outer boundary is deliberately excluded even though it's
// also an indestructible wall: bouncing off the edge of the arena reads as
// "the game rejected my shot," not as a mechanic to play around, so it's
// just a hit. One bounce keeps the Iron case readable: a shot that grazes
// a pillar gets one more chance to land, not an indefinitely ping-ponging
// shell.
pub const SHELL_RICOCHET_BOUNCES: u32 = 1;

// Enemy AI tuning. Distances are in pixels, times in seconds.
// 800 (was 520) - at the old value enemies never noticed the player past
// roughly half the default 1280x720 window, reading as passive/oblivious
// (user feedback, 2026-08). See also ENEMY_ALERT_HOLD_SECONDS below, which
// extends awareness beyond even this range once any one enemy has spotted
// the player.
pub const ENEMY_VIEW_RANGE: f32 = 800.0; // start chasing the player within this
pub const ENEMY_ATTACK_RANGE: f32 = 340.0; // stop and fight within this
pub const ENEMY_FIRE_ALIGN_PX: f32 = 24.0; // fire when player is within this of the axis
pub const ENEMY_FIRE_INTERVAL: f32 = 1.2; // min seconds between AI shots
pub const ENEMY_AIM_SETTLE: f32 = 0.25; // must be lined up this long before firing
pub const ENEMY_DAMAGE_MIN: f32 = 5.0; // enemy shell damage lower bound (weaker)
pub const ENEMY_DAMAGE_MAX: f32 = 15.0; // enemy shell damage upper bound
pub const ENEMY_FLEE_DAMAGE: f32 = 70.0; // retreat once this hurt
pub const ENEMY_RETARGET_SECONDS: f32 = 3.0; // how often patrol picks a new point
// How many candidate waypoints `Ai::wander` rolls per resample, keeping
// whichever is both reachable and farthest from every other live tank,
// rather than committing to the first random point - see that function's
// own doc comment for why plain uniform sampling wasn't enough (a large
// fraction of a typical battlefield isn't pathfinding-reachable from any
// given spot, mainly the fortress, so independent wandering enemies kept
// landing in the same small accessible pocket with no awareness of each
// other). Small enough to stay cheap (each candidate costs one grid
// pathfind check), big enough to reliably find some spread even when the
// reachable fraction of the map is small.
pub const WANDER_SPREAD_CANDIDATES: u32 = 6;

// Engagement spacing: `act_chase` and `act_attack`'s reposition branch used
// to steer every enemy that's chasing or repositioning at the player's exact
// position, so any group of enemies converging on the player independently
// picked the same destination and piled up on top of each other/each other's
// pathfinding routes - the actual cause of tank "clustering", not a
// pathfinding bug. `simulation.rs::Game::update` now claims each engaged
// enemy a distinct slot out of 4 cardinal axes (N/E/S/W through the player,
// since `act_attack` can only ever fire from exactly on-axis -
// ENEMY_FIRE_ALIGN_PX - so an off-axis point would strand a tank that could
// never turn it into a firing solution) x 2 lateral firing positions x a
// reserve rank (see ENGAGE_LATERAL_OFFSET/ENGAGE_RESERVE_RADIUS below), with
// real per-frame mutual exclusion so two tanks can never both resolve to the
// same point - the same "known positions of teammates -> spread out" idea
// `wander`'s WANDER_SPREAD_CANDIDATES already uses for patrol, just claimed
// once and held steady (see `engage_slot_choice`) rather than resampled
// every frame. Comfortably inside ENEMY_ATTACK_RANGE so a firing-line enemy
// still ends up close enough to fight rather than orbiting just outside
// range.
pub const ENGAGE_RING_RADIUS: f32 = ENEMY_ATTACK_RANGE * 0.8;

// Lateral offset (px, perpendicular to the axis) between the two rank-0
// firing slots on the same cardinal axis. Kept under ENEMY_FIRE_ALIGN_PX
// (24) so *both* slots stay inside the fire-alignment band - a tank in
// either one can still get a shot off - while still separating the pair by
// double this (36px), comfortably clear of every hull width
// (TANK_HULL_BBOX_BY_ROW) and of ENEMY_FIRE_ALIGN_PX itself, so
// `friendly_blocks_shot` no longer treats a paired teammate as blocking the
// shot the way same-axis depth-stacking used to.
pub const ENGAGE_LATERAL_OFFSET: f32 = 18.0;

// Forward distance (px) of the reserve rank (rank 1) an axis's 3rd/4th
// engaged tank claims once both rank-0 firing slots on every reachable axis
// are taken. Deliberately past ENEMY_ATTACK_RANGE so a reserve tank neither
// attempts to fire nor sits in a firing lane, and far enough behind
// ENGAGE_RING_RADIUS (128px) to stay outside the probe's CLUSTER_RADIUS,
// while still comfortably inside ENEMY_VIEW_RANGE so Chase keeps steering
// it there instead of losing track of the fight.
pub const ENGAGE_RESERVE_RADIUS: f32 = ENEMY_ATTACK_RANGE + 60.0;

// The shortest forward distance a rank-0 firing slot may be clamped down to
// when a near-wall player would otherwise push the full ENGAGE_RING_RADIUS
// point off the battlefield (see `simulation.rs`'s `engage_point` closure) -
// below this the slot is invalid rather than clamped further. Set just past
// ENEMY_MISFIRE_RANGE so a clamped slot never lands inside the forced-
// misfire zone.
pub const ENGAGE_MIN_RADIUS: f32 = 190.0;

// Shared aggression: once any enemy has the player within ENEMY_VIEW_RANGE,
// every enemy on the field treats the player's current position as a shared
// "last known sighting" and converges on it (see Ai::think's `alert`
// parameter, ai.rs's act_patrol) for this many seconds after the last actual
// sighting, instead of each enemy only reacting within its own individual
// view range. Refreshed continuously while the sighting holds, so this is
// purely "how long the whole map stays alerted after it loses the player
// again," not a one-shot ping.
pub const ENEMY_ALERT_HOLD_SECONDS: f32 = 6.0;

// Retaliation: getting shot is itself a reason to fight back, independent of
// ENEMY_VIEW_RANGE/the shared alert above - both of those gate on the enemy
// having actually *seen* the player, so a shot landing from outside view
// range (or before any enemy has spotted the player at all) previously did
// nothing but chip health; the hit tank just kept patrolling/wandering as if
// nothing happened. `Ai::notify_hit` (called from `Game::update`'s shell-hit
// resolution whenever a shell damages a still-alive enemy) sets this many
// seconds on that one tank's own `hit_alert_timer`, which `ai.rs`'s Chase
// condition treats as equivalent to "player in view range" - so a hit enemy
// immediately starts closing in and fighting back (or fleeing/retreating
// first, if ENEMY_FLEE_DAMAGE/ammo-low already applies - those checks sit
// above Chase in the behavior tree's priority order and are unconditional on
// visibility already). Deliberately per-tank, not broadcast to the whole map
// like ENEMY_ALERT_HOLD_SECONDS - shooting one enemy makes *it* fight back,
// not summon the whole field.
pub const ENEMY_HIT_ALERT_SECONDS: f32 = 6.0;

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
pub const ENEMY_FIRE_INTERVAL_AGGRESSIVE: f32 = 0.7;

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

// Camera shake: a short, decaying screen-space wobble on the exact same
// "kill shockwave" trigger `Game::shock` already drives (see `Shockwave`
// above and `Game::render`'s blit of `scene_target`) - no separate trigger
// plumbing needed. Deliberately much shorter than SHOCKWAVE_DURATION so it
// reads as one punchy hit rather than a full second of wobble.
pub const CAMERA_SHAKE_DURATION: f32 = 0.3; // seconds the shake lasts
pub const CAMERA_SHAKE_MAGNITUDE: f32 = 10.0; // px offset at full strength
pub const CAMERA_SHAKE_FREQUENCY: f32 = 40.0; // radians/sec of the underlying wobble

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
// Obstacle positions land on a world-space grid this many px per cell (see
// `map::cell_to_world`) so every wall tile a map places lands visually
// aligned instead of at arbitrary fractional offsets. Exactly
// `Obstacle::size()` (OBSTACLE_TEXTURE_SIZE * OBSTACLE_SCALE) - one grid
// cell is one obstacle tile, so a grid-aligned obstacle's sprite exactly
// fills its cell with no gap or overlap.
pub const OBSTACLE_GRID_SIZE: f32 = OBSTACLE_TEXTURE_SIZE * OBSTACLE_SCALE;
// Minimum clearance (px) an obstacle spawn must keep from the player's
// start position and every enemy's start position - same idea as the
// enemy_clear/clear checks in Game::init, just reused for a third entity
// type.
pub const OBSTACLE_CLEAR: f32 = 90.0;

// Ground/terrain layer (grass base, road painted under every static
// obstacle tile and every cell a map explicitly marks as road) - see
// ground.rs for the placement/autotile logic, docs/GROUND_SPEC.md for the
// full design writeup. Drawn from
// static/punyworld/punyworld-overworld-tileset.png, a third-party tileset -
// deliberately NOT on the Resurrect 64 palette every other sheet uses (see
// docs/PALETTE.md and static/punyworld/SOURCE.md), a documented exception
// rather than an oversight.
pub const GROUND_TEXTURE_SIZE: f32 = 16.0; // native tile size in the source sheet
// 2x, not OBSTACLE_SCALE-style 1.0 - the source art is 16px/tile, and
// GROUND_WORLD_TILE below needs to land on OBSTACLE_GRID_SIZE (32px) so the
// ground layer's grid lines up with the obstacle/physics grid (see
// ground.rs's module doc comment for the centered-vs-top-left-aligned
// phase-matching that actually makes this line-up hold in practice).
pub const GROUND_SCALE: f32 = 2.0;
pub const GROUND_WORLD_TILE: f32 = GROUND_TEXTURE_SIZE * GROUND_SCALE; // = OBSTACLE_GRID_SIZE

pub const OBSTACLE_SHADOW_OFFSET: f32 = 3.0; // px, same grounded distance as TANK_SHADOW_OFFSET
pub const OBSTACLE_SHADOW_OPACITY: f32 = 0.35;

// HUD: the SHELLS/HP numbers (Game::render) shift color as that resource
// drops, as a fraction of its max (MAX_SHELLS / MAX_DAMAGE respectively) -
// default gray above HUD_WARN_THRESHOLD, orange between the two, red below
// HUD_CRITICAL_THRESHOLD. Conservative on purpose: only flag real trouble,
// not every routine dip.
pub const HUD_WARN_THRESHOLD: f32 = 0.40;
pub const HUD_CRITICAL_THRESHOLD: f32 = 0.15;

// HUD font sizes (Game::render): the SHELLS/HP line reads as the "primary"
// readout so it's drawn 10% bigger than the base 24px; the version/build
// line is secondary, drawn 10% smaller.
pub const HUD_FONT_SIZE: i32 = 26; // 24 * 1.1, rounded
pub const HUD_VERSION_FONT_SIZE: i32 = 22; // 24 * 0.9, rounded

// Shared screen-edge inset for both HUD corners: the SHELLS/HP line's
// top-left origin and the version line's bottom-right origin, so the two
// sit the same distance from their respective edges.
pub const HUD_MARGIN: i32 = 20;

// health_bar.png is a hand-authored (not tools/spritegen-generated) 96x64
// sheet: a 3x2 grid of 32x32 cells, five used left-to-right/top-to-bottom
// (the sixth, bottom-right cell is unused/fully transparent). Each cell
// holds one small heart+4-pip icon at a fixed offset, depleting one pip per
// cell: index 0 = 4/4 pips (full) through index 4 = 0/4 pips (empty). Colors
// were remapped from the source PNG's supplied saturated red onto
// punypalette's RED_DK to match the rest of the game's palette (see
// docs/PALETTE.md) - see static/_original/health_bar.png for the pristine
// pre-recolor copy.
pub const HEALTH_BAR_CELL_SIZE: f32 = 32.0;
pub const HEALTH_BAR_VARIANTS: i32 = 5;
// The icon within each 32x32 cell doesn't fill it - it's a tight 22x7 glyph
// at this offset, so drawing crops to just the glyph rather than the cell.
pub const HEALTH_BAR_ICON_OFFSET: (f32, f32) = (5.0, 7.0);
pub const HEALTH_BAR_ICON_SIZE: (f32, f32) = (22.0, 7.0);
// Columns per row in the sheet (see the layout comment above) - used to turn
// a linear frame index into a (col, row) cell position.
pub const HEALTH_BAR_COLUMNS: i32 = 3;
// On-screen scale for the HUD readout - deliberately matches Tank::scale
// (2.0) so the health bar's pixels read at the same on-screen size as every
// other sprite (tanks, walls_sheet.png's PIXELATE_FACTOR) rather than
// looking chunkier or finer than the rest of the game.
pub const HEALTH_BAR_HUD_SCALE: f32 = 2.0;

// Overhead health bar (Game::render, drawn under a tank rather than in the
// HUD corner): shown for HEALTH_BAR_OVERHEAD_SECONDS after `Tank::mark_hit`
// fires, so a tank that's just been shot/rammed/caught in a blast briefly
// reads its HP at a glance without needing the player's own HUD line.
// Matches ENEMY_RETARGET_SECONDS's "a few seconds" ballpark rather than a
// fresh guess. Fades out (alpha ramp) over the trailing
// HEALTH_BAR_OVERHEAD_FADE_SECONDS instead of popping off abruptly.
pub const HEALTH_BAR_OVERHEAD_SECONDS: f32 = 3.0;
pub const HEALTH_BAR_OVERHEAD_FADE_SECONDS: f32 = 0.6;
// Gap in px between a tank's sprite bottom edge and the bar drawn under it.
pub const HEALTH_BAR_OVERHEAD_GAP: f32 = 4.0;

// ToxicFrog (src/frog.rs): the player's protect-objective - a static NPC
// that ends the round in a loss the instant its health reaches zero, same
// severity as the player's own tank being destroyed. See
// static/toxic_frog/SOURCE.md for the sprite's provenance and
// docs/FROG_SPEC.md for the full frame/animation layout - each of the five
// animation PNGs is a plain 48x48-cell filmstrip, no slicing math beyond
// `col * FROG_TEXTURE_SIZE`.
pub const FROG_TEXTURE_SIZE: f32 = 48.0;
// On-screen scale - deliberately matches Tank::scale/HEALTH_BAR_HUD_SCALE
// (2.0) for the same reason both of those do: consistent on-screen pixel
// density across every sprite in the game. The frog's actual content is a
// small glyph within the 48x48 cell (see docs/FROG_SPEC.md), so this reads
// as a modest, tank-sized presence on the field, not an oversized 96px prop.
pub const FROG_SCALE: f32 = 2.0;
pub const FROG_IDLE_FRAMES: i32 = 8;
pub const FROG_HURT_FRAMES: i32 = 4;
pub const FROG_HOP_FRAMES: i32 = 7;
pub const FROG_ATTACK_FRAMES: i32 = 6;
pub const FROG_EXPLOSION_FRAMES: i32 = 9;
pub const FROG_IDLE_FPS: f32 = 6.0;
pub const FROG_HURT_FPS: f32 = 10.0;
pub const FROG_HOP_FPS: f32 = 10.0;
pub const FROG_ATTACK_FPS: f32 = 10.0;
pub const FROG_EXPLOSION_FPS: f32 = 10.0;
// How long each one-shot clip plays before `Frog::anim` falls back to a
// lower-priority state - exactly FRAMES/FPS (one full pass); kept as their
// own constants since `Frog::anim` needs a value, not an expression, to
// compute "seconds elapsed since the trigger" for frame timing.
pub const FROG_HURT_SECONDS: f32 = 0.4;
pub const FROG_HOP_SECONDS: f32 = 0.7;
pub const FROG_ATTACK_SECONDS: f32 = 0.6;
// Health: deliberately much lower than MAX_DAMAGE - a couple of hits end
// the round, so "protect the frog" is a real constraint on where the player
// fights, not a background stat that never comes into play.
pub const FROG_MAX_HEALTH: f32 = 40.0;
// Physics collider half-extents (px) - sized to the sprite's actual visible
// footprint (see docs/FROG_SPEC.md's per-frame bbox measurements), not the
// full padded 48x48 cell. Same `Physics::spawn_static` fixed-body shape as
// an Obstacle: blocks tank movement and doubles as the shell-hit target,
// no separate sensor collider needed.
pub const FROG_COLLIDER_HALF_EXTENT: (f32, f32) = (22.0, 16.0);
// Spawn placement (Game::init): kept near the player's fortress rather than
// anywhere on the map, so there's actually a fighting chance to defend it -
// far enough not to spawn inside the fortress/on top of the player, close
// enough that "protect the frog" and "protect yourself" are the same fight
// early on rather than two unrelated ones.
pub const FROG_SPAWN_MIN_DIST: f32 = 90.0;
pub const FROG_SPAWN_MAX_DIST: f32 = 240.0;

// Evasion: hop away from incoming fire. Both this and the attack range
// below are expressed as a factor of `Frog::size()` (the on-screen sprite
// footprint, FROG_TEXTURE_SIZE * FROG_SCALE) rather than an independent
// pixel constant, so a hit/attack "reach" that reads as fair right now
// keeps reading as fair if the frog's own on-screen size is ever retuned -
// per-instance data, not a fixed magic number unrelated to what's actually
// on screen. 0.75 (was 1.5, itself halved from an original 3.0 - a full 3x
// its own size, read as an unnaturally huge leap for a creature this small;
// halved again so a hop now covers roughly three-quarters of a body-length
// instead of one and a half - user feedback, 2026-08).
pub const FROG_HOP_DISTANCE_FACTOR: f32 = 0.75;
// Debounce so a rapid volley of hits doesn't trigger a hop every single
// frame one lands - roughly one hop per FROG_HOP_COOLDOWN_SECONDS even
// under sustained fire.
pub const FROG_HOP_COOLDOWN_SECONDS: f32 = 1.0;
// `simulation::frog_hop_target`'s search: tries the ideal dead-away-from-
// the-shot angle first (plus a little random jitter so hops don't all look
// mechanically identical), then this fan of offsets from it, so a frog
// backed into a corner/wall still has a shot at finding *some* clear
// landing spot rather than never hopping at all near terrain.
pub const FROG_HOP_ANGLE_JITTER_DEG: f32 = 10.0;
pub const FROG_HOP_ANGLE_FAN_DEG: [f32; 5] = [0.0, 25.0, -25.0, 50.0, -50.0];
// Landing spots must stay this far inside the battlefield edge - same idea
// as the enemy-spawn/obstacle-placement margins in battlefield.rs, just a
// flat constant here since a hop's landing zone is small and local rather
// than needing a fraction-of-board-size margin.
pub const FROG_HOP_BOUNDS_MARGIN: f32 = 40.0;

// Retaliation: bite any tank - either side - that gets too close. See
// FROG_HOP_DISTANCE_FACTOR's comment above for why this is a size factor
// rather than a flat pixel constant.
pub const FROG_ATTACK_RANGE_FACTOR: f32 = 0.9;
pub const FROG_ATTACK_COOLDOWN_SECONDS: f32 = 1.5;
pub const FROG_ATTACK_DAMAGE_MIN: f32 = 4.0;
pub const FROG_ATTACK_DAMAGE_MAX: f32 = 10.0;

// Personal space: hop away from the nearest tank - either side, same "no
// favorites" symmetry as the bite above - once one gets within this range,
// independent of (and deliberately larger than) FROG_ATTACK_RANGE_FACTOR,
// so this is "keep your distance" rather than "retaliate": the two reflexes
// aren't mutually exclusive, a tank that closes the gap faster than
// FROG_HOP_COOLDOWN_SECONDS clears just eats a bite before the frog gets a
// chance to react. Bigger than FROG_ATTACK_RANGE_FACTOR but well inside
// FROG_HOP_DISTANCE_FACTOR (a single hop reliably clears it, rather than
// the frog needing several to actually gain distance) - scaled down to 1.2
// alongside FROG_HOP_DISTANCE_FACTOR's own reduction (was 2.0/3.0) to keep
// that same margin between the two.
pub const FROG_AVOID_RANGE_FACTOR: f32 = 1.2;

// Health/ammo pickups (pickup.rs): spawn only at the map's own `Pickup`
// cells (see `map::CellObject::Pickup`, `battlefield::spawn_from_map`) - the
// map's placed cells are the pickup *slots*, respawning at a random
// currently-empty slot after a delay once collected. A map with no pickup
// cells simply has no pickups; there's no random-placement fallback. Sprite
// is 32x32 (see static/pickups/SOURCE.md) drawn 1:1, same convention as
// obstacles (OBSTACLE_SCALE = 1.0) rather than the tanks' chunky 2x - a
// pickup icon reads fine at native res and doesn't need to match the tanks'
// pixelated look the way terrain does.
pub const PICKUP_TEXTURE_SIZE: f32 = 32.0;
pub const PICKUP_SCALE: f32 = 1.0;
// Seconds after a pickup is collected before a fresh one spawns at a random
// empty slot (see `simulation::respawn_from_slots`) - keeps the field topped
// up to however many slots the map placed, rather than depleting it.
pub const PICKUP_RESPAWN_SECONDS: f32 = 15.0;
// How close a tank's center needs to get to collect a pickup - a bit more
// forgiving than requiring true hull overlap, so it doesn't feel like it
// needs pixel-perfect contact.
pub const PICKUP_COLLECT_RADIUS: f32 = 32.0;
// Health pickup: deliberately not a full heal (MAX_DAMAGE=100) - a
// meaningful chunk (roughly 2-3 enemy hits' worth, see ENEMY_DAMAGE_MIN/MAX)
// worth detouring for, not an automatic full reset.
pub const PICKUP_HEAL_AMOUNT: f32 = 30.0;
// Ammo pickup: how many shells it adds. Uncapped - separate from MAX_SHELLS
// (10), which stays exactly what it was: the passive-recharge target (see
// Tank::tick_recharge, untouched by this feature). A pickup is the only way
// past 10, and stacking pickups can push a magazine arbitrarily high.
pub const PICKUP_AMMO_AMOUNT: i32 = 4;

// --- Laser pickup/weapon (pickup.rs's PickupKind::Laser, laser.rs,
// simulation.rs's fire_laser/resolve_laser_hit) ---
// A limited-charge, instant-hit weapon: while Tank::laser_charges > 0,
// firing resolves a hit the same frame (no travel time, unlike Shell) and
// consumes one charge; at zero the tank reverts to its normal shell. Same
// pickup mechanics as health/ammo (collect on touch, respawn from map
// slots) - see PickupKind::Laser's own handling in
// `Game::update`'s pickup-collection section.
pub const LASER_CHARGES_PER_PICKUP: i32 = 6;
// Lower than PLAYER_DAMAGE_MIN/MAX (10..30) - a laser never misses (no
// travel time to dodge), so its per-hit damage is toned down to compensate
// for that guaranteed accuracy rather than stacking a shell's damage on top
// of it.
pub const LASER_DAMAGE_MIN: f32 = 8.0;
pub const LASER_DAMAGE_MAX: f32 = 14.0;
// Two variants (see `laser::LaserVariant`), rolled once per pickup rather
// than per shot - LASER_BLUE_PICKUP_CHANCE is Blue's odds (so Red is the
// remaining 60%), and a Blue charge batch fires at LASER_DAMAGE_MIN/MAX
// scaled by this factor instead of the Red baseline.
pub const LASER_BLUE_PICKUP_CHANCE: f32 = 0.4;
pub const LASER_BLUE_DAMAGE_FACTOR: f32 = 1.2;
// How long a fired beam stays on screen before fading out (see
// `laser::LaserBeam`) - purely cosmetic, long enough to read as a flash,
// short enough not to look like it lingers.
pub const LASER_BEAM_DISPLAY_SECONDS: f32 = 0.12;
// Line thickness (px) `laser::draw_laser_beam` draws its glow pass at - the
// core pass draws thinner, at a fixed fraction of this.
pub const LASER_BEAM_WIDTH: f32 = 4.0;

// --- Minigun pickup/weapon (pickup.rs's PickupKind::Minigun, bullet.rs,
// simulation.rs's fire_bullet/MinigunBurst handling) ---
// Sits in weapon priority between the laser (used/depleted first) and the
// tank's traditional shell (used last) - see Tank::active_weapon. A burst
// fires MINIGUN_BURST_SIZE individually-simulated Bullet entities, not one
// abstract "burst" object: the first immediately on the trigger frame, the
// rest queued MINIGUN_BULLET_DELAY_SECONDS apart (Tank::minigun_burst).
pub const MINIGUN_AMMO_PER_PICKUP: i32 = 40; // ~5 full bursts per pickup
pub const MINIGUN_BURST_SIZE: u32 = 8;
// In TANK_TWIN_SHOT_DELAY_SECONDS's own 0.05s neighborhood, just a touch
// tighter so the stutter reads faster/busier than a twin-cannon's one-beat
// second shot.
pub const MINIGUN_BULLET_DELAY_SECONDS: f32 = 0.04;
// Extra gap held after a burst's last bullet before Tank::fire_cooldown
// clears, so a fresh trigger pulse can't stack a new burst on an unfinished
// one (same mechanism/field pending_shot's own TANK_TWIN_SHOT_DELAY_SECONDS
// already relies on, just generalized to a much longer queue).
pub const MINIGUN_BURST_TRAILING_GAP_SECONDS: f32 = 0.1;
pub const MINIGUN_BURST_COOLDOWN_SECONDS: f32 = (MINIGUN_BURST_SIZE - 1) as f32
    * MINIGUN_BULLET_DELAY_SECONDS
    + MINIGUN_BURST_TRAILING_GAP_SECONDS;
// Each bullet's direction is jittered by up to this many degrees off the aim
// line, independent of (and stacked on top of) the same point-blank misfire
// skew a shell/laser can already roll - the minigun's own "spray" identity,
// distinct from the laser's guaranteed hit and a shell's clean-unless-
// misfired aim.
pub const MINIGUN_BULLET_SPREAD_DEG: f32 = 4.0;

pub const MINIGUN_BULLET_TEXTURE_SIZE: f32 = 32.0;
pub const MINIGUN_BULLET_SCALE: f32 = 2.0; // matches SHELL_SCALE - same on-screen chunkiness
pub const MINIGUN_BULLET_SPEED: f32 = 900.0; // faster than SHELL_SPEED (500) - a zippy tracer, not a lobbed shell
pub const MINIGUN_BULLET_HIT_HALF_EXTENT: f32 = 2.0; // smaller than SHELL_HIT_HALF_EXTENT (3.0) - a lighter caliber
// Deliberately well below LASER_DAMAGE_MIN/MAX (8..14) and PLAYER_DAMAGE_
// MIN/MAX (10..30) per bullet - a single round is a non-event. A fully-
// landed burst (8 * ~3.0 avg = ~24) lands near one solid shell hit or a bit
// above one laser hit, rewarding sustained accuracy across a whole burst
// rather than any single round mattering. Applied the same way laser damage
// is - one shared range for player and enemy, scaled only by
// TANK_CHASSIS_DAMAGE_FACTOR_BY_ROW, not shell's PLAYER_/ENEMY_ split.
pub const MINIGUN_BULLET_DAMAGE_MIN: f32 = 2.0;
pub const MINIGUN_BULLET_DAMAGE_MAX: f32 = 4.0;
// Much lighter than SHELL_RECOIL_SPEED/_MAX (18.0/40.0) - a full burst
// should rattle the tank, not shove it once hard like a cannon shot.
pub const MINIGUN_BULLET_RECOIL_SPEED: f32 = 3.0;
pub const MINIGUN_BULLET_RECOIL_MAX_SPEED: f32 = 10.0;
// Tighter than SHELL_SHADOW_OFFSET_MIN/MAX (9..20) - a smaller, lower round.
pub const MINIGUN_BULLET_SHADOW_OFFSET_MIN: f32 = 5.0;
pub const MINIGUN_BULLET_SHADOW_OFFSET_MAX: f32 = 10.0;
pub const MINIGUN_BULLET_SHADOW_OPACITY: f32 = 0.30; // matches SHELL_SHADOW_OPACITY

// --- Minigun mount (visual only): tank.rs's draw_minigun_mount, the
// barrel-cluster overlay layered on the turret while minigun_ammo > 0 -
// tools/spritegen/gen_minigun_mount.py ---
pub const MINIGUN_MOUNT_TEXTURE_SIZE: f32 = 32.0;
// How long each of minigun_mount.png's 3 "hot barrel" frames is shown
// before advancing to the next, while a burst is active - see
// Tank::tick_minigun_spin/minigun_cycle_frame. Deliberately a discrete
// frame swap, not a continuous rotation: this game is top-down, and a real
// minigun's barrels point along the ground plane toward the target, so
// their rotation axis is edge-on to the camera, not face-on to it -
// spinning the sprite in the screen plane would read as a helicopter rotor
// seen from above (wrong axis for this camera angle), not a side-mounted
// minigun. Cycling which barrel reads as freshly-fired fakes "rounds
// cycling through firing position" correctly for this view instead. Tuned
// close to MINIGUN_BULLET_DELAY_SECONDS (0.04) so roughly one barrel-swap
// happens per bullet fired.
pub const MINIGUN_CYCLE_SECONDS: f32 = 0.05;
// Dest-rect scale for the one shared mount texture, layered on top of
// Tank::scale - deliberately the same on every chassis (not indexed by
// row): the minigun is a fixed piece of hardware, so it reads as one
// consistent size regardless of which tank it's bolted to, the same way
// its ammo count/damage don't scale with chassis either. Since Tank::scale
// itself is already a flat 2.0 for every chassis (the tank-to-tank size
// difference lives in the sprite art, not in `scale`), this constant alone
// is what to tune if the mount should read bigger/smaller overall.
pub const MINIGUN_MOUNT_SCALE: f32 = 1.0;

// --- Plasma pickup/weapon (pickup.rs's PickupKind::Plasma, plasma.rs,
// simulation.rs's fire_plasma/PendingPlasmaShot handling) ---
// Sits above the traditional shell in weapon priority but below the
// instant-hit laser (see Tank::active_weapon) - a straight damage upgrade
// over a shell rather than a different playstyle the way the laser
// (guaranteed hit) and minigun (spray) are. Fired the exact same way a
// shell is - straight down the barrel, a twin-barrel chassis firing one
// bolt per barrel a beat apart (see PendingPlasmaShot, mirroring
// Tank::pending_shot) rather than the minigun's rapid individually-queued
// burst - so it costs 2 ammo per twin-barrel shot exactly like a shell does.
pub const PLASMA_AMMO_PER_PICKUP: i32 = 10; // 10 single shots, or 5 twin-barrel volleys
// 24% stronger than a traditional shell - applied as a flat multiplier on
// top of PLAYER_DAMAGE_MIN/MAX or ENEMY_DAMAGE_MIN/MAX (same split a shell
// itself uses) and TANK_CHASSIS_DAMAGE_FACTOR_BY_ROW, not a separate damage
// range of its own - see the plasma hit-resolution block in
// simulation.rs's `update`.
pub const PLASMA_DAMAGE_FACTOR: f32 = 1.24;

pub const PLASMA_TEXTURE_SIZE: f32 = 32.0;
// Bigger than SHELL_SCALE (2.0) - a plasma bolt reads as visibly larger and
// heavier than a normal shell, matching its bigger damage per hit. Also
// scales the in-flight pulse glow (see `plasma::glow_pulse`'s `base_radius`),
// so this one constant sizes the whole effect. 2.08 = the original 2.6
// tuning, reduced 20% after it read too big on screen.
pub const PLASMA_SCALE: f32 = 2.08;
// 504 = the original 420 tuning, increased 20% for a punchier, faster-
// closing round - now a touch faster than SHELL_SPEED (500) rather than
// slower, on top of already hitting harder (PLASMA_DAMAGE_FACTOR).
pub const PLASMA_SPEED: f32 = 504.0;
// Bigger than SHELL_HIT_HALF_EXTENT (3.0) - a fatter bolt is easier to land
// a hit with, matching its bigger on-screen size.
pub const PLASMA_HIT_HALF_EXTENT: f32 = 5.0;
// A bit more than SHELL_RECOIL_SPEED/_MAX (18.0/40.0) - a heavier launch kick.
pub const PLASMA_RECOIL_SPEED: f32 = 22.0;
pub const PLASMA_RECOIL_MAX_SPEED: f32 = 45.0;
pub const PLASMA_SHADOW_OFFSET_MIN: f32 = 10.0; // px
pub const PLASMA_SHADOW_OFFSET_MAX: f32 = 22.0; // px
pub const PLASMA_SHADOW_OPACITY: f32 = 0.30; // matches SHELL_SHADOW_OPACITY
// More than SHELL_IMPACT_KNOCKBACK_SPEED (35.0) - a heavier "tap" on hit,
// matching the bolt's own bigger mass/damage.
pub const PLASMA_IMPACT_KNOCKBACK_SPEED: f32 = 45.0;
// Pulsating in-flight glow (see plasma::draw_plasma) - a sine wave drawn on
// top of the base sprite at runtime, the same "cheap per-frame animation"
// convention laser::draw_laser_beam's fade-out already uses.
pub const PLASMA_PULSE_HZ: f32 = 6.0; // pulses per second while flying
pub const PLASMA_PULSE_MIN_SCALE: f32 = 0.85; // glow radius at the pulse's low point (x sprite radius)
pub const PLASMA_PULSE_MAX_SCALE: f32 = 1.35; // glow radius at the pulse's high point
// The Flying state's own baked art also breathes now (4 keyframes -
// plasma::flying_col - instead of one static frame), layered underneath the
// runtime glow above rather than duplicating it: a subtle in/out pulse on
// the orb's core/rim, cycling through all 4 frames this many times per
// second. Deliberately not tied to PLASMA_PULSE_HZ - two independent cycles
// (a discrete 4-frame baked shimmer plus a continuous drawn halo) read as
// richer than the same single rate driving both.
pub const PLASMA_FLYING_CYCLE_FPS: f32 = 10.0;

// Two variants (see plasma::PlasmaVariant), rolled once per PickupKind::Plasma
// pickup rather than per shot - same mechanism as LASER_BLUE_PICKUP_CHANCE/
// LASER_BLUE_DAMAGE_FACTOR. PLASMA_PURPLE_PICKUP_CHANCE is Purple's odds (so
// Teal, the base variant, is the remaining 70%), and a Purple charge batch
// fires at PLASMA_DAMAGE_FACTOR scaled further by this factor instead of the
// Teal baseline.
pub const PLASMA_PURPLE_PICKUP_CHANCE: f32 = 0.3;
pub const PLASMA_PURPLE_DAMAGE_FACTOR: f32 = 1.10;

// --- Speed-up pickup (pickup.rs's PickupKind::SpeedUp, Tank::speed_boost_timer) ---
// A timed stat buff rather than a weapon/ammo pickup: collecting one sets
// (not adds to - see "only one at a time" below) `speed_boost_timer` to
// this many seconds, during which `Tank::effective_speed` is scaled by
// SPEED_BOOST_MULTIPLIER. Same pickup mechanics as every other kind (collect
// on touch, respawn from map slots) - see PickupKind::SpeedUp's handling in
// `Game::update`'s pickup-collection section.
pub const SPEED_BOOST_MULTIPLIER: f32 = 1.3; // 30% faster top speed while active
// Picking up a second one while already boosted refreshes this timer back
// to the full duration rather than adding to it or stacking the multiplier
// - a tank can only ever be under one speed boost at a time, never a
// double-strength one.
pub const SPEED_BOOST_DURATION_SECONDS: f32 = 12.0;

// --- Map editor (dev-only, docs/map-editor-design.md) ---
// Kept minimal - the editor is a `map-editor`-feature-only, presentation-only
// tool, not gameplay tuning, so most of its layout math lives directly in
// `editor.rs` rather than here. These are the few values worth naming since
// they're shared between the palette panel and the in-game hamburger toggle
// (main.rs).

/// Side length of one palette/toolbar icon button, in pixels.
pub const EDITOR_ICON_SIZE: f32 = 48.0;
/// Gap between adjacent icon buttons within a panel, in pixels.
pub const EDITOR_ICON_GAP: f32 = 8.0;
/// Padding between a panel's edge and the icons/controls inside it.
pub const EDITOR_PANEL_PADDING: f32 = 10.0;
/// Corner roundness passed to `draw_rectangle_rounded` (raylib's 0..1
/// fraction of the shorter side, not a pixel radius) - small on purpose, a
/// gentle curve rather than a pill shape.
pub const EDITOR_PANEL_ROUNDNESS: f32 = 0.12;
/// Segment count for the rounded-rect draw calls - raylib's usual default.
pub const EDITOR_PANEL_SEGMENTS: i32 = 8;
/// Panel drop-shadow offset, in pixels (down-right), and its opacity
/// fraction of solid black.
pub const EDITOR_PANEL_SHADOW_OFFSET: f32 = 4.0;
pub const EDITOR_PANEL_SHADOW_OPACITY: f32 = 0.35;
/// Panel border thickness, in pixels, and its opacity fraction of solid
/// black.
pub const EDITOR_PANEL_BORDER_THICKNESS: f32 = 1.5;
pub const EDITOR_PANEL_BORDER_OPACITY: f32 = 0.6;
/// Panel fill color (a dark near-opaque backing so icon sprites with
/// transparent/light edges stay legible over any battlefield tile behind
/// them) and its opacity.
pub const EDITOR_PANEL_FILL: (u8, u8, u8) = (20, 20, 24);
pub const EDITOR_PANEL_FILL_OPACITY: f32 = 0.85;
/// Fixed gap between the bottom-center object palette and the bottom of the
/// screen.
pub const EDITOR_PALETTE_BOTTOM_MARGIN: f32 = 16.0;
/// Fixed margin from the top-right corner for the Save/Load/Close toolbar,
/// and from the top-left corner for the hamburger toggle button.
pub const EDITOR_TOOLBAR_MARGIN: f32 = 16.0;
/// Side length of the top-left hamburger/back toggle button.
pub const EDITOR_HAMBURGER_SIZE: f32 = 40.0;

/// Parse a round-seed CLI value: plain decimal, or hex with a `0x`/`0X`
/// prefix - shared by both binaries' `--seed` flags (main.rs and
/// src/bin/probe.rs; a bin can't import from another bin, so this lives in
/// the library). Hex matters because that's the form seeds are *printed*
/// in (`seed=0x{:016x}` in the probe's ANOMALY/summary lines - see
/// docs/gameplay-verification-design.md §1.5), so a printed seed must
/// paste straight back into `--seed` without a manual base conversion.
pub fn parse_seed(s: &str) -> Result<u64, String> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16)
            .map_err(|e| format!("invalid seed '{s}': {e}")),
        None => s
            .parse()
            .map_err(|e| format!("invalid seed '{s}': {e}")),
    }
}

pub mod ai;
pub mod battlefield;
pub mod bt;
pub mod bullet;
pub mod damage_stage;
#[cfg(feature = "map-editor")]
pub mod editor;
pub mod frog;
pub mod game;
pub mod ground;
pub mod laser;
pub mod map;
pub mod obstacle;
pub mod pathfind;
pub mod physics;
pub mod pickup;
pub mod plasma;
pub mod shell;
pub mod shockwave;
pub mod simulation;
pub mod tank;
pub mod track;
