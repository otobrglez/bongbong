//! The Android entry (docs/android-port-prd.md): the game is raylib on its
//! own Android platform, a NativeActivity whose `android_main`
//! (rcore_android.c) calls a C `main` that the `android/` cdylib exports;
//! that `main` calls `app_main` here. Everything the game prints goes to
//! stderr, which Android discards, so the first thing this does is route
//! stderr into logcat under the tag `bongbong`.
use clap::Parser as _;
use std::ffi::{c_char, c_int, c_void, CString};

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}
unsafe extern "C" {
    fn pipe(fds: *mut c_int) -> c_int;
    fn dup2(old: c_int, new: c_int) -> c_int;
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
}

const ANDROID_LOG_INFO: c_int = 4;
const ANDROID_LOG_ERROR: c_int = 6;

unsafe extern "C" {
    fn __system_property_get(name: *const c_char, value: *mut c_char) -> c_int;
}

/// Whether this is the Android Emulator: its EGL swap does not wait for
/// the display, so the loop needs raylib's frame cap there, while a phone
/// is paced by its display. `ro.kernel.qemu` is 1 on every emulator image.
pub fn is_emulator() -> bool {
    let mut buf = [0 as c_char; 92]; // PROP_VALUE_MAX
    // SAFETY: the buffer is PROP_VALUE_MAX bytes, as the API requires.
    let n = unsafe { __system_property_get(c"ro.kernel.qemu".as_ptr(), buf.as_mut_ptr()) };
    n > 0 && buf[0] as u8 == b'1'
}

/// One line to logcat, tag `bongbong`.
pub fn log(prio: c_int, msg: &str) {
    let msg = CString::new(msg.replace('\0', "?")).unwrap_or_default();
    // SAFETY: NUL-terminated strings; liblog copies them.
    unsafe {
        __android_log_write(prio, c"bongbong".as_ptr(), msg.as_ptr());
    }
}

/// Replace fd 2 with a pipe whose reader thread forwards every line to
/// logcat: `eprintln!`, a texture `expect`, the panic message, all of it.
fn forward_stderr_to_logcat() {
    let mut fds = [0 as c_int; 2];
    // SAFETY: plain libc calls on our own descriptors.
    unsafe {
        if pipe(fds.as_mut_ptr()) != 0 || dup2(fds[1], 2) < 0 {
            return;
        }
    }
    let read_end = fds[0];
    std::thread::Builder::new()
        .name("stderr->logcat".into())
        .spawn(move || {
            let mut pending = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                // SAFETY: reading our pipe into a local buffer.
                let n = unsafe { read(read_end, buf.as_mut_ptr() as *mut c_void, buf.len()) };
                if n <= 0 {
                    break;
                }
                pending.extend_from_slice(&buf[..n as usize]);
                while let Some(nl) = pending.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=nl).collect();
                    log(ANDROID_LOG_INFO, String::from_utf8_lossy(&line[..nl]).as_ref());
                }
            }
        })
        .ok();
}

/// Runs the game with default options and returns what raylib's `main`
/// should return (`android_main` ignores it and finishes the activity).
/// A panic is logged and turned into a 1: unwinding out of an `extern "C"`
/// frame would abort the process before anything reached logcat.
pub fn app_main() -> c_int {
    forward_stderr_to_logcat();
    std::panic::set_hook(Box::new(|info| log(ANDROID_LOG_ERROR, &format!("panic: {info}"))));
    match std::panic::catch_unwind(|| super::run(super::Args::parse_from(["bongbong"]))) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}
