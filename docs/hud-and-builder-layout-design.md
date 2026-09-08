# HUD sidebar + in-game map builder — layout sketch

Status: proposal, nothing implemented. Sketches where the player's
inventory readout (health, shells, weapon queue, wave, objective) lives so
it never covers the battlefield, and reserves the same space for the map
builder once it is fused into the game instead of being the separate
`--editor` driver it is today (docs/map-editor-design.md).

## Where things are today

- The **battlefield is the window**. `battlefield::wall_rects` puts the
  boundary walls' inner faces at `0/width/0/height`, `Game::init`/`update`,
  `ground::build` and `Game::nav_grid` all take the window size, and maps
  are authored on that grid: `maps/default.toml` spans cells 0..40 x 0..22
  (32 px cells, so 1280x704 of the 1280x720 default).
- The **HUD is an overlay**: `SHELLS/HP/…` top-left, `WAVE n/m ENEMIES k`
  top-right, the version line bottom-right, the dev `DEV overlays:` label
  and inspect readout under the top-left line (`game.rs::render`). All of
  it sits on top of playable cells.
- The **editor chrome is an overlay too**: hamburger top-left, `New/Save/
  Load/Close` top-right and a 20-icon palette bottom-centre that is 1132 px
  wide and hides the bottom two rows of cells (`MapEditor::point_on_ui`
  refuses clicks there). Its palette floats over the very rows the default
  map uses.
- The web page (`site/src/pages/index.astro`) sizes the canvas to
  `min(100vw, 1280px) x min(100vh, 720px)`; the tuning panel sits below it.

## The one rule: the battlefield's size is frozen

Every seeded baseline in the repo assumes a 1280x720 field: the probe
sweeps (`just probe-fixtures`' recorded ceilings), `maplint`'s spawn-band
counts, `determinism_tests`, and every hand-authored map under `maps/`.
Shrinking the field to make room for a HUD (say 40x20 cells, an 80 px band)
would drop `default.toml`'s rows 20-22 and shift every spawn sample. Scaling
the field down to fit next to a panel would resample pixel art at a
non-integer factor and blur the look the whole asset pipeline is built
around.

So the panel is added **outside** the field and the **window grows**:
window = field + sidebar. `--resolution` keeps meaning the field (that is
what the probe shares with the game via `DEFAULT_SCREEN_WIDTH/HEIGHT`).

## Layout: a right-hand sidebar

```
 ┌────────────────────────────────────────────────────┬──────────┐
 │                                                    │ PROTECT  │
 │                                                    │ WAVE 2/5 │
 │                                                    │ ▮▮▮▮▯▯▯  │  enemies alive / pending
 │                                                    │ ♥  100   │
 │              battlefield 1280 x 720                │ ▬  12    │  shells
 │              40 x 22.5 cells, unchanged            │ [L]  3   │  weapon queue: icon + count, active outlined
 │              world origin stays (0,0)              │ [P]  6   │
 │                                                    │ [M]  --  │
 │                                                    │ SPEED ▭  │  timed buffs
 │                                                    │ FROG  ▭  │  objective HP
 │                                                    │          │
 │                                                    │ [BUILD]  │  map-editor builds only
 │                                                    │ v0.0.9   │
 └────────────────────────────────────────────────────┴──────────┘
                      1280                                160
```

- **Sidebar width 160 px = 5 cells** (`OBSTACLE_GRID_SIZE` multiples, so
  the ground tiles can continue under it if we ever want that). Default
  window becomes **1440x720** (2:1). 128 px (4 cells) also works for
  icon+number readouts but is too tight for the 26 px `SHELLS 12`-style
  labels and for a three-column palette, see below.
- **Right, not bottom**, for three reasons: the field's scarcer dimension
  is height (22.5 cells vs 40), so a side band costs nothing the player
  feels; the readouts are a vertical list (weapon queue, buffs, objective)
  and read better stacked; and it is the Battle City arrangement the game
  is pastiching, enemies-remaining column included.
- Play-mode stack, top to bottom (all from `Game` read-only accessors that
  `render` already uses: `with_tank`, `wave_status`, mission, frog):
  mission + wave, enemy pips (alive solid, pending outlined - the wave
  plan's `pending` count is already in `wave_status`), heart glyph + HP,
  shell glyph + count, the weapon queue as **icon + count rows** (the
  pickup icons already identify laser/plasma/minigun; a 26 px `MINIGUN 120`
  word label is ~150 px and does not fit a 136 px column) with the active
  row outlined in its accent colour instead of the current `>` marker,
  speed/shield timer bars, the objective frog's HP (Protect and Hunt), then
  the footer: the dev `DEV overlays:` preset label and inspect readout, the
  `BUILD` toggle (only with `map-editor`), the version line.
- The mission intro banner, pause/end-screen dims and the barrel flash
  centre on and cover the **field rect**, not the window - the sidebar
  never flashes or dims.

## The same sidebar in build mode

```
 ┌────────────────────────────────────────────────────┬──────────┐
 │                                                    │ [ PLAY ] │  test this map now
 │                                                    │ New Save │
 │                                                    │ Load Clos│
 │                                                    │ ▢ ▢ ▢    │  palette: 20 tools,
 │              same field, every cell clickable      │ ▢ ▢ ▢    │  3 columns x 7 rows,
 │              (the palette no longer covers it)     │ ▢ ▢ ▢    │  40 px icons
 │                                                    │ ▢ ▢ ▢    │
 │                                                    │ ▢ ▢ ▢    │
 │                                                    │ ▢ ▢ ▢    │
 │                                                    │ ▢ ▢      │
 │                                                    │ tanks  6 │  map-level keys the
 │                                                    │ tank tita│  editor cannot set today
 │                                                    │ mission  │
 │                                                    │ arena1 * │  name, dirty, cursor cell
 └────────────────────────────────────────────────────┴──────────┘
```

Budget at 160 px wide, 12 px padding: `PLAY` 40 + toolbar 2x2 of 66x36
buttons 80 + palette 3x7 of 40 px icons with 8 px gaps 328 + map settings
~96 + footer ~48, with 16 px group gaps = ~650 of 720. The current 48 px
icons in two columns (10 rows, 552 px) also fit but leave no room for the
settings block; 40 px cells still hold the 32 px sprites with a 4 px inset.
The hamburger goes away: the mode toggle is the sidebar's top slot in both
modes (plus a key, `Tab`).

The map-level settings block is the real win of fusing the builder: `tanks`,
`tank`, `mission.kind`, `spawn.kind`/`waves`/`wave_size` are map keys today
that only a text editor can set. Steppers in the sidebar make a map fully
authorable in-game.

## Mode switch

```
             clone game.map (MapFile)              Tab / [BUILD]
   Game  ─────────────────────────────────────►  MapEditor
   (Play) ◄─────────────────────────────────────  (Build)
             game.map = editor map; Game::init(field)   Tab / [PLAY]
```

- `main.rs` owns a `Driver { Play, Build }` next to the two structs; both
  exist for the whole session and share the loaded `Textures`. The loop
  body branches on the driver: Play gathers `simulation::Input` and calls
  `Game::update`/`render`; Build calls `MapEditor::update`/`render`. The
  simulation never learns the editor exists (the design doc's boundary).
- **Play → Build** seeds the editor from `Game::map` (a `pub MapFile`, the
  map the round was built from). No `from_placed` conversion of live
  obstacles is needed: the source of truth is already in memory, and
  editing the *authored* map rather than the damaged battlefield is what
  "tweak what I'm looking at" should mean.
- **Build → Play** is the dev server's `restart {map_toml}` path without
  the TOML: `game.map = editor.map().clone(); game.init(field_w, field_h)`.
  The round restarts on the edited map (fresh seed unless `--seed`). If
  the editor's dirty flag is clear, resume the paused round instead.
- Save/Load keep writing `maps/*.toml`; `PLAY` needs no file, so iterate
  first and save when it plays well.
- Gating is unchanged: everything build-side is `#[cfg(feature =
  "map-editor")]`; a release build has the sidebar with the HUD only and
  no toggle. `--editor` still works and simply starts in Build.

## Code seams

Small, and almost all presentation-side:

| Where | Change |
| --- | --- |
| `lib.rs` | `SIDEBAR_WIDTH: i32 = 160` (5 x `OBSTACLE_GRID_SIZE`) and a `Layout { field: Rectangle, sidebar: Rectangle }` with `Layout::for_field(w, h)`. Layout, not a knob - it belongs here, not in `tuning.rs`. `DEFAULT_SCREEN_WIDTH/HEIGHT` keep their value and become the default *field* size. |
| `main.rs` | Window = `field + SIDEBAR_WIDTH`. `Game::init`/`update`, `RippleFx::load`, `ground::build`, `scene_target` all get the **field** size - no simulation change. The `Driver` switch and Build-mode mouse handling live here. |
| `game.rs::render` | `scene_target` is already a separate render texture blitted with `blit_offset` (camera shake); add `layout.field` origin to that offset. `screen_to_ripple_uv` uses field dims. The post-composite pass (debug overlays, banners, flash) draws field-relative: a `Camera2D` with `offset = field origin`, or the origin added to the few `draw_rectangle(0, 0, w, h)` calls. The HUD text block moves out. |
| `hud.rs` (new) | `HudModel` (plain numbers gathered from `Game`) and `draw_sidebar(d, layout.sidebar, &HudModel, &HudTextures)`. `HUD_*` constants and `hud_number_color` move here. |
| `editor.rs` | Chrome rects take `&Layout`: `toolbar_button_rect(layout, i)` (2x2), `palette_icon_rect(layout, i)` (3 columns), `point_on_ui = layout.sidebar.contains(mouse)`, cursor cell from `mouse - field origin`. Hamburger removed. Map settings steppers are new. |
| `site/src/pages/index.astro` | Canvas `min(100vw, 1440px)`; the tuning panel is unaffected. |
| `devserver.rs` screenshots | `after_render` reads the presented frame; a screenshot now includes the sidebar, which is what a QA eye wants. `Layout` gives it the field rect if a tool ever needs to crop. |

Everything under `simulation/`, `battlefield.rs`, `map.rs`, `maplint.rs`,
`pathfind.rs` and both probes stay untouched: the field is still 1280x720
with its origin at (0,0).

## Small screens

1440x720 does not fit a 1366x768 laptop. Two answers, in order:

1. `--resolution 1120x630` (35 x ~19.7 cells) already works today and
   would keep working; maps simply have less room.
2. Later, if wanted: composite the whole window into one render texture
   and blit it to the real window at an integer scale (or letterboxed
   fit). That is the standard pixel-game answer, it keeps the art crisp,
   and it would also give the web canvas an aspect-preserving scale
   instead of the current non-uniform CSS stretch. Not needed for the
   sidebar itself.

## Alternatives considered

- **Bottom bar** (window 1280x844): the 20-icon palette fits in one row
  but the toolbar and settings need a second, the HUD becomes one long
  line again, and a 16:9 field over a 124 px band is an awkward 1.52
  window. Workable, just worse for the vertical readouts.
- **Shrink the field** to 40x20 cells with an 80 px band: breaks
  `default.toml` rows 20-22, every seeded baseline and the lint fixtures.
- **Keep overlays**, only tidier: still covers cells, still blocks clicks
  in the builder - the thing the ask rules out.
- **Scale the field down** next to a panel in a 1280x720 window: blurred
  pixel art.
