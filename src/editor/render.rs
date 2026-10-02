//! The builder's chrome with raylib: `MapEditor::render` (the field
//! through a `Camera2D`, then the bar and the open popup in window space),
//! the bar and popup painters, the tool icons and the field markers, and
//! `EditorTextures`, the sheets the builder draws from. A child of
//! `editor` because it draws the builder's private state; the edit model
//! and every hit rect stay in `mod.rs` and run headless.

use sola_raylib::prelude::*;

use super::*;
use super::camera::{scene_plan, window_mapping};
use crate::text::{keys, text};
use crate::canvas::Sheet;
use crate::frog::FrogAnim;
use crate::hud::{BAR_FILL, DIM, HUD_LABEL_SIZE, HUD_TEXT_SIZE, TEXT};
use crate::math::{Color, Rectangle};
use crate::obstacle;
use crate::portal::{draw_portal, portal_icon_source_rec};
use crate::render::canvas::{GpuCanvas, Sheets};

// The bar's fixed x offsets only the drawing reads (the hit rects' own
// slots stay in mod.rs); `bar_tests` pins that nothing overlaps.
const SLOT_BUILD: f32 = 8.0;
const SLOT_NAME: f32 = 72.0;
const NAME_W: f32 = 152.0;
/// The gap a button's drawn box keeps from the slot after it, so two
/// adjacent outlines never touch.
const BUTTON_GAP: f32 = 8.0;
/// The category button's text column, right of its full-bleed icon.
/// The category caret's left edge: flush with the button's drawn box,
/// two blocks in from the active outline, so the 10 px name (`GROUND`
/// is the widest) ends clear of it.
const CATEGORY_CARET_X: i32 = (CATEGORY_W - BUTTON_GAP) as i32 - CARET_W;
/// A caret is 10 px wide (`draw_caret`).
const CARET_W: i32 = 10;
/// A dropdown row's name column, right of its 32 px icon at 8 px inset.
const DROPDOWN_TEXT_X: i32 = 48;
const SETTINGS_VALUE_X: i32 = 180;
const SETTINGS_LABEL_SIZE: i32 = 16;

/// What the canvas area shows where the map is not: past its edges when
/// the whole of a field map is shown, and under the field before the
/// ground is drawn.
const CANVAS_FILL: Color = Color::new(30, 30, 34, 255);

/// Where a TANK row shows the chassis it has picked: a tank frame's 32 px
/// (`TANK_FRAME_SIZE`, one sheet pixel each), centred on the row and
/// right-aligned in the label column, one inset clear of the `<` button.
/// The TANK labels' budget in `text::budgets` stops short of it.
fn settings_icon_rect(row: Rectangle) -> Rectangle {
    let size = crate::TANK_FRAME_SIZE;
    Rectangle::new(row.x + SETTINGS_DEC_X - SETTINGS_INSET - size, row.y + (row.height - size) / 2.0, size, size)
}

/// The width the default font gives `text` at `size`: `text::width`,
/// which is `MeasureText`'s answer with no handle.
fn text_width(text: &str, size: i32) -> f32 {
    crate::text::width(text, size) as f32
}

/// `text` cut down to what fits in `width` at `size`, the cut marked `~`.
fn fit_text(text: &str, width: f32, size: i32) -> String {
    crate::text::fit(text, width as i32, size).into_owned()
}

/// A tool's name as the dropdown rows spell it, in the language on
/// screen: the catalogue's `tool-<name>` message.
fn label(tool: Tool) -> String {
    text().named("tool", tool.name())
}

/// A tool's name at the width the bar's 10 px line and the cursor
/// readout have room for: the catalogue's `tool-short-<name>` where the
/// language has one, else its full name.
fn short_label(tool: Tool) -> String {
    let t = text();
    t.message(&format!("tool-short-{}", tool.name()), &[]).unwrap_or_else(|| t.named("tool", tool.name()))
}

/// What the cursor readout calls the object in a cell.
fn cell_label(obj: &CellObject) -> String {
    TOOLS.iter().copied().find(|t| t.object().as_ref() == Some(obj)).map_or_else(|| "?".to_string(), short_label)
}

use crate::{
    EDITOR_PANEL_BORDER_OPACITY, EDITOR_PANEL_BORDER_THICKNESS, EDITOR_PANEL_FILL, EDITOR_PANEL_FILL_OPACITY, EDITOR_PANEL_ROUNDNESS,
    EDITOR_PANEL_SEGMENTS, EDITOR_PANEL_SHADOW_OFFSET, EDITOR_PANEL_SHADOW_OPACITY, EDITOR_TOOLBAR_MARGIN, OBSTACLE_GRID_SIZE,
};

/// The sprite atlases the builder needs to draw placed objects and their
/// bar/dropdown icons - a small subset of `game::Textures`, plus the one
/// texture only the builder uses (`eraser`, see docs/map-editor-design.md).
pub struct EditorTextures<'a> {
    pub obstacles: &'a Texture2D,
    /// The props sheet (`obstacle::Sheet::Props`).
    pub props: &'a Texture2D,
    /// The ground tileset of the canvas map's theme
    /// (`Theme::ground_texture_path`) - `app.rs` picks it per frame.
    pub ground: &'a Texture2D,
    /// The tall-grass sheet of the canvas map's theme
    /// (`Theme::grass_texture_path`).
    pub grass: &'a Texture2D,
    pub frog_idle: &'a Texture2D,
    pub pickup_health: &'a Texture2D,
    pub pickup_ammo: &'a Texture2D,
    pub pickup_laser: &'a Texture2D,
    pub pickup_minigun: &'a Texture2D,
    pub pickup_plasma: &'a Texture2D,
    pub pickup_missiles: &'a Texture2D,
    pub pickup_speedup: &'a Texture2D,
    pub pickup_shield: &'a Texture2D,
    pub pickup_flamethrower: &'a Texture2D,
    pub pickup_frog_health: &'a Texture2D,
    pub eraser: &'a Texture2D,
    /// static/portal_sheet.png - the spiral and its bar icon (portal.rs).
    pub portal: &'a Texture2D,
    pub tanks: &'a Texture2D,
    pub trees: &'a Texture2D,
    /// static/towers_sheet.png - the defence towers' bases and tops.
    pub towers: &'a Texture2D,
    pub pickup_tower_pack: &'a Texture2D,
    /// The canvas's floor shade as `app.rs` uploaded it before the frame,
    /// with the stamp it was baked under.
    pub shade: Option<(u64, &'a Texture2D)>,
    /// The navigator's picture (`MapEditor::minimap`) as `app.rs` uploaded
    /// it before the frame, with the stamp it was baked under.
    pub minimap: Option<(u64, &'a Texture2D)>,
}

/// The builder's own scene target: the canvas drawn at a texel per world
/// pixel (`camera::scene_plan`) whenever the view is not the whole field
/// at its own size. `app.rs` holds one for the session; it is made the
/// first time it is needed, grown when a bigger view needs more, and made
/// again smaller when it holds four times what the view needs.
#[derive(Default)]
pub struct BuilderScene {
    held: Option<(RenderTexture2D, (i32, i32))>,
}

impl BuilderScene {
    /// The target, holding at least `need` texels.
    fn target(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, need: (i32, i32)) -> Option<&mut RenderTexture2D> {
        let need = (need.0.max(1), need.1.max(1));
        let fits = |size: (i32, i32)| size.0 >= need.0 && size.1 >= need.1 && size.0 * size.1 <= 4 * need.0 * need.1;
        if !self.held.as_ref().is_some_and(|(_, size)| fits(*size)) {
            self.held = None;
            let texture = rl.load_render_texture(thread, need.0 as u32, need.1 as u32).ok()?;
            self.held = Some((texture, need));
        }
        self.held.as_mut().map(|(texture, _)| texture)
    }
}

/// The builder's `Sheet` lookup, for the `ground::draw` it shares with the
/// game. It holds the sheets a map can show at rest; a sheet only a live
/// round draws from (damage, tracks, blasts, the frog's other clips) is a
/// programming error here.
impl Sheets for EditorTextures<'_> {
    fn blocks_texture(&self, stamp: u64) -> Option<&Texture2D> {
        self.shade.filter(|(held, _)| *held == stamp).map(|(_, texture)| texture)
    }

    fn texture(&self, sheet: Sheet) -> &Texture2D {
        match sheet {
            // Picked per frame by `app.rs` from the canvas's theme, the
            // theme `render` names here.
            Sheet::Ground(_) => self.ground,
            Sheet::Walls => self.obstacles,
            Sheet::Props => self.props,
            Sheet::Trees => self.trees,
            Sheet::Towers => self.towers,
            Sheet::Grass(_) => self.grass,
            Sheet::Portal => self.portal,
            Sheet::Tanks => self.tanks,
            Sheet::Frog { clip: FrogAnim::Idle, .. } => self.frog_idle,
            Sheet::Pickup(PickupKind::Health) => self.pickup_health,
            Sheet::Pickup(PickupKind::Ammo) => self.pickup_ammo,
            Sheet::Pickup(PickupKind::Laser) => self.pickup_laser,
            Sheet::Pickup(PickupKind::Minigun) => self.pickup_minigun,
            Sheet::Pickup(PickupKind::Plasma) => self.pickup_plasma,
            Sheet::Pickup(PickupKind::Missiles) => self.pickup_missiles,
            Sheet::Pickup(PickupKind::SpeedUp) => self.pickup_speedup,
            Sheet::Pickup(PickupKind::Shield) => self.pickup_shield,
            Sheet::Pickup(PickupKind::Flamethrower) => self.pickup_flamethrower,
            Sheet::Pickup(PickupKind::FrogHealth) => self.pickup_frog_health,
            Sheet::Pickup(PickupKind::TowerPack) => self.pickup_tower_pack,
            Sheet::TankGlow
            | Sheet::TankModules
            | Sheet::TankModulesGlow
            | Sheet::Tracks
            | Sheet::BarrelExplosion
            | Sheet::Frog { .. } => {
                panic!("the builder has no {sheet:?} sheet")
            }
        }
    }
}
impl MapEditor {
    /// The field cell under the last pointer position, when that is on
    /// the canvas and not on chrome - what the hover highlight and the
    /// cursor readout show, through the camera like every press.
    fn cursor_cell(&self, layout: &Layout) -> Option<(i32, i32)> {
        self.cell_at(self.pointer?, layout)
    }

    /// Draw the builder and put it on the window through `view`. At FIT on
    /// an arena - the camera `Camera::whole` over the field's own bitmap -
    /// the canvas, the bar and any open popup are drawn into `composite`
    /// (the bitmap: the bar over the field, `layout.window_size()` in size)
    /// and that is presented, exactly as `Game::render` does for an arena's
    /// round. Any other view draws the canvas into `scene` at a texel per
    /// world pixel (`scene_plan`), the way a round draws its world, and
    /// puts the view's texels straight onto the window's canvas area
    /// (`window_mapping`), so a whole-block zoom keeps its blocks whole on
    /// the glass; the bar and the popups go over it.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        composite: &mut RenderTexture2D,
        scene: &mut BuilderScene,
        view: &crate::view::View,
        backdrop: Color,
        layout: &Layout,
        textures: &EditorTextures,
    ) {
        let cursor = self.cursor_cell(layout);
        // A clock read, not an input: the builder has no round clock, and
        // the wall clock animates the water and turns a placed portal so
        // the author sees what a round will show.
        let time = rl.get_time() as f32;
        let camera = self.view_camera(layout);
        let field = self.map.field_size();
        if camera == crate::view::Camera::whole(field) && (layout.field.w, layout.field.h) == field {
            self.render_whole(rl, thread, composite, view, backdrop, layout, textures, cursor, time);
            return;
        }
        let plan = scene_plan(&camera);
        // Room for the view at FIT too, which is the most a zoom of this
        // map in this window ever needs: a pinch never reallocates.
        let mut fit = self.camera;
        fit.fit();
        let most = scene_plan(&fit.view(&self.viewport_in(layout))).size;
        let Some(target) = scene.target(rl, thread, (plan.size.0.max(most.0), plan.size.1.max(most.1))) else {
            return;
        };
        let cull = camera.cull();
        let in_target = Camera2D { offset: Vector2::new(0.0, 0.0), target: plan.origin.into(), rotation: 0.0, zoom: plan.zoom };
        let held = Rectangle::new(
            plan.origin.x,
            plan.origin.y,
            target.texture.width as f32 / plan.zoom,
            target.texture.height as f32 / plan.zoom,
        );
        rl.draw_texture_mode(thread, target, |mut d| {
            d.clear_background(CANVAS_FILL);
            d.draw_mode2D(in_target, |mut d, _| {
                self.draw_canvas(&mut d, textures, time, cursor, cull);
                cover_past_field(&mut d, field, held);
            });
        });
        // A render texture reads back bottom-up.
        let height = target.texture.height as f32;
        let s = plan.source;
        let source = Rectangle::new(s.x, height - s.y - s.height, s.width, -s.height);
        let area = window_mapping(&camera, view, layout).area;
        let base = Camera2D { offset: view.offset.into(), target: Vector2::new(0.0, 0.0), rotation: 0.0, zoom: view.scale };
        rl.draw(thread, |mut d| {
            d.clear_background(Color::BLACK);
            crate::render::view::letterbox(&mut d, view, backdrop);
            d.draw_texture_pro(&*target, source, area, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
            d.draw_mode2D(base, |mut d, _| {
                self.draw_status_line(&mut d, cursor, EDITOR_TOOLBAR_MARGIN, layout.field.y + layout.field.h - 22.0);
                self.draw_navigator(&mut d, layout, textures, &camera);
                self.draw_chrome(&mut d, layout, textures, cursor);
            });
        });
    }

    /// The navigator (`navigator_rect`), under the bar and the popups: the
    /// canvas's picture as its upload holds it, and the outline of what
    /// `camera` shows over it.
    fn draw_navigator(&self, d: &mut impl RaylibDraw, layout: &Layout, textures: &EditorTextures, camera: &crate::view::Camera) {
        let (Some(rect), Some((stamp, texture))) = (self.navigator_rect(layout), textures.minimap) else { return };
        if stamp != self.minimap.image().stamp {
            return;
        }
        let field = self.map.field_size();
        let marks = crate::minimap::Marks { view: Some(camera.rect()), ..crate::minimap::Marks::default() };
        let font = crate::indicators::label_font(self.viewport_in(layout).px(1.0));
        let picture = crate::minimap::picture(&marks, rect, field, font, &crate::tuning::tuning());
        crate::render::minimap::draw_minimap(d, rect, texture, field, &picture, 1.0);
    }

    /// The builder at FIT on an arena: the canvas, the bar and the popups
    /// into the bitmap, then the bitmap onto the window.
    #[allow(clippy::too_many_arguments)]
    fn render_whole(
        &self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        composite: &mut RenderTexture2D,
        view: &crate::view::View,
        backdrop: Color,
        layout: &Layout,
        textures: &EditorTextures,
        cursor: Option<(i32, i32)>,
        time: f32,
    ) {
        let height = layout.field.h;
        let camera = Camera2D {
            offset: layout.field_origin().into(),
            target: Vector2::new(0.0, 0.0),
            rotation: 0.0,
            zoom: 1.0,
        };
        rl.draw_texture_mode(thread, composite, |mut d| {
            d.clear_background(CANVAS_FILL);
            d.draw_mode2D(camera, |mut d, _| {
                self.draw_canvas(&mut d, textures, time, cursor, None);
                self.draw_status_line(&mut d, cursor, EDITOR_TOOLBAR_MARGIN, height - 22.0);
            });
            self.draw_chrome(&mut d, layout, textures, cursor);
        });
        crate::render::view::present(rl, thread, composite, view, backdrop);
    }

    /// The bar and the open popup, in bitmap pixels.
    fn draw_chrome(&self, d: &mut impl RaylibDraw, layout: &Layout, textures: &EditorTextures, cursor: Option<(i32, i32)>) {
        self.draw_bar(d, layout, textures, cursor);
        match &self.popup {
            None => {}
            Some(Popup::Dropdown(category)) => self.draw_dropdown(d, layout, textures, *category),
            Some(Popup::Settings) => self.draw_settings(d, layout, textures),
            Some(Popup::File) => Self::draw_file_menu(d, layout),
            Some(Popup::Load { entries, scroll }) => Self::draw_load_list(d, layout, entries, *scroll),
            Some(Popup::Save { name }) => {
                let panel = Self::save_prompt_rect(layout);
                draw_panel(d, panel);
                d.draw_text(&text().get(keys::EDITOR_SAVE_AS), (panel.x + 12.0) as i32, (panel.y + 10.0) as i32, 16, TEXT);
                d.draw_text(&format!("{name}_"), (panel.x + 12.0) as i32, (panel.y + 34.0) as i32, 18, TEXT);
                d.draw_text(
                    &text().get(keys::EDITOR_SAVE_HINT),
                    (panel.x + 12.0) as i32,
                    (panel.y + 58.0) as i32,
                    12,
                    Color::GRAY,
                );
            }
        }
    }

    /// The status line, bottom-left of the canvas at (`x`, `y`): the
    /// active tool, the cell under the pointer (or the last tapped one),
    /// and any message.
    fn draw_status_line(&self, d: &mut impl RaylibDraw, cursor: Option<(i32, i32)>, x: f32, y: f32) {
        let mut line = format!("{}: {}", self.active_category().map_or_else(|| text().get(keys::EDITOR_TOOL), |c| c.label()), short_label(self.active_tool));
        if let Some((col, row)) = cursor {
            let under = self.map.cell(col, row).map(cell_label).unwrap_or_default();
            line.push_str(&format!("   {col},{row} {under}"));
        }
        if let Some(status) = &self.status {
            line.push_str(&format!("   {status}"));
        }
        d.draw_text(&line, x as i32, y as i32, 14, Color::LIGHTGRAY);
    }

    /// The canvas in world pixels - ground, placed objects and the hover
    /// highlight - for a `Camera2D` that puts the world where the frame
    /// shows it. `cull` is the world rectangle worth drawing
    /// (`Camera::cull`); `None` draws everything.
    fn draw_canvas<D: RaylibDraw>(&self, d: &mut D, textures: &EditorTextures, time: f32, cursor: Option<(i32, i32)>, cull: Option<Rectangle>) {
        let (width, height) = self.map.field_size();
        let culled = |pos: Position, reach: f32| {
            cull.is_some_and(|r| pos.x < r.x - reach || pos.y < r.y - reach || pos.x > r.x + r.width + reach || pos.y > r.y + r.height + reach)
        };
        {
            if self.plain_canvas {
                d.draw_rectangle(0, 0, width as i32, height as i32, Color::WHITE);
            } else {
                ground::draw(&mut GpuCanvas::culled(&mut *d, textures, cull), &self.ground, self.map.theme, time);
                ground::draw_shade(&mut GpuCanvas::new(&mut *d, textures), &self.ground);
            }

            // Portals first, under every other cell: three cells of art on
            // one anchor cell, and `iter_cells` is row-sorted, so a portal
            // drawn in the loop would cover a wall placed above it. Ghosted
            // while the network is inactive (fewer than two), with the
            // anchor outlined like a gate off its edge - placed, not
            // usable. `time` is the wall clock above.
            let portals = self.map.portal_cells();
            let active = portals.len() >= 2;
            let tint = if active { Color::WHITE } else { Color::new(255, 255, 255, 110) };
            for &(col, row) in &portals {
                let pos = map::cell_to_world(col, row);
                if culled(pos, 0.0) {
                    continue;
                }
                draw_portal(&mut GpuCanvas::new(&mut *d, textures), pos, time, tint);
                if !active {
                    let size = OBSTACLE_GRID_SIZE;
                    d.draw_rectangle_lines_ex(Rectangle::new(pos.x - size / 2.0, pos.y - size / 2.0, size, size), 2.0, GATE_COLOR);
                }
            }

            for (col, row, obj) in self.map.iter_cells() {
                let pos = map::cell_to_world(col, row);
                // A tower's reach ring spreads far past its cell.
                let reach = obj.tower().map_or(0.0, |(kind, _)| kind.range());
                if culled(pos, reach) {
                    continue;
                }
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
                    CellObject::Tesla { .. } | CellObject::GunTower { .. } | CellObject::BioSlush { .. } => {
                        // At the sprite's own 48px like a tree, base then
                        // top pointing up, with its reach ringed faintly so
                        // a map maker sees what it covers.
                        let (kind, side) = obj.tower().expect("tower cells name a tower");
                        let big = crate::TREE_TEXTURE_SIZE;
                        let dest = Rectangle::new(pos.x, pos.y, big, big);
                        let origin = Vector2::new(big / 2.0, big / 2.0);
                        let reach = if side == crate::frog::Side::Enemy { Color::new(230, 60, 60, 70) } else { Color::new(77, 155, 230, 70) };
                        d.draw_circle_lines(pos.x as i32, pos.y as i32, kind.range(), reach);
                        for src in [crate::tower::icon_source_rec(kind, side), crate::tower::icon_top_rec(kind, side)] {
                            d.draw_texture_pro(textures.towers, src, dest, origin, 0.0, Color::WHITE);
                        }
                    }
                    CellObject::Road | CellObject::Water => {} // already painted into `self.ground`
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
                        draw_player_ring(d, pos, size / 2.0, 0);
                        for src in crate::tank::icon_source_recs(0) {
                            d.draw_texture_pro(textures.tanks, src, dest, origin, 0.0, Color::WHITE);
                        }
                    }
                    CellObject::Start2 => {
                        draw_player_ring(d, pos, size / 2.0, 1);
                        for src in crate::tank::icon_source_recs(1) {
                            d.draw_texture_pro(textures.tanks, src, dest, origin, 0.0, Color::WHITE);
                        }
                    }
                    CellObject::EnemyFrog => {
                        draw_enemy_ring(d, pos, size / 2.0);
                        let src = Rectangle::new(0.0, 0.0, crate::FROG_TEXTURE_SIZE, crate::FROG_TEXTURE_SIZE);
                        d.draw_texture_pro(textures.frog_idle, src, dest, origin, 0.0, Color::WHITE);
                    }
                    CellObject::Gate => match gate_inward(pos, width, height) {
                        Some(inward) => draw_gate_chevron(d, pos, size, inward),
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
                    // Drawn in the pre-pass above.
                    CellObject::Portal => {}
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

        d.draw_text(&text().get(keys::EDITOR_BUILD), px + SLOT_BUILD as i32, text_y, HUD_TEXT_SIZE, BUILD_ACCENT);
        let name = format!("{}{}", self.name(), if self.dirty() { " *" } else { "" });
        d.draw_text(&fit_text(&name, NAME_W, HUD_TEXT_SIZE), px + SLOT_NAME as i32, text_y, HUD_TEXT_SIZE, TEXT);

        for category in Category::ALL {
            self.draw_category_button(d, layout, textures, category);
        }

        let erase = Self::erase_rect(layout);
        draw_tool_icon(d, textures, self.map.theme, Tool::Eraser, Rectangle::new(erase.x + 4.0, erase.y, ICON_PX, ICON_PX));
        if self.active_tool == Tool::Eraser {
            active_outline(d, erase.x as i32, py, (SMALL_BUTTON_W - 4.0) as i32, ph, BUILD_ACCENT);
        }

        let undo_color = if self.history.undo_depth() > 0 { TEXT } else { DIM };
        draw_small_button(d, Self::undo_rect(layout), &text().get(keys::EDITOR_UNDO), undo_color);
        let redo_color = if self.history.redo_depth() > 0 { TEXT } else { DIM };
        draw_small_button(d, Self::redo_rect(layout), &text().get(keys::EDITOR_REDO), redo_color);

        let file_open = matches!(self.popup, Some(Popup::File | Popup::Load { .. } | Popup::Save { .. }));
        draw_menu_button(d, Self::file_rect(layout), &text().get(keys::EDITOR_FILE), file_open);
        draw_menu_button(d, Self::map_rect(layout), &text().get(keys::EDITOR_MAP), matches!(self.popup, Some(Popup::Settings)));
        // FIT, dim while the whole canvas is what is shown.
        let fit_color = if self.camera.is_fit() { DIM } else { TEXT };
        draw_small_button(d, Self::fit_rect(layout), &text().get(keys::EDITOR_FIT), fit_color);

        let _ = cursor; // the readout is the field's status line, see `render`
        // PLAY HERE beside PLAY, in PLAY's amber.
        draw_small_button(d, Self::here_rect(layout), &text().get(keys::EDITOR_PLAY_HERE), BUILD_ACCENT);
        crate::render::hud::draw_mode_button(d, panel, &text().get(keys::BUTTON_PLAY), BUILD_ACCENT);
    }

    /// The FILE menu below its button.
    fn draw_file_menu(d: &mut impl RaylibDraw, layout: &Layout) {
        draw_panel(d, Self::file_menu_rect(layout));
        for (i, row) in FileRow::all().iter().enumerate() {
            let rect = Self::file_row_rect(layout, i);
            d.draw_text(&row.label(), rect.x as i32 + 16, (rect.y + (rect.height - HUD_TEXT_SIZE as f32) / 2.0) as i32, HUD_TEXT_SIZE, TEXT);
        }
    }

    /// The Load list: one row per map, shipped ones marked, a scroll hint
    /// when there are more than fit.
    fn draw_load_list(d: &mut impl RaylibDraw, layout: &Layout, entries: &[MapEntry], scroll: usize) {
        let panel = Self::load_panel_rect(layout, entries.len());
        draw_panel(d, panel);
        if entries.is_empty() {
            d.draw_text(&text().get(keys::EDITOR_NO_MAPS), panel.x as i32 + 16, panel.y as i32 + 15, HUD_TEXT_SIZE, DIM);
            return;
        }
        let rows = Self::load_page_rows(entries.len());
        for (i, entry) in entries.iter().skip(scroll).take(rows).enumerate() {
            let row = Self::load_row_rect(panel, i);
            let text_y = (row.y + (row.height - HUD_TEXT_SIZE as f32) / 2.0) as i32;
            d.draw_text(&fit_text(&entry.name, LOAD_PANEL_W - 120.0, HUD_TEXT_SIZE), row.x as i32 + 16, text_y, HUD_TEXT_SIZE, TEXT);
            if !entry.on_disk {
                d.draw_text(&text().get(keys::EDITOR_SHIPPED), (row.x + row.width - 80.0) as i32, text_y, HUD_TEXT_SIZE, DIM);
            }
        }
        if entries.len() > rows {
            // The pager: `<` at the left end, `>` at the right, each dim at
            // its own end of the list, and the span on screen between them.
            let pager = Self::load_row_rect(panel, rows);
            let last = entries.len() - rows;
            let arrow_y = (pager.y + (pager.height - HUD_TEXT_SIZE as f32) / 2.0) as i32;
            d.draw_text("<", pager.x as i32 + 16, arrow_y, HUD_TEXT_SIZE, if scroll > 0 { TEXT } else { DIM });
            let right = pager.x + pager.width - 16.0 - text_width(">", HUD_TEXT_SIZE);
            d.draw_text(">", right as i32, arrow_y, HUD_TEXT_SIZE, if scroll < last { TEXT } else { DIM });
            let hint = text().fmt(
                keys::EDITOR_PAGE,
                &[("from", (scroll + 1).into()), ("to", (scroll + rows).min(entries.len()).into()), ("n", entries.len().into())],
            );
            let hint_x = pager.x + (pager.width - text_width(&hint, HUD_LABEL_SIZE)) / 2.0;
            d.draw_text(&hint, hint_x as i32, (pager.y + (pager.height - HUD_LABEL_SIZE as f32) / 2.0) as i32, HUD_LABEL_SIZE, DIM);
        }
    }

    /// One category button: the current tool's icon full-bleed at the
    /// left and a caret beside it, outlined in the accent while the
    /// active brush is one of its tools. The tool's name is in the
    /// field's status line.
    fn draw_category_button(&self, d: &mut impl RaylibDraw, layout: &Layout, textures: &EditorTextures, category: Category) {
        let rect = Self::category_rect(layout, category);
        let tool = self.current_tool(category);
        let (x, y, h) = (rect.x as i32, rect.y as i32, rect.height as i32);
        draw_tool_icon(d, textures, self.map.theme, tool, Rectangle::new(rect.x, rect.y, ICON_PX, ICON_PX));
        if self.singleton_placed(tool) {
            draw_badge(d, rect.x + ICON_PX, rect.y);
        }
        let open = matches!(self.popup, Some(Popup::Dropdown(c)) if c == category);
        let label_color = if open { BUILD_ACCENT } else { DIM };
        draw_caret(d, x + CATEGORY_CARET_X, y + (h - CARET_W) / 2, label_color);
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
            draw_tool_icon(d, textures, self.map.theme, tool, icon);
            if self.singleton_placed(tool) {
                draw_badge(d, icon.x + icon.width - 4.0, icon.y + 4.0);
            }
            let text_y = row.y as i32 + (EDITOR_DROPDOWN_ROW_H as i32 - HUD_TEXT_SIZE) / 2;
            d.draw_text(&label(tool), row.x as i32 + DROPDOWN_TEXT_X, text_y, HUD_TEXT_SIZE, TEXT);
        }
    }

    /// The MAP settings panel (docs/game-editor-fusion.md section 9): a
    /// stepper per map key and the RESET MAP button.
    fn draw_settings(&self, d: &mut impl RaylibDraw, layout: &Layout, textures: &EditorTextures) {
        draw_hanging_panel(d, Self::settings_rect(layout));
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
                let label = row.label();
                let w = text_width(&label, HUD_TEXT_SIZE);
                d.draw_text(&label, (button.x + (button.width - w) / 2.0) as i32, text_y, HUD_TEXT_SIZE, color);
                continue;
            }
            let dim = waves_off && row.is_wave_row();
            let label_color = if dim { Color::new(70, 70, 76, 255) } else { DIM };
            let value_color = if dim { DIM } else { TEXT };
            d.draw_text(&row.label(), rect.x as i32 + SETTINGS_INSET as i32, text_y, SETTINGS_LABEL_SIZE, label_color);
            if let Some((kind, player)) = row.chassis(&settings) {
                let icon = settings_icon_rect(rect);
                for src in crate::tank::chassis_icon_source_recs(kind, player) {
                    d.draw_texture_pro(textures.tanks, src, icon, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
                }
            }
            for (button, glyph) in [(Self::settings_dec_rect(rect), "<"), (Self::settings_inc_rect(rect), ">")] {
                let inset = Rectangle::new(button.x + 2.0, button.y + 4.0, button.width - 4.0, button.height - 8.0);
                d.draw_rectangle_rounded_lines_ex(inset, 0.2, EDITOR_PANEL_SEGMENTS, 1.0, Color::new(255, 255, 255, 60));
                let w = text_width(glyph, HUD_TEXT_SIZE);
                d.draw_text(glyph, (button.x + (button.width - w) / 2.0) as i32, text_y, HUD_TEXT_SIZE, value_color);
            }
            let value = row.value(&settings, self.size_cells());
            let value_x = rect.x as i32 + SETTINGS_VALUE_X;
            if *row == SettingsRow::Anchor {
                draw_anchor(d, Rectangle::new(value_x as f32, rect.y, 28.0, rect.height), self.resize_anchor, value_color);
            }
            d.draw_text(&value, value_x, text_y, HUD_TEXT_SIZE, value_color);
            if row.cli_override(self.cli_overrides) {
                let x = value_x + text_width(&value, HUD_TEXT_SIZE) as i32 + 4;
                d.draw_text(&text().get(keys::SETTINGS_CLI), x, text_y + 5, HUD_LABEL_SIZE, DIM);
            }
        }
    }
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
        PickupKind::Missiles => textures.pickup_missiles,
        PickupKind::SpeedUp => textures.pickup_speedup,
        PickupKind::Shield => textures.pickup_shield,
        PickupKind::Flamethrower => textures.pickup_flamethrower,
        PickupKind::FrogHealth => textures.pickup_frog_health,
        PickupKind::TowerPack => textures.pickup_tower_pack,
    }
}

/// Draw a rounded, bordered, drop-shadowed panel background - shared by
/// the builder's panels (the dropdowns, the FILE menu and its Load list,
/// the Save prompt), per docs/map-editor-design.md's "Panel chrome"
/// section; the MAP panel is `draw_hanging_panel`'s.
pub fn draw_panel(d: &mut impl RaylibDraw, rect: Rectangle) {
    d.draw_rectangle_rounded(panel_shadow(rect), EDITOR_PANEL_ROUNDNESS, EDITOR_PANEL_SEGMENTS, PANEL_SHADOW);
    d.draw_rectangle_rounded(rect, EDITOR_PANEL_ROUNDNESS, EDITOR_PANEL_SEGMENTS, PANEL_FILL);
    d.draw_rectangle_rounded_lines_ex(rect, EDITOR_PANEL_ROUNDNESS, EDITOR_PANEL_SEGMENTS, EDITOR_PANEL_BORDER_THICKNESS, PANEL_BORDER);
}

/// A panel hanging from the bar - the MAP settings panel: `draw_panel`'s
/// shadow, fill and border with the top corners square, so the panel
/// meets the bar's straight edge, and the bottom ones rounded as raylib
/// rounds them (the radius `EDITOR_PANEL_ROUNDNESS` of half the shorter
/// side, the border outside the panel).
pub fn draw_hanging_panel(d: &mut impl RaylibDraw, rect: Rectangle) {
    let r = rect.width.min(rect.height) * EDITOR_PANEL_ROUNDNESS / 2.0;
    fill_hanging(d, panel_shadow(rect), r, PANEL_SHADOW);
    fill_hanging(d, rect, r, PANEL_FILL);
    // The border: a run across the top and down both sides to the
    // corners, a quarter ring round each, the bottom edge between them.
    let (x, y, w, h, t) = (rect.x, rect.y, rect.width, rect.height, EDITOR_PANEL_BORDER_THICKNESS);
    d.draw_rectangle_rec(Rectangle::new(x - t, y - t, w + 2.0 * t, t), PANEL_BORDER);
    d.draw_rectangle_rec(Rectangle::new(x - t, y, t, h - r), PANEL_BORDER);
    d.draw_rectangle_rec(Rectangle::new(x + w, y, t, h - r), PANEL_BORDER);
    d.draw_rectangle_rec(Rectangle::new(x + r, y + h, w - 2.0 * r, t), PANEL_BORDER);
    d.draw_ring(Vector2::new(x + r, y + h - r), r, r + t, 90.0, 180.0, EDITOR_PANEL_SEGMENTS, PANEL_BORDER);
    d.draw_ring(Vector2::new(x + w - r, y + h - r), r, r + t, 0.0, 90.0, EDITOR_PANEL_SEGMENTS, PANEL_BORDER);
}

/// The hanging panel's shape in one colour, laid in pieces that never
/// overlap, since every panel colour is translucent: all of it above the
/// bottom corners, the strip between them and a quarter disc in each.
fn fill_hanging(d: &mut impl RaylibDraw, rect: Rectangle, r: f32, color: Color) {
    let (x, y, w, h) = (rect.x, rect.y, rect.width, rect.height);
    d.draw_rectangle_rec(Rectangle::new(x, y, w, h - r), color);
    d.draw_rectangle_rec(Rectangle::new(x + r, y + h - r, w - 2.0 * r, r), color);
    d.draw_circle_sector(Vector2::new(x + r, y + h - r), r, 90.0, 180.0, EDITOR_PANEL_SEGMENTS, color);
    d.draw_circle_sector(Vector2::new(x + w - r, y + h - r), r, 0.0, 90.0, EDITOR_PANEL_SEGMENTS, color);
}

/// A panel's drop shadow: the panel moved down and right.
fn panel_shadow(rect: Rectangle) -> Rectangle {
    Rectangle::new(rect.x + EDITOR_PANEL_SHADOW_OFFSET, rect.y + EDITOR_PANEL_SHADOW_OFFSET, rect.width, rect.height)
}

// Every panel's colours, from lib.rs's `EDITOR_PANEL_*` knobs.
const PANEL_SHADOW: Color = Color::new(0, 0, 0, (255.0 * EDITOR_PANEL_SHADOW_OPACITY) as u8);
const PANEL_FILL: Color =
    Color::new(EDITOR_PANEL_FILL.0, EDITOR_PANEL_FILL.1, EDITOR_PANEL_FILL.2, (255.0 * EDITOR_PANEL_FILL_OPACITY) as u8);
const PANEL_BORDER: Color = Color::new(0, 0, 0, (255.0 * EDITOR_PANEL_BORDER_OPACITY) as u8);

/// A tool's icon inside `rect` (4 px inset).
pub fn draw_tool_icon(d: &mut impl RaylibDraw, textures: &EditorTextures, theme: Theme, tool: Tool, rect: Rectangle) {
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
        Tool::Water => {
            // The pack's flat water, straight off the theme's tileset.
            d.draw_texture_pro(textures.ground, ground::water_icon_source_rec(), dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        Tool::Frog => {
            let src = Rectangle::new(0.0, 0.0, crate::FROG_TEXTURE_SIZE, crate::FROG_TEXTURE_SIZE);
            d.draw_texture_pro(textures.frog_idle, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        Tool::Start => {
            let center = Position::new(dest.x + dest.width / 2.0, dest.y + dest.height / 2.0);
            draw_player_ring(d, center, dest.width / 2.0, 0);
            for src in crate::tank::icon_source_recs(0) {
                d.draw_texture_pro(textures.tanks, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
            }
        }
        Tool::Start2 => {
            let center = Position::new(dest.x + dest.width / 2.0, dest.y + dest.height / 2.0);
            draw_player_ring(d, center, dest.width / 2.0, 1);
            for src in crate::tank::icon_source_recs(1) {
                d.draw_texture_pro(textures.tanks, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
            }
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
        Tool::Portal => {
            d.draw_texture_pro(textures.portal, portal_icon_source_rec(), dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        Tool::TallGrass => {
            // One tuft from the nature sheet on a patch of the theme's
            // floor, so the icon reads as grass-on-ground rather than
            // loose pixels.
            let (r, g, b) = theme.floor_color();
            d.draw_rectangle_rounded(dest, 0.15, EDITOR_PANEL_SEGMENTS, Color::new(r, g, b, 255));
            let cell = crate::GRASS_TEXTURE_SIZE;
            let src = Rectangle::new(0.0, 0.0, cell, cell);
            d.draw_texture_pro(textures.grass, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        Tool::Pickup(pickup) => {
            let texture = pickup_texture(textures, pickup);
            let src = Rectangle::new(0.0, 0.0, crate::PICKUP_TEXTURE_SIZE, crate::PICKUP_TEXTURE_SIZE);
            d.draw_texture_pro(texture, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        Tool::Tower(kind, side) => {
            // Base and top as a round draws them, the top pointing up.
            for src in [crate::tower::icon_source_rec(kind, side), crate::tower::icon_top_rec(kind, side)] {
                d.draw_texture_pro(textures.towers, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
            }
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
        obstacle::Sheet::Towers => textures.towers,
        other => panic!("a map cell never draws from {other:?}"),
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

/// A player's team-coloured ring, the same shape as the enemy frog's red
/// one, under the start markers: a `start` cell reads as "a tank, the blue
/// one" and `start2` as the pink one, the colours the tanks will be.
fn draw_player_ring(d: &mut impl RaylibDraw, center: Position, radius: f32, player: usize) {
    let c = crate::tank::team_color(player as u8);
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

impl FileRow {
    fn label(self) -> String {
        text().get(match self {
            FileRow::Load => keys::FILE_LOAD,
            FileRow::Save => keys::FILE_SAVE,
            FileRow::SaveAs => keys::FILE_SAVE_AS,
            FileRow::Clear => keys::FILE_CLEAR,
        })
    }
}

impl SettingsRow {
    fn label(self) -> String {
        text().get(match self {
            SettingsRow::Tanks => keys::SETTINGS_TANKS,
            SettingsRow::Tank => keys::SETTINGS_TANK,
            SettingsRow::Tank2 => keys::SETTINGS_TANK2,
            SettingsRow::Mission => keys::SETTINGS_MISSION,
            SettingsRow::Spawn => keys::SETTINGS_SPAWN,
            SettingsRow::Waves => keys::SETTINGS_WAVES,
            SettingsRow::Size => keys::SETTINGS_SIZE,
            SettingsRow::Growth => keys::SETTINGS_GROWTH,
            SettingsRow::TierStart => keys::SETTINGS_TIER_START,
            SettingsRow::TierEnd => keys::SETTINGS_TIER_END,
            SettingsRow::Theme => keys::SETTINGS_THEME,
            SettingsRow::Weather => keys::SETTINGS_WEATHER,
            SettingsRow::Width => keys::SETTINGS_WIDTH,
            SettingsRow::Height => keys::SETTINGS_HEIGHT,
            SettingsRow::Anchor => keys::SETTINGS_ANCHOR,
            SettingsRow::Reset => keys::SETTINGS_RESET,
        })
    }

    /// The chassis a TANK row has picked and the seat whose colours it
    /// is drawn in; `None` on `auto` and on every other row.
    fn chassis(self, s: &MapSettings) -> Option<(TankKind, u8)> {
        match self {
            SettingsRow::Tank => s.tank.map(|kind| (kind, 0)),
            SettingsRow::Tank2 => s.tank2.map(|kind| (kind, 1)),
            _ => None,
        }
    }

    /// One of the five rows that only matter to a Waves spawn plan.
    fn is_wave_row(self) -> bool {
        matches!(
            self,
            SettingsRow::Waves | SettingsRow::Size | SettingsRow::Growth | SettingsRow::TierStart | SettingsRow::TierEnd
        )
    }

    /// The row's value as the panel shows it, in the language on screen:
    /// a data name looked up by its family (`tank-scout`, `theme-desert`),
    /// the word for `auto` where the map leaves it to the game, or the
    /// map's size in cells (`size`, columns and rows). The ANCHOR row is a
    /// picture (`draw_anchor`), no words.
    fn value(self, s: &MapSettings, size: (f32, f32)) -> String {
        let t = text();
        let auto_or = |v: Option<String>| v.unwrap_or_else(|| t.get(keys::SETTINGS_AUTO));
        match self {
            SettingsRow::Tanks => auto_or(s.tanks.map(|n| n.to_string())),
            SettingsRow::Tank => auto_or(s.tank.map(|k| t.named("tank", k.name()))),
            SettingsRow::Tank2 => auto_or(s.tank2.map(|k| t.named("tank", k.name()))),
            SettingsRow::Mission => t.named("mission", s.mission.name()),
            SettingsRow::Spawn => t.named("spawn", s.spawn.name()),
            SettingsRow::Waves => auto_or(s.waves.map(|n| n.to_string())),
            SettingsRow::Size => auto_or(s.size.map(|n| n.to_string())),
            SettingsRow::Growth => auto_or(s.growth.map(|n| n.to_string())),
            SettingsRow::TierStart => auto_or(s.tier_start.map(|tier| t.named("tier", tier.name()))),
            SettingsRow::TierEnd => auto_or(s.tier_end.map(|tier| t.named("tier", tier.name()))),
            SettingsRow::Theme => t.named("theme", s.theme.name()),
            SettingsRow::Weather => t.named("weather", s.weather.name()),
            SettingsRow::Width => cells_text(size.0),
            SettingsRow::Height => cells_text(size.1),
            SettingsRow::Anchor | SettingsRow::Reset => String::new(),
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
            // No CLI flag names a theme: the map is the only source.
            SettingsRow::Theme => false,
            // `--weather` (and the web page's `?weather=`) is the
            // `weather_override` knob, which outranks every map's sky.
            SettingsRow::Weather => crate::tuning::tuning().weather_override >= 0,
            // The size is the map's alone, and the anchor the panel's.
            SettingsRow::Width | SettingsRow::Height | SettingsRow::Anchor | SettingsRow::Reset => false,
        }
    }
}

/// A size in cells as the panel shows it: whole, or with its half.
fn cells_text(cells: f32) -> String {
    if cells.fract() == 0.0 { format!("{}", cells as i32) } else { format!("{cells}") }
}

/// Paint the world `held` shows past the field's edge in the canvas fill.
/// The ground reaches half a tile past every edge and a tree, a tower's
/// reach ring or a gate's chevron in an edge cell leans out of it; a
/// round's field area clips all of that, and so does this, so a view that
/// shows past the edge draws the map its own size.
fn cover_past_field(d: &mut impl RaylibDraw, field: (f32, f32), held: Rectangle) {
    let (w, h) = field;
    let (left, top, right, bottom) = (held.x, held.y, held.x + held.width, held.y + held.height);
    for strip in [
        Rectangle::new(left, top, right - left, -top),
        Rectangle::new(left, h, right - left, bottom - h),
        Rectangle::new(left, 0.0, -left, h),
        Rectangle::new(w, 0.0, right - w, h),
    ] {
        if strip.width > 0.0 && strip.height > 0.0 {
            d.draw_rectangle_rec(strip, CANVAS_FILL);
        }
    }
}

/// The ANCHOR row's picture in `rect`: a 3 x 3 grid of squares, the one
/// the old map sits at filled in the accent. Lines and gaps are whole
/// 2 px blocks, so the grid survives the bitmap drawn at under its size
/// on a phone.
fn draw_anchor(d: &mut impl RaylibDraw, rect: Rectangle, anchor: Anchor, color: Color) {
    const SIDE: f32 = 8.0;
    const GAP: f32 = 2.0;
    let span = 3.0 * SIDE + 2.0 * GAP;
    let (x0, y0) = (rect.x, rect.y + (rect.height - span) / 2.0);
    let (ax, ay) = anchor.grid();
    for gy in 0..3 {
        for gx in 0..3 {
            let square = Rectangle::new(x0 + gx as f32 * (SIDE + GAP), y0 + gy as f32 * (SIDE + GAP), SIDE, SIDE);
            if (gx, gy) == (ax, ay) {
                d.draw_rectangle_rec(square, BUILD_ACCENT);
            } else {
                d.draw_rectangle_lines_ex(square, 2.0, color);
            }
        }
    }
}

#[cfg(test)]
mod bar_tests {
    use super::*;
    use crate::{DEFAULT_SCREEN_HEIGHT, DEFAULT_SCREEN_WIDTH};

    const W: f32 = DEFAULT_SCREEN_WIDTH as f32;
    const H: f32 = DEFAULT_SCREEN_HEIGHT as f32;

    /// The bar's slots must never overlap and must all end before the
    /// SAVE and PLAY buttons at the default width, with the widest thing
    /// each can hold written down here.
    #[test]
    fn bar_slots_fit_the_default_bar_without_overlapping() {
        assert!(SLOT_BUILD + crate::text::width("BUILD", HUD_TEXT_SIZE) as f32 <= SLOT_NAME);
        assert!(SLOT_NAME + NAME_W <= SLOT_CATEGORIES);
        assert!(SLOT_CATEGORIES + Category::ALL.len() as f32 * CATEGORY_W <= SLOT_ERASE);
        assert!(SLOT_ERASE + SMALL_BUTTON_W <= SLOT_UNDO);
        assert!(SLOT_UNDO + SMALL_BUTTON_W <= SLOT_REDO);
        assert!(SLOT_REDO + SMALL_BUTTON_W <= SLOT_FILE);
        assert!(SLOT_FILE + MAP_BUTTON_W <= SLOT_MAP);
        assert!(SLOT_MAP + MAP_BUTTON_W <= SLOT_FIT);
        assert!(SLOT_FIT + SMALL_BUTTON_W <= SLOT_HERE, "FIT runs into PLAY HERE");
        let layout = Layout::for_field(W, H);
        let play = mode_button_rect(layout.panel);
        assert!(SLOT_HERE + HERE_W <= play.x, "PLAY HERE runs into PLAY");
        // The caret sits clear of the icon and inside the button's box.
        assert!(CATEGORY_CARET_X as f32 >= ICON_PX, "the caret overlaps the icon");
        assert!(CATEGORY_CARET_X + CARET_W <= (CATEGORY_W - BUTTON_GAP) as i32, "caret leaves the button");
    }

    /// A TANK row's chassis icon sits inside its row, between the TANK
    /// labels' budget (`text::budgets`: 80 px from the inset) and the `<`
    /// button.
    #[test]
    fn the_chassis_icon_sits_between_the_label_and_the_stepper() {
        let row = Rectangle::new(0.0, 0.0, EDITOR_SETTINGS_W, EDITOR_DROPDOWN_ROW_H);
        let icon = settings_icon_rect(row);
        assert!(icon.x >= SETTINGS_INSET + 80.0 + 4.0, "the icon runs into the TANK labels");
        assert!(icon.x + icon.width + SETTINGS_INSET <= MapEditor::settings_dec_rect(row).x, "the icon runs into the < button");
        assert!(icon.y >= row.y && icon.y + icon.height <= row.y + row.height, "the icon leaves its row");
    }

    /// The cursor readout spells a cell the way the dev server does.
    #[test]
    fn the_readout_names_cells_by_their_tool() {
        assert_eq!(cell_label(&CellObject::Start2), "start2");
        assert_eq!(cell_label(&CellObject::Start), "start");
        assert_eq!(cell_label(&CellObject::Portal), "portal");
        assert_eq!(cell_label(&CellObject::TallGrass), "grass");
    }
}
