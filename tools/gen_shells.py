#!/usr/bin/env python3
"""Generate static/shells.png: a 5-frame 32x32 shell sprite sheet (160x32).

Frames (indexed by column, matching the tanks.png 32px grid):
  0 explosion  - big muzzle blast as the shell exits the tank
  1 fired      - shell just left the barrel, small flash trailing behind it
  2 flying     - the shell projectile mid-air
  3 bang       - impact burst (starting)
  4 bang       - impact burst (expanding / dissipating)

Shells point UP (tip at top) to match tank rotation 0.0 == facing up.
Simple flat pixel-art palette, in the spirit of tanks.png.
"""
import struct
import zlib

TILE = 32
FRAMES = 5
W = TILE * FRAMES
H = TILE

# RGBA canvas, transparent.
buf = bytearray(W * H * 4)


def px(x, y, rgba):
    if 0 <= x < W and 0 <= y < H:
        i = (y * W + x) * 4
        buf[i], buf[i + 1], buf[i + 2], buf[i + 3] = rgba


def fpx(fx, x, y, rgba):
    """Set a pixel using frame-local coords (x,y within the 32x32 tile)."""
    px(fx * TILE + x, y, rgba)


def rect(fx, x0, y0, x1, y1, rgba):
    """Filled rect in frame `fx`, coords local to the 32x32 tile."""
    ox = fx * TILE
    for y in range(y0, y1 + 1):
        for x in range(x0, x1 + 1):
            px(ox + x, y, rgba)


def disc(fx, cx, cy, r, rgba):
    """Filled circle in frame `fx`, local coords, center (cx,cy)."""
    ox = fx * TILE
    for y in range(cy - r, cy + r + 1):
        for x in range(cx - r, cx + r + 1):
            if (x - cx) ** 2 + (y - cy) ** 2 <= r * r:
                px(ox + x, y, rgba)


# Palette
WHITE = (255, 255, 255, 255)
FLASH = (255, 236, 120, 255)   # bright yellow
ORANGE = (255, 150, 40, 255)
RED = (220, 70, 30, 255)
SMOKE = (120, 120, 128, 220)
BODY = (70, 72, 82, 255)       # shell steel
BODY_HI = (120, 122, 132, 255)  # highlight
TIP = (210, 180, 70, 255)      # brass tip


def draw_shell(fx, cy):
    """A small pointed projectile centered on x=16, base near y=cy, tip up."""
    # body (rounded-ish rectangle)
    rect(fx, 14, cy - 3, 17, cy + 5, BODY)
    rect(fx, 15, cy - 3, 15, cy + 5, BODY_HI)  # vertical highlight
    # brass tip
    rect(fx, 15, cy - 6, 16, cy - 4, TIP)
    fpx(fx, 14, cy - 4, TIP)
    fpx(fx, 17, cy - 4, TIP)
    fpx(fx, 15, cy - 7, TIP)
    fpx(fx, 16, cy - 7, TIP)


def star(fx, cx, cy, r, rgba):
    """A rough 4-point burst."""
    for d in range(-r, r + 1):
        fpx(fx, cx + d, cy, rgba)  # horizontal
        fpx(fx, cx, cy + d, rgba)  # vertical
    # diagonals, shorter
    for d in range(-(r // 2), r // 2 + 1):
        fpx(fx, cx + d, cy + d, rgba)
        fpx(fx, cx + d, cy - d, rgba)


# --- Frame 0: explosion (muzzle blast leaving tank) ---
disc(0, 16, 16, 10, ORANGE)
disc(0, 16, 16, 7, FLASH)
disc(0, 16, 16, 3, WHITE)
star(0, 16, 16, 14, FLASH)

# --- Frame 1: fired (shell just left, small flash behind/below it) ---
draw_shell(1, 14)
disc(1, 16, 24, 5, ORANGE)
disc(1, 16, 24, 3, FLASH)
disc(1, 16, 25, 1, WHITE)

# --- Frame 2: flying (clean projectile mid-air) ---
draw_shell(2, 16)
# tiny motion streak trailing below
fpx(2, 16, 24, SMOKE)
fpx(2, 16, 26, SMOKE)

# --- Frame 3: bang (impact, starting) ---
disc(3, 16, 16, 8, RED)
disc(3, 16, 16, 6, ORANGE)
disc(3, 16, 16, 3, FLASH)
star(3, 16, 16, 11, ORANGE)

# --- Frame 4: bang (impact, expanding / smoke) ---
disc(4, 16, 16, 12, SMOKE)
disc(4, 16, 16, 8, RED)
disc(4, 16, 16, 4, ORANGE)
star(4, 16, 16, 14, RED)
# a few scattered debris pixels
for (dx, dy) in [(-11, -6), (10, -8), (-9, 9), (12, 7), (0, -13), (-13, 2)]:
    fpx(4, 16 + dx, 16 + dy, SMOKE)


def write_png(path, width, height, data):
    def chunk(tag, payload):
        c = tag + payload
        return struct.pack(">I", len(payload)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    # add filter byte (0) per scanline
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


write_png("static/shells.png", W, H, buf)
print(f"wrote static/shells.png ({W}x{H}, {FRAMES} frames)")
