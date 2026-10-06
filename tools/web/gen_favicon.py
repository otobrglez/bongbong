"""The site's favicons, the game's app icon at a tab's size: site/public/
favicon.svg, favicon.ico (16, 32, 48) and apple-touch-icon.png (180).

The tank is the iOS and macOS icons' (tools/ios/gen_app_icon.py: player one's
chassis from static/scifi_tanks_sheet.png, hull and turret at rest). The SVG
and the ico draw it on a rounded square of the grass fill's own green - the
grass's texture is noise at 16 px - and the SVG draws it as one rect per run
of sprite pixels, so it stays crisp at any size. The apple-touch icon is the
iOS icon itself, grass and all, scaled down (iOS rounds its corners).

    nix-shell -p "python3.withPackages (ps: [ps.pillow])" \
        --run "python3 tools/web/gen_favicon.py [--row N]"
"""

import argparse
import os
import sys

from PIL import Image, ImageChops, ImageDraw

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "ios"))
import gen_app_icon as ios  # noqa: E402

ROOT = os.path.normpath(ios.ROOT)
OUT = os.path.join(ROOT, "site", "public")
TANK_FILL = 0.84  # a tab icon has no room to spare round the tank
RADIUS = 0.18  # the rounded square's corners, of its side
ICO_SIZES = (16, 32, 48)
TOUCH_SIZE = 180


def sprite(row):
    sheet = Image.open(os.path.join(ROOT, "static/scifi_tanks_sheet.png")).convert("RGBA")
    y = (ios.PLAYER_ONE_BLOCK + row) * ios.TANK_CELL
    hull = sheet.crop((0, y, ios.TANK_CELL, y + ios.TANK_CELL))
    turret = sheet.crop((ios.TURRET_COL * ios.TANK_CELL, y, (ios.TURRET_COL + 1) * ios.TANK_CELL, y + ios.TANK_CELL))
    hull.alpha_composite(turret)
    return hull.crop(hull.getbbox())


def grass_green():
    """The grass fill's commonest colour."""
    tileset = Image.open(os.path.join(ROOT, "static/punyworld/punyworld-overworld-tileset.png")).convert("RGB")
    i = ios.GRASS_FILL[0]
    sx, sy = (i % ios.GROUND_COLS) * ios.GROUND_TILE, (i // ios.GROUND_COLS) * ios.GROUND_TILE
    tile = tileset.crop((sx, sy, sx + ios.GROUND_TILE, sy + ios.GROUND_TILE))
    return max(tile.getcolors())[1]


def grid(tank):
    """The icon's side in sprite pixels and the tank's corner on it."""
    side = round(max(tank.size) / TANK_FILL)
    return side, ((side - tank.width) // 2, (side - tank.height) // 2)


def hexa(c):
    return "#%02x%02x%02x" % c[:3]


def svg(tank, green):
    side, (x0, y0) = grid(tank)
    shadow_alpha = ios.SHADOW[3] / 255
    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {side} {side}" shape-rendering="crispEdges">',
        f'<rect width="{side}" height="{side}" rx="{side * RADIUS:.1f}" fill="{hexa(green)}"/>',
    ]
    runs = []  # (colour, opacity, x, y, w)
    px = tank.load()
    for y in range(tank.height):
        x = 0
        while x < tank.width:
            c = px[x, y]
            if c[3] == 0:
                x += 1
                continue
            w = 1
            while x + w < tank.width and px[x + w, y] == c:
                w += 1
            runs.append((c, x, y, w))
            x += w
    # The shadow, one sprite pixel down-right, as the app icons cast it.
    out.append(f'<g fill="{hexa(ios.SHADOW)}" fill-opacity="{shadow_alpha:.3f}">')
    for y in range(tank.height):
        x = 0
        while x < tank.width:
            if px[x, y][3] == 0:
                x += 1
                continue
            w = 1
            while x + w < tank.width and px[x + w, y][3]:
                w += 1
            out.append(f'<rect x="{x0 + x + 1}" y="{y0 + y + 1}" width="{w}" height="1"/>')
            x += w
    out.append("</g>")
    for c, x, y, w in runs:
        extra = "" if c[3] == 255 else f' fill-opacity="{c[3] / 255:.3f}"'
        out.append(f'<rect x="{x0 + x}" y="{y0 + y}" width="{w}" height="1" fill="{hexa(c)}"{extra}/>')
    out.append("</svg>")
    return "\n".join(out) + "\n"


def raster(tank, green, scale):
    """The SVG's picture at `scale` px a sprite pixel."""
    side, (x0, y0) = grid(tank)
    size = side * scale
    icon = Image.new("RGBA", (size, size), green + (255,))
    big = tank.resize((tank.width * scale, tank.height * scale), Image.NEAREST)
    shadow = Image.new("RGBA", big.size, ios.SHADOW)
    shadow.putalpha(big.getchannel("A").point(lambda a: ios.SHADOW[3] if a else 0))
    icon.alpha_composite(shadow, ((x0 + 1) * scale, (y0 + 1) * scale))
    icon.alpha_composite(big, (x0 * scale, y0 * scale))
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, size - 1, size - 1), round(size * RADIUS), fill=255)
    icon.putalpha(ImageChops.multiply(icon.getchannel("A"), mask))
    return icon


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--row", type=int, default=1)
    args = ap.parse_args()
    tank = sprite(args.row)
    green = grass_green()

    path = os.path.join(OUT, "favicon.svg")
    with open(path, "w") as f:
        f.write(svg(tank, green))
    print(path)

    big = raster(tank, green, 16)
    path = os.path.join(OUT, "favicon.ico")
    big.save(path, sizes=[(s, s) for s in ICO_SIZES])
    print(path)

    ios_icon = os.path.join(ROOT, "tools/ios/Assets.xcassets/AppIcon.appiconset/icon-1024.png")
    path = os.path.join(OUT, "apple-touch-icon.png")
    Image.open(ios_icon).convert("RGB").resize((TOUCH_SIZE, TOUCH_SIZE), Image.LANCZOS).save(path, optimize=True)
    print(path)


if __name__ == "__main__":
    main()
