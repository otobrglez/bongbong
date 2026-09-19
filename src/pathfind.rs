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
//!
//! Two ways to route on one grid, behind the one `next_step` call:
//!
//! - **A\*** per query (`search`), for a target nobody else is heading to
//!   (an engagement slot, a wander waypoint, a pickup).
//! - **A flow field** per shared target (`Field`, built by `add_field`): one
//!   Dijkstra outward from the goal cell stores every cell's cost to reach
//!   it, and a query from anywhere is then a read of four neighbours. The
//!   round builds one for each player and for the player's frog every
//!   frame, so however many enemies are alive the frame's routing work is
//!   bounded by the number of targets, not searchers. `next_step` picks
//!   the field whose goal cell the target falls in and falls back to A\*
//!   otherwise, so callers never know which served them.
//!
//! Step costs (`cost`) are what make the field tactical: `weigh` prices a
//! ford, `surcharge` adds to the cells a player's barrel points down and
//! to the cells other enemies stand in, and the same cheapest-neighbour
//! rule then bends every route out of the line of fire and around a
//! clump without any steering logic knowing.
//!
//! Portals are the one non-local feature: `Grid::with_portals` gives the
//! search a single virtual **hub** node. Stepping from any portal footprint
//! cell into the hub costs `hop_cost`, stepping out of the hub onto any
//! footprint cell costs nothing, so a route may teleport once. With two
//! portals that is exactly what the game does; with three or more the exit
//! is random in play, so the plan is *optimistic* (it assumes the best exit)
//! and the caller re-plans after landing. A grid built without portals, or
//! whose portals do not survive `with_portals`, runs the very same search
//! code with no hub edges and the same tie order, so its answers are
//! byte-identical to a plain grid's.
//! Both routers walk the hub: `search` as graph edges, and a flow field
//! (`add_field`) by relaxing every entrance from an exit it has priced,
//! with `descend` offering the exits beside the four neighbours - so a
//! target served by a field is reached through a portal exactly when
//! A* would go through one.

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

    /// Per portal, its nav footprint: the open cells whose centre lies
    /// within the portal's radius of its centre. Empty unless
    /// `with_portals` found at least two portals with a footprint.
    portals: Vec<Vec<(usize, usize)>>,

    /// Every cell of every footprint in `portals`, sorted and deduped, so
    /// membership is one binary search.
    portal_cells: Vec<(usize, usize)>,
    /// The portals' centres, one per entry of `portals` - the trigger
    /// point `next_step` hands out for a step onto a footprint cell.
    portal_centres: Vec<Position>,

    /// What stepping from a footprint cell into the hub costs, in cells;
    /// 0.0 while there are no portals (never read then).
    hop_cost: f32,
    /// Per cell, what a step *into* it costs the router: 1 for open
    /// ground, more for a cell worth avoiding when a cheaper way round
    /// exists (`weigh` - a ford). Never below 1, so the Manhattan
    /// heuristic stays admissible and A* still returns a cheapest route.
    cost: Vec<u8>,
    /// One flow field per shared goal cell (`add_field`); `next_step`
    /// serves a target from its field when one exists.
    fields: Vec<Field>,
    /// Connected-component labels once `label` has run: `connected`
    /// answers reachability from them in O(1) instead of a search.
    comps: Option<Components>,
}

/// Every cell's cost to reach one goal cell along cardinal steps - a
/// Dijkstra run outward from the goal over the grid's `cost`s, stored
/// so that routing from any cell is a read of its neighbours. Built by
/// `Grid::add_field`, consulted by `Grid::next_step` and `path_cost`.
struct Field {
    goal: (usize, usize),
    /// Per cell: the summed step cost from that cell to `goal`
    /// (`UNREACHABLE` where no route exists, `0` at the goal). Stepping
    /// *into* a cell costs that cell's `Grid::cost`, so a cell's value
    /// is its cheapest neighbour's value plus that neighbour's cost.
    to_goal: Vec<u32>,
}

/// `Field::to_goal` for a cell no route reaches.
const UNREACHABLE: u32 = u32::MAX;

impl Grid {
    /// Build a grid covering `0..width, 0..height` in `cell_size`-px cells.
    /// `obstacles` is every blocking shape as (center, half-extent); each
    /// obstacle blocks every cell whose *center* lies within
    /// `half_extent + margin` of its own center (in both axes) - `margin`
    /// should be a tank's worst-case half-hull, so pathfinding never
    /// routes through a gap too narrow for a tank to actually fit through.
    /// The area's own outer boundary is solid on the same terms: a cell
    /// whose centre is within `margin` of it is blocked.
    ///
    /// `cell_size` must match the pitch of whatever grid the obstacles are
    /// placed on (in this game, `PATHFIND_CELL_SIZE == OBSTACLE_GRID_SIZE`).
    /// Because occupancy is decided at cell *centers*, a mismatched pitch
    /// gives the rasterization a phase that repeats every `lcm` of the two,
    /// and whether a corridor survives it then depends on where its author
    /// happened to put it rather than on how wide it is - see
    /// `PATHFIND_CELL_SIZE`'s own comment for the measured damage.
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

        // The area's own boundary is solid, and gets the same `margin`
        // every obstacle does: a tank's centre can no more sit `margin`
        // inside the edge than `margin` inside a wall tile. `neighbors`
        // and `blocked_ahead` already refuse to leave the grid, but that
        // alone made the boundary a wall with *zero* clearance - the
        // outermost lane of cells was reported open, so the router offered
        // routes whose centre line runs inside the boundary's collider,
        // which `ai.rs`'s own `heads_into_wall` then refused. Measured on
        // the maps/test fixtures at a cell size equal to the map grid:
        // without this the probe's border-stuck count went 1 -> 12 and
        // wall-grind 0 -> 10 over the same 70 rounds.
        //
        // This also covers the overhang when the area is not a whole number
        // of cells: those centres are past the edge outright.
        //
        // `battlefield::gate_candidates` insets its roll-in lanes by the
        // same margin, since a lane cell no tank can stand in is not part
        // of the lane.
        for row in 0..rows {
            for col in 0..cols {
                let (cx, cy) = ((col as f32 + 0.5) * cell_size, (row as f32 + 0.5) * cell_size);
                if cx < margin || cx > width - margin || cy < margin || cy > height - margin {
                    blocked[row * cols + col] = true;
                }
            }
        }

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

        let cost = vec![1; cols * rows];
        Self {
            cell_size,
            cols,
            rows,
            blocked,
            portals: Vec::new(),
            portal_centres: Vec::new(),
            portal_cells: Vec::new(),
            hop_cost: 0.0,
            cost,
            fields: Vec::new(),
            comps: None,
        }
    }

    /// Make every cell whose centre a position in `cells` falls in cost
    /// `cost` (at least 1) to step into, where it was 1: the router then
    /// takes such a cell only when the dry way round is longer than the
    /// extra it charges. Occupancy is untouched - a weighted cell is still
    /// open to `usable`, `blocked_ahead` and the flood fills - so this
    /// changes which route is chosen, never whether one exists.
    pub fn weigh(&mut self, cells: impl Iterator<Item = Position>, cost: u32) {
        let cost = cost.clamp(1, u8::MAX as u32) as u8;
        for pos in cells {
            if pos.x < 0.0 || pos.y < 0.0 || pos.x > self.cols as f32 * self.cell_size || pos.y > self.rows as f32 * self.cell_size {
                continue;
            }
            let cell = self.cell_of(pos);
            self.cost[cell.1 * self.cols + cell.0] = cost;
        }
    }

    /// Give the grid its portals. `centres` are the portals' centres (in
    /// this game a grid *corner*, since map cells are multiples of the
    /// cell size while cell centres sit at half-cells); a portal's nav
    /// footprint is every open cell whose centre lies within `radius` of
    /// its centre - four cells at the game's default radius. A boxed-in
    /// footprint cell still counts: it is open, and the search's own rules
    /// decide whether anything can walk onto it. Portals whose footprint is
    /// empty (all of their cells blocked) are dropped, and if fewer than
    /// two remain the grid keeps no portals at all - a lone portal has
    /// nowhere to send anyone - so a single or sealed portal leaves the
    /// grid's every answer identical to a plain grid's.
    ///
    /// `hop_cost` is what entering the hub costs, in cells, `>= 1.0`. It
    /// is raised to at least the widest footprint's Manhattan span (2 for a
    /// four-cell footprint): the hub links *every* footprint cell to every
    /// other, its own portal's included, and a hop cheaper than crossing a
    /// footprint on foot would let the search "teleport" between two cells
    /// of the same portal and undercount that route. At the clamp the
    /// same-portal hop can only tie with walking, and a tie is harmless: it
    /// changes no cost, and the first step it can change is from a cell
    /// the tank teleports off before it moves anyway.
    pub fn with_portals(mut self, centres: &[Position], radius: f32, hop_cost: f32) -> Self {
        let mut portals: Vec<Vec<(usize, usize)>> = Vec::with_capacity(centres.len());
        let mut kept: Vec<Position> = Vec::with_capacity(centres.len());
        for &centre in centres {
            // Cells whose centre can possibly lie within `radius`.
            let lo = |v: f32| (((v - radius) / self.cell_size - 0.5).ceil().max(0.0)) as usize;
            let hi = |v: f32, n: usize| {
                (((v + radius) / self.cell_size - 0.5).floor() as isize).clamp(-1, n as isize - 1)
            };
            let (max_col, max_row) = (hi(centre.x, self.cols), hi(centre.y, self.rows));
            if max_col < 0 || max_row < 0 {
                continue;
            }
            let mut footprint = Vec::new();
            for row in lo(centre.y)..=max_row as usize {
                for col in lo(centre.x)..=max_col as usize {
                    let cell = (col, row);
                    if !self.blocked_at(cell) && self.center_of(cell).distance_to(centre) <= radius {
                        footprint.push(cell);
                    }
                }
            }
            if !footprint.is_empty() {
                portals.push(footprint);
                kept.push(centre);
            }
        }
        if portals.len() < 2 {
            return self;
        }
        let span = portals
            .iter()
            .flat_map(|fp| fp.iter().flat_map(move |&a| fp.iter().map(move |&b| heuristic(a, b))))
            .fold(0.0, f32::max);
        let mut portal_cells: Vec<(usize, usize)> = portals.iter().flatten().copied().collect();
        portal_cells.sort_unstable();
        portal_cells.dedup();
        self.portals = portals;
        self.portal_cells = portal_cells;
        self.portal_centres = kept;
        self.hop_cost = hop_cost.max(1.0).max(span);
        self
    }

    /// The centre of the portal whose footprint holds `cell` - the
    /// nearest one when footprints overlap - or `None` off every footprint.
    fn portal_centre_of(&self, cell: (usize, usize)) -> Option<Position> {
        if !self.is_portal_cell(cell) {
            return None;
        }
        let here = self.center_of(cell);
        self.portals
            .iter()
            .zip(&self.portal_centres)
            .filter(|(fp, _)| fp.contains(&cell))
            .map(|(_, &c)| c)
            .min_by(|a, b| a.distance_to(here).total_cmp(&b.distance_to(here)))
    }

    /// Every portal footprint cell, sorted - empty on a grid without
    /// (surviving) portals.
    pub fn portal_cells(&self) -> &[(usize, usize)] {
        &self.portal_cells
    }

    /// The cells a tank standing on `cell` can come out at: every
    /// footprint cell of every portal other than the one(s) `cell` belongs
    /// to. Empty when `cell` is not a portal cell. Lets a flood fill of its
    /// own (the map linter's) mirror the search's hub without knowing how
    /// it is built.
    pub fn portal_links(&self, cell: (usize, usize)) -> impl Iterator<Item = (usize, usize)> + '_ {
        let member = self.portal_cells.binary_search(&cell).is_ok();
        self.portals
            .iter()
            .filter(move |fp| member && !fp.contains(&cell))
            .flat_map(|fp| fp.iter().copied())
    }

    fn is_portal_cell(&self, cell: (usize, usize)) -> bool {
        self.portal_cells.binary_search(&cell).is_ok()
    }

    /// Add `extra` to the step cost of every cell a position in `cells`
    /// falls in, saturating at the cost type's ceiling - the tactical
    /// layer on top of `weigh`'s absolute prices (a firing lane, a cell
    /// another tank stands in). Like `weigh`, occupancy is untouched: a
    /// surcharged cell is still open, it is only dearer, so a route that
    /// has no other way through still takes it. Call before `add_field`,
    /// which bakes the costs in.
    pub fn surcharge(&mut self, cells: impl Iterator<Item = Position>, extra: u32) {
        if extra == 0 {
            return;
        }
        for pos in cells {
            if pos.x < 0.0 || pos.y < 0.0 || pos.x > self.cols as f32 * self.cell_size || pos.y > self.rows as f32 * self.cell_size {
                continue;
            }
            let cell = self.cell_of(pos);
            let slot = &mut self.cost[cell.1 * self.cols + cell.0];
            *slot = (*slot as u32 + extra).min(u8::MAX as u32) as u8;
        }
    }

    /// Run the connected-component flood fill (`components`) once and
    /// keep the labels, so `connected` answers from them for the rest of
    /// this grid's life. Occupancy never changes after `build`, so the
    /// labels cannot go stale.
    pub fn label(&mut self) {
        self.comps = Some(self.components());
    }

    /// True if a cardinal-step route exists between `from` and `to` -
    /// the same answer `next_step(from, to).is_some() || same_cell(from,
    /// to)` gives, read from the labels when `label` has run and found by
    /// a search otherwise. The cheap reachability test for a candidate
    /// nobody is routing to yet (`Ai::wander` sampling waypoints, the
    /// engagement ring validating slots).
    pub fn connected(&self, from: Position, to: Position) -> bool {
        match &self.comps {
            Some(comps) => comps.connected(self, from, to),
            None => self.same_cell(from, to) || self.next_step(from, to).is_some(),
        }
    }

    /// Build and keep a flow field toward `goal` (see `Field`): from now on
    /// `next_step` and `path_cost` toward any point in `goal`'s cell read
    /// the field instead of searching. The goal cell counts as open
    /// whatever `blocked` says, exactly as A* treats it. Adding a goal
    /// whose cell already has a field is a no-op. Costs are read at build
    /// time, so `weigh`/`surcharge` first.
    pub fn add_field(&mut self, goal: Position) {
        let goal = self.cell_of(goal);
        if self.field_for(goal).is_some() {
            return;
        }
        let n = self.cols * self.rows;
        let idx = |c: (usize, usize)| c.1 * self.cols + c.0;
        let mut to_goal = vec![UNREACHABLE; n];
        // (cost, cell index): a min-heap through `Reverse`. Ties pop by
        // index, though the final costs are the same whatever the order.
        let mut open = BinaryHeap::new();
        to_goal[idx(goal)] = 0;
        open.push(std::cmp::Reverse((0u32, idx(goal))));
        while let Some(std::cmp::Reverse((cost, at))) = open.pop() {
            if cost > to_goal[at] {
                continue;
            }
            let cell = (at % self.cols, at / self.cols);
            // Reaching the goal from a neighbour means stepping *into*
            // this cell, which costs this cell's own price.
            let via = cost + self.cost[at] as u32;
            for next in self.neighbors(cell) {
                let ni = idx(next);
                if self.blocked[ni] || via >= to_goal[ni] {
                    continue;
                }
                to_goal[ni] = via;
                open.push(std::cmp::Reverse((via, ni)));
            }
            // The hub, walked backwards: this cell is the free exit of a
            // hop whose entrance is any other portal cell, so each of
            // them reaches the goal for this cost plus the hop - the
            // exit's own price is not charged, exactly as `search` relaxes
            // an exit at the hub's cost. The hop is rounded once, as
            // `search` rounds its whole total.
            if self.is_portal_cell(cell) {
                let hop = cost + self.hop_cost.round() as u32;
                for &entrance in &self.portal_cells {
                    let ei = idx(entrance);
                    if entrance == cell || hop >= to_goal[ei] {
                        continue;
                    }
                    to_goal[ei] = hop;
                    open.push(std::cmp::Reverse((hop, ei)));
                }
            }
        }
        self.fields.push(Field { goal, to_goal });
    }

    /// A cell's step cost - 1 for plain ground, more where `weigh` or
    /// `surcharge` priced it; 0 off the grid. For overlays and dumps.
    pub fn cost_at(&self, col: usize, row: usize) -> u32 {
        if col >= self.cols || row >= self.rows {
            return 0;
        }
        self.cost[row * self.cols + col] as u32
    }

    /// The goal cell of every field this grid carries, in the order they
    /// were added (player 1 first in the round's grid).
    pub fn goals(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.fields.iter().map(|f| f.goal)
    }

    /// From cell (`col`, `row`), the neighbour cell the field toward
    /// `goal`'s cell hands out - the arrow an overlay draws - or `None`
    /// when there is no field for that goal, the cell is the goal, or no
    /// route from it exists. Reads the same `descend` the router uses.
    pub fn flow(&self, goal: Position, col: usize, row: usize) -> Option<(usize, usize)> {
        let goal = self.cell_of(goal);
        if (col, row) == goal || col >= self.cols || row >= self.rows {
            return None;
        }
        self.field_for(goal).and_then(|f| self.descend(f, (col, row))).map(|hit| hit.first_step)
    }

    /// The field's stored cost from cell (`col`, `row`) to `goal`'s cell -
    /// `Some(0)` at the goal, `None` for a blocked or unreachable cell or
    /// when no field covers that goal.
    pub fn to_goal(&self, goal: Position, col: usize, row: usize) -> Option<u32> {
        let goal = self.cell_of(goal);
        if col >= self.cols || row >= self.rows {
            return None;
        }
        let field = self.field_for(goal)?;
        let cost = field.to_goal[row * self.cols + col];
        (cost != UNREACHABLE).then_some(cost)
    }

    fn field_for(&self, goal: (usize, usize)) -> Option<&Field> {
        self.fields.iter().find(|f| f.goal == goal)
    }

    /// The cheapest neighbour of `start` to step into on the way to
    /// `field`'s goal, with the whole route's cost through it: the
    /// field's equivalent of `search`, including its rules that the start
    /// cell is open whatever `blocked` says and the goal cell is enterable.
    /// Ties break on the lowest cell index, so the answer is a pure
    /// function of the grid. `None` when no neighbour reaches the goal.
    fn descend(&self, field: &Field, start: (usize, usize)) -> Option<SearchHit> {
        let idx = |c: (usize, usize)| c.1 * self.cols + c.0;
        let mut best: Option<(u32, usize, (usize, usize))> = None;
        for next in self.neighbors(start) {
            let ni = idx(next);
            if next != field.goal && self.blocked[ni] {
                continue;
            }
            let there = field.to_goal[ni];
            if there == UNREACHABLE {
                continue;
            }
            let total = there + self.cost[ni] as u32;
            if best.is_none_or(|(c, i, _)| (total, ni) < (c, i)) {
                best = Some((total, ni, next));
            }
        }
        // From a portal cell the hub is a step too: out at any other
        // portal cell for the hop, its own price uncharged, and the first
        // step is then the exit - the tank never drives there, the
        // teleport fires first, but the heading is right for the far side
        // (`next_step`).
        if self.is_portal_cell(start) {
            let hop = self.hop_cost.round() as u32;
            for &exit in &self.portal_cells {
                let ei = idx(exit);
                if exit == start || field.to_goal[ei] == UNREACHABLE {
                    continue;
                }
                let total = field.to_goal[ei] + hop;
                if best.is_none_or(|(c, i, _)| (total, ei) < (c, i)) {
                    best = Some((total, ei, exit));
                }
            }
        }
        best.map(|(cost, _, first_step)| SearchHit { first_step, cost })
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
    /// (found via the probe harness: closing in for an attack routinely
    /// puts the two tanks in the same cell well before they're actually
    /// touching).
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
        self.bfs_open(from, true, None, |center| {
            avoid.iter().all(|&p| center.distance_to(p) >= avoid_clear)
        })
        .unwrap_or(from)
    }

    /// The center of the nearest usable cell (see `usable`) a tank at `from`
    /// could *drive* to whose centre satisfies `ok`, or `None` if there is
    /// no such cell within `max_steps` cardinal steps. Unlike
    /// `nearest_open`, the expansion only crosses open cells - `from`'s own
    /// cell is the one exception, traversable whatever its state, the same
    /// rule `next_step` applies to its start - so the answer is always
    /// somewhere the tank can actually get to from where it stands, not
    /// merely the fewest cells away as the crow flies.
    pub fn nearest_open_reachable(
        &self,
        from: Position,
        max_steps: usize,
        ok: impl Fn(Position) -> bool,
    ) -> Option<Position> {
        self.bfs_open(from, false, Some(max_steps), ok)
    }

    /// The breadth-first walk behind `nearest_open` and
    /// `nearest_open_reachable`: outward cardinally from `from`'s cell, the
    /// first usable cell whose centre passes `ok` wins. `through_blocked`
    /// lets the frontier cross blocked cells (they are never *returned* -
    /// `usable_cell` rules them out - only walked through); `max_steps`
    /// caps how many steps from the start the frontier may reach.
    fn bfs_open(
        &self,
        from: Position,
        through_blocked: bool,
        max_steps: Option<usize>,
        ok: impl Fn(Position) -> bool,
    ) -> Option<Position> {
        let start = self.cell_of(from);
        let idx = |c: (usize, usize)| c.1 * self.cols + c.0;
        let mut visited = vec![false; self.cols * self.rows];
        visited[idx(start)] = true;
        let mut queue = VecDeque::new();
        queue.push_back((start, 0usize));
        while let Some((cell, steps)) = queue.pop_front() {
            if self.usable_cell(cell) {
                let center = self.center_of(cell);
                if ok(center) {
                    return Some(center);
                }
            }
            if max_steps.is_some_and(|cap| steps >= cap) {
                continue;
            }
            for next in self.neighbors(cell) {
                if !visited[idx(next)] && (through_blocked || !self.blocked_at(next)) {
                    visited[idx(next)] = true;
                    queue.push_back((next, steps + 1));
                }
            }
        }
        None
    }

    /// Label every open cell with its connected component (4-neighbour
    /// flood fill) so `Components::connected` answers "does a route exist
    /// between these two points" in O(1) - the same answer `next_step`
    /// would give, without running A* per query. Build once per frame and
    /// query as often as needed (engagement-slot validation asks up to 16
    /// times per enemy). Portals join components the way the hub joins
    /// routes: every component holding a portal cell is relabelled to the
    /// smallest of their labels.
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
        let mut portal_labels: Vec<u32> = self
            .portal_cells
            .iter()
            .map(|&c| label[c.1 * self.cols + c.0])
            .collect();
        portal_labels.sort_unstable();
        portal_labels.dedup();
        if portal_labels.len() >= 2 {
            let merged = portal_labels[0];
            for l in label.iter_mut() {
                if portal_labels.binary_search(l).is_ok() {
                    *l = merged;
                }
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
    ///
    /// A first step onto a portal footprint cell is handed out as that
    /// portal's *centre* rather than the cell's: the centre is the trigger
    /// point, and a hull steering at the middle of the footprint lands
    /// inside the trigger radius whatever its momentum overshoots, where
    /// one steering at a cell centre beside the anchor could round the 2x2
    /// footprint a few pixels outside the radius, pass after pass. When
    /// the route teleports and `from` is already on the entrance, the first
    /// step is the exit portal's centre - the tank never drives there, the
    /// teleport fires first, but the heading is right for the far side.
    pub fn next_step(&self, from: Position, to: Position) -> Option<Position> {
        let start = self.cell_of(from);
        let goal = self.cell_of(to);
        if start == goal {
            return None;
        }
        self.route(start, goal)
            .map(|hit| self.portal_centre_of(hit.first_step).unwrap_or_else(|| self.center_of(hit.first_step)))
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
        self.route(start, goal).map(|hit| hit.cost)
    }

    /// The field toward `goal` when one was added, A* otherwise - the
    /// one seam both public routers go through, so they can never
    /// disagree about which served a target.
    fn route(&self, start: (usize, usize), goal: (usize, usize)) -> Option<SearchHit> {
        match self.field_for(goal) {
            Some(field) => self.descend(field, start),
            None => self.search(start, goal, true),
        }
    }

    /// `next_step` on foot: the same route with the portal hub left out,
    /// for a tank whose portal cooldown is running. Such a tank cannot
    /// use a portal yet, and a route through one would walk it back onto
    /// the portal it just came out of, where the cooldown stands still
    /// (`Game::tick_timers`) - it would have to leave and come back,
    /// circling the footprint in the meantime. Always a search: the flow
    /// fields price the hub in. `None` where no walking route exists (a
    /// room joined to the target's only by a portal), which `Ai::steer`
    /// answers by wandering until the cooldown runs out. A grid without
    /// portals routes exactly as `next_step`.
    pub fn next_step_walking(&self, from: Position, to: Position) -> Option<Position> {
        let start = self.cell_of(from);
        let goal = self.cell_of(to);
        if start == goal {
            return None;
        }
        self.search(start, goal, false).map(|hit| self.center_of(hit.first_step))
    }

    /// The one A* implementation `route` searches with when no field
    /// covers the goal - callers guarantee `start != goal` (each handles the same-cell case
    /// itself, with different semantics). Returns `None` when no path
    /// exists; on a hit, both the first cell to move into (what `next_step`
    /// wants) and the whole path's cost (what `path_cost` wants), since the
    /// goal-pop moment has both on hand anyway.
    ///
    /// The graph is the grid's open cells plus the portal hub (see the
    /// module doc): a cell's successors are its four open neighbours at
    /// cost 1 and, for a portal cell, the hub at `hop_cost`; the hub's
    /// successors are every portal cell at cost 0. The heuristic stays
    /// admissible and consistent with the hub in the graph: a cell's is the
    /// smaller of the plain Manhattan distance and "Manhattan to the nearest
    /// portal cell, plus the hop, plus `exit_h`", the hub's is `exit_h`
    /// alone, where `exit_h` is the Manhattan distance from the nearest
    /// portal cell to the goal, computed once per search. Without portals,
    /// or with `hub` false (`next_step_walking`), the extra terms are
    /// skipped outright and the search is the plain grid A*, tie order
    /// included.
    fn search(&self, start: (usize, usize), goal: (usize, usize), hub: bool) -> Option<SearchHit> {
        let hub = hub && !self.portal_cells.is_empty();
        let mut open = BinaryHeap::new();
        // One slot per cell plus the hub's sentinel slot at the end.
        let slots = self.cols * self.rows + 1;
        let mut came_from: Vec<Option<NavNode>> = vec![None; slots];
        let mut g_score = vec![f32::INFINITY; slots];
        // Nodes already expanded (popped and relaxed) once. Without this, a
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
        let mut closed = vec![false; slots];
        let idx = |n: NavNode| n.idx(self.cols, self.rows);

        let exit_h = self
            .portal_cells
            .iter()
            .map(|&q| heuristic(q, goal))
            .fold(f32::INFINITY, f32::min);
        let h = |node: NavNode| -> f32 {
            match node {
                NavNode::Hub => exit_h,
                NavNode::Cell(c) => {
                    let walk = heuristic(c, goal);
                    if !hub {
                        return walk;
                    }
                    let to_portal = self
                        .portal_cells
                        .iter()
                        .map(|&p| heuristic(c, p))
                        .fold(f32::INFINITY, f32::min);
                    walk.min(to_portal + self.hop_cost + exit_h)
                }
            }
        };

        let start_node = NavNode::Cell(start);
        g_score[idx(start_node)] = 0.0;
        open.push(OpenEntry {
            node: start_node,
            priority: h(start_node),
        });

        while let Some(OpenEntry { node, .. }) = open.pop() {
            if closed[idx(node)] {
                // Stale heap entry from before this node's last improvement.
                continue;
            }
            closed[idx(node)] = true;

            if node == NavNode::Cell(goal) {
                // Unit steps summed in f32 stay exact integers (well under
                // f32's 2^24 exact-integer range on any sane grid); only a
                // fractional hop cost makes the rounding do anything.
                let cost = g_score[idx(node)].round() as u32;
                // Walk back to the step right after `start`, skipping the
                // hub: the first step is always a cell, the exit when the
                // route teleports straight out of `start`.
                let mut at = node;
                let mut first_step = goal;
                while let Some(prev) = came_from[idx(at)] {
                    if prev == start_node {
                        break;
                    }
                    at = prev;
                    if let NavNode::Cell(c) = at {
                        first_step = c;
                    }
                }
                return Some(SearchHit { first_step, cost });
            }

            let g = g_score[idx(node)];
            let mut relax = |next: NavNode, tentative: f32| {
                if tentative < g_score[idx(next)] {
                    came_from[idx(next)] = Some(node);
                    g_score[idx(next)] = tentative;
                    open.push(OpenEntry {
                        node: next,
                        priority: tentative + h(next),
                    });
                }
            };
            match node {
                NavNode::Cell(cell) => {
                    for next in self.neighbors(cell) {
                        if closed[idx(NavNode::Cell(next))] {
                            continue;
                        }
                        if next != goal && self.blocked_at(next) {
                            continue;
                        }
                        // A step costs what the cell charges (`weigh`), 1 on
                        // open ground.
                        relax(NavNode::Cell(next), g + self.cost[idx(NavNode::Cell(next))] as f32);
                    }
                    if hub && !closed[idx(NavNode::Hub)] && self.is_portal_cell(cell) {
                        relax(NavNode::Hub, g + self.hop_cost);
                    }
                }
                NavNode::Hub => {
                    for &exit in &self.portal_cells {
                        if closed[idx(NavNode::Cell(exit))] {
                            continue;
                        }
                        relax(NavNode::Cell(exit), g);
                    }
                }
            }
        }
        None
    }
}

/// What one successful `Grid::search` run hands back to its two public
/// wrappers: the first cell to step into (for `next_step`) and the full
/// path's cost in cells (for `path_cost`).
struct SearchHit {
    first_step: (usize, usize),
    cost: u32,
}

/// Manhattan distance in cells - admissible since movement is 4-directional
/// and no step costs less than 1 (`Grid::cost`), so A* with this heuristic
/// finds a cheapest path.
fn heuristic(a: (usize, usize), b: (usize, usize)) -> f32 {
    (a.0 as f32 - b.0 as f32).abs() + (a.1 as f32 - b.1 as f32).abs()
}

/// A node of the search graph: a grid cell, or the one portal hub every
/// portal footprint cell links to (see the module doc).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NavNode {
    Cell((usize, usize)),
    Hub,
}

impl NavNode {
    /// Slot in the search's per-node tables: cells in row-major order, the
    /// hub in the one extra slot after them.
    fn idx(self, cols: usize, rows: usize) -> usize {
        match self {
            NavNode::Cell((col, row)) => row * cols + col,
            NavNode::Hub => cols * rows,
        }
    }
}

/// One entry in the open set: a node plus its f-score (g + heuristic).
/// `BinaryHeap` is a max-heap, so `Ord` is reversed on `priority` to pop the
/// lowest f-score first.
struct OpenEntry {
    node: NavNode,
    priority: f32,
}

impl PartialEq for OpenEntry {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}
impl Eq for OpenEntry {}
impl PartialOrd for OpenEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OpenEntry {
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

    /// 10 x 10 grid of 40 px cells, zero margin.
    const CELL: f32 = 40.0;
    const SIDE: f32 = 400.0;

    /// The centre of cell (`col`, `row`).
    fn at(col: usize, row: usize) -> Position {
        Position::new((col as f32 + 0.5) * CELL, (row as f32 + 0.5) * CELL)
    }

    /// One obstacle filling cell (`col`, `row`) exactly.
    fn block(col: usize, row: usize) -> (Position, f32) {
        (at(col, row), CELL / 2.0)
    }

    /// The whole of column `col`, minus `gap_rows`.
    fn wall(col: usize, gap_rows: &[usize]) -> Vec<(Position, f32)> {
        (0..10)
            .filter(|row| !gap_rows.contains(row))
            .map(|row| block(col, row))
            .collect()
    }

    /// Portal radius that takes exactly the four cells around a corner
    /// (their centres are 28.3 px away, the next ring 63.2).
    const RADIUS: f32 = 40.0;

    /// A portal centred on the corner shared by cells (`col`, `row`) and
    /// its right/lower neighbours.
    fn corner(col: usize, row: usize) -> Position {
        Position::new((col as f32 + 1.0) * CELL, (row as f32 + 1.0) * CELL)
    }

    /// Two portals on either side of a sealed wall: the only route hops.
    /// The left footprint is cells (1..=2, 1..=2), the right (7..=8, 7..=8).
    fn two_portal_grid(hop: f32) -> Grid {
        Grid::build(SIDE, SIDE, CELL, 0.0, wall(5, &[]).into_iter())
            .with_portals(&[corner(1, 1), corner(7, 7)], RADIUS, hop)
    }

    #[test]
    fn portals_route_across_a_sealed_wall() {
        let grid = two_portal_grid(3.0);
        assert_eq!(grid.portal_cells().len(), 8);
        let left = at(0, 0);
        let right = at(9, 9);
        // Walk to the nearest entrance (1,1): 2; hop: 3; nearest exit (8,8)
        // to the goal: 2.
        assert_eq!(grid.path_cost(left, right), Some(7));
        assert_eq!(grid.path_cost(right, left), Some(7));
        // Beside the entrance, the first step is the entrance portal's
        // centre (the trigger point, not the footprint cell's centre).
        assert_eq!(grid.next_step(at(1, 0), right), Some(corner(1, 1)));
        // On the entrance, the first step is the exit portal's centre on
        // the far side.
        assert_eq!(grid.next_step(at(1, 1), right), Some(corner(7, 7)));
        // Off every footprint a step is still a cell's centre.
        let first = grid.next_step(at(0, 0), right).expect("routes");
        assert!(first == at(1, 0) || first == at(0, 1), "{first:?}");
        assert!(grid.components().connected(&grid, left, right));
        // The hub never beats walking within one footprint.
        assert_eq!(grid.path_cost(at(1, 1), at(2, 2)), Some(2));
    }

    /// A field toward a goal behind the wall prices and steps every cell
    /// exactly as the search does, hub included: the entrances are priced
    /// through the exit, and standing on one the first step is the exit.
    #[test]
    fn fields_route_through_portals_like_the_search() {
        let plain = two_portal_grid(3.0);
        let mut grid = two_portal_grid(3.0);
        let right = at(9, 9);
        grid.add_field(right);
        grid.add_field(at(2, 2));
        assert_eq!(grid.path_cost(at(0, 0), right), Some(7));
        assert_eq!(grid.next_step(at(1, 0), right), Some(corner(1, 1)));
        assert_eq!(grid.next_step(at(1, 1), right), Some(corner(7, 7)));
        // The hub never beats walking within one footprint here either.
        assert_eq!(grid.path_cost(at(1, 1), at(2, 2)), Some(2));
        // Costs agree from every cell (first steps may differ where two
        // are equally good: the field breaks ties on cell index, A* on
        // its heap order).
        for row in 0..10 {
            for col in 0..10 {
                let from = at(col, row);
                assert_eq!(grid.path_cost(from, right), plain.path_cost(from, right), "cost from ({col},{row})");
            }
        }
    }

    /// On foot the sealed wall is sealed: no route, whatever the hub
    /// offers; on a walkable target the walking route is the ordinary one.
    #[test]
    fn walking_routes_leave_the_hub_out() {
        let grid = two_portal_grid(3.0);
        assert_eq!(grid.next_step_walking(at(0, 0), at(9, 9)), None);
        assert_eq!(grid.next_step_walking(at(1, 1), at(9, 9)), None);
        assert!(grid.next_step(at(0, 0), at(9, 9)).is_some());
        assert_eq!(grid.next_step_walking(at(0, 0), at(3, 0)), Some(at(1, 0)));
        assert_eq!(grid.next_step_walking(at(0, 0), at(0, 0)), None);
    }

    #[test]
    fn hop_cost_is_clamped_to_the_footprint_span() {
        // Asked for a hop of 1, the grid charges 2 - one cell less than
        // the four-cell footprint's diagonal and the search would teleport
        // between two cells of the same portal.
        let grid = two_portal_grid(1.0);
        assert_eq!(grid.path_cost(at(1, 1), at(2, 2)), Some(2));
        assert_eq!(grid.path_cost(at(0, 0), at(9, 9)), Some(2 + 2 + 2));
    }

    #[test]
    fn portal_links_name_the_other_portals_cells() {
        let grid = two_portal_grid(3.0);
        let mut links: Vec<_> = grid.portal_links((1, 1)).collect();
        links.sort_unstable();
        assert_eq!(links, vec![(7, 7), (7, 8), (8, 7), (8, 8)]);
        assert_eq!(grid.portal_links((0, 0)).count(), 0);
    }

    #[test]
    fn three_portals_take_the_best_exit() {
        // Left portal at (1..=2, 1..=2); right portals at (7..=8, 7..=8)
        // and (7..=8, 1..=2), all behind the sealed wall.
        let grid = Grid::build(SIDE, SIDE, CELL, 0.0, wall(5, &[]).into_iter())
            .with_portals(&[corner(1, 1), corner(7, 7), corner(7, 1)], RADIUS, 3.0);
        let left = at(0, 0);
        // Top-right goal: exit (8,1) is 2 away, (8,7) is 8.
        assert_eq!(grid.path_cost(left, at(9, 0)), Some(2 + 3 + 2));
        // Bottom-right goal: exit (8,8) is 2 away, (8,2) is 8.
        assert_eq!(grid.path_cost(left, at(9, 9)), Some(2 + 3 + 2));
        // Mid-right goal (9,5): exit (8,7) is 3 away, (8,2) is 4.
        assert_eq!(grid.path_cost(left, at(9, 5)), Some(2 + 3 + 3));
        assert_eq!(grid.next_step(at(1, 1), at(9, 0)), Some(corner(7, 1)));
        assert_eq!(grid.next_step(at(1, 1), at(9, 9)), Some(corner(7, 7)));
    }

    /// Every answer of `plain` and `with` agrees over every cell pair.
    fn assert_grids_identical(plain: &Grid, with: &Grid) {
        assert!(with.portal_cells().is_empty());
        let plain_comps = plain.components();
        let with_comps = with.components();
        for a in 0..100 {
            for b in 0..100 {
                let from = at(a % 10, a / 10);
                let to = at(b % 10, b / 10);
                assert_eq!(plain.path_cost(from, to), with.path_cost(from, to), "{from:?} -> {to:?}");
                assert_eq!(plain.next_step(from, to), with.next_step(from, to), "{from:?} -> {to:?}");
                assert_eq!(
                    plain_comps.connected(plain, from, to),
                    with_comps.connected(with, from, to),
                    "{from:?} -> {to:?}"
                );
            }
        }
    }

    #[test]
    fn a_lone_portal_changes_nothing() {
        let obstacles = wall(5, &[9]);
        let plain = Grid::build(SIDE, SIDE, CELL, 0.0, obstacles.clone().into_iter());
        let with = Grid::build(SIDE, SIDE, CELL, 0.0, obstacles.into_iter())
            .with_portals(&[corner(1, 1)], RADIUS, 3.0);
        assert_grids_identical(&plain, &with);
    }

    #[test]
    fn a_portal_with_a_blocked_footprint_is_dropped() {
        // The second portal sits on a 2 x 2 block of obstacles, so its
        // footprint is empty and only one portal is left - none survive.
        let mut obstacles = wall(5, &[9]);
        obstacles.extend([block(7, 7), block(8, 7), block(7, 8), block(8, 8)]);
        let plain = Grid::build(SIDE, SIDE, CELL, 0.0, obstacles.clone().into_iter());
        let with = Grid::build(SIDE, SIDE, CELL, 0.0, obstacles.into_iter())
            .with_portals(&[corner(1, 1), corner(7, 7)], RADIUS, 3.0);
        assert_grids_identical(&plain, &with);
    }

    #[test]
    fn nearest_open_reachable_stays_on_open_cells() {
        // (0,0) is open but both its neighbours are blocked: nothing can
        // be driven to, while `nearest_open` walks through the blocked
        // cells and lands on (0,2), the first usable cell in BFS order.
        let grid = Grid::build(SIDE, SIDE, CELL, 0.0, [block(1, 0), block(0, 1)].into_iter());
        assert_eq!(grid.nearest_open_reachable(at(0, 0), 100, |_| true), None);
        assert_eq!(grid.nearest_open(at(0, 0), &[], 0.0), at(0, 2));

        // A sealed wall: the far side is a few blocked cells away, but not
        // by driving.
        let walled = Grid::build(SIDE, SIDE, CELL, 0.0, wall(5, &[]).into_iter());
        let far_side = |p: Position| p.x > 6.0 * CELL;
        assert_eq!(walled.nearest_open_reachable(at(0, 0), 100, far_side), None);
    }

    #[test]
    fn nearest_open_reachable_honours_the_step_cap() {
        let grid = Grid::build(SIDE, SIDE, CELL, 0.0, std::iter::empty());
        let past_col_four = |p: Position| p.x > 5.0 * CELL;
        // Column 5 is five steps from (0,0); the only column-5 cell at
        // exactly that distance is (5,0).
        assert_eq!(grid.nearest_open_reachable(at(0, 0), 4, past_col_four), None);
        assert_eq!(grid.nearest_open_reachable(at(0, 0), 5, past_col_four), Some(at(5, 0)));
        // A blocked start still expands into its open neighbours.
        let boxed = Grid::build(SIDE, SIDE, CELL, 0.0, std::iter::once(block(0, 0)));
        assert_eq!(boxed.nearest_open_reachable(at(0, 0), 1, |_| true), Some(at(0, 1)));
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
        let mut grid = Grid::build(400.0, 400.0, cell, 0.0, obstacles);
        let left = Position::new(20.0, 20.0);
        let right = Position::new(380.0, 20.0);
        let left_low = Position::new(20.0, 380.0);
        // Unlabelled, `connected` searches; labelled, it reads. Same answers.
        for labelled in [false, true] {
            if labelled {
                grid.label();
            }
            assert!(grid.next_step(left, right).is_none());
            assert!(!grid.connected(left, right));
            assert!(grid.next_step(left, left_low).is_some());
            assert!(grid.connected(left, left_low));
            // Same cell counts as connected, matching `same_cell`.
            assert!(grid.connected(left, Position::new(25.0, 25.0)));
        }
    }

    /// A point standing inside a blocked cell (an obstacle's clearance
    /// margin) still routes out through its open neighbours - A* treats
    /// the start cell as open, and so must this.
    #[test]
    fn blocked_start_cell_uses_its_open_neighbours() {
        let cell = 40.0;
        let mut grid = Grid::build(400.0, 400.0, cell, 0.0, std::iter::once((Position::new(220.0, 220.0), cell / 2.0)));
        grid.label();
        let inside = Position::new(220.0, 220.0);
        let far = Position::new(20.0, 20.0);
        assert!(grid.next_step(inside, far).is_some());
        assert!(grid.connected(inside, far));
    }
}

#[cfg(test)]
mod field_tests {
    use super::*;

    /// 9 x 5 cells of 40 px, margin 0, a two-cell wall at column 4 rows
    /// 0-1, the goal at (7, 2): the worked example in the routing
    /// write-up. `at` is a cell's centre.
    fn nine_by_five() -> Grid {
        let cell = 40.0;
        let walls = [(4usize, 0usize), (4, 1)].into_iter().map(move |(c, r)| (at(c, r), cell / 2.0));
        Grid::build(360.0, 200.0, cell, 0.0, walls)
    }

    fn at(c: usize, r: usize) -> Position {
        Position::new(c as f32 * 40.0 + 20.0, r as f32 * 40.0 + 20.0)
    }

    /// Follow `next_step` from `from` until it reaches `to`'s cell,
    /// returning the cells visited after `from`.
    fn walk(grid: &Grid, from: Position, to: Position) -> Vec<(usize, usize)> {
        let mut route = Vec::new();
        let mut pos = from;
        for _ in 0..100 {
            if grid.same_cell(pos, to) {
                return route;
            }
            pos = grid.next_step(pos, to).expect("route exists");
            route.push(grid.cell_of(pos));
        }
        panic!("route never arrived: {route:?}");
    }

    /// With a field toward the goal, every open cell reports the same
    /// route cost A* finds, and the step it hands out lowers that cost by
    /// exactly the step's own price - the field is a Dijkstra table, not
    /// an approximation.
    #[test]
    fn field_costs_and_steps_agree_with_astar_from_every_cell() {
        let astar = nine_by_five();
        let mut field = nine_by_five();
        field.add_field(at(7, 2));
        for r in 0..5 {
            for c in 0..9 {
                if astar.is_blocked(c, r) {
                    continue;
                }
                let expect = astar.path_cost(at(c, r), at(7, 2));
                let got = field.path_cost(at(c, r), at(7, 2));
                assert_eq!(got, expect, "cost from ({c}, {r})");
                if let Some(step) = field.next_step(at(c, r), at(7, 2)) {
                    let (sc, sr) = field.cell_of(step);
                    let rest = field.path_cost(step, at(7, 2)).expect("step is on a route");
                    assert_eq!(rest + field.cost[sr * 9 + sc] as u32, got.unwrap(), "step from ({c}, {r}) to ({sc}, {sr})");
                }
            }
        }
    }

    /// A start inside a blocked cell (an obstacle's margin) and a goal
    /// inside one both route with the field exactly as A* lets them.
    #[test]
    fn field_treats_start_and_goal_cells_as_open_like_astar() {
        let mut grid = nine_by_five();
        grid.add_field(at(4, 1));
        assert!(grid.next_step(at(0, 2), at(4, 1)).is_some(), "goal inside the wall");
        assert_eq!(grid.path_cost(at(0, 2), at(4, 1)), nine_by_five().path_cost(at(0, 2), at(4, 1)));
        grid.add_field(at(7, 2));
        assert!(grid.next_step(at(4, 0), at(7, 2)).is_some(), "start inside the wall");
        assert_eq!(grid.path_cost(at(4, 0), at(7, 2)), nine_by_five().path_cost(at(4, 0), at(7, 2)));
    }

    /// Sealed off from the goal, the field has no step to offer - the
    /// same `None` A* returns, which `Ai::steer` reads as "wander".
    #[test]
    fn field_has_no_step_across_a_sealed_wall() {
        let cell = 40.0;
        let obstacles = (0..10).map(|row| (Position::new(220.0, row as f32 * cell + cell / 2.0), cell / 2.0));
        let mut grid = Grid::build(400.0, 400.0, cell, 0.0, obstacles);
        let left = Position::new(20.0, 20.0);
        let right = Position::new(380.0, 20.0);
        grid.add_field(right);
        assert!(grid.next_step(left, right).is_none());
        assert_eq!(grid.path_cost(left, right), None);
        assert!(grid.next_step(left, Position::new(20.0, 380.0)).is_some(), "the open half still routes by A*");
    }

    /// The firing-lane surcharge: with every step costing 1 an enemy at
    /// (0, 2) drives straight down row 2 into the goal at (7, 2). Charge
    /// 3 extra on the five lane cells in front of the goal and the same
    /// cheapest-neighbour rule drops to row 3, runs along it, and enters
    /// the lane only at the last step - two steps longer, never in the
    /// line of fire.
    #[test]
    fn a_surcharged_lane_bends_the_route_round_it() {
        let goal = at(7, 2);
        let mut plain = nine_by_five();
        plain.add_field(goal);
        assert_eq!(plain.path_cost(at(0, 2), goal), Some(7));
        assert_eq!(walk(&plain, at(0, 2), goal), [(1, 2), (2, 2), (3, 2), (4, 2), (5, 2), (6, 2), (7, 2)]);

        let mut lane = nine_by_five();
        lane.surcharge((2..=6).map(|c| at(c, 2)), 3);
        lane.add_field(goal);
        assert_eq!(lane.path_cost(at(0, 2), goal), Some(9));
        assert_eq!(walk(&lane, at(0, 2), goal), [(1, 2), (1, 3), (2, 3), (3, 3), (4, 3), (5, 3), (6, 3), (7, 3), (7, 2)]);
        assert!(lane.usable(at(4, 2)), "a surcharged cell is still open");
        // Without a way round, the lane is still taken: block rows 3 and
        // 4 at column 4 too and every route must cross (4, 2), entered
        // from (3, 2) and left through (5, 2) - three lane cells at 4
        // each plus the eight dry steps that skirt the other two.
        let mut penned = Grid::build(360.0, 200.0, 40.0, 0.0, [(4usize, 0usize), (4, 1), (4, 3), (4, 4)].into_iter().map(|(c, r)| (at(c, r), 20.0)));
        penned.surcharge((2..=6).map(|c| at(c, 2)), 3);
        penned.add_field(goal);
        assert_eq!(penned.path_cost(at(0, 2), goal), Some(3 * 4 + 8));
    }

    /// A surcharge saturates instead of wrapping, and a zero surcharge
    /// leaves the grid untouched.
    #[test]
    fn surcharge_saturates_and_zero_is_a_no_op() {
        let mut grid = nine_by_five();
        grid.surcharge(std::iter::once(at(1, 1)), 0);
        assert_eq!(grid.cost[1 * 9 + 1], 1);
        grid.surcharge(std::iter::once(at(1, 1)), 300);
        assert_eq!(grid.cost[1 * 9 + 1], u8::MAX);
        grid.surcharge(std::iter::once(at(1, 1)), 300);
        assert_eq!(grid.cost[1 * 9 + 1], u8::MAX);
    }

    /// The read accessors an overlay and the dev server's `field` dump
    /// use see the same table the router steps by.
    #[test]
    fn accessors_read_the_field_the_router_steps_by() {
        let mut grid = nine_by_five();
        grid.surcharge(std::iter::once(at(3, 2)), 3);
        assert_eq!(grid.cost_at(3, 2), 4);
        assert_eq!(grid.cost_at(0, 0), 1);
        assert_eq!(grid.cost_at(99, 0), 0, "off the grid");
        assert!(grid.goals().next().is_none());
        assert_eq!(grid.flow(at(7, 2), 0, 2), None, "no field yet");
        grid.add_field(at(7, 2));
        assert_eq!(grid.goals().collect::<Vec<_>>(), [(7, 2)]);
        assert_eq!(grid.to_goal(at(7, 2), 7, 2), Some(0));
        assert_eq!(grid.to_goal(at(7, 2), 4, 0), None, "blocked");
        assert_eq!(grid.to_goal(at(7, 2), 0, 2), grid.path_cost(at(0, 2), at(7, 2)));
        let step = grid.flow(at(7, 2), 0, 2).expect("flows");
        assert_eq!(grid.center_of(step), grid.next_step(at(0, 2), at(7, 2)).unwrap());
        assert_eq!(grid.flow(at(7, 2), 7, 2), None, "the goal has no arrow");
    }

    /// Adding the same goal twice keeps one field, and a second goal gets
    /// its own - `next_step` toward each reads its own table.
    #[test]
    fn one_field_per_goal_cell() {
        let mut grid = nine_by_five();
        grid.add_field(at(7, 2));
        grid.add_field(Position::new(at(7, 2).x + 5.0, at(7, 2).y - 5.0));
        assert_eq!(grid.fields.len(), 1);
        grid.add_field(at(0, 0));
        assert_eq!(grid.fields.len(), 2);
        assert_eq!(grid.cell_of(grid.next_step(at(0, 2), at(0, 0)).unwrap()), (0, 1));
        assert_eq!(grid.cell_of(grid.next_step(at(0, 2), at(7, 2)).unwrap()), (1, 2));
    }
}

#[cfg(test)]
mod dims_tests {
    use super::*;

    /// A grid's dims are the ceil of its size in cells, and a single
    /// obstacle shows up as exactly its blocked cell in `ascii`. Built at
    /// an explicit 48px cell with a zero margin rather than through
    /// `PATHFIND_CELL_SIZE`, so it pins `build`'s arithmetic and not the
    /// game's current choice of cell size.
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

    #[test]
    fn a_weighed_cell_is_taken_only_when_the_way_round_costs_more() {
        // 9 x 3 cells of 10 px, no obstacles, margin 0: a straight run
        // along the middle row from (1, 1) to (7, 1) is 6 steps.
        let mut grid = Grid::build(90.0, 30.0, 10.0, 0.0, std::iter::empty());
        let at = |c: usize, r: usize| Position::new(c as f32 * 10.0 + 5.0, r as f32 * 10.0 + 5.0);
        assert_eq!(grid.path_cost(at(1, 1), at(7, 1)), Some(6));
        // A ford at (4, 1) costing 3: stepping round it through row 0 or
        // row 2 is 8 unit steps, the straight run is 5 + 3 = 8 - a tie, so
        // make the ford cost 4 and the detour wins ...
        grid.weigh(std::iter::once(at(4, 1)), 4);
        assert_eq!(grid.path_cost(at(1, 1), at(7, 1)), Some(8), "the dry way round is cheaper");
        assert_ne!(grid.next_step(at(1, 1), at(7, 1)), Some(at(2, 1)).filter(|_| false), "still routes");
        // ... and a whole column of fords (rows 0..3 at col 4) leaves no
        // dry way round, so the router wades through at the ford's price.
        grid.weigh([at(4, 0), at(4, 2)].into_iter(), 4);
        assert_eq!(grid.path_cost(at(1, 1), at(7, 1)), Some(9), "5 dry steps plus one ford at 4");
        assert!(grid.usable(at(4, 1)), "a weighed cell is still open");
        assert!(!grid.blocked_ahead(at(3, 1), Position::new(1.0, 0.0)));
    }
}
