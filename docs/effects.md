# The effects language

Every explosion, hit, muzzle flash, fire, smoke plume and damage mark in
bongbong is drawn in one language, so that a drum going up, a shell
striking a wall and a hull burning read as the same fire, and all of it as
part of the same pixel-art field the tanks and walls are. This is that
language: its rules, its colours, the modules that speak it and the knobs
that tune it.

The rules live in code in `src/pyro.rs`; everything else composes from it.

## The rules

1. **Whole 2 px blocks.** Every sprite covers the field in 2 px blocks (a
   tank is a 32 px tile drawn at scale 2; the wall sheet bakes the same
   chunkiness in), so everything that lingers on screen does too. Discs,
   lines and marks lie on the field's even grid (`pyro::block_disc`,
   `block_line`, `mark`), anchored to the field rather than to what is
   drawn, so two puffs dissolve in step and a moving one does not crawl.
   The two shaders that draw effects (`static/flame_jet.fs`,
   `static/plasma_orb.fs`) work their picture out once per 2 px block of
   the field from `gl_FragCoord`, not per pixel of their quad.
2. **Colours are ramp steps.** Fire, smoke, dust and char are steps of the
   Puny palette (below). A colour is a step, never a blend of two; a
   gradient is bands with dithered edges (`step_dithered`). Energy - plasma,
   the laser, the tesla, ooze, the rainbow shield - keeps its own
   deliberately off-palette ramps, as the plasma sheet always has.
3. **Fades dissolve, then fade in eighths.** A thinning puff drops blocks
   through the field's 4x4 Bayer pattern down to half of them; below half
   the pattern holds and the kept blocks fade in eighths
   (`pyro::dither_disc`), because a sparse dither over a wide puff reads as
   a screen door. Smoke mostly breaks up by *shrinking*, thinning only at
   the very end.
4. **Light in steps.** A glow is a few flat bands of light whose edges are
   dithered across the middle third of each step (`pyro::glow`,
   `glow_bands`) - the rule the weather's light pass draws the night with,
   so a blast's bloom by day and a lamp at night are the same light. Every
   glow is drawn inside an additive blend. A fire throws *one* light, over
   where it burns hottest - never one per puff, which pile up into a white
   blot where two shells land together.
5. **Shaded, not outlined.** A puff has a shadow step down and right, its
   body, a lit side up and left (the field is lit from the top left, as the
   shadows fall) and, while it burns, a hotter core (`pyro::Puff`,
   `pyro::shade`). An outline reads as a sticker.
6. **Smoke leans with the grass.** Smoke, embers and dust drift down-wind
   by the wind that bends the tall grass (`pyro::smoke_lean`, from
   `grass::wind_at`), so a column leans and swings with the tufts under it.
7. **Fast things may be bright; lingering things are shaded.** A tracer, a
   spark in flight, the first frames of a flash are brief and bright; a
   fireball, a smoke column, a burning deck are shaded and stepped.
8. **Hashed, never rolled.** Every cosmetic choice hashes from a position,
   a slot or a seed (`pyro::unit`, `blast::seed_at`); nothing draws the
   round's RNG, so a replica, a paused frame and a test draw the same
   picture, and seeded replays are untouched. Composers are pure functions
   of what they draw and its age, and return `pyro::Shape`s; painters only
   paint them.

## The ramps

| Ramp | Steps, dark to bright | From |
|------|-----------------------|------|
| `FIRE` | `#4A2221` `#812F27` `#9C3527` `#E44219` `#DC9C4A` `#EEA343` `#FFE2A0` `#FFFFFF` | the palette's reds and golds, `FIRE_PALE`, white |
| `SMOKE` | `#252525` `#373737` `#5A5A5A` `#7E7E7E` `#9E9E96` `#C1C1C1` `#DADADA` | the palette's greys |
| `DUST` | `#67512A` `#73624D` `#B7A248` `#C9B266` `#D2BA6B` | sand and ash |
| `CHAR` | `#252525` `#373737` `#4A2221` `#59341F` | soot and scorch |
| `PLASMA_TEAL`, `PLASMA_PURPLE` | the orb shader's own five | off-palette energy |
| `LASER_RED`, `LASER_BLUE`, `TESLA`, `SHIELD`, `OOZE` | their weapons' own | off-palette energy |

`FIRE_PALE` (`#FFE2A0`) is the one step the palette gained, so a flame
cools white, pale gold, gold rather than jumping (`tools/punypalette.py`).
Oil burns black (its smoke runs a step darker, `fx::SOOT_T`), a fuel drum
cleaner and paler.

`pyro::dust_of` gives each material its dust as a puff's shadow, body and
lit steps: stone off masonry and the towers' armour, sand off a sandbag,
wood dust off timber, leaves off a tree. Iron sparks, glass glints, a drum
blows up - they throw none.

## Who draws what

| What | Composed in | Notes |
|------|-------------|-------|
| A blast's fireball - a drum, a plain kill, a missile's burst, a cook-off | `fireball.rs` | a flash with rays, shaded puffs burning white to red and cooling into smoke, fragments, dust racing out; the form (round, column, flat, double) is `BlastFx::row` |
| A dying tank's mushroom cloud | `mushroom.rs` | the same puffs and ramps, a stem, a rolling cap, a dust ring |
| Every hit | `burst.rs` | a shell's flash and small fireball thrown back toward the gun; a bullet's spark star; plasma and tesla rings; a laser's molten splash; ooze |
| Every muzzle flash | `burst::muzzle` | a tongue of fire down the shot's line in three steps; a plasma cannon's flash burns in its bolt's colour |
| Dust off a tile a shot hit; a tile coming down | `burst.rs` (`ImpactKind::Dust`, `Collapse`, `Ash`) | in the material's dust; a tile that burnt out falls in as ash |
| A hit's flash | `fx::Flash`, drawn by `render/game.rs` | the hull, tile or tower drawn again over itself in an additive blend, stepping down in three over `hit_flash_seconds` |
| Flames on the ground, a burning tile, a burning tank | `pyro::tongues` | teardrop tongues in three nested layers, flickering through four heights and leaning with the wind |
| A tank's damage | the tank sheet, then `damage_stage.rs` | the sheet's damage tiers and wrecks carry the wear; the code adds smoke from the damaged tier, deck fire from the critical one and a wreck burning down |
| A burnt-out ground fire | `Scorch::burn` | a soft burn scar, not a darkened square |
| Particles | `fx.rs`, drawn by `render/fx.rs` | sparks and embers cool down the fire ramp; smoke, dust and a missile's trail are shaded puffs; chips are small squares |
| The light shots throw | `render/shot_fx.rs` | stepped glows and streaks of blocks |
| The flamethrower's jet, the plasma orb | `static/flame_jet.fs`, `static/plasma_orb.fs` | worked out per 2 px block, coloured in their ramps' flat steps, edges dithered |
| A volcano's cone, smoke and eruption; a lava bomb | `volcano.rs` (docs/volcano.md) | the cone a baked picture of blocks in `ASH`/`SCORIA` with molten gullies; the plume shaded `SMOKE` puffs leaning with the wind; an eruption a flash with rays, a fountain of `pyro::tongues`, drops and a shock ring of marks; a bomb a rock with a glowing trail and a warning ring of marks where it lands |
| Lava | `lava.rs` (docs/volcano.md) | every 2 px block a `FIRE` step - flow bands running down the stream, crust plates on a lake, a toasted bank baked once into a `BlockImage`; a bomb's pool a metaball cooling down the ramp |

A tank's damage, a step per tier of the tank sheet (`TANK_DAMAGE_TIERS`,
docs/SPRITESHEET_SPEC.md). The sheet carries the wear - scuffs, plates
gone, the wrecks - so the code draws only what moves, from one hashed spot
on the engine deck behind the turret (`damage_stage::engine_deck`, one of
`DAMAGE_VARIANTS` per tank):

- **Scuffed** (25): the art alone.
- **Damaged** (50, `SMOKES_AT`): smoke rises off the deck
  (`damage_stage::smoke`, put up by `fx.rs`), a wisp at first and a column
  as the hull gets worse (`hull_smoke_rate`).
- **Critical** (75, `BURNS_AT`), or with afterburn on it: the deck burns
  (`damage_stage::fire`, `flames`) and the smoke turns black. A wreck
  burns hard over its whole hulk and dies down over the last fifth of
  `wreck_burn_seconds`, and its kill lays the grass round it flat.

## Painting

`pyro::draw(b, &shapes)` paints puffs, marks and lines alpha-blended and
leaves glows out; `pyro::draw_glows(b, &shapes, bands)` paints the glows
and belongs inside an additive blend block. `b` is any `pyro::Blocks`:
every `Canvas` (so `game.rs`'s stages and the CPU tests paint the same
shapes) and the raylib handle through `render::pyro::Rl`. The field's
stages draw what stands - a tank's marks and flames, burning tiles - in
their y order; `Game::render`'s glowing pass draws the hits, blasts and
flashes over everything, each list composed once a frame, its glows in one
additive block (a blend switch breaks raylib's batch).

## Knobs

| Knob | What it tunes |
|------|---------------|
| `glow_bands` | steps in every glow; 0 draws smooth gradients, for comparison |
| `smoke_wind_drift` | how far smoke leans in a typical gust |
| `blast_fireball_px`, `blast_fireball_seconds` | a blast's size and how long it burns and smokes |
| `blast_glow_seconds`, `blast_glow_radius`, `blast_glow_strength` | the bloom a blast opens with |
| `barrel_fps_jitter`, `barrel_scale_jitter` | per-blast pace and size variety |
| `ground_fire_tongues`, `ground_fire_height_px`, `ground_fire_spread_px` | flames on a burning cell |
| `*_hit_seconds`, `hit_fx_scale` | how long each hit plays and its size |
| `tile_dust_seconds`, `tile_collapse_seconds` | a tile's dust when hit and when it comes down |
| `hit_flash_seconds` | a hit's flash; 0 turns it off |
| `hull_smoke_rate` | smoke off a damaged hull's deck |
| `muzzle_flash_duration`, `muzzle_glow_radius` | a muzzle flash's life and size |
| `shot_glow_strength` | the light shots throw; 0 draws the plain sprites |

## Adding an effect

Compose it as a pure function returning `Vec<pyro::Shape>` from what it is,
its age and a seed; take its colours from a ramp; give it one glow; let
its smoke lean (`pyro::smoke_lean`); draw it with `pyro::draw` in the pass
it belongs to and its glows in that pass's additive block. Test it
headless: that it is on the grid, in its ramp and gone by its end, and
that the same inputs give the same shapes. The ignored `preview` tests in
`fireball.rs` and `mushroom.rs` write strips to `target/` for a look
without a window.
