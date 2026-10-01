"""PROTOTYPE - the experimental line. Lab machines that were never meant
to leave the proving ground: independent track pods at the corners with
open ground between them (six or eight on the super-heavies), a narrow
core with its reactor burning white in an orb, crystal-cut turrets riding
a magnetic ring that glows round them on the deck, coilguns banded with
energy and railguns with the charge glowing between their rails, glass
sensors. The accent colour becomes light: these tanks are drawn in their
glow as much as in their paint - the ring and the bands are the accent on
an enemy and the team colour on a player - and exposed internals glow
blue-white instead of burning.

Line vocabulary (helpers below): `pod` (tread, body cowl, strut hub),
`maglev` (the ring), `orb` (the reactor), `gem` (a crystal-cut shape with
designed facet steps), `hexagon`, `dome_cheeks` (blow-off plates on a
round turret), `coil_barrel`."""

from kit import *

LINE = Line('prototype', 'Prototype', 'Experimental: quad track pods, a visible reactor, coilguns, a floating turret.',
            notes='Track pods at the corners with open ground between them (six or eight on the super-heavies); a '
                  'reactor orb burning white in the tail; crystal-cut turrets riding a maglev ring that glows round '
                  'them; coilguns banded with light, railguns with the charge glowing between the rails. Exposed '
                  'internals glow blue-white instead of burning. Weapon modules are lab hardware in team-neutral '
                  'glows: an ion-coil rotary, a capsule revolver, a ruby laser, plasma ring collars, a gel-tank '
                  'flamer.')


# ---------------------------------------------------------------------------
# The line's parts
# ---------------------------------------------------------------------------
def pod(b, x0, y0, x1, y1, side, z=1.2, style='std', period=2, cowl='inner', hub=True, name=None):
    """A track pod: a short tread run under an armoured cowl in the body
    colour. The tread shows along the outer edge and round both ends; a
    steel hub on the cowl's inner edge is where the pod's strut meets the
    core."""
    b.tread(x0, x1, y0, y1, side, z=z, style=style, period=period, name=name)
    body = b.ctx.body
    if cowl == 'inner':
        cx0, cx1 = (x0 + 1, x1) if side == 'L' else (x0, x1 - 1)
        b.part(rect(cx0, y0 + 1, cx1, y1 - 1), body, 'plate', z + 1.3, step=-1,
               name=(name or 'pod') + '_cowl', tags=('armor',))
        if hub:
            hx = cx1 if side == 'L' else cx0
            hy = y0 + (y1 - y0) // 2
            b.part(rect(hx, hy, hx, hy + 1) if y1 - y0 >= 5 else {(hx, hy)}, STEEL, 'plate', z + 1.5, step=1,
                   name='hub')


def maglev(b, r_out, r_in, clip=None, z=2.4, cy=0.0, name='maglev'):
    """The magnetic ring the turret rides on: a glowing circle on the deck."""
    px = ring(0.0, cy, r_out, r_in)
    if clip is not None:
        px &= clip
    return b.lamp(px, 'marker', z=z, kind='marker', name=name, tags=('ring',))


def orb(b, cx, cy, r, z=2.6, housing=1, name='reactor', collar=None, collar_mode='plate', collar_step=-1):
    """The reactor as an orb: a white-hot core with a rim in the marker
    colour, set in a collar - steel by default, or the body's own socket
    (`collar=body, collar_mode='inset'`). `housing=0` leaves it bare."""
    core = circle(cx, cy, r)
    if housing:
        b.part(grow(core, housing, diag=True) - core, collar or STEEL, collar_mode, z - 0.1, step=collar_step,
               name=name + '_collar', tags=('vent',))
    # Tagged `engine`: a damaged reactor flickers before it dies.
    b.lamp(core, 'core', z=z, kind='marker', name=name, tags=('engine',))
    return core


def facet_step(a, amp=1.5, bias=-0.2):
    """The ramp step of a facet facing `a` radians (0 = right, pi/2 = the
    rear): brightest toward the light (the front left), darkest away from
    it, kept within +-2 so no facet turns into a dark mass."""
    return int(round(amp * math.cos(a - math.radians(225)) + bias))


def gem(b, outer, inner, mat, z, dirs=8, name='gem', tags=(), cx=0.0, cy=0.0, top_step=0, bevel=0, top_tags=(),
        skip=(), armor='cheeks', amp=1.5, bias=-0.2):
    """A cut-crystal shape: the band between `outer` and `inner` split into
    facets by direction from (cx, cy), each lit by `facet_step` (the front
    and left faces catch the light; `bias` lifts a dark body), the inner
    table flat on top.
    `armor` names the facets that can be blown off: 'cheeks' (the two side
    faces), a tuple of indices, or None. Returns {facet index: part} plus
    'top'. Facet k faces k * 360 / dirs degrees (0 = right, 90 = rear)."""
    if armor == 'cheeks':
        armor = (0, dirs // 2)
    armor = set(armor or ())
    groups = {}
    for (x, y) in set(outer) - set(inner):
        a = math.atan2(y + 0.5 - cy, x + 0.5 - cx)
        k = int(round(a / (2 * math.pi / dirs))) % dirs
        groups.setdefault(k, set()).add((x, y))
    out = {}
    for k, px in sorted(groups.items()):
        if k in skip:
            continue
        a = k * 2 * math.pi / dirs
        out[k] = b.part(px, mat, 'plate', z, step=facet_step(a, amp, bias), bevel=bevel,
                        name='%s_f%d' % (name, k), tags=tuple(tags) + (('armor',) if k in armor else ()))
    if inner:
        out['top'] = b.part(inner, mat, 'plate', z + 0.05, step=top_step, name=name + '_top', tags=top_tags)
    return out


def hexagon(x0, y0, x1, y1, cut):
    """A hexagon pointed front and back: `cut` rows taper to the tips."""
    w = x1 - x0 + 1
    pts = [(x0, y0 + cut), (x0 + w / 2.0, y0), (x1 + 1, y0 + cut), (x1 + 1, y1 + 1 - cut), (x0 + w / 2.0, y1 + 1),
           (x0, y1 + 1 - cut)]
    return poly(pts)


def dome_cheeks(b, r, body, z, rows=(-1, 1), depth=2, name='cheek'):
    """Side armour on a round turret: a plate on each flank of the dome,
    `depth` pixels in from its edge - the pieces a hit blows off."""
    shell = circle(0, 0, r)
    for sx, nm in ((-1, '_l'), (1, '_r')):
        px = {(x, y) for (x, y) in shell if rows[0] <= y <= rows[1] and (x < 0) == (sx < 0) and
              any((x + sx * k, y) not in shell for k in range(1, depth + 1))}
        b.part(px, body, 'plate', z, step=0 if sx < 0 else -1, name=name + nm, tags=('armor',))


def coil_barrel(b, xc2, width, y_tip, y_base, recoil, z=2.5, bands=3, gap=2, start=2):
    """A coilgun: a steel barrel wound with glowing bands."""
    parts = b.barrel(xc2, width, y_tip, y_base, mat=STEEL, z=z, recoil=recoil, bore=True)
    x0 = (xc2 - width) // 2
    for k in range(bands):
        y = y_tip + recoil + start + k * gap
        if y < y_base:
            b.lamp(rect(x0, y, x0 + width - 1, y), 'marker', z=z + 0.2, tags=('barrel', 'engine'))
    return parts


# ---------------------------------------------------------------------------
# Weapon modules: lab hardware. Modules are drawn once for every team, so
# they glow in team-neutral colours (ion blue-white, plasma, a ruby laser,
# amber gel) - never `marker`, which is the enemy's accent.
# ---------------------------------------------------------------------------
# A ruby lasing crystal and the flamer's amber gel, both on the palette.
RUBY = Mat('ruby', [RED_DARKEST, RED_DK, RED_DEEP, RED_MD, RED_BRIGHT, GOLD_BRIGHT, WHITE], 2)
GEL = Mat('gel', [WOOD_DARKEST, WOOD_DEEPER, WOOD_DK, WOOD_AMBER, GOLD_BRIGHT, WOOD_PALE], 3)


@LINE.module_fn('minigun')
def minigun(d, b, st, hp):
    """A coil-driven rotary: a motor drum, three barrels through a glowing
    drive coil; the hot barrel walks round as it spins."""
    hx, hy = hp['minigun']
    b.part(rect(hx - 1, hy - 1, hx + 1, hy + 2), STEEL, 'cylv', 2, step=-1, name='mg_drum')
    b.part(rect(hx - 1, hy + 3, hx + 1, hy + 3), DARK, 'flat', 2, step=1, name='mg_feed')
    for i, x in enumerate((hx - 1, hx, hx + 1)):
        b.part(rect(x, hy - 6, x, hy - 2), STEEL, 'flat', 3, step=(1 if i == 0 else 0 if i == 1 else -1))
        if st > 0 and (st - 1) == i:
            b.lamp({(x, hy - 6)}, 'hot', z=3.5)
    b.part(rect(hx - 2, hy - 3, hx + 2, hy - 3), STEEL, 'cylv', 3.2, step=0, name='mg_coil')
    b.lamp(rect(hx - 1, hy - 3, hx + 1, hy - 3), 'ion', z=3.3)  # the drive coil


@LINE.module_fn('missiles')
def missiles(d, b, st, hp):
    """A capsule revolver: a round drum of four capsules round a hub, the
    seeker heads lit while loaded; it empties one capsule per salvo."""
    hx, hy = hp['missiles']
    drum = circle(hx, hy + 0.5, 2.6)
    b.part(drum, STEEL, 'dome', 5, step=0, gain=0.45, name='ml_drum')
    b.part(edge(drum), STEEL, 'plate', 5.05, step=-1, bevel=0, name='ml_rim')
    b.part({(hx - 1, hy), (hx, hy)}, STEEL, 'flat', 5.3, step=2, name='ml_hub')
    # The four ports in a pinwheel round the hub.
    ports = [(hx - 2, hy), (hx + 1, hy), (hx - 1, hy - 1), (hx, hy + 1)]
    for i, (x, y) in enumerate(ports):
        if i >= st:
            b.lamp({(x, y)}, 'ion', z=5.5)
        else:
            b.part({(x, y)}, DARK, 'flat', 5.5, step=-1)


@LINE.module_fn('laser')
def laser(d, b, st, hp):
    """A crystal laser: a ruby rod in a steel clamp; it lights from within
    as it charges and burns white as it fires. Spans hy-4..hy (hy-5
    firing)."""
    hx, hy = hp['laser']
    b.part(rect(hx - 1, hy - 2, hx, hy), STEEL, 'plate', 3, step=-1, name='lz_clamp')
    b.part(rect(hx - 1, hy - 1, hx, hy - 1), DARK, 'flat', 3.1, step=1)
    crystal = rect(hx - 1, hy - 4, hx, hy - 3)
    b.part(crystal, RUBY, 'glass', 3.2, name='lz_crystal')
    if st == 1:
        b.lamp(rect(hx - 1, hy - 3, hx, hy - 3), 'laser', z=3.4)
    elif st == 2:
        b.lamp(crystal, 'laser', z=3.4)
        b.lamp(rect(hx - 1, hy - 5, hx, hy - 5), 'white', z=3.5)


@LINE.module_fn('plasma')
def plasma(d, b, st, hp):
    """A ring emitter at each muzzle: a steel collar round the barrel with
    a glowing torus; on twin guns it only reaches outward, so the barrels
    keep their air."""
    role = 'plasma' if st > 0 else BLUE_DK
    w = getattr(d, 'gun_w', 2)
    for (mx, my) in hp['plasma']:
        x0 = int(math.floor(mx - w / 2.0))
        x1 = x0 + w - 1
        lo, hi = x0 - 1, x1 + 1
        if len(hp['plasma']) > 1:
            if mx < 0:
                hi = x1
            else:
                lo = x0
        y = int(my) + 2
        b.part(rect(lo, y, hi, y + 2), STEEL, 'cylv', 7, step=-1, bevel=0, name='pl_collar')
        b.lamp(rect(lo, y + 1, hi, y + 1), role, z=7.2)
        if st == 2:
            b.lamp(rect(x0, y - 1, x1, y - 1), 'white', z=7.3)


@LINE.module_fn('flame')
def flame(d, b, st, hp):
    """A gel-tank flamer: an amber gel canister between steel caps, a
    nozzle forward with a pilot flame. Spans hy-4..hy+3 (hy-5 firing)."""
    hx, hy = hp['flame']
    b.part(rect(hx, hy, hx + 1, hy + 3), GEL, 'cylv', 3, step=1, name='fl_tank')
    b.part(rect(hx, hy, hx + 1, hy) | rect(hx, hy + 3, hx + 1, hy + 3), STEEL, 'flat', 3.1, step=0, name='fl_caps')
    b.part(rect(hx, hy - 3, hx, hy - 1), STEEL, 'cylv', 3.1, step=0, name='fl_nozzle')
    if st >= 2:
        b.lamp({(hx, hy - 4), (hx, hy - 5)} if st == 2 else {(hx, hy - 4), (hx + 1, hy - 5), (hx, hy - 5)},
               'fire', z=3.5)
    else:
        b.lamp({(hx, hy - 4)}, 'hot' if st == 1 else 'fire', z=3.5)


# ---------------------------------------------------------------------------
# Assault - twin coilguns on a floating turret
# ---------------------------------------------------------------------------
# Concept: the line's reference. Four track pods at the corners of a narrow
# core, open ground between them - the H every Prototype is built on. The
# maglev ring peeks out as glowing arcs in front of and behind a crystal
# box turret; the reactor orb burns white in its socket in the tail; two
# coilguns banded in the accent. Night: teal arcs, the orb, the bands.
@LINE.design('assault')
class Assault(Design):
    codename = 'Tesseract'
    blurb = 'Four track pods, a reactor orb glowing in the tail, twin coilguns on a crystal turret riding a glowing ring.'
    locomotion = 'quad'
    marks = 'quad'
    internal_glow = 'ion'

    def hull(self, b, f):
        body = b.ctx.body
        # Four pods at the corners, a notch of open ground between them.
        pod(b, -7, -10, -5, -5, 'L', name='pod_fl')
        pod(b, 4, -10, 6, -5, 'R', name='pod_fr')
        pod(b, -7, 4, -5, 9, 'L', name='pod_rl')
        pod(b, 4, 4, 6, 9, 'R', name='pod_rr')
        # The core chassis.
        core = chamfer(-4, -9, 3, 9, tl=2, tr=2, bl=1, br=1)
        b.part(core, body, 'plate', 2, name='core', sep=True)
        b.part(chamfer(-4, -9, 3, -7, tl=2, tr=2), body, 'facet', 2.3, normal=(0.0, -0.9), name='prow', tags=('armor',))
        maglev(b, 5.9, 4.9, clip=core)
        orb(b, 0.0, 6.5, 2.1, collar=body, collar_mode='inset', collar_step=0)
        b.lamp(rect(-2, -10, 1, -10), 'lamp', z=3, kind='head', name='lightbar')
        b.lamp({(-6, -10)}, 'lamp', z=3, kind='head', name='pod_lamp_l')
        b.lamp({(5, -10)}, 'lamp', z=3, kind='head', name='pod_lamp_r')
        b.lamp({(-6, 9)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(5, 9)}, 'tail', z=3, kind='tail', name='tail_r')

    def turret(self, b, pose):
        body = b.ctx.body
        for i, xc2 in enumerate((-6, 6)):
            coil_barrel(b, xc2, 2, -14, -4, self.recoil(pose, i, 2))
        outer = chamfer(-5, -4, 4, 3, tl=2, tr=2, bl=1, br=1)
        inner = chamfer(-3, -2, 2, 1, tl=1, tr=1, bl=1, br=1)
        gem(b, outer, inner, body, 3, dirs=8, name='turret', cy=-0.5)
        b.glass(circle(0, -0.5, 1.6), 3.5, mat=GLASS)
        b.lamp({(-1, -1), (0, -1)}, 'sensor', z=3.7, kind='sensor', name='eye')
        b.meta['muzzles'] = [(-3.0, -14), (3.0, -14)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, -1), laser=(-6, -3), missiles=(0, 3), flame=(-6, 2),
                    plasma=[(-3.0, -14), (3.0, -14)])


# ---------------------------------------------------------------------------
# Scout - a tri-pod: two pods up front, one drive pod under the tail
# ---------------------------------------------------------------------------
# Concept: the lab's fastest runner. An arrowhead silhouette no other
# chassis has - wide at the front on two steering pods, narrowing to a
# single drive pod under the tail that carries the reactor. A small round
# turret wears one big glass sensor orb (the focal point) and a slim
# coilgun. Night: the reactor on the tail, the red eye, the barrel bands.
@LINE.design('scout')
class Scout(Design):
    codename = 'Quark'
    blurb = 'A tri-pod runner: two steering pods, one drive pod under a glowing tail; a glass sensor orb.'
    locomotion = 'quad'
    marks = 'quad'
    internal_glow = 'ion'

    def hull(self, b, f):
        body = b.ctx.body
        pod(b, -6, -8, -4, -3, 'L', name='pod_fl')
        pod(b, 3, -8, 5, -3, 'R', name='pod_fr')
        # The drive pod: one wide run on the centre line.
        b.tread(-2, -1, 3, 8, 'L', z=1.2, style='fine', period=2, name='pod_rl')
        b.tread(0, 1, 3, 8, 'R', z=1.2, style='fine', period=2, name='pod_rr')
        core = chamfer(-3, -7, 2, 1, tl=2, tr=2) | rect(-2, 2, 1, 3)
        b.part(core, body, 'plate', 2, name='core', sep=True)
        b.part(chamfer(-3, -7, 2, -5, tl=2, tr=2), body, 'facet', 2.3, normal=(0.0, -0.9), name='prow', tags=('armor',))
        # The reactor rides the drive pod.
        b.part(rect(-1, 4, 0, 7), body, 'plate', 2.4, step=-1, name='tail_cowl', tags=('armor',))
        b.lamp({(-1, 4), (0, 4), (-1, 6), (0, 6)}, 'marker', z=2.7, name='reactor_rim', tags=('engine',))
        b.lamp({(-1, 5), (0, 5)}, 'core', z=2.7, kind='marker', name='reactor', tags=('engine',))
        maglev(b, 5.9, 4.9, clip=core)
        b.lamp({(-5, -8)}, 'lamp', z=3, kind='head', name='pod_lamp_l')
        b.lamp({(4, -8)}, 'lamp', z=3, kind='head', name='pod_lamp_r')
        b.lamp({(-1, -8), (0, -8)}, 'lamp', z=3, kind='head', name='nose_lamp')
        b.lamp({(-2, 8), (1, 8)}, 'tail', z=3, kind='tail', name='tail')

    def turret(self, b, pose):
        body = b.ctx.body
        coil_barrel(b, 0, 2, -14, -4, self.recoil(pose), bands=2, gap=3, start=3)
        b.dome(circle(0, 0, 4.5), body, 3, gain=0.55, name='shell')
        b.part(rect(-4, 2, 3, 3) & circle(0, 0, 4.5), body, 'facet', 3.1, normal=(0.0, 0.8), name='skirt')
        dome_cheeks(b, 4.5, body, 3.2, rows=(-2, 1))
        # The sensor orb: a big glass eye on the front of the dome.
        b.dome(circle(0.0, -1.0, 2.1), GLASS, 3.5, step=1, gain=0.6, name='orb', tags=('optic',))
        b.lamp({(-1, -1), (0, -1)}, 'sensor', z=3.7, kind='sensor', name='eye')
        b.part(rect(1, 0, 2, 1), body, 'inset', 3.3, name='hatch', tags=('hatch',))
        # A whip antenna trailing off the back of the turret.
        b.part(line(2, 2, 3, 5), STEEL, 'flat', 3.8, step=1, tags=('antenna',), contact=False, cast=False)
        b.part({(3, 5)}, STEEL, 'flat', 3.9, step=3, tags=('antenna',), contact=False, cast=False)
        b.meta['muzzles'] = [(0.0, -14)]

    def hardpoints(self, ctx):
        return dict(minigun=(4, -1), laser=(-4, -2), missiles=(0, 2), flame=(-5, 3), plasma=[(0.0, -14)])


# ---------------------------------------------------------------------------
# Breaker - the rammer: an electromagnetic ram across the whole front
# ---------------------------------------------------------------------------
# Concept: a heavy brawler built round its ram - a coil-wound bar across
# the full width of the nose, its windings glowing gold, that throws the
# tank's mass into whatever it hits. Wide four-track pods, a hex turret, one
# short fat coilgun with two heavy field collars. Silhouette: the widest
# front in the roster, a T. Night: the ram's gold coils, the reactor.
@LINE.design('breaker')
class Breaker(Design):
    codename = 'Impulse'
    blurb = 'A coil-wound kinetic ram across the whole nose, wide pods, one short fat coilgun.'
    locomotion = 'quad'
    marks = 'quad'
    internal_glow = 'ion'
    gun_w = 4

    def hull(self, b, f):
        body = b.ctx.body
        pod(b, -8, -8, -5, -3, 'L', name='pod_fl')
        pod(b, 4, -8, 7, -3, 'R', name='pod_fr')
        pod(b, -8, 5, -5, 10, 'L', name='pod_rl')
        pod(b, 4, 5, 7, 10, 'R', name='pod_rr')
        core = chamfer(-4, -9, 3, 10, bl=1, br=1)
        b.part(core, body, 'plate', 2, name='core', sep=True)
        b.part(rect(-4, -8, 3, -6), body, 'facet', 2.3, normal=(0.0, -0.9), name='prow', tags=('armor',))
        # The ram: a steel coil across the full width, wound with glowing
        # rings, capped in armour at both ends.
        b.part(rect(-7, -11, 6, -9), STEEL, 'cylh', 3, step=0, name='ram')
        for x in (-6, -3, 2, 5):
            b.lamp(rect(x, -11, x, -9), 'marker', z=3.2, name='ram_coil')
        b.part(sym(chamfer(-8, -11, -7, -9, tl=1, bl=1)), body, 'plate', 3.3, step=0, name='ram_cap', tags=('armor',))
        maglev(b, 5.9, 4.9, clip=core)
        orb(b, 0.0, 7.5, 2.1, collar=body, collar_mode='inset', collar_step=0)
        b.lamp({(-1, -11), (0, -11)}, 'lamp', z=3.4, kind='head', name='ram_lamp')
        b.lamp({(-7, -10)}, 'lamp', z=3.4, kind='head', name='head_l')
        b.lamp({(6, -10)}, 'lamp', z=3.4, kind='head', name='head_r')
        b.lamp({(-7, 10)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(6, 10)}, 'tail', z=3, kind='tail', name='tail_r')

    def turret(self, b, pose):
        body = b.ctx.body
        r = self.recoil(pose)
        b.barrel(0, 4, -12, -5, mat=STEEL, z=2.5, recoil=r)
        for y in (-10, -7):
            b.part(rect(-3, y + r, 2, y + 1 + r), STEEL, 'cylv', 2.7, step=1, name='collar', tags=('barrel',))
            b.lamp(rect(-3, y + r, 2, y + r), 'marker', z=2.8, tags=('barrel', 'engine'))
        outer = hexagon(-5, -5, 4, 4, 2)
        inner = hexagon(-3, -3, 2, 2, 1)
        gem(b, outer, inner, body, 3, dirs=6, name='turret', bias=0.4)
        b.glass(circle(0, -1.0, 1.5), 3.5, mat=GLASS)
        b.lamp({(-1, -1), (0, -1)}, 'sensor', z=3.7, kind='sensor', name='eye')
        b.meta['muzzles'] = [(0.0, -12)]

    def hardpoints(self, ctx):
        return dict(minigun=(5, -1), laser=(-5, -3), missiles=(0, 3), flame=(-6, 2), plasma=[(0.0, -12)])


# ---------------------------------------------------------------------------
# Longbow - a true railgun: two long rails, a glowing channel between
# ---------------------------------------------------------------------------
# Concept: the lab's sniper. Two bare rails reach far past the nose, open
# at the tip like a tuning fork, with the charge running between them as a
# line of light - the longest glow in the roster. The pods sit at the very
# ends of a long wheelbase; a round turret with a glass rangefinder; the
# reactor orb in the tail. Night: the red channel and the reactor.
@LINE.design('longbow')
class Longbow(Design):
    codename = 'Needlepoint'
    blurb = 'An open railgun - two rails with the charge glowing between them - on a long four-pod spine.'
    locomotion = 'quad'
    marks = 'quad'
    internal_glow = 'ion'
    gun_w = 4

    def hull(self, b, f):
        body = b.ctx.body
        pod(b, -7, -12, -5, -6, 'L', name='pod_fl')
        pod(b, 4, -12, 6, -6, 'R', name='pod_fr')
        pod(b, -7, 5, -5, 11, 'L', name='pod_rl')
        pod(b, 4, 5, 6, 11, 'R', name='pod_rr')
        core = chamfer(-4, -11, 3, 11, tl=2, tr=2, bl=1, br=1)
        b.part(core, body, 'plate', 2, name='core', sep=True)
        b.part(chamfer(-4, -11, 3, -9, tl=2, tr=2), body, 'facet', 2.3, normal=(0.0, -0.9), name='prow', tags=('armor',))
        maglev(b, 5.9, 4.9, clip=core)
        orb(b, 0.0, 8.0, 2.1, collar=body, collar_mode='inset', collar_step=0)
        b.lamp(rect(-2, -12, 1, -12), 'lamp', z=3, kind='head', name='lightbar')
        b.lamp({(-6, -12)}, 'lamp', z=3, kind='head', name='pod_lamp_l')
        b.lamp({(5, -12)}, 'lamp', z=3, kind='head', name='pod_lamp_r')
        b.lamp({(-6, 11)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(5, 11)}, 'tail', z=3, kind='tail', name='tail_r')

    def turret(self, b, pose):
        body = b.ctx.body
        r = self.recoil(pose)
        # Two bare rails, open at the tip like a tuning fork, the charge
        # glowing between them; the shell leaves from between the prongs.
        b.part(rect(-2, -18 + r, -2, -5), STEEL, 'plate', 2.5, step=1, bevel=0, tags=('barrel',), name='rail_l')
        b.part(rect(1, -18 + r, 1, -5), STEEL, 'plate', 2.5, step=-1, bevel=0, tags=('barrel',), name='rail_r')
        b.lamp(rect(-1, -15 + r, 0, -5), 'marker', z=2.4, name='channel', tags=('barrel', 'engine'))
        for y in (-13, -9):
            b.part(rect(-2, y + r, 1, y + r), STEEL, 'plate', 2.6, step=0, bevel=0, tags=('barrel',))
        b.dome(circle(0, 0, 5.0), body, 3, step=1, gain=0.55, name='shell')
        b.part(rect(-5, 2, 4, 4) & circle(0, 0, 5.0), body, 'facet', 3.1, step=1, normal=(0.0, 0.8), gain=0.6, name='skirt')
        dome_cheeks(b, 5.0, body, 3.2, rows=(-2, 1))
        b.part(rect(-2, -5, 1, -4), STEEL, 'plate', 3.2, step=-1, name='breech')
        b.glass(rect(-4, -2, -2, -1), 3.4, mat=GLASS)
        b.lamp({(-4, -2)}, 'sensor', z=3.6, kind='sensor', name='eye')
        b.part(rect(1, 0, 3, 1), body, 'inset', 3.3, name='hatch', tags=('hatch',))
        b.meta['muzzles'] = [(0.0, -16)]

    def hardpoints(self, ctx):
        return dict(minigun=(5, -1), laser=(-5, -3), missiles=(0, 3), flame=(-6, 2), plasma=[(0.0, -16)])


# ---------------------------------------------------------------------------
# Flak - twin pulse cannons fed by capacitor banks, a search radar
# ---------------------------------------------------------------------------
# Concept: a squat air-defence block. Two stubby pulse cannons with fat
# charge chambers that bulge outward, capacitor cells glowing in both
# cheeks of a hex turret, and a search-radar dish on its own mast off the
# turret's back right - the one asymmetric turret in the line. Four short
# pods on a compact core. Night: pale gold chambers, cells and ring.
@LINE.design('flak')
class Flak(Design):
    codename = 'Chorus'
    blurb = 'Twin stubby pulse cannons fed by capacitor cells in both cheeks; a search-radar dish off the turret\'s back.'
    locomotion = 'quad'
    marks = 'quad'
    internal_glow = 'ion'

    def hull(self, b, f):
        body = b.ctx.body
        pod(b, -7, -8, -5, -4, 'L', name='pod_fl')
        pod(b, 4, -8, 6, -4, 'R', name='pod_fr')
        pod(b, -7, 3, -5, 7, 'L', name='pod_rl')
        pod(b, 4, 3, 6, 7, 'R', name='pod_rr')
        core = chamfer(-4, -8, 3, 7, tl=2, tr=2, bl=1, br=1)
        b.part(core, body, 'plate', 2, name='core', sep=True)
        b.part(chamfer(-4, -8, 3, -6, tl=2, tr=2), body, 'facet', 2.3, normal=(0.0, -0.9), name='prow', tags=('armor',))
        maglev(b, 5.9, 4.9, clip=core)
        orb(b, 0.0, 5.0, 1.1, z=2.7, housing=0)
        b.lamp({(-6, -8)}, 'lamp', z=3, kind='head', name='pod_lamp_l')
        b.lamp({(5, -8)}, 'lamp', z=3, kind='head', name='pod_lamp_r')
        b.lamp({(-6, 7)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(5, 7)}, 'tail', z=3, kind='tail', name='tail_r')

    def turret(self, b, pose):
        body = b.ctx.body
        for i, xc2 in enumerate((-6, 6)):
            r = self.recoil(pose, i, 2)
            b.barrel(xc2, 2, -10, -4, mat=STEEL, z=2.5, recoil=r)
            x0 = (xc2 - 2) // 2
            # The pulse chamber: a fat collar that reaches outward only.
            xo = x0 - 1 if xc2 < 0 else x0
            b.part(rect(xo, -7 + r, xo + 2, -5 + r), STEEL, 'cylv', 2.7, step=0, tags=('barrel',), name='chamber')
            b.lamp(rect(xo, -6 + r, xo + 2, -6 + r), 'marker', z=2.8, tags=('barrel', 'engine'))
        outer = hexagon(-5, -5, 4, 4, 2)
        inner = hexagon(-3, -3, 2, 2, 1)
        gem(b, outer, inner, body, 3, dirs=6, name='turret')
        # Capacitor banks down each cheek.
        for x in (-5, 4):
            b.part(rect(x, -2, x, 2), STEEL, 'plate', 3.2, step=-1, name='bank', tags=('armor',))
            b.lamp({(x, -1)}, 'marker', z=3.3)
            b.lamp({(x, 1)}, 'marker', z=3.3)
        # The sensor array: a search-radar dish on its own mast, off to the
        # rear right - a smoked bowl in a bright rim, its feed glowing.
        dish = circle(2.0, 4.0, 2.2)
        b.part(edge(dish), STEEL, 'plate', 3.45, step=2, bevel=0, name='dish_rim', tags=('fragile',))
        b.part(dish - edge(dish), SMOKED, 'plate', 3.4, step=0, bevel=0, name='dish', tags=('fragile',))
        b.lamp({(1, 3)}, 'marker', z=3.6, name='feed')
        b.glass(circle(0, -1.5, 1.4), 3.5, mat=GLASS)
        b.lamp({(-1, -2), (0, -2)}, 'sensor', z=3.7, kind='sensor', name='eye')
        b.meta['muzzles'] = [(-3.0, -10), (3.0, -10)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, 0), laser=(-6, -2), missiles=(-2, 3), flame=(-7, 3),
                    plasma=[(-3.0, -10), (3.0, -10)])


# ---------------------------------------------------------------------------
# Wraith - a flying wing on four slim feet: phase-cloak glass, almost dark
# ---------------------------------------------------------------------------
# Concept: the one that is not there. A faceted flying wing - a delta with
# a sawtooth trailing edge - hides four slim pods, only their feet showing
# ahead of the leading edges and behind the trailing edge. Smoked-glass
# phase-emitter panels lie along both wings; the lights are few and cold.
# A blade turret. Silhouette: the only delta in the roster, the scout's
# arrowhead turned round. Night: barely anything - the point of it.
WING = poly([(-1, -8.0), (1, -8.0), (6, 2.6), (6, 5.4), (3.6, 4.2), (1, 6.9), (-1, 6.9), (-3.6, 4.2), (-6, 5.4),
             (-6, 2.6)])


@LINE.design('wraith')
class Wraith(Design):
    codename = 'Phase'
    blurb = 'A faceted flying wing on four slim feet; smoked-glass phase emitters, a blade turret, almost no light.'
    locomotion = 'quad'
    marks = 'quad'
    internal_glow = 'ion'

    def hull(self, b, f):
        body = b.ctx.body
        b.tread(-5, -4, -5, -1, 'L', z=1.2, style='fine', period=2, name='pod_fl')
        b.tread(3, 4, -5, -1, 'R', z=1.2, style='fine', period=2, name='pod_fr')
        b.tread(-5, -4, 3, 8, 'L', z=1.2, style='fine', period=2, name='pod_rl')
        b.tread(3, 4, 3, 8, 'R', z=1.2, style='fine', period=2, name='pod_rr')
        inner = poly([(-1, -4.5), (1, -4.5), (3.5, 2.0), (1, 4.8), (-1, 4.8), (-3.5, 2.0)])
        gem(b, WING, inner, body, 2, dirs=8, name='wing', armor=(3, 1))
        # Phase-emitter glass along both wings.
        for sx in (-1, 1):
            panel = {(x, y) for (x, y) in WING if 1 <= y <= 3 and 3 <= (x if sx > 0 else -1 - x) <= 4}
            b.part(panel, SMOKED, 'glass', 2.3, step=1, name='phase', tags=('optic', 'armor'))
        # The maglev ring runs dark: unlit smoked glass, no glow to give it away.
        b.part(ring(0.0, 0.0, 5.9, 4.9) & WING, SMOKED, 'flat', 2.4, step=0, name='maglev_dark', tags=('ring',))
        b.lamp({(-1, 5), (0, 5)}, 'core', z=2.4, kind='marker', name='reactor', tags=('engine',))
        b.lamp({(-1, -8), (0, -8)}, 'lamp', z=3, kind='head', name='nose_lamp')
        b.lamp({(-5, -5)}, 'lamp', z=3, kind='head', name='foot_lamp_l')
        b.lamp({(4, -5)}, 'lamp', z=3, kind='head', name='foot_lamp_r')
        b.lamp({(-5, 8)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(4, 8)}, 'tail', z=3, kind='tail', name='tail_r')

    def turret(self, b, pose):
        body = b.ctx.body
        r = self.recoil(pose)
        coil_barrel(b, 0, 2, -13, -4, r, bands=1, gap=3, start=4)
        blade = poly([(0, -6), (4.5, 1), (3.5, 3.5), (-3.5, 3.5), (-4.5, 1)])
        gem(b, blade, poly([(0, -3), (2, 1), (1.5, 2.5), (-1.5, 2.5), (-2, 1)]), body, 3, dirs=6,
            name='turret', cy=0.5)
        b.glass(rect(-1, -2, 0, -1), 3.5, mat=SMOKED)
        b.lamp({(-1, -1), (0, -1)}, 'sensor', z=3.6, kind='sensor', name='eye')
        b.meta['muzzles'] = [(0.0, -13)]

    def hardpoints(self, ctx):
        return dict(minigun=(4, 0), laser=(-4, -2), missiles=(0, 2), flame=(-5, 3), plasma=[(0.0, -13)])


# ---------------------------------------------------------------------------
# Warden - a shield projector: a glowing hoop round the whole machine
# ---------------------------------------------------------------------------
# Concept: the support tank. Its shield emitter is a hoop bigger than the
# hull's core: it rides the four pods on emitter nodes and floats across
# the open gaps between them, glowing all the way round the turret. A hex
# turret with one heavy coilgun. Silhouette: a ring inscribed in the tank,
# crossing the notches. Night: a full teal circle - the line's ring at its
# largest.
@LINE.design('warden')
class Warden(Design):
    codename = 'Aegis'
    blurb = 'A shield-emitter hoop rides the four pods and floats across the gaps, glowing all round the turret.'
    locomotion = 'quad'
    marks = 'quad'
    internal_glow = 'ion'
    gun_w = 4

    def hull(self, b, f):
        body = b.ctx.body
        pod(b, -7, -10, -5, -5, 'L', name='pod_fl')
        pod(b, 4, -10, 6, -5, 'R', name='pod_fr')
        pod(b, -7, 4, -5, 9, 'L', name='pod_rl')
        pod(b, 4, 4, 6, 9, 'R', name='pod_rr')
        core = chamfer(-4, -9, 3, 9, tl=2, tr=2, bl=1, br=1)
        b.part(core, body, 'plate', 2, name='core', sep=True)
        b.part(chamfer(-4, -9, 3, -7, tl=2, tr=2), body, 'facet', 2.3, normal=(0.0, -0.9), name='prow', tags=('armor',))
        # The emitter hoop and its four nodes.
        b.lamp(ring(0.0, 0.0, 7.0, 6.0), 'marker', z=2.9, kind='marker', name='maglev', tags=('ring',))
        for (x, y) in ((-6, -6), (4, -6), (-6, 4), (4, 4)):
            b.part(rect(x, y, x + 1, y + 1), STEEL, 'plate', 3.0, step=0, name='node')
        orb(b, 0.0, 8.5, 1.1, z=2.7, housing=0)
        b.lamp({(-1, -10), (0, -10)}, 'lamp', z=3, kind='head', name='nose_lamp')
        b.lamp({(-6, -10)}, 'lamp', z=3, kind='head', name='pod_lamp_l')
        b.lamp({(5, -10)}, 'lamp', z=3, kind='head', name='pod_lamp_r')
        b.lamp({(-6, 9)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(5, 9)}, 'tail', z=3, kind='tail', name='tail_r')

    def turret(self, b, pose):
        body = b.ctx.body
        r = self.recoil(pose)
        b.barrel(0, 4, -14, -5, mat=STEEL, z=2.5, recoil=r)
        for y in (-12, -9):
            b.part(rect(-2, y + r, 1, y + r), STEEL, 'plate', 2.6, step=1, bevel=0, tags=('barrel',))
            b.lamp(rect(-1, y + 1 + r, 0, y + 1 + r), 'marker', z=2.7, tags=('barrel', 'engine'))
        outer = hexagon(-5, -5, 4, 4, 2)
        inner = hexagon(-3, -3, 2, 2, 1)
        gem(b, outer, inner, body, 3, dirs=6, name='turret')
        b.glass(circle(0, -1.0, 1.5), 3.5, mat=GLASS)
        b.lamp({(-1, -1), (0, -1)}, 'sensor', z=3.7, kind='sensor', name='eye')
        b.meta['muzzles'] = [(0.0, -14)]

    def hardpoints(self, ctx):
        return dict(minigun=(5, -1), laser=(-5, -3), missiles=(0, 3), flame=(-6, 2), plasma=[(0.0, -14)])


# ---------------------------------------------------------------------------
# Ravager - twin coilguns and arc emitters on both flanks
# ---------------------------------------------------------------------------
# Concept: heavy assault that fights at knife range. A tesla electrode is
# clamped to each flank in the gap between the pods - white-hot in a red
# corona, ready to arc into whatever comes alongside. A round turret with
# twin coilguns. Silhouette: a bump of light on both sides. Night: three
# white-hot points (the electrodes and the reactor) inside the red ring.
@LINE.design('ravager')
class Ravager(Design):
    codename = 'Tempest'
    blurb = 'Twin coilguns on a round turret; a white-hot tesla electrode burns on each flank between the pods.'
    locomotion = 'quad'
    marks = 'quad'
    internal_glow = 'ion'

    def hull(self, b, f):
        body = b.ctx.body
        pod(b, -8, -11, -6, -5, 'L', name='pod_fl')
        pod(b, 5, -11, 7, -5, 'R', name='pod_fr')
        pod(b, -8, 4, -6, 10, 'L', name='pod_rl')
        pod(b, 5, 4, 7, 10, 'R', name='pod_rr')
        core = chamfer(-5, -10, 4, 10, tl=2, tr=2, bl=1, br=1)
        b.part(core, body, 'plate', 2, name='core', sep=True)
        b.part(chamfer(-5, -10, 4, -8, tl=2, tr=2), body, 'facet', 2.3, normal=(0.0, -0.9), name='prow', tags=('armor',))
        # Arc emitters: a tesla electrode on each flank - white-hot in a red
        # corona - clamped to the core across the gap between the pods.
        for sx in (-1, 1):
            cx = -7 if sx < 0 else 6
            ball = {(cx, -2), (cx - 1, -1), (cx, -1), (cx + 1, -1), (cx - 1, 0), (cx, 0), (cx + 1, 0), (cx, 1)}
            b.lamp(ball, 'core', z=2.4, name='emitter')
            b.part({(cx - sx, -3), (cx - sx, 2), (cx, -3), (cx, 2)}, STEEL, 'plate', 2.2, step=0,
                   name='emitter_clamp')
        maglev(b, 5.9, 4.9, clip=core)
        orb(b, 0.0, 6.5, 2.1, collar=body, collar_mode='inset', collar_step=0)
        b.lamp(rect(-2, -11, 1, -11), 'lamp', z=3, kind='head', name='lightbar')
        b.lamp({(-7, -11)}, 'lamp', z=3, kind='head', name='pod_lamp_l')
        b.lamp({(6, -11)}, 'lamp', z=3, kind='head', name='pod_lamp_r')
        b.lamp({(-7, 10)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(6, 10)}, 'tail', z=3, kind='tail', name='tail_r')

    def turret(self, b, pose):
        body = b.ctx.body
        for i, xc2 in enumerate((-6, 6)):
            coil_barrel(b, xc2, 2, -14, -4, self.recoil(pose, i, 2))
        b.dome(circle(0, 0, 5.0), body, 3, gain=0.55, name='shell')
        b.part(rect(-5, 2, 4, 4) & circle(0, 0, 5.0), body, 'facet', 3.1, normal=(0.0, 0.8), gain=0.6, name='skirt')
        dome_cheeks(b, 5.0, body, 3.2, rows=(-2, 1))
        b.glass(circle(0, -1.0, 1.5), 3.5, mat=GLASS)
        b.lamp({(-1, -1), (0, -1)}, 'sensor', z=3.7, kind='sensor', name='eye')
        b.meta['muzzles'] = [(-3.0, -14), (3.0, -14)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, -1), laser=(-6, -3), missiles=(0, 3), flame=(-6, 2),
                    plasma=[(-3.0, -14), (3.0, -14)])


# ---------------------------------------------------------------------------
# Glacier - an ice-breaker prow, a coolant canister, frost vents
# ---------------------------------------------------------------------------
# Concept: the arctic all-rounder. A V-shaped ice-breaker blade wraps the
# nose, its leading edge frosted white. Its coilgun is cryo-cooled: a glass
# canister of white-blue coolant lies across the back of the box turret,
# and the tail breathes through a frost radiator round the cryo pump.
# Short, square, sure-footed on four pods. Silhouette: the only pointed
# front in the compact class. Night: the coolant bar, the pump, the ring.
@LINE.design('glacier')
class Glacier(Design):
    codename = 'Kelvin'
    blurb = 'A frosted ice-breaker blade round the nose, a coolant canister glowing across the turret, frost vents.'
    locomotion = 'quad'
    marks = 'quad'
    internal_glow = 'ion'

    def hull(self, b, f):
        body = b.ctx.body
        pod(b, -7, -4, -5, -1, 'L', name='pod_fl')
        pod(b, 4, -4, 6, -1, 'R', name='pod_fr')
        pod(b, -7, 3, -5, 7, 'L', name='pod_rl')
        pod(b, 4, 3, 6, 7, 'R', name='pod_rr')
        core = chamfer(-4, -6, 3, 7, tl=2, tr=2, bl=1, br=1)
        b.part(core, body, 'plate', 2, name='core', sep=True)
        # The ice-breaker blade and its frosted edge.
        blade = poly([(-7, -3), (0, -7.6), (7, -3), (7, -1.2), (0, -5.6), (-7, -1.2)])
        frost = poly([(-7, -3), (0, -7.6), (7, -3), (7, -2.2), (0, -6.6), (-7, -2.2)])
        b.part({q for q in blade if q[0] < 0}, body, 'facet', 2.8, normal=(-0.5, -0.7), name='blade_l', tags=('armor',))
        b.part({q for q in blade if q[0] >= 0}, body, 'facet', 2.8, normal=(0.5, -0.7), name='blade_r', tags=('armor',))
        b.part(frost, PALE, 'plate', 2.9, step=0, name='frost')
        maglev(b, 5.9, 4.9, clip=core)
        # The frost radiator across the tail: white-rimed grilles either
        # side of the cryo pump, which glows white-blue between them.
        for x0 in (-3, 1):
            b.part(rect(x0, 4, x0 + 1, 6), PALE, 'grille', 2.5, step=1, pattern=dict(period=2, dir='h'),
                   name='frost_vent', tags=('vent',))
        b.part(rect(-1, 3, 0, 7), STEEL, 'plate', 2.4, step=0, name='pump')
        b.lamp({(-1, 4), (0, 4), (-1, 6), (0, 6)}, 'marker', z=2.6, name='cryo_rim', tags=('engine',))
        b.lamp({(-1, 5), (0, 5)}, 'core', z=2.6, kind='marker', name='cryo', tags=('engine',))
        b.lamp({(-4, -5)}, 'lamp', z=3, kind='head', name='blade_lamp_l')
        b.lamp({(3, -5)}, 'lamp', z=3, kind='head', name='blade_lamp_r')
        b.lamp({(-6, 7)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(5, 7)}, 'tail', z=3, kind='tail', name='tail_r')

    def turret(self, b, pose):
        body = b.ctx.body
        coil_barrel(b, 0, 2, -14, -4, self.recoil(pose), bands=3, gap=2, start=2)
        outer = chamfer(-5, -4, 4, 4, tl=2, tr=2, bl=1, br=1)
        inner = chamfer(-3, -2, 2, 2, tl=1, tr=1, bl=1, br=1)
        gem(b, outer, inner, body, 3, dirs=8, name='turret')
        b.glass(rect(-3, -3, 2, -2) - {(-3, -3), (2, -3)}, 3.5, mat=GLASS)
        b.lamp({(-1, -2), (0, -2)}, 'sensor', z=3.7, kind='sensor', name='eye')
        # The coolant canister across the turret's back: a glass capsule of
        # white-blue cryo fluid between steel end caps.
        can = rect(-4, 1, 3, 3) - {(-4, 1), (3, 1), (-4, 3), (3, 3)}
        b.part(grow(can, 1) & chamfer(-5, 0, 4, 4, tl=1, tr=1, bl=1, br=1), GLASS, 'plate', 3.4, step=0, bevel=0,
               name='can_sleeve', tags=('optic',))
        b.lamp(can - {(-4, 2), (3, 2)}, 'core', z=3.5, name='coolant')
        b.part({(-4, 2), (3, 2), (-5, 2), (4, 2)}, STEEL, 'plate', 3.6, step=1, bevel=0, name='can_cap')
        b.meta['muzzles'] = [(0.0, -14)]

    def hardpoints(self, ctx):
        return dict(minigun=(5, -1), laser=(-5, -3), missiles=(0, 3), flame=(-6, 2), plasma=[(0.0, -14)])


# ---------------------------------------------------------------------------
# Obelisk - a split railgun on a monolith turret, four outrigger anchors
# ---------------------------------------------------------------------------
# Concept: the siege engine. Its gun is one railgun split in two: two long
# rails at +-3 whose inner faces glow gold with the charge, so the pair
# reads as a single weapon with the shot running down the gap between.
# The turret is a monolith lying down - a long pointed wedge - and four
# hydraulic outriggers splay out of the flanks onto foot pads to brace the
# hull for the shot. Silhouette: long, crab-legged, with a pointed turret.
# Night: two parallel gold lines, the foot lights, the reactor.
@LINE.design('obelisk')
class Obelisk(Design):
    codename = 'Monolith'
    blurb = 'One railgun split into two long rails with the charge glowing between; four outriggers brace the hull.'
    locomotion = 'quad'
    marks = 'quad'
    internal_glow = 'ion'

    def hull(self, b, f):
        body = b.ctx.body
        pod(b, -7, -12, -5, -8, 'L', name='pod_fl')
        pod(b, 4, -12, 6, -8, 'R', name='pod_fr')
        pod(b, -7, 7, -5, 11, 'L', name='pod_rl')
        pod(b, 4, 7, 6, 11, 'R', name='pod_rr')
        core = chamfer(-4, -11, 3, 11, tl=2, tr=2, bl=1, br=1)
        b.part(core, body, 'plate', 2, name='core', sep=True)
        b.part(chamfer(-4, -11, 3, -9, tl=2, tr=2), body, 'facet', 2.3, normal=(0.0, -0.9), name='prow', tags=('armor',))
        # Four outriggers: a hydraulic strut splayed out of each flank onto
        # a foot pad, the front pair raked forward, the rear pair back.
        for (strut, foot, lamp) in (
                ({(-5, -3), (-5, -4), (-6, -4), (-6, -5)}, rect(-7, -6, -6, -5), (-7, -6)),
                ({(-5, 2), (-5, 3), (-6, 3), (-6, 4)}, rect(-7, 4, -6, 5), (-7, 5))):
            b.part(sym(strut), STEEL, 'plate', 1.9, step=0, name='outrigger')
            b.part(foot, STEEL, 'plate', 2.0, step=1, name='foot_l')
            b.part(mirror(foot), STEEL, 'plate', 2.0, step=0, name='foot_r')
            b.lamp(sym({lamp}), 'marker', z=2.1, name='foot_lamp')
        maglev(b, 5.9, 4.9, clip=core)
        orb(b, 0.0, 8.0, 2.1, collar=body, collar_mode='inset', collar_step=0)
        b.lamp(rect(-2, -12, 1, -12), 'lamp', z=3, kind='head', name='lightbar')
        b.lamp({(-6, -12)}, 'lamp', z=3, kind='head', name='pod_lamp_l')
        b.lamp({(5, -12)}, 'lamp', z=3, kind='head', name='pod_lamp_r')
        b.lamp({(-6, 11)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(5, 11)}, 'tail', z=3, kind='tail', name='tail_r')

    def turret(self, b, pose):
        body = b.ctx.body
        # The split rails: steel outside, the charge glowing on the inner
        # faces that look at each other across the gap.
        for i, (xo, xi) in enumerate(((-4, -3), (3, 2))):
            r = self.recoil(pose, i, 2)
            b.part(rect(xo, -17 + r, xo, -5), STEEL, 'plate', 2.5, step=1 if i == 0 else -1, bevel=0,
                   tags=('barrel',), name='rail')
            b.part(rect(xi, -17 + r, xi, -15 + r) | rect(xi, -5, xi, -5), STEEL, 'plate', 2.5, step=0, bevel=0,
                   tags=('barrel',), name='rail_cap')
            b.lamp(rect(xi, -14 + r, xi, -6), 'marker', z=2.4, name='charge', tags=('barrel', 'engine'))
            for y in (-12, -8):
                b.part({(xo, y + r)}, STEEL, 'flat', 2.6, step=2, tags=('barrel',), name='rail_tie')
        mono = poly([(0, -8), (4.6, -2), (4.6, 4), (-4.6, 4), (-4.6, -2)])
        gem(b, mono, poly([(0, -4.5), (2.4, -1.5), (2.4, 2.5), (-2.4, 2.5), (-2.4, -1.5)]), body, 3, dirs=6,
            name='turret', cy=0.0, bias=0.6)
        # A slit visor, dark glass, the red eye in its middle.
        b.part(rect(-2, -3, 1, -2) - {(-2, -3), (1, -3)}, SMOKED, 'plate', 3.5, step=-1, bevel=0, name='visor',
               tags=('optic',))
        b.lamp({(-1, -2), (0, -2)}, 'sensor', z=3.7, kind='sensor', name='eye')
        b.part(rect(-1, 1, 0, 3), body, 'inset', 3.3, step=0, name='hatch', tags=('hatch',))
        b.meta['muzzles'] = [(-3.0, -16), (3.0, -16)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, 0), laser=(-6, -2), missiles=(0, 2), flame=(-6, 3),
                    plasma=[(-3.0, -16), (3.0, -16)])


# ---------------------------------------------------------------------------
# Titan - six pods, a huge reactor, twin heavy coilguns
# ---------------------------------------------------------------------------
# Concept: the super-heavy. Six pods in two ranks of three carry a broad
# armoured core: sponson plates between the pods, a heavy glacis, and a
# huge reactor glowing white through the tail between two cooling
# grilles. A big crystal hex turret with two 4 px coilguns in field
# collars, wound with white-hot bands. Silhouette: the widest block in the
# roster on six notched feet. Night: white - the reactor, the ring, the
# bands.
@LINE.design('titan')
class Titan(Design):
    codename = 'Colossus'
    blurb = 'Six pods, a huge reactor glowing between cooling grilles, twin heavy coilguns on a crystal turret.'
    locomotion = 'quad'
    marks = 'quad'
    internal_glow = 'ion'
    gun_w = 4

    def hull(self, b, f):
        body = b.ctx.body
        for i, (y0, y1) in enumerate(((-12, -7), (-3, 2), (6, 11))):
            pod(b, -11, y0, -8, y1, 'L', name='pod_l%d' % i)
            pod(b, 7, y0, 10, y1, 'R', name='pod_r%d' % i)
        core = chamfer(-7, -12, 6, 11, tl=3, tr=3, bl=2, br=2)
        b.part(core, body, 'plate', 2, name='core', sep=True)
        b.part(chamfer(-7, -12, 6, -9, tl=3, tr=3), body, 'facet', 2.3, normal=(0.0, -0.9), name='glacis')
        b.part(rect(-7, -8, 6, -8), body, 'flat', 2.2, step=-2, name='seam')
        # Sponson plates across the gaps between the pods.
        for (y0, y1) in ((-6, -4), (3, 5)):
            for (x0, nm) in ((-7, 'sponson_l'), (5, 'sponson_r')):
                b.part(rect(x0, y0, x0 + 1, y1), body, 'plate', 2.4, step=0, name=nm, tags=('armor',))
        maglev(b, 8.4, 7.4, clip=core)
        # The reactor glows through two vent slats, between two cooling
        # grilles cut into the deck.
        orb(b, 0.0, 8.0, 2.6, collar=body, collar_mode='inset', collar_step=0)
        for y in (7, 9):
            b.part(rect(-2, y, 1, y), STEEL, 'flat', 2.75, step=1, name='slat', tags=('vent',))
        for x0 in (-6, 4):
            b.grille(x0, 6, x0 + 1, 10, body, 2.4, period=2, dir='h', step=0)
        b.lamp(rect(-3, -12, 2, -12), 'lamp', z=3, kind='head', name='lightbar')
        b.lamp({(-10, -12), (-9, -12)}, 'lamp', z=3, kind='head', name='pod_lamp_l')
        b.lamp({(8, -12), (9, -12)}, 'lamp', z=3, kind='head', name='pod_lamp_r')
        b.lamp({(-10, 11)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(9, 11)}, 'tail', z=3, kind='tail', name='tail_r')

    def turret(self, b, pose):
        body = b.ctx.body
        for i, xc2 in enumerate((-10, 10)):
            r = self.recoil(pose, i, 2)
            coil_barrel(b, xc2, 4, -16, -6, r, bands=2, gap=3, start=2)
            x0 = (xc2 - 4) // 2
            # A field collar at the root, reaching outward only.
            xo = x0 - 1 if xc2 < 0 else x0
            b.part(rect(xo, -8 + r, xo + 4, -7 + r), STEEL, 'cylv', 2.7, step=0, tags=('barrel',), name='collar')
        outer = hexagon(-7, -7, 6, 6, 3)
        inner = hexagon(-4, -4, 3, 3, 2)
        gem(b, outer, inner, body, 3, dirs=6, name='turret')
        # Cheek plates and a commander's glass dome.
        b.part(rect(-7, -2, -6, 2), body, 'plate', 3.2, step=0, name='cheek_l', tags=('armor',))
        b.part(rect(5, -2, 6, 2), body, 'plate', 3.2, step=-1, name='cheek_r', tags=('armor',))
        b.dome(circle(2.5, 2.5, 1.6), GLASS, 3.5, step=1, gain=0.6, name='cupola', tags=('optic',))
        b.glass(rect(-2, -3, 1, -2), 3.5, mat=GLASS)
        b.lamp({(-1, -3), (0, -3)}, 'sensor', z=3.7, kind='sensor', name='eye')
        b.part(rect(-3, 3, -2, 4), body, 'inset', 3.3, name='hatch', tags=('hatch',))
        b.meta['muzzles'] = [(-5.0, -16), (5.0, -16)]

    def hardpoints(self, ctx):
        return dict(minigun=(8, 0), laser=(-8, -1), missiles=(-2, 4), flame=(-7, 3),
                    plasma=[(-5.0, -16), (5.0, -16)])


# ---------------------------------------------------------------------------
# Leviathan - eight pods, a spinal railgun
# ---------------------------------------------------------------------------
# Concept: the super-heavy siege platform. Eight pods, four a side, run
# down its flanks like the legs of something from the sea floor. The
# railgun is spinal: its housing runs the whole length of the round turret
# and out past the nose, the charge glowing white-hot down its channel,
# with heavy field collars along the rails. Silhouette: the longest, most
# segmented flank in the roster. Night: the white spine, the ring, the
# reactor.
@LINE.design('leviathan')
class Leviathan(Design):
    codename = 'Abyssal'
    blurb = 'Eight pods down its flanks; a spinal railgun runs the length of the turret, its charge glowing white.'
    locomotion = 'quad'
    marks = 'quad'
    internal_glow = 'ion'
    gun_w = 4

    def hull(self, b, f):
        body = b.ctx.body
        for i, (y0, y1) in enumerate(((-13, -9), (-6, -2), (1, 5), (8, 12))):
            pod(b, -10, y0, -8, y1, 'L', name='pod_l%d' % i)
            pod(b, 7, y0, 9, y1, 'R', name='pod_r%d' % i)
        core = chamfer(-7, -13, 6, 12, tl=3, tr=3, bl=2, br=2)
        b.part(core, body, 'plate', 2, name='core', sep=True)
        b.part(chamfer(-7, -13, 6, -10, tl=3, tr=3), body, 'facet', 2.3, normal=(0.0, -0.9), name='glacis')
        b.part(rect(-7, -9, 6, -9), body, 'flat', 2.2, step=-2, name='seam')
        for (x0, nm) in ((-7, 'flank_l'), (5, 'flank_r')):
            for (y0, y1) in ((-8, -6), (7, 9)):
                b.part(rect(x0, y0, x0 + 1, y1), body, 'plate', 2.4, step=0, name=nm, tags=('armor',))
        maglev(b, 8.4, 7.4, clip=core)
        orb(b, 0.0, 9.5, 2.1, collar=body, collar_mode='inset', collar_step=0)
        b.lamp(rect(-3, -13, 2, -13), 'lamp', z=3, kind='head', name='lightbar')
        b.lamp({(-9, -13)}, 'lamp', z=3, kind='head', name='pod_lamp_l')
        b.lamp({(8, -13)}, 'lamp', z=3, kind='head', name='pod_lamp_r')
        b.lamp({(-9, 12)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(8, 12)}, 'tail', z=3, kind='tail', name='tail_r')

    def turret(self, b, pose):
        body = b.ctx.body
        r = self.recoil(pose)
        # The rails and their field collars, out past the nose.
        b.part(rect(-2, -17 + r, -2, -7), STEEL, 'plate', 2.5, step=1, bevel=0, tags=('barrel',), name='rail_l')
        b.part(rect(1, -17 + r, 1, -7), STEEL, 'plate', 2.5, step=-1, bevel=0, tags=('barrel',), name='rail_r')
        b.lamp(rect(-1, -16 + r, 0, -7), 'white', z=2.4, name='channel', tags=('barrel', 'engine'))
        for y in (-15, -11):
            b.part(rect(-3, y + r, 2, y + 1 + r), STEEL, 'cylv', 2.6, step=0, tags=('barrel',), name='collar')
        b.dome(circle(0, 0, 7.0), body, 3, gain=0.5, name='shell')
        b.part(rect(-7, 3, 6, 7) & circle(0, 0, 7.0), body, 'facet', 3.1, normal=(0.0, 0.8), gain=0.6, name='skirt')
        # The spine: the gun's housing down the whole turret, glowing inside.
        b.part(rect(-2, -8, 1, 6), STEEL, 'plate', 3.3, step=-1, name='spine')
        b.lamp(rect(-1, -7, 0, 5), 'white', z=3.4, name='spine_glow')
        for y in (-4, 1):
            b.part(rect(-2, y, 1, y), STEEL, 'flat', 3.45, step=1, name='spine_rib')
        for (x0, nm) in ((-7, 'cheek_l'), (5, 'cheek_r')):
            b.part(rect(x0, -2, x0 + 1, 1), body, 'plate', 3.2, step=0, name=nm, tags=('armor',))
        b.dome(circle(-4.5, -2.5, 1.5), GLASS, 3.5, step=1, gain=0.6, name='sensor_dome', tags=('optic',))
        b.lamp({(-5, -3)}, 'sensor', z=3.7, kind='sensor', name='eye')
        b.part(rect(3, 2, 4, 3), body, 'inset', 3.3, name='hatch', tags=('hatch',))
        b.meta['muzzles'] = [(0.0, -17)]

    def hardpoints(self, ctx):
        return dict(minigun=(7, -2), laser=(-7, 1), missiles=(5, 2), flame=(-6, 4), plasma=[(0.0, -17)])
