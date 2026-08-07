#!/usr/bin/env python3
"""Generate static/obstacles.png: static battlefield terrain sprites.

32x32 tile atlas (96x64 = 3 cols x 2 rows), transparent background, same
tile-atlas convention as tanks.png/shells.png:
  row 0, col 0: Rock  - indestructible terrain (ObstacleKind::Rock). Only
                one frame is used; the other two columns in this row are
                left blank.
  row 1, col 0..2: Crate - destructible terrain (ObstacleKind::Crate),
                    OBSTACLE_CRATE_STAGES health-stage frames from pristine
                    (col 0) through cracked (col 1) to about to break
                    (col 2) - see Obstacle::col in obstacle.rs, which picks
                    the frame from current health / max health.

Simple flat pixel-art palette, in the spirit of tanks.png / shells.png.
"""
import math
import struct
import zlib

TILE = 32
COLS = 3
ROWS = 2
W = TILE * COLS
H = TILE * ROWS

buf = bytearray(W * H * 4)


def px(x, y, rgba):
    if 0 <= x < W and 0 <= y < H:
        i = (y * W + x) * 4
        buf[i], buf[i + 1], buf[i + 2], buf[i + 3] = rgba


def plot(col, row, x, y, rgba):
    px(col * TILE + x, row * TILE + y, rgba)


def disc(col, row, cx, cy, r, rgba):
    for y in range(cy - r, cy + r + 1):
        for x in range(cx - r, cx + r + 1):
            if (x - cx) ** 2 + (y - cy) ** 2 <= r * r:
                plot(col, row, x, y, rgba)


def rect(col, row, x0, y0, x1, y1, rgba):
    for y in range(y0, y1 + 1):
        for x in range(x0, x1 + 1):
            plot(col, row, x, y, rgba)


def line(col, row, x0, y0, x1, y1, rgba):
    # Bresenham, 1px wide - fine at this tile size for a crack/scratch.
    dx, dy = abs(x1 - x0), abs(y1 - y0)
    sx = 1 if x0 < x1 else -1
    sy = 1 if y0 < y1 else -1
    err = dx - dy
    x, y = x0, y0
    while True:
        plot(col, row, x, y, rgba)
        if x == x1 and y == y1:
            break
        e2 = 2 * err
        if e2 > -dy:
            err -= dy
            x += sx
        if e2 < dx:
            err += dx
            y += sy


# --- Rock (row 0, col 0): a single indestructible boulder -----------------
ROCK_OUTLINE = (35, 33, 30, 255)
ROCK_BASE = (120, 115, 108, 255)
ROCK_LIGHT = (150, 145, 136, 255)
ROCK_DARK = (85, 80, 74, 255)
ROCK_SHADE = (60, 57, 52, 255)


def build_rock():
    col, row = 0, 0
    disc(col, row, 16, 17, 12, ROCK_BASE)
    disc(col, row, 12, 13, 7, ROCK_LIGHT)
    disc(col, row, 21, 21, 6, ROCK_DARK)
    # Rough outline ring so the silhouette reads as a boulder, not a circle.
    for a in range(0, 360, 4):
        rad = math.radians(a)
        rx = 16 + round(12.4 * math.cos(rad))
        ry = 18 + round(11.6 * math.sin(rad))
        plot(col, row, rx, ry, ROCK_OUTLINE)
    # Facet cracks/shading flecks.
    for (x, y) in [(14, 12), (18, 11), (22, 16), (10, 18), (13, 22), (20, 24), (16, 19)]:
        plot(col, row, x, y, ROCK_SHADE)


# --- Crate (row 1, cols 0..2): a destructible wooden crate -----------------
CRATE_OUTLINE = (40, 26, 14, 255)
CRATE_WOOD = (150, 105, 60, 255)
CRATE_WOOD_LIGHT = (185, 138, 84, 255)
CRATE_WOOD_DARK = (110, 74, 40, 255)
CRATE_BAND = (90, 60, 32, 255)
CRACK = (25, 16, 10, 255)
SCORCH = (45, 38, 32, 220)


def build_crate_stage(col, chip):
    row = 1
    rect(col, row, 6, 6, 25, 25, CRATE_WOOD)
    # Plank highlights (vertical) + cross-grain shading.
    for x in (8, 14, 20):
        for y in range(7, 25):
            plot(col, row, x, y, CRATE_WOOD_LIGHT)
    for y in (9, 15, 21):
        for x in range(7, 25):
            plot(col, row, x, y, CRATE_WOOD_DARK)
    # Corner/edge bands.
    rect(col, row, 6, 6, 25, 8, CRATE_BAND)
    rect(col, row, 6, 23, 25, 25, CRATE_BAND)
    rect(col, row, 6, 6, 8, 25, CRATE_BAND)
    rect(col, row, 23, 6, 25, 25, CRATE_BAND)
    # Outline.
    for x in range(6, 26):
        plot(col, row, x, 6, CRATE_OUTLINE)
        plot(col, row, x, 25, CRATE_OUTLINE)
    for y in range(6, 26):
        plot(col, row, 6, y, CRATE_OUTLINE)
        plot(col, row, 25, y, CRATE_OUTLINE)

    if chip >= 1:
        line(col, row, 9, 9, 20, 19, CRACK)
        for (x, y) in [(10, 8), (18, 22), (22, 12)]:
            plot(col, row, x, y, SCORCH)
    if chip >= 2:
        line(col, row, 22, 8, 12, 23, CRACK)
        # Chipped corner: punch a small ragged notch out of the top-right.
        for (x, y) in [(24, 7), (25, 7), (24, 8), (23, 7), (25, 8)]:
            plot(col, row, x, y, (0, 0, 0, 0))
        for (x, y) in [(9, 12), (15, 18), (20, 9), (12, 20)]:
            plot(col, row, x, y, SCORCH)


build_rock()
for stage in range(3):
    build_crate_stage(stage, stage)


def write_png(path, width, height, data):
    def chunk(tag, payload):
        c = tag + payload
        return struct.pack(">I", len(payload)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    raw = bytearray()
    for y in range(height):
        raw.append(0)
        raw.extend(data[y * width * 4:(y + 1) * width * 4])

    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    idat = zlib.compress(bytes(raw), 9)
    with open(path, "wb") as f:
        f.write(sig)
        f.write(chunk(b"IHDR", ihdr))
        f.write(chunk(b"IDAT", idat))
        f.write(chunk(b"IEND", b""))


write_png("static/obstacles.png", W, H, buf)
print(f"wrote static/obstacles.png ({W}x{H}, rock + {3}-stage crate)")
