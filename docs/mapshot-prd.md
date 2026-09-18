# PRD: `mapshot`, a map-to-PNG command line tool

Written 2026-09-17 and built the same day (section 7 records what shipped
and how it was verified). Same shape as docs/android-port-prd.md: the
decision, what existed before, the design by area, the phases, the risks.
Before this nothing in the tree rendered a map without a window and
nothing in `site/`, `docs/` or the README mentioned thumbnails.

## 1. The decision, in one paragraph

Add a bin, `src/bin/mapshot.rs`, that takes map TOML files (or
directories of them) and writes one PNG per map showing **the first frame
of a fresh round on that map, field only, at 1:1 pixels**: ground
tileset, road, oil pools, grass, walls, props, trees, frogs, pickups and
the player start tank(s) with their steady ring, and nothing that moves or
fights - no enemies, no intro banner, no round-start locate cue. It has
**two renderers behind one flag, `--renderer cpu|gpu`, default `cpu`**.
Both run the *same* drawing code: the game's static-layer draw functions
become generic over a small new `Canvas` trait. The GPU canvas wraps
raylib's draw handle and the real textures inside a hidden window and is
the game's own pass 1 to the pixel. The CPU canvas is a small
nearest-neighbour rasteriser over a pixel buffer that raylib's `Image`
functions decode the sprite sheets into and encode the result out of; it
never opens a window, never touches GL, and so runs on a server with no
GPU and no display. No new crate. The bin is a dev tool like `probe`, not
in the release archives, and the consumer for now is a shell, not a site
gallery or an in-game picker.

## 2. Goals and non-goals

Goals
- `mapshot maps/default.toml -o default.png` writes a 1088 x 544 PNG of
  what a player sees when that map's round starts, and `mapshot --out-dir
  target/thumbnails maps` does it for every map under `maps/`.
- Identical content to the game's first rendered frame except for the
  two-second locate cue (ripple and `P1`/`P2` label), which is animation,
  not map.
- Deterministic: the same (map, seed, flags) gives byte-identical PNGs,
  so thumbnails can be regenerated on any machine and diffed.
- `--renderer cpu` works on a headless Linux box; `--renderer gpu` is the
  cross-check that the CPU output matches the game.
- `just thumbnails` for the batch, a `cargo test --lib` test that renders
  the shipped maps on the CPU canvas against pinned hashes, and
  `mapshot --check` (`just mapshot-compare`) for the GPU-vs-CPU
  comparison.

Non-goals, for now
- A map gallery on the site, thumbnails in the builder's load dialog or a
  level select. The renderer is designed so the CPU path *could* run in
  the wasm build later (it is plain Rust over a pixel buffer), but no page
  or dialog is built here.
- Window framing: no HUD bar, no letterbox, no `View` scaling. The output
  is the field the map defines.
- Mid-round states (wrecks, fire, rubble). The devserver's `screenshot`
  tool already covers those on a running game.
- Shipping `mapshot` in the cargo-dist archives (`dist-workspace.toml`
  `[dist.binaries]` stays `bongbong` + `probe`).
- Touching the builder's renderer (`Builder::render`, src/editor/mod.rs),
  which blits authored cells straight from `MapFile` and shares only
  `ground::draw` with the game.

## 3. What we had before (verified 2026-09-17, before the build)

### The game's renderer needs a window and ends on the screen
- `Game::render` (src/game.rs:162) takes the `RaylibHandle`, two
  `RenderTexture2D`s (`scene_target` at field size, `composite` at window
  size), `Textures` (23 texture refs, src/game.rs:48), `Effects` (three
  `RippleFx` shaders and the particle layer), `Layout`, `PlayChrome`, and
  always ends in `view::present` (src/view.rs:112), a real begin/end
  drawing to the window.
- Pass 1 (src/game.rs:232-499) draws the field into `scene_target` in a
  fixed order: ground and edge shade, tread marks, scorches, landed
  rubble, oil pools, burning cells, non-tree obstacles with shadow and
  cap, the additive glows, pickups, the y-sorted `Standing` walk that
  merges tanks and frogs with grass tufts by root, trees sorted by y, the
  player labels, projectiles, blasts, airborne rubble and drums, then
  particles. Every leaf draw function takes `d: &mut impl RaylibDraw` plus
  a texture ref (`draw_obstacle`, `draw_tree`, `draw_tuft`, `draw_frog`,
  `draw_pickup`, `draw_tank`, ...). Between them they use seven raylib
  draw calls: `draw_texture_pro` (16 sites), `draw_rectangle` (6),
  `draw_ring` (5), `draw_circle_v` (4), `draw_text` (2),
  `draw_rectangle_gradient_v`/`_h` (2 + 2).
- On frame 0 a player tank draws the locate ripple and the steady ring
  (`draw_one_tank`, src/game.rs:105; `player_locate_active(0.0)` is true
  for `player_locate_seconds`, 2 s) and the `P1` label over everything
  (src/game.rs:432).
- `plain_canvas` and `hide_players` (src/simulation/mod.rs:593, :596) exist
  for the conference demos and are honoured in pass 1.

### A round can be built without raylib
- `Game::init` (src/simulation/mod.rs:695) has no raylib dependency; the
  probe (src/bin/probe.rs:1501) and the linter (src/maplint.rs:1067) build
  rounds this way. `show_intro = false` gives no banner;
  `enemy_count_override = Some(0)` places no enemies for a Band plan (a
  Waves plan places none in `init` either, src/simulation/mod.rs:828, and
  ignores the count, src/level.rs:163). Pickups spawn in `init`
  (src/simulation/mod.rs:1001), so do the frogs (:962, :997); ground and
  grass are built at :1023 from the round RNG, so `seed_override` pins
  every cosmetic tile pick.
- **The first frame is init plus one update.** `Tank::ring_position`
  starts at `Position::default()` (src/tank.rs:559) and is snapped onto
  the hull by `ease_ring_position` inside `Game::update`
  (src/simulation/mod.rs:2928). The windowed game always updates before
  its first render. A renderer that skips the update draws the player
  ring at (0, 0). With no enemies that one update draws no RNG, so the
  frame is still a pure function of (map, seed).
- `Game::ground`, `grass` and `oil_cells` are `pub(crate)`
  (src/simulation/mod.rs:441): a bin cannot paint the field on its own,
  the shared routine has to be a `Game` method in the library.
- `Game` derives `Default`; `shadows_enabled` is false unless set, and the
  game sets it (src/app.rs:636).

### Precedents in the tree
- `ntk/src/bin/06_explosions.rs` opens its own raylib window, loads the
  same ~25 textures by hand, configures a `Game` with `show_intro = false`,
  `enemy_count_override = Some(0)`, `plain_canvas`, `hide_players`, and
  calls `game.render` with its own handle.
- The devserver's `screenshot` (src/devserver.rs:859) reads a render
  texture back with `scene.load_image()` + `flip_vertical()` and encodes
  with `export_image_to_memory(".png")`. Its one-frame lag is a *screen*
  read-back artefact; a render texture reads back at once.
- `bbmcp` (src/bin/bbmcp.rs:14) shows the emscripten stub `main` every
  native-only bin needs, since the wasm dev build compiles every bin.
  `probe` is auto-discovered from `src/bin/` with no `[[bin]]` entry.

### What sola-raylib gives us (../sola-raylib/raylib/src/core/)
- `RaylibBuilder::hidden()` (mod.rs:229); its doc comment describes
  exactly this use. `RaylibDraw` (drawing.rs:497) is a trait of provided
  methods that every mode wrapper (`RaylibTextureMode`, `RaylibMode2D`,
  `RaylibBlendMode`, ...) implements with an empty `impl`, so a blanket
  `impl<D: RaylibDraw> Canvas for GpuCanvas<'_, D>` is clean.
- `Image::load_image(path)` (texture.rs:950), `get_image_data` (:252),
  `gen_image_color` (:823), `draw_pixel` (:421), `export_image` (:230) and
  `export_image_to_memory` (:771) are pure C calls into rtextures (stb)
  that take no `RaylibHandle`. `ffi::SetTraceLogLevel` is callable before
  any window. `Color`, `Rectangle`, `Vector2` are plain `repr(C)` structs.
- **raylib's own `ImageDraw` is not usable for the CPU blits.** The
  vendored raylib 6.0 resizes with stb's *linear* filter when source and
  destination sizes differ (tanks draw at `tank.scale`, grass at
  `grass_scale`, pickups at `PICKUP_SCALE` - every scaled sprite would
  blur), ignores a negative source width (no mirroring, which
  src/grass.rs:222 and src/frog.rs:495 rely on), `ImageDrawRectangle`
  copies bytes without alpha blending, and `ImageDrawText` lazily loads
  the default font through a GL texture. Hence an own rasteriser, with
  raylib only decoding the sheets and encoding the PNG.

### No `image` or `png` crate
Neither is in `Cargo.toml` or `Cargo.lock`; PNG encoding today goes
through raylib. This design keeps it that way.

## 4. Design by area

### 4.1 `src/canvas.rs`: the trait and its two backends

`Sheet` names a sprite sheet without saying where it lives:

```rust
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Sheet {
    Ground, Tanks, Walls, Props, Trees, Grass, Damage, MinigunMount,
    Tracks, BarrelExplosion,
    Pickup(PickupKind),
    Frog { variant: u8, clip: FrogAnim },
}
```

It replaces `obstacle::Sheet` (src/obstacle.rs:74, the three-variant
Walls/Props/Trees enum) so there are not two enums of that name; the
pickup `match` at src/game.rs:317-329 folds into `Sheet::Pickup(kind)`,
and `Sheet::Frog` addresses `textures.frog_variants[variant].<clip>`. A
single path table (`Sheet::path()` or `pub const` items beside it) is the
one list the GPU loaders in src/app.rs:477-560, the ntk demo and the CPU
loader all read.

`Canvas` is the whole drawing vocabulary the static field needs:

```rust
pub trait Canvas {
    /// `DrawTexturePro` semantics: `dest.x/y` is where `origin` lands,
    /// `rotation` in degrees clockwise (y down), a negative `src.width`
    /// mirrors, `tint` multiplies RGBA, alpha-over blending.
    fn blit(&mut self, sheet: Sheet, src: Rectangle, dest: Rectangle,
            origin: Vector2, rotation: f32, tint: Color);
    fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Color);
    fn gradient_v(&mut self, x: i32, y: i32, w: i32, h: i32, top: Color, bottom: Color);
    fn gradient_h(&mut self, x: i32, y: i32, w: i32, h: i32, left: Color, right: Color);
    fn disc(&mut self, center: Vector2, radius: f32, color: Color);
    /// raylib `DrawRing` angles: degrees from +x, clockwise on a y-down screen.
    fn ring(&mut self, center: Vector2, inner: f32, outer: f32,
            start_deg: f32, end_deg: f32, segments: i32, color: Color);
}
```

There is deliberately **no text method**: the CPU side cannot draw text
without a window, and the only text on the field is the transient
`P1`/`P2` label, which stays raylib-only (4.3).

`GpuCanvas<'a, D: RaylibDraw, S: Sheets>` holds `&mut D` and `&S`, where
`Sheets` is the one-method lookup trait `fn texture(&self, Sheet) ->
&Texture2D`, implemented by `game::Textures` (which otherwise stays as it
is), by `editor::EditorTextures` (for the `ground::draw` the builder
shares, panicking on a sheet only a live round has) and by
`thumbnail::GpuSheets` (a `BTreeMap<Sheet, Texture2D>` loaded straight
from `Sheet::all()`). Every method forwards to the matching raylib call,
so the game's output does not change by a pixel. A `GpuCanvas` borrows
the draw handle for one statement, so `render` makes one per stage.

`CpuCanvas { width, height, pixels: Vec<Color>, sheets: BTreeMap<Sheet,
Pixels> }`, where `Pixels` is a decoded sheet (`Image::load_image` +
`get_image_data`). `CpuCanvas::load(width, height)` reads the path table;
`write_png(&self, path)` builds an `Image` with `gen_image_color` and
`draw_pixel`, encodes with `export_image_to_memory(".png")` and writes the
bytes (the devserver's own route, src/devserver.rs:883). The rasteriser:
- `blit`: a `rotation == 0` fast path (integer destination rect, per-axis
  source step, point sampling; the common case for ground, walls, props,
  trees, pickups and tanks facing up) and a general path that walks the
  rotated quad's bounding box and inverse-maps each pixel (translate by
  `-dest.xy`, rotate by `-rotation`, add `origin`, scale to source, point
  sample; a negative `src.width` mirrors the sample). The general path is
  required, not optional: grass tufts carry a small non-zero `bend`
  rotation on frame 0 (src/grass.rs:229).
- Blending: `src * tint` then alpha-over, integer arithmetic rounding the
  way raylib's `ColorAlphaBlend` does. The canvas is cleared opaque white
  like pass 1 (src/game.rs:236), so output alpha stays 255.
- `disc` is a distance test; `ring` is the true annulus with an angle
  window (`atan2(dy, dx)` in degrees, `rem_euclid(360)`, inside
  `[start, end]`), `segments` ignored. GL draws a 48-gon here; this is the
  main GPU-vs-CPU difference and what the comparison tolerance in 4.5
  absorbs. Gradients lerp per row or column, then blend.

### 4.2 Draw functions that become generic

Mechanical: `d: &mut impl RaylibDraw` becomes `c: &mut impl Canvas`, the
texture parameter goes, `d.draw_texture_pro(tex, ...)` becomes
`c.blit(Sheet::X, ...)`, `draw_rectangle`/`draw_ring`/`draw_circle_v`/the
gradients become the trait calls.

| module | functions |
|---|---|
| src/ground.rs:292, 325 | `draw_edge_shade`, `draw` |
| src/grass.rs:205 | `draw_tuft` |
| src/obstacle.rs:636-963 | `draw_oil_cell`, `draw_obstacle`, `draw_obstacle_cap` (and the private scorched-faces helper), `draw_tree`, `draw_tree_shadow`, `draw_obstacle_shadow`, `draw_flying_drum`; `ObstacleTextures` leaves the draw path |
| src/frog.rs:460, 479 | `draw_frog_ring`, `draw_frog`; `FrogTextures::as_frog_textures` goes |
| src/pickup.rs:106 | `draw_pickup` |
| src/tank.rs:1260-1800 | `draw_tank`, `draw_tank_shadow`, `draw_ground_ring`, `draw_ground_ring_at`, `draw_ground_ring_scaled`, `draw_tank_shield`, `draw_player_ring`, `draw_enemy_ring`, `draw_player_locate`, `draw_minigun_mount`, `draw_minigun_mount_shadow` |
| src/damage_stage.rs:83 | `draw_damage` |
| src/track.rs:57, src/blast.rs:391 (`draw_scorch`), src/decal.rs:175 (`draw_decal`) | one `draw_texture_pro` each |

Raylib-only and unchanged: `draw_player_label` (text), the additive glow
set (`draw_fuse_glow`, `draw_fire_glow`, `draw_flame_glow`,
`draw_burning_hull_glow`, `draw_blast_glow`), `draw_ground_fire`,
`draw_blast`, `draw_decal_shadow`, every projectile drawer, `Fx::draw`,
all of pass 2, the HUD and the editor. The editor's one `ground::draw`
call (src/editor/mod.rs:1260) wraps its handle in a `GpuCanvas` over a
small `EditorTextures`-backed lookup, or `ground` keeps a one-line GPU
helper - whichever is smaller when it comes to it.

### 4.3 The shared routine: `Game::paint_*` in src/game.rs

Presentation code that reads `Game` and never mutates it, the module's
existing rule. Pass 1 is cut at the seams where GPU-only content is
inserted:

```rust
impl Game {
    /// Ground, edge shade, tread marks, scorches, landed rubble, oil pools.
    pub fn paint_floor(&self, c: &mut impl Canvas);
    /// Walls and props with their shadows and caps. Not trees.
    pub fn paint_tiles(&self, c: &mut impl Canvas);
    /// Pickups, the y-sorted tanks/frogs/grass walk, then the trees.
    pub fn paint_standing(&self, c: &mut impl Canvas, opts: PaintOptions);
    /// The three above in order: the static field as a fresh round shows it.
    pub fn paint_field(&self, c: &mut impl Canvas, opts: PaintOptions);
}

pub struct PaintOptions { pub locate_cue: bool }
```

`Game::render` pass 1 becomes: clear white, `paint_floor`
(`plain_canvas` is checked inside it, as at src/game.rs:241), the burning
cells, `paint_tiles`, the additive glows, `paint_standing(locate_cue:
true)`, `draw_player_label`, projectiles, blasts, airborne rubble and
drums, particles. The draw order is exactly today's, so the game's output
is byte-identical. `Standing`, `TankRole` and `draw_one_tank`
(src/game.rs:93-137) move into the generic stage unchanged apart from the
canvas type.

`locate_cue` exists because the ripple is drawn *under* the ring inside
the sorted walk (src/game.rs:113-118) and cannot be left out afterwards;
the label can, and is, since `Game::render` draws it after the stage. The
thumbnail passes `false`: the steady ring is the player's static marker
and health gauge, the ripple and label are a two-second cue. `--no-tanks`
reuses `hide_players`.

### 4.4 The command line: `src/bin/mapshot.rs` over `src/thumbnail.rs`

The library side is `thumbnail.rs`: `ThumbnailOptions` (seed, players,
chassis rows, `hide_players`, `plain_canvas`, shadows), `stage_round`
(the recipe below), `load_cpu_sheets`/`render_cpu`, `GpuSheets`/
`render_gpu`, `compare_pixels`/`Comparison`, and the tests of 4.5. The
bin is argument parsing, batch planning and file writing.

Clap derive in the probe's style (src/bin/probe.rs:325-410, `ExitCode`
main at :1827):

```
mapshot [OPTIONS] <MAP>...      files or directories (a directory expands
                                to its **/*.toml, sorted)
  -o, --out <FILE>              one map only; conflicts with --out-dir
      --out-dir <DIR>           default: beside each map as <stem>.png;
                                with a directory input, the relative path
                                with .toml -> .png under <DIR>
      --renderer <cpu|gpu>      default cpu
      --scale <N>               integer >= 1, nearest-neighbour (resize_nn)
      --seed <S>                decimal or 0x-hex; default a fixed
                                constant (0xB0B5) so batches are stable
      --players <1|2>           default 1
      --tank <KIND>, --tank2    TankKind, as the game's flags
      --no-tanks                hide_players
      --plain                   plain_canvas (flat white ground)
      --no-shadows              shadows are on by default, as in the game
      --quiet                   silence raylib's own log (warnings show
                                by default, its INFO chatter never does)
      --check                   render both ways, print how far apart they
                                are, fail beyond tolerance (see 4.5)
```

Per map: `MapFile::load` (src/map.rs:242), then `Game::default()` with
`show_intro = false`, `enemy_count_override = Some(0)`,
`level_overrides.spawn = Some(SpawnKind::Band)` (so the zero count is
honoured whatever the map's plan says), `seed_override`, `shadows_enabled
= true`, `players`, `player_row_override`, then `init(w, h)` with
`map.field_size()` (src/map.rs:235), **one**
`update(Input::default(), PHYSICS_FIXED_DT, w, h)`, then
`paint_field(locate_cue: false)` into the chosen canvas and a PNG of
`field_size x scale`. Never the HUD bar.

The GPU renderer opens one `.hidden()` window for the whole batch, loads
every sheet in `Sheet::all()` as a texture (`GpuSheets`), creates a
`load_render_texture(w, h)` per map, paints inside `draw_texture_mode`
through a `GpuCanvas`, then `scene.load_image()` + `flip_vertical()`. It
calls `paint_field`, never `Game::render`, so it needs no `Effects`,
ripple shaders, `View`, `Layout` or `PlayChrome`, and reads nothing back
from the swap chain. One draw is enough: the read-back is from the
texture, not the screen.

Exit codes: 0 when every map was written; 1 when any map failed (bad TOML,
missing sheet, unwritable output - the batch continues and each failure is
one `eprintln!` line); 2 for usage errors (clap's own, and `-o` with more
than one map). stdout prints one `path -> out.png WxH` line per success.
Sheets resolve against the working directory (`static/...`), the contract
every bin has; a missing `static/` is one clear error before any map is
attempted. `#[cfg(target_os = "emscripten")] fn main() {}` as in `bbmcp`.

### 4.5 Tests and recipes

- `canvas.rs` tests the rasteriser itself, no window: a plain blit lands
  texels where the GPU would, a scaled blit is nearest-neighbour, a
  negative source width mirrors, a quarter turn about the centre, the
  origin shift, tint and alpha-over values, transparent paint is a no-op,
  clipping never panics, disc and ring coverage, the ring's raylib angle
  convention (and swapped angles), gradient direction, the hash, every
  `Sheet::path` distinct, every sheet decoding from `static/`.
- `thumbnail.rs` tests, under `cargo test --lib`, no window: every
  `map::SHIPPED_MAPS` entry (src/map.rs:430) through `stage_round` onto a
  `CpuCanvas`; the size equals `field_size`, the picture has more than 64
  distinct colours and stays opaque, two renders of one map hash the
  same, and the hash equals a **pinned constant per map**. Re-baseline
  consciously after a deliberate art, map or tuning change, the way `just
  probe-fixtures` ceilings are treated, never to go green. Also: the
  options change the picture (`hide_players`, `plain_canvas`, the seed), a
  Waves map and a Band map both stage zero enemies, and `compare_pixels`
  measures what it says.
- GPU-vs-CPU is **`mapshot --check`**, not a test: macOS creates windows
  on the main thread only and the test harness runs tests on worker
  threads, so an `#[ignore]`d test that opens a window hangs there. The
  flag renders each map both ways and prints `Comparison` - the mean
  absolute per-channel difference and the share of pixels whose summed RGB
  difference exceeds 24 - and a map beyond `MEAN_TOLERANCE` (2.0) or
  `OFF_TOLERANCE` (2 %) counts as a failure. `just mapshot-compare` runs
  it on the shipped maps and the props fixture.
- `just thumbnails`: `cargo run --bin mapshot -- --out-dir
  target/thumbnails maps` (under `target/`, already ignored);
  `just thumbnails-gpu` the same through the GPU renderer.

## 5. Phases (all done 2026-09-17)

1. `src/canvas.rs`: `Sheet`, `Canvas`, `Sheets`, `GpuCanvas`, `CpuCanvas`
   with the loader, rasteriser and PNG encoding, the rasteriser tests.
2. Convert the leaf draw functions of 4.2 and the editor's `ground::draw`
   call; `obstacle::Sheet` becomes a re-export, `ObstacleTextures`,
   `texture_for` and `FrogTextures` go (`FrogVariantTextures::clip`
   replaces the last).
3. Split `Game::render` pass 1 into `paint_floor` / `paint_tiles` /
   `paint_standing` / `paint_field`, verified byte-identical (section 7).
4. `src/thumbnail.rs` and `src/bin/mapshot.rs`: options, staging, both
   renderers, `--check`, batch planning, exit codes. No emscripten stub
   turned out to be needed: the bin is plain std over the library.
5. Shipped-map hash tests, the three `just` recipes, CLAUDE.md bullets for
   `canvas.rs`, `game.rs`'s paint stages and `bin/mapshot.rs`.

## 6. Risks and how the design absorbs them

- **raylib `ImageDraw` semantics** (linear resize, no mirror, no blend on
  rectangles, text needs GL): avoided entirely by the own rasteriser;
  raylib only decodes and encodes. raylib 6.1's `ImageDrawImagePro`
  (../raylib/src/rtextures.c) still resizes linearly, so waiting for it
  would not help.
- **Performance**: roughly 600 ground cells plus a few hundred tiles at
  32 x 32 and a handful of larger sprites - a few million pixel operations
  per shot, well under 100 ms even in a debug build. The final
  `draw_pixel` copy into a raylib `Image` is about 600 k FFI calls at
  1088 x 544, fine for a CLI.
- **A headless Linux server** still links raylib's GLFW/X11 stack, so the
  runtime `.so` files must be installed (CI's apt list in
  .github/workflows/ci.yml already installs the dev packages), but the
  CPU path never calls `InitWindow`, so no `DISPLAY` is needed. Only
  `--renderer gpu` needs a display or Xvfb. raylib aborts hard when a
  window cannot open, which is why there is no automatic gpu-to-cpu
  fallback and `cpu` is the default.
- **Trace log noise**: raylib logs every decoded file at INFO, straight
  to stdout. The bin sets the level to warnings before its first raylib
  call and again on the window builder (which otherwise resets it);
  `--quiet` silences it entirely. The tests do the same for the sheets
  they decode.
- **Time-dependent cosmetics** (grass sway, the trees' shimmer column,
  the frog's idle frame) are functions of `time = 1/60` and position
  hashes, so they are deterministic and the pinned hashes catch any
  drift. Ground and grass tile picks come from the round RNG and are
  pinned by `--seed`.
- **The `Sheet` rename** touches src/obstacle.rs and every
  `ObstacleTextures` site; it is mechanical and the compiler finds all of
  them.
- **The pass-1 split** must keep the game's draw order exactly. Phase 3's
  before/after hash on a pinned seed is the guard; CLAUDE.md's rule that
  presentation never mutates `Game` keeps the split honest.
- **wasm**: the library side is plain Rust over a pixel buffer plus
  raylib's CPU image calls, and the bin uses only std, so both compile
  for emscripten as they are; the wasm build was not exercised for this
  work (not a target of it).
- **A test cannot open a window on macOS**: GLFW creates windows on the
  main thread only, the test harness runs tests on worker threads, and
  the attempt hangs rather than failing. Hence `--check` in the bin and
  the `just mapshot-compare` recipe instead of an `#[ignore]`d test.

## 7. What shipped and how it was verified (2026-09-17)

- `cargo test --lib` green, including the new `canvas` and `thumbnail`
  tests; `cargo build --all-targets --features dev-tools` clean.
- **Byte-identity of the game's render**: the previous commit and the
  refactored tree were built with `dev-tools`, each run with `--seed 1`
  on its own dev port, driven to the same lockstep frame with `restart
  {seed: 1, intro: false}` and `step`, and their `screenshot {source:
  "scene"}` PNGs compared by SHA-256. Frame 2 (the first frame, player
  ring and locate cue up) and frame 400 (four wave enemies live on the
  field, particles budgeted to zero on both since `fx.rs` draws from an
  unseeded RNG) were identical byte for byte.
- **GPU against CPU** (`mapshot --check` on `default`, `hunt-basic`,
  `waves-basic` and `maps/test/props.toml`): mean per-channel difference
  0.03-0.04, at most 0.01 % of pixels off. The residue is the ring
  polygon against the true annulus and GL's edge rules.
- **Speed**: the CPU renderer does all 30 maps under `maps/` in under five
  seconds in a debug build, sheet decoding included.
- The CPU thumbnail of `default.toml` was eyeballed beside the GPU one:
  ground, road, walls, props, trees, grass, pickups, the frog with its
  ring and the player tank with its steady ring, no cue, no enemies.
