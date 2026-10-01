"""Generate the seeker-missile sheet (missile.rs):

- static/missile.png - MISSILE_FRAMES (4) cells of 32x32 in one row: one
  missile pointing up (rotation 0), its exhaust flame below it, the frames
  differing only in the flame's flicker. Designed on a 16x16 grid and
  doubled, the tanks' and shells' density, so a missile on the ground reads
  a little bigger than a shell. The sprite's centre (16, 16) is the draw
  origin; the flame sits behind it, where `Missile::tail` puts the smoke.

The launcher a volley leaves from is a weapon module of the tank art
(tools/spritegen/tankdesign, docs/SPRITESHEET_SPEC.md).

Every colour is an exact punypalette entry (no snap needed, no green), so
`just check-sheets` holds it on the palette.

Regenerate in place with:
  nix-shell -p "python3.withPackages (ps: [ps.pillow])" \\
    --run "SPRITE_OUT=static python3 tools/spritegen/gen_missiles.py"
"""
from PIL import Image
import os, sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from punypalette import (BLACK, WHITE, STONE_PALE, STONE_LT, STONE_MD, STONE_DK, STONE_DARKEST,
                         RED_BRIGHT, RED_MD, RED_DEEP, RED_DK, GOLD_BRIGHT, WOOD_LT)

OUT = os.environ.get('SPRITE_OUT', 'assets/sprites')
os.makedirs(OUT, exist_ok=True)


def c(rgb):
    return rgb + (255,)


OUTLINE = c(BLACK)

# ---------------------------------------------------------------------------
# The missile, on a 16 x 16 design grid, pointing up.
# ---------------------------------------------------------------------------
D = 16
FRAMES = 4

# Body rows 3..10, three columns wide (6, 7, 8): lit left, bright middle,
# shaded right, one red band.
BODY = {6: c(STONE_LT), 7: c(STONE_PALE), 8: c(STONE_MD)}

# The flame's length and core per frame: a flicker, not a cycle anyone reads.
FLAMES = [
    (3, True),
    (4, True),
    (2, False),
    (4, False),
]


def missile_frame(i):
    img = Image.new('RGBA', (D, D), (0, 0, 0, 0))
    p = img.putpixel
    # Nose cone.
    p((7, 1), c(RED_BRIGHT))
    p((6, 2), c(RED_MD))
    p((7, 2), c(RED_BRIGHT))
    p((8, 2), c(RED_DEEP))
    # Body with a band.
    for y in range(3, 11):
        for x, col in BODY.items():
            p((x, y), col)
    for x in (6, 7, 8):
        p((x, 5), c(RED_DK))
    # Fins: swept back at the tail.
    for x, y in ((5, 8), (9, 8), (5, 9), (9, 9), (4, 10), (5, 10), (9, 10), (10, 10)):
        p((x, y), c(RED_DEEP))
    # Nozzle.
    for x in (6, 7, 8):
        p((x, 11), c(STONE_DARKEST))
    outline(img, D)
    # Flame last, over the outline: hot core down the middle, orange edge.
    length, wide = FLAMES[i]
    for k in range(length):
        y = 12 + k
        if y >= D:
            break
        core = c(WHITE) if k == 0 else c(GOLD_BRIGHT)
        p((7, y), core if k < length - 1 else c(WOOD_LT))
        if wide and k < length - 1:
            p((6, y), c(WOOD_LT))
            p((8, y), c(RED_BRIGHT))
    return img


def outline(img, size):
    src = img.copy()
    for y in range(size):
        for x in range(size):
            if src.getpixel((x, y))[3] != 0:
                continue
            for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                nx, ny = x + dx, y + dy
                if 0 <= nx < size and 0 <= ny < size and src.getpixel((nx, ny))[3] != 0:
                    img.putpixel((x, y), OUTLINE)
                    break


def main():
    sheet = Image.new('RGBA', (32 * FRAMES, 32), (0, 0, 0, 0))
    for i in range(FRAMES):
        cell = missile_frame(i).resize((32, 32), Image.NEAREST)
        sheet.paste(cell, (32 * i, 0))
    sheet.save(os.path.join(OUT, 'missile.png'))
    print(f'wrote {OUT}/missile.png')


if __name__ == '__main__':
    main()
