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
// The tank hull only fills part of its 32x32 tile (the rest is transparent
// padding). This scalar fraction still backs the AI's avoidance-radius math,
// the ground-decal rear-edge offset, and spawn-clearance checks (see
// `Tank::hull_size`) - all of those only need an approximate footprint, so
// they're left alone. The tank's actual physics collider is sized more
// precisely per row instead - see TANK_HULL_BBOX_BY_ROW.
pub const TANK_HULL_FRACTION: f32 = 0.7;
// Per-row hull footprint (width, height) in tile px, in the sprite's own
// "facing up" reference frame - measured bounding boxes from the spec's §9
// table. This is the tank's *damage* silhouette: it sizes the hull hit box
// the swept projectile test (`simulation::hits::Terrain::sweep`) checks
// directly, and - unlike TANK_HULL_FRACTION above - is
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
// by. It *is* used for shell/laser hit-testing though: the swept projectile
// test (`simulation::hits::Terrain::sweep`) checks `Tank::turret_bbox_world`
// alongside the hull box, so a
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
// knock around - see `simulation::drive_tank` and `simulation::combat`, all of which
// already divided by `Tank::mass` and so pick this up automatically), while
// `long`/`wide` and especially `super_heavy`/`super_long` end up
// meaningfully heavier (sluggish to accelerate, more perpendicular drift
// through a turn, and shove lighter tanks further than they get shoved back
// in a ram).
// Chassis power tier per sprite row (`Tank::row`), for wave composition
// (docs/maps-to-levels.md): the four rungs of the wave ladder, grouped by
// the same role hints the mass/damage tables use - narrow/compact rows are
// light, std medium, long/wide heavy, the two super-heavies super.
pub const TANK_TIER_BY_ROW: [level::Tier; 12] = [
    level::Tier::Light,  // scout
    level::Tier::Medium, // assault
    level::Tier::Heavy,  // breaker
    level::Tier::Heavy,  // longbow
    level::Tier::Light,  // flak
    level::Tier::Light,  // wraith
    level::Tier::Medium, // warden
    level::Tier::Heavy,  // ravager
    level::Tier::Light,  // glacier
    level::Tier::Heavy,  // obelisk
    level::Tier::Super,  // titan
    level::Tier::Super,  // leviathan
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
// How far forward (tile px, toward wherever the hull currently faces) a
// shell spawns from the tank's center - i.e. where the turret/barrel tip
// actually sits, not the tank's own center or the edge of its 32x32 tile.
// Varies per row: taken directly from the spec's published turret
// bounding-box y0 per row (§9), converted to an above-center distance
// (16 - y0). Indexed by `Tank::row` - see `Shell::spawn`.
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
// How long after a twin-barrel chassis's first shell the second one fires
// (see `Tank::pending_shot`) - long enough to read as two distinct shots,
// short enough that it's still clearly one trigger-pull. Comfortably under
// both PLAYER_FIRE_INTERVAL (0.15s) and the enemy AI's fastest fire_interval
// (ENEMY_FIRE_INTERVAL_AGGRESSIVE, 0.7s), so the pending second shell always
// resolves well before that same tank could legally fire again - nothing
// handles a fresh trigger-pull arriving while one is still pending.
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
pub const SHELL_SCALE: f32 = 2.0;

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
pub const MAX_DAMAGE: f32 = 100.0;

// AI pathfinding (see pathfind.rs): a coarse grid A* layer so an enemy
// routes around static obstacles (obstacle.rs) instead of just walking into
// one and getting physically stuck by its collider - `Ai::steer` swaps its
// naive straight-line heading for the grid's first step toward the target.
// Rebuilt fresh every frame in Game::update (obstacles are few and the grid
// is small, so this is cheap enough not to need caching/invalidation).
pub const PATHFIND_CELL_SIZE: f32 = 48.0; // px per grid cell

// Track marks: tracks.png is a single 32x32 tile of two tread ladders (matching
// the tank sprite orientation). A tank drops a mark every TRACK_SPACING pixels it
// travels, and each mark fades out over TRACK_LIFETIME seconds.
pub const TRACK_TEXTURE_SIZE: f32 = 32.0;

// Default window (and battlefield) size, shared by the game binary and the
// headless probe (src/bin/probe.rs) so the battlefield the probe sweeps is
// byte-for-byte the one the real game opens with - moved here from a
// main.rs-private static exactly so the two can't drift (a bin can't
// import from another bin). `--resolution` (main.rs) still overrides at
// runtime; the probe always runs at this default.
pub const DEFAULT_SCREEN_WIDTH: i32 = 1280;
pub const DEFAULT_SCREEN_HEIGHT: i32 = 720;

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
// Fraction of spawned Wood obstacles that catch fire when destroyed instead
// of breaking outright (Obstacle::flammable, rolled once at spawn) -
// "breaks easily" vs "catches fire" is gameplay data layered onto shared
// art, per docs/WALLS_SPEC.md.
// Burning wood's 3-frame flicker loop cadence (cols 4-6) - ~7.7 FPS, the
// spec's own suggested reading for a good flicker.
// Total time a Wood obstacle spends in the burning loop before charring
// (col 7) and finally being removed - same instant-vanish-once-destroyed
// convention as every other material (see Obstacle::tick_burn).
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
// Physics collider half-extents (px) - sized to the sprite's actual visible
// footprint (see docs/FROG_SPEC.md's per-frame bbox measurements), not the
// full padded 48x48 cell. Same `Physics::spawn_static` fixed-body shape as
// an Obstacle: blocks tank movement and doubles as the shell-hit target,
// no separate sensor collider needed.
pub const FROG_COLLIDER_HALF_EXTENT: (f32, f32) = (22.0, 16.0);

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
// Debounce so a rapid volley of hits doesn't trigger a hop every single
// frame one lands - roughly one hop per FROG_HOP_COOLDOWN_SECONDS even
// under sustained fire.
// `simulation::frog_hop_target`'s search: tries the ideal dead-away-from-
// the-shot angle first (plus a little random jitter so hops don't all look
// mechanically identical), then this fan of offsets from it, so a frog
// backed into a corner/wall still has a shot at finding *some* clear
// landing spot rather than never hopping at all near terrain.
pub const FROG_HOP_ANGLE_FAN_DEG: [f32; 5] = [0.0, 25.0, -25.0, 50.0, -50.0];

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

pub const MINIGUN_BULLET_TEXTURE_SIZE: f32 = 32.0;
pub const MINIGUN_BULLET_SCALE: f32 = 2.0; // matches SHELL_SCALE - same on-screen chunkiness

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
// Dest-rect scale for the one shared mount texture, layered on top of
// Tank::scale - deliberately the same on every chassis (not indexed by
// row): the minigun is a fixed piece of hardware, so it reads as one
// consistent size regardless of which tank it's bolted to, the same way
// its ammo count/damage don't scale with chassis either. Since Tank::scale
// itself is already a flat 2.0 for every chassis (the tank-to-tank size
// difference lives in the sprite art, not in `scale`), this constant alone
// is what to tune if the mount should read bigger/smaller overall.
pub const MINIGUN_MOUNT_SCALE: f32 = 1.0;

pub const PLASMA_TEXTURE_SIZE: f32 = 32.0;
// Bigger than SHELL_SCALE (2.0) - a plasma bolt reads as visibly larger and
// heavier than a normal shell, matching its bigger damage per hit. Also
// scales the in-flight pulse glow (see `plasma::glow_pulse`'s `base_radius`),
// so this one constant sizes the whole effect. 2.08 = the original 2.6
// tuning, reduced 20% after it read too big on screen.
pub const PLASMA_SCALE: f32 = 2.08;

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
#[cfg(feature = "dev-tools")]
pub mod capi;
pub mod damage_stage;
#[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
pub mod devserver;
#[cfg(feature = "map-editor")]
pub mod editor;
pub mod frog;
pub mod game;
pub mod ground;
pub mod laser;
pub mod level;
pub mod map;
pub mod maplint;
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
pub mod tuning;
