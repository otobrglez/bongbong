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
use super::chrome::{
    button_box, caret_at, icon_box, BarTools, BAR_SMALL_TEXT, BOX_EDGE, BOX_PAD, CARET_GAP, CategoryButton, STAMP_NAME_W,
    STAMP_PICTURE,
};
use crate::text::{keys, text};
use crate::canvas::Sheet;
use crate::frog::FrogAnim;
use crate::hud::{Hints, BAR_FILL, DIM, HUD_TEXT_SIZE, TEXT, UI_SMALL_TEXT};
use crate::math::{Color, Rectangle};
use crate::obstacle;
use crate::portal::{draw_portal, portal_icon_source_rec};
use crate::render::canvas::{BlockTexture, GpuCanvas, Sheets};

/// The icons in the dropdown rows and the palette, the sheets' own 32 px
/// drawn a point a pixel.
const ICON_PX: f32 = 32.0;
/// A dropdown row's name column, right of its 32 pt icon at 8 pt inset.
const DROPDOWN_TEXT_X: i32 = 48;
use super::chrome::{MAP_CHECK, MAP_HEADING_SIZE, MAP_LABEL_SIZE, MAP_SKY_SIZE, MAP_SWATCH, MAP_VALUE_SIZE};

/// What the canvas area shows where the map is not: past its edges when
/// the whole of a field map is shown, and under the field before the
/// ground is drawn.
const CANVAS_FILL: Color = Color::new(30, 30, 34, 255);

/// The width the default font gives `text` at `size`: `text::width`,
/// which is `MeasureText`'s answer with no handle.
fn text_width(text: &str, size: i32) -> f32 {
    crate::text::width(text, size) as f32
}

/// `text` cut down to what fits in `width` at `size`, the cut marked `~`.
fn fit_text(text: &str, width: f32, size: i32) -> String {
    crate::text::fit(text, width as i32, size).into_owned()
}

/// `text` cut from its start until it fits `width` at `size`: the end of
/// a name being typed, where the cursor is, stays in view.
fn tail_fit(text: &str, width: f32, size: i32) -> String {
    let mut tail = text;
    while text_width(tail, size) > width {
        let mut chars = tail.chars();
        if chars.next().is_none() {
            break;
        }
        tail = chars.as_str();
    }
    tail.to_string()
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
    /// static/crates_sheet.png - the pickups' crates, a row per kind.
    pub crates: &'a Texture2D,
    /// static/pickup_glyphs.png - the crates' symbols on their own: a
    /// pickup tool's icon where its crate does not fit.
    pub pickup_glyphs: &'a Texture2D,
    pub eraser: &'a Texture2D,
    /// static/portal_sheet.png - the spiral and its bar icon (portal.rs).
    pub portal: &'a Texture2D,
    pub tanks: &'a Texture2D,
    pub trees: &'a Texture2D,
    /// static/bushes_sheet.png - bushes and reeds (docs/BUSHES_SPEC.md).
    pub bushes: &'a Texture2D,
    /// The canvas map's theme: a desert map's trees are drawn dry
    /// (`Material::is_dry`), as the round will draw them.
    pub theme: Theme,
    /// static/target_sheet.png - the range board.
    pub target: &'a Texture2D,
    /// static/towers_sheet.png - the defence towers' bases and tops.
    pub towers: &'a Texture2D,
    /// The canvas's floor shade as `app.rs` uploaded it before the frame,
    /// with the stamp it was baked under.
    pub shade: Option<(u64, &'a Texture2D)>,
    /// The navigator's picture (`MapEditor::minimap`) as `app.rs` uploaded
    /// it before the frame, with the stamp it was baked under.
    pub minimap: Option<(u64, &'a Texture2D)>,
    /// The Load list's thumbnails as `app.rs` uploaded them before the
    /// frame (`ThumbnailTextures::sync`).
    pub thumbnails: Option<&'a ThumbnailTextures>,
}

/// The Load list's thumbnails on the GPU (`MapEditor::thumbnails`): a
/// `BlockTexture` each, uploaded once per picture and dropped - its
/// texture freed - once its picture is no longer shown, the list's close
/// among them. `app.rs` keeps one and syncs it before the builder draws.
#[derive(Default)]
pub struct ThumbnailTextures {
    held: std::collections::BTreeMap<String, BlockTexture>,
}

impl ThumbnailTextures {
    /// Hold a texture for every thumbnail `editor` shows, uploading a
    /// picture only when it is new, and let the others go.
    pub fn sync(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, editor: &MapEditor) {
        let shown: Vec<(&str, &crate::canvas::BlockImage)> = editor.thumbnails().filter_map(|(name, thumb)| Some((name, thumb.image.as_ref()?))).collect();
        self.held.retain(|name, _| shown.iter().any(|(n, _)| *n == name.as_str()));
        for (name, image) in shown {
            self.held.entry(name.to_string()).or_default().sync(rl, thread, image);
        }
    }

    /// `name`'s thumbnail texture, with the stamp of the picture it shows.
    pub fn get(&self, name: &str) -> Option<(u64, &Texture2D)> {
        self.held.get(name)?.held()
    }

    /// How many textures are held.
    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }
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
/// round draws from (damage, blasts, the frog's other clips) is a
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
            Sheet::Bushes => self.bushes,
            Sheet::Target => self.target,
            Sheet::Towers => self.towers,
            Sheet::Grass(_) => self.grass,
            Sheet::Portal => self.portal,
            Sheet::Tanks => self.tanks,
            Sheet::Crates => self.crates,
            Sheet::PickupGlyphs => self.pickup_glyphs,
            Sheet::Frog { clip: FrogAnim::Idle, .. } => self.frog_idle,
            Sheet::TankGlow
            | Sheet::TankModules
            | Sheet::TankModulesGlow
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
        question: Option<&crate::mapstore::Question>,
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
        // The chrome's camera goes straight onto the window.
        let units = crate::render::view::window_camera_units(rl);
        if camera == crate::view::Camera::whole(field) && (layout.field.w, layout.field.h) == field {
            // The canvas into the bitmap, the bitmap onto the window.
            rl.draw_texture_mode(thread, composite, |mut d| {
                d.clear_background(CANVAS_FILL);
                self.draw_canvas(&mut d, textures, time, cursor, None);
            });
            let magnified = self.loupe(frame).and_then(|loupe| Some((loupe, self.draw_loupe_world(rl, thread, &mut scene.loupe, &loupe, textures, time)?)));
            rl.draw(thread, |mut d| {
                crate::render::view::present_into(&mut d, composite, view, backdrop, None);
                self.draw_window_chrome(&mut d, frame, &chrome, textures, cursor, &camera, magnified.as_ref().map(|(l, p)| (l, *p)), units, question);
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
            self.draw_window_chrome(&mut d, frame, &chrome, textures, cursor, &camera, magnified.as_ref().map(|(l, p)| (l, *p)), units, question);
        });
    }

    /// The chrome over the canvas on the window, in UI points through
    /// `frame.ui`'s scale - carried in framebuffer pixels, `units` to the
    /// window unit (`render::view::window_camera_units`): the status line
    /// and the navigator, the bar, the open popup and, over a painting
    /// finger, the loupe.
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
        units: f32,
        question: Option<&crate::mapstore::Question>,
    ) {
        let ui = crate::render::view::onto_window(
            Camera2D { offset: Vector2::new(0.0, 0.0), target: Vector2::new(0.0, 0.0), rotation: 0.0, zoom: frame.ui.scale },
            units,
        );
        d.draw_mode2D(ui, |mut d, _| {
            self.draw_status_line(&mut d, frame, chrome, cursor);
            self.draw_navigator(&mut d, frame, textures, camera);
            self.draw_bar(&mut d, &chrome.bar, textures);
            if let Some(strip) = &chrome.strip {
                self.draw_strip(&mut d, strip, chrome.bar.touch);
            }
            self.draw_popup(&mut d, chrome, textures, frame.ui.hints);
            if let Some((loupe, picture)) = magnified {
                draw_loupe(&mut d, loupe, picture);
            }
            // A question about the canvas's map (BB-33), over everything.
            if let Some(question) = question {
                crate::render::hud::draw_question(&mut d, frame.ui.screen, frame.ui.area, question);
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
        // The size of the rectangle a RECT drag is drawing, else of the
        // paste ghost while one stands, else of the selection.
        if let Some(rect) = self.rect_stroke().map(|(r, _)| r).or(self.ghost().map(|g| g.rect())).or(self.selection()) {
            line.push_str(&format!("   {}x{}", rect.cols, rect.rows));
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

            // The cells as the kept index holds them, made once per edit
            // rather than parsed and sorted again every frame.
            let index = self.cell_index();
            // Portals first, under every other cell: three cells of art on
            // one anchor cell, and the cells are row-sorted, so a portal
            // drawn in the loop would cover a wall placed above it. Ghosted
            // while the network is inactive (fewer than two), with the
            // anchor outlined like a gate off its edge - placed, not
            // usable. `time` is the wall clock above.
            let portals = index.portals();
            let active = portals.len() >= 2;
            let tint = if active { Color::WHITE } else { Color::new(255, 255, 255, 110) };
            for &(col, row) in portals {
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

            // The lava over the ground, from the canvas's own layout, then
            // each volcano's cone at its crater (docs/volcano.md): a round's
            // picture, asleep and calm, under every other cell.
            let lava = self.lava_layout();
            if !lava.is_empty() {
                let look = crate::lava::Look { surge: 0.0, speed: crate::tuning::tuning().lava_flow_speed };
                let mut c = GpuCanvas::new(&mut *d, textures);
                for (col, row) in lava.cells() {
                    if !culled(map::cell_to_world(col, row), 0.0) {
                        crate::lava::draw_cell(&mut c, &lava, col, row, time, look, false, 1.0);
                        crate::lava::draw_floes(&mut c, &lava, col, row, time, look);
                    }
                }
            }
            let t = crate::tuning::tuning();
            for &(col, row) in &self.map.volcano_cells() {
                let pos = map::cell_to_world(col, row);
                if culled(pos, 96.0) {
                    continue;
                }
                let outlets: Vec<f32> = lava
                    .cells()
                    .filter(|&(c, r)| (-1..=1).any(|dr| (-1..=1).any(|dc| crate::volcano::in_footprint(col, row, c + dc, r + dr))))
                    .map(|(c, r)| ((r - row) as f32).atan2((c - col) as f32))
                    .collect();
                let picture = crate::volcano::cone(&outlets);
                let asleep = crate::volcano::phase(-1.0, 0.0, &t);
                let mut c = GpuCanvas::new(&mut *d, textures);
                crate::volcano::draw_skirt(&mut c, pos, &picture);
                crate::volcano::draw_cone_shadow(&mut c, pos, &picture, (t.shadow_dir_x, t.shadow_dir_y));
                crate::volcano::draw_cone(&mut c, pos, &picture, &asleep, time);
            }

            // Only the rows the cull spans (`CellIndex::near`).
            for (col, row, obj) in index.near(cull) {
                // Road and water are painted into `self.ground`, lava,
                // volcanoes and portals in the pre-passes above.
                if matches!(obj, CellObject::Road | CellObject::Water | CellObject::Lava | CellObject::Volcano | CellObject::Portal) {
                    continue;
                }
                let pos = map::cell_to_world(*col, *row);
                // A tower's reach ring spreads far past its cell.
                let reach = obj.tower().map_or(0.0, |(kind, _)| kind.range());
                if culled(pos, reach) {
                    continue;
                }
                draw_cell(d, textures, (width, height), pos, obj, 255, time);
            }

            // The select tool's marks over the map, then what the brush's
            // shape is about to do.
            self.draw_selection(d, textures, time, &culled);
            self.draw_brush_marks(d, textures, time, cursor, &culled);

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
    /// and PLAY - every button in one box (`chrome::button_box`) and by one
    /// rule of colours (`Face`), its words at one size.
    fn draw_bar(&self, d: &mut impl RaylibDraw, bar: &Bar, textures: &EditorTextures) {
        d.draw_rectangle_rec(bar.strip, BAR_FILL);
        let touch = bar.touch;
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
                    self.draw_category_button(d, button, textures, category, touch);
                }
            }
            BarTools::Folded(r) => self.draw_tools_button(d, *r, textures, touch),
        }
        if let Some(r) = bar.brush {
            self.draw_brush_button(d, r, touch);
        }

        let erase = button_box(bar.erase, touch);
        draw_box(d, erase, Face::IDLE.in_force(self.active_tool == Tool::Eraser));
        draw_tool_icon(d, textures, self.map.theme, Tool::Eraser, icon_box(erase, false));

        let undo = Face::able(self.history.undo_depth() > 0);
        draw_word_button(d, bar.undo, touch, &text().get(keys::EDITOR_UNDO), undo);
        let redo = Face::able(self.history.redo_depth() > 0);
        draw_word_button(d, bar.redo, touch, &text().get(keys::EDITOR_REDO), redo);

        let file_open = matches!(self.popup, Some(Popup::File | Popup::Load { .. } | Popup::Save { .. }));
        draw_menu_button(d, bar.file, touch, &text().get(keys::EDITOR_FILE), Face::IDLE.open(file_open));
        let map_open = matches!(self.popup, Some(Popup::Settings { .. }));
        draw_menu_button(d, bar.map, touch, &text().get(keys::EDITOR_MAP), Face::IDLE.open(map_open));
        // FIT, dim while the whole canvas is what is shown.
        draw_word_button(d, bar.fit, touch, &text().get(keys::EDITOR_FIT), Face::able(!self.camera.is_fit()));
        let check_open = matches!(self.popup, Some(Popup::Lint { .. }));
        draw_word_button(d, bar.check, touch, &text().get(keys::EDITOR_CHECK), Face::IDLE.open(check_open));
        draw_clear_readout(d, bar.clear, self.par(), BAR_SMALL_TEXT);

        draw_word_button(d, bar.here, touch, &text().get(keys::EDITOR_PLAY_HERE), Face::ACTION);
        draw_word_button(d, bar.play, touch, &text().get(keys::BUTTON_PLAY), Face::ACTION);
    }

    /// One category button: its box, the current tool's icon at its left
    /// end and a caret at its right, outlined in the amber while the
    /// active brush is one of its tools and washed while its list is up.
    /// The tool's name is in the status line.
    fn draw_category_button(&self, d: &mut impl RaylibDraw, button: &CategoryButton, textures: &EditorTextures, category: Category, touch: bool) {
        let open = matches!(self.popup, Some(Popup::Dropdown(c)) if c == category);
        let face = Face::IDLE.open(open).in_force(self.active_category() == Some(category));
        let icon = draw_menu_box(d, button.rect, touch, face);
        let tool = self.current_tool(category);
        draw_tool_icon(d, textures, self.map.theme, tool, icon);
        if self.singleton_placed(tool) {
            draw_badge(d, icon.x + icon.width, icon.y);
        }
    }

    /// The TOOLS button the categories and BRUSH fold into: the brush's
    /// icon - the select tool's while it is the brush, or while the eraser
    /// is the brush the tool it came from - and a caret, outlined in the
    /// amber while a tool from the palette is the brush.
    fn draw_tools_button(&self, d: &mut impl RaylibDraw, rect: Rectangle, textures: &EditorTextures, touch: bool) {
        let selecting = self.active_tool == Tool::Select;
        let open = matches!(self.popup, Some(Popup::Palette));
        let face = Face::IDLE.open(open).in_force(self.active_category().is_some() || selecting);
        let icon = draw_menu_box(d, rect, touch, face);
        if selecting {
            draw_brush_icon(d, BrushRow::Select, icon, TEXT);
        } else {
            let tool = self.tools_button_tool();
            draw_tool_icon(d, textures, self.map.theme, tool, icon);
            if self.singleton_placed(tool) {
                draw_badge(d, icon.x + icon.width, icon.y);
            }
        }
    }

    /// BRUSH: the picture of what a press on the canvas does - the brush's
    /// shape, or the select tool - and a caret, outlined in the amber while
    /// the select tool is the brush.
    fn draw_brush_button(&self, d: &mut impl RaylibDraw, rect: Rectangle, touch: bool) {
        let open = matches!(self.popup, Some(Popup::Brush | Popup::Stamps { .. }));
        let face = Face::IDLE.open(open).in_force(self.tool() == Tool::Select);
        let icon = draw_menu_box(d, rect, touch, face);
        draw_brush_icon(d, self.brush_shown(), icon, TEXT);
    }

    /// Whether a row of BRUSH's list is the one in force: the brush's
    /// shape while it paints, the select tool while it is the brush.
    fn brush_row_current(&self, row: BrushRow) -> bool {
        match row {
            BrushRow::Shape(shape) => self.tool() != Tool::Select && self.shape() == shape,
            BrushRow::Select => self.tool() == Tool::Select,
            BrushRow::Stamps => false,
        }
    }

    /// BRUSH's list below its button: a row per `BrushRow`, its picture and
    /// its name, the one in force lit and a shape the brush cannot take
    /// dim.
    fn draw_brush_list(&self, d: &mut impl RaylibDraw, rows: &chrome::Rows) {
        draw_panel(d, rows.panel);
        for (i, row) in BrushRow::ALL.into_iter().enumerate() {
            let r = rows.row(i);
            if self.brush_row_current(row) {
                let inset = Rectangle::new(r.x + 4.0, r.y + 2.0, r.width - 8.0, r.height - 4.0);
                d.draw_rectangle_rounded(inset, 0.2, EDITOR_PANEL_SEGMENTS, Color::new(255, 255, 255, 40));
            }
            let color = if self.brush_row_live(row) { TEXT } else { DIM };
            // Centred in the row, which a narrow window shortens
            // (`chrome::hanging_list`).
            let icon_y = r.y + (r.height - (ICON_PX + 8.0)) / 2.0;
            draw_brush_icon(d, row, Rectangle::new(r.x + 4.0, icon_y, ICON_PX + 8.0, ICON_PX + 8.0), color);
            let text_y = r.y as i32 + (r.height as i32 - HUD_TEXT_SIZE) / 2;
            d.draw_text(&brush_label(row), r.x as i32 + DROPDOWN_TEXT_X, text_y, HUD_TEXT_SIZE, color);
        }
    }

    /// The STAMPS list: a row per stamp - its picture, its name and its
    /// size in cells - and the pager when there are more than fit.
    fn draw_stamps_list(&self, d: &mut impl RaylibDraw, list: &chrome::LoadLayout, scroll: usize, hints: Hints) {
        let panel = list.rows.panel;
        draw_panel(d, panel);
        let rows = list.per_page;
        let last = self.stamps().len().saturating_sub(rows);
        let scroll = scroll.min(last);
        for (i, stamp) in self.stamps().iter().skip(scroll).take(rows).enumerate() {
            let row = list.rows.row(i);
            let picture = Rectangle::new(row.x + 8.0, row.y + 4.0, STAMP_PICTURE.0, STAMP_PICTURE.1);
            draw_clip_picture(d, &stamp.clip, picture, self.map().theme);
            let text_y = (row.y + (row.height - HUD_TEXT_SIZE as f32) / 2.0) as i32;
            let text_x = picture.x + picture.width + 12.0;
            d.draw_text(&fit_text(&stamp.label(), STAMP_NAME_W, HUD_TEXT_SIZE), text_x as i32, text_y, HUD_TEXT_SIZE, TEXT);
            let size = format!("{}x{}", stamp.clip.cols, stamp.clip.rows);
            let size_x = row.x + row.width - 12.0 - text_width(&size, UI_SMALL_TEXT);
            d.draw_text(&size, size_x as i32, (row.y + (row.height - UI_SMALL_TEXT as f32) / 2.0) as i32, UI_SMALL_TEXT, DIM);
        }
        if let Some(pager) = list.pager {
            let n = self.stamps().len();
            let hint = text().fmt(page_key(hints), &[("from", (scroll + 1).into()), ("to", (scroll + rows).min(n).into()), ("n", n.into())]);
            draw_pager(d, pager.row, &hint, scroll > 0, scroll < last);
        }
    }

    /// The select tool's strip under the bar: its plate and its buttons,
    /// drawn as the bar's are - a word's, or a flip's picture -, a dim one
    /// where it cannot act, PLACE in the amber.
    fn draw_strip(&self, d: &mut impl RaylibDraw, strip: &Strip, touch: bool) {
        crate::render::hud::draw_plate(d, strip.panel, crate::render::hud::PLATE_EDGE, 1.0);
        for slot in &strip.slots {
            let face = match (slot.enabled, slot.button) {
                (false, _) => Face::DIM,
                (true, StripButton::Place) => Face::ACTION,
                (true, _) => Face::IDLE,
            };
            match slot.button {
                StripButton::FlipH => draw_flip_button(d, slot.rect, touch, Axis::Horizontal, face),
                StripButton::FlipV => draw_flip_button(d, slot.rect, touch, Axis::Vertical, face),
                button => draw_word_button(d, slot.rect, touch, &strip_label(button), face),
            }
        }
    }

    /// The open popup, in UI points, its hints naming `hints`' input.
    fn draw_popup(&self, d: &mut impl RaylibDraw, chrome: &Chrome, textures: &EditorTextures, hints: Hints) {
        match (&self.popup, &chrome.popup) {
            (Some(Popup::Dropdown(category)), Some(PopupLayout::Dropdown(_, rows))) => self.draw_dropdown(d, rows, textures, *category),
            (Some(Popup::Brush), Some(PopupLayout::Brush(rows))) => self.draw_brush_list(d, rows),
            (Some(Popup::Stamps { scroll }), Some(PopupLayout::Stamps(list))) => self.draw_stamps_list(d, list, *scroll, hints),
            (Some(Popup::Palette), Some(PopupLayout::Palette(palette))) => self.draw_palette(d, palette, textures),
            (Some(Popup::Settings { .. }), Some(PopupLayout::Settings(panel))) => self.draw_settings(d, panel, textures),
            (Some(Popup::Lint { page }), Some(PopupLayout::Lint(lint))) => self.draw_lint_panel(d, lint, *page, hints),
            (Some(Popup::File), Some(PopupLayout::File(rows))) => Self::draw_file_menu(d, rows, &self.file_rows()),
            (Some(Popup::Load { entries, scroll }), Some(PopupLayout::Load(load))) => self.draw_load_list(d, load, entries, *scroll, hints, textures),
            (Some(Popup::Save { name }), Some(PopupLayout::Save(panel))) => {
                let panel = *panel;
                let touch = chrome.bar.touch;
                draw_panel(d, panel);
                d.draw_text(&text().get(keys::EDITOR_SAVE_AS), (panel.x + 12.0) as i32, (panel.y + 10.0) as i32, 16, TEXT);
                // The name's end, where it is being typed, stays in view.
                let typed = tail_fit(&format!("{name}_"), chrome::SAVE_NAME_W, 18);
                d.draw_text(&typed, (panel.x + 12.0) as i32, (panel.y + 34.0) as i32, 18, TEXT);
                let face = if name.is_empty() { Face::DIM } else { Face::ACTION };
                draw_word_button(d, chrome::save_button(panel, touch), touch, &text().get(keys::FILE_SAVE), face);
                let hint = text().get(hints.pick(keys::EDITOR_SAVE_HINT, keys::EDITOR_SAVE_HINT_TOUCH));
                d.draw_text(&hint, (panel.x + 12.0) as i32, (panel.y + 58.0) as i32, UI_SMALL_TEXT, Color::GRAY);
            }
            _ => {}
        }
    }

    /// The FILE menu below its button.
    fn draw_file_menu(d: &mut impl RaylibDraw, rows: &chrome::Rows, file_rows: &[FileRow]) {
        draw_panel(d, rows.panel);
        for (i, row) in file_rows.iter().enumerate() {
            let rect = rows.row(i);
            d.draw_text(&row.label(), rect.x as i32 + 16, (rect.y + (rect.height - HUD_TEXT_SIZE as f32) / 2.0) as i32, HUD_TEXT_SIZE, TEXT);
        }
    }

    /// The Load list: a row per map - its thumbnail (a dark box until its
    /// page has made it), its name, and under the name its size in cells
    /// and whether it ships with the game - and the pager when there are
    /// more than fit.
    fn draw_load_list(
        &self,
        d: &mut impl RaylibDraw,
        load: &chrome::LoadLayout,
        entries: &[MapEntry],
        scroll: usize,
        hints: Hints,
        textures: &EditorTextures,
    ) {
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
            let thumb = self.thumbnail(&entry.name);
            let texture = textures.thumbnails.and_then(|t| t.get(&entry.name));
            draw_thumbnail(d, chrome::load_picture(row), thumb, texture);
            let at = chrome::load_text(row);
            d.draw_text(&fit_text(&entry.name, at.width, HUD_TEXT_SIZE), at.x as i32, (row.y + chrome::LOAD_NAME_Y) as i32, HUD_TEXT_SIZE, TEXT);
            let mut detail = thumb.filter(|t| t.image.is_some()).map(|t| format!("{} x {}", t.cells.0, t.cells.1)).unwrap_or_default();
            if !entry.on_disk {
                if !detail.is_empty() {
                    detail.push_str("   ");
                }
                detail.push_str(&text().get(if entry.modified { keys::EDITOR_MODIFIED } else { keys::EDITOR_SHIPPED }));
            }
            d.draw_text(&fit_text(&detail, at.width, UI_SMALL_TEXT), at.x as i32, (row.y + chrome::LOAD_DETAIL_Y) as i32, UI_SMALL_TEXT, DIM);
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
            // A 40 pt rect: the icon's own 4 pt inset makes it 32 at 8;
            // centred in the row, which a narrow window shortens
            // (`chrome::hanging_list`).
            let icon = Rectangle::new(row.x + 4.0, row.y + (row.height - (ICON_PX + 8.0)) / 2.0, ICON_PX + 8.0, ICON_PX + 8.0);
            draw_tool_icon(d, textures, self.map.theme, tool, icon);
            if self.singleton_placed(tool) {
                draw_badge(d, icon.x + icon.width - 4.0, icon.y + 4.0);
            }
            let text_y = row.y as i32 + (row.height as i32 - HUD_TEXT_SIZE) / 2;
            d.draw_text(&label(tool), row.x as i32 + DROPDOWN_TEXT_X, text_y, HUD_TEXT_SIZE, TEXT);
        }
    }

    /// The palette the folded TOOLS button opens: a row per category, its
    /// name in the accent while the brush is one of its tools, then a cell
    /// per tool - the category's current tool lit, the brush outlined - and
    /// the brush's row under them as BRUSH's list draws it.
    fn draw_palette(&self, d: &mut impl RaylibDraw, palette: &chrome::Palette, textures: &EditorTextures) {
        draw_panel(d, palette.panel);
        let label = palette.brush_label();
        let color = if self.tool() == Tool::Select { BUILD_ACCENT } else { DIM };
        let y = (label.y + (label.height - UI_SMALL_TEXT as f32) / 2.0) as i32;
        d.draw_text(&text().get(keys::EDITOR_BRUSH), label.x as i32 + 8, y, UI_SMALL_TEXT, color);
        for (i, row) in BrushRow::ALL.into_iter().enumerate() {
            let cell = palette.brush_cell(i);
            if self.brush_row_current(row) {
                let inset = Rectangle::new(cell.x + 2.0, cell.y + 2.0, cell.width - 4.0, cell.height - 4.0);
                d.draw_rectangle_rounded(inset, 0.2, EDITOR_PANEL_SEGMENTS, Color::new(255, 255, 255, 40));
            }
            let color = if self.brush_row_live(row) { TEXT } else { DIM };
            draw_brush_icon(d, row, Rectangle::new(cell.x + 4.0, cell.y + 4.0, ICON_PX + 8.0, ICON_PX + 8.0), color);
            if row == BrushRow::Select && self.tool() == Tool::Select {
                d.draw_rectangle_lines_ex(Rectangle::new(cell.x + 2.0, cell.y + 2.0, cell.width - 4.0, cell.height - 4.0), 2.0, BUILD_ACCENT);
            }
        }
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

    /// The MAP panel (docs/game-editor-fusion.md section 9) as
    /// `chrome::MapPanel` lays it out: the rail's tabs or the groups'
    /// headings, every control shown and RESET MAP.
    fn draw_settings(&self, d: &mut impl RaylibDraw, layout: &chrome::MapPanel, textures: &EditorTextures) {
        let t = text();
        draw_hanging_panel(d, layout.panel);
        let settings = self.settings();
        let cli = self.cli_overrides;
        let mark = t.get(keys::SETTINGS_CLI);
        for (tab, rect) in &layout.tabs {
            let on = *tab == layout.tab;
            if on {
                d.draw_rectangle_rec(*rect, Color::new(255, 255, 255, 24));
                d.draw_rectangle_rec(Rectangle::new(rect.x, rect.y + 4.0, 4.0, rect.height - 8.0), BUILD_ACCENT);
            }
            let label = t.get(tab.key());
            let w = text_width(&label, MAP_LABEL_SIZE);
            let y = rect.y + (rect.height - MAP_LABEL_SIZE as f32) / 2.0;
            d.draw_text(&label, (rect.x + (rect.width - w) / 2.0) as i32, y as i32, MAP_LABEL_SIZE, if on { BUILD_ACCENT } else { TEXT });
            if tab.overridden(cli) {
                let mw = text_width(&mark, UI_SMALL_TEXT);
                d.draw_text(&mark, (rect.x + (rect.width - mw) / 2.0) as i32, (y + MAP_LABEL_SIZE as f32 + 2.0) as i32, UI_SMALL_TEXT, DIM);
            }
        }
        if let Some((_, first)) = layout.tabs.first() {
            d.draw_rectangle_rec(Rectangle::new(first.x + first.width - 1.0, layout.panel.y, 1.0, layout.panel.height), PANEL_BORDER);
        }
        for (tab, rect) in &layout.headings {
            let label = t.get(tab.key());
            let y = rect.y + (rect.height - MAP_HEADING_SIZE as f32) / 2.0 - 2.0;
            d.draw_text(&label, rect.x as i32 + 2, y as i32, MAP_HEADING_SIZE, BUILD_ACCENT);
            let mut x = rect.x + 2.0 + text_width(&label, MAP_HEADING_SIZE) + 10.0;
            if *tab == MapTab::Sky {
                let hint = t.get(keys::SETTINGS_SKY_HINT);
                d.draw_text(&hint, x as i32, (y + 3.0) as i32, UI_SMALL_TEXT, DIM);
                x += text_width(&hint, UI_SMALL_TEXT) + 8.0;
            }
            if tab.overridden(cli) {
                d.draw_text(&mark, x as i32, (y + 3.0) as i32, UI_SMALL_TEXT, DIM);
            }
            d.draw_rectangle_rec(Rectangle::new(rect.x, rect.y + rect.height - 3.0, rect.width, 1.0), Color::new(255, 255, 255, 40));
        }
        let reset = layout.reset;
        let color = if self.dirty() { BUILD_ACCENT } else { DIM };
        let inset = Rectangle::new(reset.x + 6.0, reset.y + 6.0, reset.width - 12.0, reset.height - 12.0);
        d.draw_rectangle_rounded_lines_ex(inset, 0.2, EDITOR_PANEL_SEGMENTS, 2.0, color);
        let label = t.get(keys::SETTINGS_RESET);
        let w = text_width(&label, MAP_LABEL_SIZE);
        d.draw_text(&label, (reset.x + (reset.width - w) / 2.0) as i32, (reset.y + (reset.height - MAP_LABEL_SIZE as f32) / 2.0) as i32, MAP_LABEL_SIZE, color);
        for placed in &layout.fields {
            self.draw_field(d, placed, &settings, textures);
        }
    }

    /// One control of the MAP panel and its label.
    fn draw_field(&self, d: &mut impl RaylibDraw, placed: &chrome::PlacedField, s: &MapSettings, textures: &EditorTextures) {
        let t = text();
        let field = placed.field;
        let rect = placed.rect;
        let outline = Color::new(255, 255, 255, 60);
        if let Some(label) = placed.label {
            let words = field.label();
            let beside = label.height >= rect.height;
            let flagged = field.overridden(self.cli_overrides);
            let mark = t.get(keys::SETTINGS_CLI);
            if beside {
                let lines = if flagged { MAP_LABEL_SIZE as f32 + 2.0 + UI_SMALL_TEXT as f32 } else { MAP_LABEL_SIZE as f32 };
                let y = label.y + (label.height - lines) / 2.0;
                d.draw_text(&words, (label.x + 4.0) as i32, y as i32, MAP_LABEL_SIZE, DIM);
                if flagged {
                    d.draw_text(&mark, (label.x + 4.0) as i32, (y + MAP_LABEL_SIZE as f32 + 2.0) as i32, UI_SMALL_TEXT, DIM);
                }
            } else {
                d.draw_text(&words, (label.x + 2.0) as i32, label.y as i32, UI_SMALL_TEXT, DIM);
                if flagged {
                    let x = label.x + 2.0 + text_width(&words, UI_SMALL_TEXT) + 4.0;
                    d.draw_text(&mark, x as i32, label.y as i32, UI_SMALL_TEXT, DIM);
                }
            }
        }
        let text_y = |r: Rectangle, size: i32| (r.y + (r.height - size as f32) / 2.0) as i32;
        match field.kind() {
            FieldKind::Choice(n) => {
                let chosen = field.chosen(s);
                for (i, option) in chrome::segments(rect, n).into_iter().enumerate() {
                    let words = field.option_label(i);
                    let w = text_width(&words, MAP_VALUE_SIZE);
                    let x = (option.x + (option.width - w) / 2.0) as i32;
                    if chosen == Some(i) {
                        d.draw_rectangle_rounded(option, 0.2, EDITOR_PANEL_SEGMENTS, BUILD_ACCENT);
                        d.draw_text(&words, x, text_y(option, MAP_VALUE_SIZE), MAP_VALUE_SIZE, BAR_FILL);
                    } else {
                        let inset = Rectangle::new(option.x + 1.0, option.y + 1.0, option.width - 2.0, option.height - 2.0);
                        d.draw_rectangle_rounded_lines_ex(inset, 0.2, EDITOR_PANEL_SEGMENTS, 1.0, outline);
                        d.draw_text(&words, x, text_y(option, MAP_VALUE_SIZE), MAP_VALUE_SIZE, TEXT);
                    }
                }
            }
            FieldKind::Stepper | FieldKind::Chassis(_) => {
                let inset = Rectangle::new(rect.x + 1.0, rect.y + 1.0, rect.width - 2.0, rect.height - 2.0);
                d.draw_rectangle_rounded_lines_ex(inset, 0.2, EDITOR_PANEL_SEGMENTS, 1.0, outline);
                let glyphs = if field.is_number() { ["-", "+"] } else { ["<", ">"] };
                for (button, glyph) in [(chrome::stepper_dec(rect), glyphs[0]), (chrome::stepper_inc(rect), glyphs[1])] {
                    let w = text_width(glyph, HUD_TEXT_SIZE);
                    d.draw_text(glyph, (button.x + (button.width - w) / 2.0) as i32, text_y(button, HUD_TEXT_SIZE), HUD_TEXT_SIZE, TEXT);
                }
                let middle = Rectangle::new(rect.x + chrome::MAP_STEP_W, rect.y, rect.width - 2.0 * chrome::MAP_STEP_W, rect.height);
                if field == MapField::Anchor {
                    const SPAN: f32 = 28.0;
                    draw_anchor(d, Rectangle::new(middle.x + (middle.width - SPAN) / 2.0, middle.y, SPAN, middle.height), self.resize_anchor, TEXT);
                    return;
                }
                let words = field.value(s, self.size_cells());
                let w = text_width(&words, MAP_VALUE_SIZE);
                match field.chassis(s) {
                    Some((kind, seat)) => {
                        let size = crate::TANK_FRAME_SIZE;
                        let x = middle.x + (middle.width - size - 8.0 - w) / 2.0;
                        let icon = Rectangle::new(x, middle.y + (middle.height - size) / 2.0, size, size);
                        for src in crate::tank::chassis_icon_source_recs(kind, seat) {
                            d.draw_texture_pro(textures.tanks, src, icon, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
                        }
                        d.draw_text(&words, (x + size + 8.0) as i32, text_y(middle, MAP_VALUE_SIZE), MAP_VALUE_SIZE, TEXT);
                    }
                    None => d.draw_text(&words, (middle.x + (middle.width - w) / 2.0) as i32, text_y(middle, MAP_VALUE_SIZE), MAP_VALUE_SIZE, TEXT),
                }
            }
            FieldKind::Sky(sky) => {
                let on = s.weather.contains(sky);
                let inset = Rectangle::new(rect.x + 1.0, rect.y + 1.0, rect.width - 2.0, rect.height - 2.0);
                if on {
                    d.draw_rectangle_rounded_lines_ex(inset, 0.2, EDITOR_PANEL_SEGMENTS, 2.0, BUILD_ACCENT);
                } else {
                    d.draw_rectangle_rounded_lines_ex(inset, 0.2, EDITOR_PANEL_SEGMENTS, 1.0, outline);
                }
                let swatch = Rectangle::new(rect.x + 8.0, rect.y + (rect.height - MAP_SWATCH) / 2.0, MAP_SWATCH, MAP_SWATCH);
                let tint = if on { 255 } else { 110 };
                d.draw_rectangle_rec(swatch, sky_swatch(sky).alpha(tint as f32 / 255.0));
                d.draw_rectangle_lines_ex(swatch, 2.0, Color::new(0, 0, 0, 120));
                let name = sky_label(sky);
                d.draw_text(&name, (swatch.x + MAP_SWATCH + 8.0) as i32, text_y(rect, MAP_SKY_SIZE), MAP_SKY_SIZE, if on { TEXT } else { DIM });
                draw_check(d, Rectangle::new(rect.x + rect.width - 8.0 - MAP_CHECK, rect.y, MAP_CHECK, rect.height), on);
            }
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
                let fix = chrome::LintLayout::fix_box(row);
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

    /// The select tool's marks on the canvas, in world pixels over the map:
    /// the cells a drag carries, drawn where they would land; the paste
    /// ghost drawn faintly on a tinted rectangle in the accent's outline;
    /// the rectangle a drag is drawing and the selection, each in a
    /// marching outline.
    fn draw_selection<D: RaylibDraw>(&self, d: &mut D, textures: &EditorTextures, time: f32, culled: impl Fn(Position, f32) -> bool) {
        let field = self.map.field_size();
        let cells = |d: &mut D, clip: &Clip, at: (i32, i32), alpha: u8| {
            for (col, row, obj) in clip.placed_at(at) {
                let pos = map::cell_to_world(col, row);
                if !culled(pos, 0.0) {
                    draw_cell(d, textures, field, pos, &obj, alpha, time);
                }
            }
        };
        if let Some((rect, clip)) = self.lifted() {
            cells(d, clip, (rect.col, rect.row), LIFTED_ALPHA);
            draw_marching(d, rect.world(), time);
        }
        if let Some(ghost) = self.ghost() {
            let r = ghost.rect().world();
            d.draw_rectangle_rec(r, Color::new(BUILD_ACCENT.r, BUILD_ACCENT.g, BUILD_ACCENT.b, 36));
            cells(d, &ghost.clip, ghost.at, GHOST_ALPHA);
            d.draw_rectangle_lines_ex(r, 2.0, BUILD_ACCENT);
        }
        if let Some(rect) = self.selecting() {
            d.draw_rectangle_rec(rect.world(), Color::new(255, 255, 255, 30));
            draw_marching(d, rect.world(), time);
        }
        if let (Some(rect), None) = (self.selection(), self.lifted()) {
            draw_marching(d, rect.world(), time);
        }
    }

    /// What the brush's shape is about to do, in world pixels over the
    /// map: the rectangle a RECT drag is drawing - the brush's object faint
    /// in each of its cells (an outline alone past `RECT_GHOST_CELLS`), or a
    /// red wash where it erases - and SCATTER's footprint round the cell
    /// under the pointer.
    fn draw_brush_marks<D: RaylibDraw>(
        &self,
        d: &mut D,
        textures: &EditorTextures,
        time: f32,
        cursor: Option<(i32, i32)>,
        culled: impl Fn(Position, f32) -> bool,
    ) {
        if let Some((rect, erase)) = self.rect_stroke() {
            let r = rect.world();
            if erase {
                d.draw_rectangle_rec(r, Color::new(ERASE_MARK.r, ERASE_MARK.g, ERASE_MARK.b, 60));
                d.draw_rectangle_lines_ex(r, 2.0, ERASE_MARK);
                return;
            }
            if let Some(obj) = self.tool().object()
                && (rect.cols * rect.rows) as usize <= RECT_GHOST_CELLS
            {
                let field = self.map.field_size();
                for (col, row) in rect.cells() {
                    let pos = map::cell_to_world(col, row);
                    if !culled(pos, 0.0) {
                        draw_cell(d, textures, field, pos, &obj, GHOST_ALPHA, time);
                    }
                }
            } else {
                d.draw_rectangle_rec(r, Color::new(255, 255, 255, 40));
            }
            d.draw_rectangle_lines_ex(r, 2.0, Color::new(255, 255, 255, 220));
        } else if self.tool() != Tool::Select
            && self.shape() == Shape::Scatter
            && let Some(cell) = cursor
        {
            let half = OBSTACLE_GRID_SIZE / 2.0;
            for (col, row) in brush::footprint(cell, BrushRules::current().scatter_radius, self.field_cells()) {
                let pos = map::cell_to_world(col, row);
                d.draw_rectangle_rec(Rectangle::new(pos.x - half, pos.y - half, OBSTACLE_GRID_SIZE, OBSTACLE_GRID_SIZE), Color::new(255, 255, 255, 36));
            }
        }
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

/// How a button of the bar, the select tool's strip or the Save prompt
/// is drawn: one box for all of them (`chrome::button_box`, its outline
/// `chrome::BOX_EDGE` of square corners, as play's corner buttons are
/// framed) and one rule for its colours - the outline and the word or
/// picture in it in one colour each, washed while its popup is up.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Face {
    edge: Color,
    ink: Color,
    wash: bool,
}

/// A button that can act: a quiet outline round a word in the text colour.
const EDGE: Color = Color::new(255, 255, 255, 70);
/// A button that cannot act now: an outline fainter still round a dim word.
const EDGE_DIM: Color = Color::new(255, 255, 255, 30);
/// What a button is washed with while its popup is up, as a list's row in
/// force is.
const WASH: Color = Color::new(255, 255, 255, 40);

impl Face {
    /// A button that can act.
    const IDLE: Face = Face { edge: EDGE, ink: TEXT, wash: false };
    /// A button that cannot act now: UNDO with nothing to undo, FIT at FIT,
    /// COPY with nothing selected, SAVE with no name.
    const DIM: Face = Face { edge: EDGE_DIM, ink: DIM, wash: false };
    /// What leaves the builder for a round, or settles what is held: PLAY,
    /// PLAY HERE, PLACE, SAVE - outline and word in the builder's amber, as
    /// play's corner draws BUILD.
    const ACTION: Face = Face { edge: BUILD_ACCENT, ink: BUILD_ACCENT, wash: false };

    /// `IDLE` where the button can act, else `DIM`.
    fn able(can: bool) -> Face {
        if can { Face::IDLE } else { Face::DIM }
    }

    /// Washed while the button's popup is up.
    fn open(self, open: bool) -> Face {
        Face { wash: self.wash || open, ..self }
    }

    /// Outlined in the amber while the brush the button holds is the one in
    /// force: a category's tool, the eraser, the select tool.
    fn in_force(self, on: bool) -> Face {
        if on { Face { edge: BUILD_ACCENT, ..self } } else { self }
    }
}

/// A button's box in `face`: washed while its popup is up, then outlined.
fn draw_box(d: &mut impl RaylibDraw, bx: Rectangle, face: Face) {
    if face.wash {
        d.draw_rectangle_rec(bx, WASH);
    }
    d.draw_rectangle_lines_ex(bx, BOX_EDGE, face.edge);
}

/// Where a word stands in `bx`: centred between `left` and `right`, and
/// top to bottom, at the bar's one size.
fn word_at(bx: Rectangle, left: f32, right: f32, word: &str) -> (i32, i32) {
    let w = text_width(word, BAR_SMALL_TEXT);
    ((left + (right - left - w) / 2.0).round() as i32, (bx.y + (bx.height - BAR_SMALL_TEXT as f32) / 2.0) as i32)
}

/// A word's button in `slot`: UNDO, REDO, FIT, CHECK, PLAY HERE, PLAY, the
/// strip's words and the Save prompt's SAVE.
fn draw_word_button(d: &mut impl RaylibDraw, slot: Rectangle, touch: bool, word: &str, face: Face) {
    let bx = button_box(slot, touch);
    draw_box(d, bx, face);
    let (x, y) = word_at(bx, bx.x, bx.x + bx.width, word);
    d.draw_text(word, x, y, BAR_SMALL_TEXT, face.ink);
}

/// A button opening a popup in `slot`: its box and its caret at the right
/// end; the icon box before the caret (`chrome::icon_box`) for the caller
/// to draw into. The category buttons, BRUSH and TOOLS.
fn draw_menu_box(d: &mut impl RaylibDraw, slot: Rectangle, touch: bool, face: Face) -> Rectangle {
    let bx = button_box(slot, touch);
    draw_box(d, bx, face);
    let caret = caret_at(bx);
    draw_caret(d, caret.x as i32, caret.y as i32, DIM);
    icon_box(bx, true)
}

/// FILE / MAP: a word's button opening a menu, the word centred in the
/// room before the caret.
fn draw_menu_button(d: &mut impl RaylibDraw, slot: Rectangle, touch: bool, word: &str, face: Face) {
    let bx = button_box(slot, touch);
    draw_box(d, bx, face);
    let caret = caret_at(bx);
    let (x, y) = word_at(bx, bx.x + BOX_EDGE + BOX_PAD, caret.x - CARET_GAP, word);
    d.draw_text(word, x, y, BAR_SMALL_TEXT, face.ink);
    draw_caret(d, caret.x as i32, caret.y as i32, DIM);
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
        Tool::Prop(crate::obstacle::Material::Lamp) => draw_lamp_icon(d, dest),
        Tool::Lava => draw_lava_icon(d, dest),
        Tool::Volcano => draw_volcano_icon(d, dest),
        Tool::Wall(material) | Tool::Prop(material) => {
            let (sheet, src) = obstacle::icon_source_rec(material, theme);
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
            draw_player_ring(d, center, dest.width / 2.0, 0, 255);
            for src in crate::tank::icon_source_recs(0) {
                d.draw_texture_pro(textures.tanks, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
            }
        }
        Tool::Start2 => {
            let center = Position::new(dest.x + dest.width / 2.0, dest.y + dest.height / 2.0);
            draw_player_ring(d, center, dest.width / 2.0, 1, 255);
            for src in crate::tank::icon_source_recs(1) {
                d.draw_texture_pro(textures.tanks, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
            }
        }
        Tool::EnemyFrog => {
            let center = Position::new(dest.x + dest.width / 2.0, dest.y + dest.height / 2.0);
            draw_enemy_ring(d, center, dest.width / 2.0, 255);
            let src = Rectangle::new(0.0, 0.0, crate::FROG_TEXTURE_SIZE, crate::FROG_TEXTURE_SIZE);
            d.draw_texture_pro(textures.frog_idle, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        Tool::Gate => {
            let center = Position::new(dest.x + dest.width / 2.0, dest.y + dest.height / 2.0);
            draw_gate_chevron(d, center, dest.width, Position::new(1.0, 0.0), GATE_COLOR);
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
        Tool::Bush(bush) => {
            // On a patch of the theme's floor like tall grass: a bush is
            // ground cover, not a thing standing on the field.
            let (r, g, b) = theme.floor_color();
            d.draw_rectangle_rounded(dest, 0.15, EDITOR_PANEL_SEGMENTS, Color::new(r, g, b, 255));
            let src = crate::grass::bush_source_rec(bush, theme);
            d.draw_texture_pro(textures.bushes, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        Tool::Pickup(pickup) => {
            // The crate as it stands on the field - the brush is the thing
            // it places - or, in a slot too small for it (the bar's), its
            // symbol, either at a whole multiple of its sheet's scale so
            // every block stays: centred on `rect`, a cell's empty rim free
            // to reach into the inset.
            let side = rect.width.min(rect.height);
            let (texture, cell, src) = if side >= crate::CRATE_CELL {
                (textures.crates, crate::CRATE_CELL, crate::pickup::crate_src(pickup, crate::CRATE_COL_INTACT))
            } else {
                (textures.pickup_glyphs, crate::PICKUP_GLYPH_CELL, crate::pickup::glyph_src(pickup))
            };
            let size = cell * (side / cell).floor().max(1.0);
            let at = Rectangle::new((rect.x + (rect.width - size) / 2.0).round(), (rect.y + (rect.height - size) / 2.0).round(), size, size);
            d.draw_texture_pro(texture, src, at, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
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
        Tool::Select => draw_brush_icon(d, BrushRow::Select, rect, TEXT),
    }
}

/// The lava tool's icon: a molten pool in the fire ramp's steps inside a
/// crust of black, bands of gold running across it.
fn draw_lava_icon(d: &mut impl RaylibDraw, dest: Rectangle) {
    use crate::pyro::{FIRE, SMOKE};
    d.draw_rectangle_rounded(dest, 0.3, EDITOR_PANEL_SEGMENTS, SMOKE[0]);
    let inner = Rectangle::new(dest.x + 3.0, dest.y + 3.0, dest.width - 6.0, dest.height - 6.0);
    d.draw_rectangle_rounded(inner, 0.3, EDITOR_PANEL_SEGMENTS, FIRE[3]);
    let band = inner.height / 5.0;
    for (i, color) in [(1, FIRE[5]), (3, FIRE[4])] {
        d.draw_rectangle_rec(Rectangle::new(inner.x + 2.0, inner.y + band * i as f32, inner.width - 4.0, band * 0.6), color);
    }
    d.draw_rectangle_rec(Rectangle::new(inner.x + inner.width * 0.3, inner.y + band, band, band * 0.6), FIRE[6]);
}

/// The volcano tool's icon: the cone from above, ash at its foot and rust
/// at its rim, the crater molten.
fn draw_volcano_icon(d: &mut impl RaylibDraw, dest: Rectangle) {
    use crate::pyro::{FIRE, SMOKE};
    let c = Vector2::new(dest.x + dest.width / 2.0, dest.y + dest.height / 2.0);
    let r = dest.width.min(dest.height) / 2.0;
    d.draw_circle_v(c, r, SMOKE[0]);
    d.draw_circle_v(c, r - 2.0, SMOKE[2]);
    d.draw_circle_v(Vector2::new(c.x - 1.0, c.y - 1.0), r * 0.68, Color::new(0x8D, 0x4A, 0x25, 255));
    d.draw_circle_v(c, r * 0.36, SMOKE[0]);
    d.draw_circle_v(c, r * 0.28, FIRE[3]);
    d.draw_circle_v(Vector2::new(c.x - 1.0, c.y - 1.0), r * 0.14, FIRE[6]);
}

/// The lamp post's icon: the post and its lantern, lit.
fn draw_lamp_icon(d: &mut impl RaylibDraw, dest: Rectangle) {
    use crate::pyro::{FIRE, SMOKE};
    let (cx, w, h) = (dest.x + dest.width / 2.0, dest.width, dest.height);
    d.draw_circle_v(Vector2::new(cx, dest.y + h * 0.3), w * 0.32, Color::new(255, 214, 120, 70));
    d.draw_rectangle_rec(Rectangle::new(cx - w * 0.06, dest.y + h * 0.35, w * 0.12, h * 0.55), SMOKE[0]);
    d.draw_rectangle_rec(Rectangle::new(cx - w * 0.22, dest.y + h * 0.85, w * 0.44, h * 0.1), SMOKE[1]);
    d.draw_rectangle_rec(Rectangle::new(cx - w * 0.18, dest.y + h * 0.12, w * 0.36, h * 0.3), SMOKE[0]);
    d.draw_rectangle_rec(Rectangle::new(cx - w * 0.12, dest.y + h * 0.16, w * 0.24, h * 0.22), FIRE[5]);
    d.draw_rectangle_rec(Rectangle::new(cx - w * 0.05, dest.y + h * 0.2, w * 0.1, h * 0.12), FIRE[6]);
}

/// A row of BRUSH's list as a picture inside `rect` (4 pt inset, like a
/// tool's icon) in `color`, laid out on a grid of sixteen units a side: the
/// pen a stair of cells, the select tool a dashed square, the stamps a
/// rubber stamp over its print.
pub fn draw_brush_icon(d: &mut impl RaylibDraw, row: BrushRow, rect: Rectangle, color: Color) {
    let dest = Rectangle::new(rect.x + 4.0, rect.y + 4.0, rect.width - 8.0, rect.height - 8.0);
    let u = dest.width.min(dest.height) / 16.0;
    let mut unit = |x: f32, y: f32, w: f32, h: f32| {
        let r = Rectangle::new((dest.x + x * u).round(), (dest.y + y * u).round(), (w * u).round().max(1.0), (h * u).round().max(1.0));
        d.draw_rectangle_rec(r, color);
    };
    match row {
        BrushRow::Shape(Shape::Pen) => {
            for i in 0..4 {
                let f = 3.0 * i as f32;
                unit(2.0 + f, 11.0 - f, 3.0, 3.0);
            }
        }
        BrushRow::Shape(Shape::Rect) => {
            // A solid block, its corner marked where the drag began.
            unit(2.0, 4.0, 12.0, 9.0);
            unit(1.0, 3.0, 2.0, 2.0);
        }
        BrushRow::Shape(Shape::Fill) => {
            // A bucket and its drip.
            unit(4.0, 1.0, 5.0, 1.0);
            unit(3.0, 2.0, 1.0, 2.0);
            unit(9.0, 2.0, 1.0, 2.0);
            unit(2.0, 4.0, 9.0, 1.0);
            unit(3.0, 5.0, 7.0, 8.0);
            unit(12.0, 6.0, 2.0, 3.0);
            unit(12.0, 11.0, 2.0, 2.0);
        }
        BrushRow::Shape(Shape::Scatter) => {
            for (x, y) in [(2.0, 2.0), (9.0, 1.0), (13.0, 5.0), (5.0, 7.0), (10.0, 9.0), (1.0, 12.0), (7.0, 13.0), (13.0, 13.0)] {
                unit(x, y, 2.0, 2.0);
            }
        }
        BrushRow::Select => {
            // A square of dashes: two units on, one off.
            for i in 0..4 {
                let f = 3.0 * i as f32;
                unit(2.0 + f, 2.0, 2.0, 1.0);
                unit(2.0 + f, 13.0, 2.0, 1.0);
                unit(2.0, 2.0 + f, 1.0, 2.0);
                unit(13.0, 2.0 + f, 1.0, 2.0);
            }
        }
        BrushRow::Stamps => {
            unit(6.0, 1.0, 4.0, 2.0);
            unit(7.0, 3.0, 2.0, 5.0);
            unit(2.0, 8.0, 12.0, 3.0);
            unit(2.0, 13.0, 12.0, 1.0);
        }
    }
}

/// How opaque the cells a drag of the select tool carries are drawn.
const LIFTED_ALPHA: u8 = 230;
/// The most cells a RECT drag draws the brush's object in; a larger
/// rectangle is a wash and an outline, so a drag across a 250 x 250 map
/// does not draw tens of thousands of sprites a frame.
const RECT_GHOST_CELLS: usize = 4096;
/// What an erasing RECT drag is outlined in.
const ERASE_MARK: Color = Color::new(230, 60, 60, 220);
/// How opaque the paste ghost's cells are drawn.
const GHOST_ALPHA: u8 = 150;

/// One map cell's object at world `pos` on the canvas, `alpha` opaque
/// (255 for the map itself, less for what the select tool carries), on a
/// map of `field` world pixels (a gate's chevron points in from its edge).
/// Road and water, which the map's own cells paint into the ground, are
/// drawn as a tile of their own here - what a lifted or pasted one shows.
fn draw_cell<D: RaylibDraw>(d: &mut D, textures: &EditorTextures, field: (f32, f32), pos: Position, obj: &CellObject, alpha: u8, time: f32) {
    let size = OBSTACLE_GRID_SIZE;
    let dest = Rectangle::new(pos.x, pos.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);
    let tint = Color::new(255, 255, 255, alpha);
    let faded = |c: Color| Color::new(c.r, c.g, c.b, ((c.a as u32 * alpha as u32) / 255) as u8);
    match *obj {
        CellObject::Barrel { drum: Some(drum) } => {
            let src = obstacle::drum_source_rec(drum);
            d.draw_texture_pro(textures.props, src, dest, origin, 0.0, tint);
        }
        CellObject::Wall { .. } | CellObject::Sandbag | CellObject::Barrel { .. } | CellObject::Fence | CellObject::Target => {
            // At the sheet's own cell size: the cell for a wall or a prop,
            // a range board's 44px, so its overhang matches a round.
            let material = obj.material().expect("solid cells have a material");
            let (sheet, src) = obstacle::icon_source_rec(material, textures.theme);
            let drawn = sheet.cell();
            let dest = Rectangle::new(pos.x, pos.y, drawn, drawn);
            let origin = Vector2::new(drawn / 2.0, drawn / 2.0);
            d.draw_texture_pro(sheet_texture(textures, sheet), src, dest, origin, 0.0, tint);
        }
        CellObject::Oil => {
            let src = obstacle::oil_source_rec(pos);
            d.draw_texture_pro(textures.props, src, dest, origin, 0.0, tint);
        }
        CellObject::Tree
        | CellObject::Pine
        | CellObject::Spruce
        | CellObject::Scots
        | CellObject::Fir
        | CellObject::Birch
        | CellObject::Willow
        | CellObject::Palm
        | CellObject::Snag => {
            // Drawn at the sprite's own 48px, not the 32px cell, so the
            // canopy overhang matches a round.
            let material = obj.material().expect("tree cells have a material");
            let (sheet, src) = obstacle::icon_source_rec(material, textures.theme);
            let big = crate::TREE_TEXTURE_SIZE;
            let dest = Rectangle::new(pos.x, pos.y, big, big);
            let origin = Vector2::new(big / 2.0, big / 2.0);
            d.draw_texture_pro(sheet_texture(textures, sheet), src, dest, origin, 0.0, tint);
        }
        CellObject::Tesla { .. } | CellObject::GunTower { .. } | CellObject::BioSlush { .. } => {
            // At the sprite's own 48px like a tree, base then top pointing
            // up, with its reach ringed faintly so a map maker sees what it
            // covers.
            let (kind, side) = obj.tower().expect("tower cells name a tower");
            let big = crate::TREE_TEXTURE_SIZE;
            let dest = Rectangle::new(pos.x, pos.y, big, big);
            let origin = Vector2::new(big / 2.0, big / 2.0);
            let reach = if side == crate::frog::Side::Enemy { Color::new(230, 60, 60, 70) } else { Color::new(77, 155, 230, 70) };
            d.draw_circle_lines(pos.x as i32, pos.y as i32, kind.range(), faded(reach));
            for src in [crate::tower::icon_source_rec(kind, side), crate::tower::icon_top_rec(kind, side)] {
                d.draw_texture_pro(textures.towers, src, dest, origin, 0.0, tint);
            }
        }
        CellObject::Road => {
            d.draw_rectangle_rec(Rectangle::new(pos.x - size / 2.0, pos.y - size / 2.0, size, size), faded(ROAD_TILE));
        }
        CellObject::Water => {
            d.draw_texture_pro(textures.ground, ground::water_icon_source_rec(), dest, origin, 0.0, tint);
        }
        // A lifted or pasted lava cell on its own: the stream piece with
        // no neighbours. The canvas draws the map's lava from its layout.
        CellObject::Lava => {
            let r = Rectangle::new(pos.x - size / 2.0 + 2.0, pos.y - size / 2.0 + 2.0, size - 4.0, size - 4.0);
            d.draw_rectangle_rounded(r, 0.4, EDITOR_PANEL_SEGMENTS, faded(crate::pyro::SMOKE[0]));
            let inner = Rectangle::new(r.x + 4.0, r.y + 4.0, r.width - 8.0, r.height - 8.0);
            d.draw_rectangle_rounded(inner, 0.4, EDITOR_PANEL_SEGMENTS, faded(crate::pyro::FIRE[3]));
        }
        CellObject::Volcano => {
            let picture = crate::volcano::cone(&[]);
            let asleep = crate::volcano::phase(-1.0, 0.0, &crate::tuning::tuning());
            crate::volcano::draw_cone(&mut GpuCanvas::new(&mut *d, textures), pos, &picture, &asleep, time);
        }
        CellObject::Lamp => {
            let t = crate::tuning::tuning();
            crate::lamp::draw_post(&mut GpuCanvas::new(&mut *d, textures), pos, time, (t.shadow_dir_x, t.shadow_dir_y), true, true);
        }
        CellObject::Portal => draw_portal(&mut GpuCanvas::new(&mut *d, textures), pos, time, tint),
        CellObject::Door { .. } => crate::training::draw_door(&mut GpuCanvas::new(&mut *d, textures), pos),
        CellObject::Flag => {
            crate::training::draw_flag(&mut GpuCanvas::new(&mut *d, textures), pos, crate::tank::team_color(0), false, time);
        }
        CellObject::TallGrass => {
            // The round scatters several hashed tufts per cell; one centred
            // tuft is enough to show the cell is grassed.
            let cell = crate::GRASS_TEXTURE_SIZE;
            let src = Rectangle::new(0.0, 0.0, cell, cell);
            let scale = cell * crate::tuning::tuning().grass_scale;
            let at = Rectangle::new(pos.x, pos.y + size / 2.0, scale, scale);
            d.draw_texture_pro(textures.grass, src, at, Vector2::new(scale / 2.0, scale), 0.0, tint);
        }
        CellObject::Bush | CellObject::BerryBush | CellObject::Juniper | CellObject::Fern | CellObject::AutumnBush | CellObject::Reeds => {
            // The bush as a round stands it: its first variant, dry on a
            // desert map, filling its cell.
            let bush = obj.bush().expect("a bush cell grows a bush");
            let src = crate::grass::bush_source_rec(bush, textures.theme);
            d.draw_texture_pro(textures.bushes, src, dest, origin, 0.0, tint);
        }
        CellObject::Frog => {
            let src = Rectangle::new(0.0, 0.0, crate::FROG_TEXTURE_SIZE, crate::FROG_TEXTURE_SIZE);
            d.draw_texture_pro(textures.frog_idle, src, dest, origin, 0.0, tint);
        }
        CellObject::Start | CellObject::Start2 => {
            let player = u8::from(*obj == CellObject::Start2);
            draw_player_ring(d, pos, size / 2.0, player as usize, alpha);
            for src in crate::tank::icon_source_recs(player) {
                d.draw_texture_pro(textures.tanks, src, dest, origin, 0.0, tint);
            }
        }
        CellObject::EnemyFrog => {
            draw_enemy_ring(d, pos, size / 2.0, alpha);
            let src = Rectangle::new(0.0, 0.0, crate::FROG_TEXTURE_SIZE, crate::FROG_TEXTURE_SIZE);
            d.draw_texture_pro(textures.frog_idle, src, dest, origin, 0.0, tint);
        }
        CellObject::Gate => match gate_inward(pos, field.0, field.1) {
            Some(inward) => draw_gate_chevron(d, pos, size, inward, faded(GATE_COLOR)),
            None => d.draw_rectangle_lines_ex(Rectangle::new(pos.x - size / 2.0, pos.y - size / 2.0, size, size), 2.0, faded(GATE_COLOR)),
        },
        CellObject::Pickup { pickup } => {
            // The crate at its own size, a little past its cell, as a round
            // stands it.
            let size = crate::CRATE_CELL;
            let src = crate::pickup::crate_src(pickup, crate::CRATE_COL_INTACT);
            let dest = Rectangle::new(pos.x, pos.y, size, size);
            d.draw_texture_pro(textures.crates, src, dest, Vector2::new(size / 2.0, size / 2.0), 0.0, tint);
        }
    }
}

/// The road tool's own tile colour, the dirt under a lifted or pasted
/// road cell.
const ROAD_TILE: Color = Color::new(150, 111, 74, 255);

/// A selection's outline in world pixels: 2 px dashes of four blocks, light
/// and dark by turns, marching round `r` at a block every
/// `MARCH_SECONDS` - it reads over any ground and any wall.
fn draw_marching(d: &mut impl RaylibDraw, r: Rectangle, time: f32) {
    const DASH: f32 = 8.0;
    const LINE: f32 = 2.0;
    let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.width, r.y + r.height);
    // The piece `from`..`to` of each side, clockwise from the top-left
    // corner: along the top, down the right, back along the bottom, up the
    // left - 2 px inside the rectangle.
    let piece = |side: usize, from: f32, to: f32| match side {
        0 => Rectangle::new(x0 + from, y0, to - from, LINE),
        1 => Rectangle::new(x1 - LINE, y0 + from, LINE, to - from),
        2 => Rectangle::new(x1 - to, y1 - LINE, to - from, LINE),
        _ => Rectangle::new(x0, y1 - to, LINE, to - from),
    };
    // The run round the perimeter starts a block further on each step of
    // the clock, so the dashes march.
    let mut run = -(((time / MARCH_SECONDS).floor() as i64).rem_euclid((2.0 * DASH / LINE) as i64) as f32) * LINE;
    for (side, len) in [r.width, r.height, r.width, r.height].into_iter().enumerate() {
        let mut along = 0.0;
        while along < len {
            let dash = (run / DASH).floor();
            let step = ((dash + 1.0) * DASH - run).min(len - along);
            let light = (dash as i64).rem_euclid(2) == 0;
            let color = if light { Color::new(255, 255, 255, 230) } else { Color::new(16, 16, 20, 230) };
            d.draw_rectangle_rec(piece(side, along, along + step), color);
            along += step;
            run += step;
        }
    }
}

/// How long the selection's outline takes to march one 2 px block.
const MARCH_SECONDS: f32 = 0.08;

/// A row of BRUSH's list's name in the language on screen.
fn brush_label(row: BrushRow) -> String {
    let t = text();
    match row {
        BrushRow::Shape(shape) => t.get(shape.label_key()),
        BrushRow::Select => label(Tool::Select),
        BrushRow::Stamps => t.get(keys::BRUSH_STAMPS),
    }
}

/// A strip button's word in the language on screen.
fn strip_label(button: StripButton) -> String {
    button.label_key().map(|key| text().get(key)).unwrap_or_default()
}

/// A flip's strip button: a button's box round a picture of whole 2 pt
/// blocks - two arrowheads pointing away from a bar between them, left and
/// right for `Axis::Horizontal`, up and down for `Axis::Vertical`.
fn draw_flip_button(d: &mut impl RaylibDraw, slot: Rectangle, touch: bool, axis: Axis, face: Face) {
    let bx = button_box(slot, touch);
    draw_box(d, bx, face);
    let (cx, cy) = ((bx.x + bx.width / 2.0).round(), (bx.y + bx.height / 2.0).round());
    let mut block = |along: f32, across: f32, w: f32, h: f32| {
        let r = match axis {
            Axis::Horizontal => Rectangle::new(cx + along, cy + across, w, h),
            Axis::Vertical => Rectangle::new(cx + across, cy + along, h, w),
        };
        d.draw_rectangle_rec(r, face.ink);
    };
    // The bar, then each arrowhead's three columns, widest at the bar.
    block(-1.0, -8.0, 2.0, 16.0);
    for (i, half) in [6.0_f32, 4.0, 2.0].into_iter().enumerate() {
        let step = 2.0 * i as f32;
        block(-5.0 - step, -half, 2.0, 2.0 * half);
        block(3.0 + step, -half, 2.0, 2.0 * half);
    }
}

/// A clip's picture in `rect`: the cells it holds in the minimap's colours
/// (`minimap::Class`) on the theme's ground, each a square of whole points
/// as large as the box allows, centred.
fn draw_clip_picture(d: &mut impl RaylibDraw, clip: &Clip, rect: Rectangle, theme: Theme) {
    use crate::minimap::Class;
    let (cols, rows) = (clip.cols.max(1) as f32, clip.rows.max(1) as f32);
    let k = (rect.width / cols).min(rect.height / rows).floor().max(1.0);
    let (w, h) = (cols * k, rows * k);
    let (x0, y0) = ((rect.x + (rect.width - w) / 2.0).round(), (rect.y + (rect.height - h) / 2.0).round());
    d.draw_rectangle_rec(Rectangle::new(x0, y0, w, h), Class::Ground.color(theme));
    for &(c, r, obj) in &clip.cells {
        let class = Class::solid_of(&obj).unwrap_or(Class::floor(Some(&obj), ground::Depth::Shallow));
        d.draw_rectangle_rec(Rectangle::new(x0 + c as f32 * k, y0 + r as f32 * k, k, k), class.color(theme));
    }
}

/// A map's thumbnail in its Load row's box: a dark box, and over it the
/// map's minimap (`thumbs.rs`) at its own shape (`chrome::fit_picture`),
/// the field's texels as the navigator draws them (`minimap::source`) -
/// once the texture `app.rs` uploaded holds this very picture.
fn draw_thumbnail(d: &mut impl RaylibDraw, rect: Rectangle, thumb: Option<&thumbs::Thumb>, texture: Option<(u64, &Texture2D)>) {
    d.draw_rectangle_rec(rect, Color::new(0, 0, 0, 110));
    let (Some(thumb), Some((stamp, texture))) = (thumb, texture) else { return };
    let (Some(image), Some(at)) = (thumb.image.as_ref(), chrome::fit_picture(rect, thumb.field)) else { return };
    if image.stamp == stamp {
        d.draw_texture_pro(texture, crate::minimap::source(thumb.field), at, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
    }
}

/// The atlas a material's icon comes from.
fn sheet_texture<'a>(textures: &EditorTextures<'a>, sheet: obstacle::Sheet) -> &'a Texture2D {
    match sheet {
        obstacle::Sheet::Walls => textures.obstacles,
        obstacle::Sheet::Props => textures.props,
        obstacle::Sheet::Trees => textures.trees,
        obstacle::Sheet::Target => textures.target,
        obstacle::Sheet::Towers => textures.towers,
        other => panic!("a map cell never draws from {other:?}"),
    }
}

/// The enemy side's marker colour - the red ground ring the game draws
/// under the enemy frog, so an `enemy_frog` cell reads the same here.
const ENEMY_RING_COLOR: Color = Color::new(230, 60, 60, 220);

const GATE_COLOR: Color = Color::ORANGE;

/// `alpha` of an opaque marker's own alpha: what a lifted or pasted
/// cell's marker is drawn at.
fn scaled(c: Color, alpha: u8) -> Color {
    Color::new(c.r, c.g, c.b, ((c.a as u32 * alpha as u32) / 255) as u8)
}

/// A flat red ring of radius `radius` centred on `center`, `alpha` of
/// its own strength.
fn draw_enemy_ring(d: &mut impl RaylibDraw, center: Position, radius: f32, alpha: u8) {
    d.draw_ring(center, radius * 0.75, radius, 0.0, 360.0, 24, scaled(ENEMY_RING_COLOR, alpha));
    d.draw_circle_v(center, radius * 0.75, scaled(Color::new(230, 60, 60, 50), alpha));
}

/// A player's team-coloured ring, the same shape as the enemy frog's red
/// one, under the start markers: a `start` cell reads as "a tank, the blue
/// one" and `start2` as the pink one, the colours the tanks will be.
fn draw_player_ring(d: &mut impl RaylibDraw, center: Position, radius: f32, player: usize, alpha: u8) {
    let c = crate::tank::team_color(player as u8);
    d.draw_ring(center, radius * 0.75, radius, 0.0, 360.0, 24, scaled(Color::new(c.r, c.g, c.b, 220), alpha));
    d.draw_circle_v(center, radius * 0.75, scaled(Color::new(c.r, c.g, c.b, 50), alpha));
}

/// A chevron of overall size `size` at `center` in `color`, its point
/// aimed along `inward` (a unit axis vector) - the direction a tank
/// rolling in through the gate travels.
fn draw_gate_chevron(d: &mut impl RaylibDraw, center: Position, size: f32, inward: Position, color: Color) {
    let half = size / 2.0 - 3.0;
    let side = Position::new(-inward.y, inward.x);
    let tip = center + inward * half;
    let tail = center - inward * (half * 0.4);
    let wing_a = tail + side * half;
    let wing_b = tail - side * half;
    d.draw_line_ex(wing_a, tip, 3.0, color);
    d.draw_line_ex(wing_b, tip, 3.0, color);
    d.draw_line_ex(center - inward * half, tip, 3.0, color);
}

impl FileRow {
    fn label(self) -> String {
        text().get(match self {
            FileRow::Load => keys::FILE_LOAD,
            FileRow::Save => keys::FILE_SAVE,
            FileRow::SaveAs => keys::FILE_SAVE_AS,
            FileRow::Revert => keys::FILE_REVERT,
            FileRow::Clear => keys::FILE_CLEAR,
        })
    }
}

impl MapTab {
    /// Whether a CLI flag outranks a field of the group at PLAY.
    fn overridden(self, o: CliOverrides) -> bool {
        self.rows(true).iter().flatten().any(|f| f.overridden(o))
    }
}

impl MapField {
    /// The words a control's label shows.
    fn label(self) -> String {
        match (self, self.label_key()) {
            (MapField::Sky(sky), _) => sky_label(sky),
            (_, Some(key)) => text().get(key),
            (_, None) => String::new(),
        }
    }

    /// A choice's `i`th option as its button shows it, in capitals.
    fn option_label(self, i: usize) -> String {
        self.option_text(&text(), i)
    }

    /// The steppers that count rather than choose: `-` and `+`, where a
    /// choice's are `<` and `>`.
    fn is_number(self) -> bool {
        matches!(self, MapField::Tanks | MapField::Waves | MapField::Size | MapField::Growth | MapField::Width | MapField::Height)
    }

    /// The chassis a TANK stepper has picked and the seat whose colours it
    /// is drawn in; `None` on `auto` and on every other field.
    fn chassis(self, s: &MapSettings) -> Option<(TankKind, u8)> {
        match self {
            MapField::Tank => s.tank.map(|kind| (kind, 0)),
            MapField::Tank2 => s.tank2.map(|kind| (kind, 1)),
            _ => None,
        }
    }

    /// A stepper's value, in the language on screen: a data name looked up
    /// by its family (`tank-scout`, `tier-heavy`), the word for `auto` where
    /// the map leaves it to the game - the short one in a row of three -
    /// or the map's size in cells (`size`, columns and rows).
    fn value(self, s: &MapSettings, size: (f32, f32)) -> String {
        let t = text();
        let auto = |v: Option<String>, short: bool| {
            v.unwrap_or_else(|| t.get(if short { keys::SETTINGS_AUTO_SHORT } else { keys::SETTINGS_AUTO }))
        };
        match self {
            MapField::Tanks => auto(s.tanks.map(|n| n.to_string()), false),
            MapField::Tank => auto(s.tank.map(|k| t.named("tank", k.name())), false),
            MapField::Tank2 => auto(s.tank2.map(|k| t.named("tank", k.name())), false),
            MapField::Waves => auto(s.waves.map(|n| n.to_string()), true),
            MapField::Size => auto(s.size.map(|n| n.to_string()), true),
            MapField::Growth => auto(s.growth.map(|n| n.to_string()), true),
            MapField::TierStart => auto(s.tier_start.map(|tier| t.named("tier", tier.name())), false),
            MapField::TierEnd => auto(s.tier_end.map(|tier| t.named("tier", tier.name())), false),
            MapField::Width => cells_text(size.0),
            MapField::Height => cells_text(size.1),
            MapField::Mission | MapField::Spawn | MapField::Theme | MapField::Anchor | MapField::Sky(_) => String::new(),
        }
    }

    /// Whether a CLI flag outranks this field's value at PLAY.
    fn overridden(self, o: CliOverrides) -> bool {
        match self {
            MapField::Tanks => o.tanks,
            MapField::Tank => o.tank,
            MapField::Tank2 => o.tank2,
            MapField::Mission => o.mission,
            MapField::Spawn => o.spawn,
            MapField::Waves => o.waves,
            MapField::Size => o.wave_size,
            MapField::Growth => o.wave_growth,
            MapField::TierStart => o.tier_start,
            MapField::TierEnd => o.tier_end,
            // No CLI flag names a theme: the map is the only source.
            MapField::Theme => false,
            // `--weather` (and the web page's `?weather=`) is the
            // `weather_override` knob, which outranks every map's sky.
            MapField::Sky(_) => crate::tuning::tuning().weather_override >= 0,
            // The size is the map's alone, and the anchor the panel's.
            MapField::Width | MapField::Height | MapField::Anchor => false,
        }
    }
}

/// A sky tile's name: the sky's in capitals (`sky_label_text`).
fn sky_label(sky: Weather) -> String {
    sky_label_text(&text(), sky)
}

/// A sky tile's swatch: a colour that reads as its sky at a glance.
fn sky_swatch(sky: Weather) -> Color {
    match sky {
        Weather::Clear | Weather::Random => Color::new(122, 178, 78, 255),
        Weather::Night => Color::new(28, 40, 72, 255),
        Weather::Dusk => Color::new(176, 124, 64, 255),
        Weather::Rain => Color::new(64, 104, 120, 255),
        Weather::Storm => Color::new(40, 48, 64, 255),
        Weather::Fog => Color::new(176, 188, 192, 255),
        Weather::Sandstorm => Color::new(212, 168, 96, 255),
        Weather::Snow => Color::new(232, 240, 244, 255),
        Weather::HeatHaze => Color::new(232, 196, 120, 255),
    }
}

/// A sky tile's check box in `rect`'s height, a filled square inside a
/// frame while `on`.
fn draw_check(d: &mut impl RaylibDraw, rect: Rectangle, on: bool) {
    let side = MAP_CHECK;
    let frame = Rectangle::new(rect.x, rect.y + (rect.height - side) / 2.0, side, side);
    d.draw_rectangle_lines_ex(frame, 2.0, if on { TEXT } else { DIM });
    if on {
        d.draw_rectangle_rec(Rectangle::new(frame.x + 4.0, frame.y + 4.0, side - 8.0, side - 8.0), BUILD_ACCENT);
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

    /// A category button's icon and caret, with a mouse and on a touch
    /// screen: the icon inside the icon half and inside the button's drawn
    /// box, the caret clear of it in the list half and inside the box, at
    /// the 38 pt a desktop's bar draws it at with a mouse, the icon
    /// centred top to bottom in the bar.
    #[test]
    fn a_category_buttons_icon_and_caret_stay_in_their_halves() {
        for touch in [false, true] {
            let ui = crate::hud::UiFrame::new((1600.0, 900.0), 1.0, 1.0, crate::hud::Insets::default(), touch);
            let bar = Bar::of(&ui);
            let button = bar.category(Category::Wall).expect("a wide bar has the five");
            let bx = button_box(button.rect, touch);
            let icon = icon_box(bx, true);
            let caret = caret_at(bx);
            assert!(icon.x >= button.icon.x && icon.x + icon.width <= button.icon.x + button.icon.width, "touch={touch}: {icon:?}");
            assert!(icon.y >= bx.y && icon.y + icon.height <= bx.y + bx.height, "touch={touch}: {icon:?}");
            assert!(caret.x >= icon.x + icon.width && caret.x >= button.list.x, "touch={touch}: the caret overlaps the icon");
            assert!(caret.x + chrome::CARET_W <= bx.x + bx.width - BOX_EDGE, "touch={touch}: the caret leaves the button");
            if !touch {
                assert_eq!(caret.x - button.rect.x, 38.0);
                let y = button.rect.y + (crate::HUD_BAR_HEIGHT as f32 - ICON_PX) / 2.0;
                assert_eq!(icon, Rectangle::new(button.rect.x, y, ICON_PX, ICON_PX));
            }
        }
    }

    /// Every button of the bar is drawn in one box: as tall as every
    /// other, at the same height in the bar, its slot less the same gap -
    /// with a mouse and on a touch screen, folded or not.
    #[test]
    fn every_bar_button_is_drawn_in_one_box() {
        for touch in [false, true] {
            for width in [1600.0, 720.0] {
                let ui = crate::hud::UiFrame::new((width, 900.0), 1.0, 1.0, crate::hud::Insets::default(), touch);
                let bar = Bar::of(&ui);
                let mut slots = vec![bar.erase, bar.undo, bar.redo, bar.file, bar.map, bar.fit, bar.check, bar.here, bar.play];
                match &bar.tools {
                    BarTools::Categories(buttons) => slots.extend(buttons.iter().map(|b| b.rect)),
                    BarTools::Folded(r) => slots.push(*r),
                }
                slots.extend(bar.brush);
                let first = button_box(slots[0], touch);
                for slot in slots {
                    let bx = button_box(slot, touch);
                    assert_eq!((bx.y, bx.height), (first.y, first.height), "touch={touch} width={width}: {slot:?}");
                    assert_eq!(slot.width - bx.width, chrome::BOX_GAP);
                }
            }
        }
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
