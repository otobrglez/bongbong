"""The home page's pictures, into site/public/art/: three shipped maps as the
game draws them, the four players' tanks and an enemy, and the seven bongs -
all at the game's own pixels, which the page scales with `image-rendering:
pixelated`.

The maps come from `mapshot` (docs/mapshot-prd.md) on its CPU renderer and
default seed with two player tanks staged, so a rerun on any machine draws
the same field; the crops are cut from those. The sprites are cut from
static/scifi_tanks_sheet.png (hull and turret at rest, the layout in
src/lib.rs) and the first frame of static/bongs/<kind>/idle.png. Rerun after
the art or one of these maps changes.

    nix-shell -p "python3.withPackages (ps: [ps.pillow])" \
        --run "python3 tools/web/gen_site_art.py [--mapshot target/release/mapshot]"

With `--iphone BEZEL SCREEN` it also writes iphone.webp, the "Take it with
you" picture: a simulator screenshot of the game (`just run-ios-sim` with
BONGBONG_IOS_DEVICE on the newest iPhone, then `just ios-screenshot`) set into
Apple's product bezel for that iPhone, landscape (Bezel-iPhone-<N>.dmg from
developer.apple.com/design/resources, under Apple's Design Resources license;
the bezel itself never goes in the repo). Apple's marketing guidelines want
the latest model and the bezel as provided - only scaled, never cropped,
tilted or shadowed - with the app on its screen as it runs.
"""

import argparse
import os
import subprocess
import sys
import tempfile

from PIL import Image, ImageDraw

ROOT = os.path.normpath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(ROOT, "site", "public", "art")

# Each crop: the file it writes, the map it is cut from and its box on the
# map's 1:1 render, (x, y, width, height) in field pixels. The cards are
# 2:1 at about the size a card shows them; the bands are a page's width.
CROPS = [
    ("field-default", "default", (600, 420, 400, 200)),
    ("field-archipelago", "archipelago", (40, 180, 400, 200)),
    ("field-oasis", "oasis-bazaar", (330, 280, 400, 200)),
    ("turn-default", "default", (610, 440, 400, 170)),
    ("band-archipelago", "archipelago", (0, 325, 1792, 160)),
    ("band-oasis", "oasis-bazaar", (0, 300, 1536, 160)),
]

TANK_CELL = 40
TURRET_COL = 20
TANK_ROWS_PER_TEAM = 12  # src/lib.rs
# The chassis the app's icon draws (tools/ios/gen_app_icon.py's default
# --row), so the page's tanks are the logo's, in each team's colours: the
# enemy block first, then players one to four.
LOGO_CHASSIS = 1
TANKS = [(name, block * TANK_ROWS_PER_TEAM + LOGO_CHASSIS) for block, name in enumerate(["enemy", "p1", "p2", "p3", "p4"])]

BONG_CELL = 48
BONGS = ["blip", "capi", "fuzz", "gloop", "oggle", "puff", "spike"]


def render_maps(mapshot, out_dir):
    maps = sorted({m for _, m, _ in CROPS})
    paths = [os.path.join(ROOT, "maps", f"{m}.toml") for m in maps]
    command = [mapshot] if mapshot else ["cargo", "run", "--release", "--bin", "mapshot", "--"]
    command += ["--renderer", "cpu", "--players", "2", "--out-dir", out_dir] + paths
    subprocess.run(command, cwd=ROOT, check=True)
    return {m: Image.open(os.path.join(out_dir, f"{m}.png")).convert("RGBA") for m in maps}


def save(image, name, ext):
    path = os.path.join(OUT, f"{name}.{ext}")
    if ext == "webp":
        image.save(path, "WEBP", lossless=True, quality=100, method=6)
    else:
        image.save(path, "PNG", optimize=True)
    print(os.path.relpath(path, ROOT), image.size)


IPHONE_WIDTH = 1120  # twice the widest the page shows it, for a 2x screen


def iphone(bezel_path, screen_path):
    """The screenshot in the bezel's transparent screen, which is found by
    filling from the bezel's centre and clips the screenshot to its rounded
    corners, then scaled to `IPHONE_WIDTH`."""
    bezel = Image.open(bezel_path).convert("RGBA")
    opening = bezel.getchannel("A").point(lambda v: 255 if v < 8 else 0)
    ImageDraw.floodfill(opening, (bezel.width // 2, bezel.height // 2), 128)
    opening = opening.point(lambda v: 255 if v == 128 else 0)
    box = opening.getbbox()
    screen = Image.open(screen_path).convert("RGBA")
    size = (box[2] - box[0], box[3] - box[1])
    if screen.size != size:
        sys.exit(f"the screenshot is {screen.size[0]}x{screen.size[1]}, the bezel's screen {size[0]}x{size[1]}")
    picture = Image.new("RGBA", bezel.size)
    picture.paste(screen, box[:2], opening.crop(box))
    picture.alpha_composite(bezel)
    height = round(picture.height * IPHONE_WIDTH / picture.width)
    picture = picture.resize((IPHONE_WIDTH, height), Image.LANCZOS)
    path = os.path.join(OUT, "iphone.webp")
    picture.save(path, "WEBP", quality=90, method=6)
    print(os.path.relpath(path, ROOT), picture.size)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--mapshot", help="a built mapshot binary (default: cargo run --release --bin mapshot)")
    parser.add_argument("--iphone", nargs=2, metavar=("BEZEL", "SCREEN"),
                        help="also write iphone.webp: the landscape bezel PNG and a simulator screenshot")
    args = parser.parse_args()
    os.makedirs(OUT, exist_ok=True)
    if args.iphone:
        iphone(*args.iphone)

    with tempfile.TemporaryDirectory() as shots:
        fields = render_maps(args.mapshot, shots)
        for name, map_name, (x, y, w, h) in CROPS:
            field = fields[map_name]
            if x + w > field.width or y + h > field.height:
                sys.exit(f"{name}: the box runs off {map_name}'s {field.width}x{field.height} render")
            save(field.crop((x, y, x + w, y + h)).convert("RGB"), name, "webp")

    sheet = Image.open(os.path.join(ROOT, "static", "scifi_tanks_sheet.png")).convert("RGBA")
    for name, row in TANKS:
        y = row * TANK_CELL
        hull = sheet.crop((0, y, TANK_CELL, y + TANK_CELL))
        hull.alpha_composite(sheet.crop((TURRET_COL * TANK_CELL, y, (TURRET_COL + 1) * TANK_CELL, y + TANK_CELL)))
        save(hull, f"tank-{name}", "png")

    for kind in BONGS:
        idle = Image.open(os.path.join(ROOT, "static", "bongs", kind, "idle.png")).convert("RGBA")
        save(idle.crop((0, 0, BONG_CELL, BONG_CELL)), f"bong-{kind}", "png")


if __name__ == "__main__":
    main()
