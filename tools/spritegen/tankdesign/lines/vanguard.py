"""VANGUARD - the refit line. The roster you know, finished properly:
tracked, bevelled armour plates with crisp panel lines, fenders over the
track ends, twin headlights in housings, red tail lights, a glowing strip in
the chassis accent, grilled engine decks, cupolas, sensor blocks, antennas
and stowage. The safest pick: every tank reads as itself at a glance."""

from kit import *

LINE = Line('vanguard', 'Vanguard', 'The refit: the roster you know, finished properly.',
            notes='Tracked. Bevelled plates, fenders, twin headlights, tail lights, an accent light strip, '
                  'grilled engine decks. Weapon modules are bolt-on military hardware.')


# ---------------------------------------------------------------------------
# The Vanguard toolkit (helpers on top of kit.py, local to this line)
# ---------------------------------------------------------------------------
def vpaint(b, rows, x0, y0, legend, mirror_x=True):
    """`Builder.paint` with two differences. A mirrored entry marked
    `split=True` becomes a left and a right part, so each side is its own
    armour plate (one fender blows off, not both). A mirrored glow with a
    `kind` registers one light per side (two headlight cones, not one in the
    middle). Facet normals flip on the right half, as in `paint`."""
    groups = {}
    for j, row in enumerate(rows):
        for i, ch in enumerate(row):
            if ch in '. ':
                continue
            groups.setdefault(ch, set()).add((x0 + i, y0 + j))
    made = {}
    for ch, px in groups.items():
        spec = dict(legend[ch])
        split = spec.pop('split', False)
        name = spec.pop('name', None)
        if 'glow' in spec:
            role = spec.pop('glow')
            kind = spec.pop('kind', None)
            z = spec.pop('z', 5)
            kw = dict(dir=spec.pop('dir', 0.0), blink=spec.pop('blink', None), tags=spec.pop('tags', None))
            if mirror_x and (kind or split):
                made[ch] = b.lamp(px, role, z=z, kind=kind, name=(name + '_l') if name else None, **kw)
                b.lamp(mirror(px), role, z=z, kind=kind, name=(name + '_r') if name else None, **kw)
            else:
                made[ch] = b.lamp(sym(px) if mirror_x else px, role, z=z, kind=kind, name=name, **kw)
            continue
        mat = spec.pop('mat')
        mode = spec.pop('mode', 'plate')
        z = spec.pop('z', 0)
        if not mirror_x:
            made[ch] = b.part(px, mat, mode, z, name=name, **spec)
            continue
        nrm = spec.get('normal')
        if split or (nrm and nrm[0] != 0):
            right = dict(spec)
            if nrm:
                right['normal'] = (-nrm[0], nrm[1])
            made[ch] = b.part(px, mat, mode, z, name=(name + '_l') if name else None, **spec)
            b.part(mirror(px), mat, mode, z, name=(name + '_r') if name else None, **right)
        else:
            made[ch] = b.part(sym(px), mat, mode, z, name=name, **spec)
    return made


def runs(b, x_out, x_in, y0, y1, z=1, style='std', period=4):
    """Both track runs: the left one from x_out to x_in, the right one its
    mirror."""
    b.tread(x_out, x_in, y0, y1, 'L', z=z, style=style, period=period, name='track_l')
    b.tread(-1 - x_in, -1 - x_out, y0, y1, 'R', z=z, style=style, period=period, name='track_r')


def turret_ring(b, r_out=4.6, r_in=3.4, cy=0.0, z=2.1, mat=None, step=-2):
    """The turret race: a flat shadow ring in the hull colour, just outside
    the turret, so a turned turret shows the gap it turns in."""
    b.part(ring(0, cy, r_out, r_in), mat or b.ctx.body, 'flat', z, step=step, tags=('ring',), name='ring',
           contact=False, cast=False)


def rim_shade(px, cx, cy, lit=1, shade=-1, deep=None, top=0):
    """A stepmap for a flat-topped round or faceted part seen from above:
    the rim that faces the light (top left) lit, the far rim shaded, the
    top flat. `deep` darkens a second row of the far rim."""
    rim = edge(px)
    inner_rim = edge(px - rim)
    m = {}
    for (x, y) in px:
        dx, dy = x + 0.5 - cx, y + 0.5 - cy
        d = math.hypot(dx, dy) or 1.0
        facing = -(dx / d * 0.57 + dy / d * 0.82)
        o = top
        if (x, y) in rim:
            o = lit if facing > 0.3 else shade if facing < -0.3 else top
        elif deep is not None and (x, y) in inner_rim and facing < -0.6:
            o = deep
        m[(x, y)] = o
    return m


def faces(b, rows, x0, y0, mat, z, steps, name='hex', tags=(), mirror_x=True, extra=None, roof_bevel=True,
          roof_name=None):
    """A faceted body from an ASCII half, every face its own flat tone:
    `steps` maps a face letter to (left step, right step) - the faces that
    look toward the light (top left) step up, the far ones down, so a hex or
    a wedge reads as cut planes rather than a blob. `R` is the roof."""
    groups = {}
    for j, row in enumerate(rows):
        for i, ch in enumerate(row):
            if ch in '. ':
                continue
            groups.setdefault(ch, set()).add((x0 + i, y0 + j))
    made = {}
    for ch, px in groups.items():
        if extra and ch in extra:
            spec = dict(extra[ch])
            m = spec.pop('mat')
            mode = spec.pop('mode', 'plate')
            zz = spec.pop('z', z)
            made[ch] = b.part(sym(px) if mirror_x else px, m, mode, zz, **spec)
            continue
        ls, rs = steps.get(ch, (0, 0))
        nm = '%s_%s' % (name, ch)
        if ch == 'R' and mirror_x and roof_bevel:
            # The roof is one raised plate: its own bevel gives the lit and
            # shaded rims against the faces round it.
            made[ch] = b.part(sym(px), mat, 'plate', z, step=ls, corner=False, name=roof_name or nm, tags=tags)
            continue
        made[ch] = b.part(px, mat, 'flat', z, step=ls, name=nm + '_l', tags=tags)
        if mirror_x:
            b.part(mirror(px), mat, 'flat', z, step=rs, name=nm + '_r', tags=tags)
    return made


# The standard face tones: front edge, front cheek, side, rear cheek, rear.
HEX_STEPS = dict(R=(0, 0), a=(0, 0), b=(1, 0), c=(0, -1), d=(-1, -1), e=(-1, -1))


def whip(b, x, y, n=3, z=5.0, dx=-1):
    """A whip antenna seen from above: a dark socket and a thin rod swept
    back (and out) from it, the tip catching the light."""
    b.part({(x, y)}, GUNMETAL, 'flat', z, step=-1, tags=('antenna',), name='whip_base', contact=False, cast=False)
    rod = {(x + dx * k, y + k) for k in range(1, n + 1)}
    b.part(rod, STEEL, 'flat', z + 0.1, step=0, tags=('antenna',), name='whip', contact=False, cast=False)
    b.part({(x + dx * n, y + n)}, STEEL, 'flat', z + 0.2, step=2, tags=('antenna',), name='whip_tip', contact=False,
           cast=False)


def livery(b, rows, x0, y0, steps, z, under=None, name='livery'):
    """Painted marks (camouflage, bands) in the body colour's own ramp: each
    character of `rows` is a step on the body ramp (`steps` maps it), so a
    player's tank carries the same marks in its team tones. Only the
    interior of `under` is painted, so the plates keep their bevels."""
    groups = {}
    for j, row in enumerate(rows):
        for i, ch in enumerate(row):
            if ch in '. ':
                continue
            groups.setdefault(ch, set()).add((x0 + i, y0 + j))
    inner = shrink(under) if under is not None else None
    for ch, px in groups.items():
        if inner is not None:
            px &= inner
        if px:
            b.part(px, b.ctx.body, 'flat', z, step=steps[ch], name='%s_%s' % (name, ch), contact=False, cast=False)


def headlamp(b, x, y, name, z=4, back=1, side=0, kind='head'):
    """A headlight in its housing: the lens on the front edge and a dark
    steel bezel behind it (and beside it with `side`), so the lamp reads as
    a fitted unit rather than a stray bright pixel."""
    hp = {(x, y + back)} | ({(x + side, y)} if side else set())
    b.part(hp, GUNMETAL, 'flat', z - 0.3, step=-1, name=name + '_housing', tags=('housing',), contact=False,
           cast=False)
    return b.lamp({(x, y)}, 'lamp', z=z, kind=kind, name=name)


def taillamp(b, x, y, name, z=4, fwd=-1):
    """A tail light: the red lens on the rear edge, its dark bezel ahead of it."""
    b.part({(x, y + fwd)}, GUNMETAL, 'flat', z - 0.3, step=-1, name=name + '_housing', tags=('housing',),
           contact=False, cast=False)
    return b.lamp({(x, y)}, 'tail', z=z, kind='tail', name=name)


def brake2(b, xc2, width, y_tip, recoil, z=2.8, mat=None):
    """A double-baffle muzzle brake: two collars a row apart at the tip."""
    mat = mat or STEEL
    x0 = (xc2 - width) // 2 - 1
    x1 = x0 + width + 1
    for dy in (0, 2):
        b.part(rect(x0, y_tip + recoil + dy, x1, y_tip + recoil + dy), mat, 'cylv', z, bevel=0, name='brake',
               tags=('barrel', 'brake'))


# ---------------------------------------------------------------------------
# Weapon modules (shared by every Vanguard chassis, placed at hardpoints):
# bolt-on military hardware in fixed materials - gunmetal housings, steel
# barrels, brass feeds, red warheads - since a module is drawn once for every
# team. Each is small and dark-framed, so it reads as added kit on the paint.
# ---------------------------------------------------------------------------
DIM_LENS = (0x81, 0x2F, 0x27)     # RED_DK: an unlit laser lens
DIM_PLASMA = (0x03, 0x8A, 0xAB)   # BLUE_DK: plasma coils at rest
PILOT = (0x27, 0xD8, 0xC5)        # BLUE_BRIGHT: the flamer's pilot flame
DIM_RAIL = (0x0E, 0x8B, 0x96)     # BLUE_DARKEST: a gauss rail's charge cells at rest


@LINE.module_fn('minigun')
def minigun(d, b, st, hp):
    """A rotary gun on the right cheek: a dark drum housing, the barrel
    cluster read as two lit barrels either side of a dark bore gap, bound by
    a bright clamp and a muzzle ring, a brass belt feeding in from the turret
    side. The barrel that just fired glows at its tip."""
    hx, hy = hp['minigun']
    b.meta['muzzle'] = (hx + 0.5, hy - 5.0)
    b.part(chamfer(hx - 1, hy, hx + 1, hy + 3, bl=1, br=1), GUNMETAL, 'plate', 3, step=-1, corner=False,
           name='mg_housing')
    b.part(rect(hx - 1, hy - 5, hx - 1, hy - 1), STEEL, 'flat', 3.2, step=1, name='mg_barrel_l')
    b.part(rect(hx, hy - 5, hx, hy - 1), DARK, 'flat', 3.2, step=1, name='mg_gap')
    b.part(rect(hx + 1, hy - 5, hx + 1, hy - 1), STEEL, 'flat', 3.2, step=-1, name='mg_barrel_r')
    b.part(rect(hx - 1, hy - 3, hx + 1, hy - 3), STEEL, 'flat', 3.3, step=2, name='mg_clamp')
    b.part(rect(hx - 1, hy - 5, hx + 1, hy - 5), STEEL, 'flat', 3.3, step=0, name='mg_muzzle')
    b.part(rect(hx - 2, hy + 1, hx - 2, hy + 2), BRASS, 'flat', 2.9, step=0, name='mg_belt')
    if st > 0:
        b.lamp({(hx - 1 + (st - 1), hy - 6)}, 'hot', z=3.5, name='mg_flash')


@LINE.module_fn('missiles')
def missiles(d, b, st, hp):
    """A four-cell launcher on the roof: a bevelled gunmetal box, each
    loaded cell a red warhead lit on its top left, each fired cell a black
    tube. `st` cells are empty, fired front row first."""
    hx, hy = hp['missiles']
    b.part(rect(hx - 3, hy - 3, hx + 2, hy + 2), GUNMETAL, 'plate', 5, step=0, corner=False, name='ml_box')
    cells = [(hx - 2, hy - 2), (hx, hy - 2), (hx - 2, hy), (hx, hy)]
    b.meta['tubes'] = [(x + 1.0, y + 1.0) for (x, y) in cells]
    for i, (x, y) in enumerate(cells):
        px = rect(x, y, x + 1, y + 1)
        if i >= st:
            b.part(px, REDM, 'plate', 5.5, step=0, corner=True, name='ml_warhead')
        else:
            b.part(px, DARK, 'flat', 5.5, step=-1, name='ml_tube')
            b.part({(x + 1, y + 1)}, DARK, 'flat', 5.6, step=1, name='ml_tube_rim')


@LINE.module_fn('laser')
def laser(d, b, st, hp):
    """A slim emitter along the left cheek: a gunmetal housing with a finned
    heat sink at its root and a bright collar at its muzzle; the lens is dull
    red at rest, red when charged, white-hot when it fires."""
    hx, hy = hp['laser']
    b.meta['muzzle'] = (float(hx), hy - 5.0)
    b.part(rect(hx - 1, hy - 3, hx, hy + 1), GUNMETAL, 'plate', 3, step=-1, corner=False, name='lz_housing')
    b.part(rect(hx - 1, hy, hx, hy + 1), DARK, 'grille', 3.2, step=2, pattern=dict(period=2, dir='h'), name='lz_fins')
    b.part(rect(hx - 1, hy - 4, hx, hy - 4), STEEL, 'plate', 3.1, step=1, corner=False, name='lz_collar')
    role = DIM_LENS if st == 0 else 'laser' if st == 1 else 'white'
    b.lamp(rect(hx - 1, hy - 5, hx, hy - 5), role, z=3.5, name='lz_lens')


@LINE.module_fn('plasma')
def plasma(d, b, st, hp):
    """Coil sleeves round each barrel behind the muzzle: a gunmetal sleeve,
    a plasma band glowing through its middle, the muzzle alight when it
    fires. On twin guns a sleeve grows outward only, so the pair keeps its
    air."""
    w = getattr(d, 'gun_w', 2)
    for (mx, my) in hp['plasma']:
        x2 = int(round(mx * 2))
        x0 = (x2 - w) // 2
        x1 = x0 + w - 1
        s0 = x0 - (1 if mx <= 0 else 0)
        s1 = x1 + (1 if mx >= 0 else 0)
        b.part(rect(s0, my + 2, s1, my + 5), GUNMETAL, 'cylv', 7, step=0, bevel=0, name='pl_sleeve')
        b.lamp(rect(s0, my + 3, s1, my + 3), DIM_PLASMA if st == 0 else 'plasma', z=7.2, name='pl_coil')
        if st == 2:
            b.lamp(rect(x0, my, x1, my), 'plasma', z=7.3, name='pl_muzzle')


@LINE.module_fn('flame')
def flame(d, b, st, hp):
    """A flamethrower on the left side: a red fuel tank strapped in steel, a
    hose to a steel nozzle forward; a blue pilot flame at rest, a jet of fire
    when it fires."""
    hx, hy = hp['flame']
    b.meta['muzzle'] = (hx + 0.5, hy - 5.0)
    b.part(rect(hx - 1, hy - 1, hx, hy + 3), REDM, 'cylv', 3, step=-1, name='fl_tank')
    b.part(rect(hx - 1, hy + 1, hx, hy + 1), STEEL, 'flat', 3.1, step=0, name='fl_strap')
    b.part(rect(hx, hy - 4, hx, hy - 2), STEEL, 'flat', 3.1, step=1, name='fl_nozzle')
    b.part({(hx - 1, hy - 2)}, GUNMETAL, 'flat', 3.05, step=0, name='fl_hose')
    if st < 2:
        b.lamp({(hx, hy - 5)}, PILOT if st == 0 else 'ion', z=3.5, name='fl_pilot')
    else:
        b.lamp({(hx, hy - 5), (hx, hy - 6)} if st == 2 else {(hx, hy - 5), (hx - 1, hy - 6), (hx, hy - 6)},
               'fire', z=3.5, name='fl_jet')


@LINE.module_fn('grenade')
def grenade(d, b, st, hp):
    """A grenade launcher on the roof: a round gunmetal drum of four
    chambers, each loaded one a brass grenade lit on its top left, each fired
    one a dark empty chamber (`st` fired, front row first), and a short
    wide-bore steel barrel forward with a muzzle band. It shares the
    missiles' hardpoint unless the design gives it its own - a tank carries
    one special weapon at a time."""
    hx, hy = hp.get('grenade', hp['missiles'])
    b.meta['muzzle'] = (hx + 0.5, hy - 6.0)
    b.part(chamfer(hx - 3, hy - 3, hx + 3, hy + 3, tl=2, tr=2, br=2, bl=2), GUNMETAL, 'plate', 5, step=0,
           corner=False, name='gl_drum')
    b.part({(hx, hy)}, STEEL, 'flat', 5.2, step=1, name='gl_axle')
    b.part(rect(hx - 1, hy - 6, hx + 1, hy - 3), STEEL, 'cylv', 5.4, step=0, bevel=0, name='gl_barrel')
    b.part(rect(hx - 1, hy - 6, hx + 1, hy - 6), STEEL, 'flat', 5.5, step=2, name='gl_muzzle')
    b.part({(hx, hy - 6)}, DARK, 'flat', 5.6, step=-1, name='gl_bore')
    cells = [(hx - 2, hy - 2), (hx + 1, hy - 2), (hx - 2, hy + 1), (hx + 1, hy + 1)]
    for i, (x, y) in enumerate(cells):
        px = rect(x, y, x + 1, y + 1)
        if i >= st:
            b.dome(px, BRASS, 5.5, gain=0.8, bevel=0, name='gl_round')
        else:
            b.part(px, DARK, 'flat', 5.5, step=-1, name='gl_chamber')


@LINE.module_fn('sonic')
def sonic(d, b, st, hp):
    """A sonic hammer on the roof (docs/sonic-hammer.md): a long-range
    acoustic dish - a round gunmetal mount, a short steel stem and a shallow
    dish across its front whose horns curve forward a pixel, its face a row
    of transducer dots. `st` 0 at rest, the face dark; 1 and 2 the wind-up,
    the inner and then the outer dots lit pale; 3 a blast, the whole face
    lit and the horns flexed forward. It shares the missiles' hardpoint
    unless the design gives it its own - a tank carries one special weapon
    at a time."""
    hx, hy = hp.get('sonic', hp['missiles'])
    b.meta['muzzle'] = (hx + 0.5, hy - 4.0)
    b.part(chamfer(hx - 2, hy - 1, hx + 2, hy + 2, tl=1, tr=1, br=1, bl=1), GUNMETAL, 'plate', 5, step=0,
           corner=False, name='sd_mount')
    b.part({(hx, hy - 2)}, STEEL, 'flat', 5.2, step=1, name='sd_stem')
    b.part(rect(hx - 2, hy - 2, hx + 2, hy - 2) - {(hx, hy - 2)}, GUNMETAL, 'flat', 5.3, step=1, name='sd_bowl')
    horns = {(hx - 3, hy - 3), (hx + 3, hy - 3), (hx - 3, hy - 4), (hx + 3, hy - 4)}
    if st == 3:
        horns |= {(hx - 3, hy - 5), (hx + 3, hy - 5)}
    b.part(horns, STEEL, 'flat', 5.4, step=2, name='sd_horns')
    b.part(rect(hx - 2, hy - 4, hx + 2, hy - 3), DARK, 'grille', 5.5, step=1, pattern=dict(period=2, dir='v'),
           name='sd_face')
    inner = rect(hx - 1, hy - 3, hx + 1, hy - 3)
    outer = rect(hx - 2, hy - 4, hx + 2, hy - 4) | {(hx - 2, hy - 3), (hx + 2, hy - 3)}
    if st == 1:
        b.lamp(inner, 'white', z=5.7, name='sd_inner')
    elif st == 2:
        b.lamp(outer, 'white', z=5.7, name='sd_outer')
    elif st == 3:
        b.lamp(inner | outer, 'white', z=5.7, name='sd_blast')


@LINE.module_fn('emp')
def emp(d, b, st, hp):
    """An EMP projector on the roof (docs/emp-burst.md): a squat gunmetal
    plinth carrying a toroid coil - a ring of brass windings round a dark
    core, lit on its top left rim and shaded on the far one so it reads round
    - and a steel emitter stub forward. `st` 0 armed, the core's lamp dim; 1
    and 2 the crackle before an enemy's pulse, the left and then the right
    half of the windings lit; 3 the pulse, the whole ring lit and the core;
    4 offline, the windings scorched and the core dark. It shares the
    missiles' hardpoint unless the design gives it its own."""
    hx, hy = hp.get('emp', hp['missiles'])
    b.meta['coil'] = (hx + 0.5, hy + 0.5)
    b.part(chamfer(hx - 3, hy - 2, hx + 3, hy + 3, tl=1, tr=1, br=1, bl=1), GUNMETAL, 'plate', 5, step=0,
           corner=False, name='em_plinth')
    core = rect(hx - 1, hy - 1, hx + 1, hy + 1)
    windings = chamfer(hx - 2, hy - 2, hx + 2, hy + 2, tl=1, tr=1, br=1, bl=1) - core
    b.part(windings, RUST if st == 4 else BRASS, 'map', 5.4, step=0,
           stepmap=rim_shade(windings, hx + 0.5, hy + 0.5, lit=1, shade=-1), name='em_windings')
    b.part(core, DARK, 'flat', 5.4, step=-1 if st == 4 else 0, name='em_core')
    b.part(rect(hx, hy - 4, hx, hy - 3), STEEL, 'flat', 5.2, step=1, name='em_stub')
    if st == 0:
        b.lamp({(hx, hy)}, DIM_PLASMA, z=5.7, name='em_core_lamp')
    elif st == 1:
        b.lamp({(x, y) for (x, y) in windings if x <= hx}, 'ion', z=5.7, name='em_crackle_l')
    elif st == 2:
        b.lamp({(x, y) for (x, y) in windings if x >= hx}, 'ion', z=5.7, name='em_crackle_r')
    elif st == 3:
        b.lamp(windings, 'white', z=5.7, name='em_pulse')
        b.lamp({(hx, hy)}, 'ion', z=5.7, name='em_pulse_core')


@LINE.module_fn('gauss')
def gauss(d, b, st, hp):
    """A gauss rail on the left cheek (docs/gauss-rail.md): a gunmetal
    capacitor block at the root with a column of four charge cells down its
    middle, and two steel rails running forward of it with a dark bore
    between them, their tips a step brighter. `st` 0 idle, the cells dark; 1
    to 4 the charge, that many cells lit from the rear; 5 full, every cell
    white and the slug glowing in the bore; 6 the shot, the rails and the
    bore white and the cells spent. It shares the laser's cheek unless the
    design gives it its own - a tank carries one special at a time."""
    hx, hy = hp.get('gauss', hp['laser'])
    b.meta['muzzle'] = (hx + 0.5, hy - 6.0)
    b.part(rect(hx - 1, hy - 1, hx + 1, hy + 2), GUNMETAL, 'plate', 3, step=-1, corner=False, name='gr_block')
    for x in (hx - 1, hx + 1):
        b.part(rect(x, hy - 5, x, hy - 2), STEEL, 'flat', 3.2, step=0, name='gr_rail')
        b.part({(x, hy - 6)}, STEEL, 'flat', 3.2, step=1, name='gr_rail_tip')
    bore = rect(hx, hy - 6, hx, hy - 2)
    b.part(bore, DARK, 'flat', 3.1, step=-1, name='gr_bore')
    cells = [(hx, hy + 2 - i) for i in range(4)]
    if st == 6:
        b.lamp(rect(hx - 1, hy - 6, hx + 1, hy - 2), 'white', z=3.5, name='gr_shot')
        b.part(set(cells), DARK, 'flat', 3.3, step=-1, name='gr_spent')
        return
    for i, c in enumerate(cells):
        if st == 5:
            b.lamp({c}, 'white', z=3.5, name='gr_cell_full')
        elif i < st:
            b.lamp({c}, 'ion', z=3.5, name='gr_cell_lit')
        else:
            b.lamp({c}, DIM_RAIL, z=3.5, name='gr_cell')
    if st == 5:
        b.lamp(bore, 'ion', z=3.5, name='gr_slug')


# ---------------------------------------------------------------------------
# Scout - fast recon. Silhouette: an arrowhead nose ahead of short narrow
# runs, a small round turret with the optic pod bulging off its left cheek
# and a radar dish over its right rear, a long thin gun. Lights: headlamps
# on the fender tips, a red chevron on the nose (marker), the eye in the
# mantlet, tail lamps.
# ---------------------------------------------------------------------------
@LINE.design('scout')
class Scout(Design):
    codename = 'Lynx'
    blurb = 'Light recon on narrow runs: an arrowhead nose, a small round turret with a big optic and a radar dish.'

    def hull(self, b, f):
        body = b.ctx.body
        runs(b, -6, -4, -4, 8)
        L = dict(
            N=dict(mat=body, mode='facet', z=2.2, normal=(0.0, -0.9), corner=False, name='nose'),
            f=dict(mat=body, mode='plate', z=3, corner=False, name='fender', tags=('armor',), split=True),
            S=dict(mat=body, mode='plate', z=2.6, step=-1, name='lip', tags=('skirt',), split=True),
            D=dict(mat=body, mode='plate', z=2, name='deck'),
            E=dict(mat=DARK, mode='grille', z=2.5, step=2, pattern=dict(period=2, dir='h'), tags=('vent',)),
            K=dict(mat=DARK, mode='flat', z=2.5, step=0, tags=('exhaust',)),
        )
        vpaint(b, [
            '....NN',
            '...NNN',
            '..NNNN',
            '.ffNNN',
            'fffDDD',
            '..SDDD',
            '..SDDD',
            '..SDDD',
            '..SDDD',
            '..SDDD',
            '..SDDD',
            '..SDDD',
            '..SDDD',
            '..SDDD',
            '..SEED',
            'fffDDD',
            '.ffKDD',
        ], -6, -8, L)
        turret_ring(b, 4.4, 3.4, 0.5)
        b.lamp({(-2, -7), (-3, -6)}, 'marker', z=2.6, kind='marker', name='chevron_l')
        b.lamp({(1, -7), (2, -6)}, 'marker', z=2.6, name='chevron_r')
        headlamp(b, -5, -5, 'head_l')
        headlamp(b, 4, -5, 'head_r')
        taillamp(b, -5, 8, 'tail_l')
        taillamp(b, 4, 8, 'tail_r')

    def turret(self, b, pose):
        body = b.ctx.body
        b.barrel(0, 2, -15, -4, mat=STEEL, z=2.5, recoil=self.recoil(pose))
        shell = mask([
            '..####..',
            '.######.',
            '########',
            '########',
            '########',
            '########',
            '.######.',
            '..####..',
        ], -4, -3)
        b.part(shell, body, 'map', 3, stepmap=rim_shade(shell, 0.0, 1.0, lit=1, shade=-1), name='casting')
        b.part(rect(-2, -4, 1, -3), STEEL, 'plate', 3.5, name='mantlet', tags=('armor',))
        b.lamp({(1, -4)}, 'sensor', z=4, kind='sensor', name='eye')
        # The big optic: a lens pod on the left cheek, its glass facing forward.
        b.part(chamfer(-5, -2, -2, 1, tl=1), body, 'plate', 3.8, step=0, corner=False, name='optic_pod', tags=('armor',))
        b.glass(rect(-4, -2, -3, -1), 4.2)
        # The radar dish on its arm, rear right, over the turret edge.
        b.part(rect(1, 2, 2, 2), STEEL, 'flat', 4, step=0, tags=('antenna',), name='dish_arm')
        b.dome(circle(3.5, 2.5, 1.6), PALE, 5, gain=0.5, tags=('antenna',), name='dish')
        b.part({(3, 2)}, DARK, 'flat', 5.2, step=1, tags=('antenna',), contact=False, name='feed')
        b.meta['muzzles'] = [(0.0, -15)]

    def hardpoints(self, ctx):
        return dict(minigun=(5, -1), laser=(-6, 1), missiles=(0, 2), flame=(-6, 3), plasma=[(0.0, -15)])


# ---------------------------------------------------------------------------
# Assault - general purpose, the line's reference: skirted runs, fenders
# over all four track ends, a slab-sided box turret with twin 60 mm guns, a
# cupola and a strapped bustle. Lights: headlamps on the front fenders, a
# teal strip across the glacis (marker), a status lamp and the eye on the
# turret, tail lamps.
# ---------------------------------------------------------------------------
@LINE.design('assault')
class Assault(Design):
    codename = 'Bulwark'
    blurb = 'Twin 60 mm in a slab-sided box turret; skirted tracks, a teal strip across the glacis.'

    def hull(self, b, f):
        body = b.ctx.body
        runs(b, -7, -5, -9, 8)
        L = dict(
            F=dict(mat=body, mode='plate', z=3, corner=False, name='fender', tags=('armor',), split=True),
            G=dict(mat=body, mode='facet', z=2.2, normal=(0.0, -0.9), corner=False, name='glacis'),
            S=dict(mat=body, mode='plate', z=2.6, step=-1, name='lip', tags=('skirt',), split=True),
            D=dict(mat=body, mode='plate', z=2, name='deck'),
            E=dict(mat=DARK, mode='grille', z=2.5, step=2, pattern=dict(period=2, dir='h'), tags=('vent',)),
            K=dict(mat=DARK, mode='flat', z=2.5, step=0, tags=('exhaust',)),
        )
        vpaint(b, [
            '.FFFGGG',
            'FFF.GGG',
            '..SGGGG',
            '..SGGGG',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDEEE',
            '..SDEEE',
            'FFFDDDD',
            '.FF.KDD',
        ], -7, -10, L)
        turret_ring(b, 6.2, 5.2, 0.0)
        headlamp(b, -6, -10, 'head_l')
        headlamp(b, 5, -10, 'head_r')
        taillamp(b, -4, 9, 'tail_l')
        taillamp(b, 3, 9, 'tail_r')
        b.lamp(rect(-2, -8, 1, -8), 'marker', z=3, kind='marker', name='strip')

    def turret(self, b, pose):
        body = b.ctx.body
        for i, xc2 in enumerate((-6, 6)):
            r = self.recoil(pose, i, 2)
            b.barrel(xc2, 2, -13, -5, mat=STEEL, z=2.5, recoil=r)
            x0 = (xc2 - 2) // 2
            b.part(rect(x0, -6 + r, x0 + 1, -6 + r), STEEL, 'cylv', 3.6, step=-1, name='collar%d' % i, tags=('barrel',))
        L = dict(
            m=dict(mat=body, mode='plate', z=3.5, step=-1, corner=False, name='mantlet', tags=('armor',)),
            f=dict(mat=body, mode='facet', z=3.2, normal=(0.0, -0.8), name='brow'),
            c=dict(mat=body, mode='plate', z=2, step=-1, name='cheek'),
            R=dict(mat=body, mode='plate', z=3, name='roof'),
            r=dict(mat=body, mode='plate', z=3, name='roof_rear'),
            k=dict(mat=body, mode='plate', z=2.2, step=-1, name='bustle', tags=('stowage',)),
            s=dict(mat=BRASS, mode='flat', z=2.3, step=-1, name='strap', tags=('stowage',)),
        )
        vpaint(b, [
            'mmmmf',
            'cmmmf',
            'cRRRR',
            'cRRRR',
            'cRRRR',
            'crrrr',
            'crrrr',
            'crrrr',
            '.cccc',
            '.kskk',
        ], -5, -5, L)
        b.lamp({(-1, -5), (0, -5)}, 'sensor', z=4, kind='sensor', name='eye')
        b.part(rect(-4, 0, -3, 1), body, 'inset', 3.5, name='hatch', tags=('hatch',))
        b.dome(circle(2.0, 0.0, 1.7), STEEL, 4, gain=0.5, name='cupola')
        b.glass({(1, -2), (2, -2)}, 4.2)
        b.rivets({(-4, -3), (3, -3)}, body, 3.4)
        b.part(rect(-4, 2, 3, 2), body, 'flat', 3.3, step=-2, name='seam')
        b.lamp({(4, 1)}, 'marker', z=4, name='status')
        b.meta['muzzles'] = [(-3.0, -13), (3.0, -13)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, -1), laser=(-7, -1), missiles=(0, 3), flame=(-7, 3),
                    plasma=[(-3.0, -13), (3.0, -13)])


# ---------------------------------------------------------------------------
# Breaker - the brawler that rams, in the line's own vocabulary: wide 4-px
# runs under a lip, fenders over all four ends, and a ram prow built into
# the bow (a thick front plate with two blunt steel ram blocks) instead of a
# separate blade. The warden's clean hex turret carries a heavy 4-px gun
# with one muzzle ring. Lights: a gold strip across the prow, headlamps on
# the front fenders, the eye, tail lamps.
# ---------------------------------------------------------------------------
@LINE.design('breaker')
class Breaker(Design):
    codename = 'Maul'
    gun_w = 4
    blurb = 'The rammer: a ram prow with two steel ram blocks built into the bow, wide skirted runs, a heavy gun on a hex turret.'

    def hull(self, b, f):
        body = b.ctx.body
        runs(b, -8, -5, -8, 8)
        L = dict(
            X=dict(mat=STEEL, mode='plate', z=3.3, step=0, corner=False, name='ram_block', tags=('armor',), split=True),
            R=dict(mat=body, mode='facet', z=3.0, normal=(0.0, -0.9), corner=False, name='prow'),
            G=dict(mat=body, mode='facet', z=2.2, normal=(0.0, -0.9), corner=False, name='glacis'),
            f=dict(mat=body, mode='plate', z=3, corner=False, name='fender', tags=('armor',), split=True),
            S=dict(mat=body, mode='plate', z=2.6, step=-1, name='lip', tags=('skirt',), split=True),
            D=dict(mat=body, mode='plate', z=2, name='deck'),
            E=dict(mat=DARK, mode='grille', z=2.5, step=2, pattern=dict(period=2, dir='h'), tags=('vent',)),
            K=dict(mat=DARK, mode='flat', z=2.5, step=0, tags=('exhaust',)),
        )
        vpaint(b, [
            '.ffXX...',
            'fffXXRRR',
            'ffffRRRR',
            '..SSGGGG',
            '..SSGGGG',
            '..SSDDDD',
            '..SSDDDD',
            '..SSDDDD',
            '..SSDDDD',
            '..SSDDDD',
            '..SSDDDD',
            '..SSDDDD',
            '..SSDDDD',
            '..SSDDDD',
            '..SSDDDD',
            '..SSDDDD',
            '..SSDDEE',
            '..SSDDEE',
            'ffffDDDD',
            'ffffDDDD',
            '.fffKKDD',
        ], -8, -11, L)
        turret_ring(b, 6.2, 5.2, -0.5)
        b.lamp(rect(-2, -9, 1, -9), 'marker', z=3.1, kind='marker', name='strip')
        headlamp(b, -7, -11, 'head_l', z=3.4)
        headlamp(b, 6, -11, 'head_r', z=3.4)
        taillamp(b, -7, 9, 'tail_l', z=3.4)
        taillamp(b, 6, 9, 'tail_r', z=3.4)

    def turret(self, b, pose):
        body = b.ctx.body
        r = self.recoil(pose)
        b.barrel(0, 4, -12, -5, mat=STEEL, z=2.5, recoil=r)
        b.part(rect(-3, -12 + r, 2, -12 + r), STEEL, 'cylv', 2.7, step=1, name='muzzle_ring', tags=('barrel',))
        faces(b, [
            '..aaa',
            '.bRRR',
            'bRRRR',
            'cRRRR',
            'cRRRR',
            'cRRRR',
            'dRRRR',
            '.dRRR',
            '..eee',
        ], -5, -5, body, 3, HEX_STEPS)
        b.part(rect(-3, -6, 2, -5), body, 'plate', 3.5, step=-1, name='mantlet', tags=('armor',))
        b.part(rect(-2, -7 + r, 1, -7 + r), STEEL, 'cylv', 3.6, step=-1, name='collar', tags=('barrel',))
        b.lamp({(-3, -5)}, 'sensor', z=4, kind='sensor', name='eye')
        b.dome(circle(2.5, 0.5, 1.7), STEEL, 4, gain=0.5, name='cupola')
        b.glass({(2, -1), (3, -1)}, 4.2)
        b.part(rect(-4, -1, -3, 0), body, 'inset', 3.5, name='hatch', tags=('hatch',))
        b.rivets({(-4, -3), (3, -3)}, body, 3.4)
        b.part(rect(-3, 4, 2, 4), body, 'plate', 2.8, step=-1, name='bustle', tags=('stowage',))
        b.meta['muzzles'] = [(0.0, -12)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, -2), laser=(-6, -2), missiles=(0, 1), flame=(-6, 3), plasma=[(0.0, -12)])


# ---------------------------------------------------------------------------
# Longbow - the sniper: a very long gun with a bore evacuator and a double-
# baffle brake, a round turret carried aft and lit a step above the hull,
# rangefinder ears across it, sniper camouflage on the decks, stabiliser
# spades folded on the stern. Lights: red strips either side of the gun,
# headlamps, the eye, tail lamps.
# ---------------------------------------------------------------------------
@LINE.design('longbow')
class Longbow(Design):
    codename = 'Yew'
    blurb = 'A sniper gun on a long hull: rangefinder ears across the round turret, a bore evacuator, stern spades.'

    def hull(self, b, f):
        body = b.ctx.body
        runs(b, -7, -5, -9, 8)
        L = dict(
            G=dict(mat=body, mode='facet', z=2.2, normal=(0.0, -0.9), corner=False, name='glacis'),
            f=dict(mat=body, mode='plate', z=3, corner=False, name='fender', tags=('armor',), split=True),
            S=dict(mat=body, mode='plate', z=2.6, step=-1, name='lip', tags=('skirt',), split=True),
            D=dict(mat=body, mode='plate', z=2, name='deck'),
            E=dict(mat=DARK, mode='grille', z=2.5, step=2, pattern=dict(period=2, dir='h'), tags=('vent',)),
            s=dict(mat=body, mode='plate', z=2.8, step=0, corner=False, name='spade', tags=('armor',), split=True),
            t=dict(mat=STEEL, mode='plate', z=2.8, step=0, name='spade_blade'),
            h=dict(mat=GUNMETAL, mode='plate', z=2.9, step=0, name='hinge'),
        )
        m = vpaint(b, [
            '...GGGG',
            '..GGGGG',
            '.fGGGGG',
            'ffSGGGG',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDEED',
            '..SDEED',
            'ffSDDDD',
            'fffDDDD',
            '...hssD',
            '...sss.',
            '...ttt.',
        ], -7, -12, L)
        # Sniper camouflage: a few big patches, light and dark, on the decks.
        livery(b, [
            '..............',
            '..............',
            '..............',
            '..............',
            '...dd.........',
            '..ddd.........',
            '...d..........',
            '..............',
            '..............',
            '..............',
            '..............',
            '..............',
            '..............',
            '..............',
            '..............',
            '..............',
            '..............',
            '.........ll...',
            '........lll...',
            '...dd....l....',
            '..ddd.........',
        ], -7, -12, dict(l=1, d=-1), 2.3, under=m['G'].px | m['D'].px)
        turret_ring(b, 4.8, 3.8, 1.0)
        b.lamp(rect(-4, -10, -3, -10), 'marker', z=2.6, kind='marker', name='strip_l')
        b.lamp(rect(2, -10, 3, -10), 'marker', z=2.6, name='strip_r')
        headlamp(b, -6, -10, 'head_l')
        headlamp(b, 5, -10, 'head_r')
        taillamp(b, -7, 8, 'tail_l')
        taillamp(b, 6, 8, 'tail_r')

    def turret(self, b, pose):
        body = b.ctx.body
        r = self.recoil(pose)
        b.barrel(0, 2, -17, -4, mat=STEEL, z=2.5, recoil=r)
        brake2(b, 0, 2, -17, r)
        b.part(rect(-2, -11 + r, 1, -9 + r), STEEL, 'cylv', 2.7, name='evacuator', tags=('barrel',))
        shell = mask([
            '...####...',
            '.########.',
            '.########.',
            '##########',
            '##########',
            '##########',
            '##########',
            '.########.',
            '.########.',
            '...####...',
        ], -5, -4)
        b.part(shell, body, 'map', 3, stepmap=rim_shade(shell, 0.0, 1.0, lit=2, shade=0, top=1), name='shell')
        livery(b, [
            '..........',
            '..........',
            '.dd.......',
            '.ddd......',
            '..........',
            '.......ll.',
            '......lll.',
            '..........',
            '..........',
            '..........',
        ], -5, -4, dict(l=2, d=0), 3.05, under=shell)
        b.part(rect(-2, -5, 1, -4), body, 'plate', 3.5, step=-1, name='mantlet', tags=('armor',))
        b.part(rect(-1, -5, 0, -5), STEEL, 'cylv', 3.6, step=-1, name='collar', tags=('barrel',))
        # Rangefinder ears across the turret: a painted tube, steel lens heads.
        b.part(rect(-6, -1, 5, -1), body, 'plate', 3.8, step=0, name='rangefinder', tags=('armor',))
        for x0 in (-7, 5):
            b.part(rect(x0, -2, x0 + 1, 0), STEEL, 'plate', 3.9, step=0, name='lens_head', tags=('optic',))
        b.glass({(-7, -2), (-6, -2)}, 4.0)
        b.glass({(5, -2), (6, -2)}, 4.0)
        b.lamp({(-2, -4)}, 'sensor', z=4, kind='sensor', name='eye')
        b.part(rect(-3, 2, -2, 3), body, 'inset', 3.5, name='hatch', tags=('hatch',))
        b.dome(circle(2.0, 2.5, 1.5), STEEL, 4, gain=0.5, name='cupola')
        b.meta['muzzles'] = [(0.0, -17)]

    def hardpoints(self, ctx):
        return dict(minigun=(5, -4), laser=(-6, -4), missiles=(0, 3), flame=(-6, 3), plasma=[(0.0, -17)])


# ---------------------------------------------------------------------------
# Flak - anti-air, compact: a stubby hull, twin short autocannons with flash
# hiders on a hex turret, gunmetal ammo drums on both cheeks (the
# silhouette's ears), a flat radar array over the bustle. Lights: a pale
# strip across the glacis, headlamps, the eye, tail lamps.
# ---------------------------------------------------------------------------
@LINE.design('flak')
class Flak(Design):
    codename = 'Hornet'
    blurb = 'Twin autocannons with flash hiders, drum-fed from both cheeks, a radar panel on the bustle of a stubby hull.'

    def hull(self, b, f):
        body = b.ctx.body
        runs(b, -7, -5, -6, 6)
        L = dict(
            G=dict(mat=body, mode='facet', z=2.2, normal=(0.0, -0.9), corner=False, name='glacis'),
            f=dict(mat=body, mode='plate', z=3, corner=False, name='fender', tags=('armor',), split=True),
            S=dict(mat=body, mode='plate', z=2.6, step=-1, name='lip', tags=('skirt',), split=True),
            D=dict(mat=body, mode='plate', z=2, name='deck'),
            E=dict(mat=DARK, mode='grille', z=2.5, step=2, pattern=dict(period=2, dir='h'), tags=('vent',)),
            K=dict(mat=DARK, mode='flat', z=2.5, step=0, tags=('exhaust',)),
        )
        vpaint(b, [
            '.ffGGGG',
            'fffGGGG',
            '..SGGGG',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDEED',
            '..SDEED',
            'fffDDDD',
            'fffDDDD',
            '.ffKDDD',
        ], -7, -8, L)
        turret_ring(b, 4.8, 3.8, 0.0)
        b.lamp(rect(-2, -7, 1, -7), 'marker', z=2.6, kind='marker', name='strip')
        headlamp(b, -5, -8, 'head_l')
        headlamp(b, 4, -8, 'head_r')
        taillamp(b, -6, 7, 'tail_l')
        taillamp(b, 5, 7, 'tail_r')

    def turret(self, b, pose):
        body = b.ctx.body
        for i, xc2 in enumerate((-6, 6)):
            r = self.recoil(pose, i, 2)
            b.barrel(xc2, 2, -11, -4, mat=STEEL, z=2.5, recoil=r)
            x0 = (xc2 - 2) // 2
            b.part(rect(x0, -11 + r, x0 + 1, -10 + r), GUNMETAL, 'cylv', 2.7, step=-1, name='hider%d' % i,
                   tags=('barrel',))
        faces(b, [
            '..aaa',
            '.bRRR',
            'bRRRR',
            'cRRRR',
            'cRRRR',
            'dRRRR',
            '.dRRR',
            '..eee',
        ], -5, -4, body, 3, HEX_STEPS)
        b.part(rect(-4, -5, 3, -4), body, 'plate', 3.5, step=-1, name='mantlet', tags=('armor',))
        b.lamp({(-1, -4), (0, -4)}, 'sensor', z=4, kind='sensor', name='eye')
        # Ammo drums on both cheeks: gunmetal cylinders on their sides, a
        # brass feed band round each.
        for x0, nm in ((-8, 'drum_l'), (5, 'drum_r')):
            b.part(rect(x0, -2, x0 + 2, 0), GUNMETAL, 'cylh', 3.6, step=0, name=nm, tags=('armor',))
            b.part(rect(x0 + 1, -2, x0 + 1, 0), BRASS, 'cylh', 3.7, step=0, name=nm + '_band')
        # The radar: a flat array on a post over the bustle, lit face and dark back.
        b.part(rect(-1, 1, 0, 1), STEEL, 'plate', 3.8, name='radar_post')
        b.part(rect(-3, 2, 2, 2), PALE, 'flat', 4, step=0, name='radar', tags=('fragile',))
        b.part(rect(-3, 3, 2, 3), GUNMETAL, 'flat', 4, step=-1, name='radar_back', tags=('fragile',))
        b.meta['muzzles'] = [(-3.0, -11), (3.0, -11)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, -3), laser=(-7, -3), missiles=(0, 1), flame=(-7, 2), plasma=[(-3.0, -11), (3.0, -11)])


# ---------------------------------------------------------------------------
# Wraith - stealth: every plate a flat cut facet (the left planes lit, the
# right ones dark), skirts hiding the runs, a low kite turret with a lit
# spine and one visor slit, a dark gun. Lights: dim - a pale slit down each
# skirt (marker), the eye burning at the end of the visor, small head and
# tail lamps.
# ---------------------------------------------------------------------------
@LINE.design('wraith')
class Wraith(Design):
    codename = 'Shade'
    blurb = 'Stealth: every plate a flat cut facet, skirts the full length of the runs, a low kite turret with one visor slit.'

    def hull(self, b, f):
        body = b.ctx.body
        runs(b, -6, -4, -5, 7)
        # Facet tones: left planes face the light, right planes fall away.
        faces(b, [
            '...ppp',
            '..pppp',
            '.kpppp',
            'kkqppp',
            'kkqRRR',
            'kkqRRR',
            'kkqRRR',
            'kkqRRR',
            'kkqRRR',
            'kkqRRR',
            'kkqRRR',
            'kkqRRR',
            'kkqRRR',
            'kkdddd',
            '.kdddd',
            '..dddd',
            '...eee',
        ], -6, -8, body, 2.4, dict(p=(1, -1), k=(1, -1), q=(0, -2), R=(0, 0), d=(-1, -2), e=(-2, -2)),
            name='hull')
        b.part(rect(-6, -3, -6, 3) | rect(5, -3, 5, 3), body, 'flat', 2.5, step=0, name='skirt_panel',
               tags=('skirt',))
        turret_ring(b, 4.4, 3.4, 0.0)
        b.grille(-2, 6, 1, 6, DARK, 2.6, step=2)
        b.lamp(rect(-6, -2, -6, 1), 'marker', z=2.8, kind='marker', name='skirt_l')
        b.lamp(rect(5, -2, 5, 1), 'marker', z=2.8, name='skirt_r')
        headlamp(b, -3, -8, 'head_l', z=3)
        headlamp(b, 2, -8, 'head_r', z=3)
        taillamp(b, -3, 8, 'tail_l', z=3)
        taillamp(b, 2, 8, 'tail_r', z=3)

    def turret(self, b, pose):
        body = b.ctx.body
        b.barrel(0, 2, -13, -4, mat=GUNMETAL, z=2.5, recoil=self.recoil(pose))
        faces(b, [
            '....a',
            '...bb',
            '..bbb',
            '.bbbs',
            'bbbbs',
            'dddds',
            '.ddds',
            '..dds',
            '...dd',
        ], -5, -5, body, 3, dict(a=(1, -1), b=(1, -1), s=(2, -1), d=(0, -2)), name='kite')
        # One visor slit across the brow, the sensor burning at its end.
        b.part(rect(-3, -2, 2, -2), DARK, 'flat', 3.4, step=0, name='visor', tags=('optic',))
        b.lamp({(-3, -2)}, 'sensor', z=3.6, kind='sensor', name='eye')
        b.meta['muzzles'] = [(0.0, -13)]

    def hardpoints(self, ctx):
        return dict(minigun=(5, -1), laser=(-6, -1), missiles=(0, 1), flame=(-6, 2), plasma=[(0.0, -13)])


# ---------------------------------------------------------------------------
# Warden - the defender: shield emitters glowing on the four corners and
# down both turret flanks, a layered frontal plate, a hex turret with one
# heavy gun wearing a fume extractor. Lights: the teal emitters (marker),
# headlamps, the eye, tail lamps.
# ---------------------------------------------------------------------------
@LINE.design('warden')
class Warden(Design):
    codename = 'Aegis'
    gun_w = 4
    blurb = 'Shield emitters glowing on the four corners and down the turret flanks, layered frontal plate, one heavy gun.'

    def hull(self, b, f):
        body = b.ctx.body
        runs(b, -7, -5, -8, 7)
        L = dict(
            H=dict(mat=body, mode='facet', z=2.6, normal=(0.0, -0.9), corner=False, name='front_plate'),
            A=dict(mat=body, mode='plate', z=2.8, step=0, corner=False, name='applique', tags=('armor',), split=True),
            G=dict(mat=body, mode='plate', z=2.2, step=0, name='glacis'),
            f=dict(mat=body, mode='plate', z=3, corner=False, name='fender', tags=('armor',), split=True),
            S=dict(mat=body, mode='plate', z=2.6, step=-1, name='lip', tags=('skirt',), split=True),
            D=dict(mat=body, mode='plate', z=2, name='deck'),
            E=dict(mat=DARK, mode='grille', z=2.5, step=2, pattern=dict(period=2, dir='h'), tags=('vent',)),
            K=dict(mat=DARK, mode='flat', z=2.5, step=0, tags=('exhaust',)),
        )
        vpaint(b, [
            '..fHHHH',
            '.ffHHHH',
            'fffAAAA',
            '..SAAAA',
            '..SGGGG',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDEED',
            '..SDEED',
            'fffDDDD',
            'fffDDDD',
            '.ffKDDD',
        ], -7, -10, L)
        # Seams between the appliqué blocks.
        b.part({(-3, -8), (-3, -7), (2, -8), (2, -7)}, body, 'flat', 2.9, step=-2, name='seam')
        turret_ring(b, 6.2, 5.2, -1.0)
        # The shield emitters: a steel knob on each corner, a teal core.
        for (cx, cy, nm) in ((-5.5, -8.5, 'fl'), (5.5, -8.5, 'fr'), (-5.5, 7.5, 'rl'), (5.5, 7.5, 'rr')):
            b.dome(circle(cx, cy, 1.5), body, 3.5, step=1, gain=0.5, name='emitter_' + nm, tags=('fragile',))
            b.lamp({(int(math.floor(cx)), int(math.floor(cy)))}, 'marker', z=3.8,
                   kind='marker' if nm in ('fl', 'fr') else None, name='core_' + nm)
        headlamp(b, -4, -10, 'head_l', z=3)
        headlamp(b, 3, -10, 'head_r', z=3)
        taillamp(b, -3, 9, 'tail_l', z=3)
        taillamp(b, 2, 9, 'tail_r', z=3)

    def turret(self, b, pose):
        body = b.ctx.body
        r = self.recoil(pose)
        b.barrel(0, 4, -14, -5, mat=STEEL, z=2.5, recoil=r)
        b.part(rect(-3, -10 + r, 2, -9 + r), STEEL, 'cylv', 2.7, name='extractor', tags=('barrel',))
        b.part(rect(-2, -14 + r, 1, -14 + r), STEEL, 'cylv', 2.7, step=1, name='muzzle_ring', tags=('barrel',))
        faces(b, [
            '..aaa',
            '.bRRR',
            'bRRRR',
            'cRRRR',
            'cRRRR',
            'cRRRR',
            'dRRRR',
            '.dRRR',
            '..eee',
        ], -5, -5, body, 3, HEX_STEPS)
        b.part(rect(-3, -6, 2, -5), body, 'plate', 3.5, step=-1, name='mantlet', tags=('armor',))
        b.part(rect(-2, -7 + r, 1, -7 + r), STEEL, 'cylv', 3.6, step=-1, name='collar', tags=('barrel',))
        b.lamp({(-3, -5)}, 'sensor', z=4, kind='sensor', name='eye')
        b.dome(circle(2.5, 0.5, 1.7), STEEL, 4, gain=0.5, name='cupola')
        b.glass({(2, -1), (3, -1)}, 4.2)
        b.part(rect(-4, -1, -3, 0), body, 'inset', 3.5, name='hatch', tags=('hatch',))
        # Shield emitter strips down both flanks, in steel frames.
        for x, nm in ((-6, 'l'), (5, 'r')):
            b.part(rect(x, -3, x, 1), STEEL, 'plate', 3.5, step=-1, corner=False, name='emitter_frame_' + nm,
                   tags=('armor',))
            b.lamp(rect(x, -2, x, 0), 'marker', z=3.7, name='emitter_strip_' + nm)
        b.part(rect(-3, 4, 2, 4), body, 'plate', 2.8, step=-1, name='bustle', tags=('stowage',))
        b.meta['muzzles'] = [(0.0, -14)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, -2), laser=(-6, -2), missiles=(0, 1), flame=(-6, 3), plasma=[(0.0, -14)])


# ---------------------------------------------------------------------------
# Ravager - heavy assault: a full-width ram with three steel teeth, runs
# inset so the ram and the sponsons stand past them, MGs out of the
# sponsons, tall twin stacks, twin guns on a round turret. Lights: red
# strips in the ram, headlamps, the eye, tail lamps.
# ---------------------------------------------------------------------------
@LINE.design('ravager')
class Ravager(Design):
    codename = 'Warhog'
    blurb = 'Heavy assault: a spiked ram, sponsons bulging past the runs with MGs, twin stacks, twin guns on a big round turret.'

    def hull(self, b, f):
        body = b.ctx.body
        runs(b, -7, -5, -7, 9)
        L = dict(
            x=dict(mat=STEEL, mode='plate', z=3.2, step=1, corner=False, name='spike'),
            B=dict(mat=body, mode='plate', z=3, step=0, corner=False, name='ram', tags=('armor',), split=True),
            u=dict(mat=body, mode='flat', z=3, step=-1, name='ram_foot'),
            G=dict(mat=body, mode='facet', z=2.2, normal=(0.0, -0.9), corner=False, name='glacis'),
            f=dict(mat=body, mode='plate', z=3, corner=False, name='fender', tags=('armor',), split=True),
            P=dict(mat=body, mode='plate', z=3.2, step=0, corner=False, name='sponson', tags=('armor',), split=True),
            S=dict(mat=body, mode='plate', z=2.6, step=-1, name='lip', tags=('skirt',), split=True),
            D=dict(mat=body, mode='plate', z=2, name='deck'),
            E=dict(mat=DARK, mode='grille', z=2.5, step=2, pattern=dict(period=2, dir='h'), tags=('vent',)),
        )
        vpaint(b, [
            '.x.....x',
            'xxx...xx',
            'BBBBBBBB',
            'uuuuuuuu',
            '.fffGGGG',
            '...SGGGG',
            '...SDDDD',
            '...SDDDD',
            'PPPPDDDD',
            'PPPPDDDD',
            'PPPPDDDD',
            'PPPPDDDD',
            'PPPPDDDD',
            'PPPPDDDD',
            '...SDDDD',
            '...SDDDD',
            '...SDDDD',
            '...SEEED',
            '...SEEED',
            '.fffDDDD',
            '.fffDDDD',
            '..ffDDDD',
        ], -8, -11, L)
        turret_ring(b, 6.6, 5.6, 0.5)
        # Sponson MGs poking forward past the runs.
        for x in (-8, 7):
            b.part({(x, -5), (x, -4)}, STEEL, 'flat', 3.3, step=1, name='sponson_mg')
            b.part({(x, -5)}, DARK, 'flat', 3.35, step=1, name='sponson_mg_bore', contact=False, cast=False)
        # Exhaust stacks on the rear fenders: tall pipes, sooty mouths.
        for x0 in (-7, 5):
            b.part(rect(x0, 8, x0 + 1, 10), STEEL, 'cylv', 3.5, step=-1, bevel=0, tags=('exhaust',), name='stack')
            b.part(rect(x0, 8, x0 + 1, 8), DARK, 'flat', 3.6, step=0, tags=('exhaust',), name='stack_mouth',
                   contact=False)
        b.lamp(rect(-4, -9, -3, -9), 'marker', z=3.3, kind='marker', name='strip_l')
        b.lamp(rect(2, -9, 3, -9), 'marker', z=3.3, name='strip_r')
        headlamp(b, -7, -9, 'head_l', z=3.4)
        headlamp(b, 6, -9, 'head_r', z=3.4)
        taillamp(b, -4, 10, 'tail_l')
        taillamp(b, 3, 10, 'tail_r')
        self.emitters = dict(smoke=[(-6.0, 8.5), (6.0, 8.5)])

    def turret(self, b, pose):
        body = b.ctx.body
        for i, xc2 in enumerate((-6, 6)):
            b.barrel(xc2, 2, -14, -5, mat=STEEL, z=2.5, recoil=self.recoil(pose, i, 2))
        shell = circle(0.0, 0.5, 5.0)
        b.part(shell, body, 'map', 3, stepmap=rim_shade(shell, 0.0, 0.5, lit=1, shade=-1), name='turret_shell')
        b.part(rect(-5, -6, 4, -5), body, 'plate', 3.5, step=-1, corner=False, name='mantlet', tags=('armor',))
        for x0 in (-4, 2):
            b.part(rect(x0, -7, x0 + 1, -7), STEEL, 'cylv', 3.6, step=-1, name='collar', tags=('barrel',))
        b.lamp({(-1, -5), (0, -5)}, 'sensor', z=4, kind='sensor', name='eye')
        b.dome(circle(2.5, 1.5, 1.7), STEEL, 4, gain=0.5, name='cupola')
        b.glass({(2, 0), (3, 0)}, 4.2)
        b.part(rect(-4, 1, -3, 2), body, 'inset', 3.5, name='hatch', tags=('hatch',))
        b.part(rect(-2, 4, 1, 4), BRASS, 'flat', 3.4, step=-1, name='strap', tags=('stowage',))
        b.meta['muzzles'] = [(-3.0, -14), (3.0, -14)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, -2), laser=(-6, -2), missiles=(0, 1), flame=(-6, 3), plasma=[(-3.0, -14), (3.0, -14)])


# ---------------------------------------------------------------------------
# Glacier - the line at its simplest: the flak's compact hull (fenders over
# the four track ends, a lip over each run, a grilled engine deck) and the
# assault's box turret with one long gun. The arctic hint is the chassis's
# white accent alone: a pale strip across the glacis that glows cold at
# night. Lights: the strip, headlamps on the front fenders, the eye, tail
# lamps.
# ---------------------------------------------------------------------------
@LINE.design('glacier')
class Glacier(Design):
    codename = 'Floe'
    blurb = 'The line at its simplest: a compact skirted hull, a box turret with one long gun, a white strip across the glacis.'

    def hull(self, b, f):
        body = b.ctx.body
        runs(b, -7, -5, -5, 5)
        L = dict(
            G=dict(mat=body, mode='facet', z=2.2, normal=(0.0, -0.9), corner=False, name='glacis'),
            f=dict(mat=body, mode='plate', z=3, corner=False, name='fender', tags=('armor',), split=True),
            S=dict(mat=body, mode='plate', z=2.6, step=-1, name='lip', tags=('skirt',), split=True),
            D=dict(mat=body, mode='plate', z=2, name='deck'),
            E=dict(mat=DARK, mode='grille', z=2.5, step=2, pattern=dict(period=2, dir='h'), tags=('vent',)),
            K=dict(mat=DARK, mode='flat', z=2.5, step=0, tags=('exhaust',)),
        )
        vpaint(b, [
            '.ffGGGG',
            'fffGGGG',
            '..SGGGG',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDEED',
            '..SDEED',
            'fffDDDD',
            'fffDDDD',
            '.ffKDDD',
        ], -7, -7, L)
        turret_ring(b, 4.8, 3.8, -0.5)
        b.lamp(rect(-2, -6, 1, -6), 'marker', z=2.6, kind='marker', name='strip')
        headlamp(b, -5, -7, 'head_l')
        headlamp(b, 4, -7, 'head_r')
        taillamp(b, -6, 6, 'tail_l')
        taillamp(b, 5, 6, 'tail_r')

    def turret(self, b, pose):
        body = b.ctx.body
        r = self.recoil(pose)
        b.barrel(0, 2, -14, -5, mat=STEEL, z=2.5, recoil=r)
        b.part(rect(-1, -6 + r, 0, -6 + r), STEEL, 'cylv', 3.6, step=-1, name='collar', tags=('barrel',))
        L = dict(
            m=dict(mat=body, mode='plate', z=3.5, step=-1, corner=False, name='mantlet', tags=('armor',)),
            f=dict(mat=body, mode='facet', z=3.2, normal=(0.0, -0.8), name='brow'),
            c=dict(mat=body, mode='plate', z=2, step=-1, name='cheek'),
            R=dict(mat=body, mode='plate', z=3, name='roof'),
            k=dict(mat=body, mode='plate', z=2.2, step=-1, name='bustle', tags=('stowage',)),
        )
        vpaint(b, [
            '..mmf',
            'cfmmf',
            'cRRRR',
            'cRRRR',
            'cRRRR',
            'cRRRR',
            'cRRRR',
            '.cccc',
            '..kkk',
        ], -5, -5, L)
        b.lamp({(-1, -4), (0, -4)}, 'sensor', z=4, kind='sensor', name='eye')
        b.part(rect(-4, 0, -3, 1), body, 'inset', 3.5, name='hatch', tags=('hatch',))
        b.dome(circle(2.0, 0.0, 1.7), STEEL, 4, gain=0.5, name='cupola')
        b.glass({(1, -2), (2, -2)}, 4.2)
        b.meta['muzzles'] = [(0.0, -14)]

    def hardpoints(self, ctx):
        return dict(minigun=(6, -1), laser=(-7, -1), missiles=(0, 2), flame=(-7, 2), plasma=[(0.0, -14)])


# ---------------------------------------------------------------------------
# Obelisk - siege: twin long guns out of a wedge turret lit a step above its
# dark hull, a revolver autoloader ringed with brass rounds on the bustle,
# stabiliser legs folded to the four corners on round feet. Lights: a gold
# strip on the glacis, headlamps on the fenders, the eye, tail lamps.
# ---------------------------------------------------------------------------
@LINE.design('obelisk')
class Obelisk(Design):
    codename = 'Trebuchet'
    blurb = 'Siege: twin long guns out of a wedge turret, a revolver autoloader on the bustle, stabiliser feet at the corners.'

    def hull(self, b, f):
        body = b.ctx.body
        runs(b, -7, -5, -9, 8)
        L = dict(
            G=dict(mat=body, mode='facet', z=2.2, normal=(0.0, -0.9), corner=False, name='glacis'),
            f=dict(mat=body, mode='plate', z=3, corner=False, name='fender', tags=('armor',), split=True),
            S=dict(mat=body, mode='plate', z=2.6, step=-1, name='lip', tags=('skirt',), split=True),
            D=dict(mat=body, mode='plate', z=2, name='deck'),
            E=dict(mat=DARK, mode='grille', z=2.5, step=2, pattern=dict(period=2, dir='h'), tags=('vent',)),
            l=dict(mat=STEEL, mode='plate', z=3.3, step=-1, corner=False, name='leg'),
        )
        vpaint(b, [
            '...GGGG',
            '.l.GGGG',
            '.lfGGGG',
            'fffGGGG',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDDDD',
            '..SDEED',
            '..SDEED',
            '..SDDDD',
            'fffDDDD',
            '.lfDDDD',
            '.l.DDDD',
            '....DDD',
        ], -7, -12, L)
        # The stabiliser feet: a steel pad at the end of each folded leg.
        for (cx, cy, nm) in ((-6.0, -11.0, 'fl'), (6.0, -11.0, 'fr'), (-6.0, 11.0, 'rl'), (6.0, 11.0, 'rr')):
            b.dome(circle(cx, cy, 1.5), GUNMETAL, 3.5, step=0, gain=0.5, name='foot_' + nm, tags=('pad',))
        turret_ring(b, 6.0, 5.0, 0.0)
        b.lamp(rect(-2, -11, 1, -11), 'marker', z=2.6, kind='marker', name='strip')
        headlamp(b, -5, -10, 'head_l', z=3.4)
        headlamp(b, 4, -10, 'head_r', z=3.4)
        taillamp(b, -3, 11, 'tail_l', z=3)
        taillamp(b, 2, 11, 'tail_r', z=3)

    def turret(self, b, pose):
        body = b.ctx.body
        for i, xc2 in enumerate((-6, 6)):
            r = self.recoil(pose, i, 2)
            b.barrel(xc2, 2, -16, -5, mat=STEEL, z=2.5, recoil=r)
            x0 = (xc2 - 2) // 2
            b.part(rect(x0, -16 + r, x0 + 1, -16 + r), STEEL, 'cylv', 2.7, step=1, name='muzzle%d' % i, tags=('barrel',))
        faces(b, [
            '..aaaa',
            '..bRRR',
            '.bbRRR',
            '.bRRRR',
            'bbRRRR',
            'bRRRRR',
            'bRRRRR',
            'bRRRRR',
            'eeeeee',
        ], -6, -5, body, 3, dict(R=(1, 1), a=(1, 1), b=(2, 0), e=(0, 0)), name='wedge', roof_name='roof')
        b.part(rect(-5, -6, 4, -5), body, 'plate', 3.5, step=-1, corner=False, name='mantlet', tags=('armor',))
        b.lamp({(-1, -5), (0, -5)}, 'sensor', z=4, kind='sensor', name='eye')
        # The revolver autoloader on the bustle: a drum ringed with brass rounds.
        drum = circle(0.0, 3.5, 2.6)
        b.part(drum, GUNMETAL, 'dome', 3.6, step=0, gain=0.4, name='carousel')
        rounds = {(-2, 2), (1, 2), (-3, 3), (2, 3), (-2, 5), (1, 5)}
        b.part(rounds, BRASS, 'flat', 3.7, step=1, name='rounds', contact=False)
        b.part({(-1, 3), (0, 3), (-1, 4), (0, 4)}, STEEL, 'plate', 3.8, step=0, name='hub')
        b.glass(rect(2, -3, 3, -3), 3.4)
        b.meta['muzzles'] = [(-3.0, -16), (3.0, -16)]

    def hardpoints(self, ctx):
        return dict(minigun=(7, -2), laser=(-8, -2), missiles=(0, -1), flame=(-8, 1), plasma=[(-3.0, -16), (3.0, -16)])


# ---------------------------------------------------------------------------
# Titan - super-heavy: four runs (two a side, a beam between them), a
# massive hex turret with twin 4-px guns in bolted cheek mantlets, a
# commander's cupola with its own MG, a roof vent, a whip antenna. Lights: a
# white light bar, a headlamp on every run, a turret searchlight, the eye,
# tail lamps.
# ---------------------------------------------------------------------------
@LINE.design('titan')
class Titan(Design):
    codename = 'Colossus'
    gun_w = 4
    blurb = 'Super-heavy on four runs: a massive hex turret, twin 4-px guns in cheek mantlets, a commander\'s MG cupola.'

    def hull(self, b, f):
        body = b.ctx.body
        b.tread(-11, -10, -10, 9, 'L', z=1, name='track_lo')
        b.tread(-8, -7, -10, 9, 'L', z=1, name='track_li')
        b.tread(9, 10, -10, 9, 'R', z=1, name='track_ro')
        b.tread(6, 7, -10, 9, 'R', z=1, name='track_ri')
        L = dict(
            G=dict(mat=body, mode='plate', z=2.2, step=0, corner=False, name='glacis'),
            f=dict(mat=body, mode='plate', z=3, step=-1, corner=False, name='fender', tags=('armor',), split=True),
            S=dict(mat=body, mode='plate', z=2.6, step=-1, name='lip', tags=('skirt',), split=True),
            M=dict(mat=body, mode='plate', z=2.6, step=-1, name='beam', tags=('skirt',), split=True),
            D=dict(mat=body, mode='plate', z=2, name='deck'),
            E=dict(mat=DARK, mode='grille', z=2.5, step=2, pattern=dict(period=2, dir='h'), tags=('vent',)),
            K=dict(mat=DARK, mode='flat', z=2.5, step=0, tags=('exhaust',)),
        )
        vpaint(b, [
            '.fffffGGGGG',
            'ffffffGGGGG',
            'fffffffGGGG',
            '..M..SGGGGG',
            '..M..SDDDDD',
            '..M..SDDDDD',
            '..M..SDDDDD',
            '..M..SDDDDD',
            '..M..SDDDDD',
            '..M..SDDDDD',
            '..M..SDDDDD',
            '..M..SDDDDD',
            '..M..SDDDDD',
            '..M..SDDDDD',
            '..M..SDDDDD',
            '..M..SDDDDD',
            '..M..SDDDDD',
            '..M..SDEEED',
            '..M..SDEEED',
            '..M..SDDDDD',
            '..M..SDDDDD',
            'fffffffDDDD',
            'ffffffKDDDD',
            '.fffffDKDDD',
        ], -11, -12, L)
        turret_ring(b, 8.0, 7.0, -0.5)
        b.lamp(rect(-3, -11, 2, -11), 'marker', z=2.6, kind='marker', name='lightbar')
        for (x, nm) in ((-10, 'head_lo'), (-7, 'head_li'), (6, 'head_ri'), (9, 'head_ro')):
            headlamp(b, x, -12, nm, z=3.4)
        for (x, nm) in ((-10, 'tail_lo'), (9, 'tail_ro')):
            taillamp(b, x, 11, nm, z=3.4)

    def turret(self, b, pose):
        body = b.ctx.body
        for i, xc2 in enumerate((-10, 10)):
            r = self.recoil(pose, i, 2)
            b.barrel(xc2, 4, -16, -6, mat=STEEL, z=2.5, recoil=r)
            x0 = (xc2 - 4) // 2
            b.part(rect(x0, -16 + r, x0 + 3, -16 + r), STEEL, 'cylv', 2.7, step=1, name='muzzle%d' % i, tags=('barrel',))
            b.part(rect(x0, -8 + r, x0 + 3, -8 + r), STEEL, 'cylv', 3.6, step=-1, name='collar%d' % i, tags=('barrel',))
        faces(b, [
            '...aaaa',
            '..bRRRR',
            '.bRRRRR',
            'bRRRRRR',
            'cRRRRRR',
            'cRRRRRR',
            'cRRRRRR',
            'cRRRRRR',
            'dRRRRRR',
            '.dRRRRR',
            '..eeeee',
        ], -7, -6, body, 3, HEX_STEPS)
        # Gun mantlets bolted over the front cheeks.
        vpaint(b, ['mmmmmm', 'mmmmmm', 'mmmm..'], -8, -7,
               dict(m=dict(mat=body, mode='plate', z=3.5, step=-1, corner=False, name='mantlet', tags=('armor',),
                           split=True)))
        b.lamp({(-1, -6), (0, -6)}, 'sensor', z=4, kind='sensor', name='eye')
        # Commander's cupola with its MG.
        b.dome(circle(3.5, 1.0, 2.1), STEEL, 4, gain=0.5, name='cupola')
        b.part(rect(3, -3, 3, -1), GUNMETAL, 'flat', 4.4, step=1, name='cupola_mg')
        b.glass({(2, 0), (3, 0)}, 4.2)
        b.part(rect(-5, 0, -3, 2), body, 'inset', 3.5, name='hatch', tags=('hatch',))
        # A roof vent over the breech, a searchlight on the left cheek.
        b.grille(-2, 2, 1, 3, DARK, 3.3, period=2, dir='h', step=2)
        b.part(rect(-7, -4, -6, -3), STEEL, 'plate', 3.6, step=-1, corner=False, name='searchlight_box')
        b.lamp({(-7, -4)}, 'lamp', z=3.8, kind='spot', name='searchlight')
        whip(b, -5, 3, 3)
        b.meta['muzzles'] = [(-5.0, -16), (5.0, -16)]

    def hardpoints(self, ctx):
        return dict(minigun=(8, -1), laser=(-8, -1), missiles=(0, 2), flame=(-8, 3), plasma=[(-5.0, -16), (5.0, -16)])


# ---------------------------------------------------------------------------
# Leviathan - super-long siege: one massive gun with an evacuator and a
# double-baffle brake on a big two-tier round turret, road wheels showing
# down both runs, a pair of whips, a strapped bustle, twin engine decks.
# Lights: blue strips on the glacis, four headlamps, the eye, tail lamps.
# ---------------------------------------------------------------------------
@LINE.design('leviathan')
class Leviathan(Design):
    codename = 'Behemoth'
    gun_w = 4
    blurb = 'Super-long siege: one massive braked gun on a big round turret, road wheels down both runs, twin engine decks.'

    def hull(self, b, f):
        body = b.ctx.body
        runs(b, -10, -7, -11, 11)
        L = dict(
            G=dict(mat=body, mode='facet', z=2.2, step=-1, normal=(0.0, -0.9), corner=False, name='glacis'),
            f=dict(mat=body, mode='plate', z=3, corner=False, name='fender', tags=('armor',), split=True),
            S=dict(mat=body, mode='plate', z=2.6, step=-1, name='lip', tags=('skirt',), split=True),
            D=dict(mat=body, mode='plate', z=2, name='deck'),
            E=dict(mat=DARK, mode='grille', z=2.5, step=2, pattern=dict(period=2, dir='h'), tags=('vent',)),
        )
        vpaint(b, [
            '..ffGGGGGG',
            '.fffGGGGGG',
            'ffffGGGGGG',
            '...SGGGGGG',
            '...SDDDDDD',
            '...SDDDDDD',
            '...SDDDDDD',
            '...SDDDDDD',
            '...SDDDDDD',
            '...SDDDDDD',
            '...SDDDDDD',
            '...SDDDDDD',
            '...SDDDDDD',
            '...SDDDDDD',
            '...SDDDDDD',
            '...SDDDDDD',
            '...SDDDDDD',
            '...SDDDDDD',
            '...SDEEEDD',
            '...SDEEEDD',
            '...SDDDDDD',
            '...SDEEEDD',
            '...SDEEEDD',
            'ffffDDDDDD',
            'ffffDDDDDD',
            '.fffDDDDDD',
        ], -10, -13, L)
        turret_ring(b, 8.2, 7.2, -1.0)
        # Road wheels: a hub between every other pair of links, down both runs.
        hubs = set()
        for y in range(-9, 10, 3):
            hubs |= {(-9, y), (-9, y + 1)}
        b.part(sym(hubs), STEEL, 'flat', 1.5, step=1, name='hubs', tags=('wheel',))
        b.lamp(rect(-4, -12, -3, -12), 'marker', z=2.6, kind='marker', name='strip_l')
        b.lamp(rect(2, -12, 3, -12), 'marker', z=2.6, name='strip_r')
        for (x, nm) in ((-8, 'head_l'), (-6, 'head_l2'), (5, 'head_r2'), (7, 'head_r')):
            headlamp(b, x, -13, nm, z=3.4)
        taillamp(b, -9, 12, 'tail_l', z=3.4)
        taillamp(b, 8, 12, 'tail_r', z=3.4)

    def turret(self, b, pose):
        body = b.ctx.body
        r = self.recoil(pose)
        b.barrel(0, 4, -17, -7, mat=STEEL, z=2.5, recoil=r)
        brake2(b, 0, 4, -17, r)
        b.part(rect(-3, -12 + r, 2, -11 + r), STEEL, 'cylv', 2.7, name='evacuator', tags=('barrel',))
        shell = circle(0.0, -0.5, 7.2)
        b.part(shell, body, 'map', 3, stepmap=rim_shade(shell, 0.0, -0.5, lit=1, shade=-1, deep=-1, top=-1),
               name='shell')
        # The roof: a raised disc inside the sloped rim.
        roof = circle(0.0, 0.0, 5.2)
        b.part(roof, body, 'map', 3.2, stepmap=rim_shade(roof, 0.0, 0.0, lit=1, shade=0, top=0), name='upper_roof')
        b.part(rect(-4, 6, 3, 6), body, 'plate', 3.1, step=-1, corner=False, name='bustle', tags=('stowage',))
        b.part(rect(-2, 6, -2, 6) | rect(1, 6, 1, 6), BRASS, 'flat', 3.15, step=-1, name='bustle_strap',
               tags=('stowage',))
        b.part(rect(-3, -8, 2, -6), body, 'plate', 3.5, step=-1, corner=False, name='mantlet', tags=('armor',))
        b.part(rect(-2, -9 + r, 1, -9 + r), STEEL, 'cylv', 3.6, step=-1, name='collar', tags=('barrel',))
        b.lamp({(-3, -6)}, 'sensor', z=4, kind='sensor', name='eye')
        b.dome(circle(3.5, 0.5, 2.0), STEEL, 4, gain=0.5, name='cupola')
        b.glass({(3, -1), (4, -1)}, 4.2)
        b.part(rect(-5, -1, -3, 1), body, 'inset', 3.5, name='hatch', tags=('hatch',))
        # The antenna array: a pair of whips swept back off the left rear.
        whip(b, -5, 3, 3)
        whip(b, -3, 4, 3)
        b.meta['muzzles'] = [(0.0, -17)]

    def hardpoints(self, ctx):
        return dict(minigun=(8, -3), laser=(-9, -3), missiles=(0, 3), flame=(-9, 2), plasma=[(0.0, -17)])
