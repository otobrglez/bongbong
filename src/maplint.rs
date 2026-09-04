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

use crate::tuning::tuning;
use std::collections::{HashSet, VecDeque};
use std::fmt;

use hecs::Entity;

use crate::frog::{Frog, Side};
use crate::level::{Mission, SpawnKind};
use crate::map::{self, CellObject};
use crate::obstacle::Obstacle;
use crate::pathfind::Grid;
use crate::simulation::Game;
use crate::tank::Tank;
use crate::{
    FROG_COLLIDER_HALF_EXTENT,
    OBSTACLE_HULL_FRACTION,
    OBSTACLE_SCALE,
    OBSTACLE_TEXTURE_SIZE,
    PATHFIND_CELL_SIZE,
    Position,
    battlefield,
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
    /// A pickup slot can't be approached from the playfield even with
    /// every destructible wall shot away (§3.2.1).
    UnreachablePickup,
    /// A pickup slot only approachable once one or more destructible
    /// (non-Iron) walls are destroyed (§3.2.1) - deliberate gated loot the
    /// player can breach, but which the AI (which routes on intact
    /// terrain) will never go for.
    GatedPickup,
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
    /// A `gate` cell that is not on a nav-grid edge cell - a wave tank
    /// rolls in from outside the boundary, so an interior gate has no
    /// outside to come from.
    GateNotOnEdge,
    /// An edge `gate` whose lane inward (the gate cell plus
    /// `wave_gate_inward_cells` cells toward the interior) is not entirely
    /// `Grid::usable` - a tank would arrive on a cell it cannot leave.
    GateBlocked,
    /// A waves-plan map with no `gate` cell whose intact terrain offers
    /// the automatic edge scan (`battlefield::gate_candidates`) no lane
    /// either: every wave would fall back to the spawn band, so nothing
    /// ever rolls in.
    WavesNoGates,
    /// A Hunt map with no `enemy_frog` cell: the round falls back to a
    /// procedural spot in the enemy spawn band.
    HuntMissingEnemyFrog,
    /// The `enemy_frog` cell can't be approached from the playfield under
    /// the same reach rule as the player's frog.
    EnemyFrogUnreachable,
}

impl LintKind {
    fn tag(self) -> &'static str {
        match self {
            LintKind::UnreachableFrog => "unreachable-frog",
            LintKind::UnreachablePickup => "unreachable-pickup",
            LintKind::GatedPickup => "gated-pickup",
            LintKind::DisconnectedRegion => "disconnected-region",
            LintKind::BoxedInCell => "boxed-in-cell",
            LintKind::SpawnBandTooTight => "spawn-band-too-tight",
            LintKind::PlannerPhysicsMismatch => "planner-physics-mismatch",
            LintKind::NarrowCorridor => "narrow-corridor",
            LintKind::GateNotOnEdge => "gate-not-on-edge",
            LintKind::GateBlocked => "gate-blocked",
            LintKind::WavesNoGates => "waves-no-gates",
            LintKind::HuntMissingEnemyFrog => "hunt-missing-enemy-frog",
            LintKind::EnemyFrogUnreachable => "enemy-frog-unreachable",
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

    // The same playfield once every destructible wall is gone: only Iron
    // (never destroyed - see `obstacle::Material`) and the frog still
    // block. Built exactly like `Game::nav_grid`, minus the breakable
    // tiles, so "reachable by breaching" is the AI's own occupancy rule
    // applied to the terrain a player can shoot their way to.
    let breach_grid = Grid::build(
        width,
        height,
        PATHFIND_CELL_SIZE,
        battlefield::max_tank_avoidance_radius(),
        game.world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| o.material.is_permanent())
            .map(|o| (o.position, o.hull_size() * 0.5))
            .chain(game.world.query::<&Frog>().iter().map(|fr| {
                (fr.position, FROG_COLLIDER_HALF_EXTENT.0.max(FROG_COLLIDER_HALF_EXTENT.1))
            })),
    );
    let mut breach_open = vec![false; cols * rows];
    for row in 0..rows {
        for col in 0..cols {
            breach_open[row * cols + col] = probe_open(&breach_grid, cols, rows, col, row);
        }
    }
    let mut breach_cells = Cells { cols, rows, open: breach_open, playfield: vec![false; cols * rows] };
    flood(&mut breach_cells, start_cell);

    let obstacle_positions: Vec<Position> =
        game.world.query::<&Obstacle>().iter().map(|o| o.position).collect();
    let frog_pos = game.world.query::<&Frog>().iter().next().map(|f| f.position);

    let mut findings = Vec::new();
    check_reachability(game, &cells, &breach_cells, frog_pos, &mut findings);
    check_enemy_frog(game, &cells, &mut findings);
    check_gates(game, &grid, &mut findings);
    check_wave_gates(game, &grid, width, height, player_pos, &mut findings);
    check_disconnected_regions(&cells, &mut findings);
    check_boxed_in(&grid, &cells, &mut findings);
    // Only the band plan places enemies in the border band at init; a
    // waves plan rolls them in through gates, so band capacity is moot.
    if game.map.spawn.kind == SpawnKind::Band {
        check_spawn_band(
            game,
            &cells,
            width,
            height,
            player_pos,
            player_size,
            &grid,
            &obstacle_positions,
            &mut findings,
        );
    }
    check_planner_physics(&cells, &physics_boxes(&obstacle_positions, frog_pos), &mut findings);
    check_narrow_corridors(&cells, &mut findings);
    findings
}

/// The Hunt mission's target: a Hunt map without an `enemy_frog` cell
/// plays on a procedural spot in the enemy spawn band (a warning - the
/// author most likely meant to place one), and a placed cell must be
/// approachable from the playfield under the player frog's own reach rule
/// (`check_reachability`), whatever the map's mission - an error, since
/// hunters could never get to it. Read from the map cell rather than the
/// world so the check holds for every mission the map might be run under.
fn check_enemy_frog(game: &Game, cells: &Cells, findings: &mut Vec<LintFinding>) {
    let map = &game.map;
    let Some((col, row)) = map.enemy_frog_cell() else {
        if map.mission.kind == Mission::Hunt {
            findings.push(LintFinding {
                severity: LintSeverity::Warning,
                kind: LintKind::HuntMissingEnemyFrog,
                message: "hunt mission with no enemy_frog cell - the round falls back to a procedural spot in the enemy spawn band".to_string(),
            });
        }
        return;
    };
    let pos = map::cell_to_world(col, row);
    let reach = FROG_COLLIDER_HALF_EXTENT.0.max(FROG_COLLIDER_HALF_EXTENT.1)
        + battlefield::max_tank_avoidance_radius()
        + APPROACH_SLACK;
    if !point_reachable(cells, pos, reach) {
        findings.push(LintFinding {
            severity: LintSeverity::Error,
            kind: LintKind::EnemyFrogUnreachable,
            message: format!(
                "enemy_frog at map cell ({col},{row}) = ({:.0},{:.0}) has no playfield cell within {reach:.0}px - hunters can never reach it",
                pos.x, pos.y
            ),
        });
    }
}

/// Explicit `gate` cells (docs/maps-to-levels.md "Gates and roll-in"):
/// each must sit on a nav-grid edge cell - col 0, the last col, row 0 or
/// the last row of `grid` - and its lane inward must be entirely
/// `Grid::usable`. The lane is exactly what `battlefield::gate_candidates`
/// walks: `wave_gate_inward_cells` cells counting the gate cell itself
/// (so the innermost, where the body spawns, is `inward - 1` cells in),
/// straight in from that edge, a corner taking its side edge - the same
/// rule `gates_from_cells` applies when the round turns these cells into
/// lanes, so a gate this check passes is one the game will use.
fn check_gates(game: &Game, grid: &Grid, findings: &mut Vec<LintFinding>) {
    let (cols, rows, cell) = grid.dims();
    let (cols, rows) = (cols as isize, rows as isize);
    let inward_cells = (tuning().wave_gate_inward_cells as isize).max(1);
    let center = |c: isize, r: isize| Position::new((c as f32 + 0.5) * cell, (r as f32 + 0.5) * cell);
    for (col, row) in game.map.gate_cells() {
        let pos = map::cell_to_world(col, row);
        let gc = ((pos.x / cell) as isize).clamp(0, cols - 1);
        let gr = ((pos.y / cell) as isize).clamp(0, rows - 1);
        let inward = if gc == 0 {
            (1, 0)
        } else if gc == cols - 1 {
            (-1, 0)
        } else if gr == 0 {
            (0, 1)
        } else if gr == rows - 1 {
            (0, -1)
        } else {
            findings.push(LintFinding {
                severity: LintSeverity::Error,
                kind: LintKind::GateNotOnEdge,
                message: format!(
                    "gate at map cell ({col},{row}) = ({:.0},{:.0}) is nav-grid cell ({gc},{gr}), not on an edge of the {cols}x{rows} grid",
                    pos.x, pos.y
                ),
            });
            continue;
        };
        let blocked = (0..inward_cells)
            .map(|k| (gc + k * inward.0, gr + k * inward.1))
            .find(|&(c, r)| c < 0 || r < 0 || c >= cols || r >= rows || !grid.usable(center(c, r)));
        if let Some((c, r)) = blocked {
            findings.push(LintFinding {
                severity: LintSeverity::Error,
                kind: LintKind::GateBlocked,
                message: format!(
                    "gate at map cell ({col},{row}) = ({:.0},{:.0}) needs its {inward_cells}-cell lane from nav-grid cell ({gc},{gr}) inward all usable, but ({c},{r}) is not",
                    pos.x, pos.y
                ),
            });
        }
    }
}

/// A waves-plan map has to offer somewhere to roll in from. With explicit
/// `gate` cells that is `check_gates`' business; without any, the round
/// scans the intact nav grid's edges (`battlefield::gate_candidates`,
/// the same call, same `waves` knobs, avoiding the player start and the
/// player's frog by `wave_gate_min_player_dist`) and, finding no lane,
/// silently drops every wave into the spawn band instead - an error,
/// since the map then never plays as authored. Other plans are not
/// judged: a band map needs no gate.
fn check_wave_gates(
    game: &Game,
    grid: &Grid,
    width: f32,
    height: f32,
    player_pos: Position,
    findings: &mut Vec<LintFinding>,
) {
    if game.map.spawn.kind != SpawnKind::Waves || !game.map.gate_cells().is_empty() {
        return;
    }
    let (inward, min_dist) = {
        let t = tuning();
        (t.wave_gate_inward_cells, t.wave_gate_min_player_dist)
    };
    let mut avoid = vec![player_pos];
    avoid.extend(game.world.query::<&Frog>().iter().filter(|fr| fr.side == Side::Player).map(|fr| fr.position));
    if battlefield::gate_candidates(grid, width, height, &avoid, min_dist, inward).is_empty() {
        findings.push(LintFinding {
            severity: LintSeverity::Error,
            kind: LintKind::WavesNoGates,
            message: format!(
                "waves plan with no gate cells, and no edge lane ({inward} usable nav cells straight in, at least {min_dist:.0}px from the player start and frog) for the automatic scan to use: every wave would fall back to the spawn band"
            ),
        });
    }
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
/// approachable from the playfield - a pickup approachable only after
/// destructible walls are shot away is a `GatedPickup` warning rather
/// than an error (see `LintKind`). "Approachable" is a radius test
/// against playfield cell centers rather than "its own cell is
/// playfield", because both targets legitimately sit on blocked cells:
/// the frog *is* an obstacle in the nav grid (its own cell is always
/// blocked by its own margin), and a pickup tucked beside a wall sits
/// inside that wall's conservative margin while remaining perfectly
/// collectable. See `APPROACH_SLACK` for the radius rationale.
fn check_reachability(
    game: &Game,
    cells: &Cells,
    breach_cells: &Cells,
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
        if point_reachable(cells, pos, pickup_reach) {
            continue;
        }
        // Not approachable on intact terrain. Gated loot if shooting the
        // breakable walls away opens a way in; sealed for good otherwise.
        if point_reachable(breach_cells, pos, pickup_reach) {
            findings.push(LintFinding {
                severity: LintSeverity::Warning,
                kind: LintKind::GatedPickup,
                message: format!(
                    "pickup slot at map cell ({col},{row}) = ({:.0},{:.0}) is only reachable by destroying walls - the AI never will",
                    pos.x, pos.y
                ),
            });
        } else {
            findings.push(LintFinding {
                severity: LintSeverity::Error,
                kind: LintKind::UnreachablePickup,
                message: format!(
                    "pickup slot at map cell ({col},{row}) = ({:.0},{:.0}) has no playfield cell within {pickup_reach:.0}px even with every destructible wall gone",
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

/// §3.2.3: enough legal enemy-spawn cells in the border band. Counts
/// playfield cells passing `battlefield::enemy_spawn_legal` - the very
/// predicate `Game::init`'s enemy loop samples with (sample domain, band
/// depth, player clearance, nav-grid usability) - minus the enemy-vs-enemy
/// spacing term, which depends on where earlier enemies landed rather
/// than on the map. Cell count is a capacity
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
    grid: &Grid,
    obstacle_positions: &[Position],
    findings: &mut Vec<LintFinding>,
) {
    let short_side = width.min(height);
    let margin_min = short_side * tuning().enemy_spawn_margin_min;
    let margin_max = short_side * tuning().enemy_spawn_margin_max;
    let clear = player_size * 2.0;

    let mut capacity = 0usize;
    for row in 0..cells.rows {
        for col in 0..cells.cols {
            if !cells.is_open(col as isize, row as isize) || !cells.in_playfield(col, row) {
                continue;
            }
            let p = cells.center(col, row);
            if battlefield::enemy_spawn_legal(
                p,
                width,
                height,
                margin_min,
                margin_max,
                player_pos,
                clear,
                grid,
                obstacle_positions,
            ) {
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
        .unwrap_or(tuning().enemy_count_min);
    if capacity < required {
        findings.push(LintFinding {
            severity: LintSeverity::Error,
            kind: LintKind::SpawnBandTooTight,
            message: format!(
                "only {capacity} legal enemy-spawn cell(s) in the border band for {required} tank(s) - spawning will hit the rejection-sampling attempt cap"
            ),
        });
    } else if capacity < tuning().enemy_count_max {
        findings.push(LintFinding {
            severity: LintSeverity::Warning,
            kind: LintKind::SpawnBandTooTight,
            message: format!(
                "only {capacity} legal enemy-spawn cell(s) in the border band (fewer than enemy_count_max = {}) - high tank counts will crowd or degrade to the attempt cap",
                tuning().enemy_count_max
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
    const KNOWN_ERROR_KINDS: &[(&str, &[LintKind])] = &[("maps/default.toml", &[])];

    /// Headless seeded round on `map`, linted - the §3.1 setup. The fixed
    /// seed matters for maps that leave frog/start placement to `init`'s
    /// (seeded) fallback rolls: same seed, same layout, same findings.
    fn lint_map(map: MapFile) -> Vec<LintFinding> {
        lint(&init_game(map), W, H)
    }

    /// The seeded headless round `lint_map` lints, for tests that also
    /// want to ask the game itself what it makes of the map.
    fn init_game(map: MapFile) -> Game {
        let mut game = Game::default();
        game.seed_override = Some(0xB0B5);
        game.enemy_count_override = Some(4);
        game.map = map;
        game.init(W, H);
        game
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

    /// The same vault in Brick: the player can shoot a way in, so the
    /// slot is gated loot (a warning), not sealed (an error).
    #[test]
    fn brick_vaulted_pickup_is_gated_not_unreachable() {
        let mut map = base_map();
        sealed_vault(&mut map);
        let iron: Vec<(i32, i32)> = map
            .iter_cells()
            .filter(|(_, _, obj)| matches!(obj, CellObject::Wall { material: Material::Iron }))
            .map(|(c, r, _)| (c, r))
            .collect();
        for (col, row) in iron {
            map.set_cell(col, row, CellObject::Wall { material: Material::Brick });
        }
        map.set_cell(17, 11, CellObject::Pickup { pickup: PickupKind::Health });
        let findings = lint_map(map);
        dump("brick vault pickup", &findings);
        assert!(has(&findings, LintKind::GatedPickup));
        assert!(!has(&findings, LintKind::UnreachablePickup));
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

    // --- level cells: gates and the enemy frog ---

    fn gate(map: &mut MapFile, col: i32, row: i32) {
        map.set_cell(col, row, CellObject::Gate);
    }

    /// Map cells 0/39 (x = 0/1248) and rows 0/22 (y = 0/704) land on the
    /// nav grid's first/last column and row on the default battlefield.
    #[test]
    fn edge_gates_with_open_lanes_are_clean() {
        let mut map = base_map();
        gate(&mut map, 0, 11);
        gate(&mut map, 39, 11);
        gate(&mut map, 20, 0);
        gate(&mut map, 20, 22);
        let findings = lint_map(map);
        dump("edge gates", &findings);
        assert!(!has(&findings, LintKind::GateNotOnEdge));
        assert!(!has(&findings, LintKind::GateBlocked));
        assert!(errors(&findings).is_empty(), "four edge gates on an open map are fully legal");
    }

    #[test]
    fn interior_gate_is_not_on_an_edge() {
        let mut map = base_map();
        gate(&mut map, 20, 11);
        let findings = lint_map(map);
        dump("interior gate", &findings);
        assert!(
            findings.iter().any(|f| f.kind == LintKind::GateNotOnEdge && f.severity == LintSeverity::Error),
            "a gate in the middle of the field is an Error"
        );
        assert!(!has(&findings, LintKind::GateBlocked), "an interior gate has no lane to judge");
    }

    #[test]
    fn walled_lane_behind_an_edge_gate_is_blocked() {
        let mut map = base_map();
        gate(&mut map, 0, 11);
        // An iron slab across the lane one nav cell in from the left edge.
        for col in 2..=4 {
            for row in 9..=13 {
                wall(&mut map, col, row);
            }
        }
        let findings = lint_map(map);
        dump("blocked gate", &findings);
        assert!(
            findings.iter().any(|f| f.kind == LintKind::GateBlocked && f.severity == LintSeverity::Error),
            "a gate whose lane inward is walled is an Error"
        );
        assert!(!has(&findings, LintKind::GateNotOnEdge));
    }

    /// The linter's gate verdict and the round's own lane construction
    /// (`battlefield::gates_from_cells`, which applies `gate_candidates`'
    /// lane rule to explicit cells) agree cell for cell: every gate the
    /// linter passes is one the game rolls tanks through, and a gate it
    /// flags is one the game drops.
    #[test]
    fn lint_clean_gates_are_the_gates_the_round_uses() {
        let inward = tuning().wave_gate_inward_cells;

        let mut map = base_map();
        gate(&mut map, 0, 11);
        gate(&mut map, 39, 11);
        gate(&mut map, 20, 0);
        gate(&mut map, 20, 22);
        let cells = map.gate_cells();
        let game = init_game(map);
        let findings = lint(&game, W, H);
        assert!(!has(&findings, LintKind::GateBlocked));
        let used = battlefield::gates_from_cells(&game.nav_grid(W, H), W, H, &cells, inward);
        assert_eq!(used.len(), 4, "four lint-clean gates are four lanes the round uses");

        let mut map = base_map();
        gate(&mut map, 0, 11);
        for col in 2..=4 {
            for row in 9..=13 {
                wall(&mut map, col, row);
            }
        }
        let cells = map.gate_cells();
        let game = init_game(map);
        let findings = lint(&game, W, H);
        assert!(has(&findings, LintKind::GateBlocked));
        let used = battlefield::gates_from_cells(&game.nav_grid(W, H), W, H, &cells, inward);
        assert!(used.is_empty(), "a gate the linter flags as blocked is one the round drops");
    }

    /// An iron ring one map cell in from every edge blocks every edge
    /// lane the automatic gate scan could offer: under the waves plan,
    /// with no explicit gate, that is `WavesNoGates`; the same ring under
    /// the band plan needs no gate and is not judged.
    #[test]
    fn walled_in_waves_map_without_gates_is_an_error() {
        fn ringed(kind: SpawnKind) -> MapFile {
            let mut map = base_map();
            map.spawn.kind = kind;
            // Map cells run 0..=39 across and 0..=22 down on the default
            // battlefield; the ring sits on 1/38 and 1/21.
            for col in 1..=38 {
                wall(&mut map, col, 1);
                wall(&mut map, col, 21);
            }
            for row in 1..=21 {
                wall(&mut map, 1, row);
                wall(&mut map, 38, row);
            }
            map
        }
        let findings = lint_map(ringed(SpawnKind::Waves));
        dump("ringed, waves", &findings);
        assert!(
            findings.iter().any(|f| f.kind == LintKind::WavesNoGates && f.severity == LintSeverity::Error),
            "a waves map the scan finds no lane on is an Error"
        );

        let findings = lint_map(ringed(SpawnKind::Band));
        dump("ringed, band", &findings);
        assert!(!has(&findings, LintKind::WavesNoGates), "a band map is never judged on gates");
    }

    /// A waves map with no gate cells but open edges is fine: the
    /// automatic scan finds lanes, so the map plays as authored.
    #[test]
    fn open_waves_map_without_gates_is_clean() {
        let mut map = base_map();
        map.spawn.kind = SpawnKind::Waves;
        let findings = lint_map(map);
        dump("open, waves", &findings);
        assert!(!has(&findings, LintKind::WavesNoGates));
        assert!(errors(&findings).is_empty(), "an open waves map with no gate cells is fully legal");
    }

    #[test]
    fn hunt_map_without_an_enemy_frog_warns() {
        let mut map = base_map();
        map.mission.kind = Mission::Hunt;
        let findings = lint_map(map);
        dump("hunt, no enemy frog", &findings);
        assert!(
            findings.iter().any(|f| f.kind == LintKind::HuntMissingEnemyFrog && f.severity == LintSeverity::Warning),
            "a hunt map with no enemy_frog cell is a Warning"
        );

        let mut map = base_map();
        map.mission.kind = Mission::Hunt;
        map.set_cell(7, 4, CellObject::EnemyFrog);
        let findings = lint_map(map);
        dump("hunt, enemy frog placed", &findings);
        assert!(!has(&findings, LintKind::HuntMissingEnemyFrog));
        assert!(!has(&findings, LintKind::EnemyFrogUnreachable));
        assert!(errors(&findings).is_empty());
    }

    #[test]
    fn protect_map_without_an_enemy_frog_is_fine() {
        let findings = lint_map(base_map());
        assert!(!has(&findings, LintKind::HuntMissingEnemyFrog), "only Hunt needs an enemy frog");
    }

    /// The enemy frog in the same iron vault `sealed_frog_is_unreachable`
    /// buries the player's frog in - same reach rule, same verdict.
    #[test]
    fn sealed_enemy_frog_is_unreachable() {
        let mut map = MapFile::new();
        map.set_cell(27, 11, CellObject::Start);
        map.set_cell(30, 5, CellObject::Frog);
        map.mission.kind = Mission::Hunt;
        sealed_vault(&mut map);
        map.set_cell(16, 11, CellObject::EnemyFrog);
        let findings = lint_map(map);
        dump("sealed enemy frog", &findings);
        assert!(
            findings.iter().any(|f| f.kind == LintKind::EnemyFrogUnreachable && f.severity == LintSeverity::Error),
            "a vaulted enemy frog is an Error"
        );
        assert!(!has(&findings, LintKind::HuntMissingEnemyFrog));
    }

    /// The same carpeted band as `walled_band_has_no_spawn_capacity`, on a
    /// waves plan: nobody spawns in the band, so its capacity is not judged.
    #[test]
    fn waves_plan_skips_the_spawn_band_check() {
        let mut map = base_map();
        map.tanks = Some(10);
        map.spawn.kind = SpawnKind::Waves;
        for col in 5..=35 {
            wall(&mut map, col, 7);
            wall(&mut map, col, 15);
        }
        for row in 8..=14 {
            wall(&mut map, 5, row);
            wall(&mut map, 35, row);
        }
        let findings = lint_map(map);
        dump("walled band, waves", &findings);
        assert!(!has(&findings, LintKind::SpawnBandTooTight));
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
        // The 21-tile rails leave the band's outer cells legal under the
        // nav-grid spawn predicate, so the fixture is fully legal: its
        // provocation is the corridor drive itself, not spawn pressure.
        assert!(errors(&f).is_empty(), "tight-corridors is tight but fully legal");

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

        // The props playground is sparse on purpose: every prop is a
        // plain solid tile to the linter, and nothing is sealed off.
        let f = lint_path("maps/test/props.toml").expect("fixture loads");
        dump("props", &f);
        assert!(errors(&f).is_empty(), "props is an open playground");
    }

    /// maps/missions/ fixtures are clean starting points for one mission/
    /// spawn combination each, not provocations: no errors, and each one's
    /// level cells lint as intended (see their headers).
    #[test]
    fn mission_fixtures_lint_clean() {
        let f = lint_path("maps/missions/hunt-basic.toml").expect("fixture loads");
        dump("hunt-basic", &f);
        assert!(errors(&f).is_empty(), "hunt-basic must be fully legal");
        assert!(!has(&f, LintKind::HuntMissingEnemyFrog), "hunt-basic places its enemy frog");

        let f = lint_path("maps/missions/waves-basic.toml").expect("fixture loads");
        dump("waves-basic", &f);
        assert!(errors(&f).is_empty(), "waves-basic must be fully legal");
        assert!(!has(&f, LintKind::SpawnBandTooTight), "a waves plan is never judged on band capacity");
        assert!(!has(&f, LintKind::WavesNoGates), "waves-basic places its gates explicitly");
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
