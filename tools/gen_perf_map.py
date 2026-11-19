"""Write maps/perf/dense-250.toml: the largest field the map format allows,
built up from edge to edge, for timing what a big map costs.

  python3 tools/gen_perf_map.py [-o maps/perf/dense-250.toml]

250 x 250 cells of town: 31 x 31 blocks of 6 x 6 cells with streets two
cells wide between them (the narrowest corridor the router takes), a road
down one side of every street. A block is a building (a brick, wood, glass
or iron shell with two doors), a wood (four clumps of trees or pines), a
pond, a field of tall grass or a fenced yard with sandbags and drums; the
block in the middle is left open for the two starts. Which block is which
comes from a fixed seed, so the file is the same every time.

It is a measuring fixture, not a level: `just perf-field` times
`Game::update` on it. It sits in maps/perf/ so that neither
`just probe-fixtures` (maps/test/*.toml) nor `just probe-fields` (a list
of names) holds it to a ceiling.
"""

import argparse
import random

SIZE = 250
BLOCK = 6
STREET = 2
PERIOD = BLOCK + STREET
BLOCKS = (SIZE - 1) // PERIOD
SEED = 157

HEADER = """\
# dense-250 - the largest field the format allows (250 x 250 cells), built
# up from edge to edge: a measuring fixture for what a big map costs the
# simulation and the painters, not a level. Written by
# tools/gen_perf_map.py - change the generator and run it again rather
# than editing this file; re-saving from the map editor would drop these
# comments.
#
# 31 x 31 blocks of 6 x 6 cells - buildings, woods, ponds, tall grass,
# fenced yards - with streets two cells wide between them and the middle
# block open for the starts. `just perf-field` times `Game::update` on it
# (docs/tooling.md). In maps/perf/ on purpose: `just probe-fixtures` and
# `just probe-fields` hold it to no ceiling.

version = 1
size = [250, 250]
tanks = 16
mission.kind = "destroy"
spawn.kind = "band"

"""


def block(rng, cells, x0, y0):
    kind = rng.choices(["building", "wood", "pond", "grass", "yard"], weights=[35, 25, 10, 15, 15])[0]
    last = BLOCK - 1
    ring = [(x, y) for y in range(BLOCK) for x in range(BLOCK) if x in (0, last) or y in (0, last)]
    if kind == "building":
        material = rng.choice(["brick", "brick", "wood", "glass", "iron"])
        # A door two cells wide in the north and the south wall.
        doors = {(2, 0), (3, 0), (2, last), (3, last)}
        for x, y in ring:
            if (x, y) not in doors:
                cells[(x0 + x, y0 + y)] = f'{{ kind = "wall", material = "{material}" }}'
    elif kind == "wood":
        tree = rng.choice(["tree", "pine"])
        for cx, cy in [(0, 0), (4, 0), (0, 4), (4, 4)]:
            for dx in range(2):
                for dy in range(2):
                    cells[(x0 + cx + dx, y0 + cy + dy)] = f'{{ kind = "{tree}" }}'
    elif kind == "pond":
        for y in range(BLOCK):
            for x in range(BLOCK):
                cells[(x0 + x, y0 + y)] = '{ kind = "water" }'
    elif kind == "grass":
        for y in range(BLOCK):
            for x in range(BLOCK):
                cells[(x0 + x, y0 + y)] = '{ kind = "tall_grass" }'
    else:
        gaps = {(2, 0), (3, 0), (2, last), (3, last), (0, 2), (0, 3), (last, 2), (last, 3)}
        for x, y in ring:
            if (x, y) not in gaps:
                cells[(x0 + x, y0 + y)] = '{ kind = "fence" }'
        cells[(x0 + 1, y0 + 1)] = '{ kind = "sandbag" }'
        cells[(x0 + 4, y0 + 4)] = '{ kind = "sandbag" }'
        cells[(x0 + 4, y0 + 1)] = '{ kind = "barrel", drum = "fuel" }'
        cells[(x0 + 1, y0 + 4)] = '{ kind = "barrel", drum = "oil" }'


def generate():
    rng = random.Random(SEED)
    cells = {}
    middle = BLOCKS // 2
    for by in range(BLOCKS):
        for bx in range(BLOCKS):
            x0, y0 = 1 + bx * PERIOD, 1 + by * PERIOD
            if (bx, by) == (middle, middle):
                cells[(x0 + 1, y0 + 2)] = '{ kind = "start" }'
                cells[(x0 + 4, y0 + 2)] = '{ kind = "start2" }'
                continue
            block(rng, cells, x0, y0)
    # A road down the first cell of every street.
    for b in range(BLOCKS):
        lane = 1 + b * PERIOD + BLOCK
        for i in range(SIZE):
            cells.setdefault((lane, i), '{ kind = "road" }')
            cells.setdefault((i, lane), '{ kind = "road" }')
    return cells


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("-o", "--out", default="maps/perf/dense-250.toml")
    args = parser.parse_args()
    cells = generate()
    with open(args.out, "w") as out:
        out.write(HEADER)
        for (x, y), cell in sorted(cells.items(), key=lambda item: (item[0][1], item[0][0])):
            out.write(f'cells."{x},{y}" = {cell}\n')
    print(f"{args.out}: {len(cells)} cells")


if __name__ == "__main__":
    main()
