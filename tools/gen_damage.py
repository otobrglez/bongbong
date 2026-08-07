#!/usr/bin/env python3
"""Generate static/damage.png: a 14-frame 32x32 damage-overlay sheet, laid out
as DAMAGE_VARIANTS row-variants (448x160 = 14 cols x 5 rows of 32x32).

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

Rows (indexed by DAMAGE_VARIANTS, one row per variant): the same 14-frame
recipe above is redrawn per row through a palette swap (hue-shifted dust/
smoke/fire/char colours) and a geometric transform (mirror, offset, scale)
so each row reads as a distinct "flavour" of damage rather than a recolor
with identical silhouettes. A tank rolls one row once at spawn (see
Tank::damage_variant) and keeps it for its whole life, same pattern as
shells.png's row-variants.

Simple flat pixel-art palette, in the spirit of tanks.png / shells.png.
"""
import struct
import zlib

TILE = 32
FRAMES = 14
VARIANTS = 5
W = TILE * FRAMES
H = TILE * VARIANTS

buf = bytearray(W * H * 4)


def px(x, y, rgba):
    if 0 <= x < W and 0 <= y < H:
        i = (y * W + x) * 4
        buf[i], buf[i + 1], buf[i + 2], buf[i + 3] = rgba


def plot(col, row, x, y, rgba):
    px(col * TILE + x, row * TILE + y, rgba)


def _transform(cx, cy, geom):
    mirror, dx, dy, scale = geom
    nx = 16 + (cx - 16) * scale
    ny = 16 + (cy - 16) * scale
    if mirror:
        nx = 31 - nx
    nx += dx
    ny += dy
    return int(round(nx)), int(round(ny))


def tplot(col, row, x, y, rgba, geom):
    nx, ny = _transform(x, y, geom)
    plot(col, row, nx, ny, rgba)


def tdisc(col, row, cx, cy, r, rgba, geom):
    _, _, _, scale = geom
    r2 = max(1, int(round(r * scale)))
    ncx, ncy = _transform(cx, cy, geom)
    for y in range(ncy - r2, ncy + r2 + 1):
        for x in range(ncx - r2, ncx + r2 + 1):
            if (x - ncx) ** 2 + (y - ncy) ** 2 <= r2 * r2:
                plot(col, row, x, y, rgba)


def tscatter(col, row, points, rgba, geom):
    for (x, y) in points:
        tplot(col, row, x, y, rgba, geom)


# --- Per-variant palettes -----------------------------------------------
# Each palette is a dict of the named colours used by the frame recipe below.
PALETTES = [
    # 0: classic warm tan/gray dust, warm orange-yellow fire
    dict(
        DUST=(200, 185, 150, 160), DUST2=(170, 150, 115, 190),
        GRAY=(90, 90, 95, 170), GRAY2=(60, 60, 65, 200),
        SMOKE_L=(140, 140, 148, 180), SMOKE=(100, 100, 108, 210),
        SMOKE_D=(70, 70, 78, 230), SMOKE_K=(45, 45, 50, 240),
        FIRE_Y=(255, 230, 90, 255), FIRE_O=(255, 150, 40, 255), FIRE_R=(215, 65, 30, 255),
        CHAR=(35, 33, 33, 165), CHAR_D=(20, 18, 18, 210), SOOT=(60, 58, 58, 200),
    ),
    # 1: ashen-blue smoke, deep red fire, mirrored silhouette
    dict(
        DUST=(180, 175, 190, 160), DUST2=(150, 145, 170, 190),
        GRAY=(80, 85, 100, 170), GRAY2=(55, 58, 72, 200),
        SMOKE_L=(150, 155, 175, 180), SMOKE=(105, 110, 135, 210),
        SMOKE_D=(70, 75, 100, 230), SMOKE_K=(40, 42, 60, 240),
        FIRE_Y=(255, 200, 110, 255), FIRE_O=(240, 110, 40, 255), FIRE_R=(180, 40, 25, 255),
        CHAR=(30, 32, 40, 165), CHAR_D=(15, 16, 22, 210), SOOT=(55, 58, 70, 200),
    ),
    # 2: rusty-purple ash, bigger/blockier flames
    dict(
        DUST=(210, 170, 190, 160), DUST2=(180, 140, 165, 190),
        GRAY=(100, 80, 100, 170), GRAY2=(70, 55, 75, 200),
        SMOKE_L=(160, 130, 160, 180), SMOKE=(120, 95, 125, 210),
        SMOKE_D=(85, 65, 95, 230), SMOKE_K=(55, 42, 65, 240),
        FIRE_Y=(255, 215, 120, 255), FIRE_O=(255, 120, 60, 255), FIRE_R=(200, 50, 55, 255),
        CHAR=(40, 30, 38, 165), CHAR_D=(22, 16, 20, 210), SOOT=(65, 52, 62, 200),
    ),
    # 3: toxic-green ash, white-hot flame cores, shifted up and shrunk
    dict(
        DUST=(190, 200, 150, 160), DUST2=(160, 175, 120, 190),
        GRAY=(95, 105, 80, 170), GRAY2=(65, 72, 55, 200),
        SMOKE_L=(150, 165, 130, 180), SMOKE=(110, 125, 95, 210),
        SMOKE_D=(78, 90, 65, 230), SMOKE_K=(48, 56, 40, 240),
        FIRE_Y=(255, 250, 200, 255), FIRE_O=(255, 190, 80, 255), FIRE_R=(230, 110, 40, 255),
        CHAR=(32, 38, 26, 165), CHAR_D=(18, 22, 14, 210), SOOT=(58, 64, 48, 200),
    ),
    # 4: sooty-black heavy wreck, mirrored, shifted down and enlarged
    dict(
        DUST=(150, 140, 135, 160), DUST2=(120, 112, 108, 190),
        GRAY=(65, 62, 62, 170), GRAY2=(40, 38, 38, 200),
        SMOKE_L=(110, 108, 112, 180), SMOKE=(75, 74, 80, 210),
        SMOKE_D=(50, 50, 56, 230), SMOKE_K=(28, 28, 32, 240),
        FIRE_Y=(255, 205, 80, 255), FIRE_O=(230, 110, 30, 255), FIRE_R=(160, 35, 20, 255),
        CHAR=(22, 20, 20, 165), CHAR_D=(10, 9, 9, 210), SOOT=(45, 43, 43, 200),
    ),
]

# geom = (mirror: bool, dx: float, dy: float, scale: float)
GEOMS = [
    (False, 0, 0, 1.0),
    (True, 0, 0, 1.0),
    (False, 1, -1, 1.12),
    (False, -1, 1, 0.92),
    (True, 1, 1, 1.05),
]


def build_variant(row, P, geom):
    # --- Frame 0: dusty (faint tan specks scattered over the hull) ---
    tscatter(0, row, [(10, 12), (14, 9), (19, 13), (22, 18), (12, 20), (17, 22),
                       (24, 11), (8, 16), (20, 8), (15, 16)], P["DUST"], geom)
    tscatter(0, row, [(13, 14), (18, 19), (23, 15), (11, 22)], P["DUST2"], geom)

    # --- Frame 1: gray (scorched patches) ---
    tdisc(1, row, 13, 14, 4, P["GRAY"], geom)
    tdisc(1, row, 20, 19, 3, P["GRAY"], geom)
    tscatter(1, row, [(16, 10), (23, 12), (9, 20), (18, 23), (12, 9)], P["GRAY2"], geom)

    # --- Frame 2: small smoke A (a small puff rising from center) ---
    tdisc(2, row, 16, 12, 3, P["SMOKE_L"], geom)
    tdisc(2, row, 16, 14, 2, P["SMOKE"], geom)
    tplot(2, row, 15, 8, P["SMOKE_L"], geom)
    tplot(2, row, 17, 7, P["SMOKE_L"], geom)

    # --- Frame 3: small smoke B (variant: puff shifted right + rounder) ---
    tdisc(3, row, 18, 11, 3, P["SMOKE_L"], geom)
    tdisc(3, row, 17, 14, 2, P["SMOKE"], geom)
    tdisc(3, row, 18, 9, 2, P["SMOKE_L"], geom)
    tplot(3, row, 20, 6, P["SMOKE_L"], geom)
    tplot(3, row, 16, 7, P["SMOKE_L"], geom)

    # --- Frame 4: more smoke A (larger, darker column) ---
    tdisc(4, row, 16, 15, 4, P["SMOKE"], geom)
    tdisc(4, row, 15, 10, 4, P["SMOKE_L"], geom)
    tdisc(4, row, 17, 6, 3, P["SMOKE_L"], geom)
    tdisc(4, row, 16, 14, 2, P["SMOKE_D"], geom)

    # --- Frame 5: more smoke B (variant: column leaning left, reshaped) ---
    tdisc(5, row, 17, 16, 4, P["SMOKE"], geom)
    tdisc(5, row, 15, 11, 4, P["SMOKE_L"], geom)
    tdisc(5, row, 13, 7, 3, P["SMOKE_L"], geom)
    tdisc(5, row, 17, 15, 2, P["SMOKE_D"], geom)
    tplot(5, row, 12, 4, P["SMOKE_L"], geom)

    # --- Frame 6: small fire A (flames at base, smoke above) ---
    tdisc(6, row, 16, 12, 3, P["SMOKE_L"], geom)
    tdisc(6, row, 16, 8, 2, P["SMOKE_L"], geom)
    tdisc(6, row, 16, 19, 3, P["FIRE_O"], geom)
    tdisc(6, row, 16, 20, 2, P["FIRE_Y"], geom)
    tplot(6, row, 16, 15, P["FIRE_Y"], geom)
    tplot(6, row, 14, 21, P["FIRE_R"], geom)
    tplot(6, row, 18, 21, P["FIRE_R"], geom)

    # --- Frame 7: small fire B (flicker: taller/leaning flame, smoke shifted) ---
    tdisc(7, row, 18, 11, 3, P["SMOKE_L"], geom)
    tdisc(7, row, 18, 7, 2, P["SMOKE_L"], geom)
    tdisc(7, row, 15, 19, 3, P["FIRE_O"], geom)
    tdisc(7, row, 15, 20, 2, P["FIRE_Y"], geom)
    tplot(7, row, 16, 14, P["FIRE_Y"], geom)
    tplot(7, row, 15, 13, P["FIRE_O"], geom)
    tplot(7, row, 13, 21, P["FIRE_R"], geom)
    tplot(7, row, 17, 21, P["FIRE_R"], geom)

    # --- Frame 8: large fire A (big flames engulfing the hull) ---
    tdisc(8, row, 16, 18, 6, P["FIRE_R"], geom)
    tdisc(8, row, 16, 18, 4, P["FIRE_O"], geom)
    tdisc(8, row, 16, 19, 2, P["FIRE_Y"], geom)
    for (x, y) in [(16, 9), (13, 12), (19, 12), (16, 11), (11, 16), (21, 16)]:
        tplot(8, row, x, y, P["FIRE_O"], geom)
    tdisc(8, row, 15, 8, 3, P["SMOKE"], geom)
    tdisc(8, row, 18, 6, 2, P["SMOKE_L"], geom)

    # --- Frame 9: large fire B (flicker: flames reshaped, tongues to the side) ---
    tdisc(9, row, 16, 18, 6, P["FIRE_R"], geom)
    tdisc(9, row, 17, 17, 4, P["FIRE_O"], geom)
    tdisc(9, row, 16, 18, 2, P["FIRE_Y"], geom)
    for (x, y) in [(14, 10), (18, 10), (12, 14), (20, 13), (16, 8), (16, 12)]:
        tplot(9, row, x, y, P["FIRE_O"], geom)
    tplot(9, row, 16, 7, P["FIRE_Y"], geom)
    tdisc(9, row, 18, 8, 3, P["SMOKE"], geom)
    tdisc(9, row, 14, 6, 2, P["SMOKE_L"], geom)

    # --- Frame 10: wrecked A (heavy fire + thick dark smoke) ---
    tdisc(10, row, 16, 20, 7, P["FIRE_R"], geom)
    tdisc(10, row, 16, 20, 5, P["FIRE_O"], geom)
    tdisc(10, row, 16, 21, 3, P["FIRE_Y"], geom)
    for (x, y) in [(16, 11), (12, 15), (20, 15), (16, 13)]:
        tplot(10, row, x, y, P["FIRE_O"], geom)
    tdisc(10, row, 15, 8, 4, P["SMOKE_K"], geom)
    tdisc(10, row, 18, 5, 3, P["SMOKE_D"], geom)
    tdisc(10, row, 14, 4, 2, P["SMOKE"], geom)

    # --- Frame 11: wrecked B (variant: fire leans, smoke billows right) ---
    tdisc(11, row, 17, 20, 7, P["FIRE_R"], geom)
    tdisc(11, row, 15, 19, 4, P["FIRE_O"], geom)
    tdisc(11, row, 17, 21, 3, P["FIRE_Y"], geom)
    for (x, y) in [(14, 12), (19, 13), (16, 10), (21, 16)]:
        tplot(11, row, x, y, P["FIRE_O"], geom)
    tdisc(11, row, 18, 8, 4, P["SMOKE_K"], geom)
    tdisc(11, row, 20, 4, 3, P["SMOKE_D"], geom)
    tdisc(11, row, 15, 6, 2, P["SMOKE"], geom)

    # --- Frame 12: wrecked C (variant: broader fire, smoke billows left) ---
    tdisc(12, row, 16, 20, 8, P["FIRE_R"], geom)
    tdisc(12, row, 17, 20, 5, P["FIRE_O"], geom)
    tdisc(12, row, 15, 21, 3, P["FIRE_Y"], geom)
    for (x, y) in [(13, 13), (18, 12), (16, 11), (11, 17), (21, 16)]:
        tplot(12, row, x, y, P["FIRE_O"], geom)
    tdisc(12, row, 14, 8, 4, P["SMOKE_K"], geom)
    tdisc(12, row, 11, 5, 3, P["SMOKE_D"], geom)
    tdisc(12, row, 18, 5, 2, P["SMOKE"], geom)

    # --- Frame 13: dead (burnt-out hulk, no fire/smoke) ---
    # A translucent charred wash over the hull greys out the coloured tank beneath
    # so it plainly reads as dead, with darker scorch blotches and soot flecks.
    for y in range(4, 28):
        for x in range(4, 28):
            if 5 <= x <= 26 and 5 <= y <= 26:
                tplot(13, row, x, y, P["CHAR"], geom)
    tdisc(13, row, 13, 13, 4, P["CHAR_D"], geom)
    tdisc(13, row, 20, 19, 4, P["CHAR_D"], geom)
    tdisc(13, row, 17, 9, 3, P["CHAR_D"], geom)
    tscatter(13, row, [(9, 20), (23, 11), (11, 10), (22, 22), (16, 16),
                        (8, 15), (24, 17), (15, 23), (19, 13)], P["SOOT"], geom)


for i in range(VARIANTS):
    build_variant(i, PALETTES[i], GEOMS[i])


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
print(f"wrote static/damage.png ({W}x{H}, {FRAMES} frames x {VARIANTS} variants)")
