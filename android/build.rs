//! Link glue for the Android cdylib. sola-raylib is `nobuild` on Android
//! (../Cargo.toml), so nothing else names libraylib.a or the platform
//! libraries; the prebuilt raylib comes from tools/setup_android.sh
//! (PLATFORM=Android, OpenGL ES 2.0, native_app_glue compiled in). raylib's
//! own Makefile is the reference for the link args: `--wrap=fopen` routes
//! every asset read through raylib's `__wrap_fopen` into the APK's
//! `assets/` (without it every LoadTexture fails silently), `-u
//! ANativeActivity_onCreate` keeps the activity entry that lives in the
//! static archive, and 16 KB pages are what Android 15+ devices want.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("android") {
        return; // a host `cargo check --workspace`: an empty cdylib
    }
    println!("cargo:rerun-if-env-changed=BONGBONG_ANDROID_LIBS");
    let abi = match target.as_str() {
        "aarch64-linux-android" => "arm64-v8a",
        "armv7-linux-androideabi" => "armeabi-v7a",
        "x86_64-linux-android" => "x86_64",
        t => panic!("unsupported Android target {t}"),
    };
    let prefix = std::env::var_os("BONGBONG_ANDROID_LIBS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let data = std::env::var_os("XDG_DATA_HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME")).join(".local/share"));
            data.join("bongbong-android")
        });
    let lib = prefix.join(abi).join("lib");
    assert!(
        lib.join("libraylib.a").is_file(),
        "{}/libraylib.a is missing - run `just android-setup` (tools/setup_android.sh)",
        lib.display()
    );
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=static=raylib");
    for l in ["log", "android", "EGL", "GLESv2", "OpenSLES", "c", "m"] {
        println!("cargo:rustc-link-lib={l}");
    }
    for arg in ["-Wl,--wrap=fopen", "-Wl,-u,ANativeActivity_onCreate", "-Wl,-z,max-page-size=16384"] {
        println!("cargo:rustc-cdylib-link-arg={arg}");
    }
}
