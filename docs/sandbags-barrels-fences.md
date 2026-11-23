# Sanbags, oil barrels and fences

This is requirement document to add three new items to the game.

## Visuals

- All three items need to be portrayed from the top. That is the perspective of the game. They could be slightly tilted so that they have some shape to it.

## Mechanics

- All three itemss should be placed on the map, same as with walls and other elements that the game supports.

## Sandbags
- Tanks can shoot over them as well as at them. Come up with some ration of wht passes over and what does not.
- They are destroyed if tank rams into them
- Visually there should be three different kinds of similar bags in different arrangements, but they should also have each 3 different states based on level damage.
- The idea of sandbag is more to slow down progress but be destroyable.

## Oil barrels
- Barrel should also be placed on the map
- Barrel should be such that once it is damaged enough it explodes.
- When barrel explodes it impacts close by barrels that should also explode (Doom like)
- Sometimes bullets bounce away (in small percentage) and some times bullets fly over them in also small percantage. But i a lot of cases they explode spectaculary
- There should be 2 variations of barrel
- Figure out how to create interesting visual for the explosion
- This is now and important and interesting part of the game. So make it spectaculat. especially around explosion and impact on the environment.

## Fences
- Have 2 variations of fences.
- In 70% of cases shooting at it destroys them. In other cases they get easily destoryed. 
- Visually also have keyframes for damaged state

## Acceptance criteria

- In map editor user should be able to place these elements on a map
- They should all have colors in similar style/guidience then the rest of the game
- All of them should be destructable
- There should be some knobs that can be tuned later on as well
- Create also a test map with these elements that can be used for testing 
## Mechanics as built (2026-09)

All three are `obstacle::Material` variants (`Sandbag`, `Barrel`, `Fence`)
placed as `kind = "sandbag" | "barrel" | "fence"` cells; the variant is
rolled per tile at spawn. Rules live in `simulation/props.rs`; art in
`static/props_sheet.png` / `static/barrel_explosion.png` (docs/PROPS_SPEC.md);
knobs in `tuning.rs`'s `group props`. Test map: `maps/test/props.toml`.

- **Sandbags**: a shot passes over at `sandbag_pass_over_chance` (0.35) odds,
  rolled per projectile per tile, else hits (`sandbag_max_health` 45 over
  three stages). A tank pushing into one collapses it after
  `sandbag_ram_seconds` (0.4). Don't block line of sight. Pushing is the
  hull's body going into the prop: at `prop_ram_min_speed` (2 px/s) or more
  along the contact between them as the step began, whether driven, knocked
  or skidding - a hull grazing along a prop's face does not ram it. The
  same test runs a fence's and a tree's clock and a barrel's ram damage.
- **Barrels**: `barrel_max_health` 18; a lethal hit or a hard knock
  (`barrel_burst_speed`, "Loose barrels" below) detonates it at once. The blast
  (`barrel_blast_radius` 96, `barrel_blast_damage_min/max` 15/30 with
  linear falloff, `barrel_blast_knockback_speed` 140) hurts everyone — both
  sides, bongs, walls and props — and puts every barrel in range on a
  fuse of about `barrel_fuse_seconds` (0.18; shorter near the centre,
  longer at the edge, then times the drum's own factor), so a cluster
  cascades outward. Shots fly over at `barrel_pass_over_chance` (0.08) and
  shells/bullets ricochet at `barrel_deflect_chance` (0.1). Visuals:
  fireball sprite, additive bloom, screen flash, the shockwave ripple and
  camera shake, and a scorch mark. **The two liveries are two kinds**
  (docs/barrel-explosion-variety.md, 2026-09): the red *oil* drum
  smoulders on a fuse (`oil_fuse_factor` 3) and leaves a burning pool
  (`oil_pool_*`); the grey *fuel* drum cracks first (`fuel_fuse_factor`
  0.5), blasts harder (`fuel_blast_*`) and, when another blast sets it
  off, launches two or three cells and detonates where it lands
  (`fuel_launch_*`). A map pins a kind with `drum = "oil" | "fuel"`.
  Every blast varies by position hash (four fireball sheets, quarter-turns,
  fps and size jitter), by cause (a shot leans the fire downrange and
  streaks the scorch, a ram lurches the hull and goes up as a column, a
  chained drum leans away from what lit it and grows with how long it
  smouldered), throws drum parts, queues cook-off pops, flattens grass,
  burns tread marks in, re-throws rubble already in the radius and soots
  the wall faces that looked at it. `kind = "oil"` ground cells are
  trails a fire runs along (`oil_trail_*`) to whatever drum is at the end.
- **What an enemy sees over, a shot can pass over.** Every tile with
  `Material::blocks_sight` false has a `pass_over_chance`: sandbags and
  barrels as above, a fence's mesh (`fence_pass_over_chance` 0.5), a lamp
  post (`lamp_pass_over_chance` 0.75), a fir sapling
  (`sapling_pass_over_chance` 0.35), a palm (`palm_pass_over_chance` 0.6)
  and a snag (`snag_pass_over_chance` 0.5) - so an enemy firing across one
  is not firing into a wall it cannot see. The roll is a lobbed shot's:
  shells, bullets, plasma. A laser's beam and a gauss rail's slug fly flat
  at the muzzle's height, roll neither a pass-over nor a deflection, and
  strike every tile on their line.
- **Fences**: a hit on a pristine fence destroys it at
  `fence_one_shot_chance` (0.7) odds, else it goes to its damaged keyframe
  and the next hit finishes it; rammed through in `fence_ram_seconds`
  (0.15). Don't block line of sight.
- **How they feel to drive into** (BB-205, docs/physics-engine-design.md
  "Hard and soft"): sandbags, fences, lamp posts and range boards are
  *soft* - a hull coming at one head on slows over its last ~10 px
  (`soft_cushion_decel`) and settles against it with no dead stop and no
  bounce, where iron, concrete and brick stop it flush and throw it back
  off a fast hit (`hard_bounce`). A drum is *firm* (a ~2 px give). A lamp
  post snaps under a ram (`lamp_ram_seconds`) and a range board tips over
  (`target_ram_seconds`), as a fence does.
- Events: `obstacle_destroyed { material, x, y }` for any tile death,
  `blast { x, y, chained, drum }` for a detonation, `drum_launched { x, y,
  to_x, to_y }` for a fuel drum leaving the ground, and `fire_started { x,
  y, pool }` for a ground cell catching.

## Loose barrels (BB-206, 2026-11-22)

Oto: "can barrels also be movable meaning that they don't behave same as
wall but more like objects that can be moved/bounced etc." A barrel is a
loose object now: still placed as a `kind = "barrel"` cell in the builder,
but in the round a dynamic body a hull shoves, a blast throws and a wall
throws back. `Material::is_loose` names it (a barrel today); the rules are
`simulation/props.rs` (`sync_loose`, `throw_loose`), the body
`Physics::spawn_loose`, the knobs `group props`.

**What a player sees.** Drive into a drum at a hull's own pace and it is
shoved ahead and slides about two cells, slowing to rest. Roll one at a
wall and it bounces off; it bounces off hulls and other drums too. A blast
throws the drums round it - the ones it lit as well, which go off where
they come to rest - out to `barrel_throw_reach` (2.2) times its radius. A
drum goes off on a hard enough knock: rammed by a hull going faster than
any drives (a speed boost, a hull itself thrown), thrown hard into a wall
or into another drum. Shots set it off as ever, and the two drums keep their
own explosions, fuses, launches and pools (docs/barrel-explosion-variety.md).

The decisions, each system in turn:

- **The body** (`Physics::spawn_loose`): a dynamic ball of the tile's own
  half extent (12 px), rotation locked - a drum standing upright slides, it
  does not spin - of `barrel_mass` (0.35 of the standard chassis), rapier's
  linear damping `barrel_damping` (1.6) and a steady ground friction on top
  (`barrel_friction`, 140 px/s each second, applied in `sync_loose`) so it
  comes to a stop rather than creeping forever, and restitution
  `barrel_bounce` (0.45, the larger side's, so a drum bounces off a wall or
  a hull though neither bounces off anything else). It looks a step ahead
  as a hull does, so a thrown drum stops at a wall's face. The wrecks'
  `settle_wreck` was the precedent: a body the solver moves, braked by
  damping.
- **Going off** (`Game::sync_loose`, after the step, before the rams): a
  drum whose velocity the step changed by `barrel_burst_speed` (300 px/s)
  or more is set off as a ram sets one off (the ram's column, the rammer's
  lurch). A hull at its own top speed (210) gives a drum about 1.1 times
  its speed, so it shoves; 300 is a boosted or flung hull, a drum thrown at
  a wall at about 210 or more, two drums thrown together. The knock is
  measured on the step alone - a blast's throw is an impulse between steps
  and is not a knock. A lit drum is left to its fuse. The old ram damage
  (`barrel_ram_damage_per_second`) is gone: a slow push only moves a drum.
- **Blasts** (`Game::blast` -> `throw_loose`): every loose drum within
  `barrel_throw_reach` of the radius is thrown straight away from the
  centre at `barrel_throw_factor` (2.5) times the blast's knockback on a
  hull, falling linearly to nothing at the reach. Inside the radius a
  drum is lit as before and thrown too.
- **Identity** (`Obstacle::home`, `key_cell`): a drum is known by the cell
  the map placed it in, wherever it has gone. `Obstacle::cell` is the cell
  it stands on now (the nearest, `map::world_to_cell`); what reads a drum's
  cell for a rule - a fire beside it, an oil trail's end, the dev server's
  `detonate` - reads where it stands.
- **Projectiles** (`hits.rs`): `Terrain::build` is rebuilt every frame from
  where every tile stands, so a shot meets a drum where it is; a drum takes
  its own box (`battlefield::tile_half_extent`) and closes no seam with a
  neighbour, here, in the colliders or in the nav grid. Lag compensation
  rewinds hulls and bongs only: a drum is shot where the room has it now -
  it moves only when something shoves it, and a shove is the room's.
- **Nav and AI** (`nav.rs`): a loose drum is no wall. The layer leaves it
  out (and the kept layer's tile list too, so a drum shoved about rebuilds
  nothing), so the router drives straight through a drum's cell and the
  hull shoves the drum out of the way; a shoved drum leaves no wall behind
  and blocks no cell where it stops. The kept grid stays the one a rebuild
  makes (`a_round_routed_on_the_kept_grid_plays_as_one_routed_from_scratch`).
  The map linter holds the planner only to the fixed colliders
  (`check_planner_physics`) and does not call the nook a drum was placed in
  a trap (`check_boxed_in`). Enemy spawn clearance still keeps off a drum's
  cell.
- **Contacts**: a hull pressing a drum touches neither terrain nor a tank
  (`ContactStats::touching_loose`): the probe's contact kinds, the parked
  hold and the slam effects leave it out. The turn fit and the lane assist
  ask only fixed bodies, so they never steer round a drum.
- **Drawing** (`pose.rs`, `tile_index.rs`): a drum keeps a `Smooth`
  (`Obstacle::smooth`), begun and posed with the tanks and bongs, and is
  drawn and shadowed from it (`Obstacle::drawn_at`). The painters' tile
  index sees a drum that moved and files the tiles again that frame
  (`TileIndex::matches`).
- **Online** (protocol 37): `Snapshot::loose` lists every drum off its own
  cell's centre, by home cell, at the wire's quarter pixel (`LooseState`,
  diffed in `delta.rs`); `apply_loose` stands the replica's drum - body and
  all, so a client's prediction sandbox meets it where the room's hull
  did - there, and one no longer listed back home. A drum's death is
  `Event::LooseGone { home }` beside its `ObstacleDestroyed` (which says
  where it went off): the replica takes the drum out by its home cell and
  the snapshot's tile entry is written under it too. A replica's drum is
  never stepped.
- **The builder and maps**: unchanged - a drum is a cell; only where it
  starts comes from the map.
- **Determinism**: no RNG on contact; the drums are synced, knocked and
  thrown in home-cell order; rapier is deterministic in the spawn order.

**Wrecks** (by the same request): a wreck is shovable. Its damping is
`wreck_linear_damping` 16 times its chassis's mass factor (30 before): an
assault hull shoves an assault's wreck at about 83 px/s, a scout's at 133 and
a titan's at 28, and let go it slides a few px and stops - cover you can
push into place. A blast throws a wreck as it throws a live hull, at
`wreck_blast_factor` (1) of the knock, but never hurts it and draws no roll;
a wreck's own death blast spares it.

## New props (BB-209, 2026-11-22)

Five props that use the hard and soft faces and the loose bodies, each a
`Material` (`obstacle::NEW_PROPS`, in sheet row order) placed as its own
cell, drawn from `props_sheet.png` rows 10-14 (docs/PROPS_SPEC.md), named in
the builder's WALL list (`tool-<name>` in `lang/en.ftl` and `lang/sl.ftl`) -
the things a hull runs into - since PROP's list is full at eleven rows.

| Prop | Map kind | Body | A hull meeting it | Shots | Knobs |
|---|---|---|---|---|---|
| Tyre stack | `tyres` | fixed | thrown back off it, harder than off iron (`Surface::Springy`, `tyres_bounce` 0.7) | pass over at 0.3; wear it to a slumped stack, then it goes | `tyres_max_health` 30 |
| Hay bale | `hay` | loose, twice a drum's weight (`hay_mass` 0.8), soft | shoves it; slides little (`hay_friction`) | pass over at 0.35; a blast or a flame lights it (it always burns, `Material::catches_fire`) and it burns out to ash | `hay_*` |
| Wooden crate | `wood_crate` | loose, a drum's weight | shoves it; a knock of `wood_crate_burst_speed` (260 px/s) splinters it, nothing goes off | pass over at 0.3; one or two shots splinter it | `wood_crate_*` |
| Concrete barrier | `barrier` | loose, six times a hull (`barrier_mass` 6), braked hard (`barrier_friction` 600) | a standard hull's held push creeps it at about 17 px/s; a light hull's does not move it | stops every shot and hides what is behind it, as concrete | `barrier_*` |
| Tank trap | `tank_trap` | fixed | the hardest stop and throw (`Surface::Hardest`) | pass between its beams at 0.6 | `tank_trap_max_health` 160 |

A blast throws the loose ones as it throws a drum, slower as they outweigh
one (`throw_loose`). The fixed two are walls to the router; the loose three
are not ("Loose barrels" above). What the AI sees over has a pass-over
chance and the barrier, which hides, has none (`Material::blocks_sight`).
Placed on the campaign where they fit: hay, crates, tyre stacks, barriers
and tank traps round Serpent River's two camps and its villages, a row of
tyres and two barriers in the Scrapyard, four lone tank traps across No
Man's Land.
