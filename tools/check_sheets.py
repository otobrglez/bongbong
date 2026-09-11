"""Palette guards for the generated sprite sheets.

Two checks, both of which have caught real defects:

1. **On-palette.** Every opaque pixel of a generated sheet must be one of
   `punypalette.PUNY_PALETTE`'s colours. A sheet drifting off the set is
   how a generator quietly stops matching the ground layer everything else
   was recoloured to fit (docs/PALETTE.md).

   `walls_sheet.png` is checked against the **extended** set instead
   (`PUNY_PALETTE_ALL`): it is the one sheet that shades masonry and steel,
   which is exactly where the sampled ramp has no intermediate steps. That
   makes this check *stronger*, not weaker - an extended colour turning up
   in any other sheet means someone passed the wrong palette to `snap()`,
   and it fails here.

2. **No green on anything that sits on the grass.** Walls, props and the
   blast/rubble sheet are drawn *on top of* the ground layer, so a green
   pixel there reads as terrain showing through - the olive look the
   de-green pass exists to prevent. This is scoped deliberately: a green
   tank chassis is a real colour choice, so `scifi_tanks_sheet.png` is not
   checked, and `damage.png` carries a green tint of its own.

   `nature_sheet.png` and `trees_sheet.png` are the deliberate exceptions,
   and they are what the rule always meant: **manufactured objects are
   never green; vegetation is.** Grass that cannot be green is not grass.

`plasma.png` is deliberately off-palette (a glowing bolt that does not sit
in the terrain), so it is not listed here.

Run: `just check-sheets` (or `python3 tools/check_sheets.py`). Exits 1 on
any violation and names the offending sheet.
"""

import os
import sys

from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import punypalette as pp

STATIC = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'static')

# Generated sheets whose every opaque pixel must be on the palette.
ON_PALETTE = [
    'walls_sheet.png',
    'nature_sheet.png',
    'trees_sheet.png',
    'props_sheet.png',
    'barrel_explosion.png',
    'scifi_tanks_sheet.png',
    'shells.png',
    'minigun_bullets.png',
    'minigun_mount.png',
    'damage.png',
    'tracks.png',
]

# The subset that is drawn over the ground layer and so must carry no green.
NO_GREEN = ['walls_sheet.png', 'props_sheet.png', 'barrel_explosion.png']

PALETTE = {tuple(c) for c in pp.PUNY_PALETTE}
PALETTE_ALL = {tuple(c) for c in pp.PUNY_PALETTE_ALL}
TEAM = {tuple(c) for c in pp.PUNY_TEAM}
GREENS = {tuple(getattr(pp, n)) for n in dir(pp) if n.startswith('GREEN_')}

# The tank sheet is three blocks of the same roster (enemy, player 1,
# player 2 - docs/SPRITESHEET_SPEC.md); the player blocks alone may use the
# off-palette team family, and the enemy block must not, so a generator
# slip that tints an enemy row fails here rather than shipping.
TANK_SHEET = 'scifi_tanks_sheet.png'
TANK_ROWS_PER_TEAM = 12
TANK_CELL = 32

# Sheets allowed the palette extension (punypalette.PUNY_EXTRA): the walls
# sheet for its stone/rust steps, the vegetation sheets for GREEN_SHADE
# (and, on trees, WOOD_ASH for burnt-out foliage).
EXTENDED = {'walls_sheet.png', 'nature_sheet.png', 'trees_sheet.png'}


def scan(name):
    allowed = PALETTE_ALL if name in EXTENDED else PALETTE
    img = Image.open(os.path.join(STATIC, name)).convert('RGBA')
    off = green = 0
    for y in range(img.height):
        row_allowed = allowed
        if name == TANK_SHEET and y >= TANK_ROWS_PER_TEAM * TANK_CELL:
            row_allowed = PALETTE | TEAM
        for x in range(img.width):
            r, g, b, a = img.getpixel((x, y))
            if not a:
                continue
            if (r, g, b) not in row_allowed:
                off += 1
            if (r, g, b) in GREENS:
                green += 1
    return off, green


def main():
    failures = []
    for name in ON_PALETTE:
        off, green = scan(name)
        checks = [f'off-palette={off}' + (' (extended set)' if name in EXTENDED else '')]
        if off:
            failures.append(f'{name}: {off} off-palette pixels')
        if name in NO_GREEN:
            checks.append(f'green={green}')
            if green:
                failures.append(f'{name}: {green} green pixels (it sits on the grass)')
        print(f'{name:28s} ' + '  '.join(checks))
    if failures:
        print('\nFAILED:')
        for f in failures:
            print('  ' + f)
        return 1
    print('\nall sheet palette checks passed')
    return 0


if __name__ == '__main__':
    sys.exit(main())
