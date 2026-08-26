#!/usr/bin/env python3
"""Generate static/pickups/minigun.png: a 32x32 minigun pickup icon.

Loud/high-contrast on purpose, matching the other pickup icons' aesthetic
(see static/pickups/SOURCE.md) rather than the muted punypalette every
generated terrain/vehicle sheet draws from - a pickup needs to read at a
glance against any terrain behind it. Drawn from scratch, same raw-PNG-
bytes convention as tools/gen_laser_pickup.py (no Pillow dependency, same
as tools/gen_damage.py).

Icon: three small muzzle sparks stacked vertically with short staggered
trailing streaks - encoding "burst of rounds" the way gen_laser_pickup.py's
single spark + one long tapering beam encodes "one continuous beam". Bold
amber-orange, distinct from laser.png's magenta and health/ammo's reds.
"""
import struct
import zlib

W = H = 32
buf = bytearray(W * H * 4)


def px(x, y, rgba):
    if 0 <= x < W and 0 <= y < H:
        i = (y * W + x) * 4
        buf[i], buf[i + 1], buf[i + 2], buf[i + 3] = rgba


DARK = (40, 20, 10, 255)     # outline, for contrast against any background
GLOW = (255, 140, 20, 255)   # amber-orange
CORE = (255, 230, 140, 255)  # pale hot core

CX = 9  # shared muzzle column, three rows stacked around the tile's center
ROWS = (10, 16, 22)  # three muzzle origins - top/middle/bottom barrel
# Staggered streak lengths per row, suggesting three barrels firing a beat
# apart rather than one continuous beam.
STREAK_END = (24, 29, 26)

for cy, streak_end in zip(ROWS, STREAK_END):
    # Muzzle spark: dark outline ring, bright ring, hot core - smaller than
    # gen_laser_pickup.py's single muzzle since three need to fit.
    for y in range(H):
        for x in range(W):
            d = ((x - CX) ** 2 + (y - cy) ** 2) ** 0.5
            if d <= 1.6:
                px(x, y, CORE)
            elif d <= 2.6:
                px(x, y, GLOW)
            elif d <= 3.6:
                px(x, y, DARK)

    # Short trailing streak toward the right edge, tapering at the tip.
    beam_y0, beam_y1 = cy - 1, cy + 1
    for x in range(CX + 3, streak_end):
        for y in range(beam_y0 - 1, beam_y1 + 2):
            if y in (beam_y0 - 1, beam_y1 + 1):
                px(x, y, DARK)
            elif y in (beam_y0, beam_y1):
                px(x, y, GLOW)
            else:
                px(x, y, CORE)
    for i, x in enumerate(range(streak_end, min(streak_end + 3, W))):
        top, bottom = beam_y0 + i, beam_y1 - i
        if top > bottom:
            break
        for y in range(top, bottom + 1):
            px(x, y, GLOW if y in (top, bottom) else CORE)


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


write_png("static/pickups/minigun.png", W, H, buf)
print("wrote static/pickups/minigun.png (32x32)")
