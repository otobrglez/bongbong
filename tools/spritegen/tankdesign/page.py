"""Assemble the Motor Pool review page from a build directory.

  python3 page.py [--build DIR] [--out FILE] [--density DIR]

Reads `render.py build`'s atlases and designs.json, today's sheet, the
ground and field renders (mapshot, in the output directory's maps/), the default
map's cells for the field test's walkable grid, and writes one HTML file
with everything inlined (the Artifact page contract: no external assets).
"""

import base64
import json
import os
import sys
import tomllib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from kit import CHASSIS, CHASSIS_ORDER  # noqa: E402
from render import LINES, REPO, SCRATCH  # noqa: E402

PASSABLE = {'tall_grass', 'road', 'pickup', 'start', 'start2', 'oil', 'gate'}


def data_uri(path, mime='image/png'):
    with open(path, 'rb') as f:
        return 'data:%s;base64,%s' % (mime, base64.b64encode(f.read()).decode())


def grid(map_path):
    m = tomllib.load(open(map_path, 'rb'))
    cols, rows = int(m['size'][0]), int(round(m['size'][1]))
    blocked = []
    for key, cell in m.get('cells', {}).items():
        c, r = (int(v) for v in key.split(','))
        if cell.get('kind') not in PASSABLE:
            blocked.append(r * cols + c)
    return cols, rows, sorted(blocked)


def main(argv):
    opts = {}
    i = 0
    while i < len(argv):
        opts[argv[i].lstrip('-')] = argv[i + 1]
        i += 2
    build = opts.get('build', os.path.join(SCRATCH, 'build'))
    out = opts.get('out', os.path.join(SCRATCH, 'motor-pool.html'))
    designs = json.load(open(os.path.join(build, 'designs.json')))
    for d in designs:
        d['atlas'] = data_uri(os.path.join(build, d['atlas']))
    lines = []
    for k in LINES:
        p = os.path.join(build, 'line-%s.json' % k)
        if os.path.exists(p):
            lines.append(json.load(open(p)))
    chassis = []
    for k in CHASSIS_ORDER:
        c = CHASSIS[k]
        chassis.append(dict(key=k, name=k.capitalize(), role=c['role'], tier=c['tier'], cls=c['cls'], fp=list(c['fp']),
                            guns=c['guns'], gun=c['gun'], turret=c['turret'], lat=c['lat'], muzzle=c['muzzle'], row=c['row'],
                            glow='#%02x%02x%02x' % tuple(c['glow'][:3])))
    maps = os.path.join(SCRATCH, 'maps')
    cols, rows, blocked = grid(os.path.join(REPO, 'maps', 'default.toml'))
    field = dict(cols=cols, rows=rows, blocked=blocked,
                 img=dict(grass=data_uri(os.path.join(maps, 'default.png')),
                          desert=data_uri(os.path.join(maps, 'default-desert.png'))))
    # The desert map is the same layout re-themed; fall back to the grass
    # grid if it is not.
    try:
        dc, dr, db = grid(os.path.join(REPO, 'maps', 'default-desert.toml'))
        if (dc, dr) == (cols, rows) and db != blocked:
            field['blocked'] = sorted(set(blocked) | set(db))
    except Exception:
        pass
    data = dict(
        S=40,
        lines=lines,
        chassis=chassis,
        designs=designs,
        today=data_uri(os.path.join(REPO, 'static', 'scifi_tanks_sheet.png')),
        grounds=dict(grass=data_uri(os.path.join(maps, 'showroom.png')),
                     desert=data_uri(os.path.join(maps, 'showroom-desert.png'))),
        field=field,
    )
    # Earlier versions of redrawn designs, for a before/after (their atlases
    # carry the teams they were built with).
    prev = opts.get('prev')
    if prev and os.path.exists(os.path.join(prev, 'designs.json')):
        for d in json.load(open(os.path.join(prev, 'designs.json'))):
            d['atlas'] = data_uri(os.path.join(prev, d['atlas']))
            data['designs'].append(d)
    dens = opts.get('density')
    if dens and os.path.exists(os.path.join(dens, 'density.json')):
        D = json.load(open(os.path.join(dens, 'density.json')))
        D['img'] = {k: data_uri(os.path.join(dens, v)) for k, v in D['img'].items()}
        data['density'] = D
    blob = json.dumps(data, separators=(',', ':')).replace('</', '<\\/')
    tpl = open(os.path.join(HERE, 'page', 'template.html')).read()
    app = open(os.path.join(HERE, 'page', 'app.js')).read()
    html = tpl.replace('__DATA_JSON__', blob).replace('__APP_JS__', app)
    with open(out, 'w') as f:
        f.write(html)
    print('wrote', out, '%.1f MB' % (len(html) / 1e6), len(designs), 'designs')


if __name__ == '__main__':
    main(sys.argv[1:])
