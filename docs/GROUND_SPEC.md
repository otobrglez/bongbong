# Ground Layer — grass / road under objects / water

**Status: integrated and wired in, screenshot-verified.** `src/ground.rs`
builds this once per round from `simulation::Game::init` — after every
obstacle/wall for the round has been placed, not near the top of `init` —
and `game.rs::render` draws it first, before tread marks/obstacles/tanks.
Purely decorative — no physics body, no gameplay effect.

The layout is deterministic, not random: grass everywhere, then road
painted at exactly two kinds of cell — under every static obstacle/wall
tile (fortress + scattered structures alike) and inside the player
fortress's `B`/`O` glyphs (their hollow interior, not the tiles themselves)
— and nowhere else. This replaced an earlier version that also rolled a
few random dirt patches and walked one random road from edge to edge; both
are gone now in favor of this fully deterministic, object-driven placement
(see §5).

This replaced an earlier from-scratch procedural design (hand-drawn
wavy-edge tiles snapped to Resurrect 64 — see `tools/spritegen/gen_ground.py`,
kept only as historical reference, not part of the live pipeline). That
version worked but looked flat next to hand-painted terrain; this doc now
describes what's actually running: a third-party tileset with its own
pre-built autotile data, used as-is.

---

## 1. Source art

`static/punyworld/punyworld-overworld-tileset.png` — the "Puny World"
overworld tileset (432×1040, 16×16 tiles, 27 columns), third-party,
confirmed usable by the project owner.

**The live PNGs are retinted copies, one per theme.** The pack's own
grass fill (`#85A643`, hue ~80°) is a yellow-green and its dirt-path tiles
(`#C4B253`, hue ~50°) bright yellow-khaki. `tools/retint_ground.py` always
reads the pristine original preserved at
`static/punyworld/_original/punyworld-overworld-tileset.png` and writes one
live file per `map::Theme` (`Theme::ground_texture_path`), so it's
idempotent — tweak a theme and rerun to iterate; never hand-edit a live PNG
or overwrite `_original/`. A map picks its theme with a top-level `theme`
key (docs/desert-theme.md), and `app.rs` draws with that theme's tileset
and its matching tall-grass sheet from `tools/spritegen/gen_grass.py`:

- **`desert`** (`punyworld-overworld-tileset-desert.png`): the grass fill becomes pale, pebbly dust
  (`#85A643` → `#CCB385`, its speck tones kept a step lighter and darker so
  they read as grains and pebbles), the dirt paths a darker packed-earth
  road (`#C4B253` → `#A08058`), and the pack's sand a slightly darker,
  smoother hardpan (`#C9B266` → `#C2A87D`) that `ground.rs` drifts over the
  open floor in soft patches (§8). The colours the game actually draws go
  through an exact table rather than the hue curve, because the pack's
  sand and dirt sit four hue degrees apart and *share* their grass-edge
  dither pixels — a curve cannot send one dark and the other light without
  tearing every dither. The dithers land between the dust and the hardpan,
  which also puts them between the dust and the road.
- **`grass`** (`punyworld-overworld-tileset.png`, the default theme): the de-green pass. A smooth piecewise-linear HSV curve
  shifts grass hues toward the pack's *own* deeper tree-canopy green
  (`#85A643` → `#619541`, landing next to its `#5E914B` foliage),
  desaturates and darkens dirt toward earth-tan (`#C4B253` → `#B1A567`),
  and leaves everything outside hue 40–110° (wood, red/teal roofs, water,
  greys) untouched. With the ground covering most of the screen, the
  pack's own tones read yellow/green even after the sprite sheets were
  de-olived (see `docs/PALETTE.md`, "The de-green pass"); the same pass
  set `battlefield.rs`'s `FORTRESS_ROAD_SURROUND` to 0 (see §5 and that
  constant's doc comment) so the fortress no longer sits in a merged dirt
  moat.

`ground.rs` names its materials after the pack's wangset colours (grass,
sand, dirt paths), not after what a theme paints them as — the only thing
it takes from the theme is whether to drift the sand (`Theme::drifts`).

See `static/punyworld/SOURCE.md` for
the full provenance note, including: no license file was bundled with it,
and it's **deliberately not on the Resurrect 64 palette** (see
`docs/PALETTE.md`) — mechanically snapping it onto R64 was tried and
visibly degraded it (flattened the shading, and collapsed the road and
grass onto nearly the same colour, breaking the one thing a road needs to
do — read as different from the ground around it). This is a documented
exception, not an oversight; recoloring the rest of the game's sheets
*toward* this palette instead, rather than the other way around, is a
possible future direction but out of scope here.

`static/punyworld/punyworld-overworld-tiles.tsx` is the original Tiled
tileset definition, kept for reference — not loaded by the game. Everything
`ground.rs` needs from it has been extracted into the Rust tables described
below.

---

## 2. Why this needed no custom autotile design (unlike the first pass)

The pack ships full Tiled **wangset** metadata — which tile goes where, by
matching terrain against each neighbour — for exactly the materials this
needed. The one this module actually uses:

- `pathways` (type `edge`): dirt/sand/water paths, each tile's 4 edges
  independently labelled "connects" or not.

Parsing this (see §3) turned out to give **complete** coverage for the dirt
road — all 16 possible N/E/S/W edge combinations have a hand-painted tile,
including "no road neighbours at all" (index 0), which the old edge-to-edge
random walk could never produce but the current object-driven placement
routinely does (an isolated obstacle tile with no orthogonal road neighbour
is a real, common case now). No rotation, no fallback tile, no
approximation needed anywhere — every case `ground::build` can produce has
exact source art.

The `overworld` (type `corner`) wangset — grass/dirt/sand/cliff/trees/
river/3×seawater — supplies the **sand-against-grass corner set** the drift
patches use (§8, `ground::SAND_CORNER`): all 14 mixed-corner tiles plus the
flat sand fill, so every patch edge is hand-painted. Its other materials
are unused; grass cells outside a patch take the plain "every corner grass"
fill variants (`ground::GRASS_FILL`).

---

## 3. Road: edge autotile

`ground::ROAD_EDGE`, indexed by a 4-bit mask (`bit3=N bit2=E bit1=S
bit0=W`, 1=road neighbour):

| mask (N E S W) | tileid | mask | tileid | mask | tileid | mask | tileid |
|---|---|---|---|---|---|---|---|
| 0000 | 32 (isolated) | 0100 | 85 | 1000 | 57 | 1100 | 58 |
| 0001 | 87 | 0101 | 86 | 1001 | 60 | 1101 | 59 |
| 0010 | 3 | 0110 | 4 | 1010 | 30 | 1110 | 31 |
| 0011 | 6 | 0111 | 5 | 1011 | 33 | 1111 | 32 (crossroads) |

This table is complete enough to support a genuinely connected road network
of any shape, including branches/crossings — already exercised today, since
a fortress glyph's interior is a solid multi-cell block (mask 1111/32,
"crossroads", in the interior; the various edge masks around its border).

---

## 4. Shading

The ground carries two shading cues, both added 2026-09. They are separate
mechanisms on purpose, and the reason is worth keeping.

**Wall ambient occlusion — a per-cell tint, baked in `build`.** Cells within
`ground_wall_shade_cells` of a wall darken by up to `ground_wall_shade`, on a
smoothstepped *euclidean* falloff (a Chebyshev/box distance gives square
iso-contours, which show up as rectangular banding). Stored in
`GroundGrid::tints` and applied by `draw` in place of `Color::WHITE`, so it
costs no extra draw calls. Baked rather than per-frame because `draw` already
issues one call per cell over the whole screen, and the field never changes
during a round — obstacles are only ever removed.

**Screen-edge vignette — four per-pixel gradient bands, `draw_edge_shade`.**
Drawn immediately after `draw`, so it shades the floor only; tanks and walls
stand in front of it. `ground_edge_shade` sets the darkness at the very edge
and `ground_edge_shade_px` how far it reaches inward.

> **Why the vignette is not a cell tint.** It was, first. A tint is flat
> across a whole 32×32 cell, so on flat-coloured grass any gradient built
> that way stair-steps at every cell boundary regardless of how smooth the
> underlying field is. Near a wall that is fine — the steps land *on the wall
> grid* and read as the edge of a shadow — but out in the open there is
> nothing for them to align with and it reads as banding. Gradient bands
> interpolate per pixel and have no such problem. If you ever want a radial
> vignette rather than four bands, it needs a texture or a shader, not a
> finer grid.

---

## 5. Placement (`ground::build`, called from `Game::init`)

Unlike the first version, placement isn't random at all — every road cell
is derived directly from where the round's static terrain actually ended
up, computed **after** every obstacle/wall for the round has been placed
(fortress + scattered structures + the fortress's `B`/`O` interior), not
near the top of `init` where `ground::build` used to run.

1. Grid: `cols = ceil(width / GROUND_WORLD_TILE) + 1`, same for rows (the
   `+1` matters now — see §5.1 on grid phase).
2. Start all-grass.
3. For each position in the `road_cells: &[Position]` slice `Game::init`
   passes in, mark that cell Road. Today that slice is
   `obstacle_positions` (every wall/brick tile placed this round — fortress
   glyphs and scattered `structures.rs` shapes alike, already collected as
   a flat `Vec<Position>` for clearance-checking purposes) extended with
   `battlefield::spawn_player_fortress`'s `Fortress::road_cells`: the `B`
   and `O` glyphs' hollow interior cells (see `battlefield::
   glyph_interior_cells`, a flood-fill from outside each glyph's own
   bounding box, general enough to find any glyph's enclosed cells without
   hand-picking coordinates), plus a `FORTRESS_ROAD_SURROUND`-wide (1 grid
   cell - "1x" a wall tile's own width) ring around every wall tile (see
   `battlefield::dilate_cells` - a Chebyshev/square dilation of every
   wall-tile grid cell, minus the wall cells themselves). `GLYPH_GAP` (2)
   is wider than the ring (1), so adjacent glyphs' rings stay separate - a
   1-cell grass sliver between neighbouring glyphs, not one continuous
   moat around the word as a whole. A position outside the grid is
   silently ignored.
4. Resolve every cell to a source tile id via §3's table (or a random
   `GRASS_FILL` pick for plain grass) into one flat `Vec<i32>` — this is
   the only per-round autotile computation; `draw` just blits it.

Road placement has **no clearance logic** of its own against the player,
enemies, or anything else — ground is drawn first and everything else
draws on top, so it can never visually conflict with what's above it.

### 5.1 Grid phase: centered, not top-left-aligned

A ground cell `(gx, gy)` is drawn *centered* on world position
`(gx * GROUND_WORLD_TILE, gy * GROUND_WORLD_TILE)`, matching how
`Obstacle::position` places obstacle tiles (a tile center, always an exact
multiple of `OBSTACLE_GRID_SIZE`, which equals `GROUND_WORLD_TILE`). The
first version drew cells top-left-aligned instead (cell `x` spanning
`[x*T, x*T+T)`) — harmless when the ground layer had nothing to align to,
but a real bug once road needed to sit exactly under an obstacle: a
top-left-aligned cell's boundaries are multiples of `T`, while an obstacle
tile's footprint spans `[k*T-T/2, k*T+T/2)` — boundaries at *odd* multiples
of `T/2`, never coinciding with a top-left-aligned cell no matter which
index is chosen. Centering `ground.rs`'s own cells on the same phase as
obstacle tiles fixes this: cell `k`'s span becomes `[k*T-T/2, k*T+T/2)`,
identical to obstacle tile `k`'s footprint, so `(pos.x / T).round()` maps a
world position to the cell that's actually under it, pixel for pixel. The
`+1` on `cols`/`rows` in §5 step 1 accounts for the last cell's own
half-tile overhang past `width`/`height` that centering introduces (see
`ground.rs`'s module doc comment for the full derivation).

---

## 6. Verification done so far

- `cargo build` / `cargo check --all-targets` clean, no warnings.
- `cargo run --bin probe -- --scenario advance --enemies 4 --obstacles 6
  --rounds 10` and a second sweep at `--obstacles 12` — zero panics, same
  pre-existing AI-navigation anomaly rates as before this change (ground is
  purely decorative, so this exercises the surrounding code paths, not the
  visual result itself).
- **In-game screenshot taken and inspected** (native `cargo run`, macOS):
  confirmed grass is the only material anywhere except under objects; the
  road patch inside the `O` reads as one clean connected interior (bounded
  by its glass walls, itself teal/cross-hatched — a wall material's own
  color, not ground bleeding through); the `B`'s two loops are filled the
  same way; the road tiles' edges land exactly flush against the wall
  tiles above them with no half-tile seam, confirming the §5.1 centering
  fix; the `N` glyph and the open field show plain grass only, no road.

---

## 7. Extension notes

- **A random wandering road** (the first version's behavior) could come
  back as an *additional* source feeding into the same
  `road_cells`/material-grid mechanism §5 now uses, rather than a full
  revert — nothing about the object-driven placement conflicts with it.
- **More materials**: the pack's `overworld` wangset has full corner data
  for `dirt`(2) and `cliff`(4) against grass too, plus 3×`seawater` (the
  `river` colour is water now, §9) — same extraction method as the sand
  table in §8 (a short Python script over the `.tsx`, not
  hand-transcription).
- **Gameplay effects** (road = speed bonus?) would read
  `ground::GroundGrid` at the tank's position — nothing in `ground.rs`
  currently exposes a "what material is at this world position" query,
  only the resolved tile-id grid used for drawing; that'd be a small
  addition (map world position → grid cell → re-derive `Material` from the
  tile id, or just store `Material` alongside the tile id in `GroundGrid`
  instead of discarding it after resolve).

---

## 8. Drift patches (`ground::SAND_CORNER`, `ground_drift_*`)

A flat fill with sparse specks reads as a painted floor once it is dust
rather than grass, so the desert theme adds one more decorative layer:
soft patches of the pack's **sand** tiles — the smoother, slightly darker
hardpan under the desert retint — laid over the open floor.

- **Corner autotile, not per-cell noise.** Sand is decided at the grid
  *vertices*: a vertex is sand where a smooth value noise
  (`ground::drift_noise`, hashed lattice points every `ground_drift_scale`
  cells, smoothstep-blended) exceeds `1 - ground_drift_cover`. Each grass
  cell then takes the tile whose four corners match
  (`SAND_CORNER[TL<<3 | TR<<2 | BR<<1 | BL]`), so every edge is one of the
  pack's 14 hand-painted rounded transitions and a patch is a blob a few
  cells across, never a checkerboard.
- **Never beside a road.** A vertex touching a road cell is forced to
  grass: the road tiles carry the plain fill's dithered edge baked in, and
  a road through a patch would show a fringe of the wrong tone along it.
  Walls stand on road cells, so built-up areas are plain dust and the
  patches live in the open, which is where they read.
- **Keyed like `grass_variant`**, by `seed` and the lattice coordinates
  alone — the editor's fixed-seed rebuilds keep every patch in place across
  edits, and a round's floor replays from its seed.
- Only on a theme whose `map::Theme::drifts` says so (`build`'s `drifts`
  argument): on the grass retint the sand is a khaki that would bring back
  the dirt patches the object-driven placement replaced.
  `ground_drift_cover` 0 turns the layer off on the desert too.

Extraction: the mask → tile table came from the `overworld` wangset's
corner wangids, `[N, NE, E, SE, S, SW, W, NW]` with grass = 1 and sand = 3,
reading the corner slots 7/1/3/5 as TL/TR/BR/BL:

| mask (TL TR BR BL) | tile | mask | tile | mask | tile | mask | tile |
|---|---|---|---|---|---|---|---|
| 0001 | 24 | 0101 | 80 | 1001 | 51 | 1101 | 53 |
| 0010 | 22 | 0110 | 49 | 1010 | 79 | 1110 | 52 |
| 0011 | 23 | 0111 | 25 | 1011 | 26 | 1111 | 50 |
| 0100 | 76 | 1000 | 78 | 1100 | 77 | 0000 | `GRASS_FILL` |

---

## 9. Water (`ground::WATER_CHANNEL`, `WATER_SHORE`, `WATER_FRAMES`, `water_*`)

A map paints water one cell at a time (`kind = "water"`, the builder's
Water brush in the GROUND category), exactly like road, and the game treats
it exactly like road: not solid, no nav effect, tanks drive across. What
differs is the picture. One brush draws two things, decided by the shape
the author painted, so the format needs no river/lake distinction:

- **A stream** is any water cell that is not inside a 2x2 block of water —
  a line one cell wide, painted the way a road is. It resolves through the
  pack's `water-paths` **edge** autotile (`WATER_CHANNEL`), the same 4x4
  layout as `ROAD_EDGE` fifteen rows down the sheet and indexed the same
  way (N E S W, 1 = a water neighbour of either kind). The isolated case
  (0000) is tile 351, the rounded pool beside the end caps, where 84 sits
  beside the road's.
- **A lake** is every water cell inside some 2x2 block. Lake cells resolve
  through the pack's `river` **corner** autotile (`WATER_SHORE`, laid out
  like `SAND_CORNER`): a grid vertex is wet where all four cells around it
  are water, so a painted block becomes a pool whose rounded shore runs half
  a cell inside its outline, and a block two wide is a channel one cell
  wide with a shore on each side. A vertex is also wet where three of its
  four cells are water and one of them is a lake cell — that is a stream
  meeting a shore, and it opens the shore into a mouth around the stream
  instead of leaving a lip of grass between them. The two diagonal masks
  (0101/1010) are not in the wangset and this rule never produces them
  (a lake cell's block centre is one of its corners, and wetting the
  opposite corner wets one of the other two); the table holds flat water
  there so a mistake would show as water, never as grass inside a lake.
- **Road wins** over water on the same cell (a wall stands on dirt), and
  road and water never join each other's autotile.
- **Never under a drift**: a vertex beside a water cell does not drift, for
  the same fringe reason as road (§8).

Extraction, same script as §8. `WATER_CHANNEL` from the `pathways` edge
wangset with `water-paths` = 3, slots N/E/S/W:

| mask (N E S W) | tile | mask | tile | mask | tile | mask | tile |
|---|---|---|---|---|---|---|---|
| 0000 | 351 (pool) | 0100 | 352 | 1000 | 324 | 1100 | 325 |
| 0001 | 354 | 0101 | 353 | 1001 | 327 | 1101 | 326 |
| 0010 | 270 | 0110 | 271 | 1010 | 297 | 1110 | 298 |
| 0011 | 273 | 0111 | 272 | 1011 | 300 | 1111 | 299 |

`WATER_SHORE` from the `overworld` corner wangset with `river` = 7, corner
slots read as in §8:

| mask (TL TR BR BL) | tile | mask | tile | mask | tile | mask | tile |
|---|---|---|---|---|---|---|---|
| 0001 | 279 | 0101 | 305 (never) | 1001 | 306 | 1101 | 308 |
| 0010 | 277 | 0110 | 304 | 1010 | 305 (never) | 1110 | 307 |
| 0011 | 278 | 0111 | 280 | 1011 | 281 | 1111 | 305 |
| 0100 | 331 | 1000 | 333 | 1100 | 332 | 0000 | 305 (never) |

**Animation.** Every one of those tiles but 305 carries a four-frame
`<animation>` in the `.tsx` (100 ms each); the frames are scattered over
the sheet (stream tiles step by 108, shore tiles by 81 or 54), so
`WATER_FRAMES` is a table, not an offset, and `build` bakes all four ids
into the cell (`GroundGrid::tiles` is `[i32; 4]` per cell — non-water cells
repeat one id, so `draw` indexes every cell the same way).
`water_frame_seconds` (default 0.14) is the frame time. 305 is a single
flat tone with no animation in the pack (its would-be frames are
byte-identical), which is what the next paragraph is for.

**The current flows down the map.** The pack's frames shimmer in place, so
the direction comes from `ground::draw_current`: short marks in the pack's
own ripple highlight (`#1DCCCB`, a colour the sheet already has and one the
retint leaves alone) drift southward at `water_flow_speed` px/s,
`water_flow_lanes` per column, over every open lake cell (mask 1111) and
along every stream cell joined north or south — inside the stream art's
own water columns (`WATER_CHANNEL_BAND`, source columns 3..13), which open
water contains too, so a lane keeps its column from a lake into the stream
that drains it. Shores, bends and sideways streams get no marks (there is
no room that is reliably water) and keep the shimmer. A lane is keyed by
its column and repeats every `WATER_FLOW_PERIOD_CELLS` (3) cells:
vertically adjacent cells show the same lane at the same moment, so a mark
crosses a cell edge without a jump and a lake reads as one body of water.
2 px blocks, clipped to the cell, hashed with no seed and no RNG — the
builder shows the same water a round will (it drives the animation from
the wall clock; a round uses `Game::time`).

**Placement**: `build` takes `water_cells` beside `road_cells`
(`battlefield::MapSpawn::water_cells` from the map, the builder's own cell
list in `rebuild_ground`). The rules read the same `ground::Layout` through
`ground::WaterLayout` (open water is deep, everything else painted is a
ford, a north/south stream carries the current) — docs/water.md.
