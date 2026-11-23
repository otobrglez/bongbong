"""Write maps/harbor-lights.toml: Harbor Lights, level 10 - a fishing
harbour at night, the bong in the harbourmaster's yard on the quay, 97 x 54
cells, laid out set piece by set piece.

  python3 tools/gen_harbor_lights.py [-o maps/harbor-lights.toml] [--stats] [--ascii]

This script is the source: change it and run it again rather than editing
the TOML (a re-save from the map editor would drop the header too). It draws
no random numbers: every choice is hashed from the cell's position with a
fixed salt, so a rerun writes the same file byte for byte. `--ascii` prints
the map a character a cell, `--stats` the counts, each gate's walk from the
start and every pickup.

What it lays out (the header written into the map says how it plays):

- the sea along the south edge: the inner basin before the quay, deep; the
  west and east wharves a little further out; the west beach and the east
  headland, where the lighthouse stands on the point;
- the harbourmaster's yard on the quay, the players' walled base round the
  bong - gates west, north and east, the water at its back, a jetty out
  into the basin - with its towers and its stores;
- the west docks: two warehouses and the oil depot, a fenced yard of fuel
  and oil drums wired with oil trails, its pump house and the oil jetty
  with a tanker moored at it; the canal down through the town into the
  basin, and the canal quay between it and the yard;
- the east docks: the cannery and its yard, the fish market, the net
  lofts and the east pier with fishing boats moored along it;
- the town to the north: Front Street along the waterfront, Market Street
  across the middle, three streets up to the north edge, the church square
  at the head of the High Street, the market square, the town hall and
  terraces of houses with gardens behind them;
- lamp posts along the streets and the quay, at the pier heads and round
  the lighthouse - the light the night is fought by.
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

BONG = (48, 32)
START = (45, 37)
START2 = (51, 37)
# the harbourmaster's yard: its wall's outline (the south side is the
# basin, open), and its three gates
YARD = (38, 24, 58, 39)
YARD_GATES = {(38, r) for r in (27, 28, 29)} | {(58, r) for r in (27, 28, 29)} | {(c, 24) for c in (42, 43, 44)}
# the first water row: the basin before the quay, the wharves either side
QUAY_ROW = 40
WHARF_ROW = 42
CANAL = 30
# the streets: open rows (or columns) first to last; the road runs on the
# middle two
FRONT = (17, 21)
MARKET = (8, 11)
AVENUE_W = (19, 22)
HIGH = (46, 49)
AVENUE_E = (73, 76)
# the church square at the head of the High Street, the market square
CHURCH = (40, 0, 55, 7)
MARKET_SQ = (52, 12, 62, 16)

GATES = [
    (21, 0), (75, 0),  # the town's two north streets
    (0, 9), (96, 9),  # Market Street, both ends
    (0, 34),  # the west beach
    (96, 30),  # the coast road round the headland
]
GATE_LANES = set()

# a weapon met first in the campaign here: its crate lies in the start's
# sight box (docs/levels.md, "The weapons along the way")
FIRST_HERE = {"gauss_rail"}
# a seat's sight box, half extents in cells (`sight_box_half_cols`/`_rows`)
SIGHT = (11.5, 7.5)
# what an enemy turns off its road to fetch lies beyond every gate's leash
# (`enemy_leash_px`, 640 px = 20 cells)
FETCHED = {
    "laser", "minigun", "plasma", "missiles", "sonic_hammer", "emp_burst", "gauss_rail", "fpv_swarm",
    "rod_from_god", "gravity_well", "speedup", "shield",
}
# a blast weapon's crate lies at least two of its blasts' radii from the
# players' bong (docs/levels.md): 256 px for the grenade, 128 for a jet's
# bomb, 88 for a rocket (in cells, rounded up)
BLAST_CLEAR = {"grenades": 8.0, "fpv_swarm": 3.0, "air_jet": 4.0, "air_heli": 3.0, "missiles": 3.0}
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
    """A road two cells wide over the canal leaves no road cell with water
    on both sides, which is what the look draws as a plank deck
    (`ground::GroundGrid::bridge`): give the canal back the second row, so
    the first is a deck over it. The cell given back is a ford again."""
    for c, r in sorted(was_water):
        if (c, r) not in roads:
            continue
        if ((c, r - 1) in water and (c, r + 1) in water) or ((c - 1, r) in water and (c + 1, r) in water):
            continue
        for n in ((c, r + 1), (c + 1, r)):
            if n in roads and n in was_water:
                put(*n, "water")
                break


# ---------------------------------------------------------------- the sea


def shore_row(c):
    """The first water row of column `c`: the west beach shelving away to
    the south-west, the wharves, the basin before the quay, the headland's
    rocky shore pushing out to the point."""
    if c <= 11:
        return 44 + round(2.4 * fbm(c / 3.0, 0.5, 5)) + (1 if c < 5 else 0)
    if c < CANAL:
        return WHARF_ROW
    if c == CANAL:
        return 0
    if c <= 66:
        return QUAY_ROW
    if c <= 82:
        return WHARF_ROW
    # the headland: out to the point at 88-91, ragged
    point = 52 - abs(c - 89) * 0.45
    return max(WHARF_ROW + 1, min(H - 2, round(point + 1.6 * (fbm(c / 2.5, 3.5, 7) - 0.5))))


def sea():
    for c in range(W):
        if c == CANAL:
            continue
        for r in range(shore_row(c), H):
            put(c, r, "water")


def canal():
    """The canal down through the town into the basin: one cell wide, a
    ford its whole length with a current to the south, open banks either
    side for a towpath."""
    for r in range(0, H):
        put(CANAL, r, "water")
    for r in range(0, QUAY_ROW):
        for c in (CANAL - 1, CANAL + 1):
            reserved.add((c, r))


def is_deep(c, r):
    """Open water: a water cell whose eight neighbours are all water, the
    map's edge reading as the cell beside it (`ground::Layout` clamps)."""
    clamp = lambda c2, r2: (min(max(c2, 0), W - 1), min(max(r2, 0), H - 1))
    return all(clamp(c + dc, r + dr) in water for dc in (-1, 0, 1) for dr in (-1, 0, 1))


def pier(col, r0, r1, loot):
    """A plank pier: a road one cell wide out over the water (a deck, with
    the ford either side of it), ending in a head of dry boards three
    cells across and three long - `loot`'s two crates either side of its
    near end, a lamp at its far end."""
    for r in range(r0, r1 + 1):
        put(col, r, "road")
    for c, r in rect(col - 1, r1 + 1, col + 1, r1 + 3):
        clear(c, r)
        OPEN_GROUND.add((c, r))
    pickup(col - 1, r1 + 1, loot[0])
    pickup(col + 1, r1 + 1, loot[1])
    put(col, r1 + 3, "lamp")


def boat(c0, r0, length, hull="wood", cabin="glass", across=False):
    """A boat moored in the water: a hull two cells wide, a cabin of glass
    (or a wheelhouse) a third of the way along."""
    for i in range(length):
        for j in range(2):
            c, r = (c0 + i, r0 + j) if across else (c0 + j, r0 + i)
            mat = cabin if length // 3 <= i < length // 3 + max(1, length // 4) else hull
            wall(c, r, mat)


def rocks():
    """Rocks off the headland and the beach, standing in the shallows."""
    for c, r in ((83, 50), (81, 46), (2, 49), (9, 48), (12, 48)):
        if (c, r) in water:
            wall(c, r, "concrete")


# ---------------------------------------------------------------- the yard


def harbourmaster_yard():
    """The players' base on the quay: a brick wall round the bong on three
    sides with concrete posts at its gates, the basin at its
    back; gun towers on its north corners, a tesla behind the north wall
    and a bio slush by the water; the bong in a ring of sandbags open on
    every side; the stores round the walls; the jetty out into the basin."""
    c0, r0, c1, r1 = YARD
    clear_rect(c0 + 1, r0 + 1, c1 - 1, r1)
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) in YARD_GATES or (r == r1 and c0 < c < c1):
            continue  # the gates, and the quay open to the basin
        posts = any(abs(c - gc) + abs(r - gr) == 1 for gc, gr in YARD_GATES)
        wall(c, r, "concrete" if posts else "brick")
    put(c0, r0, "gun_tower")
    put(c1, r0, "gun_tower")
    put(BONG[0], r0 + 2, "tesla")
    put(c0 + 2, r1 - 1, "bio_slush")
    put(*BONG, "bong")
    put(*START, "start")
    put(*START2, "start2")
    bc, br = BONG
    for c, r in outline(bc - 2, br - 2, bc + 2, br + 2):
        if c == bc or r == br:
            continue
        put(c, r, "sandbag")
    # the stores: grenades and rockets in the north corners, air support
    # by the water, the gauss rail by the start, health, ammo and the
    # bong's packs round the walls - all clear of the ways in from the
    # gates (rows 27-29 across, columns 42-44 down)
    pickup(c0 + 2, r0 + 1, "grenades")
    pickup(c1 - 2, r0 + 1, "missiles")
    pickup(c0 + 1, r0 + 1, "health")
    pickup(c1 - 1, r0 + 1, "health")
    pickup(c0 + 1, 32, "ammo")
    pickup(c1 - 1, 32, "ammo")
    pickup(c0 + 1, 34, "health")
    pickup(c1 - 1, 34, "health")
    pickup(c0 + 4, r1, "air_heli")
    pickup(c1 - 4, r1, "air_jet")
    pickup(c0 + 4, 35, "gauss_rail")
    pickup(c1 - 4, 35, "shield")
    pickup(bc - 5, br + 1, "bong_health")
    pickup(bc + 5, br + 1, "bong_health")
    pickup(c0 + 2, r1, "ammo")
    pickup(c1 - 2, r1, "ammo")
    # crates stacked against the walls
    for c, r in ((c0 + 1, r0 + 2), (c1 - 1, r0 + 2), (c0 + 1, 36), (c1 - 1, 36)):
        put(c, r, "wood_crate")
    # the jetty: a plank deck out into the basin, its head stacked with
    # what the players need out there
    pier(BONG[0], QUAY_ROW, QUAY_ROW + 4, ("ammo", "health"))
    # sandbag wings either side of every gate, funnels out from the wall
    for c, r in ((36, 25), (37, 25), (36, 31), (37, 31),
                 (59, 25), (60, 25), (59, 31), (60, 31),
                 (40, 22), (40, 23), (46, 22), (46, 23)):
        put(c, r, "sandbag")
    # the apron before the north wall: tank traps, concrete barriers to
    # shove into the way, a stack of oil drums for whoever comes by it
    for c, r in ((51, 21), (56, 21)):
        put(c, r, "tank_trap")
    for c, r in ((44, 21), (37, 21)):
        put(c, r, "barrier")
    drum_cluster(((53, 22), (54, 22)), "oil")


# ---------------------------------------------------------------- the docks


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


def warehouse(c0, r0, c1, r1, salt, door=None, glass_roof=True):
    """A brick warehouse: the walls brick with a window of glass now and
    then, the roof brick with a skylight of glass down its middle. With
    `door` (cells of the wall left open) it is hollow inside."""
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
        corner = c in (c0, c1) and r in (r0, r1)
        if corner:
            mat = "brick"
        elif edge:
            mat = "glass" if hash01(c, r, salt) < 0.22 else "brick"
        else:
            mid = (r0 + r1) // 2
            mat = "glass" if glass_roof and r in (mid, mid + (r1 - r0 + 1) % 2 - 1) and (c - c0) % 3 != 0 else "brick"
        wall(c, r, mat)


def west_docks():
    """Two warehouses on Front Street - the east one open, plasma, health
    and ammo inside - and behind them the oil depot."""
    keep_open(12, 21, CANAL - 1, 21)
    warehouse(12, 22, 18, 27, 31)
    warehouse(23, 22, 28, 27, 33, door={(28, 24), (28, 25)})
    pickup(24, 23, "plasma")
    pickup(24, 26, "health")
    pickup(26, 26, "ammo")
    clear_rect(19, 22, 22, 28)


def oil_depot():
    """The oil depot on the west wharf: a fenced yard of fuel drums in
    tanks and oil drums in stacks, wired together by oil trails, a gate on
    every side; the pump house in the south-west (a flamethrower and a
    health pack - mind the drums); the oil jetty out to the tanker, missiles
    and speed at its head."""
    c0, r0, c1, r1 = 12, 29, 28, 41
    clear_rect(c0, r0, c1, r1)
    gates = {(c, r0) for c in (19, 20, 21, 22)} | {(c0, r) for r in (33, 34, 35)} | {(c1, r) for r in (33, 34, 35)}
    gates |= {(c, r1) for c in (21, 22, 23)}
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) in gates:
            continue
        put(c, r, "fence")
    # the pump house, its door to the yard road
    for c, r in outline(13, 37, 18, 40):
        if (c, r) in ((15, 37), (16, 37)):
            continue
        wall(c, r, "brick")
    keep_open(14, 38, 17, 39)
    pickup(14, 38, "flamethrower")
    pickup(17, 39, "health")
    # the tanks: fuel drums in squares, oil drums stacked, the trails
    # between them a fuse a blast runs along
    drum_cluster(((14, 31), (15, 31), (14, 32)), "fuel")
    drum_cluster(((25, 31), (26, 31), (26, 32)), "fuel")
    drum_cluster(((24, 38), (25, 38), (25, 39), (26, 39)), "oil")
    drum_cluster(((21, 38),), "fuel")
    oil_trail([(16, 31), (18, 31)])
    oil_trail([(23, 31), (24, 31)])
    oil_trail([(26, 33), (26, 36), (25, 36), (25, 37)])
    oil_trail([(20, 39), (22, 39)])
    # the oil jetty and the tanker
    pier(22, WHARF_ROW, WHARF_ROW + 5, ("missiles", "speedup"))
    for c, r in outline(25, WHARF_ROW + 2, 27, WHARF_ROW + 10):
        wall(c, r, "iron")
    for r in range(WHARF_ROW + 3, WHARF_ROW + 10):
        wall(26, r, "glass" if r in (WHARF_ROW + 7, WHARF_ROW + 8) else "wood")


def canal_quay():
    """Between the canal and the yard's west wall: the canal quay, open
    ground with a friendly gun tower by the basin and crates on the quay."""
    clear_rect(CANAL + 1, 21, YARD[0] - 1, QUAY_ROW - 1)
    put(35, 38, "gun_tower")
    for c, r in ((32, 36), (32, 37), (33, 37)):
        put(c, r, "wood_crate")
    # oil drums at the quay's head on Front Street, for whoever comes down
    # it to the yard's west gate
    drum_cluster(((33, 22), (34, 22), (33, 23)), "oil")


def east_docks():
    """The cannery on Front Street and its yard (an EMP burst among the
    fish crates, fuel drums by the boiler, an enemy bio slush watching
    over them); the fish market by the water (a minigun, health and ammo
    among the stalls) with a friendly gun tower on the quay beside it; the
    net lofts; the east pier, a sonic hammer and health at its head."""
    clear_rect(YARD[2] + 1, 21, 82, WHARF_ROW - 1)
    warehouse(63, 22, 72, 27, 41)
    # the yard, south of the coast road
    pickup(64, 32, "emp_burst")
    pickup(67, 32, "ammo")
    drum_cluster(((70, 33), (71, 33), (71, 32)), "fuel")
    put(74, 33, "bio_slush", side="enemy")
    for c, r in ((62, 32), (62, 33), (66, 33)):
        put(c, r, "wood_crate")
    # the fish market: two rows of stalls with a lane between, open at
    # both ends
    for c in (61, 62, 63, 67, 68, 69):
        wall(c, 35, "wood")
        put(c, 38, "wood_crate" if c in (62, 68) else "hay")
    pickup(65, 36, "minigun")
    pickup(65, 38, "health")
    pickup(71, 37, "ammo")
    put(61, QUAY_ROW - 1, "gun_tower")
    # the net lofts
    for c, r in rect(76, 22, 81, 25):
        edge = c in (76, 81) or r in (22, 25)
        wall(c, r, "glass" if edge and hash01(c, r, 43) < 0.2 and not (c in (76, 81) and r in (22, 25)) else "wood")
    for c, r in ((77, 27), (78, 27), (80, 27)):
        put(c, r, "hay")
    # the east pier and the boats moored along it
    pier(77, WHARF_ROW, WHARF_ROW + 5, ("sonic_hammer", "health"))
    boat(79, WHARF_ROW + 1, 4)
    boat(74, WHARF_ROW + 2, 4, cabin="wood")


def boatyard():
    """The boatyard on the east wharf: a trawler up on the slip for
    repair, timber stacked beside it, the crane's iron foot, tyre fenders
    along the wharf's edge."""
    for c, r in rect(77, 33, 79, 38):
        wall(c, r, "glass" if c == 78 and r in (35, 36) else "wood")
    for c, r in ((75, 33), (75, 34), (81, 36), (81, 37), (74, 38)):
        put(c, r, "wood_crate")
    for c, r in rect(81, 32, 82, 33):
        wall(c, r, "iron")
    for c in (69, 73, 80):
        put(c, WHARF_ROW - 1, "tyres")


def basin_boats():
    """Fishing boats moored in the basin and off the west wharf."""
    boat(33, QUAY_ROW + 2, 5)
    boat(53, QUAY_ROW + 2, 4, cabin="wood")
    boat(60, QUAY_ROW + 3, 6)
    boat(41, QUAY_ROW + 7, 5, across=True)
    boat(16, WHARF_ROW + 2, 4)
    boat(65, QUAY_ROW + 9, 6, across=True, cabin="wood")


# ---------------------------------------------------------------- the shore


def west_beach():
    """The beach west of the depot: dunes of marram and juniper, boats
    hauled up on the sand, the boat shed (grenades, a health pack), nets
    drying on fences, a fire of oil drums by the slipway."""
    keep_open(0, 33, 11, 35)
    # the boat shed, its door to the east
    for c, r in outline(2, 24, 7, 28):
        if (c, r) in ((7, 25), (7, 26)):
            continue
        wall(c, r, "wood")
    keep_open(7, 25, 9, 26)
    for c, r in rect(3, 25, 6, 27):
        clear(c, r)
        OPEN_GROUND.add((c, r))
    pickup(4, 26, "grenades")
    pickup(5, 26, "health")
    # boats hauled up the beach
    for c0, r0 in ((3, 38), (8, 39)):
        for c, r in rect(c0, r0, c0 + 1, r0 + 2):
            wall(c, r, "wood")
    # nets on their poles
    for c in (2, 3, 4):
        put(c, 31, "fence")
    for r in (41, 42):
        put(5, r, "fence")
    drum_cluster(((9, 30), (10, 30)), "oil")
    keep_open(7, 29, 11, 31)


def headland():
    """The headland east of the docks: the coast road round it from the
    east edge, pines and rocks, the keeper's cottage (a helicopter strike,
    health) and the round lighthouse on the point with an enemy gun tower
    before it and the FPV swarm at its foot."""
    keep_open(83, 28, W - 1, 31)
    # the lighthouse: a round tower of white brick, its glass lantern in
    # the middle
    lc, lr = 88, 46
    for c, r in rect(lc, lr, lc + 3, lr + 3):
        if c in (lc, lc + 3) and r in (lr, lr + 3):
            continue
        inner = lc < c < lc + 3 and lr < r < lr + 3
        wall(c, r, "glass" if inner else "brick")
    for c, r in rect(lc - 2, lr - 2, lc + 5, lr + 5):
        if (c, r) not in cells and (c, r) not in water:
            clear(c, r)
            OPEN_GROUND.add((c, r))
    put(lc - 1, lr - 1, "lamp")
    put(lc + 4, lr - 1, "lamp")
    put(86, 41, "gun_tower", side="enemy")
    pickup(85, 48, "fpv_swarm")
    pickup(86, 46, "health")
    # the keeper's cottage, its door to the west
    for c, r in outline(90, 36, 94, 40):
        if (c, r) in ((90, 37), (90, 38)):
            continue
        wall(c, r, "wood" if r in (36, 40) else "brick")
    for c, r in rect(91, 37, 93, 39):
        clear(c, r)
        OPEN_GROUND.add((c, r))
    pickup(92, 38, "air_heli")
    pickup(92, 37, "health")
    keep_open(86, 37, 89, 40)


# ---------------------------------------------------------------- the town


# a house's walls and roof: white brick under terracotta (the roof is
# drawn in timber's orange), timber under slate (brick's grey), or brick
# with a glass roof - a shop, a conservatory
HOUSE_STYLES = (("brick", "wood", 55), ("wood", "brick", 30), ("brick", "glass", 15))


def house(c0, r0, c1, r1, salt, front):
    """A house, solid: its walls in one material with glass windows along
    the `front` face (a row), its roof in another, and a concrete chimney
    on the roof now and then."""
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
    """A row of houses along a block's street front, from column c0 to
    c1, rows r0 to r1: widths of four to six, now and then a yard three
    wide between two of them (a way through to the gardens behind), and
    now and then a house set back behind a front garden a row deep."""
    c = c0
    i = 0
    back = -1 if front == r1 else 1
    while c <= c1:
        w = 4 + int(hash01(c, salt, 3) * 3)
        if c + w - 1 > c1 or c1 - (c + w - 1) < 4:
            w = c1 - c + 1
        hr0, hr1, hfront = r0, r1, front
        if hash01(c, salt, 11) < 0.35:
            # set back: a row of garden between it and the street
            if back < 0:
                hr1, hfront = r1 - 1, r1 - 1
            else:
                hr0, hfront = r0 + 1, r0 + 1
            for gc in range(c, c + w):
                gr = front
                reserved.add((gc, gr))
                if hash01(gc, gr, salt + 13) < 0.6:
                    put(gc, gr, pick(GARDEN, hash01(gc, gr, salt + 17)))
        house(c, hr0, c + w - 1, hr1, salt + i, hfront)
        c += w
        i += 1
        if c <= c1 - 7 and hash01(c, salt, 7) < 0.45:
            keep_open(c, r0, c + 2, r1)
            c += 3


def church_square():
    """The church at the head of the High Street, its square before it: a
    fountain in the middle, lamps on the corners, an enemy gun tower on
    the east side over the laser and a box of ammo."""
    c0, r0, c1, r1 = CHURCH
    clear_rect(c0, r0, c1, r1)
    # the church: brick, its rose window and its aisles' windows glass
    for c, r in rect(44, 0, 51, 3):
        win = (r == 3 and c in (46, 49)) or (r == 1 and c in (47, 48)) or (c in (44, 51) and r == 1)
        wall(c, r, "glass" if win else "brick")
    for c, r in ((45, 3), (50, 3)):
        wall(c, r, "concrete")
    # the fountain, a basin two cells across: shallow
    for c, r in rect(47, 5, 48, 6):
        put(c, r, "water")
    put(53, 5, "gun_tower", side="enemy")
    pickup(54, 3, "laser")
    pickup(55, 6, "ammo")
    for c, r in ((40, 0), (55, 0)):
        put(c, r, "lamp")
    for c, r in ((41, 1), (42, 1), (53, 1), (54, 1), (40, 6), (41, 7)):
        put(c, r, "autumn_bush" if hash01(c, r, 51) < 0.5 else "berry_bush")


def market_square():
    """The market square between Market Street and Front Street: stalls of
    crates and hay, drums by the fish stall, an enemy tesla in the middle
    of it over plasma, speed and ammo."""
    c0, r0, c1, r1 = MARKET_SQ
    clear_rect(c0, r0, c1, r1)
    put(57, 14, "tesla", side="enemy")
    pickup(55, 13, "plasma")
    pickup(60, 15, "speedup")
    pickup(55, 15, "ammo")
    for c, r in ((53, 12), (62, 12), (53, 16), (62, 16)):
        put(c, r, "wood_crate")
    drum_cluster(((61, 13),), "oil")
    for c, r in ((59, 12),):
        put(c, r, "hay")


def town_hall():
    """The town hall on Front Street: brick and glass round a hall, its
    doors to the street, the second gauss rail and a health pack inside."""
    c0, r0, c1, r1 = 35, 12, 41, 16
    for c, r in outline(c0, r0, c1, r1):
        if r == r1 and c in (37, 38, 39):
            continue
        win = r == r0 and (c - c0) % 2 == 1
        wall(c, r, "glass" if win else ("concrete" if c in (c0, c1) and r in (r0, r1) else "brick"))
    for c, r in rect(c0 + 1, r0 + 1, c1 - 1, r1 - 1):
        clear(c, r)
        OPEN_GROUND.add((c, r))
    pickup(40, 13, "gauss_rail")
    pickup(36, 13, "health")
    keep_open(37, 16, 39, 16)


def chandlery():
    """The ship's chandler's in the north-west, open to Market Street:
    health and ammo on its shelves."""
    c0, r0, c1, r1 = 8, 3, 13, 7
    for c, r in outline(c0, r0, c1, r1):
        if r == r1 and c in (10, 11):
            continue
        wall(c, r, "wood" if r == r0 else "brick")
    for c, r in rect(c0 + 1, r0 + 1, c1 - 1, r1 - 1):
        clear(c, r)
        OPEN_GROUND.add((c, r))
    pickup(9, 4, "health")
    pickup(12, 4, "ammo")
    keep_open(10, 7, 11, 7)


def chapel_garden():
    """A walled garden in the north-east on Market Street, its gate to
    the street: a health pack and ammo by its back wall."""
    c0, r0, c1, r1 = 78, 3, 85, 7
    for c, r in outline(c0, r0, c1, r1):
        if r == r1 and c in (81, 82):
            continue
        wall(c, r, "brick")
    for c, r in rect(c0 + 1, r0 + 1, c1 - 1, r1 - 1):
        clear(c, r)
        OPEN_GROUND.add((c, r))
    pickup(80, 4, "health")
    pickup(83, 4, "ammo")
    keep_open(81, 7, 82, 7)


def town():
    """The town: the terraces along the streets, the gardens behind them."""
    church_square()
    market_square()
    town_hall()
    chandlery()
    chapel_garden()
    # the north band: terraces fronting Market Street, gardens behind
    north_front = MARKET[0] - 1
    for c0, c1, salt in ((0, 7, 101), (14, AVENUE_W[0] - 1, 103), (AVENUE_W[1] + 1, CANAL - 2, 107),
                         (CANAL + 2, CHURCH[0] - 1, 109), (CHURCH[2] + 1, AVENUE_E[0] - 1, 113),
                         (87, W - 1, 131)):
        if c1 - c0 >= 3:
            terrace(c0, north_front - 3, c1, north_front, north_front, salt)
    # the south band: terraces fronting Front Street, backing onto Market
    # Street's south side
    south_front = FRONT[0] - 1
    for c0, c1, salt in ((0, AVENUE_W[0] - 1, 301), (AVENUE_W[1] + 1, CANAL - 2, 307),
                         (CANAL + 2, 34, 311), (42, HIGH[0] - 1, 313), (HIGH[1] + 1, MARKET_SQ[0] - 1, 317),
                         (MARKET_SQ[2] + 1, AVENUE_E[0] - 1, 331), (AVENUE_E[1] + 1, W - 1, 337)):
        if c1 - c0 >= 3:
            terrace(c0, MARKET[1] + 1, c1, south_front, south_front, salt)


def lamps():
    """Lamp posts: along Front Street against the houses, along the quay,
    at the street corners of Market Street, at the bridge ends."""
    spots = []
    for c in range(3, W - 3, 7):
        spots.append((c, FRONT[0], -1))
    for c in range(7, W - 3, 9):
        spots.append((c, FRONT[1], 1))
    for c in range(5, W - 3, 11):
        spots.append((c, MARKET[0], -1))
        spots.append((c + 5, MARKET[1], 1))
    placed = 0
    for c, r, back in spots:
        behind = (c, r + back)
        if kind_at(behind) == "wall" and (c, r) not in cells and (c, r) not in roads and (c, r) not in water:
            put(c, r, "lamp")
            placed += 1
    # the quay: inside the yard's corners by the water and on the canal
    # quay and the east quay
    for c, r in ((YARD[0] + 1, YARD[3]), (YARD[2] - 1, YARD[3]), (CANAL + 1, QUAY_ROW - 1), (66, QUAY_ROW - 1),
                 (CANAL - 1, WHARF_ROW - 1), (82, WHARF_ROW - 1)):
        if (c, r) not in cells:
            put(c, r, "lamp")
            placed += 1
    return placed


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


GARDEN = (("berry_bush", 35), ("autumn_bush", 35), ("bush", 20), ("juniper", 10))
GARDEN_TREES = (("birch", 30), ("tree", 30), ("palm", 25), ("willow", 15))
DUNE_TREES = (("scots", 60), ("pine", 25), ("palm", 15))
HEADLAND_TREES = (("pine", 40), ("scots", 30), ("fir", 15), ("snag", 15))
BANK = (("willow", 70), ("birch", 30))


def zone(c, r):
    if r < FRONT[0]:
        return "town"
    if c <= 11:
        return "beach"
    if c >= 83:
        return "headland"
    return "docks"


def planting():
    """Everything not a set piece: gardens behind the terraces (flowering
    bushes, birches, palms and broadleaf trees), dunes of marram and
    juniper with pines on the beach, pines, firs, snags and juniper on the
    headland, reeds along the water. The docks stay bare."""
    wd = distance_field(water, 3)
    for r in range(H):
        for c in range(W):
            p = (c, r)
            if p in cells or p in reserved:
                continue
            z = zone(c, r)
            if z == "docks":
                continue
            dw = wd.get(p, 9)
            u = hash01(c, r, 13)
            v = hash01(c, r, 19)
            if dw == 1 and hash01(c, r, 85) < 0.45:
                put(c, r, "reeds")
            elif z == "town":
                g = fbm(c / 3, r / 3, 17)
                if u < 0.09:
                    put(c, r, pick(GARDEN_TREES, hash01(c, r, 79)))
                elif g > 0.58 and v < 0.55:
                    put(c, r, pick(GARDEN, hash01(c, r, 23)))
                elif g < 0.35 and v < 0.25:
                    put(c, r, "tall_grass")
                else:
                    continue
            elif z == "beach":
                g = fbm(c / 3.5, r / 3.5, 29)
                if u < 0.07 and dw > 2:
                    put(c, r, pick(DUNE_TREES, hash01(c, r, 31)))
                elif g > 0.46 and v < 0.85:
                    put(c, r, "tall_grass")
                elif v < 0.12:
                    put(c, r, "juniper")
                else:
                    continue
            else:  # headland
                d = fbm(c / 4, r / 4, 37)
                if u < (0.34 if d > 0.5 else 0.1):
                    put(c, r, pick(HEADLAND_TREES, hash01(c, r, 41)))
                elif d < 0.45 and v < 0.5:
                    put(c, r, "tall_grass")
                elif v < 0.25:
                    put(c, r, "juniper")
                elif v > 0.975:
                    wall(c, r, "concrete")  # a boulder
                else:
                    continue
            reserved.discard(p)


def canal_banks():
    """Willows and reeds now and then on the canal's banks - a willow on
    one bank only, so the towpath on the other stays a way through."""
    for r in range(0, QUAY_ROW):
        for c in (CANAL - 1, CANAL + 1):
            p = (c, r)
            if p in cells or p in roads or p in OPEN_GROUND:
                continue
            u = hash01(c, r, 61)
            if c == CANAL - 1 and r in (2, 5, 13, 15, 37) and kind_at((c - 1, r)) != "tree":
                put(c, r, "willow")
            elif u < 0.18:
                put(c, r, "reeds")


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
                    z = zone(*p)
                    tree = {"town": GARDEN_TREES, "beach": DUNE_TREES, "headland": HEADLAND_TREES}.get(z, BANK)
                    put(*p, pick(tree, hash01(*p, 73)))
                    filled += 1
                    break
            else:
                # a pocket of shallows - between a boat and the shore, say:
                # a rock rises in it
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
    sea()
    canal()
    # The streets run in straight reaches along the rows and columns: a
    # hull drives four ways, and a diagonal corridor makes it stair-step.
    was_water = set(water)
    keep_open(0, FRONT[0], W - 1, FRONT[1])
    keep_open(0, MARKET[0], W - 1, MARKET[1])
    keep_open(AVENUE_W[0], 0, AVENUE_W[1], FRONT[1])
    keep_open(AVENUE_E[0], 0, AVENUE_E[1], FRONT[1])
    keep_open(HIGH[0], CHURCH[3], HIGH[1], FRONT[1])
    road([(0, FRONT[0] + 1), (W - 2, FRONT[0] + 1)])
    road([(0, MARKET[0] + 1), (W - 2, MARKET[0] + 1)])
    road([(AVENUE_W[0] + 1, 0), (AVENUE_W[0] + 1, 28)])
    road([(AVENUE_E[0] + 1, 0), (AVENUE_E[0] + 1, FRONT[1])])
    road([(HIGH[0] + 1, CHURCH[3] + 1), (HIGH[0] + 1, FRONT[1])])
    # the depot's yard road, the coast road, the quay road east
    road([(0, 33), (CANAL - 1, 33)])
    road([(YARD[2] + 3, 29), (W - 2, 29)])
    bridge_decks(was_water)

    canal_quay()
    west_docks()
    oil_depot()
    east_docks()
    boatyard()
    harbourmaster_yard()
    basin_boats()
    west_beach()
    headland()
    rocks()
    town()
    placed_lamps = lamps()
    gate_aprons()
    for gc, gr in GATES:
        put(gc, gr, "gate")
        GATE_LANES.update(lane(gc, gr))
    # palms along Front Street before the warehouses and the cannery
    for c in (13, 18, 23, 28, 64, 67, 73 - 1):
        if (c, FRONT[1]) not in cells and kind_at((c, FRONT[1] + 1)) == "wall":
            put(c, FRONT[1], "palm")
    canal_banks()
    planting()
    filled = fill_pockets()
    close_narrow_trails()
    return filled, placed_lamps


HEADER = """\
# harbor-lights - Harbor Lights, level 10: a fishing harbour at night.
# Written by tools/gen_harbor_lights.py - change the generator and run it
# again rather than editing this file; re-saving from the map editor would
# drop these comments. Level tables as dotted keys on purpose - a `[spawn]`
# header would swallow every `cells.` line after it.
#
# Protect on the waves plan: five waves of 3 to 7 tanks, light to heavy, 25
# in all, rolling in from the town and the shore - four gates in the town
# (the two north streets, both ends of Market Street), one on the west beach
# and one on the coast road round the headland. Always night (docs/weather.md:
# the enemies see six tenths as far, and whoever stands in a lamp's light is
# seen at the full range), the lamp posts along the streets and the quay the
# only light but the fires. 97 x 54 cells, followed by the camera, in the
# floodplain's look: slate water, paving round the towers by the water.
#
# The players hold the harbourmaster's yard on the quay: a brick wall round
# the bong on three sides, concrete posts at its three gates (west, north,
# east), the basin at its back. Gun towers on its north corners, a tesla
# behind the north wall, a bio slush by the water, the bong in a ring of
# sandbags; sandbag wings either side of every gate, tank traps, barriers
# and a stack of oil drums on the apron before it. Its stores: the gauss rail
# by the start (first met here - a slug goes through brick, glass and wood,
# and through either side's bong), grenades and missiles in the north
# corners, a helicopter and a jet strike by the water, a shield, two bong
# health packs, four health packs and four of ammo; ammo and health at the
# head of its jetty.
#
# The west docks: two warehouses on Front Street (plasma, health and ammo in
# the open one), and behind them the oil depot - a fenced yard of fuel and
# oil drums wired together by oil trails, the road from the west beach
# running through it, the flamethrower in its pump house, missiles and speed
# at the head of the oil jetty beside a moored tanker. The canal comes down
# through the town into the basin past the canal quay, where a friendly gun
# tower stands by the water. The west beach beyond: dunes, boats hauled up,
# the boat shed (grenades, health).
#
# The east docks: the cannery and its yard (the EMP burst among the crates,
# fuel drums by the boiler, an enemy bio slush), the fish market's stalls by
# the water (a minigun, health, ammo) with a second friendly gun tower on the
# quay, the net lofts, the east pier with the sonic hammer at its head and
# boats moored along it, and the boatyard with a trawler up on the slip. The
# headland past them, the coast road round it: pines and boulders, the
# keeper's cottage (a helicopter strike, health), and the lighthouse on the
# point, guarded by an enemy gun tower, the FPV swarm and health at its foot.
#
# The town to the north: Front Street along the waterfront, Market Street
# across the middle, two streets up to the north edge, the High Street up to
# the church square and the canal under its bridges. The church square at the head of the High Street holds the
# laser under an enemy gun tower; the market square plasma and speed under an
# enemy tesla; the town hall on Front Street a second gauss rail; terraces of
# brick and timber houses with gardens behind them, a chandler's and a walled
# garden with health and ammo. Lamp posts along the streets and the quay, at
# the pier heads and round the lighthouse; oil drums at the head of the
# canal quay, on the apron and in the market for whoever comes by them.
"""


def write(path):
    lines = [HEADER, "version = 1",
             'bong = "spike"',
             'tank = "ravager"',
             'tank2 = "titan"',
             'biome = "floodplain"',
             'weather = "night"',
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
    reach = nav_reach()
    near = lambda p, rr: any((p[0] + dc, p[1] + dr) in rr for dc in range(-4, 4) for dr in range(-4, 4))
    for p in (START, START2, BONG):
        assert near(p, reach), p
    for c, r, kind in pickups:
        assert reachable((c, r), reach), f"{kind} at {c},{r} cannot be reached"
        if kind in FETCHED:
            far = min(math.hypot(c - gc, r - gr) for gc, gr in GATES)
            assert far >= 20.5, f"{kind} at {c},{r} is {far:.1f} cells from a gate"
        if kind in BLAST_CLEAR:
            d = math.hypot(c - BONG[0], r - BONG[1])
            assert d >= BLAST_CLEAR[kind], f"{kind} at {c},{r} is {d:.1f} cells from the bong"
    for kind in FIRST_HERE:
        firsts = [(pc, pr) for pc, pr, k in pickups if k == kind]
        assert any(abs(pc - START[0]) <= SIGHT[0] and abs(pr - START[1]) <= SIGHT[1] for pc, pr in firsts), \
            f"no {kind} in the start's sight box"
    for gc, gr in GATES:
        assert all(nav_open(*n) for n in lane(gc, gr)), f"gate {gc},{gr}"
        assert lane(gc, gr)[-1] in reach, f"gate {gc},{gr} is cut off"
    # drums and teslas go off: none within reach of the bong
    for (c, r), t in cells.items():
        k = t.split('"')[1]
        if k == "barrel":
            assert math.hypot(c - BONG[0], r - BONG[1]) >= 8, f"a drum at {c},{r} is near the bong"
        if k == "tesla":
            assert math.hypot(c - BONG[0], r - BONG[1]) >= 4, f"a tesla at {c},{r} is near the bong"


ASCII = {
    'wall': '#', 'road': '=', 'water': '~', 'bong': 'B', 'start': '1', 'start2': '2',
    'pickup': 'P', 'sandbag': 's', 'fence': 'f', 'tree': 'T', 'palm': 'Y', 'snag': 'x', 'willow': 'W',
    'birch': 'b', 'pine': 'p', 'scots': 'p', 'fir': 'p', 'spruce': 'p', 'tall_grass': '"', 'bush': 'o',
    'berry_bush': 'o', 'autumn_bush': 'o', 'fern': ',', 'reeds': '|', 'gate': 'G', 'tesla': 'Z',
    'gun_tower': 'K', 'bio_slush': 'S', 'barrel': 'D', 'hay': 'h', 'tyres': 'O', 'wood_crate': 'X',
    'barrier': 'n', 'tank_trap': '*', 'juniper': 'o', 'lamp': 'L', 'oil': ':',
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
            row.append(ch)
        print(f"{r:3d} " + "".join(row))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("-o", "--out", default="maps/harbor-lights.toml")
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
