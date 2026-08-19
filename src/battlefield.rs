//! Static battlefield terrain generation: the boundary walls, the player's
//! "BONG!" fortress, and the scattered obstacle structures - everything that
//! decides *where* static terrain goes for a fresh round. Called from
//! `simulation::Game::init` at the points where the data each step needs
//! becomes available (`spawn_player_fortress` needs the player's spawn
//! position; `scatter_obstacles` needs the final enemy positions), so a
//! single `init` can't just make one `battlefield::build(...)` call - see
//! `Game::init` for the actual call order.
//!
//! The ground layer (`ground.rs`) and the obstacle/structure *types*
//! themselves (`obstacle.rs`/`structures.rs`) stay in their own modules -
//! this one only owns placement: deciding which world-space positions get a
//! wall, a fortress tile, or a rolled obstacle structure, and spawning the
//! physics bodies/ECS entities for them.

use std::collections::HashSet;

use rand::RngExt;
use rapier2d::prelude::ColliderHandle;

use crate::obstacle::{MATERIALS, Material, Obstacle};
use crate::physics::Physics;
use crate::structures::{self, Blueprint};
use crate::tank::Tank;
use crate::{
    OBSTACLE_CLEAR, OBSTACLE_CLUSTER_CLEAR, OBSTACLE_COUNT_MAX, OBSTACLE_COUNT_MIN,
    OBSTACLE_GRID_SIZE, OBSTACLE_STRUCTURE_CHANCE, OBSTACLE_WOOD_FLAMMABLE_CHANCE, Position,
    TANK_HULL_BBOX_BY_ROW, WALL_THICKNESS,
};

/// Cap on rejection-sampling attempts for a single enemy/obstacle spawn
/// position (see `sample_clear_position`) - keeps `Game::init` provably
/// bounded regardless of how unlucky the RNG gets, rather than an unbounded
/// `while` loop that could in principle stall for a long time. At this
/// scale (a few hundred px of clearance against a >=1280x720 board)
/// satisfying every constraint on the first handful of tries is the
/// overwhelming common case; this cap only ever matters on a genuinely
/// pathological draw, and even then costs at most a couple thousand cheap
/// float comparisons - not a perceptible delay, let alone the alternative.
/// That alternative matters more than it sounds: on native a slow frame is
/// an invisible hitch the OS schedules around, but this project's web
/// build deliberately skips `-sASYNCIFY=1` (see `game_loop::run`'s doc
/// comment) to keep binary size down, so `Game::init` runs as one
/// uninterruptible synchronous call from the browser's perspective - any
/// real stall here freezes the whole tab until it returns, not just drops
/// a frame. Hitting the cap without finding a fully-valid position isn't
/// treated as an error: `sample_clear_position` just returns its last
/// (possibly still-too-close) sample rather than looping further - an
/// occasional slightly-crowded spawn beats a frozen tab.
const PLACEMENT_MAX_ATTEMPTS: u32 = 2000;

/// Sample a random position within `margin_min..(width-margin_min)` x
/// `margin_min..(height-margin_min)`, retrying until `valid` accepts one or
/// `PLACEMENT_MAX_ATTEMPTS` is reached - see that constant's doc comment
/// for why this is capped rather than an unbounded loop. Used by the enemy
/// placement loop in `Game::init`; the obstacle loop uses the grid-snapping,
/// multi-tile variant below instead (see `sample_structure_positions`).
pub fn sample_clear_position(
    rng: &mut rand::rngs::ThreadRng,
    width: f32,
    height: f32,
    margin_min: f32,
    mut valid: impl FnMut(Position) -> bool,
) -> Position {
    let mut pos = Position::new(0.0, 0.0);
    for _ in 0..PLACEMENT_MAX_ATTEMPTS {
        pos = Position::new(
            rng.random_range(margin_min..(width - margin_min)),
            rng.random_range(margin_min..(height - margin_min)),
        );
        if valid(pos) {
            break;
        }
    }
    pos
}

/// Rejection-samples a grid-aligned anchor cell for `blueprint` (see
/// `structures.rs`), retrying until every one of its tiles (anchor + each
/// offset, in grid cells) is in-bounds and passes `valid`, or
/// `PLACEMENT_MAX_ATTEMPTS` is reached - at which point it gives up and
/// returns `None` rather than force-placing a partially invalid shape (a
/// multi-tile structure landing half out-of-bounds or overlapping is worse
/// than just skipping it for this round; see `sample_clear_position` for the
/// single-point placement loops, which instead fall back to their last
/// attempt since a single bad point is a much smaller problem). Used by
/// `scatter_obstacles`.
pub fn sample_structure_positions(
    rng: &mut rand::rngs::ThreadRng,
    width: f32,
    height: f32,
    margin_min: f32,
    grid: f32,
    blueprint: structures::Blueprint,
    mut valid: impl FnMut(&[Position]) -> bool,
) -> Option<Vec<Position>> {
    for _ in 0..PLACEMENT_MAX_ATTEMPTS {
        let anchor_x = (rng.random_range(margin_min..(width - margin_min)) / grid).round();
        let anchor_y = (rng.random_range(margin_min..(height - margin_min)) / grid).round();
        let tiles: Vec<Position> = blueprint
            .iter()
            .map(|&(dx, dy)| {
                Position::new((anchor_x + dx as f32) * grid, (anchor_y + dy as f32) * grid)
            })
            .collect();
        let in_bounds = tiles.iter().all(|p| {
            p.x >= margin_min
                && p.x <= width - margin_min
                && p.y >= margin_min
                && p.y <= height - margin_min
        });
        if in_bounds && valid(&tiles) {
            return Some(tiles);
        }
    }
    None
}

/// One glyph in the tiny pixel font `spawn_player_fortress` spells "BONG!"
/// with. Each row is a string where `#` marks a filled tile and anything
/// else (conventionally `.`) is empty; every row in a glyph must be the same
/// length, since that length *is* the glyph's width in tiles - a narrower
/// glyph (see `GLYPH_BANG`) just uses shorter rows.
type Glyph = &'static [&'static str];

const GLYPH_B: Glyph = &[
    "####.", "#...#", "#...#", "####.", "#...#", "#...#", "####.",
];
const GLYPH_O: Glyph = &[
    ".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
];
const GLYPH_N: Glyph = &[
    "#...#", "##..#", "#.#.#", "#.#.#", "#..##", "#...#", "#...#",
];
const GLYPH_G: Glyph = &[
    ".####", "#....", "#....", "#..##", "#...#", "#...#", ".###.",
];
const GLYPH_BANG: Glyph = &["#", "#", "#", "#", "#", ".", "#"];

/// Column gap (in tiles) between adjacent glyphs.
const GLYPH_GAP: i32 = 2;
const WORD_GLYPHS: [Glyph; 5] = [GLYPH_B, GLYPH_O, GLYPH_N, GLYPH_G, GLYPH_BANG];
const WORD_MATERIALS: [Material; 5] = [
    Material::Brick,
    Material::Glass,
    Material::Wood,
    Material::Iron,
    Material::Brick,
];
/// Index into `WORD_GLYPHS`/`WORD_MATERIALS` of the `O` the player spawns
/// inside.
const WORD_O_INDEX: usize = 1;

/// How many grid cells of road to surround each of the "BONG!" sign's wall
/// tiles with (a Chebyshev/square dilation - see `dilate_cells`), in grid
/// cells (each `OBSTACLE_GRID_SIZE`/`GROUND_WORLD_TILE`, the same width as
/// one wall tile). 0 since the de-green pass (docs/PALETTE.md): at 1, the
/// dirt tiles' soft baked-in fringes visually closed the 1-cell gap the
/// `GLYPH_GAP` (2) spacing was supposed to keep, merging every glyph's ring
/// into one giant khaki moat spanning the whole word - a huge fraction of
/// the screen reading yellow. With 0, dirt shows only inside the `B`/`O`
/// courtyards and as thin autotile fringes peeking out from under wall
/// tiles, and the sign sits on grass. Bump back up for a deliberate
/// plaza/moat look, knowing rings will merge.
const FORTRESS_ROAD_SURROUND: i32 = 0;

/// Column each glyph starts at (left to right, `GLYPH_GAP` tiles apart), the
/// column the `O` glyph is centered on, and the column the whole word's
/// bounding box is centered on. Shared by `spawn_player_fortress` (which
/// places every tile relative to the `O`'s column) and `fortress_center_offset`
/// (which needs the gap between those two centers) so the two can't drift
/// out of sync with each other or with `GLYPH_GAP`.
fn fortress_columns() -> ([i32; 5], i32, i32) {
    let mut glyph_start_col = [0i32; 5];
    let mut cursor = 0;
    for (i, glyph) in WORD_GLYPHS.iter().enumerate() {
        glyph_start_col[i] = cursor;
        cursor += glyph[0].len() as i32 + GLYPH_GAP;
    }
    let word_width_cols = cursor - GLYPH_GAP;
    let o_width = WORD_GLYPHS[WORD_O_INDEX][0].len() as i32;
    let o_center_col = glyph_start_col[WORD_O_INDEX] + o_width / 2;
    let word_center_col = (word_width_cols - 1) / 2;
    (glyph_start_col, o_center_col, word_center_col)
}

/// Every `.` cell of `glyph` that's enclosed by `#` tiles (unreachable from
/// outside the glyph's own bounding box without crossing one) - e.g. the
/// donut hole in `GLYPH_O`, or `GLYPH_B`'s two loops. Found by flood-filling
/// from just outside the glyph (a 1-cell padding border, guaranteed empty)
/// and marking every `.` cell that flood reaches; whatever's left over is
/// interior. Returns `(row, col)` offsets in the same indexing
/// `spawn_player_fortress`'s own wall-tile loop uses. General on purpose (no
/// hand-picked coordinates) so it stays correct if `GLYPH_B`/`GLYPH_O`'s art
/// ever changes.
fn glyph_interior_cells(glyph: Glyph) -> Vec<(i32, i32)> {
    let rows = glyph.len() as i32;
    let cols = glyph[0].len() as i32;
    let is_wall = |r: i32, c: i32| -> bool {
        r >= 0 && c >= 0 && r < rows && c < cols && glyph[r as usize].as_bytes()[c as usize] == b'#'
    };
    // Padded by 1 cell on every side so the flood-fill's start point
    // (-1, -1) is trivially outside the glyph and never accidentally walled in.
    let padded_cols = cols + 2;
    let idx = |r: i32, c: i32| -> usize { ((r + 1) * padded_cols + (c + 1)) as usize };
    let mut exterior = vec![false; ((rows + 2) * padded_cols) as usize];
    let mut stack = vec![(-1i32, -1i32)];
    exterior[idx(-1, -1)] = true;
    while let Some((r, c)) = stack.pop() {
        for (dr, dc) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let (nr, nc) = (r + dr, c + dc);
            if nr < -1 || nc < -1 || nr > rows || nc > cols || exterior[idx(nr, nc)] || is_wall(nr, nc) {
                continue;
            }
            exterior[idx(nr, nc)] = true;
            stack.push((nr, nc));
        }
    }
    (0..rows)
        .flat_map(|r| (0..cols).map(move |c| (r, c)))
        .filter(|&(r, c)| !is_wall(r, c) && !exterior[idx(r, c)])
        .collect()
}

/// Every cell within Chebyshev distance `radius` of some cell in `cells`
/// (a square structuring element, so a straight wall run gets a straight
/// margin, not a rounded/diamond one), excluding `cells` themselves - i.e.
/// just the surrounding ring/moat, not the original shape. Dedupes
/// naturally: a cell near two different source cells (e.g. an inside
/// corner) only appears once.
fn dilate_cells(cells: &HashSet<(i32, i32)>, radius: i32) -> Vec<(i32, i32)> {
    let mut ring = HashSet::new();
    for &(x, y) in cells {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let c = (x + dx, y + dy);
                if !cells.contains(&c) {
                    ring.insert(c);
                }
            }
        }
    }
    ring.into_iter().collect()
}

/// World-space x offset from "BONG!"'s own bounding-box center to the `O`'s
/// center (there's no y equivalent - every glyph is the same height, so the
/// word is already vertically symmetric around the O). `BONG!` isn't
/// symmetric around its `O` left-to-right (just `B` hangs to the left, but
/// `N`, `G` and `!` all hang to the right), so anchoring the word on the `O`
/// and putting the `O` at the screen's actual center read as visibly
/// off-center, bulked right. `Game::init` subtracts this from the screen's
/// true center to find where the `O` - and the player spawning inside it -
/// should actually go, so the word as a whole reads centered instead.
pub fn fortress_center_offset() -> f32 {
    let (_, o_center_col, word_center_col) = fortress_columns();
    (word_center_col - o_center_col) as f32 * OBSTACLE_GRID_SIZE
}

/// Result of `spawn_player_fortress`.
pub struct Fortress {
    /// Every wall-tile position placed (folded into `Game::init`'s
    /// `obstacle_positions` so later spawns steer clear of them
    /// individually, and so ground can paint road under each one).
    pub tiles: Vec<Position>,
    /// Exclusion radius in px - the furthest any tile sits from `center`
    /// (the `O`'s own center, not the whole word's, since the word isn't
    /// symmetric around it), so a circular distance check from `center`
    /// fully contains the whole sign despite `BONG!`'s lopsided shape.
    pub exclusion_radius: f32,
    /// World positions of extra ground cells `Game::init` should paint road
    /// onto, on top of the "road under every object" treatment wall tiles
    /// already get elsewhere: the `B`/`O` glyphs' own hollow interior cells
    /// (see `glyph_interior_cells` - not wall tiles, just the floor inside
    /// them, where the player spawns in the `O`'s case) plus a
    /// `FORTRESS_ROAD_SURROUND`-wide ring around every wall tile (see
    /// `dilate_cells`).
    pub road_cells: Vec<Position>,
}

/// The player's starting enclosure: not a shape this time, but the word
/// "BONG!" spelled out in wall tiles (see `Glyph`/`GLYPH_*` above), one
/// material per letter - B is Brick, N is Wood, G is Iron, and the `!`
/// reuses Brick (only four materials exist, so with five glyphs one repeat
/// is unavoidable; reusing B's non-adjacent material keeps neighboring
/// glyphs visually distinct). The player spawns inside the `O`, which is
/// Glass - the weakest material (`Material::max_health`) and the same
/// material the original circular version of this enclosure used for its
/// shootable weak point - so a shot or two from inside cracks an escape
/// route same as before. Every tile still spawns as its own `Obstacle`,
/// same as a `structures.rs`-generated shape, but this layout is fixed
/// rather than randomly rolled and mixes materials per glyph (unlike the
/// generator's "one material per structure" rule) - exactly why it's its
/// own function instead of a `structures::Blueprint`.
///
/// Skips entirely (returns an empty `Fortress`) if the word wouldn't fully
/// fit inside the battlefield's walls - same "give up gracefully" convention
/// as `sample_structure_positions`, rather than spawning a word that clips
/// through the boundary.
pub fn spawn_player_fortress(
    physics: &mut Physics,
    world: &mut hecs::World,
    rng: &mut rand::rngs::ThreadRng,
    center: Position,
    obstacle_half_extent: f32,
    width: f32,
    height: f32,
) -> Fortress {
    // Place the grid so the O's own center lands exactly on `center` - the
    // word hangs however it hangs left/right of that, not centered as a
    // whole (B.../!'s combined width isn't symmetric around the O; see
    // `fortress_center_offset`, which is how `Game::init` compensates for
    // that when it picks where `center` itself goes).
    let (glyph_start_col, o_center_col, _) = fortress_columns();
    let row_center = WORD_GLYPHS[0].len() as i32 / 2;
    let center_gx = (center.x / OBSTACLE_GRID_SIZE).round() as i32 - o_center_col;
    let center_gy = (center.y / OBSTACLE_GRID_SIZE).round() as i32 - row_center;
    let grid_to_pos =
        |gx: i32, gy: i32| Position::new(gx as f32 * OBSTACLE_GRID_SIZE, gy as f32 * OBSTACLE_GRID_SIZE);

    let mut tiles: Vec<(Position, Material)> = Vec::new();
    let mut wall_cells: HashSet<(i32, i32)> = HashSet::new();
    for (i, glyph) in WORD_GLYPHS.iter().enumerate() {
        let material = WORD_MATERIALS[i];
        for (row, line) in glyph.iter().enumerate() {
            for (col, ch) in line.chars().enumerate() {
                if ch != '#' {
                    continue;
                }
                let gx = center_gx + glyph_start_col[i] + col as i32;
                let gy = center_gy + row as i32;
                wall_cells.insert((gx, gy));
                tiles.push((grid_to_pos(gx, gy), material));
            }
        }
    }

    // The playable area is exactly [0,width] x [0,height] - `spawn_walls`'s
    // boundary colliders sit entirely outside that range (inner edge right
    // at 0/width), so WALL_THICKNESS itself isn't part of this margin; only
    // half the obstacle's own sprite needs to clear the edge.
    let margin = obstacle_half_extent;
    let fits = tiles.iter().all(|(p, _)| {
        p.x >= margin && p.x <= width - margin && p.y >= margin && p.y <= height - margin
    });
    if !fits {
        return Fortress {
            tiles: Vec::new(),
            exclusion_radius: 0.0,
            road_cells: Vec::new(),
        };
    }

    // Cells enclosed by the B and O glyphs (not wall tiles - the hollow
    // floor inside them, see `glyph_interior_cells`), in the same
    // grid-offset coordinate space the wall tiles above were placed in.
    let mut road_cells: Vec<Position> = [0usize, WORD_O_INDEX]
        .into_iter()
        .flat_map(|glyph_index| {
            glyph_interior_cells(WORD_GLYPHS[glyph_index])
                .into_iter()
                .map(move |(row, col)| {
                    let gx = center_gx + glyph_start_col[glyph_index] + col;
                    let gy = center_gy + row;
                    grid_to_pos(gx, gy)
                })
        })
        .collect();
    // A road ring around every wall tile - see
    // `FORTRESS_ROAD_SURROUND`/`dilate_cells`.
    road_cells.extend(
        dilate_cells(&wall_cells, FORTRESS_ROAD_SURROUND)
            .into_iter()
            .map(|(gx, gy)| grid_to_pos(gx, gy)),
    );

    // One variant per material (docs/WALLS_SPEC.md: "keep one variant per
    // structure") - every tile of a given material (even across the two
    // glyphs Brick covers) shares that material's single rolled variant, so
    // the sign reads as one consistent build rather than mismatched patches.
    let brick_variant = rng.random_range(0..Material::Brick.variants());
    let glass_variant = rng.random_range(0..Material::Glass.variants());
    let wood_variant = rng.random_range(0..Material::Wood.variants());
    let iron_variant = rng.random_range(0..Material::Iron.variants());

    let mut positions = Vec::with_capacity(tiles.len());
    for (pos, material) in tiles {
        let variant = match material {
            Material::Brick => brick_variant,
            Material::Glass => glass_variant,
            Material::Wood => wood_variant,
            Material::Iron => iron_variant,
        };
        let max_health = material.max_health();
        let flammable =
            material == Material::Wood && rng.random_bool(OBSTACLE_WOOD_FLAMMABLE_CHANCE);
        let body = physics.spawn_static(
            pos,
            Position::new(obstacle_half_extent, obstacle_half_extent),
        );
        positions.push(pos);
        world.spawn((Obstacle {
            material,
            variant,
            position: pos,
            health: max_health,
            max_health,
            flammable,
            burning: false,
            burn_frame: 0,
            burn_frame_timer: 0.0,
            burn_elapsed: 0.0,
            body,
            destroyed: false,
        },));
    }
    let exclusion_radius = positions
        .iter()
        .fold(0.0f32, |max, p| max.max(p.distance_to(center)))
        + obstacle_half_extent;
    Fortress {
        tiles: positions,
        exclusion_radius,
        road_cells,
    }
}

/// Scatter this round's obstacle structures: static in-arena terrain, placed
/// after tanks so it can steer clear of both the player's fixed start and
/// every enemy's rolled start (see `OBSTACLE_CLEAR`) instead of tanks
/// needing to dodge obstacles placed first. Reuses the same margin/border
/// band as enemy spawning (`margin_min`) so obstacles land within the same
/// playable interior, not hugging the boundary walls.
///
/// Each iteration places one *structure* (see `structures.rs`) rather than
/// one tile: usually a lone `structures::SINGLE`, sometimes
/// (`OBSTACLE_STRUCTURE_CHANCE`) a real multi-tile shape - a wall run, a
/// corner, a bunker. Every tile of a structure shares one rolled
/// material/variant (docs/WALLS_SPEC.md: "keep one variant per structure" so
/// the art's bond/rib/board pattern runs unbroken); Wood's `flammable` still
/// rolls per tile. `obstacle_positions` starts pre-seeded with the player
/// fortress's own tiles (see `spawn_player_fortress`) so these structures
/// naturally steer clear of it too, on top of the `clear` radius check.
///
/// Minimum gap between a new structure's tiles and every *previously
/// placed* structure's tiles (`obstacle_positions` only holds prior
/// structures' tiles at check time - this structure's own are pushed after
/// it's accepted, so this is purely a cross-structure check, never a
/// same-structure one). `OBSTACLE_CLUSTER_CLEAR` alone (one grid cell - see
/// its own doc comment) only keeps tiles from visually overlapping; it says
/// nothing about whether a tank can actually fit through the resulting gap.
/// Two independently-rolled structures landing that close together could
/// otherwise pinch a passage narrower than any tank's own pathfinding
/// margin (`max_tank_avoidance_radius`, used to build the obstacle grid in
/// `Game::update`) needs to route through - `Grid::next_step` then has no
/// path at all, and steering falls back to aiming straight at an
/// unreachable target (see `Ai::steer`'s stuck-escape, which exists as the
/// safety net for exactly this, but preventing the pinch outright is
/// cheaper than repeatedly escaping it). No equivalent explicit
/// wall-clearance margin is needed: `margin_min` is expected to already be
/// well over twice any tank's avoidance radius on its own (true of
/// `Game::init`'s `0.2 * short_side`, 144px on a 720-tall field).
///
/// A structure that can't find a valid spot within its attempt budget is
/// just skipped rather than force-placed broken - the round ends up with
/// slightly fewer structures than `structure_count` rather than an
/// unnavigable layout.
#[allow(clippy::too_many_arguments)]
pub fn scatter_obstacles(
    physics: &mut Physics,
    world: &mut hecs::World,
    rng: &mut rand::rngs::ThreadRng,
    width: f32,
    height: f32,
    margin_min: f32,
    obstacle_half_extent: f32,
    center: Position,
    clear: f32,
    enemy_positions: &[Position],
    enemy_clear: f32,
    obstacle_positions: &mut Vec<Position>,
    structure_count: usize,
) {
    let structure_corridor_clear = 2.0 * max_tank_avoidance_radius();
    for _ in 0..structure_count {
        let blueprint: Blueprint = if rng.random_bool(OBSTACLE_STRUCTURE_CHANCE) {
            structures::STRUCTURES[rng.random_range(0..structures::STRUCTURES.len())]
        } else {
            structures::SINGLE
        };
        let Some(tiles) = sample_structure_positions(
            rng,
            width,
            height,
            margin_min,
            OBSTACLE_GRID_SIZE,
            blueprint,
            |tiles| {
                tiles.iter().all(|pos| {
                    pos.distance_to(center) >= clear + OBSTACLE_CLEAR
                        && enemy_positions
                            .iter()
                            .all(|&p| pos.distance_to(p) >= enemy_clear + OBSTACLE_CLEAR)
                        && obstacle_positions.iter().all(|&p| {
                            pos.distance_to(p) >= OBSTACLE_CLUSTER_CLEAR.max(structure_corridor_clear)
                        })
                })
            },
        ) else {
            continue;
        };
        let material = MATERIALS[rng.random_range(0..MATERIALS.len())];
        let variant = rng.random_range(0..material.variants());
        let max_health = material.max_health();
        for pos in tiles {
            let flammable =
                material == Material::Wood && rng.random_bool(OBSTACLE_WOOD_FLAMMABLE_CHANCE);
            let body = physics.spawn_static(
                pos,
                Position::new(obstacle_half_extent, obstacle_half_extent),
            );
            obstacle_positions.push(pos);
            world.spawn((Obstacle {
                material,
                variant,
                position: pos,
                health: max_health,
                max_health,
                flammable,
                burning: false,
                burn_frame: 0,
                burn_frame_timer: 0.0,
                burn_elapsed: 0.0,
                body,
                destroyed: false,
            },));
        }
    }
}

/// How many obstacle structures to place this round, honoring `override_count`
/// (see `Game::obstacle_count_override`, `src/bin/probe.rs`'s `--obstacles`)
/// when set, otherwise rolling `OBSTACLE_COUNT_MIN..=OBSTACLE_COUNT_MAX`.
pub fn roll_structure_count(
    rng: &mut rand::rngs::ThreadRng,
    override_count: Option<usize>,
) -> usize {
    override_count.unwrap_or_else(|| rng.random_range(OBSTACLE_COUNT_MIN..=OBSTACLE_COUNT_MAX))
}

/// Build the battlefield boundary: four static wall colliders positioned so
/// their inner faces sit exactly at the screen edges (0..width, 0..height) -
/// the same bound the old hand-rolled `Tank::clamp_to_field` used to
/// enforce, since a tank's collider now stops flush against a wall's
/// surface instead. Padded outward by `WALL_THICKNESS` so corners are
/// covered with no gap; that padding is otherwise arbitrary since it's
/// never rendered. Returns each wall's collider handle (see `Game::walls`)
/// so a shell's flight can be checked against them too, the same way as
/// tanks/obstacles - see `find_shell_target`.
pub fn spawn_walls(physics: &mut Physics, width: f32, height: f32) -> [ColliderHandle; 4] {
    let t = WALL_THICKNESS;
    let left = physics.spawn_static(
        Position::new(-t / 2.0, height / 2.0),
        Position::new(t / 2.0, height / 2.0 + t),
    );
    let right = physics.spawn_static(
        Position::new(width + t / 2.0, height / 2.0),
        Position::new(t / 2.0, height / 2.0 + t),
    );
    let top = physics.spawn_static(
        Position::new(width / 2.0, -t / 2.0),
        Position::new(width / 2.0 + t, t / 2.0),
    );
    let bottom = physics.spawn_static(
        Position::new(width / 2.0, height + t / 2.0),
        Position::new(width / 2.0 + t, t / 2.0),
    );
    [left, right, top, bottom].map(|body| physics.collider_of(body))
}

/// The largest `Tank::avoidance_radius` reachable by *any* row in
/// TANK_HULL_BBOX_BY_ROW, in either cardinal orientation (a row's own bbox is
/// asymmetric, so facing sideways can give a bigger radius than facing up) -
/// used both as `scatter_obstacles`'s corridor-clearance margin and as the
/// pathfinding grid's obstacle-clearance margin (see its call site in
/// `Game::update`) so a route never opens a gap too narrow for the biggest
/// tank that might need it (titan/leviathan, currently). Folds over only 12
/// rows x 2 orientations, so recomputing this fresh each frame (rather than
/// caching) is well within the same "cheap enough" budget the grid rebuild
/// itself already accepts.
pub fn max_tank_avoidance_radius() -> f32 {
    let scale = Tank::default().scale;
    TANK_HULL_BBOX_BY_ROW
        .iter()
        .flat_map(|&(w, h)| [(w, h), (h, w)])
        .map(|(w, h)| {
            let (hx, hy) = (w * 0.5 * scale, h * 0.5 * scale);
            (hx * hx + hy * hy).sqrt()
        })
        .fold(0.0, f32::max)
}
