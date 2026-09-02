//! Static battlefield-map linter (Phase 3 of
//! docs/gameplay-verification-design.md): exhaustive, provable checks over
//! a round's finished static terrain. Terrain is finite (~27x15 nav-grid
//! cells at `PATHFIND_CELL_SIZE` on the default battlefield), so unlike
//! emergent AI behavior every claim here is a proof over the whole grid,
//! not a statistical sample.
//!
//! The linter never builds its own occupancy model: setup is a real,
//! seeded, headless `Game::init` on the map under test, and every cell
//! query goes through the exact `pathfind::Grid` that `Game::nav_grid`
//! hands the AI each frame (see §3.1's "same code path, or it verifies
//! the wrong thing"). `Grid` deliberately keeps its cell storage private,
//! so per-cell occupancy is read back through the same public predicates
//! `Ai::steer` itself uses (`blocked_ahead`/`boxed_in`) - which also
//! means the linter *cannot* disagree with what the AI sees.
//!
//! Run via `cargo test --lib maplint` (see `map_lint_tests` at the bottom
//! for the supported-map gate, the fixture-profile assertions, and the
//! print-only scratch-map tier).

use std::collections::{HashSet, VecDeque};
use std::fmt;

use hecs::Entity;

use crate::frog::Frog;
use crate::map::{self, CellObject};
use crate::obstacle::Obstacle;
use crate::pathfind::Grid;
use crate::simulation::Game;
use crate::tank::Tank;
use crate::{
    ENEMY_COUNT_MAX, ENEMY_COUNT_MIN, ENEMY_SPAWN_MARGIN_MAX, ENEMY_SPAWN_MARGIN_MIN,
    FROG_COLLIDER_HALF_EXTENT, OBSTACLE_CLEAR, OBSTACLE_HULL_FRACTION, OBSTACLE_SCALE,
    OBSTACLE_TEXTURE_SIZE, PATHFIND_CELL_SIZE, Position, battlefield,
};

/// How far past a target point's own footprint a tank must be able to
/// stand for that point to count as reachable (see `point_reachable`):
/// two nav-grid cells of slack on top of the worst-case tank radius (and,
/// for the frog, its own collider half-extent). Generous on purpose - a
/// target squeezed against a wall still has its nearest open cell within
/// roughly one cell of its footprint, so only genuine sealing (a closed
/// ring, a walled-off pocket) can fail it; a tighter radius would start
/// flagging legal snug placements whose exact distance depends on how
/// the 48px grid happens to align with the map's 32px tiles.
const APPROACH_SLACK: f32 = 2.0 * PATHFIND_CELL_SIZE;

/// Open components smaller than this many cells are reported as `Info`
/// (sometimes decorative slivers); at or above it they're a `Warning`
/// (likely an authoring mistake - real playable area cut off from the
/// playfield). Straight from the design doc's §3.2.1.
const DISCONNECTED_WARNING_CELLS: usize = 4;

/// How severe a finding is - `Error` findings fail the supported-map gate
/// (see `map_lint_tests::supported_maps_no_new_errors`, which ratchets
/// against `KNOWN_ERROR_BUDGET`); `Warning`/`Info` are advisory.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LintSeverity {
    Error,
    Warning,
    Info,
}

impl fmt::Display for LintSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LintSeverity::Error => "error",
            LintSeverity::Warning => "warning",
            LintSeverity::Info => "info",
        })
    }
}

/// Which check produced a finding - typed (rather than string-matched out
/// of the message) so fixture-profile tests can assert on it robustly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LintKind {
    /// The frog can't be approached from the playfield (§3.2.1).
    UnreachableFrog,
    /// A pickup slot can't be approached from the playfield (§3.2.1).
    UnreachablePickup,
    /// An open region disconnected from the playfield (§3.2.1).
    DisconnectedRegion,
    /// An open cell every one of whose neighbors is blocked (§3.2.2).
    BoxedInCell,
    /// Too few legal enemy-spawn cells in the border band (§3.2.3).
    SpawnBandTooTight,
    /// The nav grid calls a cell open that a worst-case tank physically
    /// can't occupy (§3.2.4) - the stuck-tank generator class.
    PlannerPhysicsMismatch,
    /// A single-cell-wide passage (§3.2.5) - legal but scrape-prone.
    NarrowCorridor,
}

impl LintKind {
    fn tag(self) -> &'static str {
        match self {
            LintKind::UnreachableFrog => "unreachable-frog",
            LintKind::UnreachablePickup => "unreachable-pickup",
            LintKind::DisconnectedRegion => "disconnected-region",
            LintKind::BoxedInCell => "boxed-in-cell",
            LintKind::SpawnBandTooTight => "spawn-band-too-tight",
            LintKind::PlannerPhysicsMismatch => "planner-physics-mismatch",
            LintKind::NarrowCorridor => "narrow-corridor",
        }
    }
}

/// One lint result: severity + which check + a self-contained message
/// (nav-grid cell coordinates and world px where relevant).
pub struct LintFinding {
    pub severity: LintSeverity,
    pub kind: LintKind,
    pub message: String,
}

impl fmt::Display for LintFinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} [{}]: {}", self.severity, self.kind.tag(), self.message)
    }
}

/// Read-back of the nav grid's occupancy plus the flood-filled playfield
/// membership, indexed `[row * cols + col]`. Cell geometry (cols/rows/
/// centers) mirrors `Grid::build`'s two sizing lines - a coordinate
/// convention only; the occupancy itself always comes from the real
/// `Grid` via `probe_open`, never from a reimplementation of its margin
/// logic.
struct Cells {
    cols: usize,
    rows: usize,
    open: Vec<bool>,
    playfield: Vec<bool>,
}

impl Cells {
    fn idx(&self, col: usize, row: usize) -> usize {
        row * self.cols + col
    }

    /// World-space center of a cell - same `(i + 0.5) * cell_size`
    /// convention as `Grid`'s own (private) `center_of`.
    fn center(&self, col: usize, row: usize) -> Position {
        Position::new(
            (col as f32 + 0.5) * PATHFIND_CELL_SIZE,
            (row as f32 + 0.5) * PATHFIND_CELL_SIZE,
        )
    }

    /// Which cell a world position falls in - same clamped formula as
    /// `Grid`'s own (private) `cell_of`.
    fn cell_of(&self, p: Position) -> (usize, usize) {
        let col = ((p.x / PATHFIND_CELL_SIZE) as isize).clamp(0, self.cols as isize - 1) as usize;
        let row = ((p.y / PATHFIND_CELL_SIZE) as isize).clamp(0, self.rows as isize - 1) as usize;
        (col, row)
    }

    fn is_open(&self, col: isize, row: isize) -> bool {
        if col < 0 || row < 0 || col as usize >= self.cols || row as usize >= self.rows {
            return false; // off-grid counts as blocked, matching `blocked_ahead`
        }
        self.open[row as usize * self.cols + col as usize]
    }

    fn in_playfield(&self, col: usize, row: usize) -> bool {
        self.playfield[self.idx(col, row)]
    }
}

/// Read one cell's occupancy out of the real nav grid. `Grid` exposes no
/// direct per-cell accessor (its callers only ever ask relative
/// questions), so this asks `blocked_ahead` *from an adjacent cell's
/// center, stepping into the queried cell* - every in-grid neighbor
/// agrees, since they all read the same underlying cell. Any cell on a
/// grid with at least two columns or rows has such a neighbor; a 1x1
/// grid (never a real battlefield - `Grid::build`'s `.max(1)` floor only
/// exists for degenerate inputs) has nothing to lint and reports open.
fn probe_open(grid: &Grid, cols: usize, rows: usize, col: usize, row: usize) -> bool {
    let center = |c: usize, r: usize| {
        Position::new(
            (c as f32 + 0.5) * PATHFIND_CELL_SIZE,
            (r as f32 + 0.5) * PATHFIND_CELL_SIZE,
        )
    };
    let (from, dir) = if col > 0 {
        ((col - 1, row), Position::new(1.0, 0.0))
    } else if col + 1 < cols {
        ((col + 1, row), Position::new(-1.0, 0.0))
    } else if row > 0 {
        ((col, row - 1), Position::new(0.0, 1.0))
    } else if row + 1 < rows {
        ((col, row + 1), Position::new(0.0, -1.0))
    } else {
        return true;
    };
    !grid.blocked_ahead(center(from.0, from.1), dir)
}

/// Lint the round `game` currently holds (call right after a headless
/// seeded `Game::init` - see the module doc). `width`/`height` must be
/// the same dimensions `init` ran with. Returns every finding, in check
/// order (§3.2's numbering); an empty vec is a fully clean map.
pub fn lint(game: &Game, width: f32, height: f32) -> Vec<LintFinding> {
    let grid = game.nav_grid(width, height);
    let cols = ((width / PATHFIND_CELL_SIZE).ceil() as usize).max(1);
    let rows = ((height / PATHFIND_CELL_SIZE).ceil() as usize).max(1);

    let mut open = vec![false; cols * rows];
    for row in 0..rows {
        for col in 0..cols {
            open[row * cols + col] = probe_open(&grid, cols, rows, col, row);
        }
    }

    // The player's actual spawned position seeds the playfield flood fill
    // - `init` already resolved the map's `Start` cell (or the
    // center-fallback), so reading it back means zero duplicated spawn
    // logic. Its cell counts as playfield even if blocked (a start inside
    // an obstacle's conservative margin is legal - `next_step` treats
    // start/goal cells as open for exactly the same reason).
    let player_entity = game.player.expect("lint runs on an initialized game");
    let (player_pos, player_size) = {
        let mut query = game.world.query::<(Entity, &Tank)>();
        query
            .iter()
            .find(|(entity, _)| *entity == player_entity)
            .map(|(_, tank)| (tank.position, tank.size()))
            .expect("player tank exists after init")
    };

    let mut cells = Cells { cols, rows, open, playfield: vec![false; cols * rows] };
    let start_cell = cells.cell_of(player_pos);
    flood(&mut cells, start_cell);

    let obstacle_positions: Vec<Position> =
        game.world.query::<&Obstacle>().iter().map(|o| o.position).collect();
    let frog_pos = game.world.query::<&Frog>().iter().next().map(|f| f.position);

    let mut findings = Vec::new();
    check_reachability(game, &cells, frog_pos, &mut findings);
    check_disconnected_regions(&cells, &mut findings);
    check_boxed_in(&grid, &cells, &mut findings);
    check_spawn_band(
        game,
        &cells,
        width,
        height,
        player_pos,
        player_size,
        &obstacle_positions,
        &mut findings,
    );
    check_planner_physics(&cells, &physics_boxes(&obstacle_positions, frog_pos), &mut findings);
    check_narrow_corridors(&cells, &mut findings);
    findings
}

/// BFS the open region reachable from `seed`, marking `playfield`. The
/// seed cell is always included (see the start-cell note in `lint`).
fn flood(cells: &mut Cells, seed: (usize, usize)) {
    let mut queue = VecDeque::new();
    cells.playfield[seed.1 * cells.cols + seed.0] = true;
    queue.push_back(seed);
    while let Some((col, row)) = queue.pop_front() {
        for (dc, dr) in [(0i32, -1i32), (0, 1), (-1, 0), (1, 0)] {
            let (nc, nr) = (col as i32 + dc, row as i32 + dr);
            if !cells.is_open(nc as isize, nr as isize) {
                continue;
            }
            let idx = nr as usize * cells.cols + nc as usize;
            if !cells.playfield[idx] {
                cells.playfield[idx] = true;
                queue.push_back((nc as usize, nr as usize));
            }
        }
    }
}

/// §3.2.1 (targets): the frog and every map pickup slot must be
/// approachable from the playfield. "Approachable" is a radius test
/// against playfield cell centers rather than "its own cell is
/// playfield", because both targets legitimately sit on blocked cells:
/// the frog *is* an obstacle in the nav grid (its own cell is always
/// blocked by its own margin), and a pickup tucked beside a wall sits
/// inside that wall's conservative margin while remaining perfectly
/// collectable. See `APPROACH_SLACK` for the radius rationale.
fn check_reachability(
    game: &Game,
    cells: &Cells,
    frog_pos: Option<Position>,
    findings: &mut Vec<LintFinding>,
) {
    let tank_radius = battlefield::max_tank_avoidance_radius();
    if let Some(frog) = frog_pos {
        let frog_reach =
            FROG_COLLIDER_HALF_EXTENT.0.max(FROG_COLLIDER_HALF_EXTENT.1) + tank_radius + APPROACH_SLACK;
        if !point_reachable(cells, frog, frog_reach) {
            findings.push(LintFinding {
                severity: LintSeverity::Error,
                kind: LintKind::UnreachableFrog,
                message: format!(
                    "frog at ({:.0},{:.0}) has no playfield cell within {frog_reach:.0}px - tanks can never reach it",
                    frog.x, frog.y
                ),
            });
        }
    }
    let pickup_reach = tank_radius + APPROACH_SLACK;
    for (col, row, obj) in game.map.iter_cells() {
        // The kind doesn't matter for reachability (and `PickupKind`
        // deliberately isn't matched here, so new kinds lint for free).
        if !matches!(obj, CellObject::Pickup { .. }) {
            continue;
        }
        let pos = map::cell_to_world(col, row);
        if !point_reachable(cells, pos, pickup_reach) {
            findings.push(LintFinding {
                severity: LintSeverity::Error,
                kind: LintKind::UnreachablePickup,
                message: format!(
                    "pickup slot at map cell ({col},{row}) = ({:.0},{:.0}) has no playfield cell within {pickup_reach:.0}px",
                    pos.x, pos.y
                ),
            });
        }
    }
}

/// True if some playfield cell's center lies within `reach` of `pos`.
fn point_reachable(cells: &Cells, pos: Position, reach: f32) -> bool {
    for row in 0..cells.rows {
        for col in 0..cells.cols {
            if cells.in_playfield(col, row) && cells.center(col, row).distance_to(pos) <= reach {
                return true;
            }
        }
    }
    false
}

/// §3.2.1 (regions): every open component that isn't the playfield.
fn check_disconnected_regions(cells: &Cells, findings: &mut Vec<LintFinding>) {
    let mut seen = cells.playfield.clone();
    for row in 0..cells.rows {
        for col in 0..cells.cols {
            if seen[cells.idx(col, row)] || !cells.is_open(col as isize, row as isize) {
                continue;
            }
            // Collect this whole component before reporting it once.
            let mut component = Vec::new();
            let mut queue = VecDeque::from([(col, row)]);
            seen[cells.idx(col, row)] = true;
            while let Some((c, r)) = queue.pop_front() {
                component.push((c, r));
                for (dc, dr) in [(0i32, -1i32), (0, 1), (-1, 0), (1, 0)] {
                    let (nc, nr) = (c as i32 + dc, r as i32 + dr);
                    if cells.is_open(nc as isize, nr as isize)
                        && !seen[nr as usize * cells.cols + nc as usize]
                    {
                        seen[nr as usize * cells.cols + nc as usize] = true;
                        queue.push_back((nc as usize, nr as usize));
                    }
                }
            }
            let severity = if component.len() >= DISCONNECTED_WARNING_CELLS {
                LintSeverity::Warning
            } else {
                LintSeverity::Info
            };
            let (c0, r0) = component[0];
            findings.push(LintFinding {
                severity,
                kind: LintKind::DisconnectedRegion,
                message: format!(
                    "{} open cell(s) unreachable from the playfield, e.g. grid cell ({c0},{r0}) around ({:.0},{:.0})",
                    component.len(),
                    cells.center(c0, r0).x,
                    cells.center(c0, r0).y
                ),
            });
        }
    }
}

/// §3.2.2: open cells `Grid::boxed_in` says nothing can route out of.
fn check_boxed_in(grid: &Grid, cells: &Cells, findings: &mut Vec<LintFinding>) {
    for row in 0..cells.rows {
        for col in 0..cells.cols {
            if !cells.is_open(col as isize, row as isize) {
                continue;
            }
            let center = cells.center(col, row);
            if grid.boxed_in(center) {
                findings.push(LintFinding {
                    severity: LintSeverity::Error,
                    kind: LintKind::BoxedInCell,
                    message: format!(
                        "grid cell ({col},{row}) around ({:.0},{:.0}) is open but every neighbor is blocked - unusable, and a trap for knocked-back tanks",
                        center.x, center.y
                    ),
                });
            }
        }
    }
}

/// §3.2.3: enough legal enemy-spawn cells in the border band. Mirrors the
/// clearance predicate of `Game::init`'s enemy loop cell-by-cell (sample
/// domain, band depth, player clearance, per-tile obstacle clearance) -
/// minus the enemy-vs-enemy spacing term, which depends on where earlier
/// enemies landed rather than on the map. Cell count is a capacity
/// *proxy* (tanks also need ~1.5-tank spacing between each other), so the
/// thresholds are deliberately the loosest defensible ones: fewer cells
/// than tanks is provably too tight (`Error`), fewer than
/// `ENEMY_COUNT_MAX` merely suspicious (`Warning`) - the sampler would
/// degrade into its attempt cap and cram spawns in anyway, its documented
/// worst case (see `PLACEMENT_MAX_ATTEMPTS`).
#[allow(clippy::too_many_arguments)] // plumbing lint context, not an API
fn check_spawn_band(
    game: &Game,
    cells: &Cells,
    width: f32,
    height: f32,
    player_pos: Position,
    player_size: f32,
    obstacle_positions: &[Position],
    findings: &mut Vec<LintFinding>,
) {
    let short_side = width.min(height);
    let margin_min = short_side * ENEMY_SPAWN_MARGIN_MIN;
    let margin_max = short_side * ENEMY_SPAWN_MARGIN_MAX;
    let clear = player_size * 2.0;
    let enemy_clear = player_size * 1.5;

    let mut capacity = 0usize;
    for row in 0..cells.rows {
        for col in 0..cells.cols {
            if !cells.is_open(col as isize, row as isize) || !cells.in_playfield(col, row) {
                continue;
            }
            let p = cells.center(col, row);
            let in_domain = p.x >= margin_min
                && p.x <= width - margin_min
                && p.y >= margin_min
                && p.y <= height - margin_min;
            let border_dist = p.x.min(width - p.x).min(p.y).min(height - p.y);
            if in_domain
                && border_dist <= margin_max
                && p.distance_to(player_pos) >= clear
                && obstacle_positions
                    .iter()
                    .all(|&o| p.distance_to(o) >= enemy_clear + OBSTACLE_CLEAR)
            {
                capacity += 1;
            }
        }
    }

    // Same 1..=31 clamp `Game::init` applies to a map's `tanks` count (31
    // = the rapier collision-group bit budget - see `enemy_owner_slot`).
    let required = game
        .map
        .tanks
        .map(|n| (n as usize).clamp(1, 31))
        .unwrap_or(ENEMY_COUNT_MIN);
    if capacity < required {
        findings.push(LintFinding {
            severity: LintSeverity::Error,
            kind: LintKind::SpawnBandTooTight,
            message: format!(
                "only {capacity} legal enemy-spawn cell(s) in the border band for {required} tank(s) - spawning will hit the rejection-sampling attempt cap"
            ),
        });
    } else if capacity < ENEMY_COUNT_MAX {
        findings.push(LintFinding {
            severity: LintSeverity::Warning,
            kind: LintKind::SpawnBandTooTight,
            message: format!(
                "only {capacity} legal enemy-spawn cell(s) in the border band (fewer than ENEMY_COUNT_MAX = {ENEMY_COUNT_MAX}) - high tank counts will crowd or degrade to the attempt cap"
            ),
        });
    }
}

/// The physics-side AABBs the §3.2.4 agreement check tests against:
/// every obstacle at its *seam-widened* per-axis half-extents (the same
/// `tile_hull_half_extent` call `spawn_from_map` sized the real colliders
/// with - the nav grid, by contrast, only ever saw a scalar
/// `hull_size()/2`), plus the frog's collider.
fn physics_boxes(
    obstacle_positions: &[Position],
    frog_pos: Option<Position>,
) -> Vec<(Position, Position)> {
    let base = OBSTACLE_TEXTURE_SIZE * OBSTACLE_SCALE * OBSTACLE_HULL_FRACTION * 0.5;
    let wall_cells: HashSet<(i32, i32)> = obstacle_positions
        .iter()
        .map(|&p| battlefield::pos_to_cell(p))
        .collect();
    let mut boxes: Vec<(Position, Position)> = obstacle_positions
        .iter()
        .map(|&p| {
            let (gx, gy) = battlefield::pos_to_cell(p);
            (p, battlefield::tile_hull_half_extent(&wall_cells, gx, gy, base))
        })
        .collect();
    if let Some(frog) = frog_pos {
        boxes.push((
            frog,
            Position::new(FROG_COLLIDER_HALF_EXTENT.0, FROG_COLLIDER_HALF_EXTENT.1),
        ));
    }
    boxes
}

/// §3.2.4: for every open cell, a worst-case tank square centered on the
/// cell must not overlap any physics collider - if it does, the grid is
/// telling the AI "drive here" while the solver says no, the exact
/// mismatch class behind the historical frog-stuck bug. Under today's
/// constants this check *cannot* fire (the grid's scalar margin is
/// strictly more conservative than any collider extent - the worked
/// numbers are in the design doc), which is the point: it's a tripwire
/// for the two sides drifting apart, e.g. someone widening
/// `tile_hull_half_extent` or shrinking the grid margin independently.
/// (The converse - blocked cell that's physically fine - is expected
/// conservatism and deliberately not reported.)
fn check_planner_physics(
    cells: &Cells,
    boxes: &[(Position, Position)],
    findings: &mut Vec<LintFinding>,
) {
    let tank_half = battlefield::max_tank_avoidance_radius();
    for row in 0..cells.rows {
        for col in 0..cells.cols {
            if !cells.is_open(col as isize, row as isize) {
                continue;
            }
            let center = cells.center(col, row);
            for &(pos, half) in boxes {
                if aabb_overlap(center, tank_half, pos, half) {
                    findings.push(LintFinding {
                        severity: LintSeverity::Error,
                        kind: LintKind::PlannerPhysicsMismatch,
                        message: format!(
                            "grid cell ({col},{row}) around ({:.0},{:.0}) is open but a worst-case tank there overlaps the collider at ({:.0},{:.0})",
                            center.x, center.y, pos.x, pos.y
                        ),
                    });
                    break; // one report per cell is enough
                }
            }
        }
    }
}

/// Strict AABB overlap between a square of half-extent `a_half` at `a`
/// and a rectangle of per-axis half-extents `b_half` at `b` - touching
/// edges do not count (a tank flush against a collider face is exactly
/// where the solver legitimately rests it).
fn aabb_overlap(a: Position, a_half: f32, b: Position, b_half: Position) -> bool {
    (a.x - b.x).abs() < a_half + b_half.x && (a.y - b.y).abs() < a_half + b_half.y
}

/// §3.2.5: playfield cells forming single-cell-wide passages - open, both
/// neighbors on one axis blocked (or off-grid), and at least one neighbor
/// on the other axis open so traffic actually flows through rather than
/// dead-ending (an all-four-blocked cell is `check_boxed_in`'s finding,
/// not a corridor). Legal, but every tank passing through drives at the
/// clearance limit - read Phase 4's wall-grind numbers on such maps with
/// this in mind.
fn check_narrow_corridors(cells: &Cells, findings: &mut Vec<LintFinding>) {
    for row in 0..cells.rows {
        for col in 0..cells.cols {
            if !cells.is_open(col as isize, row as isize) || !cells.in_playfield(col, row) {
                continue;
            }
            let (c, r) = (col as isize, row as isize);
            let x_sealed = !cells.is_open(c - 1, r) && !cells.is_open(c + 1, r);
            let y_sealed = !cells.is_open(c, r - 1) && !cells.is_open(c, r + 1);
            let x_flows = cells.is_open(c - 1, r) || cells.is_open(c + 1, r);
            let y_flows = cells.is_open(c, r - 1) || cells.is_open(c, r + 1);
            if (x_sealed && y_flows) || (y_sealed && x_flows) {
                let center = cells.center(col, row);
                findings.push(LintFinding {
                    severity: LintSeverity::Info,
                    kind: LintKind::NarrowCorridor,
                    message: format!(
                        "grid cell ({col},{row}) around ({:.0},{:.0}) is a single-cell-wide passage",
                        center.x, center.y
                    ),
                });
            }
        }
    }
}

#[cfg(test)]
mod map_lint_tests {
    use super::*;
    use crate::map::MapFile;
    use crate::obstacle::Material;
    use crate::pickup::PickupKind;
    use crate::{DEFAULT_SCREEN_HEIGHT, DEFAULT_SCREEN_WIDTH};

    const W: f32 = DEFAULT_SCREEN_WIDTH as f32;
    const H: f32 = DEFAULT_SCREEN_HEIGHT as f32;

    /// Maps the game actually ships/loads by default - gated by
    /// `supported_maps_no_new_errors` against `KNOWN_ERROR_BUDGET` below.
    /// Grow this list as maps graduate from scratch to shipped.
    const SUPPORTED_MAPS: &[&str] = &["maps/default.toml"];

    /// Real, recorded map debt in the supported maps: `Error` *kinds* the
    /// linter is right about but that predate it (found the day it landed
    /// - see docs/gameplay-verification-design.md §3's landed notes). The
    /// gate accepts errors of these recorded kinds and fails on any kind
    /// outside the list - so known debt doesn't block the build, while a
    /// brand-new error class (a boxed-in cell, a planner/physics
    /// mismatch, an unreachable frog) still fails instantly. Deliberately
    /// kinds, not counts: default.toml is under active hand-editing (its
    /// unreachable-pickup count grew from 5 to 12 *while this gate was
    /// being written*), and count-ratcheting a live canvas just flaps.
    /// Once the map settles, tighten this back to per-kind counts and
    /// burn the debt down by editing the map - never grow the list to
    /// make the test pass.
    ///
    /// default.toml (2026-08-27): a strip of pickups along the top edge
    /// that no playfield cell center comes within approach reach of, and
    /// a border band with *zero* cells passing the enemy-spawn legality
    /// predicate - so every enemy spawn on this map degrades to
    /// `sample_clear_position`'s attempt-cap fallback (a very plausible
    /// source of this map's recorded stale-start/stall anomaly baseline).
    const KNOWN_ERROR_KINDS: &[(&str, &[LintKind])] = &[(
        "maps/default.toml",
        &[LintKind::UnreachablePickup, LintKind::SpawnBandTooTight],
    )];

    /// Headless seeded round on `map`, linted - the §3.1 setup. The fixed
    /// seed matters for maps that leave frog/start placement to `init`'s
    /// (seeded) fallback rolls: same seed, same layout, same findings.
    fn lint_map(map: MapFile) -> Vec<LintFinding> {
        let mut game = Game::default();
        game.seed_override = Some(0xB0B5);
        game.enemy_count_override = Some(4);
        game.map = map;
        game.init(W, H);
        lint(&game, W, H)
    }

    fn wall(map: &mut MapFile, col: i32, row: i32) {
        map.set_cell(col, row, CellObject::Wall { material: Material::Iron });
    }

    /// The pockets-fixture ring: a sealed square of walls whose interior
    /// is exactly one open-but-boxed-in nav cell (the geometry worked out
    /// in maps/test/pockets.toml's own header).
    /// A closed wall ring whose interior is exactly ONE open nav-grid cell
    /// (every neighbor blocked - the `boxed_in` pathology). Sized against
    /// `Grid::build`'s center-inside-reach blocking rule: walls at cols
    /// 14/18 put the open x-band at (492, 532), containing only the col-10
    /// center x=504; rows 8/12 likewise leave only the row-6 center y=312.
    /// A wider ring (e.g. the 14..20 x 8..14 one this test originally
    /// used) yields a 2x2 open interior instead, where no cell is boxed -
    /// each has an open neighbor - and only `DisconnectedRegion` fires.
    fn sealed_ring(map: &mut MapFile) {
        for row in 8..=12 {
            wall(map, 14, row);
            wall(map, 18, row);
        }
        for col in 14..=18 {
            wall(map, col, 8);
            wall(map, col, 12);
        }
    }

    /// A wider closed ring (2x2 open interior, so nothing in it is
    /// boxed-in) for burying a frog/pickup deep enough that no playfield
    /// cell center lies within their approach reach (~128px pickups,
    /// ~150px frog) - `sealed_ring`'s single-cell interior sits only
    /// ~136px from the nearest playfield center, inside the frog's reach.
    fn sealed_vault(map: &mut MapFile) {
        for row in 8..=14 {
            wall(map, 14, row);
            wall(map, 20, row);
        }
        for col in 14..=20 {
            wall(map, col, 8);
            wall(map, col, 14);
        }
    }

    fn base_map() -> MapFile {
        let mut map = MapFile::new();
        map.set_cell(27, 11, CellObject::Start);
        map.set_cell(7, 12, CellObject::Frog);
        map
    }

    fn has(findings: &[LintFinding], kind: LintKind) -> bool {
        findings.iter().any(|f| f.kind == kind)
    }

    fn errors(findings: &[LintFinding]) -> Vec<&LintFinding> {
        findings.iter().filter(|f| f.severity == LintSeverity::Error).collect()
    }

    fn dump(label: &str, findings: &[LintFinding]) {
        println!("--- {label}: {} finding(s)", findings.len());
        for f in findings {
            println!("    {f}");
        }
    }

    // --- synthetic maps proving each check fires ---

    #[test]
    fn empty_map_is_clean() {
        let findings = lint_map(base_map());
        dump("empty", &findings);
        assert!(findings.is_empty(), "an all-open map should produce zero findings");
    }

    #[test]
    fn ring_interior_is_boxed_in() {
        let mut map = base_map();
        sealed_ring(&mut map);
        let findings = lint_map(map);
        dump("ring", &findings);
        assert!(has(&findings, LintKind::BoxedInCell));
        // The interior is also its own tiny (Info-sized) disconnected
        // component.
        assert!(has(&findings, LintKind::DisconnectedRegion));
    }

    #[test]
    fn sealed_pickup_is_unreachable() {
        let mut map = base_map();
        sealed_vault(&mut map);
        map.set_cell(17, 11, CellObject::Pickup { pickup: PickupKind::Health });
        let findings = lint_map(map);
        dump("sealed pickup", &findings);
        assert!(has(&findings, LintKind::UnreachablePickup));
    }

    #[test]
    fn sealed_frog_is_unreachable() {
        let mut map = MapFile::new();
        map.set_cell(27, 11, CellObject::Start);
        sealed_vault(&mut map);
        // One cell further in than the pickup test's slot: the frog's
        // approach reach is ~150px (its own collider on top of the
        // tank radius), and (17,11)'s ~152px to the nearest playfield
        // center would pass by only ~2px - too fragile against constant
        // tweaks to be what this test hinges on.
        map.set_cell(16, 11, CellObject::Frog);
        let findings = lint_map(map);
        dump("sealed frog", &findings);
        assert!(has(&findings, LintKind::UnreachableFrog));
    }

    #[test]
    fn split_field_flags_disconnected_region() {
        let mut map = MapFile::new();
        map.set_cell(28, 11, CellObject::Start);
        map.set_cell(30, 5, CellObject::Frog);
        for row in 1..=21 {
            wall(&mut map, 20, row); // full-height wall, no gap
        }
        let findings = lint_map(map);
        dump("split field", &findings);
        assert!(
            findings.iter().any(|f| f.kind == LintKind::DisconnectedRegion
                && f.severity == LintSeverity::Warning),
            "the sealed-off left half is a Warning-sized region"
        );
    }

    #[test]
    fn walled_band_has_no_spawn_capacity() {
        let mut map = base_map();
        map.tanks = Some(10);
        // A wall rectangle running through the middle of the enemy spawn
        // band - every band cell ends up within the obstacle-clearance
        // radius of some tile.
        for col in 5..=35 {
            wall(&mut map, col, 7);
            wall(&mut map, col, 15);
        }
        for row in 8..=14 {
            wall(&mut map, 5, row);
            wall(&mut map, 35, row);
        }
        let findings = lint_map(map);
        dump("walled band", &findings);
        assert!(
            findings.iter().any(|f| f.kind == LintKind::SpawnBandTooTight
                && f.severity == LintSeverity::Error),
            "10 tanks with a carpeted band must be an Error"
        );
    }

    #[test]
    fn single_cell_lane_flags_narrow_corridor() {
        let mut map = MapFile::new();
        map.set_cell(20, 11, CellObject::Start); // inside the lane
        map.set_cell(20, 19, CellObject::Frog);
        for col in 10..=30 {
            wall(&mut map, col, 8);
            wall(&mut map, col, 14);
        }
        let findings = lint_map(map);
        dump("lane", &findings);
        assert!(has(&findings, LintKind::NarrowCorridor));
    }

    /// §3.2.4's detector logic, exercised directly: under the game's real
    /// margin the check can never fire (see `check_planner_physics`'s doc
    /// comment), so the firing case is proven against a `Grid` built with
    /// the margin zeroed out - same real `Grid::build`, just without the
    /// conservatism that normally guarantees agreement.
    #[test]
    fn planner_physics_mismatch_fires_without_the_margin() {
        let obstacle = (Position::new(400.0, 300.0), 12.0);
        let boxes = vec![(obstacle.0, Position::new(16.0, 12.0))];
        let cols = ((W / PATHFIND_CELL_SIZE).ceil() as usize).max(1);
        let rows = ((H / PATHFIND_CELL_SIZE).ceil() as usize).max(1);

        let lint_against = |margin: f32| {
            let grid = Grid::build(W, H, PATHFIND_CELL_SIZE, margin, [obstacle].into_iter());
            let mut open = vec![false; cols * rows];
            for row in 0..rows {
                for col in 0..cols {
                    open[row * cols + col] = probe_open(&grid, cols, rows, col, row);
                }
            }
            let cells = Cells { cols, rows, playfield: open.clone(), open };
            let mut findings = Vec::new();
            check_planner_physics(&cells, &boxes, &mut findings);
            findings
        };

        assert!(
            !lint_against(0.0).is_empty(),
            "with no margin, cells beside the obstacle are open yet a tank there overlaps it"
        );
        assert!(
            lint_against(battlefield::max_tank_avoidance_radius()).is_empty(),
            "the real margin keeps every open cell clear of every collider"
        );
    }

    // --- on-disk maps ---

    fn lint_path(path: &str) -> Result<Vec<LintFinding>, String> {
        MapFile::load(std::path::Path::new(path)).map(lint_map)
    }

    #[test]
    fn supported_maps_no_new_errors() {
        for path in SUPPORTED_MAPS {
            let findings = lint_path(path).expect("supported map must load");
            dump(path, &findings);
            let allowed: &[LintKind] = KNOWN_ERROR_KINDS
                .iter()
                .find(|(p, _)| p == path)
                .map(|(_, kinds)| *kinds)
                .unwrap_or(&[]);
            for err in errors(&findings) {
                assert!(
                    allowed.contains(&err.kind),
                    "{path}: a NEW error class the recorded debt doesn't cover: {err}",
                );
            }
        }
    }

    /// Each fixture must keep provoking what it was built to provoke - a
    /// fixture that lints clean of its intended profile is itself broken
    /// (see maps/test/*.toml's own headers and the design doc §2.2/§3.3).
    #[test]
    fn fixture_maps_match_their_intended_profiles() {
        let f = lint_path("maps/test/pockets.toml").expect("fixture loads");
        dump("pockets", &f);
        // The fixture's sealed ring interior under cell-center
        // rasterization: a boxed-in cell when it shakes out to a single
        // open cell, a small disconnected pocket when wider - either way
        // a sealed pocket (see `ring_interior_is_boxed_in` for the
        // worked single-cell geometry).
        assert!(
            has(&f, LintKind::DisconnectedRegion) || has(&f, LintKind::BoxedInCell),
            "pockets' ring interior is a sealed pocket"
        );

        // choke/tight-corridors/frog-block lanes are two grid cells wide
        // under `Grid::build`'s center-blocking rule (they were single-cell
        // under the older overlap rule they were first cut against), so
        // `NarrowCorridor` rightly does NOT fire - their provocations are
        // behavioral (funneling, corridor scraping), not static lint
        // signatures. What the linter owes them is "fully legal": these
        // must never accidentally become illegal maps.
        let f = lint_path("maps/test/choke.toml").expect("fixture loads");
        dump("choke", &f);
        assert!(errors(&f).is_empty(), "choke is tight but fully legal");

        let f = lint_path("maps/test/tight-corridors.toml").expect("fixture loads");
        dump("tight-corridors", &f);
        // The 21-tile rails eat most of the border band's tile clearance:
        // the linter counts only ~3 legal spawn cells for the map's 4
        // tanks, so spawn pressure (attempt-cap fallback spawns) is part
        // of this fixture's recorded profile - but nothing else about it
        // may be illegal.
        assert!(
            errors(&f).iter().all(|e| e.kind == LintKind::SpawnBandTooTight),
            "tight-corridors may only carry its known spawn-band pressure"
        );
        assert!(has(&f, LintKind::SpawnBandTooTight), "the rails squeeze the band by design");

        let f = lint_path("maps/test/frog-block.toml").expect("fixture loads");
        dump("frog-block", &f);
        assert!(errors(&f).is_empty(), "the corridor structure itself is legal");
        assert!(
            !has(&f, LintKind::UnreachableFrog),
            "the plugging frog is reachable from the corridor mouths by design"
        );

        let f = lint_path("maps/test/maze.toml").expect("fixture loads");
        dump("maze", &f);
        assert!(errors(&f).is_empty(), "maze is tight but fully legal");

        let f = lint_path("maps/test/u-trap.toml").expect("fixture loads");
        dump("u-trap", &f);
        assert!(errors(&f).is_empty(), "the trap pocket is open and reachable");
    }

    /// Everything else under maps/ is scratch: lint-and-print only
    /// (`--nocapture` to see it), never a build failure - a half-finished
    /// editor session must not break `cargo test`.
    #[test]
    fn scratch_maps_lint_report_only() {
        let Ok(dir) = std::fs::read_dir("maps") else { return };
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }
            let display = path.display().to_string();
            if SUPPORTED_MAPS.contains(&display.as_str()) {
                continue;
            }
            match lint_path(&display) {
                Ok(findings) => dump(&display, &findings),
                Err(e) => println!("--- {display}: skipped ({e})"),
            }
        }
    }
}
