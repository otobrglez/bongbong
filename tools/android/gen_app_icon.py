"""The Android launcher icon: tools/android/res/mipmap-*/ic_launcher_*.png.

An adaptive icon (the only kind a minSdk 29 app needs): a background layer
of the Puny World grass fill and a foreground layer with a player-one tank
from static/scifi_tanks_sheet.png (hull column 0 with the turret at rest,
column 20, drawn over it - docs/SPRITESHEET_SPEC.md) - the same picture as
the iOS icon (tools/ios/gen_app_icon.py), split in two so the launcher can
mask and parallax it. Both layers are 108 dp
and the launcher shows the middle 72 dp at most, so the tank stands inside
that circle. tools/android/res/mipmap-anydpi-v26/ic_launcher.xml names the
layers; tools/android/package.sh compiles the whole res/ tree into the APK.

    nix-shell -p "python3.withPackages (ps: [ps.pillow])" \
        --run "python3 tools/android/gen_app_icon.py [--row N]"

The master is drawn at xxxhdpi (432 px) by whole multiples so every sprite
pixel is a crisp square there; the lower densities are area-averaged from
it, since no integer scale lands on 324, 216, 162 or 108.
"""

import argparse
import os

from PIL import Image

ROOT = os.path.join(os.path.dirname(__file__), "..", "..")
RES = os.path.join(ROOT, "tools/android/res")
# Adaptive icon layers are 108 dp; one entry per density bucket.
DENSITIES = {"mdpi": 108, "hdpi": 162, "xhdpi": 216, "xxhdpi": 324, "xxxhdpi": 432}
MASTER = DENSITIES["xxxhdpi"]
TANK_CELL = 40
TURRET_COL = 20
TANK_FILL = 0.5  # the tallest the cropped sprite may stand, of the layer (safe zone is 0.667)
PLAYER_ONE_BLOCK = 12  # first row of the player-one team block
GROUND_TILE = 16
GROUND_SCALE = 9  # 16 px -> 144 px, three tiles across the 432 px master
GROUND_COLS = 27
# src/ground.rs GRASS_FILL: the grass tiles the floor picks from by hash.
GRASS_FILL = [0, 1, 2, 27, 28, 29, 54, 55, 56]
SHADOW = (0x25, 0x25, 0x25, 96)  # punypalette BLACK, the game's hull shadow tone


def grass(tileset):
    out = Image.new("RGBA", (MASTER, MASTER))
    step = GROUND_TILE * GROUND_SCALE
    for ty in range(MASTER // step):
        for tx in range(MASTER // step):
            i = GRASS_FILL[(tx * 7 + ty * 13 + tx * ty) % len(GRASS_FILL)]
            sx, sy = (i % GROUND_COLS) * GROUND_TILE, (i // GROUND_COLS) * GROUND_TILE
            tile = tileset.crop((sx, sy, sx + GROUND_TILE, sy + GROUND_TILE))
            out.paste(tile.resize((step, step), Image.NEAREST), (tx * step, ty * step))
    return out


def tank(sheet, row):
    y = (PLAYER_ONE_BLOCK + row) * TANK_CELL
    hull = sheet.crop((0, y, TANK_CELL, y + TANK_CELL))
    turret = sheet.crop((TURRET_COL * TANK_CELL, y, (TURRET_COL + 1) * TANK_CELL, y + TANK_CELL))
    hull.alpha_composite(turret)
    hull = hull.crop(hull.getbbox())
    scale = int(MASTER * TANK_FILL) // max(hull.size)
    return hull.resize((hull.width * scale, hull.height * scale), Image.NEAREST), scale


def foreground(sheet, row):
    layer = Image.new("RGBA", (MASTER, MASTER), (0, 0, 0, 0))
    sprite, scale = tank(sheet, row)
    x, y = (MASTER - sprite.width) // 2, (MASTER - sprite.height) // 2
    shadow = Image.new("RGBA", sprite.size, SHADOW)
    shadow.putalpha(sprite.getchannel("A").point(lambda a: SHADOW[3] if a else 0))
    layer.alpha_composite(shadow, (x + scale, y + scale))  # one sprite pixel down-right
    layer.alpha_composite(sprite, (x, y))
    return layer


def write(layer, name):
    for density, size in DENSITIES.items():
        out_dir = os.path.join(RES, f"mipmap-{density}")
        os.makedirs(out_dir, exist_ok=True)
        img = layer if size == MASTER else layer.resize((size, size), Image.BOX)
        path = os.path.join(out_dir, f"{name}.png")
        img.save(path, optimize=True)
        print(os.path.relpath(path, ROOT))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--row", type=int, default=1)
    args = ap.parse_args()
    sheet = Image.open(os.path.join(ROOT, "static/scifi_tanks_sheet.png")).convert("RGBA")
    tileset = Image.open(os.path.join(ROOT, "static/punyworld/punyworld-overworld-tileset.png")).convert("RGBA")
    write(grass(tileset).convert("RGB"), "ic_launcher_background")
    write(foreground(sheet, args.row), "ic_launcher_foreground")


if __name__ == "__main__":
    main()
