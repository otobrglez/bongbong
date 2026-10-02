#!/usr/bin/env python3
"""Scale a map's field by a factor, keeping its shape and its routes.

    python3 tools/scale_map.py maps/x.toml [--factor 1.5] [-o out.toml]

A plain nearest-neighbour upscale would thicken every wall and copy every
pickup; this keeps what each cell *is* (docs/large-maps-follow-camera.md):

- Every old cell has an anchor in the new grid, `floor(c * factor)`, with
  the last column and row anchored on the new edge so gates stay on it.
- Barriers (walls, props, trees, water, towers) are drawn thin: the
  object at its anchor, plus a bridge to every barrier it touched along a
  side and through a corner, so a one-cell wall line stays one cell thick
  and nothing that was closed opens. A block of barriers fills in, so a
  lake stays a lake and a river stays a river.
- Lines that are not barriers (road, oil, gates) are bridged to their own
  kind only.
- Areas (tall grass) fill the old cell's whole block.
- Points (pickups, starts, the frogs, portals, towers, a lone barrel) are
  placed once, at the anchor.

The header comment is kept with a line naming the scale; top-level keys
are kept (the band's `tanks` grows by the factor), and everything is
written as dotted keys (CLAUDE.md: a `[spawn]` header would swallow the
`cells.` lines after it).
"""

import argparse
import math
import tomllib

BARRIER = {"wall", "sandbag", "fence", "barrel", "tree", "pine", "water", "tesla", "gun_tower", "bio_slush"}
TOWER = {"tesla", "gun_tower", "bio_slush"}
LINE = {"road", "oil", "gate"}
AREA = {"tall_grass"}
# Which object a bridge between two different barriers is made of: the
# higher of the two. A tower is never copied and a barrel only when both
# ends are barrels, so a bridge never multiplies either.
BRIDGE_RANK = {"wall": 6, "fence": 5, "sandbag": 5, "tree": 4, "pine": 4, "water": 3, "barrel": 1}
# Barrels are explosive points: copied only along a run of barrels.
POINT_BARRIER = {"barrel"} | TOWER


def anchors(n_old, size_old, factor):
    """The anchor of each old index along one axis, and the new size.

    Index `n_old` is the cell one step past the edge, where a map paints
    the water and roads that run off it; it stays one step past the new
    edge."""
    size_new = math.floor(size_old * factor * 2 + 0.5) / 2
    # The old last row lands on the new last row of the same kind: a
    # whole row stays whole, so an edge pickup is never half off the
    # field, and a half row stays half. Past it is the cell the run-off
    # is painted in, beyond every row on the field.
    if float(size_old).is_integer():
        last_new = math.floor(size_new) - 1
    else:
        last_new = math.ceil(size_new) - 1
    beyond = math.ceil(size_new)
    # The two outermost rows keep their distance to the edge, as the two
    # first ones do (0 -> 0, 1 -> 1): a line one cell in from the edge
    # stays one cell in, rather than opening a lane behind it.
    a = [min(math.floor(i * factor), last_new) for i in range(n_old)]
    a[n_old - 1] = last_new
    if n_old >= 3:
        a[n_old - 2] = max(a[n_old - 3] + 1, last_new - 1)
    a.append(beyond)
    return a, size_new, beyond


def scale(doc, factor):
    cols_old, rows_old = doc["size"]
    nc, nr = math.ceil(cols_old), math.ceil(rows_old)
    ax, cols_new, beyond_x = anchors(nc, cols_old, factor)
    ay, rows_new, beyond_y = anchors(nr, rows_old, factor)
    old = {tuple(map(int, k.split(","))): v for k, v in doc["cells"].items()}
    new = {}

    def put(x, y, obj, force=False):
        if 0 <= x <= beyond_x and 0 <= y <= beyond_y and (force or (x, y) not in new):
            new[(x, y)] = dict(obj)

    def block(c, r):
        x1 = ax[c + 1] - 1 if c + 1 <= nc else ax[c]
        y1 = ay[r + 1] - 1 if r + 1 <= nr else ay[r]
        return range(ax[c], x1 + 1), range(ay[r], y1 + 1)

    def bridge_obj(a, b):
        if a["kind"] == b["kind"] and a["kind"] not in TOWER:
            return a
        pick = [o for o in (a, b) if o["kind"] not in POINT_BARRIER]
        if not pick:
            return None
        return max(pick, key=lambda o: BRIDGE_RANK.get(o["kind"], 0))

    def corner(a, b):
        """What keeps two barriers that touched at a corner touching: never
        a barrel or a tower, whose corners a wall would seal."""
        if a is None or b is None or a["kind"] in POINT_BARRIER or b["kind"] in POINT_BARRIER:
            return None
        return joins(a, b)

    def joins(a, b):
        """The object the cells between two neighbours get, or None."""
        if a is None or b is None:
            return None
        ka, kb = a["kind"], b["kind"]
        if ka in BARRIER and kb in BARRIER:
            return bridge_obj(a, b)
        if ka in LINE and ka == kb:
            return a
        return None

    # Points and areas first, then anchors, then bridges; the anchors win
    # their own cells (`force`), a bridge only takes a free cell.
    for (c, r), obj in old.items():
        if obj["kind"] in AREA:
            xs, ys = block(c, r)
            for x in xs:
                for y in ys:
                    put(x, y, obj)
    for (c, r), obj in sorted(old.items()):
        put(ax[c], ay[r], obj, force=True)
    for (c, r), a in sorted(old.items()):
        for dc, dr in ((1, 0), (0, 1)):
            if c + dc > nc or r + dr > nr:
                continue
            b = old.get((c + dc, r + dr))
            obj = joins(a, b)
            if obj is None:
                continue
            if dc:
                for x in range(ax[c] + 1, ax[c + 1]):
                    put(x, ay[r], obj, force=new.get((x, ay[r]), {}).get("kind") in AREA)
            else:
                for y in range(ay[r] + 1, ay[r + 1]):
                    put(ax[c], y, obj, force=new.get((ax[c], y), {}).get("kind") in AREA)
        # A solid 2 x 2 fills its middle; a pair touching only at a corner
        # keeps touching through one cell between their anchors.
        b, d, e = old.get((c + 1, r)), old.get((c, r + 1)), old.get((c + 1, r + 1))
        if c + 1 > nc or r + 1 > nr:
            continue
        xs = range(ax[c] + 1, ax[c + 1])
        ys = range(ay[r] + 1, ay[r + 1])
        if all(joins(a, o) for o in (b, d, e)):
            obj = joins(a, e)
            for x in xs:
                for y in ys:
                    put(x, y, obj, force=new.get((x, y), {}).get("kind") in AREA)
        elif corner(a, e) and not joins(a, b) and not joins(a, d):
            put(ax[c + 1] - 1, ay[r + 1] - 1, corner(a, e), force=new.get((ax[c + 1] - 1, ay[r + 1] - 1), {}).get("kind") in AREA)
    # The anti-diagonal: (c+1, r) against (c, r+1).
    for (c, r), a in sorted(old.items()):
        if c < 1 or c > nc or r + 1 > nr:
            continue
        e = old.get((c - 1, r + 1))
        if corner(a, e) and not joins(a, old.get((c - 1, r))) and not joins(a, old.get((c, r + 1))):
            x, y = ax[c] - 1, ay[r + 1] - 1
            if x > ax[c - 1] or y > ay[r]:
                put(max(x, ax[c - 1]), max(y, ay[r]), corner(a, e), force=new.get((x, y), {}).get("kind") in AREA)
    return new, cols_new, rows_new


def toml_value(v):
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return repr(v)
    return '"' + str(v).replace('"', '\\"') + '"'


def size_value(v):
    return str(int(v)) if float(v).is_integer() else repr(float(v))


def write(path, header, doc, cells, cols, rows, factor):
    out = list(header)
    if out and out[-1].strip():
        out.append("#")
    old_cols, old_rows = doc["size"]
    out.append(
        f"# Scaled {factor}x from the {size_value(old_cols)} x {size_value(old_rows)} original by "
        "tools/scale_map.py (thin walls, bridged lines, single pickups); the"
    )
    out.append("# cell coordinates above are the original's.")
    out.append("")
    out.append("version = 1")
    out.append(f"size = [{size_value(cols)}, {size_value(rows)}]")
    for key, v in doc.items():
        if key in ("version", "size", "cells", "mission", "spawn"):
            continue
        if key == "tanks":
            v = min(31, round(v * factor))
        out.append(f"{key} = {toml_value(v)}")
    for table in ("mission", "spawn"):
        for key, v in doc.get(table, {}).items():
            out.append(f"{table}.{key} = {toml_value(v)}")
    out.append("")
    for (x, y) in sorted(cells, key=lambda p: (p[1], p[0])):
        obj = cells[(x, y)]
        fields = ", ".join(f"{k} = {toml_value(v)}" for k, v in obj.items())
        out.append(f'cells."{x},{y}" = {{ {fields} }}')
    with open(path, "w") as f:
        f.write("\n".join(out) + "\n")


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("map")
    p.add_argument("--factor", type=float, default=1.5)
    p.add_argument("-o", "--out")
    args = p.parse_args()
    with open(args.map, "rb") as f:
        doc = tomllib.load(f)
    with open(args.map) as f:
        header = []
        for line in f:
            if not line.startswith("#"):
                break
            header.append(line.rstrip("\n"))
    cells, cols, rows = scale(doc, args.factor)
    write(args.out or args.map, header, doc, cells, cols, rows, args.factor)
    print(f"{args.map}: {doc['size'][0]} x {doc['size'][1]} -> {cols} x {rows}, {len(doc['cells'])} -> {len(cells)} cells")


if __name__ == "__main__":
    main()
