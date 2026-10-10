"""Trees for `static/trees_sheet.png` - the destructible tree species.

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
    cols 16-31 the same sixteen columns again, dry (the desert theme)
    rows 0-3   broadleaf variants        (Material::Tree)
    rows 4-7   conifer variants          (Material::Pine)
    row  8     rubble: leaf litter and snapped branches
    row  9     rubble: the same, burnt out
    rows 10-37 four variants each of spruce, Scots pine, fir sapling,
               birch, willow, date palm and dead snag (Material::Spruce ..
               Material::Snag, in that order)

The dry half is the green half recoloured, never redrawn: every foliage
step maps to one named step of the sand and wood ramps (`DRY_RAMP`) and each
variant loses a few rim clumps, the same ones in every frame and stage, so
its own branches show. A date palm stays green (it grows where the water
is) and a snag has no foliage to dry, so their dry cells are copies.

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
    GOLD_BRIGHT, GOLD_MD, GOLD_PALE,
    RED_DARKEST, RED_DEEP, RED_MD,
    SAND_DK, SAND_LT, SAND_MD, SAND_PALE,
    STONE_DARKEST, STONE_DK, STONE_LT, STONE_PALE,
    TEAL_DARKEST,
    WOOD_AMBER, WOOD_ASH, WOOD_DARKEST, WOOD_DEEPER, WOOD_DK,
)

S = 48                      # sheet cell, in sheet pixels
GRID = 2                    # 1 design pixel = 2 sheet pixels = 2 screen px
D = S // GRID               # 24 design pixels per cell
STAGES = 3                  # visible damage stages, mirrors lib.rs
SHIMMER = 4                 # dapple frames per stage, mirrors lib.rs
STUMP_COL = STAGES * SHIMMER
BURN_COL = STUMP_COL + 1
GREEN_COLS = BURN_COL + 3   # one species row's columns, green
COLS = GREEN_COLS * 2       # then the same again, dry
NEW_SPECIES_ROW = 10        # first row after the rubble, mirrors lib.rs
ROWS = NEW_SPECIES_ROW + 7 * 4
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
# The woodland pass species (rows 10-37)
# --------------------------------------------------------------------
# Each builder returns the layer under the crown (trunk and the branch
# skeleton damage reveals) and the crown itself, plus the crown's centre
# and radius for bite/dapple. Geometry is seeded per variant, not per
# stage, so a wounded tree is the same tree with leaves missing.

def edge_of(b):
    mask = filled(b)
    return {(x, y) for (x, y) in mask
            if any((x + dx, y + dy) not in mask for dx, dy in ((0, -1), (1, 0), (0, 1), (-1, 0)))}


def shade_down_right(b, cx, cy, r, inner, step, k):
    for (x, y) in inner:
        if (x + 0.5 - cx) + (y + 0.5 - cy) > r * k:
            b[y][x] = step.get(b[y][x], b[y][x])


# Spruce: a hard star of tiers, bluer and darker than the conifer - the
# spike of its silhouette, not its colour, is what tells the two apart.
S_RIM, S_BODY, S_MID, S_LIT = op(TEAL_DARKEST), op(GREEN_DARKEST), op(GREEN_SHADE), op(GREEN_DK)

SPRUCE = [
    dict(r=9.6, points=8, dx=0.0),
    dict(r=8.8, points=7, dx=-0.5),
    dict(r=10.2, points=9, dx=0.5),
    dict(r=9.2, points=8, dx=0.0),
]
# A fir sapling is a young spruce: the same build a third smaller.
FIR = [
    dict(r=6.4, points=7, dx=0.0),
    dict(r=6.0, points=6, dx=-0.5),
    dict(r=6.8, points=8, dx=0.5),
    dict(r=6.2, points=7, dx=0.0),
]


def build_spruce(v, rng, seed):
    r, points = v['r'], v['points']
    cx, cy = 11.5 + v['dx'], 10.5
    b = buf()
    for t, (radius, colour) in enumerate(((r, S_RIM), (r * 0.72, S_BODY), (r * 0.46, S_MID))):
        phase = rng.uniform(-0.15, 0.15) + (math.pi / points) * t
        disc(b, cx, cy, radius * 0.55, colour)
        for i in range(points):
            a = math.tau * i / points + phase
            # One branch tip: a wedge narrowing outward.
            for s in range(7):
                f = radius * (0.35 + 0.1 * s)
                w = max(0.55, 1.9 - 0.25 * s) * (radius / r) ** 0.3
                disc(b, cx + math.cos(a) * f, cy + math.sin(a) * f * 0.95, w, colour)
    for (x, y) in edge_of(b):
        b[y][x] = S_RIM
    inner = {(x, y) for (x, y) in filled(b) if b[y][x] is not S_RIM}
    shade_down_right(b, cx, cy, r, inner, {S_LIT: S_MID, S_MID: S_BODY, S_BODY: S_RIM}, 0.3)
    # Needle texture: hashed specks one rung up on the lit half.
    for (x, y) in inner:
        if ((x * 7349) ^ (y * 3517) ^ seed) % 7 == 0 and (x - cx) + (y - cy) < 0:
            b[y][x] = {S_BODY: S_MID, S_MID: S_LIT}.get(b[y][x], b[y][x])
    disc(b, cx - 0.4, cy - 0.6, r * 0.13, S_LIT, inner)
    put(b, cx - 0.8, cy - 1.0, op(GREEN_MD))
    under = buf()
    branches(under, cx, 13.0, rng, 5, r * 0.8)
    trunk(under, int(cx) - 1, 14, 21)
    return under, b, cx, cy, r


# Scots pine: an umbrella of separate needle clumps on bare orange-brown
# limbs. From above you see into it, and the warm bark through the gaps is
# what tells it from a spruce at a glance.
P_BARK, P_BARK_LT = op(WOOD_DK), op(WOOD_AMBER)

SCOTS = [
    dict(r=9.8, clumps=6),
    dict(r=9.0, clumps=5),
    dict(r=10.4, clumps=7),
    dict(r=9.4, clumps=6),
]


def build_scots(v, rng, seed):
    r, clumps = v['r'], v['clumps']
    cx, cy = 11.5, 10.0
    under = buf()
    angles = []
    for i in range(clumps):
        a = math.tau * i / clumps + rng.uniform(-0.3, 0.3)
        angles.append(a)
        d = r * rng.uniform(0.55, 0.75)
        line(under, cx, cy + 1, cx + math.cos(a) * d, cy + math.sin(a) * d, P_BARK)
    for y in range(int(cy), 22):
        put(under, cx - 0.5, y, P_BARK_LT)
        put(under, cx + 0.5, y, P_BARK)
    put(under, cx - 1.5, 21, BARK_DK)
    put(under, cx + 1.5, 21, BARK_DK)
    b = buf()
    spots = [(cx, cy, r * 0.30)]
    for a in angles:
        d = r * rng.uniform(0.62, 0.74)
        spots.append((cx + math.cos(a) * d, cy + math.sin(a) * d * 0.92, r * rng.uniform(0.25, 0.31)))
    for (sx, sy, sr) in spots:
        disc(b, sx, sy, sr, BODY)
        # A ragged rim: a few needle tips poking out, not a halo of specks.
        for k in range(4):
            a = math.tau * k / 4 + rng.uniform(0, 1.2)
            disc(b, sx + math.cos(a) * sr * 0.95, sy + math.sin(a) * sr * 0.95, 0.8, BODY)
    outline(b)
    inner = {(x, y) for (x, y) in filled(b) if b[y][x] is not RIM}
    for (sx, sy, sr) in spots:
        disc(b, sx - sr * 0.3, sy - sr * 0.35, sr * 0.55, MID, inner)
        disc(b, sx - sr * 0.45, sy - sr * 0.5, sr * 0.25, LIT, inner)
    shade_down_right(b, cx, cy, r, inner, {LIT: MID, MID: BODY}, 0.45)
    for (sx, sy, sr) in spots[1:3]:
        if (int(sx - sr * 0.5), int(sy - sr * 0.6)) in inner:
            put(b, sx - sr * 0.5, sy - sr * 0.6, SUN)
    return under, b, cx, cy, r


# Birch: light and airy, small pale lobes, a white trunk with black marks.
# The bright counterpoint at a wood's edge.
B_RIM, B_BODY, B_MID, B_LIT, B_SUN = op(GREEN_SHADE), op(GREEN_DK), op(GREEN_MD), op(GREEN_LT), op(GREEN_BRIGHT)

BIRCH = [dict(r=8.6), dict(r=7.8), dict(r=9.0), dict(r=8.2)]


def build_birch(v, rng, seed):
    r = v['r']
    cx, cy = 11.5, 9.6
    under = buf()
    for y in range(11, 22):
        put(under, cx - 0.5, y, op(STONE_PALE))
        put(under, cx + 0.5, y, op(STONE_LT))
        if y % 3 == rng.randint(0, 2):
            put(under, cx - 0.5 + rng.choice((0, 1)), y, SOOT)
    put(under, cx - 1.5, 21, op(STONE_DK))
    put(under, cx + 1.5, 21, op(STONE_DK))
    for i in range(5):
        a = -math.pi / 2 + (i - 2) * 0.5
        line(under, cx, 12, cx + math.cos(a) * r * 0.8, 12 + math.sin(a) * r * 0.8, op(STONE_LT))
    b = buf()
    lobes = []
    for i in range(12):
        a = math.tau * i / 12 + rng.uniform(-0.3, 0.3)
        d = r * (0.62 if i % 2 else 0.3) * rng.uniform(0.85, 1.1)
        lobes.append((cx + math.cos(a) * d, cy + math.sin(a) * d * 0.9, rng.uniform(2.0, 2.8)))
    for (lx, ly, lr) in lobes:
        disc(b, lx, ly, lr, B_BODY)
    for (x, y) in edge_of(b):
        b[y][x] = B_RIM if (x - cx) + (y - cy) > -3 else B_BODY
    inner = {(x, y) for (x, y) in filled(b) if b[y][x] is not B_RIM}
    for (lx, ly, lr) in lobes:
        disc(b, lx - lr * 0.3, ly - lr * 0.35, lr * 0.6, B_MID, inner)
        disc(b, lx - lr * 0.45, ly - lr * 0.5, lr * 0.3, B_LIT, inner)
    for (lx, ly, lr) in lobes[:3]:
        put(b, lx - lr * 0.5, ly - lr * 0.6, B_SUN)
    # A couple of real holes, so the white limbs show through.
    for _ in range(2):
        a = rng.uniform(0, math.tau)
        disc_clear(b, cx + math.cos(a) * r * 0.35, cy + math.sin(a) * r * 0.35, 0.9)
    return under, b, cx, cy, r


# Willow: a fringed dome of hanging strands, paler at the tips and sagging
# south. The bank tree - a row of them reads as "water is here".
W_RIM, W_BODY, W_MID, W_LIT, W_TIP = op(GREEN_DARKEST), op(GREEN_SHADE), op(GREEN_DK), op(GREEN_MD), op(GREEN_LT)

WILLOW = [dict(r=10.2), dict(r=9.4), dict(r=10.8), dict(r=9.8)]


def build_willow(v, rng, seed):
    r = v['r']
    cx, cy = 11.5, 9.6
    under = buf()
    branches(under, cx, 12.0, rng, 6, r * 0.85)
    trunk(under, int(cx) - 1, 13, 21)
    b = buf()
    # The mass first, so the crown has weight against the grass; the
    # strands are drawn into it and scallop its rim.
    disc(b, cx, cy + 0.6, r * 0.80, W_BODY)
    n = 34
    for i in range(n):
        a = math.tau * i / n + rng.uniform(-0.06, 0.06)
        length = r * rng.uniform(0.78, 1.02)
        lit_side = math.cos(a + math.pi * 0.75) > 0.2     # light from the upper left
        for k in range(12):
            t = k / 11
            f = length * (0.25 + 0.75 * t)
            x, y = cx + math.cos(a) * f, cy + math.sin(a) * f * 0.9 + 1.4 * t * t
            if t < 0.35:
                c = W_MID
            elif t < 0.8:
                c = W_LIT if lit_side else W_MID
            else:
                c = W_TIP if (lit_side and i % 3 == 0) else (W_LIT if lit_side else W_BODY)
            put(b, x, y, c)
    m = filled(b)
    for (x, y) in sorted(m):
        if (x, y + 1) not in m:
            put(b, x, y + 1, W_RIM)
        elif (x + 1, y) not in m and (x - cx) > 0:
            put(b, x + 1, y, W_RIM)
    disc(b, cx - 1.0, cy - 1.2, r * 0.18, W_LIT, filled(b))
    return under, b, cx, cy, r


# Date palm: long fronds arching from one crown point, leaflets along
# each, on a ringed trunk with a lean. The oasis tree.
PALM = [
    dict(fronds=7, lean=1.0),
    dict(fronds=6, lean=-1.0),
    dict(fronds=8, lean=0.5),
    dict(fronds=7, lean=1.5),
]


def build_palm(v, rng, seed):
    lean = v['lean']
    tx, ty = 11.5 + lean, 9.0
    under = buf()
    for k, y in enumerate(range(10, 22)):
        x = tx - lean * (y - 10) / 11
        put(under, x, y, op(WOOD_DK) if k % 2 else BARK)
        put(under, x + 1, y, BARK if k % 2 else BARK_DK)
    put(under, tx - lean - 1, 21, BARK_DK)
    put(under, tx - lean + 2, 21, BARK_DK)
    b = buf()
    fronds = v['fronds']
    for i in range(fronds):
        a = math.tau * i / fronds + rng.uniform(-0.15, 0.15) - math.pi / 2
        length = rng.uniform(9.0, 11.0)
        for k in range(18):
            t = k / 17
            f = length * t
            # Each frond droops as it goes out.
            x = tx + math.cos(a) * f
            y = ty + math.sin(a) * f * 0.85 + 2.6 * t * t
            put(b, x, y, MID if t < 0.6 else BODY)
            if 2 < k < 16 and k % 2 == 0:
                w = 1.6 * math.sin(math.pi * t)
                for side in (-1, 1):
                    px = x + math.cos(a + side * 1.35) * w
                    py = y + math.sin(a + side * 1.35) * w * 0.85 + 0.4
                    put(b, px, py, LIT if side < 0 and math.sin(a) < 0.3 else BODY)
    m = filled(b)
    for (x, y) in m:
        if (x + 1, y + 1) not in m and (x, y + 1) not in m and b[y][x] is BODY:
            b[y][x] = RIM
    disc(b, tx, ty, 1.4, op(WOOD_DK))
    put(b, tx - 0.5, ty - 0.5, op(GOLD_MD))
    return under, b, tx, ty + 1.0, 9.5


# Dead snag: a grey trunk and bare limbs, nothing to burn but wood. The
# "crown" is the limbs themselves, so damage snaps them off.
SNAG_WOOD, SNAG_DK = op(WOOD_ASH), BARK_DK

SNAG = [dict(n=6, reach=8.5), dict(n=5, reach=7.8), dict(n=7, reach=9.0), dict(n=5, reach=8.2)]


def build_snag(v, rng, seed):
    cx, cy = 11.5, 11.0
    under = buf()
    trunk(under, 10, 9, 21)
    for (x, y) in filled(under):
        if under[y][x] == BARK:
            under[y][x] = SNAG_WOOD
    b = buf()
    branches(b, cx, cy, rng, v['n'], v['reach'])
    for (x, y) in filled(b):
        b[y][x] = SNAG_WOOD if b[y][x] == BARK else SNAG_DK
    return under, b, cx, cy - 2.0, v['reach']


def draw_species(build, v, state, seed, frame=0):
    """One cell of a woodland-pass species. `seed` is the variant's, the
    same for every stage and frame; the stage's own wear is seeded on top
    of it, so the same holes open the same way in every dapple frame."""
    rng = random.Random(seed)
    under, canopy, cx, cy, r = build(v, rng, seed)
    wear = random.Random(seed * 31 + state)
    if state == 3:                                   # stump, never drawn
        stump = buf()
        trunk(stump, int(cx) - 1, 18, 21)
        disc(stump, int(cx) - 0.5, 18.0, 1.7, BARK_LT)
        put(stump, int(cx) - 1, 18, BARK_DK)
        return to_image(stump)
    if state == 1:
        bite(canopy, cx, cy, r, wear, 7)
    elif state == 2:
        bite(canopy, cx, cy, r, wear, 10, inner=3)
    elif state >= 4:                                 # the burn loop
        f = state - 4
        bite(canopy, cx, cy, r, wear, 6, inner=2)
        char(canopy, wear)
        composed = over(under, canopy)
        fires = buf()
        for i in range(2):
            a = math.tau * (i / 2.0) + f * 0.9 + 0.2
            d = r * (0.24 + 0.25 * ((i + f) % 3))
            flame(fires, cx + math.cos(a) * d, cy + math.sin(a) * d * 0.9,
                  1.6 + 0.45 * ((i + f) % 2), wear)
        return to_image(over(composed, fires))
    dapple(canopy, cx, cy, r, seed, frame)
    return to_image(over(under, canopy))


SPECIES = [
    (build_spruce, SPRUCE),
    (build_scots, SCOTS),
    (build_spruce, FIR),
    (build_birch, BIRCH),
    (build_willow, WILLOW),
    (build_palm, PALM),
    (build_snag, SNAG),
]
# Which of SPECIES keep their green art on the dry half: the palm grows
# where the water is, and a snag has no foliage left to dry.
EVERGREEN = {5, 6}


# --------------------------------------------------------------------
# The dry half (cols 16-31)
# --------------------------------------------------------------------
# One-to-one onto named steps of the sand and wood ramps - never a
# computed tint, which is how a darkened sand tone once came out green
# (docs/PALETTE.md).
DRY_RAMP = {
    TEAL_DARKEST: WOOD_DARKEST,
    GREEN_DARKEST: SAND_DK,
    GREEN_SHADE: WOOD_ASH,
    GREEN_DK: SAND_MD,
    GREEN_MD: SAND_LT,
    GREEN_LT: SAND_PALE,
    GREEN_BRIGHT: GOLD_PALE,
}


def dry_holes(seed):
    """The rim clumps a dry tree has lost, in design pixels: seeded per
    variant row, so every frame and stage of it loses the same ones."""
    rng = random.Random(seed ^ 0xD27)
    holes = set()
    for _ in range(7):
        a = rng.uniform(0, math.tau)
        d = rng.uniform(6.0, 10.0)
        hx, hy, hr = 11.5 + math.cos(a) * d, 10.0 + math.sin(a) * d, math.sqrt(rng.uniform(2.2, 5.0))
        for y in range(D):
            for x in range(D):
                if (x + 0.5 - hx) ** 2 + (y + 0.5 - hy) ** 2 < hr * hr:
                    holes.add((x, y))
    return holes


def dry(cell, holes):
    """The dry version of one green cell: foliage recoloured and thinned.
    Only foliage goes - a trunk or a branch inside a hole stays."""
    out = cell.copy()
    px = out.load()
    for y in range(S):
        for x in range(S):
            p = px[x, y]
            if not p[3] or p[:3] not in DRY_RAMP:
                continue
            if (x // GRID, y // GRID) in holes and y < 17 * GRID:
                px[x, y] = (0, 0, 0, 0)
            else:
                px[x, y] = DRY_RAMP[p[:3]] + (255,)
    return out


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


def emit_species(build, v, row, seed):
    for stage in range(STAGES):
        for frame in range(SHIMMER):
            place(draw_species(build, v, stage, seed, frame), frame * STAGES + stage, row)
    place(draw_species(build, v, 3, seed), STUMP_COL, row)
    for b in range(3):
        place(draw_species(build, v, 4 + b, seed), BURN_COL + b, row)


def emit_dry(row, evergreen=False, thin=True):
    """Copy a finished green row onto the dry half, recoloured."""
    holes = dry_holes(row) if thin else set()
    for col in range(GREEN_COLS):
        cell = sheet.crop((col * S, row * S, col * S + S, row * S + S))
        place(cell if evergreen else dry(cell, holes), GREEN_COLS + col, row)


for r, v in enumerate(BROADLEAF):
    emit(draw_broadleaf, v, r, 1300 + r * 37)
for r, v in enumerate(CONIFER):
    emit(draw_conifer, v, 4 + r, 1500 + r * 37)
for c in range(RUBBLE_VARIANTS):
    place(draw_rubble_tree(c, 1700 + c * 13, False), c, 8)
    place(draw_rubble_tree(c, 1800 + c * 13, True), c, 9)
for k, (build, variants) in enumerate(SPECIES):
    for i, v in enumerate(variants):
        emit_species(build, v, NEW_SPECIES_ROW + k * 4 + i, 2100 + k * 101 + i * 37)

for row in range(ROWS):
    if row in (8, 9):
        # Rubble: litter recoloured, never thinned - it is already sparse.
        emit_dry(row, thin=False)
    else:
        species = (row - NEW_SPECIES_ROW) // 4 if row >= NEW_SPECIES_ROW else None
        emit_dry(row, evergreen=species in EVERGREEN)

# The block grid is guaranteed by construction (every primitive writes into
# the design-pixel buffer, which only expands to whole GRID x GRID blocks,
# and the dry pass recolours or clears whole blocks), so unlike
# gen_walls.py there is no pixelate() pass to confirm it.
sheet.save(f'{OUT}/trees_sheet.png')
print('trees_sheet.png', sheet.size)
