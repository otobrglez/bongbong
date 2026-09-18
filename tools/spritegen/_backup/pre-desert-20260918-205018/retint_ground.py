"""Retint the Puny World ground tileset: deeper grass, muted dirt.

The pack's own grass fill (#85A643, hue ~80) is a yellow-green and its
dirt-path tiles (#C4B253, hue ~50) are bright yellow-khaki -- together they
made the whole battlefield read yellow-green even after the sprite palette
was de-olived (docs/PALETTE.md "The de-green pass"). This script applies a
smooth, hue-based HSV curve to the tileset:

  - yellow dirt (hue ~40-55): desaturated and darkened toward earth-tan
  - yellow-green grass (hue ~60-95): hue-shifted toward the pack's OWN
    deeper tree-canopy green (#5E914B, hue ~100) and slightly darkened
  - everything else (wood/gold below 40, true green above 110, teal, red,
    neutral greys, water): untouched

The curve is piecewise-linear and continuous, so the pixel-art dither
between grass and dirt keeps transitioning smoothly -- no banding at the
classification boundary.

IDEMPOTENT BY CONSTRUCTION: always reads the pristine original at
static/punyworld/_original/punyworld-overworld-tileset.png and writes the
live game path static/punyworld/punyworld-overworld-tileset.png. Running it
twice produces the same output; tweak the control points below and rerun to
iterate. Run from the repo root:

  nix-shell -p "python3.withPackages (ps: [ps.pillow])" \
      --run "python3 tools/retint_ground.py"
"""

import colorsys
import os

from PIL import Image

SRC = "static/punyworld/_original/punyworld-overworld-tileset.png"
DST = "static/punyworld/punyworld-overworld-tileset.png"

# (hue_deg, hue_shift_deg, sat_mul, val_mul) control points; linear
# interpolation between them, identity outside the first/last point.
CURVE = [
    (40.0, 0.0, 1.00, 1.00),
    (50.0, 0.0, 0.72, 0.90),   # dirt core: strongly muted, a touch darker
    (62.0, 8.0, 0.85, 0.92),   # dirt->grass dither zone
    (80.0, 17.0, 0.95, 0.90),  # grass core: pushed to ~hue 97, deepened
    (95.0, 10.0, 1.00, 0.95),
    (110.0, 0.0, 1.00, 1.00),
]

# Leave near-greys and very dark pixels alone (outlines, shadows).
MIN_SAT = 0.18
MIN_VAL = 0.25


def curve_at(h):
    if h <= CURVE[0][0] or h >= CURVE[-1][0]:
        return 0.0, 1.0, 1.0
    for (h0, s0, sm0, vm0), (h1, s1, sm1, vm1) in zip(CURVE, CURVE[1:]):
        if h0 <= h <= h1:
            t = (h - h0) / (h1 - h0)
            return (
                s0 + (s1 - s0) * t,
                sm0 + (sm1 - sm0) * t,
                vm0 + (vm1 - vm0) * t,
            )
    return 0.0, 1.0, 1.0


def retint(rgb):
    r, g, b = [c / 255.0 for c in rgb]
    h, s, v = colorsys.rgb_to_hsv(r, g, b)
    hd = h * 360.0
    if s < MIN_SAT or v < MIN_VAL:
        return rgb
    shift, sat_mul, val_mul = curve_at(hd)
    if shift == 0.0 and sat_mul == 1.0 and val_mul == 1.0:
        return rgb
    h2 = ((hd + shift) % 360.0) / 360.0
    r2, g2, b2 = colorsys.hsv_to_rgb(h2, min(1.0, s * sat_mul), min(1.0, v * val_mul))
    return (round(r2 * 255), round(g2 * 255), round(b2 * 255))


def main():
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
                out = retint(key)
                cache[key] = out
            px[x, y] = (out[0], out[1], out[2], a)
    os.makedirs(os.path.dirname(DST), exist_ok=True)
    img.save(DST)
    print(f"wrote {DST} ({img.width}x{img.height})")
    for name, rgb in [("grass", (0x85, 0xA6, 0x43)), ("dirt", (0xC4, 0xB2, 0x53)),
                      ("path-sand", (0xC9, 0xB2, 0x66)), ("tuft", (0x7E, 0x9E, 0x3F))]:
        print(f"  {name}: #{rgb[0]:02X}{rgb[1]:02X}{rgb[2]:02X} -> "
              f"#{'%02X%02X%02X' % retint(rgb)}")


if __name__ == "__main__":
    main()
