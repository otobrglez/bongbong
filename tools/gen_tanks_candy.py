#!/usr/bin/env python3
"""Recolor static/tanks.png into a vivid "candy crush" palette.

Unlike gen_shells.py/gen_damage.py, tanks.png isn't procedurally drawn from
scratch here - it's an existing hand-authored sprite sheet (8x8 grid of 32x32
tanks, only row 0 is currently wired up in-game, see TANK_ROW/TANK_COUNT in
game.rs). This script decodes its real pixels, keeps every shape/shading
exactly as drawn, and remaps hue/saturation per pixel so each of the 8 hull
columns gets its own saturated candy color instead of muted military drab.

Recolor rule, per opaque pixel, in HSV:
  - true black outline (value < 0.05): left untouched, so linework stays crisp
  - shadow (value < 0.20): recolored but kept dark - a rich colored shade
    rather than flat gray/black
  - highlight (value >= 0.86): pushed toward a faint tint of the candy color
    instead of full saturation, so it still reads as a glossy specular shine
  - midtone (everything else): hue replaced with the column's candy hue and
    saturation boosted well past the low saturation of the original drab art

Writes to static/tanks_candy.png - a *separate* file from static/tanks.png,
so this is a preview to look at before deciding whether to swap it in as the
live asset (tank.rs would need its texture load path changed for that).
"""
import colorsys
import struct
import zlib

TILE = 32
GRID = 8

# One candy hue per hull column (degrees), applied to every row that shares
# that column so a given hull shape always reads as the same candy color.
CANDY_HUES_DEG = [
    330,  # col 0: bubblegum pink
    95,   # col 1: lime green
    200,  # col 2: sky blue
    28,   # col 3: tangerine orange
    275,  # col 4: grape purple
    50,   # col 5: lemon yellow
    350,  # col 6: watermelon red
    165,  # col 7: turquoise / mint
]


def load_png(path):
    with open(path, "rb") as f:
        data = f.read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    pos = 8
    width = height = None
    idat = b""
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos:pos + 4])
        tag = data[pos + 4:pos + 8]
        payload = data[pos + 8:pos + 8 + length]
        if tag == b"IHDR":
            width, height, bitd, ctype = struct.unpack(">IIBB", payload[:10])
            assert bitd == 8 and ctype == 6, (bitd, ctype)
        elif tag == b"IDAT":
            idat += payload
        elif tag == b"IEND":
            break
        pos += 12 + length
    raw = zlib.decompress(idat)
    stride = width * 4
    out = bytearray()
    prev = bytearray(stride)
    p = 0
    for _ in range(height):
        ft = raw[p]
        p += 1
        line = bytearray(raw[p:p + stride])
        p += stride
        if ft == 1:  # Sub
            for i in range(4, stride):
                line[i] = (line[i] + line[i - 4]) & 0xFF
        elif ft == 2:  # Up
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 0xFF
        elif ft == 3:  # Average
            for i in range(stride):
                a = line[i - 4] if i >= 4 else 0
                line[i] = (line[i] + ((a + prev[i]) >> 1)) & 0xFF
        elif ft == 4:  # Paeth
            for i in range(stride):
                a = line[i - 4] if i >= 4 else 0
                b = prev[i]
                c = prev[i - 4] if i >= 4 else 0
                pp = a + b - c
                pa, pb, pc = abs(pp - a), abs(pp - b), abs(pp - c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pr) & 0xFF
        out.extend(line)
        prev = line
    return width, height, out


def write_png(path, width, height, data):
    def chunk(tag, payload):
        c = tag + payload
        return struct.pack(">I", len(payload)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    raw = bytearray()
    for y in range(height):
        raw.append(0)
        raw.extend(data[y * width * 4:(y + 1) * width * 4])

    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)  # 8-bit RGBA
    idat = zlib.compress(bytes(raw), 9)
    with open(path, "wb") as f:
        f.write(sig)
        f.write(chunk(b"IHDR", ihdr))
        f.write(chunk(b"IDAT", idat))
        f.write(chunk(b"IEND", b""))


def recolor_pixel(r, g, b, hue_frac):
    h, s, v = colorsys.rgb_to_hsv(r / 255.0, g / 255.0, b / 255.0)

    if v < 0.05:
        # True black outline/linework - leave alone so edges stay crisp.
        return r, g, b
    elif v < 0.16:
        # Shadow: brightened and richly colored rather than flat dark gray -
        # a deep candy shade, not mud.
        new_h, new_s, new_v = hue_frac, min(1.0, s * 0.5 + 0.45), max(v * 1.15, 0.14)
    elif v >= 0.90:
        # Highlight: faint tint of the candy color, not full saturation -
        # keeps it reading as a glossy shine instead of a solid color patch.
        new_h, new_s, new_v = hue_frac, s * 0.30, min(1.0, v * 1.1)
    else:
        # Midtone: this is the color of the tank - push it hard toward a
        # bright, flat, cartoonish candy color.
        new_h, new_s, new_v = hue_frac, min(1.0, 0.75 + s * 0.3), min(1.0, v * 1.3)

    nr, ng, nb = colorsys.hsv_to_rgb(new_h, new_s, new_v)
    return round(nr * 255), round(ng * 255), round(nb * 255)


def main():
    width, height, px = load_png("static/tanks.png")
    out = bytearray(px)

    for ty in range(GRID):
        for tx in range(GRID):
            hue_frac = CANDY_HUES_DEG[tx % len(CANDY_HUES_DEG)] / 360.0
            ox, oy = tx * TILE, ty * TILE
            for y in range(TILE):
                for x in range(TILE):
                    px_x, px_y = ox + x, oy + y
                    i = (px_y * width + px_x) * 4
                    a = px[i + 3]
                    if a == 0:
                        continue
                    r, g, b = px[i], px[i + 1], px[i + 2]
                    nr, ng, nb = recolor_pixel(r, g, b, hue_frac)
                    out[i], out[i + 1], out[i + 2] = nr, ng, nb

    write_png("static/tanks_candy.png", width, height, out)
    print(f"wrote static/tanks_candy.png ({width}x{height}, preview only - tanks.png untouched)")


if __name__ == "__main__":
    main()
