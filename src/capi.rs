//! The `dev-tools` C ABI over `tuning.rs` - what the web page's tuning panel
//! (site/src/pages/index.astro) calls through Emscripten's `Module.ccall`,
//! and what a native transport (a local HTTP/MCP bridge, later) would bind
//! to. See docs/runtime-tuning-design.md §6.
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

use crate::tuning;

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
}
