#!/usr/bin/env python3
"""Generate static/pickups/flamethrower.png: a 32x32 flamethrower pickup icon.

Loud/high-contrast on purpose, like laser.png/minigun.png/plasma.png -
a pickup has to read at a glance against any terrain, so it is not
palette-snapped (see static/pickups/SOURCE.md). Drawn from scratch as raw
PNG bytes (no Pillow), the same convention as tools/gen_laser_pickup.py.

Icon: a dark fuel tank on the left with a short nozzle, spitting a tongue
of flame to the right - white-hot at the nozzle, orange, then red at the
tip, so the ramp matches the in-game stream (`fx.rs`'s fire ramp). The
orange is also the HUD's fourth-slot accent (`hud::HUD_FLAME_COLOR`).
"""
import struct
import zlib

W = H = 32
buf = bytearray(W * H * 4)


def px(x, y, rgba):
    if 0 <= x < W and 0 <= y < H:
        i = (y * W + x) * 4
        buf[i], buf[i + 1], buf[i + 2], buf[i + 3] = rgba


DARK = (30, 18, 12, 255)      # outline
TANK = (90, 60, 40, 255)      # the fuel tank's body
TANK_HI = (150, 110, 70, 255) # its highlight
NOZZLE = (110, 115, 120, 255) # steel nozzle
WHITE = (255, 250, 220, 255)  # white-hot root of the flame
ORANGE = (255, 140, 40, 255)  # HUD_FLAME_COLOR
RED = (225, 60, 25, 255)      # the flame's tip

# --- Fuel tank: a rounded drum, 8 wide x 14 tall, on the left. ---
tx0, ty0, tx1, ty1 = 3, 9, 10, 23
for y in range(ty0, ty1 + 1):
    for x in range(tx0, tx1 + 1):
        edge = x in (tx0, tx1) or y in (ty0, ty1)
        corner = (x in (tx0, tx1)) and (y in (ty0, ty1))
        if corner:
            continue
        if edge:
            px(x, y, DARK)
        else:
            px(x, y, TANK_HI if x == tx0 + 2 and ty0 + 2 <= y <= ty1 - 2 else TANK)
# A strap across the drum.
for x in range(tx0 + 1, tx1):
    px(x, 16, DARK)

# --- Nozzle: a short tube from the tank's upper right. ---
for x in range(tx1 + 1, tx1 + 6):
    px(x, 12, DARK)
    px(x, 13, NOZZLE)
    px(x, 14, NOZZLE)
    px(x, 15, DARK)
px(tx1 + 5, 12, DARK)
px(tx1 + 5, 15, DARK)

# --- Flame: widens from the nozzle to the right, licks up at the tip. ---
fx0 = tx1 + 6
rows = {
    # x offset: (top, bottom) of the tongue at that column
    0: (12, 15), 1: (11, 16), 2: (11, 16), 3: (10, 17), 4: (10, 17), 5: (9, 17),
    6: (9, 16), 7: (8, 16), 8: (8, 15), 9: (7, 14), 10: (6, 13), 11: (5, 11), 12: (4, 9), 13: (3, 6),
}
for dx, (top, bottom) in rows.items():
    x = fx0 + dx
    for y in range(top - 1, bottom + 2):
        if y in (top - 1, bottom + 1):
            px(x, y, DARK)
        elif dx < 3:
            px(x, y, WHITE if top + 1 <= y <= bottom - 1 else ORANGE)
        elif dx < 9:
            px(x, y, ORANGE if top + 1 <= y <= bottom - 1 else RED)
        else:
            px(x, y, RED)
# Embers flicking off the tip.
for (x, y) in ((fx0 + 12, 12), (fx0 + 14, 9), (fx0 + 15, 5), (fx0 + 13, 15)):
    px(x, y, ORANGE)
    px(x + 1, y, DARK)


def write_png(path, width, height, data):
    raw = b"".join(b"\x00" + bytes(data[y * width * 4:(y + 1) * width * 4]) for y in range(height))

    def chunk(tag, body):
        c = struct.pack(">I", len(body)) + tag + body
        return c + struct.pack(">I", zlib.crc32(tag + body) & 0xFFFFFFFF)

    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(raw, 9)))
        f.write(chunk(b"IEND", b""))


if __name__ == "__main__":
    write_png("static/pickups/flamethrower.png", W, H, buf)
    print("wrote static/pickups/flamethrower.png")
