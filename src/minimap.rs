//! The map at a glance (docs/large-maps-follow-camera.md sections 7, 9 and
//! 15): the play minimap under the top-right corner cluster on tablets and
//! desktops, and the builder's navigator on every device. Headless, the
//! `hud.rs` and `indicators.rs` pattern: plain values the renderer paints
//! (`render::minimap`) and nothing else reads.
//!
//! Three parts:
//!
//! - **The image** (`Minimap`): the map as one texel per cell, a palette
//!   step per class of what the cell is (`Class`) - the ground of the map's
//!   theme, tall grass, road, water by depth, a gate, and the solid tile on
//!   it by material class. It is a `canvas::BlockImage`, so its GPU copy
//!   (`render::canvas::BlockTexture`) takes only the texels a change
//!   touched: the builder repaints the cells a stroke reached (`repaint`,
//!   after `GroundGrid::repaint`, whose answer names every cell whose water
//!   can have changed depth), and a round's image (`RoundMinimap`) is baked
//!   once per round and patched where a tile died. Texel `(col, row)` is
//!   the map cell centred on world `(col, row) * 32` - the cell
//!   `map::world_to_cell` names - so the field's own rectangle starts half
//!   a texel in (`source`) and the half cells along its edges show as half
//!   cells, as they do on the field.
//! - **The marks** (`Marks`, `picture`): what goes over the image each
//!   frame - the camera's view, every seat in its ring colour with its
//!   number, the frogs, the gates flashing while a wave rolls in, and the
//!   enemies the screen's off-screen arrows could point at
//!   (`indicators::Indicators::in_sight`: the one concealment rule, tall
//!   grass and the sky's sight), so the minimap never shows what the field
//!   hides. Terrain, not light: no sky darkens it.
//! - **The rules** (`MinimapRules`): whether a screen shows it - never on a
//!   phone (`is_phone`), and the `minimap_show` knob over everything - and
//!   how large (`size_pt`), in UI points.
//!
//! It reads a `Game` (a local round or a replica) or a `MapFile` and never
//! writes either; no RNG.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::canvas::BlockImage;
use crate::framing::Screen;
use crate::frog::{Frog, Side};
use crate::ground::Depth;
use crate::indicators::{Fill, Indicators, Label};
use crate::map::{CellObject, MapFile, Theme};
use crate::math::{Color, Rectangle, Vec2};
use crate::obstacle::{Material, Obstacle};
use crate::simulation::Game;
use crate::tank::Tank;
use crate::tuning::{Tuning, tuning};
use crate::{OBSTACLE_GRID_SIZE, Position};

/// A map cell's side in world pixels: a texel of the image.
const CELL: f32 = OBSTACLE_GRID_SIZE;

// ---- the rules ----------------------------------------------------------------

/// When the play minimap is drawn (`minimap_show`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    Never,
    /// On a tablet or a desktop, never on a phone (`MinimapRules::is_phone`).
    NotOnPhones,
    Always,
}

/// The minimap's knobs (the `minimap` tuning group), read once a frame and
/// passed in, so a test states the rules it runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinimapRules {
    pub show: Show,
    /// `minimap_phone_short_pt`: a screen whose short side is under this
    /// many points is a phone's.
    pub phone_short_pt: f32,
    /// `minimap_phone_short_mm`: so is one under this many millimetres,
    /// where the platform reports the screen's size.
    pub phone_short_mm: f32,
    /// `minimap_width_pt` and `minimap_height_pt`: the box the minimap is
    /// fitted into, in points.
    pub width_pt: f32,
    pub height_pt: f32,
}

/// How much of the box's fit a minimap keeps when its cells are snapped
/// down to whole half points: at least this, or it keeps the exact fit.
/// Half points are whole device pixels on a 2x screen, so a cell is drawn
/// as an even block rather than one a pixel wider than its neighbour.
const SNAP_KEEP: f32 = 0.75;

impl MinimapRules {
    /// The rules in the tuning table this frame.
    pub fn current() -> MinimapRules {
        MinimapRules::of(&tuning())
    }

    /// The rules in `t`.
    pub fn of(t: &Tuning) -> MinimapRules {
        MinimapRules {
            show: match t.minimap_show {
                0 => Show::Never,
                2 => Show::Always,
                _ => Show::NotOnPhones,
            },
            phone_short_pt: t.minimap_phone_short_pt,
            phone_short_mm: t.minimap_phone_short_mm,
            width_pt: t.minimap_width_pt,
            height_pt: t.minimap_height_pt,
        }
    }

    /// Whether `screen` is a phone's (docs/large-maps-follow-camera.md
    /// section 15): its short side under `phone_short_pt` points - a
    /// landscape phone is 360 to 440 of them, the smallest tablet 744 - or,
    /// where its physical size is known, under `phone_short_mm`
    /// millimetres, which catches a small panel that reports more points
    /// than a phone's.
    pub fn is_phone(&self, screen: &Screen) -> bool {
        let short = screen.size.0.min(screen.size.1);
        short < self.phone_short_pt || screen.mm_per_point.is_some_and(|mm| short * mm < self.phone_short_mm)
    }

    /// Whether the play minimap is drawn on `screen`: never, on anything
    /// but a phone, or always, as `minimap_show` says.
    pub fn shown_on(&self, screen: &Screen) -> bool {
        match self.show {
            Show::Never => false,
            Show::NotOnPhones => !self.is_phone(screen),
            Show::Always => true,
        }
    }

    /// The minimap's size in points for a field of `field` world pixels:
    /// the map's shape fitted into `width_pt` x `height_pt` - a long map
    /// as wide as the box and shorter, a tall one as tall and narrower -
    /// its cells snapped down to whole half points where that keeps
    /// `SNAP_KEEP` of the fit (80 x 45 cells is 160 x 90 points at two
    /// points a cell, the 96 x 54 study map 144 x 81 at one and a half).
    pub fn size_pt(&self, field: (f32, f32)) -> (f32, f32) {
        let cols = (field.0 / CELL).max(1.0);
        let rows = (field.1 / CELL).max(1.0);
        let fit = (self.width_pt.max(1.0) / cols).min(self.height_pt.max(1.0) / rows);
        let snapped = (fit * 2.0).floor() / 2.0;
        let per_cell = if snapped >= fit * SNAP_KEEP { snapped } else { fit };
        (cols * per_cell, rows * per_cell)
    }
}

// ---- the image -----------------------------------------------------------------

/// What a cell of the map is, as the minimap colours it: the floor, or the
/// solid tile standing on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Open ground in the map's theme.
    Ground,
    /// Tall grass: cover a tank can hide in.
    Grass,
    /// Road, and the dirt every wall stands on.
    Road,
    /// A ford: any water a hull can drive through.
    Shallow,
    /// Open lake water no hull enters.
    Deep,
    /// Water frozen over under snow.
    Ice,
    /// A wave gate.
    Gate,
    Brick,
    Iron,
    Wood,
    Glass,
    /// A sandbag, a drum or a fence.
    Prop,
    /// A tree or a pine.
    Tree,
    /// A defence tower, by the side it fights for.
    Tower(Side),
}

impl Class {
    /// The floor a map cell lays - `obj` what the cell holds, `depth` the
    /// water there: road under a wall as under a road cell, the water by
    /// its depth (a water cell past the map, where the water is picture
    /// only, a ford), tall grass, a gate, and open ground under everything
    /// else.
    pub fn floor(obj: Option<&CellObject>, depth: Depth) -> Class {
        match obj {
            Some(CellObject::Wall { .. } | CellObject::Road) => Class::Road,
            Some(CellObject::Gate) => Class::Gate,
            Some(CellObject::TallGrass) => Class::Grass,
            Some(CellObject::Water) => match depth {
                Depth::Deep => Class::Deep,
                Depth::Ice => Class::Ice,
                Depth::Dry | Depth::Shallow => Class::Shallow,
            },
            _ => Class::Ground,
        }
    }

    /// The class of a solid tile of `material`; `side` is the side a tower
    /// fights for.
    pub fn solid(material: Material, side: Side) -> Class {
        match material {
            Material::Brick => Class::Brick,
            Material::Iron => Class::Iron,
            Material::Wood => Class::Wood,
            Material::Glass => Class::Glass,
            Material::Sandbag | Material::Barrel | Material::Fence => Class::Prop,
            Material::Tree | Material::Pine => Class::Tree,
            Material::Tesla | Material::GunTower | Material::BioSlush => Class::Tower(side),
        }
    }

    /// The solid tile a map cell places, if it places one.
    pub fn solid_of(obj: &CellObject) -> Option<Class> {
        let side = obj.tower().map_or(Side::Player, |(_, side)| side);
        obj.material().map(|material| Class::solid(material, side))
    }

    /// The palette step a cell of this class is drawn in (docs/PALETTE.md)
    /// on a map of `theme`: the tiles in the colours they wear on the field
    /// - brick light grey (STONE_LT), iron dark grey (STONE_SHADE), timber
    /// orange (WOOD_MD), glass pale cyan (BLUE_PALE), props grey
    /// (STONE_MD), trees the darkest green (GREEN_DARKEST) - the floor a
    /// step or two under the marks drawn over it: the grass theme's ground,
    /// cover and road GREEN_DK, GREEN_SHADE and SAND_MD, the desert's
    /// SAND_MD, WOOD_ASH and WOOD_DK; fords BLUE_LT, open water BLUE_DK,
    /// ice STONE_PALE, a gate the indicators' amber (GOLD_BRIGHT). A tower
    /// is TEAL_LT on the player's side and RED_MD on the enemy's.
    pub fn color(self, theme: Theme) -> Color {
        let rgb = |r, g, b| Color::new(r, g, b, 255);
        let desert = theme == Theme::Desert;
        match self {
            Class::Ground if desert => rgb(0xB7, 0xA2, 0x48),
            Class::Ground => rgb(0x5F, 0x91, 0x4B),
            Class::Grass if desert => rgb(0x73, 0x62, 0x4D),
            Class::Grass => rgb(0x3D, 0x6E, 0x3F),
            Class::Road if desert => rgb(0x99, 0x65, 0x24),
            Class::Road => rgb(0xB7, 0xA2, 0x48),
            Class::Shallow => rgb(0x1E, 0xB3, 0xAE),
            Class::Deep => rgb(0x03, 0x8A, 0xAB),
            Class::Ice => rgb(0xF0, 0xF0, 0xF0),
            Class::Gate => rgb(0xEE, 0xA3, 0x43),
            Class::Brick => rgb(0xC1, 0xC1, 0xC1),
            Class::Iron => rgb(0x5A, 0x5A, 0x5A),
            Class::Wood => rgb(0xCA, 0x8A, 0x3B),
            Class::Glass => rgb(0x93, 0xEC, 0xE2),
            Class::Prop => rgb(0x9E, 0x9E, 0x96),
            Class::Tree => rgb(0x1C, 0x4C, 0x33),
            Class::Tower(Side::Player) => rgb(0x00, 0xBB, 0x8F),
            Class::Tower(Side::Enemy) => rgb(0xE4, 0x42, 0x19),
        }
    }
}

/// The cells the image holds across and down for a field of `field` world
/// pixels: every cell whose middle is on the field, the half cells along
/// its far edges included.
pub fn cells_of(field: (f32, f32)) -> (usize, usize) {
    let n = |side: f32| (side.max(0.0) / CELL).floor() as usize + 1;
    (n(field.0), n(field.1))
}

/// The texels of the image the field's own rectangle covers: from half a
/// texel in, the field's size in cells - what is drawn into the minimap's
/// rectangle.
pub fn source(field: (f32, f32)) -> Rectangle {
    Rectangle::new(0.5, 0.5, field.0.max(0.0) / CELL, field.1.max(0.0) / CELL)
}

/// Names every image a minimap bakes or patches, so a GPU copy of one is
/// never taken for another's (`render::canvas::BlockTexture`).
static NEXT_STAMP: AtomicU64 = AtomicU64::new(1);

fn next_stamp() -> u64 {
    NEXT_STAMP.fetch_add(1, Ordering::Relaxed)
}

/// The map as an image of one texel per cell (`Class::color`), and the
/// classes it was drawn from.
#[derive(Clone, Debug, Default)]
pub struct Minimap {
    /// The field the image covers, world pixels.
    field: (f32, f32),
    cols: usize,
    rows: usize,
    theme: Theme,
    /// Per cell, what lies on the floor and what stands on it.
    floor: Vec<Class>,
    solid: Vec<Option<Class>>,
    image: BlockImage,
}

impl Minimap {
    /// The minimap of `map`'s own cells: the floor each one lays with the
    /// water `depth` gives, and the tile each one places - the builder's
    /// canvas, which `depth` reads off its ground (`GroundGrid::depth`).
    pub fn of_map(map: &MapFile, depth: impl Fn(i32, i32) -> Depth) -> Minimap {
        let field = map.field_size();
        let (floor, solid) = classes_of_map(map, field, &depth);
        Minimap::of_classes(field, map.theme, floor, solid)
    }

    /// The minimap of the round `game` holds: its map's floor under its
    /// own sky's water (`Game::water`, frozen in the snow) and the tiles
    /// still standing in its world, wherever they are.
    pub fn of_round(game: &Game) -> Minimap {
        let field = game.map.field_size();
        let depth = |col: i32, row: i32| game.water.depth_at(crate::map::cell_to_world(col, row));
        let (floor, _) = classes_of_map(&game.map, field, &depth);
        let solid = solids_of_world(game, cells_of(field));
        Minimap::of_classes(field, game.map.theme, floor, solid)
    }

    fn of_classes(field: (f32, f32), theme: Theme, floor: Vec<Class>, solid: Vec<Option<Class>>) -> Minimap {
        let (cols, rows) = cells_of(field);
        let texels = floor.iter().zip(&solid).map(|(f, s)| s.unwrap_or(*f).color(theme)).collect();
        let image = BlockImage { width: cols, height: rows, block: 1, texels, stamp: next_stamp(), patches: Vec::new() };
        Minimap { field, cols, rows, theme, floor, solid, image }
    }

    /// The image: a texel per cell, row by row.
    pub fn image(&self) -> &BlockImage {
        &self.image
    }

    /// The field the image covers, world pixels.
    pub fn field(&self) -> (f32, f32) {
        self.field
    }

    /// The cells across and down.
    pub fn cells(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    /// What cell `(col, row)` shows: the tile on it, else its floor.
    pub fn class_at(&self, col: i32, row: i32) -> Option<Class> {
        self.index(col, row).map(|i| self.solid[i].unwrap_or(self.floor[i]))
    }

    fn index(&self, col: i32, row: i32) -> Option<usize> {
        (col >= 0 && row >= 0 && (col as usize) < self.cols && (row as usize) < self.rows).then(|| row as usize * self.cols + col as usize)
    }

    /// Make cells `cells` what `classify` says each is now - its floor and
    /// the tile on it - repainting only those texels, under a new stamp
    /// with a `BlockPatch` round the ones that changed colour, so the GPU
    /// copy uploads only them. A cell past the image is ignored. Whether a
    /// texel changed.
    pub fn repaint(&mut self, cells: impl IntoIterator<Item = (i32, i32)>, classify: impl Fn(i32, i32) -> (Class, Option<Class>)) -> bool {
        let mut changed = Vec::new();
        for (col, row) in cells {
            let Some(i) = self.index(col, row) else { continue };
            let (floor, solid) = classify(col, row);
            self.floor[i] = floor;
            self.solid[i] = solid;
            changed.push(i);
        }
        self.repaint_texels(changed)
    }

    /// The tiles standing on every cell, `solid` one per cell in row order
    /// (`solids_of_world`): what a round's world holds now. Repaints the
    /// cells whose tile came or went, as `repaint` does.
    fn set_solids(&mut self, solid: Vec<Option<Class>>) -> bool {
        if solid.len() != self.solid.len() {
            return false;
        }
        let changed: Vec<usize> = (0..solid.len()).filter(|&i| solid[i] != self.solid[i]).collect();
        self.solid = solid;
        self.repaint_texels(changed)
    }

    /// Recolour the texels `cells` (indices) from their classes, and note
    /// the rectangle round the ones whose colour moved as one patch.
    fn repaint_texels(&mut self, cells: Vec<usize>) -> bool {
        let mut bounds: Option<(usize, usize, usize, usize)> = None;
        for i in cells {
            let color = self.solid[i].unwrap_or(self.floor[i]).color(self.theme);
            if self.image.texels[i] == color {
                continue;
            }
            self.image.texels[i] = color;
            let (x, y) = (i % self.cols, i / self.cols);
            bounds = Some(match bounds {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
        let Some((x0, y0, x1, y1)) = bounds else { return false };
        self.image.patched(next_stamp(), x0, y0, x1 - x0 + 1, y1 - y0 + 1);
        true
    }
}

/// Every cell's floor and the tile `map` places on it, in row order, for a
/// field of `field` world pixels.
fn classes_of_map(map: &MapFile, field: (f32, f32), depth: &impl Fn(i32, i32) -> Depth) -> (Vec<Class>, Vec<Option<Class>>) {
    let (cols, rows) = cells_of(field);
    let mut floor = vec![Class::Ground; cols * rows];
    let mut solid = vec![None; cols * rows];
    for (col, row, obj) in map.iter_cells() {
        if col < 0 || row < 0 || col as usize >= cols || row as usize >= rows {
            continue;
        }
        let i = row as usize * cols + col as usize;
        floor[i] = Class::floor(Some(obj), depth(col, row));
        solid[i] = Class::solid_of(obj);
    }
    (floor, solid)
}

/// The tile standing on every cell of a `cells` image in `game`'s world,
/// in row order.
fn solids_of_world(game: &Game, cells: (usize, usize)) -> Vec<Option<Class>> {
    let (cols, rows) = cells;
    let mut solid = vec![None; cols * rows];
    for obstacle in game.world.query::<&Obstacle>().iter() {
        let (col, row) = crate::map::world_to_cell(obstacle.position);
        if col < 0 || row < 0 || col as usize >= cols || row as usize >= rows {
            continue;
        }
        let side = if obstacle.material.is_tower() { crate::tower::side_of_variant(obstacle.variant) } else { Side::Player };
        solid[row as usize * cols + col as usize] = Some(Class::solid(obstacle.material, side));
    }
    solid
}

/// What a round's still picture was made for - its minimap, the weather's
/// cell mask (`render::weather`): another seed, field, theme, map, sky or
/// name is another round's. A frame counter that went back is too (a
/// restart on a pinned seed), which a holder checks beside it.
#[derive(Clone, Debug, PartialEq)]
pub struct RoundKey {
    seed: u64,
    field: (u32, u32),
    theme: Theme,
    cells: usize,
    frozen: bool,
    name: Option<String>,
}

impl RoundKey {
    /// The round `game` holds.
    pub fn of(game: &Game) -> RoundKey {
        let (w, h) = game.map.field_size();
        RoundKey {
            seed: game.round_seed(),
            field: (w.to_bits(), h.to_bits()),
            theme: game.map.theme,
            cells: game.map.cells.len(),
            frozen: game.water.is_frozen(),
            name: game.map.name.clone(),
        }
    }
}

/// A round's minimap across frames (`app.rs` keeps one): baked once per
/// round - a new seed, map or sky, or a frame counter that went back, which
/// is a restart on a pinned seed - and patched in place when the round's
/// tiles change, which a count of them tells: a tile only ever goes, as a
/// wall broken, a drum blown, a tree burnt or a tower brought down.
#[derive(Clone, Debug, Default)]
pub struct RoundMinimap {
    minimap: Minimap,
    key: Option<RoundKey>,
    frame: u64,
    tiles: usize,
}

impl RoundMinimap {
    /// The minimap of the round `game` holds this frame.
    pub fn sync(&mut self, game: &Game) -> &Minimap {
        let key = RoundKey::of(game);
        let frame = game.frame();
        let tiles = game.world.query::<&Obstacle>().iter().count();
        if self.key.as_ref() != Some(&key) || frame < self.frame {
            self.minimap = Minimap::of_round(game);
            self.key = Some(key);
        } else if tiles != self.tiles {
            let solid = solids_of_world(game, self.minimap.cells());
            self.minimap.set_solids(solid);
        }
        self.tiles = tiles;
        self.frame = frame;
        &self.minimap
    }

    /// The minimap as last synced.
    pub fn minimap(&self) -> &Minimap {
        &self.minimap
    }
}

// ---- where things are on it --------------------------------------------------------

/// A world point on a minimap drawn in `rect` (points) for a field of
/// `field` world pixels: the rectangle is the field's own.
pub fn to_point(rect: Rectangle, field: (f32, f32), world: Position) -> Vec2 {
    let (w, h) = (field.0.max(1.0), field.1.max(1.0));
    Vec2::new(rect.x + world.x / w * rect.width, rect.y + world.y / h * rect.height)
}

/// The world point under `point` on a minimap drawn in `rect` for a field
/// of `field` world pixels, kept on the field: where the navigator sends
/// the builder's camera.
pub fn to_world(rect: Rectangle, field: (f32, f32), point: Vec2) -> Position {
    let fx = if rect.width > 0.0 { ((point.x - rect.x) / rect.width).clamp(0.0, 1.0) } else { 0.5 };
    let fy = if rect.height > 0.0 { ((point.y - rect.y) / rect.height).clamp(0.0, 1.0) } else { 0.5 };
    Position::new(fx * field.0, fy * field.1)
}

/// The part of a world rectangle a minimap in `rect` shows, in points: the
/// view's outline, clipped to the minimap. `None` where they do not meet.
pub fn rect_on(rect: Rectangle, field: (f32, f32), world: Rectangle) -> Option<Rectangle> {
    let a = to_point(rect, field, Position::new(world.x, world.y));
    let b = to_point(rect, field, Position::new(world.x + world.width, world.y + world.height));
    let (x0, y0) = (a.x.max(rect.x), a.y.max(rect.y));
    let (x1, y1) = (b.x.min(rect.x + rect.width), b.y.min(rect.y + rect.height));
    (x1 > x0 && y1 > y0).then(|| Rectangle::new(x0, y0, x1 - x0, y1 - y0))
}

// ---- the marks ------------------------------------------------------------------------

/// A seat on the minimap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeatMark {
    /// The owner slot: its ring colour (`tank::team_color`) and its number.
    pub seat: u8,
    /// Where its tank is, brought onto the field while it drives in.
    pub at: Position,
    /// A seat this screen plays.
    pub local: bool,
}

/// A live frog: the player's, or the enemy's a Hunt round is after.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrogMark {
    pub side: Side,
    pub at: Position,
}

/// A wave gate flashing while a tank rolls in through it
/// (`indicators::GateFlash`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GateMark {
    pub at: Position,
    /// 1 as a tank starts in or comes through, falling to 0.
    pub flash: f32,
}

/// What goes over the image this frame, in world pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marks {
    /// The world rectangle the camera shows.
    pub view: Option<Rectangle>,
    /// The second half's, while a couch's screen is split
    /// (`follow::Split`): both views are outlined.
    pub second_view: Option<Rectangle>,
    /// Every live seat, in seat order.
    pub seats: Vec<SeatMark>,
    pub frogs: Vec<FrogMark>,
    /// The enemies the screen's seats have in sight, each once.
    pub enemies: Vec<Position>,
    pub gates: Vec<GateMark>,
    /// The round clock the gates blink on.
    pub time: f32,
    /// Whether the seats carry their numbers: a round of two or more.
    pub numbered: bool,
}

impl Marks {
    /// This frame's marks for a screen showing `view` (the camera's world
    /// rectangle) of the round `game`, playing the seats `local`, whose
    /// indicators (`indicators::ScreenAwareness::shown`) say which enemies
    /// are in sight and which gates flash.
    pub fn gather(game: &Game, local: &[u8], shown: &[Indicators], view: Rectangle) -> Marks {
        let field = game.map.field_size();
        let seats = (0..game.players.count())
            .filter_map(|i| {
                let entity = game.seat(i)?;
                let tank = game.world.get::<&Tank>(entity).ok()?;
                let seat = i as u8;
                (!tank.is_wreck()).then(|| SeatMark { seat, at: onto_field(tank.position, field), local: local.contains(&seat) })
            })
            .collect();
        let frogs = [game.frog, game.enemy_frog]
            .into_iter()
            .flatten()
            .filter_map(|e| game.world.get::<&Frog>(e).ok().filter(|f| !f.is_dead()).map(|f| FrogMark { side: f.side, at: f.position }))
            .collect();
        Marks { view: Some(view), seats, frogs, time: game.time, numbered: game.players.count() > 1, ..Marks::from_shown(shown) }
    }

    /// The enemies and gates `shown` - one `Indicators` per seat the screen
    /// plays - hold: every enemy any of them has in sight, once, in the
    /// first seat's order, and the first seat's flashing gates, as the
    /// screen's arrows are the first seat's.
    pub fn from_shown(shown: &[Indicators]) -> Marks {
        let mut enemies: Vec<Position> = Vec::new();
        for at in shown.iter().flat_map(|ind| ind.in_sight.iter().copied()) {
            if !enemies.contains(&at) {
                enemies.push(at);
            }
        }
        let gates = shown
            .first()
            .map(|ind| ind.gates.iter().filter(|g| g.flash > 0.0).map(|g| GateMark { at: g.at, flash: g.flash }).collect())
            .unwrap_or_default();
        Marks { enemies, gates, ..Marks::default() }
    }
}

/// `p` brought onto a field of `field`: a tank driving in through a gate
/// lane stands just past the edge.
fn onto_field(p: Position, field: (f32, f32)) -> Position {
    Position::new(p.x.clamp(0.0, field.0.max(0.0)), p.y.clamp(0.0, field.1.max(0.0)))
}

// ---- the picture ------------------------------------------------------------------------

/// A mark's body and the near-black rim round it, in points: enemies the
/// smallest, then frogs and seats; a flashing gate's frame; the view's
/// outline.
const ENEMY_PT: i32 = 3;
const FROG_PT: i32 = 4;
const SEAT_PT: i32 = 4;
const RIM_PT: i32 = 1;
const GATE_FRAME_PT: i32 = 9;
const VIEW_LINE_PT: i32 = 1;

/// The view's outline: white at most of its strength, over its rim.
const VIEW_ALPHA: f32 = 0.85;

/// What a minimap paints over its image, in points: blocks, then labels on
/// their plates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Picture {
    pub fills: Vec<Fill>,
    pub labels: Vec<Label>,
}

/// The marks drawn on a minimap in `rect` (points) for a field of `field`
/// world pixels, each kept inside the rectangle, in the indicators' colours
/// (`indicators::HOSTILE` and its neighbours) with the near-black rim every
/// mark of theirs wears: the gates flashing in amber, blinking at
/// `indicator_gate_blink_hz` on the round clock; the view's outline in
/// white, both halves' on a split screen; the enemies in sight in the
/// hostile red; the frogs in the FROG gauge's green, the enemy frog rimmed
/// red; and the seats in their ring colours, the screen's own rimmed white
/// and the others numbered in a round of two or more - the numbers set in
/// `font` on dark plates.
pub fn picture(marks: &Marks, rect: Rectangle, field: (f32, f32), font: i32, t: &Tuning) -> Picture {
    use crate::indicators::{FROG_GREEN, GATE_AMBER, HOSTILE, RIM, WHITE};
    let mut out = Picture::default();
    // Inside the rectangle's last whole point, or on its corner where it
    // is under a point across.
    let at = |world: Position| {
        let p = to_point(rect, field, world);
        let x = p.x.clamp(rect.x, (rect.x + rect.width - 1.0).max(rect.x));
        let y = p.y.clamp(rect.y, (rect.y + rect.height - 1.0).max(rect.y));
        (x.floor() as i32, y.floor() as i32)
    };
    let inside = Rectangle::new(rect.x.ceil(), rect.y.ceil(), rect.width.floor(), rect.height.floor());

    if blink_on(marks.time, t.indicator_gate_blink_hz) {
        for gate in &marks.gates {
            let alpha = 0.4 + 0.6 * gate.flash.clamp(0.0, 1.0);
            let (x, y) = at(gate.at);
            let half = GATE_FRAME_PT / 2;
            frame(&mut out.fills, x - half - RIM_PT, y - half - RIM_PT, GATE_FRAME_PT + 2 * RIM_PT, RIM_PT, crate::pyro::alpha(RIM, alpha), inside);
            frame(&mut out.fills, x - half, y - half, GATE_FRAME_PT, RIM_PT, crate::pyro::alpha(GATE_AMBER, alpha), inside);
        }
    }
    for view in [marks.view, marks.second_view].into_iter().flatten().filter_map(|v| rect_on(rect, field, v)) {
        let (x0, y0) = (view.x.round() as i32, view.y.round() as i32);
        let (x1, y1) = ((view.x + view.width).round() as i32, (view.y + view.height).round() as i32);
        let w = (x1 - x0).max(2);
        let h = (y1 - y0).max(2);
        outline(&mut out.fills, x0 - RIM_PT, y0 - RIM_PT, w + 2 * RIM_PT, h + 2 * RIM_PT, RIM_PT, crate::pyro::alpha(RIM, VIEW_ALPHA), inside);
        outline(&mut out.fills, x0, y0, w, h, VIEW_LINE_PT, crate::pyro::alpha(WHITE, VIEW_ALPHA), inside);
    }
    for &enemy in &marks.enemies {
        square(&mut out.fills, at(enemy), ENEMY_PT, HOSTILE, RIM, inside);
    }
    for frog in &marks.frogs {
        let rim = if frog.side == Side::Enemy { HOSTILE } else { RIM };
        square(&mut out.fills, at(frog.at), FROG_PT, FROG_GREEN, rim, inside);
    }
    // The other seats, then the screen's own over them.
    let mut seats: Vec<&SeatMark> = marks.seats.iter().collect();
    seats.sort_by_key(|s| (s.local, s.seat));
    for seat in seats {
        let (x, y) = at(seat.at);
        let color = crate::tank::team_color(seat.seat);
        square(&mut out.fills, (x, y), SEAT_PT, color, if seat.local { WHITE } else { RIM }, inside);
        if marks.numbered && !seat.local {
            let text = (u32::from(seat.seat) + 1).to_string();
            let (w, h) = (crate::text::width(&text, font) + 2, font + 2);
            // Beside the mark on its right, or its left at the right edge,
            // and inside the minimap.
            let reach = SEAT_PT / 2 + RIM_PT + 1;
            let mut lx = x + reach + 1;
            if lx + w > (inside.x + inside.width) as i32 {
                lx = x - reach - w;
            }
            let lx = lx.clamp(inside.x as i32, ((inside.x + inside.width) as i32 - w).max(inside.x as i32));
            let ly = (y - h / 2).clamp(inside.y as i32, ((inside.y + inside.height) as i32 - h).max(inside.y as i32));
            out.labels.push(Label {
                text,
                x: lx + 1,
                y: ly + 1,
                size: font,
                color,
                plate: Fill { x: lx, y: ly, w, h, color: crate::pyro::alpha(RIM, 0.8) },
            });
        }
    }
    out
}

/// A square of `side` points round the point `at`, in `body`, with a rim
/// of `RIM_PT` round it in `rim`, cut to `bounds`.
fn square(out: &mut Vec<Fill>, at: (i32, i32), side: i32, body: Color, rim: Color, bounds: Rectangle) {
    let x = at.0 - side / 2;
    let y = at.1 - side / 2;
    push(out, x - RIM_PT, y - RIM_PT, side + 2 * RIM_PT, side + 2 * RIM_PT, rim, bounds);
    push(out, x, y, side, side, body, bounds);
}

/// A square frame `side` points across from (`x`, `y`), its line `line`
/// points thick.
fn frame(out: &mut Vec<Fill>, x: i32, y: i32, side: i32, line: i32, color: Color, bounds: Rectangle) {
    outline(out, x, y, side, side, line, color, bounds);
}

/// The outline of a `w` x `h` rectangle from (`x`, `y`), its line `line`
/// points thick, inside the rectangle.
#[allow(clippy::too_many_arguments)]
fn outline(out: &mut Vec<Fill>, x: i32, y: i32, w: i32, h: i32, line: i32, color: Color, bounds: Rectangle) {
    push(out, x, y, w, line, color, bounds);
    push(out, x, y + h - line, w, line, color, bounds);
    push(out, x, y + line, line, h - 2 * line, color, bounds);
    push(out, x + w - line, y + line, line, h - 2 * line, color, bounds);
}

/// One fill, cut to `bounds`; nothing where it falls outside them.
fn push(out: &mut Vec<Fill>, x: i32, y: i32, w: i32, h: i32, color: Color, bounds: Rectangle) {
    let (bx0, by0) = (bounds.x as i32, bounds.y as i32);
    let (bx1, by1) = ((bounds.x + bounds.width) as i32, (bounds.y + bounds.height) as i32);
    let (x0, y0) = (x.max(bx0), y.max(by0));
    let (x1, y1) = ((x + w).min(bx1), (y + h).min(by1));
    if x1 > x0 && y1 > y0 && color.a > 0 {
        out.push(Fill { x: x0, y: y0, w: x1 - x0, h: y1 - y0, color });
    }
}

/// Whether a blink `hz` times a second is in its lit half at round time
/// `time` - the indicators' gate blink.
fn blink_on(time: f32, hz: f32) -> bool {
    (time.max(0.0) * hz).rem_euclid(1.0) < 0.5
}

#[cfg(test)]
mod minimap_tests {
    use super::*;
    use crate::ground::GroundGrid;
    use crate::indicators::{Awareness, Scene, SeatView, TankView, ViewFrame};
    use crate::simulation::PlayerCount;

    fn rules() -> MinimapRules {
        MinimapRules::of(&Tuning::DEFAULT)
    }

    /// A small map with one of everything the image tells apart.
    fn sample() -> MapFile {
        MapFile::from_toml_str(
            r#"
            version = 1
            size = [12, 8]
            cells."1,1" = { kind = "wall", material = "brick" }
            cells."2,1" = { kind = "wall", material = "iron" }
            cells."3,1" = { kind = "wall", material = "wood" }
            cells."4,1" = { kind = "wall", material = "glass" }
            cells."5,1" = { kind = "sandbag" }
            cells."6,1" = { kind = "barrel" }
            cells."7,1" = { kind = "fence" }
            cells."8,1" = { kind = "tree" }
            cells."9,1" = { kind = "pine" }
            cells."10,1" = { kind = "tesla", side = "enemy" }
            cells."10,2" = { kind = "gun_tower" }
            cells."1,3" = { kind = "road" }
            cells."2,3" = { kind = "tall_grass" }
            cells."3,3" = { kind = "gate" }
            cells."4,4" = { kind = "water" }
            cells."5,4" = { kind = "water" }
            cells."6,4" = { kind = "water" }
            cells."4,5" = { kind = "water" }
            cells."5,5" = { kind = "water" }
            cells."6,5" = { kind = "water" }
            cells."4,6" = { kind = "water" }
            cells."5,6" = { kind = "water" }
            cells."6,6" = { kind = "water" }
            cells."10,5" = { kind = "water" }
            cells."11,0" = { kind = "start" }
            "#,
        )
        .expect("the sample map parses")
    }

    /// The image of the builder's canvas: a texel per cell, in the colour
    /// of the cell's class - the tiles by material class, road, cover, a
    /// gate, the water by the depth the ground reads (the middle of a 3 x 3
    /// lake open water, its rim a ford, a lone cell a stream) - and
    /// everything the map leaves empty or does not draw open ground.
    #[test]
    fn the_image_matches_the_maps_cells() {
        let map = sample();
        let (w, h) = map.field_size();
        let ground = crate::ground::build(w, h, 1, &[], &water_cells(&map), &[], crate::ground::Look::default());
        let mm = Minimap::of_map(&map, |c, r| ground.depth(c, r));
        assert_eq!(mm.cells(), (13, 9), "every cell whose middle is on the field");
        let expect = [
            ((1, 1), Class::Brick),
            ((2, 1), Class::Iron),
            ((3, 1), Class::Wood),
            ((4, 1), Class::Glass),
            ((5, 1), Class::Prop),
            ((6, 1), Class::Prop),
            ((7, 1), Class::Prop),
            ((8, 1), Class::Tree),
            ((9, 1), Class::Tree),
            ((10, 1), Class::Tower(Side::Enemy)),
            ((10, 2), Class::Tower(Side::Player)),
            ((1, 3), Class::Road),
            ((2, 3), Class::Grass),
            ((3, 3), Class::Gate),
            ((5, 5), Class::Deep),
            ((4, 4), Class::Shallow),
            ((6, 6), Class::Shallow),
            ((10, 5), Class::Shallow),
            ((11, 0), Class::Ground),
            ((0, 0), Class::Ground),
            ((12, 8), Class::Ground),
        ];
        for ((col, row), class) in expect {
            assert_eq!(mm.class_at(col, row), Some(class), "cell {col},{row}");
            let texel = mm.image().texels[row as usize * 13 + col as usize];
            assert_eq!(texel, class.color(map.theme), "cell {col},{row}'s texel");
        }
        assert_eq!(mm.class_at(13, 0), None, "past the image");
        // The field's rectangle starts half a texel in.
        assert_eq!(source((w, h)), Rectangle::new(0.5, 0.5, 12.0, 8.0));
        // Every class reads apart from the ground it lies on, in both
        // themes.
        for theme in Theme::ALL {
            let all = [
                Class::Grass, Class::Road, Class::Shallow, Class::Deep, Class::Ice, Class::Gate, Class::Brick, Class::Iron,
                Class::Wood, Class::Glass, Class::Prop, Class::Tree, Class::Tower(Side::Player), Class::Tower(Side::Enemy),
            ];
            for class in all {
                assert_ne!(class.color(theme), Class::Ground.color(theme), "{class:?} on {theme:?}");
            }
        }
    }

    fn water_cells(map: &MapFile) -> Vec<Position> {
        map.iter_cells().filter(|(_, _, o)| matches!(o, CellObject::Water)).map(|(c, r, _)| crate::map::cell_to_world(c, r)).collect()
    }

    /// A round's image is the map's floor with the tiles standing in its
    /// world; a tile that goes takes its texel back to the floor under it -
    /// road under a wall - as a patch of that one texel under a new stamp,
    /// and a round started again on the same seed comes back whole.
    #[test]
    fn a_tile_death_patches_the_image() {
        let mut game = Game::default();
        game.enemy_count_override = Some(0);
        game.seed_override = Some(5);
        game.map = sample();
        let (w, h) = game.map.field_size();
        game.init(w, h);
        let mut round = RoundMinimap::default();
        let stamp = round.sync(&game).image().stamp;
        assert_eq!(round.minimap().class_at(1, 1), Some(Class::Brick));
        let brick = game
            .world
            .query::<(hecs::Entity, &Obstacle)>()
            .iter()
            .find(|(_, o)| crate::map::world_to_cell(o.position) == (1, 1))
            .map(|(e, _)| e)
            .expect("the brick tile");
        game.world.despawn(brick).expect("despawned");
        let after = round.sync(&game).image().clone();
        assert_ne!(after.stamp, stamp, "a new stamp");
        let patch = after.changed_since(stamp).expect("a patch from the stamp the copy holds");
        assert_eq!((patch.x, patch.y, patch.width, patch.height), (1, 1, 1, 1), "only the brick's texel");
        assert_eq!(round.minimap().class_at(1, 1), Some(Class::Road), "the dirt the wall stood on");
        assert_eq!(after.texels[13 + 1], Class::Road.color(Theme::Grass));
        // Nothing changed, nothing patched.
        let held = round.minimap().image().stamp;
        assert_eq!(round.sync(&game).image().stamp, held);
        // The same seed again: a new round, the wall back.
        game.init(w, h);
        assert_eq!(round.sync(&game).class_at(1, 1), Some(Class::Brick));
    }

    /// The minimap shows only the enemies the arrows could point at: one
    /// hidden in tall grass far from the seat is left out, one in the open
    /// is in, and the hidden one shows the moment it fires.
    #[test]
    fn concealed_enemies_are_left_out() {
        let t = Tuning::DEFAULT;
        let world = Rectangle::new(0.0, 0.0, 400.0, 300.0);
        let view = ViewFrame::new(world, 1.0, Vec2::zero(), world, &t);
        let seat = SeatView { slot: 0, pos: Position::new(200.0, 150.0), wreck: false, gate: None };
        let hidden = TankView { slot: 5, seat: None, pos: Position::new(900.0, 150.0), wreck: false, gate: None, in_grass: true, lane: false };
        let open = TankView { slot: 6, pos: Position::new(-500.0, 150.0), in_grass: false, ..hidden };
        let scene = |time: f32| Scene { time, seat: Some(seat), tanks: vec![hidden, open], ..Scene::default() };
        let mut aw = Awareness::new();
        let marks = Marks::from_shown(&[aw.frame(&scene(0.0), &view, &t)]);
        assert_eq!(marks.enemies, vec![open.pos], "the one in the grass is left out");
        aw.note(0.5, crate::indicators::Note::Fired { slot: 5, at_seat: false });
        let marks = Marks::from_shown(&[aw.frame(&scene(0.5), &view, &t)]);
        assert_eq!(marks.enemies, vec![hidden.pos, open.pos], "firing gives it away");
        // A second seat on the screen adds the enemies it sees, once: this
        // one stands beside the hidden tank, on a screen that shows it.
        let mut other = Awareness::new();
        let near = Scene { seat: Some(SeatView { pos: Position::new(880.0, 150.0), ..seat }), ..scene(0.0) };
        let beside = ViewFrame::new(Rectangle::new(700.0, 0.0, 400.0, 300.0), 1.0, Vec2::zero(), world, &t);
        let mut aw = Awareness::new();
        let marks = Marks::from_shown(&[aw.frame(&scene(0.0), &view, &t), other.frame(&near, &beside, &t)]);
        assert_eq!(marks.enemies, vec![open.pos, hidden.pos], "the second seat is beside the hidden one");
        // Under the sky an enemy past its sight is gone from the minimap too.
        let dark = Scene { sight: Some(300.0), ..scene(0.0) };
        assert!(Marks::from_shown(&[Awareness::new().frame(&dark, &view, &t)]).enemies.is_empty());
    }

    /// Through a whole round: the seats, the frogs and the view.
    #[test]
    fn the_marks_read_the_round() {
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(3);
        game.players = PlayerCount::from_count(2).expect("two seats");
        game.map = MapFile::from_toml_str(include_str!("../maps/default.toml")).expect("the default map");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        let view = Rectangle::new(10.0, 20.0, 300.0, 200.0);
        let marks = Marks::gather(&game, &[0], &[], view);
        assert_eq!(marks.view, Some(view));
        assert_eq!(marks.seats.iter().map(|s| (s.seat, s.local)).collect::<Vec<_>>(), vec![(0, true), (1, false)]);
        assert!(marks.numbered);
        assert_eq!(marks.frogs.len(), 1, "the player's frog");
        // A wrecked seat drops off.
        let entity = game.seat(1).expect("seat 1");
        game.world.get::<&mut Tank>(entity).expect("a tank").damage = crate::MAX_DAMAGE;
        assert_eq!(Marks::gather(&game, &[0], &[], view).seats.len(), 1);
    }

    /// The phone rule: a phone's short side - in points, or in millimetres
    /// where the platform says - turns the minimap off unless the knob
    /// says always; a tablet and a desktop show it unless the knob says
    /// never.
    #[test]
    fn the_phone_rule_turns_it_off_on_phones() {
        let r = rules();
        let iphone = Screen::new(852.0, 393.0, 3.0, 460.0);
        let se = Screen::new(667.0, 375.0, 2.0, 326.0);
        let pixel = Screen::new(923.0, 411.0, 2.625, 422.0);
        let ipad = Screen::new(1180.0, 820.0, 2.0, 264.0);
        let ipad_mini = Screen::new(1133.0, 744.0, 2.0, 326.0);
        let desktop = Screen::new(1920.0, 1080.0, 1.0, 96.0).with_mm_per_point(0.2768);
        let laptop = Screen::new(1470.0, 956.0, 2.0, 224.0).with_mm_per_point(0.2);
        let small_window = Screen::new(852.0, 393.0, 1.0, 96.0).with_mm_per_point(0.2646);
        for phone in [iphone, se, pixel, small_window] {
            assert!(r.is_phone(&phone), "{phone:?}");
            assert!(!r.shown_on(&phone), "{phone:?}");
        }
        for big in [ipad, ipad_mini, desktop, laptop] {
            assert!(!r.is_phone(&big), "{big:?}");
            assert!(r.shown_on(&big), "{big:?}");
        }
        // A small panel with a desktop's points is a phone by its size.
        let tiny_panel = Screen::new(1080.0, 600.0, 1.0, 400.0).with_mm_per_point(0.0635);
        assert!(r.is_phone(&tiny_panel));
        // The knob: never, or everywhere.
        let mut t = Tuning::DEFAULT;
        t.minimap_show = 0;
        assert!(!MinimapRules::of(&t).shown_on(&desktop));
        t.minimap_show = 2;
        assert!(MinimapRules::of(&t).shown_on(&iphone));
        assert_eq!(rules().show, Show::NotOnPhones, "by the screen by default");
    }

    /// The size: the map's shape in the box, whole half points a cell
    /// where that keeps three quarters of the fit.
    #[test]
    fn the_size_keeps_the_maps_shape_inside_the_box() {
        let r = rules();
        let cells = |c: f32, rows: f32| (c * CELL, rows * CELL);
        assert_eq!(r.size_pt(cells(80.0, 45.0)), (160.0, 90.0), "an 80 x 45 field at two points a cell");
        assert_eq!(r.size_pt(cells(112.0, 63.0)), (160.0, 90.0), "longwater, its fit kept unsnapped");
        assert_eq!(r.size_pt(cells(96.0, 54.0)), (144.0, 81.0), "the study map at one and a half");
        assert_eq!(r.size_pt(cells(40.0, 22.5)), (160.0, 90.0));
        assert_eq!(r.size_pt(cells(48.0, 24.0)), (144.0, 72.0));
        let (w, h) = r.size_pt(cells(30.0, 120.0));
        assert!((h - 120.0).abs() < 1e-3 && (w - 30.0).abs() < 1e-3, "a tall map is as tall as the box: {w} x {h}");
        let (w, h) = r.size_pt(cells(250.0, 20.0));
        assert!(w <= 160.0 + 1e-3 && h < 20.0 && w > 120.0, "a long thin map keeps its shape: {w} x {h}");
        let (w, h) = r.size_pt(cells(250.0, 250.0));
        assert!((w - 120.0).abs() < 1e-3 && (h - 120.0).abs() < 1e-3, "the largest map fits the box: {w} x {h}");
    }

    /// A point and its world point round-trip through the minimap's
    /// rectangle, and the view's outline is clipped to it.
    #[test]
    fn points_map_to_the_field_and_back() {
        let field = (2560.0, 1440.0);
        let rect = Rectangle::new(100.0, 50.0, 160.0, 90.0);
        let p = to_point(rect, field, Position::new(1280.0, 720.0));
        assert_eq!(p, Vec2::new(180.0, 95.0));
        assert_eq!(to_world(rect, field, p), Position::new(1280.0, 720.0));
        assert_eq!(to_world(rect, field, Vec2::new(0.0, 500.0)), Position::new(0.0, 1440.0), "kept on the field");
        let view = rect_on(rect, field, Rectangle::new(-320.0, 0.0, 960.0, 540.0)).expect("on the minimap");
        assert_eq!(view, Rectangle::new(100.0, 50.0, 40.0, 33.75));
        assert_eq!(rect_on(rect, field, Rectangle::new(3000.0, 0.0, 100.0, 100.0)), None);
    }

    /// The picture: every mark inside the minimap, the view's outline
    /// white over its rim, the seats numbered in a round of two and the
    /// screen's own not, the gates only in the lit half of their blink.
    #[test]
    fn the_picture_keeps_its_marks_inside() {
        let t = Tuning::DEFAULT;
        let field = (2560.0, 1440.0);
        let rect = Rectangle::new(100.0, 50.0, 160.0, 90.0);
        let marks = Marks {
            view: Some(Rectangle::new(800.0, 400.0, 960.0, 540.0)),
            second_view: None,
            seats: vec![
                SeatMark { seat: 0, at: Position::new(1280.0, 720.0), local: true },
                SeatMark { seat: 1, at: Position::new(2560.0, 0.0), local: false },
            ],
            frogs: vec![FrogMark { side: Side::Enemy, at: Position::new(0.0, 1440.0) }],
            enemies: vec![Position::new(50.0, 50.0), Position::new(-400.0, 3000.0)],
            gates: vec![GateMark { at: Position::new(1200.0, 0.0), flash: 1.0 }],
            time: 0.0,
            numbered: true,
        };
        let pic = picture(&marks, rect, field, 10, &t);
        let inside = |f: &Fill| f.x >= 100 && f.y >= 50 && f.x + f.w <= 260 && f.y + f.h <= 140;
        assert!(pic.fills.iter().all(inside), "{:?}", pic.fills.iter().find(|f| !inside(f)));
        assert_eq!(pic.labels.len(), 1, "seat 2 is numbered, the screen's own seat is not");
        assert_eq!(pic.labels[0].text, "2");
        let l = &pic.labels[0];
        assert!(l.plate.x >= 100 && l.plate.x + l.plate.w <= 260 && l.plate.y >= 50 && l.plate.y + l.plate.h <= 140, "{l:?}");
        let has = |c: Color| pic.fills.iter().any(|f| f.color.r == c.r && f.color.g == c.g && f.color.b == c.b);
        assert!(has(crate::indicators::GATE_AMBER), "a gate lit at time 0");
        assert!(has(crate::indicators::HOSTILE) && has(crate::indicators::FROG_GREEN) && has(crate::tank::team_color(1)));
        let dark = picture(&Marks { time: 0.5 / t.indicator_gate_blink_hz + 0.01, ..marks.clone() }, rect, field, 10, &t);
        let amber = |p: &Picture| p.fills.iter().filter(|f| (f.color.r, f.color.g, f.color.b) == (0xEE, 0xA3, 0x43)).count();
        assert_eq!(amber(&dark), 0, "and dark in the other half");
        // A split screen outlines both halves' views, inside like the rest.
        let one = picture(&marks, rect, field, 10, &t);
        let both = picture(&Marks { second_view: Some(Rectangle::new(1700.0, 100.0, 960.0, 540.0)), ..marks.clone() }, rect, field, 10, &t);
        let white = |p: &Picture| p.fills.iter().filter(|f| (f.color.r, f.color.g, f.color.b) == (255, 255, 255)).count();
        assert!(white(&both) > white(&one), "{} vs {}", white(&both), white(&one));
        assert!(both.fills.iter().all(inside));
        // One seat alone carries no number.
        let solo = Marks { numbered: false, ..marks };
        assert!(picture(&solo, rect, field, 10, &t).labels.is_empty());
    }

    /// The builder's image follows a stroke cell by cell: a wall painted
    /// on a cell moves that texel alone, water painted round a lone cell
    /// of water turns the block's middle into open water - a cell the
    /// stroke never touched, which the ground's repaint names - and the
    /// patch reaches only the texels that changed.
    #[test]
    fn a_repaint_patches_only_the_cells_it_changed() {
        let mut map = sample();
        let (w, h) = map.field_size();
        let mut ground = crate::ground::build(w, h, 1, &[], &water_cells(&map), &[], crate::ground::Look::default());
        let mut mm = Minimap::of_map(&map, |c, r| ground.depth(c, r));
        let before = mm.image().clone();
        map.set_cell(8, 6, CellObject::Wall { material: Material::Iron });
        let floor = crate::ground::CellFloor { road: true, water: false, wall: true };
        let touched = ground.repaint(&[(8, 6, floor)]);
        assert!(touched.contains(&(8, 6)));
        let classify = |map: &MapFile, ground: &GroundGrid, c: i32, r: i32| {
            let obj = map.cell(c, r);
            (Class::floor(obj, ground.depth(c, r)), obj.and_then(Class::solid_of))
        };
        assert!(mm.repaint(touched.iter().copied().chain([(8, 6)]), |c, r| classify(&map, &ground, c, r)));
        assert_eq!(mm.class_at(8, 6), Some(Class::Iron));
        let patch = mm.image().changed_since(before.stamp).expect("a patch");
        assert_eq!((patch.x, patch.y, patch.width, patch.height), (8, 6, 1, 1));
        let diff: Vec<usize> = (0..before.texels.len()).filter(|&i| before.texels[i] != mm.image().texels[i]).collect();
        assert_eq!(diff, vec![6 * 13 + 8], "one texel moved");
        // A repaint that changes nothing leaves the stamp.
        let stamp = mm.image().stamp;
        assert!(!mm.repaint([(8, 6), (0, 0)], |c, r| classify(&map, &ground, c, r)));
        assert_eq!(mm.image().stamp, stamp);

        // A lone cell of water at (9, 4) is a ford; a second stroke round it
        // makes a 3 x 3 lake whose middle - a cell it never touched - is
        // open water.
        let water = crate::ground::CellFloor { road: false, water: true, wall: false };
        let paint = |map: &mut MapFile, ground: &mut GroundGrid, mm: &mut Minimap, cells: &[(i32, i32)]| {
            for &(c, r) in cells {
                map.set_cell(c, r, CellObject::Water);
            }
            let floors: Vec<_> = cells.iter().map(|&(c, r)| (c, r, water)).collect();
            let touched = ground.repaint(&floors);
            mm.repaint(touched.iter().copied().chain(cells.iter().copied()), |c, r| classify(map, ground, c, r));
            touched
        };
        paint(&mut map, &mut ground, &mut mm, &[(9, 4)]);
        assert_eq!(mm.class_at(9, 4), Some(Class::Shallow), "a lone cell of water");
        let stamp = mm.image().stamp;
        let ring = [(8, 3), (9, 3), (10, 3), (8, 4), (10, 4), (8, 5), (9, 5), (10, 5)];
        let touched = paint(&mut map, &mut ground, &mut mm, &ring);
        assert!(touched.contains(&(9, 4)), "the middle's depth can have moved: {touched:?}");
        assert_eq!(mm.class_at(9, 4), Some(Class::Deep), "the middle is open water");
        assert!(ring.iter().all(|&(c, r)| mm.class_at(c, r) == Some(Class::Shallow)), "its rim a ford");
        let patch = mm.image().changed_since(stamp).expect("a patch");
        assert_eq!((patch.x, patch.y, patch.width, patch.height), (8, 3, 3, 3), "the block alone");
        // The whole image agrees with one made afresh from the edited map.
        let fresh = Minimap::of_map(&map, |c, r| ground.depth(c, r));
        assert_eq!(fresh.image().texels, mm.image().texels);
    }
}
