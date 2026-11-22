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
| **Floodplain** (Reclaimed Floodplain) | slate still water with a lighter submerged margin, sedge on the wet banks, peat at the shore, an old floodwork apron of paving round the towers by the water | Lotus Lagoon |
| **Karst** (Dry Karst Frontier) | on the desert's dust: warm hardpan, flat limestone plates with hairline fissures, dark pools with a damp seam, dry scrub in the shelter of walls | No Man's Land |

All three are rows of one table (`Biome`, `BiomeLook`); the floodplain and
the karst came in stage 6 (BB-198).

## How a map gets one

- **Map file**: a top-level `biome = "valley"` (`map::MapFile::biome`).
  Absent draws the plain tiles and is not written back, so every older map
  re-saves byte-identical and keeps its revision. An unknown name is a
  parse error, not a fallback.
- **The builder**: the MAP panel's **LOOK** row, under THEME in the field
  group: a stepper through none and the biomes made for the map's theme,
  each press one undo step, the canvas redrawn in it at once. The dev
  server's `builder_settings {biome}` is the same edit.
- **Over every local round**: the `biome_override` knob (cosmetics group,
  `@ Restart`): `-1` follows the map, `0` draws none, `1` the valley, `2`
  the floodplain, `3` the karst. It is a row in the PR preview's tuning
  panel, which is how a biome is compared on a map that does not name one
  (`biome::in_force`); it outranks the map, so LOOK shows `(cli)` under it.
- **Separate from `theme`, but made for one.** A theme picks the tileset
  retint and the grass sheet, and also decides a rule: a desert's trees are
  dry. A biome changes no rule, so it never rides on the theme. But its
  tileset is one theme's retint (`Biome::theme`: the valley and the
  floodplain grass, the karst the desert), so on the other theme its
  ground would not match the trees and the tall grass. The LOOK row offers
  only the biomes of the map's theme, and a change of THEME takes a look
  made for the other one off in the same undo step; the knob does not ask.

No shipped map names a biome yet. All three looks go live together once
the third is accepted (decided 2026-11-20); until then each is opt-in.

**The key travels as text.** A room is handed its map as TOML
(`wire::map_toml`) or by a shipped map's name, so `biome` reaches every
seat with no change to the wire and no `PROTOCOL_VERSION` bump. It is part
of a map's revision (`MapFile::revision`), so naming a biome is an edit
like any other: a builder map's clear stamp no longer holds, and a shipped
map's own revision moves with it.

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
3. **The shore band** (`BiomeLook::shore`): along the water, coming and
   going in a noise about a cell and a half across. **Gravel** shelves up
   to 30 px wide where a shelf is broad, fraying into the meadow over
   their last 8 px; or a **sedge** bed (the floodplain): upright stems a
   block wide and three tall - lit tip, stem, dark foot - three columns in
   five, each column's stems lifted by its own hash so the tips never line
   up, the grass between. A stem is whole or absent: the bed's extent is
   read at its tip.
4. **Road shoulders**: packed earth up to 10 px out from a road's dirt.
5. **Limestone plates** (`Plates`, the karst): pale patches of flat stone
   in a noise about two cells across, at least 16 px from the water and
   8 px from a track, their rim crisp so a plate reads as a flat face, not
   a fade, with hairline fissures across them - a contour of a finer
   noise, a block wide, broken every few blocks, a step darker than the
   stone. Nothing in them is strong enough to read as a ledge.
6. **Open-ground patches**: two neighbouring tones laid over the floor at
   a low opacity in patches five to ten cells across, so the tiles' own
   specks show through - a meadow's two greens on grass, warm hardpan and
   pale dust on the karst's dust.

Water and a road's dirt are never painted: they are the tiles'.

Placement reads **the tiles' own shapes**, never their colours.
`tools/ground_mask.py` writes `src/ground_mask.rs`, a mask per road and
water tile that says, for each of its 16 x 16 source pixels, whether it is
water, bank, road dirt or plain ground; `GroundGrid::floor_px` answers for
any field pixel through the tile drawn there. A painted water cell is not
all water - a lake's shore runs half a cell inside its outline - so a cell
test would put gravel in the river. The weather's ground pass reads the
same shapes (stage 4, below).

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
  rain and ice read water off the tiles' shapes (stage 4, below).

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

### Planting and places (stage 3, BB-195)

- **The map.** Serpent River's woods stand at least five cells from its
  water, so it has no bank trees to swap. Instead the broadleaf trees on
  the river-facing edges of the two nearest stands turned to willow, with
  birch a step behind: 6 willows and 5 birches of its 52 broadleaves,
  edited by hand in `maps/serpent-river.toml`. They are softer than a
  broadleaf (a willow takes 10 to fell, a birch 8 and goes over quicker
  under a ram) and hide a hull just the same. This one change is
  gameplay, and it lands with or without a biome.
- **Reeds on open banks.** Under a biome, an open bank cell with no tree
  within two cells grows reeds too, in clumps (`Woods::sheltered_reeds`):
  a hashed field over 3 x 3 cell blocks, never beside a road. They are
  undergrowth, picture only, like the reeds on a wooded bank.
- **Places, found by rule** (`biome::places`, `Places`): a timber building
  within three cells of water gets a **mill race**, a stone-lined channel
  6 px across from its nearest wall to that water with weir foam where it
  meets the river. Tall grass within four cells of a fence is a **field**,
  ploughed in furrows every 6 px that run along the fence. Nothing is
  authored, so every map with a biome gets them; `GroundGrid::set_places`
  holds them, and the round and the builder set them from the map.
  Only a look that works the land dresses them (`BiomeLook::farms`: the
  valley).
- **Masonry and the post fence** (BB-203). A biome that dresses its
  built tiles (`Biome::dresses_tiles`; the valley does) draws every brick
  wall as pale dressed limestone and every fence as one slender run of wire
  between square post caps, through every state: whole, struck, breached,
  given way, and the rubble and pieces it leaves. The row is picked at
  draw time (`obstacle::TileLook`, read off the ground by
  `Game::tile_look` and the builder's canvas); the material, its health,
  its chunks and the rubble decal the simulation lays are brick's and the
  fence's, so a round plays the same plain or dressed
  (`props_tests::a_dressed_wall_and_fence_play_and_break_exactly_as_plain_ones`).
  The art is rows 33-41 of the walls sheet (docs/WALLS_SPEC.md §14).

### Weather (stage 4, BB-196)

- **Water by the tiles' shapes.** The ground pass reads water and a road's
  dirt off the shape of the tile under each pixel (the cell mask's blue
  names it, `ground::shape_atlas` holds the shapes), not off its colour:
  rain rings land only on real water, puddles only on a road's dirt, and
  ice covers exactly the water drawn - a deck's planks stay dry, and a
  lake corner rounded into a grass cell freezes with the rest (BB-200,
  which this fixes on every map). The shader-free fallback lays ice on the
  water by the same shapes instead of snow.
- **Rain by material.** With a biome each material takes the rain its own
  way: pale gravel darkens least, earth - damp silt, a packed shoulder, a
  road's dirt - most; puddles are fewer, smaller and lighter.
- **Snow over the materials.** Gravel, silt and shoulders take snow like
  the grass round them, from the same noise. A dark seam at the bank was
  considered and left out: the ice's own pale blue and the shore outline
  already mark where it begins, and a dark line round frozen water would
  read as open water on ice a hull can drive. What shows through the snow
  in patches is the ground under it, here as on any map.
- **Serpent River's skies**: `weather = ["clear", "dusk", "rain", "fog",
  "snow"]`, the round's seed picking one. The rules come with them: rain
  halves grip, fog shortens the enemies' sight, snow freezes the river so
  the whole of it is open ground. This is level design, live whatever the
  biome switch says.

### Everything that shows the map (stage 5, BB-197)

- **The whole field.** The materials were always baked over the whole
  field, so all of Serpent River takes the look at once: the western
  meadow approach stays quiet - patches only - the wooded pockets sit in
  meadow, and every reach of the river has its banks. Nothing in this stage
  is placed by hand.
- **The margins.** The floor carried past an arena (`GroundGrid::beyond`)
  carries the materials on (`biome::bake_beyond`, keyed by the world block
  so they meet the field's without a seam) and thins them away through the
  Bayer pattern by `ground::BEYOND_MATERIAL_CELLS` (6) past the edge - the
  margins of a tablet or an ultrawide round a standard arena; beyond that
  the margin shade's plateau hides the ground. The shade still deepens off
  the playfield, so the edge of play reads as before. `render::margin`
  uploads the layer itself. A followed field map has no margins.
- **The minimap** (`Minimap::texel`): open ground beside water where the
  bank's gravel shelf is broad takes the shelf's middle stone, and the
  meadow takes its patches from the same noise as the field
  (`BiomeLook::minimap_ground`), so the small picture has the field's large
  shapes. Fords and open water were the biome's already. The builder's
  navigator and the Load list's thumbnails are the same minimap.
- **The thumbnail.** The CPU thumbnail always drew the materials; the GPU
  one (`mapshot --renderer gpu`) now uploads them too.
- **Tread marks** (docs/ground-memory.md "Colours"): a mark takes the
  colours of the material under its block (`GroundGrid::floor_material`,
  `Ramp::in_biome`) - pressed meadow deepening into earth, not olive; dark
  tracks crushed into a gravel shelf; dark mud on a bank's silt; dry dust on
  packed earth and the road - and a hull's wet carry out of a ford prints
  the silt's mud into the meadow.

Measured on 2026-11-22: a 1280x720 window holds frame p99 within noise with
the biome on (`tools/perf/scene.py`, 12 s a scene: Serpent River 7.06 off,
7.03 on; Longwater 7.33, 7.30). The CPU canvas pays for blending the layer
every frame (`just perf-paint`, 60 frames: Serpent River 13.7 to 15.8 ms,
Longwater 49.9 to 57.0 ms); the window draws it as one texture.

### The floodplain and the karst (stage 6, BB-198)

- **Their tilesets.** `tools/retint_ground.py` writes
  `...-floodplain.png` (the grass retint, slate water, the shore outline in
  peat, its sandy specks in sedge) and `...-karst.png` - the **desert**
  retint (`base`), dark pools, and the desert's grey-brown shore outline
  as a damp seam drying to grit. The other three sheets come out
  byte-identical.
- **The floodwork apron** (`Places::aprons`, `Apron`): round every tower
  that stands within two cells of water, two cells each way, and joined
  across the way in between where two such towers stand within eight
  cells on one row or column - on Lotus Lagoon the two tesla towers at
  (21, 13) and (27, 13) flanking the peninsula's mouth. Slabs 16 x 12 px in
  a running bond on the field's grid, a few lifted in the middle and most
  at the rim so it frays into the meadow, stained by the water within
  12 px of it and in damp patches. Only open ground is paved: water, bank
  and road keep their tiles, and tall grass and the towers stand on it as
  before. Found by rule, like a mill race; only a look with an `Apron` lays
  it.
- **Dry scrub** (`BiomeLook::scrub`, `Woods::scrub`): under the karst, an
  open cell beside a wall, with no tree, water or road round it, grows a
  bush or a juniper - drawn dry, as the desert draws them - in clumps, a
  hashed field over 3 x 3 cell blocks, about a third of them. Undergrowth:
  picture only, it hides nobody.
- **Sea water** on Archipelago and Harbor Lights, from the pack's three
  unused sea sets, waits on the Puny World licence question (BB-109): it
  uses more of the pack (BB-204).


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
`#23C0BF`; ripple ← `#1DCCCB`, `#13DBDA`; glint ← `#81E5EF`. The other two
biomes map the same roles.

## The floodplain's and the karst's palettes

Picked by measurement (BB-198) against the rules below, on renders of
Lotus Lagoon and No Man's Land by the game's own painter: the designers'
concepts B and C as the direction, every tone moved until it cleared ΔE 20
from every seat, crate and shell and the de-green rule.

| Role | Floodplain | Karst |
|---|---|---|
| Water: edge, dark, fill, light, ripple, glint | `#26404F` `#3A5D6E` `#4A7383` `#6A93A0` `#9AB9BE` `#CFE0DF` | `#0F3F3E` `#16524F` `#1F6A64` `#2C7D74` `#5FA59A` `#A8D6CB` |
| Open water: fill, dark | `#33536A` `#2A465A` | `#154A49` `#103D3C` |
| On a ford's bed | weed `#5A7B6C` | `#3E7A6A` |
| The shore band | sedge: tip `#C9B27C`, stem `#A8925F`, foot `#7F6C46` | grit `#E8D7B0` `#CDBB93` `#A8956E` |
| Damp ground at the bank | peat `#4C3C2E` `#3A2E24` | damp seam `#957A57` `#7A6245` |
| Road shoulder | trodden mud `#8E7759` `#735F47` | worn dust `#B99D72` `#A68B62` |
| Open-ground patches | `#1C4C33` at 18%, `#7FB15A` at 12% | hardpan `#A07D52` at 35%, pale dust `#E2CFA4` at 31% |
| Deck: rail, seam, planks | wet timber `#2E241B` `#3E3125` `#7A6046` `#6A533D` | the valley's |
| Apron: slabs, seam, wet | `#A39E8E` `#959080` `#868274` `#737064` | - |
| Plates: faces, fissure | - | `#DED0AE` `#D6C6A2` `#A88F69` |

The closest pairs: the floodplain's pale slab and seat 3's silver, 25.9;
the karst's pale plate and the same silver, 24.1. The floodplain's ford
stands 31 from glass and 14 from its open water; the karst's 33 from the
teal hull and 14 from its open water.

## Rules every biome keeps

- **Readable on it.** Every material stands at least ΔE 20 (CIELAB) from
  every seat colour, the health and ammo crates and both shell colours;
  today's road and the ammo crate are 30 apart
  (`biome::tests::every_material_stands_apart_from_every_seat_pickup_and_shell`).
  The closest valley pair is the pale gravel and seat 3's silver, 24.9.
  Open water stands ΔE 10 from a ford, and both the ford and open water
  stand ΔE 20 from every seat colour, the teal hull and glass. Seat 8 is
  violet `#6B3FD8` for this: its old deep teal was ΔE 5 from the valley's
  ford (BB-202).
- **De-greened.** Large areas stay deep green or earth tan, never olive or
  yellow-khaki: on grass a patch keeps green its strongest channel, on the
  desert's dust it stays a warm earth, red over green by a clear step
  (`a_meadow_patch_stays_green_and_a_patch_of_dust_stays_earth`,
  docs/PALETTE.md). The floodplain's straw sedge is stems on the grass,
  never an area of it.
- **Off by default, exact when off.** A map with no biome draws the pixels
  it always did, so the pinned CPU renders of the shipped maps
  (`thumbnail.rs`) hold until a look is turned on deliberately.
- **Flat.** Nothing a biome paints may read as a ledge, a wall or a step:
  anything that looks like cover has to be cover (an authored solid with
  the same footprint) or it is a gameplay change and its own issue.

## Adding a biome

One `Biome` variant with its name, the theme it is made for and its
tileset (a theme in `tools/retint_ground.py` with that theme as `base`),
one `BiomeLook` constant, a `biome-<name>` word in every `lang/*.ftl`, and
`biome_override`'s range. The tests walk `Biome::ALL`, so the new look is
held to the same contrast and de-green rules at once, its sheet has to
exist and its name has to fit the LOOK row.

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

A round under a biome on any map: `--tuning` a patch with
`{"biome_override": 1}` (2 the floodplain, 3 the karst), or the tuning
panel's row on a PR preview. A still of one: a copy of the map with
`biome = "karst"` added under `version = 1`, through `mapshot`.
