"""Bushes and reeds for `static/bushes_sheet.png` - the soft cover.

A bush is not an obstacle: a tank drives through it and flattens it, the
way it does tall grass, and like grass it hides what stands in it
(`grass.rs`, docs/BUSHES_SPEC.md). What it shares with the obstacles is the
art grid: a 32 px cell drawn at 1:1, its own 2x2 blocks baked in, so one
design pixel is two screen pixels like a wall, a tree or a tank. A bush is
wall-sized, smaller than a tree's 48 px.

Layout (docs/BUSHES_SPEC.md):

    rows 0-5   bush, berry bush, juniper, fern, autumn bush, reeds
               (`grass::Bush` order)
    cols 0-3   four variants, green
    cols 4-7   the same four, dry - what a desert map draws

The dry half is the green half recoloured, never redrawn, with the same
one-to-one ramp `gen_trees.py` dries its trees with (`DRY_RAMP`): every
foliage step onto a named sand or wood step, never a computed multiple
(docs/PALETTE.md). The autumn bush is already gold and brown, so drying it
changes only its few green pixels.

Green is allowed here as on the trees and grass sheets: vegetation, not a
manufactured object; `just check-sheets` holds it to the extended palette.

Run: SPRITE_OUT=static python3 tools/spritegen/gen_bushes.py
"""

from PIL import Image
import math
import os
import random
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from punypalette import (
    GOLD_BRIGHT, GOLD_MD, GOLD_PALE,
    GREEN_BRIGHT, GREEN_DARKEST, GREEN_DK, GREEN_LT, GREEN_MD, GREEN_SHADE,
    RED_BRIGHT, RED_MD,
    SAND_DK, SAND_LT, SAND_MD, SAND_PALE,
    TEAL_DARKEST,
    WOOD_ASH, WOOD_DARKEST, WOOD_DEEPER, WOOD_DK,
)

S = 32                      # sheet cell, in sheet pixels
GRID = 2                    # 1 design pixel = 2 sheet pixels = 2 screen px
D = S // GRID               # 16 design pixels per cell
VARIANTS = 4                # mirrors lib.rs BUSH_VARIANTS
SPECIES = 6                 # mirrors grass::Bush
OUT = os.environ.get('SPRITE_OUT', 'assets/sprites')
os.makedirs(OUT, exist_ok=True)


def op(c):
    return c + (255,)


# The trees' foliage ramp, shaded rim to sunlit tip; light from the upper
# left, so highlights sit up-left of each clump.
RIM = op(GREEN_DARKEST)
BODY = op(GREEN_SHADE)
MID = op(GREEN_DK)
LIT = op(GREEN_MD)
SUN = op(GREEN_LT)
SPARK = op(GREEN_BRIGHT)
LEAFY = (RIM, BODY, MID, LIT, SUN)

# A spruce's needles, for the juniper that grows beside spruce stands.
N_RIM, N_BODY, N_MID, N_LIT = op(TEAL_DARKEST), op(GREEN_DARKEST), op(GREEN_SHADE), op(GREEN_DK)

AUTUMN = (op(WOOD_DARKEST), op(WOOD_DEEPER), op(WOOD_DK), op(GOLD_MD), op(GOLD_BRIGHT))
BERRIES = (op(RED_MD), op(RED_BRIGHT))


# --------------------------------------------------------------------
# Design-pixel canvas
# --------------------------------------------------------------------

def buf():
    return [[None] * D for _ in range(D)]


def put(b, x, y, c):
    xi, yi = int(math.floor(x)), int(math.floor(y))
    if 0 <= xi < D and 0 <= yi < D:
        b[yi][xi] = c


def disc(b, cx, cy, r, c, mask=None):
    for y in range(D):
        for x in range(D):
            dx, dy = x + 0.5 - cx, y + 0.5 - cy
            if dx * dx + dy * dy <= r * r and (mask is None or (x, y) in mask):
                b[y][x] = c


def filled(b):
    return {(x, y) for y in range(D) for x in range(D) if b[y][x] is not None}


def rim(b, colour):
    """Close the silhouette in `colour` and return what is inside it."""
    m = filled(b)
    for (x, y) in m:
        if any((x + dx, y + dy) not in m for dx, dy in ((0, -1), (1, 0), (0, 1), (-1, 0))):
            b[y][x] = colour
    return {(x, y) for (x, y) in m if b[y][x] is not colour}


def to_image(b):
    img = Image.new('RGBA', (S, S), (0, 0, 0, 0))
    for y in range(D):
        for x in range(D):
            if b[y][x] is not None:
                for oy in range(GRID):
                    for ox in range(GRID):
                        img.putpixel((x * GRID + ox, y * GRID + oy), b[y][x])
    return img


# --------------------------------------------------------------------
# The species
# --------------------------------------------------------------------
# Every bush stands on the bottom of its cell: the root `grass.rs` draws
# it from is the cell's bottom middle, so the art's foot is the last row.

def leafy(seed, ramp=LEAFY, berries=None, n=5, r=6.2):
    """A round shrub of overlapping leaf clusters, each with its own
    highlight, so it reads as clumps rather than a green ball."""
    rng = random.Random(seed)
    edge, body, mid, lit, sun = ramp
    cx, cy = 7.6, 8.6
    b = buf()
    lobes = [(cx, cy, r * 0.62)]
    for i in range(n):
        a = math.tau * i / n + rng.uniform(-0.3, 0.3)
        d = r * rng.uniform(0.36, 0.5)
        lobes.append((cx + math.cos(a) * d, cy + math.sin(a) * d * 0.85, r * rng.uniform(0.4, 0.52)))
    for lobe in lobes:
        disc(b, *lobe, body)
    inner = rim(b, edge)
    for (lx, ly, lr) in lobes:
        disc(b, lx - lr * 0.25, ly - lr * 0.3, lr * 0.6, mid, inner)
        disc(b, lx - lr * 0.4, ly - lr * 0.45, lr * 0.28, lit, inner)
    for (lx, ly, lr) in lobes[1:3]:
        x, y = int(lx - lr * 0.5), int(ly - lr * 0.6)
        if (x, y) in inner:
            b[y][x] = sun
    if berries:
        for _ in range(6):
            x, y = rng.choice(sorted(inner))
            b[y][x] = berries[0]
            if (x - 1, y - 1) in inner and rng.random() < 0.5:
                b[y - 1][x - 1] = berries[1]
    return b


def juniper(seed):
    """Low, spiky and dark: a creeping star of needle sprays."""
    rng = random.Random(seed)
    cx, cy = 7.6, 9.0
    b = buf()
    for i in range(11):
        a = math.tau * i / 11 + rng.uniform(-0.2, 0.2)
        length = rng.uniform(4.5, 6.4)
        for s in range(6):
            f = length * s / 5
            disc(b, cx + math.cos(a) * f, cy + math.sin(a) * f * 0.8, max(0.6, 1.5 - 0.18 * s), N_BODY)
    inner = rim(b, N_RIM)
    disc(b, cx - 0.8, cy - 0.8, 2.6, N_MID, inner)
    for (x, y) in inner:
        # Hashed, not parity: a parity test lays a diagonal weave.
        if ((x * 7349) ^ (y * 3517) ^ seed) % 5 == 0 and x + y < cx + cy:
            b[y][x] = N_LIT
    return b


def fern(seed):
    """Fronds radiating from a centre - the forest floor's undergrowth."""
    rng = random.Random(seed)
    cx, cy = 7.5, 9.0
    b = buf()
    for i in range(6):
        a = math.tau * i / 6 + rng.uniform(-0.25, 0.25)
        length = rng.uniform(5.0, 6.6)
        for s in range(14):
            f = length * s / 13
            x, y = cx + math.cos(a) * f, cy + math.sin(a) * f * 0.8
            put(b, x, y, MID if s < 6 else BODY)
            if s % 2 == 1 and s < 12:
                for side in (-1, 1):
                    px, py = x + math.cos(a + side * 1.2) * 1.2, y + math.sin(a + side * 1.2) * 1.0
                    if 0 <= int(px) < D and 0 <= int(py) < D and b[int(py)][int(px)] is None:
                        b[int(py)][int(px)] = BODY if (math.cos(a) + math.sin(a)) > 0 else MID
    # A dark edge under each frond, so it lifts off the grass.
    m = filled(b)
    for y in range(D):
        for x in range(D):
            if (x, y) not in m and ((x - 1, y - 1) in m or (x, y - 1) in m):
                b[y][x] = RIM
    return b


def reeds(seed):
    """Cattails: upright blades with brown heads - a bank's cover."""
    rng = random.Random(seed)
    b = buf()
    base = 14
    for i in range(7):
        x = 3 + i * 1.6 + rng.uniform(-0.4, 0.4)
        h = rng.randint(7, 12)
        for y in range(base - h, base):
            xi = int(x + (base - y) * rng.uniform(-0.08, 0.08))
            if 0 <= xi < D:
                b[y][xi] = MID if i % 2 else LIT
                if xi + 1 < D and b[y][xi + 1] is None and i % 2:
                    b[y][xi + 1] = BODY
        if i % 2 == 0:
            top = base - h
            for y in range(top, top + 3):
                b[y][int(x)] = op(WOOD_DEEPER)
            b[top][int(x)] = op(WOOD_DK)
    for x in range(2, 14):
        if b[base][x] is None and rng.random() < 0.7:
            b[base][x] = RIM
    return b


DRAW = [
    lambda s: leafy(s),
    lambda s: leafy(s, berries=BERRIES),
    juniper,
    fern,
    lambda s: leafy(s, ramp=AUTUMN),
    reeds,
]


# --------------------------------------------------------------------
# The dry half
# --------------------------------------------------------------------
# The same ramp gen_trees.py dries its trees with.
DRY_RAMP = {
    TEAL_DARKEST: WOOD_DARKEST,
    GREEN_DARKEST: SAND_DK,
    GREEN_SHADE: WOOD_ASH,
    GREEN_DK: SAND_MD,
    GREEN_MD: SAND_LT,
    GREEN_LT: SAND_PALE,
    GREEN_BRIGHT: GOLD_PALE,
}


def dry(cell):
    out = cell.copy()
    px = out.load()
    for y in range(S):
        for x in range(S):
            p = px[x, y]
            if p[3] and p[:3] in DRY_RAMP:
                px[x, y] = DRY_RAMP[p[:3]] + (255,)
    return out


# --------------------------------------------------------------------
# Sheet
# --------------------------------------------------------------------

sheet = Image.new('RGBA', (S * VARIANTS * 2, S * SPECIES), (0, 0, 0, 0))
for row, draw in enumerate(DRAW):
    for v in range(VARIANTS):
        cell = to_image(draw(4000 + row * 31 + v * 7))
        sheet.paste(cell, (v * S, row * S))
        sheet.paste(dry(cell), ((VARIANTS + v) * S, row * S))

# The block grid holds by construction: every primitive writes design
# pixels, which only expand to whole GRID x GRID blocks.
sheet.save(f'{OUT}/bushes_sheet.png')
print('bushes_sheet.png', sheet.size)
