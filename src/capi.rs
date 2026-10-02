//! The `dev-tools` C ABI over `tuning.rs` - what the web page's tuning panel
//! (site/src/pages/index.astro) calls through Emscripten's `Module.ccall`,
//! and what a native transport (a local HTTP/MCP bridge, later) would bind
//! to. See docs/runtime-tuning-design.md §6. Beside the tuning, what a
//! script drives a browser tab by: an online round's readings
//! (`bb_net_stats`), the local seat's input (`bb_input`) and the window as
//! the game laid it out, buttons and the last press included
//! (`bb_ui_json`).
//!
//! Only compiled with `--features dev-tools`, so a release build exports
//! nothing. `main.rs` calls [`keep_alive`] once so every function here is
//! genuinely referenced from the binary (an unreferenced `#[no_mangle]` in
//! an rlib is otherwise free for the linker to drop), and `build.rs` lists
//! them in emcc's `EXPORTED_FUNCTIONS` so they survive wasm-ld's GC and
//! land on `Module`.
//!
//! String ownership: every `*const c_char` this module hands out points into
//! a thread-local scratch buffer that stays valid until the *next* `bb_*`
//! call on that thread. No `free` across the boundary - this is a dev tool
//! called one request at a time from the page's event loop.

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};

use serde_json::{json, Map, Value};

use crate::hud::{CornerShape, UiFrame};
use crate::math::{Rectangle, Vec2};
use crate::mode::{Driver, Session};
use crate::tuning;
use crate::view::View;
use crate::Layout;

thread_local! {
    static SCRATCH: RefCell<CString> = RefCell::new(CString::default());
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

/// Park `s` in the scratch buffer and hand out a pointer to it (valid until
/// the next call that replaces the buffer).
fn hand_out(s: String) -> *const c_char {
    // A NUL inside the payload can't come from serde_json output, but never
    // panic across an FFI boundary - degrade to an empty string instead.
    let c = CString::new(s).unwrap_or_default();
    SCRATCH.with(|slot| {
        *slot.borrow_mut() = c;
        slot.borrow().as_ptr()
    })
}

fn set_last_error(msg: String) {
    let c = CString::new(msg).unwrap_or_default();
    LAST_ERROR.with(|slot| *slot.borrow_mut() = c);
}

/// # Safety
/// `ptr` must be null or point at a NUL-terminated string that outlives the
/// call. A null pointer reads as an empty string.
unsafe fn read_str<'a>(ptr: *const c_char) -> Result<&'a str, String> {
    if ptr.is_null() {
        return Ok("");
    }
    // SAFETY: the caller guarantees `ptr` is a live NUL-terminated string.
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|e| format!("argument is not valid UTF-8: {e}"))
}

/// The schema table (see `tuning::schema_json`), as a JSON string.
#[unsafe(no_mangle)]
pub extern "C" fn bb_tuning_schema_json() -> *const c_char {
    hand_out(tuning::schema_json())
}

/// The live table as a flat JSON object.
#[unsafe(no_mangle)]
pub extern "C" fn bb_tuning_current_json() -> *const c_char {
    hand_out(tuning::current_json())
}

/// Only the rows that differ from the defaults, as a JSON object.
#[unsafe(no_mangle)]
pub extern "C" fn bb_tuning_diff_json() -> *const c_char {
    hand_out(tuning::diff_json())
}

/// The differing rows as `tunables!` table rows ("Copy as Rust").
#[unsafe(no_mangle)]
pub extern "C" fn bb_tuning_diff_rust() -> *const c_char {
    hand_out(tuning::diff_rust())
}

/// Stage a JSON patch object to land at the next frame boundary. Returns
/// the number of keys applied (>= 0), or -1 with the reason available from
/// `bb_last_error`.
///
/// # Safety
/// `json` must be null or a live NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bb_tuning_apply_json(json: *const c_char) -> c_int {
    // SAFETY: forwarded from this function's own contract.
    let text = match unsafe { read_str(json) } {
        Ok(s) => s,
        Err(e) => {
            set_last_error(e);
            return -1;
        }
    };
    match tuning::submit_json(text) {
        Ok(n) => c_int::try_from(n).unwrap_or(c_int::MAX),
        Err(e) => {
            set_last_error(e);
            -1
        }
    }
}

/// The message from the most recent failed `bb_*` call.
#[unsafe(no_mangle)]
pub extern "C" fn bb_last_error() -> *const c_char {
    LAST_ERROR.with(|slot| slot.borrow().as_ptr())
}

/// Stage a reset of every knob to its default.
#[unsafe(no_mangle)]
pub extern "C" fn bb_tuning_reset() {
    tuning::submit_reset();
}

/// Ask the main loop to restart the round at the next frame boundary.
#[unsafe(no_mangle)]
pub extern "C" fn bb_game_restart() {
    tuning::request_restart();
}

/// The online round's readings as of the last frame (`OnlineRound::stats_json`),
/// published by the frame loop; empty outside an online round.
static NET_STATS: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// A scripted input the page asked for: the move direction (0 none, 1-4 as
/// `wire::dir_code`), fire, and how many frames it still holds for. While
/// it holds, it replaces the local player's keyboard - the web build's
/// counterpart of the dev server's `input` tool, so a browser session can
/// be driven by a measurement script.
static SCRIPTED: std::sync::Mutex<Option<(u8, bool, u32)>> = std::sync::Mutex::new(None);

/// Publish this frame's online readings (the frame loop calls this,
/// through `NetStatsFeed`).
pub fn publish_net_stats(json: String) {
    *NET_STATS.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = json;
}

/// What `bb_net_stats` hands out follows the driver (`app.rs` keeps one):
/// an online round's readings on every frame one is played, and the empty
/// string once, on the first frame it is not - a round that has ended or
/// been given up is not the page's to measure. Only a change takes the
/// lock.
#[derive(Debug, Default)]
pub struct NetStatsFeed {
    /// The page holds a round's readings.
    live: bool,
}

impl NetStatsFeed {
    /// One frame: `stats` is the round's readings while the window plays
    /// one, `None` in every other mode.
    pub fn frame(&mut self, stats: Option<String>) {
        if let Some(json) = self.publishing(stats) {
            publish_net_stats(json);
        }
    }

    /// What this frame publishes, if anything.
    fn publishing(&mut self, stats: Option<String>) -> Option<String> {
        match stats {
            Some(json) => {
                self.live = true;
                Some(json)
            }
            None if self.live => {
                self.live = false;
                Some(String::new())
            }
            None => None,
        }
    }
}

/// The scripted input for this frame, if one holds, counting it down.
pub fn take_scripted_input() -> Option<crate::ai::Intent> {
    let mut slot = SCRIPTED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (dir, fire, frames) = (*slot)?;
    *slot = (frames > 1).then_some((dir, fire, frames - 1));
    Some(crate::ai::Intent { move_dir: crate::net::wire::dir_from_code(dir), fire, ..crate::ai::Intent::default() })
}

/// The online round's readings as JSON (`OnlineRound::stats_json`): the
/// link, the interpolator, the prediction and the drawn tanks, as of the
/// last frame. `""` outside an online round.
#[unsafe(no_mangle)]
pub extern "C" fn bb_net_stats() -> *const c_char {
    let s = NET_STATS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    hand_out(s)
}

/// Drive the local player for `frames` frames: `move_dir` 0 none, 1 up, 2
/// down, 3 left, 4 right; `fire` 0 or 1. `frames` 0 hands the keyboard
/// back at once.
#[unsafe(no_mangle)]
pub extern "C" fn bb_input(move_dir: c_int, fire: c_int, frames: c_int) {
    let mut slot = SCRIPTED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *slot = (frames > 0).then_some((move_dir.clamp(0, 4) as u8, fire != 0, frames as u32));
}

/// The window as the frame loop last laid it out (`window_json`): what
/// `bb_ui_json` hands the page.
static WINDOW: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Publish this frame's window (the web build's frame loop calls this).
pub fn publish_window(json: String) {
    *WINDOW.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = json;
}

/// The window as the game laid it out on its last frame, as JSON
/// (`window_json`): what a script finds a button by before it taps the
/// canvas, and reads back to see where the tap landed. `""` before the
/// first frame, and on a build whose frame loop publishes none.
#[unsafe(no_mangle)]
pub extern "C" fn bb_ui_json() -> *const c_char {
    let s = WINDOW.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    hand_out(s)
}

/// The chrome as `ui` lays it out for the session's mode: the UI scale
/// (window units per point), the window and the safe area the chrome keeps
/// to in points, whether it is laid out for touch, the input its `hints`
/// name (`keys` or `touch`, `hud::Hints`), in play and online the
/// corners' `buttons`, both `clusters` and the `minimap` picture under the
/// right one (`null` where none is drawn - a press there does nothing),
/// and the `screen_buttons` of whatever stands over the round
/// (`Session::screen_buttons`) - every rectangle in window coordinates.
/// The dev server's `status.ui`, and the heart of `bb_ui_json`.
pub fn ui_status(session: &Session, ui: &UiFrame) -> Value {
    let rect = |r: crate::Rect| json!({ "x": r.x, "y": r.y, "w": r.w, "h": r.h });
    let on_window = |r: Rectangle| {
        let r = ui.rect_to_window(r);
        json!({ "x": r.x, "y": r.y, "w": r.width, "h": r.height })
    };
    let mut v = json!({ "scale": ui.scale, "screen": rect(ui.screen), "area": rect(ui.area), "touch": ui.touch, "hints": ui.hints.name() });
    let corners = CornerShape::of(&session.play_chrome(), session.shown().players.count()).map(|shape| crate::hud::corners(ui, &shape));
    if let Some(corners) = corners {
        let buttons: Map<String, Value> = corners.buttons().into_iter().map(|(b, r)| (b.name().to_string(), on_window(r))).collect();
        v["buttons"] = Value::Object(buttons);
        v["clusters"] = json!({ "left": on_window(corners.left()), "right": on_window(corners.right) });
        v["minimap"] = corners.minimap.map_or(Value::Null, on_window);
    }
    let screen = session.screen_buttons(ui);
    if !screen.is_empty() {
        v["screen_buttons"] = Value::Object(screen.into_iter().map(|(name, r)| (name, on_window(r))).collect());
    }
    v
}

/// A press the frame loop saw: the `count`th since the window opened,
/// where it landed on the window, and whether a finger made it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Press {
    pub count: u32,
    pub at: Vec2,
    pub touch: bool,
}

/// What the frame loop knows of its window on one frame (`window_json`).
pub struct WindowReport<'a> {
    pub session: &'a Session,
    pub ui: &'a UiFrame,
    pub layout: &'a Layout,
    pub view: &'a View,
    /// The window's size in its own units - the canvas buffer's pixels on
    /// the web - and how many of them make a point.
    pub window: (i32, i32),
    pub units_per_point: f32,
    /// Frames the window has drawn, so a reader can tell a fresh readout
    /// from the one it read before.
    pub frame: u64,
    pub press: Option<Press>,
    /// What the page said its own controls take of the canvas, as the
    /// frame read it (`hud::PageOverlay`, CSS pixels).
    pub page_overlay: crate::hud::PageOverlay,
}

/// One frame's `bb_ui_json`: the `frame` it was drawn on, the `window` and
/// its `units_per_point`, the mode, the map, the level and what stands
/// over the round, the chrome (`ui_status`), where the bitmap lands
/// (`view`: its scale, offset and size, and its field area as `field`), in
/// build mode the builder's buttons by name (`MapEditor::named_buttons`:
/// the bar's - `play`, `map`, `file`, `fit`, ... - and the open popup's)
/// with the `tool` and the open `menu`, the last `press` - on the window,
/// in UI points and on the bitmap - and the `page_overlay` the frame read
/// (in CSS pixels, as the page wrote it). Every rectangle and point is the
/// window's unless named otherwise: a page turns one into CSS pixels by
/// dividing by `units_per_point` and adding the canvas's own corner.
pub fn window_json(r: &WindowReport) -> String {
    let rect = |x: Rectangle| json!({ "x": x.x, "y": x.y, "w": x.width, "h": x.height });
    let on_window = |x: Rectangle| {
        let corner = r.view.to_window(Vec2::new(x.x, x.y));
        rect(Rectangle::new(corner.x, corner.y, x.width * r.view.scale, x.height * r.view.scale))
    };
    let session = r.session;
    let field = r.layout.field;
    let press = r.press.map(|p| {
        let ui = r.ui.to_ui(p.at);
        let bitmap = r.view.to_bitmap(p.at);
        json!({ "count": p.count, "touch": p.touch, "window": [p.at.x, p.at.y], "ui": [ui.x, ui.y], "bitmap": [bitmap.x, bitmap.y] })
    });
    let mut v = json!({
        "frame": r.frame,
        "window": [r.window.0, r.window.1],
        "units_per_point": r.units_per_point,
        "mode": session.mode().name(),
        "map": session.shown().map.name,
        "level": session.level().map(|i| i + 1),
        "levels_open": session.level_select.is_some(),
        "dialog_open": session.dialog,
        "players_dialog_open": session.players_dialog,
        "ui": ui_status(session, r.ui),
        "view": { "scale": r.view.scale, "offset": [r.view.offset.x, r.view.offset.y], "bitmap": [r.view.bitmap.0, r.view.bitmap.1] },
        "field": on_window(Rectangle::new(field.x, field.y, field.w, field.h)),
        "press": press,
        "page_overlay": {
            "top": r.page_overlay.top,
            "rect": r.page_overlay.rect.map(rect),
        },
    });
    if session.mode() == Driver::Build {
        // The builder's chrome stands on the window in UI points, its
        // canvas under it through the view (`editor::BuilderFrame`).
        let frame = crate::editor::BuilderFrame { layout: *r.layout, view: *r.view, ui: *r.ui };
        let builder = &session.builder;
        let buttons: Map<String, Value> =
            builder.named_buttons(&frame).into_iter().map(|(name, b)| (name, rect(r.ui.rect_to_window(b)))).collect();
        v["builder"] = json!({
            "tool": builder.tool().name(),
            "menu": builder.open_menu(),
            "buttons": buttons,
        });
    }
    v.to_string()
}

/// Every exported entry point. `build.rs` carries the same list (with a
/// leading underscore each) for emcc - keep the two in sync;
/// `exports_match_build_rs` below checks.
pub const EXPORTS: &[(&str, *const ())] = &[
    ("bb_tuning_schema_json", bb_tuning_schema_json as *const ()),
    ("bb_tuning_current_json", bb_tuning_current_json as *const ()),
    ("bb_tuning_diff_json", bb_tuning_diff_json as *const ()),
    ("bb_tuning_diff_rust", bb_tuning_diff_rust as *const ()),
    ("bb_tuning_apply_json", bb_tuning_apply_json as *const ()),
    ("bb_last_error", bb_last_error as *const ()),
    ("bb_tuning_reset", bb_tuning_reset as *const ()),
    ("bb_game_restart", bb_game_restart as *const ()),
    ("bb_net_stats", bb_net_stats as *const ()),
    ("bb_input", bb_input as *const ()),
    ("bb_ui_json", bb_ui_json as *const ()),
];

/// Pin every entry point into the final link by taking its address through
/// an opaque call - `main.rs` calls this once at startup. Without a real
/// reference from the binary, an rlib's unreferenced `#[no_mangle]` functions
/// never make it into the link, and emcc then fails on the undefined
/// exports `build.rs` asked for.
#[inline(never)]
pub fn keep_alive() {
    for (_, f) in EXPORTS {
        std::hint::black_box(*f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_match_build_rs() {
        let build_rs = include_str!("../build.rs");
        for (name, _) in EXPORTS {
            assert!(
                build_rs.contains(&format!("_{name}")),
                "build.rs EXPORTED_FUNCTIONS is missing _{name}"
            );
        }
    }

    #[test]
    fn schema_and_error_strings_round_trip() {
        // SAFETY: the pointers come straight from this module's own scratch
        // buffers and are read before the next call replaces them.
        let schema = unsafe { CStr::from_ptr(bb_tuning_schema_json()) }.to_str().unwrap();
        assert!(schema.starts_with('['));
        assert!(schema.contains("\"tank_speed\""));

        let bad = CString::new(r#"{"nope": 1}"#).unwrap();
        // SAFETY: `bad` is a live NUL-terminated string for the call.
        let rc = unsafe { bb_tuning_apply_json(bad.as_ptr()) };
        assert_eq!(rc, -1);
        let err = unsafe { CStr::from_ptr(bb_last_error()) }.to_str().unwrap();
        assert!(err.contains("nope"), "{err}");

        let rc = unsafe { bb_tuning_apply_json(std::ptr::null()) };
        assert_eq!(rc, -1, "an empty string is not a JSON object");
    }

    fn session() -> Session {
        let mut game = crate::simulation::Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(7);
        game.map = crate::map::MapFile::from_toml_str(include_str!("../maps/default.toml")).expect("default map parses");
        game.init(crate::DEFAULT_SCREEN_WIDTH as f32, crate::DEFAULT_SCREEN_HEIGHT as f32);
        Session::new(game)
    }

    /// `bb_ui_json` is the window in its own units: a phone's canvas
    /// buffer three device pixels to the point puts the corners' BUILD
    /// button where the chrome laid it out times three, keeps the press
    /// where it landed and says where that is in points and on the bitmap;
    /// in build mode the bar's buttons come through the view. The page's
    /// controls come back as the page wrote them, in CSS pixels.
    #[test]
    fn the_window_readout_is_in_window_units() {
        let mut s = session();
        let window = (2556, 1179);
        let ui = UiFrame::new((window.0 as f32, window.1 as f32), 3.0, 1.0, crate::hud::Insets::default(), true);
        let field = s.field_size();
        let layout = Layout::bare(field.0, field.1);
        let view = View::fit(field, (window.0 as f32, window.1 as f32));
        let press = Press { count: 4, at: Vec2::new(300.0, 150.0), touch: true };
        let page_overlay = crate::hud::PageOverlay { top: 28.0, rect: Some(Rectangle::new(299.25, 3.0, 253.5, 22.0)) };
        let report = |s: &Session, layout: &Layout, view: &View| {
            window_json(&WindowReport { session: s, ui: &ui, layout, view, window, units_per_point: 3.0, frame: 9, press: Some(press), page_overlay })
        };
        let v: Value = serde_json::from_str(&report(&s, &layout, &view)).expect("JSON");
        assert_eq!(v["mode"], "play");
        assert_eq!(v["frame"], 9);
        assert_eq!(v["window"], json!([2556, 1179]));
        let build = &v["ui"]["buttons"]["build"];
        let corners = crate::hud::corners(&ui, &CornerShape::of(&s.play_chrome(), 1).expect("play draws corners"));
        let (_, drawn) = corners.buttons().into_iter().find(|(b, _)| b.name() == "build").expect("a BUILD button");
        assert_eq!(build["x"].as_f64().unwrap() as f32, drawn.x * ui.scale);
        assert_eq!(build["w"].as_f64().unwrap() as f32, drawn.width * ui.scale);
        // serde_json reads a float back to within an ulp, not exactly.
        let point = |p: &Value| Vec2::new(p[0].as_f64().unwrap() as f32, p[1].as_f64().unwrap() as f32);
        assert_eq!(point(&v["press"]["window"]), Vec2::new(300.0, 150.0));
        assert_eq!(point(&v["press"]["ui"]), Vec2::new(100.0, 50.0));
        let bitmap = view.to_bitmap(press.at);
        assert!((point(&v["press"]["bitmap"]) - bitmap).length() < 1e-3, "{} vs {bitmap:?}", v["press"]["bitmap"]);
        assert!(v.get("builder").is_none(), "no bar in play mode");
        assert_eq!(v["page_overlay"], json!({ "top": 28.0, "rect": { "x": 299.25, "y": 3.0, "w": 253.5, "h": 22.0 } }), "the page's own numbers");

        // In build mode the bar's buttons stand on the window in the UI's
        // points, over the canvas the view puts under them.
        s.driver = Driver::Build;
        let frame = crate::editor::BuilderFrame::new(ui, field, s.builder.map().class(), None);
        let v: Value = serde_json::from_str(&report(&s, &frame.layout, &frame.view)).expect("JSON");
        assert_eq!(v["mode"], "build");
        let play = &v["builder"]["buttons"]["play"];
        let drawn = ui.rect_to_window(frame.bar().play);
        let at = |k: &str| play[k].as_f64().unwrap() as f32;
        assert!((at("x") - drawn.x).abs() < 1e-3 && (at("y") - drawn.y).abs() < 1e-3, "{play} vs {drawn:?}");
        assert!((at("w") - drawn.width).abs() < 1e-3 && (at("h") - drawn.height).abs() < 1e-3);
        assert!(at("h") >= crate::hud::UI_TOUCH_PT * ui.scale - 1e-3, "a finger's height on a touch screen");
    }

    /// `bb_net_stats` carries a round's readings while one is played and
    /// nothing once it is not, taken down on the first frame after it -
    /// not the finished round's numbers for as long as the page asks.
    #[test]
    fn net_stats_are_taken_down_the_frame_the_round_is_not_played() {
        let mut feed = NetStatsFeed::default();
        assert_eq!(feed.publishing(None), None, "nothing to take down before a round");
        assert_eq!(feed.publishing(Some("{\"seat\":0}".into())), Some("{\"seat\":0}".into()));
        assert_eq!(feed.publishing(None), Some(String::new()), "the round is not played: taken down");
        assert_eq!(feed.publishing(None), None, "once, not every frame");

        // SAFETY: the pointer is this module's own scratch buffer, read
        // before the next call replaces it.
        let read = || unsafe { CStr::from_ptr(bb_net_stats()) }.to_str().unwrap().to_string();
        feed.frame(Some("{\"seat\":1}".into()));
        assert_eq!(read(), "{\"seat\":1}");
        feed.frame(None);
        assert_eq!(read(), "", "the export reads empty once the round is gone");
    }
}
