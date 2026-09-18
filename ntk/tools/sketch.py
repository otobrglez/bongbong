"""Shared drawing helpers for the NTK slide sketches (white on black SVG,
rendered to PNG with rsvg-convert). Used by gen_pipeline.py and
gen_multiplayer.py."""
import subprocess

FONT = "Helvetica Neue, Helvetica, Arial, sans-serif"


class Sketch:
    def __init__(self, w=1920, h=1080):
        self.w, self.h = w, h
        self.out = [
            f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">',
            '<defs><marker id="a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" '
            'orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#fff"/></marker></defs>',
            f'<rect width="{w}" height="{h}" fill="#000"/>',
        ]

    def add(self, s):
        self.out.append(s)

    def box(self, x, y, w, h, lines, title=None, dashed=False, size=22, bold_first=False):
        dash = ' stroke-dasharray="10 8"' if dashed else ""
        self.add(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="10" fill="none" stroke="#fff" stroke-width="2.5"{dash}/>')
        n = len(lines) + (1 if title else 0)
        lh = size * 1.35
        cy = y + h / 2 - (n - 1) * lh / 2
        if title:
            self.text(x + w / 2, cy, title, size + 4, 700, "middle")
            cy += lh
        for i, ln in enumerate(lines):
            self.text(x + w / 2, cy, ln, size, 700 if (bold_first and i == 0) else 400, "middle")
            cy += lh

    def text(self, x, y, s, size=22, weight=400, anchor="start"):
        self.add(f'<text x="{x}" y="{y}" fill="#fff" font-family="{FONT}" font-size="{size}" font-weight="{weight}" '
                 f'text-anchor="{anchor}" dominant-baseline="middle">{s}</text>')

    def line(self, x1, y1, x2, y2, dashed=False):
        dash = ' stroke-dasharray="10 8"' if dashed else ""
        self.add(f'<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="#fff" stroke-width="2.5"{dash}/>')

    def arrow(self, x1, y1, x2, y2, both=False, dashed=False):
        dash = ' stroke-dasharray="10 8"' if dashed else ""
        start = ' marker-start="url(#a)"' if both else ""
        self.add(f'<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="#fff" stroke-width="2.5" marker-end="url(#a)"{start}{dash}/>')

    def region(self, x, y, w, h, label):
        """A dashed enclosure with a label on its top edge."""
        self.add(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="14" fill="none" stroke="#fff" stroke-width="2" stroke-dasharray="12 10"/>')
        self.add(f'<rect x="{x + 20}" y="{y - 14}" width="{len(label) * 13 + 20}" height="28" fill="#000"/>')
        self.text(x + 30, y, label, 20, 700)

    def save(self, svg_path, png_path, zoom=4):
        self.add("</svg>")
        open(svg_path, "w").write("\n".join(self.out))
        subprocess.run(["rsvg-convert", "-z", str(zoom), "-o", png_path, svg_path], check=True)
        print("wrote", svg_path, "and", png_path)
