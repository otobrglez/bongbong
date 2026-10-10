# Ground memory: tread marks and what a hull throws

A tank leaves the field changed. Its runs press tread marks into the ground in
2 px blocks, the grain everything lingering on the floor lies on
(docs/effects.md), and those marks stay for the round: they take their colour
from what they were pressed into, record how the hull moved (rolling,
pivoting, sliding, a ford it climbed out of) and age in steps until the
weather wears them away. A moving hull also throws up what it drives through:
dust off dry ground, mud in the wet, foam and brown spray in a ford, snow
powder, chips off ice, exhaust as it pulls away.

The research, the proposals and the decisions behind this are on the Tread
Lab page (https://claude.ai/artifact/RKznnB1TsYgwPXEwBRacWa) and in BB-87;
this document is what is built (BB-88). Trampled cover (BB-89) and oil prints
(BB-90) are the gameplay follow-ups and are not built.

## Where it lives

| Module | What it owns |
|--------|--------------|
| `src/wear.rs` | The headless half: `WearGrid`, its blocks and cells, `Stamp`, the rules (`press_factor`, `is_soft`, `erodes`), the colour ramps, the bake and the pictures (`refresh_pictures`). |
| `src/simulation/wear.rs` | The world half: `press_treads` (a hull's step pressed into the grid), `roll_tread`, `Underfoot` (what lies where), `Game::tick_wear` (gusts, silt), `drain_marks` (a gravity well's scrub), `char_marks`, `hold_worn_grass`, `Game::driving`. |
| `src/lib.rs` | `TreadProfile`, `TREAD_BY_ROW`: every chassis's runs as the art draws them. |
| `src/fx.rs` | `Fx::drive`: the driving effects (`ParticleKind::Kick`, `ParticleKind::Foam`, and spray, chips and smoke reused). |
| `src/render/fx.rs` | `draw_ground`: dust, powder and foam drawn with the floor, under the light. |
| `src/game.rs` | `paint_floor_marks` draws the worn cells first, under scorches, craters and rubble; `refresh_pictures` and `with_block_images` bring the pictures up and hand them to the GPU. |

## The grid

`WearGrid` keeps a `Cell` of 16 x 16 blocks for every map cell a hull has
touched (a `BTreeMap`, so walks are in cell order), and nothing for the rest.
A block is four bytes:

- `press`: the passes pressed into it, in sixteenths, saturating at 255. On
  open water the same byte is its silt.
- `bits`: its `Kind` (pad, grouser, smear, churn, drip, splat, char), the way
  the hull faced (lawn stripes), a berm flag and how wet, in thirds.
- `laid`: the round time it was last pressed, in eighths of a second.

A field map keeps at most `wear_max_cells` (8,192) worn cells, about 8 MB; past
that the cell pressed longest ago goes, the first in cell order on a tie, so a
seeded replay evicts the same cells. An arena never reaches it.

## Pressing the ground

Every step a hull moved, `press_treads` stamps its contact patch at the drawn
heading (`Tank::visual_rotation`): the round calls it from
`sync_tanks_and_ram` (off the physics step) and `rollin_phase`, a replica from
`ease_hulls` (off `Tank::track_from`). It reads only poses, the ground and the
round clock, and draws no RNG.

- **The runs.** `TREAD_BY_ROW` lists each chassis's runs, contact patch and
  link period as the shipped art draws them
  (`tools/spritegen/tankdesign/lines/vanguard.py`, its `runs`/`tread` calls),
  in tile px about the pivot. An assault presses two 3-block runs at its
  hull's edges; the titan has an outer and an inner run a side and presses
  four. `Tank::scale` takes tile px to the field.
- **Grousers.** A block is a grouser's bar when its coordinate along the hull's
  heading, in tile px, falls in the first half of the art's link period (4)
  plus the hull's `tread_phase`: the ladder is fixed to the ground, so a
  straight drive prints a steady ladder and the pattern only shifts where the
  hull turned.
- **Passes.** A block pressed again within `wear_pass_gap_seconds` is the same
  pass still rolling over it; otherwise the pass adds `wear_chassis_press`
  (per chassis) times `tread_press` (per hull, rolled at spawn) times the
  ground's factor: a dirt road 0.7, sand 1.3, desert dust 1.1, rain 1.8, snow
  1.3, a pivot 1.5. Depths are `wear_rut_passes` (1.5), `wear_deep_passes`
  (3) and `wear_path_passes` (5).
- **How it moved.** Turning while slower than `wear_pivot_speed_fraction` of
  its top speed is a pivot and churns; moving across its runs (sideways
  travel over `wear_slip_fraction` of its travel above `wear_slide_min_speed`,
  or knocked off its tracks, `Tank::skid`) smears; a start that spins on ice
  or under rain churns too. Otherwise it rolls.
- **Berms.** On soft ground (sand, desert dust, anything under snow, grass in
  the rain) the block just outside each outer run is pushed up, unless it is
  already pressed, and crumbles over `wear_berm_seconds`. It shows in lumps
  (the bake's clump noise), never as a ruled line: turned turf on grass,
  the light, loose top of sand and dust.
- **Water.** Open water takes no mark. A hull whose centre is in a ford has
  wet tracks; it prints wet for `wear_wet_carry_px` of travel once out (the
  runs still on the bank while it wades print dry), drops a drip between its
  runs every `wear_drip_spacing_px` (by a hash of its odometer) and stirs the
  ford's silt (`wear_silt_stir`) in a disc the hull's width, thinning to
  its rim. Ice takes only scratches.
- **Mud.** Under rain on grass or dirt, or with tracks still wet from a ford, a
  hull throws a mud splat behind it every `wear_splat_spacing_px` at a place
  hashed from its odometer; rain washes a splat off over `wear_splat_seconds`.
- **Wrecks and blasts.** `char_marks` burns every pressed block within
  `wear_char_radius_px` of a wreck or a drum's blast in: it never thins.
- **Wells.** A pulling gravity well scrubs the marks in its reach
  (`drain_marks`): their grousers swirl into a smear and they fade at
  `wear_well_scrub` times the pull's strength.
- **Gusts.** A sandstorm's gust (`weather::gust_at`, a pure function of the
  round clock) scours sand, desert dust, its roads and anything under snow at
  `wear_gust_scour`; grousers go first.
- **Silt.** Twenty steps a second, a share (`wear_silt_drift`) of each
  stream block's silt drifts a block south where the current runs
  (`WaterLayout::pushes_south`); a lake keeps it where it was stirred; all of
  it settles over `wear_silt_seconds`.
- **Worn grass.** Tall grass on a lane past `wear_deep_passes` never quite
  stands back up (`wear_grass_crush_floor`). Cover is the cells', not the
  tufts', so this is cosmetic.

`roll_tread` takes the four RNG draws every spawn takes: the phase and the
press are the hull's, the other two set nothing and hold every seed's stream
where the pinned replays expect it.

## How a block looks

The bake works a block's colour out from its age and the round's `Look` (the
map's theme and `Sky::of` the round's weather):

- **Fresh to settled.** Grousers show until `wear_fresh_seconds` and dissolve
  into the pad through the field's 4x4 Bayer pattern by `wear_settle_seconds`
  (sand four times as fast, a dirt road half as long again, desert dust and
  rain a little faster).
- **A single pass** thins after `wear_settle_seconds` to half its blocks by
  `wear_fade_seconds` and holds there for the round, the kept blocks fading in
  eighths below half (effects rule 3). Ruts never thin.
- **Depth.** A rut's top-left inner wall is a step darker (the field is lit
  from the top left); a pass on grass lies lighter driven up or left and
  darker driven down or right; past `wear_path_passes` a grass lane takes the
  tileset's own path colours.
- **Rain** darkens the marks as the weather shader darkens the ground and,
  after `wear_puddle_fill_seconds`, pools water along the ruts in lumps.
- **Snow** turns the marks to compacted blue-grey with the ground showing in
  deep ruts and white berms, and falling snow fills them back in at
  `wear_snow_refill_per_second` passes a second.
- **Ice scratches** last `wear_ice_scratch_seconds`. **Silt** shows in
  clumps as a muddy teal, brown where it is thick.

### Colours

Every ramp is steps from the Puny palette (`tools/punypalette.py`) and the
retinted ground tileset's own pixels, never a blend; the tables are `GRASS`,
`ROAD`, `SAND`, `DESERT_DUST`, `DESERT_ROAD` and `DESERT_SAND` in `wear.rs`,
with the snow ramp built over each ground's dark step. Burnt-in marks take
`pyro::CHAR`.

## The pictures

The marks are drawn as a `TileAtlas` of baked cells, the lava layer's pattern
(`lava::LavaPictures`): `Game::refresh_pictures` (from `app.rs`, before the
frame's textures upload) drops the tiles of cells no longer worn or out of
view, then bakes the cells in view that have no tile, that a stamp changed or
whose last bake is `wear_rebake_seconds` old, at most `wear_bakes_per_frame`
a frame (every one after a jump in the clock). The atlas holds at most 2,048
cells, which keeps it inside the 4,096 px a WebGL texture is sure of, and is
sized in powers of two of rows (`TileAtlas::reserve`), so it resizes a handful
of times a round. Each bake is its own patch, and `BlockTexture::sync`
uploads patches lying far apart one by one rather than as their bounding
rectangle. A room server never refreshes pictures; a fresh round has none, so
`mapshot` and the pinned thumbnail hashes are unchanged.

## Driving effects

`Game::driving()` reports every live hull's tread (`Tank::tread`, set by
`press_treads`): its surface, roll, speed, velocity, whether it is speeding
up, how wet. `Fx::drive` throws, from the rear of the outer runs:

| Surface | Throws | When | Leaves |
|---------|--------|------|--------|
| Dirt road, sand | Dust puffs (`Kick`), thicker on sand | Dry ground, dry tracks | - |
| Desert dust | A pale plume, larger and longer-lived | Dry; a gust carries it | - |
| Grass | A little dust; torn turf through a pivot | Dry | - |
| Mud | Clods off the top of the runs, both ways through a pivot | Rain, or wet tracks | The simulation's splats |
| A filled rut | Muddy water from the front of the runs | Rain, a rut older than half `wear_puddle_fill_seconds` | - |
| Ford | Foam (`Foam`) off the bow and sides; brown spray where the silt is thick | Wading | Silt drifting downstream |
| Snow | Powder (`Kick`) | Snowy sky | - |
| Ice | White chips | Sliding or spinning | Scratches |
| Any | Exhaust (`Smoke`) off the engine deck | Pulling away | - |

Dust grows as speed to `drive_dust_speed_power` (1.3; unpaved-road dust rises
faster than linearly with speed) times the chassis's press, and nearly doubles
through a pivot or a slide. The knobs are the `drive_*` rows of the `fx`
group; the wading spray keeps `water_spray_rate`.

`Kick` and `Foam` are drawn with the floor in the lit field
(`render::fx::draw_ground`), under the hulls and the light pass, so the night
darkens them and a headlight shows them; everything else draws where it
always has. The particle layer is `app.rs`'s and may use `rand::rng()`;
whatever stays on the ground is the simulation's, chosen by hash.

## Online

Nothing crosses the wire. A replica presses its own marks from the hulls the
snapshots place (`Tank::track_from`, reset across a jump so no mark is laid
across a portal or a placement) and throws its own effects from them, so its
marks can differ from the room's by an interpolated pixel.

## Tests

- `wear::tests`: the runs under an assault and a titan, grousers fixed to the
  ground, passes and the gap, water and ice, slide and pivot, berms, the cap,
  burning in, gusts on sand and road, silt in a stream and a lake, the bake's
  ages, snow refill, rain in a rut, the pictures following the view.
- `simulation::wear::tests`: the turn.
- `a_ford_slows_a_hull_and_takes_no_tread_marks`, `ice_slides_where_the_ground_would_stop_a_hull`,
  `a_barrel_blast_throws_parts_queues_pops_and_rearranges_the_ground`,
  `tread_marks_are_scrubbed_away`, and the replica's
  `a_hull_that_jumps_lays_no_tread_marks_across_the_gap` and
  `a_welcome_puts_on_no_show`.
