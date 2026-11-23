# Field life: crows, ducks and fireflies

The lakes have fish (`fish.rs`, docs/water.md "Presentation"). The field
has three more creatures built the same way (BB-218): crows on open grass,
mallards on a lake's open water and warm fireflies at night. They are
scenery that reacts to the fight - a hull coming near, a shot flying past,
a blast - and nothing else.

## The rules they keep

- **Presentation only.** Nothing in the simulation reads a creature,
  nothing travels on the wire, and no creature draws the round's RNG or
  `rand::rng()`. Every choice is hashed (`pyro::unit`) from the round's
  seed, a cell or the creature's own seed.
- **Owned by `app.rs`.** `fauna::Fauna` lives in `fx::Fx`, beside the
  fish's `Shoal`, and `Fx::observe_events` steps it after every simulation
  step on the round's clock (`Game::time`), in ticks of at most
  `PHYSICS_FIXED_DT`. A frame already seen does nothing; a new round (a
  new seed, a frame counter going back, `Event::RoundStarted`) rebuilds it.
  The same steps from the same seed give the same field, so a dev-server
  lockstep replays it, and a replica - which reads the same `Game` - draws
  the same creatures.
- **What stirs them** is one `fauna::Stir` a step: every live hull; every
  shell, bullet, plasma bolt, missile and orb in the air; every hit,
  ricochet, laser beam and rail slug (a point every cell along it); every
  blast, missile or grenade burst, wreck, cook-off, air strike, drone
  burst, well collapse and rod impact; and every tree a hull struck
  (`Event::HullStruck`), a shot hit or that fell.
- **The effects language** (docs/effects.md): whole 2 px blocks on the
  field's grid, Puny Palette steps (tools/punypalette.py). A creature is a
  small grid of blocks drawn facing up, one character a block
  (`fauna::compose_grid`), turned by sampling the grid at each block's
  centre turned back into it - the fish's way, so a turned shape keeps
  whole blocks and no gaps - in sixteen headings less the four diagonals,
  where a three-block-wide grid breaks into a checkerboard
  (`drawn_heading`). A flier's shadow is the same grid in black at eighths,
  along the field's light (`shadow_dir_x`/`_y`) by half its height.
- **Where they draw** (`render/fauna.rs`): what sits or swims on the floor
  straight after the fish, under everything that stands; what flies, and
  its shadow, in the air stages (`paint_air_shadows`, `paint_air`), so a
  flier passes over the tanks and the trees; the fireflies over the lit
  field (`paint_field_glowing`), so the night does not darken them. Each
  layer is culled to the camera's view. None of it is in `paint_field`, so
  mapshot's thumbnails and their pinned hashes, and a headless round's
  screenshot, show none.

## Crows

A flock of five to eight (`crow_per_flock`) on a stretch of open grass: a
cell inside the field with nothing the map put on it and nothing grown on
it (undergrowth included), whose floor is grass at all four quarters, and
whose eight neighbours are the same. One flock per `crow_flock_cells` open
cells, at most `crow_max_flocks`, each at least `crow_spacing_cells` from
the next, picked in a hashed order - so an arena holds two or three and a
big field map a dozen. The birds stand in two loose rows of four about
the flock's spot. `crow_roost_share` of the trees hold one to three more
in the crown.

Colours: the body STONE_DARKEST, wings and head BLACK, the beak STONE_DK.

```
sitting   pecking   wings out   wings in
 .k.       ...       ..k..       ..k..
 .H.       .k.       ..H..       ..H..
 BWB       .H.       WWBWW       .WBW.
 BWB       BWB       W.B.W       .WBW.
 .W.       BWB       .W.W.       .W.W.
```

At rest a crow pecks in about a third of its hashed windows, and every
`crow_hop_seconds` or so turns to a hashed heading or hops a few pixels
about its place. A hull within `crow_flush_px`, a shot or a hit within
`crow_shot_px` or a blast within `crow_blast_px` of any bird flushes the
whole flock: it picks the stretch it settles on next, `crow_land_min_px` to
`crow_land_max_px` from the cause (the farthest stretch when none is), and
each bird turns away and lifts off within `crow_stagger_seconds`. It flies
straight away for `crow_scatter_seconds`, swings round the new spot for
`crow_circle_seconds` (the flock all one way), then comes in, climbing to
`crow_altitude_px` and coming down over the last 48 px, at
`crow_fly_speed`, turning at most `crow_turn_deg` a second and beating its
wings at `crow_flap_hz` - spread as it glides in low. A crow in a crown
flushes when its tree is struck within `crow_tree_px` of the trunk, or a
shot or a blast comes near; a hull going by does not move it. It settles
on the grass. A perched crow is drawn in the air stage, over the crown,
and swings with it (`TreeShake::lean`).

## Ducks

A group of three to five mallards (`duck_per_group`) per
`duck_group_cells` of a lake's open water - its deep cells, the ones the
fish swim in (`fish::lakes`) - on a lake of at least
`duck_min_lake_cells`. 6 x 12 px. Colours: the body WOOD_ASH, the back
STONE_MD, the head TEAL_DK, the bill GOLD_BRIGHT.

```
swim    dabble    wings out   wings in
.k.      ...       ..k..       ..k..
.H.      ...       ..H..       ..H..
BBB      BHB       WWBWW       .WBW.
BWB      BWB       W.B.W       .WBW.
BWB      BWB       ..B..       ..B..
.B.      .B.
```

A duck paddles at `duck_speed` from near one deep cell's centre to near a
neighbour's (the fish's legs), its group making for a spot of the lake that
changes every `duck_drift_seconds`, with a pale V of BLUE_PALE blocks
falling behind it, every other step in turn and fainter as it spreads
(`duck_wake_opacity`); it dabbles in about one hashed window in seven. A
hull within `duck_shy_px` sends it paddling off at `duck_paddle_speed`,
wary for `duck_calm_seconds`. A shot or a hit within `duck_shot_px`, or a
blast within `duck_blast_px`, puts the whole group up: it picks open water
at least `duck_land_min_px` from the cause (a deep cell whose eight
neighbours are deep, in this lake or another), and each duck, within
`duck_stagger_seconds`, runs along the surface for `duck_run_seconds` on the
heading nearest straight away that keeps the run over water, throwing
WHITE and BLUE_PALE spray behind it, then flies at `duck_fly_speed` and
`duck_altitude_px` with its shadow on the water, and lands with a little
spray. Nothing on ice: a frozen round has no ducks, and water that freezes
under them hides and stops them.

## Fireflies

At night - full under `Weather::Night` or once a map's nightfall has come
(`Game::nightfall_mix`), `firefly_dusk` at dusk, none under any other sky
(`fireflies::shown`). They live over the glades: open grass, water and soft
cover within two cells of a tree, the map's tall grass or bushes, or the
undergrowth round a wood - one in a cell at `firefly_density`, at most
`firefly_max`, the hashed first. Each drifts `firefly_drift_px` about its
spot on two slow hashed sines and glows in and out at about
`firefly_pulse_hz`, its brightness in quarters: a FIRE_PALE core while it is
bright, GOLD_BRIGHT while it is dim, in a GOLD_BRIGHT glow of
`firefly_glow_px` at `firefly_opacity`, the glow falling off as the 1.5th
power of the distance, stepped to eighths and broken up by the Bayer
pattern. A blast within `firefly_blast_px` pushes the near ones away (up to
`firefly_push_px`) and puts them out; they drift back over
`firefly_return_seconds` and light again over `firefly_relight_seconds`.

The glow is drawn as blocks over the lit field, not as a light in the
weather's light map: the light map's pool would light the grass round it
as well, and the look chosen (BB-218's preview) is the dithered glow
alone.

## Knobs

The `fauna` tuning group: `crow_*`, `duck_*` and `firefly_*`, each creature
with an on/off (`crow_enabled`, `duck_enabled`, `firefly_enabled`); the
counts and densities apply from the next round.

## Tests

`crows::tests`, `ducks::tests`, `fireflies::tests` and `fauna::tests`:
placement only on legal cells (open grass, open water, glades), the same
seed placing and moving the same creatures, a blast or a shot flushing a
flock and a group that land again on grass and open water, a hull flushing
only within reach, a struck tree flushing its crows, none on ice, fireflies
only in the dark, every block on the grid in its palette steps, and a
shipped lake map (`maps/lotus-lagoon.toml`) holding all three.
`cargo test --lib fauna::tests::preview -- --ignored` writes
`target/fauna-preview.png`: every crow and duck grid in every heading.
