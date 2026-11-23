"""Write maps/archipelago.toml: Archipelago, level 9 - island hopping
between two fortified bong islands, 97 x 48 cells.

  python3 tools/gen_archipelago.py [-o maps/archipelago.toml] [--stats] [--ascii] [--nav]

A map this size is too big to lay out by hand, so this script is the
source: change it and run it again rather than editing the TOML (a
re-save from the map editor would drop the header too). Nothing is drawn
at random - the shores and the planting are hashed from cell positions -
so the script writes the same file every time. `--ascii` prints the map a
character a cell, `--nav` the nav grid the AI routes on (what is reached,
what is one cell wide), `--stats` the counts and each gate's walk to the
start. The checks run every time and fail the run: every pickup, start,
crossing and gate lane reached (the portals carrying a hull between them),
no boxed-in nav cell, every ford shallow from shore to shore, no drum by a
start or on a trail, no blast weapon's crate within 288 px of the players'
bong, and no loose drum or prop on a wave's cheapest way in.

The sea is the map: whatever is not an island is water, and the rules read
its shape (docs/water.md). Open water - a water cell whose eight
neighbours are all water - is deep, a wall to hulls and nothing to shells;
water beside land is a ford. So two shores two cells apart make a ford a
hull wades across, three apart leave a deep line down the middle, and a
road one cell wide laid over the water is a plank bridge (dry; the
valley's look draws it as a deck).

What it lays out (the header written into the map says how it plays):

- Gloop Cay in the west, the players' island: the old shore battery, a
  curtain wall round the bong with three gates, a gun tower and a baffle
  at each, a tesla and a bio slush, the stores and the air support
  inside; a portal on its north point;
- the north chain - Palm Key, Lighthouse Rock, Turtle Isle - and the south
  chain - Fisher's Isle, Coral Isle, Mangrove Isle - each island joined to
  the next by a ford or a plank bridge, a cache on every one;
- Customs Isle in the middle-west, the main bridge from the battery onto
  it, fords north and south to both chains;
- the lagoon in the middle, the treasure islet in it, deep water all
  round: a long plank pier from the customs house and the portals are
  the ways on;
- the Corsair outpost on the middle-east island, an enemy gun tower in
  its sandbags;
- Blip Atoll in the east, the enemy's: a walled fort round its bong,
  towers at every gate, a fuel dump, a portal on its south-west point,
  the reinforcements' landing beaches on the east edge;
- two outlying islets, each a ford off the players' island, each with a
  weapon cache: Gull Rock and Wreck Reef.
"""

import argparse
import math
from collections import deque
import heapq

W, H = 97, 48

land = set()
# road cells laid over the sea: the plank bridges
decks = set()
# (col, row) -> inline TOML table text, for every cell that is not plain
# water or plain ground
cells = {}
# cells the planting pass must leave alone
reserved = set()

BONG = (12, 25)
START = (7, 23)
START2 = (7, 27)
ENEMY_BONG = (86, 24)
# the battery's curtain wall, (c0, r0, c1, r1)
FORT = (3, 16, 21, 33)

GATES = [
    (61, 0),  # north, Turtle Isle's nesting beach
    (48, H - 1),  # south, Coral Isle's beach
    (W - 1, 11), (W - 1, 37),  # east, the atoll's landing beaches
]

PORTALS = [(8, 11), (49, 24), (79, 39)]

pickups = []  # (c, r, kind)
DRUMS = []
# what a set piece tried to put in the sea: each fails the run
MISPLACED = []

TREE_KINDS = {"tree", "pine", "spruce", "scots", "fir", "birch", "willow", "palm", "snag"}
COVER_KINDS = {"tall_grass", "bush", "berry_bush", "juniper", "fern", "autumn_bush", "reeds"}
# the blast weapons: their crates keep away from the players' bong
# (docs/levels.md, "Blasts away from the bong")
BLAST = {"grenades", "fpv_swarm", "air_heli", "air_jet", "gravity_well"}
# two of the widest blast's radii (the gravity well's 140 px), and more than
# the 256 px the grenade's two need
BLAST_CLEAR = 288


def inb(c, r):
    return 0 <= c < W and 0 <= r < H


def clamp(c, r):
    return min(max(c, 0), W - 1), min(max(r, 0), H - 1)


def obj(kind, **kw):
    parts = [f'kind = "{kind}"'] + [f'{k} = "{v}"' for k, v in kw.items()]
    return "{ " + ", ".join(parts) + " }"


def put(c, r, kind, **kw):
    if not inb(c, r):
        return
    if kind != "road" and (c, r) not in land:
        MISPLACED.append(f"{kind} at {c},{r} would stand in the sea")
        return
    cells[(c, r)] = obj(kind, **kw)
    reserved.add((c, r))


def wall(c, r, material):
    put(c, r, "wall", material=material)


def clear(c, r):
    if inb(c, r):
        if (c, r) not in decks:
            cells.pop((c, r), None)
        reserved.add((c, r))


def put_free(c, r, kind, **kw):
    """A decoration that gives way to anything already placed."""
    if inb(c, r) and (c, r) not in cells and (c, r) in land:
        put(c, r, kind, **kw)


def kind_at(p):
    t = cells.get(p)
    return t.split('"')[1] if t else None


def is_water(p):
    c, r = clamp(*p)
    return (c, r) not in land and (c, r) not in decks


def is_deep(c, r):
    """Open water: a water cell whose eight neighbours are water too (the
    map's edge reads as the cell beside it, as the ground does)."""
    return all(is_water((c + dc, r + dr)) for dc in (-1, 0, 1) for dr in (-1, 0, 1))


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


def ring(cx, cy, rad):
    """The cells of a circle's outline: those inside the disc with any of
    their eight neighbours outside it, so the ring is closed side to side
    and reads as one wall."""
    inside = set(disc(cx, cy, rad))
    for c, r in sorted(inside, key=lambda p: (p[1], p[0])):
        if any((c + dc, r + dr) not in inside for dc in (-1, 0, 1) for dr in (-1, 0, 1)):
            yield c, r


OPEN_GROUND = set()
INTERIOR = set()
# the trails' and roads' own cells: the ways the AI drives, which drums
# keep off - a hull that meets a drum on its way stops to shoot it out of
# the way (`Brain::wants_breach`) and stands in its own blast's reach
WAYS = set()


def inside(cells_):
    INTERIOR.update(cells_)


def clear_rect(c0, r0, c1, r1):
    for c, r in list(rect(c0, r0, c1, r1)):
        clear(c, r)
        OPEN_GROUND.add((c, r))


def clear_disc(cx, cy, rad):
    for c, r in list(disc(cx, cy, rad)):
        clear(c, r)
        OPEN_GROUND.add((c, r))


def keep_clear(c0, r0, c1, r1):
    for p in rect(c0, r0, c1, r1):
        reserved.add(p)
        OPEN_GROUND.add(p)


def pickup(c, r, kind):
    if (c, r) not in land or (c, r) in decks:
        MISPLACED.append(f"{kind} at {c},{r} would cover water or a bridge")
        return
    put(c, r, "pickup", pickup=kind)
    pickups.append((c, r, kind))
    # the planting keeps a crate's own cell's neighbours open
    for q in rect(c - 1, r - 1, c + 1, r + 1):
        if q in land and q not in cells:
            reserved.add(q)


def hut(c0, r0, c1, r1, doors, material):
    """A building's walls round (c0, r0)..(c1, r1), open at `doors` (a set
    of cells on the outline), its floor kept clear of the planting."""
    clear_rect(c0, r0, c1, r1)
    inside(rect(c0 + 1, r0 + 1, c1 - 1, r1 - 1))
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) not in doors and (c, r) in land:
            wall(c, r, material)


def drum(c, r, kind):
    put(c, r, "barrel", drum=kind)
    DRUMS.append((c, r))


def drums(spots, salt, fuel=0.35):
    """Drums at `spots`, each oil or fuel by a hash of its cell: `fuel` of
    them the grey fuel drums that launch when chained."""
    for c, r in spots:
        drum(c, r, "fuel" if hash01(c, r, salt) < fuel else "oil")


def cluster(cx, cy, n, salt, fuel=0.35):
    """`n` drums packed round (cx, cy), nearest first - a heap that goes up
    together."""
    near = sorted(rect(cx - 2, cy - 2, cx + 2, cy + 2),
                  key=lambda p: ((p[0] - cx) ** 2 + (p[1] - cy) ** 2, hash01(*p, salt)))
    spots = [p for p in near if p not in cells and p in land and p not in WAYS][:n]
    drums(spots, salt, fuel)


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


def track(points, width=2, margin=1):
    """A sandy track `width` cells across over the land, from each point
    down and to the right, kept clear `margin` round it."""
    path = polyline(points)
    for c, r in path:
        for dc in range(width):
            for dr in range(width):
                p = (c + dc, r + dr)
                if p in land:
                    put(*p, "road")
                WAYS.add(p)
    for c, r in path:
        for dc in range(-margin, width + margin):
            for dr in range(-margin, width + margin):
                p = (c + dc, r + dr)
                if inb(*p):
                    reserved.add(p)
                    OPEN_GROUND.add(p)
    return path


def trail(points, half=1):
    """A way across an island kept open - bare sand and grass, no road -
    `2 * half + 1` cells across."""
    for c, r in polyline(points):
        for p in rect(c - half, r - half, c + half, r + half):
            if p in land and kind_at(p) in TREE_KINDS | COVER_KINDS:
                clear(*p)
            reserved.add(p)
            OPEN_GROUND.add(p)
            WAYS.add(p)


# ---------------------------------------------------------------- the islands

# Each island a union of ellipses (cx, cy, rx, ry) with a shore ragged by
# noise, and rectangles of land laid over them where a crossing needs a
# shore just so.
ISLANDS = {
    "gloop": [(11.5, 25, 10.4, 12.2), (8.5, 12.0, 5.8, 3.8), (9.5, 37.5, 5.6, 3.2)],
    "gull": [(6.5, 3.0, 5.4, 2.7)],
    "wreck": [(8.0, 45.2, 5.4, 2.4)],
    "palm": [(29.5, 6.5, 7.6, 4.6), (21.5, 11.0, 3.8, 2.8)],
    "lighthouse": [(45, 4.5, 6.0, 4.0)],
    "turtle": [(61.5, 7.0, 7.2, 5.2)],
    "customs": [(33, 25, 6.6, 6.6)],
    "fisher": [(31, 41.5, 8.4, 4.2), (23.5, 39.5, 3.4, 2.4)],
    "coral": [(48, 42.5, 5.4, 3.8)],
    "mangrove": [(62.5, 40.0, 7.0, 5.0)],
    "corsair": [(64, 24.5, 6.4, 6.4)],
    "treasure": [(49, 24.5, 4.4, 3.4)],
    "atoll": [(86.5, 24.5, 11.5, 14.5), (78.5, 12.0, 4.8, 3.8), (78.5, 37.0, 4.8, 3.8)],
}

# Land laid over the ellipses: the battery's ground, the gates' beaches, a
# hut's footing.
LAND_RECTS = [
    (2, 15, 22, 35),  # Gloop Cay: the battery and the strip round its wall
    (2, 1, 8, 4),  # Gull Rock's signal hut
    (58, 0, 64, 3),  # Turtle Isle's nesting beach, the north gate
    (45, 44, 51, H - 1),  # Coral Isle's beach, the south gate
    (91, 9, W - 1, 15), (91, 33, W - 1, 38),  # the atoll's landing beaches
]

# Fords: (name, strait, near landing, far landing) as rectangles. The strait
# is water two cells across from landing to landing; the landings are land,
# so every cell of the strait has a shore beside it and is shallow.
FORDS = [
    ("Gull Rock", (5, 7, 9, 8), (5, 5, 9, 6), (5, 9, 9, 10)),
    ("Wreck Reef", (5, 42, 9, 43), (5, 40, 9, 41), (5, 44, 9, 45)),
    ("Palm Key", (16, 10, 17, 13), (13, 10, 15, 13), (18, 10, 20, 13)),
    ("Fisher's Isle", (18, 36, 22, 37), (18, 34, 22, 35), (18, 38, 22, 39)),
    ("Customs Isle's north shore", (31, 14, 35, 15), (31, 11, 35, 13), (31, 16, 35, 19)),
    ("Customs Isle's south shore", (31, 34, 35, 35), (31, 32, 35, 33), (31, 36, 35, 38)),
    ("Turtle Isle", (52, 3, 53, 7), (49, 3, 51, 7), (54, 3, 56, 7)),
    ("the outpost's north shore", (62, 15, 66, 16), (62, 12, 66, 14), (62, 17, 66, 19)),
    ("the outpost's south shore", (62, 32, 66, 33), (62, 30, 66, 31), (62, 34, 66, 36)),
    ("Mangrove Isle", (54, 39, 55, 43), (51, 39, 53, 43), (56, 39, 58, 43)),
    ("the atoll's south-west shore", (71, 36, 72, 40), (68, 36, 70, 40), (73, 36, 75, 40)),
]

# Plank bridges: one cell wide, from a point on one shore to a point on the
# other, laid as road over the sea (and over the shore they meet) - so every
# deck cell has the sea on both sides, which the valley's look draws as
# planks (`ground::GroundGrid::bridge`), and the water either side of it is
# a ford: three cells a hull can cross on, the middle one dry.
BRIDGES = [
    ("the main bridge", (22, 24), (28, 24)),
    ("the lighthouse bridge", (36, 6), (41, 6)),
    ("the coral bridge", (39, 41), (43, 41)),
    ("the atoll's north bridge", (67, 11), (75, 11)),
    ("the atoll's west bridge", (70, 24), (76, 24)),
    ("the treasure pier", (39, 24), (46, 24)),
]


def shape_islands():
    for name, blobs in ISLANDS.items():
        salt = sum(map(ord, name))
        for cx, cy, rx, ry in blobs:
            for r in range(int(cy - ry) - 2, int(cy + ry) + 3):
                for c in range(int(cx - rx) - 2, int(cx + rx) + 3):
                    if not inb(c, r):
                        continue
                    dx, dy = (c - cx) / rx, (r - cy) / ry
                    wobble = 0.34 * (fbm(c / 3, r / 3, salt) - 0.5)
                    if dx * dx + dy * dy <= 1.0 + wobble:
                        land.add((c, r))
    for c0, r0, c1, r1 in LAND_RECTS:
        land.update(rect(c0, r0, c1, r1))
    # smooth the shores: no lone rock, no one-cell inlet
    for _ in range(2):
        grown, eaten = [], []
        for r in range(H):
            for c in range(W):
                n = sum((c + dc, r + dr) in land for dc in (-1, 0, 1) for dr in (-1, 0, 1) if dc or dr)
                if (c, r) in land and n <= 2:
                    eaten.append((c, r))
                elif (c, r) not in land and n >= 6:
                    grown.append((c, r))
        land.difference_update(eaten)
        land.update(grown)
    for _, strait, near, far in FORDS:
        land.update(rect(*near))
        land.update(rect(*far))
        land.difference_update(rect(*strait))
    for _, a, b in BRIDGES:
        for p in polyline([a, b]):
            if p not in land:
                decks.add(p)
            cells[p] = obj("road")
            reserved.add(p)
            OPEN_GROUND.add(p)
            WAYS.add(p)


# ---------------------------------------------------------------- set pieces


def battery():
    """Gloop Cay's shore battery, the players' base: a curtain wall of
    concrete with iron bastions round the whole of the island's middle,
    three gates - the sea gate east onto the main bridge, the land gates
    north and south - each with a gun tower beside it, and inside them one
    baffle wall bent round the north, east and south of the bong, open
    only to the west, so no way in has a straight shot at it; a tesla by
    the bong and a bio slush at the back. The stores round the bong, the
    blast weapons in the wall's corners - every one at least nine cells off
    the bong."""
    c0, r0, c1, r1 = FORT
    bx, by = BONG
    clear_rect(c0, r0, c1, r1)
    inside(rect(c0, r0, c1, r1))
    chamfer = {(c0, r0), (c0 + 1, r0), (c0, r0 + 1), (c1, r0), (c1 - 1, r0), (c1, r0 + 1),
               (c0, r1), (c0 + 1, r1), (c0, r1 - 1), (c1, r1), (c1 - 1, r1), (c1, r1 - 1)}
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) in chamfer:
            continue
        if c == c1 and by - 1 <= r <= by + 1:
            continue  # the sea gate
        if r in (r0, r1) and bx - 1 <= c <= bx + 1:
            continue  # the land gates
        wall(c, r, "concrete")
    for c, r in ((c0 + 1, r0 + 1), (c1 - 1, r0 + 1), (c0 + 1, r1 - 1), (c1 - 1, r1 - 1)):
        wall(c, r, "iron")  # the bastions
    # the baffles: one wall bent round the north, east and south of the
    # bong, open only to the west - every way in from a gate goes round it
    for c in range(bx - 3, bx + 6):
        wall(c, by - 5, "concrete")
        wall(c, by + 4, "concrete")
    for r in range(by - 4, by + 4):
        wall(bx + 5, r, "concrete")
    put(c1, by - 3, "gun_tower")
    put(c1, by + 3, "gun_tower")
    put(bx + 2, r0, "gun_tower")
    put(bx - 2, r1, "gun_tower")
    put(bx + 3, by + 1, "tesla")
    put(c0 + 1, by, "bio_slush")
    put(*START, "start")
    put(*START2, "start2")
    put(*BONG, "bong")
    # the stores round the bong
    pickup(bx - 2, by - 3, "bong_health")
    pickup(bx + 2, by - 3, "ammo")
    pickup(bx - 2, by + 3, "health")
    pickup(bx + 1, by + 3, "tower_pack")
    pickup(c0 + 4, r0 + 2, "shield")
    pickup(c1 - 2, r0 + 2, "missiles")
    pickup(c0 + 2, r1 - 3, "health")
    # air support and the grenades in the corners
    pickup(c0 + 2, r0 + 2, "air_jet")
    pickup(c0 + 2, r1 - 1, "air_heli")
    pickup(c1 - 2, r1 - 2, "grenades")
    # lamps at the gates; sandbags at the baffle's open end, cover for the
    # starts - the ways up to the gates are left clear, so a queue at one
    # has room to spread
    for c, r in ((c1 + 1, by - 4), (c1 + 1, by + 4), (bx - 3, r0 - 1), (bx + 3, r1 + 1)):
        put_free(c, r, "lamp")
    for c, r in ((bx - 4, by - 5), (bx - 4, by + 4)):
        put_free(c, r, "sandbag")


def gloop_cay():
    """The rest of the players' island: the ways from the battery's gates
    to its crossings, the north point with the portal, the laser and a
    lookout lamp."""
    c0, r0, c1, r1 = FORT
    bx, by = BONG
    track([(c1 - 3, by - 1), (c1 + 1, by - 1)])  # the sea gate onto the main bridge
    trail([(bx, r0 - 1), (bx, 13), (15, 11)])  # the north gate to Palm Key's ford
    trail([(bx, 13), (7, 9)])  # to Gull Rock's ford
    trail([(bx, r1 + 1), (bx, 37), (7, 40)])  # the south gate to Wreck Reef's ford
    trail([(bx, r1 + 1), (20, r1 + 1), (20, 35)])  # along the wall to Fisher's ford
    trail([(c1 + 1, r0), (c1 + 1, r1)])  # outside the sea wall
    clear_disc(*PORTALS[0], 2.2)
    put(*PORTALS[0], "portal")
    pickup(11, 10, "laser")
    pickup(12, 12, "health")


def gull_rock():
    """Gull Rock, a ford off the north point: the old signal hut, the
    minigun and a health pack inside."""
    hut(2, 1, 6, 4, {(4, 4), (5, 4)}, "wood")
    pickup(3, 2, "minigun")
    pickup(5, 2, "health")
    for c, r in ((9, 2), (10, 3), (11, 2)):
        put_free(c, r, "palm")


def wreck_reef():
    """Wreck Reef, a ford off the south point: a smuggler's ship run
    aground, its hull of iron and timber split open, the gravity well in
    the hold and its fuel spilled about."""
    trail([(7, 44), (7, 46)])
    hull = [(3, 45), (4, 44), (5, 44), (10, 44), (11, 44), (11, 46), (10, 47), (4, 47), (3, 46)]
    for i, (c, r) in enumerate(hull):
        if (c, r) in land:
            wall(c, r, "iron" if i % 3 == 0 else "wood")
    pickup(9, 46, "gravity_well")
    pickup(5, 46, "health")
    drums([(12, 45), (12, 46)], 7, fuel=1.0)
    put_free(13, 44, "snag")


def palm_key():
    """Palm Key, the north chain's first island: a copra plantation of
    palms in rows, the drying shed with the flamethrower in it, and its
    drums of copra oil."""
    trail([(19, 11), (24, 10), (28, 9), (33, 10), (33, 12)])
    trail([(28, 9), (36, 7)])
    hut(30, 3, 34, 6, {(31, 6), (32, 6)}, "wood")
    pickup(32, 4, "flamethrower")
    pickup(24, 7, "health")
    pickup(21, 9, "ammo")
    for c in range(22, 30, 3):
        for r in (4, 7):
            if (c, r) not in reserved:
                put_free(c, r, "palm")
    drums([(25, 3), (26, 3), (26, 4)], 11, fuel=0.2)


def lighthouse_rock():
    """Lighthouse Rock: the lighthouse a round brick tower, its lamp by
    the door, the keeper's EMP and the lamp oil in drums that go up
    together."""
    trail([(41, 6), (50, 5)])
    for c, r in disc(46, 3, 1.6):
        wall(c, r, "brick")
    put_free(43, 5, "lamp")
    pickup(42, 3, "emp_burst")
    pickup(49, 7, "health")
    drums([(43, 1), (44, 1)], 17, fuel=0.7)


def turtle_isle():
    """Turtle Isle: the turtles' nesting beach on the north edge, where a
    wave lands; a ruined fort on the hill with the sonic hammer, and an
    enemy bio slush over the ford to the Corsair outpost."""
    trail([(55, 5), (61, 6), (64, 10), (64, 13)])
    trail([(61, 1), (61, 6)])
    trail([(64, 10), (67, 11)])
    for c, r in outline(55, 7, 59, 10):
        if r == 7 and c in (56, 57):
            continue
        if c == 59 and r in (8, 9):
            continue
        if (c, r) in land and hash01(c, r, 21) < 0.85:
            wall(c, r, "brick")
    pickup(57, 9, "sonic_hammer")
    pickup(66, 5, "health")
    pickup(58, 3, "speedup")
    put(60, 11, "bio_slush", side="enemy")
    cluster(67, 7, 3, 23)


def customs_isle():
    """Customs Isle: the crossroads of the islands - the main bridge in from
    the west, fords north and south to both chains, and the customs house
    on the lagoon shore, the road running through it and out onto the
    treasure pier. A gun tower of the players' over the bridge, the
    customs' stores in the house, the seized cargo's drums on the quay."""
    trail([(33, 19), (33, 31)])
    hut(35, 22, 38, 27, {(35, 24), (35, 25), (38, 24), (38, 25)}, "brick")
    track([(28, 24), (39, 24)])
    pickup(37, 23, "shield")
    pickup(36, 26, "ammo")
    pickup(29, 28, "health")
    put(31, 21, "gun_tower")
    for c, r in ((30, 21), (30, 22)):
        put_free(c, r, "sandbag")
    for c, r in ((30, 29), (31, 29)):
        put_free(c, r, "wood_crate")
    cluster(37, 30, 3, 29)


def fisher_isle():
    """Fisher's Isle: the fishing village - two huts on the shore, the
    grenades in the net store, the smokehouse's fish oil on the beach."""
    trail([(20, 38), (25, 39), (33, 39), (33, 37)])
    trail([(33, 39), (38, 41)])
    huts = [(24, 41, 28, 44), (30, 42, 34, 45)]
    for c0, r0, c1, r1 in huts:
        hut(c0, r0, c1, r1, {(c0 + 1, r0), (c0 + 2, r0)}, "wood")
    pickup(26, 42, "grenades")
    pickup(32, 43, "health")
    pickup(36, 40, "ammo")
    cluster(28, 45, 3, 31, fuel=0.4)


def coral_isle():
    """Coral Isle: a reef garden - heads of brain coral (glass, brittle)
    round the plasma, the beach where a wave lands to the south."""
    trail([(43, 41), (52, 41)])
    trail([(48, 41), (48, 46)])
    for c, r in ((45, 40), (46, 40), (50, 39), (51, 39), (45, 44), (51, 44)):
        if (c, r) not in reserved:
            put_free(c, r, "wall", material="glass")
    pickup(47, 43, "plasma")
    pickup(44, 42, "health")
    pickup(51, 42, "missiles")


def mangrove_isle():
    """Mangrove Isle: willows to the water, reeds in the shallows, the FPV
    swarm in a stilt hut among the roots, and an enemy tesla over the ford
    to the atoll."""
    trail([(57, 41), (63, 40), (64, 36)])
    trail([(63, 40), (69, 38)])
    hut(59, 41, 63, 44, {(60, 41), (61, 41)}, "wood")
    pickup(61, 43, "fpv_swarm")
    pickup(67, 41, "health")
    pickup(64, 37, "ammo")
    put(66, 42, "tesla", side="enemy")
    cluster(65, 44, 3, 37)


def corsair_outpost():
    """The Corsair outpost on the middle-east island, the enemy's forward
    post: a gun tower in a sandbag nest over the lagoon, their stores
    beside it, drums by the shore."""
    trail([(64, 18), (64, 31)])
    trail([(59, 24), (70, 24)])
    put(60, 22, "gun_tower", side="enemy")
    for c, r in ((59, 21), (60, 21), (61, 21), (59, 22), (61, 23)):
        put_free(c, r, "sandbag")
    pickup(62, 28, "plasma")
    pickup(68, 27, "health")
    pickup(67, 20, "missiles")
    cluster(60, 28, 3, 41)


def treasure_islet():
    """The treasure islet in the lagoon, deep water all round, at the end
    of the long plank pier from the customs house: a ruined shrine's four
    pillars, the portal in the middle and the hoard about it - laser,
    missiles, a shield and health."""
    clear_disc(*PORTALS[1], 2.2)
    put(*PORTALS[1], "portal")
    for c, r in ((47, 22), (51, 22), (47, 27), (51, 27)):
        put_free(c, r, "wall", material="brick")
    pickup(46, 22, "laser")
    pickup(52, 24, "missiles")
    pickup(49, 22, "shield")
    pickup(49, 27, "health")


def atoll():
    """Blip Atoll, the enemy's island: a walled fort - brick, iron corners
    - round the enemy bong, gates west (the bridge from the outpost), north
    and south, a gun tower beside each, two teslas and a bio slush in the
    corners inside, a baffle inside the west gate, the bong's yard open
    between them for its guards, their stash by the walls; the third portal
    on the south-west point, well out of the bong's hops, the garrison's
    fuel dump on the north-west point, the landing beaches east where the
    reinforcements come ashore."""
    ex, ey = ENEMY_BONG
    c0, r0, c1, r1 = 80, 17, 91, 31
    clear_rect(c0 - 2, r0 - 2, c1 + 1, r1 + 2)
    inside(rect(c0, r0, c1, r1))
    for c, r in outline(c0, r0, c1, r1):
        if c == c0 and 23 <= r <= 26:
            continue  # the west gate
        if r in (r0, r1) and ex - 1 <= c <= ex + 1:
            continue  # the north and south gates
        corner = c in (c0, c1) and r in (r0, r1)
        wall(c, r, "iron" if corner else "brick")
    put(*ENEMY_BONG, "enemy_bong")
    put(c0, 22, "gun_tower", side="enemy")
    put(c0, 27, "gun_tower", side="enemy")
    put(ex - 2, r0, "gun_tower", side="enemy")
    put(ex + 2, r1, "gun_tower", side="enemy")
    put(c0 + 2, r0 + 2, "tesla", side="enemy")
    put(c0 + 2, r1 - 2, "tesla", side="enemy")
    put(c1 - 2, r0 + 2, "bio_slush", side="enemy")
    for c, r in ((c1 - 1, ey - 1), (c1 - 1, ey), (c1 - 1, ey + 1)):
        put(c, r, "sandbag")
    # a baffle inside the west gate: the road from the outpost - and a shot
    # out of the treasure islet's portal, which stands on the same row -
    # has no straight line to the bong
    for r in range(ey - 2, ey + 3):
        wall(c0 + 3, r, "brick")
    clear_disc(*PORTALS[2], 1.6)
    put(*PORTALS[2], "portal")
    pickup(c1 - 2, r1 - 2, "health")
    pickup(c0 + 2, ey + 1, "ammo")
    pickup(c1 - 3, ey, "plasma")
    # the ways round the fort and onto the beaches
    trail([(75, 12), (80, 13), (88, 13), (95, 11)])
    trail([(77, 36), (82, 35), (89, 35), (95, 37)])
    trail([(76, 13), (76, 36)])
    trail([(ex, 13), (ex, 16)])
    trail([(ex, 32), (ex, 35)])
    trail([(94, 13), (94, 35)])
    track([(76, 24), (80, 24)])
    # the garrison's fuel dump on the north-west point, off every way the
    # guards and the waves drive - a drum on a tank's way is one it stops to
    # shoot, inside its own blast's reach
    cluster(80, 8, 3, 53, fuel=0.6)
    pickup(82, 38, "health")
    pickup(83, 11, "ammo")


# ---------------------------------------------------------------- planting


def distance_field(sources, limit, within=None):
    dist = {p: 0 for p in sources}
    q = deque(sorted(sources))
    while q:
        c, r = q.popleft()
        d = dist[(c, r)]
        if d >= limit:
            continue
        for dc, dr in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            n = (c + dc, r + dr)
            if inb(*n) and n not in dist and (within is None or n in within):
                dist[n] = d + 1
                q.append(n)
    return dist


JUNGLE = (("tree", 40), ("palm", 35), ("willow", 10), ("birch", 5), ("snag", 2))
SHORE = (("palm", 85), ("tree", 15))
FLOWERS = (("berry_bush", 30), ("bush", 25), ("fern", 20), ("juniper", 12), ("autumn_bush", 13))

# islands planted to their own taste
MANGROVE = set()


def tree_for(c, r, wd):
    if (c, r) in MANGROVE:
        return "willow" if hash01(c, r, 71) < 0.7 else "tree"
    if wd.get((c, r), 9) <= 2:
        return pick(SHORE, hash01(c, r, 73))
    return pick(JUNGLE, hash01(c, r, 79))


PLANT_GROUND = []
# how far round the enemy bong the planting grows no tree, in cells: past
# the guards' leash (`guard_leash_px`, 260 px) and a hop or two of the bong
GUARD_BEAT = 13
# the trees the planting set, which a cut-off pocket may take back
PLANTED = set()


def plant():
    """Everything the set pieces leave: palms along the shores, jungle on
    the high ground of every island, flowering bushes and long grass at its
    skirts, reeds where the water is shallow - but a beach a cell wide
    kept open round every shore, so a hull can always drive along it."""
    sea = {(c, r) for r in range(H) for c in range(W) if is_water((c, r))}
    wd = distance_field(sea, 6)
    solid = {p for p in cells if blocks(p)}
    near_built = distance_field(solid, 2)
    free = sorted(fbm(c / 5, r / 5, 11) for (c, r) in land if (c, r) not in cells and (c, r) not in reserved)
    PLANT_GROUND.append(len(free))
    q = lambda share: free[int(len(free) * share)]
    grove, skirt = q(0.36), q(0.18)
    for r in range(H):
        for c in range(W):
            p = (c, r)
            if p not in land or p in cells or p in reserved:
                continue
            u = hash01(c, r, 13)
            v = hash01(c, r, 19)
            d = fbm(c / 5, r / 5, 11)
            shore = wd.get(p, 9)
            # no tree where the enemy bong's guards keep their beat - it
            # hops, and they follow it, shooting out what stands in their way
            room = near_built.get(p, 9) >= 2 and math.hypot(c - ENEMY_BONG[0], r - ENEMY_BONG[1]) > GUARD_BEAT
            if shore == 1:
                if p in MANGROVE and u < 0.5:
                    put(c, r, "reeds")
                elif room and u < 0.1:
                    put(c, r, "palm")  # a palm leaning out over the water
                    PLANTED.add(p)
                elif v < 0.14:
                    put(c, r, "reeds")
                continue
            if (shore == 2 and room and u < 0.5) or (room and d > grove and u < 0.85):
                put(c, r, tree_for(c, r, wd))
                PLANTED.add(p)
            elif d > skirt and v < 0.45:
                put(c, r, pick(FLOWERS, hash01(c, r, 23)))
            elif d > skirt and v < 0.65:
                put(c, r, "tall_grass")
            elif v < 0.08:
                put(c, r, pick(FLOWERS, hash01(c, r, 29)))


def meadow():
    """Flower beds and long grass over the open ground the set pieces and
    the trails keep - soft cover, so no route changes - but never inside a
    building or a fort, on a road, a gate's lane or round a start."""
    starts = (START, START2)
    spots = [(c, r) for (c, r) in sorted(land) if (c, r) not in cells and (c, r) not in INTERIOR
             and (c, r) not in GATE_APRON and min(math.hypot(c - s[0], r - s[1]) for s in starts) > 3]
    noise = sorted(fbm(c / 4, r / 4, 31) for c, r in spots)
    q = lambda share: noise[int(len(noise) * share)]
    bed, grass = q(0.85), q(0.72)
    for c, r in spots:
        d = fbm(c / 4, r / 4, 31)
        u = hash01(c, r, 37)
        if d > bed and u < 0.6:
            put(c, r, pick(FLOWERS, hash01(c, r, 41)))
        elif d > grass and u < 0.45:
            put(c, r, "tall_grass")
        elif u < 0.03:
            put(c, r, pick(FLOWERS, hash01(c, r, 43)))


# ---------------------------------------------------------------- the nav grid
#
# The game routes on a grid of 32 px cells whose centres sit half a cell off
# the map's (`pathfind::Grid`, `maplint`): a tile at map cell (c, r) blocks
# nav cells c-1..c by r-1..r. So two tiles one cell apart leave no nav cell
# between them, and a way the AI can take needs two empty map cells side by
# side. Everything below reasons on that grid, the way the linter does.

LOOSE = {"barrel", "hay", "wood_crate", "barrier"}
SOFT = {"road", "water", "pickup", "oil", "portal", "start", "start2", "gate", "bong", "enemy_bong"} | COVER_KINDS | LOOSE
GATE_LANES = set()


def blocks(p):
    """Whether map cell `p` holds something the nav grid treats as a wall:
    any fixed tile, deep water."""
    if not inb(*p):
        return False
    if is_water(p):
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
    """The four nav cells round map cell `p` (each holds a quarter of it)."""
    c, r = p
    return [(c - 1, r - 1), (c, r - 1), (c - 1, r), (c, r)]


def portal_footprints():
    return [[n for n in nav_of(p) if nav_open(*n)] for p in PORTALS]


def nav_reach():
    """The nav cells a hull can reach from the start, the portals carrying
    it from any one of them to every other."""
    start = next(n for n in nav_of(START) if nav_open(*n))
    seen = {start}
    q = deque([start])
    feet = portal_footprints()
    hopped = False
    while q:
        k, l = q.popleft()
        nbrs = [(k + 1, l), (k - 1, l), (k, l + 1), (k, l - 1)]
        if not hopped and any((k, l) in f for f in feet):
            hopped = True
            nbrs += [n for f in feet for n in f]
        for n in nbrs:
            if n not in seen and nav_open(*n):
                seen.add(n)
                q.append(n)
    return seen


def reachable(p, reach):
    return any(n in reach for n in nav_of(p))


def carvable(p):
    k = kind_at(p)
    return p in land and p not in decks and (k is None or k in TREE_KINDS or k in COVER_KINDS)


def fill_pockets():
    """Make the planting fit the playfield, until nothing changes: a crate
    the trees walled off, or a nav cell they boxed in, has the trees round
    it give way (and that ground is kept open); then open ground the
    playfield never reaches becomes jungle, so nothing is left cut off."""
    sea = {(c, r) for r in range(H) for c in range(W) if is_water((c, r))}
    wd = distance_field(sea, 3)

    def give_way(area):
        changed = False
        for p in area:
            if p in PLANTED:
                cells.pop(p)
                PLANTED.discard(p)
                changed = True
            if p in land:
                OPEN_GROUND.add(p)
        return changed

    filled = 0
    for _ in range(16):
        reach = nav_reach()
        changed = False
        for c, r, _ in pickups:
            if not reachable((c, r), reach):
                changed |= give_way(list(rect(c - 2, r - 2, c + 2, r + 2)))
        for k, l in boxed_in():
            changed |= give_way(list(rect(k - 1, l - 1, k + 2, l + 2)))
        if changed:
            continue
        stray = [(k, l) for l in range(H) for k in range(W) if (k, l) not in reach and nav_open(k, l)]
        for k, l in stray:
            if not nav_open(k, l):
                continue
            corners = [(k + dc, l + dr) for dc in (0, 1) for dr in (0, 1)]
            corners.sort(key=lambda p: (sum(n in reach for n in nav_of(p)), p[1], p[0]))
            for p in corners:
                if p in land and (kind_at(p) is None or kind_at(p) in COVER_KINDS) and p not in OPEN_GROUND:
                    put(*p, tree_for(*p, wd))
                    PLANTED.add(p)
                    filled += 1
                    changed = True
                    break
        if not changed:
            break
    return filled


def close_narrow_trails():
    """An empty cell one wide among trees - something on both sides across
    it - is a trail a hull can take but the AI never will (the linter's
    narrow trail): it grows a fern, soft cover that changes no route."""
    taken = lambda p: p in cells or not inb(*p) or is_water(p)
    filled = 0
    for _ in range(3):
        grown = []
        for r in range(H):
            for c in range(W):
                p = (c, r)
                if p in cells or p not in land:
                    continue
                trees = sum(kind_at((c + a, r + b)) in TREE_KINDS for a in (-1, 0, 1) for b in (-1, 0, 1))
                across = (taken((c - 1, r)) and taken((c + 1, r))) or (taken((c, r - 1)) and taken((c, r + 1)))
                if trees >= 2 and across:
                    grown.append(p)
        if not grown:
            break
        for p in grown:
            put(*p, "fern")
        filled += len(grown)
    return filled


GATE_APRON = set()


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
            reserved.add(p)
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
    out = []
    for l in range(H):
        for k in range(W):
            if not nav_open(k, l):
                continue
            if not any(nav_open(*n) for n in ((k + 1, l), (k - 1, l), (k, l + 1), (k, l - 1))):
                out.append((k, l))
    return out


def shallow_nav(n):
    """Whether the router prices nav cell `n` as a ford: the game weighs
    the nav cell holding each shallow map cell's centre (`Grid::weigh`),
    and map cell (k, l)'s centre is nav cell (k, l)'s top-left corner."""
    k, l = n
    return inb(k, l) and is_water((k, l))


# ---------------------------------------------------------------- the walk


def walk_costs(origin=None):
    start = next(n for n in nav_of(origin or START) if nav_open(*n))
    cost = {start: 0}
    heap = [(0, start)]
    while heap:
        d, (k, l) = heapq.heappop(heap)
        if d > cost.get((k, l), 1 << 30):
            continue
        for n in ((k + 1, l), (k - 1, l), (k, l + 1), (k, l - 1)):
            if not nav_open(*n):
                continue
            nd = d + (3 if shallow_nav(n) else 1)
            if nd < cost.get(n, 1 << 30):
                cost[n] = nd
                heapq.heappush(heap, (nd, n))
    return cost


def wave_routes():
    """The map cells on each gate's cheapest walk to the players' bong - the
    way a wave's hunters come in. A loose drum or prop on one is a thing
    the router drives straight at and the hull then has to shove, so the
    islands keep them beside the ways in."""
    cost = walk_costs(BONG)
    cells_on = set()
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
    shape_islands()
    for c0, r0, c1, r1 in [(54, 32, 70, 46)]:
        MANGROVE.update(p for p in rect(c0, r0, c1, r1) if p in land)
    battery()
    gloop_cay()
    gull_rock()
    wreck_reef()
    palm_key()
    lighthouse_rock()
    turtle_isle()
    customs_isle()
    fisher_isle()
    coral_isle()
    mangrove_isle()
    corsair_outpost()
    treasure_islet()
    atoll()
    gate_aprons()
    for gc, gr in GATES:
        put(gc, gr, "gate")
        GATE_LANES.update(lane(gc, gr))
    plant()
    filled = fill_pockets()
    close_narrow_trails()
    meadow()
    return filled


HEADER = """\
# archipelago - Archipelago, level 9: island hopping between two fortified
# bong islands, the portal network kept.
# Written by tools/gen_archipelago.py - change the generator and run it
# again rather than editing this file; re-saving from the map editor would
# drop these comments. Level tables as dotted keys on purpose - a `[spawn]`
# header would swallow every `cells.` line after it.
#
# Hunt on the waves plan: four waves of 2 to 5 tanks, 14 in all, medium to
# super-heavy - fewer than the levels round it, but tougher, and the islands
# are the fight: every way across is a ford two cells wide that slows a hull
# to a wade or a plank bridge one cell wide with shallows either side, and
# the open sea between is deep - a wall to hulls, nothing to shells. The
# waves come ashore at four gates: Turtle Isle's nesting beach on the north
# edge, Coral Isle's beach on the south edge and the atoll's two landing
# beaches on the east edge. A clear sky, rain, or dusk when the lamps come
# on. 97 x 48 cells, followed by the camera, in the valley's look: clear
# water, gravel beaches, plank decks.
#
# The players hold Gloop Cay in the west: the old shore battery, a curtain
# wall of concrete with iron bastions round the whole middle of the island,
# its sea gate east onto the main bridge and its land gates north and
# south, a gun tower at each, and inside them one baffle wall bent round
# the north, east and south of the bong, open only to the west, so no way
# in has a straight shot at it; a tesla by the bong, a bio slush at the
# back, sandbags by the starts. Inside: ammo, health, the bong's health
# pack, a tower pack and a shield round the bong, and in the wall's corners
# missiles, grenades and both air strikes - every blast weapon nine cells
# or more off the bong. The north point outside the wall: the first portal,
# the laser and a health pack. Fords off the north and south points reach
# Gull Rock (the signal hut: a minigun) and Wreck Reef (a smuggler's ship
# run aground: the gravity well in its hold, its fuel spilled about).
#
# The main bridge runs east to Customs Isle - a gun tower of the players'
# over it, the seized cargo's drums on the quay - and from there fords
# reach both chains. The road runs on through the customs house (a shield
# and ammo inside) and out along the treasure pier. North: Palm Key (a
# copra plantation, the flamethrower in the drying shed, copra oil in
# drums), a plank bridge to Lighthouse Rock (the keeper's EMP, the lamp oil
# in drums), a ford to Turtle Isle (the sonic hammer in a ruined fort, an
# enemy bio slush over the ford south). South: Fisher's Isle, also a ford
# off the battery's south-east corner (grenades in the net store, fish oil
# on the beach), a plank bridge to Coral Isle (plasma among glass coral
# heads, missiles), a ford to Mangrove Isle (the FPV swarm in a stilt hut,
# an enemy tesla over the ford to the atoll).
#
# In the middle the lagoon, and in it the treasure islet, deep water all
# round: the long plank pier from the customs house and the second portal
# are the ways on - laser, missiles, a shield and health among a shrine's
# four pillars. Each portal throws a hull out at any of the others, so the
# jump is a coin toss. East of the lagoon the Corsair outpost: an enemy gun
# tower in its sandbags, plasma, missiles and health, fords north and south
# and a bridge east.
#
# Blip Atoll in the east is the enemy's: a brick fort with iron corners
# round the enemy bong, gates west (the bridge from the outpost), north and
# south, a gun tower at each gate, two teslas and a bio slush in the
# corners inside, a baffle inside the west gate so the long road in from
# the outpost has no straight shot at the bong, the bong's yard open for
# its guards, their stash by the walls; the third portal on the south-west
# point, the garrison's fuel dump on the north-west point. Palms on every
# shore, jungle, flowering bushes and long grass inland, reeds in the
# shallows, willows on Mangrove Isle.
"""


def write(path):
    lines = [HEADER, "version = 1",
             'bong = "gloop"',
             'enemy_bong = "blip"',
             'tank = "flak"',
             'tank2 = "glacier"',
             'biome = "valley"',
             'weather = ["clear", "rain", "dusk"]',
             f"size = [{W}, {H}]",
             'mission.kind = "hunt"',
             'spawn.kind = "waves"',
             "spawn.waves = 4",
             "spawn.size = 2",
             "spawn.growth = 1",
             'spawn.tier_start = "medium"',
             'spawn.tier_end = "super"',
             ""]
    out = dict(cells)
    for r in range(H):
        for c in range(W):
            if is_water((c, r)):
                out[(c, r)] = obj("water")
    for (c, r) in sorted(out, key=lambda p: (p[1], p[0])):
        lines.append(f'cells."{c},{r}" = {out[(c, r)]}')
    with open(path, "w") as f:
        f.write("\n".join(lines) + "\n")


def check():
    problems = list(MISPLACED)
    for gc, gr in GATES:
        if not (gc in (0, W - 1) or gr in (0, H - 1)):
            problems.append(f"gate {gc},{gr} is not on the edge")
    reach = nav_reach()
    near = lambda p, rr: any((p[0] + dc, p[1] + dr) in rr for dc in range(-1, 3) for dr in range(-1, 3))
    for p in (START, START2, BONG, ENEMY_BONG):
        if not near(p, reach):
            problems.append(f"{kind_at(p)} {p} cannot be reached")
    for c, r, kind in pickups:
        if not reachable((c, r), reach):
            problems.append(f"{kind} at {c},{r} cannot be reached")
    for p, f in zip(PORTALS, portal_footprints()):
        if not f:
            problems.append(f"portal {p} has no open nav cell")
    for gc, gr in GATES:
        if not all(nav_open(*n) for n in lane(gc, gr)):
            problems.append(f"gate {gc},{gr}: its lane is shut")
        elif lane(gc, gr)[-1] not in reach:
            problems.append(f"gate {gc},{gr} is cut off")
    for k, l in boxed_in():
        problems.append(f"nav cell {k},{l} is boxed in")
    for name, strait, _, _ in FORDS:
        deep = [p for p in rect(*strait) if is_deep(*p)]
        if deep:
            problems.append(f"the ford to {name} is deep at {deep[0]}")
        if not any(reachable(p, reach) for p in rect(*strait)):
            problems.append(f"the ford to {name} cannot be reached")
    for name, a, b in BRIDGES:
        if not reachable(a, reach) or not reachable(b, reach):
            problems.append(f"{name} cannot be reached")
    for c, r in DRUMS:
        if math.hypot(c - START[0], r - START[1]) < 7 or math.hypot(c - START2[0], r - START2[1]) < 7:
            problems.append(f"drum at {c},{r} is by a start")
    for c, r, kind in pickups:
        if kind in BLAST and math.hypot(c - BONG[0], r - BONG[1]) * 32 < BLAST_CLEAR:
            problems.append(f"{kind} at {c},{r} lies within {BLAST_CLEAR} px of the players' bong")
    for c, r in loose_on_routes():
        problems.append(f"{kind_at((c, r))} at {c},{r} stands on a wave's way in")
    for c, r in DRUMS:
        if (c, r) in WAYS:
            problems.append(f"drum at {c},{r} stands on a trail")
    return problems


ASCII = {
    'wall': '#', 'road': '=', 'start': '1', 'start2': '2', 'bong': 'B', 'enemy_bong': 'E',
    'pickup': 'P', 'sandbag': 's', 'fence': 'f', 'tree': 'T', 'pine': 'T', 'spruce': 'T', 'scots': 'T',
    'fir': 'T', 'palm': 'Y', 'snag': 'x', 'willow': 'W', 'birch': 'b', 'tall_grass': '"', 'bush': 'o',
    'berry_bush': 'o', 'autumn_bush': 'o', 'juniper': 'o', 'fern': ',', 'reeds': '|', 'gate': 'G',
    'tesla': 'Z', 'gun_tower': 'K', 'bio_slush': 'Q', 'barrel': 'D', 'hay': 'h', 'tyres': 'O',
    'wood_crate': 'X', 'barrier': 'n', 'tank_trap': '*', 'lamp': 'L', 'oil': ':', 'target': '@', 'portal': '@',
}


def ascii_map():
    print("    " + "".join(str(c // 10 % 10) for c in range(W)))
    print("    " + "".join(str(c % 10) for c in range(W)))
    for r in range(H):
        row = []
        for c in range(W):
            k = kind_at((c, r))
            if is_water((c, r)):
                ch = '%' if is_deep(c, r) else '~'
            elif k:
                ch = ASCII.get(k, '?')
                if k == 'wall':
                    ch = {'brick': '#', 'iron': 'I', 'wood': 'w', 'concrete': 'C', 'glass': 'g'}.get(
                        cells[(c, r)].split('"')[3], '#')
                if k == 'barrel':
                    ch = 'F' if 'fuel' in cells[(c, r)] else 'D'
                if k in ('gun_tower', 'tesla', 'bio_slush') and 'enemy' in cells[(c, r)]:
                    ch = ch.lower()
                if k == 'road' and (c, r) in decks:
                    ch = 'H'
            else:
                ch = '.'
            row.append(ch)
        print(f"{r:3d} " + "".join(row))


def narrow():
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
    print("nav: '.' reached, '~' a ford, '+' one wide, '!' open but cut off, ' ' shut")
    print("    " + "".join(str(c // 10 % 10) for c in range(W)))
    print("    " + "".join(str(c % 10) for c in range(W)))
    for l in range(H):
        row = []
        for k in range(W):
            n = (k, l)
            if n in thin:
                ch = "+"
            elif n in reach:
                ch = "~" if shallow_nav(n) else "."
            elif nav_open(k, l):
                ch = "!"
            else:
                ch = " "
            row.append(ch)
        print(f"{l:3d} " + "".join(row))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("-o", "--out", default="maps/archipelago.toml")
    ap.add_argument("--stats", action="store_true")
    ap.add_argument("--ascii", action="store_true")
    ap.add_argument("--nav", action="store_true")
    a = ap.parse_args()
    filled = build()
    if a.ascii:
        ascii_map()
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
        water = sum(is_water((c, r)) for r in range(H) for c in range(W))
        deep = sum(is_deep(c, r) for r in range(H) for c in range(W) if is_water((c, r)))
        print(f"{len(land)} land cells, {water} water ({deep} deep), {len(decks)} deck cells, "
              f"{PLANT_GROUND[0]} left to planting, {filled} pocket cells filled, {len(narrow())} one-wide nav cells")
        for k, n in sorted(kinds.items(), key=lambda kv: -kv[1]):
            print(f"  {k:12} {n}")
        cost = walk_costs()
        for gc, gr in GATES:
            c = cost.get(lane(gc, gr)[-1])
            print(f"  gate {gc},{gr}: walk {c} = {c * 32 / 160:.1f} s" if c else f"  gate {gc},{gr}: no walk")
        c = cost.get(nav_of(ENEMY_BONG)[0])
        print(f"  start to the enemy bong: walk {c} = {c * 32 / 160:.1f} s" if c else "  enemy bong: no walk")
        for c, r, kind in pickups:
            print(f"  pickup {kind:13} {c},{r}  {math.hypot(c - BONG[0], r - BONG[1]) * 32:.0f} px from the bong")
    if problems:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
