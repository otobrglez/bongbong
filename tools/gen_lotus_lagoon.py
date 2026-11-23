"""Write maps/lotus-lagoon.toml: Lotus Lagoon, level 1 - a floodplain lagoon
in bloom with the bong's pavilion on an island in it, 68 x 34 cells, laid
out set piece by set piece.

  python3 tools/gen_lotus_lagoon.py [-o maps/lotus-lagoon.toml] [--stats] [--ascii]

This script is the source: change it and run it again rather than editing
the TOML (a re-save from the map editor would drop the header too). It draws
no random numbers: every choice is hashed from the cell's position with a
fixed salt, so a rerun writes the same file byte for byte. `--ascii` prints
the map a character a cell, `--stats` the counts, the pickups and each
one's walk from the start.

What it lays out (the header written into the map says how it plays):

- the lagoon across the north middle, open to the north edge, deep in its
  heart with palm sandbars, and its southern bay: a ring of shallow water
  two cells wide round the pavilion island, crossed by three plank bridges
  and forded anywhere;
- the pavilion on the island - the bong in a ring of sandbags with brick
  pillars, a railing along the north shore, two gun towers on the island's
  tips and two teslas at the mouth of the south bridge, the stores;
- two channels out of the lagoon, west and east, running down to the south
  edge, and the levee road along the south crossing them on plank decks;
- the lotus gardens on the south bank where the players start: a boathouse
  and a gazebo with the first weapons, open lanes either side, and across
  the levee road the lower garden's ponds and flower beds;
- the stilt village on the west channel (huts, a fenced paddy, hay, drums),
  the reed marsh and a ruined shrine in the north-west held by an enemy bio
  slush, the poachers' camp in the north-east held by an enemy gun tower,
  the tea house and its palm grove in the east;
- meadow everywhere else - willows on the banks, palms, birches, broadleaf
  trees, flower beds, reeds and tall grass - with two hidden caches.
"""

import argparse
import heapq
import math
from collections import deque

W, H = 68, 34

# (col, row) -> inline TOML table text
cells = {}
# cells the meadow pass must leave alone
reserved = set()
water = set()
roads = set()

BONG = (34, 15)
START = (33, 26)
START2 = (38, 26)
# the pavilion island and the shallow ring round it
ISLAND = (27, 10, 41, 19)
RING = 2
# the levee road's two rows
LEVEE = (28, 29)

# a weapon met first in the campaign here: its crate lies in the start's
# sight box (docs/levels.md, "The weapons along the way")
FIRST_HERE = {"missiles", "plasma", "sonic_hammer"}
# a seat's sight box, half extents in cells (`sight_box_half_cols`/`_rows`)
SIGHT = (11.5, 7.5)
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


def rect(c0, r0, c1, r1):
    for r in range(r0, r1 + 1):
        for c in range(c0, c1 + 1):
            if inb(c, r):
                yield c, r


def outline(c0, r0, c1, r1):
    for c, r in rect(c0, r0, c1, r1):
        if c in (c0, c1) or r in (r0, r1):
            yield c, r


# the set pieces' open ground, which the meadow's trees keep off
OPEN_GROUND = set()


def clear_rect(c0, r0, c1, r1):
    for c, r in list(rect(c0, r0, c1, r1)):
        if (c, r) not in roads and (c, r) not in water:
            clear(c, r)
        reserved.add((c, r))
        OPEN_GROUND.add((c, r))


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


def meander(start, end_row, lo, hi, salt, wander=0.35):
    """A one-cell channel from `start` down the rows to `end_row`, wandering
    between columns `lo` and `hi`. It never turns straight back within two
    rows, so it never makes a 2 x 2 block and reads as a stream, never a
    pool - and every cell of it is a ford."""
    c, r = start
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
            run = 1 + int(hash01(c, r, salt + 2) * 2)
            for _ in range(run):
                nc = c + want
                if lo <= nc <= hi:
                    c = nc
                    path.append((c, r))
            side, since = want, 0
        r += 1
        since += 1
        path.append((c, r))
    for p in path:
        if inb(*p):
            put(*p, "water")
    return path


def road(points, width=2, margin=0):
    """A dirt road `width` cells across along the rows and columns."""
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


def bridge_decks(was_water):
    """A road two cells wide over a channel one cell wide leaves no road cell
    with water on both sides, which is what the floodplain's look draws as a
    plank deck (`ground::GroundGrid::bridge`): give the channel back the
    second row (or column), so the first is a deck over it. The cell given
    back is a ford again, which a hull wades."""
    for c, r in sorted(was_water):
        if (c, r) not in roads:
            continue
        if ((c, r - 1) in water and (c, r + 1) in water) or ((c - 1, r) in water and (c + 1, r) in water):
            continue
        for n in ((c, r + 1), (c + 1, r)):
            if n in roads and n in was_water:
                put(*n, "water")
                break


# ---------------------------------------------------------------- the water


def in_island(c, r):
    """The pavilion island: a rounded rectangle inside `ISLAND`, its corners
    cut back a cell or two."""
    c0, r0, c1, r1 = ISLAND
    cx, cy = (c0 + c1) / 2, (r0 + r1) / 2
    ax, ay = (c1 - c0) / 2 + 0.8, (r1 - r0) / 2 + 0.8
    return abs((c - cx) / ax) ** 2.6 + abs((r - cy) / ay) ** 2.6 <= 1.0


def island_cells():
    c0, r0, c1, r1 = ISLAND
    return [(c, r) for c, r in rect(c0, r0, c1, r1) if in_island(c, r)]


def ring_cells():
    """The bay round the island: every cell within `RING` of it, the
    Chebyshev way - two cells wide all round, so every cell of it has dry
    ground beside it and is a ford."""
    isle = set(island_cells())
    c0, r0, c1, r1 = ISLAND
    return [(c, r) for c, r in rect(c0 - RING, r0 - RING, c1 + RING, r1 + RING)
            if (c, r) not in isle
            and any((c + a, r + b) in isle for a in range(-RING, RING + 1) for b in range(-RING, RING + 1))]


def lagoon():
    """The lagoon: an ellipse open to the north edge with a ragged shore,
    deep in its heart, and the bay round the island - a ring of water two
    cells wide, every cell of it a ford."""
    cx, cy, rx, ry = 34, 1.0, 17.5, 9.6
    c0, r0, c1, r1 = ISLAND
    for r in range(0, r0):
        for c in range(cx - 20, cx + 21):
            if not inb(c, r):
                continue
            dx, dy = (c - cx) / rx, (r - cy) / ry
            wobble = 0.5 * (fbm(c / 3.2, r / 3.2, 7) - 0.5)
            if dx * dx + dy * dy <= 1.0 + wobble:
                put(c, r, "water")
    ring = set(ring_cells())
    for c, r in sorted(ring):
        put(c, r, "water")
    # the bay's banks stay dry, so the ring stays two cells wide and
    # shallow all round: no lagoon water south of the island's top row but
    # the ring's
    for c, r in rect(c0 - RING - 3, r0, c1 + RING + 3, r1 + RING + 2):
        if (c, r) in water and (c, r) not in ring:
            clear(c, r)
            reserved.discard((c, r))
    for c, r in island_cells():
        clear(c, r)


def islets():
    """Palms on sandbars out in the lagoon's deep water: each islet is one
    solid cell, so the shoal round it is a ford no hull can reach."""
    for c, r in ((24, 3), (30, 1), (41, 2), (46, 5), (37, 6)):
        assert is_deep(c, r), f"an islet at {c},{r} must stand in open water"
    for c, r, kind in ((24, 3, "palm"), (30, 1, "willow"), (41, 2, "palm"), (46, 5, "palm"), (37, 6, "willow")):
        put(c, r, kind)


def channels():
    """The two channels out of the lagoon: west from its west tip, then
    down to the south edge past the stilt village; east from its east tip,
    then down past the palm orchard. Each is one cell wide - a ford its
    whole length, with a current to the south."""
    west_tip = min((c for c, r in water if r == 6), default=18)
    for c in range(15, west_tip + 1):
        put(c, 6, "water")
    meander((15, 7), H - 1, 13, 17, 41)
    east_tip = max((c for c, r in water if r == 6), default=50)
    for c in range(east_tip, 55):
        put(c, 6, "water")
    meander((54, 7), H - 1, 52, 56, 53)


def pond(c0, r0, c1, r1):
    """A lotus pond: two rows (or columns) at most, so all of it is a ford."""
    assert min(c1 - c0, r1 - r0) <= 1, "a pond two cells across or less stays shallow"
    for c, r in rect(c0, r0, c1, r1):
        put(c, r, "water")


# ---------------------------------------------------------------- set pieces


def on_island(c, r):
    assert in_island(c, r), f"{c},{r} is off the island"
    return c, r


def pavilion():
    """The bong's pavilion on the island: the bong in a ring of sandbags
    with brick pillars at its corners, open at the middle of every side; a
    railing along the north shore against shots across the lagoon, open in
    the middle where steps go down to the water; gun towers on the island's
    west and east ends over the side bridges; teslas either side of the
    south bridge's landing; sandbags beside the side landings; the stores."""
    c0, r0, c1, r1 = ISLAND
    bc, br = BONG
    put(bc, br, "bong")
    for c, r in outline(bc - 2, br - 2, bc + 2, br + 2):
        if c == bc or r == br:
            continue
        if abs(c - bc) == 2 and abs(r - br) == 2:
            wall(c, r, "brick")
        else:
            put(c, r, "sandbag")
    for c in range(bc - 5, bc + 6):
        if abs(c - bc) > 1:
            wall(*on_island(c, r0), "wood")
    put(*on_island(c0 + 1, br - 2), "gun_tower")
    put(*on_island(c1 - 1, br - 2), "gun_tower")
    put(*on_island(bc - 3, r1), "tesla")
    put(*on_island(bc + 3, r1), "tesla")
    for c in (c0, c1):
        put(*on_island(c, br - 1), "sandbag")
        put(*on_island(c, br + 2), "sandbag")
    # the stores, along the sides
    pickup(*on_island(c0 + 2, br), "health")
    pickup(*on_island(c1 - 2, br), "health")
    pickup(*on_island(c1 - 4, r0 + 1), "bong_health")
    pickup(*on_island(c0 + 4, r0 + 1), "tower_pack")
    pickup(*on_island(c0 + 3, r1 - 1), "minigun")
    pickup(*on_island(c1 - 3, r1 - 1), "ammo")
    # palms on the rounded corners, flowers and reeds
    for c, r in ((c0 + 1, r0 + 1), (c1 - 1, r0 + 1), (c0 + 1, r1 - 1), (c1 - 1, r1 - 1)):
        put(*on_island(c, r), "palm")
    for c, r in ((bc - 4, br - 2), (bc + 4, br + 2), (bc - 4, br + 2), (bc + 4, br - 2)):
        put_free(c, r, "autumn_bush")
    for c, r in ((bc - 1, r0), (bc + 1, r0), (c0 + 2, r1), (c1 - 2, r1)):
        put_free(*on_island(c, r), "reeds")


def island_bridges():
    """Three plank bridges over the ring, each one cell wide across its two:
    west and east into the island's ends below the gun towers, south in the
    line of the garden road."""
    bc, br = BONG
    ring = set(ring_cells())
    row, col = br + 1, bc
    west = [c for c in range(bc, -1, -1) if (c, row) in ring]
    east = [c for c in range(bc, W) if (c, row) in ring]
    south = [r for r in range(br, H) if (col, r) in ring]
    for c in west + east:
        put(c, row, "road")
    for r in south:
        put(col, r, "road")


def lotus_gardens():
    """The south bank where the players start, between the ring and the
    levee road: the garden road down from the south bridge, the boathouse
    on the water (missiles, health), the glass gazebo (plasma), palms along
    the road and flower beds."""
    c0, r0, c1, r1 = ISLAND
    bc = BONG[0]
    top = r1 + RING + 1
    clear_rect(c0 - 3, top, c1 + 3, LEVEE[0] - 1)
    road([(bc, top), (bc, LEVEE[0])], width=2)
    put(*START, "start")
    put(*START2, "start2")
    # the boathouse on the bank west of the bridge, its door to the south
    hc0, hr0, hc1, hr1 = c0 - 2, top, c0 + 3, top + 3
    for c, r in outline(hc0, hr0, hc1, hr1):
        if r == hr1 and c in (hc0 + 2, hc0 + 3):
            continue
        wall(c, r, "wood")
    pickup(hc0 + 1, hr0 + 1, "missiles")
    pickup(hc1 - 1, hr0 + 1, "health")
    # the gazebo east of the bridge: glass on brick pillars, open south
    gc0, gr0, gc1, gr1 = c1 - 3, top, c1 + 1, top + 3
    for c, r in outline(gc0, gr0, gc1, gr1):
        if r == gr1 and gc0 < c < gc1:
            continue
        if c in (gc0, gc1) and r in (gr0, gr1):
            wall(c, r, "brick")
        else:
            wall(c, r, "glass")
    pickup(gc0 + 1, gr0 + 1, "plasma")
    pickup(gc1 - 1, gr0 + 1, "sonic_hammer")
    # palms along the road, flower beds before the levee
    for r in (top, top + 2):
        put_free(bc - 1, r, "palm")
        put_free(bc + 2, r, "palm")
    for c in list(range(c0 - 3, bc - 2)) + list(range(bc + 4, c1 + 4)):
        r = LEVEE[0] - 1
        if hash01(c, r, 95) < 0.7:
            put_free(c, r, pick(FLOWERS, hash01(c, r, 96)))


def lotus_ponds():
    """The lotus ponds south of the levee road, two cells deep at most so
    every cell is a ford, ringed by reeds and flowers: two in the lower
    garden, one in each wood beyond the channels. The upper garden between
    the levee and the ring is kept open - the lanes the enemies come along
    to the start."""
    lo = LEVEE[1] + 1
    for pc0, pr0, pc1, pr1 in ((19, lo + 1, 20, lo + 2), (48, lo + 1, 49, lo + 2),
                               (27, lo + 1, 29, lo + 2), (40, lo + 1, 42, lo + 2)):
        pond(pc0, pr0, pc1, pr1)
        for c, r in outline(pc0 - 1, pr0 - 1, pc1 + 1, pr1 + 1):
            if (c, r) in roads:
                continue
            u = hash01(c, r, 91)
            if u < 0.4:
                put_free(c, r, "reeds")
            elif u < 0.7:
                put_free(c, r, pick(FLOWERS, hash01(c, r, 92)))


def south_garden():
    """South of the levee road: the lower garden - flower beds in rows
    either side of the road, plasma on the bench by the west pond, ammo and
    a second bong health pack in the arbour by the east one."""
    bc = BONG[0]
    lo = LEVEE[1] + 1
    clear_rect(24, lo, 45, H - 1)
    road([(bc, lo), (bc, H - 1)], width=2)
    pickup(bc - 3, lo + 1, "plasma")
    pickup(bc + 3, lo + 2, "bong_health")
    pickup(bc + 3, lo + 1, "ammo")
    for r in (H - 2,):
        for c in list(range(25, 32)) + list(range(38, 45)):
            if hash01(c, r, 93) < 0.75:
                put_free(c, r, pick(FLOWERS, hash01(c, r, 94)))
    # the arbour: a hedge of berry bushes round the packs, open to the road
    for c, r in ((bc + 3, lo), (bc + 4, lo), (bc + 4, lo + 1), (bc + 4, lo + 2), (bc + 4, lo + 3)):
        put_free(c, r, "berry_bush")
    for c, r in ((26, H - 1), (31, H - 1), (38, H - 1), (43, H - 1), (24, lo + 1), (45, lo + 1)):
        put_free(c, r, "palm")


def stilt_village():
    """The stilt village on the west channel: three huts of wood (a health
    pack, ammo and the second sonic hammer in them), a fenced paddy by the
    track, hay bales and a cluster of oil drums by the huts."""
    clear_rect(1, 9, 12, 26)
    huts = [(2, 15, 6, 18, ((6, 16), (6, 17)), "health"), (9, 18, 12, 21, ((9, 19), (9, 20)), "sonic_hammer"),
            (1, 21, 5, 24, ((3, 21), (4, 21)), "ammo")]
    for c0, r0, c1, r1, door, loot in huts:
        for c, r in outline(c0, r0, c1, r1):
            if (c, r) in door:
                continue
            wall(c, r, "wood")
        pickup(c1 - 1 if door[0][0] != c1 else c0 + 1, r0 + 1, loot)
    # the paddy: a field of tall grass behind fences, its gate on the road
    pc0, pr0, pc1, pr1 = 1, 10, 6, 13
    for c, r in outline(pc0, pr0, pc1, pr1):
        if c == pc1 and r in (11, 12):
            continue
        put(c, r, "fence")
    for c, r in rect(pc0 + 1, pr0 + 1, pc1 - 1, pr1 - 1):
        put(c, r, "tall_grass")
    for c, r in ((9, 10), (10, 10), (10, 23), (11, 24)):
        put_free(c, r, "hay")
    drum_cluster(((10, 12), (11, 12), (11, 13)), "oil")
    for c, r in ((1, 26), (12, 10), (1, 14), (6, 25)):
        put_free(c, r, "palm")


def reed_marsh_and_shrine():
    """The north-west: a reed marsh of small pools, and a ruined shrine of
    old concrete held by an enemy bio slush - plasma, a shield and a health
    pack inside, two oil drums by its broken east wall."""
    clear_rect(1, 1, 10, 8)
    s0, sr0, s1, sr1 = 2, 1, 9, 7
    gaps = {(5, sr1), (6, sr1), (s1, 3), (s1, 4), (s0, 5)}
    for c, r in outline(s0, sr0, s1, sr1):
        if (c, r) in gaps:
            continue
        if hash01(c, r, 61) < 0.1:
            continue  # fallen in
        wall(c, r, "concrete")
    put(5, 4, "bio_slush", side="enemy")
    pickup(3, 2, "plasma")
    pickup(8, 2, "health")
    pickup(3, 6, "shield")
    drum_cluster(((11, 6), (11, 7)), "oil")
    for pc0, pr0, pc1, pr1 in ((12, 1, 13, 2), (16, 9, 17, 10)):
        pond(pc0, pr0, pc1, pr1)
    for c, r in rect(10, 0, 22, 12):
        if (c, r) in cells:
            continue
        near_wet = any((c + a, r + b) in water for a in (-1, 0, 1) for b in (-1, 0, 1))
        if near_wet and hash01(c, r, 63) < 0.5:
            put(c, r, "reeds")


def poachers_camp():
    """The north-east: the poachers' camp, an enemy gun tower in a sandbag
    nest before their stash - a pen of tyre stacks open to the west with
    minigun, missiles and a health pack in it - fuel drums stacked in the
    open below the pen, speed by the track."""
    clear_rect(57, 1, 67, 10)
    put(59, 4, "gun_tower", side="enemy")
    for c, r in ((58, 3), (60, 3), (58, 5), (60, 5)):
        put(c, r, "sandbag")
    for c in range(63, 68):
        put(c, 1, "tyres")
        put(c, 6, "tyres")
    for r in range(2, 6):
        put(67, r, "tyres")
    pickup(65, 3, "minigun")
    pickup(65, 4, "missiles")
    pickup(64, 4, "health")
    pickup(61, 9, "speedup")
    drum_cluster(((65, 9), (66, 9), (66, 10)), "fuel")


def tea_house():
    """The east: the tea house of brick and glass with its door to the
    track (missiles, a health pack and ammo inside), palms round it; its
    yard's drums are `bridge_traps`'."""
    clear_rect(59, 13, 66, 24)
    h0, hr0, h1, hr1 = 60, 14, 65, 19
    for c, r in outline(h0, hr0, h1, hr1):
        if c == h0 and r in (16, 17):
            continue  # the door, to the track
        if r == hr0 and c in (62, 63):
            wall(c, r, "glass")
        elif c == h1 and r in (16, 17):
            wall(c, r, "glass")
        else:
            wall(c, r, "brick")
    pickup(62, 16, "missiles")
    pickup(63, 18, "health")
    pickup(63, 15, "ammo")
    for c, r in ((66, 13), (66, 24), (63, 24)):
        put_free(c, r, "palm")


def drum_cluster(at, drum):
    """A cluster of drums standing in the open: the ring of cells round it
    is cleared, so a hull that shoves one rolls it away rather than pinning
    it against a tree, a wall or the edge."""
    for c, r in at:
        for p in rect(c - 1, r - 1, c + 1, r + 1):
            if p not in water and p not in roads and kind_at(p) in (None,) + tuple(TREE_KINDS | COVER_KINDS):
                clear(*p)
    for c, r in at:
        put(c, r, "barrel", drum=drum)


def bridge_traps():
    """Oil drums by the west track, short of the west bridge and well away
    from the bong - one shot as a tank comes by and the cluster goes up -
    and a cluster in the tea house's yard. Each stands off the ways the
    enemies route, where no hull pins one against a wall or the water."""
    br = BONG[1]
    drum_cluster(((20, br - 2), (21, br - 2), (20, br - 3)), "oil")
    drum_cluster(((64, 21), (65, 21), (65, 22)), "oil")


def caches():
    """Two hidden caches: a shield and a minigun in the birch wood in the
    south-west, speed and missiles in the palms in the south-east, each a
    little clearing with a health pack beside them."""
    for c, r, what, weapon in ((7, 31, "shield", "minigun"), (61, 31, "speedup", "missiles")):
        for p in rect(c - 1, r - 1, c + 1, r + 1):
            clear(*p)
        pickup(c, r, what)
        pickup(c + 1, r + 1, "health")
        pickup(c + 1, r - 1, weapon)


# ---------------------------------------------------------------- the meadow


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


FLOWERS = (("berry_bush", 45), ("autumn_bush", 35), ("bush", 20))
BLOOM = (("autumn_bush", 45), ("berry_bush", 40), ("juniper", 15))
BANK = (("willow", 70), ("palm", 15), ("birch", 15))
UNDER = (("bush", 30), ("berry_bush", 30), ("autumn_bush", 20), ("fern", 20))


def zone_trees(c, r):
    """The trees of each part of the plain: birches and willows in the
    south-west wood, palms in the east, broadleaf and willow elsewhere."""
    if c < 22 and r > 24:
        return (("birch", 45), ("willow", 30), ("tree", 25))
    if c > 44:
        return (("palm", 50), ("tree", 30), ("birch", 20))
    if r < 12:
        return (("willow", 50), ("tree", 35), ("palm", 15))
    return (("tree", 50), ("willow", 25), ("palm", 25))


def meadow():
    """Everything not a set piece: open meadow with groves, willows on the
    banks, flower beds and tall grass. Woods thicken toward the corners and
    the sides; the ground before the bays stays open to fight over."""
    wd = distance_field(water, 3)
    for r in range(H):
        for c in range(W):
            p = (c, r)
            if p in cells or p in reserved:
                continue
            dw = wd.get(p, 9)
            u = hash01(c, r, 13)
            if dw == 1 and hash01(c, r, 85) < 0.4:
                put(c, r, "reeds")
                reserved.discard(p)
                continue
            if dw == 1 and u < 0.12:
                put(c, r, pick(BANK, hash01(c, r, 73)))
                reserved.discard(p)
                continue
            d = fbm(c / 6, r / 6, 11)
            # the plain round the island is kept open, and the garden lanes
            # between the ring and the levee road grow no trees at all
            core = math.hypot((c - BONG[0]) / 1.6, r - BONG[1])
            if core < 13:
                d -= 0.25
            if 18 <= c <= 51 and ISLAND[3] + RING < r < LEVEE[0]:
                d = -1.0
            if d > 0.62:
                density = 0.7
            elif d > 0.52:
                density = 0.35
            else:
                density = 0.04 if d > -0.5 else 0.0
            if u < density:
                put(c, r, pick(zone_trees(c, r), hash01(c, r, 79)))
                reserved.discard(p)
                continue
            g = fbm(c / 4, r / 4, 17)
            v = hash01(c, r, 19)
            bloom = fbm(c / 2.5, r / 2.5, 31)
            if bloom > 0.7 and v < 0.85:
                # a bed of flowers in bloom
                put(c, r, pick(BLOOM, hash01(c, r, 37)))
            elif g > 0.58 and v < 0.8:
                put(c, r, "tall_grass")
            elif g < 0.3 and v < 0.35:
                put(c, r, pick(FLOWERS, hash01(c, r, 23)))
            elif d > 0.45 and v < 0.25:
                put(c, r, pick(UNDER, hash01(c, r, 29)))
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

# a loose prop is no wall in the nav grid (`nav_tile_shapes_where`): a hull
# shoves it out of its way
LOOSE = {"barrel", "hay", "wood_crate", "barrier"}
SOFT = {"road", "water", "pickup", "oil", "portal", "start", "start2", "gate"} | COVER_KINDS | LOOSE


def is_deep(c, r):
    """Open water: a water cell whose eight neighbours are all water, the
    map's edge reading as the cell beside it (`ground::Layout` clamps), so
    the lagoon is deep right up to the north edge."""
    clamp = lambda c2, r2: (min(max(c2, 0), W - 1), min(max(r2, 0), H - 1))
    return all(clamp(c + dc, r + dr) in water for dc in (-1, 0, 1) for dr in (-1, 0, 1))


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
    """An open nav cell. The field's own edge is a wall with the same margin
    as a tile's, so the outermost ring counts as shut."""
    if not (1 <= k < W - 1 and 1 <= l < H - 1):
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


def connect(c, r, reach):
    """Cut a way three cells wide from map cell (c, r) out to the playfield
    through trees and cover."""
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
    p = goal
    while p is not None:
        pc, pr = p
        for dc in (-1, 0, 1):
            for dr in (-1, 0, 1):
                n = (pc + dc, pr + dr)
                if inb(*n) and carvable(n) and kind_at(n) in TREE_KINDS:
                    clear(*n)
        p = prev[p]
    return True


def wire_up():
    for _ in range(4):
        reach = nav_reach()
        lost = [(c, r) for c, r, _ in pickups if not reachable((c, r), reach)]
        if not lost:
            return
        for c, r in lost:
            connect(c, r, reach)


def fill_pockets():
    """Open ground the playfield never reaches grows a tree (or a reed bed
    by the water), so no nav cell is left boxed in or cut off."""
    wd = distance_field(water, 3)
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
                if (kind_at(p) is None or kind_at(p) in COVER_KINDS) and p not in water and p not in roads \
                        and p not in OPEN_GROUND:
                    put(*p, pick(BANK, hash01(*p, 73)) if wd.get(p, 9) <= 1 else pick(zone_trees(*p), hash01(*p, 79)))
                    filled += 1
                    break
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


# ---------------------------------------------------------------- the walk


def walk_costs():
    """What the router charges from the start to every nav cell: a ford
    `water_ford_path_cost` (3), dry ground 1."""
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
    lagoon()
    channels()
    islets()
    # The roads run in straight reaches along the rows and columns: a hull
    # drives four ways, and a diagonal corridor makes it stair-step.
    was_water = set(water)
    road([(0, LEVEE[0]), (W - 2, LEVEE[0])])
    road([(7, LEVEE[0]), (7, 9)])
    road([(57, LEVEE[0]), (57, 7), (60, 7)])
    # the west and east tracks to the side bridges
    track = BONG[1] + 1
    road([(9, track), (24, track)], width=1)
    road([(44, track), (56, track)], width=1)
    bridge_decks(was_water)
    island_bridges()
    pavilion()
    lotus_gardens()
    south_garden()
    lotus_ponds()
    stilt_village()
    reed_marsh_and_shrine()
    poachers_camp()
    tea_house()
    bridge_traps()
    caches()
    meadow()
    wire_up()
    filled = fill_pockets()
    close_narrow_trails()
    return filled


HEADER = """\
# lotus-lagoon - Lotus Lagoon, level 1: a floodplain lagoon in bloom, the
# bong's pavilion on an island in it. Written by tools/gen_lotus_lagoon.py -
# change the generator and run it again rather than editing this file;
# re-saving from the map editor would drop these comments.
# Level tables as dotted keys on purpose - a `[spawn]` header would swallow
# every `cells.` line after it.
#
# Protect on the band plan, seven tanks, under a sky picked afresh each round
# (docs/weather.md; under snow the lagoon freezes and the island can be
# reached across the ice) - the breather after Boot Camp. 68 x 34 cells,
# followed by the camera, drawn as a reclaimed floodplain (`biome`,
# docs/biomes.md): slate still water, sedge and peat on the banks, a
# floodwork apron round the towers by the water.
#
# The lagoon fills the north middle, open to the north edge, deep in its
# heart, palms and willows on sandbars out in it. Its southern bay is a ring
# of shallow water two cells wide round the pavilion island - forded
# anywhere, or crossed dry on three plank bridges: west and east into the
# island's ends, south into the gardens. On the island the bong sits in a
# ring of sandbags with brick pillars at its corners, open at the middle of
# every side; a wooden railing along the north shore stops shots from across
# the lagoon; two gun towers on the ends watch the side bridges and two
# teslas the south one. The island's stores: two health packs, ammo, a
# minigun, a bong health pack and a tower pack.
#
# The players start in the lotus gardens on the south bank, between the
# boathouse on the water (missiles, health) and the glass gazebo (plasma and
# the sonic hammer - shove a tank off a bridge or into the drums); the lanes
# either side are kept open. Across the levee road the lower garden: ponds
# ringed with reeds and flowers, plasma on the bench, ammo and a second bong
# health pack in the arbour. Two channels leave the lagoon west and east and
# run down to the south edge; the levee road crosses them on plank decks.
#
# West: the stilt village on its channel - three huts (health, ammo, the
# second sonic hammer), a fenced paddy, hay, oil drums. North-west: the reed
# marsh and a ruined shrine held by an enemy bio slush (plasma, a shield,
# health). North-east: the poachers' camp, an enemy gun tower in a sandbag
# nest before a pen of tyre stacks (minigun, missiles, health), fuel drums
# below the pen, speed by the track. East: the tea house in its palm grove
# (missiles, health, ammo), oil drums in its yard. More oil drums wait by
# the west track short of the west bridge, and two caches hide in the woods
# in the south corners - a shield and a minigun, speed and missiles, each
# with a health pack. The enemies are placed out of sight round the lagoon
# and come to the island over the channels and the ring.
"""


def write(path):
    lines = [HEADER, "version = 1",
             'bong = "gloop"',
             "tanks = 7",
             'tank = "glacier"',
             'tank2 = "scout"',
             'weather = "random"',
             'biome = "floodplain"',
             f"size = [{W}, {H}]",
             'mission.kind = "protect"',
             'spawn.kind = "band"',
             ""]
    for (c, r) in sorted(cells, key=lambda p: (p[1], p[0])):
        lines.append(f'cells."{c},{r}" = {cells[(c, r)]}')
    with open(path, "w") as f:
        f.write("\n".join(lines) + "\n")


def check():
    c0, r0, c1, r1 = ISLAND
    # the ring round the island, the channels and the ponds are fords: the
    # only deep water is the lagoon's heart
    for c, r in sorted(water):
        if is_deep(c, r):
            assert r < r0 + 2, f"deep water at {c},{r} outside the lagoon"
    for c, r in ring_cells():
        if r >= r0 + 2:
            assert (c, r) in water or (c, r) in roads, f"the ring is dry at {c},{r}"
            assert not is_deep(c, r), f"the ring is deep at {c},{r}"
    reach = nav_reach()
    near = lambda p, rr: any((p[0] + dc, p[1] + dr) in rr for dc in range(-4, 4) for dr in range(-4, 4))
    for p in (START, START2, BONG):
        assert near(p, reach), p
    for c, r, kind in pickups:
        assert reachable((c, r), reach), f"{kind} at {c},{r} cannot be reached"
    for c, r, kind in pickups:
        if kind in FIRST_HERE:
            firsts = [(pc, pr) for pc, pr, k in pickups if k == kind]
            assert any(abs(pc - START[0]) <= SIGHT[0] and abs(pr - START[1]) <= SIGHT[1] for pc, pr in firsts), \
                f"no {kind} in the start's sight box"
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
    'birch': 'b', 'tall_grass': '"', 'bush': 'o', 'berry_bush': 'o', 'autumn_bush': 'o', 'fern': ',',
    'reeds': '|', 'gate': 'G', 'tesla': 'Z', 'gun_tower': 'K', 'bio_slush': 'S', 'barrel': 'D', 'hay': 'h',
    'tyres': 'O', 'wood_crate': 'X', 'barrier': 'n', 'tank_trap': '*', 'juniper': 'o',
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
                ch = {'brick': '#', 'iron': 'I', 'wood': 'w', 'concrete': 'C', 'glass': 'g'}.get(cells[(c, r)].split('"')[3], '#')
            if k == 'water' and is_deep(c, r):
                ch = '%'
            row.append(ch)
        print(f"{r:3d} " + "".join(row))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("-o", "--out", default="maps/lotus-lagoon.toml")
    ap.add_argument("--stats", action="store_true")
    ap.add_argument("--ascii", action="store_true")
    a = ap.parse_args()
    filled = build()
    if a.ascii:
        ascii_map()
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
        cost = walk_costs()
        for c, r, kind in pickups:
            walk = min((cost[n] for n in nav_of((c, r)) if n in cost), default=None)
            print(f"  pickup {kind:13} {c},{r}  walk {walk}")


if __name__ == "__main__":
    main()
