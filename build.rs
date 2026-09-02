//! Build script: on the web target, with the `dev-tools` feature on, tell
//! emcc to export the tuning C API (src/capi.rs) from the `bongbong` wasm
//! binary so the page's tuning panel can `Module.ccall` it. Without the
//! feature nothing is added and the link is byte-for-byte what it was.
//!
//! Lives here rather than in `.cargo/config.toml`'s static rustflags
//! because the export list has to be conditional on a cargo feature (the
//! symbols don't exist in a non-dev build, and emcc errors on an undefined
//! exported symbol), and because `cargo:rustc-link-arg-bin` scopes it to
//! the game binary only - `probe` (also built by a bare
//! `cargo build --target wasm32-unknown-emscripten`) never references the
//! API and must not be asked to export it.
//!
//! `_main` has to be listed explicitly: setting EXPORTED_FUNCTIONS replaces
//! emcc's default list, which is just `_main`. The bb_* names must match
//! `capi::EXPORTS` (a unit test there checks this file mentions each one).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").unwrap_or_default();
    let dev_tools = std::env::var_os("CARGO_FEATURE_DEV_TOOLS").is_some();
    if dev_tools && target == "wasm32-unknown-emscripten" {
        let exports = [
            "_main",
            "_bb_tuning_schema_json",
            "_bb_tuning_current_json",
            "_bb_tuning_diff_json",
            "_bb_tuning_diff_rust",
            "_bb_tuning_apply_json",
            "_bb_last_error",
            "_bb_tuning_reset",
            "_bb_game_restart",
        ];
        println!(
            "cargo:rustc-link-arg-bin=bongbong=-sEXPORTED_FUNCTIONS={}",
            exports.join(",")
        );
    }
}
