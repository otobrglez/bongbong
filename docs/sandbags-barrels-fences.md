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
  `sandbag_ram_seconds` (0.4). Don't block line of sight.
- **Barrels**: `barrel_max_health` 18; a lethal hit or a ram
  (`barrel_ram_damage_per_second` 40) detonates it at once. The blast
  (`barrel_blast_radius` 96, `barrel_blast_damage_min/max` 15/30 with
  linear falloff, `barrel_blast_knockback_speed` 140) hurts everyone — both
  sides, frogs, walls and props — and puts every barrel in range on a
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
- **Fences**: a hit on a pristine fence destroys it at
  `fence_one_shot_chance` (0.7) odds, else it goes to its damaged keyframe
  and the next hit finishes it; rammed through in `fence_ram_seconds`
  (0.15). Don't block line of sight.
- Events: `obstacle_destroyed { material, x, y }` for any tile death,
  `blast { x, y, chained, drum }` for a detonation, `drum_launched { x, y,
  to_x, to_y }` for a fuel drum leaving the ground, and `fire_started { x,
  y, pool }` for a ground cell catching.
