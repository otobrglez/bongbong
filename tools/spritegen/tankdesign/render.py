"""Render the tank design study: per-design atlases + metadata for the
viewer page, and contact-sheet previews for working on a line.

  python3 render.py preview <line> [chassis ...] [--out F] [--scale N]
  python3 render.py build [--out DIR] [line ...]

Run under nix's Pillow:
  nix-shell -p "python3.withPackages (ps: [ps.pillow])" --run "python3 render.py ..."

Atlas layout (one PNG per design, 40 x 40 cells):
  columns 0-15  hull: tier t, track frame f at 4 t + f
  columns 16-19 wrecks: blown, gutted, husk, cookoff
  columns 20-31 turret: tier t, pose p (0 rest, 1 first barrel recoiled,
                2 second barrel / return) at 20 + 3 t + p
  column  32    broken turret (every wreck but `blown`, which throws it)
  two rows per team in TEAMS order (base, then emissive): enemy, player 1
  (blue), player 2 (pink), then the green, white and orange candidates for
  players 3 and 4; then two rows of weapon modules (team-neutral),
  base/emissive, columns in WEAPONS order with WEAPON_STATES frames each.
"""

import importlib
import json
import os
import sys
import zlib

from PIL import Image, ImageChops, ImageFilter

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from kit import (CHASSIS, CHASSIS_ORDER, S, HALF, TEAMS, WEAPONS, WEAPON_STATES, Builder, Ctx, compose, resolve,  # noqa: E402
                 to_images, bbox)
from damage import (TIERS, WRECKS, apply_pixels, break_turret_parts, broken_turret_pixels, damage_parts,  # noqa: E402
                    make_plan, seed_of)

LINES = ['vanguard', 'skimmer', 'foundry', 'prototype']
REPO = os.path.normpath(os.path.join(HERE, '..', '..', '..'))
# Where previews, hero sheets and the review page's build land
# (`TANKDESIGN_OUT`, else the repo's gitignored target/).
SCRATCH = os.environ.get('TANKDESIGN_OUT', os.path.join(REPO, 'target', 'tankdesign'))
GROUND = {
    'grass': os.path.join(SCRATCH, 'maps', 'showroom.png'),
    'desert': os.path.join(SCRATCH, 'maps', 'showroom-desert.png'),
}

N_HULL = 16
C_WRECK = 16
C_TURRET = 20
C_BROKEN = 32
N_COLS = 33
MODULE_COLS = sum(WEAPON_STATES[w] for w in WEAPONS)
# The modules a preview draws on one tank at once: the grenade launcher, the
# sonic hammer, the EMP projector and the FPV relay share the missiles' roof,
# the gauss rail the laser's cheek, and a tank carries one special at a time.
SHOWN_TOGETHER = [w for w in WEAPONS if w not in ('grenade', 'sonic', 'emp', 'gauss', 'fpv', 'rod', 'well')]


def load_line(key):
    mod = importlib.import_module('lines.' + key)
    return mod.LINE


# ---------------------------------------------------------------------------
# Rendering one design
# ---------------------------------------------------------------------------
class Rendered:
    pass


def _parts(design, ctx, layer, frame=0, pose=0):
    b = Builder(ctx, layer, frame, pose)
    if layer == 'hull':
        design.hull(b, frame)
    else:
        design.turret(b, pose)
    return b


def design_key(design):
    return '%s.%s' % (design.line.key, design.chassis)


def render_design(design, teams=TEAMS, want_modules=True):
    """Every cell of one design, for every team."""
    key = design_key(design)
    ctx0 = Ctx(design.chassis, 'enemy')
    hb = _parts(design, ctx0, 'hull', 0)
    tb = _parts(design, ctx0, 'turret', 0, 0)
    hplan = make_plan(key, 'hull', hb.parts)
    tplan = make_plan(key, 'turret', tb.parts)
    s_h = seed_of(key, 'hull')
    s_t = seed_of(key, 'turret')
    iglow = getattr(design, 'internal_glow', 'fire')
    out = Rendered()
    out.design = design
    out.cells = {}
    for team in teams:
        ctx = Ctx(design.chassis, team)
        cells = {}
        for tier in range(4):
            for f in range(4):
                b = _parts(design, ctx, 'hull', f)
                parts = damage_parts(b.parts, tier, hplan, None, 'hull', iglow)
                grid = compose(parts, f)
                apply_pixels(grid, tier, hplan, f, None, s_h, 'hull')
                resolve(grid, ctx, f)
                cells['hull', tier, f] = to_images(grid)
        for w in WRECKS:
            b = _parts(design, ctx, 'hull', 0)
            parts = damage_parts(b.parts, 3, hplan, w, 'hull', iglow)
            grid = compose(parts, 0)
            apply_pixels(grid, 3, hplan, 0, w, s_h + WRECKS.index(w), 'hull')
            resolve(grid, ctx, 0)
            cells['wreck', w] = to_images(grid)
        for tier in range(4):
            for pose in range(3):
                b = _parts(design, ctx, 'turret', 0, pose)
                parts = damage_parts(b.parts, tier, tplan, None, 'turret', iglow)
                grid = compose(parts, 0)
                apply_pixels(grid, tier, tplan, 0, None, s_t, 'turret')
                resolve(grid, ctx, 0)
                cells['turret', tier, pose] = to_images(grid)
        b = _parts(design, ctx, 'turret', 0, 0)
        parts = break_turret_parts(b.parts, tplan)
        parts = damage_parts(parts, 3, tplan, 'husk', 'turret', iglow)
        grid = compose(parts, 0)
        broken_turret_pixels(grid, tplan, s_t + 9)
        resolve(grid, ctx, 0)
        cells['broken'] = to_images(grid)
        out.cells[team] = cells
    # Metadata from the pristine enemy render.
    out.hull_lights = hb.lights
    out.turret_lights = tb.lights
    out.hull_meta = hb.meta
    out.turret_meta = tb.meta
    hp = design.hardpoints(ctx0)
    out.hardpoints = hp
    out.modules = {}
    if want_modules:
        for w in WEAPONS:
            for st in range(WEAPON_STATES[w]):
                b = Builder(ctx0, 'module', 0, 0)
                design.module(b, w, st, hp)
                if b.parts:
                    grid = compose(b.parts, st)
                    resolve(grid, ctx0, st)
                    out.modules[w, st] = to_images(grid)
                else:
                    out.modules[w, st] = (Image.new('RGBA', (S, S)), Image.new('RGBA', (S, S)))
    return out


def atlas(r):
    mrow = 2 * len(TEAMS)
    img = Image.new('RGBA', (N_COLS * S, (mrow + 2) * S), (0, 0, 0, 0))
    for ti, team in enumerate(TEAMS):
        cells = r.cells.get(team)
        if not cells:
            continue
        def put(col, pair):
            img.paste(pair[0], (col * S, (2 * ti) * S))
            img.paste(pair[1], (col * S, (2 * ti + 1) * S))
        for tier in range(4):
            for f in range(4):
                put(tier * 4 + f, cells['hull', tier, f])
        for i, w in enumerate(WRECKS):
            put(C_WRECK + i, cells['wreck', w])
        for tier in range(4):
            for pose in range(3):
                put(C_TURRET + tier * 3 + pose, cells['turret', tier, pose])
        put(C_BROKEN, cells['broken'])
    col = 0
    for w in WEAPONS:
        for st in range(WEAPON_STATES[w]):
            pair = r.modules.get((w, st))
            if pair:
                img.paste(pair[0], (col * S, mrow * S))
                img.paste(pair[1], (col * S, (mrow + 1) * S))
            col += 1
    return img


def meta(r):
    d = r.design
    return dict(
        id=design_key(d), line=d.line.key, chassis=d.chassis, codename=d.codename, blurb=d.blurb,
        locomotion=d.locomotion, marks=d.marks, hover=d.hover,
        hull_lights=r.hull_lights, turret_lights=r.turret_lights,
        hardpoints={k: list(v) if isinstance(v, tuple) else v for k, v in r.hardpoints.items()},
        muzzles=r.turret_meta.get('muzzles', []), hull=r.hull_meta, turret=r.turret_meta,
        emitters=getattr(d, 'emitters', {}),
        teams=list(TEAMS),
    )


# ---------------------------------------------------------------------------
# Previews
# ---------------------------------------------------------------------------
_ground_cache = {}


def ground_patch(w, h, theme='grass', seed=0):
    """A patch of the real ground render (field pixels, 1:1)."""
    key = (theme,)
    if key not in _ground_cache:
        _ground_cache[key] = Image.open(GROUND[theme]).convert('RGBA')
    g = _ground_cache[key]
    # Stay inside the border walls (one 32 px cell).
    gx0, gy0 = 40, 40
    gw, gh = g.size[0] - 80 - w, g.size[1] - 80 - h
    x = gx0 + (seed * 37) % max(1, gw)
    y = gy0 + (seed * 53) % max(1, gh)
    return g.crop((x, y, x + w, y + h))


def rot(img, deg):
    """Rotate a cell clockwise by `deg` (the game's rotation) about its
    centre, nearest neighbour, like raylib's DrawTexturePro."""
    if deg % 360 == 0:
        return img
    return img.rotate(-deg, resample=Image.NEAREST, center=(img.size[0] / 2.0, img.size[1] / 2.0))


def shadow_of(img, alpha=0.486):
    a = img.split()[3].point(lambda v: int(v * alpha))
    sh = Image.new('RGBA', img.size, (0, 0, 0, 255))
    sh.putalpha(a)
    return sh


def stack(layers, hull_rot=0, turret_rot=0, hover=0):
    """(base, emissive) of hull + turret + modules composed at 2x on a
    transparent 80 x 80 tile, rotated, with the shadow."""
    T = S * 2
    base = Image.new('RGBA', (T + 8, T + 8), (0, 0, 0, 0))
    emis = Image.new('RGBA', (T + 8, T + 8), (0, 0, 0, 0))
    off = 4
    sh_off = (int(round(0.595 * (3 + hover * 2))), int(round(0.48 * (3 + hover * 2))))
    rendered = []
    for (b, e, which) in layers:
        r = hull_rot if which == 'hull' else turret_rot
        b2 = rot(b.resize((T, T), Image.NEAREST), r)
        e2 = rot(e.resize((T, T), Image.NEAREST), r)
        rendered.append((b2, e2))
    for (b2, e2) in rendered:
        base.alpha_composite(shadow_of(b2), (off + sh_off[0], off + sh_off[1]))
    for (b2, e2) in rendered:
        base.alpha_composite(b2, (off, off))
        # A layer hides the light beneath it before adding its own.
        cover = Image.new('RGBA', emis.size, (0, 0, 0, 0))
        cover.paste(b2, (off, off))
        keep = ImageChops.invert(cover.split()[3])
        a = ImageChops.multiply(emis.split()[3], keep)
        emis.putalpha(a)
        emis.alpha_composite(e2, (off, off))
    return base, emis


def on_ground(base, emis, theme='grass', seed=0, night=False):
    g = ground_patch(base.size[0], base.size[1], theme, seed).copy()
    g.alpha_composite(base)
    if night:
        amb = Image.new('RGBA', g.size, (70, 82, 128, 255))
        g = ImageChops.multiply(g, amb)
        glow = emis.filter(ImageFilter.GaussianBlur(3))
        g = ImageChops.add(g.convert('RGB'), glow.convert('RGB')).convert('RGBA')
        g = ImageChops.add(g.convert('RGB'), glow.convert('RGB')).convert('RGBA')
        e = emis.copy()
        g.alpha_composite(e)
    return g


def label(img, text):
    # Pillow's default bitmap font, one line.
    from PIL import ImageDraw
    d = ImageDraw.Draw(img)
    d.rectangle((0, 0, 6 * len(text) + 4, 11), fill=(20, 20, 20, 200))
    d.text((2, 0), text, fill=(240, 240, 240, 255))
    return img


def current_sprite(chassis, which):
    """Today's sheet cells, for comparison (32 x 32 padded to 40 x 40)."""
    sheet = Image.open(os.path.join(REPO, 'static', 'scifi_tanks_sheet.png')).convert('RGBA')
    row = CHASSIS[chassis]['row']
    col = {'hull': 0, 'turret': 1}[which]
    cell = sheet.crop((col * 32, row * 32, col * 32 + 32, row * 32 + 32))
    out = Image.new('RGBA', (S, S), (0, 0, 0, 0))
    out.paste(cell, (4, 4))
    return out


def preview_design(r, scale=2, theme='grass'):
    d = r.design
    e = r.cells['enemy']
    tiles = []

    def tile(layers, name, hull_rot=0, turret_rot=0, night=False, seed=0):
        b, em = stack(layers, hull_rot, turret_rot, d.hover)
        g = on_ground(b, em, theme, seed + CHASSIS_ORDER.index(d.chassis), night)
        g = g.resize((g.size[0] * scale // 1, g.size[1] * scale // 1), Image.NEAREST)
        return label(g, name)

    cur = [(current_sprite(d.chassis, 'hull'), Image.new('RGBA', (S, S)), 'hull'),
           (current_sprite(d.chassis, 'turret'), Image.new('RGBA', (S, S)), 'turret')]
    tiles.append(tile(cur, 'today'))
    H = lambda t, f=0, team='enemy': (r.cells[team]['hull', t, f][0], r.cells[team]['hull', t, f][1], 'hull')
    T = lambda t, p=0, team='enemy': (r.cells[team]['turret', t, p][0], r.cells[team]['turret', t, p][1], 'turret')
    tiles.append(tile([H(0), T(0)], 'new'))
    tiles.append(tile([H(0, 1), T(0, 1)], 'f1 fire', turret_rot=0))
    tiles.append(tile([H(0), T(0)], 'turn', hull_rot=90, turret_rot=35))
    tiles.append(tile([H(0), T(0)], 'night', night=True))
    tiles.append(tile([H(0, 0, 'p1'), T(0, 0, 'p1')], 'p1'))
    tiles.append(tile([H(0, 0, 'p2'), T(0, 0, 'p2')], 'p2'))
    for t in (1, 2, 3):
        tiles.append(tile([H(t), T(t)], TIERS[t]))
    tiles.append(tile([H(3, 1), T(3)], 'crit n', night=True))
    for w in WRECKS:
        layers = [(e['wreck', w][0], e['wreck', w][1], 'hull')]
        if w != 'blown':
            layers.append((e['broken'][0], e['broken'][1], 'turret'))
        tiles.append(tile(layers, w, turret_rot=20 if w == 'gutted' else -15))
    # Every module at once, but the ones that share the missiles' roof.
    mods = [H(0), T(0)]
    for w in SHOWN_TOGETHER:
        mb, me = r.modules[w, 0]
        mods.append((mb, me, 'turret'))
    tiles.append(tile(mods, 'all mods'))
    mods2 = [H(0), T(0)]
    for w in SHOWN_TOGETHER:
        mb, me = r.modules[w, min(1, WEAPON_STATES[w] - 1)]
        mods2.append((mb, me, 'turret'))
    tiles.append(tile(mods2, 'mods n', night=True))
    tw, th = tiles[0].size
    out = Image.new('RGBA', (tw * len(tiles), th), (30, 30, 30, 255))
    for i, t in enumerate(tiles):
        out.paste(t, (i * tw, 0))
    return out


def preview(line_key, chassis=None, out=None, scale=2, theme='grass'):
    line = load_line(line_key)
    rows = []
    for ch in (chassis or CHASSIS_ORDER):
        cls = line.designs.get(ch)
        if cls is None:
            continue
        r = render_design(cls(), teams=TEAMS)
        rows.append(preview_design(r, scale, theme))
    if not rows:
        print('nothing to preview')
        return
    w = max(im.size[0] for im in rows)
    h = sum(im.size[1] for im in rows)
    sheet = Image.new('RGBA', (w, h), (30, 30, 30, 255))
    y = 0
    for im in rows:
        sheet.paste(im, (0, y))
        y += im.size[1]
    out = out or os.path.join(SCRATCH, 'preview_%s.png' % line_key)
    sheet.save(out)
    print('wrote', out, sheet.size)


def cells_sheet(line_key, chassis, out=None, zoom=6, team='enemy'):
    """Raw cells of one design at a big zoom on flat grey: every hull tier,
    turret pose and tier, wreck, broken turret and module state."""
    line = load_line(line_key)
    r = render_design(line.designs[chassis](), teams=[team])
    c = r.cells[team]
    order = [('hull', t, 0) for t in range(4)] + [('hull', 0, f) for f in (1, 2, 3)] + [('wreck', w) for w in WRECKS] + \
            [('turret', t, 0) for t in range(4)] + [('turret', 0, 1), ('turret', 0, 2), 'broken']
    imgs = [c[k] for k in order]
    mods = [r.modules[w, st] for w in WEAPONS for st in range(WEAPON_STATES[w])]
    per = 11
    rows = [imgs[i:i + per] for i in range(0, len(imgs), per)] + [mods[i:i + per] for i in range(0, len(mods), per)]
    W = per * S * zoom
    sheet = Image.new('RGBA', (W, len(rows) * S * zoom * 2), (46, 50, 58, 255))
    for ri, row in enumerate(rows):
        for ci, (b, e) in enumerate(row):
            x, y = ci * S * zoom, ri * S * zoom * 2
            bg = Image.new('RGBA', (S, S), (96, 148, 64, 255))
            bg.alpha_composite(b)
            sheet.paste(bg.resize((S * zoom, S * zoom), Image.NEAREST), (x, y))
            ne = Image.new('RGBA', (S, S), (18, 20, 30, 255))
            ne.alpha_composite(e)
            sheet.paste(ne.resize((S * zoom, S * zoom), Image.NEAREST), (x, y + S * zoom))
    out = out or os.path.join(SCRATCH, 'cells_%s_%s.png' % (line_key, chassis))
    sheet.save(out)
    print('wrote', out, sheet.size)


def hero(line_key, chassis_list, out=None, zoom=8, states=None):
    """Composites (hull + turret, shadows, on the real ground) at a big
    zoom: the view for judging the pixel work itself."""
    line = load_line(line_key)
    states = states or ['new', 'p1', 'turn', 'night', 'damaged', 'critical', 'gutted', 'mods']
    rows = []
    for ch in chassis_list:
        r = render_design(line.designs[ch]())
        e = r.cells['enemy']
        d = r.design
        tiles = []
        for st in states:
            H = lambda t=0, f=0, team='enemy': (r.cells[team]['hull', t, f][0], r.cells[team]['hull', t, f][1], 'hull')
            T = lambda t=0, p=0, team='enemy': (r.cells[team]['turret', t, p][0], r.cells[team]['turret', t, p][1], 'turret')
            night = False
            hr, tr = 0, 0
            if st == 'new':
                layers = [H(), T()]
            elif st == 'today':
                layers = [(current_sprite(ch, 'hull'), Image.new('RGBA', (S, S)), 'hull'),
                          (current_sprite(ch, 'turret'), Image.new('RGBA', (S, S)), 'turret')]
            elif st in ('p1', 'p2'):
                layers = [H(team=st), T(team=st)]
            elif st == 'turn':
                layers = [H(), T()]
                hr, tr = 90, 45
            elif st == 'night':
                layers = [H(), T()]
                night = True
            elif st in TIERS:
                t = TIERS.index(st)
                layers = [H(t), T(t)]
            elif st in WRECKS:
                layers = [(e['wreck', st][0], e['wreck', st][1], 'hull')]
                if st != 'blown':
                    layers.append((e['broken'][0], e['broken'][1], 'turret'))
            elif st.startswith('mod:'):
                _, w, n = st.split(':')
                layers = [H(), T()] + [(r.modules[w, int(n)][0], r.modules[w, int(n)][1], 'turret')]
            elif st == 'mods':
                layers = [H(), T()] + [(r.modules[w, 0][0], r.modules[w, 0][1], 'turret') for w in SHOWN_TOGETHER]
            else:
                continue
            b, em = stack(layers, hr, tr, d.hover)
            g = on_ground(b, em, 'grass', CHASSIS_ORDER.index(ch), night)
            # Crop to the middle 64 x 64 screen px (the tank at 2x).
            cx, cy = g.size[0] // 2, g.size[1] // 2
            g = g.crop((cx - 34, cy - 40, cx + 34, cy + 34))
            tiles.append(label(g.resize((g.size[0] * zoom // 2, g.size[1] * zoom // 2), Image.NEAREST), st))
        tw, th = tiles[0].size
        row = Image.new('RGBA', (tw * len(tiles), th), (30, 30, 30, 255))
        for i, t in enumerate(tiles):
            row.paste(t, (i * tw, 0))
        rows.append(row)
    w = max(im.size[0] for im in rows)
    sheet = Image.new('RGBA', (w, sum(im.size[1] for im in rows)), (30, 30, 30, 255))
    y = 0
    for im in rows:
        sheet.paste(im, (0, y))
        y += im.size[1]
    out = out or os.path.join(SCRATCH, 'hero_%s.png' % line_key)
    sheet.save(out)
    print('wrote', out, sheet.size)


# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
def build(out_dir, lines=None):
    os.makedirs(out_dir, exist_ok=True)
    index = []
    for lk in (lines or LINES):
        try:
            line = load_line(lk)
        except ModuleNotFoundError:
            print('skip line', lk)
            continue
        for ch in CHASSIS_ORDER:
            cls = line.designs.get(ch)
            if cls is None:
                continue
            r = render_design(cls())
            a = atlas(r)
            name = '%s-%s.png' % (lk, ch)
            a.save(os.path.join(out_dir, name), optimize=True)
            m = meta(r)
            m['atlas'] = name
            index.append(m)
            print('built', name)
        index_line = dict(key=line.key, title=line.title, tagline=line.tagline, notes=line.notes)
        with open(os.path.join(out_dir, 'line-%s.json' % lk), 'w') as f:
            json.dump(index_line, f, indent=1)
    with open(os.path.join(out_dir, 'designs.json'), 'w') as f:
        json.dump(index, f, indent=1)
    print('designs:', len(index))


def main(argv):
    if not argv:
        print(__doc__)
        return
    cmd = argv[0]
    args = argv[1:]
    opts = {}
    pos = []
    i = 0
    while i < len(args):
        a = args[i]
        if a.startswith('--'):
            opts[a[2:]] = args[i + 1]
            i += 2
        else:
            pos.append(a)
            i += 1
    if cmd == 'preview':
        preview(pos[0], pos[1:] or None, opts.get('out'), int(opts.get('scale', 2)), opts.get('theme', 'grass'))
    elif cmd == 'cells':
        cells_sheet(pos[0], pos[1], opts.get('out'), int(opts.get('zoom', 6)), opts.get('team', 'enemy'))
    elif cmd == 'hero':
        hero(pos[0], pos[1:], opts.get('out'), int(opts.get('zoom', 8)),
             opts['states'].split(',') if 'states' in opts else None)
    elif cmd == 'build':
        build(opts.get('out', os.path.join(SCRATCH, 'build')), pos or None)
    else:
        print(__doc__)


if __name__ == '__main__':
    main(sys.argv[1:])
