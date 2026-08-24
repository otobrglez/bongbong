//! Procedurally-placed ground/terrain layer: a grass base with road painted
//! at specific cells - under every static obstacle/wall tile, and inside the
//! player fortress's `B`/`O` glyphs - drawn from the third-party Puny World
//! tileset (`static/punyworld/punyworld-overworld-tileset.png` - see
//! `static/punyworld/SOURCE.md` for provenance and `docs/GROUND_SPEC.md` for
//! the full design writeup, including why this sheet is deliberately not on
//! the Resurrect 64 palette everything else uses).
//!
//! Purely decorative: no physics body, no gameplay effect. `build` runs
//! once per round (from `simulation::Game::init`, after every obstacle for
//! the round has been placed - see that function's own comments for why)
//! and resolves the whole layout into a flat `Vec<i32>` of source tile ids,
//! one per `GROUND_WORLD_TILE` grid cell - so `draw` is just an index
//! lookup and a blit per cell, no per-frame autotile work.
//!
//! The autotile tables below (`GRASS_FILL`, `ROAD_EDGE`) are extracted from
//! the source pack's own Tiled wangset data
//! (`static/punyworld/punyworld-overworld-tiles.tsx`), not invented here -
//! see docs/GROUND_SPEC.md for exactly how each entry was derived and for
//! the wangset's own documentation of the tile grid.
//!
//! Grid phase: a cell `(gx, gy)` is drawn *centered* on world position
//! `(gx * GROUND_WORLD_TILE, gy * GROUND_WORLD_TILE)`, not top-left-aligned
//! there - matching how `battlefield.rs`/`obstacle.rs` place and draw
//! obstacle tiles (`Obstacle::position` is a tile *center*, and every
//! obstacle position is an exact multiple of `OBSTACLE_GRID_SIZE`, which
//! equals `GROUND_WORLD_TILE`). A top-left-aligned ground grid would put
//! every obstacle tile's footprint straddling two ground cells (off by half
//! a tile in both x and y), so a road cell painted "under" an obstacle would
//! visibly miss it by half a tile in both directions - centering is what
//! actually makes `road_cells` in `build` line up pixel-for-pixel with the
//! object it's meant to sit under.

use sola_raylib::prelude::*;

use crate::{GROUND_WORLD_TILE, Position};

/// Columns in punyworld-overworld-tileset.png (432px / 16px).
const TILESET_COLS: i32 = 27;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Material {
    Grass,
    Road,
}

/// Plain grass fill tiles - all 9 are the "every corner grass" case (wangid
/// corners all 1) in the source wangset, i.e. cosmetic variants of the same
/// flat grass with no autotile meaning, picked at random per cell purely to
/// avoid a visibly repeating texture.
const GRASS_FILL: &[i32] = &[0, 1, 2, 27, 28, 29, 54, 55, 56];

/// Road (the source pack's "dirt-paths") Wang **edge** autotile - 15 of the
/// 16 possible N/E/S/W neighbour combinations have their own tile; the
/// missing one (no road neighbours at all) can come up here (an isolated
/// road cell, e.g. a lone obstacle tile with no orthogonal road neighbour)
/// unlike the old edge-to-edge road walk this replaced, so index 0 is a
/// real, reachable case now, not just a defensive fallback - the crossroads
/// tile (32) reads fine standing alone too. Indexed by a 4-bit mask,
/// bit3=N bit2=E bit1=S bit0=W (1=road neighbour).
const ROAD_EDGE: [i32; 16] = [
    32, // 0000 isolated
    87, // 0001 W
    3,  // 0010 S
    6,  // 0011 S+W
    85, // 0100 E
    86, // 0101 E+W
    4,  // 0110 E+S
    5,  // 0111 E+S+W
    57, // 1000 N
    60, // 1001 N+W
    30, // 1010 N+S
    33, // 1011 N+S+W
    58, // 1100 N+E
    59, // 1101 N+E+W
    31, // 1110 N+E+S
    32, // 1111 N+E+S+W (crossroads)
];

/// This round's resolved ground layer: a flat, row-major grid of source
/// tile ids (into `punyworld-overworld-tileset.png`), one per
/// `GROUND_WORLD_TILE` cell. Built once by `build`, drawn every frame by
/// `draw` - no autotile recomputation happens outside `build`.
#[derive(Default)]
pub struct GroundGrid {
    pub cols: usize,
    pub rows: usize,
    tiles: Vec<i32>,
}

impl GroundGrid {
    fn idx(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x as usize >= self.cols || y as usize >= self.rows {
            None
        } else {
            Some(y as usize * self.cols + x as usize)
        }
    }
}

/// Deterministic per-cell grass-fill variant pick, keyed by `seed` (one
/// value for the whole `build` call - see its own doc comment) and the
/// cell's own grid coordinates, rather than a shared sequential RNG stream.
/// This matters because a plain `rng.random_range(..)` draw per cell (the
/// original implementation) makes every *later* cell's pick depend on how
/// many earlier cells happened to be Grass vs Road - so re-running `build`
/// after just one cell's material changes (Grass<->Road) would shift every
/// subsequent Grass cell's draw and reshuffle its tile, even though nothing
/// about that cell itself changed. Hashing `(seed, x, y)` directly instead
/// makes each cell's pick depend only on itself: calling `build` again with
/// the same `seed`/`road_cells` reproduces byte-identical output, and
/// changing one cell never touches any other cell's tile. A live round only
/// ever calls `build` once (so the old stream-based version never visibly
/// misbehaved there), but the map editor (`editor.rs`) rebuilds this layer
/// after every single edit - see its own `rebuild_ground` doc comment.
/// (SplitMix64's mixing step - fast, decent avalanche, no external crate.)
fn grass_variant(seed: u64, x: i32, y: i32) -> i32 {
    let mut h = seed ^ 0x9E37_79B9_7F4A_7C15;
    h ^= (x as i64 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= (y as i64 as u64).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    GRASS_FILL[(h as usize) % GRASS_FILL.len()]
}

/// Roll this round's ground layout: grass everywhere, then road painted at
/// exactly `road_cells` (world positions - typically every static obstacle
/// tile's own position plus the player fortress's `B`/`O` interior cells,
/// see `Game::init`) and nowhere else. `width`/`height` are the same
/// playable-area extents `Game::init` already threads through everything
/// else (`battlefield::spawn_walls`, enemy/obstacle placement). `seed`
/// drives every grass cell's cosmetic tile pick (`grass_variant`) - pass a
/// freshly rolled value (e.g. `rng.random()`) for a normal round so it
/// varies, or a value held fixed across repeated calls (the editor) so
/// unrelated cells' grass doesn't visibly change on every edit. A
/// `road_cells` entry that lands outside the grid (shouldn't happen given
/// every real caller's positions are already clamped inside the
/// battlefield, but not asserted here) is silently ignored rather than
/// panicking.
pub fn build(width: f32, height: f32, seed: u64, road_cells: &[Position]) -> GroundGrid {
    // +1 over the plain `ceil(width / T)` cell count: since cells are
    // centered rather than top-left-aligned (see module doc comment), the
    // last cell's own right/bottom half-tile can fall short of `width`/
    // `height` otherwise, leaving an uncovered strip at the edge.
    let cols = (width / GROUND_WORLD_TILE).ceil().max(1.0) as usize + 1;
    let rows = (height / GROUND_WORLD_TILE).ceil().max(1.0) as usize + 1;
    let mut material = vec![Material::Grass; cols * rows];
    let at = |m: &[Material], x: i32, y: i32| -> Material {
        if x < 0 || y < 0 || x as usize >= cols || y as usize >= rows {
            Material::Grass // treat off-grid as grass, same as "not part of the road"
        } else {
            m[y as usize * cols + x as usize]
        }
    };

    for pos in road_cells {
        let gx = (pos.x / GROUND_WORLD_TILE).round() as i32;
        let gy = (pos.y / GROUND_WORLD_TILE).round() as i32;
        if gx >= 0 && gy >= 0 && (gx as usize) < cols && (gy as usize) < rows {
            material[gy as usize * cols + gx as usize] = Material::Road;
        }
    }

    // --- resolve: pick the exact source tile for every cell ---
    let mut tiles = vec![0i32; cols * rows];
    for y in 0..rows as i32 {
        for x in 0..cols as i32 {
            let tile = match at(&material, x, y) {
                Material::Grass => grass_variant(seed, x, y),
                Material::Road => {
                    let is_road = |dx: i32, dy: i32| at(&material, x + dx, y + dy) == Material::Road;
                    let n = is_road(0, -1);
                    let e = is_road(1, 0);
                    let s = is_road(0, 1);
                    let w = is_road(-1, 0);
                    let mask = ((n as usize) << 3) | ((e as usize) << 2) | ((s as usize) << 1) | (w as usize);
                    ROAD_EDGE[mask]
                }
            };
            tiles[y as usize * cols + x as usize] = tile;
        }
    }

    GroundGrid { cols, rows, tiles }
}

fn source_rec(tile_id: i32) -> Rectangle {
    let col = tile_id % TILESET_COLS;
    let row = tile_id / TILESET_COLS;
    Rectangle::new(
        col as f32 * crate::GROUND_TEXTURE_SIZE,
        row as f32 * crate::GROUND_TEXTURE_SIZE,
        crate::GROUND_TEXTURE_SIZE,
        crate::GROUND_TEXTURE_SIZE,
    )
}

/// Blit the whole resolved ground layer, one draw call per cell, each
/// centered on `(x * GROUND_WORLD_TILE, y * GROUND_WORLD_TILE)` - see the
/// module doc comment for why centered rather than top-left-aligned. No
/// rotation, no shadow (ground is the floor everything else sits on) -
/// drawn first, before tread marks/obstacles/tanks.
pub fn draw(d: &mut impl RaylibDraw, texture: &Texture2D, grid: &GroundGrid) {
    let size = GROUND_WORLD_TILE;
    let origin = Vector2::new(size / 2.0, size / 2.0);
    for y in 0..grid.rows {
        for x in 0..grid.cols {
            let Some(i) = grid.idx(x as i32, y as i32) else {
                continue;
            };
            let src = source_rec(grid.tiles[i]);
            let dest = Rectangle::new(x as f32 * GROUND_WORLD_TILE, y as f32 * GROUND_WORLD_TILE, size, size);
            d.draw_texture_pro(texture, src, dest, origin, 0.0, Color::WHITE);
        }
    }
}
