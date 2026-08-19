# Ground Layer — grass / road under objects

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

**The live PNG is a retinted copy since the de-green pass (2026-08).** The
pack's own grass fill (`#85A643`, hue ~80°) is a yellow-green and its
dirt-path tiles (`#C4B253`, hue ~50°) bright yellow-khaki — with the ground
covering most of the screen, the whole game read yellow/green even after
the sprite sheets were de-olived (see `docs/PALETTE.md`, "The de-green
pass"). `tools/retint_ground.py` applies a smooth piecewise-linear HSV
curve: grass hues shift toward the pack's *own* deeper tree-canopy green
(`#85A643` → `#619541`, landing next to its `#5E914B` foliage), dirt is
desaturated/darkened toward earth-tan (`#C4B253` → `#B1A567`), and
everything outside hue 40–110° (wood, red/teal roofs, water, greys) is
untouched. The script always reads the pristine original preserved at
`static/punyworld/_original/punyworld-overworld-tileset.png` and writes the
live path, so it's idempotent — tweak its `CURVE` control points and rerun
to iterate; never hand-edit the live PNG or overwrite `_original/`. The
same pass also set `battlefield.rs`'s `FORTRESS_ROAD_SURROUND` to 0 (see
§5 and that constant's doc comment) so the fortress no longer sits in a
merged dirt moat.

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
river/3×seawater — is *not* used: this version has no dirt patches, so
grass cells only ever need the plain "every corner grass" fill variants
(`ground::GRASS_FILL`), not the corner autotile. See §7 if a future pass
wants dirt patches back.

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

- **Dirt patches / a random wandering road** (the first version's
  behavior) could come back as an *additional* source feeding into the
  same `road_cells`/material-grid mechanism §5 now uses, rather than a
  full revert — nothing about the object-driven placement conflicts with
  also rolling some independent random patches, if that texture is wanted
  again. The removed `Material::Dirt`/`ground::DIRT_CORNER` corner-autotile
  table (grass/dirt Wang corners, extracted from the `overworld` wangset)
  would need restoring first — see this doc's git history for the exact
  table.
- **More materials**: the pack's `overworld` wangset has full corner data
  for `sand`(3) and `cliff`(4) against grass too, plus `river`/3×`seawater`
  for water — same extraction method as the removed dirt table (a short
  Python script over the `.tsx`, not hand-transcription).
- **Gameplay effects** (road = speed bonus?) would read
  `ground::GroundGrid` at the tank's position — nothing in `ground.rs`
  currently exposes a "what material is at this world position" query,
  only the resolved tile-id grid used for drawing; that'd be a small
  addition (map world position → grid cell → re-derive `Material` from the
  tile id, or just store `Material` alongside the tile id in `GroundGrid`
  instead of discarding it after resolve).
