"""Generate static/barrel_explosion.png - the oil barrel's blast animation
and the scorch marks it leaves (docs/PROPS_SPEC.md, blast.rs).

Layout: 768x384, six rows of 64x64 cells.
    row 0  cols 0-11  the one-shot blast: flash, fireball, mushroom, smoke
    row 1  cols 0-4   five scorch-decal variants (three blots, two oil
                      splatters), col 5 the directional streak a shot's
                      blast adds (points right; the game quarter-turns
                      it), cols 6-8 the three-frame ground-fire loop a
                      burning pool or trail cell draws, cols 9-11 blank
    row 2  cols 0-11  the tall blast: a narrow column that rises fast
    row 3  cols 0-11  the flat blast: a wide, low splash, little smoke
    row 4  cols 0-11  the double blast: two cores, the second a beat behind
    row 5  cols 0-11  the mushroom cloud a dying tank goes up in: a fireball
                      that climbs a stem of fire, spreads into a rolling
                      cap over a ring of dust, and thins out as smoke
Row 0 is drawn by exactly the code it always was, so it is byte-identical
to the single-row sheet; the new rows share its primitives through a
shape table (sx, sy, rise, smoke, twin) and pick the same frame stages.
Drawn at native 64px and shown at scale 2 (blast_anim_scale), so one drawn
pixel is a 2x2 block on screen - the same density as the tanks. Frame 0
carries the flash on purpose: it replaces the barrel sprite the frame it
vanishes, so it must never be blank; the last frame is a few specks, not
empty, so the animation fades rather than pops.

Colours from tools/punypalette.py (docs/PALETTE.md); smoke uses palette
greys with reduced alpha (alpha never changes RGB, so the off-palette pixel
check still passes). Primitives copied from gen_shells.py / gen_walls.py.

Run:  nix-shell -p "python3.withPackages (ps: [ps.pillow])" \\
        --run "SPRITE_OUT=static python3 tools/spritegen/gen_barrel_explosion.py"
"""
from PIL import Image
import os, random, math, sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from punypalette import (BLACK, WHITE, STONE_DARKEST, STONE_DK, STONE_MD, STONE_LT,
                         RED_DEEP, RED_MD, RED_BRIGHT, RED_DK, GOLD_BRIGHT, WOOD_DEEPER)

S = 64
FRAMES = 12
SCORCHES = 5
STREAK_COL = 5
FIRE_COL = 6
FIRE_FRAMES = 3
ROWS = 6
OUT = os.environ.get('SPRITE_OUT', 'assets/sprites')
os.makedirs(OUT, exist_ok=True)


def C(c, a=255):
    return c + (a,)


def blank():
    return Image.new('RGBA', (S, S), (0, 0, 0, 0))


def put(img, x, y, c):
    x, y = int(x), int(y)
    if 0 <= x < S and 0 <= y < S and c is not None:
        img.putpixel((x, y), c)


def disc(img, cx, cy, r, c):
    r2 = r * r
    for y in range(max(0, int(cy - r) - 1), min(S, int(cy + r) + 2)):
        for x in range(max(0, int(cx - r) - 1), min(S, int(cx + r) + 2)):
            dx, dy = x - cx, y - cy
            if dx * dx + dy * dy <= r2:
                put(img, x, y, c)


def annulus(img, cx, cy, r0, r1, c):
    for y in range(max(0, int(cy - r1) - 1), min(S, int(cy + r1) + 2)):
        for x in range(max(0, int(cx - r1) - 1), min(S, int(cx + r1) + 2)):
            d = math.hypot(x - cx, y - cy)
            if r0 <= d <= r1:
                put(img, x, y, c)


def rays(img, cx, cy, length, c, diagonal=True, start=0):
    for i in range(start, int(round(length)) + 1):
        put(img, cx + 0.5, cy - i, c)
        put(img, cx + 0.5, cy + i, c)
        put(img, cx - i, cy + 0.5, c)
        put(img, cx + i, cy + 0.5, c)
        if diagonal:
            j = i * 0.7071
            put(img, cx + 0.5 + j, cy + 0.5 - j, c)
            put(img, cx + 0.5 - j, cy + 0.5 - j, c)
            put(img, cx + 0.5 + j, cy + 0.5 + j, c)
            put(img, cx + 0.5 - j, cy + 0.5 + j, c)


def lump(img, cx, cy, r, c, rng, n=7):
    """A lumpy cloud: the union of n jittered discs around (cx, cy)."""
    disc(img, cx, cy, r * 0.8, c)
    for _ in range(n):
        a = rng.random() * math.tau
        d = rng.random() * r * 0.5
        disc(img, cx + math.cos(a) * d, cy + math.sin(a) * d, r * (0.45 + 0.35 * rng.random()), c)


def crack(img, x0, y0, x1, y1, c, rng, jitter=1, width=1):
    steps = int(max(abs(x1 - x0), abs(y1 - y0))) + 1
    for i in range(steps + 1):
        t = i / max(1, steps)
        x = x0 + (x1 - x0) * t + rng.randint(-jitter, jitter)
        y = y0 + (y1 - y0) * t + rng.randint(-jitter, jitter)
        for w in range(width):
            put(img, x + w, y, c)


def debris(img, cx, cy, radius, n, rng, cols):
    for i in range(n):
        a = rng.random() * math.tau
        d = radius * (0.85 + 0.3 * rng.random())
        x, y = cx + math.cos(a) * d, cy + math.sin(a) * d
        c = cols[i % len(cols)]
        put(img, x, y, c)
        put(img, x + 1, y, c)
        put(img, x, y + 1, c)
        put(img, x + 1, y + 1, c)


def embers(img, cx, cy, radius, n, rng, cols):
    for i in range(n):
        a = rng.random() * math.tau
        d = radius * rng.random()
        put(img, cx + math.cos(a) * d, cy + math.sin(a) * d, cols[i % len(cols)])


FIRE = [C(RED_DEEP), C(RED_MD), C(RED_BRIGHT), C(GOLD_BRIGHT), C(WHITE)]
DEBRIS = [C(STONE_DARKEST), C(WOOD_DEEPER), C(BLACK)]
CX = 31.5


def smoke(a):
    return C(STONE_DK, a)


def smoke_dk(a):
    return C(STONE_DARKEST, a)


def smoke_lt(a):
    return C(STONE_MD, a)


def frame(f, rng):
    img = blank()
    cy = CX - max(0, f - 3) * 1.0          # the cloud rises after the peak
    if f == 0:
        disc(img, CX, CX, 9, C(WHITE))
        annulus(img, CX, CX, 9, 14, C(GOLD_BRIGHT))
        rays(img, CX, CX, 26, C(WHITE), start=10)
        for k in range(8):
            a = k * math.tau / 8 + math.pi / 8
            put(img, CX + 0.5 + math.cos(a) * 18, CX + 0.5 + math.sin(a) * 18, C(GOLD_BRIGHT))
    elif f == 1:
        disc(img, CX, CX, 20, C(RED_MD))
        disc(img, CX, CX, 15, C(GOLD_BRIGHT))
        disc(img, CX, CX, 8, C(WHITE))
        rays(img, CX, CX, 30, C(GOLD_BRIGHT), start=16)
        debris(img, CX, CX, 22, 8, rng, DEBRIS)
    elif f == 2:
        lump(img, CX, CX, 26, C(RED_DEEP), rng, 9)
        lump(img, CX, CX, 23, C(RED_MD), rng, 8)
        lump(img, CX, CX, 19, C(RED_BRIGHT), rng, 7)
        disc(img, CX, CX, 14, C(GOLD_BRIGHT))
        disc(img, CX, CX, 7, C(WHITE))
        debris(img, CX, CX, 26, 12, rng, DEBRIS)
        embers(img, CX, CX, 28, 14, rng, [C(GOLD_BRIGHT), C(RED_BRIGHT)])
    elif f == 3:
        lump(img, CX, CX, 29, C(RED_DEEP), rng, 9)
        lump(img, CX, CX, 25, C(RED_MD), rng, 8)
        lump(img, CX, CX, 20, C(RED_BRIGHT), rng, 7)
        disc(img, CX, CX, 13, C(GOLD_BRIGHT))
        disc(img, CX, CX, 4, C(WHITE))
        for k in range(5):
            a = -math.pi * (0.2 + 0.15 * k)
            lump(img, CX + math.cos(a) * 20, CX + math.sin(a) * 20, 9, smoke(220), rng, 4)
        debris(img, CX, CX, 30, 10, rng, DEBRIS)
        embers(img, CX, CX, 30, 12, rng, [C(GOLD_BRIGHT), C(RED_BRIGHT)])
    elif f == 4:
        lump(img, CX, cy - 6, 26, smoke(230), rng, 10)
        lump(img, CX, cy - 8, 20, smoke_dk(230), rng, 8)
        lump(img, CX - 10, cy - 14, 8, smoke_lt(230), rng, 4)
        lump(img, CX, cy + 5, 17, C(RED_DEEP), rng, 6)
        lump(img, CX, cy + 5, 13, C(RED_MD), rng, 5)
        disc(img, CX, cy + 5, 8, C(GOLD_BRIGHT))
        disc(img, CX, cy + 5, 3, C(WHITE))
        embers(img, CX, cy, 30, 12, rng, [C(GOLD_BRIGHT), C(RED_BRIGHT), C(RED_MD)])
    elif f == 5:
        lump(img, CX, cy - 4, 28, smoke_dk(210), rng, 11)
        lump(img, CX, cy - 8, 20, smoke(210), rng, 7)
        lump(img, CX - 9, cy - 14, 7, smoke_lt(210), rng, 4)
        for k in range(3):
            a = math.pi * (0.15 + 0.35 * k)
            px_, py_ = CX + math.cos(a) * 11, cy + 6 + math.sin(a) * 6
            disc(img, px_, py_, 9 - k, C(RED_MD))
            disc(img, px_, py_, 5 - k, C(GOLD_BRIGHT))
        embers(img, CX, cy, 30, 10, rng, [C(GOLD_BRIGHT), C(RED_BRIGHT)])
    elif f == 6:
        lump(img, CX, cy - 4, 28, smoke(200), rng, 11)
        lump(img, CX, cy - 2, 22, smoke_dk(200), rng, 8)
        for k in range(3):
            a = math.pi * (0.2 + 0.3 * k)
            disc(img, CX + math.cos(a) * 12, cy + 6 + math.sin(a) * 5, 5, C(RED_DEEP))
            put(img, CX + math.cos(a) * 12, cy + 6 + math.sin(a) * 5, C(RED_MD))
        embers(img, CX, cy, 30, 9, rng, [C(RED_BRIGHT), C(RED_MD)])
    elif f in (7, 8, 9):
        alpha = {7: 170, 8: 130, 9: 95}[f]
        r = {7: 12, 8: 10, 9: 8}[f]
        spread = {7: 12, 8: 16, 9: 19}[f]
        for k in range(4):
            a = math.pi * (0.25 + 0.5 * k)
            lump(img, CX + math.cos(a) * spread, cy - 4 + math.sin(a) * spread * 0.8, r, smoke_dk(alpha), rng, 5)
            lump(img, CX + math.cos(a) * spread * 0.6, cy - 6 + math.sin(a) * spread * 0.5, r * 0.8, smoke(alpha), rng, 4)
        if f == 9:
            for k in range(6):
                a = rng.random() * math.tau
                disc(img, CX + math.cos(a) * 22, cy - 4 + math.sin(a) * 18, 3, C(BLACK, 60))
        embers(img, CX, cy, 26, 8 - (f - 7) * 2, rng, [C(RED_DK), C(RED_DEEP)])
    elif f == 10:
        for k in range(5):
            a = rng.random() * math.tau
            lump(img, CX + math.cos(a) * 20, cy - 6 + math.sin(a) * 16, 6, smoke(55), rng, 4)
        for _ in range(3):
            put(img, CX + rng.randint(-24, 24), cy + rng.randint(-24, 16), smoke_dk(120))
        embers(img, CX, cy, 24, 2, rng, [C(RED_DK)])
    else:
        for k in range(4):
            a = rng.random() * math.tau
            lump(img, CX + math.cos(a) * 22, cy - 8 + math.sin(a) * 16, 4, smoke(25), rng, 3)
        for _ in range(5):
            put(img, CX + rng.randint(-26, 26), cy + rng.randint(-26, 14), smoke_dk(90))
    return img


def scorch(rng):
    img = blank()
    for _ in range(6):
        a = rng.random() * math.tau
        d = rng.random() * 9
        disc(img, CX + math.cos(a) * d, CX + math.sin(a) * d, 14 + rng.random() * 4, C(BLACK, 150))
    for _ in range(9):
        a = rng.random() * math.tau
        d = 16 + rng.random() * 8
        disc(img, CX + math.cos(a) * d, CX + math.sin(a) * d, 4 + rng.random() * 4, C(STONE_DARKEST, 120))
    for k in range(10):
        a = k * math.tau / 10 + rng.random() * 0.4
        ln = 22 + rng.random() * 6
        crack(img, CX, CX, CX + math.cos(a) * ln, CX + math.sin(a) * ln, C(BLACK, 100), rng, width=2)
    disc(img, CX, CX, 5, C(STONE_DK, 90))
    embers(img, CX, CX, 12, 5, rng, [C(RED_DK, 200)])
    for _ in range(18):
        a = rng.random() * math.tau
        d = 20 + rng.random() * 8
        put(img, CX + math.cos(a) * d, CX + math.sin(a) * d, C(STONE_DARKEST, 140))
    return img



# ---------------------------------------------------------------- shaped blasts
# The same stages as `frame`, stretched by a shape: sx/sy scale every
# radius on each axis, `rise` is how fast the cloud goes up after the
# peak, `smoke` scales the cloud, `twin` offsets a second core.
SHAPES = {
    'tall':   dict(sx=0.72, sy=1.35, rise=2.4, smoke=0.8, twin=0),
    'flat':   dict(sx=1.45, sy=0.60, rise=0.35, smoke=0.55, twin=0),
    'double': dict(sx=0.85, sy=0.85, rise=1.0, smoke=1.0, twin=7),
}


def ell(img, cx, cy, rx, ry, c):
    """A filled ellipse, the shaped stand-in for `disc`."""
    if rx < 0.5 or ry < 0.5:
        return
    for y in range(max(0, int(cy - ry) - 1), min(S, int(cy + ry) + 2)):
        for x in range(max(0, int(cx - rx) - 1), min(S, int(cx + rx) + 2)):
            dx, dy = (x - cx) / rx, (y - cy) / ry
            if dx * dx + dy * dy <= 1.0:
                put(img, x, y, c)


def elump(img, cx, cy, rx, ry, c, rng, n=7):
    ell(img, cx, cy, rx * 0.8, ry * 0.8, c)
    for _ in range(n):
        a = rng.random() * math.tau
        d = rng.random() * 0.5
        ell(img, cx + math.cos(a) * d * rx, cy + math.sin(a) * d * ry,
            rx * (0.45 + 0.35 * rng.random()), ry * (0.45 + 0.35 * rng.random()), c)


def shaped_frame(f, rng, sh, cx=CX):
    """One frame of a shaped blast, drawn around (cx, CX)."""
    img = blank()
    sx, sy = sh['sx'], sh['sy']
    cy = CX - max(0, f - 3) * sh['rise']
    smk = sh['smoke']
    if f == 0:
        ell(img, cx, CX, 9 * sx, 9 * sy, C(WHITE))
        ell(img, cx, CX, 14 * sx, 14 * sy, C(GOLD_BRIGHT))
        ell(img, cx, CX, 9 * sx, 9 * sy, C(WHITE))
        rays(img, cx, CX, 26 * max(sx, sy), C(WHITE), start=int(10 * min(sx, sy)))
        for k in range(8):
            a = k * math.tau / 8 + math.pi / 8
            put(img, cx + 0.5 + math.cos(a) * 18 * sx, CX + 0.5 + math.sin(a) * 18 * sy, C(GOLD_BRIGHT))
    elif f == 1:
        ell(img, cx, CX, 20 * sx, 20 * sy, C(RED_MD))
        ell(img, cx, CX, 15 * sx, 15 * sy, C(GOLD_BRIGHT))
        ell(img, cx, CX, 8 * sx, 8 * sy, C(WHITE))
        rays(img, cx, CX, 30 * max(sx, sy), C(GOLD_BRIGHT), start=int(16 * min(sx, sy)))
        debris(img, cx, CX, 22 * max(sx, sy), 8, rng, DEBRIS)
    elif f in (2, 3):
        r = 26 if f == 2 else 29
        elump(img, cx, CX, r * sx, r * sy, C(RED_DEEP), rng, 9)
        elump(img, cx, CX, (r - 3) * sx, (r - 3) * sy, C(RED_MD), rng, 8)
        elump(img, cx, CX, (r - 7) * sx, (r - 7) * sy, C(RED_BRIGHT), rng, 7)
        ell(img, cx, CX, 14 * sx, 14 * sy, C(GOLD_BRIGHT))
        ell(img, cx, CX, (7 if f == 2 else 4) * sx, (7 if f == 2 else 4) * sy, C(WHITE))
        if f == 3:
            for k in range(5):
                a = -math.pi * (0.2 + 0.15 * k)
                elump(img, cx + math.cos(a) * 20 * sx, CX + math.sin(a) * 20 * sy, 9 * sx * smk, 9 * sy * smk, smoke(220), rng, 4)
        debris(img, cx, CX, r * max(sx, sy), 12 if f == 2 else 10, rng, DEBRIS)
        embers(img, cx, CX, 28 * max(sx, sy), 14 if f == 2 else 12, rng, [C(GOLD_BRIGHT), C(RED_BRIGHT)])
    elif f == 4:
        elump(img, cx, cy - 6, 26 * sx * smk, 20 * sy * smk, smoke(230), rng, 10)
        elump(img, cx, cy - 8, 20 * sx * smk, 14 * sy * smk, smoke_dk(230), rng, 8)
        elump(img, cx - 10 * sx, cy - 14, 8 * sx * smk, 6 * sy * smk, smoke_lt(230), rng, 4)
        elump(img, cx, cy + 5, 17 * sx, 17 * sy, C(RED_DEEP), rng, 6)
        elump(img, cx, cy + 5, 13 * sx, 13 * sy, C(RED_MD), rng, 5)
        ell(img, cx, cy + 5, 8 * sx, 8 * sy, C(GOLD_BRIGHT))
        ell(img, cx, cy + 5, 3 * sx, 3 * sy, C(WHITE))
        embers(img, cx, cy, 30 * max(sx, sy), 12, rng, [C(GOLD_BRIGHT), C(RED_BRIGHT), C(RED_MD)])
    elif f == 5:
        elump(img, cx, cy - 4, 28 * sx * smk, 20 * sy * smk, smoke_dk(210), rng, 11)
        elump(img, cx, cy - 8, 20 * sx * smk, 14 * sy * smk, smoke(210), rng, 7)
        elump(img, cx - 9 * sx, cy - 14, 7 * sx * smk, 5 * sy * smk, smoke_lt(210), rng, 4)
        for k in range(3):
            a = math.pi * (0.15 + 0.35 * k)
            px_, py_ = cx + math.cos(a) * 11 * sx, cy + 6 + math.sin(a) * 6 * sy
            ell(img, px_, py_, (9 - k) * sx, (9 - k) * sy, C(RED_MD))
            ell(img, px_, py_, (5 - k) * sx, (5 - k) * sy, C(GOLD_BRIGHT))
        embers(img, cx, cy, 30 * max(sx, sy), 10, rng, [C(GOLD_BRIGHT), C(RED_BRIGHT)])
    elif f == 6:
        elump(img, cx, cy - 4, 28 * sx * smk, 20 * sy * smk, smoke(200), rng, 11)
        elump(img, cx, cy - 2, 22 * sx * smk, 16 * sy * smk, smoke_dk(200), rng, 8)
        for k in range(3):
            a = math.pi * (0.2 + 0.3 * k)
            ell(img, cx + math.cos(a) * 12 * sx, cy + 6 + math.sin(a) * 5 * sy, 5 * sx, 5 * sy, C(RED_DEEP))
            put(img, cx + math.cos(a) * 12 * sx, cy + 6 + math.sin(a) * 5 * sy, C(RED_MD))
        embers(img, cx, cy, 30 * max(sx, sy), 9, rng, [C(RED_BRIGHT), C(RED_MD)])
    elif f in (7, 8, 9):
        alpha = {7: 170, 8: 130, 9: 95}[f]
        r = {7: 12, 8: 10, 9: 8}[f]
        spread = {7: 12, 8: 16, 9: 19}[f]
        for k in range(4):
            a = math.pi * (0.25 + 0.5 * k)
            elump(img, cx + math.cos(a) * spread * sx, cy - 4 + math.sin(a) * spread * 0.8 * sy,
                  r * sx * smk, r * sy * smk, smoke_dk(alpha), rng, 5)
            elump(img, cx + math.cos(a) * spread * 0.6 * sx, cy - 6 + math.sin(a) * spread * 0.5 * sy,
                  r * 0.8 * sx * smk, r * 0.8 * sy * smk, smoke(alpha), rng, 4)
        if f == 9:
            for k in range(6):
                a = rng.random() * math.tau
                ell(img, cx + math.cos(a) * 22 * sx, cy - 4 + math.sin(a) * 18 * sy, 3, 3, C(BLACK, 60))
        embers(img, cx, cy, 26 * max(sx, sy), 8 - (f - 7) * 2, rng, [C(RED_DK), C(RED_DEEP)])
    elif f == 10:
        for k in range(5):
            a = rng.random() * math.tau
            elump(img, cx + math.cos(a) * 20 * sx, cy - 6 + math.sin(a) * 16 * sy, 6 * sx * smk, 6 * sy * smk, smoke(55), rng, 4)
        for _ in range(3):
            put(img, cx + rng.randint(-24, 24), cy + rng.randint(-24, 16), smoke_dk(120))
        embers(img, cx, cy, 24, 2, rng, [C(RED_DK)])
    else:
        for k in range(4):
            a = rng.random() * math.tau
            elump(img, cx + math.cos(a) * 22 * sx, cy - 8 + math.sin(a) * 16 * sy, 4 * sx * smk, 4 * sy * smk, smoke(25), rng, 3)
        for _ in range(5):
            put(img, cx + rng.randint(-26, 26), cy + rng.randint(-26, 14), smoke_dk(90))
    return img


def double_frame(f, rng, sh):
    """Two cores: the second a frame behind the first, offset either side.
    Drawn last-over-first so the later core's flash lands on the earlier
    fireball the way a chained blast does."""
    tw = sh['twin']
    first = shaped_frame(f, random.Random(rng.random()), sh, cx=CX - tw)
    if f == 0:
        return first
    second = shaped_frame(f - 1, random.Random(rng.random()), sh, cx=CX + tw)
    # Overwrite, never blend: compositing a translucent smoke pixel over a
    # fire pixel would invent a colour that is on neither the palette nor
    # the sheet.
    for y in range(S):
        for x in range(S):
            p = second.getpixel((x, y))
            if p[3]:
                first.putpixel((x, y), p)
    return first


# ---------------------------------------------------------------- mushroom cloud
# Row 5, a tank's death: never quarter-turned (the stem has to point up),
# so every frame is drawn upright. The column stands on BASE, a little
# below the cell centre, so the fire climbs off the hull rather than off
# the ground behind it; the cap tops out near the cell's upper edge.
BASE = 46


def stem(img, top, bottom, w, cols):
    """The column: nested widths, outermost colour first, a pixel of
    wobble per row so it reads as rolling fire rather than a pipe."""
    for i, c in enumerate(cols):
        half = w * (1 - i / len(cols))
        for y in range(int(top), int(bottom) + 1):
            wob = 1 if (y * 7 + i * 3) % 5 == 0 else 0
            for x in range(int(CX - half) - wob, int(CX + half) + 1 + wob):
                put(img, x + 0.5, y, c)


def skirt(img, rx, ry, c, rng, n=10):
    """The ring of dust thrown out along the ground at the base."""
    for k in range(n):
        a = k * math.tau / n + rng.random() * 0.3
        ell(img, CX + math.cos(a) * rx, BASE + math.sin(a) * ry, 4 + rng.random() * 2, 2 + rng.random(), c)


def cap(img, cy, rx, ry, outer, inner, belly, rng):
    """The rolling head: a lumpy flattened dome of `outer`, a smaller
    `inner` roll on top, and `belly` - the fire still lighting its
    underside - along the bottom edge."""
    elump(img, CX, cy, rx, ry, outer, rng, 9)
    elump(img, CX - rx * 0.25, cy - ry * 0.35, rx * 0.5, ry * 0.45, inner, rng, 5)
    elump(img, CX + rx * 0.3, cy - ry * 0.3, rx * 0.45, ry * 0.4, inner, rng, 5)
    if belly:
        for k, c in enumerate(belly):
            ell(img, CX, cy + ry * (0.45 + 0.12 * k), rx * (0.7 - 0.18 * k), ry * (0.35 - 0.08 * k), c)


def mushroom_frame(f, rng):
    img = blank()
    if f == 0:
        disc(img, CX, BASE - 4, 8, C(WHITE))
        annulus(img, CX, BASE - 4, 8, 12, C(GOLD_BRIGHT))
        rays(img, CX, BASE - 4, 17, C(WHITE), start=9)
    elif f == 1:
        skirt(img, 16, 5, smoke(200), rng)
        lump(img, CX, BASE - 8, 15, C(RED_MD), rng, 7)
        disc(img, CX, BASE - 8, 11, C(GOLD_BRIGHT))
        disc(img, CX, BASE - 8, 6, C(WHITE))
        debris(img, CX, BASE - 6, 20, 8, rng, DEBRIS)
    elif f == 2:
        skirt(img, 21, 6, smoke(210), rng)
        stem(img, BASE - 14, BASE, 5, [C(RED_DEEP), C(RED_MD), C(GOLD_BRIGHT)])
        lump(img, CX, BASE - 18, 16, C(RED_DEEP), rng, 8)
        lump(img, CX, BASE - 18, 13, C(RED_MD), rng, 7)
        disc(img, CX, BASE - 17, 9, C(GOLD_BRIGHT))
        disc(img, CX, BASE - 16, 4, C(WHITE))
        debris(img, CX, BASE - 12, 24, 10, rng, DEBRIS)
        embers(img, CX, BASE - 14, 24, 10, rng, [C(GOLD_BRIGHT), C(RED_BRIGHT)])
    elif f == 3:
        skirt(img, 24, 7, smoke(200), rng)
        stem(img, BASE - 22, BASE, 5, [C(RED_DEEP), C(RED_MD), C(RED_BRIGHT), C(GOLD_BRIGHT)])
        cap(img, BASE - 25, 20, 10, C(RED_DEEP), C(RED_MD),
            [C(RED_BRIGHT), C(GOLD_BRIGHT), C(WHITE)], rng)
        embers(img, CX, BASE - 20, 24, 12, rng, [C(GOLD_BRIGHT), C(RED_BRIGHT)])
    elif f == 4:
        skirt(img, 25, 7, smoke(180), rng)
        stem(img, BASE - 24, BASE, 5, [C(RED_DEEP), C(RED_MD), C(GOLD_BRIGHT)])
        cap(img, BASE - 27, 23, 10, smoke_dk(235), smoke(235),
            [C(RED_DEEP), C(RED_MD), C(GOLD_BRIGHT)], rng)
        embers(img, CX, BASE - 22, 24, 10, rng, [C(GOLD_BRIGHT), C(RED_BRIGHT), C(RED_MD)])
    elif f == 5:
        skirt(img, 26, 7, smoke(150), rng)
        stem(img, BASE - 26, BASE, 4, [smoke_dk(220), C(RED_DEEP), C(RED_MD)])
        cap(img, BASE - 28, 24, 10, smoke_dk(225), smoke(225),
            [C(RED_DEEP), C(RED_MD)], rng)
        embers(img, CX, BASE - 24, 24, 8, rng, [C(GOLD_BRIGHT), C(RED_BRIGHT)])
    elif f == 6:
        skirt(img, 26, 7, smoke(115), rng)
        stem(img, BASE - 27, BASE - 2, 4, [smoke(200), smoke_dk(200), C(RED_DEEP)])
        cap(img, BASE - 29, 25, 10, smoke_dk(210), smoke_lt(210), [C(RED_DEEP)], rng)
        embers(img, CX, BASE - 26, 26, 6, rng, [C(RED_BRIGHT), C(RED_MD)])
    elif f in (7, 8, 9):
        alpha = {7: 175, 8: 135, 9: 95}[f]
        rise = f - 6
        stem(img, BASE - 28 - rise, BASE - 4 - rise * 3, 3 - (f - 7), [smoke(alpha - 30), smoke_dk(alpha - 30)])
        cap(img, BASE - 30 - rise // 2, 25 + rise, 10 - (f - 7), smoke_dk(alpha), smoke(alpha), None, rng)
        embers(img, CX, BASE - 30, 24, 8 - (f - 7) * 2, rng, [C(RED_DK), C(RED_DEEP)])
    elif f == 10:
        for k in range(6):
            a = rng.random() * math.tau
            lump(img, CX + math.cos(a) * 22, BASE - 32 + math.sin(a) * 7, 5, smoke(55), rng, 4)
        for _ in range(4):
            put(img, CX + rng.randint(-26, 26), BASE - 32 + rng.randint(-8, 18), smoke_dk(120))
        embers(img, CX, BASE - 30, 20, 2, rng, [C(RED_DK)])
    else:
        for k in range(5):
            a = rng.random() * math.tau
            lump(img, CX + math.cos(a) * 24, BASE - 33 + math.sin(a) * 7, 3, smoke(25), rng, 3)
        for _ in range(5):
            put(img, CX + rng.randint(-27, 27), BASE - 33 + rng.randint(-8, 18), smoke_dk(90))
    return img


def streak(rng):
    """Oil thrown downrange by a shot's blast: streaks fanning out to the
    right from just off centre, drawn over the blot."""
    img = blank()
    for i in range(7):
        a = (rng.random() - 0.5) * 0.9
        ln = 18 + rng.random() * 14
        crack(img, CX + 4, CX, CX + 4 + math.cos(a) * ln, CX + math.sin(a) * ln * 0.8, C(BLACK, 130), rng, width=2)
    for _ in range(10):
        a = (rng.random() - 0.5) * 1.1
        d = 14 + rng.random() * 18
        put(img, CX + 4 + math.cos(a) * d, CX + math.sin(a) * d * 0.8, C(STONE_DARKEST, 160))
    return img


def splatter(rng):
    """An oil splatter: a lopsided blot with drips and a sheen, next to
    the three blast blots."""
    img = blank()
    for _ in range(5):
        a = rng.random() * math.tau
        d = rng.random() * 7
        disc(img, CX + math.cos(a) * d, CX + math.sin(a) * d, 10 + rng.random() * 5, C(BLACK, 165))
    for _ in range(7):
        a = rng.random() * math.tau
        d = 14 + rng.random() * 9
        disc(img, CX + math.cos(a) * d, CX + math.sin(a) * d, 2 + rng.random() * 3, C(BLACK, 140))
    for _ in range(12):
        a = rng.random() * math.tau
        d = 16 + rng.random() * 10
        put(img, CX + math.cos(a) * d, CX + math.sin(a) * d, C(STONE_DARKEST, 150))
    for _ in range(3):
        put(img, CX + rng.randint(-6, 6), CX + rng.randint(-6, 6), C(STONE_DK, 110))
    return img


def fire_frame(k, rng):
    """One frame of the ground-fire loop: tongues of flame rising out of a
    low bed of fire in the bottom half of the cell (the sprite is drawn
    at scale 2 over a 32px cell, centred a little above it, so the bed
    sits on the cell and the tongues rise past it)."""
    img = blank()
    base_y = 44
    # The bed: a low, wide pool of deep red with a brighter core.
    lump(img, CX, base_y, 13, C(RED_DEEP), rng, 6)
    lump(img, CX, base_y, 9, C(RED_MD), rng, 5)
    lump(img, CX, base_y + 1, 5, C(GOLD_BRIGHT), rng, 3)
    # The tongues: a few tapering columns, their heights and lean
    # varying by frame so the loop licks rather than blinks.
    for i in range(5):
        x = CX - 10 + i * 5 + rng.randint(-1, 1)
        h = 8 + rng.randint(0, 10) + (k * 3 + i * 2) % 7
        lean = rng.randint(-2, 2)
        for j in range(h):
            t = j / max(1, h)
            w = max(1, int(3 * (1 - t)))
            col = C(RED_DEEP) if t > 0.75 else C(RED_MD) if t > 0.45 else C(GOLD_BRIGHT) if t > 0.15 else C(WHITE)
            for dx in range(-w // 2, w // 2 + 1):
                put(img, x + dx + int(lean * t), base_y - 2 - j, col)
    embers(img, CX, base_y - 12, 14, 6, rng, [C(GOLD_BRIGHT), C(RED_BRIGHT)])
    return img


sheet = Image.new('RGBA', (S * FRAMES, S * ROWS), (0, 0, 0, 0))
for f in range(FRAMES):
    sheet.paste(frame(f, random.Random(700 + f * 7)), (f * S, 0))          # no mask
for k in range(3):
    sheet.paste(scorch(random.Random(900 + k * 13)), (k * S, S))
for k in range(3, SCORCHES):
    sheet.paste(splatter(random.Random(950 + k * 17)), (k * S, S))
sheet.paste(streak(random.Random(990)), (STREAK_COL * S, S))
for k in range(FIRE_FRAMES):
    sheet.paste(fire_frame(k, random.Random(1100 + k * 11)), ((FIRE_COL + k) * S, S))
for row, name in ((2, 'tall'), (3, 'flat'), (4, 'double')):
    sh = SHAPES[name]
    for f in range(FRAMES):
        rng = random.Random(1200 + row * 100 + f * 7)
        cell = double_frame(f, rng, sh) if sh['twin'] else shaped_frame(f, rng, sh)
        sheet.paste(cell, (f * S, row * S))
for f in range(FRAMES):
    sheet.paste(mushroom_frame(f, random.Random(1800 + f * 7)), (f * S, 5 * S))
sheet.save(f'{OUT}/barrel_explosion.png')
print('barrel_explosion.png', sheet.size)
