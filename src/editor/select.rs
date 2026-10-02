//! The builder's rectangles of cells (docs/large-maps-patterns.md,
//! "Selection, copy and stamps"): the select tool's selection, what COPY
//! and CUT keep, the paste ghost that follows the pointer until a press
//! puts it down, and the stamps - the shipped ones under `maps/stamps/` and
//! the ones saved this session. Pure data over `MapFile` cells, headless;
//! `MapEditor` turns it into edits on the map and its undo stack, and
//! `render.rs` draws it.
//!
//! **A clip is transparent**: it holds the placed cells of its rectangle
//! and nothing for the empty ones, so a paste or a move writes only the
//! cells it carries and leaves the rest of the rectangle as it was - a
//! fort put down on grass keeps the grass between its walls.

use crate::map::{self, CellObject, MapFile};
use crate::math::Rectangle;
use crate::OBSTACLE_GRID_SIZE;

/// A rectangle of map cells: its top-left cell and its size in cells, at
/// least one each way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRect {
    pub col: i32,
    pub row: i32,
    pub cols: i32,
    pub rows: i32,
}

impl CellRect {
    /// The rectangle with `a` and `b` as opposite corners, both inside.
    pub fn spanning(a: (i32, i32), b: (i32, i32)) -> CellRect {
        let (col, row) = (a.0.min(b.0), a.1.min(b.1));
        CellRect { col, row, cols: (a.0 - b.0).abs() + 1, rows: (a.1 - b.1).abs() + 1 }
    }

    /// The bottom-right cell.
    pub fn last(self) -> (i32, i32) {
        (self.col + self.cols - 1, self.row + self.rows - 1)
    }

    pub fn contains(self, (col, row): (i32, i32)) -> bool {
        col >= self.col && row >= self.row && col < self.col + self.cols && row < self.row + self.rows
    }

    /// Every cell, row by row.
    pub fn cells(self) -> impl Iterator<Item = (i32, i32)> {
        (self.row..self.row + self.rows).flat_map(move |row| (self.col..self.col + self.cols).map(move |col| (col, row)))
    }

    /// The same rectangle `by` cells along.
    pub fn shifted(self, by: (i32, i32)) -> CellRect {
        CellRect { col: self.col + by.0, row: self.row + by.1, ..self }
    }

    /// The part inside `bounds`, `None` where they do not meet.
    pub fn within(self, bounds: CellRect) -> Option<CellRect> {
        let (col, row) = (self.col.max(bounds.col), self.row.max(bounds.row));
        let (end_col, end_row) = ((self.col + self.cols).min(bounds.col + bounds.cols), (self.row + self.rows).min(bounds.row + bounds.rows));
        (end_col > col && end_row > row).then_some(CellRect { col, row, cols: end_col - col, rows: end_row - row })
    }

    /// Where a rectangle of this size stands with its top-left at `at`,
    /// moved the least it takes to lie inside `bounds` - on an axis it is
    /// longer than `bounds`, at their near edge.
    pub fn kept_inside(self, bounds: CellRect) -> CellRect {
        let axis = |at: i32, len: i32, lo: i32, room: i32| if len <= room { at.clamp(lo, lo + room - len) } else { lo };
        CellRect { col: axis(self.col, self.cols, bounds.col, bounds.cols), row: axis(self.row, self.rows, bounds.row, bounds.rows), ..self }
    }

    /// The cells' square in world pixels: from half a cell before the
    /// first cell's middle to half a cell past the last one's.
    pub fn world(self) -> Rectangle {
        let p = map::cell_to_world(self.col, self.row);
        let half = OBSTACLE_GRID_SIZE / 2.0;
        Rectangle::new(p.x - half, p.y - half, self.cols as f32 * OBSTACLE_GRID_SIZE, self.rows as f32 * OBSTACLE_GRID_SIZE)
    }
}

/// The cells a map of `field` world pixels holds: every cell whose middle
/// is on the field, the half cells along its edges included - what a
/// pointer on the field can paint, and where a paste may put a cell.
pub fn field_cells(field: (f32, f32)) -> CellRect {
    let (cols, rows) = crate::minimap::cells_of(field);
    CellRect { col: 0, row: 0, cols: cols as i32, rows: rows as i32 }
}

/// Whether `obj` is one the map keeps at most one of - a start, player 2's
/// start, the frog, the enemy frog - which the brushes move rather than
/// copy, and a paste places only where the map holds none.
pub fn is_singleton(obj: &CellObject) -> bool {
    matches!(obj, CellObject::Start | CellObject::Start2 | CellObject::Frog | CellObject::EnemyFrog)
}

/// Which way a flip mirrors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Left to right: FLIP with the horizontal arrows.
    Horizontal,
    /// Top to bottom.
    Vertical,
}

impl Axis {
    /// As the dev server spells the flip: `flip_h`, `flip_v`.
    pub fn name(self) -> &'static str {
        match self {
            Axis::Horizontal => "flip_h",
            Axis::Vertical => "flip_v",
        }
    }
}

/// Cells lifted off the map: what COPY and CUT keep, what a stamp is and
/// what a paste puts down. `cells` are relative to the clip's top-left, in
/// row order, every one inside `cols` x `rows`; an empty cell of the
/// rectangle is simply not there (the module docs: a clip is transparent).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clip {
    pub cols: i32,
    pub rows: i32,
    pub cells: Vec<(i32, i32, CellObject)>,
}

impl Clip {
    /// The placed cells of `map` inside `rect`.
    pub fn of(map: &MapFile, rect: CellRect) -> Clip {
        let cells = map
            .iter_cells()
            .filter(|&(col, row, _)| rect.contains((col, row)))
            .map(|(col, row, obj)| (col - rect.col, row - rect.row, *obj))
            .collect();
        Clip { cols: rect.cols, rows: rect.rows, cells }
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// The clip mirrored along `axis` within its own rectangle, still in
    /// row order.
    pub fn flipped(&self, axis: Axis) -> Clip {
        let mut cells: Vec<(i32, i32, CellObject)> = self
            .cells
            .iter()
            .map(|&(c, r, obj)| match axis {
                Axis::Horizontal => (self.cols - 1 - c, r, obj),
                Axis::Vertical => (c, self.rows - 1 - r, obj),
            })
            .collect();
        cells.sort_by_key(|&(c, r, _)| (r, c));
        Clip { cols: self.cols, rows: self.rows, cells }
    }

    /// The clip cut down to the rectangle round its cells - a stamp saved
    /// from a loose selection carries no empty rows; an empty clip comes
    /// back as it is.
    pub fn trimmed(&self) -> Clip {
        let Some(c0) = self.cells.iter().map(|c| c.0).min() else { return self.clone() };
        let r0 = self.cells.iter().map(|c| c.1).min().unwrap_or(0);
        let c1 = self.cells.iter().map(|c| c.0).max().unwrap_or(0);
        let r1 = self.cells.iter().map(|c| c.1).max().unwrap_or(0);
        Clip { cols: c1 - c0 + 1, rows: r1 - r0 + 1, cells: self.cells.iter().map(|&(c, r, obj)| (c - c0, r - r0, obj)).collect() }
    }

    /// The clip's rectangle with its top-left at `at`.
    pub fn rect_at(&self, at: (i32, i32)) -> CellRect {
        CellRect { col: at.0, row: at.1, cols: self.cols.max(1), rows: self.rows.max(1) }
    }

    /// The cells as they land with the clip's top-left at `at`.
    pub fn placed_at(&self, at: (i32, i32)) -> impl Iterator<Item = (i32, i32, CellObject)> + '_ {
        self.cells.iter().map(move |&(c, r, obj)| (at.0 + c, at.1 + r, obj))
    }
}

/// The paste ghost: a clip standing on the canvas, outlined and drawn
/// faintly, until a press puts it down where it stands (`at`, its top-left
/// cell) or Escape, CANCEL or another tool takes it away.
#[derive(Clone, Debug, PartialEq)]
pub struct Ghost {
    pub clip: Clip,
    pub at: (i32, i32),
}

impl Ghost {
    /// `clip` standing with its middle cell on `cell`, inside `bounds`.
    pub fn new(clip: Clip, cell: (i32, i32), bounds: CellRect) -> Ghost {
        let mut ghost = Ghost { clip, at: (0, 0) };
        ghost.centre_on(cell, bounds);
        ghost
    }

    pub fn rect(&self) -> CellRect {
        self.clip.rect_at(self.at)
    }

    /// Stand with the middle cell on `cell` (the left and upper of two
    /// middles), kept inside `bounds` where it fits.
    pub fn centre_on(&mut self, cell: (i32, i32), bounds: CellRect) {
        let at = (cell.0 - (self.clip.cols - 1) / 2, cell.1 - (self.clip.rows - 1) / 2);
        self.move_to(at, bounds);
    }

    /// Stand with the top-left at `at`, kept inside `bounds` where it fits.
    pub fn move_to(&mut self, at: (i32, i32), bounds: CellRect) {
        let r = self.clip.rect_at(at).kept_inside(bounds);
        self.at = (r.col, r.row);
    }
}

/// Where a stamp came from: one of `SHIPPED_STAMPS` by its file's name, or
/// the `n`th saved this session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StampSource {
    Shipped(&'static str),
    Saved(u32),
}

/// A named clip the STAMPS list offers: choosing one makes it the paste
/// ghost.
#[derive(Clone, Debug, PartialEq)]
pub struct Stamp {
    pub source: StampSource,
    pub clip: Clip,
}

impl Stamp {
    /// The stamp as the dev server and `status.builder.buttons` name it:
    /// its file's name, or `saved_<n>`.
    pub fn key(&self) -> String {
        match self.source {
            StampSource::Shipped(name) => name.to_string(),
            StampSource::Saved(n) => format!("saved_{n}"),
        }
    }

    /// Its name in the language on screen: the catalogue's `stamp-<file>`,
    /// or the numbered saved one's.
    pub fn label(&self) -> String {
        let t = crate::text::text();
        match self.source {
            StampSource::Shipped(name) => t.named("stamp", name),
            StampSource::Saved(n) => t.fmt(crate::text::keys::STAMP_SAVED, &[("n", n.into())]),
        }
    }
}

/// The stamps every build offers, by file name: small maps under
/// `maps/stamps/`, embedded like `map::SHIPPED_MAPS` so the web build has
/// them too, each file's header saying what it is for. A stamp's extent is
/// its map's `size`, from its top-left cell.
pub const SHIPPED_STAMPS: &[(&str, &str)] = &[
    ("fort", include_str!("../../maps/stamps/fort.toml")),
    ("bunker", include_str!("../../maps/stamps/bunker.toml")),
    ("river-bend", include_str!("../../maps/stamps/river-bend.toml")),
];

/// The shipped stamps, in `SHIPPED_STAMPS` order. A file that does not
/// parse is left out (a test holds every one to parsing).
pub fn shipped_stamps() -> Vec<Stamp> {
    SHIPPED_STAMPS.iter().filter_map(|&(name, text)| stamp_of(name, text).ok()).collect()
}

/// One stamp file read: its map's cells inside its `size`.
fn stamp_of(name: &'static str, text: &str) -> Result<Stamp, String> {
    let map = MapFile::from_toml_str(text)?;
    let (cols, rows) = map.size.ok_or_else(|| format!("stamp {name} has no size"))?;
    let rect = CellRect { col: 0, row: 0, cols: cols.ceil() as i32, rows: rows.ceil() as i32 };
    Ok(Stamp { source: StampSource::Shipped(name), clip: Clip::of(&map, rect) })
}

#[cfg(test)]
mod select_tests {
    use super::*;
    use crate::obstacle::Material;

    fn brick() -> CellObject {
        CellObject::Wall { material: Material::Brick }
    }

    #[test]
    fn a_rect_spans_its_corners_either_way_and_keeps_inside_its_bounds() {
        let r = CellRect::spanning((5, 7), (2, 3));
        assert_eq!(r, CellRect { col: 2, row: 3, cols: 4, rows: 5 });
        assert_eq!(r.last(), (5, 7));
        assert!(r.contains((2, 3)) && r.contains((5, 7)) && !r.contains((6, 7)) && !r.contains((2, 2)));
        assert_eq!(r.cells().count(), 20);
        assert_eq!(r.cells().next(), Some((2, 3)));
        let bounds = CellRect { col: 0, row: 0, cols: 10, rows: 6 };
        assert_eq!(r.within(bounds), Some(CellRect { col: 2, row: 3, cols: 4, rows: 3 }));
        assert_eq!(CellRect::spanning((20, 20), (22, 22)).within(bounds), None);
        assert_eq!(r.shifted((7, 0)).kept_inside(bounds), CellRect { col: 6, row: 1, cols: 4, rows: 5 });
        assert_eq!(r.shifted((-9, -9)).kept_inside(bounds), CellRect { col: 0, row: 0, cols: 4, rows: 5 });
        let wide = CellRect { col: 3, row: 0, cols: 12, rows: 2 };
        assert_eq!(wide.kept_inside(bounds).col, 0, "longer than its bounds: at their near edge");
        let world = CellRect { col: 1, row: 2, cols: 2, rows: 1 }.world();
        assert_eq!(world, Rectangle::new(16.0, 48.0, 64.0, 32.0));
        assert_eq!(field_cells((1088.0, 544.0)), CellRect { col: 0, row: 0, cols: 35, rows: 18 });
    }

    #[test]
    fn a_clip_holds_its_cells_relative_flips_in_place_and_trims() {
        let mut map = MapFile::new();
        map.set_cell(4, 4, brick());
        map.set_cell(6, 5, CellObject::Water);
        map.set_cell(9, 9, CellObject::Gate);
        let clip = Clip::of(&map, CellRect::spanning((3, 3), (7, 6)));
        assert_eq!((clip.cols, clip.rows), (5, 4));
        assert_eq!(clip.cells, vec![(1, 1, brick()), (3, 2, CellObject::Water)]);
        let h = clip.flipped(Axis::Horizontal);
        assert_eq!(h.cells, vec![(3, 1, brick()), (1, 2, CellObject::Water)]);
        let v = clip.flipped(Axis::Vertical);
        assert_eq!(v.cells, vec![(3, 1, CellObject::Water), (1, 2, brick())], "row order after the flip");
        assert_eq!(h.flipped(Axis::Horizontal), clip, "twice is none");
        let t = clip.trimmed();
        assert_eq!((t.cols, t.rows), (3, 2));
        assert_eq!(t.cells, vec![(0, 0, brick()), (2, 1, CellObject::Water)]);
        assert!(Clip::of(&map, CellRect::spanning((0, 0), (2, 2))).is_empty());
        assert_eq!(clip.placed_at((10, 20)).collect::<Vec<_>>(), vec![(11, 21, brick()), (13, 22, CellObject::Water)]);
    }

    #[test]
    fn a_ghost_centres_on_a_cell_and_keeps_inside_the_field() {
        let clip = Clip { cols: 3, rows: 2, cells: vec![(0, 0, brick())] };
        let bounds = CellRect { col: 0, row: 0, cols: 20, rows: 10 };
        let mut ghost = Ghost::new(clip, (10, 5), bounds);
        assert_eq!(ghost.at, (9, 5));
        assert_eq!(ghost.rect(), CellRect { col: 9, row: 5, cols: 3, rows: 2 });
        ghost.centre_on((0, 0), bounds);
        assert_eq!(ghost.at, (0, 0));
        ghost.centre_on((19, 9), bounds);
        assert_eq!(ghost.at, (17, 8), "the far corner keeps it whole on the field");
    }

    /// Every shipped stamp parses, is no larger than a screen's worth of
    /// map, holds cells inside its own size and none the map keeps one of
    /// - a stamp put down never moves a start - and has a name in English.
    #[test]
    fn every_shipped_stamp_parses_and_holds_no_singleton() {
        let stamps = shipped_stamps();
        assert_eq!(stamps.len(), SHIPPED_STAMPS.len(), "a shipped stamp does not parse");
        let english = crate::text::Catalogue::new("en");
        for stamp in &stamps {
            let clip = &stamp.clip;
            assert!(!clip.is_empty(), "{}", stamp.key());
            assert!(clip.cols <= 16 && clip.rows <= 12, "{} is {}x{}", stamp.key(), clip.cols, clip.rows);
            for &(c, r, obj) in &clip.cells {
                assert!(c >= 0 && r >= 0 && c < clip.cols && r < clip.rows, "{}: ({c},{r}) outside", stamp.key());
                assert!(!is_singleton(&obj) && obj != CellObject::Gate, "{}: {obj:?}", stamp.key());
            }
            let StampSource::Shipped(name) = stamp.source else { panic!("shipped") };
            assert!(english.message(&format!("stamp-{name}"), &[]).is_some(), "lang/en.ftl names no stamp-{name}");
            assert_eq!(stamp.key(), name);
        }
        let saved = Stamp { source: StampSource::Saved(3), clip: Clip::default() };
        assert_eq!(saved.key(), "saved_3");
    }
}
