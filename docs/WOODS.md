# Woods — what makes trees read as a forest

`src/woods.rs`, BB-77, part of the woodland pass (BB-74; design:
the woodland pass artifact linked from BB-74). The trees themselves are
docs/TREES_SPEC.md and the bushes docs/BUSHES_SPEC.md. This is how a stand
of them is drawn so it reads as a wood rather than a grid of identical
crowns on 32 px cells.

All of it is worked out once, when a round starts, from the map alone
(`Woods::build`). It is hashed from cell positions and draws no RNG.
Nothing in it is read by a collider, the hit test, the nav grid or the AI,
so a seed replays exactly as it did before, and every map with trees
changes its picture with no map edit.

## 1. Trails

An open cell is a **trail** when wood lies within two cells on both sides
along its row or its column (open ground between), with at least two trees
among its eight neighbours, and at least three more such cells round it in
its 5 x 5 block. The last rule keeps a one- or two-cell notch in a wood's
edge from counting. `Woods::is_trail`. BB-78 builds on this: tracks,
cover and the builder's warning.

## 2. Crowns

Every tree's crown is drawn off its cell's centre by up to 4 px on each
axis, on the 2 px grid, and mirrored by its hash
(`obstacle::Obstacle::crown`, set at `init`, read by `draw_tree`, its
shadow and its hit flash). 4 px stays inside the 8 px a 48 px crown already
overhangs its cell, so the art never reaches further than it did.

- **Away from a trail or a wall** beside the tree. A crown leaning into
  a trail would close the way in, and one leaning over a wall would draw
  leaves on masonry.
- **Out over water** beside it, a full 4 px. Bank trees lean out.

Trees are drawn back to front by where the crown is drawn. The collider,
the hit box, burning, ramming and the drone's canopy all stay on the cell.

## 3. The forest floor

A layer baked once onto the floor, like the lava's banks
(`Woods::floor`, drawn in `paint_floor_marks` over the ground's own
shade). Every crown adds a cone of density `REACH` = 34 px across at its
drawn centre, weighted by its kind:

| kind | weight |
|---|---|
| a green tree | 1.0 |
| a date palm | 0.6 |
| a dry tree (`Material::is_dry`) | 0.3 |
| a dead snag | 0.15 |

The density is stepped through the 4 x 4 Bayer matrix on the 2 px block
grid (`pyro::bayer`), the dither every fade in the game uses, into two
tones: `GREEN_SHADE` and `GREEN_DK` on grass, `SAND_DK` and `SAND_MD` on
desert. Where it is dense, one block in about forty is a speck of litter.
**No block on a water cell is shaded**, so a wood's floor stops at the
bank. Felled trees leave their floor: it is ground under a canopy, and
the litter stays.

## 4. Undergrowth

Bush sprites (`grass::bush_tuft`) round each wood, in `Game::grass`
beside the tall grass, so a hull presses them down, a blast flattens them
and a fire chars them like any tuft. They are **picture only**: their
cells are not `Game::grass_cells`, so they hide nobody. Cover is what a
map places.

- **The edge:** each open cell beside a wood grows one with a chance of
  12% plus 6% per tree among its eight neighbours. It is a juniper, a fern
  or a bush where the conifers have it (a hashed field over 4 x 4 cell
  blocks), and a bush, a berry bush or a fern elsewhere. On desert it is a
  bush or a juniper, drawn dry. It sits up to 6 px off the cell's middle,
  centred where a wall stands beside it.
- **Banks:** an open cell beside water and within two cells of a tree
  grows reeds seven times in ten. There are none on desert.
- **Interior:** one tree in five whose eight neighbours are all trees has
  a fern rooted in its own cell, under the crowns.
- **Never** on water, a trail or a cell beside one, or a cell the map put
  anything on: a wall, a pickup, grass, a road, a start.

A burning cell chars every sprite rooted in it (`Game::tick_fires`, and
the replica's `apply`), so the undergrowth goes with the wood.
