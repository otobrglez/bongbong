"""Write maps/castle-moat.toml: Castle Moat, level 15 - a siege of a moated
castle, the bong in its keep, 97 x 54 cells, laid out set piece by set
piece.

  python3 tools/gen_castle_moat.py [-o maps/castle-moat.toml] [--stats] [--ascii]

This script is the source: change it and run it again rather than editing
the TOML (a re-save from the map editor would drop the header too). It draws
no random numbers: every choice is hashed from the cell's position with a
fixed salt, so a rerun writes the same file byte for byte. `--ascii` prints
the map a character a cell, `--stats` the counts, each gate's walk from the
start and every pickup.

What it lays out (the header written into the map says how it plays):

- the castle on its island in the middle: a moat three cells wide all
  round it, deep in the middle; the curtain wall on the island's edge with
  a gun tower on every corner and on the long walls, four gatehouses in a
  pinwheel - a drawbridge north, east and south (two plank decks with a
  ford between them), the stone causeway of the main gate west, behind its
  barbican;
- inside the curtain the outer bailey, split by two cross walls into a
  ward for each of the keep's gates - the stables and the granary, the
  barracks and the armoury, the chapel and its herb garden, the forge,
  the archery butts - and in the middle the keep, its own wall with two
  gates, neither in line with an outer one, round the bong;
- the outer road all round the castle, and on it the four quarters the
  waves come from: the mill and its orchard north-west, the besiegers'
  camp north-east, the siege park south-east and the village west and
  south-west, with the north battery, the tiltyard and the hunting woods
  between them.
"""

import argparse
import heapq
import math
from collections import deque

W, H = 97, 54

# (col, row) -> inline TOML table text
cells = {}
# cells the planting pass must leave alone
reserved = set()
water = set()
roads = set()

# the moat's outer edge and the island inside it; the curtain wall stands
# on the island's edge, so the moat laps at its foot
MOAT = (23, 9, 73, 44)
ISLAND = (26, 12, 70, 41)
# the keep's wall; the inner ward is inside it
KEEP = (38, 19, 58, 34)
BONG = (48, 27)
# the seats either side of the bong on its row: each one's sight box (an
# enemy fires at a seat only from inside it) stays inside the curtain,
# never out over the moat, where a tank on the far bank would wait for a
# way in that is not there
START = (43, 27)
START2 = (53, 27)

# the four ways across the moat, in a pinwheel: each crossing's cells
# (the three it is wide), and the curtain's gate behind it
N_WAY = (32, 33, 34)   # columns, over the north arm
E_WAY = (16, 17, 18)   # rows, over the east arm
S_WAY = (62, 63, 64)   # columns, over the south arm
W_WAY = (35, 36, 37)   # rows, over the west arm (the main gate)
# the keep's two gates, neither in line with an outer gate
KEEP_N = (52, 53, 54)
KEEP_S = (42, 43, 44)

# the outer road round the castle: two cells wide
NORTH_ROAD = (6, 7)
SOUTH_ROAD = (47, 48)
WEST_ROAD = (11, 12)   # the village street
EAST_ROAD = (85, 86)

# The gates, two by every corner, each 45 cells and more from the bong:
# a wave tank's leash (`enemy_leash_px`, 20 cells) round its gate then
# stays outside the 800 px round the bong where the lull is counted.
GATES = [
    (11, 0), (0, 10),     # the mill, north-west
    (85, 0), (96, 16),    # the besiegers' camp, north-east
    (96, 44), (85, 53),   # the siege park, south-east
    (0, 36), (11, 53),    # the village, west and south-west
]
GATE_LANES = set()
# the roads as laid, before the set pieces: none of them may build on one
ROAD_PLAN = set()

# a weapon met first in the campaign here: none (docs/levels.md)
FIRST_HERE = set()
# a seat's sight box, half extents in cells (`sight_box_half_cols`/`_rows`)
SIGHT = (11.5, 7.5)
# what an enemy turns off its road to fetch lies beyond every gate's leash
# (`enemy_leash_px`, 640 px = 20 cells): the weapons it carries, speed,
# shield, the bong pack it takes to deny the players and the tower pack.
# The flamethrower, the grenades and the air strikes are the players'
# alone, so those may lie anywhere.
FETCHED = {
    "laser", "minigun", "plasma", "missiles", "sonic_hammer", "emp_burst", "gauss_rail", "fpv_swarm",
    "rod_from_god", "gravity_well", "speedup", "shield", "bong_health", "tower_pack",
}
# a blast weapon's crate keeps 256 px (eight cells) and more from the
# players' bong (docs/levels.md: two of the grenade's blast radii)
BLAST = {"grenades", "missiles", "fpv_swarm", "air_jet", "air_heli", "gravity_well"}
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


# the set pieces' open ground, which the planting keeps off
OPEN_GROUND = set()


def clear_rect(c0, r0, c1, r1):
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
    assert kind_at((c, r)) is None or kind_at((c, r)) in TREE_KINDS | COVER_KINDS, \
        f"{kind} at {c},{r} would cover a {kind_at((c, r))}"
    put(c, r, "pickup", pickup=kind)
    pickups.append((c, r, kind))


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


def road(points, width=2):
    """A road `width` cells across along the rows and columns."""
    path = polyline(points)
    for c, r in path:
        for dc in range(width):
            for dr in range(width):
                if inb(c + dc, r + dr):
                    put(c + dc, r + dr, "road")
    return path


def bridge_decks(was_water):
    """A road two cells wide over a stream leaves no road cell with water
    on both sides, which is what the look draws as a plank deck
    (`ground::GroundGrid::bridge`): give the stream back the second row,
    so the first is a deck over it. The cell given back is a ford again."""
    for c, r in sorted(was_water):
        if (c, r) not in roads:
            continue
        if ((c, r - 1) in water and (c, r + 1) in water) or ((c - 1, r) in water and (c + 1, r) in water):
            continue
        for n in ((c, r + 1), (c + 1, r)):
            if n in roads and n in was_water:
                put(*n, "water")
                break


def stream(points):
    """A stream one cell wide: a ford its whole length."""
    for p in polyline(points):
        if inb(*p) and p not in roads:
            put(*p, "water")


# ---------------------------------------------------------------- the water


def moat():
    """The moat: three cells wide all round the island, so its middle row
    is open water - a wall to hulls, nothing to shots - and the rows
    either side of it, against the curtain and the bank, are fords too
    narrow for a hull. The mill pond north-west, the millrace from it into
    the moat's west arm, and the moat's overflow off the south edge."""
    for c, r in rect(*MOAT):
        if not (ISLAND[0] <= c <= ISLAND[2] and ISLAND[1] <= r <= ISLAND[3]):
            put(c, r, "water")
    # the mill pond, fed by a brook off the north edge
    for c, r in rect(15, 1, 20, 4):
        if (c, r) in ((15, 1), (20, 1), (15, 4), (20, 4)):
            continue
        put(c, r, "water")
    stream([(18, 0), (18, 1)])
    # the millrace, from the pond's foot round to the moat
    stream([(17, 5), (17, 10), (22, 10)])
    # the overflow, from the south arm's west end off the south edge
    stream([(29, 45), (29, H - 1)])


def is_deep(c, r):
    """Open water: a water cell whose eight neighbours are all water, the
    map's edge reading as the cell beside it (`ground::Layout` clamps)."""
    clamp = lambda c2, r2: (min(max(c2, 0), W - 1), min(max(r2, 0), H - 1))
    return all(clamp(c + dc, r + dr) in water for dc in (-1, 0, 1) for dr in (-1, 0, 1))


def crossings():
    """The ways over the moat. North, east and south a drawbridge: two
    plank decks with the moat left a ford between them - a road cell with
    water either side is what the look draws as a deck - so the crossing
    is three cells wide and the water beside it, no longer open, a ford
    too. West the main gate's stone causeway, three cells of road."""
    for r in range(MOAT[1], ISLAND[1]):
        for c in (N_WAY[0], N_WAY[2]):
            put(c, r, "road")
    for c in range(ISLAND[2] + 1, MOAT[2] + 1):
        for r in (E_WAY[0], E_WAY[2]):
            put(c, r, "road")
    for r in range(ISLAND[3] + 1, MOAT[3] + 1):
        for c in (S_WAY[0], S_WAY[2]):
            put(c, r, "road")
    for c in range(MOAT[0], ISLAND[0]):
        for r in W_WAY:
            put(c, r, "road")


# ---------------------------------------------------------------- the castle


def curtain():
    """The curtain wall on the island's edge: dressed stone, a gun tower
    on each corner bastion and one on each long wall, the gatehouses'
    towers in grey ashlar either side of each gate, built out a cell into
    the bailey."""
    c0, r0, c1, r1 = ISLAND
    clear_rect(c0 + 1, r0 + 1, c1 - 1, r1 - 1)
    gates = {(c, r0) for c in N_WAY} | {(c1, r) for r in E_WAY} | {(c, r1) for c in S_WAY} | {(c0, r) for r in W_WAY}
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) in gates:
            continue
        wall(c, r, "brick")
    # the corner bastions: grey stone, the tower on the corner itself
    for cc, rr, dc, dr in ((c0, r0, 1, 1), (c1, r0, -1, 1), (c0, r1, 1, -1), (c1, r1, -1, -1)):
        for c, r in ((cc + dc, rr), (cc, rr + dr), (cc + dc, rr + dr)):
            wall(c, r, "concrete")
        put(cc, rr, "gun_tower")
    # a gun tower on each long wall, over the keep's gate below it
    put(58, r0, "gun_tower")
    put(38, r1, "gun_tower")
    for c in (57, 59):
        wall(c, r0, "concrete")
    for c in (37, 39):
        wall(c, r1, "concrete")
    # the gatehouses: a tower either side of each gate, two cells deep
    for c in (N_WAY[0] - 1, N_WAY[2] + 1):
        for r in (r0, r0 + 1):
            wall(c, r, "concrete")
    for c in (S_WAY[0] - 1, S_WAY[2] + 1):
        for r in (r1, r1 - 1):
            wall(c, r, "concrete")
    for r in (E_WAY[0] - 1, E_WAY[2] + 1):
        for c in (c1, c1 - 1):
            wall(c, r, "concrete")
    for r in (W_WAY[0] - 1, W_WAY[2] + 1):
        for c in (c0, c0 + 1):
            wall(c, r, "concrete")
    # a tesla inside the north and the south gate, beside the way in (the
    # barbican's towers keep the main gate, the corner bastions the east);
    # a brazier inside every gate
    put(N_WAY[2] + 3, r0 + 2, "tesla")
    put(S_WAY[0] - 3, r1 - 2, "tesla")
    for c, r in ((N_WAY[0] - 2, r0 + 1), (c1 - 1, E_WAY[0] - 2), (S_WAY[2] + 2, r1 - 1), (c0 + 2, W_WAY[2] + 1)):
        put(c, r, "lamp")


def keep():
    """The keep in the middle of the bailey: its wall of grey ashlar with
    a gun tower on two corners, a gate north and a gate south - the north
    one east of the bong, the south one west of it, so whoever comes in
    turns under the towers to face it. Inside, the bong in a ring of
    sandbags open only north and south, the seats either side of it, a
    tesla over each gate, braziers
    in the corners and the stores along the walls, the blast weapons in
    the corners eight cells and more from the bong."""
    c0, r0, c1, r1 = KEEP
    gates = {(c, r0) for c in KEEP_N} | {(c, r1) for c in KEEP_S}
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) in gates:
            continue
        wall(c, r, "concrete")
    # the gate posts: dressed stone like the curtain
    for c in (KEEP_N[0] - 1, KEEP_N[2] + 1):
        wall(c, r0, "brick")
    for c in (KEEP_S[0] - 1, KEEP_S[2] + 1):
        wall(c, r1, "brick")
    put(c0, r0, "gun_tower")
    put(c1, r1, "gun_tower")
    # the teslas inside the gates: their reach (3.5 cells) takes in each
    # gate and the ground just outside it, and no other wall
    put(KEEP_N[0] - 1, r0 + 3, "tesla")
    put(KEEP_S[2] + 1, r1 - 3, "tesla")
    bc, br = BONG
    put(bc, br, "bong")
    put(*START, "start")
    put(*START2, "start2")
    for c, r in outline(bc - 2, br - 2, bc + 2, br + 2):
        if c == bc:
            continue
        put(c, r, "sandbag")
    for c, r in ((c0 + 1, r0 + 1), (c1 - 1, r0 + 1), (c0 + 1, r1 - 1), (c1 - 1, r1 - 1)):
        put(c, r, "lamp")
    # flower beds along the north and south walls
    for c, r in ((45, r0 + 1), (46, r0 + 1), (50, r0 + 1), (49, r0 + 1), (46, r1 - 1), (47, r1 - 1),
                 (50, r1 - 1), (51, r1 - 1)):
        put(c, r, pick(GARDEN, hash01(c, r, 3)))
    # the stores: the blast weapons and the air strikes in the corners
    pickup(c0 + 2, r0 + 1, "grenades")
    pickup(c1 - 2, r0 + 1, "air_jet")
    pickup(c0 + 2, r1 - 1, "air_heli")
    pickup(c1 - 2, r1 - 1, "missiles")
    # by the seats: the bong's packs, health, ammo, a shield, a tower pack
    pickup(bc - 4, br - 3, "bong_health")
    pickup(bc + 4, br + 3, "bong_health")
    pickup(START[0] - 1, br - 3, "health")
    pickup(START2[0] + 1, br + 3, "health")
    pickup(START[0] - 1, br + 3, "ammo")
    pickup(START2[0] + 1, br - 3, "ammo")
    pickup(bc - 4, br + 3, "shield")
    pickup(bc + 4, br - 3, "tower_pack")
    # plasma and machine guns at hand
    pickup(c0 + 2, br, "plasma")
    pickup(c1 - 2, br, "minigun")
    # the gravity well by the north-west brazier, nine cells from the bong
    pickup(c0 + 3, r0 + 2, "gravity_well")


def bailey():
    """The outer bailey between the curtain and the keep: the stables and
    the granary against the north wall, the barracks and its armoury west,
    the chapel and the herb garden east, the forge against the south wall,
    the archery butts and the well against the north wall, and the two
    cross walls that make it two wards. Every building stands against the
    curtain, so the lanes round the keep stay four cells wide and more."""
    c0, r0, c1, r1 = ISLAND
    # the stables: timber stalls along the north wall, hay in front
    for c, r in rect(39, r0 + 1, 47, r0 + 2):
        wall(c, r, "wood" if r == r0 + 1 or c in (39, 47) else ("glass" if c in (41, 45) else "wood"))
    for c in (40, 43, 46):
        put(c, r0 + 3, "hay")
    pickup(44, r0 + 3, "speedup")
    # the north ward's stores by the gate: a health pack, a minigun
    pickup(39, r0 + 4, "health")
    pickup(c0 + 4, r0 + 4, "minigun")
    pickup(41, r0 + 3, "ammo")
    # the granary: brick with a timber roof, east of the stables
    for c, r in rect(61, r0 + 1, 66, r0 + 2):
        wall(c, r, "brick" if c in (61, 66) or r == r0 + 1 else "wood")
    pickup(63, r0 + 3, "health")
    pickup(64, r0 + 3, "missiles")
    # the archery butts: target boards against the north wall between the
    # stables and the well
    for c in (49, 51):
        put(c, r0 + 1, "target")
    # the barracks: a long brick hall against the west wall, glass windows
    # to the bailey, a timber roof; the armoury at its south end, hollow,
    # its door to the bailey
    for c, r in rect(c0 + 1, 17, c0 + 4, 26):
        edge = c in (c0 + 1, c0 + 4) or r in (17, 26)
        window = c == c0 + 4 and r % 2 == 0 and 17 < r < 26
        chimney = not edge and r in (19, 24) and c == c0 + 2
        wall(c, r, "glass" if window else ("brick" if edge else ("concrete" if chimney else "wood")))
    for c, r in outline(c0 + 1, 28, c0 + 5, 33):
        if c == c0 + 5 and r in (30, 31):
            continue
        wall(c, r, "brick")
    for c, r in rect(c0 + 2, 29, c0 + 4, 32):
        clear(c, r)
        OPEN_GROUND.add((c, r))
    pickup(c0 + 2, 29, "plasma")
    pickup(c0 + 2, 32, "minigun")
    pickup(c0 + 4, 29, "laser")
    pickup(c0 + 4, 32, "ammo")
    # the chapel: brick with stained glass down both sides, its door west
    # to the bailey; the infirmary's stores inside
    for c, r in outline(63, 22, c1 - 1, 31):
        if c == 63 and r in (26, 27):
            continue
        window = (c in (63, c1 - 1) and r % 2 == 1 and 22 < r < 31) or (r in (22, 31) and c in (65, 67))
        wall(c, r, "glass" if window else "brick")
    for c, r in rect(64, 23, c1 - 2, 30):
        clear(c, r)
        OPEN_GROUND.add((c, r))
    pickup(65, 24, "health")
    pickup(67, 24, "bong_health")
    pickup(65, 29, "health")
    pickup(67, 29, "ammo")
    # the bailey's corners: a flower bed by the north-west bastion, hay
    # and crates stacked against the south wall
    for c, r in rect(c0 + 2, r0 + 2, c0 + 4, r0 + 3):
        if hash01(c, r, 17) < 0.8:
            put(c, r, pick(GARDEN, hash01(c, r, 19)))
    for c, r in ((c0 + 4, r1 - 1), (c0 + 5, r1 - 1), (c0 + 8, r1 - 1)):
        put(c, r, "hay")
    for c, r in ((c0 + 9, r1 - 1), (c0 + 10, r1 - 1), (44, r1 - 1)):
        put(c, r, "wood_crate")
    pickup(c0 + 6, r1 - 2, "plasma")
    # the cross walls that split the bailey in two wards, one from the
    # keep to the chapel, one from the keep past the barracks to the west
    # wall: the north and east gates lead to the keep's north gate, the
    # west and south ones to its south gate, and no tank has two ways to
    # weigh against each other
    for c, r in rect(KEEP[2] + 1, 22, 62, 23):
        wall(c, r, "brick")
    # the stair tower against the keep's north-east corner, on the cross
    # wall: it leaves no corner there for a tank to circle in
    for c, r in rect(KEEP[2] + 1, KEEP[1], 62, 21):
        wall(c, r, "concrete" if c in (KEEP[2] + 1, 62) and r in (KEEP[1], 21) else "brick")
    for c, r in rect(c0 + 1, 27, KEEP[0] - 1, 27):
        wall(c, r, "brick")
    for c, r in rect(c0 + 6, 28, KEEP[0] - 1, 28):
        wall(c, r, "brick")
    # the herb garden north of the chapel and the orchard trees south
    for c, r in rect(64, 20, 68, 21):
        if hash01(c, r, 7) < 0.7:
            put(c, r, pick(GARDEN, hash01(c, r, 9)))
    # the kitchen garden in the south-east corner
    for c, r in rect(66, 34, 69, 38):
        if hash01(c, r, 11) < 0.6:
            put(c, r, pick(GARDEN, hash01(c, r, 13)))
    # the forge against the south wall: brick, an iron anvil block, coal
    # and oil drums by the wall, its stores in front
    for c, r in rect(47, r1 - 2, 55, r1 - 1):
        wall(c, r, "iron" if c in (50, 51) and r == r1 - 2 else ("brick" if r == r1 - 2 or c in (47, 55) else "wood"))
    pickup(48, r1 - 3, "grenades")
    pickup(54, r1 - 3, "gauss_rail")
    pickup(51, r1 - 3, "ammo")
    # the well against the north wall between the stables and the
    # granary: a shallow basin, two by two
    for c, r in rect(54, r0 + 1, 55, r0 + 2):
        put(c, r, "water")
    put_free(56, r0 + 1, "reeds")


# ---------------------------------------------------------------- outside


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


def tent(c0, r0, w, h, salt):
    """A tent: canvas (timber's orange) on a frame, its ridge glass now
    and then - a lantern inside showing through."""
    for c, r in rect(c0, r0, c0 + w - 1, r0 + h - 1):
        ridge = (h >= 3 and r == r0 + h // 2) or (h < 3 and c == c0 + w // 2)
        wall(c, r, "glass" if ridge and hash01(c, r, salt) < 0.3 else "wood")


def trebuchet(c0, r0, across=False):
    """A trebuchet: a timber frame three by two, its iron counterweight
    hung off one end."""
    if across:
        for c, r in rect(c0, r0, c0 + 1, r0 + 2):
            wall(c, r, "wood")
        wall(c0, r0 + 3, "iron")
    else:
        for c, r in rect(c0, r0, c0 + 2, r0 + 1):
            wall(c, r, "wood")
        wall(c0 + 3, r0, "iron")


def barbican():
    """The barbican before the main gate, on the moat's far bank: a walled
    forecourt round the causeway's head, open west to the village road,
    a friendly gun tower on its north-west corner, and the stores a
    sortie takes with it."""
    c0, r0, c1, r1 = 17, 33, 22, 43
    clear_rect(c0, r0, c1, r1)
    for c, r in outline(c0, r0, c1, r1):
        if c == c1 and r0 < r < r1:
            continue  # the moat side, open onto the causeway
        if c == c0 and r in W_WAY:
            continue  # the way in from the village road
        wall(c, r, "brick")
    for c, r in ((c0, W_WAY[0] - 1), (c0, W_WAY[2] + 1)):
        wall(c, r, "concrete")
    put(c0, r0, "gun_tower")
    wall(c0, r1, "concrete")
    pickup(19, 35, "health")
    pickup(19, 41, "ammo")
    pickup(21, 35, "grenades")
    pickup(21, 41, "flamethrower")


def mill():
    """The mill on its pond in the north-west: the mill house of timber
    and brick beside the race, its iron wheel in the water's edge, sacks
    of flour (hay) at the door, and the miller's stores inside; an orchard
    of fruit trees in rows south of it, tall grass between."""
    # the mill house on the pond's east bank, its door south onto the road
    for c, r in outline(21, 1, 24, 4):
        if r == 4 and c in (22, 23):
            continue
        wall(c, r, "wood" if r == 1 else "brick")
    for c, r in rect(22, 2, 23, 3):
        clear(c, r)
        OPEN_GROUND.add((c, r))
    keep_open(21, 5, 24, 5)
    wall(21, 2, "iron")  # the wheel, in the pond's edge
    pickup(22, 2, "health")
    pickup(23, 2, "flamethrower")
    for c, r in ((25, 2), (25, 3)):
        put(c, r, "hay")
    # the besiegers hold the mill: a bio slush on the race's bank
    put(14, 8, "bio_slush", side="enemy")
    # the orchard: rows of fruit trees, every other row, tall grass in
    # the rows between
    for c, r in rect(1, 13, 9, 21):
        if c in (1, 4, 7) and r in (13, 16, 19):
            put(c, r, "tree")
        elif hash01(c, r, 77) < 0.35:
            put(c, r, "tall_grass")
    keep_open(1, 13, 9, 21)
    for c, r in rect(14, 13, 20, 19):
        if c in (14, 17, 20) and r in (14, 17):
            put(c, r, "tree" if hash01(c, r, 5) < 0.6 else "birch")
        elif hash01(c, r, 78) < 0.3:
            put(c, r, "tall_grass")
    keep_open(14, 13, 20, 19)


def village():
    """The village west and south-west of the castle, on its street and the
    road to the main gate: the church and its churchyard, the inn, the
    smithy with its oil drums, the market square at the crossroads under
    an enemy gun tower the besiegers set up there, and terraces of houses
    with gardens behind them."""
    st0, st1 = WEST_ROAD
    # the market square at the crossroads, west of the barbican
    keep_open(13, 34, 16, 43)
    keep_open(0, 34, st0 - 1, 36)
    put(14, 35, "gun_tower", side="enemy")
    for c, r in ((14, 34), (15, 34), (13, 35)):
        put(c, r, "sandbag")
    for c, r in ((15, 42), (16, 42), (13, 42)):
        put(c, r, "hay" if c != 16 else "wood_crate")
    pickup(15, 41, "health")
    put(16, 39, "lamp")
    # the church west of the street, its door to the street; the
    # churchyard behind it
    for c, r in rect(2, 24, 9, 30):
        edge = c in (2, 9) or r in (24, 30)
        door = c == 9 and r in (26, 27, 28)
        if door:
            clear(c, r)
            OPEN_GROUND.add((c, r))
            continue
        window = (r in (24, 30) and c in (4, 7)) or (c == 2 and r == 27)
        wall(c, r, "glass" if window else ("brick" if edge else "wood"))
    for c, r in rect(3, 25, 8, 29):
        clear(c, r)
        OPEN_GROUND.add((c, r))
    keep_open(10, 26, 10, 28)
    pickup(4, 26, "grenades")
    pickup(4, 28, "health")
    pickup(7, 27, "ammo")
    for c, r in ((3, 22), (5, 22), (7, 22), (3, 32), (5, 32), (7, 32)):
        put(c, r, "barrier")  # gravestones
    put(1, 23, "willow")
    put(9, 32, "willow")
    # the inn east of the street: timber and brick, its yard open to the
    # street, the stores inside
    for c, r in outline(14, 21, 20, 26):
        if c == 14 and r in (23, 24):
            continue
        wall(c, r, "wood" if r == 21 else ("glass" if r == 26 and c in (16, 18) else "brick"))
    for c, r in rect(15, 22, 19, 25):
        clear(c, r)
        OPEN_GROUND.add((c, r))
    pickup(16, 22, "health")
    pickup(18, 25, "ammo")
    pickup(19, 22, "air_heli")
    # the smithy south of the square, its forge and its oil drums
    for c, r in outline(2, 40, 7, 44):
        if c == 7 and r in (41, 42):
            continue
        wall(c, r, "brick" if r != 40 else "wood")
    for c, r in rect(3, 41, 6, 43):
        clear(c, r)
        OPEN_GROUND.add((c, r))
    wall(5, 43, "iron")
    pickup(3, 41, "flamethrower")
    pickup(3, 43, "ammo")
    keep_open(1, 45, 9, 46)
    drum_cluster(((3, 46), (4, 46)), "oil")
    # terraces along the street
    for c0, r0, c1, r1, salt in ((14, 44, 20, 45, 13), (1, 49, 9, 51, 17),
                                 (14, 49, 20, 51, 19), (14, 28, 20, 31, 23)):
        terrace(c0, r0, c1, r1, salt)


HOUSE_STYLES = (("brick", "wood", 55), ("wood", "brick", 30), ("brick", "glass", 15))


def house(c0, r0, c1, r1, salt):
    """A house, solid: its walls in one material with a glass window or
    two, its roof in another, a stone chimney now and then."""
    u = hash01(c0, r0, salt)
    acc = 0
    for walls, roof, w in HOUSE_STYLES:
        acc += w / 100
        if u < acc:
            break
    for c, r in rect(c0, r0, c1, r1):
        edge = c in (c0, c1) or r in (r0, r1)
        if edge:
            window = r == r1 and c0 < c < c1 and (c - c0) % 2 == 1
            wall(c, r, "glass" if window else walls)
        else:
            wall(c, r, roof)


def terrace(c0, r0, c1, r1, salt):
    """A row of houses, widths of three to five, a yard two wide between
    two of them now and then."""
    c = c0
    i = 0
    while c <= c1:
        w = 3 + int(hash01(c, salt, 3) * 3)
        if c + w - 1 > c1 or c1 - (c + w - 1) < 3:
            w = c1 - c + 1
        house(c, r0, c + w - 1, r1, salt + i)
        c += w
        i += 1
        if c <= c1 - 5 and hash01(c, salt, 7) < 0.5:
            keep_open(c, r0, c + 1, r1)
            c += 2


def north_battery():
    """The besiegers' north battery across the north road: two trebuchets
    in sandbag pits trained on the curtain, an enemy gun tower between
    them, a stack of powder (fuel drums) behind, and the stores the crews
    left: the EMP, the sonic hammer, plasma, missiles and a health pack."""
    clear_rect(40, 0, 62, 5)
    trebuchet(43, 2)
    trebuchet(56, 2)
    put(51, 3, "gun_tower", side="enemy")
    for c, r in ((50, 4), (52, 4), (50, 2), (52, 2)):
        put(c, r, "sandbag")
    for c, r in ((42, 4), (47, 4), (55, 4), (60, 4)):
        put(c, r, "sandbag")
    drum_cluster(((48, 1), (49, 1), (54, 1)), "fuel")
    oil_trail([(50, 1), (53, 1)])
    pickup(41, 1, "emp_burst")
    pickup(61, 1, "sonic_hammer")
    pickup(46, 4, "plasma")
    pickup(59, 4, "health")
    pickup(53, 3, "missiles")


def besiegers_camp():
    """The besiegers' camp in the north-east, where two gates open: a
    palisade of stakes round it, gaps for the roads, the tents in rows,
    the commander's pavilion, a siege tower being built, the powder store
    of fuel and oil drums in a sandbag ring wired by an oil trail, an enemy
    gun tower at the gap onto the outer road and a tesla by the powder."""
    c0, r0, c1, r1 = 76, 0, 96, 12
    clear_rect(c0, r0, c1, r1)
    # the palisade along its south and west sides
    for c in range(c0, c1 + 1):
        if c in EAST_ROAD or c in (EAST_ROAD[0] - 1, EAST_ROAD[1] + 1):
            continue
        put(c, r1, "fence")
    for r in range(r0 + 2, r1):
        if r in NORTH_ROAD or r in (NORTH_ROAD[0] - 1, NORTH_ROAD[1] + 1):
            continue
        put(c0, r, "fence")
    tent(78, 1, 3, 2, 31)
    tent(78, 9, 3, 2, 33)
    tent(90, 1, 3, 2, 35)
    tent(88, 9, 2, 3, 37)
    # the commander's pavilion: bigger, its glass lantern in the middle
    for c, r in rect(90, 4, 94, 6):
        wall(c, r, "glass" if (c, r) == (92, 5) else "wood")
    # the siege tower, half built: timber with an iron frame
    for c, r in rect(81, 2, 83, 4):
        wall(c, r, "iron" if c in (81, 83) and r in (2, 4) else "wood")
    tent(77, 4, 2, 2, 39)
    tent(93, 1, 2, 2, 45)
    for c, r in ((80, 11), (81, 11), (84, 2), (84, 4), (88, 7), (95, 6)):
        put(c, r, "wood_crate")
    for c, r in ((81, 9), (80, 8), (89, 1)):
        put(c, r, "hay")
    put(EAST_ROAD[0] - 2, r1 - 1, "gun_tower", side="enemy")
    for c, r in ((82, 11), (84, 11), (84, 10)):
        put(c, r, "sandbag")
    put(95, 3, "tesla", side="enemy")
    # the powder store: fuel drums in a sandbag ring, an oil trail to the
    # oil drums by the tents
    for c, r in ((92, 8), (95, 8), (92, 11), (95, 11)):
        put(c, r, "sandbag")
    drum_cluster(((93, 9), (94, 9), (94, 10)), "fuel")
    oil_trail([(91, 10), (93, 10)])
    drum_cluster(((91, 9),), "oil")
    pickup(79, 4, "grenades")
    pickup(88, 2, "health")
    pickup(82, 9, "ammo")
    put(87, 4, "lamp")
    put(77, 8, "lamp")


def siege_park():
    """The siege park in the south-east, where two more gates open: the
    battering ram and two more trebuchets waiting their turn, tents, the
    powder store in a sandbag ring with its oil trail, an enemy gun tower
    over the outer road, a bio slush by the tents, and a stash of air
    support and grenades in the quartermaster's tent."""
    c0, r0, c1, r1 = 76, 41, 96, 53
    clear_rect(c0, r0, c1, r1)
    for c in range(c0, c1 + 1):
        if c in EAST_ROAD or c in (EAST_ROAD[0] - 1, EAST_ROAD[1] + 1):
            continue
        put(c, r0, "fence")
    # the battering ram: an iron-shod timber beam under its roof
    for c, r in rect(78, 43, 81, 44):
        wall(c, r, "iron" if c == 78 else "wood")
    trebuchet(78, 50)
    trebuchet(92, 47)
    tent(88, 42, 3, 2, 41)
    tent(92, 42, 3, 2, 43)
    # the quartermaster's tent, hollow, its flap north
    for c, r in outline(87, 49, 91, 52):
        if r == 49 and c in (89, 90):
            continue
        wall(c, r, "wood")
    for c, r in rect(88, 50, 90, 51):
        clear(c, r)
        OPEN_GROUND.add((c, r))
    pickup(88, 50, "air_jet")
    pickup(90, 50, "grenades")
    pickup(89, 51, "flamethrower")
    put(EAST_ROAD[0] - 2, r0 + 2, "gun_tower", side="enemy")
    put(81, 46, "bio_slush", side="enemy")
    put(92, 52, "tesla", side="enemy")
    # the powder store in the far corner: fuel drums in a sandbag ring, an
    # oil trail from it to the trebuchet's oil
    for c, r in ((93, 50), (96, 50), (93, 53), (96, 53)):
        put(c, r, "sandbag")
    drum_cluster(((94, 51), (95, 51), (95, 52)), "fuel")
    oil_trail([(94, 53), (94, 52)])
    drum_cluster(((83, 51),), "oil")
    tent(77, 52, 3, 2, 47)
    for c, r in ((82, 43), (82, 44), (90, 47), (96, 46), (91, 46)):
        put(c, r, "wood_crate")
    for c, r in ((77, 49), (84, 52), (88, 53)):
        put(c, r, "hay")
    for c, r in ((82, 42), (84, 42), (84, 43)):
        put(c, r, "sandbag")
    pickup(84, 44, "health")
    pickup(77, 46, "ammo")
    put(84, 49, "lamp")


def hunting_woods():
    """The hunting woods east of the castle, between the camp and the
    siege park: pine and fir, a lodge in a clearing with a health pack
    and ammo, the woodcutters' log stacks by the road."""
    # the lodge, timber, its door to the clearing south
    for c, r in outline(79, 26, 83, 29):
        if r == 29 and c in (80, 81, 82):
            continue
        wall(c, r, "wood" if r == 26 else "brick")
    for c, r in rect(80, 27, 82, 28):
        clear(c, r)
        OPEN_GROUND.add((c, r))
    keep_open(77, 29, 84, 32)
    pickup(80, 27, "health")
    pickup(82, 27, "ammo")
    put(84, 31, "lamp")
    # log stacks by the east road
    for c, r in ((90, 18), (91, 18), (92, 18), (90, 37), (91, 37)):
        wall(c, r, "wood")


def tiltyard():
    """The tiltyard south of the castle across the south road: the lists
    fenced round, a barrier down the middle, the stands either side and
    the prizes laid out in them - a laser, the FPV swarm, a minigun, a
    health pack."""
    c0, r0, c1, r1 = 38, 49, 58, 53
    clear_rect(c0, r0, c1, r1)
    for c in range(c0, c1 + 1):
        if c in (41, 42, 43, 53, 54, 55):
            continue  # the ways in, at each end
        put(c, r0, "fence")
    for c in range(45, 52):
        put(c, 51, "fence")  # the tilt barrier
    # the stands: timber, at each end
    for c, r in rect(c0, r0 + 1, c0 + 1, r1):
        wall(c, r, "wood")
    for c, r in rect(c1 - 1, r0 + 1, c1, r1):
        wall(c, r, "wood")
    pickup(41, 52, "laser")
    pickup(55, 52, "fpv_swarm")
    pickup(43, 50, "minigun")
    pickup(53, 50, "health")


def glacis():
    """The ground swept by the walls' towers, out from the moat all round
    and three cells wide on the east bank before the woods: bare, so the
    bank is a way round the castle and nothing stands in it."""
    c0, r0, c1, r1 = MOAT
    for c, r in rect(c0 - 2, r0 - 1, c1 + 3, r1 + 2):
        if (c, r) in water or (c, r) in roads or kind_at((c, r)) is not None:
            continue
        reserved.add((c, r))
        OPEN_GROUND.add((c, r))


def siege_lines():
    """The besiegers' lines in the fields beyond the outer road, north and
    south: tank traps in a broken row, concrete barriers between them, and
    a stack of oil drums in each, well off every road."""
    for c in (28, 31, 34, 37, 65, 68, 71, 74):
        put_free(c, 3, "tank_trap")
    for c in (25, 33, 36, 61, 64, 67, 73):
        put_free(c, 51, "tank_trap")
    for c, r in ((29, 4), (66, 4), (72, 2), (32, 50), (60, 52), (70, 50)):
        put_free(c, r, "barrier")
    drum_cluster(((35, 1), (36, 1)), "oil")
    drum_cluster(((69, 52), (70, 52)), "oil")


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


GARDEN = (("berry_bush", 35), ("autumn_bush", 30), ("bush", 25), ("juniper", 10))
GARDEN_TREES = (("birch", 35), ("tree", 50), ("willow", 15))
WOODS = (("pine", 30), ("fir", 25), ("spruce", 20), ("scots", 15), ("snag", 5), ("birch", 5))
HEDGE = (("bush", 40), ("berry_bush", 25), ("autumn_bush", 25), ("juniper", 10))
BANK = (("willow", 70), ("birch", 30))


def zone(c, r):
    if c >= 77 and 14 <= r <= 39:
        return "woods"
    if c <= 22 and r >= 21:
        return "village"
    if c <= 22:
        return "mill"
    if c >= 76:
        return "camp"
    return "fields"


def planting():
    """Everything not a set piece: gardens behind the village's houses,
    the hunting woods' pine and fir, hedgerows and tall grass on the
    fields north and south, trampled ground round the camps, reeds along
    the water."""
    wd = distance_field(water, 3)
    for r in range(H):
        for c in range(W):
            p = (c, r)
            if p in cells or p in reserved:
                continue
            z = zone(c, r)
            dw = wd.get(p, 9)
            u = hash01(c, r, 13)
            v = hash01(c, r, 19)
            if dw == 1 and hash01(c, r, 85) < 0.45:
                put(c, r, "reeds")
            elif z == "woods":
                d = fbm(c / 4, r / 4, 37)
                if u < (0.55 if d > 0.42 else 0.12):
                    put(c, r, pick(WOODS, hash01(c, r, 41)))
                elif v < 0.3:
                    put(c, r, "fern" if v < 0.15 else "juniper")
                else:
                    continue
            elif z == "village":
                g = fbm(c / 3, r / 3, 17)
                if u < 0.08:
                    put(c, r, pick(GARDEN_TREES, hash01(c, r, 79)))
                elif g > 0.56 and v < 0.55:
                    put(c, r, pick(GARDEN, hash01(c, r, 23)))
                elif g < 0.35 and v < 0.3:
                    put(c, r, "tall_grass")
                else:
                    continue
            elif z == "mill":
                g = fbm(c / 3, r / 3, 27)
                if u < 0.06:
                    put(c, r, pick(GARDEN_TREES, hash01(c, r, 81)))
                elif g > 0.5 and v < 0.6:
                    put(c, r, "tall_grass")
                elif v < 0.1:
                    put(c, r, pick(HEDGE, hash01(c, r, 83)))
                else:
                    continue
            elif z == "camp":
                if v < 0.08:
                    put(c, r, "tall_grass")
                elif v > 0.97:
                    put(c, r, "juniper")
                else:
                    continue
            else:  # fields
                kind = field_plot(c, r)
                if kind is None:
                    continue
                put(c, r, kind)
            reserved.discard(p)


# the patchwork of the fields north and south of the castle: strips of
# plots seven cells wide, each one wheat, a flower meadow, a cabbage patch
# or fallow, a hedgerow down the edge between two of them
PLOTS = (("wheat", 40), ("meadow", 25), ("cabbages", 15), ("fallow", 20))


def field_plot(c, r):
    """What grows at (c, r) in the fields' patchwork, if anything."""
    i = (c - 2) // 7
    band = 0 if r < H // 2 else 1
    u = hash01(c, r, 31)
    if (c - 2) % 7 == 0:
        return pick(HEDGE, hash01(c, r, 33)) if u < 0.55 else None
    plot = pick(PLOTS, hash01(i, band, 35))
    if plot == "wheat":
        return "tall_grass" if u < 0.85 else None
    if plot == "meadow":
        if u < 0.22:
            return "berry_bush" if hash01(c, r, 37) < 0.5 else "autumn_bush"
        return "tall_grass" if u < 0.4 else None
    if plot == "cabbages":
        return "bush" if r % 2 == band and u < 0.7 else None
    if u < 0.04:
        return pick(GARDEN_TREES, hash01(c, r, 39))
    return "juniper" if u < 0.08 else None


# ---------------------------------------------------------------- the nav grid
#
# The game routes on a grid of 32 px cells whose centres sit half a cell off
# the map's (`pathfind::Grid`, `maplint`): nav cell (k, l) spans map cells k..k+1
# by l..l+1 and is open when none of them blocks. So a way the AI can take
# needs two empty map cells side by side.

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
    """Open ground the playfield never reaches grows a bush or a tree, so
    no nav cell is left boxed in or cut off."""
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
                    tree = {"woods": WOODS}.get(zone(*p), GARDEN_TREES)
                    put(*p, pick(tree, hash01(*p, 73)))
                    filled += 1
                    break
            else:
                wet = [p for p in corners if p in water]
                if wet:
                    wall(*wet[0], "concrete")
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


# ---------------------------------------------------------------- the walk


def walk_costs():
    """What the router charges from the start to every nav cell: a ford
    `water_ford_path_cost` (3), dry ground 1. A ford prices the nav cell
    its map cell's centre falls in (`Grid::weigh`)."""
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
            nd = d + (3 if n in water else 1)
            if nd < cost.get(n, 1 << 30):
                cost[n] = nd
                heapq.heappush(heap, (nd, n))
    return cost


# ---------------------------------------------------------------- the map


def build():
    moat()
    was_water = set(water)
    # the outer road round the castle, and the roads from it to the gates
    # and the crossings - straight reaches along the rows and columns, so
    # a hull is never made to stair-step
    road([(WEST_ROAD[0], 0), (WEST_ROAD[0], H - 2)])
    road([(EAST_ROAD[0], 0), (EAST_ROAD[0], H - 2)])
    road([(WEST_ROAD[0], NORTH_ROAD[0]), (EAST_ROAD[0], NORTH_ROAD[0])])
    road([(WEST_ROAD[0], SOUTH_ROAD[0]), (EAST_ROAD[0], SOUTH_ROAD[0])])
    road([(0, 10), (WEST_ROAD[0], 10)])
    road([(0, W_WAY[1]), (MOAT[0] - 1, W_WAY[1])])
    road([(EAST_ROAD[0], E_WAY[0]), (W - 2, E_WAY[0])])
    road([(MOAT[2] + 1, E_WAY[0]), (EAST_ROAD[0], E_WAY[0])])
    road([(EAST_ROAD[0], 44), (W - 2, 44)])
    road([(N_WAY[0], NORTH_ROAD[0]), (N_WAY[0], MOAT[1] - 1)])
    road([(N_WAY[1], NORTH_ROAD[0]), (N_WAY[1], MOAT[1] - 1)])
    road([(S_WAY[0], MOAT[3] + 1), (S_WAY[0], SOUTH_ROAD[0])])
    road([(S_WAY[1], MOAT[3] + 1), (S_WAY[1], SOUTH_ROAD[0])])
    bridge_decks(was_water)
    crossings()
    # the bailey's tracks, worn from each gate to the keep's nearer gate
    # and on into the inner ward
    road([(N_WAY[1], ISLAND[1]), (N_WAY[1], E_WAY[1])])
    road([(N_WAY[1], E_WAY[1]), (ISLAND[2] - 1, E_WAY[1])])
    road([(KEEP_N[1], E_WAY[1]), (KEEP_N[1], 22)])
    road([(S_WAY[0], W_WAY[0]), (S_WAY[0], ISLAND[3] - 1)])
    road([(ISLAND[0], W_WAY[0]), (S_WAY[0], W_WAY[0])])
    road([(KEEP_S[0], 31), (KEEP_S[0], W_WAY[0])])
    ROAD_PLAN.update(roads)

    curtain()
    keep()
    bailey()
    barbican()
    mill()
    village()
    north_battery()
    besiegers_camp()
    siege_park()
    hunting_woods()
    tiltyard()
    glacis()
    siege_lines()
    gate_aprons()
    for gc, gr in GATES:
        put(gc, gr, "gate")
        GATE_LANES.update(lane(gc, gr))
    planting()
    filled = fill_pockets()
    close_narrow_trails()
    return filled


HEADER = """\
# castle-moat - Castle Moat, level 15: a siege of a moated castle.
# Written by tools/gen_castle_moat.py - change the generator and run it
# again rather than editing this file; re-saving from the map editor would
# drop these comments. Level tables as dotted keys on purpose - a `[spawn]`
# header would swallow every `cells.` line after it.
#
# Protect on the waves plan: six waves of 4 to 9 tanks, medium to super, 39
# in all, rolling in at eight gates, two by every corner of the map - the
# mill north-west, the besiegers' camp north-east, the siege park south-east,
# the village west and south-west - each a walk of 10 to 15 s from the keep.
# The players hold it with a titan and a leviathan. 97 x 54 cells, followed
# by the camera, in the valley's look, under a sky the seed picks from
# clear, dusk, rain, storm and fog.
#
# The castle stands on an island in the middle, its moat three cells wide
# all round with open water down its middle - a wall to hulls, nothing to
# shots. Four ways cross it in a pinwheel: a drawbridge north, east and
# south (two plank decks with a ford between them), and the main gate's
# stone causeway west, behind a walled barbican with a gun tower. The
# curtain wall rises from the moat, a gun tower on every corner bastion and
# on each long wall, a tesla inside the north and the south gate. Two cross
# walls split the outer bailey into two wards: the north and east gates lead
# to the keep's north gate, the west and south gates to its south gate,
# neither in line with an outer one. In the wards the stables, the granary
# and the well against the north wall, the archery butts, the barracks and
# the armoury (plasma, a minigun, the laser) west, the chapel's infirmary
# (health, a bong health pack) east, the forge (grenades, the gauss rail)
# south, and missiles, a minigun and plasma by the gates.
#
# The keep in the middle is the players' base: a wall of grey ashlar with a
# gun tower on two corners and a tesla inside each gate, the bong in a ring
# of sandbags open only north and south, the seats either side of it,
# braziers in the corners and the stores - two bong health packs,
# health, ammunition, a shield, a tower pack, plasma, a minigun and, eight
# cells and more from the bong, the gravity well, grenades, missiles and a
# jet and a helicopter strike.
#
# The outer road runs round the castle and the besiegers hold the country:
# the mill on its pond (an enemy bio slush on the race); the north battery's
# trebuchets in sandbag pits under an enemy gun tower (the EMP, the sonic
# hammer, plasma, missiles); the camp north-east (tents, the pavilion, a
# siege tower, the powder store's fuel drums wired by an oil trail, an enemy
# gun tower and tesla); the siege park south-east (the battering ram,
# trebuchets, the quartermaster's tent with a jet strike, grenades and a
# flamethrower, the powder under an enemy tesla, a gun tower and a bio
# slush); and the village's market square under an enemy gun tower. The
# village keeps its church, its inn (a helicopter strike) and the smithy with
# its oil drums; the tiltyard south of the castle the laser, the FPV swarm and
# a minigun as prizes; the hunting woods east a lodge with health and ammo.
# Tank traps, barriers and stacks of drums lie in the siege lines across the
# patchwork of fields north and south.
"""


def write(path):
    lines = [HEADER, "version = 1",
             'bong = "oggle"',
             'tank = "titan"',
             'tank2 = "leviathan"',
             'biome = "valley"',
             'weather = ["clear", "dusk", "rain", "storm", "fog"]',
             f"size = [{W}, {H}]",
             'mission.kind = "protect"',
             'spawn.kind = "waves"',
             "spawn.waves = 6",
             "spawn.size = 4",
             "spawn.growth = 1",
             'spawn.tier_start = "medium"',
             'spawn.tier_end = "super"',
             ""]
    for (c, r) in sorted(cells, key=lambda p: (p[1], p[0])):
        lines.append(f'cells."{c},{r}" = {cells[(c, r)]}')
    with open(path, "w") as f:
        f.write("\n".join(lines) + "\n")


def check():
    problems = []
    for gc, gr in GATES:
        assert gc in (0, W - 1) or gr in (0, H - 1), (gc, gr)
        d = math.hypot(gc - BONG[0], gr - BONG[1])
        if d < 45:
            problems.append(f"gate {gc},{gr} is {d:.1f} cells from the bong")
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
        if kind in BLAST:
            d = math.hypot(c - BONG[0], r - BONG[1])
            if d < 8:
                problems.append(f"{kind} at {c},{r} is {d:.1f} cells from the bong")
    for gc, gr in GATES:
        if not all(nav_open(*n) for n in lane(gc, gr)):
            problems.append(f"gate {gc},{gr} lane is shut")
        elif lane(gc, gr)[-1] not in reach:
            problems.append(f"gate {gc},{gr} is cut off")
    for (c, r), t in cells.items():
        k = t.split('"')[1]
        if k == "barrel" and math.hypot(c - BONG[0], r - BONG[1]) < 8:
            problems.append(f"a drum at {c},{r} is near the bong")
        if k == "tesla" and math.hypot(c - BONG[0], r - BONG[1]) < 4:
            problems.append(f"a tesla at {c},{r} is near the bong")
    for p in water:
        if math.hypot(p[0] - BONG[0], p[1] - BONG[1]) < 8:
            problems.append(f"water at {p} within the bong's hop")
    for p in sorted(ROAD_PLAN - roads):
        if kind_at(p) not in ("gate",):
            problems.append(f"road at {p} built over by a {kind_at(p)}")
    for p in problems:
        print("CHECK:", p)
    assert not problems, f"{len(problems)} problems"


ASCII = {
    'wall': '#', 'road': '=', 'water': '~', 'bong': 'B', 'start': '1', 'start2': '2',
    'pickup': 'P', 'sandbag': 's', 'fence': 'f', 'tree': 'T', 'palm': 'Y', 'snag': 'x', 'willow': 'W',
    'birch': 'b', 'pine': 'p', 'scots': 'p', 'fir': 'p', 'spruce': 'p', 'tall_grass': '"', 'bush': 'o',
    'berry_bush': 'o', 'autumn_bush': 'o', 'fern': ',', 'reeds': '|', 'gate': 'G', 'tesla': 'Z',
    'gun_tower': 'K', 'bio_slush': 'S', 'barrel': 'D', 'hay': 'h', 'tyres': 'O', 'wood_crate': 'X',
    'barrier': 'n', 'tank_trap': '*', 'juniper': 'o', 'lamp': 'L', 'oil': ':', 'target': 'o',
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
                ch = {'brick': '#', 'iron': 'I', 'wood': 'w', 'concrete': 'C', 'glass': 'g'}.get(material_at((c, r)), '#')
            if k == 'water' and is_deep(c, r):
                ch = '%'
            if k in ('gun_tower', 'tesla', 'bio_slush') and 'enemy' in cells[(c, r)]:
                ch = ch.lower()
            if k == 'target':
                ch = '@'
            row.append(ch)
        print(f"{r:3d} " + "".join(row))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("-o", "--out", default="maps/castle-moat.toml")
    ap.add_argument("--stats", action="store_true")
    ap.add_argument("--ascii", action="store_true")
    ap.add_argument("--no-check", action="store_true")
    a = ap.parse_args()
    filled = build()
    if a.ascii:
        ascii_map()
    if not a.no_check:
        check()
    write(a.out)
    if a.stats:
        kinds = {}
        for t in cells.values():
            k = t.split('"')[1]
            kinds[k] = kinds.get(k, 0) + 1
        print(f"{len(cells)} cells, {filled} pocket cells filled")
        for k, n in sorted(kinds.items(), key=lambda kv: -kv[1]):
            print(f"  {k:12} {n}")
        sides = {}
        for t in cells.values():
            k = t.split('"')[1]
            if k in ("gun_tower", "tesla", "bio_slush"):
                s = "enemy" if "enemy" in t else "player"
                sides[(s, k)] = sides.get((s, k), 0) + 1
        print("  towers:", sorted(sides.items()))
        cost = walk_costs()
        for gc, gr in GATES:
            inside = lane(gc, gr)[-1]
            c = cost.get(inside)
            print(f"  gate {gc},{gr}: walk {c} steps = {c / 5:.1f} s" if c else f"  gate {gc},{gr}: no walk")
        for c, r, kind in pickups:
            walk = min((cost[n] for n in nav_of((c, r)) if n in cost), default=None)
            print(f"  pickup {kind:13} {c},{r}  walk {walk}  bong {math.hypot(c - BONG[0], r - BONG[1]):.1f}")


if __name__ == "__main__":
    main()
