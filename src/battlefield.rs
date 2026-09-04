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
use crate::tank::{Dir, Tank};
use crate::{
    OBSTACLE_CLEAR,
    OBSTACLE_GRID_SIZE,
    PATHFIND_CELL_SIZE,
    Position,
    TANK_HULL_BBOX_BY_ROW,
    TANK_MOVE_BBOX_FRACTION,
    WALL_THICKNESS,
};

/// Cap on rejection-sampling attempts for a single enemy/frog spawn
/// position (see `sample_clear_position`) - keeps `Game::init` provably
/// bounded regardless of how unlucky the RNG gets, rather than an unbounded
/// `while` loop that could in principle stall for a long time. This
/// project's web build deliberately skips `-sASYNCIFY=1` (see
/// `game_loop::run`'s doc comment), so `Game::init` runs as one
/// uninterruptible synchronous call from the browser's perspective - a
/// real stall here freezes the whole tab until it returns. Hitting the cap
/// is not an error: the sampler reports it (`None`) and the caller falls
/// back to a snapped-to-open-cell placement instead of looping further.
const PLACEMENT_MAX_ATTEMPTS: u32 = 2000;

/// Sample a random position within `margin_min..(width-margin_min)` x
/// `margin_min..(height-margin_min)`, retrying until `valid` accepts one.
/// `None` once `PLACEMENT_MAX_ATTEMPTS` is reached without an accepted
/// sample - see that constant's doc comment for why this is capped rather
/// than an unbounded loop. A caller must then place the entity some other
/// way (`Game::init` snaps a fresh band sample to `Grid::nearest_open`);
/// the last rejected sample is deliberately not handed back, because a
/// rejected sample can sit inside a wall. Used by the enemy and
/// frog-fallback placement loops in `Game::init`.
pub fn sample_clear_position(
    rng: &mut SmallRng,
    width: f32,
    height: f32,
    margin_min: f32,
    mut valid: impl FnMut(Position) -> bool,
) -> Option<Position> {
    for _ in 0..PLACEMENT_MAX_ATTEMPTS {
        let pos = Position::new(
            rng.random_range(margin_min..(width - margin_min)),
            rng.random_range(margin_min..(height - margin_min)),
        );
        if valid(pos) {
            return Some(pos);
        }
    }
    None
}

/// Whether `pos` is a legal enemy spawn point on its own merits (the
/// per-round terms - clearance from the other enemies already placed - are
/// the caller's): inside the border band `margin_min..=margin_max` from the
/// nearest edge, at least `player_clear` from the player's start, in a
/// `Grid::usable` cell of `grid` (the nav grid built from the round's
/// walls, so the tank can be routed out of it), and with a worst-case
/// tank's box (`max_tank_avoidance_radius` per side) clear of every wall
/// tile in `wall_positions` (each taken at a full half-cell, the widest
/// seam-closed collider a tile can have). The usable-cell term alone is
/// not enough for that last guarantee: the grid pads walls from the
/// cell *center*, while a sample lands anywhere in a 48px cell. This is
/// the one definition `Game::init`'s sampler and `maplint`'s spawn-band
/// capacity count both use, so the linter's "legal spawn cells" is
/// exactly what the sampler accepts.
#[allow(clippy::too_many_arguments)] // one predicate, its terms spelled out
pub fn enemy_spawn_legal(
    pos: Position,
    width: f32,
    height: f32,
    margin_min: f32,
    margin_max: f32,
    player_pos: Position,
    player_clear: f32,
    grid: &Grid,
    wall_positions: &[Position],
) -> bool {
    let in_domain = pos.x >= margin_min && pos.x <= width - margin_min && pos.y >= margin_min && pos.y <= height - margin_min;
    let border_dist = pos.x.min(width - pos.x).min(pos.y).min(height - pos.y);
    let separation = OBSTACLE_GRID_SIZE * 0.5 + max_tank_avoidance_radius();
    in_domain
        && border_dist <= margin_max
        && pos.distance_to(player_pos) >= player_clear
        && grid.usable(pos)
        && wall_positions
            .iter()
            .all(|w| (pos.x - w.x).abs() >= separation || (pos.y - w.y).abs() >= separation)
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

/// Rebuild the nav grid from the finished obstacle layout (see
/// `pathfind::Grid`) and teleport any enemy whose spawn point is not
/// `Grid::usable` - sitting in a blocked cell (inside a wall's padded
/// footprint) or boxed in (every cardinal neighbor blocked, so `Ai::wander`
/// could never route it anywhere) - to the nearest cell that is.
///
/// The spawn sampler already only accepts usable cells, so this catches
/// the cases it can't: a round whose sampler hit its attempt cap and fell
/// back to a snapped placement near other tanks, and the cumulative effect
/// of several walls each individually clear of a spawn but collectively
/// sealing it in. A check against the finished layout is the only way to
/// catch the latter, which is why this runs once, after everything else
/// this round is already down.
///
/// The player is deliberately excluded (`.with::<&Ai>()` - only enemies have
/// one) - a hand-authored map can still enclose the player's fixed spawn
/// point on purpose, and relocating it would silently override that
/// design choice rather than respect it.
pub fn relocate_unusable_spawns(physics: &mut Physics, world: &mut hecs::World, width: f32, height: f32) {
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
    // included, so two stranded tanks near each other never both land on
    // the exact same nearest cell.
    let mut positions: Vec<(hecs::Entity, Position)> = world
        .query::<(hecs::Entity, &Tank)>()
        .with::<&Ai>()
        .iter()
        .map(|(entity, tank)| (entity, tank.position))
        .collect();
    for i in 0..positions.len() {
        let (entity, pos) = positions[i];
        if grid.usable(pos) {
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
    /// World position of every solid tile the map placed - walls and props,
    /// already spawned as live `Obstacle` entities by the time this
    /// returns. `Game::init` folds these into its own `obstacle_positions`
    /// *before* rolling enemy spawn points, so the existing enemy-placement
    /// clearance check (which already tests against `obstacle_positions`)
    /// makes enemies avoid a map's terrain for free, with no separate
    /// map-aware check needed.
    pub obstacle_positions: Vec<Position>,
    /// The wall tiles only (a subset of `obstacle_positions`): road is
    /// painted under these; props stand on whatever ground is there.
    pub wall_positions: Vec<Position>,
    /// World position of the map's `enemy_frog` cell, if any (Hunt mission).
    pub enemy_frog_pos: Option<Position>,
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
/// rolls per tile, same as everywhere else it's used. Props (sandbag,
/// barrel, fence) roll their variant per tile instead, at the tile's turn
/// in the sorted cell walk, so a line of sandbags varies and a map without
/// props draws exactly the RNG it always did.
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

    // Every solid cell the map defines (walls and props), in the same
    // `(col, row)` grid space `iter_cells` already yields - so
    // `tile_hull_half_extent` can widen a hand-placed tile's collider to
    // meet a hand-placed neighbor's, closing the seam between them (see
    // that function's doc comment). Props widen like walls so a line of
    // sandbags has no gap a shell could thread.
    let solid_cells: HashSet<(i32, i32)> = map
        .iter_cells()
        .filter(|(_, _, obj)| obj.is_solid())
        .map(|(col, row, _)| (col, row))
        .collect();

    let mut obstacle_positions = Vec::new();
    let mut wall_positions = Vec::new();
    let mut road_cells = Vec::new();
    let mut frog_pos = None;
    let mut enemy_frog_pos = None;
    let mut pickup_slots = Vec::new();

    for (col, row, obj) in map.iter_cells() {
        let pos = cell_to_world(col, row);
        match *obj {
            CellObject::Wall { material } => {
                let variant = material_variant[&material];
                let flammable =
                    material == Material::Wood && rng.random_bool(tuning().wood_flammable_chance);
                let body = physics.spawn_static(
                    pos,
                    tile_hull_half_extent(&solid_cells, col, row, obstacle_half_extent),
                );
                obstacle_positions.push(pos);
                wall_positions.push(pos);
                world.spawn((Obstacle::new(material, variant, pos, flammable, body),));
            }
            CellObject::Sandbag | CellObject::Barrel | CellObject::Fence => {
                let material = obj.material().expect("prop cells spawn a material");
                let variant = rng.random_range(0..material.variants());
                let body = physics.spawn_static(
                    pos,
                    tile_hull_half_extent(&solid_cells, col, row, obstacle_half_extent),
                );
                obstacle_positions.push(pos);
                world.spawn((Obstacle::new(material, variant, pos, false, body),));
            }
            CellObject::Road => road_cells.push(pos),
            CellObject::Frog => frog_pos = Some(pos),
            // The player's start position is read directly from
            // `self.map.start_cell()` in `Game::init`, before this function
            // runs (the player is spawned before map terrain is) - nothing
            // here needs to track it.
            CellObject::Start => {}
            CellObject::Pickup { pickup } => pickup_slots.push((pos, pickup)),
            CellObject::EnemyFrog => enemy_frog_pos = Some(pos),
            // Gates are read straight from `MapFile::gate_cells` by the
            // wave scheduler; nothing is spawned for them.
            CellObject::Gate => {}
        }
    }

    MapSpawn { obstacle_positions, wall_positions, road_cells, frog_pos, enemy_frog_pos, pickup_slots }
}

/// One entry lane for a wave tank (docs/maps-to-levels.md "Gates and
/// roll-in"): an edge nav cell whose lane of `inward` cells toward the
/// interior is open. A tank materialises at `outside`, rolls in
/// kinematically (no physics body) along the lane and gets its body at
/// `inside`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gate {
    /// Which battlefield edge the gate sits on: `Dir::Up` is the top edge
    /// (row 0), `Dir::Down` the bottom, `Dir::Left`/`Dir::Right` the two
    /// sides. The tank drives the opposite way.
    pub edge: Dir,
    /// The edge nav cell (col, row).
    pub cell: (usize, usize),
    /// One tank length beyond the battlefield edge, in line with the lane.
    pub outside: Position,
    /// The centre of the innermost open lane cell, where the body spawns.
    pub inside: Position,
}

impl Gate {
    /// The direction a tank entering through this gate drives.
    pub fn heading(&self) -> Dir {
        match self.edge {
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
        }
    }
}

/// Every gate the live nav grid offers, sorted by edge (`Dir::ALL` order),
/// then column, then row. A lane is an edge cell plus the next `inward - 1`
/// cells toward the interior, all `Grid::usable`, whose centre line keeps
/// a worst-case tank (`max_tank_avoidance_radius`) clear of the two
/// boundary walls it runs between - the grid itself does not mark the
/// boundary, so the first and last columns/rows would otherwise pass as
/// lanes that put a tank half inside a wall. `width`/`height` are the real
/// battlefield size (the grid's last column may overhang it). A gate whose
/// `inside` point is closer than `min_dist` to any `avoid` position (the
/// player, the player's frog) is left out.
pub fn gate_candidates(grid: &Grid, width: f32, height: f32, avoid: &[Position], min_dist: f32, inward: usize) -> Vec<Gate> {
    let (cols, rows, cell) = grid.dims();
    let inward = inward.max(1);
    let clear = max_tank_avoidance_radius();
    let tank_len = Tank::default().size();
    let centre = |i: usize| (i as f32 + 0.5) * cell;
    let mut gates = Vec::new();
    for edge in Dir::ALL {
        // Cell counts across the edge and along the lane, and the real
        // extent across (the boundary walls the lane runs between).
        let (across, along, span) = match edge {
            Dir::Up | Dir::Down => (cols, rows, width),
            Dir::Left | Dir::Right => (rows, cols, height),
        };
        if inward > along {
            continue;
        }
        for a in 0..across {
            let lane_centre = centre(a);
            if lane_centre < clear || span - lane_centre < clear {
                continue;
            }
            // Lane cell `k` steps in from the edge.
            let lane_cell = |k: usize| -> (usize, usize) {
                let depth = match edge {
                    Dir::Up | Dir::Left => k,
                    Dir::Down | Dir::Right => along - 1 - k,
                };
                match edge {
                    Dir::Up | Dir::Down => (a, depth),
                    Dir::Left | Dir::Right => (depth, a),
                }
            };
            let open = (0..inward).all(|k| {
                let (c, r) = lane_cell(k);
                grid.usable(Position::new(centre(c), centre(r)))
            });
            if !open {
                continue;
            }
            let (ic, ir) = lane_cell(inward - 1);
            let inside = Position::new(centre(ic), centre(ir));
            if avoid.iter().any(|p| p.distance_to(inside) < min_dist) {
                continue;
            }
            let outside = match edge {
                Dir::Up => Position::new(lane_centre, -tank_len),
                Dir::Down => Position::new(lane_centre, height + tank_len),
                Dir::Left => Position::new(-tank_len, lane_centre),
                Dir::Right => Position::new(width + tank_len, lane_centre),
            };
            gates.push(Gate { edge, cell: lane_cell(0), outside, inside });
        }
    }
    gates.sort_by_key(|g| (g.edge.index(), g.cell.0, g.cell.1));
    gates
}

/// The gates a map placed by hand (`MapFile::gate_cells`, in map cells of
/// `OBSTACLE_GRID_SIZE` px), each turned into the nav-grid lane of the edge
/// it touches - a corner cell takes the side edge. Cells not on an edge,
/// and lanes not open per `gate_candidates`' rules, are skipped (the map
/// linter reports them). Output order follows the input, duplicates
/// dropped.
pub fn gates_from_cells(grid: &Grid, width: f32, height: f32, cells: &[(i32, i32)], inward: usize) -> Vec<Gate> {
    let (cols, rows, cell) = grid.dims();
    let last_col = ((width / OBSTACLE_GRID_SIZE).ceil() as i32 - 1).max(0);
    let last_row = ((height / OBSTACLE_GRID_SIZE).ceil() as i32 - 1).max(0);
    let all = gate_candidates(grid, width, height, &[], 0.0, inward);
    let mut gates: Vec<Gate> = Vec::new();
    for &(col, row) in cells {
        let edge = if col <= 0 {
            Dir::Left
        } else if col >= last_col {
            Dir::Right
        } else if row <= 0 {
            Dir::Up
        } else if row >= last_row {
            Dir::Down
        } else {
            continue;
        };
        let world = cell_to_world(col, row);
        let nav_col = ((world.x / cell) as usize).min(cols.saturating_sub(1));
        let nav_row = ((world.y / cell) as usize).min(rows.saturating_sub(1));
        let found = all.iter().find(|g| {
            g.edge == edge
                && match edge {
                    Dir::Up | Dir::Down => g.cell.0 == nav_col,
                    Dir::Left | Dir::Right => g.cell.1 == nav_row,
                }
        });
        if let Some(g) = found {
            if !gates.contains(g) {
                gates.push(*g);
            }
        }
    }
    gates
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    /// A 6x6 grid of 100px cells (lane centres 50px+ from the boundary,
    /// past `max_tank_avoidance_radius`) with one wall tile at (2,1),
    /// which blocks the lane down from the top edge's column 2.
    fn grid() -> Grid {
        Grid::build(600.0, 600.0, 100.0, 0.0, [(Position::new(250.0, 150.0), 20.0)].into_iter())
    }

    #[test]
    fn open_edge_lanes_become_gates_and_a_blocked_lane_does_not() {
        let gates = gate_candidates(&grid(), 600.0, 600.0, &[], 0.0, 3);
        let top: Vec<usize> = gates.iter().filter(|g| g.edge == Dir::Up).map(|g| g.cell.0).collect();
        assert_eq!(top, vec![0, 1, 3, 4, 5], "column 2's lane runs into the wall at (2,1)");
        let g = gates.iter().find(|g| g.edge == Dir::Up && g.cell.0 == 1).unwrap();
        assert_eq!(g.outside, Position::new(150.0, -Tank::default().size()));
        assert_eq!(g.inside, Position::new(150.0, 250.0), "innermost of the three lane cells");
        assert_eq!(g.heading(), Dir::Down);
        let right = gates.iter().find(|g| g.edge == Dir::Right && g.cell.1 == 4).unwrap();
        assert_eq!(right.cell, (5, 4));
        assert_eq!(right.outside, Position::new(600.0 + Tank::default().size(), 450.0));
        assert_eq!(right.inside, Position::new(350.0, 450.0));
        let mut sorted = gates.clone();
        sorted.sort_by_key(|g| (g.edge.index(), g.cell.0, g.cell.1));
        assert_eq!(gates, sorted, "deterministic order");
    }

    #[test]
    fn gates_too_close_to_an_avoided_position_are_skipped() {
        let all = gate_candidates(&grid(), 600.0, 600.0, &[], 0.0, 2);
        let near = gate_candidates(&grid(), 600.0, 600.0, &[Position::new(150.0, 150.0)], 120.0, 2);
        assert!(near.len() < all.len());
        assert!(near.iter().all(|g| g.inside.distance_to(Position::new(150.0, 150.0)) >= 120.0));
    }

    #[test]
    fn a_lane_hugging_the_boundary_is_not_a_gate() {
        // 48px cells: column 0's centre is 24px from the left wall, inside
        // a worst-case tank's radius, so no top/bottom gate uses it.
        let grid = Grid::build(480.0, 480.0, 48.0, 0.0, std::iter::empty());
        let gates = gate_candidates(&grid, 480.0, 480.0, &[], 0.0, 3);
        assert!(gates.iter().all(|g| g.cell != (0, 0) && g.cell != (0, 9)));
        assert!(gates.iter().any(|g| g.edge == Dir::Up && g.cell.0 == 1));
    }

    #[test]
    fn explicit_cells_map_onto_their_edge_lane_and_interior_cells_are_dropped() {
        let grid = grid();
        // Map cells are 32px: (0, 14) is the left edge at y=448 -> nav row 4;
        // (9, 0) is the top edge at x=288 -> nav column 2, whose lane is
        // blocked; (7, 7) is interior; the repeat is dropped.
        let gates = gates_from_cells(&grid, 600.0, 600.0, &[(0, 14), (9, 0), (7, 7), (0, 14)], 3);
        assert_eq!(gates.len(), 1, "{gates:?}");
        assert_eq!(gates[0].edge, Dir::Left);
        assert_eq!(gates[0].cell, (0, 4));
    }
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
