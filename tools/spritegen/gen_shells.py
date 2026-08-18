from PIL import Image
import os, math

S = 32
OUT = os.environ.get('SPRITE_OUT', 'assets/sprites')
os.makedirs(OUT, exist_ok=True)

# ---- palettes lifted from the original shells.png -----------------------
# (smoke_rgb, smoke_a_dense, smoke_a_light, dark, mid, core)
FAMILIES = {
    'orange': dict(smoke=(120, 120, 128), sa=(220, 110),
                   D=(220, 70, 30), M=(255, 150, 40), C=(255, 236, 120)),
    'red':    dict(smoke=(90, 70, 66), sa=(230, 120),
                   D=(170, 35, 20), M=(235, 95, 30), C=(255, 200, 110)),
    'blue':   dict(smoke=(150, 170, 190), sa=(200, 100),
                   D=(80, 150, 235), M=(150, 220, 255), C=(235, 250, 255)),
}

# shared casing colors (identical across all rows in the original)
CASE_DK = (70, 72, 82, 255)
CASE_HI = (120, 122, 132, 255)
NOSE = (210, 180, 70, 255)
WHITE = (255, 255, 255, 255)

CX = CY = 15.5


def blank():
    return Image.new('RGBA', (S, S), (0, 0, 0, 0))


def put(img, x, y, c):
    if 0 <= x < S and 0 <= y < S and c is not None:
        img.putpixel((int(x), int(y)), c)


def disc(img, cx, cy, r, col):
    r2 = r * r
    for y in range(S):
        for x in range(S):
            dx, dy = x - cx, y - cy
            if dx * dx + dy * dy <= r2:
                put(img, x, y, col)


def annulus(img, cx, cy, r0, r1, col, dither=False):
    for y in range(S):
        for x in range(S):
            dx, dy = x - cx, y - cy
            d = (dx * dx + dy * dy) ** 0.5
            if r0 <= d <= r1:
                if dither and (x + y) % 2:
                    continue
                put(img, x, y, col)


def rays(img, cx, cy, length, col):
    """Four cardinal 1px rays, drawn on top of the disc (as in the original)."""
    for i in range(int(round(length)) + 1):
        put(img, cx + 0.5, cy - i, col)
        put(img, cx + 0.5, cy + i, col)
        put(img, cx - i, cy + 0.5, col)
        put(img, cx + i, cy + 0.5, col)


def sparkle(img, cx, cy, r):
    """Diamond of white pixels near the core, as in the original Fire0."""
    for dx, dy in ((-1, -2), (1, -2), (-2, -1), (2, -1),
                   (-2, 1), (2, 1), (-1, 2), (1, 2)):
        put(img, cx + 0.5 + dx * r / 6.0, cy + 0.5 + dy * r / 6.0, WHITE)


def blast(img, cx, cy, r, pal, kind='fire', ray_len=None, spark=False, ray_col=None):
    """Layered blast disc. 'fire' = M outer + C core; 'hit' = D + M + C."""
    if kind == 'fire':
        disc(img, cx, cy, r, pal['M'] + (255,))
        disc(img, cx, cy, r * 0.68, pal['C'] + (255,))
        rc = ray_col or pal['C'] + (255,)
    else:
        disc(img, cx, cy, r, pal['D'] + (255,))
        disc(img, cx, cy, r * 0.78, pal['M'] + (255,))
        disc(img, cx, cy, r * 0.40, pal['C'] + (255,))
        rc = ray_col or pal['M'] + (255,)
    if ray_len:
        rays(img, cx, cy, ray_len, rc)
    if spark:
        sparkle(img, cx, cy, r)


def mini_ring(img, cx, cy, r, col):
    """Small hollow ring -- the 4-dot detail inside the original Hit1."""
    for y in range(S):
        for x in range(S):
            d = ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5
            if r - 0.9 <= d <= r + 0.5:
                put(img, x, y, col)


def diamond(img, cx, cy, r, col):
    for y in range(S):
        for x in range(S):
            if abs(x - cx) + abs(y - cy) <= r:
                put(img, x, y, col)


def projectile(img, cx, top, nose_h, body_h, w):
    """Shell pointing up: gold nose tapering into a dark casing."""
    x0 = int(cx + 0.5 - w / 2.0)
    x1 = x0 + w - 1
    # nose: narrower for the first rows, then full width
    for i in range(nose_h):
        if i < nose_h - 1:
            for x in range(x0 + 1, x1):
                put(img, x, top + i, NOSE)
        else:
            for x in range(x0, x1 + 1):
                put(img, x, top + i, NOSE)
    # casing: leftmost dark, second column highlight, rest dark
    for j in range(body_h):
        y = top + nose_h + j
        for x in range(x0, x1 + 1):
            put(img, x, y, CASE_HI if x == x0 + 1 else CASE_DK)


def ellipse(img, cx, cy, rx, ry, col):
    for y in range(S):
        for x in range(S):
            dx, dy = (x - cx) / rx, (y - cy) / ry
            if dx * dx + dy * dy <= 1.0:
                put(img, x, y, col)


SPECKS = [(0.80, 0.55), (-0.72, 0.66), (0.62, -0.78), (-0.86, -0.40),
          (0.94, 0.12), (-0.35, 0.92)]


def smoke_e(img, cx, cy, rx, ry, pal, dense=True):
    a = pal['sa'][0] if dense else pal['sa'][1]
    rx, ry = min(rx, 15.4), min(ry, 15.4)
    ellipse(img, cx, cy, rx, ry, pal['smoke'] + (a,))
    for i, (ux, uy) in enumerate(SPECKS):
        put(img, cx + 0.5 + ux * rx * 1.08, cy + 0.5 + uy * ry * 1.08,
            pal['smoke'] + (a,))


def smoke(img, cx, cy, r, pal, dense=True):
    a = pal['sa'][0] if dense else pal['sa'][1]
    r = min(r, 15.4)
    disc(img, cx, cy, r, pal['smoke'] + (a,))
    for i, (ux, uy) in enumerate(SPECKS):
        rr = r * (1.06 + 0.05 * (i % 3))
        put(img, cx + 0.5 + ux * rr, cy + 0.5 + uy * rr, pal['smoke'] + (a,))


# ------------------------------------------------------------------
# Class geometry.
# 'std' is the previous art scaled to ~0.9 (regular tanks shrank 10%).
# 'super' is ~1.25x of std, for the super-heavy chassis. The outer smoke
# radius is capped at 15.2 so it still fits a 32px cell -- see notes.
# ------------------------------------------------------------------
CLASSES = {
    'std': dict(
        pw=4, nose=3, body=8,          # projectile
        f0_r=8.0,  f0_ray=12.5,
        f1_r=4.5,  f2_r=2.7,
        h0_r=6.3,  h0_ray=10.0,
        h1_smoke=10.0, h1_core=6.3, h1_ray=12.5,
        h2_smoke=13.5, h2_core=4.5,
        offset=0,
    ),
    'super': dict(
        pw=6, nose=4, body=11,
        f0_r=10.0, f0_ray=15.0,
        f1_r=5.6,  f2_r=3.4,
        h0_r=7.9,  h0_ray=13.0,
        h1_smoke=12.5, h1_core=7.9, h1_ray=15.0,
        h2_smoke=15.2, h2_core=5.6,
        offset=0,
    ),
}
# twin variants: two barrels, so paired projectiles / paired muzzle blast
TWIN_OFFSET = {'std': 3.0, 'super': 5.0}  # matches tank barrel gap (3 std, 5 titan)
TWIN_PW = {'std': 3, 'super': 4}


LAG = {'std': 4, 'super': 5}   # how far the delayed barrel trails, in px


def build_frames(family, cls, twin, lag=0):
    """lag > 0 -> the left barrel fires first and its round runs ahead."""
    pal = FAMILIES[family]
    G = CLASSES[cls]
    frames = []
    off = TWIN_OFFSET[cls] if twin else 0.0
    pw = TWIN_PW[cls] if twin else G['pw']
    xs = [CX - off, CX + off] if twin else [CX]
    lead, trail = (xs[0], xs[1]) if twin else (CX, CX)
    stag = twin and lag > 0
    mray = pal['M'] + (255,)

    # --- col 0: Fire0 -- muzzle blast ---
    f = blank()
    if stag:
        rb = G['f0_r'] * 0.72
        blast(f, lead, CY - lag * 0.5, rb, pal, 'fire', rb + 4.0, True)
        blast(f, trail, CY + 1, rb * 0.55, pal, 'fire', rb * 0.55 + 1.0, False, mray)
    elif twin:
        rb = G['f0_r'] * 0.72
        for x in xs:
            blast(f, x, CY, rb, pal, 'fire', rb + 4.0, True)
    else:
        blast(f, CX, CY, G['f0_r'], pal, 'fire', G['f0_ray'], True)
    frames.append(f)

    # --- col 1: Fire1 ---
    f = blank()
    by = min(7 + G['nose'] + G['body'] + int(G['f1_r']) - 1,
             int(30 - G['f1_r']))
    if stag:
        projectile(f, lead, 7 - lag, G['nose'], G['body'], pw)
        projectile(f, trail, 7, G['nose'], G['body'], pw)
        blast(f, trail, by, G['f1_r'] * 0.9, pal, 'fire', None, False)
        blast(f, lead, by - lag, G['f1_r'] * 0.45, pal, 'fire', None, False)
    else:
        for x in xs:
            projectile(f, x, 7, G['nose'], G['body'], pw)
        if twin:
            for x in xs:
                blast(f, x, by, G['f1_r'] * 0.8, pal, 'fire', None, False)
        else:
            blast(f, CX, by, G['f1_r'], pal, 'fire', G['f1_r'] + 1.0, False, mray)
        put(f, CX + 0.5, by, WHITE)
    frames.append(f)

    # --- col 2: Fire2 ---
    f = blank()
    by2 = min(8 + G['nose'] + G['body'] + int(G['f2_r']) + 1,
              int(30 - G['f2_r']))
    if stag:
        projectile(f, lead, 8 - lag, G['nose'], G['body'], pw)
        projectile(f, trail, 8, G['nose'], G['body'], pw)
        blast(f, trail, by2, G['f2_r'] * 1.1, pal, 'fire', None, False)
        put(f, lead + 0.5, by2 - lag, pal['M'] + (255,))
    else:
        for x in xs:
            projectile(f, x, 8, G['nose'], G['body'], pw)
        if twin:
            for x in xs:
                blast(f, x, by2, G['f2_r'], pal, 'fire', None, False)
        else:
            blast(f, CX, by2, G['f2_r'], pal, 'fire', G['f2_r'] + 1.0, False, mray)
    frames.append(f)

    # --- col 3: Flying -- the visible signature of the stagger ---
    f = blank()
    tops = [9 - lag, 9] if stag else [9] * len(xs)
    for x, t in zip(xs, tops):
        projectile(f, x, t, G['nose'], G['body'], pw)
        tail = t + G['nose'] + G['body'] + 2
        put(f, x + 0.5, tail, pal['smoke'] + (pal['sa'][0],))
        put(f, x + 0.5, tail + 2, pal['smoke'] + (pal['sa'][1],))
    frames.append(f)

    # --- col 4: Hit0 -- lead round detonates first ---
    f = blank()
    if stag:
        blast(f, lead, CY - lag * 0.45, G['h0_r'] * 0.95, pal, 'hit',
              G['h0_ray'], False)
        blast(f, trail, CY + lag * 0.45, G['h0_r'] * 0.55, pal, 'hit', None, False)
    elif twin:
        blast(f, CX, CY, G['h0_r'] * 1.12, pal, 'hit', G['h0_ray'] + 1, False)
        for x in (CX - 2.5, CX + 2.5):
            disc(f, x, CY, G['h0_r'] * 0.30, pal['C'] + (255,))
    else:
        blast(f, CX, CY, G['h0_r'], pal, 'hit', G['h0_ray'], False)
    frames.append(f)

    # --- col 5: Hit1 -- both bursts, still offset ---
    f = blank()
    if stag:
        smoke_e(f, CX, CY, G['h1_smoke'] * 1.02, G['h1_smoke'] * 1.16, pal, True)
        disc(f, lead, CY - lag * 0.40, G['h1_core'] * 0.86, pal['D'] + (255,))
        disc(f, trail, CY + lag * 0.40, G['h1_core'] * 0.72, pal['D'] + (255,))
        rays(f, CX, CY, G['h1_ray'], pal['D'] + (255,))
        dd = G['h1_core'] * 0.46
        for ox in (-dd, dd):
            for oy in (-dd, dd):
                mini_ring(f, CX + ox, CY + oy, G['h1_core'] * 0.28, mray)
    else:
        smoke(f, CX, CY, G['h1_smoke'] * (1.06 if twin else 1.0), pal, True)
        disc(f, CX, CY, G['h1_core'], pal['D'] + (255,))
        dd = G['h1_core'] * 0.46
        for ox in (-dd, dd):
            for oy in (-dd, dd):
                mini_ring(f, CX + ox, CY + oy, G['h1_core'] * 0.30, mray)
        rays(f, CX, CY, G['h1_ray'], pal['D'] + (255,))
    frames.append(f)

    # --- col 6: Hit2 -- merged smoke, elongated along travel ---
    f = blank()
    if stag:
        smoke_e(f, CX, CY, G['h2_smoke'] * 0.94, G['h2_smoke'] * 1.04, pal, False)
        disc(f, CX, CY - lag * 0.25, G['h2_core'] * 0.92, pal['D'] + (255,))
        disc(f, CX, CY + lag * 0.45, G['h2_core'] * 0.55, pal['D'] + (255,))
        diamond(f, CX, CY - lag * 0.25, G['h2_core'] * 0.55, mray)
    else:
        smoke(f, CX, CY, G['h2_smoke'] * (1.02 if twin else 1.0), pal, False)
        disc(f, CX, CY, G['h2_core'], pal['D'] + (255,))
        diamond(f, CX, CY, G['h2_core'] * 0.62, mray)
    frames.append(f)

    return frames


# ------------------------------------------------------------------
# Row order preserves the original variants 0-2 (standard, same colours)
# ------------------------------------------------------------------
ROWS = []
CLASS_ORDER = [
    ('std',   False, False),   # rows 0-2    standard, single
    ('std',   True,  False),   # rows 3-5    standard, twin
    ('super', False, False),   # rows 6-8    super, single
    ('super', True,  False),   # rows 9-11   super, twin
    ('std',   True,  True),    # rows 12-14  standard, twin STAGGERED
    ('super', True,  True),    # rows 15-17  super, twin STAGGERED
]
for cls, twin, stag in CLASS_ORDER:
    for fam in ('orange', 'red', 'blue'):
        ROWS.append((fam, cls, twin, stag))

COLS = 7
sheet = Image.new('RGBA', (S * COLS, S * len(ROWS)), (0, 0, 0, 0))
for i, (fam, cls, twin, stag) in enumerate(ROWS):
    for c, fr in enumerate(build_frames(fam, cls, twin, stag)):
        sheet.paste(fr, (c * S, i * S), fr)

sheet.save(f'{OUT}/shells.png')
sc = 5
print('shells.png', sheet.size, f'{COLS} cols x {len(ROWS)} rows')
for i, r in enumerate(ROWS):
    kind = ('twin staggered' if r[3] else 'twin') if r[2] else 'single'
    print(f'  row {i:2d}: {r[0]:6s} {r[1]:5s} {kind}')
