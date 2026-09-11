//! The battlefield map builder (docs/game-editor-fusion.md): the Build
//! mode of the game. Click or tap a grid cell to place/erase a
//! wall/prop/road/grass/gate/actor/pickup cell of the map the round was
//! built from, change the map's level keys, then `PLAY` a fresh round on
//! it. Presentation-layer only, same category as `game.rs` - never
//! touches physics/AI/hecs. `mode::Session` owns one next to the `Game`
//! and `main.rs` drives whichever mode is live.
//!
//! **Input arrives as a plain `BuilderInput`**, gathered once per frame by
//! `main.rs` exactly like `simulation::Input` is for `Game::update`: a
//! `RaylibHandle` is only ever needed to draw. That is what lets the dev
//! server's `click`/`key`/`builder_*` tools feed the same struct a mouse
//! or a finger produces, and the tests below run headlessly.
//!
//! The edit model (strokes with the toggle-erase rule, the MAP settings,
//! load/reset, the undo stack in `history.rs`) is the part every input
//! path shares; the bar, the dropdowns and the settings panel are the
//! chrome on top.

pub mod history;

use rand::RngExt;
use sola_raylib::prelude::*;

use crate::ground::{self, GroundGrid};
use crate::hud::{mode_button_rect, BAR_FILL, DIM, HUD_LABEL_SIZE, HUD_TEXT_SIZE, TEXT};
use crate::level::{Mission, SpawnKind, Tier};
use crate::map::{self, CellObject, MapEntry, MapFile};
use crate::obstacle::{self, Drum, Material};
use crate::pickup::PickupKind;
use crate::tank::TankKind;
use crate::{
    EDITOR_BAR_HIT_SLACK,
    EDITOR_DROPDOWN_ROW_H,
    EDITOR_DROPDOWN_W,
    EDITOR_PANEL_BORDER_OPACITY,
    EDITOR_PANEL_BORDER_THICKNESS,
    EDITOR_PANEL_FILL,
    EDITOR_PANEL_FILL_OPACITY,
    EDITOR_PANEL_ROUNDNESS,
    EDITOR_PANEL_SEGMENTS,
    EDITOR_PANEL_SHADOW_OFFSET,
    EDITOR_PANEL_SHADOW_OPACITY,
    EDITOR_SETTINGS_W,
    EDITOR_STEPPER_SIZE,
    EDITOR_TOOLBAR_MARGIN,
    Layout,
    OBSTACLE_GRID_SIZE,
    PATHFIND_CELL_SIZE,
    Position,
    Rect,
};
pub use history::{CellChange, EditStep, MapDiff, MapSettings, UndoStack};

/// The builder's accent: the amber the `BUILD` label, the active
/// category's outline and the mode button share (`hud::BUILD_COLOR`).
pub const BUILD_ACCENT: Color = crate::hud::BUILD_COLOR;

// Bar slots along the build bar (docs/game-editor-fusion.md section 7):
// x offsets from the panel's left, fixed so a name or a readout changing
// width never nudges the buttons after it. `bar_slot_tests` pins that
// they do not overlap and all end before the SAVE/PLAY buttons.
const SLOT_BUILD: f32 = 8.0;
const SLOT_NAME: f32 = 72.0;
const NAME_W: f32 = 160.0;
const SLOT_CATEGORIES: f32 = 240.0;
const CATEGORY_W: f32 = 100.0;
const SLOT_ERASE: f32 = 748.0;
const SLOT_UNDO: f32 = 796.0;
const SLOT_REDO: f32 = 840.0;
const SMALL_BUTTON_W: f32 = 40.0;
const SLOT_FILE: f32 = 888.0;
const SLOT_MAP: f32 = 960.0;
/// FILE and MAP share a width.
const MAP_BUTTON_W: f32 = 64.0;
const SLOT_CURSOR: f32 = 1032.0;
/// The cursor readout's budget: `col,row` plus the longest short label.
const CURSOR_W: f32 = 160.0;
/// How many rows the Load list shows at once; the wheel scrolls the rest.
const LOAD_VISIBLE_ROWS: usize = 12;
const LOAD_PANEL_W: f32 = 360.0;
/// The gap a button's drawn box keeps from the slot after it, so two
/// adjacent outlines never touch.
const BUTTON_GAP: f32 = 8.0;
/// The category button's text column, right of its full-bleed icon.
const CATEGORY_TEXT_X: i32 = 36;
/// The category caret's left edge: flush with the button's drawn box,
/// two blocks in from the active outline, so the 10 px name (`GROUND`
/// is the widest) ends clear of it.
const CATEGORY_CARET_X: i32 = (CATEGORY_W - BUTTON_GAP) as i32 - CARET_W;
/// A caret is 10 px wide (`draw_caret`).
const CARET_W: i32 = 10;
/// A dropdown row's name column, right of its 32 px icon at 8 px inset.
const DROPDOWN_TEXT_X: i32 = 48;
/// The icons in the bar and the dropdown rows, the sheets' own 32 px.
const ICON_PX: f32 = 32.0;
/// The settings panel's row layout: label at the left inset, the `<`
/// button, the value, the `>` button at the right inset.
const SETTINGS_INSET: f32 = 4.0;
const SETTINGS_DEC_X: f32 = 124.0;
const SETTINGS_VALUE_X: i32 = 180;
const SETTINGS_LABEL_SIZE: i32 = 16;

/// The sprite atlases the builder needs to draw placed objects and their
/// bar/dropdown icons - a small subset of `game::Textures`, plus the one
/// texture only the builder uses (`eraser`, see docs/map-editor-design.md).
pub struct EditorTextures<'a> {
    pub obstacles: &'a Texture2D,
    /// The props sheet (`obstacle::Sheet::Props`).
    pub props: &'a Texture2D,
    pub ground: &'a Texture2D,
    /// static/nature_sheet.png - tall grass (`grass.rs`).
    pub grass: &'a Texture2D,
    pub frog_idle: &'a Texture2D,
    pub pickup_health: &'a Texture2D,
    pub pickup_ammo: &'a Texture2D,
    pub pickup_laser: &'a Texture2D,
    pub pickup_minigun: &'a Texture2D,
    pub pickup_plasma: &'a Texture2D,
    pub pickup_speedup: &'a Texture2D,
    pub pickup_shield: &'a Texture2D,
    pub eraser: &'a Texture2D,
    pub tanks: &'a Texture2D,
    pub trees: &'a Texture2D,
}

/// One frame of raw builder input, in **window** pixels (the bar
/// included). `main.rs` fills it from the mouse, the touch screen and the
/// keyboard; the dev server fills it from `click`/`key` requests. Nothing
/// in this module reads raylib input directly.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuilderInput {
    /// Where the pointer is this frame: the mouse, or the finger while it
    /// touches. `None` on a touch screen with nothing touching.
    pub pointer: Option<Vector2>,
    /// The primary button (left mouse, a finger) went down this frame.
    pub pressed: bool,
    /// The primary button is down this frame (including the press frame).
    pub held: bool,
    /// The secondary button (right mouse) went down / is down.
    pub right_pressed: bool,
    pub right_held: bool,
    /// Mouse wheel movement, positive away from the user.
    pub wheel: f32,
    pub escape: bool,
    pub enter: bool,
    pub backspace: bool,
    /// Ctrl+Z / Ctrl+Y (Cmd on macOS).
    pub undo: bool,
    pub redo: bool,
    /// Characters typed this frame, for the dev Save prompt.
    pub typed: String,
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
    Eraser,
}

/// Every brush, in bar order: the categories one after another, the
/// eraser last.
pub const TOOLS: [Tool; 27] = [
    Tool::Wall(Material::Brick),
    Tool::Wall(Material::Iron),
    Tool::Wall(Material::Wood),
    Tool::Wall(Material::Glass),
    Tool::Prop(Material::Sandbag),
    Tool::Prop(Material::Barrel),
    Tool::Drum(Drum::Oil),
    Tool::Drum(Drum::Fuel),
    Tool::Prop(Material::Fence),
    Tool::Prop(Material::Tree),
    Tool::Prop(Material::Pine),
    Tool::Road,
    Tool::TallGrass,
    Tool::OilTrail,
    Tool::Gate,
    Tool::Start,
    Tool::Start2,
    Tool::Frog,
    Tool::EnemyFrog,
    Tool::Pickup(PickupKind::Health),
    Tool::Pickup(PickupKind::Ammo),
    Tool::Pickup(PickupKind::Laser),
    Tool::Pickup(PickupKind::Minigun),
    Tool::Pickup(PickupKind::Plasma),
    Tool::Pickup(PickupKind::SpeedUp),
    Tool::Pickup(PickupKind::Shield),
    Tool::Eraser,
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
            Tool::TallGrass => "tall_grass",
            Tool::Gate => "gate",
            Tool::Start => "start",
            Tool::Start2 => "start2",
            Tool::Frog => "frog",
            Tool::EnemyFrog => "enemy_frog",
            Tool::Pickup(PickupKind::Health) => "health",
            Tool::Pickup(PickupKind::Ammo) => "ammo",
            Tool::Pickup(PickupKind::Laser) => "laser",
            Tool::Pickup(PickupKind::Minigun) => "minigun",
            Tool::Pickup(PickupKind::Plasma) => "plasma",
            Tool::Pickup(PickupKind::SpeedUp) => "speedup",
            Tool::Pickup(PickupKind::Shield) => "shield",
            Tool::Eraser => "eraser",
        }
    }

    pub fn parse(name: &str) -> Option<Tool> {
        TOOLS.iter().copied().find(|t| t.name() == name)
    }

    /// The category this tool sits in (`None` for the eraser).
    pub fn category(self) -> Option<Category> {
        match self {
            Tool::Wall(_) => Some(Category::Wall),
            Tool::Prop(_) | Tool::Drum(_) => Some(Category::Prop),
            Tool::Road | Tool::TallGrass | Tool::OilTrail | Tool::Gate => Some(Category::Ground),
            Tool::Start | Tool::Start2 | Tool::Frog | Tool::EnemyFrog => Some(Category::Actor),
            Tool::Pickup(_) => Some(Category::Pickup),
            Tool::Eraser => None,
        }
    }

    /// The cell object this brush paints; `None` for the eraser.
    pub fn object(self) -> Option<CellObject> {
        match self {
            Tool::Wall(material) => Some(CellObject::Wall { material }),
            Tool::Prop(material) => CellObject::prop(material),
            Tool::Drum(drum) => Some(CellObject::Barrel { drum: Some(drum) }),
            Tool::OilTrail => Some(CellObject::Oil),
            Tool::Road => Some(CellObject::Road),
            Tool::Frog => Some(CellObject::Frog),
            Tool::Start => Some(CellObject::Start),
            Tool::Start2 => Some(CellObject::Start2),
            Tool::EnemyFrog => Some(CellObject::EnemyFrog),
            Tool::Gate => Some(CellObject::Gate),
            Tool::Pickup(pickup) => Some(CellObject::Pickup { pickup }),
            Tool::TallGrass => Some(CellObject::TallGrass),
            Tool::Eraser => None,
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

    pub fn label(self) -> &'static str {
        match self {
            Category::Wall => "WALL",
            Category::Prop => "PROP",
            Category::Ground => "GROUND",
            Category::Actor => "ACTOR",
            Category::Pickup => "PICKUP",
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

/// The one popup the builder can have open at a time: opening any of
/// them closes the others (docs/game-editor-fusion.md section 7).
enum Popup {
    /// A category's tool list, below its bar button.
    Dropdown(Category),
    /// The MAP settings panel, below the MAP button.
    Settings,
    /// The FILE menu, below the FILE button.
    File,
    /// The Load list over the field: every map `map::available_maps`
    /// offers, `scroll` rows in.
    Load { entries: Vec<MapEntry>, scroll: usize },
    /// The Save-as prompt (native only).
    Save { name: String },
}

/// The FILE menu's rows. `SAVE` and `SAVE AS` exist only where a file can
/// be written (`map::saving_available`); the web build's menu is `LOAD`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileRow {
    Load,
    Save,
    SaveAs,
}

impl FileRow {
    fn all() -> &'static [FileRow] {
        if map::saving_available() {
            &[FileRow::Load, FileRow::Save, FileRow::SaveAs]
        } else {
            &[FileRow::Load]
        }
    }

    fn label(self) -> &'static str {
        match self {
            FileRow::Load => "LOAD...",
            FileRow::Save => "SAVE",
            FileRow::SaveAs => "SAVE AS...",
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
}

/// A press-drag-release in progress on the field.
struct Stroke {
    /// Decided on the first cell (docs/game-editor-fusion.md section 8)
    /// and held for the whole drag.
    erase: bool,
    /// The cell painted last, so a held-but-not-moved frame does not
    /// re-place it every frame.
    last_cell: (i32, i32),
    changes: Vec<CellChange>,
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

pub struct MapEditor {
    map: MapFile,
    /// The map as it was when the builder was seeded or last loaded -
    /// what `dirty` compares against and what RESET returns to.
    baseline: MapFile,
    /// Each category's current tool, in `Category::ALL` order.
    current: [Tool; 5],
    active_tool: Tool,
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
    /// The last pointer position `update` saw, in window pixels - the
    /// hover highlight and the cursor readout draw from it, so a touch
    /// screen keeps showing the last tapped cell.
    pointer: Option<Vector2>,
    pub cli_overrides: CliOverrides,
}

impl MapEditor {
    /// Seed the canvas from `map` - the map the current round was built
    /// from, which becomes the baseline. `width` x `height` is the
    /// battlefield, not the window.
    pub fn new(map: MapFile, width: f32, height: f32) -> Self {
        let current = [
            Tool::Wall(Material::Brick),
            Tool::Prop(Material::Sandbag),
            Tool::Road,
            Tool::Start,
            Tool::Pickup(PickupKind::Health),
        ];
        let mut editor = MapEditor {
            baseline: map.clone(),
            map,
            current,
            active_tool: current[0],
            ground: GroundGrid::default(),
            ground_seed: rand::rng().random(),
            popup: None,
            status: None,
            stroke: None,
            history: UndoStack::default(),
            pointer: None,
            cli_overrides: CliOverrides::default(),
        };
        editor.rebuild_ground(width, height);
        editor
    }

    // ----- the edit model: shared by the bar, the finger and the tools -----

    pub fn map(&self) -> &MapFile {
        &self.map
    }

    pub fn baseline(&self) -> &MapFile {
        &self.baseline
    }

    /// The map's display name: its file stem, `default`, or `untitled`.
    pub fn name(&self) -> &str {
        self.map.name.as_deref().unwrap_or("untitled")
    }

    /// Edited since it was seeded or last loaded.
    pub fn dirty(&self) -> bool {
        !self.diff().is_empty()
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

    /// Make `tool` the active brush and its category's current tool.
    pub fn select_tool(&mut self, tool: Tool) {
        if let Some(category) = tool.category() {
            self.current[category.index()] = tool;
        }
        self.active_tool = tool;
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

    pub fn undo(&mut self, width: f32, height: f32) -> Option<EditStep> {
        self.finish_stroke();
        let step = self.history.undo(&mut self.map)?;
        self.rebuild_ground(width, height);
        Some(step)
    }

    pub fn redo(&mut self, width: f32, height: f32) -> Option<EditStep> {
        self.finish_stroke();
        let step = self.history.redo(&mut self.map)?;
        self.rebuild_ground(width, height);
        Some(step)
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
        self.finish_stroke();
        after.write_to(&mut self.map);
        self.history.push(EditStep::Settings { before, after });
    }

    /// Replace the canvas with `map` and make it the new baseline, as one
    /// undo step - the dev server's `builder_map {map_toml}`.
    pub fn load(&mut self, map: MapFile, width: f32, height: f32) {
        self.finish_stroke();
        let before = self.map.clone();
        self.map = map;
        self.baseline = self.map.clone();
        self.history.push(EditStep::Map { before: Box::new(before), after: Box::new(self.map.clone()) });
        self.rebuild_ground(width, height);
    }

    /// Revert cells and settings to the baseline, as one undo step.
    pub fn reset(&mut self, width: f32, height: f32) {
        self.finish_stroke();
        if !self.dirty() {
            return;
        }
        let before = self.map.clone();
        let name = self.map.name.take();
        self.map = self.baseline.clone();
        self.map.name = name;
        self.history.push(EditStep::Map { before: Box::new(before), after: Box::new(self.map.clone()) });
        self.rebuild_ground(width, height);
    }

    /// One whole stroke at once: a press on `cells[0]`, a drag through the
    /// rest, a release. `right` is the secondary button (always erase).
    /// Replies with what changed. The tools' entry point; the UI drives
    /// `begin_stroke`/`stroke_to`/`finish_stroke` frame by frame instead.
    pub fn stroke(&mut self, cells: &[(i32, i32)], right: bool, width: f32, height: f32) -> Vec<CellChange> {
        let Some(&first) = cells.first() else { return Vec::new() };
        self.finish_stroke();
        self.begin_stroke(first, right, width, height);
        for &cell in &cells[1..] {
            self.stroke_to(cell, width, height);
        }
        self.finish_stroke_changes()
    }

    /// The first cell of a press: decides paint or erase for the whole
    /// stroke. Erase when the secondary button is down, the eraser is the
    /// brush, or the cell already holds exactly the brush's object.
    fn begin_stroke(&mut self, cell: (i32, i32), right: bool, width: f32, height: f32) {
        let erase = right
            || self.active_tool == Tool::Eraser
            || self.active_tool.object().is_some_and(|obj| self.map.cell(cell.0, cell.1) == Some(&obj));
        self.stroke = Some(Stroke { erase, last_cell: cell, changes: Vec::new() });
        self.paint(cell, width, height);
    }

    /// The drag crossed into `cell` (or stayed on the last one, a no-op).
    fn stroke_to(&mut self, cell: (i32, i32), width: f32, height: f32) {
        let Some(stroke) = &mut self.stroke else { return };
        if stroke.last_cell == cell {
            return;
        }
        stroke.last_cell = cell;
        self.paint(cell, width, height);
    }

    fn finish_stroke(&mut self) {
        self.finish_stroke_changes();
    }

    fn finish_stroke_changes(&mut self) -> Vec<CellChange> {
        let Some(stroke) = self.stroke.take() else { return Vec::new() };
        let changes = stroke.changes;
        if !changes.is_empty() {
            self.history.push(EditStep::Cells(changes.clone()));
        }
        changes
    }

    /// Apply the stroke's mode to one cell and record what changed.
    fn paint(&mut self, (col, row): (i32, i32), width: f32, height: f32) {
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
                let old = match self.active_tool {
                    Tool::Frog => self.map.frog_cell(),
                    Tool::Start => self.map.start_cell(),
                    Tool::Start2 => self.map.start2_cell(),
                    Tool::EnemyFrog => self.map.enemy_frog_cell(),
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
        if let Some(stroke) = &mut self.stroke {
            stroke.changes.extend(changes);
        }
        self.rebuild_ground(width, height);
    }

    /// Recompute the decorative ground layer from the map's current wall +
    /// road cells - every wall cell paints road under itself automatically,
    /// same as a live round (`Game::init`'s "road_cells = obstacle_positions
    /// + explicit road cells" convention), plus every cell explicitly
    /// placed with the Road tool. Called after every cell edit.
    fn rebuild_ground(&mut self, width: f32, height: f32) {
        let road_cells: Vec<Position> = self
            .map
            .iter_cells()
            .filter(|(_, _, obj)| matches!(obj, CellObject::Wall { .. } | CellObject::Road))
            .map(|(col, row, _)| map::cell_to_world(col, row))
            .collect();
        let wall_cells: Vec<Position> = self
            .map
            .iter_cells()
            .filter(|(_, _, obj)| matches!(obj, CellObject::Wall { .. }))
            .map(|(col, row, _)| map::cell_to_world(col, row))
            .collect();
        self.ground = ground::build(width, height, self.ground_seed, &road_cells, &wall_cells);
    }

    // ----- the chrome -----

    /// Write the map to `maps/<name>.toml` (`name` defaults to the map's
    /// own name) and make the saved state the baseline. Native only:
    /// `map::saving_available` is false on the web. Returns the status
    /// line to show.
    pub fn save(&mut self, name: Option<&str>) -> Result<String, String> {
        if !map::saving_available() {
            return Err("saving is not available in this build: edits stay in memory for the session".to_string());
        }
        let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => n.to_string(),
            None => self.map.name.clone().ok_or("the map has no name yet: use SAVE AS")?,
        };
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(format!("map name {name:?} may only use letters, digits, - and _"));
        }
        self.finish_stroke();
        let path = map::maps_dir().join(format!("{name}.toml"));
        self.map.save(&path)?;
        self.map.name = Some(name.clone());
        self.baseline = self.map.clone();
        let line = format!("saved {name}.toml");
        self.status = Some(line.clone());
        Ok(line)
    }

    /// Load a map from the Load list (`map::open_map`) into the canvas as
    /// one undo step and the new baseline.
    pub fn load_named(&mut self, name: &str, width: f32, height: f32) -> Result<(), String> {
        let map = map::open_map(name)?;
        self.load(map, width, height);
        self.status = Some(format!("loaded {name}"));
        Ok(())
    }

    /// The Save-as prompt is taking text: the keys a player types are
    /// characters, not commands (`Tab` must not start a round).
    pub fn text_entry_open(&self) -> bool {
        matches!(self.popup, Some(Popup::Save { .. }))
    }

    /// Close whatever popup is open (a dropdown, the settings panel or
    /// the Save prompt) without acting on it - leaving the mode does this,
    /// so the canvas never comes back with a stale menu over it.
    pub fn close_popup(&mut self) {
        self.popup = None;
    }

    /// Which popup is open, as the dev server's `mode` tool spells it: a
    /// category's dropdown by the category's name, `map` for the settings
    /// panel, `save` for the dev Save prompt.
    pub fn open_menu(&self) -> Option<&'static str> {
        match &self.popup {
            None => None,
            Some(Popup::Dropdown(category)) => Some(match category {
                Category::Wall => "wall",
                Category::Prop => "prop",
                Category::Ground => "ground",
                Category::Actor => "actor",
                Category::Pickup => "pickup",
            }),
            Some(Popup::Settings) => Some("map"),
            Some(Popup::File) => Some("file"),
            Some(Popup::Load { .. }) => Some("load"),
            Some(Popup::Save { .. }) => Some("save"),
        }
    }

    // --- bar geometry, window space ---

    /// A full-height bar button `w` wide at `x` from the panel's left.
    fn bar_button(panel: Rect, x: f32, w: f32) -> Rectangle {
        Rectangle::new(panel.x + x, panel.y, w, panel.h)
    }

    fn category_rect(layout: &Layout, category: Category) -> Rectangle {
        Self::bar_button(layout.panel, SLOT_CATEGORIES + category.index() as f32 * CATEGORY_W, CATEGORY_W)
    }

    fn erase_rect(layout: &Layout) -> Rectangle {
        Self::bar_button(layout.panel, SLOT_ERASE, SMALL_BUTTON_W)
    }

    fn undo_rect(layout: &Layout) -> Rectangle {
        Self::bar_button(layout.panel, SLOT_UNDO, SMALL_BUTTON_W)
    }

    fn redo_rect(layout: &Layout) -> Rectangle {
        Self::bar_button(layout.panel, SLOT_REDO, SMALL_BUTTON_W)
    }

    fn map_rect(layout: &Layout) -> Rectangle {
        Self::bar_button(layout.panel, SLOT_MAP, MAP_BUTTON_W)
    }

    fn file_rect(layout: &Layout) -> Rectangle {
        Self::bar_button(layout.panel, SLOT_FILE, MAP_BUTTON_W)
    }

    /// The bar button under a window position, if any. Every button's
    /// hit rect reaches `EDITOR_BAR_HIT_SLACK` above and below its drawn
    /// box, so a slightly low tap on a phone still lands.
    fn bar_button_at(point: Vector2, layout: &Layout) -> Option<BarButton> {
        let on = |rect: Rectangle| hit_rect(rect).check_collision_point_rec(point);
        if on(mode_button_rect(layout.panel)) {
            return Some(BarButton::Play);
        }
        if on(Self::file_rect(layout)) {
            return Some(BarButton::File);
        }
        for category in Category::ALL {
            let rect = Self::category_rect(layout, category);
            if on(rect) {
                // The icon half selects, the name/caret half opens the list.
                return Some(if point.x < rect.x + rect.width / 2.0 {
                    BarButton::CategoryIcon(category)
                } else {
                    BarButton::CategoryMenu(category)
                });
            }
        }
        if on(Self::erase_rect(layout)) {
            return Some(BarButton::Erase);
        }
        if on(Self::undo_rect(layout)) {
            return Some(BarButton::Undo);
        }
        if on(Self::redo_rect(layout)) {
            return Some(BarButton::Redo);
        }
        if on(Self::map_rect(layout)) {
            return Some(BarButton::Map);
        }
        None
    }

    // --- popup geometry, window space ---

    /// A category's dropdown: below its button, over the field.
    fn dropdown_rect(layout: &Layout, category: Category) -> Rectangle {
        let button = Self::category_rect(layout, category);
        let rows = category.tools().count() as f32;
        Rectangle::new(button.x, layout.panel.y + layout.panel.h, EDITOR_DROPDOWN_W, rows * EDITOR_DROPDOWN_ROW_H)
    }

    fn dropdown_row_rect(layout: &Layout, category: Category, index: usize) -> Rectangle {
        let list = Self::dropdown_rect(layout, category);
        Rectangle::new(list.x, list.y + index as f32 * EDITOR_DROPDOWN_ROW_H, list.width, EDITOR_DROPDOWN_ROW_H)
    }

    /// The MAP settings panel: below the MAP button, over the field.
    fn settings_rect(layout: &Layout) -> Rectangle {
        let button = Self::map_rect(layout);
        // Anchored to its button but never past the field's right edge:
        // MAP sits close to the end of the bar, so the panel slides left.
        let right = layout.field.x + layout.field.w;
        Rectangle::new(
            button.x.min(right - EDITOR_SETTINGS_W),
            layout.panel.y + layout.panel.h,
            EDITOR_SETTINGS_W,
            SETTINGS_ROWS.len() as f32 * EDITOR_DROPDOWN_ROW_H,
        )
    }

    fn settings_row_rect(layout: &Layout, index: usize) -> Rectangle {
        let panel = Self::settings_rect(layout);
        Rectangle::new(panel.x, panel.y + index as f32 * EDITOR_DROPDOWN_ROW_H, panel.width, EDITOR_DROPDOWN_ROW_H)
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

    /// The FILE menu: below the FILE button, over the field.
    fn file_menu_rect(layout: &Layout) -> Rectangle {
        let button = Self::file_rect(layout);
        let rows = FileRow::all().len() as f32;
        Rectangle::new(button.x, layout.panel.y + layout.panel.h, EDITOR_DROPDOWN_W, rows * EDITOR_DROPDOWN_ROW_H)
    }

    fn file_row_rect(layout: &Layout, index: usize) -> Rectangle {
        let menu = Self::file_menu_rect(layout);
        Rectangle::new(menu.x, menu.y + index as f32 * EDITOR_DROPDOWN_ROW_H, menu.width, EDITOR_DROPDOWN_ROW_H)
    }

    /// The Load list: centred over the field, one row per visible map.
    fn load_panel_rect(layout: &Layout, entries: usize) -> Rectangle {
        let rows = entries.clamp(1, LOAD_VISIBLE_ROWS) as f32;
        let h = rows * EDITOR_DROPDOWN_ROW_H;
        let origin = layout.field_origin();
        Rectangle::new(
            origin.x + (layout.field.w - LOAD_PANEL_W) / 2.0,
            origin.y + ((layout.field.h - h) / 2.0).max(0.0),
            LOAD_PANEL_W,
            h,
        )
    }

    /// The `index`-th visible row of the Load list.
    fn load_row_rect(panel: Rectangle, index: usize) -> Rectangle {
        Rectangle::new(panel.x, panel.y + index as f32 * EDITOR_DROPDOWN_ROW_H, panel.width, EDITOR_DROPDOWN_ROW_H)
    }

    /// The open popup's panel, if one is open.
    fn popup_rect(&self, layout: &Layout) -> Option<Rectangle> {
        match &self.popup {
            None => None,
            Some(Popup::Dropdown(category)) => Some(Self::dropdown_rect(layout, *category)),
            Some(Popup::Settings) => Some(Self::settings_rect(layout)),
            Some(Popup::File) => Some(Self::file_menu_rect(layout)),
            Some(Popup::Load { entries, .. }) => Some(Self::load_panel_rect(layout, entries.len())),
            Some(Popup::Save { .. }) => Some(Self::save_prompt_rect(layout)),
        }
    }

    fn save_prompt_rect(layout: &Layout) -> Rectangle {
        let origin = layout.field_origin();
        Rectangle::new(origin.x + layout.field.w / 2.0 - 150.0, origin.y + layout.field.h / 2.0 - 40.0, 300.0, 80.0)
    }

    /// Whether a window position lands on the builder's own chrome (the
    /// bar, or the open popup over the field) - a press there never
    /// paints the cell behind it and the hover highlight hides.
    fn point_on_ui(&self, point: Vector2, layout: &Layout) -> bool {
        layout.panel.contains(point) || self.popup_rect(layout).is_some_and(|r| r.check_collision_point_rec(point))
    }

    /// The field cell under the last pointer position, when that is on
    /// the field and not on chrome - what the hover highlight and the
    /// cursor readout show.
    fn cursor_cell(&self, layout: &Layout) -> Option<(i32, i32)> {
        let pointer = self.pointer?;
        let (width, height) = (layout.field.w, layout.field.h);
        let m = layout.to_field(pointer);
        if self.point_on_ui(pointer, layout) || m.x < 0.0 || m.x >= width || m.y < 0.0 || m.y >= height {
            return None;
        }
        Some(map::world_to_cell(m))
    }

    // --- input ---

    /// Advance one frame on `input`: the open popup, or a press on the
    /// bar, or a stroke on the field. Returns `EditorAction::Play` the
    /// frame PLAY is pressed.
    pub fn update(&mut self, input: &BuilderInput, layout: &Layout) -> EditorAction {
        let (width, height) = (layout.field.w, layout.field.h);
        if let Some(p) = input.pointer {
            self.pointer = Some(p);
        }
        // The keyboard shortcuts work under a menu too, but not while the
        // Save prompt is taking text.
        if !matches!(self.popup, Some(Popup::Save { .. })) {
            if input.undo {
                self.undo(width, height);
            }
            if input.redo {
                self.redo(width, height);
            }
        }
        if self.popup.is_some() {
            self.update_popup(input, layout);
            return EditorAction::None;
        }

        // The wheel over a category button steps its tool; rolling the
        // wheel towards you walks down the list.
        if input.wheel != 0.0 {
            if let Some(pointer) = input.pointer {
                let over = Category::ALL
                    .into_iter()
                    .find(|&c| hit_rect(Self::category_rect(layout, c)).check_collision_point_rec(pointer));
                if let Some(category) = over {
                    self.cycle_tool(category, input.wheel < 0.0);
                }
            }
        }

        let primary = input.held || input.right_held;
        if !primary {
            // A release ends the stroke - one undo step per press.
            self.finish_stroke();
            return EditorAction::None;
        }
        let Some(pointer) = input.pointer else {
            return EditorAction::None;
        };

        // Chrome only reacts to the press edge, never every frame a drag
        // happens to stay over it.
        if input.pressed {
            if let Some(button) = Self::bar_button_at(pointer, layout) {
                self.finish_stroke();
                return self.press_bar_button(button, width, height);
            }
        }

        // The field: a press begins a stroke, a held button continues it
        // into every new cell it crosses. A press on the bar's empty parts
        // is not a paint either.
        let field_pointer = layout.to_field(pointer);
        let on_field = !self.point_on_ui(pointer, layout)
            && field_pointer.x >= 0.0
            && field_pointer.x < width
            && field_pointer.y >= 0.0
            && field_pointer.y < height;
        if on_field {
            let cell = map::world_to_cell(field_pointer);
            if input.pressed || input.right_pressed {
                self.finish_stroke();
                self.begin_stroke(cell, input.right_held && !input.held, width, height);
            } else if self.stroke.is_some() {
                self.stroke_to(cell, width, height);
            }
        }
        EditorAction::None
    }

    /// What a press on a bar button does.
    fn press_bar_button(&mut self, button: BarButton, width: f32, height: f32) -> EditorAction {
        match button {
            BarButton::Play => return EditorAction::Play,
            BarButton::File => self.popup = Some(Popup::File),
            BarButton::CategoryIcon(category) => self.select_tool(self.current_tool(category)),
            BarButton::CategoryMenu(category) => self.popup = Some(Popup::Dropdown(category)),
            BarButton::Erase => self.select_tool(Tool::Eraser),
            BarButton::Undo => {
                self.undo(width, height);
            }
            BarButton::Redo => {
                self.redo(width, height);
            }
            BarButton::Map => self.popup = Some(Popup::Settings),
        }
        EditorAction::None
    }

    /// Handle input while a popup is open, consuming it entirely: a
    /// press inside the popup works it, a press anywhere else closes it
    /// and does nothing more, `Esc` closes it.
    fn update_popup(&mut self, input: &BuilderInput, layout: &Layout) {
        let Some(popup) = self.popup.take() else { return };
        if input.escape {
            return;
        }
        let (width, height) = (layout.field.w, layout.field.h);
        let pressed = input.pressed || input.right_pressed;
        match popup {
            Popup::Save { mut name } => {
                for c in input.typed.chars() {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        name.push(c);
                    }
                }
                if input.backspace {
                    name.pop();
                }
                if input.enter && !name.is_empty() {
                    if let Err(e) = self.save(Some(&name)) {
                        self.status = Some(e);
                    }
                } else {
                    self.popup = Some(Popup::Save { name });
                }
            }
            Popup::File => {
                let Some(pointer) = input.pointer.filter(|_| pressed) else {
                    self.popup = Some(Popup::File);
                    return;
                };
                // A pick or a press elsewhere: the menu closes either way.
                if input.pressed {
                    let picked = FileRow::all()
                        .iter()
                        .enumerate()
                        .find(|&(i, _)| Self::file_row_rect(layout, i).check_collision_point_rec(pointer))
                        .map(|(_, row)| *row);
                    match picked {
                        Some(FileRow::Load) => {
                            self.popup = Some(Popup::Load { entries: map::available_maps(), scroll: 0 })
                        }
                        Some(FileRow::Save) if self.map.name.is_some() => {
                            if let Err(e) = self.save(None) {
                                self.status = Some(e);
                            }
                        }
                        Some(FileRow::Save) | Some(FileRow::SaveAs) => {
                            self.popup = Some(Popup::Save { name: self.map.name.clone().unwrap_or_default() })
                        }
                        None => {}
                    }
                }
            }
            Popup::Load { entries, mut scroll } => {
                if input.wheel != 0.0 {
                    let max = entries.len().saturating_sub(LOAD_VISIBLE_ROWS);
                    scroll = if input.wheel < 0.0 { (scroll + 1).min(max) } else { scroll.saturating_sub(1) };
                }
                let Some(pointer) = input.pointer.filter(|_| pressed) else {
                    self.popup = Some(Popup::Load { entries, scroll });
                    return;
                };
                let panel = Self::load_panel_rect(layout, entries.len());
                if !panel.check_collision_point_rec(pointer) {
                    return;
                }
                if input.pressed {
                    let picked = entries
                        .iter()
                        .skip(scroll)
                        .take(LOAD_VISIBLE_ROWS)
                        .enumerate()
                        .find(|&(i, _)| Self::load_row_rect(panel, i).check_collision_point_rec(pointer))
                        .map(|(_, e)| e.name.clone());
                    if let Some(name) = picked {
                        if let Err(e) = self.load_named(&name, width, height) {
                            self.status = Some(e);
                        }
                        return;
                    }
                }
                self.popup = Some(Popup::Load { entries, scroll });
            }
            Popup::Dropdown(category) => {
                let Some(pointer) = input.pointer.filter(|_| pressed) else {
                    self.popup = Some(Popup::Dropdown(category));
                    return;
                };
                // A pick or a press elsewhere: either way the list closes
                // and the press goes no further.
                if input.pressed {
                    let picked = category
                        .tools()
                        .enumerate()
                        .find(|&(i, _)| Self::dropdown_row_rect(layout, category, i).check_collision_point_rec(pointer))
                        .map(|(_, tool)| tool);
                    if let Some(tool) = picked {
                        self.select_tool(tool);
                    }
                }
            }
            Popup::Settings => {
                let Some(pointer) = input.pointer.filter(|_| pressed) else {
                    self.popup = Some(Popup::Settings);
                    return;
                };
                if !Self::settings_rect(layout).check_collision_point_rec(pointer) {
                    return;
                }
                if input.pressed {
                    for (i, row) in SETTINGS_ROWS.iter().enumerate() {
                        let rect = Self::settings_row_rect(layout, i);
                        if !rect.check_collision_point_rec(pointer) {
                            continue;
                        }
                        if *row == SettingsRow::Reset {
                            if Self::settings_reset_rect(rect).check_collision_point_rec(pointer) {
                                self.reset(width, height);
                            }
                        } else if Self::settings_dec_rect(rect).check_collision_point_rec(pointer) {
                            self.step_setting(*row, false);
                        } else if Self::settings_inc_rect(rect).check_collision_point_rec(pointer) {
                            self.step_setting(*row, true);
                        }
                    }
                }
                self.popup = Some(Popup::Settings);
            }
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
            SettingsRow::Reset => return,
        }
        self.apply_settings(s);
    }

    // --- drawing ---

    /// Draw the whole builder: ground, placed objects, hover highlight
    /// through a `Camera2D` at the field origin (so every cell position
    /// stays the world position the map format uses), then the bar and
    /// any open popup in window space on top.
    pub fn render(&self, rl: &mut RaylibHandle, thread: &RaylibThread, layout: &Layout, textures: &EditorTextures) {
        let (width, height) = (layout.field.w, layout.field.h);
        let cursor = self.cursor_cell(layout);
        let camera = Camera2D {
            offset: layout.field_origin(),
            target: Vector2::new(0.0, 0.0),
            rotation: 0.0,
            zoom: 1.0,
        };
        let mut d = rl.begin_drawing(thread);
        d.clear_background(Color::new(30, 30, 34, 255));

        d.draw_mode2D(camera, |mut d, _| {
            ground::draw(&mut d, textures.ground, &self.ground);

            for (col, row, obj) in self.map.iter_cells() {
                let pos = map::cell_to_world(col, row);
                let size = OBSTACLE_GRID_SIZE;
                let dest = Rectangle::new(pos.x, pos.y, size, size);
                let origin = Vector2::new(size / 2.0, size / 2.0);
                match *obj {
                    CellObject::Barrel { drum: Some(drum) } => {
                        let src = obstacle::drum_source_rec(drum);
                        d.draw_texture_pro(textures.props, src, dest, origin, 0.0, Color::WHITE);
                    }
                    CellObject::Wall { .. } | CellObject::Sandbag | CellObject::Barrel { .. } | CellObject::Fence => {
                        let material = obj.material().expect("solid cells have a material");
                        let (sheet, src) = obstacle::icon_source_rec(material);
                        d.draw_texture_pro(sheet_texture(textures, sheet), src, dest, origin, 0.0, Color::WHITE);
                    }
                    CellObject::Oil => {
                        let src = obstacle::oil_source_rec(pos);
                        d.draw_texture_pro(textures.props, src, dest, origin, 0.0, Color::WHITE);
                    }
                    CellObject::Tree | CellObject::Pine => {
                        // Drawn at the sprite's own 48px, not the 32px cell,
                        // so the canopy overhang matches a round.
                        let material = obj.material().expect("tree cells have a material");
                        let (sheet, src) = obstacle::icon_source_rec(material);
                        let big = crate::TREE_TEXTURE_SIZE;
                        let dest = Rectangle::new(pos.x, pos.y, big, big);
                        let origin = Vector2::new(big / 2.0, big / 2.0);
                        d.draw_texture_pro(sheet_texture(textures, sheet), src, dest, origin, 0.0, Color::WHITE);
                    }
                    CellObject::Road => {} // already painted into `self.ground`
                    CellObject::TallGrass => {
                        // The round scatters several hashed tufts per cell;
                        // one centred tuft is enough to show the cell is
                        // grassed.
                        let cell = crate::GRASS_TEXTURE_SIZE;
                        let src = Rectangle::new(0.0, 0.0, cell, cell);
                        let scale = cell * crate::tuning::tuning().grass_scale;
                        let at = Rectangle::new(pos.x, pos.y + size / 2.0, scale, scale);
                        d.draw_texture_pro(textures.grass, src, at, Vector2::new(scale / 2.0, scale), 0.0, Color::WHITE);
                    }
                    CellObject::Frog => {
                        let src = Rectangle::new(0.0, 0.0, crate::FROG_TEXTURE_SIZE, crate::FROG_TEXTURE_SIZE);
                        d.draw_texture_pro(textures.frog_idle, src, dest, origin, 0.0, Color::WHITE);
                    }
                    CellObject::Start => {
                        let src = crate::tank::icon_source_rec();
                        d.draw_texture_pro(textures.tanks, src, dest, origin, 0.0, Color::WHITE);
                    }
                    CellObject::Start2 => {
                        draw_player2_ring(&mut d, pos, size / 2.0);
                        let src = crate::tank::icon_source_rec();
                        d.draw_texture_pro(textures.tanks, src, dest, origin, 0.0, Color::WHITE);
                    }
                    CellObject::EnemyFrog => {
                        draw_enemy_ring(&mut d, pos, size / 2.0);
                        let src = Rectangle::new(0.0, 0.0, crate::FROG_TEXTURE_SIZE, crate::FROG_TEXTURE_SIZE);
                        d.draw_texture_pro(textures.frog_idle, src, dest, origin, 0.0, Color::WHITE);
                    }
                    CellObject::Gate => match gate_inward(pos, width, height) {
                        Some(inward) => draw_gate_chevron(&mut d, pos, size, inward),
                        None => d.draw_rectangle_lines_ex(
                            Rectangle::new(pos.x - size / 2.0, pos.y - size / 2.0, size, size),
                            2.0,
                            GATE_COLOR,
                        ),
                    },
                    CellObject::Pickup { pickup } => {
                        let texture = pickup_texture(textures, pickup);
                        let src = Rectangle::new(0.0, 0.0, crate::PICKUP_TEXTURE_SIZE, crate::PICKUP_TEXTURE_SIZE);
                        d.draw_texture_pro(texture, src, dest, origin, 0.0, Color::WHITE);
                    }
                }
            }

            // Hover highlight - no visible grid lines otherwise, per
            // docs/map-editor-design.md. On touch this is the last tapped
            // cell.
            if let Some((col, row)) = cursor {
                let pos = map::cell_to_world(col, row);
                let size = OBSTACLE_GRID_SIZE;
                d.draw_rectangle_lines_ex(
                    Rectangle::new(pos.x - size / 2.0, pos.y - size / 2.0, size, size),
                    2.0,
                    Color::new(255, 255, 255, 160),
                );
            }

            if let Some(status) = &self.status {
                d.draw_text(status, EDITOR_TOOLBAR_MARGIN as i32, (height - 22.0) as i32, 14, Color::LIGHTGRAY);
            }
        });

        self.draw_bar(&mut d, layout, textures, cursor);
        match &self.popup {
            None => {}
            Some(Popup::Dropdown(category)) => self.draw_dropdown(&mut d, layout, textures, *category),
            Some(Popup::Settings) => self.draw_settings(&mut d, layout),
            Some(Popup::File) => Self::draw_file_menu(&mut d, layout),
            Some(Popup::Load { entries, scroll }) => Self::draw_load_list(&mut d, layout, entries, *scroll),
            Some(Popup::Save { name }) => {
                let panel = Self::save_prompt_rect(layout);
                draw_panel(&mut d, panel);
                d.draw_text("Save as:", (panel.x + 12.0) as i32, (panel.y + 10.0) as i32, 16, TEXT);
                d.draw_text(&format!("{name}_"), (panel.x + 12.0) as i32, (panel.y + 34.0) as i32, 18, TEXT);
                d.draw_text(
                    "Enter to save, Esc to cancel",
                    (panel.x + 12.0) as i32,
                    (panel.y + 58.0) as i32,
                    12,
                    Color::GRAY,
                );
            }
        }
    }

    /// Whether the singleton `tool` already has its object on the map -
    /// the badge on its icon.
    pub fn singleton_placed(&self, tool: Tool) -> bool {
        match tool {
            Tool::Frog => self.map.frog_cell().is_some(),
            Tool::Start => self.map.start_cell().is_some(),
            Tool::Start2 => self.map.start2_cell().is_some(),
            Tool::EnemyFrog => self.map.enemy_frog_cell().is_some(),
            _ => false,
        }
    }

    /// The HUD bar in build mode (docs/game-editor-fusion.md section 7):
    /// `BUILD`, the map's name with a `*` while edited, the five category
    /// buttons, ERASE, UNDO, REDO, MAP, the cursor readout, the dev SAVE
    /// button and PLAY at the right end.
    fn draw_bar(&self, d: &mut impl RaylibDraw, layout: &Layout, textures: &EditorTextures, cursor: Option<(i32, i32)>) {
        let panel = layout.panel;
        let (px, py, pw, ph) = (panel.x as i32, panel.y as i32, panel.w as i32, panel.h as i32);
        d.draw_rectangle(px, py, pw, ph, BAR_FILL);
        let text_y = py + (ph - HUD_TEXT_SIZE) / 2;

        d.draw_text("BUILD", px + SLOT_BUILD as i32, text_y, HUD_TEXT_SIZE, BUILD_ACCENT);
        let name = format!("{}{}", self.name(), if self.dirty() { " *" } else { "" });
        d.draw_text(&fit_text(&name, NAME_W, HUD_TEXT_SIZE), px + SLOT_NAME as i32, text_y, HUD_TEXT_SIZE, TEXT);

        for category in Category::ALL {
            self.draw_category_button(d, layout, textures, category);
        }

        let erase = Self::erase_rect(layout);
        draw_tool_icon(d, textures, Tool::Eraser, Rectangle::new(erase.x + 4.0, erase.y, ICON_PX, ICON_PX));
        if self.active_tool == Tool::Eraser {
            active_outline(d, erase.x as i32, py, (SMALL_BUTTON_W - 4.0) as i32, ph, BUILD_ACCENT);
        }

        let undo_color = if self.history.undo_depth() > 0 { TEXT } else { DIM };
        draw_small_button(d, Self::undo_rect(layout), "UNDO", undo_color);
        let redo_color = if self.history.redo_depth() > 0 { TEXT } else { DIM };
        draw_small_button(d, Self::redo_rect(layout), "REDO", redo_color);

        let file_open = matches!(self.popup, Some(Popup::File | Popup::Load { .. } | Popup::Save { .. }));
        draw_menu_button(d, Self::file_rect(layout), "FILE", file_open);
        draw_menu_button(d, Self::map_rect(layout), "MAP", matches!(self.popup, Some(Popup::Settings)));

        if let Some((col, row)) = cursor {
            let under = self.map.cell(col, row).map(cell_label).unwrap_or("");
            let readout = fit_text(&format!("{col},{row} {under}"), CURSOR_W, HUD_TEXT_SIZE);
            d.draw_text(&readout, px + SLOT_CURSOR as i32, text_y, HUD_TEXT_SIZE, DIM);
        }

        crate::hud::draw_mode_button(d, panel, "PLAY", BUILD_ACCENT);
    }

    /// The FILE menu below its button.
    fn draw_file_menu(d: &mut impl RaylibDraw, layout: &Layout) {
        draw_panel(d, Self::file_menu_rect(layout));
        for (i, row) in FileRow::all().iter().enumerate() {
            let rect = Self::file_row_rect(layout, i);
            d.draw_text(row.label(), rect.x as i32 + 16, (rect.y + (rect.height - HUD_TEXT_SIZE as f32) / 2.0) as i32, HUD_TEXT_SIZE, TEXT);
        }
    }

    /// The Load list: one row per map, shipped ones marked, a scroll hint
    /// when there are more than fit.
    fn draw_load_list(d: &mut impl RaylibDraw, layout: &Layout, entries: &[MapEntry], scroll: usize) {
        let panel = Self::load_panel_rect(layout, entries.len());
        draw_panel(d, panel);
        if entries.is_empty() {
            d.draw_text("no maps to load", panel.x as i32 + 16, panel.y as i32 + 15, HUD_TEXT_SIZE, DIM);
            return;
        }
        for (i, entry) in entries.iter().skip(scroll).take(LOAD_VISIBLE_ROWS).enumerate() {
            let row = Self::load_row_rect(panel, i);
            let text_y = (row.y + (row.height - HUD_TEXT_SIZE as f32) / 2.0) as i32;
            d.draw_text(&fit_text(&entry.name, LOAD_PANEL_W - 120.0, HUD_TEXT_SIZE), row.x as i32 + 16, text_y, HUD_TEXT_SIZE, TEXT);
            if !entry.on_disk {
                d.draw_text("shipped", (row.x + row.width - 80.0) as i32, text_y, HUD_TEXT_SIZE, DIM);
            }
        }
        if entries.len() > LOAD_VISIBLE_ROWS {
            let hint = format!("{}-{} of {}  (wheel)", scroll + 1, (scroll + LOAD_VISIBLE_ROWS).min(entries.len()), entries.len());
            d.draw_text(&hint, panel.x as i32 + 16, (panel.y + panel.height - 14.0) as i32, HUD_LABEL_SIZE, DIM);
        }
    }

    /// One category button: the current tool's icon full-bleed at the
    /// left, the category's name and a caret on the top line, the tool's
    /// name on the bottom line, outlined in the accent while the active
    /// brush is one of its tools.
    fn draw_category_button(&self, d: &mut impl RaylibDraw, layout: &Layout, textures: &EditorTextures, category: Category) {
        let rect = Self::category_rect(layout, category);
        let tool = self.current_tool(category);
        let (x, y, h) = (rect.x as i32, rect.y as i32, rect.height as i32);
        draw_tool_icon(d, textures, tool, Rectangle::new(rect.x, rect.y, ICON_PX, ICON_PX));
        if self.singleton_placed(tool) {
            draw_badge(d, rect.x + ICON_PX, rect.y);
        }
        let open = matches!(self.popup, Some(Popup::Dropdown(c)) if c == category);
        let label_color = if open { BUILD_ACCENT } else { DIM };
        d.draw_text(category.label(), x + CATEGORY_TEXT_X, y + 5, HUD_LABEL_SIZE, label_color);
        draw_caret(d, x + CATEGORY_CARET_X, y + 7, label_color);
        d.draw_text(short_label(tool), x + CATEGORY_TEXT_X, y + 17, HUD_LABEL_SIZE, TEXT);
        if self.active_category() == Some(category) {
            active_outline(d, x, y, (CATEGORY_W - BUTTON_GAP) as i32, h, BUILD_ACCENT);
        }
    }

    /// A category's tool list below its button: one row per tool, the
    /// current one highlighted.
    fn draw_dropdown(&self, d: &mut impl RaylibDraw, layout: &Layout, textures: &EditorTextures, category: Category) {
        draw_panel(d, Self::dropdown_rect(layout, category));
        for (i, tool) in category.tools().enumerate() {
            let row = Self::dropdown_row_rect(layout, category, i);
            if tool == self.current_tool(category) {
                let inset = Rectangle::new(row.x + 4.0, row.y + 2.0, row.width - 8.0, row.height - 4.0);
                d.draw_rectangle_rounded(inset, 0.2, EDITOR_PANEL_SEGMENTS, Color::new(255, 255, 255, 40));
            }
            // A 40 px rect: the icon's own 4 px inset makes it 32 px at 8.
            let icon = Rectangle::new(row.x + 4.0, row.y + 4.0, ICON_PX + 8.0, ICON_PX + 8.0);
            draw_tool_icon(d, textures, tool, icon);
            if self.singleton_placed(tool) {
                draw_badge(d, icon.x + icon.width - 4.0, icon.y + 4.0);
            }
            let text_y = row.y as i32 + (EDITOR_DROPDOWN_ROW_H as i32 - HUD_TEXT_SIZE) / 2;
            d.draw_text(label(tool), row.x as i32 + DROPDOWN_TEXT_X, text_y, HUD_TEXT_SIZE, TEXT);
        }
    }

    /// The MAP settings panel (docs/game-editor-fusion.md section 9): a
    /// stepper per map key and the RESET MAP button.
    fn draw_settings(&self, d: &mut impl RaylibDraw, layout: &Layout) {
        draw_panel(d, Self::settings_rect(layout));
        let settings = self.settings();
        let waves_off = settings.spawn == SpawnKind::Band;
        for (i, row) in SETTINGS_ROWS.iter().enumerate() {
            let rect = Self::settings_row_rect(layout, i);
            let text_y = rect.y as i32 + (EDITOR_DROPDOWN_ROW_H as i32 - HUD_TEXT_SIZE) / 2;
            if *row == SettingsRow::Reset {
                let button = Self::settings_reset_rect(rect);
                let color = if self.dirty() { BUILD_ACCENT } else { DIM };
                let inset = Rectangle::new(button.x, button.y + 4.0, button.width, button.height - 8.0);
                d.draw_rectangle_rounded_lines_ex(inset, 0.2, EDITOR_PANEL_SEGMENTS, 2.0, color);
                let w = text_width(row.label(), HUD_TEXT_SIZE);
                d.draw_text(row.label(), (button.x + (button.width - w) / 2.0) as i32, text_y, HUD_TEXT_SIZE, color);
                continue;
            }
            let dim = waves_off && row.is_wave_row();
            let label_color = if dim { Color::new(70, 70, 76, 255) } else { DIM };
            let value_color = if dim { DIM } else { TEXT };
            d.draw_text(row.label(), rect.x as i32 + SETTINGS_INSET as i32, text_y, SETTINGS_LABEL_SIZE, label_color);
            for (button, glyph) in [(Self::settings_dec_rect(rect), "<"), (Self::settings_inc_rect(rect), ">")] {
                let inset = Rectangle::new(button.x + 2.0, button.y + 4.0, button.width - 4.0, button.height - 8.0);
                d.draw_rectangle_rounded_lines_ex(inset, 0.2, EDITOR_PANEL_SEGMENTS, 1.0, Color::new(255, 255, 255, 60));
                let w = text_width(glyph, HUD_TEXT_SIZE);
                d.draw_text(glyph, (button.x + (button.width - w) / 2.0) as i32, text_y, HUD_TEXT_SIZE, value_color);
            }
            let value = row.value(&settings);
            let value_x = rect.x as i32 + SETTINGS_VALUE_X;
            d.draw_text(&value, value_x, text_y, HUD_TEXT_SIZE, value_color);
            if row.cli_override(self.cli_overrides) {
                let x = value_x + text_width(&value, HUD_TEXT_SIZE) as i32 + 4;
                d.draw_text("(cli)", x, text_y + 5, HUD_LABEL_SIZE, DIM);
            }
        }
    }
}

/// One of the bar's buttons, as `bar_button_at` reports a press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BarButton {
    /// The icon half of a category button: select its current tool.
    CategoryIcon(Category),
    /// The name/caret half: open its list.
    CategoryMenu(Category),
    Erase,
    Undo,
    Redo,
    Map,
    File,
    Play,
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
    Reset,
}

const SETTINGS_ROWS: [SettingsRow; 11] = [
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
    SettingsRow::Reset,
];

const MISSIONS: [Mission; 3] = [Mission::Protect, Mission::Hunt, Mission::Destroy];

impl SettingsRow {
    fn label(self) -> &'static str {
        match self {
            SettingsRow::Tanks => "TANKS",
            SettingsRow::Tank => "TANK",
            SettingsRow::Tank2 => "TANK 2",
            SettingsRow::Mission => "MISSION",
            SettingsRow::Spawn => "SPAWN",
            SettingsRow::Waves => "WAVES",
            SettingsRow::Size => "SIZE",
            SettingsRow::Growth => "GROWTH",
            SettingsRow::TierStart => "TIER START",
            SettingsRow::TierEnd => "TIER END",
            SettingsRow::Reset => "RESET MAP",
        }
    }

    /// One of the five rows that only matter to a Waves spawn plan.
    fn is_wave_row(self) -> bool {
        matches!(
            self,
            SettingsRow::Waves | SettingsRow::Size | SettingsRow::Growth | SettingsRow::TierStart | SettingsRow::TierEnd
        )
    }

    /// The row's value as the panel shows it; `auto` for `None`.
    fn value(self, s: &MapSettings) -> String {
        fn auto_or<T>(v: Option<T>, f: impl Fn(T) -> String) -> String {
            v.map(f).unwrap_or_else(|| "auto".to_string())
        }
        match self {
            SettingsRow::Tanks => auto_or(s.tanks, |n| n.to_string()),
            SettingsRow::Tank => auto_or(s.tank, |k| k.name().to_string()),
            SettingsRow::Tank2 => auto_or(s.tank2, |k| k.name().to_string()),
            SettingsRow::Mission => s.mission.name().to_string(),
            SettingsRow::Spawn => s.spawn.name().to_string(),
            SettingsRow::Waves => auto_or(s.waves, |n| n.to_string()),
            SettingsRow::Size => auto_or(s.size, |n| n.to_string()),
            SettingsRow::Growth => auto_or(s.growth, |n| n.to_string()),
            SettingsRow::TierStart => auto_or(s.tier_start, |t| t.name().to_string()),
            SettingsRow::TierEnd => auto_or(s.tier_end, |t| t.name().to_string()),
            SettingsRow::Reset => String::new(),
        }
    }

    /// Whether a CLI flag outranks this row's value at PLAY.
    fn cli_override(self, o: CliOverrides) -> bool {
        match self {
            SettingsRow::Tanks => o.tanks,
            SettingsRow::Tank => o.tank,
            SettingsRow::Tank2 => o.tank2,
            SettingsRow::Mission => o.mission,
            SettingsRow::Spawn => o.spawn,
            SettingsRow::Waves => o.waves,
            SettingsRow::Size => o.wave_size,
            SettingsRow::Growth => o.wave_growth,
            SettingsRow::TierStart => o.tier_start,
            SettingsRow::TierEnd => o.tier_end,
            SettingsRow::Reset => false,
        }
    }
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

/// A bar button's hit rect: its drawn box plus `EDITOR_BAR_HIT_SLACK`
/// above and below (docs/game-editor-fusion.md section 10).
fn hit_rect(rect: Rectangle) -> Rectangle {
    Rectangle::new(rect.x, rect.y - EDITOR_BAR_HIT_SLACK, rect.width, rect.height + 2.0 * EDITOR_BAR_HIT_SLACK)
}

/// The width the default font gives `text` at `size`, near enough to
/// centre or clip by (~0.61 of the size per character).
fn text_width(text: &str, size: i32) -> f32 {
    text.chars().count() as f32 * size as f32 * 0.61
}

/// `text` cut down to what fits in `width` at `size`.
fn fit_text(text: &str, width: f32, size: i32) -> String {
    let max = (width / (size as f32 * 0.61)).floor() as usize;
    if text.chars().count() <= max {
        text.to_string()
    } else {
        text.chars().take(max.saturating_sub(1)).chain(std::iter::once('~')).collect()
    }
}

/// A tool's name as the dropdown rows spell it.
fn label(tool: Tool) -> &'static str {
    match tool {
        Tool::TallGrass => "tall grass",
        Tool::Drum(Drum::Oil) => "oil drum",
        Tool::Drum(Drum::Fuel) => "fuel drum",
        Tool::OilTrail => "oil trail",
        Tool::Start => "p1 start",
        Tool::Start2 => "p2 start",
        Tool::EnemyFrog => "enemy frog",
        Tool::Pickup(PickupKind::SpeedUp) => "speed-up",
        other => other.name(),
    }
}

/// A tool's name at the width the bar's 10 px line and the cursor
/// readout have room for.
fn short_label(tool: Tool) -> &'static str {
    match tool {
        Tool::TallGrass => "grass",
        Tool::Drum(Drum::Oil) => "oil",
        Tool::Drum(Drum::Fuel) => "fuel",
        Tool::OilTrail => "oil",
        Tool::EnemyFrog => "e.frog",
        other => other.name(),
    }
}

/// What the cursor readout calls the object in a cell.
fn cell_label(obj: &CellObject) -> &'static str {
    TOOLS.iter().copied().find(|t| t.object().as_ref() == Some(obj)).map_or("?", short_label)
}

/// The outline marking the active category/eraser: 2 px, inset one
/// block from the bar's top and bottom edges, like the play bar's live
/// weapon slot.
fn active_outline(d: &mut impl RaylibDraw, x: i32, y: i32, w: i32, h: i32, color: Color) {
    d.draw_rectangle_lines_ex(
        Rectangle::new((x - 2) as f32, (y + 2) as f32, (w + 4) as f32, (h - 6) as f32),
        2.0,
        color,
    );
}

/// A down-pointing caret of 2 px blocks, 10 px wide, its top-left at
/// (`x`, `y`).
fn draw_caret(d: &mut impl RaylibDraw, x: i32, y: i32, color: Color) {
    for (i, w) in [10, 6, 2].into_iter().enumerate() {
        d.draw_rectangle(x + (10 - w) / 2, y + i as i32 * 2, w, 2, color);
    }
}

/// The singleton badge: a dot at an icon's top-right corner.
fn draw_badge(d: &mut impl RaylibDraw, right: f32, top: f32) {
    d.draw_circle((right - 5.0) as i32, (top + 5.0) as i32, 4.0, Color::LIME);
}

/// An outlined bar button with a 10 px label centred in it (UNDO/REDO).
/// FILE / MAP: an outlined button with its label and a caret, the label
/// in the accent while its menu is open.
fn draw_menu_button(d: &mut impl RaylibDraw, rect: Rectangle, label: &str, open: bool) {
    let color = if open { BUILD_ACCENT } else { TEXT };
    d.draw_rectangle_rounded_lines_ex(
        Rectangle::new(rect.x, rect.y + 4.0, rect.width - BUTTON_GAP, rect.height - 8.0),
        0.2,
        EDITOR_PANEL_SEGMENTS,
        1.0,
        Color::new(255, 255, 255, 60),
    );
    let text_y = (rect.y + (rect.height - HUD_TEXT_SIZE as f32) / 2.0) as i32;
    d.draw_text(label, rect.x as i32 + 6, text_y, HUD_TEXT_SIZE, color);
    draw_caret(d, rect.x as i32 + 48, (rect.y + (rect.height - 6.0) / 2.0) as i32, color);
}

fn draw_small_button(d: &mut impl RaylibDraw, rect: Rectangle, text: &str, color: Color) {
    let inset = Rectangle::new(rect.x, rect.y + 4.0, rect.width - 4.0, rect.height - 8.0);
    d.draw_rectangle_rounded_lines_ex(inset, 0.2, EDITOR_PANEL_SEGMENTS, 1.0, Color::new(255, 255, 255, 60));
    let w = text_width(text, HUD_LABEL_SIZE);
    d.draw_text(
        text,
        (inset.x + (inset.width - w) / 2.0) as i32,
        (rect.y + (rect.height - HUD_LABEL_SIZE as f32) / 2.0) as i32,
        HUD_LABEL_SIZE,
        color,
    );
}

/// The pickup icon for a kind.
fn pickup_texture<'a>(textures: &EditorTextures<'a>, pickup: PickupKind) -> &'a Texture2D {
    match pickup {
        PickupKind::Health => textures.pickup_health,
        PickupKind::Ammo => textures.pickup_ammo,
        PickupKind::Laser => textures.pickup_laser,
        PickupKind::Minigun => textures.pickup_minigun,
        PickupKind::Plasma => textures.pickup_plasma,
        PickupKind::SpeedUp => textures.pickup_speedup,
        PickupKind::Shield => textures.pickup_shield,
    }
}

/// Draw a rounded, bordered, drop-shadowed panel background - shared by
/// every panel the builder draws (dropdowns, the settings panel, the
/// Save prompt), per
/// docs/map-editor-design.md's "Panel chrome" section.
pub fn draw_panel(d: &mut impl RaylibDraw, rect: Rectangle) {
    let shadow = Rectangle::new(
        rect.x + EDITOR_PANEL_SHADOW_OFFSET,
        rect.y + EDITOR_PANEL_SHADOW_OFFSET,
        rect.width,
        rect.height,
    );
    d.draw_rectangle_rounded(
        shadow,
        EDITOR_PANEL_ROUNDNESS,
        EDITOR_PANEL_SEGMENTS,
        Color::new(0, 0, 0, (255.0 * EDITOR_PANEL_SHADOW_OPACITY) as u8),
    );
    d.draw_rectangle_rounded(
        rect,
        EDITOR_PANEL_ROUNDNESS,
        EDITOR_PANEL_SEGMENTS,
        Color::new(
            EDITOR_PANEL_FILL.0,
            EDITOR_PANEL_FILL.1,
            EDITOR_PANEL_FILL.2,
            (255.0 * EDITOR_PANEL_FILL_OPACITY) as u8,
        ),
    );
    d.draw_rectangle_rounded_lines_ex(
        rect,
        EDITOR_PANEL_ROUNDNESS,
        EDITOR_PANEL_SEGMENTS,
        EDITOR_PANEL_BORDER_THICKNESS,
        Color::new(0, 0, 0, (255.0 * EDITOR_PANEL_BORDER_OPACITY) as u8),
    );
}

/// A tool's icon inside `rect` (4 px inset).
pub fn draw_tool_icon(d: &mut impl RaylibDraw, textures: &EditorTextures, tool: Tool, rect: Rectangle) {
    let dest = Rectangle::new(rect.x + 4.0, rect.y + 4.0, rect.width - 8.0, rect.height - 8.0);
    match tool {
        Tool::Wall(material) | Tool::Prop(material) => {
            let (sheet, src) = obstacle::icon_source_rec(material);
            d.draw_texture_pro(sheet_texture(textures, sheet), src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        Tool::Drum(drum) => {
            let src = obstacle::drum_source_rec(drum);
            d.draw_texture_pro(textures.props, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        Tool::OilTrail => {
            d.draw_rectangle_rounded(dest, 0.15, EDITOR_PANEL_SEGMENTS, Color::new(97, 149, 65, 255));
            let src = obstacle::oil_source_rec(Position::new(0.0, 0.0));
            d.draw_texture_pro(textures.props, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        Tool::Road => {
            d.draw_rectangle_rounded(dest, 0.15, EDITOR_PANEL_SEGMENTS, Color::new(150, 111, 74, 255));
        }
        Tool::Frog => {
            let src = Rectangle::new(0.0, 0.0, crate::FROG_TEXTURE_SIZE, crate::FROG_TEXTURE_SIZE);
            d.draw_texture_pro(textures.frog_idle, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        Tool::Start => {
            let src = crate::tank::icon_source_rec();
            d.draw_texture_pro(textures.tanks, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        Tool::Start2 => {
            let center = Position::new(dest.x + dest.width / 2.0, dest.y + dest.height / 2.0);
            draw_player2_ring(d, center, dest.width / 2.0);
            let src = crate::tank::icon_source_rec();
            d.draw_texture_pro(textures.tanks, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        Tool::EnemyFrog => {
            let center = Position::new(dest.x + dest.width / 2.0, dest.y + dest.height / 2.0);
            draw_enemy_ring(d, center, dest.width / 2.0);
            let src = Rectangle::new(0.0, 0.0, crate::FROG_TEXTURE_SIZE, crate::FROG_TEXTURE_SIZE);
            d.draw_texture_pro(textures.frog_idle, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        Tool::Gate => {
            let center = Position::new(dest.x + dest.width / 2.0, dest.y + dest.height / 2.0);
            draw_gate_chevron(d, center, dest.width, Position::new(1.0, 0.0));
        }
        Tool::TallGrass => {
            // One tuft from the nature sheet on a patch of ground green, so
            // the icon reads as grass-on-grass rather than loose pixels.
            d.draw_rectangle_rounded(dest, 0.15, EDITOR_PANEL_SEGMENTS, Color::new(97, 149, 65, 255));
            let cell = crate::GRASS_TEXTURE_SIZE;
            let src = Rectangle::new(0.0, 0.0, cell, cell);
            d.draw_texture_pro(textures.grass, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        Tool::Pickup(pickup) => {
            let texture = pickup_texture(textures, pickup);
            let src = Rectangle::new(0.0, 0.0, crate::PICKUP_TEXTURE_SIZE, crate::PICKUP_TEXTURE_SIZE);
            d.draw_texture_pro(texture, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        Tool::Eraser => {
            d.draw_texture_pro(
                textures.eraser,
                Rectangle::new(0.0, 0.0, textures.eraser.width() as f32, textures.eraser.height() as f32),
                dest,
                Vector2::new(0.0, 0.0),
                0.0,
                Color::WHITE,
            );
        }
    }
}

/// The atlas a material's icon comes from.
fn sheet_texture<'a>(textures: &EditorTextures<'a>, sheet: obstacle::Sheet) -> &'a Texture2D {
    match sheet {
        obstacle::Sheet::Walls => textures.obstacles,
        obstacle::Sheet::Props => textures.props,
        obstacle::Sheet::Trees => textures.trees,
    }
}

/// The enemy side's marker colour - the red ground ring the game draws
/// under the enemy frog, so an `enemy_frog` cell reads the same here.
const ENEMY_RING_COLOR: Color = Color::new(230, 60, 60, 220);

const GATE_COLOR: Color = Color::ORANGE;

/// A flat red ring of radius `radius` centred on `center`.
fn draw_enemy_ring(d: &mut impl RaylibDraw, center: Position, radius: f32) {
    d.draw_ring(center, radius * 0.75, radius, 0.0, 360.0, 24, ENEMY_RING_COLOR);
    d.draw_circle_v(center, radius * 0.75, Color::new(230, 60, 60, 50));
}

/// Player 2's blue ring, the same shape as the enemy frog's red one, so a
/// `start2` cell reads as "a tank, the blue one" next to player 1's.
fn draw_player2_ring(d: &mut impl RaylibDraw, center: Position, radius: f32) {
    let c = crate::tank::PLAYER2_RING_COLOR;
    d.draw_ring(center, radius * 0.75, radius, 0.0, 360.0, 24, Color::new(c.r, c.g, c.b, 220));
    d.draw_circle_v(center, radius * 0.75, Color::new(c.r, c.g, c.b, 50));
}

/// An orange chevron of overall size `size` at `center`, its point aimed
/// along `inward` (a unit axis vector) - the direction a tank rolling in
/// through the gate travels.
fn draw_gate_chevron(d: &mut impl RaylibDraw, center: Position, size: f32, inward: Position) {
    let half = size / 2.0 - 3.0;
    let side = Position::new(-inward.y, inward.x);
    let tip = center + inward * half;
    let tail = center - inward * (half * 0.4);
    let wing_a = tail + side * half;
    let wing_b = tail - side * half;
    d.draw_line_ex(wing_a, tip, 3.0, GATE_COLOR);
    d.draw_line_ex(wing_b, tip, 3.0, GATE_COLOR);
    d.draw_line_ex(center - inward * half, tip, 3.0, GATE_COLOR);
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
        assert_eq!(per_category + 1, TOOLS.len(), "every tool but the eraser is in a category");
    }

    /// The two start brushes are singletons independently of each other:
    /// moving one leaves the other, and painting one over the other's
    /// cell replaces it (the badge follows).
    #[test]
    fn both_start_brushes_are_independent_singletons() {
        let mut ed = MapEditor::new(MapFile::new(), W, H);
        ed.select_tool(Tool::Start);
        ed.stroke(&[(3, 3)], false, W, H);
        ed.select_tool(Tool::Start2);
        ed.stroke(&[(4, 4)], false, W, H);
        assert!(ed.singleton_placed(Tool::Start) && ed.singleton_placed(Tool::Start2));
        assert_eq!(ed.map().start_cell(), Some((3, 3)));
        assert_eq!(ed.map().start2_cell(), Some((4, 4)));
        let depth = ed.history().undo_depth();
        // A move: the old cell cleared, the new one placed, player 1 untouched.
        ed.stroke(&[(6, 6)], false, W, H);
        assert_eq!(ed.map().start2_cell(), Some((6, 6)));
        assert_eq!(ed.map().cell(4, 4), None);
        assert_eq!(ed.map().start_cell(), Some((3, 3)));
        assert_eq!(ed.history().undo_depth(), depth + 1);
        ed.undo(W, H);
        assert_eq!(ed.map().start2_cell(), Some((4, 4)));
        // Player 1's brush over player 2's cell replaces it.
        ed.select_tool(Tool::Start);
        ed.stroke(&[(4, 4)], false, W, H);
        assert_eq!(ed.map().start_cell(), Some((4, 4)));
        assert_eq!(ed.map().start2_cell(), None);
        assert!(!ed.singleton_placed(Tool::Start2));
        assert_eq!(Tool::parse("start2"), Some(Tool::Start2));
        assert_eq!(cell_label(&CellObject::Start2), "start2");
        assert_eq!(cell_label(&CellObject::Start), "start");
    }

    #[test]
    fn a_stroke_paints_once_per_cell_and_toggle_erases_on_its_first_cell() {
        let mut ed = MapEditor::new(MapFile::new(), W, H);
        ed.select_tool(Tool::Wall(Material::Brick));
        let changes = ed.stroke(&[(10, 5), (11, 5), (11, 5), (12, 5)], false, W, H);
        assert_eq!(changes.len(), 3);
        assert_eq!(ed.map().cell(11, 5), Some(&brick()));
        assert_eq!(ed.history().undo_depth(), 1);

        // A press on a cell holding exactly the brush's object clears it,
        // and the drag keeps clearing - never paints - past empty cells.
        let changes = ed.stroke(&[(11, 5), (12, 5), (13, 5)], false, W, H);
        assert_eq!(changes.len(), 2);
        assert_eq!(ed.map().cell(11, 5), None);
        assert_eq!(ed.map().cell(13, 5), None);

        // A press on an empty cell paints, and painting over a brick with
        // a brick is a no-op inside the same stroke.
        ed.stroke(&[(13, 5), (10, 5)], false, W, H);
        assert_eq!(ed.map().cell(13, 5), Some(&brick()));
        assert_eq!(ed.map().cell(10, 5), Some(&brick()));

        // A different material overwrites rather than toggles.
        ed.select_tool(Tool::Wall(Material::Iron));
        ed.stroke(&[(10, 5)], false, W, H);
        assert_eq!(ed.map().cell(10, 5), Some(&CellObject::Wall { material: Material::Iron }));

        // The secondary button erases whatever the brush.
        ed.stroke(&[(10, 5)], true, W, H);
        assert_eq!(ed.map().cell(10, 5), None);
        assert!(ed.dirty());
    }

    #[test]
    fn singletons_move_and_undo_restores_the_old_cell() {
        let mut ed = MapEditor::new(MapFile::new(), W, H);
        ed.select_tool(Tool::Frog);
        ed.stroke(&[(3, 3)], false, W, H);
        let changes = ed.stroke(&[(6, 6)], false, W, H);
        assert_eq!(changes.len(), 2, "a move is a clear and a placement");
        assert_eq!(ed.map().frog_cell(), Some((6, 6)));
        ed.undo(W, H);
        assert_eq!(ed.map().frog_cell(), Some((3, 3)));
        ed.redo(W, H);
        assert_eq!(ed.map().frog_cell(), Some((6, 6)));
        // Tapping the frog with the frog tool removes it.
        ed.stroke(&[(6, 6)], false, W, H);
        assert_eq!(ed.map().frog_cell(), None);
        assert!(ed.singleton_placed(Tool::Frog) == false);
    }

    #[test]
    fn settings_load_and_reset_are_undo_steps_and_drive_dirty() {
        let mut base = MapFile::new();
        base.set_cell(1, 1, brick());
        base.name = Some("arena".into());
        let mut ed = MapEditor::new(base.clone(), W, H);
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
        ed.stroke(&[(2, 2)], false, W, H);
        assert_eq!(ed.diff().added, 1);
        ed.reset(W, H);
        assert!(!ed.dirty());
        assert_eq!(ed.name(), "arena");
        ed.undo(W, H);
        assert!(ed.dirty());

        let mut other = MapFile::new();
        other.set_cell(5, 5, CellObject::Gate);
        ed.load(other, W, H);
        assert!(!ed.dirty(), "a load is the new baseline");
        assert_eq!(ed.map().cell(5, 5), Some(&CellObject::Gate));
        ed.undo(W, H);
        assert_eq!(ed.map().cell(2, 2), Some(&CellObject::Road));
    }

    #[test]
    fn update_from_input_strokes_on_press_and_hold_and_ends_on_release() {
        let layout = Layout::for_field(W, H);
        let mut ed = MapEditor::new(MapFile::new(), W, H);
        let at = |col: i32, row: i32| {
            let p = map::cell_to_world(col, row);
            Vector2::new(p.x + layout.field.x, p.y + layout.field.y)
        };
        let press = BuilderInput { pointer: Some(at(4, 4)), pressed: true, held: true, ..Default::default() };
        assert_eq!(ed.update(&press, &layout), EditorAction::None);
        let drag = BuilderInput { pointer: Some(at(5, 4)), held: true, ..Default::default() };
        ed.update(&drag, &layout);
        ed.update(&drag, &layout);
        assert_eq!(ed.history().undo_depth(), 0, "the stroke is open until release");
        ed.update(&BuilderInput::default(), &layout);
        assert_eq!(ed.history().undo_depth(), 1);
        assert_eq!(ed.map().cells.len(), 2);
        // Ctrl+Z through the same struct.
        ed.update(&BuilderInput { undo: true, ..Default::default() }, &layout);
        assert_eq!(ed.map().cells.len(), 0);
        // A press on the PLAY slot is the mode switch, not a paint.
        let play = mode_button_rect(layout.panel);
        let on_play = Vector2::new(play.x + 2.0, play.y + 2.0);
        let press = BuilderInput { pointer: Some(on_play), pressed: true, held: true, ..Default::default() };
        assert_eq!(ed.update(&press, &layout), EditorAction::Play);
        assert_eq!(ed.map().cells.len(), 0);
    }

    /// A press-and-release at a window position, the way a click or a
    /// tap arrives over two frames.
    fn click(ed: &mut MapEditor, layout: &Layout, at: Vector2) -> EditorAction {
        let press = BuilderInput { pointer: Some(at), pressed: true, held: true, ..Default::default() };
        let action = ed.update(&press, layout);
        ed.update(&BuilderInput { pointer: Some(at), ..Default::default() }, layout);
        action
    }

    fn center(r: Rectangle) -> Vector2 {
        Vector2::new(r.x + r.width / 2.0, r.y + r.height / 2.0)
    }

    /// The bar's slots must never overlap and must all end before the
    /// SAVE and PLAY buttons at the default width, with the widest thing
    /// each can hold written down here.
    #[test]
    fn bar_slots_fit_the_default_bar_without_overlapping() {
        let ch = 11.0; // default-font advance at 18 px
        assert!(SLOT_BUILD + "BUILD".len() as f32 * ch <= SLOT_NAME);
        assert!(SLOT_NAME + NAME_W <= SLOT_CATEGORIES);
        assert!(SLOT_CATEGORIES + Category::ALL.len() as f32 * CATEGORY_W <= SLOT_ERASE);
        assert!(SLOT_ERASE + SMALL_BUTTON_W <= SLOT_UNDO);
        assert!(SLOT_UNDO + SMALL_BUTTON_W <= SLOT_REDO);
        assert!(SLOT_REDO + SMALL_BUTTON_W <= SLOT_FILE);
        assert!(SLOT_FILE + MAP_BUTTON_W <= SLOT_MAP);
        assert!(SLOT_MAP + MAP_BUTTON_W <= SLOT_CURSOR);
        // The readout's worst case: two-digit coordinates and the longest
        // short label of any placeable object.
        let longest = TOOLS.iter().map(|t| short_label(*t).len()).max().unwrap();
        let readout = format!("{},{} {}", 39, 22, "x".repeat(longest));
        assert!(readout.len() as f32 * ch <= CURSOR_W, "cursor readout {readout:?} is wider than its slot");
        let layout = Layout::for_field(W, H);
        let play = mode_button_rect(layout.panel);
        assert!(SLOT_CURSOR + CURSOR_W <= play.x, "cursor readout runs into PLAY");
        // The FILE menu and the Load list fit the field.
        let menu = MapEditor::file_menu_rect(&layout);
        assert!(menu.y + menu.height <= layout.field.y + layout.field.h);
        let list = MapEditor::load_panel_rect(&layout, 40);
        assert!(list.y >= layout.field.y && list.y + list.height <= layout.field.y + layout.field.h);
        // The category name and the longest short tool name fit the 10 px
        // lines beside the icon: the name ends a gap short of the caret,
        // the tool name short of the outline. The default font advances
        // about 6.5 px per character at 10 px.
        let small = 6.5;
        let name_end = CATEGORY_TEXT_X as f32 + "GROUND".len() as f32 * small;
        assert!(name_end + 4.0 <= CATEGORY_CARET_X as f32, "GROUND runs into its caret");
        assert!(CATEGORY_CARET_X + CARET_W <= (CATEGORY_W - BUTTON_GAP) as i32, "caret leaves the button");
        let text_room = CATEGORY_W - BUTTON_GAP - CATEGORY_TEXT_X as f32;
        assert!(longest as f32 * small <= text_room);
    }

    #[test]
    fn every_bar_button_hit_rect_reaches_into_the_field_gutter() {
        let layout = Layout::for_field(W, H);
        let below = |r: Rectangle| Vector2::new(r.x + r.width / 2.0, r.y + r.height + EDITOR_BAR_HIT_SLACK - 1.0);
        assert_eq!(
            MapEditor::bar_button_at(below(MapEditor::category_rect(&layout, Category::Wall)), &layout),
            Some(BarButton::CategoryMenu(Category::Wall))
        );
        assert_eq!(MapEditor::bar_button_at(below(MapEditor::map_rect(&layout)), &layout), Some(BarButton::Map));
        let far = Vector2::new(SLOT_MAP + 2.0, layout.panel.h + EDITOR_BAR_HIT_SLACK + 1.0);
        assert_eq!(MapEditor::bar_button_at(far, &layout), None);
    }

    #[test]
    fn every_dropdown_and_the_settings_panel_fit_inside_the_field() {
        let layout = Layout::for_field(W, H);
        let inside = |r: Rectangle| {
            r.x >= layout.field.x
                && r.x + r.width <= layout.field.x + layout.field.w
                && r.y >= layout.field.y
                && r.y + r.height <= layout.field.y + layout.field.h
        };
        for category in Category::ALL {
            let list = MapEditor::dropdown_rect(&layout, category);
            assert!(inside(list), "{} dropdown leaves the field: {list:?}", category.label());
            for i in 0..category.tools().count() {
                let row = MapEditor::dropdown_row_rect(&layout, category, i);
                assert!(row.height >= 48.0 && inside(row));
            }
        }
        let settings = MapEditor::settings_rect(&layout);
        assert!(inside(settings), "settings panel leaves the field: {settings:?}");
        for i in 0..SETTINGS_ROWS.len() {
            let row = MapEditor::settings_row_rect(&layout, i);
            let (dec, inc) = (MapEditor::settings_dec_rect(row), MapEditor::settings_inc_rect(row));
            assert!(dec.width >= 48.0 && dec.height >= 48.0 && inc.width >= 48.0 && inc.height >= 48.0);
            assert!(dec.x + dec.width <= inc.x && inside(dec) && inside(inc));
            let reset = MapEditor::settings_reset_rect(row);
            assert!(reset.height >= 48.0 && inside(reset));
        }
    }

    #[test]
    fn a_category_button_selects_on_its_icon_half_and_opens_its_list_on_its_caret_half() {
        let layout = Layout::for_field(W, H);
        let mut ed = MapEditor::new(MapFile::new(), W, H);
        let prop = MapEditor::category_rect(&layout, Category::Prop);
        // The caret half opens the dropdown and selects nothing yet.
        click(&mut ed, &layout, Vector2::new(prop.x + prop.width - 10.0, prop.y + 16.0));
        assert_eq!(ed.open_menu(), Some("prop"));
        assert_eq!(ed.tool(), Tool::Wall(Material::Brick));
        // Picking a row selects that tool, makes it the category's current
        // one, closes the list and paints nothing.
        let row = MapEditor::dropdown_row_rect(&layout, Category::Prop, 1);
        click(&mut ed, &layout, center(row));
        assert_eq!(ed.tool(), Tool::Prop(Material::Barrel));
        assert_eq!(ed.current_tool(Category::Prop), Tool::Prop(Material::Barrel));
        assert_eq!(ed.open_menu(), None);
        assert!(ed.map().cells.is_empty());
        // The icon half of WALL makes its current tool the brush again.
        let wall = MapEditor::category_rect(&layout, Category::Wall);
        click(&mut ed, &layout, Vector2::new(wall.x + 10.0, wall.y + 16.0));
        assert_eq!(ed.tool(), Tool::Wall(Material::Brick));
        assert_eq!(ed.open_menu(), None);
        // ERASE is a plain button; UNDO with nothing to undo is harmless.
        click(&mut ed, &layout, center(MapEditor::erase_rect(&layout)));
        assert_eq!(ed.tool(), Tool::Eraser);
        click(&mut ed, &layout, center(MapEditor::undo_rect(&layout)));
        assert!(ed.map().cells.is_empty());
    }

    #[test]
    fn a_press_outside_an_open_dropdown_closes_it_and_does_not_paint() {
        let layout = Layout::for_field(W, H);
        let mut ed = MapEditor::new(MapFile::new(), W, H);
        let wall = MapEditor::category_rect(&layout, Category::Wall);
        click(&mut ed, &layout, Vector2::new(wall.x + 80.0, wall.y + 16.0));
        assert_eq!(ed.open_menu(), Some("wall"));
        let p = map::cell_to_world(30, 15);
        let on_cell = Vector2::new(p.x + layout.field.x, p.y + layout.field.y);
        click(&mut ed, &layout, on_cell);
        assert_eq!(ed.open_menu(), None);
        assert!(ed.map().cells.is_empty(), "the dismissing press painted through the menu");
        // Esc closes too, and the next press on the same cell paints.
        click(&mut ed, &layout, Vector2::new(wall.x + 80.0, wall.y + 16.0));
        ed.update(&BuilderInput { escape: true, ..Default::default() }, &layout);
        assert_eq!(ed.open_menu(), None);
        click(&mut ed, &layout, on_cell);
        assert_eq!(ed.map().cell(30, 15), Some(&brick()));
    }

    #[test]
    fn the_map_panel_steps_values_and_never_paints_through() {
        let layout = Layout::for_field(W, H);
        let mut ed = MapEditor::new(MapFile::new(), W, H);
        click(&mut ed, &layout, center(MapEditor::map_rect(&layout)));
        assert_eq!(ed.open_menu(), Some("map"));
        let row = |r: SettingsRow| {
            let i = SETTINGS_ROWS.iter().position(|&x| x == r).unwrap();
            MapEditor::settings_row_rect(&layout, i)
        };
        let inc = |r: SettingsRow| center(MapEditor::settings_inc_rect(row(r)));
        let dec = |r: SettingsRow| center(MapEditor::settings_dec_rect(row(r)));
        // TANKS walks auto, 0, 1, .. and back to auto below 0.
        assert_eq!(ed.settings().tanks, None);
        click(&mut ed, &layout, inc(SettingsRow::Tanks));
        assert_eq!(ed.settings().tanks, Some(0));
        click(&mut ed, &layout, inc(SettingsRow::Tanks));
        assert_eq!(ed.settings().tanks, Some(1));
        click(&mut ed, &layout, dec(SettingsRow::Tanks));
        click(&mut ed, &layout, dec(SettingsRow::Tanks));
        assert_eq!(ed.settings().tanks, None);
        click(&mut ed, &layout, dec(SettingsRow::Tanks));
        assert_eq!(ed.settings().tanks, None, "auto is the floor");
        assert_eq!(ed.history().undo_depth(), 4, "one undo step per press that changed something");
        // TANK cycles auto and the chassis list; TIER START the tiers.
        click(&mut ed, &layout, inc(SettingsRow::Tank));
        assert_eq!(ed.settings().tank, Some(TankKind::ALL[0]));
        click(&mut ed, &layout, dec(SettingsRow::Tank));
        assert_eq!(ed.settings().tank, None);
        click(&mut ed, &layout, dec(SettingsRow::Tank));
        assert_eq!(ed.settings().tank, Some(TankKind::ALL[TankKind::ALL.len() - 1]), "wraps");
        click(&mut ed, &layout, inc(SettingsRow::Tank2));
        assert_eq!(ed.settings().tank2, Some(TankKind::ALL[0]));
        click(&mut ed, &layout, dec(SettingsRow::Tank2));
        assert_eq!(ed.settings().tank2, None);
        click(&mut ed, &layout, inc(SettingsRow::TierStart));
        assert_eq!(ed.settings().tier_start, Some(Tier::Light));
        click(&mut ed, &layout, inc(SettingsRow::Mission));
        assert_eq!(ed.settings().mission, Mission::Hunt);
        click(&mut ed, &layout, inc(SettingsRow::Spawn));
        assert_eq!(ed.settings().spawn, SpawnKind::Waves);
        // WAVES: auto, then 1..=20 with a hard ceiling.
        click(&mut ed, &layout, inc(SettingsRow::Waves));
        assert_eq!(ed.settings().waves, Some(1));
        let mut s = ed.settings();
        s.waves = Some(20);
        ed.apply_settings(s);
        click(&mut ed, &layout, inc(SettingsRow::Waves));
        assert_eq!(ed.settings().waves, Some(20));
        // Every press so far landed in the panel: it is still open and
        // nothing was painted under it.
        assert_eq!(ed.open_menu(), Some("map"));
        assert!(ed.map().cells.is_empty());
        assert!(ed.dirty());
        // RESET MAP reverts to the baseline as one undoable step.
        let reset = MapEditor::settings_reset_rect(row(SettingsRow::Reset));
        click(&mut ed, &layout, center(reset));
        assert!(!ed.dirty());
        assert_eq!(ed.open_menu(), Some("map"));
        // A press outside closes the panel and paints nothing.
        let p = map::cell_to_world(5, 15);
        click(&mut ed, &layout, Vector2::new(p.x + layout.field.x, p.y + layout.field.y));
        assert_eq!(ed.open_menu(), None);
        assert!(ed.map().cells.is_empty());
    }

    #[test]
    fn the_wheel_over_a_category_button_cycles_its_tool() {
        let layout = Layout::for_field(W, H);
        let mut ed = MapEditor::new(MapFile::new(), W, H);
        let wall = center(MapEditor::category_rect(&layout, Category::Wall));
        ed.update(&BuilderInput { pointer: Some(wall), wheel: -1.0, ..Default::default() }, &layout);
        assert_eq!(ed.tool(), Tool::Wall(Material::Iron));
        ed.update(&BuilderInput { pointer: Some(wall), wheel: 1.0, ..Default::default() }, &layout);
        assert_eq!(ed.tool(), Tool::Wall(Material::Brick));
        ed.update(&BuilderInput { pointer: Some(wall), wheel: 1.0, ..Default::default() }, &layout);
        assert_eq!(ed.tool(), Tool::Wall(Material::Glass), "wraps");
        // Off the buttons the wheel does nothing.
        let field = Vector2::new(400.0, 400.0);
        ed.update(&BuilderInput { pointer: Some(field), wheel: -1.0, ..Default::default() }, &layout);
        assert_eq!(ed.tool(), Tool::Wall(Material::Glass));
    }

    #[test]
    fn opening_one_popup_closes_the_other_and_save_reports_itself() {
        let layout = Layout::for_field(W, H);
        let mut ed = MapEditor::new(MapFile::new(), W, H);
        click(&mut ed, &layout, center(MapEditor::map_rect(&layout)));
        assert_eq!(ed.open_menu(), Some("map"));
        // A press on the ACTOR caret while the panel is open only closes
        // the panel; the next one opens the list.
        let actor = MapEditor::category_rect(&layout, Category::Actor);
        let caret = Vector2::new(actor.x + 80.0, actor.y + 16.0);
        click(&mut ed, &layout, caret);
        assert_eq!(ed.open_menu(), None);
        click(&mut ed, &layout, caret);
        assert_eq!(ed.open_menu(), Some("actor"));
        ed.popup = Some(Popup::Save { name: String::new() });
        assert_eq!(ed.open_menu(), Some("save"));
    }
}

#[cfg(test)]
mod file_tests {
    use super::*;
    use crate::{DEFAULT_SCREEN_HEIGHT, DEFAULT_SCREEN_WIDTH};

    const W: f32 = DEFAULT_SCREEN_WIDTH as f32;
    const H: f32 = DEFAULT_SCREEN_HEIGHT as f32;

    fn press(ed: &mut MapEditor, layout: &Layout, at: Vector2) {
        ed.update(&BuilderInput { pointer: Some(at), pressed: true, held: true, ..Default::default() }, layout);
        ed.update(&BuilderInput { pointer: Some(at), ..Default::default() }, layout);
    }

    fn center(r: Rectangle) -> Vector2 {
        Vector2::new(r.x + r.width / 2.0, r.y + r.height / 2.0)
    }

    /// FILE opens its menu; LOAD... opens the list; picking a row loads
    /// that map as the new baseline; a press outside closes the list
    /// without painting.
    #[test]
    fn file_menu_load_list_loads_a_shipped_map_by_name() {
        let layout = Layout::for_field(W, H);
        let mut ed = MapEditor::new(MapFile::new(), W, H);
        press(&mut ed, &layout, center(MapEditor::file_rect(&layout)));
        assert_eq!(ed.open_menu(), Some("file"));
        press(&mut ed, &layout, center(MapEditor::file_row_rect(&layout, 0)));
        assert_eq!(ed.open_menu(), Some("load"));
        let entries = map::available_maps();
        let row = entries.iter().position(|e| e.name == "default").expect("default is always listed");
        let panel = MapEditor::load_panel_rect(&layout, entries.len());
        if row < LOAD_VISIBLE_ROWS {
            press(&mut ed, &layout, center(MapEditor::load_row_rect(panel, row)));
            assert_eq!(ed.open_menu(), None);
            assert_eq!(ed.name(), "default");
            assert!(!ed.dirty(), "a load is the new baseline");
            assert!(!ed.map().cells.is_empty());
            assert_eq!(ed.history().undo_depth(), 1);
        } else {
            // Scroll down until the row is visible, then pick it.
            let wheel = BuilderInput { pointer: Some(center(panel)), wheel: -1.0, ..Default::default() };
            for _ in 0..(row + 1 - LOAD_VISIBLE_ROWS) {
                ed.update(&wheel, &layout);
            }
            press(&mut ed, &layout, center(MapEditor::load_row_rect(panel, LOAD_VISIBLE_ROWS - 1)));
            assert_eq!(ed.name(), "default");
        }
        // A press outside an open list closes it and paints nothing.
        press(&mut ed, &layout, center(MapEditor::file_rect(&layout)));
        press(&mut ed, &layout, center(MapEditor::file_row_rect(&layout, 0)));
        let before = ed.map().cells.len();
        let field_corner = Vector2::new(layout.field.x + 16.0, layout.field.y + layout.field.h - 16.0);
        press(&mut ed, &layout, field_corner);
        assert_eq!(ed.open_menu(), None);
        assert_eq!(ed.map().cells.len(), before);
    }

    /// `load_named` fails cleanly on an unknown name; `save` refuses a bad
    /// name and, where saving exists, round-trips a file and clears dirty.
    #[test]
    fn load_named_and_save_report_errors_and_round_trip() {
        let mut ed = MapEditor::new(MapFile::new(), W, H);
        assert!(ed.load_named("no-such-map", W, H).is_err());
        assert!(ed.save(Some("bad name/with slash")).is_err());
        assert!(ed.save(None).is_err(), "an unnamed map needs SAVE AS");
        if !map::saving_available() {
            return;
        }
        ed.select_tool(Tool::Wall(Material::Glass));
        ed.stroke(&[(7, 7)], false, W, H);
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
