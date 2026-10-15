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
"""

import argparse
import os
import subprocess
import sys
import tempfile

from PIL import Image

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
# Sheet rows: the first chassis of each team block (src/lib.rs
# `TANK_ROWS_PER_TEAM`), the enemy first, then players one to four.
TANKS = [("enemy", 0), ("p1", 12), ("p2", 24), ("p3", 36), ("p4", 48)]

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


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--mapshot", help="a built mapshot binary (default: cargo run --release --bin mapshot)")
    args = parser.parse_args()
    os.makedirs(OUT, exist_ok=True)

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
