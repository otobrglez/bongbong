"""Write maps/scrapyard.toml: Scrapyard, level 12 - a junkyard behind the
wire on the desert, 68 x 34 cells, laid out piece by piece.

  python3 tools/gen_scrapyard.py [-o maps/scrapyard.toml] [--stats] [--ascii [--routes]] [--nav]

This script is the map's source: change it and run it again rather than
editing the TOML (a re-save from the map editor would drop the header too).
Nothing is drawn at random - the set pieces are placed by hand and the junk
is hashed from cell positions - so every run writes the same file.
`--ascii` prints the map a character a cell (`--routes` marks the waves'
ways in), `--nav` the nav grid the AI routes on, `--stats` the counts, each
gate's walk to the start and every pickup. The checks run every time and
fail the run: every pickup, start and gate lane reached, no boxed-in or
cut-off nav cell, no drum by a start, no loose prop on a wave's cheapest
way in (the router drives straight through loose props and the hull then
has to shove them), and no straight line from anywhere outside the yard
office's walls to either start.

What it lays out (the header written into the map says how it plays):

- the yard office in the west, the players' start: a compound of junk
  walls on a concrete footing, four gates, a concrete baffle inside each,
  gun towers and bio slush on the gates' posts, a tesla inside, the stores
  round the court;
- the wire round the whole yard on the field's edge, open at the waves'
  six gates;
- the tyre stacks banked along the wire north of the office and the old
  filling station south of it, its pumps and fuel drums against the wire,
  spilt oil running east to a heap of drums;
- the crane in the north, its magnet down over a heap of wrecks, an enemy
  gun tower on its counter-jib over the cab's stash;
- the haul road east through the yard between heaps of scrap, oil spilt
  along it, to the crusher, a hydraulic press with a weapon in its jaws
  between two enemy gun towers, its bales stacked behind it;
- the container yard in the north-east, a stash in each alley under an
  enemy tesla; the sorting bays in the south-east, an enemy bio slush
  between them; the car lot in the south, a row of wrecks and a bus;
- junk strewn between - wrecks, scrap, tyres, tank traps, crates - drums in
  heaps beside the ways in but never on them, and weeds through it all.
"""

import argparse
import heapq
import math
from collections import deque

W, H = 68, 34

# (col, row) -> inline TOML table text
cells = {}
# cells the junk and weed passes must leave alone
reserved = set()
roads = set()
pickups = []  # (c, r, kind)
DRUMS = []

COMPOUND = (1, 9, 16, 25)
START = (7, 15)
START2 = (7, 19)

GATES = [
    (38, 0), (55, 0),  # north: past the crane's magnet, and through the container yard
    (67, 10), (67, 24),  # east, either side of the crusher's bales
    (31, 33), (58, 33),  # south, the car lot and the sorting bays
]

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
    if kind == "road":
        roads.add((c, r))
    else:
        roads.discard((c, r))
    reserved.add((c, r))


def wall(c, r, material):
    put(c, r, "wall", material=material)


def clear(c, r):
    if inb(c, r):
        cells.pop((c, r), None)
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


def keep_clear(c0, r0, c1, r1):
    """Open ground a set piece needs: the junk and the weeds leave it."""
    for p in rect(c0, r0, c1, r1):
        reserved.add(p)


def clear_rect(c0, r0, c1, r1):
    for c, r in list(rect(c0, r0, c1, r1)):
        clear(c, r)


def pickup(c, r, kind):
    assert (c, r) not in roads, f"{kind} at {c},{r} would cover the road"
    put(c, r, "pickup", pickup=kind)
    keep_clear(c - 1, r - 1, c + 1, r + 1)
    pickups.append((c, r, kind))


def tower(c, r, kind, enemy=False):
    if enemy:
        put(c, r, kind, side="enemy")
    else:
        put(c, r, kind)


PLANNED_LOOSE = []


def drum(c, r, kind):
    """A drum (`kind` oil or fuel) the set piece wants at (c, r): placed
    last, by `place_loose`, once the waves' ways in are known, and only
    beside them. Its cell is held meanwhile so nothing else takes it."""
    PLANNED_LOOSE.append((c, r, kind))
    reserved.add((c, r))


def loose_prop(c, r, kind):
    """A loose prop - a wooden crate - the set piece wants at (c, r),
    placed the way a drum is."""
    PLANNED_LOOSE.append((c, r, kind))
    reserved.add((c, r))


def place_loose(routes):
    """Every planned drum and crate on its cell, or - where its
    cell lies on a wave's way in (`routes`) or was built over - on the
    nearest free cell beside it that does not, so a cluster keeps its drums
    and a wave meets none in its road. No drum within seven cells of a
    start."""
    for c, r, kind in PLANNED_LOOSE:
        for p in sorted(rect(c - 1, r - 1, c + 1, r + 1),
                        key=lambda q: ((q[0] - c) ** 2 + (q[1] - r) ** 2, q[1], q[0])):
            if p in cells or p in routes or not (1 <= p[0] < W - 1 and 1 <= p[1] < H - 1):
                continue
            if kind in ("oil", "fuel"):
                if min(math.hypot(p[0] - s[0], p[1] - s[1]) for s in (START, START2)) < 7:
                    continue
                put(*p, "barrel", drum=kind)
                DRUMS.append(p)
            else:
                put(*p, kind)
            break


def drums(spots, salt, fuel=0.35):
    """Drums at `spots`, each oil or fuel by a hash of its cell: `fuel` of
    them the grey fuel drums that launch when chained."""
    for c, r in spots:
        drum(c, r, "fuel" if hash01(c, r, salt) < fuel else "oil")


def oil(spots):
    for c, r in spots:
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


# ---------------------------------------------------------------- pieces

# what a heap of scrap is made of: sheet iron and car bodies (iron), pallets
# (wood), white goods - fridges, washers, a boiler - (brick, which the
# desert draws white), windscreens (glass)
SCRAP = (("iron", 42), ("wood", 22), ("brick", 22), ("glass", 14))
# white goods: the heap by the office
WHITE_GOODS = (("brick", 62), ("glass", 18), ("iron", 20))
# a heap of wrecks dropped under the magnet
WRECKS = (("iron", 34), ("wood", 22), ("brick", 22), ("glass", 22))
# the yard office's junk walls: sheet iron and pallets on a concrete footing
JUNK_WALL = (("iron", 50), ("wood", 26), ("brick", 14), ("concrete", 10))
# a wreck's paint: grey, orange, white
PAINT = ("iron", "wood", "brick")


def car(c, r, salt, vertical=False):
    """A wreck three cells long: bonnet and boot in its paint, the cab's
    glass between them."""
    body = PAINT[int(hash01(c, r, salt) * len(PAINT)) % len(PAINT)]
    for i in range(3):
        p = (c, r + i) if vertical else (c + i, r)
        wall(*p, "glass" if i == 1 else body)


def heap(cx, cy, rx, ry, salt, table=SCRAP):
    """A heap of scrap: a ragged ellipse of mixed junk, filled solid."""
    out = []
    for r in range(int(cy - ry) - 2, int(cy + ry) + 3):
        for c in range(int(cx - rx) - 2, int(cx + rx) + 3):
            if not inb(c, r) or (c, r) in cells or (c, r) in roads:
                continue
            dx, dy = (c - cx) / rx, (r - cy) / ry
            wobble = 0.5 * (fbm(c / 2.5, r / 2.5, salt) - 0.5)
            if dx * dx + dy * dy <= 1.0 + wobble:
                wall(c, r, pick(table, hash01(c, r, salt + 7)))
                out.append((c, r))
    return out


def the_wire():
    """The chain-link fence round the yard on the field's own edge, open at
    every gate four cells wide, iron posts at its corners. On the edge, so
    a stretch blown or burnt away opens nothing but ground the field's
    border already walls: a fence a cell in would leave a strip behind it
    that a hull could reach only through the hole and stick in."""
    gaps = set()
    for gc, gr in GATES:
        if gr == 0 or gr == H - 1:
            gaps.update((c, gr) for c in range(gc - 1, gc + 3))
        else:
            gaps.update((gc, r) for r in range(gr - 1, gr + 3))
    for c, r in outline(0, 0, W - 1, H - 1):
        if (c, r) not in gaps:
            put(c, r, "fence")
    for c, r in ((0, 0), (W - 1, 0), (0, H - 1), (W - 1, H - 1)):
        wall(c, r, "iron")


def yard_office():
    """The players' compound: junk walls on a concrete footing, a gate three
    cells wide in the north and south walls and two in the east, each
    gate's posts concrete. Inside every gate a concrete baffle, so nothing
    outside the walls has a straight shot at either start: a tank coming in
    turns along the wall and reaches the court only round a baffle's end.
    Gun towers on the east gates' outer posts, bio slush on the north and
    south gates', a tesla at the north end of the gap between the east
    baffles - enough to thin a wave, not to hold the yard for a crew that
    sits still. The office trailer and the workshop fill the north and
    south corridors' east ends; the stores lie round the walls."""
    c0, r0, c1, r1 = COMPOUND
    north = {(c, r0) for c in range(6, 9)}
    south = {(c, r1) for c in range(6, 9)}
    ne = {(c1, r) for r in range(11, 14)}
    se = {(c1, r) for r in range(21, 24)}
    gates = north | south | ne | se
    posts = set()
    for g in gates:
        for dc, dr in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            n = (g[0] + dc, g[1] + dr)
            if n not in gates and (n[0] in (c0, c1) or n[1] in (r0, r1)):
                posts.add(n)
    clear_rect(c0 + 1, r0 + 1, c1 - 1, r1 - 1)
    keep_clear(c0, r0, c1, r1)
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) in gates:
            continue
        if c == c0 or (c, r) in posts:
            wall(c, r, "concrete")
        else:
            wall(c, r, pick(JUNK_WALL, hash01(c, r, 3)))
    tower(c1, 10, "gun_tower")
    tower(c1, 24, "gun_tower")
    tower(5, r0, "bio_slush")
    tower(9, r1, "bio_slush")
    # the baffles
    for c in range(5, 12):
        wall(c, 13, "concrete")
        wall(c, 21, "concrete")
    for r in range(10, 15):
        wall(12, r, "concrete")
    for r in range(19, 25):
        wall(12, r, "concrete")
    tower(12, 15, "tesla")
    # the office trailer (timber, its windows glass) and the workshop
    # (white block, an iron roller door) at the corridors' east ends
    for c, r in rect(9, 10, 11, 12):
        wall(c, r, "glass" if r == 12 and c != 9 else "wood")
    for c, r in rect(9, 22, 11, 24):
        wall(c, r, "iron" if r == 22 and c != 9 else "brick")
    # the concrete apron round the starts
    for c, r in rect(4, 14, 10, 20):
        put(c, r, "road")
    put(*START, "start")
    put(*START2, "start2")
    # the stores: supplies round the court, an air strike at the end of
    # each corridor, rockets in the workshop's corridor
    pickup(3, 15, "health")
    pickup(3, 19, "ammo")
    pickup(11, 14, "minigun")
    pickup(11, 20, "plasma")
    pickup(2, 11, "air_jet")
    pickup(5, 11, "health")
    pickup(2, 23, "air_heli")
    pickup(5, 23, "missiles")
    pickup(15, 10, "ammo")
    pickup(15, 24, "shield")
    pickup(14, 17, "tower_pack")
    put(2, 14, "palm")
    put(2, 20, "palm")


def forecourt():
    """Outside the office's east wall: a sandbag either side of both east
    gates, floodlights on the road between them, and the grenades and a
    second rack of missiles out along the wall - lobbed over it, a grenade
    reaches a tank before the tank reaches a gate. Wrecks shoved up against
    the south wall."""
    for c, r in ((17, 10), (17, 14), (17, 20), (17, 24)):
        put(c, r, "sandbag")
    put(17, 16, "lamp")
    put(17, 18, "lamp")
    keep_clear(17, 11, 22, 13)
    keep_clear(17, 21, 22, 23)
    keep_clear(17, 15, 18, 19)
    pickup(20, 8, "grenades")
    pickup(20, 26, "missiles")
    car(2, 26, 9)
    car(12, 26, 11)


def drum_traps():
    """Heaps of drums beside the gates' ways in, for whoever is quicker to
    set them off: in the corner of the tyre bank and the wire on the
    north-west way, the container yard's corners, the bays and the far end
    of the oil trail. Then wooden crates, and drums at the back of the
    bays, where no wave passes. No concrete barrier: six hulls
    heavy, it barely creeps under a hull's push, and the router drives at
    it as if its cell were open, so near a way in it stalls the tank."""
    drums([(22, 1), (23, 1), (22, 2)], 191)
    drums([(51, 5), (52, 5), (52, 6), (51, 6)], 193)
    drums([(65, 7), (66, 7), (65, 8)], 197)
    drums([(23, 31), (24, 31), (23, 32), (24, 32), (25, 32)], 211)
    drums([(56, 29), (56, 30), (57, 31)], 223)
    for c, r in ((63, 12), (66, 19), (42, 26), (20, 30)):
        loose_prop(c, r, "wood_crate")
    # and where no wave passes: at the back of the bays
    drums([(46, 30), (47, 30), (49, 30), (50, 30), (61, 30), (62, 30)], 233)


def tyre_stacks():
    """North of the office: the tyre stacks, a bank of tyres along the wire
    stacked deeper in three places, the laser, a flamethrower and health
    laid out in front of it."""
    for c, r in rect(1, 1, 21, 2):
        put(c, r, "tyres")
    for c0, c1 in ((3, 4), (11, 13), (18, 19)):
        for c in range(c0, c1 + 1):
            put(c, 3, "tyres")
    pickup(7, 5, "laser")
    pickup(10, 6, "flamethrower")
    pickup(15, 5, "health")
    keep_clear(1, 1, 22, 8)


def filling_station():
    """South of the office: the old filling station, its pumps against the
    wire with fuel drums either side and spilt oil in front, the kiosk
    (white block, glass windows) with a stash inside, and the oil running on
    east along the wire to the drums by the car lot's south gate."""
    for c in (4, 8, 12):
        wall(c, 32, "iron")
        drum(c - 1, 32, "fuel")
        drum(c + 1, 32, "fuel")
    oil([(c, 31) for c in range(2, 23)])
    for c, r in outline(15, 26, 19, 29):
        if c == 15 and r in (27, 28):
            continue
        wall(c, r, "glass" if r == 26 and c in (16, 18) else "brick")
    pickup(17, 27, "plasma")
    pickup(17, 28, "health")
    pickup(13, 28, "flamethrower")
    keep_clear(1, 26, 14, 32)
    keep_clear(15, 26, 19, 31)


def crane():
    """The crane in the north: a concrete mast, its counter-jib and the
    counterweight west, the lattice jib east and the magnet down at its end
    over a heap of wrecks. An enemy gun tower rides the counter-jib, over
    the cab's stash at the mast's foot - the gauss rail and health."""
    for c, r in rect(23, 5, 24, 6):
        wall(c, r, "concrete")  # counterweight
    tower(25, 6, "gun_tower", enemy=True)
    put(26, 6, "tank_trap")  # counter-jib
    for c, r in rect(27, 5, 28, 7):
        wall(c, r, "concrete")  # mast
    for c in range(29, 36):
        if c % 2:
            wall(c, 6, "iron")
        else:
            put(c, 6, "tank_trap")  # jib
    for c, r in rect(36, 6, 37, 7):
        wall(c, r, "iron")  # magnet
    heap(37.5, 9.2, 2.6, 1.3, 13, table=WRECKS)
    pickup(30, 8, "gauss_rail")
    pickup(26, 8, "health")
    pickup(41, 10, "minigun")
    keep_clear(22, 2, 37, 4)
    keep_clear(25, 7, 31, 8)


def haul_road():
    """The haul road from the office's east gates through the yard to the
    crusher's mouth, puddles of oil the trucks spilt along it - nothing to
    a hull until a blast or a burning neighbour lights one."""
    for c, r in rect(19, 16, 51, 17):
        if (c, r) not in cells:
            put(c, r, "road")
    for c, r in rect(19, 11, 21, 23):
        if (c, r) not in cells:
            put(c, r, "road")
    for c, r in ((29, 16), (30, 16), (30, 17), (39, 17), (40, 17), (40, 16), (46, 16), (47, 16), (47, 17)):
        put(c, r, "oil")
    keep_clear(19, 15, 52, 18)


def scrap_heaps():
    """Heaps of scrap either side of the haul road - iron and pallets, the
    white goods by the office - each with a crate in its lee, drums on the
    far rim of the one under the crane; two more out in the east yard. No
    drum stands on a heap's rim by the road: a hull clipping the heap would
    shove it into the junk and stick there."""
    heap(26, 12.5, 2.6, 1.4, 51)
    heap(27, 20, 2.8, 1.2, 53, table=WHITE_GOODS)
    heap(37, 12.5, 2.4, 1.5, 57)
    heap(38, 20, 2.4, 1.2, 59)
    heap(46.5, 12, 1.8, 1.3, 61)
    heap(46.5, 20, 1.6, 1.1, 67)
    heap(50, 24, 2.2, 1.0, 69)
    heap(57, 10, 1.6, 0.9, 71, table=WRECKS)
    pickup(30, 13, "missiles")
    pickup(31, 20, "health")
    pickup(40, 13, "plasma")
    pickup(34, 20, "grenades")
    pickup(49, 12, "ammo")
    pickup(49, 20, "minigun")
    drums([(36, 10), (35, 10)], 71)


def crusher():
    """The crusher at the haul road's end: a hydraulic press, its jaws iron
    on a concrete frame, its mouth open to the road with the sonic hammer
    in it on a floor of spilt oil between two fuel drums of hydraulic
    fluid, the bales stacked behind it. Two enemy gun towers flank the
    mouth."""
    for c in range(53, 58):
        wall(c, 14, "iron")
        wall(c, 18, "iron")
    for c, r in rect(56, 15, 57, 17):
        wall(c, r, "iron")  # the ram
    for r in range(13, 20):
        wall(58, r, "concrete")
    for c, r in ((52, 13), (52, 19), (53, 13), (53, 19)):
        wall(c, r, "concrete")
    oil([(53, 15), (54, 15), (53, 17), (54, 17), (55, 16)])
    pickup(54, 16, "sonic_hammer")
    drums([(55, 15), (55, 17)], 79, fuel=1.0)
    tower(51, 12, "gun_tower", enemy=True)
    tower(51, 20, "gun_tower", enemy=True)
    # the bales: crushed cars in two-by-two cubes
    for c0, r0 in ((61, 13), (61, 18), (64, 15), (62, 21)):
        for c, r in rect(c0, r0, c0 + 1, r0 + 1):
            wall(c, r, PAINT[int(hash01(c, r, 81) * 3) % 3])
    pickup(63, 17, "ammo")
    pickup(57, 22, "health")
    keep_clear(50, 14, 55, 18)


def containers():
    """The container yard in the north-east: two pairs of shipping
    containers, each pair closed at one end by a third so the alley between
    is a stash - the gravity well and health in the west one, the
    helicopter strike and plasma in the east - an enemy tesla on a corner
    at each alley's mouth, drums at its back."""
    for c0, c1, material in ((42, 48, "brick"), (59, 64, "wood")):
        for c, r in rect(c0, 2, c1, 3):
            wall(c, r, material)
    for c0, c1, material in ((42, 48, "iron"), (59, 64, "brick")):
        for c, r in rect(c0, 7, c1, 8):
            wall(c, r, material)
    for r in range(4, 7):
        wall(42, r, "wood")
        wall(65, r, "iron")
    pickup(45, 5, "gravity_well")
    pickup(47, 5, "health")
    pickup(60, 5, "air_heli")
    pickup(62, 5, "plasma")
    tower(48, 7, "tesla", enemy=True)
    tower(59, 3, "tesla", enemy=True)
    drums([(43, 4), (43, 6), (64, 4), (64, 6)], 83)
    keep_clear(43, 4, 48, 6)
    keep_clear(59, 4, 64, 6)


def sorting_bays():
    """The sorting bays in the south-east: concrete bays four cells wide,
    open to the north, each with its heap of one sort of scrap in a back
    corner and a crate in front of it, an enemy bio slush between them and
    drums beside it."""
    for c0 in (43, 48, 60):
        for c, r in outline(c0, 27, c0 + 5, 31):
            if r == 27 and c0 < c < c0 + 5:
                continue
            wall(c, r, "concrete")
    for c, r, material in ((44, 30, "iron"), (45, 30, "iron"), (44, 29, "iron"),
                           (52, 30, "wood"), (51, 30, "wood"), (52, 29, "wood"),
                           (64, 30, "brick"), (63, 30, "brick"), (64, 29, "brick")):
        wall(c, r, material)
    pickup(46, 28, "missiles")
    pickup(50, 28, "minigun")
    pickup(62, 28, "health")
    tower(55, 26, "bio_slush", enemy=True)
    drums([(54, 30), (55, 30), (54, 31)], 89)
    for c0 in (44, 49, 61):
        keep_clear(c0, 27, c0 + 3, 29)


def car_lot():
    """The car lot in the south: a row of wrecks with the south gate's way
    through it, a wrecked bus and stacks of wrecks beyond, the EMP by the
    bus, a shield and the FPV swarm in the aisles."""
    for c in (21, 25, 34, 38):
        car(c, 25, 97)
    car(22, 28, 99)
    car(22, 29, 101)
    car(41, 27, 103, vertical=True)
    # the bus, glazed down its north side
    for c in range(35, 40):
        wall(c, 29, "glass" if c % 2 else "wood")
        wall(c, 30, "wood")
    pickup(37, 28, "emp_burst")
    pickup(27, 27, "shield")
    pickup(39, 27, "fpv_swarm")
    pickup(20, 29, "speedup")
    pickup(32, 27, "health")
    pickup(24, 23, "ammo")
    keep_clear(28, 22, 33, 31)
    keep_clear(21, 26, 40, 28)


def junk_scatter(avoid):
    """Junk strewn over the open ground in drifts: wrecked cars, clumps of
    scrap, tyre stacks, a pair of tank traps - each kept only where it cuts
    no ground off and pinches no way to one cell, and off `avoid`, the
    waves' ways in and a cell either side, so no corner of junk stands in a
    wave's road for a hull to clip."""
    def spot_ok(p):
        return (inb(*p) and p not in cells and p not in reserved and p not in avoid
                and min(math.hypot(p[0] - s[0], p[1] - s[1]) for s in (START, START2)) >= 6)

    anchors = []
    for r in range(2, H - 2):
        for c in range(2, W - 2):
            if not spot_ok((c, r)):
                continue
            d = fbm(c / 5, r / 5, 151)
            if hash01(c, r, 157) < min(0.6, max(0.0, (d - 0.3) * 1.6)):
                anchors.append((c, r))
    placed = 0
    for c, r in anchors:
        u = hash01(c, r, 163)
        if u < 0.26:
            vertical = hash01(c, r, 165) < 0.4
            body = PAINT[int(hash01(c, r, 167) * 3) % 3]
            run = [(c, r + i) if vertical else (c + i, r) for i in range(3)]
            group = {p: obj("wall", material="glass" if i == 1 else body) for i, p in enumerate(run)}
        elif u < 0.6:
            shape = ((0, 0), (1, 0), (0, 1)) if hash01(c, r, 169) < 0.5 else ((0, 0), (1, 0), (1, 1), (2, 1))
            group = {(c + dc, r + dr): obj("wall", material=pick(SCRAP, hash01(c + dc, r + dr, 171)))
                     for dc, dr in shape}
        elif u < 0.88:
            n = 1 + int(hash01(c, r, 173) * 3)
            group = {(c + i, r): obj("tyres") for i in range(n)}
        else:
            group = {(c, r): obj("tank_trap"), (c + 1, r + 1): obj("tank_trap")}
        if all(spot_ok(p) for p in group) and try_fixed(group):
            reserved.update(group)
            placed += 1
    return placed


def loose_scatter(routes):
    """Loose drums and wooden crates over the junk's drifts, never on a
    wave's way in (`routes`) and no drum near a start."""
    loose = (("drum", 64), ("wood_crate", 36))
    placed = 0
    for r in range(2, H - 2):
        for c in range(2, W - 2):
            p = (c, r)
            if p in cells or p in reserved or p in routes:
                continue
            if min(math.hypot(c - s[0], r - s[1]) for s in (START, START2)) < 8:
                continue
            d = fbm(c / 5, r / 5, 151)
            if hash01(c, r, 177) >= min(0.12, max(0.0, (d - 0.45) * 0.5)):
                continue
            kind = pick(loose, hash01(c, r, 179))
            if kind == "drum":
                put(*p, "barrel", drum="fuel" if hash01(c, r, 181) < 0.35 else "oil")
                DRUMS.append(p)
            else:
                put(*p, kind)
            placed += 1
    return placed


def lamps():
    """Floodlights round the yard: at the crusher's mouth and the
    container yard."""
    for p in ((50, 15), (50, 18), (53, 9)):
        put(*p, "lamp")


WEEDS = (("tall_grass", 54), ("juniper", 12), ("bush", 8), ("autumn_bush", 16), ("berry_bush", 10))


def weeds():
    """Weeds through the open ground the yard left: long grass and scrub in
    noise patches - soft cover, so no route changes - but never round a
    start."""
    for r in range(2, H - 2):
        for c in range(2, W - 2):
            p = (c, r)
            if p in cells or p in roads:
                continue
            if COMPOUND[0] <= c <= COMPOUND[2] and COMPOUND[1] <= r <= COMPOUND[3]:
                continue
            if min(math.hypot(c - s[0], r - s[1]) for s in (START, START2)) < 4:
                continue
            d = fbm(c / 4, r / 4, 131)
            u = hash01(c, r, 137)
            if d > 0.62 and u < 0.6:
                put(c, r, pick(WEEDS, hash01(c, r, 139)))
            elif u < 0.025:
                put(c, r, "tall_grass")


# ---------------------------------------------------------------- the nav grid
#
# The game routes on a grid of 32 px cells whose centres sit half a cell off
# the map's (`pathfind::Grid`, `maplint`): a tile at map cell (c, r) blocks
# nav cells c-1..c by r-1..r. So two tiles one cell apart leave no nav cell
# between them, and a way the AI can take needs two empty map cells side by
# side. Everything below reasons on that grid, the way the linter does.

LOOSE = {"barrel", "hay", "wood_crate", "barrier"}
SOFT = {"road", "water", "pickup", "oil", "start", "start2", "gate"} | COVER_KINDS | LOOSE
GATE_LANES = set()


def blocks(p):
    """Whether map cell `p` holds something the nav grid treats as a wall."""
    if not inb(*p):
        return False
    k = kind_at(p)
    return k is not None and k not in SOFT


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


def narrow_near(c, r):
    """The one-wide nav cells round map cell (c, r): what a tile there
    could change."""
    out = set()
    for l in range(max(1, r - 3), min(H - 1, r + 3)):
        for k in range(max(1, c - 3), min(W - 1, c + 3)):
            if not nav_open(k, l):
                continue
            x_sealed = not nav_open(k - 1, l) and not nav_open(k + 1, l)
            y_sealed = not nav_open(k, l - 1) and not nav_open(k, l + 1)
            x_flows = nav_open(k - 1, l) or nav_open(k + 1, l)
            y_flows = nav_open(k, l - 1) or nav_open(k, l + 1)
            if (x_sealed and y_flows) or (y_sealed and x_flows):
                out.add((k, l))
    return out


REACH = []


def try_fixed(group):
    """Place a group of fixed tiles - `group` maps each cell to the inline
    table it holds - unless it would cut ground off the playfield (the
    playfield may lose only the nav cells the tiles themselves shut) or
    pinch a way to one cell. Whether it stayed."""
    if any(p in cells or p in reserved or not inb(*p) for p in group):
        return False
    if not REACH:
        REACH.append(nav_reach())
    before = REACH[0]
    narrow_before = set().union(*(narrow_near(*p) for p in group))
    cells.update(group)
    after = nav_reach()
    shut = len({n for p in group for n in nav_of(p) if n in before and not nav_open(*n)})
    narrow_after = set().union(*(narrow_near(*p) for p in group))
    if len(before) - len(after) > shut or narrow_after - narrow_before:
        for p in group:
            cells.pop(p, None)
        return False
    REACH[0] = after
    return True


def pockets():
    reach = nav_reach()
    return [(k, l) for l in range(H) for k in range(W) if nav_open(k, l) and (k, l) not in reach]


def fill_pockets():
    """Open ground the playfield never reaches gets a tyre stack, so no nav
    cell is left boxed in or cut off."""
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
                if kind_at(p) is None or kind_at(p) in COVER_KINDS:
                    put(*p, "tyres")
                    filled += 1
                    break
    return filled


def gate_aprons():
    """The lane a wave rolls in by: clear five cells in from every gate."""
    for gc, gr in GATES:
        if gr == 0:
            area = rect(gc - 1, 0, gc + 2, 5)
        elif gr == H - 1:
            area = rect(gc - 1, H - 6, gc + 2, H - 1)
        elif gc == 0:
            area = rect(0, gr - 1, 5, gr + 2)
        else:
            area = rect(W - 6, gr - 1, W - 1, gr + 2)
        for p in list(area):
            k = kind_at(p)
            if k is None or k in COVER_KINDS or k == "fence":
                clear(*p)
            reserved.add(p)


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


# ---------------------------------------------------------------- the walk


def walk_costs(start=START):
    first = next(n for n in nav_of(start) if nav_open(*n))
    cost = {first: 0}
    heap_ = [(0, first)]
    while heap_:
        d, (k, l) = heapq.heappop(heap_)
        if d > cost.get((k, l), 1 << 30):
            continue
        for n in ((k + 1, l), (k - 1, l), (k, l + 1), (k, l - 1)):
            if not nav_open(*n):
                continue
            nd = d + 1
            if nd < cost.get(n, 1 << 30):
                cost[n] = nd
                heapq.heappush(heap_, (nd, n))
    return cost


def wave_routes():
    """The map cells on each gate's cheapest walk to either start - the way
    a wave comes in."""
    cells_on = set()
    for s in (START, START2):
        cost = walk_costs(s)
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


def widen(area, n=1):
    """`area` and every cell within `n` of it."""
    return {(c + dc, r + dr) for c, r in area for dc in range(-n, n + 1) for dr in range(-n, n + 1)}


def loose_on_routes():
    on = wave_routes()
    return sorted((c, r) for (c, r) in cells if kind_at((c, r)) in LOOSE and (c, r) in on)


# ---------------------------------------------------------------- sight lines

# what stops a shot and blinds a line for good enough to count as cover:
# every wall but glass, and the towers
HARD = {"iron", "concrete", "brick", "wood"}


def hard(p):
    k = kind_at(p)
    return (k == "wall" and material_at(p) in HARD) or k in TOWERS


def sees(a, b):
    """Whether the straight line between the centres of cells `a` and `b`
    crosses no hard cell (sampled finely, ends excluded)."""
    (x0, y0), (x1, y1) = (a[0] + 0.5, a[1] + 0.5), (b[0] + 0.5, b[1] + 0.5)
    n = int(max(abs(x1 - x0), abs(y1 - y0)) * 8) + 1
    for i in range(1, n):
        t = i / n
        p = (math.floor(x0 + (x1 - x0) * t), math.floor(y0 + (y1 - y0) * t))
        if p != a and p != b and hard(p):
            return False
    return True


def shots_at_starts():
    """Open cells outside the office's walls with a straight line to a
    start."""
    c0, r0, c1, r1 = COMPOUND
    out = []
    for r in range(H):
        for c in range(W):
            if c0 <= c <= c1 and r0 <= r <= r1:
                continue
            if blocks((c, r)):
                continue
            for s in (START, START2):
                if sees((c, r), s):
                    out.append(((c, r), s))
    return out


# ---------------------------------------------------------------- the map


def build():
    the_wire()
    yard_office()
    forecourt()
    tyre_stacks()
    filling_station()
    crane()
    haul_road()
    scrap_heaps()
    crusher()
    containers()
    sorting_bays()
    car_lot()
    drum_traps()
    lamps()
    gate_aprons()
    for gc, gr in GATES:
        put(gc, gr, "gate")
        GATE_LANES.update(lane(gc, gr))
    junk_scatter(widen(wave_routes()))
    routes = wave_routes()
    place_loose(widen(routes, 1))
    loose_scatter(widen(routes, 2))
    weeds()
    return fill_pockets()


HEADER = """\
# scrapyard - Scrapyard, level 12: a junkyard behind the wire on the desert.
# Written by tools/gen_scrapyard.py - change the generator and run it again
# rather than editing this file; re-saving from the map editor would drop
# these comments. Level tables as dotted keys on purpose - a `[spawn]` header
# would swallow every `cells.` line after it.
#
# The breather before Vulkan: short, loud and full of guns. Destroy on the
# waves plan: four waves of 5 to 8 tanks, 26 in all, light ones first and
# medium ones last, rolling in through six gates in the wire that runs round
# the whole yard - two on the north edge, past the crane's magnet and
# through the container yard, two on the east either side of the crusher's
# bales, two on the south through the car lot and past the sorting bays.
# A clear sky, dusk (the floodlights come on) or heat haze. 68 x 34 cells
# of desert, followed by the camera.
#
# The players start in the yard office in the west: a compound of junk
# walls - sheet iron, pallets, white goods - on a concrete footing, with a
# gate three cells wide in its north and south walls and two in its east
# wall, each between concrete posts, and a concrete baffle inside every
# gate, so nothing outside the walls has a straight shot at either start: a
# tank coming in turns along a wall and reaches the court only round a
# baffle's end. Gun towers on the east gates' outer posts, bio slush on the
# north and south gates', a tesla at the gap between the east baffles. The
# office trailer and the workshop close the north and south corridors. The
# stores: health, ammo, a minigun, plasma, a tower pack and a shield round
# the court, the jet strike and the helicopter strike at the corridors'
# ends, missiles by the workshop; outside the east wall, sandbags and
# floodlights at the gates, the grenades by the north-east corner and more
# missiles by the south-east.
#
# Round it the yard: the tyre stacks banked along the wire to the north,
# the laser, a flamethrower and health in front of them; the old filling
# station to the south, its pumps against the wire between fuel drums,
# spilt oil running east along the wire to a heap of drums, its kiosk
# holding plasma and health, a second flamethrower by the pumps. The crane
# in the north - a concrete mast, its counterweight and counter-jib, a
# lattice jib and the magnet down over a heap of wrecks - with an enemy gun
# tower on the counter-jib over the cab's stash (the gauss rail, health)
# and a minigun past the magnet. The haul road runs east from the office
# through heaps of scrap, oil puddles spilt along it, to the crusher: a
# hydraulic press, iron jaws on a concrete frame, the sonic hammer in its
# mouth on spilt oil between two fuel drums, an enemy gun tower either side
# of the mouth, the bales stacked behind with ammo among them and health
# past them. The heaps along the road hide missiles, plasma, grenades,
# health, ammo and a minigun. In the north-east
# the container yard: two pairs of containers, each pair's alley a stash -
# the gravity well and health in one, the helicopter strike and plasma in
# the other - under an enemy tesla. In the south-east the sorting bays,
# concrete, each with its heap of iron, timber or white goods and a crate
# (missiles, a minigun, health), an enemy bio slush between them. In the
# south the car lot: a row of wrecks with the south gate's way through it,
# a wrecked bus with the EMP beside it, the FPV swarm, a shield, health and
# ammo in the aisles. Junk everywhere between - wrecks, scrap, tyre stacks,
# tank traps, wooden crates - and weeds through it, 40 oil and fuel drums
# in heaps beside the ways in, 25 weapon crates and ten health packs.
"""


def write(path):
    lines = [HEADER, "version = 1",
             'tank = "ravager"',
             'tank2 = "breaker"',
             'theme = "desert"',
             'weather = ["clear", "dusk", "heat_haze"]',
             f"size = [{W}, {H}]",
             'mission.kind = "destroy"',
             'spawn.kind = "waves"',
             "spawn.waves = 4",
             "spawn.size = 5",
             "spawn.growth = 1",
             'spawn.tier_start = "light"',
             'spawn.tier_end = "medium"',
             ""]
    for (c, r) in sorted(cells, key=lambda p: (p[1], p[0])):
        lines.append(f'cells."{c},{r}" = {cells[(c, r)]}')
    with open(path, "w") as f:
        f.write("\n".join(lines) + "\n")


def check():
    problems = []
    for gc, gr in GATES:
        if not (gc in (0, W - 1) or gr in (0, H - 1)):
            problems.append(f"gate {gc},{gr} is not on the edge")
    reach = nav_reach()
    for p in (START, START2):
        if not reachable(p, reach):
            problems.append(f"start {p} cannot be reached")
    for c, r, kind in pickups:
        if not reachable((c, r), reach):
            problems.append(f"{kind} at {c},{r} cannot be reached")
        if kind_at((c, r)) != "pickup":
            problems.append(f"{kind} at {c},{r} was built over")
    for gc, gr in GATES:
        if not all(nav_open(*n) for n in lane(gc, gr)):
            problems.append(f"gate {gc},{gr}: its lane is shut")
        elif lane(gc, gr)[-1] not in reach:
            problems.append(f"gate {gc},{gr} is cut off")
    for k, l in boxed_in():
        problems.append(f"nav cell {k},{l} is boxed in")
    for k, l in pockets():
        problems.append(f"nav cell {k},{l} is cut off")
    for c, r in DRUMS:
        if kind_at((c, r)) != "barrel":
            problems.append(f"drum at {c},{r} was built over")
        if min(math.hypot(c - s[0], r - s[1]) for s in (START, START2)) < 7:
            problems.append(f"drum at {c},{r} is by a start")
    for c, r in loose_on_routes():
        problems.append(f"{kind_at((c, r))} at {c},{r} stands on a wave's way in")
    for (c, r), s in shots_at_starts():
        problems.append(f"{c},{r} has a straight shot at the start at {s}")
    return problems


ASCII = {
    'wall': '#', 'road': '=', 'water': '~', 'start': '1', 'start2': '2',
    'pickup': 'P', 'sandbag': 's', 'fence': 'f', 'tree': 'T', 'pine': 'T', 'spruce': 'T', 'scots': 'T',
    'fir': 'T', 'palm': 'Y', 'snag': 'x', 'willow': 'W', 'birch': 'b', 'tall_grass': '"', 'bush': 'o',
    'berry_bush': 'o', 'autumn_bush': 'o', 'juniper': 'o', 'fern': ',', 'reeds': '|', 'gate': 'G',
    'tesla': 'Z', 'gun_tower': 'K', 'bio_slush': 'Q', 'barrel': 'D', 'hay': 'h', 'tyres': 'O',
    'wood_crate': 'X', 'barrier': 'n', 'tank_trap': '*', 'lamp': 'L', 'oil': ':', 'target': '@',
}


def ascii_map(marked=()):
    print("    " + "".join(str(c // 10 % 10) for c in range(W)))
    print("    " + "".join(str(c % 10) for c in range(W)))
    for r in range(H):
        row = []
        for c in range(W):
            k = kind_at((c, r))
            ch = ASCII.get(k, '?') if k else ('+' if (c, r) in marked else '.')
            if k == 'wall':
                ch = {'brick': 'B', 'iron': 'I', 'wood': 'w', 'concrete': 'C', 'glass': 'g'}.get(material_at((c, r)), '#')
            if k == 'barrel':
                ch = 'F' if 'fuel' in cells[(c, r)] else 'D'
            if k in TOWERS and 'enemy' in cells[(c, r)]:
                ch = ch.lower()
            row.append(ch)
        print(f"{r:3d} " + "".join(row))


def nav_map():
    reach = nav_reach()
    thin = narrow()
    print("nav: '.' reached, '+' one wide, '!' open but cut off, ' ' shut")
    print("    " + "".join(str(c // 10 % 10) for c in range(W)))
    print("    " + "".join(str(c % 10) for c in range(W)))
    for l in range(H):
        print(f"{l:3d} " + "".join("+" if (k, l) in thin else "." if (k, l) in reach else "!" if nav_open(k, l) else " "
                                   for k in range(W)))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("-o", "--out", default="maps/scrapyard.toml")
    ap.add_argument("--stats", action="store_true")
    ap.add_argument("--ascii", action="store_true")
    ap.add_argument("--nav", action="store_true")
    ap.add_argument("--routes", action="store_true", help="with --ascii, mark the waves' ways in with '+'")
    a = ap.parse_args()
    filled = build()
    if a.ascii:
        ascii_map(wave_routes() if a.routes else ())
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
        print(f"{len(cells)} cells, {filled} pocket cells filled, {len(narrow())} one-wide nav cells")
        for k, n in sorted(kinds.items(), key=lambda kv: -kv[1]):
            print(f"  {k:12} {n}")
        cost = walk_costs()
        for gc, gr in GATES:
            inside = lane(gc, gr)[-1]
            c = cost.get(inside)
            print(f"  gate {gc},{gr}: walk {c} cells = {c * 32 / 160:.1f} s" if c else f"  gate {gc},{gr}: no walk")
        by = {}
        for c, r, kind in pickups:
            by.setdefault(kind, []).append((c, r))
        for kind, at in sorted(by.items()):
            print(f"  {kind:13} {len(at)}: {' '.join(f'({c},{r})' for c, r in at)}")
        print(f"  drums {len(DRUMS)}")
    if problems:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
