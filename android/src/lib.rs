//! The Android entry shell (docs/android-port-prd.md): raylib's
//! `android_main` (rcore_android.c, compiled into the prebuilt libraylib.a
//! together with the NDK's native_app_glue) calls a C `main(argc, argv)`;
//! this is it. The game is `bongbong::app`; this crate exists so the
//! exported symbol sits in the cdylib's own objects (see
//! `bongbong::capi::keep_alive` for why an rlib-only `#[no_mangle]` is not
//! enough). Built by `cargo ndk` from `just build-android`.
#![cfg(target_os = "android")]
use std::ffi::{c_char, c_int, c_void};

#[unsafe(no_mangle)]
pub extern "C" fn main(_argc: c_int, _argv: *mut *mut c_char) -> c_int {
    bongbong::app::android::app_main()
}

unsafe extern "C" {
    fn ANativeActivity_onCreate(activity: *mut c_void, saved_state: *mut c_void, saved_state_size: usize);
}

/// The activity entry NativeActivity looks up by name
/// (`android.app.func_name` in the manifest). rustc links a cdylib with a
/// version script that hides every symbol but this crate's own exports, so
/// native_app_glue's `ANativeActivity_onCreate` inside libraylib.a is
/// linked in (`-u`) yet invisible to the loader; this wrapper is visible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bongbong_on_create(activity: *mut c_void, saved_state: *mut c_void, saved_state_size: usize) {
    // SAFETY: forwarded verbatim from the activity's own loader call.
    unsafe { ANativeActivity_onCreate(activity, saved_state, saved_state_size) }
}
