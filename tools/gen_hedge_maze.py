"""Write maps/hedge-maze.toml: the Hedge Maze, level 8 - a formal garden
round a walled fountain court, 97 x 48 cells, laid out piece by piece.

  python3 tools/gen_hedge_maze.py [-o maps/hedge-maze.toml] [--stats] [--ascii]

The layout is too big to keep by hand, so this script is the source:
change it and run it again rather than editing the TOML (a re-save from
the map editor would drop the header too). Nothing in it is drawn at
random - every scatter is hashed from cell positions - so it writes the
same file every time. `--ascii` prints the map a character a cell,
`--stats` the counts, each gate's walk from the start and every crate.

What it lays out (the header written into the map says how it plays):

- the Fountain Court in the middle, the players' walled base: a gate in
  every side with a baffle wall inside it, so nothing outside has a
  straight shot at the bong on the fountain's island; towers on its
  corners and in two baffles, the stores - supplies, air strikes,
  grenades and missiles - along the walk inside its walls;
- the maze round it: two rings of hedge two cells thick - pine inside,
  fir outside - with gravel walks between them, the inner ring opened at
  one corner of each side in a pinwheel and the outer ring at the middle
  of its west and east sides, so every way in winds;
- the west wing, the players' side: the kitchen garden by the north-west
  gate, the Orangery - a glasshouse outpost with a gun tower and a stash
  - on the lawn every west route crosses, and the rose garden with its
  bio slush by the south-west gate;
- the east wing, the enemy's ground, an open lawn of topiary: the
  gardeners' stockade by the north-east gate with its drums and fuse, the
  Folly - a ruined temple the enemy holds with three towers over a weapon
  stash - and the boating pond by the south-east gate with a gun tower on
  its islet;
- the boundary woods along the north and south edges, with a cache in a
  clearing at the end of each of four dead-end walks.
"""

import argparse
import heapq
import math
from collections import deque

W, H = 97, 48
# (col, row) -> inline TOML table text
cells = {}
# cells the decoration passes must leave alone
reserved = set()
water = set()
roads = set()

COURT = (40, 16, 57, 31)
RING_IN = (33, 10, 64, 37)
RING_OUT = (26, 3, 71, 44)
BONG = (48, 23)
START = (44, 29)
START2 = (53, 29)

GATES = [
    (0, 5), (0, 42),  # west: the kitchen garden, the rose garden
    (96, 5), (96, 42),  # east: the gardeners' yard, the boating pond
]

# what an enemy turns off its road to fetch lies beyond every gate's
# leash (`enemy_leash_px`, 640 px = 20 cells; `Tank::wants_pickup`): the
# flamethrower, grenades and air strikes are the players' alone, and
# health and ammo are left to lie anywhere, as on every field map
FETCHED = {
    "laser", "minigun", "plasma", "missiles", "sonic_hammer", "emp_burst",
    "gauss_rail", "fpv_swarm", "rod_from_god", "gravity_well", "tower_pack",
    "speedup", "shield",
}
# a blast weapon's crate lies at least two of its blast's radii from the
# players' bong (docs/levels.md, "Blasts away from the bong") and never
# nearer than the grenade's 256 px - the missiles' too - in cells
BLAST_CLEARANCE = {
    "grenades": 2 * 128 / 32,
    "missiles": 256 / 32,
    "fpv_swarm": 256 / 32,
    "gravity_well": 2 * 140 / 32,
    "air_jet": 256 / 32,
    "air_heli": 256 / 32,
}
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


def pickup(c, r, kind):
    assert (c, r) not in water, f"{kind} at {c},{r} would cover water"
    assert kind_at((c, r)) in (None, "road") or kind_at((c, r)) in COVER_KINDS, f"{kind} at {c},{r} lands on {kind_at((c, r))}"
    put(c, r, "pickup", pickup=kind)
    pickups.append((c, r, kind))


def drums(spots, kind):
    for c, r in spots:
        put(c, r, "barrel", drum=kind)


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


def pick(table, u):
    total = sum(w for _, w in table)
    acc = 0
    for name, w in table:
        acc += w
        if u * total < acc:
            return name
    return table[-1][0]


def value_noise(x, y, salt):
    i, j = math.floor(x), math.floor(y)
    fx, fy = x - i, y - j
    sx, sy = fx * fx * (3 - 2 * fx), fy * fy * (3 - 2 * fy)
    a = hash01(i, j, salt)
    b = hash01(i + 1, j, salt)
    c = hash01(i, j + 1, salt)
    d = hash01(i + 1, j + 1, salt)
    return (a + (b - a) * sx) + ((c + (d - c) * sx) - (a + (b - a) * sx)) * sy


# ---------------------------------------------------------------- hedges


def hline(c0, c1, r):
    return [(c, r) for c in range(c0, c1 + 1)]


def vline(c, r0, r1):
    return [(c, r) for r in range(r0, r1 + 1)]


def block(c0, r0, c1, r1):
    return set(rect(c0, r0, c1, r1))


def hedge(points, species, gaps=()):
    """A hedge through `points`, every cell in `gaps` left open."""
    for p in points:
        if p in gaps or not inb(*p):
            continue
        put(*p, species)


def ring(c0, r0, c1, r1):
    """The cells of a hedge two cells thick round the box (c0, r0)-(c1, r1),
    its outer edge on the box."""
    return list(outline(c0, r0, c1, r1)) + list(outline(c0 + 1, r0 + 1, c1 - 1, r1 - 1))


def walk_line(points):
    """A gravel walk: road cells, one wide, over bare ground or soft cover
    but never through anything standing."""
    for p in points:
        if inb(*p) and (kind_at(p) is None or kind_at(p) in COVER_KINDS):
            put(*p, "road")


def beds(spots, kind):
    for p in spots:
        put_free(*p, kind)


# ---------------------------------------------------------------- the court


def court():
    """The players' walled base: limestone walls with a gate four cells
    wide in the middle of every side, gun towers on the north-west and
    south-east corners and bio slush on the other two. Three cells inside
    every gate stands a baffle wall as wide as the gate, so nothing outside
    has a straight shot at the bong, which lives on the island in the middle
    of the fountain between the four baffles: a tank coming in turns along
    the walls and reaches the fountain only round a baffle's end. A tesla
    stands in the north and the south baffle. The stores lie along the walk
    round the court."""
    c0, r0, c1, r1 = COURT
    gates = block(47, r0, 50, r0) | block(47, r1, 50, r1) | block(c0, 22, c0, 25) | block(c1, 22, c1, 25)
    for c, r in rect(c0, r0, c1, r1):
        clear(c, r)
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) not in gates:
            wall(c, r, "brick")
    put(c0, r0, "gun_tower")
    put(c1, r1, "gun_tower")
    put(c1, r0, "bio_slush")
    put(c0, r1, "bio_slush")
    # the baffles
    for p in hline(47, 50, 20) + hline(47, 50, 27) + vline(44, 22, 25) + vline(53, 22, 25):
        wall(*p, "brick")
    put(48, 20, "tesla")
    put(49, 27, "tesla")
    # the fountain: a basin four cells across, the bong on its island - a
    # bong hops to water, so it keeps to the basin
    for c, r in rect(47, 22, 50, 25):
        put(c, r, "water")
    put(*BONG, "bong")
    put(*START, "start")
    put(*START2, "start2")
    beds(((45, 21), (52, 26), (45, 26), (52, 21)), "tall_grass")
    # the stores along the walk round the court: supplies, both air
    # strikes, the grenades and the missiles - every blast's crate at least
    # two of its blast's radii from the bong
    pickup(41, 17, "grenades")
    pickup(44, 17, "bong_health")
    pickup(53, 17, "ammo")
    pickup(56, 17, "air_jet")
    pickup(41, 21, "tower_pack")
    pickup(56, 21, "health")
    pickup(41, 26, "health")
    pickup(56, 26, "shield")
    pickup(41, 30, "missiles")
    pickup(56, 30, "air_heli")
    # a gravel walk round the court inside its walls, and in from every gate
    walk_line(hline(42, 55, 18) + hline(42, 55, 29) + vline(42, 18, 29) + vline(55, 18, 29))
    walk_line(vline(48, 13, 18) + vline(49, 29, 34) + hline(37, 42, 23) + hline(55, 60, 24))
    # concrete barriers to shove across a side gate
    for c, r in ((38, 21), (38, 26), (59, 21), (59, 26)):
        put(c, r, "barrier")


# ---------------------------------------------------------------- the maze


def maze():
    """Two rings of hedge round the court, each two cells thick. The inner
    ring (pine) opens at one corner of each side, turning like a pinwheel;
    the outer ring (fir) opens at the middle of its west and east sides and
    onto four dead-end walks into the boundary woods."""
    a0, b0, a1, b1 = RING_IN
    pin = block(35, b0, 38, b0 + 1) | block(a1 - 1, 12, a1, 15) | block(59, b1 - 1, 62, b1) | block(a0, 32, a0 + 1, 35)
    hedge(ring(a0, b0, a1, b1), "pine", gaps=pin)
    o0, p0, o1, p1 = RING_OUT
    mouths = block(o0, 22, o0 + 1, 25) | block(o1 - 1, 22, o1, 25)
    alcoves = block(39, p0, 42, p0 + 1) | block(55, p0, 58, p0 + 1) | block(39, p1 - 1, 42, p1) | block(55, p1 - 1, 58, p1)
    hedge(ring(o0, p0, o1, p1), "fir", gaps=mouths | alcoves)
    # clipped cones of spruce on the rings' corners
    for c, r in ((a0, b0), (a1, b0), (a0, b1), (a1, b1), (o0, p0), (o1, p0), (o0, p1), (o1, p1)):
        put(c, r, "spruce")
    # the walks' caches: a weapon by each of the inner ring's openings, a
    # special in the middle of the outer walk's long sides, the stores in
    # its corners
    pickup(35, 12, "laser")
    pickup(62, 12, "grenades")
    pickup(62, 35, "minigun")
    pickup(35, 35, "missiles")
    pickup(48, 6, "emp_burst")
    pickup(49, 41, "sonic_hammer")
    pickup(29, 14, "plasma")
    pickup(68, 33, "missiles")
    pickup(44, 33, "bong_health")
    pickup(69, 5, "health")
    pickup(28, 42, "health")
    pickup(28, 5, "ammo")
    pickup(69, 42, "ammo")
    # flower beds in the walks' corners
    beds(((28, 6), (29, 5), (29, 6), (68, 41), (68, 42), (69, 41)), "tall_grass")
    beds(((68, 5), (68, 6), (28, 41), (29, 42)), "autumn_bush")
    beds(((39, 12), (39, 13), (58, 34), (58, 35), (35, 15), (62, 32)), "tall_grass")
    beds(((44, 7), (52, 7), (45, 40), (53, 40)), "berry_bush")
    # oil and fuel drums in the outer walk's east side, a trap on the way in
    # from the enemy's lawn
    drums([(69, 18), (69, 19)], "oil")
    drums([(69, 28), (69, 29)], "fuel")
    # the gravel walks down the middle of both rings and through every gap
    walk_line(hline(37, 60, 13) + hline(37, 60, 34) + vline(37, 13, 34) + vline(60, 13, 34))
    walk_line(hline(30, 67, 7) + hline(30, 67, 40) + vline(30, 7, 40) + vline(67, 7, 40))
    walk_line(vline(37, 7, 13) + hline(60, 67, 13) + vline(60, 34, 40) + hline(30, 37, 34))
    walk_line(hline(21, 30, 23) + hline(67, 76, 24))
    # flower beds of tall grass to hide in along the galleries: two cells
    # against a hedge every few cells down the walks' long sides
    for c in range(38, 60, 7):
        beds(((c, 9), (c + 1, 9), (c + 3, 38), (c + 4, 38)), "tall_grass")
    for r in (10, 17, 30, 36):
        beds(((28, r), (28, r + 1), (69, 47 - r), (69, 46 - r)), "tall_grass")


WOOD = (("tree", 40), ("birch", 22), ("scots", 18), ("pine", 10), ("willow", 4), ("fir", 6))


def boundary_woods():
    """The woods along the north and south edges over the maze, and the
    four dead-end walks into them, each to a cache in a clearing."""
    o0, p0, o1, p1 = RING_OUT
    for c, r in rect(o0, 0, o1, p0 - 1):
        put(c, r, pick(WOOD, hash01(c, r, 3)))
    for c, r in rect(o0, p1 + 1, o1, H - 1):
        put(c, r, pick(WOOD, hash01(c, r, 5)))
    for c0 in (39, 55):
        for c, r in rect(c0, 1, c0 + 3, p0 - 1):
            clear(c, r)
        for c, r in rect(c0, p1 + 1, c0 + 3, H - 2):
            clear(c, r)
    pickup(40, 2, "gravity_well")
    pickup(57, 2, "fpv_swarm")
    pickup(40, 45, "grenades")
    pickup(57, 45, "flamethrower")
    beds(((39, 2), (58, 2), (39, 45), (58, 45)), "fern")


# ---------------------------------------------------------------- the west wing


def west_wing():
    """The players' side: three garden rooms divided by spruce hedges -
    the kitchen garden by the north-west gate, the Orangery lawn in the
    middle and the rose garden by the south-west gate. Both gates' roads
    turn south or north through the dividers' wide openings by the maze
    and cross the Orangery's lawn to the outer ring's west mouth; a
    narrower opening by the edge is the flank."""
    for r0 in (14, 32):
        hedge(rect(0, r0, 25, r0 + 1), "spruce", gaps=block(18, r0, 23, r0 + 1) | block(1, r0, 4, r0 + 1))
    orangery()
    kitchen_garden()
    rose_garden()
    walk_line(hline(1, 21, 5) + vline(21, 5, 42) + hline(1, 21, 42))


def orangery():
    """The Orangery: a glasshouse on the lawn, palms in tubs inside, doors on
    every side, a player gun tower on its north-east corner; the stash
    inside - a laser, the sonic hammer, a minigun and a speed-up - with
    health and ammo."""
    c0, r0, c1, r1 = 5, 19, 16, 28
    for c, r in rect(c0, r0, c1, r1):
        clear(c, r)
    doors = block(c1, 20, c1, 22) | block(c1, 25, c1, 27) | block(9, r0, 11, r0) | block(9, r1, 11, r1)
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) not in doors:
            wall(c, r, "glass")
    put(c1, r0, "gun_tower")
    for c, r in ((7, 21), (7, 26), (14, 23)):
        put(c, r, "palm")
    pickup(12, 23, "laser")
    pickup(12, 24, "sonic_hammer")
    pickup(14, 25, "minigun")
    pickup(14, 21, "speedup")
    pickup(9, 23, "health")
    pickup(9, 24, "ammo")
    # beds of roses along its west wall and box hedges at its doors
    for r in range(20, 28):
        put(4, r, "berry_bush" if r % 3 else "autumn_bush")
    beds(((17, 19), (17, 28), (8, 18), (12, 18), (8, 29), (12, 29)), "juniper")
    # oil drums by the lawn's north opening, a trap where the waves come
    # through the divider
    drums([(24, 17), (25, 17), (25, 18)], "oil")
    beds(((1, 17), (2, 17), (1, 30), (2, 30), (24, 30), (25, 30), (25, 29)), "tall_grass")


def kitchen_garden():
    """The kitchen garden by the north-west gate: four fenced beds - wheat,
    greens, berries and pumpkins - a potting shed with the stores, hay,
    crates and two oil drums, and the flamethrower, for the weeds, on the
    path along the edge."""
    crops = ("tall_grass", "berry_bush", "fern", "autumn_bush")
    for (c0, r0), crop in zip(((6, 8), (13, 8), (6, 11), (13, 11)), crops):
        for c, r in outline(c0, r0, c0 + 5, r0 + 2):
            put(c, r, "fence")
        for c in range(c0 + 1, c0 + 5):
            put(c, r0 + 1, crop)
    for c, r in outline(13, 0, 19, 4):
        if (c, r) in ((15, 4), (16, 4), (17, 4)):
            continue
        wall(c, r, "wood")
    pickup(14, 3, "health")
    pickup(18, 3, "ammo")
    pickup(3, 11, "flamethrower")
    for c, r in ((22, 1), (23, 1), (22, 2)):
        put(c, r, "hay")
    for c, r in ((11, 1), (11, 2)):
        put(c, r, "wood_crate")
    drums([(24, 9), (24, 10)], "oil")
    for c in (1, 4, 7):
        put_free(c, 0, "birch")


def rose_garden():
    """The rose garden: a ring of roses round an inner ring of berries and
    a belvedere of four stone pillars with a player bio slush in it, paths
    in from every side; a helicopter strike on its lawn by the maze, health
    and ammo by the gate."""
    cx, cy = 11, 38
    for c, r in disc(cx, cy, 3.6):
        d = math.hypot(c - cx, r - cy)
        on_path = c == cx or r == cy
        if 2.6 < d and not on_path:
            put(c, r, "autumn_bush")
        elif 1.5 < d <= 2.6 and not on_path:
            put(c, r, "berry_bush")
    for c, r in ((cx - 1, cy - 1), (cx + 1, cy - 1), (cx - 1, cy + 1), (cx + 1, cy + 1)):
        wall(c, r, "concrete")
    put(cx, cy, "bio_slush")
    pickup(24, 37, "plasma")
    pickup(4, 45, "health")
    pickup(17, 46, "ammo")
    for c, r in ((2, 35), (19, 35), (2, 44), (19, 39), (24, 44)):
        put_free(c, r, "willow")


# ---------------------------------------------------------------- the east wing


def east_wing():
    """The enemy's ground: one open lawn of topiary from the maze to the
    east edge, the gardeners' yard in its north, the Folly in its middle
    and the boating pond in its south; a road down it from gate to gate
    with a branch to the outer ring's east mouth."""
    gardeners_yard()
    folly()
    boating_pond()
    # an enemy tesla by the east mouth, tank traps either side of it
    put(72, 19, "tesla", side="enemy")
    for c, r in ((72, 21), (72, 26)):
        put(c, r, "tank_trap")
    # topiary on the lawn behind the Folly: clipped cones of spruce and
    # scots pine among box hedges
    for c, r in ((93, 18), (93, 29)):
        put(c, r, "spruce")
    for c, r in ((77, 16), (77, 31)):
        put(c, r, "scots")
    beds(((94, 18), (94, 29), (92, 20), (92, 27), (95, 22), (95, 25)), "juniper")
    beds(((78, 33), (79, 33), (78, 34), (78, 14), (79, 14)), "tall_grass")
    pickup(74, 23, "health")
    walk_line(vline(76, 5, 42) + hline(76, 95, 5) + hline(76, 95, 42))


def gardeners_yard():
    """The gardeners' yard by the north-east gate: a timber stockade with
    a gate on its west and north sides and an enemy gun tower in its
    corner, grenades, a fuel dump and a compost heap of hay inside; north of
    the road the potting shed, and west of it two clusters of drums tied by
    an oil fuse."""
    c0, r0, c1, r1 = 81, 8, 95, 14
    gates = block(c0, 10, c0, 12) | block(86, r0, 88, r0)
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) not in gates:
            wall(c, r, "wood")
    put(93, 9, "gun_tower", side="enemy")
    drums([(83, 13), (84, 13), (83, 12)], "fuel")
    for c, r in ((90, 13), (91, 13), (92, 13), (91, 12)):
        put(c, r, "hay")
    for c, r in ((94, 12), (94, 13)):
        put(c, r, "wood_crate")
    pickup(87, 11, "health")
    pickup(89, 10, "grenades")
    pickup(85, 10, "ammo")
    # the potting shed and its fuse
    for c, r in outline(85, 0, 90, 4):
        if (c, r) in ((87, 4), (88, 4), (89, 4)):
            continue
        wall(c, r, "wood")
    for c, r in ((86, 1), (89, 1)):
        put(c, r, "wood_crate")
    drums([(78, 1), (79, 1), (78, 2)], "fuel")
    for c in range(80, 84):
        put(c, 2, "oil")
    drums([(84, 1), (84, 2)], "oil")
    for c, r in ((92, 1), (93, 1), (92, 2)):
        put(c, r, "hay")
    for c, r in ((80, 7), (79, 7)):
        put(c, r, "tyres")


def folly():
    """The Folly: a ruined temple of grey stone the enemy holds - a door on
    three sides, its walls broken in places, gun towers on two corners and
    a bio slush at its heart, sandbags flanking its west door - and in it the
    enemy's stash: plasma, missiles, the EMP burst and a health pack. Fuel
    drums stand behind it."""
    c0, r0, c1, r1 = 80, 18, 89, 29
    for c, r in rect(c0, r0, c1, r1):
        clear(c, r)
    doors = block(c0, 22, c0, 25) | block(83, r0, 86, r0) | block(83, r1, 86, r1)
    broken = {(c1, 21), (c1, 26)}
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) in doors or (c, r) in broken:
            continue
        wall(c, r, "concrete")
    put(c0, r0, "gun_tower", side="enemy")
    put(c1, r1, "gun_tower", side="enemy")
    put(85, 23, "bio_slush", side="enemy")
    pickup(82, 20, "plasma")
    pickup(82, 27, "missiles")
    pickup(82, 23, "emp_burst")
    pickup(87, 20, "health")
    for c, r in ((79, 20), (79, 21), (79, 26), (79, 27)):
        put(c, r, "sandbag")
    drums([(91, 23), (91, 24), (92, 24)], "fuel")


def boating_pond():
    """The boating pond by the south-east gate: a lake with reeds and
    willows on its banks and an enemy gun tower on the islet in its middle,
    a boathouse on the shore by the road."""
    cx, cy = 87, 36
    for c, r in disc(cx, cy, 3.6):
        put(c, r, "water")
    for c, r in disc(cx, cy, 4.7):
        if (c, r) not in water and hash01(c, r, 11) < 0.5:
            put(c, r, "reeds")
    put(cx, cy, "gun_tower", side="enemy")
    for c, r in outline(78, 43, 84, 47):
        if (c, r) in ((80, 43), (81, 43), (82, 43)):
            continue
        wall(c, r, "wood")
    pickup(79, 45, "health")
    pickup(83, 45, "flamethrower")
    pickup(81, 46, "ammo")
    drums([(86, 45), (87, 45), (86, 46)], "oil")
    for c, r in ((81, 34), (93, 33), (93, 39), (81, 39)):
        put_free(c, r, "willow")


# ---------------------------------------------------------------- the lawns


FLOWERS = (("tall_grass", 50), ("autumn_bush", 18), ("berry_bush", 14), ("fern", 10), ("juniper", 8))
SPECIMEN = (("birch", 45), ("willow", 30), ("tree", 25))


def lawns():
    """The wings' open lawns grow flowers in drifts and a specimen tree
    here and there - outside the lanes the waves take between the wings'
    roads and the maze, never within two cells of a road, never against a wall,
    a prop or a tree and never on a gate's apron."""
    near_gate = lambda c, r: any(math.hypot(c - gc, r - gr) < 7 for gc, gr in GATES)
    for r in range(H):
        for c in list(range(0, 20)) + list(range(79, W)):
            p = (c, r)
            if p in cells or p in reserved or near_gate(c, r):
                continue
            ring = [(c + dc, r + dr) for dc in (-2, -1, 0, 1, 2) for dr in (-2, -1, 0, 1, 2)]
            if any(q in roads for q in ring):
                continue
            if any(q in cells and kind_at(q) not in COVER_KINDS for q in ring if abs(q[0] - c) <= 1 and abs(q[1] - r) <= 1):
                continue
            drift = value_noise(c / 3.5, r / 3.5, 61)
            u = hash01(c, r, 67)
            if drift > 0.68 and u < 0.55:
                put(c, r, pick(FLOWERS, hash01(c, r, 71)))
            elif drift < 0.2 and u < 0.06 and not any((c + dc, r + dr) in cells for dc in (-2, -1, 0, 1, 2) for dr in (-2, -1, 0, 1, 2)):
                put(c, r, pick(SPECIMEN, hash01(c, r, 73)))


# ---------------------------------------------------------------- the nav grid
#
# The game routes on a grid of 32 px cells whose centres sit half a cell off
# the map's (`pathfind::Grid`, `maplint`): a tile at map cell (c, r) blocks
# nav cells c-1..c by r-1..r. So two trees one cell apart leave no nav cell
# between them, and a way the AI can take needs two empty map cells side by
# side. Everything below reasons on that grid, the way the linter does.

LOOSE = {"barrel", "hay", "wood_crate", "barrier"}
SOFT = {"road", "water", "pickup", "oil", "portal", "start", "start2", "gate"} | COVER_KINDS | LOOSE
GATE_LANES = set()


def is_deep(c, r):
    return all((c + dc, r + dr) in water for dc in (-1, 0, 1) for dr in (-1, 0, 1))


def blocks(p):
    """Whether map cell `p` holds something the nav grid treats as a wall:
    any tile, deep water, the bong."""
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


def fill_pockets():
    """Open ground the playfield never reaches grows a fern, then a tree, so
    no nav cell is left boxed in or cut off."""
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
                if (kind_at(p) is None or kind_at(p) in COVER_KINDS) and p not in water and p not in roads:
                    put(*p, pick(WOOD, hash01(*p, 41)))
                    filled += 1
                    break
    return filled


def close_narrow_trails():
    """An empty cell one wide among trees - something on both sides across
    it - is a trail a hull can take but the AI never will
    (`woods::Woods::narrow_trails`): it grows a fern, soft cover that
    changes no route, so it reads as undergrowth rather than a way
    through."""
    taken = lambda p: blocks(p) or not inb(*p)
    filled = 0
    for _ in range(3):
        grown = []
        for r in range(H):
            for c in range(W):
                p = (c, r)
                if blocks(p) or kind_at(p) in COVER_KINDS:
                    continue
                trees = sum(kind_at((c + a, r + b)) in TREE_KINDS for a in (-1, 0, 1) for b in (-1, 0, 1))
                across = (taken((c - 1, r)) and taken((c + 1, r))) or (taken((c, r - 1)) and taken((c, r + 1)))
                if trees >= 2 and across:
                    grown.append(p)
        if not grown:
            break
        for p in grown:
            if kind_at(p) == "pickup":
                raise AssertionError(f"a crate at {p} sits in a trail one cell wide")
            put(*p, "fern")
        filled += len(grown)
    return filled


def gate_aprons():
    """The lane a wave rolls in by: trees and cover cleared five cells in
    from every gate."""
    for gc, gr in GATES:
        if gc == 0:
            area = rect(0, gr - 2, 5, gr + 2)
        else:
            area = rect(W - 6, gr - 2, W - 1, gr + 2)
        for p in list(area):
            k = kind_at(p)
            if k in TREE_KINDS or k in COVER_KINDS:
                clear(*p)


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


def step_cost(k, l):
    """What the router charges to step into nav cell (k, l): a ford
    `water_ford_path_cost` (3), dry ground 1."""
    return 3 if (k, l) in water else 1


def walk_costs():
    start = next(n for n in nav_of(START) if nav_open(*n))
    cost = {start: 0}
    heap = [(0, start)]
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
    boundary_woods()
    maze()
    court()
    west_wing()
    east_wing()
    lawns()
    gate_aprons()
    for gc, gr in GATES:
        put(gc, gr, "gate")
        GATE_LANES.update(lane(gc, gr))
    filled = fill_pockets()
    ferns = close_narrow_trails()
    return filled, ferns


HEADER = """\
# hedge-maze - the Hedge Maze, level 8: a formal garden round a walled
# fountain court. Written by tools/gen_hedge_maze.py - change the generator
# and run it again rather than editing this file; re-saving from the map
# editor would drop these comments. Level tables as dotted keys on purpose -
# a `[spawn]` header would swallow every `cells.` line after it.
#
# Protect on the waves plan: five waves of 3 to 7 tanks, light to heavy,
# rolling in at the four corner gates, in a scout, under a clear sky, at
# dusk or in fog. 97 x 48 cells, followed by the camera, in the valley's
# look - the garden walls in dressed limestone.
#
# The bong lives on the island in the fountain at the heart of the Fountain
# Court, the players' walled base. A gate four cells wide opens in every
# side, and three cells inside each stands a baffle wall as wide as the
# gate, so nothing outside has a straight shot at the bong: a tank coming in
# turns along the walls and reaches the fountain only round a baffle's end.
# Gun towers stand on the court's north-west and south-east corners, bio
# slush on the other two, a tesla in the north and the south baffle. The
# stores lie along the gravel walk inside the walls: a bong pack, a tower
# pack, a shield, health and ammo, a jet and a helicopter strike, grenades
# and missiles - every blast's crate at least 256 px from the bong.
#
# Round the court the maze: an inner ring of pine and an outer ring of fir,
# each two cells thick, gravel walks four and five cells wide between them,
# flower beds of tall grass to hide in along the galleries. The inner ring
# opens at one corner of each side, turning like a pinwheel; the outer ring
# opens only at the middle of its west and east sides - so every way in
# winds, and a shell or a ram through a hedge is a shortcut. A weapon waits
# by each of the inner ring's openings (laser, grenades, minigun,
# missiles), the EMP burst and the sonic hammer in the middle of the outer
# walk's long sides, plasma and more missiles along its west and east ones;
# four dead-end walks run from the outer ring into the boundary woods to
# caches in clearings - the gravity well, the FPV swarm, grenades and the
# flamethrower. Oil and fuel drums wait in the outer walk's east side.
#
# The west wing is the players' side: three garden rooms divided by spruce
# hedges. The kitchen garden by the north-west gate (four fenced beds, the
# potting shed with the stores, the flamethrower on the edge path) and the
# rose garden by the south-west gate (a bio slush in its stone belvedere,
# plasma on its lawn by the maze) both lead onto the Orangery's lawn, where
# a glasshouse with a gun tower holds a laser, the sonic hammer, a minigun
# and a speed-up, and oil drums stand by the north opening.
#
# The east wing is the enemy's ground, an open lawn of topiary. The Folly, a
# ruined temple in its middle, holds plasma, missiles and the EMP burst under
# two enemy gun towers and a bio slush, and an enemy tesla and tank traps
# guard the outer ring's east mouth. The gardeners' stockade by the
# north-east gate has an enemy gun tower, grenades and a fuel dump, and two
# clusters of drums tied by an oil fuse by its potting shed; the boating
# pond by the south-east gate has an enemy gun tower on its islet and the
# boathouse a flamethrower.
"""


def write(path):
    lines = [HEADER, "version = 1",
             'bong = "oggle"',
             'tank = "scout"',
             'tank2 = "wraith"',
             'biome = "valley"',
             'weather = ["clear", "dusk", "fog"]',
             f"size = [{W}, {H}]",
             'mission.kind = "protect"',
             'spawn.kind = "waves"',
             "spawn.waves = 5",
             "spawn.size = 3",
             "spawn.growth = 1",
             'spawn.tier_start = "light"',
             'spawn.tier_end = "heavy"',
             ""]
    for (c, r) in sorted(cells, key=lambda p: (p[1], p[0])):
        lines.append(f'cells."{c},{r}" = {cells[(c, r)]}')
    with open(path, "w") as f:
        f.write("\n".join(lines) + "\n")


def check():
    for gc, gr in GATES:
        assert gc in (0, W - 1) or gr in (0, H - 1), (gc, gr)
    for c, r, kind in pickups:
        if kind in FETCHED:
            near = min(math.hypot(c - gc, r - gr) for gc, gr in GATES)
            assert near >= 20.5, f"{kind} at {c},{r} is {near:.1f} cells from a gate"
        if kind in BLAST_CLEARANCE:
            d = math.hypot(c - BONG[0], r - BONG[1])
            assert d >= BLAST_CLEARANCE[kind], f"{kind} at {c},{r} is {d:.1f} cells from the bong"
    reach = nav_reach()
    near = lambda p, rr: any((p[0] + dc, p[1] + dr) in rr for dc in range(-3, 3) for dr in range(-3, 3))
    for p in (START, START2, BONG):
        assert near(p, reach), p
    for c, r, kind in pickups:
        assert reachable((c, r), reach), f"{kind} at {c},{r} cannot be reached"
    for gc, gr in GATES:
        assert all(nav_open(*n) for n in lane(gc, gr)), f"gate {gc},{gr}"
        assert lane(gc, gr)[-1] in reach, f"gate {gc},{gr} is cut off"


ASCII = {
    'wall': '#', 'road': '=', 'water': '~', 'bong': 'B', 'start': '1', 'start2': '2',
    'pickup': 'P', 'sandbag': 's', 'fence': 'f', 'tree': 'T', 'pine': 'A', 'fir': 'F', 'spruce': 'S',
    'scots': 'c', 'palm': 'Y', 'snag': 'x', 'willow': 'W', 'birch': 'b', 'tall_grass': '"', 'bush': 'o',
    'berry_bush': 'o', 'autumn_bush': 'a', 'juniper': 'j', 'fern': ',', 'reeds': '|', 'gate': 'G',
    'tesla': 'Z', 'gun_tower': 'K', 'bio_slush': 'Q', 'barrel': 'D', 'hay': 'h', 'tyres': 'O',
    'wood_crate': 'X', 'barrier': 'n', 'tank_trap': '*', 'oil': ':',
}


def ascii_map():
    print("    " + "".join(str(c // 10 % 10) for c in range(W)))
    print("    " + "".join(str(c % 10) for c in range(W)))
    for r in range(H):
        row = []
        for c in range(W):
            k = kind_at((c, r))
            ch = ASCII.get(k, '?') if k else '.'
            t = cells.get((c, r), "")
            if k == 'wall':
                ch = {'brick': '#', 'iron': 'I', 'wood': 'w', 'concrete': 'C', 'glass': 'g'}.get(t.split('"')[3], '#')
            if 'enemy' in t:
                ch = ch.lower()
            if k == 'water' and is_deep(c, r):
                ch = '%'
            row.append(ch)
        print(f"{r:3d} " + "".join(row))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("-o", "--out", default="maps/hedge-maze.toml")
    ap.add_argument("--stats", action="store_true")
    ap.add_argument("--ascii", action="store_true")
    a = ap.parse_args()
    filled, ferns = build()
    if a.ascii:
        ascii_map()
    check()
    write(a.out)
    if a.stats:
        kinds = {}
        for t in cells.values():
            k = t.split('"')[1]
            kinds[k] = kinds.get(k, 0) + 1
        print(f"{len(cells)} cells, {filled} pocket cells filled, {ferns} narrow trails closed")
        for k, n in sorted(kinds.items(), key=lambda kv: -kv[1]):
            print(f"  {k:12} {n}")
        cost = walk_costs()
        for gc, gr in GATES:
            inside = lane(gc, gr)[-1]
            c = cost.get(inside)
            print(f"  gate {gc},{gr}: walk {c} steps = {c / 5 if c else 0:.1f} s" if c else f"  gate {gc},{gr}: no walk")
        for c, r, kind in pickups:
            print(f"  pickup {kind:13} {c},{r}")


if __name__ == "__main__":
    main()
