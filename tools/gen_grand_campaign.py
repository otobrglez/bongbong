"""Write maps/grand-campaign.toml: Grand Campaign, level 16 and the finale -
a river valley with every system in the game on it, the bong in a walled
fort on the west bank, 116 x 59 cells, laid out set piece by set piece.

  python3 tools/gen_grand_campaign.py [-o maps/grand-campaign.toml] [--stats] [--ascii]

This script is the source: change it and run it again rather than editing
the TOML (a re-save from the map editor would drop the header too). It draws
no random numbers: every choice is hashed from the cell's position with a
fixed salt, so a rerun writes the same file byte for byte. `--ascii` prints
the map a character a cell, `--stats` the counts, each gate's walk from the
start and every pickup. The checks it runs before writing: every pickup, the
starts, the bong and both portals reachable on foot; every gate on the edge,
its lane open and 45 cells or more from the bong (a leashed wave tank never
wanders into the bong's 800 px circle, BB-238); every crate an enemy would
fetch beyond every gate's leash; the rod from god in the start's sight box;
every blast weapon's crate eight cells and more from the bong; the curtain
and the redoubt whole but for their gates, and no gate of the fort with a
straight line to the bong; no drum, tesla or water by the bong.

What it lays out (the header written into the map says how it plays):

- the river: four to five cells wide with open water down its middle, from
  the north edge down the valley into the lake south of the fort, crossed
  at two bridges and two fords; the brook out of the lake to the south edge;
- the fort on the west bank, the players' base: a curtain of brick with
  concrete behind it and gun towers on its corners, gates on all four
  sides, and inside it the concrete redoubt round the bong, whose two ways
  in face away from every gate - the stores, the barracks, the mess and the
  magazine in the yards between;
- the players' outposts: the bridgehead across the main bridge, the north
  battery over the north bridge, the forest lodge with the west portal;
- the town on the east bank (the mill, the inn, terraces, the town hall,
  the church square and the market the enemy holds), the farm, the enemy's
  artillery park on the main road, the oil depot with the east portal and
  the enemy's gatehouse camp beyond it;
- the hamlet and the orchards on the hills to the north-west, the forest
  to the south-west with the woodcutters' clearing and a hidden glade.
"""

import argparse
import heapq
import math
from collections import deque

W, H = 116, 59

# (col, row) -> inline TOML table text
cells = {}
# cells the planting pass must leave alone
reserved = set()
water = set()
roads = set()
# the set pieces' open ground, which the planting keeps off
OPEN_GROUND = set()
pickups = []  # (c, r, kind)

BONG = (30, 30)
START = (34, 32)
START2 = (26, 32)
# the fort's curtain, its four gates, and the redoubt round the bong with
# its two ways in - the redoubt's walls stand square in front of every gate
FORT = (16, 17, 44, 43)
FORT_GATES = ({(44, r) for r in range(28, 34)} | {(c, 17) for c in range(21, 25)}
              | {(c, 43) for c in range(36, 40)} | {(16, r) for r in range(33, 36)})
REDOUBT = (24, 24, 36, 36)
REDOUBT_WAYS = {(c, 24) for c in (31, 32, 33)} | {(c, 36) for c in (27, 28, 29)}
# the gravel walk round the redoubt
WALK = (22, 22, 38, 38)

# The river's centre line, from the north edge into the lake. Each reach
# runs along a column, so the bridges meet it square on.
RIVER = [(53, -3), (53, 5), (51, 8), (51, 15), (53, 18), (56, 22), (57, 25), (57, 36), (55, 40), (52, 44)]
LAKE = (51.0, 48.0, 8.6, 4.6)
BROOK_COL = 61
# the roads' rows and columns (the first of the two)
HIGH = 11  # the old valley road and the town's High Street
MAIN = 30  # the main road, east from the fort's gate
SOUTH = 54  # the south road, under the lake to the depot
DEPOT_ROW = 48  # the depot road to the east edge
NORTH_ST = 70  # North Street, down the town and on to the depot
CHURCH_LN = 100
CAMP_LN = 106
DEPOT_S = 84
FORT_N_ROAD = 22  # down from the fort's north gate
FORT_S_ROAD = 37  # down from its south gate
WEST_TRACK = 9  # the forest track's column
LODGE_ROW = 50  # and its row

GATES = [
    (NORTH_ST, 0), (CHURCH_LN, 0),  # the town's two north streets
    (W - 1, HIGH), (W - 1, MAIN), (W - 1, DEPOT_ROW),  # High Street, the main road, the depot road
    (DEPOT_S, H - 1), (CAMP_LN, H - 1),  # the depot's south road, the camp lane
]
GATE_LANES = set()

# a weapon met first in the campaign here: its crate lies in the start's
# sight box (docs/levels.md, "The weapons along the way")
FIRST_HERE = {"rod_from_god"}
# a seat's sight box, half extents in cells (`sight_box_half_cols`/`_rows`)
SIGHT = (11.5, 7.5)
# what an enemy turns off its road to fetch lies beyond every gate's leash
# (`enemy_leash_px`, 640 px = 20 cells)
FETCHED = {
    "laser", "minigun", "plasma", "missiles", "sonic_hammer", "emp_burst", "gauss_rail", "fpv_swarm",
    "rod_from_god", "gravity_well", "speedup", "shield",
}
# Oto's rule for the players' base: every blast weapon's crate lies at
# least 256 px (eight cells) from the players' bong - two grenade blasts
# (docs/levels.md) - and the rod, whose circle kills the bong, as far
BLAST_CLEAR = {"grenades": 8.0, "fpv_swarm": 8.0, "air_jet": 8.0, "air_heli": 8.0, "missiles": 8.0,
               "gravity_well": 8.0, "rod_from_god": 8.0}

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


def material_at(p):
    t = cells.get(p)
    return t.split('"')[3] if t and kind_at(p) == "wall" else None


def rect(c0, r0, c1, r1):
    for r in range(r0, r1 + 1):
        for c in range(c0, c1 + 1):
            if inb(c, r):
                yield c, r


def outline(c0, r0, c1, r1):
    for c, r in rect(c0, r0, c1, r1):
        if c in (c0, c1) or r in (r0, r1):
            yield c, r


def clear_rect(c0, r0, c1, r1):
    """Bare ground: whatever stood there goes, but for water and roads."""
    for c, r in list(rect(c0, r0, c1, r1)):
        if (c, r) not in roads and (c, r) not in water:
            clear(c, r)
        reserved.add((c, r))
        OPEN_GROUND.add((c, r))


def keep_open(c0, r0, c1, r1):
    """Ground the planting leaves bare, whatever stands on it now."""
    for p in rect(c0, r0, c1, r1):
        reserved.add(p)
        OPEN_GROUND.add(p)


def pickup(c, r, kind):
    assert (c, r) not in water and (c, r) not in roads, f"{kind} at {c},{r} would cover water or road"
    assert kind_at((c, r)) is None, f"{kind} at {c},{r} would replace {kind_at((c, r))}"
    put(c, r, "pickup", pickup=kind)
    pickups.append((c, r, kind))


def hline(c0, c1, r):
    return [(c, r) for c in range(c0, c1 + 1)]


def vline(c, r0, r1):
    return [(c, r) for r in range(r0, r1 + 1)]


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
    """A dirt road `width` cells across along the rows and columns, on dry
    ground only: where it meets water it stops at the bank, and a bridge's
    deck (`deck_across`) carries it over."""
    path = polyline(points)
    for c, r in path:
        for dc in range(width):
            for dr in range(width):
                p = (c + dc, r + dr)
                if inb(*p) and p not in water:
                    put(*p, "road")
    for c, r in path:
        for dc in range(-margin, width + margin):
            for dr in range(-margin, width + margin):
                if inb(c + dc, r + dr):
                    reserved.add((c + dc, r + dr))
    return path


def deck_across(r, c0, c1):
    """A plank deck along row `r` over the water between columns c0 and c1:
    a road cell with water north and south of it, which the valley's look
    draws as a bridge (`ground::GroundGrid::bridge`). One row wide, so every
    cell of it has the river on both sides; the water either side of it is a
    ford, so the crossing is three cells wide to a hull."""
    for c in range(c0, c1 + 1):
        if (c, r) in water:
            put(c, r, "road")
    for c in range(c0, c1 + 1):
        for rr in (r - 1, r + 1):
            reserved.add((c, rr))


# ---------------------------------------------------------------- the water


def seg_dist(p, a, b):
    (px, py), (ax, ay), (bx, by) = p, a, b
    dx, dy = bx - ax, by - ay
    length = dx * dx + dy * dy
    t = 0.0 if length == 0 else max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / length))
    return math.hypot(px - (ax + t * dx), py - (ay + t * dy))


def river_dist(c, r):
    return min(seg_dist((c, r), a, b) for a, b in zip(RIVER, RIVER[1:]))


def in_lake(c, r):
    lc, lr, rx, ry = LAKE
    wobble = 0.22 * (fbm(c / 3.5, r / 3.5, 7) - 0.5)
    return ((c - lc) / rx) ** 2 + ((r - lr) / ry) ** 2 <= 1.0 + wobble


def is_deep(c, r):
    """Open water: a water cell whose eight neighbours are all water, the
    map's edge reading as the cell beside it (`ground::Layout` clamps)."""
    clamp = lambda c2, r2: (min(max(c2, 0), W - 1), min(max(r2, 0), H - 1))
    return all(clamp(c + dc, r + dr) in water for dc in (-1, 0, 1) for dr in (-1, 0, 1))


def waters():
    """The river, every cell within its half-width of the centre line (the
    half-width wandering between 2.2 and 2.6 so the banks are never ruled
    straight: four or five across, with deep water down the middle); the
    lake it runs into south-east of the fort; the brook out of the lake's
    east end down to the south edge, one cell wide and a ford all along."""
    for r in range(H):
        for c in range(W):
            hw = 2.15 + 0.5 * fbm(c / 5, r / 5, 3)
            if river_dist(c, r) <= hw or in_lake(c, r):
                put(c, r, "water")
    for c in range(56, BROOK_COL + 1):
        put(c, 49, "water")
    for r in range(49, H):
        put(BROOK_COL, r, "water")


def narrow(r0, r1, keep):
    """A ford: over rows r0..r1 the river keeps only the `keep` cells
    nearest its centre line, all of them shallow."""
    for r in range(r0, r1 + 1):
        row = sorted((c for c in range(W) if (c, r) in water and not in_lake(c, r)),
                     key=lambda c: (river_dist(c, r), c))
        for c in row[keep:]:
            clear(c, r)
            OPEN_GROUND.add((c, r))


# ---------------------------------------------------------------- the fort


def drum_cluster(at, drum):
    """A cluster of drums standing in the open: the ring of cells round it
    is cleared, so a hull that shoves one rolls it away rather than pinning
    it against a wall or the edge."""
    for c, r in at:
        for p in rect(c - 1, r - 1, c + 1, r + 1):
            if p not in water and p not in roads and kind_at(p) in (None,) + tuple(TREE_KINDS | COVER_KINDS):
                clear(*p)
    for c, r in at:
        put(c, r, "barrel", drum=drum)


def oil_trail(points):
    for c, r in polyline(points):
        if (c, r) not in cells:
            put(c, r, "oil")


def building(c0, r0, c1, r1, walls, roof, salt, windows=None, door=None):
    """A building: its walls in one material with glass windows along the
    `windows` face (a row), its roof in another. With `door` (cells of the
    wall left open) it is hollow inside."""
    hollow = door is not None
    for c, r in rect(c0, r0, c1, r1):
        edge = c in (c0, c1) or r in (r0, r1)
        if hollow and not edge:
            clear(c, r)
            OPEN_GROUND.add((c, r))
            continue
        if door and (c, r) in door:
            clear(c, r)
            OPEN_GROUND.add((c, r))
            continue
        if edge:
            window = windows is not None and r == windows and c0 < c < c1 and (c - c0) % 2 == 1
            wall(c, r, "glass" if window else walls)
        else:
            wall(c, r, roof)


def fort():
    """The players' base, walled twice. The curtain: brick outside and
    concrete behind it, two cells thick, a gun tower on every corner, a
    gate through it on every side. Inside it the concrete redoubt round
    the bong, whose ways in - the east end of its north wall, the west end
    of its south wall - face no gate, so nothing outside has a straight
    shot at the bong and a tank that gets in turns along the redoubt's
    walls to reach it. In the redoubt: the bong in a ring of sandbags, two
    teslas and a bio slush in its corners, the bong's packs, the players'
    starts. Between the walls the yards, a gravel walk round the redoubt:
    the mess against the north curtain, the barracks against the west,
    the magazine and the tents in the south; the stores along them, every
    blast weapon eight cells and more from the bong."""
    c0, r0, c1, r1 = FORT
    # the glacis: bare ground all round the curtain
    clear_rect(c0 - 4, r0 - 4, c1 + 4, r1 + 4)
    for ring, material in ((0, "brick"), (1, "concrete")):
        for c, r in outline(c0 + ring, r0 + ring, c1 - ring, r1 - ring):
            # the gate's cell in the outer ring, and the one behind it
            outer = ((c, r0) if r == r0 + ring else (c, r1) if r == r1 - ring
                     else (c0, r) if c == c0 + ring else (c1, r))
            if outer in FORT_GATES:
                continue
            post = ring == 0 and any(abs(c - gc) + abs(r - gr) == 1 for gc, gr in FORT_GATES)
            wall(c, r, "concrete" if post else material)
    for c, r in ((c0, r0), (c1, r0), (c0, r1), (c1, r1)):
        put(c, r, "gun_tower")
    # the redoubt
    k0, q0, k1, q1 = REDOUBT
    for c, r in outline(k0, q0, k1, q1):
        if (c, r) not in REDOUBT_WAYS:
            wall(c, r, "concrete")
    bc, br = BONG
    put(bc, br, "bong")
    for c, r in outline(bc - 2, br - 2, bc + 2, br + 2):
        if c == bc or r == br:
            continue
        put(c, r, "sandbag")
    # a tesla beside each way in, a bio slush in the other two corners
    put(k1 - 1, q0 + 1, "tesla")
    put(k0 + 1, q1 - 1, "tesla")
    put(k0 + 1, q0 + 1, "bio_slush")
    put(k1 - 1, q1 - 1, "bio_slush")
    # gun towers in the yards over the ways from the north and the south
    # gates to the redoubt's
    put(27, 21, "gun_tower")
    put(33, 39, "gun_tower")
    put(*START, "start")
    put(*START2, "start2")
    pickup(bc - 4, br - 3, "bong_health")
    pickup(bc + 4, br - 3, "bong_health")
    pickup(bc, br - 4, "health")
    pickup(bc, br + 4, "ammo")
    # the gravel walk round the redoubt, and in from every gate to it
    for p in outline(WALK[0], WALK[1], WALK[2], WALK[3]):
        if p not in cells:
            put(*p, "road")
    # the buildings against the curtain: the mess (north), the barracks
    # (west), the magazine (south-east, its door on the walk),
    # two tents in the south-west
    building(31, 19, 40, 20, "brick", "wood", 3, windows=20)
    building(18, 21, 19, 28, "wood", "brick", 5)
    building(39, 34, 42, 39, "brick", "concrete", 7, door={(39, 36), (39, 37)})
    for t0 in (18, 21):
        for p in rect(t0, 40, t0 + 1, 41):
            wall(*p, "wood")
    # the stores. North yard: grenades and missiles in its corners, the air
    # support either side of the walk in from the north gate
    pickup(18, 19, "grenades")
    pickup(19, 19, "health")
    pickup(42, 19, "missiles")
    pickup(41, 19, "ammo")
    pickup(26, 20, "air_jet")
    pickup(29, 20, "air_heli")
    pickup(25, 23, "ammo")
    # the magazine, a dead end off the walk no enemy's way runs through: the
    # rod from god (its first crate in the campaign, in the start's sight
    # box) and a health pack
    pickup(40, 36, "rod_from_god")
    pickup(41, 37, "health")
    # east yard: the plain weapons, a pack each side of the main gate
    pickup(40, 25, "laser")
    pickup(40, 23, "minigun")
    pickup(41, 26, "health")
    pickup(41, 33, "ammo")
    pickup(40, 40, "plasma")
    # south yard: the second grenades and missiles, a shield by the tents
    pickup(20, 37, "shield")
    pickup(42, 41, "grenades")
    pickup(18, 38, "missiles")
    pickup(25, 40, "health")
    pickup(32, 40, "ammo")
    # west yard
    pickup(21, 25, "tower_pack")
    pickup(21, 28, "plasma")
    pickup(20, 30, "health")
    pickup(20, 31, "ammo")
    # crates stacked against the walls
    for c, r in ((18, 29), (18, 30), (42, 22), (42, 23), (35, 41), (28, 41)):
        put(c, r, "wood_crate")
    # lamps on the redoubt's west corners, outside
    for c, r in ((k0 - 1, q0 - 1), (k0 - 1, q1 + 1)):
        put(c, r, "lamp")
    # outside: sandbag wings against the curtain either side of every gate
    # (flush, so the way round the walls keeps its width), tank traps and a
    # barrier out on the glacis
    for c, r in ((19, 16), (20, 16), (25, 16), (26, 16),
                 (34, 44), (35, 44), (40, 44), (41, 44),
                 (15, 31), (15, 32), (15, 36), (15, 37)):
        put(c, r, "sandbag")
    for c, r in ((50, 25), (50, 36), (12, 22), (28, 13), (31, 47), (12, 41)):
        put(c, r, "tank_trap")
    put(33, 13, "barrier")


# ---------------------------------------------------------------- the outposts


def bridgehead():
    """Across the main bridge: two sandbag works either side of the road,
    each an L open toward the river with a gun tower on its outer corner,
    ammo and health inside; a tesla by the road at their back, a lamp at
    each end of the bridge."""
    clear_rect(60, 23, 69, 40)
    for p in hline(61, 67, 23) + vline(68, 24, 27):
        put(*p, "sandbag")
    put(68, 23, "gun_tower")
    for p in hline(61, 67, 38) + vline(68, 34, 37):
        put(*p, "sandbag")
    put(68, 38, "gun_tower")
    put(65, 28, "tesla")
    pickup(62, 25, "ammo")
    pickup(64, 25, "health")
    pickup(66, 25, "minigun")
    pickup(62, 36, "health")
    pickup(64, 36, "ammo")
    for c, r in ((61, 29), (61, 33)):
        put(c, r, "lamp")
    for c, r in ((75, 27), (75, 34)):
        put(c, r, "tank_trap")


def north_battery():
    """On the hill over the north bridge: a sandbag redan facing the river,
    gun towers at its ends and a bio slush in its throat, the laser and
    plasma, health and ammo behind it."""
    clear_rect(38, 1, 48, 9)
    for p in hline(40, 46, 2) + vline(47, 3, 8) + hline(40, 46, 9):
        put(*p, "sandbag")
    put(47, 2, "gun_tower")
    put(47, 9, "gun_tower")
    put(44, 6, "bio_slush")
    pickup(40, 4, "laser")
    pickup(40, 7, "plasma")
    pickup(42, 4, "health")
    pickup(42, 7, "ammo")


def hamlet():
    """The hamlet on the hill road: three cottages, a barn open to the road
    with a stash in it, an orchard behind them, a well."""
    clear_rect(3, 2, 21, 10)
    building(4, 3, 8, 6, "brick", "wood", 11, windows=6)
    building(10, 3, 13, 6, "wood", "brick", 13, windows=6)
    building(16, 7, 20, 9, "brick", "glass", 17, windows=9)
    building(3, 7, 9, 10, "wood", "wood", 19, door={(6, 10), (7, 10)})
    pickup(4, 8, "minigun")
    pickup(5, 8, "missiles")
    pickup(8, 8, "speedup")
    pickup(15, 4, "health")
    pickup(15, 5, "ammo")
    # the well
    put(12, 8, "water")
    # the orchard on the slope behind
    for r in (2, 5, 8):
        for c in range(23 + 2 * ((r // 3) % 2), 37, 4):
            put(c, r, "tree" if hash01(c, r, 23) < 0.6 else ("birch" if hash01(c, r, 29) < 0.6 else "autumn_bush"))
    keep_open(22, 1, 37, 9)
    for c, r in ((16, 2), (17, 2), (18, 2)):
        put(c, r, "hay")


def forest_lodge():
    """Deep in the south-west forest: the timber lodge (the FPV swarm and
    the gravity well inside), the west portal before it - the forest's end
    of the fast way to the depot - and a friendly gun tower over both."""
    clear_rect(2, 51, 20, H - 2)
    building(3, 53, 10, 57, "wood", "wood", 21, door={(6, 53), (7, 53)})
    pickup(4, 55, "fpv_swarm")
    pickup(9, 55, "gravity_well")
    pickup(5, 56, "health")
    pickup(8, 56, "ammo")
    put(16, 55, "portal")
    put(13, 52, "gun_tower")
    put(18, 52, "lamp")
    for c, r in ((11, 57), (12, 57)):
        put(c, r, "wood_crate")


def woodcutters():
    """The woodcutters' clearing south of the forest track: log piles,
    plasma and the sonic hammer among them, an oil drum by the saw."""
    clear_rect(20, 52, 32, H - 2)
    for c0, r in ((21, 54), (26, 54), (21, 57), (28, 57)):
        for c in range(c0, c0 + 3):
            wall(c, r, "wood")
    pickup(24, 56, "plasma")
    pickup(30, 55, "sonic_hammer")
    pickup(25, 52, "health")
    drum_cluster(((31, 57),), "oil")


def hermit_glade():
    """A glade hidden in the west forest: the second rod from god and a
    health pack, at the end of a trail off the forest track."""
    keep_open(2, 40, 6, 44)
    clear_rect(2, 40, 6, 44)
    pickup(3, 42, "rod_from_god")
    pickup(5, 42, "health")
    for p in rect(6, 41, 8, 42):
        clear(*p)
        OPEN_GROUND.add(p)


def eastwood_glade():
    """A glade in the east wood between the main road and High Street: a
    jet strike and a health pack under the trees."""
    clear_rect(107, 24, 111, 27)
    pickup(108, 25, "air_jet")
    pickup(110, 26, "health")


def lake_shore():
    """The boathouse on the lake's west shore: plasma and health on its
    jetty, a boat drawn up."""
    clear_rect(38, 45, 43, 52)
    building(39, 46, 41, 48, "wood", "wood", 29)
    pickup(42, 46, "health") if (42, 46) not in water else None
    pickup(39, 50, "plasma")
    pickup(40, 51, "ammo")
    pickup(42, 52, "minigun")


# ---------------------------------------------------------------- the town


HOUSE_STYLES = (("brick", "wood", 55), ("wood", "brick", 30), ("brick", "glass", 15))
GARDEN = (("berry_bush", 35), ("autumn_bush", 35), ("bush", 20), ("juniper", 10))
GARDEN_TREES = (("birch", 30), ("tree", 40), ("palm", 10), ("willow", 20))


def house(c0, r0, c1, r1, salt, front):
    """A house, solid: walls in one material with glass windows along the
    `front` row, its roof in another, a concrete chimney now and then."""
    u = hash01(c0, r0, salt)
    acc = 0
    for walls, roof, w in HOUSE_STYLES:
        acc += w / 100
        if u < acc:
            break
    chimney = (c0 + 1 + int(hash01(c0, salt, 9) * max(1, c1 - c0 - 1)), r0 + 1)
    for c, r in rect(c0, r0, c1, r1):
        edge = c in (c0, c1) or r in (r0, r1)
        if edge:
            window = r == front and c0 < c < c1 and (c - c0) % 2 == 1
            wall(c, r, "glass" if window else walls)
        elif (c, r) == chimney and roof != "glass" and hash01(c, r, salt + 1) < 0.5:
            wall(c, r, "concrete")
        else:
            wall(c, r, roof)


def terrace(c0, r0, c1, r1, front, salt):
    """A row of houses along a street front, from column c0 to c1: widths
    of four to six, now and then a yard three wide between two of them (a
    way through to the gardens behind)."""
    c = c0
    i = 0
    while c <= c1:
        w = 4 + int(hash01(c, salt, 3) * 3)
        if c + w - 1 > c1 or c1 - (c + w - 1) < 4:
            w = c1 - c + 1
        house(c, r0, c + w - 1, r1, salt + i, front)
        c += w
        i += 1
        if c <= c1 - 7 and hash01(c, salt, 7) < 0.5:
            keep_open(c, r0, c + 2, r1)
            c += 3


def watermill():
    """The watermill on the east bank above the north bridge: a timber mill
    house with its wheel in the race, the miller's yard (grenades, health,
    a drum of lamp oil)."""
    clear_rect(56, 1, 66, 9)
    building(58, 1, 62, 4, "brick", "wood", 31, windows=4)
    for r in (2, 3):
        wall(57, r, "wood")  # the wheel, in the race
    pickup(59, 7, "grenades")
    pickup(61, 7, "health")
    for c, r in ((64, 2), (64, 3), (58, 9)):
        put(c, r, "wood_crate")
    drum_cluster(((65, 6),), "oil")


def inn():
    """The riverside inn south of High Street: its yard open to the street,
    health and ammo in it."""
    clear_rect(59, 14, 68, 20)
    building(61, 15, 67, 18, "brick", "wood", 37, windows=15)
    pickup(60, 19, "health")
    pickup(63, 19, "ammo")
    put(60, 14, "lamp")


def town_hall():
    """The town hall on High Street: brick round a courtyard with glass in
    its front, its doors to the street; inside a jet strike, health and
    ammo."""
    c0, r0, c1, r1 = 78, 3, 86, 9
    for c, r in outline(c0, r0, c1, r1):
        if r == r1 and c in (81, 82, 83):
            continue
        win = r == r1 and (c - c0) % 2 == 1
        wall(c, r, "glass" if win else ("concrete" if c in (c0, c1) and r in (r0, r1) else "brick"))
    for c, r in rect(c0 + 1, r0 + 1, c1 - 1, r1 - 1):
        clear(c, r)
        OPEN_GROUND.add((c, r))
    pickup(80, 5, "air_jet")
    pickup(84, 5, "health")
    pickup(82, 4, "ammo")
    keep_open(81, 10, 83, 10)


def church_square():
    """The church on the north edge by Church Lane and its square, held by
    an enemy gun tower: a fountain in the middle, lamps at the corners,
    grenades and health by the church door, a drum of lamp oil by the
    lane."""
    clear_rect(88, 0, 99, 9)
    for c, r in rect(91, 0, 97, 4):
        win = (r == 4 and c in (93, 95)) or (c in (91, 97) and r == 2)
        wall(c, r, "glass" if win else "brick")
    wall(94, 4, "concrete")
    for p in rect(93, 7, 94, 8):
        put(*p, "water")
    put(89, 7, "gun_tower", side="enemy")
    pickup(92, 6, "grenades")
    pickup(96, 6, "health")
    drum_cluster(((98, 7),), "oil")
    for c, r in ((88, 2), (99, 2), (88, 9)):
        put(c, r, "lamp")


def market():
    """The market square, held by the enemy - a forward camp: an enemy
    tesla among the stalls and an enemy gun tower on the corner, sandbag
    barricades across its mouth, a heap of oil drums by the fish stall;
    plasma, the minigun and speed on the stalls, health and ammo."""
    c0, r0, c1, r1 = 80, 14, 95, 20
    clear_rect(c0, r0, c1, r1)
    put(87, 17, "tesla", side="enemy")
    put(94, 15, "gun_tower", side="enemy")
    for c in (81, 82, 84, 85):
        put(c, 19, "wood_crate" if c in (81, 85) else "hay")
    for c in (89, 90, 92):
        wall(c, 15, "wood")
    pickup(83, 16, "plasma")
    pickup(85, 15, "minigun")
    pickup(90, 18, "speedup")
    pickup(89, 19, "health")
    pickup(86, 19, "ammo")
    for c, r in ((80, 14), (81, 14), (95, 19), (95, 20)):
        put(c, r, "sandbag")
    drum_cluster(((93, 18), (93, 19)), "oil")
    put(88, 13, "lamp")


def hedge(line, salt):
    """A garden hedge: a line of flowering scrub with a gap now and then -
    soft cover, no wall to a hull."""
    for c, r in line:
        if (c, r) in cells or (c, r) in roads or (c, r) in water:
            continue
        if hash01(c, r, salt) < 0.85:
            put(c, r, pick(GARDEN, hash01(c, r, salt + 1)))


def town():
    """The town on the east bank: terraces along High Street both sides,
    gardens behind them with their hedges and trees, the back lane under
    the south terraces."""
    watermill()
    inn()
    town_hall()
    church_square()
    market()
    # north of High Street: terraces fronting the street (rows 6-9),
    # gardens behind
    for c0, c1, salt in ((64, 68, 101), (72, 77, 103), (102, 107, 107), (109, 114, 109)):
        terrace(c0, 6, c1, 9, 9, salt)
        hedge(hline(c0, c1, 4), salt + 50)
    # south of High Street: terraces fronting it (rows 14-17), gardens
    # behind down to the back lane
    for c0, c1, salt in ((72, 78, 301), (97, 104, 307), (106, 113, 311)):
        terrace(c0, 14, c1, 17, 14, salt)
        hedge(hline(c0, c1, 19), salt + 50)


def lamps():
    """Lamp posts along High Street against the houses, at the bridge
    ends."""
    placed = 0
    for c in range(58, W - 3, 6):
        for r, back in ((HIGH - 1, -1), (HIGH + 2, 1)):
            behind = (c, r + back)
            if kind_at(behind) == "wall" and (c, r) not in cells:
                put(c, r, "lamp")
                placed += 1
    for c, r in ((47, HIGH - 1), (56, HIGH + 2)):
        if (c, r) not in cells and (c, r) not in water:
            put(c, r, "lamp")
            placed += 1
    return placed


# ---------------------------------------------------------------- the valley


def farm():
    """The farm between the bridgehead and the artillery park: a brick
    farmhouse, a timber barn open to the yard, hay, health and ammo, a
    fuel drum by the yard gate."""
    clear_rect(74, 23, 85, 28)
    building(75, 24, 78, 27, "brick", "wood", 41, windows=27)
    building(80, 23, 84, 27, "wood", "wood", 43, door={(81, 27), (82, 27), (83, 27)})
    pickup(81, 25, "health")
    pickup(83, 25, "ammo")
    pickup(82, 24, "minigun")
    for c, r in ((79, 24), (79, 25), (85, 26)):
        put(c, r, "hay")
    drum_cluster(((85, 24),), "fuel")


def artillery_park():
    """The enemy's forward camp astride the main road: two gun pits of
    sandbags facing west, an enemy gun tower in each, an enemy bio slush
    at the road; their tents and the ammunition dump of fuel drums behind
    them; the stash in their lee - the gauss rail, the EMP burst, missiles,
    grenades, a helicopter strike, health."""
    clear_rect(86, 23, 100, 40)
    # the north pit
    for p in vline(88, 23, 27) + hline(89, 92, 23) + hline(89, 92, 27):
        put(*p, "sandbag")
    put(90, 25, "gun_tower", side="enemy")
    # the south pit
    for p in vline(88, 34, 38) + hline(89, 92, 34) + hline(89, 92, 38):
        put(*p, "sandbag")
    put(90, 36, "gun_tower", side="enemy")
    put(95, 28, "bio_slush", side="enemy")
    # the tents
    for t0, q0 in ((96, 33), (96, 24)):
        for p in rect(t0, q0, t0 + 2, q0 + 1):
            wall(*p, "wood")
    pickup(94, 24, "gauss_rail")
    pickup(94, 26, "missiles")
    pickup(94, 37, "emp_burst")
    pickup(96, 37, "grenades")
    pickup(99, 27, "air_heli")
    pickup(98, 36, "health")
    drum_cluster(((98, 39), (99, 39), (99, 38)), "fuel")
    drum_cluster(((92, 40), (93, 40)), "oil")
    for c, r in ((86, 28), (86, 33)):
        put(c, r, "tank_trap")
    for c, r in ((92, 28), (92, 33)):
        put(c, r, "barrier")
    for c, r in ((87, 39), (87, 40)):
        put(c, r, "tyres")
    put(94, 33, "lamp")


def fields():
    """The fields south of the main road: plots of standing grain (tall
    grass) between hedgerows with gaps in them, hay bales in the stubble,
    a lamp at the crossroads; health by the gate of the middle field."""
    keep_open(72, 32, 87, 40)
    for r in (33, 36, 39):
        for c in range(73, 87):
            if (c, r) in cells or (c - 73) % 7 in (5, 6):
                continue
            put(c, r, "bush" if hash01(c, r, 47) < 0.5 else ("berry_bush" if hash01(c, r, 49) < 0.6 else "autumn_bush"))
    for c0, r0, c1, r1 in ((73, 34, 77, 35), (80, 37, 85, 38), (80, 34, 84, 35)):
        for p in rect(c0, r0, c1, r1):
            if p not in cells:
                put(*p, "tall_grass")
    for c, r in ((74, 37), (75, 38), (86, 35)):
        put(c, r, "hay")
    pickup(78, 35, "health")
    put(69, 29, "lamp")


# ---------------------------------------------------------------- the depot


def depot():
    """The oil depot: a fenced yard with gates north, west, south and east;
    four iron storage tanks in a square, fuel and oil drums in the aisles
    between them wired together by oil trails - a fuse that runs the whole
    yard; the pump house (the flamethrower, health, ammo); the loading bay
    (a jet strike among the crates) under an enemy gun tower; the east
    portal by the west gate; missiles and health by the office at the
    north gate."""
    c0, r0, c1, r1 = 66, 41, 98, 56
    clear_rect(c0, r0, c1, r1)
    gates = ({(c, r0) for c in range(NORTH_ST - 1, NORTH_ST + 3)} | {(c0, r) for r in range(SOUTH - 1, SOUTH + 2)}
             | {(c, r1) for c in range(DEPOT_S - 1, DEPOT_S + 3)} | {(c1, r) for r in range(DEPOT_ROW - 1, DEPOT_ROW + 3)})
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) in gates:
            continue
        put(c, r, "fence")
    # the storage tanks: iron, solid
    for t0, q0 in ((75, 43), (83, 43), (75, 50), (83, 50)):
        for p in rect(t0, q0, t0 + 3, q0 + 3):
            wall(*p, "iron")
    # the drums in the aisles, and the trails between them
    drum_cluster(((80, 44), (80, 45)), "fuel")
    drum_cluster(((80, 51), (80, 52)), "fuel")
    drum_cluster(((76, 48), (77, 48)), "fuel")
    drum_cluster(((85, 48), (86, 48)), "fuel")
    drum_cluster(((89, 44),), "oil")
    drum_cluster(((89, 53), (89, 54)), "oil")
    drum_cluster(((72, 44), (72, 45)), "oil")
    oil_trail([(80, 46), (80, 50)])
    oil_trail([(78, 48), (84, 48)])
    oil_trail([(87, 48), (88, 48), (88, 45)])
    oil_trail([(73, 45), (74, 45), (74, 47), (75, 47)])
    oil_trail([(81, 52), (82, 52), (82, 54), (88, 54), (88, 53)])
    # the office by the north gate
    building(67, 42, 69, 44, "brick", "concrete", 51)
    pickup(67, 46, "missiles")
    pickup(68, 47, "health")
    # the pump house
    building(91, 51, 96, 55, "brick", "concrete", 53, door={(91, 53), (91, 54)})
    pickup(93, 53, "flamethrower")
    pickup(95, 54, "health")
    pickup(95, 52, "ammo")
    # the loading bay, held by an enemy gun tower
    for c, r in ((92, 43), (93, 43), (95, 43), (96, 46), (96, 47), (93, 46)):
        put(c, r, "wood_crate" if hash01(c, r, 61) < 0.6 else "tyres")
    pickup(94, 45, "air_jet")
    put(94, 49, "gun_tower", side="enemy")
    # the portal by the west gate
    put(71, 52, "portal")
    for c, r in ((68, 50), (86, 50), (97, 42)):
        put(c, r, "lamp")


def gatehouse_camp():
    """Beyond the depot the enemy's gatehouse camp where the camp lane
    meets the depot road: sandbag revetments, an enemy gun tower and an
    enemy bio slush, its tents, tank traps on the lane; grenades and a
    helicopter strike in it, health and ammo by the tents."""
    clear_rect(100, 40, 113, 56)
    for p in hline(101, 104, 44) + vline(101, 45, 46) + hline(109, 112, 53) + vline(112, 51, 52):
        put(*p, "sandbag")
    put(103, 46, "gun_tower", side="enemy")
    put(110, 51, "bio_slush", side="enemy")
    for t0, q0 in ((109, 43), (101, 53)):
        for p in rect(t0, q0, t0 + 2, q0 + 1):
            wall(*p, "wood")
    pickup(103, 51, "grenades")
    pickup(104, 55, "air_heli")
    pickup(112, 46, "health")
    pickup(110, 46, "ammo")
    drum_cluster(((110, 40), (111, 40)), "fuel")
    for c, r in ((104, 41), (110, 56)):
        put(c, r, "tank_trap")
    for c, r in ((101, 50), (113, 49)):
        put(c, r, "barrier")


# ---------------------------------------------------------------- planting


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


FOREST = (("pine", 34), ("spruce", 22), ("fir", 14), ("scots", 18), ("snag", 4), ("birch", 8))
HILL_TREES = (("birch", 30), ("tree", 40), ("scots", 15), ("pine", 15))
VALLEY_TREES = (("tree", 55), ("birch", 20), ("willow", 10), ("scots", 15))
EAST_WOOD = (("tree", 45), ("birch", 20), ("scots", 20), ("spruce", 10), ("snag", 5))
BANK = (("willow", 60), ("birch", 40))
UNDER = (("fern", 30), ("juniper", 20), ("bush", 20), ("berry_bush", 15), ("autumn_bush", 15))
MEADOW = (("autumn_bush", 30), ("berry_bush", 30), ("bush", 25), ("juniper", 15))


def zone(c, r):
    if r >= 44 and c < 36:
        return "forest"
    if c < 14 and r >= 12:
        return "forest"
    if c < 48 and r < 16:
        return "hills"
    if c >= 58 and r <= 21:
        return "town"
    if c >= 64 and r >= 40:
        return "depot"
    if c >= 100 and r >= 22:
        return "eastwood"
    return "valley"


def glacis(c, r):
    """The open ground east of the fort, down to the river."""
    return 45 <= c <= 56 and 13 <= r <= 43


def tree_for(c, r, wd):
    if wd.get((c, r), 9) <= 2:
        return pick(BANK, hash01(c, r, 73))
    z = zone(c, r)
    table = {"forest": FOREST, "hills": HILL_TREES, "town": GARDEN_TREES, "eastwood": EAST_WOOD}.get(z, VALLEY_TREES)
    return pick(table, hash01(c, r, 79))


def planting():
    """Everything not a set piece: the forest thick with pine, spruce, fir
    and scots pine over ferns and juniper, glades of tall grass in it; the
    hills a meadow of tall grass and flowering scrub with clumps of birch
    and broadleaf; gardens in the town; the valley open, with trees in
    ones and twos, scrub and tall grass; reeds and willows by the water;
    the depot bare but for scrub."""
    wd = distance_field(water, 4)
    for r in range(H):
        for c in range(W):
            p = (c, r)
            if p in cells or p in reserved:
                continue
            z = zone(c, r)
            dw = wd.get(p, 9)
            u = hash01(c, r, 13)
            v = hash01(c, r, 19)
            if dw == 1 and hash01(c, r, 85) < 0.4:
                put(c, r, "reeds")
            elif dw == 2 and u < 0.18 and not glacis(c, r):
                put(c, r, pick(BANK, hash01(c, r, 73)))
            elif glacis(c, r):
                # the open ground between the fort and the river grows no
                # trees: grass and scrub a hull drives through
                if fbm(c / 4, r / 4, 71) < 0.45 and v < 0.6:
                    put(c, r, "tall_grass")
                elif v > 0.9:
                    put(c, r, pick(MEADOW, hash01(c, r, 73)))
                else:
                    continue
            elif z in ("forest", "eastwood"):
                d = fbm(c / 6, r / 6, 11)
                density = 0.8 if d > 0.52 else (0.45 if d > 0.4 else 0.08)
                if u < density:
                    put(c, r, tree_for(c, r, wd))
                elif d <= 0.4 and v < 0.55:
                    put(c, r, "tall_grass")
                elif v < 0.4:
                    put(c, r, pick(UNDER, hash01(c, r, 23)))
                else:
                    continue
            elif z == "hills":
                d = fbm(c / 5, r / 5, 31)
                if u < (0.5 if d > 0.62 else 0.04):
                    put(c, r, tree_for(c, r, wd))
                elif d < 0.48 and v < 0.7:
                    put(c, r, "tall_grass")
                elif fbm(c / 3, r / 3, 37) > 0.58 and v < 0.6:
                    put(c, r, pick(MEADOW, hash01(c, r, 43)))
                elif v > 0.92:
                    put(c, r, pick(MEADOW, hash01(c, r, 47)))
                else:
                    continue
            elif z == "town":
                g = fbm(c / 3, r / 3, 17)
                if u < 0.13:
                    put(c, r, pick(GARDEN_TREES, hash01(c, r, 79)))
                elif g > 0.5 and v < 0.6:
                    put(c, r, pick(GARDEN, hash01(c, r, 23)))
                elif g < 0.35 and v < 0.25:
                    put(c, r, "tall_grass")
                else:
                    continue
            elif z == "depot":
                if v < 0.06:
                    put(c, r, "juniper")
                elif v < 0.1:
                    put(c, r, "tall_grass")
                else:
                    continue
            else:  # the valley
                d = fbm(c / 6, r / 6, 53)
                if u < (0.32 if d > 0.62 else 0.04):
                    put(c, r, tree_for(c, r, wd))
                elif d < 0.45 and v < 0.65:
                    put(c, r, "tall_grass")
                elif fbm(c / 3, r / 3, 59) > 0.6 and v < 0.55:
                    put(c, r, pick(MEADOW, hash01(c, r, 61)))
                elif v > 0.92:
                    put(c, r, pick(MEADOW, hash01(c, r, 67)))
                else:
                    continue
            reserved.discard(p)


def edges():
    """The field's edges stand in trees two cells deep where nothing else
    is - the town's gardens, the depot's scrub - so there is no lane along
    the edge for a tank to hug."""
    wd = distance_field(water, 4)
    for c, r in rect(0, 0, W - 1, H - 1):
        if min(c, r, W - 1 - c, H - 1 - r) > 1 or (c, r) in cells or (c, r) in OPEN_GROUND or (c, r) in water:
            continue
        if any(abs(c - gc) <= 3 and abs(r - gr) <= 7 for gc, gr in GATES):
            continue
        z = zone(c, r)
        if z == "town":
            if hash01(c, r, 67) < 0.6:
                put(c, r, pick(GARDEN_TREES, hash01(c, r, 71)))
        elif z == "depot":
            if hash01(c, r, 67) < 0.5:
                put(c, r, "scots" if hash01(c, r, 71) < 0.5 else "snag")
        else:
            put(c, r, tree_for(c, r, wd))


def verges():
    """The bare ground along the roads grows a little cover."""
    for c, r in sorted(roads):
        for dc, dr in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            p = (c + dc, r + dr)
            if not inb(*p) or p in cells or p in roads or p in water or p in OPEN_GROUND:
                continue
            if any(abs(p[0] - gc) <= 2 and abs(p[1] - gr) <= 7 for gc, gr in GATES):
                continue
            u = hash01(*p, 29)
            if u < 0.08:
                put(*p, "tall_grass")
            elif u < 0.2:
                put(*p, pick(MEADOW, hash01(*p, 31)))


# ---------------------------------------------------------------- the nav grid
#
# The game routes on a grid of 32 px cells whose centres sit half a cell off
# the map's (`pathfind::Grid`, `maplint`): nav cell (k, l) spans map cells
# k..k+1 by l..l+1 and is open when none of them blocks. So a way the AI can
# take needs two empty map cells side by side.

# a loose prop is no wall in the nav grid (`nav_tile_shapes_where`): a hull
# shoves it out of its way
LOOSE = {"barrel", "hay", "wood_crate", "barrier"}
SOFT = {"road", "water", "pickup", "oil", "portal", "start", "start2", "gate"} | COVER_KINDS | LOOSE


def blocks(p):
    """Whether map cell `p` holds something the nav grid treats as a wall:
    any fixed tile, deep water, the bong."""
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


def nav_reach(start=START):
    first = next(n for n in nav_of(start) if nav_open(*n))
    seen = {first}
    q = deque([first])
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


def connect(c, r, reach):
    """Cut a trail three cells wide from map cell (c, r) out to the
    playfield through trees and cover."""
    prev = {(c, r): None}
    q = deque([(c, r)])
    goal = None
    while q:
        p = q.popleft()
        if p != (c, r) and reachable(p, reach) and not blocks(p) and math.hypot(p[0] - c, p[1] - r) > 2:
            goal = p
            break
        for dc, dr in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            n = (p[0] + dc, p[1] + dr)
            if inb(*n) and n not in prev and (carvable(n) or not blocks(n)):
                prev[n] = p
                q.append(n)
    if goal is None:
        return False
    p = goal
    while p is not None:
        pc, pr = p
        for dc in (-1, 0, 1):
            for dr in (-1, 0, 1):
                n = (pc + dc, pr + dr)
                if inb(*n) and carvable(n) and kind_at(n) in TREE_KINDS:
                    clear(*n)
        p = prev[p]
    return True


def wire_up():
    """Every pickup a way in: a crate the woods closed off gets a trail."""
    for _ in range(4):
        reach = nav_reach()
        lost = [(c, r) for c, r, _ in pickups if not reachable((c, r), reach)]
        if not lost:
            return
        for c, r in lost:
            connect(c, r, reach)


def fill_pockets():
    """Open ground the playfield never reaches grows a tree, so no nav cell
    is left boxed in or cut off."""
    wd = distance_field(water, 4)
    filled = 0
    for _ in range(12):
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
                if kind_at(p) in (None,) + tuple(COVER_KINDS) and p not in water and p not in roads:
                    put(*p, tree_for(*p, wd))
                    filled += 1
                    break
            else:
                wet = [p for p in corners if p in water]
                if wet:
                    wall(*wet[0], "concrete")  # a rock in the shallows
                    filled += 1
    return filled


def close_narrow_trails():
    """An empty cell one wide among trees - something on both sides across
    it - is a trail a hull can take but the AI never will
    (`woods::Woods::narrow_trails`, the linter's narrow trail): it grows a
    fern, soft cover that changes no route."""
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
            put(*p, "fern")
        filled += len(grown)
    return filled


def gate_aprons():
    """The lane a wave rolls in by: everything soft or growing cleared six
    cells in from every gate."""
    for gc, gr in GATES:
        if gr == 0:
            area = rect(gc - 1, 0, gc + 2, 6)
        elif gr == H - 1:
            area = rect(gc - 1, H - 7, gc + 2, H - 1)
        elif gc == 0:
            area = rect(0, gr - 1, 6, gr + 2)
        else:
            area = rect(W - 7, gr - 1, W - 1, gr + 2)
        for p in list(area):
            if carvable(p):
                clear(*p)
            OPEN_GROUND.add(p)


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


def walk_costs():
    """What the router charges from the start to every nav cell: a ford
    `water_ford_path_cost` (3), dry ground 1."""
    first = next(n for n in nav_of(START) if nav_open(*n))
    cost = {first: 0}
    heap = [(0, first)]
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


# ---------------------------------------------------------------- the map


def build():
    waters()
    narrow(19, 21, 2)  # the shallows
    narrow(38, 40, 2)  # the south ford
    # The roads run in straight reaches along the rows and columns: a hull
    # drives four ways, and a diagonal makes it stair-step.
    road([(3, HIGH), (W - 2, HIGH)])  # the old valley road, High Street in the town
    deck_across(HIGH, 46, 57)  # the north bridge
    road([(WALK[2], MAIN), (W - 2, MAIN)])  # the main road
    deck_across(MAIN, 52, 62)  # the main bridge
    road([(FORT_N_ROAD, WALK[1]), (FORT_N_ROAD, HIGH)])  # the fort's north gate up to the valley road
    road([(FORT_S_ROAD, WALK[3]), (FORT_S_ROAD, SOUTH), (65, SOUTH)])  # the south road
    deck_across(SOUTH, BROOK_COL, BROOK_COL)  # over the brook
    road([(WALK[0], 33), (WEST_TRACK, 33), (WEST_TRACK, LODGE_ROW), (FORT_S_ROAD, LODGE_ROW)])  # the forest track
    road([(NORTH_ST, 0), (NORTH_ST, 41)])  # North Street, on as the depot lane
    road([(CHURCH_LN, 0), (CHURCH_LN, HIGH)])
    road([(CAMP_LN, MAIN), (CAMP_LN, H - 1)])  # the camp lane
    road([(98, DEPOT_ROW), (W - 2, DEPOT_ROW)])  # the depot road
    road([(DEPOT_S, 56), (DEPOT_S, H - 1)])  # the depot's south gate
    road([(60, 21), (99, 21)])  # the back lane behind the south terraces

    fort()
    bridgehead()
    north_battery()
    hamlet()
    forest_lodge()
    woodcutters()
    hermit_glade()
    eastwood_glade()
    lake_shore()
    town()
    farm()
    artillery_park()
    fields()
    depot()
    gatehouse_camp()
    # oil drums on the ways across, for whoever comes over them
    drum_cluster(((48, 14), (48, 15)), "oil")  # the north bridge's west end
    drum_cluster(((78, 32), (79, 32)), "oil")  # beside the main road east of the bridgehead
    drum_cluster(((57, 14), (58, 14)), "oil")  # the north bridge's east end
    drum_cluster(((73, 39), (74, 39)), "fuel")  # by the depot's north gate
    drum_cluster(((57, 57), (58, 57)), "oil")  # where the south road crosses the brook
    placed_lamps = lamps()
    gate_aprons()
    for gc, gr in GATES:
        put(gc, gr, "gate")
        GATE_LANES.update(lane(gc, gr))
    planting()
    edges()
    verges()
    wire_up()
    filled = fill_pockets()
    close_narrow_trails()
    return filled, placed_lamps


HEADER = """\
# grand-campaign - Grand Campaign, level 16 and the finale: a river valley
# with every system in the game on it - the river, the town, the fort, the
# oil depot, the forest and the portals. Written by
# tools/gen_grand_campaign.py - change the generator and run it again rather
# than editing this file; re-saving from the map editor would drop these
# comments. Level tables as dotted keys on purpose - a `[spawn]` header would
# swallow every `cells.` line after it.
#
# Protect on the waves plan: eight waves of 4 to 11 tanks, light to super, 60
# in all, rolling in from the east, where the enemy holds the town and the
# depot: the town's two north streets, three gates on the east edge (High
# Street, the main road, the depot road) and two on the south edge (the
# depot's south road, the camp lane). None in the west, the players' side.
# 116 x 59 cells, followed by the camera, in the Soca valley's look. The
# round's seed picks clear, dusk, rain or snow (snow freezes the river and
# the lake into open ground), and night falls five minutes in: the last waves
# come in the dark, the lamps in the fort, the town and the depot the only
# light but the fires.
#
# The river runs down the valley from the north edge into the lake south-east
# of the fort, four or five cells across with open water down its middle,
# crossed only at the north bridge on High Street, the shallows, the main
# bridge on the main road and the south ford above the lake; the brook out of
# the lake to the south edge is a ford all along, planked where the south
# road crosses it.
#
# The fort on the west bank is the players' base, walled twice: a curtain of
# brick with concrete behind it, a gun tower on every corner, a gate on every
# side - the main gate east onto the main road six cells wide, sandbag wings
# outside the other three - and tank traps out on the glacis, which grows no
# trees down to the river. Inside it the concrete redoubt round the bong,
# whose two ways in - the east end of its north wall, the west end of its
# south wall - face no gate, so nothing outside has a straight shot at the
# bong: a tesla beside each way, a bio slush in the other corners, a gun
# tower in the yard over each, the bong in a ring of sandbags, two bong
# packs, health and ammo, and the players' starts. In the yards round it, on
# a gravel walk: the mess, the barracks, the tents, and the magazine, a dead
# end off the walk with the rod from god in it by the start (first met here -
# against the super-heavies; its circle kills the bong too); jet and
# helicopter strikes, grenades and missiles in the corners - every blast ten
# cells and more from the bong - the laser, a minigun, plasma twice, a
# shield, a tower pack, health and ammo.
#
# The players' outposts: the bridgehead across the main bridge, two sandbag
# works with gun towers on their corners and a tesla by the road (a minigun,
# health, ammo); the north battery on the hill over the north bridge, a
# sandbag redan with two gun towers and a bio slush (the laser, plasma); the
# forest lodge in the south-west, a gun tower over the west portal (the FPV
# swarm and the gravity well inside). The portal's twin stands by the depot's
# west gate: a quick way to the depot, and the enemy's quick way to the
# fort's back door.
#
# The hills to the north-west: the hamlet on the old valley road (a barn of
# a minigun, missiles and speed), the orchard. The forest to the south-west
# and along the west edge, the forest track looping through it from the
# fort's postern: the woodcutters' clearing (plasma, the sonic hammer), the
# hermit's glade hidden off the track (the second rod from god), the
# boathouse on the lake shore (plasma, a minigun).
#
# The east bank, held by the enemy. The town: the watermill (grenades), the
# inn, the town hall on High Street (a jet strike), terraces with gardens
# behind them, the church square under an enemy gun tower (grenades, a
# fountain), and the market, an enemy camp - a tesla among the stalls and a
# gun tower on its corner over plasma, a minigun and speed. The farm (a
# minigun in the barn) and its fields of grain behind hedgerows. Astride the
# main road the artillery park: two gun pits with enemy gun towers, a bio
# slush by the road, tents, a fuel dump, and the stash in their lee - a gauss
# rail, the EMP burst, missiles, grenades, a helicopter strike. The oil depot:
# four iron storage tanks, fuel and oil drums in the aisles between them
# wired by oil trails into one fuse, the pump house (the flamethrower), the
# loading bay under an enemy gun tower (a jet strike). Beyond it the
# gatehouse camp on the camp lane, an enemy gun tower and bio slush (grenades,
# a helicopter strike); the east wood (a jet strike in a glade).
#
# 90 pickups: 40 weapon crates, 26 health, 18 ammo, two bong packs. 35 oil and
# fuel drums, 32 cells of oil trail; 17 towers for the players and 9 for the
# enemy; 25 lamp posts.
"""


def write(path):
    lines = [HEADER, "version = 1",
             'bong = "spike"',
             'tank = "leviathan"',
             'tank2 = "titan"',
             'biome = "valley"',
             'weather = ["clear", "dusk", "rain", "snow"]',
             "nightfall = 300.0",
             f"size = [{W}, {H}]",
             'mission.kind = "protect"',
             'spawn.kind = "waves"',
             "spawn.waves = 8",
             "spawn.size = 4",
             "spawn.growth = 1",
             'spawn.tier_start = "light"',
             'spawn.tier_end = "super"',
             ""]
    for (c, r) in sorted(cells, key=lambda p: (p[1], p[0])):
        lines.append(f'cells."{c},{r}" = {cells[(c, r)]}')
    with open(path, "w") as f:
        f.write("\n".join(lines) + "\n")


def sees(a, b):
    """Whether the straight line from cell `a`'s centre to cell `b`'s
    crosses no wall."""
    (x0, y0), (x1, y1) = a, b
    n = int(max(abs(x1 - x0), abs(y1 - y0)) * 8) + 1
    for i in range(1, n):
        t = i / n
        p = (round(x0 + (x1 - x0) * t), round(y0 + (y1 - y0) * t))
        if p not in (a, b) and kind_at(p) == "wall":
            return False
    return True


def check():
    problems = []
    for gc, gr in GATES:
        if not (gc in (0, W - 1) or gr in (0, H - 1)):
            problems.append(f"gate {gc},{gr} is not on the edge")
        # a leashed wave tank never wanders into the bong's 800 px circle
        # (BB-238): 20 cells of leash and 25 of circle
        if math.hypot(gc - BONG[0], gr - BONG[1]) < 45:
            problems.append(f"gate {gc},{gr} is {math.hypot(gc - BONG[0], gr - BONG[1]):.1f} cells from the bong")
    reach = nav_reach()
    near = lambda p, rr: any((p[0] + dc, p[1] + dr) in rr for dc in range(-4, 4) for dr in range(-4, 4))
    for p in (START, START2, BONG):
        if not near(p, reach):
            problems.append(f"{p} is off the playfield")
    for c, r, kind in pickups:
        if not reachable((c, r), reach):
            problems.append(f"{kind} at {c},{r} cannot be reached")
        if kind in FETCHED:
            far = min(math.hypot(c - gc, r - gr) for gc, gr in GATES)
            if far < 20.5:
                problems.append(f"{kind} at {c},{r} is {far:.1f} cells from a gate")
        if kind in BLAST_CLEAR:
            d = math.hypot(c - BONG[0], r - BONG[1])
            if d < BLAST_CLEAR[kind]:
                problems.append(f"{kind} at {c},{r} is {d:.1f} cells from the bong")
    for kind in FIRST_HERE:
        firsts = [(pc, pr) for pc, pr, k in pickups if k == kind]
        if not any(abs(pc - START[0]) <= SIGHT[0] and abs(pr - START[1]) <= SIGHT[1] for pc, pr in firsts):
            problems.append(f"no {kind} in the start's sight box")
    for gc, gr in GATES:
        if not all(nav_open(*n) for n in lane(gc, gr)):
            problems.append(f"gate {gc},{gr} lane is shut")
        elif lane(gc, gr)[-1] not in reach:
            problems.append(f"gate {gc},{gr} is cut off")
    # the fort's curtain whole but for its gates, the redoubt's but for its ways
    for p in outline(*FORT):
        if p not in FORT_GATES and kind_at(p) not in ("wall", "gun_tower"):
            problems.append(f"the curtain is open at {p}")
    for p in outline(*REDOUBT):
        if p not in REDOUBT_WAYS and kind_at(p) != "wall":
            problems.append(f"the redoubt is open at {p}")
    # no gate of the fort has a straight shot at the bong: the line from
    # any of its cells to the bong's crosses a wall
    for g in sorted(FORT_GATES):
        if sees(g, BONG):
            problems.append(f"fort gate {g} has a clear line to the bong")
    # drums and teslas go off, water draws the bong's hop: none near it
    for (c, r), t in cells.items():
        k = t.split('"')[1]
        if k == "barrel" and math.hypot(c - BONG[0], r - BONG[1]) < 8:
            problems.append(f"a drum at {c},{r} is near the bong")
        if k == "tesla" and math.hypot(c - BONG[0], r - BONG[1]) < 4:
            problems.append(f"a tesla at {c},{r} is near the bong")
    for c, r in water:
        if math.hypot(c - BONG[0], r - BONG[1]) < 8:
            problems.append(f"water at {c},{r} within the bong's hop")
    # both portals on ground a tank walks to (BB-240)
    for p, t in cells.items():
        if '"portal"' in t and not near(p, reach):
            problems.append(f"the portal at {p} is off the playfield")
    for p in problems:
        print("CHECK:", p)
    assert not problems, f"{len(problems)} problems"


ASCII = {
    'wall': '#', 'road': '=', 'water': '~', 'bong': 'B', 'start': '1', 'start2': '2',
    'pickup': 'P', 'sandbag': 's', 'fence': 'f', 'tree': 'T', 'palm': 'Y', 'snag': 'x', 'willow': 'W',
    'birch': 'b', 'pine': 'p', 'scots': 'p', 'fir': 'p', 'spruce': 'p', 'tall_grass': '"', 'bush': 'o',
    'berry_bush': 'o', 'autumn_bush': 'o', 'fern': ',', 'reeds': '|', 'gate': 'G', 'tesla': 'Z',
    'gun_tower': 'K', 'bio_slush': 'Q', 'barrel': 'D', 'hay': 'h', 'tyres': 'O', 'wood_crate': 'X',
    'barrier': 'n', 'tank_trap': '*', 'juniper': 'o', 'lamp': 'L', 'oil': ':', 'portal': '@',
}


def ascii_map():
    print("    " + "".join(str(c // 100 % 10) for c in range(W)))
    print("    " + "".join(str(c // 10 % 10) for c in range(W)))
    print("    " + "".join(str(c % 10) for c in range(W)))
    for r in range(H):
        row = []
        for c in range(W):
            k = kind_at((c, r))
            ch = ASCII.get(k, '?') if k else '.'
            if k == 'wall':
                ch = {'brick': '#', 'iron': 'I', 'wood': 'w', 'concrete': 'C', 'glass': 'g'}.get(material_at((c, r)), '#')
            if k == 'water' and is_deep(c, r):
                ch = '%'
            if k in ('gun_tower', 'tesla', 'bio_slush') and 'enemy' in cells[(c, r)]:
                ch = ch.lower()
            row.append(ch)
        print(f"{r:3d} " + "".join(row))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("-o", "--out", default="maps/grand-campaign.toml")
    ap.add_argument("--stats", action="store_true")
    ap.add_argument("--ascii", action="store_true")
    a = ap.parse_args()
    filled, placed_lamps = build()
    if a.ascii:
        ascii_map()
    check()
    write(a.out)
    if a.stats:
        kinds = {}
        for t in cells.values():
            k = t.split('"')[1]
            kinds[k] = kinds.get(k, 0) + 1
        print(f"{len(cells)} cells, {filled} pocket cells filled, {placed_lamps} lamps placed by rule")
        for k, n in sorted(kinds.items(), key=lambda kv: -kv[1]):
            print(f"  {k:12} {n}")
        sides = {"players": 0, "enemy": 0}
        for t in cells.values():
            if any(f'"{k}"' in t for k in ("gun_tower", "tesla", "bio_slush")):
                sides["enemy" if "enemy" in t else "players"] += 1
        print(f"  towers: {sides}")
        cost = walk_costs()
        for gc, gr in GATES:
            inside = lane(gc, gr)[-1]
            c = cost.get(inside)
            print(f"  gate {gc},{gr}: walk {c} steps = {c / 5:.1f} s" if c else f"  gate {gc},{gr}: no walk")
        kinds_p = {}
        for c, r, kind in pickups:
            kinds_p[kind] = kinds_p.get(kind, 0) + 1
        print(f"  {len(pickups)} pickups: " + ", ".join(f"{k} {n}" for k, n in sorted(kinds_p.items(), key=lambda kv: -kv[1])))
        for c, r, kind in pickups:
            walk = min((cost[n] for n in nav_of((c, r)) if n in cost), default=None)
            print(f"  pickup {kind:13} {c},{r}  walk {walk}  bong {math.hypot(c - BONG[0], r - BONG[1]):.1f}")


if __name__ == "__main__":
    main()
