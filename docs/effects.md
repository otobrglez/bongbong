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
   blot where two shells land together - and burning ground cells that
   touch are one fire with one glow by day, as strong as `fire_glow_strength` (at night each cell keeps its own light, which add up)
   (`blast::fire_groups`). A source's light is one colour at noon and at
   midnight: `pyro::LIGHT` is the table the daylight's glow pools and the
   night's light map both read (fire, a fuse, a shell, a bullet, a missile,
   a muzzle, an impact, each laser).
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
| `sonic::STONE` | `#9E9E96` `#C1C1C1` `#DADADA` `#F0F0F0` | the palette's light greys: sound has no colour of its own |
| `EMP` | `#1D6071` `#038AAB` `#27D8C5` `#93ECE2` `#FFFFFF` | the palette's own blues and white (the tank kit's deep water step, `BLUE_DK`, `BLUE_BRIGHT`, `BLUE_PALE`) |
| `VOID` | `#2C1A5C` `#6B3CC9` `#9A5CF0` `#C98CFF` `#F3DCFF` | the gravity well's violets (docs/gravity-well.md): off-palette energy, the plasma purple's family a step deeper at the dark end, so the ring round a core reads as the rim of a hole; the core itself is `SMOKE[0]`, drawn after every light |
| `RAIL` | `#04A0B4` `#1EB3AE` `#27D8C5` `#93ECE2` `#FFFFFF` | the palette's blues a step greener and paler than the EMP's (`BLUE_MD`, `BLUE_LT`, `BLUE_BRIGHT`, `BLUE_PALE`) and white; its glows in `RAIL_LIGHT` (`#4C7EFF`), off the palette, since teal light over the grass reads green |

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
| A hit's flash | `fx::Flash`; a hull's and a bong's in `Game::paint_standing_hit`, a tile's in `render/game.rs` | a hull or a bong covered by its own silhouette in white (`Canvas::solid`, `hit_flash_alpha`) straight after it is drawn in the y-sorted walk - as bright on a dark chassis as on a pale one, under the grass in front of it and under every crown; a tile, tree or tower drawn again over itself in an additive blend; each stepping down in three over `hit_flash_seconds` |
| A shot landing on a bong | `Fx::bong_hit` | its flash, flecks of its own body (`BongKind::body`, `bong_hit_flecks`) and a puff of dust - never the sparks and smoke of armour |
| What bends the picture - a blast's ripple, a muzzle's puff, an impact's punch | `render/shockwave.rs`, `static/shockwave.fs` | one pass the scene leaves its target through: every bend summed and the scene sampled once, moved by whole 2 px blocks, a block at a time, through the source rectangle the camera's shake moves; none under reduced motion |
| Flames on the ground, a burning tile, a burning tank | `pyro::tongues` | teardrop tongues in three nested layers, flickering through four heights and leaning with the wind |
| A tank's damage | the tank sheet, then `damage_stage.rs` | the sheet's damage tiers and wrecks carry the wear; the code adds smoke from the damaged tier, deck fire from the critical one and a wreck burning down |
| A burnt-out ground fire | `Scorch::burn` | a soft burn scar, not a darkened square; an ember bed while it cools (`smoulder::glints`) |
| What a fight leaves smouldering - wrecks, beds, charred rubble, stumps, ruins, craters | `smoulder.rs` (docs/aftermath.md) | columns of shaded `SMOKE` puffs born on fixed ticks and hashed, as the volcano's plume is, paling to wisps and to steam in the rain, their shadows down-right on the floor; ember glints breathing through the fire ramp's low steps; flare-ups as `pyro::tongues`; a hulk's breaches, rust and snow (`damage_stage::smoulder`, `weathering`) |
| The battle's haze | `smoulder.rs` (`haze_banks`, `haze_grade`, `ash`) | banks of small dithered discs in one smoke step, clustered on a lattice drifting with the wind and thinned round every seat; one translucent warm grade over the lit field; ash in the palest smoke steps |
| Particles | `fx.rs`, drawn by `render/fx.rs` | sparks and embers cool down the fire ramp; smoke, dust and a missile's trail are shaded puffs; chips are small squares |
| The light shots throw | `render/shot_fx.rs` | stepped glows and streaks of blocks |
| The flamethrower's jet, the plasma orb | `static/flame_jet.fs`, `static/plasma_orb.fs` | worked out per 2 px block, coloured in their ramps' flat steps, edges dithered |
| Between rounds | `render/dissolve.rs`, `static/dissolve.fs` (`Session::verdict_lift`, `keeps_picture`, `dissolve`) | the end screen's words lift out over its last `round_verdict_lift_seconds` while its dim stays; the screen it ended on, kept, drops away block by block (`round_dissolve_block_pt` UI points a side) through the 4x4 Bayer pattern (`pyro::bayer`'s matrix) over the next round's first `round_dissolve_seconds`, under the dim the mission banner shares - never a blend, never black. A round started mid-play or from the builder, and reduced motion, cut |
| A volcano's cone, smoke and eruption; a lava bomb | `volcano.rs` (docs/volcano.md) | the cone a baked picture of blocks in `ASH`/`SCORIA` with molten gullies; the plume shaded `SMOKE` puffs leaning with the wind; an eruption a flash with rays, a fountain of `pyro::tongues`, drops and a shock ring of marks; a bomb a rock with a glowing trail and a warning ring of marks where it lands |
| Fish under the water | `fish.rs` (docs/water.md) | a silhouette rasterised onto blocks in sixteen headings, `BLUE_DEEP` at `fish_opacity` in eighths, a fainter tail fin swinging a block either way, a `BLUE_PALE` glint |
| A sonic hammer's wave, a tell, a stunned bong | `sonic.rs`, `bong::stun_marks` (docs/sonic-hammer.md) | the wave `sonic_wave_rings` arcs of blocks (`pyro::block_arc`, `Shape::Arc`) in `STONE`, each split at the walls' faces so the shadow shows, unlit in the glowing pass; dust thrown up off every cell the front crosses in the lit pass (`wave_dust`); a tell three arcs contracting into the dish; a stunned bong three blocks circling its head. A sonic hit draws the hull's flash and dust, never fire (`HitCause::Sonic`) |
| An EMP's ring, a disabled hull's sparks, the crackle, the posts going out | `emp.rs` (docs/emp-burst.md) | the ring three whole circles of blocks (`Shape::Arc`) in `EMP` - the front two blocks thick - with a hashed crackle of zigzags (`Shape::Line`) along it and a glow over the pivot that is gone in 0.3 s, in the glowing pass, throwing no light; a disabled hull's sparks zigzags off its box in bursts twice a second and a third-strength glow, thinning over its last second, their light (`weather.rs`) on the same beat; an enemy's crackle zigzags off its coil growing with the wind-up; a post going out a little burst of blocks off its lantern. A dud's landing is particles (`fx.rs`), a dead missile its body with no flame or light |
| A gauss rail's charge, slug and charge end | `gauss.rs` (docs/gauss-rail.md) | the charge a ring of blocks closing round the hull as it fills (`Shape::Arc`, dissolving in through the Bayer pattern, beating at full, white and jumping overcharged), motes drawn in to the bore and a growing glow there; the slug's white frame three blocks wide for `gauss_flash_seconds`, then an ionised trail a block wide wobbling and dissolving with a sparse row of ions beside it, sparks out of the far side of each pierce and a star off the stop, all glowing; in the lit pass a tile's chips in its dust; a fizzle a few blocks falling off the bore, a vent shaded steam puffs leaning with the wind |
| An FPV drone, its shadow, the halo; its burst | `fpv.rs` (docs/fpv-swarm.md) | a 4 x 4-block quad - its frame the two diagonals in `SMOKE[0]`, the body `SMOKE[2]`/`[3]`, a rotor blade a corner stepping between two blocks, the lamp a block blinking in the launcher's colour in the glowing pass - in 4 px blocks up at cruise height, 2 px low and in the halo; a falling one alternates an X and a +; its shadow a square of black at `fpv_shadow_opacity`; the burst the missile's fireball smaller (`fpv_blast_fx_scale`), no scorch in a crown. Wash, buzz, a fall's smoke and a crash are particles (`fx.rs`) |
| A rod's reticle, designator and call; its impact and crater | `rod.rs` (docs/rod-from-god.md) | the reticle eight arcs of blocks turning round its cell (`Shape::Arc`), settling in at rest, a cross at its centre (an X on the caller's own cell), and a dotted line of blocks from the uplink's lens, in the caller's team colour or the designator's red (`LASER_RED`), unlit; a call the same arcs at the circle closing and beating over its last second, a beam a block wide to the view's top (two with a white core at the end), a glow at its foot and the seconds left in block digits (`pyro::digits`), all in the glowing pass; the impact a column six blocks wide of white for three frames, then in the lit pass two dust rings racing out squashed to the ground's tilt, shaded puffs in the ground's dust (water's blues, snow's whites, lava's smoke) and debris blocks in arcs, with the stock fireball, scorch, thrown rubble, screen flash and ripple; the crater blocks of char and earth with a ragged rim on the floor, water rising in it under rain, ice under snow, smoke puffs off it for `rod_crater_smoke_seconds` |
| A gravity well's orb, snap, ring and collapse; its swallows | `well.rs` (docs/gravity-well.md) | the orb a disc of `VOID[1]` round a `VOID[3]` one with a glint block, two motes circling it and a trail of four blocks dissolving behind, swelling off the muzzle, one glow; the snap a ring of `VOID[4]` closing on the anchor with a white block, and the ripple pinched inward (`Shockwave::inward`); the well its accretion ring two arcs turning clockwise, a swirl of blocks spiralling in squashed to the ground's tilt, a dotted rim at its reach and one glow, a bright `VOID[4]` rim round the core and the core itself `SMOKE[0]` drawn after every light, beating in its last second - all in the glowing pass, its light at night violet; held drums the flying drum's sprite circling lifted over their shadows; the collapse a white implosion, two rings racing out dissolving, a glow gone in a third of a second, the outward ripple and screen flash, and in the lit pass ten dust puffs thrown out; a swallow a ring of `VOID[3]` closing on the point. Particles in a pull spiral into the core (`fx.rs`) |
| Lava | `lava.rs` (docs/volcano.md) | every 2 px block a `FIRE` step - flow bands running down the stream, crust plates on a lake, a toasted bank baked once into a `BlockImage`; a bomb's pool a metaball cooling down the ramp |
| An oil print | `obstacle::oil_print_marks` (docs/ground-memory.md "Oil prints") | two runs of blocks across the cell along the hull's way, the puddle's `SMOKE[0]` with a `SMOKE[1]` grouser link every fourth block and one `EMP[1]` sheen block a run, the outer columns ragged; a fainter print dissolves through the Bayer pattern to half, then fades in eighths |
| Tread marks | `wear.rs` (docs/ground-memory.md) | baked a map cell a tile: whole 2 px blocks under each chassis's runs, a ground's ramp a step per depth (a single pass, a rut, a deep rut, a beaten path), a rut's top-left wall a step darker, grousers dissolving into the pad through the Bayer pattern and a single pass thinning to half its blocks, then in eighths; rain's water and snow's compaction are steps of their own, a burnt-in mark `CHAR` |
| What a moving hull throws | `fx.rs` (`Fx::drive`, docs/ground-memory.md) | dust and snow powder shaded puffs on the ground (`Kick`, its shadow and lit steps the dust's, the desert's or the snow's), swelling and leaning with the wind, dissolving then thinning in eighths, in the lit pass; a ford's foam the same, one colour; mud, rut water and silty spray small squares arcing back; exhaust a smoke puff |

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
  `wreck_burn_seconds`, and its kill lays the grass round it flat. Then it
  smoulders - breaches glowing, flaring now and then, a column off it -
  gives off wisps and goes cold, and rusts (docs/aftermath.md).

## Painting

A translucent block is as bright on the glass as it is drawn: every
finished target goes onto the next, and onto the window, opaque
(`render::view::draw_opaque`), so an alpha is the share of the colour
under it that shows through and nothing more - under a clear sky and
under a weather pass alike.

`pyro::draw(b, &shapes)` paints puffs, marks and lines alpha-blended and
leaves glows out; `pyro::draw_glows(b, &shapes, bands)` paints the glows
and belongs inside an additive blend block. `b` is any `pyro::Blocks`:
every `Canvas` (so `game.rs`'s stages and the CPU tests paint the same
shapes) and the raylib handle through `render::pyro::Rl`. The field's
stages draw what stands - a tank's marks and flames, burning tiles, the
flames of each burning ground cell at their foot (`game::ground_fire`,
`GROUND_FIRE_FOOT`), a hit's silhouette - in their y order;
`Game::render`'s glowing pass draws the hits, blasts and flashes over
everything, each list composed once a frame, its glows in one additive
block (a blend switch breaks raylib's batch).

What only reflects light is drawn in the lit field, which a weathered
frame multiplies by its light map and fogs; what gives light is drawn in
the glowing pass after it. The particle layer is split on that line
(`render/fx.rs`): dust and foam with the floor (`draw_ground`), smoke, a
missile's trail, a grenade's plume, dust, chips and spray at the end of the
lit field (`draw_air`) - so a wreck's smoke is dark at night and grey in
fog - and sparks and embers in the glowing pass (`draw_light`). A
particle's ramp is a field of its tint (`fx::Tint`, `fx::Ramp`), set where
it is thrown; the drawing never tells fire from a colour by comparing
them.

Everything fades in steps: an alpha through `pyro::alpha` (eighths), a
strength that is not yet a colour through `pyro::eighths` - a scorch and a
decal stepping in, the whole-screen flash stepping out. A sprite laid on
the ground is drawn at whole texels (`blast::texel_scale`): a scorch at
1.2 pixels a texel would show some texels two pixels wide and some one. A
laser is one red (or blue): its beam, its sparks and its light are steps
of `pyro::LASER_RED` (`LaserVariant::ramp`, `light`).

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
| `hit_flash_seconds`, `hit_flash_alpha` | a hit's flash and how opaque a hull's or a bong's white silhouette starts; 0 seconds turns it off |
| `fire_glow_strength` | how strongly a fire of burning ground warms the ground around it by day (its one glow, `blast::fire_groups`) |
| `bong_hit_flecks` | flecks of its body a hit bong throws |
| `reduced_flash_scale` | what reduced motion keeps of a whole-screen flash |
| `hull_smoke_rate` | smoke off a damaged hull's deck |
| the `aftermath` group | how long what a fight leaves smoulders, its columns, embers, flare-ups and rust, and the battle's haze (docs/aftermath.md) |
| `muzzle_flash_duration`, `muzzle_glow_radius` | a muzzle flash's life and size |
| `muzzle_flash_speed`/`width`/`strength`, `impact_flash_*`, `shockwave_*` | each bend of the picture: how fast its front runs, how thick it is and how hard it pushes, in standard field heights |
| `shot_glow_strength` | the light shots throw; 0 draws the plain sprites |
| `wear_*` | how the ground keeps tread marks: passes, ages, rain, snow, gusts (docs/ground-memory.md) |
| `drive_*` | what a moving hull throws: dust, mud, puddle water, powder, ice chips, foam, exhaust |

| the `shock` group | how hard each cause shakes and ripples the screen, on one scale (below) |
| `ram_fx_*`, `ram_shock` | a slam's floor, full strength, rise, gap, burst and shake |
| `hurt_*`, `hurt_shock` | being hit: the damage that shows and that is full strength, the edge pulse's length and depth, the health tick's length, the shake |
| `kill_hitstop_seconds`, `kill_hitstop_gap_seconds` | the hold on a seat's own kill, and how soon another may hold |

## One scale for every blast

Every cause's ripple and shake is a row of the `shock` tuning group, read
as a multiple of a tank dying's (`kill_shock`, 1). A blast sits where its
reach puts it - its reach over 140 px, so a missile and a drone (44 px) are
both 0.3, a bomb (64 px) 0.45, a grenade and a fuel drum (near 125 px) 0.9
- and a cause with no blast radius (a muzzle, a shield, a slam, a blow
taken) is placed by ear against those. The simulation's `*_show` halves
read the rows, so a replica shakes as its room does.

The particles follow the same number: `Fx::explosion(at, size, kind)` is
the one burst every blast event throws, `size` the cause's shock row and
`kind` (`fx::Explosion`) only its colours - a charge, oil, fuel, lava, a
hull, a gravity well's dark, a bong's dull thud. Sparks, smoke, their
speed and their spread all come from `size` (`explosion_counts`), so two
causes of one size throw alike and a bigger one throws more.

## Being hit, ramming, a kill

`feedback.rs` (headless, owned by `app.rs` like `fx.rs`) reads the round
and never writes it, so all of it is the same on a replica and none of it
is on the wire:

- **A blow taken.** A local seat's hull losing `hurt_min_damage` or more
  in a step, whatever did it, is a blow of strength damage over
  `hurt_full_damage`: a shake of that seat's view alone (`hurt_shock`,
  never under two fifths of it - the shake lands on whole 2 px blocks),
  an **edge pulse** and a **tick** on its health number. The pulse is a
  band of 2 px blocks round the view, `hurt_pulse_blocks` deep at full
  strength, in three steps down the fire ramp, dissolving inward on the
  Bayer matrix and then fading in eighths over `hurt_pulse_seconds`. The
  tick lights the number for `hurt_tick_seconds` and drops it 2 px, then 1.
  On the couch's split each half takes only its own seat's
  (`feedback::ViewOf`); one shared view takes every local seat's.
- **A slam.** A hull whose hardest contact rises `ram_fx_rise`-fold to
  `ram_fx_min_impulse` or more - the blow that opens a contact, not the
  push after it - throws a burst at the contact point, flashes and shakes
  (`ram_shock`, a `fx::Jolt`: a shake with no ripple), all scaled by the
  impulse up to `ram_fx_full_impulse` (`fx::slam_strength`). A ram that
  hurts (`Event::Ram`) is a slam whatever its impulse and flashes both
  hulls. The steady scrape of a hull pushing on is one emitter a hull,
  keyed by owner slot.
- **A kill.** A local seat's own kill holds the round for
  `kill_hitstop_seconds` (`Feedback::time_scale`, which `app.rs` multiplies
  into the step clock's dt beside `Session::time_scale`), no sooner than
  `kill_hitstop_gap_seconds` after the last. Offline only: a room's round
  is the server's to pace, a lockstepped dev round steps exactly what it
  is told to, and the probe and the room server have no `Feedback` at all.
  The kill that decides the round is `Session::time_scale`'s.

Reduced motion (`motion.rs`) leaves the particles and the health number's
colour and takes the rest: no shake, no edge pulse, no hull flash on a
slam, no drop of the number, no hold, and nothing bending the picture -
no ripple, no muzzle puff, no impact punch. What flashes is calmed (WCAG
2.3.1): a whole-screen flash - a kill's or a blast's, a lightning strike's
- keeps `reduced_flash_scale` of its strength (`motion::flash`), a strike
is one flash stepping down in quarters with no second flicker
(`weather::lightning_shown`), and a lit fuse beats once a second between
two held steps, by day and at night, instead of strobing (`motion::beat`).
The switch is the `reduce_motion` row; iOS and the web read the platform's
setting, and there is no control for it in the game yet.

The particle layer stands still with the round (`app::fx_runs`): under a
dialog, the level select, the builder or the lobby and while paused its dt
is 0, so a burst hangs where it was and goes on when play does.

## Adding an effect

Compose it as a pure function returning `Vec<pyro::Shape>` from what it is,
its age and a seed; take its colours from a ramp; give it one glow; let
its smoke lean (`pyro::smoke_lean`); draw it with `pyro::draw` in the pass
it belongs to and its glows in that pass's additive block. Test it
headless: that it is on the grid, in its ramp and gone by its end, and
that the same inputs give the same shapes. The ignored `preview` tests in
`fireball.rs` and `mushroom.rs` write strips to `target/` for a look
without a window.
