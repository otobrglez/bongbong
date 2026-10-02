//! The builder with raylib: `MapEditor::render` (the canvas through its
//! camera, then the chrome on the window in UI points - the bar, the open
//! popup, the status line, the navigator and the loupe), the bar and popup
//! painters, the tool icons and the field markers, and `EditorTextures`,
//! the sheets the builder draws from. A child of `editor` because it draws
//! the builder's private state; the edit model, every hit test and the
//! chrome's geometry (`chrome.rs`) stay headless.

use sola_raylib::prelude::*;

use super::*;
use super::camera::{scene_plan, window_mapping};
use super::chrome::{small_text, BarTools, CategoryButton, MENU_BOX_INSET, SMALL_BOX_INSET};
use crate::text::{keys, text};
use crate::canvas::Sheet;
use crate::frog::FrogAnim;
use crate::hud::{Hints, BAR_FILL, DIM, HUD_TEXT_SIZE, TEXT, UI_SMALL_TEXT};
use crate::math::{Color, Rectangle};
use crate::obstacle;
use crate::portal::{draw_portal, portal_icon_source_rec};
use crate::render::canvas::{GpuCanvas, Sheets};
use crate::EDITOR_DROPDOWN_ROW_H;

/// The icons in the bar, the dropdown rows and the palette, the sheets'
/// own 32 px drawn a point a pixel.
const ICON_PX: f32 = 32.0;
/// The gap a button's drawn box keeps from the slot after it, so two
/// adjacent outlines never touch.
const BUTTON_GAP: f32 = 8.0;
/// A caret is 10 pt wide (`draw_caret`): a category button's sits at the
/// right end of its drawn box.
const CARET_W: i32 = 10;
/// A dropdown row's name column, right of its 32 pt icon at 8 pt inset.
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

/// A tool's name as the status line and the cursor readout spell it: the
/// catalogue's `tool-short-<name>` where the language has one, else its
/// full name.
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
    EDITOR_PANEL_SEGMENTS, EDITOR_PANEL_SHADOW_OFFSET, EDITOR_PANEL_SHADOW_OPACITY, OBSTACLE_GRID_SIZE,
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
/// at its own size, and the loupe's (`MapEditor::loupe`), the world it
/// shows drawn on its own the same way. `app.rs` holds one for the
/// session; each target is made the first time it is needed, grown when
/// a bigger picture needs more, and made again smaller when it holds four
/// times what the picture needs.
#[derive(Default)]
pub struct BuilderScene {
    held: Option<(RenderTexture2D, (i32, i32))>,
    loupe: Option<(RenderTexture2D, (i32, i32))>,
}

/// The target in `slot`, holding at least `need` texels: made, grown or
/// made again smaller as `BuilderScene` says.
fn hold<'a>(
    slot: &'a mut Option<(RenderTexture2D, (i32, i32))>,
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    need: (i32, i32),
) -> Option<&'a mut RenderTexture2D> {
    let need = (need.0.max(1), need.1.max(1));
    let fits = |size: (i32, i32)| size.0 >= need.0 && size.1 >= need.1 && size.0 * size.1 <= 4 * need.0 * need.1;
    if !slot.as_ref().is_some_and(|(_, size)| fits(*size)) {
        *slot = None;
        let texture = rl.load_render_texture(thread, need.0 as u32, need.1 as u32).ok()?;
        *slot = Some((texture, need));
    }
    slot.as_mut().map(|(texture, _)| texture)
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
    fn cursor_cell(&self, frame: &BuilderFrame) -> Option<(i32, i32)> {
        self.canvas_cell(self.pointer?, frame)
    }

    /// Draw the builder and put it on the window: the canvas through its
    /// camera, then the chrome over it on the window in UI points. At FIT
    /// on an arena - the camera `Camera::whole` over the field's own bitmap
    /// - the canvas is drawn into `composite` (the field, `layout`'s size)
    /// and that is presented through `frame.view` under the bar, exactly as
    /// `Game::render` does for an arena's round. Any other view draws the
    /// canvas into `scene` at a texel per world pixel (`scene_plan`), the
    /// way a round draws its world, and puts the view's texels straight
    /// onto the window's canvas area (`window_mapping`), so a whole-block
    /// zoom keeps its blocks whole on the glass. Either way the window
    /// round the canvas is `backdrop`, and the chrome
    /// (`draw_window_chrome`) goes over everything.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        composite: &mut RenderTexture2D,
        scene: &mut BuilderScene,
        frame: &BuilderFrame,
        backdrop: Color,
        textures: &EditorTextures,
    ) {
        let (layout, view) = (&frame.layout, &frame.view);
        let cursor = self.cursor_cell(frame);
        // A clock read, not an input: the builder has no round clock, and
        // the wall clock animates the water and turns a placed portal so
        // the author sees what a round will show.
        let time = rl.get_time() as f32;
        let camera = self.view_camera(layout);
        let field = self.map.field_size();
        let chrome = self.chrome(frame);
        if camera == crate::view::Camera::whole(field) && (layout.field.w, layout.field.h) == field {
            // The canvas into the bitmap, the bitmap onto the window.
            rl.draw_texture_mode(thread, composite, |mut d| {
                d.clear_background(CANVAS_FILL);
                self.draw_canvas(&mut d, textures, time, cursor, None);
            });
            let magnified = self.loupe(frame).and_then(|loupe| Some((loupe, self.draw_loupe_world(rl, thread, &mut scene.loupe, &loupe, textures, time)?)));
            rl.draw(thread, |mut d| {
                crate::render::view::present_into(&mut d, composite, view, backdrop, None);
                self.draw_window_chrome(&mut d, frame, &chrome, textures, cursor, &camera, magnified.as_ref().map(|(l, p)| (l, *p)));
            });
            return;
        }
        let plan = scene_plan(&camera);
        // Room for the view at FIT too, which is the most a zoom of this
        // map in this window ever needs: a pinch never reallocates.
        let mut fit = self.camera;
        fit.fit();
        let most = scene_plan(&fit.view(&self.viewport_in(layout))).size;
        let Some(target) = hold(&mut scene.held, rl, thread, (plan.size.0.max(most.0), plan.size.1.max(most.1))) else {
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
        let magnified = self.loupe(frame).and_then(|loupe| Some((loupe, self.draw_loupe_world(rl, thread, &mut scene.loupe, &loupe, textures, time)?)));
        let Some((target, _)) = scene.held.as_ref() else { return };
        rl.draw(thread, |mut d| {
            d.clear_background(Color::BLACK);
            crate::render::view::letterbox(&mut d, view, backdrop);
            d.draw_texture_pro(target, source, area, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
            self.draw_window_chrome(&mut d, frame, &chrome, textures, cursor, &camera, magnified.as_ref().map(|(l, p)| (l, *p)));
        });
    }

    /// The chrome over the canvas on the window, in UI points through
    /// `frame.ui`'s scale: the status line and the navigator, the bar, the
    /// open popup and, over a painting finger, the loupe.
    #[allow(clippy::too_many_arguments)]
    fn draw_window_chrome(
        &self,
        d: &mut impl RaylibDraw,
        frame: &BuilderFrame,
        chrome: &Chrome,
        textures: &EditorTextures,
        cursor: Option<(i32, i32)>,
        camera: &crate::view::Camera,
        magnified: Option<(&Loupe, &RenderTexture2D)>,
    ) {
        let ui = Camera2D { offset: Vector2::new(0.0, 0.0), target: Vector2::new(0.0, 0.0), rotation: 0.0, zoom: frame.ui.scale };
        d.draw_mode2D(ui, |mut d, _| {
            self.draw_status_line(&mut d, frame, chrome, cursor);
            self.draw_navigator(&mut d, frame, textures, camera);
            self.draw_bar(&mut d, &chrome.bar, textures);
            self.draw_popup(&mut d, chrome, textures, frame.ui.hints);
            if let Some((loupe, picture)) = magnified {
                draw_loupe(&mut d, loupe, picture);
            }
        });
    }

    /// The world `loupe` shows, drawn into its own target at a texel a
    /// world pixel - the canvas without the cursor's highlight, the world
    /// past the field in the canvas fill - so nothing drawn over the canvas
    /// (the status line, the navigator, the bar) and nothing past what the
    /// scene target holds ever shows in it.
    #[allow(clippy::too_many_arguments)]
    fn draw_loupe_world<'a>(
        &self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        slot: &'a mut Option<(RenderTexture2D, (i32, i32))>,
        loupe: &Loupe,
        textures: &EditorTextures,
        time: f32,
    ) -> Option<&'a RenderTexture2D> {
        let w = loupe.world;
        let target = hold(slot, rl, thread, (w.width.ceil() as i32, w.height.ceil() as i32))?;
        let held = Rectangle::new(w.x, w.y, target.texture.width as f32, target.texture.height as f32);
        let in_target = Camera2D { offset: Vector2::new(0.0, 0.0), target: Vector2::new(w.x, w.y), rotation: 0.0, zoom: 1.0 };
        let m = crate::view::CULL_MARGIN_PX;
        let cull = Rectangle::new(w.x - m, w.y - m, w.width + 2.0 * m, w.height + 2.0 * m);
        let field = self.map.field_size();
        rl.draw_texture_mode(thread, target, |mut d| {
            d.clear_background(CANVAS_FILL);
            d.draw_mode2D(in_target, |mut d, _| {
                self.draw_canvas(&mut d, textures, time, None, Some(cull));
                cover_past_field(&mut d, field, held);
            });
        });
        Some(&*target)
    }

    /// The navigator (`navigator_rect`), in UI points under the bar and
    /// the popups: the canvas's picture as its upload holds it, and the
    /// outline of what `camera` shows over it.
    fn draw_navigator(&self, d: &mut impl RaylibDraw, frame: &BuilderFrame, textures: &EditorTextures, camera: &crate::view::Camera) {
        let (Some(rect), Some((stamp, texture))) = (self.navigator_rect(frame), textures.minimap) else { return };
        if stamp != self.minimap.image().stamp {
            return;
        }
        let field = self.map.field_size();
        let marks = crate::minimap::Marks { view: Some(camera.rect()), ..crate::minimap::Marks::default() };
        let picture = crate::minimap::picture(&marks, rect, field, crate::indicators::label_font(1.0), &crate::tuning::tuning());
        crate::render::minimap::draw_minimap(d, rect, texture, field, &picture, 1.0);
    }

    /// The status line, in UI points along the bottom-left of the room
    /// under the bar (`chrome::status_line`), short of the navigator: the
    /// map's name where the bar has no room for it whole, the active tool,
    /// the cell under the pointer (or the last tapped one), and any
    /// message - cut to the room it has.
    fn draw_status_line(&self, d: &mut impl RaylibDraw, frame: &BuilderFrame, chrome: &Chrome, cursor: Option<(i32, i32)>) {
        let until = self.navigator_rect(frame).map(|r| Corners::plate(r).x);
        let at = chrome::status_line(chrome.room, until);
        let mut line = String::new();
        let name = self.bar_name();
        if chrome.bar.name.is_none_or(|r| text_width(&name, HUD_TEXT_SIZE) > r.width) {
            line.push_str(&format!("{name}   "));
        }
        line.push_str(&format!(
            "{}: {}",
            self.active_category().map_or_else(|| text().get(keys::EDITOR_TOOL), |c| c.label()),
            short_label(self.active_tool)
        ));
        if let Some((col, row)) = cursor {
            let under = self.map.cell(col, row).map(cell_label).unwrap_or_default();
            line.push_str(&format!("   {col},{row} {under}"));
        }
        if let Some(status) = &self.status {
            line.push_str(&format!("   {status}"));
        }
        let size = chrome::STATUS_TEXT;
        d.draw_text(&fit_text(&line, at.width, size), at.x as i32, at.y as i32, size, Color::LIGHTGRAY);
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

            // The finding the CHECK panel picked, over the map.
            self.draw_lint_marks(d, &culled);

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

    /// The map's name as the bar shows it, a `*` after it while edited.
    fn bar_name(&self) -> String {
        format!("{}{}", self.name(), if self.dirty() { " *" } else { "" })
    }

    /// The bar (docs/game-editor-fusion.md section 7), in UI points as
    /// `Bar::of` laid it out: the strip across the window, `BUILD` and the
    /// map's name with a `*` while edited where the bar has the room, the
    /// five category buttons or the TOOLS button they fold into, the
    /// eraser, UNDO, REDO, FILE, MAP, FIT, CHECK, the clear flag, PLAY HERE
    /// and PLAY.
    fn draw_bar(&self, d: &mut impl RaylibDraw, bar: &Bar, textures: &EditorTextures) {
        d.draw_rectangle_rec(bar.strip, BAR_FILL);
        let small = small_text(bar.touch);
        let text_y = |r: Rectangle| (r.y + (r.height - HUD_TEXT_SIZE as f32) / 2.0) as i32;
        if let Some(r) = bar.label {
            d.draw_text(&text().get(keys::EDITOR_BUILD), r.x as i32, text_y(r), HUD_TEXT_SIZE, BUILD_ACCENT);
        }
        if let Some(r) = bar.name {
            d.draw_text(&fit_text(&self.bar_name(), r.width, HUD_TEXT_SIZE), r.x as i32, text_y(r), HUD_TEXT_SIZE, TEXT);
        }
        match &bar.tools {
            BarTools::Categories(buttons) => {
                for (category, button) in Category::ALL.into_iter().zip(buttons) {
                    self.draw_category_button(d, button, textures, category, bar.touch);
                }
            }
            BarTools::Folded(r) => self.draw_tools_button(d, *r, textures, bar.touch),
        }

        let erase = icon_rect(bar.erase, bar.touch, 4.0);
        draw_tool_icon(d, textures, self.map.theme, Tool::Eraser, erase);
        if self.active_tool == Tool::Eraser {
            active_outline(d, bar.erase.x as i32, bar.erase.y as i32, (bar.erase.width - SMALL_BOX_INSET) as i32, bar.erase.height as i32, BUILD_ACCENT);
        }

        let undo_color = if self.history.undo_depth() > 0 { TEXT } else { DIM };
        draw_small_button(d, bar.undo, &text().get(keys::EDITOR_UNDO), undo_color, small);
        let redo_color = if self.history.redo_depth() > 0 { TEXT } else { DIM };
        draw_small_button(d, bar.redo, &text().get(keys::EDITOR_REDO), redo_color, small);

        let file_open = matches!(self.popup, Some(Popup::File | Popup::Load { .. } | Popup::Save { .. }));
        draw_menu_button(d, bar.file, &text().get(keys::EDITOR_FILE), file_open);
        draw_menu_button(d, bar.map, &text().get(keys::EDITOR_MAP), matches!(self.popup, Some(Popup::Settings { .. })));
        // FIT, dim while the whole canvas is what is shown.
        let fit_color = if self.camera.is_fit() { DIM } else { TEXT };
        draw_small_button(d, bar.fit, &text().get(keys::EDITOR_FIT), fit_color, small);
        // CHECK, in the accent while its panel is open.
        let check_color = if matches!(self.popup, Some(Popup::Lint { .. })) { BUILD_ACCENT } else { TEXT };
        draw_small_button(d, bar.check, &text().get(keys::EDITOR_CHECK), check_color, small);
        draw_clear_readout(d, bar.clear, self.par(), small);

        // PLAY HERE beside PLAY, in PLAY's amber.
        draw_small_button(d, bar.here, &text().get(keys::EDITOR_PLAY_HERE), BUILD_ACCENT, small);
        crate::render::hud::draw_slot_button(d, bar.play, &text().get(keys::BUTTON_PLAY), BUILD_ACCENT);
    }

    /// One category button: the current tool's icon in its icon half and
    /// a caret at the right end of its drawn box, outlined in the accent
    /// while the active brush is one of its tools. The tool's name is in
    /// the status line.
    fn draw_category_button(&self, d: &mut impl RaylibDraw, button: &CategoryButton, textures: &EditorTextures, category: Category, touch: bool) {
        let rect = button.rect;
        let tool = self.current_tool(category);
        let icon = icon_rect(button.icon, touch, 0.0);
        draw_tool_icon(d, textures, self.map.theme, tool, icon);
        if self.singleton_placed(tool) {
            draw_badge(d, icon.x + icon.width, icon.y);
        }
        let open = matches!(self.popup, Some(Popup::Dropdown(c)) if c == category);
        let caret_color = if open { BUILD_ACCENT } else { DIM };
        let caret_x = (rect.x + rect.width - BUTTON_GAP) as i32 - CARET_W;
        draw_caret(d, caret_x, (rect.y + (rect.height - CARET_W as f32) / 2.0) as i32, caret_color);
        if self.active_category() == Some(category) {
            active_outline(d, rect.x as i32, rect.y as i32, (rect.width - BUTTON_GAP) as i32, rect.height as i32, BUILD_ACCENT);
        }
    }

    /// The TOOLS button the categories fold into: the brush's icon - or,
    /// while the eraser is the brush, the tool it came from - and a caret,
    /// outlined in the accent while a category's tool is the brush.
    fn draw_tools_button(&self, d: &mut impl RaylibDraw, rect: Rectangle, textures: &EditorTextures, touch: bool) {
        let zone = Rectangle::new(rect.x, rect.y, rect.height.min(rect.width - BUTTON_GAP - CARET_W as f32), rect.height);
        let icon = icon_rect(zone, touch, 0.0);
        let tool = self.tools_button_tool();
        draw_tool_icon(d, textures, self.map.theme, tool, icon);
        if self.singleton_placed(tool) {
            draw_badge(d, icon.x + icon.width, icon.y);
        }
        let open = matches!(self.popup, Some(Popup::Palette));
        let caret_x = (rect.x + rect.width - BUTTON_GAP) as i32 - CARET_W;
        draw_caret(d, caret_x, (rect.y + (rect.height - CARET_W as f32) / 2.0) as i32, if open { BUILD_ACCENT } else { DIM });
        if self.active_category().is_some() {
            active_outline(d, rect.x as i32, rect.y as i32, (rect.width - BUTTON_GAP) as i32, rect.height as i32, BUILD_ACCENT);
        }
    }

    /// The open popup, in UI points, its hints naming `hints`' input.
    fn draw_popup(&self, d: &mut impl RaylibDraw, chrome: &Chrome, textures: &EditorTextures, hints: Hints) {
        match (&self.popup, &chrome.popup) {
            (Some(Popup::Dropdown(category)), Some(PopupLayout::Dropdown(_, rows))) => self.draw_dropdown(d, rows, textures, *category),
            (Some(Popup::Palette), Some(PopupLayout::Palette(palette))) => self.draw_palette(d, palette, textures),
            (Some(Popup::Settings { page }), Some(PopupLayout::Settings(settings))) => self.draw_settings(d, settings, *page, textures, hints),
            (Some(Popup::Lint { page }), Some(PopupLayout::Lint(lint))) => self.draw_lint_panel(d, lint, *page, hints),
            (Some(Popup::File), Some(PopupLayout::File(rows))) => Self::draw_file_menu(d, rows),
            (Some(Popup::Load { entries, scroll }), Some(PopupLayout::Load(load))) => Self::draw_load_list(d, load, entries, *scroll, hints),
            (Some(Popup::Save { name }), Some(PopupLayout::Save(panel))) => {
                let panel = *panel;
                draw_panel(d, panel);
                d.draw_text(&text().get(keys::EDITOR_SAVE_AS), (panel.x + 12.0) as i32, (panel.y + 10.0) as i32, 16, TEXT);
                d.draw_text(&format!("{name}_"), (panel.x + 12.0) as i32, (panel.y + 34.0) as i32, 18, TEXT);
                let hint = text().get(hints.pick(keys::EDITOR_SAVE_HINT, keys::EDITOR_SAVE_HINT_TOUCH));
                d.draw_text(&hint, (panel.x + 12.0) as i32, (panel.y + 58.0) as i32, UI_SMALL_TEXT, Color::GRAY);
            }
            _ => {}
        }
    }

    /// The FILE menu below its button.
    fn draw_file_menu(d: &mut impl RaylibDraw, rows: &chrome::Rows) {
        draw_panel(d, rows.panel);
        for (i, row) in FileRow::all().iter().enumerate() {
            let rect = rows.row(i);
            d.draw_text(&row.label(), rect.x as i32 + 16, (rect.y + (rect.height - HUD_TEXT_SIZE as f32) / 2.0) as i32, HUD_TEXT_SIZE, TEXT);
        }
    }

    /// The Load list: one row per map, shipped ones marked, and the pager
    /// when there are more than fit.
    fn draw_load_list(d: &mut impl RaylibDraw, load: &chrome::LoadLayout, entries: &[MapEntry], scroll: usize, hints: Hints) {
        let panel = load.rows.panel;
        draw_panel(d, panel);
        if entries.is_empty() {
            d.draw_text(&text().get(keys::EDITOR_NO_MAPS), panel.x as i32 + 16, panel.y as i32 + 15, HUD_TEXT_SIZE, DIM);
            return;
        }
        let rows = load.per_page;
        let last = entries.len().saturating_sub(rows);
        let scroll = scroll.min(last);
        for (i, entry) in entries.iter().skip(scroll).take(rows).enumerate() {
            let row = load.rows.row(i);
            let text_y = (row.y + (row.height - HUD_TEXT_SIZE as f32) / 2.0) as i32;
            d.draw_text(&fit_text(&entry.name, chrome::LOAD_PANEL_W - 120.0, HUD_TEXT_SIZE), row.x as i32 + 16, text_y, HUD_TEXT_SIZE, TEXT);
            if !entry.on_disk {
                d.draw_text(&text().get(keys::EDITOR_SHIPPED), (row.x + row.width - 80.0) as i32, text_y, HUD_TEXT_SIZE, DIM);
            }
        }
        if let Some(pager) = load.pager {
            let hint = text().fmt(
                page_key(hints),
                &[("from", (scroll + 1).into()), ("to", (scroll + rows).min(entries.len()).into()), ("n", entries.len().into())],
            );
            draw_pager(d, pager.row, &hint, scroll > 0, scroll < last);
        }
    }

    /// A category's tool list below its button: one row per tool, the
    /// current one highlighted.
    fn draw_dropdown(&self, d: &mut impl RaylibDraw, rows: &chrome::Rows, textures: &EditorTextures, category: Category) {
        draw_panel(d, rows.panel);
        for (i, tool) in category.tools().enumerate() {
            let row = rows.row(i);
            if tool == self.current_tool(category) {
                let inset = Rectangle::new(row.x + 4.0, row.y + 2.0, row.width - 8.0, row.height - 4.0);
                d.draw_rectangle_rounded(inset, 0.2, EDITOR_PANEL_SEGMENTS, Color::new(255, 255, 255, 40));
            }
            // A 40 pt rect: the icon's own 4 pt inset makes it 32 at 8.
            let icon = Rectangle::new(row.x + 4.0, row.y + 4.0, ICON_PX + 8.0, ICON_PX + 8.0);
            draw_tool_icon(d, textures, self.map.theme, tool, icon);
            if self.singleton_placed(tool) {
                draw_badge(d, icon.x + icon.width - 4.0, icon.y + 4.0);
            }
            let text_y = row.y as i32 + (EDITOR_DROPDOWN_ROW_H as i32 - HUD_TEXT_SIZE) / 2;
            d.draw_text(&label(tool), row.x as i32 + DROPDOWN_TEXT_X, text_y, HUD_TEXT_SIZE, TEXT);
        }
    }

    /// The palette the folded TOOLS button opens: a row per category, its
    /// name in the accent while the brush is one of its tools, then a cell
    /// per tool - the category's current tool lit, the brush outlined.
    fn draw_palette(&self, d: &mut impl RaylibDraw, palette: &chrome::Palette, textures: &EditorTextures) {
        draw_panel(d, palette.panel);
        for category in Category::ALL {
            let label = palette.label(category);
            let color = if self.active_category() == Some(category) { BUILD_ACCENT } else { DIM };
            let y = (label.y + (label.height - UI_SMALL_TEXT as f32) / 2.0) as i32;
            d.draw_text(&category.label(), label.x as i32 + 8, y, UI_SMALL_TEXT, color);
            for (i, tool) in category.tools().enumerate() {
                let cell = palette.cell(category, i);
                if tool == self.current_tool(category) {
                    let inset = Rectangle::new(cell.x + 2.0, cell.y + 2.0, cell.width - 4.0, cell.height - 4.0);
                    d.draw_rectangle_rounded(inset, 0.2, EDITOR_PANEL_SEGMENTS, Color::new(255, 255, 255, 40));
                }
                let icon = Rectangle::new(cell.x + 4.0, cell.y + 4.0, ICON_PX + 8.0, ICON_PX + 8.0);
                draw_tool_icon(d, textures, self.map.theme, tool, icon);
                if self.singleton_placed(tool) {
                    draw_badge(d, icon.x + icon.width - 4.0, icon.y + 4.0);
                }
                if tool == self.active_tool {
                    d.draw_rectangle_lines_ex(Rectangle::new(cell.x + 2.0, cell.y + 2.0, cell.width - 4.0, cell.height - 4.0), 2.0, BUILD_ACCENT);
                }
            }
        }
    }

    /// The MAP settings panel (docs/game-editor-fusion.md section 9): a
    /// stepper per map key and the RESET MAP button, `page`'s rows where
    /// the room under the bar pages it, and then its pager.
    fn draw_settings(&self, d: &mut impl RaylibDraw, layout: &chrome::SettingsLayout, page: usize, textures: &EditorTextures, hints: Hints) {
        draw_hanging_panel(d, layout.rows.panel);
        let page = page.min(layout.pages - 1);
        let settings = self.settings();
        let waves_off = settings.spawn == SpawnKind::Band;
        for (i, row) in SETTINGS_ROWS.iter().enumerate() {
            let Some(rect) = layout.row(i, page) else { continue };
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
                d.draw_text(&text().get(keys::SETTINGS_CLI), x, text_y + 4, UI_SMALL_TEXT, DIM);
            }
        }
        if let Some(pager) = layout.pager {
            let from = page * layout.per_page + 1;
            let to = (from + layout.per_page - 1).min(SETTINGS_ROWS.len());
            let hint = text().fmt(page_key(hints), &[("from", from.into()), ("to", to.into()), ("n", SETTINGS_ROWS.len().into())]);
            draw_pager(d, pager.row, &hint, page > 0, page + 1 < layout.pages);
        }
    }

    /// The CHECK panel (docs/large-maps-patterns.md, "Lint panel with
    /// jump-to and fixes"): a header with the panel's title, a line on how
    /// it works and each severity's count beside its mark; then a page of
    /// findings, each its mark, its words (`lint-<kind>`), where it is
    /// and, where it has one, its FIX button; the pager past a page.
    fn draw_lint_panel(&self, d: &mut impl RaylibDraw, layout: &chrome::LintLayout, page: usize, hints: Hints) {
        let t = text();
        let len = self.lint_len();
        draw_hanging_panel(d, layout.panel);
        let header = layout.row(0);
        let left = (header.x + LINT_TEXT_INSET) as i32;
        d.draw_text(&t.get(keys::CHECK_TITLE), left, (header.y + 8.0) as i32, LINT_TITLE_SIZE, TEXT);
        d.draw_text(&fit_text(&t.get(keys::CHECK_HINT), LINT_HINT_W, UI_SMALL_TEXT), left, (header.y + 29.0) as i32, UI_SMALL_TEXT, DIM);
        self.draw_clear_row(d, layout.row(1));
        let Some(report) = &self.lint else { return };
        // The counts at the header's right end, errors first.
        let mut x = header.x + header.width - LINT_TEXT_INSET;
        for severity in [LintSeverity::Info, LintSeverity::Warning, LintSeverity::Error] {
            let n = report.count(severity);
            if n == 0 {
                continue;
            }
            let count = n.to_string();
            x -= text_width(&count, LINT_TITLE_SIZE);
            d.draw_text(&count, x as i32, (header.y + 8.0) as i32, LINT_TITLE_SIZE, TEXT);
            x -= LINT_MARK + 4.0;
            draw_severity_mark(d, x, header.y + 10.0, severity);
            x -= LINT_COUNT_GAP;
        }
        if report.findings.is_empty() {
            let row = layout.row(LINT_HEAD_ROWS);
            let y = (row.y + (row.height - LINT_TITLE_SIZE as f32) / 2.0) as i32;
            d.draw_text(&t.get(keys::CHECK_NONE), left, y, LINT_TITLE_SIZE, LINT_CLEAN);
            return;
        }
        let page = page.min(layout.pages - 1);
        let per_page = layout.per_page;
        for (slot, (index, finding)) in report.findings.iter().enumerate().skip(page * per_page).take(per_page).enumerate() {
            let row = layout.row(LINT_HEAD_ROWS + slot);
            if self.lint_marked == Some(index) {
                let inset = Rectangle::new(row.x + 4.0, row.y + 2.0, row.width - 8.0, row.height - 4.0);
                d.draw_rectangle_rounded(inset, 0.2, EDITOR_PANEL_SEGMENTS, Color::new(255, 255, 255, 40));
            }
            draw_severity_mark(d, row.x + LINT_TEXT_INSET, row.y + 18.0, finding.severity);
            let words = fit_text(&t.named("lint", finding.kind.tag()), LINT_FINDING_W, LINT_TITLE_SIZE);
            let text_x = (row.x + LINT_TEXT_INSET + LINT_MARK + 8.0) as i32;
            d.draw_text(&words, text_x, (row.y + 8.0) as i32, LINT_TITLE_SIZE, TEXT);
            d.draw_text(&place_text(&finding.cells), text_x, (row.y + 29.0) as i32, UI_SMALL_TEXT, DIM);
            if finding.fix.is_some() {
                let fix = chrome::LintLayout::fix(row);
                d.draw_rectangle_rounded_lines_ex(fix, 0.2, EDITOR_PANEL_SEGMENTS, 1.0, Color::new(255, 255, 255, 60));
                let label = t.get(keys::CHECK_FIX);
                let w = text_width(&label, LINT_TITLE_SIZE);
                let y = (fix.y + (fix.height - LINT_TITLE_SIZE as f32) / 2.0) as i32;
                d.draw_text(&label, (fix.x + (fix.width - w) / 2.0) as i32, y, LINT_TITLE_SIZE, BUILD_ACCENT);
            }
        }
        if let Some(pager) = layout.pager {
            let from = page * per_page + 1;
            let to = (from + per_page - 1).min(len);
            let hint = t.fmt(page_key(hints), &[("from", from.into()), ("to", to.into()), ("n", len.into())]);
            draw_pager(d, pager.row, &hint, page > 0, page + 1 < layout.pages);
        }
    }

    /// The CHECK panel's clear check row: the flag, whether this revision
    /// of the canvas is cleared and, when it is, its par at the row's
    /// right end; under them what clearing means; a faint rule under the
    /// row, above the findings.
    fn draw_clear_row(&self, d: &mut impl RaylibDraw, row: Rectangle) {
        let t = text();
        let par = self.par();
        draw_flag(d, row.x + LINT_TEXT_INSET, row.y + 9.0, par.is_some());
        let words_x = (row.x + LINT_TEXT_INSET + LINT_MARK + 8.0) as i32;
        let (title, color, hint) = match par {
            Some(_) => (keys::CHECK_CLEARED, LINT_CLEAN, keys::CHECK_CLEARED_HINT),
            None => (keys::CHECK_NOT_CLEARED, LINT_WARNING, keys::CHECK_NOT_CLEARED_HINT),
        };
        d.draw_text(&t.get(title), words_x, (row.y + 8.0) as i32, LINT_TITLE_SIZE, color);
        if let Some(par) = par {
            let line = t.fmt(keys::CHECK_PAR, &[("time", crate::hud::clock_text(par as f32).into())]);
            let x = row.x + row.width - LINT_TEXT_INSET - text_width(&line, LINT_TITLE_SIZE);
            d.draw_text(&line, x as i32, (row.y + 8.0) as i32, LINT_TITLE_SIZE, TEXT);
        }
        d.draw_text(&fit_text(&t.get(hint), LINT_CLEAR_W, UI_SMALL_TEXT), words_x, (row.y + 29.0) as i32, UI_SMALL_TEXT, DIM);
        d.draw_rectangle((row.x + 8.0) as i32, (row.y + row.height - 2.0) as i32, (row.width - 16.0) as i32, 2, Color::new(255, 255, 255, 30));
    }

    /// The finding the CHECK panel picked, on the canvas in world pixels:
    /// each of its cells filled faintly and outlined in its severity's
    /// colour.
    fn draw_lint_marks<D: RaylibDraw>(&self, d: &mut D, culled: impl Fn(Position, f32) -> bool) {
        let Some(finding) = self.lint_marked() else { return };
        let color = severity_color(finding.severity);
        let fill = Color::new(color.r, color.g, color.b, 70);
        for cell in &finding.cells {
            let r = cell.rect();
            if culled(Position::new(r.x + r.width / 2.0, r.y + r.height / 2.0), r.width) {
                continue;
            }
            d.draw_rectangle_rec(r, fill);
            d.draw_rectangle_lines_ex(r, 2.0, color);
        }
    }
}

/// The frame round the loupe's square, opaque so the world's colours stop
/// at its edge.
const LOUPE_FRAME: Color = Color::new(12, 12, 16, 255);

/// The loupe (`MapEditor::loupe`) in UI points: `picture` holds its
/// world from its corner at a texel a world pixel (`draw_loupe_world`),
/// put on its square at the loupe's own whole-block scale, sampled
/// nearest (a render texture's filter), on a plate of its own; the cell
/// the stroke paints is outlined in the stroke's colour, clipped to the
/// square.
fn draw_loupe(d: &mut impl RaylibDraw, loupe: &Loupe, picture: &RenderTexture2D) {
    crate::render::hud::draw_plate(d, Corners::plate(loupe.rect), crate::render::hud::PLATE_EDGE, 1.0);
    let w = loupe.world;
    let height = picture.texture.height as f32;
    // A render texture reads back bottom-up.
    let texels = Rectangle::new(0.0, height - w.height, w.width, -w.height);
    d.draw_texture_pro(picture, texels, loupe.rect, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
    let k = loupe.rect.width / w.width;
    let cell = LintCell::Map(loupe.cell.0, loupe.cell.1).rect();
    let (left, top) = (loupe.rect.x + (cell.x - w.x) * k, loupe.rect.y + (cell.y - w.y) * k);
    let (right, bottom) = (left + cell.width * k, top + cell.height * k);
    let r = loupe.rect;
    let (left, top, right, bottom) = (left.max(r.x), top.max(r.y), right.min(r.x + r.width), bottom.min(r.y + r.height));
    if right > left && bottom > top {
        let color = if loupe.erase { LINT_ERROR } else { BUILD_ACCENT };
        d.draw_rectangle_lines_ex(Rectangle::new(left, top, right - left, bottom - top), 2.0, color);
    }
    d.draw_rectangle_lines_ex(loupe.rect, 2.0, LOUPE_FRAME);
}

/// The CHECK panel's text: its header and its findings' words in 16 px,
/// inset from the panel's left; a severity's mark is a 12 px square of
/// whole blocks. A finding's words run `LINT_FINDING_W` from beside it.
const LINT_TITLE_SIZE: i32 = 16;
/// The gap between two counts in the header.
const LINT_COUNT_GAP: f32 = 12.0;
const LINT_ERROR: Color = Color::new(232, 72, 64, 255);
const LINT_WARNING: Color = Color::new(255, 176, 48, 255);
const LINT_INFO: Color = Color::new(120, 170, 230, 255);
/// The line a clean map's panel says it with.
const LINT_CLEAN: Color = Color::new(120, 220, 90, 255);

fn severity_color(severity: LintSeverity) -> Color {
    match severity {
        LintSeverity::Error => LINT_ERROR,
        LintSeverity::Warning => LINT_WARNING,
        LintSeverity::Info => LINT_INFO,
    }
}

/// A severity's mark: a square of 2 px blocks in its colour, with a dark
/// block in the middle so it reads on the panel at any size.
fn draw_severity_mark(d: &mut impl RaylibDraw, x: f32, y: f32, severity: LintSeverity) {
    d.draw_rectangle(x as i32, y as i32, LINT_MARK as i32, LINT_MARK as i32, severity_color(severity));
    d.draw_rectangle(x as i32 + 4, y as i32 + 4, 4, 4, Color::new(0, 0, 0, 120));
}

/// The clear check's flag: a pole and a chequered finish flag of whole
/// 2 px blocks, 10 x 14 from (`x`, `y`) - green once the canvas's
/// revision is cleared, dim while it is not.
fn draw_flag(d: &mut impl RaylibDraw, x: f32, y: f32, cleared: bool) {
    let color = if cleared { LINT_CLEAN } else { DIM };
    let shade = Color::new(color.r, color.g, color.b, 80);
    let (x, y) = (x as i32, y as i32);
    d.draw_rectangle(x, y, 2, 14, color);
    for row in 0..3 {
        for col in 0..4 {
            let block = if (row + col) % 2 == 0 { color } else { shade };
            d.draw_rectangle(x + 2 + col * 2, y + row * 2, 2, 2, block);
        }
    }
}

/// The clear check's readout in the bar (`Bar::clear`): the flag and, once
/// the canvas's revision is cleared, its par beside it in `size` - numbers
/// only, so no language has to fit there.
fn draw_clear_readout(d: &mut impl RaylibDraw, rect: Rectangle, par: Option<f64>, size: i32) {
    let y = rect.y + ((rect.height - 14.0) / 2.0).floor();
    draw_flag(d, rect.x + 2.0, y, par.is_some());
    if let Some(par) = par {
        let label = crate::hud::clock_text(par as f32);
        let text_y = (rect.y + (rect.height - size as f32) / 2.0) as i32;
        d.draw_text(&label, (rect.x + 16.0) as i32, text_y, size, TEXT);
    }
}

/// A pager's hint for `hints`' input: the span on screen and the wheel
/// that turns it, or a tap on its arrows.
fn page_key(hints: Hints) -> crate::text::Key {
    hints.pick(keys::EDITOR_PAGE, keys::EDITOR_PAGE_TOUCH)
}

/// A pager row (`chrome::Pager`): `<` at its left end and `>` at its right,
/// each dim where there is no page that way, and `hint` - the span on
/// screen - between them.
fn draw_pager(d: &mut impl RaylibDraw, row: Rectangle, hint: &str, back: bool, next: bool) {
    let arrow_y = (row.y + (row.height - HUD_TEXT_SIZE as f32) / 2.0) as i32;
    d.draw_text("<", row.x as i32 + 16, arrow_y, HUD_TEXT_SIZE, if back { TEXT } else { DIM });
    let right = row.x + row.width - 16.0 - text_width(">", HUD_TEXT_SIZE);
    d.draw_text(">", right as i32, arrow_y, HUD_TEXT_SIZE, if next { TEXT } else { DIM });
    let hint_x = row.x + (row.width - text_width(hint, UI_SMALL_TEXT)) / 2.0;
    d.draw_text(hint, hint_x as i32, (row.y + (row.height - UI_SMALL_TEXT as f32) / 2.0) as i32, UI_SMALL_TEXT, DIM);
}

/// Where a bar icon is drawn in `zone`, a button or a category button's
/// icon half: with a mouse the 32 pt icon box the bar has always had, `x`
/// in from the zone's left; on a touch screen a 40 pt one centred in it,
/// so the sprite is drawn its own 32 pt (`draw_tool_icon` insets by 4).
fn icon_rect(zone: Rectangle, touch: bool, x: f32) -> Rectangle {
    let side = if touch { ICON_PX + 8.0 } else { ICON_PX };
    let left = if touch { zone.x + (zone.width - side) / 2.0 } else { zone.x + x };
    Rectangle::new(left, zone.y + ((zone.height - side) / 2.0).floor(), side, side)
}

/// Where a finding is, as its row says it: the map cell under its first
/// cell's middle - what the cursor readout would name there - and how
/// many more it covers. Numbers only, so no language has to say it.
fn place_text(cells: &[LintCell]) -> String {
    let Some(first) = cells.first() else { return String::new() };
    let r = first.rect();
    let (col, row) = map::world_to_cell(Position::new(r.x + r.width / 2.0, r.y + r.height / 2.0));
    match cells.len() {
        1 => format!("{col},{row}"),
        n => format!("{col},{row} +{}", n - 1),
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

/// FILE / MAP: an outlined button with its label and a caret, the label
/// in the accent while its menu is open.
fn draw_menu_button(d: &mut impl RaylibDraw, rect: Rectangle, label: &str, open: bool) {
    let color = if open { BUILD_ACCENT } else { TEXT };
    d.draw_rectangle_rounded_lines_ex(
        Rectangle::new(rect.x, rect.y + 4.0, rect.width - MENU_BOX_INSET, rect.height - 8.0),
        0.2,
        EDITOR_PANEL_SEGMENTS,
        1.0,
        Color::new(255, 255, 255, 60),
    );
    let text_y = (rect.y + (rect.height - HUD_TEXT_SIZE as f32) / 2.0) as i32;
    d.draw_text(label, rect.x as i32 + 6, text_y, HUD_TEXT_SIZE, color);
    draw_caret(d, rect.x as i32 + 48, (rect.y + (rect.height - 6.0) / 2.0) as i32, color);
}

/// An outlined bar button with its label centred in it in `size`: UNDO,
/// REDO, FIT, CHECK, PLAY HERE (`chrome::small_text`).
fn draw_small_button(d: &mut impl RaylibDraw, rect: Rectangle, text: &str, color: Color, size: i32) {
    let inset = Rectangle::new(rect.x, rect.y + 4.0, rect.width - SMALL_BOX_INSET, rect.height - 8.0);
    d.draw_rectangle_rounded_lines_ex(inset, 0.2, EDITOR_PANEL_SEGMENTS, 1.0, Color::new(255, 255, 255, 60));
    let w = text_width(text, size);
    d.draw_text(text, (inset.x + (inset.width - w) / 2.0) as i32, (rect.y + (rect.height - size as f32) / 2.0) as i32, size, color);
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
    use crate::EDITOR_SETTINGS_W;

    /// A category button's icon and caret, with a mouse and on a touch
    /// screen: the icon inside the icon half, the caret clear of it and
    /// inside the button's drawn box, at the 34 pt a desktop's bar has
    /// always drawn it at with a mouse.
    #[test]
    fn a_category_buttons_icon_and_caret_stay_in_their_halves() {
        for touch in [false, true] {
            let ui = crate::hud::UiFrame::new((1600.0, 900.0), 1.0, 1.0, crate::hud::Insets::default(), touch);
            let bar = Bar::of(&ui);
            let button = bar.category(Category::Wall).expect("a wide bar has the five");
            let icon = icon_rect(button.icon, touch, 0.0);
            let caret_x = button.rect.x + button.rect.width - BUTTON_GAP - CARET_W as f32;
            assert!(icon.x >= button.icon.x && icon.x + icon.width <= button.icon.x + button.icon.width, "touch={touch}: {icon:?}");
            assert!(icon.y >= button.rect.y && icon.y + icon.height <= button.rect.y + button.rect.height, "touch={touch}: {icon:?}");
            assert!(caret_x >= icon.x + icon.width, "touch={touch}: the caret overlaps the icon");
            assert!(caret_x + CARET_W as f32 <= button.rect.x + button.rect.width - BUTTON_GAP, "touch={touch}: the caret leaves the button");
            if !touch {
                assert_eq!(caret_x - button.rect.x, 34.0);
                assert_eq!(icon, Rectangle::new(button.rect.x, button.rect.y, ICON_PX, ICON_PX));
            }
        }
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
