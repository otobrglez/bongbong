"""Puny Palette -- a curated color set extracted directly from the third-
party Puny World tileset (static/punyworld/punyworld-overworld-tileset.png),
used to recolor bongbong's own generated sheets (tanks/shells/walls/damage)
so they sit in the same register as the ground layer instead of clashing
with it as candy-saturated Resurrect 64 colors on top of Puny World's much
softer, painterly terrain.

Every colour below is a real pixel sampled from the tileset's buildings,
roofs, wood, stone, water and grass regions (not invented, not a generic
"muted" recolor) -- see docs/PALETTE.md for exactly which crop each family
came from and the extraction method. This supersedes tools/resurrect64.py
as the shared palette every generator imports; that module is kept only as
historical reference for the earlier all-generated-art era (see
docs/PALETTE.md).

Unlike Resurrect 64, Puny World's own art doesn't include a true purple/
violet family (checked: none found anywhere in the populated region of the
sheet) -- rather than force an off-palette invented purple, roles that used
to be "the purple one" (e.g. the `wraith` tank) were reassigned to a hue
family the source art actually has. See gen_tanks.py's roster comment.
"""

# Near-black outline/char tone -- from the building region's darkest
# shadow pixel. Puny World has no true black either, same situation as R64.
BLACK = (0x25, 0x25, 0x25)
# Genuine white -- used sparingly for spark cores/highlights (e.g. the blue
# shell family's Fire0 core). Not sampled from the tileset since it's
# already the literal brightest possible value; included explicitly so
# these pixels count as on-palette rather than needing a special case.
WHITE = (0xFF, 0xFF, 0xFF)

# Stone/plaster wall ramp (light -> dark), from the grey building walls.
STONE_PALE = (0xDA, 0xE5, 0xCE)
STONE_LT = (0xAC, 0xB7, 0xA1)
STONE_MD = (0x5D, 0x65, 0x4F)
STONE_DK = (0x4C, 0x52, 0x3C)

# Wood/tan ramp (light -> dark), from wood-plank building walls and props.
WOOD_PALE = (0xD8, 0xBF, 0x8E)
WOOD_LT = (0xDE, 0x99, 0x43)
WOOD_MD = (0xCA, 0x8A, 0x3B)
WOOD_DK = (0x99, 0x65, 0x24)
WOOD_DEEPER = (0x68, 0x47, 0x1D)

# Red ramp (bright -> dark), from red-tile roofs.
RED_BRIGHT = (0xFF, 0x42, 0x1A)
RED_MD = (0xE4, 0x42, 0x19)
RED_DEEP = (0x9C, 0x35, 0x27)
RED_DK = (0x81, 0x2F, 0x27)
RED_DARKEST = (0x4A, 0x22, 0x21)

# Teal ramp (bright -> dark), from teal-tile roofs.
TEAL_BRIGHT = (0x00, 0xD0, 0x97)
TEAL_LT = (0x00, 0xBB, 0x8F)
TEAL_MD = (0x00, 0xA6, 0x7F)
TEAL_DK = (0x00, 0x7E, 0x53)
TEAL_DARKEST = (0x00, 0x37, 0x23)

# Blue ramp (bright -> dark), from the sea/river water tiles.
BLUE_BRIGHT = (0x27, 0xD8, 0xC5)
BLUE_LT = (0x1E, 0xB3, 0xAE)
BLUE_MD = (0x04, 0xA0, 0xB4)
BLUE_DK = (0x03, 0x8A, 0xAB)
BLUE_DARKEST = (0x0E, 0x8B, 0x96)

# Green ramp (bright -> dark), from the grass fill tiles.
GREEN_BRIGHT = (0x9F, 0xB7, 0x47)
GREEN_LT = (0x85, 0xA6, 0x43)
GREEN_MD = (0x7C, 0x98, 0x3C)
GREEN_DK = (0x5F, 0x91, 0x4B)
GREEN_DARKEST = (0x1C, 0x4C, 0x33)

# Gold/yellow accents, from lantern/prop highlights.
GOLD_BRIGHT = (0xEE, 0xA3, 0x43)
GOLD_MD = (0xDC, 0x9C, 0x4A)
GOLD_PALE = (0xCA, 0xC5, 0x94)

PUNY_PALETTE = [
    BLACK, WHITE,
    STONE_PALE, STONE_LT, STONE_MD, STONE_DK,
    WOOD_PALE, WOOD_LT, WOOD_MD, WOOD_DK, WOOD_DEEPER,
    RED_BRIGHT, RED_MD, RED_DEEP, RED_DK, RED_DARKEST,
    TEAL_BRIGHT, TEAL_LT, TEAL_MD, TEAL_DK, TEAL_DARKEST,
    BLUE_BRIGHT, BLUE_LT, BLUE_MD, BLUE_DK, BLUE_DARKEST,
    GREEN_BRIGHT, GREEN_LT, GREEN_MD, GREEN_DK, GREEN_DARKEST,
    GOLD_BRIGHT, GOLD_MD, GOLD_PALE,
]

_cache = {}


def nearest(rgb):
    """Nearest PUNY_PALETTE colour to an arbitrary (r, g, b[, a]) tuple, by
    squared Euclidean RGB distance. Alpha (if present) passes through
    unchanged."""
    key = (rgb[0], rgb[1], rgb[2])
    hit = _cache.get(key)
    if hit is not None:
        return hit
    best, best_d = PUNY_PALETTE[0], None
    for cand in PUNY_PALETTE:
        d = (cand[0] - key[0]) ** 2 + (cand[1] - key[1]) ** 2 + (cand[2] - key[2]) ** 2
        if best_d is None or d < best_d:
            best, best_d = cand, d
    _cache[key] = best
    return best


def snap(rgba):
    """Snap a colour to its nearest PUNY_PALETTE match, preserving alpha (or
    defaulting to opaque for a bare 3-tuple)."""
    r, g, b = nearest(rgba)
    a = rgba[3] if len(rgba) > 3 else 255
    return (r, g, b, a)
