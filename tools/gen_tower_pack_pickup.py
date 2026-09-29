#!/usr/bin/env python3
"""Generate static/pickups/tower_pack.png: the tower pack pickup icon, the
author's R2 "rainbow wrench" pick (docs/defence-towers-prd.md section 12).

Derived from static/pickups/health.png like shield.png: the box's red is
swept through the rainbow on the same diagonal (red top-left to violet
bottom-right), so it reads as one of the rainbow family - but the white
cross is painted out and a spanner is stamped in its place, in the cross's
own off-white with a dark edge, because this pack repairs your towers
rather than your tank. Same raw-PNG-bytes/no-Pillow convention and the same
minimal decoder as the shield; it inherits health.png's provenance and
terms for the same reason.
"""
import colorsys
import math
import struct
import zlib

SRC = "static/pickups/health.png"
DST = "static/pickups/tower_pack.png"

# The box is everything at or above this HSV saturation (the cross and its
# grey shading are below it), and the sweep spans HUE_SPAN degrees - both
# the shield's numbers, so the two rainbows match.
SATURATION_MIN = 0.35
HUE_SPAN = 285.0

# The spanner: off-white (the frog pack's stamp colour), a dark edge, laid
# on a WRENCH x WRENCH grid at ORIGIN.
SPANNER = (234, 219, 201)
EDGE = (60, 32, 40)
WRENCH = 16
ORIGIN = (8, 8)


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


def hue_at(x, y):
    return ((x - x0) + (y - y0)) / span * HUE_SPAN / 360.0


def spanner_mask():
    """The spanner on its own grid: a handle from the bottom-left corner to
    a ring head at the top-right with its jaw open away from the handle."""
    n = WRENCH
    hx, hy, r = n - 5.0, 4.5, 3.6
    mask = [[False] * n for _ in range(n)]
    for y in range(n):
        for x in range(n):
            px, py = x + 0.5, y + 0.5
            # The handle: a 2.6-wide band on the anti-diagonal.
            t = ((px - 2.5) + (hy - py + n - 7.0)) / 2.0
            d = abs((px - 2.5) - (n - 2.5 - py)) / math.sqrt(2.0)
            on_handle = d <= 1.3 and 0.0 <= px - 1.5 and py <= n - 1.5 and px <= hx and py >= hy
            dh = math.hypot(px - hx, py - hy)
            on_head = dh <= r
            # The jaw: a notch cut from the head, opening up and right.
            jaw = (px - hx) + (hy - py) > 0.4 and abs((px - hx) - (hy - py)) < 1.7 * math.sqrt(2.0) and dh < r + 0.5
            mask[y][x] = (on_handle or on_head) and not jaw
    return mask


W, H, buf = read_png(SRC)


def put(x, y, rgb):
    i = (y * W + x) * 4
    buf[i], buf[i + 1], buf[i + 2], buf[i + 3] = rgb[0], rgb[1], rgb[2], 255


xs = [x for y in range(H) for x in range(W) if buf[(y * W + x) * 4 + 3] > 0]
ys = [y for y in range(H) for x in range(W) if buf[(y * W + x) * 4 + 3] > 0]
x0, x1, y0, y1 = min(xs), max(xs), min(ys), max(ys)
span = float((x1 - x0) + (y1 - y0)) or 1.0

# 1. The box through the rainbow; the cross noted for painting out.
box_sv = []
cross = []
for y in range(H):
    for x in range(W):
        i = (y * W + x) * 4
        if buf[i + 3] == 0:
            continue
        h, s, v = colorsys.rgb_to_hsv(buf[i] / 255.0, buf[i + 1] / 255.0, buf[i + 2] / 255.0)
        if s < SATURATION_MIN:
            cross.append((x, y))
            continue
        box_sv.append((s, v))
        nr, ng, nb = colorsys.hsv_to_rgb(hue_at(x, y), s, v)
        buf[i], buf[i + 1], buf[i + 2] = int(round(nr * 255)), int(round(ng * 255)), int(round(nb * 255))

# 2. The cross painted over in the box's most common body tone, at the hue
# the sweep gives that pixel, so the rainbow runs on underneath it.
body_s, body_v = max(set(box_sv), key=box_sv.count)
for x, y in cross:
    r, g, b = colorsys.hsv_to_rgb(hue_at(x, y), body_s, body_v)
    put(x, y, (int(round(r * 255)), int(round(g * 255)), int(round(b * 255))))

# 3. The spanner, edged in dark on its four sides.
mask = spanner_mask()
ox, oy = ORIGIN
for y in range(WRENCH):
    for x in range(WRENCH):
        if mask[y][x]:
            continue
        if any(0 <= y + dy < WRENCH and 0 <= x + dx < WRENCH and mask[y + dy][x + dx]
               for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1))):
            put(ox + x, oy + y, EDGE)
for y in range(WRENCH):
    for x in range(WRENCH):
        if mask[y][x]:
            put(ox + x, oy + y, SPANNER)

write_png(DST, W, H, buf)
print(f"wrote {DST} ({W}x{H}, {len(cross)} cross pixels painted over)")
