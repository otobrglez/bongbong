# PRD: bongbong as a native Android app

Written 2026-09-14, after the iOS simulator and device builds landed
(docs/ios-native-port-prd.md section 4.1, PR #14). Same author, same
shape: the decision, what exists today, the design by area, the phases,
the risks. Nothing in the tree targets Android yet; Android Studio 2026.1
and its SDK were installed at `~/Library/Android/sdk` the same day.

## 1. The decision, in one paragraph

Build the game as a **shared library** for `aarch64-linux-android` on
raylib 6.0's own Android platform (a `NativeActivity` through
`android_native_app_glue`, OpenGL ES 2.0 over EGL), link it the way iOS is
linked (a prebuilt `libraylib.a` through sola-raylib's `nobuild` feature,
link lines in our own build script), package it into an APK **by hand
with the build-tools already installed** (aapt2, zipalign, apksigner; no
Gradle, no Java code) and run it on the Android Emulator, which on Apple
Silicon runs arm64 images natively, so the emulator build is the phone
build. The one structural change to the game is that the game loop moves
from the desktop binary into the library, where Android can export a C
`main` for raylib to call. Estimate: two to three days to a playable round
on the emulator, of which most is toolchain and packaging; the game-logic
diff is three constants and one window-size branch.

## 2. Goals and non-goals

Goals
- The whole game, unchanged simulation, maps, AI and tuning, running on an
  arm64 Android emulator and then on a phone, playable by touch with the
  scheme the iOS build uses (docs/fullscreen-resolution-research.md
  section 5), at the display's native resolution.
- Driven from the shell the way iOS is: `just android-setup`,
  `android-smoke`, `build-android`, `run-android`, `android-screenshot`,
  plus `adb shell input` for touches, which the iOS simulator could not do.
- Verifiable by the same evidence as iOS: logcat lines from raylib and the
  game, screenshots, the dev-tools frame stats.

Non-goals, for now
- The Play Store, an AAB, an upload key, target-API paperwork: phase 4.
- Audio (raylib's audio module compiles as plain C on Android; there is
  simply no audio in the game yet).
- x86_64 emulator images (the Mac's emulator runs arm64 natively).
- A Gradle or Android Studio project. It is not needed for a NativeActivity
  app and would be a second build system to keep alive.

## 3. What we have today (verified 2026-09-14)

### This Mac
- Android Studio 2026.1 at `/Applications/Android Studio.app`, with its
  JBR (OpenJDK 25). `JAVA_HOME`, `ANDROID_HOME` unset.
- `~/Library/Android/sdk`: `platform-tools` (adb 37.0.1), `emulator`
  37.1.11, `build-tools/36.0.0` (aapt2, zipalign, apksigner, d8),
  `platforms/android-37.0` (android.jar), the accepted SDK licence, and
  `~/.android/debug.keystore` for signing.
- **Missing**: `cmdline-tools` (so no `sdkmanager`, no `avdmanager`), any
  NDK, any system image, any AVD. The emulator cannot boot anything and
  nothing can be compiled for Android. There is no Gradle either, and the
  route below never needs one.
- Rust 1.97.1 from nix through devenv: no `aarch64-linux-android` std yet
  (it is a `devenv.nix` targets entry away, like the iOS targets). No
  `cargo-ndk`.

### The game
- The platform policy is three constants in `src/lib.rs`: `EMBEDDED`
  (web and iOS: GLSL ES 100 shaders from `static/web/`, no map saving, no
  resize or high-DPI request, lower particle budget, no view cap),
  `KEYBOARD_AVAILABLE` (false on iOS: the RESTART button replaces the R
  key, no two-player mode) and `TWO_PLAYERS_AVAILABLE`. Adding Android to
  the first two is the entire game-logic side of the port.
- Touch input in `src/main.rs` is platform-neutral raylib API
  (`get_touch_point_count`, `get_touch_position`, `get_touch_point_id`);
  `src/touch.rs` and `src/view.rs` have no platform code.
- Assets are loaded by CWD-relative path through raylib (`static/...`);
  the shipped maps are `include_str!`; every filesystem write is behind
  `map::saving_available()` = `!EMBEDDED` or behind `dev-tools`.
- The whole game loop, the clap `Args` and the iOS module live in
  `src/main.rs` (about 1250 lines). `Cargo.toml` has no `[lib]` section:
  the lib crate is a plain rlib.

### raylib's Android platform (vendored in `sola-raylib-sys-6.3.0/raylib`)
- `src/platforms/rcore_android.c`: `android_main` calls a C
  `main(1, {"raylib", NULL})` the app must define, then finishes the
  activity. `InitPlatform` blocks until the window exists; EGL asks for an
  ES 2.0 context; `eglSwapInterval` is never called, so EGL's default
  interval of 1 paces the swap.
- `InitWindow(0, 0)`: `SetupFramebuffer` sets the screen to the display's
  native pixels, identity scale, no offsets. A non-zero request smaller
  than the display is *not* letterboxed by raylib but rendered at that
  size and upscaled by the compositor, so the game must pass zero.
- Touch: up to 8 points, in screen units, first point mirrored onto the
  mouse. Back and Menu keys are swallowed (readable as `KEY_BACK`),
  `SetExitKey` is ignored, the only exit is `request_quit`.
- Lifecycle: on `APP_CMD_TERM_WINDOW` only the EGL surface is destroyed,
  the context and every texture survive; `APP_CMD_CONFIG_CHANGED` is a
  stub, so the manifest must declare `configChanges`; while unfocused,
  `PollInputEvents` blocks in the looper, which is the right behaviour.
- Assets: raylib 6.0 intercepts `fopen` by **link-time wrapping**
  (`-Wl,--wrap=fopen`); `__wrap_fopen` opens read paths through
  `AAssetManager` and write paths under `internalDataPath`. The flag is a
  CMake `PUBLIC` link option, which cargo never inherits, so the final
  `.so` link must pass it or every `LoadTexture` fails silently. Rust's
  `std::fs` never goes through `fopen` and does not see assets.
- Logging: `TRACELOG` goes to logcat under the tag `raylib`.
- CMake with `PLATFORM=Android` compiles the NDK's
  `android_native_app_glue.c` into `libraylib.a`, so
  `ANativeActivity_onCreate` and `android_main` come from the archive and
  the final link must keep that member (`-Wl,-u,ANativeActivity_onCreate`).
  The audio module needs no Objective-C trick (miniaudio dlopens AAudio
  and OpenSL ES at runtime).

### sola-raylib's Android branch (why we do not use it)
`sola-raylib-sys` build.rs has a `Platform::Android` cmake branch, but it
derives the API level from the last dash-separated part of the target
triple, which for `aarch64-linux-android` is the literal `android`, panics
for x86 ABIs, runs bindgen with the host triple and no NDK sysroot, and
emits neither of the two link args above. Under `nobuild` (the iOS route)
all of that is skipped; bindgen and the small cc-rs shim still run and are
pointed at the NDK by environment variables, which `cargo-ndk` sets.

## 4. Design, by area

### 4.1 Versions, one variable each (`tools/android/env.sh`)
| Variable | Value | Why |
|---|---|---|
| `ANDROID_API_MIN` | 29 | raylib's own default; Android 10, phones from 2019 on; AAudio needs 26+ |
| `ANDROID_API_TARGET` | 36 | Play requires API 36 for new apps from 31 Aug 2026 |
| platform (android.jar) | `platforms;android-36` | the installed `android-37.0` also works for `aapt2 link` |
| NDK | newest r28 | 16 KB page alignment by default |
| system image | `system-images;android-36;google_apis;arm64-v8a` | native on Apple Silicon |
| AVD | `bongbong`, `pixel_7` profile | one name the recipes boot |
| `JAVA_HOME` | Android Studio's JBR | `sdkmanager`, `apksigner` are Java |

### 4.2 Toolchain (`tools/setup_android.sh`, `tools/android/env.sh`, `devenv.nix`)
`env.sh` is the Android twin of `tools/ios/env.sh`: `ANDROID_HOME`,
`ANDROID_NDK_HOME` (from the pinned NDK id), `JAVA_HOME`, the tool
directories on `PATH`, the API variables, `BONGBONG_ANDROID_LIBS`
(default `~/.local/share/bongbong-android`), `BONGBONG_AVD`. It also
exports the NDK compiler for cargo so the devenv shell's nix `CC`/`LD`
cannot leak into an Android build, the trap iOS hit:
`CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER` and
`CC_/CXX_/AR_aarch64_linux_android` (`aarch64-linux-android29-clang`,
`llvm-ar`), and bindgen's sysroot
(`BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android=--sysroot=<ndk>/toolchains/llvm/prebuilt/darwin-x86_64/sysroot`;
the NDK's `darwin-x86_64` directory holds fat binaries and runs natively).

`setup_android.sh`, idempotent through marker files:
1. Download the pinned `commandlinetools-mac-*_latest.zip` into
   `$ANDROID_HOME/cmdline-tools/latest`, accept licences
   (`sdkmanager --licenses`).
2. `sdkmanager "ndk;<pinned>" "platforms;android-36"
   "system-images;android-36;google_apis;arm64-v8a"` (several GB).
3. `avdmanager create avd -n bongbong -k <image> -d pixel_7` if absent.
4. `cargo install cargo-ndk --version 4.1.2` if absent (into `~/.cargo/bin`).
5. Build raylib: copy the vendored tree to
   `~/.local/share/bongbong-android/src/raylib` (the registry is never
   edited), then
   `cmake -DPLATFORM=Android -DCMAKE_TOOLCHAIN_FILE=$NDK/build/cmake/android.toolchain.cmake -DANDROID_ABI=arm64-v8a -DANDROID_PLATFORM=android-29 -DBUILD_EXAMPLES=OFF -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=$BONGBONG_ANDROID_LIBS/arm64-v8a`,
   build, install. Check: `llvm-nm` finds `android_main` and
   `ANativeActivity_onCreate` in `libraylib.a`.

`devenv.nix`: `"aarch64-linux-android"` joins `languages.rust.targets`
(a shell opened earlier needs `direnv reload`, as with iOS).

Alternative for steps 1 and 2: Android Studio's SDK Manager once by hand;
the script then finds everything in place and only builds raylib.

### 4.3 Gate 0: the smoke app (`tools/android/smoke.c`, `smoke.sh`)
raylib's basic-window example with a touch log, compiled with the NDK
clang against the prebuilt `libraylib.a` into `libsmoke.so` (with the two
link args), packaged by the same script as the game under a `Smoke`
manifest, installed and launched on the AVD. Pass: a coloured window in
`adb exec-out screencap -p`, raylib's `Display size` line in logcat, and a
touch line after `adb shell input tap`. It proves the toolchain, the
packaging, GL ES 2 on the emulator, asset access and input before a line
of Rust is touched, and it is the standing check after every SDK or NDK
update, like `just ios-smoke`.

### 4.4 The game as a shared library
**The refactor, first and alone.** `src/main.rs` keeps only `fn main`;
everything else moves into the library as `src/app.rs` (`pub mod app` in
`lib.rs`): `Args` (pub struct, fields private), `run` (pub),
`gather_intents`, `left_shift_down`, `parse_map`, `default_map`,
`parse_resolution`, `shader_path`, `TuningWatch`, `load_ripples`. The
iOS module moves unchanged to `src/app/ios.rs` (`super::run`,
`super::Args` keep resolving); `FrameStats` moves from it into `app.rs`
under `dev-tools`, since it has no platform code. `use bongbong::` becomes
`use crate::`; `capi::keep_alive` stays the first line of `run`; every
`dev-tools` gate is carried over verbatim. `main.rs` has no tests, and
`probe` and `bbmcp` share nothing with it. Desktop, web and both iOS
builds must be unchanged after this step before anything Android starts.
The other session's desktop view-cap work also edits `main.rs`; this
step goes in after that lands.

**Policy.** `EMBEDDED` and `KEYBOARD_AVAILABLE` gain `target_os =
"android"`. That alone gives Android the GLSL ES 100 shaders, no map
saving, no resize/high-DPI/F11, the lower particle budget, the RESTART
button and no two-player mode.

**`run` on Android.** Window size `(0, 0)`; no builder flags; none of the
iOS glue (Android's EGL surface is a real framebuffer 0 and raylib renders
at native pixels, so neither the framebuffer routing nor the high-DPI
patch applies; `View::fit` reads `get_screen_width/height`, the display's
pixels, and touch is in the same space). One `eprintln!` of screen and
render size after init. `target_fps` 0: EGL's swap interval paces the
loop exactly as the iPhone's display did, measured with `FrameStats`.

**The Android entry** (`src/app/android.rs`, `cfg(target_os = "android")`):
`app_main()` first forwards stderr to logcat (a pipe, `dup2` onto fd 2,
a reader thread calling `__android_log_write` with the tag `bongbong`),
so every `eprintln!`, texture `expect` and panic message is visible in
`adb logcat`; installs a panic hook that logs synchronously; then
`catch_unwind(|| run(Args::parse_from(["bongbong"])))` and returns 0 or 1
to raylib's `android_main`, which finishes the activity instead of the
process aborting on an unwinding `extern "C"` frame.

**The `android/` crate.** The exported symbol must live in the cdylib's
own objects: an `#[no_mangle] main` inside the rlib is an unreferenced
archive member the linker drops (the `capi::keep_alive` lesson), and
rustc's cdylib version script hides everything but the crate's own
exports. So a workspace member `android/` (`bongbong-android`,
`crate-type = ["cdylib"]`, `bongbong = { path = ".." }`,
`[package.metadata.dist] dist = false`) whose whole `src/lib.rs` is
`#[unsafe(no_mangle)] pub extern "C" fn main(argc, argv) -> c_int { bongbong::app::android::app_main() }`.
Its `build.rs`, only when `TARGET` contains `android`: search path
`$BONGBONG_ANDROID_LIBS/<abi>/lib`, `static=raylib`, the platform libs
`log android EGL GLESv2 OpenSLES c m`, and `rustc-cdylib-link-arg` for
`-Wl,--wrap=fopen`, `-Wl,-u,ANativeActivity_onCreate`,
`-Wl,-z,max-page-size=16384`. The root `Cargo.toml` gains
`[workspace] members = ["android"]` (the root package stays the default
member, so every desktop command is unchanged, and the lockfile and target
directory are shared) and, next to the iOS block,
`[target.'cfg(target_os = "android")'.dependencies] sola-raylib = { version = "6.3.0", features = ["nobuild"] }`
(resolver 3 scopes it; no `sdl`). Why not `crate-type = ["rlib", "cdylib"]`
on the game crate itself: every desktop `cargo build`, `cargo watch` and
`cargo test` would then also link a full `libbongbong.dylib`, the wasm and
iOS builds would link a cdylib with the wrong arguments, and a bare
`cargo ndk build` would try to build the desktop bins for Android.

**Build.** `cargo ndk -t arm64-v8a -p 29 -o target/android/jniLibs build --release -p bongbong-android`
(cargo-ndk is the sys crate's documented route; it sets the linker,
cc-rs, bindgen sysroot and page-size flags; `ANDROID_NDK_HOME` comes from
`env.sh`). First-link check:
`llvm-nm -D --defined-only libbongbong_android.so | grep -E 'ANativeActivity_onCreate| main$'`.
If `ANativeActivity_onCreate` is hidden, the deterministic fallback is a
Rust trampoline `bongbong_on_create` in `android/src/lib.rs` plus
`android.app.func_name` in the manifest.

### 4.5 Packaging (`tools/android/AndroidManifest.xml`, `package.sh`, `just build-android`)
A `hasCode="false"` NativeActivity APK is a manifest, an `assets/` tree
and one `.so`; no dex, no resources to begin with. The manifest template:
`package="com.otobrglez.bongbong"`, `<application android:hasCode="false"
android:label="BongBong">`, `<activity android:name="android.app.NativeActivity"
android:screenOrientation="sensorLandscape"
android:configChanges="orientation|keyboardHidden|screenSize"
android:theme="@android:style/Theme.NoTitleBar.Fullscreen"
android:launchMode="singleTask">` with
`<meta-data android:name="android.app.lib_name" android:value="bongbong_android"/>`;
`INTERNET` permission only in dev-tools builds (the dev server's socket);
version code and name from `Cargo.toml`; min and target SDK from `env.sh`.

`package.sh` stages `target/android/apk/assets/static/**` (a copy of
`static/` minus `_backup`; `maps/` is not needed, the shipped maps are
compiled in) and `lib/arm64-v8a/libbongbong_android.so`, then
`aapt2 link --manifest ... -I android.jar -A assets --min-sdk-version --target-sdk-version -o unsigned.apk`,
adds the `.so` stored (`zip -0`, so `zipalign -p 4` can page-align it),
`zipalign -p 4`, `apksigner sign --ks ~/.android/debug.keystore --ks-pass pass:android`.
Exactly what raylib's own macOS build notes do, minus dex.

### 4.6 Running and observing
`just run-android`: boot `emulator -avd bongbong -gpu host` if not
running, `adb wait-for-device`, `adb install -r`,
`adb shell am start -n com.otobrglez.bongbong/android.app.NativeActivity`,
then `adb logcat -s raylib:V bongbong:V`. `just android-screenshot` is
`adb exec-out screencap -p`; `just android-tap x y` and a swipe recipe
inject touches, which makes the touch scheme verifiable from the shell for
the first time. With a dev-tools build, `adb forward tcp:4747 tcp:4747`
puts the game's dev server on the Mac's loopback and the existing MCP
tools drive the emulator; the screenshot tool's file write
(`target/devshots` under a read-only `/`) becomes best-effort under
`EMBEDDED`, the PNG still travelling in the reply.

### 4.7 Later polish
Back key opening the leave dialog (raylib eats it, `KEY_BACK` is
readable); an app icon and label (`res/` plus `--auto-add-overlay`);
immersive sticky mode through theme attributes; the punch-hole and
safe-area notes in docs/fullscreen-resolution-research.md section 8;
frame time on a real phone.

## 4.8 As built (2026-09-14, the same day)

Phases 0 to 2 landed in one session; the game runs on the `bongbong` AVD
(Pixel 7 profile, android-36, arm64) at the display's 2400 x 1080, a
round plays through, an injected swipe steers and a tap fires. What
differs from the plan above:

- **No cargo-ndk.** It does not build under the nix toolchain (its link
  wants a libiconv the nix SDK lacks), and it is not needed:
  `tools/android/env.sh` exports the NDK linker, `CC_`/`AR_` for cc-rs and
  bindgen's sysroot, and a plain
  `cargo build --release --target aarch64-linux-android -p bongbong-android`
  links; `android/build.rs` emits the page-size flag itself.
- **The activity entry did need the trampoline.** rustc's cdylib version
  script hid `ANativeActivity_onCreate` exactly as feared; `android/src/lib.rs`
  exports `bongbong_on_create`, which forwards to it, and the manifest
  names it through `android.app.func_name`. `main` is exported as planned.
- **Assets, twice.** First the packaging copied the *contents* of
  `static/` into `assets/`, so `static/x.png` was not there (the smoke
  app's asset check caught it; `package.sh` keeps the directory now). Then
  the game's three ripple shaders failed: sola-raylib's `load_shader`
  checks the path with `std::fs` before calling raylib, and Rust's
  filesystem never sees APK assets. On Android `RippleFx::load` compiles
  the GLSL ES 100 sources in with `include_str!` and uses
  `load_shader_from_memory`; textures, which the wrapper hands straight to
  raylib, load from the APK through the `fopen` wrap as designed.
- **The emulator neither vsyncs nor is ES 2 only.** Its EGL swap returns
  at once (the smoke app ran at 500+ fps) and it reports OpenGL ES 3.0
  over Metal, while raylib's Android build still asks for ES 2. So the
  frame cap is 60 on the emulator and 0 on a phone, decided at runtime
  by `app::android::is_emulator` (`ro.kernel.qemu`), the Android twin of
  iOS's `cfg!(target_abi = "sim")`.
- **The command-line tools bootstrap** lands the current tools in
  `cmdline-tools/latest-2` beside the bootstrap copy; the setup script
  promotes them. The bootstrap's `avdmanager` did not know the Pixel 7
  profile, hence the promotion before the AVD is created.
- Everything else went as planned: `nobuild` + the prebuilt raylib (with
  `android_main` and `ANativeActivity_onCreate` in the archive), the
  `android/` workspace crate, the stderr-to-logcat pipe (it is what showed
  the shader panic), `EMBEDDED`/`KEYBOARD_AVAILABLE` covering Android (the
  RESTART button and the touch hints appear as on iOS), `InitWindow(0, 0)`
  giving native pixels with the manifest fixing landscape, and the
  `main.rs` to `src/app.rs` refactor, which left desktop, web and both
  iOS builds unchanged.

## 5. Phased plan

| Phase | Work | Estimate | Acceptance |
|---|---|---|---|
| 0 Toolchain | `env.sh`, `setup_android.sh` (cmdline-tools, NDK, image, AVD, cargo-ndk, prebuilt raylib), `devenv.nix` target | half a day, mostly downloads | `sdkmanager --list_installed` shows the pieces; `emulator -list-avds` shows `bongbong`; `libraylib.a` holds `android_main` and `ANativeActivity_onCreate`; the Rust sysroot has the target |
| 1 Gate 0 | `smoke.c`, `smoke.sh`, `package.sh`, `just android-smoke` | half a day | window on screen, raylib display size in logcat, a touch line after `adb shell input tap` |
| 2a Refactor | `src/app.rs`, `src/app/ios.rs`, thin `main.rs` | half a day | desktop, web, iOS simulator and device builds and `cargo test --lib` unchanged |
| 2b The game | constants, `run` branches, `android.rs`, `android/` crate, manifest, `build-android`, `run-android` | one day | logcat: `PLATFORM: ANDROID`, native display size, ES 2.0, all textures and three shaders, the screen-size line; screencap shows the round; swipe steers, tap fires, RESTART restarts; Home and back resumes; `FrameStats` at the display rate |
| 3 Polish | Back key, icon, immersive mode, phone frame time | one day | Back opens the leave dialog; icon on the launcher; 60 fps on a mid-range phone |
| 4 Device and store | `adb install` on a USB phone (developer mode only), AAB via bundletool, upload key, target 36, 16 KB check | later | a phone plays it; Play console accepts the bundle |

## 6. Risks, mitigations, stop conditions
1. **Hidden `ANativeActivity_onCreate`** in the cdylib (rustc's version
   script): the trampoline plus `android.app.func_name`. Checked at the
   first link, not at first launch.
2. **`--wrap=fopen` missing or ineffective**: assets fail silently and the
   round is a black screen with textures "loaded" of size 0. The smoke app
   loads one asset and the check is its logcat line.
3. **nix toolchain leakage** (`CC`, `LD`) into cc-rs or the link: the
   symptom is host objects in an Android link. cargo-ndk's target-specific
   variables and `env.sh` cover it, the same way `.cargo/config.toml` does
   for iOS.
4. **Emulator GPU quirks** with GL ES 2 under `-gpu host`: fall back to
   `swiftshader_indirect` for a run; a phone decides.
5. **The refactor collides with desktop work** in `main.rs`: sequence it
   after the view-cap change lands, verify every platform before 2b.
6. **Downloads and licences**: several GB, licences accepted by the script;
   the cmdline-tools version string drifts and is pinned in the script.
7. **Orientation**: raylib picks portrait from a `0 x 0` request for its
   own config copy, which nothing reads; the manifest's `sensorLandscape`
   rules. If the emulator comes up portrait anyway, the fix is the
   manifest, not the size.
Stop condition: if Gate 0 cannot show a raylib window on the emulator
within a day, the problem is the SDK or GPU path, not the game; fix that
before touching Rust.

## 7. Open questions
1. Minimum API 29, or lower for older phones (26 is AAudio's floor;
   raylib documents 29)?
2. SDK pieces through the script (headless, default) or once through
   Android Studio's SDK Manager?
3. Play account: the individual account used for iOS, or an organisation
   (phase 4 only)?

## 8. References
- raylib `rcore_android.c`, `CMakeLists.txt`, `cmake/LibraryConfigurations.cmake`, `src/Makefile` (Android section) in the vendored tree under `~/.cargo/registry/src/*/sola-raylib-sys-6.3.0/raylib`.
- raylib wiki: Working for Android, and the macOS page (the aapt/zipalign/apksigner packaging walk-through).
- Android NDK: other build systems (`darwin-x86_64` fat binaries, `--target=aarch64-linux-android<api>`), NativeActivity sample manifest, 16 KB page sizes, emulator acceleration on Apple Silicon.
- Rust platform support: `aarch64-linux-android` (tier 2, std).
- cargo-ndk 4.1.2 (the environment it sets); cargo-apk and xbuild are unmaintained.
- Google Play: target API 36 from 31 Aug 2026; AAB required.
- This repo: docs/ios-native-port-prd.md (the template), docs/fullscreen-resolution-research.md section 8 (Android layout notes), `tools/ios/` and `tools/setup_ios.sh` (the scripts to mirror).
