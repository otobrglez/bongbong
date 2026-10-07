"""The tank design kit: a part-based pixel-art renderer for the tank redesign
study (four design lines x twelve chassis).

A design is two layers - the hull and the turret - each a list of `Part`s:
a pixel mask (built from the shape helpers below, in pivot-centred design
pixels), a `Mat` (a colour ramp off the Puny Palette), a shading mode and a
height `z`. Rendering a layer:

  1. rasterise every part (later and higher parts cover earlier ones),
  2. shade each pixel by its part's mode (bevels, facets, domes, cylinders,
     treads, grilles...) as a step up or down its material's ramp,
  3. drop contact shadows below-right of every raised part (the light comes
     from the top left, the game's `shadow_dir`),
  4. apply damage (`damage.py`: scorch, scratches, holes, blown-off armour,
     broken lamps, broken track, wrecks),
  5. resolve colours, outline the silhouette in the palette's near-black,
     and write the emissive layer (every lamp, strip, sensor and ember at
     full strength, what a night frame adds back after the light multiply).

Coordinates are design pixels relative to the pivot: x in [-20, 19], y in
[-20, 19], the pivot between pixels -1 and 0 on both axes, y down, the
barrel toward -y. A 40 x 40 cell is drawn at 2x in the game, like today's
32 x 32 cells. `mirror` maps x to -1 - x, so a symmetric part is `sym(px)`.
"""

import math
import os
import random
import sys

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.normpath(os.path.join(HERE, '..', '..')))
import punypalette as pp  # noqa: E402

# The cell, in design pixels. 40 everywhere but the density study, which
# draws one tank at twice the pixel density (TANKDESIGN_CELL=80).
S = int(os.environ.get('TANKDESIGN_CELL', '40'))
HALF = S // 2

# ---------------------------------------------------------------------------
# Palette
# ---------------------------------------------------------------------------
BLACK = pp.BLACK
WHITE = pp.WHITE
STONE_PALE, STONE_HI, STONE_LT, STONE_MID = pp.STONE_PALE, pp.STONE_HI, pp.STONE_LT, pp.STONE_MID
STONE_MD, STONE_MDK, STONE_DK, STONE_SHADE, STONE_DARKEST = pp.STONE_MD, pp.STONE_MDK, pp.STONE_DK, pp.STONE_SHADE, pp.STONE_DARKEST
SAND_PALE, SAND_LT, SAND_MD, SAND_DK = pp.SAND_PALE, pp.SAND_LT, pp.SAND_MD, pp.SAND_DK
WOOD_PALE, WOOD_LT, WOOD_MD, WOOD_AMBER = pp.WOOD_PALE, pp.WOOD_LT, pp.WOOD_MD, pp.WOOD_AMBER
WOOD_DK, WOOD_DEEPER, WOOD_DARKEST = pp.WOOD_DK, pp.WOOD_DEEPER, pp.WOOD_DARKEST
RED_BRIGHT, RED_MD, RED_DEEP, RED_DK, RED_DARKEST = pp.RED_BRIGHT, pp.RED_MD, pp.RED_DEEP, pp.RED_DK, pp.RED_DARKEST
TEAL_BRIGHT, TEAL_LT, TEAL_MD, TEAL_DK, TEAL_DARKEST = pp.TEAL_BRIGHT, pp.TEAL_LT, pp.TEAL_MD, pp.TEAL_DK, pp.TEAL_DARKEST
BLUE_BRIGHT, BLUE_LT, BLUE_MD, BLUE_DK, BLUE_DARKEST = pp.BLUE_BRIGHT, pp.BLUE_LT, pp.BLUE_MD, pp.BLUE_DK, pp.BLUE_DARKEST
BLUE_PALE = pp.BLUE_PALE
GREEN_BRIGHT, GREEN_LT, GREEN_MD, GREEN_DK, GREEN_DARKEST = pp.GREEN_BRIGHT, pp.GREEN_LT, pp.GREEN_MD, pp.GREEN_DK, pp.GREEN_DARKEST
GREEN_SHADE = pp.GREEN_SHADE
GOLD_BRIGHT, GOLD_MD, GOLD_PALE = pp.GOLD_BRIGHT, pp.GOLD_MD, pp.GOLD_PALE
RUST_MD, RUST_DK, WOOD_ASH = pp.RUST_MD, pp.RUST_DK, pp.WOOD_ASH

# Two interpolated darks the water/teal hulls need for a shadow side: the
# pack's blue family bottoms out at #0E8B96, which is not dark. Each is the
# midpoint of two sampled neighbours (the PUNY_EXTRA rule).
BLUE_DEEP = (0x1D, 0x60, 0x71)   # mid(BLUE_DK, STONE_DARKEST)
TEAL_DEEP = (0x00, 0x5A, 0x3B)   # mid(TEAL_DK, TEAL_DARKEST)
TANK_EXTRA = [BLUE_DEEP, TEAL_DEEP]

PALETTE = list(pp.PUNY_PALETTE_ALL) + TANK_EXTRA

# The team ramps, extended with their Resurrect 64 neighbours at both ends
# (the same source as TEAM_P1/TEAM_P2, admitted on the player rows only).
TEAM_RAMPS = {
    'p1': [(0x32, 0x33, 0x53), (0x48, 0x4A, 0x77), (0x4D, 0x65, 0xB4), (0x4D, 0x9B, 0xE6), (0x8F, 0xD3, 0xFF), (0xC7, 0xEA, 0xFF), WHITE],
    'p2': [(0x45, 0x29, 0x3F), (0x83, 0x1C, 0x5D), (0xC3, 0x24, 0x54), (0xF0, 0x4F, 0x78), (0xED, 0x80, 0x99), (0xFC, 0xC0, 0xB6), WHITE],
    # Candidates for players 3 and 4 (the owner picks two of the three).
    # Green is an emerald, a bluer green than the grass it drives on, lit to
    # mint; white is a cool silver-white a clear step above the grey wraith;
    # orange is a vivid, saturated orange, hotter than the wood hulls.
    'green': [(0x12, 0x2B, 0x26), (0x16, 0x5A, 0x4C), (0x23, 0x90, 0x63), (0x1E, 0xBC, 0x73), (0x6B, 0xE6, 0xA2), (0xC2, 0xF7, 0xD6), WHITE],
    'white': [(0x34, 0x39, 0x47), (0x5B, 0x64, 0x75), (0x92, 0x9C, 0xAD), (0xD3, 0xDA, 0xE3), (0xEE, 0xF1, 0xF5), WHITE, WHITE],
    'orange': [(0x4A, 0x1F, 0x16), (0x9E, 0x45, 0x39), (0xD1, 0x51, 0x1A), (0xFB, 0x6B, 0x1D), (0xF7, 0x96, 0x17), (0xF9, 0xC2, 0x2B), (0xFF, 0xF2, 0xC0)],
}
TEAM_LIGHT = {'p1': (0x8F, 0xD3, 0xFF), 'p2': (0xED, 0x80, 0x99), 'green': (0x6B, 0xE6, 0xA2), 'white': (0xF4, 0xF8, 0xFF),
              'orange': (0xFF, 0xA8, 0x3A)}
TEAM_BASE = {'p1': (0x4D, 0x9B, 0xE6), 'p2': (0xF0, 0x4F, 0x78), 'green': (0x1E, 0xBC, 0x73), 'white': (0xD3, 0xDA, 0xE3),
             'orange': (0xFB, 0x6B, 0x1D)}


def lum(c):
    return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]


# ---------------------------------------------------------------------------
# Materials
# ---------------------------------------------------------------------------
class Mat:
    """A colour ramp, dark to light, with the index its flat top face sits
    at. Shading moves a pixel up (+) or down (-) the ramp from `base`."""

    def __init__(self, name, ramp, base, role=None):
        self.name = name
        self.ramp = [tuple(c[:3]) for c in ramp]
        self.base = base
        self.role = role

    def at(self, off):
        return self.ramp[max(0, min(len(self.ramp) - 1, self.base + off))]

    def index(self, off):
        return max(0, min(len(self.ramp) - 1, self.base + off))

    def with_base(self, base, name=None):
        return Mat(name or self.name, self.ramp, base, self.role)

    def __repr__(self):
        return 'Mat(%s)' % self.name


# Neutral metals - the pack's true greys, every step (PUNY_PALETTE_ALL).
STEEL = Mat('steel', [BLACK, STONE_DARKEST, STONE_SHADE, STONE_DK, STONE_MDK, STONE_MD, STONE_MID, STONE_LT, STONE_HI, STONE_PALE], 5)
GUNMETAL = Mat('gunmetal', [BLACK, STONE_DARKEST, STONE_SHADE, STONE_DK, STONE_MDK, STONE_MD, STONE_MID, STONE_LT, STONE_HI], 4)
DARK = Mat('dark', [BLACK, STONE_DARKEST, STONE_SHADE, STONE_DK, STONE_MDK, STONE_MD], 1)
TRACK = Mat('track', [BLACK, STONE_DARKEST, STONE_SHADE, STONE_DK, STONE_MDK, STONE_MD, STONE_MID, STONE_LT], 3)
PALE = Mat('pale', [STONE_DK, STONE_MDK, STONE_MID, STONE_LT, STONE_HI, STONE_PALE, WHITE], 4)
GLASS = Mat('glass', [BLACK, TEAL_DARKEST, TEAL_DEEP, BLUE_DEEP, BLUE_DK, BLUE_LT, BLUE_BRIGHT, BLUE_PALE, WHITE], 3)
SMOKED = Mat('smoked', [BLACK, STONE_DARKEST, BLUE_DEEP, BLUE_DK, BLUE_MD, BLUE_PALE, WHITE], 2)
BRASS = Mat('brass', [WOOD_DARKEST, WOOD_DEEPER, WOOD_DK, WOOD_AMBER, GOLD_MD, GOLD_BRIGHT, WOOD_PALE], 4)
COPPER = Mat('copper', [RED_DARKEST, RUST_DK, RUST_MD, WOOD_DK, WOOD_AMBER, WOOD_MD, GOLD_BRIGHT], 3)
RUST = Mat('rust', [BLACK, RUST_DK, RUST_MD, WOOD_DK, WOOD_AMBER], 2)
CHAR = Mat('char', [BLACK, STONE_DARKEST, RUST_DK, WOOD_ASH, STONE_SHADE, STONE_DK], 1)
HAZARD = Mat('hazard', [WOOD_DEEPER, WOOD_DK, WOOD_AMBER, GOLD_BRIGHT, WOOD_PALE, STONE_PALE], 3)
REDM = Mat('red', [RED_DARKEST, RED_DK, RED_DEEP, RED_MD, RED_BRIGHT, GOLD_BRIGHT], 3)
TEALM = Mat('teal', [TEAL_DARKEST, TEAL_DEEP, TEAL_DK, TEAL_MD, TEAL_LT, TEAL_BRIGHT, BLUE_BRIGHT], 3)
CABLE_COLOURS = [RED_MD, GOLD_BRIGHT, BLUE_MD, STONE_LT]


# ---------------------------------------------------------------------------
# Chassis - the twelve rows, their footprints and colour identities
# ---------------------------------------------------------------------------
# `fp` is the hull's outer box (outline included) in pivot coordinates,
# (x0, y0, x1, y1) inclusive - the measured hit box of today's sheet
# (docs/SPRITESHEET_SPEC.md section 9, TANK_HULL_BBOX_BY_ROW), which the
# redesign keeps. `muzzle` is today's forward offset, `lat` the twin
# barrels' lateral offset, both gameplay (tank_muzzle_forward_offset,
# tank_barrel_lateral_offset). `body`/`accent` are (ramp, base index).
def _body(ramp, base):
    return (ramp, base)


CHASSIS = {
    'scout': dict(row=0, cls='narrow', fp=(-7, -9, 6, 9), guns=1, gun='thin', turret='round', lat=0, muzzle=14, tier='light',
                  role='Fast recon',
                  body=_body([SAND_DK, WOOD_DK, SAND_MD, SAND_LT, SAND_PALE, WOOD_PALE, STONE_PALE], 3),
                  accent=_body([RED_DEEP, RED_MD, RED_BRIGHT, GOLD_BRIGHT], 1), glow=RED_BRIGHT),
    'assault': dict(row=1, cls='std', fp=(-8, -11, 7, 10), guns=2, gun='std', turret='box', lat=3, muzzle=13, tier='medium',
                    role='General purpose',
                    body=_body([WOOD_DARKEST, WOOD_DEEPER, WOOD_DK, WOOD_AMBER, WOOD_MD, WOOD_LT, GOLD_BRIGHT, WOOD_PALE], 5),
                    accent=_body([TEAL_DK, TEAL_MD, TEAL_BRIGHT, BLUE_BRIGHT], 1), glow=TEAL_BRIGHT),
    'breaker': dict(row=2, cls='wide', fp=(-9, -12, 8, 11), guns=1, gun='heavy', turret='hex', lat=0, muzzle=12, tier='heavy',
                    role='Heavy brawler',
                    body=_body([BLACK, RED_DARKEST, RED_DK, RED_DEEP, RED_MD, RED_BRIGHT, GOLD_BRIGHT], 3),
                    accent=_body([WOOD_AMBER, GOLD_MD, GOLD_BRIGHT, WOOD_PALE], 1), glow=GOLD_BRIGHT),
    'longbow': dict(row=3, cls='long', fp=(-8, -13, 7, 12), guns=1, gun='long', turret='round', lat=0, muzzle=16, tier='heavy',
                    role='Artillery / sniper',
                    body=_body([GREEN_DARKEST, GREEN_SHADE, GREEN_DK, GREEN_MD, GREEN_LT, GREEN_BRIGHT, WOOD_PALE], 2),
                    accent=_body([RED_DEEP, RED_MD, RED_BRIGHT, GOLD_BRIGHT], 2), glow=RED_BRIGHT),
    'flak': dict(row=4, cls='compact', fp=(-8, -9, 7, 8), guns=2, gun='short', turret='hex', lat=3, muzzle=10, tier='light',
                 role='Anti-air / close range',
                 body=_body([STONE_DARKEST, BLUE_DEEP, BLUE_DK, BLUE_MD, BLUE_LT, BLUE_BRIGHT, BLUE_PALE], 3),
                 accent=_body([SAND_MD, GOLD_PALE, STONE_PALE, WHITE], 1), glow=GOLD_PALE),
    'wraith': dict(row=5, cls='narrow', fp=(-7, -9, 6, 9), guns=1, gun='std', turret='wedge', lat=0, muzzle=13, tier='light',
                   role='Stealth',
                   body=_body([STONE_DARKEST, STONE_SHADE, STONE_DK, STONE_MDK, STONE_MD, STONE_MID, STONE_LT, STONE_HI], 4),
                   accent=_body([STONE_LT, STONE_PALE, WHITE, WHITE], 1), glow=BLUE_PALE),
    'warden': dict(row=6, cls='std', fp=(-8, -11, 7, 10), guns=1, gun='heavy', turret='hex', lat=0, muzzle=14, tier='medium',
                   role='Support / defence',
                   body=_body([WOOD_DARKEST, WOOD_DEEPER, WOOD_DK, WOOD_AMBER, WOOD_MD, WOOD_LT, GOLD_BRIGHT], 3),
                   accent=_body([TEAL_MD, TEAL_BRIGHT, BLUE_BRIGHT, BLUE_PALE], 1), glow=TEAL_BRIGHT),
    'ravager': dict(row=7, cls='wide', fp=(-9, -12, 8, 11), guns=2, gun='std', turret='round', lat=3, muzzle=14, tier='heavy',
                    role='Heavy assault',
                    body=_body([WOOD_DARKEST, WOOD_DEEPER, WOOD_DK, WOOD_AMBER, WOOD_MD, WOOD_LT, GOLD_BRIGHT, WOOD_PALE], 4),
                    accent=_body([RED_DEEP, RED_MD, RED_BRIGHT, GOLD_BRIGHT], 1), glow=RED_BRIGHT),
    'glacier': dict(row=8, cls='compact', fp=(-8, -8, 7, 8), guns=1, gun='std', turret='box', lat=0, muzzle=14, tier='light',
                    role='Balanced',
                    body=_body([BLUE_DEEP, BLUE_DK, BLUE_MD, BLUE_LT, BLUE_BRIGHT, BLUE_PALE, STONE_PALE], 3),
                    accent=_body([STONE_LT, STONE_PALE, WHITE, WHITE], 1), glow=BLUE_PALE),
    'obelisk': dict(row=9, cls='long', fp=(-8, -13, 7, 12), guns=2, gun='long', turret='wedge', lat=3, muzzle=16, tier='heavy',
                    role='Siege',
                    body=_body([BLACK, RED_DARKEST, RUST_DK, RED_DK, RED_DEEP, RED_MD, GOLD_BRIGHT], 3),
                    accent=_body([WOOD_AMBER, GOLD_MD, GOLD_BRIGHT, WOOD_PALE], 2), glow=GOLD_BRIGHT),
    'titan': dict(row=10, cls='super_heavy', fp=(-12, -13, 11, 12), guns=2, gun='super', turret='hex', lat=5, muzzle=16, tier='super',
                  role='Super-heavy assault',
                  body=_body([RED_DARKEST, RED_DK, RED_DEEP, RED_MD, RED_BRIGHT, GOLD_BRIGHT, WOOD_PALE], 4),
                  accent=_body([STONE_LT, STONE_PALE, WHITE, WHITE], 2), glow=WHITE),
    'leviathan': dict(row=11, cls='super_long', fp=(-11, -14, 10, 13), guns=1, gun='massive', turret='round', lat=0, muzzle=16, tier='super',
                      role='Super-heavy siege',
                      body=_body([TEAL_DARKEST, TEAL_DEEP, TEAL_DK, TEAL_MD, TEAL_LT, TEAL_BRIGHT, BLUE_BRIGHT, BLUE_PALE], 5),
                      accent=_body([BLUE_LT, BLUE_BRIGHT, BLUE_PALE, WHITE], 1), glow=BLUE_BRIGHT),
}
CHASSIS_ORDER = ['scout', 'assault', 'breaker', 'longbow', 'flak', 'wraith', 'warden', 'ravager', 'glacier', 'obelisk', 'titan', 'leviathan']
TEAMS = ['enemy', 'p1', 'p2', 'green', 'white', 'orange']


# ---------------------------------------------------------------------------
# Shapes: sets of (x, y) design pixels, pivot-centred
# ---------------------------------------------------------------------------
def rect(x0, y0, x1, y1):
    if x1 < x0:
        x0, x1 = x1, x0
    if y1 < y0:
        y0, y1 = y1, y0
    return {(x, y) for x in range(x0, x1 + 1) for y in range(y0, y1 + 1)}


def chamfer(x0, y0, x1, y1, tl=0, tr=0, br=0, bl=0):
    """A rect with 45-degree corner cuts of the given depths."""
    out = set()
    for (x, y) in rect(x0, y0, x1, y1):
        if tl and (x - x0) + (y - y0) < tl:
            continue
        if tr and (x1 - x) + (y - y0) < tr:
            continue
        if br and (x1 - x) + (y1 - y) < br:
            continue
        if bl and (x - x0) + (y1 - y) < bl:
            continue
        out.add((x, y))
    return out


def ellipse(cx, cy, rx, ry):
    """Pixels whose centres lie inside the ellipse; (cx, cy) in continuous
    coordinates (the pivot is (0, 0), a pixel (x, y) spans [x, x+1))."""
    out = set()
    for y in range(int(math.floor(cy - ry)) - 1, int(math.ceil(cy + ry)) + 1):
        for x in range(int(math.floor(cx - rx)) - 1, int(math.ceil(cx + rx)) + 1):
            dx = (x + 0.5 - cx) / rx
            dy = (y + 0.5 - cy) / ry
            if dx * dx + dy * dy <= 1.0:
                out.add((x, y))
    return out


def circle(cx, cy, r):
    return ellipse(cx, cy, r, r)


def ring(cx, cy, r_out, r_in):
    return circle(cx, cy, r_out) - circle(cx, cy, r_in)


def poly(points):
    """Pixels whose centres lie inside the polygon (even-odd rule).
    Points are continuous coordinates - a vertex at (x, y) is a pixel
    corner."""
    xs = [p[0] for p in points]
    ys = [p[1] for p in points]
    out = set()
    n = len(points)
    for y in range(int(math.floor(min(ys))), int(math.ceil(max(ys))) + 1):
        py = y + 0.5
        for x in range(int(math.floor(min(xs))), int(math.ceil(max(xs))) + 1):
            px_ = x + 0.5
            inside = False
            j = n - 1
            for i in range(n):
                xi, yi = points[i]
                xj, yj = points[j]
                if (yi > py) != (yj > py):
                    xint = xi + (py - yi) * (xj - xi) / (yj - yi)
                    if px_ < xint:
                        inside = not inside
                j = i
            if inside:
                out.add((x, y))
    return out


def line(x0, y0, x1, y1):
    """A one-pixel Bresenham line between two pixels."""
    out = set()
    dx, dy = abs(x1 - x0), -abs(y1 - y0)
    sx = 1 if x0 < x1 else -1
    sy = 1 if y0 < y1 else -1
    err = dx + dy
    x, y = x0, y0
    while True:
        out.add((x, y))
        if x == x1 and y == y1:
            break
        e2 = 2 * err
        if e2 >= dy:
            err += dy
            x += sx
        if e2 <= dx:
            err += dx
            y += sy
    return out


def mask(rows, x0, y0, on='#'):
    """An ASCII mask: every character in `on` is a pixel, (x0, y0) the top
    left."""
    out = set()
    for j, row in enumerate(rows):
        for i, ch in enumerate(row):
            if ch in on:
                out.add((x0 + i, y0 + j))
    return out


def mirror(px):
    return {(-1 - x, y) for (x, y) in px}


def sym(px):
    return set(px) | mirror(px)


def shift(px, dx, dy):
    return {(x + dx, y + dy) for (x, y) in px}


def flipv(px):
    return {(x, -1 - y) for (x, y) in px}


def bbox(px):
    xs = [p[0] for p in px]
    ys = [p[1] for p in px]
    return min(xs), min(ys), max(xs), max(ys)


def grow(px, n=1, diag=False):
    out = set(px)
    for _ in range(n):
        nxt = set(out)
        for (x, y) in out:
            nbrs = [(1, 0), (-1, 0), (0, 1), (0, -1)]
            if diag:
                nbrs += [(1, 1), (1, -1), (-1, 1), (-1, -1)]
            for dx, dy in nbrs:
                nxt.add((x + dx, y + dy))
        out = nxt
    return out


def shrink(px, n=1):
    out = set(px)
    for _ in range(n):
        out = {(x, y) for (x, y) in out if all((x + dx, y + dy) in out for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)))}
    return out


def edge(px):
    """The pixels of `px` with a 4-neighbour outside it."""
    return {(x, y) for (x, y) in px if any((x + dx, y + dy) not in px for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)))}


def rotate_px(px, deg, cx=0.0, cy=0.0):
    """Rotate a pixel set about a continuous centre (nearest-neighbour,
    by inverse mapping so the result has no holes)."""
    if not px:
        return set()
    r = math.radians(deg)
    c, s = math.cos(r), math.sin(r)
    x0, y0, x1, y1 = bbox(px)
    rad = math.hypot(max(abs(x0 - cx), abs(x1 + 1 - cx)), max(abs(y0 - cy), abs(y1 + 1 - cy))) + 2
    out = set()
    for y in range(int(cy - rad), int(cy + rad) + 1):
        for x in range(int(cx - rad), int(cx + rad) + 1):
            dx, dy = x + 0.5 - cx, y + 0.5 - cy
            sx = c * dx + s * dy + cx
            sy = -s * dx + c * dy + cy
            if (int(math.floor(sx)), int(math.floor(sy))) in px:
                out.add((x, y))
    return out


# ---------------------------------------------------------------------------
# Lighting
# ---------------------------------------------------------------------------
_L = (-0.5, -0.72, 1.0)
_n = math.sqrt(sum(v * v for v in _L))
LIGHT = tuple(v / _n for v in _L)
I_FLAT = LIGHT[2]
STEP_I = 0.15


def light_steps(nx, ny, nz, gain=1.0):
    n = math.sqrt(nx * nx + ny * ny + nz * nz) or 1.0
    i = (nx * LIGHT[0] + ny * LIGHT[1] + nz * LIGHT[2]) / n
    return int(round((i - I_FLAT) / STEP_I * gain))


# ---------------------------------------------------------------------------
# Parts
# ---------------------------------------------------------------------------
class Part:
    """One piece of a layer. `px` its pixels, `mat` its ramp, `mode` how it
    is shaded, `z` how high it stands (it covers and shadows lower parts),
    `step` an offset along the ramp for the whole part, `tags` what the
    damage pass may do to it, `glow` an emissive role (see `Ctx.glow`)."""

    def __init__(self, px, mat, mode='plate', z=0, step=0, bevel=1, tags=(), name=None, sep=None,
                 normal=None, glow=None, blink=None, pattern=None, stepmap=None, corner=True,
                 contact=True, mat2=None, axis=None, center=None, gain=1.0, cast=True, shade_under=True):
        self.px = set(px)
        self.mat = mat
        self.mode = mode
        self.z = z
        self.step = step
        self.bevel = bevel
        self.tags = set(tags)
        self.name = name
        self.sep = sep
        self.normal = normal
        self.glow = glow
        self.blink = blink
        self.pattern = pattern or {}
        self.stepmap = stepmap
        self.corner = corner
        self.contact = contact
        self.mat2 = mat2
        self.axis = axis
        self.center = center
        self.gain = gain
        self.cast = cast
        self.shade_under = shade_under

    def copy(self, **kw):
        p = Part(self.px, self.mat)
        p.__dict__.update(self.__dict__)
        p.px = set(self.px)
        p.tags = set(self.tags)
        p.pattern = dict(self.pattern)
        for k, v in kw.items():
            setattr(p, k, v)
        return p


def bevel_offsets(px, width=1, corner=True):
    """Top/left faces catch the light (+1), bottom/right fall away (-1)."""
    off = {}
    for (x, y) in px:
        lit = any((x, y - k) not in px or (x - k, y) not in px for k in range(1, width + 1))
        dark = any((x, y + k) not in px or (x + k, y) not in px for k in range(1, width + 1))
        o = 0
        if lit and not dark:
            o = 1
        elif dark and not lit:
            o = -1
        if corner:
            if (x, y - 1) not in px and (x - 1, y) not in px and (x, y + 1) in px and (x + 1, y) in px:
                o = 2
            elif (x, y + 1) not in px and (x + 1, y) not in px and (x, y - 1) in px and (x - 1, y) in px:
                o = -2
        off[(x, y)] = o
    return off


def shade_part(p, frame=0):
    """Per-pixel ramp offsets for a part, from its mode."""
    px = p.px
    if not px:
        return {}
    mode = p.mode
    off = {}
    x0_, y0_, x1_, y1_ = bbox(px)
    corner = p.corner and (x1_ - x0_) >= 2 and (y1_ - y0_) >= 2
    if mode in ('plate', 'inset'):
        b = bevel_offsets(px, p.bevel, corner) if p.bevel else {q: 0 for q in px}
        sgn = 1 if mode == 'plate' else -1
        for q in px:
            off[q] = p.step + sgn * b[q]
    elif mode == 'facet':
        nx, ny = p.normal or (0.0, 0.0)
        base = light_steps(nx, ny, 1.0, p.gain)
        b = bevel_offsets(px, p.bevel, corner) if p.bevel else {q: 0 for q in px}
        for q in px:
            off[q] = p.step + base + b[q]
    elif mode == 'dome':
        x0, y0, x1, y1 = bbox(px)
        cx, cy = p.center if p.center else ((x0 + x1 + 1) / 2.0, (y0 + y1 + 1) / 2.0)
        rx = max(0.5, (x1 + 1 - x0) / 2.0)
        ry = max(0.5, (y1 + 1 - y0) / 2.0)
        for (x, y) in px:
            dx = (x + 0.5 - cx) / rx
            dy = (y + 0.5 - cy) / ry
            r2 = min(0.97, dx * dx + dy * dy)
            nz = math.sqrt(1.0 - r2)
            off[(x, y)] = p.step + light_steps(dx, dy, nz, p.gain)
        if p.bevel:
            for q in edge(px):
                x, y = q
                if (x + 1, y) not in px or (x, y + 1) not in px:
                    off[q] -= 1
    elif mode in ('cylv', 'cylh'):
        x0, y0, x1, y1 = bbox(px)
        for (x, y) in px:
            if mode == 'cylv':
                # Per row, so a tapered barrel still reads round.
                row = [xx for (xx, yy) in px if yy == y]
                a, b = min(row), max(row)
                c = (a + b + 1) / 2.0
                r = max(0.5, (b + 1 - a) / 2.0)
                d = (x + 0.5 - c) / r
                off[(x, y)] = p.step + light_steps(d, 0.0, math.sqrt(max(0.05, 1 - d * d)), p.gain)
            else:
                col = [yy for (xx, yy) in px if xx == x]
                a, b = min(col), max(col)
                c = (a + b + 1) / 2.0
                r = max(0.5, (b + 1 - a) / 2.0)
                d = (y + 0.5 - c) / r
                off[(x, y)] = p.step + light_steps(0.0, d, math.sqrt(max(0.05, 1 - d * d)), p.gain)
        if p.bevel:
            # Cylinder ends: the far end falls away.
            for (x, y) in px:
                if mode == 'cylv' and (x, y + 1) not in px:
                    off[(x, y)] -= 1
                if mode == 'cylh' and (x + 1, y) not in px:
                    off[(x, y)] -= 1
    elif mode == 'flat':
        for q in px:
            off[q] = p.step
    elif mode == 'grille':
        period = p.pattern.get('period', 2)
        horizontal = p.pattern.get('dir', 'h') == 'h'
        x0, y0, x1, y1 = bbox(px)
        b = bevel_offsets(px, 1, False)
        for (x, y) in px:
            k = (y - y0) if horizontal else (x - x0)
            slat = (k % period) == 0
            off[(x, y)] = p.step + (1 if slat else -2) + min(0, b[(x, y)])
    elif mode == 'tread':
        off = tread_offsets(p, frame)
    elif mode == 'glass':
        b = bevel_offsets(px, 1, False)
        x0, y0, x1, y1 = bbox(px)
        for (x, y) in px:
            o = p.step + (1 if b[(x, y)] < 0 else 0) - (1 if b[(x, y)] > 0 else 0)
            off[(x, y)] = o
        # A glint: the upper-left inner pixels.
        inner = sorted(px, key=lambda q: (q[0] - x0) + (q[1] - y0))
        if len(px) >= 4:
            off[inner[min(1, len(inner) - 1)]] = p.step + 3
        if len(px) >= 9:
            off[inner[min(2, len(inner) - 1)]] = p.step + 2
    elif mode == 'map':
        for q in px:
            off[q] = p.step + (p.stepmap.get(q, 0) if p.stepmap else 0)
    elif mode == 'stripes':
        w = p.pattern.get('width', 2)
        for (x, y) in px:
            off[(x, y)] = p.step
    elif mode == 'rivet':
        for q in px:
            off[q] = p.step + 2
    else:
        raise ValueError('unknown mode %s' % mode)
    return off


def tread_offsets(p, frame):
    """A track run: links every `period` rows scrolling one row per frame,
    shaded across its width (outer edge, links, inner shadow)."""
    pat = p.pattern
    period = pat.get('period', 4)
    side = pat.get('side', 'L')
    style = pat.get('style', 'std')
    direction = pat.get('dir', 1)
    x0, y0, x1, y1 = bbox(p.px)
    w = x1 - x0 + 1
    off = {}
    for (x, y) in p.px:
        ph = (y - y0 + frame * direction) % period
        # Across the run: the outer column is lit on the left track,
        # shadowed on the right one; the inner column sits in the hull's
        # shadow.
        if w == 1:
            across = 0
        else:
            k = (x - x0) if side == 'L' else (x1 - x)
            if k == 0:
                across = 1 if side == 'L' else -1
            elif k == w - 1:
                across = -1
            else:
                across = 0
        if style == 'std':
            link = [1, 0, -1, -2][ph % 4] if period == 4 else [1, -1][ph % 2]
        elif style == 'block':
            link = [1, 1, 0, -2][ph % 4] if period == 4 else [0, -2][ph % 2]
        elif style == 'fine':
            link = [0, -1][ph % 2]
        elif style == 'chevron':
            k = (x - x0) if side == 'L' else (x1 - x)
            link = [1, 0, -1, -2][(ph + k) % 4]
        elif style == 'pad':
            link = [1, 1, -1, -2][ph % 4]
        else:
            link = 0
        # Run ends curl round the sprockets.
        end = 0
        if y == y0 or y == y1:
            end = -1
        off[(x, y)] = p.step + link + across + end
    return off


# ---------------------------------------------------------------------------
# Context: one team's materials and emissive roles for one chassis
# ---------------------------------------------------------------------------
class Ctx:
    def __init__(self, chassis, team='enemy'):
        spec = CHASSIS[chassis]
        self.chassis = chassis
        self.spec = spec
        self.team = team
        self.fp = spec['fp']
        ramp, base = spec['body']
        aramp, abase = spec['accent']
        if team == 'enemy':
            self.body = Mat('body', ramp, base, 'body')
            self.accent = Mat('accent', aramp, abase, 'accent')
            self.glow_marker = spec['glow']
        else:
            tr = TEAM_RAMPS[team]
            # The body ramp's base lands on the team base, its length is
            # the team ramp's.
            self.body = Mat('body', tr, 3, 'body')
            self.accent = Mat('accent', [tr[2], tr[4], tr[5], WHITE], 1, 'accent')
            self.glow_marker = TEAM_LIGHT[team]
        self.player = team != 'enemy'

    def glow(self, role):
        """(core, rim) colours of an emissive role for this team."""
        if role == 'lamp':
            return (WHITE, WOOD_PALE) if self.player else (WOOD_PALE, GOLD_BRIGHT)
        if role == 'tail':
            return (RED_BRIGHT, RED_MD)
        if role == 'marker':
            return (self.glow_marker, self.glow_marker)
        if role == 'sensor':
            return (TEAM_LIGHT[self.team], TEAM_LIGHT[self.team]) if self.player else (RED_BRIGHT, RED_MD)
        if role == 'beacon':
            return (GOLD_BRIGHT, WOOD_LT)
        if role == 'warn':
            return (RED_BRIGHT, RED_MD)
        if role == 'engine':
            return (GOLD_BRIGHT, WOOD_LT)
        if role == 'ion':
            return (BLUE_PALE, BLUE_BRIGHT)
        if role == 'core':
            return (WHITE, self.glow_marker)
        if role == 'plasma':
            return (BLUE_PALE, BLUE_BRIGHT)
        if role == 'laser':
            return (WHITE, RED_BRIGHT)
        if role == 'fire':
            return (GOLD_BRIGHT, RED_BRIGHT)
        if role == 'hot':
            return (GOLD_BRIGHT, RED_MD)
        if role == 'ember':
            return (GOLD_BRIGHT, RED_MD)
        if role == 'spark':
            return (WHITE, GOLD_BRIGHT)
        if role == 'white':
            return (WHITE, STONE_PALE)
        if isinstance(role, tuple):
            return (role, role)
        raise ValueError('unknown glow role %r' % (role,))


# ---------------------------------------------------------------------------
# Builder: what a design's hull/turret function draws into
# ---------------------------------------------------------------------------
class Builder:
    def __init__(self, ctx, layer, frame=0, pose=0):
        self.ctx = ctx
        self.layer = layer
        self.frame = frame
        self.pose = pose
        self.parts = []
        self.lights = []
        self.meta = {}

    # -- generic
    def part(self, px, mat, mode='plate', z=0, **kw):
        p = Part(px, mat, mode, z, **kw)
        self.parts.append(p)
        return p

    def sym(self, px, mat, mode='plate', z=0, **kw):
        return self.part(sym(px), mat, mode, z, **kw)

    def cut(self, px):
        """Remove pixels from every part drawn so far (a notch, a slot)."""
        px = set(px)
        for p in self.parts:
            p.px -= px

    # -- components
    def tread(self, x0, x1, y0, y1, side='L', z=1, style='std', period=4, mat=None, tags=('track',), step=0, name=None):
        return self.part(rect(x0, y0, x1, y1), mat or TRACK, 'tread', z, step=step, tags=tags, name=name,
                         pattern=dict(side=side, style=style, period=period))

    def tread_px(self, px, side='L', z=1, style='std', period=4, mat=None, tags=('track',), step=0, name=None):
        return self.part(px, mat or TRACK, 'tread', z, step=step, tags=tags, name=name,
                         pattern=dict(side=side, style=style, period=period))

    def barrel(self, xc2, width, y_tip, y_base, mat=None, z=6, brake=0, brake_len=2, bore=True, tags=('barrel',),
               name=None, step=0, recoil=0):
        """A barrel centred on x = xc2 / 2 (xc2 is twice the centre, so
        an even-width barrel on the pivot line is xc2 = 0), from the muzzle
        row y_tip back to y_base, pulled back `recoil` rows."""
        mat = mat or GUNMETAL
        x0 = (xc2 - width) // 2
        x1 = x0 + width - 1
        y_tip += recoil
        px = rect(x0, y_tip, x1, y_base)
        parts = [self.part(px, mat, 'cylv', z, tags=tags, name=name, step=step, bevel=0)]
        if brake:
            bpx = rect(x0 - brake, y_tip, x1 + brake, y_tip + brake_len - 1)
            parts.append(self.part(bpx, mat, 'cylv', z + 0.1, tags=tags + ('brake',), step=step, bevel=0))
        if bore:
            bx0 = x0 + (1 if width >= 3 else 0)
            bx1 = x1 - (1 if width >= 3 else 0)
            if width <= 2:
                bpx = rect(x0, y_tip, x1, y_tip)
                parts.append(self.part(bpx, mat, 'flat', z + 0.2, step=-3 + step, tags=tags + ('bore',), contact=False, cast=False))
            else:
                parts.append(self.part(rect(bx0, y_tip, bx1, y_tip), DARK, 'flat', z + 0.2, step=-1, tags=tags + ('bore',), contact=False, cast=False))
        return parts

    def lamp(self, px, role='lamp', z=5, kind=None, dir=0.0, cone=None, reach=None, blink=None, tags=None, housing=None,
             housing_z=None, name=None):
        """An emissive fixture. `kind` registers a light the night render
        casts (head/spot: a cone along `dir` degrees off the layer's facing;
        tail/marker/beacon/sensor: a point)."""
        px = set(px)
        t = set(tags or ()) | {'glow', 'lamp' if role == 'lamp' else role}
        if housing is not None:
            self.part(grow(px, 1) - px, housing, 'plate', housing_z if housing_z is not None else z - 0.5, tags=('housing',))
        p = self.part(px, None, 'glow', z, glow=role, blink=blink, tags=t, name=name, contact=False, cast=False)
        if kind:
            x0, y0, x1, y1 = bbox(px)
            self.lights.append(dict(kind=kind, role=role, x=(x0 + x1 + 1) / 2.0, y=(y0 + y1 + 1) / 2.0, dir=dir,
                                    cone=cone, reach=reach, name=name))
        return p

    def rivets(self, pts, mat, z, step=0):
        px = set(pts)
        return self.part(px, mat, 'rivet', z, step=step, tags=('rivet',), contact=True, cast=False)

    def grille(self, x0, y0, x1, y1, mat, z, period=2, dir='h', step=0, tags=('vent',)):
        return self.part(rect(x0, y0, x1, y1), mat, 'grille', z, step=step, tags=tags, pattern=dict(period=period, dir=dir))

    def hazard(self, px, z, width=2, flip=False, tags=('paint',)):
        """Diagonal gold/black warning stripes."""
        px = set(px)
        a, b = set(), set()
        for (x, y) in px:
            k = ((x - y) if not flip else (x + y)) // width
            (a if k % 2 == 0 else b).add((x, y))
        self.part(a, HAZARD, 'flat', z, tags=tags)
        self.part(b, DARK, 'flat', z, step=0, tags=tags)

    def glass(self, px, z, mat=None, step=0, tags=('optic',)):
        return self.part(px, mat or GLASS, 'glass', z, step=step, tags=tags)

    def dome(self, px, mat, z, step=0, center=None, gain=1.0, tags=(), bevel=1, name=None):
        return self.part(px, mat, 'dome', z, step=step, center=center, gain=gain, tags=tags, bevel=bevel, name=name)

    def antenna(self, x, y0, y1, z=9, mat=None, tip=True, tags=('antenna',)):
        mat = mat or STEEL
        self.part(rect(x, y1, x, y0), mat, 'flat', z, step=1, tags=tags, contact=False, cast=False)
        if tip:
            self.part({(x, y1)}, mat, 'flat', z + 0.1, step=3, tags=tags, contact=False, cast=False)

    def light(self, **kw):
        self.lights.append(kw)

    def paint(self, rows, x0, y0, legend, mirror_x=False):
        """Hand-placed pixels: every character of `rows` names a legend
        entry - dict(mat=, mode=, z=, step=, tags=, name=, glow=, kind=,
        ...) - and each entry becomes one part. `.` and space are empty.
        With `mirror_x` the rows are the left half and are mirrored onto
        the right (the pivot line between the last column and its
        mirror)."""
        groups = {}
        for j, row in enumerate(rows):
            for i, ch in enumerate(row):
                if ch in '. ':
                    continue
                groups.setdefault(ch, set()).add((x0 + i, y0 + j))
        made = {}
        for ch, px in groups.items():
            spec = dict(legend[ch])
            if mirror_x and spec.get('normal') and spec['normal'][0] != 0:
                # A mirrored facet faces the other way: split it so the
                # right half gets its own normal.
                nx, ny = spec['normal']
                left = dict(spec)
                right = dict(spec, normal=(-nx, ny))
                mat = left.pop('mat')
                mode = left.pop('mode', 'facet')
                z = left.pop('z', 0)
                right.pop('mat'), right.pop('mode', None), right.pop('z', None)
                made[ch] = self.part(px, mat, mode, z, **left)
                self.part(mirror(px), mat, mode, z, **right)
                continue
            halves = [px]
            if mirror_x:
                right = mirror(px)
                touching = any((x + dx, y + dy) in right for (x, y) in px for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1), (0, 0)))
                if touching:
                    halves = [px | right]
                else:
                    halves = [px, right]
            if len(halves) == 2:
                first = None
                for h, suffix in zip(halves, ('_l', '_r')):
                    s2 = dict(spec)
                    if s2.get('name'):
                        s2['name'] = s2['name'] + suffix
                    if 'glow' in s2:
                        role = s2.pop('glow')
                        made_part = self.lamp(h, role, z=s2.pop('z', 5), kind=s2.pop('kind', None), dir=s2.pop('dir', 0.0),
                                              name=s2.pop('name', None), blink=s2.pop('blink', None), tags=s2.pop('tags', None))
                    else:
                        mat = s2.pop('mat')
                        mode = s2.pop('mode', 'plate')
                        z = s2.pop('z', 0)
                        made_part = self.part(h, mat, mode, z, **s2)
                    first = first or made_part
                made[ch] = first
                continue
            px = halves[0]
            if 'glow' in spec:
                role = spec.pop('glow')
                kind = spec.pop('kind', None)
                made[ch] = self.lamp(px, role, z=spec.pop('z', 5), kind=kind, dir=spec.pop('dir', 0.0),
                                     name=spec.pop('name', None), blink=spec.pop('blink', None),
                                     tags=spec.pop('tags', None))
                continue
            mat = spec.pop('mat')
            mode = spec.pop('mode', 'plate')
            z = spec.pop('z', 0)
            made[ch] = self.part(px, mat, mode, z, **spec)
        return made


# ---------------------------------------------------------------------------
# Composition
# ---------------------------------------------------------------------------
class Pix:
    __slots__ = ('part', 'off', 'color', 'emis', 'z', 'mat')

    def __init__(self, part, off, z):
        self.part = part
        self.off = off
        self.color = None
        self.emis = None
        self.z = z
        self.mat = None   # a damage pass's material swap (burnt), same offsets


def compose(parts, frame=0):
    """Rasterise, shade and contact-shadow a layer. Returns {(x, y): Pix}."""
    order = sorted(range(len(parts)), key=lambda i: (parts[i].z, i))
    grid = {}
    for i in order:
        p = parts[i]
        if not p.px:
            continue
        if p.mode == 'glow':
            offs = {q: 0 for q in p.px}
        else:
            offs = shade_part(p, frame)
        for q in p.px:
            x, y = q
            if -HALF <= x < HALF and -HALF <= y < HALF:
                grid[q] = Pix(p, offs.get(q, 0), p.z)
    # Contact shadows: a raised part shades what lies just below-right of it.
    shadowed = []
    for q, pix in grid.items():
        p = pix.part
        if not p.contact or p.mode == 'glow':
            continue
        x, y = q
        hit = False
        for dx, dy in ((-1, 0), (0, -1), (-1, -1)):
            o = grid.get((x + dx, y + dy))
            if o is not None and o.part is not p and o.part.cast and o.z > pix.z + 0.45:
                if dx == -1 and dy == -1 and o.z < pix.z + 1.5:
                    continue
                hit = True
                break
        if hit:
            shadowed.append(q)
    for q in shadowed:
        grid[q].off -= 1
    # Separation lines: a part flagged `sep` darkens its own border where
    # it meets a lower part.
    for q, pix in list(grid.items()):
        p = pix.part
        if not p.sep:
            continue
        x, y = q
        for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            o = grid.get((x + dx, y + dy))
            if o is not None and o.part is not p and o.z < pix.z:
                pix.color = BLACK if p.sep == 'black' else p.mat.ramp[0]
                break
    return grid


def resolve(grid, ctx, frame=0):
    """Colours and emissive for every pixel of a composed layer."""
    for q, pix in grid.items():
        p = pix.part
        if pix.color is not None and pix.emis is None and p.mode != 'glow':
            continue
        if p.mode == 'glow' and pix.color is None:
            on = True
            if p.blink is not None:
                on = frame in p.blink
            core, rim = ctx.glow(p.glow)
            # Edge pixels of a wide glow get the rim colour.
            x, y = q
            is_edge = any((x + dx, y + dy) not in p.px for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)))
            c = core if (not is_edge or len(p.px) <= 3) else rim
            if on:
                pix.color = c
                pix.emis = c
            else:
                pix.color = _dim(c)
                pix.emis = None
        elif pix.color is None:
            m = pix.mat or p.mat
            pix.color = m.at(pix.off)
    return grid


def _dim(c):
    """An unlit lens: the colour two palette steps down, roughly."""
    return nearest((c[0] * 0.45, c[1] * 0.45, c[2] * 0.45))


_near_cache = {}


def nearest(rgb, palette=None):
    palette = palette or PALETTE
    key = (id(palette), int(rgb[0]), int(rgb[1]), int(rgb[2]))
    hit = _near_cache.get(key)
    if hit:
        return hit
    best, bd = None, None
    for c in palette:
        d = (c[0] - key[1]) ** 2 + (c[1] - key[2]) ** 2 + (c[2] - key[3]) ** 2
        if bd is None or d < bd:
            best, bd = c, d
    _near_cache[key] = best
    return best


def to_images(grid, outline=True, outline_color=BLACK):
    """(base RGBA, emissive RGBA) 40 x 40 images of a resolved layer."""
    img = Image.new('RGBA', (S, S), (0, 0, 0, 0))
    em = Image.new('RGBA', (S, S), (0, 0, 0, 0))
    for (x, y), pix in grid.items():
        img.putpixel((x + HALF, y + HALF), tuple(pix.color) + (255,))
        if pix.emis is not None:
            em.putpixel((x + HALF, y + HALF), tuple(pix.emis) + (255,))
    if outline:
        occ = set(grid.keys())
        for y in range(-HALF, HALF):
            for x in range(-HALF, HALF):
                if (x, y) in occ:
                    continue
                if any((x + dx, y + dy) in occ for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1))):
                    img.putpixel((x + HALF, y + HALF), tuple(outline_color) + (255,))
    return img, em


# ---------------------------------------------------------------------------
# Designs and lines
# ---------------------------------------------------------------------------
class Design:
    """Subclass per chassis in a line module and decorate with
    `@line.design('<chassis>')`. Override `hull(b, f)` and `turret(b, pose)`;
    `module(b, weapon, state)` defaults to the line's."""

    line = None
    chassis = None
    codename = ''
    blurb = ''
    locomotion = 'tracks'   # tracks | hover | quad | wheels | legs
    marks = 'tread'         # the ground mark the design leaves
    hover = 0               # extra shadow offset (design px) for a hovering hull
    damage_seed = 0

    def __init__(self):
        pass

    def hull(self, b, f):
        raise NotImplementedError

    def turret(self, b, pose):
        raise NotImplementedError

    def hardpoints(self, ctx):
        """Where each weapon module sits on the turret, turret-local.
        Default: minigun on the right cheek, laser on the left, missiles on
        the roof behind the pivot, flamethrower under the main gun, plasma
        sleeved on the barrel(s)."""
        return self.line.default_hardpoints(self, ctx)

    def module(self, b, weapon, state, hp):
        return self.line.module(self, b, weapon, state, hp)

    # Recoil: pose 1 = first barrel back 2, pose 2 = second barrel back 2
    # (single-barrel: pose 2 = the barrel returning, back 1).
    def recoil(self, pose, barrel_index=0, n_barrels=1):
        if pose == 0:
            return 0
        if n_barrels == 1:
            return 2 if pose == 1 else 1
        if pose == 1:
            return 2 if barrel_index == 0 else 0
        return 1 if barrel_index == 0 else 2


class Line:
    def __init__(self, key, title, tagline, notes=''):
        self.key = key
        self.title = title
        self.tagline = tagline
        self.notes = notes
        self.designs = {}
        self.modules = {}

    def design(self, chassis):
        def deco(cls):
            cls.line = self
            cls.chassis = chassis
            self.designs[chassis] = cls
            return cls
        return deco

    def module_fn(self, weapon):
        def deco(fn):
            self.modules[weapon] = fn
            return fn
        return deco

    def default_hardpoints(self, design, ctx):
        return dict(minigun=(4, -2), laser=(-5, -2), missiles=(0, 3), flame=(-3, -3), plasma=(0, -4))

    def module(self, design, b, weapon, state, hp):
        fn = self.modules.get(weapon)
        if fn is None:
            return
        fn(design, b, state, hp)


WEAPONS = ['minigun', 'missiles', 'plasma', 'laser', 'flame', 'grenade', 'sonic']
WEAPON_STATES = {'minigun': 4, 'missiles': 5, 'plasma': 3, 'laser': 3, 'flame': 4, 'grenade': 5, 'sonic': 4}
