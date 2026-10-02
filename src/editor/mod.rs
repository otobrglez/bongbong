//! The battlefield map builder (docs/game-editor-fusion.md): the Build
//! mode of the game. Click or tap a grid cell to place/erase a
//! wall/prop/road/water/grass/gate/actor/pickup cell of the map the round was
//! built from, change the map's level keys, then `PLAY` a fresh round on
//! it. Presentation-layer only, same category as `game.rs` - never
//! touches physics/AI/hecs. `mode::Session` owns one next to the `Game`
//! and `main.rs` drives whichever mode is live.
//!
//! **Input arrives as a plain `BuilderInput`**, gathered once per frame by
//! `app.rs` exactly like `simulation::Input` is for `Game::update`: a
//! `RaylibHandle` is only ever needed to draw. That is what lets the dev
//! server's `click`/`key`/`builder_*` tools feed the same struct a mouse
//! or a finger produces, and the tests below run headlessly.
//!
//! The edit model (strokes with the toggle-erase rule, the MAP settings,
//! load/reset, the undo stack in `history.rs`) and every hit test are this
//! file, built into every build; the bar, the popups, the status line, the
//! navigator and the loupe are the chrome on top, laid out on the window in
//! UI points by `chrome.rs` - the one geometry table the hit tests here,
//! the painter (`render.rs`, behind the `render` feature) and the dev
//! server read - and drawn by `render.rs`. The canvas is the bitmap under
//! the bar (`chrome::BuilderFrame`).
//!
//! **The canvas has its own camera** (`camera.rs`,
//! docs/large-maps-follow-camera.md §9): FIT shows the whole map - an
//! arena at the scale its round is drawn at, letterboxed under the bar -
//! and the wheel at the cursor, a middle or Space drag, `+`/`-`, the
//! arrows and the bar's FIT button zoom and pan it. Every pointer goes
//! through that camera before a cell is hit-tested (`cell_at`), and the
//! canvas is drawn through the same one, so a press paints the cell drawn
//! under it at every zoom.
//!
//! **A touch screen is read finger by finger** (`gesture.rs`): one finger
//! paints once past the touch slop, two pan and pinch-zoom, a two-finger
//! tap undoes and a three-finger tap redoes. Where a cell is under
//! `builder_paint_min_cell_mm` on the glass - a finger cannot hit one - a
//! tap zooms in instead of painting and a drag pans (the paint threshold).
//!
//! **The brush's shape** (docs/large-maps-patterns.md, "Area brushes:
//! fill, scatter, auto-tile"; the cells are `brush.rs`), BRUSH's list:
//! PEN paints a stroke under the toggle-erase rule; RECT fills the
//! rectangle a drag draws when it ends, the eraser or the secondary button
//! clearing it instead; FILL floods the region of the pressed cell - the
//! cells joined to it edge to edge that hold exactly what it holds - under
//! the toggle-erase rule, and refuses a region past
//! `builder_fill_max_cells`; SCATTER lays the brush's object on a hashed
//! share of the empty cells round the drag, never a random draw, and thins
//! it erasing. A singleton's brush paints with the pen whatever the shape.
//! Each stroke of any shape is one `EditStep`, and the water and road it
//! lays auto-tile through the ground's repaint, which makes the whole
//! ground again for an edit past `REBUILD_SHARE` of the map.
//!
//! **The Load list shows each map's picture** (`thumbs.rs`): its minimap,
//! made a page at a time as the list shows it, a picture a frame, and kept
//! by map name and the revision of its text.
//!
//! **The select tool** (docs/large-maps-patterns.md, "Selection, copy and
//! stamps"; the data is `select.rs`): a drag draws a rectangle of cells,
//! outlined until Escape, a press outside it or another tool clears it. A
//! drag from inside it lifts its cells and carries them, landing on the
//! release; COPY, CUT, PASTE, the flips, DELETE and keeping it as a stamp
//! are the strip's buttons under the bar (`chrome::Strip`) and Ctrl+C,
//! Ctrl+X, Ctrl+V and Delete; a paste or a stamp is a ghost that follows
//! the mouse - or stands where a finger drags or taps it - until a press
//! puts it down. Every move, paste, flip, cut and delete is one `EditStep`.
//! What a selection carries is what the brushes would do with it: a start,
//! player 2's start or either frog moves with a moved selection, and a
//! paste places one only where the map holds none of it - so a paste never
//! leaves two, never reaches a cell outside its own rectangle, and a CUT
//! then PASTE carries the start like a move. Gates and portals are copied
//! and moved like any cell: a gate that lands off the map's edge is drawn
//! outlined and named by CHECK, whose FIX puts it back on the edge, and a
//! pasted portal joins the network.

pub mod brush;
pub mod camera;
pub mod chrome;
pub mod gesture;
pub mod history;
pub mod index;
#[cfg(feature = "render")]
pub mod render;
pub mod select;
pub mod thumbs;
#[cfg(feature = "render")]
pub use render::{EditorTextures, ThumbnailTextures};

pub use brush::BrushRules;
pub use camera::{BuilderCamera, CanvasRules, CanvasScreen, Viewport};
pub use chrome::{Bar, BarButton, BarTools, BuilderFrame, Chrome, PopupLayout, Strip, StripButton};
pub use select::{Axis, CellRect, Clip, Ghost, Stamp, StampSource};

use rand::RngExt;
use std::collections::BTreeMap;
use crate::math::{Color, Rectangle, Vec2};

use crate::ground::{self, GroundGrid};
use crate::hud::Corners;
use crate::maplint::{LintCell, LintFinding, LintFix, LintSetup, LintSeverity};
use crate::minimap::{Class, Minimap, MinimapRules};
use crate::level::{Mission, SpawnKind, Tier};
use crate::map::{self, CellObject, MapEntry, MapFile, Theme, Weather};
use crate::obstacle::{Drum, Material};
use crate::pickup::PickupKind;
use crate::frog::Side;
use crate::tower::TowerKind;
use crate::tank::TankKind;
use crate::{EDITOR_STEPPER_SIZE, Layout, PATHFIND_CELL_SIZE, Position};
use chrome::{LintLayout, LoadLayout, Palette, SettingsLayout, LINT_HEAD_ROWS};
pub use history::{CellChange, EditStep, MapDiff, MapSettings, UndoStack};
pub use index::CellIndex;

/// The builder's accent: the amber the `BUILD` label, the active
/// category's outline and the mode button share (`hud::BUILD_COLOR`).
pub const BUILD_ACCENT: Color = crate::hud::BUILD_COLOR;

/// A finding row's FIX button, at the row's right end.
const LINT_FIX_W: f32 = 80.0;
/// A finding row's mark (a 12 pt square) and words, inset from the row's
/// left; the words run from 8 pt past the mark to 8 pt short of the FIX
/// button - what `text_tests` holds every language's to, in 16 pt.
#[cfg_attr(not(feature = "render"), allow(dead_code))]
const LINT_TEXT_INSET: f32 = 12.0;
#[cfg_attr(not(feature = "render"), allow(dead_code))]
const LINT_MARK: f32 = 12.0;
#[cfg_attr(not(feature = "render"), allow(dead_code))]
pub(crate) const LINT_FINDING_W: f32 = chrome::LINT_PANEL_W - LINT_TEXT_INSET - LINT_MARK - 8.0 - SETTINGS_INSET - LINT_FIX_W - 8.0;
/// The width the CHECK panel's hint line has, and the clear check's words:
/// the panel less its insets, and from beside the flag to the right inset
/// - what `text_tests` holds every language's to.
#[cfg_attr(not(feature = "render"), allow(dead_code))]
pub(crate) const LINT_HINT_W: f32 = chrome::LINT_PANEL_W - 2.0 * LINT_TEXT_INSET;
#[cfg_attr(not(feature = "render"), allow(dead_code))]
pub(crate) const LINT_CLEAR_W: f32 = chrome::LINT_PANEL_W - 2.0 * LINT_TEXT_INSET - LINT_MARK - 8.0;
/// The gap a jump keeps between what it frames and the open CHECK panel.
const LINT_FREE_GAP: f32 = 16.0;
/// The loupe's side, in UI points - about 23 mm on a phone, twice and more
/// what a fingertip covers - before it is cut down to whole blocks of the
/// world at the loupe's scale.
const LOUPE_PT: f32 = 144.0;
/// The settings panel's row layout: label at the left inset, the `<`
/// button, the value, the `>` button at the right inset.
const SETTINGS_INSET: f32 = 4.0;
const SETTINGS_DEC_X: f32 = 124.0;
/// One frame of raw builder input, in the **window's** own coordinates -
/// what the mouse and the touch screen report. `app.rs` fills it from the
/// mouse, the touch screen and the keyboard; the dev server fills it from
/// `click`/`key`/`builder_touch` requests. `update` takes each point to the
/// canvas's bitmap or to the chrome's UI points through the frame
/// (`BuilderFrame`). Nothing in this module reads raylib input directly.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuilderInput {
    /// Where the pointer is this frame: the mouse, or the finger while it
    /// touches. `None` on a touch screen with nothing touching.
    pub pointer: Option<Vec2>,
    /// The primary button (left mouse, a finger) went down this frame.
    pub pressed: bool,
    /// The primary button is down this frame (including the press frame).
    pub held: bool,
    /// The secondary button (right mouse) went down / is down.
    pub right_pressed: bool,
    pub right_held: bool,
    /// The middle mouse button is down: a drag with it pans the canvas.
    pub middle_held: bool,
    /// Space is held: a drag with the primary button pans the canvas
    /// instead of painting.
    pub space_held: bool,
    /// Mouse wheel movement, positive away from the user: over a category
    /// button it steps the tool, over the canvas it zooms at the cursor.
    pub wheel: f32,
    /// `+` / `-` went down this frame: zoom about the canvas's middle.
    pub zoom_in: bool,
    pub zoom_out: bool,
    /// The arrow keys held this frame, -1, 0 or 1 on each axis (right and
    /// down positive): the view moves that way.
    pub pan_keys: Vec2,
    pub escape: bool,
    pub enter: bool,
    pub backspace: bool,
    /// Ctrl+Z / Ctrl+Y (Cmd on macOS).
    pub undo: bool,
    pub redo: bool,
    /// Ctrl+C / Ctrl+X / Ctrl+V: the selection copied or cut, the clipboard
    /// pasted as a ghost.
    pub copy: bool,
    pub cut: bool,
    pub paste: bool,
    /// Delete: the selection's cells cleared (Backspace too, outside the
    /// Save prompt).
    pub delete: bool,
    /// Characters typed this frame, for the dev Save prompt.
    pub typed: String,
    /// Every touch point down this frame, in the window's coordinates - the
    /// raw fingers (`gesture.rs`), not raylib's gestures. Empty with a
    /// mouse.
    pub touches: Vec<crate::touch::TouchPoint>,
    /// Seconds since the last frame: what a held key pans by and a tap is
    /// timed with.
    pub dt: f32,
    /// The screen the canvas is drawn on, when the window knows it
    /// (`app.rs`); `None` keeps the one the builder saw last.
    pub screen: Option<CanvasScreen>,
}

/// One brush - what a press on the field does. Grouped into `Category`s
/// for the bar; `name`/`parse` are the spelling the dev server uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Wall(Material),
    /// A destructible standalone solid: one of the three props
    /// (`Material::Sandbag`/`Barrel`/`Fence`) or one of the two tree
    /// species (`Material::Tree`/`Pine`). Any number, variant rolled per
    /// tile when the round spawns.
    Prop(Material),
    /// A barrel of a pinned kind (`obstacle::Drum`): the red oil drum
    /// that leaves a burning pool, or the grey fuel drum that goes off
    /// harder and launches when chained. `Prop(Material::Barrel)` stays
    /// the roll-at-spawn barrel.
    Drum(Drum),
    /// An oil trail cell: not solid, a fuse the author draws on the
    /// ground - a blast or a burning neighbour lights it and the fire runs
    /// along it (docs/barrel-explosion-variety.md section D).
    OilTrail,
    Road,
    /// Water: ground like road, shaped by the cells around it into a
    /// river (a line one cell wide) or a lake (a block) - `ground::build`.
    Water,
    Frog,
    /// Player 1's start - singleton, moved on placement like `Frog`.
    Start,
    /// Player 2's start (two-player rounds, docs/two-players.md) -
    /// singleton like `Start`, independent of it.
    Start2,
    /// The Hunt mission's enemy frog - singleton, moved on placement like
    /// `Frog`.
    EnemyFrog,
    /// A wave roll-in gate - any number, meant for nav-grid edge cells (the
    /// linter's `gate-not-on-edge` catches one placed elsewhere).
    Gate,
    Pickup(PickupKind),
    /// Tall grass: cover, not terrain. Any number of cells; not solid, so
    /// it never blocks movement or pathfinding (see `grass.rs`).
    TallGrass,
    /// A teleport portal anchor - any number, not solid; the network only
    /// works with two or more (the linter's `portal-alone` names a lone
    /// one, and the canvas ghosts it).
    Portal,
    /// A defence tower fighting for a side (docs/defence-towers-prd.md).
    /// One tool per kind and side rather than a side switch: the cursor's
    /// readout names a cell by the tool that paints exactly its object.
    Tower(TowerKind, Side),
    Eraser,
    /// The rectangle select tool (the module docs): a press selects,
    /// carries or pastes rather than paints. BRUSH's list and the palette's
    /// brush row pick it; any other tool clears the selection.
    Select,
}

/// Every tool, in bar order: the categories one after another, then the
/// eraser and the select tool, which no category holds. The trees sit with
/// the ground's vegetation, which leaves PROP room for the six tower tools
/// inside the eleven rows a dropdown fits.
pub const TOOLS: [Tool; 40] = [
    Tool::Wall(Material::Brick),
    Tool::Wall(Material::Iron),
    Tool::Wall(Material::Wood),
    Tool::Wall(Material::Glass),
    Tool::Prop(Material::Sandbag),
    Tool::Prop(Material::Barrel),
    Tool::Drum(Drum::Oil),
    Tool::Drum(Drum::Fuel),
    Tool::Prop(Material::Fence),
    Tool::Tower(TowerKind::Tesla, Side::Player),
    Tool::Tower(TowerKind::Tesla, Side::Enemy),
    Tool::Tower(TowerKind::Gun, Side::Player),
    Tool::Tower(TowerKind::Gun, Side::Enemy),
    Tool::Tower(TowerKind::Bio, Side::Player),
    Tool::Tower(TowerKind::Bio, Side::Enemy),
    Tool::Road,
    Tool::Water,
    Tool::TallGrass,
    Tool::Prop(Material::Tree),
    Tool::Prop(Material::Pine),
    Tool::OilTrail,
    Tool::Gate,
    Tool::Portal,
    Tool::Start,
    Tool::Start2,
    Tool::Frog,
    Tool::EnemyFrog,
    Tool::Pickup(PickupKind::Health),
    Tool::Pickup(PickupKind::Ammo),
    Tool::Pickup(PickupKind::Laser),
    Tool::Pickup(PickupKind::Minigun),
    Tool::Pickup(PickupKind::Plasma),
    Tool::Pickup(PickupKind::Missiles),
    Tool::Pickup(PickupKind::SpeedUp),
    Tool::Pickup(PickupKind::Shield),
    Tool::Pickup(PickupKind::Flamethrower),
    Tool::Pickup(PickupKind::FrogHealth),
    Tool::Pickup(PickupKind::TowerPack),
    Tool::Eraser,
    Tool::Select,
];

impl Tool {
    /// The tool's name, as the dev server and the bar spell it.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Wall(Material::Brick) => "brick",
            Tool::Wall(Material::Iron) => "iron",
            Tool::Wall(Material::Wood) => "wood",
            Tool::Wall(Material::Glass) => "glass",
            Tool::Wall(_) => "wall",
            Tool::Prop(Material::Sandbag) => "sandbag",
            Tool::Prop(Material::Barrel) => "barrel",
            Tool::Prop(Material::Fence) => "fence",
            Tool::Prop(Material::Tree) => "tree",
            Tool::Prop(Material::Pine) => "pine",
            Tool::Prop(_) => "prop",
            Tool::Drum(Drum::Oil) => "oil_drum",
            Tool::Drum(Drum::Fuel) => "fuel_drum",
            Tool::OilTrail => "oil_trail",
            Tool::Road => "road",
            Tool::Water => "water",
            Tool::TallGrass => "tall_grass",
            Tool::Gate => "gate",
            Tool::Portal => "portal",
            Tool::Start => "start",
            Tool::Start2 => "start2",
            Tool::Frog => "frog",
            Tool::EnemyFrog => "enemy_frog",
            Tool::Pickup(PickupKind::Health) => "health",
            Tool::Pickup(PickupKind::Ammo) => "ammo",
            Tool::Pickup(PickupKind::Laser) => "laser",
            Tool::Pickup(PickupKind::Minigun) => "minigun",
            Tool::Pickup(PickupKind::Plasma) => "plasma",
            Tool::Pickup(PickupKind::Missiles) => "missiles",
            Tool::Pickup(PickupKind::SpeedUp) => "speedup",
            Tool::Pickup(PickupKind::Shield) => "shield",
            Tool::Pickup(PickupKind::Flamethrower) => "flamethrower",
            Tool::Pickup(PickupKind::FrogHealth) => "frog_health",
            Tool::Pickup(PickupKind::TowerPack) => "tower_pack",
            Tool::Tower(TowerKind::Tesla, Side::Player) => "tesla",
            Tool::Tower(TowerKind::Tesla, Side::Enemy) => "tesla_enemy",
            Tool::Tower(TowerKind::Gun, Side::Player) => "gun_tower",
            Tool::Tower(TowerKind::Gun, Side::Enemy) => "gun_tower_enemy",
            Tool::Tower(TowerKind::Bio, Side::Player) => "bio_slush",
            Tool::Tower(TowerKind::Bio, Side::Enemy) => "bio_slush_enemy",
            Tool::Eraser => "eraser",
            Tool::Select => "select",
        }
    }

    pub fn parse(name: &str) -> Option<Tool> {
        TOOLS.iter().copied().find(|t| t.name() == name)
    }

    /// The category this tool sits in (`None` for the eraser and the
    /// select tool).
    pub fn category(self) -> Option<Category> {
        match self {
            Tool::Wall(_) => Some(Category::Wall),
            Tool::Prop(Material::Tree | Material::Pine) => Some(Category::Ground),
            Tool::Prop(_) | Tool::Drum(_) | Tool::Tower(..) => Some(Category::Prop),
            Tool::Road | Tool::Water | Tool::TallGrass | Tool::OilTrail | Tool::Gate | Tool::Portal => Some(Category::Ground),
            Tool::Start | Tool::Start2 | Tool::Frog | Tool::EnemyFrog => Some(Category::Actor),
            Tool::Pickup(_) => Some(Category::Pickup),
            Tool::Eraser | Tool::Select => None,
        }
    }

    /// The cell object this brush paints; `None` for the eraser and the
    /// select tool.
    pub fn object(self) -> Option<CellObject> {
        match self {
            Tool::Wall(material) => Some(CellObject::Wall { material }),
            Tool::Prop(material) => CellObject::prop(material),
            Tool::Drum(drum) => Some(CellObject::Barrel { drum: Some(drum) }),
            Tool::OilTrail => Some(CellObject::Oil),
            Tool::Road => Some(CellObject::Road),
            Tool::Water => Some(CellObject::Water),
            Tool::Frog => Some(CellObject::Frog),
            Tool::Start => Some(CellObject::Start),
            Tool::Start2 => Some(CellObject::Start2),
            Tool::EnemyFrog => Some(CellObject::EnemyFrog),
            Tool::Gate => Some(CellObject::Gate),
            Tool::Portal => Some(CellObject::Portal),
            Tool::Pickup(pickup) => Some(CellObject::Pickup { pickup }),
            Tool::TallGrass => Some(CellObject::TallGrass),
            Tool::Tower(kind, side) => Some(CellObject::for_tower(kind, side)),
            Tool::Eraser | Tool::Select => None,
        }
    }

    /// A brush that keeps at most one of its object on the map and moves
    /// it on placement.
    pub fn is_singleton(self) -> bool {
        matches!(self, Tool::Frog | Tool::Start | Tool::Start2 | Tool::EnemyFrog)
    }
}

/// The five groups the bar shows, each remembering its current tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Wall,
    Prop,
    Ground,
    Actor,
    Pickup,
}

impl Category {
    pub const ALL: [Category; 5] = [Category::Wall, Category::Prop, Category::Ground, Category::Actor, Category::Pickup];

    /// The group's name in the language on screen.
    pub fn label(self) -> String {
        crate::text::text().get(self.label_key())
    }

    /// The message that names the group.
    pub fn label_key(self) -> crate::text::Key {
        match self {
            Category::Wall => crate::text::keys::CATEGORY_WALL,
            Category::Prop => crate::text::keys::CATEGORY_PROP,
            Category::Ground => crate::text::keys::CATEGORY_GROUND,
            Category::Actor => crate::text::keys::CATEGORY_ACTOR,
            Category::Pickup => crate::text::keys::CATEGORY_PICKUP,
        }
    }

    /// The group as the dev server spells it: its open list's name
    /// (`MapEditor::open_menu`) and its bar buttons' (`category_wall`,
    /// `list_wall`).
    pub fn name(self) -> &'static str {
        match self {
            Category::Wall => "wall",
            Category::Prop => "prop",
            Category::Ground => "ground",
            Category::Actor => "actor",
            Category::Pickup => "pickup",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    /// The category's tools, in list order.
    pub fn tools(self) -> impl Iterator<Item = Tool> {
        TOOLS.iter().copied().filter(move |t| t.category() == Some(self))
    }
}

/// How the brush lays its object down - BRUSH's list and the palette's
/// brush row (`brush.rs` works out the cells).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    /// A stroke: every cell the press and its drag cross, under the
    /// toggle-erase rule.
    #[default]
    Pen,
    /// A rectangle from the press's cell to the pointer's, filled on the
    /// release - or cleared by the eraser or the secondary button.
    Rect,
    /// A flood from the pressed cell over the cells joined to it edge to
    /// edge that hold exactly what it holds, up to
    /// `builder_fill_max_cells`, under the toggle-erase rule.
    Fill,
    /// A hashed share of the cells round each cell the stroke crosses,
    /// laid on empty cells only - or, erasing, thinning the brush's object
    /// (anything, under the eraser).
    Scatter,
}

impl Shape {
    /// As the dev server spells it.
    pub fn name(self) -> &'static str {
        match self {
            Shape::Pen => "pen",
            Shape::Rect => "rect",
            Shape::Fill => "fill",
            Shape::Scatter => "scatter",
        }
    }

    /// The message that names it in BRUSH's list.
    pub fn label_key(self) -> crate::text::Key {
        match self {
            Shape::Pen => crate::text::keys::BRUSH_PEN,
            Shape::Rect => crate::text::keys::BRUSH_RECT,
            Shape::Fill => crate::text::keys::BRUSH_FILL,
            Shape::Scatter => crate::text::keys::BRUSH_SCATTER,
        }
    }

    /// Whether a press on a cell already holding exactly the brush's
    /// object makes the stroke an erase (docs/game-editor-fusion.md
    /// section 8): a pen's or a fill's - the one way to take a cell or a
    /// region back without another tool -, never a rectangle's or a
    /// scatter's, whose first cell says nothing about the rest.
    pub fn toggles(self) -> bool {
        matches!(self, Shape::Pen | Shape::Fill)
    }

    pub fn parse(s: &str) -> Option<Shape> {
        BrushRow::ALL.iter().find_map(|row| match row {
            BrushRow::Shape(shape) if shape.name() == s => Some(*shape),
            _ => None,
        })
    }
}

/// A row of BRUSH's list and a cell of the palette's brush row: a shape
/// to paint with, the select tool, or the STAMPS list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrushRow {
    Shape(Shape),
    Select,
    Stamps,
}

impl BrushRow {
    /// In list order.
    pub const ALL: [BrushRow; 6] = [
        BrushRow::Shape(Shape::Pen),
        BrushRow::Shape(Shape::Rect),
        BrushRow::Shape(Shape::Fill),
        BrushRow::Shape(Shape::Scatter),
        BrushRow::Select,
        BrushRow::Stamps,
    ];

    /// As `status.builder.buttons` names it: `shape_<name>`, the select
    /// tool's `tool_select`, `brush_stamps`.
    pub fn name(self) -> String {
        match self {
            BrushRow::Shape(shape) => format!("shape_{}", shape.name()),
            BrushRow::Select => format!("tool_{}", Tool::Select.name()),
            BrushRow::Stamps => "brush_stamps".to_string(),
        }
    }
}

/// The one popup the builder can have open at a time: opening any of
/// them closes the others (docs/game-editor-fusion.md section 7).
enum Popup {
    /// A category's tool list, below its bar button.
    Dropdown(Category),
    /// BRUSH's list, below its button.
    Brush,
    /// The palette of every category's tools and the brush's row, below
    /// the TOOLS button a narrow bar folds the five category buttons and
    /// BRUSH into.
    Palette,
    /// The STAMPS list under the bar: the shipped stamps then the ones
    /// saved this session, `scroll` rows in.
    Stamps { scroll: usize },
    /// The MAP settings panel, below the MAP button, `page` pages in where
    /// the room under the bar pages it.
    Settings { page: usize },
    /// The FILE menu, below the FILE button.
    File,
    /// The Load list under the bar: every map `map::available_maps`
    /// offers, `scroll` rows in.
    Load { entries: Vec<MapEntry>, scroll: usize },
    /// The Save-as prompt (native only).
    Save { name: String },
    /// The CHECK panel, below the CHECK button: the linter's findings
    /// over the canvas (`MapEditor::lint`), `page` pages in.
    Lint { page: usize },
}

/// One run of the linter over the canvas, for the CHECK panel.
#[derive(Clone, Debug)]
pub struct LintReport {
    /// Errors first, then warnings, then notes, each in check order
    /// (`maplint::lint`).
    pub findings: Vec<LintFinding>,
    /// `MapEditor::edits` when it ran: a report older than the map is
    /// stale.
    edits: u64,
}

impl LintReport {
    /// How many findings of `severity` there are.
    pub fn count(&self, severity: LintSeverity) -> usize {
        self.findings.iter().filter(|f| f.severity == severity).count()
    }
}

/// The loupe (docs/large-maps-patterns.md, "Touch editing without clashes,
/// and a loupe"): a magnified view of the cells under a painting finger,
/// standing clear of it, which the finger itself hides on the canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loupe {
    /// Where it stands, in UI points like the rest of the chrome: a square
    /// above the finger (`loupe_rect`), its corner on a whole device pixel.
    pub rect: Rectangle,
    /// The world it shows: whole 2 px blocks a side, centred on the cell
    /// the stroke paints, its corner on the block grid.
    pub world: Rectangle,
    /// Device pixels per world pixel inside it: the canvas's times
    /// `builder_loupe_zoom`, on the nearest whole-block scale, so every
    /// block is whole device pixels.
    pub device_scale: f32,
    /// The cell under the finger's drag - the one a stroke paints, the
    /// corner a selection stretches to - outlined in it.
    pub cell: (i32, i32),
    /// Whether the stroke erases (the outline's colour).
    pub erase: bool,
}

/// Where a loupe of `side` stands for a finger at `finger` in `area`, all
/// in one unit (the chrome's UI points): above the finger, `lift` clear of
/// it; wholly left of it where the finger is too near the area's right
/// edge for it to stand centred above (the hand is below and to the
/// right); beside it, level with it, where there is no room above; and
/// kept inside the area.
pub fn loupe_rect(finger: Vec2, side: f32, lift: f32, area: Rectangle) -> Rectangle {
    let right = area.x + area.width;
    let left_of = finger.x - lift - side;
    let mut x = if finger.x + side / 2.0 > right { left_of } else { finger.x - side / 2.0 };
    let mut y = finger.y - lift - side;
    if y < area.y {
        y = finger.y - side / 2.0;
        x = if left_of >= area.x { left_of } else { finger.x + lift };
    }
    let x = x.clamp(area.x, (right - side).max(area.x));
    let y = y.clamp(area.y, (area.y + area.height - side).max(area.y));
    Rectangle::new(x, y, side, side)
}

/// The FILE menu's rows. `SAVE` and `SAVE AS` exist only where a file can
/// be written (`map::saving_available`); the web build's menu is `LOAD`
/// and `CLEAR MAP`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileRow {
    Load,
    Save,
    SaveAs,
    /// Empty the canvas of every placed object (`MapEditor::clear`).
    Clear,
}

impl FileRow {
    fn all() -> &'static [FileRow] {
        if map::saving_available() {
            &[FileRow::Load, FileRow::Save, FileRow::SaveAs, FileRow::Clear]
        } else {
            &[FileRow::Load, FileRow::Clear]
        }
    }

    /// The row as `status.builder.buttons` names it.
    fn name(self) -> &'static str {
        match self {
            FileRow::Load => "load",
            FileRow::Save => "save",
            FileRow::SaveAs => "save_as",
            FileRow::Clear => "clear_map",
        }
    }
}

/// What the caller (`mode::Session`) should do after this frame's
/// `update` - everything else the builder handles internally.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorAction {
    None,
    /// PLAY was pressed: start a fresh round on the builder's map.
    Play,
    /// PLAY HERE was pressed: the same, seat 1 starting at the middle of
    /// the builder's view (`mode::Session::play_here`).
    PlayHere,
}

/// The cells from `from` (excluded) to `to` (included), each sharing an
/// edge with the one before: the cells a straight segment between the two
/// centres crosses, taking the axis whose next crossing comes first and
/// the vertical step on a tie.
fn edge_joined_line(from: (i32, i32), to: (i32, i32)) -> Vec<(i32, i32)> {
    let (dx, dy) = ((to.0 - from.0).abs(), (to.1 - from.1).abs());
    let (sx, sy) = ((to.0 - from.0).signum(), (to.1 - from.1).signum());
    let (mut x, mut y) = from;
    let (mut ix, mut iy) = (0, 0);
    let mut cells = Vec::with_capacity((dx + dy) as usize);
    while ix < dx || iy < dy {
        // The next vertical crossing is at (1 + 2 ix) / 2 dx of the way,
        // the next horizontal one at (1 + 2 iy) / 2 dy.
        if (1 + 2 * ix) * dy < (1 + 2 * iy) * dx {
            x += sx;
            ix += 1;
        } else {
            y += sy;
            iy += 1;
        }
        cells.push((x, y));
    }
    cells
}

/// What a map cell lays on the canvas's floor: a wall stands on road and
/// gathers the walls' shade, as in a round (`Game::init` paints road under
/// every wall), a road cell is road and a water cell water; anything else
/// leaves the floor as it is.
fn floor_of(obj: Option<&CellObject>) -> ground::CellFloor {
    let wall = matches!(obj, Some(CellObject::Wall { .. }));
    ground::CellFloor { road: wall || matches!(obj, Some(CellObject::Road)), water: matches!(obj, Some(CellObject::Water)), wall }
}

/// `repaint_ground` makes the whole ground again once an edit moves the
/// floor under more than one cell in this many of the map's. A cell of
/// water or wall repainted costs about what one and a half of the map's
/// cells made again do in a release build and three in a debug one
/// (`a_large_fill_timing`), so the two ways meet at two thirds of the map
/// and at a third: half is between.
const REBUILD_SHARE: usize = 2;

/// A press-drag-release in progress on the field.
struct Stroke {
    /// The brush's shape at the press, held for the whole drag.
    shape: Shape,
    /// Decided on the first cell (docs/game-editor-fusion.md section 8)
    /// and held for the whole drag.
    erase: bool,
    /// The cell the press landed on: a rectangle's first corner, a fill's
    /// start.
    from: (i32, i32),
    /// The cell painted last, so a held-but-not-moved frame does not
    /// re-place it every frame - a rectangle's other corner.
    last_cell: (i32, i32),
    /// A scatter's stroke counter (`brush::scattered`).
    seed: u32,
    changes: Vec<CellChange>,
}

/// A press-drag-release of the select tool in progress on the field.
#[derive(Clone, Debug, PartialEq)]
enum Drag {
    /// A rectangle being drawn from the press's cell to the pointer's;
    /// `moved` once the pointer has left the first cell (a mouse) or the
    /// finger has passed the slop - a press and release in one cell is a
    /// click, which selects nothing.
    Select { from: (i32, i32), to: (i32, i32), moved: bool },
    /// The selection's cells lifted off the map and carried: `grab` the
    /// cell the press landed on, `by` how far they have come, `from` where
    /// they were, `lifted` what they are and `changes` the lift - the
    /// first half of the one undo step the release makes.
    Move { grab: (i32, i32), by: (i32, i32), from: CellRect, lifted: Clip, changes: Vec<CellChange> },
    /// A finger carrying the paste ghost: the cell it landed on and where
    /// the ghost stood then.
    Ghost { grab: (i32, i32), start: (i32, i32) },
}

/// Which of the map's keys a CLI flag overrides at PLAY, so the settings
/// panel can say why a value is not honoured. Set by `main.rs` from its
/// `Args`; the dev server reads the same from `Game`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CliOverrides {
    pub tanks: bool,
    pub tank: bool,
    pub tank2: bool,
    pub mission: bool,
    pub spawn: bool,
    pub waves: bool,
    pub wave_size: bool,
    pub wave_growth: bool,
    pub tier_start: bool,
    pub tier_end: bool,
}

/// One revision the clear check has seen won (`MapEditor::clears`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Clear {
    /// The best win's round clock, in seconds to a tenth.
    par: f64,
    /// The canvas's key when it was that revision (`note_clear`); `None`
    /// where the canvas was not, so every key may be it.
    key: Option<ClearKey>,
}

/// What `MapEditor::par` compares before it works a revision out: the
/// map's size, its cell count and its cells' digest (`CellIndex::digest`),
/// all kept per edit. Two maps of one revision have one key, so a key no
/// cleared revision has is a canvas that is not cleared; only a match - an
/// undo back, a settings edit - costs the revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ClearKey {
    size: Option<(u32, u32)>,
    cells: usize,
    digest: u64,
}

pub struct MapEditor {
    map: MapFile,
    /// The map as it was when the builder was seeded or last loaded -
    /// what `dirty` compares against and what RESET returns to.
    baseline: MapFile,
    /// Each category's current tool, in `Category::ALL` order.
    current: [Tool; 5],
    active_tool: Tool,
    /// The category the brush came from last: the folded TOOLS button
    /// shows its current tool while the eraser is the brush.
    last_category: Category,
    ground: GroundGrid,
    /// Fixed for this builder session (rolled once in `new`) and reused by
    /// every `rebuild_ground` call - see `ground::build`'s `seed` param -
    /// so grass tiles do not re-randomise on every edit.
    ground_seed: u64,
    popup: Option<Popup>,
    /// One-line feedback (saved/error) shown at the field's bottom.
    status: Option<String>,
    stroke: Option<Stroke>,
    history: UndoStack,
    /// The last pointer position `update` saw, on the canvas's bitmap -
    /// the hover highlight and the cursor readout draw from it, so a touch
    /// screen keeps showing the last tapped cell.
    pointer: Option<Vec2>,
    /// The canvas's camera: FIT, or a zoom and where it looks.
    camera: BuilderCamera,
    /// The screen and the canvas area `update` was last given - what a
    /// tool that moves the camera between frames measures against.
    screen: CanvasScreen,
    area: (f32, f32),
    /// Whether `update` has been given a frame yet: until it has, the
    /// canvas area is the canvas's own field - FIT at a texel a pixel, the
    /// frame `BuilderFrame::headless` lays out - whatever map it holds.
    framed: bool,
    /// Where the pointer was on the last frame of a pan drag (the middle
    /// button, or Space with the primary): the drag moves the canvas by
    /// the difference.
    pan_from: Option<Vec2>,
    /// Wheel movement not yet spent on a whole step, for a coarse
    /// screen's zoom (a trackpad sends fractions of a notch).
    wheel_accum: f32,
    /// A world rectangle to show, worked out against the next canvas
    /// area `update` is given (`look_at`).
    pending_look: Option<Rectangle>,
    /// The fingers on the canvas.
    gestures: gesture::Gestures,
    /// The builder is coming up (`new`, `leave`): its next `update` takes
    /// no finger already down for the canvas's - that finger landed on
    /// what the window showed before, a round's BUILD button.
    entering: bool,
    /// Where a painting finger is, for edge scroll.
    stroke_pointer: Option<Vec2>,
    /// The navigator's picture of the canvas (`minimap.rs`): made again
    /// with the ground and repainted with it, cell for cell.
    minimap: Minimap,
    /// A press on the navigator is being dragged: the view follows it
    /// until it lifts.
    nav_drag: bool,
    /// The MAP panel's ANCHOR: where the old map sits when its size
    /// changes. The panel's choice, not the map's.
    resize_anchor: Anchor,
    /// The undo depth just after a size stepper's last press, while the
    /// panel stays open: the next press folds into that step, so a run of
    /// presses is one undo step.
    resize_session: Option<usize>,
    /// Bumped by every change to the map (`map_changed`): what the CHECK
    /// panel's report is measured against.
    edits: u64,
    /// The CHECK panel's last run of the linter over the canvas: made when
    /// the panel opens and again on the panel's first frame after an edit.
    lint: Option<LintReport>,
    /// The finding whose cells the canvas marks, an index into `lint`'s
    /// findings: a press on its row picks it, and the next edit - which
    /// may well have answered it - lets it go.
    lint_marked: Option<usize>,
    /// What the canvas is linted under: the round PLAY would set up - the
    /// session's seed, seats and overrides (`mode::Session` refreshes it
    /// every frame).
    pub lint_setup: LintSetup,
    /// The clear check (docs/large-maps-patterns.md, "Clear check before
    /// sharing"): every revision of the canvas (`MapFile::revision`) this
    /// session has seen won from plain PLAY, with its par in seconds - the
    /// best win's - so an undo back to a cleared revision finds it cleared
    /// again. A map that comes in with a stamp for its own revision adds
    /// it here; the canvas itself never carries a stamp, and SAVE writes
    /// the current revision's (`map_to_save`).
    clears: BTreeMap<u64, Clear>,
    /// How the brush lays its object down (`Shape`); a singleton's brush
    /// is always a pen.
    shape: Shape,
    /// The tool before the select tool: what picking a shape goes back to.
    paint_tool: Tool,
    /// The select tool's rectangle, outlined until Escape, a press outside
    /// it or another tool clears it.
    selection: Option<CellRect>,
    /// What COPY and CUT took last, for PASTE.
    clipboard: Option<Clip>,
    /// The paste ghost standing on the canvas.
    ghost: Option<Ghost>,
    /// The select tool's drag under way.
    drag: Option<Drag>,
    /// The shipped stamps, then the ones saved this session (in memory on
    /// every build: nothing here needs a file system).
    stamps: Vec<Stamp>,
    /// Stamps saved this session, for the next one's number.
    saved_stamps: u32,
    /// Scatter strokes made this session: each one's cells are hashed with
    /// its own count (`brush::scattered`), so a second stroke over the same
    /// ground picks other cells.
    scatters: u32,
    /// The Load list's thumbnails (`thumbs.rs`), kept by map name and the
    /// revision of the text each was made from.
    thumbs: thumbs::Thumbs,
    /// The window pointer on the last frame no button was held: a pointer
    /// that moves with nothing down is a mouse hovering - a touch screen
    /// moves it only under a finger - and the paste ghost follows it.
    hover: Option<Vec2>,
    /// The canvas's revision with the edit count it was worked out at
    /// (`edits`): worked out again only after an edit.
    revision: std::cell::Cell<Option<(u64, u64)>>,
    /// The canvas's cells as a frame reads them (`index.rs`): an edit of
    /// cells takes its cells in (`cells_changed`), any other edit lets it
    /// go to be worked out again on the next read.
    index: std::cell::OnceCell<CellIndex>,
    /// The baseline's, worked out on the first read after it moves.
    baseline_index: std::cell::OnceCell<CellIndex>,
    /// Whether the canvas differs from its baseline (`dirty`): worked out
    /// on the first read after an edit or a move of the baseline.
    dirty: std::cell::Cell<Option<bool>>,
    pub cli_overrides: CliOverrides,
    /// `render` draws a flat white field instead of the ground tileset -
    /// the builder-side twin of `Game::plain_canvas`.
    pub plain_canvas: bool,
}

impl MapEditor {
    /// Seed the canvas from `map` - the map the current round was built
    /// from, which becomes the baseline. The canvas is the map's own field
    /// (`MapFile::field_size`), never the window.
    pub fn new(mut map: MapFile) -> Self {
        let stamp = Self::take_stamp(&mut map);
        let current = [
            Tool::Wall(Material::Brick),
            Tool::Prop(Material::Sandbag),
            Tool::Road,
            Tool::Start,
            Tool::Pickup(PickupKind::Health),
        ];
        let area = map.field_size();
        let mut editor = MapEditor {
            baseline: map.clone(),
            map,
            current,
            active_tool: current[0],
            last_category: Category::Wall,
            ground: GroundGrid::default(),
            ground_seed: rand::rng().random(),
            plain_canvas: false,
            popup: None,
            status: None,
            stroke: None,
            history: UndoStack::default(),
            pointer: None,
            camera: BuilderCamera::default(),
            screen: CanvasScreen::default(),
            area,
            framed: false,
            pan_from: None,
            wheel_accum: 0.0,
            pending_look: None,
            gestures: gesture::Gestures::default(),
            entering: true,
            stroke_pointer: None,
            minimap: Minimap::default(),
            nav_drag: false,
            resize_anchor: Anchor::default(),
            resize_session: None,
            edits: 0,
            lint: None,
            lint_marked: None,
            lint_setup: LintSetup { seed: 0xB0B5, ..LintSetup::default() },
            clears: BTreeMap::new(),
            shape: Shape::default(),
            paint_tool: current[0],
            selection: None,
            clipboard: None,
            ghost: None,
            drag: None,
            stamps: select::shipped_stamps(),
            saved_stamps: 0,
            scatters: 0,
            thumbs: thumbs::Thumbs::default(),
            hover: None,
            revision: std::cell::Cell::new(None),
            index: std::cell::OnceCell::new(),
            baseline_index: std::cell::OnceCell::new(),
            dirty: std::cell::Cell::new(None),
            cli_overrides: CliOverrides::default(),
        };
        editor.note_stamp(stamp);
        editor.rebuild_ground();
        editor
    }

    // ----- the camera -----

    /// The canvas as the camera measures it: the map's field, the canvas
    /// area `update` was last given and the screen under it.
    pub fn viewport(&self) -> Viewport {
        Viewport { field: self.map.field_size(), area: self.area, screen: self.screen }
    }

    /// The same over `layout`'s field area - the bitmap a frame is drawn
    /// and hit-tested on.
    fn viewport_in(&self, layout: &Layout) -> Viewport {
        Viewport::of(self.map.field_size(), layout, self.screen)
    }

    /// The view the canvas is drawn and hit-tested through, over
    /// `layout`'s field area: `Camera::whole` at FIT on an arena.
    pub fn view_camera(&self, layout: &Layout) -> crate::view::Camera {
        self.camera.view(&self.viewport_in(layout))
    }

    /// The canvas's camera, FIT or zoomed.
    pub fn camera(&self) -> &BuilderCamera {
        &self.camera
    }

    /// Back to the whole canvas - the bar's FIT button.
    pub fn fit_camera(&mut self) {
        self.camera.fit();
    }

    /// Put the world point `center` in the middle of the canvas at `zoom`
    /// times FIT (1 is FIT), kept inside the field and the zoom's limits -
    /// the dev server's `builder_camera`.
    pub fn frame_camera(&mut self, center: Vec2, zoom: f32) {
        let vp = self.viewport();
        let scale = vp.fit_scale() * if zoom.is_finite() { zoom.max(1.0) } else { 1.0 };
        self.camera.set(center, scale, &vp, &CanvasRules::current());
    }

    /// Show the world rectangle `rect` - BUILD opening on what the round
    /// showed: FIT where it holds the whole field, else its middle at the
    /// zoom that fits it in the canvas area. Worked out again on the next
    /// `update`, which knows the canvas area this frame draws in.
    pub fn look_at(&mut self, rect: Rectangle) {
        self.pending_look = Some(rect);
        self.apply_look(rect);
    }

    fn apply_look(&mut self, rect: Rectangle) {
        let vp = self.viewport();
        let (w, h) = vp.field;
        if !(rect.width > 0.0 && rect.height > 0.0) || (rect.width >= w - 0.5 && rect.height >= h - 0.5) {
            self.camera.fit();
            return;
        }
        let scale = (vp.area.0 / rect.width).min(vp.area.1 / rect.height);
        let center = Vec2::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
        self.camera.set(center, scale, &vp, &CanvasRules::current());
    }

    /// The map cell under a window point, through the frame and the camera:
    /// `None` off the canvas, on the builder's chrome or past the field's
    /// edge. The one place a pointer becomes a cell, so a press paints the
    /// cell the canvas draws under it at every zoom.
    pub fn cell_at(&self, window: Vec2, frame: &BuilderFrame) -> Option<(i32, i32)> {
        self.canvas_cell(frame.to_canvas(window), frame)
    }

    /// The world point under a window point on the canvas, through the
    /// frame and the camera; `None` off the canvas area or on the chrome
    /// over it.
    pub fn world_at(&self, window: Vec2, frame: &BuilderFrame) -> Option<Vec2> {
        self.canvas_world(frame.to_canvas(window), frame)
    }

    /// `cell_at` for a point of the canvas's bitmap.
    fn canvas_cell(&self, bitmap: Vec2, frame: &BuilderFrame) -> Option<(i32, i32)> {
        let world = self.canvas_world(bitmap, frame)?;
        let (w, h) = self.map.field_size();
        (world.x >= 0.0 && world.x < w && world.y >= 0.0 && world.y < h).then(|| map::world_to_cell(world))
    }

    /// `world_at` for a point of the canvas's bitmap.
    fn canvas_world(&self, bitmap: Vec2, frame: &BuilderFrame) -> Option<Vec2> {
        if !self.on_canvas(bitmap, frame) {
            return None;
        }
        let layout = &frame.layout;
        Some(self.view_camera(layout).to_world(layout.to_field(bitmap)))
    }

    /// Whether a point of the canvas's bitmap is on the canvas area and not
    /// under the builder's chrome (the bar, an open popup, the navigator).
    fn on_canvas(&self, bitmap: Vec2, frame: &BuilderFrame) -> bool {
        frame.layout.field.contains(bitmap) && !self.point_on_ui(frame.canvas_to_ui(bitmap), frame)
    }

    /// Whether a finger landing at a window point is the canvas's: on the
    /// canvas, and no press of the chrome's takes it there. A bar button's
    /// hit rect reaches `EDITOR_BAR_HIT_SLACK` under the bar, over the
    /// canvas's top edge, and a finger there pressed the button; the
    /// strip's buttons and the navigator press only inside the plates
    /// `on_canvas` leaves out.
    fn lands_on_canvas(&self, window: Vec2, frame: &BuilderFrame) -> bool {
        frame.bar().hit(frame.to_ui(window)).is_none() && self.on_canvas(frame.to_canvas(window), frame)
    }

    /// Where the navigator's picture stands (docs/large-maps-follow-camera.md
    /// §9), in UI points: a minimap of the canvas in the bottom-right corner
    /// of the room under the bar (`chrome::navigator`) - the status line
    /// runs along the bottom-left, the bar's buttons and their popups hang
    /// from the top -, sized like play's minimap (`MinimapRules::size_pt`)
    /// and never more than half that room either way; its plate is
    /// `Corners::plate` round it. `None` at FIT on an arena, where it would
    /// show just what the canvas shows.
    pub fn navigator_rect(&self, frame: &BuilderFrame) -> Option<Rectangle> {
        if self.camera.is_fit() && self.map.class() == crate::framing::MapClass::Arena {
            return None;
        }
        chrome::navigator(frame.under_bar(), MinimapRules::current().size_pt(self.map.field_size()))
    }

    /// The navigator: a press on it puts the middle of the view on the
    /// world point under it (`BuilderCamera::navigate`), and a drag carries
    /// the view along under the pointer, held inside the picture, until it
    /// lifts - the mouse, a finger and the dev server's `click` and
    /// `builder_touch` alike, through the pointer and the press. A finger
    /// that lands there is no canvas finger (`on_canvas`), so no gesture
    /// takes it. Whether this frame was the navigator's.
    fn navigate(&mut self, input: &BuilderInput, frame: &BuilderFrame, rules: &CanvasRules) -> bool {
        let rect = self.navigator_rect(frame);
        let (Some(window), Some(rect), true) = (input.pointer, rect, input.held) else {
            self.nav_drag = false;
            return false;
        };
        let pointer = frame.to_ui(window);
        if input.pressed && Corners::plate(rect).contains(pointer) {
            self.settle();
            self.nav_drag = true;
        }
        if !self.nav_drag {
            return false;
        }
        let world = crate::minimap::to_world(rect, self.map.field_size(), pointer);
        let vp = self.viewport_in(&frame.layout);
        self.camera.navigate(world, &vp, rules);
        true
    }

    /// After the map was swapped whole (a load, a reset, a whole-map undo):
    /// a field of another size starts at FIT.
    fn refit_if_resized(&mut self, field_before: (f32, f32)) {
        if self.map.field_size() != field_before {
            self.camera.fit();
        }
    }

    // ----- the edit model: shared by the bar, the finger and the tools -----

    pub fn map(&self) -> &MapFile {
        &self.map
    }

    /// The canvas's ground layer, rebuilt after every edit - what `app.rs`
    /// uploads the floor shade of before the builder draws.
    pub fn ground(&self) -> &GroundGrid {
        &self.ground
    }

    pub fn baseline(&self) -> &MapFile {
        &self.baseline
    }

    /// The map's display name: its file stem, `default`, or the word for
    /// an unnamed one in the language on screen.
    pub fn name(&self) -> String {
        self.map.name.clone().unwrap_or_else(|| crate::text::text().get(crate::text::keys::EDITOR_UNTITLED))
    }

    /// Edited since it was seeded, loaded or last saved: its cells, its
    /// settings or its size differ from the baseline's - what `diff` finds
    /// non-empty, compared on the kept indexes rather than the maps, once
    /// per edit (the bar's `*`, the status line and the MAP panel's RESET
    /// read it every frame).
    pub fn dirty(&self) -> bool {
        if let Some(dirty) = self.dirty.get() {
            return dirty;
        }
        let dirty = self.cell_index().cells() != self.baseline_index.get_or_init(|| CellIndex::of(&self.baseline)).cells()
            || MapSettings::of(&self.map) != MapSettings::of(&self.baseline)
            || self.map.field_size() != self.baseline.field_size();
        self.dirty.set(Some(dirty));
        dirty
    }

    /// The canvas's cells as a frame reads them (`CellIndex`), kept across
    /// frames and edits.
    pub fn cell_index(&self) -> &CellIndex {
        self.index.get_or_init(|| CellIndex::of(&self.map))
    }

    pub fn diff(&self) -> MapDiff {
        MapDiff::between(&self.baseline, &self.map)
    }

    pub fn tool(&self) -> Tool {
        self.active_tool
    }

    /// The category the active tool belongs to (`None` for the eraser).
    pub fn active_category(&self) -> Option<Category> {
        self.active_tool.category()
    }

    /// A category's current tool.
    pub fn current_tool(&self, category: Category) -> Tool {
        self.current[category.index()]
    }

    /// Make `tool` the active brush and its category's current tool. The
    /// select tool remembers the tool it came from (`paint_tool`); any other
    /// tool lands a drag under way and clears the selection and the paste
    /// ghost.
    pub fn select_tool(&mut self, tool: Tool) {
        if let Some(category) = tool.category() {
            self.current[category.index()] = tool;
            self.last_category = category;
        }
        if tool == Tool::Select {
            if self.active_tool != Tool::Select {
                self.finish_stroke();
                self.paint_tool = self.active_tool;
            }
        } else {
            self.finish_drag();
            self.selection = None;
            self.ghost = None;
        }
        self.active_tool = tool;
    }

    /// The tool the folded TOOLS button shows: the brush, or while the
    /// eraser or the select tool is the brush the category tool it came
    /// from.
    pub fn tools_button_tool(&self) -> Tool {
        match self.active_tool.category() {
            Some(_) => self.active_tool,
            None => self.current_tool(self.last_category),
        }
    }

    /// How the brush lays its object down: its shape, or a pen for a
    /// singleton's brush, which moves one object rather than painting many.
    pub fn shape(&self) -> Shape {
        let tool = if self.active_tool == Tool::Select { self.paint_tool } else { self.active_tool };
        if tool.is_singleton() { Shape::Pen } else { self.shape }
    }

    /// Paint with `shape`: the select tool gives way to the tool it came
    /// from. A singleton's brush takes the pen only (`shape_allowed`).
    pub fn set_shape(&mut self, shape: Shape) {
        if self.active_tool == Tool::Select {
            self.select_tool(self.paint_tool);
        }
        if self.shape_allowed(shape) {
            self.shape = shape;
        }
    }

    /// Whether the brush can take `shape`: anything but a singleton's
    /// brush takes every shape, a singleton's the pen alone.
    pub fn shape_allowed(&self, shape: Shape) -> bool {
        let tool = if self.active_tool == Tool::Select { self.paint_tool } else { self.active_tool };
        shape == Shape::Pen || !tool.is_singleton()
    }

    /// What BRUSH shows: the select tool while it is the brush, else the
    /// brush's shape.
    pub fn brush_shown(&self) -> BrushRow {
        if self.active_tool == Tool::Select { BrushRow::Select } else { BrushRow::Shape(self.shape()) }
    }

    /// Step a category's current tool forwards or backwards through its
    /// list (the mouse wheel over its button) and make it active.
    pub fn cycle_tool(&mut self, category: Category, forward: bool) {
        let tools: Vec<Tool> = category.tools().collect();
        let at = tools.iter().position(|&t| t == self.current[category.index()]).unwrap_or(0);
        let next = if forward { (at + 1) % tools.len() } else { (at + tools.len() - 1) % tools.len() };
        self.select_tool(tools[next]);
    }

    pub fn history(&self) -> &UndoStack {
        &self.history
    }

    /// Take the newest step back (`UndoStack::undo`). Whether there was
    /// one; it is `history().last_undone()`.
    pub fn undo(&mut self) -> bool {
        self.step_history(-1)
    }

    /// Make the newest step taken back again (`UndoStack::redo`). Whether
    /// there was one; it is `history().last_done()`.
    pub fn redo(&mut self) -> bool {
        self.step_history(1)
    }

    /// Undo (`way` -1) or redo (1) a step, the canvas following it: the
    /// history is taken out while it does, so the step it moved is read
    /// where it lies - its cells taken into the kept index, the ground and
    /// the camera following it - rather than cloned, which for a load, a
    /// clear or a resize is two whole maps.
    fn step_history(&mut self, way: i32) -> bool {
        self.finish_stroke();
        self.finish_drag();
        self.resize_session = None;
        let field = self.map.field_size();
        let mut history = std::mem::take(&mut self.history);
        let step = if way < 0 { history.undo(&mut self.map) } else { history.redo(&mut self.map) };
        let done = step.is_some();
        if let Some(step) = step {
            match step {
                EditStep::Cells(changes) => self.cells_changed(changes),
                _ => self.map_changed(),
            }
            self.ground_after(step);
            self.follow_whole_map_step(step, field, way);
        }
        self.history = history;
        done
    }

    /// Every change to the map comes through here: the CHECK panel's
    /// report is now older than the map (run again on the panel's next
    /// frame), the canvas lets go of the finding it was marking, and what
    /// is kept of the map per edit - its revision, its cells as a frame
    /// reads them, whether it differs from the baseline - is worked out
    /// again when next read.
    fn map_changed(&mut self) {
        self.edits += 1;
        self.lint_marked = None;
        self.index.take();
        self.dirty.set(None);
        if !self.framed {
            self.area = self.map.field_size();
        }
    }

    /// `map_changed` for an edit of the cells `changes` touched alone: the
    /// kept index takes them in (`CellIndex::update`) rather than being
    /// worked out again from the whole map on the next read - a stroke on
    /// a large map costs a search a cell a frame, not a sort of the map.
    fn cells_changed(&mut self, changes: &[CellChange]) {
        let index = self.index.take();
        self.map_changed();
        if let Some(mut index) = index {
            index.update(&self.map, changes);
            let _ = self.index.set(index);
        }
    }

    /// The canvas as it stands becomes the baseline `dirty` is measured
    /// from: a load, a new document, a save.
    fn set_baseline(&mut self) {
        self.baseline = self.map.clone();
        self.baseline_index.take();
        self.dirty.set(None);
    }

    /// The camera after a step was undone (`way` -1) or redone (1): a
    /// resize moves a zoomed view with the map, and any other step that
    /// left a field of another size starts it at FIT.
    fn follow_whole_map_step(&mut self, step: &EditStep, field_before: (f32, f32), way: i32) {
        match step {
            EditStep::Resize { shift, .. } => {
                let cell = crate::OBSTACLE_GRID_SIZE;
                self.camera.shift(Vec2::new((way * shift.0) as f32 * cell, (way * shift.1) as f32 * cell));
            }
            _ => self.refit_if_resized(field_before),
        }
    }

    /// The map's size in cells: its `size`, or the standard 34 x 17.
    pub fn size_cells(&self) -> (f32, f32) {
        let (w, h) = self.map.field_size();
        (w / crate::OBSTACLE_GRID_SIZE, h / crate::OBSTACLE_GRID_SIZE)
    }

    /// The MAP panel's ANCHOR.
    pub fn resize_anchor(&self) -> Anchor {
        self.resize_anchor
    }

    pub fn set_resize_anchor(&mut self, anchor: Anchor) {
        self.resize_anchor = anchor;
    }

    /// Make the map `cols` x `rows` cells - each kept between
    /// `MIN_MAP_CELLS` and `map::MAX_SIDE_CELLS` - with the old map placed
    /// by `anchor`, as one undo step. Cells that land past the new field's
    /// edge are dropped, and undo brings them back. The canvas's ground is
    /// made again on the new field and a zoomed view moves with the map.
    /// Whether the size changed.
    pub fn resize(&mut self, cols: f32, rows: f32, anchor: Anchor) -> bool {
        self.resize_map(cols, rows, anchor, false)
    }

    /// `resize`, folded into the last step when `fold` and that step was
    /// this panel's last size press.
    fn resize_map(&mut self, cols: f32, rows: f32, anchor: Anchor, fold: bool) -> bool {
        self.finish_stroke();
        self.finish_drag();
        if !(cols.is_finite() && rows.is_finite()) {
            return false;
        }
        let old = self.size_cells();
        let new = (cols.clamp(MIN_MAP_CELLS.0, map::MAX_SIDE_CELLS), rows.clamp(MIN_MAP_CELLS.1, map::MAX_SIDE_CELLS));
        if new == old {
            return false;
        }
        // The selection names cells the anchor moves: let it go.
        self.selection = None;
        let shift = anchor.shift(old, new);
        let before = self.map.clone();
        let cell = crate::OBSTACLE_GRID_SIZE;
        let (width, height) = (new.0 * cell, new.1 * cell);
        self.map.size = Some(new);
        self.map.cells.clear();
        for (col, row, obj) in before.iter_cells() {
            let (col, row) = (col + shift.0, row + shift.1);
            // A cell is on the field while its middle is: the cells a
            // pointer on the field can paint.
            let at = map::cell_to_world(col, row);
            if at.x >= 0.0 && at.y >= 0.0 && at.x <= width && at.y <= height {
                self.map.set_cell(col, row, *obj);
            }
        }
        let folded = fold
            && self.resize_session == Some(self.history.undo_depth())
            && match self.history.last_mut() {
                Some(EditStep::Resize { after, shift: total, .. }) => {
                    **after = self.map.clone();
                    *total = (total.0 + shift.0, total.1 + shift.1);
                    true
                }
                _ => false,
            };
        if !folded {
            self.history.push(EditStep::Resize { before: Box::new(before), after: Box::new(self.map.clone()), shift });
        }
        self.resize_session = fold.then(|| self.history.undo_depth());
        self.camera.shift(Vec2::new(shift.0 as f32 * cell, shift.1 as f32 * cell));
        self.map_changed();
        self.rebuild_ground();
        true
    }

    pub fn settings(&self) -> MapSettings {
        MapSettings::of(&self.map)
    }

    /// Set the MAP menu values as one undo step. Out-of-range numbers are
    /// clamped; a no-op change records nothing.
    pub fn apply_settings(&mut self, new: MapSettings) {
        let before = self.settings();
        let mut after = new;
        after.tanks = after.tanks.map(|n| n.min(crate::tuning::tuning().wave_max_alive as u32));
        after.waves = after.waves.map(|n| n.clamp(1, 20));
        after.size = after.size.map(|n| n.clamp(1, 31));
        after.growth = after.growth.map(|n| n.min(10));
        if after == before {
            return;
        }
        self.settle();
        after.write_to(&mut self.map);
        self.history.push(EditStep::Settings { before, after });
        self.map_changed();
        if after.theme != before.theme {
            self.rebuild_ground();
        }
    }

    /// Replace the canvas with `map` and make it the new baseline, as one
    /// undo step - the dev server's `builder_map {map_toml}`. The selection
    /// goes with the old map; the clipboard and a paste ghost stay, so a
    /// piece of one map pastes into another.
    pub fn load(&mut self, mut map: MapFile) {
        self.finish_stroke();
        self.finish_drag();
        self.selection = None;
        let stamp = Self::take_stamp(&mut map);
        let before = self.map.clone();
        self.map = map;
        self.set_baseline();
        self.history.push(EditStep::Map { before: Box::new(before), after: Box::new(self.map.clone()) });
        self.map_changed();
        self.note_stamp(stamp);
        self.rebuild_ground();
        self.camera.fit();
        self.pending_look = None;
    }

    /// Open `map` as a new document: the canvas, the baseline and an
    /// empty undo history - the level a round has moved on to
    /// (`mode::Session::start_level`), which undo must not walk back out
    /// of into the level before. The camera starts at FIT.
    pub fn open(&mut self, mut map: MapFile) {
        self.finish_stroke();
        self.finish_drag();
        self.selection = None;
        self.popup = None;
        self.status = None;
        let stamp = Self::take_stamp(&mut map);
        self.map = map;
        self.set_baseline();
        self.history.clear();
        self.map_changed();
        self.note_stamp(stamp);
        self.rebuild_ground();
        self.camera.fit();
        self.pending_look = None;
    }

    /// Revert cells and settings to the baseline, as one undo step.
    pub fn reset(&mut self) {
        self.settle();
        if !self.dirty() {
            return;
        }
        let field = self.map.field_size();
        let before = self.map.clone();
        let name = self.map.name.take();
        self.map = self.baseline.clone();
        self.map.name = name;
        self.history.push(EditStep::Map { before: Box::new(before), after: Box::new(self.map.clone()) });
        self.map_changed();
        self.rebuild_ground();
        self.refit_if_resized(field);
    }

    /// Empty the canvas of every placed object - walls, props, trees,
    /// ground, actors, gates and pickups - as one undo step, keeping the
    /// map's settings, size, theme and name: the FILE menu's `CLEAR MAP`,
    /// a map started from scratch without an empty file on disk. Not a new
    /// baseline, so the map reads as edited until saved; an already empty
    /// canvas records nothing.
    pub fn clear(&mut self) {
        self.settle();
        if self.map.cells.is_empty() {
            return;
        }
        let before = self.map.clone();
        self.map.cells.clear();
        self.history.push(EditStep::Map { before: Box::new(before), after: Box::new(self.map.clone()) });
        self.map_changed();
        self.rebuild_ground();
    }

    /// One whole stroke at once: a press on `cells[0]`, a drag through the
    /// rest, a release. `right` is the secondary button (always erase).
    /// Replies with what changed. The tools' entry point; the UI drives
    /// `begin_stroke`/`stroke_to`/`finish_stroke` frame by frame instead.
    pub fn stroke(&mut self, cells: &[(i32, i32)], right: bool) -> Vec<CellChange> {
        let Some(&first) = cells.first() else { return Vec::new() };
        self.finish_stroke();
        self.begin_stroke(first, right);
        for &cell in &cells[1..] {
            self.stroke_to(cell);
        }
        self.finish_stroke_changes()
    }

    /// The first cell of a press: decides paint or erase for the whole
    /// stroke and lays the shape's first cells. Erase when the secondary
    /// button is down, the eraser is the brush, or - for a pen or a fill
    /// (`Shape::toggles`) - the cell already holds exactly the brush's
    /// object. A pen paints the cell, a fill floods from it and a scatter
    /// lays its footprint round it; a rectangle waits for the release.
    fn begin_stroke(&mut self, cell: (i32, i32), right: bool) {
        let shape = self.shape();
        let erase = right
            || self.active_tool == Tool::Eraser
            || (shape.toggles() && self.active_tool.object().is_some_and(|obj| self.map.cell(cell.0, cell.1) == Some(&obj)));
        if shape == Shape::Scatter {
            self.scatters = self.scatters.wrapping_add(1);
        }
        self.stroke = Some(Stroke { shape, erase, from: cell, last_cell: cell, seed: self.scatters, changes: Vec::new() });
        match shape {
            Shape::Pen => self.paint(cell),
            Shape::Rect => {}
            Shape::Fill => self.fill(cell),
            Shape::Scatter => self.scatter(cell),
        }
    }

    /// The drag crossed into `cell` (or stayed on the last one, a no-op): a
    /// pen paints it, a scatter lays its footprint round it and a
    /// rectangle stretches its corner to it. A fill is the press alone.
    fn stroke_to(&mut self, cell: (i32, i32)) {
        let Some(stroke) = &mut self.stroke else { return };
        if stroke.last_cell == cell || stroke.shape == Shape::Fill {
            return;
        }
        stroke.last_cell = cell;
        match stroke.shape {
            Shape::Pen => self.paint(cell),
            Shape::Scatter => self.scatter(cell),
            Shape::Rect | Shape::Fill => {}
        }
    }

    /// The pointer moved from the last cell to `cell`, possibly further
    /// than one cell in a frame: paint every cell on the way, stepping one
    /// axis at a time, so a quick or diagonal drag lays a line joined edge
    /// to edge - a river that touches only at corners draws as a string of
    /// pools - and a scatter misses no ground between frames. A
    /// rectangle's corner goes straight there. The tools' `stroke` takes
    /// its cells as given.
    fn drag_to(&mut self, cell: (i32, i32)) {
        let Some((from, shape)) = self.stroke.as_ref().map(|s| (s.last_cell, s.shape)) else { return };
        if shape == Shape::Rect {
            self.stroke_to(cell);
            return;
        }
        for step in edge_joined_line(from, cell) {
            self.stroke_to(step);
        }
    }

    /// The rectangle a RECT stroke is drawing, inside the field, and
    /// whether it erases: what the canvas outlines until the release fills
    /// it.
    pub fn rect_stroke(&self) -> Option<(CellRect, bool)> {
        let stroke = self.stroke.as_ref().filter(|s| s.shape == Shape::Rect)?;
        CellRect::spanning(stroke.from, stroke.last_cell).within(self.field_cells()).map(|rect| (rect, stroke.erase))
    }

    /// The status line's feedback - a save, a load, a fill refused - if
    /// there is any.
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    fn finish_stroke(&mut self) {
        self.finish_stroke_changes();
    }

    /// End whatever the pointer has under way - a stroke or a drag of the
    /// select tool - as its release would: before an edit of the whole map,
    /// a press on the chrome, a pan.
    fn settle(&mut self) {
        self.finish_stroke();
        self.finish_drag();
    }

    /// Take the open stroke back, cell by cell, as if it had never been:
    /// no undo step - a second finger landing on a stroke means a pinch,
    /// not paint.
    fn cancel_stroke(&mut self) {
        let Some(stroke) = self.stroke.take() else { return };
        for c in stroke.changes.iter().rev() {
            match c.before {
                Some(obj) => self.map.set_cell(c.col, c.row, obj),
                None => self.map.clear_cell(c.col, c.row),
            }
        }
        if !stroke.changes.is_empty() {
            self.cells_changed(&stroke.changes);
        }
        self.repaint_ground(&stroke.changes);
    }

    fn finish_stroke_changes(&mut self) -> Vec<CellChange> {
        if self.stroke.as_ref().is_some_and(|s| s.shape == Shape::Rect) {
            self.fill_rect();
        }
        let Some(stroke) = self.stroke.take() else { return Vec::new() };
        let changes = stroke.changes;
        if !changes.is_empty() {
            self.history.push(EditStep::Cells(changes.clone()));
        }
        changes
    }

    /// Apply the stroke's mode to one cell and record what changed.
    fn paint(&mut self, (col, row): (i32, i32)) {
        let erase = self.stroke.as_ref().is_some_and(|s| s.erase);
        let mut changes = Vec::new();
        let before = self.map.cell(col, row).copied();
        if erase {
            if before.is_some() {
                self.map.clear_cell(col, row);
                changes.push(CellChange { col, row, before, after: None });
            }
        } else if let Some(obj) = self.active_tool.object() {
            if self.active_tool.is_singleton() {
                let index = self.cell_index();
                let old = match self.active_tool {
                    Tool::Frog => index.frog(),
                    Tool::Start => index.start(),
                    Tool::Start2 => index.start2(),
                    Tool::EnemyFrog => index.enemy_frog(),
                    _ => None,
                };
                if let Some((oc, or)) = old.filter(|&c| c != (col, row)) {
                    self.map.clear_cell(oc, or);
                    changes.push(CellChange { col: oc, row: or, before: Some(obj), after: None });
                }
            }
            if before != Some(obj) {
                self.map.set_cell(col, row, obj);
                changes.push(CellChange { col, row, before, after: Some(obj) });
            }
        }
        if changes.is_empty() {
            return;
        }
        self.cells_changed(&changes);
        self.repaint_ground(&changes);
        if let Some(stroke) = &mut self.stroke {
            stroke.changes.extend(changes);
        }
    }

    /// Make each of `cells` hold its object (`None` clears it) as the open
    /// stroke's, the ground and the navigator repainted round them in one
    /// batch.
    fn lay(&mut self, cells: impl IntoIterator<Item = ((i32, i32), Option<CellObject>)>) {
        let mut changes = Vec::new();
        for ((col, row), after) in cells {
            self.change_cell(col, row, after, &mut changes);
        }
        if changes.is_empty() {
            return;
        }
        self.cells_changed(&changes);
        self.repaint_ground(&changes);
        if let Some(stroke) = &mut self.stroke {
            stroke.changes.extend(changes);
        }
    }

    /// What the open stroke lays on a cell: nothing to clear it, else the
    /// brush's object. `None` with no stroke open, or a paint stroke with
    /// no object to paint.
    fn stroke_object(&self) -> Option<Option<CellObject>> {
        let stroke = self.stroke.as_ref()?;
        if stroke.erase { Some(None) } else { self.active_tool.object().map(Some) }
    }

    /// FILL from `cell` as the open stroke's (`brush::flood`): the region
    /// that holds what `cell` holds made the brush's object, or cleared by
    /// an erase. A region past `builder_fill_max_cells` is left as it is,
    /// the status line saying why.
    fn fill(&mut self, cell: (i32, i32)) {
        let Some(after) = self.stroke_object() else { return };
        if self.map.cell(cell.0, cell.1).copied() == after {
            return;
        }
        let max = BrushRules::current().fill_max_cells;
        match brush::flood(&self.map, self.field_cells(), cell, max) {
            Some(cells) => self.lay(cells.into_iter().map(|c| (c, after))),
            None => self.status = Some(crate::text::text().fmt(crate::text::keys::EDITOR_FILL_TOO_LARGE, &[("n", max.into())])),
        }
    }

    /// RECT's release: every cell of the open stroke's rectangle on the
    /// field made the brush's object, or cleared by an erase.
    fn fill_rect(&mut self) {
        let (Some((rect, _)), Some(after)) = (self.rect_stroke(), self.stroke_object()) else { return };
        self.lay(rect.cells().map(|c| (c, after)));
    }

    /// SCATTER's footprint round `cell` as the open stroke's: of the cells
    /// the stroke's hash picks (`brush::scattered`), an empty one takes the
    /// brush's object - a scatter never paints over a cell - and an erase
    /// clears one holding the brush's object, or anything under the
    /// eraser.
    fn scatter(&mut self, cell: (i32, i32)) {
        let Some(stroke) = &self.stroke else { return };
        let (erase, seed) = (stroke.erase, stroke.seed);
        let rules = BrushRules::current();
        let object = self.active_tool.object();
        let picked: Vec<_> = brush::footprint(cell, rules.scatter_radius, self.field_cells())
            .filter(|&c| brush::scattered(c, seed, rules.scatter_density))
            .filter_map(|(col, row)| {
                let held = self.map.cell(col, row).copied();
                if erase {
                    (held.is_some() && (object.is_none() || held == object)).then_some(((col, row), None))
                } else {
                    (held.is_none() && object.is_some()).then_some(((col, row), object))
                }
            })
            .collect();
        self.lay(picked);
    }

    /// Make a lint quick fix (`maplint::LintFix`) on the map as one undo
    /// step, like a stroke: a move clears its cell and fills the empty one,
    /// a placement fills an empty cell, a removal clears one. A fix the map
    /// no longer matches - its object gone, its target taken - changes
    /// nothing. Whether it was made.
    pub fn apply_fix(&mut self, fix: LintFix) -> bool {
        self.settle();
        let cell = |map: &MapFile, (col, row): (i32, i32)| map.cell(col, row).copied();
        let changes: Vec<CellChange> = match fix {
            LintFix::Move { from, to } => match (cell(&self.map, from), cell(&self.map, to)) {
                (Some(obj), None) if from != to => vec![
                    CellChange { col: from.0, row: from.1, before: Some(obj), after: None },
                    CellChange { col: to.0, row: to.1, before: None, after: Some(obj) },
                ],
                _ => return false,
            },
            LintFix::Place { object, at } => match cell(&self.map, at) {
                None => vec![CellChange { col: at.0, row: at.1, before: None, after: Some(object) }],
                Some(_) => return false,
            },
            LintFix::Remove { at } => match cell(&self.map, at) {
                Some(obj) => vec![CellChange { col: at.0, row: at.1, before: Some(obj), after: None }],
                None => return false,
            },
        };
        for c in &changes {
            match c.after {
                Some(obj) => self.map.set_cell(c.col, c.row, obj),
                None => self.map.clear_cell(c.col, c.row),
            }
        }
        self.history.push(EditStep::Cells(changes.clone()));
        self.cells_changed(&changes);
        self.repaint_ground(&changes);
        true
    }

    // ----- the select tool (docs/large-maps-patterns.md, "Selection, copy
    // and stamps"; the module docs) -----

    /// The cells on the map's field (`select::field_cells`): where a
    /// selection, a move and a paste may put a cell.
    pub fn field_cells(&self) -> CellRect {
        select::field_cells(self.map.field_size())
    }

    /// The selection, if there is one.
    pub fn selection(&self) -> Option<CellRect> {
        self.selection
    }

    /// What COPY and CUT took last.
    pub fn clipboard(&self) -> Option<&Clip> {
        self.clipboard.as_ref()
    }

    /// The paste ghost standing on the canvas.
    pub fn ghost(&self) -> Option<&Ghost> {
        self.ghost.as_ref()
    }

    /// The stamps the STAMPS list offers: the shipped ones, then the ones
    /// saved this session.
    pub fn stamps(&self) -> &[Stamp] {
        &self.stamps
    }

    /// Where the selection's cells are being carried while a drag from
    /// inside it is under way: the rectangle and the cells, for the canvas
    /// to draw them lifted.
    pub fn lifted(&self) -> Option<(CellRect, &Clip)> {
        match &self.drag {
            Some(Drag::Move { by, from, lifted, .. }) => Some((from.shifted(*by), lifted)),
            _ => None,
        }
    }

    /// The rectangle a select drag is drawing, while one is.
    pub fn selecting(&self) -> Option<CellRect> {
        match self.drag {
            Some(Drag::Select { from, to, .. }) => CellRect::spanning(from, to).within(self.field_cells()),
            _ => None,
        }
    }

    /// Select `rect` - its part on the field - with the select tool, as a
    /// drag across it does; a paste ghost goes. Whether any of it is on the
    /// field.
    pub fn select_cells(&mut self, rect: CellRect) -> bool {
        self.select_tool(Tool::Select);
        self.finish_drag();
        self.ghost = None;
        self.selection = rect.within(self.field_cells());
        self.selection.is_some()
    }

    /// Let the selection go: Escape, a press outside it.
    pub fn clear_selection(&mut self) {
        self.finish_drag();
        self.selection = None;
    }

    /// COPY: the selection's cells into the clipboard. Whether there was a
    /// selection.
    pub fn copy_selection(&mut self) -> bool {
        self.finish_drag();
        let Some(rect) = self.selection else { return false };
        let clip = Clip::of(&self.map, rect);
        self.status = Some(crate::text::text().fmt(crate::text::keys::EDITOR_COPIED, &[("n", clip.cells.len().into())]));
        self.clipboard = Some(clip);
        true
    }

    /// CUT: COPY, then DELETE - one undo step, the delete's.
    pub fn cut_selection(&mut self) -> Vec<CellChange> {
        if !self.copy_selection() {
            return Vec::new();
        }
        let changes = self.delete_selection();
        self.status = Some(crate::text::text().fmt(crate::text::keys::EDITOR_CUT, &[("n", changes.len().into())]));
        changes
    }

    /// DELETE: every cell of the selection cleared, as one undo step. The
    /// selection stays, empty, for a paste into it.
    pub fn delete_selection(&mut self) -> Vec<CellChange> {
        self.finish_drag();
        let Some(rect) = self.selection else { return Vec::new() };
        let cells: Vec<(i32, i32)> = Clip::of(&self.map, rect).placed_at((rect.col, rect.row)).map(|(c, r, _)| (c, r)).collect();
        let mut changes = Vec::new();
        for (col, row) in cells {
            self.change_cell(col, row, None, &mut changes);
        }
        self.commit(changes)
    }

    /// FLIP: the paste ghost mirrored where it stands, or else the
    /// selection's cells mirrored in place along `axis`, as one undo step.
    pub fn flip(&mut self, axis: Axis) -> Vec<CellChange> {
        self.finish_drag();
        if let Some(ghost) = &mut self.ghost {
            ghost.clip = ghost.clip.flipped(axis);
            return Vec::new();
        }
        let Some(rect) = self.selection else { return Vec::new() };
        let at = (rect.col, rect.row);
        let clip = Clip::of(&self.map, rect);
        // Every cell the old cells or the mirrored ones touch, each to what
        // the mirror leaves there: one change a cell, whatever it held.
        let mut after: BTreeMap<(i32, i32), Option<CellObject>> = clip.placed_at(at).map(|(c, r, _)| ((c, r), None)).collect();
        after.extend(clip.flipped(axis).placed_at(at).map(|(c, r, obj)| ((c, r), Some(obj))));
        let mut changes = Vec::new();
        for ((col, row), obj) in after {
            self.change_cell(col, row, obj, &mut changes);
        }
        self.commit(changes)
    }

    /// Move the selection's cells `by` whole cells as one undo step - what
    /// a drag from inside it does, kept on the field - and select where
    /// they went.
    pub fn move_selection(&mut self, by: (i32, i32)) -> Vec<CellChange> {
        self.settle();
        let Some(from) = self.selection else { return Vec::new() };
        let grab = (from.col, from.row);
        self.lift(grab);
        self.drag_over((grab.0 + by.0, grab.1 + by.1));
        self.finish_drag()
    }

    /// PASTE: the clipboard as the paste ghost, its middle on `near` (a
    /// cell) or else on the middle of the view, with the select tool.
    /// Whether there was anything to paste.
    pub fn paste(&mut self, near: Option<(i32, i32)>) -> bool {
        let Some(clip) = self.clipboard.clone() else { return false };
        self.start_ghost(clip, near);
        true
    }

    /// Make stamp `index` the paste ghost in the middle of the view - the
    /// STAMPS list's row. Whether there is such a stamp.
    pub fn use_stamp(&mut self, index: usize) -> bool {
        let Some(clip) = self.stamps.get(index).map(|s| s.clip.clone()) else { return false };
        self.start_ghost(clip, None);
        true
    }

    /// Keep the selection's cells as a stamp for this session, trimmed to
    /// the cells it holds, at the end of the STAMPS list - in memory, on
    /// every build. The new stamp's index; `None` with nothing selected or
    /// nothing in the selection.
    pub fn save_stamp(&mut self) -> Option<usize> {
        self.finish_drag();
        let rect = self.selection?;
        let clip = Clip::of(&self.map, rect).trimmed();
        let t = crate::text::text();
        if clip.is_empty() {
            self.status = Some(t.get(crate::text::keys::EDITOR_STAMP_EMPTY));
            return None;
        }
        self.saved_stamps += 1;
        let stamp = Stamp { source: StampSource::Saved(self.saved_stamps), clip };
        self.status = Some(t.fmt(crate::text::keys::EDITOR_STAMP_KEPT, &[("name", stamp.label().into())]));
        self.stamps.push(stamp);
        Some(self.stamps.len() - 1)
    }

    /// Put the paste ghost down where it stands, as one undo step: every
    /// cell of it on the field written over what was there - a start,
    /// player 2's start or a frog only where the map holds none of it by
    /// then (the module docs) - and the rectangle it covered selected.
    pub fn place_ghost(&mut self) -> Vec<CellChange> {
        self.finish_drag();
        let Some(ghost) = self.ghost.take() else { return Vec::new() };
        let mut changes = Vec::new();
        self.put_down(&ghost.clip, ghost.at, &mut changes);
        self.selection = ghost.rect().within(self.field_cells());
        self.commit(changes)
    }

    /// Stand the paste ghost with its top-left at `at`, kept on the field.
    /// Whether there is a ghost.
    pub fn move_ghost(&mut self, at: (i32, i32)) -> bool {
        let bounds = self.field_cells();
        let Some(ghost) = &mut self.ghost else { return false };
        ghost.move_to(at, bounds);
        true
    }

    /// Take the paste ghost away: Escape, CANCEL.
    pub fn cancel_ghost(&mut self) {
        self.finish_drag();
        self.ghost = None;
    }

    /// `clip` as the paste ghost, its middle on `near` or the view's.
    fn start_ghost(&mut self, clip: Clip, near: Option<(i32, i32)>) {
        self.select_tool(Tool::Select);
        self.settle();
        let cell = near.unwrap_or_else(|| map::world_to_cell(self.camera.center(&self.viewport())));
        self.ghost = Some(Ghost::new(clip, cell, self.field_cells()));
    }

    /// Escape on the canvas: a rectangle being drawn or a drag taken back,
    /// else the paste ghost taken away, else the selection let go.
    fn escape(&mut self) {
        if self.stroke.as_ref().is_some_and(|s| s.shape == Shape::Rect) {
            self.cancel_stroke();
        } else if self.drag.is_some() {
            self.cancel_drag();
        } else if self.ghost.is_some() {
            self.ghost = None;
        } else {
            self.selection = None;
        }
    }

    /// Write `clip` with its top-left at `at`, noting each change: every
    /// cell of it on the field, the singletons last and each only where the
    /// map holds none of its kind by then - so a paste never leaves two
    /// starts and never touches a cell outside its own rectangle, while a
    /// moved or cut-and-pasted start lands where it was carried.
    fn put_down(&mut self, clip: &Clip, at: (i32, i32), changes: &mut Vec<CellChange>) {
        let bounds = self.field_cells();
        let (singles, rest): (Vec<_>, Vec<_>) =
            clip.placed_at(at).filter(|&(c, r, _)| bounds.contains((c, r))).partition(|(_, _, obj)| select::is_singleton(obj));
        for (col, row, obj) in rest {
            self.change_cell(col, row, Some(obj), changes);
        }
        for (col, row, obj) in singles {
            if !self.map.cells.values().any(|o| *o == obj) {
                self.change_cell(col, row, Some(obj), changes);
            }
        }
    }

    /// Make cell (`col`, `row`) hold `after`, noting the change in
    /// `changes` when it is one.
    fn change_cell(&mut self, col: i32, row: i32, after: Option<CellObject>, changes: &mut Vec<CellChange>) {
        let before = self.map.cell(col, row).copied();
        if before == after {
            return;
        }
        match after {
            Some(obj) => self.map.set_cell(col, row, obj),
            None => self.map.clear_cell(col, row),
        }
        changes.push(CellChange { col, row, before, after });
    }

    /// `changes`, already made, as one undo step, with the ground and the
    /// navigator's picture repainted round them like a stroke's. What
    /// changed.
    fn commit(&mut self, changes: Vec<CellChange>) -> Vec<CellChange> {
        if changes.is_empty() {
            return changes;
        }
        self.history.push(EditStep::Cells(changes.clone()));
        self.cells_changed(&changes);
        self.repaint_ground(&changes);
        changes
    }

    /// Take `changes` back, newest first, as if they had never been made:
    /// no undo step.
    fn unmake(&mut self, changes: &[CellChange]) {
        for c in changes.iter().rev() {
            match c.before {
                Some(obj) => self.map.set_cell(c.col, c.row, obj),
                None => self.map.clear_cell(c.col, c.row),
            }
        }
        if !changes.is_empty() {
            self.cells_changed(changes);
            self.repaint_ground(changes);
        }
    }

    /// Whether a stroke or a drag of the select tool is under way.
    fn dragging(&self) -> bool {
        self.stroke.is_some() || self.drag.is_some()
    }

    /// The select tool's press on `cell` with the mouse: the paste ghost
    /// put down with its middle there, or a drag begun.
    fn select_press(&mut self, cell: (i32, i32)) {
        let bounds = self.field_cells();
        if let Some(ghost) = &mut self.ghost {
            ghost.centre_on(cell, bounds);
            self.place_ghost();
        } else {
            self.begin_select_drag(cell, false);
        }
    }

    /// The select tool's drag from `cell`: a finger carries the paste
    /// ghost; a press inside the selection lifts its cells; anywhere else
    /// starts a new rectangle there. `touch`: a finger that has passed the
    /// slop, already a drag even on its first cell.
    fn begin_select_drag(&mut self, cell: (i32, i32), touch: bool) {
        self.settle();
        if let Some(ghost) = &self.ghost {
            if touch {
                self.drag = Some(Drag::Ghost { grab: cell, start: ghost.at });
            }
            return;
        }
        if self.selection.is_some_and(|s| s.contains(cell)) {
            self.lift(cell);
            return;
        }
        self.selection = None;
        self.drag = Some(Drag::Select { from: cell, to: cell, moved: touch });
    }

    /// A finger's tap on `cell` with the select tool: on the paste ghost it
    /// puts it down, off it the ghost moves there; with no ghost, a tap
    /// outside the selection lets it go.
    fn tap_select(&mut self, cell: (i32, i32)) {
        self.settle();
        let bounds = self.field_cells();
        if let Some(ghost) = &mut self.ghost {
            if ghost.rect().contains(cell) {
                self.place_ghost();
            } else {
                ghost.centre_on(cell, bounds);
            }
        } else if !self.selection.is_some_and(|s| s.contains(cell)) {
            self.selection = None;
        }
    }

    /// Lift the selection's cells off the map to carry them, the press
    /// landing on `grab`.
    fn lift(&mut self, grab: (i32, i32)) {
        let Some(from) = self.selection else { return };
        let lifted = Clip::of(&self.map, from);
        let cells: Vec<(i32, i32)> = lifted.placed_at((from.col, from.row)).map(|(c, r, _)| (c, r)).collect();
        let mut changes = Vec::new();
        for (col, row) in cells {
            self.change_cell(col, row, None, &mut changes);
        }
        if !changes.is_empty() {
            self.cells_changed(&changes);
            self.repaint_ground(&changes);
        }
        self.drag = Some(Drag::Move { grab, by: (0, 0), from, lifted, changes });
    }

    /// The pointer of a drag under way reached `cell`: a stroke paints the
    /// cells on the way, a rectangle stretches to it, lifted cells and a
    /// carried ghost follow it by whole cells, kept on the field.
    fn drag_over(&mut self, cell: (i32, i32)) {
        if self.stroke.is_some() {
            self.drag_to(cell);
            return;
        }
        let bounds = self.field_cells();
        match &mut self.drag {
            Some(Drag::Select { from, to, moved }) => {
                *moved |= cell != *from;
                *to = cell;
            }
            Some(Drag::Move { grab, by, from, .. }) => {
                let to = from.shifted((cell.0 - grab.0, cell.1 - grab.1)).kept_inside(bounds);
                *by = (to.col - from.col, to.row - from.row);
            }
            Some(Drag::Ghost { grab, start }) => {
                if let Some(ghost) = &mut self.ghost {
                    ghost.move_to((start.0 + cell.0 - grab.0, start.1 + cell.1 - grab.1), bounds);
                }
            }
            None => {}
        }
    }

    /// The release of a drag of the select tool: a rectangle drawn becomes
    /// the selection (a click selects nothing); lifted cells land where
    /// they were carried, the lift and the landing one undo step, and are
    /// selected there - or, carried nowhere, go back as they were with no
    /// step; a carried ghost stays where it was left. What a landing
    /// changed.
    fn finish_drag(&mut self) -> Vec<CellChange> {
        match self.drag.take() {
            Some(Drag::Select { from, to, moved }) => {
                self.selection = if moved { CellRect::spanning(from, to).within(self.field_cells()) } else { None };
                Vec::new()
            }
            Some(Drag::Move { by, from, lifted, mut changes, .. }) => {
                if by == (0, 0) {
                    self.unmake(&changes);
                    return Vec::new();
                }
                let to = from.shifted(by);
                self.put_down(&lifted, (to.col, to.row), &mut changes);
                self.selection = to.within(self.field_cells());
                self.commit(changes)
            }
            Some(Drag::Ghost { .. }) | None => Vec::new(),
        }
    }

    /// A drag of the select tool taken back - Escape, a second finger:
    /// lifted cells put back where they were with no step, a carried ghost
    /// back where it stood.
    fn cancel_drag(&mut self) {
        match self.drag.take() {
            Some(Drag::Move { changes, .. }) => self.unmake(&changes),
            Some(Drag::Ghost { start, .. }) => {
                if let Some(ghost) = &mut self.ghost {
                    ghost.at = start;
                }
            }
            Some(Drag::Select { .. }) | None => {}
        }
    }

    /// The buttons the select tool's strip shows now, each with whether it
    /// can act: with a paste ghost on the canvas PLACE, the flips and
    /// CANCEL; else the selection's actions, alive while there is one, and
    /// PASTE while the clipboard holds a clip. `None` unless the select
    /// tool is the brush and no popup is open.
    fn strip_buttons(&self) -> Option<Vec<(StripButton, bool)>> {
        if self.active_tool != Tool::Select || self.popup.is_some() {
            return None;
        }
        if self.ghost.is_some() {
            return Some(StripButton::GHOST.iter().map(|&b| (b, true)).collect());
        }
        let selected = self.selection.is_some();
        Some(
            StripButton::SELECTION
                .iter()
                .map(|&b| {
                    let live = match b {
                        StripButton::Paste => self.clipboard.is_some(),
                        StripButton::Stamps => true,
                        _ => selected,
                    };
                    (b, live)
                })
                .collect(),
        )
    }

    /// What a press on the strip does.
    fn press_strip(&mut self, button: StripButton) {
        match button {
            StripButton::Copy => {
                self.copy_selection();
            }
            StripButton::Cut => {
                self.cut_selection();
            }
            StripButton::Paste => {
                self.paste(None);
            }
            StripButton::FlipH => {
                self.flip(Axis::Horizontal);
            }
            StripButton::FlipV => {
                self.flip(Axis::Vertical);
            }
            StripButton::Delete => {
                self.delete_selection();
            }
            StripButton::SaveStamp => {
                self.save_stamp();
            }
            StripButton::Stamps => self.popup = Some(Popup::Stamps { scroll: 0 }),
            StripButton::Place => {
                self.place_ghost();
            }
            StripButton::Cancel => self.cancel_ghost(),
        }
    }

    /// Make the decorative ground layer again from the map's wall, road
    /// and water cells (`floor_of`) - for an edit of the whole map: a load,
    /// a reset, a resize, a theme. A cell edit repaints only its cells
    /// (`repaint_ground`).
    fn rebuild_ground(&mut self) {
        let (width, height) = self.map.field_size();
        let (mut road_cells, mut water_cells, mut wall_cells) = (Vec::new(), Vec::new(), Vec::new());
        for (col, row, obj) in self.map.iter_cells() {
            let floor = floor_of(Some(obj));
            let at = map::cell_to_world(col, row);
            for (on, cells) in [(floor.road, &mut road_cells), (floor.water, &mut water_cells), (floor.wall, &mut wall_cells)] {
                if on {
                    cells.push(at);
                }
            }
        }
        // The canvas shows the walls' shade but not the round's edge shade:
        // the author works right up to the frame.
        let look = ground::Look { theme: self.map.theme, edge_shade: false };
        self.ground = ground::build(width, height, self.ground_seed, &road_cells, &water_cells, &wall_cells, look);
        // The navigator's picture, the water as deep as the ground has it.
        self.minimap = Minimap::of_map(&self.map, |col, row| self.ground.depth(col, row));
    }

    /// Make the ground agree with the map where `changes` were made, the
    /// only cells an edit touched: `GroundGrid::repaint` resolves the tiles
    /// and bakes the floor shade again only around them, so a stroke across
    /// a large map costs what it does on a small one. The navigator's
    /// picture follows: those cells and every one whose water the repaint
    /// can have moved (`Minimap::repaint`), as a patch of only the texels
    /// that changed. An edit that moves the floor - road, water or a wall -
    /// under more than one cell in `REBUILD_SHARE` of the map's (a large
    /// RECT, FILL or paste, or the undo of one) makes the whole ground
    /// again instead (`rebuild_ground`), the same picture for less.
    fn repaint_ground(&mut self, changes: &[CellChange]) {
        let moved = changes.iter().filter(|c| floor_of(c.before.as_ref()) != floor_of(c.after.as_ref())).count();
        let (cols, rows) = crate::minimap::cells_of(self.map.field_size());
        if moved * REBUILD_SHARE > cols * rows {
            self.rebuild_ground();
            return;
        }
        let floors: Vec<_> = changes.iter().map(|c| (c.col, c.row, floor_of(self.map.cell(c.col, c.row)))).collect();
        let touched = self.ground.repaint(&floors);
        let (map, ground) = (&self.map, &self.ground);
        let edited = floors.iter().map(|&(col, row, _)| (col, row));
        self.minimap.repaint(touched.into_iter().chain(edited), |col, row| {
            let obj = map.cell(col, row);
            (Class::floor(obj, ground.depth(col, row)), obj.and_then(Class::solid_of))
        });
    }

    /// FILE > LOAD: the Load list on every map there is to load, its
    /// thumbnails checked again before they are shown (`thumbs.rs`).
    fn open_load_list(&mut self) {
        let entries = map::available_maps();
        self.thumbs.open(&entries);
        self.popup = Some(Popup::Load { entries, scroll: 0 });
    }

    /// `name`'s thumbnail in the Load list, once its page has made or
    /// checked it - nothing while the list is closed.
    pub fn thumbnail(&self, name: &str) -> Option<&thumbs::Thumb> {
        self.thumbs.get(name).filter(|_| matches!(self.popup, Some(Popup::Load { .. })))
    }

    /// Every thumbnail the open Load list has made or checked, by name:
    /// what `render.rs` holds a texture for. Nothing while the list is
    /// closed, which is what lets those textures go.
    pub fn thumbnails(&self) -> impl Iterator<Item = (&str, &thumbs::Thumb)> {
        let open = matches!(self.popup, Some(Popup::Load { .. }));
        self.thumbs.shown().filter(move |_| open)
    }

    /// The navigator's picture of the canvas (`minimap.rs`): what `app.rs`
    /// uploads before the builder draws.
    pub fn minimap(&self) -> &Minimap {
        &self.minimap
    }

    /// The ground after `step` was undone or redone: a stroke's cells
    /// repainted, a change of theme or of the whole map made again whole,
    /// any other setting left alone.
    fn ground_after(&mut self, step: &EditStep) {
        match step {
            EditStep::Cells(changes) => self.repaint_ground(&changes),
            EditStep::Settings { before, after } if before.theme == after.theme => {}
            _ => self.rebuild_ground(),
        }
    }

    // ----- the chrome -----

    /// Write the map to `maps/<name>.toml` (`name` defaults to the map's
    /// own name) and make the saved state the baseline. Native only:
    /// `map::saving_available` is false on the web. Returns the status
    /// line to show.
    pub fn save(&mut self, name: Option<&str>) -> Result<String, String> {
        let t = crate::text::text();
        if !map::saving_available() {
            return Err(t.get(crate::text::keys::EDITOR_SAVING_UNAVAILABLE));
        }
        let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => n.to_string(),
            None => self.map.name.clone().ok_or_else(|| t.get(crate::text::keys::EDITOR_NO_NAME))?,
        };
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(t.fmt(crate::text::keys::EDITOR_BAD_NAME, &[("name", name.as_str().into())]));
        }
        self.settle();
        let path = map::maps_dir().join(format!("{name}.toml"));
        self.map_to_save().save(&path)?;
        self.map.name = Some(name.clone());
        self.set_baseline();
        let line = t.fmt(crate::text::keys::EDITOR_SAVED, &[("name", name.as_str().into())]);
        self.status = Some(line.clone());
        Ok(line)
    }

    // ----- the clear check -----

    /// The canvas's revision (`MapFile::revision`), worked out again only
    /// after an edit.
    pub fn revision(&self) -> u64 {
        match self.revision.get() {
            Some((edits, revision)) if edits == self.edits => revision,
            _ => {
                let revision = self.map.revision();
                self.revision.set(Some((self.edits, revision)));
                revision
            }
        }
    }

    /// The canvas's par, in seconds, when its revision is cleared - won
    /// from plain PLAY with no edit since (`note_clear`); `None` while it
    /// is not, and while a stroke is being painted or a selection's cells
    /// are lifted, which is an edit under way: the revision is worked out
    /// once it ends rather than on every cell it paints. With nothing
    /// cleared this session nothing is worked out at all, and the revision
    /// - a pass over the whole map, about 0.3 s on a 250 x 250 map painted
    /// edge to edge in a release build - only where the canvas's
    /// `ClearKey` is a cleared revision's, which every edit away from one
    /// makes it not.
    pub fn par(&self) -> Option<f64> {
        if self.clears.is_empty() || self.stroke.is_some() || self.lifted().is_some() {
            return None;
        }
        let key = self.clear_key();
        if !self.clears.values().any(|clear| clear.key.is_none_or(|k| k == key)) {
            return None;
        }
        self.clears.get(&self.revision()).map(|clear| clear.par)
    }

    /// The canvas's `ClearKey`, off its kept index.
    fn clear_key(&self) -> ClearKey {
        let index = self.cell_index();
        ClearKey {
            size: self.map.size.map(|(cols, rows)| (cols.to_bits(), rows.to_bits())),
            cells: index.cells().len(),
            digest: index.digest(),
        }
    }

    /// `revision` was won from plain PLAY in `seconds` (`mode::Session`
    /// says so): it is cleared, and its par is its best win, to a tenth.
    /// The canvas's key goes with it where the canvas is that revision -
    /// as it is on a win, the canvas not touched since PLAY - and none
    /// where it is not, which `par` reads as "work the revision out".
    pub fn note_clear(&mut self, revision: u64, seconds: f64) {
        let par = (seconds * 10.0).round() / 10.0;
        let key = (self.revision() == revision).then(|| self.clear_key());
        let clear = self.clears.entry(revision).or_insert(Clear { par, key });
        clear.par = clear.par.min(par);
        if clear.key.is_none() {
            clear.key = key;
        }
    }

    /// A map that came in with the stamp `take_stamp` found on it, now the
    /// canvas: cleared, its revision - which `take_stamp` has just worked
    /// out - kept as the canvas's so it is not worked out twice.
    fn note_stamp(&mut self, stamp: Option<(u64, f64)>) {
        if let Some((revision, par)) = stamp {
            self.revision.set(Some((self.edits, revision)));
            self.note_clear(revision, par);
        }
    }

    /// The canvas as SAVE writes it: carrying its revision's stamp when
    /// that revision is cleared, so the file is proof of the win wherever
    /// it goes - a room hosts a map the command line hands it only so
    /// (`MapFile::hostable`).
    pub fn map_to_save(&self) -> MapFile {
        let mut map = self.map.clone();
        map.cleared = self.par().map(|par| map::Cleared::new(self.revision(), par));
        map
    }

    /// Take `map`'s stamp off it, answering the revision and par it holds
    /// when it is the map's own revision's.
    fn take_stamp(map: &mut MapFile) -> Option<(u64, f64)> {
        let stamp = map.cleared.take()?;
        let revision = map.revision();
        (stamp.revision == map::revision_text(revision)).then_some((revision, stamp.par))
    }

    /// Load a map from the Load list (`map::open_map`) into the canvas as
    /// one undo step and the new baseline.
    pub fn load_named(&mut self, name: &str) -> Result<(), String> {
        let map = map::open_map(name)?;
        self.load(map);
        self.status = Some(crate::text::text().fmt(crate::text::keys::EDITOR_LOADED, &[("name", name.into())]));
        Ok(())
    }

    /// The Save-as prompt is taking text: the keys a player types are
    /// characters, not commands (`Tab` must not start a round).
    pub fn text_entry_open(&self) -> bool {
        matches!(self.popup, Some(Popup::Save { .. }))
    }

    /// Close whatever popup is open (a dropdown, the settings panel or
    /// the Save prompt) without acting on it, and land a drag under way.
    pub fn close_popup(&mut self) {
        self.popup = None;
        self.settle();
    }

    /// Leave the builder - PLAY, PLAY HERE, Tab, a level opening - with
    /// nothing under way to come back to: the open popup closed; a RECT
    /// still being drawn and a drag of the select tool taken back, as
    /// Escape takes them (lifted cells go back where they were); a
    /// freehand stroke's cells, on the map already, kept as its one undo
    /// step; and every finger, pan and navigator drag let go. A finger
    /// down when the builder comes up again is not the canvas's
    /// (`entering`).
    pub fn leave(&mut self) {
        self.popup = None;
        if self.stroke.as_ref().is_some_and(|s| s.shape == Shape::Rect) {
            self.cancel_stroke();
        }
        self.cancel_drag();
        self.settle();
        self.gestures = gesture::Gestures::default();
        self.entering = true;
        self.stroke_pointer = None;
        self.pan_from = None;
        self.nav_drag = false;
    }

    /// Which popup is open, as the dev server's `mode` tool spells it: a
    /// category's dropdown by the category's name, `brush` for BRUSH's
    /// list, `tools` for the folded bar's palette, `stamps` for the STAMPS
    /// list, `map` for the settings panel, `save` for the dev Save prompt.
    pub fn open_menu(&self) -> Option<&'static str> {
        match &self.popup {
            None => None,
            Some(Popup::Dropdown(category)) => Some(category.name()),
            Some(Popup::Brush) => Some("brush"),
            Some(Popup::Stamps { .. }) => Some("stamps"),
            Some(Popup::Palette) => Some("tools"),
            Some(Popup::Settings { .. }) => Some("map"),
            Some(Popup::File) => Some("file"),
            Some(Popup::Load { .. }) => Some("load"),
            Some(Popup::Save { .. }) => Some("save"),
            Some(Popup::Lint { .. }) => Some("check"),
        }
    }

    // ----- the CHECK panel (docs/large-maps-patterns.md, "Lint panel with
    // jump-to and fixes") -----

    /// Run the linter over the canvas now (`maplint::lint_map`, the dev
    /// server's `lint` tool's own entry), errors first.
    pub fn run_lint(&mut self) {
        let (_, mut findings) = crate::maplint::lint_map(&self.map, &self.lint_setup);
        // A stable sort: each severity keeps the linter's check order.
        findings.sort_by_key(|f| match f.severity {
            LintSeverity::Error => 0,
            LintSeverity::Warning => 1,
            LintSeverity::Info => 2,
        });
        self.lint = Some(LintReport { findings, edits: self.edits });
        self.lint_marked = None;
    }

    /// The CHECK panel's last report, and whether the map has changed
    /// since it ran.
    pub fn lint_report(&self) -> Option<(&LintReport, bool)> {
        self.lint.as_ref().map(|report| (report, report.edits != self.edits))
    }

    /// The finding the canvas marks, if any.
    pub fn lint_marked(&self) -> Option<&LintFinding> {
        self.lint.as_ref()?.findings.get(self.lint_marked?)
    }

    /// Open the CHECK panel on a fresh run of the linter - the CHECK
    /// button.
    pub fn open_lint(&mut self) {
        self.settle();
        self.run_lint();
        self.popup = Some(Popup::Lint { page: 0 });
    }

    /// Pick finding `index` of the panel's report: the canvas marks its
    /// cells and the camera frames them (`frame_cells`), beside the
    /// panel. Whether there was such a finding.
    pub fn pick_finding(&mut self, index: usize, frame: &BuilderFrame) -> bool {
        let Some(cells) = self.lint.as_ref().and_then(|r| r.findings.get(index)).map(|f| f.cells.clone()) else {
            return false;
        };
        self.lint_marked = Some(index);
        self.frame_cells(&cells, frame);
        true
    }

    /// Make finding `index`'s quick fix, as one undo step, then lint the
    /// map it leaves and frame where the fix put the map right. Whether a
    /// fix was made.
    pub fn fix_finding(&mut self, index: usize, frame: &BuilderFrame) -> bool {
        let Some(fix) = self.lint.as_ref().and_then(|r| r.findings.get(index)).and_then(|f| f.fix) else {
            return false;
        };
        if !self.apply_fix(fix) {
            return false;
        }
        self.run_lint();
        let (col, row) = fix.target();
        self.frame_cells(&[LintCell::Map(col, row)], frame);
        true
    }

    /// Pan and zoom the canvas onto `cells` (docs/large-maps-patterns.md):
    /// their bounds, grown to at least `builder_lint_jump_cols` x
    /// `builder_lint_jump_rows` cells about their middle and by a cell all
    /// round, fitted into the part of the canvas the CHECK panel leaves
    /// free and centred there - on the canvas's middle when no panel is
    /// open. Never past the largest zoom, and FIT where the whole field
    /// already shows them that big. A finding with no cells moves nothing.
    pub fn frame_cells(&mut self, cells: &[LintCell], frame: &BuilderFrame) {
        let Some(bounds) = crate::maplint::cells_bounds(cells) else { return };
        let rules = CanvasRules::current();
        let cell = crate::OBSTACLE_GRID_SIZE;
        let (min_w, min_h) = (rules.jump_context_cells.0 * cell, rules.jump_context_cells.1 * cell);
        let middle = Vec2::new(bounds.x + bounds.width / 2.0, bounds.y + bounds.height / 2.0);
        let (w, h) = ((bounds.width + 2.0 * cell).max(min_w), (bounds.height + 2.0 * cell).max(min_h));
        let free = self.free_canvas(frame);
        let layout = &frame.layout;
        let vp = self.viewport_in(layout);
        let scale = (free.width / w).min(free.height / h);
        let area = layout.field;
        // The world point the canvas's middle shows, so `middle` lands on
        // the free part's.
        let shift = Vec2::new(
            (area.x + area.w / 2.0 - (free.x + free.width / 2.0)) / scale,
            (area.y + area.h / 2.0 - (free.y + free.height / 2.0)) / scale,
        );
        self.camera.set(Vec2::new(middle.x + shift.x, middle.y + shift.y), scale, &vp, &rules);
    }

    /// The part of the canvas area the CHECK panel leaves free, in bitmap
    /// pixels: left of it where it hangs over the canvas (a margin's gap
    /// short of it), the whole area when it is shut or stands past the
    /// canvas's right edge.
    pub fn free_canvas(&self, frame: &BuilderFrame) -> Rectangle {
        let f = frame.layout.field;
        let area = Rectangle::new(f.x, f.y, f.w, f.h);
        match (&self.popup, self.chrome(frame).popup) {
            (Some(Popup::Lint { .. }), Some(PopupLayout::Lint(lint))) => {
                let left = frame.ui_to_canvas(Vec2::new(lint.panel.x, lint.panel.y)).x;
                let gap = frame.ui_to_canvas(Vec2::new(lint.panel.x - LINT_FREE_GAP, lint.panel.y)).x;
                if left > area.x && left < area.x + area.width {
                    Rectangle::new(area.x, area.y, (gap - area.x).max(1.0), area.height)
                } else {
                    area
                }
            }
            _ => area,
        }
    }

    /// How many findings the CHECK panel's report holds (none before it
    /// first ran).
    fn lint_len(&self) -> usize {
        self.lint.as_ref().map_or(0, |r| r.findings.len())
    }

    // --- the chrome: one geometry table (`chrome.rs`) ---

    /// The builder's chrome this frame in `frame`'s UI points: the bar and
    /// the open popup laid out for what the builder holds. What the
    /// painter draws, every hit test reads and the dev server reports.
    pub fn chrome(&self, frame: &BuilderFrame) -> Chrome {
        let bar = frame.bar();
        let room = frame.under_bar();
        let popup = self.popup.as_ref().map(|popup| match popup {
            Popup::Dropdown(category) => PopupLayout::Dropdown(*category, chrome::menu_list(bar.tools_anchor(*category), room, category.tools().count())),
            Popup::Brush => PopupLayout::Brush(chrome::menu_list(bar.brush.unwrap_or_else(|| bar.tools_anchor(Category::Wall)), room, BrushRow::ALL.len())),
            Popup::Palette => PopupLayout::Palette(Palette::of(bar.tools_anchor(Category::Wall), room)),
            Popup::Settings { .. } => PopupLayout::Settings(SettingsLayout::of(bar.map, room, SETTINGS_ROWS.len())),
            Popup::File => PopupLayout::File(chrome::menu_list(bar.file, room, FileRow::all().len())),
            Popup::Load { entries, .. } => PopupLayout::Load(LoadLayout::of(room, entries.len())),
            Popup::Stamps { .. } => {
                PopupLayout::Stamps(LoadLayout::sized(room, self.stamps.len(), chrome::STAMPS_PANEL_W, chrome::STAMPS_VISIBLE_ROWS))
            }
            Popup::Save { .. } => PopupLayout::Save(chrome::save_prompt(room)),
            Popup::Lint { .. } => PopupLayout::Lint(LintLayout::of(bar.check, room, self.lint_len())),
        });
        let strip = self.strip_buttons().map(|buttons| Strip::of(room, frame.ui.touch, &buttons));
        Chrome { bar, room, popup, strip }
    }

    /// The builder's buttons a tool presses by name, in UI points - the
    /// rects the hit tests read (the dev server's `status.builder.buttons`
    /// puts them on the window): the bar's (`Bar::named`), the select
    /// tool's strip's (`sel_copy`, `sel_place`, ... - only those that can
    /// act now) and, while a popup is open, its own - a list's or the
    /// palette's `tool_<name>`, BRUSH's list's and the palette's brush
    /// row's `shape_<name>`, `tool_select` and `brush_stamps`, the FILE
    /// menu's `load`, `save`, `save_as` and `clear_map`, the Save prompt's
    /// `save_confirm` (once it holds a name), the Load list's `map_<name>`,
    /// the STAMPS list's `stamp_<key>`, the MAP panel's
    /// `<row>_dec`/`<row>_inc` and `reset`, the CHECK panel's rows
    /// (`finding_N`, from 0 over the whole report) and their FIX buttons
    /// (`fix_N`) - and a pager's halves (`page_back`, `page_next`). A shape
    /// the brush cannot take (a singleton's brush takes the pen alone) is
    /// drawn dim and named nowhere.
    pub fn named_buttons(&self, frame: &BuilderFrame) -> Vec<(String, Rectangle)> {
        let chrome = self.chrome(frame);
        let mut out = chrome.bar.named();
        if let Some(strip) = &chrome.strip {
            out.extend(strip.named());
        }
        let pager = |out: &mut Vec<(String, Rectangle)>, pager: Option<chrome::Pager>| {
            if let Some(p) = pager {
                out.push(("page_back".to_string(), p.back()));
                out.push(("page_next".to_string(), p.next()));
            }
        };
        match (&self.popup, &chrome.popup) {
            (Some(Popup::Dropdown(category)), Some(PopupLayout::Dropdown(_, rows))) => {
                for (i, tool) in category.tools().enumerate() {
                    out.push((format!("tool_{}", tool.name()), rows.row(i)));
                }
            }
            (Some(Popup::Brush), Some(PopupLayout::Brush(rows))) => {
                for (i, row) in BrushRow::ALL.into_iter().enumerate() {
                    if self.brush_row_live(row) {
                        out.push((row.name(), rows.row(i)));
                    }
                }
            }
            (Some(Popup::Palette), Some(PopupLayout::Palette(palette))) => {
                for category in Category::ALL {
                    for (i, tool) in category.tools().enumerate() {
                        out.push((format!("tool_{}", tool.name()), palette.cell(category, i)));
                    }
                }
                for (i, row) in BrushRow::ALL.into_iter().enumerate() {
                    if self.brush_row_live(row) {
                        out.push((row.name(), palette.brush_cell(i)));
                    }
                }
            }
            (Some(Popup::Stamps { scroll }), Some(PopupLayout::Stamps(list))) => {
                let scroll = (*scroll).min(self.stamps.len().saturating_sub(list.per_page));
                for (i, stamp) in self.stamps.iter().skip(scroll).take(list.per_page).enumerate() {
                    out.push((format!("stamp_{}", stamp.key()), list.rows.row(i)));
                }
                pager(&mut out, list.pager);
            }
            (Some(Popup::File), Some(PopupLayout::File(rows))) => {
                for (i, row) in FileRow::all().iter().enumerate() {
                    out.push((row.name().to_string(), rows.row(i)));
                }
            }
            (Some(Popup::Save { name }), Some(PopupLayout::Save(panel))) => {
                if !name.is_empty() {
                    out.push(("save_confirm".to_string(), chrome::save_button(*panel, frame.ui.touch)));
                }
            }
            (Some(Popup::Load { entries, scroll }), Some(PopupLayout::Load(load))) => {
                let scroll = (*scroll).min(entries.len().saturating_sub(load.per_page));
                for (i, entry) in entries.iter().skip(scroll).take(load.per_page).enumerate() {
                    out.push((format!("map_{}", entry.name), load.rows.row(i)));
                }
                pager(&mut out, load.pager);
            }
            (Some(Popup::Settings { page }), Some(PopupLayout::Settings(settings))) => {
                let page = (*page).min(settings.pages - 1);
                for (i, row) in SETTINGS_ROWS.iter().enumerate() {
                    let Some(rect) = settings.row(i, page) else { continue };
                    if *row == SettingsRow::Reset {
                        out.push(("reset".to_string(), Self::settings_reset_rect(rect)));
                    } else {
                        out.push((format!("{}_dec", row.name()), Self::settings_dec_rect(rect)));
                        out.push((format!("{}_inc", row.name()), Self::settings_inc_rect(rect)));
                    }
                }
                pager(&mut out, settings.pager);
            }
            (Some(Popup::Lint { page }), Some(PopupLayout::Lint(lint))) => {
                if let Some(report) = &self.lint {
                    let page = (*page).min(lint.pages - 1);
                    for (i, finding) in report.findings.iter().enumerate().skip(page * lint.per_page).take(lint.per_page) {
                        let row = lint.row(LINT_HEAD_ROWS + i - page * lint.per_page);
                        out.push((format!("finding_{i}"), row));
                        if finding.fix.is_some() {
                            out.push((format!("fix_{i}"), LintLayout::fix(row)));
                        }
                    }
                }
                pager(&mut out, lint.pager);
            }
            _ => {}
        }
        out
    }

    /// A settings row's `<` button.
    fn settings_dec_rect(row: Rectangle) -> Rectangle {
        Rectangle::new(row.x + SETTINGS_DEC_X, row.y, EDITOR_STEPPER_SIZE, EDITOR_STEPPER_SIZE)
    }

    /// A settings row's `>` button.
    fn settings_inc_rect(row: Rectangle) -> Rectangle {
        Rectangle::new(row.x + row.width - SETTINGS_INSET - EDITOR_STEPPER_SIZE, row.y, EDITOR_STEPPER_SIZE, EDITOR_STEPPER_SIZE)
    }

    /// The RESET MAP row's one full-width button.
    fn settings_reset_rect(row: Rectangle) -> Rectangle {
        Rectangle::new(row.x + SETTINGS_INSET, row.y, row.width - 2.0 * SETTINGS_INSET, row.height)
    }

    /// Whether a point in UI points lands on the builder's own chrome (the
    /// bar, the open popup, the select tool's strip, the navigator on its
    /// plate) - a press there never paints the cell behind it and the hover
    /// highlight hides.
    fn point_on_ui(&self, point: Vec2, frame: &BuilderFrame) -> bool {
        let chrome = self.chrome(frame);
        chrome.bar.strip.contains(point)
            || chrome.popup.is_some_and(|popup| popup.panel().contains(point))
            || chrome.strip.as_ref().is_some_and(|strip| strip.panel.contains(point))
            || self.navigator_rect(frame).is_some_and(|r| Corners::plate(r).contains(point))
    }

    /// Whether a row of BRUSH's list can be picked now: a shape the brush
    /// cannot take is not (`shape_allowed`).
    pub fn brush_row_live(&self, row: BrushRow) -> bool {
        match row {
            BrushRow::Shape(shape) => self.shape_allowed(shape),
            BrushRow::Select | BrushRow::Stamps => true,
        }
    }

    /// What a pick of a row of BRUSH's list or the palette's brush row
    /// does: paint with a shape, take the select tool, or open the STAMPS
    /// list.
    fn pick_brush_row(&mut self, row: BrushRow) {
        if !self.brush_row_live(row) {
            return;
        }
        match row {
            BrushRow::Shape(shape) => self.set_shape(shape),
            BrushRow::Select => self.select_tool(Tool::Select),
            BrushRow::Stamps => self.popup = Some(Popup::Stamps { scroll: 0 }),
        }
    }

    // --- input ---

    /// Advance one frame on `input`, whose points are the window's: the
    /// open popup, or a press on the bar, or a stroke on the canvas - each
    /// through `frame`, the chrome's in UI points and the canvas's on its
    /// bitmap. Returns `EditorAction::Play` the frame PLAY is pressed.
    pub fn update(&mut self, input: &BuilderInput, frame: &BuilderFrame) -> EditorAction {
        let layout = &frame.layout;
        if std::mem::take(&mut self.entering) {
            self.gestures = gesture::Gestures::ignoring(input.touches.iter().map(|t| t.id));
        }
        if let Some(p) = input.pointer {
            self.pointer = Some(frame.to_canvas(p));
        }
        if let Some(screen) = input.screen {
            self.screen = screen;
        }
        self.area = (layout.field.w, layout.field.h);
        self.framed = true;
        if let Some(rect) = self.pending_look.take() {
            self.apply_look(rect);
        }
        // A run of size presses is one undo step only while the panel
        // stays open.
        if !matches!(self.popup, Some(Popup::Settings { .. })) {
            self.resize_session = None;
        }
        let rules = CanvasRules::current();
        // The keyboard shortcuts work under a menu too, but not while the
        // Save prompt is taking text - a `-` there is a character.
        if !self.text_entry_open() {
            if input.undo {
                self.undo();
            }
            if input.redo {
                self.redo();
            }
            self.camera_keys(input, &rules);
        }
        // The fingers on the canvas, every frame and before a popup takes
        // the press: a finger that pressed the bar or a popup is never the
        // canvas's, and with fingers down the canvas is theirs.
        let touch = !input.touches.is_empty() || self.gestures.active();
        if touch {
            self.touch_gestures(input, frame, &rules);
        }
        if self.popup.is_some() {
            self.pan_from = None;
            self.nav_drag = false;
            self.update_popup(input, frame);
            return EditorAction::None;
        }
        self.selection_keys(input, frame);
        if self.navigate(input, frame, &rules) {
            return EditorAction::None;
        }

        if input.wheel != 0.0
            && let Some(pointer) = input.pointer
        {
            self.wheel(pointer, input.wheel, frame, &rules);
        }
        if !touch && self.pan_drag(input, frame, &rules) {
            return EditorAction::None;
        }
        self.hover_ghost(input, frame, touch);

        let primary = input.held || input.right_held;
        if !primary {
            // A release ends the stroke or the drag - one undo step per
            // press.
            if !touch {
                self.settle();
            }
            return EditorAction::None;
        }
        let Some(window) = input.pointer else {
            return EditorAction::None;
        };

        // Chrome only reacts to the press edge, never every frame a drag
        // happens to stay over it.
        if input.pressed
            && let Some(button) = frame.bar().hit(frame.to_ui(window))
        {
            self.settle();
            return self.press_bar_button(button);
        }
        if input.pressed
            && let Some(button) = self.chrome(frame).strip.and_then(|strip| strip.hit(frame.to_ui(window)))
        {
            self.settle();
            self.press_strip(button);
            return EditorAction::None;
        }
        if touch {
            return EditorAction::None;
        }

        // The canvas: a press begins a stroke, a held button continues it
        // into every new cell it crosses - the cell under the pointer
        // through the camera. A press on the chrome, or past the map's
        // edge, is not a paint. The select tool selects, carries and
        // pastes instead.
        let pointer = frame.to_canvas(window);
        let cell = self.canvas_cell(pointer, frame);
        if self.active_tool == Tool::Select {
            self.select_pointer(input, cell, pointer, frame);
        } else if let Some(cell) = cell {
            if input.pressed || input.right_pressed {
                self.finish_stroke();
                self.begin_stroke(cell, input.right_held && !input.held);
            } else if self.stroke.is_some() {
                self.drag_to(cell);
            }
        }
        self.edge_scroll(pointer, input.dt, frame, &rules);
        EditorAction::None
    }

    /// The select tool under a mouse, `cell` the map cell under it: a press
    /// puts the paste ghost down there or begins a drag (a press on the
    /// canvas off the map lets the selection go), the right button takes
    /// the ghost away or lets the selection go, and a held button carries
    /// the drag along.
    fn select_pointer(&mut self, input: &BuilderInput, cell: Option<(i32, i32)>, pointer: Vec2, frame: &BuilderFrame) {
        if input.pressed {
            match cell {
                Some(cell) => self.select_press(cell),
                None if self.ghost.is_none() && self.on_canvas(pointer, frame) => self.clear_selection(),
                None => {}
            }
        } else if input.right_pressed {
            if self.ghost.is_some() {
                self.cancel_ghost();
            } else {
                self.clear_selection();
            }
        } else if let Some(cell) = cell
            && self.drag.is_some()
        {
            self.drag_over(cell);
        }
    }

    /// The selection's keys, outside every popup: Escape takes a drag back,
    /// the paste ghost away or the selection off; Ctrl+C, Ctrl+X and Delete
    /// (or Backspace) act on the selection; Ctrl+V pastes the clipboard as
    /// a ghost under the pointer.
    fn selection_keys(&mut self, input: &BuilderInput, frame: &BuilderFrame) {
        if input.escape {
            self.escape();
        }
        if input.copy {
            self.copy_selection();
        }
        if input.cut {
            self.cut_selection();
        }
        if input.paste {
            let near = input.pointer.and_then(|p| self.cell_at(p, frame));
            self.paste(near);
        }
        if (input.delete || input.backspace) && self.active_tool == Tool::Select {
            self.delete_selection();
        }
    }

    /// The paste ghost following a hovering mouse: its middle on the cell
    /// under a pointer that moved with nothing held. A touch screen moves
    /// its pointer only under a finger, so a finger's ghost stays where it
    /// was carried or tapped to.
    fn hover_ghost(&mut self, input: &BuilderInput, frame: &BuilderFrame, touch: bool) {
        let moved = input.pointer != self.hover;
        self.hover = input.pointer;
        if touch || input.held || input.right_held || !moved || self.ghost.is_none() {
            return;
        }
        let Some(cell) = input.pointer.and_then(|p| self.cell_at(p, frame)) else { return };
        let bounds = self.field_cells();
        if let Some(ghost) = &mut self.ghost {
            ghost.centre_on(cell, bounds);
        }
    }

    /// Edge scroll: while a stroke is held with its pointer (on the
    /// canvas's bitmap) within `builder_edge_scroll_pt` of the canvas
    /// area's edge - or past it, over the bar or off the window - the view
    /// moves toward that edge, faster the deeper in, up to
    /// `builder_edge_scroll_pt_per_s`, and the stroke carries on into the
    /// cells that come under the pointer, or, held past the edge, under the
    /// nearest point of the canvas. A long wall needs no pan in the middle;
    /// a select tool's rectangle, its lifted cells and a carried ghost
    /// follow the same way.
    fn edge_scroll(&mut self, pointer: Vec2, dt: f32, frame: &BuilderFrame, rules: &CanvasRules) {
        if !self.dragging() || !(dt > 0.0) {
            return;
        }
        let layout = &frame.layout;
        let vp = self.viewport_in(layout);
        let margin = vp.px(rules.edge_scroll_pt);
        if !(margin > 0.0) {
            return;
        }
        // How far into the margin by each edge, 0 to 1: toward the near
        // edge negative, toward the far one positive.
        let depth = |near: f32, len: f32, p: f32| {
            let into_near = ((near + margin - p) / margin).clamp(0.0, 1.0);
            let into_far = ((p - (near + len - margin)) / margin).clamp(0.0, 1.0);
            into_far - into_near
        };
        let f = layout.field;
        let into = Vec2::new(depth(f.x, f.w, pointer.x), depth(f.y, f.h, pointer.y));
        if into.x == 0.0 && into.y == 0.0 {
            return;
        }
        let step = vp.px(rules.edge_scroll_pt_per_s) * dt;
        self.camera.pan(Vec2::new(-into.x * step, -into.y * step), &vp, rules);
        let on_canvas = Vec2::new(pointer.x.clamp(f.x, f.x + f.w - 1.0), pointer.y.clamp(f.y, f.y + f.h - 1.0));
        if let Some(cell) = self.canvas_cell(on_canvas, frame) {
            self.drag_over(cell);
        }
    }

    /// `+`/`-` zoom about the canvas's middle and the arrows pan it, the
    /// view moving the way an arrow points.
    fn camera_keys(&mut self, input: &BuilderInput, rules: &CanvasRules) {
        let vp = self.viewport();
        let middle = Vec2::new(vp.area.0 / 2.0, vp.area.1 / 2.0);
        if input.zoom_in {
            self.camera.step(true, middle, &vp, rules);
        }
        if input.zoom_out {
            self.camera.step(false, middle, &vp, rules);
        }
        let keys = input.pan_keys;
        if keys.x != 0.0 || keys.y != 0.0 {
            let speed = vp.px(rules.key_pan_pt_per_s) * input.dt.max(0.0);
            self.camera.pan(Vec2::new(-keys.x * speed, -keys.y * speed), &vp, rules);
        }
    }

    /// The wheel at a window point: over a category button it steps that
    /// category's tool (towards you walks down the list), over the canvas
    /// it zooms at the cursor - a whole-block step a notch on a coarse
    /// screen, smoothly on a fine one.
    fn wheel(&mut self, window: Vec2, wheel: f32, frame: &BuilderFrame, rules: &CanvasRules) {
        if let Some(category) = frame.bar().category_at(frame.to_ui(window)) {
            self.cycle_tool(category, wheel < 0.0);
            return;
        }
        let pointer = frame.to_canvas(window);
        if !self.on_canvas(pointer, frame) {
            return;
        }
        let layout = &frame.layout;
        let vp = self.viewport_in(layout);
        let at = layout.to_field(pointer);
        if vp.screen.coarse {
            // A trackpad sends fractions of a notch: whole steps only, and
            // a turn the other way starts afresh.
            if self.wheel_accum * wheel < 0.0 {
                self.wheel_accum = 0.0;
            }
            self.wheel_accum += wheel;
            while self.wheel_accum >= 1.0 {
                self.camera.step(true, at, &vp, rules);
                self.wheel_accum -= 1.0;
            }
            while self.wheel_accum <= -1.0 {
                self.camera.step(false, at, &vp, rules);
                self.wheel_accum += 1.0;
            }
        } else {
            let scale = self.camera.scale(&vp) * rules.zoom_step.max(1.01).powf(wheel);
            self.camera.zoom_at(scale, at, &vp, rules);
        }
    }

    /// One frame of fingers: the canvas's go through the gestures
    /// (`gesture.rs`), on the canvas's bitmap, and what they mean is done.
    /// Only a finger that lands on the canvas while no popup is open, and
    /// that no press of the chrome takes (`lands_on_canvas`), is the
    /// canvas's.
    fn touch_gestures(&mut self, input: &BuilderInput, frame: &BuilderFrame, rules: &CanvasRules) {
        let vp = self.viewport_in(&frame.layout);
        let touches: Vec<crate::touch::TouchPoint> =
            input.touches.iter().map(|t| crate::touch::TouchPoint { id: t.id, pos: frame.to_canvas(t.pos) }).collect();
        let canvas: Vec<i32> = if self.popup.is_some() {
            Vec::new()
        } else {
            input.touches.iter().filter(|t| self.lands_on_canvas(t.pos, frame)).map(|t| t.id).collect()
        };
        let paints = self.touch_paints(&vp, rules);
        let g = gesture::GestureRules { slop: vp.px(rules.slop_pt), tap_seconds: rules.tap_seconds };
        let events = self.gestures.update(&touches, |t| canvas.contains(&t.id), paints, input.dt, &g);
        for event in events {
            self.apply_gesture(event, frame, rules);
        }
        // A painting finger held at the canvas's edge scrolls it.
        if self.gestures.painting()
            && let Some(at) = self.stroke_pointer
        {
            self.edge_scroll(at, input.dt, frame, rules);
        }
    }

    /// The paint threshold: whether a finger can hit one cell at this
    /// zoom - a cell at least `builder_paint_min_cell_mm` on the glass.
    /// Under it a tap zooms in and a drag pans.
    fn touch_paints(&self, vp: &Viewport, rules: &CanvasRules) -> bool {
        vp.cell_mm(self.camera.scale(vp)) >= rules.paint_min_cell_mm
    }

    /// The loupe over `frame`'s canvas this frame: while one finger
    /// paints a stroke, or draws or carries a selection
    /// (`gesture::Gestures::painting` - a mouse never does), where a cell
    /// is drawn under `builder_loupe_cell_mm` on the glass. It shows the
    /// cell under the finger's drag (`finger_cell`) and what is round it,
    /// `builder_loupe_zoom` times larger on the nearest whole-block scale,
    /// in a square of about `LOUPE_PT` UI points a side standing
    /// `builder_loupe_lift_pt` clear of the finger (`loupe_rect`), on the
    /// canvas and inside the safe area - of the canvas point the stroke
    /// paints under, which held past the canvas's edge is the nearest point
    /// of it, as edge scroll paints. Its side is whole 2 px blocks of the
    /// world and its corner a whole device pixel, so no block in it is cut
    /// or uneven.
    pub fn loupe(&self, frame: &BuilderFrame) -> Option<Loupe> {
        let (cell, erase) = self.finger_cell().filter(|_| self.gestures.painting())?;
        let finger = self.stroke_pointer?;
        let layout = &frame.layout;
        let vp = self.viewport_in(layout);
        let scale = self.camera.scale(&vp);
        let rules = CanvasRules::current();
        if !(vp.cell_mm(scale) < rules.loupe_cell_mm) {
            return None;
        }
        let f = layout.field;
        let on_canvas = Vec2::new(finger.x.clamp(f.x, f.x + f.w - 1.0), finger.y.clamp(f.y, f.y + f.h - 1.0));
        let finger = frame.canvas_to_ui(on_canvas);
        let area = intersect(frame.canvas_ui(), {
            let a = frame.ui.area;
            Rectangle::new(a.x, a.y, a.w, a.h)
        });
        // Device pixels per UI point: the loupe's corner and side are whole
        // ones.
        let device = frame.device_per_point(vp.screen.device_per_px);
        let device_scale = camera::nearest_whole_block((vp.device_scale(scale) * rules.loupe_zoom).max(camera::MIN_WHOLE_BLOCK_SCALE));
        let block = crate::pyro::BLOCK;
        let blocks = (LOUPE_PT * device / (device_scale * block)).floor().max(1.0);
        let world_side = blocks * block;
        let side = world_side * device_scale / device;
        let rect = loupe_rect(finger, side, rules.loupe_lift_pt, area);
        let snap = |v: f32, lo: f32, hi: f32| {
            let lo = (lo * device).ceil();
            let hi = ((hi * device).floor()).max(lo);
            (v * device).round().clamp(lo, hi) / device
        };
        let rect = Rectangle::new(
            snap(rect.x, area.x, area.x + area.width - side),
            snap(rect.y, area.y, area.y + area.height - side),
            side,
            side,
        );
        // The world round the stroke's cell, its corner on the block grid.
        let middle = map::cell_to_world(cell.0, cell.1);
        let corner = Vec2::new(
            ((middle.x - world_side / 2.0) / block).round() * block,
            ((middle.y - world_side / 2.0) / block).round() * block,
        );
        Some(Loupe { rect, world: Rectangle::new(corner.x, corner.y, world_side, world_side), device_scale, cell, erase })
    }

    /// The cell under a finger's drag and whether it erases: the cell a
    /// stroke paints last, the corner a select tool's rectangle stretches
    /// to, the cell the press of a carry has reached. What the loupe
    /// outlines.
    fn finger_cell(&self) -> Option<((i32, i32), bool)> {
        if let Some(stroke) = &self.stroke {
            return Some((stroke.last_cell, stroke.erase));
        }
        match self.drag {
            Some(Drag::Select { to, .. }) => Some((to, false)),
            Some(Drag::Move { grab, by, .. }) => Some(((grab.0 + by.0, grab.1 + by.1), false)),
            Some(Drag::Ghost { .. }) | None => None,
        }
    }

    /// Do what a gesture means - its points on the canvas's bitmap: a tap
    /// paints its cell (or zooms in on it under the paint threshold), a
    /// stroke paints the cells it crosses - begun at the first cell on the
    /// field it reaches - and a move, a settle, an undo or a redo is the
    /// camera's or the history's.
    fn apply_gesture(&mut self, event: gesture::GestureEvent, frame: &BuilderFrame, rules: &CanvasRules) {
        use gesture::GestureEvent;
        let layout = &frame.layout;
        let vp = self.viewport_in(layout);
        match event {
            GestureEvent::Tap(at) => {
                self.pointer = Some(at);
                if self.touch_paints(&vp, rules) {
                    match self.canvas_cell(at, frame) {
                        Some(cell) if self.active_tool == Tool::Select => self.tap_select(cell),
                        Some(cell) => {
                            self.finish_stroke();
                            self.begin_stroke(cell, false);
                            self.finish_stroke();
                        }
                        // A tap on the canvas off the map is outside every
                        // selection.
                        None if self.active_tool == Tool::Select && self.ghost.is_none() && self.on_canvas(at, frame) => {
                            self.clear_selection()
                        }
                        None => {}
                    }
                } else if self.on_canvas(at, frame) {
                    self.camera.zoom_for_tap(layout.to_field(at), &vp, rules);
                }
            }
            GestureEvent::StrokeBegin(from) => {
                self.settle();
                self.stroke_pointer = Some(from);
                if let Some(cell) = self.canvas_cell(from, frame) {
                    self.finger_down(cell);
                }
            }
            GestureEvent::StrokeTo(to) => {
                self.pointer = Some(to);
                self.stroke_pointer = Some(to);
                if let Some(cell) = self.canvas_cell(to, frame) {
                    if self.dragging() {
                        self.drag_over(cell);
                    } else {
                        self.finger_down(cell);
                    }
                }
            }
            GestureEvent::StrokeEnd => {
                self.stroke_pointer = None;
                self.settle();
            }
            GestureEvent::StrokeCancel => {
                self.stroke_pointer = None;
                self.cancel_stroke();
                self.cancel_drag();
            }
            GestureEvent::Move { from, to, factor } => self.camera.move_point(layout.to_field(from), layout.to_field(to), factor, &vp, rules),
            GestureEvent::Settle(at) => self.camera.settle(layout.to_field(at), &vp, rules),
            GestureEvent::Undo => {
                self.undo();
            }
            GestureEvent::Redo => {
                self.redo();
            }
        }
    }

    /// A finger's drag reaching the field at `cell`: the select tool's drag
    /// there, or a stroke from it.
    fn finger_down(&mut self, cell: (i32, i32)) {
        if self.active_tool == Tool::Select {
            self.begin_select_drag(cell, true);
        } else {
            self.begin_stroke(cell, false);
        }
    }

    /// A pan drag - the middle button, or Space with the primary - moves
    /// the canvas with the pointer. It starts only on the canvas (a press
    /// on the bar with Space held is still a press on the bar) and ends any
    /// stroke under way. Whether this frame was one.
    fn pan_drag(&mut self, input: &BuilderInput, frame: &BuilderFrame, rules: &CanvasRules) -> bool {
        let dragging = input.middle_held || (input.space_held && input.held);
        let Some(pointer) = input.pointer.filter(|_| dragging).map(|p| frame.to_canvas(p)) else {
            self.pan_from = None;
            return false;
        };
        let from = match self.pan_from {
            Some(from) => from,
            None if self.on_canvas(pointer, frame) => {
                self.settle();
                pointer
            }
            None => return false,
        };
        let vp = self.viewport_in(&frame.layout);
        self.camera.pan(Vec2::new(pointer.x - from.x, pointer.y - from.y), &vp, rules);
        self.pan_from = Some(pointer);
        true
    }

    /// What a press on a bar button does.
    fn press_bar_button(&mut self, button: BarButton) -> EditorAction {
        match button {
            BarButton::Play => return EditorAction::Play,
            BarButton::PlayHere => return EditorAction::PlayHere,
            BarButton::File => self.popup = Some(Popup::File),
            BarButton::CategoryIcon(category) => self.select_tool(self.current_tool(category)),
            BarButton::CategoryMenu(category) => self.popup = Some(Popup::Dropdown(category)),
            BarButton::Tools => self.popup = Some(Popup::Palette),
            BarButton::Brush => self.popup = Some(Popup::Brush),
            BarButton::Erase => self.select_tool(Tool::Eraser),
            BarButton::Undo => {
                self.undo();
            }
            BarButton::Redo => {
                self.redo();
            }
            BarButton::Map => self.popup = Some(Popup::Settings { page: 0 }),
            BarButton::Fit => self.camera.fit(),
            BarButton::Check => self.open_lint(),
        }
        EditorAction::None
    }

    /// Handle input while a popup is open, consuming it entirely: a
    /// press inside the popup works it, a press anywhere else closes it
    /// and does nothing more, `Esc` closes it. The popup's rows are the
    /// chrome's (`chrome`), hit in UI points.
    fn update_popup(&mut self, input: &BuilderInput, frame: &BuilderFrame) {
        let layout = self.chrome(frame).popup;
        let Some(popup) = self.popup.take() else { return };
        if input.escape {
            return;
        }
        let pressed = input.pressed || input.right_pressed;
        let pointer = input.pointer.map(|p| frame.to_ui(p));
        match (popup, layout) {
            (Popup::Save { mut name }, layout) => {
                let press = pointer.filter(|_| pressed);
                let panel = match layout {
                    Some(PopupLayout::Save(panel)) => Some(panel),
                    _ => None,
                };
                // A press outside the prompt cancels it, as one outside any
                // popup closes it: a touch screen's Esc.
                if let (Some(p), Some(panel)) = (press, panel)
                    && !panel.contains(p)
                {
                    return;
                }
                for c in input.typed.chars() {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        name.push(c);
                    }
                }
                if input.backspace {
                    name.pop();
                }
                // Enter, or a press on SAVE: a touch screen's Enter.
                let tapped = input.pressed && press.zip(panel).is_some_and(|(p, panel)| chrome::save_button(panel, frame.ui.touch).contains(p));
                if (input.enter || tapped) && !name.is_empty() {
                    if let Err(e) = self.save(Some(&name)) {
                        self.status = Some(e);
                    }
                } else {
                    self.popup = Some(Popup::Save { name });
                }
            }
            (Popup::File, Some(PopupLayout::File(rows))) => {
                let Some(pointer) = pointer.filter(|_| pressed) else {
                    self.popup = Some(Popup::File);
                    return;
                };
                // A pick or a press elsewhere: the menu closes either way.
                if input.pressed {
                    let picked = FileRow::all().iter().enumerate().find(|&(i, _)| rows.row(i).contains(pointer)).map(|(_, row)| *row);
                    match picked {
                        Some(FileRow::Load) => self.open_load_list(),
                        Some(FileRow::Save) if self.map.name.is_some() => {
                            if let Err(e) = self.save(None) {
                                self.status = Some(e);
                            }
                        }
                        Some(FileRow::Save) | Some(FileRow::SaveAs) => {
                            self.popup = Some(Popup::Save { name: self.map.name.clone().unwrap_or_default() })
                        }
                        Some(FileRow::Clear) => self.clear(),
                        None => {}
                    }
                }
            }
            (Popup::Load { entries, mut scroll }, Some(PopupLayout::Load(load))) => {
                let rows = load.per_page;
                let max = entries.len().saturating_sub(rows);
                scroll = scroll.min(max);
                if input.wheel != 0.0 {
                    scroll = if input.wheel < 0.0 { (scroll + 1).min(max) } else { scroll.saturating_sub(1) };
                }
                // The page's thumbnails, a picture a frame
                // (`thumbs::MADE_PER_FRAME`).
                self.thumbs.update(entries.iter().skip(scroll).take(rows).map(|e| e.name.as_str()), map::map_source);
                let Some(pointer) = pointer.filter(|_| pressed) else {
                    self.popup = Some(Popup::Load { entries, scroll });
                    return;
                };
                if !load.rows.panel.contains(pointer) {
                    return;
                }
                if input.pressed {
                    // The pager: its left half pages back, its right half on.
                    if let Some(pager) = load.pager.filter(|p| p.row.contains(pointer)) {
                        scroll = if pager.back().contains(pointer) { scroll.saturating_sub(rows) } else { (scroll + rows).min(max) };
                        self.popup = Some(Popup::Load { entries, scroll });
                        return;
                    }
                    let picked = entries
                        .iter()
                        .skip(scroll)
                        .take(rows)
                        .enumerate()
                        .find(|&(i, _)| load.rows.row(i).contains(pointer))
                        .map(|(_, e)| e.name.clone());
                    if let Some(name) = picked {
                        if let Err(e) = self.load_named(&name) {
                            self.status = Some(e);
                        }
                        return;
                    }
                }
                self.popup = Some(Popup::Load { entries, scroll });
            }
            (Popup::Lint { page }, Some(PopupLayout::Lint(lint))) => {
                // An edit since the last run - an undo key, a fix, a tool's
                // stroke - is linted again before the panel reads it, and
                // the panel laid out for what that run found.
                let lint = if self.lint_report().is_none_or(|(_, stale)| stale) {
                    self.run_lint();
                    LintLayout::of(frame.bar().check, frame.under_bar(), self.lint_len())
                } else {
                    lint
                };
                let len = self.lint_len();
                let last = lint.pages - 1;
                let mut page = page.min(last);
                if input.wheel != 0.0 {
                    page = if input.wheel < 0.0 { (page + 1).min(last) } else { page.saturating_sub(1) };
                }
                self.popup = Some(Popup::Lint { page });
                let Some(pointer) = pointer.filter(|_| pressed) else { return };
                if !lint.panel.contains(pointer) {
                    // A press anywhere else closes the panel; the marks it
                    // made stay until the next edit.
                    self.popup = None;
                    return;
                }
                if !input.pressed {
                    return;
                }
                if let Some(pager) = lint.pager.filter(|p| p.row.contains(pointer)) {
                    page = if pager.back().contains(pointer) { page.saturating_sub(1) } else { (page + 1).min(last) };
                    self.popup = Some(Popup::Lint { page });
                    return;
                }
                let on_page = (page * lint.per_page..len).take(lint.per_page);
                for (slot, index) in on_page.enumerate() {
                    let row = lint.row(LINT_HEAD_ROWS + slot);
                    if !row.contains(pointer) {
                        continue;
                    }
                    let has_fix = self.lint.as_ref().is_some_and(|r| r.findings[index].fix.is_some());
                    if has_fix && LintLayout::fix(row).contains(pointer) {
                        self.fix_finding(index, frame);
                    } else {
                        self.pick_finding(index, frame);
                    }
                    break;
                }
            }
            (Popup::Dropdown(category), Some(PopupLayout::Dropdown(_, rows))) => {
                let Some(pointer) = pointer.filter(|_| pressed) else {
                    self.popup = Some(Popup::Dropdown(category));
                    return;
                };
                // A pick or a press elsewhere: either way the list closes
                // and the press goes no further.
                if input.pressed {
                    let picked = category.tools().enumerate().find(|&(i, _)| rows.row(i).contains(pointer)).map(|(_, tool)| tool);
                    if let Some(tool) = picked {
                        self.select_tool(tool);
                    }
                }
            }
            (Popup::Palette, Some(PopupLayout::Palette(palette))) => {
                let Some(pointer) = pointer.filter(|_| pressed) else {
                    self.popup = Some(Popup::Palette);
                    return;
                };
                // A pick or a press elsewhere: the palette closes either way
                // (the brush row's STAMPS opens its list in its place).
                if input.pressed {
                    let picked = Category::ALL
                        .into_iter()
                        .flat_map(|category| category.tools().enumerate().map(move |(i, tool)| (category, i, tool)))
                        .find(|&(category, i, _)| palette.cell(category, i).contains(pointer))
                        .map(|(_, _, tool)| tool);
                    if let Some(tool) = picked {
                        self.select_tool(tool);
                    }
                    let row = BrushRow::ALL.into_iter().enumerate().find(|&(i, _)| palette.brush_cell(i).contains(pointer));
                    if let Some((_, row)) = row {
                        self.pick_brush_row(row);
                    }
                }
            }
            (Popup::Brush, Some(PopupLayout::Brush(rows))) => {
                let Some(pointer) = pointer.filter(|_| pressed) else {
                    self.popup = Some(Popup::Brush);
                    return;
                };
                // A pick or a press elsewhere: the list closes either way
                // (STAMPS opens its list in its place).
                if input.pressed {
                    let row = BrushRow::ALL.into_iter().enumerate().find(|&(i, _)| rows.row(i).contains(pointer));
                    if let Some((_, row)) = row {
                        self.pick_brush_row(row);
                    }
                }
            }
            (Popup::Stamps { mut scroll }, Some(PopupLayout::Stamps(list))) => {
                let rows = list.per_page;
                let max = self.stamps.len().saturating_sub(rows);
                scroll = scroll.min(max);
                if input.wheel != 0.0 {
                    scroll = if input.wheel < 0.0 { (scroll + 1).min(max) } else { scroll.saturating_sub(1) };
                }
                let Some(pointer) = pointer.filter(|_| pressed) else {
                    self.popup = Some(Popup::Stamps { scroll });
                    return;
                };
                if !list.rows.panel.contains(pointer) {
                    return;
                }
                if input.pressed {
                    if let Some(pager) = list.pager.filter(|p| p.row.contains(pointer)) {
                        scroll = if pager.back().contains(pointer) { scroll.saturating_sub(rows) } else { (scroll + rows).min(max) };
                        self.popup = Some(Popup::Stamps { scroll });
                        return;
                    }
                    let picked = (scroll..self.stamps.len()).take(rows).enumerate().find(|&(i, _)| list.rows.row(i).contains(pointer));
                    if let Some((_, index)) = picked {
                        self.use_stamp(index);
                        return;
                    }
                }
                self.popup = Some(Popup::Stamps { scroll });
            }
            (Popup::Settings { page }, Some(PopupLayout::Settings(settings))) => {
                let last = settings.pages - 1;
                let mut page = page.min(last);
                if input.wheel != 0.0 {
                    page = if input.wheel < 0.0 { (page + 1).min(last) } else { page.saturating_sub(1) };
                }
                let Some(pointer) = pointer.filter(|_| pressed) else {
                    self.popup = Some(Popup::Settings { page });
                    return;
                };
                if !settings.rows.panel.contains(pointer) {
                    return;
                }
                if input.pressed {
                    if let Some(pager) = settings.pager.filter(|p| p.row.contains(pointer)) {
                        page = if pager.back().contains(pointer) { page.saturating_sub(1) } else { (page + 1).min(last) };
                    } else {
                        for (i, row) in SETTINGS_ROWS.iter().enumerate() {
                            let Some(rect) = settings.row(i, page).filter(|r| r.contains(pointer)) else { continue };
                            if *row == SettingsRow::Reset {
                                if Self::settings_reset_rect(rect).contains(pointer) {
                                    self.reset();
                                }
                            } else if Self::settings_dec_rect(rect).contains(pointer) {
                                self.step_setting(*row, false);
                            } else if Self::settings_inc_rect(rect).contains(pointer) {
                                self.step_setting(*row, true);
                            }
                        }
                    }
                }
                self.popup = Some(Popup::Settings { page });
            }
            // A layout always comes with its popup; a mismatch closes it.
            _ => {}
        }
    }

    /// Step one settings row's value and record it as an undo step:
    /// numbers walk `auto`, then their range without wrapping; choices
    /// cycle `auto` and their list.
    fn step_setting(&mut self, row: SettingsRow, forward: bool) {
        let mut s = self.settings();
        let cap = crate::tuning::tuning().wave_max_alive as u32;
        match row {
            SettingsRow::Tanks => s.tanks = step_option_number(s.tanks, forward, 0, cap),
            SettingsRow::Tank => s.tank = step_option_choice(s.tank, &TankKind::ALL, forward),
            SettingsRow::Tank2 => s.tank2 = step_option_choice(s.tank2, &TankKind::ALL, forward),
            SettingsRow::Mission => s.mission = step_choice(s.mission, &MISSIONS, forward),
            SettingsRow::Spawn => s.spawn = step_choice(s.spawn, &[SpawnKind::Band, SpawnKind::Waves], forward),
            SettingsRow::Waves => s.waves = step_option_number(s.waves, forward, 1, 20),
            SettingsRow::Size => s.size = step_option_number(s.size, forward, 1, 31),
            SettingsRow::Growth => s.growth = step_option_number(s.growth, forward, 0, 10),
            SettingsRow::TierStart => s.tier_start = step_option_choice(s.tier_start, &Tier::ALL, forward),
            SettingsRow::TierEnd => s.tier_end = step_option_choice(s.tier_end, &Tier::ALL, forward),
            SettingsRow::Theme => s.theme = step_choice(s.theme, &Theme::ALL, forward),
            SettingsRow::Weather => s.weather = step_choice(s.weather, &Weather::ALL, forward),
            SettingsRow::Width | SettingsRow::Height => {
                let (cols, rows) = self.size_cells();
                let (cols, rows) =
                    if row == SettingsRow::Width { (step_size(cols, forward), rows) } else { (cols, step_size(rows, forward)) };
                self.resize_map(cols, rows, self.resize_anchor, true);
                return;
            }
            SettingsRow::Anchor => {
                self.resize_anchor = step_choice(self.resize_anchor, &Anchor::ALL, forward);
                return;
            }
            SettingsRow::Reset => return,
        }
        self.apply_settings(s);
    }

    // --- drawing ---

    /// Whether the singleton `tool` already has its object on the map -
    /// the badge on its icon, read from the kept index.
    pub fn singleton_placed(&self, tool: Tool) -> bool {
        let index = self.cell_index();
        match tool {
            Tool::Frog => index.frog().is_some(),
            Tool::Start => index.start().is_some(),
            Tool::Start2 => index.start2().is_some(),
            Tool::EnemyFrog => index.enemy_frog().is_some(),
            _ => false,
        }
    }

}

/// The MAP panel's rows, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsRow {
    Tanks,
    Tank,
    /// Player 2's chassis (`MapFile::tank2`), read only in a two-player round.
    Tank2,
    Mission,
    Spawn,
    Waves,
    Size,
    Growth,
    TierStart,
    TierEnd,
    /// The look (`MapFile::theme`): the canvas redraws in it at once.
    Theme,
    /// The sky (`MapFile::weather`, docs/weather.md). The canvas stays
    /// clear to edit on; the round draws the sky.
    Weather,
    /// The map's size in cells (`MapFile::size`): columns and rows, each
    /// press one cell, the old map placed by the ANCHOR row
    /// (`MapEditor::resize`).
    Width,
    Height,
    /// Where the old map sits when the size changes: one of nine.
    Anchor,
    Reset,
}

impl SettingsRow {
    /// The row as `status.builder.buttons` names its steppers
    /// (`tanks_dec`, `tanks_inc`).
    fn name(self) -> &'static str {
        match self {
            SettingsRow::Tanks => "tanks",
            SettingsRow::Tank => "tank",
            SettingsRow::Tank2 => "tank2",
            SettingsRow::Mission => "mission",
            SettingsRow::Spawn => "spawn",
            SettingsRow::Waves => "waves",
            SettingsRow::Size => "size",
            SettingsRow::Growth => "growth",
            SettingsRow::TierStart => "tier_start",
            SettingsRow::TierEnd => "tier_end",
            SettingsRow::Theme => "theme",
            SettingsRow::Weather => "weather",
            SettingsRow::Width => "width",
            SettingsRow::Height => "height",
            SettingsRow::Anchor => "anchor",
            SettingsRow::Reset => "reset",
        }
    }
}

const SETTINGS_ROWS: [SettingsRow; 16] = [
    SettingsRow::Tanks,
    SettingsRow::Tank,
    SettingsRow::Tank2,
    SettingsRow::Mission,
    SettingsRow::Spawn,
    SettingsRow::Waves,
    SettingsRow::Size,
    SettingsRow::Growth,
    SettingsRow::TierStart,
    SettingsRow::TierEnd,
    SettingsRow::Theme,
    SettingsRow::Weather,
    SettingsRow::Width,
    SettingsRow::Height,
    SettingsRow::Anchor,
    SettingsRow::Reset,
];

const MISSIONS: [Mission; 3] = [Mission::Protect, Mission::Hunt, Mission::Destroy];

/// The smallest map the size steppers make, in cells: a phone's view of
/// a field map is about 35 x 16, and nothing smaller than this is worth a
/// round.
pub const MIN_MAP_CELLS: (f32, f32) = (16.0, 9.0);

/// Where the old map sits in a resized one (`MapEditor::resize`): the
/// MAP panel's nine-way ANCHOR. An anchor on a side keeps that edge where
/// it was; the middle shares the change between the two sides.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    #[default]
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    /// Row by row, as the panel's 3 x 3 glyph shows them.
    pub const ALL: [Anchor; 9] = [
        Anchor::TopLeft,
        Anchor::Top,
        Anchor::TopRight,
        Anchor::Left,
        Anchor::Center,
        Anchor::Right,
        Anchor::BottomLeft,
        Anchor::Bottom,
        Anchor::BottomRight,
    ];

    /// The anchor's name, as the dev server spells it.
    pub fn name(self) -> &'static str {
        match self {
            Anchor::TopLeft => "top_left",
            Anchor::Top => "top",
            Anchor::TopRight => "top_right",
            Anchor::Left => "left",
            Anchor::Center => "center",
            Anchor::Right => "right",
            Anchor::BottomLeft => "bottom_left",
            Anchor::Bottom => "bottom",
            Anchor::BottomRight => "bottom_right",
        }
    }

    pub fn parse(s: &str) -> Option<Anchor> {
        Anchor::ALL.into_iter().find(|a| a.name() == s)
    }

    /// The anchor's column and row in its 3 x 3 grid, 0 to 2.
    pub fn grid(self) -> (i32, i32) {
        let i = Anchor::ALL.iter().position(|&a| a == self).expect("every anchor is in ALL") as i32;
        (i % 3, i / 3)
    }

    /// How many whole cells the old map's cells move when the map goes
    /// from `old` to `new` cells (columns, rows): none at a near anchor,
    /// the whole change at a far one, half of it in the middle - counted
    /// from the halves of both sizes, so a run of one-cell steps moves the
    /// map a cell every other step and comes out centred.
    pub fn shift(self, old: (f32, f32), new: (f32, f32)) -> (i32, i32) {
        let (gx, gy) = self.grid();
        let axis = |g: i32, o: f32, n: f32| match g {
            0 => 0,
            1 => (n / 2.0).floor() as i32 - (o / 2.0).floor() as i32,
            _ => n.floor() as i32 - o.floor() as i32,
        };
        (axis(gx, old.0, new.0), axis(gy, old.1, new.1))
    }
}

/// One press of a size stepper: to the next whole number of cells either
/// way (a fractional size steps onto the whole ones).
fn step_size(cells: f32, forward: bool) -> f32 {
    if forward { cells.floor() + 1.0 } else { cells.ceil() - 1.0 }
}

/// Walk an optional number along `auto, min, min+1, .., max`: forwards
/// from `auto` lands on `min`, backwards from `min` on `auto`, and the
/// ends hold rather than wrap.
fn step_option_number(value: Option<u32>, forward: bool, min: u32, max: u32) -> Option<u32> {
    match (value, forward) {
        (None, true) => Some(min),
        (None, false) => None,
        (Some(n), true) => Some((n + 1).min(max)),
        (Some(n), false) => (n > min).then(|| n - 1),
    }
}

/// Cycle an optional choice through `auto` then `list`, wrapping.
fn step_option_choice<T: Copy + PartialEq>(value: Option<T>, list: &[T], forward: bool) -> Option<T> {
    let len = list.len() + 1;
    let at = value.and_then(|v| list.iter().position(|&x| x == v)).map_or(0, |i| i + 1);
    let next = if forward { (at + 1) % len } else { (at + len - 1) % len };
    if next == 0 {
        None
    } else {
        Some(list[next - 1])
    }
}

/// Cycle a choice through `list`, wrapping.
fn step_choice<T: Copy + PartialEq>(value: T, list: &[T], forward: bool) -> T {
    let at = list.iter().position(|&x| x == value).unwrap_or(0);
    let len = list.len();
    list[if forward { (at + 1) % len } else { (at + len - 1) % len }]
}

/// The overlap of two rectangles; where they do not overlap, an empty
/// rectangle at the nearest corner of `a`.
fn intersect(a: Rectangle, b: Rectangle) -> Rectangle {
    let (x0, y0) = (a.x.max(b.x), a.y.max(b.y));
    let (x1, y1) = ((a.x + a.width).min(b.x + b.width), (a.y + a.height).min(b.y + b.height));
    Rectangle::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// The inward direction of a gate placed at world position `pos`, or
/// `None` when that position is not on a nav-grid edge cell (col 0, the
/// last col, row 0 or the last row of the `PATHFIND_CELL_SIZE` grid a
/// `width` x `height` battlefield gets - the same cell arithmetic as
/// `pathfind::Grid::build`). A corner reports its horizontal edge.
pub fn gate_inward(pos: Position, width: f32, height: f32) -> Option<Position> {
    let cols = ((width / PATHFIND_CELL_SIZE).ceil() as i32).max(1);
    let rows = ((height / PATHFIND_CELL_SIZE).ceil() as i32).max(1);
    let col = ((pos.x / PATHFIND_CELL_SIZE) as i32).clamp(0, cols - 1);
    let row = ((pos.y / PATHFIND_CELL_SIZE) as i32).clamp(0, rows - 1);
    if col == 0 {
        Some(Position::new(1.0, 0.0))
    } else if col == cols - 1 {
        Some(Position::new(-1.0, 0.0))
    } else if row == 0 {
        Some(Position::new(0.0, 1.0))
    } else if row == rows - 1 {
        Some(Position::new(0.0, -1.0))
    } else {
        None
    }
}

/// The spellings the settings tools accept.
pub fn parse_mission(s: &str) -> Option<Mission> {
    match s {
        "protect" => Some(Mission::Protect),
        "hunt" => Some(Mission::Hunt),
        "destroy" => Some(Mission::Destroy),
        _ => None,
    }
}

pub fn parse_spawn(s: &str) -> Option<SpawnKind> {
    match s {
        "band" => Some(SpawnKind::Band),
        "waves" => Some(SpawnKind::Waves),
        _ => None,
    }
}

pub fn parse_tier(s: &str) -> Option<Tier> {
    match s {
        "light" => Some(Tier::Light),
        "medium" => Some(Tier::Medium),
        "heavy" => Some(Tier::Heavy),
        "super" => Some(Tier::Super),
        _ => None,
    }
}

pub fn parse_tank(s: &str) -> Option<TankKind> {
    TankKind::ALL.iter().copied().find(|k| k.name() == s)
}

#[cfg(test)]
mod editor_tests {
    use super::*;
    use crate::framing::MapClass;
    use crate::hud::{Insets, UiFrame};
    use crate::{DEFAULT_SCREEN_HEIGHT, DEFAULT_SCREEN_WIDTH};

    const W: f32 = DEFAULT_SCREEN_WIDTH as f32;
    const H: f32 = DEFAULT_SCREEN_HEIGHT as f32;

    fn brick() -> CellObject {
        CellObject::Wall { material: Material::Brick }
    }

    /// Save writes the loaded `MapFile` as-is, so a map's level tables and
    /// the two level cell kinds survive a builder session untouched.
    #[test]
    fn save_and_load_keep_level_tables_and_level_cells() {
        let mut map = MapFile::new();
        map.tanks = Some(5);
        map.mission.kind = Mission::Hunt;
        map.spawn.kind = SpawnKind::Waves;
        map.spawn.waves = Some(3);
        map.spawn.size = Some(2);
        map.spawn.growth = Some(1);
        map.spawn.tier_start = Some(Tier::Light);
        map.spawn.tier_end = Some(Tier::Heavy);
        map.tank2 = Some(TankKind::Scout);
        map.set_cell(30, 11, CellObject::Start);
        map.set_cell(32, 11, CellObject::Start2);
        map.set_cell(35, 11, CellObject::Frog);
        map.set_cell(5, 11, CellObject::EnemyFrog);
        map.set_cell(0, 11, CellObject::Gate);
        map.set_cell(39, 11, CellObject::Gate);
        map.set_cell(20, 11, CellObject::Wall { material: Material::Brick });
        map.set_cell(21, 11, CellObject::Sandbag);
        map.set_cell(22, 11, CellObject::Barrel { drum: None });
        map.set_cell(23, 11, CellObject::Fence);
        map.set_cell(10, 5, CellObject::Portal);
        map.set_cell(30, 5, CellObject::Portal);

        let dir = std::env::temp_dir().join(format!("bongbong-editor-test-{}", std::process::id()));
        let path = dir.join("round-trip.toml");
        map.save(&path).expect("save");
        let back = MapFile::load(&path).expect("load");
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(back.tanks, Some(5));
        assert_eq!(back.mission, map.mission);
        assert_eq!(back.spawn, map.spawn);
        assert_eq!(back.enemy_frog_cell(), Some((5, 11)));
        assert_eq!(back.gate_cells(), vec![(0, 11), (39, 11)]);
        assert_eq!(back.start_cell(), Some((30, 11)));
        assert_eq!(back.start2_cell(), Some((32, 11)));
        assert_eq!(back.tank2, Some(TankKind::Scout));
        assert_eq!(back.frog_cell(), Some((35, 11)));
        assert_eq!(back.portal_cells(), vec![(10, 5), (30, 5)]);
        assert_eq!(back.cells.len(), map.cells.len());
        assert_eq!(back.cell(22, 11).and_then(|c| c.material()), Some(Material::Barrel));
    }

    #[test]
    fn gate_chevrons_point_inward_only_on_edge_cells() {
        let at = |col: i32, row: i32| map::cell_to_world(col, row);
        assert_eq!(gate_inward(at(0, 11), W, H), Some(Position::new(1.0, 0.0)));
        assert_eq!(gate_inward(at(39, 11), W, H), Some(Position::new(-1.0, 0.0)));
        assert_eq!(gate_inward(at(20, 0), W, H), Some(Position::new(0.0, 1.0)));
        assert_eq!(gate_inward(at(20, 22), W, H), Some(Position::new(0.0, -1.0)));
        assert_eq!(gate_inward(at(20, 11), W, H), None);
    }

    #[test]
    fn every_tool_has_a_unique_name_that_parses_back() {
        let mut seen = std::collections::HashSet::new();
        for tool in TOOLS {
            assert!(seen.insert(tool.name()), "duplicate name {}", tool.name());
            assert_eq!(Tool::parse(tool.name()), Some(tool));
        }
        let per_category: usize = Category::ALL.iter().map(|c| c.tools().count()).sum();
        assert_eq!(per_category + 2, TOOLS.len(), "every tool but the eraser and the select tool is in a category");
        assert!(Tool::Select.category().is_none() && Tool::Select.object().is_none() && !Tool::Select.is_singleton());
    }

    /// The two start brushes are singletons independently of each other:
    /// moving one leaves the other, and painting one over the other's
    /// cell replaces it (the badge follows).
    #[test]
    fn both_start_brushes_are_independent_singletons() {
        let mut ed = MapEditor::new(MapFile::new());
        ed.select_tool(Tool::Start);
        ed.stroke(&[(3, 3)], false);
        ed.select_tool(Tool::Start2);
        ed.stroke(&[(4, 4)], false);
        assert!(ed.singleton_placed(Tool::Start) && ed.singleton_placed(Tool::Start2));
        assert_eq!(ed.map().start_cell(), Some((3, 3)));
        assert_eq!(ed.map().start2_cell(), Some((4, 4)));
        let depth = ed.history().undo_depth();
        // A move: the old cell cleared, the new one placed, player 1 untouched.
        ed.stroke(&[(6, 6)], false);
        assert_eq!(ed.map().start2_cell(), Some((6, 6)));
        assert_eq!(ed.map().cell(4, 4), None);
        assert_eq!(ed.map().start_cell(), Some((3, 3)));
        assert_eq!(ed.history().undo_depth(), depth + 1);
        ed.undo();
        assert_eq!(ed.map().start2_cell(), Some((4, 4)));
        // Player 1's brush over player 2's cell replaces it.
        ed.select_tool(Tool::Start);
        ed.stroke(&[(4, 4)], false);
        assert_eq!(ed.map().start_cell(), Some((4, 4)));
        assert_eq!(ed.map().start2_cell(), None);
        assert!(!ed.singleton_placed(Tool::Start2));
        assert_eq!(Tool::parse("start2"), Some(Tool::Start2));
    }

    #[test]
    fn a_drag_between_frames_joins_its_cells_edge_to_edge() {
        assert_eq!(edge_joined_line((2, 2), (2, 2)), vec![]);
        assert_eq!(edge_joined_line((2, 2), (5, 2)), vec![(3, 2), (4, 2), (5, 2)]);
        assert_eq!(edge_joined_line((2, 2), (2, 0)), vec![(2, 1), (2, 0)]);
        assert_eq!(edge_joined_line((0, 0), (2, 2)), vec![(0, 1), (1, 1), (1, 2), (2, 2)]);
        assert_eq!(edge_joined_line((0, 0), (-3, 1)), vec![(-1, 0), (-1, 1), (-2, 1), (-3, 1)]);
        for to in [(7, 3), (-4, 9), (0, -6), (5, 5)] {
            let line = edge_joined_line((1, 1), to);
            assert_eq!(line.len() as i32, (to.0 - 1).abs() + (to.1 - 1).abs());
            assert_eq!(line.last(), Some(&to));
            let mut prev = (1, 1);
            for &c in &line {
                assert_eq!((c.0 - prev.0).abs() + (c.1 - prev.1).abs(), 1, "{c:?} shares an edge with {prev:?}");
                prev = c;
            }
        }
        // The pointer path paints the gap; the tools' stroke does not.
        let mut ed = MapEditor::new(MapFile::new());
        ed.select_tool(Tool::Water);
        ed.begin_stroke((3, 3), false);
        ed.drag_to((5, 5));
        ed.finish_stroke();
        for c in [(3, 3), (3, 4), (4, 4), (4, 5), (5, 5)] {
            assert_eq!(ed.map().cell(c.0, c.1), Some(&CellObject::Water), "{c:?}");
        }
        ed.stroke(&[(10, 3), (12, 3)], false);
        assert_eq!(ed.map().cell(11, 3), None);
    }

    #[test]
    fn a_stroke_paints_once_per_cell_and_toggle_erases_on_its_first_cell() {
        let mut ed = MapEditor::new(MapFile::new());
        ed.select_tool(Tool::Wall(Material::Brick));
        let changes = ed.stroke(&[(10, 5), (11, 5), (11, 5), (12, 5)], false);
        assert_eq!(changes.len(), 3);
        assert_eq!(ed.map().cell(11, 5), Some(&brick()));
        assert_eq!(ed.history().undo_depth(), 1);

        // A press on a cell holding exactly the brush's object clears it,
        // and the drag keeps clearing - never paints - past empty cells.
        let changes = ed.stroke(&[(11, 5), (12, 5), (13, 5)], false);
        assert_eq!(changes.len(), 2);
        assert_eq!(ed.map().cell(11, 5), None);
        assert_eq!(ed.map().cell(13, 5), None);

        // A press on an empty cell paints, and painting over a brick with
        // a brick is a no-op inside the same stroke.
        ed.stroke(&[(13, 5), (10, 5)], false);
        assert_eq!(ed.map().cell(13, 5), Some(&brick()));
        assert_eq!(ed.map().cell(10, 5), Some(&brick()));

        // A different material overwrites rather than toggles.
        ed.select_tool(Tool::Wall(Material::Iron));
        ed.stroke(&[(10, 5)], false);
        assert_eq!(ed.map().cell(10, 5), Some(&CellObject::Wall { material: Material::Iron }));

        // The secondary button erases whatever the brush.
        ed.stroke(&[(10, 5)], true);
        assert_eq!(ed.map().cell(10, 5), None);
        assert!(ed.dirty());
    }

    /// The portal brush places any number of anchors (a network, not a
    /// singleton) and follows the toggle-erase rule like every other
    /// multi-instance brush; its cell reads back by its own name.
    #[test]
    fn portal_brush_is_multi_instance_and_toggle_erases() {
        let mut ed = MapEditor::new(MapFile::new());
        ed.select_tool(Tool::Portal);
        assert!(!Tool::Portal.is_singleton());
        assert_eq!(Tool::parse("portal"), Some(Tool::Portal));
        assert_eq!(Tool::Portal.category(), Some(Category::Ground));
        ed.stroke(&[(10, 5)], false);
        ed.stroke(&[(30, 5)], false);
        assert_eq!(ed.map().portal_cells(), vec![(10, 5), (30, 5)]);
        // A stroke starting on a portal erases it and nothing else.
        ed.stroke(&[(10, 5), (11, 5)], false);
        assert_eq!(ed.map().portal_cells(), vec![(30, 5)]);
        assert_eq!(ed.map().cell(11, 5), None);
        assert_eq!(ed.history().undo_depth(), 3);
    }

    #[test]
    fn singletons_move_and_undo_restores_the_old_cell() {
        let mut ed = MapEditor::new(MapFile::new());
        ed.select_tool(Tool::Frog);
        ed.stroke(&[(3, 3)], false);
        let changes = ed.stroke(&[(6, 6)], false);
        assert_eq!(changes.len(), 2, "a move is a clear and a placement");
        assert_eq!(ed.map().frog_cell(), Some((6, 6)));
        ed.undo();
        assert_eq!(ed.map().frog_cell(), Some((3, 3)));
        ed.redo();
        assert_eq!(ed.map().frog_cell(), Some((6, 6)));
        // Tapping the frog with the frog tool removes it.
        ed.stroke(&[(6, 6)], false);
        assert_eq!(ed.map().frog_cell(), None);
        assert!(ed.singleton_placed(Tool::Frog) == false);
    }

    /// CLEAR MAP empties the cells and nothing else, as one undo step that
    /// leaves the baseline alone; an empty canvas records nothing.
    #[test]
    fn clear_empties_the_canvas_as_one_undo_step_and_keeps_the_settings() {
        let mut base = MapFile::new();
        base.set_cell(1, 1, brick());
        base.set_cell(3, 4, CellObject::Frog);
        base.tanks = Some(4);
        base.mission.kind = Mission::Hunt;
        base.name = Some("arena".into());
        let mut ed = MapEditor::new(base.clone());
        ed.select_tool(Tool::Road);
        ed.stroke(&[(2, 2)], false);
        let settings = ed.settings();

        ed.clear();
        assert!(ed.map().cells.is_empty());
        assert_eq!(ed.settings(), settings, "settings survive a clear");
        assert_eq!(ed.name(), "arena");
        assert!(ed.dirty(), "a clear is not a new baseline");
        assert_eq!(ed.diff().removed, 2, "the baseline's two cells are gone");
        assert_eq!(ed.history().undo_depth(), 2, "the stroke and the clear");
        assert!(!ed.singleton_placed(Tool::Frog));

        ed.undo();
        assert_eq!(ed.map().cell(1, 1), Some(&brick()));
        assert_eq!(ed.map().cell(2, 2), Some(&CellObject::Road));
        assert_eq!(ed.map().cell(3, 4), Some(&CellObject::Frog));
        ed.redo();
        assert!(ed.map().cells.is_empty());

        ed.clear();
        assert_eq!(ed.history().undo_depth(), 2, "an empty canvas records nothing");
    }

    #[test]
    fn settings_load_and_reset_are_undo_steps_and_drive_dirty() {
        let mut base = MapFile::new();
        base.set_cell(1, 1, brick());
        base.name = Some("arena".into());
        let mut ed = MapEditor::new(base.clone());
        assert!(!ed.dirty());
        let mut s = ed.settings();
        s.tanks = Some(99);
        s.mission = Mission::Hunt;
        ed.apply_settings(s);
        assert_eq!(ed.settings().tanks, Some(31), "clamped to the live cap");
        assert_eq!(ed.diff().settings, vec!["tanks", "mission"]);
        ed.apply_settings(ed.settings());
        assert_eq!(ed.history().undo_depth(), 1, "a no-op change records nothing");

        ed.select_tool(Tool::Road);
        ed.stroke(&[(2, 2)], false);
        assert_eq!(ed.diff().added, 1);
        ed.reset();
        assert!(!ed.dirty());
        assert_eq!(ed.name(), "arena");
        ed.undo();
        assert!(ed.dirty());

        let mut other = MapFile::new();
        other.set_cell(5, 5, CellObject::Gate);
        ed.load(other);
        assert!(!ed.dirty(), "a load is the new baseline");
        assert_eq!(ed.map().cell(5, 5), Some(&CellObject::Gate));
        ed.undo();
        assert_eq!(ed.map().cell(2, 2), Some(&CellObject::Road));
    }

    #[test]
    fn the_water_brush_paints_ground_and_toggle_erases_like_road() {
        let mut ed = MapEditor::new(MapFile::new());
        assert_eq!(Tool::parse("water"), Some(Tool::Water));
        assert_eq!(Tool::Water.category(), Some(Category::Ground));
        assert!(!Tool::Water.is_singleton());
        ed.select_tool(Tool::Water);
        assert_eq!(ed.current_tool(Category::Ground), Tool::Water, "the category remembers it");
        ed.stroke(&[(3, 3), (3, 4), (3, 5)], false);
        for row in 3..=5 {
            assert_eq!(ed.map().cell(3, row), Some(&CellObject::Water));
        }
        assert!(!ed.map().cell(3, 4).unwrap().is_solid(), "water is ground");
        assert_eq!(ed.diff().added, 3);
        // A stroke starting on water erases, like road.
        ed.stroke(&[(3, 4), (3, 5), (3, 6)], false);
        assert_eq!(ed.map().cell(3, 3), Some(&CellObject::Water));
        assert_eq!(ed.map().cell(3, 4), None);
        assert_eq!(ed.map().cell(3, 5), None);
        assert_eq!(ed.map().cell(3, 6), None, "an erase stroke never paints");
        ed.undo();
        assert_eq!(ed.map().cell(3, 5), Some(&CellObject::Water), "one undo step per stroke");
        // Round trip through the file format.
        let text = ed.map().to_toml_string().unwrap();
        assert!(text.contains("kind = \"water\""), "{text}");
        assert_eq!(MapFile::from_toml_str(&text).unwrap().cell(3, 3), Some(&CellObject::Water));
    }

    /// The builder on the standard arena in a window just its size under a
    /// 32 pt bar (`BuilderFrame::headless`), a unit a point, no touch: the
    /// canvas at (0, 32) at its own size, the bar along the top.
    fn arena() -> BuilderFrame {
        BuilderFrame::headless((W, H), MapClass::Arena)
    }

    /// The builder on a field map in a window of `window` units, a unit a
    /// point: the canvas the shape of the window under the bar.
    fn field_frame(window: (f32, f32)) -> BuilderFrame {
        BuilderFrame::new(UiFrame::plain(window), (96.0 * 32.0, 54.0 * 32.0), MapClass::Field, None)
    }

    /// The window point a world point is drawn at, through the camera and
    /// the frame's view.
    fn window_at(ed: &MapEditor, frame: &BuilderFrame, world: Vec2) -> Vec2 {
        let v = ed.view_camera(&frame.layout).to_view(world);
        frame.view.to_window(Vec2::new(v.x + frame.layout.field.x, v.y + frame.layout.field.y))
    }

    /// The window point at the middle of a cell.
    fn on_cell(ed: &MapEditor, frame: &BuilderFrame, col: i32, row: i32) -> Vec2 {
        window_at(ed, frame, map::cell_to_world(col, row))
    }

    /// The window point `dx`, `dy` bitmap pixels into the canvas area.
    fn canvas_at(frame: &BuilderFrame, dx: f32, dy: f32) -> Vec2 {
        let f = frame.layout.field;
        frame.view.to_window(Vec2::new(f.x + dx, f.y + dy))
    }

    /// The window point in the middle of the canvas area.
    fn canvas_middle(frame: &BuilderFrame) -> Vec2 {
        canvas_at(frame, frame.layout.field.w / 2.0, frame.layout.field.h / 2.0)
    }

    /// Whether a button of that name is pressable now (`named_buttons`).
    fn has_named(ed: &MapEditor, frame: &BuilderFrame, name: &str) -> bool {
        ed.named_buttons(frame).iter().any(|(n, _)| n == name)
    }

    /// The window rect a named button stands at (`named_buttons`, which
    /// are UI points, through the UI frame).
    fn named(ed: &MapEditor, frame: &BuilderFrame, name: &str) -> Rectangle {
        let r = ed.named_buttons(frame).into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no {name} button")).1;
        frame.ui.rect_to_window(r)
    }

    /// A click on the middle of a named button.
    fn press_named(ed: &mut MapEditor, frame: &BuilderFrame, name: &str) -> EditorAction {
        let at = center(named(ed, frame, name));
        click(ed, frame, at)
    }

    /// A press-and-release at a window position, the way a click or a
    /// tap arrives over two frames.
    fn click(ed: &mut MapEditor, frame: &BuilderFrame, at: Vec2) -> EditorAction {
        let press = BuilderInput { pointer: Some(at), pressed: true, held: true, ..Default::default() };
        let action = ed.update(&press, frame);
        ed.update(&BuilderInput { pointer: Some(at), ..Default::default() }, frame);
        action
    }

    fn center(r: Rectangle) -> Vec2 {
        Vec2::new(r.x + r.width / 2.0, r.y + r.height / 2.0)
    }

    #[test]
    fn update_from_input_strokes_on_press_and_hold_and_ends_on_release() {
        let frame = arena();
        let mut ed = MapEditor::new(MapFile::new());
        let press = BuilderInput { pointer: Some(on_cell(&ed, &frame, 4, 4)), pressed: true, held: true, ..Default::default() };
        assert_eq!(ed.update(&press, &frame), EditorAction::None);
        let drag = BuilderInput { pointer: Some(on_cell(&ed, &frame, 5, 4)), held: true, ..Default::default() };
        ed.update(&drag, &frame);
        ed.update(&drag, &frame);
        assert_eq!(ed.history().undo_depth(), 0, "the stroke is open until release");
        ed.update(&BuilderInput::default(), &frame);
        assert_eq!(ed.history().undo_depth(), 1);
        assert_eq!(ed.map().cells.len(), 2);
        // Ctrl+Z through the same struct.
        ed.update(&BuilderInput { undo: true, ..Default::default() }, &frame);
        assert_eq!(ed.map().cells.len(), 0);
        // A press on PLAY is the mode switch, not a paint.
        let play = named(&ed, &frame, "play");
        let on_play = Vec2::new(play.x + 2.0, play.y + 2.0);
        let press = BuilderInput { pointer: Some(on_play), pressed: true, held: true, ..Default::default() };
        assert_eq!(ed.update(&press, &frame), EditorAction::Play);
        assert_eq!(ed.map().cells.len(), 0);
    }

    /// PLAY HERE is its own button on the bar, before PLAY: a press is the
    /// action, never a paint, with a finger as with the mouse.
    #[test]
    fn play_here_is_a_bar_button_before_play() {
        let frame = arena();
        let mut ed = MapEditor::new(MapFile::new());
        let here = named(&ed, &frame, "play_here");
        assert!(here.x + here.width <= named(&ed, &frame, "play").x);
        let at = center(here);
        let press = BuilderInput { pointer: Some(at), pressed: true, held: true, ..Default::default() };
        assert_eq!(ed.update(&press, &frame), EditorAction::PlayHere);
        ed.update(&BuilderInput { pointer: Some(at), ..Default::default() }, &frame);
        let finger = BuilderInput {
            pointer: Some(at),
            pressed: true,
            held: true,
            touches: vec![crate::touch::TouchPoint { id: 1, pos: at }],
            dt: 1.0 / 60.0,
            ..Default::default()
        };
        assert_eq!(ed.update(&finger, &frame), EditorAction::PlayHere);
        ed.update(&BuilderInput { dt: 1.0 / 60.0, ..Default::default() }, &frame);
        assert!(ed.map().cells.is_empty());
        assert_eq!(ed.history().undo_depth(), 0);
    }

    /// A 96 x 54 map holding a start penned in by iron, a lone portal and
    /// `gates` gates off the edge, in the builder on a 1080p desktop.
    fn check_editor(gates: i32) -> (MapEditor, BuilderFrame) {
        let mut map = MapFile::new();
        map.size = Some((96.0, 54.0));
        for c in 38..=42 {
            map.set_cell(c, 25, CellObject::Wall { material: Material::Iron });
            map.set_cell(c, 29, CellObject::Wall { material: Material::Iron });
        }
        for r in 25..=29 {
            map.set_cell(38, r, CellObject::Wall { material: Material::Iron });
            map.set_cell(42, r, CellObject::Wall { material: Material::Iron });
        }
        map.set_cell(40, 27, CellObject::Start);
        map.set_cell(60, 20, CellObject::Portal);
        for i in 0..gates {
            map.set_cell(20 + 2 * i, 40, CellObject::Gate);
        }
        let frame = field_frame((1920.0, 1080.0));
        let mut ed = MapEditor::new(map);
        let screen = CanvasScreen { device_per_px: frame.view.scale, points_per_px: frame.view.scale, coarse: true };
        ed.update(&BuilderInput { screen: Some(screen), ..Default::default() }, &frame);
        (ed, frame)
    }

    /// The index of the first finding of `kind` in the panel's report.
    fn finding_index(ed: &MapEditor, kind: crate::maplint::LintKind) -> usize {
        let (report, _) = ed.lint_report().expect("a report");
        report.findings.iter().position(|f| f.kind == kind).unwrap_or_else(|| panic!("no {kind:?}"))
    }

    /// CHECK opens the panel on a fresh lint, errors first; a press on a
    /// finding marks it and brings its cells to the middle of the canvas
    /// the panel leaves free; a press outside closes the panel and leaves
    /// the mark, which the next edit takes away.
    #[test]
    fn the_check_panel_lists_findings_errors_first_and_jumps_to_them() {
        use crate::maplint::LintKind;
        let (mut ed, frame) = check_editor(0);
        let layout = frame.layout;
        assert_eq!(ed.free_canvas(&frame), Rectangle::new(layout.field.x, layout.field.y, layout.field.w, layout.field.h));
        press_named(&mut ed, &frame, "check");
        assert_eq!(ed.open_menu(), Some("check"));
        let (report, stale) = ed.lint_report().expect("CHECK lints the canvas");
        assert!(!stale);
        let order: Vec<LintSeverity> = report.findings.iter().map(|f| f.severity).collect();
        let mut sorted = order.clone();
        sorted.sort_by_key(|s| *s as u8);
        assert_eq!(order, sorted, "errors first, then warnings, then notes");
        assert!(report.count(LintSeverity::Error) >= 1 && report.count(LintSeverity::Warning) >= 1, "{:?}", report.findings);
        // A press on the penned start's row - away from its FIX button.
        let i = finding_index(&ed, LintKind::StartPenned);
        let row = named(&ed, &frame, &format!("finding_{i}"));
        click(&mut ed, &frame, Vec2::new(row.x + 40.0, row.y + row.height / 2.0));
        assert_eq!(ed.open_menu(), Some("check"), "the panel stays open");
        assert_eq!(ed.lint_marked().map(|f| f.kind), Some(LintKind::StartPenned));
        assert!(!ed.camera().is_fit(), "the jump zooms in");
        let cells = ed.lint_marked().unwrap().cells.clone();
        let bounds = crate::maplint::cells_bounds(&cells).unwrap();
        let middle = Vec2::new(bounds.x + bounds.width / 2.0, bounds.y + bounds.height / 2.0);
        let on = ed.view_camera(&layout).to_view(middle);
        let free = ed.free_canvas(&frame);
        assert!(free.width < layout.field.w, "the open panel takes the canvas's right: {free:?}");
        let (x, y) = (on.x + layout.field.x, on.y + layout.field.y);
        assert!((x - (free.x + free.width / 2.0)).abs() < 2.0 && (y - (free.y + free.height / 2.0)).abs() < 2.0, "centred: {x},{y} in {free:?}");
        // The panel stands right of what it leaves free.
        let Some(PopupLayout::Lint(lint)) = ed.chrome(&frame).popup else { panic!("the CHECK panel") };
        assert!(frame.canvas_to_ui(Vec2::new(free.x + free.width, free.y)).x <= lint.panel.x, "{free:?} vs {:?}", lint.panel);
        // The pen and what is round it all show, left of the panel.
        let vp = ed.viewport();
        let scale = ed.camera().scale(&vp);
        let context = CanvasRules::of(&crate::tuning::Tuning::DEFAULT).jump_context_cells;
        assert!(context.0 * 32.0 * scale <= free.width + 1.0, "{scale}");
        // Closing the panel keeps the mark; an edit lets it go.
        let canvas = canvas_at(&frame, 40.0, 300.0);
        click(&mut ed, &frame, canvas);
        assert_eq!(ed.open_menu(), None);
        assert!(ed.lint_marked().is_some(), "the mark outlives the panel");
        assert!(ed.map().cells.len() > 3, "the closing press painted nothing");
        let painted = ed.map().cells.len();
        click(&mut ed, &frame, canvas);
        assert_eq!(ed.map().cells.len(), painted + 1);
        assert!(ed.lint_marked().is_none(), "an edit takes the mark away");
        assert!(ed.lint_report().unwrap().1, "and the report is older than the map");
    }

    /// FIX makes a finding's quick fix as one undo step - the penned start
    /// moved out, the lone portal taken away - and the panel lints the
    /// map it leaves; undo brings the finding back.
    #[test]
    fn each_quick_fix_is_one_undo_step() {
        use crate::maplint::LintKind;
        let (mut ed, frame) = check_editor(0);
        press_named(&mut ed, &frame, "check");
        let depth = ed.history().undo_depth();
        let i = finding_index(&ed, LintKind::StartPenned);
        press_named(&mut ed, &frame, &format!("fix_{i}"));
        assert_eq!(ed.history().undo_depth(), depth + 1, "one step");
        let start = ed.map().start_cell().expect("the start is still on the map");
        assert_ne!(start, (40, 27), "moved out of the pen");
        assert!(!(start.0 > 38 && start.0 < 42 && start.1 > 25 && start.1 < 29), "{start:?}");
        assert!(ed.lint_report().unwrap().0.findings.iter().all(|f| f.kind != LintKind::StartPenned), "linted again");
        assert_eq!(ed.open_menu(), Some("check"));
        let i = finding_index(&ed, LintKind::PortalAlone);
        press_named(&mut ed, &frame, &format!("fix_{i}"));
        assert_eq!(ed.history().undo_depth(), depth + 2);
        assert!(ed.map().portal_cells().is_empty());
        // Undo, under the panel: the portal is back, and so is its finding.
        ed.update(&BuilderInput { undo: true, ..Default::default() }, &frame);
        assert_eq!(ed.map().portal_cells(), vec![(60, 20)]);
        assert!(ed.lint_report().unwrap().0.findings.iter().any(|f| f.kind == LintKind::PortalAlone));
        ed.undo();
        assert_eq!(ed.map().start_cell(), Some((40, 27)));
        // A fix the map no longer matches makes nothing.
        assert!(!ed.apply_fix(crate::maplint::LintFix::Remove { at: (61, 20) }));
        assert!(!ed.apply_fix(crate::maplint::LintFix::Move { from: (60, 20), to: (40, 25) }), "onto a wall");
        assert_eq!(ed.history().undo_depth(), depth);
    }

    /// Past a page of findings the panel pages, by the pager's halves -
    /// the touch screen's way - and by the wheel.
    #[test]
    fn the_check_panel_pages() {
        use crate::maplint::LintKind;
        let (mut ed, frame) = check_editor(9);
        press_named(&mut ed, &frame, "check");
        let len = ed.lint_report().unwrap().0.findings.len();
        let Some(PopupLayout::Lint(lint)) = ed.chrome(&frame).popup else { panic!("the CHECK panel") };
        let per = lint.per_page;
        assert!(len > per, "{len} findings");
        let room = frame.under_bar();
        assert!(lint.panel.y + lint.panel.height <= room.y + room.height, "the panel fits under the bar");
        let panel = frame.ui.rect_to_window(lint.panel);
        assert!(named(&ed, &frame, "finding_0").y > panel.y, "under the header");
        assert!(!has_named(&ed, &frame, &format!("finding_{per}")), "one page at a time");
        press_named(&mut ed, &frame, "page_next");
        assert!(has_named(&ed, &frame, &format!("finding_{per}")));
        press_named(&mut ed, &frame, "page_back");
        assert!(has_named(&ed, &frame, "finding_0"));
        ed.update(&BuilderInput { pointer: Some(center(panel)), wheel: -1.0, ..Default::default() }, &frame);
        assert!(has_named(&ed, &frame, &format!("finding_{per}")), "the wheel turns it too");
        // Every interior gate is an error with its fix; one of them, on
        // the second page, taken away.
        let gates = ed.map().gate_cells().len();
        let i = (per..len).find(|&i| ed.lint_report().unwrap().0.findings[i].kind == LintKind::GateNotOnEdge).expect("a gate on page two");
        press_named(&mut ed, &frame, &format!("fix_{i}"));
        assert_eq!(ed.map().gate_cells().len(), gates - 1);
        // Esc closes it.
        ed.update(&BuilderInput { escape: true, ..Default::default() }, &frame);
        assert_eq!(ed.open_menu(), None);
    }

    #[test]
    fn every_bar_button_hit_rect_reaches_under_the_bar() {
        use crate::EDITOR_BAR_HIT_SLACK;
        let frame = arena();
        let bar = frame.bar();
        // Under the list end of a category button, under the middle of any other.
        let below = |r: Rectangle| frame.ui.to_window(Vec2::new(r.x + r.width - 8.0, r.y + r.height + EDITOR_BAR_HIT_SLACK - 1.0));
        let wall = bar.category(Category::Wall).expect("a desktop's bar has the five");
        assert_eq!(bar.hit(frame.to_ui(below(wall.rect))), Some(BarButton::CategoryMenu(Category::Wall)));
        assert_eq!(bar.hit(frame.to_ui(below(bar.map))), Some(BarButton::Map));
        let far = Vec2::new(bar.map.x + 2.0, bar.strip.y + bar.strip.height + EDITOR_BAR_HIT_SLACK + 1.0);
        assert_eq!(bar.hit(far), None);
        // A press in the slack under MAP opens it and paints nothing.
        let mut ed = MapEditor::new(MapFile::new());
        click(&mut ed, &frame, below(bar.map));
        assert_eq!(ed.open_menu(), Some("map"));
        assert!(ed.map().cells.is_empty());
    }

    /// Every category's rows and the MAP panel's steppers are a finger's
    /// size and stand in the room under the bar on a phone and a desktop,
    /// with and without touch, a stepper's `<` left of its `>`.
    #[test]
    fn every_list_row_and_map_stepper_is_a_fingers_size_under_the_bar() {
        for window in [(568.0, 320.0), (852.0, 393.0), (1088.0, 576.0), (1920.0, 1080.0)] {
            for touch in [false, true] {
                let ui = UiFrame::new(window, 1.0, 1.0, Insets::default(), touch);
                let frame = BuilderFrame::new(ui, (W, H), MapClass::Arena, None);
                let (bar, room) = (frame.bar(), frame.under_bar());
                let inside = |r: Rectangle| {
                    r.x >= room.x - 1e-3 && r.y >= room.y - 1e-3 && r.x + r.width <= room.x + room.width + 1e-3 && r.y + r.height <= room.y + room.height + 1e-3
                };
                for category in Category::ALL {
                    let n = category.tools().count();
                    let list = chrome::menu_list(bar.tools_anchor(category), room, n);
                    for i in 0..n {
                        let row = list.row(i);
                        assert!(row.height >= 48.0 && inside(row), "{window:?} touch={touch} {}: {row:?}", category.name());
                    }
                }
                let settings = SettingsLayout::of(bar.map, room, SETTINGS_ROWS.len());
                for page in 0..settings.pages {
                    for i in 0..SETTINGS_ROWS.len() {
                        let Some(row) = settings.row(i, page) else { continue };
                        let (dec, inc) = (MapEditor::settings_dec_rect(row), MapEditor::settings_inc_rect(row));
                        assert!(dec.width >= 48.0 && dec.height >= 48.0 && inc.width >= 48.0 && inc.height >= 48.0);
                        assert!(dec.x + dec.width <= inc.x && inside(dec) && inside(inc), "{window:?} touch={touch}: {dec:?} {inc:?}");
                        let reset = MapEditor::settings_reset_rect(row);
                        assert!(reset.height >= 48.0 && inside(reset));
                    }
                }
            }
        }
    }

    #[test]
    fn a_category_button_selects_on_its_icon_half_and_opens_its_list_on_its_caret_half() {
        let frame = arena();
        let mut ed = MapEditor::new(MapFile::new());
        // The caret half opens the dropdown and selects nothing yet.
        press_named(&mut ed, &frame, "list_prop");
        assert_eq!(ed.open_menu(), Some("prop"));
        assert_eq!(ed.tool(), Tool::Wall(Material::Brick));
        // Picking a row selects that tool, makes it the category's current
        // one, closes the list and paints nothing.
        press_named(&mut ed, &frame, "tool_barrel");
        assert_eq!(ed.tool(), Tool::Prop(Material::Barrel));
        assert_eq!(ed.current_tool(Category::Prop), Tool::Prop(Material::Barrel));
        assert_eq!(ed.open_menu(), None);
        assert!(ed.map().cells.is_empty());
        // The icon half of WALL makes its current tool the brush again.
        press_named(&mut ed, &frame, "category_wall");
        assert_eq!(ed.tool(), Tool::Wall(Material::Brick));
        assert_eq!(ed.open_menu(), None);
        // ERASE is a plain button; UNDO with nothing to undo is harmless.
        press_named(&mut ed, &frame, "erase");
        assert_eq!(ed.tool(), Tool::Eraser);
        press_named(&mut ed, &frame, "undo");
        assert!(ed.map().cells.is_empty());
    }

    /// On a phone the five category buttons fold into TOOLS: it opens the
    /// palette of every category, a cell picks its tool and closes it, and
    /// a press outside closes it and paints nothing.
    #[test]
    fn a_folded_bar_picks_tools_from_its_palette() {
        let insets = Insets { left: 59.0 * 3.0, top: 0.0, right: 59.0 * 3.0, bottom: 21.0 * 3.0 };
        let ui = UiFrame::new((852.0 * 3.0, 393.0 * 3.0), 3.0, 1.0, insets, true);
        let frame = BuilderFrame::new(ui, (W, H), MapClass::Arena, None);
        assert!(matches!(frame.bar().tools, BarTools::Folded(_)), "{:?}", frame.bar());
        let mut ed = MapEditor::new(MapFile::new());
        assert!(!has_named(&ed, &frame, "list_prop"));
        press_named(&mut ed, &frame, "tools");
        assert_eq!(ed.open_menu(), Some("tools"));
        for tool in TOOLS.iter().filter(|t| t.category().is_some()) {
            assert!(has_named(&ed, &frame, &format!("tool_{}", tool.name())), "{} in the palette", tool.name());
        }
        press_named(&mut ed, &frame, "tool_tesla");
        assert_eq!(ed.tool(), Tool::Tower(TowerKind::Tesla, Side::Player));
        assert_eq!(ed.current_tool(Category::Prop), Tool::Tower(TowerKind::Tesla, Side::Player));
        assert_eq!(ed.open_menu(), None);
        press_named(&mut ed, &frame, "tools");
        let p = on_cell(&ed, &frame, 30, 15);
        click(&mut ed, &frame, p);
        assert_eq!(ed.open_menu(), None);
        assert!(ed.map().cells.is_empty(), "the dismissing press painted through the palette");
        click(&mut ed, &frame, p);
        assert_eq!(ed.map().cell(30, 15), Some(&CellObject::for_tower(TowerKind::Tesla, Side::Player)));
    }

    #[test]
    fn a_press_outside_an_open_dropdown_closes_it_and_does_not_paint() {
        let frame = arena();
        let mut ed = MapEditor::new(MapFile::new());
        let caret = center(named(&ed, &frame, "list_wall"));
        click(&mut ed, &frame, caret);
        assert_eq!(ed.open_menu(), Some("wall"));
        let on_cell = on_cell(&ed, &frame, 20, 10);
        click(&mut ed, &frame, on_cell);
        assert_eq!(ed.open_menu(), None);
        assert!(ed.map().cells.is_empty(), "the dismissing press painted through the menu");
        // Esc closes too, and the next press on the same cell paints.
        click(&mut ed, &frame, caret);
        ed.update(&BuilderInput { escape: true, ..Default::default() }, &frame);
        assert_eq!(ed.open_menu(), None);
        click(&mut ed, &frame, on_cell);
        assert_eq!(ed.map().cell(20, 10), Some(&brick()));
    }

    #[test]
    fn the_map_panel_steps_values_and_never_paints_through() {
        let frame = arena();
        let mut ed = MapEditor::new(MapFile::new());
        press_named(&mut ed, &frame, "map");
        assert_eq!(ed.open_menu(), Some("map"));
        let inc = |r: SettingsRow| format!("{}_inc", r.name());
        let dec = |r: SettingsRow| format!("{}_dec", r.name());
        // TANKS walks auto, 0, 1, .. and back to auto below 0.
        assert_eq!(ed.settings().tanks, None);
        press_named(&mut ed, &frame, &inc(SettingsRow::Tanks));
        assert_eq!(ed.settings().tanks, Some(0));
        press_named(&mut ed, &frame, &inc(SettingsRow::Tanks));
        assert_eq!(ed.settings().tanks, Some(1));
        press_named(&mut ed, &frame, &dec(SettingsRow::Tanks));
        press_named(&mut ed, &frame, &dec(SettingsRow::Tanks));
        assert_eq!(ed.settings().tanks, None);
        press_named(&mut ed, &frame, &dec(SettingsRow::Tanks));
        assert_eq!(ed.settings().tanks, None, "auto is the floor");
        assert_eq!(ed.history().undo_depth(), 4, "one undo step per press that changed something");
        // TANK cycles auto and the chassis list; TIER START the tiers.
        press_named(&mut ed, &frame, &inc(SettingsRow::Tank));
        assert_eq!(ed.settings().tank, Some(TankKind::ALL[0]));
        press_named(&mut ed, &frame, &dec(SettingsRow::Tank));
        assert_eq!(ed.settings().tank, None);
        press_named(&mut ed, &frame, &dec(SettingsRow::Tank));
        assert_eq!(ed.settings().tank, Some(TankKind::ALL[TankKind::ALL.len() - 1]), "wraps");
        press_named(&mut ed, &frame, &inc(SettingsRow::Tank2));
        assert_eq!(ed.settings().tank2, Some(TankKind::ALL[0]));
        press_named(&mut ed, &frame, &dec(SettingsRow::Tank2));
        assert_eq!(ed.settings().tank2, None);
        press_named(&mut ed, &frame, &inc(SettingsRow::TierStart));
        assert_eq!(ed.settings().tier_start, Some(Tier::Light));
        // THEME cycles the list; the ground is rebuilt in the new theme.
        press_named(&mut ed, &frame, &inc(SettingsRow::Theme));
        assert_eq!(ed.settings().theme, Theme::Desert);
        assert_eq!(ed.map().theme, Theme::Desert);
        press_named(&mut ed, &frame, &inc(SettingsRow::Theme));
        assert_eq!(ed.settings().theme, Theme::Grass, "wraps");
        // WEATHER cycles every sky, backwards from clear to the last.
        press_named(&mut ed, &frame, &inc(SettingsRow::Weather));
        assert_eq!(ed.map().weather, Weather::Night);
        press_named(&mut ed, &frame, &dec(SettingsRow::Weather));
        press_named(&mut ed, &frame, &dec(SettingsRow::Weather));
        assert_eq!(ed.settings().weather, Weather::ALL[Weather::ALL.len() - 1], "wraps");
        let _ = ed.undo();
        assert_eq!(ed.settings().weather, Weather::Clear, "a weather press is one undo step");
        let _ = ed.redo();
        press_named(&mut ed, &frame, &inc(SettingsRow::Weather));
        assert_eq!(ed.settings().weather, Weather::Clear);
        press_named(&mut ed, &frame, &inc(SettingsRow::Mission));
        assert_eq!(ed.settings().mission, Mission::Hunt);
        press_named(&mut ed, &frame, &inc(SettingsRow::Spawn));
        assert_eq!(ed.settings().spawn, SpawnKind::Waves);
        // WAVES: auto, then 1..=20 with a hard ceiling.
        press_named(&mut ed, &frame, &inc(SettingsRow::Waves));
        assert_eq!(ed.settings().waves, Some(1));
        let mut s = ed.settings();
        s.waves = Some(20);
        ed.apply_settings(s);
        press_named(&mut ed, &frame, &inc(SettingsRow::Waves));
        assert_eq!(ed.settings().waves, Some(20));
        // Every press so far landed in the panel: it is still open and
        // nothing was painted under it.
        assert_eq!(ed.open_menu(), Some("map"));
        assert!(ed.map().cells.is_empty());
        assert!(ed.dirty());
        // RESET MAP reverts to the baseline as one undoable step.
        press_named(&mut ed, &frame, "reset");
        assert!(!ed.dirty());
        assert_eq!(ed.open_menu(), Some("map"));
        // A press outside closes the panel and paints nothing.
        let p = on_cell(&ed, &frame, 5, 15);
        click(&mut ed, &frame, p);
        assert_eq!(ed.open_menu(), None);
        assert!(ed.map().cells.is_empty());
    }

    /// Where the room under the bar is short the MAP panel pages: every row
    /// on one of its pages, the pager's halves and the wheel turning them,
    /// and a press on the pager steps nothing.
    #[test]
    fn a_short_map_panel_pages_by_its_pager_and_the_wheel() {
        let ui = UiFrame::new((667.0 * 2.0, 375.0 * 2.0), 2.0, 1.0, Insets::default(), true);
        let frame = BuilderFrame::new(ui, (W, H), MapClass::Arena, None);
        let mut ed = MapEditor::new(MapFile::new());
        press_named(&mut ed, &frame, "map");
        let Some(PopupLayout::Settings(settings)) = ed.chrome(&frame).popup else { panic!("the MAP panel") };
        assert!(settings.pages > 1 && settings.pager.is_some(), "{settings:?}");
        let mut seen = std::collections::BTreeSet::new();
        for page in 0..settings.pages {
            for (name, _) in ed.named_buttons(&frame) {
                seen.insert(name);
            }
            if page + 1 < settings.pages {
                press_named(&mut ed, &frame, "page_next");
            }
        }
        for row in SETTINGS_ROWS {
            let name = if row == SettingsRow::Reset { "reset".to_string() } else { format!("{}_inc", row.name()) };
            assert!(seen.contains(&name), "{name} on no page");
        }
        assert_eq!(ed.history().undo_depth(), 0, "the pager steps nothing");
        assert_eq!(ed.open_menu(), Some("map"));
        // The wheel walks the pages back, and no further than the first.
        let panel = center(frame.ui.rect_to_window(settings.rows.panel));
        for _ in 0..settings.pages + 1 {
            ed.update(&BuilderInput { pointer: Some(panel), wheel: 1.0, ..Default::default() }, &frame);
        }
        assert!(has_named(&ed, &frame, "tanks_inc"), "back on the first page");
        ed.update(&BuilderInput { pointer: Some(panel), wheel: -1.0, ..Default::default() }, &frame);
        assert!(!has_named(&ed, &frame, "tanks_inc"), "the wheel turned it on");
    }

    #[test]
    fn the_wheel_over_a_category_button_cycles_its_tool() {
        let frame = arena();
        let mut ed = MapEditor::new(MapFile::new());
        let wall = center(named(&ed, &frame, "category_wall"));
        ed.update(&BuilderInput { pointer: Some(wall), wheel: -1.0, ..Default::default() }, &frame);
        assert_eq!(ed.tool(), Tool::Wall(Material::Iron));
        ed.update(&BuilderInput { pointer: Some(wall), wheel: 1.0, ..Default::default() }, &frame);
        assert_eq!(ed.tool(), Tool::Wall(Material::Brick));
        ed.update(&BuilderInput { pointer: Some(wall), wheel: 1.0, ..Default::default() }, &frame);
        assert_eq!(ed.tool(), Tool::Wall(Material::Glass), "wraps");
        // Off the buttons the wheel does nothing to the tool.
        let field = Vec2::new(400.0, 400.0);
        ed.update(&BuilderInput { pointer: Some(field), wheel: -1.0, ..Default::default() }, &frame);
        assert_eq!(ed.tool(), Tool::Wall(Material::Glass));
    }

    /// A 96 x 54 map in the builder on a window of `window` units, a unit a
    /// point, on a screen `device` device pixels to the window unit.
    fn big_editor(window: (f32, f32), device: f32, coarse: bool) -> (MapEditor, BuilderFrame) {
        let mut map = MapFile::new();
        map.size = Some((96.0, 54.0));
        let frame = field_frame(window);
        let mut ed = MapEditor::new(map);
        let screen = CanvasScreen { device_per_px: frame.view.scale * device, points_per_px: frame.view.scale, coarse };
        ed.update(&BuilderInput { screen: Some(screen), ..Default::default() }, &frame);
        (ed, frame)
    }

    /// Until the builder is given a frame its canvas area is the canvas's
    /// own field, whatever map it holds - a map loaded then is measured at
    /// FIT a texel a pixel, as `BuilderFrame::headless` lays it out - and
    /// once a frame has set it, a load keeps the window's area.
    #[test]
    fn an_unframed_canvas_is_measured_against_the_map_it_holds() {
        let mut big = MapFile::default();
        big.size = Some((48.0, 24.0));
        let mut ed = MapEditor::new(MapFile::default());
        ed.load(big.clone());
        assert_eq!((ed.viewport().area, ed.viewport().fit_scale()), ((1536.0, 768.0), 1.0));
        let (mut ed, frame) = big_editor((1920.0, 1080.0), 1.0, true);
        ed.load(big);
        assert_eq!(ed.viewport().area, (frame.layout.field.w, frame.layout.field.h), "the window's area stays");
    }

    /// The wheel zooms at the cursor - the world under it stays - and the
    /// bar's FIT button brings the whole canvas back.
    #[test]
    fn the_wheel_zooms_at_the_cursor_and_fit_brings_the_whole_canvas_back() {
        let (mut ed, frame) = big_editor((1920.0, 1080.0), 1.0, true);
        assert!(ed.camera().is_fit());
        let at = canvas_at(&frame, 300.0, 200.0);
        let before = ed.world_at(at, &frame).expect("on the canvas");
        ed.update(&BuilderInput { pointer: Some(at), wheel: 1.0, ..Default::default() }, &frame);
        assert!(!ed.camera().is_fit(), "a notch away from you zooms in");
        let after = ed.world_at(at, &frame).expect("on the canvas");
        assert!((before.x - after.x).abs() <= 2.0 && (before.y - after.y).abs() <= 2.0, "{before:?} -> {after:?}");
        // On a coarse screen the step lands on whole blocks.
        let vp = ed.viewport();
        let device = vp.device_scale(ed.camera().scale(&vp));
        assert!(((device * 2.0) - (device * 2.0).round()).abs() < 1e-3, "{device}");
        // A trackpad's half notches add up to one step.
        let scale = ed.camera().scale(&vp);
        ed.update(&BuilderInput { pointer: Some(at), wheel: 0.5, ..Default::default() }, &frame);
        assert_eq!(ed.camera().scale(&vp), scale, "half a notch is no step yet");
        ed.update(&BuilderInput { pointer: Some(at), wheel: 0.5, ..Default::default() }, &frame);
        assert!(ed.camera().scale(&vp) > scale, "the second half is");
        // FIT takes it back; nothing was painted.
        press_named(&mut ed, &frame, "fit");
        assert!(ed.camera().is_fit());
        assert!(ed.map().cells.is_empty());
        // The wheel over the bar's empty parts zooms nothing.
        ed.update(&BuilderInput { pointer: Some(Vec2::new(1000.0, 10.0)), wheel: 1.0, ..Default::default() }, &frame);
        assert!(ed.camera().is_fit());
    }

    /// A drag with the middle button, or with Space held, pans the canvas
    /// with the pointer and paints nothing; the primary alone paints.
    #[test]
    fn a_middle_or_space_drag_pans_and_never_paints() {
        let (mut ed, frame) = big_editor((1920.0, 1080.0), 1.0, false);
        let mid = canvas_middle(&frame);
        ed.update(&BuilderInput { zoom_in: true, ..Default::default() }, &frame);
        ed.update(&BuilderInput { zoom_in: true, ..Default::default() }, &frame);
        assert!(!ed.camera().is_fit());
        let vp = ed.viewport();
        for drag in [
            BuilderInput { middle_held: true, ..Default::default() },
            BuilderInput { space_held: true, held: true, pressed: true, ..Default::default() },
        ] {
            let start = ed.camera().center(&vp);
            let under = ed.world_at(mid, &frame).unwrap();
            ed.update(&BuilderInput { pointer: Some(mid), ..drag.clone() }, &frame);
            let to = Vec2::new(mid.x - 60.0, mid.y + 25.0);
            ed.update(&BuilderInput { pointer: Some(to), pressed: false, ..drag.clone() }, &frame);
            ed.update(&BuilderInput { pointer: Some(to), ..Default::default() }, &frame);
            let moved = ed.camera().center(&vp);
            assert!((moved.x - start.x).abs() > 1.0 && (moved.y - start.y).abs() > 1.0, "{start:?} -> {moved:?}");
            let now = ed.world_at(to, &frame).unwrap();
            assert!((now.x - under.x).abs() <= 2.0 && (now.y - under.y).abs() <= 2.0, "the world followed the pointer: {under:?} vs {now:?}");
            assert!(ed.map().cells.is_empty(), "a pan paints nothing");
        }
        // The primary alone paints where it presses.
        let press = BuilderInput { pointer: Some(mid), pressed: true, held: true, ..Default::default() };
        ed.update(&press, &frame);
        ed.update(&BuilderInput { pointer: Some(mid), ..Default::default() }, &frame);
        let cell = ed.cell_at(mid, &frame).unwrap();
        assert_eq!(ed.map().cell(cell.0, cell.1), Some(&brick()));
    }

    /// `+`/`-` step the zoom about the canvas's middle and a held arrow
    /// moves the view the way it points; neither reaches a Save prompt.
    #[test]
    fn keys_zoom_and_pan_but_not_into_a_text_prompt() {
        let (mut ed, frame) = big_editor((1180.0, 820.0), 2.0, true);
        let vp = ed.viewport();
        ed.update(&BuilderInput { zoom_in: true, ..Default::default() }, &frame);
        ed.update(&BuilderInput { zoom_in: true, ..Default::default() }, &frame);
        let at = ed.camera().center(&vp);
        ed.update(&BuilderInput { pan_keys: Vec2::new(1.0, 0.0), dt: 0.1, ..Default::default() }, &frame);
        let moved = ed.camera().center(&vp);
        assert!(moved.x > at.x + 1.0 && (moved.y - at.y).abs() < 1e-3, "right moves the view right: {at:?} -> {moved:?}");
        ed.update(&BuilderInput { zoom_out: true, ..Default::default() }, &frame);
        ed.update(&BuilderInput { zoom_out: true, ..Default::default() }, &frame);
        ed.update(&BuilderInput { zoom_out: true, ..Default::default() }, &frame);
        assert!(ed.camera().is_fit(), "back out to FIT and no further");
        ed.popup = Some(Popup::Save { name: String::new() });
        ed.update(&BuilderInput { zoom_in: true, typed: "-".into(), ..Default::default() }, &frame);
        assert!(ed.camera().is_fit(), "a key in the Save prompt is a character");
    }

    /// A pointer anywhere on the canvas paints the cell the canvas draws
    /// under it - through `window_mapping`, the drawing's own placement -
    /// at every zoom and pan, on an arena and on a field map, on a coarse
    /// and on a fine screen.
    #[test]
    fn a_pointer_maps_to_the_cell_drawn_under_it_at_every_zoom() {
        let cases = [((1920.0, 1080.0), 1.0, true), ((852.0, 393.0), 3.0, false), ((1180.0, 820.0), 2.0, true)];
        for (window, device, coarse) in cases {
            for arena in [true, false] {
                let (mut ed, frame) = if arena {
                    let frame = BuilderFrame::new(UiFrame::plain(window), (W, H), MapClass::Arena, None);
                    let mut ed = MapEditor::new(MapFile::new());
                    let screen = CanvasScreen { device_per_px: frame.view.scale * device, points_per_px: frame.view.scale, coarse };
                    ed.update(&BuilderInput { screen: Some(screen), ..Default::default() }, &frame);
                    (ed, frame)
                } else {
                    big_editor(window, device, coarse)
                };
                for zoom_steps in 0..6 {
                    for pan in [Vec2::new(0.0, 0.0), Vec2::new(-137.3, 61.7), Vec2::new(4000.0, -4000.0)] {
                        ed.camera.fit();
                        let vp = ed.viewport();
                        let mid = Vec2::new(vp.area.0 * 0.37, vp.area.1 * 0.61);
                        for _ in 0..zoom_steps {
                            ed.camera.step(true, mid, &vp, &CanvasRules::of(&crate::tuning::Tuning::DEFAULT));
                        }
                        ed.camera.pan(pan, &vp, &CanvasRules::of(&crate::tuning::Tuning::DEFAULT));
                        let mapping = camera::window_mapping(&ed.view_camera(&frame.layout), &frame.view, &frame.layout);
                        // Every cell the canvas shows, at its middle and
                        // just inside its corners.
                        let (cols, rows) = (ed.map().field_size().0 as i32 / 32, ed.map().field_size().1 as i32 / 32);
                        for row in (0..rows).step_by(3) {
                            for col in (0..cols).step_by(3) {
                                let world = map::cell_to_world(col, row);
                                for (dx, dy) in [(0.0, 0.0), (-15.0, -15.0), (15.0, 15.0), (15.0, -15.0)] {
                                    let at = mapping.to_window(Vec2::new(world.x + dx, world.y + dy));
                                    let a = mapping.area;
                                    if at.x < a.x || at.y < a.y || at.x >= a.x + a.width || at.y >= a.y + a.height {
                                        continue;
                                    }
                                    let w = Vec2::new(world.x + dx, world.y + dy);
                                    let inside = w.x >= 0.0 && w.y >= 0.0 && w.x < ed.map().field_size().0 && w.y < ed.map().field_size().1;
                                    let got = ed.cell_at(at, &frame);
                                    if ed.navigator_rect(&frame).is_some_and(|r| Corners::plate(r).contains(frame.to_ui(at))) {
                                        assert_eq!(got, None, "the navigator stands over the canvas there");
                                    } else if inside {
                                        assert_eq!(got, Some((col, row)), "{window:?} arena={arena} steps={zoom_steps} pan={pan:?} at {at:?}");
                                    } else {
                                        assert_eq!(got, None, "past the field's edge paints nothing");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// The window point at fractions `fx`, `fy` across and down the
    /// navigator.
    fn on_navigator(ed: &MapEditor, frame: &BuilderFrame, fx: f32, fy: f32) -> Vec2 {
        let r = ed.navigator_rect(frame).expect("a navigator");
        frame.ui.to_window(Vec2::new(r.x + r.width * fx, r.y + r.height * fy))
    }

    /// Where a view centred on `world` at the camera's zoom can stand: the
    /// point kept as far inside the field as the view's half reaches.
    fn centre_for(ed: &MapEditor, world: Vec2) -> Vec2 {
        let vp = ed.viewport();
        let scale = ed.camera().scale(&vp);
        let (hw, hh) = (vp.area.0 / scale / 2.0, vp.area.1 / scale / 2.0);
        let (w, h) = ed.map().field_size();
        Vec2::new(world.x.clamp(hw, w - hw), world.y.clamp(hh, h - hh))
    }

    /// The navigator: a press on it puts the middle of the view on the
    /// world point under it at the view's zoom, a drag carries the view
    /// along until it lifts, a move after the lift does nothing, and none
    /// of it paints; from FIT a press zooms in to a tap's zoom there. Its
    /// picture stands in the bottom-right corner of the room under the bar,
    /// over the canvas.
    #[test]
    fn the_navigators_click_and_drag_move_the_builder_camera() {
        let (mut ed, frame) = big_editor((1920.0, 1080.0), 1.0, true);
        let layout = frame.layout;
        let nav = ed.navigator_rect(&frame).expect("a field map has one at FIT");
        let room = frame.under_bar();
        assert!(nav.x > room.x + room.width / 2.0 && nav.y > room.y + room.height / 2.0, "in the bottom-right: {nav:?} of {room:?}");
        let plate = Corners::plate(nav);
        assert!(plate.x + plate.width <= room.x + room.width && plate.y + plate.height <= room.y + room.height);
        let canvas = frame.canvas_ui();
        assert!(plate.x >= canvas.x && plate.y + plate.height <= canvas.y + canvas.height, "over the canvas: {plate:?} {canvas:?}");
        let (w, h) = ed.map().field_size();
        // From FIT: a press zooms in there.
        let p = on_navigator(&ed, &frame, 0.5, 0.5);
        ed.update(&BuilderInput { pointer: Some(p), pressed: true, held: true, ..Default::default() }, &frame);
        ed.update(&BuilderInput { pointer: Some(p), ..Default::default() }, &frame);
        assert!(!ed.camera().is_fit(), "a press from FIT zooms in");
        let vp = ed.viewport();
        let c = ed.camera().center(&vp);
        assert!((c.x - w / 2.0).abs() < 1.0 && (c.y - h / 2.0).abs() < 1.0, "centred where it was pressed: {c:?}");
        let zoom = ed.camera().scale(&vp);
        // A press keeps the zoom and jumps; a drag carries the view along.
        let a = on_navigator(&ed, &frame, 0.2, 0.3);
        ed.update(&BuilderInput { pointer: Some(a), pressed: true, held: true, ..Default::default() }, &frame);
        let want = centre_for(&ed, Vec2::new(0.2 * w, 0.3 * h));
        let c = ed.camera().center(&vp);
        assert!((c.x - want.x).abs() < 1.0 && (c.y - want.y).abs() < 1.0, "jumped: {c:?} vs {want:?}");
        assert_eq!(ed.camera().scale(&vp), zoom, "at the zoom it had");
        let b = on_navigator(&ed, &frame, 0.7, 0.6);
        ed.update(&BuilderInput { pointer: Some(b), held: true, ..Default::default() }, &frame);
        let want = centre_for(&ed, Vec2::new(0.7 * w, 0.6 * h));
        let c = ed.camera().center(&vp);
        assert!((c.x - want.x).abs() < 1.0 && (c.y - want.y).abs() < 1.0, "dragged: {c:?} vs {want:?}");
        // Dragged past the picture, the view stops at the field's edge.
        let far = frame.ui.to_window(Vec2::new(nav.x - 400.0, nav.y - 400.0));
        ed.update(&BuilderInput { pointer: Some(far), held: true, ..Default::default() }, &frame);
        let want = centre_for(&ed, Vec2::new(0.0, 0.0));
        let c = ed.camera().center(&vp);
        assert!((c.x - want.x).abs() < 1.0 && (c.y - want.y).abs() < 1.0, "held to the picture: {c:?} vs {want:?}");
        // Lifted: a move after it is nobody's.
        ed.update(&BuilderInput { pointer: Some(b), ..Default::default() }, &frame);
        ed.update(&BuilderInput { pointer: Some(a), ..Default::default() }, &frame);
        let still = ed.camera().center(&vp);
        assert!((still.x - c.x).abs() < 1e-3 && (still.y - c.y).abs() < 1e-3);
        assert!(ed.map().cells.is_empty(), "the navigator paints nothing");
        assert_eq!(ed.history().undo_depth(), 0);
        // A press on the canvas still paints, and a stroke dragged over the
        // navigator paints nothing under it.
        let cell_point = canvas_middle(&frame);
        ed.update(&BuilderInput { pointer: Some(cell_point), pressed: true, held: true, ..Default::default() }, &frame);
        ed.update(&BuilderInput { pointer: Some(on_navigator(&ed, &frame, 0.5, 0.5)), held: true, ..Default::default() }, &frame);
        ed.update(&BuilderInput { pointer: Some(cell_point), ..Default::default() }, &frame);
        assert!(!ed.map().cells.is_empty(), "the canvas painted");
        let nav_middle = frame.to_canvas(on_navigator(&ed, &frame, 0.5, 0.5));
        let under = ed.view_camera(&layout).to_world(layout.to_field(nav_middle));
        assert_eq!(ed.map().cell(map::world_to_cell(under).0, map::world_to_cell(under).1), None, "nothing under the navigator");
    }

    /// A finger on the navigator is its press and drag, not a canvas
    /// gesture: the view jumps and follows it, and nothing is painted or
    /// zoomed by a tap.
    #[test]
    fn a_finger_on_the_navigator_moves_the_view() {
        let (mut ed, frame) = big_editor((1180.0, 820.0), 2.0, false);
        ed.update(&BuilderInput { zoom_in: true, ..Default::default() }, &frame);
        ed.update(&BuilderInput { zoom_in: true, ..Default::default() }, &frame);
        let vp = ed.viewport();
        let zoom = ed.camera().scale(&vp);
        let (w, h) = ed.map().field_size();
        let a = on_navigator(&ed, &frame, 0.8, 0.8);
        let b = on_navigator(&ed, &frame, 0.3, 0.4);
        let frames: Vec<Vec<(i32, f32, f32)>> = (0..=10)
            .map(|i| {
                let t = i as f32 / 10.0;
                vec![(4, a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)]
            })
            .collect();
        fingers(&mut ed, &frame, &frames);
        let want = centre_for(&ed, Vec2::new(0.3 * w, 0.4 * h));
        let c = ed.camera().center(&vp);
        assert!((c.x - want.x).abs() < 1.0 && (c.y - want.y).abs() < 1.0, "followed the finger: {c:?} vs {want:?}");
        assert_eq!(ed.camera().scale(&vp), zoom, "a drag on the navigator never zooms");
        // A quick tap on it jumps there and is no paint and no zoom.
        let p = on_navigator(&ed, &frame, 0.5, 0.5);
        fingers(&mut ed, &frame, &[vec![(5, p.x, p.y)]]);
        let c = ed.camera().center(&vp);
        let want = centre_for(&ed, Vec2::new(0.5 * w, 0.5 * h));
        assert!((c.x - want.x).abs() < 1.0 && (c.y - want.y).abs() < 1.0, "{c:?} vs {want:?}");
        assert_eq!(ed.camera().scale(&vp), zoom);
        assert!(ed.map().cells.is_empty());
    }

    /// On an arena at FIT the canvas is the whole map, so there is no
    /// navigator and a press in the corner paints its cell; zoomed in, the
    /// corner is the navigator's.
    #[test]
    fn the_navigator_is_hidden_at_fit_on_an_arena() {
        let frame = arena();
        let mut ed = MapEditor::new(MapFile::new());
        let screen = CanvasScreen { device_per_px: 1.0, points_per_px: 1.0, coarse: true };
        ed.update(&BuilderInput { screen: Some(screen), ..Default::default() }, &frame);
        assert_eq!(ed.navigator_rect(&frame), None);
        let corner = canvas_at(&frame, frame.layout.field.w - 40.0, frame.layout.field.h - 30.0);
        assert!(ed.cell_at(corner, &frame).is_some(), "the canvas, all of it");
        ed.update(&BuilderInput { zoom_in: true, ..Default::default() }, &frame);
        let nav = ed.navigator_rect(&frame).expect("zoomed in, a navigator");
        assert!(Corners::plate(nav).contains(frame.to_ui(corner)), "in the corner: {nav:?}");
        assert_eq!(ed.cell_at(corner, &frame), None, "the navigator's, not a cell's");
        ed.update(&BuilderInput { zoom_out: true, ..Default::default() }, &frame);
        assert!(ed.camera().is_fit());
        assert_eq!(ed.navigator_rect(&frame), None);
    }

    /// Run frames of fingers - `(id, x, y)` in window coordinates - through
    /// the builder the way `app.rs` and the dev server's `builder_touch`
    /// feed them, ending with every finger lifted.
    fn fingers(ed: &mut MapEditor, frame: &BuilderFrame, frames: &[Vec<(i32, f32, f32)>]) {
        let mut down = false;
        for f in frames.iter().cloned().chain(std::iter::once(Vec::new())) {
            let touches: Vec<crate::touch::TouchPoint> = f.iter().map(|&(id, x, y)| crate::touch::TouchPoint { id, pos: Vec2::new(x, y) }).collect();
            let now = !touches.is_empty();
            let input = BuilderInput { pointer: touches.first().map(|t| t.pos), pressed: now && !down, held: now, touches, dt: 1.0 / 60.0, ..Default::default() };
            ed.update(&input, frame);
            down = now;
        }
    }

    /// Frames of fingers as `fingers` runs them, the last frame's fingers
    /// left down.
    fn hold_fingers(ed: &mut MapEditor, frame: &BuilderFrame, frames: &[Vec<(i32, f32, f32)>]) {
        let mut down = false;
        for f in frames {
            let touches: Vec<crate::touch::TouchPoint> = f.iter().map(|&(id, x, y)| crate::touch::TouchPoint { id, pos: Vec2::new(x, y) }).collect();
            let now = !touches.is_empty();
            let input = BuilderInput { pointer: touches.first().map(|t| t.pos), pressed: now && !down, held: now, touches, dt: 1.0 / 60.0, ..Default::default() };
            ed.update(&input, frame);
            down = now;
        }
    }

    /// A one-finger stroke from `from` across to `to`, held there.
    fn hold_stroke(ed: &mut MapEditor, frame: &BuilderFrame, from: Vec2, to: Vec2) {
        let frames: Vec<Vec<(i32, f32, f32)>> =
            (0..=10).map(|i| vec![(1, from.x + (to.x - from.x) * i as f32 / 10.0, from.y + (to.y - from.y) * i as f32 / 10.0)]).collect();
        hold_fingers(ed, frame, &frames);
    }

    /// The clear flag works no revision out for a canvas an edit has taken
    /// away from every cleared revision - its key (size, cell count and
    /// cells' digest) is none of theirs - and still answers exactly where a
    /// key matches: the same cell painted and taken off again by hand, or
    /// a settings edit, which the key does not see and the revision does.
    /// A map loaded with its stamp is keyed as it comes in.
    #[test]
    fn the_clear_flag_works_out_no_revision_away_from_every_cleared_one() {
        let mut map = MapFile::new();
        map.set_cell(3, 8, CellObject::Start);
        let mut ed = MapEditor::new(map);
        let first = ed.revision();
        ed.note_clear(first, 50.0);
        let worked_at = |ed: &MapEditor| ed.revision.get().map(|(edits, _)| edits);
        ed.select_tool(Tool::Wall(Material::Brick));
        ed.stroke(&[(10, 5)], false);
        assert_eq!(ed.par(), None);
        assert_ne!(worked_at(&ed), Some(ed.edits), "a revision no cleared one could be was worked out");
        // The brick taken off again by hand: the cleared revision's cells.
        ed.stroke(&[(10, 5)], false);
        assert_eq!(ed.map().cell(10, 5), None, "the toggle-erase rule took it off");
        assert_eq!(ed.par(), Some(50.0));
        // A settings edit leaves the key as it was; the revision says no.
        let mut settings = ed.settings();
        settings.tanks = Some(settings.tanks.unwrap_or(3) + 1);
        ed.apply_settings(settings);
        assert_eq!(ed.par(), None);
        assert_eq!(worked_at(&ed), Some(ed.edits), "a key that matches works the revision out");
        ed.undo();
        assert_eq!(ed.par(), Some(50.0));
        // A map that comes in with its stamp is keyed as it comes in.
        let saved = ed.map_to_save();
        let mut other = MapEditor::new(MapFile::new());
        other.load(saved);
        assert_eq!(other.par(), Some(50.0));
        other.select_tool(Tool::Wall(Material::Brick));
        other.stroke(&[(12, 6)], false);
        assert_eq!(other.par(), None);
        assert_ne!(worked_at(&other), Some(other.edits), "nor for a map that came in cleared");
    }

    /// The clear check is kept per revision of the canvas: a win noted on
    /// it clears it, its par the best of its wins to a tenth; an edit is a
    /// new revision nobody has won, and an undo back finds the cleared one
    /// again; SAVE's map carries the stamp of a cleared revision only; a
    /// map that comes in with a stamp for itself comes in cleared, a stale
    /// stamp proves nothing, and the canvas never keeps a stamp of its own.
    #[test]
    fn the_clear_check_is_kept_per_revision() {
        let mut map = MapFile::new();
        map.set_cell(3, 8, CellObject::Start);
        let mut ed = MapEditor::new(map.clone());
        assert_eq!(ed.par(), None);
        let first = ed.revision();
        assert_eq!(first, map.revision());
        ed.note_clear(first, 83.44);
        assert_eq!(ed.par(), Some(83.4), "to a tenth");
        ed.note_clear(first, 90.0);
        assert_eq!(ed.par(), Some(83.4), "the best of its wins");
        ed.note_clear(first, 61.26);
        assert_eq!(ed.par(), Some(61.3));
        let saved = ed.map_to_save();
        assert_eq!(saved.cleared_par(), Some(61.3), "SAVE writes the stamp");
        assert_eq!(ed.map().cleared, None, "the canvas carries none");
        // An edit: another revision, which nobody has won.
        ed.stroke(&[(10, 5)], false);
        assert_ne!(ed.revision(), first);
        assert_eq!(ed.par(), None);
        assert_eq!(ed.map_to_save().cleared, None, "SAVE writes no stamp for it");
        ed.undo();
        assert_eq!(ed.revision(), first);
        assert_eq!(ed.par(), Some(61.3), "undone back to the cleared revision");
        // A map that comes in with its stamp, by LOAD or as the builder's
        // first map, comes in cleared.
        let mut other = MapEditor::new(MapFile::new());
        other.load(saved.clone());
        assert_eq!(other.par(), Some(61.3));
        assert_eq!(other.map().cleared, None);
        assert_eq!(MapEditor::new(saved.clone()).par(), Some(61.3));
        // A stamp for another revision - the file edited by hand since.
        let mut stale = saved;
        stale.set_cell(20, 5, CellObject::Gate);
        assert_eq!(MapEditor::new(stale).par(), None);
    }

    /// The loupe's place: above the finger, clear of it; wholly left of it
    /// where it cannot stand centred above it for the right edge; beside it,
    /// level with it, where there is no room above; never off the area.
    #[test]
    fn the_loupe_stands_above_the_finger_and_on_the_canvas() {
        let area = Rectangle::new(0.0, 32.0, 1200.0, 600.0);
        let (side, lift) = (150.0, 50.0);
        let inside = |r: Rectangle| r.x >= area.x && r.y >= area.y && r.x + r.width <= area.x + area.width && r.y + r.height <= area.y + area.height;
        let r = loupe_rect(Vec2::new(500.0, 400.0), side, lift, area);
        assert_eq!(r, Rectangle::new(425.0, 200.0, side, side), "centred above, clear by the lift");
        let r = loupe_rect(Vec2::new(1150.0, 400.0), side, lift, area);
        assert!(r.x + r.width <= 1150.0 - lift + 1e-3 && r.y + r.height <= 400.0 - lift + 1e-3 && inside(r), "left of it by the right edge: {r:?}");
        let r = loupe_rect(Vec2::new(500.0, 100.0), side, lift, area);
        assert!(r.x + r.width <= 500.0 - lift + 1e-3 && inside(r), "beside it by the top: {r:?}");
        let r = loupe_rect(Vec2::new(60.0, 60.0), side, lift, area);
        assert!(r.x >= 60.0 + lift - 1e-3 && inside(r), "right of it in the top-left corner: {r:?}");
        for finger in [Vec2::new(0.0, 32.0), Vec2::new(1199.0, 631.0), Vec2::new(600.0, 631.0), Vec2::new(1199.0, 32.0)] {
            assert!(inside(loupe_rect(finger, side, lift, area)), "{finger:?}");
        }
    }

    /// The loupe over a painting finger where cells are small on the glass:
    /// in UI points like the rest of the chrome, above the finger, on the
    /// canvas and inside the safe area, showing the cell the stroke paints
    /// magnified on whole blocks, every block whole device pixels; none for
    /// a mouse, none where a cell is a finger's size and more, none once
    /// the finger lifts. On a phone whose UI point is several of its
    /// window's units as on a desktop.
    #[test]
    fn the_loupe_shows_the_cell_a_finger_paints_where_cells_are_small() {
        let rules = CanvasRules::of(&crate::tuning::Tuning::DEFAULT);
        let phone = |units: f32| {
            let insets = Insets { left: 59.0 * units, top: 0.0, right: 59.0 * units, bottom: 21.0 * units };
            UiFrame::new((852.0 * units, 393.0 * units), units, 1.0, insets, true)
        };
        for ui in [UiFrame::plain((852.0, 393.0)), phone(1.0), phone(3.0)] {
            let mut map = MapFile::new();
            map.size = Some((96.0, 54.0));
            let frame = BuilderFrame::new(ui, (96.0 * 32.0, 54.0 * 32.0), MapClass::Field, None);
            let mut ed = MapEditor::new(map);
            // Three device pixels to the point.
            let screen = CanvasScreen { device_per_px: frame.view.scale * 3.0 / ui.scale, points_per_px: frame.view.scale / ui.scale, coarse: false };
            ed.update(&BuilderInput { screen: Some(screen), ..Default::default() }, &frame);
            let layout = frame.layout;
            let vp = ed.viewport();
            let mid = canvas_middle(&frame);
            ed.camera.zoom_at(vp.scale_for_cell_mm(8.0), layout.to_field(frame.to_canvas(mid)), &vp, &rules);
            assert!(rules.paint_min_cell_mm < 8.0 && 8.0 < rules.loupe_cell_mm, "a finger paints and the loupe shows");
            let to = frame.ui.to_window(Vec2::new(frame.to_ui(mid).x + 120.0, frame.to_ui(mid).y + 40.0));
            hold_stroke(&mut ed, &frame, mid, to);
            let loupe = ed.loupe(&frame).expect("a loupe over a painting finger");
            let r = loupe.rect;
            let finger = frame.to_ui(to);
            let device = frame.device_per_point(vp.screen.device_per_px);
            assert!((device - 3.0).abs() < 1e-3, "{ui:?}: {device}");
            assert!(r.y + r.height <= finger.y - rules.loupe_lift_pt + 1.0 / device + 1e-3, "{ui:?}: above the finger: {r:?} {finger:?}");
            assert!((r.x + r.width / 2.0 - finger.x).abs() <= 0.5 / device + 1e-3, "{ui:?}: centred over it: {r:?}");
            let c = frame.canvas_ui();
            let a = ui.area;
            assert!(r.x >= c.x && r.y >= c.y && r.x + r.width <= c.x + c.width && r.y + r.height <= c.y + c.height, "{ui:?}: on the canvas");
            assert!(r.x >= a.x && r.y >= a.y && r.x + r.width <= a.x + a.w && r.y + r.height <= a.y + a.h, "{ui:?}: inside the safe area");
            let whole = |v: f32| (v * device - (v * device).round()).abs() < 1e-3;
            assert!(whole(r.x) && whole(r.y), "{ui:?}: its corner on a whole device pixel: {r:?}");
            assert_eq!(Some(loupe.cell), ed.cell_at(to, &frame), "the cell under the finger");
            let cell = map::cell_to_world(loupe.cell.0, loupe.cell.1);
            let w = loupe.world;
            assert!(cell.x - 16.0 >= w.x && cell.y - 16.0 >= w.y && cell.x + 16.0 <= w.x + w.width && cell.y + 16.0 <= w.y + w.height, "the whole cell in it: {w:?}");
            assert_eq!((w.x % 2.0, w.y % 2.0, w.width % 2.0, w.height % 2.0), (0.0, 0.0, 0.0, 0.0), "whole blocks on the block grid");
            assert!(((loupe.device_scale * 2.0).fract()).abs() < 1e-4, "whole blocks: {}", loupe.device_scale);
            assert!((r.width * device - w.width * loupe.device_scale).abs() < 1e-2, "every block whole device pixels: {r:?} {w:?}");
            assert!(loupe.device_scale > vp.device_scale(ed.camera().scale(&vp)), "magnified");
            assert!(!loupe.erase);
            // Lifted: gone.
            hold_fingers(&mut ed, &frame, &[Vec::new()]);
            assert_eq!(ed.loupe(&frame), None);
            // A mouse stroke: none.
            ed.update(&BuilderInput { pointer: Some(mid), pressed: true, held: true, ..Default::default() }, &frame);
            ed.update(&BuilderInput { pointer: Some(to), held: true, ..Default::default() }, &frame);
            assert_eq!(ed.loupe(&frame), None, "the mouse shows the cell it paints itself");
            ed.update(&BuilderInput { pointer: Some(to), ..Default::default() }, &frame);
            // A cell bigger than a fingertip and some: none.
            ed.camera.zoom_at(vp.scale_for_cell_mm(rules.loupe_cell_mm + 2.0), layout.to_field(frame.to_canvas(mid)), &vp, &rules);
            hold_stroke(&mut ed, &frame, mid, to);
            assert_eq!(ed.loupe(&frame), None);
            hold_fingers(&mut ed, &frame, &[Vec::new()]);
            // By the canvas's right edge the loupe stands left of the finger.
            ed.camera.zoom_at(vp.scale_for_cell_mm(8.0), layout.to_field(frame.to_canvas(mid)), &vp, &rules);
            let edge = Vec2::new(c.x + c.width - 20.0, finger.y);
            let from = frame.ui.to_window(Vec2::new(edge.x - 100.0, edge.y));
            hold_stroke(&mut ed, &frame, from, frame.ui.to_window(edge));
            let r = ed.loupe(&frame).expect("a loupe at the edge").rect;
            assert!(r.x + r.width <= edge.x - rules.loupe_lift_pt + 1.0 / device + 1e-3, "{ui:?}: left of the finger: {r:?} for {edge:?}");
            hold_fingers(&mut ed, &frame, &[Vec::new()]);
        }
    }

    /// The standard arena on a screen where its cells are 7.6 mm on the
    /// glass: one finger paints there.
    fn touch_arena() -> (MapEditor, BuilderFrame) {
        let frame = arena();
        let mut ed = MapEditor::new(MapFile::new());
        let screen = CanvasScreen { device_per_px: 3.0, points_per_px: 1.5, coarse: false };
        ed.update(&BuilderInput { screen: Some(screen), ..Default::default() }, &frame);
        (ed, frame)
    }

    /// A finger resting on the canvas, even wobbling inside the slop,
    /// paints nothing; a quick tap paints its cell, one undo step.
    #[test]
    fn a_resting_finger_paints_nothing_and_a_tap_paints_its_cell() {
        let (mut ed, frame) = touch_arena();
        let Vec2 { x, y } = on_cell(&ed, &frame, 10, 5);
        let rest: Vec<Vec<(i32, f32, f32)>> = (0..60).map(|i| vec![(7, x + (i % 4) as f32, y)]).collect();
        fingers(&mut ed, &frame, &rest);
        assert!(ed.map().cells.is_empty(), "a resting finger painted");
        assert_eq!(ed.history().undo_depth(), 0);
        fingers(&mut ed, &frame, &[vec![(8, x, y)], vec![(8, x + 2.0, y + 1.0)]]);
        assert_eq!(ed.map().cell(10, 5), Some(&brick()));
        assert_eq!(ed.history().undo_depth(), 1);
        // The toggle-erase rule holds for a tap too.
        fingers(&mut ed, &frame, &[vec![(9, x, y)]]);
        assert_eq!(ed.map().cell(10, 5), None);
    }

    /// Past the slop one finger strokes from where it landed, every cell
    /// it crosses, as one undo step.
    #[test]
    fn one_finger_past_the_slop_strokes_from_where_it_landed() {
        let (mut ed, frame) = touch_arena();
        let Vec2 { x: x0, y } = on_cell(&ed, &frame, 4, 8);
        let x1 = on_cell(&ed, &frame, 9, 8).x;
        let frames: Vec<Vec<(i32, f32, f32)>> = (0..=10).map(|i| vec![(1, x0 + (x1 - x0) * i as f32 / 10.0, y)]).collect();
        fingers(&mut ed, &frame, &frames);
        for col in 4..=9 {
            assert_eq!(ed.map().cell(col, 8), Some(&brick()), "col {col}");
        }
        assert_eq!(ed.map().cells.len(), 6);
        assert_eq!(ed.history().undo_depth(), 1);
    }

    /// Two fingers pinch about their middle - the world under it stays -
    /// and move together to pan; neither paints. On a coarse screen the
    /// zoom settles on whole blocks when they lift.
    #[test]
    fn two_fingers_pinch_at_their_middle_and_pan() {
        let (mut ed, frame) = big_editor((1180.0, 820.0), 2.0, true);
        let mid = canvas_middle(&frame);
        let under = ed.world_at(mid, &frame).unwrap();
        let spread = |d: f32| vec![(1, mid.x - d, mid.y), (2, mid.x + d, mid.y)];
        let frames: Vec<Vec<(i32, f32, f32)>> = (0..=20).map(|i| spread(40.0 + 8.0 * i as f32)).collect();
        fingers(&mut ed, &frame, &frames);
        assert!(!ed.camera().is_fit(), "spread fingers zoom in");
        let vp = ed.viewport();
        let device = vp.device_scale(ed.camera().scale(&vp));
        assert!(((device * 2.0) - (device * 2.0).round()).abs() < 1e-3, "settled on whole blocks: {device}");
        let now = ed.world_at(mid, &frame).unwrap();
        assert!((now.x - under.x).abs() < 4.0 && (now.y - under.y).abs() < 4.0, "the middle stayed put: {under:?} -> {now:?}");
        // Together, sideways: a pan.
        let before = ed.camera().center(&vp);
        let frames: Vec<Vec<(i32, f32, f32)>> =
            (0..=10).map(|i| vec![(3, mid.x - 50.0 + 6.0 * i as f32, mid.y), (4, mid.x + 50.0 + 6.0 * i as f32, mid.y)]).collect();
        fingers(&mut ed, &frame, &frames);
        let after = ed.camera().center(&vp);
        assert!(after.x < before.x - 1.0, "dragging right shows what is to the left: {before:?} -> {after:?}");
        assert!(ed.map().cells.is_empty());
    }

    /// A two-finger tap undoes and a three-finger tap redoes, the fingers
    /// lifting in any order.
    #[test]
    fn a_two_finger_tap_undoes_and_a_three_finger_tap_redoes() {
        let (mut ed, frame) = touch_arena();
        ed.stroke(&[(3, 3), (4, 3)], false);
        ed.stroke(&[(6, 6)], false);
        assert_eq!(ed.history().undo_depth(), 2);
        let Vec2 { x, y } = on_cell(&ed, &frame, 15, 10);
        fingers(&mut ed, &frame, &[vec![(1, x, y)], vec![(1, x, y), (2, x + 80.0, y)], vec![(2, x + 80.0, y)]]);
        assert_eq!(ed.history().undo_depth(), 1, "two fingers undid");
        assert_eq!(ed.map().cell(6, 6), None);
        fingers(&mut ed, &frame, &[vec![(1, x, y), (2, x + 80.0, y)], vec![(1, x, y), (2, x + 80.0, y), (3, x + 40.0, y + 60.0)], vec![(3, x + 40.0, y + 60.0)]]);
        assert_eq!(ed.history().undo_depth(), 2, "three fingers redid");
        assert_eq!(ed.map().cell(6, 6), Some(&brick()));
        assert_eq!(ed.map().cells.len(), 3, "the taps painted nothing");
    }

    /// A second finger landing on a stroke takes the stroke back - no
    /// cell, no undo step - and the finger left behind paints nothing.
    #[test]
    fn a_second_finger_on_a_stroke_takes_it_back() {
        let (mut ed, frame) = touch_arena();
        let Vec2 { x: x0, y } = on_cell(&ed, &frame, 4, 8);
        let x1 = on_cell(&ed, &frame, 8, 8).x;
        fingers(
            &mut ed,
            &frame,
            &[vec![(1, x0, y)], vec![(1, x1, y)], vec![(1, x1, y), (2, x1 + 100.0, y)], vec![(1, x1 - 30.0, y), (2, x1 + 70.0, y)], vec![(2, x1 + 70.0, y)], vec![(2, x1 + 150.0, y + 64.0)]],
        );
        assert!(ed.map().cells.is_empty(), "{:?}", ed.map().cells);
        assert_eq!(ed.history().undo_depth(), 0);
    }

    /// Where a cell is under the paint threshold on the glass a tap zooms
    /// in on its point to a cell of `builder_tap_zoom_cell_mm`, and a drag
    /// pans; neither paints. A mouse paints at any size.
    #[test]
    fn under_the_paint_threshold_a_tap_zooms_in_and_a_drag_pans() {
        let rules = CanvasRules::of(&crate::tuning::Tuning::DEFAULT);
        let (mut ed, frame) = big_editor((852.0, 393.0), 3.0, false);
        let vp = ed.viewport();
        assert!(vp.cell_mm(ed.camera().scale(&vp)) < rules.paint_min_cell_mm, "the study map at FIT on a phone is too small to paint");
        let at = canvas_at(&frame, 400.0, 200.0);
        let under = ed.world_at(at, &frame).unwrap();
        fingers(&mut ed, &frame, &[vec![(1, at.x, at.y)]]);
        assert!(ed.map().cells.is_empty());
        let mm = vp.cell_mm(ed.camera().scale(&vp));
        assert!((mm - rules.tap_zoom_cell_mm).abs() < 0.01, "a fine screen goes to the tap zoom exactly: {mm}");
        let now = ed.world_at(at, &frame).unwrap();
        assert!((now.x - under.x).abs() < 2.0 && (now.y - under.y).abs() < 2.0, "about the tapped point");
        // At 4 mm a finger cannot paint: a drag pans.
        ed.camera.zoom_at(vp.scale_for_cell_mm(4.0), frame.layout.to_field(frame.to_canvas(at)), &vp, &rules);
        let before = ed.camera().center(&vp);
        let frames: Vec<Vec<(i32, f32, f32)>> = (0..=10).map(|i| vec![(2, at.x - 8.0 * i as f32, at.y)]).collect();
        fingers(&mut ed, &frame, &frames);
        assert!(ed.camera().center(&vp).x > before.x + 1.0, "dragging left shows more to the right");
        assert!(ed.map().cells.is_empty());
        // The mouse is not held to it.
        ed.update(&BuilderInput { pointer: Some(at), pressed: true, held: true, ..Default::default() }, &frame);
        ed.update(&BuilderInput { pointer: Some(at), ..Default::default() }, &frame);
        assert_eq!(ed.map().cells.len(), 1);
    }

    /// A finger that lands on the bar presses its button and is no part
    /// of a gesture: dragged onto the canvas it paints nothing.
    #[test]
    fn a_finger_on_the_bar_presses_it_and_never_paints() {
        let (mut ed, frame) = touch_arena();
        let map = center(named(&ed, &frame, "map"));
        let Vec2 { x, y } = on_cell(&ed, &frame, 10, 8);
        fingers(&mut ed, &frame, &[vec![(1, map.x, map.y)], vec![(1, x, y)], vec![(1, x + 60.0, y)]]);
        assert_eq!(ed.open_menu(), Some("map"));
        assert!(ed.map().cells.is_empty());
        // With the popup open a finger on the canvas only closes it.
        fingers(&mut ed, &frame, &[vec![(2, x, y)], vec![(2, x + 60.0, y)]]);
        assert_eq!(ed.open_menu(), None);
        assert!(ed.map().cells.is_empty(), "the dismissing finger painted");
    }

    /// A bar button's hit rect reaches past the bar, over the canvas's top
    /// edge: a finger there presses the button and is no canvas finger. On
    /// a tablet zoomed in where a finger paints, a tap a few points under
    /// UNDO undoes once, lays nothing down, zooms nothing and leaves the
    /// redo branch whole; a tap on the select tool's strip acts and keeps
    /// the selection.
    #[test]
    fn a_tap_under_a_bar_or_strip_button_presses_it_and_never_paints() {
        use crate::EDITOR_BAR_HIT_SLACK;
        let ui = UiFrame::new((1180.0, 820.0), 1.0, 1.0, Insets::default(), true);
        let mut map = MapFile::new();
        map.size = Some((96.0, 54.0));
        let frame = BuilderFrame::new(ui, map.field_size(), MapClass::Field, None);
        let mut ed = MapEditor::new(map);
        let screen = CanvasScreen { device_per_px: frame.view.scale * 2.0, points_per_px: frame.view.scale, coarse: false };
        ed.update(&BuilderInput { screen: Some(screen), ..Default::default() }, &frame);
        let rules = CanvasRules::of(&crate::tuning::Tuning::DEFAULT);
        let vp = ed.viewport();
        ed.camera.zoom_at(vp.scale_for_cell_mm(rules.paint_min_cell_mm * 1.5), Vec2::new(vp.area.0 / 2.0, vp.area.1 / 2.0), &vp, &rules);
        let (c, r) = ed.cell_at(canvas_middle(&frame), &frame).expect("the view's middle is on the map");
        ed.stroke(&[(c, r), (c + 1, r)], false);
        ed.stroke(&[(c + 2, r + 2)], false);
        ed.undo();
        let (before, camera) = (ed.map().clone(), *ed.camera());
        let undo = named(&ed, &frame, "undo");
        let at = Vec2::new(undo.x + undo.width / 2.0, undo.y + undo.height + EDITOR_BAR_HIT_SLACK - 2.0);
        assert!(ed.world_at(at, &frame).is_some(), "the slack under UNDO stands over the canvas");
        fingers(&mut ed, &frame, &[vec![(1, at.x, at.y)]]);
        assert_eq!((ed.history().undo_depth(), ed.history().redo_depth()), (0, 2), "undone once, the redo branch whole");
        assert!(ed.map().cells.is_empty(), "the tap laid a cell down: {:?}", ed.map().cells);
        assert_eq!(*ed.camera(), camera, "the tap moved the view");
        ed.redo();
        assert_eq!(ed.map().cells, before.cells);

        // The strip: a finger on COPY's lowest points copies.
        ed.select_tool(Tool::Select);
        let (a, b) = (on_cell(&ed, &frame, c - 1, r - 1), on_cell(&ed, &frame, c + 2, r + 1));
        drag(&mut ed, &frame, a, b);
        let selection = ed.selection();
        assert!(selection.is_some());
        let copy = named(&ed, &frame, "sel_copy");
        fingers(&mut ed, &frame, &[vec![(2, copy.x + copy.width / 2.0, copy.y + copy.height - 1.0)]]);
        assert!(ed.clipboard().is_some(), "COPY took nothing");
        assert_eq!(ed.selection(), selection, "the tap let the selection go");
        assert_eq!(ed.history().undo_depth(), 1);
        assert_eq!(ed.map().cells, before.cells);
    }

    /// On a build with a keyboard the chrome turns to thumbs on the frame
    /// the session's first finger lands (`TouchScheme::touch_chrome`, as
    /// `app.rs` decides it before the builder's update), not a frame later:
    /// the bar has the height it keeps until the finger lifts, the canvas
    /// stands still under it, and the first tap is a tap - under the paint
    /// threshold it zooms in rather than reading as a drag.
    #[test]
    fn the_first_finger_on_a_keyboard_build_lands_on_a_touch_frame_and_taps() {
        let rules = CanvasRules::of(&crate::tuning::Tuning::DEFAULT);
        let mut map = MapFile::new();
        map.size = Some((96.0, 54.0));
        let field = map.field_size();
        let frame_for = |touch: bool| BuilderFrame::new(UiFrame::new((852.0, 393.0), 1.0, 1.0, Insets::default(), touch), field, MapClass::Field, None);
        let mut ed = MapEditor::new(map);
        let mut scheme = crate::touch::TouchScheme::default();
        assert!(!scheme.touch_chrome(true, false, false), "no finger yet: the mouse's chrome");
        let mouse = frame_for(false);
        let screen = CanvasScreen { device_per_px: mouse.view.scale * 3.0, points_per_px: mouse.view.scale, coarse: false };
        ed.update(&BuilderInput { screen: Some(screen), ..Default::default() }, &mouse);
        let vp = ed.viewport();
        assert!(vp.cell_mm(ed.camera().scale(&vp)) < rules.paint_min_cell_mm, "a tap here zooms in");
        let at = canvas_at(&mouse, 400.0, 200.0);
        let mut down = false;
        for touching in [true, true, true, false] {
            let touch = scheme.touch_chrome(true, false, touching);
            assert!(touch, "touching {touching}: the frame is laid out for the finger from its first");
            let frame = frame_for(touch);
            let touches = if touching { vec![crate::touch::TouchPoint { id: 4, pos: at }] } else { Vec::new() };
            let input = BuilderInput { pointer: Some(at), pressed: touching && !down, held: touching, touches: touches.clone(), dt: 1.0 / 60.0, ..Default::default() };
            ed.update(&input, &frame);
            scheme.update(&touches, crate::Rect::new(0.0, 0.0, 852.0, 393.0), false, 1.0 / 60.0);
            down = touching;
        }
        let vp = ed.viewport();
        let mm = vp.cell_mm(ed.camera().scale(&vp));
        assert!((mm - rules.tap_zoom_cell_mm).abs() < 0.01, "the first tap zoomed in like any other: {mm}");
        assert!(ed.map().cells.is_empty());
    }

    /// A stroke held at the canvas's edge scrolls the view toward it and
    /// keeps painting into the cells that come under the pointer - with a
    /// mouse and with a finger, and with a mouse held past the edge, where
    /// it paints under the nearest point of the canvas - and stops at the
    /// field's edge; away from the edge nothing scrolls.
    #[test]
    fn a_stroke_held_at_the_canvas_edge_scrolls_and_keeps_painting() {
        for (finger, past) in [(false, false), (true, false), (false, true)] {
            let (mut ed, frame) = big_editor((1180.0, 820.0), 2.0, false);
            let vp = ed.viewport();
            let rules = CanvasRules::of(&crate::tuning::Tuning::DEFAULT);
            ed.camera.zoom_at(vp.scale_for_cell_mm(12.0), Vec2::new(200.0, 300.0), &vp, &rules);
            let f = frame.layout.field;
            let start = canvas_at(&frame, 200.0, 300.0);
            let row_y = start.y;
            let edge = canvas_at(&frame, f.w + if past { 40.0 } else { -4.0 }, 300.0);
            let shown_edge = canvas_at(&frame, f.w - 4.0, 300.0);
            let (first, row) = ed.cell_at(start, &frame).unwrap();
            let center = ed.camera().center(&vp);
            let shown_right = ed.cell_at(shown_edge, &frame).unwrap().0;
            let frames = 90;
            if finger {
                let mut f: Vec<Vec<(i32, f32, f32)>> = (0..=8).map(|i| vec![(1, start.x + (edge.x - start.x) * i as f32 / 8.0, row_y)]).collect();
                f.extend((0..frames).map(|_| vec![(1, edge.x, row_y)]));
                fingers(&mut ed, &frame, &f);
            } else {
                ed.update(&BuilderInput { pointer: Some(start), pressed: true, held: true, dt: 1.0 / 60.0, ..Default::default() }, &frame);
                for _ in 0..frames {
                    ed.update(&BuilderInput { pointer: Some(edge), held: true, dt: 1.0 / 60.0, ..Default::default() }, &frame);
                }
                ed.update(&BuilderInput { pointer: Some(edge), dt: 1.0 / 60.0, ..Default::default() }, &frame);
            }
            let moved = ed.camera().center(&vp);
            assert!(moved.x > center.x + 64.0 && (moved.y - center.y).abs() < 1e-3, "finger={finger} past={past}: {center:?} -> {moved:?}");
            let painted_right = (0..96).rev().find(|&c| ed.map().cell(c, row).is_some()).unwrap();
            assert!(painted_right > shown_right + 1, "finger={finger} past={past}: the stroke went on past the first view's edge ({painted_right} vs {shown_right})");
            assert!((first..=painted_right).all(|c| ed.map().cell(c, row).is_some()), "finger={finger} past={past}: one unbroken row from {first}");
            assert_eq!(ed.map().cells.len() as i32, painted_right - first + 1, "finger={finger} past={past}: on that row alone");
            assert_eq!(ed.history().undo_depth(), 1, "finger={finger} past={past}: one stroke, one step");
        }
        // Held in the middle of the canvas nothing scrolls.
        let (mut ed, frame) = big_editor((1180.0, 820.0), 2.0, false);
        let vp = ed.viewport();
        ed.camera.zoom_at(vp.fit_scale() * 4.0, Vec2::new(500.0, 300.0), &vp, &CanvasRules::of(&crate::tuning::Tuning::DEFAULT));
        let center = ed.camera().center(&vp);
        let mid = canvas_middle(&frame);
        ed.update(&BuilderInput { pointer: Some(mid), pressed: true, held: true, dt: 1.0 / 60.0, ..Default::default() }, &frame);
        for _ in 0..30 {
            ed.update(&BuilderInput { pointer: Some(mid), held: true, dt: 1.0 / 60.0, ..Default::default() }, &frame);
        }
        assert_eq!(ed.camera().center(&vp), center);
    }

    /// Each anchor keeps its edge or shares the change, in whole cells, and
    /// a run of one-cell steps about the middle comes out centred.
    #[test]
    fn an_anchor_keeps_its_edge_and_the_middle_shares_the_change() {
        let grow = |a: Anchor| a.shift((34.0, 17.0), (40.0, 21.0));
        assert_eq!(grow(Anchor::TopLeft), (0, 0));
        assert_eq!(grow(Anchor::Top), (3, 0));
        assert_eq!(grow(Anchor::TopRight), (6, 0));
        assert_eq!(grow(Anchor::Left), (0, 2));
        assert_eq!(grow(Anchor::Center), (3, 2));
        assert_eq!(grow(Anchor::Right), (6, 2));
        assert_eq!(grow(Anchor::BottomLeft), (0, 4));
        assert_eq!(grow(Anchor::Bottom), (3, 4));
        assert_eq!(grow(Anchor::BottomRight), (6, 4));
        assert_eq!(Anchor::BottomRight.shift((40.0, 22.5), (34.0, 17.0)), (-6, -5), "a half row steps onto the whole ones");
        let mut total = 0;
        for cols in 34..44 {
            total += Anchor::Center.shift((cols as f32, 17.0), (cols as f32 + 1.0, 17.0)).0;
        }
        assert_eq!(total, 5, "ten one-cell steps about the middle move the map five cells");
        for a in Anchor::ALL {
            assert_eq!(Anchor::parse(a.name()), Some(a));
        }
    }

    /// A resize with every anchor places the old map at it, drops the
    /// cells that land past the new edge, makes the ground on the new
    /// field, and one undo brings the map back whole; redo repeats it.
    #[test]
    fn a_resize_with_every_anchor_places_the_map_and_undo_brings_it_back() {
        let mut base = MapFile::new();
        let corners = [(0, 0), (33, 0), (0, 16), (33, 16), (17, 8)];
        for (i, &(c, r)) in corners.iter().enumerate() {
            base.set_cell(c, r, if i == 4 { CellObject::Frog } else { brick() });
        }
        base.set_cell(34, 17, CellObject::Road);
        for anchor in Anchor::ALL {
            for (cols, rows) in [(24.0, 12.0), (40.0, 21.0)] {
                let mut ed = MapEditor::new(base.clone());
                ed.set_resize_anchor(anchor);
                assert!(ed.resize(cols, rows, anchor));
                assert_eq!(ed.size_cells(), (cols, rows));
                let (dx, dy) = anchor.shift((34.0, 17.0), (cols, rows));
                for (c, r, obj) in base.iter_cells() {
                    let (nc, nr) = (c + dx, r + dy);
                    let inside = nc >= 0 && nr >= 0 && nc as f32 <= cols && nr as f32 <= rows;
                    assert_eq!(ed.map().cell(nc, nr), inside.then_some(obj), "{anchor:?} to {cols} x {rows}: {c},{r} -> {nc},{nr}");
                }
                let kept = base.iter_cells().filter(|&(c, r, _)| {
                    let (nc, nr) = (c + dx, r + dy);
                    nc >= 0 && nr >= 0 && nc as f32 <= cols && nr as f32 <= rows
                });
                assert_eq!(ed.map().cells.len(), kept.count());
                assert_eq!(ed.ground().cols, cols as usize + 1, "the ground is the new field's");
                assert!(ed.dirty() && ed.diff().settings.contains(&"size"), "{:?}", ed.diff());
                assert_eq!(ed.history().undo_depth(), 1);
                let after = ed.map().clone();
                ed.undo();
                assert_eq!(ed.map(), &base, "{anchor:?}: undo brings every cell back");
                assert!(!ed.dirty());
                ed.redo();
                assert_eq!(ed.map(), &after);
            }
        }
    }

    /// The panel's WIDTH and HEIGHT steppers resize a cell a press about
    /// the ANCHOR, a run of presses one undo step while the panel stays
    /// open, held between 16 x 9 and the largest map; a zoomed view moves
    /// with the map.
    #[test]
    fn the_size_steppers_resize_a_cell_a_press_as_one_step() {
        let frame = arena();
        let mut ed = MapEditor::new(MapFile::new());
        ed.stroke(&[(10, 5)], false);
        press_named(&mut ed, &frame, "map");
        assert_eq!(ed.open_menu(), Some("map"));
        let inc = |r: SettingsRow| format!("{}_inc", r.name());
        let dec = |r: SettingsRow| format!("{}_dec", r.name());
        // The anchor walks the nine, from the middle.
        assert_eq!(ed.resize_anchor(), Anchor::Center);
        press_named(&mut ed, &frame, &inc(SettingsRow::Anchor));
        assert_eq!(ed.resize_anchor(), Anchor::Right);
        press_named(&mut ed, &frame, &dec(SettingsRow::Anchor));
        press_named(&mut ed, &frame, &dec(SettingsRow::Anchor));
        assert_eq!(ed.resize_anchor(), Anchor::Left);
        let depth = ed.history().undo_depth();
        for _ in 0..4 {
            press_named(&mut ed, &frame, &inc(SettingsRow::Width));
        }
        press_named(&mut ed, &frame, &dec(SettingsRow::Height));
        assert_eq!(ed.size_cells(), (38.0, 16.0));
        assert_eq!(ed.map().cell(10, 5), Some(&brick()), "left anchor: the map stays at the left, 17 to 16 rows round the same middle row");
        press_named(&mut ed, &frame, &dec(SettingsRow::Height));
        assert_eq!(ed.map().cell(10, 4), Some(&brick()), "and the next row off moves it up one");
        press_named(&mut ed, &frame, &inc(SettingsRow::Height));
        assert_eq!(ed.history().undo_depth(), depth + 1, "five presses, one step");
        // Closing the panel ends the run.
        ed.update(&BuilderInput { escape: true, ..Default::default() }, &frame);
        ed.update(&BuilderInput::default(), &frame);
        press_named(&mut ed, &frame, "map");
        press_named(&mut ed, &frame, &inc(SettingsRow::Height));
        assert_eq!(ed.history().undo_depth(), depth + 2);
        ed.undo();
        ed.undo();
        assert_eq!(ed.size_cells(), (34.0, 17.0));
        assert_eq!(ed.map().cell(10, 5), Some(&brick()));
        // The limits hold.
        assert!(!ed.resize(5.0, 3.0, Anchor::Center) || ed.size_cells() == MIN_MAP_CELLS);
        ed.resize(5.0, 3.0, Anchor::Center);
        assert_eq!(ed.size_cells(), MIN_MAP_CELLS);
        ed.resize(1000.0, 1000.0, Anchor::TopLeft);
        assert_eq!(ed.size_cells(), (map::MAX_SIDE_CELLS, map::MAX_SIDE_CELLS));
        // A zoomed view moves with the map.
        let (mut ed, frame) = big_editor((1180.0, 820.0), 2.0, false);
        let vp = ed.viewport();
        ed.camera.zoom_at(vp.fit_scale() * 4.0, Vec2::new(500.0, 300.0), &vp, &CanvasRules::of(&crate::tuning::Tuning::DEFAULT));
        let before = ed.camera().center(&ed.viewport());
        ed.resize(100.0, 54.0, Anchor::Right);
        let after = ed.camera().center(&ed.viewport());
        assert!((after.x - before.x - 4.0 * 32.0).abs() < 1e-3 && (after.y - before.y).abs() < 1e-3, "{before:?} -> {after:?}");
        ed.update(&BuilderInput::default(), &frame);
        ed.undo();
        let back = ed.camera().center(&ed.viewport());
        assert!((back.x - before.x).abs() < 1e-3, "undo moves it back: {before:?} -> {back:?}");
    }

    /// A cell edit repaints the ground the whole map would make: strokes
    /// of wall, road, water and the eraser across the study map's water,
    /// one taken back by a second finger, undo and redo - each against the
    /// ground built afresh from the map with the same seed.
    #[test]
    fn a_stroke_repaints_the_ground_a_rebuild_would_make() {
        let map = MapFile::load(std::path::Path::new("maps/study/frontier.toml")).expect("the study map");
        let mut ed = MapEditor::new(map);
        let _ = ed.ground().shade();
        let check = |ed: &MapEditor, what: &str| {
            let mut fresh = MapEditor::new(ed.map().clone());
            fresh.ground_seed = ed.ground_seed;
            fresh.rebuild_ground();
            assert!(ed.ground().draws_like(fresh.ground()), "{what}");
        };
        let water: Vec<(i32, i32)> = ed.map().iter_cells().filter(|(_, _, obj)| matches!(obj, CellObject::Water)).map(|(c, r, _)| (c, r)).collect();
        assert!(water.len() > 20, "the study map has a lake and a river");
        let (wc, wr) = water[water.len() / 2];
        let strokes = [
            (Tool::Wall(Material::Brick), (wc - 6, wr), (wc + 6, wr)),
            (Tool::Road, (wc, wr - 6), (wc, wr + 6)),
            (Tool::Water, (wc - 5, wr - 5), (wc + 5, wr + 5)),
            (Tool::Eraser, (wc - 4, wr + 1), (wc + 4, wr + 1)),
        ];
        for (tool, from, to) in strokes {
            ed.select_tool(tool);
            ed.begin_stroke(from, false);
            ed.drag_to(to);
            check(&ed, tool.name());
            ed.finish_stroke();
        }
        ed.select_tool(Tool::Water);
        ed.begin_stroke((wc + 2, wr - 3), false);
        ed.drag_to((wc + 8, wr - 3));
        ed.cancel_stroke();
        check(&ed, "a stroke taken back");
        for (way, what) in [(-1, "undo"), (-1, "a second undo"), (1, "redo")] {
            if way < 0 {
                ed.undo();
            } else {
                ed.redo();
            }
            check(&ed, what);
        }
    }

    /// What a drag across the 96 x 54 study map costs, a cell a frame: the
    /// cell painted, then the floor shade the next frame draws, and the
    /// texels the frame uploads to catch its copy of the shade up - for a
    /// wall, a road and a river stroke, and the undo of each. Prints; run
    /// with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn a_stroke_across_the_study_map_timing() {
        let map = MapFile::load(std::path::Path::new("maps/study/frontier.toml")).expect("the study map");
        let mut ed = MapEditor::new(map);
        let (cols, _) = ed.size_cells();
        let cols = cols as i32;
        for (tool, row) in [(Tool::Wall(Material::Brick), 27), (Tool::Road, 20), (Tool::Water, 33)] {
            let _ = ed.ground().shade();
            ed.select_tool(tool);
            let mut held = ed.ground().shade().stamp;
            let mut paint = std::time::Duration::ZERO;
            let mut shade = std::time::Duration::ZERO;
            let mut texels = 0usize;
            let start = std::time::Instant::now();
            for col in 0..cols {
                let t = std::time::Instant::now();
                if col == 0 {
                    ed.begin_stroke((col, row), false);
                } else {
                    ed.drag_to((col, row));
                }
                paint += t.elapsed();
                let t = std::time::Instant::now();
                let image = ed.ground().shade();
                shade += t.elapsed();
                texels += uploaded(image, held);
                held = image.stamp;
            }
            ed.finish_stroke();
            let total = start.elapsed();
            let t = std::time::Instant::now();
            ed.undo();
            let image = ed.ground().shade();
            let undo = t.elapsed();
            let undo_texels = uploaded(image, held);
            let n = cols as f64;
            eprintln!(
                "{:>6} stroke of {cols} cells on 96 x 54: {:.2} ms a cell (paint {:.2} ms, shade {:.2} ms), {:.0} texels uploaded a cell; undo {:.1} ms, {undo_texels} texels",
                tool.name(),
                total.as_secs_f64() * 1e3 / n,
                paint.as_secs_f64() * 1e3 / n,
                shade.as_secs_f64() * 1e3 / n,
                texels as f64 / n,
                undo.as_secs_f64() * 1e3,
            );
        }

        /// The texels a copy of the shade holding `held` uploads to show
        /// `image` (`render::canvas::BlockTexture::sync`).
        fn uploaded(image: &crate::canvas::BlockImage, held: u64) -> usize {
            if image.stamp == held {
                0
            } else {
                image.changed_since(held).map_or(image.width * image.height, |p| p.width * p.height)
            }
        }
    }

    /// What a large RECT or FILL costs - the cells laid, the ground and the
    /// navigator repainted, the floor shade the next frame draws - against
    /// making the whole ground again, on the study map, longwater and an
    /// empty 250 x 250 map: what `repaint_ground`'s switch to a rebuild and
    /// `builder_fill_max_cells` are set from. Prints; run with
    /// `--ignored --nocapture` (and `--release` for a phone's order).
    #[test]
    #[ignore]
    fn a_large_fill_timing() {
        let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1e3;
        let mut empty = MapFile::default();
        empty.size = Some((250.0, 250.0));
        let maps = [
            ("study 96 x 54", MapFile::load(std::path::Path::new("maps/study/frontier.toml")).expect("the study map")),
            ("longwater 112 x 63", map::open_map("longwater").expect("longwater")),
            ("empty 250 x 250", empty),
        ];
        for (name, map) in maps {
            let t = std::time::Instant::now();
            let mut ed = MapEditor::new(map);
            let _ = ed.ground().shade();
            let open = ms(t);
            let t = std::time::Instant::now();
            ed.rebuild_ground();
            let _ = ed.ground().shade();
            eprintln!("{name}: open {open:.1} ms, rebuild {:.1} ms", ms(t));
            let (cols, rows) = ed.size_cells();
            let (cols, rows) = (cols as i32, rows as i32);
            ed.set_shape(Shape::Rect);
            for (tool, label) in [(Tool::Water, "water"), (Tool::Wall(Material::Brick), "brick"), (Tool::TallGrass, "grass")] {
                ed.select_tool(tool);
                for side in [16, 32, 64, 128, 250] {
                    if side > cols.max(rows) && side != 250 {
                        continue;
                    }
                    let (w, h) = (side.min(cols), side.min(rows));
                    let (c0, r0) = ((cols - w) / 2, (rows - h) / 2);
                    let t = std::time::Instant::now();
                    let changes = ed.stroke(&[(c0, r0), (c0 + w - 1, r0 + h - 1)], false);
                    let lay = ms(t);
                    let t = std::time::Instant::now();
                    let _ = ed.ground().shade();
                    let shade = ms(t);
                    let t = std::time::Instant::now();
                    ed.undo();
                    let _ = ed.ground().shade();
                    let undo = ms(t);
                    eprintln!("  rect {label:>5} {w:>3} x {h:<3} {:>6} cells: lay {lay:.1} ms, shade {shade:.1} ms, undo {undo:.1} ms", changes.len());
                    if w == cols && h == rows {
                        break;
                    }
                }
            }
            let bounds = ed.field_cells();
            for max in [4096, 16384, 70000] {
                let t = std::time::Instant::now();
                let found = brush::flood(ed.map(), bounds, (cols / 2, rows / 2), max).map(|c| c.len());
                eprintln!("  flood from the middle, at most {max}: {found:?} in {:.1} ms", ms(t));
            }
        }
    }

    /// A 250 x 250 map painted edge to edge - each cell a wall, a prop, a
    /// tree, grass, road, water, oil or a pickup - with a start, a frog, two
    /// portals and a tower among them.
    fn dense_map() -> MapFile {
        let mut map = MapFile::new();
        map.size = Some((250.0, 250.0));
        let kinds = [
            brick(),
            CellObject::Wall { material: Material::Iron },
            CellObject::Wall { material: Material::Wood },
            CellObject::Sandbag,
            CellObject::Barrel { drum: None },
            CellObject::Fence,
            CellObject::Tree,
            CellObject::Pine,
            CellObject::TallGrass,
            CellObject::Road,
            CellObject::Water,
            CellObject::Oil,
            CellObject::Pickup { pickup: PickupKind::Health },
        ];
        for row in 0..250 {
            for col in 0..250 {
                map.set_cell(col, row, kinds[((col * 7 + row * 13 + col * row) % kinds.len() as i32) as usize]);
            }
        }
        map.set_cell(10, 10, CellObject::Start);
        map.set_cell(240, 240, CellObject::Frog);
        map.set_cell(100, 100, CellObject::Portal);
        map.set_cell(150, 150, CellObject::Portal);
        map.set_cell(120, 60, CellObject::Tesla { side: None });
        map
    }

    /// What a frame of the builder reads of the map, on `dense_map`: the
    /// name's edited mark twice (the status line and the bar), the canvas's
    /// portals and its cells (every one at FIT, those of the rows a phone's
    /// zoomed view spans otherwise) and the ACTOR button's badge - walked
    /// out of `MapFile`, and read from the kept index; and the frame after
    /// a cell painted (the index takes it in) and after a whole-map edit
    /// (the index is worked out again). Prints; run with `--ignored
    /// --nocapture` (and `--release` for a phone's order).
    #[test]
    #[ignore]
    fn a_dense_maps_frame_timing() {
        let ms = |t: std::time::Instant, n: usize| t.elapsed().as_secs_f64() * 1e3 / n as f64;
        let t = std::time::Instant::now();
        let mut ed = MapEditor::new(dense_map());
        eprintln!("dense 250 x 250, {} cells: opened in {:.0} ms", ed.map().cells.len(), ms(t, 1));
        // What the clear flag works out once after an edit, while the
        // session has cleared a revision of the canvas.
        let t = std::time::Instant::now();
        std::hint::black_box(ed.map().revision());
        eprintln!("  its revision: {:.1} ms", ms(t, 1));
        // The clear flag after an edit with that revision cleared: the key
        // rules out a canvas no cleared revision can be, and only a key
        // that matches - the cell put back - works the revision out.
        let revision = ed.revision();
        ed.note_clear(revision, 60.0);
        let was = ed.map().cell(10, 10).copied();
        let other = if was == Some(CellObject::Fence) { CellObject::Sandbag } else { CellObject::Fence };
        let mut changes = Vec::new();
        ed.change_cell(10, 10, Some(other), &mut changes);
        ed.cells_changed(&changes);
        let t = std::time::Instant::now();
        assert_eq!(ed.par(), None);
        let away = ms(t, 1);
        let mut changes = Vec::new();
        ed.change_cell(10, 10, was, &mut changes);
        ed.cells_changed(&changes);
        let t = std::time::Instant::now();
        assert_eq!(ed.par(), Some(60.0));
        eprintln!("  the clear flag after an edit away: {away:.3} ms; after the cell is put back: {:.1} ms", ms(t, 1));
        let shown = |cull: Option<Rectangle>, col: i32, row: i32| {
            let pos = map::cell_to_world(col, row);
            cull.is_none_or(|r| pos.x >= r.x && pos.y >= r.y && pos.x <= r.x + r.width && pos.y <= r.y + r.height)
        };
        let frames = 20;
        let zoomed = Rectangle::new(110.0 * 32.0, 115.0 * 32.0, 40.0 * 32.0, 20.0 * 32.0);
        for (view, cull) in [("FIT", None), ("a zoomed 40 x 20 cells", Some(zoomed))] {
            let (map, baseline) = (ed.map().clone(), ed.baseline().clone());
            let walk = || {
                let mut seen = 0;
                for _ in 0..2 {
                    seen += !MapDiff::between(&baseline, &map).is_empty() as usize;
                }
                seen += map.portal_cells().len();
                seen += map.iter_cells().filter(|&(c, r, _)| shown(cull, c, r)).count();
                seen + map.start_cell().is_some() as usize
            };
            let read = |ed: &MapEditor| {
                let mut seen = 0;
                for _ in 0..2 {
                    seen += ed.dirty() as usize;
                }
                let index = ed.cell_index();
                seen += index.portals().len();
                seen += index.near(cull).filter(|&&(c, r, _)| shown(cull, c, r)).count();
                seen + ed.singleton_placed(Tool::Start) as usize
            };
            assert_eq!(walk(), read(&ed), "the index reads what the walks read");
            let t = std::time::Instant::now();
            for _ in 0..frames {
                std::hint::black_box(walk());
            }
            let walked = ms(t, frames);
            let t = std::time::Instant::now();
            for _ in 0..frames {
                std::hint::black_box(read(&ed));
            }
            let kept = ms(t, frames);
            let t = std::time::Instant::now();
            for _ in 0..frames {
                ed.map_changed();
                std::hint::black_box(read(&ed));
            }
            let remade = ms(t, frames);
            // A stroke's frame: a cell painted and taken in.
            let t = std::time::Instant::now();
            for i in 0..frames {
                let mut changes = Vec::new();
                let obj = if i % 2 == 0 { CellObject::Fence } else { CellObject::Sandbag };
                ed.change_cell(120 + i as i32, 125, Some(obj), &mut changes);
                ed.cells_changed(&changes);
                std::hint::black_box(read(&ed));
            }
            let painted = ms(t, frames);
            assert_eq!(ed.cell_index(), &CellIndex::of(ed.map()));
            eprintln!(
                "  {view}: walked {walked:.2} ms a frame; kept {kept:.3} ms a frame, {painted:.3} ms on a stroke's frame, {remade:.2} ms on the frame after a whole-map edit"
            );
        }
    }

    /// The kept index is the map's own after every kind of edit, whether it
    /// took the edit in or was worked out again: strokes, a rectangle, a
    /// fill and a scatter, a singleton moved, a selection lifted and
    /// carried, a paste, a flip, a delete, a quick fix, a stroke taken
    /// back by a second finger, undo and redo, a clear and a load.
    #[test]
    fn the_kept_index_follows_every_edit() {
        let (mut ed, frame) = touch_arena();
        ed.load(select_map());
        let steps: Vec<(&str, Box<dyn Fn(&mut MapEditor)>)> = vec![
            ("a stroke", Box::new(|ed| drop(ed.stroke(&[(10, 10), (14, 10)], false)))),
            ("an erasing stroke", Box::new(|ed| drop(ed.stroke(&[(10, 10), (12, 10)], false)))),
            (
                "the start moved",
                Box::new(|ed| {
                    ed.select_tool(Tool::Start);
                    drop(ed.stroke(&[(20, 5), (22, 5)], false));
                }),
            ),
            (
                "a rectangle",
                Box::new(|ed| {
                    ed.select_tool(Tool::Wall(Material::Wood));
                    ed.set_shape(Shape::Rect);
                    drop(ed.stroke(&[(2, 12), (6, 14)], false));
                }),
            ),
            (
                "a scatter",
                Box::new(|ed| {
                    ed.select_tool(Tool::Prop(Material::Tree));
                    ed.set_shape(Shape::Scatter);
                    drop(ed.stroke(&[(12, 3), (18, 6)], false));
                }),
            ),
            (
                "a selection carried",
                Box::new(|ed| {
                    ed.select_cells(CellRect::spanning((3, 3), (6, 6)));
                    drop(ed.move_selection((5, 1)));
                }),
            ),
            (
                "a paste",
                Box::new(|ed| {
                    ed.copy_selection();
                    ed.paste(Some((24, 12)));
                    drop(ed.place_ghost());
                }),
            ),
            ("a flip", Box::new(|ed| drop(ed.flip(Axis::Horizontal)))),
            ("a delete", Box::new(|ed| drop(ed.delete_selection()))),
            (
                "a quick fix",
                Box::new(|ed| {
                    let (col, row, _) = ed.cell_index().cells()[0];
                    assert!(ed.apply_fix(LintFix::Remove { at: (col, row) }));
                }),
            ),
            (
                "a fill",
                Box::new(|ed| {
                    ed.select_tool(Tool::Road);
                    ed.set_shape(Shape::Fill);
                    drop(ed.stroke(&[(30, 2)], false));
                }),
            ),
            ("an undo", Box::new(|ed| assert!(ed.undo()))),
            ("a redo", Box::new(|ed| assert!(ed.redo()))),
            ("a clear", Box::new(|ed| ed.clear())),
            ("the clear undone", Box::new(|ed| assert!(ed.undo()))),
            ("a load", Box::new(|ed| ed.load(select_map()))),
        ];
        for (what, edit) in &steps {
            let _ = ed.cell_index();
            edit(&mut ed);
            assert_eq!(ed.cell_index(), &CellIndex::of(ed.map()), "{what}");
        }
        // A second finger takes a stroke back, cell by cell.
        ed.select_tool(Tool::Wall(Material::Brick));
        ed.set_shape(Shape::Pen);
        let _ = ed.cell_index();
        let Vec2 { x: x0, y } = on_cell(&ed, &frame, 4, 8);
        let x1 = on_cell(&ed, &frame, 8, 8).x;
        hold_fingers(&mut ed, &frame, &[vec![(1, x0, y)], vec![(1, x1, y)]]);
        assert_eq!(ed.cell_index(), &CellIndex::of(ed.map()), "a stroke under way");
        hold_fingers(&mut ed, &frame, &[vec![(1, x1, y), (2, x1 + 100.0, y)]]);
        assert_eq!(ed.cell_index(), &CellIndex::of(ed.map()), "the stroke taken back");
    }

    /// The edited mark read from the kept indexes is `diff`'s: through
    /// strokes, an undo back to the baseline, a settings change, a resize,
    /// a load and a move of the baseline.
    #[test]
    fn the_kept_edited_mark_is_the_diffs() {
        let frame = arena();
        let mut ed = MapEditor::new(select_map());
        let check = |ed: &MapEditor, what: &str| assert_eq!(ed.dirty(), !ed.diff().is_empty(), "{what}");
        check(&ed, "fresh");
        assert!(!ed.dirty());
        ed.stroke(&[(10, 10), (12, 10)], false);
        check(&ed, "a stroke");
        assert!(ed.dirty());
        ed.undo();
        check(&ed, "undone back to the baseline");
        assert!(!ed.dirty());
        ed.redo();
        check(&ed, "redone");
        ed.stroke(&[(10, 10), (12, 10)], false);
        check(&ed, "erased again by the toggle");
        assert!(!ed.dirty());
        press_named(&mut ed, &frame, "map");
        ed.step_setting(SettingsRow::Theme, true);
        check(&ed, "a theme");
        assert!(ed.dirty());
        ed.undo();
        check(&ed, "the theme undone");
        ed.resize(30.0, 17.0, Anchor::TopLeft);
        check(&ed, "a resize");
        assert!(ed.dirty());
        ed.undo();
        check(&ed, "the resize undone");
        assert!(!ed.dirty());
        ed.stroke(&[(3, 3)], false);
        ed.set_baseline();
        check(&ed, "the baseline moved, as a save moves it");
        assert!(!ed.dirty());
        ed.load(MapFile::new());
        check(&ed, "a load");
        assert!(!ed.dirty());
        ed.undo();
        check(&ed, "the load undone");
        assert!(ed.dirty());
    }

    /// The Save prompt takes text and Enter; Esc cancels it, and so does a
    /// press outside it - a touch screen's Esc, which the prompt's hint
    /// names after a touch -, painting nothing; a press inside it is the
    /// prompt's.
    #[test]
    fn a_press_outside_the_save_prompt_cancels_it() {
        let frame = arena();
        let mut ed = MapEditor::new(MapFile::new());
        ed.popup = Some(Popup::Save { name: String::new() });
        ed.update(&BuilderInput { typed: "abc".into(), ..Default::default() }, &frame);
        let Some(PopupLayout::Save(panel)) = ed.chrome(&frame).popup else { panic!("the Save prompt") };
        click(&mut ed, &frame, frame.ui.to_window(center(panel)));
        assert!(matches!(&ed.popup, Some(Popup::Save { name }) if name == "abc"), "a press inside is the prompt's");
        let outside = on_cell(&ed, &frame, 2, 2);
        assert!(!panel.contains(frame.to_ui(outside)));
        click(&mut ed, &frame, outside);
        assert_eq!(ed.open_menu(), None, "a press outside cancels it");
        assert!(ed.map().cells.is_empty(), "and paints nothing");
        ed.popup = Some(Popup::Save { name: String::new() });
        ed.update(&BuilderInput { escape: true, ..Default::default() }, &frame);
        assert_eq!(ed.open_menu(), None, "Esc cancels it");
    }

    /// The Save prompt's SAVE button saves as Enter does - a touch
    /// screen's way, with no Enter to press: no button while the name is
    /// empty, and a press on it once one is typed saves and closes the
    /// prompt.
    #[test]
    fn the_save_prompts_button_saves_as_enter_does() {
        let ui = UiFrame::new((852.0, 393.0), 1.0, 1.0, Insets::default(), true);
        let frame = BuilderFrame::new(ui, (W, H), MapClass::Arena, None);
        let mut ed = MapEditor::new(MapFile::new());
        ed.popup = Some(Popup::Save { name: String::new() });
        assert!(!has_named(&ed, &frame, "save_confirm"), "no name, no button");
        let Some(PopupLayout::Save(panel)) = ed.chrome(&frame).popup else { panic!("the Save prompt") };
        click(&mut ed, &frame, frame.ui.to_window(center(chrome::save_button(panel, true))));
        assert_eq!(ed.open_menu(), Some("save"), "an empty name saves nothing");
        if !map::saving_available() {
            return;
        }
        let name = format!("zz-save-button-{}", std::process::id());
        ed.update(&BuilderInput { typed: name.clone(), ..Default::default() }, &frame);
        let at = center(named(&ed, &frame, "save_confirm"));
        click(&mut ed, &frame, at);
        let path = map::maps_dir().join(format!("{name}.toml"));
        let saved = path.exists();
        let _ = std::fs::remove_file(&path);
        assert!(saved, "SAVE wrote the map");
        assert_eq!(ed.open_menu(), None);
        assert_eq!(ed.name(), name);
    }

    #[test]
    fn opening_one_popup_closes_the_other_and_save_reports_itself() {
        let frame = arena();
        let mut ed = MapEditor::new(MapFile::new());
        press_named(&mut ed, &frame, "map");
        assert_eq!(ed.open_menu(), Some("map"));
        // A press on the ACTOR caret while the panel is open only closes
        // the panel; the next one opens the list.
        let caret = center(named(&ed, &frame, "list_actor"));
        click(&mut ed, &frame, caret);
        assert_eq!(ed.open_menu(), None);
        click(&mut ed, &frame, caret);
        assert_eq!(ed.open_menu(), Some("actor"));
        ed.popup = Some(Popup::Save { name: String::new() });
        assert_eq!(ed.open_menu(), Some("save"));
    }

    // ----- the select tool -----

    /// A map with a little of everything for the select tool: two bricks, a
    /// road, water, player 1's start and the frog in one corner of it, a
    /// gate on the left edge and a pair of portals.
    fn select_map() -> MapFile {
        let mut map = MapFile::new();
        map.set_cell(4, 4, brick());
        map.set_cell(5, 4, brick());
        map.set_cell(4, 5, CellObject::Road);
        map.set_cell(6, 6, CellObject::Water);
        map.set_cell(5, 6, CellObject::Start);
        map.set_cell(20, 10, CellObject::Frog);
        map.set_cell(0, 8, CellObject::Gate);
        map.set_cell(8, 4, CellObject::Portal);
        map.set_cell(25, 12, CellObject::Portal);
        map
    }

    /// The newest step undoes to exactly `before` and redoes to exactly
    /// the map as it stands.
    fn undo_redo_exact(ed: &mut MapEditor, before: &MapFile) {
        let after = ed.map().clone();
        ed.undo();
        assert_eq!(ed.map().cells, before.cells, "the undo is exact");
        ed.redo();
        assert_eq!(ed.map().cells, after.cells, "the redo is exact");
    }

    /// A mouse press at `from`, a held drag to `to` and a release there.
    fn drag(ed: &mut MapEditor, frame: &BuilderFrame, from: Vec2, to: Vec2) {
        ed.update(&BuilderInput { pointer: Some(from), pressed: true, held: true, ..Default::default() }, frame);
        for i in 1..=4 {
            let t = i as f32 / 4.0;
            let p = Vec2::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
            ed.update(&BuilderInput { pointer: Some(p), held: true, ..Default::default() }, frame);
        }
        ed.update(&BuilderInput { pointer: Some(to), ..Default::default() }, frame);
    }

    /// How many cells of the map hold `obj`.
    fn count(ed: &MapEditor, obj: CellObject) -> usize {
        ed.map().cells.values().filter(|o| **o == obj).count()
    }

    /// The ground the editor holds is the one its map would build afresh
    /// with the same seed.
    fn ground_is_rebuilt(ed: &MapEditor) {
        let mut fresh = MapEditor::new(ed.map().clone());
        fresh.ground_seed = ed.ground_seed;
        fresh.rebuild_ground();
        assert!(ed.ground().draws_like(fresh.ground()), "the ground repaints as a rebuild would make it");
    }

    #[test]
    fn a_mouse_drag_selects_a_rectangle_and_escape_a_press_outside_or_another_tool_lets_it_go() {
        let frame = arena();
        let mut ed = MapEditor::new(select_map());
        ed.select_tool(Tool::Select);
        assert_eq!(ed.tool(), Tool::Select);
        assert_eq!(ed.current_tool(Category::Wall), Tool::Wall(Material::Brick), "the categories keep their tools");
        let (a, b) = (on_cell(&ed, &frame, 6, 6), on_cell(&ed, &frame, 3, 3));
        drag(&mut ed, &frame, a, b);
        assert_eq!(ed.selection(), Some(CellRect::spanning((3, 3), (6, 6))));
        // A click outside it lets it go and selects nothing.
        let outside = on_cell(&ed, &frame, 15, 10);
        click(&mut ed, &frame, outside);
        assert_eq!(ed.selection(), None);
        drag(&mut ed, &frame, a, b);
        ed.update(&BuilderInput { escape: true, ..Default::default() }, &frame);
        assert_eq!(ed.selection(), None, "Escape lets it go");
        drag(&mut ed, &frame, a, b);
        ed.update(&BuilderInput { pointer: Some(outside), right_pressed: true, right_held: true, ..Default::default() }, &frame);
        ed.update(&BuilderInput { pointer: Some(outside), ..Default::default() }, &frame);
        assert_eq!(ed.selection(), None, "the right button lets it go");
        drag(&mut ed, &frame, a, b);
        ed.select_tool(Tool::Road);
        assert_eq!(ed.selection(), None, "another tool lets it go");
        assert_eq!(ed.map().cells, select_map().cells, "selecting changes nothing");
        assert_eq!(ed.history().undo_depth(), 0);
    }

    #[test]
    fn a_drag_from_inside_the_selection_lifts_and_carries_its_cells_as_one_undo_step() {
        let frame = arena();
        let before = select_map();
        let mut ed = MapEditor::new(before.clone());
        assert!(ed.select_cells(CellRect::spanning((3, 3), (6, 6))));
        let grab = on_cell(&ed, &frame, 4, 4);
        ed.update(&BuilderInput { pointer: Some(grab), pressed: true, held: true, ..Default::default() }, &frame);
        let to = on_cell(&ed, &frame, 14, 7);
        ed.update(&BuilderInput { pointer: Some(to), held: true, ..Default::default() }, &frame);
        // Held: the cells are off the map and carried.
        assert_eq!(ed.map().cell(4, 4), None, "lifted");
        let (rect, lifted) = ed.lifted().expect("cells carried");
        assert_eq!((rect.col, rect.row), (13, 6));
        assert_eq!(lifted.cells.len(), 5);
        assert_eq!(ed.par(), None);
        ed.update(&BuilderInput { pointer: Some(to), ..Default::default() }, &frame);
        assert_eq!(ed.lifted(), None);
        for (cell, obj) in [((14, 7), brick()), ((15, 7), brick()), ((14, 8), CellObject::Road), ((16, 9), CellObject::Water), ((15, 9), CellObject::Start)] {
            assert_eq!(ed.map().cell(cell.0, cell.1), Some(&obj), "{cell:?}");
        }
        for cell in [(4, 4), (5, 4), (4, 5), (6, 6), (5, 6)] {
            assert_eq!(ed.map().cell(cell.0, cell.1), None, "{cell:?} vacated");
        }
        assert_eq!(count(&ed, CellObject::Start), 1, "the start moved with it");
        assert_eq!(ed.selection(), Some(CellRect { col: 13, row: 6, cols: 4, rows: 4 }), "the selection went with its cells");
        assert_eq!(ed.history().undo_depth(), 1, "one undo step");
        ground_is_rebuilt(&ed);
        undo_redo_exact(&mut ed, &before);
        ground_is_rebuilt(&ed);
        // A drag that comes back to where it began changes nothing and
        // records nothing.
        let depth = ed.history().undo_depth();
        let at = on_cell(&ed, &frame, 14, 7);
        drag(&mut ed, &frame, at, at);
        assert_eq!(ed.history().undo_depth(), depth);
        // Carried past the map's edge, the cells stop at it.
        let changes = ed.move_selection((100, 0));
        assert!(!changes.is_empty());
        let rect = ed.selection().expect("still selected");
        assert_eq!(rect.col + rect.cols, ed.field_cells().cols, "kept on the field");
        assert_eq!(ed.map().cells.len(), before.cells.len(), "nothing dropped off the edge");
    }

    #[test]
    fn copy_then_paste_puts_a_copy_down_where_the_ghost_stands_as_one_step() {
        let frame = arena();
        let before = select_map();
        let mut ed = MapEditor::new(before.clone());
        ed.select_cells(CellRect::spanning((3, 3), (6, 6)));
        assert!(ed.copy_selection());
        assert_eq!(ed.clipboard().map(|c| (c.cols, c.rows, c.cells.len())), Some((4, 4, 5)));
        // Ctrl+V with the mouse on (20, 4): the ghost stands centred there
        // and follows the mouse.
        ed.update(&BuilderInput { paste: true, pointer: Some(on_cell(&ed, &frame, 20, 4)), ..Default::default() }, &frame);
        assert_eq!(ed.ghost().map(|g| g.at), Some((19, 3)));
        ed.update(&BuilderInput { pointer: Some(on_cell(&ed, &frame, 22, 6)), ..Default::default() }, &frame);
        assert_eq!(ed.ghost().map(|g| g.at), Some((21, 5)), "the ghost follows a hovering mouse");
        assert_eq!(ed.map().cells, before.cells, "a ghost is not on the map");
        // A press puts it down where it is pressed.
        let place = on_cell(&ed, &frame, 22, 6);
        click(&mut ed, &frame, place);
        assert_eq!(ed.ghost(), None);
        assert_eq!(ed.map().cell(22, 6), Some(&brick()));
        assert_eq!(ed.map().cell(23, 6), Some(&brick()));
        assert_eq!(ed.map().cell(22, 7), Some(&CellObject::Road));
        assert_eq!(ed.map().cell(24, 8), Some(&CellObject::Water));
        assert_eq!(ed.map().cell(23, 8), None, "the map holds a start already");
        assert_eq!(ed.map().cell(4, 4), Some(&brick()), "the original stays");
        assert_eq!(count(&ed, CellObject::Start), 1, "a paste never leaves two starts");
        assert_eq!(ed.selection(), Some(CellRect { col: 21, row: 5, cols: 4, rows: 4 }), "the paste is selected");
        assert_eq!(ed.history().undo_depth(), 1);
        ground_is_rebuilt(&ed);
        undo_redo_exact(&mut ed, &before);
    }

    /// The singletons in a selection (docs above): a paste places a start
    /// or a frog only where the map holds none of it once the rest of the
    /// paste is down - never two, and never a cell outside the paste's own
    /// rectangle - so a cut and a paste carry the start like a move, and a
    /// paste over the map's own start puts the pasted one in its place.
    #[test]
    fn a_paste_never_leaves_two_starts_and_a_cut_and_paste_carries_them() {
        let mut ed = MapEditor::new(select_map());
        ed.select_cells(CellRect::spanning((3, 3), (6, 6)));
        ed.copy_selection();
        ed.paste(Some((15, 15)));
        ed.place_ghost();
        assert_eq!(ed.map().start_cell(), Some((5, 6)), "a copy's start stays where it was");
        assert_eq!(count(&ed, CellObject::Start), 1);
        // Cut and paste: the start goes where the paste goes.
        ed.select_cells(CellRect::spanning((3, 3), (6, 6)));
        ed.cut_selection();
        assert_eq!(ed.map().start_cell(), None);
        ed.paste(Some((26, 5)));
        ed.place_ghost();
        assert_eq!(ed.map().start_cell(), Some((27, 7)));
        // Again: the map holds one, so the second leaves it out.
        ed.paste(Some((10, 12)));
        let changes = ed.place_ghost();
        assert!(!changes.is_empty());
        assert_eq!(count(&ed, CellObject::Start), 1, "never two");
        // A paste whose brick lands on the map's start: the start is gone
        // under it, so the pasted one takes its place.
        let mut ed = MapEditor::new(select_map());
        ed.select_cells(CellRect::spanning((4, 4), (5, 6)));
        ed.copy_selection();
        ed.paste(None);
        assert!(ed.move_ghost((4, 6)));
        ed.place_ghost();
        assert_eq!(ed.map().cell(5, 6), Some(&brick()), "the brick over the old start");
        assert_eq!(ed.map().start_cell(), Some((5, 8)), "the pasted start in its place");
        assert_eq!(count(&ed, CellObject::Start), 1);
        // The frog alike, by a move: it goes with the cells.
        let mut ed = MapEditor::new(select_map());
        ed.select_cells(CellRect::spanning((19, 9), (21, 11)));
        ed.move_selection((3, 2));
        assert_eq!(ed.map().frog_cell(), Some((23, 12)));
        assert_eq!(count(&ed, CellObject::Frog), 1);
    }

    #[test]
    fn flips_mirror_the_selection_in_place_and_the_ghost_where_it_stands() {
        let before = select_map();
        let mut ed = MapEditor::new(before.clone());
        ed.select_cells(CellRect::spanning((3, 3), (6, 6)));
        let changes = ed.flip(Axis::Horizontal);
        assert!(!changes.is_empty());
        // (col 3..6): a cell at col c lands on 9 - c.
        assert_eq!(ed.map().cell(5, 5), Some(&CellObject::Road));
        assert_eq!(ed.map().cell(3, 6), Some(&CellObject::Water));
        assert_eq!(ed.map().cell(4, 6), Some(&CellObject::Start));
        assert_eq!(ed.map().cell(4, 4), Some(&brick()));
        assert_eq!(ed.map().cell(5, 4), Some(&brick()));
        assert_eq!(ed.map().cell(6, 6), None);
        assert_eq!(ed.history().undo_depth(), 1, "one step");
        assert_eq!(ed.selection(), Some(CellRect::spanning((3, 3), (6, 6))), "the selection stays");
        ground_is_rebuilt(&ed);
        undo_redo_exact(&mut ed, &before);
        ed.undo();
        ed.flip(Axis::Vertical);
        // (row 3..6): a cell at row r lands on 9 - r.
        assert_eq!(ed.map().cell(4, 4), Some(&CellObject::Road));
        assert_eq!(ed.map().cell(4, 5), Some(&brick()));
        assert_eq!(ed.map().cell(5, 3), Some(&CellObject::Start));
        ed.flip(Axis::Vertical);
        assert_eq!(ed.map().cells, before.cells, "a flip twice is none");
        ed.flip(Axis::Horizontal);
        ed.flip(Axis::Horizontal);
        assert_eq!(ed.map().cells, before.cells, "a flip twice is none");
        // A ghost flips where it stands, and the map does not.
        let depth = ed.history().undo_depth();
        ed.copy_selection();
        ed.paste(Some((20, 5)));
        ed.flip(Axis::Horizontal);
        assert_eq!(ed.history().undo_depth(), depth);
        assert_eq!(ed.map().cells, before.cells);
        let ghost = ed.ghost().expect("a ghost");
        assert!(ghost.clip.cells.contains(&(0, 3, CellObject::Water)), "{:?}", ghost.clip.cells);
    }

    #[test]
    fn delete_and_cut_clear_the_selection_as_one_step_and_keep_it_selected() {
        let before = select_map();
        let mut ed = MapEditor::new(before.clone());
        ed.select_cells(CellRect::spanning((3, 3), (6, 6)));
        let changes = ed.delete_selection();
        assert_eq!(changes.len(), 5);
        assert_eq!(ed.map().cells.len(), before.cells.len() - 5);
        assert_eq!(ed.selection(), Some(CellRect::spanning((3, 3), (6, 6))));
        assert_eq!(ed.history().undo_depth(), 1);
        ground_is_rebuilt(&ed);
        undo_redo_exact(&mut ed, &before);
        ed.undo();
        let changes = ed.cut_selection();
        assert_eq!(changes.len(), 5);
        assert_eq!(ed.clipboard().map(|c| c.cells.len()), Some(5));
        undo_redo_exact(&mut ed, &before);
        // Nothing selected: nothing to delete, cut or copy.
        ed.clear_selection();
        assert!(ed.delete_selection().is_empty() && ed.cut_selection().is_empty() && !ed.copy_selection());
    }

    /// Gates and portals are carried like any cell: a gate moved off the
    /// edge is the map's and CHECK's to name, a pasted portal joins the
    /// network.
    #[test]
    fn gates_and_portals_are_carried_like_any_cell() {
        let mut ed = MapEditor::new(select_map());
        ed.select_cells(CellRect::spanning((0, 7), (1, 9)));
        ed.move_selection((5, 0));
        assert_eq!(ed.map().gate_cells(), vec![(5, 8)]);
        assert_eq!(gate_inward(map::cell_to_world(5, 8), W, H), None, "off the edge now: drawn outlined, named by CHECK");
        ed.select_cells(CellRect::spanning((8, 4), (8, 4)));
        ed.copy_selection();
        ed.paste(Some((12, 12)));
        ed.place_ghost();
        assert_eq!(ed.map().portal_cells(), vec![(8, 4), (12, 12), (25, 12)], "a third portal in the network");
    }

    #[test]
    fn stamps_are_the_shipped_ones_then_the_ones_kept_this_session() {
        let mut ed = MapEditor::new(select_map());
        let keys: Vec<String> = ed.stamps().iter().map(Stamp::key).collect();
        assert_eq!(keys, ["fort", "bunker", "river-bend"]);
        // Kept from a loose selection: trimmed to its cells.
        ed.select_cells(CellRect::spanning((2, 2), (7, 7)));
        let index = ed.save_stamp().expect("a stamp kept");
        assert_eq!(index, 3);
        let stamp = &ed.stamps()[index];
        assert_eq!(stamp.key(), "saved_1");
        assert_eq!((stamp.clip.cols, stamp.clip.rows), (3, 3));
        assert_eq!(ed.history().undo_depth(), 0, "keeping a stamp edits nothing");
        // An empty selection keeps nothing.
        ed.select_cells(CellRect::spanning((10, 10), (12, 12)));
        assert_eq!(ed.save_stamp(), None);
        assert_eq!(ed.stamps().len(), 4);
        // A stamp is the paste ghost, in the middle of the view.
        ed.select_tool(Tool::Road);
        assert!(ed.use_stamp(0));
        assert_eq!(ed.tool(), Tool::Select);
        let ghost = ed.ghost().expect("the fort as a ghost").clone();
        assert_eq!((ghost.clip.cols, ghost.clip.rows), (10, 8));
        let middle = map::world_to_cell(ed.camera().center(&ed.viewport()));
        assert!(ghost.rect().contains(middle), "{:?} round {middle:?}", ghost.rect());
        let before = ed.map().clone();
        let changes = ed.place_ghost();
        assert_eq!(changes.len(), ghost.clip.cells.len());
        assert_eq!(ed.history().undo_depth(), 1);
        undo_redo_exact(&mut ed, &before);
        assert!(!ed.use_stamp(99));
    }

    /// The strip under the bar by name, as a mouse presses it: BRUSH's list
    /// takes the select tool; COPY, CUT, DELETE, the flips and + STAMP
    /// stand dim until something is selected and PASTE until the
    /// clipboard holds something; PASTE makes a ghost, PLACE puts it down,
    /// CANCEL takes it away; STAMPS opens the list, whose row makes its
    /// stamp the ghost.
    #[test]
    fn the_strip_acts_on_the_selection_by_name() {
        let frame = arena();
        let before = select_map();
        let mut ed = MapEditor::new(before.clone());
        press_named(&mut ed, &frame, "brush");
        assert_eq!(ed.open_menu(), Some("brush"));
        press_named(&mut ed, &frame, "tool_select");
        assert_eq!(ed.tool(), Tool::Select);
        assert_eq!(ed.open_menu(), None);
        for name in ["sel_copy", "sel_cut", "sel_paste", "sel_flip_h", "sel_flip_v", "sel_delete", "sel_stamp"] {
            assert!(!has_named(&ed, &frame, name), "{name} is dim with nothing selected");
        }
        assert!(has_named(&ed, &frame, "sel_stamps"));
        let (a, b) = (on_cell(&ed, &frame, 3, 3), on_cell(&ed, &frame, 6, 6));
        drag(&mut ed, &frame, a, b);
        assert!(has_named(&ed, &frame, "sel_copy") && !has_named(&ed, &frame, "sel_paste"));
        press_named(&mut ed, &frame, "sel_copy");
        assert!(has_named(&ed, &frame, "sel_paste"));
        press_named(&mut ed, &frame, "sel_flip_h");
        assert_eq!(ed.map().cell(5, 5), Some(&CellObject::Road));
        press_named(&mut ed, &frame, "sel_flip_h");
        assert_eq!(ed.map().cells, before.cells);
        press_named(&mut ed, &frame, "sel_paste");
        assert!(ed.ghost().is_some());
        assert!(has_named(&ed, &frame, "sel_place") && has_named(&ed, &frame, "sel_cancel") && !has_named(&ed, &frame, "sel_copy"));
        press_named(&mut ed, &frame, "sel_place");
        assert_eq!(ed.ghost(), None);
        assert_eq!(count(&ed, brick()), 4, "a copy put down");
        press_named(&mut ed, &frame, "sel_delete");
        assert_eq!(count(&ed, brick()), 2);
        press_named(&mut ed, &frame, "sel_paste");
        press_named(&mut ed, &frame, "sel_cancel");
        assert_eq!(ed.ghost(), None);
        assert_eq!(count(&ed, brick()), 2, "CANCEL put nothing down");
        drag(&mut ed, &frame, a, b);
        press_named(&mut ed, &frame, "sel_stamp");
        assert_eq!(ed.stamps().len(), 4);
        press_named(&mut ed, &frame, "sel_stamps");
        assert_eq!(ed.open_menu(), Some("stamps"));
        assert!(has_named(&ed, &frame, "stamp_saved_1"));
        press_named(&mut ed, &frame, "stamp_bunker");
        assert_eq!(ed.open_menu(), None);
        assert_eq!(ed.ghost().map(|g| (g.clip.cols, g.clip.rows)), Some((6, 5)));
        // The keys: Escape takes the ghost away, then the selection.
        ed.update(&BuilderInput { escape: true, ..Default::default() }, &frame);
        assert_eq!(ed.ghost(), None);
        // A press on the strip never selects the cells under it.
        assert!(!ed.on_canvas(frame.to_canvas(center(named(&ed, &frame, "sel_stamps"))), &frame));
    }

    /// On a phone the bar folds BRUSH into TOOLS: the palette's brush row
    /// takes the select tool and opens the STAMPS list.
    #[test]
    fn a_folded_bar_keeps_the_brush_in_its_palette() {
        let insets = Insets { left: 59.0 * 3.0, top: 0.0, right: 59.0 * 3.0, bottom: 21.0 * 3.0 };
        let ui = UiFrame::new((852.0 * 3.0, 393.0 * 3.0), 3.0, 1.0, insets, true);
        let frame = BuilderFrame::new(ui, (W, H), MapClass::Arena, None);
        let mut ed = MapEditor::new(select_map());
        assert!(!has_named(&ed, &frame, "brush"));
        press_named(&mut ed, &frame, "tools");
        for name in ["shape_pen", "tool_select", "brush_stamps"] {
            assert!(has_named(&ed, &frame, name), "{name} in the palette");
        }
        press_named(&mut ed, &frame, "tool_select");
        assert_eq!(ed.tool(), Tool::Select);
        press_named(&mut ed, &frame, "tools");
        press_named(&mut ed, &frame, "brush_stamps");
        assert_eq!(ed.open_menu(), Some("stamps"));
        press_named(&mut ed, &frame, "stamp_fort");
        assert!(ed.ghost().is_some());
        press_named(&mut ed, &frame, "tools");
        press_named(&mut ed, &frame, "shape_pen");
        assert_eq!(ed.tool(), Tool::Wall(Material::Brick), "the pen goes back to the tool before the select tool");
        assert_eq!(ed.ghost(), None);
    }

    /// With a finger: a drag past the slop draws the rectangle, a tap
    /// outside lets it go, a drag from inside carries its cells as one
    /// step, a second finger landing on that drag takes it back; a pasted
    /// ghost follows a drag from anywhere by the cells it moves, a tap off
    /// it moves it there and a tap on it puts it down.
    #[test]
    fn a_finger_selects_carries_and_pastes() {
        let (mut ed, frame) = touch_arena();
        ed.load(select_map());
        let before_select = ed.map().clone();
        ed.select_tool(Tool::Select);
        let p = |ed: &MapEditor, c: i32, r: i32| on_cell(ed, &frame, c, r);
        let path = |from: Vec2, to: Vec2| -> Vec<Vec<(i32, f32, f32)>> {
            (0..=10).map(|i| vec![(1, from.x + (to.x - from.x) * i as f32 / 10.0, from.y + (to.y - from.y) * i as f32 / 10.0)]).collect()
        };
        let (a, b) = (p(&ed, 3, 3), p(&ed, 6, 6));
        fingers(&mut ed, &frame, &path(a, b));
        assert_eq!(ed.selection(), Some(CellRect::spanning((3, 3), (6, 6))));
        let outside = p(&ed, 15, 10);
        fingers(&mut ed, &frame, &[vec![(2, outside.x, outside.y)]]);
        assert_eq!(ed.selection(), None, "a tap outside lets it go");
        fingers(&mut ed, &frame, &path(a, b));
        let depth = ed.history().undo_depth();
        let (grab, to) = (p(&ed, 4, 4), p(&ed, 12, 4));
        fingers(&mut ed, &frame, &path(grab, to));
        assert_eq!(ed.map().cell(12, 4), Some(&brick()));
        assert_eq!(ed.history().undo_depth(), depth + 1, "one step");
        ed.undo();
        assert_eq!(ed.map().cells, before_select.cells);
        // A second finger on a carry takes it back.
        fingers(&mut ed, &frame, &path(a, b));
        let mut frames = path(grab, to);
        frames.push(vec![(1, to.x, to.y), (2, to.x + 60.0, to.y)]);
        fingers(&mut ed, &frame, &frames);
        assert_eq!(ed.map().cells, before_select.cells, "taken back");
        assert_eq!(ed.history().undo_depth(), depth);
        // PASTE by its button: the ghost in the middle of the view.
        ed.copy_selection();
        let paste = center(named(&ed, &frame, "sel_paste"));
        fingers(&mut ed, &frame, &[vec![(3, paste.x, paste.y)]]);
        let at = ed.ghost().expect("a ghost").at;
        // A drag from anywhere carries it by the cells the finger moves.
        let (from, to) = (p(&ed, 20, 12), p(&ed, 23, 13));
        fingers(&mut ed, &frame, &path(from, to));
        assert_eq!(ed.ghost().map(|g| g.at), Some((at.0 + 3, at.1 + 1)));
        // A tap off it moves it there; a tap on it puts it down.
        let off = p(&ed, 10, 13);
        fingers(&mut ed, &frame, &[vec![(4, off.x, off.y)]]);
        assert_eq!(ed.ghost().map(|g| g.at), Some((9, 12)));
        assert_eq!(ed.map().cells, before_select.cells, "a tap off it puts nothing down");
        fingers(&mut ed, &frame, &[vec![(5, off.x, off.y)]]);
        assert_eq!(ed.ghost(), None);
        assert_eq!(ed.map().cell(10, 13), Some(&brick()));
    }

    #[test]
    fn the_keys_copy_cut_paste_and_delete() {
        let frame = arena();
        let before = select_map();
        let mut ed = MapEditor::new(before.clone());
        ed.select_cells(CellRect::spanning((3, 3), (6, 6)));
        ed.update(&BuilderInput { copy: true, ..Default::default() }, &frame);
        assert!(ed.clipboard().is_some());
        ed.update(&BuilderInput { delete: true, ..Default::default() }, &frame);
        assert_eq!(ed.map().cell(4, 4), None);
        ed.undo();
        ed.update(&BuilderInput { backspace: true, ..Default::default() }, &frame);
        assert_eq!(ed.map().cell(4, 4), None, "Backspace deletes too");
        ed.undo();
        ed.update(&BuilderInput { cut: true, ..Default::default() }, &frame);
        assert_eq!(ed.map().cell(4, 4), None);
        let at = on_cell(&ed, &frame, 4, 4);
        ed.update(&BuilderInput { paste: true, pointer: Some(at), ..Default::default() }, &frame);
        click(&mut ed, &frame, at);
        assert_eq!(ed.map().cells, before.cells, "cut and pasted back where it was");
        // Ctrl+V with a brush pastes too, taking the select tool.
        ed.select_tool(Tool::Road);
        ed.update(&BuilderInput { paste: true, pointer: Some(on_cell(&ed, &frame, 20, 6)), ..Default::default() }, &frame);
        assert_eq!(ed.tool(), Tool::Select);
        assert!(ed.ghost().is_some());
        // The Save prompt keeps its keys.
        ed.popup = Some(Popup::Save { name: String::new() });
        ed.update(&BuilderInput { escape: true, ..Default::default() }, &frame);
        assert!(ed.ghost().is_some(), "Escape closed the prompt, not the ghost");
    }

    // ----- the brush's shapes -----

    /// A closed ring of brick round cells 2..=6 each way, its inside
    /// empty.
    fn ring_map() -> MapFile {
        let mut map = MapFile::new();
        for i in 2..=6 {
            for (c, r) in [(i, 2), (i, 6), (2, i), (6, i)] {
                map.set_cell(c, r, brick());
            }
        }
        map
    }

    /// How many cells of `rect` hold `obj`.
    fn held_in(ed: &MapEditor, rect: CellRect, obj: CellObject) -> usize {
        rect.cells().filter(|&(c, r)| ed.map().cell(c, r) == Some(&obj)).count()
    }

    /// RECT: held, the rectangle is only drawn; the release fills it, over
    /// whatever was there, as one step - begun on the brush's own object
    /// too, since a rectangle does not toggle. The secondary button and
    /// the eraser clear it instead, and Escape takes the drag back.
    #[test]
    fn a_rect_fills_on_the_release_and_the_eraser_or_the_right_button_clears_it() {
        let frame = arena();
        let before = select_map();
        let mut ed = MapEditor::new(before.clone());
        ed.set_shape(Shape::Rect);
        assert_eq!(ed.shape(), Shape::Rect);
        let rect = CellRect::spanning((3, 3), (7, 6));
        let (a, b) = (on_cell(&ed, &frame, 7, 6), on_cell(&ed, &frame, 3, 3));
        ed.update(&BuilderInput { pointer: Some(a), pressed: true, held: true, ..Default::default() }, &frame);
        ed.update(&BuilderInput { pointer: Some(b), held: true, ..Default::default() }, &frame);
        assert_eq!(ed.rect_stroke(), Some((rect, false)), "dragged up and left: the same rectangle");
        assert_eq!(ed.map().cells, before.cells, "nothing laid while the drag is held");
        ed.update(&BuilderInput { pointer: Some(b), ..Default::default() }, &frame);
        assert_eq!(ed.rect_stroke(), None);
        assert_eq!(held_in(&ed, rect, brick()), 20);
        assert_eq!(count(&ed, CellObject::Start), 0, "painted over, as a pen stroke paints over a start");
        assert_eq!(ed.history().undo_depth(), 1);
        ground_is_rebuilt(&ed);
        undo_redo_exact(&mut ed, &before);
        // Begun on a brick, it still paints.
        let before = ed.map().clone();
        ed.stroke(&[(4, 4), (9, 4)], false);
        assert_eq!(held_in(&ed, CellRect::spanning((4, 4), (9, 4)), brick()), 6);
        undo_redo_exact(&mut ed, &before);
        // The secondary button clears it, whatever is in it ...
        let before = ed.map().clone();
        ed.stroke(&[(2, 2), (8, 7)], true);
        assert_eq!(CellRect::spanning((2, 2), (8, 7)).cells().filter(|&(c, r)| ed.map().cell(c, r).is_some()).count(), 0);
        assert_eq!(ed.map().cell(20, 10), Some(&CellObject::Frog), "and nothing past it");
        undo_redo_exact(&mut ed, &before);
        ground_is_rebuilt(&ed);
        // ... and so does the eraser.
        ed.select_tool(Tool::Eraser);
        ed.stroke(&[(4, 4), (4, 5)], false);
        assert_eq!((ed.map().cell(4, 4), ed.map().cell(4, 5)), (None, None));
        ed.undo();
        // Escape takes a drag back: nothing laid, no step.
        ed.select_tool(Tool::Water);
        let (before, depth) = (ed.map().clone(), ed.history().undo_depth());
        ed.update(&BuilderInput { pointer: Some(on_cell(&ed, &frame, 10, 10)), pressed: true, held: true, ..Default::default() }, &frame);
        ed.update(&BuilderInput { pointer: Some(on_cell(&ed, &frame, 14, 12)), held: true, ..Default::default() }, &frame);
        assert!(ed.rect_stroke().is_some());
        ed.update(&BuilderInput { pointer: Some(on_cell(&ed, &frame, 14, 12)), held: true, escape: true, ..Default::default() }, &frame);
        ed.update(&BuilderInput { pointer: Some(on_cell(&ed, &frame, 14, 12)), ..Default::default() }, &frame);
        assert_eq!((ed.map().cells == before.cells, ed.history().undo_depth()), (true, depth));
    }

    /// FILL: a press floods the region of the pressed cell - the cells
    /// joined to it edge to edge holding exactly what it holds - as one
    /// step, and the water it lays auto-tiles; the brush on its own object
    /// takes the region back (the toggle-erase rule), another brush on a
    /// region of walls fills the walls, and the secondary button clears.
    #[test]
    fn a_fill_floods_the_region_of_the_pressed_cell_and_toggles() {
        let frame = arena();
        let before = ring_map();
        let mut ed = MapEditor::new(before.clone());
        ed.select_tool(Tool::Water);
        ed.set_shape(Shape::Fill);
        let at = on_cell(&ed, &frame, 4, 4);
        click(&mut ed, &frame, at);
        assert_eq!(held_in(&ed, CellRect::spanning((3, 3), (5, 5)), CellObject::Water), 9);
        assert_eq!(count(&ed, CellObject::Water), 9, "the ring holds it in");
        assert_eq!(ed.history().undo_depth(), 1);
        ground_is_rebuilt(&ed);
        assert_eq!(ed.ground().depth(4, 4), ground::Depth::Deep, "a 3 x 3 lake, its middle open water");
        undo_redo_exact(&mut ed, &before);
        // The same brush on its own object takes the region back.
        let filled = ed.map().clone();
        let at = on_cell(&ed, &frame, 3, 5);
        click(&mut ed, &frame, at);
        assert_eq!(count(&ed, CellObject::Water), 0);
        ground_is_rebuilt(&ed);
        undo_redo_exact(&mut ed, &filled);
        ed.undo();
        // Iron on the ring fills the ring: sixteen bricks joined edge to edge.
        ed.select_tool(Tool::Wall(Material::Iron));
        let before = ed.map().clone();
        ed.stroke(&[(2, 2)], false);
        let iron = CellObject::Wall { material: Material::Iron };
        assert_eq!((count(&ed, iron), count(&ed, brick())), (16, 0));
        undo_redo_exact(&mut ed, &before);
        // The secondary button clears the region pressed; the eraser too.
        ed.stroke(&[(6, 4)], true);
        assert_eq!(count(&ed, iron), 0);
        assert_eq!(count(&ed, CellObject::Water), 9, "only the region pressed");
        ed.select_tool(Tool::Eraser);
        ed.stroke(&[(4, 4)], false);
        assert!(ed.map().cells.is_empty());
        // Outside the ring - the rest of the arena, inside its cap - fills
        // up to the ring and the map's edge, never past them.
        ed.undo();
        ed.undo();
        ed.select_tool(Tool::TallGrass);
        ed.stroke(&[(0, 0)], false);
        let field = ed.field_cells();
        assert_eq!(count(&ed, CellObject::TallGrass), (field.cols * field.rows) as usize - 25);
    }

    /// FILL past `builder_fill_max_cells` changes nothing, makes no step
    /// and says why on the status line; within it the same field fills.
    #[test]
    fn a_fill_past_its_cap_changes_nothing_and_says_why() {
        let rules = BrushRules::of(&crate::tuning::Tuning::DEFAULT);
        let mut map = MapFile::new();
        map.size = Some((80.0, 60.0));
        let mut ed = MapEditor::new(map);
        let field = ed.field_cells();
        assert!((field.cols * field.rows) as usize > rules.fill_max_cells, "the field is past the cap");
        ed.select_tool(Tool::TallGrass);
        ed.set_shape(Shape::Fill);
        assert!(ed.stroke(&[(10, 10)], false).is_empty());
        assert!(ed.map().cells.is_empty());
        assert_eq!(ed.history().undo_depth(), 0, "no step");
        let message = ed.status().expect("the status line says why");
        assert!(message.contains(&rules.fill_max_cells.to_string()), "{message}");
        // Walled off, the corner fills.
        ed.select_tool(Tool::Wall(Material::Brick));
        ed.set_shape(Shape::Rect);
        ed.stroke(&[(30, 0), (30, 30)], false);
        ed.stroke(&[(0, 30), (30, 30)], false);
        ed.select_tool(Tool::TallGrass);
        ed.set_shape(Shape::Fill);
        assert_eq!(ed.stroke(&[(10, 10)], false).len(), 30 * 30);
    }

    /// SCATTER: of the cells round each cell the stroke crosses, a hashed
    /// share takes the brush's object - empty cells only, so a wall in the
    /// way stays -, the same share for the same stroke in any editor (no
    /// random draw), another for the next stroke; erasing, it thins what it
    /// scatters and leaves the rest. One step a stroke.
    #[test]
    fn a_scatter_lays_a_hashed_share_of_its_footprint_on_empty_cells_and_thins_erasing() {
        let rules = BrushRules::current();
        let mut map = MapFile::new();
        map.set_cell(14, 5, brick());
        let line: Vec<(i32, i32)> = (8..=20).map(|c| (c, 5)).collect();
        let scatter = |map: &MapFile| {
            let mut ed = MapEditor::new(map.clone());
            ed.select_tool(Tool::Prop(Material::Tree));
            ed.set_shape(Shape::Scatter);
            ed
        };
        let mut ed = scatter(&map);
        let before = ed.map().clone();
        let changes = ed.stroke(&line, false);
        let trees = count(&ed, CellObject::Tree);
        assert_eq!(changes.len(), trees);
        let reach = (rules.scatter_radius as f32 + 0.5).powi(2);
        let near = |(c, r): (i32, i32)| line.iter().any(|&(lc, lr)| (((c - lc).pow(2) + (r - lr).pow(2)) as f32) <= reach);
        let footprint = ed.field_cells().cells().filter(|&c| near(c)).count();
        assert!(ed.map().iter_cells().all(|(c, r, o)| *o != CellObject::Tree || near((c, r))), "only round the stroke");
        let share = trees as f32 / footprint as f32;
        assert!(share > rules.scatter_density * 0.4 && share < rules.scatter_density * 1.8, "{trees} of {footprint}");
        assert_eq!(ed.map().cell(14, 5), Some(&brick()), "never painted over");
        assert_eq!(ed.history().undo_depth(), 1);
        undo_redo_exact(&mut ed, &before);
        let mut twin = scatter(&map);
        twin.stroke(&line, false);
        assert_eq!(twin.map().cells, ed.map().cells, "the same stroke, the same cells");
        ed.stroke(&line, false);
        let thick = count(&ed, CellObject::Tree);
        assert!(thick > trees, "a second stroke picks other cells");
        let before = ed.map().clone();
        ed.stroke(&line, true);
        let thinned = count(&ed, CellObject::Tree);
        assert!(thinned < thick && thinned > 0, "{thinned} of {thick}");
        assert_eq!(ed.map().cell(14, 5), Some(&brick()), "a tree brush's erase thins trees only");
        undo_redo_exact(&mut ed, &before);
        // Begun on a tree, a scatter still paints.
        let on = ed.map().iter_cells().find(|(_, _, o)| **o == CellObject::Tree).map(|(c, r, _)| (c, r)).expect("a tree");
        let n = count(&ed, CellObject::Tree);
        ed.stroke(&[on], false);
        assert!(count(&ed, CellObject::Tree) >= n);
        // Water scattered auto-tiles.
        ed.select_tool(Tool::Water);
        ed.stroke(&[(25, 10), (28, 10)], false);
        ground_is_rebuilt(&ed);
    }

    /// A singleton's brush paints with the pen whatever the shape: the
    /// start moves along a stroke, BRUSH's shapes but the pen are dim and
    /// refused, and the shape comes back with a brush that takes it.
    /// BRUSH's list and the palette's row pick the shape.
    #[test]
    fn a_singletons_brush_paints_with_the_pen_and_brushs_rows_pick_the_shape() {
        let frame = arena();
        let mut ed = MapEditor::new(select_map());
        press_named(&mut ed, &frame, "brush");
        assert_eq!(ed.open_menu(), Some("brush"));
        for name in ["shape_pen", "shape_rect", "shape_fill", "shape_scatter", "tool_select", "brush_stamps"] {
            assert!(has_named(&ed, &frame, name), "{name} in BRUSH's list");
        }
        press_named(&mut ed, &frame, "shape_rect");
        assert_eq!((ed.shape(), ed.open_menu()), (Shape::Rect, None));
        ed.select_tool(Tool::Start);
        assert_eq!(ed.shape(), Shape::Pen);
        assert!(!ed.brush_row_live(BrushRow::Shape(Shape::Fill)) && ed.brush_row_live(BrushRow::Shape(Shape::Pen)));
        press_named(&mut ed, &frame, "brush");
        assert!(!has_named(&ed, &frame, "shape_scatter") && has_named(&ed, &frame, "shape_pen"), "dim rows are no buttons");
        ed.close_popup();
        ed.set_shape(Shape::Scatter);
        assert_eq!(ed.shape(), Shape::Pen, "refused");
        ed.stroke(&[(10, 10), (14, 12)], false);
        assert_eq!(count(&ed, CellObject::Start), 1, "one start, moved");
        assert_eq!(ed.map().start_cell(), Some((14, 12)));
        ed.select_tool(Tool::Wall(Material::Brick));
        assert_eq!(ed.shape(), Shape::Rect, "back with a brush that takes it");
        // The folded palette's brush row.
        let insets = Insets { left: 59.0 * 3.0, top: 0.0, right: 59.0 * 3.0, bottom: 21.0 * 3.0 };
        let ui = UiFrame::new((852.0 * 3.0, 393.0 * 3.0), 3.0, 1.0, insets, true);
        let narrow = BuilderFrame::new(ui, (W, H), MapClass::Arena, None);
        press_named(&mut ed, &narrow, "tools");
        press_named(&mut ed, &narrow, "shape_fill");
        assert_eq!(ed.shape(), Shape::Fill);
    }

    /// With a finger: a drag draws the rectangle and the lift fills it, a
    /// second finger landing on it takes it back, a tap floods and a drag
    /// scatters.
    #[test]
    fn a_finger_draws_a_rect_floods_with_a_tap_and_scatters_with_a_drag() {
        let (mut ed, frame) = touch_arena();
        ed.load(ring_map());
        let p = |ed: &MapEditor, c: i32, r: i32| on_cell(ed, &frame, c, r);
        let path = |from: Vec2, to: Vec2| -> Vec<Vec<(i32, f32, f32)>> {
            (0..=10).map(|i| vec![(1, from.x + (to.x - from.x) * i as f32 / 10.0, from.y + (to.y - from.y) * i as f32 / 10.0)]).collect()
        };
        ed.select_tool(Tool::Road);
        ed.set_shape(Shape::Rect);
        let frames = path(p(&ed, 10, 3), p(&ed, 14, 5));
        hold_fingers(&mut ed, &frame, &frames);
        assert_eq!(ed.rect_stroke(), Some((CellRect::spanning((10, 3), (14, 5)), false)));
        assert_eq!(count(&ed, CellObject::Road), 0);
        fingers(&mut ed, &frame, &[]);
        assert_eq!(count(&ed, CellObject::Road), 15);
        let (before, depth) = (ed.map().clone(), ed.history().undo_depth());
        let (from, to) = (p(&ed, 20, 3), p(&ed, 24, 6));
        let mut frames = path(from, to);
        frames.push(vec![(1, to.x, to.y), (2, to.x + 60.0, to.y)]);
        fingers(&mut ed, &frame, &frames);
        assert_eq!((ed.map().cells == before.cells, ed.history().undo_depth()), (true, depth), "a second finger takes it back");
        ed.select_tool(Tool::Water);
        ed.set_shape(Shape::Fill);
        let at = p(&ed, 4, 4);
        fingers(&mut ed, &frame, &[vec![(3, at.x, at.y)]]);
        assert_eq!(count(&ed, CellObject::Water), 9);
        ed.select_tool(Tool::TallGrass);
        ed.set_shape(Shape::Scatter);
        let frames = path(p(&ed, 12, 10), p(&ed, 24, 10));
        fingers(&mut ed, &frame, &frames);
        assert!(count(&ed, CellObject::TallGrass) > 0);
    }

    /// An edit that moves the floor under most of the map makes the ground
    /// again whole (`REBUILD_SHARE`), a smaller one repaints its cells, and
    /// both - and their undos - leave the ground a fresh build would make.
    #[test]
    fn a_large_edit_makes_the_ground_again_and_a_small_one_repaints_it_alike() {
        let mut ed = MapEditor::new(select_map());
        ed.set_shape(Shape::Rect);
        let field = ed.field_cells();
        for (tool, to) in [(Tool::Water, (5, 5)), (Tool::Water, (field.cols - 1, field.rows - 1)), (Tool::Road, (30, 12)), (Tool::Wall(Material::Brick), (20, 17))] {
            ed.select_tool(tool);
            let changes = ed.stroke(&[(1, 1), to], false);
            assert!(!changes.is_empty());
            ground_is_rebuilt(&ed);
            ed.undo();
            ground_is_rebuilt(&ed);
            ed.redo();
            ground_is_rebuilt(&ed);
        }
    }
}

#[cfg(test)]
mod file_tests {
    use super::*;
    use crate::framing::MapClass;
    use crate::{DEFAULT_SCREEN_HEIGHT, DEFAULT_SCREEN_WIDTH};

    const W: f32 = DEFAULT_SCREEN_WIDTH as f32;
    const H: f32 = DEFAULT_SCREEN_HEIGHT as f32;

    /// The builder on the standard arena in a window just its size under a
    /// 32 pt bar (`BuilderFrame::headless`), a unit a point.
    fn arena() -> BuilderFrame {
        BuilderFrame::headless((W, H), MapClass::Arena)
    }

    fn press(ed: &mut MapEditor, frame: &BuilderFrame, at: Vec2) {
        ed.update(&BuilderInput { pointer: Some(at), pressed: true, held: true, ..Default::default() }, frame);
        ed.update(&BuilderInput { pointer: Some(at), ..Default::default() }, frame);
    }

    fn center(r: Rectangle) -> Vec2 {
        Vec2::new(r.x + r.width / 2.0, r.y + r.height / 2.0)
    }

    /// A press on the middle of a named button (`named_buttons`, through
    /// the UI frame onto the window).
    fn press_named(ed: &mut MapEditor, frame: &BuilderFrame, name: &str) {
        let r = ed.named_buttons(frame).into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no {name} button")).1;
        press(ed, frame, center(frame.ui.rect_to_window(r)));
    }

    /// The open Load list's layout.
    fn load_layout(ed: &MapEditor, frame: &BuilderFrame) -> LoadLayout {
        match ed.chrome(frame).popup {
            Some(PopupLayout::Load(load)) => load,
            other => panic!("no Load list: {other:?}"),
        }
    }

    /// FILE > CLEAR MAP is the menu's last row on every build.
    #[test]
    fn file_menu_clear_map_row_clears_the_canvas_and_closes_the_menu() {
        let frame = arena();
        let mut base = MapFile::new();
        base.set_cell(1, 1, CellObject::Gate);
        let mut ed = MapEditor::new(base);
        press_named(&mut ed, &frame, "file");
        assert_eq!(ed.open_menu(), Some("file"));
        let rows = FileRow::all();
        assert_eq!(rows.last(), Some(&FileRow::Clear));
        press_named(&mut ed, &frame, "clear_map");
        assert_eq!(ed.open_menu(), None);
        assert!(ed.map().cells.is_empty());
        assert!(ed.dirty());
        assert_eq!(ed.history().undo_depth(), 1);
    }

    /// Where an open Load list is scrolled to.
    fn load_scroll(ed: &MapEditor) -> Option<usize> {
        match &ed.popup {
            Some(Popup::Load { scroll, .. }) => Some(*scroll),
            _ => None,
        }
    }

    /// The open Load list's maps, as it listed them when it opened - not a
    /// second listing of `maps/`, which another test saving a map there
    /// at the same moment would change.
    fn load_entries(ed: &MapEditor) -> Vec<map::MapEntry> {
        match &ed.popup {
            Some(Popup::Load { entries, .. }) => entries.clone(),
            _ => panic!("no Load list open"),
        }
    }

    /// A touch screen has no wheel: an overflowing Load list turns its last
    /// row into a pager - a tap on the right half shows the next page, on
    /// the left half the one before, never past either end - and a row
    /// picked on a later page loads that page's map.
    #[test]
    fn the_load_list_pages_by_touch() {
        let frame = arena();
        let mut ed = MapEditor::new(MapFile::new());
        press_named(&mut ed, &frame, "file");
        press_named(&mut ed, &frame, "load");
        assert_eq!(ed.open_menu(), Some("load"));
        let entries = load_entries(&ed);
        let load = load_layout(&ed, &frame);
        let rows = load.per_page;
        assert!(map::SHIPPED_MAPS.len() > chrome::LOAD_VISIBLE_ROWS && entries.len() > rows, "the shipped maps alone overflow one page");
        let pager = load.pager.expect("a pager").row;
        let panel = load.rows.panel;
        assert!(pager.y + pager.height <= panel.y + panel.height + 0.5, "the pager is the panel's last row");
        let back = frame.ui.to_window(Vec2::new(pager.x + 20.0, pager.y + pager.height / 2.0));
        let next = frame.ui.to_window(Vec2::new(pager.x + pager.width - 20.0, pager.y + pager.height / 2.0));
        press(&mut ed, &frame, back);
        assert_eq!(load_scroll(&ed), Some(0), "the first page does not page back");
        press(&mut ed, &frame, next);
        assert_eq!(load_scroll(&ed), Some(rows));
        let last = entries.len() - rows;
        for _ in 0..entries.len() {
            press(&mut ed, &frame, next);
        }
        assert_eq!(load_scroll(&ed), Some(last), "the last page stops at the end");
        press(&mut ed, &frame, back);
        assert_eq!(load_scroll(&ed), Some(last.saturating_sub(rows)));
        // Page back to the start, then on until a shipped map late in the
        // alphabet is on screen, and pick it.
        for _ in 0..entries.len() {
            press(&mut ed, &frame, back);
        }
        let target = entries.iter().position(|e| e.name == "waves-basic").expect("waves-basic is always listed");
        while load_scroll(&ed).is_some_and(|s| target >= s + rows) {
            press(&mut ed, &frame, next);
        }
        let scroll = load_scroll(&ed).expect("the list is still open");
        press(&mut ed, &frame, center(frame.ui.rect_to_window(load.rows.row(target - scroll))));
        assert_eq!(ed.open_menu(), None);
        assert_eq!(ed.name(), "waves-basic");
    }

    /// The Load list's thumbnails: the open list makes its page's
    /// pictures one a frame - each its map's minimap, a texel a cell - and
    /// none off the page; closed, it shows none; opened again, it checks
    /// the page's against their text and makes none of them again. The
    /// pages are the open list's own: other tests write maps into `maps/`
    /// while this one runs, so a second listing could differ.
    #[test]
    fn the_load_lists_thumbnails_come_a_picture_a_frame_and_go_with_the_list() {
        let frame = arena();
        let mut ed = MapEditor::new(MapFile::new());
        press_named(&mut ed, &frame, "file");
        press_named(&mut ed, &frame, "load");
        let rows = load_layout(&ed, &frame).per_page;
        let listed = |ed: &MapEditor| -> Vec<String> {
            match &ed.popup {
                Some(Popup::Load { entries, .. }) => entries.iter().map(|e| e.name.clone()).collect(),
                _ => panic!("no Load list"),
            }
        };
        let entries = listed(&ed);
        let page: Vec<&str> = entries.iter().take(rows).map(String::as_str).collect();
        let made = |ed: &MapEditor, page: &[&str]| page.iter().filter(|n| ed.thumbnail(n).is_some()).count();
        let idle = BuilderInput::default();
        let start = made(&ed, &page);
        assert!(start <= thumbs::MADE_PER_FRAME, "the frame that opened it made at most one: {start}");
        ed.update(&idle, &frame);
        assert_eq!(made(&ed, &page), start + thumbs::MADE_PER_FRAME, "one more a frame");
        for _ in 0..rows {
            ed.update(&idle, &frame);
        }
        assert_eq!(made(&ed, &page), page.len(), "the whole page");
        assert!(entries.iter().skip(rows).all(|name| ed.thumbnail(name).is_none()), "nothing off the page");
        for name in &page {
            let thumb = ed.thumbnail(name).expect("made");
            if let (Some(image), Ok(map)) = (&thumb.image, map::open_map(name)) {
                assert_eq!((image.width, image.height), crate::minimap::cells_of(map.field_size()), "{name}: a texel a cell");
            }
        }
        let stamp = |ed: &MapEditor, name: &str| ed.thumbnail(name).and_then(|t| t.image.as_ref()).map(|i| i.stamp);
        let stamps: Vec<Option<u64>> = page.iter().map(|n| stamp(&ed, n)).collect();
        ed.update(&BuilderInput { escape: true, ..Default::default() }, &frame);
        assert_eq!(ed.open_menu(), None);
        assert_eq!(ed.thumbnails().count(), 0, "none once it closes");
        press_named(&mut ed, &frame, "file");
        press_named(&mut ed, &frame, "load");
        // A map another test has just written is made on the next frame.
        ed.update(&idle, &frame);
        let entries = listed(&ed);
        let again: Vec<&str> = entries.iter().take(rows).map(String::as_str).collect();
        assert_eq!(made(&ed, &again), again.len(), "the page checked again");
        for (name, before) in page.iter().zip(&stamps) {
            if again.contains(name) {
                assert_eq!(stamp(&ed, name), *before, "{name}: kept, not made again");
            }
        }
    }

    /// FILE opens its menu; LOAD... opens the list; picking a row loads
    /// that map as the new baseline; a press outside closes the list
    /// without painting.
    #[test]
    fn file_menu_load_list_loads_a_shipped_map_by_name() {
        let frame = arena();
        let mut ed = MapEditor::new(MapFile::new());
        press_named(&mut ed, &frame, "file");
        assert_eq!(ed.open_menu(), Some("file"));
        press_named(&mut ed, &frame, "load");
        assert_eq!(ed.open_menu(), Some("load"));
        let entries = load_entries(&ed);
        let row = entries.iter().position(|e| e.name == "default").expect("default is always listed");
        let load = load_layout(&ed, &frame);
        let rows = load.per_page;
        if row >= rows {
            // Scroll down until the row is visible.
            let wheel = BuilderInput { pointer: Some(center(frame.ui.rect_to_window(load.rows.panel))), wheel: -1.0, ..Default::default() };
            for _ in 0..(row + 1 - rows) {
                ed.update(&wheel, &frame);
            }
        }
        press_named(&mut ed, &frame, "map_default");
        assert_eq!(ed.open_menu(), None);
        assert_eq!(ed.name(), "default");
        assert!(!ed.dirty(), "a load is the new baseline");
        assert!(!ed.map().cells.is_empty());
        assert_eq!(ed.history().undo_depth(), 1);
        // A press outside an open list closes it and paints nothing.
        press_named(&mut ed, &frame, "file");
        press_named(&mut ed, &frame, "load");
        let before = ed.map().cells.len();
        let f = frame.layout.field;
        let field_corner = frame.view.to_window(Vec2::new(f.x + 16.0, f.y + f.h - 16.0));
        press(&mut ed, &frame, field_corner);
        assert_eq!(ed.open_menu(), None);
        assert_eq!(ed.map().cells.len(), before);
    }

    /// `load_named` fails cleanly on an unknown name; `save` refuses a bad
    /// name and, where saving exists, round-trips a file and clears dirty.
    #[test]
    fn load_named_and_save_report_errors_and_round_trip() {
        let mut ed = MapEditor::new(MapFile::new());
        assert!(ed.load_named("no-such-map").is_err());
        assert!(ed.save(Some("bad name/with slash")).is_err());
        assert!(ed.save(None).is_err(), "an unnamed map needs SAVE AS");
        if !map::saving_available() {
            return;
        }
        ed.select_tool(Tool::Wall(Material::Glass));
        ed.stroke(&[(7, 7)], false);
        assert!(ed.dirty());
        let name = format!("zz-editor-test-{}", std::process::id());
        let path = map::maps_dir().join(format!("{name}.toml"));
        let line = ed.save(Some(&name)).expect("save");
        let back = MapFile::load(&path);
        let _ = std::fs::remove_file(&path);
        assert!(line.contains(&name));
        assert!(!ed.dirty(), "saved is the baseline");
        assert_eq!(ed.name(), name);
        assert_eq!(back.expect("load back").cell(7, 7), Some(&CellObject::Wall { material: Material::Glass }));
        // Plain SAVE now works: the map has a name.
        assert!(ed.save(None).is_ok());
        let _ = std::fs::remove_file(&path);
    }
}
