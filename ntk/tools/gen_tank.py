"""Generate the NTK tank sprites.

  ntk/assets/tank.png       4 frames of 32 x 32, side by side (128 x 32), green.
  ntk/assets/tank-big.png   the same sheet scaled to 2000 px wide (slides).
  ntk/assets/more-tanks.png the big sheet with a pink and a blue tank as
                            extra rows (2000 x 1500).

Each frame faces up. Only the track stripes differ between frames: they
shift down by one row per frame, so cycling 0-1-2-3 while moving reads as
the tracks rolling. Drawn at 1x; the demo scales it up when drawing.

    nix-shell -p "python3.withPackages (ps: [ps.pillow])" --run "python3 ntk/tools/gen_tank.py"
"""
from PIL import Image, ImageDraw

FRAME = 32
FRAMES = 4
BIG_WIDTH = 2000

BARREL = (50, 50, 50)
TRACK = (40, 40, 40)
TREAD = (120, 120, 120)

# (hull, hull outline, turret) per colour.
PALETTES = {
    "green": ((86, 112, 60), (60, 80, 42), (110, 140, 76)),
    "pink": ((214, 84, 150), (150, 50, 105), (240, 130, 185)),
    "blue": ((70, 120, 200), (45, 80, 145), (110, 160, 230)),
}


def draw_sheet(palette):
    hull, hull_dark, turret = palette
    sheet = Image.new("RGBA", (FRAME * FRAMES, FRAME), (0, 0, 0, 0))
    for frame in range(FRAMES):
        img = Image.new("RGBA", (FRAME, FRAME), (0, 0, 0, 0))
        d = ImageDraw.Draw(img)

        # Two tracks: 6 px wide, full height minus a margin.
        for x0 in (2, 24):
            d.rectangle([x0, 2, x0 + 5, 29], fill=TRACK)
            # Tread stripes every 4 px, offset by the frame number.
            for y in range(2 - 4 + frame, 30, 4):
                if 2 <= y <= 29:
                    d.line([(x0, y), (x0 + 5, y)], fill=TREAD)

        # Hull between the tracks, turret on top, barrel pointing up.
        d.rectangle([8, 4, 23, 27], fill=hull, outline=hull_dark)
        d.rectangle([11, 11, 20, 22], fill=turret, outline=hull_dark)
        d.rectangle([14, 0, 17, 12], fill=BARREL)

        sheet.paste(img, (frame * FRAME, 0))
    return sheet


def big(sheet):
    w, h = sheet.size
    return sheet.resize((BIG_WIDTH, BIG_WIDTH * h // w), Image.NEAREST)


green = draw_sheet(PALETTES["green"])
green.save("ntk/assets/tank.png")
big(green).save("ntk/assets/tank-big.png")

rows = [big(draw_sheet(PALETTES[c])) for c in ("green", "pink", "blue")]
more = Image.new("RGBA", (BIG_WIDTH, sum(r.size[1] for r in rows)), (0, 0, 0, 0))
y = 0
for row in rows:
    more.paste(row, (0, y))
    y += row.size[1]
more.save("ntk/assets/more-tanks.png")
print("wrote tank.png, tank-big.png, more-tanks.png", more.size)
