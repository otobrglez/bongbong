//! The brush's shapes past PEN (docs/large-maps-patterns.md, "Area
//! brushes: fill, scatter, auto-tile"): which cells a press lays the
//! brush's object on - RECT's rectangle, FILL's flood and SCATTER's hashed
//! share of a footprint. Pure functions of the map and the press,
//! headless; `MapEditor` makes them edits on the map and its undo stack,
//! one step a stroke, and `render.rs` draws what a shape is about to do.
//! Water and road a shape lays auto-tile like a stroke's, through the
//! ground's repaint.
//!
//! **Nothing here draws a random number**: a scatter's cells are a hash of
//! the cell and a stroke counter (`blast::seed_at`), so a stroke decides
//! each cell once however often its drag crosses it, and the next stroke
//! over the same ground picks others.

use super::select::CellRect;
use crate::map::{self, MapFile};
use crate::tuning::{tuning, Tuning};

/// The `builder` tuning rows the shapes read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrushRules {
    /// `builder_fill_max_cells`: the most cells one FILL changes.
    pub fill_max_cells: usize,
    /// `builder_scatter_radius_cells`: the footprint's radius, in cells.
    pub scatter_radius: i32,
    /// `builder_scatter_density`: the share of the footprint a scatter
    /// paints.
    pub scatter_density: f32,
}

impl BrushRules {
    /// The rules in the tuning table this frame.
    pub fn current() -> BrushRules {
        BrushRules::of(&tuning())
    }

    /// The rules in `t`.
    pub fn of(t: &Tuning) -> BrushRules {
        BrushRules {
            fill_max_cells: t.builder_fill_max_cells,
            scatter_radius: t.builder_scatter_radius_cells,
            scatter_density: t.builder_scatter_density,
        }
    }
}

/// FILL's cells from `start`: every cell inside `bounds` joined to it edge
/// to edge through cells that hold exactly what it holds - nothing
/// included -, in row order. `None` once there are more than `max`: a fill
/// that has found its way out of a fort through a gap stops there rather
/// than flooding a 250 x 250 map in one frame, and the search itself never
/// looks at more than `max` cells and their neighbours. A start outside
/// `bounds` fills nothing.
pub fn flood(map: &MapFile, bounds: CellRect, start: (i32, i32), max: usize) -> Option<Vec<(i32, i32)>> {
    if !bounds.contains(start) {
        return Some(Vec::new());
    }
    let cols = bounds.cols as usize;
    let index = |(col, row): (i32, i32)| (row - bounds.row) as usize * cols + (col - bounds.col) as usize;
    let target = map.cell(start.0, start.1).copied();
    let mut seen = vec![false; cols * bounds.rows as usize];
    seen[index(start)] = true;
    let mut stack = vec![start];
    let mut cells = Vec::new();
    while let Some(cell) = stack.pop() {
        if cells.len() == max {
            return None;
        }
        cells.push(cell);
        let (col, row) = cell;
        for next in [(col + 1, row), (col - 1, row), (col, row + 1), (col, row - 1)] {
            if !bounds.contains(next) {
                continue;
            }
            let i = index(next);
            if !seen[i] && map.cell(next.0, next.1).copied() == target {
                seen[i] = true;
                stack.push(next);
            }
        }
    }
    cells.sort_by_key(|&(col, row)| (row, col));
    Some(cells)
}

/// SCATTER's footprint round `cell`: the cells whose middles lie within
/// `radius` and a half cells of its middle - a disc `2 * radius + 1` cells
/// across, the one cell at 0 - inside `bounds`, in row order.
pub fn footprint(cell: (i32, i32), radius: i32, bounds: CellRect) -> impl Iterator<Item = (i32, i32)> {
    let radius = radius.max(0);
    let reach = (radius as f32 + 0.5) * (radius as f32 + 0.5);
    (-radius..=radius)
        .flat_map(move |dy| (-radius..=radius).map(move |dx| (dx, dy)))
        .filter(move |&(dx, dy)| (dx * dx + dy * dy) as f32 <= reach)
        .map(move |(dx, dy)| (cell.0 + dx, cell.1 + dy))
        .filter(move |&c| bounds.contains(c))
}

/// Whether scatter stroke `stroke` lays its object on `cell`: a hash of the
/// cell and the stroke under `density`. The same answer every time it is
/// asked, so a drag crossing a cell twice decides it once.
pub fn scattered(cell: (i32, i32), stroke: u32, density: f32) -> bool {
    crate::blast::hash_unit(crate::blast::seed_at(map::cell_to_world(cell.0, cell.1), stroke), SCATTER_SALT) < density
}

/// Keeps a scatter's choice of cells apart from every other hash of a cell
/// (`blast::hash_unit`'s `k`).
const SCATTER_SALT: u32 = 0x5ca7;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::CellObject;
    use crate::obstacle::Material;

    fn bounds() -> CellRect {
        CellRect { col: 0, row: 0, cols: 20, rows: 12 }
    }

    fn brick() -> CellObject {
        CellObject::Wall { material: Material::Brick }
    }

    /// A ring of brick round (2..=6, 2..=6), open nowhere, with water in
    /// its middle and one road cell outside it.
    fn walled() -> MapFile {
        let mut map = MapFile::default();
        for i in 2..=6 {
            for (c, r) in [(i, 2), (i, 6), (2, i), (6, i)] {
                map.set_cell(c, r, brick());
            }
        }
        map.set_cell(4, 4, CellObject::Water);
        map.set_cell(10, 3, CellObject::Road);
        map
    }

    #[test]
    fn a_flood_takes_the_cells_joined_edge_to_edge_that_hold_what_the_start_holds() {
        let map = walled();
        // Inside the ring: the eight empty cells round the water.
        let inside = flood(&map, bounds(), (3, 3), 1000).expect("small");
        assert_eq!(inside.len(), 8, "{inside:?}");
        assert!(!inside.contains(&(4, 4)), "the water is not empty");
        assert!(inside.windows(2).all(|w| (w[0].1, w[0].0) < (w[1].1, w[1].0)), "row order: {inside:?}");
        // The ring itself: brick joined edge to edge, 16 cells.
        assert_eq!(flood(&map, bounds(), (2, 2), 1000).expect("small").len(), 16);
        // The water alone.
        assert_eq!(flood(&map, bounds(), (4, 4), 1000), Some(vec![(4, 4)]));
        // Outside the ring: everything empty but the ring's inside.
        let outside = flood(&map, bounds(), (0, 0), 1000).expect("small");
        assert_eq!(outside.len(), (20 * 12 - 25 - 1) as usize, "the field less the ring's 5 x 5 and the road");
        assert!(outside.iter().all(|&c| bounds().contains(c)), "never past the map");
        // A start past the map fills nothing.
        assert_eq!(flood(&map, bounds(), (30, 3), 1000), Some(Vec::new()));
    }

    #[test]
    fn a_flood_past_its_cap_is_none_and_looks_no_further() {
        let map = walled();
        assert_eq!(flood(&map, bounds(), (3, 3), 8).map(|c| c.len()), Some(8), "exactly the cap fills");
        assert_eq!(flood(&map, bounds(), (3, 3), 7), None, "one past it does not");
        assert_eq!(flood(&map, bounds(), (0, 0), 100), None);
        // A 250 x 250 map, empty: refused after the cap, quickly.
        let big = CellRect { col: 0, row: 0, cols: 251, rows: 251 };
        let start = std::time::Instant::now();
        assert_eq!(flood(&MapFile::default(), big, (125, 125), 4096), None);
        assert!(start.elapsed().as_secs_f32() < 1.0, "{:?}", start.elapsed());
    }

    #[test]
    fn a_footprint_is_a_disc_inside_the_map() {
        let all = |r: i32| footprint((10, 6), r, bounds()).collect::<Vec<_>>();
        assert_eq!(all(0), vec![(10, 6)]);
        assert_eq!(all(1).len(), 9, "a 3 x 3 block");
        let two = all(2);
        assert_eq!(two.len(), 21, "5 x 5 less its corners");
        assert!(!two.contains(&(12, 8)) && two.contains(&(12, 7)) && two.contains(&(10, 8)));
        // At the map's corner only what is on the map.
        let corner: Vec<_> = footprint((0, 0), 2, bounds()).collect();
        assert!(corner.iter().all(|&c| bounds().contains(c)));
        assert_eq!(corner.len(), 8, "a quarter of the disc and its edges: {corner:?}");
    }

    #[test]
    fn a_scatter_picks_its_share_of_cells_the_same_way_every_time_and_another_share_each_stroke() {
        let cells: Vec<(i32, i32)> = bounds().cells().collect();
        let picked = |stroke: u32, density: f32| cells.iter().filter(|&&c| scattered(c, stroke, density)).copied().collect::<Vec<_>>();
        let first = picked(1, 0.3);
        assert_eq!(first, picked(1, 0.3), "a pure function of the cell and the stroke");
        let share = first.len() as f32 / cells.len() as f32;
        assert!((0.2..0.4).contains(&share), "about the density: {share}");
        let second = picked(2, 0.3);
        assert_ne!(first, second, "another stroke, other cells");
        assert!(picked(1, 0.1).iter().all(|c| first.contains(c)), "a lower density picks a subset");
        assert_eq!(picked(1, 1.0).len(), cells.len(), "density 1 is every cell");
    }
}
