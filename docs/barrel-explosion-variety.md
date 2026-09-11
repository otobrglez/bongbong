# Oil barrel explosions: more variety

Status: research and proposals, 2026-09. Nothing here is built. A live
sketch of every proposal in section 4 (canvas, palette-true, hash-seeded)
is the "Barrel Blast Lab" artifact linked from the session that wrote this. Written
against `simulation/props.rs`, `blast.rs`, `fx.rs`, `combat.rs`,
`tools/spritegen/gen_barrel_explosion.py` and the `group props` rows of
`tuning.rs`; docs/sandbags-barrels-fences.md is the original PRD and
docs/PROPS_SPEC.md the sheet layout.

Contents

1. What a barrel does today, and why every blast looks the same
2. What other games do with a barrel
3. The rules any proposal has to respect
4. Proposals
   - A. Same barrel, a different show every time (cosmetic, cheap)
   - B. Barrel *types* (behaviour, map-authorable)
   - C. What a blast does to the ground around it
   - D. Oil trails: the chain reaction as a map-building tool
5. Recommended order
6. How to verify each step
7. Where else the same treatment pays off

## 1. What a barrel does today

The mechanics are in `Game::damage_obstacle` / `apply_blast` /
`tick_fuses` (`simulation/props.rs`). A barrel dies three ways and all three
end in the same `apply_blast`:

| Trigger | Path | Delay |
|---|---|---|
| A shot | `Obstacle::damage` against `barrel_max_health` (18) | none, the frame health hits zero |
| A ram | `barrel_ram_damage_per_second` (40) | about half a second of pushing |
| Another blast | health zeroed, `fuse = barrel_fuse_seconds * (0.5 .. 2.5)` by distance | 0.09 s at the centre to 0.45 s at the edge |

`apply_blast` then does exactly one thing, every time, whatever set it off:

- damage everyone inside `barrel_blast_radius` (96 px) with a linear
  falloff, `barrel_blast_damage_min..max` (15..30) rolled per victim,
  knockback `barrel_blast_knockback_speed` (140);
- fuse every other barrel in range, hurt every tile in range;
- push one `BlastFx` (the single 12-frame fireball on
  `barrel_explosion.png`, mirrored by a position hash), one `Shockwave`
  at `SHOCK_BARREL` (0.7 of a kill), one impact quad, one `Scorch` (3
  variants, mirrored and quarter-turned by hash), and a screen flash
  through `flash_screen` (rate-limited to one per 0.35 s);
- `fx.rs` adds 18 sparks and 7 smoke puffs (half that on a chained link);
- `obstacle_died` leaves one static rubble decal (`RUBBLE_ROW_BARREL`).

The two barrel art variants (rusty red drum with a band, grey drum with a
hazard rim and teal bung, `gen_props.py`'s `BARRELS`) are rolled per tile
in `battlefield::spawn_from_map` and are cosmetic only. So the whole
variety budget today is: one of two drums, one fireball with one of two
mirrors, one of 24 scorch forms. Everything else, radius, damage, timing,
smoke, shake and flash, is identical for a lone barrel, the twelfth link of
a chain, a rammed barrel under a tank and a barrel a shell clipped from
across the map.

Things the code already has that a barrel does *not* use:

- **Delayed cook-off pops** (`Game::wreck_fx`, `tick_cookoffs`,
  `BlastFx::small`): a wreck gets 2 small secondaries over 1.6 s; a barrel
  gets none.
- **Thrown debris** (`Decal::thrown`, the arc/flight/shadow machinery): a
  wreck throws 5 parts; a barrel drops one static decal in place.
- **Scorched tread marks** (`scorch_tracks`): a kill burns the last tracks
  under it; a barrel does not.
- **Burning tiles and their emitters** (`Obstacle::tick_burn`,
  `Game::burning_tiles`, the `wood_ember_rate`/`wood_smoke_rate` sampling
  in `fx.rs`): fire exists as a state with particles, but only on wood and
  trees.
- **Blast strength ratios** (`SHOCK_KILL`/`SHOCK_BARREL`/`SHOCK_FROG`/
  `SHOCK_COOKOFF`): one constant per *event*, none per barrel.
- **Grass crush** (`grass.rs`'s `crush`): grass flattens under a hull, but
  a blast leaves it standing.
- **The hit point and direction** (`Event::Hit { x, y }`, the projectile's
  velocity in `resolve_projectiles`): known at the moment a shot pops a
  barrel and thrown away; the fireball is always centred and symmetric.

One stale comment to fix on the way: `obstacle_died` says props leave no
rubble, but `Material::rubble_row` maps Barrel to `RUBBLE_ROW_BARREL` and a
barrel does leave a decal.

## 2. What other games do with a barrel

The reference points, in the order they matter for a top-down arcade tank
game:

- **Doom** is the chain-reaction benchmark. A barrel is a monster with hit
  points; its blast has a fixed 128-unit radius, damages other barrels,
  and the "barrel frag" (wait for the crowd, shoot the barrel) is the whole
  tactic. Bongbong already has this, fuse cascade included. Doom gets its
  variety not from the barrel but from *placement*: rows, rings, barrels
  in doorways, and the map "Barrels o' Fun" built entirely around it. The
  lesson is that the map editor is the first variety tool, and section D
  builds on that.
- **Half-Life 2** has one explosive barrel with two behaviours: shot, it
  detonates; set alight, it burns for a few seconds and then *launches*
  itself skyward before going off, which is what makes a stack of them
  read as fireworks rather than one bang. The delay plus the launch is
  the memorable part, and both are cheap in a top-down game: a fuse that
  can be long, and a decal that flies.
- **Worms / Liero-style games** vary the *cause*: an oil drum leaves a
  burning pool that keeps hurting, a mine is the concussive one. Two
  kinds of "explosive" in one palette, distinguished by what they leave
  behind.
- **Crysis and later shooters** show the failure mode: an explosion that
  is only a bigger particle system. The variety that reads is in the
  after-effects (fire, thrown objects, a second bang), not in more
  particles per bang, which matters here because `fx_max_particles` is a
  hard cap the web build halves.
- **Pixel-art explosion practice** (the Lospec and Penusbmic tutorials)
  is consistent on three things: a limited fire ramp, an expanding
  silhouette that changes *shape* between frames rather than just size,
  and "sequential bursts", smaller explosions inside or after the main
  one. The current sheet does the first two. The third is the missing
  one, and it is also what the code already has infrastructure for
  (cook-offs).

The variety that pays off is therefore: (1) the same barrel not looking the
same twice, (2) two or three barrel kinds a map author can place on
purpose, (3) blasts that change the ground, and (4) chain reactions the
author can *draw*.

## 3. The rules any proposal has to respect

These are the constraints from the module map and the conventions; every
proposal below is written against them.

- **Determinism on the simulation path.** Anything that affects gameplay
  draws from `Frame::rng` in a fixed order. Anything cosmetic hashes
  `blast::seed_at(position, salt)`, never RNG. A map with no barrels must
  draw exactly the RNG it draws today (`props_tests::
  a_map_without_props_spawns_the_same_round_it_always_did`), and
  `just probe-fixtures` must stay byte-identical for every fixture a
  change does not touch. The rule that makes this cheap: **a zero-chance
  knob draws no RNG**.
- **No RNG on the simulation path for cosmetics; free RNG in `fx.rs`.**
  Particles vanish and never feed back, so `fx.rs` may roll freely;
  anything whose position matters later is a `Decal` chosen in
  `simulation/` from a hash.
- **The 2 px block.** New sheet art is authored at 64 px and drawn at
  scale 2 (`blast_anim_scale`); glows go through `pixel_disc`; a smooth
  circle or a fractional offset reads as a different game.
- **Palette.** Everything on `barrel_explosion.png` and `props_sheet.png`
  is on the Puny Palette and contains no green (`just check-sheets`).
- **Screen-level effects are rate-limited.** One flash per
  `blast_screen_flash_min_gap_seconds`, `SHOCK_MAX` (4) ripples resolved
  in one blit, shake clamped by `camera_shake_max_stack`. Variety must
  not reopen the strobe.
- **Feel numbers are `tunables!` rows**, never `pub const`s in `lib.rs`.
- **Shaders need a hand-ported GLSL ES 100 twin** in `static/web/`.
- **Particles are capped and the web budget is half.** A proposal that
  needs more particles per blast is the wrong proposal; one that needs a
  *later* burst is fine.
- **The AI reasons over snapshots**, and concealment is already gated in
  four places. Anything that blocks sight or changes passability touches
  the probe baselines and needs re-baselining, so it is called out where
  it applies.

## 4. Proposals

### A. Same barrel, a different show every time

All cosmetic, all seeded from `seed_at`, none of it changes a single RNG
draw or a probe number. This is the cheapest and the most visible tier.

**A1. Three fireball sheets instead of one.** Add rows 2..4 to
`barrel_explosion.png` (the sheet is 768x128 today; growing it to 768x320
keeps every existing constant): a *tall* blast (a column that rises and
narrows, smoke going "away" from the camera fast), a *flat* one (a wide,
low splash of fire with a ring of debris and almost no mushroom) and a
*double* one (two overlapping cores a few pixels apart, the second a frame
behind). `BlastFx` gains a `row: i32` picked as `(seed >> 8) % 3`;
`draw_blast` also gains the quarter-turn the scorch already has, which
takes 3 sheets x 2 mirrors x 4 turns to 24 apparent forms. The generator
already has every primitive (`lump`, `rays`, `debris`, `embers`); each new
row is one more `frame(f, rng)` branch. Cost: one generator day, one
`i32` on `BlastFx`.

**A2. Seeded timing jitter.** `BlastFx::frame` plays at `blast_anim_fps`
scaled by `0.85..1.15` from the seed, and `draw_blast` scales the frame by
`0.9..1.1`. Two chained blasts side by side then never step in lockstep,
which is the single biggest "cloned" tell in a cascade today (the mirror
alone does not hide a synchronised frame change). Cost: a few lines.

**A3. Cook-offs for barrels.** Reuse `Game::cookoffs` as `wreck_fx` does:
after a blast, queue 0..2 `BlastFx::small` pops over `cookoff_window_seconds`
at hashed offsets. Make the count hashed too, so a third of barrels get
none, a third one, a third two. This is the "sequential burst" every
explosion tutorial asks for, and the queue, the small fireball, the
`SHOCK_COOKOFF` ripple and the `Event::CookOff` spark burst already exist.
New knob: `barrel_cookoff_max: i32 = 2`. Cost: ten lines in `apply_blast`.

**A4. Thrown drum parts.** In `obstacle_died` for `is_explosive()`
materials, throw `barrel_parts` (default 3) `Decal::thrown` pieces from
`RUBBLE_ROW_BARREL` exactly as `wreck_fx` throws tank parts, hashed angles
and distances, in addition to (or instead of) the static decal. A lid
skidding to a stop two cells away is what sells "that was a drum" rather
than "that was a fireball sprite". If `RUBBLE_ROW_BARREL`'s 8 variants read
as "pile" rather than "part", `gen_walls.py` gets two more columns (a lid,
a split staved half); the row has room. Cost: small, and it uses the
Phase-3 hook `Decal::blocks` was left for.

**A5. Cause-shaped blasts.** The three death paths carry information the
fireball throws away:

- *Shot*: `resolve_projectiles` knows the projectile's direction. Offset the
  `BlastFx` centre 6..10 px along it and pick the *flat* sheet from A1
  more often. The fire then leans away from the shooter.
- *Rammed*: the barrel is under a hull. Skip the fireball offset, use the
  *tall* sheet, and give the ramming tank an extra shove along its own
  heading (`explosion_hit` already knocks it back; the extra is a factor
  on `params.knockback` for the rammer, knob `barrel_ram_kick_factor`).
  A tank that drives into a barrel should visibly lurch.
- *Chained*: offset the fireball away from the blast that lit the fuse and
  scale it with the fuse it had (`0.5..2.5`): a barrel at the edge of a
  blast that smouldered for half a second goes up bigger than one that
  went instantly. Store the source position on `Obstacle::fuse` (make it
  a small struct `Fuse { left, from: Position }`).

All of it is a `BlastKind`/offset on `BlastFx` and `DeadTile`, decided in
the simulation from data already in hand, with no new RNG. Cost: a day.

**A6. Fuse tells.** The lit-fuse column and the pulsing glow already
exist. Add: the drum *rocks* (a hashed +-2 px lateral offset at 12 Hz in
`draw_obstacle` while `fuse.is_some()`, whole blocks only), and `fx.rs`
samples fused barrels the way it samples burning tiles, emitting one
spark every few frames from the bung. A long fuse (see B2) then reads as
"it is about to go" instead of "it froze".

**A7. Tracks and grass.** Call `scorch_tracks(center)` from `apply_blast`
too, so tread marks through a blast stay burnt in. And set `crush = 1` on
every tall-grass tuft inside `barrel_blast_radius * 0.6` (the same field
`tick_grass` drives under a hull), so a blast in a meadow leaves a
flattened ring that recovers over `grass_crush_recover_seconds`. Both are
cosmetic fields the simulation already owns; no RNG.

### B. Barrel types

Today's two art variants become two behaviours, plus one new drum. The
map author can pick one; a bare `kind = "barrel"` keeps rolling. Types are
a `BarrelKind` on `Obstacle` (or a `variant` the material interprets),
`BlastParams` grows a per-kind constructor, and `group props` gains one
row per differing knob.

**B1. Oil drum (the red one): fire.** The blast is what it is today, but
it leaves a **burning pool**: every cell inside `oil_pool_radius` (default
1 cell around the centre, so a plus shape) becomes a burning ground cell
for `oil_pool_seconds` (default 4). A tank whose hull box overlaps a
burning cell takes `oil_pool_damage_per_second` (default 6, so crossing it
costs about a shell's worth) and a wood tile, tree or barrel *in* a burning
cell ignites (a barrel gets a long fuse, see B2). Implementation: a
`Game::fires: Vec<GroundFire { cell, left }>` list beside `scorches`,
ticked in a `tick_fires` phase after `tick_burns`, drawn as the same
3-frame burn loop the wood tile uses (`walls_sheet.png` cols 4-6, so no
new art), and sampled by `fx.rs`'s `burning_tiles` emitter (extend
`burning_tiles` to return ground fires too, and the embers and smoke come
for free). The pool is the after-effect that makes an oil drum *oil*, and
a burning doorway is a tactical object: the AI's `Terrain` snapshot should
mark burning cells as blocked for pathfinding (a `Grid` cost, not a wall)
so enemies route around it or wait, which is a probe-baseline change on
`maps/test/props.toml` only, since no other fixture has a barrel. Knobs:
`oil_pool_radius_cells`, `oil_pool_seconds`, `oil_pool_damage_per_second`,
`oil_pool_chance` (1.0 by default; 0.0 draws nothing).

**B2. Fuel drum (the grey hazard one): concussion and a launch.** Bigger,
sharper, shorter: `fuel_blast_radius` 128, damage 20..40, knockback 220,
no pool, a whiter and faster fireball (A1's *tall* sheet at 1.15x
`blast_anim_fps`), a harder shake (`SHOCK_FUEL` 0.9), and, when *chained*
rather than shot, a Half-Life launch: instead of the fuse popping it in
place, the drum flies. Mechanically that is a `Decal::thrown` of the
intact drum sprite over `debris_flight_seconds` to a landing cell chosen
in the simulation (hashed direction away from the source blast, distance
2..3 cells, snapped to a free cell with `Grid::nearest_open`), and a
*second* `apply_blast` at the landing spot when it lands, with the drum's
`Obstacle` despawned at launch. The landing blast is real damage, so the
landing cell is a simulation decision (no RNG needed, it is hashed from
the two positions, and the hash is deterministic); the flight is cosmetic
and cannot move it. This is the one proposal that changes what a chain
*does*: a fuel cluster does not cascade in place, it throws drums into the
room. Knobs: the four blast numbers, `fuel_launch_chance` (default 1.0
when chained, 0 = never launch, no RNG drawn), `fuel_launch_cells`.

**B3. A third drum: the dud, or the water drum.** A plain drum (blue band,
no hazard rim; one more `BARRELS` entry in `gen_props.py`, one more sheet
row) that *usually* does nothing dramatic: shot, it splits with a hiss
(a dust and steam burst, no blast, no damage) and leaves a puddle decal;
rammed, it collapses like a sandbag. Its job is contrast, the anticlimax
that makes the real ones land, and cover for the map author who wants
drums as scenery in a depot without turning the depot into a minefield.
At `dud_secret_chance` (default 0.1, rolled at spawn so a zero draws no
RNG) a "water" drum is actually fuel, which is the Doom trick in reverse:
a wall of harmless drums with one live one somewhere. The AI's
`enemy_breach` logic already shoots destructible tiles it is stuck
behind, so a hidden live drum in a corridor is a trap that works on both
sides.

**B4. Map and editor plumbing for the types.** `CellObject::Barrel` gains
an optional `kind` (`{ kind = "barrel", drum = "oil" | "fuel" | "water" }`;
absent = roll). The builder's PROP category gets one tool per drum (three
more `Tool`s, 27 in `TOOLS`; `Tool::object()` returns the typed cell), the
cursor readout names it, and `maplint` warns on a fuel drum whose launch
lane is fully walled (it would just detonate in place, which is fine, but
the author probably wanted the launch). Determinism detail: an explicit
drum must still *consume* the variant roll that a bare barrel draws today
(`rng.random_range(0..variants())` and discard), otherwise every map that
adds one typed drum shifts the RNG stream of every enemy spawn after it
and the existing fixtures can never be compared to a typed version of
themselves.

**B5. Fuse length as a type trait.** Oil smoulders (`barrel_fuse_seconds`
x 3, with A6's tells), fuel goes off almost at once (x 0.5), water never
fuses. A mixed cluster then cascades with rhythm: the fuel drums crack
first, the oil drums go up one by one afterwards while their pools spread.

### C. What a blast does to the ground

**C1. Bigger scorch, more scorch.** The three scorch decals are drawn at
`scorch_scale` 2.0 under everything, once per blast, at one size. Scale
the scorch by the blast kind (fuel 1.3x, oil 1.0x plus the pool leaving
its own darker mark when it burns out), and add three more variants to row
1 of the sheet (cols 3-5 are blank): a directional *streak* scorch for
shot barrels (rotated to the shot direction in 90-degree steps, which is
what the quarter-turn is for), and two oil-splatter shapes. Cheap and it
compounds with A5.

**C2. Blasts throw rubble that is already lying there.** Decals inside a
blast radius are re-thrown: `explosions` walks `self.decals`, and every
landed decal within `radius * 0.7` gets a new `Decal::thrown` from where
it lies to a hashed spot further out. It costs nothing new (the flight
code exists), the scene visibly *reacts* to a second blast in the same
place, and because landing spots are decided in the simulation from
hashes, replays are unaffected. Cap it at 8 per blast so a cascade over a
levelled wall does not churn 200 decals.

**C3. Wall damage that shows direction.** Tiles inside a blast take
falloff damage today and the edge-cap overlay refreshes on destruction.
Add a *blackened face* to the cap overlay: `Obstacle` gets a `scorched:
u8` mask of faces that took blast damage, and `draw_obstacle_cap` draws
those faces from a darker cap row (rows 22-25 of `walls_sheet.png` have a
sibling block free below the rubble rows). A wall next to a barrel then
carries the mark, and a corridor after a cascade is visibly the corridor
the cascade ran down. Sheet work plus one mask; no simulation change.

**C4. Ground tint scorch.** `ground.rs` bakes per-cell tints at build
time for wall ambient occlusion. Making `GroundGrid::tints` mutable and
darkening the cells under a burnt-out oil pool (B1) is a permanent crater
tone under the scorch decal that the decal alone cannot give (decals are
capped at `SCORCH_MAX` 64 and evicted oldest-first; a tint is not). Only
worth it with B1; without a pool the scorch decal is enough.

### D. Oil trails: the chain reaction as a map-building tool

The game's stated twist is "build your own battlefields", and Doom's
barrel variety came from placement. A cascade today only propagates through
`barrel_blast_radius`, three cells, so a chain is always a cluster. An
**oil trail** is a ground cell (`kind = "oil"`, painted in the GROUND
category next to road and tall grass, not solid, no nav effect) that a
blast or a burning cell ignites, and that burns along its length at
`oil_trail_cells_per_second` (default 6) as a running fire, igniting the
next oil cell, any wood or tree it passes and any barrel it reaches (a
long fuse, B5). It reuses B1's ground-fire list entirely: an oil cell is
just a cell that *can* become a `GroundFire` when a neighbour is one.

What it buys the author: a fuse line from a lone barrel at the entrance
to a depot at the back, a ring of oil around the frog that the player can
light, a trap the enemy drives across. What it buys the player: a
tactical object that is legible (a dark streak on the ground with a
sheen, `SHEEN` and `PUDDLE` are already palette entries in `gen_props.py`)
and that rewards a well-timed shot from across the map. Art is one 32 px
ground cell with 4 variants on `props_sheet.png` (rows 9-10 free if the
sheet grows to 128x352) plus the wood burn loop for the lit state.

Rules to get right: an oil cell under a tank does nothing until lit;
propagation is per cell, one step per `1/cells_per_second`, so a running
fire is a deterministic wave with no RNG; a lit oil cell damages like a
pool cell; the AI treats a burning cell as high-cost in `Grid` (B1's
change) and an unlit oil cell as open ground, so an enemy that drives over
a trail is not cheating, it simply does not know. Probe fixtures without
oil are untouched; `maps/test/props.toml` gets a trail.

## 5. Recommended order

Ordered by visible payoff per day and by risk to the seeded baselines.

1. **A2, A3, A4, A7** (jitter, cook-offs, thrown parts, tracks and grass).
   Two days. Pure cosmetics on infrastructure that exists; no baseline
   moves. This alone stops two adjacent barrels looking cloned.
2. **A1 plus C1** (three fireball sheets, quarter-turns, more and bigger
   scorches). Two to three days, mostly in `gen_barrel_explosion.py`.
   Verify with the dev server screenshot loop and `just check-sheets`.
3. **A5 and A6** (cause-shaped blasts, fuse tells). One to two days; a
   `Fuse` struct and a `BlastKind` on `BlastFx`.
4. **B1, B4, B5** (oil pool, typed drums in the map and editor, fuse
   traits). Four to five days. First real gameplay change: re-baseline
   `maps/test/props.toml` in `just probe-fixtures` and add
   `props_tests` for the pool (crossing it costs health, it ignites wood,
   it expires) and for the discarded variant roll.
5. **B2** (fuel drum with the launch). Three days. The second blast at
   the landing spot needs its own test (a launched drum lands in an open
   cell, never inside a wall, and its blast is counted as `chained`).
6. **B3** (water/dud drum). One day once B4 exists.
7. **C2, C3** (re-thrown rubble, blackened wall faces). Two days, nice to
   have; C2 first, it is nearly free.
8. **D** (oil trails). Four days on top of B1. Ship it with a new
   `maps/test/` fixture and a shipped map that shows it off, or nobody
   will find it.

Skip, or defer with a reason:

- A *bigger* particle burst per blast: the cap is the cap, and the web
  build halves it. Variety in *when* (A3), not *how many*.
- Blast-spawned real projectiles (shrapnel as `Bullet`s with an owner):
  a balance change across every map with a barrel, it doubles the hit
  loop's work during a cascade, and thrown decals (A4) give the visual
  without it.
- A smoke cloud that blocks line of sight: it would reuse
  `Terrain::conceals`, but concealment is gated in four places for a
  reason (see `grass.rs`'s module-map entry) and every AI baseline moves.
  Only after B1 has settled, and only as its own project.
- A new ripple shape per barrel kind: `shockwave.fs` accumulates up to
  `SHOCK_MAX` rings in one blit and a second shape means a second uniform
  set plus a hand-ported GLSL ES twin; the strength ratio (`SHOCK_FUEL`)
  gives most of the difference for none of the cost.

## 6. How to verify each step

- **Cosmetic tiers (A, C1, C2):** `cargo test --lib props` and
  `just probe-fixtures` must pass unchanged, a JSON diff of every fixture's
  probe records is byte-identical, and `just check-sheets` passes with the
  new rows. Then the dev-server loop from docs/dev-server-design.md:
  `restart {map_toml}` with a 2x2 cluster and a 5-in-a-row chain,
  `step` to the pop, `screenshot` every 3 frames; the screenshots are the
  review artefact. `props_tests::a_dying_tank_throws_wreckage_and_cooks_off`
  is the template for the barrel versions of A3 and A4.
- **Type tiers (B, D):** one `props_tests` test per promised rule (the
  list in section 5), a `maplint` check for the new cell kinds, and a
  re-baselined ceiling for `maps/test/props.toml` only, recorded in
  docs/gameplay-verification-design.md's baseline table with the reason.
  `a_map_without_props_spawns_the_same_round_it_always_did` is the guard
  that a barrel-free map draws no new RNG; extend it with a
  "typed-drum-free map" variant once B4 lands.
- **Feel:** every new number is a `tunables!` row, so the web PR preview's
  tuning panel is the playtest surface; `Copy as Rust` brings the QA'd
  values back into `tuning.rs`.

## 7. Where else the same treatment pays off

A pass through `fx.rs`, `game.rs`, the projectile modules and the event
log, looking for the moments that share the barrel's machinery. The
structural finding first: `Fx::observe` consumes 6 of the 27 `Event`
variants (`RoundStarted`, `ObstacleDestroyed`, the surviving-obstacle
`Hit`, `Wreck`, `Blast`, `CookOff`); the rest fall into the `_ => {}` arm.
Thirteen of the ignored ones are real gameplay moments that already carry
a position, so a particle response for each is an arm in `fx.rs` and no
simulation change at all. The `strength` field on `impact_flashes` and
`muzzle_flashes` is never read either: only `shocks` uses `gains`, so every
impact quad and every muzzle shimmer is the same size and force whatever
fired or was hit.

Ordered by cost. *Free* means an `fx.rs` arm or a field that exists;
*cheap* means a sheet row or a hashed cosmetic; *gameplay* means a
play-test and possibly a baseline.

- **Shells hitting a tank** (free). `Event::Hit { Player | Enemy }` carries
  damage, killed and the hit point and is dropped; a tank gets the generic
  130 px heat quad and its health ring blinks on. A shell chipping a fence
  throws more than a shell hitting steel. Add a directional spall burst
  scaled by damage (the `tile_chip` pattern), one frame of white hull tint
  (`Tank::tint` is only a despawn fade today), and lean the burst away
  from the shooter so the hit side reads.
- **Shell-vs-shell cancel** (free). `Event::ShellsCollided` is ignored; the
  rarest moment in the game reads like a wall hit. A double white spark
  burst plus a `SHOCK_COOKOFF`-strength ripple. Only `Shell` is queried for
  cancels; plasma and bullets never meet.
- **Ricochets and barrel deflections** (free). The impact quad fires and
  the shell flies on in its plain frame. A directional spray along the
  reflected normal from a new `Event::Ricochet { x, y, nx, ny }`, a
  brighter sprite on the last bounce (`bounces_left` is on the shell), a
  hashed scuff decal on the iron.
- **Muzzle flash** (cheap). `muzzle_flash.fs` is a displacement only, no
  light; shell, plasma and laser push an identical `Shockwave::new`; the
  minigun's Muzzle state lasts one frame. An additive `pixel_disc` bloom
  at the tip per weapon colour (the same call `draw_blast_glow` makes), a
  smoke puff for heavy chassis, and the unused `strength` field to grade
  them.
- **A tank dying** (cheap). `wreck_fx` draws the barrel's fireball sprite
  verbatim, and `Fx::wreck` throws stone-coloured chips from the wall
  palette. Once A1 makes sheet rows cheap: a hull-shaped row with the
  turret as a thrown `Decal` that stays, chips in the chassis accent, and
  a burn-out transition (the embers stop the instant `wreck_burn_seconds`
  ends).
- **Frog bite, hop and death** (free). Bite: a filmstrip frame and nothing
  on the victim. Hop: no dust, no landing puff, no shadow (the frog is the
  one standing thing with none). Death: a 0.6 ripple, no screen flash,
  though it ends the round. A victim burst and short shake on
  `FrogBite`, dust and grass crush on landing with a height-scaled shadow
  like thrown debris, and on death the flash, a toxic-green scorch and
  lingering smoke sampled like a burning wreck.
- **Pickups** (free). A static unrotated blit; collection vanishes with no
  feedback (`PickupCollected` ignored); respawn pops in; the speed boost
  has no visual beyond moving faster. A collect burst in the accent
  colour, a hashed 2 px bob and a ground ring, and for the boost a doubled
  track density plus a dust wake, all fields that exist.
- **Laser** (cheap). Two alpha-faded lines; the hit reuses the warm-orange
  impact quad even for the blue variant; no mark where the beam ends. A
  thin streak decal at the end point, a core that narrows over
  `laser_beam_display_seconds`, and the impact quad tinted by variant.
- **Shield deflections** (free). `Event::Deflected` is ignored and the
  stolen shell flies on unchanged. A rainbow burst at the contact point,
  a brief brightening of the ring, and the shell tinted to the shield's
  hue for its flight.
- **Ramming** (free). Contact sparks come from rate-limited impulse
  sampling and can miss the damage moment; `Event::Ram` is dropped. A
  burst on the event scaled by damage, and the one-frame hull flash A5's
  ram lurch wants.
- **Boundary wall hits** (free). No particles at all; `impact.fs` was
  patched to stay visible on bright ground instead. The brick `tile_chip`
  arm from `Event::Hit { Wall }`, and a scuff decal.
- **Wave roll-in and round end** (cheap). A wave tank enters with no dust;
  the WAVE banner has no motion; the round end dims the field and does
  nothing to the world. Gate dust from `TankEntered`; a last `SHOCK_KILL`
  ripple centred on the deciding event, then a slow desaturation.
- **Ripple, shake and flash as a set** (gameplay, for the play-test rather
  than the simulation). Four events push a ripple (kill 1.0, barrel 0.7,
  frog 0.6, cook-off 0.1) and two flash the screen; a player taking a
  heavy hit never shakes the camera. A 0.2 shake on a heavy hit to the
  player gated by damage, and per-kind drum strength (`SHOCK_FUEL`) once
  B2 lands.

Sources consulted for section 2: the Doom Wiki's barrel entry
(doomwiki.org/wiki/Barrel), the ZDoom `ExplosiveBarrel` class notes, Aidan
Grennen's write-up of "Barrels o' Fun", the TV Tropes and Giant Bomb
"Exploding Barrels" pages for the cross-game survey, the SourceRuns
barrel-flying thread and the "What You Didn't Know About Half-Life 2's
Explosive Barrels" breakdown, and the Lospec and Penusbmic pixel-art
explosion tutorials.
