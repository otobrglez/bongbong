"""Single-design big zoom: pristine composite, hull alone, turret alone
(design pixels at `zoom`, grid lines every pixel) - for pixel-level work."""
import sys, os
from PIL import Image, ImageDraw
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from render import load_line, render_design, SCRATCH, stack, on_ground, CHASSIS_ORDER
from kit import S

def main(line, ch, zoom=14, team='enemy', tier=0):
    r = render_design(load_line(line).designs[ch](), teams=[team])
    c = r.cells[team]
    hb, he = c['hull', tier, 0]
    tb, te = c['turret', tier, 0]
    comp = Image.new('RGBA', (S, S), (96, 148, 64, 255))
    comp.alpha_composite(hb); comp.alpha_composite(tb)
    tiles = []
    for im in (comp, hb, tb):
        bg = Image.new('RGBA', (S, S), (96, 148, 64, 255)); bg.alpha_composite(im)
        crop = bg.crop((6, 4, 34, 36))
        big = crop.resize((crop.size[0] * zoom, crop.size[1] * zoom), Image.NEAREST)
        d = ImageDraw.Draw(big)
        for i in range(crop.size[0] + 1):
            d.line([(i * zoom, 0), (i * zoom, big.size[1])], fill=(0, 0, 0, 40))
        for j in range(crop.size[1] + 1):
            d.line([(0, j * zoom), (big.size[0], j * zoom)], fill=(0, 0, 0, 40))
        # pivot cross
        px, py = (20 - 6) * zoom, (20 - 4) * zoom
        d.line([(px - 6, py), (px + 6, py)], fill=(255, 0, 255, 200)); d.line([(px, py - 6), (px, py + 6)], fill=(255, 0, 255, 200))
        tiles.append(big)
    out = Image.new('RGBA', (sum(t.size[0] for t in tiles) + 20, tiles[0].size[1]), (30, 30, 30, 255))
    x = 0
    for t in tiles:
        out.paste(t, (x, 0)); x += t.size[0] + 10
    p = os.path.join(SCRATCH, 'zoom_%s_%s.png' % (line, ch))
    out.save(p); print('wrote', p, out.size)

if __name__ == '__main__':
    a = sys.argv[1:]
    main(a[0], a[1], int(a[2]) if len(a) > 2 else 14, a[3] if len(a) > 3 else 'enemy', int(a[4]) if len(a) > 4 else 0)
