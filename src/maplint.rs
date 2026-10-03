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
use crate::math::Rectangle;
use crate::obstacle::Obstacle;
use crate::pathfind::Grid;
use crate::simulation::Game;
use crate::tank::Tank;
use crate::tower::TowerKind;
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
/// 96px of slack on top of the worst-case tank half-extent (and, for the
/// frog, its own collider half-extent). Generous on purpose - a target
/// squeezed against a wall still has its nearest open cell within roughly
/// one cell of its footprint, so only genuine sealing (a closed ring, a
/// walled-off pocket) can fail it.
///
/// A fixed px distance rather than a multiple of `PATHFIND_CELL_SIZE`:
/// what counts as "approachable" is a property of the map, so it must not
/// shift because the router's cell size was retuned. It was
/// `2.0 * PATHFIND_CELL_SIZE` while that was 48px, hence 96.
const APPROACH_SLACK: f32 = 96.0;

/// Open components smaller than this many cells are reported as `Info`
/// (sometimes decorative slivers); at or above it they're a `Warning`
/// (likely an authoring mistake - real playable area cut off from the
/// playfield). Straight from the design doc's §3.2.1.
const DISCONNECTED_WARNING_CELLS: usize = 4;

/// Towers on one side past which `too-many-towers` notes it - advice, not
/// a cap (docs/defence-towers-prd.md section 17).
const TOWERS_PER_SIDE_INFO: usize = 6;

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
    /// No `start` cell: player 1 spawns at the nearest free cell to the
    /// centre, wherever that is on this map.
    NoStart,
    /// Player 1's start is not in the playfield. A warning when
    /// destructible terrain (fences, barrels, brick) is all that pens it
    /// in - the player shoots or rams a way out, a legitimate "break out
    /// of the garage" opening - and an error when permanent walls seal it
    /// for good.
    StartPenned,
    /// Rounds with more than one seat: a seat past player 1 (the `start2`
    /// cell, or a fallback beside player 1) is not in the playfield, so
    /// the seats start in separate regions.
    Player2Unreachable,
    /// Two-player rounds: the `start2` cell sits inside the clearance
    /// `Game::init` keeps between the seats, so the game ignores it and
    /// places player 2 at the nearest open cell instead.
    PlayersTooClose,
    /// Exactly one portal on the map: a network needs two, so this one is
    /// inert - it neither teleports nor draws in the round.
    PortalAlone,
    /// A portal no tank can use: its anchor has no open nav cell within
    /// the trigger radius (it sits in terrain), or none of those cells is
    /// in the playfield (nothing can reach it, nothing can arrive).
    PortalBlocked,
    /// An enemy tower whose reach covers a seat's start: the round opens
    /// under fire (docs/defence-towers-prd.md section 11). A tower on a
    /// gate's lane needs no lint of its own - it is solid, so the lane
    /// reads as `GateBlocked`.
    TowerAtStart,
    /// A tower whose reach covers no playfield cell: it can never fire.
    TowerNoReach,
    /// More than `TOWERS_PER_SIDE_INFO` towers on one side.
    TooManyTowers,
}

impl LintKind {
    /// Every kind, in check order.
    pub const ALL: [LintKind; 22] = [
        LintKind::UnreachableFrog,
        LintKind::UnreachablePickup,
        LintKind::GatedPickup,
        LintKind::DisconnectedRegion,
        LintKind::BoxedInCell,
        LintKind::SpawnBandTooTight,
        LintKind::PlannerPhysicsMismatch,
        LintKind::NarrowCorridor,
        LintKind::GateNotOnEdge,
        LintKind::GateBlocked,
        LintKind::WavesNoGates,
        LintKind::HuntMissingEnemyFrog,
        LintKind::EnemyFrogUnreachable,
        LintKind::NoStart,
        LintKind::StartPenned,
        LintKind::Player2Unreachable,
        LintKind::PlayersTooClose,
        LintKind::PortalAlone,
        LintKind::PortalBlocked,
        LintKind::TowerAtStart,
        LintKind::TowerNoReach,
        LintKind::TooManyTowers,
    ];

    /// The kebab-case name the lint's output and the dev server's `lint`
    /// tool use - and the builder's CHECK panel looks its words up by
    /// (`lint-<tag>` in the catalogue).
    pub(crate) fn tag(self) -> &'static str {
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
            LintKind::NoStart => "no-start",
            LintKind::StartPenned => "start-penned",
            LintKind::Player2Unreachable => "player2-unreachable",
            LintKind::PlayersTooClose => "players-too-close",
            LintKind::PortalAlone => "portal-alone",
            LintKind::PortalBlocked => "portal-blocked",
            LintKind::TowerAtStart => "tower-at-start",
            LintKind::TowerNoReach => "tower-no-reach",
            LintKind::TooManyTowers => "too-many-towers",
        }
    }
}

/// One lint result: severity + which check + a self-contained message
/// (nav-grid cell coordinates and world px where relevant), the cells it
/// is about and, where exactly one edit makes it go away, that edit.
#[derive(Clone, Debug, PartialEq)]
pub struct LintFinding {
    pub severity: LintSeverity,
    pub kind: LintKind,
    pub message: String,
    /// Where on the map the finding is - what the builder's CHECK panel
    /// frames and marks: the object a check names (a start, a portal, a
    /// pickup, a gate, a tower), or the nav cells of what it measured (a
    /// pocket, a corridor, a lane's blocked cell). Empty for a finding
    /// about the whole map (no gate anywhere, the spawn band).
    pub cells: Vec<LintCell>,
    /// The quick fix, where the check knows the one edit that answers it
    /// (a penned start moved to the nearest cell it can drive from, a
    /// lone portal taken away), worked out against the round the finding
    /// was made on. `None` where the answer is the author's to choose.
    pub fix: Option<LintFix>,
}

impl LintFinding {
    /// A finding about `cells`, with no fix.
    fn new(severity: LintSeverity, kind: LintKind, message: String, cells: Vec<LintCell>) -> LintFinding {
        LintFinding { severity, kind, message, cells, fix: None }
    }

    fn with_fix(mut self, fix: Option<LintFix>) -> LintFinding {
        self.fix = fix;
        self
    }
}

/// A place a finding is about, as the check knows it: a map cell, the
/// square round `map::cell_to_world`'s point where an object the author
/// placed stands, or a cell of the nav grid the reachability checks run
/// on - `PATHFIND_CELL_SIZE` squares from the field's corner, half a map
/// cell off the map's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LintCell {
    Map(i32, i32),
    Nav(usize, usize),
}

impl LintCell {
    /// The cell's square in world pixels.
    pub fn rect(self) -> Rectangle {
        match self {
            LintCell::Map(col, row) => {
                let p = map::cell_to_world(col, row);
                let half = crate::OBSTACLE_GRID_SIZE / 2.0;
                Rectangle::new(p.x - half, p.y - half, crate::OBSTACLE_GRID_SIZE, crate::OBSTACLE_GRID_SIZE)
            }
            LintCell::Nav(col, row) => {
                Rectangle::new(col as f32 * PATHFIND_CELL_SIZE, row as f32 * PATHFIND_CELL_SIZE, PATHFIND_CELL_SIZE, PATHFIND_CELL_SIZE)
            }
        }
    }
}

/// The world rectangle round every cell of `cells`, `None` for none.
pub fn cells_bounds(cells: &[LintCell]) -> Option<Rectangle> {
    let mut rects = cells.iter().map(|c| c.rect());
    let first = rects.next()?;
    Some(rects.fold(first, |a, b| {
        let (x, y) = (a.x.min(b.x), a.y.min(b.y));
        Rectangle::new(x, y, (a.x + a.width).max(b.x + b.width) - x, (a.y + a.height).max(b.y + b.height) - y)
    }))
}

/// A quick fix: the one edit that answers a finding, in map cells. Each
/// is one undo step in the builder (`MapEditor::apply_fix`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LintFix {
    /// Move the object on `from` to the empty cell `to`.
    Move { from: (i32, i32), to: (i32, i32) },
    /// Put `object` on the empty cell `at`.
    Place { object: CellObject, at: (i32, i32) },
    /// Take the object on `at` off the map.
    Remove { at: (i32, i32) },
}

impl LintFix {
    /// The cells the fix leaves something on - where the builder shows
    /// the map after it.
    pub fn target(self) -> (i32, i32) {
        match self {
            LintFix::Move { to, .. } => to,
            LintFix::Place { at, .. } | LintFix::Remove { at } => at,
        }
    }
}

/// What a map is linted under: the round PLAY would set up on it - the
/// seed, the seats and the command line's overrides of the session asking
/// (`LintSetup::of`) - since the checks read the terrain and the seats a
/// `Game::init` laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LintSetup {
    pub seed: u64,
    pub players: crate::simulation::PlayerCount,
    pub enemy_count_override: Option<usize>,
    pub player_row_override: Option<i32>,
    pub player2_row_override: Option<i32>,
    pub level_overrides: crate::level::LevelOverrides,
}

impl LintSetup {
    /// The setup of the round `game` is: its pinned seed or the one it
    /// drew, its seats and its overrides.
    pub fn of(game: &Game) -> LintSetup {
        LintSetup {
            seed: game.seed_override.unwrap_or_else(|| game.round_seed()),
            players: game.players,
            enemy_count_override: game.enemy_count_override,
            player_row_override: game.player_row_override,
            player2_row_override: game.player2_row_override,
            level_overrides: game.level_overrides,
        }
    }
}

/// Lint `map` on a fresh headless round set up by `setup` (§3.1's
/// setup): the one entry the dev server's `lint` tool and the builder's
/// CHECK panel share, so the two cannot disagree. The round comes back
/// with the findings, for what a caller reports of it.
pub fn lint_map(map: &crate::map::MapFile, setup: &LintSetup) -> (Game, Vec<LintFinding>) {
    let mut game = Game::default();
    game.map = map.clone();
    game.seed_override = Some(setup.seed);
    game.players = setup.players;
    game.enemy_count_override = setup.enemy_count_override;
    game.player_row_override = setup.player_row_override;
    game.player2_row_override = setup.player2_row_override;
    game.level_overrides = setup.level_overrides;
    let (width, height) = game.map.field_size();
    game.init(width, height);
    let findings = lint(&game, width, height);
    (game, findings)
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
    /// Per cell, the cells an active portal network joins it to: for a
    /// cell in one portal's footprint, every other portal's footprint
    /// (`Grid::portal_links`); empty everywhere else. The linter's own
    /// flood fills step along these exactly as the AI's planner routes
    /// through the hub, so reaching one active portal reaches them all
    /// and a room joined to the field only by a portal is playfield, not
    /// a disconnected region.
    links: Vec<Vec<(usize, usize)>>,
}

impl Cells {
    fn idx(&self, col: usize, row: usize) -> usize {
        row * self.cols + col
    }

    /// The open cells a flood fill steps to from `(col, row)`: the four
    /// cardinal neighbours, then the portal links.
    fn neighbours(&self, col: usize, row: usize) -> Vec<(usize, usize)> {
        let mut out: Vec<(usize, usize)> = [(0i32, -1i32), (0, 1), (-1, 0), (1, 0)]
            .iter()
            .map(|(dc, dr)| (col as i32 + dc, row as i32 + dr))
            .filter(|&(c, r)| self.is_open(c as isize, r as isize))
            .map(|(c, r)| (c as usize, r as usize))
            .collect();
        out.extend(self.links[self.idx(col, row)].iter().copied().filter(|&(c, r)| self.is_open(c as isize, r as isize)));
        out
    }

    /// The portal links of `grid`, laid out per cell for `neighbours`.
    fn links_from(grid: &Grid, cols: usize, rows: usize) -> Vec<Vec<(usize, usize)>> {
        let mut links = vec![Vec::new(); cols * rows];
        for &cell in grid.portal_cells() {
            if cell.0 < cols && cell.1 < rows {
                links[cell.1 * cols + cell.0] = grid.portal_links(cell).collect();
            }
        }
        links
    }

    /// Whether `cell` is in the playfield or shares an edge with a cell
    /// that is - a start or spawn inside an obstacle's conservative
    /// margin is still part of the field it sits on the edge of.
    fn touches_playfield(&self, cell: (usize, usize)) -> bool {
        let (col, row) = (cell.0 as i32, cell.1 as i32);
        [(0i32, 0i32), (0, -1), (0, 1), (-1, 0), (1, 0)].iter().any(|(dc, dr)| {
            let (c, r) = (col + dc, row + dr);
            c >= 0 && r >= 0 && (c as usize) < self.cols && (r as usize) < self.rows && self.in_playfield(c as usize, r as usize)
        })
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

    // The player's actual spawned position: `init` already resolved the
    // map's `Start` cell (or the center-fallback), so reading it back
    // means zero duplicated spawn logic.
    let player_entity = game.player().expect("lint runs on an initialized game");
    let (player_pos, player_size) = {
        let mut query = game.world.query::<(Entity, &Tank)>();
        query
            .iter()
            .find(|(entity, _)| *entity == player_entity)
            .map(|(_, tank)| (tank.position, tank.size()))
            .expect("player tank exists after init")
    };

    // Every seat past the first: the same read-back, so the lint sees the
    // fallback placements `init` really made.
    let others: Vec<Position> = game
        .players()
        .into_iter()
        .flatten()
        .skip(1)
        .map(|entity| {
            let mut query = game.world.query::<(Entity, &Tank)>();
            query
                .iter()
                .find(|(e, _)| *e == entity)
                .map(|(_, tank)| tank.position)
                .expect("a seat's tank exists after init")
        })
        .collect();
    let player_positions: Vec<Position> = std::iter::once(player_pos).chain(others.iter().copied()).collect();

    // The playfield is the largest open region of intact terrain - the
    // battlefield the AI roams, whichever cell the author put the start
    // on. The start joins it when its cell is in it or beside it (a start
    // inside an obstacle's conservative margin is legal - `next_step`
    // treats start/goal cells as open for exactly the same reason);
    // otherwise the start is penned off and `check_players` says so,
    // rather than the whole map reading as unreachable from a pen.
    let links = Cells::links_from(&grid, cols, rows);
    let mut cells = Cells { cols, rows, open, playfield: vec![false; cols * rows], links: links.clone() };
    let start_cell = cells.cell_of(player_pos);
    let seed = largest_component_seed(&cells).unwrap_or(start_cell);
    flood(&mut cells, seed);
    let start_in_playfield = cells.touches_playfield(start_cell);
    if start_in_playfield {
        flood(&mut cells, start_cell);
    }

    // The same playfield once every destructible wall is gone: only Iron
    // (never destroyed - see `obstacle::Material`) and the frog still
    // block. Built exactly like `Game::nav_grid`, minus the breakable
    // tiles, so "reachable by breaching" is the AI's own occupancy rule
    // applied to the terrain a player can shoot their way to.
    let breach_grid = Grid::build(
        width,
        height,
        PATHFIND_CELL_SIZE,
        battlefield::max_tank_clearance_half_extent(),
        game.world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| o.material.is_permanent())
            .map(|o| (o.position, o.hull_size() * 0.5))
            .chain(game.world.query::<&Frog>().iter().map(|fr| {
                (fr.position, FROG_COLLIDER_HALF_EXTENT.0.max(FROG_COLLIDER_HALF_EXTENT.1))
            }))
            // Deep water never goes away either (docs/water.md).
            .chain(game.water().deep_cells().map(|p| (p, crate::OBSTACLE_GRID_SIZE * 0.5))),
    )
    .with_portals(
        if game.portals_active() { game.portals() } else { &[] },
        tuning().portal_trigger_radius,
        tuning().portal_hop_cost,
    );
    let mut breach_open = vec![false; cols * rows];
    for row in 0..rows {
        for col in 0..cols {
            breach_open[row * cols + col] = probe_open(&breach_grid, cols, rows, col, row);
        }
    }
    let mut breach_cells = Cells { cols, rows, open: breach_open, playfield: vec![false; cols * rows], links };
    flood(&mut breach_cells, seed);
    let start_breachable = breach_cells.touches_playfield(start_cell);
    flood(&mut breach_cells, start_cell);
    let start = StartStatus { in_playfield: start_in_playfield, breachable: start_breachable };

    let obstacle_positions: Vec<Position> =
        game.world.query::<&Obstacle>().iter().map(|o| o.position).collect();
    let frog_pos = game.world.query::<&Frog>().iter().next().map(|f| f.position);

    let mut findings = Vec::new();
    let places = Places { game, grid: &grid, cells: &cells };
    check_players(&places, start, player_pos, player_size, &others, &mut findings);
    check_reachability(game, &cells, &breach_cells, frog_pos, &mut findings);
    check_enemy_frog(game, &cells, &mut findings);
    check_gates(game, &grid, width, height, &mut findings);
    check_wave_gates(game, &grid, width, height, &player_positions, &mut findings);
    check_portals(game, &cells, &mut findings);
    check_towers(game, &cells, &player_positions, &mut findings);
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
            &player_positions,
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

/// Where a quick fix can put an object down: the round's own answer to
/// "where can a tank start" (`Game::drop_cell` - spawn-legal clearance,
/// out of deep water, off a portal) narrowed to the playfield and to the
/// map's empty cells, since a fix moves or places an object and must not
/// paint over another.
struct Places<'a> {
    game: &'a Game,
    grid: &'a Grid,
    cells: &'a Cells,
}

impl Places<'_> {
    /// The empty playfield cell nearest `near` a tank can start on that
    /// also passes `also`.
    fn empty_cell(&self, near: Position, also: impl Fn(Position) -> bool) -> Option<(i32, i32)> {
        self.game.drop_cell(self.grid, near, |(col, row), p| {
            let (c, r) = self.cells.cell_of(p);
            self.cells.in_playfield(c, r) && self.game.map.cell(col, row).is_none() && also(p)
        })
    }
}

/// Where player 1's start sits relative to the playfield (see `lint`).
#[derive(Clone, Copy)]
struct StartStatus {
    in_playfield: bool,
    /// Whether shooting every destructible tile away connects it.
    breachable: bool,
}

/// The players' starts: a map with no `start` cell is a warning (the
/// game copes, but the author most likely meant to place one); a start
/// outside the playfield is `StartPenned`, a warning while the pen is
/// destructible and an error once it is permanent; every seat past the
/// first has to share the playfield with player 1, and the two authored
/// starts have to leave a tank's width between them.
///
/// The fixes put a start where a tank can start from in the playfield,
/// nearest where it was: the missing start where the game would have put
/// player 1 anyway, a penned start out of its pen, an authored `start2`
/// off its island or out of player 1's clearance.
fn check_players(
    places: &Places,
    start: StartStatus,
    player_pos: Position,
    player_size: f32,
    others: &[Position],
    findings: &mut Vec<LintFinding>,
) {
    let (game, cells) = (places.game, places.cells);
    let start_cell = LintCell::Map(map::world_to_cell(player_pos).0, map::world_to_cell(player_pos).1);
    let put_start = |to: (i32, i32)| match game.map.start_cell() {
        Some(from) => LintFix::Move { from, to },
        None => LintFix::Place { object: CellObject::Start, at: to },
    };
    if game.map.start_cell().is_none() {
        let message = format!(
            "no `start` cell: player 1 spawns at the nearest free cell to the centre, ({:.0},{:.0})",
            player_pos.x, player_pos.y
        );
        let fix = places.empty_cell(player_pos, |_| true).map(put_start);
        findings.push(LintFinding::new(LintSeverity::Warning, LintKind::NoStart, message, vec![start_cell]).with_fix(fix));
    }
    if !start.in_playfield {
        let (col, row) = cells.cell_of(player_pos);
        let (severity, how) = if start.breachable {
            (LintSeverity::Warning, "penned in by destructible terrain - the player has to shoot or ram a way out")
        } else {
            (LintSeverity::Error, "sealed off by permanent walls - the player can never leave")
        };
        let message = format!(
            "player 1 starts at ({:.0},{:.0}) - nav cell ({col},{row}) - outside the playfield, {how}",
            player_pos.x, player_pos.y
        );
        let fix = places.empty_cell(player_pos, |_| true).map(put_start);
        findings.push(LintFinding::new(severity, LintKind::StartPenned, message, vec![start_cell]).with_fix(fix));
    }
    let clear = player_size * 2.0;
    for (i, &pos) in others.iter().enumerate() {
        let (col, row) = cells.cell_of(pos);
        if !cells.touches_playfield((col, row)) {
            let message = format!(
                "player {} spawns at ({:.0},{:.0}) - nav cell ({col},{row}) - outside the playfield: the seats start in separate regions",
                i + 2,
                pos.x,
                pos.y
            );
            // Player 2 on the map's own `start2`: that cell moves, kept a
            // clear tank's width off player 1 so the game honours it.
            let authored = (i == 0)
                .then(|| game.map.start2_cell())
                .flatten()
                .filter(|&(c, r)| {
                    let (c, r) = game.map.nearest_free_cell(c, r);
                    map::cell_to_world(c, r) == pos
                });
            let fix = authored.and_then(|from| {
                places.empty_cell(pos, |p| p.distance_to(player_pos) >= clear).map(|to| LintFix::Move { from, to })
            });
            let at = map::world_to_cell(pos);
            findings.push(
                LintFinding::new(LintSeverity::Error, LintKind::Player2Unreachable, message, vec![LintCell::Map(at.0, at.1)]).with_fix(fix),
            );
        }
    }
    // The same clearance `Game::init` demands of a `start2` cell before
    // it honours it (`tank.size() * 2.0`), measured on the map's cells.
    if let (Some(s1), Some(s2)) = (game.map.start_cell(), game.map.start2_cell()) {
        let first = map::cell_to_world(s1.0, s1.1);
        let gap = map::cell_to_world(s2.0, s2.1).distance_to(first);
        if gap < clear {
            let message = format!(
                "the start2 cell is {gap:.0}px from the start cell, inside the {clear:.0}px clearance the game keeps between the players: it is ignored and player 2 is placed at the nearest open cell beside player 1 instead"
            );
            let fix = places
                .empty_cell(map::cell_to_world(s2.0, s2.1), |p| p.distance_to(first) >= clear)
                .map(|to| LintFix::Move { from: s2, to });
            let cells = vec![LintCell::Map(s1.0, s1.1), LintCell::Map(s2.0, s2.1)];
            findings.push(LintFinding::new(LintSeverity::Warning, LintKind::PlayersTooClose, message, cells).with_fix(fix));
        }
    }
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
            findings.push(LintFinding::new(
                LintSeverity::Warning,
                LintKind::HuntMissingEnemyFrog,
                "hunt mission with no enemy_frog cell - the round falls back to a procedural spot in the enemy spawn band".to_string(),
                Vec::new(),
            ));
        }
        return;
    };
    let pos = map::cell_to_world(col, row);
    let reach = FROG_COLLIDER_HALF_EXTENT.0.max(FROG_COLLIDER_HALF_EXTENT.1)
        + battlefield::max_tank_clearance_half_extent()
        + APPROACH_SLACK;
    if !point_reachable(cells, pos, reach) {
        let message = format!(
            "enemy_frog at map cell ({col},{row}) = ({:.0},{:.0}) has no playfield cell within {reach:.0}px - hunters can never reach it",
            pos.x, pos.y
        );
        findings.push(LintFinding::new(LintSeverity::Error, LintKind::EnemyFrogUnreachable, message, vec![LintCell::Map(col, row)]));
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
fn check_gates(game: &Game, grid: &Grid, width: f32, height: f32, findings: &mut Vec<LintFinding>) {
    let (cols, rows, cell) = grid.dims();
    let (ucols, urows) = (cols, rows);
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
            let message = format!(
                "gate at map cell ({col},{row}) = ({:.0},{:.0}) is nav-grid cell ({gc},{gr}), not on an edge of the {cols}x{rows} grid",
                pos.x, pos.y
            );
            // The round drops a gate with no outside to roll in from, so
            // taking it off the map changes nothing but the warning.
            let fix = Some(LintFix::Remove { at: (col, row) });
            findings.push(LintFinding::new(LintSeverity::Error, LintKind::GateNotOnEdge, message, vec![LintCell::Map(col, row)]).with_fix(fix));
            continue;
        };
        // Start past the cells the boundary clearance margin blocks, the
        // same offset `battlefield::gate_candidates` applies when it builds
        // the round's real lanes - a lane cell no tank can stand in is not
        // part of the lane, and walking from the literal edge cell would
        // report every gate on every map as blocked.
        let (along, span, near) = if inward.0 != 0 {
            (ucols, width, inward.0 > 0)
        } else {
            (urows, height, inward.1 > 0)
        };
        let start = battlefield::boundary_lane_inset(cell, along, span, near) as isize;
        let blocked = (start..start + inward_cells)
            .map(|k| (gc + k * inward.0, gr + k * inward.1))
            .find(|&(c, r)| c < 0 || r < 0 || c >= cols || r >= rows || !grid.usable(center(c, r)));
        if let Some((c, r)) = blocked {
            let message = format!(
                "gate at map cell ({col},{row}) = ({:.0},{:.0}) needs its {inward_cells}-cell lane from nav-grid cell ({gc},{gr}) inward all usable, but ({c},{r}) is not",
                pos.x, pos.y
            );
            let mut at = vec![LintCell::Map(col, row)];
            if c >= 0 && r >= 0 && c < cols && r < rows {
                at.push(LintCell::Nav(c as usize, r as usize));
            }
            findings.push(LintFinding::new(LintSeverity::Error, LintKind::GateBlocked, message, at));
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
    player_positions: &[Position],
    findings: &mut Vec<LintFinding>,
) {
    if game.map.spawn.kind != SpawnKind::Waves || !game.map.gate_cells().is_empty() {
        return;
    }
    let (inward, min_dist) = {
        let t = tuning();
        (t.wave_gate_inward_cells, t.wave_gate_min_player_dist)
    };
    let mut avoid = player_positions.to_vec();
    avoid.extend(game.world.query::<&Frog>().iter().filter(|fr| fr.side == Side::Player).map(|fr| fr.position));
    if battlefield::gate_candidates(grid, width, height, &avoid, min_dist, inward).is_empty() {
        let message = format!(
            "waves plan with no gate cells, and no edge lane ({inward} usable nav cells straight in, at least {min_dist:.0}px from the player start and frog) for the automatic scan to use: every wave would fall back to the spawn band"
        );
        findings.push(LintFinding::new(LintSeverity::Error, LintKind::WavesNoGates, message, Vec::new()));
    }
}

/// BFS the open region reachable from `seed`, marking `playfield`. The
/// seed cell is always included (see the start-cell note in `lint`).
/// A cell of the largest open component (ties: the first found scanning
/// row-major, so the result is deterministic), or `None` on a map with
/// no open cell at all.
fn largest_component_seed(cells: &Cells) -> Option<(usize, usize)> {
    let mut seen = vec![false; cells.cols * cells.rows];
    let mut best: Option<((usize, usize), usize)> = None;
    for row in 0..cells.rows {
        for col in 0..cells.cols {
            if seen[cells.idx(col, row)] || !cells.is_open(col as isize, row as isize) {
                continue;
            }
            let mut size = 0;
            let mut queue = VecDeque::from([(col, row)]);
            seen[cells.idx(col, row)] = true;
            while let Some((c, r)) = queue.pop_front() {
                size += 1;
                for (nc, nr) in cells.neighbours(c, r) {
                    if !seen[cells.idx(nc, nr)] {
                        seen[cells.idx(nc, nr)] = true;
                        queue.push_back((nc, nr));
                    }
                }
            }
            if best.is_none_or(|(_, b)| size > b) {
                best = Some(((col, row), size));
            }
        }
    }
    best.map(|(seed, _)| seed)
}

fn flood(cells: &mut Cells, seed: (usize, usize)) {
    let mut queue = VecDeque::new();
    cells.playfield[seed.1 * cells.cols + seed.0] = true;
    queue.push_back(seed);
    while let Some((col, row)) = queue.pop_front() {
        for (nc, nr) in cells.neighbours(col, row) {
            let idx = cells.idx(nc, nr);
            if !cells.playfield[idx] {
                cells.playfield[idx] = true;
                queue.push_back((nc, nr));
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
    let tank_radius = battlefield::max_tank_clearance_half_extent();
    if let Some(frog) = frog_pos {
        let frog_reach =
            FROG_COLLIDER_HALF_EXTENT.0.max(FROG_COLLIDER_HALF_EXTENT.1) + tank_radius + APPROACH_SLACK;
        if !point_reachable(cells, frog, frog_reach) {
            let message =
                format!("frog at ({:.0},{:.0}) has no playfield cell within {frog_reach:.0}px - tanks can never reach it", frog.x, frog.y);
            let (col, row) = map::world_to_cell(frog);
            findings.push(LintFinding::new(LintSeverity::Error, LintKind::UnreachableFrog, message, vec![LintCell::Map(col, row)]));
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
            let message = format!(
                "pickup slot at map cell ({col},{row}) = ({:.0},{:.0}) is only reachable by destroying walls - the AI never will",
                pos.x, pos.y
            );
            findings.push(LintFinding::new(LintSeverity::Warning, LintKind::GatedPickup, message, vec![LintCell::Map(col, row)]));
        } else {
            let message = format!(
                "pickup slot at map cell ({col},{row}) = ({:.0},{:.0}) has no playfield cell within {pickup_reach:.0}px even with every destructible wall gone",
                pos.x, pos.y
            );
            findings.push(LintFinding::new(LintSeverity::Error, LintKind::UnreachablePickup, message, vec![LintCell::Map(col, row)]));
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
/// Portals (docs/teleporting.md): one alone is inert (`PortalAlone`); one
/// whose anchor has no open nav cell within `portal_trigger_radius`, or
/// - in an active network - none in the playfield, can never be entered
/// or arrived at (`PortalBlocked`). Judged on the same footprint rule
/// `Grid::with_portals` routes by.
/// The towers (docs/defence-towers-prd.md section 11): an enemy tower
/// whose reach covers a seat's start, a tower that can reach no playfield
/// cell (for the bio slush, none beyond its minimum range), and a side
/// with more than `TOWERS_PER_SIDE_INFO`.
fn check_towers(game: &Game, cells: &Cells, player_positions: &[Position], findings: &mut Vec<LintFinding>) {
    let towers = game.tower_views();
    for view in &towers {
        let (col, row) = map::world_to_cell(view.position);
        let range = view.kind.range();
        let min_range = if view.kind == TowerKind::Bio { tuning().bio_min_range } else { 0.0 };
        if view.side == Side::Enemy {
            for (seat, &at) in player_positions.iter().enumerate() {
                let d = at.distance_to(view.position);
                if d <= range && d >= min_range {
                    let message = format!(
                        "enemy {} at map cell ({col},{row}) reaches player {}'s start {d:.0}px away (reach {range:.0}px): the round opens under fire",
                        view.kind.name(),
                        seat + 1
                    );
                    let start = map::world_to_cell(at);
                    let cells = vec![LintCell::Map(col, row), LintCell::Map(start.0, start.1)];
                    findings.push(LintFinding::new(LintSeverity::Warning, LintKind::TowerAtStart, message, cells));
                }
            }
        }
        let reaches = (0..cells.rows).flat_map(|r| (0..cells.cols).map(move |c| (c, r))).any(|(c, r)| {
            let d = cells.center(c, r).distance_to(view.position);
            cells.in_playfield(c, r) && d <= range && d >= min_range
        });
        if !reaches {
            let message = format!("{} at map cell ({col},{row}) reaches no playfield cell: it can never fire", view.kind.name());
            findings.push(LintFinding::new(LintSeverity::Info, LintKind::TowerNoReach, message, vec![LintCell::Map(col, row)]));
        }
    }
    for side in [Side::Player, Side::Enemy] {
        let mine: Vec<LintCell> = towers
            .iter()
            .filter(|v| v.side == side)
            .map(|v| {
                let (col, row) = map::world_to_cell(v.position);
                LintCell::Map(col, row)
            })
            .collect();
        let n = mine.len();
        if n > TOWERS_PER_SIDE_INFO {
            let message = format!("{n} {} towers (more than {TOWERS_PER_SIDE_INFO}): heavy on the tick and on the player", side.name());
            findings.push(LintFinding::new(LintSeverity::Info, LintKind::TooManyTowers, message, mine));
        }
    }
}

fn check_portals(game: &Game, cells: &Cells, findings: &mut Vec<LintFinding>) {
    let anchors = game.map.portal_cells();
    if anchors.len() == 1 {
        let (col, row) = anchors[0];
        let message = format!("one portal at map cell ({col},{row}): a network needs two - this one is inert and the round draws nothing");
        // Inert and undrawn in the round: taking it away changes nothing
        // but the warning.
        let fix = Some(LintFix::Remove { at: (col, row) });
        findings.push(LintFinding::new(LintSeverity::Warning, LintKind::PortalAlone, message, vec![LintCell::Map(col, row)]).with_fix(fix));
    }
    let radius = tuning().portal_trigger_radius;
    for (col, row) in anchors {
        let at = map::cell_to_world(col, row);
        let footprint: Vec<(usize, usize)> = (0..cells.rows)
            .flat_map(|r| (0..cells.cols).map(move |c| (c, r)))
            .filter(|&(c, r)| cells.is_open(c as isize, r as isize) && cells.center(c, r).distance_to(at) <= radius)
            .collect();
        if footprint.is_empty() {
            let message = format!(
                "portal at map cell ({col},{row}) = ({:.0},{:.0}) has no open nav cell within {radius:.0}px: it sits in terrain",
                at.x, at.y
            );
            findings.push(LintFinding::new(LintSeverity::Error, LintKind::PortalBlocked, message, vec![LintCell::Map(col, row)]));
        } else if game.portals_active() && !footprint.iter().any(|&c| cells.touches_playfield(c)) {
            let message = format!(
                "portal at map cell ({col},{row}) = ({:.0},{:.0}) is not in the playfield: nothing can reach it or arrive through it",
                at.x, at.y
            );
            findings.push(LintFinding::new(LintSeverity::Error, LintKind::PortalBlocked, message, vec![LintCell::Map(col, row)]));
        }
    }
}

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
                for (nc, nr) in cells.neighbours(c, r) {
                    if !seen[cells.idx(nc, nr)] {
                        seen[cells.idx(nc, nr)] = true;
                        queue.push_back((nc, nr));
                    }
                }
            }
            let severity = if component.len() >= DISCONNECTED_WARNING_CELLS {
                LintSeverity::Warning
            } else {
                LintSeverity::Info
            };
            let (c0, r0) = component[0];
            let message = format!(
                "{} open cell(s) unreachable from the playfield, e.g. grid cell ({c0},{r0}) around ({:.0},{:.0})",
                component.len(),
                cells.center(c0, r0).x,
                cells.center(c0, r0).y
            );
            let at = component.iter().map(|&(c, r)| LintCell::Nav(c, r)).collect();
            findings.push(LintFinding::new(severity, LintKind::DisconnectedRegion, message, at));
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
                let message = format!(
                    "grid cell ({col},{row}) around ({:.0},{:.0}) is open but every neighbor is blocked - unusable, and a trap for knocked-back tanks",
                    center.x, center.y
                );
                findings.push(LintFinding::new(LintSeverity::Error, LintKind::BoxedInCell, message, vec![LintCell::Nav(col, row)]));
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
    player_positions: &[Position],
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
            // Legal against every player: `Game::init` keeps enemies
            // `clear` from each of them.
            let legal = player_positions.iter().all(|&player_pos| {
                battlefield::enemy_spawn_legal(
                    p,
                    width,
                    height,
                    margin_min,
                    margin_max,
                    player_pos,
                    clear,
                    grid,
                    obstacle_positions,
                )
            });
            if legal {
                capacity += 1;
            }
        }
    }

    // Same 1..=31 clamp `Game::init` applies to a map's `tanks` count (31
    // = the cap on live enemies, `wave_max_alive`).
    let required = game
        .map
        .tanks
        .map(|n| (n as usize).clamp(1, 31))
        .unwrap_or(tuning().enemy_count_min);
    if capacity < required {
        let message = format!(
            "only {capacity} legal enemy-spawn cell(s) in the border band for {required} tank(s) - spawning will hit the rejection-sampling attempt cap"
        );
        findings.push(LintFinding::new(LintSeverity::Error, LintKind::SpawnBandTooTight, message, Vec::new()));
    } else if capacity < tuning().enemy_count_max {
        let message = format!(
            "only {capacity} legal enemy-spawn cell(s) in the border band (fewer than enemy_count_max = {}) - high tank counts will crowd or degrade to the attempt cap",
            tuning().enemy_count_max
        );
        findings.push(LintFinding::new(LintSeverity::Warning, LintKind::SpawnBandTooTight, message, Vec::new()));
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
    let tank_half = battlefield::max_tank_clearance_half_extent();
    // Each box filed under every nav cell whose centre it could overlap a
    // tank at, so a cell is tested against its few neighbours rather than
    // every box on the map - the same boxes in the same order, so the
    // same first overlap is reported.
    let mut near: Vec<Vec<usize>> = vec![Vec::new(); cells.cols * cells.rows];
    for (i, &(pos, half)) in boxes.iter().enumerate() {
        let reach_x = tank_half + half.x;
        let reach_y = tank_half + half.y;
        let span = |c: f32, reach: f32, n: usize| {
            let lo = ((c - reach) / PATHFIND_CELL_SIZE - 0.5).floor().max(0.0) as usize;
            let hi = (((c + reach) / PATHFIND_CELL_SIZE - 0.5).ceil().max(0.0) as usize).min(n.saturating_sub(1));
            lo..=hi
        };
        for row in span(pos.y, reach_y, cells.rows) {
            for col in span(pos.x, reach_x, cells.cols) {
                near[cells.idx(col, row)].push(i);
            }
        }
    }
    for row in 0..cells.rows {
        for col in 0..cells.cols {
            if !cells.is_open(col as isize, row as isize) {
                continue;
            }
            let center = cells.center(col, row);
            for &(pos, half) in near[cells.idx(col, row)].iter().map(|&i| &boxes[i]) {
                if aabb_overlap(center, tank_half, pos, half) {
                    let message = format!(
                        "grid cell ({col},{row}) around ({:.0},{:.0}) is open but a worst-case tank there overlaps the collider at ({:.0},{:.0})",
                        center.x, center.y, pos.x, pos.y
                    );
                    let (c, r) = map::world_to_cell(pos);
                    let at = vec![LintCell::Nav(col, row), LintCell::Map(c, r)];
                    findings.push(LintFinding::new(LintSeverity::Error, LintKind::PlannerPhysicsMismatch, message, at));
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
                let message = format!("grid cell ({col},{row}) around ({:.0},{:.0}) is a single-cell-wide passage", center.x, center.y);
                findings.push(LintFinding::new(LintSeverity::Info, LintKind::NarrowCorridor, message, vec![LintCell::Nav(col, row)]));
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
    
    // The synthetic maps below are drawn on the 40 x 22.5 field the
    // checks were written against; `wide()` pins that size on them so the
    // standard field's own size (`DEFAULT_SCREEN_*`) is not what they get.
    const W: f32 = 1280.0;
    const H: f32 = 720.0;

    fn wide() -> MapFile {
        let mut map = MapFile::new();
        map.size = Some((40.0, 22.5));
        map
    }

    /// Maps the game actually ships/loads by default - gated by
    /// `supported_maps_no_new_errors` against `KNOWN_ERROR_BUDGET` below.
    /// Grow this list as maps graduate from scratch to shipped.
    const SUPPORTED_MAPS: &[&str] = &[
        "maps/default.toml",
        "maps/default-desert.toml",
        "maps/towers.toml",
        "maps/longwater.toml",
        "maps/lotus-lagoon.toml",
        "maps/vulkan.toml",
        "maps/hedge-maze.toml",
        "maps/oasis-bazaar.toml",
        "maps/castle-moat.toml",
        "maps/archipelago.toml",
        "maps/black-gold.toml",
        "maps/harbor-lights.toml",
        "maps/carnival.toml",
        "maps/jungle-temple.toml",
        "maps/serpent-river.toml",
        "maps/no-mans-land.toml",
        "maps/glasshouses.toml",
        "maps/scrapyard.toml",
        "maps/grand-campaign.toml",
    ];

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
    const KNOWN_ERROR_KINDS: &[(&str, &[LintKind])] =
        &[("maps/default.toml", &[]), ("maps/default-desert.toml", &[]), ("maps/towers.toml", &[])];

    /// Headless seeded round on `map`, linted - the §3.1 setup. The fixed
    /// seed matters for maps that leave frog/start placement to `init`'s
    /// (seeded) fallback rolls: same seed, same layout, same findings.
    /// Two passes: the single-player round for every check, then a
    /// two-player round for the player-2 checks only (the terrain is the
    /// same, so its other findings would only be duplicates).
    fn lint_map(map: MapFile) -> Vec<LintFinding> {
        let (w, h) = map.field_size();
        let mut findings = lint(&init_game(map.clone()), w, h);
        findings.extend(
            lint(&init_game_seats(map, 2), w, h)
                .into_iter()
                .filter(|f| matches!(f.kind, LintKind::Player2Unreachable | LintKind::PlayersTooClose)),
        );
        findings
    }

    /// The seeded headless round `lint_map` lints, for tests that also
    /// want to ask the game itself what it makes of the map.
    fn init_game(map: MapFile) -> Game {
        let mut game = Game::default();
        game.seed_override = Some(0xB0B5);
        game.enemy_count_override = Some(4);
        let (w, h) = map.field_size();
        game.map = map;
        game.init(w, h);
        game
    }

    /// The same round with `seats` seats, for the checks that only fire
    /// once a map has to seat more than player 1.
    fn init_game_seats(map: MapFile, seats: usize) -> Game {
        let mut game = Game::default();
        game.seed_override = Some(0xB0B5);
        game.enemy_count_override = Some(4);
        game.players = crate::simulation::PlayerCount::from_count(seats).expect("a legal seat count");
        let (w, h) = map.field_size();
        game.map = map;
        game.init(w, h);
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
    /// `Grid::build`'s center-inside-reach blocking rule. A wall tile's
    /// centre sits on a nav-cell boundary (both grids have the same 32px
    /// pitch, anchored half a cell apart), so each wall row/column blocks
    /// the two nav cells either side of it: walls three map cells apart
    /// leave exactly one open nav cell between them. Hence 14/17 and 8/11.
    /// A wider ring yields a 2x2 open interior instead, where no cell is
    /// boxed - each has an open neighbor - and only `DisconnectedRegion`
    /// fires.
    fn sealed_ring(map: &mut MapFile) {
        for row in 8..=11 {
            wall(map, 14, row);
            wall(map, 17, row);
        }
        for col in 14..=17 {
            wall(map, col, 8);
            wall(map, col, 11);
        }
    }

    /// A much wider closed ring, for burying a frog/pickup deep enough that
    /// no playfield cell center lies within their approach reach
    /// (`APPROACH_SLACK` plus the tank half-extent and, for the frog, its
    /// own collider - ~143px). Nothing in this interior is boxed-in, which
    /// is `sealed_ring`'s job instead.
    ///
    /// Sized from the far side of the wall, not the near one: the ring
    /// blocks the two nav cells either side of each wall line, so with
    /// walls on cols 12/22 the nearest *playfield* cell centre is col 10's
    /// x=336, and a centred occupant at col 17 (x=544) sits 208px from it.
    /// The 14..20 ring this used to be left that gap at ~112px, well inside
    /// the reach, so the burial stopped registering as unreachable at all.
    fn sealed_vault(map: &mut MapFile) {
        for row in 6..=16 {
            wall(map, 12, row);
            wall(map, 22, row);
        }
        for col in 12..=22 {
            wall(map, col, 6);
            wall(map, col, 16);
        }
    }

    fn base_map() -> MapFile {
        let mut map = wide();
        map.set_cell(27, 11, CellObject::Start);
        map.set_cell(7, 12, CellObject::Frog);
        map
    }

    fn has(findings: &[LintFinding], kind: LintKind) -> bool {
        findings.iter().any(|f| f.kind == kind)
    }

    /// Water is ground: a river across the whole field neither splits the
    /// playfield nor boxes anything in, and enemies may spawn on it.
    #[test]
    fn water_is_open_ground_to_the_planner_and_the_spawner() {
        let mut map = base_map();
        for row in 0..23 {
            map.set_cell(17, row, CellObject::Water);
        }
        for col in 14..=20 {
            for row in 9..=13 {
                map.set_cell(col, row, CellObject::Water);
            }
        }
        let findings = lint_map(map.clone());
        dump("river and lake", &findings);
        assert!(errors(&findings).is_empty(), "a river and a lake lint clean");
        let game = init_game(map);
        let (w, h) = game.map.field_size();
        let grid = game.nav_grid(w, h);
        assert!(!grid.usable(map::cell_to_world(17, 11)), "the lake's deep middle is a wall to the planner");
        assert!(grid.usable(map::cell_to_world(17, 4)), "the river is a ford, open to the planner and the spawner");
        assert!(!grid.boxed_in(map::cell_to_world(17, 4)));
    }

    /// The two-player checks: a `start2` sealed off from `start` is an
    /// error, one painted right beside it a warning, a map with no start
    /// at all a warning whose fallback pair still shares a playfield.
    #[test]
    fn player_two_start_is_linted_against_player_one() {
        let mut map = base_map();
        map.set_cell(3, 3, CellObject::Start2);
        for (c, r) in [(2, 2), (3, 2), (4, 2), (2, 3), (4, 3), (2, 4), (3, 4), (4, 4)] {
            wall(&mut map, c, r);
        }
        let f = lint_map(map);
        dump("sealed start2", &f);
        assert!(has(&f, LintKind::Player2Unreachable));
        assert!(!has(&f, LintKind::PlayersTooClose));
        assert!(!has(&f, LintKind::NoStart));

        let mut map = base_map();
        map.set_cell(28, 11, CellObject::Start2);
        let f = lint_map(map);
        dump("adjacent start2", &f);
        assert!(has(&f, LintKind::PlayersTooClose));
        assert!(!has(&f, LintKind::Player2Unreachable));
        assert!(errors(&f).is_empty(), "too close is advisory");

        let mut map = wide();
        map.set_cell(7, 12, CellObject::Frog);
        let f = lint_map(map);
        dump("no start", &f);
        assert!(has(&f, LintKind::NoStart));
        assert!(!has(&f, LintKind::Player2Unreachable), "the fallback beside player 1 shares its playfield");
        assert!(!has(&f, LintKind::PlayersTooClose));
        assert!(errors(&f).is_empty());
    }

    /// The seat check generalises: every shipped map seats all eight
    /// without pushing one outside the playfield (the fallback walks
    /// outward from player 1 until it finds a cell a tank can stand in,
    /// so only an authored start can land off the playfield), and an
    /// authored `start2` sealed off still reports - naming the seat by
    /// number, with the seats after it placed by the fallback and clean.
    #[test]
    fn the_seat_checks_generalise_past_player_two() {
        for path in SUPPORTED_MAPS {
            let map = MapFile::load(std::path::Path::new(path)).expect("supported map must load");
            let (w, h) = map.field_size();
            for seats in 1..=crate::MAX_SEATS {
                let f = lint(&init_game_seats(map.clone(), seats), w, h);
                let penned: Vec<&str> =
                    f.iter().filter(|f| f.kind == LintKind::Player2Unreachable).map(|f| f.message.as_str()).collect();
                assert!(penned.is_empty(), "{path} with {seats} seats: {penned:?}");
            }
        }

        let mut map = base_map();
        map.set_cell(3, 3, CellObject::Start2);
        for (c, r) in [(2, 2), (3, 2), (4, 2), (2, 3), (4, 3), (2, 4), (3, 4), (4, 4)] {
            wall(&mut map, c, r);
        }
        let (w, h) = map.field_size();
        let f = lint(&init_game_seats(map, 4), w, h);
        dump("four seats, sealed start2", &f);
        let penned: Vec<&str> =
            f.iter().filter(|f| f.kind == LintKind::Player2Unreachable).map(|f| f.message.as_str()).collect();
        assert_eq!(penned.len(), 1, "only the authored start is off the playfield: {penned:?}");
        assert!(penned[0].starts_with("player 2 "), "the seat is named by number: {penned:?}");
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

    /// One portal is inert and says so; a portal walled in on every side
    /// has no footprint and is an error; two portals across a full iron
    /// wall join the rooms for the linter exactly as they do for the
    /// planner - the far room is playfield, not a disconnected region.
    #[test]
    fn portals_lint_as_a_network() {
        let mut lone = base_map();
        lone.set_cell(20, 5, CellObject::Portal);
        let f = lint_map(lone);
        dump("lone portal", &f);
        assert!(has(&f, LintKind::PortalAlone));
        assert!(!has(&f, LintKind::PortalBlocked));
        assert!(errors(&f).is_empty(), "a lone portal is a warning, not an error");

        let mut walled = base_map();
        walled.set_cell(20, 5, CellObject::Portal);
        walled.set_cell(30, 5, CellObject::Portal);
        for (c, r) in [(19, 4), (20, 4), (21, 4), (19, 5), (21, 5), (19, 6), (20, 6), (21, 6)] {
            wall(&mut walled, c, r);
        }
        let f = lint_map(walled);
        dump("walled portal", &f);
        assert!(has(&f, LintKind::PortalBlocked), "an anchor ringed by iron has no open footprint");
        assert!(!has(&f, LintKind::PortalAlone));

        let mut rooms = base_map();
        for row in 0..=22 {
            wall(&mut rooms, 20, row);
        }
        rooms.set_cell(15, 11, CellObject::Portal);
        rooms.set_cell(25, 11, CellObject::Portal);
        let split = {
            let mut m = rooms.clone();
            m.clear_cell(15, 11);
            m.clear_cell(25, 11);
            m
        };
        let f_split = lint_map(split);
        dump("split rooms", &f_split);
        assert!(has(&f_split, LintKind::DisconnectedRegion), "without portals the far room is cut off");
        let f = lint_map(rooms);
        dump("portal rooms", &f);
        assert!(!has(&f, LintKind::DisconnectedRegion), "the portals join the rooms");
        assert!(!has(&f, LintKind::PortalBlocked) && !has(&f, LintKind::PortalAlone));
        assert!(!has(&f, LintKind::UnreachableFrog), "the frog in the other room is reachable through the network");
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

    /// A start walled off from the field does not make the field
    /// unreachable: the playfield is the largest open region, and the
    /// start itself is what gets reported - a warning behind fences the
    /// player can ram or shoot through, an error behind iron.
    #[test]
    fn penned_start_is_reported_not_the_whole_map() {
        let mut map = base_map();
        // A fence pen around the start at (27,11), one cell of slack.
        for c in 25..=29 {
            map.set_cell(c, 9, CellObject::Fence);
            map.set_cell(c, 13, CellObject::Fence);
        }
        for r in 9..=13 {
            map.set_cell(25, r, CellObject::Fence);
            map.set_cell(29, r, CellObject::Fence);
        }
        let findings = lint_map(map.clone());
        dump("fenced start", &findings);
        assert!(!has(&findings, LintKind::UnreachableFrog), "the frog is on the open field");
        let penned: Vec<_> = findings.iter().filter(|f| f.kind == LintKind::StartPenned).collect();
        assert_eq!(penned.len(), 1);
        assert_eq!(penned[0].severity, LintSeverity::Warning);

        // The same pen in iron seals the player in for good.
        for c in 25..=29 {
            wall(&mut map, c, 9);
            wall(&mut map, c, 13);
        }
        for r in 9..=13 {
            wall(&mut map, 25, r);
            wall(&mut map, 29, r);
        }
        let findings = lint_map(map);
        dump("iron-sealed start", &findings);
        assert!(!has(&findings, LintKind::UnreachableFrog));
        let penned: Vec<_> = findings.iter().filter(|f| f.kind == LintKind::StartPenned).collect();
        assert_eq!(penned.len(), 1);
        assert_eq!(penned[0].severity, LintSeverity::Error);
    }

    #[test]
    fn sealed_frog_is_unreachable() {
        let mut map = wide();
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
        let mut map = wide();
        map.set_cell(28, 11, CellObject::Start);
        map.set_cell(30, 5, CellObject::Frog);
        // Every row, edge to edge. It used to stop at 1..=21 and lean on
        // the clearance margin to seal the last cell at each end, which is
        // not what this test is about and made it sensitive to the margin's
        // exact value.
        for row in 0..=22 {
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
        let mut map = wide();
        map.set_cell(20, 9, CellObject::Start); // inside the lane
        map.set_cell(20, 19, CellObject::Frog);
        // Three map rows apart, which is what leaves exactly one open nav
        // row between them - see `sealed_ring` for the arithmetic.
        for col in 10..=30 {
            wall(&mut map, col, 8);
            wall(&mut map, col, 11);
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
            let cells = Cells { cols, rows, playfield: open.clone(), links: vec![Vec::new(); open.len()], open };
            let mut findings = Vec::new();
            check_planner_physics(&cells, &boxes, &mut findings);
            findings
        };

        assert!(
            !lint_against(0.0).is_empty(),
            "with no margin, cells beside the obstacle are open yet a tank there overlaps it"
        );
        assert!(
            lint_against(battlefield::max_tank_clearance_half_extent()).is_empty(),
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
        let mut map = wide();
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

    // --- corridor width vs. nav-grid routability ---

    /// A full-height iron divider down `col` with one horizontal slot
    /// `free` map cells tall whose topmost open row is `slot_top` - the
    /// `maps/test/choke.toml` shape reduced to two variables. `base_map`'s
    /// start (27,11) sits right of the divider and its frog (7,12) left, so
    /// neither ever plugs the slot.
    fn divider_map(col: i32, slot_top: i32, free: i32) -> MapFile {
        let mut map = base_map();
        for row in 0..=22 {
            if row < slot_top || row >= slot_top + free {
                wall(&mut map, col, row);
            }
        }
        map
    }

    /// Whether the AI's own nav grid can route across `divider_map`'s
    /// divider - asked through `Game::nav_path_cells`, i.e. the exact grid
    /// `Game::nav_grid` hands `Ai::steer` every frame, so this cannot
    /// disagree with what the AI sees.
    fn crosses(col: i32, slot_top: i32, free: i32) -> bool {
        let map = divider_map(col, slot_top, free);
        let (w, h) = map.field_size();
        let game = init_game(map);
        let mid = (slot_top as f32 + free as f32 / 2.0) * crate::OBSTACLE_GRID_SIZE;
        let left = Position::new((col - 6) as f32 * crate::OBSTACLE_GRID_SIZE, mid);
        let right = Position::new((col + 6) as f32 * crate::OBSTACLE_GRID_SIZE, mid);
        game.nav_path_cells(left, right, w, h).is_some()
    }

    /// The physically drivable width, for the two tests below to measure
    /// the grid against: the widest movement collider any chassis presents
    /// at any cardinal facing (`2 * max_tank_clearance_half_extent`), which
    /// is what actually has to fit between two wall faces.
    fn widest_hull_px() -> f32 {
        2.0 * battlefield::max_tank_clearance_half_extent()
    }

    /// A slot's width is the only thing that may decide whether the AI can
    /// use it. It used not to be: nav cells were 48px over a 32px map grid,
    /// so rasterization had a phase that repeated every `lcm(32,48)/32` = 3
    /// map rows, and the same 3-cell slot routed at two rows out of three
    /// and was sealed at the third - identical geometry, different answer,
    /// depending only on where the author happened to put it.
    #[test]
    fn corridor_routability_depends_on_width_alone_not_on_which_row_it_sits() {
        for free in 1..=5 {
            let answers: Vec<bool> = (0..3).map(|phase| crosses(20, 8 + phase, free)).collect();
            println!(
                "corridor free={free} cells ({}px) -> routable by row-phase {:?}",
                free * crate::OBSTACLE_GRID_SIZE as i32,
                answers
            );
            assert!(
                answers.iter().all(|&a| a == answers[0]),
                "a {free}-cell slot routes at some rows and not others ({answers:?}) - \
                 nav-grid rasterization must not have a phase relative to the map grid",
            );
        }
    }

    /// Where the width threshold actually sits, and that it is justified by
    /// geometry rather than by rasterization luck. A two-cell slot is 64px
    /// of free floor and the widest hull in the roster is 50.4px, so the AI
    /// must be willing to drive it; a one-cell slot is 32px and only the
    /// two smallest chassis would fit, so sealing it is correct for a grid
    /// that carries one shared margin.
    ///
    /// Recovering the one-cell case means per-tank clearance (a scout
    /// routing where a leviathan cannot), which this grid deliberately does
    /// not do - see `max_tank_clearance_half_extent`.
    #[test]
    fn a_two_cell_corridor_is_drivable_and_the_grid_agrees() {
        assert!(
            widest_hull_px() < 2.0 * crate::OBSTACLE_GRID_SIZE,
            "a two-cell slot must physically fit every chassis for this test to mean anything",
        );
        assert!(
            widest_hull_px() > crate::OBSTACLE_GRID_SIZE,
            "a one-cell slot must NOT fit every chassis, or sealing it would be wrong",
        );
        for phase in 0..3 {
            assert!(crosses(20, 8 + phase, 2), "a 64px slot fits every hull and must route");
            assert!(!crosses(20, 8 + phase, 1), "a 32px slot fits almost nothing and must not route");
        }
    }

    // --- on-disk maps ---

    fn lint_path(path: &str) -> Result<Vec<LintFinding>, String> {
        MapFile::load(std::path::Path::new(path)).map(lint_map)
    }

    /// The `maps/test/corridors/` fixtures exist to be looked at - in the
    /// game with `nav_grid`, or under the probe - so this only pins that
    /// each one still says what its header says, i.e. that the divider is
    /// crossable at exactly the widths the headers claim. They live in a
    /// subdirectory because `just probe-fixtures` globs `maps/test/*.toml`
    /// and these are shaped to be pathological, not to hold a budget.
    #[test]
    fn corridor_fixtures_cross_at_the_widths_their_headers_claim() {
        for (name, crossable) in [
            ("slot-1", false),
            ("slot-2", true),
            ("slot-3-row8", true),
            ("slot-3-row9", true),
        ] {
            let path = format!("maps/test/corridors/{name}.toml");
            let map = MapFile::load(std::path::Path::new(&path)).expect("fixture loads");
            let (w, h) = map.field_size();
            let game = init_game(map);
            let left = Position::new(14.0 * crate::OBSTACLE_GRID_SIZE, 11.0 * crate::OBSTACLE_GRID_SIZE);
            let right = Position::new(26.0 * crate::OBSTACLE_GRID_SIZE, 11.0 * crate::OBSTACLE_GRID_SIZE);
            assert_eq!(
                game.nav_path_cells(left, right, w, h).is_some(),
                crossable,
                "{name} stopped matching its own header",
            );
        }
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

        // Two rooms joined by nothing but a portal each: legal only
        // because the linter walks the network like the planner does.
        let f = lint_path("maps/test/portals.toml").expect("fixture loads");
        dump("portals", &f);
        assert!(errors(&f).is_empty(), "the portal rooms are one playfield");
        assert!(!has(&f, LintKind::PortalAlone) && !has(&f, LintKind::PortalBlocked));
        assert!(!has(&f, LintKind::DisconnectedRegion), "the far room is joined through the portals");
        assert!(!has(&f, LintKind::UnreachableFrog));

        // Every tower kind on both sides, each able to reach the field, and
        // nothing firing on the start.
        let f = lint_path("maps/test/towers.toml").expect("fixture loads");
        dump("towers", &f);
        assert!(errors(&f).is_empty(), "the towers fixture is fully legal");
        assert!(!has(&f, LintKind::TowerNoReach) && !has(&f, LintKind::TowerAtStart));
    }

    /// The towers: an enemy tesla over the start is `tower-at-start`, a
    /// tower sealed in the vault reaches nothing, a seventh tower on one
    /// side is noted - and a map with none of that says nothing.
    #[test]
    fn tower_lints_fire_on_what_they_describe() {
        use crate::frog::Side;
        use crate::tower::TowerKind;
        let tower = |map: &mut MapFile, col: i32, row: i32, side: Side| {
            map.set_cell(col, row, CellObject::for_tower(TowerKind::Tesla, side));
        };
        let mut map = base_map();
        tower(&mut map, 27, 9, Side::Enemy);
        let f = lint_map(map);
        dump("tower-at-start", &f);
        assert!(has(&f, LintKind::TowerAtStart), "an enemy tesla two cells from the start");

        let mut map = base_map();
        sealed_vault(&mut map);
        tower(&mut map, 17, 11, Side::Player);
        let f = lint_map(map);
        assert!(has(&f, LintKind::TowerNoReach), "a tesla in the vault reaches nothing");
        assert!(!has(&f, LintKind::TooManyTowers));

        let mut map = base_map();
        for col in 0..7 {
            tower(&mut map, 30 + col, 3, Side::Player);
        }
        let f = lint_map(map);
        assert!(has(&f, LintKind::TooManyTowers), "seven on one side");
        assert!(!has(&f, LintKind::TowerAtStart), "the player's own towers are no threat to its start");
        assert!(!has(&f, LintKind::TowerNoReach));
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

        let f = lint_path("maps/portals.toml").expect("shipped map loads");
        dump("portals (shipped)", &f);
        assert!(errors(&f).is_empty(), "the shipped portal map must be fully legal");
        assert!(!has(&f, LintKind::PortalAlone) && !has(&f, LintKind::PortalBlocked));
    }

    /// `fix` applied to `map` the way the builder applies it.
    fn apply(map: &mut MapFile, fix: LintFix) {
        match fix {
            LintFix::Move { from, to } => {
                let obj = *map.cell(from.0, from.1).expect("a fix moves an object that is there");
                assert!(map.cell(to.0, to.1).is_none(), "a fix moves onto an empty cell");
                map.clear_cell(from.0, from.1);
                map.set_cell(to.0, to.1, obj);
            }
            LintFix::Place { object, at } => {
                assert!(map.cell(at.0, at.1).is_none(), "a fix places onto an empty cell");
                map.set_cell(at.0, at.1, object);
            }
            LintFix::Remove { at } => {
                assert!(map.cell(at.0, at.1).is_some(), "a fix removes an object that is there");
                map.clear_cell(at.0, at.1);
            }
        }
    }

    fn finding(findings: &[LintFinding], kind: LintKind) -> &LintFinding {
        findings.iter().find(|f| f.kind == kind).unwrap_or_else(|| panic!("no {kind:?} in {findings:?}"))
    }

    /// Every finding names its place: the object a check is about as its
    /// map cell, what it measured as nav cells - a pocket's every cell, a
    /// boxed-in cell, a lane's blocked one - and nothing for a finding
    /// about the whole map.
    #[test]
    fn findings_carry_the_cells_they_are_about() {
        let mut map = base_map();
        map.set_cell(20, 5, CellObject::Portal);
        sealed_ring(&mut map);
        map.set_cell(20, 11, CellObject::Gate);
        map.mission.kind = Mission::Hunt;
        let f = lint_map(map);
        dump("cells", &f);
        assert_eq!(finding(&f, LintKind::PortalAlone).cells, vec![LintCell::Map(20, 5)]);
        assert_eq!(finding(&f, LintKind::GateNotOnEdge).cells, vec![LintCell::Map(20, 11)]);
        let boxed = finding(&f, LintKind::BoxedInCell);
        assert!(matches!(boxed.cells[..], [LintCell::Nav(..)]), "{boxed:?}");
        // The boxed-in cell is the one the ring closes round: inside it.
        let inside = boxed.cells[0].rect();
        let ring = Rectangle::new(14.0 * 32.0, 8.0 * 32.0, 3.0 * 32.0, 3.0 * 32.0);
        assert!(inside.x >= ring.x && inside.y >= ring.y && inside.x + inside.width <= ring.x + ring.width + 1.0, "{inside:?} in {ring:?}");
        let region = finding(&f, LintKind::DisconnectedRegion);
        assert!(!region.cells.is_empty() && region.cells.iter().all(|c| matches!(c, LintCell::Nav(..))), "{region:?}");
        assert!(finding(&f, LintKind::HuntMissingEnemyFrog).cells.is_empty(), "about the whole map");
        // The bounds of a finding's cells hold every one of them.
        let b = cells_bounds(&region.cells).unwrap();
        for c in &region.cells {
            let r = c.rect();
            assert!(r.x >= b.x && r.y >= b.y && r.x + r.width <= b.x + b.width && r.y + r.height <= b.y + b.height);
        }
        assert_eq!(cells_bounds(&[]), None);
    }

    /// The quick fixes: each is the one edit its finding asks for, onto an
    /// empty cell where it puts something down, and the map it leaves
    /// lints clean of that finding.
    #[test]
    fn each_quick_fix_answers_its_finding() {
        // A lone portal and an interior gate: taken away.
        let mut map = base_map();
        map.set_cell(20, 5, CellObject::Portal);
        map.set_cell(20, 11, CellObject::Gate);
        let f = lint_map(map.clone());
        assert_eq!(finding(&f, LintKind::PortalAlone).fix, Some(LintFix::Remove { at: (20, 5) }));
        assert_eq!(finding(&f, LintKind::GateNotOnEdge).fix, Some(LintFix::Remove { at: (20, 11) }));
        for kind in [LintKind::PortalAlone, LintKind::GateNotOnEdge] {
            apply(&mut map, finding(&f, kind).fix.unwrap());
        }
        let after = lint_map(map);
        assert!(!has(&after, LintKind::PortalAlone) && !has(&after, LintKind::GateNotOnEdge), "{after:?}");

        // A start penned in iron: moved out to the nearest cell a tank can
        // start from on the playfield, which is the start's own side.
        let mut map = base_map();
        for c in 25..=29 {
            wall(&mut map, c, 9);
            wall(&mut map, c, 13);
        }
        for r in 9..=13 {
            wall(&mut map, 25, r);
            wall(&mut map, 29, r);
        }
        let f = lint_map(map.clone());
        let penned = finding(&f, LintKind::StartPenned);
        assert_eq!(penned.cells, vec![LintCell::Map(27, 11)]);
        let Some(LintFix::Move { from, to }) = penned.fix else { panic!("{penned:?}") };
        assert_eq!(from, (27, 11));
        let d = map::cell_to_world(to.0, to.1).distance_to(map::cell_to_world(27, 11));
        assert!(d < 5.0 * 32.0, "nearby: {to:?}");
        apply(&mut map, penned.fix.unwrap());
        let after = lint_map(map.clone());
        dump("start moved out", &after);
        assert!(!has(&after, LintKind::StartPenned), "{after:?}");

        // No start at all: put where the game would have put player 1.
        let mut map = wide();
        map.set_cell(7, 12, CellObject::Frog);
        let f = lint_map(map.clone());
        let none = finding(&f, LintKind::NoStart);
        let Some(LintFix::Place { object: CellObject::Start, at }) = none.fix else { panic!("{none:?}") };
        assert_eq!(none.cells, vec![LintCell::Map(at.0, at.1)], "where the game puts player 1 is a cell a start can be");
        apply(&mut map, none.fix.unwrap());
        assert!(!has(&lint_map(map), LintKind::NoStart));

        // Player 2's start inside player 1's clearance: moved out of it.
        let mut map = base_map();
        map.set_cell(28, 11, CellObject::Start2);
        let f = lint_map(map.clone());
        let close = finding(&f, LintKind::PlayersTooClose);
        assert_eq!(close.cells, vec![LintCell::Map(27, 11), LintCell::Map(28, 11)]);
        apply(&mut map, close.fix.expect("a fix"));
        assert!(!has(&lint_map(map), LintKind::PlayersTooClose));

        // Player 2's start sealed off: moved onto the playfield, a clear
        // tank's width off player 1.
        let mut map = base_map();
        map.set_cell(3, 3, CellObject::Start2);
        for (c, r) in [(2, 2), (3, 2), (4, 2), (2, 3), (4, 3), (2, 4), (3, 4), (4, 4)] {
            wall(&mut map, c, r);
        }
        let f = lint_map(map.clone());
        let sealed = finding(&f, LintKind::Player2Unreachable);
        assert!(matches!(sealed.fix, Some(LintFix::Move { from: (3, 3), .. })), "{sealed:?}");
        apply(&mut map, sealed.fix.unwrap());
        let after = lint_map(map);
        assert!(!has(&after, LintKind::Player2Unreachable) && !has(&after, LintKind::PlayersTooClose), "{after:?}");

        // Where the answer is the author's - a sealed pickup, a blocked
        // gate's lane - there is no fix.
        let mut map = base_map();
        sealed_vault(&mut map);
        map.set_cell(17, 11, CellObject::Pickup { pickup: PickupKind::Health });
        assert_eq!(finding(&lint_map(map), LintKind::UnreachablePickup).fix, None);
    }

    /// What linting a large map costs, the way the builder's CHECK panel
    /// runs it (a headless round set up and linted): prints, run with
    /// `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn lint_timing_on_the_large_maps() {
        for path in ["maps/study/frontier.toml", "maps/longwater.toml", "maps/default.toml"] {
            let map = MapFile::load(std::path::Path::new(path)).expect("the map loads");
            let setup = LintSetup { seed: 0xB0B5, ..LintSetup::default() };
            let _ = super::lint_map(&map, &setup);
            let runs = 5;
            let start = std::time::Instant::now();
            let mut n = 0;
            for _ in 0..runs {
                n = super::lint_map(&map, &setup).1.len();
            }
            let each = start.elapsed().as_secs_f64() * 1e3 / runs as f64;
            let mut game = Game::default();
            game.seed_override = Some(0xB0B5);
            game.map = map.clone();
            let (w, h) = map.field_size();
            let t = std::time::Instant::now();
            game.init(w, h);
            let init = t.elapsed().as_secs_f64() * 1e3;
            let t = std::time::Instant::now();
            let _ = lint(&game, w, h);
            let checks = t.elapsed().as_secs_f64() * 1e3;
            eprintln!("{path}: {each:.1} ms a run ({init:.1} ms the round, {checks:.1} ms the checks), {n} findings");
        }
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
