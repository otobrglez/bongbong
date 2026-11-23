"""Write maps/vulkan.toml: Vulkan, level 13 - a volcano in an equatorial
rainforest, 90 x 52 cells, laid out region by region from a fixed seed.

  python3 tools/gen_vulkan.py [-o maps/vulkan.toml] [--stats] [--ascii]

A map this size is too big to lay out by hand, so this script is the
source: change it and run it again rather than editing the TOML (a
re-save from the map editor would drop the header too). The same seed
writes the same file every time - the noise is hashed from cell
positions, and every other draw comes from one seeded `random.Random`
in a fixed order. `--ascii` prints the map a character a cell, `--stats`
the counts and each gate's walk from the start.

What it lays out (the header written into the map says how it plays):

- the volcano in the middle and its three rivers of lava - west into a
  lava lake with a basalt island, north-east off the north edge, south off
  the south edge - so the lava splits the field in two, crossed at
  stone crossings or waded at a burn; the ash slopes round the cone;
- two jungle rivers of water: one down the west side into a lagoon,
  one down the east side past the bong's station;
- the Ranger Station in the south-east, the players' walled compound
  round the bong, with gun towers on its corners;
- the bai (a swampy forest clearing) in the north-west, the fishing
  village on the lagoon, the vulcanologists' hut on the north slope and
  the mine across the east river with its enemy tower;
- the rainforest everywhere else - broadleaf canopy, palms, willows on
  the banks, ferns and bushes underneath, elephant grass in the open -
  with trails cut through it and hidden caches at the end of a few.
"""

import argparse
import math
import random
from collections import deque
import heapq

W, H = 90, 52
SEED = 0xB0B220

rng = random.Random(SEED)

# (col, row) -> inline TOML table text
cells = {}
# cells the jungle pass must leave alone
reserved = set()
water = set()
lava = set()
roads = set()

VOLCANO = (44, 19)
BONG = (65, 41)
START = (61, 43)
START2 = (69, 43)
# bombs land within volcano_bomb_range_px (460 px, 14.4 cells) of the crater
ASH_RADIUS = 9.5

GATES = [
    (36, 0), (56, 0), (76, 0),  # north
    (89, 6),  # east
    (0, 30),  # west
    (4, 51),  # south-west
]

# what an enemy turns off its road to fetch lies beyond every gate's
# leash (`enemy_leash_px`, 640 px = 20 cells); enemies never take a heat
# shield, so those may lie anywhere
FETCHED = {
    "laser", "minigun", "plasma", "missiles", "flamethrower", "grenades",
    "sonic_hammer", "emp_burst", "gauss_rail", "fpv_swarm", "rod_from_god",
    "gravity_well", "air_heli", "air_jet", "tower_pack", "speedup", "shield",
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
    for layer, name in ((water, "water"), (lava, "lava"), (roads, "road")):
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
        lava.discard((c, r))
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


def wet(p):
    return p in water or p in lava


# the set pieces' open ground, which the verges' undergrowth keeps off
OPEN_GROUND = set()


def clear_disc(cx, cy, rad):
    """Open ground for a set piece: everything goes but roads, water and
    lava running through it."""
    for c, r in list(disc(cx, cy, rad)):
        if (c, r) not in roads and not wet((c, r)):
            clear(c, r)
        reserved.add((c, r))
        OPEN_GROUND.add((c, r))


def clear_rect(c0, r0, c1, r1):
    for c, r in list(rect(c0, r0, c1, r1)):
        if (c, r) not in roads and not wet((c, r)):
            clear(c, r)
        reserved.add((c, r))
        OPEN_GROUND.add((c, r))


def pickup(c, r, kind):
    assert not wet((c, r)) and (c, r) not in roads, f"{kind} at {c},{r} would cover water, lava or road"
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


def stream(points, layer):
    """A one-cell stream of `layer` ("water" or "lava") along `points`,
    rasterised 4-connected: a line one cell wide reads as a stream, never
    a lake."""
    path = polyline(points)
    for c, r in path:
        if inb(c, r):
            put(c, r, layer)
    return path


def meander(start, end_row, lo, hi, layer, salt, wander=0.4):
    """A one-cell stream from `start` down (or up) the rows to `end_row`,
    wandering between columns `lo` and `hi` (making for them first if it
    starts outside). It never turns straight back within two rows, so it
    never makes a 2 x 2 block and reads as a stream, not a lake."""
    c, r = start
    step = 1 if end_row > r else -1
    path = [(c, r)]
    side, since = 0, 9
    while r != end_row:
        want = 0
        if c > hi:
            want = -1
        elif c < lo:
            want = 1
        elif hash01(c, r, salt) < wander:
            want = -1 if hash01(r, c, salt + 1) < 0.5 else 1
        if want and not (side and want != side and since < 2):
            run = 1 + int(hash01(c, r, salt + 2) * 3)
            for _ in range(run):
                nc = c + want
                if lo <= nc <= hi or (want < 0 and c > hi) or (want > 0 and c < lo):
                    c = nc
                    path.append((c, r))
            side, since = want, 0
        r += step
        since += 1
        path.append((c, r))
    for p in path:
        if inb(*p):
            put(*p, layer)
    return path


def blob(cx, cy, rx, ry, layer, salt, keep=lambda c, r: True):
    """A lake: an ellipse with a ragged shore."""
    out = []
    for r in range(int(cy - ry) - 2, int(cy + ry) + 3):
        for c in range(int(cx - rx) - 2, int(cx + rx) + 3):
            if not inb(c, r) or not keep(c, r):
                continue
            dx, dy = (c - cx) / rx, (r - cy) / ry
            wobble = 0.5 * (fbm(c / 3, r / 3, salt) - 0.5)
            if dx * dx + dy * dy <= 1.0 + wobble:
                put(c, r, layer)
                out.append((c, r))
    return out


def road(points, width=2, margin=1):
    """A dirt road `width` cells across; a road cell over water is a
    bridge (a plank deck in the valley's look), over lava a stone
    crossing."""
    path = polyline(points)
    for c, r in path:
        for dc in range(width):
            for dr in range(width):
                if inb(c + dc, r + dr):
                    put(c + dc, r + dr, "road")
    for c, r in path:
        for dc in range(-margin, width + margin):
            for dr in range(-margin, width + margin):
                if inb(c + dc, r + dr):
                    reserved.add((c + dc, r + dr))
    return path


def trail(points, width=3, grass=0.0, salt=0):
    """A path cut through the jungle: a band `width` cells wide that the
    canopy leaves open, a one-cell dirt track down its middle, and on its
    edges undergrowth a hull pushes through - `grass` of them elephant
    grass, about a third ferns and bushes. Where it meets lava it crosses
    on stone (road cells across the stream); water it fords."""
    path = polyline(points)
    lo = -(width // 2)
    track = set(path)
    for c, r in path:
        for dc in range(lo, lo + width):
            for dr in range(lo, lo + width):
                p = (c + dc, r + dr)
                if p in lava:
                    put(*p, "road")
                    continue
                if not inb(*p) or p in roads or wet(p):
                    continue
                k = kind_at(p)
                if k is None or k in TREE_KINDS or k in COVER_KINDS:
                    clear(*p)
    for p in path:
        if inb(*p) and kind_at(p) is None and not wet(p):
            put(*p, "road")
    for c, r in path:
        for dc in range(lo, lo + width):
            for dr in range(lo, lo + width):
                p = (c + dc, r + dr)
                if not inb(*p) or p in track or kind_at(p) is not None:
                    continue
                u = hash01(*p, salt + 5)
                if u < grass:
                    put(*p, "tall_grass")
                elif u < grass + 0.45:
                    put(*p, pick(UNDER, hash01(*p, salt + 6)))
    return path


def verges():
    """The bare ground left along the roads grows undergrowth a hull
    pushes through: ferns, bushes, a little elephant grass."""
    for c, r in sorted(roads):
        for dc, dr in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            p = (c + dc, r + dr)
            if not inb(*p) or p in cells or p in roads or wet(p) or p in OPEN_GROUND:
                continue
            u = hash01(*p, 29)
            if u < 0.12:
                put(*p, "tall_grass")
            elif u < 0.45:
                put(*p, pick(UNDER, hash01(*p, 31)))


# ---------------------------------------------------------------- the lava


def volcano():
    vc, vr = VOLCANO
    put(vc, vr, "volcano")
    # the cone covers the cells within sqrt(5) of the crater: leave them
    # for the game to fill
    for c, r in rect(vc - 2, vr - 2, vc + 2, vr + 2):
        if (c - vc) ** 2 + (r - vr) ** 2 <= 5:
            reserved.add((c, r))


def lava_rivers():
    """The three rivers, each a run of straight reaches so that every
    crossing meets one square on: the west into the lava lake, the
    north-east off the north edge, the south off the south edge. Each
    starts beside the cone, which makes it a source."""
    # west, into the lava lake: crossed on the lake track at (36, 21)
    stream([(41, 19), (38, 19), (38, 21), (34, 21), (34, 22), (31, 22)], "lava")
    # the lake, its middle deep, and a basalt island in it a wade off the
    # south shore
    blob(25, 22, 6.0, 4.3, "lava", 7)
    for c, r in rect(24, 23, 25, 24):
        clear(c, r)
    # north-east, off the north edge: crossed on the hut road at (51, 13)
    # and on the north trail at (60, 4)
    stream([(47, 18), (48, 18), (48, 16), (51, 16), (51, 11), (54, 11), (54, 9), (57, 9), (57, 6),
            (60, 6), (60, 2), (63, 2), (63, 0)], "lava")
    # south, off the south edge toward the town nobody lives in any more:
    # crossed on the ash trail at (45, 32) and the west road at (43, 41)
    stream([(44, 22), (44, 25), (46, 25), (46, 29), (45, 29), (45, 35), (43, 35), (43, 46),
            (46, 46), (46, 49), (47, 49), (47, H - 1)], "lava")


# ---------------------------------------------------------------- the water


def jungle_rivers():
    # the west river, down into the lagoon
    meander((7, 0), 38, 6, 11, "water", 41)
    blob(12, 40, 6.6, 4.4, "water", 43)
    # the lagoon's outlet, off the south edge
    meander((13, 44), H - 1, 12, 16, "water", 47)
    # the east river: down from the north-east, then close past the
    # station's east wall - straight down column 77 past its east gate,
    # where the road bridges it - then off the south edge
    east = meander((82, 0), 18, 79, 85, "water", 53)
    east = meander((east[-1][0], 19), 34, 76, 78, "water", 59, wander=0.3)
    east = meander((east[-1][0], 35), 46, 77, 77, "water", 59)
    meander((east[-1][0], 47), H - 1, 76, 81, "water", 61)


def bridge_decks(was_water):
    """A road two cells wide over a stream one cell wide leaves no road
    cell with water on both sides, which is what the valley's look draws
    as a plank deck (`ground::GroundGrid::bridge`): give the stream back
    the second row (or column), so the first is a deck over it. The cell
    given back is a ford again, which a hull wades."""
    for c, r in sorted(was_water):
        if (c, r) not in roads:
            continue
        if ((c, r - 1) in water and (c, r + 1) in water) or ((c - 1, r) in water and (c + 1, r) in water):
            continue
        for n in ((c, r + 1), (c + 1, r)):
            if n in roads and n in was_water:
                put(*n, "water")
                break


# ---------------------------------------------------------------- set pieces


def ranger_station():
    """The players' compound: a limestone wall round the bong with gates
    north, west and east, gun towers on the north corners, a tesla by
    the bong, and the bong in a ring of sandbags."""
    c0, r0, c1, r1 = 57, 35, 73, 47
    # the station's clearing, from the southern lava to the east river,
    # and the clearing up the road where the north trails meet it
    clear_rect(45, r0 - 6, c1 + 1, r1 + 2)
    clear_rect(61, 24, 70, 28)
    gates = {(c, r0) for c in (64, 65, 66)} | {(c0, r) for r in (40, 41, 42)} | {(c1, r) for r in (40, 41, 42)}
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) in gates:
            continue
        wall(c, r, "brick")
    put(c0, r0, "gun_tower")
    put(c1, r0, "gun_tower")
    put(BONG[0], BONG[1] + 4, "tesla")
    put(*BONG, "bong")
    put(*START, "start")
    put(*START2, "start2")
    # the bong's ring: sandbags round it, open on every side
    bc, br = BONG
    for c, r in outline(bc - 2, br - 2, bc + 2, br + 2):
        if c == bc or r == br:
            continue
        put(c, r, "sandbag")
    # the stores along the walls
    pickup(59, 37, "ammo")
    pickup(59, 46, "health")
    pickup(71, 46, "health")
    pickup(71, 37, "bong_health")
    pickup(62, 46, "shield")
    pickup(68, 37, "sonic_hammer")
    for c, r in ((60, 46), (70, 46), (72, 45)):
        put(c, r, "wood_crate")
    # the armoury: ammo round the walls and helicopter strikes in the
    # corners, clear of the cross the gates' ways to the bong make (the
    # north gate down column 65, the west and east gates along rows 40-42),
    # the nearest strike 5 cells from the bong, past two of a rocket's blasts
    for c, r in ((61, 37), (70, 38), (59, 44), (71, 43), (67, 46)):
        pickup(c, r, "ammo")
    for c, r in ((58, 36), (72, 36), (64, 46)):
        pickup(c, r, "air_heli")
    # sandbag nests outside the north and west gates, a gap left for the
    # road (the east gate opens onto the bridge)
    for c, r in ((61, 33), (62, 33), (62, 32), (68, 33), (69, 33), (68, 32)):
        put(c, r, "sandbag")
    for c, r in ((55, 38), (55, 37), (54, 38), (55, 44), (55, 45), (54, 44)):
        put(c, r, "sandbag")
    # concrete barriers to shove into the way, and tank traps out front
    for c, r in ((60, 31), (70, 31), (53, 40), (53, 43)):
        put(c, r, "barrier")
    for c, r in ((58, 30), (72, 30)):
        put(c, r, "tank_trap")
    # elephant grass about the clearing, to wait in unseen, and a few palms
    for cx, cy, rad in ((54, 31, 2.2), (73, 30.5, 1.8), (53, 47, 1.6), (48, 34, 2.0), (49, 46, 1.6),
                        (62, 26.5, 1.3), (69, 27, 1.3)):
        for c, r in disc(cx, cy, rad):
            if (c, r) not in cells and not wet((c, r)) and hash01(c, r, 37) < 0.85:
                put(c, r, "tall_grass")
    for c, r in ((47, 38), (50, 30), (70, 25), (48, 48), (75, 31)):
        put_free(c, r, "palm")


def lagoon_village():
    """The fishing village on the lagoon's east shore: three huts, a jetty
    out into the water, a cassava patch fenced off and palms."""
    clear_rect(20, 37, 33, 48)
    huts = [(22, 38, 25, 40, (23, 40)), (28, 38, 31, 40, (29, 40)), (27, 44, 30, 46, (28, 44))]
    for c0, r0, c1, r1, door in huts:
        for c, r in outline(c0, r0, c1, r1):
            if (c, r) == door or (c == door[0] + 1 and r == door[1]):
                continue
            wall(c, r, "wood")
    pickup(23, 39, "minigun")
    pickup(29, 39, "ammo")
    pickup(28, 45, "health")
    # the jetty: road over the lagoon, a plank deck in the valley's look
    for c in range(17, 21):
        if (c, 42) in water or c >= 19:
            put(c, 42, "road")
    # the cassava patch
    for c, r in outline(21, 44, 25, 47):
        if c == 25 and r in (45, 46):
            continue  # its gate
        put(c, r, "fence")
    for c, r in rect(22, 45, 24, 46):
        put(c, r, "tall_grass")
    for c, r in ((26, 37), (32, 41), (20, 47), (32, 47), (33, 38)):
        put_free(c, r, "palm")
    for c, r in ((26, 41), (31, 43)):
        put_free(c, r, "hay")
    put_free(24, 42, "barrel", drum="oil")


def bai():
    """A bai: a swampy clearing in the forest, ringed by elephant grass, a
    pool in the middle and the missiles left by whoever camped here."""
    cx, cy = 15, 12
    clear_disc(cx, cy, 7.5)
    for c, r in disc(cx, cy, 7.5):
        d = math.hypot(c - cx, r - cy)
        if d > 4.2 and hash01(c, r, 71) < 0.8 and (c, r) not in water:
            put(c, r, "tall_grass")
    for c, r in rect(cx + 1, cy - 3, cx + 3, cy - 2):
        put(c, r, "water")
    for c, r in ((cx, cy - 3), (cx + 4, cy - 2), (cx + 2, cy - 4)):
        put_free(c, r, "reeds")
    pickup(cx - 1, cy + 1, "missiles")
    pickup(cx + 2, cy + 1, "health")
    for c, r in ((cx - 3, cy - 2), (cx + 3, cy + 3), (cx - 2, cy + 3)):
        put(c, r, "palm")


def lava_lake_shore():
    """The lake's island and shore: plasma on the basalt island, a wade of
    burning lava off the south shore, and a heat shield on the shore
    across from it to make the wade on."""
    pickup(24, 23, "plasma")
    pickup(25, 24, "health")
    pickup(27, 28, "heat_shield")
    for c, r in ((18, 20), (32, 25), (20, 26), (30, 17)):
        put_free(c, r, "snag")


def ash_slopes():
    """Round the cone: bare ash, dead snags thickening outward, boulders of
    grey stone (concrete, which the bombs wear down) to hide behind, and the
    prizes at the cone's foot."""
    vc, vr = VOLCANO
    for c, r in disc(vc, vr, ASH_RADIUS):
        reserved.add((c, r))
    boulders = [(39, 15), (40, 15), (40, 16), (50, 25), (51, 25), (51, 24), (38, 26), (39, 26),
                (53, 21), (53, 22), (49, 9), (50, 9)]
    for c, r in boulders:
        if (c, r) not in cells and not wet((c, r)):
            wall(c, r, "concrete")
    pickup(40, 24, "missiles")
    pickup(48, 21, "sonic_hammer")
    pickup(39, 16, "health")
    pickup(47, 16, "heat_shield")
    for c, r in disc(vc, vr, ASH_RADIUS):
        if (c, r) in cells or wet((c, r)) or (c - vc) ** 2 + (r - vr) ** 2 <= 5:
            continue
        if any((c + dc, r + dr) in lava for dc in (-1, 0, 1) for dr in (-1, 0, 1)):
            continue
        u = hash01(c, r, 77)
        dv = math.hypot(c - vc, r - vr)
        if u < (0.06 if dv <= 5 else 0.18 if dv <= 7.5 else 0.3):
            put(c, r, "snag")
        elif dv > 5 and u < 0.4:
            put(c, r, "tall_grass")


def observatory():
    """The vulcanologists' hut on the north slope, its roof long gone: a
    concrete shell with a heat shield and a health pack inside, within
    reach of the bombs."""
    c0, r0, c1, r1 = 42, 6, 47, 10
    clear_rect(c0 - 1, r0 - 1, c1 + 1, r1 + 1)
    for c, r in outline(c0, r0, c1, r1):
        if r == r1 and c in (44, 45):
            continue  # its door, onto the road
        if (c, r) in ((47, 7), (42, 9)):
            continue  # fallen in
        wall(c, r, "concrete")
    pickup(43, 8, "heat_shield")
    pickup(46, 8, "health")


def east_mine():
    """The mine across the east river: huts, fuel drums, crates, and an
    enemy gun tower in a sandbag nest guarding the plasma."""
    cx, cy = 84, 28
    clear_rect(cx - 4, cy - 6, cx + 4, cy + 6)
    put(cx, cy - 3, "gun_tower", side="enemy")
    for c, r in ((cx - 1, cy - 4), (cx + 1, cy - 4), (cx - 1, cy - 2), (cx + 1, cy - 2)):
        put(c, r, "sandbag")
    for c, r in outline(cx + 1, cy + 1, cx + 3, cy + 4):
        if (c, r) in ((cx + 1, cy + 2), (cx + 1, cy + 3)):
            continue
        wall(c, r, "wood")
    pickup(cx + 2, cy + 2, "plasma")
    pickup(cx - 2, cy + 1, "ammo")
    pickup(cx - 2, cy + 4, "health")
    for c, r in ((cx - 3, cy - 5), (cx - 3, cy - 4)):
        put(c, r, "barrel", drum="fuel")
    for c, r in ((cx + 3, cy - 1), (cx + 2, cy - 1)):
        put(c, r, "wood_crate")
    put(cx - 3, cy + 5, "tyres")
    put(cx - 2, cy + 5, "tyres")


def crossings():
    """Heat shields and cover at the crossings the waves come over."""
    pickup(51, 38, "heat_shield")
    for c, r in ((50, 44), (51, 44), (44, 38), (44, 37)):
        put_free(c, r, "sandbag")
    pickup(62, 8, "heat_shield")


# ---------------------------------------------------------------- the jungle


def distance_field(sources, limit, diagonal=False):
    dist = {p: 0 for p in sources}
    q = deque(sorted(sources))
    steps = ((1, 0), (-1, 0), (0, 1), (0, -1))
    if diagonal:
        steps += ((1, 1), (1, -1), (-1, 1), (-1, -1))
    while q:
        c, r = q.popleft()
        d = dist[(c, r)]
        if d >= limit:
            continue
        for dc, dr in steps:
            n = (c + dc, r + dr)
            if inb(*n) and n not in dist:
                dist[n] = d + 1
                q.append(n)
    return dist


CANOPY = (("tree", 74), ("palm", 20), ("willow", 6))
BANK = (("willow", 40), ("palm", 35), ("tree", 25))
UNDER = (("fern", 40), ("bush", 35), ("berry_bush", 25))


def tree_for(c, r, wd):
    if wd.get((c, r), 9) <= 2:
        return pick(BANK, hash01(c, r, 73))
    return pick(CANOPY, hash01(c, r, 79))


JUNGLE_GROUND = []


def canopy_noise(c, r):
    """How thick the forest stands at (c, r): high is canopy, low a glade.
    It thins toward the ash round the cone."""
    d = fbm(c / 7, r / 7, 11)
    if math.hypot(c - VOLCANO[0], r - VOLCANO[1]) < ASH_RADIUS + 2:
        d -= 0.08
    return d


def jungle():
    wd = distance_field(water, 4)
    ld = distance_field(lava, 3, diagonal=True)
    # the bands are quantiles of the noise over the ground the jungle may
    # take, so the share of canopy, thinner forest and glade is set here
    # whatever the noise's spread
    free = sorted(canopy_noise(c, r) for r in range(H) for c in range(W)
                  if (c, r) not in cells and (c, r) not in reserved)
    JUNGLE_GROUND.append(len(free))
    q = lambda share: free[int(len(free) * share)]
    glade, thin, mid = q(0.08), q(0.18), q(0.32)
    for r in range(H):
        for c in range(W):
            p = (c, r)
            if p in cells or p in reserved:
                continue
            dl = ld.get(p, 9)
            dw = wd.get(p, 9)
            u = hash01(c, r, 13)
            if dl <= 1:
                if u < 0.04:
                    put(c, r, "snag")
                    reserved.discard(p)
                continue
            if dl == 2:
                if u < 0.12:
                    put(c, r, "snag")
                elif u < 0.2:
                    put(c, r, "tall_grass")
                else:
                    continue
                reserved.discard(p)
                continue
            if dw == 1 and hash01(c, r, 85) < 0.45:
                put(c, r, "reeds")
                reserved.discard(p)
                continue
            d = canopy_noise(c, r)
            if d > mid:
                density = 0.9
            elif d > thin:
                density = 0.6
            elif d > glade:
                density = 0.3
            else:
                density = 0.05
            if u < density:
                put(c, r, tree_for(c, r, wd))
                reserved.discard(p)
                continue
            g = fbm(c / 5, r / 5, 17)
            v = hash01(c, r, 19)
            if d <= thin and g > 0.45 and v < 0.85:
                put(c, r, "tall_grass")
            elif d > glade and v < 0.6:
                put(c, r, pick(UNDER, hash01(c, r, 23)))
            elif v < 0.12:
                put(c, r, "fern")
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

LOOSE = {"barrel", "hay", "wood_crate", "barrier"}
SOFT = {"road", "water", "lava", "pickup", "oil", "portal", "start", "start2", "gate"} | COVER_KINDS | LOOSE
GATE_LANES = set()


def is_deep(c, r, layer):
    return all((c + dc, r + dr) in layer for dc in (-1, 0, 1) for dr in (-1, 0, 1))


def in_cone(p):
    return (p[0] - VOLCANO[0]) ** 2 + (p[1] - VOLCANO[1]) ** 2 <= 5


def blocks(p):
    """Whether map cell `p` holds something the nav grid treats as a wall:
    any tile, deep water or lava, the volcano's cone, the bong."""
    if not inb(*p):
        return False
    if p in water:
        return is_deep(*p, water)
    if p in lava:
        return is_deep(*p, lava)
    if in_cone(p):
        return True
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
    return p not in roads and not wet(p) and (k is None or k in TREE_KINDS or k in COVER_KINDS)


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


def hollows(count=3):
    """Caches in the thickest of the jungle: a little clearing with
    something in it, reached by a trail whose mouth is hidden in bushes."""
    loot = ["speedup", "shield", "minigun"]
    made = []
    tries = 0
    while len(made) < count and tries < 20000:
        tries += 1
        c, r = rng.randint(4, W - 5), rng.randint(4, H - 5)
        if any(math.hypot(c - gc, r - gr) < 22 for gc, gr in GATES):
            continue
        if any(math.hypot(c - mc, r - mr) < 16 for mc, mr in made):
            continue
        if math.hypot(c - VOLCANO[0], r - VOLCANO[1]) < 16 or math.hypot(c - BONG[0], r - BONG[1]) < 14:
            continue
        block = list(rect(c - 2, r - 2, c + 2, r + 2))
        if sum(kind_at(p) in TREE_KINDS for p in block) < 20:
            continue
        made.append((c, r))
    for (c, r), what in zip(made, loot):
        for p in rect(c - 1, r - 1, c + 1, r + 1):
            clear(*p)
        for p in outline(c - 2, r - 2, c + 2, r + 2):
            if kind_at(p) in TREE_KINDS and hash01(*p, 91) < 0.35:
                clear(*p)
        pickup(c, r, what)
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
    """Open ground the playfield never reaches becomes jungle, so no nav
    cell is left boxed in or cut off."""
    wd = distance_field(water, 4)
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
                if (kind_at(p) is None or kind_at(p) in COVER_KINDS) and not wet(p) and p not in roads \
                        and not in_cone(p):
                    ash = math.hypot(p[0] - VOLCANO[0], p[1] - VOLCANO[1]) < ASH_RADIUS + 1
                    put(*p, "snag" if ash else tree_for(*p, wd))
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
                if p in cells or in_cone(p):
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
    """The lane a wave rolls in by: the trees and cover cleared five cells
    in from every gate (a wall a set piece put there stays)."""
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


def step_cost(k, l):
    """What the router charges to step into nav cell (k, l): a lava ford
    `lava_ford_path_cost` (14), a water ford `water_ford_path_cost` (3),
    dry ground 1. A ford prices the nav cell its map cell's centre falls
    in (`Grid::weigh`), which is nav cell (k, l) for map cell (k, l)."""
    if (k, l) in lava:
        return 14
    if (k, l) in water:
        return 3
    return 1


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
    volcano()
    lava_rivers()
    jungle_rivers()

    # Every road and trail runs in straight reaches along the rows and
    # columns: a hull drives four ways, and a diagonal corridor makes it
    # stair-step all the way along.
    # The roads out of the station: west over the southern lava to the
    # village, north up the volcano's east flank and over the north-east
    # lava to the hut, east over the river to the mine.
    was_water = set(water)
    road([(56, 41), (33, 41)])
    road([(65, 34), (65, 24), (59, 24), (59, 13), (44, 13), (44, 11)])
    road([(74, 41), (83, 41), (83, 34)])
    bridge_decks(was_water)
    # the trails
    trail([(40, 41), (40, 36), (36, 36), (36, 0)], grass=0.3, salt=1)
    trail([(36, 32), (56, 32)], grass=0.3, salt=2)
    trail([(42, 12), (37, 12)], salt=3)
    trail([(36, 4), (76, 4), (76, 0)], grass=0.2, salt=4)
    trail([(56, 4), (56, 0)], salt=13)
    trail([(68, 4), (68, 10), (71, 10), (71, 24), (66, 24)], grass=0.25, salt=5)
    trail([(36, 10), (22, 10)], grass=0.3, salt=6)
    trail([(0, 30), (17, 30)], grass=0.3, salt=7)
    trail([(17, 19), (17, 30), (22, 30), (22, 36)], grass=0.2, salt=8)
    trail([(24, 48), (4, 48), (4, 51)], grass=0.3, salt=9)
    trail([(84, 22), (86, 22), (86, 6), (89, 6)], grass=0.2, salt=10)
    trail([(74, 49), (84, 49), (84, 43)], salt=11)
    trail([(71, 16), (86, 16)], grass=0.2, salt=12)

    ash_slopes()
    observatory()
    ranger_station()
    lagoon_village()
    bai()
    lava_lake_shore()
    east_mine()
    crossings()
    gate_aprons()
    for gc, gr in GATES:
        put(gc, gr, "gate")
        GATE_LANES.update(lane(gc, gr))

    verges()
    jungle()
    made = hollows()
    wire_up(made)
    filled = fill_pockets()
    close_narrow_trails()
    return filled, made


HEADER = """\
# vulkan - Vulkan, level 13: a volcano in the rainforest (docs/volcano.md).
# Written by tools/gen_vulkan.py from a fixed seed - change the generator and
# run it again rather than editing this file; re-saving from the map editor
# would drop these comments. Level tables as dotted keys on purpose - a
# `[spawn]` header would swallow every `cells.` line after it.
#
# Protect on the waves plan: four waves of 3 to 6 tanks, light to heavy, in
# daylight under a clear sky (the eruptions are the spectacle - no night, no
# lamps). 90 x 52 cells, followed by the camera, in the valley's look: teal
# jungle rivers, silt and gravel on their banks.
#
# The volcano stands in the middle and wakes every half minute, throwing lava
# bombs round itself and at anyone near. Three rivers of lava leave it and cut
# the field in two: west into a lava lake whose middle is deep, north-east off
# the north edge, south off the south edge. Each is a burning ford crossed at
# a wade, or dry at a stone crossing where a road or trail meets it - five of
# them, three inside the bombs' reach. Round the cone a ring of dead snags and
# ash, concrete-grey boulders to hide behind, missiles and a second sonic
# hammer at its foot, a heat shield and a health pack in the vulcanologists'
# roofless hut on the north slope. Plasma and a health pack wait on a basalt
# island in the lava lake, a wade of burning lava off its south shore, and a
# heat shield for the wade lies on that shore.
#
# The players start in the Ranger Station in the south-east: a limestone wall
# round the bong's sandbag ring with gates north, west and east, two gun
# towers on its north corners and a tesla by the bong, sandbag nests and
# barriers outside the gates, the stores inside (six ammo crates round the
# walls, three helicopter strikes in its corners and by its south wall,
# health, shield, a bong health pack and the sonic hammer - shove a tank
# into the lava). Its
# clearing runs from the southern lava to the east river, which passes the
# east gate under a bridge. The road west crosses the southern lava to the
# fishing village on the lagoon (minigun in the first hut), the road north
# climbs the volcano's east flank and crosses the north-east lava to the hut.
# Across the east river the mine, plasma in its shed, guarded by an enemy gun
# tower.
#
# Everything else is rainforest: broadleaf canopy that hides a hull, palms,
# willows on the banks, ferns and bushes underneath that slow a hull, and
# elephant grass in the clearings to sit in unseen - the bai in the
# north-west (missiles), the trails the waves come down, and three caches at
# the ends of hidden trails whose mouths are bushes. The waves roll in at six
# gates - three on the north edge, one east, one west, one south-west - and
# the ones from the west half have to cross the lava to reach the station.
"""


def write(path):
    lines = [HEADER, "version = 1",
             'bong = "spike"',
             'tank = "warden"',
             'tank2 = "scout"',
             'biome = "valley"',
             "size = [90, 52]",
             'mission.kind = "protect"',
             'spawn.kind = "waves"',
             "spawn.waves = 4",
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
    for p in water:
        assert not any((p[0] + dc, p[1] + dr) in lava for dc in (-1, 0, 1) for dr in (-1, 0, 1)), f"water {p} meets lava"
    reach = nav_reach()
    # the bong sits in a tight sandbag ring, so the nearest open nav cells
    # are a few cells off: the linter's own reach test is the judge
    near = lambda p, rr: any((p[0] + dc, p[1] + dr) in rr for dc in range(-4, 4) for dr in range(-4, 4))
    for p in (START, START2, BONG):
        assert near(p, reach), p
    for c, r, kind in pickups:
        assert reachable((c, r), reach), f"{kind} at {c},{r} cannot be reached"
    for gc, gr in GATES:
        assert all(nav_open(*n) for n in lane(gc, gr)), f"gate {gc},{gr}"
        assert lane(gc, gr)[-1] in reach, f"gate {gc},{gr} is cut off"


ASCII = {
    'wall': '#', 'road': '=', 'water': '~', 'lava': 'L', 'bong': 'B', 'start': '1', 'start2': '2',
    'pickup': 'P', 'sandbag': 's', 'fence': 'f', 'tree': 'T', 'palm': 'Y', 'snag': 'x', 'willow': 'W',
    'birch': 'b', 'tall_grass': '"', 'bush': 'o', 'berry_bush': 'o', 'fern': ',', 'reeds': '|',
    'volcano': 'V', 'gate': 'G', 'tesla': 'Z', 'gun_tower': 'K', 'barrel': 'D', 'hay': 'h', 'tyres': 'O',
    'wood_crate': 'X', 'barrier': 'n', 'tank_trap': '*',
}


def ascii_map():
    print("    " + "".join(str(c // 10 % 10) for c in range(W)))
    print("    " + "".join(str(c % 10) for c in range(W)))
    for r in range(H):
        row = []
        for c in range(W):
            k = kind_at((c, r))
            ch = ASCII.get(k, '?') if k else ('^' if in_cone((c, r)) else '.')
            if k == 'wall':
                ch = {'brick': '#', 'iron': 'I', 'wood': 'w', 'concrete': 'C'}.get(cells[(c, r)].split('"')[3], '#')
            if k == 'lava' and is_deep(c, r, lava):
                ch = '@'
            if k == 'water' and is_deep(c, r, water):
                ch = '%'
            row.append(ch)
        print(f"{r:3d} " + "".join(row))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("-o", "--out", default="maps/vulkan.toml")
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
        print(f"{len(cells)} cells, {JUNGLE_GROUND[0]} left to the jungle, {len(reserved)} reserved, "
              f"hollows at {made}, {filled} pocket cells filled")
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
