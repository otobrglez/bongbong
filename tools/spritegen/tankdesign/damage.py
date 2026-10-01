"""Damage for the tank design kit: four live tiers and four wrecks.

Damage is *planned once* per design and layer from the pristine render
(where the scorches, scratches, dents and holes go, which armour plate
comes off first, which lamp breaks first), then each tier applies a
longer prefix of that plan - so a scratch taken at 25 % is still there at
75 %, and a tank reads as the same tank getting worse, never as four
different tanks. The plan is seeded from the design's name (crc32, never
Python's salted `hash`), so a rebuild is byte-identical.

Burning never recolours pixel by pixel: every mark raises a *burn level*
on the pixels it covers (the maximum wins), and the levels are applied
once at the end - 1 and 2 darken a pixel along its own ramp, 3 swaps its
material for BURNT at the same shading offset (so a charred plate keeps
its bevels and still reads as a plate), 4 is soot-black. Scorch edges
follow a smooth noise field, not a per-pixel hash, so they come out as
blobs rather than salt and pepper.

Tiers (live, still driving and firing):
  0 pristine   0-24 %
  1 scuffed   25-49 %  scratches to bare metal, a scorch, a dent, one lamp
                       cracked, loose stowage gone
  2 damaged   50-74 %  an armour plate blown off (the frame and a cable
                       show), a penetration hole, antennas snapped, half
                       the lamps out, a spark at the wound
  3 critical  75-99 %  more plates gone and fire inside, a broken track
                       run, every light dead but a blinking warning lamp,
                       the silhouette chipped
Wrecks (four peers, one rolled per kill):
  blown    the turret ring blown out (the turret thrown clear), embers
  gutted   armour stripped, burning inside
  husk     cold, burnt out - the end state of every wreck
  cookoff  the deck torn open by an ammunition fire, one track run gone
"""

import math
import random
import zlib

from kit import (BLACK, DARK, GOLD_BRIGHT, RED_BRIGHT, RED_MD, RUST_DK, RUST_MD, SMOKED, STEEL, STONE_DARKEST,
                 STONE_DK, STONE_LT, STONE_MID, STONE_MDK, STONE_SHADE, WHITE, WOOD_ASH, CABLE_COLOURS, BLUE_PALE,
                 BLUE_BRIGHT, Mat, Part, bbox, compose, edge, lum)

TIERS = ['pristine', 'scuffed', 'damaged', 'critical']
TIER_RANGES = ['0-24 %', '25-49 %', '50-74 %', '75-99 %']
WRECKS = ['blown', 'gutted', 'husk', 'cookoff']

# Charred metal: brown-grey, ordered like any ramp so a shading offset
# carries over.
BURNT = Mat('burnt', [BLACK, STONE_DARKEST, RUST_DK, WOOD_ASH, STONE_SHADE, STONE_DK, STONE_MDK, STONE_MID], 3)


def seed_of(*parts):
    return zlib.crc32('|'.join(str(p) for p in parts).encode())


def _h(*a):
    return zlib.crc32(','.join(str(v) for v in a).encode()) / 4294967296.0


def smooth(x, y, s, cell=2.5):
    """Value noise in [0, 1): hashed lattice, bilinear - blob-shaped."""
    fx, fy = x / cell, y / cell
    ix, iy = math.floor(fx), math.floor(fy)
    tx, ty = fx - ix, fy - iy
    tx = tx * tx * (3 - 2 * tx)
    ty = ty * ty * (3 - 2 * ty)
    a = _h(ix, iy, s)
    b = _h(ix + 1, iy, s)
    c = _h(ix, iy + 1, s)
    d = _h(ix + 1, iy + 1, s)
    return (a * (1 - tx) + b * tx) * (1 - ty) + (c * (1 - tx) + d * tx) * ty


def _damageable(pix):
    p = pix.part
    if p.mode == 'glow':
        return False
    if p.tags & {'bore'}:
        return False
    return True


def _is_armour(pix):
    p = pix.part
    if p.mode == 'glow':
        return False
    if p.tags & {'track', 'barrel', 'bore', 'internal', 'optic', 'rivet', 'wheel', 'pad', 'vent'}:
        return False
    return True


# ---------------------------------------------------------------------------
# The plan
# ---------------------------------------------------------------------------
class Plan:
    pass


def _pkey(p):
    """A part's identity across renders (frames, tiers, teams): its name
    and first pixels - never `id()`, which a rebuilt part list changes."""
    return (p.name, tuple(sorted(p.px))[:3])


def _key(p):
    """A lamp's identity across frames and poses: its name, else its role
    and columns - never its rows, which move when a barrel recoils."""
    if p.name:
        return ('n', p.name)
    xs = [q[0] for q in p.px]
    return ('p', str(p.glow), min(xs), max(xs), len(p.px))


def make_plan(design_key, layer, pristine_parts, frame=0):
    rng = random.Random(seed_of(design_key, layer, 'plan'))
    grid = compose(pristine_parts, frame)
    armour_px = sorted(q for q, pix in grid.items() if _is_armour(pix))
    interior = [q for q in armour_px if all((q[0] + dx, q[1] + dy) in grid for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)))]
    if not interior:
        interior = armour_px or sorted(grid.keys())
    pl = Plan()
    pl.layer = layer
    pl.seed = seed_of(design_key, layer, 'live')
    # Scorch centres spread over the body: each new one prefers a spot
    # away from the ones already placed.
    pl.scorch = []
    placed = []
    for i in range(10):
        best, bd = None, -1
        for _ in range(12):
            q = rng.choice(interior)
            d = min((abs(q[0] - a) + abs(q[1] - b) for (a, b) in placed), default=99)
            if d > bd:
                best, bd = q, d
        placed.append(best)
        r = 1.3 + 0.3 * min(i, 6) + rng.random() * 0.5
        pl.scorch.append((best[0] + 0.5, best[1] + 0.5, r))
    # Scratches: short, never crossing each other.
    pl.scratch = []
    used = set()
    for i in range(12):
        for _ in range(20):
            x, y = rng.choice(interior)
            dx, dy = rng.choice([(1, 1), (1, 0), (0, 1), (1, -1)])
            pts = [(x + dx * k, y + dy * k) for k in range(2)]
            ring = {(a + u, b + v) for (a, b) in pts for u in (-1, 0, 1) for v in (-1, 0, 1)}
            if not (ring & used) and all(p in grid and _is_armour(grid[p]) for p in pts):
                used |= set(pts)
                pl.scratch.append(pts)
                break
    pl.dents = [rng.choice(interior) for _ in range(6)]
    # Holes, apart from each other.
    pl.holes = []
    for i in range(5):
        for _ in range(20):
            q = rng.choice(interior)
            if all(abs(q[0] - a) + abs(q[1] - b) > 4 for (a, b) in pl.holes):
                pl.holes.append(q)
                break
    while len(pl.holes) < 5:
        pl.holes.append(rng.choice(interior))
    # Armour plates in blow-off order: small first.
    arm = [p for p in pristine_parts if 'armor' in p.tags and p.px]
    rng.shuffle(arm)
    arm.sort(key=lambda p: len(p.px))
    pl.armour_order = [_pkey(p) for p in arm]
    # Lamps in breaking order: the small fixtures. Pads, engines and big
    # glowing parts (rings, strips, reactors) are not broken one by one -
    # they flicker, then sputter (`damage_parts`).
    lamps = [p for p in pristine_parts if _small_lamp(p)]
    rng.shuffle(lamps)
    pl.lamp_keys = [_key(p) for p in lamps]
    # The track run that breaks, and where.
    tracks = [p for p in pristine_parts if 'track' in p.tags and p.px]
    if tracks:
        t = rng.choice(tracks)
        x0, y0, x1, y1 = bbox(t.px)
        yb = rng.randint(y0 + 3, max(y0 + 3, y1 - 5))
        pl.track_break = (_tkey(t), yb, 2)
        other = [u for u in tracks if u is not t]
        pl.track_lost = _tkey(other[0]) if other else _tkey(t)
    else:
        pl.track_break = None
        pl.track_lost = None
    # Silhouette chips: convex corners of the armour first.
    occ = set(grid.keys())
    sil = [q for q in edge(occ) if _is_armour(grid[q])]

    def cornerness(q):
        return sum((q[0] + dx, q[1] + dy) not in occ for dx in (-1, 0, 1) for dy in (-1, 0, 1))
    sil.sort(key=lambda q: (-cornerness(q), _h(q[0], q[1], pl.seed)))
    pl.chips = sil[:20]
    return pl


# ---------------------------------------------------------------------------
# Part-level damage
# ---------------------------------------------------------------------------
def _internal(p, fire=None):
    q = Part(p.px, DARK, 'flat', p.z - 0.01, tags=('internal',), name=(p.name or '') + ':internal', contact=False)
    q.internal_glow = fire
    return q


def _small_lamp(p):
    return p.mode == 'glow' and len(p.px) <= 6 and not (set(p.tags) & {'pad', 'engine'})


def _dead_lamp(p):
    q = p.copy(mat=SMOKED, mode='flat', step=0, glow=None, blink=None)
    q.tags = (set(q.tags) - {'glow'}) | {'dead'}
    return q


def damage_parts(parts, tier, plan, wreck=None, layer='hull', internal_glow='fire'):
    """Remove or swap parts for a tier (0-3) or a wreck."""
    if tier == 0 and wreck is None:
        return list(parts)
    rng = random.Random(plan.seed + tier * 7919 + (WRECKS.index(wreck) * 104729 if wreck else 0))
    if wreck is None:
        n_arm = {1: 0, 2: 1, 3: 2}[tier]
    else:
        n_arm = {'blown': 1, 'gutted': 4, 'husk': 3, 'cookoff': 3}[wreck]
    blown = set(plan.armour_order[:n_arm])
    lamp_rank = {k: i for i, k in enumerate(plan.lamp_keys)}
    n_lamps = sum(1 for p in parts if _small_lamp(p))
    dead_n = n_lamps if wreck is not None else {1: min(1, n_lamps), 2: (n_lamps + 1) // 2, 3: n_lamps}[tier]
    fire = internal_glow if (wreck in ('gutted', 'cookoff') or (wreck is None and tier >= 3)) else None
    warn_given = False
    out = []
    for p in parts:
        if _pkey(p) in blown:
            out.append(_internal(p, fire))
            continue
        if p.mode == 'glow' and not _small_lamp(p):
            # Pads, engines, rings, strips: they flicker when damaged,
            # sputter when critical, and die with the tank.
            if wreck is not None:
                out.append(_dead_lamp(p))
            elif tier == 2:
                out.append(p.copy(blink={0, 1, 3}))
            elif tier == 3:
                # The turret has one frame, so a sputter there never shows
                # as off: a critical turret's big glows are simply out.
                out.append(_dead_lamp(p) if layer == 'turret' else p.copy(blink={0}))
            else:
                out.append(p)
            continue
        if p.mode == 'glow':
            rank = lamp_rank.get(_key(p), 0)
            if rank < dead_n:
                if wreck is None and tier == 3 and not warn_given and p.glow in ('marker', 'sensor', 'tail', 'beacon', 'warn'):
                    warn_given = True
                    out.append(p.copy(glow='warn', blink={0, 1}))
                    continue
                out.append(_dead_lamp(p))
                continue
            out.append(p)
            continue
        if 'antenna' in p.tags and (tier >= 2 or wreck is not None):
            continue
        if 'stowage' in p.tags and (tier >= 1 or wreck is not None):
            if wreck is None and tier == 1 and rng.random() < 0.5:
                out.append(p)
            continue
        if 'fragile' in p.tags and (tier >= 2 or wreck is not None):
            continue
        if 'skirt' in p.tags and (tier >= 3 or wreck is not None) and p.px:
            x0, y0, x1, y1 = bbox(p.px)
            if y1 - y0 >= 5:
                a = y0 + 2 + int(_h(x0, y0, plan.seed) * max(1, (y1 - y0 - 5)))
                n = 2 if wreck is None else 3
                out.append(p.copy(px={q for q in p.px if not (a <= q[1] < a + n)}))
                continue
        out.append(p)
    return out


# ---------------------------------------------------------------------------
# Pixel-level damage
# ---------------------------------------------------------------------------
class Burn:
    def __init__(self):
        self.level = {}

    def add(self, q, lv):
        if lv > self.level.get(q, 0):
            self.level[q] = lv

    def apply(self, grid):
        for q, lv in self.level.items():
            pix = grid.get(q)
            if pix is None or not _damageable(pix):
                continue
            if pix.color is not None:
                continue
            if 'internal' in pix.part.tags:
                continue
            if lv == 1:
                pix.off -= 1
            elif lv == 2:
                pix.off -= 2
            elif lv == 3:
                pix.mat = BURNT
                pix.off = max(-2, min(2, pix.off - pix.part.step))
            else:
                pix.mat = BURNT
                pix.off = max(-3, min(0, pix.off - pix.part.step - 2))


def scorch(burn, grid, cx, cy, r, top, s):
    """A blob of burn levels up to `top`, centred at (cx, cy)."""
    for q, pix in grid.items():
        if not _damageable(pix):
            continue
        x, y = q
        d = math.hypot(x + 0.5 - cx, y + 0.5 - cy)
        rr = r * (0.7 + 0.6 * smooth(x, y, s))
        if d >= rr:
            continue
        t = 1.0 - d / rr
        lv = 1 + int(t * top)
        burn.add(q, min(top, lv))


def scratch(grid, pts):
    for q in pts:
        pix = grid.get(q)
        if pix is None or not _is_armour(pix) or pix.color is not None:
            continue
        pix.color = STONE_MID if pix.off <= 0 else STONE_LT


def dent(grid, q):
    x, y = q
    a = grid.get((x, y))
    b = grid.get((x + 1, y + 1))
    if a and _is_armour(a):
        a.off -= 1
    if b and _is_armour(b):
        b.off += 1


def hole(grid, burn, q, hot=False, frame=0, big=False):
    x, y = q
    core = [(x, y), (x + 1, y)] + ([(x, y + 1), (x + 1, y + 1)] if big else [])
    for c in core:
        pix = grid.get(c)
        if pix is not None and _damageable(pix):
            pix.color = BLACK
            pix.emis = None
    for (a, b) in core:
        for dx in (-1, 0, 1):
            for dy in (-1, 0, 1):
                burn.add((a + dx, b + dy), 3)
    if hot:
        c = core[frame % 2] if len(core) > 1 else core[0]
        pix = grid.get(c)
        if pix is not None:
            col = GOLD_BRIGHT if frame % 2 == 0 else RED_BRIGHT
            pix.color = col
            pix.emis = col


def paint_internals(grid, frame, s, fire_on=True, cold=False):
    """Ribs, a frame edge and one cable where plates came off; a small
    fire where the part burns."""
    parts = {}
    for q, pix in grid.items():
        if 'internal' in pix.part.tags:
            parts.setdefault(id(pix.part), (pix.part, []))[1].append(q)
    for pid, (p, qs) in parts.items():
        qs.sort()
        qset = set(qs)
        cx = sum(q[0] for q in qs) / len(qs)
        cy = sum(q[1] for q in qs) / len(qs)
        inner = [q for q in qs if all((q[0] + dx, q[1] + dy) in qset for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)))]
        by_d = sorted(inner or qs, key=lambda q: (abs(q[0] + 0.5 - cx) + abs(q[1] + 0.5 - cy), q))
        for q in qs:
            x, y = q
            pix = grid[q]
            border = q not in inner
            if border:
                pix.color = STONE_SHADE if cold else STONE_DK
            else:
                pix.color = (BLACK if cold else STONE_DARKEST) if y % 2 == 0 else (STONE_DARKEST if cold else STONE_SHADE)
        if not cold and by_d:
            c = by_d[0]
            grid[c].color = CABLE_COLOURS[int(_h(c[0], c[1], s) * len(CABLE_COLOURS))]
            if len(inner) > 12 and len(by_d) > 3:
                c2 = by_d[3]
                grid[c2].color = CABLE_COLOURS[(int(_h(c2[0], c2[1], s) * len(CABLE_COLOURS)) + 1) % len(CABLE_COLOURS)]
        role = getattr(p, 'internal_glow', None)
        if role and fire_on and by_d:
            n = 2 if len(by_d) < 8 else 3
            for k, c in enumerate(by_d[1:1 + n]):
                if role == 'fire':
                    col = [GOLD_BRIGHT, RED_BRIGHT, RED_MD][(k + frame) % 3]
                elif role == 'ion':
                    col = [BLUE_PALE, BLUE_BRIGHT, WHITE][(k + frame) % 3]
                else:
                    col = [GOLD_BRIGHT, RED_BRIGHT][(k + frame) % 2]
                grid[c].color = col
                grid[c].emis = col


def spark(grid, frame, s):
    """One spark at the first wound, on alternate frames."""
    wound = sorted(q for q, pix in grid.items() if 'internal' in pix.part.tags)
    if not wound:
        wound = sorted(q for q, pix in grid.items() if pix.color == BLACK)
    if not wound:
        return
    q = wound[int(_h(s, 7) * len(wound))]
    if frame in (0, 2):
        pix = grid[q]
        col = WHITE if frame == 0 else GOLD_BRIGHT
        pix.color = col
        pix.emis = col


def _tkey(p):
    """A track run's identity: its pixels (a run never moves)."""
    return tuple(sorted(p.px))[:2]


def _find_part(grid, key, tag):
    for q, pix in grid.items():
        if tag in pix.part.tags and _tkey(pix.part) == key:
            return pix.part
    return None


def break_track(grid, plan, wide=False):
    if not plan.track_break:
        return
    key, yb, n = plan.track_break
    part = _find_part(grid, key, 'track')
    if part is None:
        return
    n = n + 2 if wide else n
    x0, y0, x1, y1 = bbox(part.px)
    side = part.pattern.get('side', 'L')
    for (x, y) in list(part.px):
        if yb <= y < yb + n and (x, y) in grid and grid[(x, y)].part is part:
            pix = grid[(x, y)]
            mid = (x0 + x1) // 2 if side == 'L' else (x0 + x1 + 1) // 2
            pix.color = STONE_DK if (x == mid and (y - yb) % 2 == 0) else BLACK
    xo = x0 if side == 'L' else x1
    for y in range(yb, yb + n):
        if (xo, y) in grid and grid[(xo, y)].part is part:
            del grid[(xo, y)]


def lose_track(grid, plan):
    """A whole track run blown away (the cook-off)."""
    if not plan.track_lost:
        return
    part = _find_part(grid, plan.track_lost, 'track')
    if part is None:
        return
    for q in list(part.px):
        if q in grid and grid[q].part is part:
            del grid[q]


def crater(grid, burn, cx, cy, r, hot, frame, s, peel=True):
    for q, pix in list(grid.items()):
        x, y = q
        d = math.hypot(x + 0.5 - cx, y + 0.5 - cy)
        rr = r * (0.8 + 0.4 * smooth(x, y, s + 3, 2.0))
        if d < rr * 0.55:
            pix.color = BLACK
            pix.emis = None
            if hot and smooth(x, y, s + 90, 1.5) > 0.62:
                col = GOLD_BRIGHT if (x + y + frame) % 2 == 0 else RED_BRIGHT
                pix.color = col
                pix.emis = col
        elif d < rr:
            burn.add(q, 4)
        elif d < rr + 2.0:
            burn.add(q, 3)
            if peel and pix.part.mode != 'glow' and _h(x, y, s + 17) > 0.8:
                pix.color = STONE_MID if _h(x, y, s + 18) > 0.5 else STONE_LT


def chip(grid, qs):
    for q in qs:
        if q in grid:
            del grid[q]


def burn_everything(grid, burn, s, level=3, spare=0.0):
    """Every pixel to `level`, except whole parts spared (they keep a
    trace of their paint at level 2)."""
    for q, pix in grid.items():
        if not _damageable(pix):
            continue
        p = pix.part
        # Only small parts keep a trace of paint: a big one left bright on
        # a burnt wreck reads as untouched.
        spared = len(p.px) <= 20 and _h(p.name or '', len(p.px), s) < spare
        burn.add(q, 2 if spared else level)


def apply_pixels(grid, tier, plan, frame=0, wreck=None, s=0, layer='hull'):
    if tier == 0 and wreck is None:
        return grid
    burn = Burn()
    if wreck is None:
        n_sc = {1: 1, 2: 3, 3: 5}[tier]
        top = {1: 2, 2: 3, 3: 3}[tier]
        n_scr = {1: 2, 2: 3, 3: 4}[tier]
        n_dent = {1: 1, 2: 2, 3: 3}[tier]
        n_hole = {1: 0, 2: 1, 3: 2}[tier]
        for i in range(min(n_scr, len(plan.scratch))):
            scratch(grid, plan.scratch[i])
        for i in range(n_dent):
            dent(grid, plan.dents[i])
        for i in range(n_sc):
            cx, cy, r = plan.scorch[i]
            scorch(burn, grid, cx, cy, r * (0.8 if tier == 1 else 1.0 if tier == 2 else 1.2), top, s + i)
        for i in range(n_hole):
            hole(grid, burn, plan.holes[i], hot=(tier >= 3 and i == 0), frame=frame)
        paint_internals(grid, frame, s, fire_on=tier >= 3)
        if tier >= 2:
            spark(grid, frame, s)
        if tier >= 3:
            if layer == 'hull':
                break_track(grid, plan)
            chip(grid, plan.chips[:3])
        burn.apply(grid)
        return grid
    if wreck == 'blown':
        for i in range(5):
            cx, cy, r = plan.scorch[i]
            scorch(burn, grid, cx, cy, r * 1.2, 3, s + i)
        if layer == 'hull':
            crater(grid, burn, 0.0, 0.0, 4.6 if _span(grid) < 26 else 6.0, True, frame, s)
            break_track(grid, plan)
        paint_internals(grid, frame, s, fire_on=False)
        chip(grid, plan.chips[:4])
    elif wreck == 'gutted':
        for i in range(8):
            cx, cy, r = plan.scorch[i]
            scorch(burn, grid, cx, cy, r * 1.4, 4, s + i)
        for i in range(2):
            hole(grid, burn, plan.holes[i], hot=(i == 0), frame=frame, big=(i == 1))
        paint_internals(grid, frame, s, fire_on=True)
        if layer == 'hull':
            break_track(grid, plan)
        chip(grid, plan.chips[:6])
    elif wreck == 'husk':
        burn_everything(grid, burn, s, 3, spare=0.18)
        for i in range(4):
            cx, cy, r = plan.scorch[i]
            scorch(burn, grid, cx, cy, r * 1.1, 4, s + i)
        for i in range(3):
            hole(grid, burn, plan.holes[i], hot=False, big=(i == 0))
        paint_internals(grid, frame, s, fire_on=False, cold=True)
        if layer == 'hull':
            break_track(grid, plan, wide=True)
        chip(grid, plan.chips[:10])
    elif wreck == 'cookoff':
        for i in range(6):
            cx, cy, r = plan.scorch[i]
            scorch(burn, grid, cx, cy, r * 1.3, 4, s + i)
        if layer == 'hull':
            crater(grid, burn, 0.0, 0.5, 6.0 if _span(grid) < 26 else 7.5, True, frame, s, peel=True)
            lose_track(grid, plan)
        paint_internals(grid, frame, s, fire_on=True)
        chip(grid, plan.chips[:8])
    burn.apply(grid)
    return grid


def _span(grid):
    xs = [q[0] for q in grid]
    ys = [q[1] for q in grid]
    return max(max(xs) - min(xs), max(ys) - min(ys))


# ---------------------------------------------------------------------------
# The broken turret
# ---------------------------------------------------------------------------
def break_turret_parts(parts, plan, s=0):
    """Severed barrels, dead optics and lamps, the antenna gone. Barrel
    parts are grouped into guns by touching pixels; each gun is cut once,
    and everything above the cut inside that gun's span goes with it -
    coil bands, rail glows and brakes included - so nothing floats."""
    rng = random.Random(plan.seed + 555)
    barrel_px = set()
    for p in parts:
        if 'barrel' in p.tags:
            barrel_px |= p.px
    # Guns: connected groups of barrel pixels (8-neighbour).
    guns = []
    left = set(barrel_px)
    while left:
        seed_q = min(left)
        comp = {seed_q}
        stack = [seed_q]
        left.discard(seed_q)
        while stack:
            x, y = stack.pop()
            for dx in (-1, 0, 1):
                for dy in (-1, 0, 1):
                    n = (x + dx, y + dy)
                    if n in left:
                        left.discard(n)
                        comp.add(n)
                        stack.append(n)
        guns.append(comp)
    cuts = []
    for comp in sorted(guns, key=lambda c: min(c)):
        x0, y0, x1, y1 = bbox(comp)
        yc = y0 + int((y1 - y0 + 1) * (0.35 + 0.3 * rng.random()))
        cuts.append((x0 - 1, x1 + 1, yc))

    def above_cut(q):
        return any(a <= q[0] <= b and q[1] < yc for (a, b, yc) in cuts)

    out = []
    for p in parts:
        keep = {q for q in p.px if not above_cut(q)}
        if not keep:
            continue
        if 'barrel' in p.tags:
            # A torn end: one pixel of the cut row missing.
            q = p.copy(px=keep)
            top = min(y for (_, y) in keep)
            row = sorted(v for v in keep if v[1] == top)
            if len(row) > 1:
                q.px.discard(row[rng.randrange(len(row))])
            out.append(q)
            continue
        if keep != p.px:
            p = p.copy(px=keep)
        if p.mode == 'glow':
            out.append(_dead_lamp(p))
            continue
        if p.tags & {'antenna', 'stowage', 'fragile'}:
            continue
        if 'optic' in p.tags:
            out.append(p.copy(mat=SMOKED, mode='flat', step=-1))
            continue
        if 'hatch' in p.tags:
            q = p.copy(mat=DARK, mode='flat', step=-1)
            q.tags = set(q.tags) | {'bore'}
            out.append(q)
            continue
        out.append(p)
    return out


def broken_turret_pixels(grid, plan, s):
    burn = Burn()
    burn_everything(grid, burn, s, 3, spare=0.3)
    for i in range(3):
        cx, cy, r = plan.scorch[i]
        scorch(burn, grid, cx, cy, r * 1.2, 4, s + i)
    burn.apply(grid)
    return grid
