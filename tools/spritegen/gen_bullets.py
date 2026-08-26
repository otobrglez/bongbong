"""Generate static/minigun_bullets.png: the minigun's individual-bullet
sprite sheet (see bullet::BulletState, docs/BULLETS_SPEC.md).

3 columns x 1 row of 32x32 cells (96x32 total) - deliberately NOT another
set of rows appended to shells.png: SHELLS_SPEC.md documents "the column
layout is untouched, all seven ShellState columns keep their meaning" as a
hard invariant of that sheet, and a bullet's 3-state machine (Muzzle,
Flying, Hit - see bullet.rs) doesn't fit inside it. One shared row - no
chassis-colour variants like shells.png's 18 rows - every bullet looks the
same regardless of which tank fired it (bullet.rs's own doc comment).

Reuses gen_shells.py's drawing primitives (copied in here rather than
imported - gen_shells.py doesn't import from gen_tanks.py either, only
punypalette is shared across generator scripts), tuned smaller/faster/more
tracer-like: a small muzzle spark, a projectile() tracer roughly a third
the size of shells.png's own 'std' class plus a short motion streak
standing in for a shell's smoke trail, and a quick small impact spark
(not a lingering multi-stage smoke cloud like Shell's Hit0/Hit1/Hit2).

Regenerate in place with:
  nix-shell -p "python3.withPackages (ps: [ps.pillow])" \\
    --run "SPRITE_OUT=static python3 tools/spritegen/gen_bullets.py"
"""
from PIL import Image
import os, sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from punypalette import STONE_DK, STONE_LT, WOOD_LT

S = 32
OUT = os.environ.get('SPRITE_OUT', 'assets/sprites')
os.makedirs(OUT, exist_ok=True)

CASE_DK = STONE_DK + (255,)
CASE_HI = STONE_LT + (255,)
# Same warm bronze-gold family as shells.png's NOSE tone - a "hot metal"
# kinship with shells without being identical.
TRACER = WOOD_LT + (255,)
WHITE = (255, 255, 255, 255)

CX = CY = 15.5


def blank():
    return Image.new('RGBA', (S, S), (0, 0, 0, 0))


def put(img, x, y, c):
    if 0 <= x < S and 0 <= y < S and c is not None:
        img.putpixel((int(x), int(y)), c)


def disc(img, cx, cy, r, col):
    r2 = r * r
    for y in range(S):
        for x in range(S):
            dx, dy = x - cx, y - cy
            if dx * dx + dy * dy <= r2:
                put(img, x, y, col)


def rays(img, cx, cy, length, col):
    for i in range(int(round(length)) + 1):
        put(img, cx + 0.5, cy - i, col)
        put(img, cx + 0.5, cy + i, col)
        put(img, cx - i, cy + 0.5, col)
        put(img, cx + i, cy + 0.5, col)


def projectile(img, cx, top, nose_h, body_h, w):
    """Bullet pointing up: bright tracer nose tapering into a dark casing -
    same shape/logic as gen_shells.py's projectile(), just a much smaller
    nose_h/body_h/w so it reads as a light, fast round rather than a shell."""
    x0 = int(cx + 0.5 - w / 2.0)
    x1 = x0 + w - 1
    for i in range(nose_h):
        if i < nose_h - 1:
            for x in range(x0 + 1, x1):
                put(img, x, top + i, TRACER)
        else:
            for x in range(x0, x1 + 1):
                put(img, x, top + i, TRACER)
    for j in range(body_h):
        y = top + nose_h + j
        for x in range(x0, x1 + 1):
            put(img, x, y, CASE_HI if x == x0 + 1 else CASE_DK)


def frame_muzzle():
    f = blank()
    disc(f, CX, CY, 2.2, CASE_DK)
    disc(f, CX, CY, 1.4, TRACER)
    disc(f, CX, CY, 0.6, WHITE)
    rays(f, CX, CY, 3.0, TRACER)
    return f


def frame_flying():
    f = blank()
    # A third the size of shells.png's own 'std' class (nose=3, body=8, pw=4).
    projectile(f, CX, 12, 1, 3, 2)
    # Short trailing motion streak, standing in for a shell's smoke trail.
    put(f, CX - 0.5, 18, TRACER)
    put(f, CX - 0.5, 19, CASE_HI)
    return f


def frame_hit():
    f = blank()
    disc(f, CX, CY, 3.0, CASE_DK)
    disc(f, CX, CY, 2.0, TRACER)
    disc(f, CX, CY, 0.9, WHITE)
    rays(f, CX, CY, 2.0, TRACER)
    return f


COLS = [frame_muzzle(), frame_flying(), frame_hit()]

sheet = Image.new('RGBA', (S * len(COLS), S), (0, 0, 0, 0))
for c, fr in enumerate(COLS):
    sheet.paste(fr, (c * S, 0))

sheet.save(f'{OUT}/minigun_bullets.png')
print('minigun_bullets.png', sheet.size, f'{len(COLS)} cols x 1 row')
