"""The App Store icon: tools/ios/Assets.xcassets/AppIcon.appiconset/icon-1024.png.

A player-one tank from static/scifi_tanks_sheet.png (hull column 0 with the
turret, column 1, drawn over it) on the Puny World grass fill, both blown up
by whole multiples so every sprite pixel stays a crisp square. The App Store
refuses an icon with an alpha channel, so the result is RGB.

    nix-shell -p "python3.withPackages (ps: [ps.pillow])" \
        --run "python3 tools/ios/gen_app_icon.py [--row N] [--out PATH]"

--row picks the chassis in sheet-row order within the player-one block
(0-11, `TankKind` order); the default is the one the icon ships with.
"""

import argparse
import os

from PIL import Image

ROOT = os.path.join(os.path.dirname(__file__), "..", "..")
SIZE = 1024
TANK_CELL = 32
TANK_FILL = 0.8  # the tallest the cropped sprite may stand, of the icon
PLAYER_ONE_BLOCK = 12  # first row of the player-one team block
GROUND_TILE = 16
GROUND_SCALE = 8  # 16 px -> 128 px, eight tiles across
GROUND_COLS = 27
# src/ground.rs GRASS_FILL: the grass tiles the floor picks from by hash.
GRASS_FILL = [0, 1, 2, 27, 28, 29, 54, 55, 56]
SHADOW = (0x25, 0x25, 0x25, 96)  # punypalette BLACK, the game's hull shadow tone


def grass(tileset):
    out = Image.new("RGBA", (SIZE, SIZE))
    step = GROUND_TILE * GROUND_SCALE
    for ty in range(SIZE // step):
        for tx in range(SIZE // step):
            i = GRASS_FILL[(tx * 7 + ty * 13 + tx * ty) % len(GRASS_FILL)]
            sx, sy = (i % GROUND_COLS) * GROUND_TILE, (i // GROUND_COLS) * GROUND_TILE
            tile = tileset.crop((sx, sy, sx + GROUND_TILE, sy + GROUND_TILE))
            out.paste(tile.resize((step, step), Image.NEAREST), (tx * step, ty * step))
    return out


def tank(sheet, row):
    y = (PLAYER_ONE_BLOCK + row) * TANK_CELL
    hull = sheet.crop((0, y, TANK_CELL, y + TANK_CELL))
    turret = sheet.crop((TANK_CELL, y, 2 * TANK_CELL, y + TANK_CELL))
    hull.alpha_composite(turret)
    hull = hull.crop(hull.getbbox())
    scale = int(SIZE * TANK_FILL) // max(hull.size)
    return hull.resize((hull.width * scale, hull.height * scale), Image.NEAREST), scale


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--row", type=int, default=1)
    ap.add_argument("--out", default=os.path.join(
        ROOT, "tools/ios/Assets.xcassets/AppIcon.appiconset/icon-1024.png"))
    args = ap.parse_args()
    sheet = Image.open(os.path.join(ROOT, "static/scifi_tanks_sheet.png")).convert("RGBA")
    tileset = Image.open(os.path.join(ROOT, "static/punyworld/punyworld-overworld-tileset.png")).convert("RGBA")

    icon = grass(tileset)
    sprite, scale = tank(sheet, args.row)
    x, y = (SIZE - sprite.width) // 2, (SIZE - sprite.height) // 2
    shadow = Image.new("RGBA", sprite.size, SHADOW)
    shadow.putalpha(sprite.getchannel("A").point(lambda a: SHADOW[3] if a else 0))
    icon.alpha_composite(shadow, (x + scale, y + scale))  # one sprite pixel down-right
    icon.alpha_composite(sprite, (x, y))
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    icon.convert("RGB").save(args.out)
    print(args.out)


if __name__ == "__main__":
    main()
