"""The defence towers' sheet: static/towers_sheet.png (docs/TOWERS_SPEC.md,
docs/defence-towers-prd.md section 12).

The three designs the author picked in the design artifact - T3 capacitor
dome, G2 armoured turret, B2 sludge mortar - each split, like a tank, into a
base that stands still and a top layer the game rotates to the tower's
heading. 48 px cells on the 2 px block grid (24 x 24 design pixels, the
trees' density), the tanks' 1-design-pixel #252525 outline round the outside
of each layer, the Puny palette with its extended steel greys. Two liveries
per tower: the player's trim in the TEAM_P1 ramp, the enemy's in the red
ramp. The ooze (the mortar's canisters and mouth) is the one green on the
sheet, off the palette on purpose like plasma.png, admitted by
tools/check_sheets.py for this sheet alone.

Layout, 8 columns x 6 rows of 48 px:

    row = kind * 2 + side        kind: tesla, gun, bio; side: player, enemy
    col 0-3  base, four standing stages (intact, scuffed, damaged, critical)
    col 4    the ruin
    col 5    top layer, intact, pointing up
    col 6    top layer, damaged
    col 7    glow overlay (the tesla's lit lens and prong tips, the mortar's
             loaded mouth), drawn over the top with its charge as alpha

No Pillow: raw PNG bytes, the pickup generators' convention. Deterministic:
every choice is a position hash.

    python3 tools/spritegen/gen_towers.py        # writes static/towers_sheet.png
"""

import math
import os
import struct
import sys
import zlib

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
import punypalette as pp  # noqa: E402

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'static', 'towers_sheet.png')

GRID = 24          # design pixels per cell side
CELL = GRID * 2    # sheet pixels per cell side
COLS, ROWS = 8, 6

C = {
    'K': pp.BLACK, 'W': pp.WHITE,
    'S0': pp.STONE_PALE, 'SH': pp.STONE_HI, 'S1': pp.STONE_LT, 'SM': pp.STONE_MID, 'S2': pp.STONE_MD,
    'SK': pp.STONE_MDK, 'S3': pp.STONE_DK, 'SS': pp.STONE_SHADE, 'S4': pp.STONE_DARKEST,
    'W0': pp.WOOD_PALE, 'W1': pp.WOOD_LT, 'W2': pp.WOOD_MD, 'WA': pp.WOOD_AMBER, 'W3': pp.WOOD_DK,
    'W4': pp.WOOD_DEEPER, 'W5': pp.WOOD_DARKEST,
    'R0': pp.RED_BRIGHT, 'R1': pp.RED_MD, 'R2': pp.RED_DEEP, 'R3': pp.RED_DK, 'R4': pp.RED_DARKEST,
    'B0': pp.BLUE_BRIGHT, 'B1': pp.BLUE_LT, 'B2': pp.BLUE_MD, 'B3': pp.BLUE_DK, 'B4': pp.BLUE_DARKEST,
    'BP': pp.BLUE_PALE, 'G0': pp.GOLD_BRIGHT, 'G1': pp.GOLD_MD,
    'RU': pp.RUST_MD, 'RK': pp.RUST_DK, 'WX': pp.WOOD_ASH,
}

# The side's trim (dark to light): the player's team blue, the enemy's red.
TRIM = {
    'player': list(pp.TEAM_P1),
    'enemy': [pp.RED_DK, pp.RED_DEEP, pp.RED_MD, pp.RED_BRIGHT],
}

# The ooze, acid lime (the author's O1 pick): hi, lt, md, dk, darkest.
OOZE = list(pp.OOZE)

CHAINS = [
    ['S0', 'SH', 'S1', 'SM', 'S2', 'SK', 'S3', 'SS', 'S4', 'K'],
    ['W0', 'W1', 'W2', 'WA', 'W3', 'W4', 'W5', 'K'],
    ['R0', 'R1', 'R2', 'R3', 'R4', 'K'],
    ['BP', 'B0', 'B1', 'B2', 'B3', 'B4', 'S4'],
    ['G0', 'G1', 'WA', 'W3'],
    ['RU', 'RK', 'W5'],
    ['A3', 'A2', 'A1', 'A0'],
    ['Z0', 'Z1', 'Z2', 'Z3', 'Z4'],
]
CHAR = ['S4', 'K', 'RK', 'W5', 'SS', 'R4']
NOWEAR, GLOW = 2, 4
ORB_HOT = ['W', 'BP', 'B0', 'B2']


def darker(c, n=1):
    for chain in CHAINS:
        if c in chain:
            return chain[min(chain.index(c) + n, len(chain) - 1)]
    return c


def h32(x, y, s):
    h = (x * 0x27d4eb2d) ^ ((y + 0x165667b1) * 668265263) ^ ((s + 0x9e3779b9) * 2246822519)
    h &= 0xffffffff
    h = ((h ^ (h >> 15)) * 2246822519) & 0xffffffff
    h = ((h ^ (h >> 13)) * 3266489917) & 0xffffffff
    h ^= h >> 16
    return h / 4294967296.0


class Spr:
    """One layer on the 24 x 24 design grid: a colour key per pixel and a
    tag (NOWEAR, GLOW) that keeps wear off outlines and lit parts."""

    def __init__(self):
        self.c = [[None] * GRID for _ in range(GRID)]
        self.t = [[0] * GRID for _ in range(GRID)]

    def ok(self, x, y):
        return 0 <= x < GRID and 0 <= y < GRID

    def p(self, x, y, c, t=0):
        x, y = int(math.floor(x)), int(math.floor(y))
        if self.ok(x, y):
            self.c[y][x] = c
            self.t[y][x] = t

    def get(self, x, y):
        return self.c[y][x] if self.ok(x, y) else None

    def clr(self, x, y):
        if self.ok(x, y):
            self.c[y][x] = None
            self.t[y][x] = 0

    def r(self, x, y, w, h, c, t=0):
        for j in range(h):
            for i in range(w):
                self.p(x + i, y + j, c, t)

    def each(self, cx, cy, rx, ry, fn):
        for y in range(int(math.floor(cy - ry - 1)), int(math.ceil(cy + ry + 1)) + 1):
            for x in range(int(math.floor(cx - rx - 1)), int(math.ceil(cx + rx + 1)) + 1):
                nx, ny = (x + 0.5 - cx) / rx, (y + 0.5 - cy) / ry
                d2 = nx * nx + ny * ny
                if d2 <= 1:
                    fn(x, y, nx, ny, d2)

    def disc(self, cx, cy, r, c, t=0):
        self.each(cx, cy, r, r, lambda x, y, *_: self.p(x, y, c, t))

    def ball(self, cx, cy, r, ramp, t=0):
        def f(x, y, nx, ny, d2):
            nz = math.sqrt(max(0.0, 1 - d2))
            light = -0.5 * nx - 0.6 * ny + 0.62 * nz
            i = 0 if light > 0.86 else 1 if light > 0.62 else 2 if light > 0.3 else 3
            self.p(x, y, ramp[min(i, len(ramp) - 1)], t)
        self.each(cx, cy, r, r, f)

    def line(self, x0, y0, x1, y1, c, t=0):
        x0, y0, x1, y1 = round(x0), round(y0), round(x1), round(y1)
        dx, dy = abs(x1 - x0), -abs(y1 - y0)
        sx, sy = (1 if x0 < x1 else -1), (1 if y0 < y1 else -1)
        e = dx + dy
        while True:
            self.p(x0, y0, c, t)
            if x0 == x1 and y0 == y1:
                break
            e2 = 2 * e
            if e2 >= dy:
                e += dy
                x0 += sx
            if e2 <= dx:
                e += dx
                y0 += sy

    def ray(self, cx, cy, a, r0, r1, c, t=0, off=0.0):
        dx, dy, px, py = math.sin(a), -math.cos(a), math.cos(a), math.sin(a)
        r = r0
        while r <= r1 + 1e-6:
            self.p(cx + dx * r + px * off, cy + dy * r + py * off, c, t)
            r += 0.35

    def barrel(self, cx, cy, a, r0, r1, off=0.0):
        self.ray(cx, cy, a, r0, r1, 'S4', 0, off - 1)
        self.ray(cx, cy, a, r0, r1, 'S4', 0, off + 1)
        self.ray(cx, cy, a, r0, r1, 'S1', 0, off)
        self.ray(cx, cy, a, r1 - 0.7, r1, 'K', 0, off)

    def wear(self, stage, seed):
        if stage <= 0:
            return
        dent = [0, 0.1, 0.2, 0.26][stage]
        hole = [0, 0, 0.05, 0.07][stage]
        for y in range(GRID):
            for x in range(GRID):
                c = self.c[y][x]
                if c is None or self.t[y][x] & (NOWEAR | GLOW):
                    continue
                n = c
                if h32(x >> 1, y >> 1, seed) < dent and h32(x, y, seed + 3) < 0.7:
                    n = darker(n, 2 if stage >= 2 else 1)
                if h32(x, y, seed + 11) < hole:
                    n = 'S4'
                if stage >= 3 and h32(x >> 1, y >> 1, seed + 29) < 0.45 and h32(x, y, seed + 31) < 0.75:
                    n = CHAR[int(h32(x, y, seed + 37) * len(CHAR))]
                self.c[y][x] = n

    def outline(self):
        """The outside of the silhouette only: enclosed gaps stay open."""
        out = [[False] * GRID for _ in range(GRID)]
        stack = [(x, 0) for x in range(GRID)] + [(x, GRID - 1) for x in range(GRID)]
        stack += [(0, y) for y in range(GRID)] + [(GRID - 1, y) for y in range(GRID)]
        while stack:
            x, y = stack.pop()
            if not self.ok(x, y) or out[y][x] or self.c[y][x] is not None:
                continue
            out[y][x] = True
            stack += [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]
        add = []
        for y in range(GRID):
            for x in range(GRID):
                if out[y][x] and any(self.get(x + dx, y + dy) is not None for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1))):
                    add.append((x, y))
        for x, y in add:
            self.p(x, y, 'K', NOWEAR)

    def rgba(self, trim):
        """The layer as 48 x 48 RGBA rows, each design pixel a 2 x 2 block."""
        rows = []
        for y in range(GRID):
            row = []
            for x in range(GRID):
                c = self.c[y][x]
                if c is None:
                    px = (0, 0, 0, 0)
                elif c[0] == 'A':
                    px = tuple(trim[int(c[1])]) + (255,)
                elif c[0] == 'Z':
                    px = tuple(OOZE[int(c[1])]) + (255,)
                else:
                    px = tuple(C[c]) + (255,)
                row += [px, px]
            rows += [row, list(row)]
        return rows


# --- T3, the capacitor dome ------------------------------------------------

PRONGS = [0.0, 2.094, 4.189]


def tesla_base(s, stage):
    s.disc(12, 12, 10.2, 'S4')
    s.ball(12, 12, 9.8, ['S1', 'S2', 'S3', 'SS'])
    for k in range(8):
        s.ray(12, 12, k * math.pi / 4 + math.pi / 8, 7.4, 9.6, 'SS')
    s.each(12, 12, 7.6, 7.6, lambda x, y, nx, ny, d2: s.p(x, y, 'A2') if d2 > 0.78 else None)
    for i, a in enumerate(PRONGS):
        if stage >= 2 and i == 1:
            s.ray(12, 12, a, 6, 8, 'S3', 0, -0.5)
            s.ray(12, 12, a, 6, 8, 'S4', 0, 0.5)
            continue
        s.ray(12, 12, a, 6, 9.6, 'S3', 0, -0.5)
        s.ray(12, 12, a, 6, 9.6, 'S4', 0, 0.5)
        s.ball(12 + math.sin(a) * 10.4, 12 - math.cos(a) * 10.4, 1.5, ['W0', 'W1', 'W2', 'W3'])


def tesla_top(s, damaged):
    s.ball(12, 11.7, 6.2, ['S0', 'S1', 'S2', 'S3'])
    for k in range(6):
        a = k * math.pi / 3 + 0.5
        s.p(12 + math.sin(a) * 5, 11.7 - math.cos(a) * 5, 'S3')
    s.ball(12, 11.5, 3, ['B1', 'B2', 'B3', 'B4'])
    if damaged:
        s.line(11, 10, 13, 12, 'K', NOWEAR)
        s.p(8, 9, 'SS')
        s.p(15, 13, 'SS')


def tesla_glow(s):
    """The lens and the two prongs that survive damage (the second prong
    snaps off at stage 2, and a lit tip must not hang where it was)."""
    s.ball(12, 11.5, 3, ORB_HOT, GLOW)
    for i, a in enumerate(PRONGS):
        if i != 1:
            s.ball(12 + math.sin(a) * 10.4, 12 - math.cos(a) * 10.4, 1.5, ORB_HOT, GLOW)


def tesla_ruin(s):
    s.disc(12, 12, 10.2, 'S4')
    s.ball(12, 12, 9.8, ['S2', 'S3', 'SS', 'S4'])
    for y in range(GRID):
        for x in range(GRID):
            if math.hypot(x + 0.5 - 12, y + 0.5 - 12) > 6.5 and h32(x >> 1, y >> 1, 21) < 0.32:
                s.clr(x, y)
    s.disc(12, 12, 5.8, 'S4')
    s.disc(12.4, 12.6, 4.2, 'K')
    for x, y in ((11, 12), (13, 13), (12, 14)):
        s.p(x, y, 'R1', GLOW)
    for x, y in ((1, 5), (21, 3), (3, 21), (20, 20), (22, 11)):
        s.r(x, y, 2, 2, 'S2')
    s.ray(12, 12, 0, 6, 8, 'S3')
    s.ball(4, 4, 1.3, ['W1', 'W2', 'W3', 'W4'])


# --- G2, the armoured turret -----------------------------------------------

def gun_plate(s, x, y):
    ex, ey = min(x - 4, 19 - x), min(y - 4, 19 - y)
    return ex >= 0 and ey >= 0 and ex + ey >= 2


def gun_base(s, stage):
    for y in range(4, 20):
        for x in range(4, 20):
            if not gun_plate(s, x, y):
                continue
            c = 'S2'
            if x <= 5 or y <= 5:
                c = 'S1'
            if x >= 18 or y >= 18:
                c = 'S3'
            if x + y <= 12 or x + y >= 34:
                c = 'A3' if ((x - y + 40) // 2) % 2 == 0 else 'A0'
            s.p(x, y, c)
    for x, y in ((17, 6), (6, 17)):
        s.p(x, y, 'SS')
    s.disc(12, 12, 6.4, 'SS')
    s.disc(12, 12, 5.4, 'S4')
    if stage >= 2:
        s.r(15, 16, 2, 2, 'S4')
        s.p(7, 8, 'S4')


def gun_top(s, damaged):
    a = 0.0
    s.barrel(12, 12, a, 3, 6.5 if damaged else 10.8, -1.7)
    s.barrel(12, 12, a, 3, 10.8, 1.7)
    off = -2.5
    while off <= 2.5:
        s.ray(12, 12, a, 4.5, 6.4, 'S3', 0, off)
        off += 0.5
    s.ball(12, 12, 5.2, ['S0', 'S1', 'S2', 'S3'])
    s.ball(12, 14.2, 1.5, ['S1', 'S2', 'S3', 'SS'])
    for x, y in ((8, 14), (15, 14)):
        s.p(x, y, 'A2')
    if damaged:
        s.p(10, 10, 'SS')
        s.p(14, 12, 'S4')


def gun_ruin(s):
    for y in range(4, 20):
        for x in range(4, 20):
            if not gun_plate(s, x, y):
                continue
            c = 'SS' if h32(x, y, 2) < 0.2 else 'S3'
            if x + y <= 12 or x + y >= 34:
                c = 'A1' if ((x - y + 40) // 2) % 2 == 0 else 'A0'
            s.p(x, y, c)
    s.disc(11, 11, 4.8, 'S4')
    s.disc(11.3, 11.6, 3.2, 'K')
    s.line(4, 14, 8, 12, 'K')
    s.line(15, 5, 13, 8, 'K')
    s.ray(18, 19, 1.1, -3, 5, 'S3', 0, -1.2)
    s.ray(18, 19, 1.1, -3, 3, 'S3', 0, 1.2)
    s.ball(17.5, 17.5, 3.8, ['S2', 'S3', 'SS', 'S4'])


# --- B2, the sludge mortar -------------------------------------------------

def bio_plate(x, y):
    ex, ey = min(x - 3, 20 - x), min(y - 3, 20 - y)
    return ex >= 0 and ey >= 0 and ex + ey >= 4


CANISTERS = [0.8, 2.9, 5.0]


def bio_base(s, stage):
    for y in range(3, 21):
        for x in range(3, 21):
            if not bio_plate(x, y):
                continue
            c = 'S2'
            if not bio_plate(x - 1, y) or not bio_plate(x, y - 1):
                c = 'S1'
            if not bio_plate(x + 1, y) or not bio_plate(x, y + 1):
                c = 'S3'
            if x + y <= 11 or x + y >= 35:
                c = 'A3' if ((x - y + 40) // 2) % 2 == 0 else 'A0'
            s.p(x, y, c)
    for x, y in ((12, 4), (4, 12), (19, 12), (12, 19)):
        s.p(x, y, 'SS')
    for k, a in enumerate(CANISTERS):
        cx, cy = 12 + math.sin(a) * 6.6, 12 - math.cos(a) * 6.6
        if stage >= 2 and k == 1:
            s.disc(cx, cy, 2, 'S4')
            s.p(cx + 1, cy + 1, 'Z2', GLOW)
            continue
        s.disc(cx, cy, 2.3, 'SS')
        s.ball(cx, cy, 1.9, ['S1', 'S2', 'S3', 'SS'])
        s.disc(cx, cy, 1.1, 'Z1', GLOW)
    s.disc(12, 12, 3.6, 'SS')


def bio_top(s, damaged):
    a = 0.0
    s.ball(12, 12, 3.1, ['S1', 'S2', 'S3', 'SS'])
    off = -2.0
    reach = 5.6 if damaged else 6.4
    while off <= 2.0:
        c = 'S4' if abs(off) > 1.6 else ('S1' if off < -0.6 else 'S2')
        s.ray(12, 12, a, 0, reach, c, 0, off)
        off += 0.5
    my = 12 - reach + 0.4
    s.disc(12, my, 1.8, 'S4')
    s.disc(12, my, 1.2, 'Z3', GLOW)
    if damaged:
        s.p(13, 9, 'SS')
        s.p(10, 10, 'S4')


def bio_glow(s):
    s.disc(12, 6.0, 1.4, 'Z1', GLOW)
    s.p(11.5, 5.5, 'Z0', GLOW)


def bio_ruin(s):
    for y in range(3, 21):
        for x in range(3, 21):
            if not bio_plate(x, y) or h32(x >> 1, y >> 1, 5) < 0.12:
                continue
            c = 'SS' if h32(x, y, 6) < 0.25 else 'S3'
            if x + y <= 11 or x + y >= 35:
                c = 'A1' if ((x - y + 40) // 2) % 2 == 0 else 'A0'
            s.p(x, y, c)
    s.disc(12, 12, 3.4, 'S4')
    s.disc(12.3, 12.3, 2.2, 'K')
    s.line(4, 9, 8, 11, 'K')
    s.line(16, 16, 19, 14, 'K')
    off = -2.0
    while off <= 2.0:
        s.ray(17, 18, 2.4, 0, 5, 'S4' if abs(off) > 1.6 else 'S3', 0, off)
        off += 0.5
    for x, y in ((5, 5), (19, 6), (4, 18)):
        s.r(x, y, 2, 2, 'S3')
    for x, y in ((10, 13), (13, 11), (8, 16), (15, 8), (6, 12)):
        s.p(x, y, 'Z2', GLOW)


KINDS = [
    ('tesla', tesla_base, tesla_top, tesla_glow, tesla_ruin),
    ('gun', gun_base, gun_top, None, gun_ruin),
    ('bio', bio_base, bio_top, bio_glow, bio_ruin),
]
SIDES = ['player', 'enemy']


def cells():
    """Every (row, col) -> 48 x 48 RGBA rows the sheet holds."""
    out = {}
    for k, (name, base, top, glow, ruin) in enumerate(KINDS):
        for si, side in enumerate(SIDES):
            row = k * 2 + si
            trim = TRIM[side]
            seed = 71 + k * 13
            for stage in range(4):
                s = Spr()
                base(s, stage)
                s.wear(stage, seed)
                s.outline()
                out[(row, stage)] = s.rgba(trim)
            s = Spr()
            ruin(s)
            s.wear(2, 101 + k)
            s.outline()
            out[(row, 4)] = s.rgba(trim)
            for col, damaged in ((5, False), (6, True)):
                s = Spr()
                top(s, damaged)
                if damaged:
                    s.wear(2, seed + 5)
                s.outline()
                out[(row, col)] = s.rgba(trim)
            if glow:
                s = Spr()
                glow(s)
                out[(row, 7)] = s.rgba(trim)
    return out


def png(width, height, pixels):
    raw = b''.join(b'\x00' + b''.join(struct.pack('BBBB', *p) for p in row) for row in pixels)

    def chunk(kind, data):
        body = kind + data
        return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body) & 0xffffffff)

    header = struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', header) + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b'')


def main():
    width, height = COLS * CELL, ROWS * CELL
    pixels = [[(0, 0, 0, 0)] * width for _ in range(height)]
    for (row, col), rgba in cells().items():
        for y in range(CELL):
            pixels[row * CELL + y][col * CELL:(col + 1) * CELL] = rgba[y]
    with open(OUT, 'wb') as f:
        f.write(png(width, height, pixels))
    print(f'wrote {os.path.normpath(OUT)} ({width}x{height})')


if __name__ == '__main__':
    main()
