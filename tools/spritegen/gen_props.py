"""Generate static/props_sheet.png - the three destructible props (sandbags,
oil barrels, fences) that share the 32px obstacle grid with walls but are
discrete objects rather than tiling wall tiles. See docs/PROPS_SPEC.md for
the full sheet map and docs/sandbags-barrels-fences.md for the feature.

Layout: 128x288, 4 cols x 9 rows of 32x32 cells.
    rows 0-2  Sandbag  cols 0-2: straight row / staggered wall / heaped pile
                                 x intact/torn/collapsed
    rows 3-4  Barrel   cols 0-3: two drum liveries x intact/dented/critical,
                                 col 3 = lit fuse (drawn while the fuse burns)
    rows 5-8  Fence    cols 0-1: wooden H, wooden V, wire H, wire V x
                                 intact/damaged (a fence variant owns two
                                 rows; the game picks H/V from neighbours)
Cells outside those ranges are blank and never sampled.

Every cell is drawn on a 16x16 "macro pixel" canvas and upscaled 2x with
NEAREST, which is pixel-for-pixel what gen_walls.py's pixelate() post-process
does to the walls sheet (tanks draw their 32px tile at scale 2, obstacles at
scale 1 - baking the 2x chunkiness in keeps every static thing on screen at
one pixel density). Do not also pixelate the result.

Colours come from tools/punypalette.py (docs/PALETTE.md): identity colours
hand-picked, derived shades through mul()+snap, paste without a mask.
Primitives copied from gen_walls.py (no generator imports another's).

Run:  nix-shell -p "python3.withPackages (ps: [ps.pillow])" \\
        --run "SPRITE_OUT=static python3 tools/spritegen/gen_props.py"
"""
from PIL import Image
import os, random, math, sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from punypalette import (BLACK, WHITE, STONE_PALE, STONE_LT, STONE_MD, STONE_DK,
                         STONE_DARKEST, SAND_PALE, SAND_LT, SAND_MD, SAND_DK,
                         WOOD_PALE, WOOD_LT, WOOD_MD, WOOD_DK, WOOD_DEEPER,
                         WOOD_DARKEST, RED_BRIGHT, RED_MD, RED_DEEP, RED_DK,
                         RED_DARKEST, TEAL_MD, TEAL_DK, BLUE_DK, GOLD_BRIGHT, snap)

S = 16            # macro canvas: one drawn pixel = a 2x2 block in the 32px cell
CELL = 32
COLS, ROWS = 4, 9
OUT = os.environ.get('SPRITE_OUT', 'assets/sprites')
os.makedirs(OUT, exist_ok=True)


def C(c, a=255):
    return c + (a,)


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


def get(img, x, y):
    x, y = int(x), int(y)
    if 0 <= x < S and 0 <= y < S:
        return img.getpixel((x, y))
    return (0, 0, 0, 0)


def rect(img, x0, y0, x1, y1, c):
    for y in range(int(y0), int(y1) + 1):
        for x in range(int(x0), int(x1) + 1):
            px(img, x, y, c)


def clear(img, x0, y0, x1, y1):
    for y in range(int(y0), int(y1) + 1):
        for x in range(int(x0), int(x1) + 1):
            if 0 <= x < S and 0 <= y < S:
                img.putpixel((x, y), (0, 0, 0, 0))


def disc(img, cx, cy, r, c):
    for y in range(int(cy - r) - 1, int(cy + r) + 2):
        for x in range(int(cx - r) - 1, int(cx + r) + 2):
            if (x - cx) ** 2 + (y - cy) ** 2 <= r * r:
                px(img, x, y, c)


def disc_clear(img, cx, cy, r):
    for y in range(int(cy - r) - 1, int(cy + r) + 2):
        for x in range(int(cx - r) - 1, int(cx + r) + 2):
            if (x - cx) ** 2 + (y - cy) ** 2 <= r * r:
                if 0 <= x < S and 0 <= y < S:
                    img.putpixel((x, y), (0, 0, 0, 0))


def ellipse(img, cx, cy, rx, ry, c):
    for y in range(S):
        for x in range(S):
            dx, dy = (x - cx) / rx, (y - cy) / ry
            if dx * dx + dy * dy <= 1.0:
                px(img, x, y, c)


def ellipse_rim(img, cx, cy, rx, ry, c, pick=None):
    """The 1px outer band of an ellipse; `pick(angle)` may swap the colour."""
    for y in range(S):
        for x in range(S):
            dx, dy = (x - cx) / rx, (y - cy) / ry
            d = dx * dx + dy * dy
            if 0.62 <= d <= 1.0:
                col = c if pick is None else pick(math.atan2(y - cy, x - cx))
                px(img, x, y, col)


def splat(img, cx, cy, r, c, rng, n=5):
    for _ in range(n):
        a = rng.random() * math.tau
        d = rng.random() * r * 0.55
        disc(img, cx + math.cos(a) * d, cy + math.sin(a) * d,
             r * (0.42 + 0.42 * rng.random()), c)


def crack(img, x0, y0, x1, y1, c, rng, jitter=1):
    steps = int(max(abs(x1 - x0), abs(y1 - y0))) + 1
    for i in range(steps + 1):
        t = i / max(1, steps)
        px(img, x0 + (x1 - x0) * t + rng.randint(-jitter, jitter),
           y0 + (y1 - y0) * t + rng.randint(-jitter, jitter), c)


def declutter(img, min_size=3):
    seen = [[False] * S for _ in range(S)]
    for y in range(S):
        for x in range(S):
            if seen[y][x] or img.getpixel((x, y))[3] == 0:
                continue
            stack, comp = [(x, y)], []
            seen[y][x] = True
            while stack:
                cx, cy = stack.pop()
                comp.append((cx, cy))
                for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                    nx, ny = cx + dx, cy + dy
                    if 0 <= nx < S and 0 <= ny < S and not seen[ny][nx] \
                            and img.getpixel((nx, ny))[3] != 0:
                        seen[ny][nx] = True
                        stack.append((nx, ny))
            if len(comp) < min_size:
                for (cx, cy) in comp:
                    img.putpixel((cx, cy), (0, 0, 0, 0))


def front_strip(img, c):
    """The tilt cue: one darker pixel under the lowest solid pixel of every
    column (the object's +y side face, seen from slightly above)."""
    for x in range(S):
        low = None
        for y in range(S):
            if get(img, x, y)[3] != 0:
                low = y
        if low is not None and low + 1 < S and get(img, x, low + 1)[3] == 0:
            px(img, x, low + 1, c)


def up2(img):
    return img.resize((CELL, CELL), Image.NEAREST)


# ---------------------------------------------------------------- sandbags
SAND_TOP, SAND_BODY, SAND_LOW, SAND_GAP = C(SAND_PALE), C(SAND_LT), C(SAND_MD), C(SAND_DK)


def bag(img, x0, y0, x1, y1, sag=False):
    """One bag: rounded box, lighter top row, darker bottom row, one seam."""
    body = SAND_LOW if sag else SAND_BODY
    rect(img, x0, y0, x1, y1, body)
    if not sag:
        rect(img, x0 + 1, y0, x1 - 1, y0, SAND_TOP)
    rect(img, x0 + 1, y1, x1 - 1, y1, SAND_LOW if not sag else SAND_GAP)
    for (cx, cy) in ((x0, y0), (x1, y0), (x0, y1), (x1, y1)):
        px(img, cx, cy, SAND_GAP if get(img, cx, cy)[3] else None)
    mid = (x0 + x1) // 2
    px(img, mid, y0 + (y1 - y0) // 2, SAND_LOW)


# Arrangements as courses (top course first): each bag is (x0, y0, x1, y1).
def straight():
    return [
        [(-3, 4, 2, 7), (3, 4, 8, 7), (9, 4, 13, 7), (14, 4, 18, 7)],
        [(0, 8, 4, 11), (5, 8, 10, 11), (11, 8, 15, 11)],
    ]


def staggered():
    return [
        [(-2, 2, 3, 5), (4, 2, 9, 5), (10, 2, 15, 5)],
        [(0, 6, 4, 9), (5, 6, 10, 9), (11, 6, 15, 9)],
        [(-3, 10, 2, 13), (3, 10, 8, 13), (9, 10, 13, 13), (14, 10, 18, 13)],
    ]


def pile():
    # A heaped mound: one bag on top, two, then a full course (top course
    # first, so it is the one a collapse drops).
    return [
        [(5, 2, 10, 5)],
        [(2, 6, 7, 9), (8, 6, 13, 9)],
        [(-1, 10, 4, 13), (5, 10, 10, 13), (11, 10, 16, 13)],
    ]


SANDBAGS = [straight, staggered, pile]


def draw_sandbag(variant, stage, rng):
    courses = SANDBAGS[variant]()
    img = blank()
    if stage == 2:
        dropped, courses = courses[0], courses[1:]
    # Gap colour under every course so the seams read dark.
    for course in courses:
        for (x0, y0, x1, y1) in course:
            rect(img, x0, y0, x1, y1 + 1, SAND_GAP)
    sagging = set()
    if stage >= 1:
        flat = [b for course in courses for b in course]
        for b in rng.sample(flat, min(2, len(flat))):
            sagging.add(b)
    for course in courses:
        for b in course:
            x0, y0, x1, y1 = b
            if stage == 2:
                y0 += 1            # flattened
            bag(img, x0, y0, x1, y1, sag=b in sagging)
    if stage >= 1:
        # Torn bags: a dark slit and a trickle of pale sand beside it.
        for b in sagging:
            x0, y0, x1, y1 = b
            sx = max(1, min(S - 2, (x0 + x1) // 2 + rng.randint(-1, 1)))
            rect(img, sx, y0 + 1, sx, y1 - 1, SAND_GAP)
            for _ in range(4):
                px(img, sx + rng.randint(-1, 3), y1 + rng.randint(1, 2), SAND_TOP)
    if stage == 2:
        # The dropped course became spill: flat pale mounds where it was.
        for (x0, y0, x1, y1) in dropped:
            cx, cy = (x0 + x1) / 2.0, y1 + 0.5
            disc(img, cx, cy, 2.2, SAND_LOW)
            for _ in range(3):
                px(img, cx + rng.randint(-2, 2), cy + rng.randint(-1, 1), SAND_TOP)
    front_strip(img, SAND_GAP)
    declutter(img, 2)
    return img


# ---------------------------------------------------------------- barrels
BARRELS = [
    dict(body=C(RED_DK), lid=C(RED_DEEP), lid_hi=C(RED_MD), rim=C(RED_DARKEST),
         band=C(WOOD_DK), band_hi=C(WOOD_LT), rust=C(WOOD_DEEPER), hazard=False),
    dict(body=C(STONE_DK), lid=C(STONE_MD), lid_hi=C(STONE_LT), rim=C(STONE_DARKEST),
         band=C(TEAL_DK), band_hi=C(TEAL_MD), rust=C(WOOD_DEEPER), hazard=True),
]
PUDDLE, PUDDLE_SPECK, SHEEN = C(BLACK, 210), C(STONE_DARKEST), C(BLUE_DK, 120)
CX, CY = 7.5, 8.0


def draw_barrel(variant, stage, rng):
    p = BARRELS[variant]
    img = blank()
    if stage >= 1:
        # Oil puddle spreading right/down; bigger when critical.
        rx, ry = (2.5, 1.0) if stage == 1 else (5.0, 1.6)
        ellipse(img, 10.5, 14.2, rx, ry, PUDDLE)
        for _ in range(2 if stage == 1 else 4):
            px(img, 9 + rng.randint(-2, 4), 14 + rng.randint(-1, 1), PUDDLE_SPECK)
        px(img, 11, 14, SHEEN)
    ellipse(img, CX, CY, 6.0, 6.0, p['body'])            # skirt / side
    ellipse(img, CX, CY - 1.5, 6.0, 5.0, p['rim'])       # lid rim
    if p['hazard']:
        seg = math.tau / 8.0
        ellipse_rim(img, CX, CY - 1.5, 6.0, 5.0, p['rim'],
                    pick=lambda a: C(GOLD_BRIGHT) if int((a + math.pi) / seg) % 2 == 0 else C(BLACK))
    ellipse(img, CX, CY - 1.5, 4.8, 3.8, p['lid'])       # lid top face
    ellipse(img, CX - 0.5, CY - 2.5, 2.4, 1.6, p['lid_hi'])
    if not p['hazard']:
        rect(img, 3, CY - 1, 12, CY - 1, p['band'])       # the chord band
        px(img, 5, CY - 1, p['band_hi'])
    px(img, CX + 2, CY - 3, C(STONE_DARKEST))            # bung
    px(img, CX - 3, CY - 4, C(STONE_PALE))               # spec highlight
    px(img, CX + 2, CY + 3, p['band'])                   # side band peek
    if stage >= 1:
        for _ in range(2):
            disc(img, CX + rng.randint(-3, 3), CY - 1.5 + rng.randint(-2, 2), 1.2, mul(p['lid'], 0.6))
        splat(img, CX + rng.randint(-3, 3), CY + rng.randint(0, 3), 1.6, p['rust'], rng, n=3)
    if stage >= 2:
        crack(img, CX - 4, CY - 3, CX + 4, CY, C(BLACK), rng)
        px(img, CX - 4, CY - 1, C(RED_BRIGHT))
        px(img, CX + 4, CY - 2, C(GOLD_BRIGHT))
    if stage == 3:
        # Lit fuse: the lid is white-hot and throws four rays.
        ellipse(img, CX, CY - 1.5, 6.0, 5.0, C(RED_BRIGHT))
        ellipse(img, CX, CY - 1.5, 4.6, 3.6, C(GOLD_BRIGHT))
        disc(img, CX, CY - 1.5, 2.0, C(WHITE))
        for dx, dy in ((0, -1), (0, 1), (-1, 0), (1, 0)):
            for i in range(6, 9):
                px(img, CX + dx * i, CY - 1.5 + dy * i, C(WHITE))
    return img


# ---------------------------------------------------------------- fences
def draw_wood_fence(stage, rng):
    img = blank()
    posts = [(1, 2), (13, 14)]
    for (x0, x1) in posts:
        rect(img, x0, 3, x1, 11, C(WOOD_DEEPER))
        rect(img, x0, 2, x1, 2, C(WOOD_PALE))
    for y in (6, 9):
        rect(img, 0, y, 15, y, C(WOOD_MD))
    pickets = [4, 7, 10]
    if stage == 1:
        gone = rng.sample(pickets, 2)
        pickets = [x for x in pickets if x not in gone]
    for x in pickets:
        rect(img, x, 3, x, 11, C(WOOD_LT))
        px(img, x, 2, C(WOOD_PALE))
        px(img, x, 11, C(WOOD_DK))
    if stage == 1:
        # One picket leans, a rail is broken, splinters on the ground.
        crack(img, gone[0], 11, gone[0] + 2, 4, C(WOOD_LT), rng, jitter=0)
        clear(img, gone[1] - 1, 6, gone[1], 6)
        for _ in range(4):
            px(img, rng.randint(3, 12), 12 + rng.randint(0, 1), C(WOOD_DK))
    rect(img, 0, 12, 15, 12, C(WOOD_DARKEST))
    return img


def draw_wire_fence(stage, rng):
    img = blank()
    for (x0, x1) in ((1, 2), (13, 14)):
        rect(img, x0, 3, x1, 11, C(STONE_DK))
        rect(img, x0, 3, x1, 3, C(STONE_PALE))
    rect(img, 0, 3, 15, 3, C(STONE_LT))
    for y in range(4, 11):
        for x in range(S):
            if (x + y) % 2 == 0:
                px(img, x, y, C(STONE_MD))
    rect(img, 0, 11, 15, 11, C(STONE_DK))
    if stage == 1:
        hx, hy = 7.5 + rng.randint(-1, 1), 7.0
        disc_clear(img, hx, hy, 2.6)
        for a in (0.6, 2.2, 3.9, 5.4):
            px(img, hx + math.cos(a) * 3.2, hy + math.sin(a) * 3.2, C(STONE_LT))
        px(img, hx, hy - 3, C(STONE_LT))
    rect(img, 0, 12, 15, 12, C(STONE_DARKEST))
    return img


def transpose(img):
    return img.transpose(Image.TRANSPOSE)


# ---------------------------------------------------------------- sheet
def seed(row, col):
    return random.Random(500 + row * 31 + col * 7)


ROW_DRAWERS = [
    (0, lambda c: draw_sandbag(0, c, seed(0, c)) if c < 3 else None),
    (1, lambda c: draw_sandbag(1, c, seed(1, c)) if c < 3 else None),
    (2, lambda c: draw_sandbag(2, c, seed(2, c)) if c < 3 else None),
    (3, lambda c: draw_barrel(0, c, seed(3, c))),
    (4, lambda c: draw_barrel(1, c, seed(4, c))),
    (5, lambda c: draw_wood_fence(c, seed(5, c)) if c < 2 else None),
    (6, lambda c: transpose(draw_wood_fence(c, seed(5, c))) if c < 2 else None),
    (7, lambda c: draw_wire_fence(c, seed(7, c)) if c < 2 else None),
    (8, lambda c: transpose(draw_wire_fence(c, seed(7, c))) if c < 2 else None),
]

sheet = Image.new('RGBA', (CELL * COLS, CELL * ROWS), (0, 0, 0, 0))
for row, draw in ROW_DRAWERS:
    for col in range(COLS):
        cell = draw(col)
        if cell is not None:
            sheet.paste(up2(cell), (col * CELL, row * CELL))   # no mask: no alpha drift
sheet.save(f'{OUT}/props_sheet.png')
print('props_sheet.png', sheet.size)
