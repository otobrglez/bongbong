//! Build script: on the web target, with the `dev-tools` feature on, tell
//! emcc to export the tuning C API (src/capi.rs) from the `bongbong` wasm
//! binary so the page's tuning panel can `Module.ccall` it, and to link
//! binaryen's stack-overflow check (see `main`). Without the feature
//! nothing is added and the link is byte-for-byte what it was.
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

//!
//! On iOS targets it also links the prebuilt raylib + SDL3 slice from
//! tools/setup_ios.sh (see `ios_link`).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("apple-ios") {
        ios_link();
    }
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
        // A stack overflow is silent in a plain web build: the shadow stack
        // sits just above the static data, so it corrupts whatever lives
        // there (stb_image's tables are the first casualty, and the game then
        // fails somewhere unrelated, e.g. "Failed to load image data" on a
        // valid PNG). Binaryen's check makes every frame allocation abort
        // with the stack limits instead - worth its compare-per-call on the
        // QA surface, since PR previews are dev-tools builds. The stack's
        // size itself is `-sSTACK_SIZE` in .cargo/config.toml.
        println!("cargo:rustc-link-arg-bin=bongbong=-sSTACK_OVERFLOW_CHECK=2");
    }
}

/// iOS: link the raylib (SDL backend, OpenGL ES 2.0) and static SDL3 slice
/// that tools/setup_ios.sh installs under `$BONGBONG_IOS_LIBS` (default
/// ~/.local/share/bongbong-ios/sim). sola-raylib is built with `nobuild`
/// for iOS (Cargo.toml), so nothing else emits a raylib link line. The
/// frameworks come from SDL's own pkg-config file rather than a copied
/// list, so an SDL upgrade cannot leave them stale. Lives here rather than
/// in .cargo/config.toml because the prefix needs `$HOME`, the framework
/// list is read from a file, and the link args stay scoped to this package.
fn ios_link() {
    use std::path::PathBuf;
    println!("cargo:rerun-if-env-changed=BONGBONG_IOS_LIBS");
    let prefix = std::env::var_os("BONGBONG_IOS_LIBS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let data = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").expect("HOME")).join(".local/share"));
            data.join("bongbong-ios/sim")
        });
    let lib = prefix.join("lib");
    let pc_path = lib.join("pkgconfig/sdl3.pc");
    println!("cargo:rerun-if-changed={}", pc_path.display());
    let pc = std::fs::read_to_string(&pc_path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}\nthe iOS simulator libraries are missing - run `just ios-setup` (tools/setup_ios.sh)",
            pc_path.display()
        )
    });
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=static=raylib");
    println!("cargo:rustc-link-lib=static=SDL3");
    // "Libs: -L${libdir} -lSDL3 -Wl,-framework,UIKit -Wl,-weak_framework,CoreHaptics ..."
    for line in pc.lines().filter(|l| l.starts_with("Libs:") || l.starts_with("Libs.private:")) {
        let mut tokens = line.split_once(':').map(|(_, rest)| rest).unwrap_or("").split_whitespace();
        while let Some(token) = tokens.next() {
            if let Some(fw) = token.strip_prefix("-Wl,-framework,") {
                println!("cargo:rustc-link-lib=framework={fw}");
            } else if let Some(fw) = token.strip_prefix("-Wl,-weak_framework,") {
                println!("cargo:rustc-link-arg-bins=-Wl,-weak_framework,{fw}");
            } else if token == "-framework" {
                if let Some(fw) = tokens.next() {
                    println!("cargo:rustc-link-lib=framework={fw}");
                }
            } else if let Some(name) = token.strip_prefix("-l") {
                if name != "SDL3" {
                    println!("cargo:rustc-link-lib={name}");
                }
            }
        }
    }
    // raylib's own GL calls (through glad) resolve against OpenGLES.framework.
    println!("cargo:rustc-link-lib=framework=OpenGLES");
    // Static SDL3 carries Objective-C categories; without -ObjC the linker
    // drops the archive members that only define them.
    println!("cargo:rustc-link-arg-bins=-ObjC");
}
