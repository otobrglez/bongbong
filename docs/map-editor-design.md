# Battlefield / map builder — design doc

Status: implemented, except the in-game hamburger entry point (see
"Entering the editor" below). `map.rs` (format), `battlefield::spawn_from_map`,
the `simulation::Game::init`/`update` wiring (walls/road/frog/pickup slots),
`editor.rs` (the `--editor` standalone tool, palette/toolbar/save/load), the
`map-editor` Cargo feature, and `static/ui/eraser.png` are all in place and
building clean (`cargo check --features map-editor`, `cargo check` without
it). The hamburger toggle for entering the editor *from a running round* is
not wired up: `game.rs::render` currently draws the entire frame into an
offscreen `RenderTexture2D` with no call back to the real window surface
visible in the working tree at the time this was built (that pipeline was
mid-edit by unrelated in-progress work), so there was no safe place to hang
a second on-screen draw call without either fighting that WIP or risking a
double-present/flicker bug. Once that rendering pipeline settles, wiring the
hamburger in is a small addition (an overlay hook or icon draw call inside
whatever `game.rs::render`'s final on-screen pass turns out to be) - the
`--editor` standalone path already proves out the rest (`MapEditor`, its
input handling, panel chrome) independent of that.

**Later change (past the original scope below): maps are no longer
optional.** The procedural "HELLO"-fortress player enclosure and the random
obstacle-scatter generator were both removed outright (not just skipped when
a map is present) - `battlefield.rs` no longer has any procedural terrain
generator at all. Every round now loads exactly one `map::MapFile`: `-m`/
`--map <path>` if given, otherwise `maps/default.toml` (`main.rs`'s
`default_map()`) - there is no "no map" state any more, and `Game::map` is a
plain `MapFile`, not `Option<MapFile>`. Health/ammo pickups followed the same
change: a map's `Pickup` cells are now the *only* source of pickups (no
random-corner fallback even when a map places zero of them - the round
simply has none). The sections below still describe the original
"map is optional, everything else falls back to random" design as it was
first built; where later text in this doc says "falls back to random" for
obstacles/pickups, that fallback no longer exists - only the frog still has
one (a map with no `Frog` cell still gets a random near-center placement,
since a round needs exactly one live frog for the protect-objective
mechanic to mean anything).

**Later change #2: player start point + a map-level default enemy count.**
A map can now place one `Start` cell (editor palette icon: a tank sprite,
same fixed representative sprite - `tank::icon_source_rec`, Scout row/col
0 - as the other editor icons, not any particular round's rolled chassis) -
singleton, same "clicking a new cell moves it" convention as the Frog tool
(`MapFile::start_cell`, mirroring `frog_cell`). `Game::init` reads it
directly off `self.map` *before* spawning the player (the player is created
before `battlefield::spawn_from_map` runs, so it can't wait on that
function's output the way the frog fallback does) and uses it as the
player's spawn position; a map with no `Start` cell falls back to
`MapFile::nearest_free_cell` around the battlefield's exact center - an
expanding-ring search (by grid cell, only `Wall` cells block it) that snaps
to the nearest non-wall cell rather than always the literal center point,
so a map with a wall placed at/near center doesn't spawn the player wedged
inside it.
`battlefield::spawn_from_map`'s own per-cell match still has to handle the
`CellObject::Start` variant (the match is exhaustive) but does nothing with
it there - it exists purely as saved position data, not something that
spawns an entity.

`MapFile` also gained a plain `tanks: Option<u32>` field (TOML: a top-level
`tanks = N` alongside `version`, not a cell) - the map's own default enemy
count. Precedence in `Game::init`: `-e`/`--enemies` wins outright when
given; otherwise `map.tanks` if the map set one; otherwise the existing
random `ENEMY_COUNT_MIN..=ENEMY_COUNT_MAX` roll. `#[serde(default)]` keeps
every map file saved before this change (including the ones already in
`maps/`) parsing unchanged as `tanks = None`. Not exposed anywhere in the
editor UI (no button/field for it) - a designer sets it by hand-editing the
saved TOML's `tanks = N` line, the same way `version` itself is never
edited through the UI either.

**Later change #3: a map-level player chassis.** `MapFile` also has a
`tank: Option<TankKind>` field (TOML: a top-level `tank = "titan"` alongside
`version`/`tanks`, not a cell), naming the chassis the player drives on this
map - a corridor map can hand the player a scout, a wide-open one a titan,
without anyone remembering a `--tank` flag. The names are
`tank::TankKind`'s, the single list the CLI's `--tank`, the map key and
`tuning::TANK_NAMES` all spell a chassis from. Precedence in `Game::init`
(`simulation::resolve_player_row`, unit-tested in `player_chassis_tests`):
`--tank` wins outright, then the `player_tank` tuning knob when it isn't its
-1 "unset" default, then `map.tank`, then the existing random
`0..TANK_VARIANTS` roll - which is only *drawn* in that last case, so
choosing a chassis never shifts the seeded RNG stream every later spawn draw
comes out of. `#[serde(default)]` again keeps older map files parsing
unchanged, and like `tanks` it's a hand-edited TOML line rather than an
editor control (the editor preserves whatever the loaded map had when it
saves).

## Goals

- Let a developer hand-author a battlefield layout — walls (per material),
  glass, road, and a single frog — by clicking cells on the existing
  battlefield grid, then save it as a map file.
- Let the game load one of these maps at startup instead of the current
  fully-random layout, so a designed map plays out on the same terrain every
  time.
- Ship this as a dev-only capability first (native binary, gated so it never
  reaches an end user build), but design the data model so it can later be
  exposed to players in the browser build without a format change — only a
  storage-backend change (file on disk → `localStorage`).

## Non-goals (this pass)

- In-browser editor UI, `localStorage` wiring, or the predefined-vs-custom
  map distinction for players — noted under "Future: browser exposure"
  below as the reason certain choices are made now, not built now.
- Placing player/enemy spawn points, battlefield size, or border walls in
  the map. A map only overrides the *interior static terrain* — walls,
  road, the frog, and pickup slots. Border walls (`battlefield::spawn_walls`)
  and enemy spawn positions stay procedural/random on top of whatever
  terrain the map supplies; the player always spawns at the exact screen
  center (there is no fortress/enclosure any more - see the status note at
  the top of this doc). This keeps the map format small and keeps the
  editor from having to duplicate spawn placement logic.
- Undo/redo, multi-select, copy/paste, rotation - single click-to-place,
  click-to-erase only.
- Maps larger than the default battlefield. The editor canvas is exactly
  the game's normal battlefield size (1280x720 at `OBSTACLE_GRID_SIZE`
  cells) — no scroll/pan/camera.

## Grid & coordinate model

Reuse the existing 32px cell size the game already builds its obstacle and
ground layout on — `OBSTACLE_GRID_SIZE` (`lib.rs`), the same grid
`pathfind::Grid` and `ground::GroundGrid` already use. The editor doesn't
invent a new grid; it just lets a human place into the one that already
exists. Grid is derived, not stored: `cols = width / OBSTACLE_GRID_SIZE`,
`rows = height / OBSTACLE_GRID_SIZE`, from the same fixed 1280x720 default
battlefield size the game uses when no `--resolution` override is given.
Grid lines are not drawn (per requirement); the editor instead highlights
the single cell under the mouse cursor each frame so placement is precise
without visual clutter.

A click maps screen position → `(col, row)` via integer division by
`OBSTACLE_GRID_SIZE`, then places/erases at that cell. No sub-cell
positioning, no free placement — this matches how `battlefield.rs` already
snaps every obstacle to the grid today.

## Placeable objects & toolbar

Bottom-center palette (horizontally centered as one panel, anchored a
small fixed margin above the bottom edge — not full-width), one icon per
object, left to right:

| Icon | Places | Notes |
|---|---|---|
| Brick wall | `Obstacle` material `Brick` | uses existing `walls_sheet.png` tile |
| Iron wall | `Obstacle` material `Iron` | " |
| Wood wall | `Obstacle` material `Wood` | " |
| Glass wall | `Obstacle` material `Glass` | called out separately in the requirements, but it's just the fourth `obstacle::Material` — same placement code path as the other three |
| Road | `ground::GroundGrid` cell → `Road` | see "Road & autotiling" below |
| Frog | single `Frog` placement | see "Frog: singleton enforcement" below |
| Tank (start point) | single `Start` placement | player spawn position - see "Later change #2" above; singleton, same move-on-click behavior as Frog |
| Health pickup | a `Pickup` spawn slot, kind `Health` | see "Pickups: fixed spawn slots" below |
| Ammo pickup | a `Pickup` spawn slot, kind `Ammo` | " |
| Laser pickup | a `Pickup` spawn slot, kind `Laser` | " |
| Minigun pickup | a `Pickup` spawn slot, kind `Minigun` | " |
| Plasma pickup | a `Pickup` spawn slot, kind `Plasma` | " |
| Speed-up pickup | a `Pickup` spawn slot, kind `SpeedUp` | " |
| Eraser | clears whatever occupies the clicked cell | see "Eraser" below |

Each icon is a small button (~48x48px) drawn with the corresponding sprite
already loaded by the game (`Textures.walls`/`Textures.frog_idle`/etc.) —
no new sprite generation needed for the palette itself, except the eraser.
The currently-selected tool is highlighted (border/background tint); the
active tool determines what a grid click does. Clicking a tool button
consumes the click (doesn't also place into whatever cell happens to be
under the palette).

### Panel chrome

The palette is drawn as one rounded rectangle panel behind the row of
icon buttons, not bare icons floating on the battlefield — same "readable
over hectic mid-game visuals" reasoning as the HUD elements `game.rs`
already draws:

- **Shape**: slightly rounded corners (raylib's
  `draw_rectangle_rounded`, small roundness — a few px radius, not a pill
  shape) rather than the razor-rectangle look everything else in the
  editor uses for grid cells.
- **Border**: a thin, slightly-transparent black outline
  (`draw_rectangle_rounded_lines`, ~1-2px) around the panel.
- **Shadow**: a tiny drop shadow — the same rounded rect drawn once more,
  a few px down-right, solid black at low alpha, *behind* the panel body
  (draw shadow copy first, then the real panel on top). Cheap immediate-
  mode fake shadow, not a blur/shader — consistent with the editor having
  no post-processing of its own (unlike `RippleFx`'s shader work in
  gameplay rendering, which doesn't apply here).
- **Fill**: a solid or near-solid dark panel background behind the icons,
  so icon sprites (many of which have transparent/light edges) stay
  legible over any battlefield tile behind them.

This same rounded/bordered/shadowed panel treatment applies to the
top-right Save/Load/Close toolbar (and its Load file-list / Save filename
popups) for visual consistency — one small `draw_panel(rect, ...)` helper
in `editor.rs` used by every panel in the editor rather than one-off
drawing code per toolbar.

### Eraser icon

`bongbong-assets/craftpix-net-741764-free-skill-32x32-icons-for-cyberpunk-game/1 Icons/4/Skillicon4_06.png`
lives outside this repo, in the separate craftpix asset-pack folder — it
can't be loaded from there at runtime (path won't exist on another
machine or in the wasm build, and nothing outside `static/` is bundled by
`dist-workspace.toml`/the emscripten `--preload-file` flag). Per the
project's asset convention (everything `main.rs` loads lives under
`static/`), copy it in once as a plain file copy — no generator script
needed, it's hand-authored third-party art, same treatment as
`static/punyworld/...`:

```
static/ui/eraser.png   (32x32, copied from Skillicon4_06.png)
```

Add a `static/ui/SOURCE.md` noting where it came from (mirrors
`static/punyworld/SOURCE.md`'s convention for attributing third-party art).
Loaded once alongside the other textures, only when the editor is
compiled/entered (see "Dev-only gating" below) so the normal game binary
doesn't pay for a texture it never draws.

### Road & autotiling

`ground::GroundGrid` already computes its Wang-autotile road/grass
transitions from a `Material` grid (`Material::Grass`/`Material::Road`, see
`ground.rs`). The editor doesn't need to know anything about tile-edge
variants — clicking with the Road tool just flips that cell's material to
`Road` (erasing flips it back to `Grass`), and the existing
`GroundGrid` build step recomputes the correct autotile sprite for every
affected cell and its neighbors, exactly as it does for the procedural
ground layer today. The editor calls the same `GroundGrid` construction
path the game uses, just seeded from the map's road cells instead of a
random layout.

### Frog: singleton enforcement

Only one frog may exist on a map, since killing it ends the round. The
Frog tool doesn't refuse a second click — it *moves* the frog: clicking a
new cell while a frog is already placed clears the old cell and places the
frog at the new one, so there's always at most one and the user never
hits an error state. The palette's Frog icon shows a small badge/outline
when a frog is currently placed on the canvas, so it's visually obvious
one already exists.

### Pickups: fixed spawn slots

Today's pickups (`pickup.rs`, `simulation.rs`'s `spawn_pickup`) have no
fixed positions at all — at round start, and again any time the live count
drops below `PICKUP_COUNT`, a kind and a random corner-adjacent position
are rolled fresh. A map's Health/Ammo/Laser/Minigun placements don't
replace that mechanism, they give it fixed candidate *slots* to draw from
instead of rolling a random corner: any number of each may be placed (no
singleton restriction like the frog — a map with zero, three, or ten health
pickups is equally valid), each cell records only its kind, not "currently
occupied," since a slot's occupancy is just whether a live `Pickup` entity
currently sits there.

**Superseded again, past even the "Later change" note at the top of this
doc:** at round start every placed slot spawns its pickup unconditionally,
with no clearance check at all (`simulation::spawn_pickup_at`) — a
map-placed pickup is a deliberate placement, same as the frog, so it's
honored exactly rather than silently skipped for sitting close to a wall.
An earlier version of this feature *did* apply the old random-placement's
clearance check here too, which meant pickups placed near a dense wall
cluster (the kind a real hand-authored map tends to have) would often just
silently fail to spawn - not what "map represents placeholders for health
and ammo spawning" was supposed to mean. When a pickup is collected and the
live count drops below the slot count, the top-up re-spawn draws from the
map's slot list (an empty slot, picked at random among currently-unoccupied
ones), so a designed map's pickups keep reappearing exactly where they were
placed. Maps with no pickup slots at all simply have no pickups — this only
activates when
the map actually places at least one.

### Eraser

Clicking the Eraser tool over any occupied cell (wall, glass, road, or
frog) clears that cell back to empty/grass. Clicking an already-empty cell
is a no-op. No confirmation — single-cell, single-undo-step-equivalent
actions are cheap enough that a confirm dialog would just be friction (and
there's no undo to fall back on if a click was a misclick, so the cost of
a wrong erase is "click the tool again," not data loss).

## Toolbar: save / load / close

Separate row from the object palette — top-right corner of the screen, three
buttons:

- **Save** — opens a single-line text input (defaulted to the current map's
  filename if one is loaded, otherwise empty) for the map name, writes
  `maps/<name>.toml`. `maps/` is created if it doesn't exist. Overwriting an
  existing file is allowed without confirmation (dev tool, not a player
  facing save slot).
- **Load** — opens a simple vertical list of every `maps/*.toml` file
  found, click one to load it into the canvas (replacing whatever's
  currently placed, no confirmation — same reasoning as Eraser above).
- **Close** — exits the editor. Behavior depends on how the editor was
  entered: launched via `--editor`, it closes the window/process (there's
  no game to go back to); entered mid-game via the hamburger button
  (below), it's equivalent to that same hamburger/back icon — switches the
  driver back to `Game` and resumes the paused round. Discards unsaved
  editor changes without prompting either way, consistent with Load's
  no-confirmation stance; worth a one-line reminder in the UI ("unsaved
  changes are lost") rather than a modal, if this turns out to bite anyone
  in practice.

No in-editor "new/clear map" button is needed separately from Load, but
worth adding trivially (clears the canvas without loading a file) since
it's a few lines once Load exists — starting a map from scratch shouldn't
require an empty file on disk first.

## Map file format

TOML, matching the project's existing preference for plain hand-editable
config (`Cargo.toml`, `devenv.nix`, `dist-workspace.toml`). One file per
map under `maps/` (new top-level directory, sibling to `static/`/`tools/`).

```toml
# maps/arena1.toml
version = 1

[cells."3,4"]
kind = "wall"
material = "brick"

[cells."3,5"]
kind = "wall"
material = "glass"

[cells."10,2"]
kind = "road"

[cells."14,9"]
kind = "frog"

[cells."1,1"]
kind = "pickup"
pickup = "health"

[cells."18,11"]
kind = "pickup"
pickup = "ammo"
```

- Keys are `"<col>,<row>"` strings (TOML tables require string keys; this
  matches the preview already shown and approved). Only occupied cells are
  written — empty/grass cells have no entry, so a mostly-empty map stays a
  small file.
- `kind` is one of `"wall" | "road" | "frog" | "pickup"`; `material` is
  present only when `kind = "wall"` (one of
  `"brick" | "iron" | "wood" | "glass"`); `pickup` is present only when
  `kind = "pickup"` (one of `"health" | "ammo"`).
- `version` is a plain integer, bumped only if the schema changes
  incompatibly later — read defensively (reject/warn on an unknown future
  version rather than guessing).

Rust side, in a new `src/map.rs`:

```rust
#[derive(Serialize, Deserialize)]
pub struct MapFile {
    pub version: u32,
    pub cells: HashMap<String, CellObject>, // key: "col,row"
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CellObject {
    Wall { material: obstacle::Material },
    Road,
    Frog,
    Pickup { pickup: pickup::PickupKind },
}
```

`obstacle::Material` and `pickup::PickupKind` already derive nothing
serde-related today — add `Serialize, Deserialize` to both (harmless
addition, no behavior change to existing code) rather than hand-rolling a
parallel string enum for each.
`toml`/`serde` are already project dependencies (`Cargo.toml`); no new
crate needed for the native path (`toml::to_string_pretty`/`toml::from_str`
against `MapFile`, plumbing through plain `std::fs::read_to_string`/
`std::fs::write`).

### `map.rs` also owns the editor↔game bridge

Two small conversions live here, since it's the one module that knows
both the on-disk shape and the in-game grid shape:

- `MapFile::from_placed(cells: &HashMap<(i32,i32), CellObject>) -> MapFile`
  — editor state → savable file.
- `MapFile::cell_lookup(&self) -> HashMap<(i32,i32), &CellObject>` — file →
  something `battlefield.rs` can query per-cell while spawning, parsing the
  `"col,row"` string keys back into integers once at load time rather than
  re-parsing per lookup.

## Wiring into the game

### Loading a map to play (`-m`/`--map <path>`)

New `main.rs` CLI flag, alongside the existing `--enemies`/`--resolution`/
`--no-shadows`:

```rust
/// Load a saved map's terrain instead of the default random layout
/// (see docs/map-editor-design.md). Border walls, the player fortress,
/// and enemy spawns stay procedural on top of the map's terrain.
#[arg(short = 'm', long = "map")]
map: Option<PathBuf>,
```

`Game::init` currently always calls `battlefield::scatter_obstacles`
(random layout), rolls a random frog position, and calls `spawn_pickup`
once per corner for the initial pickups (`simulation.rs` ~line 474-523).
Add a `battlefield::spawn_from_map(&MapFile, &mut Physics, &mut World)`
entry point alongside `scatter_obstacles`, and have `Game::init` take an
`Option<&MapFile>` (threaded down from `main.rs`, which reads/parses the
file once at startup and fails fast with a clear error on a missing/
malformed file — a game that can't find the map it was told to load should
not silently fall back to random). When `Some`, spawn exactly the map's
walls/road cells, place the frog at its map cell instead of the random
roll, and spawn the map's pickup slots instead of the corner-based initial
placement (see "Pickups: fixed spawn slots" above for how top-up respawn
also switches to drawing from those slots for the rest of the round); when
`None`, behavior is byte-for-byte what it is today. Enemy spawn sampling
(`sample_clear_position` etc.) already avoids obstacle positions
generically, so map-placed obstacles get the same clearance treatment
random ones do — no special-casing needed there.

### Entering the editor (`--editor [path]`, or in-game via a hamburger button)

Two entry points into the same `MapEditor` driver:

```rust
#[arg(long = "editor")]
editor: bool,
```

`cargo run --features map-editor -- --editor` opens straight into a blank
canvas; combined with the always-on `-m`/`--map` flag (`--editor --map
maps/arena1.toml`) it opens pre-loaded with that map instead - `--editor`
just redirects where an already-parsed `map::MapFile` goes, rather than
`--map` meaning two different things depending on whether `--editor` is
also present. This is the "skip the game entirely" path. (An earlier draft
of this doc sketched `--editor [MAP]` as one combined flag with an optional
positional value; implemented as two plain flags instead, since clap's
"flag with an optional trailing value" pattern needs more ceremony than
just reusing `--map`.)

The second entry point is a small **hamburger icon (☰), top-left corner**,
drawn over the normal game HUD whenever the game is running with the
`map-editor` feature enabled (same gate as `--editor` — see "Dev-only
gating" below; it never exists in a release build). Clicking it pauses the
current round and switches the top-level driver from `Game` to
`MapEditor`, seeded from whatever obstacle/road/frog layout the current
round already has on screen (built once via `MapFile::from_placed`-style
conversion out of the live obstacle/ground/frog state, not a blank
canvas) — an editor entered mid-game becomes "tweak what I'm looking at",
not "start over". `MapEditor` gets its own matching hamburger/back icon in
the same top-left spot to switch back to `Game`, which resumes the paused
round untouched (edits made in the editor session only affect the
in-memory battlefield if the user explicitly hits Save then reloads via
`-m`/`--map`, or a future "apply to current round" action — out of scope
for this pass, noted under Open questions).

Either way, once inside it, `main.rs` drives `MapEditor`'s update/render
loop instead of `Game`'s — same window, same loaded `Textures`, different
top-level driver. This keeps the editor from having to fight the normal
game's input→`simulation::Input` pipeline (mouse clicks on a toolbar are
not tank movement commands) and keeps `simulation.rs`'s "no `RaylibHandle`
dependency" boundary intact — the editor is presentation-layer only, same
category as `game.rs`, and never needs a physics/AI tick since nothing in
it moves.

### Dev-only gating

Ship `--editor` behind a Cargo feature, `map-editor`, off by default:

```toml
[features]
map-editor = []
```

```rust
#[cfg(feature = "map-editor")]
#[arg(long = "editor", num_args = 0..=1, value_name = "MAP")]
editor: Option<Option<PathBuf>>,
```

`cargo watch -x "run --features map-editor"` (or a `just` recipe) for the
dev loop; the plain `cargo run` / the cargo-dist release build
(`dist-workspace.toml`) build without the feature, so `--editor` doesn't
exist and `static/ui/eraser.png`/`src/editor.rs`'s code isn't even compiled
in for anything a player downloads. `-m`/`--map` stays a normal
always-on flag — *loading* a hand-built map to play is a real player-facing
feature already (per the requirements), it's only *authoring* one that's
dev-only for now.

## Future: browser exposure

Not built now, but the choices above are made to keep this cheap later:

- `map.rs`'s `MapFile`/`CellObject` types are plain serde data with no
  filesystem assumptions baked in — the browser build swaps
  `std::fs::read_to_string`/`write` for `serde_json` + `web_sys` (or
  equivalent) `localStorage` calls; TOML vs JSON is a serde format-feature
  switch, not a data-model change.
- The design's "map = interior terrain only, everything else procedural"
  scope keeps a browser-saved custom map small and cheap to store in
  `localStorage`'s per-origin quota.
- Predefined vs. custom maps: ship a small set of hand-authored `maps/*.toml`
  files bundled into the wasm build the same way `static/` already is
  (`--preload-file`), loaded read-only; player-authored maps are the only
  ones that ever write to `localStorage`, keyed separately (e.g. a
  `custom:` prefix) so a predefined map's name can never be shadowed or
  overwritten by a player save. The in-browser editor UI itself (touch/
  click palette, save-to-`localStorage` flow) is a separate follow-up
  design, not covered here.

## Open questions / risks

- **UI toolkit.** The project has no button/text-input widgets today —
  everything drawn is game sprites. The editor needs basic immediate-mode
  buttons (palette icons, Save/Load/Close, a text input for the save
  filename, a scrollable list for Load) built from raylib primitives
  (`DrawRectangle`/`DrawText`/mouse-rect hit-testing) directly in
  `editor.rs`, rather than pulling in a UI crate — consistent with the
  project having no existing UI dependency, but worth flagging as the
  single biggest chunk of genuinely new code this feature needs (nothing
  to reuse from `game.rs`, which only ever draws, never takes input).
- **Wall material vs. decay stage.** `obstacle::Material` walls have a
  decay/rust/burn lifecycle (`obstacle.rs`) driven by gameplay damage over
  time. A freshly-placed editor wall should spawn at each material's
  pristine/undamaged stage — need to confirm `Obstacle`'s constructor
  already defaults there (expected, but worth checking against
  `battlefield::scatter_obstacles`'s own spawn call as the reference).
