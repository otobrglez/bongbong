"""Retint the Puny World ground tileset for the live theme.

The live game PNG is a *retinted copy* of the third-party pack, generated
from the pristine original every time this runs. The theme decides what the
field is painted in:

  desert  the pack's yellow-green grass fill becomes pale, pebbly dust, its
          dirt paths a darker packed-earth road, and its sand a slightly
          darker, wind-smoothed hardpan that ground.rs scatters in soft
          patches (`Material::Sand`). Wood, roofs, water and greys are
          untouched.
  meadow  the de-green pass: grass hue-shifted toward the pack's own deeper
          tree-canopy green (#85A643 -> #619541) and slightly darkened,
          yellow dirt desaturated toward earth-tan (#C4B253 -> #B1A567).

Select with BONGBONG_THEME=desert|meadow; `desert` is the default. The same
variable drives tools/spritegen/gen_grass.py, so one setting regenerates a
matching floor and tall grass.

Two mechanisms, applied in this order:

1. **An exact colour table** for the handful of pack colours the game
   actually draws - the nine grass-fill and dirt-path tiles plus the sand
   corner set (`src/ground.rs`). A table because hue alone cannot separate
   the pack's materials: its sand (#C9B266, hue 46) sits 4 degrees from its
   dirt (#C4B253, hue 50) and the two *share* dither colours at their
   grass edges, so a curve that sends dirt dark and sand light would tear
   every dither in half. The table also lets the desert keep a speck a
   little lighter than the dust around it, where a proportional value
   multiplier would flatten the difference.
2. **A smooth piecewise-linear HSV curve** for everything else in the hue
   band - so the rest of the sheet (never drawn, but kept coherent for
   anyone browsing it) moves with the field instead of staying green next
   to it. Continuous, so pixel-art dithers between materials keep
   transitioning smoothly - no banding at a classification boundary.

Desert dithers: the pack's grass->khaki transition pixels are used both at
road edges and at sand edges, so they are mapped *between the dust and the
hardpan*, which also puts them between the dust and the (darker) road. A
road edge in the desert therefore steps a little harder than a meadow one;
a lighter hardpan would instead have given every patch a dark rim.

IDEMPOTENT BY CONSTRUCTION: always reads
static/punyworld/_original/punyworld-overworld-tileset.png and writes the
live game path static/punyworld/punyworld-overworld-tileset.png. Running it
twice produces the same output; tweak a theme below and rerun to iterate.
Run from the repo root:

  nix-shell -p "python3.withPackages (ps: [ps.pillow])" \
      --run "python3 tools/retint_ground.py"
"""

import colorsys
import os

from PIL import Image

SRC = "static/punyworld/_original/punyworld-overworld-tileset.png"
DST = "static/punyworld/punyworld-overworld-tileset.png"

THEME = os.environ.get("BONGBONG_THEME", "desert")


def hx(s):
    return tuple(int(s[i:i + 2], 16) for i in (1, 3, 5))


# A theme is a curve of (hue_deg, hue_shift_deg, sat_mul, val_mul) control
# points - linear interpolation between them, identity outside the first
# and last - and a table of exact overrides consulted first.
THEMES = {
    "meadow": {
        "curve": [
            (40.0, 0.0, 1.00, 1.00),
            (50.0, 0.0, 0.72, 0.90),   # dirt core: strongly muted, a touch darker
            (62.0, 8.0, 0.85, 0.92),   # dirt->grass dither zone
            (80.0, 17.0, 0.95, 0.90),  # grass core: pushed to ~hue 97, deepened
            (95.0, 10.0, 1.00, 0.95),
            (110.0, 0.0, 1.00, 1.00),
        ],
        "table": {},
    },
    "desert": {
        # The grass band (66-92) is one flat plateau so anything the table
        # does not name keeps its relative shading when it turns to dust;
        # the ramp in from 40 walks the dirt band into the same register.
        "curve": [
            (40.0, 0.0, 1.00, 1.00),
            (50.0, -22.0, 0.70, 0.80),
            (62.0, -32.0, 0.60, 1.00),
            (66.0, -42.0, 0.55, 1.20),
            (92.0, -42.0, 0.55, 1.20),
            (110.0, 0.0, 1.00, 1.00),
        ],
        "table": {
            # grass fill -> dust: the flat base and its three speck tones,
            # kept a step lighter/darker than the base so the specks still
            # read as grains and pebbles.
            hx("#85A643"): hx("#CCB385"),
            hx("#9FB747"): hx("#DAC59B"),
            hx("#96B146"): hx("#D3BC90"),
            hx("#7E9E3F"): hx("#BAA277"),
            # dirt paths -> packed-earth road, dark enough to read under
            # the wall shading and as a track across the open.
            hx("#C4B253"): hx("#A08058"),
            hx("#C0AB4A"): hx("#A78761"),
            # the dithers both dirt and sand share at their grass edge:
            # between the dust and the hardpan (see the module doc).
            hx("#B7A248"): hx("#B69C72"),
            hx("#B8AF49"): hx("#BFA57A"),
            hx("#9DA747"): hx("#C6AD80"),
            # sand -> hardpan: the smoother, slightly darker tone
            # ground.rs paints in soft patches.
            hx("#C9B266"): hx("#C2A87D"),
            hx("#C6AD5A"): hx("#BEA478"),
        },
    },
}

# Leave near-greys and very dark pixels alone (outlines, shadows).
MIN_SAT = 0.18
MIN_VAL = 0.25


def curve_at(curve, h):
    if h <= curve[0][0] or h >= curve[-1][0]:
        return 0.0, 1.0, 1.0
    for (h0, s0, sm0, vm0), (h1, s1, sm1, vm1) in zip(curve, curve[1:]):
        if h0 <= h <= h1:
            t = (h - h0) / (h1 - h0)
            return (
                s0 + (s1 - s0) * t,
                sm0 + (sm1 - sm0) * t,
                vm0 + (vm1 - vm0) * t,
            )
    return 0.0, 1.0, 1.0


def retint(rgb, theme):
    hit = theme["table"].get(rgb)
    if hit is not None:
        return hit
    r, g, b = [c / 255.0 for c in rgb]
    h, s, v = colorsys.rgb_to_hsv(r, g, b)
    hd = h * 360.0
    if s < MIN_SAT or v < MIN_VAL:
        return rgb
    shift, sat_mul, val_mul = curve_at(theme["curve"], hd)
    if shift == 0.0 and sat_mul == 1.0 and val_mul == 1.0:
        return rgb
    h2 = ((hd + shift) % 360.0) / 360.0
    r2, g2, b2 = colorsys.hsv_to_rgb(h2, min(1.0, s * sat_mul), min(1.0, v * val_mul))
    return (round(r2 * 255), round(g2 * 255), round(b2 * 255))


def main():
    theme = THEMES[THEME]
    img = Image.open(SRC).convert("RGBA")
    px = img.load()
    cache = {}
    for y in range(img.height):
        for x in range(img.width):
            r, g, b, a = px[x, y]
            if a == 0:
                continue
            key = (r, g, b)
            out = cache.get(key)
            if out is None:
                out = retint(key, theme)
                cache[key] = out
            px[x, y] = (out[0], out[1], out[2], a)
    os.makedirs(os.path.dirname(DST), exist_ok=True)
    img.save(DST)
    print(f"wrote {DST} ({img.width}x{img.height}) theme={THEME}")
    for name, rgb in [("grass", (0x85, 0xA6, 0x43)), ("dirt", (0xC4, 0xB2, 0x53)),
                      ("sand", (0xC9, 0xB2, 0x66)), ("tuft", (0x7E, 0x9E, 0x3F))]:
        print(f"  {name}: #{rgb[0]:02X}{rgb[1]:02X}{rgb[2]:02X} -> "
              f"#{'%02X%02X%02X' % retint(rgb, theme)}")


if __name__ == "__main__":
    main()
