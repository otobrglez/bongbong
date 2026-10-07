//! The builder's chrome in UI points (docs/large-maps-follow-camera.md §8,
//! §9; docs/game-editor-fusion.md §7): the bar along the window's top
//! inside the safe area, the popups hanging from it, the status line, the
//! navigator and the loupe - laid out on the window from its size in points
//! the way play's corners are (`hud::UiFrame`), never at the canvas's
//! scale, so a phone's bar is a finger's height and its text no smaller
//! than a phone can read, while a desktop at UI scale 1 draws its bar a
//! point a pixel.
//!
//! `BuilderFrame` is where the builder stands on the window this frame:
//! the canvas's bitmap - the canvas alone, no bar - and the view that puts
//! it on the window under the bar, and the UI frame the chrome is laid out
//! in. `Bar` and `PopupLayout` (gathered by `MapEditor::chrome`) are the one
//! geometry table the painter (`render.rs`), every hit test (`mod.rs`), the
//! dev server's `status.builder.buttons` and `click`, and the web build's
//! `bb_ui_json` read.
//!
//! **The bar is laid out from the safe area's width** in points. With the
//! room - a desktop window, a tablet - it is one row: BUILD, the map's
//! name, the five category buttons and the BRUSH button (the brush's
//! shape, the select tool and the stamps), the eraser, UNDO, REDO, FILE,
//! MAP, FIT, CHECK and the clear flag from the left, PLAY HERE and PLAY at
//! the right end. Where it has less, the BUILD label goes first, then the
//! name (the status line names the map instead), and then the five
//! category buttons and BRUSH fold into one TOOLS button that opens a
//! palette of every category with the brush's row under them, so every
//! button stays reachable at its full size - 44 points on both sides on a
//! touch screen (`hud::UI_TOUCH_PT`) - rather than shrinking. A UI frame is
//! never narrower than `hud::UI_MIN_W` points (`UiFrame::new` shrinks the
//! point instead), and the folded bar fits that on a touch screen.
//!
//! **The popups take the room under the bar**: a list too long for it runs
//! on in a second column, and the MAP panel, the CHECK panel, the Load list
//! and the STAMPS list turn pages by a pager row - the touch screen's way,
//! since it has no wheel - where their rows do not fit.
//!
//! **The select tool's actions stand in a strip under the bar** (`Strip`):
//! COPY, CUT, PASTE, the two flips, DELETE, keeping the selection as a
//! stamp and the STAMPS list - or, while a paste ghost stands on the
//! canvas, PLACE, the flips and CANCEL - a finger's size on a touch screen,
//! on a plate from the room's top-left corner. Under the bar rather than
//! beside the selection: it never moves while a finger drags the selection
//! or pans and pinches the view, it never stands over the cells being
//! worked on (only a selection at the very top of the view meets it), and
//! eight finger-size buttons beside a selection would cover a good part of
//! a phone's canvas wherever the selection was.

use crate::framing::MapClass;
use crate::hud::{button_height, UiFrame, PLATE_PAD, UI_EDGE_PT};
use crate::math::{Rectangle, Vec2};
use crate::view::{ScaleCap, View};
use crate::{Layout, EDITOR_BAR_HIT_SLACK, EDITOR_DROPDOWN_ROW_H, EDITOR_DROPDOWN_W, HUD_BAR_HEIGHT};

use super::{BrushRow, Category, FieldKind, MapField, MapTab, PanelButton};

/// The text size of every word on a button of the bar - UNDO, REDO, FILE,
/// MAP, FIT, CHECK, PLAY HERE, PLAY - and of the clear flag's par, the
/// select tool's strip's words and the Save prompt's SAVE, with a mouse
/// and on a touch screen alike: 14 points, between the HUD's small words
/// (`hud::UI_SMALL_TEXT`, 12) and its corner buttons' 18. The bar is one
/// row at `hud::UI_MIN_W`, so its buttons cannot widen to the corners'
/// size, and every language's label fits its box at this one (`label_room`)
/// - still over 11 where the UI frame shrinks the point on the smallest
/// phones.
pub const BAR_SMALL_TEXT: i32 = 14;

/// The width a button's drawn box keeps short of its slot, so two
/// neighbours' outlines never touch: every button on the bar and on the
/// select tool's strip, and the Save prompt's SAVE.
pub const BOX_GAP: f32 = 4.0;

/// A button box's outline, drawn inside the box: 2 points of square
/// corners, as play's corner buttons are framed.
pub const BOX_EDGE: f32 = 2.0;

/// The least room a label, an icon or a caret keeps from its box's
/// outline.
pub const BOX_PAD: f32 = 2.0;

/// A caret's width (a down-pointing triangle of 2 pt blocks), and the room
/// between it and the word or icon before it.
pub const CARET_W: f32 = 10.0;
pub const CARET_GAP: f32 = 4.0;

/// A button box's height: the bar's icon box - 32 points with a mouse, 40
/// on a touch screen - so an icon's sprite (drawn 4 in) stands `BOX_PAD`
/// clear of the outline, and every button of the bar is as tall as every
/// other.
pub fn box_height(touch: bool) -> f32 {
    if touch { 40.0 } else { 32.0 }
}

/// The drawn box of a button standing in `slot`: the slot less `BOX_GAP`
/// at its right end, `box_height` tall (or the slot's height, where that
/// is less) and centred in it top to bottom on a whole point. One box for
/// every button, so none is taller, wider-framed or differently cornered
/// than the next.
pub fn button_box(slot: Rectangle, touch: bool) -> Rectangle {
    let h = box_height(touch).min(slot.height);
    Rectangle::new(slot.x, slot.y + ((slot.height - h) / 2.0).floor(), slot.width - BOX_GAP, h)
}

/// The width a label has in `bx`: inside its outline and `BOX_PAD` clear
/// of it either side, less the caret and its gap where the button has one.
pub fn label_room(bx: Rectangle, caret: bool) -> f32 {
    let caret = if caret { CARET_W + CARET_GAP } else { 0.0 };
    bx.width - 2.0 * (BOX_EDGE + BOX_PAD) - caret
}

/// Where a caret stands in `bx`: at its right end, `BOX_PAD` clear of the
/// outline, centred top to bottom.
pub fn caret_at(bx: Rectangle) -> Vec2 {
    Vec2::new(bx.x + bx.width - BOX_EDGE - BOX_PAD - CARET_W, (bx.y + (bx.height - 6.0) / 2.0).floor())
}

/// The icon box of a button whose box is `bx`: a square of its height,
/// at its left end where a caret follows it, else centred in it.
pub fn icon_box(bx: Rectangle, caret: bool) -> Rectangle {
    let side = bx.height;
    let x = if caret { bx.x } else { (bx.x + (bx.width - side) / 2.0).floor() };
    Rectangle::new(x, bx.y, side, side)
}

/// The bar's height: `HUD_BAR_HEIGHT` with a mouse - a play corner
/// plate's - and a finger's (`hud::UI_TOUCH_PT`) on a touch screen. The
/// select tool's strip's buttons are as tall.
pub fn bar_height(touch: bool) -> f32 {
    if touch { button_height(true) } else { HUD_BAR_HEIGHT as f32 }
}

/// The CHECK panel's width, in points: a finding's words, its place and
/// its FIX button, and the panel's two hint lines in the chrome's small
/// text in every language.
pub const LINT_PANEL_W: f32 = 500.0;

/// The most findings one page of the CHECK panel lists, where the room
/// under the bar holds them.
pub const LINT_PAGE_ROWS: usize = 7;

/// The CHECK panel's rows above its findings: the header and the clear
/// check.
pub const LINT_HEAD_ROWS: usize = 2;

/// The most rows the Load list shows at once (eight fit under the bar of a
/// desktop's window); fewer where the room under the bar holds fewer.
pub const LOAD_VISIBLE_ROWS: usize = 8;

/// The Load list's width.
pub const LOAD_PANEL_W: f32 = 400.0;

/// The box a map's thumbnail stands in at its Load row's left, 8 points
/// in and centred top to bottom (`load_picture`): a 16:9 map fills its
/// height, a 2:1 one its width.
pub const LOAD_PICTURE: (f32, f32) = (72.0, 40.0);

/// The room a Load row's text has (`load_text`): from 12 points past the
/// picture to 12 short of the row's right end. The name runs along its
/// top line, the map's size in cells and whether it ships with the game
/// along its second (`text_tests` holds the second line's words to it).
pub const LOAD_TEXT_W: f32 = LOAD_PANEL_W - 8.0 - LOAD_PICTURE.0 - 12.0 - 12.0;

/// How far down a Load row its name's line and its second line stand.
pub const LOAD_NAME_Y: f32 = 7.0;
pub const LOAD_DETAIL_Y: f32 = 29.0;

/// The STAMPS list's width: a stamp's picture, its name and its size.
pub const STAMPS_PANEL_W: f32 = 320.0;

/// The most rows the STAMPS list shows at once, as the Load list.
pub const STAMPS_VISIBLE_ROWS: usize = LOAD_VISIBLE_ROWS;

/// The box a stamp's picture stands in at its STAMPS row's left, 8 points
/// in.
pub const STAMP_PICTURE: (f32, f32) = (56.0, 40.0);

/// The room a stamp's name has in its row: from 12 points past its picture
/// to the size in cells at the row's right end (`text_tests` holds every
/// language's names to it).
pub const STAMP_NAME_W: f32 = STAMPS_PANEL_W - 8.0 - STAMP_PICTURE.0 - 12.0 - 52.0;

/// The Save prompt's size.
pub const SAVE_PROMPT: (f32, f32) = (300.0, 80.0);

/// The Save prompt's SAVE button's width (`save_button`).
pub const SAVE_BUTTON_W: f32 = 72.0;

/// How wide the Save prompt's name may run from its left inset: up to a
/// gap short of the SAVE button.
pub const SAVE_NAME_W: f32 = SAVE_PROMPT.0 - 12.0 - SAVE_BUTTON_W - 12.0 - 8.0;

/// The palette the folded TOOLS button opens: a row per category and the
/// brush's row, its name in a column of this width, then a square cell per
/// tool.
pub const PALETTE_LABEL_W: f32 = 72.0;

/// A palette cell: a dropdown row's height, square.
pub const PALETTE_CELL: f32 = EDITOR_DROPDOWN_ROW_H;

/// The least room the map's name keeps in the bar before it gives its
/// place up to the buttons: a few letters and the edited mark.
pub const NAME_MIN_W: f32 = 64.0;

/// The status line's text size and its rise from the area's bottom edge.
pub const STATUS_TEXT: i32 = 14;

/// The width of one of the bar's elements and the gap it keeps from the
/// next, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Slot {
    w: f32,
    gap: f32,
}

const fn slot(w: f32, gap: f32) -> Slot {
    Slot { w, gap }
}

/// PLAY HERE's width, on a mouse's bar and a touch screen's: its label in
/// its box in every language (`label_room`; `chrome_tests`, `text_tests`).
pub const HERE_W: f32 = 92.0;

/// Every element's slot, for a mouse or a touch screen. The mouse's are a
/// desktop bar's, a point a pixel, at the HUD's scale - a 40-point bar of
/// 32-point boxes, as a play corner's plate holds its buttons; a touch
/// screen's are at least `hud::UI_TOUCH_PT` across for every press, a
/// category button's two halves included. Every button's drawn box is its
/// slot less `BOX_GAP` (`button_box`), and the slots keep 4 points apart
/// from the TOOLS button to MAP, so the folded bar holds PLAY HERE whole
/// at `hud::UI_MIN_W`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Metrics {
    /// `BUILD`, the mode, in the builder's amber.
    label: Slot,
    /// The map's name and its edited mark.
    name: Slot,
    /// A category button: its icon half and its list half.
    category_icon: f32,
    category_list: f32,
    /// After the five category buttons.
    categories_gap: f32,
    /// BRUSH, after the five: the brush's shape, the select tool and the
    /// stamps (`BrushRow`).
    brush: Slot,
    /// The TOOLS button the five and BRUSH fold into.
    folded: Slot,
    erase: Slot,
    undo: Slot,
    redo: Slot,
    file: Slot,
    map: Slot,
    fit: Slot,
    check: Slot,
    clear: Slot,
    /// The least room between the clear flag and PLAY HERE.
    right_gap: f32,
    here: Slot,
    play: f32,
}

const MOUSE: Metrics = Metrics {
    label: slot(64.0, 0.0),
    name: slot(152.0, 8.0),
    category_icon: 36.0,
    category_list: 20.0,
    categories_gap: 8.0,
    brush: slot(56.0, 8.0),
    folded: slot(56.0, 4.0),
    erase: slot(44.0, 4.0),
    undo: slot(52.0, 4.0),
    redo: slot(52.0, 4.0),
    file: slot(60.0, 4.0),
    map: slot(60.0, 4.0),
    fit: slot(44.0, 4.0),
    check: slot(76.0, 2.0),
    clear: slot(52.0, 0.0),
    right_gap: 2.0,
    here: slot(HERE_W, 4.0),
    play: crate::hud::MODE_BUTTON_W,
};

const TOUCH: Metrics = Metrics {
    label: slot(64.0, 0.0),
    name: slot(152.0, 8.0),
    category_icon: 44.0,
    category_list: 44.0,
    categories_gap: 8.0,
    brush: slot(64.0, 8.0),
    folded: slot(64.0, 4.0),
    erase: slot(44.0, 4.0),
    undo: slot(48.0, 4.0),
    redo: slot(48.0, 4.0),
    file: slot(60.0, 4.0),
    map: slot(60.0, 4.0),
    fit: slot(44.0, 4.0),
    check: slot(76.0, 2.0),
    clear: slot(52.0, 0.0),
    right_gap: 2.0,
    here: slot(HERE_W, 4.0),
    play: crate::hud::MODE_BUTTON_W,
};

fn metrics(touch: bool) -> &'static Metrics {
    if touch { &TOUCH } else { &MOUSE }
}

/// One category button: the whole of it, the icon half that selects the
/// category's current tool and the list half that opens its list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CategoryButton {
    pub rect: Rectangle,
    pub icon: Rectangle,
    pub list: Rectangle,
}

/// The tool categories in the bar: a button each, or folded into one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BarTools {
    /// The five category buttons, in `Category::ALL` order; BRUSH follows
    /// them (`Bar::brush`).
    Categories([CategoryButton; 5]),
    /// One TOOLS button opening the palette of every category and the
    /// brush's row: what a bar too narrow for the five and BRUSH shows.
    Folded(Rectangle),
}

/// The builder's bar in UI points: the strip along the window's top, under
/// the safe area's top edge (and the web page's band, which the UI frame
/// counts as one), across the window's whole width, and every element on
/// it inside the safe area. `Bar::of` lays it out; the painter, the hit
/// tests and the dev server read it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    pub strip: Rectangle,
    /// The BUILD label, where the bar has the room.
    pub label: Option<Rectangle>,
    /// The map's name, where the bar has the room.
    pub name: Option<Rectangle>,
    pub tools: BarTools,
    /// BRUSH, after the five category buttons; `None` on a folded bar,
    /// whose palette carries the brush's row instead.
    pub brush: Option<Rectangle>,
    pub erase: Rectangle,
    pub undo: Rectangle,
    pub redo: Rectangle,
    pub file: Rectangle,
    pub map: Rectangle,
    pub fit: Rectangle,
    pub check: Rectangle,
    pub clear: Rectangle,
    pub here: Rectangle,
    pub play: Rectangle,
    /// Laid out for a touch screen.
    pub touch: bool,
}

/// What a press on the bar lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarButton {
    /// The icon half of a category button: select its current tool.
    CategoryIcon(Category),
    /// The list half: open its list.
    CategoryMenu(Category),
    /// The folded TOOLS button: open the palette.
    Tools,
    /// BRUSH: open its list (`BrushRow`).
    Brush,
    Erase,
    Undo,
    Redo,
    Map,
    File,
    /// The camera back to the whole canvas.
    Fit,
    /// The CHECK panel, from its button or the clear check's readout.
    Check,
    Play,
    /// PLAY from the middle of the view.
    PlayHere,
}

impl Bar {
    /// The strip alone: across the window, from the safe area's top edge,
    /// `bar_height` tall - 44 points on a touch screen and 40 with a
    /// mouse.
    pub fn strip_of(ui: &UiFrame) -> Rectangle {
        Rectangle::new(0.0, ui.area.y - UI_EDGE_PT, ui.screen.w, bar_height(ui.touch))
    }

    /// The bar laid out in `ui` (see the module docs): PLAY at the safe
    /// area's right end with PLAY HERE before it; from the left the buttons
    /// in a row, the five category buttons and BRUSH folded into TOOLS
    /// where the row would not reach PLAY HERE with them, then the map's
    /// name and the BUILD label in what room is left, each only whole.
    pub fn of(ui: &UiFrame) -> Bar {
        let m = metrics(ui.touch);
        let strip = Self::strip_of(ui);
        let (top, h) = (strip.y, strip.height);
        let left = ui.area.x;
        let right = ui.area.x + ui.area.w;
        let at = |x: f32, w: f32| Rectangle::new(x, top, w, h);

        let play = at(right - m.play, m.play);
        let here = at(play.x - m.here.gap - m.here.w, m.here.w);
        // The row from the eraser to the clear flag, whatever comes first.
        let after: f32 = [m.erase, m.undo, m.redo, m.file, m.map, m.fit, m.check, m.clear].iter().map(|s| s.w + s.gap).sum();
        let room = here.x - m.right_gap - left;
        let categories = Category::ALL.len() as f32 * (m.category_icon + m.category_list) + m.categories_gap + m.brush.w + m.brush.gap;
        let folded = categories + after > room;
        let tools_w = if folded { m.folded.w + m.folded.gap } else { categories };
        let spare = room - tools_w - after;
        let label = spare >= m.label.w + m.label.gap + m.name.w + m.name.gap;
        let name_w = if spare >= m.name.w + m.name.gap {
            Some(m.name.w)
        } else if spare >= NAME_MIN_W + m.name.gap {
            Some(spare - m.name.gap)
        } else {
            None
        };

        let mut x = left;
        let label = label.then(|| {
            let r = at(x, m.label.w);
            x += m.label.w + m.label.gap;
            r
        });
        let name = name_w.map(|w| {
            let r = at(x, w);
            x += w + m.name.gap;
            r
        });
        let (tools, brush) = if folded {
            let r = at(x, m.folded.w);
            x += m.folded.w + m.folded.gap;
            (BarTools::Folded(r), None)
        } else {
            let w = m.category_icon + m.category_list;
            let buttons = std::array::from_fn(|i| {
                let bx = x + i as f32 * w;
                CategoryButton { rect: at(bx, w), icon: at(bx, m.category_icon), list: at(bx + m.category_icon, m.category_list) }
            });
            let brush = at(x + Category::ALL.len() as f32 * w + m.categories_gap, m.brush.w);
            x += categories;
            (BarTools::Categories(buttons), Some(brush))
        };
        let mut next = |s: Slot| {
            let r = at(x, s.w);
            x += s.w + s.gap;
            r
        };
        let (erase, undo, redo, file, map, fit, check, clear) =
            (next(m.erase), next(m.undo), next(m.redo), next(m.file), next(m.map), next(m.fit), next(m.check), next(m.clear));
        Bar { strip, label, name, tools, brush, erase, undo, redo, file, map, fit, check, clear, here, play, touch: ui.touch }
    }

    /// The category button of `category`, while the five are in the bar.
    pub fn category(&self, category: Category) -> Option<CategoryButton> {
        match &self.tools {
            BarTools::Categories(buttons) => Some(buttons[category.index()]),
            BarTools::Folded(_) => None,
        }
    }

    /// The rect a popup of `category`'s tools hangs from: its category
    /// button, or the folded TOOLS button.
    pub fn tools_anchor(&self, category: Category) -> Rectangle {
        match &self.tools {
            BarTools::Categories(buttons) => buttons[category.index()].rect,
            BarTools::Folded(r) => *r,
        }
    }

    /// Every press target with what it is, in the order a press is tested.
    pub fn buttons(&self) -> Vec<(BarButton, Rectangle)> {
        let mut out = vec![(BarButton::Play, self.play), (BarButton::File, self.file)];
        match &self.tools {
            BarTools::Categories(buttons) => {
                for (category, b) in Category::ALL.into_iter().zip(buttons) {
                    out.push((BarButton::CategoryIcon(category), b.icon));
                    out.push((BarButton::CategoryMenu(category), b.list));
                }
            }
            BarTools::Folded(r) => out.push((BarButton::Tools, *r)),
        }
        out.extend(self.brush.map(|r| (BarButton::Brush, r)));
        out.extend([
            (BarButton::Erase, self.erase),
            (BarButton::Undo, self.undo),
            (BarButton::Redo, self.redo),
            (BarButton::Map, self.map),
            (BarButton::Fit, self.fit),
            (BarButton::Check, self.check),
            (BarButton::Check, self.clear),
            (BarButton::PlayHere, self.here),
        ]);
        out
    }

    /// The button under `p` (UI points). Every hit rect reaches
    /// `EDITOR_BAR_HIT_SLACK` above and below the button's drawn box
    /// (docs/game-editor-fusion.md section 10), so a slightly low tap still
    /// lands.
    pub fn hit(&self, p: Vec2) -> Option<BarButton> {
        self.buttons().into_iter().find(|(_, r)| hit_rect(*r).contains(p)).map(|(b, _)| b)
    }

    /// The category button `p` is over, either half - where the wheel
    /// steps that category's tool.
    pub fn category_at(&self, p: Vec2) -> Option<Category> {
        Category::ALL.into_iter().find(|&c| self.category(c).is_some_and(|b| hit_rect(b.rect).contains(p)))
    }

    /// The bar's buttons a tool presses by name (`status.builder.buttons`).
    pub fn named(&self) -> Vec<(String, Rectangle)> {
        let mut out: Vec<(String, Rectangle)> = vec![
            ("play".into(), self.play),
            ("play_here".into(), self.here),
            ("check".into(), self.check),
            ("clear".into(), self.clear),
            ("fit".into(), self.fit),
            ("map".into(), self.map),
            ("file".into(), self.file),
            ("erase".into(), self.erase),
            ("undo".into(), self.undo),
            ("redo".into(), self.redo),
        ];
        match &self.tools {
            BarTools::Categories(buttons) => {
                for (category, b) in Category::ALL.into_iter().zip(buttons) {
                    out.push((format!("category_{}", category.name()), b.icon));
                    out.push((format!("list_{}", category.name()), b.list));
                }
            }
            BarTools::Folded(r) => out.push(("tools".into(), *r)),
        }
        out.extend(self.brush.map(|r| ("brush".to_string(), r)));
        out
    }
}

/// A bar button's hit rect: its drawn box plus `EDITOR_BAR_HIT_SLACK`
/// above and below.
pub fn hit_rect(rect: Rectangle) -> Rectangle {
    Rectangle::new(rect.x, rect.y - EDITOR_BAR_HIT_SLACK, rect.width, rect.height + 2.0 * EDITOR_BAR_HIT_SLACK)
}

/// Where the builder stands on the window this frame: the canvas's bitmap
/// (`layout` - its field area the canvas area, with no bar in it), the view
/// that puts that bitmap on the window under the bar, and the UI frame the
/// chrome is laid out in. A pointer reaches the canvas through `view`
/// (`to_canvas`) and the chrome through `ui` (`to_ui`); `BuilderInput`
/// carries the window's own coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuilderFrame {
    pub layout: Layout,
    pub view: View,
    pub ui: UiFrame,
}

impl BuilderFrame {
    /// The builder on the window `ui` describes, for a map of `field` world
    /// pixels and class `class`: the bar along the top (`Bar::strip_of`),
    /// and under it the canvas, between the safe area's sides - so a notch
    /// or a rounded corner never stands over the map's edge columns - down
    /// to the window's bottom. An arena's bitmap is its field, fitted into
    /// that region at the scale a round's would be (`cap`) and letterboxed
    /// - at FIT the very picture a round draws of it. A field map's bitmap
    /// is made to the region's shape (`canvas_frame`) and the builder's
    /// camera chooses what of the map it shows.
    pub fn new(ui: UiFrame, field: (f32, f32), class: MapClass, cap: Option<ScaleCap>) -> BuilderFrame {
        let window = (ui.screen.w * ui.scale, ui.screen.h * ui.scale);
        let strip = Bar::strip_of(&ui);
        let top = (strip.y + strip.height) * ui.scale;
        let left = (ui.area.x - UI_EDGE_PT) * ui.scale;
        let right = (ui.area.x + ui.area.w + UI_EDGE_PT) * ui.scale;
        let region = ((right - left).max(1.0), (window.1 - top).max(1.0));
        let (layout, view) = if class.follows() {
            canvas_frame(region, cap)
        } else {
            (Layout::bare(field.0, field.1), View::fit_capped(field, region, cap))
        };
        let view = View { window, offset: Vec2::new(view.offset.x + left, view.offset.y + top), ..view };
        BuilderFrame { layout, view, ui }
    }

    /// The builder in a window of the map's field and a 40 px bar
    /// (`HUD_BAR_HEIGHT`) over it, a unit a point, nothing inset, no
    /// touch: what a dev server with no window lays the builder out in, and
    /// the tests. A field of at
    /// least `hud::UI_MIN_W` x `UI_MIN_H` less the bar (720 x 312) - the
    /// standard arena's, say - stands at (0, 40) at its own size; a smaller
    /// one shrinks the UI point (`UiFrame::new`), so its bar is shorter and
    /// the field is centred in the room left under it.
    pub fn headless(field: (f32, f32), class: MapClass) -> BuilderFrame {
        let window = (field.0.max(1.0), field.1.max(1.0) + HUD_BAR_HEIGHT as f32);
        BuilderFrame::new(UiFrame::plain(window), field, class, None)
    }

    /// A window point on the canvas's bitmap.
    pub fn to_canvas(&self, window: Vec2) -> Vec2 {
        self.view.to_bitmap(window)
    }

    /// A window point in UI points.
    pub fn to_ui(&self, window: Vec2) -> Vec2 {
        self.ui.to_ui(window)
    }

    /// A point of the canvas's bitmap in UI points.
    pub fn canvas_to_ui(&self, bitmap: Vec2) -> Vec2 {
        self.ui.to_ui(self.view.to_window(bitmap))
    }

    /// A UI point on the canvas's bitmap.
    pub fn ui_to_canvas(&self, ui: Vec2) -> Vec2 {
        self.view.to_bitmap(self.ui.to_window(ui))
    }

    /// The bar, laid out in this frame's UI.
    pub fn bar(&self) -> Bar {
        Bar::of(&self.ui)
    }

    /// The canvas area on the window, in UI points.
    pub fn canvas_ui(&self) -> Rectangle {
        let f = self.layout.field;
        let a = self.canvas_to_ui(Vec2::new(f.x, f.y));
        let b = self.canvas_to_ui(Vec2::new(f.x + f.w, f.y + f.h));
        Rectangle::new(a.x, a.y, b.x - a.x, b.y - a.y)
    }

    /// The room under the bar, inside the safe area: what the popups hang
    /// in and centre in, and where the status line and the navigator stand.
    pub fn under_bar(&self) -> Rectangle {
        let strip = Bar::strip_of(&self.ui);
        let a = self.ui.area;
        let top = strip.y + strip.height;
        Rectangle::new(a.x, top, a.w, (a.y + a.h - top).max(0.0))
    }

    /// Device pixels per UI point, given the canvas's device pixels per
    /// bitmap pixel (`CanvasScreen::device_per_px`).
    pub fn device_per_point(&self, device_per_px: f32) -> f32 {
        let v = device_per_px / self.view.scale.max(1e-6) * self.ui.scale;
        if v.is_finite() && v > 0.0 { v } else { 1.0 }
    }
}

/// How a field map's canvas lands under the bar in `region` (the window
/// under the bar, in its units): a bitmap made to the region's shape - no
/// letterbox - at the scale the standard field would be fitted into it
/// (`View::fit_capped` under the same `cap`), so a bitmap pixel is the same
/// size over a field map as over an arena in the same window. The view
/// puts it at the region's corner; `BuilderFrame::new` moves it under the
/// bar.
pub fn canvas_frame(region: (f32, f32), cap: Option<ScaleCap>) -> (Layout, View) {
    let standard = (crate::DEFAULT_SCREEN_WIDTH as f32, crate::DEFAULT_SCREEN_HEIGHT as f32);
    let scale = View::fit_capped(standard, region, cap).scale;
    let width = (region.0.max(1.0) / scale).floor().max(1.0);
    let height = (region.1.max(1.0) / scale).floor().max(1.0);
    (Layout::bare(width, height), View::fill((width, height), region))
}

/// The builder's chrome on one frame, in UI points (`MapEditor::chrome`):
/// the bar, the room under it, the open popup's layout and, with the select
/// tool and no popup open, the action strip.
#[derive(Clone, Debug, PartialEq)]
pub struct Chrome {
    pub bar: Bar,
    /// The room under the bar inside the safe area (`BuilderFrame::under_bar`).
    pub room: Rectangle,
    pub popup: Option<PopupLayout>,
    pub strip: Option<Strip>,
}

/// Where the open popup's panel and rows stand.
#[derive(Clone, Debug, PartialEq)]
pub enum PopupLayout {
    /// A category's tool list, a row per tool.
    Dropdown(Category, Rows),
    /// BRUSH's list, a row per `BrushRow`.
    Brush(Rows),
    /// The folded bar's palette of every category and the brush's row.
    Palette(Palette),
    /// The FILE menu, a row per `FileRow`.
    File(Rows),
    Load(LoadLayout),
    /// The STAMPS list, the Load list's shape at `STAMPS_PANEL_W`.
    Stamps(LoadLayout),
    /// The Save prompt's panel.
    Save(Rectangle),
    Settings(MapPanel),
    Lint(LintLayout),
}

impl PopupLayout {
    /// The popup's panel: a press inside it is the popup's.
    pub fn panel(&self) -> Rectangle {
        match self {
            PopupLayout::Dropdown(_, rows) | PopupLayout::Brush(rows) | PopupLayout::File(rows) => rows.panel,
            PopupLayout::Palette(palette) => palette.panel,
            PopupLayout::Load(list) | PopupLayout::Stamps(list) => list.rows.panel,
            PopupLayout::Save(panel) => *panel,
            PopupLayout::Settings(panel) => panel.panel,
            PopupLayout::Lint(lint) => lint.panel,
        }
    }
}

/// A popup's rows: a panel of columns, each `per_column` rows of
/// `row_w` x `row_h` points, filled down then across.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rows {
    pub panel: Rectangle,
    pub per_column: usize,
    pub row_w: f32,
    pub row_h: f32,
}

impl Rows {
    /// The `i`th row, down the first column and on into the next.
    pub fn row(&self, i: usize) -> Rectangle {
        let per = self.per_column.max(1);
        let (column, row) = (i / per, i % per);
        Rectangle::new(self.panel.x + column as f32 * self.row_w, self.panel.y + row as f32 * self.row_h, self.row_w, self.row_h)
    }
}

/// A pager: a row along a panel's bottom whose left half pages back and
/// right half on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pager {
    pub row: Rectangle,
}

impl Pager {
    pub fn back(&self) -> Rectangle {
        Rectangle::new(self.row.x, self.row.y, self.row.width / 2.0, self.row.height)
    }

    pub fn next(&self) -> Rectangle {
        let half = self.row.width / 2.0;
        Rectangle::new(self.row.x + half, self.row.y, self.row.width - half, self.row.height)
    }
}

/// `w` points from `x`, slid left to end inside `room` and kept from its
/// left edge.
fn slide(x: f32, w: f32, room: Rectangle) -> f32 {
    x.min(room.x + room.width - w).max(room.x)
}

/// Whole rows of `row_h` the room under the bar holds, at least one.
fn rows_in(room: Rectangle, row_h: f32) -> usize {
    ((room.height / row_h).floor() as usize).max(1)
}

/// A list of `n` rows of `w` x `EDITOR_DROPDOWN_ROW_H` hanging from
/// `anchor` under the bar: one column where the room holds it, else as
/// many columns as it takes, the rows shared out evenly; where those
/// columns would leave the room's width, the rows shorten - never under a
/// finger's `UI_TOUCH_PT` - until the columns the room is wide for hold
/// them. Slid to stay in the room.
pub fn hanging_list(anchor: Rectangle, room: Rectangle, n: usize, w: f32) -> Rows {
    let wide = ((room.width / w).floor() as usize).max(1);
    let (h, max) = if n.max(1).div_ceil(rows_in(room, EDITOR_DROPDOWN_ROW_H)) > wide {
        let rows = n.max(1).div_ceil(wide);
        let h = (room.height / rows as f32).clamp(crate::hud::UI_TOUCH_PT, EDITOR_DROPDOWN_ROW_H);
        (h, if h * rows as f32 <= room.height + 1e-3 { rows } else { rows_in(room, h) })
    } else {
        (EDITOR_DROPDOWN_ROW_H, rows_in(room, EDITOR_DROPDOWN_ROW_H))
    };
    let columns = n.max(1).div_ceil(max);
    let per_column = n.max(1).div_ceil(columns);
    let width = columns as f32 * w;
    Rows { panel: Rectangle::new(slide(anchor.x, width, room), room.y, width, per_column as f32 * h), per_column, row_w: w, row_h: h }
}

/// The palette the folded TOOLS button opens: a row per category - its
/// name, then a cell per tool - and under them the brush's row, a cell per
/// `BrushRow`, hanging from the button. A category with more tools than the
/// room is wide for runs on into a second row under its first, so the
/// palette never leaves the room's width; where its rows would not fit the
/// room's height the cells shrink to fit, never under a finger's
/// `UI_TOUCH_PT`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub panel: Rectangle,
    /// Tool cells a row holds.
    per_row: usize,
    /// A cell's side: `PALETTE_CELL`, less where the room is short.
    cell: f32,
}

impl Palette {
    /// The palette hanging from `anchor` in `room`, as wide as its longest
    /// row needs and the room allows.
    pub fn of(anchor: Rectangle, room: Rectangle) -> Palette {
        let most = Category::ALL.iter().map(|c| c.tools().count()).max().unwrap_or(1).max(BrushRow::ALL.len());
        let fits = (((room.width - PALETTE_LABEL_W) / PALETTE_CELL).floor().max(1.0) as usize).max(BrushRow::ALL.len());
        let per_row = most.min(fits);
        let rows: usize = Category::ALL.iter().map(|&c| Self::rows_of(c, per_row)).sum::<usize>() + 1;
        let cell = PALETTE_CELL.min((room.height / rows as f32).floor()).max(crate::hud::UI_TOUCH_PT);
        let w = PALETTE_LABEL_W + per_row as f32 * cell;
        let h = rows as f32 * cell;
        Palette { panel: Rectangle::new(slide(anchor.x, w, room), room.y, w, h), per_row, cell }
    }

    /// Rows `category` takes at `per_row` cells a row.
    fn rows_of(category: Category, per_row: usize) -> usize {
        category.tools().count().div_ceil(per_row.max(1)).max(1)
    }

    /// Row `index`: the categories', then the brush's.
    fn row_at(&self, index: usize) -> Rectangle {
        Rectangle::new(self.panel.x, self.panel.y + index as f32 * self.cell, self.panel.width, self.cell)
    }

    /// The index of `category`'s first row.
    fn first_row(&self, category: Category) -> usize {
        Category::ALL.iter().take_while(|&&c| c != category).map(|&c| Self::rows_of(c, self.per_row)).sum()
    }

    /// The category's first row.
    pub fn row(&self, category: Category) -> Rectangle {
        self.row_at(self.first_row(category))
    }

    /// The category's name, at its first row's left.
    pub fn label(&self, category: Category) -> Rectangle {
        let row = self.row(category);
        Rectangle::new(row.x, row.y, PALETTE_LABEL_W, row.height)
    }

    /// The category's `i`th tool's cell, running on into the category's
    /// next row past the room's width.
    pub fn cell(&self, category: Category, i: usize) -> Rectangle {
        let per_row = self.per_row.max(1);
        let row = self.row_at(self.first_row(category) + i / per_row);
        Rectangle::new(row.x + PALETTE_LABEL_W + (i % per_row) as f32 * self.cell, row.y, self.cell, self.cell)
    }

    /// The brush's row, under the categories'.
    pub fn brush_row(&self) -> Rectangle {
        self.row_at(Category::ALL.iter().map(|&c| Self::rows_of(c, self.per_row)).sum())
    }

    /// The brush row's name, at its left.
    pub fn brush_label(&self) -> Rectangle {
        let row = self.brush_row();
        Rectangle::new(row.x, row.y, PALETTE_LABEL_W, row.height)
    }

    /// The brush row's `i`th cell (`BrushRow::ALL`).
    pub fn brush_cell(&self, i: usize) -> Rectangle {
        let row = self.brush_row();
        Rectangle::new(row.x + PALETTE_LABEL_W + i as f32 * self.cell, row.y, self.cell, self.cell)
    }
}

/// The MAP panel's one geometry table (docs/game-editor-fusion.md section
/// 9): the map file's keys in four groups (`MapTab`) and RESET MAP. Where
/// the room under the bar holds them all - a desktop, a tablet - every group
/// stands at once as a section with its heading, ROUND and TANKS in the
/// left column, FIELD, SKY and RESET MAP in the right (`MapShape::Sections`);
/// where it does not - a phone, a small window - a rail of tabs down the
/// left shows one group at a time, RESET MAP its last tab
/// (`MapShape::Tabs`). A row is one control with its label beside it, or
/// several side by side with their labels over them; every button is a
/// finger's size. The groups below a row never move when the spawn plan
/// changes what that row holds: ROUND keeps the room its wave rows take.
#[derive(Clone, Debug, PartialEq)]
pub struct MapPanel {
    pub panel: Rectangle,
    pub shape: MapShape,
    /// The group the rail shows (`MapShape::Tabs`); every group is shown
    /// as sections.
    pub tab: MapTab,
    /// The rail's tabs, `MapTab::ALL` top to bottom (none as sections).
    pub tabs: Vec<(MapTab, Rectangle)>,
    /// Each group's heading line (none behind tabs).
    pub headings: Vec<(MapTab, Rectangle)>,
    pub reset: Rectangle,
    /// Every control shown.
    pub fields: Vec<PlacedField>,
}

/// How the MAP panel is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapShape {
    Sections,
    Tabs,
}

/// One control of the MAP panel: its box and, where its row has one, its
/// label's - beside it for a control alone in its row, over it for one of
/// several.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedField {
    pub field: MapField,
    pub rect: Rectangle,
    pub label: Option<Rectangle>,
}

/// A group's column, and the room the rail leaves for the group it shows.
pub const MAP_COLUMN_W: f32 = 420.0;

/// A label beside its control.
pub const MAP_LABEL_W: f32 = 104.0;

/// A label over its control, in a row of several.
pub const MAP_ABOVE_H: f32 = 16.0;

/// The rail of tabs down a short panel's left.
pub const MAP_RAIL_W: f32 = 120.0;

/// A group's heading line, as sections.
pub const MAP_HEADING_H: f32 = 24.0;

/// A stepper's `-` or `+`, at either end of its box.
pub const MAP_STEP_W: f32 = crate::hud::UI_TOUCH_PT;

/// Between a choice's options.
pub const MAP_SEGMENT_GAP: f32 = 4.0;

/// Between the controls of a row and between rows.
pub const MAP_ROW_GAP: f32 = 6.0;

/// The panel's text: a label beside its control, an option's or a
/// stepper's value, a group's heading; a label over its control is the
/// chrome's small text (`hud::UI_SMALL_TEXT`).
pub const MAP_LABEL_SIZE: i32 = 16;
pub const MAP_VALUE_SIZE: i32 = 16;
pub const MAP_HEADING_SIZE: i32 = 16;

/// A sky tile's name, and the swatch before it and the check box after,
/// each 8 pt in from the next.
pub const MAP_SKY_SIZE: i32 = 14;
pub const MAP_SWATCH: f32 = 24.0;
pub const MAP_CHECK: f32 = 16.0;

const MAP_PAD: f32 = 12.0;
const MAP_GUTTER: f32 = 20.0;
const MAP_GROUP_GAP: f32 = 14.0;

impl MapPanel {
    /// The panel hanging from `anchor` in `room`, showing `tab` where it is
    /// a rail's, with the wave rows where the spawn plan is `waves`.
    pub fn of(anchor: Rectangle, room: Rectangle, tab: MapTab, waves: bool) -> MapPanel {
        let (sections_w, sections_h) = Self::sections_size();
        if room.width >= sections_w && room.height >= sections_h {
            let panel = Rectangle::new(slide(anchor.x, sections_w, room), room.y, sections_w, sections_h);
            let left = panel.x + MAP_PAD;
            let right = left + MAP_COLUMN_W + MAP_GUTTER;
            let mut headings = Vec::new();
            let mut fields = Vec::new();
            let mut y = panel.y + MAP_PAD;
            for group in [MapTab::Round, MapTab::Tanks] {
                headings.push((group, Rectangle::new(left, y, MAP_COLUMN_W, MAP_HEADING_H)));
                place_group(group, waves, left, y + MAP_HEADING_H, &mut fields);
                y += MAP_HEADING_H + group_height(group, true) + MAP_GROUP_GAP;
            }
            let mut y = panel.y + MAP_PAD;
            for group in [MapTab::Field, MapTab::Sky] {
                headings.push((group, Rectangle::new(right, y, MAP_COLUMN_W, MAP_HEADING_H)));
                place_group(group, waves, right, y + MAP_HEADING_H, &mut fields);
                y += MAP_HEADING_H + group_height(group, true) + MAP_GROUP_GAP;
            }
            let h = EDITOR_DROPDOWN_ROW_H;
            let reset = Rectangle::new(right, panel.y + panel.height - MAP_PAD - h, MAP_COLUMN_W, h);
            return MapPanel { panel, shape: MapShape::Sections, tab, tabs: Vec::new(), headings, reset, fields };
        }
        let (w, h) = Self::tabs_size();
        let panel = Rectangle::new(slide(anchor.x, w, room), room.y, w, h);
        let slots = MapTab::ALL.len() + 1;
        let slot_h = h / slots as f32;
        let slot = |i: usize| Rectangle::new(panel.x, panel.y + i as f32 * slot_h, MAP_RAIL_W, slot_h);
        let tabs = MapTab::ALL.iter().enumerate().map(|(i, t)| (*t, slot(i))).collect();
        let mut fields = Vec::new();
        place_group(tab, waves, panel.x + MAP_RAIL_W + MAP_PAD, panel.y + MAP_PAD, &mut fields);
        MapPanel { panel, shape: MapShape::Tabs, tab, tabs, headings: Vec::new(), reset: slot(slots - 1), fields }
    }

    /// Every group at once: two columns, ROUND and TANKS beside FIELD, SKY
    /// and RESET MAP.
    pub fn sections_size() -> (f32, f32) {
        let column = |groups: &[MapTab]| groups.iter().map(|g| MAP_HEADING_H + group_height(*g, true)).sum::<f32>() + (groups.len() - 1) as f32 * MAP_GROUP_GAP;
        let left = column(&[MapTab::Round, MapTab::Tanks]);
        let right = column(&[MapTab::Field, MapTab::Sky]) + MAP_GROUP_GAP + EDITOR_DROPDOWN_ROW_H;
        (2.0 * MAP_PAD + 2.0 * MAP_COLUMN_W + MAP_GUTTER, 2.0 * MAP_PAD + left.max(right))
    }

    /// One group at a time beside the rail: as tall as the tallest group,
    /// and never so short a tab is under a finger's size.
    pub fn tabs_size() -> (f32, f32) {
        let group = MapTab::ALL.iter().map(|g| group_height(*g, true)).fold(0.0, f32::max) + 2.0 * MAP_PAD;
        let rail = (MapTab::ALL.len() + 1) as f32 * EDITOR_DROPDOWN_ROW_H;
        (MAP_RAIL_W + 2.0 * MAP_PAD + MAP_COLUMN_W, group.max(rail))
    }

    /// Every button the panel holds, as the painter draws it and every hit
    /// test reads it: the rail's tabs, RESET MAP, then each control's -
    /// a choice's options, a stepper's `-` and `+`, a sky's tile.
    pub fn buttons(&self) -> Vec<(PanelButton, Rectangle)> {
        let mut out: Vec<(PanelButton, Rectangle)> = self.tabs.iter().map(|(t, r)| (PanelButton::Tab(*t), *r)).collect();
        out.push((PanelButton::Reset, self.reset));
        for placed in &self.fields {
            match placed.field.kind() {
                FieldKind::Choice(n) => {
                    for (i, r) in segments(placed.rect, n).into_iter().enumerate() {
                        out.push((PanelButton::Option(placed.field, i), r));
                    }
                }
                FieldKind::Stepper | FieldKind::Chassis(_) => {
                    out.push((PanelButton::Dec(placed.field), stepper_dec(placed.rect)));
                    out.push((PanelButton::Inc(placed.field), stepper_inc(placed.rect)));
                }
                FieldKind::Sky(sky) => out.push((PanelButton::Toggle(sky), placed.rect)),
            }
        }
        out
    }
}

/// The height a group's rows take, its wave rows in or not.
fn group_height(group: MapTab, waves: bool) -> f32 {
    let rows = group.rows(waves);
    rows.iter().map(|r| row_height(r)).sum::<f32>() + rows.len().saturating_sub(1) as f32 * MAP_ROW_GAP
}

/// A row: a control alone with its label beside it, or several with
/// their labels over them (a sky's tile carries its own name).
fn row_height(row: &[MapField]) -> f32 {
    if labels_above(row) { MAP_ABOVE_H + EDITOR_DROPDOWN_ROW_H } else { EDITOR_DROPDOWN_ROW_H }
}

fn labels_above(row: &[MapField]) -> bool {
    row.len() > 1 && !matches!(row[0], MapField::Sky(_))
}

/// Lay `group`'s rows down from `(x, y)` in a column `MAP_COLUMN_W` wide.
fn place_group(group: MapTab, waves: bool, x: f32, mut y: f32, out: &mut Vec<PlacedField>) {
    let h = EDITOR_DROPDOWN_ROW_H;
    for row in group.rows(waves) {
        if row.len() == 1 && !matches!(row[0], MapField::Sky(_)) {
            let label = Rectangle::new(x, y, MAP_LABEL_W, h);
            out.push(PlacedField { field: row[0], rect: Rectangle::new(x + MAP_LABEL_W, y, MAP_COLUMN_W - MAP_LABEL_W, h), label: Some(label) });
        } else {
            let above = if labels_above(&row) { MAP_ABOVE_H } else { 0.0 };
            let n = row.len() as f32;
            let w = (MAP_COLUMN_W - (n - 1.0) * MAP_ROW_GAP) / n;
            for (i, field) in row.iter().enumerate() {
                let fx = x + i as f32 * (w + MAP_ROW_GAP);
                let label = (above > 0.0).then(|| Rectangle::new(fx, y, w, above));
                out.push(PlacedField { field: *field, rect: Rectangle::new(fx, y + above, w, h), label });
            }
        }
        y += row_height(&row) + MAP_ROW_GAP;
    }
}

/// A choice's `n` options side by side across `rect`.
pub fn segments(rect: Rectangle, n: usize) -> Vec<Rectangle> {
    let w = (rect.width - (n - 1) as f32 * MAP_SEGMENT_GAP) / n as f32;
    (0..n).map(|i| Rectangle::new(rect.x + i as f32 * (w + MAP_SEGMENT_GAP), rect.y, w, rect.height)).collect()
}

/// A stepper's `-` (or `<`), at its box's left end.
pub fn stepper_dec(rect: Rectangle) -> Rectangle {
    Rectangle::new(rect.x, rect.y, MAP_STEP_W, rect.height)
}

/// A stepper's `+` (or `>`), at its box's right end.
pub fn stepper_inc(rect: Rectangle) -> Rectangle {
    Rectangle::new(rect.x + rect.width - MAP_STEP_W, rect.y, MAP_STEP_W, rect.height)
}

/// The CHECK panel: its header and the clear check's row, a page of
/// findings - as many as the room under the bar holds, `LINT_PAGE_ROWS`
/// at most - and, past a page, the pager.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LintLayout {
    pub panel: Rectangle,
    /// Findings a page lists.
    pub per_page: usize,
    pub pages: usize,
    pub pager: Option<Pager>,
}

impl LintLayout {
    /// The panel for `findings` findings, hanging from `anchor` in `room`.
    pub fn of(anchor: Rectangle, room: Rectangle, findings: usize) -> LintLayout {
        let h = EDITOR_DROPDOWN_ROW_H;
        let max = rows_in(room, h);
        let unpaged = LINT_PAGE_ROWS.min(max.saturating_sub(LINT_HEAD_ROWS)).max(1);
        let (per_page, paged) = if findings <= unpaged {
            (unpaged, false)
        } else {
            (LINT_PAGE_ROWS.min(max.saturating_sub(LINT_HEAD_ROWS + 1)).max(1), true)
        };
        let shown = if paged { per_page } else { findings.max(1) };
        let rows = LINT_HEAD_ROWS + shown + paged as usize;
        let panel = Rectangle::new(slide(anchor.x, LINT_PANEL_W, room), room.y, LINT_PANEL_W, rows as f32 * h);
        let pager = paged.then(|| Pager { row: Self::row_in(panel, LINT_HEAD_ROWS + per_page) });
        LintLayout { panel, per_page, pages: findings.div_ceil(per_page).max(1), pager }
    }

    fn row_in(panel: Rectangle, index: usize) -> Rectangle {
        Rectangle::new(panel.x, panel.y + index as f32 * EDITOR_DROPDOWN_ROW_H, panel.width, EDITOR_DROPDOWN_ROW_H)
    }

    /// The panel's `index`th row: 0 is the header, 1 the clear check, the
    /// next `per_page` the page's findings.
    pub fn row(&self, index: usize) -> Rectangle {
        Self::row_in(self.panel, index)
    }

    /// A finding row's FIX button, at its right end: what a press hits, the
    /// row's whole height - a finger's size on a touch screen.
    pub fn fix(row: Rectangle) -> Rectangle {
        Rectangle::new(row.x + row.width - super::SETTINGS_INSET - super::LINT_FIX_W, row.y, super::LINT_FIX_W, row.height)
    }

    /// The FIX button's outline, drawn inside its hit rect (`fix`) a little
    /// short of the row's top and bottom, so the rows' outlines stand apart.
    pub fn fix_box(row: Rectangle) -> Rectangle {
        let hit = Self::fix(row);
        Rectangle::new(hit.x, hit.y + 4.0, hit.width, hit.height - 8.0)
    }
}

/// The Load list: centred in the room under the bar, a row per map shown,
/// and, where there are more maps than rows, a pager as its last row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoadLayout {
    pub rows: Rows,
    /// Maps a page shows.
    pub per_page: usize,
    pub pager: Option<Pager>,
}

impl LoadLayout {
    pub fn of(room: Rectangle, entries: usize) -> LoadLayout {
        Self::sized(room, entries, LOAD_PANEL_W, LOAD_VISIBLE_ROWS)
    }

    /// A list of `entries` rows `width` wide, at most `most` at once, in
    /// the Load list's shape: the STAMPS list's.
    pub fn sized(room: Rectangle, entries: usize, width: f32, most: usize) -> LoadLayout {
        let h = EDITOR_DROPDOWN_ROW_H;
        let max = rows_in(room, h).clamp(2, most.max(2));
        let shown = entries.clamp(1, max);
        let paged = entries > max;
        let per_page = if paged { max - 1 } else { max };
        let height = shown as f32 * h;
        let panel = crate::hud::centred_in(crate::Rect::new(room.x, room.y, room.width, room.height), width, height);
        let rows = Rows { panel, per_column: shown, row_w: width, row_h: h };
        let pager = paged.then(|| Pager { row: rows.row(per_page) });
        LoadLayout { rows, per_page, pager }
    }
}

/// A map's thumbnail box in its Load-list `row`: at the row's left, 8
/// points in, centred top to bottom.
pub fn load_picture(row: Rectangle) -> Rectangle {
    Rectangle::new(row.x + 8.0, row.y + ((row.height - LOAD_PICTURE.1) / 2.0).round(), LOAD_PICTURE.0, LOAD_PICTURE.1)
}

/// Where a Load row's text stands: past the picture, `LOAD_TEXT_W` wide,
/// the row's height.
pub fn load_text(row: Rectangle) -> Rectangle {
    Rectangle::new(row.x + 8.0 + LOAD_PICTURE.0 + 12.0, row.y, LOAD_TEXT_W, row.height)
}

/// A field `field` world pixels in size drawn as large as fits in `rect`,
/// its shape kept, centred, its edges on whole points - a thumbnail in its
/// box. Nothing where the field has no size.
pub fn fit_picture(rect: Rectangle, field: (f32, f32)) -> Option<Rectangle> {
    let (w, h) = field;
    if !(w > 0.0 && h > 0.0) {
        return None;
    }
    let k = (rect.width / w).min(rect.height / h);
    let (pw, ph) = ((w * k).round().max(1.0), (h * k).round().max(1.0));
    Some(Rectangle::new((rect.x + (rect.width - pw) / 2.0).round(), (rect.y + (rect.height - ph) / 2.0).round(), pw, ph))
}

/// What a press on the select tool's strip lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripButton {
    Copy,
    Cut,
    Paste,
    /// Mirror the selection (or the paste ghost) left to right.
    FlipH,
    /// Mirror it top to bottom.
    FlipV,
    Delete,
    /// Keep the selection as a stamp for this session.
    SaveStamp,
    /// Open the STAMPS list.
    Stamps,
    /// Put the paste ghost down where it stands.
    Place,
    /// Take the paste ghost away.
    Cancel,
}

impl StripButton {
    /// The strip's buttons while a selection is being worked on, in order.
    pub const SELECTION: [StripButton; 8] = [
        StripButton::Copy,
        StripButton::Cut,
        StripButton::Paste,
        StripButton::FlipH,
        StripButton::FlipV,
        StripButton::Delete,
        StripButton::SaveStamp,
        StripButton::Stamps,
    ];

    /// The strip's buttons while a paste ghost stands on the canvas.
    pub const GHOST: [StripButton; 4] = [StripButton::Place, StripButton::FlipH, StripButton::FlipV, StripButton::Cancel];

    /// As `status.builder.buttons` names it.
    pub fn name(self) -> &'static str {
        match self {
            StripButton::Copy => "sel_copy",
            StripButton::Cut => "sel_cut",
            StripButton::Paste => "sel_paste",
            StripButton::FlipH => "sel_flip_h",
            StripButton::FlipV => "sel_flip_v",
            StripButton::Delete => "sel_delete",
            StripButton::SaveStamp => "sel_stamp",
            StripButton::Stamps => "sel_stamps",
            StripButton::Place => "sel_place",
            StripButton::Cancel => "sel_cancel",
        }
    }

    /// Whether it is a picture rather than a word: the two flips.
    pub fn is_icon(self) -> bool {
        matches!(self, StripButton::FlipH | StripButton::FlipV)
    }

    /// The message on a button with a word; `None` for a picture's.
    pub fn label_key(self) -> Option<crate::text::Key> {
        use crate::text::keys;
        Some(match self {
            StripButton::Copy => keys::SELECT_COPY,
            StripButton::Cut => keys::SELECT_CUT,
            StripButton::Paste => keys::SELECT_PASTE,
            StripButton::Delete => keys::SELECT_DELETE,
            StripButton::SaveStamp => keys::SELECT_SAVE_STAMP,
            StripButton::Stamps => keys::SELECT_STAMPS,
            StripButton::Place => keys::SELECT_PLACE,
            StripButton::Cancel => keys::SELECT_CANCEL,
            StripButton::FlipH | StripButton::FlipV => return None,
        })
    }

    /// Its width in points: a word's button, or a picture's - a finger's
    /// size on a touch screen, the bar's small buttons' with a mouse.
    pub fn width(self, touch: bool) -> f32 {
        match (self.is_icon(), touch) {
            (true, true) => STRIP_ICON_W.1,
            (true, false) => STRIP_ICON_W.0,
            (false, true) => STRIP_WORD_W.1,
            (false, false) => STRIP_WORD_W.0,
        }
    }
}

/// A strip button with a word, with a mouse and on a touch screen: its
/// label in the bar's small text (`BAR_SMALL_TEXT`) inside its box
/// (`button_box`, `label_room`), the room `text_tests` holds every
/// language to.
pub const STRIP_WORD_W: (f32, f32) = (82.0, 82.0);

/// A strip button with a picture: square, the bar's height (`bar_height`)
/// - 40 with a mouse, 44 on a touch screen.
pub const STRIP_ICON_W: (f32, f32) = (40.0, 44.0);

/// Between two of the strip's buttons.
const STRIP_GAP: f32 = 4.0;

/// One of the strip's buttons: where it stands and whether it can act now
/// - a dim one (COPY with nothing selected) is drawn and never pressed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StripSlot {
    pub button: StripButton,
    pub rect: Rectangle,
    pub enabled: bool,
}

/// The select tool's action strip (the module docs): its buttons in a row
/// on a plate hanging under the bar from the room's top-left corner.
#[derive(Clone, Debug, PartialEq)]
pub struct Strip {
    /// The plate: a press inside it is the strip's, never the canvas's.
    pub panel: Rectangle,
    pub slots: Vec<StripSlot>,
}

impl Strip {
    /// The strip of `buttons` (each with whether it can act now) in `room`,
    /// laid out for a touch screen or a mouse.
    pub fn of(room: Rectangle, touch: bool, buttons: &[(StripButton, bool)]) -> Strip {
        let h = bar_height(touch);
        let y = room.y + PLATE_PAD;
        let mut x = room.x + PLATE_PAD;
        let slots: Vec<StripSlot> = buttons
            .iter()
            .map(|&(button, enabled)| {
                let w = button.width(touch);
                let rect = Rectangle::new(x, y, w, h);
                x += w + STRIP_GAP;
                StripSlot { button, rect, enabled }
            })
            .collect();
        let right = if slots.is_empty() { room.x + 2.0 * PLATE_PAD } else { x - STRIP_GAP + PLATE_PAD };
        Strip { panel: Rectangle::new(room.x, room.y, right - room.x, h + 2.0 * PLATE_PAD), slots }
    }

    /// The button that can act under `p` (UI points).
    pub fn hit(&self, p: Vec2) -> Option<StripButton> {
        self.slots.iter().find(|s| s.enabled && s.rect.contains(p)).map(|s| s.button)
    }

    /// The buttons that can act, by name (`status.builder.buttons`).
    pub fn named(&self) -> Vec<(String, Rectangle)> {
        self.slots.iter().filter(|s| s.enabled).map(|s| (s.button.name().to_string(), s.rect)).collect()
    }
}

/// The Save prompt, centred in the room under the bar.
pub fn save_prompt(room: Rectangle) -> Rectangle {
    crate::hud::centred_in(crate::Rect::new(room.x, room.y, room.width, room.height), SAVE_PROMPT.0, SAVE_PROMPT.1)
}

/// The Save prompt's SAVE button, at its right end beside the title and
/// the name, a bar button's height (`hud::button_height`): what a finger
/// saves with, as Enter does from the keys.
pub fn save_button(panel: Rectangle, touch: bool) -> Rectangle {
    Rectangle::new(panel.x + panel.width - 12.0 - SAVE_BUTTON_W, panel.y + 8.0, SAVE_BUTTON_W, button_height(touch))
}

/// A dropdown or FILE menu row of `EDITOR_DROPDOWN_W`.
pub fn menu_list(anchor: Rectangle, room: Rectangle, n: usize) -> Rows {
    hanging_list(anchor, room, n, EDITOR_DROPDOWN_W)
}

/// Where the status line's text starts and how wide it may run: from the
/// left of the room under the bar, its baseline at the room's bottom, up
/// to `until` (the navigator's plate) or the room's right edge.
pub fn status_line(room: Rectangle, until: Option<f32>) -> Rectangle {
    let x = room.x + 8.0;
    let right = until.map_or(room.x + room.width, |u| u - 8.0);
    Rectangle::new(x, room.y + room.height - STATUS_TEXT as f32, (right - x).max(0.0), STATUS_TEXT as f32)
}

/// The navigator's picture of `size` points in the room under the bar: in
/// its bottom-right corner, its plate's pad in from the edges, never more
/// than half the room either way. `None` where it would have no size.
pub fn navigator(room: Rectangle, size: (f32, f32)) -> Option<Rectangle> {
    let (w, h) = size;
    if !(w > 0.0 && h > 0.0) {
        return None;
    }
    let fit = (room.width * 0.5 / w).min(room.height * 0.5 / h).min(1.0);
    if !(fit > 0.0) {
        return None;
    }
    let (w, h) = (w * fit, h * fit);
    Some(Rectangle::new(room.x + room.width - PLATE_PAD - w, room.y + room.height - PLATE_PAD - h, w, h))
}

#[cfg(test)]
mod chrome_tests {
    use super::*;
    use crate::hud::{Insets, UI_SMALL_TEXT, UI_TOUCH_PT};
    use crate::text::{width, Catalogue, SHIPPED_LANGS};

    /// Every window the tests lay the builder out in: phones to a 1440p
    /// monitor, landscape.
    const WINDOWS: [(f32, f32); 12] = [
        (568.0, 320.0),
        (667.0, 375.0),
        (812.0, 375.0),
        (852.0, 393.0),
        (932.0, 430.0),
        (1024.0, 600.0),
        (1088.0, 576.0),
        (1180.0, 820.0),
        (1366.0, 1024.0),
        (1600.0, 900.0),
        (1920.0, 1080.0),
        (2560.0, 1440.0),
    ];

    /// Each window with and without touch, a unit a point and three, with
    /// a phone's safe area round a narrow one.
    fn frames() -> Vec<UiFrame> {
        let mut out = Vec::new();
        for (w, h) in WINDOWS {
            for touch in [false, true] {
                for units in [1.0, 3.0] {
                    let insets = if w < 1000.0 && touch {
                        Insets { left: 59.0 * units, top: 0.0, right: 59.0 * units, bottom: 21.0 * units }
                    } else {
                        Insets::default()
                    };
                    out.push(UiFrame::new((w * units, h * units), units, 1.0, insets, touch));
                }
            }
        }
        out
    }

    fn inside(r: Rectangle, outer: Rectangle) -> bool {
        r.x >= outer.x - 1e-3
            && r.y >= outer.y - 1e-3
            && r.x + r.width <= outer.x + outer.width + 1e-3
            && r.y + r.height <= outer.y + outer.height + 1e-3
    }

    /// The safe area itself: the chrome's area and its edges.
    fn safe(ui: &UiFrame) -> Rectangle {
        Rectangle::new(ui.area.x - UI_EDGE_PT, ui.area.y - UI_EDGE_PT, ui.area.w + 2.0 * UI_EDGE_PT, ui.area.h + 2.0 * UI_EDGE_PT)
    }

    fn overlap(a: Rectangle, b: Rectangle) -> bool {
        a.x < b.x + b.width - 1e-3 && b.x < a.x + a.width - 1e-3 && a.y < b.y + b.height - 1e-3 && b.y < a.y + a.height - 1e-3
    }

    /// The bar on every window: inside the safe area under its top edge,
    /// every button inside the chrome's area, no two overlapping, PLAY at
    /// the right end, 44 points both ways on a touch screen and the 40 a
    /// desktop's bar has with a mouse; BRUSH beside the five categories
    /// exactly while they are in the bar.
    #[test]
    fn the_bar_holds_every_button_inside_the_safe_area_on_every_window() {
        for ui in frames() {
            let bar = Bar::of(&ui);
            assert_eq!(bar.brush.is_some(), matches!(bar.tools, BarTools::Categories(_)), "{ui:?}");
            if let (Some(brush), Some(pickup)) = (bar.brush, bar.category(Category::Pickup)) {
                assert!(brush.x >= pickup.rect.x + pickup.rect.width && brush.x + brush.width <= bar.erase.x, "{ui:?}: BRUSH between the five and the eraser");
            }
            assert!(inside(Rectangle::new(0.0, bar.strip.y, bar.strip.width, bar.strip.height), Rectangle::new(0.0, 0.0, ui.screen.w, ui.screen.h)));
            assert!(bar.strip.y >= safe(&ui).y - 1e-3, "{ui:?}: the bar under the safe area's top");
            let buttons = bar.buttons();
            let mut rects: Vec<Rectangle> = buttons.iter().map(|(_, r)| *r).collect();
            rects.extend(bar.label);
            rects.extend(bar.name);
            for r in &rects {
                assert!(inside(*r, Rectangle::new(ui.area.x, bar.strip.y, ui.area.w, bar.strip.height)), "{ui:?}: {r:?} leaves the bar");
            }
            for (i, a) in rects.iter().enumerate() {
                for b in &rects[i + 1..] {
                    assert!(!overlap(*a, *b), "{ui:?}: {a:?} overlaps {b:?}");
                }
            }
            let least = if ui.touch { UI_TOUCH_PT } else { HUD_BAR_HEIGHT as f32 };
            for (button, r) in &buttons {
                assert!(r.height >= least - 1e-3, "{ui:?}: {button:?} is {} tall", r.height);
                if ui.touch {
                    assert!(r.width >= UI_TOUCH_PT - 1e-3, "{ui:?}: {button:?} is {} wide", r.width);
                }
            }
            assert!((bar.play.x + bar.play.width - (ui.area.x + ui.area.w)).abs() < 1e-3, "PLAY at the right end");
            assert!(bar.here.x + bar.here.width <= bar.play.x);
        }
    }

    /// A desktop's window lays the bar's slots out a point a pixel, BRUSH
    /// after the five categories; short of room the BUILD label goes
    /// first, then the name narrows, then the categories and BRUSH fold
    /// rather than shrink a button.
    #[test]
    fn a_desktops_bar_keeps_its_slots_and_a_narrow_one_folds() {
        let ui = UiFrame::plain((1280.0, 720.0));
        let bar = Bar::of(&ui);
        assert_eq!(bar.strip, Rectangle::new(0.0, 0.0, 1280.0, 40.0));
        assert_eq!(bar.label.map(|r| r.x), Some(8.0));
        assert_eq!(bar.name, Some(Rectangle::new(72.0, 0.0, 152.0, 40.0)));
        let wall = bar.category(Category::Wall).expect("five category buttons");
        assert_eq!(wall.rect, Rectangle::new(232.0, 0.0, 56.0, 40.0));
        assert_eq!(wall.icon.width, 36.0);
        let slots = [
            (bar.brush.expect("BRUSH beside the five"), 520.0, 56.0),
            (bar.erase, 584.0, 44.0),
            (bar.undo, 632.0, 52.0),
            (bar.redo, 688.0, 52.0),
            (bar.file, 744.0, 60.0),
            (bar.map, 808.0, 60.0),
            (bar.fit, 872.0, 44.0),
            (bar.check, 920.0, 76.0),
            (bar.clear, 998.0, 52.0),
            (bar.here, 1104.0, HERE_W),
            (bar.play, 1200.0, 72.0),
        ];
        for (r, x, w) in slots {
            assert_eq!((r.x, r.width, r.y, r.height), (x, w, 0.0, 40.0), "{r:?}");
        }
        // Wider: the same slots from the left, PLAY HERE and PLAY at the
        // right end.
        let wide = Bar::of(&UiFrame::plain((1600.0, 900.0)));
        assert_eq!((wide.erase.x, wide.clear.x, wide.play.x), (584.0, 998.0, 1600.0 - 8.0 - 72.0));
        // The arena's own window: the label gone, the name narrowed and
        // every button whole.
        let arena = Bar::of(&UiFrame::plain((1088.0, 576.0)));
        assert!(arena.label.is_none() && matches!(arena.tools, BarTools::Categories(_)), "{arena:?}");
        assert_eq!(arena.name, Some(Rectangle::new(8.0, 0.0, 76.0, 40.0)));
        assert_eq!((arena.erase.x, arena.clear.x, arena.here.x), (444.0, 858.0, 912.0));
        // Narrower: the categories and BRUSH fold into TOOLS, and the room
        // it frees gives the label back; narrower still, the name goes.
        let narrow = Bar::of(&UiFrame::plain((900.0, 500.0)));
        assert!(narrow.brush.is_none() && matches!(narrow.tools, BarTools::Folded(_)), "{narrow:?}");
        assert_eq!(narrow.name.map(|r| r.width), Some(152.0));
        let narrower = Bar::of(&UiFrame::plain((720.0, 400.0)));
        assert!(narrower.name.is_none() && matches!(narrower.tools, BarTools::Folded(_)), "{narrower:?}");
        assert_eq!(narrower.erase.width, 44.0, "folded, not shrunk");
    }

    /// Every label the bar draws fits its box in every shipped language, at
    /// the size it is drawn in, on every window: BUILD in its slot, FILE
    /// and MAP beside their carets, every other button's word in its box,
    /// all at one size, and the clear flag's longest par beside the flag;
    /// and every size is at least 11 points.
    #[test]
    fn every_bar_label_fits_its_box_in_every_language() {
        use crate::text::keys;
        for (tag, _) in SHIPPED_LANGS {
            let t = Catalogue::new(tag);
            for touch in [false, true] {
                let ui = UiFrame::new((1600.0, 900.0), 1.0, 1.0, Insets::default(), touch);
                let bar = Bar::of(&ui);
                let small = BAR_SMALL_TEXT;
                assert!(small >= 11);
                let fits = |key, size: i32, room: f32| {
                    let label = t.get(key);
                    assert!(width(&label, size) as f32 <= room, "{tag} touch={touch}: {label:?} at {size} is over {room}");
                };
                fits(keys::EDITOR_BUILD, crate::hud::HUD_TEXT_SIZE, bar.label.expect("a wide bar has its label").width);
                // Every word in its button's box, at the one size: FILE and
                // MAP before their carets.
                for (key, r) in [(keys::EDITOR_FILE, bar.file), (keys::EDITOR_MAP, bar.map)] {
                    fits(key, small, label_room(button_box(r, touch), true));
                }
                for (key, r) in [
                    (keys::EDITOR_UNDO, bar.undo),
                    (keys::EDITOR_REDO, bar.redo),
                    (keys::EDITOR_FIT, bar.fit),
                    (keys::EDITOR_CHECK, bar.check),
                    (keys::EDITOR_PLAY_HERE, bar.here),
                    (keys::BUTTON_PLAY, bar.play),
                ] {
                    fits(key, small, label_room(button_box(r, touch), false));
                }
                // The readout has no box: the flag, then the par from 16 in.
                assert!(16.0 + width("59:59", small) as f32 <= bar.clear.width, "touch={touch}: the par overflows its readout");
            }
            // The palette's category names in their column.
            for category in Category::ALL {
                let label = t.get(category.label_key());
                assert!(width(&label, UI_SMALL_TEXT) as f32 <= PALETTE_LABEL_W - 12.0, "{tag}: {label:?} overflows the palette");
            }
        }
    }

    /// Every popup fits the room under the bar on every window - inside the
    /// chrome's area, under the bar - with its rows a finger's height, and
    /// pages where the room is short: the category lists in columns, the
    /// palette whole, the MAP panel, the CHECK panel and the Load list
    /// paged.
    #[test]
    fn every_popup_fits_under_the_bar_on_every_window() {
        for ui in frames() {
            let frame = BuilderFrame::new(ui, (1088.0, 544.0), MapClass::Arena, None);
            let bar = frame.bar();
            let room = frame.under_bar();
            assert!(room.height >= 6.0 * EDITOR_DROPDOWN_ROW_H, "{ui:?}: {room:?}");
            let check = |what: &str, r: Rectangle| {
                assert!(inside(r, room), "{ui:?}: {what} {r:?} leaves {room:?}");
                assert!(r.y >= bar.strip.y + bar.strip.height - 1e-3, "{what} over the bar");
            };
            for category in Category::ALL {
                let n = category.tools().count();
                let list = menu_list(bar.tools_anchor(category), room, n);
                check("a list", list.panel);
                for i in 0..n {
                    check("a list row", list.row(i));
                    assert!(list.row(i).height >= UI_TOUCH_PT);
                }
            }
            let palette = Palette::of(bar.tools_anchor(Category::Wall), room);
            check("the palette", palette.panel);
            for category in Category::ALL {
                for i in 0..category.tools().count() {
                    let cell = palette.cell(category, i);
                    check("a palette cell", cell);
                    assert!(cell.width >= UI_TOUCH_PT && cell.height >= UI_TOUCH_PT);
                }
            }
            for i in 0..BrushRow::ALL.len() {
                let cell = palette.brush_cell(i);
                check("a palette brush cell", cell);
                assert!(cell.width >= UI_TOUCH_PT && cell.height >= UI_TOUCH_PT);
            }
            let brush = menu_list(bar.brush.unwrap_or_else(|| bar.tools_anchor(Category::Wall)), room, BrushRow::ALL.len());
            check("BRUSH's list", brush.panel);
            for i in 0..BrushRow::ALL.len() {
                check("a BRUSH row", brush.row(i));
                assert!(brush.row(i).height >= UI_TOUCH_PT);
            }
            for entries in [1, 3, 9, 40] {
                let stamps = LoadLayout::sized(room, entries, STAMPS_PANEL_W, STAMPS_VISIBLE_ROWS);
                check("the STAMPS list", stamps.rows.panel);
                assert!(entries <= stamps.per_page || stamps.pager.is_some(), "{entries} stamps and no pager");
            }
            let file = menu_list(bar.file, room, 4);
            check("the FILE menu", file.panel);
            // The MAP panel, each group shown and the spawn plan either
            // way: inside the room, every button inside its panel and a
            // finger's size, no two overlapping.
            for tab in MapTab::ALL {
                for waves in [false, true] {
                    let map = MapPanel::of(bar.map, room, tab, waves);
                    check("the MAP panel", map.panel);
                    let buttons = map.buttons();
                    for (i, (button, rect)) in buttons.iter().enumerate() {
                        assert!(inside(*rect, map.panel), "{ui:?}: {} {rect:?} leaves the MAP panel", button.name());
                        assert!(rect.width >= UI_TOUCH_PT && rect.height >= UI_TOUCH_PT, "{ui:?}: {} is {rect:?}", button.name());
                        for (other, r) in &buttons[i + 1..] {
                            let apart = rect.x + rect.width <= r.x + 1e-3 || r.x + r.width <= rect.x + 1e-3 || rect.y + rect.height <= r.y + 1e-3 || r.y + r.height <= rect.y + 1e-3;
                            assert!(apart, "{ui:?}: {} overlaps {}", button.name(), other.name());
                        }
                    }
                }
            }
            for findings in [0, 1, 5, 7, 8, 30] {
                let lint = LintLayout::of(bar.check, room, findings);
                check("the CHECK panel", lint.panel);
                assert!(lint.per_page >= 1 && lint.per_page * lint.pages >= findings);
                if let Some(pager) = lint.pager {
                    check("the CHECK pager", pager.row);
                }
                // Each finding's FIX: inside its row, its outline inside
                // what a press hits, a finger's size on a touch screen.
                for slot in 0..lint.per_page.min(findings) {
                    let row = lint.row(LINT_HEAD_ROWS + slot);
                    let (fix, outline) = (LintLayout::fix(row), LintLayout::fix_box(row));
                    assert!(inside(fix, row) && inside(outline, fix), "{ui:?}: FIX {fix:?} in {row:?}");
                    if ui.touch {
                        assert!(fix.width >= UI_TOUCH_PT && fix.height >= UI_TOUCH_PT, "{ui:?}: FIX is {fix:?}");
                    }
                }
            }
            for entries in [0, 3, 8, 9, 40] {
                let load = LoadLayout::of(room, entries);
                check("the Load list", load.rows.panel);
                assert!(entries <= load.per_page || load.pager.is_some(), "{entries} maps and no pager");
                // Each row's picture and text inside the row, apart.
                for i in 0..load.per_page.min(entries) {
                    let row = load.rows.row(i);
                    let (picture, text) = (load_picture(row), load_text(row));
                    assert!(inside(picture, row) && inside(text, row), "{ui:?}: a Load row's parts in the row");
                    assert!(!overlap(picture, text));
                    assert!(LOAD_DETAIL_Y + UI_SMALL_TEXT as f32 <= row.height, "two lines in a row");
                }
            }
            check("the Save prompt", save_prompt(room));
            let save = save_button(save_prompt(room), ui.touch);
            assert!(inside(save, save_prompt(room)), "{ui:?}: SAVE {save:?} leaves its prompt");
            if ui.touch {
                assert!(save.width >= UI_TOUCH_PT && save.height >= UI_TOUCH_PT, "{ui:?}: SAVE is {save:?}");
            }
            let nav = navigator(room, (180.0, 100.0)).expect("a navigator");
            check("the navigator", crate::hud::Corners::plate(nav));
        }
    }

    /// A thumbnail fills its box as far as its shape lets it, centred, on
    /// whole points: a 16:9 map its height, a 2:1 map its width, a tall one
    /// its height; a field with no size has none.
    #[test]
    fn a_thumbnail_keeps_its_shape_inside_its_box() {
        let row = Rectangle::new(100.0, 200.0, LOAD_PANEL_W, crate::EDITOR_DROPDOWN_ROW_H);
        let b = load_picture(row);
        assert_eq!(b, Rectangle::new(108.0, 204.0, 72.0, 40.0));
        for (field, want) in [
            ((2560.0, 1440.0), (71.0, 40.0)),
            ((1088.0, 544.0), (72.0, 36.0)),
            ((1536.0, 768.0), (72.0, 36.0)),
            ((32.0 * 250.0, 32.0 * 250.0), (40.0, 40.0)),
            ((544.0, 1088.0), (20.0, 40.0)),
        ] {
            let p = fit_picture(b, field).expect("a picture");
            assert_eq!((p.width, p.height), want, "{field:?}");
            assert!(inside(p, b), "{field:?}: {p:?} in {b:?}");
            assert!((p.x - (b.x + (b.width - p.width) / 2.0)).abs() <= 0.5, "centred");
            assert_eq!((p.x.fract(), p.y.fract()), (0.0, 0.0), "whole points");
        }
        assert_eq!(fit_picture(b, (0.0, 544.0)), None);
    }

    /// The select tool's strip on every window, both its faces: on a plate
    /// hanging from the room's top-left corner, inside the room, its
    /// buttons in a row inside the plate with none overlapping another, a
    /// finger's size both ways on a touch screen and the bar's height with
    /// a mouse; a press on a live button comes back through the UI frame
    /// onto it, and one on a dim button presses nothing.
    #[test]
    fn the_strip_hangs_under_the_bar_inside_the_room_on_every_window() {
        for ui in frames() {
            let frame = BuilderFrame::new(ui, (1088.0, 544.0), MapClass::Arena, None);
            let (bar, room) = (frame.bar(), frame.under_bar());
            for face in [&StripButton::SELECTION[..], &StripButton::GHOST[..]] {
                let buttons: Vec<(StripButton, bool)> = face.iter().enumerate().map(|(i, &b)| (b, i % 3 != 1)).collect();
                let strip = Strip::of(room, ui.touch, &buttons);
                assert!(inside(strip.panel, room), "{ui:?}: the strip {:?} leaves {room:?}", strip.panel);
                assert!(strip.panel.y >= bar.strip.y + bar.strip.height - 1e-3, "{ui:?}: the strip over the bar");
                assert_eq!(strip.slots.len(), face.len());
                for (i, slot) in strip.slots.iter().enumerate() {
                    assert!(inside(slot.rect, strip.panel), "{ui:?}: {:?} leaves its plate", slot.button);
                    if ui.touch {
                        assert!(slot.rect.width >= UI_TOUCH_PT - 1e-3 && slot.rect.height >= UI_TOUCH_PT - 1e-3, "{ui:?}: {:?} {:?}", slot.button, slot.rect);
                    } else {
                        assert_eq!(slot.rect.height, HUD_BAR_HEIGHT as f32);
                    }
                    for other in &strip.slots[i + 1..] {
                        assert!(!overlap(slot.rect, other.rect), "{ui:?}: {:?} overlaps {:?}", slot.button, other.button);
                    }
                    let at = Vec2::new(slot.rect.x + slot.rect.width / 2.0, slot.rect.y + slot.rect.height / 2.0);
                    let back = ui.to_ui(ui.to_window(at));
                    assert_eq!(strip.hit(back), slot.enabled.then_some(slot.button), "{ui:?}: {:?}", slot.button);
                }
                let named: Vec<String> = strip.named().into_iter().map(|(n, _)| n).collect();
                assert_eq!(named.len(), buttons.iter().filter(|(_, live)| *live).count(), "only the live buttons are named");
            }
        }
    }

    /// A desktop's popups keep their size: the MAP panel every group at
    /// once in two columns, the CHECK panel seven findings a page, the Load
    /// list eight rows, the longest category list one column.
    #[test]
    fn a_desktops_popups_keep_their_shape() {
        let frame = BuilderFrame::headless((1088.0, 544.0), MapClass::Arena);
        let (bar, room) = (frame.bar(), frame.under_bar());
        let map = MapPanel::of(bar.map, room, MapTab::Sky, true);
        assert_eq!(map.shape, MapShape::Sections);
        assert_eq!((map.panel.width, map.panel.height), MapPanel::sections_size());
        assert!(map.tabs.is_empty() && map.headings.len() == MapTab::ALL.len());
        let lint = LintLayout::of(bar.check, room, 30);
        assert_eq!(lint.per_page, LINT_PAGE_ROWS);
        assert_eq!(LoadLayout::of(room, 40).per_page, LOAD_VISIBLE_ROWS - 1);
        let prop = menu_list(bar.tools_anchor(Category::Prop), room, Category::Prop.tools().count());
        assert_eq!(prop.per_column, Category::Prop.tools().count());
        assert_eq!(prop.panel.y, HUD_BAR_HEIGHT as f32, "hanging from the bar");
    }

    /// A press anywhere on a button's drawn box, in window coordinates on
    /// any window, comes back through the UI frame onto that button.
    #[test]
    fn a_press_on_a_button_round_trips_through_the_ui_frame() {
        for ui in frames() {
            let bar = Bar::of(&ui);
            for (button, r) in bar.buttons() {
                for (fx, fy) in [(0.5, 0.5), (0.1, 0.1), (0.9, 0.9)] {
                    let at = Vec2::new(r.x + r.width * fx, r.y + r.height * fy);
                    let window = ui.to_window(at);
                    let back = ui.to_ui(window);
                    let hit = bar.hit(back);
                    // The clear flag opens the CHECK panel like CHECK.
                    assert_eq!(hit, Some(button), "{ui:?}: {button:?} at {window:?}");
                }
            }
        }
    }

    /// The canvas lands under the bar: an arena letterboxed at its own
    /// shape, a field map filling the window under it; both through the
    /// frame's view, from the window back to the bitmap.
    #[test]
    fn the_canvas_stands_under_the_bar() {
        for ui in frames() {
            let strip = Bar::strip_of(&ui);
            let bottom = (strip.y + strip.height) * ui.scale;
            for (field, class) in [((1088.0, 544.0), MapClass::Arena), ((96.0 * 32.0, 54.0 * 32.0), MapClass::Field)] {
                let frame = BuilderFrame::new(ui, field, class, None);
                let f = frame.layout.field;
                let corner = frame.view.to_window(Vec2::new(f.x, f.y));
                assert!(corner.y >= bottom - 0.5, "{ui:?} {class:?}: the canvas starts under the bar at {corner:?}");
                let far = frame.view.to_window(Vec2::new(f.x + f.w, f.y + f.h));
                let (ww, wh) = (ui.screen.w * ui.scale, ui.screen.h * ui.scale);
                assert!(far.x <= ww + 0.5 && far.y <= wh + 0.5, "{ui:?} {class:?}: off the window at {far:?}");
                if class == MapClass::Field {
                    let (left, right) = ((ui.area.x - UI_EDGE_PT) * ui.scale, (ui.area.x + ui.area.w + UI_EDGE_PT) * ui.scale);
                    assert!(
                        (far.y - wh).abs() <= frame.view.scale + 0.5 && (corner.x - left).abs() <= frame.view.scale + 0.5 && (far.x - right).abs() <= frame.view.scale + 0.5,
                        "{ui:?}: a field map fills the safe area's sides under the bar"
                    );
                } else {
                    assert!(((far.x - corner.x) / (far.y - corner.y) - 2.0).abs() < 0.01, "the arena keeps its shape");
                }
                let p = Vec2::new(f.x + f.w * 0.3, f.y + f.h * 0.7);
                let round = frame.to_canvas(frame.view.to_window(p));
                assert!((round.x - p.x).abs() < 1e-2 && (round.y - p.y).abs() < 1e-2);
            }
        }
        // A desktop's arena stands right under the 40 pt bar at its own
        // size.
        let frame = BuilderFrame::headless((1088.0, 544.0), MapClass::Arena);
        assert_eq!((frame.view.scale, frame.view.offset), (1.0, Vec2::new(0.0, HUD_BAR_HEIGHT as f32)));
    }

    /// The canvas keeps to the safe area's sides on every window - on a
    /// phone in landscape the notch's side and the rounded corners' - an
    /// arena letterboxed between them and a field map filling them to a
    /// bitmap pixel, with a mouse and on a touch screen.
    #[test]
    fn the_canvas_keeps_to_the_safe_areas_sides() {
        // An iPhone in landscape: 852 x 393 points at three device pixels
        // a point, the notch's 59 points on either side, the home
        // indicator's 21 under the glass.
        let insets = Insets { left: 59.0 * 3.0, top: 0.0, right: 59.0 * 3.0, bottom: 21.0 * 3.0 };
        let phone = [false, true].map(|touch| UiFrame::new((852.0 * 3.0, 393.0 * 3.0), 3.0, 1.0, insets, touch));
        for ui in frames().into_iter().chain(phone) {
            let s = safe(&ui);
            for (field, class) in [((1088.0, 544.0), MapClass::Arena), ((96.0 * 32.0, 54.0 * 32.0), MapClass::Field)] {
                let frame = BuilderFrame::new(ui, field, class, None);
                let c = frame.canvas_ui();
                assert!(c.x >= s.x - 1e-3 && c.x + c.width <= s.x + s.width + 1e-3, "{ui:?} {class:?}: the canvas {c:?} leaves the safe area's sides {s:?}");
                if class == MapClass::Field {
                    let px = frame.view.scale / ui.scale;
                    assert!(c.x - s.x <= px + 1e-3 && s.x + s.width - (c.x + c.width) <= px + 1e-3, "{ui:?}: a field map fills them, {c:?} in {s:?}");
                }
            }
        }
        // The phone's: 59 points in on either side.
        let frame = BuilderFrame::new(phone[1], (96.0 * 32.0, 54.0 * 32.0), MapClass::Field, None);
        let c = frame.canvas_ui();
        assert!(c.x >= 59.0 - 1e-3 && c.x + c.width <= 852.0 - 59.0 + 1e-3, "{c:?}");
    }

    /// A field map's canvas is the shape of the room it is given - no
    /// letterbox - a bitmap pixel the size the standard field's would be
    /// fitted into it.
    #[test]
    fn a_field_maps_canvas_fills_its_region_at_the_standard_fields_scale() {
        let cap = Some(ScaleCap { max_scale: 1.5, snap_half: false });
        for (region, cap) in [((852.0, 349.0), None), ((1180.0, 776.0), None), ((1920.0, 1048.0), cap), ((1088.0, 544.0), cap)] {
            let (layout, view) = canvas_frame(region, cap);
            let standard = View::fit_capped((1088.0, 544.0), region, cap);
            assert!((view.scale - standard.scale).abs() / standard.scale < 0.01, "{region:?}: {view:?} vs {standard:?}");
            assert_eq!(layout.panel.h, 0.0, "no bar in the canvas's bitmap");
            let d = view.dest();
            assert!((d.width - region.0).abs() <= view.scale + 0.01 && (d.height - region.1).abs() <= view.scale + 0.01, "{region:?}: {d:?}");
        }
    }
}
