//! A small, coarse grid-based A* pathfinder for AI movement around static
//! obstacles (see `obstacle.rs`). Deliberately cardinal-only and grid-based
//! rather than a general navmesh, matching the game's 4-direction movement
//! convention (see CLAUDE.md) - a tank never needs to travel anywhere but
//! along grid-aligned steps anyway.
//!
//! Kept game-agnostic in spirit (like `bt.rs`): this module only knows about
//! a rectangular grid of blocked/open cells, not about tanks, obstacles, or
//! any other game type. `Game::update` builds a fresh `Grid` each frame from
//! the current obstacle layout and hands it down through `Ai::think`, in
//! keeping with `docs/physics-engine-design.md`'s "AI decoupling" principle:
//! the AI reasons over lightweight snapshots, never the physics world or ECS
//! directly.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};

use crate::Position;

/// Connected-component labels over a `Grid`'s open cells - see
/// `Grid::components`.
pub struct Components {
    /// Per cell: 0 for blocked, otherwise a component id starting at 1.
    label: Vec<u32>,
}

impl Components {
    /// True if a cardinal-step route exists between `from` and `to` on
    /// `grid` (the grid these labels were built from) - equivalent to
    /// `grid.next_step(from, to).is_some() || grid.same_cell(from, to)`,
    /// including A*'s rule that the start and goal cells count as open.
    pub fn connected(&self, grid: &Grid, from: Position, to: Position) -> bool {
        if grid.same_cell(from, to) {
            return true;
        }
        let from_labels: Vec<u32> = grid.component_labels(self, from).collect();
        grid.component_labels(self, to).any(|l| from_labels.contains(&l))
    }
}

/// A coarse occupancy grid over a rectangular area, marking which cells are
/// blocked (a static obstacle occupies them, plus a clearance margin - see
/// `build`) versus open.
pub struct Grid {
    cell_size: f32,
    cols: usize,
    rows: usize,
    blocked: Vec<bool>,
}

impl Grid {
    /// Build a grid covering `0..width, 0..height` in `cell_size`-px cells.
    /// `obstacles` is every blocking shape as (center, half-extent); each
    /// obstacle blocks every cell whose *center* lies within
    /// `half_extent + margin` of its own center (in both axes) - `margin`
    /// should be a tank's worst-case half-hull, so pathfinding never
    /// routes through a gap too narrow for a tank to actually fit through.
    ///
    /// Cell-center occupancy, not any-overlap, on purpose: `next_step`
    /// steers tanks *at cell centers* (`center_of`), so "can a tank's
    /// center stand at this cell's center" is exactly the question the
    /// router needs answered, and a center outside `reach` already
    /// guarantees the full `margin` of hull clearance from the obstacle's
    /// edge. The earlier any-overlap rule ("blocked if the inflated box
    /// touches any part of the cell") stacked a second, hidden margin of
    /// up to a whole `cell_size` per side on top of the real one: a
    /// physically drivable corridor needed ~`2*margin + obstacle +
    /// 2*cell_size` of gap before a single open cell survived
    /// rasterization, which on the shipped default map sealed most of the
    /// battlefield into disconnected pockets - engagement slots all
    /// failed their reachability check, `Ai::steer` fell back to `wander`
    /// nearly everywhere, and pocketed tanks visibly spun in place
    /// (found via the probe harness's `spin`/`churn` sweeps and a
    /// frame-by-frame slot trace; see docs/gameplay-verification-design.md).
    pub fn build(
        width: f32,
        height: f32,
        cell_size: f32,
        margin: f32,
        obstacles: impl Iterator<Item = (Position, f32)>,
    ) -> Self {
        let cols = ((width / cell_size).ceil() as usize).max(1);
        let rows = ((height / cell_size).ceil() as usize).max(1);
        let mut blocked = vec![false; cols * rows];

        for (center, half_extent) in obstacles {
            let reach = half_extent + margin;
            // Lowest/highest cell index whose center (at `(i + 0.5) *
            // cell_size`) falls inside `[center-reach, center+reach]`:
            // solving `(i + 0.5) * cell_size >= center - reach` for the
            // lower bound and the mirror for the upper. A center landing
            // exactly on the boundary counts as blocked (it would leave
            // exactly zero hull clearance).
            let min_col_raw = ((center.x - reach) / cell_size - 0.5).ceil() as isize;
            let max_col_raw = ((center.x + reach) / cell_size - 0.5).floor() as isize;
            let min_row_raw = ((center.y - reach) / cell_size - 0.5).ceil() as isize;
            let max_row_raw = ((center.y + reach) / cell_size - 0.5).floor() as isize;
            // Entirely outside the grid on at least one axis - no cell to mark.
            if max_col_raw < 0
                || min_col_raw >= cols as isize
                || max_row_raw < 0
                || min_row_raw >= rows as isize
            {
                continue;
            }
            let min_col = min_col_raw.clamp(0, cols as isize - 1) as usize;
            let max_col = max_col_raw.clamp(0, cols as isize - 1) as usize;
            let min_row = min_row_raw.clamp(0, rows as isize - 1) as usize;
            let max_row = max_row_raw.clamp(0, rows as isize - 1) as usize;
            for row in min_row..=max_row {
                for col in min_col..=max_col {
                    blocked[row * cols + col] = true;
                }
            }
        }

        Self {
            cell_size,
            cols,
            rows,
            blocked,
        }
    }

    fn cell_of(&self, p: Position) -> (usize, usize) {
        let col = ((p.x / self.cell_size) as isize).clamp(0, self.cols as isize - 1) as usize;
        let row = ((p.y / self.cell_size) as isize).clamp(0, self.rows as isize - 1) as usize;
        (col, row)
    }

    fn center_of(&self, cell: (usize, usize)) -> Position {
        Position::new(
            (cell.0 as f32 + 0.5) * self.cell_size,
            (cell.1 as f32 + 0.5) * self.cell_size,
        )
    }

    fn blocked_at(&self, cell: (usize, usize)) -> bool {
        self.blocked[cell.1 * self.cols + cell.0]
    }

    /// (columns, rows, cell size in px) - for tooling that draws or prints
    /// the grid.
    pub fn dims(&self) -> (usize, usize, f32) {
        (self.cols, self.rows, self.cell_size)
    }

    /// Whether cell (`col`, `row`) is blocked; anything off the grid is.
    pub fn is_blocked(&self, col: usize, row: usize) -> bool {
        col >= self.cols || row >= self.rows || self.blocked_at((col, row))
    }

    /// One line per row, top row first: `#` blocked, `.` open.
    pub fn ascii(&self) -> String {
        let mut out = String::with_capacity((self.cols + 1) * self.rows);
        for row in 0..self.rows {
            for col in 0..self.cols {
                out.push(if self.blocked_at((col, row)) { '#' } else { '.' });
            }
            out.push('\n');
        }
        out
    }

    /// True if `from` and `to` already fall in the same grid cell - the
    /// other reason `next_step` returns `None` besides genuine
    /// unreachability (see that method's `start == goal` check). Lets a
    /// caller (see `Ai::steer`) tell "already arrived, nothing left to
    /// route" apart from "no path exists at all" - `next_step`'s `None`
    /// alone conflates the two, and callers that treat every `None` as
    /// "unreachable" would otherwise misfire constantly at close range
    /// (found via the probe harness: at PATHFIND_CELL_SIZE=48px, closing in
    /// for an attack routinely puts the two tanks in the same cell well
    /// before they're actually touching).
    pub fn same_cell(&self, from: Position, to: Position) -> bool {
        self.cell_of(from) == self.cell_of(to)
    }

    /// True if stepping one cell forward from `from` along `dir` (a unit
    /// direction vector, e.g. `Dir::vec()`) would land in a blocked cell -
    /// or off the grid entirely, which counts as blocked too (the real
    /// boundary walls stop a tank from ever actually getting there, so
    /// pathfinding shouldn't route through the gap either). Lets a caller
    /// check "does my current heading walk into a known obstacle" without a
    /// full `next_step` pathfind call - see `Ai::steer`'s
    /// obstacle-vs-commitment override.
    ///
    /// Steps by *grid cell*, not by a flat `cell_size` in world space from
    /// `from`'s own exact pixel position: `from` is rarely sitting exactly
    /// on its cell's center, so a fixed-distance world-space probe can
    /// under- or overshoot into a different cell than the one directly
    /// adjacent to `from`'s own cell - the same cell `next_step`'s A*
    /// reasons about when it calls a neighbor "blocked" or not. Near a
    /// corner, that mismatch could make this function and `next_step`
    /// disagree about whether the very same heading is safe, each tick
    /// re-litigating the disagreement - `Ai::steer`'s obstacle-ahead
    /// override (which leans on this) held a heading for only 0.1s at a
    /// time in exactly that situation instead of the usual longer,
    /// jitter-resistant hold, reading as the tank's heading rapidly
    /// flip-flopping in place near obstacle corners specifically (found via
    /// the probe harness's per-commit trace).
    pub fn blocked_ahead(&self, from: Position, dir: Position) -> bool {
        let (col, row) = self.cell_of(from);
        let nc = col as i32 + dir.x.round() as i32;
        let nr = row as i32 + dir.y.round() as i32;
        if nc < 0 || nr < 0 || nc as usize >= self.cols || nr as usize >= self.rows {
            return true;
        }
        self.blocked_at((nc as usize, nr as usize))
    }

    fn neighbors_all_blocked(&self, cell: (usize, usize)) -> bool {
        self.neighbors(cell).all(|n| self.blocked_at(n))
    }

    /// True if every in-bounds cardinal neighbor of `from`'s cell is
    /// blocked - i.e. there is no first step `next_step` could ever return
    /// from here, to *any* target, not just whichever one it was actually
    /// asked about. A tank can end up here two ways: several
    /// independently-placed obstacles each individually respecting their
    /// own clearance from it, but collectively still sealing every
    /// direction out (see `battlefield::relocate_unusable_spawns`, which
    /// checks every tank against this once at round init and relocates any
    /// that fail); or getting rammed/knocked into a tight pocket mid-round.
    /// Either way, resampling a *different* target (see `Ai::wander`)
    /// can't help - every candidate fails the same way, for the same
    /// reason. Without this check, that meant re-rolling a fresh random
    /// waypoint every single frame while boxed in: each one pointed a
    /// different direction, so the tank visibly spun in place instead of
    /// just sitting still (found via the probe harness's per-commit trace:
    /// the same frozen position, a brand new waypoint on nearly every
    /// frame, heading flipping every 0.1-0.4s).
    pub fn boxed_in(&self, from: Position) -> bool {
        self.neighbors_all_blocked(self.cell_of(from))
    }

    /// True if a tank standing at `at` can actually be routed from there:
    /// its own cell is open (it is not sitting inside an obstacle's
    /// footprint) and it is not `boxed_in`. This is the spawn-legality
    /// test `battlefield::enemy_spawn_legal` and
    /// `battlefield::relocate_unusable_spawns` share - a spawn that fails
    /// it is either physically inside terrain or sealed in, and either
    /// way needs relocating, so the two tests stay one check.
    pub fn usable(&self, at: Position) -> bool {
        self.usable_cell(self.cell_of(at))
    }

    fn usable_cell(&self, cell: (usize, usize)) -> bool {
        !self.blocked_at(cell) && !self.neighbors_all_blocked(cell)
    }

    /// The center of the nearest cell to `from` that is both unblocked and
    /// not itself boxed in (see `boxed_in`) - i.e. a genuinely usable spot,
    /// not just a technically-open single cell surrounded by blocked ones
    /// (which would just relocate the same problem one cell over) - and at
    /// least `avoid_clear` from every position in `avoid`, so relocating
    /// several boxed-in tanks in the same small area doesn't send two of
    /// them to the exact same nearest cell (found via the probe harness:
    /// two enemies landing at literally identical coordinates, `dist=0.0`,
    /// since two independent `nearest_open` calls with no `avoid` had no
    /// way to know about each other). BFS outward cardinally from `from`'s
    /// own cell, so "nearest" means fewest grid steps, not raw pixel
    /// distance - used once per flagged tank at round init (see
    /// `battlefield::relocate_unusable_spawns`), not a hot path. Falls back
    /// to `from` itself if the entire grid turns out unusable (never
    /// actually hit in practice - a real obstacle layout leaves most of the
    /// battlefield open - but a plain fallback beats a panic over a
    /// pathological map).
    pub fn nearest_open(&self, from: Position, avoid: &[Position], avoid_clear: f32) -> Position {
        let start = self.cell_of(from);
        let idx = |c: (usize, usize)| c.1 * self.cols + c.0;
        let mut visited = vec![false; self.cols * self.rows];
        visited[idx(start)] = true;
        let mut queue = VecDeque::new();
        queue.push_back(start);
        while let Some(cell) = queue.pop_front() {
            if self.usable_cell(cell) {
                let center = self.center_of(cell);
                if avoid.iter().all(|&p| center.distance_to(p) >= avoid_clear) {
                    return center;
                }
            }
            for next in self.neighbors(cell) {
                if !visited[idx(next)] {
                    visited[idx(next)] = true;
                    queue.push_back(next);
                }
            }
        }
        from
    }

    /// Label every open cell with its connected component (4-neighbour
    /// flood fill) so `Components::connected` answers "does a route exist
    /// between these two points" in O(1) - the same answer `next_step`
    /// would give, without running A* per query. Build once per frame and
    /// query as often as needed (engagement-slot validation asks up to 16
    /// times per enemy).
    pub fn components(&self) -> Components {
        let mut label = vec![0u32; self.cols * self.rows];
        let mut next = 1u32;
        let mut stack = Vec::new();
        for row in 0..self.rows {
            for col in 0..self.cols {
                let idx = row * self.cols + col;
                if self.blocked[idx] || label[idx] != 0 {
                    continue;
                }
                label[idx] = next;
                stack.push((col, row));
                while let Some(cell) = stack.pop() {
                    for n in self.neighbors(cell) {
                        let ni = n.1 * self.cols + n.0;
                        if !self.blocked[ni] && label[ni] == 0 {
                            label[ni] = next;
                            stack.push(n);
                        }
                    }
                }
                next += 1;
            }
        }
        Components { label }
    }

    /// The component labels a point can start a route from: its own cell's
    /// label if that cell is open, otherwise the labels of its open
    /// cardinal neighbours - mirroring `search`, which treats the start and
    /// goal cells as open regardless of `blocked`.
    fn component_labels(&self, comps: &Components, p: Position) -> impl Iterator<Item = u32> + '_ {
        let cell = self.cell_of(p);
        let own = if self.blocked_at(cell) { 0 } else { comps.label[cell.1 * self.cols + cell.0] };
        let labels: Vec<u32> = if own != 0 {
            vec![own]
        } else {
            self.neighbors(cell)
                .map(|n| comps.label[n.1 * self.cols + n.0])
                .filter(|&l| l != 0)
                .collect()
        };
        labels.into_iter()
    }

    fn neighbors(&self, cell: (usize, usize)) -> impl Iterator<Item = (usize, usize)> + '_ {
        let (col, row) = cell;
        let cols = self.cols;
        let rows = self.rows;
        [(0i32, -1i32), (0, 1), (-1, 0), (1, 0)]
            .into_iter()
            .filter_map(move |(dc, dr)| {
                let nc = col as i32 + dc;
                let nr = row as i32 + dr;
                if nc < 0 || nr < 0 || nc as usize >= cols || nr as usize >= rows {
                    None
                } else {
                    Some((nc as usize, nr as usize))
                }
            })
    }

    /// Find a cardinal-step path from `from` to `to` and return the
    /// world-space center of the *first* cell to move into - the caller
    /// (`Ai::steer`) turns that into a heading exactly like it would for any
    /// other target. Returns `None` if `from` and `to` already share a cell
    /// (nothing to route around) or no path exists at all (fully enclosed
    /// target) - the caller falls back to steering straight at `to` either
    /// way. The start and goal cells are always treated as open regardless
    /// of `blocked`, so standing next to (or aiming at a point inside) an
    /// obstacle's margin never fails pathfinding outright.
    pub fn next_step(&self, from: Position, to: Position) -> Option<Position> {
        let start = self.cell_of(from);
        let goal = self.cell_of(to);
        if start == goal {
            return None;
        }
        self.search(start, goal)
            .map(|hit| self.center_of(hit.first_step))
    }

    /// Shortest-path length from `from` to `to`, in grid steps (cells, not
    /// pixels - multiply by the grid's cell size for a px length): the same
    /// route `next_step` walks one step at a time, measured whole. `Some(0)`
    /// when the two points already share a cell - unlike `next_step`, which
    /// deliberately conflates "already arrived" with "unreachable" into
    /// `None` (see `same_cell`), a cost query has room to keep the two
    /// apart: `None` here always means genuinely no path. Start and goal
    /// cells are treated as open exactly like `next_step` does. Built for
    /// external tooling (the probe's path-stretch metric - see
    /// docs/gameplay-verification-design.md §5), not the per-frame AI path,
    /// so it's fine to call this once per round rather than per tick.
    pub fn path_cost(&self, from: Position, to: Position) -> Option<u32> {
        let start = self.cell_of(from);
        let goal = self.cell_of(to);
        if start == goal {
            return Some(0);
        }
        self.search(start, goal).map(|hit| hit.cost)
    }

    /// The one A* implementation both `next_step` and `path_cost` share -
    /// callers guarantee `start != goal` (each handles the same-cell case
    /// itself, with different semantics). Returns `None` when no path
    /// exists; on a hit, both the first cell to move into (what `next_step`
    /// wants) and the whole path's step count (what `path_cost` wants),
    /// since the goal-pop moment has both on hand anyway.
    fn search(&self, start: (usize, usize), goal: (usize, usize)) -> Option<SearchHit> {
        let mut open = BinaryHeap::new();
        let mut came_from = vec![None; self.cols * self.rows];
        let mut g_score = vec![f32::INFINITY; self.cols * self.rows];
        // Cells already expanded (popped and relaxed) once. Without this, a
        // cell whose g_score improves after it's already been expanded gets
        // pushed to `open` again and, once repopped, has its neighbors
        // relaxed all over again - on an open grid with many reachable
        // cells this reprocessing cascades combinatorially instead of the
        // O(cells) A* is supposed to guarantee, which is cheap enough not to
        // matter on native but was enough to stall a frame for minutes on
        // wasm's slower per-op cost (observed as a frozen, unresponsive tab
        // during web playtesting). Marking a cell closed the first time it's
        // popped bounds every cell to at most one expansion, same as
        // textbook Dijkstra/A*.
        let mut closed = vec![false; self.cols * self.rows];
        let idx = |c: (usize, usize)| c.1 * self.cols + c.0;

        g_score[idx(start)] = 0.0;
        open.push(Node {
            cell: start,
            priority: heuristic(start, goal),
        });

        while let Some(Node { cell, .. }) = open.pop() {
            if closed[idx(cell)] {
                // Stale heap entry from before this cell's last improvement.
                continue;
            }
            closed[idx(cell)] = true;

            if cell == goal {
                // Unit-cost steps summed in f32 stay exact integers (well
                // under f32's 2^24 exact-integer range on any sane grid),
                // so this cast is lossless.
                let cost = g_score[idx(cell)] as u32;
                // Walk back to the step right after `start`.
                let mut step = cell;
                while let Some(prev) = came_from[idx(step)] {
                    if prev == start {
                        return Some(SearchHit { first_step: step, cost });
                    }
                    step = prev;
                }
                return Some(SearchHit { first_step: step, cost });
            }
            for next in self.neighbors(cell) {
                if closed[idx(next)] {
                    continue;
                }
                if next != goal && self.blocked_at(next) {
                    continue;
                }
                let tentative = g_score[idx(cell)] + 1.0;
                if tentative < g_score[idx(next)] {
                    came_from[idx(next)] = Some(cell);
                    g_score[idx(next)] = tentative;
                    open.push(Node {
                        cell: next,
                        priority: tentative + heuristic(next, goal),
                    });
                }
            }
        }
        None
    }
}

/// What one successful `Grid::search` run hands back to its two public
/// wrappers: the first cell to step into (for `next_step`) and the full
/// path's step count (for `path_cost`).
struct SearchHit {
    first_step: (usize, usize),
    cost: u32,
}

/// Manhattan distance in cells - admissible since movement is 4-directional
/// with unit cost per step, so A* with this heuristic finds a shortest path.
fn heuristic(a: (usize, usize), b: (usize, usize)) -> f32 {
    (a.0 as f32 - b.0 as f32).abs() + (a.1 as f32 - b.1 as f32).abs()
}

/// One entry in the open set: a cell plus its f-score (g + heuristic).
/// `BinaryHeap` is a max-heap, so `Ord` is reversed on `priority` to pop the
/// lowest f-score first.
struct Node {
    cell: (usize, usize),
    priority: f32,
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .priority
            .partial_cmp(&self.priority)
            .unwrap_or(Ordering::Equal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_cell_returns_none() {
        let grid = Grid::build(400.0, 400.0, 40.0, 0.0, std::iter::empty());
        assert!(
            grid.next_step(Position::new(10.0, 10.0), Position::new(15.0, 15.0))
                .is_none()
        );
    }

    #[test]
    fn open_field_steps_straight_toward_target() {
        let grid = Grid::build(400.0, 400.0, 40.0, 0.0, std::iter::empty());
        let from = Position::new(20.0, 20.0);
        let to = Position::new(340.0, 20.0);
        let step = grid.next_step(from, to).expect("path should exist");
        // Next cell over on the same row, moving toward `to`.
        assert!((step.x - 60.0).abs() < 1.0);
        assert!((step.y - 20.0).abs() < 1.0);
    }

    #[test]
    fn routes_around_a_wall_spanning_obstacle() {
        // A obstacle wall blocking the whole middle column except one gap
        // near the bottom - the path must detour down through the gap
        // rather than walking straight into the wall.
        let cell = 40.0;
        let width = 400.0;
        let height = 400.0;
        let gap_row = (height / cell) as usize - 1; // bottom row is the gap
        let mut obstacles = Vec::new();
        for row in 0..(height / cell) as usize {
            if row == gap_row {
                continue;
            }
            obstacles.push((
                Position::new(width / 2.0 + cell / 2.0, row as f32 * cell + cell / 2.0),
                cell / 2.0,
            ));
        }
        let grid = Grid::build(width, height, cell, 0.0, obstacles.into_iter());

        let from = Position::new(20.0, 20.0);
        let to = Position::new(380.0, 20.0);

        // Walk the path step by step, staying clear of the blocked column
        // except at the gap row, and confirm it actually reaches the target
        // side.
        let mut pos = from;
        let mut reached_other_side = false;
        for _ in 0..200 {
            let Some(step) = grid.next_step(pos, to) else {
                break;
            };
            let (col, row) = grid.cell_of(step);
            let mid_col = ((width / 2.0 + cell / 2.0) / cell) as usize;
            assert!(
                col != mid_col || row == gap_row,
                "path cut through the wall away from its gap"
            );
            pos = step;
            if pos.x > width / 2.0 {
                reached_other_side = true;
            }
        }
        assert!(reached_other_side, "path never made it past the wall");
    }

    #[test]
    fn path_cost_same_cell_is_zero() {
        // Unlike `next_step` (whose same-cell answer is `None` - see
        // `same_cell`'s doc comment), a cost query keeps "already there"
        // and "unreachable" apart.
        let grid = Grid::build(400.0, 400.0, 40.0, 0.0, std::iter::empty());
        assert_eq!(
            grid.path_cost(Position::new(10.0, 10.0), Position::new(15.0, 15.0)),
            Some(0)
        );
    }

    #[test]
    fn path_cost_open_field_is_manhattan_cell_distance() {
        let grid = Grid::build(400.0, 400.0, 40.0, 0.0, std::iter::empty());
        // Straight along one row: cells (0,0) -> (8,0).
        assert_eq!(
            grid.path_cost(Position::new(20.0, 20.0), Position::new(340.0, 20.0)),
            Some(8)
        );
        // Diagonal corner-to-corner: cardinal-only movement pays the full
        // Manhattan sum, cells (0,0) -> (8,8).
        assert_eq!(
            grid.path_cost(Position::new(20.0, 20.0), Position::new(340.0, 340.0)),
            Some(16)
        );
    }

    #[test]
    fn path_cost_detour_exceeds_open_field_cost() {
        // Same wall-with-one-gap layout as
        // `routes_around_a_wall_spanning_obstacle`: crossing the middle
        // column is only possible at the bottom row, so the shortest path
        // from (0,0) to (9,0) is down 9, across 9, back up 9 - strictly
        // more than the open-field Manhattan distance of 9.
        let cell = 40.0;
        let width = 400.0;
        let height = 400.0;
        let gap_row = (height / cell) as usize - 1;
        let mut obstacles = Vec::new();
        for row in 0..(height / cell) as usize {
            if row == gap_row {
                continue;
            }
            obstacles.push((
                Position::new(width / 2.0 + cell / 2.0, row as f32 * cell + cell / 2.0),
                cell / 2.0,
            ));
        }
        let grid = Grid::build(width, height, cell, 0.0, obstacles.into_iter());
        let from = Position::new(20.0, 20.0);
        let to = Position::new(380.0, 20.0);

        let open = Grid::build(width, height, cell, 0.0, std::iter::empty());
        let open_cost = open.path_cost(from, to).expect("open field always has a path");
        let detour_cost = grid.path_cost(from, to).expect("gap route should exist");
        assert_eq!(open_cost, 9);
        assert_eq!(detour_cost, 27);
        assert!(detour_cost > open_cost);
    }

    #[test]
    fn path_cost_sealed_goal_is_none() {
        // Corner goal cell (9,0) with both of its in-bounds neighbors -
        // (8,0) and (9,1) - blocked: the goal cell itself counts as open
        // (same rule as `next_step`), but nothing can ever reach it.
        let cell = 40.0;
        let obstacles = vec![
            (Position::new(340.0, 20.0), cell / 2.0),
            (Position::new(380.0, 60.0), cell / 2.0),
        ];
        let grid = Grid::build(400.0, 400.0, cell, 0.0, obstacles.into_iter());
        assert_eq!(
            grid.path_cost(Position::new(20.0, 20.0), Position::new(380.0, 20.0)),
            None
        );
    }
}

#[cfg(test)]
mod component_tests {
    use super::*;

    /// A full-height wall down the middle with no gap: the two halves are
    /// separate components, so `connected` must agree with `next_step`.
    #[test]
    fn components_agree_with_astar_across_a_sealed_wall() {
        let cell = 40.0;
        let obstacles = (0..10).map(|row| {
            (Position::new(220.0, row as f32 * cell + cell / 2.0), cell / 2.0)
        });
        let grid = Grid::build(400.0, 400.0, cell, 0.0, obstacles);
        let comps = grid.components();
        let left = Position::new(20.0, 20.0);
        let right = Position::new(380.0, 20.0);
        let left_low = Position::new(20.0, 380.0);
        assert!(grid.next_step(left, right).is_none());
        assert!(!comps.connected(&grid, left, right));
        assert!(grid.next_step(left, left_low).is_some());
        assert!(comps.connected(&grid, left, left_low));
        // Same cell counts as connected, matching `same_cell`.
        assert!(comps.connected(&grid, left, Position::new(25.0, 25.0)));
    }

    /// A point standing inside a blocked cell (an obstacle's clearance
    /// margin) still routes out through its open neighbours - A* treats
    /// the start cell as open, and so must this.
    #[test]
    fn blocked_start_cell_uses_its_open_neighbours() {
        let cell = 40.0;
        let grid = Grid::build(400.0, 400.0, cell, 0.0, std::iter::once((Position::new(220.0, 220.0), cell / 2.0)));
        let comps = grid.components();
        let inside = Position::new(220.0, 220.0);
        let far = Position::new(20.0, 20.0);
        assert!(grid.next_step(inside, far).is_some());
        assert!(comps.connected(&grid, inside, far));
    }
}

#[cfg(test)]
mod dims_tests {
    use super::*;

    /// The battlefield at PATHFIND_CELL_SIZE is 27x15 cells (ceil), and a
    /// single obstacle shows up as exactly its blocked cell in `ascii`.
    #[test]
    fn dims_and_ascii_match_the_built_grid() {
        let open = Grid::build(1280.0, 720.0, 48.0, 0.0, std::iter::empty());
        assert_eq!(open.dims(), (27, 15, 48.0));
        let text = open.ascii();
        assert_eq!(text.lines().count(), 15);
        assert!(text.lines().all(|l| l.len() == 27 && l.chars().all(|c| c == '.')));

        let one = Grid::build(1280.0, 720.0, 48.0, 0.0, std::iter::once((Position::new(72.0, 72.0), 8.0)));
        assert!(one.is_blocked(1, 1));
        assert!(!one.is_blocked(0, 0));
        assert!(one.is_blocked(27, 0), "off-grid counts as blocked");
        assert_eq!(one.ascii().lines().nth(1).unwrap(), ".#.........................");
    }
}
