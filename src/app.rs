//! The game as a program: the command line (`Args`), the window, the
//! assets, the frame loop (`run`). The desktop binary (`src/main.rs`)
//! parses the command line and calls `run`; the iOS entry (`app::ios`,
//! started by SDL) and the Android entry (`app::android`, called by
//! raylib's `android_main` from the `android/` cdylib) call it with the
//! defaults. Lives in the library so a platform that needs a shared
//! object rather than an executable can reach it.

use crate::tuning::tuning;
use crate::ai::Intent;
use crate::editor::{BuilderInput, CanvasScreen, CliOverrides, EditorTextures};
use crate::render::game::{Effects, Textures};
use crate::hud::{self, leave_dialog_rects, players_dialog_rects, CornerButton, CornerShape, Corners, Fade, UiFrame, BAR_FILL};
use crate::level_select::SelectInput;
use crate::levels::{Campaign, Levels};
use crate::lobby::LobbyInput;
use crate::mode::{Driver, Session};
// Online play reaches a room over a socket or the rig over a thread,
// and the emscripten build has neither on the command line: the page
// passes a room code instead (docs/online-coop-prd.md §4.13).
#[cfg(not(target_os = "emscripten"))]
use crate::net::client::{Identity, RoomClient, RoomSetup, Target};
#[cfg(not(target_os = "emscripten"))]
use crate::net::round::{AnyRound, OnlineRound};
#[cfg(not(target_os = "emscripten"))]
use crate::net::transport::Transport;
use crate::render::shockwave::{RippleFx, RippleTuning};
use crate::simulation::{Game, Input, PlayerCount};
use crate::tuning;
use crate::tank::{Dir, TankKind};
use crate::touch::TouchPoint;
use crate::framing::{Screen, Seating, SightBox, ViewRules};
use crate::view::{Camera, FollowFrame, ScaleCap, View};
use crate::{
    Layout,
    PHYSICS_FIXED_DT,
    SIM_MAX_STEPS_PER_FRAME,
};
use clap::Parser;
use sola_raylib::core::game_loop;
use sola_raylib::prelude::{KeyboardKey, RaylibHandle};

/// This frame's raw movement/fire commands for both players, from the
/// keyboard. `fire` is the raw held state - whether it actually fires
/// (edge-triggered for shells, full-auto while a laser is charged) is
/// `Game::update`'s call, not this function's; see `Input::seats`.
/// Player 1 is always the arrows + Space; from two seats up, player 2 is
/// WASD + Left Shift, otherwise idle (docs/two-players.md). The keyboard
/// has no third or fourth pair of keys: the seats past these two are a
/// room's, and their intents arrive over the wire.
fn gather_intents(rl: &RaylibHandle, players: PlayerCount) -> (Intent, Intent) {
    let dir = |up: KeyboardKey, down: KeyboardKey, left: KeyboardKey, right: KeyboardKey| {
        if rl.is_key_down(up) {
            Some(Dir::Up)
        } else if rl.is_key_down(down) {
            Some(Dir::Down)
        } else if rl.is_key_down(left) {
            Some(Dir::Left)
        } else if rl.is_key_down(right) {
            Some(Dir::Right)
        } else {
            None
        }
    };
    let arrows = dir(KeyboardKey::KEY_UP, KeyboardKey::KEY_DOWN, KeyboardKey::KEY_LEFT, KeyboardKey::KEY_RIGHT);
    // The lamp key sets a lantern down (docs/volcano.md): Enter beside the
    // arrows, E beside WASD.
    let player1 = Intent {
        move_dir: arrows,
        fire: rl.is_key_down(KeyboardKey::KEY_SPACE),
        lamp: rl.is_key_down(KeyboardKey::KEY_ENTER) || rl.is_key_down(KeyboardKey::KEY_KP_ENTER),
        ..Intent::default()
    };
    if players.count() < 2 {
        return (player1, Intent::default());
    }
    let wasd = dir(KeyboardKey::KEY_W, KeyboardKey::KEY_S, KeyboardKey::KEY_A, KeyboardKey::KEY_D);
    (player1, Intent { move_dir: wasd, fire: left_shift_down(rl), lamp: rl.is_key_down(KeyboardKey::KEY_E), ..Intent::default() })
}

/// Whether the left Shift key - player 2's fire key - is held. Native reads
/// raylib's key. On the web emscripten's GLFW layer reports the DOM Shift
/// key as `GLFW_KEY_LEFT_SHIFT` whichever side was pressed (`libglfw.js`
/// maps keyCode 0x10 to the left key and never looks at `event.location`),
/// so Right Shift would fire player 2's tank too: the page keeps
/// `window.bbShift` (bit 1 = ShiftLeft, from `keydown`/`keyup` on
/// `event.code`) and this reads it once a frame instead.
#[cfg(target_os = "emscripten")]
fn left_shift_down(_rl: &RaylibHandle) -> bool {
    unsafe extern "C" {
        fn emscripten_run_script_int(script: *const std::os::raw::c_char) -> std::os::raw::c_int;
    }
    // SAFETY: a NUL-terminated literal, evaluated synchronously by the
    // emscripten runtime; the value is a plain int.
    let mask = unsafe { emscripten_run_script_int(c"(window.bbShift|0)".as_ptr()) };
    mask & 1 != 0
}

#[cfg(not(target_os = "emscripten"))]
fn left_shift_down(rl: &RaylibHandle) -> bool {
    rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
}

/// Whether a key was pressed this frame, what turns the hints back to the
/// keys (`hud::Hints`): any key but the buttons an Android phone sends from
/// outside the game - Back, Menu and the volume keys, raylib's 4, 5, 24
/// and 25. Drains raylib's queue of pressed keys, which nothing else reads
/// (every key the game acts on it asks for by name, `is_key_pressed`).
fn key_pressed(rl: &mut RaylibHandle) -> bool {
    let mut any = false;
    while let Some(key) = rl.get_key_pressed_number() {
        any |= !matches!(key, 4 | 5 | 24 | 25);
    }
    any
}

/// The seats this window plays, the first the one its arrows are cast for
/// (`indicators::picture`): the room's one in an online round, once there
/// is a replica to read, else player 1 and, on a couch, player 2. A seat
/// past the keyboard's two stands idle in a local round, a teammate on
/// this screen rather than a seat of it.
fn local_seats(session: &Session) -> Vec<u8> {
    match session.mode() {
        Driver::Online => session.online.as_ref().filter(|round| round.game().is_some()).and_then(|round| round.seat()).into_iter().collect(),
        _ => (0..session.game.players.count().min(2) as u8).collect(),
    }
}

/// How many of the window's units make a point - what the indicators'
/// sizes are given in (`indicators::in_points`): one where the window is
/// laid out in points, and where it is in device pixels, the device pixels
/// a point is - on Android the display's density over 160 of them to the
/// dp, on the web the canvas buffer's pixels to the CSS pixel
/// (`web::units_per_point`).
fn window_units_per_point(_rl: &RaylibHandle) -> f32 {
    #[cfg(target_os = "android")]
    let units = _rl.get_window_scale_dpi().x.max(1.0);
    #[cfg(target_os = "emscripten")]
    let units = web::units_per_point(_rl);
    #[cfg(not(any(target_os = "android", target_os = "emscripten")))]
    let units = 1.0;
    units
}

/// The window as the chrome lays itself out in it (`hud::UiFrame`): its
/// size, the window units a point is (`window_units_per_point`), the
/// `ui_scale` knob and the safe area - read from SDL on iOS, whose window
/// covers the whole screen, the Dynamic Island and the rounded corners
/// included; on the web the band along the canvas's top that the page's
/// own controls take on a touch screen (`web::overlay`), since the page
/// pads the canvas with `env(safe-area-inset-*)` itself; none elsewhere,
/// since Android's NativeActivity keeps a landscape window out of the
/// cutout. `touch` is whether thumbs are on the glass, which makes every
/// button a finger's size.
fn ui_frame(rl: &mut RaylibHandle, touch: bool) -> UiFrame {
    let window = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);
    #[cfg(target_os = "ios")]
    let insets = ios::safe_area_insets(rl).unwrap_or_default();
    #[cfg(target_os = "emscripten")]
    let insets = web::overlay().insets(window_units_per_point(rl));
    #[cfg(not(any(target_os = "ios", target_os = "emscripten")))]
    let insets = crate::hud::Insets::default();
    UiFrame::new(window, window_units_per_point(rl), tuning().ui_scale, insets, touch)
}

/// A rectangle of the chrome's, in UI points, in the bitmap's pixels: what
/// the off-screen arrows, laid out on the bitmap, keep out of.
fn ui_rect_on_bitmap(ui: &UiFrame, view: &View, r: crate::math::Rectangle) -> crate::math::Rectangle {
    let a = view.to_bitmap(ui.to_window(crate::math::Vec2::new(r.x, r.y)));
    let b = view.to_bitmap(ui.to_window(crate::math::Vec2::new(r.x + r.width, r.y + r.height)));
    crate::math::Rectangle::new(a.x, a.y, b.x - a.x, b.y - a.y)
}

/// This window as the framing rules see it (`framing::Screen`,
/// docs/large-maps-follow-camera.md §3): its size in points, the device
/// pixels a point is (raylib's window scale DPI), how dense they are and,
/// where the platform says, how big a point is.
///
/// - A desktop reads the monitor the window is on: its physical width
///   when it reports one - the density and the millimetres both follow -
///   and otherwise a desktop's 96 points to the inch, its size unknown.
///   GLFW gives a monitor's mode in points on macOS and in pixels
///   elsewhere.
/// - The web has no physical size to read. Its window is the canvas's
///   buffer in device pixels, which `web::units_per_point` turns back into
///   CSS pixels; on a desktop a CSS pixel is the reference pixel, 96 points
///   to the inch, while a touch screen's page (`web::touch_screen`) is a
///   phone or a tablet, framed as the app on that device is - fine, its
///   size unknown -, so a room's seats in a browser and in the app see the
///   same world.
/// - A phone or a tablet is fine (above `view_fine_ppi`), so the zoom stays
///   exact, its size unknown. Android's window is in device pixels, which
///   the scale DPI turns back into points.
fn screen(rl: &RaylibHandle) -> Screen {
    let (width, height) = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);
    #[cfg(not(target_os = "emscripten"))]
    let dpr = Some(rl.get_window_scale_dpi().x).filter(|s| s.is_finite() && *s > 0.0).unwrap_or(1.0);
    #[cfg(target_os = "emscripten")]
    {
        let units = web::units_per_point(rl);
        if web::touch_screen() {
            Screen::new(width / units, height / units, units, f32::INFINITY)
        } else {
            Screen::new(width / units, height / units, units, 96.0 * units).with_mm_per_point(25.4 / 96.0)
        }
    }
    #[cfg(target_os = "android")]
    {
        Screen::new(width / dpr, height / dpr, dpr, f32::INFINITY)
    }
    #[cfg(target_os = "ios")]
    {
        Screen::new(width, height, dpr, f32::INFINITY)
    }
    #[cfg(not(any(target_os = "emscripten", target_os = "android", target_os = "ios")))]
    {
        use sola_raylib::core::window::{get_current_monitor, get_monitor_physical_width, get_monitor_width};
        let screen = Screen::new(width, height, dpr, 96.0 * dpr);
        let monitor = get_current_monitor();
        let mm = get_monitor_physical_width(monitor) as f32;
        let mode = get_monitor_width(monitor) as f32;
        let points = if cfg!(target_os = "macos") { mode } else { mode / dpr };
        if mm > 0.0 && points > 0.0 {
            Screen { ppi: points * dpr / (mm / 25.4), ..screen }.with_mm_per_point(mm / points)
        } else {
            screen
        }
    }
}

/// The touch screen's points to the millimetre, which the builder measures
/// a cell under a finger by: a full-size iPad's point is a fifth larger
/// than a phone's - the model says so in the app, the page in a browser -,
/// and everywhere else it is `indicators::POINTS_PER_MM`.
fn touch_points_per_mm() -> f32 {
    #[cfg(target_os = "ios")]
    {
        ios::points_per_mm()
    }
    #[cfg(target_os = "emscripten")]
    {
        if web::full_size_ipad() { crate::indicators::IPAD_POINTS_PER_MM } else { crate::indicators::POINTS_PER_MM }
    }
    #[cfg(not(any(target_os = "ios", target_os = "emscripten")))]
    {
        crate::indicators::POINTS_PER_MM
    }
}

/// Device pixels per unit of the window's coordinates: what the
/// framebuffer holds across one point - 2 on a Retina desktop, 1 on the
/// web canvas and on Android, whose window is in pixels already. A
/// followed view's sub-block shift is rounded to these.
fn framebuffer_ratio(rl: &RaylibHandle) -> f32 {
    let (screen, render) = (rl.get_screen_width(), rl.get_render_width());
    if screen > 0 && render > 0 { render as f32 / screen as f32 } else { 1.0 }
}

/// Hand the page's scripts the window as this frame laid it out
/// (`capi::bb_ui_json`): the chrome in `ui`, the bitmap through `view`, and
/// the last press. Every frame that presents publishes, the builder's too.
#[cfg(all(feature = "dev-tools", target_os = "emscripten"))]
fn publish_window(rl: &RaylibHandle, session: &Session, ui: &UiFrame, layout: &Layout, view: &View, frame: u64, press: Option<crate::capi::Press>) {
    crate::capi::publish_window(crate::capi::window_json(&crate::capi::WindowReport {
        session,
        ui,
        layout,
        view,
        window: (rl.get_screen_width(), rl.get_screen_height()),
        units_per_point: window_units_per_point(rl),
        frame,
        press,
        page_overlay: web::overlay(),
    }));
}

/// How a frame lands on the window (docs/large-maps-follow-camera.md): the
/// bitmap's layout and the view that puts it on the window, and the render
/// targets that takes. An arena - and a view the dev server pinned - is the
/// bitmap of the whole field, fitted into whatever the window is
/// (`View::fit_capped`). A field map is followed: its bitmap is the world
/// this window shows, framed into the whole window, a bitmap pixel per
/// world pixel (`FollowFrame`) - `Seating::Room` in a room's round, `Local`
/// otherwise, the sight box the tuning table's. Play draws no bar: its HUD
/// stands in the window's corners (`hud::corners`). The builder's bar
/// stands on the window in UI points and its canvas under it
/// (`editor::BuilderFrame`): an arena's field fitted there, a field map's
/// canvas made to the shape of the window under the bar, the builder's own
/// camera choosing what of the map it shows. While a field map's
/// establishing shot plays (`establish.rs`) its frame keeps the follow
/// camera going underneath, but the targets hold the whole field, a texel a
/// world pixel, as an arena's do.
struct Presentation {
    /// The field it was made for, and the mode: a change of either makes it
    /// stale (`stale`).
    field: (f32, f32),
    mode: Driver,
    /// A field map's frame; `None` for the whole-field bitmap.
    followed: Option<FollowFrame>,
    /// Whether the targets are made for the establishing shot.
    establishing: bool,
    /// The view the dev server pinned, if one is.
    pinned: Option<Camera>,
    layout: Layout,
    view: View,
    /// The scene target's size and `composite`'s: a followed view's
    /// composite holds the scene target's every texel (`Game::render`).
    scene: (i32, i32),
    composite: (i32, i32),
}

impl Presentation {
    fn of(rl: &RaylibHandle, session: &Session, pinned: Option<Camera>, establishing: bool, ui: &UiFrame) -> Presentation {
        let field = session.field_size();
        let mode = session.mode();
        let window = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);
        let follows = pinned.is_none() && mode != Driver::Build && session.shown().map.class().follows();
        if follows {
            let (half_w, half_h) = tuning().sight_box_half_px();
            let seating = if mode == Driver::Online { Seating::Room } else { Seating::Local };
            let frame = FollowFrame::new(screen(rl), window, seating, SightBox::new(half_w, half_h), &ViewRules::current());
            let (scene, composite) = if establishing {
                (Camera::whole(field).target_size(), Layout::bare(field.0, field.1).window_size())
            } else {
                (frame.target_size(), frame.target_size())
            };
            return Presentation { field, mode, followed: Some(frame), establishing, pinned, layout: frame.layout, view: frame.view, scene, composite };
        }
        // The cap is for a window that can be any size - a desktop's, or
        // the web page's canvas, which fills its box on a monitor too. The
        // knob is in points, and the web's window in device pixels. A
        // phone's or a tablet's screen is never large enough for it to
        // bind.
        let cap = if cfg!(any(target_os = "ios", target_os = "android")) {
            None
        } else {
            let t = tuning();
            let units = window_units_per_point(rl);
            (t.view_max_scale > 0.0).then(|| ScaleCap { max_scale: t.view_max_scale * units, snap_half: t.view_scale_snap != 0 })
        };
        if mode == Driver::Build {
            // The bar on the window under the safe area's top - and the
            // band the web page's controls take along it on a touch screen,
            // which the UI frame counts as one -, the canvas under it.
            let class = session.builder.map().class();
            let frame = crate::editor::BuilderFrame::new(*ui, field, class, cap);
            let bitmap = frame.layout.window_size();
            let (pinned, scene) = if class.follows() { (None, bitmap) } else { (pinned, pinned.unwrap_or(Camera::whole(field)).target_size()) };
            return Presentation { field, mode, followed: None, establishing: false, pinned, layout: frame.layout, view: frame.view, scene, composite: bitmap };
        }
        // Play draws the field alone, its HUD standing in the window's
        // corners (`hud::corners`).
        let layout = Layout::bare(field.0, field.1);
        let (w, h) = layout.window_size();
        Presentation {
            field,
            mode,
            followed: None,
            establishing: false,
            pinned,
            layout,
            view: View::fit_capped((w as f32, h as f32), window, cap),
            scene: pinned.unwrap_or(Camera::whole(field)).target_size(),
            composite: layout.window_size(),
        }
    }

    /// The builder's frame on the window: this presentation's canvas bitmap
    /// and view, and the UI frame its chrome is laid out in.
    fn builder_frame(&self, ui: &UiFrame) -> crate::editor::BuilderFrame {
        crate::editor::BuilderFrame { layout: self.layout, view: self.view, ui: *ui }
    }

    /// Whether the session has moved on to another mode or field since.
    fn stale(&self, session: &Session) -> bool {
        session.mode() != self.mode || session.field_size() != self.field
    }

    /// Whether the view shows less than the whole field: a followed field
    /// map wider or taller than its view by more than half a world pixel,
    /// or a pinned zoom - where the minimap is drawn
    /// (docs/large-maps-follow-camera.md §7). An arena drawn whole, and a
    /// field map no larger than a monitor's wide view, show none.
    fn shows_part(&self) -> bool {
        let short = |visible: f32, field: f32| visible + 0.5 < field;
        match (&self.followed, &self.pinned) {
            (Some(frame), _) => short(frame.framing.visible.0, self.field.0) || short(frame.framing.visible.1, self.field.1),
            (None, Some(pin)) => !pin.shows_whole_field(),
            (None, None) => false,
        }
    }

    /// Whether this window draws the play minimap this frame
    /// (`Session::minimap_on`): a round, in play or a room's, on a screen
    /// that shows one (`minimap::MinimapRules::shown_on` - not a phone's,
    /// as `minimap_show` says), whose view shows part of the field.
    fn minimap_on(&self, screen: &Screen) -> bool {
        matches!(self.mode, Driver::Play | Driver::Online) && self.shows_part() && crate::minimap::MinimapRules::current().shown_on(screen)
    }

    /// Re-create the render targets this presentation needs where their
    /// sizes differ.
    fn fit_targets(
        &self,
        rl: &mut RaylibHandle,
        thread: &sola_raylib::prelude::RaylibThread,
        scene_target: &mut sola_raylib::prelude::RenderTexture2D,
        scene_size: &mut (i32, i32),
        composite: &mut sola_raylib::prelude::RenderTexture2D,
        composite_size: &mut (i32, i32),
    ) {
        if self.scene != *scene_size {
            *scene_size = self.scene;
            *scene_target = rl
                .load_render_texture(thread, scene_size.0 as u32, scene_size.1 as u32)
                .expect("failed re-creating scene render texture");
        }
        if self.composite != *composite_size {
            *composite_size = self.composite;
            *composite = rl
                .load_render_texture(thread, composite_size.0 as u32, composite_size.1 as u32)
                .expect("failed re-creating composite render texture");
        }
    }
}

/// The URL the page was opened on, as the page published it
/// (`site/src/scripts/room.ts`). The web build's command line, in full:
/// a browser has no argv, so the room a link names, the rooms server a
/// dev link overrides and the `?lang=` and `?weather=` a tester adds all
/// travel in here (`net::rooms::Invite`, docs/online-coop-prd.md §4.10;
/// `text::lang_from_url`, `weather::weather_from_url`).
#[cfg(target_os = "emscripten")]
const PAGE_INVITE: &std::ffi::CStr = c"(function(){try{return String(window.bbInvite||'')}catch(e){return ''}})()";

/// The browser's preferred languages, as the page published them
/// (`navigator.languages`, comma-joined): the platform's word on which
/// language to speak (docs/localization-prd.md section 4.5).
#[cfg(target_os = "emscripten")]
const PAGE_LANG: &std::ffi::CStr = c"(function(){try{return String(window.bbLang||'')}catch(e){return ''}})()";

/// Whether the browser asks for reduced motion, as the page published it
/// (`site/src/scripts/motion.ts`, the `prefers-reduced-motion` media
/// query): `reduce`, `no-preference`, or nothing.
#[cfg(target_os = "emscripten")]
const PAGE_MOTION: &std::ffi::CStr = c"(function(){try{return String(window.bbMotion||'')}catch(e){return ''}})()";

/// Whether the page is on a touch screen, as it published it
/// (`site/src/scripts/overlay.ts`, the query its controls move by): `1`,
/// or nothing.
#[cfg(target_os = "emscripten")]
const PAGE_TOUCH: &std::ffi::CStr = c"(function(){try{return String(window.bbTouch||'')}catch(e){return ''}})()";

/// Whether the page is on a full-size iPad, as it published it
/// (`site/src/scripts/overlay.ts`): `1`, or nothing.
#[cfg(target_os = "emscripten")]
const PAGE_IPAD: &std::ffi::CStr = c"(function(){try{return String(window.bbIpad||'')}catch(e){return ''}})()";

/// This browser's reconnect key, minted and kept by the page - one per
/// tab, so two tabs in one browser are two seats rather than one seat
/// taken twice.
#[cfg(all(feature = "online", target_os = "emscripten"))]
const PAGE_TOKEN: &std::ffi::CStr = c"(function(){try{return String(window.bbToken||'')}catch(e){return ''}})()";

/// Evaluate `script` in the page and copy back what it said.
///
/// The second half of the `window.bbShift` bargain (see
/// `left_shift_down`): the page publishes a fact, the game reads it.
/// These are read once at startup rather than once a frame, because a
/// command line does not change either.
///
/// Two things are load-bearing. `emscripten_run_script_string` answers
/// with a pointer into the runtime's own buffer, good only until the
/// next call, so the string is copied here and not held; and an
/// exception out of `eval` takes the runtime down with it, which is why
/// every script is wrapped and answers with an empty string rather than
/// throwing.
#[cfg(target_os = "emscripten")]
fn page_string(script: &std::ffi::CStr) -> String {
    unsafe extern "C" {
        fn emscripten_run_script_string(script: *const std::os::raw::c_char) -> *const std::os::raw::c_char;
    }
    // SAFETY: a NUL-terminated literal, evaluated synchronously; the
    // answer is a NUL-terminated string in emscripten's scratch buffer,
    // copied before anything else can run.
    let answer = unsafe { emscripten_run_script_string(script.as_ptr()) };
    if answer.is_null() {
        return String::new();
    }
    unsafe { std::ffi::CStr::from_ptr(answer) }.to_string_lossy().into_owned()
}

/// The language asked for outright: `--lang` on a desktop, the page's
/// `?lang=` on the web (a browser has no argv, and the page's URL is the
/// one place a tester can write it).
fn explicit_language(args: &Args) -> Option<String> {
    #[cfg(target_os = "emscripten")]
    {
        let _ = args;
        return crate::text::lang_from_url(&page_string(PAGE_INVITE));
    }
    #[cfg(not(target_os = "emscripten"))]
    args.lang.clone()
}

/// The sky asked for outright: `--weather` on a desktop, the page's
/// `?weather=` on the web.
fn explicit_weather(args: &Args) -> Option<crate::map::Weather> {
    #[cfg(target_os = "emscripten")]
    {
        let _ = args;
        return crate::weather::weather_from_url(&page_string(PAGE_INVITE));
    }
    #[cfg(not(target_os = "emscripten"))]
    args.weather
}

/// The languages this platform prefers, most preferred first, as the
/// platform spells them: the page's `navigator.languages` on the web,
/// SDL's list on iOS, the system properties on Android, `sys-locale`'s
/// answer everywhere else.
fn platform_languages() -> Vec<String> {
    #[cfg(target_os = "emscripten")]
    {
        page_string(PAGE_LANG).split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect()
    }
    #[cfg(target_os = "ios")]
    {
        ios::platform_languages()
    }
    #[cfg(target_os = "android")]
    {
        android::platform_languages()
    }
    #[cfg(not(any(target_os = "emscripten", target_os = "ios", target_os = "android")))]
    {
        crate::text::platform_languages()
    }
}

/// Whether this platform asks for reduced motion (`motion.rs`): the page's
/// `prefers-reduced-motion` on the web, iOS's Reduce Motion, and nothing
/// anywhere the answer would take more than one plain call - Android's
/// needs JNI; macOS, Linux and Windows are left at full motion too - so
/// the switch starts off there.
fn platform_motion() -> Option<bool> {
    #[cfg(target_os = "emscripten")]
    {
        crate::motion::from_page(&page_string(PAGE_MOTION))
    }
    #[cfg(target_os = "ios")]
    {
        Some(ios::reduce_motion())
    }
    #[cfg(not(any(target_os = "emscripten", target_os = "ios")))]
    {
        None
    }
}

/// Who a desktop, iOS or Android build is to a room
/// (docs/online-coop-prd.md §4.13), made once per run. A `--nick` picks
/// the machine's reconnect key for that name, so two clients here under
/// two names are two seats and either reclaims its own seat on a
/// reconnect, even after a restart. With no name - a phone always, a
/// desktop window started without `--nick` - the player is
/// `Identity::anonymous`: a token minted for this run and the
/// `Player #ABC102` read off it, so two such windows are two seats under
/// two names rather than one seat each takes from the other. A browser
/// mints its token per tab instead (`site/src/scripts/room.ts`).
#[cfg(all(feature = "online", not(target_os = "emscripten")))]
fn cli_identity(args: &Args) -> Identity {
    match args.nick.as_deref().map(str::trim) {
        Some(nick) if !nick.is_empty() => Identity::new(nick, format!("bongbong-{nick}")),
        _ => Identity::anonymous(),
    }
}

/// Open the player's map store (`mapstore`) for this run, unless
/// `--map-modding false` turned it off: the page's `localStorage` on the
/// web, else a `maps` directory beside the level progress - `BONGBONG_MAPS`
/// names one outright, Android's is the activity's own. With no such
/// directory the run keeps no maps, as it would with modding off.
fn enable_map_store(args: &Args) {
    if !args.map_modding {
        eprintln!("[mapstore] map modding off: SAVE writes under {}", crate::map::maps_dir().display());
        return;
    }
    #[cfg(target_os = "emscripten")]
    {
        crate::mapstore::enable(Box::new(PageMaps));
        eprintln!("[mapstore] maps kept in the page's localStorage");
    }
    #[cfg(not(target_os = "emscripten"))]
    {
        #[cfg(target_os = "android")]
        let data = android::data_dir();
        #[cfg(not(target_os = "android"))]
        let data = crate::levels::data_dir();
        let dir = std::env::var_os("BONGBONG_MAPS").filter(|d| !d.is_empty()).map(std::path::PathBuf::from).or_else(|| data.map(|d| d.join("maps")));
        match dir {
            Some(dir) => {
                eprintln!("[mapstore] maps kept in {}", dir.display());
                crate::mapstore::enable(Box::new(crate::mapstore::Dir(dir)));
            }
            None => eprintln!("[mapstore] no data directory: maps are not kept"),
        }
    }
}

/// The web build's map store: the page's `localStorage`, a map's text
/// under `bongbong.map.<key>`. Every script catches its own exceptions, so
/// a storage that throws (a private window, a full quota) only forgets -
/// a write answers whether it took.
#[cfg(target_os = "emscripten")]
struct PageMaps;

#[cfg(target_os = "emscripten")]
impl PageMaps {
    /// `key` as a JavaScript string literal naming its storage slot.
    fn slot(key: &str) -> String {
        serde_json::to_string(&format!("bongbong.map.{key}")).unwrap_or_default()
    }

    /// The page's answer to `script` (`page_string`), which must catch its
    /// own exceptions.
    fn ask(script: String) -> Option<String> {
        let script = std::ffi::CString::new(script).ok()?;
        Some(page_string(&script))
    }
}

#[cfg(target_os = "emscripten")]
impl crate::mapstore::Backend for PageMaps {
    fn read(&self, key: &str) -> Option<String> {
        // A leading mark tells a kept empty text from nothing kept.
        let slot = Self::slot(key);
        let answer = Self::ask(format!("(function(){{try{{var t=localStorage.getItem({slot});return t===null?'':'='+t}}catch(e){{return ''}}}})()"))?;
        answer.strip_prefix('=').map(str::to_string)
    }

    fn contains(&self, key: &str) -> bool {
        let slot = Self::slot(key);
        Self::ask(format!("(function(){{try{{return localStorage.getItem({slot})===null?'':'1'}}catch(e){{return ''}}}})()")).as_deref() == Some("1")
    }

    fn write(&self, key: &str, text: &str) -> Result<(), String> {
        let (slot, text) = (Self::slot(key), serde_json::to_string(text).map_err(|e| e.to_string())?);
        match Self::ask(format!("(function(){{try{{localStorage.setItem({slot},{text});return '1'}}catch(e){{return ''}}}})()")).as_deref() {
            Some("1") => Ok(()),
            _ => Err(format!("the browser would not keep {key}")),
        }
    }

    fn remove(&self, key: &str) -> Result<(), String> {
        let slot = Self::slot(key);
        Self::ask(format!("(function(){{try{{localStorage.removeItem({slot})}}catch(e){{}}return ''}})()"));
        Ok(())
    }

    fn keys(&self) -> Vec<String> {
        let script = "(function(){try{var k=[];for(var i=0;i<localStorage.length;i++){var n=localStorage.key(i);if(n&&n.indexOf('bongbong.map.')===0)k.push(n.slice(13))}return k.join('\\n')}catch(e){return ''}})()";
        Self::ask(script.to_string()).map(|keys| keys.lines().filter(|k| !k.is_empty()).map(str::to_string).collect()).unwrap_or_default()
    }
}

/// The level progress the page kept in `localStorage`: the map name of
/// the furthest level reached (docs/levels.md).
#[cfg(target_os = "emscripten")]
const PAGE_PROGRESS: &std::ffi::CStr =
    c"(function(){try{return String(localStorage.getItem('bongbong.level')||'')}catch(e){return ''}})()";

/// Where a desktop or a phone keeps its level progress: the platform's
/// data directory (`levels::progress_path`), or on Android the activity's
/// own files directory.
#[cfg(not(target_os = "emscripten"))]
fn progress_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "android")]
    {
        android::data_dir().map(|dir| dir.join("progress.toml"))
    }
    #[cfg(not(target_os = "android"))]
    {
        crate::levels::progress_path()
    }
}

/// The furthest level this player reached in an earlier session, as its
/// map's name: the page's `localStorage` on the web, a file elsewhere.
/// `None` for a first session, or a store that cannot be read.
fn load_progress() -> Option<String> {
    #[cfg(target_os = "emscripten")]
    {
        let level = page_string(PAGE_PROGRESS).trim().to_string();
        (!level.is_empty()).then_some(level)
    }
    #[cfg(not(target_os = "emscripten"))]
    {
        progress_path().and_then(|path| crate::levels::read_progress(&path))
    }
}

/// Keep `level` as the furthest level reached, where `load_progress`
/// will find it. A store that cannot be written is logged and skipped:
/// the session plays on, it only forgets.
fn save_progress(level: &str) {
    #[cfg(target_os = "emscripten")]
    {
        unsafe extern "C" {
            fn emscripten_run_script(script: *const std::os::raw::c_char);
        }
        // A level's name is a map name - letters, digits, `-` and `_`
        // (`Levels::parse`) - so it goes into the script as it is.
        if !level.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return;
        }
        let script = format!("(function(){{try{{localStorage.setItem('bongbong.level','{level}')}}catch(e){{}}}})()");
        let Ok(script) = std::ffi::CString::new(script) else { return };
        // SAFETY: a NUL-terminated script, evaluated synchronously; it
        // catches its own exceptions, so nothing unwinds into the runtime.
        unsafe { emscripten_run_script(script.as_ptr()) };
    }
    #[cfg(not(target_os = "emscripten"))]
    match progress_path() {
        Some(path) => match crate::levels::write_progress(&path, level) {
            Ok(()) => eprintln!("[levels] reached {level} ({})", path.display()),
            Err(e) => eprintln!("[levels] {e}"),
        },
        None => eprintln!("[levels] reached {level}, with nowhere to keep it"),
    }
}

/// The play clock: real frame time paid out in whole simulation steps of
/// `PHYSICS_FIXED_DT`, so `Game::update` always sees the step the dev
/// server, the probe and a replay see (docs/online-coop-prd.md §4.1). A
/// rendered frame runs the steps it has accumulated - none on every other
/// frame of a 120 Hz display, one per frame at 60 Hz, two after a hitch -
/// and at most `SIM_MAX_STEPS_PER_FRAME`: a frame owing more runs the cap
/// and forgets the rest, so a stall costs the round real time, never a
/// spiral of catch-up steps.
#[derive(Default)]
struct StepClock {
    /// Real seconds not yet paid out as a step, below one step after
    /// `advance`.
    owed: f32,
}

impl StepClock {
    /// Add a rendered frame's `dt` and take the steps it pays for.
    fn advance(&mut self, dt: f32) -> u32 {
        let cap = SIM_MAX_STEPS_PER_FRAME as f32 * PHYSICS_FIXED_DT;
        self.owed = (self.owed + dt.max(0.0)).min(cap);
        let mut steps = 0;
        while self.owed >= PHYSICS_FIXED_DT && steps < SIM_MAX_STEPS_PER_FRAME {
            self.owed -= PHYSICS_FIXED_DT;
            steps += 1;
        }
        if steps == SIM_MAX_STEPS_PER_FRAME {
            self.owed = 0.0;
        }
        steps
    }

    /// Forget what is owed: the round was not running (a dialog, the
    /// builder, the dev server's freeze), so it resumes on a fresh step
    /// rather than a burst for the time it stood still.
    fn reset(&mut self) {
        self.owed = 0.0;
    }
}

/// Command-line flags for bongbong's native binary. All optional - with none
/// given, behavior matches today's defaults exactly (random enemy count,
/// random player chassis, 1280x720 window, shadows on).
#[derive(Parser)]
#[command(name = "bongbong", about = "A pixelated tank shooter")]
pub struct Args {
    /// Override the number of enemies spawned this round. Takes precedence
    /// over the loaded map's own `tanks` default (see `-m`/`--map` below and
    /// `map::MapFile::tanks`); with neither given, falls back to a random
    /// count between `enemy_count_min` and `enemy_count_max`. 0 is a
    /// sandbox round: nobody to fight, and it never ends by wreck count.
    #[arg(short = 'e', long = "enemies")]
    enemies: Option<usize>,

    /// Override the map's mission (what ends the round): protect (keep the
    /// frog alive, wreck every enemy), hunt (kill the enemy frog first) or
    /// destroy (no frog, wreck every enemy). See docs/maps-to-levels.md.
    #[arg(long = "mission", value_enum)]
    mission: Option<crate::level::Mission>,

    /// Override the map's spawn plan: band (everyone placed at once, the
    /// default) or waves (enemies roll in through edge gates wave after
    /// wave; see --waves/--wave-size/--wave-growth/--tier-start/--tier-end).
    #[arg(long = "spawn", value_enum)]
    spawn: Option<crate::level::SpawnKind>,

    /// Waves plan: number of waves.
    #[arg(long = "waves")]
    waves: Option<u32>,

    /// Waves plan: tanks in the first wave.
    #[arg(long = "wave-size")]
    wave_size: Option<u32>,

    /// Waves plan: tanks added per wave.
    #[arg(long = "wave-growth")]
    wave_growth: Option<u32>,

    /// Waves plan: chassis tier of the first wave (light, medium, heavy,
    /// super).
    #[arg(long = "tier-start", value_enum)]
    tier_start: Option<crate::level::Tier>,

    /// Waves plan: chassis tier of the last wave.
    #[arg(long = "tier-end", value_enum)]
    tier_end: Option<crate::level::Tier>,

    /// Force the player's tank to a specific chassis - e.g. `--tank titan`
    /// for the twin-barrel super-heavy, without restarting until it happens
    /// to roll. Outranks every other way a chassis gets picked: the
    /// `player_tank` tuning knob, then the loaded map's own `tank` key, then
    /// (with none of the three set) a random roll each round. Persists
    /// across in-game restarts (R key).
    #[arg(long = "tank", value_enum)]
    tank: Option<TankKind>,

    /// Two-player mode: pin player 2's chassis, over the loaded map's own
    /// `tank2` key (else a random roll each round). Persists across
    /// restarts like `--tank`.
    #[arg(long = "tank2", value_enum)]
    tank2: Option<TankKind>,

    /// How many seats the round holds, 1 to `MAX_SEATS` - the players
    /// button in the HUD's corner switches between one and two later
    /// (docs/two-players.md). Player 1 is always the arrows + Space; two
    /// players adds player 2 on WASD + Left Shift, and the seats past
    /// those two have no keys of their own: they are a room's, and stand
    /// idle in a local round. Kept across restarts.
    #[arg(long = "players", default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=crate::MAX_SEATS as i64))]
    players: u8,

    /// The window's initial size, e.g. `--resolution 1920x1080` (default:
    /// the map's own bitmap - the field, under the builder's bar with
    /// `--editor` - at 1.5x where the monitor has the room). The battlefield
    /// itself is the map's `size` and every player in a match shares it;
    /// the window only decides how large it is drawn, letterboxed so the
    /// whole field is always on screen (view.rs). Resizable afterwards;
    /// F11 toggles borderless full screen.
    #[arg(long = "resolution", value_parser = parse_resolution)]
    resolution: Option<(i32, i32)>,

    /// Start in borderless full screen on the current monitor (F11
    /// toggles it back).
    #[arg(long = "fullscreen")]
    fullscreen: bool,

    /// The most the field is scaled up on a big screen (the
    /// `view_max_scale` knob): 1 is the classic 64 px tank, 1.5 the
    /// default, 0 fills the window whatever its size.
    #[arg(long = "zoom")]
    zoom: Option<f32>,

    /// Development aid: the left mouse button acts as a touch point, so
    /// the touch scheme (touch.rs: joystick on one half, tap-to-fire on
    /// the other) can be tried on a desktop. Off, the mouse keeps to the
    /// bar's buttons, the dialogs and the builder.
    #[arg(long = "touch-from-mouse")]
    touch_from_mouse: bool,

    /// Disable tank/shell drop shadows (on by default). Can also be toggled
    /// at runtime with the L key - see docs/sprite-shadows-design.md.
    #[arg(long = "no-shadows")]
    no_shadows: bool,

    /// Play this map instead of the levels (see docs/map-editor-design.md
    /// and docs/levels.md) - border walls and enemy spawns stay procedural
    /// on top of the map's terrain. Free play, which restarts on its own,
    /// unless the file is one of the levels' maps by name. Loaded (and
    /// validated) eagerly at CLI-parse time, so a missing/malformed map
    /// file fails fast with a clear error. With `--editor` the builder
    /// opens on this map.
    #[arg(short = 'm', long = "map", value_parser = parse_map)]
    map: Option<crate::map::MapFile>,

    /// Start on this level of levels.toml - its number, counted from 1,
    /// or its map's name - instead of the furthest one reached
    /// (docs/levels.md). Progress still moves only on a win.
    #[arg(long = "level", conflicts_with = "map")]
    level: Option<String>,

    /// Start in Build mode - the map builder - instead of playing
    /// (docs/game-editor-fusion.md). The round is set up as usual, so
    /// `PLAY` starts it on the map as edited; `--map` picks the map to
    /// edit.
    #[arg(long = "editor")]
    editor: bool,

    /// Pin the round RNG seed (decimal or 0x-hex) so the round replays
    /// identically - spawn layout, chassis/speed rolls, ground cosmetics
    /// and AI decisions all reproduce, and the R-key/auto restart replays
    /// the *same* round instead of rolling a new one. This is the repro
    /// loop for a round the probe harness flagged: paste the `seed=0x...`
    /// from its ANOMALY line here (with the same `--map`/`--enemies`) to
    /// watch that exact layout play out. See
    /// docs/gameplay-verification-design.md for what a seed does and
    /// doesn't promise (windowed runs share the probe's layout but diverge
    /// over time under variable frame dt).
    #[arg(long = "seed", value_parser = crate::parse_seed)]
    seed: Option<u64>,

    /// Load a tuning patch (a JSON object of `{"knob": value}` pairs - the
    /// dev panel's "Copy JSON" output, see docs/runtime-tuning-design.md)
    /// at startup, and keep watching the file: every edit saved to it is
    /// re-applied at the next frame boundary, so any text editor becomes a
    /// tuning UI on native. A malformed file at startup fails fast; a
    /// malformed edit later is reported on stderr and ignored.
    #[arg(long = "tuning")]
    tuning: Option<std::path::PathBuf>,

    /// Port for the local dev server the `bbmcp` MCP adapter drives
    /// (lockstep stepping, snapshots, screenshots, scenario setup - see
    /// docs/dev-server-design.md). Only in `--features dev-tools` native
    /// builds; a port already in use is reported and the game runs without
    /// the server. Falls back to `BONGBONG_DEV_PORT`, then 4747.
    #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
    #[arg(long = "dev-port")]
    dev_port: Option<u16>,

    /// Run without the dev server even though it is compiled in.
    #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
    #[arg(long = "no-dev-server")]
    no_dev_server: bool,

    /// Play the offline rig (docs/online-coop-prd.md §4.13): an
    /// authoritative round on a background thread, an in-process link
    /// with `--delay`/`--jitter`/`--loss`, and the replica in the window.
    /// The whole online client - the encoder, the snapshots, the
    /// interpolation, the feel of the hull at 80 ms - with no server, no
    /// socket and no port. `-m`, `--seed`, `--enemies`, `--tank` and the
    /// mission/spawn flags set up the round it simulates.
    #[cfg(not(target_os = "emscripten"))]
    #[arg(long = "rig")]
    rig: bool,

    /// One-way link delay in milliseconds for `--rig`.
    #[cfg(not(target_os = "emscripten"))]
    #[arg(long = "delay", default_value_t = 80)]
    delay: u64,

    /// Spread around `--delay`, milliseconds, never reordering arrivals.
    #[cfg(not(target_os = "emscripten"))]
    #[arg(long = "jitter", default_value_t = 20)]
    jitter: u64,

    /// The share of messages `--rig` drops, 0.0 to 1.0.
    #[cfg(not(target_os = "emscripten"))]
    #[arg(long = "loss", default_value_t = 0.02)]
    loss: f64,

    /// Open a room on the rooms server and play it: the code to share is
    /// printed and shown over the field, and ENTER starts the round once
    /// whoever is joining has a seat. `-m` sends this map to the room.
    #[cfg(all(feature = "online", not(target_os = "emscripten")))]
    #[arg(long = "host", conflicts_with = "join")]
    host: bool,

    /// Take a seat in the room this code names (`--join AK7QX`). The
    /// round is the host's to start.
    #[cfg(all(feature = "online", not(target_os = "emscripten")))]
    #[arg(long = "join", value_name = "CODE")]
    join: Option<String>,

    /// The name on the room's roster. Also picks this machine's device
    /// token, so two clients under different nicknames are two seats.
    /// Without it the window is `Player #` and six random letters and
    /// digits, with a device token of its own for this run.
    #[cfg(all(feature = "online", not(target_os = "emscripten")))]
    #[arg(long = "nick")]
    nick: Option<String>,

    /// The rooms server to talk to, over `BONGBONG_ROOMS` and the
    /// cluster's own (`ws://127.0.0.1:4848` for `just run-server`).
    #[cfg(all(feature = "online", not(target_os = "emscripten")))]
    #[arg(long = "rooms", value_name = "URL")]
    rooms: Option<String>,

    /// The language to play in, as a tag (`sl`, `en`), for testing a
    /// translation on a machine set to another language
    /// (docs/localization-prd.md section 4.5). Without it the game speaks
    /// the platform's language where a shipped one matches, else English;
    /// a tag the game has not got is ignored. The web build reads
    /// `?lang=` off its page instead.
    #[arg(long = "lang", value_name = "TAG")]
    lang: Option<String>,

    /// Put one sky over every map this run (docs/weather.md): clear,
    /// night, dusk, rain, storm, fog, sandstorm, snow or heat_haze, or
    /// random - a sky picked by each round's seed, so `--seed` pins it. It
    /// is the `weather_override` knob, staged like `--zoom`, so a map's own
    /// WEATHER key is left as it is and the tuning panel shows the pick.
    /// The web build reads `?weather=` off its page instead.
    #[arg(long = "weather", value_name = "SKY", value_parser = parse_weather)]
    weather: Option<crate::map::Weather>,

    /// Keep the player's maps between sessions (`mapstore`, BB-33): the
    /// builder saves on its own, a shipped map as its modified copy
    /// (`<name>-modd`) in the user's data directory - `BONGBONG_MAPS` names
    /// another - and that copy opens in the original's place until FILE >
    /// REVERT MAP. On by default; `--map-modding false` keeps the
    /// development path, where SAVE writes `maps/<name>.toml` and a file
    /// there shadows the shipped map of its name. The web, iOS and Android
    /// builds always keep them.
    #[arg(long = "map-modding", value_name = "BOOL", default_value_t = true, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true")]
    map_modding: bool,
}

/// The online round the command line asks for, with the rig's handle if
/// it is the rig: `--rig` plays against a thread, `--host` opens a room
/// on the rooms server and `--join` takes a seat in one. `None` is an
/// ordinary local session.
///
/// Whichever it is, the window's side is the same `OnlineRound` over a
/// boxed transport, so nothing past this function knows which.
#[cfg(not(target_os = "emscripten"))]
fn open_online(args: &Args, map: &crate::map::MapFile, identity: &Identity) -> Option<(AnyRound, Option<crate::net::rig::Rig>)> {
    if args.rig {
        let options = crate::net::rig::RigOptions {
            map: map.clone(),
            seed: args.seed,
            enemies: args.enemies,
            overrides: level_overrides(args),
            tank_row: args.tank.map(TankKind::row),
            quality: crate::net::loopback::LinkQuality::new(args.delay, args.jitter, args.loss),
            // The link replays with the round: one seed for both.
            link_seed: args.seed.unwrap_or(0xB0B5),
        };
        eprintln!(
            "[rig] an authoritative round on a thread, link {} ms +/- {} ms, {:.1} % loss",
            args.delay,
            args.jitter,
            args.loss * 100.0
        );
        let (rig, link) = crate::net::rig::start(options);
        let client = RoomClient::host(
            Box::new(link) as Box<dyn Transport>,
            Identity::new("rig", "tok-rig"),
            RoomSetup::default(),
        );
        return Some((OnlineRound::new(client, "RIG"), Some(rig)));
    }
    #[cfg(feature = "online")]
    {
        use crate::net::rooms::{RoomCode, RoomsHost};
        if !args.host && args.join.is_none() {
            return None;
        }
        let host = RoomsHost::resolve(args.rooms.as_deref());
        let identity = identity.clone();
        let target = match args.join.as_deref().map(RoomCode::parse) {
            Some(Ok(code)) => Target::Join(code),
            Some(Err(e)) => {
                eprintln!("[online] {e}");
                std::process::exit(2);
            }
            // `-m` carries the whole map to the room; the lobby's own
            // `HOST` sends a shipped map's name instead.
            None => {
                let setup = RoomSetup {
                    map: map.name.clone().unwrap_or_else(|| "default".into()),
                    map_toml: args.map.as_ref().and_then(|m| m.to_toml_string().ok()),
                    mission: args.mission.unwrap_or(crate::level::Mission::Protect),
                    seed: args.seed,
                };
                // The clear check: a map of one's own goes to a room only
                // once won as it stands (`MapFile::hostable`). Refused, the
                // room is never dialled and the lobby opens on its closed
                // face saying why, the way a socket that never opened does.
                if args.map.as_ref().is_some_and(|m| !m.hostable()) {
                    let why = crate::text::text().get(crate::text::keys::NOTE_NOT_CLEARED);
                    eprintln!("[online] {why}");
                    let refused = Box::new(crate::net::transport::Failed::new(why)) as Box<dyn Transport>;
                    return Some((OnlineRound::new(RoomClient::host(refused, identity, setup), "ROOM"), None));
                }
                Target::Host(setup)
            }
        };
        return Some((OnlineRound::new(crate::net::client::connect(&host, identity, target), "ROOM"), None));
    }
    #[cfg(not(feature = "online"))]
    {
        let _ = identity;
        None
    }
}

/// The mission and spawn overrides the command line carries, shared by
/// the local round and the rig's authoritative one.
fn level_overrides(args: &Args) -> crate::level::LevelOverrides {
    crate::level::LevelOverrides {
        mission: args.mission,
        spawn: args.spawn,
        waves: args.waves,
        wave_size: args.wave_size,
        wave_growth: args.wave_growth,
        tier_start: args.tier_start,
        tier_end: args.tier_end,
    }
}

fn parse_weather(s: &str) -> Result<crate::map::Weather, String> {
    crate::map::Weather::parse(s).ok_or_else(|| {
        let names: Vec<&str> = crate::map::Weather::ALL.iter().map(|w| w.name()).collect();
        format!("unknown weather '{s}': one of {}", names.join(", "))
    })
}

fn parse_map(s: &str) -> Result<crate::map::MapFile, String> {
    crate::map::MapFile::load(std::path::Path::new(s))
}

/// The battlefield a normal (non-`--editor`) round loads when `-m`/`--map`
/// wasn't given. Embedded into the binary at compile time (`include_str!`)
/// rather than read from `maps/default.toml` on disk at startup - neither
/// the wasm/web build's emscripten virtual filesystem nor a cargo-dist
/// native release archive bundles anything outside `static/` (see
/// `MapFile::from_toml_str`'s doc comment and CLAUDE.md's Web/wasm build and
/// Releases sections), so a disk read here would fail in both of this
/// project's actual distribution paths - it only ever worked when run from
/// a `cargo run` checkout with `maps/` sitting right there. `cargo watch -x
/// "run"` still picks up edits to the on-disk `maps/default.toml` live in
/// dev, since `include_str!` makes rustc treat it as a compile input and
/// trigger a rebuild.
fn default_map() -> crate::map::MapFile {
    let mut map = crate::map::MapFile::from_toml_str(include_str!("../maps/default.toml"))
        .expect("failed parsing the embedded default map");
    map.name = Some("default".to_string());
    map
}

/// Parses a `WxH` string (e.g. `1920x1080`) into a `(width, height)` pair,
/// validating both parts are positive integers.
fn parse_resolution(s: &str) -> Result<(i32, i32), String> {
    let (w, h) = s
        .split_once('x')
        .ok_or_else(|| format!("invalid resolution '{s}': expected format WxH, e.g. 1920x1080"))?;
    let width: i32 = w
        .trim()
        .parse()
        .map_err(|_| format!("invalid resolution '{s}': width '{w}' is not a valid integer"))?;
    let height: i32 = h
        .trim()
        .parse()
        .map_err(|_| format!("invalid resolution '{s}': height '{h}' is not a valid integer"))?;
    if width <= 0 || height <= 0 {
        return Err(format!(
            "invalid resolution '{s}': width and height must be positive"
        ));
    }
    Ok((width, height))
}

// raylib's PLATFORM_WEB build defaults to OpenGL ES2, and the iOS build is
// raylib on its SDL backend with the ES 2.0 renderer (tools/setup_ios.sh);
// both only accept GLSL ES 100 shaders - desktop's `#version 330` files
// won't compile there. static/web/ holds GLSL ES 100 ports of the same
// effects (see CLAUDE.md).
fn shader_path(name: &str) -> String {
    if crate::EMBEDDED { format!("static/web/{name}") } else { format!("static/{name}") }
}


/// Native-only tuning transport: re-apply `--tuning <file>` whenever its
/// mtime changes (polled every `POLL_FRAMES` frames - a stat, not a read),
/// so editing the JSON in any editor is a live tuning UI. Web gets the same
/// effect through the page's panel and capi.rs instead.
struct TuningWatch {
    path: std::path::PathBuf,
    last_modified: Option<std::time::SystemTime>,
    frames: u32,
}

impl TuningWatch {
    const POLL_FRAMES: u32 = 30;

    fn new(path: &std::path::Path) -> Self {
        Self {
            path: path.to_path_buf(),
            last_modified: std::fs::metadata(path).and_then(|m| m.modified()).ok(),
            frames: 0,
        }
    }

    fn poll(&mut self) {
        self.frames += 1;
        if self.frames < Self::POLL_FRAMES {
            return;
        }
        self.frames = 0;
        let Ok(modified) = std::fs::metadata(&self.path).and_then(|m| m.modified()) else {
            return;
        };
        if self.last_modified == Some(modified) {
            return;
        }
        self.last_modified = Some(modified);
        match tuning::submit_file(&self.path) {
            Ok(n) => eprintln!("[tuning] reloaded {n} knob(s) from {}", self.path.display()),
            Err(e) => eprintln!("[tuning] ignored edit: {e}"),
        }
    }
}

/// The three ripple post-effects, compiled once: they measure every ring in
/// the standard field (`shockwave::RIPPLE_FRAME`) whatever the map.
fn load_ripples(rl: &mut RaylibHandle, thread: &sola_raylib::prelude::RaylibThread) -> (RippleFx, RippleFx, RippleFx) {
    let shock = RippleFx::load(
        rl,
        thread,
        &shader_path("shockwave.fs"),
        RippleTuning {
            speed: tuning().shockwave_speed,
            width: tuning().shockwave_width,
            strength: tuning().shockwave_strength * tuning().screen_fx_intensity,
            duration: tuning().shockwave_duration,
        },
    );
    let muzzle = RippleFx::load(
        rl,
        thread,
        &shader_path("muzzle_flash.fs"),
        RippleTuning {
            speed: tuning().muzzle_flash_speed,
            width: tuning().muzzle_flash_width,
            strength: tuning().muzzle_flash_strength,
            duration: tuning().muzzle_flash_duration,
        },
    );
    let impact = RippleFx::load(
        rl,
        thread,
        &shader_path("impact.fs"),
        RippleTuning {
            speed: tuning().impact_flash_speed,
            width: tuning().impact_flash_width,
            strength: tuning().impact_flash_strength,
            duration: tuning().impact_flash_duration,
        },
    );
    (shock, muzzle, impact)
}

/// The whole game, from window to loop, for one set of options. `main`
/// fills them from the command line on desktop; the iOS entry runs them
/// with the defaults.
pub fn run(args: Args) {
    // Keep the `dev-tools` C API (src/capi.rs) linked into this binary: an
    // `extern "C"` function nobody here references is fair game for the
    // linker to drop before emcc's EXPORTED_FUNCTIONS (build.rs) can export
    // it. See `capi::keep_alive`.
    #[cfg(feature = "dev-tools")]
    crate::capi::keep_alive();

    // Started from BongBong.app, the game works out of the bundle's
    // Resources and saves maps under the user's data directory.
    #[cfg(target_os = "macos")]
    macos::enter_bundle();
    // The player's maps (BB-33), before anything opens one.
    enable_map_store(&args);

    // The language every string is drawn in (docs/localization-prd.md
    // section 4.5): an explicit `--lang` - or the page's `?lang=` on the
    // web - when it names a shipped language, else the platform's own
    // list, else English. Set once, here, before anything gathers a
    // string; the dev server's `lang` tool is the only later writer.
    let language = crate::text::set_language(crate::text::choose(explicit_language(&args).as_deref(), &platform_languages()));
    eprintln!("[text] language {language}");
    // The platform's word on motion (`motion.rs`), read once like the
    // language: the `reduce_motion` row follows it unless set.
    crate::motion::set_platform(platform_motion());
    // A page on a touch screen is framed as the app on that device
    // (`screen`), read once like the motion switch.
    #[cfg(target_os = "emscripten")]
    web::set_touch_screen(page_string(PAGE_TOUCH).trim() == "1");
    // And a full-size iPad's larger point, which the builder measures a
    // cell under a finger by (`touch_points_per_mm`).
    #[cfg(target_os = "emscripten")]
    web::set_full_size_ipad(page_string(PAGE_IPAD).trim() == "1");
    eprintln!(
        "[motion] the platform asks for {}",
        match crate::motion::platform() {
            Some(true) => "reduced motion",
            Some(false) => "full motion",
            None => "nothing (full motion)",
        }
    );

    // The battlefield is the map's (`MapFile::field_size`); the bitmap the
    // game draws is that field - under the builder's bar in Build
    // (docs/hud-and-builder-layout-design.md) - and the window is whatever
    // the player makes it - the bitmap is fitted into it by `view::View`.
    // The simulation, the physics, the maps and the probe only ever see
    // the field.
    // The levels (docs/levels.md): a session opens on the furthest one
    // this player has reached, or on `--level`; `-m` is free play on the
    // map it names.
    let campaign = Campaign::new(Levels::shipped(), load_progress().as_deref());
    let map = match &args.map {
        Some(map) => map.clone(),
        None => {
            let start = match args.level.as_deref() {
                Some(spec) => campaign.levels.find(spec).unwrap_or_else(|| {
                    eprintln!("[levels] no level {spec:?}: a number from {} to {} or a level's map name", campaign.levels.first_number(), campaign.levels.last_number());
                    std::process::exit(2);
                }),
                None => campaign.reached(),
            };
            campaign.map(start).unwrap_or_else(|e| {
                eprintln!("[levels] level {}: {e}; playing the default map", campaign.levels.number(start));
                default_map()
            })
        }
    };
    let (screen_width, screen_height) = {
        let (w, h) = map.field_size();
        (w.round() as i32, h.round() as i32)
    };
    // The window of the first frame: the field alone in Play, under the
    // builder's bar (`HUD_BAR_HEIGHT`) with `--editor`.
    let bitmap = Layout::for_field(screen_width as f32, screen_height as f32).window_size();
    // The field a desktop window opens for: the map's when it is shown
    // whole, the standard field's when the camera follows it - a followed
    // map is framed into any window, so its window need not hold the
    // field, and one that tried would outgrow the monitor.
    #[cfg(not(any(target_os = "ios", target_os = "android", target_os = "emscripten")))]
    let opening = {
        let (w, h) = if map.class().follows() {
            (crate::DEFAULT_SCREEN_WIDTH as f32, crate::DEFAULT_SCREEN_HEIGHT as f32)
        } else {
            (screen_width as f32, screen_height as f32)
        };
        if args.editor { Layout::for_field(w, h).window_size() } else { Layout::bare(w, h).window_size() }
    };
    // iOS: the window is the screen, and raylib's SDL backend sizes its
    // render target from the size InitWindow is asked for (it never reads
    // the window back), so the screen's point size has to go in here.
    #[cfg(target_os = "ios")]
    let (window_width, window_height) = {
        ios::set_hints();
        ios::screen_size_points().unwrap_or(bitmap)
    };
    // Android: a zero in either dimension makes raylib size the framebuffer
    // from the display (rcore_android.c, SetupFramebuffer): native pixels,
    // identity scale, touch in the same space; the manifest fixes
    // landscape. A non-zero request would be rendered at that size and
    // upscaled by the compositor instead, on top of `view::View`.
    #[cfg(target_os = "android")]
    let (window_width, window_height) = (0, 0);
    // The web: the window is the canvas's buffer, the canvas's box in
    // device pixels (`web::follow_canvas`), which the page has laid out by
    // the time the runtime starts.
    #[cfg(target_os = "emscripten")]
    let (window_width, window_height) = web::canvas_buffer().map_or(bitmap, |(size, _)| size);
    // A desktop window opens at the size it will play at: the opening
    // field's bitmap at the scale cap (1.5x), shrunk where the monitor has
    // less room - never past half the bitmap, the smallest window that
    // still reads.
    #[cfg(not(any(target_os = "ios", target_os = "android", target_os = "emscripten")))]
    let (window_width, window_height) = args.resolution.unwrap_or_else(|| {
        let zoom = args.zoom.unwrap_or(tuning().view_max_scale);
        let open_at = if zoom > 0.0 { zoom.min(1.5).max(1.0) } else { 1.5 };
        let monitor = sola_raylib::core::window::get_current_monitor();
        let (mw, mh) = (sola_raylib::core::window::get_monitor_width(monitor), sola_raylib::core::window::get_monitor_height(monitor));
        let room = if mw > 0 && mh > 0 {
            ((mw - 80) as f32 / opening.0 as f32).min((mh - 120) as f32 / opening.1 as f32)
        } else {
            open_at
        };
        let scale = open_at.min(room).max(0.5);
        (((opening.0 as f32) * scale).round() as i32, ((opening.1 as f32) * scale).round() as i32)
    });

    let mut builder = sola_raylib::init();
    builder.size(window_width, window_height).title(&format!("BongBong! v{}", env!("CARGO_PKG_VERSION")));
    // On the web the window follows the canvas's box once a frame
    // (`web::follow_canvas`); raylib's resizable flag would size it to the
    // tab instead. On iOS the window is the screen, drawn at the panel's
    // full density (the raylib build carries
    // tools/ios/raylib-sdl-highdpi.patch for that; screen coordinates,
    // touch included, stay in points). Native windows resize freely and
    // draw at the panel's real density.
    if !crate::EMBEDDED {
        builder.resizable().highdpi();
    }
    #[cfg(target_os = "ios")]
    builder.fullscreen().highdpi().vsync();
    let (mut rl, thread) = builder.build();
    #[cfg(target_os = "macos")]
    macos::dock_icon();
    #[cfg(target_os = "ios")]
    {
        ios::route_default_framebuffer(&mut rl);
        ios::log_screen_geometry(&mut rl);
    }
    #[cfg(target_os = "android")]
    eprintln!(
        "bongbong: Android screen {}x{}, render {}x{}",
        rl.get_screen_width(),
        rl.get_screen_height(),
        rl.get_render_width(),
        rl.get_render_height()
    );
    if !crate::EMBEDDED {
        // Half the standard field's bitmap is the smallest window that
        // still reads; a larger map is fitted or followed into it.
        let standard = Layout::for_field(crate::DEFAULT_SCREEN_WIDTH as f32, crate::DEFAULT_SCREEN_HEIGHT as f32).window_size();
        rl.set_window_min_size(standard.0 / 2, standard.1 / 2);
        if args.fullscreen {
            rl.toggle_borderless_windowed();
        }
    }
    // ES 2's draw batch is a quarter of the desktop's; give it the
    // desktop's (`render::batch`). The frame's closure holds it, since on
    // the web `game_loop::run` returns while the loop goes on, and natively
    // drops the closure before the handle.
    #[cfg(any(target_os = "ios", target_os = "android", target_os = "emscripten"))]
    let batch = crate::render::batch::RenderBatch::load();
    // raylib closes the window on Esc by default; here Esc keeps playing
    // in the leave dialog and dismisses a builder menu, so it must never
    // reach `window_should_close`.
    rl.set_exit_key(None);

    let tanks_texture = rl
        .load_texture(&thread, "static/scifi_tanks_sheet.png")
        .expect("failed loading tanks texture");
    let shells_texture = rl
        .load_texture(&thread, "static/shells.png")
        .expect("failed loading shells texture");
    let plasma_texture = rl
        .load_texture(&thread, "static/plasma.png")
        .expect("failed loading plasma texture");
    let minigun_bullets_texture = rl
        .load_texture(&thread, "static/minigun_bullets.png")
        .expect("failed loading minigun bullets texture");
    // One ground tileset and one tall-grass sheet per `map::Theme`, all
    // loaded up front and indexed by `Theme::ALL` position: the pair the
    // frame draws with is the live map's, so a builder THEME change or a
    // restart on another map switches at once with no reload.
    let ground_textures: Vec<sola_raylib::prelude::Texture2D> = crate::map::Theme::ALL
        .iter()
        .map(|t| {
            rl.load_texture(&thread, t.ground_texture_path())
                .unwrap_or_else(|e| panic!("failed loading {} ground texture: {e}", t.name()))
        })
        .collect();
    let grass_textures: Vec<sola_raylib::prelude::Texture2D> = crate::map::Theme::ALL
        .iter()
        .map(|t| {
            rl.load_texture(&thread, t.grass_texture_path())
                .unwrap_or_else(|e| panic!("failed loading {} grass texture: {e}", t.name()))
        })
        .collect();
    let theme_index = |t: crate::map::Theme| crate::map::Theme::ALL.iter().position(|x| *x == t).expect("every theme is in ALL");
    let trees_texture = rl
        .load_texture(&thread, "static/trees_sheet.png")
        .expect("failed loading trees texture");
    let towers_texture = rl
        .load_texture(&thread, "static/towers_sheet.png")
        .expect("failed loading towers texture");
    let crates_texture = rl
        .load_texture(&thread, "static/crates_sheet.png")
        .expect("failed loading crates texture");
    let pickup_glyphs_texture = rl
        .load_texture(&thread, "static/pickup_glyphs.png")
        .expect("failed loading pickup symbols texture");
    let portal_texture = rl
        .load_texture(&thread, "static/portal_sheet.png")
        .expect("failed loading portal texture");
    let tank_glow_texture = rl
        .load_texture(&thread, "static/scifi_tanks_glow.png")
        .expect("failed loading tank glow texture");
    let tank_modules_texture = rl
        .load_texture(&thread, "static/tank_modules.png")
        .expect("failed loading tank modules texture");
    let tank_modules_glow_texture = rl
        .load_texture(&thread, "static/tank_modules_glow.png")
        .expect("failed loading tank modules glow texture");
    let missile_texture = rl
        .load_texture(&thread, "static/missile.png")
        .expect("failed loading missile texture");
    let tracks_texture = rl
        .load_texture(&thread, "static/tracks.png")
        .expect("failed loading tracks texture");
    let obstacles_texture = rl
        .load_texture(&thread, "static/walls_sheet.png")
        .expect("failed loading obstacles texture");
    let props_texture = rl
        .load_texture(&thread, "static/props_sheet.png")
        .expect("failed loading props texture");
    let target_texture = rl
        .load_texture(&thread, "static/target_sheet.png")
        .expect("failed loading target texture");
    let barrel_explosion_texture = rl
        .load_texture(&thread, "static/barrel_explosion.png")
        .expect("failed loading barrel explosion texture");
    // One full clip set per colour variant (see `frog::FROG_VARIANT_DIRS`) -
    // `Frog::variant` (rolled per round in `Game::init`) picks which one
    // `game.rs::render` draws from. Loaded up front like every other
    // texture, kept alive for the whole game loop.
    let frog_textures: Vec<crate::render::frog::FrogVariantTextures> = crate::frog::FROG_VARIANT_DIRS
        .iter()
        .map(|dir| crate::render::frog::FrogVariantTextures {
            idle: rl
                .load_texture(&thread, &format!("static/toxic_frog/{dir}/idle.png"))
                .expect("failed loading frog idle texture"),
            hurt: rl
                .load_texture(&thread, &format!("static/toxic_frog/{dir}/hurt.png"))
                .expect("failed loading frog hurt texture"),
            hop: rl
                .load_texture(&thread, &format!("static/toxic_frog/{dir}/hop.png"))
                .expect("failed loading frog hop texture"),
            attack: rl
                .load_texture(&thread, &format!("static/toxic_frog/{dir}/attack.png"))
                .expect("failed loading frog attack texture"),
            explosion: rl
                .load_texture(&thread, &format!("static/toxic_frog/{dir}/explosion.png"))
                .expect("failed loading frog explosion texture"),
        })
        .collect();
    let eraser_texture = rl
        .load_texture(&thread, "static/ui/eraser.png")
        .expect("failed loading eraser texture");

    let (mut shock_fx, mut muzzle_fx, mut impact_fx) = load_ripples(&mut rl, &thread);
    // The plasma orb and flame jet shaders. A driver that cannot compile
    // them still plays: the bolt flies as its baked sprite and the stream
    // is its particles.
    let mut shot_shaders = match crate::render::shot_shaders::ShotShaders::load(&mut rl, &thread) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("[render] shot shaders unavailable, drawing the plain shots: {e}");
            None
        }
    };
    // The weather's passes (docs/weather.md). A driver that cannot compile
    // them draws every sky without them (`weather::plain`), so the night
    // stays dark there too; the round plays the same either way.
    let mut weather_fx = crate::render::weather::WeatherFx::load(&mut rl, &thread);
    // The world an arena shows past its field in the window's margins
    // (`margin.rs`): made for each round's floor and kept.
    let mut margin_fx = crate::render::margin::MarginFx::default();
    // The short-lived particle layer lives here rather than on `Game`:
    // it is presentation only, so nothing in the simulation can see it and
    // it is free to use `rand::rng()` (see fx.rs). The web build starts at
    // a lower density - wasm is the tighter budget and a dense wave is
    // where that shows.
    let mut fx = crate::fx::Fx::default();
    // What the screen cannot see (indicators.rs, docs/large-maps-follow-
    // camera.md section 7): a memory per local seat on this screen, fed
    // every step's events like the particle layer - presentation only, and
    // a new round, seat or replica starts it over by itself.
    let mut awareness = crate::indicators::ScreenAwareness::default();
    // The frog's voice in a training round (bubble.rs,
    // docs/training-stage.md): fed every step's events like the
    // indicators, aged on the time the round ran.
    let mut voice = crate::bubble::FrogVoice::default();
    // The GPU copies of what the round and the builder bake: the floor
    // shade (`ground::GroundGrid::shade`), the lava's banks and the
    // pictures it keeps between frames, the cones
    // (`Game::with_block_images`) - uploaded where a bake changed them, a
    // new round or a builder edit, and drawn a quad each.
    let mut round_blocks = crate::render::canvas::BlockTextures::default();
    let mut builder_shade = crate::render::canvas::BlockTexture::default();
    // The minimaps (minimap.rs): the round's, baked once per round and
    // patched where a tile dies, and the builder's navigator's, repainted
    // with every stroke - each uploaded only where it changed.
    let mut round_minimap = crate::minimap::RoundMinimap::default();
    let mut round_minimap_texture = crate::render::canvas::BlockTexture::default();
    let mut builder_minimap_texture = crate::render::canvas::BlockTexture::default();
    // The Load list's thumbnails, held only while the list shows them.
    let mut builder_thumbnails = crate::editor::ThumbnailTextures::default();
    // The builder's own scene target, for a canvas zoomed or a field map
    // (`editor::render::BuilderScene`): made when first needed.
    let mut builder_scene = crate::editor::render::BuilderScene::default();
    // iOS dev-tools builds report frame time to the console every few
    // seconds: the phone has no keyboard for the overlay cycle and its dev
    // server is not reachable from the Mac, so the console is the one
    // channel that says whether the frame budget holds on real hardware.
    #[cfg(all(feature = "dev-tools", any(target_os = "ios", target_os = "android")))]
    let mut phone_frame_stats = FrameStats::default();
    if crate::EMBEDDED {
        // A literal patch of one known knob: it cannot fail, and there is
        // nothing sensible to do at startup if it somehow did.
        let _ = tuning::submit_json(r#"{"fx_density": 0.5}"#);
    }

    // Pass 1's target: the part of the world the frame shows (`Camera`), a
    // texel per world pixel - the whole field for an arena, the view and a
    // block of margin for a followed field map. Re-created when that
    // changes size (a dev-server `restart` on a map of another size, the
    // builder loading one, a `camera` pin, a resized window).
    let mut scene_size = Camera::whole((screen_width as f32, screen_height as f32)).target_size();
    let mut scene_target = rl
        .load_render_texture(&thread, scene_size.0 as u32, scene_size.1 as u32)
        .expect("failed creating scene render texture");
    // The composited bitmap - the field, under the builder's bar in Build -
    // that `view::present` fits into the window, or a followed view's world
    // alone, the scene target's size (`Game::render`); re-created when that
    // size changes.
    let mut composite_size = bitmap;
    let mut composite = rl
        .load_render_texture(&thread, bitmap.0 as u32, bitmap.1 as u32)
        .expect("failed creating composite render texture");
    // The follow camera's state (follow.rs), and what it last followed in -
    // a room's round or the local one, and the map - so a change of either
    // cuts.
    let mut follow = crate::follow::Follow::default();
    let mut followed_in: Option<(bool, Option<String>, (f32, f32))> = None;
    // A field map's establishing shot (establish.rs): the round it opens
    // and how far it has come.
    let mut establish = crate::establish::Establish::default();
    // The second half of a couch's split screen (follow::Split): its own
    // scene target and composite at the first half's size, made the first
    // time a split opens and kept for the next.
    let mut split_targets: Option<(sola_raylib::prelude::RenderTexture2D, sola_raylib::prelude::RenderTexture2D, (i32, i32))> = None;
    let mut touch = crate::touch::TouchScheme::default();
    // How far each corner cluster has faded under the fight (`hud::Fade`).
    let mut fade = Fade::default();
    let touch_from_mouse = args.touch_from_mouse;

    // `--zoom` is the `view_max_scale` knob, staged like a `--tuning`
    // patch so the dev panel shows the value in force.
    if let Some(zoom) = args.zoom {
        let _ = tuning::submit_json(&format!(r#"{{"view_max_scale": {}}}"#, zoom.clamp(0.0, 8.0)));
        tuning::apply_pending();
    }
    // `--weather` (the page's `?weather=`) is the `weather_override` knob,
    // staged the same way; a `--tuning` file below can still set it.
    if let Some(weather) = explicit_weather(&args) {
        let _ = tuning::submit_json(&format!(r#"{{"weather_override": {}}}"#, weather.index()));
        tuning::apply_pending();
        eprintln!("[weather] {} over every map", weather.name());
    }
    if let Some(path) = &args.tuning {
        match tuning::submit_file(path) {
            Ok(n) => eprintln!("[tuning] loaded {n} knob(s) from {}", path.display()),
            Err(e) => {
                eprintln!("[tuning] {e}");
                std::process::exit(2);
            }
        }
        tuning::apply_pending();
    }
    let mut tuning_watch = args.tuning.as_deref().map(TuningWatch::new);

    let mut game = Game::default();
    game.enemy_count_override = args.enemies;
    game.level_overrides = level_overrides(&args);
    game.show_intro = true;
    game.player_row_override = args.tank.map(TankKind::row);
    game.player2_row_override = args.tank2.map(TankKind::row);
    game.players = PlayerCount::from_count(args.players as usize).expect("clap limits --players to the seats");
    game.shadows_enabled = !args.no_shadows;
    game.seed_override = args.seed;
    game.map = map;
    game.init(screen_width as f32, screen_height as f32);

    // The two modes (docs/game-editor-fusion.md): the round and the map
    // builder, whichever is live. `--editor` starts on the builder.
    let mut session = Session::new(game);
    session.set_campaign(campaign);
    session.builder.cli_overrides = CliOverrides {
        tanks: args.enemies.is_some(),
        tank: args.tank.is_some(),
        tank2: args.tank2.is_some(),
        mission: args.mission.is_some(),
        spawn: args.spawn.is_some(),
        waves: args.waves.is_some(),
        wave_size: args.wave_size.is_some(),
        wave_growth: args.wave_growth.is_some(),
        tier_start: args.tier_start.is_some(),
        tier_end: args.tier_end.is_some(),
    };
    if args.editor {
        session.driver = Driver::Build;
    }
    // `--rig`, `--host` or `--join`: the window plays a round somebody
    // else simulates. The local round above is still built and still
    // sits there untouched, which is what leaving an online round comes
    // back to. The rig's handle is held for the rest of `run` - the
    // window's whole life - and stops its thread on the way out.
    #[cfg(not(target_os = "emscripten"))]
    let room_map = session.game.map.clone();
    // The rooms server the lobby's invite link names and its buttons
    // dial, and the name this player takes into a room.
    #[cfg(all(feature = "online", not(target_os = "emscripten")))]
    {
        session.rooms = crate::net::rooms::RoomsHost::resolve(args.rooms.as_deref());
        let identity = cli_identity(&args);
        session.nick = identity.nick;
        session.token = identity.device_token;
    }
    // The web build's command line is the page it was opened on: the
    // room a `/j/AK7QX` or `?join=AK7QX` link names, the `?rooms=`
    // override a dev QR carries so a scan reaches the laptop rather
    // than the cluster, and this tab's own device token. A link naming
    // a room takes the window straight into the lobby on it.
    #[cfg(all(feature = "online", target_os = "emscripten"))]
    {
        let invite = crate::net::rooms::Invite::parse(&page_string(PAGE_INVITE));
        if let Some(host) = invite.rooms_host() {
            session.rooms = host;
        }
        // The page's own origin, so the lobby's link and QR come back
        // here. On bongbong.io this is the deployed site anyway; on a PR
        // preview it is the difference between a scan reaching this
        // build and reaching production's.
        if let Some(site) = invite.site.clone() {
            session.site = site;
        }
        // The page's token is this tab's; the name is read off it, so a
        // reload keeps both (`Identity::anonymous_with`).
        let token = page_string(PAGE_TOKEN);
        if !token.trim().is_empty() {
            let identity = crate::net::client::Identity::anonymous_with(token);
            session.nick = identity.nick;
            session.token = identity.device_token;
        }
        eprintln!(
            "[online] site {}, rooms {}, seat token {}, link code {}",
            session.site.base(),
            session.rooms.base(),
            session.token,
            invite.code.as_ref().map_or("-", |c| c.text.as_str())
        );
        if let Some(code) = invite.code {
            session.join_room(code);
        }
    }
    #[cfg(not(target_os = "emscripten"))]
    let _rig = match open_online(&args, &room_map, &Identity::new(session.nick.clone(), session.token.clone())) {
        // The lobby shows the code and the QR while the room fills up
        // and hands over to `Driver::Online` when the round begins; the
        // rig's room is already playing by the time it answers, so
        // `--rig` passes straight through.
        Some((round, rig)) => {
            session.open_lobby_with(round);
            rig
        }
        None => None,
    };

    // The dev server is serviced at the frame boundary below, like the
    // tuning transports; failing to bind is a warning, not a fatal error.
    #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
    let mut dev: Option<crate::devserver::DevServer> = if args.no_dev_server {
        None
    } else {
        let port = args
            .dev_port
            .or_else(|| std::env::var("BONGBONG_DEV_PORT").ok()?.parse().ok())
            .unwrap_or(crate::devserver::DEFAULT_PORT);
        match crate::devserver::DevServer::start(port) {
            Ok(server) => {
                eprintln!("[dev] listening on 127.0.0.1:{} (bbmcp / just mcp-call)", server.port());
                Some(server)
            }
            Err(e) => {
                eprintln!("[dev] could not bind 127.0.0.1:{port}: {e} - running without the dev server");
                None
            }
        }
    };

    // A held finger is reported as a touch-point *count*, not as a press, so
    // the press edge has to be found here - one tap must produce exactly one
    // builder stroke or button press.
    let mut touch_held_last_frame = false;
    // Which input the hints name (`hud::Hints`): the last one used - a
    // touch landing, a key pressed - opening on taps where there is no
    // keyboard or the mouse stands in for a finger.
    let mut hints = crate::hud::Hints::at_start(!crate::KEYBOARD_AVAILABLE || touch_from_mouse);

    // game_loop::run drives a plain `while !window_should_close()` loop on
    // native, and hands this closure to emscripten's main loop on web - same
    // source for both, and no -sASYNCIFY=1 needed to keep the browser tab
    // responsive (see .cargo/config.toml).
    //
    // **The fps argument means something different on each side.** On native
    // it is `SetTargetFPS`, a cap. On web it picks the main loop's *driver*:
    // `emscripten_set_main_loop_arg` takes any `fps > 0` as a request for
    // `EM_TIMING_SETTIMEOUT` at `1000/fps` ms, and only `0` selects
    // `EM_TIMING_RAF` (emscripten's `libeventloop.js`, `setMainLoop`). A
    // timer is not synced to the display refresh, so a finished frame waits
    // an arbitrary slice of a refresh interval before it is shown - which
    // the player feels as lag between a tap and the tank answering it - the
    // phase drifts, which reads as judder, and browsers throttle timers
    // harder than rAF, phones most of all. Rendering 120 ticks/s to a 60 Hz
    // display also throws half the work away.
    // iOS is a plain blocking loop (SDL pumps UIKit's run loop from inside
    // event polling). A phone's display paces the swap at exactly the
    // refresh rate (measured 16.67 ms on an iPhone 14 with no cap, against
    // 17.1 ms and a little jitter with raylib's 60 cap), so the device runs
    // uncapped; the simulator ignores the swap interval and would spin, so
    // it keeps the cap.
    let target_fps = if cfg!(target_os = "emscripten") {
        0
    } else if cfg!(target_os = "ios") {
        if cfg!(target_abi = "sim") { 60 } else { 0 }
    } else if cfg!(target_os = "android") {
        // A phone's EGL swap paces the loop at the display rate; the
        // emulator's does not and would spin.
        #[cfg(target_os = "android")]
        {
            if android::is_emulator() { 60 } else { 0 }
        }
        #[cfg(not(target_os = "android"))]
        {
            0
        }
    } else {
        120
    };
    // The round's clock and the input of a frame that ran no step (its
    // presses are owed to the next step - see `Input::or_presses`).
    let mut clock = StepClock::default();
    let mut carried = Input::default();
    // What `bb_net_stats` hands the page: the online round's readings.
    #[cfg(feature = "dev-tools")]
    let mut net_stats = crate::capi::NetStatsFeed::default();
    // The frames drawn and the last press the window saw, which
    // `bb_ui_json` reports.
    #[cfg(all(feature = "dev-tools", target_os = "emscripten"))]
    let (mut frames_drawn, mut last_press): (u64, Option<crate::capi::Press>) = (0, None);
    game_loop::run(rl, thread, target_fps, move |rl, thread| {
        #[cfg(any(target_os = "ios", target_os = "android", target_os = "emscripten"))]
        let _held = &batch;
        crate::frame_stages::frame_done(rl.get_frame_time() * 1000.0);
        // The web's window is the canvas's box: it follows a resize, a
        // rotation or full screen before anything reads its size.
        #[cfg(target_os = "emscripten")]
        web::follow_canvas(rl);
        #[cfg(all(feature = "dev-tools", target_os = "emscripten"))]
        {
            frames_drawn += 1;
        }
        // Frame boundary, first: dev-server requests (state reads and
        // writes, tuning patches, an armed step or screenshot), so anything
        // they stage lands in this same frame.
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        if let Some(dev) = &mut dev {
            let (width, height) = session.field_size();
            dev.before_frame(&mut session, width, height);
        }
        // The field is the live mode's map's (a `restart` above may have
        // just swapped it).
        let (width, height) = session.field_size();
        // How the frame lands on the window (`Presentation`), and the
        // render targets that takes. A view the dev server's `camera` tool
        // pinned outranks the map.
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        let pinned = dev.as_ref().and_then(|dev| dev.pinned_camera((width, height)));
        #[cfg(not(all(feature = "dev-tools", not(target_os = "emscripten"))))]
        let pinned: Option<Camera> = None;
        // Raw pointer state, shared by every mode. Touch is edge-detected
        // by hand because raylib reports a held finger as a point count,
        // not a press.
        let touching = rl.get_touch_point_count() > 0;
        let touch_pressed = touching && !touch_held_last_frame;
        touch_held_last_frame = touching;
        let mouse_pressed = rl.is_mouse_button_pressed(sola_raylib::prelude::MouseButton::MOUSE_BUTTON_LEFT);
        let mouse_held = rl.is_mouse_button_down(sola_raylib::prelude::MouseButton::MOUSE_BUTTON_LEFT);
        // The hints follow the input last used: a touch landing - or the
        // mouse's press standing in for one - turns them to taps, a key
        // turns them back; the dev server's `key`, `builder_touch` and
        // `click {touch}` stand in for the same.
        hints = hints.follow(touch_pressed || (touch_from_mouse && mouse_pressed), key_pressed(rl));
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        if let Some(used) = dev.as_mut().and_then(|dev| dev.take_hints()) {
            hints = used;
        }
        // The window the chrome lays itself out in (`hud::UiFrame`): its
        // size in points, its safe area, and whether thumbs are on the
        // glass - a build with no keyboard, `--touch-from-mouse`, a touch
        // seen this session or one landing now (`TouchScheme::touch_chrome`:
        // the frame a finger lands on is already the one it lifts from) -,
        // and the hints. The builder's canvas stands under its bar, so the
        // frame is laid out first.
        let touch_screen = touch.touch_chrome(crate::KEYBOARD_AVAILABLE, touch_from_mouse, touching);
        let ui = ui_frame(rl, touch_screen).with_hints(hints);
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        if let Some(dev) = &mut dev {
            dev.publish_ui(ui);
        }
        let mut plan = Presentation::of(rl, &session, pinned, establish.showing(), &ui);
        plan.fit_targets(rl, thread, &mut scene_target, &mut scene_size, &mut composite, &mut composite_size);
        let (mut layout, mut view) = (plan.layout, plan.view);
        if !crate::EMBEDDED && rl.is_key_pressed(KeyboardKey::KEY_F11) {
            rl.toggle_borderless_windowed();
        }
        // The minimap's slot in the right cluster, which the hit tests and
        // the painter read alike (`PlayChrome::minimap`): this screen, this
        // view.
        let this_screen = screen(rl);
        session.minimap_on = plan.minimap_on(&this_screen);
        // Then land any tuning edits staged since last frame (dev panel
        // via capi.rs, the `--tuning` file watch, or the dev server)
        // before the simulation reads the table, so a frame never sees two
        // values of one knob. The ripple shaders cache their knobs as
        // uniforms, so re-upload those only when something actually changed.
        if let Some(watch) = &mut tuning_watch {
            watch.poll();
        }
        if tuning::apply_pending() {
            let t = tuning::current();
            shock_fx.set_tuning(RippleTuning {
                speed: t.shockwave_speed,
                width: t.shockwave_width,
                strength: t.shockwave_strength * t.screen_fx_intensity,
                duration: t.shockwave_duration,
            });
            muzzle_fx.set_tuning(RippleTuning {
                speed: t.muzzle_flash_speed,
                width: t.muzzle_flash_width,
                strength: t.muzzle_flash_strength,
                duration: t.muzzle_flash_duration,
            });
            impact_fx.set_tuning(RippleTuning {
                speed: t.impact_flash_speed,
                width: t.impact_flash_width,
                strength: t.impact_flash_strength,
                duration: t.impact_flash_duration,
            });
        }
        // A pointer is read on the window, where the builder takes it (its
        // frame puts it on the canvas or on its chrome), and in UI points,
        // where the corners' buttons, the dialogs, the end screen, the
        // level select and the lobby stand.
        let window_pointer: crate::math::Vec2 = if touching { rl.get_touch_position(0).into() } else { rl.get_mouse_position().into() };
        let ui_pointer = ui.to_ui(window_pointer);
        // The corners as this frame finds them (`hud::corners`), the one
        // geometry the painter draws and these hit tests read. A touch that
        // lands on either cluster is the HUD's, never a stick or a shot.
        let corners = CornerShape::of(&session.play_chrome(), session.shown().players.count()).map(|shape| hud::corners(&ui, &shape));
        let corner_hit = corners.as_ref().and_then(|c| c.hit(ui_pointer));
        // The lamp row's button, pressed this frame (`CornerButton::Lamp`).
        let mut lamp_tap = false;
        // The pause button, pressed this frame (`CornerButton::Pause`).
        let mut pause_tap = false;
        let keep_out: Vec<crate::math::Rectangle> = corners.iter().flat_map(Corners::keep_out).collect();
        touch.set_keep_out(&keep_out);
        // This frame's touch points, ids included so a stick follows its
        // own finger: on the window in UI points for the touch scheme,
        // which lives there like the HUD, and on the window itself for the
        // builder's gestures. `--touch-from-mouse` stands a held left
        // button in for one.
        let mut window_touches: Vec<(i32, crate::math::Vec2)> =
            (0..rl.get_touch_point_count()).map(|i| (rl.get_touch_point_id(i), rl.get_touch_position(i).into())).collect();
        if touch_from_mouse && mouse_held && window_touches.is_empty() {
            window_touches.push((-1, rl.get_mouse_position().into()));
        }
        let touch_points: Vec<TouchPoint> = window_touches.iter().map(|&(id, at)| TouchPoint { id, pos: at }).collect();
        let ui_touch_points: Vec<TouchPoint> = window_touches.iter().map(|&(id, at)| TouchPoint { id, pos: ui.to_ui(at) }).collect();
        let steer_right = crate::TOUCH_STEER_RIGHT;
        let pressed = mouse_pressed || touch_pressed;
        let held = mouse_held || touching;
        #[cfg(all(feature = "dev-tools", target_os = "emscripten"))]
        if pressed {
            let count = last_press.map_or(1, |p| p.count + 1);
            last_press = Some(crate::capi::Press { count, at: window_pointer, touch: touch_pressed });
        }
        let tab = rl.is_key_pressed(KeyboardKey::KEY_TAB);
        // A modified map whose original this build changed asks first.
        session.watch_originals();
        let ctrl = rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL)
            || rl.is_key_down(KeyboardKey::KEY_LEFT_SUPER)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_SUPER);
        let dt = rl.get_frame_time();
        #[cfg(all(feature = "dev-tools", any(target_os = "ios", target_os = "android")))]
        phone_frame_stats.sample(dt, rl.get_time(), rl.get_fps(), fx.live());

        match session.mode() {
            Driver::Play => {
                // The two buttons and the two dialogs come first: a press
                // on any of them is never a tank order. Nothing here
                // touches the simulation - a frozen round is one whose
                // `update` is not called (see `Session::playing`).
                if session.question.is_some() {
                    // A question about a kept map takes every press, and
                    // its press is nobody's shot.
                    if pressed {
                        touch.claim(&ui_touch_points);
                        session.press_question(ui_pointer, ui.area);
                    } else if rl.is_key_pressed(KeyboardKey::KEY_ENTER) {
                        session.answer_question(true);
                    } else if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) || tab {
                        session.answer_question(false);
                    }
                } else if session.level_select.is_some() {
                    // The level select takes every press and key while it
                    // is up, and a press on it is nobody's shot - neither
                    // the round's it closes back onto nor the next level's.
                    if pressed {
                        touch.claim(&ui_touch_points);
                    }
                    let input = SelectInput {
                        pointer: Some(ui_pointer),
                        pressed,
                        left: rl.is_key_pressed(KeyboardKey::KEY_LEFT),
                        right: rl.is_key_pressed(KeyboardKey::KEY_RIGHT),
                        up: rl.is_key_pressed(KeyboardKey::KEY_UP),
                        down: rl.is_key_pressed(KeyboardKey::KEY_DOWN),
                        enter: rl.is_key_pressed(KeyboardKey::KEY_ENTER),
                        escape: rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) || tab,
                    };
                    session.update_level_select(&input, ui.area);
                } else if session.players_dialog {
                    let rects = players_dialog_rects(ui.area);
                    if pressed {
                        // The dialog's own press is nobody's shot: ONE or
                        // TWO starts a round under the finger.
                        touch.claim(&ui_touch_points);
                        if rects.one.contains(ui_pointer) {
                            session.answer_players(PlayerCount::ONE);
                        } else if rects.two.contains(ui_pointer) {
                            session.answer_players(PlayerCount::TWO);
                        } else if !rects.panel.contains(ui_pointer) {
                            session.close_players_dialog();
                        }
                    }
                    if rl.is_key_pressed(KeyboardKey::KEY_ONE) {
                        session.answer_players(PlayerCount::ONE);
                    } else if rl.is_key_pressed(KeyboardKey::KEY_TWO) {
                        session.answer_players(PlayerCount::TWO);
                    } else if rl.is_key_pressed(KeyboardKey::KEY_ENTER) {
                        // Enter is the action, as it is "leave" in the other
                        // dialog: the point of opening this one is to switch.
                        let other =
                            if session.game.players == PlayerCount::ONE { PlayerCount::TWO } else { PlayerCount::ONE };
                        session.answer_players(other);
                    } else if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) || tab {
                        session.close_players_dialog();
                    }
                } else if session.dialog {
                    let rects = leave_dialog_rects(ui.area);
                    if pressed {
                        // The dialog's own press is nobody's shot.
                        touch.claim(&ui_touch_points);
                        if rects.leave.contains(ui_pointer) {
                            session.answer_dialog(true);
                        } else if rects.stay.contains(ui_pointer) || !rects.panel.contains(ui_pointer) {
                            session.answer_dialog(false);
                        }
                    }
                    if rl.is_key_pressed(KeyboardKey::KEY_ENTER) {
                        session.answer_dialog(true);
                    } else if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) || tab {
                        session.answer_dialog(false);
                    }
                } else if pressed && session.press_result(ui_pointer, ui.area) {
                    // A level's end screen (docs/levels.md): PLAY AGAIN or
                    // the way on. The tap that pressed it must not also be
                    // the fire press that skips the next banner.
                    touch.claim(&ui_touch_points);
                } else if rl.is_key_pressed(KeyboardKey::KEY_ENTER) && session.enter_result() {
                    // Enter takes the way on after a win, PLAY AGAIN after
                    // a loss; R is the simulation's own restart.
                } else if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) && session.press_levels() {
                    // Esc opens the level select over the round, wherever
                    // it stands (docs/levels.md).
                } else if pressed && session.level_button().is_some() && corner_hit == Some(CornerButton::Level) {
                    // The level button, in the mission word's place.
                    session.press_levels();
                } else if tab || (pressed && corner_hit == Some(CornerButton::Build)) {
                    session.press_build();
                } else if crate::TWO_PLAYERS_AVAILABLE && pressed && corner_hit == Some(CornerButton::Players) {
                    session.press_players();
                } else if crate::ONLINE_AVAILABLE && pressed && corner_hit == Some(CornerButton::Online) {
                    // The ONLINE button opens the lobby over the local
                    // round, which is left exactly where it stands.
                    session.press_online();
                } else if pressed && corner_hit == Some(CornerButton::Lamp) {
                    // The lamp row's count, the lamp key's stand-in.
                    lamp_tap = true;
                    touch.claim(&ui_touch_points);
                } else if pressed && corner_hit == Some(CornerButton::Pause) {
                    // The pause button, the P key's stand-in: this frame's
                    // `Input::pause_pressed`.
                    pause_tap = true;
                    touch.claim(&ui_touch_points);
                } else if !crate::KEYBOARD_AVAILABLE && pressed && corner_hit == Some(CornerButton::Restart) {
                    // The RESTART button stands in for the R key: staged the
                    // way the dev panel's button is, it becomes this frame's
                    // `Input::restart_pressed`.
                    tuning::request_restart();
                }
            }
            Driver::Lobby => {
                // The lobby's own screen, in UI points like its rects.
                // Its touches are its own, and a finger still down when
                // START, REMATCH or LEAVE hands the window to a round is
                // nobody's stick or shot there.
                touch.claim(&ui_touch_points);
                let mut typed = String::new();
                while let Some(c) = rl.get_char_pressed() {
                    typed.push(c);
                }
                let input = LobbyInput {
                    pointer: Some(ui_pointer),
                    pressed,
                    typed,
                    backspace: rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE),
                    enter: rl.is_key_pressed(KeyboardKey::KEY_ENTER),
                    escape: rl.is_key_pressed(KeyboardKey::KEY_ESCAPE),
                };
                session.update_lobby(&input, ui.area, dt);
            }
            Driver::Online => {
                // A round belongs to its room: the corners carry one
                // button, `LEAVE`, and Esc does the same thing - the
                // seat goes back and the window comes out at the local
                // round exactly where it stood. A press on the corners is
                // never a play input, so the hit test is safe here.
                let left = pressed && corner_hit == Some(CornerButton::Leave);
                if left || rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
                    session.leave_online();
                } else if pressed && corner_hit == Some(CornerButton::Lamp) {
                    lamp_tap = true;
                    touch.claim(&ui_touch_points);
                }
            }
            Driver::Build => {
                // Every touch is the builder's, and one still down when
                // PLAY or PLAY HERE starts a round under it is nobody's
                // stick or shot there - it would skip the round's banner.
                touch.claim(&ui_touch_points);
                let mut typed = String::new();
                while let Some(c) = rl.get_char_pressed() {
                    typed.push(c);
                }
                // The arrows pan the canvas while held, each the way it
                // points.
                let axis = |less: KeyboardKey, more: KeyboardKey| (rl.is_key_down(more) as i32 - rl.is_key_down(less) as i32) as f32;
                let input = BuilderInput {
                    pointer: Some(window_pointer),
                    pressed,
                    held,
                    right_pressed: rl.is_mouse_button_pressed(sola_raylib::prelude::MouseButton::MOUSE_BUTTON_RIGHT),
                    right_held: rl.is_mouse_button_down(sola_raylib::prelude::MouseButton::MOUSE_BUTTON_RIGHT),
                    middle_held: rl.is_mouse_button_down(sola_raylib::prelude::MouseButton::MOUSE_BUTTON_MIDDLE),
                    space_held: rl.is_key_down(KeyboardKey::KEY_SPACE),
                    wheel: rl.get_mouse_wheel_move(),
                    zoom_in: rl.is_key_pressed(KeyboardKey::KEY_EQUAL) || rl.is_key_pressed(KeyboardKey::KEY_KP_ADD),
                    zoom_out: rl.is_key_pressed(KeyboardKey::KEY_MINUS) || rl.is_key_pressed(KeyboardKey::KEY_KP_SUBTRACT),
                    pan_keys: crate::math::Vec2::new(
                        axis(KeyboardKey::KEY_LEFT, KeyboardKey::KEY_RIGHT),
                        axis(KeyboardKey::KEY_UP, KeyboardKey::KEY_DOWN),
                    ),
                    escape: rl.is_key_pressed(KeyboardKey::KEY_ESCAPE),
                    enter: rl.is_key_pressed(KeyboardKey::KEY_ENTER),
                    backspace: rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE),
                    undo: ctrl && rl.is_key_pressed(KeyboardKey::KEY_Z),
                    redo: ctrl && rl.is_key_pressed(KeyboardKey::KEY_Y),
                    copy: ctrl && rl.is_key_pressed(KeyboardKey::KEY_C),
                    cut: ctrl && rl.is_key_pressed(KeyboardKey::KEY_X),
                    paste: ctrl && rl.is_key_pressed(KeyboardKey::KEY_V),
                    delete: rl.is_key_pressed(KeyboardKey::KEY_DELETE),
                    typed,
                    // The fingers themselves, ids and all - the builder's
                    // gestures read every one (`editor::gesture`), and
                    // `--touch-from-mouse` stands a held left button in
                    // for one.
                    touches: touch_points.clone(),
                    dt,
                    // The screen the canvas is measured on: its zoom steps
                    // in device pixels, its touch sizes in points, a cell
                    // under a finger in millimetres.
                    screen: Some(CanvasScreen {
                        device_per_px: view.scale * framebuffer_ratio(rl),
                        points_per_px: view.scale / window_units_per_point(rl),
                        coarse: screen(rl).ppi < tuning().view_fine_ppi,
                        points_per_mm: touch_points_per_mm(),
                    }),
                };
                // Fingers a dev server's `builder_touch {hold: true}` left
                // down stay down across frames while the screen has none.
                #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
                let input = match dev.as_ref().map(|dev| dev.held_touches()).filter(|held| !held.is_empty() && input.touches.is_empty()) {
                    Some(held) => BuilderInput { pointer: held.first().map(|t| t.pos), pressed: false, held: true, touches: held.to_vec(), ..input },
                    None => input,
                };
                if tab && session.question.is_some() {
                    session.answer_question(false);
                } else if tab {
                    session.toggle();
                } else {
                    session.update_builder(&input, &plan.builder_frame(&ui));
                }
            }
        }

        // A press this frame may have changed what the window shows -
        // BUILD, PLAY, a level started on another map: the frame presents
        // that rather than what it began with (its presses were hit-tested
        // on what was on screen).
        if plan.stale(&session) {
            #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
            let pinned = dev.as_ref().and_then(|dev| dev.pinned_camera(session.field_size()));
            #[cfg(not(all(feature = "dev-tools", not(target_os = "emscripten"))))]
            let pinned: Option<Camera> = None;
            plan = Presentation::of(rl, &session, pinned, establish.showing(), &ui);
            plan.fit_targets(rl, thread, &mut scene_target, &mut scene_size, &mut composite, &mut composite_size);
            (layout, view) = (plan.layout, plan.view);
            session.minimap_on = plan.minimap_on(&this_screen);
        }

        // The Load list's thumbnails, every frame, so the frame the list
        // closes - or the builder is left - lets their textures go.
        builder_thumbnails.sync(rl, thread, &session.builder);

        if session.mode() == Driver::Build {
            // Nothing is steering while the builder is up - every touch is
            // its own, so the scheme's area is empty -, but the scheme
            // still sees the frame so a finger lifted here is not a stick
            // still held when play resumes - on a fresh clock, owing no
            // step and no press.
            touch.update(&ui_touch_points, crate::Rect::new(0.0, 0.0, 0.0, 0.0), steer_right, dt);
            clock.reset();
            carried = Input::default();
            // The builder shows its canvas through its own camera.
            #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
            if let Some(dev) = &mut dev {
                let camera = session.builder.view_camera(&layout);
                dev.publish_camera(crate::follow::CameraReport { mode: crate::follow::CameraMode::Build, camera, layout, view, follow: None });
            }
            let shade = builder_shade.sync(rl, thread, session.builder.ground().shade());
            let minimap = builder_minimap_texture.sync(rl, thread, session.builder.minimap().image());
            session.builder.render(
                rl,
                thread,
                &mut composite,
                &mut builder_scene,
                &plan.builder_frame(&ui),
                BAR_FILL,
                &EditorTextures {
                    obstacles: &obstacles_texture,
                    props: &props_texture,
                    ground: &ground_textures[theme_index(session.builder.map().theme)],
                    grass: &grass_textures[theme_index(session.builder.map().theme)],
                    trees: &trees_texture,
                    target: &target_texture,
                    towers: &towers_texture,
                    crates: &crates_texture,
                    pickup_glyphs: &pickup_glyphs_texture,
                    // Palette icon: the first colour variant's idle frame -
                    // a fixed representative sprite, since the builder
                    // places a frog *cell*, not a rolled colour.
                    frog_idle: &frog_textures[0].idle,
                    eraser: &eraser_texture,
                    portal: &portal_texture,
                    tanks: &tanks_texture,
                    shade,
                    minimap,
                    thumbnails: Some(&builder_thumbnails),
                },
                session.question.as_ref(),
            );
            // The presented frame is the builder; a pending `screenshot`
            // reads it from the screen here, or the client waits forever.
            #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
            if let Some(dev) = &mut dev {
                dev.after_render(rl, thread, &scene_target, &session.game);
            }
            #[cfg(all(feature = "dev-tools", target_os = "emscripten"))]
            publish_window(rl, &session, &ui, &layout, &view, frames_drawn, last_press);
            return;
        }

        // Gather this frame's raw input into a plain `Input` - `Game::update`
        // itself decides what to do with it (e.g. whether a wreck can move),
        // so nothing simulation-related needs to know a `RaylibHandle`
        // exists. See simulation.rs's module doc comment.
        let (mut player1, player2) = gather_intents(rl, session.game.players);
        // A touch screen drives player 1 through the same intent the
        // keyboard does; a held key still wins the direction, a tap or a
        // key both fire. Fed every frame, dialog or not, so a lifted
        // finger is never a stick still held. The scheme takes touches
        // from the whole window, in UI points: an arena's margins are
        // where a tablet's thumbs rest.
        let touch_intent = touch.update(&ui_touch_points, ui.screen, steer_right, dt);
        player1.move_dir = player1.move_dir.or(touch_intent.move_dir);
        player1.fire = player1.fire || touch_intent.fire;
        player1.lamp |= lamp_tap;
        let mut input = Input::two(player1, player2);
        input.pause_pressed = rl.is_key_pressed(KeyboardKey::KEY_P) || pause_tap;
        // The dev panel's "Restart round" button lands here too, as if R
        // had been pressed - the simulation never learns a browser exists.
        input.restart_pressed = rl.is_key_pressed(KeyboardKey::KEY_R) || tuning::take_restart_request();
        input.toggle_shadows_pressed = rl.is_key_pressed(KeyboardKey::KEY_L);
        // The I key is inert in a release build: overlays are dev-only.
        input.cycle_overlays_pressed = cfg!(feature = "dev-tools") && rl.is_key_pressed(KeyboardKey::KEY_I);
        // A press from a frame that ran no step is still owed to the
        // next one.
        let input = input.or_presses(carried);
        // Injected input (the dev server's `input` tool) replaces the
        // keyboard's intent for as many frames as it asked.
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        let input = match &mut dev {
            Some(dev) => dev.shape_input(input),
            None => input,
        };

        // Online: this seat's own intent goes to the room and the
        // replica is written from what came back. Seat 0 of the frame's
        // input is the local player, whichever seat the room gave them.
        // Nothing here runs `Game::update`, and the local round is left
        // exactly where it stood.
        if session.mode() == Driver::Online {
            #[allow(unused_mut)]
            let mut intent = input.seat(0);
            // A page's measurement script drives the seat (`capi::bb_input`).
            #[cfg(feature = "dev-tools")]
            if let Some(scripted) = crate::capi::take_scripted_input() {
                intent = scripted;
            }
            session.update_online(&intent, dt);
        }
        // The page reads a round's numbers only while one is played: the
        // frame the window is in any other mode - the round over and the
        // lobby back, the seat given up - they are taken down.
        #[cfg(feature = "dev-tools")]
        net_stats.frame(match (session.mode(), session.online.as_ref()) {
            (Driver::Online, Some(round)) => Some(round.stats_json().to_string()),
            _ => None,
        });

        // The round advances in whole steps of `PHYSICS_FIXED_DT`, as many
        // as this frame's real time pays for (`StepClock`), every step on
        // this frame's input with the one-shot presses spent by the first.
        // With the dev server attached it runs those steps, or a lockstep
        // `step`'s own frames, or nothing while frozen - and a frozen or
        // dialog frame leaves the clock at zero, so the round resumes on a
        // fresh step rather than catching up on the time it stood still.
        // How far the particle layer ages this frame: the frame's real
        // time, except in a dev-server lockstep, where it is the simulated
        // time the frame's `step` ran - so a frozen round's sparks and hits
        // freeze with it and a recording stepped a frame at a time plays
        // them at their real pace.
        // The blow that decided the round is held, then slowed
        // (`Session::time_scale`): the round's clock and the particle
        // layer take that share of the frame's real time.
        let scaled_dt = dt * session.time_scale();
        #[cfg_attr(not(all(feature = "dev-tools", not(target_os = "emscripten"))), allow(unused_mut))]
        let mut fx_dt = scaled_dt;
        // How far the local round got this frame, which is how far a
        // followed view moves: with the round, step for step.
        let frame_before = session.game.frame();
        if session.playing() {
            #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
            let frozen = dev.as_ref().is_some_and(|dev| dev.lockstep());
            #[cfg(not(all(feature = "dev-tools", not(target_os = "emscripten"))))]
            let frozen = false;
            let steps = if frozen {
                clock.reset();
                0
            } else {
                clock.advance(scaled_dt)
            };
            let steps_stage = crate::frame_stages::stage("sim");
            let seats = local_seats(&session);
            #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
            let advanced = match &mut dev {
                Some(dev) => {
                    let before = session.game.frame();
                    dev.advance(&mut session.game, input, steps, width, height, &mut |game| {
                        fx.observe_events(game);
                        follow.observe_events(game);
                        awareness.observe_events(game, &seats);
                        voice.observe(game);
                    });
                    if frozen {
                        fx_dt = session.game.frame().saturating_sub(before) as f32 * PHYSICS_FIXED_DT;
                    }
                    true
                }
                None => false,
            };
            #[cfg(not(all(feature = "dev-tools", not(target_os = "emscripten"))))]
            let advanced = false;
            if !advanced {
                for i in 0..steps {
                    let step_input = if i == 0 { input } else { input.held_only() };
                    session.game.update(step_input, PHYSICS_FIXED_DT, width, height);
                    // The particle layer, the follow camera and the
                    // indicators read each step's events before the next
                    // step clears them.
                    fx.observe_events(&session.game);
                    follow.observe_events(&session.game);
                    awareness.observe_events(&session.game, &seats);
                    voice.observe(&session.game);
                }
            }
            drop(steps_stage);
            carried = if steps == 0 && !frozen { input } else { Input::default() };
        } else {
            clock.reset();
            carried = Input::default();
        }
        // A won level is progress, kept the frame it is won - a player who
        // closes the window on the end screen has still reached the next.
        session.note_outcome();
        if let Some(level) = session.take_progress() {
            save_progress(&level);
        }
        // A level's end screen that has counted down takes its way: the
        // next level after a win, the same one again after a loss. Then
        // the fade through black between rounds moves on a frame.
        session.follow_countdown();
        session.tick_curtain(dt);
        // A level's end screen that counted down may have opened another
        // map: present that one.
        if plan.stale(&session) {
            #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
            let pinned = dev.as_ref().and_then(|dev| dev.pinned_camera(session.field_size()));
            #[cfg(not(all(feature = "dev-tools", not(target_os = "emscripten"))))]
            let pinned: Option<Camera> = None;
            plan = Presentation::of(rl, &session, pinned, establish.showing(), &ui);
            plan.fit_targets(rl, thread, &mut scene_target, &mut scene_size, &mut composite, &mut composite_size);
            (layout, view) = (plan.layout, plan.view);
            session.minimap_on = plan.minimap_on(&this_screen);
        }
        let field = plan.field;
        // The round on screen: the room's replica in an online round,
        // the session's own otherwise.
        let game = session.shown();
        // The part of the world this frame shows: a pinned view, the whole
        // field, or a field map's followed view, moved on by the time the
        // round on screen advanced - a local round's steps, a replica's
        // frame of real time, nothing while a dialog or the lobby holds
        // the round still.
        let mut establishing = false;
        let mut shown_rect: Option<crate::math::Rectangle> = None;
        // A couch's split screen: the second half's camera and the split,
        // or while the establishing shot zooms into it, the second half's
        // zoom (`establish::zoom_split`) and how far apart the follow views
        // it lands on stand; and the world the second half shows, for the
        // minimap.
        let mut split_view: Option<(Camera, crate::follow::Split)> = None;
        let mut zoom_split: Option<(View, crate::establish::ZoomSplit, f32)> = None;
        let mut second_shown: Option<crate::math::Rectangle> = None;
        let camera = match plan.followed {
            Some(frame) => {
                // The round on screen: a room's replica or the local round
                // (which the lobby and the dialogs stand over, still), on
                // which map.
                let scene = (session.mode() == Driver::Online, game.map.name.clone(), field);
                if followed_in.as_ref() != Some(&scene) {
                    follow.cut();
                    followed_in = Some(scene);
                }
                follow.observe_events(game);
                let advanced = match session.mode() {
                    Driver::Online => dt,
                    _ => session.game.frame().checked_sub(frame_before).unwrap_or(0) as f32 * PHYSICS_FIXED_DT,
                };
                // The seats this screen plays (`local_seats`): until a
                // room's `Welcome` the window draws the local round, where
                // the room's seat is nobody's, and the view holds.
                let local: Vec<usize> = local_seats(&session).into_iter().map(usize::from).collect();
                let stage = crate::follow::Stage { visible: frame.framing.visible, field, sight: frame.sight };
                let shot = follow.update(&crate::follow::Seats::of(game, &local), advanced, &stage, &crate::follow::FollowRules::current());
                let device_scale = frame.device_scale(framebuffer_ratio(rl));
                let follow_camera = Camera::following(field, shot.corner, frame.framing.visible, 1.0, device_scale);
                // A couch pair apart: the second half's own camera.
                let second = shot.split.map(|split| (Camera::following(field, split.corner, frame.framing.visible, 1.0, device_scale), split));
                // The establishing shot (establish.rs): a local round's
                // opening on a field map its view shows part of shows the
                // whole map, then zooms down to the follow view - drawn
                // through whole-field targets, as an arena is, and put on
                // the window by the zoom's mapping - while the follow camera
                // keeps going underneath, so the zoom lands where it stands.
                let rules = crate::establish::EstablishRules::current();
                let wanted = crate::establish::wanted(session.mode() == Driver::Play, plan.shows_part(), field, &rules);
                let phase = establish.update(game.round_seed(), game.frame(), game.intro_timer, wanted, &rules, crate::motion::reduced());
                establishing = phase.showing();
                if establishing != plan.establishing {
                    plan = Presentation::of(rl, &session, plan.pinned, establishing, &ui);
                    plan.fit_targets(rl, thread, &mut scene_target, &mut scene_size, &mut composite, &mut composite_size);
                    (layout, view) = (plan.layout, plan.view);
                }
                let camera = if establishing {
                    let window = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);
                    let whole = Camera::whole(field);
                    let from = crate::establish::Mapping::of(&whole, crate::math::Vec2::zero(), &View::fit(field, window));
                    let to = crate::establish::Mapping::of(&follow_camera, frame.layout.field_origin(), &frame.view);
                    let mapping = match phase {
                        crate::establish::Phase::Zoom(q) => crate::establish::between(from, to, q),
                        _ => from,
                    };
                    // A couch pair the round opens apart: the second half
                    // zooms from the same whole map into its own follow
                    // view, so the zoom lands on the split screen.
                    if let (Some((second, split)), crate::establish::Phase::Zoom(q)) = (second, phase) {
                        let to = crate::establish::Mapping::of(&second, frame.layout.field_origin(), &frame.view);
                        let zoom = crate::establish::between(from, to, q);
                        if let (Some(a), Some(b)) = (shot.keeps[0], split.keeps[0]) {
                            zoom_split = Some((zoom.view(field, window), crate::establish::zoom_split(mapping, zoom, (a, b), q), split.apart));
                            second_shown = Some(zoom.shows(window));
                        }
                    }
                    layout = Layout::bare(field.0, field.1);
                    view = mapping.view(field, window);
                    shown_rect = Some(mapping.shows(window));
                    whole
                } else {
                    split_view = second;
                    second_shown = second.map(|(camera, _)| camera.rect());
                    follow_camera
                };
                #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
                if let Some(dev) = &mut dev {
                    let mode = if establishing { crate::follow::CameraMode::Establishing } else { crate::follow::CameraMode::Follow };
                    dev.publish_camera(crate::follow::CameraReport {
                        mode,
                        camera,
                        layout,
                        view,
                        follow: Some(crate::follow::FollowReport {
                            framing: frame.framing,
                            seating: frame.seating,
                            sight: frame.sight,
                            shot,
                            establishing: phase,
                            second: second.map(|(camera, _)| camera),
                        }),
                    });
                }
                camera
            }
            None => {
                followed_in = None;
                let camera = plan.pinned.unwrap_or(Camera::whole(field));
                #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
                if let Some(dev) = &mut dev {
                    let mode = if plan.pinned.is_some() { crate::follow::CameraMode::Pinned } else { crate::follow::CameraMode::Whole };
                    dev.publish_camera(crate::follow::CameraReport { mode, camera, layout, view, follow: None });
                }
                camera
            }
        };
        // Between the steps and the draw: the particle layer samples the
        // world the round is at (its events it read after each step),
        // then ages what is already in flight. Deliberately not inside
        // `Game` - see fx.rs.
        let fx_stage = crate::frame_stages::stage("fx");
        fx.observe(game, fx_dt);
        fx.tick(fx_dt);
        drop(fx_stage);
        // What the screen cannot see: a replica's events read once a frame
        // (a local round's were read after every step), and the arrows
        // drawn only while the camera shows less than the whole field, so
        // an arena draws none. A touch screen keeps them out from under the
        // thumbs.
        let seats = local_seats(&session);
        if session.mode() == Driver::Online {
            awareness.observe_events(game, &seats);
        }
        // The chrome the frame draws, and its corners laid out on the
        // window as this frame left it - a press may have opened a dialog
        // or started another round since the hit tests ran.
        let chrome = session.play_chrome();
        let corners = CornerShape::of(&chrome, game.players.count()).map(|shape| hud::corners(&ui, &shape));
        // A cluster fades while a tank, a shot or a blast is under it, on
        // the frame's own time - a lockstep `step`'s, as the particles'.
        if let Some(corners) = &corners {
            let on_screen = hud::WorldOnScreen { camera, field_origin: layout.field_origin(), view, ui_scale: ui.scale };
            let t = tuning();
            let marks = hud::action_marks(game);
            let covered = match &split_view {
                // A split screen: each half counts what it shows on its side
                // of the divider.
                Some((second, split)) => {
                    let (first, other): (Vec<_>, Vec<_>) = marks.iter().copied().partition(|&(at, _)| !split.in_second(camera.to_view(at)));
                    let other: Vec<_> = other.into_iter().filter(|&(at, _)| split.in_second(second.to_view(at))).collect();
                    let a = hud::covered(corners, &first, &on_screen);
                    let b = hud::covered(corners, &other, &hud::WorldOnScreen { camera: *second, ..on_screen });
                    [a[0] || b[0], a[1] || b[1]]
                }
                None => hud::covered(corners, &marks, &on_screen),
            };
            fade.step(covered, fx_dt, t.ui_fade_opacity, t.ui_fade_seconds);
        }
        // A split screen's halves draw the indicators' marks in their own
        // worlds, first half first.
        let mut split_marks: [Vec<crate::indicators::Fill>; 2] = [Vec::new(), Vec::new()];
        let indicators = (matches!(session.mode(), Driver::Play | Driver::Online) && !camera.shows_whole_field()).then(|| {
            // The bitmap's pixels to a UI point, so an arrow is its rows'
            // size on the glass however the bitmap is scaled, at the scale
            // the rest of the chrome is drawn at.
            let points = ui.scale / view.scale;
            let t = crate::indicators::in_points(&tuning(), points);
            let field = crate::math::Rectangle::new(layout.field.x, layout.field.y, layout.field.w, layout.field.h);
            let mut frame = crate::indicators::ViewFrame::of_camera(&camera, field, &t);
            if touch_screen {
                // The thumbs are where they are on the glass, whatever the
                // UI scale: their pads are measured in the window's own
                // points.
                let corner = view.to_bitmap(crate::math::Vec2::zero());
                let far = view.to_bitmap(crate::math::Vec2::new(view.window.0, view.window.1));
                let screen = crate::math::Rectangle::new(corner.x, corner.y, far.x - corner.x, far.y - corner.y);
                let physical = window_units_per_point(rl) / view.scale;
                frame.keep_out.extend(crate::indicators::thumb_rests(screen, physical, &t));
            }
            // No arrow lands under a corner cluster or the minimap, nor
            // under the page's own controls.
            frame.keep_out.extend(corners.iter().flat_map(Corners::keep_out).map(|r| ui_rect_on_bitmap(&ui, &view, r)));
            #[cfg(target_os = "emscripten")]
            if let Some(r) = web::overlay().rect_in(window_units_per_point(rl)) {
                let ui_rect = crate::math::Rectangle::new(r.x / ui.scale, r.y / ui.scale, r.width / ui.scale, r.height / ui.scale);
                frame.keep_out.push(ui_rect_on_bitmap(&ui, &view, ui_rect));
            }
            match &split_view {
                // A couch's split screen: each half its own seat's arrows,
                // the other half off screen for it; the marks that lie in
                // the world go into each half's own picture of it.
                Some((second, split)) => {
                    // The divider in the bitmap's pixels, where the frames
                    // lay their arrows out.
                    let line = split.at + crate::math::Vec2::new(field.x, field.y);
                    let first = frame.clone().split_at(line, split.normal, &t);
                    let mut other = crate::indicators::ViewFrame::of_camera(second, field, &t).split_at(line, split.normal * -1.0, &t);
                    other.keep_out = first.keep_out.clone();
                    let mut halves = awareness.pictures(game, &seats, &[first, other], points).into_iter();
                    let (a, b) = (halves.next().unwrap_or_default(), halves.next().unwrap_or_default());
                    split_marks = [a.world, b.world];
                    crate::indicators::Picture {
                        world: Vec::new(),
                        screen: a.screen.into_iter().chain(b.screen).collect(),
                        labels: a.labels.into_iter().chain(b.labels).collect(),
                    }
                }
                None => awareness.picture(game, &seats, &frame, points),
            }
        });
        // The frog's line, if it has one up: aged on the time the round
        // ran (none while a dialog or the level select freezes it), its
        // keys or taps the input last used, laid out over the frog.
        let voice_dt = if session.playing() && session.mode() == Driver::Play { fx_dt } else { 0.0 };
        let catalogue = crate::text::text();
        let words = |key: &str| catalogue.message(key, &[]).unwrap_or_else(|| key.to_string());
        voice.update(game, voice_dt, &tuning(), |key| words(key).chars().count());
        let bubble = (session.mode() == Driver::Play)
            .then(|| voice.showing())
            .flatten()
            .zip(game.frog_position())
            .map(|(line, frog)| {
                let touch = ui.hints == hud::Hints::Touch;
                let key = crate::bubble::key_for(&line.key, touch, |k| catalogue.message(k, &[]).is_some());
                let text = words(&key);
                let shown = crate::bubble::revealed(&text, line.age, &tuning());
                let field = crate::math::Rectangle::new(layout.field.x, layout.field.y, layout.field.w, layout.field.h);
                let at = camera.to_view(frog) + crate::math::Vec2::new(field.x, field.y);
                let points = ui.scale / view.scale;
                let keep_out: Vec<crate::math::Rectangle> = corners.iter().flat_map(Corners::keep_out).map(|r| ui_rect_on_bitmap(&ui, &view, r)).collect();
                crate::bubble::layout(&text, shown, at, field, &keep_out, points, |w, size| crate::text::width(w, size.round() as i32) as f32)
            });
        // The minimap, where the corners hold its slot: the round's
        // picture, synced to the round on screen and uploaded where it
        // changed, and this frame's marks - its enemies the ones the
        // arrows could point at.
        let minimap_marks = corners.as_ref().and_then(|c| c.minimap).map(|_| {
            let shown: &[crate::indicators::Indicators] = if indicators.is_some() { awareness.shown() } else { &[] };
            let marks = crate::minimap::Marks::gather(game, &seats, shown, shown_rect.unwrap_or(camera.rect()));
            crate::minimap::Marks { second_view: second_shown, ..marks }
        });
        let minimap_image = minimap_marks.is_some().then(|| round_minimap.sync(game));
        let minimap_texture = minimap_image.and_then(|m| round_minimap_texture.sync(rl, thread, m.image()));
        let minimap = match (minimap_texture, minimap_marks.as_ref(), minimap_image) {
            (Some((_, texture)), Some(marks), Some(image)) => Some(crate::render::minimap::MinimapLayer { texture, field: image.field(), marks }),
            _ => None,
        };
        // The second half's targets, the first half's size.
        if split_view.is_some() && split_targets.as_ref().is_none_or(|(_, _, size)| *size != plan.scene) {
            let mut make = || rl.load_render_texture(thread, plan.scene.0 as u32, plan.scene.1 as u32).expect("failed creating a split half's render texture");
            split_targets = Some((make(), make(), plan.scene));
        }
        // The pictures the round keeps (the lava, its pools, the craters)
        // brought up to this frame for what the window shows, then every
        // baked image uploaded where it changed - before the draw, which
        // only reads them.
        let views: Vec<crate::math::Rectangle> = match (camera.cull(), split_view.as_ref().map(|(second, _)| second.cull())) {
            (None, _) | (_, Some(None)) => Vec::new(),
            (Some(first), second) => std::iter::once(first).chain(second.flatten()).collect(),
        };
        let pictures_stage = crate::frame_stages::stage("pictures");
        game.refresh_pictures(&views);
        let blocks = game.with_block_images(|images| round_blocks.sync(rl, thread, images));
        drop(pictures_stage);
        game.render(
            rl,
            thread,
            &mut scene_target,
            &mut composite,
            &view,
            &camera,
            BAR_FILL,
            &mut Effects {
                shock: &mut shock_fx,
                muzzle: &mut muzzle_fx,
                impact: &mut impact_fx,
                shots: shot_shaders.as_mut(),
                weather: Some(&mut weather_fx),
                fx: &fx,
                // No stick over the lobby: the field behind it is frozen
                // and every press there belongs to the screen.
                touch: (session.mode() != Driver::Lobby).then_some((&touch, steer_right)),
                indicators: indicators.as_ref(),
                bubble: bubble.as_ref(),
                minimap,
                // An establishing shot is drawn whole as an arena is, but
                // zooms: no margins, which stand still round an arena.
                margins: (!establishing).then_some(&mut margin_fx),
                split: match (split_view, split_targets.as_mut(), zoom_split) {
                    (Some((camera, split)), Some((scene, composite, _)), _) => Some(crate::render::game::SplitLayer::Follow {
                        camera,
                        scene,
                        composite,
                        at: split.at,
                        normal: split.normal,
                        apart: split.apart,
                        marks: [&split_marks[0], &split_marks[1]],
                    }),
                    (_, _, Some((view, zoom, apart))) => Some(crate::render::game::SplitLayer::Zoom {
                        view,
                        at: zoom.at,
                        normal: zoom.normal,
                        alpha: zoom.alpha,
                        apart,
                    }),
                    _ => None,
                },
            },
            &Textures {
                tanks: &tanks_texture,
                tank_glow: &tank_glow_texture,
                tank_modules: &tank_modules_texture,
                tank_modules_glow: &tank_modules_glow_texture,
                shells: &shells_texture,
                plasma: &plasma_texture,
                minigun_bullets: &minigun_bullets_texture,
                missile: &missile_texture,
                tracks: &tracks_texture,
                obstacles: &obstacles_texture,
                props: &props_texture,
                barrel_explosion: &barrel_explosion_texture,
                ground: &ground_textures[theme_index(game.map.theme)],
                frog_variants: &frog_textures,
                grass: &grass_textures[theme_index(game.map.theme)],
                trees: &trees_texture,
                target: &target_texture,
                towers: &towers_texture,
                crates: &crates_texture,
                pickup_glyphs: &pickup_glyphs_texture,
                portal: &portal_texture,
                blocks,
            },
            &layout,
            &chrome,
            &ui,
            fade,
        );
        // A pending screenshot reads the frame just presented.
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        if let Some(dev) = &mut dev {
            dev.after_render(rl, thread, &scene_target, session.shown());
        }
        #[cfg(all(feature = "dev-tools", target_os = "emscripten"))]
        publish_window(rl, &session, &ui, &layout, &view, frames_drawn, last_press);
        // What the local round showed, for BUILD to open the builder on.
        if session.mode() == Driver::Play {
            session.play_view = Some(camera.rect());
        }
    });
}

#[cfg(target_os = "ios")]
pub mod ios;
#[cfg(target_os = "android")]
pub mod android;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "emscripten")]
mod web;

/// Frame-time sampling for the console on a phone, dev-tools builds only:
/// no keyboard for the overlay cycle there, and the dev server binds the
/// phone's own loopback, so a log line every few seconds is the one way to
/// read whether the frame budget holds on real hardware.
#[cfg(feature = "dev-tools")]
#[derive(Default)]
pub struct FrameStats {
    sum: f32,
    max: f32,
    frames: u32,
    last_report: f64,
}

#[cfg(feature = "dev-tools")]
impl FrameStats {
    const REPORT_EVERY_SECONDS: f64 = 5.0;

    pub fn sample(&mut self, dt: f32, now: f64, fps: u32, live_fx: usize) {
        self.sum += dt;
        self.max = self.max.max(dt);
        self.frames += 1;
        if now - self.last_report >= Self::REPORT_EVERY_SECONDS && self.frames > 0 {
            eprintln!(
                "bongbong: frame avg {:.2} ms, max {:.2} ms, {} fps, {} fx over {} frames; {}",
                self.sum / self.frames as f32 * 1000.0,
                self.max * 1000.0,
                fps,
                live_fx,
                self.frames,
                crate::frame_stages::line()
            );
            *self = Self { last_report: now, ..Self::default() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 120 Hz display pays for a step every other frame, a 60 Hz one
    /// every frame, and the round never falls behind either: over a second
    /// both run sixty steps.
    #[test]
    fn the_step_clock_pays_real_time_out_in_whole_steps() {
        let mut clock = StepClock::default();
        let per_frame: Vec<u32> = (0..8).map(|_| clock.advance(1.0 / 120.0)).collect();
        assert_eq!(per_frame, vec![0, 1, 0, 1, 0, 1, 0, 1]);
        let mut clock = StepClock::default();
        assert!((0..600).all(|_| clock.advance(1.0 / 60.0) == 1), "one step per 60 Hz frame, no drift");
        let mut clock = StepClock::default();
        assert_eq!((0..120).map(|_| clock.advance(1.0 / 120.0)).sum::<u32>(), 60);
        let mut clock = StepClock::default();
        assert!((0..30).all(|_| clock.advance(1.0 / 30.0) == 2), "two steps per 30 Hz frame");
    }

    /// `--host -m` with a map nobody has won as it stands never dials: the
    /// round opens closed, its note the clear check's refusal, which is
    /// what the lobby's closed face shows.
    #[cfg(feature = "online")]
    #[test]
    fn hosting_a_map_nobody_has_cleared_is_refused_before_dialling() {
        let dir = std::env::temp_dir().join(format!("bongbong-host-{}", std::process::id()));
        let path = dir.join("mine.toml");
        let mut map = crate::map::MapFile::new();
        map.set_cell(3, 8, crate::map::CellObject::Start);
        map.save(&path).expect("the map is written");
        let args = Args::parse_from(["bongbong", "--host", "-m", path.to_str().expect("a path")]);
        let (mut round, rig) = open_online(&args, args.map.as_ref().expect("the map"), &cli_identity(&args)).expect("a round");
        assert!(rig.is_none());
        round.frame(&Intent::default(), 1.0 / 60.0);
        let why = crate::text::text().get(crate::text::keys::NOTE_NOT_CLEARED);
        assert_eq!(round.note(), Some(why.as_str()));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A stall runs the cap and forgets the rest: the next frame owes
    /// nothing extra, and a reset clock owes nothing at all.
    #[test]
    fn the_step_clock_caps_a_stall_and_drops_the_remainder() {
        let mut clock = StepClock::default();
        assert_eq!(clock.advance(2.0), SIM_MAX_STEPS_PER_FRAME);
        assert_eq!(clock.advance(0.0), 0);
        assert_eq!(clock.advance(1.0 / 60.0), 1);
        let mut clock = StepClock::default();
        clock.advance(0.9 / 60.0);
        clock.reset();
        assert_eq!(clock.advance(0.2 / 60.0), 0, "a reset forgets the time owed");
        assert_eq!(clock.advance(-1.0), 0, "a negative dt owes nothing");
    }
}
