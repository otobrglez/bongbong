#!/usr/bin/env bash
# Time `Game::update` in wasm: the headless probe built for
# wasm32-unknown-emscripten and run under node, which stands in for a
# browser's engine (the same V8 as Chrome; Safari's is not measured).
#
#   tools/perf/field_wasm.sh [probe args]      (`just perf-field-wasm`)
#
# RUSTFLAGS replaces the link arguments .cargo/config.toml gives the game
# (GLFW, the websocket library, the preloaded static/): the probe needs
# none of them, and node reads the map from disk through NODERAWFS. The
# stack and the memory growth are the game's own.
set -euo pipefail
cd "$(dirname "$0")/../.."

command -v emcc >/dev/null 2>&1 \
    || source ~/.local/share/emsdk/emsdk_env.sh >/dev/null 2>&1 \
    || { echo "[perf-field-wasm] emcc not on PATH and no emsdk at ~/.local/share/emsdk/. Run just setup-web first." >&2; exit 1; }

export RUSTFLAGS="-C link-arg=-sNODERAWFS=1 -C link-arg=-sALLOW_MEMORY_GROWTH=1 -C link-arg=-sSTACK_SIZE=4194304 -C link-arg=-sSUPPORT_LONGJMP=wasm"
cargo build --release --target wasm32-unknown-emscripten --no-default-features --features dev-tools --bin probe

if [ "$#" -eq 0 ]; then
    set -- --map maps/perf/dense-250.toml --scenario defend --frames 3600 --rounds 3 --seed 1000
fi
node target/wasm32-unknown-emscripten/release/probe.js "$@" | grep -E "^probe: (outcomes|timing|phases)"
