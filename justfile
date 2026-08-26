# Native dev loop: watch, rebuild, and run on source changes.
watch:
    cargo watch -x "run"

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
