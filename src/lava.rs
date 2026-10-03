//! Lava (docs/volcano.md): the rules' and the picture's view of a map's
//! lava cells, built from the map alone (no RNG, no world) so `Game::init`
//! can have it before anything is placed, the way water's is.
//!
//! The shape is water's: `ground::WaterLayout` reads the painted cells, so
//! a line one cell wide is a stream - a ford, `Depth::Shallow` - and the
//! open middle of a block three or more wide is a lake, `Depth::Deep`, a
//! wall to hulls and nothing to shots. Lava never freezes and never joins
//! water. On top of the shape this keeps what water has no use for:
//!
//! - **Flow** runs away from the volcano. Every lava cell beside a
//!   volcano's cone is a source, and a walk along the lava from the
//!   sources gives each cell its distance downstream and its direction; a
//!   run that touches no cone - one a road crossing cuts off, or one
//!   painted on its own - flows from its end nearest a volcano, or with
//!   none on the map from its northernmost end, down the map like water.
//! - **Heat** radiates from every lava cell: 1 in the lava, falling by
//!   `lava_heat_falloff` a cell out (by the larger of the two offsets) and
//!   gone past four. What the scorched banks, the lights and
//!   the damage a hull takes all read (`Game::heat_at`).
//!
//! The drawing (`draw_cell`, `draw_floes`) is composed in the effects
//! language, block by block from a cell's neighbour mask - banks of crust
//! where a side is closed, rounded outer corners, a nub at an inner one,
//! and a molten body whose bands run downstream - generic over
//! `canvas::Canvas`, so a thumbnail and the builder draw what a round does.

use crate::canvas::{BlockImage, Canvas, TileAtlas, TILE_BLOCKS};
use crate::ground::{Depth, WaterLayout};
use crate::math::{Color, Rectangle, Vec2};
use crate::pyro::{FIRE, SMOKE};
use crate::{Position, GROUND_WORLD_TILE, OBSTACLE_GRID_SIZE};

/// How far heat reaches from the lava, in cells.
pub const HEAT_REACH: i32 = 4;

/// Heat under this is none at all.
const HEAT_FLOOR: f32 = 0.05;

/// A cell's side in px.
const CELL: f32 = OBSTACLE_GRID_SIZE;

/// The rules' and the picture's view of a map's lava.
#[derive(Clone, Default)]
pub struct LavaLayout {
    cols: usize,
    rows: usize,
    /// The shape, read the way water's is: what is a ford and what deep.
    shape: WaterLayout,
    /// Painted lava per cell - what is drawn, ford or lake alike.
    lava: Vec<bool>,
    /// Per lava cell: part of a 2 x 2 block (a lake's body, drawn without
    /// a current).
    lake: Vec<bool>,
    /// Per lava cell: the unit direction it flows in.
    flow: Vec<Vec2>,
    /// Per lava cell: cells downstream of its source.
    along: Vec<f32>,
    /// Per cell: the heat the lava radiates, 0..1.
    heat: Vec<f32>,
    /// Per lava cell: which sides (N, E, S, W) run on into more lava, a
    /// volcano's cone or off the map - the rest are banks.
    open: Vec<u8>,
    /// Per lava cell: which diagonals (NW, NE, SW, SE) are lava, so an
    /// inner corner gets its nub.
    diag: Vec<u8>,
    /// The field's size in px, which `banks` covers.
    field: (f32, f32),
    /// The scorched banks baked onto the floor, made the first time they
    /// are drawn (`banks`), so a room server never pays for them.
    banks: std::sync::OnceLock<crate::canvas::BlockImage>,
    /// The picture kept between frames (`refresh_pictures`), made only
    /// where a window draws the round.
    pictures: std::cell::RefCell<LavaPictures>,
}

/// The four sides' bits in `LavaLayout::open`.
pub const N: u8 = 1;
pub const E: u8 = 2;
pub const S: u8 = 4;
pub const W: u8 = 8;

impl LavaLayout {
    /// Build from the map's lava cells (world positions), with the cells
    /// painted road or wall over them (`road_cells`) - a wall stands on
    /// dirt - and the cells under every volcano's cone, which the lava
    /// leaving them flows away from. `falloff` is `lava_heat_falloff`.
    pub fn build(width: f32, height: f32, road_cells: &[Position], lava_cells: &[Position], cones: &[(i32, i32)], falloff: f32) -> LavaLayout {
        let map_cols = (width / GROUND_WORLD_TILE).ceil().max(1.0) as usize;
        let map_rows = (height / GROUND_WORLD_TILE).ceil().max(1.0) as usize;
        let (cols, rows) = (map_cols + 1, map_rows + 1);
        let shape = WaterLayout::build(width, height, road_cells, lava_cells);
        let mut lava = vec![false; cols * rows];
        for pos in lava_cells {
            let (c, r) = cell_of(*pos);
            if c >= 0 && r >= 0 && (c as usize) < cols && (r as usize) < rows && shape.depth_at(*pos) != Depth::Dry {
                lava[r as usize * cols + c as usize] = true;
            }
        }
        let mut layout = LavaLayout {
            cols,
            rows,
            shape,
            lava,
            lake: vec![false; cols * rows],
            flow: vec![Vec2::zero(); cols * rows],
            along: vec![0.0; cols * rows],
            heat: vec![0.0; cols * rows],
            open: vec![0; cols * rows],
            diag: vec![0; cols * rows],
            field: (width, height),
            banks: std::sync::OnceLock::new(),
            pictures: std::cell::RefCell::new(LavaPictures::default()),
        };
        if layout.is_empty() {
            return layout;
        }
        let cone: std::collections::BTreeSet<(i32, i32)> =
            cones.iter().flat_map(|&(c, r)| crate::volcano::footprint(c, r)).collect();
        layout.settle_masks(&cone, map_cols as i32, map_rows as i32);
        layout.settle_flow(&cone);
        layout.settle_heat(falloff);
        layout
    }

    /// True when the map has no lava at all - every rule short-circuits.
    pub fn is_empty(&self) -> bool {
        !self.lava.contains(&true)
    }

    fn index(&self, c: i32, r: i32) -> Option<usize> {
        (c >= 0 && r >= 0 && (c as usize) < self.cols && (r as usize) < self.rows).then(|| r as usize * self.cols + c as usize)
    }

    /// Lava painted at cell `(c, r)`.
    pub fn is_lava(&self, c: i32, r: i32) -> bool {
        self.index(c, r).is_some_and(|i| self.lava[i])
    }

    fn settle_masks(&mut self, cone: &std::collections::BTreeSet<(i32, i32)>, map_cols: i32, map_rows: i32) {
        for r in 0..self.rows as i32 {
            for c in 0..self.cols as i32 {
                let i = r as usize * self.cols + c as usize;
                if !self.lava[i] {
                    continue;
                }
                // A side runs on into lava, into a cone the lava leaves,
                // or off the map's edge, as water does.
                let runs = |dc: i32, dr: i32| {
                    let (nc, nr) = (c + dc, r + dr);
                    self.is_lava(nc, nr) || cone.contains(&(nc, nr)) || nc < 0 || nr < 0 || nc >= map_cols || nr >= map_rows
                };
                let mut open = 0;
                for (bit, dc, dr) in [(N, 0, -1), (E, 1, 0), (S, 0, 1), (W, -1, 0)] {
                    if runs(dc, dr) {
                        open |= bit;
                    }
                }
                self.open[i] = open;
                let mut diag = 0;
                for (bit, dc, dr) in [(1u8, -1, -1), (2, 1, -1), (4, -1, 1), (8, 1, 1)] {
                    if self.is_lava(c + dc, r + dr) || cone.contains(&(c + dc, r + dr)) {
                        diag |= bit;
                    }
                }
                self.diag[i] = diag;
                let block = |dc: i32, dr: i32| self.is_lava(c + dc, r) && self.is_lava(c, r + dr) && self.is_lava(c + dc, r + dr);
                self.lake[i] = block(1, 1) || block(-1, 1) || block(1, -1) || block(-1, -1);
            }
        }
    }

    /// Walk the lava from its sources: each cell's distance downstream and
    /// the direction it flows in. Ties break on cell order, no RNG.
    fn settle_flow(&mut self, cones: &std::collections::BTreeSet<(i32, i32)>) {
        const UNSEEN: u32 = u32::MAX;
        let mut dist = vec![UNSEEN; self.cols * self.rows];
        let mut queue = std::collections::VecDeque::new();
        let beside_cone = |c: i32, r: i32| (-1..=1).any(|dr| (-1..=1).any(|dc| cones.contains(&(c + dc, r + dr))));
        for r in 0..self.rows as i32 {
            for c in 0..self.cols as i32 {
                let i = r as usize * self.cols + c as usize;
                if self.lava[i] && beside_cone(c, r) {
                    dist[i] = 0;
                    queue.push_back((c, r));
                }
            }
        }
        loop {
            while let Some((c, r)) = queue.pop_front() {
                let d = dist[self.index(c, r).expect("queued cells are on the grid")];
                for (dc, dr) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                    if let Some(j) = self.index(c + dc, r + dr) {
                        if self.lava[j] && dist[j] == UNSEEN {
                            dist[j] = d + 1;
                            queue.push_back((c + dc, r + dr));
                        }
                    }
                }
            }
            // A run no cone feeds - cut off by a crossing, or painted on its
            // own - flows from its end nearest a volcano, or with none on
            // the map from its northernmost end, down the map like water.
            let unseen: Vec<usize> = (0..self.lava.len()).filter(|&i| self.lava[i] && dist[i] == UNSEEN).collect();
            let Some(&first) = unseen.first() else { break };
            let component = self.component(first);
            let ends: Vec<usize> = component.iter().copied().filter(|&i| self.lava_neighbours(i) <= 1).collect();
            let pick = |cells: &[usize]| -> Option<usize> {
                if cones.is_empty() {
                    return cells.first().copied();
                }
                cells.iter().copied().min_by(|&a, &b| self.cone_distance(a, cones).total_cmp(&self.cone_distance(b, cones)))
            };
            let i = pick(&ends).or_else(|| pick(&component)).unwrap_or(first);
            dist[i] = 0;
            queue.push_back(((i % self.cols) as i32, (i / self.cols) as i32));
        }
        for r in 0..self.rows as i32 {
            for c in 0..self.cols as i32 {
                let i = r as usize * self.cols + c as usize;
                if !self.lava[i] {
                    continue;
                }
                let d = dist[i];
                self.along[i] = d as f32;
                let mut down = Vec2::zero();
                let mut up = Vec2::zero();
                for (dc, dr) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                    let Some(j) = self.index(c + dc, r + dr) else { continue };
                    if !self.lava[j] {
                        continue;
                    }
                    if dist[j] == d + 1 {
                        down = down + Vec2::new(dc as f32, dr as f32);
                    } else if dist[j] + 1 == d {
                        up = up + Vec2::new(dc as f32, dr as f32);
                    }
                }
                // A source flows away from the cone it leaves.
                if d == 0 && up.length() == 0.0 {
                    for (dc, dr) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                        if cones.contains(&(c + dc, r + dr)) {
                            up = up + Vec2::new(dc as f32, dr as f32);
                        }
                    }
                }
                // In through the cells upstream, out through the ones
                // downstream; the end of a run carries straight on.
                let dir = down - up;
                self.flow[i] = if dir.length() > 1e-3 {
                    unit(dir)
                } else if down.length() > 1e-3 {
                    unit(down)
                } else if up.length() > 1e-3 {
                    unit(Vec2::zero() - up)
                } else {
                    Vec2::new(0.0, 1.0)
                };
            }
        }
    }

    /// The lava cells joined to cell `i` edge to edge, in index order.
    fn component(&self, i: usize) -> Vec<usize> {
        let mut seen = std::collections::BTreeSet::from([i]);
        let mut stack = vec![i];
        while let Some(j) = stack.pop() {
            let (c, r) = ((j % self.cols) as i32, (j / self.cols) as i32);
            for (dc, dr) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                if let Some(k) = self.index(c + dc, r + dr) {
                    if self.lava[k] && seen.insert(k) {
                        stack.push(k);
                    }
                }
            }
        }
        seen.into_iter().collect()
    }

    /// How many of cell `i`'s four neighbours are lava.
    fn lava_neighbours(&self, i: usize) -> usize {
        let (c, r) = ((i % self.cols) as i32, (i / self.cols) as i32);
        [(0, -1), (1, 0), (0, 1), (-1, 0)].iter().filter(|&&(dc, dr)| self.is_lava(c + dc, r + dr)).count()
    }

    /// How far cell `i` lies from the nearest of the cones' cells.
    fn cone_distance(&self, i: usize, cones: &std::collections::BTreeSet<(i32, i32)>) -> f32 {
        let (c, r) = ((i % self.cols) as i32, (i / self.cols) as i32);
        cones.iter().map(|&(cc, cr)| (((cc - c) * (cc - c) + (cr - r) * (cr - r)) as f32).sqrt()).fold(f32::MAX, f32::min)
    }

    fn settle_heat(&mut self, falloff: f32) {
        let falloff = falloff.clamp(0.0, 1.0);
        for r in 0..self.rows as i32 {
            for c in 0..self.cols as i32 {
                let mut near = i32::MAX;
                for dr in -HEAT_REACH..=HEAT_REACH {
                    for dc in -HEAT_REACH..=HEAT_REACH {
                        if self.is_lava(c + dc, r + dr) {
                            near = near.min(dc.abs().max(dr.abs()));
                        }
                    }
                }
                if near == i32::MAX {
                    continue;
                }
                let h = (0..near).fold(1.0f32, |h, _| h * falloff);
                if h >= HEAT_FLOOR {
                    self.heat[r as usize * self.cols + c as usize] = h;
                }
            }
        }
    }

    /// The lava under a world position (a hull's centre): a ford, a lake's
    /// deep middle, or dry. Never ice.
    pub fn depth_at(&self, pos: Position) -> Depth {
        self.shape.depth_at(pos)
    }

    /// The heat the lava radiates at cell `(c, r)`, 0..1.
    pub fn heat(&self, c: i32, r: i32) -> f32 {
        self.index(c, r).map_or(0.0, |i| self.heat[i])
    }

    /// The heat at a world position, by the cell it falls in.
    pub fn heat_at(&self, pos: Position) -> f32 {
        let (c, r) = cell_of(pos);
        self.heat(c, r)
    }

    /// The direction lava flows in at cell `(c, r)` (zero off the lava).
    pub fn flow(&self, c: i32, r: i32) -> Vec2 {
        self.index(c, r).map_or(Vec2::zero(), |i| self.flow[i])
    }

    /// Cells downstream of the source at cell `(c, r)`.
    pub fn along(&self, c: i32, r: i32) -> f32 {
        self.index(c, r).map_or(0.0, |i| self.along[i])
    }

    /// The centre of every deep cell: the static colliders a round spawns
    /// and the cells the nav grid blocks.
    pub fn deep_cells(&self) -> impl Iterator<Item = Position> + '_ {
        self.shape.deep_cells()
    }

    /// The centre of every ford cell: the cells the nav grid weighs.
    pub fn ford_cells(&self) -> impl Iterator<Item = Position> + '_ {
        self.shape.shallow_cells()
    }

    /// Grid coordinates of every deep cell, for seam-closing colliders.
    pub fn deep_grid_cells(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.shape.deep_grid_cells()
    }

    /// Every lava cell, row by row.
    pub fn cells(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        let cols = self.cols;
        self.lava.iter().enumerate().filter(|(_, l)| **l).map(move |(i, _)| ((i % cols) as i32, (i / cols) as i32))
    }

    /// Every cell with any heat on it and that heat, row by row.
    pub fn hot_cells(&self) -> impl Iterator<Item = ((i32, i32), f32)> + '_ {
        let cols = self.cols;
        self.heat.iter().enumerate().filter(|(_, h)| **h > 0.0).map(move |(i, h)| (((i % cols) as i32, (i / cols) as i32), *h))
    }
}

/// `v` scaled to length 1.
fn unit(v: Vec2) -> Vec2 {
    let l = v.length();
    Vec2::new(v.x / l, v.y / l)
}

/// The cell a world position falls in.
fn cell_of(pos: Position) -> (i32, i32) {
    ((pos.x / CELL).round() as i32, (pos.y / CELL).round() as i32)
}

/// 0..1 from three integers: a cosmetic choice hashed, never rolled.
pub fn hash3(a: i32, b: i32, c: i32) -> f32 {
    let mut h = (a as u32).wrapping_mul(374_761_393) ^ (b as u32).wrapping_mul(668_265_263) ^ (c as u32).wrapping_mul(1_442_695_041);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// How the lava looks this frame: the eruption's surge (0 calm, 1 at its
/// height - faster, brighter, its banks a block narrower) and how fast its
/// bands run downstream, px a second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub surge: f32,
    pub speed: f32,
}

/// The crust: black, the darkest grey and the shade (`SMOKE`).
const CRUST: [Color; 3] = [SMOKE[0], SMOKE[1], SMOKE[2]];

/// The molten steps, dark red to pale gold (`FIRE` less white).
const MOLTEN: [Color; 7] = [FIRE[0], FIRE[1], FIRE[2], FIRE[3], FIRE[4], FIRE[5], FIRE[6]];

/// The colour of the block at (`u`, `v`) px into cell `(c, r)` (block
/// centres, 1..31), or `None` where the cell's ground shows. `bright`
/// says whether it is molten enough to shine by itself.
pub fn block(layout: &LavaLayout, c: i32, r: i32, u: i32, v: i32, time: f32, look: Look) -> Option<(Color, bool)> {
    let geo = block_geo(layout, c, r, u, v)?;
    Some(block_color(&geo, c, r, u, v, time, look))
}

/// What of a lava block never changes: where it lies in its cell's shape
/// and its hashes - worked out once per layout for the cached picture
/// (`LavaPictures`), and every call for `block`.
#[derive(Clone, Copy, Debug)]
struct BlockGeo {
    /// How far inside the cell's banks the block lies (px).
    d: f32,
    /// The bank here is a block wider (the ragged edge's hash).
    wide: bool,
    /// The block's own hash.
    hb: f32,
    /// A crust block on a north or west bank catching the light.
    lit: bool,
    /// A lake's body, drawn without a current.
    lake: bool,
    /// Along the flow (px) and across it, for the bands.
    s: f32,
    lateral: f32,
}

/// The fixed part of block `(u, v)` of cell `(c, r)`; `None` off the lava.
fn block_geo(layout: &LavaLayout, c: i32, r: i32, u: i32, v: i32) -> Option<BlockGeo> {
    let i = layout.index(c, r)?;
    if !layout.lava[i] {
        return None;
    }
    let open = layout.open[i];
    let diag = layout.diag[i];
    let (uf, vf) = (u as f32, v as f32);
    let wide = hash3(c * 32 + (u >> 2), r * 32 + (v >> 2), 7) < 0.35;
    let mut d: f32 = 99.0;
    if open & N == 0 {
        d = d.min(vf);
    }
    if open & S == 0 {
        d = d.min(32.0 - vf);
    }
    if open & W == 0 {
        d = d.min(uf);
    }
    if open & E == 0 {
        d = d.min(32.0 - uf);
    }
    const RC: f32 = 13.0;
    let corner = |a: f32, b: f32| RC - ((RC - a) * (RC - a) + (RC - b) * (RC - b)).sqrt();
    if open & (N | W) == 0 && uf < RC && vf < RC {
        d = d.min(corner(uf, vf));
    }
    if open & (N | E) == 0 && 32.0 - uf < RC && vf < RC {
        d = d.min(corner(32.0 - uf, vf));
    }
    if open & (S | W) == 0 && uf < RC && 32.0 - vf < RC {
        d = d.min(corner(uf, 32.0 - vf));
    }
    if open & (S | E) == 0 && 32.0 - uf < RC && 32.0 - vf < RC {
        d = d.min(corner(32.0 - uf, 32.0 - vf));
    }
    let nub = |a: f32, b: f32| (a * a + b * b).sqrt() - 3.0;
    if open & N != 0 && open & W != 0 && diag & 1 == 0 {
        d = d.min(nub(uf, vf));
    }
    if open & N != 0 && open & E != 0 && diag & 2 == 0 {
        d = d.min(nub(32.0 - uf, vf));
    }
    if open & S != 0 && open & W != 0 && diag & 4 == 0 {
        d = d.min(nub(uf, 32.0 - vf));
    }
    if open & S != 0 && open & E != 0 && diag & 8 == 0 {
        d = d.min(nub(32.0 - uf, 32.0 - vf));
    }
    if d < 0.0 {
        return None;
    }
    let hb = hash3(c * 64 + u, r * 64 + v, 5);
    let lit = ((open & N == 0 && v < 8) || (open & W == 0 && u < 8)) && hb > 0.7;
    let dir = layout.flow[i];
    let s = layout.along[i] * CELL + (uf - 16.0) * dir.x + (vf - 16.0) * dir.y;
    let lateral = (uf * dir.y - vf * dir.x) * 0.12;
    Some(BlockGeo { d, wide, hb, lit, lake: layout.lake[i], s, lateral })
}

/// The colour of a block with the fixed part `geo` at `time` under `look`,
/// and whether it is molten enough to shine.
fn block_color(geo: &BlockGeo, c: i32, r: i32, u: i32, v: i32, time: f32, look: Look) -> (Color, bool) {
    let bank = 7.0 - 3.0 * look.surge + if geo.wide { 2.0 } else { 0.0 };
    let (d, hb) = (geo.d, geo.hb);
    if d < bank {
        if d < bank * 0.55 {
            return (if hb < 0.3 { CRUST[1] } else if geo.lit { CRUST[2] } else { CRUST[0] }, false);
        }
        return if look.surge > 0.4 {
            (if hb < 0.5 { MOLTEN[2] } else { MOLTEN[3] }, hb >= 0.5)
        } else {
            (if hb < 0.6 { MOLTEN[1] } else { MOLTEN[2] }, false)
        };
    }
    let m = ((d - bank) / (16.0 - bank)).clamp(0.0, 1.0);
    if geo.lake {
        return lake_block(c, r, u, v, time, look, m);
    }
    let band = 0.5 + 0.5 * crate::trig::sin((geo.s - time * look.speed) / 13.0 * std::f32::consts::TAU + geo.lateral);
    let flick = hash3(c * 64 + u, r * 64 + v, (time * 3.0 + hb * 4.0).floor() as i32);
    let val = 0.22 + 0.42 * m + 0.24 * band * (0.4 + 0.6 * m) + (flick - 0.5) * 0.12 + look.surge * 0.14;
    let step = if val > 0.93 {
        6
    } else if val > 0.78 {
        5
    } else if val > 0.62 {
        4
    } else if val > 0.42 {
        3
    } else if val > 0.28 {
        2
    } else {
        1
    };
    (MOLTEN[step], step >= 3)
}

/// A block of a lake's body: plates of crust drifting slowly over the
/// molten rock, glowing cracks between them, the rock showing through
/// redder toward the shore. A surge melts the crust back.
fn lake_block(c: i32, r: i32, u: i32, v: i32, time: f32, look: Look, m: f32) -> (Color, bool) {
    let (x, y) = ((c * 32 + u) as f32, (r * 32 + v) as f32);
    let n = noise2(x / 22.0 + time * 0.12, y / 22.0 - time * 0.07, 13) * 0.7 + noise2(x / 9.0, y / 9.0 + time * 0.2, 17) * 0.3;
    let crust = 0.44 - 0.22 * look.surge + 0.08 * (1.0 - m);
    let hb = hash3(c * 64 + u, r * 64 + v, 9);
    if n < crust {
        let lit = noise2((x - 2.0) / 22.0 + time * 0.12, (y - 2.0) / 22.0 - time * 0.07, 13) * 0.7 + noise2((x - 2.0) / 9.0, (y - 2.0) / 9.0 + time * 0.2, 17) * 0.3;
        return (if lit >= crust { CRUST[2] } else if hb < 0.25 { CRUST[0] } else { CRUST[1] }, false);
    }
    if n < crust + 0.05 {
        return (if hb < 0.5 { MOLTEN[5] } else { MOLTEN[4] }, true);
    }
    let flick = hash3(c * 64 + u, r * 64 + v, (time * 3.0 + hb * 4.0).floor() as i32);
    let step = if flick > 0.93 {
        5
    } else if n > 0.72 || flick > 0.8 {
        4
    } else if m > 0.4 {
        3
    } else {
        2
    };
    (MOLTEN[step], step >= 3)
}

/// Value noise in 0..1, smooth between whole coordinates.
fn noise2(x: f32, y: f32, s: i32) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (xf, yf) = (x - xi, y - yi);
    let (u, v) = (xf * xf * (3.0 - 2.0 * xf), yf * yf * (3.0 - 2.0 * yf));
    let (xi, yi) = (xi as i32, yi as i32);
    let a = hash3(xi, yi, s) + (hash3(xi + 1, yi, s) - hash3(xi, yi, s)) * u;
    let b = hash3(xi, yi + 1, s) + (hash3(xi + 1, yi + 1, s) - hash3(xi, yi + 1, s)) * u;
    a + (b - a) * v
}

/// Paint lava cell `(c, r)` block by block, a row's run of one colour as
/// one rect. With `bright_only` only the blocks molten enough to shine are
/// painted: the glowing pass draws those again over a darkened field, so
/// lava lights the night rather than being dimmed by it.
pub fn draw_cell(canvas: &mut impl Canvas, layout: &LavaLayout, c: i32, r: i32, time: f32, look: Look, bright_only: bool, alpha: f32) {
    let (ox, oy) = (c * CELL as i32 - CELL as i32 / 2, r * CELL as i32 - CELL as i32 / 2);
    let fade = |col: Color| if alpha >= 1.0 { col } else { Color::new(col.r, col.g, col.b, (col.a as f32 * alpha.clamp(0.0, 1.0)) as u8) };
    for v in (1..32).step_by(2) {
        let mut run: Option<(i32, Color)> = None;
        for u in (1..=33).step_by(2) {
            let here = if u < 32 {
                block(layout, c, r, u, v, time, look).and_then(|(col, bright)| (!bright_only || bright).then_some(col))
            } else {
                None
            };
            match (run, here) {
                (Some((_, col)), Some(now)) if now == col => {}
                (Some((start, col)), _) => {
                    canvas.fill_rect(ox + start - 1, oy + v - 1, u - start, 2, fade(col));
                    run = here.map(|now| (u, now));
                }
                (None, Some(now)) => run = Some((u, now)),
                (None, None) => {}
            }
        }
    }
}

/// The crust floes drifting down every stream cell `(c, r)` carries: a
/// floe every 19 px of the river's length, hashed in or out and to one
/// side, moving with the bands so they cross from cell to cell.
pub fn draw_floes(canvas: &mut impl Canvas, layout: &LavaLayout, c: i32, r: i32, time: f32, look: Look) {
    const GAP: f32 = 19.0;
    let Some(i) = layout.index(c, r) else { return };
    if !layout.lava[i] || layout.lake[i] {
        return;
    }
    let dir = layout.flow[i];
    let centre = Vec2::new(c as f32 * CELL, r as f32 * CELL);
    let s0 = layout.along[i] * CELL - CELL * 0.5;
    let shift = time * look.speed;
    let first = ((s0 - shift) / GAP).floor() as i32;
    for k in first..=first + 2 {
        let s = k as f32 * GAP + shift;
        if s < s0 || s >= s0 + CELL {
            continue;
        }
        // The same floe in every cell it crosses: its slot, not the cell.
        if hash3(k, 2, 11) > 0.55 - 0.2 * look.surge {
            continue;
        }
        let off = (hash3(k, 3, 11) - 0.5) * 9.0 * (1.0 - look.surge * 0.5);
        let at = centre + dir * (s - s0 - CELL * 0.5) + Vec2::new(-dir.y, dir.x) * off;
        let (x, y) = (crate::pyro::snap(at.x), crate::pyro::snap(at.y));
        let w = if hash3(k, 4, 11) < 0.5 { 4 } else { 6 };
        let h = if hash3(k, 5, 11) < 0.5 { 4 } else { 2 };
        canvas.fill_rect(x - w / 2 + 2, y - h / 2 + 2, w, h, CRUST[0]);
        canvas.fill_rect(x - w / 2, y - h / 2, w, h, CRUST[1]);
        canvas.fill_rect(x - w / 2, y - h / 2, 2, 2, CRUST[2]);
    }
}

impl LavaLayout {
    /// The ground the lava toasts, baked once onto the floor: within a
    /// cell and a half of its edge, dry earth, then rust, then char toward
    /// the lava, in steps dithered on the 2 px block grid from the field's
    /// corner (`pyro::CHAR`, `pyro::DUST`), the reach ragged by a hash. No
    /// RNG; an empty layout bakes a one-texel image nothing shows through.
    pub fn banks(&self) -> &crate::canvas::BlockImage {
        self.banks.get_or_init(|| {
            let stamp = crate::ground::next_block_stamp();
            if self.is_empty() {
                return crate::canvas::BlockImage { width: 1, height: 1, block: 2, texels: vec![Color::new(0, 0, 0, 0)], stamp, patches: Vec::new() };
            }
            let (w, h) = (((self.field.0 / 2.0).ceil() as usize).max(1), ((self.field.1 / 2.0).ceil() as usize).max(1));
            let mut texels = vec![Color::new(0, 0, 0, 0); w * h];
            // Dry earth, then rust, then char, toward the lava's edge.
            let steps = [
                Color::new(crate::pyro::DUST[0].r, crate::pyro::DUST[0].g, crate::pyro::DUST[0].b, 64),
                Color::new(crate::pyro::CHAR[3].r, crate::pyro::CHAR[3].g, crate::pyro::CHAR[3].b, 104),
                Color::new(crate::pyro::CHAR[0].r, crate::pyro::CHAR[0].g, crate::pyro::CHAR[0].b, 136),
            ];
            for by in 0..h {
                for bx in 0..w {
                    let (x, y) = (bx as f32 * 2.0 + 1.0, by as f32 * 2.0 + 1.0);
                    let (c, r) = cell_of(Position::new(x, y));
                    if self.heat(c, r) <= 0.0 || self.is_lava(c, r) {
                        continue;
                    }
                    // How far the block stands from the nearest lava
                    // cell's edge, a ragged reach hashed per block.
                    let mut near = f32::MAX;
                    for rr in r - 2..=r + 2 {
                        for cc in c - 2..=c + 2 {
                            if self.is_lava(cc, rr) {
                                let (lx, ly) = (cc as f32 * CELL, rr as f32 * CELL);
                                let dx = ((x - lx).abs() - CELL * 0.5).max(0.0);
                                let dy = ((y - ly).abs() - CELL * 0.5).max(0.0);
                                near = near.min((dx * dx + dy * dy).sqrt());
                            }
                        }
                    }
                    let reach = 34.0 + 12.0 * hash3(bx as i32 / 4, by as i32 / 4, 41);
                    let toast = 1.0 - near / reach;
                    if toast <= 0.0 {
                        continue;
                    }
                    let level = toast * steps.len() as f32;
                    let i = level.floor() as usize;
                    let i = if level - i as f32 > crate::pyro::bayer(bx as i32, by as i32) { i + 1 } else { i };
                    if i == 0 {
                        continue;
                    }
                    texels[by * w + bx] = steps[(i - 1).min(steps.len() - 1)];
                }
            }
            crate::canvas::BlockImage { width: w, height: h, block: 2, texels, stamp, patches: Vec::new() }
        })
    }
}

/// How hot a splash of lava from a bomb still is, 1 to 0, with `left` of
/// its `total` seconds to burn: white-hot for its first two fifths, then
/// cooling to a crust.
pub fn pool_heat(left: f32, total: f32) -> f32 {
    let age = (total - left).max(0.0);
    let hot = total * 0.4;
    if age < hot { 1.0 } else { (1.0 - (age - hot) / (total - hot).max(1e-3)).clamp(0.0, 1.0) }
}

/// The splashes of lava the bombs left (`GroundFire::lava`): every cell a
/// pull in one metaball field, so a bomb's cells run together into one
/// pool, molten in the middle, crusting at its edge and going grey as it
/// cools. `pools` is each cell with its heat (`pool_heat`). Each 2 px
/// block is worked out once; hashed, never rolled.
pub fn draw_pools(canvas: &mut impl Canvas, pools: &[((i32, i32), f32)], time: f32) {
    if pools.is_empty() {
        return;
    }
    let at: std::collections::BTreeMap<(i32, i32), f32> = pools.iter().copied().collect();
    let pull = |x: f32, y: f32| -> (f32, f32) {
        let (cc, cr) = ((x / CELL).round() as i32, (y / CELL).round() as i32);
        let mut field = 0.0;
        let mut heat: f32 = 0.0;
        for r in cr - 1..=cr + 1 {
            for c in cc - 1..=cc + 1 {
                let Some(&h) = at.get(&(c, r)) else { continue };
                let (dx, dy) = (x - c as f32 * CELL, y - r as f32 * CELL);
                let k = 1.0 - (dx * dx + dy * dy) / (POOL_REACH * POOL_REACH);
                if k > 0.0 {
                    field += k * k;
                    heat = heat.max(h * (k * 2.2).min(1.0));
                }
            }
        }
        (field, heat)
    };
    let mut done = std::collections::BTreeSet::new();
    for &((c, r), _) in pools {
        let (x0, y0) = (c * CELL as i32 - 34, r * CELL as i32 - 34);
        for y in (y0..y0 + 68).step_by(2) {
            for x in (x0..x0 + 68).step_by(2) {
                if !done.insert((x, y)) {
                    continue;
                }
                let (field, heat) = pull(x as f32 + 1.0, y as f32 + 1.0);
                let edge = 0.38 + (hash3(x >> 2, y >> 2, 5) - 0.5) * 0.12;
                if field < edge {
                    continue;
                }
                let lit = pull(x as f32 - 3.0, y as f32 - 3.0).0 < field - 0.12;
                canvas.fill_rect(x, y, 2, 2, pool_rule(x, y, field < edge + 0.08, field < edge + 0.16, lit, heat, time));
            }
        }
    }
}

/// How many refreshes a cached tile waits between bakes: each refresh
/// bakes the tiles whose turn it is, a quarter of those on screen, so the
/// work is spread evenly and a tile is never more than a few frames old.
const STAGGER: u64 = 4;

/// A clock that moved further than this since the last refresh has
/// jumped: every tile is baked again.
const JUMP_SECONDS: f32 = 0.1;

/// The strength the lava's light on the ground is baked at
/// (`LavaPictures::ground_light`): a surge's, the most it is drawn at, so
/// a calmer frame draws the bake fainter.
pub const GROUND_LIGHT_BAKED: f32 = 0.32;

/// The lava's ground light: its radius, colour and strength by day with
/// the lava calm (`Game::draw_lava_ground_light`, the bake).
pub const GROUND_LIGHT_RADIUS: f32 = 44.0;
pub const GROUND_LIGHT_COLOR: Color = Color::new(255, 140, 50, 255);
pub const GROUND_LIGHT_STRENGTH: f32 = 0.16;

/// The lava's picture kept between frames, so a frame draws a quad a cell
/// rather than a rect a block (docs/volcano.md, "Drawing it cheaply").
/// Nothing in it is worked out per frame that does not change: each lava
/// cell's blocks keep what is fixed about them (`BlockGeo`), and the
/// cells are baked a few at a time (`STAGGER`) into a tile atlas of the
/// lava and one of the blocks bright enough to shine; the bombs' pools
/// keep their shape until a pool comes or goes and are recoloured the same
/// way; the light the lava throws on the ground by day is baked once; each
/// crater's molten pixels are baked in turn. `refresh_pictures` brings it
/// up to a frame, and a painter draws from it only on the frame it was
/// brought up to (`fresh`) - anything else, a thumbnail or a test, draws
/// block by block as before, the same picture.
#[derive(Clone, Default)]
pub struct LavaPictures {
    /// The clock and look the pictures were last brought up to.
    at: Option<(f32, Look)>,
    tick: u64,
    /// Each lava cell and its blocks' fixed part (`None` off the lava),
    /// in `cells()` order; made with the first refresh.
    geo: Vec<((i32, i32), Vec<Option<BlockGeo>>)>,
    /// The lava, and the blocks of it molten enough to shine.
    pub lava: TileAtlas,
    pub glow: TileAtlas,
    pools: PoolPicture,
    /// The lava's light on the ground by day at `GROUND_LIGHT_BAKED`, for
    /// the glow steps it was baked in.
    ground: Option<(u32, TileAtlas)>,
    /// Each crater's molten pixels, every one and the bright ones, by
    /// crater cell.
    cones: Vec<((i32, i32), BlockImage, BlockImage)>,
}

/// What a frame brings the pictures up to (`LavaLayout::refresh_pictures`).
pub struct PictureFrame<'a> {
    pub time: f32,
    pub look: Look,
    /// The world rectangles the window shows; a tile outside all of them
    /// waits its turn. Empty: every tile is on screen.
    pub views: &'a [Rectangle],
    /// Every pool of lava a bomb left: its cell and heat (`pool_heat`).
    pub pools: &'a [((i32, i32), f32)],
    /// Every crater: its cell, its cone's picture and its phase.
    pub cones: &'a [((i32, i32), std::sync::Arc<crate::volcano::ConePicture>, crate::volcano::Phase)],
    /// The glow steps the ground light is drawn in (`glow_bands`).
    pub bands: u32,
}

impl LavaPictures {
    /// Whether the pictures were brought up to `time` - the frame a
    /// painter may draw them on.
    pub fn fresh(&self, time: f32) -> bool {
        self.at.is_some_and(|(at, _)| at == time)
    }

    /// The bombs' pools as tiles.
    pub fn pools(&self) -> &TileAtlas {
        &self.pools.atlas
    }

    /// The lava's light on the ground, baked at `GROUND_LIGHT_BAKED`.
    pub fn ground_light(&self) -> Option<&TileAtlas> {
        self.ground.as_ref().map(|(_, atlas)| atlas)
    }

    /// Crater `cell`'s molten pixels and the bright ones, if baked.
    pub fn cone(&self, cell: (i32, i32)) -> Option<(&BlockImage, &BlockImage)> {
        self.cones.iter().find(|(at, ..)| *at == cell).map(|(_, body, glow)| (body, glow))
    }

    /// Every image held, in a fixed order, for the GPU copies.
    pub fn images(&self) -> Vec<&BlockImage> {
        let mut out = vec![&self.lava.image, &self.glow.image, &self.pools.atlas.image];
        if let Some((_, atlas)) = &self.ground {
            out.push(&atlas.image);
        }
        for (_, body, glow) in &self.cones {
            out.push(body);
            out.push(glow);
        }
        out
    }
}

/// Whether cell `cell` lies on one of `views` (all of them when empty).
fn on_view(views: &[Rectangle], cell: (i32, i32)) -> bool {
    let at = crate::map::cell_to_world(cell.0, cell.1);
    views.is_empty()
        || views.iter().any(|v| at.x >= v.x - CELL && at.x <= v.x + v.width + CELL && at.y >= v.y - CELL && at.y <= v.y + v.height + CELL)
}

impl LavaLayout {
    /// The pictures as last brought up (`refresh_pictures`).
    pub fn pictures(&self) -> std::cell::Ref<'_, LavaPictures> {
        self.pictures.borrow()
    }

    /// Bring the kept pictures up to `frame`: on its first call every tile
    /// is baked; after that, on each new clock reading, the lava cells and
    /// pool tiles on screen whose turn it is (`STAGGER`), every pool tile
    /// a pool coming or going changed, and the craters in turn. Nothing on
    /// a clock it was already brought up to. Call before the frame's
    /// textures are uploaded, so nothing changes while it is drawn.
    pub fn refresh_pictures(&self, frame: &PictureFrame) {
        let mut guard = self.pictures.borrow_mut();
        let p = &mut *guard;
        if p.at == Some((frame.time, frame.look)) {
            return;
        }
        // The first refresh, and any after the clock jumped - a new round,
        // a dev server's step of many frames - bakes every tile, so no
        // picture is ever more than `STAGGER` frames of play old.
        let first = p.at.is_none_or(|(at, _)| (frame.time - at).abs() > JUMP_SECONDS);
        p.tick = p.tick.wrapping_add(1);
        let tick = p.tick;
        if p.geo.is_empty() && !self.is_empty() {
            p.geo = self
                .cells()
                .map(|(c, r)| {
                    let blocks = (0..TILE_BLOCKS * TILE_BLOCKS)
                        .map(|i| block_geo(self, c, r, (i % TILE_BLOCKS) as i32 * 2 + 1, (i / TILE_BLOCKS) as i32 * 2 + 1))
                        .collect();
                    ((c, r), blocks)
                })
                .collect();
        }
        let LavaPictures { geo, lava, glow, .. } = p;
        for (k, ((c, r), blocks)) in geo.iter().enumerate() {
            if !first && ((k as u64 + tick) % STAGGER != 0 || !on_view(frame.views, (*c, *r))) {
                continue;
            }
            let mut colors = [None; TILE_BLOCKS * TILE_BLOCKS];
            for (i, block) in blocks.iter().enumerate() {
                if let Some(g) = block {
                    let (u, v) = ((i % TILE_BLOCKS) as i32 * 2 + 1, (i / TILE_BLOCKS) as i32 * 2 + 1);
                    colors[i] = Some(block_color(g, *c, *r, u, v, frame.time, frame.look));
                }
            }
            lava.put((*c, *r), |u, v| colors[v * TILE_BLOCKS + u].map(|(color, _)| color));
            glow.put((*c, *r), |u, v| colors[v * TILE_BLOCKS + u].and_then(|(color, bright)| bright.then_some(color)));
        }
        lava.commit();
        glow.commit();
        self.refresh_pools(&mut p.pools, frame, tick, first);
        if !self.is_empty() && p.ground.as_ref().is_none_or(|(bands, _)| *bands != frame.bands) {
            p.ground = Some((frame.bands, self.bake_ground_light(frame.bands)));
        }
        p.cones.retain(|(cell, ..)| frame.cones.iter().any(|(at, ..)| at == cell));
        for (k, (cell, picture, phase)) in frame.cones.iter().enumerate() {
            let held = p.cones.iter().position(|(at, ..)| at == cell);
            if held.is_some() && (k as u64 + tick) % STAGGER != 0 {
                continue;
            }
            let i = held.unwrap_or_else(|| {
                p.cones.push((*cell, BlockImage::default(), BlockImage::default()));
                p.cones.len() - 1
            });
            let (_, body, glow) = &mut p.cones[i];
            crate::volcano::bake_molten(picture, phase, frame.time, body, glow);
        }
        p.at = Some((frame.time, frame.look));
    }

    /// The lava's light on the ground by day, every other cell a stepped
    /// glow (`pyro::glow`) at `GROUND_LIGHT_BAKED`, added up the way the
    /// additive blend adds them, as tiles.
    fn bake_ground_light(&self, bands: u32) -> TileAtlas {
        struct Sum(std::collections::BTreeMap<(i32, i32), [f32; 3]>);
        impl crate::pyro::Blocks for Sum {
            fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Color) {
                let k = color.a as f32 / 255.0;
                for by in (y..y + height).step_by(2) {
                    for bx in (x..x + width).step_by(2) {
                        let at = self.0.entry((bx.div_euclid(2), by.div_euclid(2))).or_insert([0.0; 3]);
                        at[0] += color.r as f32 * k;
                        at[1] += color.g as f32 * k;
                        at[2] += color.b as f32 * k;
                    }
                }
            }
        }
        let mut sum = Sum(Default::default());
        let color = Color::new(GROUND_LIGHT_COLOR.r, GROUND_LIGHT_COLOR.g, GROUND_LIGHT_COLOR.b, (255.0 * GROUND_LIGHT_BAKED) as u8);
        for (col, row) in self.cells().filter(|(c, r)| (c + r) % 2 == 0) {
            crate::pyro::glow(&mut sum, crate::map::cell_to_world(col, row), GROUND_LIGHT_RADIUS, color, bands);
        }
        let half = TILE_BLOCKS as i32 / 2;
        let mut tiles: std::collections::BTreeMap<(i32, i32), [Option<Color>; TILE_BLOCKS * TILE_BLOCKS]> = Default::default();
        for (&(bx, by), rgb) in &sum.0 {
            // Block (bx, by) covers field px (2 bx, 2 by); cell c spans
            // blocks 16 c - 8 .. 16 c + 8.
            let (c, r) = ((bx + half).div_euclid(TILE_BLOCKS as i32), (by + half).div_euclid(TILE_BLOCKS as i32));
            let (u, v) = ((bx + half).rem_euclid(TILE_BLOCKS as i32) as usize, (by + half).rem_euclid(TILE_BLOCKS as i32) as usize);
            let texel = Color::new(rgb[0].round().min(255.0) as u8, rgb[1].round().min(255.0) as u8, rgb[2].round().min(255.0) as u8, 255);
            tiles.entry((c, r)).or_insert([None; TILE_BLOCKS * TILE_BLOCKS])[v * TILE_BLOCKS + u] = Some(texel);
        }
        let mut atlas = TileAtlas::default();
        for (cell, texels) in &tiles {
            atlas.put(*cell, |u, v| texels[v * TILE_BLOCKS + u]);
        }
        atlas.commit();
        atlas
    }

    /// Bring the pools' tiles up to `frame`: the tiles round every pool
    /// that came or went take their shape again and bake now; every other
    /// tile on screen recolours in its turn.
    fn refresh_pools(&self, pools: &mut PoolPicture, frame: &PictureFrame, tick: u64, first: bool) {
        let mut cells: Vec<(i32, i32)> = frame.pools.iter().map(|(cell, _)| *cell).collect();
        cells.sort_unstable();
        cells.dedup();
        let mut reshaped: std::collections::BTreeSet<(i32, i32)> = Default::default();
        if cells != pools.cells {
            let changed = cells.iter().filter(|c| pools.cells.binary_search(c).is_err()).chain(pools.cells.iter().filter(|c| cells.binary_search(c).is_err()));
            for &(c, r) in changed {
                for dr in -1..=1 {
                    for dc in -1..=1 {
                        reshaped.insert((c + dc, r + dr));
                    }
                }
            }
            let mut here = vec![false; self.cols * self.rows];
            for &(c, r) in &cells {
                if let Some(i) = self.index(c, r) {
                    here[i] = true;
                }
            }
            for &tile in &reshaped {
                let shape = self.pool_tile(tile, &here);
                if shape.blocks.is_empty() {
                    pools.tiles.remove(&tile);
                    pools.atlas.remove(tile);
                } else {
                    pools.tiles.insert(tile, shape);
                }
            }
            pools.cells = cells;
        }
        if pools.tiles.is_empty() {
            pools.atlas.commit();
            return;
        }
        let mut heat = vec![0.0f32; self.cols * self.rows];
        for &((c, r), h) in frame.pools {
            if let Some(i) = self.index(c, r) {
                heat[i] = h;
            }
        }
        let PoolPicture { tiles, atlas, .. } = pools;
        for (k, (tile, shape)) in tiles.iter().enumerate() {
            let due = first || reshaped.contains(tile) || ((k as u64 + tick) % STAGGER == 0 && on_view(frame.views, *tile));
            if !due {
                continue;
            }
            let mut colors = [None; TILE_BLOCKS * TILE_BLOCKS];
            for b in &shape.blocks {
                let near = &shape.near[b.from as usize..b.from as usize + b.count as usize];
                colors[b.v as usize * TILE_BLOCKS + b.u as usize] = Some(pool_color(b, near, &heat, frame.time));
            }
            atlas.put(*tile, |u, v| colors[v * TILE_BLOCKS + u]);
        }
        atlas.commit();
    }

    /// The fixed part of the pools' picture over tile `tile` with pools on
    /// the cells `here` marks: the blocks the metaball field covers, each
    /// with the pools it draws its heat from.
    fn pool_tile(&self, tile: (i32, i32), here: &[bool]) -> PoolTile {
        let mut out = PoolTile::default();
        let pull = |x: f32, y: f32, near: Option<&mut Vec<(u32, f32)>>| -> f32 {
            let (cc, cr) = ((x / CELL).round() as i32, (y / CELL).round() as i32);
            let mut field = 0.0;
            let mut near = near;
            for r in cr - 1..=cr + 1 {
                for c in cc - 1..=cc + 1 {
                    let Some(i) = self.index(c, r).filter(|&i| here[i]) else { continue };
                    let (dx, dy) = (x - c as f32 * CELL, y - r as f32 * CELL);
                    let k = 1.0 - (dx * dx + dy * dy) / (POOL_REACH * POOL_REACH);
                    if k > 0.0 {
                        field += k * k;
                        if let Some(near) = near.as_deref_mut() {
                            near.push((i as u32, (k * 2.2).min(1.0)));
                        }
                    }
                }
            }
            field
        };
        let (ox, oy) = TileAtlas::origin(tile);
        for v in 0..TILE_BLOCKS {
            for u in 0..TILE_BLOCKS {
                let (x, y) = (ox + u as i32 * 2, oy + v as i32 * 2);
                let from = out.near.len();
                let field = pull(x as f32 + 1.0, y as f32 + 1.0, Some(&mut out.near));
                let edge = 0.38 + (hash3(x >> 2, y >> 2, 5) - 0.5) * 0.12;
                if field < edge {
                    out.near.truncate(from);
                    continue;
                }
                let lit = pull(x as f32 - 3.0, y as f32 - 3.0, None) < field - 0.12;
                out.blocks.push(PoolBlock {
                    u: u as u8,
                    v: v as u8,
                    x,
                    y,
                    rim: field < edge + 0.08,
                    inner: field < edge + 0.16,
                    lit,
                    from: from as u32,
                    count: (out.near.len() - from) as u8,
                });
            }
        }
        out
    }
}

/// How far a pool cell's pull reaches (px).
const POOL_REACH: f32 = 34.0;

/// The pools' picture kept between frames (`LavaPictures`).
#[derive(Clone, Default)]
struct PoolPicture {
    /// The pool cells the tiles' shapes are of, in order.
    cells: Vec<(i32, i32)>,
    tiles: std::collections::BTreeMap<(i32, i32), PoolTile>,
    atlas: TileAtlas,
}

/// The blocks of one tile the pools' field covers, and the pools each
/// draws its heat from: (layout cell index, weight) runs, `PoolBlock::from`
/// `count` long.
#[derive(Clone, Default, Debug, PartialEq)]
struct PoolTile {
    blocks: Vec<PoolBlock>,
    near: Vec<(u32, f32)>,
}

/// What never changes about one block of a pool (`draw_pools`'s rules).
#[derive(Clone, Copy, Debug, PartialEq)]
struct PoolBlock {
    u: u8,
    v: u8,
    x: i32,
    y: i32,
    /// At the field's edge: the rim.
    rim: bool,
    /// Just inside it.
    inner: bool,
    /// Facing the light.
    lit: bool,
    from: u32,
    count: u8,
}

/// The colour of pool block `b` at `time`, its heat the hottest of the
/// pools in `near` it lies within (`heat` by layout cell): `draw_pools`'s
/// rule.
fn pool_color(b: &PoolBlock, near: &[(u32, f32)], heat: &[f32], time: f32) -> Color {
    let (x, y) = (b.x, b.y);
    let mut h: f32 = 0.0;
    for &(i, w) in near {
        h = h.max(heat[i as usize] * w);
    }
    pool_rule(x, y, b.rim, b.inner, b.lit, h, time)
}

/// The colour of a pool block at (`x`, `y`) by where it lies in the field
/// (`rim`, `inner`, `lit`) and its `heat` at `time`.
fn pool_rule(x: i32, y: i32, rim: bool, inner: bool, lit: bool, heat: f32, time: f32) -> Color {
    if rim {
        if heat > 0.55 { FIRE[1] } else { SMOKE[0] }
    } else if heat < 0.42 && hash3(x >> 1, y >> 1, 7) > heat * 2.1 {
        if hash3(x, y, 3) < 0.2 { SMOKE[2] } else { SMOKE[1] }
    } else {
        let bub = hash3(x >> 1, y >> 1, (time * 2.5 + hash3(x, y, 2) * 4.0).floor() as i32);
        if inner && !lit {
            if heat >= 0.55 { FIRE[2] } else { FIRE[1] }
        } else if heat >= 0.55 {
            if lit {
                if bub > 0.6 { FIRE[5] } else { FIRE[4] }
            } else if bub > 0.95 {
                FIRE[6]
            } else if bub > 0.8 {
                FIRE[4]
            } else if bub < 0.12 {
                FIRE[2]
            } else {
                FIRE[3]
            }
        } else if heat >= 0.28 {
            if lit {
                FIRE[3]
            } else if bub > 0.9 {
                FIRE[4]
            } else if bub < 0.3 {
                FIRE[1]
            } else {
                FIRE[2]
            }
        } else if bub > 0.85 {
            FIRE[2]
        } else if bub < 0.4 {
            FIRE[0]
        } else {
            FIRE[1]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::cell_to_world;

    fn cells(list: &[(i32, i32)]) -> Vec<Position> {
        list.iter().map(|&(c, r)| cell_to_world(c, r)).collect()
    }

    #[test]
    fn a_stream_is_a_ford_and_a_wide_block_is_deep_in_its_middle() {
        let mut list: Vec<(i32, i32)> = (2..12).map(|c| (c, 3)).collect();
        for r in 8..11 {
            for c in 8..11 {
                list.push((c, r));
            }
        }
        let lava = LavaLayout::build(640.0, 480.0, &[], &cells(&list), &[], 0.55);
        assert_eq!(lava.depth_at(cell_to_world(5, 3)), Depth::Shallow);
        assert_eq!(lava.depth_at(cell_to_world(9, 9)), Depth::Deep);
        assert_eq!(lava.depth_at(cell_to_world(5, 6)), Depth::Dry);
    }

    #[test]
    fn lava_flows_away_from_the_cone_it_leaves() {
        // A cone at (5, 5) reaches (7, 5); the stream leaves it eastward
        // and turns south.
        let list = [(8, 5), (9, 5), (10, 5), (10, 6), (10, 7)];
        let lava = LavaLayout::build(640.0, 480.0, &[], &cells(&list), &[(5, 5)], 0.55);
        assert_eq!(lava.along(8, 5), 0.0);
        assert_eq!(lava.along(10, 7), 4.0);
        assert!(lava.flow(9, 5).x > 0.9, "east along the run: {:?}", lava.flow(9, 5));
        assert!(lava.flow(10, 7).y > 0.9, "south at the end: {:?}", lava.flow(10, 7));
        let corner = lava.flow(10, 5);
        assert!(corner.x > 0.5 && corner.y > 0.5, "the bend turns: {corner:?}");
    }

    #[test]
    fn a_run_cut_off_by_a_crossing_flows_on_away_from_the_cone() {
        // A cone at (5, 5); a stream leaves it east along row 5, a road
        // crosses it at (11, 5), and the far part runs on east then turns
        // north - away from the cone, though its northernmost cell is its
        // last.
        let list = [(8, 5), (9, 5), (10, 5), (12, 5), (13, 5), (14, 5), (14, 4), (14, 3)];
        let lava = LavaLayout::build(640.0, 480.0, &[], &cells(&list), &[(5, 5)], 0.55);
        assert_eq!(lava.along(12, 5), 0.0, "the cut-off part starts at the crossing");
        assert!(lava.flow(13, 5).x > 0.9, "and flows on east: {:?}", lava.flow(13, 5));
        assert!(lava.flow(14, 3).y < -0.9, "then north: {:?}", lava.flow(14, 3));
    }

    #[test]
    fn a_run_no_cone_feeds_flows_down_the_map() {
        let list = [(4, 2), (4, 3), (4, 4)];
        let lava = LavaLayout::build(640.0, 480.0, &[], &cells(&list), &[], 0.55);
        assert!(lava.flow(4, 3).y > 0.9);
    }

    #[test]
    fn heat_falls_off_a_cell_at_a_time() {
        let list = [(10, 5), (10, 6), (10, 7)];
        let lava = LavaLayout::build(640.0, 480.0, &[], &cells(&list), &[], 0.5);
        assert_eq!(lava.heat(10, 6), 1.0);
        assert_eq!(lava.heat(11, 6), 0.5);
        assert_eq!(lava.heat(12, 6), 0.25);
        assert_eq!(lava.heat(13, 6), 0.125);
        assert_eq!(lava.heat(14, 6), 0.0625);
        assert_eq!(lava.heat(15, 6), 0.0, "past the reach");
        assert_eq!(lava.heat(2, 2), 0.0);
    }

    #[test]
    fn a_map_without_lava_is_empty_and_cold() {
        let lava = LavaLayout::build(640.0, 480.0, &[], &[], &[], 0.55);
        assert!(lava.is_empty());
        assert_eq!(lava.heat(5, 5), 0.0);
        assert_eq!(lava.depth_at(cell_to_world(5, 5)), Depth::Dry);
    }

    #[test]
    fn a_stream_cell_draws_banks_where_its_sides_are_closed() {
        let list = [(4, 2), (4, 3), (4, 4)];
        let lava = LavaLayout::build(640.0, 480.0, &[], &cells(&list), &[], 0.55);
        let look = Look { surge: 0.0, speed: 10.0 };
        // The west edge of the middle cell is a bank of crust, its middle
        // molten, and nothing outside the lava.
        let (edge, _) = block(&lava, 4, 3, 1, 15, 0.0, look).expect("bank");
        assert!(CRUST.contains(&edge));
        let (middle, _) = block(&lava, 4, 3, 15, 15, 0.0, look).expect("body");
        assert!(MOLTEN.contains(&middle));
        assert!(block(&lava, 5, 3, 15, 15, 0.0, look).is_none());
    }

    #[test]
    fn the_same_time_draws_the_same_lava() {
        let list = [(4, 2), (4, 3), (4, 4)];
        let lava = LavaLayout::build(640.0, 480.0, &[], &cells(&list), &[], 0.55);
        let look = Look { surge: 0.3, speed: 10.0 };
        let mut a = crate::canvas::CpuCanvas::blank(320, 320);
        let mut b = crate::canvas::CpuCanvas::blank(320, 320);
        for canvas in [&mut a, &mut b] {
            for r in 2..=4 {
                draw_cell(canvas, &lava, 4, r, 1.25, look, false, 1.0);
                draw_floes(canvas, &lava, 4, r, 1.25, look);
            }
        }
        assert!(a.pixels() == b.pixels());
        assert!(a.pixels().iter().any(|p| p.a > 0));
    }

    /// A river, a lake beside it and a cone at its head: every kind of
    /// block the lava draws.
    fn mixed_layout() -> LavaLayout {
        let mut list = vec![(3, 2), (3, 3), (3, 4), (4, 4), (5, 4), (6, 4), (7, 4)];
        for r in 5..=8 {
            for c in 6..=9 {
                list.push((c, r));
            }
        }
        LavaLayout::build(480.0, 352.0, &[], &cells(&list), &[(3, 0)], 0.55)
    }

    fn frame<'a>(time: f32, look: Look, views: &'a [Rectangle], pools: &'a [((i32, i32), f32)]) -> PictureFrame<'a> {
        PictureFrame { time, look, views, pools, cones: &[], bands: 5 }
    }

    /// The kept picture, its first refresh baking every tile, draws the
    /// lava and its bright blocks exactly as block-by-block drawing does
    /// at that clock, the surge's narrower banks included.
    #[test]
    fn the_kept_lava_is_the_lava_drawn_block_by_block() {
        let lava = mixed_layout();
        for (time, surge) in [(0.0, 0.0), (2.37, 0.0), (7.9, 0.8)] {
            let look = Look { surge, speed: 12.0 };
            let fresh = mixed_layout();
            fresh.refresh_pictures(&frame(time, look, &[], &[]));
            let pictures = fresh.pictures();
            assert!(pictures.fresh(time));
            for bright in [false, true] {
                let mut kept = crate::canvas::CpuCanvas::blank(480, 352);
                let mut drawn = crate::canvas::CpuCanvas::blank(480, 352);
                if bright { pictures.glow.draw(&mut kept) } else { pictures.lava.draw(&mut kept) }
                for (c, r) in lava.cells() {
                    draw_cell(&mut drawn, &lava, c, r, time, look, bright, 1.0);
                }
                assert!(kept.pixels() == drawn.pixels(), "time {time}, bright {bright}");
            }
        }
    }

    /// After the first refresh a new clock bakes only the cells on screen
    /// whose turn it is - a cell off screen keeps its tile - and within
    /// `STAGGER` refreshes every cell on screen has been baked again.
    #[test]
    fn a_refresh_bakes_the_cells_on_screen_in_turn() {
        let lava = mixed_layout();
        let look = Look { surge: 0.0, speed: 12.0 };
        lava.refresh_pictures(&frame(0.0, look, &[], &[]));
        let before = lava.pictures().lava.image.clone();
        let tile = |image: &BlockImage, slot: (usize, usize, usize, usize)| -> Vec<Color> {
            (slot.1..slot.1 + slot.3).flat_map(|y| image.texels[y * image.width + slot.0..y * image.width + slot.0 + slot.2].to_vec()).collect()
        };
        // On screen: the river's head only.
        let view = [Rectangle::new(64.0, 32.0, 64.0, 128.0)];
        let mut baked = std::collections::BTreeSet::new();
        for k in 1..=STAGGER {
            let time = k as f32 / 60.0;
            lava.refresh_pictures(&frame(time, look, &view, &[]));
            let pictures = lava.pictures();
            for ((x, y), slot) in pictures.lava.tiles() {
                if tile(&before, slot) != tile(&pictures.lava.image, slot) {
                    baked.insert((x, y));
                }
            }
        }
        let on_screen: Vec<(i32, i32)> = lava.cells().filter(|&cell| on_view(&view, cell)).map(TileAtlas::origin).collect();
        assert!(!on_screen.is_empty());
        for at in &on_screen {
            assert!(baked.contains(at), "{at:?} on screen was never baked again");
        }
        assert!(baked.iter().all(|at| on_screen.contains(at)), "a cell off screen was baked: {baked:?}");
        // A clock that jumps - a dev server's long step - bakes every cell.
        let before = lava.pictures().lava.image.clone();
        lava.refresh_pictures(&frame(9.0, look, &view, &[]));
        let pictures = lava.pictures();
        let changed = pictures.lava.tiles().filter(|(_, slot)| tile(&before, *slot) != tile(&pictures.lava.image, *slot)).count();
        assert!(changed > on_screen.len(), "only {changed} cells baked after a jump");
    }

    /// The pools kept as tiles draw what `draw_pools` draws, as the bombs'
    /// splashes come and burn out - each change reshaping only the tiles
    /// round it.
    #[test]
    fn the_kept_pools_are_the_pools_drawn_block_by_block() {
        let lava = mixed_layout();
        let look = Look { surge: 0.0, speed: 12.0 };
        let sets: [&[((i32, i32), f32)]; 4] = [
            &[((10, 2), 1.0), ((11, 2), 0.9), ((10, 3), 0.7)],
            &[((10, 2), 1.0), ((11, 2), 0.9), ((10, 3), 0.7), ((4, 8), 0.5), ((5, 8), 0.3)],
            &[((11, 2), 0.6), ((10, 3), 0.2), ((4, 8), 0.45), ((5, 8), 0.25)],
            &[((4, 8), 0.1)],
        ];
        for (k, pools) in sets.iter().enumerate() {
            let time = 1.0 + k as f32 * 0.7;
            lava.refresh_pictures(&frame(time, look, &[], pools));
            // A picture made with every pool new bakes every tile: the
            // pools as `draw_pools` draws them.
            let fresh = mixed_layout();
            fresh.refresh_pictures(&frame(time, look, &[], pools));
            let mut kept = crate::canvas::CpuCanvas::blank(480, 352);
            let mut drawn = crate::canvas::CpuCanvas::blank(480, 352);
            fresh.pictures().pools().draw(&mut kept);
            draw_pools(&mut drawn, pools, time);
            assert!(kept.pixels() == drawn.pixels(), "pool set {k}");
            // The picture kept through the changes has every tile's shape
            // the fresh one has, the tiles round each change made again.
            assert!(lava.pictures().pools.tiles == fresh.pictures().pools.tiles, "pool set {k}");
            assert_eq!(lava.pictures().pools().cells().collect::<Vec<_>>(), fresh.pictures().pools().cells().collect::<Vec<_>>(), "pool set {k}");
        }
        lava.refresh_pictures(&frame(9.0, look, &[], &[]));
        assert!(lava.pictures().pools().is_empty(), "every pool burnt out");
    }

    /// The cone's slopes, shadow, skirt and molten rock drawn from their
    /// baked images are the runs and pixels drawn one by one.
    #[test]
    fn the_baked_cone_is_the_cone_drawn_by_runs() {
        let t = crate::tuning::Tuning::DEFAULT;
        let picture = crate::volcano::cone(&[0.4, 2.2]);
        let centre = cell_to_world(6, 6);
        for time in [3.0, 41.5, 44.2] {
            let phase = crate::volcano::phase(time, 0.0, &t);
            let mut body = BlockImage::default();
            let mut glow = BlockImage::default();
            crate::volcano::bake_molten(&picture, &phase, time, &mut body, &mut glow);
            let mut baked = crate::canvas::CpuCanvas::blank(416, 416);
            crate::volcano::draw_skirt(&mut baked, centre, &picture);
            crate::volcano::draw_cone_shadow(&mut baked, centre, &picture, (0.7, 0.7));
            crate::volcano::draw_cone_with(&mut baked, centre, &picture, Some(&body), &phase, time);
            crate::volcano::draw_cone_glow_with(&mut baked, centre, &picture, Some(&glow), &phase, time, 1.0);
            // The runs: a canvas that has no image uploaded.
            struct Runs(crate::canvas::CpuCanvas);
            impl Canvas for Runs {
                fn blit(&mut self, s: crate::canvas::Sheet, a: Rectangle, b: Rectangle, o: Vec2, r: f32, t: Color) {
                    self.0.blit(s, a, b, o, r, t)
                }
                fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Color) {
                    self.0.fill_rect(x, y, w, h, c)
                }
                fn gradient_v(&mut self, x: i32, y: i32, w: i32, h: i32, a: Color, b: Color) {
                    self.0.gradient_v(x, y, w, h, a, b)
                }
                fn gradient_h(&mut self, x: i32, y: i32, w: i32, h: i32, a: Color, b: Color) {
                    self.0.gradient_h(x, y, w, h, a, b)
                }
                fn disc(&mut self, c: Position, r: f32, color: Color) {
                    self.0.disc(c, r, color)
                }
                fn ring(&mut self, c: Position, i: f32, o: f32, s: f32, e: f32, n: i32, color: Color) {
                    self.0.ring(c, i, o, s, e, n, color)
                }
                fn blocks(&mut self, image: &BlockImage) {
                    self.0.blocks(image)
                }
                fn blocks_part(&mut self, image: &BlockImage, src: (usize, usize, usize, usize), at: (i32, i32)) {
                    self.0.blocks_part(image, src, at)
                }
                fn has_blocks(&self, _stamp: u64) -> bool {
                    false
                }
            }
            let mut runs = Runs(crate::canvas::CpuCanvas::blank(416, 416));
            crate::volcano::draw_skirt(&mut runs, centre, &picture);
            crate::volcano::draw_cone_shadow(&mut runs, centre, &picture, (0.7, 0.7));
            crate::volcano::draw_cone(&mut runs, centre, &picture, &phase, time);
            crate::volcano::draw_cone_glow(&mut runs, centre, &picture, &phase, time, 1.0);
            assert!(baked.pixels() == runs.0.pixels(), "time {time}");
        }
    }

    /// The lava's light on the ground is baked round the lava alone, and
    /// brighter the nearer a block lies to it.
    #[test]
    fn the_ground_light_is_baked_round_the_lava() {
        let lava = mixed_layout();
        lava.refresh_pictures(&frame(0.0, Look { surge: 0.0, speed: 12.0 }, &[], &[]));
        let pictures = lava.pictures();
        let light = pictures.ground_light().expect("baked with the first refresh");
        let texel = |x: i32, y: i32| -> Color {
            let cell = ((x + 16).div_euclid(32), (y + 16).div_euclid(32));
            light
                .tiles()
                .find(|(at, _)| *at == TileAtlas::origin(cell))
                .map(|(at, slot)| {
                    let (u, v) = (((x - at.0) / 2) as usize, ((y - at.1) / 2) as usize);
                    light.image.texels[(slot.1 + v) * light.image.width + slot.0 + u]
                })
                .unwrap_or(Color::new(0, 0, 0, 0))
        };
        // On a lit lava cell (every other one is), a little off it and far off.
        let on = texel(4 * 32, 4 * 32);
        let near = texel(4 * 32, 4 * 32 - 30);
        assert!(on.r > near.r && near.r > 0, "{on:?} {near:?}");
        assert_eq!(texel(13 * 32, 1 * 32).a, 0, "nothing far from the lava");
    }
}
