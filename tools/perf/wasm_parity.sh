#!/usr/bin/env bash
# Does a seed play the same round in wasm as it does natively? The headless
# probe is built twice - for this machine and for wasm32-unknown-emscripten
# (run under node, as tools/perf/field_wasm.sh does) - and both play the
# same rounds printing `Game::state_hash` every frame (`--state-hash-every
# 1`). The STATE and ANOMALY lines must match line for line; the first
# pair that differs names the frame and the part (rng, tanks, bodies,
# world) the two builds part ways in.
#
#   tools/perf/wasm_parity.sh [probe args]      (`just probe-wasm-parity`)
#
# With no arguments: 600 frames of `defend` from seed 1000 on the default
# arena, a field map and the 250 x 250 perf map. Arguments replace that
# set with one run of the probe (`--map maps/longwater.toml --scenario
# advance --frames 3600 --seed 7`).
set -euo pipefail
cd "$(dirname "$0")/../.."

command -v emcc >/dev/null 2>&1 \
    || source ~/.local/share/emsdk/emsdk_env.sh >/dev/null 2>&1 \
    || { echo "[probe-wasm-parity] emcc not on PATH and no emsdk at ~/.local/share/emsdk/. Run just setup-web first." >&2; exit 1; }

cargo build --release --no-default-features --features dev-tools --bin probe
RUSTFLAGS="-C link-arg=-sNODERAWFS=1 -C link-arg=-sALLOW_MEMORY_GROWTH=1 -C link-arg=-sSTACK_SIZE=4194304 -C link-arg=-sSUPPORT_LONGJMP=wasm" \
    cargo build --release --target wasm32-unknown-emscripten --no-default-features --features dev-tools --bin probe

out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

# One probe run on both builds; 0 when their STATE and ANOMALY lines agree.
compare() {
    local name="$1"
    shift
    target/release/probe "$@" --state-hash-every 1 | grep -E "^(STATE|ANOMALY) " > "$out/native.txt" || true
    node target/wasm32-unknown-emscripten/release/probe.js "$@" --state-hash-every 1 | grep -E "^(STATE|ANOMALY) " > "$out/wasm.txt" || true
    local lines
    lines="$(wc -l < "$out/native.txt" | tr -d ' ')"
    if [ "$lines" -eq 0 ]; then
        echo "[probe-wasm-parity] $name: the native probe printed no STATE line" >&2
        return 1
    fi
    if cmp -s "$out/native.txt" "$out/wasm.txt"; then
        echo "[probe-wasm-parity] $name: ok ($lines lines agree)"
        return 0
    fi
    echo "[probe-wasm-parity] $name: DIVERGED - the first lines that differ (native, then wasm):"
    diff "$out/native.txt" "$out/wasm.txt" | grep -E "^[<>]" | sed -n '1p;/^>/{p;q;}' || true
    return 1
}

failed=0
if [ "$#" -gt 0 ]; then
    compare "probe $*" "$@" || failed=1
else
    for map in maps/default.toml maps/longwater.toml maps/perf/dense-250.toml; do
        compare "$map" --map "$map" --scenario defend --frames 600 --rounds 1 --seed 1000 || failed=1
    done
fi
exit "$failed"
