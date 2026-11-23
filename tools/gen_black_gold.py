"""Write maps/black-gold.toml: Black Gold, level 14 - an oil refinery in the
desert wired with fuses, 97 x 48 cells, laid out set piece by set piece.

  python3 tools/gen_black_gold.py [-o maps/black-gold.toml] [--stats] [--ascii] [--nav]

A map this size is too big to lay out by hand, so this script is the
source: change it and run it again rather than editing the TOML (a
re-save from the map editor would drop the header too). Nothing is drawn
at random - the noise is hashed from cell positions - so the script
writes the same file every time. `--ascii` prints the map a character a
cell, `--nav` the nav grid the AI routes on (what is reached, what is one
cell wide), `--stats` the counts, each gate's walk to the start and every
crate. The checks run every time and fail the run: every pickup, start
and gate lane reached, no boxed-in nav cell, no straight shot from outside
the depot at either start, no drum by a start, no weapon an enemy would
fetch inside a gate's leash, and no loose drum or prop on a wave's
cheapest way in (the router drives straight through loose props, and a
hull that meets one on its way has to shove it).

What it lays out (the header written into the map says how it plays):

- the Wildcat Depot in the west, the players' walled base and start:
  concrete walls with a gate north, east and south, a baffle wall inside
  every gate so nothing outside has a straight shot at the starts, gun
  towers, teslas and bio slush, and the stores - supplies, air strikes,
  missiles and grenades, plasma and the minigun - inside; its fuel dump
  and the gravity well north-west of it, the truck park south-west;
- the pipe alley, the haul road east from the depot between two
  pipelines, an oil fuse down its length;
- the oil field either side of it: Oiltown (the drillers' shacks, a
  cantina, a well under palms) and pump jacks to the north, the settling
  pond, more pump jacks and an old wooden derrick to the south - every
  jack's sump on a flow line to the next, so the field goes up in a chain;
- the refinery in the east, the enemy's: the tank farm (three storage
  tanks on a fuse ring) behind the north pipe rack, the process unit (the
  distillation columns, the control room with the enemy's stash under
  four towers, the furnace and the flare stack), and behind the south
  pipe rack the loading terminal (a train of tank cars on the siding, the
  loading rack and the fenced drum yard);
- the desert everywhere else: dunes of dry grass, juniper and scrub,
  palms and dead snags.
"""

import argparse
import heapq
import math
from collections import deque

W, H = 97, 48

# (col, row) -> inline TOML table text
cells = {}
# cells the scrub pass must leave alone
reserved = set()
water = set()
roads = set()

DEPOT = (0, 8, 25, 39)
START = (6, 16)
START2 = (6, 21)

GATES = [
    (60, 0), (85, 0),  # north: by Oiltown's road, between the storage tanks
    (96, 30),  # east: the refinery's road
    (60, 47), (85, 47),  # south: by the settling pond's road, the terminal
]

# what an enemy turns off its road to fetch lies beyond every gate's
# leash (`enemy_leash_px`, 640 px = 20 cells; `Tank::wants_pickup`): the
# flamethrower, grenades, air strikes and the heat shield are the players'
# alone, and health and ammo are left to lie anywhere, as on every field map
FETCHED = {
    "laser", "minigun", "plasma", "missiles", "sonic_hammer", "emp_burst",
    "gauss_rail", "fpv_swarm", "rod_from_god", "gravity_well", "tower_pack",
    "speedup", "shield",
}
pickups = []  # (c, r, kind)
DRUMS = []

TREE_KINDS = {"tree", "pine", "spruce", "scots", "fir", "birch", "willow", "palm", "snag"}
COVER_KINDS = {"tall_grass", "bush", "berry_bush", "juniper", "fern", "autumn_bush", "reeds"}
TOWER_KINDS = {"gun_tower", "tesla", "bio_slush"}


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


def hline(c0, c1, r):
    return [(c, r) for c in range(c0, c1 + 1)]


def vline(c, r0, r1):
    return [(c, r) for r in range(r0, r1 + 1)]


# the set pieces' open ground, which the scrub keeps off
OPEN_GROUND = set()


def clear_rect(c0, r0, c1, r1):
    for c, r in list(rect(c0, r0, c1, r1)):
        if (c, r) not in roads and (c, r) not in water:
            clear(c, r)
        reserved.add((c, r))
        OPEN_GROUND.add((c, r))


def keep_clear(c0, r0, c1, r1):
    """A walkway between set pieces: the scrub leaves it open."""
    for p in rect(c0, r0, c1, r1):
        reserved.add(p)
        OPEN_GROUND.add(p)


def pickup(c, r, kind):
    assert inb(c, r), (c, r, kind)
    assert (c, r) not in water and (c, r) not in roads, f"{kind} at {c},{r} would cover water or road"
    assert kind_at((c, r)) is None or kind_at((c, r)) in COVER_KINDS, f"{kind} at {c},{r} lands on {kind_at((c, r))}"
    put(c, r, "pickup", pickup=kind)
    pickups.append((c, r, kind))


def drum(c, r, kind):
    put(c, r, "barrel", drum=kind)
    DRUMS.append((c, r))


def drums(spots, salt, fuel=0.35):
    """Drums at `spots`, each oil or fuel by a hash of its cell: `fuel` of
    them the grey fuel drums that go off harder and launch when chained."""
    for c, r in spots:
        drum(c, r, "fuel" if hash01(c, r, salt) < fuel else "oil")


def oil(points):
    """An oil trail through `points`: a fuse a blast or a fire runs along,
    setting off every drum it reaches. It gives way to what stands."""
    for c, r in points:
        if inb(c, r) and (c, r) not in cells:
            put(c, r, "oil")


def tower(c, r, kind, enemy=False):
    if enemy:
        put(c, r, kind, side="enemy")
    else:
        put(c, r, kind)


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
                if inb(c + dc, r + dr) and kind_at((c + dc, r + dr)) in {None, "oil"} | COVER_KINDS:
                    put(c + dc, r + dr, "road")
    for c, r in path:
        for dc in range(-margin, width + margin):
            for dr in range(-margin, width + margin):
                if inb(c + dc, r + dr):
                    reserved.add((c + dc, r + dr))
                    OPEN_GROUND.add((c + dc, r + dr))
    return path


def blob(cx, cy, rx, ry, salt):
    """A pond: an ellipse with a ragged shore."""
    out = []
    for r in range(int(cy - ry) - 2, int(cy + ry) + 3):
        for c in range(int(cx - rx) - 2, int(cx + rx) + 3):
            if not inb(c, r):
                continue
            dx, dy = (c - cx) / rx, (r - cy) / ry
            wobble = 0.4 * (fbm(c / 3, r / 3, salt) - 0.5)
            if dx * dx + dy * dy <= 1.0 + wobble:
                put(c, r, "water")
                out.append((c, r))
    return out


# ---------------------------------------------------------------- the depot


def depot():
    """The Wildcat Depot, the players' walled base against the west edge:
    concrete walls with a gate four cells wide north, east and south, and
    three cells inside every gate a baffle wall two cells wider than it,
    so nothing outside has a straight shot at the starts and a tank coming
    in turns along the walls. Gun towers on the two east corners and in
    the east baffle, a tesla in the north and the south baffle, bio slush
    on the north and south walls; the stores in the yard."""
    c0, r0, c1, r1 = DEPOT
    clear_rect(c0, r0, c1, r1)
    BARE.update(rect(c0, r0, c1, r1))
    gates = set(rect(12, r0, 15, r0)) | set(rect(12, r1, 15, r1)) | set(rect(c1, 22, c1, 25))
    # the west side is the field's own edge
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) not in gates and (c > c0 or r in (r0, r1)):
            wall(c, r, "concrete")
    # the baffles, four cells in from their gates; the north and the south
    # one run west to leave a lane three cells wide by the field's edge,
    # and the east one south to the south one's end, so from every gate
    # one way round is the shorter and a tank never wavers between two
    for p in hline(4, 18, r0 + 4) + hline(4, 18, r1 - 4) + vline(c1 - 4, 19, r1 - 5):
        wall(*p, "concrete")
    tower(c1, r0, "gun_tower")
    tower(c1, r1, "gun_tower")
    tower(c1 - 4, 23, "tesla")
    tower(13, r0 + 4, "tesla")
    tower(14, r1 - 4, "tesla")
    tower(20, r0, "bio_slush")
    tower(20, r1, "bio_slush")
    put(*START, "start")
    put(*START2, "start2")
    # the stores: supplies by the west wall, the specials across the yard
    pickup(2, 15, "health")
    pickup(2, 17, "ammo")
    pickup(2, 30, "ammo")
    pickup(2, 32, "health")
    pickup(9, 18, "tower_pack")
    pickup(9, 29, "shield")
    pickup(12, 23, "minigun")
    pickup(12, 24, "plasma")
    pickup(16, 17, "air_jet")
    pickup(16, 30, "air_heli")
    pickup(18, 21, "missiles")
    pickup(18, 26, "grenades")
    pickup(4, 10, "heat_shield")
    pickup(4, 37, "health")
    pickup(23, 10, "ammo")
    # tyres stacked in the east corners, the depot's trucks parked against
    # the south baffle, and the helipad: a square of packed gravel with the
    # helicopter strike on it
    for p in ((24, 9), (24, 38)):
        put(*p, "tyres")
    for c, r in hline(6, 8, r1 - 5) + hline(11, 13, r1 - 5):
        wall(c, r, "iron")
    for p in outline(14, 28, 18, 32):
        if p != (16, 30):
            put(*p, "road")
    # the depot's supply pipelines along the north and the south road, so
    # a tank comes down the road to the gate and turns in, rather than
    # cutting across the sand at it
    for c in range(9, 33):
        wall(c, r0 - 4, "iron")
        wall(c, r1 + 4, "iron")
    # behind each pipeline a line of fuel drums on a fuse, closed off at
    # the west end: one shot and the road beside it goes up
    for c in (11, 15, 19, 23, 27):
        drums(((c, r0 - 6), (c + 1, r0 - 6)), 15, fuel=0.5)
        drums(((c, r1 + 6), (c + 1, r1 + 6)), 16, fuel=0.5)
    oil(hline(10, 30, r0 - 5))
    oil(hline(10, 30, r1 + 5))
    for r in range(r1 + 5, H):
        put(8, r, "sandbag")
    # rockets and grenades round it
    pickup(10, 6, "grenades")
    pickup(6, 43, "missiles")
    for p in ((29, 14), (29, 32)):
        put(*p, "tank_trap")


def fuel_dump():
    """The depot's fuel dump north-west of it: drums behind a sandbag wall
    open to the east, far enough from the starts that a shell into them in
    the opening does not reach the yard, and the gravity well beside them."""
    keep_clear(0, 0, 11, 7)
    spots = [(1, 1), (2, 1), (3, 1), (1, 2), (2, 2), (3, 2), (5, 1), (6, 1), (5, 2), (6, 2)]
    drums(spots, 11, fuel=0.5)
    for p in vline(8, 0, 3):
        put(*p, "sandbag")
    pickup(5, 5, "gravity_well")
    pickup(2, 6, "health")


def truck_park():
    """South-west of the depot: tanker trucks parked nose to tail, tyres,
    health and ammo."""
    keep_clear(0, 40, 7, 47)
    for r in (41, 45):
        for c in (1, 2, 3):
            wall(c, r, "iron")
    put(6, 46, "tyres")
    pickup(4, 43, "health")
    pickup(5, 47, "ammo")


# ---------------------------------------------------------------- the pipe alley


def pipe_alley():
    """The haul road east from the depot between two pipelines, iron on
    rows 20 and 27 with gaps to cross by, and an oil fuse down the alley
    under the north pipe: lit, it burns the whole alley end to end."""
    north_gaps = set(range(33, 36)) | set(range(44, 47)) | set(range(54, 57))
    south_gaps = set(range(38, 41)) | set(range(49, 52))
    # the north pipeline runs on to the depot's wall, so a tank at the
    # alley's mouth has the east gate or the way south, never two ways
    # round the depot as like as each other
    for c in range(26, 60):
        if c not in north_gaps:
            wall(c, 20, "iron")
        if c not in south_gaps and c >= 30:
            wall(c, 27, "iron")
    oil(hline(31, 58, 21))
    oil(hline(31, 58, 26))
    keep_clear(26, 21, 62, 26)
    # plasma and a minigun dropped in the alley, for whoever gets there
    # first
    pickup(38, 22, "plasma")
    pickup(50, 25, "minigun")
    # the valve drums: fuel beside the crossings outside the pipes, the
    # alley's fuse run out to them through the gap
    for g0 in (33, 54):
        drums(((g0 - 1, 19),), 17, fuel=1.0)
        oil(((g0, 20), (g0, 19)))
    drums(((47, 19),), 17, fuel=1.0)
    oil(((46, 20), (46, 19)))
    for g0 in (38, 49):
        drums(((g0 + 3, 28),), 19, fuel=1.0)
        oil(((g0 + 2, 27), (g0 + 2, 28)))


# ---------------------------------------------------------------- the oil field


def pump_jack(c, r, salt, drum_at=(3, 1)):
    """A pump jack: the walking beam in iron, the post in wood, the sump
    under it an oil spill with a drum of crude beside it (`drum_at` from
    the beam's west end, by the sump unless it says otherwise)."""
    wall(c, r, "iron")
    wall(c + 1, r, "iron")
    wall(c + 2, r, "wood")
    oil(((c, r + 1), (c + 1, r + 1), (c + 2, r + 1)))
    drum(c + drum_at[0], r + drum_at[1], "fuel" if hash01(c, r, salt) < 0.4 else "oil")
    for p in rect(c - 1, r - 1, c + 4, r + 2):
        reserved.add(p)


def oiltown():
    """The drillers' town north of the alley: a timber shack with plasma,
    the cantina with the minigun, a well under palms, the water tower,
    hay, crates and tyres."""
    clear_rect(27, 8, 44, 18)
    # the shack: wood, door south
    for c, r in outline(29, 9, 34, 13):
        if not (r == 13 and c in (31, 32, 33)):
            wall(c, r, "wood")
    pickup(31, 11, "plasma")
    pickup(32, 11, "ammo")
    # the cantina: brick, glass windows, door west
    for c, r in outline(38, 9, 43, 14):
        if c == 38 and r in (11, 12, 13):
            continue
        wall(c, r, "glass" if (r == 9 and c in (40, 41)) or (c == 43 and r in (11, 12)) else "brick")
    pickup(40, 11, "minigun")
    pickup(41, 12, "health")
    # the well under palms, the water tower on its stilts
    for p in rect(31, 16, 32, 17):
        put(*p, "water")
    for p in ((27, 16), (36, 16), (34, 18)):
        put(*p, "palm")
    wall(41, 18, "iron")
    for p in ((28, 14), (29, 15)):
        put(*p, "hay")
    put(35, 10, "wood_crate")
    # the drillers' gun tower on the cantina's corner, their fuel store
    # behind the shack
    tower(43, 14, "gun_tower")
    drums(((28, 10), (28, 11), (27, 11)), 23, fuel=0.6)


def north_field():
    """Pump jacks between Oiltown and the refinery, each sump's oil on a
    flow line to the next: shoot one drum and the field goes up."""
    jacks = [(47, 9), (54, 12), (47, 15), (55, 16)]
    for i, (c, r) in enumerate(jacks):
        pump_jack(c, r, 21 + i, drum_at=(1, -1) if i == 1 else (3, 1))
    oil(polyline([(51, 10), (52, 10), (52, 11), (54, 11)]))
    oil(polyline([(46, 10), (46, 16)]))
    oil(polyline([(51, 16), (51, 17), (54, 17)]))
    oil(polyline([(57, 13), (57, 14), (58, 14), (58, 16)]))
    pickup(44, 13, "sonic_hammer")
    pickup(40, 16, "gravity_well")
    pickup(57, 9, "health")


def settling_pond():
    """The settling pond south of the alley, reeds on its banks, palms and
    the FPV swarm on its shore."""
    pond = blob(36, 36, 5.5, 3.2, 31)
    for c, r in pond:
        for dc, dr in ((0, -1), (0, 1), (-1, 0), (1, 0)):
            q = (c + dc, r + dr)
            if inb(*q) and q not in water and q not in cells and hash01(*q, 33) < 0.55:
                put(*q, "reeds")
    for c, r in list(rect(28, 30, 45, 40)):
        reserved.add((c, r))
    grove(((29, 33), (43, 39), (30, 39), (28, 36), (42, 30), (33, 31), (39, 31)), 35,
          (("palm", 4), ("tree", 1)))
    pickup(42, 32, "fpv_swarm")
    pickup(33, 33, "minigun")
    pickup(27, 36, "speedup")
    pickup(29, 36, "health")
    # the drillers' bio slush on the west shore
    tower(30, 30, "bio_slush")


def south_field():
    """Pump jacks and an old wooden derrick between the pond and the
    refinery, on their own flow lines."""
    jacks = [(46, 31), (54, 31), (46, 36), (54, 36)]
    for i, (c, r) in enumerate(jacks):
        pump_jack(c, r, 41 + i)
    oil(hline(50, 53, 32))
    oil(hline(50, 53, 37))
    oil(vline(45, 32, 37))
    # the old derrick in the dunes: its wooden base, drums of crude by it
    for p in rect(38, 45, 40, 46):
        wall(*p, "wood")
    drums(((41, 45), (41, 46)), 47, fuel=0.0)
    oil(((42, 45), (42, 46), (37, 45), (37, 46)))
    pickup(43, 33, "laser")
    # the derrick's cache in the dunes past the south road
    pickup(36, 44, "missiles")
    pickup(35, 46, "plasma")
    pickup(44, 44, "health")
    pickup(57, 34, "ammo")


# ---------------------------------------------------------------- the refinery


def storage_tank(cx, cy, rad=3.3):
    spot = set(disc(cx, cy, rad))
    for c, r in spot:
        rim = any((c + dc, r + dr) not in spot for dc in (-1, 0, 1) for dr in (-1, 0, 1))
        wall(c, r, "iron" if rim or (c, r) == (cx, cy) else "glass")


def tank_farm():
    """Three storage tanks behind the north pipe rack, fuel drums at
    their feet on a ring of oil that runs from tank to tank."""
    clear_rect(63, 0, 96, 12)
    for cx in (68, 78, 92):
        storage_tank(cx, 4)
    # a concrete firewall closes the farm's west end, so the waves from
    # the north-east gate take their road and leave the farm's drums be
    for r in range(0, 13):
        wall(63, r, "concrete")
    for r in range(0, 2):
        wall(64, r, "concrete")
    drums(((66, 8), (67, 8), (70, 8), (71, 8), (76, 8), (77, 8), (80, 8), (81, 8),
           (90, 8), (91, 8), (94, 8), (73, 2), (74, 2), (73, 3)), 51)
    oil(hline(64, 95, 9))
    oil(vline(74, 3, 8))
    oil(vline(83, 4, 8))
    tower(72, 1, "gun_tower", enemy=True)
    drums(((66, 11), (70, 11), (76, 11), (80, 11), (91, 11), (94, 11)), 53)
    pickup(73, 6, "flamethrower")
    pickup(74, 10, "health")
    pickup(83, 2, "grenades")
    pickup(95, 6, "air_heli")


def pipe_racks():
    """The north and south pipe racks closing the process unit off from
    the tank farm and the terminal, gaps for the roads."""
    for c in range(64, 97):
        if c == 83:
            tower(c, 12, "tesla", enemy=True)
        elif c not in (84, 85, 86, 87):
            wall(c, 12, "iron")
        if c not in (66, 67, 68, 76, 77, 78, 84, 85, 86, 87):
            wall(c, 34, "iron")


def process_unit():
    """The distillation columns, the control room with the enemy's stash
    under four towers, the furnace and the flare stack."""
    clear_rect(63, 13, 96, 33)
    # the distillation columns, drums of naphtha between them on a fuse
    for c in (63, 68):
        for p in rect(c, 16, c + 1, 17):
            wall(*p, "iron")
    drums(((65, 16), (66, 16), (67, 16), (65, 17), (67, 17)), 63)
    oil(((66, 17), (66, 18), (66, 15)))
    # the control room
    c0, r0, c1, r1 = 73, 19, 82, 27
    BARE.update(rect(c0, r0, c1, r1))
    for c, r in outline(c0, r0, c1, r1):
        if r in (r0, r1) and c in (77, 78, 79):
            continue
        glass = (c in (c0, c1) and r in (21, 22, 24, 25)) or (r in (r0, r1) and c in (75, 76))
        wall(c, r, "glass" if glass else "brick")
    tower(c0 - 1, r0 - 1, "gun_tower", enemy=True)
    tower(c1 + 1, r1 + 1, "gun_tower", enemy=True)
    tower(c1 + 1, r0 - 1, "tesla", enemy=True)
    tower(c0 - 1, r1 + 1, "bio_slush", enemy=True)
    # the specials an enemy would take in the west half, out of the east
    # gate's leash
    pickup(75, 21, "gauss_rail")
    pickup(76, 21, "plasma")
    pickup(75, 25, "emp_burst")
    pickup(76, 25, "missiles")
    pickup(80, 21, "grenades")
    pickup(80, 25, "flamethrower")
    pickup(78, 23, "health")
    pickup(78, 24, "ammo")
    # the furnace and its stack, drums of fuel oil under it on a fuse
    for p in rect(88, 16, 91, 18):
        wall(*p, "brick")
    wall(92, 17, "iron")
    drums(((88, 20), (89, 20), (91, 20)), 61)
    oil(hline(87, 92, 19))
    oil(((90, 20),))
    # the flare stack, its knock-out drums at its foot
    wall(95, 15, "iron")
    put(95, 16, "lamp")
    drums(((93, 14), (94, 14), (96, 14)), 65)
    oil(((95, 14), (94, 15), (93, 15)))
    # the checkpoint at the pipe alley's east mouth: two enemy gun towers
    # on the pipelines' ends
    tower(59, 19, "gun_tower", enemy=True)
    tower(59, 28, "gun_tower", enemy=True)


def terminal():
    """The loading terminal behind the south pipe rack: a train of tank
    cars on the siding, the loading rack, and the fenced drum yard by the
    south-east gate - one spark and the yard goes up."""
    clear_rect(63, 35, 96, 46)
    # the terminal's fence on the spine road, its gate the yard by the
    # south road, so the waves from the south-east cross the open yard
    # and leave the rack's drums be
    for r in range(35, 43):
        put(63, r, "fence")
    # the siding and its tank cars
    for c in range(64, 84):
        put(c, 39, "road")
    for c0 in (65, 70, 75, 80):
        for c in range(c0, c0 + 3):
            wall(c, 38, "iron")
    # drums between the cars, waiting to be loaded, each over a spill on
    # the track that runs into the loading rack's fuse
    drums(((68, 38), (73, 38), (78, 38), (69, 38)), 75)
    for c in (68, 73, 78):
        put(c, 39, "oil")
    # pipe stacks and crates on the yard by the fence
    for p in ((64, 35), (65, 35), (70, 35), (71, 35)):
        wall(*p, "iron")
    for p in ((80, 35), (81, 35)):
        put(*p, "wood_crate")
    # the loading rack, its drums on a fuse
    for c in (65, 70, 75, 80):
        wall(c, 42, "iron")
    drums(((66, 41), (71, 41), (76, 41), (81, 41), (67, 41)), 71)
    oil(hline(64, 82, 40))
    # the drum yard, fenced against the field's edges
    x0, y0 = 88, 36
    BARE.update(rect(x0, y0, 96, 47))
    for c in range(x0, 96):
        put(c, y0, "fence")
    for r in range(y0, 47):
        if r not in (39, 40, 41):
            put(x0, r, "fence")
    yard = [(c, r) for r in (37, 40, 44) for c in (90, 91, 94, 95)] + [(90, 45), (91, 45), (94, 45), (95, 45)]
    drums(yard, 73, fuel=0.5)
    oil(hline(89, 96, 38))
    oil(hline(89, 96, 42))
    oil(vline(92, 37, 46))
    oil(vline(93, 37, 46))
    tower(87, 36, "bio_slush", enemy=True)
    pickup(82, 44, "air_jet")
    pickup(72, 44, "health")
    pickup(66, 44, "heat_shield")


# ---------------------------------------------------------------- roads


def roads_all():
    # the north road: the depot's north gate to the north-west gate
    road([(13, 8), (13, 5), (59, 5), (59, 0)])
    # the haul road down the pipe alley into the refinery
    road([(25, 23), (66, 23)])
    # the south road: the depot's south gate to the south-west gate
    road([(13, 39), (13, 41), (59, 41), (59, 47)])
    # the refinery's spine, gate to gate
    road([(61, 5), (61, 41)])
    # the east road
    road([(96, 30), (62, 30)])
    # the north-east and south-east gates' roads
    road([(85, 0), (85, 19)])
    road([(85, 33), (85, 47)])


# ---------------------------------------------------------------- the desert


SCRUB = (("tall_grass", 30), ("juniper", 14), ("bush", 8), ("autumn_bush", 9), ("berry_bush", 5))
WEEDS = (("tall_grass", 6), ("juniper", 2), ("autumn_bush", 1))
SOLO = (("palm", 5), ("snag", 4), ("tree", 1))
# ground kept bare: the depot's yard, the control room, the drum yard
BARE = set()


def desert():
    """Dunes of dry grass, juniper and scrub over the open sand, with a
    lone palm or dead snag here and there - never against anything that
    stands and never on a gate's apron; weeds, thinner, in the set
    pieces' yards and along the roads' verges."""
    near_gate = lambda c, r: any(math.hypot(c - gc, r - gr) < 7 for gc, gr in GATES)
    for r in range(H):
        for c in range(W):
            p = (c, r)
            if p in cells or p in BARE or near_gate(c, r):
                continue
            drift = fbm(c / 5.0, r / 5.0, 81)
            u = hash01(c, r, 83)
            if p in reserved:
                if drift > 0.58 and u < 0.16:
                    put(c, r, pick(WEEDS, hash01(c, r, 89)))
                continue
            if drift > 0.53 and u < 0.55:
                put(c, r, pick(SCRUB, hash01(c, r, 85)))
            elif drift < 0.45 and u < 0.045:
                ring = [(c + dc, r + dr) for dc in (-2, -1, 0, 1, 2) for dr in (-2, -1, 0, 1, 2)]
                if not any(q in cells and kind_at(q) not in COVER_KINDS for q in ring) and \
                        not any(q in roads for q in ring):
                    put(c, r, pick(SOLO, hash01(c, r, 87)))


def grove(cells_, salt, kinds=(("palm", 1),)):
    """Trees on `cells_` - an oasis's palms, a burnt-out stand's snags -
    each standing clear of every other tree and tile by two cells, so the
    ground between them stays a way a tank can take."""
    for c, r in cells_:
        if (c, r) in cells or not inb(c, r):
            continue
        ring = [(c + dc, r + dr) for dc in range(-2, 3) for dr in range(-2, 3) if (dc, dr) != (0, 0)]
        if any(q in cells and kind_at(q) not in COVER_KINDS | {"oil", "road"} for q in ring):
            continue
        put(c, r, pick(kinds, hash01(c, r, salt)))


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
    """Open ground the playfield never reaches grows a juniper, then a
    snag, so no nav cell is left boxed in or cut off."""
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
                    put(*p, "snag")
                    FILLED.append(p)
                    filled += 1
                    break
    return filled


FILLED = []


def gate_aprons():
    """The lane a wave rolls in by: anything soft cleared five cells in
    from every gate."""
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
            nd = d + (3 if n in water else 1)
            if nd < cost.get(n, 1 << 30):
                cost[n] = nd
                heapq.heappush(heap, (nd, n))
    return cost


def wave_routes():
    """The map cells on each gate's cheapest walk to the start - the way a
    wave comes in."""
    cost = walk_costs()
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


SOLID_FOR_SHOTS = None


def stops_shot(p):
    """Whether a shell flying over map cell `p` meets something that stops
    it: a wall, a tower, a tree or a prop that is not a sandbag (shots pass
    over sandbags) or a tank trap (between its beams, more often than not)."""
    k = kind_at(p)
    if k is None or k in SOFT or k in ("sandbag", "tank_trap", "fence"):
        return k in ("barrel", "hay", "wood_crate", "barrier")
    return True


def fuse_groups():
    """The drums that go up together: two drums are one group when a fuse
    of oil runs between them (a drum sets off the oil beside it, burning
    oil the drums beside it) or one stands within the other's blast
    (`barrel_blast_radius`, three cells)."""
    oil_cells = {p for p in cells if kind_at(p) == "oil"}
    parent = {p: p for p in DRUMS}

    def find(p):
        while parent[p] != p:
            parent[p] = parent[parent[p]]
            p = parent[p]
        return p

    def union(a, b):
        parent[find(a)] = find(b)

    for i, a in enumerate(DRUMS):
        for b in DRUMS[i + 1:]:
            if math.hypot(a[0] - b[0], a[1] - b[1]) <= 3:
                union(a, b)
    # oil components, each joined to every drum beside it
    seen = set()
    for start in sorted(oil_cells):
        if start in seen:
            continue
        comp, q = [], deque([start])
        seen.add(start)
        while q:
            c, r = q.popleft()
            comp.append((c, r))
            for n in ((c + 1, r), (c - 1, r), (c, r + 1), (c, r - 1)):
                if n in oil_cells and n not in seen:
                    seen.add(n)
                    q.append(n)
        beside = [d for d in DRUMS if any((d[0] + dc, d[1] + dr) in comp for dc, dr in ((1, 0), (-1, 0), (0, 1), (0, -1)))]
        for d in beside[1:]:
            union(beside[0], d)
    groups = {}
    for d in DRUMS:
        groups.setdefault(find(d), []).append(d)
    return sorted(groups.values(), key=len, reverse=True)


def straight_shots(seat):
    """Every bearing from `seat` along which a shell leaves the depot
    without meeting anything that stops it."""
    c0, r0, c1, r1 = DEPOT
    sx, sy = seat[0] + 0.5, seat[1] + 0.5
    out = []
    for i in range(1440):
        a = 2 * math.pi * i / 1440
        dx, dy = math.cos(a), math.sin(a)
        x, y = sx, sy
        while True:
            x += dx * 0.05
            y += dy * 0.05
            p = (math.floor(x), math.floor(y))
            if p[0] < 0:
                break  # the field's own edge
            if not (c0 <= p[0] <= c1 and r0 <= p[1] <= r1):
                out.append(round(math.degrees(a), 1))
                break
            if p != seat and stops_shot(p):
                break
    return out


# ---------------------------------------------------------------- the map


def build():
    roads_all()
    depot()
    fuel_dump()
    truck_park()
    pipe_alley()
    oiltown()
    north_field()
    settling_pond()
    south_field()
    tank_farm()
    pipe_racks()
    process_unit()
    terminal()
    gate_aprons()
    for gc, gr in GATES:
        put(gc, gr, "gate")
        GATE_LANES.update(lane(gc, gr))
    desert()
    return fill_pockets()


HEADER = """\
# black-gold - Black Gold, level 14: an oil refinery in the desert, wired
# with fuses - one spark and a yard goes up. Written by
# tools/gen_black_gold.py - change the generator and run it again rather
# than editing this file; re-saving from the map editor would drop these
# comments. Level tables as dotted keys on purpose - a `[spawn]` header
# would swallow every `cells.` line after it.
#
# Destroy on the waves plan: six waves of 4 to 9 tanks, light to super, 39
# in all, rolling in at five gates - two on the north edge (by Oiltown's
# road and between the storage tanks), one on the east, two on the south.
# 97 x 48 cells, followed by the camera, under a clear sky, at dusk, in a
# sandstorm or in the heat haze.
#
# West: the Wildcat Depot, the players' walled base and start, its back to
# the field's edge. Concrete walls with a gate north, east and south, and
# four cells inside every gate a concrete baffle, so nothing outside has a
# straight shot at the starts: a tank that comes in turns along the walls,
# the east gate's way only north round its baffle. Gun towers on the two
# east corners, a tesla in every baffle, bio slush on the north and south
# walls. The stores: health and ammo by the west wall, a tower pack and a
# shield, the minigun and plasma, a jet strike and a helicopter strike on
# the helipad, missiles and grenades, and a heat shield in the north
# corridor. The supply pipelines run along the north and south roads to
# the gates, a line of fuel drums on a fuse behind each; the fuel dump and
# the gravity well north-west, the truck park with missiles, health and
# ammo south-west.
#
# Middle: the pipe alley, the haul road east from the depot between two
# iron pipelines with an oil fuse down each side, fuel drums at the
# crossings (lit, the alley burns end to end), plasma and a minigun in it
# and two enemy gun towers at its east mouth. North of it Oiltown, the
# drillers' town - the timber shack (plasma, ammo), the cantina (the
# minigun, health) with the players' gun tower on its corner, a well under
# palms, the water tower and a fuel store - and four pump jacks whose sumps
# lie on flow lines from jack to jack, with the sonic hammer and a second
# gravity well among them. South of it the settling pond in its reeds and
# palms with the players' bio slush, the FPV swarm, a minigun and a speed
# up on its shore, four more pump jacks on their flow lines with the laser,
# and in the dunes past the south road the old derrick, its drums and a
# cache of missiles and plasma.
#
# East: the refinery, the enemy's. The tank farm - three iron storage tanks
# with glass roofs, drums at their feet on a ring of oil, a concrete
# firewall to the west and an enemy gun tower between the tanks over the
# flamethrower, grenades, a helicopter strike and health - behind the north
# pipe rack and its enemy tesla. The process unit: the distillation columns
# with drums of naphtha between them, the control room holding the enemy's
# stash (the gauss rail, the EMP burst, plasma, missiles, grenades, the
# flamethrower, health and ammo) under two enemy gun towers, a tesla and a
# bio slush, the furnace on its drums of fuel oil and the flare stack.
# Behind the south pipe rack the loading terminal: a train of tank cars on
# the siding with drums between them, the loading rack's drums on a fuse,
# the jet strike, health and a heat shield, and the fenced drum yard by the
# south-east gate - sixteen drums on a cross of oil and an enemy bio slush.
#
# The desert everywhere else: dunes of dry grass, juniper and scrub,
# palms and dead snags.
"""


def write(path):
    lines = [HEADER, "version = 1",
             'tank = "breaker"',
             'tank2 = "ravager"',
             'theme = "desert"',
             'weather = ["clear", "dusk", "sandstorm", "heat_haze"]',
             f"size = [{W}, {H}]",
             'mission.kind = "destroy"',
             'spawn.kind = "waves"',
             "spawn.waves = 6",
             "spawn.size = 4",
             "spawn.growth = 1",
             'spawn.tier_start = "light"',
             'spawn.tier_end = "super"',
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
    for c, r, kind in pickups:
        if kind in FETCHED:
            near = min(math.hypot(c - gc, r - gr) for gc, gr in GATES)
            if near < 20.5:
                problems.append(f"{kind} at {c},{r} is {near:.1f} cells from a gate")
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
        if math.hypot(c - START[0], r - START[1]) < 8 or math.hypot(c - START2[0], r - START2[1]) < 8:
            problems.append(f"drum at {c},{r} is by a start")
    for c, r in loose_on_routes():
        problems.append(f"{kind_at((c, r))} at {c},{r} stands on a wave's way in")
    for seat in (START, START2):
        shots = straight_shots(seat)
        if shots:
            problems.append(f"start {seat} has {len(shots)} straight shots out, e.g. {shots[:4]}")
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
            if k in TOWER_KINDS and 'enemy' in cells[(c, r)]:
                ch = ch.lower()
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
    print("nav: '.' reached, '+' one wide, '!' open but cut off, ' ' shut")
    print("    " + "".join(str(c // 10 % 10) for c in range(W)))
    print("    " + "".join(str(c % 10) for c in range(W)))
    for l in range(H):
        print(f"{l:3d} " + "".join("+" if (k, l) in thin else "." if (k, l) in reach else "!" if nav_open(k, l) else " "
                                   for k in range(W)))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("-o", "--out", default="maps/black-gold.toml")
    ap.add_argument("--stats", action="store_true")
    ap.add_argument("--ascii", action="store_true")
    ap.add_argument("--nav", action="store_true")
    ap.add_argument("--routes", action="store_true")
    a = ap.parse_args()
    filled = build()
    if a.ascii or a.routes:
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
        towers = {"player": 0, "enemy": 0}
        for t in cells.values():
            if t.split('"')[1] in TOWER_KINDS:
                towers["enemy" if "enemy" in t else "player"] += 1
        print(f"{len(cells)} cells, {filled} pocket cells filled, {len(narrow())} one-wide nav cells, "
              f"{len(DRUMS)} drums, towers {towers}")
        print(f"  pockets filled at {sorted(FILLED)}")
        groups = fuse_groups()
        print(f"  {len(groups)} drum groups that go up together, sizes {[len(g) for g in groups]}")
        for k, n in sorted(kinds.items(), key=lambda kv: -kv[1]):
            print(f"  {k:12} {n}")
        cost = walk_costs()
        for gc, gr in GATES:
            inside = lane(gc, gr)[-1]
            c = cost.get(inside)
            print(f"  gate {gc},{gr}: walk {c} cells = {c * 32 / 160:.1f} s" if c else f"  gate {gc},{gr}: no walk")
        for c, r, kind in pickups:
            print(f"  pickup {kind:13} {c},{r}")
    if problems:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
