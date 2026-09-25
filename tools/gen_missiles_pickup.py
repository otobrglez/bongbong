#!/usr/bin/env python3
"""Generate static/pickups/missiles.png: a 32x32 seeker-missiles pickup icon.

Loud/high-contrast on purpose, like the other pickup icons (see
static/pickups/SOURCE.md) rather than the muted punypalette the terrain and
vehicle sheets draw from. Same raw-PNG-bytes, no-Pillow convention as
tools/gen_minigun_pickup.py.

Icon: a volley of four small missiles side by side, staggered as if they
left their tubes a beat apart, each with a red nose, a lime body (the HUD's
`HUD_MISSILES_COLOR`) and a flame under it - the four-tube pod's volley,
distinct from the minigun's sparks and the plasma's orb.
"""
import struct
import zlib

W = H = 32
buf = bytearray(W * H * 4)


def px(x, y, rgba):
    if 0 <= x < W and 0 <= y < H:
        i = (y * W + x) * 4
        buf[i], buf[i + 1], buf[i + 2], buf[i + 3] = rgba


def get_alpha(x, y):
    if 0 <= x < W and 0 <= y < H:
        return buf[(y * W + x) * 4 + 3]
    return 0


DARK = (30, 25, 20, 255)      # outline, for contrast against any background
BODY = (190, 240, 70, 255)    # lime, HUD_MISSILES_COLOR
BODY_HI = (235, 255, 170, 255)
BODY_LO = (120, 170, 30, 255)
NOSE = (255, 60, 40, 255)
FIN = (200, 40, 30, 255)
FLAME = (255, 170, 30, 255)
CORE = (255, 245, 200, 255)

# Column of each missile's left body pixel, and how far down it starts: a
# ripple, the first one highest.
MISSILES = [(4, 3), (11, 7), (18, 4), (25, 8)]
LENGTH = 14  # nose to nozzle


def missile(x0, top):
    # Nose: two rows narrowing to a point.
    px(x0 + 1, top, NOSE)
    for x in range(x0, x0 + 3):
        px(x, top + 1, NOSE)
    # Body: lit left, bright middle, shaded right.
    for y in range(top + 2, top + LENGTH):
        px(x0, y, BODY_HI)
        px(x0 + 1, y, BODY)
        px(x0 + 2, y, BODY_LO)
    # Fins at the tail.
    for y in range(top + LENGTH - 3, top + LENGTH):
        px(x0 - 1, y, FIN)
        px(x0 + 3, y, FIN)


def flame(x0, top):
    y = top + LENGTH
    px(x0 + 1, y, CORE)
    px(x0, y, FLAME)
    px(x0 + 2, y, FLAME)
    px(x0 + 1, y + 1, FLAME)
    px(x0 + 1, y + 2, FLAME)


for x0, top in MISSILES:
    missile(x0, top)

# Dark outline around every missile, for contrast on any ground.
solid = [(x, y) for y in range(H) for x in range(W) if get_alpha(x, y)]
solid_set = set(solid)
for y in range(H):
    for x in range(W):
        if (x, y) in solid_set:
            continue
        if any((x + dx, y + dy) in solid_set for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1))):
            px(x, y, DARK)

# Flames after the outline, so they read as light, not as part of the hull.
for x0, top in MISSILES:
    flame(x0, top)


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


write_png("static/pickups/missiles.png", W, H, buf)
print("wrote static/pickups/missiles.png (32x32)")
