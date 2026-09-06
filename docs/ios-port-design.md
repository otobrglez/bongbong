# Shipping bongbong to iOS

Status: design / decision record, 2026-09. Nothing in the tree targets iOS
yet; this is the map of what stands between the current codebase and an
App Store build, what already helps, and the order to do it in.

## TL;DR

There are two realistic routes and one to reject:

| Route | What it is | Effort to first build on a device | Ships to the App Store? |
|---|---|---|---|
| **A. Web wrapper** | The existing `just build-web` wasm in a `WKWebView` inside a thin Xcode app | Days | Yes, with review risk (guideline 4.2 "minimum functionality") and a permanent 60 Hz / WebGL / memory ceiling |
| **B. Native (recommended)** | The Rust binary itself, raylib on its SDL backend with SDL3's iOS support, OpenGL ES 2, packaged by Xcode | 1-2 weeks to a playable TestFlight build | Yes, as a normal native game |
| C. Rewrite on macroquad/miniquad | Swap raylib for a Rust framework with iOS built in | Weeks; rewrites every drawing module | Rejected: the simulation would survive but `game.rs`, `tank.rs`, shaders and the whole asset pipeline would not |

Recommendation: do **B**. Use **A** only if a "see it on the phone this
weekend" demo is worth a throwaway.

The single hard fact driving this: **raylib 6.0 has no iOS platform**
(`sola-raylib-sys/raylib/src/platforms/` holds GLFW, RGFW, SDL, Win32,
Android, DRM, Web and Memory backends, nothing for UIKit), and GLFW, the
backend every current build uses, does not run on iOS at all. The only stock
raylib backend that does is `rcore_desktop_sdl.c` (`PLATFORM=SDL`), because
SDL3 itself has first-class iOS support (UIKit window, EAGL GL ES context,
multi-touch, app lifecycle). So the port is "raylib on SDL3 on iOS", not
"raylib on iOS".

## What the codebase already does right for this

These are not things to build; they are reasons the port is tractable.

- **Simulation and presentation are already separate.** `simulation/` never
  touches raylib; `main.rs` is the *only* file that reads a `RaylibHandle`
  for input and folds it into a plain `simulation::Input`. Touch controls
  are therefore a `main.rs`-side change, not a game change. The probe and
  every headless test keep working untouched.
- **The GLSL ES 100 shaders already exist.** iOS has no desktop OpenGL; it
  has OpenGL ES 2/3, which only accepts `#version 100`. `static/web/*.fs`
  are exactly that port (done for emscripten, which also runs GL ES 2).
  `main.rs`'s `shader_path` just needs `target_os = "ios"` added next to
  `emscripten`.
- **The scene already renders into a fixed 1280x720 offscreen target**
  (`scene_target` in `main.rs`, drawn by `Game::render`). That is the
  natural virtual resolution: keep simulating and drawing at 1280x720 and
  scale the final blit to whatever the phone's screen is (see "Screen"
  below). The map, nav grid and colliders are all sized from the width and
  height passed into `Game::init`/`update`, so pinning those to the virtual
  size instead of the live window size keeps every layout identical to
  desktop.
- **Every non-raylib dependency is pure Rust** (`rapier2d`, `hecs`, `rand`,
  `serde`, `toml`, `clap`) and compiles for `aarch64-apple-ios` as is.
  `clap` with an empty argv just yields the defaults.
- **The default map is embedded** (`include_str!("../maps/default.toml")`),
  so the bundle only needs `static/`.
- **No audio yet.** Nothing to do about `AVAudioSession` or the wasm-style
  `ASYNCIFY` question for now. When audio arrives, raylib's miniaudio
  backend already has `TARGET_OS_IPHONE` Core Audio paths.
- **Dev-only surfaces are already gated.** `devserver.rs` is
  `not(target_os = "emscripten")` and behind `dev-tools`; the editor is
  behind `map-editor`. An iOS build simply enables neither feature. (Widen
  the devserver cfg to `not(any(emscripten, ios))` only if a dev-tools iOS
  build is ever wanted.)

## The blockers, precisely

1. **The `sola-raylib-sys` build script cannot build for iOS.**
   `platform_from_target` maps any non-wasm/android/armv7 triple to
   `Desktop`, then `uname()` on the Mac says `Darwin`, so it links the
   *macOS* frameworks (`OpenGL`, `Cocoa`, `IOKit`) which do not exist in the
   iOS SDK, and it drives raylib's CMake with `PLATFORM=Desktop`, i.e. GLFW.
   Two ways out:
   - **No-fork path:** the crate's `nobuild` feature skips both CMake and
     the `link()` step (bindgen and the small raygui `cc` compile still
     run). We prebuild `libraylib.a` for iOS ourselves and supply the link
     line from `.cargo/config.toml`. Because this package uses the 2024
     edition (resolver 3, which carries resolver 2's target-scoped feature
     rule), the feature can be enabled for iOS only:
     ```toml
     [target.'cfg(target_os = "ios")'.dependencies]
     sola-raylib = { version = "6.3.0", features = ["nobuild"] }
     ```
     Native and web builds do not see the feature.
   - **Upstream path:** teach `../sola-raylib`'s build script an `Ios`
     platform (target contains `apple-ios`): CMake with
     `-DCMAKE_SYSTEM_NAME=iOS -DPLATFORM=SDL -DOPENGL_VERSION="ES 2.0"`,
     link `SDL3` plus the iOS frameworks, never `Cocoa`/`OpenGL`. Cleaner,
     and worth doing once the no-fork path has proven the exact flags.
2. **Graphics API.** Build raylib with `GRAPHICS_API_OPENGL_ES2`
   (`OPENGL_VERSION="ES 2.0"`; the SDL backend asks SDL for an
   `SDL_GL_CONTEXT_PROFILE_ES` 2.0 context). Everything the game draws
   (textures, render texture, three fragment shaders, additive blend for
   the barrel blast) is ES 2 territory. Apple deprecated OpenGL ES in iOS
   12 but still ships it; if it is ever removed, the fallback is ANGLE (GL
   ES on top of Metal, what Chrome and Unity use), and that is a bridge to
   cross if it happens, not now.
3. **Window/input backend.** raylib `PLATFORM=SDL` against **SDL3**
   (`rcore_desktop_sdl.c` handles both SDL2 and SDL3; pick SDL3, it is the
   maintained iOS path). The backend already turns `SDL_FINGERDOWN/UP/
   MOTION` into raylib's touch points and gestures, so
   `rl.get_touch_point_count()` / `get_touch_position(i)` (both exposed by
   `sola-raylib`'s `core/input.rs`) work with no glue.
4. **Entry point and main loop.** iOS apps must run through
   `UIApplicationMain`. With SDL3 the C side does that via `SDL_RunApp`,
   which calls a user-supplied main once UIKit is up; from Rust:
   ```rust
   #[cfg(target_os = "ios")]
   fn main() {
       unsafe extern "C" { fn SDL_RunApp(argc: c_int, argv: *mut *mut c_char,
           main: extern "C" fn(c_int, *mut *mut c_char) -> c_int, reserved: *mut c_void) -> c_int; }
       extern "C" fn app_main(_: c_int, _: *mut *mut c_char) -> c_int { run_game(Args::default()); 0 }
       unsafe { SDL_RunApp(0, std::ptr::null_mut(), app_main, std::ptr::null_mut()); }
   }
   ```
   Inside `app_main` the existing `sola_raylib::init()...build()` works
   (raylib's SDL backend calls `SDL_Init` and `SDL_CreateWindow` from
   `InitWindow`). A blocking `while` loop runs, but SDL documents
   `SDL_SetiOSAnimationCallback` as the proper loop on iOS: it ties frames
   to `CADisplayLink` and **stops calling back while the app is in the
   background**, which matters because raylib's SDL backend does not handle
   `SDL_EVENT_WILL_ENTER_BACKGROUND` and iOS kills an app that keeps
   issuing GL calls after backgrounding. The right home for that is a
   `target_os = "ios"` branch in `sola_raylib::core::game_loop::run`,
   exactly mirroring its emscripten branch (register the closure, return
   immediately); it needs the `SDL_Window*`, which the backend exposes
   through raylib's `GetWindowHandle()`.
5. **Assets are loaded relative to the working directory**
   (`"static/scifi_tanks_sheet.png"`). An iOS bundle is flat (executable and
   resources side by side inside `BongBong.app/`), so a
   `std::env::set_current_dir(current_exe().parent())` at the top of
   `app_main`, with `static/` added to Xcode as a folder reference (blue
   folder, keeps the directory), makes every existing path resolve. This
   is the same fix the Releases section of `CLAUDE.md` deferred for a
   PATH install on desktop; doing it iOS-only keeps `cargo run` unchanged.
6. **Screen: size, aspect, DPI, safe area.** An iPhone is ~19.5:9 and
   reports its size in points, not the 1280x720 the game assumes. Plan:
   - Simulate and draw at the fixed virtual 1280x720 (pass
     `DEFAULT_SCREEN_WIDTH/HEIGHT` into `init`/`update`, not the live
     window size), then blit `scene_target` with `draw_texture_pro` into
     a letterboxed destination rectangle (integer-ish scale, black bars
     on the sides). `Game::render` currently blits 1:1 with
     `draw_texture_rec`; this is the one presentation change the sim never
     sees. The HUD text, muzzle/impact ripple quads and the end-screen
     overlays use `get_screen_width()` directly today and need to draw in
     virtual space (i.e. into the target) or scale their coordinates.
   - The SDL backend does not support `FLAG_WINDOW_HIGHDPI`, so the GL
     drawable comes up at point resolution (non-Retina). For chunky pixel
     art that is fine and halves the fill cost; revisit only if the
     scaled sprites look soft.
   - Keep on-screen controls inside `SDL_GetWindowSafeArea` (notch, home
     indicator). Lock to landscape in `Info.plist`
     (`UISupportedInterfaceOrientations`), set `UIRequiresFullScreen`,
     hide the status bar, and call `SDL_DisableScreenSaver` so the idle
     timer never dims mid-round.
7. **Input: the game is keyboard-only.** Needed on iOS: a virtual d-pad
   (or swipe-to-face) for the four `Dir`s, a fire button, and small
   pause/restart buttons replacing P/R. All of it lives in the `main.rs`
   closure that builds `Input`, reading raylib touch points in *screen*
   space (the controls are screen-space UI; only the game view is
   scaled). `Game::update` needs nothing. Also drop the ESC exit key on
   iOS (`set_exit_key(KEY_NULL)`); there is no window to close.
8. **Packaging, signing, store.** Apple Developer Program membership
   ($99/yr), a bundle id, an Xcode project that owns signing, icons
   (`AppIcon` asset catalog), a launch screen storyboard, a
   `PrivacyInfo.xcprivacy` privacy manifest (required since 2024; SDL3
   ships one covering its own API use), TestFlight, then App Store Connect
   metadata, screenshots per device class, age rating, export-compliance
   answer. `cargo-dist` knows nothing about any of this and stays desktop
   only.
9. **CI needs macOS.** Every workflow today runs on Ubuntu. An iOS lane
   needs a `macos-15` runner with Xcode, and a signing identity plus
   provisioning profile imported from secrets for anything beyond a
   simulator smoke build.

## Route B, step by step

### Phase 0: prove the toolchain (on a Mac, no repo changes yet)

1. `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`.
2. Build SDL3 for iOS: the SDL repo's `Xcode/SDL/SDL.xcodeproj` produces
   `SDL3.xcframework` (device + simulator slices); or CMake with
   `-G Xcode -DCMAKE_SYSTEM_NAME=iOS`.
3. Build raylib for iOS from the raylib 6.0 tree vendored inside the
   `sola-raylib-sys` crate (`~/.cargo/registry/src/*/sola-raylib-sys-6.3.0/raylib`,
   or the same directory of the `../sola-raylib` checkout); it is the tree
   the crate's bindings were generated from, so do not substitute another
   raylib checkout:
   ```sh
   cmake -S <that raylib dir> -B build/raylib-ios -G Xcode \
     -DCMAKE_SYSTEM_NAME=iOS -DCMAKE_OSX_DEPLOYMENT_TARGET=15.0 \
     -DPLATFORM=SDL -DOPENGL_VERSION="ES 2.0" \
     -DSDL3_DIR=<path to SDL3 cmake config> \
     -DBUILD_EXAMPLES=OFF -DCUSTOMIZE_BUILD=ON -DSUPPORT_SCREEN_CAPTURE=OFF
   cmake --build build/raylib-ios --config Release -- -sdk iphoneos
   ```
   Output: `libraylib.a`. Repeat with `-sdk iphonesimulator` for the
   simulator slice.
4. Cargo side, in `.cargo/config.toml`:
   ```toml
   [target.aarch64-apple-ios]
   rustflags = [
     "-L", "native=ios/lib/iphoneos",
     "-l", "static=raylib", "-l", "static=SDL3",
     "-l", "framework=UIKit", "-l", "framework=OpenGLES", "-l", "framework=QuartzCore",
     "-l", "framework=CoreGraphics", "-l", "framework=Foundation",
     "-l", "framework=AVFoundation", "-l", "framework=GameController",
     "-l", "framework=CoreMotion", "-l", "framework=CoreHaptics",
     "-l", "framework=Metal", "-l", "framework=AudioToolbox",
   ]
   [env]
   BINDGEN_EXTRA_CLANG_ARGS_aarch64_apple_ios = "--sysroot=<xcrun --sdk iphoneos --show-sdk-path>"
   ```
   (the framework list is SDL3's; trim once the linker says what is
   unused.) bindgen picks up `TARGET` for its clang target but needs the
   iOS sysroot to find system headers, hence the env var. `cc` (the
   raygui wrapper) handles `*-apple-ios` on its own via `xcrun`.
5. `cargo build --target aarch64-apple-ios-sim` of the unchanged game
   should link (the `nobuild` feature from blocker 1 goes in here). It will
   not *run* correctly yet (no `SDL_RunApp`, wrong CWD), but a clean link
   proves every C-side decision.

### Phase 1: entry point, bundle, first frame on a device

1. Move the body of `main.rs`'s `main` (texture loads, shader loads,
   `Game` setup, the frame closure) into a library function, e.g.
   `src/app.rs::run_game(opts: AppOptions)`, keeping `main.rs` as the CLI
   parse plus a call. `Args` becomes the desktop way to fill `AppOptions`;
   iOS fills it with defaults (`show_intro`, shadows on, embedded map).
   This is the one refactor the port forces on the desktop code, and it is
   also what a future `bongbong-ios` staticlib crate would call.
2. Add the `#[cfg(target_os = "ios")]` entry from blocker 4 (`SDL_RunApp`
   → `app_main` → chdir to the bundle → `run_game`).
3. Xcode project under `ios/` (hand-written, or generated from a small
   `project.yml` with `xcodegen` so it is diffable): an App target whose
   executable is the Rust binary. Two workable shapes:
   - **Staticlib (standard):** a tiny `ios/bongbong-ios` crate with
     `crate-type = ["staticlib"]` depending on `bongbong` by path and
     exporting `extern "C" fn bongbong_main()`; the Xcode target has a
     one-line `main.m` calling it and links the `.a` via a Run Script
     build phase that runs `cargo build --target ...`. Xcode owns signing,
     `Info.plist`, icons, launch screen, and copies `static/` as a folder
     reference.
   - **Bare binary (dev only):** copy the `cargo build` executable into a
     hand-made `.app` with an `Info.plist` and `static/`, `codesign`, run
     on the simulator with `xcrun simctl install/launch`. Good for the
     first frame and for CI smoke tests; not how the store build is made.
4. Sanity checks on that first build: textures load (CWD fix), the three
   `static/web` shaders compile under ES 2, the render texture + shader
   passes blit, the intro banner shows, enemies drive. Expect to have to
   pin the virtual size (blocker 6) here already, since the map is sized
   from what `init` is told.

### Phase 2: make it playable on a phone

1. Virtual 1280x720 + letterboxed `draw_texture_pro` blit; move the HUD and
   overlay draws into virtual space (blocker 6).
2. Touch controls (blocker 7): d-pad on the left, fire on the right,
   pause/restart in a corner, all inside the safe area; a first-touch
   "tap to start" replaces the intro's key press if the intro needs one.
   Show the controls only when a touch has been seen, so a gamepad or
   keyboard (iPad) player never sees them.
3. `game_loop` iOS branch on `SDL_SetiOSAnimationCallback` (blocker 4) so
   backgrounding is safe; also pause the round on
   `SDL_EVENT_DID_ENTER_BACKGROUND` (feed `Input::pause_pressed`).
4. Performance pass on a real device: target 60 (the `120` passed to
   `game_loop::run` is fine on ProMotion, SDL vsyncs); watch fill rate of
   the full-screen ripple passes at point resolution.
5. Gamepad: SDL3 maps MFi/PS/Xbox controllers to raylib's gamepad API for
   free; wire `is_gamepad_button_down` into the same `Input` while here.

### Phase 3: store readiness

Icons, launch screen, orientation lock, `UIRequiresFullScreen`, hidden
status bar, idle timer, privacy manifest, `LSApplicationCategoryType`
games, an age rating, TestFlight internal testing, then external
TestFlight for a few weeks before submitting. Keep the tuning panel and
dev server out (no `dev-tools`), as they are on production web.

### Phase 4: CI

- `ios-smoke.yml` on `macos-15`: cache the prebuilt `libraylib.a`/SDL3
  slices (or rebuild them from the pinned sources with the commands in
  Phase 0), `cargo build --target aarch64-apple-ios-sim`, boot a simulator,
  install the bare-binary `.app`, launch, screenshot via `simctl io`. No
  signing needed.
- Release lane on version tags: import a distribution certificate and
  profile from secrets, `xcodebuild archive` + `-exportArchive`, upload with
  `xcrun altool`/`notarytool` or `fastlane pilot`. Mirror the
  cargo-dist tag trigger so one `git tag vX.Y.Z` fans out to desktop
  archives, the web deploy and a TestFlight build.

## Route A, if a quick demo is wanted first

Wrap `site/dist/` in a `WKWebView`:

- Serve the bundle through a `WKURLSchemeHandler` (or
  `loadFileURL(_:allowingReadAccessTo:)`); emscripten fetches
  `bongbong.data` by relative URL and `file://` fetches are blocked.
- Touch controls still have to be written, either as a JS overlay that
  synthesises the arrow/space key events the wasm listens for, or in Rust
  using raylib's touch API, which works on `PLATFORM_WEB` too (that half
  of the work carries over to Route B, so prefer it).
- Known ceilings: WKWebView caps at 60 Hz, no ProMotion; WebGL over Metal
  via ANGLE inside WebKit; a lower memory jetsam limit than a native
  process; audio later requires a user gesture to start.
- Review: a self-contained offline game in a web view is accepted in
  practice, but it is judged case by case under guideline 4.2, and it
  does not get the store's "game" treatment for controllers, Game Center
  and the like without native code anyway.

## Open questions to settle before Phase 1

- Fork or upstream `sola-raylib`'s build script for an iOS platform? The
  no-fork `nobuild` path works and is what Phase 0 proves; upstreaming an
  `Ios` platform (plus the `game_loop` iOS branch) is the long-term home
  for both pieces.
- Minimum iOS version: 15 covers every device that can run the game
  comfortably and keeps SDL3's requirements happy.
- iPad: same binary, larger letterbox or fill more of the screen? Fixed
  virtual size first; a wider virtual map is a design question for later.
- Audio, when it lands: plan for `AVAudioSession` category and
  interruption handling at the same time, not after.
