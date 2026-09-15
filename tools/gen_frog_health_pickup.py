#!/usr/bin/env python3
"""Generate static/pickups/frog_health.png: the frog health pack pickup icon.

Not drawn from scratch like most generated pickup icons - it is
static/pickups/health.png (the third-party health pack, see
static/pickups/SOURCE.md) with two edits, the same derive-from-health
trick tools/gen_shield_pickup.py uses:

  1. every saturated (red) pixel is rotated HUE_SHIFT degrees around the
     colour wheel to green, saturation and value untouched, so the box's
     outline, highlights and shading all survive exactly as they are;
  2. the white cross is painted out and a small frog is stamped in its
     place, in the cross's own off-white with the box's darkest tone for
     the eyes.

The point is that it reads as "a health pack, but for the frog" at 32px -
the same box the player already knows, in the frog's colour, carrying the
frog instead of the cross. Deliberately not palette-snapped: pickups are
loud on purpose so they pop against muted terrain.

Same raw-PNG-bytes convention as tools/gen_laser_pickup.py & co. (no
Pillow), plus the minimal 8-bit RGBA decoder gen_shield_pickup.py uses.
"""
import colorsys
import struct
import zlib

SRC = "static/pickups/health.png"
DST = "static/pickups/frog_health.png"

# Pixels at or above this HSV saturation are the box's red and get rotated
# to green; everything below it (the white cross and its grey shadow) is
# the cross, which is painted out and replaced by the frog instead.
SATURATION_MIN = 0.35
# Degrees to rotate the box by. The pack's body sits at hue 3, so 103 puts
# it at 106 - the hue of HUD_FROG_COLOR (120, 220, 90). Applied as a
# rotation rather than a fixed target so the darker outline tones keep
# their slight hue offset from the body.
HUE_SHIFT = 103.0

# The frog, 12x12 design pixels centred in the box (which spans x 6..25,
# y 6..25, so both centres land on 15.5). Top-down, like the frog on the
# battlefield: two eye bumps, splayed front legs, folded hind legs.
#   'X' the frog's body, '.' the box showing through, 'o' an eye.
FROG = [
    "..XX....XX..",
    ".XooX..XooX.",
    ".XXXXXXXXXX.",
    "..XXXXXXXX..",
    ".XXXXXXXXXX.",
    "XXXXXXXXXXXX",
    "XX.XXXXXX.XX",
    "...XXXXXX...",
    "..XXXXXXXX..",
    ".XXXXXXXXXX.",
    "XX..XXXX..XX",
    "XX........XX",
]
FROG_X, FROG_Y = 10, 10

# Sampled straight out of health.png, before the hue rotation: the flat
# interior red the cross sits on, the cross's off-white, and the box's
# darkest outline tone for the eyes. The first and last are rotated with
# everything else; the off-white is not (it is below SATURATION_MIN).
BODY = (182, 60, 53)
FROG_BODY = (234, 219, 201)
EYE = (94, 7, 17)


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

    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(bytes(raw), 9)))
        f.write(chunk(b"IEND", b""))


def rotate(rgb):
    """The same hue rotation every saturated pixel in the box gets."""
    h, s, v = colorsys.rgb_to_hsv(rgb[0] / 255.0, rgb[1] / 255.0, rgb[2] / 255.0)
    r, g, b = colorsys.hsv_to_rgb((h + HUE_SHIFT / 360.0) % 1.0, s, v)
    return int(round(r * 255)), int(round(g * 255)), int(round(b * 255))


W, H, buf = read_png(SRC)


def put(x, y, rgb):
    i = (y * W + x) * 4
    buf[i], buf[i + 1], buf[i + 2], buf[i + 3] = rgb[0], rgb[1], rgb[2], 255


# --- 1. the box: red to green, shading intact ---
rotated = 0
for y in range(H):
    for x in range(W):
        i = (y * W + x) * 4
        if buf[i + 3] == 0:
            continue
        h, s, _ = colorsys.rgb_to_hsv(buf[i] / 255.0, buf[i + 1] / 255.0, buf[i + 2] / 255.0)
        if s < SATURATION_MIN:
            continue
        buf[i], buf[i + 1], buf[i + 2] = rotate((buf[i], buf[i + 1], buf[i + 2]))
        rotated += 1

# --- 2. paint the cross out ---
# Every remaining low-saturation opaque pixel is the cross or its shadow -
# the box itself is saturated everywhere, including its outline.
body = rotate(BODY)
painted = 0
for y in range(H):
    for x in range(W):
        i = (y * W + x) * 4
        if buf[i + 3] == 0:
            continue
        _, s, _ = colorsys.rgb_to_hsv(buf[i] / 255.0, buf[i + 1] / 255.0, buf[i + 2] / 255.0)
        if s >= SATURATION_MIN:
            continue
        put(x, y, body)
        painted += 1

# --- 3. stamp the frog ---
eye = rotate(EYE)
for row, line in enumerate(FROG):
    for col, ch in enumerate(line):
        if ch == ".":
            continue
        put(FROG_X + col, FROG_Y + row, FROG_BODY if ch == "X" else eye)

write_png(DST, W, H, buf)
print(f"wrote {DST} ({W}x{H}, {rotated} box pixels rotated, {painted} cross pixels replaced by the frog)")
