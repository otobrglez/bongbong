"""FOUNDRY - the heavy industrial line. Machines built in a shipyard: slab
armour held on with rivets, bolted side skirts over heavy block treads,
hazard-striped bumpers, exhaust stacks, radiator grilles, caged headlamps,
an amber rotating beacon, short thick guns with big muzzle brakes. Warm,
worn and loud: the line that looks like it was welded, not moulded."""

from kit import *

LINE = Line('foundry', 'Foundry', 'Heavy industrial: slabs, rivets, stacks, hazard stripes, a rotating beacon.',
            notes='Tracked, with bolted skirts over heavy block treads. Hazard-striped bumpers, exhaust '
                  'stacks (smoke comes out of them), caged lamps, an amber beacon that turns. Weapon modules '
                  'are welded-on workshop hardware: drum-fed, crate launchers, tank-fed flamers.')

WOOD_M = Mat('crate', [WOOD_DARKEST, WOOD_DEEPER, WOOD_DK, WOOD_AMBER, WOOD_MD, WOOD_LT, WOOD_PALE], 3)


# ---------------------------------------------------------------------------
# The shop's vocabulary
# ---------------------------------------------------------------------------
def hazard(b, px, z, width=2, flip=False, tags=('paint',), shade=True):
    """Hazard stripes painted on a plate: gold and black diagonals, the gold
    falling a step along the plate's lower and right edges so a striped
    bumper still reads as a slab. Symmetric bands are drawn as a chevron
    (the right half mirrors the left)."""
    px = set(px)
    off = bevel_offsets(px, 1, False) if shade else {q: 0 for q in px}
    a, d = set(), set()
    for (x, y) in px:
        xx = x if x < 0 else -1 - x          # mirror the right half: a chevron
        k = ((xx + y) if flip else (xx - y)) // width
        (a if k % 2 == 0 else d).add((x, y))
    b.part(a, HAZARD, 'map', z, stepmap={q: min(0, off[q]) for q in a}, tags=tags)
    b.part(d, DARK, 'flat', z, step=0, tags=tags)


def stack(b, x, y, z=3.6, size=3, name='stack', mat=None):
    """An exhaust stack seen from above: a steel pipe end with a sooty mouth.
    (x, y) is the top-left pixel; returns the mouth centre (for smoke)."""
    mat = mat or STEEL
    if size == 3:
        px = rect(x, y, x + 2, y + 2)
        mouth = {(x + 1, y + 1)}
    else:
        px = chamfer(x, y, x + 3, y + 3, tl=1, tr=1, bl=1, br=1)
        mouth = rect(x + 1, y + 1, x + 2, y + 2)
    b.part(px, mat, 'dome', z, step=0, gain=0.8, tags=('exhaust',), name=name)
    b.part(mouth, DARK, 'flat', z + 0.1, step=0, tags=('exhaust',), contact=False, cast=False)
    return (x + size / 2.0, y + size / 2.0)


def applique(b, px, z, name, step=0, rivets=(), mat=None, normal=None):
    """A bolted applique plate: bevelled, its rivets part of its own
    shading (so they leave with it), tagged armour - the damage pass blows
    these off smallest first and shows the frame behind."""
    px = set(px)
    mat = mat or b.ctx.body
    off = bevel_offsets(px, 1, False)
    if normal is not None:
        base = light_steps(normal[0], normal[1], 1.0)
        off = {q: v + base for q, v in off.items()}
    for q in rivets:
        if q in px:
            off[q] = 2
    return b.part(px, mat, 'map', z, step=step, stepmap=off, name=name, tags=('armor',))


def caged_lamp(b, x, y, kind='head', z=4, dir=0.0, name=None, role='lamp'):
    """A lamp in a steel cage (a bracket round a single lens)."""
    b.lamp({(x, y)}, role, z=z, kind=kind, dir=dir, name=name, housing=STEEL, housing_z=z - 0.3)


def hatch(b, x, y, z=2.6, size=3, mat=None, name='hatch', step=1):
    """A heavy hatch: a raised plate (a 3 x 3 with two corners cut, or a
    rounded square) with a steel handle. (x, y) is the top-left of its box.
    The broken turret opens it (the damage pass turns 'hatch' parts dark)."""
    mat = mat or b.ctx.body
    if size == 3:
        disc = rect(x, y, x + 2, y + 2) - {(x + 2, y), (x, y + 2)}
    else:
        disc = chamfer(x, y, x + size - 1, y + size - 1, tl=1, tr=1, bl=1, br=1)
    b.part(disc, mat, 'plate', z, step=step, name=name, tags=('hatch',), corner=False)
    b.part({(x + 1, y + 1)}, STEEL, 'flat', z + 0.1, step=1, name=name + '_handle', tags=('hatch',), contact=False)


def beacon(b, x, y, z=4.5, name='beacon'):
    """The amber rotating beacon: a 2 x 2 lens whose lit half turns one
    quarter per hull frame. Each quarter is its own lamp with fixed pixels
    (so the damage plan breaks it quarter by quarter), lit on two frames."""
    quads = [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)]
    for k, q in enumerate(quads):
        b.lamp({q}, 'beacon', z=z, blink={k, (k + 1) % 4}, name='%s%d' % (name, k))
    b.light(kind='beacon', role='beacon', x=x + 1.0, y=y + 1.0, dir=0.0, cone=None, reach=None, name=name)


def clearance(b, px, z=3.5, name=None, kind='marker'):
    """Clearance lamps in the chassis accent (the team colour on a player's
    tank): the line's identity at night."""
    return b.lamp(set(px), 'marker', z=z, kind=kind, name=name)


def gun(b, xc2, width, y_tip, y_base, recoil=0, z=2.5, brake=1, side='both', baffles=2, baffle_rows=1, gap=1,
        name=None):
    """A short thick Foundry gun: a light steel tube with a slotted muzzle
    brake (baffles with a port between each), pulled back by `recoil`.
    `side` is where the brake grows: 'both', 'out' (away from the pivot -
    twins keep their air), 'L' or 'R'."""
    parts = b.barrel(xc2, width, y_tip, y_base, mat=STEEL, z=z, brake=0, recoil=recoil, name=name)
    x0 = (xc2 - width) // 2
    x1 = x0 + width - 1
    if side == 'out':
        side = 'L' if xc2 < 0 else 'R'
    bl = brake if side in ('both', 'L') else 0
    br = brake if side in ('both', 'R') else 0
    y = y_tip + recoil
    for k in range(baffles):
        px = rect(x0 - bl, y, x1 + br, y + baffle_rows - 1)
        parts.append(b.part(px, STEEL, 'cylv', z + 0.1, step=0, bevel=0, tags=('barrel', 'brake')))
        y += baffle_rows + gap
    return parts


def mrect(x0, y0, x1, y1):
    """The mirror image of rect(x0, y0, x1, y1) across the pivot line."""
    return rect(-1 - x1, y0, -1 - x0, y1)


TRACK_STEP = -1


def tracks(b, xo, xi, y0, y1, z=1, style='block', period=4, step=None):
    """Both track runs: the left from column xo to xi, the right mirrored.
    Foundry links are dark forged steel (a step below the kit's track)."""
    step = TRACK_STEP if step is None else step
    b.tread(xo, xi, y0, y1, 'L', z=z, style=style, period=period, name='track_l', step=step)
    b.tread(-1 - xi, -1 - xo, y0, y1, 'R', z=z, style=style, period=period, name='track_r', step=step)


def skirts(b, x0, x1, spans, z=3, step=-1, rivets=(), rivet_x=None, name='skirt', tags=('skirt',)):
    """Riveted body-coloured skirt panels over both track runs (each panel
    its own part, so the damage pass tears them one at a time)."""
    body = b.ctx.body
    for k, (y0, y1) in enumerate(spans):
        b.part(rect(x0, y0, x1, y1), body, 'plate', z, step=step, name='%s_l%d' % (name, k), tags=tags)
        b.part(mrect(x0, y0, x1, y1), body, 'plate', z, step=step, name='%s_r%d' % (name, k), tags=tags)
    if rivets:
        rx = (x0 + x1) // 2 if rivet_x is None else rivet_x
        b.rivets(sym({(rx, y) for y in rivets}), body, z + 0.2, step=step)


# ---------------------------------------------------------------------------
# Weapon modules: welded-on workshop hardware
# ---------------------------------------------------------------------------
@LINE.module_fn('minigun')
def minigun(d, b, st, hp):
    """A drum-fed rotary on a welded bracket along the cheek: three barrels
    through a bright clamp, a steel receiver, a round steel ammunition drum
    with a brass hub behind it. The barrels' shading steps round as
    it spins (states 1-3) and the hot barrel's tip glows."""
    hx, hy = hp['minigun']
    inb = -1 if hx >= 0 else 1
    b.part(rect(hx - 1, hy - 1, hx + 1, hy + 1), STEEL, 'plate', 2, step=-1, name='mg_receiver')
    drum = rect(hx - 1, hy + 2, hx + 1, hy + 4) - {(hx - 1, hy + 2), (hx + 1, hy + 2), (hx - 1, hy + 4), (hx + 1, hy + 4)}
    drum |= {(hx - 1, hy + 3), (hx + 1, hy + 3)}
    b.part(drum, STEEL, 'dome', 2.2, gain=0.6, name='mg_drum')
    b.part({(hx, hy + 3)}, BRASS, 'flat', 2.3, step=1, name='mg_drum_hub')
    b.part({(hx + inb, hy + 1)}, BRASS, 'flat', 2.4, step=1, name='mg_chute')
    spin = [2, -1, 0]
    for i, x in enumerate((hx - 1, hx, hx + 1)):
        b.part(rect(x, hy - 6, x, hy - 2), STEEL, 'flat', 3, step=spin[(i + st) % 3], name='mg_barrel')
        if st > 0 and (st - 1) == i:
            b.lamp({(x, hy - 6)}, 'hot', z=3.5)
    b.part(rect(hx - 1, hy - 4, hx + 1, hy - 4), STEEL, 'flat', 3.2, step=2, name='mg_clamp')


@LINE.module_fn('missiles')
def missiles(d, b, st, hp):
    """A crate launcher: four rockets nose-up in a slatted wooden crate with
    steel corners; a fired cell shows its dark, sooty tube."""
    hx, hy = hp['missiles']
    b.part(rect(hx - 3, hy - 2, hx + 2, hy + 2), WOOD_M, 'plate', 5, step=0, name='crate')
    b.part(rect(hx - 3, hy, hx + 2, hy), WOOD_M, 'flat', 5.1, step=-2, name='crate_slat')
    b.part({(hx - 3, hy - 2), (hx + 2, hy - 2), (hx - 3, hy + 2), (hx + 2, hy + 2)}, STEEL, 'flat', 5.2, step=1,
           name='crate_corners')
    for i, (x, y) in enumerate([(hx - 2, hy - 1), (hx, hy - 1), (hx - 2, hy + 1), (hx, hy + 1)]):
        if i >= st:
            b.part({(x, y)}, REDM, 'flat', 5.5, step=1, name='warhead')
            b.part({(x + 1, y)}, REDM, 'flat', 5.5, step=-1, name='warhead')
        else:
            b.part({(x, y), (x + 1, y)}, DARK, 'flat', 5.5, step=0, name='tube')


@LINE.module_fn('laser')
def laser(d, b, st, hp):
    """A coil laser: a steel emitter tube wound with two copper coils; the
    lens glows dull red idle, red charged, white-hot with a flash ahead of
    it when firing. hp is the tube's inner column and its middle row."""
    hx, hy = hp['laser']
    b.part(rect(hx - 1, hy - 3, hx, hy + 1), STEEL, 'cylv', 3, step=0, name='lz_tube')
    for y in (hy - 2, hy):
        b.part(rect(hx - 2, y, hx + 1, y), COPPER, 'cylv', 3.1, step=1 if st == 0 else 2, bevel=0, name='lz_coil')
    role = [(0x9C, 0x35, 0x27), (0xFF, 0x42, 0x1A), 'laser'][st]
    b.lamp(rect(hx - 1, hy - 4, hx, hy - 4), role, z=3.5)
    if st == 2:
        b.lamp(rect(hx - 1, hy - 5, hx, hy - 5), (0xFF, 0x42, 0x1A), z=3.6)


@LINE.module_fn('plasma')
def plasma(d, b, st, hp):
    """Copper accelerator sleeves round each barrel, behind the brake: two
    collars with a glowing gap - dim teal idle, plasma blue charged, white
    when firing. Sleeve width, offset and growth come from the design (a
    twin's sleeves grow outboard so the guns keep their air)."""
    w = getattr(d, 'sleeve_w', 2)
    off = getattr(d, 'sleeve_off', 3)
    for (mx, my) in hp['plasma']:
        x0 = int(round(mx - w / 2.0))
        x1 = x0 + w - 1
        grow = getattr(d, 'sleeve_grow', 'both')
        if grow == 'out':
            grow = 'L' if mx < 0 else 'R'
        gl = 1 if grow in ('both', 'L') else 0
        gr = 1 if grow in ('both', 'R') else 0
        y0 = int(my) + off
        for y in (y0, y0 + 2):
            b.part(rect(x0 - gl, y, x1 + gr, y), COPPER, 'cylv', 7, step=0, bevel=0, name='pl_collar')
        role = [(0x03, 0x8A, 0xAB), 'plasma', 'white'][st]
        b.lamp(rect(x0 - gl, y0 + 1, x1 + gr, y0 + 1), role, z=7.2)


@LINE.module_fn('flame')
def flame(d, b, st, hp):
    """A tank-fed flamer: a red fuel tank strapped across the turret's rear
    corner, a short nozzle forward from its outboard end; a pilot light at
    the tip (states 0-1), a tongue of fire when firing (2-3). hp is the
    nozzle's column and the tank's top row."""
    hx, hy = hp['flame']
    out = -1 if hx < 0 else 1
    tank = rect(hx, hy, hx + 3, hy + 1) if out < 0 else rect(hx - 3, hy, hx, hy + 1)
    b.part(tank, REDM, 'cylh', 3, step=0, name='fl_tank')
    sx = hx + 2 if out < 0 else hx - 2
    b.part({(sx, hy), (sx, hy + 1)}, STEEL, 'flat', 3.1, step=0, name='fl_strap')
    b.part(rect(hx, hy - 3, hx, hy - 1), STEEL, 'cylv', 3.2, step=1, name='fl_nozzle')
    b.part({(hx, hy - 3)}, STEEL, 'flat', 3.3, step=-1, name='fl_tip')
    if st < 2:
        b.lamp({(hx, hy - 4)}, 'hot' if st == 0 else 'fire', z=3.5)
    else:
        b.lamp({(hx, hy - 4), (hx, hy - 5)} | ({(hx - out, hy - 5)} if st == 2 else {(hx, hy - 6)}), 'fire', z=3.5)


# ---------------------------------------------------------------------------
# Assault - twin short guns, a riveted box turret
# ---------------------------------------------------------------------------
# Concept: the yard's general-purpose machine and the line's template. A
# slab hull on heavy block tracks, bare at the four horns, riveted skirts,
# a steel push bar at the nose; the engine end carries the line's signature
# - twin stacks on the rear fenders, grilles, the amber beacon and a
# hazard-striped bumper. A box turret of bolted slabs (mantlet halves,
# cheeks, a roof plate) with two stubby braked guns, a caged searchlight
# and a jerrycan.
# Silhouette: square and even, the twin brakes, stacks at both rear
# corners. Night: caged lamps on the horns, teal clearance lamps and a teal
# status lamp on the turret, red tails, the beacon.
@LINE.design('assault')
class Assault(Design):
    codename = 'Riveter'
    blurb = 'Twin stubby 70 mm with slotted brakes in a riveted box; skirts, stacks, a hazard bumper.'
    marks = 'heavy'
    sleeve_w, sleeve_off, sleeve_grow = 2, 3, 'out'

    def hull(self, b, f):
        body = b.ctx.body
        # Heavy block treads, bare at the four horns.
        tracks(b, -7, -5, -10, 9)
        b.part(rect(-4, -9, 3, 9), body, 'plate', 2, name='deck')
        b.part(rect(-4, -9, 3, -7), body, 'facet', 2.3, normal=(0.0, -0.8), name='glacis')
        applique(b, rect(-4, -9, -2, -7), 2.5, 'glacis_l', normal=(0.0, -0.8), rivets={(-3, -8)})
        applique(b, mrect(-4, -9, -2, -7), 2.5, 'glacis_r', normal=(0.0, -0.8), rivets={(2, -8)})
        b.part(rect(-4, -10, 3, -10), STEEL, 'plate', 2.4, step=-1, name='bumper')
        # Skirts: two riveted panels a side.
        for x0, nm in ((-7, 'l'), (4, 'r')):
            b.part(rect(x0, -7, x0 + 2, -2), body, 'plate', 3, step=-1, name='skirt_%s1' % nm, tags=('skirt',))
            b.part(rect(x0, -1, x0 + 2, 4), body, 'plate', 3, step=-1, name='skirt_%s2' % nm, tags=('skirt',))
            b.rivets({(x0 + 1, -6), (x0 + 1, -3), (x0 + 1, 0), (x0 + 1, 3)}, body, 3.2, step=-1)
        # Twin stacks on the rear fenders, a grille each side of the beacon.
        m1 = stack(b, -7, 5)
        m2 = stack(b, 4, 5)
        b.grille(-4, 5, -2, 7, DARK, 2.5, period=2, dir='v', step=1)
        b.grille(1, 5, 3, 7, DARK, 2.5, period=2, dir='v', step=1)
        beacon(b, -1, 6, z=3.4)
        # The hazard-striped rear bumper between the horns.
        hazard(b, rect(-4, 8, 3, 9), 2.6)
        b.part(ring(0, 0, 4.6, 3.4), STEEL, 'inset', 2.1, step=-2, tags=('ring',))
        caged_lamp(b, -6, -9, name='head_l')
        caged_lamp(b, 5, -9, name='head_r')
        b.lamp({(-6, 9)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(5, 9)}, 'tail', z=3, kind='tail', name='tail_r')
        clearance(b, {(-7, -7), (6, -7)}, name='clear')
        self.emitters = dict(smoke=[m1, m2])

    def turret(self, b, pose):
        body = b.ctx.body
        for i, xc2 in enumerate((-6, 6)):
            gun(b, xc2, 2, -11, -5, recoil=self.recoil(pose, i, 2), side='out')
        L = dict(
            M=dict(mat=body, mode='facet', z=3, normal=(0.0, -0.7), name='mantlet'),
            R=dict(mat=body, mode='plate', z=2.2, name='roof'),
            B=dict(mat=body, mode='facet', z=2.2, normal=(0.0, 0.7), name='rear'),
        )
        b.paint([
            '.MMMM',
            'RMMMM',
            'RRRRR',
            'RRRRR',
            'RRRRR',
            'RRRRR',
            'RRRRR',
            'RRRRR',
            '.BBBB',
        ], -5, -5, L, mirror_x=True)
        # Bolted slabs: the mantlet halves, the cheeks, a roof plate.
        applique(b, rect(-4, -5, -1, -4), 3.1, 'mantlet_l', normal=(0.0, -0.7), rivets={(-2, -4)})
        applique(b, mrect(-4, -5, -1, -4), 3.1, 'mantlet_r', normal=(0.0, -0.7), rivets={(1, -4)})
        applique(b, rect(-5, -4, -5, 2), 2.6, 'cheek_l', step=-1, rivets={(-5, -3), (-5, 1)})
        applique(b, mrect(-5, -4, -5, 2), 2.6, 'cheek_r', step=-1, rivets={(4, -3), (4, 1)})
        applique(b, rect(1, 1, 3, 2), 2.4, 'roof_plate', rivets={(3, 1)})
        b.part({(-4, -5), (-3, -5), (2, -5), (3, -5)}, STEEL, 'plate', 3.2, step=-1, name='collars')
        hatch(b, -3, -2, z=2.6)
        b.glass(rect(1, -2, 2, -2), 2.7, step=1)
        b.lamp({(-1, -5), (0, -5)}, 'sensor', z=3.6, kind='sensor', name='eye')
        caged_lamp(b, -6, -2, kind='spot', z=3.4, name='searchlight')
        b.lamp({(3, 3)}, 'marker', z=2.9, name='status')
        b.part(rect(-4, 4, -2, 5), REDM, 'cylh', 2.2, step=-1, name='jerrycan', tags=('stowage',))
        b.meta['muzzles'] = [(-3.0, -11), (3.0, -11)]

    def hardpoints(self, ctx):
        return dict(minigun=(5, -2), laser=(-6, -1), missiles=(1, 1), flame=(-4, 3),
                    plasma=[(-3.0, -11), (3.0, -11)])


# ---------------------------------------------------------------------------
# Scout - a light tracked runabout
# ---------------------------------------------------------------------------
# Concept: the yard's errand runner, built like a rally buggy. A tube bull
# bar and a roll bar painted in the accent (a rally cage - the team colour
# on a player's tank), lamps in steel cups on the bar, narrow tracks
# rolling down both sides, an open engine bay at the rear (a block, two
# heads, a copper pipe to one stack), and a small drum turret with a
# braked pop-gun, a whip antenna and a big caged recon searchlight.
# Silhouette: the narrowest Foundry hull, the bar across the nose, the
# knobbly engine end. Night: the recon searchlight, two bar lamps, red
# clearance lamps on the mudguards, the beacon on the roll bar.
@LINE.design('scout')
class Scout(Design):
    codename = 'Ratchet'
    blurb = 'A rally-caged errand runner: a painted bull bar and roll bar, an open engine bay, a recon searchlight.'
    marks = 'heavy'
    sleeve_w, sleeve_off, sleeve_grow = 2, 3, 'both'

    def hull(self, b, f):
        body = b.ctx.body
        acc = b.ctx.accent
        tracks(b, -6, -5, -7, 8)
        L = dict(
            B=dict(mat=acc, mode='cylh', z=3.4, step=0, name='bullbar'),
            G=dict(mat=body, mode='plate', z=3.0, step=-1, name='guard_f', tags=('skirt',)),
            H=dict(mat=body, mode='plate', z=3.0, step=-1, name='guard_r', tags=('skirt',)),
            N=dict(mat=body, mode='facet', z=2.3, normal=(0.0, -0.8), name='nose'),
            D=dict(mat=body, mode='plate', z=2.0, name='tub'),
            C=dict(mat=body, mode='plate', z=3.3, step=1, name='cowl'),
            R=dict(mat=body, mode='plate', z=2.4, step=-1, name='tail_plate'),
        )
        b.paint([
            '.BBBBB',
            'GG.B..',
            'GGNNNN',
            'GG.DDD',
            '...DDD',
            '...DDD',
            '...DDD',
            '...DDD',
            '...DDD',
            '...DDD',
            '...DDD',
            '..DDDD',
            'HHDDDD',
            'HHC...',
            'HHC...',
            'HHC...',
            'HHRRRR',
        ], -6, -8, L, mirror_x=True)
        b.part(rect(-5, -5, -4, 3) | mrect(-5, -5, -4, 3), body, 'plate', 2.2, step=-1, name='tub_lips')
        b.rivets({(-5, -3), (-5, 1), (4, -3), (4, 1)}, body, 2.3, step=-1)
        applique(b, rect(-4, -6, -2, -5), 2.5, 'nose_l', normal=(0.0, -0.8), rivets={(-3, -5)})
        applique(b, mrect(-4, -6, -2, -5), 2.5, 'nose_r', normal=(0.0, -0.8), rivets={(2, -5)})
        # The open engine bay between the cowls: a block with two heads, a
        # copper pipe to one tall stack.
        b.part(rect(-3, 5, 2, 7), DARK, 'plate', 2.2, step=1, name='bay')
        b.part(rect(-2, 5, 1, 6), STEEL, 'plate', 2.6, step=0, name='block')
        b.part({(-2, 5), (1, 5)}, STEEL, 'flat', 2.7, step=2, name='heads')
        b.part(rect(-2, 7, 2, 7), COPPER, 'cylh', 2.6, step=1, name='pipe')
        m = stack(b, 3, 5, z=3.6)
        # The roll bar behind the turret, the beacon on its end.
        b.part(rect(-5, 4, 4, 4), acc, 'cylh', 3.5, step=0, name='rollbar')
        beacon(b, -5, 3, z=3.7)
        # Lamps in steel cups on the bull bar.
        b.part({(-5, -8), (-3, -8), (2, -8), (4, -8)}, STEEL, 'plate', 3.5, step=0, name='lamp_cups')
        b.lamp({(-4, -8)}, 'lamp', z=3.6, kind='head', name='head_l')
        b.lamp({(3, -8)}, 'lamp', z=3.6, kind='head', name='head_r')
        b.lamp({(-4, 8)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(3, 8)}, 'tail', z=3, kind='tail', name='tail_r')
        clearance(b, {(-6, -6), (5, -6)}, name='clear')
        b.part(ring(0, 0, 4.4, 3.4), STEEL, 'inset', 2.1, step=-2, tags=('ring',))
        self.emitters = dict(smoke=[m])

    def turret(self, b, pose):
        body = b.ctx.body
        gun(b, 0, 2, -13, -4, recoil=self.recoil(pose), brake=1, side='both')
        b.part(circle(0, 0, 3.9), body, 'dome', 2.2, gain=0.45, name='drum')
        applique(b, rect(-2, -4, -1, -3), 2.6, 'mantlet_l', normal=(0.0, -0.7))
        applique(b, mrect(-2, -4, -1, -3), 2.6, 'mantlet_r', normal=(0.0, -0.7))
        applique(b, rect(-4, -1, -3, 1), 2.4, 'cheek_l', step=-1, rivets={(-4, 0)})
        applique(b, mrect(-4, -1, -3, 1), 2.4, 'cheek_r', step=-1, rivets={(3, 0)})
        b.part({(-1, -4), (0, -4)}, STEEL, 'plate', 2.7, step=0, name='collar')
        hatch(b, -2, 0, z=2.6)
        b.lamp({(-1, -3), (0, -3)}, 'sensor', z=2.9, kind='sensor', name='eye')
        # The recon searchlight: a big lamp in a steel cage on the roof.
        b.part(rect(0, -2, 3, 1) - {(0, -2), (3, 1)}, STEEL, 'plate', 3.0, step=-1, name='light_cage')
        b.lamp(rect(1, -1, 2, 0), 'lamp', z=3.2, kind='spot', name='searchlight')
        b.part({(1, -2), (2, 1)}, STEEL, 'flat', 3.3, step=2, name='cage_bars')
        b.antenna(-3, 2, 5, z=3.2)
        b.meta['muzzles'] = [(0.0, -13)]

    def hardpoints(self, ctx):
        return dict(minigun=(4, -1), laser=(-5, 0), missiles=(1, 2), flame=(-3, 2), plasma=[(0.0, -13)])


# ---------------------------------------------------------------------------
# Breaker - the dozer
# ---------------------------------------------------------------------------
# Concept: a ram with a gun on it. A giant dozer blade spans the whole
# nose - a bracket of steel with a chevron-striped face - on two painted
# push arms; wide block tracks under riveted skirts; two big stacks round
# the beacon at the back; a squat hex turret of bolted slabs with one fat
# 4 px stub gun and a six-wide brake.
# Silhouette: the blade - a striped bar across the whole nose, its steel
# ends swept back. Night: caged lamps behind the blade, gold lamps at the
# blade's ends and along its back, the beacon between the stacks.
@LINE.design('breaker')
class Breaker(Design):
    codename = 'Sledge'
    blurb = 'A dozer with a gun: a hazard-striped blade across the whole nose, push arms, two big stacks, one fat stub gun.'
    marks = 'heavy'
    sleeve_w, sleeve_off, sleeve_grow = 4, 3, 'both'

    def hull(self, b, f):
        body = b.ctx.body
        tracks(b, -8, -6, -7, 10)
        b.part(rect(-5, -7, 4, 10), body, 'plate', 2, name='deck')
        b.part(rect(-5, -7, 4, -5), body, 'facet', 2.3, normal=(0.0, -0.8), name='glacis')
        applique(b, rect(-5, -7, -3, -6), 2.5, 'glacis_l', normal=(0.0, -0.8), rivets={(-4, -6)})
        applique(b, mrect(-5, -7, -3, -6), 2.5, 'glacis_r', normal=(0.0, -0.8), rivets={(3, -6)})
        skirts(b, -8, -6, [(-4, 0), (1, 5)], rivets=(-3, 0, 2, 4))
        applique(b, rect(-5, 3, -4, 5), 2.4, 'deck_l', rivets={(-5, 4)})
        applique(b, mrect(-5, 3, -4, 5), 2.4, 'deck_r', rivets={(4, 4)})
        # The blade: a bracket-shaped slab across the whole nose, its face
        # striped, the end bits and the back edge bare steel.
        hazard(b, rect(-7, -11, 6, -9), 3.6)
        b.part(rect(-8, -11, -8, -8) | mrect(-8, -11, -8, -8), STEEL, 'plate', 3.7, step=0, name='blade_ends')
        b.part(rect(-7, -8, 6, -8), STEEL, 'plate', 3.5, step=-2, name='blade_back')
        # Push arms: painted beams riding on the track horns.
        for nm, px in (('arm_l', rect(-7, -7, -7, -4)), ('arm_r', mrect(-7, -7, -7, -4))):
            b.part(px, body, 'plate', 3.3, step=1, name=nm)
        b.rivets({(-7, -6), (6, -6)}, body, 3.5, step=0)
        # The engine deck: two big stacks round the beacon, the rear bar.
        m1 = stack(b, -5, 6, size=4)
        m2 = stack(b, 1, 6, size=4)
        beacon(b, -1, 7, z=3.4)
        b.part(rect(-5, 10, 4, 10), STEEL, 'plate', 2.4, step=-1, name='rear_bar')
        caged_lamp(b, -4, -6, name='head_l', z=3.9)
        caged_lamp(b, 3, -6, name='head_r', z=3.9)
        clearance(b, {(-8, -11), (7, -11)}, name='clear')
        clearance(b, {(-5, -8), (4, -8)}, name='blade_lamps', kind=None)
        b.lamp({(-7, 10)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(6, 10)}, 'tail', z=3, kind='tail', name='tail_r')
        b.part(ring(0, 0, 5.0, 3.8), STEEL, 'inset', 2.1, step=-2, tags=('ring',))
        self.emitters = dict(smoke=[m1, m2])

    def turret(self, b, pose):
        body = b.ctx.body
        gun(b, 0, 4, -12, -6, recoil=self.recoil(pose), brake=1, side='both')
        L = dict(
            M=dict(mat=body, mode='facet', z=3.1, normal=(0.0, -0.8), name='shield'),
            F=dict(mat=body, mode='facet', z=2.8, normal=(-0.6, -0.6), name='corner'),
            S=dict(mat=body, mode='plate', z=2.7, step=-1, name='side'),
            R=dict(mat=body, mode='plate', z=2.6, name='roof'),
            B=dict(mat=body, mode='facet', z=2.6, normal=(0.0, 0.4), name='back'),
        )
        b.paint([
            '...MMM',
            '..FMMM',
            '.FRRRR',
            'SRRRRR',
            'SRRRRR',
            'SRRRRR',
            'SRRRRR',
            '.SRRRR',
            '..BBBB',
        ], -6, -6, L, mirror_x=True)
        applique(b, rect(-3, -6, -1, -5), 3.2, 'shield_l', normal=(0.0, -0.8), rivets={(-3, -5)})
        applique(b, mrect(-3, -6, -1, -5), 3.2, 'shield_r', normal=(0.0, -0.8), rivets={(2, -5)})
        applique(b, rect(-6, -3, -6, 0), 2.8, 'side_l', step=-1, rivets={(-6, -2)})
        applique(b, mrect(-6, -3, -6, 0), 2.8, 'side_r', step=-1, rivets={(5, -2)})
        b.part({(-2, -6), (1, -6)}, STEEL, 'plate', 3.3, step=-1, name='trunnions')
        b.rivets({(-6, 1), (5, 1)}, body, 3.3)
        hatch(b, -4, -2, z=2.8)
        b.glass(rect(2, -3, 3, -3), 2.9, step=1)
        b.lamp({(-1, -5), (0, -5)}, 'sensor', z=3.4, kind='sensor', name='eye')
        b.part(rect(1, 3, 3, 4), WOOD_M, 'plate', 2.7, step=0, name='crate', tags=('stowage',))
        b.part({(2, 3), (2, 4)}, WOOD_M, 'flat', 2.8, step=-2, tags=('stowage',))
        b.meta['muzzles'] = [(0.0, -12)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, -3), laser=(-7, -2), missiles=(1, 0), flame=(-5, 1), plasma=[(0.0, -12)])


# ---------------------------------------------------------------------------
# Flak - twin short guns fed from drums, a radar that turns
# ---------------------------------------------------------------------------
# Concept: a squat box of a hull with painted corner caps and a radar dish
# on a mast at the back that sweeps a quarter turn per track frame; a hex
# turret between two ammunition drums (steel rims, brass faces), brass
# belts feeding two short braked guns.
# Silhouette: the widest turret for its size (the drums), the dish behind.
# Night: lamps in the corner caps, sand clearance lamps, a lamp on the
# radar's feed, the beacon.
@LINE.design('flak')
class Flak(Design):
    codename = 'Hopper'
    blurb = 'A squat flak box: two stub guns belt-fed from brass drums, and a radar dish that sweeps as it drives.'
    marks = 'heavy'
    sleeve_w, sleeve_off, sleeve_grow = 2, 3, 'out'

    def hull(self, b, f):
        body = b.ctx.body
        tracks(b, -7, -5, -8, 7)
        b.part(rect(-4, -8, 3, 7), body, 'plate', 2, name='deck')
        b.part(rect(-4, -8, 3, -6), body, 'facet', 2.3, normal=(0.0, -0.8), name='glacis')
        applique(b, rect(-4, -7, -2, -6), 2.5, 'glacis_l', normal=(0.0, -0.8), rivets={(-3, -6)})
        applique(b, mrect(-4, -7, -2, -6), 2.5, 'glacis_r', normal=(0.0, -0.8), rivets={(2, -6)})
        skirts(b, -7, -5, [(-5, -1), (0, 4)], rivets=(-3, 2))
        # Square painted corner caps over the track horns.
        for nm, px in (('cap_l', rect(-7, -8, -5, -7)), ('cap_r', mrect(-7, -8, -5, -7))):
            b.part(px, body, 'plate', 3.1, step=1, name=nm, tags=('armor',))
        # The radar: a dish on a mast that turns a quarter per hull frame.
        b.part(rect(-1, 5, 0, 6), STEEL, 'plate', 2.6, step=-1, name='mast')
        base = ellipse(0.0, 5.5, 3.4, 1.2)
        dish = rotate_px(base, [0, 45, 90, 135][f % 4], 0.0, 5.5)
        b.part(dish, PALE, 'plate', 3.6, step=-1, name='dish', tags=('fragile',))
        b.lamp({(-1, 5)}, 'marker', z=3.7, name='radar_lamp')
        m = stack(b, 3, 4)
        beacon(b, -6, 5, z=3.4)
        b.lamp({(-6, -8)}, 'lamp', z=3.4, kind='head', name='head_l')
        b.lamp({(5, -8)}, 'lamp', z=3.4, kind='head', name='head_r')
        clearance(b, {(-7, -2), (6, -2)}, name='clear')
        b.lamp({(-6, 7)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(5, 7)}, 'tail', z=3, kind='tail', name='tail_r')
        b.part(ring(0, 0, 4.6, 3.4), STEEL, 'inset', 2.1, step=-2, tags=('ring',))
        self.emitters = dict(smoke=[m])

    def turret(self, b, pose):
        body = b.ctx.body
        for i, xc2 in enumerate((-6, 6)):
            gun(b, xc2, 2, -9, -4, recoil=self.recoil(pose, i, 2), side='out')
        b.part(chamfer(-4, -4, 3, 3, tl=2, tr=2, bl=2, br=2), body, 'plate', 2.4, name='hex')
        applique(b, rect(-2, -4, -1, -3), 2.6, 'brow_l', normal=(0.0, -0.8))
        applique(b, mrect(-2, -4, -1, -3), 2.6, 'brow_r', normal=(0.0, -0.8))
        applique(b, rect(-2, 2, 1, 3), 2.6, 'bustle', normal=(0.0, 0.4), rivets={(-2, 2), (1, 2)})
        # Ammunition drums: steel rims, brass faces, a hub; belts to the guns.
        for cx, nm in ((-5.0, 'drum_l'), (5.0, 'drum_r')):
            d = circle(cx, 0.5, 2.2)
            b.part(d, STEEL, 'plate', 2.8, step=-1, name=nm + '_rim')
            b.part(shrink(d, 1), BRASS, 'dome', 2.9, gain=0.6, name=nm)
            b.part({(int(math.floor(cx)), 0)}, DARK, 'flat', 3.0, step=1, name=nm + '_hub')
        b.part({(-4, -3), (-4, -2), (3, -3), (3, -2)}, BRASS, 'flat', 2.9, step=1, name='belts')
        hatch(b, -2, -1, z=2.7)
        b.lamp({(-1, -4), (0, -4)}, 'sensor', z=2.9, kind='sensor', name='eye')
        b.meta['muzzles'] = [(-3.0, -9), (3.0, -9)]

    def hardpoints(self, ctx):
        return dict(minigun=(5, -1), laser=(-6, 0), missiles=(0, 2), flame=(-3, 3), plasma=[(-3.0, -9), (3.0, -9)])


# ---------------------------------------------------------------------------
# Longbow - a long gun on a crane's chassis
# ---------------------------------------------------------------------------
# Concept: the yard's long-reach piece. A long hull with a travel lock at the
# nose (an A-frame cradle the barrel rests in when stowed), a folded
# stabiliser spade across the tail with a toothed edge and striped ends, a
# boiler-drum turret with a rangefinder bar through it, and one long gun
# with a massive triple-baffle brake.
# Silhouette: the longest gun of the line, the rangefinder's ears, the
# spade's toothed tail. Night: caged lamps on the front horns, red
# clearance lamps on the spade ends, the beacon behind the turret.
@LINE.design('longbow')
class Longbow(Design):
    codename = 'Derrick'
    blurb = 'A long-reach gun: a triple-baffle brake, a rangefinder through a drum turret, a travel lock and a folded spade.'
    marks = 'heavy'
    sleeve_w, sleeve_off, sleeve_grow = 2, 5, 'both'

    def hull(self, b, f):
        body = b.ctx.body
        tracks(b, -7, -5, -11, 9)
        b.part(rect(-4, -11, 3, 9), body, 'plate', 2, name='deck')
        b.part(rect(-4, -11, 3, -8), body, 'facet', 2.3, normal=(0.0, -0.8), name='glacis')
        applique(b, rect(-4, -11, -3, -9), 2.5, 'glacis_l', normal=(0.0, -0.8), rivets={(-4, -10)})
        applique(b, mrect(-4, -11, -3, -9), 2.5, 'glacis_r', normal=(0.0, -0.8), rivets={(3, -10)})
        skirts(b, -7, -5, [(-8, -3), (-2, 3), (4, 8)], rivets=(-7, -1, 5))
        applique(b, rect(-4, 5, -3, 7), 2.4, 'deck_l', rivets={(-4, 6)})
        # The travel lock: an A-frame on the glacis, a clamp at its apex.
        b.part(line(-3, -8, -2, -11) | line(2, -8, 1, -11), STEEL, 'plate', 3.2, step=0, name='lock')
        b.part(rect(-2, -12, 1, -12), STEEL, 'plate', 3.3, step=1, name='clamp')
        b.part({(-1, -12), (0, -12)}, HAZARD, 'flat', 3.4, step=0, name='latch')
        # The folded spade: a steel plate across the tail, toothed, striped ends.
        b.part(rect(-6, 9, 5, 10), STEEL, 'plate', 3.0, step=0, name='spade')
        b.part({(x, 11) for x in range(-6, 6) if x % 2 == 0}, STEEL, 'plate', 3.0, step=-1, name='teeth')
        hazard(b, rect(-6, 9, -4, 10) | mrect(-6, 9, -4, 10), 3.1)
        b.part(rect(-2, 7, -2, 8) | rect(1, 7, 1, 8), STEEL, 'cylv', 3.1, step=1, name='rams')
        m = stack(b, -4, 4)
        beacon(b, 1, 5, z=3.4)
        caged_lamp(b, -6, -10, name='head_l')
        caged_lamp(b, 5, -10, name='head_r')
        clearance(b, {(-7, 9), (6, 9)}, name='clear')
        b.lamp({(-3, 9)}, 'tail', z=3.3, kind='tail', name='tail_l')
        b.lamp({(2, 9)}, 'tail', z=3.3, kind='tail', name='tail_r')
        b.part(ring(0, 0, 4.8, 3.6), STEEL, 'inset', 2.1, step=-2, tags=('ring',))
        self.emitters = dict(smoke=[m])

    def turret(self, b, pose):
        body = b.ctx.body
        gun(b, 0, 2, -17, -5, recoil=self.recoil(pose), brake=2, side='both', baffles=3)
        b.part(circle(0, 0, 4.6), body, 'dome', 2.2, gain=0.45, name='drum')
        b.part(ring(0, 0, 4.6, 3.6), body, 'plate', 2.3, step=-1, name='band')
        applique(b, rect(-2, -5, -1, -3), 2.6, 'mantlet_l', normal=(0.0, -0.7), rivets={(-2, -4)})
        applique(b, mrect(-2, -5, -1, -3), 2.6, 'mantlet_r', normal=(0.0, -0.7), rivets={(1, -4)})
        applique(b, rect(-4, 1, -3, 2), 2.5, 'cheek_l', step=-1)
        applique(b, mrect(-4, 1, -3, 2), 2.5, 'cheek_r', step=-1)
        # The rangefinder: a bar through the drum, lenses at its ends.
        b.part(rect(-6, -2, 5, -2), STEEL, 'cylh', 2.8, step=0, name='rangefinder')
        b.glass({(-6, -2)}, 2.9, step=1)
        b.glass({(5, -2)}, 2.9, step=1)
        hatch(b, -1, 0, z=2.6)
        b.lamp({(-1, -3), (0, -3)}, 'sensor', z=2.9, kind='sensor', name='eye')
        b.meta['muzzles'] = [(0.0, -17)]

    def hardpoints(self, ctx):
        return dict(minigun=(5, 1), laser=(-6, 2), missiles=(0, 1), flame=(-4, 4), plasma=[(0.0, -17)])


# ---------------------------------------------------------------------------
# Obelisk - twin siege guns on a gantry
# ---------------------------------------------------------------------------
# Concept: a siege rig. Two long guns joined by a welded cross-brace (an H
# - the one deliberate bridge between twin barrels in the line), a sloped
# wedge turret, outrigger jacks with round steel feet folded at the four
# corners, a stack on each rear fender and a gantry crane straddling the
# engine deck with a hook block hanging in it.
# Silhouette: the H of the braced guns, the round corner feet, the
# gantry's frame at the back. Night: caged lamps on the glacis, gold
# clearance lamps at the gantry's feet, the beacon in front of its beam.
@LINE.design('obelisk')
class Obelisk(Design):
    codename = 'Gantry'
    blurb = 'A siege rig: two braced long guns, a wedge turret, outrigger jacks at the corners and a gantry crane.'
    marks = 'heavy'
    sleeve_w, sleeve_off, sleeve_grow = 2, 3, 'out'

    def hull(self, b, f):
        body = b.ctx.body
        tracks(b, -7, -5, -10, 9)
        b.part(rect(-4, -11, 3, 10), body, 'plate', 2, name='deck')
        b.part(rect(-4, -11, 3, -8), body, 'facet', 2.3, normal=(0.0, -0.8), name='glacis')
        applique(b, rect(-4, -11, -3, -9), 2.5, 'glacis_l', normal=(0.0, -0.8), rivets={(-4, -10)})
        applique(b, mrect(-4, -11, -3, -9), 2.5, 'glacis_r', normal=(0.0, -0.8), rivets={(3, -10)})
        skirts(b, -7, -5, [(-7, -1), (0, 6)], rivets=(-5, -2, 2, 5))
        applique(b, rect(-3, 3, -2, 4), 2.4, 'deck_l')
        applique(b, mrect(-3, 3, -2, 4), 2.4, 'deck_r')
        # Outrigger jacks folded at the four corners: round steel feet,
        # a bolt in each.
        for (y0, nm) in ((-12, 'f'), (9, 'r')):
            for side, px in ((-1, rect(-7, y0, -5, y0 + 2)), (1, mrect(-7, y0, -5, y0 + 2))):
                b.part(px - ({(-7, y0), (-7, y0 + 2)} if side < 0 else {(6, y0), (6, y0 + 2)}), STEEL, 'dome', 3.4,
                       gain=0.6, name='foot_%s%d' % (nm, side), tags=('pad',))
                cx = -6 if side < 0 else 5
                b.part({(cx, y0 + 1)}, DARK, 'flat', 3.5, step=1, name='foot_bolt')
        # The gantry: two posts and a beam straddling the engine deck, a
        # hook block hanging in it; a grille under the hook.
        b.grille(-2, 6, 1, 9, DARK, 2.5, period=2, step=1)
        b.part(rect(-4, 5, -4, 9) | mrect(-4, 5, -4, 9), body, 'plate', 3.6, step=1, name='posts')
        b.part(rect(-4, 5, 3, 5), body, 'plate', 3.7, step=1, name='beam')
        b.part(rect(-1, 7, 0, 8), STEEL, 'plate', 3.2, step=1, name='hook')
        m1 = stack(b, -7, 6)
        m2 = stack(b, 4, 6)
        beacon(b, -1, 3, z=3.9)
        caged_lamp(b, -3, -10, name='head_l', z=3.8)
        caged_lamp(b, 2, -10, name='head_r', z=3.8)
        clearance(b, {(-4, 9), (3, 9)}, name='clear')
        b.lamp({(-4, 10)}, 'tail', z=3.8, kind='tail', name='tail_l')
        b.lamp({(3, 10)}, 'tail', z=3.8, kind='tail', name='tail_r')
        b.part(ring(0, 0, 4.6, 3.4), STEEL, 'inset', 2.1, step=-2, tags=('ring',))
        self.emitters = dict(smoke=[m1, m2])

    def turret(self, b, pose):
        body = b.ctx.body
        for i, xc2 in enumerate((-6, 6)):
            gun(b, xc2, 2, -17, -6, recoil=self.recoil(pose, i, 2), side='out', baffles=2)
        # The cross-brace welded between the barrels.
        b.part(rect(-2, -11, 1, -11), STEEL, 'plate', 2.4, step=-1, name='brace', tags=('barrel',))
        L = dict(
            W=dict(mat=body, mode='facet', z=2.8, normal=(0.0, -0.9), name='wedge'),
            S=dict(mat=body, mode='facet', z=2.6, normal=(-0.8, -0.3), name='flank'),
            R=dict(mat=body, mode='plate', z=2.6, name='roof'),
            B=dict(mat=body, mode='facet', z=2.6, normal=(0.0, 0.8), name='back'),
        )
        b.paint([
            '...WW',
            '..WWW',
            '.SWWW',
            'SSRRR',
            'SSRRR',
            'SSRRR',
            'SSRRR',
            '.BBBB',
        ], -5, -6, L, mirror_x=True)
        applique(b, rect(-5, -3, -4, 0), 2.7, 'flank_l', normal=(-0.8, -0.3), rivets={(-4, -2)})
        applique(b, mrect(-5, -3, -4, 0), 2.7, 'flank_r', normal=(0.8, -0.3), rivets={(3, -2)})
        applique(b, rect(-2, -4, -1, -3), 2.9, 'wedge_l', normal=(0.0, -0.9))
        applique(b, mrect(-2, -4, -1, -3), 2.9, 'wedge_r', normal=(0.0, -0.9))
        hatch(b, -2, -1, z=2.8)
        b.lamp({(-1, -6), (0, -6)}, 'sensor', z=3.0, kind='sensor', name='eye')
        b.meta['muzzles'] = [(-3.0, -17), (3.0, -17)]

    def hardpoints(self, ctx):
        return dict(minigun=(5, -2), laser=(-6, -1), missiles=(0, 0), flame=(-4, 1), plasma=[(-3.0, -17), (3.0, -17)])


# ---------------------------------------------------------------------------
# Wraith - industrial stealth
# ---------------------------------------------------------------------------
# Concept: a night-shift prowler. Slat (bar) armour cages standing off
# both sides in body colour, a knotted camouflage net thrown over the
# engine deck and the turret bustle, a baffled muffler across the tail
# instead of stacks, blackout lamps (hooded slits), a faceted stealth
# wedge turret with smoke-discharger racks, and a darkened gun wearing a
# fat suppressor can instead of a brake.
# Silhouette: the ribbed (comb) sides, the net spilling over one cage, the
# can on the gun. Night: almost dark - two slit lamps, pale-blue
# clearance slits, the sensor, the beacon on the tail.

@LINE.design('wraith')
class Wraith(Design):
    codename = 'Smog'
    blurb = 'A night-shift prowler: slat armour cages, a net over the deck, a baffled muffler, a suppressed gun, hooded lamps.'
    marks = 'heavy'
    sleeve_w, sleeve_off, sleeve_grow = 2, 4, 'both'

    def hull(self, b, f):
        body = b.ctx.body
        tracks(b, -5, -4, -7, 8)
        b.part(rect(-3, -8, 2, 8), body, 'plate', 2, name='deck')
        b.part(rect(-3, -8, 2, -6), body, 'facet', 2.3, normal=(0.0, -0.8), name='glacis')
        applique(b, rect(-3, -7, -2, -6), 2.5, 'glacis_l', normal=(0.0, -0.8))
        applique(b, mrect(-3, -7, -2, -6), 2.5, 'glacis_r', normal=(0.0, -0.8))
        # Slat cages standing off each side: a rail and a comb of bars, the
        # gaps open to the ground (the silhouette itself is ribbed).
        for nm, px in (('rail_l', rect(-5, -6, -5, 6)), ('rail_r', mrect(-5, -6, -5, 6))):
            b.part(px, body, 'plate', 3.2, step=-1, name=nm, tags=('skirt',))
        bars = {(-6, y) for y in range(-6, 7, 2)}
        b.part(bars, body, 'plate', 3.3, step=1, name='slats_l', bevel=0)
        b.part(mirror(bars), body, 'plate', 3.3, step=0, name='slats_r', bevel=0)
        # The baffled muffler across the tail (no stack: it breathes out low).
        b.part(rect(-3, 7, 2, 8), GUNMETAL, 'cylh', 2.8, step=0, name='muffler', tags=('exhaust',))
        b.part(rect(-2, 7, -2, 8) | rect(1, 7, 1, 8), GUNMETAL, 'cylh', 2.9, step=-1, name='baffles', tags=('exhaust',))
        b.part({(3, 8)}, DARK, 'flat', 2.9, step=0, name='outlet', tags=('exhaust',))
        # The camouflage net thrown over the engine deck: a draped, knotted
        # blanket that spills over the right-hand cage.
        net = rect(-3, 3, 2, 6) | {(3, 4), (3, 5), (4, 5), (-4, 4)}
        mesh = bevel_offsets(net, 1, False)
        for (x, y) in net:
            mesh[(x, y)] += 0 if ((x // 2 + y // 2) % 2) == 0 else -1
        b.part(net, body, 'map', 3.0, stepmap=mesh, name='net', tags=('stowage',))
        b.lamp({(-3, -8)}, 'lamp', z=3, kind='head', name='slit_l')
        b.lamp({(2, -8)}, 'lamp', z=3, kind='head', name='slit_r')
        b.part({(-4, -8), (3, -8)}, DARK, 'flat', 3, step=1, name='hoods')
        clearance(b, {(-6, -5), (5, -5)}, name='clear')
        beacon(b, -6, 7, z=3.4)
        b.lamp({(-3, 6)}, 'tail', z=3.2, kind='tail', name='tail_l')
        b.lamp({(2, 6)}, 'tail', z=3.2, kind='tail', name='tail_r')
        b.part(ring(0, 0, 4.2, 3.2), STEEL, 'inset', 2.1, step=-2, tags=('ring',))
        self.emitters = dict(smoke=[(3.5, 8.5)])

    def turret(self, b, pose):
        body = b.ctx.body
        r = self.recoil(pose)
        b.barrel(0, 2, -13, -4, mat=GUNMETAL, z=2.5, recoil=r, step=1)
        b.part(rect(-2, -13 + r, 1, -10 + r), GUNMETAL, 'cylv', 2.7, step=0, bevel=0, tags=('barrel', 'brake'), name='can')
        b.part(rect(-2, -11 + r, 1, -11 + r), GUNMETAL, 'cylv', 2.75, step=-1, bevel=0, tags=('barrel', 'brake'))
        b.part(rect(-1, -13 + r, 0, -13 + r), DARK, 'flat', 2.8, step=0, tags=('barrel', 'bore'))
        # A faceted wedge: every face its own slope, so the lit left half
        # meets the shadowed right along a ridge.
        L = dict(
            A=dict(mat=body, mode='facet', z=2.8, normal=(-0.35, -0.9), name='nose', bevel=0),
            B=dict(mat=body, mode='facet', z=2.7, normal=(-0.9, -0.3), name='cheek', bevel=0),
            C=dict(mat=body, mode='facet', z=2.8, normal=(-0.25, 0.1), name='ridge', bevel=0),
            D=dict(mat=body, mode='facet', z=2.7, normal=(-0.8, 0.5), name='haunch', bevel=0),
            E=dict(mat=body, mode='facet', z=2.7, normal=(0.0, 0.8), name='tail', bevel=0),
        )
        b.paint([
            '....A',
            '...AA',
            '..AAA',
            '.BBAA',
            'BBBCC',
            'BBCCC',
            'DDCCC',
            'DDCCC',
            '.DEEE',
        ], -5, -6, L, mirror_x=True)
        # Smoke dischargers: a rack of three on each flank.
        for px in (rect(-5, -1, -5, 1), mrect(-5, -1, -5, 1)):
            b.part(px, GUNMETAL, 'cylv', 2.9, step=0, name='smoke_rack')
        b.part({(-5, 0), (4, 0)}, DARK, 'flat', 3.0, step=1, name='smoke_mouths')
        net = rect(-4, 3, 3, 4) | {(-5, 3), (4, 2)}
        mesh = bevel_offsets(net, 1, False)
        for (x, y) in net:
            mesh[(x, y)] += 0 if ((x // 2 + y // 2) % 2) == 0 else -1
        b.part(net, body, 'map', 2.9, stepmap=mesh, name='bustle_net', tags=('stowage',))
        applique(b, rect(-3, -2, -2, 0), 2.7, 'flank_l', normal=(-0.6, -0.2))
        applique(b, mrect(-3, -2, -2, 0), 2.7, 'flank_r', normal=(0.6, -0.2))
        b.lamp({(-1, -3), (0, -3)}, 'sensor', z=3.0, kind='sensor', name='eye')
        b.part(rect(-1, -1, 0, -1), GUNMETAL, 'plate', 2.8, step=-1, name='visor', tags=('optic',))
        b.meta['muzzles'] = [(0.0, -13)]

    def hardpoints(self, ctx):
        return dict(minigun=(5, -1), laser=(-6, 0), missiles=(0, 1), flame=(-4, 3), plasma=[(0.0, -13)])


# ---------------------------------------------------------------------------
# Glacier - arctic industrial
# ---------------------------------------------------------------------------
# Concept: the ice-road machine. A V snow plough across the whole nose - a
# painted mouldboard behind a bright steel cutting edge, on two rams -,
# cleated (chevron) tracks, a coolant tank strapped across the engine deck
# (in the accent - the team colour on a player's tank), a box turret with
# a heated optic (smoked glass under a copper heater bar).
# Silhouette: the arrow nose of the plough on the squattest hull. Night:
# pale lamps on the plough's wingtips, head lamps behind them, the beacon.
@LINE.design('glacier')
class Glacier(Design):
    codename = 'Icebreaker'
    blurb = 'An ice-road machine: a V plough with a steel cutting edge, cleated tracks, a coolant tank and a heated optic.'
    marks = 'heavy'
    sleeve_w, sleeve_off, sleeve_grow = 2, 3, 'both'

    def hull(self, b, f):
        body = b.ctx.body
        acc = b.ctx.accent
        tracks(b, -7, -5, -3, 7, style='chevron')
        b.part(rect(-4, -5, 3, 7), body, 'plate', 2, name='deck')
        skirts(b, -7, -5, [(-1, 5)], rivets=(0, 3))
        # The V plough: a steel wedge, a bright cutting edge, striped tips.
        plough = set()
        for x in range(-7, 0):
            yf = -7 + (-1 - x) // 2
            plough |= {(x, yf), (x, yf + 1), (x, yf + 2)}
        plough = sym(plough)
        front = {q for q in plough if (q[0], q[1] - 1) not in plough}
        b.part(plough - front, body, 'plate', 3.4, step=-1, name='mouldboard')
        b.part(front, STEEL, 'plate', 3.5, step=1, name='cutting_edge', bevel=0)
        # Ice-cutter teeth along the leading edge: a serrated arrow.
        teeth = sym({(x, -7 + (-1 - x) // 2 - 1) for x in (-6, -4)})
        b.part(teeth, STEEL, 'plate', 3.5, step=2, name='teeth', bevel=0)
        clearance(b, {(-7, -4), (6, -4)}, z=3.6, name='wingtips')
        b.part(rect(-3, -4, -3, -3) | mrect(-3, -4, -3, -3), STEEL, 'cylv', 3.3, step=1, name='rams')
        # The coolant tank across the engine deck, in the accent.
        b.part(rect(-4, 5, 2, 6), acc, 'cylh', 2.8, step=-1, name='coolant')
        applique(b, rect(-7, -1, -6, 1), 3.2, 'skirt_plate_l', step=-1, rivets={(-7, 0)})
        applique(b, mrect(-7, -1, -6, 1), 3.2, 'skirt_plate_r', step=-1, rivets={(6, 0)})
        b.part({(-2, 5), (-2, 6), (1, 5), (1, 6)}, STEEL, 'flat', 2.9, step=0, name='straps')
        b.part({(3, 5), (3, 6)}, STEEL, 'plate', 2.9, step=-1, name='valve')
        m = stack(b, 4, 4)
        beacon(b, -7, 5, z=3.4)
        b.lamp({(-6, -3)}, 'lamp', z=3.7, kind='head', name='head_l')
        b.lamp({(5, -3)}, 'lamp', z=3.7, kind='head', name='head_r')
        b.lamp({(-4, 7)}, 'tail', z=3.2, kind='tail', name='tail_l')
        b.lamp({(3, 7)}, 'tail', z=3.2, kind='tail', name='tail_r')
        b.part(ring(0, 0, 4.4, 3.4), STEEL, 'inset', 2.1, step=-2, tags=('ring',))
        self.emitters = dict(smoke=[m])

    def turret(self, b, pose):
        body = b.ctx.body
        gun(b, 0, 2, -13, -4, recoil=self.recoil(pose), brake=1, side='both')
        b.part(chamfer(-4, -4, 3, 4, tl=1, tr=1, bl=1, br=1), body, 'plate', 2.4, name='box')
        applique(b, rect(-3, -4, -1, -3), 2.6, 'brow_l', normal=(0.0, -0.8))
        applique(b, mrect(-3, -4, -1, -3), 2.6, 'brow_r', normal=(0.0, -0.8))
        applique(b, rect(-4, -2, -4, 3), 2.6, 'cheek_l', step=-1)
        applique(b, mrect(-4, -2, -4, 3), 2.6, 'cheek_r', step=-1)
        # The heated sight: an armoured hood on the front corner, smoked glass
        # facing forward under a copper heater strip.
        b.part(rect(2, -5, 4, -2), body, 'plate', 2.9, step=-1, name='sight_hood')
        b.part(rect(2, -5, 4, -5), SMOKED, 'flat', 3.0, step=1, name='sight_glass', tags=('optic',))
        b.part(rect(2, -4, 4, -4), COPPER, 'flat', 3.0, step=1, name='heater')
        hatch(b, -3, -1, z=2.7)
        b.rivets({(-4, -3), (3, -3), (-3, 4), (2, 4)}, body, 2.9)
        b.lamp({(-1, -4), (0, -4)}, 'sensor', z=2.9, kind='sensor', name='eye')
        b.meta['muzzles'] = [(0.0, -13)]

    def hardpoints(self, ctx):
        return dict(minigun=(4, -1), laser=(-5, 0), missiles=(0, 1), flame=(-3, 3), plasma=[(0.0, -13)])


# ---------------------------------------------------------------------------
# Warden - a mobile bunker
# ---------------------------------------------------------------------------
# Concept: a pillbox on tracks. Thick slab skirts with a raised lip,
# sandbags stacked on the front fenders, a thick hex turret with a
# sandbag parapet on its brow, periscopes round a domed commander's
# cupola, and a heavy gun wearing a body-coloured armour sleeve with a
# steel brake.
# Silhouette: the fattest turret of the standard hulls, the sandbag lumps
# at its corners and on the nose. Night: caged lamps, teal clearance
# lamps, four glowing periscopes, the beacon between the stacks.
BURLAP = Mat('burlap', [WOOD_DARKEST, SAND_DK, WOOD_DK, SAND_MD, SAND_LT, SAND_PALE], 3)


def sandbag(b, x, y, z=3.4, w=3, name='bag'):
    """A burlap sandbag seen from above: a w x 2 pillow, lit along its top,
    tucked darker at its right end (stowage: the first thing a hit loses)."""
    px = rect(x, y, x + w - 1, y + 1)
    off = {}
    for (qx, qy) in px:
        o = 1 if qy == y else 0
        if qx == x + w - 1:
            o -= 1
        if qx == x and qy == y + 1:
            o -= 1
        off[(qx, qy)] = o
    b.part(px, BURLAP, 'map', z, stepmap=off, name=name, tags=('stowage',))


@LINE.design('warden')
class Warden(Design):
    codename = 'Pillbox'
    blurb = 'A pillbox on tracks: slab skirts, a sandbagged nose, a periscope-ringed hex turret, a sleeved heavy gun.'
    marks = 'heavy'
    sleeve_w, sleeve_off, sleeve_grow = 2, 3, 'both'

    def hull(self, b, f):
        body = b.ctx.body
        tracks(b, -7, -5, -10, 9)
        b.part(rect(-4, -10, 3, 9), body, 'plate', 2, name='deck')
        b.part(rect(-4, -10, 3, -7), body, 'facet', 2.3, normal=(0.0, -0.8), name='glacis')
        skirts(b, -7, -5, [(-8, -1), (0, 7)], rivets=(-7, -3, 1, 5), step=0)
        applique(b, rect(-4, 5, -3, 6), 2.4, 'deck_l')
        applique(b, mrect(-4, 5, -3, 6), 2.4, 'deck_r')
        b.part(rect(-7, -8, -7, 7) | mrect(-7, -8, -7, 7), body, 'plate', 3.3, step=1, name='lips', bevel=1)
        # Sandbags on the front fenders.
        for (x, y) in ((-7, -8), (-6, -7)):
            sandbag(b, x, y, z=3.8 + 0.1 * (y + 8))
            sandbag(b, -1 - (x + 2), y, z=3.8 + 0.1 * (y + 8))
        m1 = stack(b, -4, 6)
        m2 = stack(b, 1, 6)
        beacon(b, -1, 7, z=3.4)
        b.part(rect(-4, 9, 3, 9), STEEL, 'plate', 2.4, step=-1, name='rear_bar')
        caged_lamp(b, -6, -9, name='head_l')
        caged_lamp(b, 5, -9, name='head_r')
        clearance(b, {(-7, -8), (6, -8)}, name='clear')
        b.lamp({(-6, 9)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(5, 9)}, 'tail', z=3, kind='tail', name='tail_r')
        b.part(ring(0, 0, 4.8, 3.6), STEEL, 'inset', 2.1, step=-2, tags=('ring',))
        self.emitters = dict(smoke=[m1, m2])

    def turret(self, b, pose):
        body = b.ctx.body
        r = self.recoil(pose)
        gun(b, 0, 2, -14, -5, recoil=r, brake=1, side='both')
        b.part(rect(-2, -9 + r, 1, -5), body, 'cylv', 2.6, step=0, bevel=0, name='sleeve', tags=('barrel',))
        b.part(rect(-2, -9 + r, 1, -9 + r), body, 'cylv', 2.7, step=-1, bevel=0, tags=('barrel',))
        L = dict(
            F=dict(mat=body, mode='facet', z=3, normal=(0.0, -0.8), name='brow'),
            C=dict(mat=body, mode='facet', z=2.8, normal=(-0.6, -0.6), name='corner'),
            S=dict(mat=body, mode='plate', z=2.8, step=-1, name='side'),
            R=dict(mat=body, mode='plate', z=2.8, name='roof'),
            B=dict(mat=body, mode='facet', z=2.8, normal=(0.0, 0.5), name='back'),
        )
        b.paint([
            '..FFFF',
            '.CFFFF',
            'CRRRRR',
            'SRRRRR',
            'SRRRRR',
            'SRRRRR',
            'SRRRRR',
            '.SRRRR',
            '..BBBB',
        ], -6, -5, L, mirror_x=True)
        # A sandbag parapet on the brow corners.
        for (x, y) in ((-6, -5), (-5, -4)):
            sandbag(b, x, y, z=3.3 + 0.1 * (y + 5))
            sandbag(b, -1 - (x + 2), y, z=3.3 + 0.1 * (y + 5))
        applique(b, rect(-6, -2, -6, 1), 2.9, 'slab_l', step=-1, rivets={(-6, -1)})
        applique(b, mrect(-6, -2, -6, 1), 2.9, 'slab_r', step=-1, rivets={(5, -1)})
        applique(b, rect(-3, 2, -2, 3), 2.9, 'roof_l')
        applique(b, mrect(-3, 2, -2, 3), 2.9, 'roof_r')
        b.part(circle(0, 0.5, 2.1), body, 'dome', 3.2, gain=0.5, name='cupola')
        b.part({(-1, 0), (0, 0)}, STEEL, 'flat', 3.3, step=1, name='hatch_handle', tags=('hatch',))
        for q in ((-3, -2), (2, -2), (-5, 1), (4, 1)):
            b.lamp({q}, 'marker', z=3.0, name='scope')
        b.lamp({(-1, -5), (0, -5)}, 'sensor', z=3.3, kind='sensor', name='eye')
        b.meta['muzzles'] = [(0.0, -14)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, -2), laser=(-7, -1), missiles=(0, 1), flame=(-5, 2), plasma=[(0.0, -14)])


# ---------------------------------------------------------------------------
# Ravager - the drill
# ---------------------------------------------------------------------------
# Concept: a mining machine gone to war. Twin steel augers on gearboxes
# over the front track horns, their spiral flutes stepping round with the
# track frame (they spin as it drives), welded spikes down both skirts,
# twin stacks, a round riveted turret with twin braked guns.
# Silhouette: the two drill points at the front corners, the spiked
# (sawtooth) sides. Night: lamps on the glacis, red clearance lamps on
# the gearboxes, the beacon between the stacks.
@LINE.design('ravager')
class Ravager(Design):
    codename = 'Auger'
    blurb = 'A mining machine gone to war: twin augers that spin as it drives, spiked skirts, twin braked guns.'
    marks = 'heavy'
    sleeve_w, sleeve_off, sleeve_grow = 2, 3, 'out'

    def hull(self, b, f):
        body = b.ctx.body
        tracks(b, -7, -5, -6, 10)
        b.part(rect(-4, -9, 3, 10), body, 'plate', 2, name='deck')
        b.part(rect(-4, -9, 3, -7), body, 'facet', 2.3, normal=(0.0, -0.8), name='glacis')
        applique(b, rect(-4, -9, -2, -8), 2.5, 'glacis_l', normal=(0.0, -0.8), rivets={(-3, -8)})
        applique(b, mrect(-4, -9, -2, -8), 2.5, 'glacis_r', normal=(0.0, -0.8), rivets={(2, -8)})
        skirts(b, -7, -5, [(-4, 1), (2, 7)], rivets=(-3, 0, 4, 6))
        # Welded spikes down the skirts.
        b.part(sym({(-8, y) for y in (-3, 0, 3, 6)}), STEEL, 'plate', 3.1, step=2, name='spikes', bevel=0)
        # Twin augers on gearboxes over the track horns; the flutes step
        # round with the hull frame, so they spin as it drives.
        for side in (-1, 1):
            cone = mask(['.#.', '###', '###', '###'], -8, -11)
            box = rect(-8, -7, -5, -5)
            if side > 0:
                cone, box = mirror(cone), mirror(box)
            flutes = {(x, y): (1 if ((x * side + y + f) % 3) == 0 else -1 if ((x * side + y + f) % 3) == 1 else 0)
                      for (x, y) in cone}
            b.part(cone, STEEL, 'map', 3.2, stepmap=flutes, name='auger')
            b.part(box, body, 'plate', 3.0, step=-1, name='gearbox', tags=('armor',))
        b.rivets(sym({(-7, -6)}), body, 3.1)
        m1 = stack(b, -4, 7)
        m2 = stack(b, 1, 7)
        beacon(b, -1, 5, z=3.4)
        b.lamp({(-3, -9)}, 'lamp', z=3.4, kind='head', name='head_l')
        b.lamp({(2, -9)}, 'lamp', z=3.4, kind='head', name='head_r')
        clearance(b, {(-8, -6), (7, -6)}, name='clear')
        b.lamp({(-4, 10)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(3, 10)}, 'tail', z=3, kind='tail', name='tail_r')
        b.part(ring(0, 0, 5.0, 3.8), STEEL, 'inset', 2.1, step=-2, tags=('ring',))
        self.emitters = dict(smoke=[m1, m2])

    def turret(self, b, pose):
        body = b.ctx.body
        for i, xc2 in enumerate((-6, 6)):
            gun(b, xc2, 2, -13, -5, recoil=self.recoil(pose, i, 2), side='out')
        b.part(circle(0, 0, 5.0), body, 'dome', 2.2, gain=0.45, name='drum')
        b.part(ring(0, 0, 5.0, 4.0), body, 'plate', 2.3, step=-1, name='band')
        applique(b, rect(-4, -5, -1, -4), 2.6, 'mantlet_l', normal=(0.0, -0.7), rivets={(-2, -4)})
        applique(b, mrect(-4, -5, -1, -4), 2.6, 'mantlet_r', normal=(0.0, -0.7), rivets={(1, -4)})
        applique(b, rect(-5, 0, -4, 2), 2.5, 'cheek_l', step=-1, rivets={(-5, 1)})
        applique(b, mrect(-5, 0, -4, 2), 2.5, 'cheek_r', step=-1, rivets={(4, 1)})
        b.part({(-4, -5), (-3, -5), (2, -5), (3, -5)}, STEEL, 'plate', 2.7, step=0, name='collars')
        b.rivets({(-3, 3), (2, 3)}, body, 2.5)
        hatch(b, -3, -2, z=2.6)
        b.glass(rect(1, -2, 2, -2), 2.7, step=1)
        b.lamp({(-1, -5), (0, -5)}, 'sensor', z=2.9, kind='sensor', name='eye')
        b.meta['muzzles'] = [(-3.0, -13), (3.0, -13)]

    def hardpoints(self, ctx):
        return dict(minigun=(5, -2), laser=(-6, -1), missiles=(0, 1), flame=(-4, 3), plasma=[(-3.0, -13), (3.0, -13)])


# ---------------------------------------------------------------------------
# Titan - the behemoth
# ---------------------------------------------------------------------------
# Concept: the yard's biggest machine. Four track runs (a pair each side),
# all four horns bare fore and aft, riveted skirts over the outer runs,
# striped bumpers between the inner ones front and back, two pairs of
# stacks, two beacons, and a big hex turret of bolted slabs with twin 4 px
# guns and huge brakes.
# Silhouette: the widest hull, the four horns, the paired stacks.
# Night: four caged lamps, four white clearance lamps down the skirts,
# two beacons, a searchlight on the turret.
@LINE.design('titan')
class Titan(Design):
    codename = 'Crucible'
    blurb = 'The behemoth: four track runs, hazard bumpers fore and aft, two pairs of stacks, twin 4 px guns with huge brakes.'
    marks = 'heavy'
    sleeve_w, sleeve_off, sleeve_grow = 4, 3, 'both'

    def hull(self, b, f):
        body = b.ctx.body
        # Four track runs, a pair each side, all four horns bare fore and aft.
        tracks(b, -11, -9, -11, 10)
        tracks(b, -8, -6, -11, 10)
        b.part(rect(-8, -8, 7, 6) | rect(-5, -11, 4, 11), body, 'plate', 2, name='deck')
        b.part(rect(-5, -10, 4, -8), body, 'facet', 2.3, normal=(0.0, -0.8), name='glacis')
        applique(b, rect(-5, -10, -3, -8), 2.5, 'glacis_l', normal=(0.0, -0.8), rivets={(-4, -9)})
        applique(b, mrect(-5, -10, -3, -8), 2.5, 'glacis_r', normal=(0.0, -0.8), rivets={(3, -9)})
        applique(b, rect(-5, 3, -3, 4), 2.4, 'deck_l', rivets={(-4, 3)})
        applique(b, mrect(-5, 3, -3, 4), 2.4, 'deck_r', rivets={(3, 3)})
        # Skirts over the outer runs; the deck is widened over the inner ones.
        skirts(b, -11, -9, [(-8, -2), (-1, 6)], rivets=(-5, 2), step=-1)
        # Striped bumpers between the inner runs, fore and aft.
        hazard(b, rect(-5, -12, 4, -11), 3.2)
        hazard(b, rect(-5, 10, 4, 11), 3.2)
        # Two pairs of stacks, a beacon on each outer fender.
        ms = [stack(b, x, 6) for x in (-8, -5, 2, 5)]
        beacon(b, -11, 7, z=3.6, name='beacon_l')
        beacon(b, 9, 7, z=3.6, name='beacon_r')
        caged_lamp(b, -10, -10, name='head_l')
        caged_lamp(b, 9, -10, name='head_r')
        caged_lamp(b, -7, -10, name='head_cl')
        caged_lamp(b, 6, -10, name='head_cr')
        clearance(b, {(-11, -7), (10, -7)}, name='clear')
        clearance(b, {(-11, 5), (10, 5)}, name='clear_rear', kind=None)
        b.lamp({(-7, 10)}, 'tail', z=3.4, kind='tail', name='tail_l')
        b.lamp({(6, 10)}, 'tail', z=3.4, kind='tail', name='tail_r')
        b.part(ring(0, 0, 6.2, 5.0), STEEL, 'inset', 2.1, step=-2, tags=('ring',))
        self.emitters = dict(smoke=ms)

    def turret(self, b, pose):
        body = b.ctx.body
        for i, xc2 in enumerate((-10, 10)):
            gun(b, xc2, 4, -16, -7, recoil=self.recoil(pose, i, 2), brake=1, side='both', baffle_rows=1)
        L = dict(
            M=dict(mat=body, mode='facet', z=3.1, normal=(0.0, -0.8), name='front'),
            F=dict(mat=body, mode='facet', z=2.9, normal=(-0.6, -0.6), name='corner'),
            S=dict(mat=body, mode='plate', z=2.8, step=-1, name='side'),
            R=dict(mat=body, mode='plate', z=2.8, name='roof'),
            B=dict(mat=body, mode='facet', z=2.8, normal=(0.0, 0.5), name='back'),
        )
        b.paint([
            '...MMMM',
            '..FMMMM',
            '.FRRRRR',
            'SRRRRRR',
            'SRRRRRR',
            'SRRRRRR',
            'SRRRRRR',
            'SRRRRRR',
            '.SRRRRR',
            '..BBBBB',
        ], -7, -7, L, mirror_x=True)
        for nm, px in (('mantlet_l', rect(-7, -7, -4, -6)), ('mantlet_r', rect(3, -7, 6, -6))):
            b.part(px, body, 'plate', 3.2, step=-1, name=nm)
        b.part({(-6, -7), (-5, -7), (4, -7), (5, -7)}, STEEL, 'plate', 3.3, step=-1, name='collars')
        applique(b, rect(-7, -3, -7, 1), 2.9, 'side_l', step=-1, rivets={(-7, -2), (-7, 0)})
        applique(b, mrect(-7, -3, -7, 1), 2.9, 'side_r', step=-1, rivets={(6, -2), (6, 0)})
        applique(b, rect(-3, -6, -1, -5), 3.2, 'front_l', normal=(0.0, -0.8), rivets={(-2, -5)})
        applique(b, mrect(-3, -6, -1, -5), 3.2, 'front_r', normal=(0.0, -0.8), rivets={(1, -5)})
        hatch(b, -4, -3, z=3.0, size=4, step=0)
        b.glass(rect(2, -3, 3, -3), 3.1, step=1)
        b.lamp({(-1, -7), (0, -7)}, 'sensor', z=3.3, kind='sensor', name='eye')
        caged_lamp(b, -8, -2, kind='spot', z=3.4, name='searchlight')
        b.meta['muzzles'] = [(-5.0, -16), (5.0, -16)]

    def hardpoints(self, ctx):
        return dict(minigun=(7, 0), laser=(-8, -1), missiles=(0, 0), flame=(-6, 1), plasma=[(-5.0, -16), (5.0, -16)])


# ---------------------------------------------------------------------------
# Leviathan - the land ironclad
# ---------------------------------------------------------------------------
# Concept: a dreadnought built on a slipway. A ship's bow narrowing to a
# steel cutwater, painted road-wheel hubs with steel bolts down both
# skirts, a command tower on the engine deck (a band of lit bridge
# windows, the beacon on its roof) between two stacks, and one huge 4 px
# gun with a massive double brake in a big riveted drum turret with a
# caged searchlight.
# Silhouette: the prow, the hub rhythm down the flanks, the tower behind
# the turret. Night: caged lamps on the bow, blue clearance lamps along
# the hull, the glowing bridge, the beacon on the tower.
@LINE.design('leviathan')
class Leviathan(Design):
    codename = 'Ironclad'
    blurb = "A land ironclad: a ship's bow, bolted road-wheel hubs down the skirts, a command tower, one huge braked gun."
    marks = 'heavy'
    sleeve_w, sleeve_off, sleeve_grow = 4, 5, 'both'

    def hull(self, b, f):
        body = b.ctx.body
        tracks(b, -10, -8, -9, 12)
        # The hull: a ship's bow narrowing to a steel cutwater.
        hull = chamfer(-7, -13, 6, 12, tl=5, tr=5, bl=1, br=1)
        b.part(hull, body, 'plate', 2, name='hull')
        b.part(chamfer(-7, -13, 6, -8, tl=5, tr=5), body, 'facet', 2.3, normal=(0.0, -0.9), name='bow')
        applique(b, poly([(-6, -8), (-3, -11), (-3, -8)]) | rect(-6, -9, -4, -8), 2.5, 'bow_l', normal=(-0.5, -0.8),
                 rivets={(-5, -9)})
        applique(b, mirror(poly([(-6, -8), (-3, -11), (-3, -8)]) | rect(-6, -9, -4, -8)), 2.5, 'bow_r',
                 normal=(0.5, -0.8), rivets={(4, -9)})
        applique(b, rect(-6, 5, -5, 7), 2.4, 'deck_l', rivets={(-6, 6)})
        applique(b, mrect(-6, 5, -5, 7), 2.4, 'deck_r', rivets={(5, 6)})
        b.part(rect(-1, -13, 0, -12), STEEL, 'plate', 2.6, step=1, name='cutwater')
        # Skirts with a row of bolted road-wheel hubs.
        skirts(b, -10, -8, [(-6, 1), (2, 9)], step=-1)
        for y in (-5, -1, 3, 7):
            for cx in (-8.5, 7.5):
                hub = circle(cx, y + 0.5, 1.5) - {(int(math.floor(cx)) - 1, y - 1), (int(math.floor(cx)) + 1, y - 1),
                                                  (int(math.floor(cx)) - 1, y + 1), (int(math.floor(cx)) + 1, y + 1)}
                b.part(hub, body, 'dome', 3.3, gain=0.7, step=1, name='hub')
                b.part({(int(math.floor(cx)), y)}, STEEL, 'flat', 3.4, step=2, name='hub_bolt')
        # The command tower behind the turret: bridge windows, a mast.
        tower = chamfer(-4, 8, 3, 12, tl=1, tr=1)
        b.part(tower, body, 'plate', 3.6, step=1, bevel=1, name='tower')
        b.lamp(rect(-3, 8, 2, 8), 'marker', z=3.7, kind='marker', name='bridge')
        b.part(rect(-2, 10, 1, 11), body, 'plate', 3.8, step=0, name='tower_top')
        beacon(b, -1, 10, z=3.9)
        m1 = stack(b, -7, 10)
        m2 = stack(b, 4, 10)
        caged_lamp(b, -4, -10, name='head_l', z=3.5)
        caged_lamp(b, 3, -10, name='head_r', z=3.5)
        clearance(b, {(-7, -8), (6, -8), (-7, 9), (6, 9)}, name='clear')
        b.lamp({(-9, 12)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(8, 12)}, 'tail', z=3, kind='tail', name='tail_r')
        b.part(ring(0, 0, 6.4, 5.2), STEEL, 'inset', 2.1, step=-2, tags=('ring',))
        self.emitters = dict(smoke=[m1, m2])

    def turret(self, b, pose):
        body = b.ctx.body
        gun(b, 0, 4, -17, -7, recoil=self.recoil(pose), brake=2, side='both', baffle_rows=2)
        b.part(circle(0, 0, 6.6), body, 'dome', 2.4, gain=0.45, name='drum')
        b.part(ring(0, 0, 6.6, 5.6), body, 'plate', 2.5, step=-1, name='band')
        applique(b, rect(-3, -7, -1, -5), 2.8, 'mantlet_l', normal=(0.0, -0.7), rivets={(-2, -5)})
        applique(b, mrect(-3, -7, -1, -5), 2.8, 'mantlet_r', normal=(0.0, -0.7), rivets={(1, -5)})
        applique(b, rect(1, 1, 3, 3), 2.8, 'roof_plate', step=-1, rivets={(3, 1)})
        b.rivets({(-6, -2), (5, -2), (-6, 1), (5, 1), (-3, 5), (2, 5)}, body, 2.8)
        hatch(b, -4, -2, z=2.8, size=4)
        b.glass(rect(2, -3, 3, -3), 2.9, step=1)
        b.lamp({(-1, -7), (0, -7)}, 'sensor', z=3.0, kind='sensor', name='eye')
        caged_lamp(b, -7, -3, kind='spot', z=3.1, name='searchlight')
        b.meta['muzzles'] = [(0.0, -17)]

    def hardpoints(self, ctx):
        return dict(minigun=(7, -2), laser=(-8, -1), missiles=(0, 2), flame=(-6, 4), plasma=[(0.0, -17)])
