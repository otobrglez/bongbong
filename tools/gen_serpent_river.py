"""Write maps/serpent-river.toml: Serpent River, a hunt across a valley a
broad river snakes through, 107 x 54 cells, laid out region by region from
a fixed seed.

  python3 tools/gen_serpent_river.py [-o maps/serpent-river.toml] [--stats] [--ascii]

A map this size is too big to lay out by hand, so this script is the
source: change it and run it again rather than editing the TOML (a
re-save from the map editor would drop the header too). The same seed
writes the same file every time - the noise is hashed from cell
positions, and every other draw comes from one seeded `random.Random`
in a fixed order. `--ascii` prints the map a character a cell, `--stats`
the counts, the walk to the enemy's bong and every pickup. The checks it
runs before writing: every pickup and both bongs reachable, the base's
walls whole but for their gates, a crate of every weapon met first here in
the start's sight box and every blast weapon two blasts' reach from the
bong, the river a wall but for its six crossings and each of them a way
across, the watermill the one timber building by the water, and only
willow and birch on the banks.

What it lays out (the header written into the map says how it plays):

- the river: five to seven cells wide with deep water down its middle, in
  three loops from the north edge to the south, so that each bank pushes a
  tongue of land into the other's side; it is crossed at three plank
  bridges, two fords where it narrows to a wade and an island with a
  wade either side of it;
- Camp Oggle in the west, the players' base - an outer wall and a
  concrete compound round their bong whose one gate faces away from every
  outer gate - and Fuzz's keep in the north-east, the enemy's walled fort
  round theirs, each with its own towers;
- the hamlet and its orchard in the north-west, the watermill in the
  northern loop, the island, Riverbend (a village the enemy holds in the
  middle loop), the farm in the southern loop, Fuzzton (the enemy's
  village in the south-east) and the woodcutters' camp on the north bank,
  each with a cache of crates, and two ruins for cover;
- the valley's woods everywhere else - pine, spruce, broadleaf and scots
  pine on the slopes, willow and birch near the water, bushes, berries,
  juniper, ferns and autumn scrub under them, tall grass in the glades -
  with two caches hidden in the thickest of them.
"""

import argparse
import heapq
import math
import random
from collections import deque

W, H = 107, 54
SEED = 0xB0B225

rng = random.Random(SEED)

# (col, row) -> inline TOML table text
cells = {}
# cells the woods must leave alone
reserved = set()
water = set()
roads = set()
# the set pieces' open ground, which the verges' undergrowth keeps off
OPEN_GROUND = set()
pickups = []  # (c, r, kind)

BONG = (14, 34)
START = (23, 31)
START2 = (23, 36)
ENEMY_BONG = (89, 16)

# The river's centre line, from the north edge to the south. Each reach
# runs along a row or a column, so every bridge and ford meets it square
# on; the bends between them are rounded by the width.
RIVER = [(44, -3), (44, 4), (46, 8), (50, 9), (66, 9), (69, 11), (70, 14), (70, 19), (68, 23), (64, 24),
         (41, 24), (37, 26), (36, 29), (36, 37), (38, 40), (42, 41), (57, 41), (60, 43), (62, 46), (62, 57)]

# Where the river can be crossed (map cells, inclusive boxes): the checks
# shut each in turn to prove the deep water is a wall everywhere else.
CROSSINGS = {
    "mill bridge": (55, 4, 61, 14),
    "the enemy's ford": (66, 11, 74, 21),
    "the island": (46, 19, 58, 29),
    "main bridge": (31, 30, 41, 36),
    "the shallows": (43, 37, 52, 45),
    "south bridge": (57, 46, 67, 52),
}

TREE_KINDS = {"tree", "pine", "spruce", "scots", "fir", "birch", "willow", "palm", "snag"}
COVER_KINDS = {"tall_grass", "bush", "berry_bush", "juniper", "fern", "autumn_bush", "reeds"}
# weapons: what an enemy on shells turns off its way to fetch
# the weapon rules of docs/levels.md, "The weapons along the way": no rod
# or rail on a Hunt (either ends it in one press), and a blast weapon's
# crate at least two of its blast's radii (256 px) from the players' bong
BANNED_ON_A_HUNT = {"rod_from_god", "gauss_rail"}
BLASTS = {"grenades", "fpv_swarm", "air_heli", "air_jet", "gravity_well"}


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
    """A detail that gives way to anything already placed."""
    if inb(c, r) and (c, r) not in cells:
        put(c, r, kind, **kw)


def kind_at(p):
    t = cells.get(p)
    return t.split('"')[1] if t else None


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
    """Open ground for a set piece: everything goes but the roads and the
    water running through it."""
    for c, r in list(rect(c0, r0, c1, r1)):
        if (c, r) not in roads and (c, r) not in water:
            clear(c, r)
        reserved.add((c, r))
        OPEN_GROUND.add((c, r))


def pickup(c, r, kind):
    assert (c, r) not in water and (c, r) not in roads, f"{kind} at {c},{r} would cover water or road"
    put(c, r, "pickup", pickup=kind)
    pickups.append((c, r, kind))


def bong_ring(bc, br):
    """Sandbags round a bong three cells out, in four corner pieces with a
    gap three cells wide on every side: cover that never seals it. A gap
    of one cell is no way through to the nav grid, and a hunter that
    cannot route to the bong gives it up for the nearest fight."""
    for c, r in outline(bc - 3, br - 3, bc + 3, br + 3):
        if abs(c - bc) <= 1 or abs(r - br) <= 1:
            continue
        put(c, r, "sandbag")


def block(c0, r0, c1, r1, material):
    """A house seen from above: a solid block of wall."""
    for c, r in rect(c0, r0, c1, r1):
        wall(c, r, material)


def house(c0, r0, c1, r1, material, doors):
    """A building: a ring of walls with the `doors` cells left open."""
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) in doors:
            clear(c, r)
            continue
        wall(c, r, material)
    for c, r in rect(c0 + 1, r0 + 1, c1 - 1, r1 - 1):
        clear(c, r)


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
    """A dirt road `width` cells across, on dry ground only: where it meets
    the river it stops at the bank, and a bridge's deck (`deck`) carries it
    over."""
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


def track(points):
    """A one-cell dirt track: fords stay water, the cells either side are
    kept open (soft cover may grow on them)."""
    path = polyline(points)
    for c, r in path:
        if (c, r) in water:
            continue
        k = kind_at((c, r))
        if k is None or k in TREE_KINDS or k in COVER_KINDS:
            put(c, r, "road")
    for c, r in path:
        for dc in (-1, 0, 1):
            for dr in (-1, 0, 1):
                p = (c + dc, r + dr)
                if inb(*p) and p not in water and p not in roads:
                    k = kind_at(p)
                    if k in TREE_KINDS:
                        clear(*p)
                    reserved.add(p)
    return path


def deck_across(r, c0, c1):
    """A plank deck along row `r` over the water between columns c0 and
    c1: a road cell with water north and south of it, which the valley's
    look draws as a bridge (`ground::GroundGrid::bridge`). One row wide, so
    every cell of it has the river on both sides."""
    for c in range(c0, c1 + 1):
        if (c, r) in water:
            put(c, r, "road")
    for c in range(c0, c1 + 1):
        for rr in (r - 1, r + 1):
            reserved.add((c, rr))


def deck_down(c, r0, r1):
    """The same along column `c`, over a reach running east-west."""
    for r in range(r0, r1 + 1):
        if (c, r) in water:
            put(c, r, "road")
    for r in range(r0, r1 + 1):
        for cc in (c - 1, c + 1):
            reserved.add((cc, r))


# ---------------------------------------------------------------- the river


def seg_dist(p, a, b):
    (px, py), (ax, ay), (bx, by) = p, a, b
    dx, dy = bx - ax, by - ay
    length = dx * dx + dy * dy
    t = 0.0 if length == 0 else max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / length))
    return math.hypot(px - (ax + t * dx), py - (ay + t * dy))


def river_dist(c, r):
    return min(seg_dist((c, r), a, b) for a, b in zip(RIVER, RIVER[1:]))


def river():
    """The river: every cell within its half-width of the centre line, the
    half-width wandering between 2.5 and 3.1 cells so the banks are never
    ruled straight. Five or six cells across, so three or four of them down
    the middle are open water no hull enters (docs/water.md)."""
    for r in range(H):
        for c in range(W):
            hw = 2.45 + 0.6 * fbm(c / 5, r / 5, 3)
            if river_dist(c, r) <= hw:
                put(c, r, "water")


def fords_and_island():
    """Where the river can be waded: it narrows to two cells for five rows
    (a ford, all shallow), or spreads round an island with a wade two cells
    wide either side of it."""
    # the enemy's ford, on the reach past Fuzz's keep
    for r in range(14, 19):
        for c in range(63, 78):
            if (c, r) in water and c not in (69, 70):
                clear(c, r)
        for c in (69, 70):
            put(c, r, "water")
    # the shallows, between Riverbend and the farm
    for c in range(43, 48):
        for r in range(35, 48):
            if (c, r) in water and r not in (40, 41):
                clear(c, r)
        for r in (40, 41):
            put(c, r, "water")
    # the island, in the middle of the reach below the watermill's loop
    for c, r in rect(47, 21, 57, 27):
        put(c, r, "water")
    for c, r in rect(49, 23, 55, 25):
        clear(c, r)
        OPEN_GROUND.add((c, r))


# ---------------------------------------------------------------- set pieces


def camp_oggle():
    """The players' base, two rings deep. The outer wall has gates east onto
    the bridge road and north and south onto the other two, a gun tower on
    every corner and sandbag nests outside each gate. Inside it a concrete
    compound round the bong, its one gate on the far, west side: the
    compound's walls stand square in front of all three outer gates, so no
    road has a shot at the bong, and a tank that comes in has to go round it
    by the north or the south lane and down the west one to reach the bong.
    The east yard behind the main gate is where the players start, with the
    armoury - the laser, the air support, grenades, missiles and the sonic
    hammer - and the stores lie along the lanes."""
    c0, r0, c1, r1 = 5, 25, 27, 43
    # the camp's clearing: out to the bank east, and north and south past
    # the corners where the roads turn for its gates, so a tank sliding
    # through the turn has room; west of it a windbreak of pines, a wall
    # with no gate to come round
    clear_rect(c0, r0 - 4, c1 + 7, r1 + 3)
    clear_rect(11, 15, 22, 23)
    clear_rect(11, 44, 22, 52)
    for c, r in rect(1, r0 - 3, c0 - 1, r1 + 3):
        put(c, r, "spruce" if hash01(c, r, 41) < 0.4 else "pine")
    gates = {(c1, r) for r in (32, 33, 34)} | {(c, r0) for c in (15, 16, 17)} | {(c, r1) for c in (15, 16, 17)}
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) not in gates:
            wall(c, r, "brick")
    for c, r in ((c0, r0), (c1, r0), (c0, r1), (c1, r1)):
        put(c, r, "gun_tower")
    # the compound: concrete two cells thick, so a tank pressed against it
    # has no muzzle past its far face, its gate in the west wall
    k0, q0, k1, q1 = 9, 30, 19, 38
    for ring in (0, 1):
        for c, r in outline(k0 + ring, q0 + ring, k1 - ring, q1 - ring):
            if c in (k0, k0 + 1) and r in (33, 34, 35):
                continue
            wall(c, r, "concrete")
    put(*BONG, "bong")
    put(11, 32, "tesla")
    put(17, 32, "bio_slush")
    pickup(17, 36, "bong_health")
    pickup(11, 36, "health")
    put(*START, "start")
    put(*START2, "start2")
    # the armoury in the east yard, by the start: the laser, the air
    # support and the grenades in its sight box, the blast weapons two
    # blasts' reach (256 px) or more from the bong. Enemies take neither
    # the air support nor the grenades; the missiles, which they would take
    # and fly over the walls at the bong, lie deep inside, in the compound
    # and the west lane, where the players come first
    pickup(25, 27, "laser")
    pickup(21, 27, "air_heli")
    pickup(22, 37, "air_jet")
    pickup(26, 30, "grenades")
    pickup(25, 38, "sonic_hammer")
    pickup(12, 36, "missiles")
    pickup(7, 37, "missiles")
    # the stores along the lanes
    pickup(7, 27, "ammo")
    pickup(12, 27, "health")
    pickup(18, 27, "tower_pack")
    pickup(8, 41, "health")
    pickup(13, 41, "shield")
    pickup(20, 41, "ammo")
    for c, r in ((6, 26), (6, 42), (26, 42), (26, 26)):
        put(c, r, "wood_crate")
    # sandbag nests outside the gates either side of the road, the bank in
    # front of the east gate dressed with barriers and tank traps
    for c, r in ((29, 30), (29, 31), (30, 30), (29, 36), (29, 35), (30, 36),
                 (13, 23), (14, 23), (19, 23), (13, 45), (19, 45), (20, 45)):
        put(c, r, "sandbag")
    for c, r in ((31, 28), (31, 38)):
        put(c, r, "barrier")
    for c, r in ((32, 26), (32, 40), (8, 22), (24, 46)):
        put(c, r, "tank_trap")
    # grenades outside the north and south gates
    pickup(21, 22, "grenades")
    pickup(21, 46, "grenades")
    # hay on the meadow outside the north wall
    for c, r in ((9, 22), (10, 22), (11, 22)):
        put(c, r, "hay")


def fuzz_keep():
    """The enemy's fort: a limestone wall with iron in its north-east
    corner, gates west onto the ford, south onto the road and north onto
    the north bank, three enemy gun towers on its corners, an enemy tesla
    and a bio slush inside, their bong in a ring of sandbags, and fuel
    drums and sandbags out on the ways in."""
    c0, r0, c1, r1 = 80, 7, 99, 25
    clear_rect(c0 - 5, r0 - 3, c1, r1 + 4)
    # behind its east wall the forest stands right up to it: no lane round
    # the back for a guard to lose itself in
    for c, r in rect(c1 + 1, r0 - 2, W - 1, r1 + 3):
        put(c, r, "spruce" if hash01(c, r, 47) < 0.45 else "pine")
    gates = {(c0, r) for r in (15, 16, 17)} | {(c, r1) for c in (89, 90, 91)} | {(c, r0) for c in (89, 90, 91, 92)}
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) in gates:
            continue
        wall(c, r, "iron" if (c, r) == (c1, r0) else "brick")
    put(c0, r0, "gun_tower", side="enemy")
    put(c0, r1, "gun_tower", side="enemy")
    put(c1, r1, "gun_tower", side="enemy")
    put(ENEMY_BONG[0], ENEMY_BONG[1] - 5, "tesla", side="enemy")
    put(*ENEMY_BONG, "enemy_bong")
    bong_ring(*ENEMY_BONG)
    # an inner wall screening the bong from the west gate, so the way in
    # turns north or south round it; the barracks in the north-east corner
    # with a health pack and ammo, and the stores
    for r in range(14, 19):
        wall(83, r, "brick")
    house(93, 8, 97, 11, "brick", {(93, 9), (93, 10)})
    pickup(95, 9, "health")
    pickup(96, 10, "ammo")
    for c, r in ((82, 9), (83, 9), (82, 23), (83, 23)):
        put(c, r, "wood_crate")
    # a sandbag line screening the bong from the south gate, and a pair of
    # fuel drums tucked in the yard's far corner, far enough from the bong
    # that their blast does not reach it
    for c in range(86, 93):
        put(c, 22, "sandbag")
    for c, r in ((98, 23), (98, 24)):
        put(c, r, "barrel", drum="fuel")
    # out on the ways in: sandbags at the gates, fuel drums by the ford's
    # track, oil drums by the south road, tank traps on the meadow
    for c, r in ((77, 14), (78, 14), (77, 18), (78, 18), (87, 27), (88, 27), (92, 27), (93, 27),
                 (88, 5), (93, 5)):
        put(c, r, "sandbag")
    for c, r in ((75, 12), (76, 12), (75, 13), (75, 20), (76, 21), (75, 21)):
        put(c, r, "barrel", drum="fuel")
    for c, r in ((84, 28), (85, 28), (84, 29)):
        put(c, r, "barrel", drum="oil")
    for c, r in ((74, 24), (78, 27), (82, 30), (96, 28), (99, 30)):
        put(c, r, "tank_trap")


def hamlet():
    """The hamlet on the north-west bank: two timber houses and a shed with
    plasma in it, a fenced field, hay, and an orchard down to the road."""
    clear_rect(20, 3, 39, 15)
    house(21, 5, 25, 8, "wood", {(23, 8), (24, 8)})
    block(28, 5, 31, 7, "wood")
    block(24, 11, 27, 13, "wood")
    pickup(22, 6, "plasma")
    pickup(32, 9, "ammo")
    pickup(29, 13, "health")
    # the field east of the houses, fenced along its south side
    for c in range(34, 40):
        put(c, 12, "fence")
    for c, r in rect(34, 4, 39, 11):
        if hash01(c, r, 7) < 0.8:
            put(c, r, "tall_grass")
    for c, r in ((30, 11), (31, 11), (20, 10)):
        put(c, r, "hay")
    put(26, 10, "lamp")
    for c, r in ((20, 4), (33, 14), (21, 14)):
        put_free(c, r, "birch")
    # the orchard between the hamlet and the camp: fruit trees three cells
    # apart, so a hull passes between any two of them
    clear_rect(24, 19, 36, 23)
    for c in range(25, 33, 3):
        for r in range(20, 24, 3):
            put(c, r, "tree")
    for c, r in ((27, 21), (30, 21), (33, 21), (29, 23)):
        put(c, r, "berry_bush")


def watermill():
    """The watermill on the north loop's bank (the valley's look runs its
    race to the water): missiles inside, health at the door, and the mill
    bridge beside it."""
    clear_rect(46, 12, 62, 20)
    house(48, 13, 53, 16, "wood", {(50, 16), (51, 16)})
    pickup(49, 14, "missiles")
    pickup(52, 14, "health")
    for c, r in ((61, 19), (62, 19), (61, 20)):
        put(c, r, "wood_crate")
    put(61, 13, "lamp")


def island():
    """The island in the river, reached by a wade either side: the second
    laser and a health pack among the reeds, a willow at each end."""
    pickup(54, 24, "laser")
    pickup(50, 24, "health")
    put(49, 23, "willow")
    put(55, 25, "willow")
    for c, r in ((51, 23), (53, 25), (54, 23), (50, 25)):
        put(c, r, "reeds")


def riverbend():
    """The village in the middle loop, across the main bridge from Camp
    Oggle and held by the enemy: stone houses either side of the road, a
    walled yard with the minigun and plasma watched by an enemy tesla, and
    fuel drums at the bridgehead."""
    clear_rect(39, 27, 64, 38)
    block(42, 28, 45, 30, "brick")
    block(48, 36, 51, 38, "brick")
    block(55, 36, 58, 38, "brick")
    # the yard
    for c, r in outline(57, 27, 63, 31):
        if r == 31 and c in (59, 60, 61):
            continue
        wall(c, r, "brick")
    pickup(58, 28, "minigun")
    pickup(62, 28, "plasma")
    pickup(60, 29, "ammo")
    put(64, 32, "tesla", side="enemy")
    pickup(53, 37, "health")
    pickup(47, 29, "health")
    # the bridgehead's drums, either side of the road
    for c, r in ((40, 30), (41, 30), (40, 29), (40, 37), (41, 37)):
        put(c, r, "barrel", drum="fuel")
    for c, r in ((47, 32), (54, 32)):
        put(c, r, "lamp")
    # the square's market stall, tyre stacks that stand their ground
    for c, r in ((52, 28), (53, 28)):
        put(c, r, "tyres")


def farm():
    """The farm in the southern loop: the farmhouse, a barn with the
    second minigun and speed in it, fenced fields of tall grass and hay."""
    clear_rect(33, 45, 59, 53)
    block(35, 45, 38, 47, "brick")
    house(49, 45, 55, 48, "brick", {(51, 48), (52, 48)})
    pickup(50, 46, "minigun")
    pickup(54, 46, "speedup")
    pickup(40, 46, "health")
    pickup(42, 48, "ammo")
    # the fields between the road and the valley's edge, fenced along the
    # edge: open to the road, so no lane runs along the edge behind them
    for c in range(35, 57):
        put(c, 53, "fence")
    for c, r in rect(35, 51, 56, 52):
        if hash01(c, r, 11) < 0.85:
            put(c, r, "tall_grass")
    for c, r in ((43, 46), (57, 47), (57, 48), (46, 47)):
        put(c, r, "hay")


def fuzzton():
    """The enemy's village in the south-east: houses, the stash in the big
    brick house (missiles and the second sonic hammer), an enemy bio slush
    on the square and a walled yard of drums that goes up together."""
    clear_rect(66, 36, 98, 48)
    block(68, 38, 71, 40, "brick")
    block(75, 38, 78, 40, "wood")
    block(69, 44, 72, 46, "wood")
    house(80, 44, 85, 47, "brick", {(80, 45), (80, 46)})
    pickup(83, 45, "missiles")
    pickup(83, 46, "sonic_hammer")
    pickup(73, 39, "health")
    pickup(76, 44, "ammo")
    pickup(91, 37, "health")
    put(78, 43, "bio_slush", side="enemy")
    # the drum dump east of the road: eight drums packed together, oil and
    # fuel, that go up as one, walled in tyre stacks so no route runs
    # through it (the router drives through a loose drum, a hull into a
    # block of them stalls)
    for c, r in outline(91, 41, 96, 44):
        put(c, r, "tyres")
    for c, r in rect(92, 42, 95, 43):
        put(c, r, "barrel", drum="fuel" if (c + r) % 2 else "oil")
    put(83, 42, "lamp")
    put(66, 42, "tyres")


def ruins():
    """Cover on the open ground: the broken walls of an old toll house on
    the east bank, where the assault on the keep's south gate gathers, and
    a ruined watch tower in the north loop over the enemy's ford."""
    clear_rect(69, 26, 77, 31)
    for c, r in ((71, 27), (72, 27), (73, 27), (71, 28), (71, 30), (75, 27), (75, 28), (75, 30), (74, 30)):
        wall(c, r, "concrete")
    put(73, 29, "tank_trap")
    clear_rect(60, 18, 66, 22)
    for c, r in ((62, 19), (63, 19), (62, 20), (65, 21), (65, 20)):
        wall(c, r, "concrete")
    pickup(64, 20, "ammo")


def woodcutters():
    """The woodcutters' camp on the north bank, on the road to the keep's
    north gate: log piles, a stack of tyres, plasma and a health pack."""
    clear_rect(63, 2, 76, 6)
    for c, r in ((64, 2), (65, 2), (66, 2), (64, 6), (65, 6)):
        put(c, r, "wood_crate")
    pickup(68, 2, "plasma")
    pickup(71, 2, "health")
    put(74, 2, "tyres")
    put(73, 2, "tyres")
    put(70, 6, "hay")


def edges():
    """The valley's edges stand in trees two cells deep, the river's banks
    in willow and birch, and the corner north-east of the keep is forest:
    no lane along the map's edge for a tank to hug."""
    wd = distance_field(water, 5)
    edge = [(c, r) for c, r in rect(0, 0, W - 1, H - 1) if min(c, r, W - 1 - c, H - 1 - r) <= 1]
    edge += list(rect(95, 2, W - 1, 5))
    for c, r in edge:
        if (c, r) in cells:
            continue
        if wd.get((c, r), 9) <= 3:
            put(c, r, tree_for(c, r, wd))
        else:
            put(c, r, "spruce" if hash01(c, r, 53) < 0.4 else "pine")


def crossings():
    """Open ground round every crossing, so its way on and off is never
    planted over; sandbags on the far side of the shallows, lamps at the
    bridge ends."""
    for c0, r0, c1, r1 in CROSSINGS.values():
        for p in rect(c0, r0, c1, r1):
            if p not in cells:
                reserved.add(p)
                OPEN_GROUND.add(p)
    for c, r in ((33, 31), (39, 35)):
        put_free(c, r, "lamp")
    put_free(57, 3, "lamp")
    put_free(59, 47, "lamp")
    for c, r in ((42, 37), (48, 39)):
        put_free(c, r, "sandbag")


# ---------------------------------------------------------------- the woods


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


CANOPY = (("pine", 30), ("spruce", 18), ("tree", 26), ("scots", 12), ("birch", 8), ("fir", 6))
BANK = (("willow", 60), ("birch", 40))
UNDER = (("bush", 26), ("berry_bush", 22), ("juniper", 16), ("fern", 18), ("autumn_bush", 18))


def tree_for(c, r, wd):
    if wd.get((c, r), 9) <= 3:
        return pick(BANK, hash01(c, r, 73))
    return pick(CANOPY, hash01(c, r, 79))


WOOD_GROUND = []


def wood_noise(c, r):
    """How thick the woods stand at (c, r): high is forest, low a meadow."""
    return fbm(c / 8, r / 8, 11)


def woods():
    wd = distance_field(water, 5)
    free = sorted(wood_noise(c, r) for r in range(H) for c in range(W)
                  if (c, r) not in cells and (c, r) not in reserved)
    WOOD_GROUND.append(len(free))
    q = lambda share: free[int(len(free) * share)]
    meadow, thin, mid = q(0.34), q(0.52), q(0.7)
    for r in range(H):
        for c in range(W):
            p = (c, r)
            if p in cells or p in reserved:
                continue
            dw = wd.get(p, 9)
            u = hash01(c, r, 13)
            if dw == 1:
                if hash01(c, r, 85) < 0.4:
                    put(c, r, "reeds")
                    reserved.discard(p)
                continue
            d = wood_noise(c, r)
            if dw <= 3:
                # the banks stay open meadow and gravel, with willows and
                # birches standing along them
                density = 0.2 if d > thin else 0.1
            elif d > mid:
                density = 0.85
            elif d > thin:
                density = 0.55
            elif d > meadow:
                density = 0.2
            else:
                density = 0.03
            if u < density:
                put(c, r, tree_for(c, r, wd))
                reserved.discard(p)
                continue
            g = fbm(c / 5, r / 5, 17)
            v = hash01(c, r, 19)
            if d <= meadow and g > 0.55 and v < 0.75:
                put(c, r, "tall_grass")
            elif d <= meadow and fbm(c / 3, r / 3, 37) > 0.66 and v < 0.55:
                # a patch of autumn scrub and berries in the meadow
                put(c, r, "autumn_bush" if hash01(c, r, 43) < 0.55 else "berry_bush")
            elif d > meadow and v < 0.4:
                put(c, r, pick(UNDER, hash01(c, r, 23)))
            elif v < 0.05:
                put(c, r, pick(UNDER, hash01(c, r, 29)))
            else:
                continue
            reserved.discard(p)


def verges():
    """The bare ground along the roads grows a little cover."""
    for c, r in sorted(roads):
        for dc, dr in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            p = (c + dc, r + dr)
            if not inb(*p) or p in cells or p in roads or p in water or p in OPEN_GROUND:
                continue
            u = hash01(*p, 29)
            if u < 0.1:
                put(*p, "tall_grass")
            elif u < 0.3:
                put(*p, pick(UNDER, hash01(*p, 31)))


# ---------------------------------------------------------------- the nav grid
#
# The game routes on a grid of 32 px cells whose centres sit half a cell off
# the map's (`pathfind::Grid`, `maplint`): a tile at map cell (c, r) blocks
# nav cells c-1..c by r-1..r. So two trees one cell apart leave no nav cell
# between them, and a way the AI can take needs two empty map cells side by
# side. Everything below reasons on that grid, the way the linter does.

LOOSE = {"barrel", "hay", "wood_crate", "barrier"}
SOFT = {"road", "water", "pickup", "start", "start2"} | COVER_KINDS | LOOSE


def is_deep(c, r):
    """Open water: a water cell whose eight neighbours are water too. The
    game's own reading also deepens the odd cell on a diagonal bend
    (`ground::Layout::wet_vertices`), which only ever adds to the wall."""
    return all((c + dc, r + dr) in water for dc in (-1, 0, 1) for dr in (-1, 0, 1))


def blocks(p):
    """Whether map cell `p` holds something the nav grid treats as a wall:
    any tile, open water, a bong."""
    if not inb(*p):
        return False
    if p in water:
        return is_deep(*p)
    k = kind_at(p)
    if k is None:
        return False
    return k not in SOFT


SHUT = set()


def nav_open(k, l):
    """An open nav cell. The field's own edge is a wall with the same
    margin as a tile's, so the outermost ring counts as shut."""
    if not (0 < k < W - 1 and 0 < l < H - 1):
        return False
    if (k, l) in SHUT:
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


def near_reach(p, reach, d=3):
    return any((p[0] + dc, p[1] + dr) in reach for dc in range(-d, d + 1) for dr in range(-d, d + 1))


def carvable(p):
    k = kind_at(p)
    return p not in roads and p not in water and (k is None or k in TREE_KINDS or k in COVER_KINDS)


def connect(c, r, reach, mouth=None):
    """Cut a trail three cells wide from map cell (c, r) out to the
    playfield through trees and cover. `mouth` names the cover the last
    cells of it are hidden in."""
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
    path = []
    p = goal
    while p is not None:
        path.append(p)
        p = prev[p]
    for i, (pc, pr) in enumerate(path):
        for dc in (-1, 0, 1):
            for dr in (-1, 0, 1):
                n = (pc + dc, pr + dr)
                if not inb(*n) or not carvable(n):
                    continue
                if mouth and i < 3:
                    put(n[0], n[1], mouth)
                elif kind_at(n) in TREE_KINDS:
                    clear(*n)
    return True


def hollows(count=2):
    """Caches in the thickest of the woods: a little clearing with
    something in it, reached by a trail whose mouth is hidden in bushes."""
    loot = [("shield", "health"), ("speedup", "health")]
    made = []
    tries = 0
    keep_off = [BONG, ENEMY_BONG, (52, 24), (60, 30), (77, 45), (50, 48), (28, 9), (51, 15), (69, 3)]
    while len(made) < count and tries < 20000:
        tries += 1
        c, r = rng.randint(4, W - 5), rng.randint(4, H - 5)
        if any(math.hypot(c - mc, r - mr) < 30 for mc, mr in made):
            continue
        if any(math.hypot(c - kc, r - kr) < 12 for kc, kr in keep_off):
            continue
        block = list(rect(c - 2, r - 2, c + 2, r + 2))
        if sum(kind_at(p) in TREE_KINDS for p in block) < 18:
            continue
        made.append((c, r))
    for (c, r), (first, second) in zip(made, loot):
        for p in rect(c - 1, r - 1, c + 1, r + 1):
            clear(*p)
        for p in outline(c - 2, r - 2, c + 2, r + 2):
            if kind_at(p) in TREE_KINDS and hash01(*p, 91) < 0.35:
                clear(*p)
        pickup(c, r, first)
        pickup(c + 1, r, second)
    return made


def wire_up(hollow_cells):
    for c, r in hollow_cells:
        connect(c, r, nav_reach(), mouth="bush")
    for _ in range(4):
        reach = nav_reach()
        lost = [(c, r) for c, r, _ in pickups if not reachable((c, r), reach)]
        if not lost:
            return
        for c, r in lost:
            connect(c, r, reach)


def fill_pockets():
    """Open ground the playfield never reaches becomes woods, so no nav
    cell is left boxed in or cut off."""
    wd = distance_field(water, 5)
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
                if (kind_at(p) is None or kind_at(p) in COVER_KINDS) and p not in water and p not in roads:
                    put(*p, tree_for(*p, wd))
                    filled += 1
                    break
    return filled


def close_narrow_trails():
    """An empty cell one wide in the woods - something on both sides across
    it, trees round it - is a trail a hull can take but the AI never will
    (`woods::Woods::narrow_trails`, the linter's narrow trail): it grows a
    fern, soft cover that changes no route, so it reads as undergrowth
    rather than a way through."""
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


# ---------------------------------------------------------------- the walk


def step_cost(k, l):
    """What the router charges to step into nav cell (k, l): a ford
    `water_ford_path_cost` (3), dry ground 1."""
    return 3 if (k, l) in water else 1


def walk_costs(start=START):
    first = next(n for n in nav_of(start) if nav_open(*n))
    cost = {first: 0}
    heap = [(0, first)]
    while heap:
        d, (k, l) = heapq.heappop(heap)
        if d > cost.get((k, l), 1 << 30):
            continue
        for n in ((k + 1, l), (k - 1, l), (k, l + 1), (k, l - 1)):
            if not nav_open(*n):
                continue
            nd = d + step_cost(*n)
            if nd < cost.get(n, 1 << 30):
                cost[n] = nd
                heapq.heappush(heap, (nd, n))
    return cost


# ---------------------------------------------------------------- the map


def build():
    river()
    fords_and_island()

    # The three roads out of Camp Oggle, each to a gate of Fuzz's keep:
    # north by the hamlet and the watermill over the mill bridge and along
    # the north bank; east over the main bridge through Riverbend; south
    # by the farm over the south bridge through Fuzzton.
    road([(16, 25), (16, 17), (58, 17), (58, 3), (90, 3), (90, 6)])
    deck_down(58, 4, 14)
    road([(25, 33), (90, 33), (90, 26)])
    deck_across(33, 30, 42)
    road([(16, 43), (16, 49), (86, 49), (86, 34)])
    deck_across(49, 56, 68)
    # the tracks: over the enemy's ford to the keep's west gate, down to
    # the island's wade from the mill road and on to Riverbend, through
    # the shallows from Riverbend to the farm, and the hamlet's lane
    track([(59, 16), (79, 16)])
    track([(52, 18), (52, 27)])
    track([(45, 35), (45, 48)])
    track([(27, 15), (27, 17)])

    camp_oggle()
    fuzz_keep()
    hamlet()
    watermill()
    island()
    riverbend()
    farm()
    fuzzton()
    woodcutters()
    ruins()
    edges()
    crossings()

    verges()
    woods()
    made = hollows()
    wire_up(made)
    filled = fill_pockets()
    close_narrow_trails()
    return filled, made


HEADER = """\
# serpent-river - Serpent River: a broad river snakes down a valley between
# two bong camps, and the players cross it to hunt the enemy's bong in its
# keep. Written by tools/gen_serpent_river.py from a fixed seed - change the
# generator and run it again rather than editing this file; re-saving from
# the map editor would drop these comments. Level tables as dotted keys on
# purpose - a `[spawn]` header would swallow every `cells.` line after it.
#
# Hunt on the band plan: twelve tanks, out of sight about a quarter of a
# minute's drive from the players. The hunters among them make for the
# players' bong as soon as the round opens; the guards fall back on their
# own. 107 x 54 cells, followed by the camera.
#
# The river is five to seven cells across with open water down its middle,
# and it loops three times from the north edge to the south, so each bank
# pushes a tongue of land into the other side. It is crossed at six places
# and only there: the main bridge east of the players' camp, the mill bridge
# by the watermill and the south bridge below the farm (plank decks in the
# valley's look); the enemy's ford under their keep and the shallows below
# Riverbend, where it narrows to a two-cell wade; and the island, a wade
# either side of it. Snow freezes the whole river into open ground.
#
# West: Camp Oggle, the players' base, two rings deep. A limestone outer wall
# with gates east onto the bridge road and north and south onto the other
# two, a gun tower on every corner, sandbag nests outside the gates and tank
# traps on the bank; inside it a concrete compound two cells thick round the
# bong, with a tesla and a bio slush, a bong pack and a health pack, and its
# one gate on the far, west side. The compound stands square in front of all
# three outer gates, so no road has a shot at the bong and a tank that gets
# in must go round it by the north or the south lane and down the west one.
# The players start in the east yard behind the main gate, among the
# armoury: the laser, a helicopter and a jet of air support, grenades by
# the gate and the sonic hammer; missiles in the compound and the west lane,
# more grenades outside the north and south gates, ammo, health, a shield
# and a tower pack along the lanes, and a windbreak of pines behind the
# west wall. North of it the hamlet (plasma in the shed), an orchard, the
# watermill in the northern loop (missiles inside), a ruined watch tower
# over the enemy's ford, and the island below the mill (the second laser in
# the reeds).
#
# Middle: Riverbend, across the main bridge and held by the enemy - fuel
# drums at the bridgehead, stone houses, an enemy tesla over a walled yard
# with the minigun and plasma. South of the shallows the farm: a barn with a
# minigun and speed, fields of tall grass behind fences.
#
# East: Fuzz's keep, the enemy's fort - a limestone wall with gates west onto
# the ford, south onto the road and north onto the north bank, enemy gun
# towers on three corners and an enemy tesla by the bong, an inner wall
# screening the bong from the west gate and a sandbag line from the south
# one, the barracks (health, ammo), fuel drums in the yard, and more drums,
# sandbags and tank traps out on the ways in; the forest stands right up to
# its back wall. Before its south gate the broken walls of an old toll house,
# cover for the assault. Fuzzton, the enemy's village to the south-east:
# missiles and the second sonic hammer in the big stone house, an enemy bio
# slush on the square, a dump of drums that goes up as one. The woodcutters'
# camp on the north bank, plasma among the log piles.
#
# Pine, spruce, broadleaf and scots pine on the slopes, willow and birch by
# the water, bushes, berries, juniper and autumn scrub under them and in
# patches on the meadows, tall grass in the glades, and two caches hidden in
# the thickest woods at the end of trails whose mouths are bushes.
#
# A valley climate: the round's seed picks clear, dusk, rain, fog or snow.
# Rain halves every hull's grip, fog shortens the enemies' sight, and snow
# freezes the river. Drawn as the Soca gravel valley (`biome`,
# docs/biomes.md): clear alpine water, gravel by the banks, plank decks at
# the bridges, a race at the watermill and furrows in the fenced fields.
"""


def write(path):
    lines = [HEADER, "version = 1",
             'biome = "valley"',
             'weather = ["clear", "dusk", "rain", "fog", "snow"]',
             'bong = "oggle"',
             'enemy_bong = "fuzz"',
             "size = [107, 54]",
             "tanks = 12",
             'tank = "longbow"',
             'tank2 = "obelisk"',
             'mission.kind = "hunt"',
             'spawn.kind = "band"',
             ""]
    for (c, r) in sorted(cells, key=lambda p: (p[1], p[0])):
        lines.append(f'cells."{c},{r}" = {cells[(c, r)]}')
    with open(path, "w") as f:
        f.write("\n".join(lines) + "\n")


def shut_crossings(names):
    SHUT.clear()
    for name in names:
        c0, r0, c1, r1 = CROSSINGS[name]
        for k in range(c0 - 1, c1 + 1):
            for l in range(r0 - 1, r1 + 1):
                SHUT.add((k, l))


def check():
    reach = nav_reach()
    for p in (START, START2):
        assert near_reach(p, reach), p
    # a hull routes right up to each bong: the hunters' and the players'
    # way to it is open
    for p in (BONG, ENEMY_BONG):
        assert near_reach(p, reach, 2), p
    for c, r, kind in pickups:
        assert reachable((c, r), reach), f"{kind} at {c},{r} cannot be reached"
    for c, r, kind in pickups:
        assert kind not in BANNED_ON_A_HUNT, f"{kind} at {c},{r}"
        assert kind not in BLASTS or math.hypot(c - BONG[0], r - BONG[1]) >= 8, f"{kind} at {c},{r}"
    # the base's walls stand whole, but for their gates: nothing laid out
    # after it has cut through them
    for c, r in outline(5, 25, 27, 43):
        if not ((c == 27 and 32 <= r <= 34) or (r in (25, 43) and 15 <= c <= 17)):
            assert kind_at((c, r)) in ("wall", "gun_tower"), (c, r)
    for ring in (0, 1):
        for c, r in outline(9 + ring, 30 + ring, 19 - ring, 38 - ring):
            if not (c in (9, 10) and 33 <= r <= 35):
                assert kind_at((c, r)) == "wall", (c, r)
    # the weapons met first on this level - the laser, the sonic hammer, the
    # air support and the grenades - each have a crate inside the start's
    # sight box (+-11.5 by +-7.5 cells)
    in_box = lambda c, r: abs(c - START[0]) <= 11 and abs(r - START[1]) <= 7
    for first in ("laser", "sonic_hammer", "air_heli", "air_jet", "grenades"):
        assert any(k == first and in_box(c, r) for c, r, k in pickups), first
    # the watermill is the one timber building by the water, so it alone
    # gets the valley's mill race (`biome::places`, RACE_REACH 3 cells)
    timber = [p for p, t in cells.items() if 'material = "wood"' in t]
    by_water = sorted({p for p in timber if any(max(abs(p[0] - w[0]), abs(p[1] - w[1])) <= 3 for w in water)})
    assert by_water and all(46 <= c <= 55 and 12 <= r <= 17 for c, r in by_water), by_water
    # every tree within three cells of the water is a willow or a birch
    # (docs/biomes.md, "Planting and places")
    near_water = distance_field(water, 3)
    for p, t in cells.items():
        k = t.split('"')[1]
        if k in TREE_KINDS and p not in water and near_water.get(p, 9) <= 3:
            assert k in ("willow", "birch"), f"{k} at {p} by the water"
    # the river is a wall but for its crossings: with all of them shut the
    # keep cannot be reached, with any one of them open it can
    shut_crossings(CROSSINGS)
    assert not near_reach(ENEMY_BONG, nav_reach()), "the river leaks"
    for name in CROSSINGS:
        shut_crossings([n for n in CROSSINGS if n != name])
        assert near_reach(ENEMY_BONG, nav_reach()), f"{name} does not cross"
    SHUT.clear()


ASCII = {
    'wall': '#', 'road': '=', 'water': '~', 'bong': 'B', 'enemy_bong': 'E', 'start': '1', 'start2': '2',
    'pickup': 'P', 'sandbag': 's', 'fence': 'f', 'tree': 'T', 'pine': 'A', 'spruce': 'A', 'scots': 'A', 'fir': 'F',
    'willow': 'W', 'birch': 'b', 'tall_grass': '"', 'bush': 'o', 'berry_bush': 'o', 'juniper': 'o',
    'autumn_bush': 'o', 'fern': ',', 'reeds': '|', 'tesla': 'Z', 'gun_tower': 'K', 'bio_slush': 'Q',
    'barrel': 'D', 'hay': 'h', 'tyres': 'O', 'wood_crate': 'X', 'barrier': 'n', 'tank_trap': '*', 'lamp': 'l',
}


def ascii_map():
    print("    " + "".join(str(c // 10 % 10) for c in range(W)))
    print("    " + "".join(str(c % 10) for c in range(W)))
    for r in range(H):
        row = []
        for c in range(W):
            k = kind_at((c, r))
            ch = ASCII.get(k, '?') if k else '.'
            if k == 'wall':
                ch = {'brick': '#', 'iron': 'I', 'wood': 'w', 'concrete': 'C'}.get(cells[(c, r)].split('"')[3], '#')
            if k in ('gun_tower', 'tesla', 'bio_slush') and 'enemy' in cells[(c, r)]:
                ch = ch.lower()
            if k == 'water' and is_deep(c, r):
                ch = '%'
            row.append(ch)
        print(f"{r:3d} " + "".join(row))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("-o", "--out", default="maps/serpent-river.toml")
    ap.add_argument("--stats", action="store_true")
    ap.add_argument("--ascii", action="store_true")
    a = ap.parse_args()
    filled, made = build()
    if a.ascii:
        ascii_map()
    check()
    write(a.out)
    if a.stats:
        kinds = {}
        for t in cells.values():
            k = t.split('"')[1]
            kinds[k] = kinds.get(k, 0) + 1
        print(f"{len(cells)} cells, {WOOD_GROUND[0]} left to the woods, hollows at {made}, "
              f"{filled} pocket cells filled")
        for k, n in sorted(kinds.items(), key=lambda kv: -kv[1]):
            print(f"  {k:12} {n}")
        cost = walk_costs()
        bc, br = ENEMY_BONG
        near = [cost[(bc + dc, br + dr)] for dc in range(-4, 5) for dr in range(-4, 5) if (bc + dc, br + dr) in cost]
        print(f"  walk to the enemy bong: {min(near)} steps = {min(near) / 5:.1f} s")
        for c, r, kind in pickups:
            print(f"  pickup {kind:13} {c},{r}")


if __name__ == "__main__":
    main()
