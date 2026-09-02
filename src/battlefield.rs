//! Static battlefield terrain generation: the boundary walls and the
//! hand-authored/editor-saved map's walls/road/frog/pickup-slots -
//! everything that decides *where* static terrain goes for a fresh round.
//! Called from `simulation::Game::init` - see that function for the actual
//! call order.
//!
//! Every round loads a `map::MapFile` (either `-m`/`--map`, or
//! `maps/default.toml` when none is given - see `main.rs`); there is no
//! procedural fallback battlefield any more; a map is the *only* source of
//! static terrain now. See docs/map-editor-design.md.
//!
//! The ground layer (`ground.rs`) and the obstacle types themselves
//! (`obstacle.rs`) stay in their own modules - this one only owns placement:
//! deciding which world-space positions get a wall, and spawning the
//! physics bodies/ECS entities for them.

use crate::tuning::tuning;
use std::collections::{HashMap, HashSet};

use rand::RngExt;
use rand::rngs::SmallRng;

use crate::ai::Ai;
use crate::map::{CellObject, MapFile, cell_to_world};
use crate::obstacle::{MATERIALS, Material, Obstacle};
use crate::pathfind::Grid;
use crate::physics::Physics;
use crate::pickup::PickupKind;
use crate::tank::Tank;
use crate::{
    OBSTACLE_CLEAR,
    OBSTACLE_GRID_SIZE,
    PATHFIND_CELL_SIZE,
    Position,
    TANK_HULL_BBOX_BY_ROW,
    TANK_MOVE_BBOX_FRACTION,
    WALL_THICKNESS,
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
/// and frog-fallback placement loops in `Game::init`.
pub fn sample_clear_position(
    rng: &mut SmallRng,
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

/// Collision half-extents for one obstacle tile at grid cell `(gx, gy)`,
/// given the full set of grid cells occupied by tiles in the same map
/// (`cells`) - closes the gap `OBSTACLE_HULL_FRACTION`
/// would otherwise leave against any adjacent tile from that same set.
///
/// Two neighboring tiles are placed with centers exactly `OBSTACLE_GRID_SIZE`
/// (32px) apart, but each tile's own collider is only `OBSTACLE_HULL_FRACTION`
/// (0.75) of that - 24px wide, centered in its cell - leaving an 8px seam
/// between two adjacent tiles' colliders. A shell's own hit box
/// (`SHELL_HIT_HALF_EXTENT * 2` = 6px) fits inside that gap, so a
/// well-aligned shot can thread clean between two visually touching wall
/// tiles without ever registering a hit on either one - no frame hitch or
/// lucky timing needed (contrast `simulation::hits::Terrain::sweep`'s doc
/// comment, which covers a *timing*-based tunneling gap; this one is a pure
/// placement gap, deterministic the moment a shot lines up with the seam).
///
/// Fixed by widening each tile's half-extent, per axis, to the full half
/// cell (16px) on any axis where it has a same-group neighbor immediately
/// adjacent - meeting that neighbor's own matching extension exactly in the
/// middle, closing the gap to zero. An axis with no neighbor keeps the
/// normal (smaller) half-extent unchanged, so a lone obstacle's footprint,
/// and a structure's outward-facing silhouette, both look and feel exactly
/// as before - only the seams *between* a structure's own tiles close up.
pub(crate) fn tile_hull_half_extent(cells: &HashSet<(i32, i32)>, gx: i32, gy: i32, base: f32) -> Position {
    let full = OBSTACLE_GRID_SIZE * 0.5;
    let has_x_neighbor = cells.contains(&(gx - 1, gy)) || cells.contains(&(gx + 1, gy));
    let has_y_neighbor = cells.contains(&(gx, gy - 1)) || cells.contains(&(gx, gy + 1));
    Position::new(
        if has_x_neighbor { full } else { base },
        if has_y_neighbor { full } else { base },
    )
}

/// Grid cell of a tile already placed at `pos` (a `Position` that's always
/// an exact multiple of `OBSTACLE_GRID_SIZE` - see `grid_to_pos`/
/// `cell_to_world`) - the inverse of that conversion, so call sites that
/// only kept the world-space `Position` can still recover `(gx, gy)` for
/// `tile_hull_half_extent`'s neighbor lookup.
pub(crate) fn pos_to_cell(pos: Position) -> (i32, i32) {
    (
        (pos.x / OBSTACLE_GRID_SIZE).round() as i32,
        (pos.y / OBSTACLE_GRID_SIZE).round() as i32,
    )
}

/// The four battlefield boundary walls as (center, half-extents) rectangles:
/// inner faces exactly at the screen edges (0..width, 0..height), padded
/// outward by `WALL_THICKNESS` so the corners are covered. Shared by
/// `spawn_walls` (the physics bodies tanks stop against) and the projectile
/// hit test (`simulation::hits::Terrain`), so the two can never disagree.
pub fn wall_rects(width: f32, height: f32) -> [(Position, Position); 4] {
    let t = WALL_THICKNESS;
    [
        (Position::new(-t / 2.0, height / 2.0), Position::new(t / 2.0, height / 2.0 + t)),
        (Position::new(width + t / 2.0, height / 2.0), Position::new(t / 2.0, height / 2.0 + t)),
        (Position::new(width / 2.0, -t / 2.0), Position::new(width / 2.0 + t, t / 2.0)),
        (Position::new(width / 2.0, height + t / 2.0), Position::new(width / 2.0 + t, t / 2.0)),
    ]
}

/// Spawn the battlefield boundary: four static wall colliders (see
/// `wall_rects`) a tank's movement collider stops flush against. Never
/// rendered and never removed.
pub fn spawn_walls(physics: &mut Physics, width: f32, height: f32) {
    for (center, half) in wall_rects(width, height) {
        physics.spawn_static(center, half);
    }
}

/// The largest `Tank::avoidance_radius` reachable by *any* row in
/// TANK_HULL_BBOX_BY_ROW, in either cardinal orientation (a row's own bbox is
/// asymmetric, so facing sideways can give a bigger radius than facing up) -
/// used as the pathfinding grid's obstacle-clearance margin (see its call site in
/// `Game::update`) so a route never opens a gap too narrow for the biggest
/// tank that might need it (titan/leviathan, currently). Applies the same
/// TANK_MOVE_BBOX_FRACTION shrink `Tank::move_half_extents` does, mirroring
/// `avoidance_radius`'s own movement-box basis - keep the two formulas in
/// lockstep, or the grid's notion of "fits" drifts from what the physics
/// bodies actually block on. Folds over only 12 rows x 2 orientations, so
/// recomputing this fresh each frame (rather than caching) is well within
/// the same "cheap enough" budget the grid rebuild itself already accepts.
pub fn max_tank_avoidance_radius() -> f32 {
    let scale = Tank::default().scale;
    TANK_HULL_BBOX_BY_ROW
        .iter()
        .flat_map(|&(w, h)| [(w, h), (h, w)])
        .map(|(w, h)| {
            let (hx, hy) = (
                w * 0.5 * scale * TANK_MOVE_BBOX_FRACTION,
                h * 0.5 * scale * TANK_MOVE_BBOX_FRACTION,
            );
            (hx * hx + hy * hy).sqrt()
        })
        .fold(0.0, f32::max)
}

/// After every obstacle for the round is placed (a map's own walls - called
/// once, from `Game::init`, right after `spawn_from_map`), check every enemy
/// tank against the same pathfinding grid `Game::update` builds each frame
/// (see `pathfind::Grid`) and teleport any whose rolled spawn point turned
/// out fully boxed in (`Grid::boxed_in` - every cardinal neighbor cell
/// blocked, so `Ai::wander` could never route it anywhere) to the nearest
/// cell that isn't.
///
/// This can happen even though the enemy spawn loop's own rejection
/// sampling already keeps each enemy clear of every *individual* obstacle
/// tile (`OBSTACLE_CLEAR`): several independently-placed obstacles, each
/// individually respecting that clearance from the enemy, can still
/// collectively seal every direction out of its cell - the per-placement
/// checks only ever reason about one obstacle at a time, so nothing during
/// placement itself catches the *cumulative* effect. A check against the
/// finished layout is the only way to catch it, which is why this runs
/// once, after everything else this round is already down.
///
/// The player is deliberately excluded (`.with::<&Ai>()` - only enemies have
/// one) - a hand-authored map can still enclose the player's fixed spawn
/// point on purpose, and relocating it would silently override that
/// design choice rather than respect it.
pub fn relocate_boxed_in_tanks(physics: &mut Physics, world: &mut hecs::World, width: f32, height: f32) {
    let grid = Grid::build(
        width,
        height,
        PATHFIND_CELL_SIZE,
        max_tank_avoidance_radius(),
        world
            .query::<&Obstacle>()
            .iter()
            .map(|o| (o.position, o.hull_size() * 0.5)),
    );
    // Snapshot every enemy's entity + position up front, then keep it in
    // sync as tanks get relocated below - `nearest_open`'s `avoid` list
    // needs every *other* tank's current position, relocated ones
    // included, not just the original layout, so two boxed-in tanks near
    // each other never both land on the exact same nearest cell (found via
    // the probe harness: two enemies at literally identical coordinates).
    let mut positions: Vec<(hecs::Entity, Position)> = world
        .query::<(hecs::Entity, &Tank)>()
        .with::<&Ai>()
        .iter()
        .map(|(entity, tank)| (entity, tank.position))
        .collect();
    for i in 0..positions.len() {
        let (entity, pos) = positions[i];
        if !grid.boxed_in(pos) {
            continue;
        }
        let avoid: Vec<Position> = positions
            .iter()
            .enumerate()
            .filter(|&(j, _)| j != i)
            .map(|(_, &(_, p))| p)
            .collect();
        let new_pos = grid.nearest_open(pos, &avoid, OBSTACLE_CLEAR);
        positions[i].1 = new_pos;
        let mut q = world.query_one::<&mut Tank>(entity);
        let tank = q.get().expect("entity from this same world's own query always has a Tank");
        tank.position = new_pos;
        if let Some(body) = tank.body {
            physics.set_position(body, new_pos);
        }
    }
}

/// What `spawn_from_map` found and spawned - handed back to `Game::init` so
/// it can fold each part into the same clearance/road/frog/pickup handling
/// a random round already does, rather than duplicating any of it here. See
/// docs/map-editor-design.md.
pub struct MapSpawn {
    /// World position of every wall tile the map placed - already spawned
    /// as live `Obstacle` entities by the time this returns. `Game::init`
    /// folds these into its own `obstacle_positions` *before* rolling enemy
    /// spawn points, so the existing enemy-placement clearance check (which
    /// already tests against `obstacle_positions`) makes enemies avoid a
    /// map's walls for free, with no separate map-aware check needed.
    pub obstacle_positions: Vec<Position>,
    /// World position of every cell the map explicitly marked as road (not
    /// including the "road under every wall tile" `Game::init` already
    /// paints for free from `obstacle_positions` - this is only the
    /// standalone road cells, e.g. a path with no wall on it).
    pub road_cells: Vec<Position>,
    /// The map's one frog placement, if any - `None` means the map didn't
    /// place a frog, in which case `Game::init` falls back to a random
    /// near-center roll (every round needs exactly one live frog for the
    /// protect-objective mechanic to mean anything).
    pub frog_pos: Option<Position>,
    /// Every health/ammo pickup slot the map placed, in map order. This is
    /// now the *only* source of pickups for the round - an empty list means
    /// the round simply has none, there's no random fallback any more (see
    /// "Pickups: fixed spawn slots" in docs/map-editor-design.md).
    pub pickup_slots: Vec<(Position, PickupKind)>,
}

/// Spawn every cell `map` defines as a live entity - walls as `Obstacle`s
/// (physics body included), road/frog/pickup cells as plain positions for
/// `Game::init` to act on. Called once from `Game::init`, right after the
/// player is spawned - see `MapSpawn`'s field docs for exactly how each
/// part gets folded back into the rest of round setup. This is the *only*
/// source of static terrain for a round now - there is no procedural
/// fallback (see this module's own doc comment).
///
/// Every wall of a given material shares one rolled cosmetic variant across
/// the whole map (`Material::variants`) - a consistent-build convention, one
/// roll per material rather than per tile, so a map's walls of the same
/// material don't visually mismatch each other. `Wood`'s `flammable` still
/// rolls per tile, same as everywhere else it's used.
pub fn spawn_from_map(
    physics: &mut Physics,
    world: &mut hecs::World,
    rng: &mut SmallRng,
    map: &MapFile,
    obstacle_half_extent: f32,
) -> MapSpawn {
    let material_variant: HashMap<Material, i32> = MATERIALS
        .iter()
        .map(|&m| (m, rng.random_range(0..m.variants())))
        .collect();

    // Every wall cell the map defines, in the same `(col, row)` grid space
    // `iter_cells` already yields - so `tile_hull_half_extent` can widen a
    // hand-placed tile's collider to meet a hand-placed neighbor's, closing
    // the same seam a procedurally-scattered structure's tiles get closed
    // for (see that function's doc comment).
    let wall_cells: HashSet<(i32, i32)> = map
        .iter_cells()
        .filter(|(_, _, obj)| matches!(obj, CellObject::Wall { .. }))
        .map(|(col, row, _)| (col, row))
        .collect();

    let mut obstacle_positions = Vec::new();
    let mut road_cells = Vec::new();
    let mut frog_pos = None;
    let mut pickup_slots = Vec::new();

    for (col, row, obj) in map.iter_cells() {
        let pos = cell_to_world(col, row);
        match *obj {
            CellObject::Wall { material } => {
                let variant = material_variant[&material];
                let max_health = material.max_health();
                let flammable =
                    material == Material::Wood && rng.random_bool(tuning().wood_flammable_chance);
                let body = physics.spawn_static(
                    pos,
                    tile_hull_half_extent(&wall_cells, col, row, obstacle_half_extent),
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
            CellObject::Road => road_cells.push(pos),
            CellObject::Frog => frog_pos = Some(pos),
            // The player's start position is read directly from
            // `self.map.start_cell()` in `Game::init`, before this function
            // runs (the player is spawned before map terrain is) - nothing
            // here needs to track it.
            CellObject::Start => {}
            CellObject::Pickup { pickup } => pickup_slots.push((pos, pickup)),
        }
    }

    MapSpawn { obstacle_positions, road_cells, frog_pos, pickup_slots }
}

#[cfg(test)]
mod tile_seam_tests {
    use super::*;

    #[test]
    fn lone_tile_keeps_the_normal_shrunk_half_extent() {
        let cells: HashSet<(i32, i32)> = [(5, 5)].into_iter().collect();
        let half = tile_hull_half_extent(&cells, 5, 5, 12.0);
        assert_eq!(half, Position::new(12.0, 12.0));
    }

    #[test]
    fn middle_of_a_horizontal_run_widens_only_x() {
        let cells: HashSet<(i32, i32)> = [(4, 5), (5, 5), (6, 5)].into_iter().collect();
        let half = tile_hull_half_extent(&cells, 5, 5, 12.0);
        assert_eq!(half, Position::new(OBSTACLE_GRID_SIZE * 0.5, 12.0));
    }

    #[test]
    fn end_of_a_horizontal_run_still_widens_x() {
        // The outward-facing side has no neighbor either, but widening is
        // per-axis, not per-side - see `tile_hull_half_extent`'s own doc
        // comment on why a symmetric widen (both sides of the axis) is the
        // deliberate simplification here.
        let cells: HashSet<(i32, i32)> = [(5, 5), (6, 5)].into_iter().collect();
        let half = tile_hull_half_extent(&cells, 5, 5, 12.0);
        assert_eq!(half, Position::new(OBSTACLE_GRID_SIZE * 0.5, 12.0));
    }

    #[test]
    fn corner_of_an_l_shape_widens_both_axes() {
        let cells: HashSet<(i32, i32)> = [(5, 5), (6, 5), (5, 6)].into_iter().collect();
        let half = tile_hull_half_extent(&cells, 5, 5, 12.0);
        assert_eq!(
            half,
            Position::new(OBSTACLE_GRID_SIZE * 0.5, OBSTACLE_GRID_SIZE * 0.5)
        );
    }

    #[test]
    fn two_adjacent_tiles_widened_extents_meet_with_zero_gap() {
        // The actual bug this exists to fix: two touching tiles' widened
        // half-extents must sum to exactly one grid pitch, so their
        // colliders meet with no seam a shell can slip through.
        let cells: HashSet<(i32, i32)> = [(5, 5), (6, 5)].into_iter().collect();
        let left = tile_hull_half_extent(&cells, 5, 5, 12.0);
        let right = tile_hull_half_extent(&cells, 6, 5, 12.0);
        assert_eq!(left.x + right.x, OBSTACLE_GRID_SIZE);
    }

    #[test]
    fn pos_to_cell_inverts_grid_to_pos() {
        let pos = Position::new(7.0 * OBSTACLE_GRID_SIZE, -3.0 * OBSTACLE_GRID_SIZE);
        assert_eq!(pos_to_cell(pos), (7, -3));
    }
}
