#!/usr/bin/env python3
"""Generate static/pickups/heat_shield.png: the heat shield pickup icon
(docs/volcano.md) - "a special shield against the volcano", red on top and
black underneath as the note that asked for it drew it.

A heater shield seen face on: the top half molten red and orange, lit from
the top left, the bottom half black basalt with a grey glint, a gold seam
between the two where the heat meets the stone, and a dark outline. Drawn
from scratch on a 32 x 32 grid - the same raw-PNG-bytes/no-Pillow
convention as the other generated pickups and the same loud,
not-palette-snapped treatment. Regenerate with
`python3 tools/gen_heat_shield_pickup.py`.
"""
import math
import struct
import zlib

DST = "static/pickups/heat_shield.png"
N = 32

OUTLINE = (26, 14, 14)
RED_DK = (156, 53, 39)
RED = (228, 66, 25)
ORANGE = (238, 132, 52)
GOLD = (255, 214, 120)
WHITE = (255, 244, 214)
BLACK = (37, 37, 37)
STONE = (55, 55, 55)
GLINT = (126, 126, 126)


def inside(px, py):
    """The shield: a flat top with rounded corners, straight sides, then
    two arcs meeting in a point at the bottom."""
    left, right, top = 5.0, 27.0, 3.0
    if py < top or px < left or px > right:
        return False
    cx = (left + right) / 2.0
    # Rounded top corners.
    for corner_x in (left + 3.0, right - 3.0):
        if py < top + 3.0 and abs(px - corner_x) > 0 and ((px < left + 3.0 and corner_x == left + 3.0) or (px > right - 3.0 and corner_x == right - 3.0)):
            if math.hypot(px - corner_x, py - (top + 3.0)) > 3.0:
                return False
    if py <= 16.0:
        return True
    # The lower half narrows to a point at y = 29.
    t = (py - 16.0) / 13.0
    if t >= 1.0:
        return False
    half = (right - left) / 2.0 * math.cos(t * math.pi / 2.0) ** 0.8
    return abs(px - cx) <= half


def write_png(path, width, height, data):
    def chunk(tag, payload):
        c = tag + payload
        return struct.pack(">I", len(payload)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    raw = bytearray()
    for y in range(height):
        raw.append(0)
        raw.extend(data[y * width * 4:(y + 1) * width * 4])
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(bytes(raw), 9)))
        f.write(chunk(b"IEND", b""))


def main():
    mask = [[inside(x + 0.5, y + 0.5) for x in range(N)] for y in range(N)]
    buf = bytearray(N * N * 4)

    def put(x, y, rgb):
        i = (y * N + x) * 4
        buf[i], buf[i + 1], buf[i + 2], buf[i + 3] = rgb[0], rgb[1], rgb[2], 255

    seam = 15
    for y in range(N):
        for x in range(N):
            if not mask[y][x]:
                continue
            edge = any(
                not (0 <= x + dx < N and 0 <= y + dy < N and mask[y + dy][x + dx])
                for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1))
            )
            if edge:
                put(x, y, OUTLINE)
                continue
            # The seam: a gold line with a lighter middle, dipping a pixel
            # in the centre like a chevron.
            dip = 1 if 12 <= x <= 19 else 0
            if y == seam + dip:
                put(x, y, GOLD if 9 <= x <= 22 else ORANGE)
                continue
            if y < seam + dip:
                # Molten top, lit from the top left.
                light = (x - 5) + (y - 3)
                if light < 7:
                    put(x, y, ORANGE if light > 3 else GOLD)
                elif light < 22:
                    put(x, y, RED)
                else:
                    put(x, y, RED_DK)
            else:
                # Basalt bottom with a glint down its left side.
                if x in (8, 9) and seam + 3 <= y <= seam + 8:
                    put(x, y, GLINT)
                elif (x + y) % 7 == 0 and y > seam + 2:
                    put(x, y, STONE)
                else:
                    put(x, y, BLACK)
    # A white-hot spark in the molten half.
    for x, y in ((10, 7), (11, 7), (10, 8)):
        put(x, y, WHITE)
    write_png(DST, N, N, buf)


if __name__ == "__main__":
    main()
