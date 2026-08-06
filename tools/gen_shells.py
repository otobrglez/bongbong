#!/usr/bin/env python3
"""Generate static/shells.png: a 7-frame x 3-variant shell sprite sheet.

32x32 tiles, 7 frames per row, across 3 row-variants (see SHELL_VARIANTS in
lib.rs and Tank::shell_variant):

  col 0  fire 1   - big muzzle blast as the shell exits the tank
  col 1  fire 2   - shell just left the barrel, small flash trailing behind it
  col 2  fire 3   - flash finishing/dissipating as the shell pulls away
  col 3  flying   - the shell projectile mid-air (identical across all rows -
                     the shell looks the same in flight no matter which
                     fire/impact variant it was fired or will land with)
  col 4  hit 1    - impact burst (starting)
  col 5  hit 2    - impact burst (expanding)
  col 6  hit 3    - impact burst (dissipating smoke + embers)

  row 0  classic     - orange/yellow
  row 1  hot/red      - denser, redder blast (heavier round)
  row 2  white-hot    - pale cyan-white flash (hotter-burning propellant)

Each tank is randomly assigned a row (Tank::shell_variant) once per round;
every shell it fires uses that row, so the flying frame - the part the
player sees longest - stays visually consistent per tank while fire/hit
still varies tank-to-tank.

Shells point UP (tip at top) to match tank rotation 0.0 == facing up.
Simple flat pixel-art palette, in the spirit of tanks.png.
"""
import struct
import zlib

TILE = 32
FRAMES = 7
ROWS = 3
W = TILE * FRAMES
H = TILE * ROWS

# RGBA canvas, transparent.
buf = bytearray(W * H * 4)


def px(x, y, rgba):
    if 0 <= x < W and 0 <= y < H:
        i = (y * W + x) * 4
        buf[i], buf[i + 1], buf[i + 2], buf[i + 3] = rgba


def fpx(col, row, x, y, rgba):
    """Set a pixel using frame-local coords (x,y within the 32x32 tile)."""
    px(col * TILE + x, row * TILE + y, rgba)


def rect(col, row, x0, y0, x1, y1, rgba):
    """Filled rect in frame (col,row), coords local to the 32x32 tile."""
    ox, oy = col * TILE, row * TILE
    for y in range(y0, y1 + 1):
        for x in range(x0, x1 + 1):
            px(ox + x, oy + y, rgba)


def disc(col, row, cx, cy, r, rgba):
    """Filled circle in frame (col,row), local coords, center (cx,cy)."""
    ox, oy = col * TILE, row * TILE
    for y in range(cy - r, cy + r + 1):
        for x in range(cx - r, cx + r + 1):
            if (x - cx) ** 2 + (y - cy) ** 2 <= r * r:
                px(ox + x, oy + y, rgba)


def star(col, row, cx, cy, r, rgba):
    """A rough 4-point burst."""
    for d in range(-r, r + 1):
        fpx(col, row, cx + d, cy, rgba)  # horizontal
        fpx(col, row, cx, cy + d, rgba)  # vertical
    for d in range(-(r // 2), r // 2 + 1):
        fpx(col, row, cx + d, cy + d, rgba)
        fpx(col, row, cx + d, cy - d, rgba)


def draw_shell(col, row, cy, body, body_hi, tip):
    """A small pointed projectile centered on x=16, base near y=cy, tip up."""
    rect(col, row, 14, cy - 3, 17, cy + 5, body)
    rect(col, row, 15, cy - 3, 15, cy + 5, body_hi)  # vertical highlight
    rect(col, row, 15, cy - 6, 16, cy - 4, tip)
    fpx(col, row, 14, cy - 4, tip)
    fpx(col, row, 17, cy - 4, tip)
    fpx(col, row, 15, cy - 7, tip)
    fpx(col, row, 16, cy - 7, tip)


def debris(col, row, offsets, rgba):
    for (dx, dy) in offsets:
        fpx(col, row, 16 + dx, 16 + dy, rgba)


WHITE = (255, 255, 255, 255)
BODY = (70, 72, 82, 255)        # shell steel, same across all variants
BODY_HI = (120, 122, 132, 255)
TIP = (210, 180, 70, 255)       # brass tip

DEBRIS_OFFSETS = [(-11, -6), (10, -8), (-9, 9), (12, 7), (0, -13), (-13, 2)]
WIDE_DEBRIS_OFFSETS = [(-15, -8), (14, -10), (-13, 11), (16, 9), (0, -17), (-17, 3), (17, -3), (-4, 16)]

ROW_PALETTES = [
    # row 0: classic orange/yellow (matches shells.png)
    dict(
        flash=(255, 236, 120, 255),
        orange=(255, 150, 40, 255),
        red=(220, 70, 30, 255),
        smoke=(120, 120, 128, 220),
        smoke_fade=(120, 120, 128, 110),
        ember=(255, 150, 40, 200),
    ),
    # row 1: hot/red, denser blast
    dict(
        flash=(255, 200, 110, 255),
        orange=(235, 95, 30, 255),
        red=(170, 35, 20, 255),
        smoke=(90, 70, 66, 230),
        smoke_fade=(90, 70, 66, 120),
        ember=(235, 95, 30, 200),
    ),
    # row 2: white-hot, pale cyan-white flash
    dict(
        flash=(235, 250, 255, 255),
        orange=(150, 220, 255, 255),
        red=(80, 150, 235, 255),
        smoke=(150, 170, 190, 200),
        smoke_fade=(150, 170, 190, 100),
        ember=(150, 220, 255, 200),
    ),
]

for row, pal in enumerate(ROW_PALETTES):
    FLASH, ORANGE, RED = pal["flash"], pal["orange"], pal["red"]
    SMOKE, SMOKE_FADE, EMBER = pal["smoke"], pal["smoke_fade"], pal["ember"]

    # --- col 0: fire 1 (muzzle blast leaving tank) ---
    disc(0, row, 16, 16, 10, ORANGE)
    disc(0, row, 16, 16, 7, FLASH)
    disc(0, row, 16, 16, 3, WHITE)
    star(0, row, 16, 16, 14, FLASH)

    # --- col 1: fire 2 (shell just left, small flash behind/below it) ---
    draw_shell(1, row, 14, BODY, BODY_HI, TIP)
    disc(1, row, 16, 24, 5, ORANGE)
    disc(1, row, 16, 24, 3, FLASH)
    disc(1, row, 16, 25, 1, WHITE)

    # --- col 2: fire 3 (flash finishing, shell already pulling away) [NEW] ---
    draw_shell(2, row, 15, BODY, BODY_HI, TIP)
    disc(2, row, 16, 26, 3, ORANGE)
    disc(2, row, 16, 26, 1, FLASH)

    # --- col 3: flying (identical across every row) ---
    draw_shell(3, row, 16, BODY, BODY_HI, TIP)
    fpx(3, row, 16, 24, (120, 120, 128, 220))
    fpx(3, row, 16, 26, (120, 120, 128, 220))

    # --- col 4: hit 1 (impact, starting) ---
    disc(4, row, 16, 16, 8, RED)
    disc(4, row, 16, 16, 6, ORANGE)
    disc(4, row, 16, 16, 3, FLASH)
    star(4, row, 16, 16, 11, ORANGE)

    # --- col 5: hit 2 (impact, expanding / smoke) ---
    disc(5, row, 16, 16, 12, SMOKE)
    disc(5, row, 16, 16, 8, RED)
    disc(5, row, 16, 16, 4, ORANGE)
    star(5, row, 16, 16, 14, RED)
    debris(5, row, DEBRIS_OFFSETS, SMOKE)

    # --- col 6: hit 3 (impact, dissipating - wider faint smoke, dying embers) [NEW] ---
    disc(6, row, 16, 16, 15, SMOKE_FADE)
    disc(6, row, 16, 16, 6, RED)
    disc(6, row, 16, 16, 2, EMBER)
    debris(6, row, WIDE_DEBRIS_OFFSETS, SMOKE_FADE)


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


write_png("static/shells.png", W, H, buf)
print(f"wrote static/shells.png ({W}x{H}, {FRAMES} frames x {ROWS} rows)")
