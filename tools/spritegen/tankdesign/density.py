"""The density study: the Vanguard Assault at the game's 2-px grid (the
line's own design, cropped from its atlas) beside the same design redrawn
at twice the pixel density - an 80 x 80 cell drawn at 1x, the same size on
screen. Run with TANKDESIGN_CELL=80 (the kit reads its cell size at import):

  TANKDESIGN_CELL=80 python3 density.py [--build DIR] [--out DIR]
"""

import json
import os
import sys

os.environ.setdefault('TANKDESIGN_CELL', '80')
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from PIL import Image  # noqa: E402

from kit import *  # noqa: E402,F401,F403
from kit import S, Builder, Ctx, compose, resolve, to_images  # noqa: E402

assert S == 80, 'run with TANKDESIGN_CELL=80'


class AssaultHD(Design):
    """Vanguard's Assault at 1-px density: the same plan, twice the
    pixels. Parts sit one pixel inside the doubled footprint
    (-16, -22, 15, 21)."""

    def hull(self, b, f):
        body = b.ctx.body
        # Tracks: 6 px runs, links every 4 rows.
        b.tread(-15, -10, -19, 18, 'L', z=1, period=4)
        b.tread(9, 14, -19, 18, 'R', z=1, period=4)
        # Road-wheel hubs show between the links on the inner half.
        for y in range(-16, 17, 6):
            b.part({(-12, y), (-12, y + 1)}, STEEL, 'flat', 1.2, step=-1, tags=('wheel',))
            b.part({(11, y), (11, y + 1)}, STEEL, 'flat', 1.2, step=-2, tags=('wheel',))
        L = dict(
            F=dict(mat=body, mode='plate', z=3, name='fender', tags=('armor',)),
            G=dict(mat=body, mode='facet', z=2.2, normal=(0.0, -0.9), name='glacis'),
            S=dict(mat=body, mode='plate', z=2.4, step=-1, name='lip', tags=('skirt',)),
            D=dict(mat=body, mode='plate', z=2, name='deck'),
            E=dict(mat=DARK, mode='grille', z=2.5, step=1, pattern=dict(period=2, dir='h'), tags=('vent',)),
            K=dict(mat=DARK, mode='flat', z=2.5, step=-1, tags=('exhaust',)),
        )
        rows = (['..FFFFFFGGGGGG',
                 '.FFFFFFFGGGGGG',
                 'FFFFFF..GGGGGG',
                 'FFFFFF..GGGGGG',
                 '.....SSGGGGGGG',
                 '.....SSGGGGGGG',
                 '.....SSGGGGGGG',
                 '.....SSGGGGGGG'] +
                ['.....SSDDDDDDD'] * 24 +
                ['.....SSDDEEEEE'] * 4 +
                ['FFFFFFFDDDDDDD',
                 'FFFFFFFDDDDDDD',
                 '.FFFF...KKDDDD',
                 '..FF....KKDDDD'])
        b.paint(rows, -15, -21, L, mirror_x=True)
        # Panel lines engraved across the deck, rivets along the glacis.
        for y in (-12, 4):
            b.part({(x, y) for x in range(-8, 8)}, body, 'flat', 2.3, step=-2)
            b.part({(x, y + 1) for x in range(-8, 8)}, body, 'flat', 2.3, step=1)
        b.rivets({(x, -14) for x in (-7, -4, 3, 6)}, body, 2.6)
        b.rivets({(-11, y) for y in (-12, -6, 0, 6, 12)} | {(10, y) for y in (-12, -6, 0, 6, 12)}, body, 2.6)
        # Tow hooks and a tool rack.
        b.part({(-6, -21), (-6, -20), (5, -21), (5, -20)}, STEEL, 'flat', 3.2, step=-1)
        b.part(rect(-7, 14, 6, 14), STEEL, 'flat', 2.6, step=1, tags=('stowage',))
        b.part(rect(-7, 13, -5, 13), WOOD_LT_M, 'flat', 2.6, step=0, tags=('stowage',))
        b.part(ring(0, 0, 9.2, 7.0), STEEL, 'inset', 2.1, step=-2, tags=('ring',))
        # Lights: 2x2 lamps in dark housings, 3x1 tails, the teal strip.
        b.lamp(rect(-9, -19, -8, -18), 'lamp', z=4, kind='head', name='head_l', housing=DARK, housing_z=3.6)
        b.lamp(rect(7, -19, 8, -18), 'lamp', z=4, kind='head', name='head_r', housing=DARK, housing_z=3.6)
        b.lamp(rect(-10, 19, -8, 19), 'tail', z=4, kind='tail', name='tail_l')
        b.lamp(rect(7, 19, 9, 19), 'tail', z=4, kind='tail', name='tail_r')
        b.lamp(rect(-5, -16, 4, -16), 'marker', z=3, kind='marker', name='strip')

    def turret(self, b, pose):
        body = b.ctx.body
        for i, xc2 in enumerate((-12, 12)):
            r = self.recoil(pose, i, 2) * 2
            b.barrel(xc2, 4, -26, -10, mat=STEEL, z=2.5, recoil=r)
            x0 = (xc2 - 4) // 2
            # A bore evacuator and a slotted brake.
            b.part(rect(x0 - 1, -18 + r, x0 + 4, -16 + r), STEEL, 'cylv', 2.7, step=0, bevel=0)
            b.part(rect(x0 - 1, -26 + r, x0 + 4, -24 + r), STEEL, 'cylv', 2.7, step=-1, bevel=0)
            b.part({(x0 - 1, -25 + r), (x0 + 4, -25 + r)}, DARK, 'flat', 2.8, step=0)
        L = dict(
            m=dict(mat=STEEL, mode='plate', z=3.5, name='collar', tags=('armor',)),
            f=dict(mat=body, mode='facet', z=3.2, normal=(0.0, -0.8), name='brow'),
            c=dict(mat=body, mode='plate', z=2, step=-1, name='cheek'),
            R=dict(mat=body, mode='plate', z=3, name='roof'),
            k=dict(mat=body, mode='plate', z=2.2, step=-1, name='bustle', tags=('stowage',)),
            s=dict(mat=BRASS, mode='flat', z=2.3, step=-1, name='strap', tags=('stowage',)),
        )
        rows = (['..mmmmmmff',
                 '.mmmmmmmff',
                 'cmmmmmmmff',
                 'ccmmmmmmff'] +
                ['ccRRRRRRRR'] * 11 +
                ['.ccccccccc',
                 '..kkskkkks',
                 '..kkskkkks'])
        b.paint(rows, -10, -10, L, mirror_x=True)
        b.lamp(rect(-1, -9, 0, -8), 'sensor', z=4, kind='sensor', name='eye', housing=DARK, housing_z=3.8)
        # The loader's hatch, hinged, and the commander's cupola.
        b.part(rect(-8, 0, -4, 3), body, 'inset', 3.5, name='hatch', tags=('hatch',))
        b.part({(-6, 1), (-6, 2)}, STEEL, 'flat', 3.6, step=1, tags=('hatch',))
        b.dome(circle(4.0, 0.0, 3.4), STEEL, 4, gain=0.6, name='cupola')
        b.glass({(2, -3), (3, -3), (4, -3), (5, -3)}, 4.2)
        b.glass({(7, -1), (7, 0)}, 4.2)
        b.part({(4, 0), (3, 0)}, STEEL, 'flat', 4.3, step=2)
        # Seam, rivets, smoke dischargers, the whip antenna.
        b.part(rect(-8, 5, 7, 5), body, 'flat', 3.3, step=-2, name='seam')
        b.part(rect(-8, 6, 7, 6), body, 'flat', 3.3, step=1)
        b.rivets({(-8, -5), (7, -5), (-8, 8), (7, 8)}, body, 3.4)
        for y in (-3, -1):
            b.part({(-11, y), (-12, y)}, STEEL, 'flat', 2.4, step=-1)
            b.part({(10, y), (11, y)}, STEEL, 'flat', 2.4, step=-2)
        b.lamp({(8, 2), (8, 3)}, 'marker', z=4, name='status')
        b.part({(-9, 7)}, STEEL, 'flat', 3.6, step=0)
        b.antenna(-9, 13, 7, z=6)


WOOD_LT_M = Mat('handle', [WOOD_DARKEST, WOOD_DEEPER, WOOD_DK, WOOD_AMBER, WOOD_MD, WOOD_LT], 3)


def main(argv):
    opts = {}
    i = 0
    while i < len(argv):
        opts[argv[i].lstrip('-')] = argv[i + 1]
        i += 2
    scratch = os.environ.get('TANKDESIGN_OUT', os.path.join(HERE, '..', '..', '..', 'target', 'tankdesign'))
    build = opts.get('build', os.path.join(scratch, 'build'))
    out = opts.get('out', os.path.join(scratch, 'density'))
    team = opts.get('team', 'enemy')
    os.makedirs(out, exist_ok=True)
    d = AssaultHD()
    ctx = Ctx('assault', team)
    hd = Image.new('RGBA', (5 * S, S), (0, 0, 0, 0))
    for f in range(4):
        b = Builder(ctx, 'hull', f, 0)
        d.hull(b, f)
        grid = compose(b.parts, f)
        resolve(grid, ctx, f)
        hd.paste(to_images(grid)[0], (f * S, 0))
    b = Builder(ctx, 'turret', 0, 0)
    d.turret(b, 0)
    grid = compose(b.parts, 0)
    resolve(grid, ctx, 0)
    hd.paste(to_images(grid)[0], (4 * S, 0))
    hd.save(os.path.join(out, 'grid1.png'))
    # The line's own 2-px Assault: hull frames and the resting turret.
    atlas = Image.open(os.path.join(build, 'vanguard-assault.png')).convert('RGBA')
    ti = ['enemy', 'p1', 'p2'].index(team)
    g2 = Image.new('RGBA', (5 * 40, 40), (0, 0, 0, 0))
    for f in range(4):
        g2.paste(atlas.crop((f * 40, ti * 80, f * 40 + 40, ti * 80 + 40)), (f * 40, 0))
    g2.paste(atlas.crop((20 * 40, ti * 80, 21 * 40, ti * 80 + 40)), (4 * 40, 0))
    g2.save(os.path.join(out, 'grid2.png'))
    meta = dict(
        lede='You chose to keep the game\'s 2-pixel grid. For comparison, here is the Vanguard Assault both ways at the same '
             'size on screen: the design as it would ship, and the same plan redrawn with four times the pixels.',
        img=dict(grid2='grid2.png', grid1='grid1.png'),
        items=[
            dict(title='2-pixel grid (your choice)', img='grid2', cell=40,
                 caption='The Vanguard Assault as drawn for the game: the same chunky pixel as the walls, the ground and '
                         'every effect.'),
            dict(title='1-pixel grid (for comparison)', img='grid1', cell=80,
                 caption='The same design with four times the pixels: 1-px rivets and panel lines, slotted brakes, '
                         'periscopes, wheel hubs. Crisper than the world around it, and four times the drawing per tank.'),
        ],
    )
    with open(os.path.join(out, 'density.json'), 'w') as fjson:
        json.dump(meta, fjson, indent=1)
    print('wrote', out)


if __name__ == '__main__':
    main(sys.argv[1:])
