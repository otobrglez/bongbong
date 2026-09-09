"""Tall grass for `static/nature_sheet.png`.

Drawn from scratch on the Puny Palette rather than lifted from a pack: the
reference art (ninjikin's GRASS+, see docs/grass-improvements.md) is 100%
off this game's palette - its greens sit around hue 100 and swing to teal
174 in shadow, against Puny World's yellow-green - so its pixels would read
as a different game pasted on top. The *shapes* are the useful part, and
those are what this reproduces.

Two things about the colours are worth knowing before editing them:

1. **A clump is darker than the field, not the same green as it.** The
   live ground is `#619541` (a *retinted* copy - tools/retint_ground.py
   shifts the grass band +17 hue, and it was made after `punypalette` was
   sampled, which is why the palette's GREEN_LT `#85A643` is the yellower
   pristine value). GREEN_DK `#5F914B` lands within ~11 RGB units of that
   live ground, and building blades out of it was the first attempt: they
   were almost invisible. Standing grass is a dense mass that shades
   itself, so the body is GREEN_DARKEST and only the tips catch light -
   which is also what the reference gif shows, dark tufts over a lighter
   field.
2. **Green is allowed here.** `just check-sheets` bans GREEN_* on walls,
   props and blasts because a green pixel on a manufactured object reads as
   terrain showing through. Vegetation is the exception the rule always
   implied, and this sheet is the only one that takes it.

**Density: one design pixel here is one *sheet* pixel, not two.** This is
the one sheet drawn at a scale of its own - `grass_scale` is 2.0, so a
sheet pixel already covers a 2x2 screen block, exactly like a tank's. The
walls sheet needs its own 2px block grid because obstacles draw at 1:1;
doing the same here stacks the two and makes grass twice as chunky as
everything around it, which is what it was until the trees landed beside it
and the difference became obvious on screen. Blades are two sheet pixels
wide at the base and taper to one at the tip - the same apparent weight as
before, with a tip that is actually a tip.

Run: SPRITE_OUT=static python3 tools/spritegen/gen_grass.py
"""

from PIL import Image
import os, random, sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from punypalette import (
    GREEN_BRIGHT, GREEN_DARKEST, GREEN_DK, GREEN_MD, GREEN_SHADE,
    PUNY_PALETTE_ALL,
    SAND_DK,
)

S = 32                      # sheet cell, in sheet pixels
GRID = 1                    # 1 design pixel = 1 sheet pixel = 2 screen px
D = S // GRID               # 32 design pixels per cell
OUT = os.environ.get('SPRITE_OUT', 'assets/sprites')
os.makedirs(OUT, exist_ok=True)

SPECIES = 3
VARIANTS = 8

# Blade ramp, darkest at the root to brightest at the tip. Four explicit
# steps rather than computed shades: snap() is nearest-Euclidean over the
# whole palette and a multiplied green happily crosses into another family
# (this is how a sand tone once ended up green - see docs/PALETTE.md).
ROOT = GREEN_DARKEST + (255,)
BODY = GREEN_SHADE + (255,)     # darker than the field, but not a silhouette
MID = GREEN_DK + (255,)
LIT = GREEN_MD + (255,)
TIP = GREEN_BRIGHT + (255,)     # only the very tips catch the light
DRY = SAND_DK + (255,)          # a rare sun-bleached tip, never a whole blade


def blank():
    return Image.new('RGBA', (S, S), (0, 0, 0, 0))


def dpx(img, dx, dy, c):
    """One *design* pixel: a GRID x GRID block, clipped to the cell."""
    if c is None:
        return
    x, y = dx * GRID, dy * GRID
    for oy in range(GRID):
        for ox in range(GRID):
            if 0 <= x + ox < S and 0 <= y + oy < S:
                img.putpixel((x + ox, y + oy), c)


def blade(img, rng, base_x, height, lean, dry=False):
    """One blade, growing up from the bottom edge.

    Leans progressively rather than all at once, so it reads as bending
    under its own weight instead of as a diagonal line.
    """
    x = float(base_x)
    for i in range(height):
        y = D - 1 - i
        if y < 0:
            break
        # Lean accelerates toward the tip.
        x += lean * (i / max(1, height - 1)) ** 1.6
        col = int(round(x))
        t = i / max(1, height - 1)
        if t > 0.90:
            c = DRY if dry else TIP
        elif t > 0.66:
            c = LIT
        elif t > 0.38:
            c = MID
        elif i < 2:
            c = ROOT
        else:
            c = BODY
        dpx(img, col, y, c)
        # The second pixel of the stalk, on the lee side and dark all the
        # way up: the light is on the tips only, and a blade lit down both
        # edges stops reading as a blade. Dropping it above 0.62 is the
        # taper - the whole reason this is authored at one design pixel per
        # sheet pixel rather than two.
        if t < 0.62:
            dpx(img, col + (1 if lean >= 0 else -1), y, ROOT if t < 0.20 else BODY)


def draw_tuft(species, variant, seed):
    """One small tuft - a handful of blades from a common root, not a whole
    clump.

    Density comes from scattering several of these per cell at draw time,
    the way the reference gif does it. Drawing a full clump per sprite was
    the first attempt and the blades merged into a solid dark bar along the
    bottom: there is not room for a dozen separate blades across one cell,
    and without gaps between them the silhouette stops reading as grass.
    """
    rng = random.Random(seed)
    img = blank()

    # Species differ in blade count, height and how far they splay.
    # Heights are deliberately about half a tank: a design pixel here is
    # 2 screen px and a tank is 64 screen px, so 12-20 design px is 24-40
    # screen px.
    # Tuft height and field density are the same trade-off - both decide how
    # much of a tank the field covers - and this is the half that can be
    # spent without making the field sparse. Measured: at these heights,
    # 2 tufts per cell occludes a parked tank ~45%, against the reference
    # gif's ~44% worst case.
    if species == 0:      # fine and upright
        n, lo, hi, lean, spread = 4 + variant // 3, 12, 18, 0.06, 6
    elif species == 1:    # tall and spiky
        n, lo, hi, lean, spread = 3 + variant // 4, 16, 22, 0.04, 4
    else:                 # short and splayed
        n, lo, hi, lean, spread = 5 + variant // 3, 10, 14, 0.15, 8

    root = D // 2 + rng.randint(-4, 4)
    for _ in range(n):
        bx = max(1, min(D - 2, root + rng.randint(-spread, spread)))
        h = rng.randint(lo, hi)
        # Blades splay away from the root rather than all leaning one way -
        # a whole tuft leaning together reads as wind, and the wind is
        # applied at draw time.
        away = 1.0 if bx >= root else -1.0
        blade(img, rng, bx, h, lean * away * rng.uniform(0.5, 1.4), dry=rng.random() < 0.12)
    return img


sheet = Image.new('RGBA', (S * VARIANTS, S * SPECIES), (0, 0, 0, 0))
for sp in range(SPECIES):
    for v in range(VARIANTS):
        cell = draw_tuft(sp, v, 900 + sp * 37 + v * 11)
        sheet.paste(cell, (v * S, sp * S))   # direct copy: exact RGBA

sheet.save(f'{OUT}/nature_sheet.png')
print('nature_sheet.png', sheet.size)
