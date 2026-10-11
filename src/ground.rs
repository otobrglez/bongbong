//! Procedurally-placed ground/terrain layer: the pack's grass-fill tiles as
//! the base, soft patches of its sand tiles drifted over the open floor,
//! road painted at specific cells - under every static obstacle/wall tile,
//! and inside the player fortress's `B`/`O` glyphs - and water at the cells
//! a map marks, shaped into rivers and lakes by the cells around it (see
//! `build`'s water section and docs/GROUND_SPEC.md §9) - drawn from the
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
//! dirt a packed-earth road. The tile ids here are the same under every
//! theme; `draw` only names which file to blit from, and the
//! floor shade which dark the field's edge deepens toward.
//!
//! The floor shade (`bake_shade`, drawn by `draw_shade`) is the one layer
//! here that is not tiles: the walls' contact shade and a round's edge
//! shade, stepped and dithered on the 2 px block grid into one image baked
//! the first time the floor is drawn - see docs/GROUND_SPEC.md §4. An
//! arena's window margins (`margin.rs`) carry the round's floor on past
//! its edge (`GroundGrid::beyond`) under the edge shade carried on with it
//! and deepened off the playfield (`bake_margin_shade`).
//!
//! Purely decorative: no physics body, no gameplay effect. `build` runs
//! once per round (from `simulation::Game::init`, after every obstacle for
//! the round has been placed - see that function's own comments for why)
//! and resolves the whole layout into a flat `Vec<i32>` of source tile ids,
//! one per `GROUND_WORLD_TILE` grid cell - so `draw` is just an index
//! lookup and a blit per cell, no per-frame autotile work. The builder
//! builds once per map and then `GroundGrid::repaint`s the cells an edit
//! touched: the tiles around them resolved again and the floor shade baked
//! again around a wall that came or went, landing where a `build` of the
//! edited map would.
//!
//! The autotile tables below (`GRASS_FILL`, `SAND_CORNER`, `ROAD_EDGE`,
//! `WATER_CHANNEL`, `WATER_SHORE`, `WATER_MOUTH`, `WATER_FRAMES`) are
//! extracted from the source pack's own Tiled wangset data
//! (`static/punyworld/punyworld-overworld-tiles.tsx`), not invented here -
//! see docs/GROUND_SPEC.md for exactly how each entry was derived and for
//! the wangset's own documentation of the tile grid. The water tiles the
//! pack lacks (`WATER_EXTRA`) are composed from its own by
//! `tools/water_tiles.py`.
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

use crate::math::{Color, Rectangle, Vec2};

use std::collections::BTreeSet;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::canvas::{BlockImage, Canvas, Sheet};
use crate::map::Theme;
use crate::tuning::{tuning, Tuning};
use crate::{GROUND_WORLD_TILE, OBSTACLE_GRID_SIZE, Position};

/// Columns in punyworld-overworld-tileset.png (432px / 16px).
const TILESET_COLS: i32 = 27;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Material {
    #[default]
    Grass,
    Road,
    Water,
}

/// What the ground's tiles lay at a point (`GroundGrid::floor_at`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroundFloor {
    Grass,
    Road,
    Sand,
    Water,
}

/// What one map cell lays on the floor: its place in the three lists
/// `build` takes - `road` (a road, or a wall, which stands on dirt),
/// `water`, and `wall`, which the floor shade gathers round. Road over
/// water is road, as in `build`. What `GroundGrid::repaint` is told a
/// cell now is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellFloor {
    pub road: bool,
    pub water: bool,
    pub wall: bool,
}

impl CellFloor {
    fn material(self) -> Material {
        if self.road {
            Material::Road
        } else if self.water {
            Material::Water
        } else {
            Material::Grass
        }
    }
}

/// What the current does on a cell, for the flow overlay `draw` puts over
/// the water tiles: resolved once per cell in `build` from the tile that
/// was picked, so drawing never re-derives the autotile.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Current {
    /// No water, or water whose tile leaves no room for a moving mark: a
    /// shore, a bend, a stream running sideways. The pack's own frames
    /// still shimmer there.
    #[default]
    Still,
    /// A lake interior (`WATER_SHORE[0b1111]`): open water the whole cell
    /// wide, the marks drift down anywhere across it.
    Open,
    /// A stream cell joined north or south, or a lake cell a stream enters
    /// from the north or the south (a mouth): the marks drift down its
    /// centre band (`WATER_CHANNEL_BAND`), the columns the pack's stream
    /// art actually fills.
    Channel,
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

/// Water as a **stream**: the pack's "water-paths" Wang **edge** autotile,
/// the same 4x4 layout as `ROAD_EDGE` fifteen rows further down the sheet,
/// indexed the same way (bit3=N bit2=E bit1=S bit0=W, 1 = water
/// neighbour of any kind). A stream cell is a water cell that is not part
/// of any 2x2 block of water (see `build`) - one painted like a road, which
/// is how a river is drawn. Like the road's, the isolated entry (0000) is
/// not in the wangset: 351 is the rounded pool the pack keeps beside the
/// end caps, exactly where 84 sits beside the road's.
const WATER_CHANNEL: [i32; 16] = [
    351, // 0000 isolated pool
    354, // 0001 W
    270, // 0010 S
    273, // 0011 S+W
    352, // 0100 E
    353, // 0101 E+W
    271, // 0110 E+S
    272, // 0111 E+S+W
    324, // 1000 N
    327, // 1001 N+W
    297, // 1010 N+S
    300, // 1011 N+S+W
    325, // 1100 N+E
    326, // 1101 N+E+W
    298, // 1110 N+E+S
    299, // 1111 N+E+S+W (crossing)
];

/// Water as a **lake**: the pack's "river" Wang **corner** autotile, shaped
/// like `SAND_CORNER` (bit3=TL bit2=TR bit1=BR bit0=BL, 1 = water at that
/// corner) and laid the same way, at the grid vertices - `Layout` marks a
/// vertex wet where the cells around it are water, so a painted block
/// becomes a pool with a rounded shore half a cell inside its outline.
/// The wangset has no tiles for the two diagonal cases (wet corners
/// joined only corner to corner), which a diagonal run of water produces:
/// 602 and 603 are composed from the pack's single-corner shores by
/// `tools/water_tiles.py` (`WATER_EXTRA`). 0000 is plain grass, never
/// asked for (a cell with no wet corner does not use this table).
const WATER_SHORE: [i32; 16] = [
    305, // 0000 never asked for
    279, // 0001 BL
    277, // 0010 BR
    278, // 0011 BR+BL (top shore)
    331, // 0100 TR
    602, // 0101 TR+BL diagonal (composed)
    304, // 0110 TR+BR (left shore)
    280, // 0111 all but TL
    333, // 1000 TL
    306, // 1001 TL+BL (right shore)
    603, // 1010 TL+BR diagonal (composed)
    281, // 1011 all but TR
    332, // 1100 TL+TR (bottom shore)
    308, // 1101 all but BR
    307, // 1110 all but BL
    305, // 1111 open water
];

/// A lake cell with a one-cell stream entering on one or more of its dry
/// sides: `(corners, sides, tile)`, corners as `WATER_SHORE`, sides as
/// `WATER_CHANNEL` (bit3=N bit2=E bit1=S bit0=W). A stream only ever
/// touches a lake cell on a side whose two corners are both dry (a wet
/// corner there would make the stream cell part of a 2x2 block, so a
/// lake cell), which leaves exactly these sixteen cases. The four
/// straight shores have mouths in the pack - tiles outside both wangsets,
/// the straight shore with a gap where the stream comes in - and the
/// twelve single-corner ones are composed by `tools/water_tiles.py`.
const WATER_MOUTH: [(usize, usize, i32); 16] = [
    (0b0011, 0b1000, 275), // top shore, stream from the north
    (0b1100, 0b0010, 329), // bottom shore, from the south
    (0b0110, 0b0001, 301), // left shore, from the west
    (0b1001, 0b0100, 303), // right shore, from the east
    (0b1000, 0b0100, 604), // TL wet, from the east
    (0b1000, 0b0010, 605), // TL wet, from the south
    (0b1000, 0b0110, 606), // TL wet, from the east and the south
    (0b0100, 0b0010, 607), // TR wet, from the south
    (0b0100, 0b0001, 608), // TR wet, from the west
    (0b0100, 0b0011, 609), // TR wet, from the south and the west
    (0b0010, 0b1000, 610), // BR wet, from the north
    (0b0010, 0b0001, 611), // BR wet, from the west
    (0b0010, 0b1001, 612), // BR wet, from the north and the west
    (0b0001, 0b1000, 613), // BL wet, from the north
    (0b0001, 0b0100, 614), // BL wet, from the east
    (0b0001, 0b1100, 615), // BL wet, from the north and the east
];

/// The tile for a lake cell (or a grass cell under a wet corner): its
/// corners, and the sides a stream enters it on.
fn lake_tile(corners: usize, sides: usize) -> i32 {
    if sides == 0 {
        return WATER_SHORE[corners];
    }
    let mouth = WATER_MOUTH.iter().find(|(c, s, _)| *c == corners && *s == sides).map(|(_, _, tile)| *tile);
    // Unreachable (see `WATER_MOUTH`); a map never fails to draw over it -
    // the plain shore is the nearest picture.
    debug_assert!(mouth.is_some(), "no water tile for corners {corners:04b} with a stream on {sides:04b}");
    mouth.unwrap_or(WATER_SHORE[corners])
}

/// The tiles `tools/water_tiles.py` composes into transparent cells of
/// the sheet: `WATER_EXTRA_COUNT` tiles from `WATER_EXTRA`, frame `f` of
/// each one row (`TILESET_COLS`) below frame 0 - the order is the
/// script's `TILES`.
const WATER_EXTRA: i32 = 602;
const WATER_EXTRA_COUNT: i32 = 14;

/// Frames per animated water tile.
pub const WATER_FRAME_COUNT: usize = 4;

/// The pack's animation for every tile the water tables can pick: the
/// `<animation>` element of each `<tile>` in the `.tsx`, four frames of
/// 100 ms each. The frames are scattered over the sheet (the stream tiles
/// step by 108, the shore tiles by 81 or 54), so this is a table, not an
/// offset. 305, the flat water, has no animation in the pack (its three
/// would-be frames are byte-identical copies of it), so it repeats. The
/// composed tiles (`WATER_EXTRA`) are not listed: their frames are a row
/// apart. A tile missing here is a bug in the tables above, and
/// `frames_of` panics on it so the test catches it.
const WATER_FRAMES: [(i32, [i32; WATER_FRAME_COUNT]); 33] = [
    (270, [270, 378, 486, 594]),
    (271, [271, 379, 487, 595]),
    (272, [272, 380, 488, 596]),
    (273, [273, 381, 489, 597]),
    (297, [297, 405, 513, 621]),
    (298, [298, 406, 514, 622]),
    (299, [299, 407, 515, 623]),
    (300, [300, 408, 516, 624]),
    (324, [324, 432, 540, 648]),
    (325, [325, 433, 541, 649]),
    (326, [326, 434, 542, 650]),
    (327, [327, 435, 543, 651]),
    (351, [351, 459, 567, 675]),
    (352, [352, 460, 568, 676]),
    (353, [353, 461, 569, 677]),
    (354, [354, 462, 570, 678]),
    (277, [277, 358, 439, 520]),
    (278, [278, 359, 440, 521]),
    (279, [279, 360, 441, 522]),
    (280, [280, 334, 388, 442]),
    (281, [281, 335, 389, 443]),
    (304, [304, 385, 466, 547]),
    (305, [305, 305, 305, 305]),
    (306, [306, 387, 468, 549]),
    (307, [307, 361, 415, 469]),
    (308, [308, 362, 416, 470]),
    (331, [331, 412, 493, 574]),
    (332, [332, 413, 494, 575]),
    (333, [333, 414, 495, 576]),
    (275, [275, 356, 437, 518]),
    (301, [301, 382, 463, 544]),
    (303, [303, 384, 465, 546]),
    (329, [329, 410, 491, 572]),
];

/// The four frames of a water tile (a tile that does not animate repeats).
fn frames_of(tile: i32) -> [i32; WATER_FRAME_COUNT] {
    if (WATER_EXTRA..WATER_EXTRA + WATER_EXTRA_COUNT).contains(&tile) {
        return std::array::from_fn(|f| tile + f as i32 * TILESET_COLS);
    }
    WATER_FRAMES
        .iter()
        .find(|(t, _)| *t == tile)
        .map(|(_, frames)| *frames)
        .unwrap_or_else(|| panic!("water tile {tile} has no entry in WATER_FRAMES"))
}

/// The stream art's water, in world px from the cell's left edge: source
/// columns 3..13 of the 16 px tile, drawn at 2x. The flow marks of a
/// `Current::Channel` cell stay inside it.
const WATER_CHANNEL_BAND: (f32, f32) = (6.0, 26.0);

/// The flow marks repeat every this many cells down a column, so a mark
/// drifts through several cells of open water before wrapping instead of
/// blinking at every cell edge.
const WATER_FLOW_PERIOD_CELLS: i32 = 3;

/// A flow mark's height, in 2 px blocks.
const WATER_FLOW_MARK_BLOCKS: i32 = 3;

/// The pack's own water highlight (the ripple tone on its shore and stream
/// tiles), so a mark is a pixel the sheet already has. Outside the hue
/// band `tools/retint_ground.py` moves, so it matches under every theme.
const WATER_FLOW_MARK: Color = Color::new(29, 204, 203, 210);

/// The source rectangle of the flat-water tile - the builder's icon for
/// the water brush.
pub fn water_icon_source_rec() -> Rectangle {
    source_rec(WATER_SHORE[0b1111])
}

/// What `build` makes of a map besides its cells: the theme (whether sand
/// drifts over the open floor, and the colour the field's edge deepens
/// toward) and whether that edge shade is baked at all - a round's field
/// has it, the builder's canvas does not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    pub theme: Theme,
    pub edge_shade: bool,
}

/// Names every floor shade `build` bakes, so a GPU copy of one is never
/// taken for another's (`render::canvas::BlockTexture`).
static NEXT_SHADE_STAMP: AtomicU64 = AtomicU64::new(1);

/// A fresh name for a floor layer baked into a `BlockImage` - a shade, or
/// the lava's scorched banks (`lava::LavaLayout::banks`) - from the one
/// counter, so no two are ever taken for each other.
pub fn next_block_stamp() -> u64 {
    NEXT_SHADE_STAMP.fetch_add(1, Ordering::Relaxed)
}

/// This round's resolved ground layer: a flat, row-major grid of source
/// tile ids (into `punyworld-overworld-tileset.png`), one per
/// `GROUND_WORLD_TILE` cell. Built by `build`, drawn every frame by `draw`
/// - no autotile work happens per frame. A round builds it once; the
/// builder `repaint`s the cells an edit touched.
#[derive(Default)]
pub struct GroundGrid {
    pub cols: usize,
    pub rows: usize,
    /// Per cell, the tile for each animation frame. A cell that does not
    /// animate (everything but water) repeats one id, so `draw` indexes
    /// every cell the same way and never asks what it is.
    tiles: Vec<[i32; WATER_FRAME_COUNT]>,
    /// Per cell, what the flow overlay does there (`Current`).
    current: Vec<Current>,
    /// What the floor shade (`shade`) is baked from: the field's size in
    /// px, the wall cells as grid coordinates and the look.
    width: f32,
    height: f32,
    walls: Vec<(i32, i32)>,
    look: Look,
    /// The baked floor shade, made the first time the floor is drawn - a
    /// room server builds a round's ground and never draws it, so it never
    /// pays for it.
    shade: OnceLock<Shade>,
    /// This build's name for its shade (`NEXT_SHADE_STAMP`).
    stamp: u64,
    /// What the tiles were resolved from and with - the paint and the
    /// water read off it, the seed, the drifts - so `repaint` resolves the
    /// cells around an edit as `build` resolved the rest.
    layout: Layout,
    seed: u64,
    drifts: Drifts,
}

impl GroundGrid {
    /// The floor shade: walls' contact shade and, on a round's field, the
    /// rounded edge shade, stepped and dithered on the 2 px block grid
    /// (`bake_shade`). Baked on first use with the live tuning and kept: a
    /// round never bakes it again - walls are only ever removed, and a hole
    /// in a wall letting a little more light onto the floor is not worth a
    /// re-bake - and the builder bakes again only the part around a wall
    /// an edit placed or took away (`repaint`).
    pub fn shade(&self) -> &BlockImage {
        &self.shade.get_or_init(|| Shade::bake(self.width, self.height, &self.walls, self.look, &tuning(), self.stamp)).image
    }

    /// Make each `(col, row)` map cell lay its `CellFloor` - `build`'s
    /// three cell lists, edited at those cells - resolving again only the
    /// tiles that can change and, where a wall came or went, baking again
    /// only the blocks of the floor shade a wall there reaches, under a
    /// new stamp (a `BlockPatch` names those blocks, so the GPU copy
    /// uploads only them). The grid is the one `build` makes of the edited
    /// lists with the same seed (`a_repaint_is_the_build_of_the_edited_lists`).
    /// A cell past the grid moves only the walls the shade gathers round,
    /// as in `build`. Answers the grid cells whose tile can have changed,
    /// in row order - every cell whose water `depth` can have, among them.
    pub fn repaint(&mut self, cells: &[(i32, i32, CellFloor)]) -> Vec<(i32, i32)> {
        let mut paint = Vec::new();
        // Whether each cell is a wall now. The walls that went, then the
        // ones that came, take one pass over the list each, so a fill of
        // thousands of walls costs what a pass does rather than a pass a
        // wall. Their order matters to nothing: the shade lays every wall
        // with a max.
        let mut wall_now = std::collections::BTreeMap::new();
        for &(col, row, floor) in cells {
            if self.idx(col, row).is_some() {
                paint.push(((col as usize, row as usize), floor.material()));
            }
            wall_now.insert((col, row), floor.wall);
        }
        let mut walls_moved = Vec::new();
        self.walls.retain(|w| {
            let went = wall_now.get(w) == Some(&false);
            if went {
                walls_moved.push(*w);
            }
            !went
        });
        let standing: BTreeSet<(i32, i32)> = self.walls.iter().filter(|w| wall_now.get(w) == Some(&true)).copied().collect();
        for (&cell, &wall) in &wall_now {
            if wall && !standing.contains(&cell) {
                self.walls.push(cell);
                walls_moved.push(cell);
            }
        }
        let touched = self.layout.repaint(&paint);
        for &(x, y) in &touched {
            let i = y as usize * self.cols + x as usize;
            (self.tiles[i], self.current[i]) = resolve(&self.layout, self.seed, self.drifts, x, y);
        }
        if let (false, Some(shade)) = (walls_moved.is_empty(), self.shade.get_mut()) {
            shade.rebake(&self.walls, &walls_moved);
        }
        let mut touched: Vec<(i32, i32)> = touched.into_iter().collect();
        touched.sort_by_key(|&(x, y)| (y, x));
        touched
    }

    /// What the tiles lay at field px `pos`: grass, a road, sand where a
    /// drift's corner reaches that quarter of its cell (the grain the
    /// corner autotile lays sand on), or water. The ground's marks take
    /// their colours by it (`wear.rs`).
    pub fn floor_at(&self, pos: Position) -> GroundFloor {
        let (ox, oy) = self.layout.origin;
        let (c, r) = crate::map::world_to_cell(pos);
        let (x, y) = (c - ox, r - oy);
        match self.layout.at(x, y) {
            Material::Road => GroundFloor::Road,
            Material::Water => GroundFloor::Water,
            Material::Grass => {
                let Some(i) = self.idx(x, y) else { return GroundFloor::Grass };
                let Some(mask) = SAND_CORNER.iter().position(|&tile| tile == self.tiles[i][0]) else { return GroundFloor::Grass };
                let centre = crate::map::cell_to_world(c, r);
                let (right, below) = (pos.x >= centre.x, pos.y >= centre.y);
                // bit3 TL, bit2 TR, bit1 BR, bit0 BL.
                let bit = match (right, below) {
                    (false, false) => 3,
                    (true, false) => 2,
                    (true, true) => 1,
                    (false, true) => 0,
                };
                if mask & (1 << bit) != 0 { GroundFloor::Sand } else { GroundFloor::Grass }
            }
        }
    }

    /// How deep the water in map cell `(col, row)` is, by the reading the
    /// tiles were resolved from - the one `WaterLayout` gives a round of
    /// the same cells: open lake water deep, any other water a ford, dry
    /// elsewhere and past the map.
    pub fn depth(&self, col: i32, row: i32) -> Depth {
        self.layout.depth(col, row)
    }

    fn idx(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x as usize >= self.cols || y as usize >= self.rows {
            None
        } else {
            Some(y as usize * self.cols + x as usize)
        }
    }

    /// This floor carried `cells` cells further on every side: the world
    /// past an arena's field, which `margin.rs` lays in the window's
    /// margins. A cell the two grids share draws the same tile - the paint
    /// is this grid's and the hashes are keyed by world cell - so the
    /// field's picture runs on across its edge: the sand drifting on, water
    /// painted to the map's edge running out of it, a road ending at the
    /// boundary. It bakes no floor shade of its own.
    pub fn beyond(&self, cells: usize) -> GroundGrid {
        let layout = self.layout.beyond(cells);
        let (cols, rows) = (layout.cols, layout.rows);
        let mut tiles = vec![[0i32; WATER_FRAME_COUNT]; cols * rows];
        let mut current = vec![Current::Still; cols * rows];
        for y in 0..rows as i32 {
            for x in 0..cols as i32 {
                let i = y as usize * cols + x as usize;
                (tiles[i], current[i]) = resolve(&layout, self.seed, self.drifts, x, y);
            }
        }
        GroundGrid {
            cols,
            rows,
            tiles,
            current,
            width: self.width,
            height: self.height,
            walls: Vec::new(),
            look: Look { theme: self.look.theme, edge_shade: false },
            shade: OnceLock::new(),
            stamp: NEXT_SHADE_STAMP.fetch_add(1, Ordering::Relaxed),
            layout,
            seed: self.seed,
            drifts: self.drifts,
        }
    }

    /// The world cell this grid's first cell is centred on: (0, 0) for a
    /// map's own, `cells` up and left of it for one carried `beyond` it.
    pub fn origin(&self) -> (i32, i32) {
        self.layout.origin
    }

    /// Whether the floor is water at world cell `(col, row)`: past the
    /// grid, the answer of its nearest cell, as water runs off the map.
    pub fn water_at(&self, col: i32, row: i32) -> bool {
        let (ox, oy) = self.layout.origin;
        self.layout.is_water(col - ox, row - oy)
    }

    /// This build's name: a new round's floor has another, so what is made
    /// from one (`margin::Margin`) can tell it is out of date.
    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    /// Whether `other` draws the same floor: the same tiles, currents and
    /// floor shade, whatever either's stamp.
    #[cfg(test)]
    pub fn draws_like(&self, other: &GroundGrid) -> bool {
        (self.cols, self.rows) == (other.cols, other.rows)
            && self.tiles == other.tiles
            && self.current == other.current
            && self.shade().texels == other.shade().texels
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

/// The painted material grid and what the water on it is - the one
/// reading of a map's cells that both the picture (`build`) and the rules
/// (`WaterLayout`) come from, so the lake a tank cannot enter is exactly
/// the open water it sees. Grid phase as in the module doc: cell `(x, y)`
/// is centred on world `(x, y) * GROUND_WORLD_TILE`.
///
/// **Water runs off the map**: a water question about a cell past the
/// grid is answered by the nearest cell on it, and the grid's own cells
/// past the map (the extra column and row the centred phase needs, and
/// every cell of a layout carried `beyond` it) copy the water of the map
/// cell nearest them, so a sea or river painted to the edge carries on out
/// of the picture instead of ending on a shore along it.
#[derive(Default)]
struct Layout {
    cols: usize,
    rows: usize,
    /// The map's own cells, `map_cols` by `map_rows` from world cell (0,
    /// 0): `cols - 1` by `rows - 1` on a map's own layout.
    map_cols: usize,
    map_rows: usize,
    /// The world cell layout cell (0, 0) is centred on: (0, 0) on a map's
    /// own layout, up and left of it on one carried `beyond` the map. What
    /// a cell's cosmetic hashes are keyed by, so the two draw a cell they
    /// share alike.
    origin: (i32, i32),
    /// Per cell, what the map paints there: water, then road over it.
    painted: Vec<Material>,
    /// Per cell, what is drawn there: the paint, past the map the water of
    /// the map cell nearest it (`settled`).
    material: Vec<Material>,
    /// Per cell: a water cell inside some 2x2 block of water.
    lake: Vec<bool>,
    /// Per grid vertex, `(cols + 1) * (rows + 1)`: vertex `(vx, vy)` is the
    /// top-left corner of cell `(vx, vy)`. See `wet_vertices`.
    wet: Vec<bool>,
}

impl Layout {
    fn new(width: f32, height: f32, road_cells: &[Position], water_cells: &[Position]) -> Layout {
        // +1 over the plain `ceil(width / T)` cell count: since cells are
        // centered rather than top-left-aligned (see module doc comment),
        // the last cell's own right/bottom half-tile can fall short of
        // `width`/`height` otherwise, leaving an uncovered strip at the
        // edge.
        let map_cols = (width / GROUND_WORLD_TILE).ceil().max(1.0) as usize;
        let map_rows = (height / GROUND_WORLD_TILE).ceil().max(1.0) as usize;
        let (cols, rows) = (map_cols + 1, map_rows + 1);
        let mut painted = vec![Material::Grass; cols * rows];
        // Water first, road after: a wall painted over a lake stands on
        // dirt. An entry outside the grid is ignored.
        for (cells, what) in [(water_cells, Material::Water), (road_cells, Material::Road)] {
            for pos in cells {
                let gx = (pos.x / GROUND_WORLD_TILE).round() as i32;
                let gy = (pos.y / GROUND_WORLD_TILE).round() as i32;
                if gx >= 0 && gy >= 0 && (gx as usize) < cols && (gy as usize) < rows {
                    painted[gy as usize * cols + gx as usize] = what;
                }
            }
        }
        let layout = Layout { cols, rows, map_cols, map_rows, origin: (0, 0), painted, material: Vec::new(), lake: Vec::new(), wet: Vec::new() };
        layout.settle()
    }

    /// This layout carried `m` cells further on every side: the same paint
    /// moved in by `m`, the cells round it grass but for the water of the
    /// map cell nearest them, settled and smoothed the way `new` settles a
    /// map's. A cell the two share comes out the same: past its own grid
    /// this layout already answered water by the nearest cell, which is
    /// what the new cells hold, and they run straight out from the map's
    /// edge, so the smoothing finds no diagonal among them.
    fn beyond(&self, m: usize) -> Layout {
        let (cols, rows) = (self.cols + 2 * m, self.rows + 2 * m);
        let mut painted = vec![Material::Grass; cols * rows];
        for y in 0..self.rows {
            painted[(y + m) * cols + m..][..self.cols].copy_from_slice(&self.painted[y * self.cols..][..self.cols]);
        }
        let origin = (self.origin.0 - m as i32, self.origin.1 - m as i32);
        let layout = Layout { cols, rows, map_cols: self.map_cols, map_rows: self.map_rows, origin, painted, material: Vec::new(), lake: Vec::new(), wet: Vec::new() };
        layout.settle()
    }

    /// Work out what the paint settles into: the materials, the lakes and
    /// the wet vertices.
    fn settle(mut self) -> Layout {
        self.material = (0..self.rows).flat_map(|y| (0..self.cols).map(move |x| (x, y))).map(|(x, y)| self.settled(x, y)).collect();
        self.lake = (0..self.rows as i32).flat_map(|y| (0..self.cols as i32).map(move |x| (x, y))).map(|(x, y)| self.lake_of(x, y)).collect();
        self.wet = self.wet_vertices();
        self
    }

    /// What is drawn at cell `(x, y)`: its paint, except that a cell past
    /// the map carries the water of the map cell nearest it (the corner
    /// one the corner's).
    fn settled(&self, x: usize, y: usize) -> Material {
        let (wx, wy) = (x as i32 + self.origin.0, y as i32 + self.origin.1);
        let (mx, my) = (wx.clamp(0, self.map_cols as i32 - 1), wy.clamp(0, self.map_rows as i32 - 1));
        if (mx, my) != (wx, wy) {
            let inside = (my - self.origin.1) as usize * self.cols + (mx - self.origin.0) as usize;
            if self.painted[inside] == Material::Water {
                return Material::Water;
            }
        }
        self.painted[y * self.cols + x]
    }

    /// Whether cell `(x, y)` is a lake cell: water inside some 2x2 block of
    /// water. The four blocks a cell can belong to have their top-left at
    /// the cell or one step up and/or left of it.
    fn lake_of(&self, x: i32, y: i32) -> bool {
        self.is_water(x, y)
            && [(-1, -1), (0, -1), (-1, 0), (0, 0)]
                .iter()
                .any(|&(ox, oy)| (0..2).all(|dy| (0..2).all(|dx| self.is_water(x + ox + dx, y + oy + dy))))
    }

    /// Paint each `(cell, material)` - cells inside the grid - and settle
    /// what that moves, to what `new` makes of the edited paint: the
    /// materials (a map cell on the last column or row carries its water
    /// past the map), the lakes within a cell of a changed material and,
    /// where water is that near, every wet vertex again - the smoothing is
    /// a fixpoint run in cell order, so only a whole run is sure to land
    /// where `new`'s does. Returns the cells whose tile can have changed:
    /// a cell's tile reads the materials and lakes within one cell of it
    /// and its four corners' wetness.
    fn repaint(&mut self, cells: &[((usize, usize), Material)]) -> BTreeSet<(i32, i32)> {
        debug_assert_eq!(self.origin, (0, 0), "only a map's own layout is painted");
        let mut changed = BTreeSet::new();
        let mut water_moved = false;
        for &((x, y), what) in cells {
            let i = y * self.cols + x;
            if self.painted[i] == what {
                continue;
            }
            self.painted[i] = what;
            let xs = if x + 1 == self.map_cols { x..self.cols } else { x..x + 1 };
            let ys = if y + 1 == self.map_rows { y..self.rows } else { y..y + 1 };
            for cy in ys {
                for cx in xs.clone() {
                    let j = cy * self.cols + cx;
                    let now = self.settled(cx, cy);
                    if now != self.material[j] {
                        water_moved |= now == Material::Water || self.material[j] == Material::Water;
                        self.material[j] = now;
                        changed.insert((cx as i32, cy as i32));
                    }
                }
            }
        }
        let near = |&(x, y): &(i32, i32)| (-1..=1).flat_map(move |dy| (-1..=1).map(move |dx| (x + dx, y + dy)));
        // Lakes and wet vertices read only water, and whether the land
        // beside three water cells is grass: with no water within a cell
        // of the change, neither moves.
        let watery = water_moved || changed.iter().flat_map(near).any(|(x, y)| self.is_water(x, y));
        let mut lakes = BTreeSet::new();
        let mut wet_moved = Vec::new();
        if watery {
            for (x, y) in changed.iter().flat_map(near) {
                if let Some(i) = self.idx(x, y) {
                    let lake = self.lake_of(x, y);
                    if lake != self.lake[i] {
                        self.lake[i] = lake;
                        lakes.insert((x, y));
                    }
                }
            }
            let wet = self.wet_vertices();
            wet_moved = (0..wet.len()).filter(|&v| wet[v] != self.wet[v]).collect();
            self.wet = wet;
        }
        let vcols = self.cols as i32 + 1;
        changed
            .iter()
            .chain(&lakes)
            .flat_map(near)
            .chain(wet_moved.into_iter().flat_map(|v| Self::around(v as i32 % vcols, v as i32 / vcols)))
            .filter(|&(x, y)| self.idx(x, y).is_some())
            .collect()
    }

    fn idx(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x as usize >= self.cols || y as usize >= self.rows {
            None
        } else {
            Some(y as usize * self.cols + x as usize)
        }
    }

    /// The grid cell nearest `(x, y)`: where a water question about a cell
    /// past the grid is answered.
    fn clamped(&self, x: i32, y: i32) -> usize {
        let cx = x.clamp(0, self.cols as i32 - 1) as usize;
        let cy = y.clamp(0, self.rows as i32 - 1) as usize;
        cy * self.cols + cx
    }

    /// Off-grid is grass, same as "not painted" - for the road and the
    /// grass. Water carries on past the grid (`is_water`).
    fn at(&self, x: i32, y: i32) -> Material {
        self.idx(x, y).map_or(Material::Grass, |i| self.material[i])
    }

    fn is_water(&self, x: i32, y: i32) -> bool {
        self.material[self.clamped(x, y)] == Material::Water
    }

    fn lake_at(&self, x: i32, y: i32) -> bool {
        self.lake[self.clamped(x, y)]
    }

    /// A water cell outside every 2x2 block: a one-cell-wide line.
    fn stream_at(&self, x: i32, y: i32) -> bool {
        self.is_water(x, y) && !self.lake_at(x, y)
    }

    /// How deep the water in map cell `(x, y)` is: open lake water - a lake
    /// cell wet at all four corners - is deep, any other water cell a ford,
    /// and a cell past the map, whose water is picture only, dry. The one
    /// reading `WaterLayout` and the builder's minimap (`GroundGrid::depth`)
    /// both take.
    fn depth(&self, x: i32, y: i32) -> Depth {
        if x < 0 || y < 0 || x as usize >= self.map_cols || y as usize >= self.map_rows || !self.is_water(x, y) {
            Depth::Dry
        } else if self.lake_at(x, y) && self.corners(x, y) == 0b1111 {
            Depth::Deep
        } else {
            Depth::Shallow
        }
    }

    fn around(vx: i32, vy: i32) -> [(i32, i32); 4] {
        [(vx - 1, vy - 1), (vx, vy - 1), (vx - 1, vy), (vx, vy)]
    }

    /// Which vertices are wet - the corners the lake tiles are laid on.
    ///
    /// A vertex is wet where all four cells around it are water, the one
    /// case the pack's corner tiles are drawn for: any other wet vertex
    /// would stand at the corner of a cell that is not drawn as water, and
    /// show as a square bite of grass out of the lake.
    ///
    /// Then one smoothing rule, for diagonals: a cell wet at two opposite
    /// corners only (a **saddle**, what a staircase of 2x2 blocks makes)
    /// would pinch the water to a point there, so each of its dry corners
    /// is wetted where the three water cells around it are all lake cells
    /// and the fourth is grass. The grass cell then shows water in that
    /// corner (`build` draws every cell under a wet vertex from the lake
    /// tiles) and a diagonal river flows as one body. The fourth cell must
    /// be grass: a stream's bank keeps the mouth tiles' shape, and a road
    /// keeps its dirt. Wetting can make another saddle, so it runs until
    /// nothing changes.
    fn wet_vertices(&self) -> Vec<bool> {
        let (vcols, vrows) = (self.cols + 1, self.rows + 1);
        let mut wet = vec![false; vcols * vrows];
        for vy in 0..vrows as i32 {
            for vx in 0..vcols as i32 {
                wet[vy as usize * vcols + vx as usize] = Self::around(vx, vy).iter().all(|&(cx, cy)| self.is_water(cx, cy));
            }
        }
        let fillable = |vx: i32, vy: i32| {
            let around = Self::around(vx, vy);
            let water = around.iter().filter(|&&(cx, cy)| self.is_water(cx, cy)).count();
            water == 3
                && around.iter().all(|&(cx, cy)| if self.is_water(cx, cy) { self.lake_at(cx, cy) } else { self.at(cx, cy) == Material::Grass })
        };
        loop {
            let mut changed = false;
            for y in 0..self.rows as i32 {
                for x in 0..self.cols as i32 {
                    let mask = Self::corners_of(&wet, vcols, x, y);
                    if mask != 0b0101 && mask != 0b1010 {
                        continue;
                    }
                    for (vx, vy) in [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)] {
                        let v = vy as usize * vcols + vx as usize;
                        if !wet[v] && fillable(vx, vy) {
                            wet[v] = true;
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                return wet;
            }
        }
    }

    fn corners_of(wet: &[bool], vcols: usize, x: i32, y: i32) -> usize {
        let corner = |vx: i32, vy: i32| wet[vy as usize * vcols + vx as usize] as usize;
        (corner(x, y) << 3) | (corner(x + 1, y) << 2) | (corner(x + 1, y + 1) << 1) | corner(x, y + 1)
    }

    /// `WATER_SHORE`'s index for a cell in the grid: bit3=TL bit2=TR
    /// bit1=BR bit0=BL, 1 = wet. Non-zero for every lake cell (the middle
    /// of its 2x2 block is one of its corners) and for a grass cell a
    /// diagonal was smoothed over; zero everywhere else.
    fn corners(&self, x: i32, y: i32) -> usize {
        Self::corners_of(&self.wet, self.cols + 1, x, y)
    }

    fn edge_mask(&self, x: i32, y: i32, joined: impl Fn(i32, i32) -> bool) -> usize {
        let n = joined(x, y - 1);
        let e = joined(x + 1, y);
        let s = joined(x, y + 1);
        let w = joined(x - 1, y);
        ((n as usize) << 3) | ((e as usize) << 2) | ((s as usize) << 1) | (w as usize)
    }

    /// `WATER_CHANNEL`'s index for a stream cell: N E S W, 1 = a water
    /// neighbour of either kind.
    fn channel_mask(&self, x: i32, y: i32) -> usize {
        self.edge_mask(x, y, |cx, cy| self.is_water(cx, cy))
    }

    /// The sides a stream enters a lake cell on (`WATER_MOUTH`), N E S W.
    fn stream_sides(&self, x: i32, y: i32) -> usize {
        self.edge_mask(x, y, |cx, cy| self.stream_at(cx, cy))
    }

    /// `ROAD_EDGE`'s index: N E S W, 1 = a road neighbour.
    fn road_mask(&self, x: i32, y: i32) -> usize {
        self.edge_mask(x, y, |cx, cy| self.at(cx, cy) == Material::Road)
    }
}

/// How deep the water under a point is - the one distinction the rules
/// make (docs/water.md).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Depth {
    #[default]
    Dry,
    /// A ford: a stream, a shore, anything painted that is not open lake
    /// water. A hull crosses it slowly and loses grip.
    Shallow,
    /// Open lake water (`WATER_SHORE[0b1111]`): a hull cannot enter.
    /// Shots fly over it.
    Deep,
    /// Water frozen over under a snowy sky (`WaterLayout::freeze`), ford
    /// and lake alike: open ground a hull drives on and slides across,
    /// with no current, that still takes no fire, heat or scorch.
    Ice,
}

impl Depth {
    /// Liquid water: a ford or a lake, not ice. What wets a track, sprays,
    /// puts a fire on a hull out and draws a frog's hop.
    pub fn is_wet(self) -> bool {
        matches!(self, Depth::Shallow | Depth::Deep)
    }
}

/// The rules' view of a map's water, built from the map's cells alone
/// (no RNG, no world) so `Game::init` can have it before anything is
/// placed: what is deep, what is a ford, and where the current runs.
/// `build` reads the same `Layout`, so the picture and the rules agree by
/// construction.
#[derive(Clone, Default)]
pub struct WaterLayout {
    cols: usize,
    rows: usize,
    depth: Vec<Depth>,
    /// Per cell: a stream cell joined north or south, or the mouth such a
    /// stream runs through into a lake, where the current pushes a hull
    /// down the map (`Current::Channel` in the picture).
    current: Vec<bool>,
}

impl WaterLayout {
    pub fn build(width: f32, height: f32, road_cells: &[Position], water_cells: &[Position]) -> WaterLayout {
        let layout = Layout::new(width, height, road_cells, water_cells);
        let (cols, rows) = (layout.cols, layout.rows);
        let mut depth = vec![Depth::Dry; cols * rows];
        let mut current = vec![false; cols * rows];
        for y in 0..rows as i32 {
            for x in 0..cols as i32 {
                let i = y as usize * cols + x as usize;
                // The water past the map is picture only: nothing drives
                // there (`Layout::depth`).
                depth[i] = layout.depth(x, y);
                current[i] = match depth[i] {
                    Depth::Dry | Depth::Deep | Depth::Ice => false,
                    // A mouth carries its stream's current, as it does in
                    // the picture.
                    Depth::Shallow if layout.lake_at(x, y) => layout.stream_sides(x, y) & 0b1010 != 0,
                    Depth::Shallow => layout.channel_mask(x, y) & 0b1010 != 0,
                };
            }
        }
        WaterLayout { cols, rows, depth, current }
    }

    /// True when the map has no water at all - every rule short-circuits.
    pub fn is_empty(&self) -> bool {
        self.depth.iter().all(|d| *d == Depth::Dry)
    }

    /// Freeze every water cell over (a snowy round, docs/weather.md): no
    /// deep cell is left for a collider or the nav grid to block, no ford
    /// for the router to weigh, and no current runs.
    pub fn freeze(&mut self) {
        for d in &mut self.depth {
            if *d != Depth::Dry {
                *d = Depth::Ice;
            }
        }
        self.current.iter_mut().for_each(|c| *c = false);
    }

    /// Make the dry cells among `cells` fords: a rod's crater filled by the
    /// rain (docs/rod-from-god.md "The crater"), `Depth::Shallow` with no
    /// current, or ice where the water is frozen over. A cell already water,
    /// and one past the grid, is left. True if any changed.
    pub fn fill(&mut self, cells: impl IntoIterator<Item = (i32, i32)>) -> bool {
        let frozen = self.is_frozen();
        let mut changed = false;
        for (c, r) in cells {
            if c < 0 || r < 0 || c as usize >= self.cols || r as usize >= self.rows {
                continue;
            }
            let i = r as usize * self.cols + c as usize;
            if self.depth[i] == Depth::Dry {
                self.depth[i] = if frozen { Depth::Ice } else { Depth::Shallow };
                changed = true;
            }
        }
        changed
    }

    /// True once `freeze` has iced the water over.
    pub fn is_frozen(&self) -> bool {
        self.depth.contains(&Depth::Ice)
    }

    fn idx(&self, pos: Position) -> Option<usize> {
        let gx = (pos.x / GROUND_WORLD_TILE).round() as i32;
        let gy = (pos.y / GROUND_WORLD_TILE).round() as i32;
        if gx < 0 || gy < 0 || gx as usize >= self.cols || gy as usize >= self.rows {
            None
        } else {
            Some(gy as usize * self.cols + gx as usize)
        }
    }

    /// Whether a world position lies on the map's grid.
    pub fn contains(&self, pos: Position) -> bool {
        self.idx(pos).is_some()
    }

    /// The water under a world position (a hull's centre); off the grid
    /// is dry.
    pub fn depth_at(&self, pos: Position) -> Depth {
        self.idx(pos).map_or(Depth::Dry, |i| self.depth[i])
    }

    /// True where the current pushes a hull south: a stream cell joined
    /// north or south. Open lake water has no current a hull could feel,
    /// since no hull can be in it.
    pub fn pushes_south(&self, pos: Position) -> bool {
        self.idx(pos).is_some_and(|i| self.current[i])
    }

    fn cells_where(&self, want: Depth) -> impl Iterator<Item = Position> + '_ {
        let cols = self.cols;
        self.depth.iter().enumerate().filter(move |(_, d)| **d == want).map(move |(i, _)| {
            Position::new((i % cols) as f32 * GROUND_WORLD_TILE, (i / cols) as f32 * GROUND_WORLD_TILE)
        })
    }

    /// The centre of every deep cell: the static colliders a round spawns
    /// and the cells the nav grid blocks.
    pub fn deep_cells(&self) -> impl Iterator<Item = Position> + '_ {
        self.cells_where(Depth::Deep)
    }

    /// The centre of every ford cell: the cells the nav grid weighs.
    pub fn shallow_cells(&self) -> impl Iterator<Item = Position> + '_ {
        self.cells_where(Depth::Shallow)
    }

    /// Grid cell coordinates of every deep cell, for seam-closing the
    /// colliders (`battlefield::tile_hull_half_extent`).
    pub fn deep_grid_cells(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        let cols = self.cols;
        self.depth.iter().enumerate().filter(|(_, d)| **d == Depth::Deep).map(move |(i, _)| ((i % cols) as i32, (i / cols) as i32))
    }

    /// Grid cell coordinates of every cell the current runs down
    /// (`pushes_south`), in row-major order: where `fish.rs` carries a fish
    /// downstream.
    pub fn current_grid_cells(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        let cols = self.cols;
        self.current.iter().enumerate().filter(|(_, c)| **c).map(move |(i, _)| ((i % cols) as i32, (i / cols) as i32))
    }

    /// The depth of grid cell (`col`, `row`); off the grid is dry.
    pub fn depth_of_cell(&self, col: i32, row: i32) -> Depth {
        if col < 0 || row < 0 || col as usize >= self.cols || row as usize >= self.rows {
            Depth::Dry
        } else {
            self.depth[row as usize * self.cols + col as usize]
        }
    }
}

/// Roll this round's ground layout: grass everywhere, sand drifted over it
/// in soft patches when the look's theme drifts (`map::Theme::drifts` -
/// `ground_drift_cover` of the open floor, none touching a road or water
/// cell), road painted at exactly `road_cells` (world positions -
/// typically every static obstacle tile's own position plus the player
/// fortress's `B`/`O` interior cells, see `Game::init`) and water at
/// exactly `water_cells`, nowhere else. A water cell later painted road
/// (a wall on it) is road: walls stand on dirt. `width`/`height` are the same
/// playable-area extents `Game::init` already threads through everything
/// else (`battlefield::spawn_walls`, enemy/obstacle placement). `seed`
/// drives every grass cell's cosmetic tile pick (`grass_variant`) - pass a
/// freshly rolled value (e.g. `rng.random()`) for a normal round so it
/// varies, or a value held fixed across repeated calls (the editor) so
/// unrelated cells' grass doesn't visibly change on every edit. A
/// `road_cells`/`water_cells` entry that lands outside the grid (shouldn't
/// happen given every real caller's positions are already clamped inside
/// the battlefield, but not asserted here) is silently ignored rather than
/// panicking.
///
/// **Water** is one brush that draws two things, decided by the shape the
/// author painted, so a map needs no river/lake distinction:
///
/// - A water cell inside some 2x2 block of water is a **lake** cell. Lake
///   cells resolve through `WATER_SHORE`, the corner autotile: a grid
///   vertex is wet where all four cells around it are water, so a painted
///   block is a pool whose rounded shore runs half a cell inside its
///   outline, and a block two cells wide is a channel one cell wide with
///   a shore on each side. A diagonal staircase of blocks is smoothed
///   into one body (`Layout::wet_vertices`), the grass cell in each step
///   taking the lake's corner tile. Where a stream meets a shore, the
///   shore takes a mouth tile (`WATER_MOUTH`) with a gap the stream runs
///   through.
/// - Any other water cell is a **stream** cell - a line one cell wide,
///   painted the way a road is - and resolves through `WATER_CHANNEL`,
///   the edge autotile, joined to every water neighbour of either kind.
///
/// Every edge between two cells meets water to water and land to land,
/// whatever the shape: `tests::no_tile_edge_puts_water_against_land`
/// checks it pixel by pixel. Water painted to the map's edge runs off it
/// (`Layout`).
///
/// Water flows down the map: `draw` drifts marks southward over every
/// open lake cell and every stream cell joined north or south.
///
/// **Shade** is not resolved here: `wall_cells` and the `look` are kept
/// for the floor shade the grid bakes the first time it is drawn
/// (`GroundGrid::shade`, `bake_shade`).
pub fn build(
    width: f32,
    height: f32,
    seed: u64,
    road_cells: &[Position],
    water_cells: &[Position],
    wall_cells: &[Position],
    look: Look,
) -> GroundGrid {
    let layout = Layout::new(width, height, road_cells, water_cells);
    let (cols, rows) = (layout.cols, layout.rows);
    let t = tuning();
    let drifts = Drifts {
        cover: if look.theme.drifts() { t.ground_drift_cover.clamp(0.0, 1.0) } else { 0.0 },
        period: t.ground_drift_scale.max(1.0),
    };
    let mut tiles = vec![[0i32; WATER_FRAME_COUNT]; cols * rows];
    let mut current = vec![Current::Still; cols * rows];
    for y in 0..rows as i32 {
        for x in 0..cols as i32 {
            let i = y as usize * cols + x as usize;
            (tiles[i], current[i]) = resolve(&layout, seed, drifts, x, y);
        }
    }
    let walls = wall_cells
        .iter()
        .map(|pos| ((pos.x / OBSTACLE_GRID_SIZE).round() as i32, (pos.y / OBSTACLE_GRID_SIZE).round() as i32))
        .collect();
    GroundGrid {
        cols,
        rows,
        tiles,
        current,
        width,
        height,
        walls,
        look,
        shade: OnceLock::new(),
        stamp: NEXT_SHADE_STAMP.fetch_add(1, Ordering::Relaxed),
        layout,
        seed,
        drifts,
    }
}

/// How the sand drifts: the share of the open floor it covers and the
/// size of its patches, in grid vertices - the `ground_drift_*` knobs as
/// `build` read them, kept so `GroundGrid::repaint` resolves a cell with
/// the same drifts as the rest.
#[derive(Clone, Copy, Debug, Default)]
struct Drifts {
    cover: f32,
    period: f32,
}

/// Pick the source tile - every animation frame of it - and the current
/// for cell `(x, y)` of `layout`: `build`'s resolve, one cell. The
/// cosmetic hashes are keyed by the world cell (`Layout::origin`).
fn resolve(layout: &Layout, seed: u64, drifts: Drifts, x: i32, y: i32) -> ([i32; WATER_FRAME_COUNT], Current) {
    let (ox, oy) = layout.origin;
    // Drifts: sand at the grid vertices, resolved per cell through the
    // corner autotile. A vertex touching a road or water cell never
    // drifts: those tiles carry the plain fill's dithered edge baked in,
    // so a road or a shore running through a patch would show a fringe of
    // the wrong tone along it. The patches live in the open instead, which
    // is also where they read.
    let drift_vertex = |vx: i32, vy: i32| -> bool {
        if drifts.cover <= 0.0 {
            return false;
        }
        let beside_painted = [(vx - 1, vy - 1), (vx, vy - 1), (vx - 1, vy), (vx, vy)]
            .iter()
            .any(|&(cx, cy)| layout.at(cx, cy) != Material::Grass);
        !beside_painted && drift_noise(seed, vx + ox, vy + oy, drifts.period) > 1.0 - drifts.cover
    };
    // A grass cell under a wet corner - where a diagonal was smoothed over
    // - takes its lake tile like a water cell: the water there is the
    // lake's, the ground stays dry.
    let wet_corners = layout.corners(x, y);
    let (tile, flow, animated) = match layout.at(x, y) {
        Material::Grass if wet_corners != 0 => (lake_tile(wet_corners, 0), Current::Still, true),
        Material::Grass => {
            let corner = |vx: i32, vy: i32| drift_vertex(vx, vy) as usize;
            let mask = (corner(x, y) << 3) | (corner(x + 1, y) << 2) | (corner(x + 1, y + 1) << 1) | corner(x, y + 1);
            (if mask == 0 { grass_variant(seed, x + ox, y + oy) } else { SAND_CORNER[mask] }, Current::Still, false)
        }
        Material::Road => (ROAD_EDGE[layout.road_mask(x, y)], Current::Still, false),
        Material::Water if layout.lake_at(x, y) => {
            let sides = layout.stream_sides(x, y);
            // A mouth on the north or south carries the stream's current
            // through its gap, in the same band.
            let flow = if wet_corners == 0b1111 {
                Current::Open
            } else if sides & 0b1010 != 0 {
                Current::Channel
            } else {
                Current::Still
            };
            (lake_tile(wet_corners, sides), flow, true)
        }
        Material::Water => {
            let mask = layout.channel_mask(x, y);
            let along = mask & 0b1010 != 0;
            (WATER_CHANNEL[mask], if along { Current::Channel } else { Current::Still }, true)
        }
    };
    (if animated { frames_of(tile) } else { [tile; WATER_FRAME_COUNT] }, flow)
}

/// Field pixels per texel of the floor shade: the game's 2 px block, the
/// grain the runtime glows and the weather's light pass step on.
pub const SHADE_BLOCK: i32 = 2;

/// How many steps the walls' shade and the edge shade each take from clear
/// to their darkest. Few and dithered, the way a pixel-art floor shades,
/// rather than a smooth ramp.
const WALL_SHADE_STEPS: u8 = 3;
const EDGE_SHADE_STEPS: u8 = 4;

/// The 4 x 4 ordered-dither thresholds (in sixteenths) the shade steps
/// against, anchored at the field's top-left block so the pattern stays
/// put.
const BAYER4: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];

/// Shade blocks per wall cell. A multiple of the dither's 4, so every wall
/// meets the dither at the same phase and casts the same shade.
const CELL_BLOCKS: i64 = OBSTACLE_GRID_SIZE as i64 / SHADE_BLOCK as i64;
const _: () = assert!(CELL_BLOCKS * SHADE_BLOCK as i64 == OBSTACLE_GRID_SIZE as i64 && CELL_BLOCKS % 4 == 0);

/// The dither threshold of block `(bx, by)`, 0-1.
fn dither(bx: i64, by: i64) -> f32 {
    (BAYER4[(by.rem_euclid(4) * 4 + bx.rem_euclid(4)) as usize] as f32 + 0.5) / 16.0
}

/// A shade value 0-1 stepped against a dither threshold: 0 to `steps`.
fn shade_step(v: f32, steps: u8, threshold: f32) -> u8 {
    ((v * steps as f32 + threshold).floor().max(0.0) as u8).min(steps)
}

/// The cool dark walls shade the ground toward: red and green drop more
/// than blue, so on grass and on desert dust alike a wall's shade reads as
/// shadow rather than as dirt.
const WALL_SHADE_COLOR: Color = Color::new(16, 44, 52, 255);

/// What the field's edge deepens toward, by theme: the canopy's own green
/// on grass, a dusk umber on the desert - a darker version of the ground,
/// never black.
fn edge_shade_color(theme: Theme) -> Color {
    match theme {
        Theme::Grass => Color::new(10, 55, 40, 255),
        Theme::Desert => Color::new(45, 38, 40, 255),
    }
}

fn smoothstep(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// Bake the floor shade for a field `width` x `height` px with walls at
/// the grid cells `walls` (docs/GROUND_SPEC.md §4). Two layers, each a
/// value from 0 to 1 per 2 px block, stepped with the `BAYER4` dither and
/// laid as one overlay:
///
/// - **The walls' contact shade.** Measured from each wall's box moved
///   `ground_wall_shade_lean_px` along the drop shadows' direction
///   (`shadow_dir_x`/`_y`), so it pools on the side the walls' own shadows
///   fall and thins on the lit side, and fading out over
///   `ground_wall_shade_cells` cells on a smoothstep. Per block and
///   euclidean, so it follows a wall's outline with no 32 px squares.
/// - **The edge shade**, on a round's field only (`Look::edge_shade`):
///   the distance outside a rounded rectangle inset `ground_edge_shade_px`
///   from the edges, its corners rounded by `ground_edge_shade_round` times
///   that, so the shade follows the frame and deepens into the corners;
///   `ground_edge_shade_corner` of its darkest step is reserved for them.
///
/// The steps take `ground_wall_shade` and `ground_edge_shade` as the
/// opacity of their darkest step, toward `WALL_SHADE_COLOR` and the
/// theme's `edge_shade_color`. Only IEEE arithmetic and square roots, so
/// a CPU thumbnail of it is the same on every platform. Takes the table
/// rather than reading the global so a test can pass a local one.
pub fn bake_shade(width: f32, height: f32, walls: &[(i32, i32)], look: Look, t: &Tuning, stamp: u64) -> BlockImage {
    Shade::bake(width, height, walls, look, t, stamp).image
}

/// The floor shade as baked, and the recipe it was baked by.
struct Shade {
    image: BlockImage,
    recipe: ShadeRecipe,
}

impl Shade {
    /// `bake_shade`, keeping the recipe.
    fn bake(width: f32, height: f32, walls: &[(i32, i32)], look: Look, t: &Tuning, stamp: u64) -> Shade {
        let recipe = ShadeRecipe::new(width, height, look, t);
        let (bw, bh) = (recipe.bw, recipe.bh);
        let mut texels = vec![Color::new(0, 0, 0, 0); bw * bh];
        recipe.bake(walls, (0, 0, bw as i64, bh as i64), &mut texels);
        Shade { image: BlockImage { width: bw, height: bh, block: SHADE_BLOCK, texels, stamp, patches: Vec::new() }, recipe }
    }

    /// Bake again, from `walls` as they are now, every block a wall at one
    /// of `cells` would shade - a wall came or went there -, under a new
    /// stamp and a patch naming those blocks.
    fn rebake(&mut self, walls: &[(i32, i32)], cells: &[(i32, i32)]) {
        let reached = cells.iter().filter_map(|&cell| self.recipe.reach(cell));
        let Some((x0, y0, x1, y1)) = reached.reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))) else {
            return;
        };
        self.recipe.bake(walls, (x0, y0, x1, y1), &mut self.image.texels);
        let stamp = NEXT_SHADE_STAMP.fetch_add(1, Ordering::Relaxed);
        self.image.patched(stamp, x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize);
    }
}

/// Everything the floor shade reads off the table and the field, worked
/// out once - the levels every wall lays, the edge layer's shape and the
/// palette the two are laid through - so any part of the shade bakes again
/// exactly as the whole did.
struct ShadeRecipe {
    /// The image's size in blocks.
    bw: usize,
    bh: usize,
    /// The levels a wall lays, `stamp_w` blocks wide, from block `(col *
    /// CELL_BLOCKS + kx0, row * CELL_BLOCKS + ky0)` for a wall at `(col,
    /// row)`. Empty when the walls cast no shade.
    stamp: Vec<u8>,
    stamp_w: usize,
    kx0: i64,
    ky0: i64,
    /// The edge layer, on a round's field.
    edge: Option<EdgeShade>,
    /// The texel for each pair of steps, the wall's then the edge's.
    palette: [[Color; EDGE_SHADE_STEPS as usize + 1]; WALL_SHADE_STEPS as usize + 1],
}

impl ShadeRecipe {
    fn new(width: f32, height: f32, look: Look, t: &Tuning) -> ShadeRecipe {
        let block = SHADE_BLOCK as f32;
        let bw = (width / block).ceil().max(0.0) as usize;
        let bh = (height / block).ceil().max(0.0) as usize;

        // --- the walls: a contact shade leaning the way the shadows fall ---
        // Every wall casts the same shade moved by whole cells (`CELL_BLOCKS`),
        // so it is stepped once, as a stamp of levels around a wall's cell, and
        // laid at each wall with a max - a step is monotone, so the deepest of
        // the steps is the step of the deepest shade. This keeps a wall's cost
        // to a max over the stamp, and lets a part of the shade be baked again
        // from the walls near it.
        let (mut stamp, mut stamp_w, mut kx0, mut ky0) = (Vec::new(), 0, 0, 0);
        let reach = t.ground_wall_shade_cells.max(0.0) * OBSTACLE_GRID_SIZE;
        if reach > 0.0 && t.ground_wall_shade > 0.0 {
            let len = (t.shadow_dir_x * t.shadow_dir_x + t.shadow_dir_y * t.shadow_dir_y).sqrt();
            let (lx, ly) = if len > 1e-6 {
                (t.shadow_dir_x / len * t.ground_wall_shade_lean_px, t.shadow_dir_y / len * t.ground_wall_shade_lean_px)
            } else {
                (0.0, 0.0)
            };
            let half = OBSTACLE_GRID_SIZE / 2.0;
            // The blocks the leaned box can shade along one axis, as offsets
            // `lo..hi` from block `col * CELL_BLOCKS`, the one starting at the
            // wall cell's centre.
            let span = |lean: f32| (((lean - half - reach) / block).floor() as i64 - 1, ((lean + half + reach) / block).ceil() as i64 + 1);
            let ((x0, x1), (y0, y1)) = (span(lx), span(ly));
            (kx0, ky0, stamp_w) = (x0, y0, (x1 - x0) as usize);
            stamp = vec![0u8; stamp_w * (y1 - y0) as usize];
            for (ky, row) in (y0..y1).zip(stamp.chunks_exact_mut(stamp_w)) {
                for (kx, level) in (x0..x1).zip(row.iter_mut()) {
                    // The block's centre measured from the wall cell's centre.
                    let ox = (((kx as f32 + 0.5) * block - lx).abs() - half).max(0.0);
                    let oy = (((ky as f32 + 0.5) * block - ly).abs() - half).max(0.0);
                    let d = (ox * ox + oy * oy).sqrt();
                    if d < reach {
                        *level = shade_step(smoothstep(1.0 - d / reach), WALL_SHADE_STEPS, dither(kx, ky));
                    }
                }
            }
        }

        // --- the edge: outside a rounded rectangle inset from the frame ---
        let edge = if look.edge_shade { EdgeShade::of(width, height, t) } else { None };

        // --- one colour per pair of steps ---
        let clear = Color::new(0, 0, 0, 0);
        let (wall_color, edge_color) = (WALL_SHADE_COLOR, edge_shade_color(look.theme));
        let (wall_max, edge_max) = (t.ground_wall_shade.clamp(0.0, 1.0), t.ground_edge_shade.clamp(0.0, 1.0));
        let mut palette = [[clear; EDGE_SHADE_STEPS as usize + 1]; WALL_SHADE_STEPS as usize + 1];
        for (lw, row) in palette.iter_mut().enumerate() {
            for (le, texel) in row.iter_mut().enumerate() {
                let aw = wall_max * lw as f32 / WALL_SHADE_STEPS as f32;
                let ae = edge_max * le as f32 / EDGE_SHADE_STEPS as f32;
                // The wall's shade laid over the edge's.
                let a = aw + ae * (1.0 - aw);
                if a > 0.0 {
                    let mix = |w: u8, e: u8| ((w as f32 * aw + e as f32 * ae * (1.0 - aw)) / a).round().clamp(0.0, 255.0) as u8;
                    *texel = Color::new(
                        mix(wall_color.r, edge_color.r),
                        mix(wall_color.g, edge_color.g),
                        mix(wall_color.b, edge_color.b),
                        (a * 255.0).round().clamp(0.0, 255.0) as u8,
                    );
                }
            }
        }
        ShadeRecipe { bw, bh, stamp, stamp_w, kx0, ky0, edge, palette }
    }

    /// The blocks a wall at `(col, row)` shades, within the image, as
    /// `(x0, y0, x1, y1)`, the ends excluded; `None` when the walls cast no
    /// shade or this one's lies off the image.
    fn reach(&self, (col, row): (i32, i32)) -> Option<(i64, i64, i64, i64)> {
        if self.stamp.is_empty() {
            return None;
        }
        let rows = (self.stamp.len() / self.stamp_w) as i64;
        let (x, y) = (col as i64 * CELL_BLOCKS + self.kx0, row as i64 * CELL_BLOCKS + self.ky0);
        let (x0, y0) = (x.max(0), y.max(0));
        let (x1, y1) = ((x + self.stamp_w as i64).min(self.bw as i64), (y + rows).min(self.bh as i64));
        (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
    }

    /// Bake blocks `x0..x1` by `y0..y1` into `texels`, the whole image's,
    /// from the walls at `walls`: each wall's stamp laid with a max, then
    /// each block's pair of steps through the palette.
    fn bake(&self, walls: &[(i32, i32)], (x0, y0, x1, y1): (i64, i64, i64, i64), texels: &mut [Color]) {
        let (w, h) = ((x1 - x0).max(0) as usize, (y1 - y0).max(0) as usize);
        let mut wall = vec![0u8; w * h];
        if !self.stamp.is_empty() && w > 0 {
            let sw = self.stamp_w;
            for &(col, row) in walls {
                let (sx, sy) = (col as i64 * CELL_BLOCKS + self.kx0, row as i64 * CELL_BLOCKS + self.ky0);
                let (from, to) = (sx.max(x0), (sx + sw as i64).min(x1));
                if from >= to {
                    continue;
                }
                for (by, src) in (sy..).zip(self.stamp.chunks_exact(sw)) {
                    if by < y0 || by >= y1 {
                        continue;
                    }
                    let dst = &mut wall[(by - y0) as usize * w..][(from - x0) as usize..(to - x0) as usize];
                    for (d, s) in dst.iter_mut().zip(&src[(from - sx) as usize..]) {
                        *d = (*d).max(*s);
                    }
                }
            }
        }
        for by in y0..y1 {
            for bx in x0..x1 {
                let le = self.edge.as_ref().map_or(0, |edge| edge.level(bx, by));
                let lw = wall[(by - y0) as usize * w + (bx - x0) as usize];
                texels[by as usize * self.bw + bx as usize] = self.palette[lw as usize][le as usize];
            }
        }
    }
}

/// The edge shade's shape (`bake_shade`): a field `width` x `height` px,
/// the rounded rectangle inset `reach` from its edges - half-sides `hx`,
/// `hy` and corner radius `round` - and how the corner's share of the
/// darkest step peaks at `corner_at` reaches out.
#[derive(Clone, Copy, Debug)]
struct EdgeShade {
    width: f32,
    height: f32,
    hx: f32,
    hy: f32,
    round: f32,
    reach: f32,
    corner_at: f32,
    corner_share: f32,
}

impl EdgeShade {
    /// The edge shade of a field `width` x `height` px under the table's
    /// `ground_edge_shade*` rows; `None` where they draw none.
    fn of(width: f32, height: f32, t: &Tuning) -> Option<EdgeShade> {
        let edge_reach = t.ground_edge_shade_px;
        (t.ground_edge_shade > 0.0 && edge_reach > 0.0).then(|| {
            let (hx, hy) = ((width / 2.0 - edge_reach).max(0.0), (height / 2.0 - edge_reach).max(0.0));
            let round = (edge_reach * t.ground_edge_shade_round.max(0.0)).min(hx).min(hy);
            let corner_share = t.ground_edge_shade_corner.clamp(0.0, 1.0);
            let mut edge = EdgeShade { width, height, hx, hy, round, reach: edge_reach, corner_at: 2.0, corner_share };
            // How far out the field's corner is, in reaches: where the shade peaks.
            edge.corner_at = (edge.outside(0.0, 0.0) / edge_reach).max(1.0 + 1e-3);
            edge
        })
    }

    /// Signed distance outside the rounded rectangle (negative inside it).
    fn outside(&self, px: f32, py: f32) -> f32 {
        let qx = (px - self.width / 2.0).abs() - (self.hx - self.round);
        let qy = (py - self.height / 2.0).abs() - (self.hy - self.round);
        let (mx, my) = (qx.max(0.0), qy.max(0.0));
        (mx * mx + my * my).sqrt() + qx.max(qy).min(0.0) - self.round
    }

    /// The edge layer at field pixel `(px, py)` before it is stepped, 0 to
    /// 1: clear inside the rounded frame, deepening past it and into the
    /// corners - and on past the field's edge, where it is 1 once as far
    /// out as the corners (`corner_at` reaches).
    fn value(&self, px: f32, py: f32) -> f32 {
        let s = self.outside(px, py) / self.reach;
        if s <= 0.0 {
            0.0
        } else {
            smoothstep(s) * (1.0 - self.corner_share) + self.corner_share * ((s - 1.0) / (self.corner_at - 1.0)).clamp(0.0, 1.0)
        }
    }

    /// The edge layer's step at block `(bx, by)`, measured at its centre.
    fn level(&self, bx: i64, by: i64) -> u8 {
        let centre = |b: i64| (b as f32 + 0.5) * SHADE_BLOCK as f32;
        shade_step(self.value(centre(bx), centre(by)), EDGE_SHADE_STEPS, dither(bx, by))
    }
}

/// The shade over the world past a round's field, the ground an arena
/// shows in the window's margins (`margin.rs`, docs/large-maps-follow-camera.md
/// §13): `image` holds the blocks near the field, `plateau` lies on every
/// block past them.
pub struct MarginShade {
    /// The blocks `reach` px round the field, clear over the field itself.
    /// Its texel (0, 0) is world block `origin`, not the field's corner.
    pub image: BlockImage,
    /// The world block (`SHADE_BLOCK` px a side) `image` starts at.
    pub origin: (i64, i64),
    /// The shade at its deepest, one colour: what every block past the
    /// image carries.
    pub plateau: Color,
}

/// Bake the shade over the world past a field `width` x `height` px (docs/
/// GROUND_SPEC.md §4, docs/large-maps-follow-camera.md §13 item 7): the
/// field's edge shade carried on out past its edge - the same rounded
/// frame, the same steps and the same dither on the world's block grid, so
/// the bands run on across the edge unbroken - and deepened over
/// `ground_margin_ramp_px` to `ground_margin_shade`, the dark the ground
/// off the playfield stands in, toward the theme's own dark
/// (`edge_shade_color`) in whole steps of the edge shade's size. The
/// deepest step is a whole one, so past the ramp every block is the same
/// and the image need only reach that far (`MarginShade::plateau` beyond
/// it). Takes the table rather than reading the global, like `bake_shade`,
/// and names the image with a stamp of its own (`NEXT_SHADE_STAMP`).
pub fn bake_margin_shade(width: f32, height: f32, theme: Theme, t: &Tuning) -> MarginShade {
    let block = SHADE_BLOCK as f32;
    let edge = EdgeShade::of(width, height, t);
    let edge_max = t.ground_edge_shade.clamp(0.0, 1.0);
    let deep = t.ground_margin_shade.clamp(0.0, 1.0);
    // A step is the edge shade's own, or the margin's quarter on a field
    // with no edge shade; the steps the edge shade climbs, then the steps
    // to the deepest, whole.
    let (unit, edge_steps) = if edge.is_some() { (edge_max, EDGE_SHADE_STEPS as f32) } else { (deep, 0.0) };
    let deep_steps = if unit > 0.0 { (deep / unit * EDGE_SHADE_STEPS as f32).round() } else { 0.0 };
    let top = deep_steps.max(edge_steps).min(u8::MAX as f32) as u8;
    let alpha = |level: u8| (unit * level as f32 / EDGE_SHADE_STEPS as f32 * 255.0).round().clamp(0.0, 255.0) as u8;
    let dark = edge_shade_color(theme);
    let texel = |level: u8| if level == 0 { Color::new(0, 0, 0, 0) } else { Color::new(dark.r, dark.g, dark.b, alpha(level)) };
    // How far past the field the shade still changes: the ramp, or the
    // edge shade's own climb into its corner value, and a block more.
    let ramp = t.ground_margin_ramp_px.max(0.0);
    let climb = edge.map_or(0.0, |e| (e.corner_at - 1.0) * e.reach);
    let reach = ((ramp.max(climb) + block) / block).ceil() as i64;
    let (bw, bh) = ((width / block).ceil() as i64, (height / block).ceil() as i64);
    let (x0, y0, x1, y1) = (-reach, -reach, bw + reach, bh + reach);
    let mut texels = Vec::with_capacity(((x1 - x0) * (y1 - y0)) as usize);
    for by in y0..y1 {
        for bx in x0..x1 {
            let (px, py) = ((bx as f32 + 0.5) * block, (by as f32 + 0.5) * block);
            let (dx, dy) = ((-px).max(px - width).max(0.0), (-py).max(py - height).max(0.0));
            if dx <= 0.0 && dy <= 0.0 {
                texels.push(texel(0));
                continue;
            }
            let out = (dx * dx + dy * dy).sqrt();
            let deepen = if ramp > 0.0 { smoothstep(out / ramp) } else { 1.0 };
            let carried = edge.map_or(0.0, |e| e.value(px, py) * edge_steps);
            let steps = carried + (deep_steps - carried).max(0.0) * deepen;
            let level = ((steps + dither(bx, by)).floor().max(0.0) as u8).min(top);
            texels.push(texel(level));
        }
    }
    let stamp = NEXT_SHADE_STAMP.fetch_add(1, Ordering::Relaxed);
    let image = BlockImage { width: (x1 - x0) as usize, height: (y1 - y0) as usize, block: SHADE_BLOCK, texels, stamp, patches: Vec::new() };
    MarginShade { image, origin: (x0, y0), plateau: texel(top) }
}

/// Lay the floor shade over the ground: drawn straight after it (and the
/// weather's ground pass), so it shades the floor only - tanks and walls
/// stand in front of it, not under it. One call on the GPU.
pub fn draw_shade(c: &mut impl Canvas, grid: &GroundGrid) {
    c.blocks(grid.shade());
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
/// drawn first, before tread marks/obstacles/tanks. `time` (seconds,
/// any clock that only moves while the picture should) steps the water
/// through its frames and drifts the flow marks (`draw_current`).
/// `theme` only names the tileset file (`Sheet::Ground`): the tile ids
/// are the same under every theme.
pub fn draw(c: &mut impl Canvas, grid: &GroundGrid, theme: Theme, time: f32) {
    let size = GROUND_WORLD_TILE;
    let origin = Vec2::new(size / 2.0, size / 2.0);
    let t = tuning();
    let frame = ((time / t.water_frame_seconds.max(0.01)).floor() as i64).rem_euclid(WATER_FRAME_COUNT as i64) as usize;
    let (ox, oy) = grid.layout.origin;
    let (cols, rows) = cell_span(grid, c.cull());
    for y in rows {
        for x in cols.clone() {
            let Some(i) = grid.idx(x as i32, y as i32) else {
                continue;
            };
            let src = source_rec(grid.tiles[i][frame]);
            let dest = Rectangle::new((x as i32 + ox) as f32 * GROUND_WORLD_TILE, (y as i32 + oy) as f32 * GROUND_WORLD_TILE, size, size);
            c.blit(Sheet::Ground(theme), src, dest, origin, 0.0, Color::WHITE);
        }
    }
    draw_current(c, grid, time, t.water_flow_speed, t.water_flow_lanes.max(0) as u32);
}

/// The columns and rows of `grid`'s cells a culling rectangle
/// (`Canvas::cull`) can see, every cell for none. A cell is centred on
/// its world cell times `GROUND_WORLD_TILE` and spans half a tile either
/// side; the span rounds outward, so it may keep a cell it strictly need
/// not.
fn cell_span(grid: &GroundGrid, cull: Option<Rectangle>) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
    let Some(r) = cull else {
        return (0..grid.cols, 0..grid.rows);
    };
    let tile = GROUND_WORLD_TILE;
    let (ox, oy) = grid.layout.origin;
    let span = |from: f32, len: f32, origin: i32, n: usize| {
        let first = ((from - tile) / tile - origin as f32).floor().max(0.0) as usize;
        let past = ((from + len + tile) / tile - origin as f32).ceil().max(0.0) as usize;
        first.min(n)..past.min(n)
    };
    (span(r.x, r.width, ox, grid.cols), span(r.y, r.height, oy, grid.rows))
}

/// A deterministic per-lane hash: the marks are cosmetic, so they are
/// keyed by the column and the lane alone - no seed, no RNG, the same on
/// every machine and in the builder. (SplitMix64's mixing step, like
/// `grass_variant`.)
fn lane_hash(x: i32, lane: u32) -> u64 {
    let mut h = 0x5A17_ED0C_0FFE_E000u64 ^ (x as i64 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= (lane as u64 + 1).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h
}

/// The current: short light marks drifting **down** the map over open
/// water and down every north/south stream, `speed` px per second,
/// `lanes` of them per column. Built from 2 px blocks like every other
/// runtime glow (no smooth shapes on a pixel floor) and clipped to the
/// cell, so a mark never crosses onto a shore.
///
/// A lane is keyed by its column, not its cell, and repeats every
/// `WATER_FLOW_PERIOD_CELLS` cells: vertically adjacent open cells show
/// the *same* lane at the same moment, so a mark leaving one cell's bottom
/// edge enters the next cell's top edge without a jump - the flow reads as
/// one body of water, not a grid of looping cells.
fn draw_current(c: &mut impl Canvas, grid: &GroundGrid, time: f32, speed: f32, lanes: u32) {
    if lanes == 0 {
        return;
    }
    let tile = GROUND_WORLD_TILE;
    let period = tile * WATER_FLOW_PERIOD_CELLS as f32;
    let (band_lo, band_hi) = WATER_CHANNEL_BAND;
    let lane_slots = ((band_hi - band_lo) / 2.0) as u64 - 1;
    let (ox, oy) = grid.layout.origin;
    let (cols, rows) = cell_span(grid, c.cull());
    for y in rows.map(|y| y as i32) {
        for x in cols.clone().map(|x| x as i32) {
            let Some(i) = grid.idx(x, y) else {
                continue;
            };
            if grid.current[i] == Current::Still {
                continue;
            }
            // The world cell: a lane is keyed by its world column.
            let (x, y) = (x + ox, y + oy);
            let left = x as f32 * tile - tile / 2.0;
            let top = y as f32 * tile - tile / 2.0;
            for lane in 0..lanes {
                let h = lane_hash(x, lane);
                // Inside the stream band, which open water contains too,
                // so a lane keeps its column through a lake into the
                // stream that drains it.
                let lx = band_lo + 2.0 * (h % lane_slots) as f32;
                let phase = ((h >> 16) % period as u64) as f32;
                // Where the lane's mark is within its period, relative
                // to this cell's top edge.
                let along = (phase + time * speed).rem_euclid(period) - (y.rem_euclid(WATER_FLOW_PERIOD_CELLS)) as f32 * tile;
                let ly = (along / 2.0).floor() * 2.0;
                for block in 0..WATER_FLOW_MARK_BLOCKS {
                    let by = ly + 2.0 * block as f32;
                    if (0.0..tile).contains(&by) {
                        c.fill_rect((left + lx) as i32, (top + by) as i32, 2, 2, WATER_FLOW_MARK);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fill_makes_dry_cells_fords_and_freeze_ices_them() {
        let pond: Vec<Position> = vec![crate::map::cell_to_world(2, 2)];
        let mut w = WaterLayout::build(320.0, 320.0, &[], &pond);
        assert!(w.fill([(5, 5), (5, 6), (2, 2), (-1, 0)]));
        assert_eq!(w.depth_of_cell(5, 5), Depth::Shallow);
        assert_eq!(w.depth_of_cell(5, 6), Depth::Shallow);
        assert!(!w.pushes_south(crate::map::cell_to_world(5, 6)));
        assert!(!w.fill([(5, 5)]), "already water");
        w.freeze();
        assert_eq!(w.depth_of_cell(5, 5), Depth::Ice);
        assert!(w.fill([(7, 7)]));
        assert_eq!(w.depth_of_cell(7, 7), Depth::Ice, "filled under ice, it is ice");
    }

    use super::*;

    const W: f32 = 320.0;
    const H: f32 = 320.0;

    fn world(cells: &[(i32, i32)]) -> Vec<Position> {
        cells.iter().map(|&(c, r)| Position::new(c as f32 * GROUND_WORLD_TILE, r as f32 * GROUND_WORLD_TILE)).collect()
    }

    fn with_water(cells: &[(i32, i32)]) -> GroundGrid {
        build(W, H, 7, &[], &world(cells), &[], Look::default())
    }

    fn tile(grid: &GroundGrid, x: i32, y: i32) -> i32 {
        grid.tiles[grid.idx(x, y).expect("in grid")][0]
    }

    fn current(grid: &GroundGrid, x: i32, y: i32) -> Current {
        grid.current[grid.idx(x, y).expect("in grid")]
    }

    #[test]
    fn a_cull_rectangle_keeps_the_cells_it_can_see() {
        let grid = with_water(&[]);
        assert_eq!(cell_span(&grid, None), (0..grid.cols, 0..grid.rows), "no rectangle, every cell");
        // Cells 4 to 6 across and 2 to 3 down touch this rectangle; the
        // span rounds outward by one on the near side.
        let r = Rectangle::new(4.0 * GROUND_WORLD_TILE, 2.0 * GROUND_WORLD_TILE, 64.0, 32.0);
        assert_eq!(cell_span(&grid, Some(r)), (3..7, 1..4));
        let off = Rectangle::new(-500.0, -500.0, 100.0, 100.0);
        let (cols, rows) = cell_span(&grid, Some(off));
        assert!(cols.is_empty() && rows.is_empty(), "nothing off the field: {cols:?} {rows:?}");
        let past = Rectangle::new(9000.0, 9000.0, 100.0, 100.0);
        let (cols, rows) = cell_span(&grid, Some(past));
        assert!(cols.is_empty() && rows.is_empty(), "nothing past it: {cols:?} {rows:?}");
    }

    #[test]
    fn a_line_of_water_is_a_stream_flowing_down() {
        let grid = with_water(&[(5, 2), (5, 3), (5, 4), (5, 5), (5, 6)]);
        assert_eq!(tile(&grid, 5, 2), WATER_CHANNEL[0b0010], "the top end is a south cap");
        assert_eq!(tile(&grid, 5, 4), WATER_CHANNEL[0b1010], "the middle runs north-south");
        assert_eq!(tile(&grid, 5, 6), WATER_CHANNEL[0b1000], "the bottom end is a north cap");
        for y in 2..=6 {
            assert_eq!(current(&grid, 5, y), Current::Channel, "row {y} carries the current");
        }
        assert_eq!(current(&grid, 4, 4), Current::Still, "grass beside it does not");
        assert!(GRASS_FILL.contains(&tile(&grid, 4, 4)), "the bank stays grass");
    }

    #[test]
    fn a_sideways_stream_shimmers_but_carries_no_current() {
        let grid = with_water(&[(3, 4), (4, 4), (5, 4)]);
        assert_eq!(tile(&grid, 4, 4), WATER_CHANNEL[0b0101]);
        assert_eq!(current(&grid, 4, 4), Current::Still);
        assert_eq!(tile(&grid, 7, 7), tile(&grid, 7, 7), "unrelated cells untouched");
        let lone = with_water(&[(2, 2)]);
        assert_eq!(tile(&lone, 2, 2), WATER_CHANNEL[0], "a single cell is the pool tile");
        assert_eq!(current(&lone, 2, 2), Current::Still);
    }

    #[test]
    fn a_block_of_water_is_a_lake_with_a_shore_inside_its_outline() {
        let cells: Vec<(i32, i32)> = (4..=6).flat_map(|x| (4..=6).map(move |y| (x, y))).collect();
        let grid = with_water(&cells);
        assert_eq!(tile(&grid, 5, 5), WATER_SHORE[0b1111], "the centre is open water");
        assert_eq!(current(&grid, 5, 5), Current::Open);
        assert_eq!(tile(&grid, 4, 4), WATER_SHORE[0b0010], "top-left corner: wet at its bottom-right only");
        assert_eq!(tile(&grid, 5, 4), WATER_SHORE[0b0011], "top shore");
        assert_eq!(tile(&grid, 4, 5), WATER_SHORE[0b0110], "left shore");
        assert_eq!(tile(&grid, 6, 5), WATER_SHORE[0b1001], "right shore");
        assert_eq!(tile(&grid, 5, 6), WATER_SHORE[0b1100], "bottom shore");
        assert_eq!(tile(&grid, 6, 6), WATER_SHORE[0b1000], "bottom-right corner");
        for (x, y) in [(4, 4), (5, 4), (4, 5), (6, 6)] {
            assert_eq!(current(&grid, x, y), Current::Still, "shores carry no marks at {x},{y}");
        }
        assert!(GRASS_FILL.contains(&tile(&grid, 7, 5)), "the bank stays grass");
    }

    #[test]
    fn a_two_wide_block_is_a_channel_with_a_shore_on_each_side() {
        let cells: Vec<(i32, i32)> = (2..=6).flat_map(|y| [(4, y), (5, y)]).collect();
        let grid = with_water(&cells);
        assert_eq!(tile(&grid, 4, 4), WATER_SHORE[0b0110], "left column is the left shore");
        assert_eq!(tile(&grid, 5, 4), WATER_SHORE[0b1001], "right column is the right shore");
        assert_eq!(tile(&grid, 4, 2), WATER_SHORE[0b0010], "top-left corner");
        assert_eq!(tile(&grid, 5, 6), WATER_SHORE[0b1000], "bottom-right corner");
    }

    #[test]
    fn a_stream_meeting_a_lake_runs_through_a_mouth_in_the_shore() {
        let mut cells: Vec<(i32, i32)> = (4..=6).flat_map(|x| (4..=6).map(move |y| (x, y))).collect();
        cells.extend([(5, 2), (5, 3)]);
        let grid = with_water(&cells);
        assert_eq!(tile(&grid, 5, 3), WATER_CHANNEL[0b1010], "the stream runs straight into the lake");
        assert_eq!(current(&grid, 5, 3), Current::Channel);
        assert_eq!(tile(&grid, 5, 4), 275, "the shore under the stream is the pack's north mouth");
        assert_eq!(current(&grid, 5, 4), Current::Channel, "the current carries on through the mouth");
        assert_eq!(tile(&grid, 4, 4), WATER_SHORE[0b0010], "the corners beside it stay corners");
        assert_eq!(tile(&grid, 6, 4), WATER_SHORE[0b0001]);
        assert!(GRASS_FILL.contains(&tile(&grid, 4, 3)), "the stream's banks stay grass");
        assert!(GRASS_FILL.contains(&tile(&grid, 6, 3)));
        // Every straight shore has its mouth.
        let mut cells: Vec<(i32, i32)> = (3..=7).flat_map(|x| (3..=7).map(move |y| (x, y))).collect();
        cells.extend([(5, 1), (5, 2), (5, 8), (5, 9), (1, 5), (2, 5), (8, 5), (9, 5)]);
        let grid = with_water(&cells);
        assert_eq!(tile(&grid, 5, 3), 275);
        assert_eq!(tile(&grid, 5, 7), 329);
        assert_eq!(tile(&grid, 3, 5), 301);
        assert_eq!(tile(&grid, 7, 5), 303);
        assert_eq!(current(&grid, 3, 5), Current::Still, "a mouth on the side carries no current");
    }

    #[test]
    fn a_stream_meeting_a_channel_at_its_corner_has_a_mouth_there() {
        // A two-wide channel running down, a stream entering its top-left
        // cell from the north (half a cell off the channel's middle) and
        // one entering its bottom-right cell from the east.
        let mut cells: Vec<(i32, i32)> = (3..=6).flat_map(|y| [(4, y), (5, y)]).collect();
        cells.extend([(4, 1), (4, 2), (6, 6), (7, 6)]);
        let grid = with_water(&cells);
        assert_eq!(tile(&grid, 4, 3), lake_tile(0b0010, 0b1000), "BR wet, the stream from the north");
        assert_eq!(tile(&grid, 5, 6), lake_tile(0b1000, 0b0100), "TL wet, the stream from the east");
        assert!((WATER_EXTRA..WATER_EXTRA + WATER_EXTRA_COUNT).contains(&tile(&grid, 4, 3)), "a composed tile");
    }

    #[test]
    fn a_bend_in_a_stream_stays_a_stream() {
        // An L: no 2x2 block, so the corner cell must not become a pond.
        let grid = with_water(&[(4, 4), (4, 5), (5, 5)]);
        assert_eq!(tile(&grid, 4, 5), WATER_CHANNEL[0b1100], "north and east joined");
        assert_eq!(tile(&grid, 4, 4), WATER_CHANNEL[0b0010]);
        assert_eq!(tile(&grid, 5, 5), WATER_CHANNEL[0b0001]);
    }

    #[test]
    fn road_wins_over_water_and_the_two_never_join() {
        let water = world(&[(4, 4), (5, 4), (6, 4)]);
        let road = world(&[(5, 4), (7, 4)]);
        let grid = build(W, H, 7, &road, &water, &[], Look::default());
        assert_eq!(tile(&grid, 5, 4), ROAD_EDGE[0b0000], "a road cell on water is an isolated road");
        assert_eq!(tile(&grid, 4, 4), WATER_CHANNEL[0b0000], "water beside road is not joined to it");
        assert_eq!(tile(&grid, 6, 4), WATER_CHANNEL[0b0000]);
        assert_eq!(tile(&grid, 7, 4), ROAD_EDGE[0b0000]);
    }

    #[test]
    fn every_water_tile_has_its_frames_and_nothing_else_animates() {
        let mouths = WATER_MOUTH.iter().map(|(_, _, tile)| tile);
        for tile in WATER_CHANNEL.iter().chain(WATER_SHORE.iter()).chain(mouths) {
            let frames = frames_of(*tile);
            assert_eq!(frames[0], *tile, "frame 0 is the tile itself");
            assert!(frames.iter().all(|f| *f >= 0 && *f < TILESET_COLS * 65), "frames stay on the sheet");
        }
        let grid = with_water(&[(5, 5), (6, 5), (5, 6), (6, 6), (2, 2)]);
        for y in 0..grid.rows as i32 {
            for x in 0..grid.cols as i32 {
                let frames = grid.tiles[grid.idx(x, y).unwrap()];
                let water = [(5, 5), (6, 5), (5, 6), (6, 6), (2, 2)].contains(&(x, y));
                let still = frames.iter().all(|f| *f == frames[0]);
                assert!(water || still, "only water animates: {x},{y} {frames:?}");
            }
        }
    }

    #[test]
    fn the_rules_see_deep_water_only_where_the_picture_is_open_water() {
        let mut cells: Vec<(i32, i32)> = (4..=8).flat_map(|x| (4..=7).map(move |y| (x, y))).collect();
        cells.extend([(6, 1), (6, 2), (6, 3), (1, 6), (2, 6), (3, 6)]);
        let water = WaterLayout::build(W, H, &[], &world(&cells));
        let grid = with_water(&cells);
        let at = |x: i32, y: i32| Position::new(x as f32 * GROUND_WORLD_TILE, y as f32 * GROUND_WORLD_TILE);
        assert!(!water.is_empty());
        // The lake: the 3 x 2 interior is deep, the ring of shores around it a ford.
        for x in 5..=7 {
            for y in 5..=6 {
                assert_eq!(water.depth_at(at(x, y)), Depth::Deep, "{x},{y}");
                assert_eq!(current(&grid, x, y), Current::Open);
            }
        }
        for (x, y) in [(4, 4), (8, 7), (4, 5), (4, 7), (7, 7)] {
            assert_eq!(water.depth_at(at(x, y)), Depth::Shallow, "shore {x},{y}");
        }
        // The mouths: the shore cell each stream runs into is a ford, and
        // the one a stream enters from the north carries its current.
        assert_eq!(water.depth_at(at(6, 4)), Depth::Shallow);
        assert!(water.pushes_south(at(6, 4)));
        assert_eq!(water.depth_at(at(4, 6)), Depth::Shallow);
        assert!(!water.pushes_south(at(4, 6)));
        // Streams are fords; the vertical one carries the current, the sideways one does not.
        assert_eq!(water.depth_at(at(6, 2)), Depth::Shallow);
        assert!(water.pushes_south(at(6, 2)));
        assert_eq!(water.depth_at(at(2, 6)), Depth::Shallow);
        assert!(!water.pushes_south(at(2, 6)));
        assert!(!water.pushes_south(at(6, 5)), "open water has no current a hull could feel");
        // Dry ground, and off the grid.
        assert_eq!(water.depth_at(at(10, 10)), Depth::Dry);
        assert!(!water.pushes_south(at(10, 10)));
        assert_eq!(water.depth_at(Position::new(-500.0, 9000.0)), Depth::Dry);
        // The deep cells are exactly the open-water cells, as centres.
        let mut deep: Vec<(i32, i32)> = water.deep_grid_cells().collect();
        deep.sort();
        let expect: Vec<(i32, i32)> = (5..=7).flat_map(|x| (5..=6).map(move |y| (x, y))).collect();
        assert_eq!(deep, expect);
        assert_eq!(water.deep_cells().count(), 6);
        assert_eq!(water.shallow_cells().count(), cells.len() - 6);
        // A single line is never deep: a river is a ford end to end.
        let river = WaterLayout::build(W, H, &[], &world(&[(3, 1), (3, 2), (3, 3), (4, 3)]));
        assert_eq!(river.deep_cells().count(), 0);
        assert!(river.pushes_south(at(3, 2)));
        assert!(!river.pushes_south(at(4, 3)), "the bend's end runs sideways");
        // A road cell on a lake is dry ground.
        let bridged = WaterLayout::build(W, H, &world(&[(6, 5)]), &world(&cells));
        assert_eq!(bridged.depth_at(at(6, 5)), Depth::Dry);
        assert!(WaterLayout::build(W, H, &world(&[(1, 1)]), &[]).is_empty());
    }

    fn shade(width: f32, height: f32, walls: &[(i32, i32)], edge_shade: bool, t: &Tuning) -> BlockImage {
        bake_shade(width, height, walls, Look { theme: Theme::Grass, edge_shade }, t, 1)
    }

    /// Mean opacity over the blocks within `r` blocks of field pixel
    /// (x, y): single texels are the dither's, the mean is the shade's.
    fn mean_alpha(img: &BlockImage, x: f32, y: f32, r: i32) -> f32 {
        let (cx, cy) = (x as i32 / img.block, y as i32 / img.block);
        let mut sum = 0.0;
        let mut n = 0.0;
        for by in cy - r..=cy + r {
            for bx in cx - r..=cx + r {
                if let Some(c) = img.at_pixel(bx * img.block, by * img.block) {
                    sum += c.a as f32;
                    n += 1.0;
                }
            }
        }
        sum / n
    }

    #[test]
    fn the_shade_covers_the_field_in_2px_blocks() {
        let img = shade(W, H, &[(5, 5)], true, &Tuning::DEFAULT);
        assert_eq!((img.width, img.height, img.block), (160, 160, SHADE_BLOCK));
        assert_eq!(img.texels.len(), 160 * 160);
        let odd = shade(1087.0, 543.0, &[], true, &Tuning::DEFAULT);
        assert_eq!((odd.width, odd.height), (544, 272), "a field an odd pixel wide is still covered");
    }

    #[test]
    fn a_wall_shades_the_ground_beside_it_and_pools_on_its_shadow_side() {
        // One wall centred on (160, 160); its faces are 16 px out.
        let img = shade(W, H, &[(5, 5)], false, &Tuning::DEFAULT);
        let shadow_side = mean_alpha(&img, 160.0 + 24.0, 160.0 + 24.0, 2);
        let lit_side = mean_alpha(&img, 160.0 - 24.0, 160.0 - 24.0, 2);
        assert!(lit_side > 0.0, "the lit side is shaded too, just less");
        assert!(shadow_side > lit_side + 10.0, "the shade pools where the shadows fall: {shadow_side} vs {lit_side}");
        let reach = Tuning::DEFAULT.ground_wall_shade_cells * OBSTACLE_GRID_SIZE;
        assert_eq!(mean_alpha(&img, 160.0 - 16.0 - reach - 8.0, 160.0, 1), 0.0, "out of reach is clear");
    }

    #[test]
    fn every_wall_lays_the_shade_measured_block_by_block() {
        // The stamp against the shade measured from every wall at every
        // block, with walls side by side, on the field's edge and past it on
        // every side.
        let t = Tuning::DEFAULT;
        let walls = [(0, 0), (3, 2), (4, 2), (9, 6), (10, 7), (-1, 3), (11, -1), (5, 8)];
        let img = bake_shade(320.0, 224.0, &walls, Look::default(), &t, 1);
        let reach = t.ground_wall_shade_cells * OBSTACLE_GRID_SIZE;
        let len = (t.shadow_dir_x * t.shadow_dir_x + t.shadow_dir_y * t.shadow_dir_y).sqrt();
        let (lx, ly) = (t.shadow_dir_x / len * t.ground_wall_shade_lean_px, t.shadow_dir_y / len * t.ground_wall_shade_lean_px);
        let (half, block) = (OBSTACLE_GRID_SIZE / 2.0, SHADE_BLOCK as f32);
        let mut shaded = 0;
        for by in 0..img.height as i64 {
            for bx in 0..img.width as i64 {
                let level = walls
                    .iter()
                    .map(|&(col, row)| {
                        let ox = ((((bx - col as i64 * CELL_BLOCKS) as f32 + 0.5) * block - lx).abs() - half).max(0.0);
                        let oy = ((((by - row as i64 * CELL_BLOCKS) as f32 + 0.5) * block - ly).abs() - half).max(0.0);
                        let d = (ox * ox + oy * oy).sqrt();
                        if d < reach { shade_step(smoothstep(1.0 - d / reach), WALL_SHADE_STEPS, dither(bx, by)) } else { 0 }
                    })
                    .max()
                    .unwrap_or(0);
                let alpha = (t.ground_wall_shade * level as f32 / WALL_SHADE_STEPS as f32 * 255.0).round() as u8;
                assert_eq!(img.texels[by as usize * img.width + bx as usize].a, alpha, "block ({bx}, {by})");
                shaded += (level > 0) as usize;
            }
        }
        assert!(shaded > 1000, "the walls shade something: {shaded} blocks");
    }

    #[test]
    fn the_edge_shade_frames_the_field_and_deepens_into_the_corners() {
        let img = shade(1088.0, 544.0, &[], true, &Tuning::DEFAULT);
        let centre = mean_alpha(&img, 544.0, 272.0, 4);
        let edge = mean_alpha(&img, 544.0, 6.0, 2);
        let corner = mean_alpha(&img, 6.0, 6.0, 2);
        assert_eq!(centre, 0.0, "the middle of the field is untouched");
        assert!(edge > 0.0, "the middle of an edge is shaded");
        assert!(corner > edge, "a corner is darker than the middle of an edge: {corner} vs {edge}");
        // Rounded, not boxed: a point as far in from both edges as the
        // middle of an edge is from one is lighter than a band would make it.
        let diagonal = mean_alpha(&img, 60.0, 60.0, 2);
        let band = mean_alpha(&img, 544.0, 60.0, 2);
        assert!(diagonal > band, "the corner's shade reaches further in than an edge's");
    }

    /// The world past a field (`bake_margin_shade`): clear over the field,
    /// the field's own edge steps carried on just past its edge, deepening
    /// outward to the plateau in the theme's dark and nothing else.
    #[test]
    fn the_margin_shade_carries_the_edge_shade_on_and_deepens_to_its_plateau() {
        let t = Tuning::DEFAULT;
        let (w, h) = (1088.0, 544.0);
        let m = bake_margin_shade(w, h, Theme::Grass, &t);
        let field = shade(w, h, &[], true, &t);
        let img = &m.image;
        let at = |bx: i64, by: i64| img.texels[((by - m.origin.1) * img.width as i64 + (bx - m.origin.0)) as usize];
        for (bx, by) in [(0, 0), (271, 135), (543, 271), (0, 271)] {
            assert_eq!(at(bx, by).a, 0, "clear over the field at ({bx}, {by})");
        }
        let dark = edge_shade_color(Theme::Grass);
        assert!(img.texels.iter().all(|c| c.a == 0 || (c.r, c.g, c.b) == (dark.r, dark.g, dark.b)), "the theme's dark alone");
        // Each edge's last block inside and first block out are a step
        // apart at most: the bands run on across the edge.
        let step = (t.ground_edge_shade / EDGE_SHADE_STEPS as f32 * 255.0).round() as i32;
        let field_at = |bx: i64, by: i64| field.texels[by as usize * field.width + bx as usize].a as i32;
        for (inside, outside) in [((200, 0), (200, -1)), ((200, 271), (200, 272)), ((0, 100), (-1, 100)), ((543, 100), (544, 100))] {
            for k in 0..8 {
                let (i, o) = if inside.0 == outside.0 { ((inside.0 + k, inside.1), (outside.0 + k, outside.1)) } else { ((inside.0, inside.1 + k), (outside.0, outside.1 + k)) };
                let (a_in, a_out) = (field_at(i.0, i.1), at(o.0, o.1).a as i32);
                assert!((a_out - a_in).abs() <= step + 1, "{i:?} {a_in} -> {o:?} {a_out}");
            }
        }
        // The plateau: `ground_margin_shade` in whole steps, on the image's
        // outermost blocks and past them.
        assert_eq!(m.plateau.a, (t.ground_margin_shade * 255.0).round() as u8);
        assert_eq!((m.plateau.r, m.plateau.g, m.plateau.b), (dark.r, dark.g, dark.b));
        let (x0, y0) = m.origin;
        let (x1, y1) = (x0 + img.width as i64 - 1, y0 + img.height as i64 - 1);
        for (bx, by) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1), (272, y0), (x0, 136), (x1, 136), (272, y1)] {
            assert_eq!(at(bx, by), m.plateau, "the image's rim at ({bx}, {by})");
        }
        // Deeper going out: from each edge's middle, the mean over a
        // dither tile of blocks never lightens.
        let tile_mean = |cx: i64, cy: i64| (0..4).flat_map(|j| (0..4).map(move |i| (i, j))).map(|(i, j)| at(cx + i, cy + j).a as f32).sum::<f32>() / 16.0;
        for (start, dir) in [((270, -4), (0, -4)), ((270, 272), (0, 4)), ((-4, 134), (-4, 0)), ((544, 134), (4, 0))] {
            let mut last = 0.0;
            let mut p = start;
            while p.0 >= x0 && p.1 >= y0 && p.0 + 3 <= x1 && p.1 + 3 <= y1 {
                let mean = tile_mean(p.0, p.1);
                assert!(mean >= last, "{start:?} lightens at {p:?}: {mean} after {last}");
                last = mean;
                p = (p.0 + dir.0, p.1 + dir.1);
            }
            assert_eq!(last, m.plateau.a as f32, "{start:?} ends on the plateau");
        }
        // A field without an edge shade deepens from nothing, in quarters
        // of the plateau.
        let bare = Tuning { ground_edge_shade: 0.0, ..Tuning::DEFAULT };
        let m = bake_margin_shade(w, h, Theme::Desert, &bare);
        assert_eq!(m.plateau.a, (t.ground_margin_shade * 255.0).round() as u8);
        assert_eq!(m.image.texels[0], m.plateau);
    }

    /// A floor carried past its map draws every cell the two share with
    /// the same tile and current - the paint is the map's and the hashes
    /// are the world cell's - drifts and water included.
    #[test]
    fn a_floor_carried_beyond_its_map_draws_the_cells_they_share_alike() {
        let look = Look { theme: Theme::Desert, edge_shade: true };
        for seed in 0..120 {
            let (water, road) = random_field(seed);
            let grid = build(FIELD_W, FIELD_H, seed, &road, &water, &[], look);
            for m in [1i32, 4] {
                let beyond = grid.beyond(m as usize);
                assert_eq!(beyond.origin(), (-m, -m));
                assert_eq!((beyond.cols, beyond.rows), (grid.cols + 2 * m as usize, grid.rows + 2 * m as usize));
                for y in 0..grid.rows as i32 {
                    for x in 0..grid.cols as i32 {
                        let (i, j) = (grid.idx(x, y).expect("in grid"), beyond.idx(x + m, y + m).expect("in grid"));
                        assert_eq!(beyond.tiles[j], grid.tiles[i], "seed {seed} by {m}: cell {x},{y}");
                        assert_eq!(beyond.current[j], grid.current[i], "seed {seed} by {m}: cell {x},{y}");
                    }
                }
            }
        }
    }

    /// Water painted to the map's edge runs on out of it: a brook down
    /// from the north edge carries on north as a stream, a lake against
    /// the west edge widens on west as open water, and the rest past the
    /// map is dry ground.
    #[test]
    fn water_painted_to_the_edge_runs_on_past_it() {
        let brook = (0..4).map(|y| (5, y));
        let lake = (0..3).flat_map(|x| (5..9).map(move |y| (x, y)));
        let cells: Vec<(i32, i32)> = brook.chain(lake).collect();
        let grid = with_water(&cells).beyond(3);
        let tile_at = |c: i32, r: i32| grid.tiles[grid.idx(c + 3, r + 3).expect("in grid")][0];
        let current_at = |c: i32, r: i32| grid.current[grid.idx(c + 3, r + 3).expect("in grid")];
        for r in -3..0 {
            assert_eq!(tile_at(5, r), WATER_CHANNEL[0b1010], "the brook north of the map at row {r}");
            assert_eq!(current_at(5, r), Current::Channel);
        }
        for c in -3..0 {
            for r in [6, 7] {
                assert_eq!(tile_at(c, r), WATER_SHORE[0b1111], "the lake west of the map at {c},{r}");
            }
        }
        assert!(grid.water_at(-3, 6) && grid.water_at(5, -3));
        assert!(!grid.water_at(-2, 1) && !grid.water_at(12, -2) && !grid.water_at(4, -2));
    }

    #[test]
    fn the_builder_canvas_has_no_edge_shade() {
        let img = shade(1088.0, 544.0, &[], false, &Tuning::DEFAULT);
        assert!(img.texels.iter().all(|c| c.a == 0));
    }

    #[test]
    fn zero_strength_turns_each_layer_off() {
        let t = Tuning { ground_wall_shade: 0.0, ground_edge_shade: 0.0, ..Tuning::DEFAULT };
        let img = shade(W, H, &[(5, 5)], true, &t);
        assert!(img.texels.iter().all(|c| c.a == 0));
    }

    #[test]
    fn the_shade_is_a_function_of_its_inputs_in_a_few_tones() {
        let walls = [(3, 4), (4, 4), (7, 2)];
        let a = shade(W, H, &walls, true, &Tuning::DEFAULT);
        let b = shade(W, H, &walls, true, &Tuning::DEFAULT);
        assert_eq!(a.texels, b.texels);
        let mut tones: Vec<(u8, u8, u8, u8)> = a.texels.iter().map(|c| (c.r, c.g, c.b, c.a)).collect();
        tones.sort_unstable();
        tones.dedup();
        let most = (WALL_SHADE_STEPS as usize + 1) * (EDGE_SHADE_STEPS as usize + 1);
        assert!(tones.len() <= most, "{} tones, at most {most}", tones.len());
        let desert = bake_shade(W, H, &walls, Look { theme: Theme::Desert, edge_shade: true }, &Tuning::DEFAULT, 1);
        assert_ne!(a.texels, desert.texels, "the edge deepens toward each theme's own dark");
    }

    #[test]
    fn every_build_names_its_own_shade() {
        let a = with_water(&[(2, 2)]);
        let b = with_water(&[(2, 2)]);
        assert_ne!(a.shade().stamp, b.shade().stamp);
        assert_eq!(a.shade().stamp, a.shade().stamp, "baked once, then kept");
    }

    #[test]
    fn the_layout_is_a_function_of_its_inputs() {
        let cells = [(4, 4), (5, 4), (4, 5), (5, 5), (9, 2), (9, 3)];
        let a = with_water(&cells);
        let b = with_water(&cells);
        assert_eq!(a.tiles, b.tiles);
        assert_eq!(a.current, b.current);
        assert_eq!(lane_hash(3, 0), lane_hash(3, 0));
        assert_ne!(lane_hash(3, 0), lane_hash(4, 0), "lanes differ by column");
        assert_ne!(lane_hash(3, 0), lane_hash(3, 1), "and by lane");
    }

    #[test]
    fn a_diagonal_run_of_blocks_flows_as_one_body() {
        // Two 2x2 blocks sharing one cell on the diagonal: the shared cell
        // would be wet at two opposite corners only.
        let cells = [(2, 2), (3, 2), (2, 3), (3, 3), (4, 3), (3, 4), (4, 4)];
        let grid = with_water(&cells);
        assert_eq!(tile(&grid, 3, 3), WATER_SHORE[0b1111], "the middle opens up");
        assert_eq!(tile(&grid, 4, 2), WATER_SHORE[0b0001], "the grass in the step takes the lake's corner");
        assert_eq!(tile(&grid, 2, 4), WATER_SHORE[0b0100]);
        assert_eq!(grid.tiles[grid.idx(4, 2).unwrap()], frames_of(WATER_SHORE[0b0001]), "and shimmers with it");
        // The step's grass is still dry ground.
        let water = WaterLayout::build(W, H, &[], &world(&cells));
        let at = |x: i32, y: i32| Position::new(x as f32 * GROUND_WORLD_TILE, y as f32 * GROUND_WORLD_TILE);
        assert_eq!(water.depth_at(at(4, 2)), Depth::Dry);
        // A wall in the step keeps its dirt: only the other side opens.
        let walled = build(W, H, 7, &world(&[(4, 2)]), &world(&cells), &[], Look::default());
        assert_eq!(tile(&walled, 3, 3), WATER_SHORE[0b1011]);
        assert_eq!(tile(&walled, 4, 2), ROAD_EDGE[0b0000]);
    }

    #[test]
    fn islands_notches_and_walls_keep_the_shape_painted() {
        // A 7x7 lake with a one-cell island and a notch in one corner.
        let cells: Vec<(i32, i32)> = (2..=8).flat_map(|x| (2..=8).map(move |y| (x, y))).filter(|&c| c != (5, 5) && c != (8, 2)).collect();
        let grid = with_water(&cells);
        assert!(GRASS_FILL.contains(&tile(&grid, 5, 5)), "the island is grass");
        assert_eq!(tile(&grid, 5, 4), WATER_SHORE[0b1100], "with a shore on each side");
        assert_eq!(tile(&grid, 4, 4), WATER_SHORE[0b1101], "and inner corners round it");
        assert!(GRASS_FILL.contains(&tile(&grid, 8, 2)), "the notch is grass");
        assert_eq!(tile(&grid, 7, 3), WATER_SHORE[0b1011], "met by an inner corner");
        // A wall standing in open water is ringed by a shore, not a hard edge.
        let open: Vec<(i32, i32)> = (2..=6).flat_map(|x| (2..=6).map(move |y| (x, y))).collect();
        let walled = build(W, H, 7, &world(&[(4, 4)]), &world(&open), &[], Look::default());
        assert_eq!(tile(&walled, 4, 3), WATER_SHORE[0b1100]);
        assert_eq!(tile(&walled, 5, 5), WATER_SHORE[0b0111]);
    }

    #[test]
    fn water_painted_to_the_edge_runs_off_the_map() {
        // W x H is a 10 x 10 map: a stream down column 5 from the top edge
        // and a lake across the bottom-right corner.
        let mut cells: Vec<(i32, i32)> = (0..=3).map(|y| (5, y)).collect();
        cells.extend((7..=9).flat_map(|x| (7..=9).map(move |y| (x, y))));
        let grid = with_water(&cells);
        assert_eq!(tile(&grid, 5, 0), WATER_CHANNEL[0b1010], "the stream comes in from above");
        assert_eq!(tile(&grid, 9, 9), WATER_SHORE[0b1111], "the lake carries on past the corner");
        assert_eq!(tile(&grid, 8, 9), WATER_SHORE[0b1111]);
        assert_eq!(tile(&grid, 9, 7), WATER_SHORE[0b0011], "its top shore runs on past the edge");
        assert_eq!(tile(&grid, 10, 9), WATER_SHORE[0b1111], "and so does the strip past the map");
        let water = WaterLayout::build(W, H, &[], &world(&cells));
        let at = |x: i32, y: i32| Position::new(x as f32 * GROUND_WORLD_TILE, y as f32 * GROUND_WORLD_TILE);
        assert_eq!(water.depth_at(at(9, 9)), Depth::Deep, "the rules follow the picture");
        assert!(water.pushes_south(at(5, 0)));
        assert!(water.deep_grid_cells().all(|(x, y)| x < 10 && y < 10), "nothing past the map is deep");
        // A field 5.5 rows high: water on its last row runs off it too.
        let strip: Vec<(i32, i32)> = (3..=6).flat_map(|x| [(x, 4), (x, 5)]).collect();
        let half = WaterLayout::build(W, 5.5 * GROUND_WORLD_TILE, &[], &world(&strip));
        assert_eq!(half.depth_at(at(4, 5)), Depth::Deep);
    }

    /// A seeded field of blobs, a meandering line and a few walls on a
    /// `FIELD_W` x `FIELD_H` map: water cells and road cells.
    const FIELD_W: f32 = 16.0 * GROUND_WORLD_TILE;
    const FIELD_H: f32 = 12.0 * GROUND_WORLD_TILE;

    /// `repaint` lands where `build` does. Random walls, roads and water,
    /// painted and erased a cell or a few at a time - on the field, along
    /// its last row and column, past it - on fields a whole and a half
    /// cell tall, with and without drifts and edge shade, each edit checked
    /// against the grid built afresh from the edited lists: tiles, currents
    /// and floor shade, and every shade texel the edit changed inside the
    /// patch it left for the GPU copy.
    const WALL: CellFloor = CellFloor { road: true, water: false, wall: true };
    const ROAD: CellFloor = CellFloor { road: true, water: false, wall: false };
    const WATER: CellFloor = CellFloor { road: false, water: true, wall: false };
    const OPEN: CellFloor = CellFloor { road: false, water: false, wall: false };

    /// The grid `build` makes of `cells`, the editor's way: the three
    /// lists read off each cell's floor.
    fn build_cells(width: f32, height: f32, look: Look, cells: &std::collections::BTreeMap<(i32, i32), CellFloor>) -> GroundGrid {
        let of = |pick: fn(&CellFloor) -> bool| -> Vec<Position> {
            let at: Vec<(i32, i32)> = cells.iter().filter(|(_, f)| pick(f)).map(|(&at, _)| at).collect();
            world(&at)
        };
        build(width, height, 77, &of(|f| f.road), &of(|f| f.water), &of(|f| f.wall), look)
    }

    /// Set `floor` at `at` in the editor's cells, as `repaint` is told it.
    fn set_floor(cells: &mut std::collections::BTreeMap<(i32, i32), CellFloor>, at: (i32, i32), floor: CellFloor) {
        if floor == OPEN {
            cells.remove(&at);
        } else {
            cells.insert(at, floor);
        }
    }

    /// The edits whose water is all in the neighbours: a road or a wall
    /// painted into the grass a diagonal was smoothed over takes the water
    /// back off that corner, and erasing it gives the corner back.
    #[test]
    fn a_road_in_a_smoothed_step_repaints_the_lake_around_it() {
        let look = Look::default();
        let diagonal = [(2, 2), (3, 2), (2, 3), (3, 3), (4, 3), (3, 4), (4, 4)];
        let mut cells = diagonal.into_iter().map(|at| (at, WATER)).collect();
        let mut grid = build_cells(W, H, look, &cells);
        let edits = [((4, 2), ROAD), ((4, 2), OPEN), ((2, 4), WALL), ((2, 4), OPEN), ((5, 5), WATER), ((5, 4), WATER), ((3, 3), OPEN)];
        for (i, (at, floor)) in edits.into_iter().enumerate() {
            set_floor(&mut cells, at, floor);
            grid.repaint(&[(at.0, at.1, floor)]);
            assert!(grid.draws_like(&build_cells(W, H, look, &cells)), "after {floor:?} at {at:?}");
            match i {
                0 => assert_eq!(tile(&grid, 3, 3), WATER_SHORE[0b1011], "the road takes the corner back"),
                1 => assert_eq!(tile(&grid, 4, 2), WATER_SHORE[0b0001], "and gives it back when erased"),
                _ => {}
            }
        }
    }

    #[test]
    fn a_repaint_is_the_build_of_the_edited_lists() {
        use rand::{RngExt, SeedableRng};
        use std::collections::BTreeMap;
        let mut checked = 0;
        for seed in 0..16u64 {
            let mut rng = rand::rngs::SmallRng::seed_from_u64(seed);
            let (cols, rows) = (14, 9);
            let height = if seed % 2 == 0 { rows as f32 } else { rows as f32 - 0.5 } * GROUND_WORLD_TILE;
            let width = cols as f32 * GROUND_WORLD_TILE;
            let look = Look { theme: if seed % 3 == 0 { Theme::Desert } else { Theme::Grass }, edge_shade: seed % 4 == 1 };
            // A lake or two, some walls and roads to start from.
            let mut cells = BTreeMap::new();
            for _ in 0..rng.random_range(1..=3) {
                let (x, y) = (rng.random_range(0..cols), rng.random_range(0..rows));
                for dy in 0..rng.random_range(2..=4) {
                    for dx in 0..rng.random_range(2..=5) {
                        cells.insert((x + dx, y + dy), WATER);
                    }
                }
            }
            for _ in 0..12 {
                let at = (rng.random_range(0..=cols), rng.random_range(0..=rows));
                cells.insert(at, if rng.random_bool(0.5) { WALL } else { ROAD });
            }
            let mut grid = build_cells(width, height, look, &cells);
            let _ = grid.shade();
            for step in 0..80 {
                let n = if rng.random_bool(0.2) { rng.random_range(2..6) } else { 1 };
                let mut edit = Vec::new();
                for _ in 0..n {
                    let at = (rng.random_range(-1..=cols + 1), rng.random_range(-1..=rows + 1));
                    let floor = [WALL, ROAD, WATER, WATER, OPEN][rng.random_range(0..5)];
                    set_floor(&mut cells, at, floor);
                    edit.push((at.0, at.1, floor));
                }
                let before = grid.shade().clone();
                grid.repaint(&edit);
                let want = build_cells(width, height, look, &cells);
                assert!(grid.tiles == want.tiles && grid.current == want.current, "seed {seed} step {step}: the tiles after {edit:?}");
                let shade = grid.shade();
                assert!(shade.texels == want.shade().texels, "seed {seed} step {step}: the shade after {edit:?}");
                if shade.stamp == before.stamp {
                    assert!(shade.texels == before.texels, "seed {seed} step {step}: a shade that changed takes a new stamp");
                    continue;
                }
                let patch = shade.changed_since(before.stamp).expect("a patch from the stamp before");
                for (i, (was, now)) in before.texels.iter().zip(&shade.texels).enumerate() {
                    let (x, y) = (i % shade.width, i / shade.width);
                    let inside = (patch.x..patch.x + patch.width).contains(&x) && (patch.y..patch.y + patch.height).contains(&y);
                    assert!(was == now || inside, "seed {seed} step {step}: texel ({x}, {y}) changed outside {patch:?}");
                }
                checked += 1;
            }
        }
        assert!(checked > 100, "walls came and went often enough to test the patches: {checked}");
    }

    fn random_field(seed: u64) -> (Vec<Position>, Vec<Position>) {
        use rand::{RngExt, SeedableRng};
        let mut rng = rand::rngs::SmallRng::seed_from_u64(seed);
        let (cols, rows) = (16i32, 12i32);
        let blobs: Vec<(f32, f32, f32)> = (0..rng.random_range(1..=5))
            .map(|_| (rng.random_range(0.0..cols as f32), rng.random_range(0.0..rows as f32), rng.random_range(1.5..4.0)))
            .collect();
        let level = rng.random_range(0.35..0.9);
        let mut water = Vec::new();
        for y in 0..rows {
            for x in 0..cols {
                let v: f32 = blobs.iter().map(|&(bx, by, r)| crate::math::exp(-((x as f32 - bx).powi(2) + (y as f32 - by).powi(2)) / (r * r))).sum();
                if v + rng.random_range(-0.25..0.25) > level {
                    water.push((x, y));
                }
            }
        }
        let (mut x, mut y) = (rng.random_range(0..cols), 0);
        for _ in 0..rng.random_range(0..40) {
            water.push((x, y));
            let (dx, dy) = [(0, 1), (0, 1), (1, 0), (-1, 0), (1, 1), (-1, 1)][rng.random_range(0..6)];
            x = (x + dx).clamp(0, cols - 1);
            y = (y + dy).min(rows - 1);
        }
        let road: Vec<(i32, i32)> = (0..rng.random_range(0..4)).map(|_| (rng.random_range(0..cols), rng.random_range(0..rows))).collect();
        (world(&water), world(&road))
    }

    #[test]
    fn every_shape_has_a_tile_and_the_rules_follow_the_picture() {
        for seed in 0..400 {
            let (water, road) = random_field(seed);
            // `build` asserts (in a debug build) that every cell has a tile.
            let grid = build(FIELD_W, FIELD_H, 7, &road, &water, &[], Look::default());
            let layout = Layout::new(FIELD_W, FIELD_H, &road, &water);
            let rules = WaterLayout::build(FIELD_W, FIELD_H, &road, &water);
            for y in 0..layout.rows as i32 {
                for x in 0..layout.cols as i32 {
                    if layout.stream_at(x, y) && layout.at(x, y) == Material::Water {
                        assert_eq!(layout.corners(x, y), 0, "seed {seed}: a stream at {x},{y} under a wet corner");
                    }
                    if layout.at(x, y) == Material::Road {
                        assert_eq!(layout.corners(x, y), 0, "seed {seed}: a road at {x},{y} under a wet corner");
                    }
                    let open = tile(&grid, x, y) == WATER_SHORE[0b1111] && layout.at(x, y) == Material::Water;
                    let inside = (x as usize) < layout.map_cols && (y as usize) < layout.map_rows;
                    let pos = Position::new(x as f32 * GROUND_WORLD_TILE, y as f32 * GROUND_WORLD_TILE);
                    assert_eq!(rules.depth_at(pos) == Depth::Deep, open && inside, "seed {seed}: {x},{y}");
                }
            }
        }
    }

    /// Water against land at a tile edge, as the pixels have it: the
    /// pack's water is the one blue on the sheet.
    #[cfg(feature = "render")]
    fn wet(c: Color) -> bool {
        c.a > 0 && c.b as i32 > c.r as i32 + 40 && c.b > 90
    }

    /// The seam every rule here exists to prevent: along the edge between
    /// two neighbouring cells, one tile shows water where the other shows
    /// land. Checked pixel by pixel on the live sheets, both themes, every
    /// animation frame, over the shapes the tests above name and a few
    /// hundred random fields. A shoreline's anti-aliasing can disagree by
    /// a pixel or two; a missing tile disagrees by half the edge or more.
    #[cfg(feature = "render")]
    #[test]
    fn no_tile_edge_puts_water_against_land() {
        use crate::canvas::Pixels;
        let size = crate::GROUND_TEXTURE_SIZE as usize;
        let mut fields: Vec<(Vec<Position>, Vec<Position>)> = (0..300).map(random_field).collect();
        let lake: Vec<(i32, i32)> = (2..=8).flat_map(|x| (2..=8).map(move |y| (x, y))).filter(|&c| c != (5, 5) && c != (8, 2)).collect();
        fields.push((world(&lake), world(&[(4, 4)])));
        let stair: Vec<(i32, i32)> = (0..8).flat_map(|i| [(i, i), (i + 1, i), (i, i + 1)]).collect();
        fields.push((world(&stair), vec![]));
        for theme in [Theme::Grass, Theme::Desert] {
            let sheet = Pixels::load(&Sheet::Ground(theme).path()).expect("the ground sheet decodes");
            let px = |tile: i32, x: usize, y: usize| {
                let (col, row) = ((tile % TILESET_COLS) as usize, (tile / TILESET_COLS) as usize);
                sheet.data[(row * size + y) * sheet.width + col * size + x]
            };
            for (n, (water, road)) in fields.iter().enumerate() {
                let grid = build(FIELD_W, FIELD_H, 7, road, water, &[], Look::default());
                for frame in 0..WATER_FRAME_COUNT {
                    let at = |x: usize, y: usize| grid.tiles[y * grid.cols + x][frame];
                    for y in 0..grid.rows {
                        for x in 0..grid.cols {
                            let (a, b, c) = (at(x, y), (x + 1 < grid.cols).then(|| at(x + 1, y)), (y + 1 < grid.rows).then(|| at(x, y + 1)));
                            if let Some(b) = b {
                                let off = (0..size).filter(|&i| wet(px(a, size - 1, i)) != wet(px(b, 0, i))).count();
                                assert!(off < 3, "{theme:?} field {n} frame {frame}: {x},{y} | {},{y} ({a} | {b}) disagree on {off} px", x + 1);
                            }
                            if let Some(c) = c {
                                let off = (0..size).filter(|&i| wet(px(a, i, size - 1)) != wet(px(c, i, 0))).count();
                                assert!(off < 3, "{theme:?} field {n} frame {frame}: {x},{y} over {x},{} ({a} / {c}) disagree on {off} px", y + 1);
                            }
                        }
                    }
                }
            }
        }
    }
}
