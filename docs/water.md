# Water: what rivers and lakes do to the game

`docs/GROUND_SPEC.md` §9 is the picture: how a painted `water` cell becomes
a river or a lake, the animation and the current's marks. This document is
the rules - what that water does to a hull, a shot, a fire, a frog and the
AI's routing - and where each rule lives.

## One reading, two consumers

`ground::Layout` reads a map's cells once and both sides read it:
`ground::build` for the tiles, `ground::WaterLayout` for the rules. The
rules see three depths (`ground::Depth`):

| Depth | Where | What it is |
|---|---|---|
| `Dry` | anything not painted water, and a road cell painted over water | plain ground |
| `Shallow` | a stream cell, a lake's shore cells, a river mouth's banks | a **ford** |
| `Deep` | a lake cell whose four corners are all wet (`WATER_SHORE[0b1111]`) | **open water** |

So the deep water is exactly the flat open water on screen, and the shore
you can see is exactly the ford you can drive. `WaterLayout` is built at the
top of `Game::init` from the map's cells alone (no RNG, no world), before
anything is placed, and lives on `Game::water`; `hits::Terrain` carries a
copy for the frog. A map without water leaves every rule a no-op, and no
rule draws RNG, so every existing seed replays unchanged.

## Deep water: a wall to hulls, nothing to shots

- **Colliders.** Every deep cell gets a static physics box in `init`,
  seam-closed against its deep neighbours through
  `battlefield::tile_hull_half_extent` like a run of wall tiles, so a hull
  slides along a shore without catching. Nothing is spawned in `world`, so
  `hits::Terrain` never sees them: **shells, bullets, plasma and the laser
  cross a lake** and a tank on the far bank is still in the fight. Physics
  is solid bodies only (docs/physics-engine-design.md), so this is the one
  way to make ground impassable.
- **The nav grid** (`Game::nav_grid`) chains the deep cells in as obstacles
  at a cell's half-extent, so the AI routes round a lake exactly as round a
  wall, `blocked_ahead` steers off its edge, and `maplint`'s breach grid
  counts it as permanent. Enemy clearance rolls, frog and bonus placement
  all treat deep cells as terrain (`obstacle_positions`), so nothing spawns
  in a lake.
- **The player's start.** A cell holds one object, so a `start` cell can
  never itself be water. The centre fallback of a map without one can land
  in a lake; `dry_cell_near` moves it to the nearest shore.

## Fords: slow, loose, and downstream

`Footing` (simulation/mod.rs) is what the ground under a hull says to
`drive_tank_with` this frame; dry ground says nothing.

- **Pace.** In shallow water the commanded top speed and `tank_accel_force`
  are both scaled by `water_speed_factor` (0.55): a hull entering at speed
  is pulled down to wading pace by the ordinary deceleration curve, and it
  climbs back out slowly.
- **Grip.** `tank_turn_grip_force` is scaled by `water_grip_factor` (0.5),
  so a turn in a ford sloshes wide and momentum carries the hull.
- **The current.** A stream cell joined north or south moves at
  `water_current_speed` (28 px/s) down the map, and the whole locomotion
  model runs *in the water's frame*: `current` is taken relative to the
  flow, so a hull that stops drifts south at the water's speed, one driving
  upstream nets the difference, and one crossing has to aim upstream.
  Impulses are deltas, so nothing else in the model changes. Sideways
  segments, bends and lake shores are still water with no current; the
  marks in the picture draw where the push is.
- **A speed-up ends** the moment its hull wades in.
- **The AI's router prices a ford.** `pathfind::Grid::weigh` makes a step
  into a shallow cell cost `water_ford_path_cost` (3) dry steps. Occupancy
  is untouched - a ford is open to `usable`, `blocked_ahead` and the flood
  fills - so this changes which route is chosen, never whether one exists:
  an enemy skirts a river when the dry way round is shorter than the
  crossing's price and wades when it is not. The Manhattan heuristic stays
  admissible because no step costs less than 1.

## Fire

- Ground cells that are water take no heat under the flamethrower
  (`flame.rs`) and refuse to light (`props::light_cell`): no pool forms on a
  lake, and an oil trail's fire stops at the water's edge.
- A hull with afterburn on it is put out the frame it wades in
  (`flame.rs`'s afterburn tick).
- A blast or a death on water leaves no scorch; `fx.rs` throws spray
  instead.

## Frogs love water

`combat::frog_hop_target` runs its ladder twice: first taking only a
landing in water (any depth), then any landing. One jitter draw either
way, so a map without water hops exactly as before. A frog in deep water is
out of every hull's reach - a sanctuary beside the player's frog in
Protect, a fortress to shell from the bank in Hunt - but still in every
shot's reach. It does not heal there; healing stays with the frog health
pack (docs/frog-health-pack-prd.md).

## Presentation

- **Spray** (`fx::ParticleKind::Spray`): a splash the frame a hull wades in,
  droplets at `water_spray_rate` scaled by its speed while it moves, and a
  splash for a blast or a death on water. Droplets arc up and are gone the
  moment they land - water does not bounce.
- **Tread marks** stop in water (`lay_tracks`): the treads still turn, but
  nothing is pressed into a river bed. For `water_wet_track_seconds` after
  wading out a hull lays *wet* marks (`Track::wet`): `water_wet_track_darken`
  times darker, fading over that same time.

## Knobs

All in `tuning.rs`'s ground group: `water_speed_factor`,
`water_grip_factor`, `water_current_speed`, `water_ford_path_cost`
(restart), `water_wet_track_seconds`, `water_wet_track_darken`,
`water_spray_rate`; the picture's `water_frame_seconds`,
`water_flow_speed`, `water_flow_lanes`.

## Not done, on purpose

Sideways stream segments carry no current (their direction is ambiguous),
there is no bridge tile (a road cell over water is dry ground and stays
in the road autotile), oil does not float, wrecks do not sink, and no
chassis is amphibious. Each is a separate decision.
