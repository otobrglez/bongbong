"""Generate static/portal_sheet.png - the portal: a slowly turning three-arm
logarithmic spiral, a sci-fi blue black hole sunk into the ground.

Layout: 1152x288, three rows of 12 cells, each 96x96 (`PORTAL_SHEET_COLS`
in lib.rs; rows of twelve keep the sheet under the 2048 px GL ES 2 floor).
    cells 0-23 Rotation frames, row-major (frame f at col f % 12, row
               f // 12). A three-arm spiral is 120 degrees periodic, so
               frame f is the spiral turned by -f * 5 degrees and the
               twenty-four frames make exactly one visual period; play
               them in order and loop. The sign is the spin direction:
               with y down a positive turn is clockwise on screen, so the
               frames turn the arms anticlockwise - against their winding,
               which reads as the spiral pulling inward. The code only
               steps the frame index; flip the sign here to turn it the
               other way.
    cell 24    (col 0, row 2) The builder's 32x32 tool icon in the top-left
               corner of the cell (the rest transparent): the same spiral
               at frame 0 on a 16x16 design buffer.
The frame's 48-unit design field maps EXTENT = 23 units to the cell's edge
and the disc stops at RADIUS = 20.7 of them (90% - a portal has to sit
inside the biggest hull's turning circle, not swallow it), so the disc is
about 42 design px across, a frame's corners are always transparent and the
drawn portal spills cleanly over neighbouring cells.

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
phi = atan2(dy, dx), every radius below a fraction of RADIUS (R):
    a = ((phi - theta0 - TWIST * ln(max(r, 1))) mod (2pi/ARMS)) / (2pi/ARMS)
    theta0 = -f * (2pi/ARMS) / FRAMES
    r <= 0.15 R        opaque BLACK core
    0.15 R < r <= 0.26 R   dk ring (the event horizon)
    0.26 R < r <= R    on-arm when a < w(r), w(r) = 0.55 - 0.35 * r / R
                       (arms thin outward): lt (r < 0.43 R) / base
                       (r < 0.65 R) / md (r < 0.87 R) / dk, alpha 255 below
                       0.78 R, 176 to 0.91 R, 96 beyond; between the arms a
                       dk haze at alpha 96 for r < 0.52 R, transparent past
    r > R              transparent
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
FRAMES = 24
ARMS = 3
TWIST = 1.6       # radians of arm swing per e-fold of radius
EXTENT = 23.0     # design units from the centre to the cell's edge
RADIUS = EXTENT * 0.9   # the disc's outer edge, in the same units
COLS = 12         # cells per sheet row (lib.rs PORTAL_SHEET_COLS)
ROWS = FRAMES // COLS + 1   # the icon sits alone on the last row
OUT = os.environ.get('SPRITE_OUT', 'assets/sprites')
os.makedirs(OUT, exist_ok=True)

A_FULL, A_MID, A_LOW = 255, 176, 96
SECTOR = math.tau / ARMS
SPECK_RADII = (0.35 * RADIUS, 0.48 * RADIUS, 0.61 * RADIUS)


def C(c, a=255):
    return c + (a,)


def arm_width(r):
    """Angular half-width of an arm as a fraction of its sector; thins outward."""
    return 0.55 - 0.35 * r / RADIUS


def arm_coord(r, phi, theta0):
    return ((phi - theta0 - TWIST * math.log(max(r, 1.0))) % SECTOR) / SECTOR


def arm_colour(r):
    if r < 0.43 * RADIUS:
        c = LT
    elif r < 0.65 * RADIUS:
        c = BASE
    elif r < 0.87 * RADIUS:
        c = MD
    else:
        c = DK
    if r < 0.78 * RADIUS:
        a = A_FULL
    elif r < 0.91 * RADIUS:
        a = A_MID
    else:
        a = A_LOW
    return C(c, a)


def sample(r, phi, theta0):
    """The spiral field at polar (r, phi) in the 48-buffer's design units."""
    if r > RADIUS:
        return None
    if r <= 0.15 * RADIUS:
        return C(BLACK)
    if r <= 0.26 * RADIUS:
        return C(DK)
    if arm_coord(r, phi, theta0) < arm_width(r):
        return arm_colour(r)
    if r < 0.52 * RADIUS:
        return C(DK, A_LOW)
    return None


def draw(size, theta0):
    """One spiral on a size x size buffer, the 48-buffer field scaled so
    EXTENT lands on the buffer's edge (the disc stops at RADIUS inside it)."""
    img = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    centre = (size - 1) / 2.0
    scale = EXTENT / (size / 2.0 - 0.5)     # buffer px -> 48-buffer design units
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


sheet = Image.new('RGBA', (CELL * COLS, CELL * ROWS), (0, 0, 0, 0))
for f in range(FRAMES):
    theta0 = -f * SECTOR / FRAMES
    sheet.paste(up2(draw(S, theta0), CELL), ((f % COLS) * CELL, (f // COLS) * CELL))   # no mask: no alpha drift
sheet.paste(up2(draw(ICON_S, 0.0), ICON), ((FRAMES % COLS) * CELL, (FRAMES // COLS) * CELL))
sheet.save(f'{OUT}/portal_sheet.png')
print('portal_sheet.png', sheet.size)
