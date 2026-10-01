# Props sheet spec — `static/props_sheet.png` and `static/barrel_explosion.png`

The three destructible props (sandbags, oil barrels, fences —
docs/sandbags-barrels-fences.md) share the walls' 32px obstacle grid, hull
and draw path (`obstacle.rs`) but are discrete objects rather than tiling
wall tiles, so they live on their own sheet. Both files are generated
(`tools/spritegen/gen_props.py`, `tools/spritegen/gen_barrel_explosion.py`),
never hand-edited; regenerate with

```
nix-shell -p "python3.withPackages (ps: [ps.pillow])" \
  --run "SPRITE_OUT=static python3 tools/spritegen/gen_props.py && \
         SPRITE_OUT=static python3 tools/spritegen/gen_barrel_explosion.py"
```

## 1. Files

| File | Size | Grid | Drawn at |
|---|---|---|---|
| `props_sheet.png` | 128x320 | 4 cols x 10 rows of 32x32 | `OBSTACLE_SCALE` (1:1, like walls) |
| `barrel_explosion.png` | 768x320 | 12 cols x 5 rows of 64x64 | `scorch_scale` (2.0 default); only row 1 is drawn |

RGBA, no padding, nearest-neighbour sampling. Slice `x = col*cell, y =
row*cell`. Every non-transparent pixel is on the Puny Palette
(docs/PALETTE.md); smoke and puddles use palette greys/black with reduced
alpha, which the palette check ignores by design.

Props are drawn on a 16x16 "macro pixel" canvas and upscaled 2x with
NEAREST — pixel-for-pixel what `gen_walls.py`'s `pixelate()` does to the
walls sheet, so a prop has the same 2px chunkiness as a wall or a tank. Do
not pixelate the sheet again. The explosion is drawn at native 64px and
shown at scale 2, which gives the same density.

## 2. Sheet map — `props_sheet.png`

Rows = material x variant, columns = damage stage (`Obstacle::col()`:
`((1 - health/max) * visible_stages) as i32`, clamped). `row = row_base +
variant`, except fences: `row_base + variant*2 + axis`.

| Rows | Material | `row_base` | Variants | Valid cols | Stages |
|---|---|---|---|---|---|
| 0–2 | Sandbag | 0 | straight row / staggered wall / heaped pile | 0–2 | intact, torn (slits + spilled sand, two sagging bags), collapsed (top course gone, flattened, spill mounds) |
| 3–4 | Barrel | 3 | rusty red drum with band / grey drum with hazard rim and teal bung | 0–3 | intact, dented (dents, rust, small puddle), critical (big puddle, cracked lid, hot rim), **col 3 = lit fuse** |
| 5–6 | Fence, wooden | 5 | horizontal (row 5), vertical (row 6) | 0–1 | intact, damaged (two pickets gone, one leaning, broken rail, splinters) |
| 7–8 | Fence, wire | 5 | horizontal (row 7), vertical (row 8) | 0–1 | intact, damaged (a hole torn in the mesh, loose wire ends) |
| 9 | Oil trail (`PROPS_OIL_ROW`) | – | four puddle variants, picked by position hash | 0–3 | not an obstacle: the `kind = "oil"` ground cell a fire runs along (`obstacle::draw_oil_cell`), drawn under everything that stands |

14 of 40 cells are blank; never sample them.

**The two barrel liveries are the two drum kinds** (`obstacle::Drum`,
docs/barrel-explosion-variety.md section B): row 3, the red drum, is oil
and leaves a burning pool; row 4, the grey hazard drum, is fuel, goes off
harder and launches when another blast sets it off. `Obstacle::variant`
for a barrel *is* its drum; a map pins one with `kind = "barrel", drum =
"oil" | "fuel"` and otherwise the roll at spawn decides. The terminal state (flattened
sandbag, detonated barrel, fence stubs) is never drawn — an obstacle is
despawned the frame it dies, same as walls.

The lit-fuse column (`PROPS_BARREL_LIT_COL`) is drawn whenever
`Obstacle::fuse` is armed, whatever the health; the renderer adds a pulsing
additive glow on top (`blast::draw_fuse_glow`).

A fence's axis is a render-time choice, not map data: `obstacle::fence_axis`
picks the axis with more fence neighbours, horizontal for a lone tile or a
corner. Vertical rows are the transposed horizontal cells.

### Per-item notes

- **Sandbags** fill the cell edge to edge horizontally (bags cross the side
  edges) so a line of them reads continuous, with a one-pixel darker gap
  colour under every course and a darker "front strip" under the lowest
  bag of each column — the tilt cue every prop uses (`front_strip`).
- **Barrels** are a 12-macro-pixel drum: side ellipse, darker rim, lighter
  lid, a highlight at ten o'clock and a bung; the runtime drop shadow is
  its only grounding. Puddles spread right/down from the drum.
- **Fences** keep posts at both ends of the tile (x 1–2 and 13–14) and run
  rails edge to edge so tiles join; the wire mesh is a single-pixel checker
  so the ground shows through.

The entity-level drop shadow (`draw_obstacle_shadow`, the same source rect
tinted black and offset) is what gives a lone prop its footing; nothing is
baked into the art.

## 3. Sheet map — `barrel_explosion.png`

The game draws one row of this sheet, the scorches (row 1). The blast
itself - the flash, the fireball, the smoke, the dust - is composed at
draw time in the effects language (`fireball.rs`, docs/effects.md), in
the four forms the sheet's other rows were drawn in: round, a tall column
that rises fast, a flat, low splash with little smoke and a double core
(`BLAST_SHAPE_ROWS`, picked by `blast::BlastFx::row`). Which form a blast
takes is hashed from its position unless its cause picks one: a shot takes
the flat or the double and throws the fire downrange, a ram or a fire
takes the column, a fuel drum always takes the column, burns hotter and
cleaner and draws bigger (`blast::BlastShape`). Every blast's pace and
size are jittered by its position hash (`barrel_fps_jitter`,
`barrel_scale_jitter`) so two chained blasts never burn in lockstep; drawn
oldest first, a chained blast's flash lands on the earlier fireball and
reads as a second detonation. Rows 0 and 2-4 of the sheet are that
design's reference art, not sampled.

Row 1, cols 0–4: five scorch decals (`SCORCH_VARIANTS`) — three lumpy
black blots with a darker ring of lumps, ten radial streaks, a lighter
crater floor and a few embers, then two lopsided oil splatters with drips.
Picked, mirrored and quarter-turned by the seed, drawn under obstacles at
`scorch_opacity` times the drum's scale (`scorch_fuel_scale` for a fuel
drum), fading in over `scorch_fade_in_seconds`, kept for the round (oldest
dropped past `SCORCH_MAX`). Col 5 (`SCORCH_STREAK_COL`) is the directional
streak a *shot's* blast draws over its blot, pointing right on the sheet
and quarter-turned toward the shot's travel. A ground fire that burns
out leaves one of the blots too, smaller (`Scorch::burn`), so a burnt
trail reads as one scorched streak. The flames on a burning cell are
drawn, not sampled: tongues of fire leaning with the wind
(`pyro::tongues`, `ground_fire_*`). Cols 6–11 of row 1 are not drawn.

## 4. Integration (Rust)

```rust
// obstacle.rs
Material::sheet()            // Sheet::Props for Sandbag | Barrel | Fence
Material::row_base()         // Sandbag 0, Barrel 3, Fence 5 (rows within props_sheet.png)
Material::variants()         // 3, 2, 2
Material::visible_stages()   // 3, 3, 2
Obstacle::row(axis)          // fences: base + variant*2 + axis
Obstacle::col()              // burning → burn loop; fuse armed → PROPS_BARREL_LIT_COL; else the stage
draw_obstacle(d, &ObstacleTextures { walls, props }, obstacle, fence_axis(obstacle, &fence_cells))

// blast.rs
BlastFx::shaped(center, kind, shape)  // form/turn/pace/scale hashed, then shaped by the cause
fireball::compose(fx, lean)  // the flash, fire, smoke and dust as pyro::Shapes (docs/effects.md)
draw_scorch / Scorch::burn   // the scorch row; render/blast.rs: draw_fuse_glow / draw_fire_glow
// obstacle.rs
Obstacle::drum() / Obstacle::fuse_rock(time) / draw_oil_cell / draw_flying_drum
```

Layout constants live in `lib.rs` next to the obstacle block
(`PROPS_COLUMNS`, `PROPS_ROWS`, `PROPS_BARREL_LIT_COL`, `PROPS_OIL_ROW`,
`PROPS_OIL_VARIANTS`, `BARREL_EXPLOSION_TEXTURE_SIZE`,
`BLAST_SHAPE_ROWS`, `SCORCH_ROW`, `SCORCH_VARIANTS`, `SCORCH_STREAK_COL`,
`SCORCH_MAX`); the feel numbers (the fireball's size and pace, jitter,
glow, flash, scorch opacity, the pool, the launch, the fuel blast) are
`group props` rows in `tuning.rs`.

## 5. Palette

| Item | Identity colours |
|---|---|
| Sandbags | `SAND_PALE` top, `SAND_LT` body, `SAND_MD` low, `SAND_DK` gaps/strip |
| Barrel 0 | `RED_DK` side, `RED_DEEP` lid, `RED_MD` highlight, `RED_DARKEST` rim, `WOOD_DK`/`WOOD_LT` band, `WOOD_DEEPER` rust |
| Barrel 1 | `STONE_DK` side, `STONE_MD` lid, `STONE_LT` highlight, `STONE_DARKEST` rim with `GOLD_BRIGHT`/`BLACK` hazard segments, `TEAL_DK`/`TEAL_MD` band |
| Puddle | `BLACK` a210, `STONE_DARKEST` specks, one `BLUE_DK` a120 sheen pixel |
| Wooden fence | `WOOD_DEEPER` posts, `WOOD_PALE` caps/tips, `WOOD_MD` rails, `WOOD_LT` pickets, `WOOD_DK` bases, `WOOD_DARKEST` strip |
| Wire fence | `STONE_DK` posts, `STONE_PALE` caps, `STONE_LT` top wire, `STONE_MD` mesh, `STONE_DARKEST` strip |
| Blast | `RED_DEEP` → `RED_MD` → `RED_BRIGHT` → `GOLD_BRIGHT` → `WHITE`; smoke `STONE_MD`/`STONE_DK`/`STONE_DARKEST`/`BLACK` at reduced alpha; debris `STONE_DARKEST`/`WOOD_DEEPER`/`BLACK`; embers `GOLD_BRIGHT`/`RED_BRIGHT`/`RED_DK` |

No greens: objects are not green in this game (only the ground is).
Derived shades go through `mul()` + `snap()`; small factors snap back to
the same colour on this palette, so highlights and shadows are explicit
adjacent steps.

## 6. Known limitations

- The fence axis is a per-frame neighbour heuristic: a line cut down to a
  lone tile flips to horizontal.
- Damage patterns are baked per cell, so two torn sandbag tiles of the same
  arrangement look identical.
- Barrel damage stages mostly show under minigun fire; a player shell
  usually pops an intact barrel outright (`barrel_max_health`).
- The ground-fire loop is one set of three frames for every burning cell;
  the hashed phase and mirror are what keep a lit trail from blinking in
  unison.
