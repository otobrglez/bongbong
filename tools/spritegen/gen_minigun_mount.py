"""Generate static/minigun_mount.png: the minigun barrel-cluster overlay
drawn on top of a tank's turret while it holds minigun ammo (see
tank::draw_minigun_mount). 3 frames x 1 row of 32x32 cells (96x32 total),
one "hot barrel" per frame, cycled while firing (see
lib.rs's MINIGUN_CYCLE_SECONDS) - NOT a single tile rotated in the screen
plane at runtime.

Why not just rotate the sprite (an earlier version of this script did):
this game is a top-down view, and a real minigun's barrels point ALONG the
ground plane toward the target - their shared rotation axis is horizontal,
lying flat in the exact plane the camera looks straight down at. A top-down
camera is therefore edge-on to that axis, not face-on to it. Spinning the
whole flat cluster sprite within the screen plane is the view you'd get
looking straight down the barrels from the front/back (where the rotor
face-on reads as a pinwheel) - which reads as a helicopter's top rotor, not
a side-mounted minigun, and is the wrong axis entirely for this camera
angle. Cycling which one of the three (already correctly side-on, parallel,
forward-pointing) barrels reads as "hot" fakes the same idea - rounds
cycling through firing position - without ever rotating anything, which is
the part a top-down camera could never actually see happening in the first
place.

Authored around the same (16, 16)-ish pivot every tank turret already uses
(see docs/SPRITESHEET_SPEC.md), with three parallel barrels extending
forward from it - the same "draw the barrel rects from the pivot outward"
approach gen_tanks.py's draw_turret already uses for every chassis's own
gun barrel(s), just repeated three times side by side instead of looping
over a twin-barrel pair (see turret_cols/draw_turret there). Deliberately
NOT a radiating pinwheel - matches the existing multi-barrel convention of
parallel forward-pointing barrels.

Gunmetal ramp is sampled directly from punypalette's STONE_MD/LT/DK/DARKEST
- the exact same ramp gen_tanks.py already uses for every turret's own
barrel(s) (see its Ramp class) - so this overlay reads as "the same metal"
as whatever barrel it's replacing the read of, with deliberately no
chassis body-colour tint: it's bolt-on hardware, not part of the painted
turret. The "hot" barrel's glow uses WOOD_LT, the same warm tone
gen_bullets.py's tracer uses, tying the two visually together.

Regenerate in place with:
  nix-shell -p "python3.withPackages (ps: [ps.pillow])" \\
    --run "SPRITE_OUT=static python3 tools/spritegen/gen_minigun_mount.py"
"""
from PIL import Image
import os, sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from punypalette import BLACK, STONE_DARKEST, STONE_DK, STONE_LT, STONE_MD, STONE_PALE, WOOD_LT

S = 32
OUT = os.environ.get('SPRITE_OUT', 'assets/sprites')
os.makedirs(OUT, exist_ok=True)

OUTLINE = BLACK + (255,)
GM = STONE_MD + (255,)   # gunmetal mid
GL = STONE_LT + (255,)   # gunmetal highlight
GP = STONE_PALE + (255,)  # bright edge on the "hot" barrel
GD = STONE_DK + (255,)   # gunmetal shadow
GDARK = STONE_DARKEST + (255,)  # deep shadow between barrels
GLOW = WOOD_LT + (255,)  # hot-barrel muzzle glow - same family as the bullet tracer

CX = CY = 15.5
HUB_RADIUS = 4.5
BARREL_W = 2
BARREL_GAP = 3  # center-to-center spacing, matching the twin-gun gap range
BARREL_LEN = 13  # a representative middle value of TANK_MUZZLE_FORWARD_OFFSET_BY_ROW


def blank():
    return Image.new('RGBA', (S, S), (0, 0, 0, 0))


def px(img, x, y, c):
    if 0 <= x < S and 0 <= y < S and c is not None:
        img.putpixel((int(x), int(y)), c)


def rect(img, x0, y0, x1, y1, c):
    for y in range(int(y0), int(y1) + 1):
        for x in range(int(x0), int(x1) + 1):
            px(img, x, y, c)


def outline(img):
    src = img.copy()
    for y in range(S):
        for x in range(S):
            if src.getpixel((x, y))[3] != 0:
                continue
            for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                nx, ny = x + dx, y + dy
                if 0 <= nx < S and 0 <= ny < S and src.getpixel((nx, ny))[3] != 0:
                    px(img, x, y, OUTLINE)
                    break


def barrel_cols():
    """Three parallel barrel x-ranges (left, center, right), evenly spaced
    by BARREL_GAP between centers - generalizes turret_cols()'s twin-barrel
    formula in gen_tanks.py from 2 barrels to 3."""
    half = BARREL_W // 2
    cols = []
    for i in (-1, 0, 1):
        center = 16 + i * BARREL_GAP
        cols.append((center - half, center - 1 + half))
    return cols


def draw_mount(hot):
    """`hot`: index (0=left, 1=center, 2=right) of the barrel currently
    drawn as freshly-fired for this frame - see this module's doc comment
    for why cycling this, rather than rotating the sprite, is the correct
    fake for a top-down camera."""
    img = blank()

    # Hub/drum: a disc with the same diagonal highlight/shadow shading
    # gen_tanks.py's draw_turret already applies to every turret body.
    r2 = HUB_RADIUS * HUB_RADIUS
    for y in range(S):
        for x in range(S):
            dx, dy = x - CX, y - CY
            if dx * dx + dy * dy <= r2:
                px(img, x, y, GM)
    for y in range(S):
        for x in range(S):
            if img.getpixel((x, y))[3] == 0:
                continue
            dx, dy = x - CX, y - CY
            if dx + dy > HUB_RADIUS * 0.62:
                px(img, x, y, GD)
            elif dx + dy < -HUB_RADIUS * 0.78:
                px(img, x, y, GL)

    # Thin dark ring at the hub's outer edge, for definition against
    # whatever turret art sits beneath it.
    for y in range(S):
        for x in range(S):
            d = ((x - CX) ** 2 + (y - CY) ** 2) ** 0.5
            if HUB_RADIUS - 0.6 <= d <= HUB_RADIUS + 0.2:
                px(img, x, y, GD)

    # Three parallel barrels, drawn from the pivot outward - same per-barrel
    # shading (mid body / light edge / dark cap) as gen_tanks.py's
    # draw_turret uses for its own barrel rects, except the `hot` one draws
    # brighter/warmer with a small muzzle glow.
    top = max(1, int(CY - BARREL_LEN))
    for i, (a, b) in enumerate(barrel_cols()):
        is_hot = i == hot
        body = GL if is_hot else GM
        edge = GP if is_hot else GL
        rect(img, a, top, b, 17, body)
        rect(img, a, top, a, 17, edge)
        rect(img, a - 1, top, b + 1, top + 2, body)
        rect(img, a - 1, top, a - 1, top + 2, edge)
        rect(img, a, top, b, top, GD)
        rect(img, a - 1, top + 4, b + 1, top + 4, GD)
        if is_hot:
            mid = (a + b) / 2.0
            px(img, mid, top, GLOW)
            px(img, mid, top - 1, GLOW)

    # Deep shadow in the gaps between barrels, right where they meet the
    # hub, so the three read as distinct cylinders rather than one fused
    # block.
    for gap_center in (16 - BARREL_GAP // 2 - 1, 16 + BARREL_GAP // 2):
        rect(img, gap_center, top + 3, gap_center, 16, GDARK)

    outline(img)
    return img


FRAMES = [draw_mount(i) for i in range(3)]

sheet = Image.new('RGBA', (S * len(FRAMES), S), (0, 0, 0, 0))
for c, fr in enumerate(FRAMES):
    sheet.paste(fr, (c * S, 0))

sheet.save(f'{OUT}/minigun_mount.png')
print('minigun_mount.png', sheet.size, f'{len(FRAMES)} cols x 1 row')
