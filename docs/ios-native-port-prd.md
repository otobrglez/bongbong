# PRD: bongbong as a native iOS app (iPhone and iPad)

Status: decision document with research and test results, 2026-09-09.
Nothing in the tree targets iOS yet. Owner constraint: **a native app**, not
a PWA and not a web-view wrapper.

Related documents, all read and verified against source while writing this:

- `docs/ios-port-design.md` on branch `claude/ship-game-ios-aeu81j` (never
  merged): the native route, blockers and phases. This PRD adopts that route
  after testing its critical links, and supersedes it.
- `docs/mobile-and-scaling-design.md` on branch
  `claude/inventory-layout-map-builder-tcahk9`: the virtual-resolution fit,
  per-device layout table, safe area and touch-zone thinking. All of it
  carries over to the native app unchanged; its PWA and Safari parts do not.
- `docs/tap-navigation.md` (PR stacked on #5) and `docs/tap-orders-design.md`
  (same branch as above): the input model a phone needs.

## 1. The decision, in one paragraph

Port by building the existing Rust binary for `aarch64-apple-ios` on
raylib's **SDL backend with SDL3**, which has first-class iOS support, using
**OpenGL ES 2**, packaged and signed by a small Xcode project. It is feasible:
every link of the engine chain that can be exercised without an iOS SDK was
compiled and run today (section 4), and the whole game already runs on that
backend on macOS. It is not free: about **three to four weeks** of focused
work after tap navigation lands, plus Xcode, an Apple Developer membership
and a test device. Recommendation: **go**, starting with a one to two day
toolchain proof on this Mac as soon as Xcode is installed (Phase 0). If
Phase 0 cannot link an iOS binary, stop and reassess (section 9).

## 2. Goals and non-goals

Goals

- One universal app for iPhone and iPad, landscape only, on the App Store.
- Same game: same maps, rules, AI, tuning and art. The simulation is not
  touched, so the probe, the headless tests and seeded replays stay valid.
- Touch-first play: tap orders (tap to move, tap to attack) as the primary
  scheme, a fire control and pause/restart, plus game controllers for free.
- 60 fps on an iPhone 12-class device; ProMotion is a bonus, not a target.

Non-goals

- PWA or WKWebView wrapper (owner constraint). Noted only for completeness
  in section 5.
- Android. Cheap later: the bindings crate already builds raylib's Android
  platform through `cargo ndk`, and every screen and input change here is
  shared.
- The map editor on phones. The editor stays a desktop, dev-only feature.
- Audio. The game has none yet; section 9 says what to plan for when it
  arrives.
- A Metal renderer. OpenGL ES 2 is deprecated but present on iOS 26; the
  fallback if Apple removes it is ANGLE, not a rewrite.

## 3. What we have today (verified)

The game

- One crate, 116 dependencies in `Cargo.lock`; the **only native dependency
  is `sola-raylib-sys`**, which builds raylib 6.0 from the C tree vendored in
  the crate with CMake, bindgen and a small `cc` compile. `rapier2d`,
  `hecs`, `rand`, `serde`, `toml`, `clap` are pure Rust and already compile
  for iOS targets.
- The simulation touches raylib through one type: `lib.rs` aliases
  `Position = Vector2`, used at 56 sites in six files (`simulation/mod.rs`,
  `weapons.rs`, `waves.rs`, `combat.rs`, `debug.rs`, `ai.rs`). No `Color`,
  no `Rectangle`, no drawing trait below the presentation layer.
- `main.rs` is the only file that reads input: six keys (arrows, Space, P,
  R, L, I) folded into a plain `simulation::Input`. No mouse, gamepad or
  touch on the play path (the editor reads the mouse, dev-only).
- The scene renders into a fixed `1280x720` render texture and is blitted
  1:1 with `draw_texture_rec`; the window is created at `1280x720` with no
  resizable, vsync, fullscreen or high-DPI flags. About 20 HUD and overlay
  draw sites in `game.rs` read the live screen size directly.
- GLSL ES 100 ports of all three post-processing shaders already exist in
  `static/web/` (used by the emscripten build). `main.rs::shader_path`
  picks them by `cfg(target_os)`.
- Assets load by CWD-relative path (`static/...`); the default map is
  compiled in with `include_str!`. No audio. No threads, sockets or file
  writes outside the `dev-tools` feature (the map builder ships in every
  build, but its one file write - the dev `SAVE` - is `dev-tools`-only).
- `sola_raylib::game_loop::run` has two branches: a blocking native loop
  and an emscripten callback loop. Nothing for iOS.

raylib 6.0 (the tree the bindings were generated from, in the cargo
registry copy of `sola-raylib-sys` 6.3.0; the checkout at `../raylib` is
6.1-dev and must not be used for iOS builds)

- Platforms: GLFW, RGFW, SDL, Win32, Android, DRM, Web, Memory. **No iOS
  platform, no UIKit or EAGL code anywhere in first-party source.** GLFW,
  the backend every current build uses, does not exist on iOS.
- `platforms/rcore_desktop_sdl.c` (2250 lines) supports SDL2 and SDL3, maps
  `SDL_FINGERDOWN/UP/MOTION` into raylib's touch points and gestures,
  requests an OpenGL ES context when built with `GRAPHICS_API_OPENGL_ES2`,
  and exposes the `SDL_Window` through `GetWindowHandle()`. It does **not**
  support `FLAG_WINDOW_HIGHDPI` and handles **no** app lifecycle events
  (background/foreground).
- CMake: `PLATFORM=SDL` links only `SDL3::SDL3` (or SDL2); the macOS-only
  frameworks (`Cocoa`, `IOKit`, `OpenGL`) are added only in the GLFW and
  RGFW branches, so an iOS cross-compile is not blocked by raylib's build
  files. `OPENGL_VERSION` accepts `ES 2.0` and `ES 3.0`.
- Upstream, pull request raysan5/raylib#5881 (open since May 2026, last
  activity July 2026) adds a UIKit/EAGL `rcore_ios.c`. Audio does not work,
  keyboard input is missing, the game loop runs on a shared dispatch queue,
  and the maintainer's stated concern is long-term maintenance. Not
  something to depend on.

sola-raylib (the wrapper this game uses, developed alongside it at
`../sola-raylib`)

- `build.rs` maps targets to four platforms (Web, Desktop, Android, RPI);
  `aarch64-apple-ios` falls into Desktop, which drives CMake with
  `PLATFORM=Desktop` (GLFW) and links `OpenGL`, `Cocoa`, `IOKit`. **An iOS
  build fails there today.**
- Escape hatches already exist: the `nobuild` feature skips CMake and the
  link step (bindgen and the raygui shim still run), and there are `sdl`,
  `opengl_es_20` and `opengl_es_30` features.
- The touch and gesture API is fully exposed (`get_touch_point_count`,
  `get_touch_position`, `get_gesture_detected`, drag and pinch vectors),
  as are `set_exit_key`, `set_target_fps`, `get_render_width` and the
  config-flag builder methods. No safe-area or orientation helper (raylib
  has none either); no `get_window_handle` wrapper (the raw `ffi` call is
  available).

This Mac (where the port would be built)

- macOS 15.6.1. **No Xcode**: `xcode-select -p` points at the nix
  `apple-sdk-14.4`, `xcodebuild` is missing, `xcrun --sdk iphoneos` finds no
  SDK, and there are no simulator runtimes.
- Rust 1.97.1 comes from nix through devenv, with **no rustup** and only
  the `wasm32-unknown-emscripten` extra target. The iOS standard library is
  not installed, so `cargo build --target aarch64-apple-ios` cannot even
  start.
- cmake 4.3.4 is present; the nixpkgs pin has no `sdl3` package (SDL2 is
  there). SDL3 was built from source for the tests below.

## 4. Tests run on 2026-09-09 and what they prove

| # | Test | Result | Proves |
|---|---|---|---|
| T1 | Toolchain inventory (`xcode-select`, `xcrun --sdk iphoneos`, `simctl`, `rustc --print sysroot` target dirs) | No Xcode, no iOS SDK, no simulators, no iOS Rust std, no rustup | Phase 0 cannot run on this machine until Xcode is installed and the nix toolchain gains the iOS targets |
| T2 | Dependency audit (`cargo tree`, `Cargo.lock`) | 116 crates, one native (`sola-raylib-sys`) | The only cross-compilation problem is raylib itself |
| T3 | Coupling audit (grep of `sola_raylib` users, `Vector2` sites, input and screen-size sites) | 26 of 41 source files import raylib; the simulation uses only `Vector2` at 56 sites; input is 6 keys in one file; ~20 HUD sites use live screen size; blit is 1:1 | The simulation needs no change; the port's game-side work is confined to `main.rs`, `game.rs` and a new `app.rs` |
| T4 | raylib 6.0 SDL backend and CMake audit (registry tree) | Touch mapping, ES profile request and window handle present; no high-DPI, no lifecycle events; SDL branch links no macOS framework; `OPENGL_VERSION="ES 2.0"` exists | The SDL route has no hidden blocker in raylib's own source or build files |
| T5 | Build SDL3 (release-3.2.x, static) from source, then raylib 6.0 with `-DPLATFORM=SDL -DSDL3_DIR=...` for `OPENGL_VERSION=3.3` and for `ES 2.0` | Both build cleanly on macOS; `libraylib.a` 2.2 MB (3.3) and 2.0 MB (ES 2.0, compile only) | The exact vendored raylib compiles against SDL3 with the ES 2 renderer selected |
| T6 | Link and run the whole game against the SDL3-backed raylib: `cargo build --features sola-raylib/nobuild,dev-tools` with the link line in `RUSTFLAGS`, separate target dir | Links (1763 SDL symbols, 0 GLFW); runs with `--dev-port 4748`; a 240-frame round with firing renders every layer (ground, grass, trees, tanks, rings, plasma bursts, frog, HUD) and emits fired/hit/obstacle-destroyed events | The game's rendering, render texture, shader passes and dev server all work on raylib's SDL3 backend, the backend iOS uses; the `nobuild` link path works as designed |
| T7 | Research (sources in section 11) | Rust iOS targets are Tier 2 with std; OpenGL ES still works on iOS 26.x with no announced removal; SDL3 documents its iOS entry point, animation callback and lifecycle events; Apple guideline 4.7 is about non-embedded software and does not apply to a native game; Xcode 26 needs macOS 15.6+, Xcode 26.4 needs macOS 26.2; GitHub `macos-26` runners with Xcode 26 are generally available | The route has no platform-policy blocker and CI is possible on hosted runners |

Not tested, because it needs Xcode and a device: the cross-compile and link
for `aarch64-apple-ios` and `-sim`, a simulator boot, a device run, OpenGL
ES 2 at runtime on iOS, touch mapping, performance and thermals,
backgrounding. These are exactly Phase 0 and Phase 1.

Reproducing T5 and T6 (paths under the session scratch directory are
disposable; only the commands matter):

```sh
# SDL3, static, from source
git clone --depth 1 --branch release-3.2.x https://github.com/libsdl-org/SDL.git sdl3-src
cmake -S sdl3-src -B sdl3-build -DSDL_SHARED=OFF -DSDL_STATIC=ON -DSDL_TEST_LIBRARY=OFF \
      -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=$PWD/sdl3-install
cmake --build sdl3-build -j8 && cmake --install sdl3-build

# raylib 6.0 from the crate's vendored tree, SDL backend, ES 2 renderer
RL=$(ls -d ~/.cargo/registry/src/*/sola-raylib-sys-6.3.0/raylib)
cmake -S $RL -B rl-sdl3 -DPLATFORM=SDL -DOPENGL_VERSION="ES 2.0" -DBUILD_EXAMPLES=OFF \
      -DCMAKE_BUILD_TYPE=Release -DSDL3_DIR=$PWD/sdl3-install/lib/cmake/SDL3
cmake --build rl-sdl3 -j8

# the game against it (macOS, desktop GL 3.3 build of raylib for running it)
FW=(CoreMedia CoreVideo Cocoa UniformTypeIdentifiers IOKit ForceFeedback Carbon CoreAudio \
    AudioToolbox AVFoundation Foundation GameController Metal QuartzCore CoreHaptics OpenGL)
FLAGS="-L native=$PWD/rl-sdl3/raylib -l static=raylib -L native=$PWD/sdl3-install/lib -l static=SDL3"
for f in "${FW[@]}"; do FLAGS="$FLAGS -l framework=$f"; done
RUSTFLAGS="$FLAGS" CARGO_TARGET_DIR=target-sdl cargo build --features sola-raylib/nobuild,dev-tools --bin bongbong
target-sdl/debug/bongbong --dev-port 4748     # then: BONGBONG_DEV_PORT=4748 just mcp-call screenshot
```

## 5. Options considered

| Route | What it is | Verdict |
|---|---|---|
| **B. raylib on SDL3, native** | The Rust binary for `aarch64-apple-ios`, raylib built with `PLATFORM=SDL` and `GRAPHICS_API_OPENGL_ES2` against SDL3's iOS support, an Xcode project for signing and the bundle | **Recommended.** Every part that can be tested without an iOS SDK passed today (T4 to T6). Native performance, real touch and controllers, normal App Store review |
| B'. raylib's upstream iOS platform (PR #5881) | Vendor the patched raylib with `rcore_ios.c` (UIKit + EAGL) | Not now. Unmerged, no audio, no keyboard, threading concerns, maintainer undecided. Watch it; if it lands in a raylib release the SDL layer could be dropped later |
| C. Rewrite on macroquad/miniquad | Rust framework with built-in iOS support (an `.app` is "a folder with the binary, `Info.plist` and assets") | Rejected. The simulation would survive, but 16 rendering files (71 raylib type sites), three shader passes, the ripple pipeline, the editor and the sprite pipeline conventions would all be rewritten. Weeks of work to reach today's look |
| A. WKWebView wrapper of the wasm build | Thin Xcode app loading `site/dist/` | Excluded by the owner constraint. For the record: capped at 60 Hz, judged under guideline 4.2 "minimum functionality", lower memory ceiling |
| PWA | Home-screen web app | Excluded by the owner constraint. Its scaling and touch-layout work is reused by route B anyway |

Graphics API fallback for route B: if Apple ever removes OpenGL ES, ANGLE
(OpenGL ES on Metal, what Chrome and Unity use on Apple platforms) slots
under SDL3 with no game code change. Not needed today.

## 6. Design: what changes, by area

6.1 Build chain

- **Bindings crate.** Add an `Ios` platform to `sola-raylib-sys/build.rs`
  (target contains `apple-ios`): CMake with `-DCMAKE_SYSTEM_NAME=iOS
  -DCMAKE_OSX_DEPLOYMENT_TARGET=15.0 -DPLATFORM=SDL -DOPENGL_VERSION="ES 2.0"
  -DSDL3_DIR=...`, bindgen with the iOS sysroot
  (`BINDGEN_EXTRA_CLANG_ARGS_aarch64_apple_ios=--sysroot=$(xcrun --sdk
  iphoneos --show-sdk-path)`), and a link line of `SDL3` plus the iOS
  frameworks SDL3 declares (UIKit, OpenGLES, QuartzCore, CoreGraphics,
  Foundation, AVFoundation, GameController, CoreMotion, CoreHaptics, Metal,
  AudioToolbox, CoreMedia, CoreVideo, UniformTypeIdentifiers); never
  `Cocoa`, `IOKit` or `OpenGL`. The bindings crate lives next to the game,
  so this is the clean home for it.
- **Proof-of-concept path before that**, proven on macOS today (T6): the
  `nobuild` feature enabled only for iOS through resolver 3's
  target-scoped dependency features:
  ```toml
  [target.'cfg(target_os = "ios")'.dependencies]
  sola-raylib = { version = "6.3.0", features = ["nobuild"] }
  ```
  with the prebuilt `libraylib.a` and `libSDL3.a` and the framework list in
  `.cargo/config.toml` under `[target.aarch64-apple-ios]` and
  `[target.aarch64-apple-ios-sim]`.
- **SDL3 for iOS**: build the `SDL3.xcframework` (device + simulator slices)
  from SDL's `Xcode/SDL/SDL.xcodeproj`, or CMake with `-G Xcode
  -DCMAKE_SYSTEM_NAME=iOS`, from the same `release-3.2.x` line tested in T5.
- **Toolchain in devenv**: no rustup here, so the targets go into
  `devenv.nix`: `languages.rust.targets = [ "wasm32-unknown-emscripten"
  "aarch64-apple-ios" "aarch64-apple-ios-sim" ]`. Pin
  `CC_aarch64_apple_ios`/`CXX_...`/`AR_...` to `xcrun clang`/`ar` in
  `.cargo/config.toml`'s `[env]`, the same fix the emscripten target needed
  against nix's wrapped host clang. Xcode itself is installed outside nix.

6.2 Entry point and main loop

- Move the body of `main.rs::main` (window, textures, shaders, `Game`
  setup, the per-frame closure) into `src/app.rs::run_game(AppOptions)`.
  The CLI `Args` fills `AppOptions` on desktop; iOS fills defaults (intro
  on, shadows on, embedded map). This is the one refactor the port forces
  on desktop code, and a `bongbong-ios` staticlib crate would call it too.
- iOS `main`: `SDL_RunApp` → `app_main` → chdir to the bundle → `run_game`
  (the snippet in the earlier design doc is correct for SDL3).
- Add an iOS branch to `sola_raylib::game_loop::run` mirroring the
  emscripten one: register the frame closure with
  `SDL_SetiOSAnimationCallback` on the window from `ffi::GetWindowHandle()`
  and return, so frames pace on `CADisplayLink` and stop while backgrounded
  (iOS kills an app that issues GL calls in the background).
- raylib's SDL backend drops the lifecycle events, so install an SDL event
  watch in that branch: on `SDL_EVENT_DID_ENTER_BACKGROUND` feed
  `Input::pause_pressed` so the round pauses; resume on foreground.
- `rl.set_exit_key(None)` on iOS; there is no window to close.

6.3 Assets

- `std::env::set_current_dir(current_exe().parent())` at the top of
  `app_main`; add `static/` to the Xcode target as a folder reference (blue
  folder) so every `static/...` path resolves unchanged.
- `shader_path`: `#[cfg(any(target_os = "emscripten", target_os = "ios"))]`
  → `static/web/`, the ES 100 ports.

6.4 Screen

- Simulate and draw at the virtual `1280x720` (pass `DEFAULT_SCREEN_WIDTH/
  HEIGHT` into `Game::init`/`update`, not the live size), then blit
  `scene_target` with `draw_texture_pro` into the field rectangle from
  `Layout::fit(screen_w, screen_h)` (the mechanism in the mobile-scaling
  doc: largest 16:9 rectangle, scale snapped to 0.5 when within 5 % so 2 px
  art blocks land on device pixels, black bars elsewhere). Ripple UVs and
  camera shake gain one multiply by the scale.
- Move the ~20 HUD and overlay draw sites in `game.rs` into virtual space
  (draw into the target) or scale their coordinates; touch controls draw in
  screen space, sized in points (44 pt minimum targets).
- The SDL backend has no high-DPI flag, so the GL drawable comes up at
  point resolution. For 2 px-block pixel art this is acceptable and halves
  fill cost; revisit only if sprites look soft on a device.
- Safe area from `SDL_GetWindowSafeArea` (a small `extern "C"` declaration
  or a wrapper in the bindings crate); landscape lock via
  `UISupportedInterfaceOrientations`; `UIRequiresFullScreen`; status bar
  hidden; `SDL_DisableScreenSaver` so the idle timer never dims a round.
- iPad: same binary; at 4:3 the fit leaves a band below the field for the
  HUD bar and controls (the scaling doc's table). Whether to fill more of
  the screen with a wider field is an open question (section 10).

6.5 Input

- Primary: **tap orders** from `docs/tap-navigation.md` (tap to move via
  pathfinding, tap an enemy to attack), which need no reserved zones. This
  work is already stacked on PR #5 and is a prerequisite.
- Plus: a hold-to-fire control (raw held state maps straight to
  `Intent::fire`, exactly like Space), pause and restart buttons in the
  safe area, all read from raylib's touch API in `main.rs`'s input block;
  a virtual stick is an optional fallback in the leftover columns.
- Show the controls only after a touch has been seen, so an iPad with a
  keyboard or a controller never sees them. Game controllers arrive through
  SDL3 and raylib's gamepad API with no extra work.

6.6 Performance

- Halve `fx_density` on iOS the way the emscripten build does
  (`cfg!(target_os = "ios")` next to the existing emscripten line).
- Target 60 fps; keep the loop's 120 request for ProMotion devices.
- Measure with the dev overlay's frame-time label on a real device. The
  suspects are the full-screen ripple passes at point resolution.

6.7 Packaging and store

- `ios/` directory: a small `bongbong-ios` staticlib crate (`crate-type =
  ["staticlib"]`, depends on `bongbong` by path, exports `extern "C" fn
  bongbong_main()`), an Xcode project generated from a `project.yml` with
  `xcodegen` so it is diffable, a one-line `main.m`, a Run Script phase that
  runs `cargo build --target ...`, `Info.plist`, `AppIcon` catalog, launch
  screen storyboard, `PrivacyInfo.xcprivacy` (SDL3 ships one for its own
  API use), `LSApplicationCategoryType` games.
- Build without `dev-tools`, as production web does.
- TestFlight internal, then external for a few weeks, then App Store
  Connect metadata, screenshots per device class, age rating, export
  compliance.

6.8 CI

- `ios-smoke.yml` on `macos-26`: cache or rebuild the SDL3 and raylib iOS
  slices, `cargo build --target aarch64-apple-ios-sim`, boot a simulator,
  install the bare-binary `.app`, launch, screenshot with `simctl io`. No
  signing needed.
- Release lane on version tags next to cargo-dist and the web deploy:
  import a distribution certificate and profile from secrets,
  `xcodebuild archive` and `-exportArchive`, upload with `notarytool` or
  fastlane.

6.9 What does not change

`simulation/`, `ai.rs`, `pathfind.rs`, maps, the probe, determinism, the
desktop and web builds. The iOS branch of every change above is behind
`cfg(target_os = "ios")` or lives in `ios/` and the bindings crate.

## 7. Phased plan

| Phase | Work | Estimate | Acceptance |
|---|---|---|---|
| 0. Toolchain proof (this Mac) | Install Xcode 26 and a simulator runtime; add the iOS targets to `devenv.nix`; build SDL3 xcframework; build raylib for `iphoneos` and `iphonesimulator` with the T5 flags plus `-DCMAKE_SYSTEM_NAME=iOS`; `cargo build --target aarch64-apple-ios-sim` with the `nobuild` link line from T6 | 1 to 2 days | An unchanged game binary links for both slices |
| 1. First frame | `app.rs` refactor; `SDL_RunApp` entry; bundle chdir; `xcodegen` project; run on the simulator | 3 to 5 days | `simctl io` screenshot shows the intro banner and enemies driving; textures and the three ES 2 shaders load |
| 2. Playable | Virtual resolution and letterbox; HUD in virtual space; touch controls on top of tap orders; iOS animation callback and background pause; performance pass; controllers | About 1 week | A full round on an iPhone and on an iPad by touch alone; no crash on background/foreground; controls inside the safe area; 60 fps on an iPhone 12-class device |
| 3. Store readiness | Icons, launch screen, orientation lock, privacy manifest, age rating, TestFlight internal then external | About 1 week of calendar time | Three external testers install and play from TestFlight |
| 4. CI | Simulator smoke on `macos-26`; release lane on tags | 1 to 2 days | Every PR produces a simulator screenshot; a tag produces a TestFlight build |

Total: three to four weeks of focused work by one person, after the tap
navigation PR lands. The earlier design doc's "1 to 2 weeks to TestFlight"
assumed a rustup toolchain and no HUD refactor; this estimate includes the
nix toolchain work, the HUD move to virtual space and the touch controls.

## 8. Costs and prerequisites checklist

- [ ] Xcode 26 from the Mac App Store (roughly 2 GB download, about 9 GB on
      disk, plus an iOS simulator runtime of several GB). macOS 15.6.1 runs
      Xcode 26.0 to 26.3; Xcode 26.4 requires macOS 26.2.
- [ ] Apple Developer Program, USD 99 per year (individual or organisation;
      decide which, it sets the seller name on the store).
- [ ] A physical iPhone and, ideally, an iPad. OpenGL ES, touch, thermals
      and backgrounding cannot be judged on the simulator.
- [ ] `devenv.nix` gains the two iOS targets; SDL3 is built from source
      (not in the nixpkgs pin).
- [ ] The tap navigation PR merged, or at least its order model usable.

## 9. Risks, mitigations and stop conditions

- **OpenGL ES deprecation.** Deprecated since iOS 12, still working on iOS
  26.x per developer reports, no removal announced. Mitigation: ANGLE under
  SDL3 if it ever goes; nothing in the game would change.
- **raylib's SDL backend is "not tested" by raylib on anything but Windows
  and Linux.** T5 and T6 show it compiles with SDL3 and runs the whole game
  on macOS; iOS itself is proven only in Phase 0/1. Stop condition: if
  Phase 1 cannot get a frame on the simulator within its budget, fall back
  to evaluating PR #5881's platform (B') before spending more.
- **Backgrounding.** raylib's SDL backend ignores lifecycle events; without
  the animation callback and the event watch the app is killed on the first
  home-button press. Built into Phase 2, verified by the acceptance test.
- **Point resolution (no high-DPI on the SDL backend).** Acceptable for the
  art style; if not, the fix is a raylib patch to request a high-DPI SDL
  window, which the vendored tree simply warns about today.
- **Fill rate on old devices.** Full-screen ripple passes; mitigations are
  the `fx_density` knob and skipping ripple passes on iOS.
- **Toolchain friction under nix.** The wrapped host clang already broke
  emscripten once; expect to pin `CC_aarch64_apple_ios` and possibly run
  `cargo` from an Xcode-aware shell. Budgeted in Phase 0.
- **App Store review.** A native offline game is the ordinary case;
  guideline 4.7 (mini apps not embedded in the binary) does not apply.
  The privacy manifest and export-compliance answers are required.
- **Audio later.** raylib's miniaudio has a Core Audio backend with iOS
  session handling; on iOS the audio module must be compiled as
  Objective-C (`-x objective-c`) because that backend uses
  `AVAudioSession`. Plan it together with `AVAudioSession` category and
  interruption handling when audio arrives.
- **Two engines on one screen.** A stray `get_screen_width()` left in
  screen space after the virtual-resolution change shows up as misplaced
  HUD text on the first device run. The ~20 sites are listed in T3; the
  dev overlay makes them visible.

## 10. Open questions for the owner

1. Minimum iOS version: 15 (covers every device that can run the game
   comfortably; SDL3 deploys to iOS 11+) or 17 (fewer test permutations)?
2. iPad layout: keep the 16:9 field and put the HUD bar and controls in the
   band below, or design a wider field for 4:3 later?
3. Store account: individual or organisation? Free app or paid?
4. Upstream the `Ios` platform into `sola-raylib` in the same release line
   (6.x), or keep the `nobuild` link line in this repo until iOS ships?
5. Should Phase 0 start now on this Mac (needs Xcode installed by you), or
   on a CI `macos-26` runner first?

## 11. References

- Earlier design docs: `git show f750348:docs/ios-port-design.md`,
  `git show 2ace0ac:docs/mobile-and-scaling-design.md`,
  `git show ad1ce15:docs/tap-orders-design.md`.
- raylib iOS pull request: https://github.com/raysan5/raylib/pull/5881
- raylib community game published on iOS (April 2026):
  https://x.com/raysan5/status/2040723519986614702
- SDL3 iOS notes: https://wiki.libsdl.org/SDL3/README-ios
- Rust platform support (iOS targets, Tier 2):
  https://doc.rust-lang.org/nightly/rustc/platform-support.html
- Apple App Review Guidelines (4.2, 4.7):
  https://developer.apple.com/app-store/review/guidelines/
- Apple, migrating OpenGL to Metal (deprecation context):
  https://developer.apple.com/documentation/Metal/migrating-opengl-code-to-metal
- OpenGL ES still working on iOS 26 (developer forum thread via
  https://developer.apple.com/forums/tags/opengl)
- WKWebView 60 Hz cap (for the excluded route):
  https://developer.apple.com/forums/thread/773222
- Xcode 26 release notes and requirements:
  https://developer.apple.com/documentation/xcode-release-notes/xcode-26-release-notes
- GitHub `macos-26` runners: https://github.blog/changelog/2026-02-26-macos-26-is-now-generally-available-for-github-hosted-runners/
- macroquad on iOS (the rejected rewrite route): https://macroquad.rs/articles/ios/
