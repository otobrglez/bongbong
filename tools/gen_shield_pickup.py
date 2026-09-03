#!/usr/bin/env python3
"""Generate static/pickups/shield.png: the rainbow-shield pickup icon.

Not drawn from scratch like the other generated pickup icons - it is
static/pickups/health.png (the third-party health pack, see
static/pickups/SOURCE.md) with its red hue swept through the rainbow. The
box, the white cross, the dark outline and every bit of shading stay
exactly as they are; only the hue of the coloured (saturated) pixels is
replaced, diagonally from red at the top-left to violet at the bottom-
right, so the shield reads as "a health pack, but rainbow" - the pickup it
is paired with on the battlefield (`simulation::maybe_spawn_bonus_shield`).

Same raw-PNG-bytes convention as tools/gen_laser_pickup.py & co. (no
Pillow), plus a minimal decoder for the 8-bit RGBA, non-interlaced source.
"""
import colorsys
import struct
import zlib

SRC = "static/pickups/health.png"
DST = "static/pickups/shield.png"

# Pixels at or above this HSV saturation are the box's red and get
# recoloured; anything below (white cross, grey highlights, near-black
# outline) is left untouched.
SATURATION_MIN = 0.35
# Hue sweep across the icon's diagonal, in degrees (0 = red ... 285 =
# violet), so the full rainbow fits inside the box without wrapping back
# to red.
HUE_SPAN = 285.0


def read_png(path):
    d = open(path, "rb").read()
    assert d[:8] == b"\x89PNG\r\n\x1a\n"
    w, h, depth, ctype, _, _, interlace = struct.unpack(">IIBBBBB", d[16:29])
    assert (depth, ctype, interlace) == (8, 6, 0), "expected 8-bit RGBA, non-interlaced"
    idat = bytearray()
    i = 8
    while i < len(d):
        n = struct.unpack(">I", d[i:i + 4])[0]
        tag = d[i + 4:i + 8]
        if tag == b"IDAT":
            idat += d[i + 8:i + 8 + n]
        i += 12 + n
    raw = zlib.decompress(bytes(idat))
    stride = w * 4
    out = bytearray(w * h * 4)
    prev = bytearray(stride)
    pos = 0
    for y in range(h):
        f = raw[pos]
        line = bytearray(raw[pos + 1:pos + 1 + stride])
        pos += 1 + stride
        for x in range(stride):
            a = line[x - 4] if x >= 4 else 0
            b = prev[x]
            c = prev[x - 4] if x >= 4 else 0
            if f == 1:
                line[x] = (line[x] + a) & 0xFF
            elif f == 2:
                line[x] = (line[x] + b) & 0xFF
            elif f == 3:
                line[x] = (line[x] + (a + b) // 2) & 0xFF
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[x] = (line[x] + pred) & 0xFF
        out[y * stride:(y + 1) * stride] = line
        prev = line
    return w, h, out


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


W, H, buf = read_png(SRC)

# Bounding box of the opaque pixels, so the sweep spans the box itself
# rather than the tile's transparent padding.
xs = [x for y in range(H) for x in range(W) if buf[(y * W + x) * 4 + 3] > 0]
ys = [y for y in range(H) for x in range(W) if buf[(y * W + x) * 4 + 3] > 0]
x0, x1, y0, y1 = min(xs), max(xs), min(ys), max(ys)
span = float((x1 - x0) + (y1 - y0)) or 1.0

recoloured = 0
for y in range(H):
    for x in range(W):
        i = (y * W + x) * 4
        r, g, b, a = buf[i], buf[i + 1], buf[i + 2], buf[i + 3]
        if a == 0:
            continue
        h, s, v = colorsys.rgb_to_hsv(r / 255.0, g / 255.0, b / 255.0)
        if s < SATURATION_MIN:
            continue
        hue = ((x - x0) + (y - y0)) / span * HUE_SPAN / 360.0
        nr, ng, nb = colorsys.hsv_to_rgb(hue, s, v)
        buf[i], buf[i + 1], buf[i + 2] = int(round(nr * 255)), int(round(ng * 255)), int(round(nb * 255))
        recoloured += 1

write_png(DST, W, H, buf)
print(f"wrote {DST} ({W}x{H}, {recoloured} pixels recoloured from {SRC})")
