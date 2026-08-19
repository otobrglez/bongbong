from PIL import Image
import os, random, sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from punypalette import BLACK, STONE_DK, STONE_MD, STONE_LT, snap

S = 32
# Near-black tone sampled from the Puny World tileset's own shadow pixels
# (see tools/punypalette.py) -- that pack has no true black either.
OUTLINE = BLACK + (255,)
OUT = os.environ.get('SPRITE_OUT', 'assets/sprites')
os.makedirs(OUT, exist_ok=True)


def blank():
    return Image.new('RGBA', (S, S), (0, 0, 0, 0))


def mul(c, f):
    # Scale, then snap back onto the 64-colour set -- without this, darkening/
    # lightening a palette colour by an arbitrary factor drifts it off-palette.
    return snap((max(0, min(255, int(c[0] * f))),
                 max(0, min(255, int(c[1] * f))),
                 max(0, min(255, int(c[2] * f))), 255))


class Ramp:
    def __init__(self, base):
        b = tuple(base[:3])
        self.dk = mul(b, 0.56)
        self.md = mul(b, 0.78)
        self.bs = b + (255,)
        self.lt = mul(b, 1.20)
        # Gunmetal barrel ramp: the stone-wall grey steps sampled from Puny
        # World's own buildings, not a generic metal grey.
        self.gm = STONE_MD + (255,)
        self.gl = STONE_LT + (255,)
        self.gd = STONE_DK + (255,)


def px(img, x, y, c):
    if 0 <= x < S and 0 <= y < S and c is not None:
        img.putpixel((int(x), int(y)), c)


def rect(img, x0, y0, x1, y1, c):
    for y in range(int(y0), int(y1) + 1):
        for x in range(int(x0), int(x1) + 1):
            px(img, x, y, c)


def outline(img):
    src = img.copy()
    for y in range(S):
        for x in range(S):
            if src.getpixel((x, y))[3] != 0:
                continue
            for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                nx, ny = x + dx, y + dy
                if 0 <= nx < S and 0 <= ny < S and src.getpixel((nx, ny))[3] != 0:
                    px(img, x, y, OUTLINE)
                    break


# ----------------------------------------------------------------------
# BUILD DIMENSIONS  (tread_w, body_half_w, half_h) -> symmetric about 15.5
# Regular chassis are 10% smaller than the previous revision;
# super chassis are 20% larger than that original baseline.
# ----------------------------------------------------------------------
BUILD_PARAMS = {
    'narrow':      (3, 3,  9),   # 12 x 18
    'std':         (3, 4, 10),   # 14 x 20
    'wide':        (3, 5, 11),   # 16 x 22
    'long':        (3, 4, 12),   # 14 x 24
    'compact':     (3, 4,  8),   # 14 x 16
    'super_heavy': (4, 7, 12),   # 22 x 24
    'super_long':  (4, 6, 13),   # 20 x 26
}


def geom(build):
    tw, bhw, hh = BUILD_PARAMS[build]
    th = tw + bhw
    bx0, bx1 = 16 - bhw, 15 + bhw
    tl0, tl1 = 16 - th, 16 - th + tw - 1
    tr1, tr0 = 15 + th, 15 + th - tw + 1
    y0, y1 = 16 - hh, 15 + hh
    return tl0, tl1, tr0, tr1, bx0, bx1, y0, y1


# ----------------------------------------------------------------------
# HULL
# ----------------------------------------------------------------------
def draw_hull(spec, frame=0):
    img = blank()
    R = Ramp(spec['body'])
    acc = spec['accent']
    tl0, tl1, tr0, tr1, bx0, bx1, y0, y1 = geom(spec['build'])
    tstyle = spec['tread']

    ty0, ty1 = y0 + 1, y1 - 1
    for (a, b) in ((tl0, tl1), (tr0, tr1)):
        rect(img, a, ty0, b, ty1, R.md)
        for yy in range(ty0, ty1 + 1):
            k = (yy + frame) % 4
            if tstyle == 'fine' and k == 0:
                rect(img, a, yy, b, yy, R.dk)
            elif tstyle == 'seg' and k in (0, 1):
                rect(img, a, yy, b, yy, R.dk)
            elif tstyle == 'block' and k in (0, 1, 2):
                rect(img, a, yy, b, yy, R.dk)
        rect(img, a, ty0, a, ty1, R.bs)

    rect(img, bx0, y0, bx1, y1, R.bs)

    nose = spec['nose']
    depth = {'wedge': 4, 'chamfer': 3, 'blunt': 1}[nose]
    for i in range(depth):
        for k in range(depth - i):
            px(img, bx0 + k, y0 + i, (0, 0, 0, 0))
            px(img, bx1 - k, y0 + i, (0, 0, 0, 0))
    for i in range(2):
        for k in range(2 - i):
            px(img, bx0 + k, y1 - i, (0, 0, 0, 0))
            px(img, bx1 - k, y1 - i, (0, 0, 0, 0))

    rect(img, bx0, y0 + depth + 1, bx0 + 1, y1 - 2, R.lt)
    rect(img, bx1 - 1, y0 + depth + 1, bx1, y1 - 2, R.md)

    rect(img, bx0 + 2, y0 + depth + 1, bx1 - 2, y0 + depth + 1, R.dk)
    rect(img, bx0 + 3, y0 + depth - 1, bx1 - 3, y0 + depth, R.lt)

    if spec['deck'] == 'ribbed':
        for yy in range(y0 + depth + 3, y1 - 5, 3):
            rect(img, bx0 + 2, yy, bx1 - 2, yy, R.md)
    elif spec['deck'] == 'plates':
        rect(img, bx0 + 2, y0 + depth + 3, bx0 + 3, y1 - 6, R.md)
        rect(img, bx1 - 3, y0 + depth + 3, bx1 - 2, y1 - 6, R.md)

    if spec.get('sponson'):
        rect(img, bx0 + 1, 14, bx0 + 2, 17, R.md)
        rect(img, bx1 - 2, 14, bx1 - 1, 17, R.md)

    rect(img, bx0 + 1, y1 - 5, bx1 - 1, y1 - 5, R.dk)
    rect(img, bx0 + 2, y1 - 3, bx1 - 2, y1 - 2, R.md)
    n = spec['thrusters']
    if n == 2:
        ports = [bx0 + 2, bx1 - 2]
    elif n == 3:
        ports = [bx0 + 2, (bx0 + bx1) // 2, bx1 - 2]
    else:
        ports = [bx0 + 2, bx0 + 4, bx1 - 4, bx1 - 2]
    for pxx in sorted(set(p for p in ports if bx0 + 1 <= p <= bx1 - 1)):
        px(img, pxx, y1 - 2, acc)
        px(img, pxx, y1 - 3, mul(acc, 0.55))

    # turret mount ring, scaled to this tank's turret
    tr = spec['tr']
    inner, outer = tr * 0.62, tr * 0.85
    cx = cy = 15.5
    for y in range(S):
        for x in range(S):
            if img.getpixel((x, y))[3] == 0:
                continue
            d = ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5
            if d <= inner:
                img.putpixel((x, y), mul(spec['body'], 0.86))
            elif d <= outer:
                img.putpixel((x, y), mul(spec['body'], 0.70))
    br = int(round(tr * 0.95))
    for bxp, byp in ((15, 16 - br), (16, 16 - br), (16 - br, 15), (16 - br, 16),
                     (15 + br, 15), (15 + br, 16), (15, 15 + br), (16, 15 + br)):
        if 0 <= bxp < S and 0 <= byp < S and img.getpixel((bxp, byp))[3] != 0:
            px(img, bxp, byp, R.md)

    outline(img)
    return img


# ----------------------------------------------------------------------
# TURRET
# ----------------------------------------------------------------------
def turret_cols(spec):
    bw = spec['bw']
    half = bw // 2
    if spec['guns'] == 1:
        return [(16 - half, 15 + half)]
    g = spec['gap']
    return [(16 - g - half, 15 - g + half), (16 + g - half, 15 + g + half)]


def draw_turret(spec):
    img = blank()
    R = Ramp(mul(spec['body'], 1.16))
    acc = spec['accent']
    cx = cy = 15.5
    r = spec['tr']
    shape = spec['tshape']

    for y in range(S):
        for x in range(S):
            dx, dy = x - cx, y - cy
            if shape == 'round':
                inside = dx * dx + dy * dy <= r * r
            elif shape == 'hex':
                inside = abs(dy) <= r and abs(dx) * 0.95 + abs(dy) * 0.5 <= r
            elif shape == 'box':
                inside = abs(dx) <= r * 0.86 and abs(dy) <= r
            else:
                inside = abs(dx) + abs(dy) * 0.72 <= r + 0.6 and abs(dy) <= r
            if inside:
                px(img, x, y, R.bs)

    for y in range(S):
        for x in range(S):
            if img.getpixel((x, y))[3] == 0:
                continue
            dx, dy = x - cx, y - cy
            if dx + dy > r * 0.62:
                px(img, x, y, R.md)
            elif dx + dy < -r * 0.78:
                px(img, x, y, R.lt)

    cols = turret_cols(spec)
    for (a, b) in cols:
        top = max(1, int(cy - r) - spec['blen'])
        rect(img, a, top, b, 16, R.gm)
        rect(img, a, top, a, 16, R.gl)
        rect(img, a - 1, top, b + 1, top + 2, R.gm)
        rect(img, a - 1, top, a - 1, top + 2, R.gl)
        rect(img, a, top, b, top, R.gd)
        rect(img, a - 1, top + 4, b + 1, top + 4, R.gd)

    m0 = min(c[0] for c in cols) - 1
    m1 = max(c[1] for c in cols) + 1
    rect(img, m0, int(cy - r + 0.5), m1, int(cy - r + 2.5), R.bs)
    rect(img, m0, int(cy - r + 0.5), m0, int(cy - r + 2.5), R.lt)
    rect(img, m1, int(cy - r + 0.5), m1, int(cy - r + 2.5), R.md)

    ox, oy = spec['optic']
    px(img, ox, oy, acc)
    px(img, ox + 1, oy, mul(acc, 0.65))
    px(img, ox, oy + 1, mul(acc, 0.65))

    hy = int(cy + r * 0.45)
    rect(img, 14, hy, 17, hy + 2, R.md)
    rect(img, 15, hy + 1, 16, hy + 1, R.dk)

    outline(img)
    return img


# ----------------------------------------------------------------------
# TRACK MARKS
# ----------------------------------------------------------------------
MARK_RGB = BLACK
MARK_WEIGHT = {'narrow': 74, 'compact': 86, 'std': 98, 'long': 108,
               'wide': 120, 'super_heavy': 150, 'super_long': 142}


def draw_tracks(spec):
    img = blank()
    tl0, tl1, tr0, tr1, bx0, bx1, y0, y1 = geom(spec['build'])
    base = MARK_WEIGHT[spec['build']]
    tstyle = spec['tread']

    def put(x, y, a):
        a = max(0, min(255, int(a)))
        cur = img.getpixel((x, y))
        if a > cur[3]:
            img.putpixel((x, y), MARK_RGB + (a,))

    for (a, b) in ((tl0, tl1), (tr0, tr1)):
        for y in range(S):
            for x in range(a, b + 1):
                put(x, y, base * 0.32)
            put(a, y, base * 0.68)
            put(b, y, base * 0.68)
            k = y % 4
            if tstyle == 'fine' and k == 0:
                for x in range(a, b + 1):
                    put(x, y, base)
            elif tstyle == 'seg' and k in (0, 1):
                for x in range(a, b + 1):
                    put(x, y, base if k == 0 else base * 0.8)
            elif tstyle == 'block' and k in (0, 1, 2):
                for x in range(a, b + 1):
                    put(x, y, base if k < 2 else base * 0.75)
    return img


# ----------------------------------------------------------------------
# DAMAGE
# ----------------------------------------------------------------------
BURN = BLACK
CHAR = BLACK + (255,)
EMBER = (0x81, 0x2F, 0x27, 255)      # Puny roof-tile dark red ember
EMBER_HOT = (0xFF, 0x42, 0x1A, 255)  # Puny roof-tile bright red hot ember
DEBRIS = STONE_DK + (255,)   # torn/shredded metal (lighter than a hole)


def scorch(c, f=0.45):
    return snap((int(c[0] * f + BURN[0] * (1 - f)),
                 int(c[1] * f + BURN[1] * (1 - f)),
                 int(c[2] * f + BURN[2] * (1 - f)), 255))


def burn_all(img, f=0.45):
    for y in range(S):
        for x in range(S):
            c = img.getpixel((x, y))
            if c[3] == 0 or c == OUTLINE:
                continue
            img.putpixel((x, y), scorch(c, f))


def opaque_pixels(img):
    return [(x, y) for y in range(S) for x in range(S)
            if img.getpixel((x, y))[3] != 0 and img.getpixel((x, y)) != OUTLINE]


def punch_holes(img, rng, n, radius=1):
    pts = opaque_pixels(img)
    if not pts:
        return
    for _ in range(n):
        hx, hy = rng.choice(pts)
        for dy in range(-radius, radius + 1):
            for dx in range(-radius, radius + 1):
                if dx * dx + dy * dy <= radius * radius:
                    if 0 <= hx + dx < S and 0 <= hy + dy < S:
                        if img.getpixel((hx + dx, hy + dy))[3] != 0:
                            img.putpixel((hx + dx, hy + dy), CHAR)


def chew_edges(img, rng, n):
    for _ in range(n):
        edge = []
        for y in range(S):
            for x in range(S):
                if img.getpixel((x, y))[3] == 0:
                    continue
                for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                    nx, ny = x + dx, y + dy
                    if not (0 <= nx < S and 0 <= ny < S) or img.getpixel((nx, ny))[3] == 0:
                        edge.append((x, y))
                        break
        if not edge:
            return
        ex, ey = rng.choice(edge)
        for dy in range(-1, 2):
            for dx in range(-1, 2):
                if abs(dx) + abs(dy) <= 1 + rng.randint(0, 1):
                    if 0 <= ex + dx < S and 0 <= ey + dy < S:
                        img.putpixel((ex + dx, ey + dy), (0, 0, 0, 0))


def despeckle(img):
    src = img.copy()
    for y in range(S):
        for x in range(S):
            if src.getpixel((x, y))[3] == 0:
                continue
            n = 0
            for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                nx, ny = x + dx, y + dy
                if 0 <= nx < S and 0 <= ny < S and src.getpixel((nx, ny))[3] != 0:
                    n += 1
            if n == 0:
                img.putpixel((x, y), (0, 0, 0, 0))


def strip_outline(img):
    for y in range(S):
        for x in range(S):
            if img.getpixel((x, y)) == OUTLINE:
                img.putpixel((x, y), (0, 0, 0, 0))


def blow_ring(img, rng, tr, prob=0.5):
    cx = cy = 15.5
    for y in range(S):
        for x in range(S):
            if img.getpixel((x, y))[3] == 0:
                continue
            d = ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5
            if d <= tr * 0.62:
                img.putpixel((x, y), CHAR)
            elif d <= tr * 0.85 and rng.random() < prob:
                img.putpixel((x, y), DEBRIS)


def draw_broken_turret(spec, seed):
    img = draw_turret(spec)
    rng = random.Random(seed)
    strip_outline(img)
    cols = turret_cols(spec)
    r = spec['tr']
    for (a, b) in cols:
        top = max(1, int(15.5 - r) - spec['blen'])
        cut = top + rng.randint(1, max(2, spec['blen'] // 2))
        rect(img, a - 1, 0, b + 1, cut, (0, 0, 0, 0))
        rect(img, a, cut + 1, b, cut + 1, CHAR)
        px(img, a, cut + 2, EMBER)
    burn_all(img, 0.62)
    punch_holes(img, rng, 2, radius=1)
    ox, oy = spec['optic']
    for dx, dy in ((0, 0), (1, 0), (0, 1)):
        px(img, ox + dx, oy + dy, CHAR)
    hy = int(15.5 + r * 0.45)
    rect(img, 14, hy, 17, hy + 2, CHAR)
    px(img, 15, hy + 1, EMBER)
    chew_edges(img, rng, 1)
    despeckle(img)
    outline(img)
    return img


def draw_damaged_hull(spec, seed, mode):
    """mode: light | disabled | wreckA | wreckB | wreckC | wreckD"""
    img = draw_hull(spec, frame=0)
    rng = random.Random(seed)
    strip_outline(img)
    tl0, tl1, tr0, tr1, bx0, bx1, y0, y1 = geom(spec['build'])
    tr = spec['tr']

    if mode == 'light':
        # cosmetic: scuffed paint, a couple of small hits, still fully operational
        burn_all(img, 0.86)
        punch_holes(img, rng, 2, radius=0)
        for _ in range(3):
            sx = rng.randint(bx0 + 1, bx1 - 1)
            sy = rng.randint(y0 + 3, y1 - 3)
            c = img.getpixel((sx, sy))
            if c[3] != 0:
                img.putpixel((sx, sy), scorch(c, 0.45))
        # one thruster flickers out
        px(img, bx0 + 2, y1 - 2, scorch((120, 110, 100), 0.5))

    elif mode == 'disabled':
        burn_all(img, 0.72)
        punch_holes(img, rng, 3, radius=1)
        for yy in range(y0 + 3, min(y1, y0 + 10)):
            if rng.random() < 0.6:
                rect(img, tl0, yy, tl1, yy, DEBRIS)
        rect(img, bx0 + 1, y1 - 3, bx1 - 1, y1 - 2, scorch((70, 66, 66), 0.5))
        px(img, bx0 + 2, y1 - 2, EMBER)
        chew_edges(img, rng, 2)

    elif mode == 'wreckA':
        # turret ring blown out, both runs shredded
        burn_all(img, 0.64)
        punch_holes(img, rng, 4, radius=1)
        punch_holes(img, rng, 1, radius=2)
        for (a, b) in ((tl0, tl1), (tr0, tr1)):
            for yy in range(y0 + 1, y1):
                if rng.random() < 0.30:
                    rect(img, a, yy, b, yy, DEBRIS)
                if rng.random() < 0.10:
                    rect(img, a, yy, b, yy, (0, 0, 0, 0))
        blow_ring(img, rng, tr, 0.5)
        px(img, 15, 15, EMBER)
        px(img, 17, 16, EMBER)
        rect(img, bx0 + 1, y1 - 3, bx1 - 1, y1 - 2, DEBRIS)
        chew_edges(img, rng, 3)

    elif mode == 'wreckB':
        # hull breach: one entire track run torn away, long gash down that flank
        burn_all(img, 0.62)
        rect(img, tl0, y0, tl1, y1, (0, 0, 0, 0))          # left run gone
        gx = bx0 + 1
        for yy in range(y0 + 3, y1 - 3):
            if rng.random() < 0.75:
                rect(img, gx, yy, gx + 1, yy, CHAR)
        punch_holes(img, rng, 3, radius=1)
        blow_ring(img, rng, tr, 0.3)
        px(img, gx, (y0 + y1) // 2, EMBER)
        rect(img, bx0 + 1, y1 - 3, bx1 - 1, y1 - 2, DEBRIS)
        chew_edges(img, rng, 3)

    elif mode == 'wreckC':
        # burnt-out husk: uniform heavy char, few holes, eroded silhouette
        burn_all(img, 0.50)
        punch_holes(img, rng, 2, radius=1)
        for (a, b) in ((tl0, tl1), (tr0, tr1)):
            for yy in range(y0 + 1, y1):
                if rng.random() < 0.5:
                    rect(img, a, yy, b, yy, DEBRIS)
        blow_ring(img, rng, tr, 0.7)
        rect(img, bx0 + 1, y1 - 3, bx1 - 1, y1 - 2, DEBRIS)
        chew_edges(img, rng, 4)

    else:  # wreckD -- ammo cook-off, catastrophic
        burn_all(img, 0.58)
        punch_holes(img, rng, 2, radius=2)
        punch_holes(img, rng, 3, radius=1)
        for (a, b) in ((tl0, tl1), (tr0, tr1)):
            for yy in range(y0 + 1, y1):
                if rng.random() < 0.35:
                    rect(img, a, yy, b, yy, DEBRIS)
                if rng.random() < 0.16:
                    rect(img, a, yy, b, yy, (0, 0, 0, 0))
        blow_ring(img, rng, tr * 1.35, 0.85)
        for dx, dy in ((0, 0), (1, -1), (-1, 1), (2, 1), (-1, -2)):
            px(img, 15 + dx, 15 + dy, EMBER_HOT if (dx + dy) % 2 == 0 else EMBER)
        rect(img, bx0 + 1, y1 - 3, bx1 - 1, y1 - 2, DEBRIS)
        chew_edges(img, rng, 4)

    despeckle(img)
    outline(img)
    return img


# ----------------------------------------------------------------------
# ROSTER — 10 regular (10% smaller) + 2 super (20% beefier)
# ----------------------------------------------------------------------
# Body/accent are curated picks from tools/punypalette.py -- colours sampled
# directly from the Puny World ground-layer tileset (roof tiles, wood/stone
# buildings, water, grass), not Resurrect 64. This is the third palette pass
# for this roster (see tools/spritegen/_backup/{muted,pre-punypalette}-*/
# for the earlier ones): the previous R64-vibrant pass looked great in
# isolation but candy-neon next to Puny World's much softer, painterly
# terrain once the ground layer actually shipped (docs/GROUND_SPEC.md) --
# see docs/PALETTE.md for the full before/after reasoning. Puny World's own
# art has no purple/violet family anywhere in it (checked); `wraith` was
# reassigned from a purple accent to a hue family the source art actually
# has rather than force an invented off-palette colour.
#
# EVERY body colour below is deliberately picked from the palette's bright/
# mid steps, never its darkest ("*_DARKEST", "*_DEEPER") ones. A first cut
# of this same pass gave the back half of the roster (wraith/ravager,
# especially the two super-heavy titan/leviathan) the darkest available step
# of their family, reasoning "heavier/stealthier = darker" -- this is
# *exactly* the muted-pass mistake this file's own history already
# documents (see above), just rediscovered a second time on a different
# palette: a dark hull body reads as dirty/muddy regardless of which
# palette it's snapped to, since the body is most of a tank's on-screen
# area. Reserve the darkest palette steps for outline/shadow/damage tones
# (BLACK, mul()'s dk step, EMBER, etc.), never for a body colour.
TANKS = [
    dict(name='scout',   body=(0x85, 0xA6, 0x43), accent=(0xEE, 0xA3, 0x43), build='narrow',
         nose='wedge',   tread='fine',  deck='ribbed', thrusters=2, sponson=False,
         tshape='round', tr=4.1, guns=1, bw=2, blen=8,  gap=0, optic=(13, 13)),
    dict(name='assault', body=(0xDE, 0x99, 0x43), accent=(0x00, 0xA6, 0x7F), build='std',
         nose='chamfer', tread='seg',   deck='plates', thrusters=3, sponson=True,
         tshape='box',   tr=4.7, guns=2, bw=2, blen=6,  gap=3, optic=(12, 18)),
    dict(name='breaker', body=(0x9C, 0x35, 0x27), accent=(0xDC, 0x9C, 0x4A), build='wide',
         nose='blunt',   tread='block', deck='ribbed', thrusters=4, sponson=True,
         tshape='hex',   tr=5.2, guns=1, bw=4, blen=5,  gap=0, optic=(12, 13)),
    dict(name='longbow', body=(0x5F, 0x91, 0x4B), accent=(0xFF, 0x42, 0x1A), build='long',
         nose='chamfer', tread='seg',   deck='plates', thrusters=2, sponson=False,
         tshape='round', tr=4.3, guns=1, bw=2, blen=11, gap=0, optic=(13, 13)),
    dict(name='flak',    body=(0x04, 0xA0, 0xB4), accent=(0xCA, 0xC5, 0x94), build='compact',
         nose='blunt',   tread='block', deck='ribbed', thrusters=2, sponson=False,
         tshape='hex',   tr=4.5, guns=2, bw=2, blen=4,  gap=3, optic=(12, 18)),
    dict(name='wraith',  body=(0x00, 0xBB, 0x8F), accent=(0xDA, 0xE5, 0xCE), build='narrow',
         nose='wedge',   tread='fine',  deck='plates', thrusters=2, sponson=False,
         tshape='wedge', tr=4.3, guns=1, bw=2, blen=7,  gap=0, optic=(13, 13)),
    dict(name='warden',  body=(0x7C, 0x98, 0x3C), accent=(0xDC, 0x9C, 0x4A), build='std',
         nose='chamfer', tread='seg',   deck='ribbed', thrusters=3, sponson=True,
         tshape='hex',   tr=4.7, guns=1, bw=4, blen=7,  gap=0, optic=(12, 13)),
    dict(name='ravager', body=(0xCA, 0x8A, 0x3B), accent=(0xE4, 0x42, 0x19), build='wide',
         nose='chamfer', tread='block', deck='plates', thrusters=4, sponson=False,
         tshape='round', tr=5.2, guns=2, bw=2, blen=7,  gap=3, optic=(12, 18)),
    dict(name='glacier', body=(0x1E, 0xB3, 0xAE), accent=(0xDA, 0xE5, 0xCE), build='compact',
         nose='wedge',   tread='fine',  deck='plates', thrusters=2, sponson=False,
         tshape='box',   tr=4.5, guns=1, bw=2, blen=8,  gap=0, optic=(13, 13)),
    dict(name='obelisk', body=(0x81, 0x2F, 0x27), accent=(0xEE, 0xA3, 0x43), build='long',
         nose='blunt',   tread='block', deck='ribbed', thrusters=4, sponson=True,
         tshape='wedge', tr=5.2, guns=2, bw=2, blen=9,  gap=3, optic=(12, 18)),
    # ---- super-heavy chassis ----
    dict(name='titan',     body=(0xFF, 0x42, 0x1A), accent=(0xFF, 0xFF, 0xFF), build='super_heavy',
         nose='blunt',   tread='block', deck='ribbed', thrusters=4, sponson=True,
         tshape='hex',   tr=7.0, guns=2, bw=4, blen=8,  gap=5, optic=(11, 19)),
    dict(name='leviathan', body=(0x00, 0xD0, 0x97), accent=(0x27, 0xD8, 0xC5), build='super_long',
         nose='chamfer', tread='block', deck='plates', thrusters=4, sponson=True,
         tshape='round', tr=6.6, guns=1, bw=4, blen=12, gap=0, optic=(11, 13)),
]

# ---- sheet: 13 columns ----
COL_ORDER = ['hull0', 'turret', 'hull1', 'hull2', 'hull3', 'brk_turret',
             'light', 'disabled', 'wreckA', 'wreckB', 'wreckC', 'wreckD', 'marks']
COLS = len(COL_ORDER)

sheet = Image.new('RGBA', (S * COLS, S * len(TANKS)), (0, 0, 0, 0))

for i, spec in enumerate(TANKS):
    cells = {
        'hull0': draw_hull(spec, 0),
        'hull1': draw_hull(spec, 1),
        'hull2': draw_hull(spec, 2),
        'hull3': draw_hull(spec, 3),
        'turret': draw_turret(spec),
        'brk_turret': draw_broken_turret(spec, 1000 + i),
        'light': draw_damaged_hull(spec, 2000 + i, 'light'),
        'disabled': draw_damaged_hull(spec, 3000 + i, 'disabled'),
        'wreckA': draw_damaged_hull(spec, 4000 + i, 'wreckA'),
        'wreckB': draw_damaged_hull(spec, 5000 + i, 'wreckB'),
        'wreckC': draw_damaged_hull(spec, 6000 + i, 'wreckC'),
        'wreckD': draw_damaged_hull(spec, 7000 + i, 'wreckD'),
        'marks': draw_tracks(spec),
    }
    for col, key in enumerate(COL_ORDER):
        sheet.paste(cells[key], (col * S, i * S))   # direct copy: preserves exact RGBA
        # (no mask arg -- passing cells[key] as its own mask would alpha-
        # composite semi-transparent pixels against the sheet's transparent
        # background, drifting the trackmarks decal's colour off-palette)

    n = spec['name']
    fnames = {'hull0': 'hull', 'turret': 'turret', 'brk_turret': 'turret_broken',
              'light': 'hull_damaged_light', 'disabled': 'hull_damaged_disabled',
              'wreckA': 'hull_wreck_a', 'wreckB': 'hull_wreck_b',
              'wreckC': 'hull_wreck_c', 'wreckD': 'hull_wreck_d',
              'marks': 'trackmarks'}
    for key, suffix in fnames.items():
        cells[key].save(f'{OUT}/scifi_{n}_{suffix}.png')

sheet.save(f'{OUT}/scifi_tanks_sheet.png')

sc = 5
print('sheet', sheet.size, f'{COLS} cols x {len(TANKS)} rows')
