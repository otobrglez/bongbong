//! The desktop binary: parse the command line and run the game. The whole
//! game - the `Args` struct, the window and the loop - is `bongbong::app`,
//! shared with the iOS entry (`app::ios`, SDL starts the application) and
//! the Android cdylib (`android/`, raylib's `android_main` calls `main`).

fn main() {
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
