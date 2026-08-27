#!/usr/bin/env python3
"""Generate static/pickups/speedup.png: a 32x32 speed-up pickup icon.

Loud/high-contrast on purpose, matching the other pickup icons' aesthetic
(see static/pickups/SOURCE.md) rather than the muted punypalette every
generated terrain/vehicle sheet draws from - a pickup needs to read at a
glance against any terrain behind it. Drawn from scratch, same raw-PNG-
bytes convention as tools/gen_laser_pickup.py/gen_minigun_pickup.py/
gen_plasma_pickup.py (no Pillow dependency, same as tools/gen_damage.py).

Icon: a filled lightning bolt (electric yellow, dark outline, white-hot
core down the middle - the same "outline ring / bright fill / white core"
grammar the other three pickup icons use, just built from a pointed
zigzag polygon instead of a circular muzzle/orb) - the universal "speed
boost" symbol, distinct from laser's beam/minigun's sparks/plasma's orb.
The outline polygon is the classic Feather-icon "zap" silhouette (points
13,2 3,14 12,14 11,22 21,10 12,10 in a 24x24 viewBox), scaled/centered
into this 32x32 tile rather than copied pixel-for-pixel.
"""
import struct
import zlib

W = H = 32
buf = bytearray(W * H * 4)


def px(x, y, rgba):
    if 0 <= x < W and 0 <= y < H:
        i = (y * W + x) * 4
        buf[i], buf[i + 1], buf[i + 2], buf[i + 3] = rgba


DARK = (55, 40, 5, 255)      # outline, for contrast against any background
GLOW = (255, 210, 30, 255)   # electric yellow
CORE = (255, 250, 220, 255)  # near-white hot core

# Classic bolt silhouette (Feather icons' "zap"), scaled from its 24x24
# viewBox up to this tile with a small margin.
_RAW = [(13, 2), (3, 14), (12, 14), (11, 22), (21, 10), (12, 10)]
_SCALE = 1.3
_MIN_X = min(x for x, _ in _RAW)
_MIN_Y = min(y for _, y in _RAW)
_SCALED = [((x - _MIN_X) * _SCALE, (y - _MIN_Y) * _SCALE) for x, y in _RAW]
_MARGIN_X = (W - max(x for x, _ in _SCALED)) / 2
_MARGIN_Y = (H - max(y for _, y in _SCALED)) / 2
BOLT = [(x + _MARGIN_X, y + _MARGIN_Y) for x, y in _SCALED]


def centroid(points):
    cx = sum(x for x, _ in points) / len(points)
    cy = sum(y for _, y in points) / len(points)
    return cx, cy


def scale_points(points, factor):
    cx, cy = centroid(points)
    return [(cx + (x - cx) * factor, cy + (y - cy) * factor) for x, y in points]


def fill_polygon(points, rgba):
    """Even-odd ray-casting fill, sampling each pixel's center - fine for a
    small icon like this, no need for edge antialiasing."""
    for y in range(H):
        yc = y + 0.5
        for x in range(W):
            xc = x + 0.5
            inside = False
            n = len(points)
            for i in range(n):
                x0, y0 = points[i]
                x1, y1 = points[(i + 1) % n]
                if (y0 > yc) != (y1 > yc):
                    x_at = x0 + (yc - y0) * (x1 - x0) / (y1 - y0)
                    if xc < x_at:
                        inside = not inside
            if inside:
                px(x, y, rgba)


# Outline ring, bright bolt, hot core - same layering idea as the other
# pickup icons' "dark rim / bright glow / white core" circles, just traced
# along the bolt's zigzag silhouette instead.
fill_polygon(scale_points(BOLT, 1.18), DARK)
fill_polygon(BOLT, GLOW)
fill_polygon(scale_points(BOLT, 0.45), CORE)


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


write_png("static/pickups/speedup.png", W, H, buf)
print("wrote static/pickups/speedup.png (32x32)")
