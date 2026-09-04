# Native dev loop: watch, rebuild, and run on source changes.
watch:
    cargo watch -x "run"

# Standing gameplay-health sweep on the shipped default map: 30 fresh
# seeded rounds (the base seed prints in the header, so any flagged round
# is replayable) plus per-kind anomaly heatmaps. See
# docs/gameplay-verification-design.md and CLAUDE.md's probe bullets.
probe-sweep:
    cargo run --bin probe -- --scenario afk --enemies 4 --frames 1800 --rounds 30 --heatmap

# Sweep every maps/test/ adversarial fixture at a pinned seed and hold it
# to the recorded baseline (re-measured 2026-09-04 after the QA'd tuning
# defaults landed in tuning.rs and the AI's stuck detector switched to
# progress-along-heading - both shift every round's RNG stream; each
# ceiling is the observed maximum across all six fixtures - deterministic
# under the pinned seed, so any exceedance is a real behavior change, not
# noise). After a deliberate AI/map/tuning change shifts the numbers:
# rerun, read the new totals, and re-baseline consciously - never bump a
# ceiling just to go green. Zero-ceilings (stale-start, stall,
# border-stuck, wall-grind, bump-rate, never-arrived, invariant) are kinds
# no fixture currently produces at all. churn=14 and clustering=17 are the
# maze at this one seed (a 30-round sweep at seed 5000 shows the maze
# *improved* on every kind versus the previous defaults - the pinned-seed
# maximum is just less lucky now). jitter=5 is the maze again after the
# rainbow-shield spawn rolls (one RNG draw per tank in `Game::init`) shifted
# every stream; measured identical with the shield knobs zeroed, so it is
# the stream, not the shield. See docs/gameplay-verification-design.md.
# Waves spawn plan health check: the maps/missions/ waves fixture, Destroy
# mission (no frog, so an AFK player only loses to gunfire), 30 seeded
# rounds. Rolling-in tanks are exempt from the anomaly checks until they
# arrive. See docs/maps-to-levels.md.
probe-waves:
    cargo run --bin probe -- --map maps/missions/waves-basic.toml --scenario afk --frames 3600 --rounds 30 --seed 2000 --heatmap

probe-fixtures:
    for m in maps/test/*.toml; do cargo run --bin probe -- --map $m --frames 1800 --rounds 10 --seed 1000 --budget stale-start=0 --budget stall=0 --budget border-stuck=0 --budget jitter=5 --budget spin=3 --budget churn=14 --budget clustering=17 --budget wall-grind=0 --budget bump-rate=0 --budget low-progress=2 --budget never-arrived=0 --budget invariant=0 || exit 1; done

run:
    cargo run

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
