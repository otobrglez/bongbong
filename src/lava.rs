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

use crate::canvas::Canvas;
use crate::ground::{Depth, WaterLayout};
use crate::math::{Color, Vec2};
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
    let i = layout.index(c, r)?;
    if !layout.lava[i] {
        return None;
    }
    let open = layout.open[i];
    let diag = layout.diag[i];
    let (uf, vf) = (u as f32, v as f32);
    let bank = 7.0 - 3.0 * look.surge + if hash3(c * 32 + (u >> 2), r * 32 + (v >> 2), 7) < 0.35 { 2.0 } else { 0.0 };
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
    if d < bank {
        if d < bank * 0.55 {
            let lit = ((open & N == 0 && v < 8) || (open & W == 0 && u < 8)) && hb > 0.7;
            return Some((if hb < 0.3 { CRUST[1] } else if lit { CRUST[2] } else { CRUST[0] }, false));
        }
        return Some(if look.surge > 0.4 {
            (if hb < 0.5 { MOLTEN[2] } else { MOLTEN[3] }, hb >= 0.5)
        } else {
            (if hb < 0.6 { MOLTEN[1] } else { MOLTEN[2] }, false)
        });
    }
    let m = ((d - bank) / (16.0 - bank)).clamp(0.0, 1.0);
    if layout.lake[i] {
        return Some(lake_block(c, r, u, v, time, look, m));
    }
    let band = {
        let dir = layout.flow[i];
        let s = layout.along[i] * CELL + (uf - 16.0) * dir.x + (vf - 16.0) * dir.y;
        let lateral = (uf * dir.y - vf * dir.x) * 0.12;
        0.5 + 0.5 * crate::trig::sin((s - time * look.speed) / 13.0 * std::f32::consts::TAU + lateral)
    };
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
    Some((MOLTEN[step], step >= 3))
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
    const REACH: f32 = 34.0;
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
                let k = 1.0 - (dx * dx + dy * dy) / (REACH * REACH);
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
                let color = if field < edge + 0.08 {
                    if heat > 0.55 { FIRE[1] } else { SMOKE[0] }
                } else if heat < 0.42 && hash3(x >> 1, y >> 1, 7) > heat * 2.1 {
                    if hash3(x, y, 3) < 0.2 { SMOKE[2] } else { SMOKE[1] }
                } else {
                    let lit = pull(x as f32 - 3.0, y as f32 - 3.0).0 < field - 0.12;
                    let bub = hash3(x >> 1, y >> 1, (time * 2.5 + hash3(x, y, 2) * 4.0).floor() as i32);
                    if field < edge + 0.16 && !lit {
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
                };
                canvas.fill_rect(x, y, 2, 2, color);
            }
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
}
