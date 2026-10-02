//! The Load list's thumbnails (docs/large-maps-follow-camera.md section 9):
//! each map's picture beside its name, since a large map's name says less
//! about it than its look. Headless; `render.rs` keeps their textures and
//! draws them.
//!
//! **The picture is the map's minimap** (`minimap.rs`): a texel a cell in
//! the minimap's palette, the water as deep as a round on the map has it -
//! not `thumbnail.rs`'s painted field. On the largest shipped map,
//! longwater, a release build makes the minimap in 1.0 ms, where staging
//! the round and painting its 3584 x 2016 pixels on the CPU canvas takes
//! 492 ms, after 88 ms decoding the sheets that canvas needs
//! (`thumbnail::tests::a_load_list_thumbnail_timing`): a page of pictures
//! painted would stall a phone for seconds.
//!
//! **Made lazily, kept by name and revision.** The list makes the pictures
//! of the maps its page shows, at most `MADE_PER_FRAME` a frame, so a page
//! of large maps fills in over a few frames rather than stalling one - the
//! parse is most of a picture's cost (8 ms for longwater in a release
//! build). Each picture is kept with the revision of the text it was made
//! from, the FNV-1a hash of what `map::map_source` reads, which any edit
//! changes: each time the list opens a picture is checked against its
//! map's text before it is shown - a read and a hash, no parse - and made
//! again only when the text has changed. A map that has left the list lets
//! its picture go when it next opens.

use std::collections::{BTreeMap, BTreeSet};

use crate::canvas::BlockImage;
use crate::ground::WaterLayout;
use crate::map::{self, CellObject, MapEntry, MapFile};
use crate::minimap::Minimap;
use crate::Position;

/// The most pictures the Load list makes in one frame: one - longwater's
/// parse and minimap take about 9 ms in a release build on a desktop, and
/// a phone a few times that, a frame's worth or two.
pub const MADE_PER_FRAME: usize = 1;

/// `map`'s minimap (`minimap.rs`): a texel a cell, the floor under its
/// water as deep as a round on it has the water, and the tiles on it.
pub fn minimap_of(map: &MapFile) -> Minimap {
    let (width, height) = map.field_size();
    let painted = |pick: fn(&CellObject) -> bool| -> Vec<Position> {
        map.iter_cells().filter(|(_, _, o)| pick(o)).map(|(c, r, _)| map::cell_to_world(c, r)).collect()
    };
    let water = WaterLayout::build(
        width,
        height,
        &painted(|o| matches!(o, CellObject::Wall { .. } | CellObject::Road)),
        &painted(|o| matches!(o, CellObject::Water)),
    );
    Minimap::of_map(map, |col, row| water.depth_at(map::cell_to_world(col, row)))
}

/// One map's thumbnail.
#[derive(Clone, Debug, PartialEq)]
pub struct Thumb {
    /// The FNV-1a hash of the text the picture was made from.
    pub revision: u64,
    /// The minimap's image - `None` for a map that does not parse.
    pub image: Option<BlockImage>,
    /// The map's field in world pixels: what the image's texels cover
    /// (`minimap::source`) and the shape the list draws it in.
    pub field: (f32, f32),
    /// The map's size in cells, as its `size` key gives it.
    pub cells: (f32, f32),
}

/// The Load list's thumbnails by map name.
#[derive(Clone, Debug, Default)]
pub struct Thumbs {
    made: BTreeMap<String, Thumb>,
    /// The maps checked against their text since the list last opened:
    /// the only ones shown.
    checked: BTreeSet<String>,
}

impl Thumbs {
    /// The list opened on `entries`: every picture is checked against its
    /// map's text again before it is shown, and the pictures of maps no
    /// longer listed go.
    pub fn open(&mut self, entries: &[MapEntry]) {
        self.checked.clear();
        self.made.retain(|name, _| entries.iter().any(|e| &e.name == name));
    }

    /// One frame of the open list showing `names`, in order: each one not
    /// yet checked since the list opened is checked against its map's text
    /// (`read`, `map::map_source` in the builder) and its picture made
    /// again where the text changed - at most `MADE_PER_FRAME` made, after
    /// which the rest wait for the next frame. Whether any picture was
    /// made.
    pub fn update<'a, R, T>(&mut self, names: impl IntoIterator<Item = &'a str>, read: R) -> bool
    where
        R: Fn(&str) -> Result<T, String>,
        T: AsRef<str>,
    {
        let mut made = 0;
        for name in names {
            if self.checked.contains(name) {
                continue;
            }
            let text = read(name).ok();
            let revision = text.as_ref().map_or(0, |t| map::fnv1a(t.as_ref().as_bytes()));
            if self.made.get(name).is_none_or(|thumb| thumb.revision != revision) {
                if made == MADE_PER_FRAME {
                    break;
                }
                made += 1;
                let parsed = text.and_then(|t| MapFile::from_toml_str(t.as_ref()).ok());
                let thumb = match parsed {
                    Some(map) => {
                        let (width, height) = map.field_size();
                        let cells = (width / crate::OBSTACLE_GRID_SIZE, height / crate::OBSTACLE_GRID_SIZE);
                        Thumb { revision, image: Some(minimap_of(&map).image().clone()), field: (width, height), cells }
                    }
                    None => Thumb { revision, image: None, field: (0.0, 0.0), cells: (0.0, 0.0) },
                };
                self.made.insert(name.to_string(), thumb);
            }
            self.checked.insert(name.to_string());
        }
        made > 0
    }

    /// `name`'s thumbnail, once checked since the list opened.
    pub fn get(&self, name: &str) -> Option<&Thumb> {
        self.made.get(name).filter(|_| self.checked.contains(name))
    }

    /// Every thumbnail checked since the list opened, by name.
    pub fn shown(&self) -> impl Iterator<Item = (&str, &Thumb)> {
        self.made.iter().filter(|(name, _)| self.checked.contains(*name)).map(|(name, thumb)| (name.as_str(), thumb))
    }

    /// How many pictures are kept, shown or not.
    pub fn kept(&self) -> usize {
        self.made.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn entry(name: &str) -> MapEntry {
        MapEntry { name: name.to_string(), on_disk: true }
    }

    const SMALL: &str = "version = 1\nsize = [20, 10]\ncells.\"3,3\" = { kind = \"water\" }\n";
    const WIDE: &str = "version = 1\nsize = [80, 45]\ncells.\"40,20\" = { kind = \"wall\", material = \"iron\" }\n";

    /// A pretend `maps/` the test edits, and the reads it was asked for.
    struct Disk {
        files: RefCell<BTreeMap<String, String>>,
        reads: RefCell<Vec<String>>,
    }

    impl Disk {
        fn new(files: &[(&str, &str)]) -> Disk {
            Disk {
                files: RefCell::new(files.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect()),
                reads: RefCell::new(Vec::new()),
            }
        }

        fn read(&self, name: &str) -> Result<String, String> {
            self.reads.borrow_mut().push(name.to_string());
            self.files.borrow().get(name).cloned().ok_or_else(|| format!("no map named {name}"))
        }
    }

    #[test]
    fn a_page_is_made_one_picture_a_frame_and_kept_by_name_and_revision() {
        let disk = Disk::new(&[("a", SMALL), ("b", WIDE), ("c", SMALL)]);
        let mut thumbs = Thumbs::default();
        let entries = [entry("a"), entry("b"), entry("c")];
        thumbs.open(&entries);
        let page = ["a", "b"];
        assert!(thumbs.update(page, |n| disk.read(n)));
        assert!(thumbs.get("a").is_some() && thumbs.get("b").is_none(), "one a frame");
        assert!(thumbs.update(page, |n| disk.read(n)));
        let b = thumbs.get("b").expect("made on the next frame");
        assert_eq!((b.cells, b.field), ((80.0, 45.0), (2560.0, 1440.0)));
        let image = b.image.as_ref().expect("a picture");
        assert_eq!((image.width, image.height), (81, 46), "a texel a cell, the edge's half cells too");
        assert!(thumbs.get("c").is_none(), "off the page: never made");
        // Nothing left to make: the next frame reads nothing.
        disk.reads.borrow_mut().clear();
        assert!(!thumbs.update(page, |n| disk.read(n)));
        assert!(disk.reads.borrow().is_empty());
        // Opened again: checked by reading, kept without a parse while the
        // text is the same - both pictures in one frame, the same stamps.
        let stamps: Vec<u64> = page.iter().map(|n| thumbs.get(n).unwrap().image.as_ref().unwrap().stamp).collect();
        thumbs.open(&entries);
        assert!(thumbs.get("a").is_none(), "not shown before it is checked");
        assert!(!thumbs.update(page, |n| disk.read(n)), "nothing made");
        let again: Vec<u64> = page.iter().map(|n| thumbs.get(n).unwrap().image.as_ref().unwrap().stamp).collect();
        assert_eq!(stamps, again);
        // An edit to a map's text makes its picture again.
        disk.files.borrow_mut().insert("a".to_string(), WIDE.to_string());
        thumbs.open(&entries);
        assert!(thumbs.update(page, |n| disk.read(n)));
        assert_eq!(thumbs.get("a").unwrap().cells, (80.0, 45.0));
        assert_ne!(thumbs.get("a").unwrap().image.as_ref().unwrap().stamp, stamps[0]);
    }

    #[test]
    fn a_map_that_does_not_parse_has_no_picture_and_a_map_off_the_list_lets_its_go() {
        let disk = Disk::new(&[("bad", "size = [oops"), ("good", SMALL)]);
        let mut thumbs = Thumbs::default();
        thumbs.open(&[entry("bad"), entry("good")]);
        thumbs.update(["bad", "good", "gone"], |n| disk.read(n));
        thumbs.update(["bad", "good", "gone"], |n| disk.read(n));
        thumbs.update(["bad", "good", "gone"], |n| disk.read(n));
        assert!(thumbs.get("bad").is_some_and(|t| t.image.is_none()), "checked, no picture");
        assert!(thumbs.get("gone").is_some_and(|t| t.image.is_none()), "unreadable: checked, no picture");
        assert_eq!(thumbs.shown().filter(|(_, t)| t.image.is_some()).count(), 1);
        thumbs.open(&[entry("good")]);
        assert_eq!(thumbs.kept(), 1, "only the listed map's picture is kept");
    }

    #[test]
    fn a_shipped_maps_picture_shows_its_water_by_depth_and_its_walls() {
        use crate::minimap::Class;
        let map = map::open_map("longwater").expect("shipped");
        let minimap = minimap_of(&map);
        let classes: Vec<Class> = (0..46).flat_map(|r| (0..81).map(move |c| (c, r))).filter_map(|(c, r)| minimap.class_at(c, r)).collect();
        assert!(classes.contains(&Class::Deep) && classes.contains(&Class::Shallow), "a lake has open water and fords");
        assert!(classes.iter().any(|c| matches!(c, Class::Brick | Class::Iron | Class::Wood)), "and walls");
    }
}
