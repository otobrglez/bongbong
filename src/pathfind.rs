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
use std::collections::BinaryHeap;

use crate::Position;

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
    /// obstacle blocks every cell within `half_extent + margin` of its
    /// center (in both axes) - `margin` should be roughly a tank's own half
    /// -hull, so pathfinding never routes through a gap too narrow for a
    /// tank to actually fit through.
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
            // Lowest/highest cell index the span [center-reach, center+reach]
            // actually has positive overlap with, not just touches - the
            // upper bound in particular needs `ceil(..) - 1`, not `floor`:
            // a span whose edge lands exactly on a cell boundary (e.g. an
            // obstacle exactly `cell_size` wide) touches the next cell at a
            // single point without overlapping its interior, and `floor`
            // alone would wrongly mark that next cell blocked too.
            let min_col_raw = ((center.x - reach) / cell_size).floor() as isize;
            let max_col_raw = ((center.x + reach) / cell_size).ceil() as isize - 1;
            let min_row_raw = ((center.y - reach) / cell_size).floor() as isize;
            let max_row_raw = ((center.y + reach) / cell_size).ceil() as isize - 1;
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

        let mut open = BinaryHeap::new();
        let mut came_from = vec![None; self.cols * self.rows];
        let mut g_score = vec![f32::INFINITY; self.cols * self.rows];
        let idx = |c: (usize, usize)| c.1 * self.cols + c.0;

        g_score[idx(start)] = 0.0;
        open.push(Node {
            cell: start,
            priority: heuristic(start, goal),
        });

        while let Some(Node { cell, .. }) = open.pop() {
            if cell == goal {
                // Walk back to the step right after `start`.
                let mut step = cell;
                while let Some(prev) = came_from[idx(step)] {
                    if prev == start {
                        return Some(self.center_of(step));
                    }
                    step = prev;
                }
                return Some(self.center_of(step));
            }
            for next in self.neighbors(cell) {
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
}
