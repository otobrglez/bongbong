//! Drawing the level select (`level_select.rs` owns the state, every rect
//! and the view): the lobby's panel over the dimmed window, one tile per
//! level, painted from the [`LevelSelectView`] a frame gathered.

use sola_raylib::prelude::*;

use crate::hud::{UiFrame, BUILD_COLOR, DIM, HUD_TEXT_SIZE, TEXT, UI_SMALL_TEXT};
use crate::level_select::{
    back_rect, content_rect, panel_rect, tile_rect, LevelSelectView, TileState, TileView, TILE_NUMBER_SIZE, TILE_TITLE_SIZE,
};
use crate::math::{Color, Rectangle};
use crate::text::width;

/// The panel and the dim behind it: the lobby's and the dialogs', so the
/// screens read as one family.
const PANEL_FILL: Color = Color::new(20, 20, 24, 244);
const PANEL_SHADOW: Color = Color::new(0, 0, 0, 90);
const PANEL_EDGE: Color = Color::new(0, 0, 0, 150);
const WINDOW_DIM: Color = Color::new(0, 0, 0, 150);
/// A won tile's backing; the furthest reached one's amber wash, the
/// colour of the way on everywhere else (`NEXT LEVEL`, the banner's
/// number); a locked one's darker well.
const WON_FILL: Color = Color::new(255, 255, 255, 16);
const NEXT_FILL: Color = Color::new(255, 200, 80, 36);
const LOCKED_FILL: Color = Color::new(0, 0, 0, 80);
/// What cannot be pressed: the lobby's dead grey.
const DEAD: Color = Color::new(70, 70, 78, 255);
/// The tick on a won level: the frog's green, the one colour the game
/// already means "safe" by.
const TICK: Color = Color::new(120, 220, 90, 255);

/// The tile's number sits this far below its top edge, the title's first
/// line this far, and each further line a line and this much below.
const NUMBER_TOP: f32 = 12.0;
const TITLE_TOP: f32 = 48.0;
const TITLE_LEADING: f32 = 3.0;

/// Draw the whole screen over the window: the dim over all of it, the
/// panel centred in the chrome's area. In UI points: call inside the UI
/// camera, as the dialogs and the lobby are called.
pub fn draw_level_select<D: RaylibDraw>(d: &mut D, ui: &UiFrame, view: &LevelSelectView) {
    let (screen, area) = (ui.screen, ui.area);
    d.draw_rectangle(0, 0, screen.w.ceil() as i32, screen.h.ceil() as i32, WINDOW_DIM);
    let panel = panel_rect(area);
    d.draw_rectangle_rounded(Rectangle::new(panel.x + 4.0, panel.y + 4.0, panel.width, panel.height), 0.05, 8, PANEL_SHADOW);
    d.draw_rectangle_rounded(panel, 0.05, 8, PANEL_FILL);
    d.draw_rectangle_rounded_lines_ex(panel, 0.05, 8, 1.5, PANEL_EDGE);

    let c = content_rect(area);
    d.draw_text(&view.title, c.x as i32, c.y as i32, 22, BUILD_COLOR);
    d.draw_text(&view.sub, c.x as i32, c.y as i32 + 26, UI_SMALL_TEXT, DIM);
    for (i, tile) in view.tiles.iter().enumerate() {
        draw_tile(d, tile_rect(area, i), tile);
    }

    let back = back_rect(area);
    d.draw_rectangle_rounded_lines_ex(back, 0.2, 8, 2.0, TEXT);
    let w = width(&view.back, HUD_TEXT_SIZE);
    d.draw_text(
        &view.back,
        (back.x + (back.width - w as f32) / 2.0) as i32,
        (back.y + (back.height - HUD_TEXT_SIZE as f32) / 2.0) as i32,
        HUD_TEXT_SIZE,
        TEXT,
    );
}

/// One tile: its number over its title, a tick once won, a padlock while
/// locked, the amber of the furthest reached, a white edge where the
/// keyboard is, and a pip on the level the round behind is.
fn draw_tile<D: RaylibDraw>(d: &mut D, r: Rectangle, tile: &TileView) {
    let (fill, edge, number, title) = match tile.state {
        TileState::Won => (WON_FILL, DIM, TEXT, DIM),
        TileState::Next => (NEXT_FILL, BUILD_COLOR, BUILD_COLOR, TEXT),
        TileState::Locked => (LOCKED_FILL, DEAD, DEAD, DEAD),
    };
    d.draw_rectangle_rounded(r, 0.12, 6, fill);
    let (edge, thick) = if tile.focus { (TEXT, 2.0) } else { (edge, 1.0) };
    d.draw_rectangle_rounded_lines_ex(r, 0.12, 6, thick, edge);

    let label = tile.number.to_string();
    let w = width(&label, TILE_NUMBER_SIZE);
    d.draw_text(&label, (r.x + (r.width - w as f32) / 2.0) as i32, (r.y + NUMBER_TOP) as i32, TILE_NUMBER_SIZE, number);
    let mut y = r.y + TITLE_TOP;
    for line in &tile.lines {
        let w = width(line, TILE_TITLE_SIZE);
        d.draw_text(line, (r.x + (r.width - w as f32) / 2.0) as i32, y as i32, TILE_TITLE_SIZE, title);
        y += TILE_TITLE_SIZE as f32 + TITLE_LEADING;
    }

    let corner = Vector2::new(r.x + r.width - 18.0, r.y + 7.0);
    match tile.state {
        TileState::Won => draw_tick(d, corner, TICK),
        TileState::Locked => draw_padlock(d, corner, DEAD),
        TileState::Next => {}
    }
    if tile.current {
        // A small arrowhead in the top-left corner: you are here.
        let (x, y) = (r.x + 7.0, r.y + 7.0);
        d.draw_triangle(Vector2::new(x, y), Vector2::new(x, y + 10.0), Vector2::new(x + 7.0, y + 5.0), TEXT);
    }
}

/// A tick in an 11 x 9 box at `at`.
fn draw_tick<D: RaylibDraw>(d: &mut D, at: Vector2, color: Color) {
    let (a, b, c) = (Vector2::new(at.x, at.y + 5.0), Vector2::new(at.x + 4.0, at.y + 9.0), Vector2::new(at.x + 11.0, at.y));
    d.draw_line_ex(a, b, 2.5, color);
    d.draw_line_ex(b, c, 2.5, color);
}

/// A padlock in a 10 x 12 box at `at`: the shackle over the body.
fn draw_padlock<D: RaylibDraw>(d: &mut D, at: Vector2, color: Color) {
    d.draw_rectangle_lines_ex(Rectangle::new(at.x + 2.0, at.y - 1.0, 6.0, 8.0), 2.0, color);
    d.draw_rectangle((at.x) as i32, (at.y + 5.0) as i32, 10, 7, color);
}
