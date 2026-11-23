"""Write maps/wildwood.toml: the Wildwood, a forest twice Longwater's size on
each side (224 x 126 cells), laid out region by region from a fixed seed.

  python3 tools/gen_wildwood.py [-o maps/wildwood.toml] [--stats]

A map this size is too big to lay out by hand, so this script is the
source: change it and run it again rather than editing the TOML (a
re-save from the map editor would drop the header too). The same seed
writes the same file every time - the noise is hashed from cell
positions, and every other draw comes from one seeded `random.Random`
in a fixed order.

What it lays out, west to east (the header written into the map says
how it plays):

- the Hollow Lodge, the players' clearing round the bong, with friendly
  towers, and the orchard farm north of it;
- a hedge maze of firs in the north-west with an EMP burst at its heart;
- the ruined chapel and its sealed brick crypt;
- the Tyre Circuit, a race track walled with tyre stacks;
- the Great Mere with a volcano on an island in the middle, reached only
  through the portal ring;
- a secret grove sealed in a fir wood, one hidden trail in;
- the lumber camp with its oil-trail fuse, the beaver dam and the autumn
  glen in the south;
- the stone circle;
- the Old Line, a front of barriers and tank traps with enemy bunkers;
- the airfield and its hangar in the north-east;
- the bandit camp in the south-east;
- the Dead Wood, no-man's-land before the Black Keep, the enemy's moated
  castle on the east edge, where their bong lives;
- a dozen forest hollows, each holding a cache and reached by a hidden
  trail.
"""

import argparse
import math
import random
from collections import deque

W, H = 224, 126
SEED = 0xB0B217

rng = random.Random(SEED)

# (col, row) -> inline TOML table text
cells = {}
# cells the forest pass must leave alone
reserved = set()
# cells meant to be unreachable on foot (the volcano isle, the sealed crypt)
intentional = set()
water = set()
roads = set()


def inb(c, r):
    return 0 <= c < W and 0 <= r < H


def obj(kind, **kw):
    parts = [f'kind = "{kind}"'] + [f'{k} = "{v}"' for k, v in kw.items()]
    return "{ " + ", ".join(parts) + " }"


def put(c, r, kind, **kw):
    if not inb(c, r):
        return
    cells[(c, r)] = obj(kind, **kw)
    if kind != "water":
        water.discard((c, r))
    if kind != "road":
        roads.discard((c, r))
    reserved.add((c, r))


def wall(c, r, material):
    put(c, r, "wall", material=material)


def clear(c, r):
    if inb(c, r):
        cells.pop((c, r), None)
        water.discard((c, r))
        roads.discard((c, r))
        reserved.add((c, r))


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


def clear_disc(cx, cy, rad):
    """Open ground for a set piece: everything goes but the roads and the
    water running through it."""
    for c, r in list(disc(cx, cy, rad)):
        if (c, r) not in roads and (c, r) not in water:
            clear(c, r)
        reserved.add((c, r))


def clear_rect(c0, r0, c1, r1):
    for c, r in list(rect(c0, r0, c1, r1)):
        if (c, r) not in roads and (c, r) not in water:
            clear(c, r)
        reserved.add((c, r))


def put_free(c, r, kind, **kw):
    """A decoration that gives way to a road, water or anything already
    placed."""
    if inb(c, r) and (c, r) not in cells:
        put(c, r, kind, **kw)


TREE_KINDS = {"tree", "pine", "spruce", "scots", "fir", "birch", "willow", "palm", "snag"}
COVER_KINDS = {"tall_grass", "bush", "berry_bush", "juniper", "fern", "autumn_bush", "reeds"}


def kind_at(p):
    t = cells.get(p)
    return t.split('"')[1] if t else None


def carvable(p):
    """Open ground, a tree or soft cover: what a trail may be cut through."""
    k = kind_at(p)
    return p not in roads and p not in water and p not in intentional and (
        k is None or k in TREE_KINDS or k in COVER_KINDS)


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


# ---------------------------------------------------------------- places

HOME = (20, 63)
VOLCANO = (112, 63)
KEEP = (206, 63)
GROVE = (132, 24)
GLEN = (80, 113)
CIRCLE = (140, 108)
BANDITS = (182, 102)
CIRCUIT = (70, 72)
NO_TREES_NEAR_VOLCANO = 17  # volcano bombs reach about 14.4 cells

GATES = [
    (78, 0), (140, 0), (212, 0),  # north
    (223, 20), (223, 105),  # east
    (30, 125), (62, 125), (125, 125), (165, 125), (205, 125),  # south
    (0, 12), (0, 115),  # west
]

# what an enemy turns off its road to fetch lies beyond every gate's
# leash (`enemy_leash_px`, 640 px = 20 cells)
SPECIALS = {
    "laser", "minigun", "plasma", "missiles", "flamethrower", "grenades",
    "sonic_hammer", "emp_burst", "gauss_rail", "fpv_swarm", "rod_from_god",
    "gravity_well", "heat_shield", "air_heli", "air_jet", "tower_pack",
    "speedup", "shield",
}
pickups = []  # (c, r, kind)


def pickup(c, r, kind):
    put(c, r, "pickup", pickup=kind)
    pickups.append((c, r, kind))


# ---------------------------------------------------------------- water


def lake_mere():
    cx, cy = VOLCANO
    rx, ry = 22.0, 14.0
    for r in range(cy - 17, cy + 18):
        for c in range(cx - 26, cx + 27):
            dx, dy = (c - cx) / rx, (r - cy) / ry
            wobble = 0.55 * (fbm(c / 7, r / 7, 7) - 0.5)
            if dx * dx + dy * dy <= 1.0 + wobble:
                isle = math.hypot(c - cx, r - cy) <= 6.6 + 0.6 * (fbm(c / 3, r / 3, 9) - 0.5)
                if not isle:
                    put(c, r, "water")
                    water.add((c, r))
                    reserved.discard((c, r))


def pond(c0, r0, c1, r1):
    for c, r in rect(c0, r0, c1, r1):
        put(c, r, "water")
        water.add((c, r))


def river(start, end_row, lo, hi, stop_on_water=False, drift=0.0):
    """A one-cell stream from `start` down (or up) to `end_row`, meandering
    between columns `lo` and `hi`. Never turns straight back after one row,
    so it never makes a 2 x 2 block and reads as a river, not a lake."""
    c, r = start
    step = 1 if end_row > r else -1
    path = [(c, r)]
    side = 0
    since_turn = 9
    while r != end_row:
        if rng.random() < 0.38 + abs(drift) * 0.2:
            want = rng.choice((-1, 1)) if rng.random() > abs(drift) else (1 if drift > 0 else -1)
            if side and want != side and since_turn < 2:
                want = side
            run = rng.randint(1, 3)
            for _ in range(run):
                nc = c + want
                if lo <= nc <= hi:
                    c = nc
                    path.append((c, r))
            side = want
            since_turn = 0
        r += step
        since_turn += 1
        path.append((c, r))
        if stop_on_water and (c, r + step) in water:
            break
    for c, r in path:
        if inb(c, r):
            put(c, r, "water")
            water.add((c, r))
    return path


def is_deep(c, r):
    return all((c + dc, r + dr) in water for dc in (-1, 0, 1) for dr in (-1, 0, 1))


# ---------------------------------------------------------------- roads


def raster_line(a, b):
    (x0, y0), (x1, y1) = a, b
    n = int(max(abs(x1 - x0), abs(y1 - y0)) * 4) + 1
    out = []
    for i in range(n + 1):
        t = i / n
        p = (round(x0 + (x1 - x0) * t), round(y0 + (y1 - y0) * t))
        if out and p != out[-1] and p[0] != out[-1][0] and p[1] != out[-1][1]:
            out.append((p[0], out[-1][1]))  # keep the road 4-connected
        if not out or p != out[-1]:
            out.append(p)
    return out


def road(points, width=2, margin=1):
    path = []
    for a, b in zip(points, points[1:]):
        path += raster_line(a, b)
    for c, r in path:
        for dc in range(width):
            for dr in range(width):
                cc, rr = c + dc, r + dr
                if inb(cc, rr):
                    put(cc, rr, "road")
                    roads.add((cc, rr))
    # the verges either side stay clear of trees
    for c, r in path:
        for dc in range(-margin, width + margin):
            for dr in range(-margin, width + margin):
                if inb(c + dc, r + dr):
                    reserved.add((c + dc, r + dr))
    return path


# ---------------------------------------------------------------- set pieces


def hollow_lodge():
    cx, cy = HOME
    clear_disc(cx, cy, 13)
    put(cx, cy, "bong")
    put(27, 60, "start")
    put(27, 66, "start2")
    # the bong's garden: a fence ring with two gaps
    for c, r in outline(16, 59, 24, 67):
        if (c == 24 and r in (62, 63, 64)) or (c == 16 and r in (62, 63, 64)):
            continue
        put(c, r, "fence")
    for c, r in ((17, 60), (23, 60), (17, 66), (23, 66)):
        put(c, r, "berry_bush")
    # the lodge (shield, bong health) and the store (ammo, health)
    for c, r in outline(13, 51, 20, 56):
        if r == 56 and c in (15, 16, 17):
            continue
        wall(c, r, "wood")
    pickup(15, 53, "shield")
    pickup(18, 53, "bong_health")
    for c, r in outline(13, 70, 20, 75):
        if r == 70 and c in (15, 16, 17):
            continue
        wall(c, r, "wood")
    pickup(15, 73, "ammo")
    pickup(18, 73, "health")
    # towers of the players' own side
    put(30, 55, "gun_tower")
    put(30, 71, "gun_tower")
    put(9, 70, "tesla")
    # sandbag arcs facing east, a gap for each road
    for r in list(range(55, 61)) + list(range(67, 73)):
        put(33, r, "sandbag")
    for c, r in ((26, 57), (26, 69), (11, 57), (11, 69)):
        put(c, r, "lamp")
    pond(5, 60, 8, 66)
    for c, r in ((4, 61), (4, 64), (9, 60), (9, 66), (6, 59), (7, 67)):
        put(c, r, "reeds")
    put(30, 59, "hay")
    put(30, 67, "hay")
    for c, r in ((23, 73), (24, 73), (23, 74)):
        put(c, r, "wood_crate")
    pickup(25, 63, "health")
    pickup(29, 63, "ammo")
    # Wildwood is level 7 (docs/levels.md): the weapons a campaign player
    # meets first here lie by the start, inside its sight box, the blast
    # ones two of their blast's radii and more from the bong (the gravity
    # well's reach is 140 px; this one lies 12.5 cells off)
    pickup(31, 57, "gravity_well")
    pickup(29, 57, "air_jet")
    pickup(31, 61, "air_heli")
    pickup(24, 57, "tower_pack")


def orchard():
    clear_rect(10, 30, 32, 46)
    for c, r in outline(11, 31, 31, 45):
        if (r == 45 and c in (21, 22, 23)) or (c == 31 and r in (36, 37, 38)):
            continue
        put(c, r, "fence")
    for c in (13, 17, 21):
        for r in (33, 36):
            put(c, r, "tree")
    for c, r in rect(24, 33, 29, 38):
        put(c, r, "tall_grass")
    for c, r in ((26, 35), (28, 37)):
        put(c, r, "hay")
    for c, r in outline(12, 39, 20, 44):
        if r == 39 and c in (15, 16, 17):
            continue
        wall(c, r, "wood")
    pickup(15, 42, "minigun")
    put(18, 42, "hay")
    for c in range(23, 30, 2):
        put(c, 43, "berry_bush")


def hedge_maze():
    c0, r0, n = 40, 4, 11
    span = n * 3 + 1
    hedge = lambda c, r: put(c, r, "fir" if hash01(c, r, 31) < 0.7 else "spruce")
    for c, r in rect(c0, r0, c0 + span - 1, r0 + span - 1):
        hedge(c, r)
    room = {(i, j) for i in range(4, 7) for j in range(4, 7)}

    def carve_unit(i, j):
        for c, r in rect(c0 + 3 * i + 1, r0 + 3 * j + 1, c0 + 3 * i + 2, r0 + 3 * j + 2):
            clear(c, r)

    def carve_between(a, b):
        (i, j), (k, l) = a, b
        if i != k:
            wc = c0 + 3 * max(i, k)
            for r in (r0 + 3 * j + 1, r0 + 3 * j + 2):
                clear(wc, r)
        else:
            wr = r0 + 3 * max(j, l)
            for c in (c0 + 3 * i + 1, c0 + 3 * i + 2):
                clear(c, wr)

    seen = {(0, 0)}
    stack = [(0, 0)]
    carve_unit(0, 0)
    degree = {}
    while stack:
        i, j = stack[-1]
        nbrs = [(i + di, j + dj) for di, dj in ((1, 0), (-1, 0), (0, 1), (0, -1))
                if 0 <= i + di < n and 0 <= j + dj < n and (i + di, j + dj) not in seen]
        if not nbrs:
            stack.pop()
            continue
        k = rng.choice(nbrs)
        seen.add(k)
        carve_unit(*k)
        carve_between((i, j), k)
        degree[(i, j)] = degree.get((i, j), 0) + 1
        degree[k] = degree.get(k, 0) + 1
        stack.append(k)
    exits = {(5, 10), (0, 8)}
    dead_ends = sorted(u for u, d in degree.items()
                       if d == 1 and u not in room and u not in exits
                       and not (3 <= u[0] <= 7 and 3 <= u[1] <= 7))
    # the heart: three by three units opened into one room
    for c, r in rect(c0 + 13, r0 + 13, c0 + 20, r0 + 20):
        clear(c, r)
    mid_c, mid_r = c0 + 16, r0 + 16
    # an EMP, not a gauss rail: on a Hunt two slugs would end the hunt
    pickup(mid_c, mid_r, "emp_burst")
    pickup(mid_c + 1, mid_r + 1, "health")
    pickup(mid_c - 2, mid_r + 2, "ammo")
    put(mid_c - 3, mid_r - 3, "lamp")
    put(mid_c + 3, mid_r - 3, "lamp")
    # ways in: south and west
    for c in (c0 + 16, c0 + 17):
        clear(c, r0 + span - 1)
    for r in (r0 + 25, r0 + 26):
        clear(c0, r)
    # three dead ends hold a little something - and one a trap
    dead_ends = [u for u in dead_ends
                 if min(math.hypot(c0 + 3 * u[0] - gc, r0 + 3 * u[1] - gr) for gc, gr in GATES) > 22]
    rng.shuffle(dead_ends)
    for (i, j), what in zip(dead_ends, ("speedup", "ammo", "trap")):
        c, r = c0 + 3 * i + 1, r0 + 3 * j + 1
        if what == "trap":
            put(c + 1, r + 1, "barrel", drum="fuel")
            pickup(c, r, "health")
        else:
            pickup(c, r, what)
    return (c0 + 16, r0 + span)


def ruined_chapel():
    clear_rect(86, 14, 104, 31)
    for c, r in outline(89, 16, 101, 27):
        if r == 27 and c in (94, 95, 96):
            continue
        if hash01(c, r, 41) < 0.28 and not (c in (89, 101) and r in (16, 27)):
            put(c, r, "tall_grass")  # the walls have fallen here
            continue
        wall(c, r, "brick")
    for c, r in ((89, 16), (101, 16), (89, 27), (101, 27)):
        wall(c, r, "iron")
    wall(95, 18, "iron")  # the altar
    pickup(93, 19, "missiles")
    pickup(97, 19, "laser")
    # the crypt, sealed in brick: shoot your way in
    for c, r in outline(97, 21, 101, 25):
        wall(c, r, "brick")
    pickup(99, 23, "gravity_well")
    pickup(100, 24, "health")
    intentional.update(rect(98, 22, 100, 24))
    for c in range(88, 103, 3):
        put(c, 30, "barrier")  # the old graves
    for c, r in ((91, 29), (99, 29), (87, 22), (103, 20)):
        put(c, r, "snag")
    put(93, 28, "lamp")
    put(97, 28, "lamp")


def tyre_circuit():
    cx, cy = CIRCUIT
    rx, ry, irx, iry = 13.0, 8.0, 7.0, 3.0
    e = lambda c, r, a, b: ((c - cx) / a) ** 2 + ((r - cy) / b) ** 2
    clear_rect(cx - 15, cy - 10, cx + 15, cy + 10)
    for c, r in rect(cx - 14, cy - 9, cx + 14, cy + 9):
        inside = e(c, r, rx, ry) <= 1
        edge = inside and any(e(c + dc, r + dr, rx, ry) > 1 for dc, dr in ((1, 0), (-1, 0), (0, 1), (0, -1)))
        if edge and not (abs(r - cy) <= 1 and c < cx) and not (abs(c - cx) <= 1 and r < cy):
            put(c, r, "tyres")
        iin = e(c, r, irx, iry) <= 1
        iedge = iin and any(e(c + dc, r + dr, irx, iry) > 1 for dc, dr in ((1, 0), (-1, 0), (0, 1), (0, -1)))
        if iedge:
            put(c, r, "tyres")
        elif iin:
            put(c, r, "tall_grass")
    pickup(cx, cy + 6, "speedup")
    pickup(cx + 10, cy - 1, "ammo")
    for c, r in ((cx - 12, cy - 6), (cx + 12, cy + 6), (cx - 12, cy + 6)):
        put(c, r, "hay")


def great_mere():
    cx, cy = VOLCANO
    put(cx, cy, "volcano")
    for c, r in disc(cx, cy, 2.3):
        if (c, r) != (cx, cy):
            reserved.add((c, r))
    put(cx, cy - 4, "portal")
    # an FPV swarm, not a rod from god: on a Hunt one call would end it
    pickup(cx - 4, cy, "fpv_swarm")
    pickup(cx + 4, cy, "heat_shield")
    pickup(cx, cy + 4, "health")
    for c, r in disc(cx, cy, 7.5):
        if (c, r) not in water:
            intentional.add((c, r))
            reserved.add((c, r))
    # the fishing pier off the north shore, plasma at its end - and the
    # island just out of reach beyond it
    top = min(r for (c, r) in water if c == cx)
    for r in range(cy - 21, top + 2):
        for c in (cx - 1, cx, cx + 1):
            put(c, r, "road")
            roads.add((c, r))
    for r in range(top, top + 3):
        put(cx - 2, r, "fence")
        put(cx + 2, r, "fence")
    pickup(cx, top + 2, "plasma")


def secret_grove():
    cx, cy = GROVE
    ring = list(disc(cx, cy, 9))
    for c, r in ring:
        d = math.hypot(c - cx, r - cy)
        if d > 5.5:
            put(c, r, "fir" if hash01(c, r, 53) < 0.6 else "spruce")
        else:
            clear(c, r)
    pond(cx + 3, cy - 4, cx + 4, cy - 3)
    put(cx - 3, cy - 3, "willow")
    for c, r in ((cx - 4, cy), (cx + 4, cy + 1), (cx - 1, cy - 5), (cx + 1, cy + 4)):
        put(c, r, "berry_bush")
    pickup(cx - 2, cy + 1, "fpv_swarm")
    pickup(cx + 1, cy + 2, "tower_pack")
    pickup(cx - 2, cy - 2, "health")
    put(cx + 1, cy - 1, "portal")
    # one trail in, winding down to the King's Road, its mouth in bushes
    trail = []
    for a, b in zip([(cx - 1, cy + 5), (cx - 4, cy + 9), (cx - 1, cy + 13), (cx - 5, cy + 17)],
                    [(cx - 4, cy + 9), (cx - 1, cy + 13), (cx - 5, cy + 17), (cx - 5, cy + 18)]):
        trail += raster_line(a, b)
    tset = {(c + dc, r) for c, r in trail for dc in (-1, 0, 1)}
    for p in tset:
        if p not in roads:
            clear(*p)
    # the fir wood the trail runs through, down to the road
    for c, r in trail:
        for dc in list(range(-5, -1)) + list(range(2, 6)):
            n = (c + dc, r)
            if n in tset or not carvable(n) or math.hypot(n[0] - cx, n[1] - cy) <= 5.5:
                continue
            put(*n, "fir" if hash01(*n, 57) < 0.6 else "spruce")
    for p in tset:
        if p not in roads and p[1] >= cy + 15:
            put(*p, "bush")


def lumber_camp():
    clear_rect(38, 90, 64, 112)
    for c, r in outline(44, 93, 50, 98):
        if r == 98 and c in (46, 47, 48):
            continue
        wall(c, r, "wood")
    pickup(47, 95, "flamethrower")
    for c0, r in ((39, 92), (39, 94), (53, 92), (53, 94), (57, 104)):
        for c in range(c0, c0 + 3):
            wall(c, r, "wood")  # log piles
    for c, r in ((53, 99), (54, 99), (53, 100), (58, 98)):
        put(c, r, "wood_crate")
    put(41, 104, "hay")
    put(42, 104, "hay")
    for r in (96, 99, 102):
        put(61, r, "target")
    for r in range(95, 104):
        put(63, r, "hay")
    pickup(55, 102, "grenades")
    pickup(44, 101, "ammo")
    pickup(48, 109, "health")
    # the fuse: an oil trail beside the road to a stack of fuel drums
    for c in range(43, 56):
        put(c, 107, "oil")
    for c, r in ((56, 106), (56, 107), (56, 108), (57, 107)):
        put(c, r, "barrel", drum="fuel")
    put(60, 109, "tyres")
    put(61, 109, "tyres")


def beaver_dam(south_river):
    at = [p for p in south_river if p[1] == 93]
    if not at:
        return
    cx = at[0][0]
    pond(cx - 2, 87, cx + 2, 91)
    for c in range(cx - 2, cx + 3):
        wall(c, 92, "wood")
    put(cx, 93, "water")
    water.add((cx, 93))
    for c, r in ((cx - 3, 88), (cx + 3, 89), (cx - 3, 90), (cx + 3, 87)):
        put(c, r, "reeds")
    pickup(cx + 4, 92, "laser")


def stone_circle():
    cx, cy = CIRCLE
    clear_disc(cx, cy, 9)
    for k in range(12):
        a = k * math.pi / 6
        wall(round(cx + 6 * math.cos(a)), round(cy + 6 * math.sin(a)), "iron")
    for c, r in disc(cx, cy, 3.6):
        if math.hypot(c - cx, r - cy) > 2.2:
            put(c, r, "tall_grass")
    pickup(cx, cy, "emp_burst")
    wall(cx, cy - 9, "iron")  # the heel stone


def old_line():
    gaps = set()
    for lo, hi in ((29, 31), (45, 53), (69, 71), (96, 104)):
        gaps.update(range(lo, hi + 1))
    for r in range(26, 101):
        if r in gaps:
            continue
        put_free(150, r, "tank_trap")
        if r % 3 == 0:
            put_free(147, r, "barrier")  # concrete blocks out front, to shove
    for r in (38, 62, 86):
        for c, rr in outline(154, r - 2, 158, r + 2):
            if c == 154 and rr == r:
                continue
            put(c, rr, "sandbag")
        put(156, r, "gun_tower", side="enemy")
    # the bunker the sonic hammer was left in
    for c, r in outline(156, 73, 160, 79):
        if c == 160 and r in (75, 76, 77):
            continue
        wall(c, r, "iron")
    pickup(158, 76, "sonic_hammer")


def airfield():
    clear_rect(160, 2, 205, 21)
    for c in range(162, 204):
        for r in (14, 15):
            put(c, r, "road")
            roads.add((c, r))
    for c in (175, 176):
        for r in range(9, 14):
            put(c, r, "road")
            roads.add((c, r))
    for c, r in outline(169, 2, 182, 9):
        if r == 9 and c in (174, 175, 176, 177):
            continue
        wall(c, r, "iron")
    pickup(172, 5, "air_heli")
    pickup(179, 5, "air_jet")
    for c, r in ((170, 3), (181, 3), (170, 4)):
        put(c, r, "barrel", drum="fuel")
    put(170, 7, "tyres")
    put(181, 7, "tyres")
    for c, r in outline(187, 3, 191, 7):
        if r == 7 and c in (188, 189, 190):
            continue
        wall(c, r, "glass")
    pickup(189, 5, "speedup")
    for c in range(164, 204, 5):
        put(c, 12, "barrier")
        put(c, 17, "barrier")
    put(161, 14, "lamp")
    put(204, 15, "lamp")
    for c, r in ((184, 9), (185, 9), (184, 10)):
        put(c, r, "wood_crate")


def bandit_camp():
    cx, cy = BANDITS
    clear_disc(cx, cy, 8)
    for c0, r0 in ((178, 98), (184, 98), (178, 105)):
        for c, r in rect(c0, r0, c0 + 2, r0 + 1):
            wall(c, r, "wood")
    for k in range(28):
        a = k * 2 * math.pi / 28
        if k in (0, 1, 13, 14, 20):
            continue  # ways in
        c, r = round(cx + 7 * math.cos(a)), round(cy + 7 * math.sin(a))
        prop = ("wood_crate", "tyres", "barrel")[k % 3]
        put_free(c, r, prop)
    put(cx, cy, "barrel", drum="oil")
    put(cx, cy - 3, "tesla", side="enemy")
    put(cx + 4, cy + 2, "gun_tower", side="enemy")
    pickup(cx - 2, cy, "plasma")
    pickup(cx + 2, cy - 1, "shield")
    pickup(cx + 1, cy + 4, "ammo")


def black_keep():
    cx, cy = KEEP
    clear_rect(184, 48, 223, 78)
    # the moat laps the walls: no strip of bank inside it to get caught on
    for c, r in rect(194, 51, 218, 75):
        if not (197 <= c <= 215 and 54 <= r <= 72):
            put(c, r, "water")
            water.add((c, r))
    for c, r in outline(197, 54, 215, 72):
        if c == 197 and r in (62, 63, 64):
            continue
        corner = c in (197, 215) and r in (54, 72)
        buttress = (c - 197) % 6 == 0 and (r - 54) % 6 == 0
        wall(c, r, "iron" if corner or buttress else "brick")
    for c in range(193, 198):
        for r in (62, 63, 64):
            put(c, r, "road")
            roads.add((c, r))
    # the bong's garden in the ward: a ring of bushes, room for its guards
    for c, r in outline(203, 60, 209, 66):
        if hash01(c, r, 67) < 0.75:
            put(c, r, "berry_bush" if (c + r) % 2 else "bush")
    put(cx, cy, "enemy_bong")
    for c, r in ((199, 56), (213, 56), (199, 70), (213, 70)):
        put(c, r, "gun_tower", side="enemy")
    put(199, 60, "tesla", side="enemy")
    put(199, 66, "tesla", side="enemy")
    put(212, 63, "bio_slush", side="enemy")
    pickup(201, 69, "health")
    for c, r in ((191, 59), (192, 67)):
        put_free(c, r, "lamp")
    for r in range(54, 60):
        put_free(189, r, "sandbag")
    for r in range(68, 73):
        put_free(192, r, "sandbag")
    for c, r in ((184, 57), (186, 65), (183, 66), (186, 55), (182, 63)):
        put_free(c, r, "tank_trap")


def autumn_glen():
    cx, cy = GLEN
    clear_disc(cx, cy, 2.5)
    pickup(cx, cy, "missiles")
    pickup(cx + 1, cy + 1, "speedup")
    for k in range(10):
        a = k * math.pi / 5
        put_free(round(cx + 3.6 * math.cos(a)), round(cy + 3.6 * math.sin(a)), "autumn_bush")


def portal_dead_wood():
    clear_disc(175, 80, 3)
    put(175, 80, "portal")
    clear_disc(36, 80, 3)
    put(36, 80, "portal")


# ---------------------------------------------------------------- forest


def zone(c, r):
    if 155 <= c <= 192 and 24 <= r <= 100:
        return "dead"
    if math.hypot(c - GLEN[0], r - GLEN[1]) < 9:
        return "glen"
    rr = r + 10 * (fbm(c / 9, r / 9, 61) - 0.5)
    if rr < 42:
        return "north"
    if rr > 88:
        return "south"
    return "mid"


TREES = {
    "north": (("pine", 35), ("spruce", 25), ("fir", 25), ("scots", 15)),
    "mid": (("tree", 30), ("pine", 25), ("birch", 15), ("spruce", 15), ("scots", 15)),
    "south": (("tree", 45), ("birch", 30), ("scots", 15), ("pine", 10)),
    "glen": (("birch", 55), ("tree", 30), ("scots", 15)),
    "dead": (("snag", 70), ("scots", 30)),
}
UNDER = {
    "north": (("fern", 50), ("juniper", 30), ("bush", 20)),
    "mid": (("bush", 40), ("fern", 30), ("berry_bush", 20), ("juniper", 10)),
    "south": (("bush", 45), ("berry_bush", 35), ("fern", 20)),
    "glen": (("autumn_bush", 80), ("bush", 20)),
    "dead": (("tall_grass", 80), ("juniper", 20)),
}


def pick(table, u):
    total = sum(w for _, w in table)
    acc = 0
    for name, w in table:
        acc += w
        if u * total < acc:
            return name
    return table[-1][0]


def water_distance(limit=3):
    dist = {p: 0 for p in water}
    q = deque(water)
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


def tree_for(c, r, wd):
    z = zone(c, r)
    near_stream = wd.get((c, r), 9) <= 2 and math.hypot(c - VOLCANO[0], r - VOLCANO[1]) > 24
    if near_stream and z != "dead" and hash01(c, r, 71) < 0.55:
        return "willow"
    return pick(TREES[z], hash01(c, r, 73))


def forest():
    wd = water_distance()
    for r in range(H):
        for c in range(W):
            p = (c, r)
            if p in cells or p in reserved:
                continue
            if math.hypot(c - VOLCANO[0], r - VOLCANO[1]) < NO_TREES_NEAR_VOLCANO:
                if wd.get(p, 9) == 1 and hash01(c, r, 81) < 0.55:
                    put(c, r, "reeds")
                    reserved.discard(p)
                elif hash01(c, r, 83) < 0.10:
                    put(c, r, "tall_grass")
                    reserved.discard(p)
                continue
            if wd.get(p, 9) == 1 and hash01(c, r, 85) < 0.4:
                put(c, r, "reeds")
                reserved.discard(p)
                continue
            z = zone(c, r)
            d = fbm(c / 17, r / 17, 11)
            u = hash01(c, r, 13)
            if z == "dead":
                density = 0.22 if d > 0.5 else 0.08
            elif d > 0.52:
                density = 0.97 if d > 0.57 else 0.75
            elif d > 0.42:
                density = 0.32
            else:
                density = 0.05
            if z == "glen":
                density = min(density, 0.35)
            if u < density:
                put(c, r, tree_for(c, r, wd))
                reserved.discard(p)
                continue
            g = fbm(c / 7, r / 7, 17)
            v = hash01(c, r, 19)
            if d <= 0.42 and g > 0.6 and v < 0.8:
                put(c, r, "tall_grass")
            elif d > 0.4 and v < (0.5 if z == "glen" else 0.13):
                put(c, r, pick(UNDER[z], hash01(c, r, 23)))
            else:
                continue
            reserved.discard(p)


# ---------------------------------------------------------------- the nav grid
#
# The game routes on a grid of 32 px cells whose centres sit half a cell off
# the map's (`pathfind::Grid`, `maplint`): a tile at map cell (c, r) blocks
# nav cells c-1..c by r-1..r. So two trees one cell apart leave no nav cell
# between them, and a way the AI can take needs two empty map cells side by
# side. Everything below reasons on that grid, the way the linter does.

# a loose tile is no wall to the router: a hull shoves it out of the way
LOOSE = {"barrel", "hay", "wood_crate", "barrier"}
SOFT = {"road", "water", "pickup", "oil", "portal", "start", "start2", "gate"} | COVER_KINDS | LOOSE


def blocks(p):
    """Whether map cell `p` holds something the nav grid treats as a wall:
    any tile, deep water, the volcano's cone, either bong."""
    if not inb(*p):
        return False  # past the edge: no tile, and the edge's own margin is under half a cell
    if p in water:
        return is_deep(*p)
    k = kind_at(p)
    if k is None:
        return (p[0] - VOLCANO[0]) ** 2 + (p[1] - VOLCANO[1]) ** 2 <= 5
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
    """The nav cells round map cell `p`'s centre point."""
    c, r = p
    return [(c - 1, r - 1), (c, r - 1), (c - 1, r), (c, r)]


def portal_anchors():
    return sorted(p for p, t in cells.items() if '"portal"' in t)


def nav_reach(portals=True):
    """The playfield: every open nav cell reached from the players' start,
    through the portal ring as the planner goes through its hub."""
    anchors = portal_anchors() if portals else []
    feet = {a: [n for n in nav_of(a) if nav_open(*n)] for a in anchors}
    linked = {n: a for a, ns in feet.items() for n in ns}
    start = next(n for n in nav_of((27, 60)) if nav_open(*n))
    seen = {start}
    q = deque([start])
    hopped = set()
    while q:
        k, l = q.popleft()
        nxt = [(k + 1, l), (k - 1, l), (k, l + 1), (k, l - 1)]
        a = linked.get((k, l))
        if a is not None and a not in hopped:
            hopped.add(a)
            for b, ns in feet.items():
                if b != a:
                    nxt += ns
        for n in nxt:
            if n not in seen and nav_open(*n):
                seen.add(n)
                q.append(n)
    return seen


def reachable(p, reach):
    return any(n in reach for n in nav_of(p))


def connect(c, r, reach, mouth=None):
    """Cut a trail three cells wide from map cell (c, r) out to the
    playfield through trees and cover - two nav cells, a way the AI's
    router takes. `mouth` names the cover the last cells of it are hidden
    in."""
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
            if inb(*n) and n not in prev and (carvable(n) or not blocks(n)) and n not in intentional:
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


def hollows(count=12):
    """Caches in the thick of the woods: a clearing with something in it,
    reached by a trail out to the playfield whose mouth is hidden in
    bushes (`wire_up`)."""
    loot = ["laser", "minigun", "plasma", "missiles", "grenades", "speedup",
            "shield", "flamethrower", "minigun", "laser", "plasma", "missiles"]
    rng.shuffle(loot)
    made = []
    tries = 0
    while len(made) < count and tries < 20000:
        tries += 1
        c, r = rng.randint(6, W - 7), rng.randint(6, H - 7)
        if fbm(c / 17, r / 17, 11) < 0.56 or zone(c, r) == "dead":
            continue
        if any(math.hypot(c - gc, r - gr) < 24 for gc, gr in GATES):
            continue
        if any(math.hypot(c - mc, r - mr) < 18 for mc, mr in made):
            continue
        if math.hypot(c - HOME[0], r - HOME[1]) < 16:
            continue
        block = list(rect(c - 2, r - 2, c + 2, r + 2))
        if sum(kind_at(p) in TREE_KINDS for p in block) < 22:
            continue
        made.append((c, r))
    for (c, r), what in zip(made, loot):
        for p in rect(c - 2, r - 2, c + 2, r + 2):
            clear(*p)
        pickup(c, r, what)
        if hash01(c, r, 97) < 0.4:  # some are booby-trapped
            put(c - 2, r - 2, "barrel", drum="fuel")
            put(c + 2, r - 2, "barrel", drum="fuel")
    return made


def wire_up(hollow_cells):
    """Every pickup the players are meant to drive to gets a way in: the
    hollows a hidden trail each, anything else the woods closed off a
    plain one."""
    for c, r in hollow_cells:
        connect(c, r, nav_reach(), mouth="bush")
    for _ in range(4):
        reach = nav_reach()
        lost = [(c, r) for c, r, _ in pickups
                if (c, r) not in intentional and not reachable((c, r), reach)]
        if not lost:
            return
        for c, r in lost:
            connect(c, r, reach)


def fill_pockets():
    """Open ground the playfield never reaches becomes wood, so no nav
    cell is left boxed in or cut off - except round the island and the
    crypt, which are meant to be got to otherwise."""
    wd = water_distance()
    filled = 0
    for _ in range(8):
        reach = nav_reach()
        stray = [(k, l) for l in range(H) for k in range(W)
                 if (k, l) not in reach and nav_open(k, l)
                 and not any((k + dc, l + dr) in intentional for dc in (0, 1) for dr in (0, 1))]
        if not stray:
            break
        for k, l in stray:
            if not nav_open(k, l):
                continue
            corners = [(k + dc, l + dr) for dc in (0, 1) for dr in (0, 1)]
            # plant the corner whose other nav cells are off the playfield too
            corners.sort(key=lambda p: (sum(n in reach for n in nav_of(p)), p[1], p[0]))
            for p in corners:
                if (kind_at(p) is None or kind_at(p) in COVER_KINDS) and p not in water \
                        and p not in roads and p not in intentional:
                    put(*p, tree_for(*p, wd))
                    filled += 1
                    break
    return filled


def gate_aprons():
    """The lane a wave rolls in by: open five cells in from every gate."""
    for gc, gr in GATES:
        if gr == 0:
            clear_rect(gc - 2, 0, gc + 2, 5)
        elif gr == H - 1:
            clear_rect(gc - 2, H - 6, gc + 2, H - 1)
        elif gc == 0:
            clear_rect(0, gr - 2, 5, gr + 2)
        else:
            clear_rect(W - 6, gr - 2, W - 1, gr + 2)



# ---------------------------------------------------------------- the map


def build():
    lake_mere()
    river((118, 0), 50, 115, 121, stop_on_water=True)
    south_top = max(r for (c, r) in water if c == 106 and r < 85) + 1
    south = river((106, south_top), H - 1, 98, 110, drift=-0.15)
    river((168, H - 1), 90, 162, 172)
    pond(158, 82, 160, 84)
    pond(166, 80, 168, 81)

    # the King's Road, west to east, and the roads off it
    kings = road([(28, 62), (36, 62), (46, 56), (60, 49), (78, 43), (96, 42), (128, 41),
                  (140, 43), (150, 48), (162, 55), (178, 60), (192, 62)])
    road([(28, 66), (36, 74), (46, 84), (54, 99), (70, 100), (86, 98), (100, 97),
          (116, 99), (131, 104)])
    road([(149, 103), (160, 99), (174, 100)])
    road([(189, 99), (196, 86), (189, 78), (188, 67), (190, 64)])
    road([(78, 43), (82, 34), (90, 30), (95, 28)], width=1, margin=2)  # to the chapel
    road([(95, 15), (100, 12), (118, 8), (140, 6), (160, 10), (163, 13)], width=1, margin=2)
    road([(140, 43), (146, 34), (151, 30), (156, 24), (166, 16)])  # to the airfield
    road([(60, 49), (57, 42)], width=1, margin=2)  # to the maze
    road([(22, 50), (22, 46)], width=1, margin=2)  # to the orchard
    # gate lanes
    road([(78, 0), (79, 10), (82, 34)], width=1, margin=2)
    road([(140, 0), (140, 6)], width=1, margin=2)
    road([(212, 0), (213, 18), (204, 16)], width=1, margin=2)
    road([(222, 20), (213, 18)], width=1, margin=2)
    road([(222, 105), (200, 100), (189, 99)], width=1, margin=2)
    road([(30, 124), (40, 113), (46, 108)], width=1, margin=2)
    road([(62, 124), (66, 112), (70, 100)], width=1, margin=2)
    road([(125, 124), (126, 104)], width=1, margin=2)
    road([(165, 124), (160, 99)], width=1, margin=2)
    road([(205, 124), (200, 100)], width=1, margin=2)
    road([(0, 12), (24, 16), (34, 26), (36, 48), (46, 56)], width=1, margin=2)
    road([(0, 114), (20, 110), (36, 96), (46, 84)], width=1, margin=2)

    hollow_lodge()
    orchard()
    maze_exit = hedge_maze()
    road([maze_exit, (57, 42)], width=1, margin=2)
    ruined_chapel()
    tyre_circuit()
    great_mere()
    secret_grove()
    lumber_camp()
    beaver_dam(south)
    stone_circle()
    old_line()
    airfield()
    bandit_camp()
    black_keep()
    autumn_glen()
    portal_dead_wood()
    gate_aprons()
    for gc, gr in GATES:
        put(gc, gr, "gate")
        GATE_LANES.update(lane(gc, gr))
    # lamps down the King's Road for the night, on its north verge
    for c, r in kings[20:-10:16]:
        lamp = (c, r - 2)
        if inb(*lamp) and lamp not in cells and lamp not in water and lamp not in intentional:
            put(*lamp, "lamp")

    forest()
    made = hollows()
    wire_up(made)
    filled = fill_pockets()
    return filled, len(made)


HEADER = """\
# wildwood - the Wildwood: a green country of forest, meadow and water twice
# Longwater's size on each side, and the enemy's bong in a castle at the far
# end of it. Written by tools/gen_wildwood.py from a fixed seed - change the
# generator and run it again rather than editing this file; re-saving from
# the map editor would drop these comments. Level tables as dotted keys on
# purpose - a `[spawn]` header would swallow every `cells.` line after it.
#
# Hunt on the waves plan: ten waves of 4 to 13 tanks, light to super, rolling
# in through twelve gates round the edge. Night falls seven minutes in, and
# the lamps along the King's Road light the way - and show whoever stands in
# them to the enemy. 224 x 126 cells, followed by the camera.
#
# Level 7 of the campaign. West: the Hollow Lodge, the players' clearing -
# the bong in its fenced garden, the lodge (shield, bong health), the store,
# two gun towers and a tesla of your own, a pond, and by the start the
# weapons a campaign player meets first here: a gravity well, helicopter
# and jet strikes, a tower pack. North of it the orchard farm (minigun in
# the barn); in the north-west a hedge maze of firs, an EMP burst at its
# heart and something in three of its dead ends, one of them a trap. The Tyre
# Circuit, a race track walled with tyre stacks, speed on the track. The
# ruined chapel on the north lane: missiles and laser by the altar, and a
# crypt sealed in brick - shoot your way in for a second gravity well.
#
# Middle: the Great Mere, deep enough that no hull crosses it, a fishing pier
# off its north shore with plasma at the end. On the island in the middle a
# volcano sleeps and wakes, with an FPV swarm and the heat shield at its
# foot - no ford reaches it. No rod and no rail anywhere: on a Hunt either
# ends the hunt in one press. Four portals make a ring: one past the lodge,
# one on the island, one in a secret grove sealed in a fir wood north of the
# mere (one hidden trail in, its mouth in bushes off the King's Road; the
# FPV swarm and the tower pack inside), one in the Dead Wood. Each throws
# you out at any of the others.
#
# South: the lumber camp (flamethrower in the sawmill, grenades by the log
# piles, a shooting range) with an oil trail along the road to a stack of
# fuel drums; the beaver dam on the river out of the mere (laser); the
# autumn glen; the stone circle (EMP at its centre).
#
# East: the Old Line, tank traps from the airfield road to the south road
# with concrete blocks out in front to shove aside, gaps where the roads and
# three breaches go through it, an enemy gun tower in
# each of three sandbag bunkers and the sonic hammer left in an iron one
# behind them. The airfield in the north-east: helicopter and jet strikes in
# the hangar, speed in the glass control tower. The bandit camp in the
# south-east behind its ring of crates, tyres and drums (plasma, shield),
# guarded by a tesla and a gun tower. Then the Dead Wood, and the Black Keep
# behind its deep moat: one bridge, iron corners, four gun towers, two
# teslas and a bio slush, and the enemy's bong in a garden in the ward.
#
# A dozen hollows deep in the thickest woods each hold a cache at the end of
# a trail whose mouth is hidden in bushes; some are wired with fuel drums.
"""


def write(path):
    lines = [HEADER, "version = 1",
             'bong = "oggle"',
             'enemy_bong = "capi"',
             "size = [224, 126]",
             'tank = "titan"',
             'tank2 = "wraith"',
             'biome = "valley"',
             'weather = ["clear", "rain", "fog"]',
             "nightfall = 420.0",
             'mission.kind = "hunt"',
             'spawn.kind = "waves"',
             "spawn.waves = 10",
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
    for gc, gr in GATES:
        assert gc in (0, W - 1) or gr in (0, H - 1), (gc, gr)
    for c, r, kind in pickups:
        if kind in SPECIALS:
            near = min(math.hypot(c - gc, r - gr) for gc, gr in GATES)
            assert near >= 20.5, f"{kind} at {c},{r} is {near:.1f} cells from a gate"
    reach = nav_reach()
    walk = nav_reach(portals=False)
    near = lambda p, rr: any((p[0] + dc, p[1] + dr) in rr for dc in range(-2, 2) for dr in range(-2, 2))
    for p in ((27, 60), (27, 66)):
        assert near(p, walk), p
    assert near((206, 63), walk), "the enemy bong"
    isle = (VOLCANO[0] - 4, VOLCANO[1])
    assert not reachable(isle, walk) and reachable(isle, reach), "the isle is for the portals"
    for p in portal_anchors():
        assert reachable(p, reach), f"portal {p}"
    for c, r, kind in pickups:
        if (c, r) not in intentional:
            assert reachable((c, r), reach), f"{kind} at {c},{r} cannot be reached"
    for gc, gr in GATES:
        assert all(nav_open(*n) for n in lane(gc, gr)), f"gate {gc},{gr}"


GATE_LANES = set()


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


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("-o", "--out", default="maps/wildwood.toml")
    ap.add_argument("--stats", action="store_true")
    a = ap.parse_args()
    filled, made = build()
    check()
    write(a.out)
    if a.stats:
        kinds = {}
        for t in cells.values():
            k = t.split('"')[1]
            kinds[k] = kinds.get(k, 0) + 1
        print(f"{len(cells)} cells, {made} hollows, {filled} pocket cells filled")
        for k, n in sorted(kinds.items(), key=lambda kv: -kv[1]):
            print(f"  {k:12} {n}")


if __name__ == "__main__":
    main()
