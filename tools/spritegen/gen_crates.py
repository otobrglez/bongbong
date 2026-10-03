"""The pickups' supply crates and their symbols: static/crates_sheet.png and
static/pickup_glyphs.png (docs/CRATES_SPEC.md).

The Stencil line of the Quartermaster study (the author's pick): a field
supply crate of dark-stained planks between two side battens, nailed at the
corners, the symbol of what is inside sprayed on in its category colour.
Straight down with a two-row front face, lit from the top left, the tanks'
#252525 outline, on the 2 px block grid (16 x 16 design pixels per 32 px
cell, every design pixel a 2 x 2 block). The wood, the nails and the outline
are the Puny palette; the symbols are the loud, off-palette inks of
`punypalette.PICKUP_INK`, admitted by tools/check_sheets.py on these two
sheets alone - the pickups' old exemption, kept for the one part that has
to be spotted at a glance.

crates_sheet.png, 7 columns x 11 rows of 32 px:

    row = PickupKind in declaration order (health, ammo, laser, minigun,
          plasma, missiles, speedup, shield, flamethrower, frog_health,
          tower_pack)
    col 0    the crate
    col 1-4  a glint sweeping the lid from top left to bottom right, a band
             of design pixels one ramp step lighter (the idle)
    col 5    damaged: a split plank, a cracked batten, paint chipped
    col 6    charred: what a fire leaves before the crate breaks

pickup_glyphs.png, 1 column x 11 rows of 20 px, the same row order: the
symbol on its own - its ink, lit along the top, shaded along the bottom,
with the outline - for what rises out of an opened crate, a spilled one,
the HUD's weapon queue and the builder's brushes.

No Pillow: raw PNG bytes, the tower generator's convention. Deterministic:
every choice is a position hash.

    python3 tools/spritegen/gen_crates.py
"""

import os
import struct
import sys
import zlib

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
import punypalette as pp  # noqa: E402

STATIC = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'static')
GRID = 16           # design pixels per crate cell side
CELL = GRID * 2     # sheet pixels per crate cell side
COLS = 7
TOKEN = 10          # design pixels per symbol cell side (the 8 x 8 symbol and its outline)

# PickupKind's declaration order: the sheets' row order.
KINDS = ['health', 'ammo', 'laser', 'minigun', 'plasma', 'missiles', 'speedup', 'shield', 'flamethrower',
         'frog_health', 'tower_pack']

# The symbols, 8 x 8 design pixels each.
GLYPHS = {
    'health': ['..XXXX..', '..XXXX..', 'XXXXXXXX', 'XXXXXXXX', 'XXXXXXXX', 'XXXXXXXX', '..XXXX..', '..XXXX..'],
    'shield': ['XXXXXXXX', 'XXXXXXXX', 'XXXXXXXX', 'XXXXXXXX', '.XXXXXX.', '.XXXXXX.', '..XXXX..', '...XX...'],
    'ammo': ['.X....X.', 'XXX..XXX', 'XXX..XXX', 'XXX..XXX', 'XXX..XXX', '........', 'XXX..XXX', 'XXX..XXX'],
    'minigun': ['XX.XX.XX', 'XX.XX.XX', 'XX.XX.XX', 'XX.XX.XX', '........', 'XXXXXXXX', 'XXXXXXXX', '.XXXXXX.'],
    'missiles': ['...XX...', '..XXXX..', '..XXXX..', '..XXXX..', '..XXXX..', '.XXXXXX.', '.X.XX.X.', '...XX...'],
    'flamethrower': ['...X....', '...XX...', '..XXX..X', '..XXXX.X', '.XXXXXXX', 'XXXX.XXX', 'XXX...XX', '.XXXXXX.'],
    'laser': ['......X.', '.....XXX', '....XXX.', '...XX...', '..XX....', '.XX.....', 'XX......', 'X.......'],
    'plasma': ['X......X', '..XXXX..', '.XXXXXX.', '.XXXXXX.', '.XXXXXX.', '.XXXXXX.', '..XXXX..', 'X......X'],
    'speedup': ['....XXX.', '...XXX..', '..XXX...', '.XXXXXXX', 'XXXXXXX.', '...XXX..', '..XXX...', '.XX.....'],
    'frog_health': ['.XX..XX.', '.XXXXXX.', 'X.XXXX.X', 'XXXXXXXX', '..XXXX..', '.XXXXXX.', 'XX.XX.XX', 'X......X'],
    'tower_pack': ['X..X....', 'XXXX....', '.XXX....', '..XXX...', '...XXX..', '....XXX.', '.....XXX', '......XX'],
}

# The crate, 16 x 16 design pixels. Lid rows 1-11 between the outline, the
# front face rows 12-13. Codes below.
CRATE = [
    '................',
    '.kkkkkkkkkkkkkk.',
    '.kbceeeeeeeecdk.',
    '.kcnffffffffnek.',
    '.kcdffffffffdek.',
    '.kcdggggggggdek.',
    '.kcdeeeeeeeedek.',
    '.kcdffffffffdek.',
    '.kcdffffffffdek.',
    '.kcdggggggggdek.',
    '.kcneeeeeeeenek.',
    '.kcdffffffffdek.',
    '.keeffffffffefk.',
    '.kffggggggggfgk.',
    '.kkkkkkkkkkkkkk.',
    '................',
]
LID = (2, 11)                       # the lid's rows inside the outline
WINDOW = (4, 2, 8, 10)              # where the symbol goes: x, y, w, h

CODE = {
    'k': pp.BLACK, 'n': pp.STONE_MID,
    'b': pp.WOOD_LT, 'c': pp.WOOD_MD, 'd': pp.WOOD_AMBER, 'e': pp.WOOD_DK, 'f': pp.WOOD_DEEPER,
    'g': pp.WOOD_DARKEST,
}
# The grain: a hashed speckle a step darker, per board.
GRAIN = {'b': pp.WOOD_MD, 'c': pp.WOOD_AMBER, 'e': pp.WOOD_DEEPER, 'f': pp.WOOD_DARKEST}

# One step lighter (the glint) and two steps darker (the char), within each
# ramp the crate is drawn in.
LIGHTER = {
    pp.WOOD_DARKEST: pp.WOOD_DEEPER, pp.WOOD_DEEPER: pp.WOOD_DK, pp.WOOD_DK: pp.WOOD_AMBER,
    pp.WOOD_AMBER: pp.WOOD_MD, pp.WOOD_MD: pp.WOOD_LT, pp.WOOD_LT: pp.WOOD_PALE, pp.STONE_MID: pp.STONE_HI,
}
CHARRED = {
    pp.WOOD_LT: pp.WOOD_DK, pp.WOOD_MD: pp.RUST_DK, pp.WOOD_AMBER: pp.WOOD_DEEPER, pp.WOOD_DK: pp.WOOD_DARKEST,
    pp.WOOD_DEEPER: pp.STONE_DARKEST, pp.WOOD_DARKEST: pp.BLACK, pp.STONE_MID: pp.STONE_SHADE,
}


def h32(x, y, s):
    h = (x * 0x27d4eb2d) ^ ((y + 0x165667b1) * 668265263) ^ ((s + 0x9e3779b9) * 2246822519)
    h &= 0xffffffff
    h ^= h >> 15
    h = (h * 0x2c1b3c6d) & 0xffffffff
    h ^= h >> 12
    return h / 4294967296.0


def on(kind, x, y):
    return 0 <= x < 8 and 0 <= y < 8 and GLYPHS[kind][y][x] == 'X'


def rows_of(kind):
    used = [y for y in range(8) if 'X' in GLYPHS[kind][y]]
    return used[0], used[-1]


def ink(kind):
    return pp.PICKUP_INK[kind]           # (shade, base, light)


def rainbow(x, y):
    return pp.PICKUP_RAINBOW[min(len(pp.PICKUP_RAINBOW) - 1, int((x + y) / 2.3))]


def crate(kind, row):
    """The crate's design pixels as {(x, y): rgb}."""
    px = {}
    for y in range(GRID):
        for x in range(GRID):
            ch = CRATE[y][x]
            if ch == '.':
                continue
            c = CODE[ch]
            if ch in GRAIN and h32(x, y, 3) < 0.12:
                c = GRAIN[ch]
            px[(x, y)] = c
    # The symbol, sprayed on: flat paint, a hashed chip or two gone from
    # its edge - never a hole inside it, which turns a solid symbol (the
    # cross) into a blot.
    wx, wy, ww, wh = WINDOW
    top, bottom = rows_of(kind)
    gx = wx + (ww - 8) // 2
    gy = wy + (wh - (bottom - top + 1)) // 2 - top
    for y in range(8):
        for x in range(8):
            if not on(kind, x, y):
                continue
            X, Y = gx + x, gy + y
            edge = not all(on(kind, x + dx, y + dy) for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)))
            if edge and h32(X, Y, 11 + row) < 0.03:
                continue
            px[(X, Y)] = rainbow(x, y) if kind == 'shield' else ink(kind)[1]
    return px


def glint(px, kind, frame):
    """Col 1-4: a band two design pixels wide, one step lighter, across the lid."""
    out = dict(px)
    p = 4 + frame * 6
    symbol = {ink(kind)[1]: ink(kind)[2], ink(kind)[2]: pp.WHITE}
    for (x, y), c in px.items():
        if not (LID[0] <= y <= LID[1]) or c == pp.BLACK:
            continue
        if 0 <= x + y - p < 2:
            out[(x, y)] = LIGHTER.get(c, symbol.get(c, pp.WHITE if c in pp.PICKUP_RAINBOW else c))
    return out


def damaged(px, kind):
    """Col 5: a split across the middle plank, a cracked batten, chipped paint."""
    out = dict(px)
    for x in range(4, 12):
        y = 6 + (1 if x in (7, 8, 9) else 0)
        out[(x, y)] = pp.WOOD_DARKEST
    out[(3, 7)] = pp.WOOD_DARKEST
    out[(12, 4)] = pp.WOOD_DARKEST
    out[(12, 5)] = pp.WOOD_DEEPER
    paint = set(ink(kind)) | set(pp.PICKUP_RAINBOW)
    for (x, y), c in px.items():
        if c in paint and h32(x, y, 21) < 0.18:
            out[(x, y)] = pp.WOOD_DEEPER
    return out


def charred(px, kind):
    """Col 6: every board two steps darker, the paint scorched to its shade."""
    out = {}
    paint = set(ink(kind)) | set(pp.PICKUP_RAINBOW)
    for (x, y), c in damaged(px, kind).items():
        if c in paint:
            out[(x, y)] = ink(kind)[0] if h32(x, y, 31) < 0.6 else pp.STONE_DARKEST
        else:
            out[(x, y)] = CHARRED.get(c, c)
    return out


def token(kind):
    """The symbol on its own: lit top, shaded bottom, outlined."""
    px = {}
    for y in range(-1, 9):
        for x in range(-1, 9):
            if on(kind, x, y):
                if kind == 'shield':
                    c = pp.WHITE if not on(kind, x, y - 1) else rainbow(x, y)
                elif not on(kind, x, y - 1):
                    c = ink(kind)[2]
                elif not on(kind, x, y + 1):
                    c = ink(kind)[0]
                else:
                    c = ink(kind)[1]
                px[(x + 1, y + 1)] = c
            elif on(kind, x - 1, y) or on(kind, x + 1, y) or on(kind, x, y - 1) or on(kind, x, y + 1):
                px[(x + 1, y + 1)] = pp.BLACK
    return px


def blit(pixels, px, ox, oy, scale=2):
    for (x, y), c in px.items():
        for j in range(scale):
            for i in range(scale):
                pixels[oy + y * scale + j][ox + x * scale + i] = (c[0], c[1], c[2], 255)


def png(width, height, pixels):
    raw = b''.join(b'\x00' + b''.join(struct.pack('BBBB', *p) for p in row) for row in pixels)

    def chunk(kind, data):
        body = kind + data
        return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body) & 0xffffffff)

    header = struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', header) + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b'')


def write(name, width, height, pixels):
    path = os.path.join(STATIC, name)
    with open(path, 'wb') as f:
        f.write(png(width, height, pixels))
    print(f'wrote {os.path.normpath(path)} ({width}x{height})')


def main():
    width, height = COLS * CELL, len(KINDS) * CELL
    sheet = [[(0, 0, 0, 0)] * width for _ in range(height)]
    for row, kind in enumerate(KINDS):
        base = crate(kind, row)
        cols = [base] + [glint(base, kind, f) for f in range(4)] + [damaged(base, kind), charred(base, kind)]
        for col, px in enumerate(cols):
            blit(sheet, px, col * CELL, row * CELL)
    write('crates_sheet.png', width, height, sheet)

    side = TOKEN * 2
    glyphs = [[(0, 0, 0, 0)] * side for _ in range(len(KINDS) * side)]
    for row, kind in enumerate(KINDS):
        blit(glyphs, token(kind), 0, row * side)
    write('pickup_glyphs.png', side, len(KINDS) * side, glyphs)


if __name__ == '__main__':
    main()
