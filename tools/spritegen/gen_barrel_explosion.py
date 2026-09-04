"""Generate static/barrel_explosion.png - the oil barrel's blast animation
and the scorch marks it leaves (docs/PROPS_SPEC.md, blast.rs).

Layout: 768x128, two rows of 64x64 cells.
    row 0  cols 0-11  the one-shot blast: flash, fireball, mushroom, smoke
    row 1  cols 0-2   three scorch-decal variants (cols 3-11 blank)
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
SCORCHES = 3
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


sheet = Image.new('RGBA', (S * FRAMES, S * 2), (0, 0, 0, 0))
for f in range(FRAMES):
    sheet.paste(frame(f, random.Random(700 + f * 7)), (f * S, 0))          # no mask
for k in range(SCORCHES):
    sheet.paste(scorch(random.Random(900 + k * 13)), (k * S, S))
sheet.save(f'{OUT}/barrel_explosion.png')
print('barrel_explosion.png', sheet.size)
