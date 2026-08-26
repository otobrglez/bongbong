# bongbong.io

The Astro site that ships the wasm build of BongBong at bongbong.io.

Astro owns the page shell (`src/pages/index.astro`), styling, and any
marketing content. It does **not** build the game itself - `public/game/`
holds `bongbong.{wasm,js,data}`, copied in from a Rust/Emscripten build
(`../target/wasm32-unknown-emscripten/release/`) by `just build-web` in the
repo root *before* `yarn build` runs. Those files are gitignored and must
exist before `yarn build`/`yarn dev` will actually show the game - run
`just build-web` from the repo root first, or just `just serve-web` which
does both steps.

Astro/Vite never processes `public/game/*`: it's copied byte-for-byte into
`dist/`, which is required since Emscripten's glue JS looks up the
`.wasm`/`.data` files by a fixed relative filename baked in at compile
time - hashed asset names would break it. Minification of that glue JS and
the wasm binary itself happens during the Rust build (Cargo's release
profile forwards `-O3` to `emcc`, which runs Binaryen's `wasm-opt` and
Emscripten's own JS minifier) - Astro/Vite only minifies its own build
output (this page's markup/CSS/scripts).

## Commands

Run from `site/`:

| Command         | Action                                      |
| :--------------- | :------------------------------------------ |
| `yarn install`   | Install dependencies                        |
| `yarn dev`       | Local dev server at `localhost:4321`        |
| `yarn build`     | Build production site to `./dist/`          |
| `yarn preview`   | Preview the build locally before deploying  |

Or from the repo root: `just build-web` (builds the wasm + the site) and
`just serve-web` (build, then serve `site/dist/` locally).
