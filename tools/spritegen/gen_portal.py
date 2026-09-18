"""Generate static/portal_sheet.png - the portal: a slowly turning two-arm
logarithmic spiral, a sci-fi blue black hole sunk into the ground.

Layout: 1248x96, one row of 13 cells, each 96x96.
    cols 0-11  Rotation frames. A two-arm spiral is 180 degrees periodic,
               so frame f is the spiral turned by f * 15 degrees and the
               twelve frames make exactly one visual period; play them in
               order and loop.
    col 12     The builder's 32x32 tool icon in the top-left corner of the
               cell (the remaining 64 px transparent): the same spiral at
               frame 0 on a 16x16 design buffer, radius ~7.5 design px.
The disc itself is 47 design px across, so a frame's corners are always
transparent and the drawn portal spills cleanly over neighbouring cells.

Every frame is drawn on a 48x48 design buffer (the icon on 16x16) and
upscaled 2x with NEAREST, the same 2 px block density gen_props.py bakes
into its 16 -> 32 cells. Do not pixelate the result again.

Colours - exactly seven, and stepped alphas only (255 / 176 / 96):
    BLACK   #252525  the core, opaque
    dk      #484A77  event-horizon ring, outer arm tips, the haze
    md      #4D65B4  mid arm
    base    #4D9BE6  inner arm
    lt      #8FD3FF  innermost arm, hugging the horizon
    WHITE   #FFFFFF  six hot specks riding the arms' centrelines
The four blues are the P1 team blue ramp (punypalette.TEAM_P1), deliberately
off the Puny palette like plasma.png and the pickup icons - a hole in the
ground has to read as not-terrain. `snap()` is never called here: it would
quantise the blues back onto the terrain's set. check_sheets.py admits the
team family on this sheet (TEAM_SHEETS).

Field, per design pixel about the centre (23.5, 23.5), r = hypot(dx, dy),
phi = atan2(dy, dx):
    a = ((phi - theta0 - TWIST * ln(max(r, 1))) mod (2pi/ARMS)) / (2pi/ARMS)
    theta0 = f * (2pi/ARMS) / FRAMES
    r <= 3.5           opaque BLACK core
    3.5 < r <= 6       dk ring (the event horizon)
    6 < r <= 23        on-arm when a < w(r), w(r) = 0.55 - 0.35 * r / 23
                       (arms thin outward): lt (r < 10) / base (r < 15) /
                       md (r < 20) / dk, alpha 255 below r 18, 176 to r 21,
                       96 beyond; between the arms a dk haze at alpha 96 for
                       r < 12, transparent past that
    r > 23             transparent
Every choice is a function of (x, y, f): no `random`, no seed.

Run:  nix-shell -p "python3.withPackages (ps: [ps.pillow])" \\
        --run "SPRITE_OUT=static python3 tools/spritegen/gen_portal.py"
"""
from PIL import Image
import os, math, sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from punypalette import BLACK, WHITE, TEAM_P1

DK, MD, BASE, LT = TEAM_P1

S = 48            # design buffer of one frame; one drawn pixel = a 2x2 block
CELL = 96
ICON_S = 16       # design buffer of the icon
ICON = 32
FRAMES = 12
ARMS = 2
TWIST = 1.6       # radians of arm swing per e-fold of radius
RADIUS = 23.0     # the disc's outer edge, in design px of the 48 buffer
COLS = FRAMES + 1
OUT = os.environ.get('SPRITE_OUT', 'assets/sprites')
os.makedirs(OUT, exist_ok=True)

A_FULL, A_MID, A_LOW = 255, 176, 96
SECTOR = math.tau / ARMS
SPECK_RADII = (8.0, 11.0, 14.0)


def C(c, a=255):
    return c + (a,)


def arm_width(r):
    """Angular half-width of an arm as a fraction of its sector; thins outward."""
    return 0.55 - 0.35 * r / RADIUS


def arm_coord(r, phi, theta0):
    return ((phi - theta0 - TWIST * math.log(max(r, 1.0))) % SECTOR) / SECTOR


def arm_colour(r):
    if r < 10.0:
        c = LT
    elif r < 15.0:
        c = BASE
    elif r < 20.0:
        c = MD
    else:
        c = DK
    if r < 18.0:
        a = A_FULL
    elif r < 21.0:
        a = A_MID
    else:
        a = A_LOW
    return C(c, a)


def sample(r, phi, theta0):
    """The spiral field at polar (r, phi) in the 48-buffer's design units."""
    if r > RADIUS:
        return None
    if r <= 3.5:
        return C(BLACK)
    if r <= 6.0:
        return C(DK)
    if arm_coord(r, phi, theta0) < arm_width(r):
        return arm_colour(r)
    if r < 12.0:
        return C(DK, A_LOW)
    return None


def draw(size, theta0):
    """One spiral on a size x size buffer, the 48-buffer field scaled so the
    disc's outer edge lands just inside the buffer."""
    img = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    centre = (size - 1) / 2.0
    scale = RADIUS / (size / 2.0 - 0.5)     # buffer px -> 48-buffer design units
    for y in range(size):
        for x in range(size):
            dx, dy = x - centre, y - centre
            r = math.hypot(dx, dy) * scale
            c = sample(r, math.atan2(dy, dx), theta0)
            if c is not None:
                img.putpixel((x, y), c)
    # Hot specks on each arm's centreline, turning with the arms.
    for k in range(ARMS):
        for r in SPECK_RADII:
            phi = theta0 + TWIST * math.log(r) + (arm_width(r) / 2.0 + k) * SECTOR
            px = int(round(centre + r / scale * math.cos(phi)))
            py = int(round(centre + r / scale * math.sin(phi)))
            if 0 <= px < size and 0 <= py < size:
                img.putpixel((px, py), C(WHITE))
    return img


def up2(img, size):
    return img.resize((size, size), Image.NEAREST)


sheet = Image.new('RGBA', (CELL * COLS, CELL), (0, 0, 0, 0))
for f in range(FRAMES):
    theta0 = f * SECTOR / FRAMES
    sheet.paste(up2(draw(S, theta0), CELL), (f * CELL, 0))   # no mask: no alpha drift
sheet.paste(up2(draw(ICON_S, 0.0), ICON), (FRAMES * CELL, 0))
sheet.save(f'{OUT}/portal_sheet.png')
print('portal_sheet.png', sheet.size)
