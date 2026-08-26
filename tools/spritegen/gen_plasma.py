"""Generate static/plasma.png: the plasma cannon's projectile sprite sheet
(see plasma::PlasmaState/PlasmaVariant, docs/PLASMA_SPEC.md).

10 columns x 2 rows of 32x32 cells (320x64 total): Fire0/1/2 (cols 0-2),
Flying (cols 3-6, a 4-frame baked breathing cycle - see `frame_flying`
below), Hit0/1/2 (cols 7-9). A plasma bolt is fired the exact same way a
shell is (Tank::pending_plasma_shot mirrors Tank::pending_shot's twin-barrel
delay, same fire/impact choreography timings), but Flying gets 4 columns
instead of shells.png's/the original plasma sheet's 1 - a subtle in/out
breathing animation baked into the sprite, layered under
`plasma::draw_plasma`'s own runtime pulse glow rather than replacing it.

Row 0 is the base Teal variant, row 1 is Purple (`plasma::PlasmaVariant`,
rolled on pickup - see PLASMA_PURPLE_PICKUP_CHANCE in lib.rs) - a genuine
second colour pass (`PALETTES` below), not a runtime tint: `draw_texture_pro`'s
tint is a per-channel multiply, which can only ever darken/filter a pixel
toward the tint colour, never invert a channel that started at zero, so
tinting the (zero-red) teal glow body can't actually turn it purple - it was
tried and just produced a darker blue with purple-tinted white highlights,
not a purple bolt. A real second row sidesteps that entirely. Still no
chassis-colour variant rows the way shells.png has 18 (bullet.rs's "every
unit shares one shared piece of art regardless of shooter" convention) -
`PlasmaVariant` is a property of the *ammo*, not the tank firing it.

Reuses gen_shells.py/gen_bullets.py's drawing primitives (copied in rather
than imported - no generator script imports another's primitives, only
punypalette is shared across generator scripts).

Regenerate in place with:
  nix-shell -p "python3.withPackages (ps: [ps.pillow])" \\
    --run "SPRITE_OUT=static python3 tools/spritegen/gen_plasma.py"
"""
import math
import os
import sys
from collections import namedtuple

from PIL import Image

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from punypalette import TEAL_BRIGHT, TEAL_DARKEST, TEAL_LT, TEAL_MD, WHITE

S = 32
OUT = os.environ.get('SPRITE_OUT', 'assets/sprites')
os.makedirs(OUT, exist_ok=True)

CORE = WHITE + (255,)
# White-hot, same as CORE, for both variants - not punypalette's own "BLUE"
# family (BLUE_BRIGHT is actually a teal-cyan sampled from water tiles, see
# punypalette.py's own doc comment, and reads as barely distinguishable from
# TEAL_BRIGHT right next to it). A bright white arc/core against either
# variant's own glow colour is what actually reads as "electric" rather than
# blending into the orb it radiates from - real electric arcs read white-hot
# regardless of the surrounding plasma's own colour, so sharing this one
# constant across both rows is deliberate, not a missed variant hook.
ARC = CORE

# Per-variant colour set a frame is drawn with - `glow`/`glow_md`/`glow_soft`
# are the bright/mid/soft-outer body tones (brightest to softest, matching
# gen_shells.py's own light-to-dark naming), `dark` is the rim/outline tone.
Palette = namedtuple('Palette', ['glow', 'glow_md', 'glow_soft', 'dark'])

TEAL = Palette(
    glow=TEAL_BRIGHT + (255,),
    glow_md=TEAL_MD + (255,),
    glow_soft=TEAL_LT + (150,),
    dark=TEAL_DARKEST + (255,),
)
# punypalette has no true purple/violet family (see punypalette.py's own doc
# comment - Puny World's source art doesn't have one either, which is why
# gen_tanks.py's "wraith" roster reassigned that role to a different hue
# instead of inventing one). There's no other hue family to redirect a
# *second weapon variant* to without it reading as a recolored duplicate of
# an existing identity (wall material, tank chassis, etc.), so this
# introduces one literal off-palette violet family specifically for this
# row - the same allowance `laser::LaserVariant::colors` already exercises
# for a runtime-drawn effect, extended here to a second baked sprite row
# since the palette has no candidate to reassign.
PURPLE = Palette(
    glow=(0x9B, 0x4D, 0xE0) + (255,),
    glow_md=(0x7A, 0x38, 0xB8) + (255,),
    glow_soft=(0xC4, 0x9A, 0xE8) + (150,),
    dark=(0x33, 0x14, 0x4D) + (255,),
)
ROWS = [TEAL, PURPLE]

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


def ring(img, cx, cy, r0, r1, col):
    r0_2, r1_2 = r0 * r0, r1 * r1
    for y in range(S):
        for x in range(S):
            dx, dy = x - cx, y - cy
            d2 = dx * dx + dy * dy
            if r0_2 <= d2 <= r1_2:
                put(img, x, y, col)


def bolt(img, cx, cy, angle_deg, length, col):
    """A short jagged lightning-bolt segment radiating outward from
    (cx, cy) at `angle_deg` - two straight legs with one kink, the
    'electronic' read Fire1/Hit0/Hit1/Hit2 lean on for a sci-fi splash
    rather than a shell's smoke-and-fire blast."""
    a = math.radians(angle_deg)
    dx, dy = math.cos(a), math.sin(a)
    px, py = -dy, dx  # perpendicular kink direction
    mx = cx + dx * length * 0.5 + px * length * 0.18
    my = cy + dy * length * 0.5 + py * length * 0.18
    ex, ey = cx + dx * length, cy + dy * length
    for (x0, y0, x1, y1) in ((cx, cy, mx, my), (mx, my, ex, ey)):
        steps = int(max(abs(x1 - x0), abs(y1 - y0))) + 1
        for i in range(steps + 1):
            t = i / steps
            put(img, round(x0 + (x1 - x0) * t), round(y0 + (y1 - y0) * t), col)


def frame_fire0(pal):
    """Charge building at the muzzle - small, dim."""
    f = blank()
    disc(f, CX, CY, 2.2, pal.glow_soft)
    disc(f, CX, CY, 1.1, pal.glow)
    return f


def frame_fire1(pal):
    """Bright flash as the bolt clears the barrel, arcs kicking out."""
    f = blank()
    disc(f, CX, CY, 5.0, pal.glow_soft)
    disc(f, CX, CY, 3.2, pal.glow_md)
    disc(f, CX, CY, 1.8, pal.glow)
    disc(f, CX, CY, 0.8, CORE)
    for ang in (20, 110, 200, 290):
        bolt(f, CX, CY, ang, 5.0, ARC)
    return f


def frame_fire2(pal):
    """Flash finishing, bolt pulling away - fading rings, no arcs left."""
    f = blank()
    disc(f, CX, CY, 6.5, pal.glow_soft)
    disc(f, CX, CY, 4.2, pal.glow)
    disc(f, CX, CY, 2.2, CORE)
    return f


def frame_flying(pal, scale, arc_angles=()):
    """One frame of the Flying breathing cycle - dark rim, bright mid-ring,
    hot core, all radii scaled by `scale` (1.0 = the original single-frame
    size), plus optional short arcs at `arc_angles` for a shimmer at the
    cycle's brighter frames. Deliberately kept fairly small/subtle even at
    its brightest: plasma::draw_plasma layers its own runtime sine-wave
    pulse glow on top of this (see PLASMA_PULSE_HZ in lib.rs), so the baked
    breathing animation stays an addition to that glow, not a duplicate of
    the same effect baked twice."""
    f = blank()
    ring(f, CX, CY, 4.6 * scale, 5.4 * scale, pal.dark)
    disc(f, CX, CY, 4.6 * scale, pal.glow_md)
    disc(f, CX, CY, 3.0 * scale, pal.glow)
    disc(f, CX, CY, 1.4 * scale, CORE)
    for ang in arc_angles:
        bolt(f, CX, CY, ang, 3.0 * scale, ARC)
    return f


def flying_frames(pal):
    """The 4-frame Flying breathing cycle (plasma::flying_col cycles through
    these in order, looping) - a dim -> rising -> peak -> falling triangle
    wave in size, with a couple of faint shimmer arcs added only at the
    brighter frames (peak gets four, rising/falling get two apiece at
    different angles so the shimmer itself seems to rotate, not just
    pulse)."""
    return [
        frame_flying(pal, 1.0),
        frame_flying(pal, 1.12, arc_angles=(45, 225)),
        frame_flying(pal, 1.25, arc_angles=(0, 90, 180, 270)),
        frame_flying(pal, 1.12, arc_angles=(135, 315)),
    ]


def frame_hit0(pal):
    """Impact burst starting - core flash plus four short arcs."""
    f = blank()
    disc(f, CX, CY, 5.0, pal.glow_soft)
    disc(f, CX, CY, 3.2, pal.glow)
    disc(f, CX, CY, 1.6, CORE)
    for ang in (0, 90, 180, 270):
        bolt(f, CX, CY, ang, 4.0, ARC)
    return f


def frame_hit1(pal):
    """Electric burst expanding - more, longer arcs radiating outward."""
    f = blank()
    disc(f, CX, CY, 8.0, pal.glow_soft)
    disc(f, CX, CY, 5.0, pal.glow_md)
    disc(f, CX, CY, 2.6, CORE)
    for i, ang in enumerate((15, 60, 105, 150, 195, 240, 285, 330)):
        bolt(f, CX, CY, ang, 9.0, ARC if i % 2 == 0 else pal.glow)
    return f


def frame_hit2(pal):
    """Burst dissipating - a fading ring, arcs reaching furthest out."""
    f = blank()
    ring(f, CX, CY, 8.0, 10.5, pal.glow_soft)
    disc(f, CX, CY, 6.0, pal.glow_soft)
    disc(f, CX, CY, 3.0, pal.glow_md)
    for ang in (30, 120, 210, 300):
        bolt(f, CX, CY, ang, 11.0, ARC)
    return f


def row_frames(pal):
    return [
        frame_fire0(pal), frame_fire1(pal), frame_fire2(pal), *flying_frames(pal),
        frame_hit0(pal), frame_hit1(pal), frame_hit2(pal),
    ]


cols_per_row = len(row_frames(TEAL))
sheet = Image.new('RGBA', (S * cols_per_row, S * len(ROWS)), (0, 0, 0, 0))
for r, pal in enumerate(ROWS):
    for c, fr in enumerate(row_frames(pal)):
        sheet.paste(fr, (c * S, r * S))

sheet.save(f'{OUT}/plasma.png')
print(f'wrote {OUT}/plasma.png ({S * cols_per_row}x{S * len(ROWS)})')
