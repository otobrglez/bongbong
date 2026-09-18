# Desert theme — dust floor, dry scrub

An experiment in what the game looks like with a desert floor instead of a
meadow: pale dust in place of grass, dry scrub in place of the green tufts.
Everything below is the state of this branch; `BONGBONG_THEME=meadow` puts
the generators back on the green sheet, and nothing in the Rust side knows
which theme is live.

![before and after](desert-theme-before-after.png)

## What changed

- **The floor** (`tools/retint_ground.py`, `static/punyworld/`). The live
  Puny World tileset is a retinted copy of the pristine original, as it
  always was; the script now carries two themes and `desert` is the
  default. The pack's grass fill becomes pale, pebbly dust (`#85A643` →
  `#CCB385`), its dirt paths a darker packed-earth road (`#C4B253` →
  `#A08058`), its sand a slightly darker, smoother hardpan (`#C9B266` →
  `#C2A87D`). The colours the game actually draws go through an exact
  table, the rest of the sheet through the hue curve — `docs/GROUND_SPEC.md`
  §1 has the reasoning (the pack's sand and dirt share their edge-dither
  pixels, so a curve alone cannot separate them).
- **Drift patches** (`src/ground.rs`, `docs/GROUND_SPEC.md` §8). New: soft
  blobs of the pack's sand tiles over the open floor, laid at the grid
  vertices by a hashed value noise and resolved through the pack's own
  sand-against-grass corner autotile, so every edge is hand-painted. Never
  beside a road cell. Two `cosmetics` knobs, both `@ Restart`:
  `ground_drift_cover` (fraction of the open floor, 0 turns them off) and
  `ground_drift_scale` (noise pitch in cells). Both are in the PR preview's
  tuning panel.
- **The tall grass** (`tools/spritegen/gen_grass.py`, `static/nature_sheet.png`).
  Three desert species on the same 3 × 8 sheet: bleached bunchgrass
  fountaining out of a dark root (one blade in eight still green), tall
  stalks carrying seed heads, and a low sagebrush clump — grey body, green
  speckle, dark twigs. Dark khaki silhouettes with pale tips, because the
  pale SAND_* steps sit right on the dust's value and straw drawn in straw
  colour vanishes against it. Still on `PUNY_PALETTE_ALL`
  (`just check-sheets` passes); crush, sway, wake, burn and concealment are
  untouched, since only the sheet changed.
- **Rustle flecks** (`src/fx.rs`). A hull crossing tall grass kicks up
  straw-coloured specks (`STRAW_*`, the tuft's own SAND_* steps) instead of
  leaf green. Trees keep `LEAF_*`.

## Not changed, deliberately

- **Trees** stay green: on dust they read as an oasis grove, which is a
  real desert thing. Palms and saguaros would be a sheet of their own
  (`gen_trees.py`, 48 px cells) and a bigger pass than this experiment.
- **Sandbags** are the one prop that loses contrast against dust (sand on
  sand); their outline and front strip still carry them. Worth a look if
  the theme sticks.
- **No runtime switch.** The theme is decided when the sheets are
  generated. If both looks are wanted in one build, the natural shape is a
  per-map `theme` key that picks the two PNGs at load time - the Rust
  side is already theme-agnostic, so that is a loading change, not a
  rendering one.

## Regenerating

```sh
# both default to the desert; BONGBONG_THEME=meadow for the green sheets
python3 tools/retint_ground.py
SPRITE_OUT=static python3 tools/spritegen/gen_grass.py
python3 tools/check_sheets.py
```

Backups of the meadow sheets and their generators, per the art-pass
convention: `static/_backup/pre-desert-*/` and
`tools/spritegen/_backup/pre-desert-*/`.
