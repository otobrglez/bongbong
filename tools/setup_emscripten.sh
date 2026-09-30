#!/usr/bin/env bash
# Install emsdk so `just build-web` can produce wasm builds. Idempotent:
# re-running pulls + reactivates. Installs to $EMSDK_DIR (default
# ~/.local/share/emsdk).
#
# Usage:
#   ./tools/setup_emscripten.sh
#   EMSDK_DIR=/opt/emsdk ./tools/setup_emscripten.sh

set -euo pipefail

EMSDK_DIR="${EMSDK_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/emsdk}"

if ! command -v git >/dev/null 2>&1; then
    echo "[setup] git is required but not on PATH" >&2
    exit 1
fi
if ! command -v python3 >/dev/null 2>&1 && ! command -v python >/dev/null 2>&1; then
    echo "[setup] python (3.x) is required by emsdk but not on PATH" >&2
    exit 1
fi

if [[ ! -d "$EMSDK_DIR" ]]; then
    echo "[setup] cloning emsdk to $EMSDK_DIR"
    git clone https://github.com/emscripten-core/emsdk.git "$EMSDK_DIR"
else
    echo "[setup] emsdk already at $EMSDK_DIR; updating"
    git -C "$EMSDK_DIR" pull --ff-only
fi

cd "$EMSDK_DIR"
# Pinned to 6.0.10, verified with Rust 1.98.1 (see devenv.nix) by building
# and playing the web build. sola-raylib's own docs pin 5.0.6 because emsdk
# 5.0.7's wasm-opt `--asyncify` pass fails on wasm built with
# `-fwasm-exceptions`, which rustc 1.93+ emits; bongbong links no
# -sASYNCIFY (see .cargo/config.toml), so that bug does not reach it. A
# bump is a build plus a play of the page, not a blind edit. Override with
# EMSDK_VERSION=... if needed.
EMSDK_VERSION="${EMSDK_VERSION:-6.0.10}"
./emsdk install "$EMSDK_VERSION"
./emsdk activate "$EMSDK_VERSION"

cat <<EOF

[setup] Done. To use emcc directly in this shell:

    source $EMSDK_DIR/emsdk_env.sh

Add that to your shell rc to persist it. Or don't: \`just build-web\` and
\`just serve-web\` auto-source emsdk_env.sh from the default install path.

Then:

    just build-web
    just serve-web    # http://localhost:3535

EOF
