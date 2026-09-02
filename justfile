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
# to the recorded baseline (measured 2026-08-27 on the landed Phase 1-6
# binary; each ceiling is the observed maximum across all six fixtures -
# deterministic under the pinned seed, so any exceedance is a real
# behavior change, not noise). After a deliberate AI/map change shifts
# the numbers: rerun, read the new totals, and re-baseline consciously -
# never bump a ceiling just to go green. Zero-ceilings (stale-start,
# stall, border-stuck, wall-grind, bump-rate, never-arrived, invariant)
# are kinds no fixture currently produces at all. See
# docs/gameplay-verification-design.md.
probe-fixtures:
    for m in maps/test/*.toml; do cargo run --bin probe -- --map $m --frames 1800 --rounds 10 --seed 1000 --budget stale-start=0 --budget stall=0 --budget border-stuck=0 --budget jitter=8 --budget spin=2 --budget churn=5 --budget clustering=6 --budget wall-grind=0 --budget bump-rate=0 --budget low-progress=3 --budget never-arrived=0 --budget invariant=0 || exit 1; done

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
build-web:
    bash -c 'set -e; \
        command -v emcc >/dev/null 2>&1 \
            || source ~/.local/share/emsdk/emsdk_env.sh >/dev/null 2>&1 \
            || { echo "[build-web] emcc not on PATH and no emsdk at ~/.local/share/emsdk/. Run just setup-web first." >&2; exit 1; }; \
        cargo build --release --target wasm32-unknown-emscripten'
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
