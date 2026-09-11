"""Trees for `static/trees_sheet.png` - the two destructible tree species.

Why a sheet of its own rather than rows on `walls_sheet.png` or
`nature_sheet.png`: **the cell is 48px, not 32.** A tree wants to be bigger
than the 32px grid cell it stands in - one cell is a wall tile, and a tree
that size reads as masonry with leaves on it - but its collider must stay
one cell or the nav grid and the map format both change. Drawing a 48px
sprite over a 32px hull is exactly that: the canopy overhangs its cell by
8 screen px on every side, so a grove's crowns interlock instead of
tiling, while the trunk is what a tank actually bumps into.

Density is unchanged. GRID = 2 sheet pixels per design pixel and
`OBSTACLE_SCALE` is 1.0, so one design pixel is two screen pixels - the
same as tanks, walls, props and the ground (CLAUDE.md's asset pipeline).
The cell is 48px because the *tree* is bigger, not because its pixels are.

Layout (`docs/TREES_SPEC.md` has the full map):

    cols 0-11  the three damage stages (pristine -> nearly bare) in each
               of SHIMMER dapple frames: col = frame * 3 + stage
    col  12    terminal stump (never drawn - see Material::visible_stages)
    cols 13-15 the 3-frame burn loop
    rows 0-3   broadleaf variants        (Material::Tree)
    rows 4-7   conifer variants          (Material::Pine)
    row  8     rubble: leaf litter and snapped branches
    row  9     rubble: the same, burnt out

The dapple frames are the tree's whole animation. A tree does **not** sway:
bending the sprite was tried and reads badly on a crown (a blade of grass
can whip, a trunk cannot), so the only thing that moves is where the light
falls through the leaves. Each frame is the *same* canopy with a few
patches stepped one rung up the foliage ramp, and the patches drift a
design pixel or so between frames, so a crown shimmers in place.

Two things about the art worth knowing before editing:

1. **Foliage is built from an explicit four-step ramp, never `mul()`.**
   Scaling a colour and snapping it back crosses palette families - that is
   how a darkened sand tone once came out green (docs/PALETTE.md). The
   greens here are picked by hand, darkest at the shaded rim.
2. **Green is allowed here**, like `nature_sheet.png`: `just check-sheets`
   bans GREEN_* on walls, props and blasts because green on a manufactured
   object reads as terrain showing through. Vegetation is the exception the
   rule always implied.

Run: SPRITE_OUT=static python3 tools/spritegen/gen_trees.py
"""

from PIL import Image
import math
import os
import random
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from punypalette import (
    GREEN_BRIGHT, GREEN_DARKEST, GREEN_DK, GREEN_LT, GREEN_MD, GREEN_SHADE,
    GOLD_BRIGHT,
    RED_DARKEST, RED_DEEP, RED_MD,
    SAND_DK,
    STONE_DARKEST,
    WOOD_ASH, WOOD_DARKEST, WOOD_DEEPER, WOOD_DK,
)

S = 48                      # sheet cell, in sheet pixels
GRID = 2                    # 1 design pixel = 2 sheet pixels = 2 screen px
D = S // GRID               # 24 design pixels per cell
STAGES = 3                  # visible damage stages, mirrors lib.rs
SHIMMER = 4                 # dapple frames per stage, mirrors lib.rs
STUMP_COL = STAGES * SHIMMER
BURN_COL = STUMP_COL + 1
COLS, ROWS = BURN_COL + 3, 10
RUBBLE_VARIANTS = 8         # mirrors lib.rs
OUT = os.environ.get('SPRITE_OUT', 'assets/sprites')
os.makedirs(OUT, exist_ok=True)


def op(c):
    return c + (255,)


# Foliage ramp, shaded rim to sunlit tip. The light in this game comes from
# the upper left (tuning's shadow_dir is +x/+y), so highlights sit up-left
# of each leaf cluster and RIM closes the silhouette down-right.
RIM = op(GREEN_DARKEST)
BODY = op(GREEN_SHADE)
MID = op(GREEN_DK)
LIT = op(GREEN_MD)
SUN = op(GREEN_LT)
SPARK = op(GREEN_BRIGHT)

BARK = op(WOOD_DEEPER)
BARK_DK = op(WOOD_DARKEST)
BARK_LT = op(WOOD_DK)
DRY = op(SAND_DK)

# The same fire ramp gen_walls.py burns wood with, so a burning tree and a
# burning wall wall read as one fire.
SCORCH = op(RED_DARKEST)
FIRE_D = op(RED_DEEP)
FIRE_M = op(RED_MD)
FIRE_C = op(GOLD_BRIGHT)
ASH = op(WOOD_ASH)
SOOT = op(STONE_DARKEST)


# --------------------------------------------------------------------
# Design-pixel canvas
# --------------------------------------------------------------------
# Everything is drawn into a D x D buffer of design pixels and expanded to
# the GRID x GRID blocks at the end, so a stray one-pixel detail is not
# something this file can express - contrast gen_walls.py, which draws in
# sheet pixels and has to snap every primitive onto the block grid by hand.

def buf():
    return [[None] * D for _ in range(D)]


def put(b, x, y, c):
    xi, yi = int(math.floor(x)), int(math.floor(y))
    if 0 <= xi < D and 0 <= yi < D:
        b[yi][xi] = c


def at(b, x, y):
    if 0 <= x < D and 0 <= y < D:
        return b[y][x]
    return None


def disc(b, cx, cy, r, c, mask=None):
    """A filled circle. With `mask`, only where that set has the pixel -
    which is how a highlight stays inside the canopy it belongs to."""
    r2 = r * r
    for y in range(max(0, int(cy - r - 1)), min(D, int(cy + r + 2))):
        for x in range(max(0, int(cx - r - 1)), min(D, int(cx + r + 2))):
            dx, dy = x + 0.5 - cx, y + 0.5 - cy
            if dx * dx + dy * dy <= r2 and (mask is None or (x, y) in mask):
                b[y][x] = c


def disc_clear(b, cx, cy, r):
    r2 = r * r
    for y in range(max(0, int(cy - r - 1)), min(D, int(cy + r + 2))):
        for x in range(max(0, int(cx - r - 1)), min(D, int(cx + r + 2))):
            dx, dy = x + 0.5 - cx, y + 0.5 - cy
            if dx * dx + dy * dy <= r2:
                b[y][x] = None


def line(b, x0, y0, x1, y1, c):
    steps = int(max(abs(x1 - x0), abs(y1 - y0)) * 2) + 1
    for i in range(steps + 1):
        t = i / steps
        put(b, x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, c)


def filled(b):
    return {(x, y) for y in range(D) for x in range(D) if b[y][x] is not None}


def over(*layers):
    out = buf()
    for layer in layers:
        for y in range(D):
            for x in range(D):
                if layer[y][x] is not None:
                    out[y][x] = layer[y][x]
    return out


def to_image(b):
    img = Image.new('RGBA', (S, S), (0, 0, 0, 0))
    for y in range(D):
        for x in range(D):
            c = b[y][x]
            if c is None:
                continue
            for oy in range(GRID):
                for ox in range(GRID):
                    img.putpixel((x * GRID + ox, y * GRID + oy), c)
    return img


# --------------------------------------------------------------------
# Shared structure
# --------------------------------------------------------------------

def outline(b):
    """Close the silhouette with RIM: one design pixel all round, two on
    the shaded down-right side so the crown reads as a dome rather than a
    flat cut-out."""
    mask = filled(b)
    edge = set()
    for (x, y) in mask:
        for (dx, dy) in ((0, -1), (1, 0), (0, 1), (-1, 0)):
            if (x + dx, y + dy) not in mask:
                edge.add((x, y))
                break
    for (x, y) in edge:
        b[y][x] = RIM
    for (x, y) in list(edge):
        for (dx, dy) in ((1, 0), (0, 1), (1, 1)):
            if (x - dx, y - dy) in mask and (x + dx, y + dy) not in mask:
                b[y - dy][x - dx] = RIM


def trunk(under, tx, top, bottom):
    for y in range(top, bottom + 1):
        put(under, tx, y, BARK)
        put(under, tx + 1, y, BARK_DK)
    # Roots flaring into the ground, so the trunk does not end as a stick.
    put(under, tx - 1, bottom, BARK_DK)
    put(under, tx + 2, bottom, BARK_DK)
    put(under, tx - 1, bottom - 1, BARK)


def branches(under, tx, ty, rng, n, reach):
    """The skeleton under the canopy. Always drawn - it is invisible until
    damage thins the foliage over it, which is the whole point: a wounded
    tree shows its own branches rather than an arbitrary new sprite."""
    for i in range(n):
        a = -math.pi / 2 + (i - (n - 1) / 2) * (2.1 / max(1, n - 1)) + rng.uniform(-0.16, 0.16)
        length = reach * rng.uniform(0.68, 1.0)
        ex, ey = tx + math.cos(a) * length, ty + math.sin(a) * length * 0.95
        line(under, tx, ty, ex, ey, BARK)
        if rng.random() < 0.7:
            a2 = a + rng.choice((-0.6, 0.6))
            line(under, ex, ey, ex + math.cos(a2) * length * 0.4,
                 ey + math.sin(a2) * length * 0.4, BARK_DK)


def bite(canopy, cx, cy, r, rng, count, inner=0):
    """Erode the canopy from the rim inward - a tree loses its outside
    first. `inner` punches holes further in for the badly damaged stage."""
    for _ in range(count):
        a = rng.uniform(0, math.tau)
        d = r * rng.uniform(0.62, 1.05)
        disc_clear(canopy, cx + math.cos(a) * d, cy + math.sin(a) * d, rng.uniform(1.3, 2.9))
    for _ in range(inner):
        a = rng.uniform(0, math.tau)
        d = r * rng.uniform(0.0, 0.62)
        disc_clear(canopy, cx + math.cos(a) * d, cy + math.sin(a) * d, rng.uniform(1.1, 2.0))


def flame(b, cx, cy, r, rng):
    disc(b, cx, cy, r, FIRE_D)
    disc(b, cx - 0.2, cy - r * 0.42, r * 0.62, FIRE_M)
    disc(b, cx - 0.3, cy - r * 0.72, r * 0.30, FIRE_C)
    for _ in range(3):
        put(b, cx + rng.uniform(-r, r), cy - r - rng.uniform(0.0, 1.8), FIRE_M)
    for _ in range(2):
        put(b, cx + rng.uniform(-r * 0.6, r * 0.6), cy - r - rng.uniform(1.0, 2.6), FIRE_D)


def char(canopy, rng):
    """Burn what foliage is left.

    Mostly soot and ash, not fire: the flames are drawn on top and are the
    only saturated thing in the cell. A canopy recoloured wholesale to the
    fire ramp stops reading as a tree at all - it becomes a fireball with a
    trunk, which is what the first pass looked like.
    """
    for y in range(D):
        for x in range(D):
            if canopy[y][x] is None:
                continue
            roll = rng.random()
            canopy[y][x] = SOOT if roll < 0.46 else SCORCH if roll < 0.70 else ASH if roll < 0.86 else RIM


def dapple(canopy, cx, cy, r, seed, frame):
    """Sunlight through leaves: a few patches stepped one rung up the
    foliage ramp, drifting between frames.

    The patches are seeded from the tree itself, so every frame lights the
    *same* places - only where they sit moves, which is what makes a crown
    read as light travelling across it rather than as noise. Inside a patch
    only every other pixel steps, on a *position*-keyed dither rather than a
    frame-keyed one: the speckle stays put while the patch slides over it,
    which is how light through leaves actually behaves.
    """
    # One rung only, and never off RIM: the rim is the silhouette, and
    # lifting it eats the crown's shape.
    up = {BODY: MID, MID: LIT}
    rng = random.Random(seed ^ 0x5EED)
    spots = [
        (
            cx + math.cos(a := rng.uniform(0, math.tau)) * (dd := r * rng.uniform(0.10, 0.45)),
            cy + math.sin(a) * dd * 0.9,
            r * rng.uniform(0.18, 0.26),
            rng.uniform(0, math.tau),
        )
        for _ in range(3)
    ]
    phase = math.tau * frame / SHIMMER
    for (sx, sy, sr, sp) in spots:
        px_, py_ = sx + math.cos(sp + phase) * 1.4, sy + math.sin(sp + phase) * 1.1
        r2 = sr * sr
        for y in range(max(0, int(py_ - sr - 1)), min(D, int(py_ + sr + 2))):
            for x in range(max(0, int(px_ - sr - 1)), min(D, int(px_ + sr + 2))):
                dx, dy = x + 0.5 - px_, y + 0.5 - py_
                # A hashed dither, not a parity one: `(x + y) % 2` and its
                # relatives lay a visible diagonal weave across the whole
                # crown, which reads as a texture bug rather than as light.
                if dx * dx + dy * dy > r2 or ((x * 73856093) ^ (y * 19349663)) % 3:
                    continue
                c = canopy[y][x]
                if c in up:
                    canopy[y][x] = up[c]


# --------------------------------------------------------------------
# Broadleaf (Material::Tree)
# --------------------------------------------------------------------
# Bushy and round: a handful of overlapping leaf clusters, each with its
# own highlight, separated by a dark arc along its own shaded side so the
# crown reads as clumps of leaves rather than one green blob.

BROADLEAF = [
    dict(r=9.6, lobes=7, hi=LIT, spark=SPARK, dx=0.0),
    dict(r=8.8, lobes=6, hi=SUN, spark=SPARK, dx=-1.0),
    dict(r=10.2, lobes=8, hi=LIT, spark=SUN, dx=1.0),
    dict(r=9.2, lobes=7, hi=SUN, spark=SPARK, dx=0.5),
]


def broadleaf_lobes(v, rng):
    cx, cy, r = 11.5 + v['dx'], 9.4, v['r']
    lobes = [(cx, cy, r * 0.60)]
    n = v['lobes']
    for i in range(n):
        a = math.tau * i / n + rng.uniform(-0.20, 0.20)
        d = r * rng.uniform(0.38, 0.52)
        lobes.append((cx + math.cos(a) * d, cy + math.sin(a) * d * 0.92,
                      r * rng.uniform(0.44, 0.56)))
    return cx, cy, r, lobes


def broadleaf_canopy(v, lobes, rng):
    b = buf()
    for (lx, ly, lr) in lobes:
        disc(b, lx, ly, lr, BODY)
    mask = filled(b)
    outline(b)
    inner = filled(b) - {(x, y) for (x, y) in filled(b) if b[y][x] is RIM}
    for (lx, ly, lr) in lobes:
        disc(b, lx - lr * 0.24, ly - lr * 0.28, lr * 0.60, MID, inner)
    for (lx, ly, lr) in lobes:
        if ly <= 9.4 + 1.5:
            disc(b, lx - lr * 0.36, ly - lr * 0.40, lr * 0.32, v['hi'], inner)
    # Cluster separation: darken each lobe's own shaded arc back down to
    # BODY, which is what stops the highlights merging into one dome.
    for (lx, ly, lr) in lobes:
        for y in range(D):
            for x in range(D):
                if (x, y) not in inner:
                    continue
                dx, dy = x + 0.5 - lx, y + 0.5 - ly
                dist = math.hypot(dx, dy)
                if lr - 1.15 <= dist <= lr and dx - dy < lr * 0.55:
                    b[y][x] = BODY
    for _ in range(rng.randint(2, 4)):
        (lx, ly, lr) = min(lobes, key=lambda l: l[0] + l[1] + rng.uniform(-3, 3))
        px, py = lx - lr * 0.45, ly - lr * 0.50
        if (int(px), int(py)) in inner:
            put(b, px, py, v['spark'])
    return b, mask


def draw_broadleaf(v, state, seed, frame=0):
    rng = random.Random(seed)
    cx, cy, r, lobes = broadleaf_lobes(v, rng)
    tx = int(cx) - 1
    under = buf()
    branches(under, cx, 12.6, rng, 5, r * 0.92)
    trunk(under, tx, 12, 21)

    if state == 3:                                   # stump, never drawn
        stump = buf()
        trunk(stump, tx, 18, 21)
        disc(stump, tx + 0.5, 18.0, 1.8, BARK_LT)
        put(stump, tx, 18, BARK_DK)
        return to_image(stump)

    canopy, _ = broadleaf_canopy(v, lobes, rng)
    if state == 1:
        bite(canopy, cx, cy, r, rng, 7)
    elif state == 2:
        bite(canopy, cx, cy, r, rng, 10, inner=3)
    elif state >= 4:                                 # burn loop, cols 4-6
        frame = state - 4
        bite(canopy, cx, cy, r, rng, 6, inner=2)
        char(canopy, rng)
        composed = over(under, canopy)
        fires = buf()
        for i in range(2):
            a = math.tau * (i / 2.0) + frame * 0.8
            d = r * (0.26 + 0.24 * ((i + frame) % 3))
            flame(fires, cx + math.cos(a) * d, cy + math.sin(a) * d * 0.9,
                  1.7 + 0.4 * ((i + frame) % 2), rng)
        return to_image(over(composed, fires))
    dapple(canopy, cx, cy, r, seed, frame)
    return to_image(over(under, canopy))


# --------------------------------------------------------------------
# Conifer (Material::Pine)
# --------------------------------------------------------------------
# Spiky and radial: seen from above a conifer is rings of needle sprays
# stepping in toward the crown tip, which is the one part that catches
# full light. Narrower than a broadleaf and much darker, so the two are
# told apart by silhouette before colour.

CONIFER = [
    dict(r=8.8, spokes=(11, 9, 7), dx=0.0),
    dict(r=8.0, spokes=(10, 8, 6), dx=-0.5),
    dict(r=9.4, spokes=(12, 9, 7), dx=0.5),
    dict(r=8.4, spokes=(11, 8, 6), dx=0.0),
]


def conifer_canopy(v, rng):
    cx, cy, r = 11.5 + v['dx'], 10.0, v['r']
    b = buf()
    disc(b, cx, cy, r * 0.78, RIM)
    rings = [(r, v['spokes'][0], RIM), (r * 0.74, v['spokes'][1], BODY), (r * 0.48, v['spokes'][2], MID)]
    for (radius, n, colour) in rings:
        phase = rng.uniform(0, math.tau)
        for i in range(n):
            a = math.tau * i / n + phase
            ox, oy = math.cos(a), math.sin(a) * 0.95
            # A needle spray: a short taper from inside the ring outward.
            for t in range(4):
                f = radius * (0.60 + 0.135 * t)
                width = 1.25 - 0.28 * t
                disc(b, cx + ox * f, cy + oy * f, width, colour)
    outline(b)
    inner = {(x, y) for (x, y) in filled(b) if b[y][x] is not RIM}
    # Shade the down-right half a step darker, the same direction the
    # broadleaf's lobe arcs shade in - without it a conifer is a flat
    # rosette with no sense of a dome under the needles.
    step = {MID: BODY, BODY: RIM}
    for (x, y) in inner:
        if (x + 0.5 - cx) + (y + 0.5 - cy) > r * 0.22:
            b[y][x] = step.get(b[y][x], b[y][x])
    # The crown tip, and only that: a conifer seen from above is dark
    # except where the very top of the spire catches the light. Made small
    # deliberately - a wide pale centre reads as a lamp, not a treetop.
    disc(b, cx - 0.5, cy - 0.9, r * 0.17, LIT, inner)
    disc(b, cx - 0.8, cy - 1.2, r * 0.08, SUN, inner)
    return cx, cy, r, b


def draw_conifer(v, state, seed, frame=0):
    rng = random.Random(seed)
    cx, cy, r, canopy = conifer_canopy(v, rng)
    tx = int(cx) - 1
    under = buf()
    branches(under, cx, 13.0, rng, 4, r * 0.80)
    trunk(under, tx, 13, 21)

    if state == 3:
        stump = buf()
        trunk(stump, tx, 18, 21)
        disc(stump, tx + 0.5, 18.0, 1.6, BARK_LT)
        put(stump, tx, 18, BARK_DK)
        return to_image(stump)

    if state == 1:
        bite(canopy, cx, cy, r, rng, 6)
    elif state == 2:
        bite(canopy, cx, cy, r, rng, 9, inner=3)
    elif state >= 4:
        frame = state - 4
        bite(canopy, cx, cy, r, rng, 5, inner=2)
        char(canopy, rng)
        composed = over(under, canopy)
        fires = buf()
        for i in range(2):
            a = math.tau * (i / 2.0) + frame * 1.0 + 0.4
            d = r * (0.22 + 0.26 * ((i + frame) % 3))
            flame(fires, cx + math.cos(a) * d, cy + math.sin(a) * d * 0.9,
                  1.6 + 0.5 * ((i + frame) % 2), rng)
        return to_image(over(composed, fires))
    dapple(canopy, cx, cy, r, seed, frame)
    return to_image(over(under, canopy))


# --------------------------------------------------------------------
# Rubble (decal.rs)
# --------------------------------------------------------------------
# One row per leftover kind, RUBBLE_VARIANTS columns of rising coverage -
# the same convention gen_walls.py's rubble block uses, so `Decal` picks a
# column from the tile's position hash and nothing here needs to know.

def draw_rubble_tree(variant, seed, charred):
    rng = random.Random(seed)
    b = buf()
    litter = [SCORCH, SOOT, ASH, BARK_DK] if charred else [BODY, MID, RIM, DRY]
    weights = [0.34, 0.26, 0.24, 0.16]
    count = 5 + variant * 6
    for _ in range(count):
        a = rng.uniform(0, math.tau)
        d = rng.uniform(0, 9.5) * (0.55 + 0.45 * variant / (RUBBLE_VARIANTS - 1))
        x, y = 11.5 + math.cos(a) * d, 11.5 + math.sin(a) * d * 0.92
        roll, acc = rng.random(), 0.0
        colour = litter[-1]
        for c, w in zip(litter, weights):
            acc += w
            if roll < acc:
                colour = c
                break
        put(b, x, y, colour)
        if rng.random() < 0.28:
            put(b, x + rng.choice((-1, 1)), y, colour)
    # Snapped branches: a felled tree leaves timber, not only leaves.
    for _ in range(1 + variant // 2):
        a = rng.uniform(0, math.tau)
        d = rng.uniform(1.5, 7.0)
        x, y = 11.5 + math.cos(a) * d, 11.5 + math.sin(a) * d * 0.9
        a2 = rng.uniform(0, math.tau)
        length = rng.uniform(2.0, 5.0)
        line(b, x, y, x + math.cos(a2) * length, y + math.sin(a2) * length,
             SOOT if charred else BARK)
    if variant >= 5:
        disc(b, 11.5, 11.5, 2.2, SOOT if charred else BARK_LT)
        disc(b, 11.5, 11.5, 1.1, BARK_DK)
    return to_image(b)


# --------------------------------------------------------------------
# Sheet
# --------------------------------------------------------------------

sheet = Image.new('RGBA', (S * COLS, S * ROWS), (0, 0, 0, 0))


def place(cell, col, row):
    sheet.paste(cell, (col * S, row * S))   # direct copy: preserves exact RGBA


def emit(draw, v, row, base):
    # The seed depends on the stage, never on the dapple frame: every frame
    # has to be the same tree with the light in a different place.
    for stage in range(STAGES):
        for frame in range(SHIMMER):
            place(draw(v, stage, base + stage * 11, frame), frame * STAGES + stage, row)
    place(draw(v, 3, base + 3 * 11), STUMP_COL, row)
    for b in range(3):
        place(draw(v, 4 + b, base + (4 + b) * 11), BURN_COL + b, row)


for r, v in enumerate(BROADLEAF):
    emit(draw_broadleaf, v, r, 1300 + r * 37)
for r, v in enumerate(CONIFER):
    emit(draw_conifer, v, 4 + r, 1500 + r * 37)
for c in range(RUBBLE_VARIANTS):
    place(draw_rubble_tree(c, 1700 + c * 13, False), c, 8)
    place(draw_rubble_tree(c, 1800 + c * 13, True), c, 9)

# The block grid is guaranteed by construction (every primitive writes into
# the design-pixel buffer, which only expands to whole GRID x GRID blocks),
# so unlike gen_walls.py there is no pixelate() pass to confirm it.
sheet.save(f'{OUT}/trees_sheet.png')
print('trees_sheet.png', sheet.size)
