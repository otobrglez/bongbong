//! The iOS entry (docs/ios-native-port-prd.md): the game is raylib on its
//! SDL3 backend, and on iOS SDL has to start the application itself -
//! `SDL_RunApp` runs UIApplicationMain and calls `app_main` from the app
//! delegate once the app has launched. Everything here is plain SDL3 ABI,
//! declared by hand because sola-raylib does not bind SDL.

use clap::Parser as _;
use std::ffi::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

#[repr(C)]
struct SdlRect {
    x: c_int,
    y: c_int,
    w: c_int,
    h: c_int,
}

const SDL_INIT_VIDEO: u32 = 0x20;

type BindFn = unsafe extern "C" fn(target: u32, id: u32);

unsafe extern "C" {
    fn SDL_RunApp(
        argc: c_int,
        argv: *mut *mut c_char,
        main_fn: extern "C" fn(c_int, *mut *mut c_char) -> c_int,
        reserved: *mut c_void,
    ) -> c_int;
    fn SDL_Init(flags: u32) -> bool;
    fn SDL_GetPrimaryDisplay() -> u32;
    fn SDL_GetDisplayBounds(display: u32, rect: *mut SdlRect) -> bool;
    fn SDL_GetWindowProperties(window: *mut c_void) -> u32;
    fn SDL_GetNumberProperty(props: u32, name: *const c_char, default: i64) -> i64;
    fn SDL_SetHint(name: *const c_char, value: *const c_char) -> bool;
    fn SDL_GetWindowSafeArea(window: *mut c_void, rect: *mut SdlRect) -> bool;
    fn SDL_GetWindowSizeInPixels(window: *mut c_void, w: *mut c_int, h: *mut c_int) -> bool;
    /// glad's entries for glBindFramebuffer/glBindRenderbuffer inside
    /// libraylib.a: raylib was built with glad loading GL ES through
    /// SDL_GL_GetProcAddress, so every rlgl GL call goes through a
    /// table of writable pointers.
    static mut glad_glBindFramebuffer: Option<BindFn>;
    static mut glad_glBindRenderbuffer: Option<BindFn>;
}

static REAL_BIND_FRAMEBUFFER: OnceLock<BindFn> = OnceLock::new();
static REAL_BIND_RENDERBUFFER: OnceLock<BindFn> = OnceLock::new();
static SCREEN_FRAMEBUFFER: AtomicU32 = AtomicU32::new(0);
static SCREEN_RENDERBUFFER: AtomicU32 = AtomicU32::new(0);

unsafe extern "C" fn bind_framebuffer_routed(target: u32, id: u32) {
    let id = if id == 0 { SCREEN_FRAMEBUFFER.load(Ordering::Relaxed) } else { id };
    if let Some(real) = REAL_BIND_FRAMEBUFFER.get() {
        // SAFETY: the pointer glad loaded; same signature.
        unsafe { real(target, id) }
    }
}

unsafe extern "C" fn bind_renderbuffer_routed(target: u32, id: u32) {
    let id = if id == 0 { SCREEN_RENDERBUFFER.load(Ordering::Relaxed) } else { id };
    if let Some(real) = REAL_BIND_RENDERBUFFER.get() {
        // SAFETY: as above.
        unsafe { real(target, id) }
    }
}

/// On iOS there is no window-system framebuffer: SDL draws the window
/// through a framebuffer object and a renderbuffer of its own, and
/// "object 0" is nothing. rlgl binds framebuffer 0 to get back to the
/// screen after every render-texture pass, and renderbuffer 0 after
/// building a render texture's depth buffer - and SDL's swap presents
/// whichever renderbuffer is bound, so after the first render texture
/// every frame would be drawn into the void and the swap would fail
/// with GL_INVALID_OPERATION. Route both 0s to SDL's objects by
/// wrapping glad's pointers, once the window (and with it SDL's
/// objects) exists.
pub fn route_default_framebuffer(rl: &mut sola_raylib::RaylibHandle) {
    const GL_FRAMEBUFFER: u32 = 0x8D40;
    const GL_RENDERBUFFER: u32 = 0x8D41;
    // SAFETY: main thread, right after InitWindow; the glad table is
    // loaded and nothing else writes it.
    unsafe {
        let props = SDL_GetWindowProperties(rl.get_window_handle());
        let fbo = SDL_GetNumberProperty(props, c"SDL.window.uikit.opengl.framebuffer".as_ptr(), 0);
        let rbo = SDL_GetNumberProperty(props, c"SDL.window.uikit.opengl.renderbuffer".as_ptr(), 0);
        if fbo <= 0 || rbo <= 0 {
            eprintln!("bongbong: SDL reports no iOS framebuffer/renderbuffer; drawing to object 0");
            return;
        }
        SCREEN_FRAMEBUFFER.store(fbo as u32, Ordering::Relaxed);
        SCREEN_RENDERBUFFER.store(rbo as u32, Ordering::Relaxed);
        let fb_slot = &raw mut glad_glBindFramebuffer;
        if let Some(real) = (*fb_slot).take() {
            let _ = REAL_BIND_FRAMEBUFFER.set(real);
            *fb_slot = Some(bind_framebuffer_routed);
            real(GL_FRAMEBUFFER, fbo as u32);
        }
        let rb_slot = &raw mut glad_glBindRenderbuffer;
        if let Some(real) = (*rb_slot).take() {
            let _ = REAL_BIND_RENDERBUFFER.set(real);
            *rb_slot = Some(bind_renderbuffer_routed);
            real(GL_RENDERBUFFER, rbo as u32);
        }
    }
}

pub fn main() -> ! {
    // SAFETY: called once, on the main thread, before any other SDL use.
    let code = unsafe { SDL_RunApp(0, std::ptr::null_mut(), app_main, std::ptr::null_mut()) };
    std::process::exit(code)
}

extern "C" fn app_main(_argc: c_int, _argv: *mut *mut c_char) -> c_int {
    // The bundle is flat (BongBong.app/{bongbong, static/, maps/} -
    // tools/ios/bundle.sh), so with the bundle as the working directory
    // every `static/...` and `maps/...` path resolves as on desktop.
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|d| d.to_path_buf())) {
        if let Err(e) = std::env::set_current_dir(&dir) {
            eprintln!("bongbong: cannot enter the app bundle {}: {e}", dir.display());
        }
    }
    super::run(super::Args::parse_from(["bongbong"]));
    0
}

/// SDL hints that have to be set before the window exists. The home
/// indicator: "2" dims it and defers the system edge gestures, so a
/// thumb sliding along the bottom of the field steers the tank instead
/// of opening the app switcher; the first swipe only brings the
/// indicator back, the second one leaves the game.
pub fn set_hints() {
    // SAFETY: plain SDL calls with static C strings.
    unsafe {
        SDL_SetHint(c"SDL_IOS_HIDE_HOME_INDICATOR".as_ptr(), c"2".as_ptr());
    }
}

/// One log line with the window in points, the drawable in pixels and
/// the safe area (the notch or Dynamic Island and the home indicator
/// cut into it), so a console capture shows what the phone drew into.
pub fn log_screen_geometry(rl: &mut sola_raylib::RaylibHandle) {
    // SAFETY: main thread, after InitWindow; the out-parameters are ours.
    unsafe {
        let window = rl.get_window_handle();
        let (mut pw, mut ph) = (0, 0);
        SDL_GetWindowSizeInPixels(window, &mut pw, &mut ph);
        let mut safe = SdlRect { x: 0, y: 0, w: 0, h: 0 };
        SDL_GetWindowSafeArea(window, &mut safe);
        eprintln!(
            "bongbong: iOS window {}x{} pt, drawable {}x{} px, safe area {}x{} at ({}, {})",
            rl.get_screen_width(),
            rl.get_screen_height(),
            pw,
            ph,
            safe.w,
            safe.h,
            safe.x,
            safe.y
        );
    }
}

/// The screen in points, landscape, read from SDL before raylib's
/// InitWindow: the SDL backend never re-reads the window size, so the
/// size InitWindow is asked for is the one it renders at. `SDL_Init` is
/// reference-counted, so raylib's own call just bumps the count.
pub fn screen_size_points() -> Option<(i32, i32)> {
    // SAFETY: plain SDL calls on the main thread; the rect is ours.
    unsafe {
        if !SDL_Init(SDL_INIT_VIDEO) {
            return None;
        }
        let display = SDL_GetPrimaryDisplay();
        if display == 0 {
            return None;
        }
        let mut rect = SdlRect { x: 0, y: 0, w: 0, h: 0 };
        if !SDL_GetDisplayBounds(display, &mut rect) || rect.w <= 0 || rect.h <= 0 {
            return None;
        }
        // The app is landscape-only (Info.plist); this early the display
        // may still report portrait bounds, so orient by hand.
        Some((rect.w.max(rect.h), rect.w.min(rect.h)))
    }
}
