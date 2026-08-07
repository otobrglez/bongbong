//! Small hand-authored obstacle-cluster shapes ("structures") for scattering
//! deliberate little builds - a wall run, a corner, an open bunker, a solid
//! block - across the battlefield instead of only ever placing lone tiles.
//!
//! Kept game-agnostic like `pathfind.rs`/`bt.rs`: this module only knows
//! about relative grid-cell offsets, nothing about `Obstacle`, materials,
//! physics, or hecs. `simulation::Game::init` rolls a blueprint, finds it a
//! valid spot, picks one material/variant for the whole thing (see
//! `docs/WALLS_SPEC.md`'s "keep one variant per structure" guidance), and
//! spawns each of its tiles as its own `Obstacle` entity.

/// A structure's shape: grid-cell offsets `(dx, dy)` from its anchor cell,
/// in units of one `OBSTACLE_GRID_SIZE` cell - not pixels. `(0, 0)` is
/// always included by convention (the anchor cell itself is part of the
/// shape).
pub type Blueprint = &'static [(i32, i32)];

/// A lone tile - the trivial 1-cell blueprint, rolled separately from
/// `STRUCTURES` (see `OBSTACLE_STRUCTURE_CHANCE`) so single obstacles stay
/// common rather than being just one more equally-likely shape.
pub const SINGLE: Blueprint = &[(0, 0)];

pub const WALL_LINE_3: Blueprint = &[(0, 0), (1, 0), (2, 0)];
pub const WALL_LINE_5: Blueprint = &[(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)];
/// A right-angle wall corner: three tiles running one direction, three
/// (sharing the corner tile) running perpendicular to it.
pub const L_CORNER: Blueprint = &[(0, 0), (1, 0), (2, 0), (0, 1), (0, 2)];
/// A solid 2x2 block - reads as a small bunker/pillbox rather than a wall.
pub const PILLAR_BLOCK: Blueprint = &[(0, 0), (1, 0), (0, 1), (1, 1)];
/// An open-fronted 3-wide bunker (a "U"): back wall plus two side stubs,
/// gap in the middle of the front to walk through/shoot out of.
pub const BUNKER: Blueprint = &[(0, 0), (1, 0), (2, 0), (0, 1), (2, 1)];

/// Every non-trivial shape - a placement rolls one of these, or falls back
/// to `SINGLE`, per `OBSTACLE_STRUCTURE_CHANCE`.
pub const STRUCTURES: &[Blueprint] = &[WALL_LINE_3, WALL_LINE_5, L_CORNER, PILLAR_BLOCK, BUNKER];
