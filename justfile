# Native dev loop: watch, rebuild, and run on source changes.
watch:
    cargo watch -x "run"

# Standing gameplay-health sweep on the shipped default map: 30 fresh
# seeded rounds (the base seed prints in the header, so any flagged round
# is replayable) plus per-kind anomaly heatmaps. See
# docs/gameplay-verification-design.md and CLAUDE.md's probe bullets.
# Pinned to a Protect band round: the shipped map's own level tables may
# say otherwise (the probe refuses --enemies under a waves plan).
probe-sweep:
    cargo run --bin probe -- --scenario afk --mission protect --spawn band --enemies 4 --frames 1800 --rounds 30 --heatmap

# Waves spawn plan health check: the maps/missions/ waves fixture, Destroy
# mission (no frog, so an AFK player only loses to gunfire), 30 seeded
# rounds. Rolling-in tanks are exempt from the anomaly checks until they
# arrive. See docs/maps-to-levels.md.
probe-waves:
    cargo run --bin probe -- --map maps/missions/waves-basic.toml --scenario afk --frames 3600 --rounds 30 --seed 2000 --heatmap

# Sweep every maps/test/ adversarial fixture at a pinned seed and hold it
# to the recorded baseline: each ceiling is the observed maximum across all
# six fixtures - deterministic under the pinned seed, so any exceedance is
# a real behavior change, not noise. After a deliberate AI/map/tuning
# change shifts the numbers: rerun, read the new totals, and re-baseline
# consciously - never bump a ceiling just to go green. Zero-ceilings
# (stale-start, stall, wall-grind, bump-rate, low-progress, never-arrived,
# invariant, tank-grind, pile-up) are kinds no fixture currently produces at
# all.
# tank-grind and pile-up joined that list 2026-09-14 with the enemy command
# & control instrumentation (docs/enemy-command-and-control-prd.md). They are
# the first two kinds that measure a tank against *another tank* rather than
# against the map, and both read 0 across the corpus and the default map at
# the time they were added - so they are regression insurance, not a
# currently-failing gate. The number that work actually has to move is the
# ram tally the sweep now prints beside the totals, which is not budgeted:
# a ram is not a failure, and once C2 can order one, budgeting it would
# budget the feature.
# Re-measured 2026-09-04, twice. First after the Protect mission's hunter
# roll (`enemy_hunter_share_protect`, one RNG draw per enemy in
# `Game::init`) shifted every stream: with the share zeroed the previous
# totals came back exactly, so those differences were the stream, not the
# hunters. Then after hunters learned to shoot through destructible walls
# (line of fire), got a snipe cooldown and a ring slot even when alone:
# jitter 2 -> 6 and churn 7 -> 10. frog-block carries the jitter=6 (and
# churn=3, clustering=4) and every one of them is a hunter - the totals are
# 0/0/4 with the share zeroed; traced frame by frame they are the normal
# diagonal-slot approach (right/down legs each held the full commitment
# hold) plus the turn to snipe the player and back, not a stall or a
# spin. maze holds churn=10 and clustering=8 at this seed; border-stuck=1
# is the maze at seed 0x3ea and pockets at 0x3eb (plain player-role
# tanks: one wedging at the map corner for ~1.5 s while routing around
# the maze's edge, one holding an aligned firing line on the player 26 px
# from the bottom wall). See docs/gameplay-verification-design.md.
probe-fixtures:
    for m in maps/test/*.toml; do cargo run --bin probe -- --map $m --frames 1800 --rounds 10 --seed 1000 --budget stale-start=0 --budget stall=0 --budget border-stuck=1 --budget jitter=6 --budget spin=1 --budget churn=10 --budget clustering=9 --budget wall-grind=0 --budget bump-rate=0 --budget low-progress=0 --budget never-arrived=0 --budget invariant=0 --budget tank-grind=0 --budget pile-up=0 || exit 1; done

run:
    cargo run

# Map thumbnails (docs/mapshot-prd.md): every map under maps/ rendered to a
# field-only PNG of its first frame, into target/thumbnails/ with the same
# relative paths. The CPU renderer needs no window, so this also runs on a
# server with no display; `mapshot --help` lists the knobs (--scale, --seed,
# --players, --tank, --no-tanks, --plain).
thumbnails:
    cargo run --bin mapshot -- --out-dir target/thumbnails maps

# The same batch through the game's own renderer in a hidden window - the
# cross-check that the CPU output is what the game draws.
thumbnails-gpu:
    cargo run --bin mapshot -- --renderer gpu --out-dir target/thumbnails-gpu maps

# GPU against CPU on the shipped maps: renders each both ways and prints the
# mean channel difference and the share of pixels off; exits 1 beyond the
# tolerance in thumbnail.rs. Opens a hidden window, hence a recipe rather
# than a `cargo test` case (macOS creates windows on the main thread only).
mapshot-compare:
    cargo run --bin mapshot -- --check --out-dir target/thumbnails-check maps/default.toml maps/portals.toml maps/missions maps/test/props.toml

# Palette guards on the generated sheets: every opaque pixel on the Puny
# Palette, and no green on anything drawn over the ground layer (walls,
# props, blasts) - see tools/check_sheets.py and docs/PALETTE.md.
check-sheets:
    nix-shell -p "python3.withPackages (ps: [ps.pillow])" --run "python3 tools/check_sheets.py"

# One-time: install the emsdk toolchain (pinned version, see
# tools/setup_emscripten.sh) and the site/'s JS dependencies (Astro).
# The wasm32-unknown-emscripten rustup target and node/yarn are already
# provided by devenv.nix (languages.rust.targets, languages.javascript).
setup-web:
    ./tools/setup_emscripten.sh
    cd site && yarn install

# Build the release wasm binary, stage bongbong.{wasm,js,data} into
# site/public/game/ (gitignored - see site/README.md), then build the Astro
# site (site/dist/) around it. Auto-sources emsdk_env.sh from
# ~/.local/share/emsdk if emcc is not on PATH.
build-web: (_build-web "")

# Same as build-web but with the `dev-tools` cargo feature: the wasm exports
# the tuning C API (src/capi.rs) and the page's tuning panel appears under
# the canvas (docs/runtime-tuning-design.md). This is what PR previews ship;
# production (cloudflare-deploy.yml) uses plain build-web.
build-web-dev: (_build-web "--features dev-tools")

_build-web features:
    bash -c 'set -e; \
        command -v emcc >/dev/null 2>&1 \
            || source ~/.local/share/emsdk/emsdk_env.sh >/dev/null 2>&1 \
            || { echo "[build-web] emcc not on PATH and no emsdk at ~/.local/share/emsdk/. Run just setup-web first." >&2; exit 1; }; \
        cargo build --release --target wasm32-unknown-emscripten {{features}}'
    mkdir -p site/public/game
    rm -f site/public/game/*
    cp target/wasm32-unknown-emscripten/release/bongbong.wasm site/public/game/
    cp target/wasm32-unknown-emscripten/release/bongbong.js site/public/game/
    cp target/wasm32-unknown-emscripten/release/deps/bongbong.data site/public/game/
    cd site && yarn build
    @echo "[build-web] site/dist/ ready. Run 'just serve-web' to open."

# Build (see build-web) then preview site/dist/ at http://localhost:4321.
serve-web: build-web
    cd site && yarn preview --port ${PORT:=4321}

# Dev-tools build (see build-web-dev) then preview it at http://localhost:4321.
serve-web-dev: build-web-dev
    cd site && yarn preview --port ${PORT:=4321}

# Preview whatever site/dist/ currently holds, without rebuilding - so a
# `just build-web-dev` isn't silently overwritten by serve-web's own
# (feature-less) build-web dependency. Fails if nothing has been built yet.
preview-web:
    test -f site/dist/index.html || { echo "[preview-web] nothing built yet - run just build-web or just build-web-dev first" >&2; exit 1; }
    cd site && yarn preview --port ${PORT:=4321}

# Native dev-tools build with the embedded dev server listening on
# 127.0.0.1:4747 (docs/dev-server-design.md): what the `bongbong` MCP
# server in .mcp.json talks to, so Claude Code (or `just mcp-call`) can
# step, inspect and screenshot the running game. Extra args pass through
# (`just run-dev --seed 0xB0B5 --enemies 4`).
run-dev *ARGS:
    cargo run --features dev-tools -- {{ARGS}}

# The same dev-tools build started in Build mode - the in-game map builder
# (docs/game-editor-fusion.md), with the dev server attached so the
# `builder_*` tools can drive it. `--editor` works in every build, and any
# native build loads and saves maps/*.toml through FILE. Extra args pass
# through (`just run-editor --map maps/test/maze.toml`).
run-editor *ARGS:
    cargo run --features dev-tools -- --editor {{ARGS}}

# `watch` with the dev server: rebuild and relaunch on every source change.
# The MCP adapter reconnects per call, so a relaunch only costs the
# in-flight request.
watch-dev:
    cargo watch -x "run --features dev-tools"

# Call one dev-server tool from the shell, e.g.
# `just mcp-call step '{"frames":120,"move_dir":"up"}'` or `just mcp-call nav_grid`.
# Same tools the MCP server exposes (src/devserver.rs's TOOLS).
mcp-call TOOL ARGS='{}':
    cargo run -q --features dev-tools --bin bbmcp -- call {{TOOL}} '{{ARGS}}'

# --- iOS simulator (docs/ios-native-port-prd.md, CLAUDE.md's iOS section) ---
# Every recipe runs through tools/ios/env.sh: Xcode as DEVELOPER_DIR (the
# devenv shell points it at nix's apple-sdk), one deployment target, the
# library prefix build.rs links from, and bindgen's simulator sysroot.

# One-time: SDL3 (static) and raylib (SDL backend, OpenGL ES 2.0) for the
# simulator into ~/.local/share/bongbong-ios/sim (tools/setup_ios.sh,
# pinned SDL tag).
ios-setup:
    ./tools/setup_ios.sh

# Gate 0: an SDL3 + GL ES 2 glDrawElements program in the simulator
# (tools/ios/smoke.c). Apple Silicon simulators have had a crash in that
# call; rerun after every Xcode or runtime update. Pass = "SMOKE OK".
ios-smoke:
    ./tools/ios/smoke.sh

# Build the game for the simulator (plain build, no dev-tools) and stage
# target/ios-sim/BongBong.app. Extra args go to cargo (`--release`).
build-ios-sim *ARGS:
    bash -c 'set -e; source tools/ios/env.sh; cargo build --target aarch64-apple-ios-sim --bin bongbong {{ARGS}}'
    bash -c 'set -e; source tools/ios/env.sh; ./tools/ios/bundle.sh debug'

# Build, boot the simulator (BONGBONG_IOS_DEVICE, default "iPhone 17"),
# install the bundle and launch it with the console attached (Ctrl-C
# detaches; the app keeps running). Rotate the simulator to landscape with
# Cmd+Left if it comes up portrait.
run-ios-sim *ARGS: (build-ios-sim ARGS)
    bash -c 'set -e; source tools/ios/env.sh; \
        xcrun simctl boot "$IOS_DEVICE" >/dev/null 2>&1 || true; open -a Simulator; \
        xcrun simctl install booted target/ios-sim/BongBong.app; \
        xcrun simctl launch --console-pty --terminate-running-process booted com.otobrglez.bongbong'

# Screenshot the booted simulator (default target/ios-sim/shot.png).
ios-screenshot OUT="target/ios-sim/shot.png":
    bash -c 'source tools/ios/env.sh; xcrun simctl io booted screenshot {{OUT}}'

# --- iPhone (device) ---
# One-time: the device slice of SDL3 + raylib into ~/.local/share/bongbong-ios/ios.
ios-setup-device:
    SLICE=ios ./tools/setup_ios.sh

# Build for the phone and stage target/ios-device/BongBong.app (unsigned).
build-ios-device *ARGS:
    bash -c 'set -e; export IOS_SLICE=ios; source tools/ios/env.sh; cargo build --target aarch64-apple-ios --bin bongbong {{ARGS}}'
    bash -c 'set -e; export IOS_SLICE=ios; source tools/ios/env.sh; ./tools/ios/bundle.sh debug'

# Build, then tools/ios/deploy.sh: sign for the wired iPhone (tools/ios/sign.sh:
# Xcode's automatic signing on the placeholder project mints the certificate
# and profile), install, launch with the console attached and verify the
# round came up (raylib's window, the render targets, the process still
# alive - "DEPLOY OK"). Refusals name the fix: pairing (Trust), Developer
# Mode, or the first run's profile trust under Settings > General > VPN &
# Device Management. Ctrl-C quits the app too; `deploy.sh --no-console`
# detaches instead. Extra args go to cargo (`--features dev-tools` for the
# frame-time log and the dev server, reachable from the Mac through
# `iproxy 4747:4747`).
run-ios-device *ARGS: (build-ios-device ARGS)
    ./tools/ios/deploy.sh iPhone

# The same for the wired iPad (the phone, even when paired over Wi-Fi, is
# never picked). BONGBONG_IOS_UDID pins a device explicitly.
run-ios-ipad *ARGS: (build-ios-device ARGS)
    ./tools/ios/deploy.sh iPad

# --- Android (docs/android-port-prd.md, CLAUDE.md's Android section) ---
# Every recipe sources tools/android/env.sh: the SDK, NDK and JDK paths,
# the API pins, the prebuilt raylib prefix and the NDK compiler for cargo.

# One-time: command-line tools, NDK, platform, arm64 system image, the
# `bongbong` AVD, and raylib built for Android (tools/setup_android.sh).
android-setup:
    ./tools/setup_android.sh

# Gate 0: raylib's Android platform in a NativeActivity (tools/android/smoke.c)
# on the AVD - proves toolchain, packaging, GL ES 2, assets and touch before
# any Rust. Pass = "SMOKE OK" on screen, a non-zero asset size and touch lines in logcat.
android-smoke:
    ./tools/android/smoke.sh

# Build libbongbong_android.so (a plain cargo build for aarch64-linux-android;
# tools/android/env.sh points cargo, cc-rs and bindgen at the NDK) and stage
# target/android/BongBong.apk (tools/android/package.sh: assets/static, the
# .so, debug signature).
build-android *ARGS:
    bash -c 'set -e; source tools/android/env.sh; cargo build --release --target aarch64-linux-android -p bongbong-android {{ARGS}}; \
        tools/android/package.sh target/aarch64-linux-android/release/libbongbong_android.so bongbong_android com.otobrglez.bongbong BongBong target/android/BongBong.apk static/ "" bongbong_on_create'

# Build, boot the AVD if needed, install and launch the game, then follow logcat (Ctrl-C detaches).
run-android *ARGS: (build-android ARGS)
    bash -c 'set -e; source tools/android/env.sh; tools/android/emulator.sh; \
        adb install -r target/android/BongBong.apk; adb logcat -c || true; \
        adb shell am start -n com.otobrglez.bongbong/android.app.NativeActivity; \
        adb logcat -s raylib:V bongbong:V'

# Screenshot the running emulator (default target/android/shot.png).
android-screenshot OUT="target/android/shot.png":
    bash -c 'source tools/android/env.sh; adb exec-out screencap -p > {{OUT}} && echo {{OUT}}'

# Inject a tap (`just android-tap 600 400`) or a swipe (`just android-swipe 1800 600 1800 300`) in screen pixels.
android-tap X Y:
    bash -c 'source tools/android/env.sh; adb shell input tap {{X}} {{Y}}'
android-swipe X1 Y1 X2 Y2 MS="300":
    bash -c 'source tools/android/env.sh; adb shell input swipe {{X1}} {{Y1}} {{X2}} {{Y2}} {{MS}}'

# Conference demos (ntk/): isolated raylib-with-Rust examples, one file per
# demo under ntk/src/bin/. `just ntk 01_hello`.
ntk NAME *ARGS:
    cargo run -p ntk-demos --bin {{NAME}} -- {{ARGS}}
