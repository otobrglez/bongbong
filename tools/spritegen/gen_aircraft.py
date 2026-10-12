"""Air support's aircraft: static/aircraft_sheet.png (docs/AIRCRAFT_SPEC.md,
docs/air-support.md).

Straight down, nose up (rotation 0 faces up, as every sprite), lit from the
top left, the tanks' #252525 outline, on the 2 px block grid: 44 x 44 design
pixels a cell, 88 px - ten per cent over a tank's 80 px sprite cell. The
Puny palette only, and no green - they fly over the grass: a desert-tan
gunship and a grey attack jet.

aircraft_sheet.png, 4 columns x 1 row of 88 px:

    col 0  the helicopter's airframe: cabin, stub wings with their rocket
           pods, tail boom, stabiliser and tail rotor, no main rotor
    col 1  its main rotor, the blades on the axes, centred in the cell -
           drawn over col 0 with its centre on the mast, `MAST_Y` ahead
    col 2  its main rotor a sixteenth of a turn on (the two alternate)
    col 3  the jet: nose gun, canopy, straight wings, the two engines on
           the tail and the twin fins

No Pillow: raw PNG bytes, the crate generator's convention. Deterministic:
nothing is random.

    python3 tools/spritegen/gen_aircraft.py
"""

import math
import os
import struct
import sys
import zlib

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
import punypalette as pp  # noqa: E402

STATIC = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'static')
GRID = 44           # design pixels per cell side
CELL = GRID * 2     # sheet pixels per cell side
COLS = 4

# Each airframe's ramp, lit side to shaded side.
TAN = (pp.SAND_PALE, pp.SAND_MD, pp.SAND_DK, pp.WOOD_DARKEST)
GREY = (pp.STONE_HI, pp.STONE_MID, pp.STONE_MDK, pp.STONE_SHADE)
CANOPY = (pp.BLUE_PALE, pp.BLUE_LT, pp.BLUE_DK, pp.BLUE_DARKEST)
METAL = (pp.STONE_MID, pp.STONE_MDK, pp.STONE_SHADE, pp.STONE_DARKEST)
BLADE = (pp.STONE_SHADE, pp.STONE_SHADE, pp.STONE_DARKEST, pp.STONE_DARKEST)


class Shape:
    """A design-pixel mask on the cell's grid, each pixel a ramp."""

    def __init__(self):
        self.px = {}

    def rect(self, x0, y0, x1, y1, ramp):
        """Fill the design pixels x0..=x1, y0..=y1."""
        for y in range(y0, y1 + 1):
            for x in range(x0, x1 + 1):
                self.px[(x, y)] = ramp

    def mirror(self, x0, y0, x1, y1, ramp):
        """`rect` and its mirror across the cell's centre line."""
        self.rect(x0, y0, x1, y1, ramp)
        self.rect(GRID - 1 - x1, y0, GRID - 1 - x0, y1, ramp)

    def disc(self, cx, cy, r, ramp):
        for y in range(GRID):
            for x in range(GRID):
                if (x + 0.5 - cx) ** 2 + (y + 0.5 - cy) ** 2 <= r * r:
                    self.px[(x, y)] = ramp

    def line(self, x0, y0, x1, y1, width, ramp):
        """A bar `width` design pixels wide from (x0, y0) to (x1, y1)."""
        steps = int(max(abs(x1 - x0), abs(y1 - y0)) * 2) + 1
        for i in range(steps + 1):
            t = i / steps
            cx, cy = x0 + (x1 - x0) * t, y0 + (y1 - y0) * t
            for dy in range(-width, width + 1):
                for dx in range(-width, width + 1):
                    x, y = int(math.floor(cx + dx * 0.5)), int(math.floor(cy + dy * 0.5))
                    if 0 <= x < GRID and 0 <= y < GRID and (dx * dx + dy * dy) * 0.25 <= (width * 0.5) ** 2:
                        self.px[(x, y)] = ramp

    def paint(self, lamps=()):
        """The design pixels as colours: each pixel its ramp's base, the
        lit edge (open above or to the left) one step up, the shaded edge
        (open below or to the right) one down, the outline round it all;
        `lamps` are (x, y, colour) painted last."""
        out = {}
        for (x, y), ramp in self.px.items():
            above = self.px.get((x, y - 1)) is ramp and self.px.get((x - 1, y)) is ramp
            below = self.px.get((x, y + 1)) is ramp and self.px.get((x + 1, y)) is ramp
            if not above:
                out[(x, y)] = ramp[0]
            elif not below:
                out[(x, y)] = ramp[2]
            else:
                out[(x, y)] = ramp[1]
        for (x, y) in list(self.px):
            for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                p = (x + dx, y + dy)
                if p not in self.px and 0 <= p[0] < GRID and 0 <= p[1] < GRID:
                    out[p] = pp.BLACK
        for x, y, c in lamps:
            out[(x, y)] = c
        return out


def heli():
    s = Shape()
    # Tail boom, its stabiliser and the tail rotor's housing (right side).
    s.rect(21, 26, 22, 40, TAN)
    s.mirror(16, 36, 20, 37, TAN)
    s.rect(23, 37, 24, 42, METAL)
    # Stub wings and the rocket pods at their tips.
    s.mirror(10, 15, 18, 17, TAN)
    s.mirror(7, 12, 10, 20, METAL)
    # The cabin: a long narrow body, the engines' bulge at its waist.
    s.rect(19, 4, 24, 27, TAN)
    s.mirror(17, 13, 18, 22, TAN)
    # Tandem canopy, nose gun.
    s.rect(20, 6, 23, 12, CANOPY)
    s.rect(21, 1, 22, 3, METAL)
    return s.paint(lamps=[(21, 40, pp.RED_BRIGHT), (22, 40, pp.RED_BRIGHT), (8, 13, pp.GOLD_BRIGHT), (35, 13, pp.GOLD_BRIGHT)])


def rotor(turn):
    """The main rotor, four blades round the mast, `turn` of a quarter
    turn on, centred in its cell: the game puts its centre on the mast,
    `MAST_Y` design pixels from the airframe's centre."""
    s = Shape()
    cx, cy, r = GRID / 2, GRID / 2, 20.5
    for k in range(4):
        a = (k + turn) * math.pi / 2
        s.line(cx, cy, cx + math.cos(a) * r, cy + math.sin(a) * r, 2, BLADE)
    s.disc(cx, cy, 2.0, METAL)
    return s.paint()


# Where the mast stands on the airframe: this many design pixels ahead of
# the cell's centre (`air_support::HELI_MAST_PX` is twice it, in world px).
MAST_Y = 6


def jet():
    s = Shape()
    # Twin fins on the tail plane.
    s.mirror(11, 35, 21, 37, GREY)
    s.mirror(10, 33, 12, 40, GREY)
    # The engines, high on the tail, and their pylons.
    s.mirror(14, 24, 17, 32, METAL)
    s.mirror(18, 26, 19, 28, GREY)
    # Straight wings, tapering to their tips.
    s.mirror(2, 16, 19, 20, GREY)
    s.mirror(0, 17, 1, 19, GREY)
    # The fuselage, nose to tail.
    s.rect(19, 3, 24, 38, GREY)
    s.rect(20, 7, 23, 11, CANOPY)
    s.rect(21, 0, 22, 2, METAL)
    return s.paint(lamps=[(0, 18, pp.RED_BRIGHT), (43, 18, pp.TEAL_BRIGHT), (15, 32, pp.GOLD_BRIGHT), (28, 32, pp.GOLD_BRIGHT)])


def png(width, height, pixels):
    raw = b''.join(b'\x00' + b''.join(struct.pack('BBBB', *p) for p in row) for row in pixels)

    def chunk(kind, data):
        body = kind + data
        return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body) & 0xffffffff)

    header = struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', header) + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b'')


def main():
    width, height = COLS * CELL, CELL
    sheet = [[(0, 0, 0, 0)] * width for _ in range(height)]
    for col, design in enumerate([heli(), rotor(0.0), rotor(0.25), jet()]):
        for (x, y), c in design.items():
            for by in range(2):
                for bx in range(2):
                    sheet[y * 2 + by][col * CELL + x * 2 + bx] = (c[0], c[1], c[2], 255)
    out = os.environ.get('SPRITE_OUT', STATIC)
    path = os.path.join(out, 'aircraft_sheet.png')
    with open(path, 'wb') as f:
        f.write(png(width, height, sheet))
    print(f'wrote {os.path.normpath(path)} ({width}x{height})')


if __name__ == '__main__':
    main()
