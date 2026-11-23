"""Write maps/jungle-temple.toml: Jungle Temple, level 6 - a stepped temple
on a moated island in the jungle, 83 x 42 cells, laid out piece by piece
from a fixed seed.

  python3 tools/gen_jungle_temple.py [-o maps/jungle-temple.toml] [--stats] [--ascii]

A map this size is too big to lay out by hand, so this script is the
source: change it and run it again rather than editing the TOML (a
re-save from the map editor would drop the header too). The same seed
writes the same file every time - the noise is hashed from cell
positions, and every other draw comes from one seeded `random.Random`
in a fixed order. `--ascii` prints the map a character a cell, `--stats`
the counts and each gate's walk from the start.

What it lays out (the header written into the map says how it plays):

- the temple island in the middle: a moat round it, a reservoir (a baray)
  off its west side, four stone causeways and three silted fords across;
  the outer enclosure wall, broken in places; the courtyards inside it with
  their galleries; the stepped terrace and, on top of it, the sanctum with
  the bong in a sandbag ring and the players' towers;
- two side shrines outside the moat, across the north and the south
  causeway, each an enemy-held stash;
- a ring of jungle trails round the moat, joined to six gates on all four
  edges, and two ruins in the outer jungle with the player-only crates;
- the jungle everywhere else - broadleaf canopy, palms, willows on the
  banks, ferns and flowering bushes underneath, elephant grass in the
  glades - with hidden caches at the ends of a few trails.
"""

import argparse
import heapq
import math
import random
from collections import deque

W, H = 83, 42
SEED = 0xB0B228

rng = random.Random(SEED)

# (col, row) -> inline TOML table text
cells = {}
# cells the jungle pass must leave alone
reserved = set()
water = set()
roads = set()

# The island the temple stands on, its enclosure wall on its edge.
ISLAND = (22, 13, 60, 29)
# The stepped terrace's wall; the sanctum is inside it.
TERRACE = (30, 17, 52, 25)
BONG = (41, 21)
# The seats start either side of the bong, on its row: a seat's sight box
# (an enemy fires at a seat only from inside it) then ends in the
# courtyards on every side, never out over the moat, where a tank on the
# far bank would wait for a way in that is not there.
START = (36, 21)
START2 = (46, 21)

GATES = [
    (5, 0), (77, 0),    # north
    (0, 32),            # west
    (82, 12),           # east
    (6, 41), (80, 41),  # south
]

# what an enemy turns off its road to fetch lies beyond every gate's
# leash (`enemy_leash_px`, 640 px = 20 cells): the weapons it carries,
# speed, shield and the bong pack it takes to deny the players. The
# flamethrower and the grenades are the players' alone, so those may lie
# anywhere.
FETCHED = {
    "laser", "minigun", "plasma", "missiles", "sonic_hammer", "emp_burst", "gauss_rail",
    "fpv_swarm", "rod_from_god", "gravity_well", "tower_pack", "speedup", "shield", "bong_health",
}
# a blast weapon's crate keeps 256 px (eight cells) and more from the
# players' bong (docs/levels.md: two of the grenade's blast radii), so the
# first fight it is used in is not at the bong
BLAST_CRATES = {"grenades": 8.0, "missiles": 8.0, "fpv_swarm": 8.0, "air_jet": 8.0, "air_heli": 8.0}
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


def wet(p):
    return p in water


# the set pieces' open ground, which the verges' undergrowth keeps off
OPEN_GROUND = set()


def clear_rect(c0, r0, c1, r1):
    for c, r in list(rect(c0, r0, c1, r1)):
        if (c, r) not in roads and not wet((c, r)):
            clear(c, r)
        reserved.add((c, r))
        OPEN_GROUND.add((c, r))


def pickup(c, r, kind):
    assert not wet((c, r)) and (c, r) not in roads, f"{kind} at {c},{r} would cover water or road"
    put(c, r, "pickup", pickup=kind)
    pickups.append((c, r, kind))


def drums(spots, salt):
    """A cluster of drums: fuel and oil mixed, hashed per cell, so a chain
    both burns and throws."""
    for c, r in spots:
        if inb(c, r) and not wet((c, r)) and (c, r) not in roads:
            put(c, r, "barrel", drum="fuel" if hash01(c, r, salt) < 0.45 else "oil")


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


def meander(start, end_row, lo, hi, salt, wander=0.4, stop=None):
    """A one-cell stream from `start` down (or up) the rows to `end_row`,
    wandering between columns `lo` and `hi`. It never turns straight back
    within two rows, so it never makes a 2 x 2 block and reads as a
    stream, not a lake. `stop` ends it early at the first cell it names."""
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
                if lo <= nc <= hi:
                    c = nc
                    path.append((c, r))
            side, since = want, 0
        r += step
        since += 1
        path.append((c, r))
        if stop and stop((c, r)):
            break
    for p in path:
        if inb(*p) and p not in roads:
            put(*p, "water")
    return path


def road(points, width=3):
    """A stone causeway or a processional way `width` cells across (a
    road cell over water is dry ground, so a causeway over the moat is
    one)."""
    path = polyline(points)
    lo = -(width // 2)
    for c, r in path:
        for dc in range(lo, lo + width):
            for dr in range(lo, lo + width):
                if inb(c + dc, r + dr):
                    put(c + dc, r + dr, "road")
    return path


def trail(points, width=3, grass=0.0, salt=0):
    """A path cut through the jungle: a band `width` cells wide that the
    canopy leaves open, a one-cell dirt track down its middle, and on its
    edges undergrowth a hull pushes through - `grass` of them elephant
    grass, about a third ferns and bushes. Water it fords."""
    path = polyline(points)
    lo = -(width // 2)
    track = set(path)
    for c, r in path:
        for dc in range(lo, lo + width):
            for dr in range(lo, lo + width):
                p = (c + dc, r + dr)
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
                elif u < grass + 0.35:
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
            elif u < 0.4:
                put(*p, pick(UNDER, hash01(*p, 31)))


# ---------------------------------------------------------------- the water


def moat():
    """The moat: three cells wide all round the island, so its middle is
    open water (a wall to hulls, nothing to shots); the baray, a broad
    reservoir, off its west side; and three stretches silted up to two
    cells wide, which are fords."""
    c0, r0, c1, r1 = ISLAND
    for c, r in rect(c0 - 3, r0 - 3, c1 + 3, r1 + 3):
        if not (c0 <= c <= c1 and r0 <= r <= r1):
            put(c, r, "water")
    # the baray: the west side widened into a reservoir, its corners
    # rounded off
    for c, r in rect(13, 14, c0 - 4, 28):
        if (c, r) in ((13, 14), (13, 28)):
            continue
        put(c, r, "water")
    # the silted stretches: the outer row of the moat gone to bank
    for c, r in [(c, r0 - 3) for c in range(23, 28)] + [(c, r1 + 3) for c in range(55, 60)] \
            + [(c1 + 3, r) for r in range(24, 28)]:
        clear(c, r)
        OPEN_GROUND.add((c, r))


def streams():
    """Jungle streams: one from the north edge down into the moat, the
    moat's overflow off the south edge from its south-east corner, and
    the baray's off the south edge from its south-west corner."""
    meander((29, 0), 9, 28, 31, 41, wander=0.35)
    meander((63, 34), H - 1, 62, 66, 47, wander=0.35)
    meander((14, 29), H - 1, 13, 16, 53, wander=0.35)


# ---------------------------------------------------------------- the temple


def temple():
    """The island: the enclosure wall on its edge, broken by four gates
    where the causeways land, three collapses where the fords come up and
    two more that fell into the moat; the courtyards; the stepped
    terrace; the sanctum on top."""
    c0, r0, c1, r1 = ISLAND
    clear_rect(c0, r0, c1, r1)
    gates = set()
    gates |= {(c0, r) for r in (19, 20, 21)}          # west, the long causeway
    gates |= {(c1, r) for r in (18, 19, 20)}          # east
    gates |= {(c, r0) for c in (39, 40, 41, 42, 43)}  # north, to the north shrine
    gates |= {(c, r1) for c in (48, 49, 50, 51, 52)}  # south, to the south shrine
    fallen = set()
    fallen |= {(c, r0) for c in (24, 25, 26)}      # the north-west ford
    fallen |= {(c, r1) for c in (56, 57, 58)}      # the south-east ford
    fallen |= {(c1, r) for r in (24, 25, 26)}      # the east ford
    fallen |= {(c, r0) for c in (52, 53)}          # into the moat
    fallen |= {(c, r1) for c in (29, 30)}          # into the moat
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) in gates or (c, r) in fallen:
            continue
        wall(c, r, "brick")
    # the gate towers: grey stone either side of each gate, and the broken
    # ends of the collapses
    for c, r in ((c0, 18), (c0, 22), (c1, 17), (c1, 21), (38, r0), (44, r0), (47, r1), (53, r1),
                 (23, r0), (27, r0), (55, r1), (59, r1), (c1, 23), (c1, 27)):
        wall(c, r, "concrete")
    # rubble the collapses threw inward, against the walls where it
    # narrows no way through
    for c, r in ((23, 15), (28, 14), (54, 14), (59, 22), (59, 28), (23, 29)):
        if (c, r) not in cells:
            wall(c, r, "concrete")
    terrace()
    sanctum()
    courtyards()


def terrace():
    """The stepped terrace: a limestone wall with its corners built out in
    grey stone, a step down to the west and east courtyards, and a stair
    through three sides - none in line with the gate below it, so whoever
    comes up a causeway has a courtyard to cross under the towers first.
    The west side has none: the long causeway's waves go round to the
    north or the south stair."""
    c0, r0, c1, r1 = TERRACE
    stairs = set()
    stairs |= {(c, r0) for c in (34, 35, 36)}  # north
    stairs |= {(c, r1) for c in (46, 47, 48)}  # south
    stairs |= {(c1, r) for r in (21, 22, 23)}  # east
    for c, r in outline(c0, r0, c1, r1):
        if (c, r) not in stairs:
            wall(c, r, "brick")
    # the steps: each corner in grey stone and built out a step into the
    # wide courtyards (never into the narrow north and south ones)
    for cc, rr, dc, dr in ((c0, r0, -1, 1), (c1, r0, 1, 1), (c0, r1, -1, -1), (c1, r1, 1, -1)):
        for c, r in ((cc, rr), (cc + dc, rr), (cc + dc, rr + dr)):
            wall(c, r, "concrete")


def sanctum():
    """The top of the terrace, the players' base: the bong in a low ring of
    sandbags open only north and south, where the terrace wall stands in
    the way of any shot down a road, the players' towers round it, the
    stores, air support, and the blast weapons at its far ends."""
    bc, br = BONG
    put(bc, br, "bong")
    for c, r in outline(bc - 2, br - 1, bc + 2, br + 1):
        if c == bc:
            continue
        put(c, r, "sandbag")
    put(*START, "start")
    put(*START2, "start2")
    # the players' towers: guns on the north-west and south-east, a tesla
    # over the east stair, the bio slush lobbing over the west wall
    put(33, 19, "gun_tower")
    put(49, 24, "gun_tower")
    put(49, 20, "tesla")
    put(33, 23, "bio_slush")
    # the stores
    pickup(37, 19, "bong_health")
    pickup(45, 19, "health")
    pickup(35, 21, "ammo")
    pickup(45, 23, "shield")
    pickup(37, 23, "health")
    # air support and the blast weapons, eight cells and more from the bong
    pickup(31, 21, "air_jet")
    pickup(51, 20, "air_heli")
    pickup(51, 24, "missiles")
    pickup(31, 24, "grenades")
    # the braziers at the terrace corners (lamps: they light the sanctum
    # at dusk and in a storm, and show whoever stands in their light)
    for c, r in ((31, 18), (51, 18)):
        put(c, r, "lamp")


def courtyards():
    """Between the enclosure and the terrace: the west and east
    courtyards with their galleries, pools and elephant grass, the narrow
    north and south ones, ruin trees and the players' outer towers over
    the long causeway and the east gate."""
    c0, r0, c1, r1 = ISLAND
    # the players' outer gun towers, one over each of the two long
    # approaches
    put(26, 16, "gun_tower")
    put(56, 22, "gun_tower")
    # sandbag nests inside the west gate, a gap left for the way
    for c, r in ((24, 17), (25, 17), (24, 23), (25, 23)):
        if (c, r) not in cells:
            put(c, r, "sandbag")
    # the west gallery, against the enclosure south of the west gate: wood
    # on three sides, open to the courtyard, its stash inside
    for c, r in list(outline(c0, 24, 26, 28)):
        if c == 26 and r in (25, 26, 27):
            continue  # its open side
        if (c, r) not in cells:
            wall(c, r, "wood")
    pickup(23, 25, "sonic_hammer")
    pickup(23, 27, "missiles")
    pickup(24, 26, "health")
    # the north-east gallery, the same against the enclosure's corner
    for c, r in list(outline(55, r0, c1, r0 + 3)):
        if r == r0 + 3 and c in (56, 57, 58):
            continue
        if (c, r) not in cells:
            wall(c, r, "wood")
    pickup(57, r0 + 1, "laser")
    pickup(58, r0 + 1, "plasma")
    pickup(56, r0 + 1, "ammo")
    # the near stash, below the terrace's south stair: the swarm and the
    # miniguns that are its counter, inside the start's sight box
    pickup(35, 27, "fpv_swarm")
    pickup(33, 27, "minigun")
    pickup(37, 27, "minigun")
    # more on the courtyards' floors
    pickup(28, 20, "plasma")
    pickup(54, 26, "health")
    pickup(45, 15, "bong_health")
    pickup(30, 15, "speedup")
    pickup(49, 15, "grenades")
    # the pools: shallow reflecting pools in the west and east courtyards
    for c, r in rect(24, 14, 25, 15):
        if (c, r) not in cells:
            put(c, r, "water")
    for c, r in rect(56, 26, 57, 27):
        if (c, r) not in cells:
            put(c, r, "water")
    for c, r in ((26, 14), (23, 16), (55, 27), (58, 27)):
        put_free(c, r, "reeds")
    # ruin trees growing out of the enclosure's corners
    for c, r in ((c0 + 1, r1 - 1), (c1 - 1, r1 - 1)):
        put_free(c, r, "tree")
    put_free(c0 + 1, r0 + 1, "palm")
    # elephant grass in the courtyards
    for cx, cy, rad in ((28, 23.5, 1.6), (54, 20, 1.3), (44, 27, 1.2), (32, 15, 1.0), (50, 15, 1.1)):
        for c, r in disc(cx, cy, rad):
            if (c, r) not in cells and hash01(c, r, 37) < 0.8:
                put(c, r, "tall_grass")
    for c, r in ((27, 28), (59, 20), (32, 28), (36, 14), (48, 16)):
        put_free(c, r, "fern")
    for c, r in ((47, 28), (25, 21)):
        put_free(c, r, "autumn_bush")


# ---------------------------------------------------------------- the shrines


def north_shrine():
    """The north shrine, on the temple's axis across the north causeway: a
    walled court held by an enemy gun tower, its drums beside it, and the
    laser, the plasma, missiles and a health pack inside."""
    c0, r0, c1, r1 = 35, 0, 47, 4
    clear_rect(c0 - 1, r0, c1 + 1, r1 + 1)
    for c, r in outline(c0, r0, c1, r1):
        if r == r1 and c in (40, 41, 42):
            continue  # its door, onto the trail and the causeway
        wall(c, r, "brick")
    for c, r in ((c0, r1), (c1, r1), (39, r1), (43, r1)):
        wall(c, r, "concrete")
    put(41, 1, "gun_tower", side="enemy")
    drums([(39, 1), (43, 1)], 3)
    pickup(37, 2, "laser")
    pickup(45, 2, "missiles")
    pickup(37, 3, "minigun")
    pickup(45, 3, "health")


def south_shrine():
    """The south shrine, off the axis across the south causeway: grey
    stone, older, held by an enemy bio slush, with the EMP, a minigun, a
    shield and ammunition inside."""
    c0, r0, c1, r1 = 44, 37, 56, 41
    clear_rect(c0 - 1, r0 - 1, c1 + 1, r1)
    for c, r in outline(c0, r0, c1, r1):
        if r == r0 and c in (49, 50, 51):
            continue  # its door
        wall(c, r, "concrete")
    put(50, 40, "bio_slush", side="enemy")
    drums([(47, 40), (53, 40)], 5)
    pickup(46, 39, "emp_burst")
    pickup(54, 39, "missiles")
    pickup(46, 40, "shield")
    pickup(54, 40, "health")


def west_ruin():
    """A ruin in the west jungle, its walls broken and choked with rubble,
    its east side fallen open to the west trail: an enemy gun tower in the
    corner still standing, its drums
    beside it, and the players' crates inside - a flamethrower, grenades
    and a health pack."""
    clear_rect(1, 8, 9, 17)
    standing = [(1, 8), (2, 8), (3, 8), (1, 9), (1, 10), (1, 11), (7, 8), (8, 8), (8, 9),
                (1, 14), (1, 15), (1, 16), (2, 16), (3, 16), (6, 16), (7, 16), (8, 16), (8, 15)]
    for c, r in standing:
        wall(c, r, "brick")
    # the fallen stretches patched with rubble: a gap one cell wide is no
    # way through for the router, only something for a hull to grind on
    for c, r in ((4, 8), (5, 8), (6, 8), (1, 12), (1, 13), (4, 16), (5, 16)):
        wall(c, r, "concrete")
    put(2, 9, "gun_tower", side="enemy")
    drums([(3, 10), (2, 11)], 7)
    pickup(4, 12, "flamethrower")
    pickup(6, 13, "grenades")
    pickup(4, 14, "health")


def east_ruin():
    """A ruin in the east jungle: a broken shed on old stone, an enemy
    tesla at its door, grenades, a flamethrower and ammunition inside,
    and its drums at the back."""
    c0, r0, c1, r1 = 74, 26, 81, 32
    clear_rect(c0, r0, c1, r1)
    for c, r in outline(c0 + 2, r0 + 1, c1, r1 - 1):
        if c == c0 + 2 and r in (28, 29, 30):
            continue  # its door, west
        if (c, r) in ((79, 27), (81, 29)):
            wall(c, r, "concrete")  # fallen in, the gap choked with rubble
            continue
        wall(c, r, "wood" if r in (r0 + 1, r1 - 1) else "brick")
    put(75, 27, "tesla", side="enemy")
    # the crates and drums along the back wall, the floor before them clear
    pickup(80, 28, "grenades")
    pickup(80, 30, "flamethrower")
    pickup(74, 31, "ammo")
    drums([(80, 29), (79, 28), (79, 30)], 9)


# ---------------------------------------------------------------- traps


def traps():
    """Drum clusters where the waves cross: beside the far end of the west
    and east causeways and either side of the north and south shrines, to
    set off under whoever bunches there; barriers to shove into a rush."""
    # off the trails' bands, so no hull shoves them about on its way past
    drums([(8, 17), (8, 18), (9, 18), (8, 22), (8, 23), (9, 22)], 11)  # the long causeway
    drums([(65, 13), (66, 13), (65, 19), (66, 19), (65, 20)], 13)  # the east causeway
    drums([(34, 3), (34, 4), (48, 3), (48, 4)], 15)  # either side of the north shrine
    drums([(42, 38), (42, 39), (58, 38), (58, 39)], 17)  # either side of the south shrine
    for c, r in ((9, 16), (9, 24), (67, 14), (67, 18)):
        put_free(c, r, "barrier")


def ford_ways():
    """The trails down to the three fords, cut after the temple so they
    run on through the collapses into the courtyards."""
    trail([(25, 7), (25, 14)], grass=0.2, salt=21)
    trail([(72, 25), (59, 25)], grass=0.2, salt=22)
    trail([(57, 35), (57, 29)], grass=0.2, salt=23)


# ---------------------------------------------------------------- the jungle


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


CANOPY = (("tree", 66), ("palm", 24), ("willow", 6), ("snag", 4))
BANK = (("willow", 40), ("palm", 40), ("tree", 20))
UNDER = (("fern", 34), ("bush", 26), ("berry_bush", 22), ("autumn_bush", 18))


def tree_for(c, r, wd):
    if wd.get((c, r), 9) <= 2:
        return pick(BANK, hash01(c, r, 73))
    return pick(CANOPY, hash01(c, r, 79))


JUNGLE_GROUND = []


def canopy_noise(c, r):
    """How thick the forest stands at (c, r): high is canopy, low a glade."""
    return fbm(c / 6, r / 6, 11)


def jungle():
    wd = distance_field(water, 4)
    # the bands are quantiles of the noise over the ground the jungle may
    # take, so the share of canopy, thinner forest and glade is set here
    # whatever the noise's spread
    free = sorted(canopy_noise(c, r) for r in range(H) for c in range(W)
                  if (c, r) not in cells and (c, r) not in reserved)
    JUNGLE_GROUND.append(len(free))
    q = lambda share: free[int(len(free) * share)]
    glade, thin, mid = q(0.08), q(0.18), q(0.3)
    for r in range(H):
        for c in range(W):
            p = (c, r)
            if p in cells or p in reserved:
                continue
            dw = wd.get(p, 9)
            u = hash01(c, r, 13)
            if dw == 1 and hash01(c, r, 85) < 0.5:
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


def hollows(loot):
    """Caches in the thickest of the jungle: a little clearing with
    something in it, reached by a trail whose mouth is hidden in bushes."""
    made = []
    tries = 0
    while len(made) < len(loot) and tries < 20000:
        tries += 1
        c, r = rng.randint(3, W - 4), rng.randint(3, H - 4)
        if any(math.hypot(c - mc, r - mr) < 14 for mc, mr in made):
            continue
        if any(math.hypot(c - pc, r - pr) < 6 for pc, pr, _ in pickups):
            continue
        block = list(rect(c - 2, r - 2, c + 2, r + 2))
        if sum(kind_at(p) in TREE_KINDS for p in block) < 19:
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
                if (kind_at(p) is None or kind_at(p) in COVER_KINDS) and not wet(p) and p not in roads:
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
    """What the router charges to step into nav cell (k, l): a water ford
    `water_ford_path_cost` (3), dry ground 1."""
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
    moat()
    streams()

    # The ways: every one runs in straight reaches along the rows and
    # columns - a hull drives four ways, and a diagonal corridor makes it
    # stair-step all the way along.
    # The four causeways, each on into its gate.
    road([(11, 20), (22, 20)])       # west, the long causeway over the baray
    road([(60, 19), (72, 19)])       # east
    road([(41, 6), (41, 13)])        # north, from the north shrine's door
    road([(50, 29), (50, 35)])       # south, to the south shrine's door
    # the ring of trails round the moat, straight along the north past the
    # north causeway (a jog there left a tank between two equal ways in,
    # swapping between them at the edge of a seat's sight)
    trail([(11, 7), (16, 7), (16, 4), (24, 4), (24, 7), (72, 7)],
          grass=0.25, salt=1)
    trail([(11, 7), (11, 35)], grass=0.3, salt=2)
    trail([(72, 7), (72, 35)], grass=0.25, salt=3)
    trail([(11, 35), (72, 35)], grass=0.3, salt=4)
    # the spokes out to the gates
    trail([(5, 0), (5, 3), (11, 3), (11, 7)], grass=0.2, salt=5)
    trail([(77, 0), (77, 7), (72, 7)], grass=0.2, salt=6)
    trail([(0, 32), (11, 32)], grass=0.3, salt=7)
    trail([(82, 12), (72, 12)], grass=0.2, salt=8)
    trail([(6, 41), (6, 38), (11, 38), (11, 35)], grass=0.3, salt=9)
    trail([(80, 41), (80, 38), (72, 38), (72, 35)], grass=0.2, salt=10)

    temple()
    north_shrine()
    south_shrine()
    west_ruin()
    east_ruin()
    traps()
    ford_ways()
    gate_aprons()
    for gc, gr in GATES:
        put(gc, gr, "gate")
        GATE_LANES.update(lane(gc, gr))

    verges()
    jungle()
    made = hollows(["health", "ammo", "health"])
    wire_up(made)
    filled = fill_pockets()
    close_narrow_trails()
    return filled, made


HEADER = """\
# jungle-temple - Jungle Temple, level 6: a stepped temple on a moated island.
# Written by tools/gen_jungle_temple.py from a fixed seed - change the
# generator and run it again rather than editing this file; re-saving from
# the map editor would drop these comments. Level tables as dotted keys on
# purpose - a `[spawn]` header would swallow every `cells.` line after it.
#
# Protect on the waves plan: five waves of 4 to 8 tanks, light to heavy, 30 in
# all, rolling in at six gates on all four edges - two on the north edge, one
# west, one east, two on the south edge - each a walk of 11 to 13 s from the
# sanctum, so every wave comes from several sides at once. 83 x 42 cells,
# followed by the camera, in the valley's look (dressed limestone, gravel on
# the banks), under a sky the seed picks from clear, dusk, rain and storm.
#
# The temple stands on an island in the middle, ringed by a moat three cells
# wide whose middle is open water - a wall to hulls, nothing to shots - and
# widened off its west side into a reservoir. Four causeways cross it: the
# long one over the reservoir from the west, one from the east, and one each
# from the north and the south shrine's door; three stretches of the moat have
# silted up to a ford (north-west, east and south-east), where the enclosure
# wall has fallen in. Inside the broken enclosure, the west and east
# courtyards with their wooden galleries (the sonic hammer and missiles; the
# laser and plasma) and reflecting pools, the narrow north and south ones
# (grenades, a bong health pack, a speed-up; below the south stair the FPV
# swarm and the two miniguns that are its counter), and a gun tower over each
# of the two long approaches.
#
# The players' base is the stepped terrace: a limestone wall with grey stone
# steps at its corners and a stair through three sides, none in line with the
# gate below it, and no stair on the west, so whoever comes up a causeway
# crosses a courtyard under the towers to reach one; no road has a straight
# line at the bong. On top, the bong in a low sandbag ring open only north and
# south, the players either side of it, two gun towers, a tesla over the east
# stair and a bio slush lobbing over the west wall, braziers at two corners
# (lamps - they light the sanctum at dusk and in a storm), and the stores: a
# bong health pack, health, ammunition, a shield, a jet and a helicopter
# strike, missiles and grenades, the blast weapons eight cells and more from
# the bong.
#
# Outside the moat, a ring of jungle trails joins the causeways to the gates.
# The north shrine, held by an enemy gun tower with drums beside it, keeps a
# laser, missiles, a minigun and a health pack; the south shrine, held by an
# enemy bio slush, the EMP, missiles, a shield and a health pack. In the west
# jungle a broken ruin under an enemy gun tower holds a flamethrower, grenades
# and health; in the east a broken shed behind an enemy tesla, grenades, a
# flamethrower and ammunition. Drum clusters wait beside the west and east
# causeways and the two shrines. Everything else is jungle - broadleaf
# canopy, palms, willows on the banks, ferns and flowering bushes underneath,
# elephant grass in the glades, three streams - with three caches at the ends
# of hidden trails whose mouths are bushes.
"""


def write(path):
    lines = [HEADER, "version = 1",
             'bong = "gloop"',
             'tank = "obelisk"',
             'tank2 = "warden"',
             'biome = "valley"',
             'weather = ["clear", "dusk", "rain", "storm"]',
             "size = [83, 42]",
             'mission.kind = "protect"',
             'spawn.kind = "waves"',
             "spawn.waves = 5",
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
    problems = []
    for gc, gr in GATES:
        assert gc in (0, W - 1) or gr in (0, H - 1), (gc, gr)
    for c, r, kind in pickups:
        if kind in FETCHED:
            near = min(math.hypot(c - gc, r - gr) for gc, gr in GATES)
            if near < 20.5:
                problems.append(f"{kind} at {c},{r} is {near:.1f} cells from a gate")
        if kind in BLAST_CRATES:
            d = math.hypot(c - BONG[0], r - BONG[1])
            if d < BLAST_CRATES[kind]:
                problems.append(f"{kind} at {c},{r} is {d:.1f} cells from the bong")
    for p, t in cells.items():
        if '"barrel"' in t and math.hypot(p[0] - BONG[0], p[1] - BONG[1]) < 8:
            problems.append(f"drum at {p} beside the bong")
    for p in water:
        if math.hypot(p[0] - BONG[0], p[1] - BONG[1]) < 8:
            problems.append(f"water at {p} within the bong's hop")
    reach = nav_reach()
    near = lambda p, rr: any((p[0] + dc, p[1] + dr) in rr for dc in range(-4, 4) for dr in range(-4, 4))
    for p in (START, START2, BONG):
        if not near(p, reach):
            problems.append(f"{p} is off the playfield")
    for c, r, kind in pickups:
        if not reachable((c, r), reach):
            problems.append(f"{kind} at {c},{r} cannot be reached")
    for gc, gr in GATES:
        if not all(nav_open(*n) for n in lane(gc, gr)):
            problems.append(f"gate {gc},{gr} lane is shut")
        elif lane(gc, gr)[-1] not in reach:
            problems.append(f"gate {gc},{gr} is cut off")
    for p in problems:
        print("CHECK:", p)
    assert not problems, f"{len(problems)} problems"


ASCII = {
    'wall': '#', 'road': '=', 'water': '~', 'bong': 'B', 'start': '1', 'start2': '2',
    'pickup': 'P', 'sandbag': 's', 'fence': 'f', 'tree': 'T', 'palm': 'Y', 'snag': 'x', 'willow': 'W',
    'birch': 'b', 'tall_grass': '"', 'bush': 'o', 'berry_bush': 'o', 'autumn_bush': 'o', 'fern': ',',
    'reeds': '|', 'gate': 'G', 'tesla': 'Z', 'gun_tower': 'K', 'bio_slush': 'Q', 'barrel': 'D', 'hay': 'h',
    'tyres': 'O', 'wood_crate': 'X', 'barrier': 'n', 'tank_trap': '*', 'lamp': 'l', 'oil': ':',
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
    ap.add_argument("-o", "--out", default="maps/jungle-temple.toml")
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
            print(f"  gate {gc},{gr}: walk {c} steps = {c / 5:.1f} s" if c else f"  gate {gc},{gr}: no walk")
        for c, r, kind in pickups:
            print(f"  pickup {kind:13} {c},{r}")


if __name__ == "__main__":
    main()
