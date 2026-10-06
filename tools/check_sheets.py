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
   tank chassis is a real colour choice, so the tank sheets are not
   checked.

   `nature_sheet.png`, `nature_sheet_desert.png`, `nature_sheet_moon.png` and `trees_sheet.png` are the deliberate exceptions,
   and they are what the rule always meant: **manufactured objects are
   never green; vegetation is.** Grass that cannot be green is not grass.

The moon's grass sheet (`nature_sheet_moon.png`) is crystal shards in
`punypalette.CRYSTAL`, a cold violet-blue off the palette on purpose (the
one hue the terrain never uses, so a shard reads as growing out of the
regolith), over the extended stone greys.

`plasma.png` is deliberately off-palette (a glowing bolt that does not sit
in the terrain), so it is not listed here.

The team family (`punypalette.PUNY_TEAM`, the two player identity ramps) is
off-palette on purpose and admitted in exactly one place of its own: the
whole of `portal_sheet.png` (`TEAM_SHEETS`), which is drawn in the P1 blue
ramp so a hole in the ground reads as not-terrain. The portal sits on the
grass, so it is also held to the no-green rule.

The tank art (`tools/spritegen/tankdesign`, docs/SPRITESHEET_SPEC.md) is
held block by block (`TANK_SHEETS`): the paint and its light layer are five
blocks of the roster on 40 px cells - the enemy, then players 1 to 4 - and
every block may use the extended set plus the kit's two deep water/teal
steps (`kit.TANK_EXTRA`); a player block may use its own team's ramp and
lamp colour (`kit.TEAM_RAMPS`, `kit.TEAM_LIGHT`) and nothing of another
team's, so a generator slip that tints an enemy row, or paints player 3 in
player 1's blue, fails here rather than shipping. The weapon modules
(`tank_modules.png` and its light layer) are one row per chassis in the
enemy's colours.

`towers_sheet.png` is a manufactured object on the grass, so it is held to
the extended set and the no-green rule, with two admissions of its own
(`TOWER_EXTRA`): the P1 team ramp, which trims the player's towers, and
`punypalette.OOZE`, the bio slush's glowing acid lime - off the palette on
purpose like plasma.png, and not one of the grass greens the rule counts.

The pickups' crates (`crates_sheet.png`, docs/CRATES_SPEC.md) are wood on
the extended set plus the symbols' inks (`punypalette.PICKUP_INK`,
`PICKUP_RAINBOW`), loud on purpose like the old pickup icons, and are held
to the no-green rule since a crate sits on the grass; the symbol sheet
(`pickup_glyphs.png`) is those inks with the outline and white alone.

Run: `just check-sheets` (or `python3 tools/check_sheets.py`). Exits 1 on
any violation and names the offending sheet.
"""

import os
import sys

from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import punypalette as pp

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), 'spritegen', 'tankdesign'))
import export as tank_export  # noqa: E402
import kit as tank_kit  # noqa: E402

STATIC = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'static')

# Generated sheets whose every opaque pixel must be on the palette.
ON_PALETTE = [
    'walls_sheet.png',
    'nature_sheet.png',
    'nature_sheet_desert.png',
    'nature_sheet_moon.png',
    'trees_sheet.png',
    'props_sheet.png',
    'barrel_explosion.png',
    'scifi_tanks_sheet.png',
    'scifi_tanks_glow.png',
    'tank_modules.png',
    'tank_modules_glow.png',
    'shells.png',
    'minigun_bullets.png',
    'missile.png',
    'tracks.png',
    'portal_sheet.png',
    'towers_sheet.png',
    'crates_sheet.png',
    'pickup_glyphs.png',
]

# The subset that is drawn over the ground layer and so must carry no green.
NO_GREEN = [
    'walls_sheet.png', 'props_sheet.png', 'barrel_explosion.png', 'portal_sheet.png', 'missile.png', 'towers_sheet.png',
    'crates_sheet.png',
]

PALETTE = {tuple(c) for c in pp.PUNY_PALETTE}
PALETTE_ALL = {tuple(c) for c in pp.PUNY_PALETTE_ALL}
TEAM = {tuple(c) for c in pp.PUNY_TEAM}
GREENS = {tuple(getattr(pp, n)) for n in dir(pp) if n.startswith('GREEN_')}

# The tank art: the paint and its light layer in team blocks of the roster,
# the modules in the enemy's colours (see the module docstring).
TANK_SHEETS = {'scifi_tanks_sheet.png', 'scifi_tanks_glow.png'}
TANK_MODULE_SHEETS = {'tank_modules.png', 'tank_modules_glow.png'}
TANK_ROWS_PER_TEAM = 12
TANK_CELL = 40
TANK_BASE = PALETTE_ALL | {tuple(c) for c in tank_kit.TANK_EXTRA}


def tank_allowed(y):
    """The colours a tank sheet's pixel row `y` may use: its block's."""
    block = tank_export.BLOCKS[min(y // (TANK_ROWS_PER_TEAM * TANK_CELL), len(tank_export.BLOCKS) - 1)]
    if block == 'enemy':
        return TANK_BASE
    return TANK_BASE | {tuple(c) for c in tank_kit.TEAM_RAMPS[block]} | {tuple(tank_kit.TEAM_LIGHT[block])}

# Sheets drawn in the team family throughout: the portal is the P1 blue ramp
# over BLACK and WHITE, every row of it.
TEAM_SHEETS = {'portal_sheet.png'}

# Sheets allowed the palette extension (punypalette.PUNY_EXTRA): the walls
# sheet for its stone/rust steps, the vegetation sheets for GREEN_SHADE
# (and, on trees, WOOD_ASH for burnt-out foliage).
EXTENDED = {'walls_sheet.png', 'nature_sheet.png', 'nature_sheet_desert.png', 'nature_sheet_moon.png', 'trees_sheet.png', 'towers_sheet.png'}

# The moon's crystal shards (`punypalette.CRYSTAL`), admitted on its own
# grass sheet alone.
MOON_GRASS_SHEET = 'nature_sheet_moon.png'
CRYSTAL = {tuple(c) for c in pp.CRYSTAL}

# The towers sheet's own admissions: the player's trim and the ooze.
TOWER_SHEET = 'towers_sheet.png'
TOWER_EXTRA = {tuple(c) for c in pp.TEAM_P1} | {tuple(c) for c in pp.OOZE}

# The pickups' symbols: their inks, loud on purpose (see the module docstring).
PICKUP_INK = {tuple(c) for ramp in pp.PICKUP_INK.values() for c in ramp} | {tuple(c) for c in pp.PICKUP_RAINBOW}
CRATE_SHEET = 'crates_sheet.png'
GLYPH_SHEET = 'pickup_glyphs.png'


def scan(name):
    allowed = PALETTE_ALL if name in EXTENDED else PALETTE
    if name in TEAM_SHEETS:
        allowed = PALETTE | TEAM
    if name == TOWER_SHEET:
        allowed = PALETTE_ALL | TOWER_EXTRA
    if name == CRATE_SHEET:
        allowed = PALETTE_ALL | PICKUP_INK
    if name == GLYPH_SHEET:
        allowed = PICKUP_INK | {tuple(pp.BLACK), tuple(pp.WHITE)}
    if name in TANK_MODULE_SHEETS:
        allowed = TANK_BASE
    if name == MOON_GRASS_SHEET:
        allowed = PALETTE_ALL | CRYSTAL
    img = Image.open(os.path.join(STATIC, name)).convert('RGBA')
    off = green = 0
    for y in range(img.height):
        row_allowed = tank_allowed(y) if name in TANK_SHEETS else allowed
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
        extended = name in EXTENDED or name in TANK_SHEETS or name in TANK_MODULE_SHEETS or name == CRATE_SHEET
        checks = [f'off-palette={off}' + (' (extended set)' if extended else '')]
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
