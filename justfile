# Native dev loop: watch, rebuild, and run on source changes.
watch:
    cargo watch -x "run"

run:
    cargo run

# One-time: install the emsdk toolchain (pinned version, see
# tools/setup_emscripten.sh) and the local HTTP server `serve-web` uses.
# The wasm32-unknown-emscripten rustup target is already provided by
# devenv.nix (languages.rust.targets).
setup-web:
    ./tools/setup_emscripten.sh
    cargo install simple-http-server

# Build the release wasm binary and stage index.html (templated from
# web/shell.html), the .js, and the .wasm into target/web/ for `just
# serve-web`. Auto-sources emsdk_env.sh from ~/.local/share/emsdk if emcc is
# not on PATH.
build-web:
    bash -c 'set -e; \
        command -v emcc >/dev/null 2>&1 \
            || source ~/.local/share/emsdk/emsdk_env.sh >/dev/null 2>&1 \
            || { echo "[build-web] emcc not on PATH and no emsdk at ~/.local/share/emsdk/. Run just setup-web first." >&2; exit 1; }; \
        cargo build --release --target wasm32-unknown-emscripten'
    mkdir -p target/web
    rm -f target/web/*
    sed 's/__BONGBONG_BIN__/bongbong/g' web/shell.html > target/web/index.html
    cp target/wasm32-unknown-emscripten/release/bongbong.wasm target/web/
    cp target/wasm32-unknown-emscripten/release/bongbong.js target/web/
    cp target/wasm32-unknown-emscripten/release/deps/bongbong.data target/web/
    @echo "[build-web] target/web/ ready. Run 'just serve-web' to open."

# Serve target/web/ on http://localhost:3535. Run after `just build-web`.
serve-web:
    simple-http-server --index --nocache -p ${PORT:=3535} target/web
