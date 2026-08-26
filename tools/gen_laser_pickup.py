#!/usr/bin/env python3
"""Generate static/pickups/laser.png: a 32x32 laser pickup icon.

Loud/high-contrast on purpose, matching health.png/ammo.png's third-party
pack aesthetic (see static/pickups/SOURCE.md) rather than the muted
punypalette every generated *terrain/vehicle* sheet draws from - a pickup
needs to read at a glance against any terrain behind it, same reasoning
SOURCE.md gives for leaving health/ammo un-recolored. Unlike those two
(copied in from a third-party pack), this one is drawn here from scratch -
there's no laser icon in that pack to copy - as raw PNG bytes (no Pillow
dependency), the same convention tools/gen_damage.py already uses.

Icon: a bright magenta muzzle spark (with a few radiating ticks) firing a
short beam that tapers to a point at the tile's right edge. Deliberately
neither of the two in-game beam colors (see `laser::LaserVariant`) - which
variant a pickup grants is rolled on collection (LASER_BLUE_PICKUP_CHANCE),
so the icon on the ground can't promise one or the other.
"""
import math
import struct
import zlib

W = H = 32
buf = bytearray(W * H * 4)


def px(x, y, rgba):
    if 0 <= x < W and 0 <= y < H:
        i = (y * W + x) * 4
        buf[i], buf[i + 1], buf[i + 2], buf[i + 3] = rgba


DARK = (40, 10, 30, 255)  # outline, for contrast against any background
GLOW = (255, 30, 120, 255)  # magenta beam
CORE = (255, 235, 250, 255)  # near-white hot core
SPARK_RING = (255, 120, 200, 255)

cx, cy = 9, 16  # muzzle/spark origin

# Muzzle spark: dark outline ring, bright ring, white-hot core.
for y in range(H):
    for x in range(W):
        d = math.hypot(x - cx, y - cy)
        if d <= 3.0:
            px(x, y, CORE)
        elif d <= 5.0:
            px(x, y, SPARK_RING)
        elif d <= 6.5:
            px(x, y, DARK)

# Radiating spark ticks around the muzzle.
for ang_deg in (20, 70, 130, 200, 250, 320):
    ang = math.radians(ang_deg)
    for r in range(7, 11):
        x = cx + round(math.cos(ang) * r)
        y = cy + round(math.sin(ang) * r)
        px(x, y, DARK if r == 10 else SPARK_RING)

# Beam: horizontal band from the muzzle toward the right edge, dark outline
# top/bottom, bright core in the middle.
beam_y0, beam_y1 = cy - 2, cy + 2
beam_end = W - 6
for x in range(cx + 5, beam_end):
    for y in range(beam_y0 - 1, beam_y1 + 2):
        if y in (beam_y0 - 1, beam_y1 + 1):
            px(x, y, DARK)
        elif y in (beam_y0, beam_y1):
            px(x, y, GLOW)
        else:
            px(x, y, CORE)

# Tapered tip at the far end, for a "shot" feel.
for i, x in enumerate(range(beam_end, W)):
    shrink = i
    top, bottom = beam_y0 + shrink, beam_y1 - shrink
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


write_png("static/pickups/laser.png", W, H, buf)
print("wrote static/pickups/laser.png (32x32)")
