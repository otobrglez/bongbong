"""Export the shipped tank art from the chosen design line.

  python3 export.py [--line vanguard] [--repo DIR]

Run under nix's Pillow from this directory:
  nix-shell -p "python3.withPackages (ps: [ps.pillow])" --run "python3 export.py"

Writes, under the repo (docs/SPRITESHEET_SPEC.md has the layout):
  static/scifi_tanks_sheet.png   the paint: 33 columns x 60 rows of 40 px cells,
                                 five team blocks of the twelve chassis
  static/scifi_tanks_glow.png    the light layer, the same layout
  static/tank_modules.png        the weapon modules: 28 columns x 12 rows
  static/tank_modules_glow.png   their light layer
  src/tank_art.rs                the anchors the engine reads (lamps, muzzles,
                                 missile tubes), generated - never edited by hand
"""

import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from PIL import Image  # noqa: E402

from kit import CHASSIS, CHASSIS_ORDER, S, WEAPONS, WEAPON_STATES, Builder, Ctx  # noqa: E402
from damage import WRECKS  # noqa: E402
from render import C_BROKEN, C_TURRET, C_WRECK, N_COLS, load_line, render_design  # noqa: E402

# The sheet's team blocks, top to bottom: the enemy, then players 1-4 (blue,
# pink, and the white and orange the owner picked for players 3 and 4).
BLOCKS = ['enemy', 'p1', 'p2', 'white', 'orange']
MODULE_COLS = sum(WEAPON_STATES[w] for w in WEAPONS)


def save_sheet(img, path):
    """Write `img` as an exact palette PNG - every RGBA colour it uses in the
    palette, alpha in tRNS - which decodes to the same pixels at about a
    third of the RGBA file's size (the art is a few dozen colours); RGBA if
    it ever uses more than a palette holds."""
    px = list(img.getdata())
    colours = sorted(set(px), key=lambda c: (c[3] != 0, c))
    if len(colours) > 256:
        img.save(path, optimize=True)
        return
    index = {c: i for i, c in enumerate(colours)}
    out = Image.new('P', img.size)
    out.putdata([index[c] for c in px])
    out.putpalette([v for (r, g, b, _) in colours for v in (r, g, b)])
    out.save(path, optimize=True, transparency=bytes(a for (_, _, _, a) in colours))


def fmt(v):
    s = ('%.1f' % v)
    return s if '.' in s else s + '.0'


def pair(p):
    return '(%s, %s)' % (fmt(p[0]), fmt(p[1]))


def main(argv):
    opts = {}
    i = 0
    while i < len(argv):
        opts[argv[i].lstrip('-')] = argv[i + 1]
        i += 2
    line_key = opts.get('line', 'vanguard')
    repo = opts.get('repo', os.path.normpath(os.path.join(HERE, '..', '..', '..')))
    line = load_line(line_key)
    rows = len(BLOCKS) * len(CHASSIS_ORDER)
    paint = Image.new('RGBA', (N_COLS * S, rows * S), (0, 0, 0, 0))
    glow = Image.new('RGBA', (N_COLS * S, rows * S), (0, 0, 0, 0))
    mod_paint = Image.new('RGBA', (MODULE_COLS * S, len(CHASSIS_ORDER) * S), (0, 0, 0, 0))
    mod_glow = Image.new('RGBA', (MODULE_COLS * S, len(CHASSIS_ORDER) * S), (0, 0, 0, 0))
    art = []
    for ci, ch in enumerate(CHASSIS_ORDER):
        design = line.designs[ch]()
        r = render_design(design, teams=BLOCKS)
        for bi, team in enumerate(BLOCKS):
            cells = r.cells[team]
            y = (bi * len(CHASSIS_ORDER) + ci) * S

            def put(col, pair_):
                paint.paste(pair_[0], (col * S, y))
                glow.paste(pair_[1], (col * S, y))
            for tier in range(4):
                for f in range(4):
                    put(tier * 4 + f, cells['hull', tier, f])
            for wi, w in enumerate(WRECKS):
                put(C_WRECK + wi, cells['wreck', w])
            for tier in range(4):
                for pose in range(3):
                    put(C_TURRET + tier * 3 + pose, cells['turret', tier, pose])
            put(C_BROKEN, cells['broken'])
        col = 0
        for w in WEAPONS:
            for st in range(WEAPON_STATES[w]):
                b, e = r.modules[w, st]
                mod_paint.paste(b, (col * S, ci * S))
                mod_glow.paste(e, (col * S, ci * S))
                col += 1
        # Anchors: the design's lamps and muzzles, and each module's muzzle
        # as its own function drew it.
        hp = r.hardpoints
        mm = {}
        for w in ('minigun', 'laser', 'flame', 'missiles', 'grenade', 'sonic'):
            b = Builder(Ctx(ch, 'enemy'), 'module', 0, 0)
            design.module(b, w, 0, hp)
            mm[w] = b.meta
        heads = [(l['x'], l['y']) for l in r.hull_lights if l['kind'] == 'head']
        spots = [(l['x'], l['y'], l.get('dir') or 0.0) for l in r.turret_lights if l['kind'] == 'spot']
        muzzles = [tuple(m) for m in r.turret_meta.get('muzzles', [])]
        art.append(dict(chassis=ch, codename=design.codename, heads=heads, spots=spots, muzzles=muzzles,
                        minigun=mm['minigun']['muzzle'], laser=mm['laser']['muzzle'], flame=mm['flame']['muzzle'],
                        grenade=mm['grenade']['muzzle'], sonic=mm['sonic']['muzzle'],
                        tubes=mm['missiles']['tubes']))
        print('exported', ch, design.codename)
    static = os.path.join(repo, 'static')
    save_sheet(paint, os.path.join(static, 'scifi_tanks_sheet.png'))
    save_sheet(glow, os.path.join(static, 'scifi_tanks_glow.png'))
    save_sheet(mod_paint, os.path.join(static, 'tank_modules.png'))
    save_sheet(mod_glow, os.path.join(static, 'tank_modules_glow.png'))
    write_rust(os.path.join(repo, 'src', 'tank_art.rs'), line_key, art)
    print('wrote the sheets and src/tank_art.rs')


def write_rust(path, line_key, art):
    L = []
    L.append('//! The anchors of the tank art, generated by `tools/spritegen/tankdesign/export.py`')
    L.append('//! from the `%s` design line (`lines/%s.py`) - regenerate, never edit by hand.' % (line_key, line_key))
    L.append('//! Every point is in design pixels from the pivot: x to the right, y toward')
    L.append('//! the tail (the gun points to -y), in the hull\'s frame (lamps) or the')
    L.append('//! turret\'s (muzzles, spotlights, modules). One design pixel is `Tank::scale`')
    L.append('//! world pixels. Indexed by chassis row (`TankKind::row`).')
    L.append('')
    L.append('/// Each chassis\'s design name, for the dev tools.')
    L.append('pub const CODENAMES: [&str; 12] = [%s];' % ', '.join('"%s"' % a['codename'] for a in art))
    L.append('')
    L.append('/// The headlamps a night throws its cones from (hull frame).')
    L.append('pub const HEADLIGHTS: [&[(f32, f32)]; 12] = [')
    for a in art:
        L.append('    &[%s], // %s' % (', '.join(pair(p) for p in a['heads']), a['chassis']))
    L.append('];')
    L.append('')
    L.append('/// Turret searchlights: (x, y, degrees off the gun line) - the cone')
    L.append('/// follows the aim (turret frame).')
    L.append('pub const SPOTLIGHTS: [&[(f32, f32, f32)]; 12] = [')
    for a in art:
        L.append('    &[%s], // %s' % (', '.join('(%s, %s, %s)' % (fmt(x), fmt(y), fmt(d)) for (x, y, d) in a['spots']),
                                    a['chassis']))
    L.append('];')
    L.append('')
    L.append('/// The main gun\'s muzzles at rest, one per barrel (turret frame).')
    L.append('pub const GUN_MUZZLES: [&[(f32, f32)]; 12] = [')
    for a in art:
        L.append('    &[%s], // %s' % (', '.join(pair(p) for p in a['muzzles']), a['chassis']))
    L.append('];')
    L.append('')
    for key, doc in (('minigun', 'The minigun module\'s muzzle, where its bullets leave (turret frame).'),
                     ('laser', 'The laser module\'s lens, where its beam starts (turret frame).'),
                     ('flame', 'The flamethrower module\'s nozzle, where its jet starts (turret frame).'),
                     ('grenade', 'The grenade launcher module\'s barrel mouth, where its grenades leave (turret frame).'),
                     ('sonic', 'The sonic hammer module\'s dish, where its wind-up is drawn (turret frame).')):
        L.append('/// %s' % doc)
        L.append('pub const %s_MUZZLE: [(f32, f32); 12] = [' % key.upper())
        for a in art:
            L.append('    %s, // %s' % (pair(a[key]), a['chassis']))
        L.append('];')
        L.append('')
    L.append('/// The missile launcher\'s four tube mouths, in firing order (turret frame).')
    L.append('pub const MISSILE_TUBES: [[(f32, f32); 4]; 12] = [')
    for a in art:
        L.append('    [%s], // %s' % (', '.join(pair(p) for p in a['tubes']), a['chassis']))
    L.append('];')
    with open(path, 'w') as f:
        f.write('\n'.join(L) + '\n')


if __name__ == '__main__':
    main(sys.argv[1:])
