# Palette — Puny Palette (formerly Resurrect 64)

Every generated sprite sheet in `static/` (tanks, shells, walls, damage) plus
the hand-authored `tracks.png` decal draws exclusively from **the Puny
Palette** — a curated set of colours sampled directly from the third-party
Puny World ground-layer tileset (`static/punyworld/`, see
`static/punyworld/SOURCE.md` and `docs/GROUND_SPEC.md`). The shared source
of truth is `tools/punypalette.py`:

```python
PUNY_PALETTE = [ ... ~35 (r, g, b) tuples, named by family ... ]
nearest(rgb)   # closest palette colour by squared RGB distance
snap(rgba)     # nearest(), alpha passed through unchanged
```

Every `tools/gen_*.py` / `tools/spritegen/gen_*.py` generator imports this
module. `tools/gen_damage.py` (no Pillow dependency) imports it too — the
module itself has no image-library dependency, just tuples and a distance
function, so it works standalone.

**This supersedes [Resurrect 64](https://lospec.com/palette-list/resurrect-64)**
(`tools/resurrect64.py`, kept only as historical reference — nothing imports
it anymore). See "Why the palette changed a second time" below for why.

## Why a fixed shared palette at all

Before any of this, each sheet's colours were arbitrary RGB literals chosen
per-sprite (see `static/_backup/pre-resurrect64-*/` for the true originals).
That meant no two sheets were guaranteed to share a single colour, and any
shading math (`mul()`'s darken/lighten, `scorch()`'s burn blend) drifted
further off from whatever the original "designed" palette had been. Snapping
everything onto one fixed set:

- makes every sheet **provably coherent** — a pixel picker anywhere in the
  game shows one of the same palette colours, whether it's a tank hull, a
  shell spark, a burning wall, a tread mark, or the ground underneath.
- makes recolors **auditable**: any generator run can be checked pixel-by-
  pixel against the palette (see "Verifying" below) instead of trusting
  that a hand-picked hex didn't drift.

## Why the palette changed a second time

This has now gone through three states, in order, all kept as concrete
before/after evidence rather than just described:

1. **Arbitrary literals** (`static/_backup/pre-resurrect64-*/`) — no shared
   palette at all.
2. **Resurrect 64**, in two sub-passes:
   - *Muted* (`tools/spritegen/_backup/muted-pass-*/`) — identity colours
     (tank hull bodies, wall material bases) picked from R64's dim/grey/
     dark-plum corner, reasoning that a military-tank chassis shouldn't be
     neon. Read as depressing and grey overall — most of a tank's on-screen
     area is its hull, so a muted hull reads as a muted tank regardless of
     how bright its accent is.
   - *Vibrant* (`tools/spritegen/_backup/pre-punypalette-*/`) — re-picked
     body *and* accent from R64's bright primary/secondary hues instead
     (candy-toy colours, since this is a fun/friendly game not a military
     sim). Looked great in isolation.
3. **Puny Palette** (current) — once the ground layer actually shipped
   (`docs/GROUND_SPEC.md`), the vibrant-R64 tanks and walls sat directly in
   the same scene as Puny World's much softer, painterly terrain for the
   first time, and read as neon plastic next to it — a mismatch that simply
   wasn't visible while the two were developed and reviewed separately.
   Rather than tone the R64 picks down by feel (right back toward the muted
   pass's mistake), the fix was to stop treating "the palette" as a fixed
   external constant (R64) and instead **derive it from the one asset
   everything else now has to sit next to** — sample real colours out of
   Puny World's own buildings/roofs/water/grass and use those directly. See
   "Extraction method" below.

The lesson that carried across both changes: **a palette being technically
"on-palette" says nothing about whether it's the *right* palette for the
scene it ends up in.** Vibrant-R64 was correct by its own rule and still
wrong once real ground art shipped next to it.

## Extraction method (how `tools/punypalette.py` was built)

Not algorithmic quantization — a first attempt at median-cut color
quantization over the whole tileset produced a palette dominated by
water/grass (they cover the most *area* in the sheet), drowning out the
smaller but more useful building/roof/wood colours actually needed for
tanks and walls. Instead: cropped the tileset down to its buildings/props
region specifically (roofs, wood walls, stone walls — the part of the art
with the richest, most game-relevant colour variety), ranked that region's
distinct colours by pixel count, and hand-picked a ramp per hue family
(near-black, stone grey, wood/tan, red, teal, blue, green, gold) from real,
frequently-occurring pixels — the same "identity colours are hand-curated"
principle as the R64 passes, just sourced from this tileset instead of a
generic pixel-art palette. One real gap found this way: **Puny World has no
purple/violet anywhere in its populated region.** Rather than invent one,
the one tank that used to be "the purple one" (`wraith`) was reassigned to
a hue family the source art actually has (mossy stone-grey) — see
`gen_tanks.py`'s roster comment.

## How each sheet uses it

Two different techniques, depending on whether a colour is "identity" (a
specific tank's body/accent, a shell family's fire colour, a wall material's
base tone) or "derived" (anything computed by darkening/lightening/blending):

- **Identity colours are hand-curated**, not auto-snapped — picked by eye
  from the Puny Palette to preserve or improve on each sheet's role/contrast
  intent (e.g. `docs/SPRITESHEET_SPEC.md` §4's tank roster table, or keeping
  all four brick/iron/wood variants on one shared tone per material per
  `docs/WALLS_SPEC.md`). `tools/gen_damage.py`'s five smoke/char palettes are
  the one exception — snapped mechanically via `nearest()` since they're
  plain tonal variants with no per-row "identity" to preserve.
- **Derived colours are snapped after the math.** `mul()` (in `gen_tanks.py`
  and `gen_walls.py`) scales an RGB tuple by a float factor for
  shading/ramps, then calls `snap()` on the result — otherwise a palette
  colour darkened by an arbitrary factor almost never lands back on another
  palette colour. `scorch()` (damage blending in `gen_tanks.py`/
  `gen_walls.py`) does the same after its blend.

Puny World has no true black; the nearest tone is `#252525` (sampled from
the tileset's own darkest shadow pixel), used everywhere the art needs a
near-black (sprite outlines, char/burn colours, dark tread-mark earth).
Genuine white (`#FFFFFF`) *is* in the palette, unlike every other colour —
not sampled, just the literal brightest value, used sparingly for spark
cores/highlights.

## Verifying

Every sheet should have **zero** off-palette opaque/semi-transparent pixels.
Check with Pillow (`pip install pillow` if you don't already have it in your
shell — see the `devenv.nix` note in `CLAUDE.md`'s asset-pipeline section):

```python
from PIL import Image
import sys
sys.path.insert(0, 'tools')
from punypalette import PUNY_PALETTE
palset = set(PUNY_PALETTE)

img = Image.open('static/scifi_tanks_sheet.png').convert('RGBA')
off = sum(1 for (r, g, b, a) in img.getdata() if a != 0 and (r, g, b) not in palset)
print('off-palette pixels:', off)   # should be 0
```

One thing this caught during the *first* (R64) recolor:
`Image.paste(cell, box, cell)` — passing an RGBA image as its own mask —
alpha-composites semi-transparent pixels against the destination instead of
copying them directly, which drifts partial-alpha pixels (smoke, tread-mark
decals) off-palette even though the source cell itself is clean.
`gen_tanks.py` and `gen_shells.py`'s sheet-assembly loops call
`.paste(cell, box)` with no mask arg (a plain copy) ever since, matching the
pattern `gen_walls.py` already used — still correct under the Puny Palette,
no regression there.

## Backups

Both the PNGs and the generator source that produced them are snapshotted
before each recolor pass, as plain file copies (not a build artifact) under
`static/_backup/<label>-<timestamp>/` and
`tools/spritegen/_backup/<label>-<timestamp>/`:

- `pre-resurrect64-<timestamp>/` — the true originals, before any shared
  palette existed.
- `muted-pass-<timestamp>/` — R64, hulls picked from the dim/grey corner.
- `pre-punypalette-<timestamp>/` — R64, vibrant pass (the version that
  shipped right up until the ground layer went in and exposed the clash).

Safe to delete once you're confident in the current palette, or keep
indefinitely for before/after comparison.
