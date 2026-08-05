#!/usr/bin/env python3
"""Detect how many barrels each tank in static/tanks.png has.

Barrels protrude from the top edge of each 32x32 tile. We scan the topmost
rows of each tile, find opaque columns, cluster adjacent columns into barrels,
and report the count per (row, col).
"""
import struct
import zlib


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
    # de-filter
    stride = width * 4
    out = bytearray()
    prev = bytearray(stride)
    p = 0
    for _ in range(height):
        ft = raw[p]; p += 1
        line = bytearray(raw[p:p + stride]); p += stride
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


W, H, px = load_png("static/tanks.png")
TILE = 32
GRID = 8


def alpha(x, y):
    return px[(y * W + x) * 4 + 3]


def is_barrel(x, y):
    """A barrel pixel: opaque and dark (near-black gun metal)."""
    i = (y * W + x) * 4
    r, g, b, a = px[i], px[i + 1], px[i + 2], px[i + 3]
    return a > 128 and r < 70 and g < 70 and b < 70


def barrel_count(tx, ty):
    """Count barrels as dark vertical protrusions near the top of the tile.

    For each column, check if it has dark 'barrel' pixels in the top band.
    Cluster adjacent barrel columns; each cluster is one barrel. We take the
    band above where the hull body begins so hull outlines don't count.
    """
    ox, oy = tx * TILE, ty * TILE
    band = range(0, 8)  # top 8 px is where barrels stick out
    cols = []
    for x in range(TILE):
        hit = sum(1 for y in band if is_barrel(ox + x, oy + y)) >= 2
        cols.append(hit)
    clusters = 0
    prev = False
    for h in cols:
        if h and not prev:
            clusters += 1
        prev = h
    return clusters


print("barrels per tank (row x col):")
two_plus = []
for ry in range(GRID):
    line = []
    for cx in range(GRID):
        n = barrel_count(cx, ry)
        line.append(str(n))
        if n >= 2:
            two_plus.append((ry, cx, n))
    print(f"row {ry}: " + " ".join(line))

print("\ntanks with >=2 barrels (row, col, count):")
for (r, c, n) in two_plus:
    print(f"  ({r},{c}) -> {n}")
print(f"\ntotal double+ turret tanks: {len(two_plus)} / {GRID*GRID}")
