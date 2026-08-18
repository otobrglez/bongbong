#!/usr/bin/env python3
"""Generate static/ground_sheet.png: tileable battlefield-floor tiles.

Not wired into the game yet -- this is a design/preview pass. See
docs/GROUND_SPEC.md for the full column-layout writeup and the extension
notes for adding more materials later (sand, snow, mud, water, ...).

------------------------------------------------------------------------
THE CORE IDEA -- O(materials) art, not O(materials^2)
------------------------------------------------------------------------
Walls (walls_sheet.png) are sparse, individually-placed obstacles, so every
tile can just be a self-contained square. Ground is different: it has to
cover the *entire* battlefield floor, edge to edge, as irregular patches of
one material sitting on top of another (a dirt patch on grass, a road
cutting across grass). Naively that needs *pairwise* transition art -- a
dirt-onto-grass edge, a road-onto-grass edge, a road-onto-dirt edge, and
another full set for every future material pair (a 4th material would mean
6 pairs, a 5th means 10, ...).

This sheet avoids that by treating grass as the implicit base layer (always
full-bleed, opaque, everywhere) and giving every *other* material a small
set of edge tiles that fade to fully transparent on their outer side. A
transparent-edged dirt patch composites correctly over grass, over another
material, or over nothing, with zero knowledge of what's underneath -- so
each new material only ever needs its OWN edge set, never one per existing
material it might border. That's the whole trick.

------------------------------------------------------------------------
COLUMN LAYOUT (same for every material row, incl. future ones)
------------------------------------------------------------------------
  0-2  Fill variants   Flat, fully opaque, seamlessly tileable. Decoration
                       (speckle/cracks/tufts) never touches the outer ~3px
                       border, so ANY two variants -- same material or
                       not -- tile against each other with no seam, because
                       the border is always the plain base colour.
  3    Edge: straight  Material fills the south (bottom) side, fades to
                       fully transparent on the north (top) side along an
                       organic wavy boundary (period 16px, so it repeats
                       seamlessly left-to-right along a straight run).
  4    Edge: outer     A convex corner -- material bulges into the SW
       corner          (bottom-left) corner of the tile, transparent NE.
  5    Edge: inner     A concave corner -- material fills nearly the whole
       corner          tile with a rounded bite taken out of the NE corner
                       (the shape that closes an outer corner from the
                       other side).
  6    Detail overlay  A sparser, more decorative variant of the fill
                       (bigger accent props: a flower, a paint dash, a
                       cluster of pebbles) meant to be scattered occasionally
                       rather than tiled edge-to-edge.
  7    Reserved        Intentionally blank for now -- see docs/GROUND_SPEC.md
                       for what goes here per material (road crosswalk,
                       dirt tire-ruts, ...).

Cols 3-5 are drawn once per material in a single canonical orientation
(material-south / transparent-north for the straight edge; bulge-SW for
the outer corner; bite-NE for the inner corner). The other 3 rotations of
each are the *same art rotated 90 deg* at draw time -- not additional art --
matching how this codebase already rotates tank/shell sprites at draw time
instead of baking directional frames.
"""
import math
import os
import random
import sys

from PIL import Image

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from resurrect64 import snap

S = 32
OUT = os.environ.get('SPRITE_OUT', 'assets/sprites')
os.makedirs(OUT, exist_ok=True)

WAVE_PERIOD = 16.0   # divides 32 -> 2 full cycles/tile -> seamless horizontally
WAVE_AMP = 3.0
OUTER_R = 25.0        # outer-corner bulge radius from the SW corner
OUTER_WOBBLE = 2.0
INNER_R = 12.0         # inner-corner bite radius from the NE corner
INNER_WOBBLE = 1.5


def mul(c, f):
    return snap((max(0, min(255, int(c[0] * f))),
                 max(0, min(255, int(c[1] * f))),
                 max(0, min(255, int(c[2] * f))),
                 c[3] if len(c) > 3 else 255))


def blank():
    return Image.new('RGBA', (S, S), (0, 0, 0, 0))


def px(img, x, y, c):
    x, y = int(x), int(y)
    if 0 <= x < S and 0 <= y < S and c is not None:
        img.putpixel((x, y), c)


def rect(img, x0, y0, x1, y1, c):
    for y in range(int(y0), int(y1) + 1):
        for x in range(int(x0), int(x1) + 1):
            px(img, x, y, c)


def disc(img, cx, cy, r, c):
    for y in range(int(cy - r) - 1, int(cy + r) + 2):
        for x in range(int(cx - r) - 1, int(cx + r) + 2):
            if (x - cx) ** 2 + (y - cy) ** 2 <= r * r:
                px(img, x, y, c)


def rotate_cw(img, deg):
    """Lossless 90-degree-multiple rotation for pixel art -- Image.rotate()
    resamples even at exact right angles, which can blur edges and create
    off-palette colours. transpose() is an exact pixel permutation instead."""
    deg %= 360
    if deg == 0:
        return img
    if deg == 90:
        return img.transpose(Image.Transpose.ROTATE_270)
    if deg == 180:
        return img.transpose(Image.Transpose.ROTATE_180)
    if deg == 270:
        return img.transpose(Image.Transpose.ROTATE_90)
    raise ValueError(f'{deg} is not a multiple of 90')


def fill_flat(mat):
    img = blank()
    rect(img, 0, 0, 31, 31, mat['base'] + (255,))
    return img


def scatter(img, mat, rng, margin=3, dashes=2, blotches=1, pop=1, rare=0.22):
    """Interior-only decoration, shared by fill/detail tiles. Never touches
    the outer `margin` px, which is what keeps every fill/detail tile
    seamlessly tileable against every other one (see module doc)."""
    lo, hi = margin, S - 1 - margin
    kind = mat['speckle']

    for _ in range(dashes):
        x, y = rng.randint(lo, hi), rng.randint(lo, hi)
        if kind == 'grass':
            # a short blade of grass: 2px vertical dash
            px(img, x, y, mat['shadow'] + (255,))
            px(img, x, y - 1, mat['light'] + (255,))
        elif kind == 'dirt':
            # a hairline crack
            dx, dy = rng.choice([(1, 0), (1, 1), (0, 1)])
            px(img, x, y, mat['shadow'] + (255,))
            px(img, x + dx, y + dy, mat['shadow'] + (200,))
        else:  # road
            dx = rng.choice([-1, 1])
            px(img, x, y, mat['shadow'] + (255,))
            px(img, x + dx, y, mat['shadow'] + (180,))

    for _ in range(blotches):
        x, y = rng.randint(lo, hi), rng.randint(lo, hi)
        if kind == 'dirt':
            px(img, x, y, mat['light'] + (255,))
        else:
            px(img, x, y, mat['light'] + (130,))

    for _ in range(pop):
        if rng.random() > 0.45:
            continue
        x, y = rng.randint(lo, hi), rng.randint(lo, hi)
        if kind == 'grass':
            px(img, x, y, mat['pop'] + (255,))
            px(img, x + rng.choice([-1, 1]), y, mat['pop'] + (150,))
        elif kind == 'dirt':
            px(img, x, y, mat['pop'] + (255,))
        else:  # road: a short paint dash (lane marking fragment)
            rect(img, x, y, x + 2, y, mat['pop'] + (255,))

    if mat.get('rare') and rng.random() < rare:
        x, y = rng.randint(lo, hi), rng.randint(lo, hi)
        if kind == 'grass':
            # a single bright bud, one pale petal beside it -- not a full
            # flower (that's what the col-6 detail tile is for)
            px(img, x, y, mat['rare'] + (255,))
            px(img, x + 1, y, mat['pop'] + (180,))
        elif kind == 'dirt':
            # a weed tuft poking through
            px(img, x, y, mat['rare'] + (255,))
            px(img, x, y - 1, mat['rare'] + (200,))


def fill_variant(mat, seed):
    img = fill_flat(mat)
    scatter(img, mat, random.Random(seed))
    return img


def detail_tile(mat, seed):
    """Col 6: sparser than a fill variant, bigger single accent prop --
    meant to be scattered occasionally by the placement code rather than
    tiled edge-to-edge, so it can be a little louder."""
    img = fill_flat(mat)
    rng = random.Random(seed)
    kind = mat['speckle']
    cx, cy = S / 2, S / 2
    if kind == 'grass':
        # a small flower cluster, bigger than the "rare" one in scatter()
        for ox, oy in ((0, 0), (-3, 1), (3, 1), (-1, -3), (1, -3)):
            disc(img, cx + ox, cy + oy, 1, mat['pop'] + (255,))
        disc(img, cx, cy, 1, mat['rare'] + (255,))
        scatter(img, mat, rng, dashes=6, blotches=2, pop=0)
    elif kind == 'dirt':
        # a small cluster of pebbles
        for ox, oy in ((-4, -2), (3, -3), (-2, 4), (4, 3), (0, 0)):
            disc(img, cx + ox, cy + oy, rng.choice([1, 1, 2]), mat['pop'] + (255,))
        scatter(img, mat, rng, dashes=4, blotches=2, pop=0)
    else:  # road: a broken centerline dash, full tile width
        rect(img, 4, int(cy) - 1, 12, int(cy), mat['pop'] + (255,))
        rect(img, 19, int(cy) - 1, 27, int(cy), mat['pop'] + (255,))
        scatter(img, mat, rng, dashes=4, blotches=2, pop=0)
    return img


def wave_y(x, base=16.0, amp=WAVE_AMP, period=WAVE_PERIOD):
    return base + amp * math.sin(2.0 * math.pi * x / period)


def edge_straight(mat, seed):
    """Material fills the south side, fades to transparent north, along a
    horizontally-seamless wavy boundary. See module doc for the rotate-in-
    code convention for the other 3 orientations."""
    img = fill_flat(mat)
    scatter(img, mat, random.Random(seed), margin=4)
    for x in range(S):
        yb = wave_y(x)
        for y in range(S):
            if y < yb - 1:
                img.putpixel((x, y), (0, 0, 0, 0))
    inner_shadow(img, mat)
    return img


def outer_corner(mat, seed):
    """Convex bulge into the SW corner; transparent NE. Rotate in code for
    the other 3 corners."""
    img = fill_flat(mat)
    scatter(img, mat, random.Random(seed), margin=4)
    for y in range(S):
        for x in range(S):
            ang = math.atan2((S - y), x + 0.001)
            rr = OUTER_R + OUTER_WOBBLE * math.sin(ang * 4.0)
            d = math.hypot(x - 0, y - S)
            if d > rr:
                img.putpixel((x, y), (0, 0, 0, 0))
    inner_shadow(img, mat)
    return img


def inner_corner(mat, seed):
    """Nearly-full tile with a rounded bite out of the NE corner -- the
    shape that closes an outer corner from the other side. Rotate in code
    for the other 3 corners."""
    img = fill_flat(mat)
    scatter(img, mat, random.Random(seed), margin=4)
    for y in range(S):
        for x in range(S):
            ang = math.atan2(y, (S - x) + 0.001)
            rr = INNER_R + INNER_WOBBLE * math.sin(ang * 4.0)
            d = math.hypot(x - S, y - 0)
            if d < rr:
                img.putpixel((x, y), (0, 0, 0, 0))
    inner_shadow(img, mat)
    return img


def inner_shadow(img, mat, f=0.68):
    """Darken material pixels that border a transparent boundary -- depth
    without an outline, same trick as gen_walls.py's inner_shadow. Cell
    edges are NOT treated as holes, so tiles stay seamless."""
    src = img.copy()
    for y in range(S):
        for x in range(S):
            c = src.getpixel((x, y))
            if c[3] == 0:
                continue
            for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                nx, ny = x + dx, y + dy
                if not (0 <= nx < S and 0 <= ny < S):
                    continue
                if src.getpixel((nx, ny))[3] == 0:
                    img.putpixel((x, y), mul(c, f))
                    break


# ======================================================================
MATERIALS = [
    dict(name='grass', base=(0x54, 0x7E, 0x64), shadow=(0x37, 0x4E, 0x4A),
         light=(0x92, 0xA9, 0x84), pop=(0xCD, 0xDF, 0x6C), rare=(0xFB, 0xFF, 0x86),
         speckle='grass'),
    dict(name='dirt', base=(0x9E, 0x45, 0x39), shadow=(0x4C, 0x3E, 0x24),
         light=(0xCD, 0x68, 0x3D), pop=(0x9B, 0xAB, 0xB2), rare=(0xA2, 0xA9, 0x47),
         speckle='dirt'),
    dict(name='road', base=(0x62, 0x55, 0x65), shadow=(0x3E, 0x35, 0x46),
         light=(0x7F, 0x70, 0x8A), pop=(0xF9, 0xC2, 0x2B), rare=None,
         speckle='road'),
]

COL_ORDER = ['fill0', 'fill1', 'fill2', 'edge', 'outer', 'inner', 'detail', 'reserved']
COLS = len(COL_ORDER)

sheet = Image.new('RGBA', (S * COLS, S * len(MATERIALS)), (0, 0, 0, 0))

for row, mat in enumerate(MATERIALS):
    base_seed = row * 1000
    cells = {
        'fill0': fill_variant(mat, base_seed + 1),
        'fill1': fill_variant(mat, base_seed + 2),
        'fill2': fill_variant(mat, base_seed + 3),
        'edge': edge_straight(mat, base_seed + 4),
        'outer': outer_corner(mat, base_seed + 5),
        'inner': inner_corner(mat, base_seed + 6),
        'detail': detail_tile(mat, base_seed + 7),
        'reserved': blank(),
    }
    for col, key in enumerate(COL_ORDER):
        sheet.paste(cells[key], (col * S, row * S))   # no mask arg: direct
        # copy, preserves exact RGBA -- see docs/PALETTE.md's paste-as-mask
        # note (gen_tanks.py/gen_shells.py hit this bug during the recolor).

sheet.save(f'{OUT}/ground_sheet.png')
print('ground_sheet.png', sheet.size, f'{COLS} cols x {len(MATERIALS)} rows')
for i, m in enumerate(MATERIALS):
    print(f'  row {i}: {m["name"]}')


# ======================================================================
# Preview only (not part of the sprite sheet): a small mock battlefield
# tiling grass as the base with a dirt patch and a road strip cut into it,
# so the edge/corner tiles can be judged in context instead of in
# isolation. Written alongside the sheet, not loaded by the game.
# ======================================================================
def make_preview():
    tiles_w, tiles_h = 14, 10
    px_w, px_h = tiles_w * S, tiles_h * S
    out = Image.new('RGBA', (px_w, px_h), (0, 0, 0, 0))
    rng = random.Random(42)

    grass, dirt, road = MATERIALS[0], MATERIALS[1], MATERIALS[2]

    def grass_fill():
        return fill_variant(grass, rng.randint(0, 99999))

    # 1. base layer: grass everywhere
    for ty in range(tiles_h):
        for tx in range(tiles_w):
            out.paste(grass_fill(), (tx * S, ty * S))

    # 2. an oval dirt patch, drawn with fill/edge/corner tiles by
    #    classifying each grid cell against the oval boundary. Kept clear of
    #    the road strip below (rows 3-5) so the two materials read as
    #    distinct zones instead of overlapping in this mockup.
    ocx, ocy, orx, ory = 4.0, 7.6, 2.7, 1.7

    def in_oval(tx, ty, pad=0.0):
        return ((tx - ocx) / (orx + pad)) ** 2 + ((ty - ocy) / (ory + pad)) ** 2 <= 1.0

    for ty in range(tiles_h):
        for tx in range(tiles_w):
            here = in_oval(tx, ty)
            if not here:
                continue
            n = in_oval(tx, ty - 1)
            s = in_oval(tx, ty + 1)
            e = in_oval(tx + 1, ty)
            w = in_oval(tx - 1, ty)
            if [n, s, e, w].count(False) >= 3:
                # an isolated 1-tile-wide spike (e.g. the oval's polar tip) --
                # no edge/corner tile represents "surrounded on 3 sides", and
                # a plain opaque square there would poke out as a hard nub.
                # Simplest fix for this demo: don't count it as patch at all.
                continue
            missing = [d for d, v in (('n', n), ('s', s), ('e', e), ('w', w)) if not v]
            if not missing:
                cell = fill_variant(dirt, rng.randint(0, 99999))
            elif len(missing) == 1:
                rot = {'n': 0, 'e': 90, 's': 180, 'w': 270}[missing[0]]
                cell = rotate_cw(edge_straight(dirt, rng.randint(0, 99999)), rot)
            elif len(missing) == 2 and set(missing) in ({'n', 'e'}, {'e', 's'}, {'s', 'w'}, {'w', 'n'}):
                # convex corner: two adjacent sides missing -> outer corner
                key = ''.join(sorted(missing))
                rot = {'en': 0, 'es': 90, 'sw': 180, 'nw': 270}[key]
                cell = rotate_cw(outer_corner(dirt, rng.randint(0, 99999)), rot)
            else:
                cell = fill_variant(dirt, rng.randint(0, 99999))
            out.paste(cell, (tx * S, ty * S), cell)

    # 3. a straight horizontal road strip, 3 tiles thick: north edge, a
    #    fill/detail middle row (centerline dashes belong here, not on an
    #    edge row), south edge (edge_straight rotated 180).
    road_y0, road_h = 3, 3
    for tx in range(tiles_w):
        for i in range(road_h):
            ty = road_y0 + i
            if i == 0:
                cell = edge_straight(road, rng.randint(0, 99999))
            elif i == road_h - 1:
                cell = rotate_cw(edge_straight(road, rng.randint(0, 99999)), 180)
            elif tx % 3 == 1:
                cell = detail_tile(road, 5000 + tx)   # centerline dash, every 3rd tile
            else:
                cell = fill_variant(road, rng.randint(0, 99999))
            out.paste(cell, (tx * S, ty * S), cell)

    out.save(f'{OUT}/ground_preview.png')
    print('ground_preview.png', out.size, f'({tiles_w}x{tiles_h} tiles, mockup only)')


make_preview()
