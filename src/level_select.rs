//! The level select (docs/levels.md): every level of `levels.toml` as a
//! tile over the dimmed field - the ones already won and the furthest
//! reached open to a press, the rest locked. It is how a player goes back
//! to a level: the campaign itself only ever moves forward, and a replay
//! moves nothing.
//!
//! Opened from the HUD's level button, the end screen's `LEVELS` and the
//! Esc key (`mode::Session::press_levels`); the round behind it stands
//! still, as it does behind the dialogs, because `Session::playing` is
//! false while it is up.
//!
//! Headless, like `lobby.rs`: this half is the one piece of state (the
//! tile the keyboard is on), the geometry - `tile_rect` and `back_rect`
//! are the one table the drawing and every hit test read - and the view a
//! painter needs; `render::level_select` is the painting. It starts
//! nothing itself: a press comes back as a [`SelectAction`] and the
//! session opens the level.

use crate::levels::Campaign;
use crate::math::{Rectangle, Vec2};
use crate::text::{fit, keys, text, width};
use crate::Rect;

/// The panel, centred in the chrome's area (`hud::UiFrame::area`), in UI
/// points like everything in this screen: the lobby's size, fixed so no
/// tile moves with the window, and as small as any window's area.
pub const SELECT_W: f32 = 704.0;
pub const SELECT_H: f32 = 336.0;
/// The gutter between the panel's edge and anything in it.
pub const SELECT_MARGIN: f32 = 20.0;

/// Tiles per row and rows: the one page holds `SELECT_TILES` levels, and a
/// test holds `levels.toml` to it.
pub const SELECT_COLUMNS: usize = 8;
pub const SELECT_ROWS: usize = 2;
pub const SELECT_TILES: usize = SELECT_COLUMNS * SELECT_ROWS;
pub const SELECT_TILE_GAP: f32 = 6.0;

/// The title and the line under it take this much of the content's top
/// before the grid starts.
const GRID_TOP: f32 = 48.0;
/// The gap between the grid and the `BACK` button under it.
const GRID_BOTTOM_GAP: f32 = 12.0;

/// The `BACK` button, centred under the grid: the lobby's button size.
pub const SELECT_BACK_W: f32 = 140.0;
pub const SELECT_BACK_H: f32 = 48.0;

/// A tile's number, and its title under it on at most two lines
/// (`wrap`).
pub const TILE_NUMBER_SIZE: i32 = 28;
pub const TILE_TITLE_SIZE: i32 = 12;
/// The inset a tile's title keeps from either side.
pub const TILE_PAD: f32 = 4.0;

/// The panel, centred in the chrome's `area` (UI points).
pub fn panel_rect(area: Rect) -> Rectangle {
    crate::hud::centred_in(area, SELECT_W, SELECT_H)
}

/// The content box inside the panel: the margin taken off every side.
pub fn content_rect(area: Rect) -> Rectangle {
    let p = panel_rect(area);
    Rectangle::new(p.x + SELECT_MARGIN, p.y + SELECT_MARGIN, SELECT_W - 2.0 * SELECT_MARGIN, SELECT_H - 2.0 * SELECT_MARGIN)
}

/// The `BACK` button, centred along the content's bottom edge.
pub fn back_rect(area: Rect) -> Rectangle {
    let c = content_rect(area);
    Rectangle::new((c.x + (c.width - SELECT_BACK_W) / 2.0).round(), c.y + c.height - SELECT_BACK_H, SELECT_BACK_W, SELECT_BACK_H)
}

/// Level `i`'s tile (0-based): row-major, `SELECT_COLUMNS` to a row,
/// filling the content between the title and the `BACK` button. Whole
/// points, so a tile's outline is crisp and two tiles never share one.
pub fn tile_rect(area: Rect, i: usize) -> Rectangle {
    let c = content_rect(area);
    let top = c.y + GRID_TOP;
    let bottom = back_rect(area).y - GRID_BOTTOM_GAP;
    let (cols, rows) = (SELECT_COLUMNS as f32, SELECT_ROWS as f32);
    let w = ((c.width - (cols - 1.0) * SELECT_TILE_GAP) / cols).floor();
    let h = ((bottom - top - (rows - 1.0) * SELECT_TILE_GAP) / rows).floor();
    // The spare points the flooring leaves go either side of the grid.
    let x0 = (c.x + (c.width - cols * w - (cols - 1.0) * SELECT_TILE_GAP) / 2.0).round();
    let (col, row) = ((i % SELECT_COLUMNS) as f32, (i / SELECT_COLUMNS) as f32);
    Rectangle::new(x0 + col * (w + SELECT_TILE_GAP), top + row * (h + SELECT_TILE_GAP), w, h)
}

/// The room one line of a tile's title has - the same in every area,
/// since the panel is fixed.
pub fn tile_text_px() -> i32 {
    (tile_rect(Rect::new(0.0, 0.0, SELECT_W, SELECT_H), 0).width - 2.0 * TILE_PAD) as i32
}

/// `title` on at most two lines of `max_px` at `size`: whole when it
/// fits on one, else broken at the space that leaves the longer line
/// shortest, so the two read as a pair - `Labirint / živih mej` rather
/// than `Labirint živih / mej`. When no break fits both lines, the first
/// takes what it can and the rest is cut with `text::fit`'s `~`.
pub fn wrap(title: &str, max_px: i32, size: i32) -> Vec<String> {
    let words: Vec<&str> = title.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let whole = words.join(" ");
    if width(&whole, size) <= max_px {
        return vec![whole];
    }
    let best = (1..words.len())
        .map(|k| (words[..k].join(" "), words[k..].join(" ")))
        .filter(|(a, b)| width(a, size) <= max_px && width(b, size) <= max_px)
        .min_by_key(|(a, b)| width(a, size).max(width(b, size)));
    if let Some((a, b)) = best {
        return vec![a, b];
    }
    let mut k = 1;
    while k < words.len() && width(&words[..=k].join(" "), size) <= max_px {
        k += 1;
    }
    let first = fit(&words[..k].join(" "), max_px, size).into_owned();
    if k == words.len() {
        return vec![first];
    }
    vec![first, fit(&words[k..].join(" "), max_px, size).into_owned()]
}

/// One frame's input to the screen, from the mouse, a finger or the dev
/// server's `click`/`key` alike.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SelectInput {
    /// The pointer in UI points (`hud::UiFrame::to_ui`).
    pub pointer: Option<Vec2>,
    pub pressed: bool,
    /// The arrow keys move the focus over the open tiles.
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    /// Starts the level the focus is on.
    pub enter: bool,
    /// Closes the screen, as `BACK` and a press outside the panel do.
    pub escape: bool,
}

/// What a frame of the screen asks the session to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectAction {
    Stay,
    Close,
    /// Start level `i` (0-based) - one that is open, never a locked one.
    Start(usize),
}

/// How a tile stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileState {
    /// Won: before the furthest level reached.
    Won,
    /// The furthest level reached - the one to beat next.
    Next,
    /// Past it: drawn, never pressed.
    Locked,
}

/// One tile, as the painter needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct TileView {
    /// Counted from 1.
    pub number: usize,
    /// The title in the language on screen, wrapped to the tile.
    pub lines: Vec<String>,
    pub state: TileState,
    /// The level on the field behind the screen.
    pub current: bool,
    /// The tile the keyboard is on: Enter starts it.
    pub focus: bool,
}

/// The screen as plain data (the `HudModel` pattern): what
/// `render::level_select::draw_level_select` paints.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelSelectView {
    pub title: String,
    pub sub: String,
    pub back: String,
    pub tiles: Vec<TileView>,
}

/// The screen's state: which tile the keyboard is on.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelSelect {
    focus: usize,
    /// Where the pointer was the frame before: a tile takes the focus
    /// when the pointer moves onto it, not because the pointer happened
    /// to rest there when the screen opened.
    last_pointer: Option<Vec2>,
}

impl LevelSelect {
    /// The screen with the focus on `current` - the level on the field -
    /// when that one is open, else on `reached`, the furthest open
    /// (`Campaign::open_to`).
    pub fn open(current: Option<usize>, reached: usize) -> Self {
        LevelSelect { focus: current.filter(|&i| i <= reached).unwrap_or(reached), last_pointer: None }
    }

    pub fn focus(&self) -> usize {
        self.focus
    }

    /// One frame: a press on an open tile starts it, one on `BACK` or
    /// outside the panel closes the screen, and one anywhere else in the
    /// panel - a locked tile included - does nothing. The panel is
    /// centred in `area` (UI points); `count` levels, `reached` the
    /// furthest open (`Campaign::open_to`).
    pub fn update(&mut self, input: &SelectInput, area: Rect, count: usize, reached: usize) -> SelectAction {
        let open = last_open(count, reached);
        self.focus = self.focus.min(open);
        let moved = matches!((self.last_pointer, input.pointer), (Some(a), Some(b)) if a != b);
        self.last_pointer = input.pointer;
        let under = input.pointer.and_then(|p| (0..=open).find(|&i| tile_rect(area, i).contains(p)));
        if input.escape {
            return SelectAction::Close;
        }
        if input.pressed
            && let Some(p) = input.pointer
        {
            if let Some(i) = under {
                self.focus = i;
                return SelectAction::Start(i);
            }
            if back_rect(area).contains(p) || !panel_rect(area).contains(p) {
                return SelectAction::Close;
            }
            return SelectAction::Stay;
        }
        if moved && let Some(i) = under {
            self.focus = i;
        }
        if input.left {
            self.focus = self.focus.saturating_sub(1);
        }
        if input.right {
            self.focus = (self.focus + 1).min(open);
        }
        if input.up && self.focus >= SELECT_COLUMNS {
            self.focus -= SELECT_COLUMNS;
        }
        if input.down && self.focus + SELECT_COLUMNS <= open {
            self.focus += SELECT_COLUMNS;
        }
        if input.enter {
            return SelectAction::Start(self.focus);
        }
        SelectAction::Stay
    }

    /// The screen over `campaign`, `current` the level on the field.
    pub fn view(&self, campaign: &Campaign, current: Option<usize>) -> LevelSelectView {
        let t = text();
        let reached = campaign.reached();
        let open = campaign.open_to();
        let max_px = tile_text_px();
        let tiles = campaign
            .levels
            .iter()
            .take(SELECT_TILES)
            .enumerate()
            .map(|(i, level)| TileView {
                number: i + 1,
                lines: wrap(&level.title(), max_px, TILE_TITLE_SIZE),
                // Past the furthest reached, a skippable level leaves the
                // one after it open (`Campaign::open_to`).
                state: match i.cmp(&reached) {
                    std::cmp::Ordering::Less => TileState::Won,
                    std::cmp::Ordering::Equal => TileState::Next,
                    std::cmp::Ordering::Greater if i <= open => TileState::Next,
                    std::cmp::Ordering::Greater => TileState::Locked,
                },
                current: current == Some(i),
                focus: i == self.focus,
            })
            .collect();
        LevelSelectView { title: t.get(keys::LEVELS_TITLE), sub: t.get(keys::LEVELS_SUB), back: t.get(keys::LEVELS_BACK), tiles }
    }
}

/// The last tile a press or a key can reach: the furthest level reached,
/// within the one page.
fn last_open(count: usize, reached: usize) -> usize {
    reached.min(count.min(SELECT_TILES).saturating_sub(1))
}

#[cfg(test)]
mod level_select_tests {
    use super::*;
    use crate::levels::Levels;
    use crate::lobby::LOBBY_TOUCH_MIN;

    /// The chrome's area (UI points) on a window of the default map's
    /// bitmap, a point a pixel, and the smallest any window lays it out
    /// in: the panel itself.
    fn areas() -> [Rect; 2] {
        [
            crate::hud::UiFrame::plain((1088.0, 576.0)).area,
            crate::hud::UiFrame::plain((crate::hud::UI_MIN_W, crate::hud::UI_MIN_H)).area,
        ]
    }

    fn centre(r: Rectangle) -> Vec2 {
        Vec2::new(r.x + r.width / 2.0, r.y + r.height / 2.0)
    }

    fn press(p: Vec2) -> SelectInput {
        SelectInput { pointer: Some(p), pressed: true, ..SelectInput::default() }
    }

    #[test]
    fn the_shipped_levels_fit_one_page() {
        assert!(
            Levels::shipped().len() <= SELECT_TILES,
            "levels.toml has more levels than the level select's one page holds - give it a pager"
        );
    }

    #[test]
    fn every_tile_and_the_back_button_are_finger_sized_inside_the_panel() {
        for area in areas() {
            let panel = panel_rect(area);
            assert!(panel.x >= area.x && panel.y >= area.y, "{area:?}");
            assert!(panel.x + panel.width <= area.x + area.w && panel.y + panel.height <= area.y + area.h, "{area:?}");
            let content = content_rect(area);
            let inside = |r: Rectangle| {
                r.x >= content.x
                    && r.y >= content.y
                    && r.x + r.width <= content.x + content.width
                    && r.y + r.height <= content.y + content.height
            };
            let back = back_rect(area);
            assert!(inside(back), "{area:?}");
            assert!(back.width >= LOBBY_TOUCH_MIN && back.height >= LOBBY_TOUCH_MIN);
            for i in 0..SELECT_TILES {
                let r = tile_rect(area, i);
                assert!(inside(r), "tile {i} leaves the panel on {area:?}");
                assert!(r.width >= LOBBY_TOUCH_MIN && r.height >= LOBBY_TOUCH_MIN, "tile {i}: {r:?}");
                assert!(r.y + r.height + GRID_BOTTOM_GAP <= back.y, "tile {i} runs into BACK");
                assert!(r.y >= content.y + GRID_TOP, "tile {i} runs into the title");
                for j in 0..i {
                    let o = tile_rect(area, j);
                    let apart = r.x >= o.x + o.width || o.x >= r.x + r.width || r.y >= o.y + o.height || o.y >= r.y + r.height;
                    assert!(apart, "tiles {j} and {i} overlap");
                }
            }
        }
    }

    #[test]
    fn a_press_starts_an_open_level_and_a_locked_one_is_no_button() {
        let area = areas()[0];
        let mut s = LevelSelect::open(Some(2), 3);
        assert_eq!(s.focus(), 2);
        assert_eq!(s.update(&press(centre(tile_rect(area, 4))), area, 14, 3), SelectAction::Stay);
        assert_eq!(s.update(&press(centre(tile_rect(area, 13))), area, 14, 3), SelectAction::Stay);
        assert_eq!(s.update(&press(centre(tile_rect(area, 0))), area, 14, 3), SelectAction::Start(0));
        assert_eq!(s.update(&press(centre(tile_rect(area, 3))), area, 14, 3), SelectAction::Start(3));
        // The panel's own face is not a button either.
        let title = Vec2::new(content_rect(area).x + 4.0, content_rect(area).y + 4.0);
        assert_eq!(s.update(&press(title), area, 14, 3), SelectAction::Stay);
    }

    #[test]
    fn back_escape_and_a_press_outside_close_it() {
        let area = areas()[0];
        let mut s = LevelSelect::open(None, 0);
        assert_eq!(s.update(&press(centre(back_rect(area))), area, 14, 0), SelectAction::Close);
        assert_eq!(s.update(&press(Vec2::new(4.0, 4.0)), area, 14, 0), SelectAction::Close);
        let escape = SelectInput { escape: true, ..SelectInput::default() };
        assert_eq!(s.update(&escape, area, 14, 0), SelectAction::Close);
    }

    #[test]
    fn the_keys_walk_the_open_tiles_and_enter_starts_one() {
        let area = areas()[0];
        let key = |f: fn(&mut SelectInput)| {
            let mut input = SelectInput::default();
            f(&mut input);
            input
        };
        // Nine levels reached: tiles 0..=8 open, the rest locked.
        let mut s = LevelSelect::open(Some(12), 8);
        assert_eq!(s.focus(), 8, "a locked level on the field puts the focus on the furthest reached");
        s.update(&key(|i| i.right = true), area, 14, 8);
        assert_eq!(s.focus(), 8, "right stops at the last open tile");
        s.update(&key(|i| i.up = true), area, 14, 8);
        assert_eq!(s.focus(), 8 - SELECT_COLUMNS);
        s.update(&key(|i| i.down = true), area, 14, 8);
        assert_eq!(s.focus(), 8);
        s.update(&key(|i| i.up = true), area, 14, 8);
        s.update(&key(|i| i.right = true), area, 14, 8);
        s.update(&key(|i| i.down = true), area, 14, 8);
        assert_eq!(s.focus(), 9 - SELECT_COLUMNS, "down onto a locked tile stays put");
        s.update(&key(|i| i.left = true), area, 14, 8);
        s.update(&key(|i| i.left = true), area, 14, 8);
        s.update(&key(|i| i.left = true), area, 14, 8);
        assert_eq!(s.focus(), 0, "left stops at the first");
        assert_eq!(s.update(&key(|i| i.enter = true), area, 14, 8), SelectAction::Start(0));
    }

    #[test]
    fn the_pointer_takes_the_focus_only_by_moving_onto_a_tile() {
        let area = areas()[0];
        let mut s = LevelSelect::open(Some(1), 5);
        let resting = |p: Vec2| SelectInput { pointer: Some(p), ..SelectInput::default() };
        s.update(&resting(centre(tile_rect(area, 4))), area, 14, 5);
        assert_eq!(s.focus(), 1, "a pointer already resting on a tile when the screen opens takes nothing");
        s.update(&resting(centre(tile_rect(area, 3))), area, 14, 5);
        assert_eq!(s.focus(), 3);
        s.update(&resting(centre(tile_rect(area, 9))), area, 14, 5);
        assert_eq!(s.focus(), 3, "a locked tile never takes the focus");
    }

    #[test]
    fn the_view_marks_won_next_and_locked_and_where_the_round_is() {
        let mut campaign = Campaign::new(Levels::shipped(), None);
        campaign.won(0);
        campaign.won(1);
        let s = LevelSelect::open(Some(0), campaign.reached());
        let v = s.view(&campaign, Some(0));
        assert_eq!(v.tiles.len(), Levels::shipped().len());
        assert_eq!(v.tiles[0].state, TileState::Won);
        assert_eq!(v.tiles[1].state, TileState::Won);
        assert_eq!(v.tiles[2].state, TileState::Next);
        assert!(v.tiles[3..].iter().all(|t| t.state == TileState::Locked));
        assert!(v.tiles[0].current && v.tiles[0].focus);
        assert!(v.tiles[1..].iter().all(|t| !t.current && !t.focus));
        assert_eq!(v.tiles[13].number, 14);
    }

    #[test]
    fn a_title_wraps_onto_two_balanced_lines_and_a_long_one_is_cut() {
        assert_eq!(wrap("Glasshouse Gardens", 76, 12), vec!["Glasshouse", "Gardens"]);
        assert_eq!(wrap("Oasis Bazaar", 200, 12), vec!["Oasis Bazaar"]);
        assert_eq!(wrap(&crate::text::fold("Labirint živih mej"), 76, 12), vec!["Labirint", "zivih mej"]);
        let long = wrap("one two three four five six seven", 60, 12);
        assert_eq!(long.len(), 2);
        assert!(long[1].ends_with('~'), "{long:?}");
        assert!(long.iter().all(|l| width(l, 12) <= 60), "{long:?}");
        assert_eq!(wrap("Supercalifragilistic", 40, 12).len(), 1);
        assert!(wrap("", 60, 12).is_empty());
    }
}
