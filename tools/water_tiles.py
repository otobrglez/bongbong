"""Compose the water tiles the Puny World pack does not have.

`src/ground.rs` draws lakes through the pack's `river` corner autotile and
streams through its `water-paths` edge autotile. Two kinds of cell have no
tile in the pack:

- **Saddles**: a lake cell wet at two opposite corners only (TR+BL, TL+BR),
  which a diagonal run of water produces. The wangset has no diagonal
  entries.
- **Corner mouths**: a lake cell wet at one corner with a one-cell stream
  entering on one or both of its dry sides - where a stream meets the end
  or the side of a two-cell channel. The pack has mouths only for the four
  straight shores (275, 301, 303, 329).

Each is composed here from the pack's own tiles, never drawn by hand: the
water of the source tiles is united (a saddle is the two single-corner
shores; a corner mouth is the single-corner shore plus the half of the
straight stream tile that enters it), the bank pixels are kept where they
still face land, and the water is re-shaded so the shoreline highlight
runs only along the shore that is left. Every tile is composed four times,
once per frame of the sources' own animation, on the same geometry, so it
shimmers in step with its neighbours.

The composed tiles are written into transparent cells of the sheet
(`TILE_ORIGIN`): tile `k` frame `f` lands at row `ROW0 + f`, column
`COL0 + k`, so its ids are `id(k) + 27 * f` - `ground::WATER_EXTRA`
spells the same ids out. `tools/retint_ground.py` calls `compose` on the
pristine original before it retints, so both themes get the tiles in their
own tones and `_original/` is never written.

The same composition checks itself on the pack's own tile: the straight
top shore (278) plus a north stream reproduces the pack's 275 closely (see
`self_check`).
"""

from collections import deque

COLS = 27
T = 16

# Transparent cells in the original sheet: rows 22..25, columns 8..21.
ROW0, COL0 = 22, 8

# Corner bits as ground.rs indexes WATER_SHORE: bit3=TL bit2=TR bit1=BR
# bit0=BL. Stream sides as WATER_CHANNEL: bit3=N bit2=E bit1=S bit0=W.
TL, TR, BR, BL = 8, 4, 2, 1
N, E, S, W = 8, 4, 2, 1

# The order here is the order of `ground::WATER_EXTRA` - never reorder
# without updating it (the ids follow the index).
TILES = [
    (TR | BL, 0),
    (TL | BR, 0),
    (TL, E), (TL, S), (TL, E | S),
    (TR, S), (TR, W), (TR, S | W),
    (BR, N), (BR, W), (BR, N | W),
    (BL, N), (BL, E), (BL, N | E),
]

# The pack's single-corner shores and straight streams, each with its four
# animation frames (the `<animation>` elements of the .tsx).
FRAMES = {
    333: [333, 414, 495, 576],  # TL wet
    331: [331, 412, 493, 574],  # TR wet
    277: [277, 358, 439, 520],  # BR wet
    279: [279, 360, 441, 522],  # BL wet
    278: [278, 359, 440, 521],  # top shore, for the self-check
    297: [297, 405, 513, 621],  # stream N+S
    353: [353, 461, 569, 677],  # stream E+W
}
CORNER = {TL: 333, TR: 331, BR: 277, BL: 279}

# The pack's flat water, the tone of open water away from any shore.
PLAIN = (4, 160, 180)

# A stream half reaches this far into a tile where the base tile has no
# water on that line: the depth of the pack's own mouth cut.
STREAM_REACH = 9

# Shading bands kept from a source tile: a pixel this many px or more
# from land is open water whatever its source says.
SHADE_BANDS = 5

GRASS_FILL = [0, 1, 2, 27, 28, 29, 54, 55, 56]


def tile_id(k, frame=0):
    return (ROW0 + frame) * COLS + COL0 + k


def crop(px, tid):
    c, r = tid % COLS, tid // COLS
    return [[px[c * T + x, r * T + y] for x in range(T)] for y in range(T)]


def is_water(p):
    r, g, b = p[0], p[1], p[2]
    return p[3] > 0 and b > r + 40 and b > 90


def land_distance(wet):
    """Chebyshev distance from each pixel to the nearest land pixel."""
    far = T * 2
    d = [[0 if not wet[y][x] else far for x in range(T)] for y in range(T)]
    q = deque((x, y) for y in range(T) for x in range(T) if not wet[y][x])
    while q:
        x, y = q.popleft()
        for dy in (-1, 0, 1):
            for dx in (-1, 0, 1):
                nx, ny = x + dx, y + dy
                if 0 <= nx < T and 0 <= ny < T and d[ny][nx] > d[y][x] + 1:
                    d[ny][nx] = d[y][x] + 1
                    q.append((nx, ny))
    return d


class Sheet:
    def __init__(self, img):
        self.px = img.load()
        self.grass = set()
        for tid in GRASS_FILL:
            for row in crop(self.px, tid):
                self.grass.update(row)

    def tile(self, tid, frame):
        return crop(self.px, FRAMES[tid][frame] if tid in FRAMES else tid)

    def land_rank(self, p):
        """Bank (1) wins over plain grass (0) where two sources are land."""
        return 0 if p in self.grass else 1

    def union(self, parts, frame):
        """`parts` is a list of (tile id, region(x, y) -> bool)."""
        imgs = [(self.tile(t, frame), region) for t, region in parts]
        wet = [[[region(x, y) and is_water(im[y][x]) for x in range(T)] for y in range(T)] for im, region in imgs]
        merged = [[any(w[y][x] for w in wet) for x in range(T)] for y in range(T)]
        dist = land_distance(merged)
        own = [land_distance(w) for w in wet]
        out = [[None] * T for _ in range(T)]
        for y in range(T):
            for x in range(T):
                if not merged[y][x]:
                    land = [im[y][x] for im, region in imgs if region(x, y)]
                    out[y][x] = max(land, key=self.land_rank)
                    continue
                # A source's shading is kept only where its own shore is as
                # far away as the composed one, so no highlight is left
                # standing in open water where a source's shore used to be.
                need = min(dist[y][x], SHADE_BANDS)
                pick = [im[y][x] for (im, _), w, d in zip(imgs, wet, own) if w[y][x] and min(d[y][x], SHADE_BANDS) == need]
                out[y][x] = pick[0] if pick else PLAIN + (255,)
        return out

    def first_water(self, tid, side, i):
        """Distance from edge `side` along line `i` to the first water pixel."""
        im = self.tile(tid, 0)
        for d in range(T):
            x, y = {N: (i, d), S: (i, T - 1 - d), W: (d, i), E: (T - 1 - d, i)}[side]
            if is_water(im[y][x]):
                return d
        return None

    def stream_half(self, base, side):
        """The straight stream tile's half on `side`, cut to meet `base`'s water."""
        src = 297 if side in (N, S) else 353
        reach = []
        for i in range(T):
            fw = self.first_water(base, side, i)
            reach.append(STREAM_REACH if fw is None else min(fw + 1, T - 1))

        def region(x, y):
            i, d = {N: (x, y), S: (x, T - 1 - y), W: (y, x), E: (y, T - 1 - x)}[side]
            return d <= reach[i]

        return (src, region)

    def compose_tile(self, corners, sides, frame):
        whole = lambda x, y: True
        parts = [(CORNER[b], whole) for b in (TL, TR, BR, BL) if corners & b]
        base = parts[0][0]
        parts += [self.stream_half(base, s) for s in (N, E, S, W) if sides & s]
        return self.union(parts, frame)


def put(px, tid, tile):
    c, r = tid % COLS, tid // COLS
    for y in range(T):
        for x in range(T):
            px[c * T + x, r * T + y] = tile[y][x]


def compose(img):
    """Write every composed tile into `img` (the pristine RGBA original)."""
    sheet = Sheet(img)
    tiles = []
    for k, (corners, sides) in enumerate(TILES):
        for frame in range(4):
            tid = tile_id(k, frame)
            if any(sheet.px[tid % COLS * T + x, tid // COLS * T + y][3] for y in range(T) for x in range(T)):
                raise SystemExit(f"water_tiles: cell {tid} is not empty in the source sheet")
            tiles.append((tid, sheet.compose_tile(corners, sides, frame)))
    for tid, tile in tiles:
        put(sheet.px, tid, tile)
    return img


def self_check(img):
    """Pixels of the pack's 275 (top shore + north mouth) the composition gets wrong."""
    sheet = Sheet(img)
    built = sheet.union([(278, lambda x, y: True), sheet.stream_half(278, N)], 0)
    pack = crop(sheet.px, 275)
    return sum(is_water(built[y][x]) != is_water(pack[y][x]) for y in range(T) for x in range(T))


if __name__ == "__main__":
    import os
    from PIL import Image

    src = os.path.join(os.path.dirname(__file__), "..", "static", "punyworld", "_original", "punyworld-overworld-tileset.png")
    img = Image.open(src).convert("RGBA")
    print(f"self-check: {self_check(img)} of {T * T} pixels differ in water/land from the pack's 275")
    compose(img)
    for k, (corners, sides) in enumerate(TILES):
        print(f"{tile_id(k):4d}  corners {corners:04b}  stream {sides:04b}")
