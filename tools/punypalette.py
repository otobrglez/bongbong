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

De-green pass (2026-08): the first extraction sampled its "stone grey" ramp
from the grey building walls, which are really green-grey, and the whole
game inherited an olive cast through every grey role. The STONE_* family now
holds the tileset's true neutral greys (rock/well props), a warm SAND_*
family (dirt paths) was added, and no olive tone remains in PUNY_PALETTE --
see the per-family comments below and docs/PALETTE.md.
"""

# Near-black outline/char tone -- from the building region's darkest
# shadow pixel. Puny World has no true black either, same situation as R64.
BLACK = (0x25, 0x25, 0x25)
# Genuine white -- used sparingly for spark cores/highlights (e.g. the blue
# shell family's Fire0 core). Not sampled from the tileset since it's
# already the literal brightest possible value; included explicitly so
# these pixels count as on-palette rather than needing a special case.
WHITE = (0xFF, 0xFF, 0xFF)

# Stone ramp (light -> dark) -- TRUE NEUTRAL greys, from the tileset's
# rock/well props and building window/chimney details (e.g. the well at
# ~(52,467), windows at ~(121,418)). The first Puny Palette pass sampled
# this family from the grey *building walls* instead, which are actually
# green-grey (#ACB7A1/#5D654F/#4C523C -- green channel dominant); since
# every "grey" role (tank greebles/treads, brick + iron walls, shell smoke)
# routed through them, the whole game picked up an olive cast on top of the
# already-green grass. De-green pass (2026-08): olive is out of the palette
# entirely -- the only greens left are the grass-green GREEN_* family,
# reserved for deliberately-green identities, never for "grey" roles.
STONE_PALE = (0xF0, 0xF0, 0xF0)
STONE_LT = (0xC1, 0xC1, 0xC1)
STONE_MD = (0x9E, 0x9E, 0x96)
STONE_DK = (0x7E, 0x7E, 0x7E)
STONE_DARKEST = (0x37, 0x37, 0x37)

# Sand/khaki ramp (light -> dark), from the dirt patches and sand paths --
# the pack's single biggest non-grass, non-water ground colour, missed
# entirely by the first palette pass. Warm (r >= g > b) all the way down;
# the dark step comes from crate/prop shadow browns, not the olive
# dark-khakis (#787F4F etc.) that dominate the path *edges*.
SAND_PALE = (0xD2, 0xBA, 0x6B)
SAND_LT = (0xC9, 0xB2, 0x66)
SAND_MD = (0xB7, 0xA2, 0x48)
SAND_DK = (0x67, 0x51, 0x2A)

# Wood/tan ramp (light -> dark), from wood-plank building walls and props,
# extended in the de-green pass with the orange-building midtones
# (#B57A28/#A76921) and deep shadow (#50330B) so warm shading math snaps
# within the family instead of drifting to whatever else is nearby.
WOOD_PALE = (0xD8, 0xBF, 0x8E)
WOOD_LT = (0xDE, 0x99, 0x43)
WOOD_MD = (0xCA, 0x8A, 0x3B)
WOOD_AMBER = (0xB5, 0x7A, 0x28)
WOOD_DK = (0x99, 0x65, 0x24)
WOOD_DEEPER = (0x68, 0x47, 0x1D)
WOOD_DARKEST = (0x50, 0x33, 0x0B)

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
    STONE_PALE, STONE_LT, STONE_MD, STONE_DK, STONE_DARKEST,
    SAND_PALE, SAND_LT, SAND_MD, SAND_DK,
    WOOD_PALE, WOOD_LT, WOOD_MD, WOOD_AMBER, WOOD_DK, WOOD_DEEPER, WOOD_DARKEST,
    RED_BRIGHT, RED_MD, RED_DEEP, RED_DK, RED_DARKEST,
    TEAL_BRIGHT, TEAL_LT, TEAL_MD, TEAL_DK, TEAL_DARKEST,
    BLUE_BRIGHT, BLUE_LT, BLUE_MD, BLUE_DK, BLUE_DARKEST,
    GREEN_BRIGHT, GREEN_LT, GREEN_MD, GREEN_DK, GREEN_DARKEST,
    GOLD_BRIGHT, GOLD_MD, GOLD_PALE,
]

# ---------------------------------------------------------------------
# The wall-detail extension (2026-09)
# ---------------------------------------------------------------------
# Puny World is a 16px-prop tileset: its ramps carry enough steps to tint a
# barrel, not to *shade* masonry. The greys are the thin ones - between
# STONE_LT #C1C1C1 and STONE_MD #9E9E96 there is no true mid step at all,
# and STONE_DK #7E7E7E to STONE_DARKEST #373737 is a 71-value cliff that
# `inner_shadow` used to jump straight across, which is why a damaged edge
# read as a hole with a black liner rather than as depth.
#
# These are *interpolated between two already-sampled neighbours in the same
# family*, then (for the greys) forced to exact neutral. The hue is still
# the tileset's; only the spacing is ours. Re-sampling the pack would not
# help - the colours simply are not in it.
#
# Additive and opt-in on purpose. `snap()` is used by every generator, so
# folding these into PUNY_PALETTE would silently re-quantise tanks, shells
# and props the next time anyone regenerated them. Only gen_walls.py passes
# PUNY_PALETTE_ALL.
#
# De-green invariant: none of these belongs to the GREEN_* family. Note the
# rule is *family membership*, not "green is not the largest channel" - the
# pack's own BLUE_BRIGHT #27D8C5 and TEAL_BRIGHT #00D097 are both
# numerically green-dominant cyans, and BLUE_PALE below is one too. A
# channel test would reject them and the water they came from. `just
# check-sheets` enforces the membership form on every sheet drawn over the
# ground layer; see docs/PALETTE.md.
STONE_HI = (0xDA, 0xDA, 0xDA)       # mid(STONE_PALE, STONE_LT)
STONE_MID = (0xB0, 0xB0, 0xB0)      # mid(STONE_LT, STONE_MD), neutralised
STONE_MDK = (0x8E, 0x8E, 0x8E)      # mid(STONE_MD, STONE_DK), neutralised
STONE_SHADE = (0x5A, 0x5A, 0x5A)    # mid(STONE_DK, STONE_DARKEST)
RUST_MD = (0x8D, 0x4A, 0x25)        # mid(WOOD_DK, RED_DK)
RUST_DK = (0x59, 0x34, 0x1F)        # mid(RED_DARKEST, WOOD_DEEPER)
WOOD_ASH = (0x73, 0x62, 0x4D)       # mid(WOOD_DEEPER, STONE_DK)
BLUE_PALE = (0x93, 0xEC, 0xE2)      # mid(BLUE_BRIGHT, WHITE)
# Vegetation shade. GREEN_DARKEST #1C4C33 to GREEN_DK #5F914B is a 70-value
# jump with nothing between, so a grass blade built from the pack's greens
# reads as a solid dark block with light specks floating over it rather than
# as blades. This is the one step that connects them - and it is genuinely
# darker than the *retinted* live ground (#619541), which GREEN_DK is not,
# so a clump reads against the field it grows out of.
GREEN_SHADE = (0x3D, 0x6E, 0x3F)    # mid(GREEN_DARKEST, GREEN_DK)

PUNY_EXTRA = [
    STONE_HI, STONE_MID, STONE_MDK, STONE_SHADE,
    RUST_MD, RUST_DK, WOOD_ASH, BLUE_PALE,
    GREEN_SHADE,
]

PUNY_PALETTE_ALL = PUNY_PALETTE + PUNY_EXTRA

# ---------------------------------------------------------------------
# The team family (2026-09): the two player identities
# ---------------------------------------------------------------------
# The player tanks are recoloured copies of the enemy chassis
# (docs/player-indicator-improvements.md), and the identity colour has to
# be one no enemy hull, wall, prop or ground tile ever shows. The Puny
# Palette cannot supply that: every one of its families is already some
# chassis's body or accent, and the handful of base colours no tank uses
# sit within 13-38 RGB units of one that does. These two ramps are
# Resurrect 64's own blue and magenta steps - the three hue regions nothing
# on screen occupies are true blue, violet and magenta - and they are
# deliberately *off* the Puny set, the same exemption the pickup icons and
# plasma.png have: an identity has to be loud against the terrain, not sit
# in it. Each is (dk, md, base, lt); the base is the hull body, the light
# step the accent. Never folded into PUNY_PALETTE or PUNY_EXTRA - snap()
# must keep quantising everything else onto the terrain's set - and
# check_sheets.py admits them on the tank sheet's player rows only.
TEAM_P1 = ((0x48, 0x4A, 0x77), (0x4D, 0x65, 0xB4), (0x4D, 0x9B, 0xE6), (0x8F, 0xD3, 0xFF))
TEAM_P2 = ((0x83, 0x1C, 0x5D), (0xC3, 0x24, 0x54), (0xF0, 0x4F, 0x78), (0xED, 0x80, 0x99))

PUNY_TEAM = [c for ramp in (TEAM_P1, TEAM_P2) for c in ramp]

# Keyed by palette identity as well as colour: one shared dict would let
# whichever generator ran first decide the answer for the other.
_cache = {}


def nearest(rgb, palette=PUNY_PALETTE):
    """Nearest colour in `palette` to an arbitrary (r, g, b[, a]) tuple, by
    squared Euclidean RGB distance. Alpha (if present) passes through
    unchanged. Defaults to the base set, so every existing caller is
    unaffected."""
    key = (id(palette), rgb[0], rgb[1], rgb[2])
    hit = _cache.get(key)
    if hit is not None:
        return hit
    best, best_d = palette[0], None
    for cand in palette:
        d = (cand[0] - key[1]) ** 2 + (cand[1] - key[2]) ** 2 + (cand[2] - key[3]) ** 2
        if best_d is None or d < best_d:
            best, best_d = cand, d
    _cache[key] = best
    return best


def snap(rgba, palette=PUNY_PALETTE):
    """Snap a colour to its nearest match in `palette`, preserving alpha (or
    defaulting to opaque for a bare 3-tuple)."""
    r, g, b = nearest(rgba, palette)
    a = rgba[3] if len(rgba) > 3 else 255
    return (r, g, b, a)
