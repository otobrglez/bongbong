# The aftermath: what a fight leaves burning

The floor of a round remembers the fight - scorches, rubble, charred tread
marks, craters (docs/ground-memory.md, docs/effects.md). This is the rest of
the memory: the heat. Wrecks and ruins smoulder for minutes after their fire
is out, burnt ground cools through embers, a burnt tree leaves a smouldering
stump, and their smoke gathers into a haze over the field that thickens as
the round goes on. The sky works on all of it: rain shortens every smoulder
and turns its smoke to steam, snow buries what has gone cold while a hot
hulk melts a ring round itself, and a sandstorm's gusts tear the smoke away.
At night everything still hot keeps throwing light.

The proposal, its live demo and the ten ideas it was cut into are the
Aftermath Lab page (https://claude.ai/artifact/SjbY71ERdPFh6xFNYFY6u5); this
document is what is built (BB-120). Nine of the ten are cosmetic. Three
rules play, each behind a knob that defaults off (read once by `init`, so a
round keeps the rules it began under) and off byte-identical: smoke that
hides, firelight that reveals, a gust relighting an ember bed.

## Where it lives

| Module | What it owns |
|--------|--------------|
| `src/smoulder.rs` | The headless half: `Kind`, `Stage`, `spans`, `stage_at`, `douse`, `Hot` (`load`, `flare`, `ember`), `step_haze`, `Rules`, and the composers - `column`, `glints`, `flare`, `haze_banks`, `haze_grade`, `ash`, `melt`, `smoke_cells`, `gust_strong`, `snow_cover`, `sunlit`. |
| `src/simulation/smoulder.rs` | The world half: `Game::wreck_hot`, `Game::smouldering` (every hot remain, gathered), `tick_haze`, `tick_smoke_cover`, `relight_beds`, `wreck_scar`. |
| `src/damage_stage.rs` | A hulk's `smoulder` (glowing breaches, flare-ups) and `weathering` (rust, snow on a cold deck). |
| `src/decal.rs` | `Decal::hot` (charred rubble smoulders), `Decal::stump` and its upright drawing. |
| `src/blast.rs` | `Scorch::ember`: the scar a ground fire left is an ember bed. |
| `src/game.rs` | `snow_keep` (cold marks fading into the snow), `paint_aftermath_floor` (melt rings, glints, flare-ups), `HulkLook` on `draw_one_tank`, `aftermath_air` / `AftermathAir` (the frame's columns, their shadows, the banks, ash and grade). |
| `src/render/game.rs` | Where the air goes: the shadows over the floor, the smoke over everything that stands, the glows in the additive pass. |
| `src/weather.rs` | `lights_in` (each hot remain's light and the haze's underlight), `is_lit` (`fire_reveals`). |
| `src/fx.rs` | Sparks off a flare-up. |
| `src/net/` | `Snapshot::smoke_cells` (protocol 29) and the replica's stump and hulk scar. |

## The remains

`Game::smouldering` gathers every remain still giving off heat from what the
round and a replica both keep, so both gather the same list and nothing new
goes on the wire:

| Remain | `Kind` | From | Its age |
|--------|--------|------|---------|
| A wreck (every one, cold too) | `Wreck` | `Tank::is_wreck` | `Tank::wreck_age`, uncapped (a replica counts its own) |
| An ember bed | `Bed` | `Scorch::ember` - `fire_burnt_out`'s scar, unless lava | `Scorch::age` |
| Charred rubble | `Ruin` | `Decal::hot` - a tile fire finished | `Decal::age` |
| A burnt tree's stump | `Stump` | `Decal::upright` (`tile_stump`, the tree sheet's `TREE_STUMP_COL`) | `Decal::age` |
| A tower's ruin | `Tower` | `Game::tower_ruins` | `TowerRuin::age` |
| A rod's crater | `Crater` | `Game::craters` | round time since it was made |

Each cools through the same stages (`stage_at`), on spans its kind and the
sky set (`spans`):

| Kind | Burning | Smouldering | Wisps |
|------|---------|-------------|-------|
| Wreck | `wreck_burn_seconds` (4) | `wreck_smoulder_seconds` (90) | `wreck_wisp_seconds` (150) |
| Bed | - | `ember_bed_seconds` (70) | - |
| Ruin | - | `ruin_smoulder_seconds` (120) | half that |
| Stump | - | 0.6 of it | 0.35 of it |
| Tower | `tower_ruin_smoke_seconds` (8) | `ruin_smoulder_seconds` | half that |
| Crater | `rod_crater_smoke_seconds` (6) | `crater_smoulder_seconds` (40) | - |

Every smoulder and wisp is multiplied by `douse(sky)`: `rain_douse_factor`
(0.35) in rain and a storm, 0.8 under snow, 1 otherwise. A remain's heat is
1 as it starts smouldering, 0.3 as it turns to wisps and 0 cold. What burns
is drawn by what already drew it - a wreck's flames (`damage_stage::fire`),
the particle layer's wreck smoke, a tower ruin's and a crater's own smoke -
so the aftermath's pictures start where those end.

## The pictures

All pure functions of a remain and the round clock, hashed from its seed,
drawing no RNG (docs/effects.md):

- **Columns** (`column`): written like the volcano's plume - a puff may be
  born every twelfth of a second and is kept by a hash against its stage's
  rate at its birth (`smoulder_column_rate` at the thickest), so a column is
  one picture for one clock on every client and uses none of the
  `fx_max_particles` budget. Dark smoke paling as the remain cools, thin
  pale wisps, pale fast steam under rain. They lean with the grass's wind
  (`pyro::smoke_lean`) and a sandstorm's gust throws them along
  (`gust_tears_smoke`). The hottest `smoulder_max_columns` (16) in view are
  drawn.
- **Shadows**: each column puff shades the floor down and to the right by
  its height (`smoke_shadow_offset`), one step on half the blocks
  (`smoke_shadow_strength`), while the sun is out (`sunlit`: clear, dusk,
  heat haze, sandstorm). The haze's banks cast none: discs of shade the
  size of a bank read as holes in the ground.
- **Embers** (`glints`): a hashed share of a bed's, a heap's, a stump's or a
  ruin's blocks (`ember_bed_glint`, 0.18, at its hottest) breathing through
  the fire ramp's four darkest steps, fewer and dimmer as it cools, with one
  glow. A hulk's are
  hashed breaches over its hull (`damage_stage::smoulder`).
- **Flare-ups** (`Hot::flare`): in a hashed half of some three-second
  stretches of a smoulder (`smoulder_flare_chance`, never in the rain), small
  tongues of flame and their light, and sparks from the particle layer
  (`spark_rate`).
- **Weathering** (`damage_stage::weathering`): rust blooms over a hulk from
  `wreck_rust_after_seconds` after its kill to `wreck_rust_max`; under snow a
  cold deck goes white.
- **Snow** (`Game::snow_keep`): under a snowy sky every cold mark - scorches,
  rubble, stumps, beds once they have cooled - fades into the snow the
  weather's ground pass lays under it, in eighths over `snow_cover_seconds`
  and never past 0.85, so a buried scorch still shows through in its own
  shape (`blast::draw_scorch_kept`, `decal::draw_decal_kept`); a hot wreck
  melts a wet ring round itself (`melt`, `snow_melt_px`).
- **Hulk scars** (`wreck_scar`): when a wave round takes a wreck's body off
  the field after `wave_wreck_despawn_seconds`, `wreck_scar_parts`
  blackened parts stay where it stood beside its scorch. A replica lays the
  same off `Event::WreckRemoved`.

## The battle's haze

The one thing with memory. Every hot remain feeds a load (`Hot::load`: a
burning wreck 1, a smouldering one up to 0.6, wisps 0.14, a cold hulk
`battle_haze_hulk_load`) and the sky drains it:

    haze += (battle_haze_gain * load * (1 - haze) - haze / drain) * dt

with `drain` `battle_haze_seconds` (600) under a clear sky,
`battle_haze_rain_seconds` (30) in the rain and 0.37 of the clear one in a
sandstorm. On the demo round of the Lab page (nine kills in six minutes, a
burnt shed, a drum's fire) it climbs to about 20% by the second minute and
55% by the sixth; six kills in half a minute take it past 40% within the
minute. Rain keeps it under 10%. `Game::battle_haze` keeps it, ticked with the effects (`tick_haze`) on
the round and on a replica alike; nothing a round plays reads it, so it can
never change a replay, and a client that joins late grows its own.

It is drawn as:

- **Banks** (`haze_banks`): a bank per cell of a lattice anchored to the
  field and drifting with the wind, so a field of any size carries the same
  density and a bank never jumps. The lattice moves at the wind's mean
  (`pyro::mean_smoke_lean`, from `grass::mean_wind`), never the wind of the
  moment: an offset of `time` times a speed that gusts would swing the
  banks back and forth, wider the longer the round. A bank is a loose cluster of small
  dithered discs strung out along the wind, densest in its middle, so it
  has no hard edge: one flat step of the smoke ramp at half strength, its
  blocks fading in eighths, a paler step under a night or storm sky so the
  dark makes it a veil rather than a blot. Each disc thins to nothing
  within `battle_haze_clear_px` of a live seat - a hull is never lost under
  the haze - and none covers more than `battle_haze_max`.
- **The grade** (`haze_grade`): one translucent warm grey over the field in
  the lit pass, at most `battle_haze_grade` - the field dims and warms a
  little; the glowing pass (shots, fire, flashes) and the chrome draw over
  it, ungraded.
- **Ash** (`ash`): above `ash_from_haze`, up to `ash_density` flakes per
  10,000 px^2 in the smoke ramp's palest steps, falling and drifting on a
  lattice anchored to the field, at the wind's mean like the banks.
- **Light at night** (`weather::lights_in`): each hot remain a dim light
  breathing with its embers and leaping as it flares
  (`smoulder_light_strength`, `ember_light_px`), the hottest
  `SMOULDER_LIGHTS` (24); under a haze a broad warm light over each
  (`haze_underlight`), so the smoke over a fire is lit from below.

## The rules that play

Each read once by `init` into `Game::aftermath_rules` (`smoulder::Rules`),
all off by default; off reads nothing and changes nothing.

- **Smoke that hides** (`smoke_cover`): every wreck, ruin and tower ruin
  still burning, or smouldering at `smoke_cover_strength` or more, fills its
  own cell and `smoke_cover_cells` more downwind (`grass_gust_heading_deg`)
  with smoke that hides a hull as tall grass does: `tick_smoke_cover` puts
  them in `Game::smoke_cells` each tick before the terrain is built, and
  `cover_cells` joins them to the grass. A shot or a hit alert gives a tank
  away as it does in grass. The arrows read cover on the client, so the
  cells travel in `Snapshot::smoke_cells` (protocol 29) and a joiner gets
  them in its welcome. The probe's ceilings are recorded with the rule off;
  turning it on is a re-baseline.
- **Firelight reveals** (`fire_reveals`): under a sky that shortens sight a
  tank within `fire_reveal_px` of something burning - a ground fire, a
  burning tile, a wreck still on fire - is seen at the full
  `enemy_view_range`, as in a lamp's light (`Game::is_lit`). AI sight is
  the room's, so nothing goes on the wire.
- **Gusts relight** (`ember_bed_relight`): every
  `ember_bed_relight_seconds`, each ember bed still smouldering that a
  strong gust is over (`gust_strong`: the grass's wind at three quarters of
  a gust or more, or half a sandstorm gust) throws an ember downwind
  (`Game::throw_ember`, the burning tiles' own), which lights the wood, tree
  or board it lands on `ember_catch_chance` of the time. Fires are already
  the room's and on the wire.

## Online

Ideas 1 to 9 add nothing to the wire. A replica has its wrecks (and counts
their `wreck_age` from the frame it first sees one), its fires (a fire
leaving the snapshot is `fire_burnt_out`, whose scar is the bed), its
`ObstacleDestroyed` events (charred rubble from the burning tile still
standing, and a stump from its variant), its tower ruins and craters, and
`Event::WreckRemoved` (the hulk scar). A player who joins late sees older
wrecks start their stages from the join, as their fire already did; scorches
and decals from before a join do not reach a joiner (`apply.rs` only removes
tiles), and their embers inherit that gap.

## Knobs

The `aftermath` tuning group, all live but the three rules (`@ Restart`):
`wreck_smoulder_seconds`, `wreck_wisp_seconds`, `ember_bed_seconds`,
`ember_bed_glint`, `ruin_smoulder_seconds`, `crater_smoulder_seconds`,
`smoulder_flare_chance`, `smoulder_column_rate`, `smoulder_max_columns`,
`rain_douse_factor`, `battle_haze` (on), `battle_haze_gain`,
`battle_haze_seconds`, `battle_haze_rain_seconds`, `battle_haze_max`,
`battle_haze_grade`, `battle_haze_clear_px`, `battle_haze_hulk_load`,
`ash_density`, `ash_from_haze`, `spark_rate`, `smoke_shadow_strength`,
`smoke_shadow_offset`, `smoulder_light_strength`, `ember_light_px`,
`haze_underlight`, `snow_cover_seconds`, `snow_melt_px`, `gust_tears_smoke`,
`wreck_rust_after_seconds`, `wreck_rust_max`, `wreck_scar_parts`;
`ember_bed_relight`, `ember_bed_relight_seconds`, `fire_reveals`,
`fire_reveal_px`, `smoke_cover`, `smoke_cover_strength`, `smoke_cover_cells`.

## Tests

- `smoulder::tests`: the stages and their order, rain's douse and steam, a
  column only off a smouldering remain and the same for the same clock, its
  shadow down and right and none in the rain, a bed's glints on the grid and
  dimming, the haze building under a fight and draining in the rain and once
  the fires are out, the banks' discs keeping out of a seat's clear ring and
  never over `battle_haze_max`, ash only under a heavy haze, smoke cells downwind of a
  hot wreck and none of a bed or a cold hulk, the melt ring.
- `damage_stage::tests`: a hulk's breaches glow until it is cold; it rusts
  in the same places and takes the snow; a live hull does not weather.
- `decal::tests`: a burnt tree's stump, upright and hot; charred rubble hot,
  broken rubble not.
- `mechanics_tests`: a wreck smoulders after its fire and feeds the haze; a
  ground fire cools through an ember bed; each of the three rules does
  nothing off and what it says on; a removed wave wreck leaves its scar.
- `props_tests`: a burnt tree leaves its litter and its stump, a tree shot
  down its litter alone.
- `net::apply::tests::thick_smoke_reaches_the_replicas_cover`: the room's
  smoke cells on the replica and in a joiner's welcome.
