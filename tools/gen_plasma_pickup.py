#!/usr/bin/env python3
"""Generate static/pickups/plasma.png: a 32x32 plasma pickup icon.

Loud/high-contrast on purpose, matching the other pickup icons' aesthetic
(see static/pickups/SOURCE.md) rather than the muted punypalette every
generated terrain/vehicle sheet draws from - a pickup needs to read at a
glance against any terrain behind it. Drawn from scratch, same raw-PNG-
bytes convention as tools/gen_laser_pickup.py/gen_minigun_pickup.py (no
Pillow dependency, same as tools/gen_damage.py).

Icon: a glowing cyan/teal orb (not a beam like laser.png, not muzzle sparks
like minigun.png - the plasma cannon's identity is the bolt itself, an orb,
not a stream) with four short electric arcs radiating outward, echoing
plasma.png's own Flying/Hit frames (tools/spritegen/gen_plasma.py) so the
ground icon and the in-flight bolt read as the same weapon.
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


DARK = (10, 35, 35, 255)      # outline, for contrast against any background
GLOW = (30, 210, 190, 255)    # teal/cyan glow
CORE = (225, 255, 250, 255)   # near-white hot core
ARC = (60, 140, 255, 255)     # electric-blue arc, distinct from the teal glow

cx, cy = 16, 16  # centered orb, unlike laser/minigun's off-center muzzle

# Orb: dark rim, bright ring, white-hot core.
for y in range(H):
    for x in range(W):
        d = math.hypot(x - cx, y - cy)
        if d <= 4.0:
            px(x, y, CORE)
        elif d <= 7.0:
            px(x, y, GLOW)
        elif d <= 8.5:
            px(x, y, DARK)

# Four short jagged arcs radiating outward - two-segment kinked lines, same
# "electronic" read as gen_plasma.py's own bolt() primitive.
for ang_deg in (30, 120, 210, 300):
    a = math.radians(ang_deg)
    dx, dy = math.cos(a), math.sin(a)
    pxn, pyn = -dy, dx
    length = 6.0
    mx, my = cx + dx * length * 0.5 + pxn * 1.4, cy + dy * length * 0.5 + pyn * 1.4
    ex, ey = cx + dx * (length + 4.0), cy + dy * (length + 4.0)
    for (x0, y0, x1, y1) in ((cx + dx * 8.5, cy + dy * 8.5, mx, my), (mx, my, ex, ey)):
        steps = int(max(abs(x1 - x0), abs(y1 - y0))) + 1
        for i in range(steps + 1):
            t = i / steps
            px(round(x0 + (x1 - x0) * t), round(y0 + (y1 - y0) * t), ARC)


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


write_png("static/pickups/plasma.png", W, H, buf)
print("wrote static/pickups/plasma.png (32x32)")
