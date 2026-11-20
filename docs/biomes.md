# Biomes — what kind of place a battlefield is

Every map used to stand on the same bright field with the same cyan water.
A **biome** makes a map look like a place that formed before the battle:
water leaves gravel and damp ground, roads wear their shoulders, the meadow
is not one even green. Tanks, bongs, pickups and shots keep their arcade
look and stay the strongest contrast on screen.

A biome is **a look and nothing else**. No collider, footing, nav cost,
cover, objective, AI snapshot or RNG draw reads it, so a map plays the same
with or without one (`biome.rs`, `ground::GroundGrid::materials`).

The work is BB-191 and its stages BB-192 to BB-198. It came from a
designers' handoff (`target/bongbong-biome-handoff.zip`, researched on
6b20287); this document is the part of it that was accepted, with what the
check against the code changed.

## The three biomes

| Biome | Look | First map |
|---|---|---|
| **Valley** (Soča Gravel Valley) | clear alpine water, gravel by the banks, earth on the road's shoulders, a meadow in broad patches, willow and birch on the banks | Serpent River |
| Floodplain (Reclaimed Floodplain) | slate still water, reed belts, peat banks, an old floodwork apron | Lotus Lagoon |
| Karst (Dry Karst Frontier) | warm hardpan, flat limestone plates and fissures, dry scrub, dark pools | No Man's Land |

Only the valley exists so far. The other two are rows in the same table in
stage 6 (BB-198).

## How a map gets one

- **Map file**: a top-level `biome = "valley"` (`map::MapFile::biome`).
  Absent draws the plain tiles and is not written back, so every older map
  re-saves byte-identical and keeps its revision. An unknown name is a
  parse error, not a fallback.
- **Over every local round**: the `biome_override` knob (cosmetics group,
  `@ Restart`): `-1` follows the map, `0` draws none, `1` the valley. It is
  a row in the PR preview's tuning panel, which is how a biome is compared
  on a map that does not name one yet (`biome::in_force`).
- **Separate from `theme`.** A theme picks the tileset retint and the grass
  sheet, and also decides a rule: a desert's trees are dry. A biome changes
  no rule, so it never rides on the theme.

No shipped map names a biome yet. All three looks go live together once
the third is accepted (decided 2026-11-20); until then each is opt-in.

## What a biome draws

### Materials (stage 1, BB-193)

A layer of 2 px blocks baked once over the floor tiles (`biome::bake`),
drawn by `ground::draw` right after the tiles. So the floor shade, the
forest floor, every mark, and the weather's snow and puddles fall on top of
it, and everything that draws the floor - a round, the weather's ground
pass, the builder's canvas, a thumbnail - draws it. It is baked the first
time the floor is drawn, so a room server never pays for it, and baked
again whole after a builder edit changes a tile.

What goes where, in this order:

1. **Bank**: the tileset's shore outline - olive today (`#5B6641`,
   `#4E5A39`, `#67744D`), against the de-green rule - is repainted in damp
   silt, darker right beside the water.
2. **Silt seam**: ground within 4 px of open water or a ford.
3. **Gravel shelves**: along the water, coming and going in a noise about
   a cell and a half across; up to 30 px wide where a shelf is broad,
   fraying into the meadow over its last 8 px.
4. **Road shoulders**: packed earth up to 10 px out from a road's dirt.
5. **Meadow patches**: two neighbouring greens laid over the grass at a
   low opacity in patches five to ten cells across, so the grass tiles'
   own specks show through.

Water and a road's dirt are never painted: they are the tiles'.

Placement reads **the tiles' own shapes**, never their colours.
`tools/ground_mask.py` writes `src/ground_mask.rs`, a mask per road and
water tile that says, for each of its 16 x 16 source pixels, whether it is
water, bank, road dirt or plain ground; `GroundGrid::floor_px` answers for
any field pixel through the tile drawn there. A painted water cell is not
all water - a lake's shore runs half a cell inside its outline - so a cell
test would put gravel in the river. The rain shader still tells water by
colour (`static/weather_ground.fs`); moving it onto the mask is stage 4
(BB-196).

Everything is hashed from position with no seed: a map draws the same in
every round, in the builder and in its thumbnail. The noise is plain
arithmetic, the same on every platform.

### Water (stage 2, BB-194)

Each biome carries a water ramp (`biome::WaterRamp`), one step for each
role the tileset's water colours play, and its own tone for open water no
hull enters, so a ford and a lake read apart for the first time (today
they share one colour).

- **Its own tileset.** `tools/retint_ground.py` writes
  `punyworld-overworld-tileset-valley.png`: the grass retint, then the
  water in the ramp, the olive shore outline in silt, and tile 305 - the
  one tile a deep cell draws, `WATER_SHORE[0b1111]` - in the open-water
  tone. The field draws it as `Sheet::BiomeGround` in place of the theme's
  tileset (`GroundGrid::sheet`); the window loads one per biome up front.
- **Open water and the ford.** `FloorPx::Deep` is tile 305's water, the
  rules' deep cells. The materials soften it into a body rather than the
  cells' squares (`biome::deepness`): every deep cell adds a cone 26 px
  across its centre, full to 16 px, so a run of cells is solid with a
  rounded edge that fades within about 10 px of the deep cells, and a lone
  deep cell - Serpent River has one - is a round pool. The ford shows a
  pebble of its bed (`WaterRamp::bed`) on about one block in 23 away from
  the open water.
- **What follows it.** The current marks take the ripple tone, the spray
  the ripple and water tones (`fx::water_tints`), the minimap its light
  and open-water tones (`minimap::Class::color_in`). Foam stays white. The
  rain and ice shaders still tell water by colour, and every valley water
  tone passes their test; moving them onto the tile mask is stage 4.

### Decks (stage 2, BB-194)

A road cell between two stretches of water - on Serpent River, (43, 18)
and (44, 18) - is dry road to the rules, so today the river stops at one
side and starts again on the other. With a biome the picture bridges it
(`GroundGrid::bridge`): every such map road cell that no wall stands on is
a `Deck`, the tiles round it are resolved as if it were water so the river
runs on under it, a road keeps the rules' own reading so it still joins the
deck, and the materials lay planks over it - a band 24 px across the road,
railed on both edges, planks running across the water. A deck's water has
no current. Nothing that plays sees any of it: `WaterLayout`, the nav
grid, footing, tread marks and the minimap read the map's own layout, and
the probe plays the same round either way. Real bridges with their own
rules are BB-62.

### Later stages

Planting and the built places (BB-195), weather on the new ground and
Serpent River's skies (BB-196), the whole map with its margins, minimap,
thumbnail and tread marks (BB-197), the other two biomes and the builder's
LOOK row (BB-198). The floor past an arena's field (`GroundGrid::beyond`)
carries no materials yet: a block layer starts at the field's corner.

## The valley's palette

Chosen on the palette card (BB-192, https://claude.ai/artifact/XJxa8DqExN4U8jBEYZwYWe),
a field rendered by the game's own painter with every sprite left as it is.

| Role | Steps |
|---|---|
| Water: edge, dark, fill, light, ripple, glint | `#145F70` `#1B7584` `#258E9A` `#38A5A9` `#6CC7C0` `#B4E6DE` |
| Open water: fill, dark | `#165A6A` `#114B59` |
| A pebble on a ford's bed | `#5E9C92` |
| Deck: rail, seam, planks | `#3A2A1C` `#4E3A28` `#8A6A48` `#7A5C3D` |
| Gravel: pale, middle, dark | `#E2D4B0` `#C9B993` `#A99A78` |
| Silt: away from the water, beside it | `#6E5A44` `#554536` |
| Road shoulder | `#B09868` `#977F55` |
| Meadow patches | `GREEN_DARKEST` `#1C4C33` at 12%, leaf green `#7FB15A` at 15% |

The water roles map from today's tileset colours: edge ← `#0E818C`,
`#4A7A72`; dark ← `#0A90A0`; fill ← `#04A0B4`; light ← `#15AEB9`,
`#23C0BF`; ripple ← `#1DCCCB`, `#13DBDA`; glint ← `#81E5EF`.

## Rules every biome keeps

- **Readable on it.** Every material stands at least ΔE 20 (CIELAB) from
  every seat colour, the health and ammo crates and both shell colours;
  today's road and the ammo crate are 30 apart
  (`biome::tests::every_material_stands_apart_from_every_seat_pickup_and_shell`).
  The closest valley pair is the pale gravel and seat 3's silver, 24.9.
  Open water stands ΔE 10 from a ford, and both the ford and open water
  stand ΔE 20 from every seat colour, the teal hull and glass - but seat 8,
  whose deep teal is ΔE 5 from the valley's ford (12 from today's cyan).
  Seat 8's colour is to change (BB-202); until then the test holds it as
  its one exception (`SEAT_ON_ITS_WATER`).
- **De-greened.** Large areas stay deep green or earth tan, never olive or
  yellow-khaki; a meadow patch keeps green its strongest channel
  (`a_meadow_patch_stays_green`, docs/PALETTE.md).
- **Off by default, exact when off.** A map with no biome draws the pixels
  it always did, so the pinned CPU renders of the shipped maps
  (`thumbnail.rs`) hold until a look is turned on deliberately.
- **Flat.** Nothing a biome paints may read as a ledge, a wall or a step:
  anything that looks like cover has to be cover (an authored solid with
  the same footprint) or it is a gameplay change and its own issue.

## Adding a biome

One `Biome` variant with its name, one `BiomeLook` constant, and a row in
`biome_override`'s range. The tests walk `Biome::ALL`, so the new look is
held to the same contrast and de-green rules at once.

## The look-book

`maps/study/look-book.toml` holds every contact once on a 26 x 13 field -
a ford broken by a road, a lake with open water, a wood, bank trees, a run
of every wall material, tall grass, crates and the bong - with `biome =
"valley"`. A sandbox, not a level: the fixture for screenshots and the
palette card.

```sh
cargo run --bin mapshot -- -o target/look-book.png maps/study/look-book.toml
cargo run --bin mapshot -- -o target/serpent-river.png maps/serpent-river.toml   # plain
```

A round under the valley on any map: `--tuning` a patch with
`{"biome_override": 1}`, or the tuning panel's row on a PR preview.
