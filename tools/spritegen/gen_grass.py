"""Tall grass for `static/nature_sheet.png`, in the live theme.

Drawn from scratch on the Puny Palette rather than lifted from a pack: the
reference art (ninjikin's GRASS+, see docs/grass-improvements.md) is 100%
off this game's palette - its greens sit around hue 100 and swing to teal
174 in shadow, against Puny World's yellow-green - so its pixels would read
as a different game pasted on top. The *shapes* are the useful part, and
those are what this reproduces.

**Themes.** One sheet per `map::Theme`, paired with the ground tileset
tools/retint_ground.py writes for the same theme; a map picks both with
its `theme` key. Every theme is written by default, BONGBONG_THEME=x one:

  grass   static/nature_sheet.png - the green tufts: fine upright, tall
          spiky, short splayed.
  desert  static/nature_sheet_desert.png - dry scrub for the dust floor.
          Three species: bleached bunchgrass splaying out of a dark root,
          tall stalks carrying seed heads, and a low sagebrush clump - a
          grey-green mound on dark twigs, the one thing in the field that
          is still alive.

Two things about the colours are worth knowing before editing them:

1. **A clump is darker than the field, not the same tone as it.** Standing
   grass is a dense mass that shades itself, so the body of a tuft is a
   dark step and only the tips catch light - which is also what the
   reference gif shows, dark tufts over a lighter field. The desert makes
   this the whole design: the dust floor is `#CCB385`, a value the pale
   SAND_* steps sit right on top of, so a straw-coloured blade vanishes
   against it. Dry grass here is a dark khaki *silhouette* with bleached
   tips, the way scrub actually reads against sand, not straw drawn in
   straw colour. The grass theme's floor is `#619541`, within ~11 RGB
   units of GREEN_DK, which is why its blades are GREEN_DARKEST/GREEN_SHADE
   with only GREEN_BRIGHT at the tip.
2. **Green is allowed here.** `just check-sheets` bans GREEN_* on walls,
   props and blasts because a green pixel on a manufactured object reads as
   terrain showing through. Vegetation is the exception the rule always
   implied, and this sheet is the only one that takes it - the desert
   spends it on the sagebrush and the rare blade that has not died back.

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
    GOLD_PALE,
    GREEN_BRIGHT, GREEN_DARKEST, GREEN_DK, GREEN_MD, GREEN_SHADE,
    SAND_DK, SAND_MD, SAND_PALE,
    STONE_MD,
    WOOD_ASH, WOOD_DARKEST, WOOD_DEEPER, WOOD_DK,
)

S = 32                      # sheet cell, in sheet pixels
GRID = 1                    # 1 design pixel = 1 sheet pixel = 2 screen px
D = S // GRID               # 32 design pixels per cell
OUT = os.environ.get('SPRITE_OUT', 'assets/sprites')
ONLY = os.environ.get('BONGBONG_THEME')
os.makedirs(OUT, exist_ok=True)

SPECIES = 3
VARIANTS = 8


def op(c):
    return c + (255,)


# --- grass ramp, darkest at the root to brightest at the tip. Four
# explicit steps rather than computed shades: snap() is nearest-Euclidean
# over the whole palette and a multiplied green happily crosses into another
# family (this is how a sand tone once ended up green - see docs/PALETTE.md).
ROOT = op(GREEN_DARKEST)
BODY = op(GREEN_SHADE)          # darker than the field, but not a silhouette
MID = op(GREEN_DK)
LIT = op(GREEN_MD)
TIP = op(GREEN_BRIGHT)          # only the very tips catch the light
DRY = op(SAND_DK)               # a rare sun-bleached tip, never a whole blade

# --- desert ramp. The body is the dark khaki of dead grass seen against
# sand; light climbs the blade to a bleached tip. GOLD_PALE is the palest
# straw the palette has and is spent on tips and seed heads only.
D_ROOT = op(WOOD_DARKEST)
D_BODY = op(SAND_DK)
D_MID = op(WOOD_DK)
D_LIT = op(SAND_MD)
D_TIP = op(SAND_PALE)
D_BLEACH = op(GOLD_PALE)        # a rare fully sun-bleached tip
D_LIVE = op(GREEN_DK)           # a rare blade that has not died back
# Sagebrush: dark twigs under a grey-green mound, lit from the upper left
# like everything else in the game (tuning's shadow_dir is +x/+y).
D_TWIG = op(WOOD_DEEPER)
D_LEAF_SHADE = op(WOOD_ASH)
D_LEAF = op(GREEN_SHADE)
D_LEAF_LIT = op(STONE_MD)


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


def blade(img, rng, base_x, height, lean, ramp, dry=False):
    """One blade, growing up from the bottom edge.

    Leans progressively rather than all at once, so it reads as bending
    under its own weight instead of as a diagonal line. `ramp` is
    (root, body, mid, lit, tip, dry_tip).
    """
    root, body, mid, lit, tip, dry_tip = ramp
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
            c = dry_tip if dry else tip
        elif t > 0.66:
            c = lit
        elif t > 0.38:
            c = mid
        elif i < 2:
            c = root
        else:
            c = body
        dpx(img, col, y, c)
        # The second pixel of the stalk, on the lee side and dark all the
        # way up: the light is on the tips only, and a blade lit down both
        # edges stops reading as a blade. Dropping it above 0.62 is the
        # taper - the whole reason this is authored at one design pixel per
        # sheet pixel rather than two.
        if t < 0.62:
            dpx(img, col + (1 if lean >= 0 else -1), y, root if t < 0.20 else body)


GRASS_RAMP = (ROOT, BODY, MID, LIT, TIP, DRY)
DESERT_RAMP = (D_ROOT, D_BODY, D_MID, D_LIT, D_TIP, D_BLEACH)


def draw_tuft(species, variant, seed):
    """One small grass-theme tuft - a handful of blades from a common root,
    not a whole clump.

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
        blade(img, rng, bx, h, lean * away * rng.uniform(0.5, 1.4), GRASS_RAMP, dry=rng.random() < 0.12)
    return img


def seed_head(img, x, y, rng):
    """A ripe seed head on the tip of a stalk: a two-wide knob three deep,
    pale on the lit side, amber in its own shadow. Reads as a head rather
    than a thick blade because it is *wider* than the stalk under it."""
    side = 1 if rng.random() < 0.5 else -1
    for i in range(3):
        yy = y - i
        dpx(img, x, yy, D_TIP if i else D_BLEACH)
        dpx(img, x + side, yy, D_MID if i == 2 else D_LIT)
    dpx(img, x, y - 3, D_BLEACH)


def draw_dry_tuft(species, variant, seed):
    """One desert tuft. Same scatter-at-draw-time contract as the grass
    theme's: a small thing, several per cell, gaps between them."""
    rng = random.Random(seed)
    img = blank()

    if species == 0:
        # Bunchgrass: short blades fountaining out of one root, the
        # strongest splay on the sheet - dead grass does not stand up. Few
        # enough, and spread wide enough, that the dust shows between the
        # stalks: packed tighter they merged into one dark block at the
        # base, which is the failure draw_tuft's comment describes.
        n, lo, hi, lean, spread = 4 + variant // 3, 11, 17, 0.34, 7
        root = D // 2 + rng.randint(-4, 4)
        for _ in range(n):
            bx = max(1, min(D - 2, root + rng.randint(-spread, spread)))
            h = rng.randint(lo, hi)
            away = 1.0 if bx >= root else -1.0
            ramp = DESERT_RAMP
            if rng.random() < 0.12:
                # One blade in eight has not died back yet.
                ramp = (D_ROOT, D_BODY, D_MID, D_LIVE, D_LIVE, D_LIVE)
            blade(img, rng, bx, h, lean * away * rng.uniform(0.6, 1.5), ramp, dry=rng.random() < 0.3)
    elif species == 1:
        # Seed stalks: a few tall, near-vertical stems, each topped with a
        # head. The tallest thing in the field, and the sparsest.
        n, lo, hi, lean, spread = 3 + variant // 4, 15, 20, 0.05, 5
        root = D // 2 + rng.randint(-4, 4)
        for _ in range(n):
            bx = max(2, min(D - 3, root + rng.randint(-spread, spread)))
            h = rng.randint(lo, hi)
            away = 1.0 if bx >= root else -1.0
            k = lean * away * rng.uniform(0.5, 1.4)
            blade(img, rng, bx, h, k, DESERT_RAMP, dry=False)
            # Where the tip landed: the blade's own lean, summed the way
            # blade() walks it.
            x = float(bx)
            for i in range(h):
                x += k * (i / max(1, h - 1)) ** 1.6
            seed_head(img, int(round(x)), D - h, rng)
    else:
        # Sagebrush: a low mound of leaf clusters on a few dark twigs. Built
        # as discs of leaf colour, shaded down-right, so it reads as a bush
        # with volume rather than as more grass.
        cx = D // 2 + rng.randint(-3, 3)
        base = D - 1
        twigs = 3 + variant % 3
        # Twigs first, so the foliage lands over them.
        for i in range(twigs):
            tx = cx + rng.randint(-5, 5)
            th = rng.randint(4, 8)
            lean = rng.uniform(-0.6, 0.6)
            x = float(tx)
            for j in range(th):
                x += lean * (j / max(1, th - 1))
                dpx(img, int(round(x)), base - j, D_TWIG)
        # Leaf clusters: a wide low disc and two or three smaller ones on
        # top and to the sides. Sagebrush is silver-grey with the green
        # only showing in the leaf mass, so the body is grey, the down-right
        # rim its warm shadow, and the green is *speckled* through it
        # rather than painted as a zone - a solid green disc read as a
        # grass-theme bush that had wandered in.
        clusters = [(cx, base - 5, 7, 3)]
        for _ in range(2 + variant % 2):
            clusters.append((cx + rng.randint(-5, 5), base - rng.randint(6, 10), rng.randint(2, 4), rng.randint(2, 3)))
        for (ox, oy, rx, ry) in clusters:
            for dy in range(-ry, ry + 1):
                for dx in range(-rx, rx + 1):
                    r2 = (dx / rx) ** 2 + (dy / ry) ** 2
                    if r2 > 1.0:
                        continue
                    # Ragged edge: skip a few rim pixels so it is not a
                    # clean ellipse.
                    if r2 > 0.7 and rng.random() < 0.35:
                        continue
                    k = (dx / rx) + (dy / ry)
                    if k > 0.8:
                        c = D_LEAF_SHADE
                    elif rng.random() < 0.38:
                        c = D_LEAF
                    else:
                        c = D_LEAF_LIT
                    dpx(img, ox + dx, oy + dy, c)
    return img


# Output names must match `map::Theme::grass_texture_path`.
THEMES = {
    'grass': ('nature_sheet.png', draw_tuft),
    'desert': ('nature_sheet_desert.png', draw_dry_tuft),
}
for name, (filename, draw) in THEMES.items():
    if ONLY is not None and ONLY != name:
        continue
    sheet = Image.new('RGBA', (S * VARIANTS, S * SPECIES), (0, 0, 0, 0))
    for sp in range(SPECIES):
        for v in range(VARIANTS):
            cell = draw(sp, v, 900 + sp * 37 + v * 11)
            sheet.paste(cell, (v * S, sp * S))   # direct copy: exact RGBA
    sheet.save(f'{OUT}/{filename}')
    print(filename, sheet.size, 'theme', name)
