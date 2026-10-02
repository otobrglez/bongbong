//! The canvas's cells as the builder reads them every frame
//! (`MapEditor::cell_index`), kept rather than walked out of the map each
//! time. `MapFile::cells` is keyed by `"col,row"` text, so every walk of it
//! parses each key and sorts (`MapFile::iter_cells`) and every lookup
//! formats one (`MapFile::cell`): what a round does once, at its start, and
//! too much for every frame on a map as large as 250 x 250 painted edge to
//! edge (`a_dense_maps_frame_timing`). An edit of a few cells is taken in
//! cell by cell (`CellIndex::update`); any other works the index out
//! again on the next read.

use super::CellChange;
use crate::map::{CellObject, MapFile};
use crate::math::Rectangle;
use crate::tower::TowerKind;
use crate::OBSTACLE_GRID_SIZE;

/// The most changes `CellIndex::update` takes in one by one: past it the
/// index is worked out again whole, which costs less than as many
/// insertions into a long list.
const PATCH_MOST: usize = 64;

/// A cell's share of `CellIndex::digest`: a hash of its place and of what
/// stands on it, the same for the same cell wherever and whenever it is
/// worked out.
fn cell_digest(col: i32, row: i32, obj: &CellObject) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = CellHasher(0);
    (col, row, obj).hash(&mut hasher);
    hasher.finish()
}

/// The hasher `cell_digest` runs: a multiply and a rotation per word fed
/// (FxHash's step), a few nanoseconds a cell where a whole map's index is
/// worked out at once, finished by SplitMix64's mix so that the sum of
/// many digests spreads over all 64 bits.
struct CellHasher(u64);

impl CellHasher {
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl std::hash::Hasher for CellHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.add(byte as u64);
        }
    }

    fn write_u8(&mut self, n: u8) {
        self.add(n as u64);
    }

    fn write_u32(&mut self, n: u32) {
        self.add(n as u64);
    }

    fn write_i32(&mut self, n: i32) {
        self.add(n as u32 as u64);
    }

    fn write_u64(&mut self, n: u64) {
        self.add(n);
    }

    fn write_usize(&mut self, n: usize) {
        self.add(n as u64);
    }

    fn write_isize(&mut self, n: isize) {
        self.add(n as u64);
    }

    fn finish(&self) -> u64 {
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// Whether the index keeps a list of `obj`'s kind beside the cells: a
/// portal, a singleton or a tower.
fn kept(obj: &CellObject) -> bool {
    matches!(obj, CellObject::Portal | CellObject::Frog | CellObject::Start | CellObject::Start2 | CellObject::EnemyFrog) || obj.tower().is_some()
}

/// Every placed cell of a map in `MapFile::iter_cells`'s order - row by
/// row, left to right - and what the builder reads of them every frame:
/// the portals, the singletons' cells and the kinds of tower whose reach
/// rings spread past their cells, and a digest of them all.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellIndex {
    cells: Vec<(i32, i32, CellObject)>,
    /// The cells' `cell_digest`s summed: the same for the same cells in
    /// any order, kept up through every change `update` takes in.
    digest: u64,
    portals: Vec<(i32, i32)>,
    frog: Option<(i32, i32)>,
    start: Option<(i32, i32)>,
    start2: Option<(i32, i32)>,
    enemy_frog: Option<(i32, i32)>,
    towers: Vec<TowerKind>,
}

impl CellIndex {
    /// The index of `map` as it stands.
    pub fn of(map: &MapFile) -> CellIndex {
        let mut index = CellIndex { cells: map.iter_cells().map(|(col, row, obj)| (col, row, *obj)).collect(), ..CellIndex::default() };
        index.digest = index.cells.iter().fold(0, |sum: u64, &(col, row, obj)| sum.wrapping_add(cell_digest(col, row, &obj)));
        index.derive();
        index
    }

    /// Take in the cells `changes` touched as `map` holds them now, rather
    /// than working the index out again: a search and a replacement, an
    /// insertion or a removal a cell, and the portals, singletons and
    /// towers read again only where one of those came or went. More than
    /// `PATCH_MOST` changes, a cell the index holds twice (a hand-written
    /// key such as `"03,4"` beside `"3,4"`) or a count that then disagrees
    /// with the map's work it out again whole - so the index is always
    /// `CellIndex::of(map)`.
    pub fn update(&mut self, map: &MapFile, changes: &[CellChange]) {
        if changes.len() > PATCH_MOST {
            *self = CellIndex::of(map);
            return;
        }
        let mut derive = false;
        for c in changes {
            let at = self.cells.binary_search_by(|&(col, row, _)| (row, col).cmp(&(c.row, c.col)));
            if at.is_ok_and(|i| self.twinned(i)) {
                *self = CellIndex::of(map);
                return;
            }
            let was = at.ok().map(|i| cell_digest(c.col, c.row, &self.cells[i].2)).unwrap_or(0);
            let now = map.cell(c.col, c.row).copied();
            self.digest = self.digest.wrapping_sub(was).wrapping_add(now.map_or(0, |obj| cell_digest(c.col, c.row, &obj)));
            match (at, now) {
                (Ok(i), Some(obj)) => {
                    derive |= kept(&self.cells[i].2) || kept(&obj);
                    self.cells[i].2 = obj;
                }
                (Ok(i), None) => {
                    derive |= kept(&self.cells[i].2);
                    self.cells.remove(i);
                }
                (Err(i), Some(obj)) => {
                    derive |= kept(&obj);
                    self.cells.insert(i, (c.col, c.row, obj));
                }
                (Err(_), None) => {}
            }
        }
        if self.cells.len() != map.cells.len() {
            *self = CellIndex::of(map);
        } else if derive {
            self.derive();
        }
    }

    /// Whether the cell at `i` shares its place with a neighbour.
    fn twinned(&self, i: usize) -> bool {
        let place = |j: usize| self.cells.get(j).map(|&(col, row, _)| (col, row));
        let here = place(i);
        (i > 0 && place(i - 1) == here) || place(i + 1) == here
    }

    /// Read the portals, the singletons and the towers off the cells.
    fn derive(&mut self) {
        self.portals.clear();
        self.towers.clear();
        (self.frog, self.start, self.start2, self.enemy_frog) = (None, None, None, None);
        for &(col, row, obj) in &self.cells {
            let slot = match obj {
                CellObject::Portal => {
                    self.portals.push((col, row));
                    None
                }
                CellObject::Frog => Some(&mut self.frog),
                CellObject::Start => Some(&mut self.start),
                CellObject::Start2 => Some(&mut self.start2),
                CellObject::EnemyFrog => Some(&mut self.enemy_frog),
                _ => None,
            };
            if let Some(slot) = slot {
                slot.get_or_insert((col, row));
            }
            if let Some((kind, _)) = obj.tower()
                && !self.towers.contains(&kind)
            {
                self.towers.push(kind);
            }
        }
    }

    /// Every placed cell, row by row and left to right.
    pub fn cells(&self) -> &[(i32, i32, CellObject)] {
        &self.cells
    }

    /// The cells' digest: maps whose cells are the same have the same one,
    /// whatever order they were placed in, and any change of a cell moves
    /// it but for a collision of 64-bit hashes.
    pub fn digest(&self) -> u64 {
        self.digest
    }

    /// The cells whose drawing can reach into `rect`, in world pixels: those
    /// whose middle stands within it grown by the farthest reach of a tower
    /// on the map (`TowerKind::range`, read now), in order - a few cells
    /// more, never fewer, so the painter still culls each by its own reach.
    /// Every cell for `None`. Only the rows the rect spans are walked.
    pub fn near(&self, rect: Option<Rectangle>) -> impl Iterator<Item = &(i32, i32, CellObject)> {
        let reach = self.towers.iter().map(|kind| kind.range()).fold(0.0_f32, f32::max);
        let (rows, cols) = match rect {
            None => ((i32::MIN, i32::MAX), (i32::MIN, i32::MAX)),
            Some(r) => {
                let span = |from: f32, len: f32| {
                    let lo = ((from - reach) / OBSTACLE_GRID_SIZE).floor() as i32 - 1;
                    let hi = ((from + len + reach) / OBSTACLE_GRID_SIZE).ceil() as i32 + 1;
                    (lo, hi)
                };
                (span(r.y, r.height), span(r.x, r.width))
            }
        };
        let start = self.cells.partition_point(|&(_, row, _)| row < rows.0);
        let end = self.cells.partition_point(|&(_, row, _)| row <= rows.1).max(start);
        self.cells[start..end].iter().filter(move |&&(col, _, _)| col >= cols.0 && col <= cols.1)
    }

    /// Every portal's anchor cell, in order.
    pub fn portals(&self) -> &[(i32, i32)] {
        &self.portals
    }

    /// The frog's cell - the first, on a map that placed more than one.
    pub fn frog(&self) -> Option<(i32, i32)> {
        self.frog
    }

    /// Player 1's start.
    pub fn start(&self) -> Option<(i32, i32)> {
        self.start
    }

    /// Player 2's start.
    pub fn start2(&self) -> Option<(i32, i32)> {
        self.start2
    }

    /// The enemy frog's cell.
    pub fn enemy_frog(&self) -> Option<(i32, i32)> {
        self.enemy_frog
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frog::Side;
    use crate::map;
    use crate::obstacle::Material;

    fn brick() -> CellObject {
        CellObject::Wall { material: Material::Brick }
    }

    /// A map with some of everything the index keeps.
    fn map() -> MapFile {
        let mut map = MapFile::new();
        map.size = Some((60.0, 40.0));
        for col in 0..60 {
            for row in (0..40).step_by(3) {
                map.set_cell(col, row, brick());
            }
        }
        map.set_cell(5, 7, CellObject::Start);
        map.set_cell(50, 31, CellObject::Frog);
        map.set_cell(9, 1, CellObject::Start2);
        map.set_cell(3, 2, CellObject::Portal);
        map.set_cell(40, 35, CellObject::Portal);
        map.set_cell(30, 20, CellObject::Tesla { side: Some(Side::Enemy) });
        map
    }

    /// The index reads the map as `MapFile`'s own walks do: every cell in
    /// `iter_cells`'s order, the portals in order, each singleton's cell.
    #[test]
    fn the_index_is_the_maps_own_reading() {
        let map = map();
        let index = CellIndex::of(&map);
        let walked: Vec<(i32, i32, CellObject)> = map.iter_cells().map(|(c, r, o)| (c, r, *o)).collect();
        assert_eq!(index.cells(), &walked[..]);
        assert_eq!(index.portals(), &map.portal_cells()[..]);
        assert_eq!(
            (index.start(), index.start2(), index.frog(), index.enemy_frog()),
            (map.start_cell(), map.start2_cell(), map.frog_cell(), map.enemy_frog_cell())
        );
    }

    /// The digest is the cells' and nothing else's: the same cells placed
    /// in another order give the same one, and a cell changed, added or
    /// taken off moves it.
    #[test]
    fn the_digest_is_the_cells_whatever_their_order() {
        let map = map();
        let digest = |map: &MapFile| CellIndex::of(map).digest();
        let mut backwards = MapFile::new();
        backwards.size = map.size;
        let mut cells: Vec<(i32, i32, CellObject)> = map.iter_cells().map(|(c, r, o)| (c, r, *o)).collect();
        cells.reverse();
        for (col, row, obj) in cells {
            backwards.set_cell(col, row, obj);
        }
        assert_eq!(digest(&backwards), digest(&map));
        let mut changed = map.clone();
        changed.set_cell(0, 0, CellObject::Wall { material: Material::Iron });
        let mut added = map.clone();
        added.set_cell(1, 1, CellObject::Road);
        let mut taken = map.clone();
        taken.clear_cell(9, 1);
        for (what, other) in [("changed", changed), ("added", added), ("taken off", taken)] {
            assert_ne!(digest(&other), digest(&map), "a cell {what}");
        }
    }

    /// An index that takes edits in (`update`) is the index worked out
    /// afresh after each: cells painted, replaced and cleared, portals,
    /// singletons and towers coming and going, a batch past `PATCH_MOST`,
    /// and a map holding one cell under two keys.
    #[test]
    fn an_updated_index_is_the_index_of_the_map() {
        let mut map = map();
        let mut index = CellIndex::of(&map);
        let objects = [
            Some(brick()),
            None,
            Some(CellObject::Road),
            Some(CellObject::Portal),
            Some(CellObject::Start),
            Some(CellObject::Frog),
            Some(CellObject::EnemyFrog),
            Some(CellObject::GunTower { side: None }),
            None,
            Some(CellObject::Wall { material: Material::Iron }),
        ];
        let mut seed = 0x2545_f491_u32;
        let mut next = |n: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed % n
        };
        for round in 0..300 {
            let n = if round % 50 == 49 { PATCH_MOST + 20 } else { 1 + next(6) as usize };
            let mut changes = Vec::new();
            for _ in 0..n {
                let (col, row) = (next(60) as i32, next(40) as i32);
                let before = map.cell(col, row).copied();
                let after = objects[next(objects.len() as u32) as usize];
                match after {
                    Some(obj) => map.set_cell(col, row, obj),
                    None => map.clear_cell(col, row),
                }
                changes.push(CellChange { col, row, before, after });
            }
            index.update(&map, &changes);
            assert_eq!(index, CellIndex::of(&map), "round {round}");
        }
        // One cell under two keys: worked out again whole.
        map.cells.insert("03,4".to_string(), CellObject::Fence);
        map.set_cell(3, 4, brick());
        let mut index = CellIndex::of(&map);
        map.set_cell(3, 4, CellObject::Sandbag);
        index.update(&map, &[CellChange { col: 3, row: 4, before: Some(brick()), after: Some(CellObject::Sandbag) }]);
        assert_eq!(index.cells().iter().filter(|&&(c, r, _)| (c, r) == (3, 4)).count(), 2);
        assert!(index.cells().contains(&(3, 4, CellObject::Sandbag)) && index.cells().contains(&(3, 4, CellObject::Fence)));
    }

    /// `near` gives every cell whose middle a rect grown by the farthest
    /// tower reach holds - whatever the rect, at the field's edges and past
    /// them - and every cell for no rect.
    #[test]
    fn near_holds_every_cell_a_rect_can_show() {
        let map = map();
        let index = CellIndex::of(&map);
        assert_eq!(index.near(None).count(), index.cells().len());
        let reach = TowerKind::Tesla.range();
        for rect in [
            Rectangle::new(0.0, 0.0, 400.0, 300.0),
            Rectangle::new(613.0, 211.0, 333.0, 170.0),
            Rectangle::new(1500.0, 1100.0, 900.0, 500.0),
            Rectangle::new(-300.0, -200.0, 200.0, 100.0),
            Rectangle::new(905.0, 600.0, 10.0, 10.0),
        ] {
            let near: Vec<_> = index.near(Some(rect)).collect();
            for (col, row, obj) in index.cells() {
                let pos = map::cell_to_world(*col, *row);
                let grown = pos.x >= rect.x - reach && pos.x <= rect.x + rect.width + reach && pos.y >= rect.y - reach && pos.y <= rect.y + rect.height + reach;
                if grown {
                    assert!(near.contains(&&(*col, *row, *obj)), "{rect:?}: ({col}, {row}) missing");
                }
            }
            assert!(near.windows(2).all(|w| (w[0].1, w[0].0) < (w[1].1, w[1].0)), "in order");
        }
    }
}
