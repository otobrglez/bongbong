"""Write maps/no-mans-land.toml: No Man's Land, level 11 - a karst frontier
between two trench lines, 87 x 43 cells, laid out set piece by set piece.

  python3 tools/gen_no_mans_land.py [-o maps/no-mans-land.toml] [--stats] [--ascii [--routes]] [--nav]

A map this size is too big to lay out by hand, so this script is the
source: change it and run it again rather than editing the TOML (a
re-save from the map editor would drop the header too). Nothing is drawn
at random - the noise is hashed from cell positions - so the script
writes the same file every time. `--ascii` prints the map a character a
cell (`--routes` marks the waves' cheapest ways in), `--nav` the nav grid the AI routes on (what is reached, what is one
cell wide), `--stats` the counts, each gate's walk to the start and every
pickup. The checks run every time and fail the run: every pickup, start
and gate lane reached, no boxed-in nav cell, the redoubt's walls whole but
for their gates and no gate with a straight line to a start, no drum by a
start, and no loose drum or prop on a wave's cheapest way in (the router
drives straight through loose props, and a hull that meets one on its way
has to shove it).

What it lays out (the header written into the map says how it plays):

- the redoubt in the west, the players' start: a concrete wall with
  gun towers at its gates, a traverse inside each gate so no way in has a
  straight line to the starts, the command dugout and the stores;
- the field hospital north of it and the ammunition dump south of it;
- the players' front line: a zig-zag sandbag parapet with its trench floor
  behind it, two pillboxes with their gun towers, two dugouts with stashes
  and a wire belt out in front;
- no man's land: shell-holes (the karst's dark pools), hedgehogs, the
  ruined chapel and the ruined farm on the old road, the splintered wood
  and the drowned craters, each with a stash;
- the enemy's line: the same parapet facing west, concrete pillboxes with
  their gun towers, teslas on the road, bio slushes behind, dugouts with
  their stores, wire before it;
- the enemy's rear: the fuel dump, the battery and their command bunker,
  and the seven gates the waves come in by.
"""

import argparse
import heapq
import math
from collections import deque

W, H = 87, 43

# (col, row) -> inline TOML table text
cells = {}
# cells the scrub pass must leave alone
reserved = set()
water = set()
roads = set()

# the starts stand behind the redoubt's traverses, out of the line of every
# gate
START = (12, 18)
START2 = (12, 24)

GATES = [
    (86, 6), (86, 21), (86, 36),  # east, behind the enemy's line
    (50, 0), (76, 0),  # north: no man's land and behind the enemy's line
    (50, 42), (76, 42),  # south
]

# the redoubt's outer wall and its gates
HQ = (2, 10, 21, 32)
HQ_GATES = {(21, 20), (21, 21), (21, 22), (6, 10), (7, 10), (8, 10), (6, 32), (7, 32), (8, 32)}

pickups = []  # (c, r, kind)

TREE_KINDS = {"tree", "pine", "spruce", "scots", "fir", "birch", "willow", "palm", "snag"}
COVER_KINDS = {"tall_grass", "bush", "berry_bush", "juniper", "fern", "autumn_bush", "reeds"}


def inb(c, r):
    return 0 <= c < W and 0 <= r < H


def obj(kind, **kw):
    parts = [f'kind = "{kind}"'] + [f'{k} = "{v}"' for k, v in kw.items()]
    return "{ " + ", ".join(parts) + " }"


def put(c, r, kind, **kw):
    if not inb(c, r):
        return
    cells[(c, r)] = obj(kind, **kw)
    for layer, name in ((water, "water"), (roads, "road")):
        if kind == name:
            layer.add((c, r))
        else:
            layer.discard((c, r))
    reserved.add((c, r))


def wall(c, r, material):
    put(c, r, "wall", material=material)


def clear(c, r):
    if inb(c, r):
        cells.pop((c, r), None)
        water.discard((c, r))
        roads.discard((c, r))
        reserved.add((c, r))


def put_free(c, r, kind, **kw):
    """A decoration that gives way to anything already placed."""
    if inb(c, r) and (c, r) not in cells:
        put(c, r, kind, **kw)


def kind_at(p):
    t = cells.get(p)
    return t.split('"')[1] if t else None


def disc(cx, cy, rad):
    for r in range(int(cy - rad) - 1, int(cy + rad) + 2):
        for c in range(int(cx - rad) - 1, int(cx + rad) + 2):
            if inb(c, r) and (c - cx) ** 2 + (r - cy) ** 2 <= rad * rad:
                yield c, r


def rect(c0, r0, c1, r1):
    for r in range(r0, r1 + 1):
        for c in range(c0, c1 + 1):
            if inb(c, r):
                yield c, r


def outline(c0, r0, c1, r1):
    for c, r in rect(c0, r0, c1, r1):
        if c in (c0, c1) or r in (r0, r1):
            yield c, r


# the set pieces' open ground, which the scrub keeps off
OPEN_GROUND = set()
# the set pieces' insides, which the scrub keeps off too
INTERIOR = set()


def inside(cells_):
    INTERIOR.update(cells_)


def clear_rect(c0, r0, c1, r1):
    for c, r in list(rect(c0, r0, c1, r1)):
        if (c, r) not in roads and (c, r) not in water:
            clear(c, r)
        reserved.add((c, r))
        OPEN_GROUND.add((c, r))


def keep_clear(c0, r0, c1, r1):
    """A way between set pieces: the scrub leaves it open."""
    for p in rect(c0, r0, c1, r1):
        reserved.add(p)
        OPEN_GROUND.add(p)


def pickup(c, r, kind):
    assert (c, r) not in water and (c, r) not in roads, f"{kind} at {c},{r} would cover water or road"
    put(c, r, "pickup", pickup=kind)
    pickups.append((c, r, kind))


DRUMS = []


def drum(c, r, kind):
    put(c, r, "barrel", drum=kind)
    DRUMS.append((c, r))


def drums(spots, salt, fuel=0.35):
    """Drums at `spots`, each oil or fuel by a hash of its cell: `fuel` of
    them the grey fuel drums that launch when chained."""
    for c, r in spots:
        drum(c, r, "fuel" if hash01(c, r, salt) < fuel else "oil")


def oil_trail(points):
    """An oil trail along `points`: a fuse a blast or a fire runs along to
    the drums at its end."""
    for c, r in polyline(points):
        if (c, r) not in cells:
            put(c, r, "oil")


# ---------------------------------------------------------------- noise


def hash01(i, j, salt):
    h = (i * 0x27D4EB2D) ^ (j * 0x165667B1) ^ (salt * 0x9E3779B1)
    h &= 0xFFFFFFFF
    h ^= h >> 15
    h = (h * 0x85EBCA6B) & 0xFFFFFFFF
    h ^= h >> 13
    h = (h * 0xC2B2AE35) & 0xFFFFFFFF
    h ^= h >> 16
    return h / 0xFFFFFFFF


def value_noise(x, y, salt):
    i, j = math.floor(x), math.floor(y)
    fx, fy = x - i, y - j
    sx, sy = fx * fx * (3 - 2 * fx), fy * fy * (3 - 2 * fy)
    a = hash01(i, j, salt)
    b = hash01(i + 1, j, salt)
    c = hash01(i, j + 1, salt)
    d = hash01(i + 1, j + 1, salt)
    return (a + (b - a) * sx) + ((c + (d - c) * sx) - (a + (b - a) * sx)) * sy


def fbm(x, y, salt, octaves=3):
    total, amp, norm = 0.0, 1.0, 0.0
    for o in range(octaves):
        total += amp * value_noise(x * 2**o, y * 2**o, salt + o * 101)
        norm += amp
        amp *= 0.5
    return total / norm


def pick(table, u):
    total = sum(w for _, w in table)
    acc = 0
    for name, w in table:
        acc += w
        if u * total < acc:
            return name
    return table[-1][0]


# ---------------------------------------------------------------- lines


def raster_line(a, b):
    """The cells from `a` to `b`, 4-connected."""
    (x0, y0), (x1, y1) = a, b
    n = int(max(abs(x1 - x0), abs(y1 - y0)) * 4) + 1
    out = []
    for i in range(n + 1):
        t = i / n
        p = (round(x0 + (x1 - x0) * t), round(y0 + (y1 - y0) * t))
        if out and p != out[-1] and p[0] != out[-1][0] and p[1] != out[-1][1]:
            out.append((p[0], out[-1][1]))
        if not out or p != out[-1]:
            out.append(p)
    return out


def polyline(points):
    path = []
    for a, b in zip(points, points[1:]):
        for p in raster_line(a, b):
            if not path or p != path[-1]:
                path.append(p)
    return path


def road(points, width=2, margin=1):
    """A road `width` cells across, from each point down and to the right;
    the `margin` round it is kept clear of the scrub."""
    path = polyline(points)
    for c, r in path:
        for dc in range(width):
            for dr in range(width):
                if inb(c + dc, r + dr) and (c + dc, r + dr) not in cells:
                    put(c + dc, r + dr, "road")
    for c, r in path:
        for dc in range(-margin, width + margin):
            for dr in range(-margin, width + margin):
                if inb(c + dc, r + dr):
                    reserved.add((c + dc, r + dr))
    return path


def pool(cx, cy, rad, salt):
    """A shell-hole: a round pool of the karst's dark water, centred on a
    cell or (at a half) on a corner, its lip ragged only where a cell sits
    in a notch between two of the pool's own, so no single cell sticks out
    to read as a stream. 2 x 2 or 4 x 4 across it is all ford; 3 x 3 or
    more, its middle is deep."""
    body = {p for p in disc(cx, cy, rad)}
    lip = set()
    for c, r in sorted(disc(cx, cy, rad + 1.0)):
        if (c, r) in body:
            continue
        held = sum((c + dc, r + dr) in body for dc, dr in ((1, 0), (-1, 0), (0, 1), (0, -1)))
        if held >= 2 and hash01(c, r, salt) < 0.5:
            lip.add((c, r))
    out = []
    for c, r in sorted(body | lip):
        if (c, r) in cells and kind_at((c, r)) not in ("road", "oil", None):
            continue
        put(c, r, "water")
        out.append((c, r))
    return out


def hole(c, r, size, salt):
    """A shell-hole `size` cells across (2, 3 or 4) with its top-left
    quarter at (c, r)."""
    if size == 2:
        return pool(c + 0.5, r + 0.5, 0.75, salt)
    if size == 3:
        return pool(c, r, 1.45, salt)
    return pool(c + 0.5, r + 0.5, 1.6, salt)


# ---------------------------------------------------------------- set pieces


def main_road():
    """The old road across the frontier, from the redoubt's east gate to
    the enemy's rear; the shell-holes in no man's land break it."""
    road([(18, 21), (86, 21)])
    road([(6, 6), (6, 11)])  # the north gate up to the field hospital
    road([(6, 31), (6, 35)])  # the south gate down to the dump


def redoubt():
    """The players' start: a concrete wall with two gun towers at its east
    gate and one at the north and the south gate; inside the east gate a
    traverse two cells
    thick, inside the north and south gates a traverse across the way, so
    a tank coming in has to turn before it sees the starts; the command
    dugout against the west wall; a tesla between the starts and a bio
    slush by the south way in; the stores round them."""
    c0, r0, c1, r1 = HQ
    clear_rect(c0 - 1, r0 - 1, c1 + 2, r1 + 1)
    inside(rect(c0 + 1, r0 + 1, c1 - 1, r1 - 1))
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) in HQ_GATES:
            clear(c, r)
            continue
        wall(c, r, "concrete")
    for c, r in ((21, 19), (21, 23), (9, 10), (9, 32)):
        put(c, r, "gun_tower")
    # the traverses
    for c, r in rect(16, 17, 17, 25):
        wall(c, r, "concrete")
    for c in range(3, 13):
        wall(c, 14, "concrete")
        wall(c, 28, "concrete")
    # the command dugout, its door east
    for c, r in outline(3, 18, 8, 24):
        if c == 8 and r in (20, 21, 22):
            continue
        wall(c, r, "concrete")
    put(*START, "start")
    put(*START2, "start2")
    put(12, 21, "tesla")
    put(14, 27, "bio_slush")
    # the stores: in the dugout health, ammo, missiles, a shield and a
    # tower pack; the two gauss rails behind the traverse between the
    # starts; air support in the east ways; grenades and missiles in the
    # far pockets; plasma and a minigun in the north and south ways
    pickup(4, 19, "health")
    pickup(4, 21, "tower_pack")
    pickup(4, 23, "ammo")
    pickup(6, 19, "missiles")
    pickup(6, 23, "shield")
    pickup(14, 20, "gauss_rail")
    pickup(14, 22, "gauss_rail")
    pickup(10, 21, "health")
    pickup(19, 13, "air_heli")
    pickup(19, 29, "air_jet")
    pickup(4, 12, "grenades")
    pickup(4, 30, "missiles")
    pickup(11, 12, "plasma")
    pickup(11, 30, "minigun")
    # sandbag nests outside the east gate, either side of the road
    for c, r in ((23, 17), (24, 17), (23, 18), (23, 24), (23, 25), (24, 25)):
        put(c, r, "sandbag")


def field_hospital():
    """North of the redoubt, the field hospital: a canvas ward (wood)
    against the west edge with its door south onto the road to the north
    gate, beds of health inside, the kitchen's crates and hay beside it."""
    clear_rect(1, 1, 18, 8)
    inside(rect(3, 3, 9, 5))
    for c, r in outline(2, 2, 10, 6):
        if r == 6 and c in (5, 6, 7):
            continue
        wall(c, r, "wood")
    pickup(3, 3, "health")
    pickup(9, 3, "health")
    pickup(3, 5, "health")
    pickup(9, 5, "shield")
    pickup(6, 3, "speedup")
    for c, r in ((14, 2), (15, 2), (14, 3)):
        put(c, r, "wood_crate")
    for c, r in ((17, 5), (17, 6)):
        put(c, r, "hay")


def ammo_dump():
    """South of the redoubt, the ammunition dump against the south edge:
    a pit of crates walled in sandbags with its way in north, and a pit of
    fuel drums with an oil trail out of it - a trap for a wave that comes
    round the south."""
    clear_rect(1, 34, 18, 41)
    inside(rect(3, 37, 16, 40))
    for c, r in outline(2, 36, 9, 41):
        if r == 36 and c in (5, 6, 7):
            continue
        put(c, r, "sandbag")
    pickup(3, 37, "ammo")
    pickup(8, 37, "grenades")
    pickup(3, 40, "air_heli")
    pickup(8, 40, "minigun")
    pickup(6, 39, "missiles")
    for c, r in outline(13, 37, 17, 41):
        if r == 37 and c in (14, 15, 16):
            continue
        put(c, r, "sandbag")
    drums([(14, 39), (15, 39), (16, 39), (14, 40), (15, 40), (16, 40)], 5, fuel=0.6)
    oil_trail([(15, 38), (15, 35), (20, 35)])


def zigzag(cols, r0, r1, step, gaps):
    """A trench parapet of sandbags from row `r0` to `r1`, standing in the
    column `cols[i]` for its `i`th bay of `step` rows and joined across
    between bays, so it zig-zags; the rows in `gaps` are the crossings.
    Returns the parapet's cells."""
    out = []
    for r in range(r0, r1 + 1):
        if r in gaps:
            continue
        c = parapet_col(cols, r0, step, r)
        out.append((c, r))
        # the step between two bays
        if (r - r0) % step == step - 1 and r + 1 <= r1 and r + 1 not in gaps:
            nxt = parapet_col(cols, r0, step, r + 1)
            for cc in range(min(c, nxt), max(c, nxt) + 1):
                out.append((cc, r))
    return out


def parapet_col(cols, r0, step, r):
    return cols[((r - r0) // step) % len(cols)]


def trench(parapet, floor_side):
    """The parapet two sandbags thick - its cells and the ones in front of
    them, away from `floor_side` - and the trench floor, dirt, a road two
    cells across, behind it on `floor_side` (-1 west, +1 east)."""
    for c, r in parapet:
        put(c, r, "sandbag")
    for c, r in parapet:
        front = (c - floor_side, r)
        if front not in cells:
            put(*front, "sandbag")
    for c, r in parapet:
        for d in (1, 2):
            f = (c + d * floor_side, r)
            if f not in cells and inb(*f):
                put(*f, "road")


def pillbox(back, r, facing, tower, side=None):
    """A concrete pillbox built into the parapet: its back wall three cells
    of the parapet's own column `back`, the tower in front of it in a U of
    concrete whose open side - the embrasure - faces `facing` (-1 west,
    +1 east)."""
    c = back + facing
    for cc, rr in ((c, r - 1), (c, r + 1), (back, r - 1), (back, r), (back, r + 1)):
        wall(cc, rr, "concrete")
    if side:
        put(c, r, tower, side=side)
    else:
        put(c, r, tower)


def dugout(c0, r0, c1, r1, door, material, stash):
    """A dugout: a room of `material` with its `door` cells open and the
    `stash` of crates inside."""
    clear_rect(c0, r0, c1, r1)
    inside(rect(c0 + 1, r0 + 1, c1 - 1, r1 - 1))
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) in door:
            continue
        wall(c, r, material)
    for c, r, kind in stash:
        pickup(c, r, kind)


OUR_COLS = [29, 31]
THEIR_COLS = [63, 61]
OUR_GAPS = set(range(6, 10)) | set(range(19, 24)) | set(range(33, 37))
THEIR_GAPS = set(range(9, 13)) | set(range(19, 24)) | set(range(30, 34))


def our_line():
    """The players' front line: a zig-zag parapet between columns 29 and
    31 with the trench floor behind it, crossed at three places (north, the
    road, south); two pillboxes built into it with friendly gun towers
    looking east, two dugouts behind it with stashes."""
    keep_clear(23, 0, 35, H - 1)
    trench(zigzag(OUR_COLS, 1, 39, 4, OUR_GAPS), -1)
    for r in (14, 26):
        pillbox(parapet_col(OUR_COLS, 1, 4, r), r, +1, "gun_tower")
    dugout(23, 1, 27, 5, {(24, 5), (25, 5), (26, 5)}, "concrete",
           [(24, 2, "health"), (26, 2, "minigun"), (25, 3, "ammo")])
    dugout(23, 37, 27, 41, {(24, 37), (25, 37), (26, 37)}, "concrete",
           [(24, 40, "health"), (26, 40, "plasma"), (25, 39, "missiles")])
    # a store on the fire step behind each pillbox
    pickup(27, 13, "ammo")
    pickup(26, 26, "health")


def their_line():
    """The enemy's line: a zig-zag parapet between columns 61 and 63, its
    trench floor behind it to the east, crossed at three places; four
    concrete pillboxes built into it with enemy gun towers looking west,
    two teslas either side of the road's crossing, two bio slushes behind;
    two dugouts behind the line with their stores."""
    keep_clear(57, 0, 66, H - 1)
    trench(zigzag(THEIR_COLS, 1, 39, 4, THEIR_GAPS), +1)
    for r in (6, 15, 27, 38):
        pillbox(parapet_col(THEIR_COLS, 1, 4, r), r, -1, "gun_tower", side="enemy")
    put(65, 18, "tesla", side="enemy")
    put(65, 25, "tesla", side="enemy")
    put(69, 11, "bio_slush", side="enemy")
    put(69, 31, "bio_slush", side="enemy")
    dugout(67, 2, 72, 6, {(67, 3), (67, 4), (67, 5)}, "concrete",
           [(68, 3, "health"), (71, 3, "flamethrower"), (68, 5, "grenades"), (71, 5, "ammo")])
    dugout(67, 37, 72, 41, {(67, 38), (67, 39), (67, 40)}, "concrete",
           [(68, 38, "health"), (71, 38, "air_heli"), (68, 40, "plasma"), (71, 40, "ammo")])


def wire(col, r0, r1, run=5, gap=3, offset=0, skip=()):
    """A wire belt down column `col`: runs of fence `run` cells long with
    `gap`-cell breaks, the pattern shifted by `offset` rows; never on the
    rows in `skip` (the road)."""
    period = run + gap
    for r in range(r0, r1 + 1):
        if (r + offset) % period < run and r not in skip:
            put_free(col, r, "fence")


def wire_belts():
    """Barbed wire before both lines, its breaks staggered against the
    trench crossings so the way through zig-zags."""
    wire(36, 1, 41, offset=2, skip=range(20, 24))
    wire(56, 1, 41, offset=6, skip=range(20, 24))


SHELL_HOLES = [
    # (col, row, size): the bigger, hand-placed ones
    (37, 10, 3), (52, 4, 4), (52, 13, 2), (37, 16, 2), (44, 18, 2),
    (46, 22, 3), (38, 21, 2), (52, 24, 3), (36, 27, 2), (51, 30, 4),
    (54, 34, 2), (75, 8, 3), (80, 32, 2), (20, 37, 2), (21, 4, 3), (16, 8, 2),
]


def shell_holes():
    """No man's land is pitted with shell-holes full of the karst's dark
    water: a small one is a ford that slows a hull, a bigger one has a deep
    middle a hull goes round."""
    for i, (c, r, size) in enumerate(SHELL_HOLES):
        hole(c, r, size, 200 + i)


def pockmarks():
    """The rest of no man's land pocked with small shell-holes on a
    jittered grid - kept off the set pieces, the wire and the other holes,
    so none narrows a way."""
    solid = {p for p in cells if kind_at(p) not in (None, "road", "oil", "water")}
    busy = distance_field(solid | water | INTERIOR, 3)
    n = 0
    for gr in range(2, H - 2, 4):
        for gc in range(37, 56, 4):
            if hash01(gc, gr, 211) > 0.75:
                continue
            c = gc + int(hash01(gc, gr, 213) * 3) - 1
            r = gr + int(hash01(gc, gr, 217) * 3) - 1
            size = 3 if hash01(gc, gr, 219) < 0.3 else 2
            spot = list(rect(c - 1, r - 1, c + size, r + size))
            if any(busy.get(p, 9) < 2 or p in GATE_APRON for p in spot):
                continue
            hole(c, r, size, 500 + n)
            busy = distance_field(solid | water | INTERIOR, 3)
            n += 1
    return n


def drowned_craters():
    """In the south of no man's land two great craters the rain filled,
    deep in the middle, reeds round them; on the lip between them a stash
    someone dug in and left."""
    pool(41, 36, 2.6, 401)
    pool(39, 33.5, 1.6, 402)
    pool(47.5, 33.5, 2.2, 403)
    pickup(44, 38, "fpv_swarm")
    pickup(45, 38, "health")


def chapel():
    """The ruined chapel north of the road: a nave of limestone with its
    apse east, its north wall breached, its south-west corner shot away,
    the stash in the nave."""
    for c, r in ((40, 12), (41, 12), (42, 12), (43, 12), (47, 12), (48, 12),
                 (40, 13), (40, 14),
                 (48, 13), (48, 14), (48, 15), (48, 16), (48, 17), (49, 14), (49, 15),
                 (45, 17), (46, 17), (47, 17)):
        wall(c, r, "brick")
    inside(rect(41, 13, 47, 16))
    pickup(42, 14, "emp_burst")
    pickup(46, 14, "health")
    pickup(44, 15, "plasma")


def farm():
    """The ruined farm south of the road: the farmhouse's broken walls
    round the stash, the barn with hay still in it and its door south, the
    farm's fuel drums by the road."""
    for c, r in ((40, 25), (44, 25),
                 (40, 26), (40, 27), (40, 28), (40, 29), (40, 30),
                 (44, 26), (44, 27), (44, 28), (44, 29), (44, 30),
                 (41, 30), (43, 30)):
        wall(c, r, "brick")
    inside(rect(41, 26, 43, 29))
    pickup(42, 27, "minigun")
    pickup(42, 29, "health")
    for c, r in outline(48, 25, 52, 29):
        if r == 29 and c in (49, 50, 51):
            continue
        wall(c, r, "wood")
    inside(rect(49, 26, 51, 28))
    put(49, 26, "hay")
    put(51, 26, "hay")
    pickup(50, 27, "gravity_well")
    drums([(41, 24), (42, 24)], 17, fuel=0.5)


def splintered_wood():
    """The splintered wood in the north of no man's land: stumps and dead
    trees, juniper between them, and the sonic hammer in a hollow in its
    middle."""
    for c, r in rect(38, 0, 47, 8):
        if (c, r) in cells:
            continue
        if math.hypot(c - 42.5, r - 5.5) < 2.0:
            continue  # the hollow
        u = hash01(c, r, 51)
        if u < 0.3:
            put(c, r, "snag")
        elif u < 0.42:
            put(c, r, "scots" if hash01(c, r, 53) < 0.5 else "tree")
        elif u < 0.6:
            put(c, r, "juniper")
    pickup(42, 5, "sonic_hammer")
    pickup(43, 6, "health")
    pickup(40, 4, "minigun")


def their_rear():
    """The enemy's rear: the fuel dump behind the line with an oil trail to
    the trench, the battery's two sandbag pits, their command bunker with
    two enemy gun towers at its door, and a bio slush by the south gate."""
    # the fuel dump
    drums([(c, r) for c in range(69, 73) for r in (15, 16)], 61, fuel=0.6)
    oil_trail([(68, 16), (64, 16)])
    # the battery
    for c0, r0 in ((71, 26), (79, 26)):
        for c, r in outline(c0, r0, c0 + 4, r0 + 4):
            if r == r0 and c in (c0 + 1, c0 + 2, c0 + 3):
                continue
            put(c, r, "sandbag")
    pickup(72, 28, "air_jet")
    pickup(74, 28, "health")
    pickup(80, 28, "ammo")
    pickup(82, 28, "grenades")
    # the command bunker
    for c, r in outline(76, 10, 83, 16):
        if c == 76 and r in (12, 13, 14):
            continue
        wall(c, r, "concrete")
    inside(rect(77, 11, 82, 15))
    put(76, 11, "gun_tower", side="enemy")
    put(76, 15, "gun_tower", side="enemy")
    pickup(77, 11, "health")
    pickup(77, 15, "laser")
    put(78, 34, "bio_slush", side="enemy")


def cluster(cx, cy, n, salt, fuel=0.35):
    """`n` drums packed round (cx, cy), nearest first - a heap that goes up
    together."""
    near = sorted(rect(cx - 2, cy - 2, cx + 2, cy + 2),
                  key=lambda p: ((p[0] - cx) ** 2 + (p[1] - cy) ** 2, hash01(*p, salt)))
    spots = [p for p in near if p not in cells and p not in water][:n]
    drums(spots, salt, fuel)


def traps():
    """Heaps of drums as traps: beside the ways the waves come in - near
    enough that a shot sets one off under a passing tank, never on the way
    itself (`loose_on_routes`) - and stacked round the enemy's positions,
    behind their pillboxes and at their bunker's door."""
    for (c, r), n, salt in (((58, 2), 3, 701), ((58, 41), 3, 709),
                            ((74, 18), 3, 711), ((73, 23), 3, 713), ((80, 22), 3, 717),
                            ((34, 40), 3, 721), ((39, 7), 2, 723), ((54, 36), 2, 727)):
        cluster(c, r, n, salt)


HEDGEHOGS = [(50, 8), (34, 16), (55, 19), (38, 26), (49, 38), (59, 24), (54, 28)]


def dressing():
    """What makes the frontier a place: lamps in the redoubt's corners that
    come on at dusk, supply crates stacked in its pockets, hedgehogs out
    in the open,
    palms round the pools behind the lines - the only green the dry karst
    keeps - and the stumps of the old avenue along the road."""
    for c, r in ((20, 11), (20, 31)):
        put(c, r, "lamp")
    for c, r in ((3, 13), (5, 13), (3, 29), (5, 29)):
        put(c, r, "wood_crate")
    for c, r in HEDGEHOGS:
        put_free(c, r, "tank_trap")
    for cx, cy in ((22, 5), (17, 9), (21, 38), (76, 9), (81, 33)):
        for c, r in disc(cx, cy, 2.3):
            if (c, r) not in cells and (c, r) not in reserved and hash01(c, r, 731) < 0.45:
                put(c, r, "palm")
    for c in range(37, 56, 3):
        for r in (19, 24):
            if (c, r) not in cells and hash01(c, r, 733) < 0.6:
                put(c, r, "snag")


# ---------------------------------------------------------------- the scrub


def distance_field(sources, limit):
    dist = {p: 0 for p in sources}
    q = deque(sorted(sources))
    while q:
        c, r = q.popleft()
        d = dist[(c, r)]
        if d >= limit:
            continue
        for dc, dr in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            n = (c + dc, r + dr)
            if inb(*n) and n not in dist:
                dist[n] = d + 1
                q.append(n)
    return dist


# behind the lines the frontier's own trees: holm oak, scots and black
# pine, a few dead; out between the lines nearly all dead
REAR_TREES = (("scots", 30), ("pine", 24), ("tree", 26), ("snag", 12), ("fir", 8))
NML_TREES = (("snag", 75), ("tree", 13), ("scots", 12))
SCRUB = (("juniper", 35), ("autumn_bush", 25), ("bush", 20), ("tall_grass", 20))
BANK = (("reeds", 70), ("juniper", 30))


def in_nml(c):
    return 36 <= c <= 57


def tree_for(c, r):
    return pick(NML_TREES if in_nml(c) else REAR_TREES, hash01(c, r, 79))


GATE_APRON = set()


def scrub():
    """Everything the war left: a belt of trees two deep along the edges,
    copses behind the lines where the noise is high and nothing is near,
    scrub and dry grass in patches, reeds round the shell-holes; out
    between the lines sparse, dead trees and scrub only."""
    wd = distance_field(water, 2)
    solid = {p for p in cells if blocks(p)}
    near = distance_field(solid | OPEN_GROUND, 2)
    for r in range(H):
        for c in range(W):
            p = (c, r)
            if p in cells or p in reserved or p in GATE_APRON or p in INTERIOR:
                continue
            if min(math.hypot(c - s[0], r - s[1]) for s in (START, START2)) <= 4:
                continue
            u = hash01(c, r, 13)
            v = hash01(c, r, 19)
            edge = min(c, r, W - 1 - c, H - 1 - r)
            d = fbm(c / 5, r / 5, 11)
            room = near.get(p, 9) >= 2
            if wd.get(p, 9) == 1 and hash01(c, r, 85) < 0.5:
                put(c, r, pick(BANK, hash01(c, r, 87)))
            elif edge <= 1 and not in_nml(c):
                put(c, r, tree_for(c, r))
            elif edge == 0:
                put(c, r, tree_for(c, r) if u < 0.6 else pick(SCRUB, v))
            elif room and d > (0.68 if in_nml(c) else 0.58) and u < 0.8:
                put(c, r, tree_for(c, r))
            elif d > 0.5 and v < (0.22 if in_nml(c) else 0.4):
                put(c, r, pick(SCRUB, hash01(c, r, 23)))
            elif v < 0.03:
                put(c, r, pick(SCRUB, hash01(c, r, 29)))
            else:
                continue
            reserved.discard(p)


# ---------------------------------------------------------------- the nav grid
#
# The game routes on a grid of 32 px cells whose centres sit half a cell off
# the map's (`pathfind::Grid`, `maplint`): a tile at map cell (c, r) blocks
# nav cells c-1..c by r-1..r. So two tiles one cell apart leave no nav cell
# between them, and a way the AI can take needs two empty map cells side by
# side. Everything below reasons on that grid, the way the linter does.

LOOSE = {"barrel", "hay", "wood_crate", "barrier"}
SOFT = {"road", "water", "pickup", "oil", "portal", "start", "start2", "gate"} | COVER_KINDS | LOOSE
GATE_LANES = set()


def is_deep(c, r):
    return all((c + dc, r + dr) in water for dc in (-1, 0, 1) for dr in (-1, 0, 1))


def blocks(p):
    """Whether map cell `p` holds something the nav grid treats as a wall:
    any fixed tile, deep water."""
    if not inb(*p):
        return False
    if p in water:
        return is_deep(*p)
    k = kind_at(p)
    if k is None:
        return False
    return k not in SOFT


def nav_open(k, l):
    """An open nav cell. The field's own edge is a wall with the same
    margin as a tile's, so the outermost ring counts as shut - but for the
    gates' lanes, which the game holds open for the waves."""
    if not (0 <= k < W and 0 <= l < H):
        return False
    if (k in (0, W - 1) or l in (0, H - 1)) and (k, l) not in GATE_LANES:
        return False
    return not any(blocks((k + dc, l + dr)) for dc in (0, 1) for dr in (0, 1))


def nav_of(p):
    c, r = p
    return [(c - 1, r - 1), (c, r - 1), (c - 1, r), (c, r)]


def nav_reach():
    start = next(n for n in nav_of(START) if nav_open(*n))
    seen = {start}
    q = deque([start])
    while q:
        k, l = q.popleft()
        for n in ((k + 1, l), (k - 1, l), (k, l + 1), (k, l - 1)):
            if n not in seen and nav_open(*n):
                seen.add(n)
                q.append(n)
    return seen


def reachable(p, reach):
    return any(n in reach for n in nav_of(p))


def carvable(p):
    k = kind_at(p)
    return p not in roads and p not in water and (k is None or k in TREE_KINDS or k in COVER_KINDS)


def fill_pockets():
    """Open ground the playfield never reaches becomes scrub or a dead
    tree, so no nav cell is left boxed in or cut off."""
    filled = 0
    for _ in range(10):
        reach = nav_reach()
        stray = [(k, l) for l in range(H) for k in range(W) if (k, l) not in reach and nav_open(k, l)]
        if not stray:
            break
        for k, l in stray:
            if not nav_open(k, l):
                continue
            corners = [(k + dc, l + dr) for dc in (0, 1) for dr in (0, 1)]
            corners.sort(key=lambda p: (sum(n in reach for n in nav_of(p)), p[1], p[0]))
            for p in corners:
                if (kind_at(p) is None or kind_at(p) in COVER_KINDS) and p not in water and p not in roads \
                        and p not in OPEN_GROUND:
                    put(*p, tree_for(*p))
                    filled += 1
                    break
    return filled


def close_narrow_trails():
    """An empty cell one wide among trees - something on both sides across
    it - is a trail a hull can take but the AI never will (the linter's
    narrow trail): it grows scrub, soft cover that changes no route."""
    taken = lambda p: p in cells or not inb(*p)
    filled = 0
    for _ in range(3):
        grown = []
        for r in range(H):
            for c in range(W):
                p = (c, r)
                if p in cells:
                    continue
                trees = sum(kind_at((c + a, r + b)) in TREE_KINDS for a in (-1, 0, 1) for b in (-1, 0, 1))
                across = (taken((c - 1, r)) and taken((c + 1, r))) or (taken((c, r - 1)) and taken((c, r + 1)))
                if trees >= 2 and across:
                    grown.append(p)
        if not grown:
            break
        for p in grown:
            put(*p, "juniper")
        filled += len(grown)
    return filled


def widen_narrow():
    """A way one nav cell wide that a tree, a stump or a dragon's tooth
    narrows is widened by taking that one away: the linter's narrow
    corridor is a scrape for a hull and a pile-up for a wave, and the
    trees are dressing. The set pieces' own walls are laid out so they
    leave none."""
    removed = 0
    for _ in range(8):
        thin = sorted(narrow(), key=lambda p: (p[1], p[0]))
        took = False
        for k, l in thin:
            for c in range(k - 1, k + 3):
                for r in range(l - 1, l + 3):
                    if kind_at((c, r)) in TREE_KINDS and (c, r) not in GATE_APRON:
                        clear(c, r)
                        removed += 1
                        took = True
        if not took:
            break
    return removed


def gate_aprons():
    """The lane a wave rolls in by: the trees and cover cleared five cells
    in from every gate."""
    for gc, gr in GATES:
        if gr == 0:
            area = rect(gc - 2, 0, gc + 3, 5)
        elif gr == H - 1:
            area = rect(gc - 2, H - 6, gc + 3, H - 1)
        elif gc == 0:
            area = rect(0, gr - 2, 5, gr + 3)
        else:
            area = rect(W - 6, gr - 2, W - 1, gr + 3)
        for p in list(area):
            if carvable(p):
                clear(*p)
            OPEN_GROUND.add(p)
            GATE_APRON.add(p)


def lane(gc, gr):
    """The nav cells a wave tank rolls in over from a gate."""
    k, l = min(gc, W - 1), min(gr, H - 1)
    if gr == 0:
        return [(k, l + i) for i in range(4)]
    if gr == H - 1:
        return [(k, l - i) for i in range(4)]
    if gc == 0:
        return [(k + i, l) for i in range(4)]
    return [(k - i, l) for i in range(4)]


def boxed_in():
    """Open nav cells whose four neighbours are all shut: a trap the linter
    calls an error."""
    out = []
    for l in range(H):
        for k in range(W):
            if not nav_open(k, l):
                continue
            if not any(nav_open(*n) for n in ((k + 1, l), (k - 1, l), (k, l + 1), (k, l - 1))):
                out.append((k, l))
    return out


# ---------------------------------------------------------------- the walk


def walk_costs(start=START):
    s = next(n for n in nav_of(start) if nav_open(*n))
    cost = {s: 0}
    heap = [(0, s)]
    while heap:
        d, (k, l) = heapq.heappop(heap)
        if d > cost.get((k, l), 1 << 30):
            continue
        for n in ((k + 1, l), (k - 1, l), (k, l + 1), (k, l - 1)):
            if not nav_open(*n):
                continue
            nd = d + (3 if n in water else 1)
            if nd < cost.get(n, 1 << 30):
                cost[n] = nd
                heapq.heappush(heap, (nd, n))
    return cost


def wave_routes():
    """The map cells on each gate's cheapest walk to either start - the way
    a wave comes in. A loose drum or prop on one is a thing the router
    drives straight at (it routes through loose props) and the hull then
    has to shove out of the way, so the frontier keeps them beside the
    ways in."""
    cells_on = set()
    for start in (START, START2):
        cost = walk_costs(start)
        for gc, gr in GATES:
            n = lane(gc, gr)[-1]
            if n not in cost:
                continue
            while cost[n] > 0:
                k, l = n
                cells_on.update(((k, l), (k + 1, l), (k, l + 1), (k + 1, l + 1)))
                n = min(((k + 1, l), (k - 1, l), (k, l + 1), (k, l - 1)),
                        key=lambda m: (cost.get(m, 1 << 30), m[1], m[0]))
    return cells_on


def loose_on_routes():
    on = wave_routes()
    near = lambda c, r: any((c + dc, r + dr) in on for dc in (-1, 0, 1) for dr in (-1, 0, 1))
    return sorted((c, r) for (c, r) in cells if kind_at((c, r)) in LOOSE and near(c, r))


# ---------------------------------------------------------------- the map


def build():
    main_road()
    redoubt()
    field_hospital()
    ammo_dump()
    our_line()
    their_line()
    wire_belts()
    shell_holes()
    drowned_craters()
    chapel()
    farm()
    splintered_wood()
    their_rear()
    traps()
    gate_aprons()
    pockmarks()
    dressing()
    for gc, gr in GATES:
        put(gc, gr, "gate")
        GATE_LANES.update(lane(gc, gr))
    scrub()
    widened = widen_narrow()
    filled = fill_pockets()
    close_narrow_trails()
    return filled, widened


HEADER = """\
# no-mans-land - No Man's Land, level 11: two trench lines across a dry
# karst frontier, and the ground between them.
# Written by tools/gen_no_mans_land.py - change the generator and run it
# again rather than editing this file; re-saving from the map editor would
# drop these comments. Level tables as dotted keys on purpose - a `[spawn]`
# header would swallow every `cells.` line after it.
#
# Destroy on the waves plan: seven waves of 3 to 9 tanks, 42 in all, light
# ones first and heavy ones last - a war of attrition. They roll in at seven
# gates: three on the east edge behind the enemy's line, one north and one
# south behind it, and one north and one south into no man's land itself.
# 87 x 43 cells, followed by the camera. The sky is clear, dusk, fog, a
# sandstorm or heat haze, by the round's seed. Drawn as the dry karst
# frontier (`biome`, docs/biomes.md): warm hardpan, limestone plates, dark
# pools, dry scrub in the walls' shelter.
#
# The players start in the redoubt in the west: a concrete wall with three
# gates - east onto the road, with a gun tower either side, and north to the
# field hospital and south to the ammunition dump, a gun tower beside each.
# Inside each gate a concrete traverse stands across the way, so no gate
# has a straight line to the starts: a tank that gets in turns before it
# sees them, under a tesla between the starts and a bio slush by the south
# way. The stores: the two gauss rails behind the east traverse,
# a helicopter and a jet of air support in the east ways, grenades and
# missiles in the far pockets, plasma and a minigun in the north and south
# ways, and in the command dugout against the west wall health, ammo,
# missiles, a shield and a tower pack. North of it the field hospital, a
# canvas ward of health with a shield and a speed-up; south of it the
# ammunition dump, a pit of crates (ammo, grenades, a helicopter, missiles,
# a minigun) and a pit of fuel drums on an oil trail.
#
# The players' front line runs down the field in front of it: a zig-zag
# parapet of sandbags with its trench dug behind it, crossed at three places
# (north, on the road, south), two concrete pillboxes in it with friendly
# gun towers looking east, two dugouts behind it (a minigun, plasma,
# missiles, health, ammo), and a belt of wire out in front.
#
# No man's land is pitted with shell-holes full of dark water - the small
# ones a ford, the bigger ones deep in the middle - and the old road across
# it is broken by them. In it: the splintered wood in the north, the sonic
# hammer and a minigun in a hollow among the dead trees; the ruined chapel
# on the road (an EMP, plasma, health); the ruined farm south of the road
# (a minigun and health in the house, the gravity well in the barn among
# the hay); two great drowned craters in the south, an FPV swarm and health
# on the lip between them.
#
# The enemy's line faces it: the same parapet with its trench behind,
# four concrete pillboxes in it with enemy gun towers looking west, two
# enemy teslas either side of the road's crossing, bio slushes behind,
# wire in front, and two dugouts with their stores (health, ammo, a
# flamethrower, grenades, a helicopter, plasma). Behind it the fuel dump on
# an oil trail to their trench, the battery's two sandbag pits (a jet,
# grenades, health, ammo) and the command bunker, two enemy gun towers at
# its door and the laser inside. Heaps of drums lie beside the ways in and
# round the enemy's positions, 38 drums in all; scots and black pine, holm
# oak and a few palms behind the lines, dead trees between them.
"""


def write(path):
    lines = [HEADER, "version = 1",
             'tank = "warden"',
             'tank2 = "assault"',
             'theme = "desert"',
             'biome = "karst"',
             'weather = ["clear", "dusk", "fog", "sandstorm", "heat_haze"]',
             "size = [87, 43]",
             'mission.kind = "destroy"',
             'spawn.kind = "waves"',
             "spawn.waves = 7",
             "spawn.size = 3",
             "spawn.growth = 1",
             'spawn.tier_start = "light"',
             'spawn.tier_end = "heavy"',
             ""]
    for (c, r) in sorted(cells, key=lambda p: (p[1], p[0])):
        lines.append(f'cells."{c},{r}" = {cells[(c, r)]}')
    with open(path, "w") as f:
        f.write("\n".join(lines) + "\n")


def sight_line(a, b):
    """The map cells a straight line from cell `a` to cell `b` crosses,
    sampled finely, ends left out."""
    (x0, y0), (x1, y1) = a, b
    n = int(max(abs(x1 - x0), abs(y1 - y0)) * 6) + 1
    out = []
    for i in range(1, n):
        t = i / n
        p = (round(x0 + (x1 - x0) * t), round(y0 + (y1 - y0) * t))
        if p not in out and p not in (a, b):
            out.append(p)
    return out


def check():
    problems = []
    for gc, gr in GATES:
        if not (gc in (0, W - 1) or gr in (0, H - 1)):
            problems.append(f"gate {gc},{gr} is not on the edge")
    reach = nav_reach()
    near = lambda p, rr: any((p[0] + dc, p[1] + dr) in rr for dc in range(-2, 2) for dr in range(-2, 2))
    for p in (START, START2):
        if not near(p, reach):
            problems.append(f"start {p} cannot be reached")
    for c, r, kind in pickups:
        if not reachable((c, r), reach):
            problems.append(f"{kind} at {c},{r} cannot be reached")
    for gc, gr in GATES:
        if not all(nav_open(*n) for n in lane(gc, gr)):
            problems.append(f"gate {gc},{gr}: its lane is shut")
        elif lane(gc, gr)[-1] not in reach:
            problems.append(f"gate {gc},{gr} is cut off")
    for k, l in boxed_in():
        problems.append(f"nav cell {k},{l} is boxed in")
    for c, r in DRUMS:
        if math.hypot(c - START[0], r - START[1]) < 7 or math.hypot(c - START2[0], r - START2[1]) < 7:
            problems.append(f"drum at {c},{r} is by a start")
    for c, r in loose_on_routes():
        problems.append(f"{kind_at((c, r))} at {c},{r} stands on a wave's way in")
    # the redoubt's wall stands whole but for its gates
    for p in outline(*HQ):
        if p not in HQ_GATES and kind_at(p) not in ("wall", "gun_tower"):
            problems.append(f"the redoubt's wall is open at {p}")
    # no gate of the redoubt has a straight line to a start: something
    # solid stands on every line from a gate cell to either start
    solid = lambda p: kind_at(p) in ("wall", "gun_tower", "tesla", "bio_slush")
    for g in sorted(HQ_GATES):
        for s in (START, START2):
            if not any(solid(p) for p in sight_line(g, s)):
                problems.append(f"the gate at {g} has a straight line to the start at {s}")
    return problems


ASCII = {
    'wall': '#', 'road': '=', 'water': '~', 'start': '1', 'start2': '2',
    'pickup': 'P', 'sandbag': 's', 'fence': 'f', 'tree': 'T', 'pine': 'T', 'spruce': 'T', 'scots': 'T',
    'fir': 'T', 'palm': 'Y', 'snag': 'x', 'willow': 'W', 'birch': 'b', 'tall_grass': '"', 'bush': 'o',
    'berry_bush': 'o', 'autumn_bush': 'o', 'juniper': 'o', 'fern': ',', 'reeds': '|', 'gate': 'G',
    'tesla': 'Z', 'gun_tower': 'K', 'bio_slush': 'Q', 'barrel': 'D', 'hay': 'h', 'tyres': 'O',
    'wood_crate': 'X', 'barrier': 'n', 'tank_trap': '*', 'lamp': 'L', 'oil': ':', 'target': '@',
}


def ascii_map(routes=False):
    on = wave_routes() if routes else set()
    print("    " + "".join(str(c // 10 % 10) for c in range(W)))
    print("    " + "".join(str(c % 10) for c in range(W)))
    for r in range(H):
        row = []
        for c in range(W):
            k = kind_at((c, r))
            ch = ASCII.get(k, '?') if k else ('`' if (c, r) in on else '.')
            if k == 'wall':
                ch = {'brick': '#', 'iron': 'I', 'wood': 'w', 'concrete': 'C', 'glass': 'g'}.get(
                    cells[(c, r)].split('"')[3], '#')
            if k == 'water' and is_deep(c, r):
                ch = '%'
            if k == 'barrel':
                ch = 'F' if 'fuel' in cells[(c, r)] else 'D'
            if k in ('gun_tower', 'tesla', 'bio_slush') and 'enemy' in cells[(c, r)]:
                ch = ch.lower()
            if k == 'pickup':
                ch = cells[(c, r)].split('"')[3][0].upper()
            row.append(ch)
        print(f"{r:3d} " + "".join(row))


def narrow():
    """Open nav cells one wide - shut both sides on one axis, open along the
    other: the linter's narrow corridors, where a hull scrapes by."""
    out = set()
    for l in range(1, H - 1):
        for k in range(1, W - 1):
            if not nav_open(k, l):
                continue
            x_sealed = not nav_open(k - 1, l) and not nav_open(k + 1, l)
            y_sealed = not nav_open(k, l - 1) and not nav_open(k, l + 1)
            x_flows = nav_open(k - 1, l) or nav_open(k + 1, l)
            y_flows = nav_open(k, l - 1) or nav_open(k, l + 1)
            if (x_sealed and y_flows) or (y_sealed and x_flows):
                out.add((k, l))
    return out


def nav_map():
    reach = nav_reach()
    thin = narrow()
    print("nav: '.' reached, '+' one wide, '!' open but cut off, ' ' shut; one wide:", sorted(thin))
    print("    " + "".join(str(c // 10 % 10) for c in range(W)))
    print("    " + "".join(str(c % 10) for c in range(W)))
    for l in range(H):
        print(f"{l:3d} " + "".join("+" if (k, l) in thin else "." if (k, l) in reach else "!" if nav_open(k, l) else " "
                                   for k in range(W)))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("-o", "--out", default="maps/no-mans-land.toml")
    ap.add_argument("--stats", action="store_true")
    ap.add_argument("--ascii", action="store_true")
    ap.add_argument("--nav", action="store_true")
    ap.add_argument("--routes", action="store_true", help="with --ascii, mark the waves' ways in")
    a = ap.parse_args()
    filled, widened = build()
    if a.ascii:
        ascii_map(a.routes)
    if a.nav:
        nav_map()
    problems = check()
    for p in problems:
        print("CHECK:", p)
    write(a.out)
    if a.stats:
        kinds = {}
        for t in cells.values():
            k = t.split('"')[1]
            kinds[k] = kinds.get(k, 0) + 1
        print(f"{len(cells)} cells, {filled} pocket cells filled, {widened} trees cleared off narrow ways, {len(narrow())} one-wide nav cells")
        for k, n in sorted(kinds.items(), key=lambda kv: -kv[1]):
            print(f"  {k:12} {n}")
        cost = walk_costs()
        for gc, gr in GATES:
            inside_ = lane(gc, gr)[-1]
            c = cost.get(inside_)
            print(f"  gate {gc},{gr}: walk {c} cells = {c * 32 / 160:.1f} s" if c else f"  gate {gc},{gr}: no walk")
        for c, r, kind in pickups:
            print(f"  pickup {kind:13} {c},{r}")
    if problems:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
