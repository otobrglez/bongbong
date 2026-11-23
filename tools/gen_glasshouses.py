"""Write maps/glasshouses.toml: Glasshouse Gardens, level 3 - a Victorian
botanical garden of glass houses, 68 x 34 cells, laid out piece by piece.

  python3 tools/gen_glasshouses.py [-o maps/glasshouses.toml] [--stats] [--ascii]

This script is the map's source: change it and run it again rather than
editing the TOML (a re-save from the map editor would drop the header too).
It draws nothing at random - the set pieces are placed by hand and the
planting is hashed from cell positions - so every run writes the same file.
`--ascii` prints the map a character a cell, `--stats` the counts, each
gate's walk to the start and every pickup.

What it lays out (the header written into the map says how it plays):

- the Palm House in the middle, a cross of glass panes on iron ribs - a
  hall with a wing either side - with the bong in a ring of sandbags under
  its palms, a tesla on two of the ring's corners, gun towers on the hall's
  corners, a bio tower in each wing, a tower in four of the sandbag nests
  outside its doors, and the start inside;
- the parterre north of it: the great fountain on the north walk, striped
  bedding with yews behind, lamp posts on the terrace;
- the long canals and their bedding on the south lawn;
- the fern dell's pond and a fountain on the broad walk in the west; the
  alpine house and a sundial among stone outcrops in the east;
- four glass houses in the corners, each holding a stash and each held by
  an enemy tower: the orchid house (north-west), the cactus house
  (north-east), the lily house round its pool (south-east), and the boiler
  house and its potting shed (south-west), its fuel drums wired by an oil
  trail to the drums on the south-west gate's way in;
- the waves' six gates: west and east at the ends of the broad walk, two
  on the north edge and two on the south, the ways in from all four of the
  north and south ones held by enemy towers;
- the planting between them, every tree kept from cutting ground off the
  playfield or pinching a way to one cell (the nav grid modelled the way
  the linter reads it), and a shelter belt along the edges.
"""

import argparse
import heapq
import math
from collections import deque

W, H = 68, 34

# (col, row) -> inline TOML table text
cells = {}
# cells the planting passes must leave alone
reserved = set()
water = set()
roads = set()
pickups = []  # (c, r, kind)

BONG = (33, 16)
START = (31, 21)
START2 = (36, 21)

GATES = [
    (0, 16),  # west, the broad walk
    (67, 17),  # east, the broad walk
    (20, 0), (47, 0),  # north
    (20, 33), (47, 33),  # south
]

# The weapons whose first crate in the campaign lies here (docs/levels.md):
# each must lie inside the start's sight box (+-11.5 x +-7.5 cells).
FIRST_HERE = {"sonic_hammer", "flamethrower"}
SIGHT_BOX = (11.5, 7.5)

TREE_KINDS = {"tree", "pine", "spruce", "scots", "fir", "birch", "willow", "palm", "snag"}
COVER_KINDS = {"tall_grass", "bush", "berry_bush", "juniper", "fern", "autumn_bush", "reeds"}
TOWERS = {"gun_tower", "tesla", "bio_slush"}


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


def rect(c0, r0, c1, r1):
    for r in range(r0, r1 + 1):
        for c in range(c0, c1 + 1):
            if inb(c, r):
                yield c, r


def outline(c0, r0, c1, r1):
    for c, r in rect(c0, r0, c1, r1):
        if c in (c0, c1) or r in (r0, r1):
            yield c, r


def disc(cx, cy, rad):
    for r in range(int(cy - rad) - 1, int(cy + rad) + 2):
        for c in range(int(cx - rad) - 1, int(cx + rad) + 2):
            if inb(c, r) and (c - cx) ** 2 + (r - cy) ** 2 <= rad * rad:
                yield c, r


def keep_clear(c0, r0, c1, r1):
    """Open ground a set piece needs: the planting leaves it alone."""
    for p in rect(c0, r0, c1, r1):
        reserved.add(p)


def pickup(c, r, kind):
    assert (c, r) not in water and (c, r) not in roads, f"{kind} at {c},{r} would cover water or road"
    put(c, r, "pickup", pickup=kind)
    keep_clear(c - 1, r - 1, c + 1, r + 1)
    pickups.append((c, r, kind))


def tower(c, r, kind, enemy=False):
    if enemy:
        put(c, r, kind, side="enemy")
    else:
        put(c, r, kind)


def drum(c, r, kind):
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


# ---------------------------------------------------------------- pieces


def glasshouse(c0, r0, c1, r1, frame, doors, rib=3, open_sides=()):
    """A glass house: panes of glass on `frame` ribs every `rib` cells and
    at the corners, with `doors` (cells) left open. `open_sides` names
    cells of the outline that are not built at all (where a wing joins)."""
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) in doors or (c, r) in open_sides:
            clear(c, r)
            continue
        corner = c in (c0, c1) and r in (r0, r1)
        on_rib = (r in (r0, r1) and (c - c0) % rib == 0) or (c in (c0, c1) and (r - r0) % rib == 0)
        wall(c, r, frame if corner or on_rib else "glass")
    keep_clear(c0, r0, c1, r1)


def fountain(c0, r0, statue="concrete"):
    """A basin four cells across with a plinth two across in its middle:
    the water round it is a ring of fords a hull wades, the plinth stone."""
    for c, r in rect(c0, r0, c0 + 3, r0 + 3):
        put(c, r, "water")
    for c, r in rect(c0 + 1, r0 + 1, c0 + 2, r0 + 2):
        wall(c, r, statue)


def road_rect(c0, r0, c1, r1):
    for c, r in rect(c0, r0, c1, r1):
        if (c, r) not in water:
            put(c, r, "road")


def bed(c0, r0, c1, r1, flowers, edge="juniper", salt=0):
    """A flower bed edged in clipped box: soft cover a hull ploughs
    through, `flowers` (a weighted table) inside the edging. `edge` None
    leaves it unedged."""
    for c, r in rect(c0, r0, c1, r1):
        if (c, r) in cells:
            continue
        if edge and (c in (c0, c1) or r in (r0, r1)):
            put(c, r, edge)
        else:
            put(c, r, pick(flowers, hash01(c, r, salt)))


def striped_bed(c0, r0, c1, r1, rows, edge="juniper"):
    """A bedding scheme: rows of one shrub each, `rows` in turn, in an
    edging of clipped box at the two ends."""
    for c, r in rect(c0, r0, c1, r1):
        if (c, r) in cells:
            continue
        if edge and c in (c0, c1):
            put(c, r, edge)
        else:
            put(c, r, rows[(r - r0) % len(rows)])


ROSES = (("berry_bush", 55), ("autumn_bush", 30), ("bush", 15))
BORDER = (("autumn_bush", 45), ("berry_bush", 35), ("tall_grass", 20))


def palm_house():
    """The players' hold: the hall (cols 26-41, rows 9-24) and its two
    wings (rows 12-21) out to cols 18 and 49, glass on iron ribs, a door
    four cells wide on every side. The bong in its sandbag ring under the
    palms, a tesla either side of it, gun towers on the hall's corners, a
    bio tower in each wing, the start inside, the stores round the walls."""
    hall = (26, 9, 41, 24)
    north = {(c, 9) for c in range(32, 36)}
    south = {(c, 24) for c in range(32, 36)}
    west_join = {(26, r) for r in range(13, 21)}
    east_join = {(41, r) for r in range(13, 21)}
    glasshouse(*hall, "iron", north | south, rib=3, open_sides=west_join | east_join)
    west_door = {(18, r) for r in range(15, 19)}
    east_door = {(49, r) for r in range(15, 19)}
    glasshouse(18, 12, 26, 21, "iron", west_door, rib=3, open_sides=west_join)
    glasshouse(41, 12, 49, 21, "iron", east_door, rib=3, open_sides=east_join)
    # the gun towers stand on the hall's four corners
    for c, r in ((26, 9), (41, 9), (26, 24), (41, 24)):
        tower(c, r, "gun_tower")
    # the bong in its ring of sandbags, open on the four walks
    bc, br = BONG
    put(bc, br, "bong")
    for c, r in outline(bc - 2, br - 2, bc + 2, br + 2):
        if c == bc or r == br:
            continue
        put(c, r, "sandbag")
    tower(bc - 2, br - 2, "tesla")
    tower(bc + 2, br + 2, "tesla")
    # a bio tower in each wing, by its door
    tower(19, 13, "bio_slush")
    tower(48, 20, "bio_slush")
    put(*START, "start")
    put(*START2, "start2")
    # the stores
    pickup(28, 22, "sonic_hammer")
    pickup(39, 22, "flamethrower")
    pickup(28, 10, "tower_pack")
    pickup(39, 10, "bong_health")
    pickup(33, 22, "health")
    pickup(24, 14, "health")
    pickup(24, 19, "minigun")
    pickup(43, 19, "ammo")
    pickup(43, 14, "shield")
    # sandbag nests outside the four doors, the walk left open between
    for c, r in ((30, 8), (31, 8), (36, 8), (37, 8), (30, 25), (31, 25), (36, 25), (37, 25),
                 (16, 13), (16, 14), (16, 19), (16, 20), (51, 13), (51, 14), (51, 19), (51, 20)):
        put(c, r, "sandbag")
    # a tower in four of the nests, on the cell a sandbag held, so the
    # doorway stays as wide: teslas at the north and south doors, a gun
    # tower over the broad walk from the west gate, a bio tower over it
    # from the east
    tower(31, 8, "tesla")
    tower(36, 25, "tesla")
    tower(16, 14, "gun_tower")
    tower(51, 19, "bio_slush")
    # the ground in front of every door stays open
    keep_clear(30, 5, 37, 8)
    keep_clear(30, 25, 37, 28)
    keep_clear(14, 13, 17, 20)
    keep_clear(50, 13, 53, 20)


def walks():
    """The gravel walks: the broad walk east-west through the wings, the
    north and south walks through the hall, and the terrace walks round
    the palm house that tie them together."""
    road_rect(0, 16, 17, 17)
    road_rect(50, 16, 67, 17)
    road_rect(33, 5, 34, 8)
    road_rect(33, 25, 34, 31)
    # the terraces north and south of the palm house, and the side walks
    road_rect(15, 7, 52, 7)
    road_rect(15, 26, 52, 26)
    road_rect(15, 7, 15, 26)
    road_rect(52, 7, 52, 26)


def parterre():
    """North of the hall: the great fountain on the north walk, the walk
    circling it, two bedding schemes either side of it and two borders
    under the wings, lamp posts on the terrace, and oil drums on the two
    ways in from the north gates."""
    for c, r in outline(30, 0, 37, 5):
        put(c, r, "road")
    fountain(32, 1)
    striped_bed(23, 2, 28, 5, ("autumn_bush", "berry_bush"))
    striped_bed(39, 2, 44, 5, ("berry_bush", "autumn_bush"))
    # clipped yews behind the beds, along the garden's north wall
    for c in list(range(22, 30)) + list(range(38, 46)):
        put(c, 1, "spruce")
    bed(18, 9, 23, 10, BORDER, salt=7)
    bed(44, 9, 49, 10, BORDER, salt=9)
    for c, r in ((24, 6), (43, 6), (16, 6), (51, 6)):
        put(c, r, "lamp")
    pickup(29, 3, "health")
    pickup(38, 3, "ammo")
    pickup(46, 6, "plasma")
    # the enemy holds both ways in from the north gates: a tesla by the
    # west one's drums, a gun tower between the east one and the cactus
    # house's door
    tower(18, 4, "tesla", enemy=True)
    tower(49, 2, "gun_tower", enemy=True)
    for c, r in ((16, 3), (16, 4), (17, 4)):
        drum(c, r, "oil")
    for c, r in ((51, 3), (51, 4), (50, 4)):
        drum(c, r, "oil")
    keep_clear(22, 1, 45, 6)


def south_lawn():
    """South of the hall: the long canal either side of the south walk,
    bedding along it, the borders under the wings, lamps at the walk's
    head, and oil drums on the two ways in from the south gates."""
    for c, r in rect(23, 28, 30, 29):
        put(c, r, "water")
    for c, r in rect(37, 28, 44, 29):
        put(c, r, "water")
    striped_bed(24, 30, 29, 31, ("berry_bush", "autumn_bush"), edge=None)
    striped_bed(38, 30, 43, 31, ("autumn_bush", "berry_bush"), edge=None)
    bed(18, 23, 23, 24, BORDER, salt=15)
    bed(44, 23, 49, 24, BORDER, salt=17)
    for c, r in ((30, 27), (37, 27)):
        put(c, r, "lamp")
    for c, r in ((22, 27), (45, 27)):
        put_free(c, r, "willow")
    pickup(31, 29, "health")
    pickup(36, 29, "bong_health")
    pickup(17, 27, "plasma")
    # and both ways in from the south gates: a gun tower by the west one's
    # drums and the oil trail's end, a bio tower against the lily house
    tower(18, 30, "gun_tower", enemy=True)
    tower(52, 32, "bio_slush", enemy=True)
    for c, r in ((16, 29), (16, 30), (17, 30)):
        drum(c, r, "oil")
    for c, r in ((51, 29), (51, 30), (50, 30)):
        drum(c, r, "oil")
    keep_clear(22, 27, 45, 31)


def arboretum():
    """The west: the broad walk's fountain with the walk circling it, the
    fern dell's pond under willows north of it, and an enemy bio tower in
    the ferns south of it covering the walk."""
    fountain(6, 15)
    for c, r in outline(5, 14, 10, 19):
        if (c, r) not in water:
            put(c, r, "road")
    # the fern dell's pond, deep in the middle
    for r, (c0, c1) in ((10, (4, 6)), (11, (3, 8)), (12, (2, 9)), (13, (3, 8))):
        for c in range(c0, c1 + 1):
            put(c, r, "water")
    for c, r in ((3, 10), (7, 10), (9, 11), (2, 13), (9, 13)):
        put_free(c, r, "reeds")
    for c, r in ((2, 11), (8, 10)):
        put_free(c, r, "fern")
    tower(4, 22, "bio_slush", enemy=True)
    pickup(11, 14, "health")
    pickup(12, 20, "plasma")
    # oil drums by the fountain: a trap for whatever comes down the walk
    for c, r in ((2, 19), (3, 19), (3, 20)):
        drum(c, r, "oil")
    for c, r in ((3, 23), (5, 23)):
        drum(c, r, "oil")
    pickup(11, 10, "speedup")
    pickup(17, 11, "health")
    keep_clear(0, 14, 11, 20)


def orchid_house():
    """North-west: a long glass range on white wooden ribs, orchids in a
    bed down its middle, an enemy gun tower inside it and a stash; a
    path from its south door to the side walk."""
    doors = {(c, 7) for c in range(8, 11)} | {(12, r) for r in range(3, 6)}
    glasshouse(1, 1, 12, 7, "wood", doors, rib=4)
    tower(3, 3, "gun_tower", enemy=True)
    for c in range(4, 10):
        put(c, 5, pick(ROSES, hash01(c, 5, 21)))
    pickup(10, 2, "laser")
    pickup(2, 6, "missiles")
    pickup(6, 2, "health")
    pickup(10, 6, "ammo")
    pickup(7, 3, "plasma")
    for c, r in ((2, 2), (4, 2)):
        drum(c, r, "oil")
    road_rect(8, 8, 15, 8)
    keep_clear(0, 0, 14, 9)


def cactus_house():
    """North-east: a glass house of cacti - palms and dead snags in the
    dry - on iron ribs, an enemy bio tower inside with fuel drums beside
    it, and a stash."""
    doors = {(c, 6) for c in range(60, 63)} | {(55, r) for r in range(2, 5)}
    glasshouse(55, 1, 66, 6, "iron", doors, rib=4)
    tower(64, 2, "bio_slush", enemy=True)
    for c, r in ((56, 5), (65, 2)):
        put(c, r, "palm")
    put(59, 2, "snag")
    pickup(62, 2, "minigun")
    pickup(64, 5, "plasma")
    pickup(57, 2, "shield")
    pickup(59, 4, "health")
    for c, r in ((65, 4), (65, 5)):
        drum(c, r, "fuel")
    keep_clear(53, 0, 67, 9)


def rock_garden():
    """The east: the sundial in its round bed on the broad walk, the walk
    parting round it; the alpine house north of it with its stash; south
    of the walk rock outcrops of grey stone, an enemy gun tower on the one
    by the edge covering the east gate, and drums by the gate."""
    for c, r in outline(56, 15, 61, 18):
        put(c, r, "road")
    for c, r in rect(57, 16, 60, 17):
        put(c, r, "autumn_bush")
    for c, r in rect(58, 16, 59, 17):
        wall(c, r, "concrete")
    doors = {(c, 14) for c in range(62, 65)}
    glasshouse(60, 10, 66, 14, "wood", doors, rib=3)
    pickup(61, 11, "laser")
    pickup(65, 12, "speedup")
    put(63, 11, "juniper")
    put(64, 11, "fern")
    for c, r in ((60, 19), (61, 19), (61, 20), (66, 19)):
        wall(c, r, "concrete")
    tower(66, 20, "gun_tower", enemy=True)
    pickup(58, 20, "health")
    pickup(63, 22, "ammo")
    for c, r in ((57, 12), (57, 13), (56, 13)):
        drum(c, r, "fuel")
    for c, r in ((65, 20), (65, 21)):
        drum(c, r, "oil")
    pickup(50, 22, "health")
    keep_clear(53, 8, 67, 23)


def lily_house():
    """South-east: a glass pavilion round the lily pool (its middle deep),
    reeds at the water, the second sonic hammer at the far side of the
    pool and an enemy gun tower at the door with oil drums either side."""
    doors = {(54, r) for r in range(27, 30)} | {(c, 24) for c in range(59, 62)}
    glasshouse(54, 24, 66, 32, "iron", doors, rib=4)
    for c, r in rect(57, 27, 63, 29):
        put(c, r, "water")
    for c, r in ((56, 27), (64, 29), (56, 29), (64, 27)):
        put_free(c, r, "reeds")
    for c, r in ((55, 25), (65, 25), (65, 31), (55, 31)):
        put(c, r, "palm")
    pickup(65, 28, "sonic_hammer")
    pickup(60, 31, "health")
    pickup(57, 31, "missiles")
    pickup(63, 25, "shield")
    pickup(62, 31, "plasma")
    tower(57, 23, "gun_tower", enemy=True)
    for c, r in ((56, 22), (58, 22)):
        drum(c, r, "oil")
    keep_clear(53, 22, 67, 33)


def boiler_yard():
    """South-west: the boiler house that heats the glass, brick round a
    stash, its fuel drums stacked outside and wired to an oil trail; the
    potting shed, compost and the gardeners' clutter, an enemy tesla at
    the boiler-house door."""
    doors = {(c, 26) for c in range(5, 8)}
    for c, r in outline(2, 26, 9, 32):
        if (c, r) in doors:
            continue
        wall(c, r, "brick")
    pickup(3, 31, "tower_pack")
    pickup(8, 31, "minigun")
    pickup(6, 30, "ammo")
    tower(3, 27, "tesla", enemy=True)
    drum(4, 27, "fuel")
    # the fuel store along the boiler house's east wall, and an oil trail
    # from it past the compost to the oil drums on the south-west gate's
    # way in: one blast at either end and the other goes up too
    for c, r in ((10, 28), (10, 29), (11, 29), (10, 30)):
        drum(c, r, "fuel")
    for c in range(10, 17):
        put(c, 31, "oil")
    # the potting shed: wood, crates inside, a door south
    for c, r in outline(10, 22, 14, 25):
        if (c, r) in ((11, 25), (12, 25), (13, 25)):
            continue
        wall(c, r, "wood")
    put(11, 23, "wood_crate")
    put(13, 23, "wood_crate")
    pickup(12, 23, "flamethrower")
    pickup(12, 24, "health")
    # compost heaps and tyres
    for c, r in ((13, 29), (14, 29), (14, 28)):
        put(c, r, "hay")
    for c, r in ((13, 32), (14, 32)):
        put(c, r, "tyres")
    keep_clear(0, 21, 14, 33)


# ---------------------------------------------------------------- planting


TREES = (("tree", 30), ("birch", 18), ("scots", 14), ("spruce", 10), ("pine", 10), ("willow", 8), ("fir", 10))
UNDER = (("bush", 25), ("berry_bush", 20), ("autumn_bush", 25), ("fern", 15), ("juniper", 15))
MEADOW = (("tall_grass", 70), ("autumn_bush", 15), ("berry_bush", 15))
TROPICS = (("fern", 45), ("bush", 25), ("tall_grass", 15), ("berry_bush", 15))
BELT = (("spruce", 30), ("tree", 25), ("pine", 20), ("birch", 15), ("scots", 10))


def palm_house_interior():
    """Under the glass: gravel walks from the four doors to the bong's
    ring, palms, and a tropical undergrowth of ferns and shrubs a hull
    hides in - the house's own tight sight lines. A cell's width round
    the ring, the doors and the start stays open."""
    road_rect(33, 10, 34, 13)
    road_rect(33, 19, 34, 23)
    road_rect(19, 16, 30, 17)
    road_rect(36, 16, 48, 17)
    for c, r in ((27, 10), (40, 10), (27, 23), (40, 23), (21, 13), (23, 20), (45, 13), (46, 20)):
        put(c, r, "palm")
    bc, br = BONG
    inside = sorted(set(rect(27, 10, 40, 23)) | set(rect(19, 13, 25, 20)) | set(rect(42, 13, 48, 20)))
    for c, r in inside:
        if (c, r) in cells:
            continue
        if abs(c - bc) <= 3 and abs(r - br) <= 3:
            continue
        if any(abs(c - sc) <= 1 and abs(r - sr) <= 1 for sc, sr in (START, START2)):
            continue
        if any(abs(c - pc) <= 1 and abs(r - pr) <= 1 for pc, pr, _ in pickups):
            continue
        if c in (19, 48) or r in (10, 23):
            continue  # inside the doors and along the hall's end walls
        if hash01(c, r, 71) < 0.55:
            put(c, r, pick(TROPICS, hash01(c, r, 73)))


def boundary_belt():
    """The garden's shelter belt: a row of trees one cell in from the
    field's edge wherever no set piece, walk or gate needs the ground, and
    shrubs in front of it - it frames the garden and keeps the waves off
    the edge, where a hull would scrape along the border."""
    near_gate = lambda c, r: any(abs(c - gc) <= 3 and abs(r - gr) <= 3 for gc, gr in GATES)
    for r in range(H):
        for c in range(W):
            p = (c, r)
            ring = min(c, r, W - 1 - c, H - 1 - r)
            if ring > 1 or p in cells or p in reserved or near_gate(c, r):
                continue
            if ring == 1:
                plant_tree(c, r, pick(BELT, hash01(c, r, 81)))
            elif hash01(c, r, 83) < 0.5:
                put(c, r, pick(UNDER, hash01(c, r, 85)))


def planting():
    """The lawns between the set pieces: specimen trees in groves with
    shrubs round them, meadows left long with wild flowers in them, and
    the odd flowering shrub on the open lawn."""
    for r in range(H):
        for c in range(W):
            p = (c, r)
            if p in cells or p in reserved:
                continue
            d = fbm(c / 4, r / 4, 41)
            u = hash01(c, r, 43)
            if d > 0.58:
                if u < 0.5:
                    plant_tree(c, r, pick(TREES, hash01(c, r, 47)))
                elif u < 0.75:
                    put(c, r, pick(UNDER, hash01(c, r, 53)))
            elif d > 0.52:
                if u < 0.4:
                    put(c, r, pick(UNDER, hash01(c, r, 55)))
            elif d < 0.36:
                if u < 0.6:
                    put(c, r, pick(MEADOW, hash01(c, r, 57)))
            elif u < 0.1:
                put(c, r, pick(UNDER, hash01(c, r, 59)))


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
    if not inb(*p):
        return False
    if p in water:
        return is_deep(*p)
    k = kind_at(p)
    if k is None:
        return False
    return k not in SOFT


def nav_open(k, l):
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


REACH = []
PLANTED = set()


def plant_tree(c, r, kind):
    """Plant a tree unless it would cut ground off the playfield: the
    playfield may lose only the nav cells the trunk itself stands on."""
    if not REACH:
        REACH.append(nav_reach())
    before = REACH[0]
    put(c, r, kind)
    after = nav_reach()
    covered = sum(1 for n in nav_of((c, r)) if n in before and not nav_open(*n))
    if len(before) - len(after) > covered:
        cells.pop((c, r), None)
        return False
    REACH[0] = after
    PLANTED.add((c, r))
    return True


def narrows(reach=None):
    """The playfield's one-cell-wide passages, as the linter finds them
    (`maplint::check_narrow_corridors`): an open nav cell sealed on one
    axis with traffic through it on the other. Every tank passing drives
    at the clearance limit there."""
    reach = reach if reach is not None else nav_reach()
    found = []
    for k, l in sorted(reach, key=lambda n: (n[1], n[0])):
        if k in (0, W - 1) or l in (0, H - 1):
            continue  # a gate's lane, held open through the field's edge
        x_sealed = not nav_open(k - 1, l) and not nav_open(k + 1, l)
        y_sealed = not nav_open(k, l - 1) and not nav_open(k, l + 1)
        x_flows = nav_open(k - 1, l) or nav_open(k + 1, l)
        y_flows = nav_open(k, l - 1) or nav_open(k, l + 1)
        if (x_sealed and y_flows) or (y_sealed and x_flows):
            found.append((k, l))
    return found


def widen_narrows():
    """Take out a planted tree that pinches a passage to one cell, until
    none is left that the planting made: the set pieces' own passages are
    laid out two cells wide by hand."""
    felled = 0
    for _ in range(8):
        cut = set()
        for k, l in narrows():
            sealed = []
            if not nav_open(k - 1, l) and not nav_open(k + 1, l):
                sealed += [(k - 1, l), (k + 1, l)]
            if not nav_open(k, l - 1) and not nav_open(k, l + 1):
                sealed += [(k, l - 1), (k, l + 1)]
            for nk, nl in sealed:
                for p in ((nk, nl), (nk + 1, nl), (nk, nl + 1), (nk + 1, nl + 1)):
                    if p in PLANTED:
                        cut.add(p)
        if not cut:
            break
        for p in sorted(cut):
            cells.pop(p, None)
            PLANTED.discard(p)
            felled += 1
    REACH[:] = [nav_reach()]
    return felled


def pockets():
    """The open nav cells the playfield does not reach, before anything is
    planted: the set pieces must leave none (each would be a dead end a
    tank could be stuck in)."""
    reach = nav_reach()
    return [(k, l) for l in range(H) for k in range(W) if (k, l) not in reach and nav_open(k, l)]


def fill_pockets():
    """Open ground the planting cut off from the playfield grows a tree, so
    no nav cell is left boxed in or cut off. Only the planting's ground:
    a set piece's own open ground must never need it."""
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
                if kind_at(p) is None and p not in water and p not in roads and p not in reserved:
                    put(*p, pick(TREES, hash01(*p, 61)))
                    filled += 1
                    break
    return filled


def close_narrow_trails():
    """An empty cell one wide between trees - something on both sides
    across it - is a trail a hull can take but the AI never will (the
    linter's narrow trail): it grows a fern, soft cover that changes no
    route."""
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
    """The lane a wave rolls in by: five cells in from every gate kept
    clear of planting."""
    for gc, gr in GATES:
        if gr == 0:
            area = rect(gc - 2, 0, gc + 2, 5)
        elif gr == H - 1:
            area = rect(gc - 2, H - 6, gc + 2, H - 1)
        elif gc == 0:
            area = rect(0, gr - 2, 5, gr + 2)
        else:
            area = rect(W - 6, gr - 2, W - 1, gr + 2)
        for p in list(area):
            k = kind_at(p)
            if k in TREE_KINDS or k in COVER_KINDS:
                clear(*p)
            reserved.add(p)


def lane(gc, gr):
    k, l = min(gc, W - 1), min(gr, H - 1)
    if gr == 0:
        return [(k, l + i) for i in range(4)]
    if gr == H - 1:
        return [(k, l - i) for i in range(4)]
    if gc == 0:
        return [(k + i, l) for i in range(4)]
    return [(k - i, l) for i in range(4)]


def step_cost(k, l):
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
    walks()
    palm_house()
    parterre()
    south_lawn()
    arboretum()
    orchid_house()
    cactus_house()
    lily_house()
    rock_garden()
    boiler_yard()
    gate_aprons()
    for gc, gr in GATES:
        put(gc, gr, "gate")
        GATE_LANES.update(lane(gc, gr))
    palm_house_interior()
    designed = pockets()
    boundary_belt()
    planting()
    felled = widen_narrows()
    filled = fill_pockets()
    narrow = close_narrow_trails()
    return designed, filled, narrow + felled


HEADER = """\
# glasshouses - Glasshouse Gardens, level 3: a Victorian botanical garden
# under glass, and the campaign's first spike.
# Written by tools/gen_glasshouses.py - every choice in it is placed by hand
# or hashed from a cell's position, so a rerun writes the same file. Change
# the generator and run it again rather than editing this file; re-saving
# from the map editor would drop these comments. Level tables as dotted keys
# on purpose - a `[spawn]` header would swallow every `cells.` line after it.
#
# Protect on the waves plan: four waves of 4 to 7 tanks (22 in all), light
# to heavy, rolling in at six gates - the two ends of the broad walk, two on
# the north edge and two on the south. 68 x 34 cells, followed by the
# camera, under a clear sky, a golden dusk or rain on the glass.
#
# The bong lives in the Palm House in the middle: a cross of glass panes on
# iron ribs - a hall with a wing either side - with a door four cells wide
# on every side. Glass stops a hull and blinds a sight line but breaks at
# the second shot, so the house is brittle cover the fight takes apart pane
# by pane while its iron ribs stand. Under the glass gravel walks run from
# the doors to the bong's ring of sandbags, a tesla on two of its corners,
# through palms and ferns a hull hides in; gun towers stand on the hall's
# four corners, a bio tower inside each wing's door, and sandbag nests flank
# every door outside, a tower in four of them - teslas at the north and
# south doors, a gun tower at the west, a bio tower at the east: twelve
# towers in all. The players start in the hall beside the sonic hammer
# (shatter the glass, shove a tank back out of a door) and the flamethrower,
# with a tower pack and a bong pack by the north wall and health, a minigun,
# a shield and ammo in the wings.
#
# Round it the gardens: the parterre to the north - the great fountain on
# the north walk, bedding in stripes with yews behind, lamp posts on the
# terrace; the long canals and their bedding on the south lawn, a bong pack
# by the walk; the fern dell's pond and the broad walk's fountain in the
# west; in the east the alpine house (laser, speed-up) and a sundial on the
# walk among grey stone outcrops. Each corner holds a glass house with a
# stash in it and an enemy tower over it: the orchid house (laser,
# missiles) under a gun tower, the cactus house (minigun, plasma, shield)
# under a bio tower, the lily house round its pool (the second sonic
# hammer, missiles, a shield) with a gun tower at its door, and the brick
# boiler house (tower pack, minigun) with a tesla inside its door, its
# potting shed holding the second flamethrower. A bio tower in the west and
# a gun tower in the east cover the broad walk's gates, and four more hold
# the ways in from the north and south gates - a tesla and a gun tower
# north, a gun and a bio tower south: ten enemy towers in all. Oil drums
# stand beside the towers and on every way in from a gate, and the boiler
# house's fuel store is wired by an oil trail past the compost to the drums
# on the south-west gate's way in. Six plasma crates are spread round the
# garden - the orchid, cactus and lily houses, the north parterre, the fern
# dell and the south-west lawn - and twelve health packs.
"""


def write(path):
    lines = [HEADER, "version = 1",
             'bong = "oggle"',
             'tank = "warden"',
             'tank2 = "glacier"',
             'weather = ["clear", "dusk", "rain"]',
             "size = [68, 34]",
             'mission.kind = "protect"',
             'spawn.kind = "waves"',
             "spawn.waves = 4",
             "spawn.size = 4",
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
    sc, sr = START
    for first in FIRST_HERE:
        assert any(k == first and abs(c - sc) <= SIGHT_BOX[0] and abs(r - sr) <= SIGHT_BOX[1] for c, r, k in pickups), \
            f"no {first} crate inside the start's sight box"
    assert not pockets(), f"open nav cells cut off from the playfield: {pockets()}"
    reach = nav_reach()
    near = lambda p, rr: any((p[0] + dc, p[1] + dr) in rr for dc in range(-4, 4) for dr in range(-4, 4))
    for p in (START, START2, BONG):
        assert near(p, reach), p
    for c, r, kind in pickups:
        assert reachable((c, r), reach), f"{kind} at {c},{r} cannot be reached"
    for gc, gr in GATES:
        assert all(nav_open(*n) for n in lane(gc, gr)), f"gate {gc},{gr}"
        assert lane(gc, gr)[-1] in reach, f"gate {gc},{gr} is cut off"


ASCII = {
    'wall': '#', 'road': '=', 'water': '~', 'bong': 'B', 'start': '1', 'start2': '2',
    'pickup': 'P', 'sandbag': 's', 'fence': 'f', 'tree': 'T', 'pine': 'A', 'spruce': 'A', 'scots': 'A',
    'fir': 'a', 'palm': 'Y', 'snag': 'x', 'willow': 'W', 'birch': 'b', 'tall_grass': '"', 'bush': 'o',
    'berry_bush': 'o', 'autumn_bush': 'o', 'juniper': 'j', 'fern': ',', 'reeds': '|', 'gate': 'G',
    'tesla': 'Z', 'gun_tower': 'K', 'bio_slush': 'Q', 'barrel': 'D', 'hay': 'h', 'tyres': 'O',
    'wood_crate': 'X', 'barrier': 'n', 'tank_trap': '*', 'lamp': 'l', 'oil': ':',
}


def ascii_map():
    reach = nav_reach()
    print("    " + "".join(str(c // 10 % 10) for c in range(W)))
    print("    " + "".join(str(c % 10) for c in range(W)))
    for r in range(H):
        row = []
        for c in range(W):
            t = cells.get((c, r))
            k = kind_at((c, r))
            ch = ASCII.get(k, '?') if k else '.'
            if k == 'wall':
                ch = {'brick': '#', 'iron': 'I', 'wood': 'w', 'concrete': 'C', 'glass': 'g'}[t.split('"')[3]]
            if k in TOWERS and 'enemy' in t:
                ch = ch.lower()
            if k == 'water' and is_deep(c, r):
                ch = '%'
            if k is None and not any(n in reach for n in nav_of((c, r))):
                ch = '!'
            row.append(ch)
        print(f"{r:3d} " + "".join(row))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("-o", "--out", default="maps/glasshouses.toml")
    ap.add_argument("--stats", action="store_true")
    ap.add_argument("--ascii", action="store_true")
    a = ap.parse_args()
    designed, filled, narrow = build()
    if designed:
        print(f"set pieces leave {len(designed)} open nav cells cut off: {designed}")
    left = narrows()
    if left:
        print(f"{len(left)} one-cell passages: {left}")
    if a.ascii:
        ascii_map()
    check()
    write(a.out)
    if a.stats:
        kinds = {}
        for t in cells.values():
            k = t.split('"')[1]
            if k == "pickup":
                k = "pickup:" + t.split('"')[3]
            kinds[k] = kinds.get(k, 0) + 1
        print(f"{len(cells)} cells, {filled} pocket cells filled, {narrow} narrow trail cells grown")
        for c, r, kind in pickups:
            print(f"  pickup {kind:13} {c},{r}")
        for k, n in sorted(kinds.items(), key=lambda kv: (-kv[1], kv[0])):
            print(f"  {k:24} {n}")
        cost = walk_costs()
        for gc, gr in GATES:
            inside = lane(gc, gr)[-1]
            c = cost.get(inside)
            print(f"  gate {gc},{gr}: walk {c} steps" if c else f"  gate {gc},{gr}: no walk")


if __name__ == "__main__":
    main()
