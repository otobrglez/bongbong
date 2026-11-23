//! The desktop binary: parse the command line and run the game. The whole
//! game - the `Args` struct, the window and the loop - is `bongbong::app`,
//! shared with the iOS entry (`app::ios`, SDL starts the application) and
//! the Android cdylib (`android/`, raylib's `android_main` calls `main`).
//!
//! A Windows release build is a window program, not a console one, so a
//! double-click opens the game's window and no console beside it
//! (docs/platforms.md, "Releases"). Started from a terminal it attaches to
//! that terminal (`attach_parent_console`), so `--help`, argument errors and
//! the game's own logs still print there. A debug build keeps its console.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    #[cfg(all(windows, not(debug_assertions)))]
    attach_parent_console();
    #[cfg(not(target_os = "ios"))]
    {
        use clap::Parser as _;
        bongbong::app::run(bongbong::app::Args::parse());
    }
    // iOS: UIKit owns the process. SDL starts the application and calls
    // back into `app::ios::app_main`, which runs the game with default
    // options from inside the app bundle.
    #[cfg(target_os = "ios")]
    bongbong::app::ios::main();
}

/// Writes to the console of the terminal the game was started from, if it
/// was started from one. A window program starts with no console; attaching
/// sets the standard handles Rust's `stdout`/`stderr` write through. Started
/// from Explorer there is no parent console, the call fails, and there is
/// nowhere to print, which is what a double-click wants. Must run before
/// anything prints - clap's `--help` and its errors included.
#[cfg(all(windows, not(debug_assertions)))]
fn attach_parent_console() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    /// `ATTACH_PARENT_PROCESS`, `(DWORD)-1`: the console of the process
    /// that started this one.
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // SAFETY: a plain Win32 call with no pointers; failure (no parent
    // console, or one already attached) is a return value, not UB.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
