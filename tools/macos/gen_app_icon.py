"""The macOS app icon: tools/macos/AppIcon.png, which tools/macos/bundle.sh
turns into the bundle's AppIcon.icns.

The iOS icon's picture (tools/ios/gen_app_icon.py: a player-one tank on the
Puny World grass fill, blown up by whole multiples so every sprite pixel
stays a crisp square) on Apple's macOS icon grid: macOS does not mask an
app's icon the way iOS does, so the picture is drawn as an 832 px rounded
square inside the 1024 px canvas, with the soft shadow under it that the
system's own icons carry.

    nix-shell -p "python3.withPackages (ps: [ps.pillow])" \
        --run "python3 tools/macos/gen_app_icon.py [--row N] [--out PATH]"
"""

import argparse
import os
import sys

from PIL import Image, ImageChops, ImageDraw, ImageFilter

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "ios"))
import gen_app_icon as ios  # noqa: E402

ROOT = os.path.normpath(ios.ROOT)
SIZE = 1024
BODY = 832  # the rounded square, on Apple's grid
RADIUS = 186  # its corners, about 22.4 % of the body
GROUND_SCALE = 6  # 16 px -> 96 px tiles
SHADOW_ALPHA = 80
SHADOW_BLUR = 14
SHADOW_DROP = 10


def grass(tileset):
    out = Image.new("RGBA", (BODY, BODY))
    step = ios.GROUND_TILE * GROUND_SCALE
    for ty in range(-(-BODY // step)):
        for tx in range(-(-BODY // step)):
            i = ios.GRASS_FILL[(tx * 7 + ty * 13 + tx * ty) % len(ios.GRASS_FILL)]
            sx, sy = (i % ios.GROUND_COLS) * ios.GROUND_TILE, (i // ios.GROUND_COLS) * ios.GROUND_TILE
            tile = tileset.crop((sx, sy, sx + ios.GROUND_TILE, sy + ios.GROUND_TILE))
            out.paste(tile.resize((step, step), Image.NEAREST), (tx * step, ty * step))
    return out


def tank(sheet, row):
    y = (ios.PLAYER_ONE_BLOCK + row) * ios.TANK_CELL
    hull = sheet.crop((0, y, ios.TANK_CELL, y + ios.TANK_CELL))
    turret = sheet.crop((ios.TURRET_COL * ios.TANK_CELL, y, (ios.TURRET_COL + 1) * ios.TANK_CELL, y + ios.TANK_CELL))
    hull.alpha_composite(turret)
    hull = hull.crop(hull.getbbox())
    scale = int(BODY * ios.TANK_FILL) // max(hull.size)
    return hull.resize((hull.width * scale, hull.height * scale), Image.NEAREST), scale


def rounded(size, radius):
    # Drawn four times larger and reduced, so the corners are smooth.
    big = Image.new("L", (size * 4, size * 4), 0)
    ImageDraw.Draw(big).rounded_rectangle((0, 0, size * 4 - 1, size * 4 - 1), radius * 4, fill=255)
    return big.resize((size, size), Image.LANCZOS)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--row", type=int, default=1)
    ap.add_argument("--out", default=os.path.join(ROOT, "tools/macos/AppIcon.png"))
    args = ap.parse_args()
    sheet = Image.open(os.path.join(ROOT, "static/scifi_tanks_sheet.png")).convert("RGBA")
    tileset = Image.open(os.path.join(ROOT, "static/punyworld/punyworld-overworld-tileset.png")).convert("RGBA")

    body = grass(tileset)
    sprite, scale = tank(sheet, args.row)
    x, y = (BODY - sprite.width) // 2, (BODY - sprite.height) // 2
    shadow = Image.new("RGBA", sprite.size, ios.SHADOW)
    shadow.putalpha(sprite.getchannel("A").point(lambda a: ios.SHADOW[3] if a else 0))
    body.alpha_composite(shadow, (x + scale, y + scale))
    body.alpha_composite(sprite, (x, y))
    mask = rounded(BODY, RADIUS)
    body.putalpha(ImageChops.multiply(body.getchannel("A"), mask))

    at = (SIZE - BODY) // 2
    icon = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    drop = Image.new("L", (SIZE, SIZE), 0)
    drop.paste(mask.point(lambda a: a * SHADOW_ALPHA // 255), (at, at + SHADOW_DROP))
    icon.putalpha(drop.filter(ImageFilter.GaussianBlur(SHADOW_BLUR)))
    icon.alpha_composite(body, (at, at))
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    icon.save(args.out)
    print(args.out)


if __name__ == "__main__":
    main()
