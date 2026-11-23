"""Write maps/carnival.toml: Carnival, level 5 - a fairground in a park,
87 x 43 cells, laid out set piece by set piece.

  python3 tools/gen_carnival.py [-o maps/carnival.toml] [--stats] [--ascii] [--nav]

A map this size is too big to lay out by hand, so this script is the
source: change it and run it again rather than editing the TOML (a
re-save from the map editor would drop the header too). Nothing is drawn
at random - the noise is hashed from cell positions - so the script
writes the same file every time. `--ascii` prints the map a character a
cell, `--nav` the nav grid the AI routes on (what is reached, what is one
cell wide), `--stats` the counts and each gate's walk to the start. The
checks run every time and fail the run: every pickup, start and gate
lane reached, no boxed-in nav cell, no drum by a start, and no loose
drum or prop on a wave's cheapest way in (the router drives straight
through loose props, and a hull that meets one on its way has to shove
it).

What it lays out (the header written into the map says how it plays):

- the showmen's camp in the west, the players' start: a ring of wagons
  with gun towers at its gate, a tesla and a bio slush, the stores
  inside and the EMP by the start;
- the midway, the fair's main road east from the camp through the
  entrance arch, lined with stalls and their stock of oil drums;
- north of it the hay farm, the big top (striped canvas, a stash round
  the ring), the shooting gallery, the Ferris wheel plaza and the log
  flume;
- south of it the boating lake (an island reached by a plank jetty), the
  generator yard, the carousel, the dodgems ringed in tyres, the
  fireworks pad, the haunted house and the helter skelter;
- in the east the rival circus's camp, the enemy's: wagons and iron
  cages, their towers and their stash;
- the park everywhere else: trees of several kinds, flower beds, long
  grass, lamps along the midway.
"""

import argparse
import math
from collections import deque
import heapq

W, H = 87, 43

# (col, row) -> inline TOML table text
cells = {}
# cells the park pass must leave alone
reserved = set()
water = set()
roads = set()

# the starts stand out of every line through the wagons' gaps, so a shell
# fired at a tower in a gap does not fly on into a seat behind it
START = (11, 16)
START2 = (11, 26)

GATES = [
    (0, 8),  # west, past the hay farm
    (43, 0), (66, 0),  # north, either side of the Ferris plaza
    (86, 13), (86, 31),  # east, either side of the rival camp
    (39, 42), (61, 42),  # south, either side of the fireworks
]

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


def ring(cx, cy, rad):
    """The cells of a circle's outline: those inside the disc with any of
    their eight neighbours outside it, so the ring is closed side to side
    (4-connected) and reads as one wall, never a string of corners."""
    inside = set(disc(cx, cy, rad))
    for c, r in sorted(inside, key=lambda p: (p[1], p[0])):
        if any((c + dc, r + dr) not in inside for dc in (-1, 0, 1) for dr in (-1, 0, 1)):
            yield c, r


# the set pieces' open ground, which the park keeps off
OPEN_GROUND = set()
# the set pieces' insides, which the meadow keeps off too
INTERIOR = set()


def inside(cells_):
    INTERIOR.update(cells_)


def clear_rect(c0, r0, c1, r1):
    for c, r in list(rect(c0, r0, c1, r1)):
        if (c, r) not in roads and (c, r) not in water:
            clear(c, r)
        reserved.add((c, r))
        OPEN_GROUND.add((c, r))


def clear_disc(cx, cy, rad):
    for c, r in list(disc(cx, cy, rad)):
        if (c, r) not in roads and (c, r) not in water:
            clear(c, r)
        reserved.add((c, r))
        OPEN_GROUND.add((c, r))


def keep_clear(c0, r0, c1, r1):
    """A walkway between set pieces: the park leaves it open."""
    for p in rect(c0, r0, c1, r1):
        reserved.add(p)
        OPEN_GROUND.add(p)


def pickup(c, r, kind):
    assert (c, r) not in water and (c, r) not in roads, f"{kind} at {c},{r} would cover water or road"
    put(c, r, "pickup", pickup=kind)
    pickups.append((c, r, kind))


DRUMS = []


def drum(c, r, kind):
    put(c, r, "barrel", drum=kind)
    DRUMS.append((c, r))


def drums(spots, salt, fuel=0.35):
    """Drums at `spots`, each oil or fuel by a hash of its cell: `fuel` of
    them the grey fuel drums that launch when chained."""
    for c, r in spots:
        drum(c, r, "fuel" if hash01(c, r, salt) < fuel else "oil")


def cluster(cx, cy, n, salt, fuel=0.35):
    """`n` drums packed round (cx, cy), nearest first - a heap that goes up
    together."""
    near = sorted(rect(cx - 2, cy - 2, cx + 2, cy + 2),
                  key=lambda p: ((p[0] - cx) ** 2 + (p[1] - cy) ** 2, hash01(*p, salt)))
    spots = [p for p in near if p not in cells and p not in water][:n]
    drums(spots, salt, fuel)


def oil_trail(points):
    """An oil trail along `points`: a fuse a blast or a fire runs along to
    the drums at its end."""
    for c, r in polyline(points):
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
    the `margin` round it is kept clear of the park."""
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


def blob(cx, cy, rx, ry, salt):
    """A lake: an ellipse with a ragged shore."""
    out = []
    for r in range(int(cy - ry) - 2, int(cy + ry) + 3):
        for c in range(int(cx - rx) - 2, int(cx + rx) + 3):
            if not inb(c, r):
                continue
            dx, dy = (c - cx) / rx, (r - cy) / ry
            wobble = 0.45 * (fbm(c / 3, r / 3, salt) - 0.5)
            if dx * dx + dy * dy <= 1.0 + wobble:
                put(c, r, "water")
                out.append((c, r))
    return out


# ---------------------------------------------------------------- set pieces


def showmen_camp():
    """The players' start: the showmen's wagons drawn up in a ring, gun
    towers either side of the gate onto the midway, a tesla between the
    two starts and a bio slush on the north side, the stores inside."""
    clear_rect(1, 11, 18, 30)
    inside(rect(3, 14, 16, 28))
    wagons = [(4, 13, 6, 14), (10, 12, 12, 13), (16, 14, 17, 16), (16, 26, 17, 28),
              (10, 29, 12, 30), (4, 28, 6, 29), (1, 17, 2, 19), (1, 23, 2, 25)]
    for i, (c0, r0, c1, r1) in enumerate(wagons):
        for c, r in rect(c0, r0, c1, r1):
            wall(c, r, "wood" if i % 2 == 0 else "brick")
    put(17, 18, "gun_tower")
    put(17, 25, "gun_tower")
    put(6, 21, "tesla")
    put(6, 16, "bio_slush")
    for c, r in ((19, 17), (19, 18), (19, 25), (19, 26)):
        put(c, r, "sandbag")
    put(*START, "start")
    put(*START2, "start2")
    # the stores: the EMP three cells from the start, the rest round the ring
    pickup(13, 21, "emp_burst")
    pickup(13, 17, "missiles")
    pickup(4, 18, "health")
    pickup(4, 26, "ammo")
    pickup(13, 26, "health")
    pickup(3, 21, "tower_pack")
    put(3, 15, "wood_crate")
    put(3, 27, "hay")


def entrance_arch():
    """The fair's gate east of the camp: stone pillars with a lamp on
    each."""
    for c, r in rect(21, 17, 22, 18):
        wall(c, r, "brick")
    for c, r in rect(21, 25, 22, 26):
        wall(c, r, "brick")
    put(23, 19, "lamp")
    put(23, 24, "lamp")


def hay_farm():
    """The show farm in the north-west: a barn (plasma inside, and the
    tractor's fuel), the pony paddock fenced off, a pumpkin patch, hay
    bales about."""
    clear_rect(1, 1, 20, 10)
    # the barn against the west edge, its door on the south side
    for c, r in outline(1, 1, 7, 5):
        if r == 5 and c in (3, 4, 5):
            continue
        wall(c, r, "wood")
    pickup(2, 3, "plasma")
    pickup(6, 3, "health")
    pickup(9, 4, "flamethrower")
    # the paddock, gates west and south
    for c, r in outline(11, 1, 17, 7):
        if (c == 11 and r in (3, 4, 5)) or (r == 7 and c in (13, 14, 15)):
            continue
        put(c, r, "fence")
    for c, r in rect(12, 2, 16, 6):
        if hash01(c, r, 3) < 0.55:
            put(c, r, "tall_grass")
    for c, r in ((13, 3), (15, 5), (9, 2), (19, 3), (19, 4)):
        put(c, r, "hay")
    drums([(15, 2), (16, 2), (16, 3), (10, 2), (10, 3)], 3)
    # the pumpkin patch
    for c, r in rect(6, 7, 12, 10):
        if r % 2 == 1 and hash01(c, r, 7) < 0.8:
            put(c, r, "autumn_bush")
    # the tractor's fuel, kept in the barn
    drums([(3, 2), (4, 2), (5, 2)], 5, fuel=0.7)


def big_top():
    """The circus tent: a round wall of striped canvas - orange and white
    by turns - with doors three wide on the south, east and west, the
    sawdust ring inside, the stash round it and the human cannonball's
    fuel drums inside the wall."""
    cx, cy, rad = 30, 9, 6.6
    clear_disc(cx, cy, rad + 1.4)
    inside(disc(cx, cy, rad))
    for c, r in ring(cx, cy, rad):
        dx, dy = c - cx, r - cy
        if (abs(dx) <= 1 and dy > 0) or abs(dy) <= 1:
            continue  # the doors, south, east and west
        a = math.atan2(dy, dx)
        stripe = int((a + math.pi) / (math.pi / 8)) % 2
        wall(c, r, "wood" if stripe else "brick")
    for c, r in disc(cx, cy, 2.2):
        put(c, r, "road")
    pickup(27, 7, "laser")
    pickup(33, 7, "grenades")
    pickup(27, 11, "health")
    pickup(33, 11, "ammo")
    drums([(27, 5), (29, 5), (31, 5), (33, 5), (27, 13), (33, 13)], 7, fuel=1.0)
    # the grove behind the tent, close against it
    wd = distance_field(water, 3)
    for c, r in rect(22, 0, 40, 5):
        if (c, r) not in cells and math.hypot(c - cx, r - cy) > rad + 0.4:
            put(c, r, tree_for(c, r, wd))
    # the approach from the midway
    road([(29, 17), (29, 20)])


def shooting_gallery():
    """A booth on the midway's north side: range boards at the back, a
    sandbag counter at the front with a way in, the prize inside."""
    clear_rect(38, 12, 48, 18)
    inside(rect(39, 13, 47, 18))
    for c, r in outline(39, 13, 47, 18):
        if r == 18:
            continue
        wall(c, r, "wood")
    for c in (41, 43, 45):
        put(c, 14, "target")
    for c in (40, 41, 42, 46):
        put(c, 18, "sandbag")
    pickup(44, 16, "minigun")
    drums([(40, 16), (40, 17)], 13)


def ferris_plaza():
    """The Ferris wheel on its paved plaza: an iron rim on eight spokes, a
    glass gondola between each pair of spokes; a food truck's fuel behind
    it, lamps round it. The wheel is one solid piece: nothing
    is left open between its spokes to be trapped in."""
    cx, cy = 55, 9
    clear_disc(cx, cy, 8.0)
    inside(disc(cx, cy, 6.6))
    for c, r in disc(cx, cy, 6.6):
        put(c, r, "road")
    for c, r in disc(cx, cy, 4.6):
        dx, dy = c - cx, r - cy
        d = math.hypot(dx, dy)
        if round(d) >= 4:
            ends = (dx == 0 or dy == 0 or abs(dx) == abs(dy))
            wall(c, r, "iron" if ends else "glass")
        elif dx == 0 or dy == 0 or abs(dx) == abs(dy):
            wall(c, r, "iron")  # hub and spokes
        else:
            clear(c, r)
    for c, r in ((49, 3), (61, 3)):
        put(c, r, "lamp")
    drums([(56, 1), (57, 1), (58, 1)], 11)
    pickup(47, 9, "missiles")
    pickup(63, 9, "health")
    pickup(55, 1, "shield")
    road([(55, 16), (55, 20)])


def log_flume():
    """The log flume in the north-east: a channel of water one cell wide
    round an island, a splash pool where the logs come down, plank bridges
    over it west and south. On the island the station - the second EMP in
    it - and an enemy gun tower on its platform."""
    clear_rect(69, 1, 85, 11)
    channel = polyline([(72, 2), (83, 2), (83, 9), (72, 9), (72, 2)])
    for c, r in channel:
        put(c, r, "water")
    for c, r in rect(80, 8, 83, 10):
        put(c, r, "water")
    # the bridges: one road cell over the channel, water either side
    put(72, 6, "road")
    put(76, 9, "road")
    # the station
    for c, r in outline(75, 4, 79, 6):
        if r == 6 and c in (76, 77, 78):
            continue
        wall(c, r, "wood")
    pickup(76, 5, "emp_burst")
    pickup(78, 5, "ammo")
    pickup(74, 4, "minigun")
    put(74, 7, "gun_tower", side="enemy")
    for c, r in ((80, 4), (81, 4)):
        put(c, r, "wood_crate")
    cluster(80, 6, 3, 17, fuel=0.6)
    for c, r in rect(73, 3, 82, 3):
        if hash01(c, r, 19) < 0.5:
            put_free(c, r, "tall_grass")
    for c, r in ((70, 4), (70, 8), (85, 5)):
        put_free(c, r, "willow")


def stalls():
    """The midway's stalls, both sides: each a booth with a counter to the
    road and a back of another material. Their stock is oil drums: behind
    the north stalls in a chain, near enough each to the next that the row
    goes up as one; in the back of each south stall, out of the lane behind
    them that the waves drive along."""
    north = [(48, 50), (58, 60), (64, 66)]
    south = [(27, 29), (35, 37), (41, 43), (50, 52), (57, 59), (66, 68)]
    looks = [("glass", "wood"), ("wood", "brick"), ("glass", "iron"), ("brick", "wood")]
    for i, (c0, c1) in enumerate(north):
        front, back = looks[i % len(looks)]
        for c in range(c0, c1 + 1):
            wall(c, 19, front)
            wall(c, 18, back)
    for i, (c0, c1) in enumerate(south):
        front, back = looks[(i + 1) % len(looks)]
        for c in range(c0, c1 + 1):
            wall(c, 24, front)
            if c == c0 + 1:
                drum(c, 25, "fuel" if i % 3 == 1 else "oil")
            else:
                wall(c, 25, back)
    drums([(58, 17), (60, 17), (62, 17), (64, 17), (66, 17)], 23, fuel=0.3)
    for c, r in ((49, 17), (50, 17)):
        put(c, r, "wood_crate")
    # the hook-a-duck pool behind the north stalls
    for c, r in rect(51, 17, 53, 17):
        put(c, r, "water")
    pickup(52, 16, "speedup")
    # lamps down the midway, never two across the road from each other
    for c in (38, 49, 64):
        put(c, 20, "lamp")
    for c in (28, 42, 57, 68):
        put(c, 23, "lamp")


def boating_lake():
    """The boating lake in the south-west corner, running off both edges:
    deep in the middle, an island with the third EMP, a health pack and
    the boats' fuel on it, reached from the north shore by a plank jetty
    one cell wide."""
    blob(9, 38.5, 10.5, 5.2, 41)
    for c, r in rect(7, 37, 11, 39):
        clear(c, r)
    pickup(8, 38, "emp_burst")
    pickup(10, 38, "health")
    drums([(7, 39), (11, 39)], 9, fuel=1.0)  # the boats' fuel
    for r in range(31, 37):
        if (9, r) in water or r >= 33:
            put(9, r, "road")


def generator_yard():
    """The generators that light the fair: fuel and oil drums heaped at the
    back of a yard of tyre stacks, out of the way between the lake and the
    carousel, and an oil trail from them up to the south stalls' stock."""
    clear_rect(20, 31, 25, 41)
    inside(rect(20, 31, 25, 41))
    for c, r in ((20, 32), (25, 32)):
        put(c, r, "tyres")
    drums([(c, r) for c in range(21, 25) for r in (40, 41)], 37, fuel=0.5)
    pickup(21, 33, "ammo")
    pickup(24, 33, "health")
    keep_clear(22, 27, 30, 31)
    oil_trail([(22, 39), (22, 28), (28, 28), (28, 26)])


def carousel():
    """The carousel: a round of painted horses - glass and wood by turns -
    about the centre pole, doors three wide north, to the midway, and
    south, to the south gate."""
    cx, cy, rad = 33, 33, 4.9
    clear_disc(cx, cy, rad + 1.2)
    inside(disc(cx, cy, rad))
    for c, r in ring(cx, cy, rad):
        dx, dy = c - cx, r - cy
        if abs(dx) <= 1:
            continue
        wall(c, r, "glass" if (c + r) % 2 else "wood")
    wall(cx, cy, "iron")
    pickup(31, 31, "health")
    pickup(35, 35, "speedup")
    drums([(31, 35), (35, 31)], 19)
    road([(32, 23), (32, 27)])


def dodgems():
    """The dodgems: a floor ringed in tyre stacks that throw a hull back,
    doors three wide at the four points, crates to shove about in the
    corners, a row of drums in one and the sonic hammer to shove with."""
    c0, r0, c1, r1 = 41, 30, 53, 36
    clear_rect(c0 - 1, r0 - 1, c1 + 1, r1 + 1)
    inside(rect(c0, r0, c1, r1))
    for c, r in outline(c0, r0, c1, r1):
        if c in (46, 47, 48) or (r in (32, 33, 34) and c in (c0, c1)):
            continue
        put(c, r, "tyres")
    for c, r in ((42, 31), (44, 35), (50, 31), (51, 31)):
        put(c, r, "wood_crate")
    drums([(49, 35), (50, 35), (51, 35)], 47)
    pickup(43, 34, "sonic_hammer")
    pickup(51, 33, "health")
    road([(46, 23), (46, 29)])


def fireworks():
    """The fireworks pad south of the dodgems: a rack of fuel drums - which
    launch and go off where they land when another blast sets them off -
    against the south edge, clear of the way past them, an oil trail up
    to them from the dodgems, and the flamethrower beside."""
    clear_rect(42, 37, 55, 41)
    inside(rect(43, 37, 54, 41))
    drums([(c, 41) for c in range(45, 53)] + [(47, 40), (48, 40), (49, 40), (50, 40)], 53, fuel=0.85)
    oil_trail([(48, 37), (48, 39)])
    pickup(43, 38, "flamethrower")


def haunted_house():
    """The haunted house: a grey stone house of two rooms, a door three
    wide between them, the front door north and the back door east,
    grenades and a shield in the back room, dead trees and an enemy bio
    slush in the garden."""
    c0, r0, c1, r1 = 57, 30, 67, 37
    clear_rect(c0 - 1, r0 - 1, c1 + 1, r1 + 1)
    inside(rect(c0, r0, c1, r1))
    for c, r in outline(c0, r0, c1, r1):
        if r == r0 and c in (63, 64, 65):
            continue  # the front door
        if c == c1 and r in (33, 34, 35):
            continue  # the back door
        wall(c, r, "brick")
    for r in range(r0 + 1, r1):
        if r not in (32, 33, 34):
            wall(61, r, "brick")
    pickup(58, 31, "grenades")
    pickup(58, 36, "shield")
    pickup(66, 31, "health")
    drums([(62, 36), (63, 36)], 57)
    keep_clear(55, 38, 69, 40)
    put(57, 41, "bio_slush", side="enemy")
    for c, r in ((55, 41), (66, 41), (69, 40), (58, 41)):
        put_free(c, r, "snag")
    cluster(65, 40, 3, 59)
    pickup(68, 39, "health")
    road([(63, 23), (63, 29)])


def helter_skelter():
    """The helter skelter: a round tower striped in a spiral, orange and
    white, the prize at its foot."""
    cx, cy, rad = 76, 35, 2.7
    clear_rect(70, 30, 82, 41)
    inside(rect(72, 32, 80, 40))
    for c, r in disc(cx, cy, rad):
        a = math.atan2(r - cy, c - cx)
        d = math.hypot(c - cx, r - cy)
        stripe = int((a + math.pi) / (2 * math.pi) * 4 + d * 0.9) % 2
        wall(c, r, "wood" if stripe else "brick")
    pickup(72, 34, "missiles")
    pickup(74, 39, "health")
    cluster(81, 39, 4, 61)


def rival_camp():
    """The rival circus's camp at the east end, the enemy's: wagons and iron
    animal cages round it, two gun towers either side of its gate on the
    midway, a tesla in the middle with their stash round it, a bio slush at
    the back, and drums beside the towers."""
    clear_rect(71, 14, 86, 28)
    inside(rect(72, 14, 86, 28))
    wagons = [(74, 14, 76, 15, "wood"), (80, 14, 81, 15, "iron"), (85, 15, 86, 16, "wood"),
              (85, 26, 86, 27, "wood"), (80, 27, 81, 28, "iron"), (74, 27, 76, 28, "wood")]
    for c0, r0, c1, r1, material in wagons:
        for c, r in rect(c0, r0, c1, r1):
            wall(c, r, material)
    put(74, 19, "gun_tower", side="enemy")
    put(74, 23, "gun_tower", side="enemy")
    put(80, 21, "tesla", side="enemy")
    put(85, 21, "bio_slush", side="enemy")
    for c, r in ((77, 17), (78, 17), (77, 25), (78, 25)):
        put(c, r, "sandbag")
    pickup(78, 19, "laser")
    pickup(78, 23, "plasma")
    pickup(82, 19, "health")
    pickup(82, 23, "ammo")
    drums([(73, 18), (75, 18), (73, 24), (75, 24), (77, 16), (78, 16), (77, 26), (78, 26)], 73, fuel=0.5)
    road([(72, 21), (76, 21)])


def park_paths():
    """The midway: the fair's main road from the camp's gate east to the
    rival camp."""
    road([(16, 21), (77, 21)])


# ---------------------------------------------------------------- the park


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


TREES = (("tree", 45), ("birch", 25), ("scots", 12), ("spruce", 8), ("fir", 10))
BANK = (("willow", 70), ("birch", 30))
FLOWERS = (("berry_bush", 35), ("autumn_bush", 30), ("bush", 20), ("juniper", 15))


def tree_for(c, r, wd):
    if wd.get((c, r), 9) <= 2:
        return pick(BANK, hash01(c, r, 73))
    return pick(TREES, hash01(c, r, 79))


WALKWAYS = [
    (7, 6, 9, 12),  # the camp's north gap to the hay farm
    (19, 7, 24, 11),  # the hay farm to the big top's west door
    (36, 6, 48, 11), (41, 0, 45, 11),  # the big top to the plaza, the north gate
    (62, 4, 70, 8), (64, 0, 68, 8),  # the plaza to the flume, the north gate
    (73, 11, 86, 13),  # the flume to the east gate
    (52, 0, 58, 2),  # the shield behind the Ferris wheel
    (62, 11, 72, 17),  # the plaza to the rival camp
    (78, 29, 86, 32),  # the rival camp to the east gate
    (68, 29, 72, 37),  # the haunted house's back door to the helter skelter
    (60, 38, 63, 42),  # the haunted garden to the south gate
    (37, 36, 40, 42),  # the carousel to the south gate
    (26, 32, 28, 38),  # the generator yard to the carousel
    (7, 29, 9, 32),  # the camp's south gap to the jetty
    (54, 27, 56, 37),  # the dodgems to the haunted house
    (16, 16, 72, 27),  # the midway and its alleys
]


PARK_GROUND = []


def park():
    """Everything the fair leaves: a park. A belt of trees along the edges,
    groves where the noise is high and the fair is not near, flower beds
    and long grass at their skirts, reeds on the lake's shore."""
    for w in WALKWAYS:
        keep_clear(*w)
    wd = distance_field(water, 3)
    solid = {p for p in cells if blocks(p)}
    near_fair = distance_field(solid | OPEN_GROUND, 3)
    free = sorted(fbm(c / 5, r / 5, 11) for r in range(H) for c in range(W)
                  if (c, r) not in cells and (c, r) not in reserved)
    PARK_GROUND.append(len(free))
    q = lambda share: free[int(len(free) * share)]
    grove, skirt = q(0.55), q(0.4)
    for r in range(H):
        for c in range(W):
            p = (c, r)
            if p in cells or p in reserved:
                continue
            u = hash01(c, r, 13)
            v = hash01(c, r, 19)
            edge = min(c, r, W - 1 - c, H - 1 - r)
            d = fbm(c / 5, r / 5, 11)
            room = near_fair.get(p, 9) >= 3
            if wd.get(p, 9) == 1 and hash01(c, r, 85) < 0.55:
                put(c, r, "reeds")
            elif edge <= 1 and u < 0.9:
                put(c, r, tree_for(c, r, wd))
            elif room and d > grove and u < 0.85:
                put(c, r, tree_for(c, r, wd))
            elif d > skirt and v < 0.45:
                put(c, r, pick(FLOWERS, hash01(c, r, 23)))
            elif d > skirt and v < 0.6:
                put(c, r, "tall_grass")
            elif v < 0.05:
                put(c, r, pick(FLOWERS, hash01(c, r, 29)))
            else:
                continue
            reserved.discard(p)


def dressing():
    """What makes the lawns between the rides a park and a trap: clumps of
    trees, and heaps of drums beside the ways the waves come in - near
    enough that a shot sets one off under a passing tank, off the way
    itself (`loose_on_routes`)."""
    wd = distance_field(water, 3)
    clumps = [(61, 1, 1.2), (84, 36, 1.4)]
    for cx, cy, rad in clumps:
        for c, r in disc(cx, cy, rad):
            if (c, r) not in cells and (c, r) not in INTERIOR and hash01(c, r, 97) < 0.9:
                put(c, r, tree_for(c, r, wd))
    traps = [((21, 29), 5, 89), ((83, 34), 4, 109), ((24, 14), 3, 127), ((16, 34), 3, 131),
             ((36, 16), 3, 139), ((28, 39), 3, 149)]
    for (c, r), n, salt in traps:
        cluster(c, r, n, salt)
    drums([(84, 6), (84, 7), (16, 31), (17, 31)], 151)


GATE_APRON = set()


def meadow():
    """Flower beds and long grass over the lawns the fair left open, and a
    sprinkle of flowers between - soft cover, so no route changes - but
    never inside a ride, on a gate's lane or round a start."""
    spots = [(c, r) for r in range(H) for c in range(W)
             if (c, r) not in cells and (c, r) not in INTERIOR and (c, r) not in GATE_APRON
             and min(math.hypot(c - s[0], r - s[1]) for s in (START, START2)) > 3]
    noise = sorted(fbm(c / 4, r / 4, 31) for c, r in spots)
    q = lambda share: noise[int(len(noise) * share)]
    bed, grass = q(0.84), q(0.7)
    for c, r in spots:
        d = fbm(c / 4, r / 4, 31)
        u = hash01(c, r, 37)
        if d > bed and u < 0.7:
            put(c, r, pick(FLOWERS, hash01(c, r, 41)))
        elif d > grass and u < 0.45:
            put(c, r, "tall_grass")
        elif u < 0.025:
            put(c, r, pick(FLOWERS, hash01(c, r, 43)))


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
    """Open ground the playfield never reaches becomes park, so no nav
    cell is left boxed in or cut off."""
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
                    put(*p, tree_for(*p, wd))
                    filled += 1
                    break
    return filled


def close_narrow_trails():
    """An empty cell one wide among trees - something on both sides across
    it - is a trail a hull can take but the AI never will (the linter's
    narrow trail): it grows a bush, soft cover that changes no route."""
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
    in from every gate."""
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
            GATE_APRON.add(p)


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


# ---------------------------------------------------------------- the walk


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
    wave comes in. A loose drum or prop on one is a thing the router drives
    straight at (it routes through loose props) and the hull then has to
    shove out of the way, so the fair keeps them beside the ways in."""
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


# ---------------------------------------------------------------- the map


def build():
    park_paths()
    showmen_camp()
    entrance_arch()
    hay_farm()
    big_top()
    shooting_gallery()
    ferris_plaza()
    log_flume()
    stalls()
    boating_lake()
    generator_yard()
    carousel()
    dodgems()
    fireworks()
    haunted_house()
    helter_skelter()
    rival_camp()
    dressing()
    gate_aprons()
    for gc, gr in GATES:
        put(gc, gr, "gate")
        GATE_LANES.update(lane(gc, gr))
    park()
    meadow()
    filled = fill_pockets()
    close_narrow_trails()
    return filled


HEADER = """\
# carnival - Carnival, level 5: a fairground in a park, and its oil drums.
# Written by tools/gen_carnival.py - change the generator and run it again
# rather than editing this file; re-saving from the map editor would drop
# these comments. Level tables as dotted keys on purpose - a `[spawn]` header
# would swallow every `cells.` line after it.
#
# Destroy on the waves plan: six waves of 3 to 8 tanks, 33 in all, light
# ones first and medium ones later - many, and quick to wreck, so the fight
# is the drums going up round them rather than a grind. They roll in at
# seven gates: two on the north edge, two on the south, two on the east
# either side of the rival camp and one on the west past the hay farm. A
# clear sky or dusk, when the lamps come on. 87 x 43 cells, followed by the
# camera, in the valley's look: a meadow, clear water, plank bridges.
#
# The players start in the showmen's camp in the west: their wagons drawn
# up in a ring, two gun towers either side of its gate onto the midway, a
# tesla and a bio slush inside, and the stores - the EMP by the start (first
# met here: a pulse stops a crowd and kills a missile volley in the air),
# missiles, two health packs, ammo and a tower pack. The starts stand out of
# the line of every gap in the ring.
#
# The midway runs east from the camp through the fair's arch to the rival
# circus's camp at the far end: stalls both sides, their stock of oil drums
# behind the north row in a chain that goes up as one, and in the back of
# each south stall. North of it: the hay farm (plasma and a flamethrower by
# the barn, hay to burn, the tractor's fuel); the big top, a ring of
# striped canvas with the laser, grenades, health and ammo round the
# sawdust ring and the human cannonball's fuel drums inside its wall; the
# shooting gallery (a minigun behind its range boards); the Ferris wheel
# on its plaza (missiles, health, a shield behind it); and the log flume,
# a channel of water round an island, the second EMP and a minigun in its
# station under an enemy gun tower. South of it: the boating lake, deep,
# the third EMP and health on its island at the end of a plank jetty; the
# generator yard, its drums on an oil trail up to the stalls; the carousel;
# the dodgems, ringed in tyre stacks that throw a hull back, the sonic
# hammer inside; the fireworks pad, a rack of fuel drums that launch when
# set off, an oil trail to it and a flamethrower beside it; the haunted
# house (grenades, a shield), an enemy bio slush in its garden; the helter
# skelter, missiles at its foot.
#
# The rival camp at the east end is the enemy's: wagons and iron cages,
# two gun towers either side of its gate on the midway with drums beside
# them, a tesla in the middle with their stash round it (laser, plasma,
# health, ammo) and a bio slush at the back. Over a hundred drums in all,
# heaped beside the ways in rather than on them, and the park between:
# trees, flower beds, long grass to wait in.
"""


def write(path):
    lines = [HEADER, "version = 1",
             'tank = "flak"',
             'tank2 = "wraith"',
             'biome = "valley"',
             'weather = ["clear", "dusk"]',
             "size = [87, 43]",
             'mission.kind = "destroy"',
             'spawn.kind = "waves"',
             "spawn.waves = 6",
             "spawn.size = 3",
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
        if math.hypot(c - START[0], r - START[1]) < 7 or math.hypot(c - START2[0], r - START2[1]) < 7:
            problems.append(f"drum at {c},{r} is by a start")
    for c, r in loose_on_routes():
        problems.append(f"{kind_at((c, r))} at {c},{r} stands on a wave's way in")
    return problems


ASCII = {
    'wall': '#', 'road': '=', 'water': '~', 'start': '1', 'start2': '2',
    'pickup': 'P', 'sandbag': 's', 'fence': 'f', 'tree': 'T', 'pine': 'T', 'spruce': 'T', 'scots': 'T',
    'fir': 'T', 'palm': 'Y', 'snag': 'x', 'willow': 'W', 'birch': 'b', 'tall_grass': '"', 'bush': 'o',
    'berry_bush': 'o', 'autumn_bush': 'o', 'juniper': 'o', 'fern': ',', 'reeds': '|', 'gate': 'G',
    'tesla': 'Z', 'gun_tower': 'K', 'bio_slush': 'Q', 'barrel': 'D', 'hay': 'h', 'tyres': 'O',
    'wood_crate': 'X', 'barrier': 'n', 'tank_trap': '*', 'lamp': 'L', 'oil': ':', 'target': '@',
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
                ch = {'brick': '#', 'iron': 'I', 'wood': 'w', 'concrete': 'C', 'glass': 'g'}.get(
                    cells[(c, r)].split('"')[3], '#')
            if k == 'water' and is_deep(c, r):
                ch = '%'
            if k == 'barrel':
                ch = 'F' if 'fuel' in cells[(c, r)] else 'D'
            if k in ('gun_tower', 'tesla', 'bio_slush') and 'enemy' in cells[(c, r)]:
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
    ap.add_argument("-o", "--out", default="maps/carnival.toml")
    ap.add_argument("--stats", action="store_true")
    ap.add_argument("--ascii", action="store_true")
    ap.add_argument("--nav", action="store_true")
    a = ap.parse_args()
    filled = build()
    if a.ascii:
        ascii_map()
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
        print(f"{len(cells)} cells, {PARK_GROUND[0]} left to the park, {filled} pocket cells filled, "
              f"{len(narrow())} one-wide nav cells")
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
