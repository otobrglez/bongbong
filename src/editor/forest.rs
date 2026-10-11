//! The forest brush (docs/WOODS.md §6): plants trees and picks each one's
//! species from where it stands, so a wood painted in one stroke reads as
//! a wood rather than a block of one tree. Pure functions of the map and
//! the cell, headless, with no RNG: a pick is a hash of the cell, so the
//! same stroke on the same map plants the same wood.
//!
//! What decides a species, first rule first:
//!
//! - **Water** within three cells (`water_distance`). Grass: willow on
//!   the bank, birch a step back, conifers and broadleaf further out.
//!   Desert: date palms on the bank, palms or broadleaf two cells out,
//!   broadleaf and pine at three, an oasis drawing itself round its water.
//! - **The wood's edge**: a tree with open ground beside it is on the
//!   outer ring, where fir saplings and birch grow. A sapling is rammed
//!   through in a moment, so a conifer stand can be forced almost anywhere.
//! - **The stand** (`woods::conifer_stand`): conifer patches and broadleaf
//!   patches, the same field the undergrowth follows.
//! - **The theme**: away from water a desert grows dry conifers and dead
//!   snags.
//!
//! **A stream is a way in.** The brush plants nothing on a cell touching a
//! stream, bends included, and clears the trees a stroke crosses there, so
//! a stream through a wood is a corridor of bank, ford and bank wide
//! enough for the AI. A lake is planted to its shore. Clearing is ordinary
//! edits on the undo stack: the map file says exactly what is there.

use crate::map::{CellObject, MapFile, Theme};
use crate::obstacle::Material;

use super::select::CellRect;

/// How far from water, in cells, the water still decides a species.
pub const WATER_REACH: i32 = 3;

/// A 0..100 hash of a cell, for the share each species gets.
fn roll(cell: (i32, i32)) -> u32 {
    crate::blast::seed_at(crate::map::cell_to_world(cell.0, cell.1), 211) % 100
}

fn water(map: &MapFile, (c, r): (i32, i32)) -> bool {
    map.cell(c, r) == Some(&CellObject::Water)
}

fn tree(map: &MapFile, (c, r): (i32, i32)) -> bool {
    map.cell(c, r).and_then(CellObject::material).is_some_and(Material::is_tree)
}

/// Whether `cell` is a stream: water in no 2 x 2 block of water, the
/// ground's own line between a river and a lake (`ground::GroundGrid`).
pub fn is_stream(map: &MapFile, (c, r): (i32, i32)) -> bool {
    water(map, (c, r))
        && ![(-1, -1), (0, -1), (-1, 0), (0, 0)]
            .iter()
            .any(|&(ox, oy)| (0..2).all(|dy| (0..2).all(|dx| water(map, (c + ox + dx, r + oy + dy)))))
}

/// Whether `cell` touches a stream, along a side or at a corner: a bend's
/// inside corner touches it only at a corner.
pub fn by_stream(map: &MapFile, (c, r): (i32, i32)) -> bool {
    (-1..=1).any(|dy| (-1..=1).any(|dx| (dx, dy) != (0, 0) && is_stream(map, (c + dx, r + dy))))
}

/// The ring, in cells, of the nearest water round `cell` (1 beside it),
/// out to `WATER_REACH`.
pub fn water_distance(map: &MapFile, (c, r): (i32, i32)) -> Option<i32> {
    (1..=WATER_REACH).find(|&k| (-k..=k).any(|dy| (-k..=k).any(|dx| (dx.abs() == k || dy.abs() == k) && water(map, (c + dx, r + dy)))))
}

/// Whether `cell` is on a wood's outer ring: one of its eight neighbours
/// on the field is not a tree. Past the field's edge counts as wood, so a
/// wood painted to the border has no ring there.
pub fn on_edge(map: &MapFile, bounds: CellRect, (c, r): (i32, i32)) -> bool {
    (-1..=1).any(|dy| (-1..=1).any(|dx| (dx, dy) != (0, 0) && bounds.contains((c + dx, r + dy)) && !tree(map, (c + dx, r + dy))))
}

/// The species the brush plants on `cell`, given the map round it (see
/// the module comment).
pub fn species(map: &MapFile, bounds: CellRect, cell: (i32, i32)) -> Material {
    use Material::*;
    let h = roll(cell);
    let edge = on_edge(map, bounds, cell);
    if map.theme == Theme::Desert {
        return match water_distance(map, cell) {
            Some(1) => Palm,
            Some(2) => if h < 70 { Palm } else { Tree },
            Some(3) => if h < 50 { Tree } else { Pine },
            _ if edge && h < 40 => Fir,
            _ => match h {
                0..30 => Snag,
                30..55 => Pine,
                55..80 => Scots,
                _ => Spruce,
            },
        };
    }
    match water_distance(map, cell) {
        Some(1) => if h < 70 { Willow } else { Birch },
        Some(2) => match h {
            0..60 => Birch,
            60..80 => Willow,
            _ => Tree,
        },
        _ => match (edge, crate::woods::conifer_stand(cell)) {
            (true, true) => if h < 70 { Fir } else { Birch },
            (true, false) => if h < 60 { Birch } else { Fir },
            (false, true) => match h {
                0..45 => Spruce,
                45..75 => Pine,
                _ => Scots,
            },
            (false, false) => if h < 80 { Tree } else { Scots },
        },
    }
}

/// What a forest stroke does to `cell`: `Some(None)` clears a tree off a
/// stream's bank, `Some(Some(tree))` plants on open ground or replants a
/// tree, and `None` leaves it - a bank with no tree on it, water, a wall,
/// anything else the map put there.
pub fn lay(map: &MapFile, bounds: CellRect, cell: (i32, i32)) -> Option<Option<CellObject>> {
    if !bounds.contains(cell) {
        return None;
    }
    let held = map.cell(cell.0, cell.1);
    let plantable = held.is_none() || tree(map, cell);
    if !plantable {
        return None;
    }
    if by_stream(map, cell) {
        return tree(map, cell).then_some(None);
    }
    Some(CellObject::prop(species(map, bounds, cell)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map from rows of text: `T` a tree, `W` water, `#` a brick wall,
    /// `.` open ground.
    fn map_of(rows: &[&str], theme: Theme) -> MapFile {
        let mut text = format!("version = 1\nsize = [{}.0, {}.0]\n", rows[0].len().max(8), rows.len().max(8));
        if theme == Theme::Desert {
            text.push_str("theme = \"desert\"\n");
        }
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

    fn bounds(map: &MapFile) -> CellRect {
        super::super::select::field_cells(map.field_size())
    }

    #[test]
    fn a_line_of_water_is_a_stream_and_a_block_is_a_lake() {
        let map = map_of(&["..W.....", "..W.....", "..WW....", "........", "....WW..", "....WW.."], Theme::Grass);
        assert!(is_stream(&map, (2, 0)) && is_stream(&map, (3, 2)), "a river and its bend");
        assert!(!is_stream(&map, (4, 4)) && !is_stream(&map, (5, 5)), "a lake");
        // The bend's inside corner touches the stream only diagonally.
        assert!(by_stream(&map, (3, 1)));
        assert!(!by_stream(&map, (7, 4)), "the lake's surroundings are no stream bank");
    }

    #[test]
    fn the_brush_clears_stream_banks_and_plants_a_lake_to_its_shore() {
        let map = map_of(&["T.W.....", "T.W.....", "TTW.....", "........", "....WW..", "....WW.."], Theme::Grass);
        let b = bounds(&map);
        assert_eq!(lay(&map, b, (1, 1)), None, "an empty bank stays empty");
        assert_eq!(lay(&map, b, (1, 2)), Some(None), "a tree on a bank is cleared");
        assert_eq!(lay(&map, b, (2, 0)), None, "water is never planted");
        assert!(matches!(lay(&map, b, (6, 4)), Some(Some(_))), "a lake's shore is planted");
        let walled = map_of(&["#......."], Theme::Grass);
        assert_eq!(lay(&walled, bounds(&walled), (0, 0)), None, "a wall is left alone");
    }

    #[test]
    fn waterside_species_step_back_from_the_bank() {
        let map = map_of(&["WW......", "WW......"], Theme::Grass);
        let b = bounds(&map);
        for r in 0..8 {
            assert!(matches!(species(&map, b, (2, r.min(2))), Material::Willow | Material::Birch));
        }
        let desert = map_of(&["WW......", "WW......"], Theme::Desert);
        assert_eq!(species(&desert, bounds(&desert), (2, 0)), Material::Palm, "a palm on the oasis bank");
        let far: Vec<Material> = (0..8).map(|r| species(&desert, bounds(&desert), (7, r))).collect();
        assert!(far.iter().all(|m| !matches!(m, Material::Palm | Material::Willow | Material::Birch)), "{far:?}");
    }

    #[test]
    fn the_outer_ring_grows_saplings_and_birch_and_the_inside_does_not() {
        let rows = ["TTTTTTTT"; 8];
        let mut map = map_of(&rows, Theme::Grass);
        let b = bounds(&map);
        // Open a clearing: its rim is the wood's edge.
        map.clear_cell(4, 4);
        assert!(on_edge(&map, b, (3, 3)) && !on_edge(&map, b, (1, 1)));
        assert!(matches!(species(&map, b, (3, 3)), Material::Fir | Material::Birch));
        assert!(!on_edge(&map, b, (0, 0)), "the field's border is no edge");
        let inside: Vec<Material> = [(0, 0), (1, 1), (6, 1), (1, 6), (7, 7)].iter().map(|&c| species(&map, b, c)).collect();
        assert!(inside.iter().all(|m| !matches!(m, Material::Fir | Material::Willow | Material::Palm)), "{inside:?}");
    }

    fn trees(map: &MapFile) -> Vec<((i32, i32), Material)> {
        let mut out: Vec<_> = map.iter_cells().filter_map(|(c, r, o)| o.material().filter(|m| m.is_tree()).map(|m| ((c, r), m))).collect();
        out.sort_by_key(|&(cell, _)| cell);
        out
    }

    #[test]
    fn a_rectangle_of_forest_keeps_the_stream_open_picks_on_release_and_undoes_as_one_step() {
        use super::super::{MapEditor, Shape, Tool};
        // A stream down column 6 of a 14 x 10 field, a tree already on its
        // bank at (5, 0) and a wall the brush must leave.
        let map = map_of(
            &[".....TW.......", "......W.......", "......W.......", "......W.......", "......W.......", "......W....#..", "......W.......", "......W......."],
            Theme::Grass,
        );
        let before = map.clone();
        let mut ed = MapEditor::new(map);
        ed.select_tool(Tool::Forest);
        ed.set_shape(Shape::Rect);
        ed.stroke(&[(0, 0), (13, 7)], false);
        let after = ed.map().clone();
        for r in 0..8 {
            for c in 5..=7 {
                assert!(!tree(&after, (c, r)), "({c}, {r}) is a stream bank or the stream");
            }
            assert_eq!(after.cell(6, r), Some(&CellObject::Water), "the stream is still there");
        }
        assert_eq!(after.cell(11, 5), before.cell(11, 5), "the wall stands");
        // Picked against the finished wood: deep inside it no sapling
        // stands, where during the drag every new tree was on its edge.
        let b = bounds(&after);
        for (cell, m) in trees(&after) {
            assert_eq!(m, species(&after, b, cell), "{cell:?} keeps its pick for the finished wood");
        }
        assert!(trees(&after).len() > 40);
        assert!(ed.undo());
        assert_eq!(trees(ed.map()), trees(&before), "one undo takes the whole stroke back");
    }
}

