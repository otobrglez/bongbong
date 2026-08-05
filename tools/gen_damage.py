#!/usr/bin/env python3
"""Generate static/damage.png: a 13-frame 32x32 damage-overlay sheet (416x32).

These frames are drawn ON TOP of a tank sprite to show escalating damage, so
the background is transparent and art is concentrated toward the top/center
(never fill the whole 32x32 tile with opaque color).

Animated stages come in A/B (and A/B/C) variants that are deliberately drawn
DIFFERENTLY (puffs shifted/resized, flames reshaped/retimed) so the game can
cycle between them to make the smoke and fire look alive and flickering.

Frames (indexed by column, matching the 32px grid):
  0  dusty          - faint tan dust specks (static)
  1  gray           - scorched/darkened gray patches (static)
  2  small smoke A  - small gray puff rising from center
  3  small smoke B  - variant of 2 (puff shifted/rounder)
  4  more smoke A   - larger, darker smoke column
  5  more smoke B   - variant of 4 (shifted/reshaped)
  6  small fire A   - small flames at base + a little smoke
  7  small fire B   - flame flicker variant of 6
  8  large fire A   - big flames engulfing the hull + smoke
  9  large fire B   - flicker variant of 8
  10 wrecked A      - heavy fire + thick dark smoke (destroyed, still burning)
  11 wrecked B      - variant of 10
  12 wrecked C      - variant of 10/11
  13 dead           - burnt-out hulk: no fire/smoke, charred grey wash + scorch
                      so the player reads the tank as a dead wreck (static)

Simple flat pixel-art palette, in the spirit of tanks.png / shells.png.
"""
import struct
import zlib

TILE = 32
FRAMES = 14
W = TILE * FRAMES
H = TILE

buf = bytearray(W * H * 4)


def px(x, y, rgba):
    if 0 <= x < W and 0 <= y < H:
        i = (y * W + x) * 4
        buf[i], buf[i + 1], buf[i + 2], buf[i + 3] = rgba


def fpx(fx, x, y, rgba):
    px(fx * TILE + x, y, rgba)


def disc(fx, cx, cy, r, rgba):
    ox = fx * TILE
    for y in range(cy - r, cy + r + 1):
        for x in range(cx - r, cx + r + 1):
            if (x - cx) ** 2 + (y - cy) ** 2 <= r * r:
                px(ox + x, y, rgba)


def star(fx, cx, cy, r, rgba):
    for d in range(-r, r + 1):
        fpx(fx, cx + d, cy, rgba)
        fpx(fx, cx, cy + d, rgba)
    for d in range(-(r // 2), r // 2 + 1):
        fpx(fx, cx + d, cy + d, rgba)
        fpx(fx, cx + d, cy - d, rgba)


def scatter(fx, points, rgba):
    for (x, y) in points:
        fpx(fx, x, y, rgba)


# Palette
DUST = (200, 185, 150, 160)
DUST2 = (170, 150, 115, 190)
GRAY = (90, 90, 95, 170)
GRAY2 = (60, 60, 65, 200)
SMOKE_L = (140, 140, 148, 180)
SMOKE = (100, 100, 108, 210)
SMOKE_D = (70, 70, 78, 230)
SMOKE_K = (45, 45, 50, 240)
FIRE_Y = (255, 230, 90, 255)
FIRE_O = (255, 150, 40, 255)
FIRE_R = (215, 65, 30, 255)
WHITE = (255, 255, 255, 255)


# --- Frame 0: dusty (faint tan specks scattered over the hull) ---
scatter(0, [(10, 12), (14, 9), (19, 13), (22, 18), (12, 20), (17, 22),
            (24, 11), (8, 16), (20, 8), (15, 16)], DUST)
scatter(0, [(13, 14), (18, 19), (23, 15), (11, 22)], DUST2)

# --- Frame 1: gray (scorched patches) ---
disc(1, 13, 14, 4, GRAY)
disc(1, 20, 19, 3, GRAY)
scatter(1, [(16, 10), (23, 12), (9, 20), (18, 23), (12, 9)], GRAY2)

# --- Frame 2: small smoke A (a small puff rising from center) ---
disc(2, 16, 12, 3, SMOKE_L)
disc(2, 16, 14, 2, SMOKE)
fpx(2, 15, 8, SMOKE_L)
fpx(2, 17, 7, SMOKE_L)

# --- Frame 3: small smoke B (variant: puff shifted right + rounder) ---
disc(3, 18, 11, 3, SMOKE_L)
disc(3, 17, 14, 2, SMOKE)
disc(3, 18, 9, 2, SMOKE_L)
fpx(3, 20, 6, SMOKE_L)
fpx(3, 16, 7, SMOKE_L)

# --- Frame 4: more smoke A (larger, darker column) ---
disc(4, 16, 15, 4, SMOKE)
disc(4, 15, 10, 4, SMOKE_L)
disc(4, 17, 6, 3, SMOKE_L)
disc(4, 16, 14, 2, SMOKE_D)

# --- Frame 5: more smoke B (variant: column leaning left, reshaped) ---
disc(5, 17, 16, 4, SMOKE)
disc(5, 15, 11, 4, SMOKE_L)
disc(5, 13, 7, 3, SMOKE_L)
disc(5, 17, 15, 2, SMOKE_D)
fpx(5, 12, 4, SMOKE_L)

# --- Frame 6: small fire A (flames at base, smoke above) ---
disc(6, 16, 12, 3, SMOKE_L)
disc(6, 16, 8, 2, SMOKE_L)
disc(6, 16, 19, 3, FIRE_O)
disc(6, 16, 20, 2, FIRE_Y)
fpx(6, 16, 15, FIRE_Y)
fpx(6, 14, 21, FIRE_R)
fpx(6, 18, 21, FIRE_R)

# --- Frame 7: small fire B (flicker: taller/leaning flame, smoke shifted) ---
disc(7, 18, 11, 3, SMOKE_L)
disc(7, 18, 7, 2, SMOKE_L)
disc(7, 15, 19, 3, FIRE_O)
disc(7, 15, 20, 2, FIRE_Y)
fpx(7, 16, 14, FIRE_Y)
fpx(7, 15, 13, FIRE_O)
fpx(7, 13, 21, FIRE_R)
fpx(7, 17, 21, FIRE_R)

# --- Frame 8: large fire A (big flames engulfing the hull) ---
disc(8, 16, 18, 6, FIRE_R)
disc(8, 16, 18, 4, FIRE_O)
disc(8, 16, 19, 2, FIRE_Y)
for (x, y) in [(16, 9), (13, 12), (19, 12), (16, 11), (11, 16), (21, 16)]:
    fpx(8, x, y, FIRE_O)
disc(8, 15, 8, 3, SMOKE)
disc(8, 18, 6, 2, SMOKE_L)

# --- Frame 9: large fire B (flicker: flames reshaped, tongues to the side) ---
disc(9, 16, 18, 6, FIRE_R)
disc(9, 17, 17, 4, FIRE_O)
disc(9, 16, 18, 2, FIRE_Y)
for (x, y) in [(14, 10), (18, 10), (12, 14), (20, 13), (16, 8), (16, 12)]:
    fpx(9, x, y, FIRE_O)
fpx(9, 16, 7, FIRE_Y)
disc(9, 18, 8, 3, SMOKE)
disc(9, 14, 6, 2, SMOKE_L)

# --- Frame 10: wrecked A (heavy fire + thick dark smoke) ---
disc(10, 16, 20, 7, FIRE_R)
disc(10, 16, 20, 5, FIRE_O)
disc(10, 16, 21, 3, FIRE_Y)
for (x, y) in [(16, 11), (12, 15), (20, 15), (16, 13)]:
    fpx(10, x, y, FIRE_O)
disc(10, 15, 8, 4, SMOKE_K)
disc(10, 18, 5, 3, SMOKE_D)
disc(10, 14, 4, 2, SMOKE)

# --- Frame 11: wrecked B (variant: fire leans, smoke billows right) ---
disc(11, 17, 20, 7, FIRE_R)
disc(11, 15, 19, 4, FIRE_O)
disc(11, 17, 21, 3, FIRE_Y)
for (x, y) in [(14, 12), (19, 13), (16, 10), (21, 16)]:
    fpx(11, x, y, FIRE_O)
disc(11, 18, 8, 4, SMOKE_K)
disc(11, 20, 4, 3, SMOKE_D)
disc(11, 15, 6, 2, SMOKE)

# --- Frame 12: wrecked C (variant: broader fire, smoke billows left) ---
disc(12, 16, 20, 8, FIRE_R)
disc(12, 17, 20, 5, FIRE_O)
disc(12, 15, 21, 3, FIRE_Y)
for (x, y) in [(13, 13), (18, 12), (16, 11), (11, 17), (21, 16)]:
    fpx(12, x, y, FIRE_O)
disc(12, 14, 8, 4, SMOKE_K)
disc(12, 11, 5, 3, SMOKE_D)
disc(12, 18, 5, 2, SMOKE)

# --- Frame 13: dead (burnt-out hulk, no fire/smoke) ---
# A translucent charred wash over the hull greys out the coloured tank beneath
# so it plainly reads as dead, with darker scorch blotches and soot flecks.
CHAR = (35, 33, 33, 165)   # translucent charcoal wash over the whole hull
CHAR_D = (20, 18, 18, 210)  # darker scorch blotches
SOOT = (60, 58, 58, 200)   # grey soot flecks / burnt highlights
for y in range(4, 28):
    for x in range(4, 28):
        # rounded-square hull footprint
        if 5 <= x <= 26 and 5 <= y <= 26:
            fpx(13, x, y, CHAR)
disc(13, 13, 13, 4, CHAR_D)
disc(13, 20, 19, 4, CHAR_D)
disc(13, 17, 9, 3, CHAR_D)
scatter(13, [(9, 20), (23, 11), (11, 10), (22, 22), (16, 16),
             (8, 15), (24, 17), (15, 23), (19, 13)], SOOT)


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


write_png("static/damage.png", W, H, buf)
print(f"wrote static/damage.png ({W}x{H}, {FRAMES} frames)")
