"""Grow a map's layout by a factor a side, keeping the file's text.

  python3 tools/scale_map.py maps/x.toml --factor 1.4 [-o out.toml]
  python3 tools/scale_map.py maps/x.toml --size 48 24 [-o out.toml]

How the levels and the shipped free-play maps were made 40 % larger a
side. The file keeps its header comments, its settings, its dotted keys or
the editor's tables; its cells are written again, row by row, and its
`size` is the new one (columns whole; rows whole where they were whole,
since a half row past the last whole one would push an edge's gates off
it, and to a half where they were a half). Cells past the old field are
dropped. Needs Python 3.11 (`tomllib`).

The layout is scaled, not its pixels. Rows then columns, each cell of the
old field becomes a block of one or two cells of the new one, the first
always holding the old cell's own thing; the second is decided by what
the two cells either side of it have to keep:

* a line one cell thick - a wall, a fence, a road, a hedge, a stream -
  stays one cell thick and grows longer; its corners stay corners;
* anything thicker - a lake, a wood, a patch of tall grass, a building -
  grows both ways, a building's shell staying thin while its inside grows
  and a mixed wall's thin posts staying thin while its panes grow;
* a single thing - a start, a frog, a pickup, a portal, a gate, a tower -
  is never repeated, and a line it sits on (a road under a pickup, a wall
  round a tower) goes on round it;
* two things that touched still touch where that matters: an oil trail
  and its drum, water and its wall or bridge, a line crossing a road;
* water two cells across stays two cells across where it has a bank at
  both ends, and a body of water with no open water in it gives a bank
  cell back wherever it would get some (ground.rs's depth rule: a third
  cell would put deep water, which no hull crosses, down a ford's middle).

Then check the result with the map linter (`cargo test --lib maplint`, or
the builder's CHECK): what is measured in pixels does not grow with it - a
frog or a pickup can end up past the linter's reach from the open ground,
and a sealed room big enough to show as a pocket.
"""
import argparse
import math
import re
import sys
import tomllib

POINT_KINDS = {"start", "start2", "frog", "enemy_frog", "pickup", "portal", "gate", "tesla", "gun_tower", "bio_slush"}
TOWERS = {"tesla", "gun_tower", "bio_slush"}
SOLID = {"wall", "sandbag", "fence", "barrel"}
TREES = {"tree", "pine"}
FLOORS = {"water", "road", "oil", "tall_grass"}


def family(obj):
    if obj is None:
        return None
    k = obj["kind"]
    if k in POINT_KINDS:
        return ("point", id(obj))
    if k in SOLID:
        return "solid"
    if k in TREES:
        return "tree"
    return k  # water, road, oil, tall_grass


def sits_on(point, material):
    """Whether a single thing between two cells of a material sits on that
    material's run - a pickup on a road, the frog in its reeds, a tower set
    into a wall - rather than in a gap between two of its pieces (a pickup
    between two pillars)."""
    if material["kind"] in FLOORS:
        return True
    return point["kind"] in TOWERS


def is_material(obj):
    return obj is not None and obj["kind"] not in POINT_KINDS


def axis_map(n_src, n_dst, length_src, length_dst):
    """S(t) for t in 0..n_dst: the source cell each target cell reads."""
    k = length_dst / length_src
    return [min(n_src - 1, int(math.floor((t + 0.5) / k))) for t in range(n_dst)]


def mat(obj):
    if obj is None:
        return None
    if obj["kind"] in POINT_KINDS:
        return ("point", id(obj))
    return tuple(sorted(obj.items()))


def scale_line(seq, S, perp):
    """One row or column: seq is the source objects, S the axis map.

    The second cell of a doubled block - the cell between a source cell's
    own and the next one's - is decided by what the two have to keep:
    a run continues or extends, a line one thick stays one thick, two
    materials that touched still touch, and a single thing is never
    repeated.
    """
    n = len(seq)
    fam = [family(o) for o in seq]
    m = [mat(o) for o in seq]

    def runs(key):
        """Run lengths by key, bridged across a single thing sitting on
        the run (flanked by it on both sides); 0 for single things."""
        def bridges(i, k):
            return (0 < i < n - 1 and seq[i] is not None and not is_material(seq[i])
                    and key[i - 1] == k and key[i + 1] == k
                    and sits_on(seq[i], seq[i - 1]))
        length = [0] * n
        span = [None] * n
        i = 0
        while i < n:
            if not is_material(seq[i]):
                i += 1
                continue
            j = i
            while True:
                if j + 1 < n and is_material(seq[j + 1]) and key[j + 1] == key[i]:
                    j += 1
                elif j + 2 < n and bridges(j + 1, key[i]) and key[j + 2] == key[i]:
                    j += 2
                else:
                    break
            for x in range(i, j + 1):
                if is_material(seq[x]):
                    length[x] = j - i + 1
                    span[x] = (i, j)
            i = j + 1
        return length, span, bridges

    mrun, _, mbridges = runs(m)
    frun, fspan, fbridges = runs(fam)
    # a family run that is a line one thick across the axis (a mixed wall
    # seen along its length) grows through its wide materials, so its
    # thin end post stays thin; any other run (a block) extends as a block
    line = [False] * n
    for x in range(n):
        if fspan[x] is not None:
            a, b = fspan[x]
            cells = [i for i in range(a + 1, b) if is_material(seq[i])] or [a, b]
            line[x] = all(not perp[i] for i in cells)

    def grows(x, y, z, ys):
        """What takes the cell between two touching materials of different
        families, x one thick here - only where the touch matters."""
        kinds = {x["kind"], y["kind"]}
        if "tall_grass" in kinds:
            return None
        if kinds == {"oil", "barrel"}:
            return x if x["kind"] == "oil" else y  # the fuse reaches its drum
        if y["kind"] == "water" and frun[ys] >= 2:
            return y  # a lake or a river reaches its wall, its bridge
        if is_material(z) and family(z) == family(y):
            return y  # x crosses a run of y: the run reaches it on both sides
        return None

    out = []
    prev_s = None
    for t, s in enumerate(S):
        first = s != prev_s
        prev_s = s
        x = seq[s]
        if first or x is None:
            out.append(x)
            continue
        y = seq[s + 1] if s + 1 < n else None
        z = seq[s - 1] if s > 0 else None
        if not is_material(x):  # a single thing: never repeated
            if is_material(y) and family(z) == family(y) and sits_on(x, y):
                out.append(y)  # it sits on a run: the run goes on round it
            else:
                out.append(None)
            continue
        if y is not None and (m[s + 1] == m[s] or mbridges(s + 1, m[s])):
            out.append(x)  # the material runs on
            continue
        if y is not None and (fam[s + 1] == fam[s] or fbridges(s + 1, fam[s])):
            # inside a run of one family: which material takes the cell
            ys = s + 1 if is_material(y) else s + 2
            yy = seq[ys]
            if x["kind"] == "barrel" and yy["kind"] != "barrel":
                out.append(yy)
                continue
            if yy["kind"] == "barrel" and x["kind"] != "barrel":
                out.append(x)
                continue
            outer = family(z) != fam[s]  # x is the run's edge
            if not perp[s] and not perp[ys]:
                # a line one thick: panes grow, posts stay one thick
                if mrun[ys] >= 2 and mrun[ys] >= mrun[s]:
                    out.append(yy)
                elif mrun[s] >= 2:
                    out.append(x)
                else:
                    out.append(yy if outer else x)
                continue
            # a block: its shell stays thin, its inside grows
            beyond = seq[ys + 1] if ys + 1 < n else None
            inner = family(beyond) == fam[s]  # yy is inside the run
            if outer and inner:
                out.append(yy)
            elif outer and not inner:
                out.append(yy)  # two layers: the outer one stays thin
            else:
                out.append(x)
            continue
        # x ends its family's run (a drum at its end is never repeated)
        if is_material(y) and y["kind"] == "water" and frun[s + 1] >= 3:
            out.append(y)  # a lake keeps its shape: it reaches whatever ends at it
        elif x["kind"] != "barrel" and (mrun[s] >= 2 or (frun[s] >= 2 and not line[s])):
            out.append(x)  # the run takes its full length
        elif y is None or not is_material(y):
            out.append(None)  # a line stays a line
        else:
            out.append(grows(x, y, z, s + 1))  # two materials that touched still touch
    keep_fords(seq, S, out, mrun)
    return out


def keep_fords(seq, S, out, mrun):
    """Water two cells across stays two cells across: a ford is a ford
    because no cell of it has water all round (ground.rs's depth rule), and
    a third cell would put deep water down its middle."""
    n = len(seq)
    block = {}
    for t, s in enumerate(S):
        block.setdefault(s, []).append(t)
    for i in range(n - 1):
        x = seq[i]
        if not (is_material(x) and x["kind"] == "water" and mrun[i] == 2):
            continue
        if i > 0 and mat(seq[i - 1]) == mat(x):
            continue  # the second cell of a run already handled
        if mat(seq[i + 1]) != mat(x):
            continue
        # a ford has a bank at both ends: dry ground, or something standing
        # on it - not the map's edge (water runs on past it) and not a boat
        # or a pier with water beyond it
        if i == 0 or i + 2 >= n:
            continue

        def bank(j, beyond):
            o = seq[j]
            if o is not None and o["kind"] == "water":
                return False
            far = seq[beyond] if 0 <= beyond < n else None
            return not (o is not None and far is not None and far["kind"] == "water")

        if not (bank(i - 1, i - 2) and bank(i + 2, i + 3)):
            continue
        cells = [t for s in (i, i + 1) for t in block.get(s, []) if out[t] is not None and mat(out[t]) == mat(x)]
        if len(cells) <= 2:
            continue
        first = block[i]
        if len(first) == 2:
            # the first cell's block is doubled: the ford starts a cell later,
            # what stands before it taking the cell where it stood
            before = seq[i - 1] if i > 0 else None
            keep = before if is_material(before) and mrun[i - 1] >= 2 else None
            out[first[0]] = keep
        else:
            out[block[i + 1][-1]] = None if len(block[i + 1]) == 2 else out[block[i + 1][-1]]


def fix_corners(get, lines, S, n_along, n_across):
    """Move a thin line in a doubled block to the block's second cell where
    the run it meets reaches that cell: a room's far wall lands on the end
    of the walls that meet it, so a corner stays a corner - no ear past it,
    no L turned into a T. get(along, across) reads the pass's input; lines
    are its output, one per across index."""
    block = {}
    for t, s in enumerate(S):
        block.setdefault(s, []).append(t)
    for s, ts in block.items():
        if len(ts) != 2:
            continue
        a, b = ts
        i = 0
        while i < n_across:
            me = get(s, i)
            if not is_material(me):
                i += 1
                continue
            f = family(me)
            j = i
            while j + 1 < n_across and is_material(get(s, j + 1)) and family(get(s, j + 1)) == f:
                j += 1
            ends = starts = False
            thin = []
            for k in range(i, j + 1):
                near, far = get(s - 1, k), get(s + 1, k)
                if family(near) == f and family(far) != f and family(lines[k][b]) == f:
                    ends = True
                if family(far) == f:
                    starts = True
                if family(near) != f and family(far) != f:
                    thin.append(k)
            if ends and not starts and thin and all(
                    not is_material(get(s - 1, k)) and lines[k][b] is None for k in thin):
                for k in thin:
                    lines[k][b], lines[k][a] = lines[k][a], None
            i = j + 1


def scale_grid(grid, W, H, cols, rows, W2, H2, cols2, rows2):
    Sx = axis_map(W, W2, cols, cols2)
    Sy = axis_map(H, H2, rows, rows2)

    def thick(get, a, b, f):
        return f is not None and (family(get(a)) == f or family(get(b)) == f)

    def src(c, r):
        return grid.get((c, r)) if 0 <= c < W and 0 <= r < H else None

    mid = [[None] * W2 for _ in range(H)]
    for r in range(H):
        row = [src(c, r) for c in range(W)]
        perp = [thick(grid.get, (c, r - 1), (c, r + 1), family(row[c])) for c in range(W)]
        mid[r] = scale_line(row, Sx, perp)
    fix_corners(src, mid, Sx, W, H)

    def at(c, r):
        return mid[r][c] if 0 <= c < W2 and 0 <= r < H else None

    cols_out = []
    for c in range(W2):
        col = [mid[r][c] for r in range(H)]
        perp = [thick(lambda p: at(*p), (c - 1, r), (c + 1, r), family(col[r])) for r in range(H)]
        cols_out.append(scale_line(col, Sy, perp))
    fix_corners(lambda r, c: at(c, r), cols_out, Sy, H, W2)
    out = {}
    for c, col in enumerate(cols_out):
        for r, o in enumerate(col):
            if o is not None:
                out[(c, r)] = o
    # join pure diagonal steps of one material
    fwdx = {}
    for t, s in enumerate(Sx):
        fwdx.setdefault(s, t)
    fwdy = {}
    for t, s in enumerate(Sy):
        fwdy.setdefault(s, t)
    for (c, r), o in list(grid.items()):
        if not is_material(o):
            continue
        f = family(o)
        for dc in (-1, 1):
            c2, r2 = c + dc, r + 1
            o2 = grid.get((c2, r2))
            if not is_material(o2) or family(o2) != f:
                continue
            if family(grid.get((c2, r))) == f or family(grid.get((c, r2))) == f:
                continue
            a = (fwdx[c], fwdy[r])
            b = (fwdx[c2], fwdy[r2])
            # walk an 8-connected line from a to b, filling empty cells
            x0, y0 = a
            x1, y1 = b
            steps = max(abs(x1 - x0), abs(y1 - y0))
            for i in range(1, steps):
                px = x0 + round((x1 - x0) * i / steps)
                py = y0 + round((y1 - y0) * i / steps)
                if (px, py) not in out:
                    out[(px, py)] = o
    keep_shallow(grid, out, W, H, W2, H2, Sx, Sy)
    return out


def water_deep(water, W, H):
    """ground.rs's depth rule, near enough: open water - water all round,
    water past the map's edge running on - is deep."""
    def wet(c, r):
        return (min(max(c, 0), W - 1), min(max(r, 0), H - 1)) in water
    return {(c, r) for (c, r) in water if all(wet(c + dc, r + dr) for dc in (-1, 0, 1) for dr in (-1, 0, 1))}


def keep_shallow(grid, out, W, H, W2, H2, Sx, Sy):
    """A body of water with no open water in it - a moat, a river, a pond, a
    ford - stays wadeable: where scaling put water all round a cell of one,
    the bank beside it takes back a cell of it."""
    water = {p for p, o in grid.items() if o["kind"] == "water"}
    deep = water_deep(water, W, H)
    body = {}
    for start in sorted(water):
        if start in body:
            continue
        stack, cells = [start], []
        body[start] = start
        while stack:
            c, r = stack.pop()
            cells.append((c, r))
            for dc, dr in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                q = (c + dc, r + dr)
                if q in water and q not in body:
                    body[q] = start
                    stack.append(q)
        if any(p in deep for p in cells):
            for p in cells:
                body[p] = None  # a lake: its open water may grow
    def shallow_body(c, r):
        s = (Sx[c], Sy[r])
        return body.get(s) is not None

    def wet(c, r):
        o = out.get((min(max(c, 0), W2 - 1), min(max(r, 0), H2 - 1)))
        return o is not None and o["kind"] == "water"

    def is_deep(c, r):
        return all(wet(c + dc, r + dr) for dc in (-1, 0, 1) for dr in (-1, 0, 1))

    def on_bank(c, r):
        return any(0 <= c + dc < W2 and 0 <= r + dr < H2 and not wet(c + dc, r + dr)
                   for dc, dr in ((1, 0), (-1, 0), (0, 1), (0, -1)))

    # only the banks as scaled give water back: the middle of a river is
    # never taken, so a river narrows and never splits into two strands
    banks = {(c, r) for (c, r), o in out.items()
             if o["kind"] == "water" and shallow_body(c, r) and on_bank(c, r)}

    def keeps_joined(c, r):
        """Taking (c, r) leaves the water round it in one piece (a simple
        point: the wet cells of its 3x3 stay 4-joined without it)."""
        ring = [(c + dc, r + dr) for dr in (-1, 0, 1) for dc in (-1, 0, 1)
                if (dc, dr) != (0, 0) and wet(c + dc, r + dr)
                and 0 <= c + dc < W2 and 0 <= r + dr < H2]
        if not ring:
            return False
        seen, stack = {ring[0]}, [ring[0]]
        while stack:
            pc, pr = stack.pop()
            for q in ((pc + 1, pr), (pc - 1, pr), (pc, pr + 1), (pc, pr - 1)):
                if q in ring and q not in seen:
                    seen.add(q)
                    stack.append(q)
        return len(seen) == len(ring)

    for _ in range(10000):
        bad = sorted((c, r) for (c, r), o in out.items()
                     if o["kind"] == "water" and shallow_body(c, r) and is_deep(c, r))
        moved = False
        for c, r in bad:
            if not is_deep(c, r):
                continue
            around = [(c + dc, r + dr) for dr in (-1, 0, 1) for dc in (-1, 0, 1) if (dc, dr) != (0, 0)]
            cands = [p for p in around if p in banks and wet(*p) and keeps_joined(*p)]
            if not cands:
                continue

            def rank(p):
                # the most exposed bank cell first, so the bank stays smooth;
                # the second cell of a doubled block before a source cell's own
                pc, pr = p
                second = (pc > 0 and Sx[pc - 1] == Sx[pc]) or (pr > 0 and Sy[pr - 1] == Sy[pr])
                wet_round = sum(wet(pc + dc, pr + dr) for dr in (-1, 0, 1) for dc in (-1, 0, 1) if (dc, dr) != (0, 0))
                return (wet_round, not second, p[1], p[0])

            take = min(cands, key=rank)
            floor = [out.get((take[0] + dc, take[1] + dr)) for dc, dr in ((1, 0), (-1, 0), (0, 1), (0, -1))]
            floor = [o for o in floor if o is not None and o["kind"] in ("tall_grass", "road")]
            if floor:
                out[take] = floor[0]
            else:
                out.pop(take, None)
            moved = True
        if not moved:
            return


def num(v):
    return int(v) if float(v).is_integer() else v


def size_of(d):
    cols, rows = d.get("size", [34, 17])
    return float(cols), float(rows)


CELL_DOTTED = re.compile(r'^cells\."(-?\d+),(-?\d+)"\s*=\s*\{.*\}\s*$')
CELL_TABLE = re.compile(r'^\[cells\."(-?\d+),(-?\d+)"\]\s*$')


def inline(obj):
    keys = ["kind"] + [k for k in obj if k != "kind"]
    return "{ " + ", ".join(f'{k} = "{obj[k]}"' for k in keys) + " }"


def table(c, r, obj):
    keys = ["kind"] + [k for k in obj if k != "kind"]
    return [f'[cells."{c},{r}"]'] + [f'{k} = "{obj[k]}"' for k in keys] + [""]


def fmt_size(v, editor):
    if editor:
        return f"{float(v):.1f}"
    return str(num(v))


def key_of(obj):
    return tuple(sorted(obj.items()))


def dotted_groups(lines):
    """A hand-written file's cells as it grouped them - runs of cell lines
    between blank lines, each group the kinds of thing it holds in the
    order it lists them - or None where it lists them as one block."""
    groups, current = [], None
    for line in lines:
        m = CELL_DOTTED.match(line)
        if m:
            if current is None:
                current = []
                groups.append(current)
            obj = tomllib.loads("x = " + line.split("=", 1)[1].strip())["x"]
            if key_of(obj) not in current:
                current.append(key_of(obj))
        elif not line.strip():
            current = None
        elif groups:
            current = None
    return groups if len(groups) > 1 else None


def rewrite(text, new_cells, new_size):
    lines = text.split("\n")
    groups = dotted_groups(lines)
    out = []
    insert_at = None
    editor = None
    i = 0
    size_done = False
    while i < len(lines):
        line = lines[i]
        if CELL_DOTTED.match(line):
            editor = False if editor is None else editor
            if insert_at is None:
                insert_at = len(out)
            i += 1
            # the blank lines between a file's groups go with its cells
            while groups and i < len(lines) and not lines[i].strip() and i + 1 < len(lines) and CELL_DOTTED.match(lines[i + 1]):
                i += 1
            continue
        if CELL_TABLE.match(line):
            editor = True
            if insert_at is None:
                insert_at = len(out)
            i += 1
            while i < len(lines) and lines[i].strip() and not lines[i].startswith("["):
                i += 1
            while i < len(lines) and not lines[i].strip():
                i += 1
            continue
        m = re.match(r"^size\s*=\s*\[(.*)$", line)
        if m and not size_done:
            size_done = True
            if "]" in line:
                out.append(f"size = [{fmt_size(new_size[0], False)}, {fmt_size(new_size[1], False)}]")
                i += 1
            else:
                out.append("size = [")
                out.append(f"    {fmt_size(new_size[0], True)},")
                out.append(f"    {fmt_size(new_size[1], True)},")
                out.append("]")
                i += 1
                while i < len(lines) and lines[i].strip() != "]":
                    i += 1
                i += 1
            continue
        out.append(line)
        i += 1
    if not size_done:
        # no size line: the default 34 x 17, so write the new one after version
        for j, line in enumerate(out):
            if line.startswith("version"):
                out.insert(j + 1, f"size = [{fmt_size(new_size[0], False)}, {fmt_size(new_size[1], False)}]")
                if insert_at is not None and insert_at > j:
                    insert_at += 1
                break
    cells = sorted(new_cells.items(), key=lambda kv: (kv[0][1], kv[0][0]))
    if editor:
        block = []
        for (c, r), o in cells:
            block += table(c, r, o)
    elif groups:
        # each cell in the first group that holds its kind of thing, in the
        # group's order of kinds and then row by row
        placed = [[] for _ in groups]
        rest = []
        for (c, r), o in cells:
            k = key_of(o)
            g = next((n for n, keys in enumerate(groups) if k in keys), None)
            if g is None:
                rest.append(((c, r), o))
            else:
                placed[g].append((groups[g].index(k), r, c, o))
        block = []
        for group in placed + ([[(0, r, c, o) for (c, r), o in rest]] if rest else []):
            if not group:
                continue
            if block:
                block.append("")
            block += [f'cells."{c},{r}" = {inline(o)}' for _, r, c, o in sorted(group, key=lambda x: x[:3])]
    else:
        block = [f'cells."{c},{r}" = {inline(o)}' for (c, r), o in cells]
    out[insert_at:insert_at] = block
    return "\n".join(out)


def grown(cols, rows, factor):
    """The size a map grows to: whole columns, and rows whole where they
    were whole and to a half where they were a half."""
    cols2 = round(cols * factor)
    rows2 = round(rows * factor) if float(rows).is_integer() else round(rows * factor * 2) / 2
    return float(cols2), float(rows2)


def scale_file(src, dst, size=None, factor=None):
    text = open(src).read()
    d = tomllib.loads(text)
    cols, rows = size_of(d)
    cols2, rows2 = size if size else grown(cols, rows, factor)
    if not (8 <= cols2 <= 250 and 8 <= rows2 <= 250):
        sys.exit(f"{src}: {cols2:g} x {rows2:g} is outside the 8 to 250 cells a side a map may be")
    W, H = math.ceil(cols), math.ceil(rows)
    W2, H2 = math.ceil(cols2), math.ceil(rows2)
    grid = {}
    for k, v in d.get("cells", {}).items():
        c, r = map(int, k.split(","))
        if 0 <= c < W and 0 <= r < H:
            grid[(c, r)] = v
    new = scale_grid(grid, W, H, cols, rows, W2, H2, cols2, rows2)
    out = rewrite(text, new, (cols2, rows2))
    if len(tomllib.loads(out).get("cells", {})) != len(new):
        sys.exit(f"{src}: the rewritten file does not read back")
    open(dst, "w").write(out)
    print(f"{src}: {cols:g} x {rows:g} -> {cols2:g} x {rows2:g}, {len(grid)} -> {len(new)} cells, written to {dst}")


def main():
    p = argparse.ArgumentParser(description="Grow a map's layout by a factor a side.")
    p.add_argument("map")
    p.add_argument("-o", "--out", help="where to write it (the map itself when left out)")
    g = p.add_mutually_exclusive_group(required=True)
    g.add_argument("--factor", type=float, help="how much longer each side grows, e.g. 1.4")
    g.add_argument("--size", type=float, nargs=2, metavar=("COLS", "ROWS"), help="the new size in cells")
    a = p.parse_args()
    scale_file(a.map, a.out or a.map, size=tuple(a.size) if a.size else None, factor=a.factor)


if __name__ == "__main__":
    main()
