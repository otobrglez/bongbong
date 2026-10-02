//! The web build's window is the page's canvas
//! (docs/large-maps-follow-camera.md §10).
//!
//! The page lays the canvas out to fill its stage at whatever shape the
//! page, `.immersive` or full screen gives it (site/src/pages/index.astro),
//! and the game makes its window - the canvas's drawing buffer - that box
//! in device pixels (`view::canvas_buffer`), once a frame before anything
//! reads the window's size: a resize, a rotation, full screen or a
//! browser bar collapsing re-frames the view on the next frame. The
//! window is then in device pixels, as Android's is, and
//! `units_per_point` is how many of them make a CSS pixel - the web's
//! point, which the chrome, the touch controls and the framing rules are
//! laid out in. Every pointer stays in the window's units on the way in:
//! emscripten's GLFW scales a mouse position by the window's width over
//! the canvas's CSS width, and raylib a touch by the screen's width over
//! the same, so a box of any shape maps exactly once the buffer fills it.
//!
//! Every reading is a synchronous call into emscripten's runtime - the box
//! through `emscripten_get_element_css_size`, the ratio through
//! `emscripten_get_device_pixel_ratio` - and the resize is raylib's own
//! `SetWindowSize`, so none of it needs `-sASYNCIFY`. raylib's resizable
//! flag stays off: its callback sizes the canvas to the tab
//! (`window.innerWidth` x `innerHeight`, in CSS pixels), which is neither
//! the canvas's box nor in device pixels.

use std::cell::Cell;
use std::ffi::CStr;
use std::os::raw::{c_char, c_double, c_int};

use sola_raylib::prelude::RaylibHandle;

unsafe extern "C" {
    fn emscripten_get_element_css_size(target: *const c_char, width: *mut c_double, height: *mut c_double) -> c_int;
    fn emscripten_get_device_pixel_ratio() -> c_double;
}

/// The canvas the game draws into: index.astro's `<canvas id="canvas">`,
/// which site/src/scripts/runtime.ts hands the runtime as `Module.canvas`.
const CANVAS: &CStr = c"#canvas";

thread_local! {
    /// Window units per CSS pixel as `follow_canvas` last sized the
    /// window; zero before it has.
    static UNITS: Cell<f32> = const { Cell::new(0.0) };
}

/// The drawing buffer the canvas's box needs now, and how many of its
/// pixels make a CSS pixel (`view::canvas_buffer`). `None` while the
/// canvas has no box.
pub fn canvas_buffer() -> Option<((i32, i32), f32)> {
    let (mut width, mut height): (c_double, c_double) = (0.0, 0.0);
    // SAFETY: a NUL-terminated selector and two live out-parameters,
    // filled synchronously by the runtime.
    let found = unsafe { emscripten_get_element_css_size(CANVAS.as_ptr(), &mut width, &mut height) } == 0;
    // SAFETY: a plain read of `window.devicePixelRatio`.
    let dpr = unsafe { emscripten_get_device_pixel_ratio() };
    if !found {
        return None;
    }
    crate::view::canvas_buffer((width as f32, height as f32), dpr as f32)
}

/// Make the window the canvas's box (`canvas_buffer`), if it is not
/// already. Asked twice at most: GLFW's port takes any element in full
/// screen - the page's `.game` - for its own canvas gone full screen, so
/// the first resize there is answered with the screen's size in CSS
/// pixels and the first one after it with the size from before, and in
/// both cases a second request lands where the first was meant to.
pub fn follow_canvas(rl: &mut RaylibHandle) {
    let Some(((width, height), units)) = canvas_buffer() else { return };
    UNITS.with(|u| u.set(units));
    for _ in 0..2 {
        if (rl.get_screen_width(), rl.get_screen_height()) == (width, height) {
            return;
        }
        rl.set_window_size(width, height);
    }
}

/// Window units per point on the web: the drawing buffer's pixels per CSS
/// pixel as the window was last sized, and the device pixel ratio before
/// it has been.
pub fn units_per_point(rl: &RaylibHandle) -> f32 {
    let units = UNITS.with(Cell::get);
    if units > 0.0 {
        units
    } else {
        Some(rl.get_window_scale_dpi().x).filter(|s| s.is_finite() && *s > 0.0).unwrap_or(1.0)
    }
}
