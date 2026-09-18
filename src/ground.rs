//! Procedurally-placed ground/terrain layer: the pack's grass-fill tiles as
//! the base, soft patches of its sand tiles drifted over the open floor, and
//! road painted at specific cells - under every static obstacle/wall tile,
//! and inside the player fortress's `B`/`O` glyphs - drawn from the
//! third-party Puny World tileset
//! (`static/punyworld/punyworld-overworld-tileset.png` - see
//! `static/punyworld/SOURCE.md` for provenance and `docs/GROUND_SPEC.md` for
//! the full design writeup, including why this sheet is deliberately not on
//! the Resurrect 64 palette everything else uses).
//!
//! The materials are named after the pack's own wangset colours - grass,
//! sand, dirt paths - not after what they look like on screen: the live PNG
//! is a *retinted* copy (`tools/retint_ground.py`), and under the desert
//! theme the "grass" is pale dust, the "sand" a smoother hardpan and the
//! dirt a packed-earth road. Nothing here knows which theme is live.
//!
//! Purely decorative: no physics body, no gameplay effect. `build` runs
//! once per round (from `simulation::Game::init`, after every obstacle for
//! the round has been placed - see that function's own comments for why)
//! and resolves the whole layout into a flat `Vec<i32>` of source tile ids,
//! one per `GROUND_WORLD_TILE` grid cell - so `draw` is just an index
//! lookup and a blit per cell, no per-frame autotile work.
//!
//! The autotile tables below (`GRASS_FILL`, `SAND_CORNER`, `ROAD_EDGE`) are
//! extracted from the source pack's own Tiled wangset data
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

use crate::tuning::tuning;
use crate::{GROUND_WORLD_TILE, OBSTACLE_GRID_SIZE, Position};

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

/// The pack's sand against its grass, a Wang **corner** autotile: which of
/// a cell's four corners sit on sand decides the tile, so sand is laid at
/// the grid *vertices* (`drift_vertex` in `build`) and every cell between
/// two kinds of vertex gets a hand-painted rounded edge. Indexed by a 4-bit
/// mask, bit3=TL bit2=TR bit1=BR bit0=BL (1 = sand). Mask 0 is not a sand
/// tile at all - a `GRASS_FILL` variant - so the entry is a placeholder
/// `build` never reads. Extracted from the `overworld` wangset the same way
/// `ROAD_EDGE` was (docs/GROUND_SPEC.md).
const SAND_CORNER: [i32; 16] = [
    -1, // 0000 no sand: `GRASS_FILL`
    24, // 0001 BL
    22, // 0010 BR
    23, // 0011 BR+BL (bottom edge)
    76, // 0100 TR
    80, // 0101 TR+BL (diagonal)
    49, // 0110 TR+BR (right edge)
    25, // 0111 all but TL
    78, // 1000 TL
    51, // 1001 TL+BL (left edge)
    79, // 1010 TL+BR (diagonal)
    26, // 1011 all but TR
    77, // 1100 TL+TR (top edge)
    53, // 1101 all but BR
    52, // 1110 all but BL
    50, // 1111 flat sand
];

/// Road (the source pack's "dirt-paths") Wang **edge** autotile - 15 of the
/// 16 possible N/E/S/W neighbour combinations have their own tile; the
/// missing one (no road neighbours at all) can come up here (an isolated
/// road cell, e.g. a lone obstacle tile with no orthogonal road neighbour)
/// unlike the old edge-to-edge road walk this replaced, so index 0 is a
/// real, reachable case now, not just a defensive fallback - the crossroads
/// tile (32) reads fine standing alone too. Indexed by a 4-bit mask,
/// bit3=N bit2=E bit1=S bit0=W (1=road neighbour).
const ROAD_EDGE: [i32; 16] = [
    84, // 0000 isolated - a rounded dirt blob, fully ringed by grass.
        // Not part of the source wangset (an edge wangtile with zero
        // connections cannot be expressed), which is why it was previously
        // the crossroads tile standing in.
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
    /// Per-cell tint, resolved once in `build` alongside the tile id.
    ///
    /// Baked rather than computed per frame on purpose: `draw` already
    /// issues one call per cell over the whole screen and is the single
    /// largest draw cost in the game, so shading has to be free at draw
    /// time. It also never changes during a round - obstacles are only
    /// ever removed, and a hole in a wall letting a little more light onto
    /// the floor is not worth rebuilding the grid for.
    tints: Vec<Color>,
}

impl GroundGrid {
    /// Darken the cell under `pos` by `factor` for the rest of the round:
    /// the crater tone a burnt-out pool leaves. The one thing that changes
    /// a tint after `build`, and it only ever darkens - so the AO bake's
    /// "never changes" reasoning holds for everything else.
    pub fn darken_cell(&mut self, pos: Position, factor: f32) {
        // A 32px obstacle cell spans three of the 16px ground tiles on
        // each axis (the middle one whole, the two beside it by half), so
        // the crater is the 3x3 block around the cell's centre - a little
        // wider than the cell, which is what a burn edge looks like.
        let half = OBSTACLE_GRID_SIZE / 2.0;
        let k = factor.clamp(0.0, 1.0);
        let x0 = ((pos.x - half) / GROUND_WORLD_TILE).round() as i32;
        let x1 = ((pos.x + half) / GROUND_WORLD_TILE).round() as i32;
        let y0 = ((pos.y - half) / GROUND_WORLD_TILE).round() as i32;
        let y1 = ((pos.y + half) / GROUND_WORLD_TILE).round() as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                if let Some(i) = self.idx(x, y) {
                    let c = self.tints[i];
                    self.tints[i] = Color::new((c.r as f32 * k) as u8, (c.g as f32 * k) as u8, (c.b as f32 * k) as u8, c.a);
                }
            }
        }
    }

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

/// Smooth value noise in `0..1` at grid vertex `(vx, vy)`: hashed lattice
/// points every `period` vertices, smoothstep-blended between them. Keyed
/// the same way as `grass_variant` - by `seed` and the lattice coordinates
/// alone - so the editor's rebuilds keep every patch where it was and a
/// round replays its floor from its seed. Thresholded by `build` into the
/// sand drifts; the blend is what makes them soft blobs a few cells across
/// rather than per-cell noise.
fn drift_noise(seed: u64, vx: i32, vy: i32, period: f32) -> f32 {
    let lattice = |ix: i32, iy: i32| -> f32 {
        let mut h = seed ^ 0xD1B5_4A32_D192_ED03;
        h ^= (ix as i64 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= (iy as i64 as u64).wrapping_mul(0x94D0_49BB_1331_11EB);
        h ^= h >> 31;
        h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
        h ^= h >> 29;
        (h >> 40) as f32 / (1u64 << 24) as f32
    };
    let smooth = |u: f32| u * u * (3.0 - 2.0 * u);
    let (fx, fy) = (vx as f32 / period, vy as f32 / period);
    let (ix, iy) = (fx.floor(), fy.floor());
    let (tx, ty) = (smooth(fx - ix), smooth(fy - iy));
    let (ix, iy) = (ix as i32, iy as i32);
    let top = lattice(ix, iy) * (1.0 - tx) + lattice(ix + 1, iy) * tx;
    let bottom = lattice(ix, iy + 1) * (1.0 - tx) + lattice(ix + 1, iy + 1) * tx;
    top * (1.0 - ty) + bottom * ty
}

/// Roll this round's ground layout: grass everywhere, sand drifted over it
/// in soft patches (`ground_drift_cover` of the open floor, none touching a
/// road cell), then road painted at exactly `road_cells` (world positions -
/// typically every static obstacle tile's own position plus the player
/// fortress's `B`/`O` interior cells, see `Game::init`) and nowhere else. `width`/`height` are the same
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
pub fn build(width: f32, height: f32, seed: u64, road_cells: &[Position], wall_cells: &[Position]) -> GroundGrid {
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

    // --- drifts: sand at the grid vertices, resolved per cell through the
    // corner autotile below ---
    //
    // A vertex touching a road cell never drifts: the road tiles carry the
    // plain fill's dithered edge baked in, so a road running through a
    // patch would show a fringe of the wrong tone along it. The patches
    // live in the open instead, which is also where they read.
    let t = tuning();
    let cover = t.ground_drift_cover.clamp(0.0, 1.0);
    let period = t.ground_drift_scale.max(1.0);
    let drift_vertex = |m: &[Material], vx: i32, vy: i32| -> bool {
        if cover <= 0.0 {
            return false;
        }
        let beside_road = [(vx - 1, vy - 1), (vx, vy - 1), (vx - 1, vy), (vx, vy)]
            .iter()
            .any(|&(cx, cy)| at(m, cx, cy) == Material::Road);
        !beside_road && drift_noise(seed, vx, vy, period) > 1.0 - cover
    };

    // --- resolve: pick the exact source tile for every cell ---
    let mut tiles = vec![0i32; cols * rows];
    for y in 0..rows as i32 {
        for x in 0..cols as i32 {
            let tile = match at(&material, x, y) {
                Material::Grass => {
                    let corner = |vx: i32, vy: i32| drift_vertex(&material, vx, vy) as usize;
                    let mask = (corner(x, y) << 3) | (corner(x + 1, y) << 2) | (corner(x + 1, y + 1) << 1) | corner(x, y + 1);
                    if mask == 0 { grass_variant(seed, x, y) } else { SAND_CORNER[mask] }
                }
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

    // --- shade: darken toward walls and toward the screen edge ---
    //
    // Walls sit *on* the ground, so the cells around them read as being in
    // their shadow; without it a wall looks pasted on rather than standing
    // on the floor.
    //
    // A per-cell tint is only defensible because these steps land *on the
    // wall grid*, where a boundary reads as the edge of a shadow. The
    // screen-edge vignette was tried the same way and had to be pulled: a
    // flat tint per 32px cell over flat-coloured grass stair-steps no
    // matter how smooth the underlying field is, and out in the open there
    // is no wall for the steps to align with, so it read as banding. It is
    // a smooth per-pixel gradient in `draw_edge_shade` instead.
    let mut walls = vec![false; cols * rows];
    for pos in wall_cells {
        let gx = (pos.x / GROUND_WORLD_TILE).round() as i32;
        let gy = (pos.y / GROUND_WORLD_TILE).round() as i32;
        if gx >= 0 && gy >= 0 && (gx as usize) < cols && (gy as usize) < rows {
            walls[gy as usize * cols + gx as usize] = true;
        }
    }
    let near = t.ground_wall_shade;
    let reach = t.ground_wall_shade_cells.max(0) as i32;
    let mut tints = vec![Color::WHITE; cols * rows];
    for y in 0..rows as i32 {
        for x in 0..cols as i32 {
            // Euclidean, not Chebyshev: a box distance gives square
            // iso-contours, and since the tint is per 32px cell those show
            // up as visible rectangular bands rather than as a shadow.
            let mut nearest = f32::MAX;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx as usize >= cols || ny as usize >= rows {
                        continue;
                    }
                    if walls[ny as usize * cols + nx as usize] {
                        let d = ((dx * dx + dy * dy) as f32).sqrt();
                        nearest = nearest.min(d);
                    }
                }
            }
            // 1.0 right beside a wall, easing to 0 at `reach`. Smoothstep
            // rather than linear so the falloff has no hard outer edge.
            let wall_k = if nearest == f32::MAX || reach == 0 {
                0.0
            } else {
                let u = (1.0 - nearest / (reach as f32 + 1.0)).clamp(0.0, 1.0);
                u * u * (3.0 - 2.0 * u)
            };
            let shade = (1.0 - wall_k * near).clamp(0.0, 1.0);
            let v = (255.0 * shade) as u8;
            tints[y as usize * cols + x as usize] = Color::new(v, v, v, 255);
        }
    }

    GroundGrid { cols, rows, tiles, tints }
}

/// Darken the ground toward the screen edges, pulling the eye to the middle
/// of the battlefield and buying back a little of the value range a screen
/// full of grass spends.
///
/// Four gradient bands rather than a per-cell tint: the tint is flat across
/// a whole 32px cell, so over open grass a gradient built that way
/// stair-steps visibly. These interpolate per pixel. Drawn straight after
/// the ground so it shades the floor only - tanks and walls stand in front
/// of it, not under it.
pub fn draw_edge_shade(d: &mut impl RaylibDraw, width: i32, height: i32) {
    let strength = tuning().ground_edge_shade;
    if strength <= 0.0 {
        return;
    }
    let a = (255.0 * strength).clamp(0.0, 255.0) as u8;
    let dark = Color::new(0, 0, 0, a);
    let clear = Color::new(0, 0, 0, 0);
    let band = (tuning().ground_edge_shade_px).max(1.0) as i32;
    // `_v` runs top->bottom and `_h` runs left->right, so the far edges
    // pass the colours the other way round.
    d.draw_rectangle_gradient_v(0, 0, width, band, dark, clear);
    d.draw_rectangle_gradient_v(0, height - band, width, band, clear, dark);
    d.draw_rectangle_gradient_h(0, 0, band, height, dark, clear);
    d.draw_rectangle_gradient_h(width - band, 0, band, height, clear, dark);
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
            d.draw_texture_pro(texture, src, dest, origin, 0.0, grid.tints[i]);
        }
    }
}
