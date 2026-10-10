//! A wood as a whole: what makes a stand of trees read as a forest rather
//! than a grid of crowns (docs/WOODS.md). Worked out once from the map
//! when a round starts, all of it hashed from cell positions - no RNG.
//! The one part that decides play is a trail's cover; nothing here is
//! read by the nav grid, a collider or a hit test.
//!
//! - **Crowns** (`Crown`): each tree's crown is drawn up to 4 px off its
//!   cell's centre on the 2 px grid and mirrored by hash, inside the 8 px a
//!   48 px crown already overhangs its 32 px cell. Never into a trail (it
//!   would close the way in) or a wall beside it, and always out over water
//!   beside it, the way bank trees lean.
//! - **Trails** (`is_trail`): an open cell with wood within two cells on
//!   both sides along a row or a column, among at least three more such
//!   cells round it - so a notch in a wood's edge is not one. A trail
//!   hides what stands on it, like tall grass (`Game::cover_cells`), for
//!   as long as its wood stands: felling a tree works its trails out again
//!   (`fell`). A trail one cell wide is one the AI's two-cell corridors
//!   never take (`narrow_trails`, the builder's CHECK). A trail that runs
//!   three cells or more is worn into the floor: a pair of ruts down its
//!   middle, running on two cells into the meadow and fading.
//! - **The forest floor** (`floor`): a 2 px Bayer dither under the trees,
//!   its density following the crowns near each block, baked once onto the
//!   floor like the lava's banks. It stops at the water, is thin under dry
//!   trees and thinner under a snag, and carries specks of litter.
//! - **Undergrowth** (`undergrowth`): bushes and ferns on the open cells
//!   round a wood, more the more trees a cell has beside it; a fern now and
//!   then between interior crowns; reeds on a wooded bank. Never on water,
//!   a trail or beside one, nor on a cell the map put anything on. It is
//!   picture only: `grass::GrassTuft`s a hull presses down and a fire chars
//!   like any other, but its cells are not `Game::grass_cells`, so it hides
//!   nobody - cover is what a map places.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use crate::canvas::BlockImage;
use crate::grass::{Bush, GrassTuft};
use crate::map::{cell_to_world, CellObject, MapFile, Theme};
use crate::math::Color;
use crate::obstacle::Material;
use crate::{OBSTACLE_GRID_SIZE, Position};

/// Where a tree's crown is drawn relative to its cell: the offset in px
/// (whole 2 px blocks) and whether it is mirrored.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Crown {
    pub dx: f32,
    pub dy: f32,
    pub mirror: bool,
}

/// The woods of one map. See the module comment.
#[derive(Default)]
pub struct Woods {
    trees: BTreeMap<(i32, i32), Material>,
    water: BTreeSet<(i32, i32)>,
    /// Every cell the map put anything on, trees included.
    taken: BTreeSet<(i32, i32)>,
    /// Solid cells that are not trees: walls, props, towers.
    walls: BTreeSet<(i32, i32)>,
    /// Open cells flanked by wood (`flanked`); a trail is one among
    /// enough others.
    candidates: BTreeSet<(i32, i32)>,
    trails: BTreeSet<(i32, i32)>,
    crowns: BTreeMap<(i32, i32), Crown>,
    theme: Theme,
    /// The field in px and in cells.
    field: (f32, f32),
    cells: (i32, i32),
    floor: OnceLock<BlockImage>,
}

/// A 32-bit hash of a cell and a salt - `blast::seed_at` on its centre.
fn hash(cell: (i32, i32), salt: u32) -> u32 {
    crate::blast::seed_at(cell_to_world(cell.0, cell.1), salt)
}

const ORTHOGONAL: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

/// How many trail cells in a straight line a trail needs before it is worn
/// into a track (`Woods::floor`).
const TRACK_RUN: usize = 3;

impl Woods {
    /// The woods of `map`: its trees, water and trails, and every crown.
    pub fn build(map: &MapFile) -> Woods {
        let (w, h) = map.field_size();
        let cells = ((w / OBSTACLE_GRID_SIZE).round() as i32, (h / OBSTACLE_GRID_SIZE).round() as i32);
        let mut woods = Woods { theme: map.theme, field: (w, h), cells, ..Woods::default() };
        for (c, r, obj) in map.iter_cells() {
            woods.taken.insert((c, r));
            match obj.material() {
                Some(m) if m.is_tree() => {
                    woods.trees.insert((c, r), m);
                }
                Some(_) => {
                    woods.walls.insert((c, r));
                }
                None if *obj == CellObject::Water => {
                    woods.water.insert((c, r));
                }
                None => {}
            }
        }
        if woods.trees.is_empty() {
            return woods;
        }
        woods.candidates = woods.open_cells().filter(|&cell| woods.flanked(cell)).collect();
        woods.trails = woods.candidates.iter().copied().filter(|&cell| woods.is_trail_among_candidates(cell)).collect();
        let crowns: BTreeMap<(i32, i32), Crown> = woods.trees.keys().map(|&cell| (cell, woods.place_crown(cell))).collect();
        woods.crowns = crowns;
        woods
    }

    /// No trees at all.
    pub fn is_empty(&self) -> bool {
        self.trees.is_empty()
    }

    /// Where the tree at `cell` draws its crown; centred and unmirrored for
    /// a cell holding no tree.
    pub fn crown(&self, cell: (i32, i32)) -> Crown {
        self.crowns.get(&cell).copied().unwrap_or_default()
    }

    /// Whether `cell` is a trail through a wood (see the module comment).
    pub fn is_trail(&self, cell: (i32, i32)) -> bool {
        self.trails.contains(&cell)
    }

    /// A candidate with at least three more candidates in its 5 x 5 block.
    fn is_trail_among_candidates(&self, (c, r): (i32, i32)) -> bool {
        self.candidates.contains(&(c, r))
            && (-2..=2).flat_map(|dy| (-2..=2).map(move |dx| (c + dx, r + dy))).filter(|n| self.candidates.contains(n)).count() >= 4
    }

    /// The tree at `cell` is gone: the trails it flanked are worked out
    /// again. Only cells near it can change - a cell's flanks reach two
    /// cells and a trail counts the candidates two cells round it - so
    /// only those are. The crowns, the floor and the undergrowth stay as
    /// the round began.
    pub fn fell(&mut self, (c, r): (i32, i32)) {
        if self.trees.remove(&(c, r)).is_none() {
            return;
        }
        // The cell itself is open ground now, like any felled tree's.
        self.taken.remove(&(c, r));
        let near = |d: i32| (-d..=d).flat_map(move |dy| (-d..=d).map(move |dx| (c + dx, r + dy)));
        for cell in near(3) {
            if self.open(cell) && self.flanked(cell) {
                self.candidates.insert(cell);
            } else {
                self.candidates.remove(&cell);
            }
        }
        for cell in near(5) {
            if self.is_trail_among_candidates(cell) {
                self.trails.insert(cell);
            } else {
                self.trails.remove(&cell);
            }
        }
    }

    /// The centres of the trail cells, in cell order: cover like tall grass.
    pub fn trail_positions(&self) -> impl Iterator<Item = Position> + '_ {
        let mut cells: Vec<(i32, i32)> = self.trails.iter().copied().collect();
        cells.sort_by_key(|&(c, r)| (r, c));
        cells.into_iter().map(|(c, r)| cell_to_world(c, r))
    }

    /// The trail cells one cell wide - something solid on both sides across
    /// the trail - in cell order: a way through a hull can take but the AI,
    /// which plans on corridors two cells wide, never will.
    pub fn narrow_trails(&self) -> Vec<(i32, i32)> {
        let mut out: Vec<(i32, i32)> = self
            .trails
            .iter()
            .copied()
            .filter(|&(c, r)| {
                let shut = |cell: (i32, i32)| !self.open(cell);
                (shut((c - 1, r)) && shut((c + 1, r))) || (shut((c, r - 1)) && shut((c, r + 1)))
            })
            .collect();
        out.sort_by_key(|&(c, r)| (r, c));
        out
    }

    fn tree(&self, cell: (i32, i32)) -> bool {
        self.trees.contains_key(&cell)
    }

    /// Inside the field, holding nothing.
    fn open(&self, (c, r): (i32, i32)) -> bool {
        c >= 0 && r >= 0 && c < self.cells.0 && r < self.cells.1 && !self.taken.contains(&(c, r))
    }

    fn open_cells(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        (0..self.cells.1).flat_map(move |r| (0..self.cells.0).map(move |c| (c, r))).filter(|&cell| self.open(cell))
    }

    /// An open cell with a tree among its eight neighbours twice over and
    /// wood within two cells on both sides along a row or a column, open
    /// ground between.
    fn flanked(&self, (c, r): (i32, i32)) -> bool {
        let around = (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (c + dx, r + dy))).filter(|&n| self.tree(n)).count();
        if around < 2 {
            return false;
        }
        let side = |dx: i32, dy: i32| {
            for k in 1..=2 {
                let n = (c + dx * k, r + dy * k);
                if self.tree(n) {
                    return true;
                }
                if !self.open(n) {
                    return false;
                }
            }
            false
        };
        (side(1, 0) && side(-1, 0)) || (side(0, 1) && side(0, -1))
    }

    /// Up to 4 px off centre on the 2 px grid, then pushed away from a
    /// trail or a wall beside the tree and out over water beside it.
    fn place_crown(&self, (c, r): (i32, i32)) -> Crown {
        let h = hash((c, r), 141);
        let mut dx = ((h % 5) as i32 - 2) * 2;
        let mut dy = (((h >> 8) % 5) as i32 - 2) * 2;
        for (ox, oy) in ORTHOGONAL {
            let n = (c + ox, r + oy);
            if self.trails.contains(&n) || self.walls.contains(&n) {
                if ox != 0 {
                    dx = -ox * dx.abs().max(2);
                } else {
                    dy = -oy * dy.abs().max(2);
                }
            } else if self.water.contains(&n) {
                if ox != 0 {
                    dx = ox * 4;
                } else {
                    dy = oy * 4;
                }
            }
        }
        Crown { dx: dx as f32, dy: dy as f32, mirror: (h >> 16) & 1 == 1 }
    }

    /// How much shade a tree of `material` throws on the floor: a dry
    /// crown is thin, a palm's a few fronds, a snag's none but its limbs.
    fn shade_weight(&self, material: Material) -> f32 {
        if material == Material::Snag {
            0.15
        } else if material.is_dry(self.theme) {
            0.3
        } else if material == Material::Palm {
            0.6
        } else {
            1.0
        }
    }

    /// The forest floor, baked the first time it is drawn (so a room server
    /// never pays for it). See the module comment.
    pub fn floor(&self) -> &BlockImage {
        self.floor.get_or_init(|| self.bake_floor())
    }

    fn bake_floor(&self) -> BlockImage {
        let stamp = crate::ground::next_block_stamp();
        let (w, h) = (((self.field.0 / 2.0).ceil() as usize).max(1), ((self.field.1 / 2.0).ceil() as usize).max(1));
        let mut texels = vec![Color::new(0, 0, 0, 0); w * h];
        if self.trees.is_empty() {
            return BlockImage { width: 1, height: 1, block: 2, texels: vec![Color::new(0, 0, 0, 0)], stamp, patches: Vec::new() };
        }
        // How dense the canopy over each block is: every crown adds a cone
        // `REACH` px across, weighted by its kind.
        const REACH: f32 = 34.0;
        let mut density = vec![0.0f32; w * h];
        for (&(c, r), &material) in &self.trees {
            let crown = self.crown((c, r));
            let centre = cell_to_world(c, r);
            let (cx, cy) = (centre.x + crown.dx, centre.y + crown.dy);
            let weight = self.shade_weight(material);
            let (bx0, bx1) = (((cx - REACH) / 2.0).floor().max(0.0) as usize, (((cx + REACH) / 2.0).ceil() as usize).min(w));
            let (by0, by1) = (((cy - REACH) / 2.0).floor().max(0.0) as usize, (((cy + REACH) / 2.0).ceil() as usize).min(h));
            for by in by0..by1 {
                for bx in bx0..bx1 {
                    let (x, y) = (bx as f32 * 2.0 + 1.0, by as f32 * 2.0 + 1.0);
                    let d = ((x - cx) * (x - cx) + (y - cy) * (y - cy)).sqrt();
                    if d < REACH {
                        density[by * w + bx] += weight * (1.0 - d / REACH);
                    }
                }
            }
        }
        let (deep, light, speck_a, speck_b) = match self.theme {
            // GREEN_SHADE and GREEN_DK; WOOD_DEEPER and SAND_DK litter.
            Theme::Grass => (Color::new(0x3D, 0x6E, 0x3F, 255), Color::new(0x5F, 0x91, 0x4B, 255), Color::new(0x68, 0x47, 0x1D, 255), Color::new(0x67, 0x51, 0x2A, 255)),
            // SAND_DK and SAND_MD; WOOD_DEEPER and WOOD_ASH litter.
            Theme::Desert => (Color::new(0x67, 0x51, 0x2A, 255), Color::new(0xB7, 0xA2, 0x48, 255), Color::new(0x68, 0x47, 0x1D, 255), Color::new(0x73, 0x62, 0x4D, 255)),
        };
        for by in 0..h {
            for bx in 0..w {
                let dens = (density[by * w + bx] * 0.7).min(1.0);
                if dens <= 0.0 {
                    continue;
                }
                let cell = crate::map::world_to_cell(Position::new(bx as f32 * 2.0 + 1.0, by as f32 * 2.0 + 1.0));
                if self.water.contains(&cell) {
                    continue;
                }
                let b = crate::pyro::bayer(bx as i32, by as i32);
                let texel = if b < dens * 0.95 {
                    if b < (dens - 0.45) * 1.2 { deep } else { light }
                } else if dens > 0.4 && crate::blast::seed_at(Position::new(bx as f32, by as f32), 9) % 41 == 0 {
                    if crate::blast::seed_at(Position::new(bx as f32, by as f32), 10) % 2 == 0 { speck_a } else { speck_b }
                } else {
                    continue;
                };
                texels[by * w + bx] = texel;
            }
        }
        self.bake_tracks(&mut texels, w, h);
        BlockImage { width: w, height: h, block: 2, texels, stamp, patches: Vec::new() }
    }

    /// Which way the trail at `cell` runs: down the map where no tree
    /// stands above or below it.
    fn runs_down(&self, (c, r): (i32, i32)) -> bool {
        !(self.tree((c, r - 1)) || self.tree((c, r + 1)))
    }

    /// The worn track along every trail that runs somewhere: ruts of dark
    /// earth with trampled specks between, running on two cells into the
    /// meadow past each end and thinning out. A trail one or two cells wide
    /// carries one pair of ruts down its middle, where a tank drives it.
    fn bake_tracks(&self, texels: &mut [Color], w: usize, h: usize) {
        if self.trails.is_empty() {
            return;
        }
        // Only a trail that runs somewhere is worn: a straight run of at
        // least `TRACK_RUN` trail cells. The gaps of a sparse wood are
        // trails too, and hide a tank, but a track through each would be
        // stubs going nowhere.
        let run = |(c, r): (i32, i32), down: bool| {
            let step = |k: i32| if down { (c, r + k) } else { (c + k, r) };
            let along = |sign: i32| (1..).take_while(|&k| self.trails.contains(&step(sign * k)) && self.runs_down(step(sign * k)) == down).count();
            1 + along(-1) + along(1)
        };
        let tracked: Vec<((i32, i32), bool)> =
            self.trails.iter().map(|&cell| (cell, self.runs_down(cell))).filter(|&(cell, down)| run(cell, down) >= TRACK_RUN).collect();
        // How worn each cell is, out of 4: a trail fully, its mouth less
        // with every cell out.
        let mut worn: BTreeMap<(i32, i32), (u32, bool)> = tracked.iter().map(|&(cell, down)| (cell, (4, down))).collect();
        for &((c, r), down) in &tracked {
            for sign in [-1, 1] {
                for (k, wear) in [(1, 3), (2, 1)] {
                    let cell = if down { (c, r + sign * k) } else { (c + sign * k, r) };
                    if self.open(cell) && !self.trails.contains(&cell) {
                        let e = worn.entry(cell).or_insert((0, down));
                        e.0 = e.0.max(wear);
                    }
                }
            }
        }
        // WOOD_DEEPER ruts; SAND_DK or WOOD_ASH trampled ground.
        let rut = Color::new(0x68, 0x47, 0x1D, 255);
        let dust = if self.theme == Theme::Desert { Color::new(0x73, 0x62, 0x4D, 255) } else { Color::new(0x67, 0x51, 0x2A, 255) };
        for (&(c, r), &(wear, down)) in &worn {
            if self.water.contains(&(c, r)) {
                continue;
            }
            let wide = |dc: i32, dr: i32| worn.contains_key(&(c + dc, r + dr));
            let (before, after) = if down { (wide(-1, 0), wide(1, 0)) } else { (wide(0, -1), wide(0, 1)) };
            // One pair of ruts per trail, 16 px apart, in 2 px blocks across
            // the cell from its edge: centred in a one-cell trail, astride
            // the shared edge of a two-cell one. The middle of a wider one
            // is trampled with no ruts. `between` is the worn strip inside
            // the pair.
            let (ruts, between): (&[i32], std::ops::RangeInclusive<i32>) = match (before, after) {
                (false, false) => (&[3, 4, 11, 12], 5..=10),
                (false, true) => (&[11, 12], 13..=15),
                (true, false) => (&[3, 4], 0..=2),
                (true, true) => (&[], 0..=15),
            };
            let centre = cell_to_world(c, r);
            let (x0, y0) = (((centre.x - 16.0) / 2.0) as i32, ((centre.y - 16.0) / 2.0) as i32);
            for j in 0..16 {
                for i in 0..16 {
                    let (bx, by) = (x0 + i, y0 + j);
                    if bx < 0 || by < 0 || bx as usize >= w || by as usize >= h {
                        continue;
                    }
                    let across = if down { i } else { j };
                    let roll = crate::blast::seed_at(Position::new(bx as f32, by as f32), 21) % 5;
                    let texel = if ruts.contains(&across) && roll < wear {
                        rut
                    } else if wear == 4 && between.contains(&across) && crate::blast::seed_at(Position::new(bx as f32, by as f32), 22) % 9 == 0 {
                        dust
                    } else {
                        continue;
                    };
                    texels[by as usize * w + bx as usize] = texel;
                }
            }
        }
    }

    /// Whether the conifers have it round `cell`: a slow hashed field over
    /// 4 x 4 cell blocks, so undergrowth comes in patches.
    fn conifer_patch(&self, (c, r): (i32, i32)) -> bool {
        hash((c.div_euclid(4), r.div_euclid(4)), 151) % 2 == 0
    }

    /// The undergrowth round the woods (see the module comment), as tufts
    /// for `Game::grass`. Pure in the map, no RNG.
    pub fn undergrowth(&self) -> Vec<GrassTuft> {
        if self.trees.is_empty() {
            return Vec::new();
        }
        let desert = self.theme == Theme::Desert;
        let mut out = Vec::new();
        for cell in self.open_cells() {
            let (c, r) = cell;
            let ring = |d: i32| (-d..=d).flat_map(move |dy| (-d..=d).map(move |dx| (c + dx, r + dy)));
            if self.trails.contains(&cell) || ring(1).any(|n| self.trails.contains(&n)) {
                continue;
            }
            let near = ring(1).filter(|&n| self.tree(n)).count() as u32;
            let bank = ORTHOGONAL.iter().any(|&(dx, dy)| self.water.contains(&(c + dx, r + dy)));
            let h = hash(cell, 161);
            let bush = if bank && !desert && ring(2).any(|n| self.tree(n)) && h % 100 < 70 {
                Bush::Reeds
            } else if near > 0 && h % 100 < 12 + near * 6 {
                let pick = (h >> 8) % 3;
                match (desert, self.conifer_patch(cell)) {
                    (true, _) => [Bush::Bush, Bush::Juniper, Bush::Bush][pick as usize],
                    (false, true) => [Bush::Juniper, Bush::Fern, Bush::Bush][pick as usize],
                    (false, false) => [Bush::Bush, Bush::Berry, Bush::Fern][pick as usize],
                }
            } else {
                continue;
            };
            // A little off the cell's middle, on the 2 px grid, and never
            // reaching over a wall beside it.
            let walled = self.walls.contains(&(c - 1, r)) || self.walls.contains(&(c + 1, r));
            let ox = if walled { 0.0 } else { (((h >> 12) % 7) as f32 - 3.0) * 2.0 };
            let oy = -(((h >> 16) % 4) as f32) * 2.0;
            let mut tuft = crate::grass::bush_tuft(cell_to_world(c, r), bush);
            tuft.base = Position::new(tuft.base.x + ox, tuft.base.y + oy);
            out.push(tuft);
        }
        // A fern between interior crowns now and then: rooted in the tree's
        // own cell, so the fire that fells the tree chars it too.
        if !desert {
            for &(c, r) in self.trees.keys() {
                let interior = (-1..=1).all(|dy| (-1..=1).all(|dx| self.tree((c + dx, r + dy))));
                if interior && hash((c, r), 171) % 5 == 0 {
                    let mut tuft = crate::grass::bush_tuft(cell_to_world(c, r), Bush::Fern);
                    tuft.base = Position::new(tuft.base.x + 10.0, tuft.base.y);
                    out.push(tuft);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map from rows of text: `T` a tree, `W` water, `#` a brick wall,
    /// `.` open ground.
    fn map_of(rows: &[&str]) -> MapFile {
        // A map is at least 8 cells a side; the rest is open ground.
        let mut text = format!("version = 1\nsize = [{}.0, {}.0]\n", rows[0].len().max(8), rows.len().max(8));
        for (r, row) in rows.iter().enumerate() {
            for (c, ch) in row.chars().enumerate() {
                let kind = match ch {
                    'T' => "{ kind = \"tree\" }",
                    'W' => "{ kind = \"water\" }",
                    '#' => "{ kind = \"wall\", material = \"brick\" }",
                    _ => continue,
                };
                text.push_str(&format!("cells.\"{c},{r}\" = {kind}\n"));
            }
        }
        MapFile::from_toml_str(&text).expect("test map parses")
    }

    const WOOD: [&str; 9] = [
        "..............",
        ".TTTTTTTTTTTT.",
        ".TTTTTTTTTTTT.",
        "..............",
        ".TTTTTTTTTTTT.",
        ".TTTTTTTTTTTT.",
        ".TTTTT.TTTTTT.",
        ".TTTTTTTTTTTT.",
        "..............",
    ];

    #[test]
    fn a_run_between_trees_is_a_trail_and_a_notch_is_not() {
        let woods = Woods::build(&map_of(&WOOD));
        // Row 3 runs between two blocks of wood: a trail along its length.
        assert!((2..=11).all(|c| woods.is_trail((c, 3))), "the open row between the stands");
        // The single gap at (6, 6) has trees round it but no run of trail
        // cells near it.
        assert!(!woods.is_trail((6, 6)), "a lone gap is not a trail");
        assert!(!woods.is_trail((0, 0)), "the meadow is not a trail");
    }

    #[test]
    fn crowns_stay_on_the_grid_lean_off_trails_and_out_over_water() {
        let woods = Woods::build(&map_of(&WOOD));
        for (&cell, crown) in &woods.crowns {
            assert!(crown.dx.abs() <= 4.0 && crown.dy.abs() <= 4.0, "{cell:?}: {crown:?}");
            assert_eq!((crown.dx % 2.0, crown.dy % 2.0), (0.0, 0.0), "{cell:?} off the 2 px grid");
        }
        // The trees either side of the trail lean away from it.
        assert!((2..=11).all(|c| woods.crown((c, 2)).dy < 0.0 && woods.crown((c, 4)).dy > 0.0));
        let bank = Woods::build(&map_of(&["TTTW", "TTTW", "TTTW"]));
        assert!((0..3).all(|r| bank.crown((2, r)).dx == 4.0), "a bank tree leans out over the water");
    }

    #[test]
    fn undergrowth_keeps_off_trails_water_and_the_maps_own_cells() {
        let map = map_of(&[
            "..........",
            ".TTTTTTT..",
            ".TTTTTTTWW",
            ".TTTTTTTWW",
            "..........",
            "...#......",
        ]);
        let woods = Woods::build(&map);
        let tufts = woods.undergrowth();
        assert!(!tufts.is_empty(), "a wood grows undergrowth round it");
        for tuft in &tufts {
            let cell = crate::map::world_to_cell(tuft.base);
            assert!(tuft.bush.is_some(), "undergrowth is bushes");
            if woods.tree(cell) {
                assert_eq!(tuft.bush, Some(Bush::Fern), "only a fern grows under the crowns");
                continue;
            }
            assert!(woods.open(cell), "{cell:?} holds something already");
            assert!(!woods.is_trail(cell));
        }
        let key = |t: &[GrassTuft]| t.iter().map(|t| (t.base.x, t.base.y, t.bush, t.col)).collect::<Vec<_>>();
        assert_eq!(key(&tufts), key(&woods.undergrowth()), "pure in the map");
    }

    const NARROW: [&str; 7] = [
        "............",
        ".TTTTTTTTTT.",
        ".TTTTTTTTTT.",
        "............",
        ".TTTTTTTTTT.",
        ".TTTTTTTTTT.",
        "............",
    ];

    #[test]
    fn a_one_cell_trail_is_narrow_and_a_two_cell_one_is_not() {
        let narrow = Woods::build(&map_of(&NARROW));
        assert!(narrow.narrow_trails().contains(&(5, 3)), "{:?}", narrow.narrow_trails());
        // Two open rows between the stands: one track, wide enough for the AI.
        let mut wide = NARROW.to_vec();
        wide.insert(3, "............");
        let wide = Woods::build(&map_of(&wide));
        assert!(wide.is_trail((5, 3)) && wide.is_trail((5, 4)));
        assert!(wide.narrow_trails().is_empty(), "{:?}", wide.narrow_trails());
    }

    #[test]
    fn felling_the_wood_beside_a_trail_opens_it_up() {
        let mut woods = Woods::build(&map_of(&NARROW));
        assert!(woods.is_trail((5, 3)));
        // Clear the stand above the trail, both rows, round its middle.
        for c in 3..=8 {
            woods.fell((c, 1));
            woods.fell((c, 2));
        }
        assert!(!woods.is_trail((5, 3)), "with no wood on one side it is open ground");
        // Felled tree by tree, the trails are what a map without those
        // trees has from the start: the local rework misses nothing.
        let mut cleared = NARROW.to_vec().into_iter().map(String::from).collect::<Vec<_>>();
        for row in &mut cleared[1..=2] {
            row.replace_range(3..=8, "......");
        }
        let fresh = Woods::build(&map_of(&cleared.iter().map(String::as_str).collect::<Vec<_>>()));
        assert_eq!(woods.trails, fresh.trails);
    }

    #[test]
    fn a_trail_that_runs_somewhere_is_worn_and_a_glade_is_not() {
        // Whether the floor carries a track: it differs from the same
        // woods' floor with no trails.
        let worn = |rows: &[&str]| {
            let woods = Woods::build(&map_of(rows));
            let bare = Woods { trails: BTreeSet::new(), ..Woods::build(&map_of(rows)) };
            woods.floor().texels != bare.floor().texels
        };
        assert!(worn(&WOOD), "the trail between the stands is a track");
        // A 2 x 2 glade inside a wood: trail cells, so it hides a tank, but
        // no run of three, so no ruts going nowhere.
        let glade = ["TTTTTT", "TTTTTT", "TT..TT", "TT..TT", "TTTTTT", "TTTTTT"];
        let woods = Woods::build(&map_of(&glade));
        assert!(woods.is_trail((2, 2)) && woods.is_trail((3, 3)));
        assert!(!worn(&glade));
    }

    #[test]
    fn the_forest_floor_stops_at_the_water() {
        let woods = Woods::build(&map_of(&["TTTWW", "TTTWW", "TTTWW"]));
        let floor = woods.floor();
        // The water's rows, 0-2, end at y = 80 (cell centres are on
        // multiples of 32), block 40; below them is open ground.
        let shaded = |x0: usize, x1: usize| (0..40).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| floor.texels[y * floor.width + x].a > 0).count();
        // The water's columns, 3-4, span x = 80..144: blocks 40..72.
        assert!(shaded(0, 30) > 0, "the floor under the trees is shaded");
        assert_eq!(shaded(40, 72), 0, "and none of it lies on the water");
    }
}
