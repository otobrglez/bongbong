#!/usr/bin/env python3
"""Generate static/two_shells.png: a 5-frame 32x32 sheet (160x32) for the
twin-barrel tank's shells -- two projectiles fired at once.

Same 5-state layout / same 32px grid as shells.png so it's a drop-in variant:
  0 explosion  - twin muzzle blast as the shells exit the tank
  1 fired      - two shells just left the barrels, flashes behind them
  2 flying     - the two projectiles mid-air, side by side
  3 bang       - twin impact burst (starting)
  4 bang       - impact burst (expanding / dissipating)

Shells point UP (tips at top) to match tank rotation 0.0 == facing up.
Cooler blue/cyan "energy round" palette so it reads as a different shell type
than the brass/orange single shells.
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


def star(fx, cx, cy, r, rgba):
    """A rough 4-point burst."""
    for d in range(-r, r + 1):
        fpx(fx, cx + d, cy, rgba)  # horizontal
        fpx(fx, cx, cy + d, rgba)  # vertical
    for d in range(-(r // 2), r // 2 + 1):
        fpx(fx, cx + d, cy + d, rgba)
        fpx(fx, cx + d, cy - d, rgba)


# Palette -- cool energy rounds
WHITE = (255, 255, 255, 255)
FLASH = (170, 240, 255, 255)   # bright cyan flash
CYAN = (80, 190, 240, 255)
BLUE = (50, 110, 220, 255)
SMOKE = (110, 120, 140, 220)
BODY = (60, 70, 95, 255)       # shell casing (steel-blue)
BODY_HI = (110, 125, 160, 255)  # highlight
TIP = (150, 220, 255, 255)     # glowing cyan tip

# The twin shells sit at these x centers within the 32px tile.
LEFT_X = 11
RIGHT_X = 21


def draw_shell(fx, cx, cy):
    """A small pointed projectile centered on x=cx, base near y=cy, tip up."""
    rect(fx, cx - 2, cy - 3, cx + 1, cy + 5, BODY)
    rect(fx, cx - 1, cy - 3, cx - 1, cy + 5, BODY_HI)  # vertical highlight
    # glowing tip
    rect(fx, cx - 1, cy - 6, cx, cy - 4, TIP)
    fpx(fx, cx - 2, cy - 4, TIP)
    fpx(fx, cx + 1, cy - 4, TIP)
    fpx(fx, cx - 1, cy - 7, TIP)
    fpx(fx, cx, cy - 7, TIP)


# --- Frame 0: explosion (twin muzzle blast leaving tank) ---
for x in (LEFT_X, RIGHT_X):
    disc(0, x, 16, 7, BLUE)
    disc(0, x, 16, 5, CYAN)
    disc(0, x, 16, 2, WHITE)
    star(0, x, 16, 9, FLASH)

# --- Frame 1: fired (two shells just left, flashes behind them) ---
for x in (LEFT_X, RIGHT_X):
    draw_shell(1, x, 14)
    disc(1, x, 24, 4, CYAN)
    disc(1, x, 24, 2, FLASH)
    fpx(1, x, 25, WHITE)

# --- Frame 2: flying (two clean projectiles mid-air) ---
for x in (LEFT_X, RIGHT_X):
    draw_shell(2, x, 16)
    fpx(2, x, 24, SMOKE)  # tiny motion streak
    fpx(2, x, 26, SMOKE)

# --- Frame 3: bang (twin impact, starting) ---
for x in (LEFT_X, RIGHT_X):
    disc(3, x, 16, 7, BLUE)
    disc(3, x, 16, 5, CYAN)
    disc(3, x, 16, 2, FLASH)
    star(3, x, 16, 9, CYAN)

# --- Frame 4: bang (impact, expanding / smoke) ---
disc(4, 16, 16, 13, SMOKE)  # merged blast cloud
for x in (LEFT_X, RIGHT_X):
    disc(4, x, 16, 7, BLUE)
    disc(4, x, 16, 4, CYAN)
    star(4, x, 16, 10, BLUE)
for (dx, dy) in [(-12, -5), (11, -7), (-10, 9), (13, 6), (0, -13), (-14, 2), (14, 2)]:
    fpx(4, 16 + dx, 16 + dy, SMOKE)


def write_png(path, width, height, data):
    def chunk(tag, payload):
        c = tag + payload
        return struct.pack(">I", len(payload)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    raw = bytearray()
    for y in range(height):
        raw.append(0)  # filter byte
        raw.extend(data[y * width * 4:(y + 1) * width * 4])

    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)  # 8-bit RGBA
    idat = zlib.compress(bytes(raw), 9)
    with open(path, "wb") as f:
        f.write(sig)
        f.write(chunk(b"IHDR", ihdr))
        f.write(chunk(b"IDAT", idat))
        f.write(chunk(b"IEND", b""))


write_png("static/two_shells.png", W, H, buf)
print(f"wrote static/two_shells.png ({W}x{H}, {FRAMES} frames)")
