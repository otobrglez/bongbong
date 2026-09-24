"""Generate the seeker-missile sheets (missile.rs, tank::draw_missile_pod):

- static/missile.png - MISSILE_FRAMES (4) cells of 32x32 in one row: one
  missile pointing up (rotation 0), its exhaust flame below it, the frames
  differing only in the flame's flicker. Designed on a 16x16 grid and
  doubled, the tanks' and shells' density, so a missile on the ground reads
  a little bigger than a shell. The sprite's centre (16, 16) is the draw
  origin; the flame sits behind it, where `Missile::tail` puts the smoke.

- static/missile_pod.png - MISSILE_POD_FRAMES (5) cells of 32x32: the
  four-tube launcher bolted on a turret, laid out and pivoted like
  minigun_mount.png (authored at 1 px per design pixel around the (16, 16)
  turret pivot, drawn at the tank's 2x scale). Two pairs of tubes either
  side of a centre spine; column k shows the leftmost k tubes empty (the
  order a volley leaves in), so the pod visibly empties through a volley and
  refills through the reload. A loaded tube shows its missile's red nose at
  the mouth, an empty one a dark bore.

  The tube columns (9-10, 12-13, 18-19, 21-22) and the mouth row (5) are
  what lib.rs's MISSILE_TUBE_OFFSETS / MISSILE_TUBE_FORWARD point at -
  change them together.

Every colour is an exact punypalette entry (no snap needed, no green), so
`just check-sheets` holds both on the palette.

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


# ---------------------------------------------------------------------------
# The pod, at 1 px per design pixel on a 32 x 32 cell, pivot (16, 16).
# ---------------------------------------------------------------------------
S = 32
TUBES = [(9, 10), (12, 13), (18, 19), (21, 22)]
FRONT, BACK = 5, 20


def pod_frame(empty):
    img = Image.new('RGBA', (S, S), (0, 0, 0, 0))
    p = img.putpixel
    # Mount hub under the spine, like the minigun's.
    for y in range(S):
        for x in range(S):
            if (x - 15.5) ** 2 + (y - 15.5) ** 2 <= 3.5 ** 2:
                p((x, y), c(STONE_DK))
    # Centre spine joining the two boxes.
    for y in range(9, BACK):
        for x in (15, 16):
            p((x, y), c(STONE_DK))
    # The two boxes: casing, then the tubes' tops running along them.
    for x0, x1 in ((8, 14), (17, 23)):
        for y in range(FRONT, BACK + 1):
            for x in range(x0, x1 + 1):
                p((x, y), c(STONE_MD))
        for y in range(FRONT, BACK + 1):
            p((x0, y), c(STONE_LT))
            p((x1, y), c(STONE_DK))
        for x in range(x0, x1 + 1):
            p((x, BACK), c(STONE_DK))
    for i, (a, b) in enumerate(TUBES):
        for y in range(FRONT + 2, BACK):
            p((a, y), c(STONE_LT))
            p((b, y), c(STONE_DK))
        # Mouth: the missile's nose while loaded, a dark bore once fired.
        if i < empty:
            for y in (FRONT, FRONT + 1):
                p((a, y), c(STONE_DARKEST))
                p((b, y), c(BLACK))
        else:
            p((a, FRONT), c(RED_BRIGHT))
            p((b, FRONT), c(RED_BRIGHT))
            p((a, FRONT + 1), c(RED_MD))
            p((b, FRONT + 1), c(RED_DEEP))
            p((a, FRONT + 2), c(RED_DK))
            p((b, FRONT + 2), c(RED_DK))
    # Grooves between the tubes of a pair.
    for x in (11, 20):
        for y in range(FRONT, BACK):
            p((x, y), c(STONE_DK))
    outline(img, S)
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

    pod = Image.new('RGBA', (S * 5, S), (0, 0, 0, 0))
    for k in range(5):
        pod.paste(pod_frame(k), (S * k, 0))
    pod.save(os.path.join(OUT, 'missile_pod.png'))
    print(f'wrote {OUT}/missile.png and {OUT}/missile_pod.png')


if __name__ == '__main__':
    main()
