"""SKIMMER - the anti-grav line. No tracks: a smooth hull lit from the left
(its right half a step darker, so it rounds away from the light), riding
on grav pads - a glowing lens in each rounded pod, in the chassis accent
(the team colour on a player's tank), pulsing hot, marker, dim, marker
round the hull. Rear ion thrusters flicker over a dark plenum outlet; the
turrets are low domes with a visor, canopy or lens, and slim steel rails
with energy rings the barrel recoils through. It floats: its shadow falls
further out than a tracked tank's and it leaves a wash, not tread marks.

Every lamp keeps its pixels in every frame (the damage plan keys lamps by
their pixels); animation is colour and `blink` only. Tail lights (hull)
and the sensor (turret) are drawn first, so the tier-3 warning lamp is
always one of them."""

from kit import *

LINE = Line('skimmer', 'Skimmer', 'Anti-grav: no tracks, glowing grav pads, rear thrusters.',
            notes='Smooth hover hulls on glowing grav pads; pads and ion thrusters are the animation (they pulse '
                  'and flicker instead of treads rolling). Low domed turrets with visors and canopies, slim rail '
                  'barrels with energy rings, pale shrouded weapon modules with ion seams. The shadow sits further '
                  'out, so the tank reads as floating; at night each chassis has its own light signature.')

PLENUM = Mat('plenum', [BLACK, STONE_DARKEST, STONE_SHADE, STONE_DK, STONE_MDK, STONE_MD], 2)

# The pad pulse: hot, marker, dim, marker - each lamp keeps its pixels in
# every frame (the damage plan keys lamps by their pixels), only the colour
# breathes.
_HOT = {RED_BRIGHT: GOLD_BRIGHT, TEAL_BRIGHT: BLUE_PALE, GOLD_BRIGHT: WOOD_PALE, GOLD_PALE: STONE_PALE,
        BLUE_PALE: WHITE, WHITE: WHITE, BLUE_BRIGHT: BLUE_PALE}
_DIM = {RED_BRIGHT: RED_MD, TEAL_BRIGHT: TEAL_MD, GOLD_BRIGHT: WOOD_MD, GOLD_PALE: SAND_MD,
        BLUE_PALE: BLUE_BRIGHT, WHITE: STONE_LT, BLUE_BRIGHT: BLUE_MD}
_DIM2 = {RED_BRIGHT: RED_DEEP, TEAL_BRIGHT: TEAL_DK, GOLD_BRIGHT: WOOD_AMBER, GOLD_PALE: SAND_DK,
         BLUE_PALE: BLUE_MD, WHITE: STONE_MD, BLUE_BRIGHT: BLUE_DK}
_DIM3 = {RED_BRIGHT: RED_DK, TEAL_BRIGHT: TEAL_DEEP, GOLD_BRIGHT: WOOD_DK, GOLD_PALE: WOOD_DK,
         BLUE_PALE: BLUE_DK, WHITE: STONE_MDK, BLUE_BRIGHT: BLUE_DEEP}


def pulse_role(ctx, k, dimmed=False):
    """The pad pulse for frame phase k: hot, marker, dim, marker. Dimmed
    (stealth): a step or two lower all round, never the hot flash."""
    if ctx.player:
        r = TEAM_RAMPS[ctx.team]
        if dimmed >= 2:
            seq = [r[3], r[2], r[2], r[3]]
        elif dimmed:
            seq = [r[4], r[3], r[2], r[3]]
        else:
            seq = [r[5], 'marker', r[3], 'marker']
        return seq[k % 4]
    g = ctx.glow_marker
    if dimmed >= 2:
        return [_DIM2.get(g, g), _DIM3.get(g, g), _DIM3.get(g, g), _DIM2.get(g, g)][k % 4]
    if dimmed:
        return [_DIM.get(g, g), _DIM2.get(g, g), _DIM2.get(g, g), _DIM.get(g, g)][k % 4]
    return [_HOT.get(g, WHITE), 'marker', _DIM.get(g, g), 'marker'][k % 4]


def ring_dim(ctx):
    """One step below the marker glow: the resting colour of a light that
    flares to `marker`."""
    return TEAM_BASE[ctx.team] if ctx.player else _DIM.get(ctx.glow_marker, ctx.glow_marker)


def lens(b, px, f, phase=0, z=2.0, name=None, light=True, dimmed=False):
    """A pad's glowing lens (pulsing) and its light for the night pass."""
    px = set(px)
    p = b.lamp(px, pulse_role(b.ctx, f + phase, dimmed), z=z, tags=('pad',), name=name)
    if light:
        x0, y0, x1, y1 = bbox(px)
        b.light(kind='marker', role='marker', x=(x0 + x1 + 1) / 2.0, y=(y0 + y1 + 1) / 2.0, dir=0.0, cone=None,
                reach=None, name=name)
    return p


def pad(b, cx, cy, f, phase=0, z=1.6, housing=None, name=None, pod=True, gain=0.35, step=0, dimmed=False,
        r=2.1, lens_r=1.0):
    """A grav pad: a rounded pod (4 x 4, body paint unless `housing`) with a
    2 x 2 lens set in it. (cx, cy) is the centre, on a pixel corner."""
    if pod:
        b.part(circle(cx, cy, r), housing or b.ctx.body, 'dome', z, step=step, gain=gain, tags=('pad',),
               name=(name or 'pad') + '_pod')
    lens(b, circle(cx, cy, lens_r), f, phase, z=z + 0.2, name=name, dimmed=dimmed)


def thruster(b, x0, y0, w, f, phase=0, z=2.6, name='thruster', flame=1, bell=2):
    """A rear thruster: a steel bell (w x `bell`) and an ion flame - a
    steady white-blue core (`flame` rows) and a bluer tail that flickers
    (constant pixels, blinking)."""
    b.part(rect(x0, y0, x0 + w - 1, y0 + bell - 1), STEEL, 'cylv', z, step=-1, bevel=0, tags=('exhaust',), name=name)
    b.part(rect(x0, y0 + bell - 1, x0 + w - 1, y0 + bell - 1), DARK, 'flat', z + 0.05, step=1, tags=('exhaust',),
           contact=False)
    yf = y0 + bell
    b.lamp(rect(x0, yf, x0 + w - 1, yf + flame - 1), 'ion', z=z + 0.1, tags=('engine',), name=name + '_flame')
    on = {(phase + 0) % 4, (phase + 1) % 4, (phase + 3) % 4}
    inset = 1 if w >= 3 else 0
    tip = rect(x0 + inset, yf + flame, x0 + w - 1 - inset, yf + flame)
    b.lamp(tip, BLUE_BRIGHT, z=z + 0.1, blink=on, tags=('engine',), name=name + '_tip')


def rail(b, xc2, width, y_tip, y_base, recoil, rings=(), z=2.5, ring_role='marker', name='rail', groove=None):
    """A slim rail barrel (steel) with energy rings that stay put on the
    turret while the barrel recoils through them. A 4-px rail is split by a
    dark groove down its middle (two rails, the charge between them)."""
    parts = b.barrel(xc2, width, y_tip, y_base, mat=STEEL, z=z, recoil=recoil, name=name,
                     bore=not (groove or width >= 4))
    x0 = (xc2 - width) // 2
    if groove or width >= 4:
        g = groove or (y_tip + recoil, y_base - 1)
        b.part(rect(x0 + 1, g[0], x0 + width - 2, g[1]), STEEL, 'flat', z + 0.1, step=-3, tags=('barrel', 'bore'),
               contact=False, cast=False, name=name + '_groove')
    for i, y in enumerate(rings):
        b.lamp(rect(x0, y, x0 + width - 1, y), ring_role, z=z + 0.2, name='%s_ring%d' % (name, i))
    return parts


def split_map(px, split=-1, bevel=1, corner=True, axis=0):
    """Plate bevels plus a darker right half (x >= 0): a hull top that
    rounds away from the light."""
    off = bevel_offsets(px, bevel, corner) if bevel else {q: 0 for q in px}
    return {q: off[q] + (split if q[axis] >= 0 else 0) for q in px}


def hullplate(b, px, z, split=-1, step=0, bevel=1, mat=None, corner=True, **kw):
    """A body-paint plate, lit from the left: its right half one step down."""
    px = set(px)
    return b.part(px, mat or b.ctx.body, 'map', z, step=step, stepmap=split_map(px, split, bevel, corner), **kw)


def race(b, r, z=2.05, cy=0.0):
    """The turret race: a thin groove in the deck where the turret turns.
    The turret hides it at rest; it shows when the turret swings round."""
    px = ring(0.0, cy, r, r - 1.0)
    deck = set()
    for p in b.parts:
        if p.mode != 'glow' and p.mat is b.ctx.body and p.z <= z:
            deck |= p.px
    px &= deck
    b.part(px, b.ctx.body, 'map', z, stepmap={q: (-2 if q[0] >= 0 else -1) for q in px}, name='race',
           tags=('ring',), contact=False, cast=False)


def draw(b, rows, x0, y0, legend, mirror_x=True):
    """Paint ASCII rows (the left half when mirrored). A legend entry is a
    part spec - mat, mode, z, step, tags, name - or a glow (glow=role,
    kind=). With `split` (and optionally `normal`, `bevel`, `gain`) the part
    is shaded as a body plate lit from the left: bevels, the facet's slope,
    and the right half `split` steps darker. Parts keep first-seen order."""
    groups = {}
    order = []
    for j, row in enumerate(rows):
        for i, ch in enumerate(row):
            if ch in '. ':
                continue
            if ch not in groups:
                groups[ch] = set()
                order.append(ch)
            groups[ch].add((x0 + i, y0 + j))
    made = {}
    for ch in order:
        spec = dict(legend[ch])
        px = groups[ch]
        full = sym(px) if mirror_x else set(px)
        if 'glow' in spec:
            role = spec.pop('glow')
            made[ch] = b.lamp(full, role, z=spec.pop('z', 5), kind=spec.pop('kind', None), dir=spec.pop('dir', 0.0),
                              name=spec.pop('name', None), blink=spec.pop('blink', None), tags=spec.pop('tags', None))
            continue
        mat = spec.pop('mat')
        mode = spec.pop('mode', 'plate')
        z = spec.pop('z', 0)
        halves = spec.pop('halves', False) and mirror_x
        if 'split' in spec:
            split = spec.pop('split')
            bevel = spec.pop('bevel', 1)
            corner = spec.pop('corner', True)
            nx, ny = spec.pop('normal', (0.0, 0.0))
            gain = spec.pop('gain', 1.0)
            m = split_map(full, split, bevel, corner)
            if nx or ny:
                sl = light_steps(nx, ny, 1.0, gain)
                slr = light_steps(-nx, ny, 1.0, gain)
                m = {q: o + (sl if q[0] < 0 else slr) for q, o in m.items()}
            if halves:
                # Two plates (left, right) that come off one at a time, shaded
                # as the whole shape.
                name = spec.pop('name', None)
                made[ch] = b.part(px, mat, 'map', z, stepmap={q: m[q] for q in px}, name=(name or ch) + '_l', **spec)
                b.part(mirror(px), mat, 'map', z, stepmap={q: m[q] for q in mirror(px)}, name=(name or ch) + '_r',
                       **spec)
            else:
                made[ch] = b.part(full, mat, 'map', z, stepmap=m, **spec)
        elif halves:
            name = spec.pop('name', None)
            nx, ny = spec.pop('normal', (0.0, 0.0))
            extra = dict(normal=(nx, ny)) if mode == 'facet' else {}
            made[ch] = b.part(px, mat, mode, z, name=(name or ch) + '_l', **dict(spec, **extra))
            extra = dict(normal=(-nx, ny)) if mode == 'facet' else {}
            b.part(mirror(px), mat, mode, z, name=(name or ch) + '_r', **dict(spec, **extra))
        elif mode == 'facet' and mirror_x and spec.get('normal', (0, 0))[0] != 0:
            nx, ny = spec.pop('normal')
            made[ch] = b.part(px, mat, 'facet', z, normal=(nx, ny), **spec)
            b.part(mirror(px), mat, 'facet', z, normal=(-nx, ny), **spec)
        else:
            made[ch] = b.part(full, mat, mode, z, **spec)
    return made


def shell(b, px, z, gain=0.55, step=0, mat=None, glint=True, name='dome', tags=(), bevel=1):
    """A domed turret shell with a two-pixel specular glint up at the top left."""
    px = set(px)
    b.dome(px, mat or b.ctx.body, z, step=step, gain=gain, name=name, tags=tags, bevel=bevel)
    if glint:
        x0, y0, x1, y1 = bbox(px)
        cx, cy = (x0 + x1 + 1) / 2.0, (y0 + y1 + 1) / 2.0
        rx, ry = (x1 + 1 - x0) / 2.0, (y1 + 1 - y0) / 2.0
        gx, gy = int(math.floor(cx - rx * 0.45)), int(math.floor(cy - ry * 0.45))
        g = {q for q in ((gx, gy), (gx + 1, gy), (gx, gy + 1)) if q in px}
        b.part(g, mat or b.ctx.body, 'flat', z + 0.05, step=step + 2, name=name + '_glint', contact=False, cast=False)


# ---------------------------------------------------------------------------
# Weapon modules: shrouded, rounded, glowing seams. Team-neutral (rendered
# once for every team), so pale shells and ion-blue seams, never the body,
# accent or marker colours.
# ---------------------------------------------------------------------------
SHELL = Mat('module_shell', [STONE_DK, STONE_MDK, STONE_MID, STONE_LT, STONE_HI, STONE_PALE, WHITE], 3)
_DIM_ION = BLUE_DK


def _small(hp):
    return hp.get('size', 'M') == 'S'


@LINE.module_fn('minigun')
def minigun(d, b, st, hp):
    """A pale capsule on the right cheek, three barrels out of its nose,
    an ion seam round its waist; state k > 0 heats barrel k."""
    hx, hy = hp['minigun']
    small = _small(hp)
    n = 4 if small else 5
    b.part(chamfer(hx - 1, hy - 1, hx + 1, hy + 3, 1, 1, 1, 1), SHELL, 'dome', 2.2, gain=0.6, name='mg_shell')
    b.lamp({(hx - 1, hy + 1), (hx, hy + 1), (hx + 1, hy + 1)}, 'ion' if st else _DIM_ION, z=2.3, name='mg_seam')
    for i, x in enumerate((hx - 1, hx, hx + 1)):
        b.part(rect(x, hy - n, x, hy - 2), STEEL, 'flat', 2.0, step=(1 if i == 0 else 0 if i == 1 else -1))
    b.part(rect(hx - 1, hy - 3, hx + 1, hy - 3), GUNMETAL, 'flat', 2.05, step=-1, name='mg_clamp')
    if st > 0:
        b.lamp({(hx - 2 + st, hy - n)}, 'hot', z=2.4, name='mg_hot')


@LINE.module_fn('missiles')
def missiles(d, b, st, hp):
    """A rounded roof pod: four tube caps in two rows (a red nose when
    loaded, dark when spent - state k = k tubes empty) either side of a
    glowing seam."""
    hx, hy = hp['missiles']
    if _small(hp):
        # A compact 4 x 4 pod: four single caps, no room for a seam.
        b.part(chamfer(hx - 2, hy - 2, hx + 1, hy + 1, 1, 1, 1, 1), SHELL, 'dome', 5, gain=0.55, name='ml_pod')
        caps = [{(hx - 1, hy - 1)}, {(hx, hy - 1)}, {(hx - 1, hy)}, {(hx, hy)}]
        for i, cap in enumerate(caps):
            b.part(cap, REDM if i >= st else DARK, 'flat', 5.3, step=1 if i >= st else -1, name='ml_cap%d' % i)
        return
    b.part(chamfer(hx - 3, hy - 2, hx + 2, hy + 2, 1, 1, 1, 1), SHELL, 'dome', 5, gain=0.55, name='ml_pod')
    b.lamp(rect(hx - 2, hy, hx + 1, hy), _DIM_ION if st >= 4 else 'ion', z=5.2, name='ml_seam')
    caps = [rect(hx - 2, hy - 1, hx - 1, hy - 1), rect(hx, hy - 1, hx + 1, hy - 1),
            rect(hx - 2, hy + 1, hx - 1, hy + 1), rect(hx, hy + 1, hx + 1, hy + 1)]
    for i, cap in enumerate(caps):
        if i >= st:
            b.part(cap, REDM, 'plate', 5.3, step=0, name='ml_nose%d' % i)
        else:
            b.part(cap, DARK, 'inset', 5.3, step=0, name='ml_empty%d' % i)


@LINE.module_fn('laser')
def laser(d, b, st, hp):
    """A slim capsule on the left cheek; its lens (at hy - 4, the beam
    leaves from hy - 5) charges red (1) and burns white behind a red flare
    (2)."""
    hx, hy = hp['laser']
    bottom = hy + (1 if _small(hp) else 2)
    b.part(chamfer(hx - 1, hy - 3, hx, bottom, 1, 1, 1, 1), SHELL, 'dome', 3, gain=0.6, name='lz_shell')
    b.lamp({(hx - 1, hy), (hx, hy)}, 'ion' if st else _DIM_ION, z=3.2, name='lz_seam')
    eye = {(hx - 1, hy - 4), (hx, hy - 4)}
    b.part(eye, GUNMETAL, 'flat', 3.05, step=-1, name='lz_bezel')
    b.lamp(eye, (RED_DK, RED_BRIGHT, 'white')[st], z=3.2, name='lz_eye')
    if st == 2:
        b.lamp({(hx - 1, hy - 5), (hx, hy - 5)}, RED_BRIGHT, z=3.3, name='lz_flare')


@LINE.module_fn('plasma')
def plasma(d, b, st, hp):
    """Two coil sleeves round each rail behind its muzzle, bulging only
    outward so twin rails keep their air; they glow when charged."""
    muz = hp['plasma']
    bw = hp.get('barrel', 2)
    twin = len(muz) > 1
    for (mx, my) in muz:
        x0 = int(round(mx - bw / 2.0))
        x1 = x0 + bw - 1
        if twin:
            out = -1 if mx < 0 else 1
            sx0, sx1 = (x0 - 1, x1) if out < 0 else (x0, x1 + 1)
        else:
            sx0, sx1 = x0 - 1, x1 + 1
        for k, y in enumerate((my + 2, my + 4)):
            b.part(rect(sx0, y, sx1, y + 1), SHELL, 'cylv', 7, step=-1, bevel=0, name='pl_sleeve%d' % k)
            b.lamp(rect(sx0, y, sx1, y), 'plasma' if st > 0 else _DIM_ION, z=7.1, name='pl_coil%d' % k)
        if st == 2:
            b.lamp(rect(x0, my, x1, my), 'white', z=7.2, name='pl_charge')


@LINE.module_fn('flame')
def flame(d, b, st, hp):
    """A rounded fuel capsule with its nozzle forward (the jet leaves from
    hy - 5); a pilot light (0-1) and a burning jet (2-3)."""
    hx, hy = hp['flame']
    b.part(chamfer(hx - 1, hy - 3, hx + 1, hy + 1, 1, 1, 1, 1), REDM, 'dome', 3, step=-1, gain=0.6, name='fl_tank')
    b.part(rect(hx - 1, hy - 1, hx + 1, hy - 1), SHELL, 'flat', 3.05, step=0, name='fl_band')
    b.part({(hx, hy - 4)}, GUNMETAL, 'flat', 3.0, step=0, name='fl_nozzle')
    if st < 2:
        b.lamp({(hx, hy - 5)}, 'hot' if st == 0 else 'fire', z=3.2, name='fl_pilot')
    else:
        b.lamp({(hx, hy - 5), (hx, hy - 6)} | ({(hx - 1, hy - 6)} if st == 2 else {(hx + 1, hy - 6)}), 'fire', z=3.2,
               name='fl_jet')


class Hover(Design):
    locomotion = 'hover'
    marks = 'wash'
    hover = 2
    internal_glow = 'ion'


# ---------------------------------------------------------------------------
# Scout - a slim dart: a needle nose, a spine exactly as wide as the
# turret (their outlines coincide, so the flanks never double up), swept
# wings at the back carrying two big pads. The turret is a glass bubble
# canopy with a thin rail under its chin.
# Light signature: two headlamps in a dark-glass V, two red wing pads, one
# blue flame, the red HUD glow in the canopy.
# ---------------------------------------------------------------------------
@LINE.design('scout')
class Scout(Hover):
    codename = 'Swift'
    blurb = 'A needle dart with swept wings on two big pads; the turret is a glass bubble with a thin rail under its chin.'

    def hull(self, b, f):
        body = b.ctx.body
        b.lamp({(-5, 8)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(4, 8)}, 'tail', z=3, kind='tail', name='tail_r')
        L = dict(
            N=dict(mat=body, z=2.3, split=-1, normal=(-0.2, -0.5), name='nose', tags=('armor',)),
            h=dict(mat=SMOKED, mode='flat', z=2.35, step=0, name='lamp_glass'),
            D=dict(mat=body, z=2.0, split=-1, step=1, name='spine'),
            W=dict(mat=body, z=2.2, split=-1, step=1, normal=(-0.3, 0.2), name='wing', tags=('armor', 'skirt'),
                   halves=True),
            T=dict(mat=body, z=2.1, split=-1, step=1, normal=(0.0, 0.3), name='tail'),
        )
        draw(b, [
            '.....N',
            '....hN',
            '...hNN',
            '..hNNN',
            '..NNNN',
            '..DDDD',
            '..DDDD',
            '..DDDD',
            '..DDDD',
            '.WDDDD',
            'WWDDDD',
            'WWWDDD',
            'WWWDDD',
            'WWWTTT',
            'WWWTTT',
            'WWW...',
            '.WW...',
        ], -6, -8, L)
        race(b, 4.0)
        for (cx, cy), ph, nm in zip(((-4, 6), (4, 6)), (0, 2), ('pad_l', 'pad_r')):
            pad(b, cx, cy, f, ph, z=2.4, name=nm)
        thruster(b, -1, 5, 2, f, name='thr')
        b.lamp({(-3, -6)}, 'lamp', z=3, kind='head', name='head_l')
        b.lamp({(2, -6)}, 'lamp', z=3, kind='head', name='head_r')

    def turret(self, b, pose):
        body = b.ctx.body
        b.lamp({(-1, -1), (0, -1)}, 'sensor', z=4, kind='sensor', name='hud')
        rail(b, 0, 2, -14, -4, self.recoil(pose), rings=(-7,), name='rail')
        shell(b, circle(0, 0, 4.0), 3.0, glint=False)
        b.part(rect(-4, 1, -3, 2) | rect(2, 1, 3, 2), body, 'plate', 3.05, step=-1, name='cheek', tags=('armor',))
        b.dome(ellipse(0, -1.0, 3.0, 2.6), GLASS, 3.3, gain=0.6, name='canopy', tags=('optic',))
        b.part({(-2, -3), (-1, -3)}, GLASS, 'flat', 3.4, step=4, name='canopy_glint', contact=False, cast=False,
               tags=('optic',))
        b.part(rect(-1, 2, 0, 2), body, 'plate', 3.1, step=0, name='hatch', tags=('hatch',))
        b.meta['muzzles'] = [(0.0, -14)]

    def hardpoints(self, ctx):
        return dict(minigun=(5, -1), laser=(-5, -2), missiles=(0, 3), flame=(-5, 6), plasma=[(0.0, -14)], size='S', barrel=2)


# ---------------------------------------------------------------------------
# Assault - the line's workhorse: a rounded hull on four corner pods, a
# canopy between the rails, teal chine lights down the flanks, two ion
# thrusters; twin rails on a rounded box turret with a visor band.
# Light signature: four teal pads and two chine strips - a teal ring.
# ---------------------------------------------------------------------------
@LINE.design('assault')
class Assault(Hover):
    codename = 'Kestrel'
    blurb = 'A twin-rail hover gunship: a canopy between the rails, teal chine lights, four pads, two ion thrusters.'

    def hull(self, b, f):
        body = b.ctx.body
        b.lamp({(-5, 8)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(4, 8)}, 'tail', z=3, kind='tail', name='tail_r')
        L = dict(
            N=dict(mat=body, z=2.4, split=-1, normal=(0.0, -0.9), name='nose', tags=('armor',)),
            D=dict(mat=body, z=2.0, split=-1, name='deck'),
            C=dict(mat=body, z=2.1, split=-1, normal=(-0.3, 0.0), name='chine', tags=('armor',), halves=True),
            T=dict(mat=body, z=2.1, split=-1, normal=(0.0, 0.5), name='tail', tags=('armor',)),
            g=dict(mat=GLASS, mode='glass', z=2.6, name='canopy', tags=('optic',)),
            e=dict(mat=PLENUM, mode='inset', z=2.2, step=0, name='engine_bay', tags=('vent',)),
        )
        draw(b, [
            '....NNN',
            '..NNNNN',
            '.DDNNgg',
            'DDDNNgg',
            'DDDDNgg',
            'CDDDDDD',
            'CDDDDDD',
            'CDDDDDD',
            'CDDDDDD',
            'CDDDDDD',
            'CDDDDDD',
            'CDDDDDD',
            'CDDDDDD',
            'CDDDDDD',
            'CDDDDDD',
            'DDDDDDD',
            'DDDDDDD',
            'DDTTTTT',
            '.DTeeee',
        ], -7, -10, L)
        race(b, 5.0)
        for (cx, cy), ph, nm in zip(((-5, -6), (5, -6), (5, 6), (-5, 6)), range(4), ('pad_fl', 'pad_fr', 'pad_rr', 'pad_rl')):
            pad(b, cx, cy, f, ph, z=2.3, name=nm)
        for i, x0 in enumerate((-3, 1)):
            thruster(b, x0, 7, 2, f, phase=2 * i, name='thr%d' % i)
        b.lamp(rect(-1, -10, 0, -10), 'lamp', z=3, kind='head', name='lightbar')
        b.lamp(rect(-7, -2, -7, 1), 'marker', z=2.5, name='chine_l')
        b.lamp(rect(6, -2, 6, 1), 'marker', z=2.5, name='chine_r')

    def turret(self, b, pose):
        body = b.ctx.body
        b.lamp(rect(-1, -3, 0, -3), 'sensor', z=4, kind='sensor', name='eye')
        for i, xc2 in enumerate((-6, 6)):
            rail(b, xc2, 2, -13, -4, self.recoil(pose, i, 2), rings=(-8,), name='rail%d' % i)
        b.part(chamfer(-5, -4, 4, 4, 2, 2, 2, 2), body, 'plate', 2.6, step=-1, name='skirt')
        shell(b, chamfer(-4, -4, 3, 3, 2, 2, 2, 2), 3.0)
        b.part(rect(-3, -3, 2, -3), SMOKED, 'glass', 3.3, name='visor', tags=('optic',))
        b.part(rect(-5, 0, -5, 2), body, 'plate', 2.9, step=-1, name='cheek_l', tags=('armor',))
        b.part(rect(4, 0, 4, 2), body, 'plate', 2.9, step=-1, name='cheek_r', tags=('armor',))
        b.part(rect(-2, 1, -1, 2), body, 'plate', 3.1, step=0, name='hatch', tags=('hatch',))
        b.meta['muzzles'] = [(-3.0, -13), (3.0, -13)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, -1), laser=(-6, -2), missiles=(0, 3), flame=(-6, 6), plasma=[(-3.0, -13), (3.0, -13)], barrel=2)


# ---------------------------------------------------------------------------
# Breaker - a wide ram-wedge: a raised prow block with a steel lip and a
# force-ram emitter glowing across the whole nose, flanks that taper to the
# stern, six pads (three a side), two wide thrusters; a split heavy rail
# on a hex turret with an angry chevron brow.
# Light signature: a gold bar across the front, six gold pads, red eyes.
# ---------------------------------------------------------------------------
@LINE.design('breaker')
class Breaker(Hover):
    codename = 'Hammerhead'
    blurb = 'A wide ram-wedge on six pads: a force-ram emitter glows across the whole prow; a split heavy rail.'

    def hull(self, b, f):
        body = b.ctx.body
        b.lamp({(-5, 9)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(4, 9)}, 'tail', z=3, kind='tail', name='tail_r')
        L = dict(
            R=dict(mat=STEEL, mode='plate', z=2.6, step=1, name='ram_lip'),
            N=dict(mat=body, z=2.5, split=0, normal=(-0.2, -0.9), name='prow', tags=('armor',), halves=True),
            n=dict(mat=body, z=2.5, split=0, step=-1, bevel=0, name='prow_lip'),
            D=dict(mat=body, z=2.0, split=0, name='deck'),
            A=dict(mat=body, z=2.2, split=0, normal=(-0.3, 0.0), name='flank', tags=('armor', 'skirt'), halves=True),
            T=dict(mat=body, z=2.1, split=0, normal=(0.0, 0.3), name='tail'),
            e=dict(mat=PLENUM, mode='inset', z=2.2, step=0, name='engine_bay', tags=('vent',)),
        )
        draw(b, [
            '........',
            'RRRRRRRR',
            'NNNNNNNN',
            'NNNNNNNN',
            'nnnnnnnn',
            'DDDDDDDD',
            'AADDDDDD',
            'AADDDDDD',
            'AADDDDDD',
            'AADDDDDD',
            'AADDDDDD',
            'AADDDDDD',
            'AADDDDDD',
            'AADDDDDD',
            'AADDDDDD',
            'AADDDDDD',
            'DDDDDDDD',
            '.DDDDDDD',
            '.DTTTTTT',
            '..TTTTTT',
            '..Teeeee',
        ], -8, -11, L)
        race(b, 5.0)
        b.lamp(rect(-7, -11, 6, -11), 'marker', z=2.8, kind='marker', name='ram_field')
        b.lamp({(-8, -10)}, 'lamp', z=3, kind='head', name='head_l')
        b.lamp({(7, -10)}, 'lamp', z=3, kind='head', name='head_r')
        for (cx, cy), ph, nm in zip(((-6, -8), (6, -8), (6, 6), (-6, 6)), (0, 1, 3, 4), ('pad_fl', 'pad_fr', 'pad_rr', 'pad_rl')):
            pad(b, cx, cy, f, ph, z=2.7, name=nm)
        lens(b, rect(-8, -2, -7, -1), f, 5, z=2.4, name='pad_ml')
        lens(b, rect(6, -2, 7, -1), f, 2, z=2.4, name='pad_mr')
        for i, x0 in enumerate((-4, 1)):
            thruster(b, x0, 8, 3, f, phase=2 * i, name='thr%d' % i, bell=2)

    def turret(self, b, pose):
        body = b.ctx.body
        b.lamp({(-2, -2), (1, -2)}, 'sensor', z=4, kind='sensor', name='eyes')
        rail(b, 0, 4, -12, -5, self.recoil(pose), rings=(-9,), name='rail')
        b.part(rect(-3, -6, 2, -5), STEEL, 'plate', 3.1, step=-1, name='mantlet', tags=('armor',))
        b.part(chamfer(-5, -5, 4, 4, 3, 3, 3, 3), body, 'plate', 2.7, step=0, name='skirt')
        shell(b, chamfer(-4, -4, 3, 3, 2, 2, 2, 2), 3.0, step=1, gain=0.5)
        brow = {(-4, -3), (-3, -3), (-3, -2), (-2, -2), (-1, -1), (0, -1), (1, -2), (2, -2), (2, -3), (3, -3)}
        b.part(brow, SMOKED, 'flat', 3.2, step=0, name='brow', tags=('optic',))
        b.part(rect(-5, 0, -5, 2), body, 'plate', 2.9, step=-1, name='cheek_l', tags=('armor',))
        b.part(rect(4, 0, 4, 2), body, 'plate', 2.9, step=-1, name='cheek_r', tags=('armor',))
        b.part(rect(-1, 1, 0, 2), body, 'plate', 3.1, step=0, name='hatch', tags=('hatch',))
        b.meta['muzzles'] = [(0.0, -12)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, -1), laser=(-6, -2), missiles=(0, 3), flame=(-6, 6), plasma=[(0.0, -12)], barrel=4)


# ---------------------------------------------------------------------------
# Longbow - a slender needle-hull, widest across its swept tail fins (their
# tips are the tail lights); a very long rail ringed with coils on a small
# round turret with a sniper scope.
# Light signature: a ladder of red rings, red pads fore and aft, red fins.
# ---------------------------------------------------------------------------
@LINE.design('longbow')
class Longbow(Hover):
    codename = 'Heron'
    blurb = 'A slender needle-hull with swept tail fins; a very long rail laddered with coils and a sniper scope.'

    def hull(self, b, f):
        body = b.ctx.body
        b.lamp({(-7, 9), (-7, 10)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(6, 9), (6, 10)}, 'tail', z=3, kind='tail', name='tail_r')
        L = dict(
            N=dict(mat=body, z=2.3, split=-1, step=1, normal=(0.0, -0.9), name='nose', tags=('armor',)),
            D=dict(mat=body, z=2.0, split=-1, step=1, name='deck'),
            C=dict(mat=body, z=2.1, split=-1, step=1, normal=(-0.3, 0.0), name='flank', tags=('armor',), halves=True),
            F=dict(mat=body, z=2.2, split=-1, step=1, normal=(-0.5, 0.0), name='fin', tags=('armor', 'skirt'), halves=True),
            T=dict(mat=body, z=2.1, split=-1, step=1, normal=(0.0, 0.5), name='tail'),
            e=dict(mat=PLENUM, mode='inset', z=2.2, step=0, name='engine_bay', tags=('vent',)),
        )
        draw(b, [
            '.....NN',
            '....NNN',
            '...NNNN',
            '...NNNN',
            '..NNNNN',
            '.DDDDDD',
            '.CDDDDD',
            '.CDDDDD',
            '.CDDDDD',
            '.CDDDDD',
            '.CDDDDD',
            '.CDDDDD',
            '.CDDDDD',
            '.CDDDDD',
            '.CDDDDD',
            '.DDDDDD',
            'FDDDDDD',
            'FFDDDDD',
            'FFFDDDD',
            'FFFDDDD',
            'FFF.TTT',
            'FF..TTT',
            'F...eee',
            '.......',
        ], -7, -12, L)
        race(b, 4.0)
        lens(b, rect(-6, -6, -5, -5), f, 0, z=2.4, name='pad_fl')
        lens(b, rect(4, -6, 5, -5), f, 1, z=2.4, name='pad_fr')
        for (cx, cy), ph, nm in zip(((5, 6), (-5, 6)), (2, 3), ('pad_rr', 'pad_rl')):
            pad(b, cx, cy, f, ph, z=2.4, name=nm)
        for i, x0 in enumerate((-3, 1)):
            thruster(b, x0, 8, 2, f, phase=2 * i, name='thr%d' % i)
        b.lamp({(-3, -10), (2, -10)}, 'lamp', z=3, kind='head', name='head')

    def turret(self, b, pose):
        body = b.ctx.body
        b.lamp({(-4, -5)}, 'sensor', z=4, kind='sensor', name='scope_eye')
        rail(b, 0, 2, -17, -4, self.recoil(pose), rings=(-14, -11, -8), name='rail')
        b.part(circle(0, 0, 4.0), body, 'plate', 2.7, step=0, name='skirt')
        shell(b, circle(0, 0, 3.3), 3.0, step=1)
        b.part(rect(-4, -4, -4, -1), GLASS, 'glass', 3.3, name='scope', tags=('optic',))
        b.part(rect(-4, 0, -4, 0), STEEL, 'plate', 3.2, step=-1, name='scope_mount')
        b.part(rect(1, 1, 2, 2), body, 'plate', 3.1, step=1, name='hatch', tags=('hatch',))
        b.part(rect(3, -1, 3, 1), body, 'plate', 2.9, step=0, name='cheek', tags=('armor',))
        b.meta['muzzles'] = [(0.0, -17)]

    def hardpoints(self, ctx):
        return dict(minigun=(5, -1), laser=(-6, -1), missiles=(0, 3), flame=(-5, 6), plasma=[(0.0, -17)], size='S', barrel=2)


# ---------------------------------------------------------------------------
# Flak - a saucer riding a single grav ring round its rim: eight arcs, a
# bright pair chasing round frame by frame. Twin short pulse cannons with
# coils on a hex turret carrying a glass sensor dome.
# Light signature: a pale-gold halo - the only round light in the line.
# ---------------------------------------------------------------------------
@LINE.design('flak')
class Flak(Hover):
    codename = 'Discus'
    blurb = 'A saucer riding one glowing grav ring whose lights chase round its rim; twin pulse cannons, a sensor dome.'

    def hull(self, b, f):
        body = b.ctx.body
        b.lamp({(-4, 6)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(3, 6)}, 'tail', z=3, kind='tail', name='tail_r')
        disc = ellipse(0, -0.5, 7.0, 7.9)
        rimmap = split_map(disc, -1, 1)
        for nm, sel in (('rim_fl', lambda q: q[0] < 0 and q[1] < -1), ('rim_fr', lambda q: q[0] >= 0 and q[1] < -1),
                        ('rim_rl', lambda q: q[0] < 0 and q[1] >= -1), ('rim_rr', lambda q: q[0] >= 0 and q[1] >= -1)):
            quarter = {q for q in disc if sel(q)}
            b.part(quarter, body, 'map', 1.9, step=-1, stepmap={q: rimmap[q] for q in quarter}, name=nm,
                   tags=('armor',))
        deck = ellipse(0, -0.5, 5.6, 6.5)
        hullplate(b, deck, 2.1, name='deck')
        race(b, 5.0)
        # The grav ring: the saucer's rim glows all round, eight arcs with a
        # bright pair chasing round it (constant pixels, only the colour
        # moves).
        rim = disc - ellipse(0, -0.5, 6.1, 7.0)
        for i in range(8):
            a0 = -math.pi + i * math.pi / 4
            arc = {q for q in rim if a0 <= math.atan2(q[1] + 0.5 + 0.5, q[0] + 0.5) < a0 + math.pi / 4}
            role = 'marker' if (i - f) % 4 == 0 else ring_dim(b.ctx)
            if arc:
                b.lamp(arc, role, z=2.2, tags=('pad',), name='ring%d' % i)
        b.light(kind='marker', role='marker', x=0.0, y=-0.5, dir=0.0, cone=None, reach=None, name='grav_ring')
        thruster(b, -1, 5, 2, f, name='thr', bell=2)
        b.lamp(rect(-1, -7, 0, -7), 'lamp', z=3, kind='head', name='lightbar')

    def turret(self, b, pose):
        body = b.ctx.body
        b.lamp({(-1, 1), (0, 1)}, 'sensor', z=4.2, kind='sensor', name='eye')
        for i, xc2 in enumerate((-6, 6)):
            rail(b, xc2, 2, -10, -4, self.recoil(pose, i, 2), rings=(), name='gun%d' % i)
            x0 = (xc2 - 2) // 2
            coil = rect(x0 - 1, -8, x0 + 1, -7) if xc2 < 0 else rect(x0, -8, x0 + 2, -7)
            b.part(coil, GUNMETAL, 'cylv', 2.6, step=0, bevel=0, name='coil%d' % i)
            b.lamp({q for q in coil if q[1] == -8}, 'marker', z=2.7, name='pulse%d' % i)
        b.part(chamfer(-5, -4, 4, 4, 3, 3, 3, 3), body, 'plate', 2.7, step=-1, name='skirt')
        shell(b, chamfer(-4, -4, 3, 3, 2, 2, 2, 2), 3.0)
        b.dome(circle(0, 1.0, 2.2), GLASS, 3.4, gain=0.6, name='sensor_dome', tags=('optic',))
        b.part({(-2, -1), (-1, -1)}, GLASS, 'flat', 3.5, step=4, name='dome_glint', contact=False, cast=False,
               tags=('optic',))
        b.part(rect(-5, -1, -5, 1), body, 'plate', 2.9, step=-1, name='cheek_l', tags=('armor',))
        b.part(rect(4, -1, 4, 1), body, 'plate', 2.9, step=-1, name='cheek_r', tags=('armor',))
        b.meta['muzzles'] = [(-3.0, -10), (3.0, -10)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, -1), laser=(-6, -2), missiles=(0, 3), flame=(-6, 6), plasma=[(-3.0, -10), (3.0, -10)], barrel=2)


# ---------------------------------------------------------------------------
# Wraith - a stealth arrowhead: hard flat planes (no bevels), dark glass
# inlays, pale chines along the leading edges, a sawtooth tail, pads
# dimmed two steps; a faceted wedge turret with a slit eye.
# Light signature: almost none - four dim slits, a red slit, a faint flame.
# ---------------------------------------------------------------------------
@LINE.design('wraith')
class Wraith(Hover):
    codename = 'Shade'
    blurb = 'A faceted stealth arrowhead: hard planes, dark glass, pale chines, a sawtooth tail and dimmed pads.'

    def hull(self, b, f):
        body = b.ctx.body
        acc = b.ctx.accent
        b.lamp({(-5, 6)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(4, 6)}, 'tail', z=3, kind='tail', name='tail_r')
        L = dict(
            a=dict(mat=body, z=2.2, split=-2, step=1, bevel=0, name='bow'),
            c=dict(mat=acc, mode='flat', z=2.3, step=0, name='chine'),
            b=dict(mat=body, z=2.1, split=-2, step=0, bevel=0, name='wing', tags=('armor',), halves=True),
            G=dict(mat=SMOKED, mode='glass', z=2.3, name='glass', tags=('optic',)),
            r=dict(mat=body, z=2.1, split=-2, step=-1, bevel=0, name='quarter', tags=('armor',), halves=True),
        )
        draw(b, [
            '.....c',
            '....ca',
            '...caa',
            '..caGG',
            '.caaGG',
            'caaaaa',
            'bbbbbb',
            'bbbbbb',
            'bbbbbb',
            'bbbbbb',
            'bbbbbb',
            'bGGbbb',
            'rGGrrr',
            'rrrrrr',
            '.rr.rr',
            '..r..r',
        ], -6, -8, L)
        race(b, 4.0)
        for (x, y), ph, nm in zip(((-6, 3), (5, 3), (5, -2), (-6, -2)), range(4), ('pad_rl', 'pad_rr', 'pad_fr', 'pad_fl')):
            lens(b, {(x, y), (x, y + 1)}, f, ph, z=2.4, name=nm, dimmed=2)
        thruster(b, -1, 4, 2, f, name='thr', bell=2)
        b.lamp({(-3, -5)}, 'lamp', z=3, kind='head', name='head_l')
        b.lamp({(2, -5)}, 'lamp', z=3, kind='head', name='head_r')

    def turret(self, b, pose):
        body = b.ctx.body
        b.lamp(rect(-2, -2, 1, -2), 'sensor', z=4, kind='sensor', name='slit')
        rail(b, 0, 2, -13, -4, self.recoil(pose), rings=(), name='gun')
        L = dict(
            W=dict(mat=body, z=3.0, split=-2, step=1, bevel=0, name='wedge'),
            k=dict(mat=SMOKED, mode='glass', z=3.1, name='top', tags=('optic',)),
            V=dict(mat=body, z=2.9, split=-2, step=-1, bevel=0, name='back'),
        )
        draw(b, [
            '...W',
            '..WW',
            '.WWW',
            'WWkk',
            'Wkkk',
            'VVVV',
            'VVVV',
            'V..V',
        ], -4, -4, L)
        b.meta['muzzles'] = [(0.0, -13)]

    def hardpoints(self, ctx):
        return dict(minigun=(5, -1), laser=(-5, -2), missiles=(0, 3), flame=(-5, 6), plasma=[(0.0, -13)], size='S', barrel=2)


# ---------------------------------------------------------------------------
# Warden - a sturdy armoured octagon: thick flank plates, four pads, a
# shield generator on the rear deck (a glowing ring round a dark core);
# the aegis - a curved shield plate with a glowing emitter edge - hugs the
# hex turret's front round a split heavy rail.
# Light signature: a teal chevron on the turret, a teal O behind it.
# ---------------------------------------------------------------------------
@LINE.design('warden')
class Warden(Hover):
    codename = 'Aegis'
    blurb = 'A sturdy armoured octagon: a glowing aegis plate curves round its turret, a shield generator rides its back.'

    def hull(self, b, f):
        body = b.ctx.body
        b.lamp({(-4, 9)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(3, 9)}, 'tail', z=3, kind='tail', name='tail_r')
        L = dict(
            N=dict(mat=body, z=2.3, split=-1, normal=(0.0, -0.9), name='brow', tags=('armor',), halves=True),
            D=dict(mat=body, z=2.0, split=-1, name='deck'),
            A=dict(mat=body, z=2.4, split=-1, normal=(-0.3, 0.0), name='flank', tags=('armor',), halves=True),
            T=dict(mat=body, z=2.1, split=-1, normal=(0.0, 0.5), name='tail'),
        )
        draw(b, [
            '...NNNN',
            '..NNNNN',
            '.NNNNNN',
            'DDDDDDD',
            'DDDDDDD',
            'AADDDDD',
            'AADDDDD',
            'AADDDDD',
            'AADDDDD',
            'AADDDDD',
            'AADDDDD',
            'AADDDDD',
            'AADDDDD',
            'AADDDDD',
            'AADDDDD',
            'DDDDDDD',
            'DDDDDDD',
            '.TTTTTT',
            '..TTTTT',
            '...TTTT',
        ], -7, -10, L)
        race(b, 5.0)
        for (cx, cy), ph, nm in zip(((-5, -8), (5, -8), (5, 4), (-5, 4)), range(4), ('pad_fl', 'pad_fr', 'pad_rr', 'pad_rl')):
            pad(b, cx, cy, f, ph, z=2.5, name=nm)
        # The shield generator: a steel housing on the rear deck, its emitter
        # ring glowing round a dark core.
        b.part(circle(0, 7.0, 2.6), STEEL, 'dome', 2.6, step=0, gain=0.5, name='generator', tags=('fragile',))
        b.lamp(ring(0, 7.0, 2.0, 1.1), 'marker', z=2.7, kind='marker', name='shield_ring')
        b.part(circle(0, 7.0, 1.1), DARK, 'flat', 2.75, step=0, name='generator_core', contact=False)
        for i, x0 in enumerate((-6, 4)):
            thruster(b, x0, 7, 2, f, phase=2 * i, name='thr%d' % i, bell=2)
        b.lamp({(-3, -10), (2, -10)}, 'lamp', z=3, kind='head', name='head')

    def turret(self, b, pose):
        body = b.ctx.body
        b.lamp(rect(-1, -3, 0, -3), 'sensor', z=4, kind='sensor', name='eye')
        rail(b, 0, 4, -14, -5, self.recoil(pose), rings=(-10,), name='rail')
        b.part(chamfer(-5, -5, 4, 4, 3, 3, 3, 3), body, 'plate', 2.7, step=-1, name='skirt')
        shell(b, chamfer(-4, -4, 3, 3, 2, 2, 2, 2), 3.0)
        # The aegis: a curved shield plate hugging the turret's front, its
        # emitter edge glowing.
        arc = {q for q in ring(0, 0.5, 5.4, 4.2) if q[1] <= -1}
        b.part(arc, body, 'map', 3.2, stepmap=split_map(arc, -1, 1), step=1, name='aegis', tags=('armor',))
        b.lamp({q for q in ring(0, 0.5, 5.4, 4.6) if q[1] <= -2} - rect(-2, -8, 1, 0), 'marker', z=3.3,
               kind='marker', name='aegis_edge')
        b.part(rect(-1, 1, 0, 2), body, 'plate', 3.1, step=0, name='hatch', tags=('hatch',))
        b.meta['muzzles'] = [(0.0, -14)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, -1), laser=(-6, -2), missiles=(0, 3), flame=(-6, 6), plasma=[(0.0, -14)], barrel=4)


# ---------------------------------------------------------------------------
# Ravager - aggressive: two intake pods reach past the nose like jaws
# round the twin rails, their dark mouths forward; two big thrusters; a
# round turret with a smoked brow and red eyes, gunmetal rail collars.
# Light signature: red eyes, four red pads, two big blue flames.
# ---------------------------------------------------------------------------
@LINE.design('ravager')
class Ravager(Hover):
    codename = 'Barracuda'
    blurb = 'Forward-swept intake pods reach past the nose like jaws round its twin rails; two big thrusters.'

    def hull(self, b, f):
        body = b.ctx.body
        b.lamp({(-6, 10)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(5, 10)}, 'tail', z=3, kind='tail', name='tail_r')
        L = dict(
            I=dict(mat=body, z=2.4, split=-1, step=0, normal=(-0.3, -0.3), name='intake', tags=('armor',), halves=True),
            N=dict(mat=body, z=2.2, split=-1, normal=(0.0, -0.9), name='nose'),
            D=dict(mat=body, z=2.0, split=-1, name='deck'),
            T=dict(mat=body, z=2.1, split=-1, normal=(0.0, 0.5), name='tail'),
            e=dict(mat=PLENUM, mode='inset', z=2.2, step=0, name='engine_bay', tags=('vent',)),
        )
        draw(b, [
            'I.......',
            'II......',
            'III.....',
            'IIII..NN',
            'IIII.NNN',
            'IIII.NNN',
            'IIIIDDDD',
            'IIIDDDDD',
            '.IDDDDDD',
            'DDDDDDDD',
            'DDDDDDDD',
            'DDDDDDDD',
            'DDDDDDDD',
            'DDDDDDDD',
            'DDDDDDDD',
            'DDDDDDDD',
            'DDDDDDDD',
            'DDDDDDDD',
            '.DDDDDDD',
            '.DTTTTTT',
            '..Teeeee',
            '........',
        ], -8, -11, L)
        race(b, 5.0)
        # The intake mouths run along the swept lips; gill slits in the flanks.
        mouth = {(-7, -11), (-6, -10), (-5, -9)}
        b.part(sym(mouth), DARK, 'flat', 2.55, name='mouth', tags=('vent',))
        gills = rect(-8, 0, -7, 0) | rect(-8, 2, -7, 2)
        b.part(sym(gills), b.ctx.body, 'flat', 2.3, step=-3, name='gills', tags=('vent',))
        for (cx, cy), ph, nm in zip(((-6, -4), (6, -4), (6, 6), (-6, 6)), range(4), ('pad_fl', 'pad_fr', 'pad_rr', 'pad_rl')):
            pad(b, cx, cy, f, ph, z=2.5, name=nm, r=1.6, lens_r=0.8)
        for i, x0 in enumerate((-5, 2)):
            thruster(b, x0, 8, 3, f, phase=2 * i, name='thr%d' % i, flame=1)
        b.lamp({(-1, -8), (0, -8)}, 'lamp', z=3, kind='head', name='head')

    def turret(self, b, pose):
        body = b.ctx.body
        b.lamp({(-3, -3), (-2, -2), (1, -2), (2, -3)}, 'sensor', z=4, kind='sensor', name='visor')
        for i, xc2 in enumerate((-6, 6)):
            rail(b, xc2, 2, -14, -5, self.recoil(pose, i, 2), rings=(), name='rail%d' % i)
            x0 = (xc2 - 2) // 2
            b.part(rect(x0, -8, x0 + 1, -7), GUNMETAL, 'cylv', 2.6, step=0, bevel=0, name='collar%d' % i)
        b.part(circle(0, 0, 5.0), body, 'plate', 2.7, step=-1, name='skirt')
        shell(b, circle(0, 0, 4.0), 3.0)
        b.part({(-3, -3), (-2, -2), (-1, -2), (0, -2), (1, -2), (2, -3), (-4, -3), (3, -3)}, SMOKED, 'flat', 3.2,
               step=0, name='brow', tags=('optic',))
        b.part(rect(-5, -1, -5, 1), body, 'plate', 2.9, step=-1, name='cheek_l', tags=('armor',))
        b.part(rect(4, -1, 4, 1), body, 'plate', 2.9, step=-1, name='cheek_r', tags=('armor',))
        b.part(rect(-1, 1, 0, 2), body, 'plate', 3.1, step=0, name='hatch', tags=('hatch',))
        b.meta['muzzles'] = [(-3.0, -14), (3.0, -14)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, -1), laser=(-6, -2), missiles=(0, 3), flame=(-6, 6), plasma=[(-3.0, -14), (3.0, -14)], barrel=2)


# ---------------------------------------------------------------------------
# Glacier - compact and clean: an ice-crystal hexagon, white-edged at the
# bow, frost vents down the flanks, four pale pads in dark pods; a neat
# box turret with a white stripe and a split visor.
# Light signature: four ice-white pads, a white lightbar.
# ---------------------------------------------------------------------------
@LINE.design('glacier')
class Glacier(Hover):
    codename = 'Floe'
    blurb = 'A compact ice-crystal hexagon, white-edged, with frost vents in its flanks; four ice-white pads.'

    def hull(self, b, f):
        body = b.ctx.body
        acc = b.ctx.accent
        b.lamp({(-3, 7)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(2, 7)}, 'tail', z=3, kind='tail', name='tail_r')
        L = dict(
            W=dict(mat=acc, mode='plate', z=2.3, step=0, name='trim'),
            N=dict(mat=body, z=2.2, split=-1, normal=(-0.3, -0.6), name='bow', tags=('armor',), halves=True),
            D=dict(mat=body, z=2.0, split=-1, name='deck'),
            T=dict(mat=body, z=2.1, split=-1, normal=(0.0, 0.6), name='stern', tags=('armor',), halves=True),
            v=dict(mat=acc, mode='grille', z=2.2, step=-1, pattern=dict(period=2, dir='h'), name='vent', tags=('vent',)),
        )
        draw(b, [
            '...WWWW',
            '..WNNNN',
            '.WNNNNN',
            'WNNNNNN',
            'DDDDDDD',
            'vDDDDDD',
            'vDDDDDD',
            'vDDDDDD',
            'vDDDDDD',
            'vDDDDDD',
            'DDDDDDD',
            'TTTTTTT',
            '.TTTTTT',
            '..TTTTT',
            '...TTTT',
        ], -7, -7, L)
        race(b, 5.0)
        for (cx, cy), ph, nm in zip(((-5, -3), (5, -3), (5, 4), (-5, 4)), range(4), ('pad_fl', 'pad_fr', 'pad_rr', 'pad_rl')):
            pad(b, cx, cy, f, ph, z=2.3, name=nm, step=-2)
        thruster(b, -1, 5, 2, f, name='thr')
        b.lamp(rect(-3, -7, 2, -7) - rect(-1, -7, 0, -7), 'lamp', z=3, kind='head', name='lightbar')

    def turret(self, b, pose):
        body = b.ctx.body
        b.lamp(rect(-1, -3, 0, -3), 'sensor', z=4, kind='sensor', name='eye')
        rail(b, 0, 2, -14, -4, self.recoil(pose), rings=(-9,), name='rail')
        b.part(chamfer(-5, -4, 4, 4, 1, 1, 1, 1), body, 'plate', 2.7, step=-1, name='skirt')
        shell(b, chamfer(-4, -4, 3, 3, 1, 1, 1, 1), 3.0)
        b.part(rect(-4, 1, 3, 1), b.ctx.accent, 'flat', 3.1, step=0, name='stripe')
        b.part(rect(-3, -3, -2, -3) | rect(1, -3, 2, -3), SMOKED, 'glass', 3.2, name='visor', tags=('optic',))
        b.part(rect(-5, -2, -5, 0), body, 'plate', 2.9, step=-1, name='cheek_l', tags=('armor',))
        b.part(rect(4, -2, 4, 0), body, 'plate', 2.9, step=-1, name='cheek_r', tags=('armor',))
        b.part(rect(-1, 2, 0, 3), body, 'plate', 3.1, step=0, name='hatch', tags=('hatch',))
        b.meta['muzzles'] = [(0.0, -14)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, -1), laser=(-6, -2), missiles=(0, 3), flame=(-6, 6), plasma=[(0.0, -14)], barrel=2)


# ---------------------------------------------------------------------------
# Obelisk - a monolith: a long dark slab with a pyramidion nose whose
# capstone is gilded in the accent, small swept feet at the back; twin long
# rails on a faceted wedge turret.
# Light signature: two ladders of gold rings, four gold pads.
# ---------------------------------------------------------------------------
@LINE.design('obelisk')
class Obelisk(Hover):
    codename = 'Spire'
    blurb = 'A dark monolith with a gilded pyramid nose and swept feet; twin long rails laddered with gold rings.'

    def hull(self, b, f):
        body = b.ctx.body
        acc = b.ctx.accent
        b.lamp({(-4, 10)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(3, 10)}, 'tail', z=3, kind='tail', name='tail_r')
        L = dict(
            G=dict(mat=acc, mode='facet', z=2.4, normal=(-0.5, -0.5), name='capstone'),
            P=dict(mat=body, mode='facet', z=2.3, normal=(-0.5, -0.5), step=1, name='pyramid', tags=('armor',), halves=True),
            D=dict(mat=body, z=2.0, split=-1, step=1, name='deck'),
            S=dict(mat=body, z=2.1, split=-1, step=1, normal=(-0.3, 0.0), name='slab', tags=('armor', 'skirt'), halves=True),
            F=dict(mat=body, z=2.2, split=-1, step=1, normal=(-0.5, 0.0), name='foot', tags=('armor',), halves=True),
            T=dict(mat=body, z=2.1, split=-1, step=1, normal=(0.0, 0.5), name='tail'),
            e=dict(mat=PLENUM, mode='inset', z=2.2, step=0, name='engine_bay', tags=('vent',)),
        )
        draw(b, [
            '......G',
            '.....GG',
            '....GPP',
            '...PPPP',
            '..PPPPP',
            '.PPPPPP',
            'SDDDDDD',
            'SDDDDDD',
            'SDDDDDD',
            'SDDDDDD',
            'SDDDDDD',
            'SDDDDDD',
            'SDDDDDD',
            'SDDDDDD',
            'SDDDDDD',
            'SDDDDDD',
            'SDDDDDD',
            'SDDDDDD',
            'SDDDDDD',
            'FDDDDDD',
            'FFDTTTT',
            'FFDTTTT',
            'FF.eeee',
            'F......',
        ], -7, -12, L)
        race(b, 5.0)
        for (cx, cy), ph, nm in zip(((-5, -5), (5, -5), (5, 6), (-5, 6)), range(4), ('pad_fl', 'pad_fr', 'pad_rr', 'pad_rl')):
            pad(b, cx, cy, f, ph, z=2.3, name=nm)
        for i, x0 in enumerate((-3, 1)):
            thruster(b, x0, 8, 2, f, phase=2 * i, name='thr%d' % i)
        b.lamp({(-3, -8), (2, -8)}, 'lamp', z=3, kind='head', name='head')

    def turret(self, b, pose):
        body = b.ctx.body
        b.lamp(rect(-1, -3, 0, -3), 'sensor', z=4, kind='sensor', name='eye')
        for i, xc2 in enumerate((-6, 6)):
            rail(b, xc2, 2, -16, -5, self.recoil(pose, i, 2), rings=(-13, -10, -7), name='rail%d' % i)
        L = dict(
            W=dict(mat=body, mode='facet', z=3.0, normal=(-0.4, -0.4), step=1, name='wedge'),
            V=dict(mat=body, mode='facet', z=2.9, normal=(-0.2, 0.5), step=1, name='back'),
        )
        draw(b, [
            '...WW',
            '..WWW',
            '.WWWW',
            'WWWWW',
            'WWWWW',
            'VVVVV',
            'VVVVV',
            '.VVVV',
        ], -5, -4, L)
        b.part(rect(-1, 1, 0, 2), body, 'plate', 3.1, step=1, name='hatch', tags=('hatch',))
        b.meta['muzzles'] = [(-3.0, -16), (3.0, -16)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, -1), laser=(-6, -2), missiles=(0, 3), flame=(-6, 6), plasma=[(-3.0, -16), (3.0, -16)], barrel=2)


# ---------------------------------------------------------------------------
# Titan - a hover fortress: a central keep between four corner bastions,
# each riding two pads (eight in all); three thrusters; twin massive split
# rails on a wide hex turret.
# Light signature: eight white pads round the corners, three flames.
# ---------------------------------------------------------------------------
@LINE.design('titan')
class Titan(Hover):
    codename = 'Citadel'
    blurb = 'A hover fortress: four corner bastions on eight pads round a central keep; twin massive split rails.'

    def hull(self, b, f):
        body = b.ctx.body
        acc = b.ctx.accent
        b.lamp({(-8, 11)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(7, 11)}, 'tail', z=3, kind='tail', name='tail_r')
        L = dict(
            s=dict(mat=PLENUM, mode='plate', z=1.5, name='skirt', tags=('skirt',)),
            K=dict(mat=body, z=2.0, split=-1, name='keep'),
            G=dict(mat=body, z=2.2, split=-1, normal=(0.0, -0.9), name='glacis'),
            B=dict(mat=body, z=2.4, split=-1, name='bastion_f', tags=('armor',), halves=True),
            b=dict(mat=body, z=2.4, split=-1, name='bastion_r', tags=('armor', 'skirt'), halves=True),
            c=dict(mat=acc, mode='plate', z=2.5, step=0, name='cap'),
            T=dict(mat=body, z=2.1, split=-1, normal=(0.0, 0.5), name='stern'),
            e=dict(mat=PLENUM, mode='inset', z=2.2, step=0, name='engine_bay', tags=('vent',)),
        )
        draw(b, [
            '..ccccGGGGG',
            '.BBBBBGGGGG',
            'BBBBBBGGGGG',
            'BBBBBBGGGGG',
            'BBBBBBKKKKK',
            'BBBBBBKKKKK',
            '.BBBBKKKKKK',
            '.sKKKKKKKKK',
            '.sKKKKKKKKK',
            '.sKKKKKKKKK',
            '.sKKKKKKKKK',
            '.sKKKKKKKKK',
            '.sKKKKKKKKK',
            '.sKKKKKKKKK',
            '.sKKKKKKKKK',
            '.bbbbKKKKKK',
            'bbbbbbKKKKK',
            'bbbbbbKKKKK',
            'bbbbbbTTTTT',
            'bbbbbbTTTTT',
            '.bbbbbTTTTT',
            '..ccccTeeee',
            '...........',
            '...........',
        ], -11, -12, L)
        race(b, 7.0)
        pads = [(-11, -10), (-11, -6), (10, -6), (10, -10), (10, 5), (10, 9), (-11, 9), (-11, 5)]
        for i, (x, y) in enumerate(pads):
            lens(b, {(x, y), (x, y + 1)}, f, i, z=2.6, name='pad%d' % i)
        for i, x0 in enumerate((-5, -1, 3)):
            thruster(b, x0, 8, 2, f, phase=i, name='thr%d' % i, bell=2)
        b.lamp({(-5, -12), (4, -12)}, 'lamp', z=3, kind='head', name='head')

    def turret(self, b, pose):
        body = b.ctx.body
        b.lamp(rect(-2, -5, 1, -5), 'sensor', z=4, kind='sensor', name='visor')
        for i, xc2 in enumerate((-10, 10)):
            rail(b, xc2, 4, -16, -7, self.recoil(pose, i, 2), rings=(-12,), name='rail%d' % i)
        for x0 in (-7, 3):
            b.part(rect(x0, -8, x0 + 3, -6), STEEL, 'plate', 2.9, step=-1, name='breech', tags=('armor',))
        b.part(chamfer(-7, -6, 6, 6, 4, 4, 4, 4), body, 'plate', 2.7, step=-1, name='skirt')
        shell(b, chamfer(-6, -5, 5, 5, 3, 3, 3, 3), 3.0)
        b.dome(circle(0, 1.0, 2.1), GLASS, 3.3, gain=0.6, name='cupola', tags=('optic',))
        b.part(rect(-7, -1, -7, 2), body, 'plate', 2.9, step=-1, name='cheek_l', tags=('armor',))
        b.part(rect(6, -1, 6, 2), body, 'plate', 2.9, step=-1, name='cheek_r', tags=('armor',))
        b.part(rect(-4, 3, -3, 4), body, 'plate', 3.1, step=0, name='hatch', tags=('hatch',))
        b.meta['muzzles'] = [(-5.0, -16), (5.0, -16)]

    def hardpoints(self, ctx):
        return dict(minigun=(8, -2), laser=(-8, -3), missiles=(0, 4), flame=(-8, 6), plasma=[(-5.0, -16), (5.0, -16)], barrel=4)


# ---------------------------------------------------------------------------
# Leviathan - a hover barge: a ship's prow with accent bow stripes, four
# pad ports down each gunwale, four thrusters across the stern; a spinal
# railgun that runs back over the big round dome, ringed all the way.
# Light signature: two rows of blue ports, a ladder of blue rings.
# ---------------------------------------------------------------------------
@LINE.design('leviathan')
class Leviathan(Hover):
    codename = 'Galleon'
    blurb = 'A long hover barge: a ship-prow bow, eight pads in its gunwales, four thrusters; a spinal railgun.'

    def hull(self, b, f):
        body = b.ctx.body
        acc = b.ctx.accent
        b.lamp({(-7, 12)}, 'tail', z=3, kind='tail', name='tail_l')
        b.lamp({(6, 12)}, 'tail', z=3, kind='tail', name='tail_r')
        L = dict(
            a=dict(mat=acc, mode='plate', z=2.4, step=0, name='bow_stripe'),
            P=dict(mat=body, z=2.3, split=-1, normal=(-0.2, -0.8), name='prow', tags=('armor',), halves=True),
            W=dict(mat=body, z=2.2, split=-1, step=-1, normal=(-0.3, 0.0), name='gunwale', tags=('armor', 'skirt'), halves=True),
            D=dict(mat=body, z=2.0, split=-1, name='deck'),
            T=dict(mat=body, z=2.1, split=-1, normal=(0.0, 0.5), name='stern'),
            e=dict(mat=PLENUM, mode='inset', z=2.2, step=0, name='engine_bay', tags=('vent',)),
        )
        draw(b, [
            '........aP',
            '.......aPP',
            '......aPPP',
            '.....aPPPP',
            '....aPPPPP',
            '...aPPPPPP',
            '..WWDDDDDD',
            '.WWWDDDDDD',
            'WWWDDDDDDD',
            'WWWDDDDDDD',
            'WWWDDDDDDD',
            'WWWDDDDDDD',
            'WWWDDDDDDD',
            'WWWDDDDDDD',
            'WWWDDDDDDD',
            'WWWDDDDDDD',
            'WWWDDDDDDD',
            'WWWDDDDDDD',
            'WWWDDDDDDD',
            'WWWDDDDDDD',
            'WWWDDDDDDD',
            'WWWDDDDDDD',
            'WWWTTTTTTT',
            '.WWTTTTTTT',
            '..WTeeeeee',
            '..........',
        ], -10, -13, L)
        race(b, 7.0)
        for i, y in enumerate((-5, -1, 3, 7)):
            sock = rect(-10, y - 1, -8, y + 2)
            b.part(sym(sock), PLENUM, 'inset', 2.4, step=0, name='port%d' % i, tags=('pad',))
            lens(b, {(-10, y), (-9, y), (-10, y + 1), (-9, y + 1)}, f, i, z=2.5, name='pad_l%d' % i)
            lens(b, {(9, y), (8, y), (9, y + 1), (8, y + 1)}, f, i + 2, z=2.5, name='pad_r%d' % i)
        for i, x0 in enumerate((-6, -3, 1, 4)):
            thruster(b, x0, 10, 2, f, phase=i, name='thr%d' % i, bell=1)
        b.lamp({(-3, -9), (2, -9)}, 'lamp', z=3, kind='head', name='head')

    def turret(self, b, pose):
        body = b.ctx.body
        b.lamp(rect(-5, -3, -4, -3) | rect(3, -3, 4, -3), 'sensor', z=4, kind='sensor', name='eyes')
        r = self.recoil(pose)
        rail(b, 0, 4, -16, -8, r, rings=(-13, -10), name='rail')
        b.part(circle(0, 0, 7.0), body, 'plate', 2.7, step=-1, name='skirt')
        shell(b, circle(0, 0, 6.0), 3.0)
        # The spinal mount: the rail runs on back over the dome to its breech,
        # ringed all the way.
        b.part(rect(-3, -7, -3, 4) | rect(2, -7, 2, 4), body, 'plate', 3.25, step=-2, name='spine_cheeks')
        b.part(rect(-2, -7, 1, 5), STEEL, 'cylv', 3.3, step=0, bevel=0, name='spine')
        b.part(rect(-1, -7, 0, 4), STEEL, 'flat', 3.35, step=-3, name='spine_slot')
        b.lamp(rect(-2, -4, 1, -4) | rect(-2, -1, 1, -1) | rect(-2, 2, 1, 2), 'marker', z=3.4, name='spine_rings')
        b.part(rect(-7, -1, -7, 1), body, 'plate', 2.9, step=-1, name='cheek_l', tags=('armor',))
        b.part(rect(6, -1, 6, 1), body, 'plate', 2.9, step=-1, name='cheek_r', tags=('armor',))
        b.part(rect(-6, 2, -5, 3), body, 'plate', 3.1, step=0, name='hatch', tags=('hatch',))
        b.meta['muzzles'] = [(0.0, -16)]

    def hardpoints(self, ctx):
        return dict(minigun=(8, -2), laser=(-8, -3), missiles=(0, 4), flame=(-8, 6), plasma=[(0.0, -16)], barrel=4)
