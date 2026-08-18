from PIL import Image
import os, random, math, sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from punypalette import STONE_DK, STONE_LT, STONE_MD, STONE_PALE, snap

S = 32
OUT = os.environ.get('SPRITE_OUT', 'assets/sprites')
os.makedirs(OUT, exist_ok=True)

# Curated tools/punypalette.py picks -- colours sampled directly from the
# Puny World ground-layer tileset (see that module's own doc comment), same
# fire ramp as gen_tanks.py/gen_shells.py so a burning wall, a burning tank,
# and a shell hit all read as the same fire. Second recolor pass for this
# sheet (see tools/spritegen/_backup/pre-punypalette-*/gen_walls.py for the
# R64 version) -- walls sit directly in the same scene as the ground layer,
# so once that shipped (docs/GROUND_SPEC.md) the R64-vivid walls read as
# neon plastic next to Puny World's much softer terrain (docs/PALETTE.md).
SCORCH = (0x4A, 0x22, 0x21, 255)
DUST = (0xD8, 0xBF, 0x8E, 255)
EMBER = (0x81, 0x2F, 0x27, 255)
FIRE_D = (0x9C, 0x35, 0x27, 255)
FIRE_M = (0xE4, 0x42, 0x19, 255)
FIRE_C = (0xEE, 0xA3, 0x43, 255)

GL_D = (0x03, 0x8A, 0xAB, 255)
GL_M = (0x04, 0xA0, 0xB4, 255)
GL_L = (0x27, 0xD8, 0xC5, 255)


def mul(c, f):
    # Scale, then snap back onto the 64-colour set -- see gen_tanks.py's mul().
    return snap((max(0, min(255, int(c[0] * f))),
                 max(0, min(255, int(c[1] * f))),
                 max(0, min(255, int(c[2] * f))),
                 c[3] if len(c) > 3 else 255))


def blank():
    return Image.new('RGBA', (S, S), (0, 0, 0, 0))


def px(img, x, y, c):
    """Full-bleed: the whole 0..31 cell is drawable, no reserved border."""
    x, y = int(x), int(y)
    if 0 <= x < S and 0 <= y < S and c is not None:
        img.putpixel((x, y), c)


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


def inner_shadow(img, f=0.52):
    """Darken material pixels that border a hole -- gives depth without an
    outline. Cell edges are NOT treated as holes, so tiles stay seamless."""
    src = img.copy()
    for y in range(S):
        for x in range(S):
            c = src.getpixel((x, y))
            if c[3] == 0:
                continue
            for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                nx, ny = x + dx, y + dy
                if not (0 <= nx < S and 0 <= ny < S):
                    continue                      # tile border: not a hole
                if src.getpixel((nx, ny))[3] == 0:
                    img.putpixel((x, y), mul(c, f))
                    break


def declutter(img, min_size=4):
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


# ======================================================================
# BRICK -- all periods divide 32 so courses line up across tiles
# ======================================================================
# One shared tone for all four bond patterns -- per the original design
# intent ("all four share one family... so mixed-variant walls still read as
# one material"), so all four pick the same base rather than four different
# ones. The per-cell tone jitter below (0.94-1.10x, routed through mul()'s
# snap) gives natural per-brick variation on top.
#
# Colour + texture are both sampled from the *same* Puny World reference now
# (static/punyworld/, the castle stone-block walls) -- a first cut used this
# texture's stipple/tick pattern but the red roof-tile colour, since that
# was the only "brick-red" tone in the source pack. Layering a masonry-block
# pattern onto a roof-tile red doesn't correspond to anything actually in
# Puny World's art, and looked exactly as uncanny as that mismatch implies.
# Pale stone (the same family IRONS below draws from, just its lighter step)
# is correct for both channels at once.
BRICK_MORTAR = STONE_DK + (255,)
BRICKS = [
    dict(name='running', base=STONE_LT, pw=8,  ph=4, bond='run'),
    dict(name='block',   base=STONE_LT, pw=16, ph=8, bond='run'),
    dict(name='long',    base=STONE_LT, pw=16, ph=4, bond='run'),
    dict(name='stacked', base=STONE_LT, pw=8,  ph=8, bond='stack'),
]


TICK = STONE_PALE + (255,)  # pale mortar highlight, from Puny World's own tick marks


def stipple_cell(img, x0, y0, x1, y1, base, tone, rng):
    """Internal mottled texture within one cell -- echoes Puny World's stone
    building walls (static/punyworld/), where each big block panel is
    subdivided into a few subtly-toned stones rather than drawn as one flat
    rectangle. Skipped for cells too small to usefully subdivide (running
    bond's 6x2 chips stay flat, matching the reference's own smallest brick
    course).

    Picks an explicit adjacent palette step (STONE_PALE/STONE_MD) rather
    than nudging `base` by a small multiplicative factor through mul() --
    a first cut did the latter (+-4-8%, the same technique the rest of this
    file's shading uses) and it was *invisible*: the Puny Palette's steps
    are spaced far enough apart that a small nudge just snaps straight back
    to the same colour it started from, unlike Resurrect 64's more tightly-
    spaced ramps that technique was originally written for."""
    w, h = x1 - x0 + 1, y1 - y0 + 1
    if w < 6 or h < 5:
        return
    sub_w = max(3, w // 3)
    sub_h = max(3, h // 2)
    sy = y0 + 1
    while sy < y1 - 1:
        sx = x0 + 1
        while sx < x1 - 1:
            if rng.random() < 0.4:
                shade = STONE_PALE if rng.random() < 0.5 else STONE_MD
                rect(img, sx, sy, min(sx + sub_w - 2, x1 - 1), min(sy + sub_h - 2, y1 - 1),
                     shade + (255,))
            sx += sub_w
        sy += sub_h


def brick_ticks(img, cells, rng):
    """Small pale tick marks along each cell's mortar seam, matching Puny
    World's own stone-wall mortar detailing (occasional highlight dashes,
    not a mark at every seam pixel) -- skipped entirely for running bond's
    small cells, where ticks this close together read as speckle rather
    than coursing detail."""
    for (x0, y0, x1, y1) in cells:
        if x1 - x0 < 6:
            continue
        for tx in range(x0 + 2, x1 - 1, 4):
            if 0 <= tx < S and rng.random() < 0.4:
                if 0 <= y0 - 1 < S:
                    px(img, tx, y0 - 1, TICK)
                if 0 <= y1 + 1 < S:
                    px(img, tx, y1 + 1, TICK)


def brick_cells(pw, ph, bond):
    cells = []
    for row, y in enumerate(range(0, S, ph)):
        off = 0 if (bond == 'stack' or row % 2 == 0) else pw // 2
        x = -off
        while x < S:
            cells.append((x, y, x + pw - 2, y + ph - 2))
            x += pw
    return cells


def draw_brick(v, decay, seed):
    rng = random.Random(seed)
    img = blank()
    base = v['base']
    rect(img, 0, 0, 31, 31, BRICK_MORTAR)

    cells = brick_cells(v['pw'], v['ph'], v['bond'])
    tones = [rng.choice([1.0, 1.0, 1.0, 0.97, 1.03, 0.94]) for _ in cells]
    for (c, tone) in zip(cells, tones):
        x0, y0, x1, y1 = c
        rect(img, x0, y0, x1, y1, mul(base, tone))
        rect(img, x0, y0, x1, y0, mul(base, tone * 1.10))
        rect(img, x0, y1, x1, y1, mul(base, tone * 0.86))
        stipple_cell(img, x0, y0, x1, y1, base, tone, rng)
    brick_ticks(img, cells, rng)

    if decay == 0:
        return img

    frac = [0.0, 0.05, 0.12, 0.22, 0.36, 0.55][decay]
    order = list(cells)
    rng.shuffle(order)
    for (x0, y0, x1, y1) in order[:int(len(cells) * frac)]:
        for y in range(y0 - 1, y1 + 2):
            for x in range(x0 - 1, x1 + 2):
                if not (0 <= x < S and 0 <= y < S):
                    continue
                if (x in (x0 - 1, x1 + 1) or y in (y0 - 1, y1 + 1)) \
                        and rng.random() < 0.45:
                    continue
                img.putpixel((x, y), (0, 0, 0, 0))

    for _ in range(decay * 2):
        cx, cy = rng.randint(2, 29), rng.randint(2, 29)
        disc(img, cx, cy, rng.uniform(1.2, 2.4), SCORCH)
        disc(img, cx, cy, rng.uniform(0.6, 1.2), mul(base, 0.58))
    for _ in range(decay * 3):
        px(img, rng.randint(0, 31), rng.randint(0, 31), mul(base, 0.74))
    for _ in range(decay):
        x0, y0 = rng.randint(2, 29), rng.randint(2, 29)
        crack(img, x0, y0, x0 + rng.randint(-9, 9), y0 + rng.randint(-9, 9),
              mul(base, 0.64), rng)

    if decay >= 4:
        for _ in range(10 if decay == 4 else 22):
            a = rng.random() * math.tau
            r = 13 + rng.random() * 5
            disc_clear(img, 15.5 + math.cos(a) * r, 15.5 + math.sin(a) * r,
                       rng.uniform(1.6, 3.2))
        for _ in range(8):
            px(img, rng.randint(0, 31), rng.randint(0, 31), DUST)

    declutter(img, 5)
    inner_shadow(img)
    return img


# ======================================================================
# IRON -- clean steel, rust arrives with damage
# ======================================================================
# Same one-shared-tone approach as BRICKS -- "all four share a steel tone...
# differing by surface treatment" (per WALLS_SPEC.md). Sampled from Puny
# World's own stone/plaster building walls rather than an invented steel
# grey, so it sits in the same register as the buildings the ground-layer
# art already draws.
IRONS = [
    dict(name='riveted',    base=(0x5D, 0x65, 0x4F), style='rivet'),
    dict(name='corrugated', base=(0x5D, 0x65, 0x4F), style='corr'),
    dict(name='banded',     base=(0x5D, 0x65, 0x4F), style='band'),
    dict(name='tread',      base=(0x5D, 0x65, 0x4F), style='tread'),
]
RUST = (0x99, 0x65, 0x24, 255)
RUST_D = (0x68, 0x47, 0x1D, 255)
RUST_L = (0xDE, 0x99, 0x43, 255)


def draw_iron(v, dmg, seed):
    rng = random.Random(seed)
    img = blank()
    base = v['base']
    rect(img, 0, 0, 31, 31, base)
    st = v['style']

    if st == 'rivet':
        # 16px lattice -> rivet grid continues across tile joins
        for ry in (0, 16):
            for rx in (0, 16):
                px(img, rx + 4, ry + 4, mul(base, 1.30))
                px(img, rx + 5, ry + 4, mul(base, 0.70))
                px(img, rx + 4, ry + 5, mul(base, 0.70))
                px(img, rx + 12, ry + 12, mul(base, 1.30))
                px(img, rx + 13, ry + 12, mul(base, 0.70))
                px(img, rx + 12, ry + 13, mul(base, 0.70))
        for y in range(0, S, 16):
            rect(img, 0, y, 31, y, mul(base, 1.08))
            rect(img, 0, y + 15, 31, y + 15, mul(base, 0.88))
    elif st == 'corr':
        for x in range(0, S, 4):
            rect(img, x, 0, x, 31, mul(base, 1.20))
            rect(img, x + 1, 0, x + 1, 31, mul(base, 1.06))
            rect(img, x + 3, 0, x + 3, 31, mul(base, 0.80))
    elif st == 'band':
        for y in range(0, S, 16):
            rect(img, y and 0 or 0, y, 31, y + 3, mul(base, 0.86))
            rect(img, 0, y, 31, y, mul(base, 1.14))
            rect(img, 0, y + 3, 31, y + 3, mul(base, 0.74))
            for rx in range(4, 32, 8):
                px(img, rx, y + 1, mul(base, 1.32))
    else:  # tread plate -- 8px diamond lattice
        for gy in range(0, S, 8):
            for gx in range(0, S, 8):
                ox = 4 if (gy // 8) % 2 else 0
                cx, cy = gx + ox + 2, gy + 4
                for k in range(3):
                    rect(img, cx - k, cy - 2 + k, cx + k, cy - 2 + k,
                         mul(base, 1.18))
                for k in range(3):
                    rect(img, cx - (2 - k), cy + 1 + k, cx + (2 - k), cy + 1 + k,
                         mul(base, 0.84))

    n_rust = [0, 3, 6, 10][dmg]
    n_deep = [0, 1, 3, 6][dmg]
    n_fleck = [2, 4, 7, 10][dmg]
    for _ in range(n_rust):
        splat(img, rng.randint(2, 29), rng.randint(2, 29),
              rng.uniform(1.6, 3.4), RUST, rng, 4)
    for _ in range(n_deep):
        splat(img, rng.randint(2, 29), rng.randint(2, 29),
              rng.uniform(1.0, 2.0), RUST_D, rng, 3)
    for _ in range(n_fleck):
        px(img, rng.randint(0, 31), rng.randint(0, 31), RUST_L)

    if dmg > 0:
        for _ in range(dmg * 3):
            cx, cy = rng.randint(2, 29), rng.randint(2, 29)
            r = rng.uniform(1.4, 2.6)
            disc(img, cx, cy, r, mul(base, 0.68))
            disc(img, cx - 0.6, cy - 0.6, r * 0.55, mul(base, 0.94))
        for _ in range(dmg * 2):
            px(img, rng.randint(0, 31), rng.randint(0, 31), SCORCH)
        for _ in range(dmg * 2):
            x0, y0 = rng.randint(2, 29), rng.randint(2, 29)
            crack(img, x0, y0, x0 + rng.randint(-7, 7), y0 + rng.randint(-7, 7),
                  mul(base, 0.64), rng)
        for _ in range(dmg):
            splat(img, rng.randint(4, 27), rng.randint(4, 27),
                  rng.uniform(2.2, 3.6), (0x4C, 0x52, 0x3C, 255), rng, 4)
    return img


# ======================================================================
# WOOD
# ======================================================================
# Warm honey wood, sampled from Puny World's own wood-plank building walls
# -- picked distinctly from BRICKS' red so wood still reads as a different
# material from brick at a glance, not just a different bond pattern.
WOODS = [
    dict(name='planks_h', base=(0xDE, 0x99, 0x43), style='horiz'),
    dict(name='planks_v', base=(0xDE, 0x99, 0x43), style='vert'),
    dict(name='stagger',  base=(0xDE, 0x99, 0x43), style='stag'),
    dict(name='palisade', base=(0xDE, 0x99, 0x43), style='logs'),
]


def wood_base(v, rng):
    img = blank()
    base = v['base']
    st = v['style']
    rect(img, 0, 0, 31, 31, mul(base, 0.80))

    if st == 'horiz':
        for y in range(0, S, 8):
            rect(img, 0, y, 31, y + 6, base)
            rect(img, 0, y, 31, y, mul(base, 1.12))
            rect(img, 0, y + 6, 31, y + 6, mul(base, 0.84))
            for _ in range(4):
                gx = rng.randint(0, 27)
                gy = y + rng.randint(2, 5)
                rect(img, gx, gy, gx + rng.randint(2, 5), gy, mul(base, 0.88))
    elif st == 'vert':
        for x in range(0, S, 8):
            rect(img, x, 0, x + 6, 31, base)
            rect(img, x, 0, x, 31, mul(base, 1.12))
            rect(img, x + 6, 0, x + 6, 31, mul(base, 0.84))
            for _ in range(4):
                gy = rng.randint(0, 27)
                gx = x + rng.randint(2, 5)
                rect(img, gx, gy, gx, gy + rng.randint(2, 5), mul(base, 0.88))
    elif st == 'stag':
        for r, y in enumerate(range(0, S, 8)):
            rect(img, 0, y, 31, y + 6, base)
            rect(img, 0, y, 31, y, mul(base, 1.12))
            rect(img, 0, y + 6, 31, y + 6, mul(base, 0.84))
            off = 0 if r % 2 == 0 else 8
            for jx in range(off, S + 16, 16):        # staggered butt joints
                rect(img, jx, y, jx, y + 6, mul(base, 0.76))
            for _ in range(3):
                gx = rng.randint(0, 27)
                gy = y + rng.randint(2, 5)
                rect(img, gx, gy, gx + rng.randint(2, 4), gy, mul(base, 0.88))
    else:  # logs
        for x in range(0, S, 8):
            rect(img, x, 0, x + 7, 31, base)
            rect(img, x, 0, x + 1, 31, mul(base, 1.16))
            rect(img, x + 2, 0, x + 2, 31, mul(base, 1.06))
            rect(img, x + 6, 0, x + 7, 31, mul(base, 0.78))
            for gy in range(2, S, 9):
                rect(img, x + 3, gy, x + 5, gy, mul(base, 0.86))
    return img


def flame_patch(img, cx, cy, r, rng):
    pts = []
    for _ in range(7):
        a = rng.random() * math.tau
        d = rng.random() * r * 0.65
        pts.append((cx + math.cos(a) * d, cy + math.sin(a) * d))
    for (bx, by) in pts:
        disc(img, bx, by, r * (0.34 + 0.32 * rng.random()), FIRE_D)
    for (bx, by) in pts[:5]:
        disc(img, bx, by - 0.4, r * (0.22 + 0.22 * rng.random()), FIRE_M)
    for (bx, by) in pts[:3]:
        disc(img, bx, by - 0.6, r * (0.12 + 0.14 * rng.random()), FIRE_C)
    for _ in range(5):
        a = rng.random() * math.tau
        tx, ty = cx + math.cos(a) * r * 0.95, cy + math.sin(a) * r * 0.95
        px(img, tx, ty, FIRE_M)
        px(img, tx, ty - 1, FIRE_D)


def draw_wood(v, state, seed):
    rng = random.Random(seed)
    base = v['base']

    if state == 3:                                   # destroyed
        img = wood_base(v, rng)
        vertical = v['style'] in ('vert', 'logs')
        for band in range(0, S, 8):
            if rng.random() < 0.22:
                if vertical:
                    clear(img, band, 0, band + 7, 31)
                else:
                    clear(img, 0, band, 31, band + 7)
            else:
                cut = rng.randint(4, 12)
                if rng.random() < 0.5:
                    if vertical:
                        clear(img, band, 0, band + 7, cut)
                    else:
                        clear(img, 0, band, cut, band + 7)
                else:
                    if vertical:
                        clear(img, band, 31 - cut, band + 7, 31)
                    else:
                        clear(img, 31 - cut, band, 31, band + 7)
        for _ in range(10):
            px(img, rng.randint(0, 31), rng.randint(0, 31), mul(base, 0.70))
        declutter(img, 5)
        inner_shadow(img)
        return img

    if state == 7:                                   # charred
        img = wood_base(v, rng)
        for y in range(S):
            for x in range(S):
                c = img.getpixel((x, y))
                if c[3] == 0:
                    continue
                f = 0.50
                img.putpixel((x, y), snap((int(c[0] * f + SCORCH[0] * (1 - f)),
                                           int(c[1] * f + SCORCH[1] * (1 - f)),
                                           int(c[2] * f + SCORCH[2] * (1 - f)), 255)))
        for _ in range(14):
            a = rng.random() * math.tau
            r = 11 + rng.random() * 6
            disc_clear(img, 15.5 + math.cos(a) * r, 15.5 + math.sin(a) * r,
                       rng.uniform(1.6, 3.0))
        for _ in range(9):
            px(img, rng.randint(0, 31), rng.randint(0, 31), EMBER)
        for _ in range(5):
            px(img, rng.randint(0, 31), rng.randint(0, 31), (0x4C, 0x52, 0x3C, 255))
        declutter(img, 5)
        inner_shadow(img)
        return img

    img = wood_base(v, rng)

    if state in (1, 2):
        n = 3 if state == 1 else 7
        for _ in range(n):
            cx, cy = rng.randint(3, 28), rng.randint(3, 28)
            disc(img, cx, cy, rng.uniform(1.2, 2.6), SCORCH)
            if state == 2 and rng.random() < 0.6:
                disc_clear(img, cx, cy, rng.uniform(1.0, 2.0))
        for _ in range(n * 2):
            x0, y0 = rng.randint(1, 30), rng.randint(1, 30)
            crack(img, x0, y0, x0 + rng.randint(-8, 8), y0 + rng.randint(-8, 8),
                  mul(base, 0.68), rng)
        if state == 2:
            for _ in range(12):
                a = rng.random() * math.tau
                r = 11 + rng.random() * 6
                disc_clear(img, 15.5 + math.cos(a) * r, 15.5 + math.sin(a) * r,
                           rng.uniform(1.2, 2.4))

    if state in (4, 5, 6):
        frame = state - 4
        for _ in range(4):
            splat(img, rng.randint(4, 27), rng.randint(4, 27),
                  rng.uniform(2.0, 3.6), SCORCH, rng, 4)
        spots = [(9, 11), (21, 9), (14, 20), (24, 22), (7, 24), (18, 14)]
        rot = spots[frame:] + spots[:frame]
        for i, (fx, fy) in enumerate(rot[:4]):
            flame_patch(img, fx + rng.randint(-2, 2), fy + rng.randint(-2, 2),
                        3.8 + ((i + frame) % 3) * 0.9, rng)
        for _ in range(7):
            px(img, rng.randint(0, 31), rng.randint(0, 31), EMBER)
        for _ in range(6):
            px(img, rng.randint(0, 31), rng.randint(0, 31), (0x5D, 0x65, 0x4F, 190))

    declutter(img, 4)
    inner_shadow(img)
    return img


# ======================================================================
# GLASS -- plain pane + wire-mesh reinforced (mesh tiles on an 8px grid)
# ======================================================================
GLASSES = [dict(name='pane', mesh=False), dict(name='reinforced', mesh=True)]
MESH = (0x4C, 0x52, 0x3C, 235)


def draw_glass(v, state, seed):
    rng = random.Random(seed)
    img = blank()
    CRACK_D = (0x03, 0x8A, 0xAB, 205)
    CRACK_L = (0xDA, 0xE5, 0xCE, 245)

    rect(img, 0, 0, 31, 31, (GL_M[0], GL_M[1], GL_M[2], 138))
    # subtle tiling ripple instead of a corner gradient (which cannot tile)
    for y in range(S):
        for x in range(S):
            if (x // 4 + y // 4) % 2 == 0:
                px(img, x, y, (GL_D[0], GL_D[1], GL_D[2], 54))
    # diagonal sheen on a 16 x 16 lattice so streaks continue across tiles
    for sy in range(0, S, 16):
        for sx in range(-16, S + 16, 16):
            for k in range(9):
                px(img, sx + k, sy + k, (GL_L[0], GL_L[1], GL_L[2], 185))
                px(img, sx + k + 1, sy + k, (GL_L[0], GL_L[1], GL_L[2], 115))

    def draw_mesh():
        for g in range(0, S, 8):
            rect(img, g, 0, g, 31, MESH)
            rect(img, 0, g, 31, g, MESH)

    if v['mesh']:
        draw_mesh()

    if state > 0:
        pts = [(rng.randint(6, 25), rng.randint(6, 25)) for _ in range(state)]
        for (cx, cy) in pts:
            for _ in range(5 + state * 3):
                a = rng.random() * math.tau
                ln = rng.uniform(7, 16)
                ex, ey = cx + math.cos(a) * ln, cy + math.sin(a) * ln
                crack(img, cx, cy, ex, ey, CRACK_D, rng, 1)
                crack(img, cx + 1, cy, ex + 1, ey, CRACK_L, rng, 0)
            for rr in ([3.5] if state == 1 else [3.5, 6.5, 9.5])[:state + 1]:
                for t in range(int(rr * 9)):
                    a = t / (rr * 9) * math.tau
                    if rng.random() < 0.78:
                        px(img, cx + math.cos(a) * rr, cy + math.sin(a) * rr, CRACK_D)
            disc(img, cx, cy, 1.6, CRACK_L)
            disc(img, cx, cy, 0.9, (GL_D[0], GL_D[1], GL_D[2], 235))

    if state == 3:
        for _ in range(11):
            a0 = rng.random() * math.tau
            spread = rng.uniform(0.35, 0.85)
            for t in range(26):
                aa = a0 + spread * (t / 26.0)
                for rr in range(2, 16):
                    xx, yy = 15.5 + math.cos(aa) * rr, 15.5 + math.sin(aa) * rr
                    if 0 <= int(xx) < S and 0 <= int(yy) < S:
                        img.putpixel((int(xx), int(yy)), (0, 0, 0, 0))
        if v['mesh']:
            draw_mesh()                       # wire survives the glass
        src = img.copy()
        for y in range(S):
            for x in range(S):
                if src.getpixel((x, y))[3] == 0:
                    continue
                for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                    nx, ny = x + dx, y + dy
                    if 0 <= nx < S and 0 <= ny < S and src.getpixel((nx, ny))[3] == 0:
                        px(img, x, y, CRACK_L)
                        break

    declutter(img, 4)
    return img


# ======================================================================
COLS, ROWS = 8, 14
sheet = Image.new('RGBA', (S * COLS, S * ROWS), (0, 0, 0, 0))


def place(cell, col, row):
    sheet.paste(cell, (col * S, row * S))   # direct copy: preserves exact RGBA


for r, v in enumerate(BRICKS):
    for c in range(6):
        place(draw_brick(v, c, 100 + r * 31 + c * 7), c, r)
for r, v in enumerate(IRONS):
    for c in range(4):
        place(draw_iron(v, c, 200 + r * 31 + c * 7), c, 4 + r)
for r, v in enumerate(WOODS):
    for c in range(8):
        place(draw_wood(v, c, 300 + r * 31 + c * 7), c, 8 + r)
for r, v in enumerate(GLASSES):
    for c in range(4):
        place(draw_glass(v, c, 400 + r * 31 + c * 7), c, 12 + r)

# Chunky-pixelate the finished sheet to match the tanks' own look: tanks
# draw a 32x32 source tile at Tank::scale=2.0 (tank.rs), so every source
# pixel covers a 2x2 screen block; obstacles draw at OBSTACLE_SCALE=1.0
# (lib.rs), so their pixels map 1:1 and read as crisper/finer than tanks
# despite identical source resolution - not more detailed, just unstretched.
# Rather than bumping OBSTACLE_SCALE itself (which would double obstacles'
# on-screen size and break OBSTACLE_GRID_SIZE/pathfinding-cell/collider
# alignment, all of which assume Obstacle::size() == OBSTACLE_TEXTURE_SIZE),
# bake the same 2x chunkiness into the art instead: downsample-then-upsample
# with NEAREST (never a box/average filter - that would blend adjacent
# pixels into new off-palette colours, drifting off the Puny Palette set
# every sheet here snaps to, see punypalette.py) so each 2x2 block collapses
# to one flat, already-on-palette colour, then blows back up to the same
# 2x2 block - the same flat-block mechanism the game's own 2x render stretch
# gives tanks, just baked into the source pixels instead of happening at
# draw time. PIXELATE_FACTOR matches Tank::scale exactly, not a separate
# guess, so obstacles end up exactly as chunky as tanks already are, no
# more and no less. 32px tiles and a 2x factor both divide evenly, so
# downsample blocks never straddle a tile boundary.
PIXELATE_FACTOR = 2


def pixelate(img, factor):
    w, h = img.size
    small = img.resize((w // factor, h // factor), Image.NEAREST)
    return small.resize((w, h), Image.NEAREST)


sheet = pixelate(sheet, PIXELATE_FACTOR)
sheet.save(f'{OUT}/walls_sheet.png')
print('walls_sheet.png', sheet.size)

sc = 6
