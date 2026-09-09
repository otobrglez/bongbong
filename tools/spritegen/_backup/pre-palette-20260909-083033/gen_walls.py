from PIL import Image
import os, random, math, sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from punypalette import (
    STONE_DARKEST, STONE_DK, STONE_LT, STONE_MD, STONE_PALE,
    RED_DARKEST,
    SAND_DK, SAND_LT, SAND_MD, SAND_PALE,
    WOOD_DARKEST, WOOD_DEEPER, WOOD_DK, WOOD_LT, WOOD_MD, WOOD_PALE,
    snap,
)

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


# Every primitive below draws a GRID x GRID block rather than a lone pixel.
# The finished sheet is downsampled by PIXELATE_FACTOR (== GRID) and scaled
# back up with NEAREST (see pixelate() at the bottom), which keeps one pixel
# out of each block and discards the other three - so a one-pixel detail
# used to survive or vanish purely on the parity of where it landed, and
# roughly three quarters of the tick marks, rivet highlights, plank edges
# and cracks drawn here never reached the screen. Snapping every write to
# the block grid makes that downsample lossless: what this file draws is
# exactly what ships. Coordinates stay in the 0..31 space every routine is
# written in - only their effective resolution is halved, to the 16x16 the
# sheet has always actually had. gen_props.py reaches the same density the
# other way round, by drawing at 16 and scaling up per cell.
GRID = 2


def _snap(v):
    return (int(v) // GRID) * GRID


def px(img, x, y, c):
    """Full-bleed: the whole 0..31 cell is drawable, no reserved border.
    Writes the whole GRID x GRID block holding (x, y) - see GRID above."""
    if c is None:
        return
    x, y = _snap(x), _snap(y)
    for dy in range(GRID):
        for dx in range(GRID):
            if 0 <= x + dx < S and 0 <= y + dy < S:
                img.putpixel((x + dx, y + dy), c)


def rect(img, x0, y0, x1, y1, c):
    for y in range(int(y0), int(y1) + 1):
        for x in range(int(x0), int(x1) + 1):
            px(img, x, y, c)


def clear(img, x0, y0, x1, y1):
    for y in range(int(y0), int(y1) + 1):
        for x in range(int(x0), int(x1) + 1):
            px(img, x, y, (0, 0, 0, 0))


def disc(img, cx, cy, r, c):
    for y in range(int(cy - r) - 1, int(cy + r) + 2):
        for x in range(int(cx - r) - 1, int(cx + r) + 2):
            if (x - cx) ** 2 + (y - cy) ** 2 <= r * r:
                px(img, x, y, c)


def disc_clear(img, cx, cy, r):
    for y in range(int(cy - r) - 1, int(cy + r) + 2):
        for x in range(int(cx - r) - 1, int(cx + r) + 2):
            if (x - cx) ** 2 + (y - cy) ** 2 <= r * r:
                px(img, x, y, (0, 0, 0, 0))


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
    outline. Cell edges are NOT treated as holes, so tiles stay seamless.

    Works a whole block at a time (see GRID): shading only the pixels that
    literally touch the hole would leave half of each block undarkened, and
    the downsample would then keep whichever half it keeps - so the depth
    cue survived or vanished at random. Darkening the block is what makes
    it survive."""
    src = img.copy()
    for by in range(0, S, GRID):
        for bx in range(0, S, GRID):
            c = src.getpixel((bx, by))
            if c[3] == 0:
                continue
            touches_hole = False
            for dy in range(GRID):
                for dx in range(GRID):
                    x, y = bx + dx, by + dy
                    for ox, oy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                        nx, ny = x + ox, y + oy
                        if not (0 <= nx < S and 0 <= ny < S):
                            continue              # tile border: not a hole
                        if src.getpixel((nx, ny))[3] == 0:
                            touches_hole = True
            if touches_hole:
                px(img, bx, by, mul(c, f))


# Scatter counts and per-pixel odds throughout this file were authored back
# when the downsample threw away roughly three pixels in four, so they were
# implicitly tuned against that loss. Now every speck lands, and lands as a
# full GRID x GRID block, so the same numbers read as noise rather than as
# grit. Scale them back to the density the art was actually drawn for.
SPECK_SCALE = 0.3


def specks(n):
    """Scatter count `n`, rescaled for the block grid - never below 1 for a
    non-zero request, so a stage that is meant to show a little debris
    always shows some."""
    return max(1, round(n * SPECK_SCALE)) if n else 0


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
# Every period is a multiple of 2*GRID and the mortar gap brick_cells
# leaves is GRID wide, so a course is (ph/GRID - 1) design pixels of brick
# face over exactly one of mortar. A 4-px course (3 px face + a 1 px seam)
# was drawn for a 32-px grid and does not survive at this density: the
# seam and the face land in the same block and the bond reads as blank
# banding. 8 px is the smallest course that still shows mortar.
BRICKS = [
    dict(name='running', base=STONE_LT, pw=16, ph=8,  bond='run'),
    dict(name='block',   base=STONE_LT, pw=16, ph=16, bond='run'),
    dict(name='long',    base=STONE_LT, pw=32, ph=8,  bond='run'),
    dict(name='stacked', base=STONE_LT, pw=8,  ph=8,  bond='stack'),
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
    # Only the large 'block' bond's panels are big enough to read as a few
    # subtly-toned stones; a running-bond chip is 3 design pixels tall, and
    # stippling it just bleaches the face and erases the mortar contrast the
    # bond depends on. The threshold is in 32-px coordinates, so it is
    # 6 x 5 design pixels - the same call the original 6x5 guard made before
    # the courses were rescaled to the block grid.
    if w < 12 or h < 10:
        return
    sub_w = max(3, w // 3)
    sub_h = max(3, h // 2)
    sy = y0 + 1
    while sy < y1 - 1:
        sx = x0 + 1
        while sx < x1 - 1:
            if rng.random() < 0.25:
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
            if 0 <= tx < S and rng.random() < 0.4 * SPECK_SCALE:
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
            # rect() is inclusive, so the face has to stop GRID+1 short of
            # the next course for the mortar to get a whole block of its
            # own - at GRID short, the face's last block covers the seam
            # and the bond reads as flat banding.
            cells.append((x, y, x + pw - GRID - 1, y + ph - GRID - 1))
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
    for _ in range(specks(decay * 3)):
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
        for _ in range(specks(8)):
            px(img, rng.randint(0, 31), rng.randint(0, 31), DUST)

    declutter(img, 5)
    inner_shadow(img)
    return img


# ======================================================================
# IRON -- clean steel, rust arrives with damage
# ======================================================================
# Same one-shared-tone approach as BRICKS -- "all four share a steel tone...
# differing by surface treatment" (per WALLS_SPEC.md). De-green pass
# (2026-08): the steel tone was #5D654F, sampled from Puny World's building
# walls, which are really green-grey -- iron walls read as olive. Now the
# true neutral STONE_DK grey (from the pack's rock/well props), one step
# darker than brick's STONE_LT base so the two grey materials still
# separate at a glance; rust accents unchanged.
IRONS = [
    dict(name='riveted',    base=STONE_DK, style='rivet'),
    dict(name='corrugated', base=STONE_DK, style='corr'),
    dict(name='banded',     base=STONE_DK, style='band'),
    dict(name='tread',      base=STONE_DK, style='tread'),
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
        # 16px lattice -> rivet grid continues across tile joins.
        # A stud is two design pixels: lit top-left, shadowed bottom-right.
        # The highlight and both shadow pixels used to sit 1 px apart, i.e.
        # inside a single block, so the shadow overwrote the highlight and
        # every rivet flattened into a plain dark dot.
        for ry in (0, 16):
            for rx in (0, 16):
                for (ox, oy) in ((4, 4), (12, 12)):
                    px(img, rx + ox, ry + oy, mul(base, 1.30))
                    px(img, rx + ox + GRID, ry + oy + GRID, mul(base, 0.70))
        for y in range(0, S, 16):
            rect(img, 0, y, 31, y, mul(base, 1.08))
            rect(img, 0, y + 15, 31, y + 15, mul(base, 0.88))
    elif st == 'corr':
        # A 4px period is exactly two design pixels, so a rib is one lit
        # block and one shadowed one - there is no room for the third,
        # mid-tone line the old version drew, and that line landed in the
        # highlight's own block and erased it.
        for x in range(0, S, 4):
            rect(img, x, 0, x + 1, 31, mul(base, 1.20))
            rect(img, x + 2, 0, x + 3, 31, mul(base, 0.80))
    elif st == 'band':
        # Three design pixels tall (lit / mid / shadowed) with the studs on
        # the mid row. At the old 4px height the mid-tone had nowhere to go:
        # the lit and shadowed rows covered both of the band's blocks.
        for y in range(0, S, 16):
            rect(img, 0, y, 31, y + 5, mul(base, 0.86))
            rect(img, 0, y, 31, y + 1, mul(base, 1.14))
            rect(img, 0, y + 4, 31, y + 5, mul(base, 0.74))
            for rx in range(4, 32, 8):
                px(img, rx, y + 2, mul(base, 1.32))
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
    for _ in range(specks(n_fleck)):
        px(img, rng.randint(0, 31), rng.randint(0, 31), RUST_L)

    if dmg > 0:
        for _ in range(dmg * 3):
            cx, cy = rng.randint(2, 29), rng.randint(2, 29)
            r = rng.uniform(1.4, 2.6)
            disc(img, cx, cy, r, mul(base, 0.68))
            disc(img, cx - 0.6, cy - 0.6, r * 0.55, mul(base, 0.94))
        for _ in range(specks(dmg * 2)):
            px(img, rng.randint(0, 31), rng.randint(0, 31), SCORCH)
        for _ in range(dmg * 2):
            x0, y0 = rng.randint(2, 29), rng.randint(2, 29)
            crack(img, x0, y0, x0 + rng.randint(-7, 7), y0 + rng.randint(-7, 7),
                  mul(base, 0.64), rng)
        for _ in range(dmg):
            splat(img, rng.randint(4, 27), rng.randint(4, 27),
                  rng.uniform(2.2, 3.6), STONE_DARKEST + (255,), rng, 4)
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
        for _ in range(specks(10)):
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
        for _ in range(specks(9)):
            px(img, rng.randint(0, 31), rng.randint(0, 31), EMBER)
        for _ in range(specks(5)):
            px(img, rng.randint(0, 31), rng.randint(0, 31), STONE_DARKEST + (255,))
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
        for _ in range(specks(7)):
            px(img, rng.randint(0, 31), rng.randint(0, 31), EMBER)
        for _ in range(specks(6)):
            px(img, rng.randint(0, 31), rng.randint(0, 31), STONE_DK + (190,))

    declutter(img, 4)
    inner_shadow(img)
    return img


# ======================================================================
# GLASS -- plain pane + wire-mesh reinforced (mesh tiles on an 8px grid)
# ======================================================================
GLASSES = [dict(name='pane', mesh=False), dict(name='reinforced', mesh=True)]
MESH = STONE_DARKEST + (235,)


def draw_glass(v, state, seed):
    rng = random.Random(seed)
    img = blank()
    CRACK_D = (0x03, 0x8A, 0xAB, 205)
    CRACK_L = STONE_PALE + (245,)

    rect(img, 0, 0, 31, 31, (GL_M[0], GL_M[1], GL_M[2], 138))
    # subtle tiling ripple instead of a corner gradient (which cannot tile)
    for y in range(S):
        for x in range(S):
            if (x // 4 + y // 4) % 2 == 0:
                px(img, x, y, (GL_D[0], GL_D[1], GL_D[2], 54))
    # diagonal sheen on a 16 x 16 lattice so streaks continue across tiles
    for sy in range(0, S, 16):
        for sx in range(-16, S + 16, 16):
            # Step the diagonal a whole block at a time and put the
            # softer trailing tone in its own block: stepping by 1 px put
            # consecutive steps inside the same block, so the streak
            # collapsed to whichever tone happened to be written last.
            for k in range(0, 18, GRID):
                px(img, sx + k, sy + k, (GL_L[0], GL_L[1], GL_L[2], 185))
                px(img, sx + k + GRID, sy + k, (GL_L[0], GL_L[1], GL_L[2], 115))

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
                # Offset by a whole block: at +1 it snapped onto the same
                # block as the dark crack above and simply overwrote it,
                # so the fracture lost the lit edge that sells it as glass.
                crack(img, cx + GRID, cy, ex + GRID, ey, CRACK_L, rng, 0)
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
# ======================================================================
# RUBBLE -- what a destroyed tile leaves on the ground (decal.rs)
# ======================================================================
# Unlike every block above these are NOT full-bleed: rubble is scattered
# debris over transparent ground, and the whole point of the row is that a
# levelled wall does not read as a grid of clones. Two things fight that.
# First, RUBBLE_VARIANTS distinct cells per material, picked per tile from
# a position hash (decal.rs) - combined with the mirror and quarter-turn
# that draw_decal already applies, one material has 8 x 8 apparent forms.
# Second, coverage *ramps across the variants*, from a few scattered chips
# to a dense pile, so a large destroyed area gets sparse and heavy patches
# instead of one uniform texture.
#
# Drawn in design-pixel space (D x D) rather than sheet pixels: rubble is
# all small chunks, and at this size the difference between a 1- and a
# 2-design-pixel chip is the whole silhouette, so it is much easier to
# reason about the shapes at the resolution they actually ship at.
RUBBLE_VARIANTS = 8
D = S // GRID


def drect(img, dx, dy, w, h, c):
    """A chunk `w` x `h` *design* pixels at design-pixel (dx, dy)."""
    rect(img, dx * GRID, dy * GRID, (dx + w) * GRID - 1, (dy + h) * GRID - 1, c)


def dpx(img, dx, dy, c):
    px(img, dx * GRID, dy * GRID, c)


def chunk(img, rng, dx, dy, w, h, pair):
    """A chunk with a darker bottom edge, so it reads as sitting on the
    ground rather than as a flat sticker. Chunks only one design pixel
    tall have no room for the edge and stay flat.

    `pair` is an explicit (tone, shade) from the same palette family, not
    a `mul()` of the tone: snap() is nearest-Euclidean over the whole
    41-colour set, so darkening a sand or wood tone by eye lands it in the
    *green* family - which is exactly the olive the de-green pass
    (docs/PALETTE.md) exists to keep off every object in the game."""
    tone, shade = pair
    drect(img, dx, dy, w, h, tone)
    if h >= 2:
        drect(img, dx, dy + h - 1, w, 1, shade)


def C255(c):
    return c + (255,)


# (tone, shade) pairs per family - always an adjacent step down the same
# ramp, never a computed darkening. See chunk().
BRICK_PAIRS = [(C255(STONE_PALE), C255(STONE_LT)), (C255(STONE_LT), C255(STONE_MD)),
               (C255(STONE_LT), C255(STONE_MD)), (C255(STONE_MD), C255(STONE_DK)),
               (C255(STONE_DK), C255(STONE_DARKEST))]
WOOD_PAIRS = [(C255(WOOD_PALE), C255(WOOD_LT)), (C255(WOOD_LT), C255(WOOD_MD)),
              (C255(WOOD_MD), C255(WOOD_DK)), (C255(WOOD_DK), C255(WOOD_DEEPER)),
              (C255(WOOD_DEEPER), C255(WOOD_DARKEST))]
CHAR_PAIRS = [(SCORCH, C255(STONE_DARKEST)), (C255(WOOD_DARKEST), C255(STONE_DARKEST)),
              (C255(WOOD_DEEPER), C255(WOOD_DARKEST)), (C255(STONE_DARKEST), C255(STONE_DARKEST))]
SAND_PAIRS = [(C255(SAND_PALE), C255(SAND_LT)), (C255(SAND_LT), C255(SAND_MD)),
              (C255(SAND_MD), C255(SAND_DK)), (C255(SAND_DK), C255(WOOD_DEEPER))]
METAL_PAIRS = [(C255(STONE_MD), C255(STONE_DK)), (C255(STONE_DK), C255(STONE_DARKEST)),
               (C255(STONE_LT), C255(STONE_MD)), (C255(STONE_DARKEST), C255(STONE_DARKEST))]


def rubble_count(variant, low, high):
    """Chunk count for `variant`, ramped low -> high across the set."""
    t = variant / max(1, RUBBLE_VARIANTS - 1)
    return int(round(low + (high - low) * t))


def draw_rubble_brick(variant, seed):
    rng = random.Random(seed)
    img = blank()
    tones = BRICK_PAIRS
    for _ in range(rubble_count(variant, 4, 20)):
        w = rng.choice([1, 1, 2, 2, 3])
        h = rng.choice([1, 1, 2])
        chunk(img, rng, rng.randint(0, D - w), rng.randint(0, D - h), w, h, rng.choice(tones))
    # Mortar dust between the chunks - the powder a broken wall leaves.
    for _ in range(rubble_count(variant, 1, 6)):
        dpx(img, rng.randint(0, D - 1), rng.randint(0, D - 1), DUST)
    declutter(img, 2)
    return img


def draw_rubble_wood(variant, seed, charred):
    rng = random.Random(seed)
    img = blank()
    if charred:
        tones = CHAR_PAIRS
    else:
        # Reach for both ends of the wood ramp, not just its middle: the
        # ground under a destroyed tile is the road layer's tan, and the
        # mid browns sit close enough to it that a splinter pile reads as
        # texture on the dirt rather than as debris on top of it.
        tones = WOOD_PAIRS
    # Splinters: long and thin, and lying along one axis or the other, so a
    # pile reads as broken boards rather than as gravel.
    for _ in range(rubble_count(variant, 4, 18)):
        long_side = rng.choice([2, 3, 3, 4, 5])
        w, h = (long_side, 1) if rng.random() < 0.5 else (1, long_side)
        chunk(img, rng, rng.randint(0, D - w), rng.randint(0, D - h), w, h, rng.choice(tones))
    if charred:
        # A few embers still glowing in the ash.
        for _ in range(rubble_count(variant, 1, 4)):
            dpx(img, rng.randint(0, D - 1), rng.randint(0, D - 1), rng.choice([EMBER, FIRE_D]))
    declutter(img, 2)
    return img


def draw_rubble_glass(variant, seed):
    rng = random.Random(seed)
    img = blank()
    body = (GL_M[0], GL_M[1], GL_M[2], 200)
    dark = (GL_D[0], GL_D[1], GL_D[2], 210)
    lit = (GL_L[0], GL_L[1], GL_L[2], 225)
    # Shards are small and sharp - mostly single design pixels with the
    # occasional two-pixel sliver, and a white glint on some of them. Glass
    # scatters further than masonry, so the counts run higher and the
    # pieces smaller.
    for _ in range(rubble_count(variant, 5, 26)):
        w, h = (2, 1) if rng.random() < 0.3 else (1, 1)
        dx, dy = rng.randint(0, D - w), rng.randint(0, D - h)
        drect(img, dx, dy, w, h, rng.choice([body, body, dark, lit]))
        if rng.random() < 0.22:
            dpx(img, dx, dy, STONE_PALE + (245,))
    declutter(img, 1)
    return img


def draw_rubble_sandbag(variant, seed):
    rng = random.Random(seed)
    img = blank()
    sand = SAND_PAIRS
    # A burst bag spills its fill rather than breaking into pieces, so this
    # is low rounded mounds of grain, denser than any of the wall rubble.
    for _ in range(rubble_count(variant, 6, 24)):
        w = rng.choice([2, 2, 3, 3, 4])
        h = rng.choice([1, 2, 2])
        chunk(img, rng, rng.randint(0, D - w), rng.randint(0, D - h), w, h, rng.choice(sand))
    # Torn hessian left among the spill.
    for _ in range(rubble_count(variant, 1, 4)):
        w = rng.choice([2, 3])
        dx, dy = rng.randint(0, D - w), rng.randint(0, D - 1)
        drect(img, dx, dy, w, 1, rng.choice([WOOD_DEEPER + (255,), WOOD_DARKEST + (255,)]))
    declutter(img, 2)
    return img


def draw_rubble_barrel(variant, seed):
    rng = random.Random(seed)
    img = blank()
    metal = METAL_PAIRS
    burnt = [SCORCH, RED_DARKEST + (255,)]
    # A barrel goes off rather than falling over: what is left is twisted
    # staves and burnt scrap. The blast already lays a Scorch under this,
    # so the debris itself carries the metal and only a little char.
    for _ in range(rubble_count(variant, 4, 16)):
        long_side = rng.choice([2, 3, 3, 4])
        w, h = (long_side, 1) if rng.random() < 0.5 else (1, long_side)
        chunk(img, rng, rng.randint(0, D - w), rng.randint(0, D - h), w, h, rng.choice(metal))
    for _ in range(rubble_count(variant, 2, 7)):
        dpx(img, rng.randint(0, D - 1), rng.randint(0, D - 1), rng.choice(burnt))
    declutter(img, 2)
    return img


def draw_rubble_fence(variant, seed):
    rng = random.Random(seed)
    img = blank()
    wood = WOOD_PAIRS
    wire = [C255(STONE_MD), C255(STONE_DK)]
    # A fence is mostly air to begin with, so it leaves the least of
    # anything here: a few snapped pickets and some loose wire.
    for _ in range(rubble_count(variant, 2, 9)):
        long_side = rng.choice([2, 3, 4])
        w, h = (long_side, 1) if rng.random() < 0.5 else (1, long_side)
        chunk(img, rng, rng.randint(0, D - w), rng.randint(0, D - h), w, h, rng.choice(wood))
    for _ in range(rubble_count(variant, 1, 5)):
        dpx(img, rng.randint(0, D - 1), rng.randint(0, D - 1), rng.choice(wire))
    declutter(img, 1)
    return img


def draw_rubble_tank(variant, seed):
    rng = random.Random(seed)
    img = blank()
    # Blown-off tank parts: bigger and blockier than any tile debris,
    # because the pieces are hull plate and track link rather than chips.
    # Kept to neutral steel plus char - the chassis palettes differ per
    # row on the tank sheet, and tinting wreckage to match would mean one
    # of these per chassis for a detail nobody reads at this size.
    for _ in range(rubble_count(variant, 2, 8)):
        w = rng.choice([2, 3, 3, 4])
        h = rng.choice([2, 2, 3])
        chunk(img, rng, rng.randint(0, D - w), rng.randint(0, D - h), w, h, rng.choice(METAL_PAIRS))
    # Track links: short even runs, the one part of a tank that reads as
    # itself at three design pixels.
    for _ in range(rubble_count(variant, 1, 5)):
        n = rng.choice([2, 3])
        dx, dy = rng.randint(0, D - n * 2), rng.randint(0, D - 1)
        for k in range(n):
            drect(img, dx + k * 2, dy, 1, 1, C255(STONE_DARKEST))
    for _ in range(rubble_count(variant, 1, 4)):
        dpx(img, rng.randint(0, D - 1), rng.randint(0, D - 1), rng.choice([SCORCH, C255(RED_DARKEST)]))
    declutter(img, 2)
    return img


COLS, ROWS = 8, 22
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

# Rubble rows - one row per leftover kind, RUBBLE_VARIANTS columns each.
# Kept after every material block so nothing above has to be renumbered
# (lib.rs's RUBBLE_ROW_* and Material::rubble_row index straight into these).
for c in range(RUBBLE_VARIANTS):
    place(draw_rubble_brick(c, 500 + c * 7), c, 14)
    place(draw_rubble_wood(c, 600 + c * 7, False), c, 15)
    place(draw_rubble_wood(c, 700 + c * 7, True), c, 16)
    place(draw_rubble_glass(c, 800 + c * 7), c, 17)
    place(draw_rubble_sandbag(c, 900 + c * 7), c, 18)
    place(draw_rubble_barrel(c, 1000 + c * 7), c, 19)
    place(draw_rubble_fence(c, 1100 + c * 7), c, 20)
    place(draw_rubble_tank(c, 1200 + c * 7), c, 21)

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
#
# Since every primitive now writes whole GRID x GRID blocks on that same
# grid (see px() at the top), this pass is a no-op that only ever confirms
# the invariant - it can no longer silently eat a detail. Keep it: it is
# cheap, and it is what guarantees the sheet is genuinely block-flat if a
# routine ever writes a stray pixel directly.
PIXELATE_FACTOR = GRID


def pixelate(img, factor):
    w, h = img.size
    small = img.resize((w // factor, h // factor), Image.NEAREST)
    return small.resize((w, h), Image.NEAREST)


sheet = pixelate(sheet, PIXELATE_FACTOR)
sheet.save(f'{OUT}/walls_sheet.png')
print('walls_sheet.png', sheet.size)

sc = 6
