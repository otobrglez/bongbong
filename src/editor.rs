//! Dev-only battlefield map editor (see docs/map-editor-design.md): click a
//! grid cell to place/erase a wall/road/frog/start/enemy-frog/gate/pickup
//! object, then Save the result under `maps/` as a `map::MapFile`. The
//! map's `mission`/`spawn` level tables have no UI; they ride along
//! untouched because Save writes the loaded `MapFile` itself. Presentation-layer only, same
//! category as `game.rs` - never touches physics/AI/hecs, and drives its
//! own render loop from `main.rs` in place of `simulation::Game`'s whenever
//! the editor is active. Gated entirely behind the `map-editor` Cargo
//! feature so none of this compiles into a release build.

use rand::RngExt;
use sola_raylib::prelude::*;

use crate::ground::{self, GroundGrid};
use crate::map::{self, CellObject, MapFile};
use crate::obstacle::{self, Material};
use crate::pickup::PickupKind;
use crate::{
    EDITOR_HAMBURGER_SIZE,
    EDITOR_ICON_GAP,
    EDITOR_ICON_SIZE,
    EDITOR_PALETTE_BOTTOM_MARGIN,
    EDITOR_PANEL_BORDER_OPACITY,
    EDITOR_PANEL_BORDER_THICKNESS,
    EDITOR_PANEL_FILL,
    EDITOR_PANEL_FILL_OPACITY,
    EDITOR_PANEL_PADDING,
    EDITOR_PANEL_ROUNDNESS,
    EDITOR_PANEL_SEGMENTS,
    EDITOR_PANEL_SHADOW_OFFSET,
    EDITOR_PANEL_SHADOW_OPACITY,
    EDITOR_TOOLBAR_MARGIN,
    OBSTACLE_GRID_SIZE,
    PATHFIND_CELL_SIZE,
    Position,
};

/// The sprite atlases the editor needs to draw placed objects and their
/// palette icons - a small subset of `game::Textures`, plus the one texture
/// only the editor uses (`eraser`, see docs/map-editor-design.md's "Eraser
/// asset" section).
pub struct EditorTextures<'a> {
    pub obstacles: &'a Texture2D,
    pub ground: &'a Texture2D,
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
}

/// One palette tool - what a grid click currently does. Order here is the
/// order icons are drawn in the bottom-center palette.
#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Wall(Material),
    Road,
    Frog,
    Start,
    /// The Hunt mission's enemy frog - singleton, moved on placement like
    /// `Frog`.
    EnemyFrog,
    /// A wave roll-in gate - any number, meant for nav-grid edge cells (the
    /// linter's `gate-not-on-edge` catches one placed elsewhere).
    Gate,
    Pickup(PickupKind),
    Eraser,
}

const TOOLS: [Tool; 17] = [
    Tool::Wall(Material::Brick),
    Tool::Wall(Material::Iron),
    Tool::Wall(Material::Wood),
    Tool::Wall(Material::Glass),
    Tool::Road,
    Tool::Frog,
    Tool::Start,
    Tool::EnemyFrog,
    Tool::Gate,
    Tool::Pickup(PickupKind::Health),
    Tool::Pickup(PickupKind::Ammo),
    Tool::Pickup(PickupKind::Laser),
    Tool::Pickup(PickupKind::Minigun),
    Tool::Pickup(PickupKind::Plasma),
    Tool::Pickup(PickupKind::SpeedUp),
    Tool::Pickup(PickupKind::Shield),
    Tool::Eraser,
];

/// A small overlay popup the editor can have open at a time - Save's
/// filename prompt or Load's file list. At most one at a time; opening
/// either closes the other.
enum Popup {
    Save { name: String },
    Load { names: Vec<String> },
}

/// What the caller (`main.rs`) should do after this frame's `update` -
/// everything else the editor handles internally.
pub enum EditorAction {
    None,
    /// The user hit Close (or the top-left back icon). What that means is
    /// the caller's call: exit the process if the editor was launched via
    /// `--editor`, or switch the driver back to `Game` if it was entered
    /// mid-game via the hamburger button - see docs/map-editor-design.md's
    /// "Entering the editor" section.
    Close,
}

pub struct MapEditor {
    pub map: MapFile,
    active_tool: Tool,
    ground: GroundGrid,
    /// Fixed for this editor session (rolled once in `new`) and reused by
    /// every `rebuild_ground` call - see `ground::build`'s `seed` param.
    /// Keeping it fixed (rather than a fresh `rand::rng()` per rebuild,
    /// which is what this used to do) is what makes grass tiles stop
    /// visibly re-randomizing on every edit - see `rebuild_ground`.
    ground_seed: u64,
    popup: Option<Popup>,
    /// File stem (no directory/extension) of the map currently loaded, if
    /// any - prefills Save's filename prompt so re-saving the map you just
    /// loaded doesn't require retyping its name.
    current_name: Option<String>,
    /// Last click was consumed by the popup this frame, so the same click
    /// doesn't also fall through to the palette/grid below it.
    status: Option<String>,
    /// Grid cell painted/erased by the current mouse-down drag, if any -
    /// lets `update` place continuously as the drag crosses into a new
    /// cell (not just on the initial click) while still only placing once
    /// per cell rather than every single frame the button stays down over
    /// it. Cleared on release, so the next press always paints its first
    /// cell even if it's the same one a previous drag ended on.
    drag_cell: Option<(i32, i32)>,
}

impl MapEditor {
    /// `initial` seeds the canvas - `Some` when entering with a map already
    /// loaded (`--editor maps/foo.toml`, or the in-game hamburger button
    /// when a map is currently active), `None` for a blank canvas.
    pub fn new(initial: Option<MapFile>, width: f32, height: f32) -> Self {
        let map = initial.unwrap_or_else(MapFile::new);
        let mut editor = MapEditor {
            map,
            active_tool: TOOLS[0],
            ground: GroundGrid::default(),
            ground_seed: rand::rng().random(),
            popup: None,
            current_name: None,
            status: None,
            drag_cell: None,
        };
        editor.rebuild_ground(width, height);
        editor
    }

    /// Recompute the decorative ground layer from the map's current wall +
    /// road cells - every wall cell paints road under itself automatically,
    /// same as a live round (`Game::init`'s "road_cells = obstacle_positions
    /// + explicit road cells" convention), plus every cell explicitly
    /// placed with the Road tool. Called after every cell edit (not just
    /// once per round, like the live game) - `ground_seed` is what keeps
    /// this from re-randomizing grass tiles unrelated to the edit each time
    /// this runs; see that field's doc comment and `ground::build`'s.
    fn rebuild_ground(&mut self, width: f32, height: f32) {
        let road_cells: Vec<Position> = self
            .map
            .iter_cells()
            .filter(|(_, _, obj)| matches!(obj, CellObject::Wall { .. } | CellObject::Road))
            .map(|(col, row, _)| map::cell_to_world(col, row))
            .collect();
        self.ground = ground::build(width, height, self.ground_seed, &road_cells);
    }

    fn hamburger_rect() -> Rectangle {
        Rectangle::new(EDITOR_TOOLBAR_MARGIN, EDITOR_TOOLBAR_MARGIN, EDITOR_HAMBURGER_SIZE, EDITOR_HAMBURGER_SIZE)
    }

    const TOOLBAR_BTN_W: f32 = 72.0;
    const TOOLBAR_BTN_H: f32 = 40.0;
    const TOOLBAR_LABELS: [&'static str; 4] = ["New", "Save", "Load", "Close"];

    fn toolbar_panel_rect(width: f32) -> Rectangle {
        let n = Self::TOOLBAR_LABELS.len() as f32;
        let w = EDITOR_PANEL_PADDING * 2.0 + n * Self::TOOLBAR_BTN_W + (n - 1.0) * EDITOR_ICON_GAP;
        let h = EDITOR_PANEL_PADDING * 2.0 + Self::TOOLBAR_BTN_H;
        Rectangle::new(width - EDITOR_TOOLBAR_MARGIN - w, EDITOR_TOOLBAR_MARGIN, w, h)
    }

    fn toolbar_button_rect(width: f32, index: usize) -> Rectangle {
        let panel = Self::toolbar_panel_rect(width);
        Rectangle::new(
            panel.x + EDITOR_PANEL_PADDING + index as f32 * (Self::TOOLBAR_BTN_W + EDITOR_ICON_GAP),
            panel.y + EDITOR_PANEL_PADDING,
            Self::TOOLBAR_BTN_W,
            Self::TOOLBAR_BTN_H,
        )
    }

    fn palette_panel_rect(width: f32, height: f32) -> Rectangle {
        let n = TOOLS.len() as f32;
        let w = EDITOR_PANEL_PADDING * 2.0 + n * EDITOR_ICON_SIZE + (n - 1.0) * EDITOR_ICON_GAP;
        let h = EDITOR_PANEL_PADDING * 2.0 + EDITOR_ICON_SIZE;
        Rectangle::new((width - w) / 2.0, height - EDITOR_PALETTE_BOTTOM_MARGIN - h, w, h)
    }

    fn palette_icon_rect(width: f32, height: f32, index: usize) -> Rectangle {
        let panel = Self::palette_panel_rect(width, height);
        Rectangle::new(
            panel.x + EDITOR_PANEL_PADDING + index as f32 * (EDITOR_ICON_SIZE + EDITOR_ICON_GAP),
            panel.y + EDITOR_PANEL_PADDING,
            EDITOR_ICON_SIZE,
            EDITOR_ICON_SIZE,
        )
    }

    /// Whether `point` lands on any of the editor's own UI chrome (the
    /// hamburger icon, the Save/Load/Close toolbar, or the object palette) -
    /// used to keep a click on those from also placing/erasing whatever
    /// battlefield cell happens to be behind them.
    fn point_on_ui(point: Vector2, width: f32, height: f32) -> bool {
        Self::hamburger_rect().check_collision_point_rec(point)
            || Self::toolbar_panel_rect(width).check_collision_point_rec(point)
            || Self::palette_panel_rect(width, height).check_collision_point_rec(point)
    }

    /// Advance one frame: handle whatever popup is open, or (with none
    /// open) a click on the hamburger/toolbar/palette, or a grid
    /// placement/erase. Returns `EditorAction::Close` the frame Close (or
    /// the back icon) is clicked - the caller decides what that means.
    pub fn update(&mut self, rl: &mut RaylibHandle, width: f32, height: f32) -> EditorAction {
        if let Some(action) = self.update_popup(rl, width, height) {
            return action;
        }

        let mouse = rl.get_mouse_position();
        let held = rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT);
        if !held {
            // Cleared on release (not just left alone) so the *next* press
            // always paints its first cell even if it's the same one a
            // previous drag happened to end on - `place`'s own dedup below
            // only looks at `drag_cell` from the *current* unbroken drag.
            self.drag_cell = None;
            return EditorAction::None;
        }

        // UI (hamburger/toolbar/palette) only reacts to the initial click of
        // a press, never every frame a drag happens to stay over it -
        // otherwise holding the mouse down on, say, Close would fire it
        // every single frame instead of once.
        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
            if Self::hamburger_rect().check_collision_point_rec(mouse) {
                return EditorAction::Close;
            }

            for (i, &label) in Self::TOOLBAR_LABELS.iter().enumerate() {
                if Self::toolbar_button_rect(width, i).check_collision_point_rec(mouse) {
                    match label {
                        "New" => {
                            self.map = MapFile::new();
                            self.current_name = None;
                            self.rebuild_ground(width, height);
                        }
                        "Save" => {
                            self.popup = Some(Popup::Save {
                                name: self.current_name.clone().unwrap_or_default(),
                            });
                        }
                        "Load" => self.popup = Some(Popup::Load { names: map::list_maps() }),
                        "Close" => return EditorAction::Close,
                        _ => unreachable!(),
                    }
                    return EditorAction::None;
                }
            }

            for (i, &tool) in TOOLS.iter().enumerate() {
                if Self::palette_icon_rect(width, height, i).check_collision_point_rec(mouse) {
                    self.active_tool = tool;
                    return EditorAction::None;
                }
            }
        }

        // Grid placement/erase: reacts to the mouse being *held*, not just
        // the initial press, so painting continues as a click-drag crosses
        // into new cells (the actual bug this block fixes - a plain
        // is_mouse_button_pressed check only ever placed the one cell under
        // the very first click of a drag). `drag_cell` still dedups so a
        // held-but-not-moved frame doesn't re-place/rebuild the same cell
        // every single frame.
        //
        // The per-icon rect checks above only reject a click that actually
        // lands on a button; a click in a panel's own padding/gaps (or the
        // palette background near the bottom of the screen, which can
        // overlap real battlefield cells) would otherwise fall through to
        // placing/erasing underneath it - `point_on_ui` catches that.
        if !Self::point_on_ui(mouse, width, height)
            && mouse.x >= 0.0
            && mouse.x <= width
            && mouse.y >= 0.0
            && mouse.y <= height
        {
            let cell = map::world_to_cell(mouse);
            if self.drag_cell != Some(cell) {
                self.drag_cell = Some(cell);
                self.place(cell.0, cell.1, width, height);
            }
        }

        EditorAction::None
    }

    /// Handle input while a popup is open, consuming it entirely (no
    /// fall-through to the palette/grid this frame) - returns `Some` only
    /// once the popup itself has nothing left to do this frame (which is
    /// every frame it's open), keeping the same `EditorAction` shape as
    /// `update`'s main branch.
    fn update_popup(&mut self, rl: &mut RaylibHandle, width: f32, height: f32) -> Option<EditorAction> {
        // Take the popup out of `self` entirely (rather than matching on
        // `&mut self.popup` in place) so the arms below can freely touch
        // other `self` fields (`self.map`, `self.rebuild_ground(...)`)
        // without fighting the borrow checker over a live borrow into
        // `self.popup` - `popup` here is a fully owned local, disconnected
        // from `self`, put back at the end only if it should stay open.
        let mut popup = self.popup.take()?;
        if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
            return Some(EditorAction::None); // already taken, so stays closed
        }
        let mut keep_open = true;
        match &mut popup {
            Popup::Save { name } => {
                while let Some(c) = rl.get_char_pressed() {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        name.push(c);
                    }
                }
                if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE) {
                    name.pop();
                }
                if rl.is_key_pressed(KeyboardKey::KEY_ENTER) && !name.is_empty() {
                    let path = map::maps_dir().join(format!("{name}.toml"));
                    self.status = Some(match self.map.save(&path) {
                        Ok(()) => format!("saved {name}.toml"),
                        Err(e) => e,
                    });
                    self.current_name = Some(name.clone());
                    keep_open = false;
                }
            }
            Popup::Load { names } => {
                if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                    let mouse = rl.get_mouse_position();
                    let panel = Self::load_panel_rect(width, height, names.len());
                    let clicked = names.iter().enumerate().find_map(|(i, name)| {
                        Self::load_row_rect(&panel, i)
                            .check_collision_point_rec(mouse)
                            .then(|| name.clone())
                    });
                    if let Some(name) = clicked {
                        let path = map::maps_dir().join(format!("{name}.toml"));
                        match MapFile::load(&path) {
                            Ok(map) => {
                                self.map = map;
                                self.current_name = Some(name.clone());
                                self.status = Some(format!("loaded {name}.toml"));
                                self.rebuild_ground(width, height);
                            }
                            Err(e) => self.status = Some(e),
                        }
                        keep_open = false;
                    } else if !panel.check_collision_point_rec(mouse) {
                        keep_open = false;
                    }
                }
            }
        }
        if keep_open {
            self.popup = Some(popup);
        }
        Some(EditorAction::None)
    }

    fn load_panel_rect(width: f32, height: f32, row_count: usize) -> Rectangle {
        let w = 260.0;
        let h = EDITOR_PANEL_PADDING * 2.0 + (row_count.max(1) as f32) * 28.0;
        Rectangle::new((width - w) / 2.0, (height - h) / 2.0, w, h)
    }

    fn load_row_rect(panel: &Rectangle, index: usize) -> Rectangle {
        Rectangle::new(
            panel.x + EDITOR_PANEL_PADDING,
            panel.y + EDITOR_PANEL_PADDING + index as f32 * 28.0,
            panel.width - EDITOR_PANEL_PADDING * 2.0,
            26.0,
        )
    }

    /// Place (or move/erase) whatever `active_tool` does at grid cell
    /// `(col, row)`. Most tools just overwrite the cell - placing a wall
    /// where a pickup was, say, simply replaces it, same "one object per
    /// cell" model the map format itself has. The singletons (Frog, Start,
    /// EnemyFrog) move their one existing placement instead
    /// (docs/map-editor-design.md's "Frog: singleton enforcement"); Eraser
    /// clears the cell outright.
    fn place(&mut self, col: i32, row: i32, width: f32, height: f32) {
        match self.active_tool {
            Tool::Wall(material) => self.map.set_cell(col, row, CellObject::Wall { material }),
            Tool::Road => self.map.set_cell(col, row, CellObject::Road),
            Tool::Frog => {
                if let Some((oc, or)) = self.map.frog_cell() {
                    self.map.clear_cell(oc, or);
                }
                self.map.set_cell(col, row, CellObject::Frog);
            }
            Tool::Start => {
                if let Some((oc, or)) = self.map.start_cell() {
                    self.map.clear_cell(oc, or);
                }
                self.map.set_cell(col, row, CellObject::Start);
            }
            Tool::EnemyFrog => {
                if let Some((oc, or)) = self.map.enemy_frog_cell() {
                    self.map.clear_cell(oc, or);
                }
                self.map.set_cell(col, row, CellObject::EnemyFrog);
            }
            Tool::Gate => self.map.set_cell(col, row, CellObject::Gate),
            Tool::Pickup(pickup) => self.map.set_cell(col, row, CellObject::Pickup { pickup }),
            Tool::Eraser => self.map.clear_cell(col, row),
        }
        self.rebuild_ground(width, height);
    }

    /// Draw the whole editor: ground, placed objects, hover highlight, then
    /// UI chrome on top (hamburger, toolbar, palette, any open popup).
    pub fn render(
        &self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        width: f32,
        height: f32,
        textures: &EditorTextures,
    ) {
        let mouse = rl.get_mouse_position();
        let (render_width, render_height) = (rl.get_render_width(), rl.get_render_height());
        let mut d = rl.begin_drawing(thread);
        d.clear_background(Color::new(30, 30, 34, 255));

        ground::draw(&mut d, textures.ground, &self.ground);

        for (col, row, obj) in self.map.iter_cells() {
            let pos = map::cell_to_world(col, row);
            let size = OBSTACLE_GRID_SIZE;
            let dest = Rectangle::new(pos.x, pos.y, size, size);
            let origin = Vector2::new(size / 2.0, size / 2.0);
            match *obj {
                CellObject::Wall { material } => {
                    let src = obstacle::icon_source_rec(material);
                    d.draw_texture_pro(textures.obstacles, src, dest, origin, 0.0, Color::WHITE);
                }
                CellObject::Road => {} // already painted into `self.ground`
                CellObject::Frog => {
                    let src = Rectangle::new(0.0, 0.0, crate::FROG_TEXTURE_SIZE, crate::FROG_TEXTURE_SIZE);
                    d.draw_texture_pro(textures.frog_idle, src, dest, origin, 0.0, Color::WHITE);
                }
                CellObject::Start => {
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
                    let texture = match pickup {
                        PickupKind::Health => textures.pickup_health,
                        PickupKind::Ammo => textures.pickup_ammo,
                        PickupKind::Laser => textures.pickup_laser,
                        PickupKind::Minigun => textures.pickup_minigun,
                        PickupKind::Plasma => textures.pickup_plasma,
                        PickupKind::SpeedUp => textures.pickup_speedup,
                        PickupKind::Shield => textures.pickup_shield,
                    };
                    let src =
                        Rectangle::new(0.0, 0.0, crate::PICKUP_TEXTURE_SIZE, crate::PICKUP_TEXTURE_SIZE);
                    d.draw_texture_pro(texture, src, dest, origin, 0.0, Color::WHITE);
                }
            }
        }

        // Hover highlight - no visible grid lines otherwise, per
        // docs/map-editor-design.md.
        if mouse.x >= 0.0 && mouse.x <= width && mouse.y >= 0.0 && mouse.y <= height {
            let (col, row) = map::world_to_cell(mouse);
            let pos = map::cell_to_world(col, row);
            let size = OBSTACLE_GRID_SIZE;
            d.draw_rectangle_lines_ex(
                Rectangle::new(pos.x - size / 2.0, pos.y - size / 2.0, size, size),
                2.0,
                Color::new(255, 255, 255, 160),
            );
        }

        draw_panel(&mut d, Self::hamburger_rect());
        d.draw_text("=", (Self::hamburger_rect().x + 14.0) as i32, (Self::hamburger_rect().y + 8.0) as i32, 22, Color::WHITE);

        let toolbar_panel = Self::toolbar_panel_rect(width);
        draw_panel(&mut d, toolbar_panel);
        for (i, &label) in Self::TOOLBAR_LABELS.iter().enumerate() {
            let rect = Self::toolbar_button_rect(width, i);
            d.draw_rectangle_rounded_lines_ex(rect, 0.2, EDITOR_PANEL_SEGMENTS, 1.0, Color::new(255, 255, 255, 60));
            d.draw_text(label, (rect.x + 10.0) as i32, (rect.y + 11.0) as i32, 16, Color::WHITE);
        }

        let palette_panel = Self::palette_panel_rect(width, height);
        draw_panel(&mut d, palette_panel);
        for (i, &tool) in TOOLS.iter().enumerate() {
            let rect = Self::palette_icon_rect(width, height, i);
            if tool == self.active_tool {
                d.draw_rectangle_rounded(rect, 0.2, EDITOR_PANEL_SEGMENTS, Color::new(255, 255, 255, 40));
            }
            draw_tool_icon(&mut d, textures, tool, rect);
            if tool == Tool::Frog && self.map.frog_cell().is_some() {
                d.draw_circle(
                    (rect.x + rect.width - 6.0) as i32,
                    (rect.y + 6.0) as i32,
                    5.0,
                    Color::LIME,
                );
            }
            if tool == Tool::Start && self.map.start_cell().is_some() {
                d.draw_circle(
                    (rect.x + rect.width - 6.0) as i32,
                    (rect.y + 6.0) as i32,
                    5.0,
                    Color::LIME,
                );
            }
            if tool == Tool::EnemyFrog && self.map.enemy_frog_cell().is_some() {
                d.draw_circle(
                    (rect.x + rect.width - 6.0) as i32,
                    (rect.y + 6.0) as i32,
                    5.0,
                    Color::LIME,
                );
            }
        }

        if let Some(popup) = &self.popup {
            match popup {
                Popup::Save { name } => {
                    let panel = Rectangle::new(width / 2.0 - 150.0, height / 2.0 - 40.0, 300.0, 80.0);
                    draw_panel(&mut d, panel);
                    d.draw_text("Save as:", (panel.x + 12.0) as i32, (panel.y + 10.0) as i32, 16, Color::WHITE);
                    d.draw_text(
                        &format!("{name}_"),
                        (panel.x + 12.0) as i32,
                        (panel.y + 34.0) as i32,
                        18,
                        Color::WHITE,
                    );
                    d.draw_text(
                        "Enter to save, Esc to cancel",
                        (panel.x + 12.0) as i32,
                        (panel.y + 58.0) as i32,
                        12,
                        Color::GRAY,
                    );
                }
                Popup::Load { names } => {
                    let panel = Self::load_panel_rect(width, height, names.len());
                    draw_panel(&mut d, panel);
                    if names.is_empty() {
                        d.draw_text(
                            "no saved maps",
                            (panel.x + EDITOR_PANEL_PADDING) as i32,
                            (panel.y + EDITOR_PANEL_PADDING) as i32,
                            14,
                            Color::GRAY,
                        );
                    }
                    for (i, name) in names.iter().enumerate() {
                        let row = Self::load_row_rect(&panel, i);
                        d.draw_text(name, (row.x + 4.0) as i32, (row.y + 4.0) as i32, 16, Color::WHITE);
                    }
                }
            }
        }

        if let Some(status) = &self.status {
            d.draw_text(status, EDITOR_TOOLBAR_MARGIN as i32, (height - 22.0) as i32, 14, Color::LIGHTGRAY);
        }

        // Temporary diagnostic for the "clicks on tools don't register"
        // report - shows exactly what raylib thinks the mouse position and
        // screen/render sizes are, so a HiDPI screen-vs-render mismatch (the
        // classic cause of "visually-correct but click-position-wrong" UI)
        // is a glance away instead of a guess. Remove once that's resolved.
        let debug = format!(
            "mouse=({:.0},{:.0}) screen={}x{} render={}x{} on_hamburger={} on_toolbar={} on_palette={}",
            mouse.x,
            mouse.y,
            width as i32,
            height as i32,
            render_width,
            render_height,
            Self::hamburger_rect().check_collision_point_rec(mouse),
            Self::toolbar_panel_rect(width).check_collision_point_rec(mouse),
            Self::palette_panel_rect(width, height).check_collision_point_rec(mouse),
        );
        d.draw_text(&debug, EDITOR_TOOLBAR_MARGIN as i32, (height - 44.0) as i32, 14, Color::YELLOW);
    }
}

/// Draw a rounded, bordered, drop-shadowed panel background - shared by
/// every panel the editor draws (palette, toolbar, popups), per
/// docs/map-editor-design.md's "Panel chrome" section.
fn draw_panel(d: &mut impl RaylibDraw, rect: Rectangle) {
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

fn draw_tool_icon(d: &mut impl RaylibDraw, textures: &EditorTextures, tool: Tool, rect: Rectangle) {
    let dest = Rectangle::new(rect.x + 4.0, rect.y + 4.0, rect.width - 8.0, rect.height - 8.0);
    match tool {
        Tool::Wall(material) => {
            let src = obstacle::icon_source_rec(material);
            d.draw_texture_pro(textures.obstacles, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
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
        Tool::Pickup(pickup) => {
            let texture = match pickup {
                PickupKind::Health => textures.pickup_health,
                PickupKind::Ammo => textures.pickup_ammo,
                PickupKind::Laser => textures.pickup_laser,
                PickupKind::Minigun => textures.pickup_minigun,
                PickupKind::Plasma => textures.pickup_plasma,
                PickupKind::SpeedUp => textures.pickup_speedup,
                PickupKind::Shield => textures.pickup_shield,
            };
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

/// The enemy side's marker colour - the red ground ring the game draws
/// under the enemy frog, so an `enemy_frog` cell reads the same in the
/// editor as in a round.
const ENEMY_RING_COLOR: Color = Color::new(230, 60, 60, 220);

const GATE_COLOR: Color = Color::ORANGE;

/// A flat red ring of radius `radius` centred on `center`, drawn under
/// whatever sits on the cell.
fn draw_enemy_ring(d: &mut impl RaylibDraw, center: Position, radius: f32) {
    d.draw_ring(center, radius * 0.75, radius, 0.0, 360.0, 24, ENEMY_RING_COLOR);
    d.draw_circle_v(center, radius * 0.75, Color::new(230, 60, 60, 50));
}

/// An orange chevron of overall size `size` at `center`, its point aimed
/// along `inward` (a unit axis vector) - the direction a tank rolling in
/// through the gate travels.
fn draw_gate_chevron(d: &mut impl RaylibDraw, center: Position, size: f32, inward: Position) {
    let half = size / 2.0 - 3.0;
    // Perpendicular to `inward`, for the chevron's two wings.
    let side = Position::new(-inward.y, inward.x);
    let tip = center + inward * half;
    let tail = center - inward * (half * 0.4);
    let wing_a = tail + side * half;
    let wing_b = tail - side * half;
    // Two wings plus a stem so the arrow still reads at 32px.
    d.draw_line_ex(wing_a, tip, 3.0, GATE_COLOR);
    d.draw_line_ex(wing_b, tip, 3.0, GATE_COLOR);
    d.draw_line_ex(center - inward * half, tip, 3.0, GATE_COLOR);
}

/// The inward direction of a gate placed at world position `pos`, or
/// `None` when that position is not on a nav-grid edge cell (col 0, the
/// last col, row 0 or the last row of the `PATHFIND_CELL_SIZE` grid a
/// `width` x `height` battlefield gets - the same cell arithmetic as
/// `pathfind::Grid::build`). A corner reports its horizontal edge.
fn gate_inward(pos: Position, width: f32, height: f32) -> Option<Position> {
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

#[cfg(test)]
mod editor_tests {
    use super::*;
    use crate::level::{Mission, SpawnKind, Tier};
    use crate::{DEFAULT_SCREEN_HEIGHT, DEFAULT_SCREEN_WIDTH};

    /// Save writes the loaded `MapFile` as-is, so a map's level tables and
    /// the two level cell kinds survive an editor session untouched.
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
        map.set_cell(30, 11, CellObject::Start);
        map.set_cell(35, 11, CellObject::Frog);
        map.set_cell(5, 11, CellObject::EnemyFrog);
        map.set_cell(0, 11, CellObject::Gate);
        map.set_cell(39, 11, CellObject::Gate);
        map.set_cell(20, 11, CellObject::Wall { material: Material::Brick });

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
        assert_eq!(back.frog_cell(), Some((35, 11)));
        assert_eq!(back.cells.len(), map.cells.len());
    }

    #[test]
    fn gate_chevrons_point_inward_only_on_edge_cells() {
        let (w, h) = (DEFAULT_SCREEN_WIDTH as f32, DEFAULT_SCREEN_HEIGHT as f32);
        let at = |col: i32, row: i32| map::cell_to_world(col, row);
        assert_eq!(gate_inward(at(0, 11), w, h), Some(Position::new(1.0, 0.0)));
        assert_eq!(gate_inward(at(39, 11), w, h), Some(Position::new(-1.0, 0.0)));
        assert_eq!(gate_inward(at(20, 0), w, h), Some(Position::new(0.0, 1.0)));
        assert_eq!(gate_inward(at(20, 22), w, h), Some(Position::new(0.0, -1.0)));
        assert_eq!(gate_inward(at(20, 11), w, h), None);
    }

    /// Seventeen icons must still fit inside the default battlefield width.
    #[test]
    fn palette_fits_the_default_battlefield_width() {
        let panel = MapEditor::palette_panel_rect(DEFAULT_SCREEN_WIDTH as f32, DEFAULT_SCREEN_HEIGHT as f32);
        assert!(panel.x >= 0.0 && panel.x + panel.width <= DEFAULT_SCREEN_WIDTH as f32);
        assert_eq!(TOOLS.len(), 17);
    }
}
