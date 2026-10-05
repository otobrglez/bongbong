# PRD: Teleportation

- Tanks should be able to teleport between locations.
- There must be at least 2 "portals" placed on map for this to work.
- If there is only one placed it should not be rendered.
- In map editor the portal should be in inventory so that user can place it on the map
- The portal should be 15%+ bigger than biggest tank
- The portal should be sci-fi blue spiral that turns slowly (like black hole)
- If there is more than one portal placed on the map, tank gets teleported randomly to one location.
- All tanks should be able to teleprot to it.
- When tank teleport to so location it should be placed in one of the free available grid spots.

# Design

The lines above are the brief; what follows is how the game does it.

## Map object

A portal is one map cell, `cells."c,r" = { kind = "portal" }` (`CellObject::Portal`,
`MapFile::portal_cells()` in `iter_cells` order). It is the **anchor**: the art is ~3x3 cells
centred on it and spills over its neighbours, which stay paintable. The cell is not solid and
not an `Obstacle` - the same reasoning as tall grass: anything spawned as an obstacle is a wall
to the planner and the linter. `battlefield::spawn_from_map` collects the anchors into
`MapSpawn::portal_cells`; `Game::init` turns them into `Game::portals` (world positions).

A network is **active** with two or more portals (`Game::portals_active`). A lone portal is
kept in `Game::portals` so tooling can list it, but nothing teleports, the round draws nothing,
and the nav grid is exactly the plain grid - a portal-free and a one-portal map replay
byte-identically.

## Trigger and arrival (`Game::portal_phase`)

The phase runs after `pickup_phase` and before `step_world`, so the body and `tank.position`
already agree when `sync_tanks_and_ram` measures travel and `lay_tracks` sees no jump.

- **Trigger**: a live, non-wreck tank with a body whose centre is within
  `portal_trigger_radius` (40 px) of an anchor, with `Tank::portal_cooldown` at zero.
- **Exit**: uniformly random among the *other* portals that have room, one round-RNG draw per
  teleport. Walk order is players in index order, then enemies by slot, so a round where nobody
  hops draws nothing.
- **Arrival cell**: `Grid::nearest_open_reachable` from the exit - a BFS through open cells
  only, at most `portal_arrival_max_cells` steps - for the first usable cell outside the exit's
  trigger radius and at least two tank half-extents plus a quarter cell from every other tank
  (wrecks included; frogs and fires are already blocked cells). Two tanks entering on one frame
  never share a cell. No candidate at any exit: nothing happens, no RNG is drawn.
- **Placement**: `Game::place_tank`, the same path the dev server's `teleport` uses - position,
  ring snap, commanded and physics velocity zeroed, movement collider re-oriented. Heading is
  kept. The tank's `portal_cooldown` is set to `portal_cooldown_seconds` (1.5 s) and ticked in
  `tick_timers` **only while the tank is outside every portal's trigger radius**: the arrival
  cell sits just outside the exit's radius, and a tank that stops there to fight and drifts onto
  the exit must not bounce back the moment the timer ends - it has to leave and come back.
- **AI on cooldown**: while `portal_cooldown` runs the AI routes on foot only
  (`Grid::next_step_walking`, `AvoidCtx::on_portal_cooldown`), and wanders where no walking
  route exists. A route through the hub would walk it straight back onto the portal it came
  out of, where the cooldown stands still, and it would circle the footprint until it left
  and came back - the orbit `maps/test/portals.toml` used to show as a `spin`. Once the
  cooldown is out it plans through the hub again and drives at the portal's centre.
- **AI on arrival**: `Ai::on_teleported` drops the heading commitment, the stuck clock's
  baseline, the waypoint, any breach and the dodge/yield timers (all measured at the old
  position); alertness, retreat state, fire timer, escape count and target player stay. Every
  `EngageRing` releases the tank's sticky slot (`EngageRing::release`) - with merged components
  its old slot is still "reachable", and it would otherwise route straight back through.
- **Events**: `Event::Teleported { slot, x, y, to_x, to_y }` and two `SHOCK_TELEPORT` ripples
  (from and to). No screen flash.

## Shots (`simulation/portals.rs`)

Shells, bullets, plasma bolts and laser beams go through the network too, while `portal_shots`
is on (a tower's bullets included). Missiles do not: they fly over everything and only their
burst touches the ground, so a portal under one is open ground.

- **Going in**: a shot whose path this frame passes within `portal_shot_radius` (28 px, under a
  tank's 40: a shot is a point and should have to hit the swirl, not graze the rim) of an
  anchor goes in at the point of its path nearest the anchor (`portals::shot_entry`). It is
  judged only up to that point (`Game::resolve_projectiles` sweeps `prev..entry`), so whatever
  stands before it stops it and nothing beyond it on that side does.
- **Coming out**: at the same offset from another portal's anchor, a hair (`EXIT_CLEARANCE_PX`)
  along its heading, with heading, speed, owner, damage, lag-compensation rewind and pass-over
  list kept (`portals::exit_point`, `Game::shot_through`). The exit is one round-RNG draw,
  uniform among the other portals, made only where a shot goes in - a round in which no shot
  meets a portal draws exactly what it would on a map with none. No room check: a shot may
  come out into a wall beside the exit, and bursts on it.
- **Leaving a portal**: a shot inside a swirl it came out of, or one fired by a tank standing
  on a portal, is *leaving* that portal (`ShotPortals::leaving`, a component attached only
  then) and cannot go into it until it has left the swirl - the shot's portal cooldown. A shot
  that came out also heads away from its exit's anchor, so the nearest-point rule would not
  take it back anyway.
- **Passes**: `portal_shot_max_passes` (4) per shot or beam; past it the portals let it fly
  over. Two portals lined up on a heading would otherwise hand a shot back and forth for ever.
- **The laser** is bent leg by leg (`Game::resolve_lasers`): each leg is swept up to the next
  portal it reaches, then carries on from the exit on the same heading for the reach it has
  left, so hits are judged along the bent path. Each leg is drawn as its own `LaserBeam` and
  logged as its own `Event::LaserBeam` (`leg` 0 from the lens, `portal` when the leg ends going
  in), each pass between two as a `ShotTeleported` with no id.
- **Events**: `Event::ShotTeleported { id, x, y, to_x, to_y }` (`id` the projectile's, `None`
  for a beam): a small blue spark flare at each end in `fx.rs`, no ripple and no shake - a
  minigun burst can send a dozen through in a second.
- **The AI** does not know about any of it: an enemy lined up on a seat through a portal fires
  as it would through open ground, and its shot comes out somewhere else.

**Online.** `WireEvent::ShotTeleported` and `LaserBeam`'s `leg`/`portal` travel (protocol
13). The interpolator draws a shot whose far snapshot carries its `ShotTeleported` as a jump -
held at the near end, drawn at the exit the frame render time reaches it, its correction offset
dropped - exactly as a hull's `Teleported`; a replica flashes a beam's lens on its first leg
alone. What is drawn in the present cannot know the room's draw, so it stops at the portal:

- *This seat's provisional shots* (`net::predict`) are swept against the portals too
  (`PresentWorld::portal_entry`, the same rule, the `leaving` portal included). One that goes
  in leaves the picture at the point it went in, with no impact; its room copy stays hidden
  until the interpolator hands over that copy's own `ShotTeleported`
  (`Predictor::shot_teleported`), and is then the shot, drawn from the exit on the room's
  timeline. A copy that bursts short of the portal, or flies on past it, is shown at once.
  The compromise: between the provisional going in and the room's copy coming out the shot is
  in neither picture, for about a round trip plus the picture's delay.
- *A beam drawn on the press* stops at the first portal on its line; the legs past it are the
  room's `LaserBeam`s (the seat's first, claimed, is the one it drew), a round trip later.
- *Incoming fire carried into the present* (`net::round`) is taken off the picture for the
  frames its carried path runs into a portal, and starts afresh from the exit, eased up to the
  lead as from a muzzle, on the frame its `ShotTeleported` is handed over. A foreign shot fired
  inside a swirl is not known to be leaving it there, so it may blink out for those frames.


## AI routing: the optimistic hub

`Grid::with_portals(centres, radius, hop_cost)` (called from `Game::nav_grid`, so the AI,
engage reachability, maplint and the probe all see it) adds one virtual **hub** node to A*.
A portal's nav footprint is every open grid cell whose centre lies within the trigger radius
of the anchor - the anchor sits on a nav-grid corner (map cells are multiples of 32, nav
centres at half cells), so that is four cells at the defaults. Footprint cell -> hub costs
`portal_hop_cost` (in cells, clamped to at least the footprint's span so the hub never beats
walking between two cells of one portal); hub -> any footprint cell costs 0. Path
reconstruction skips the hub. `next_step` hands a step onto any footprint cell out as that
portal's *centre* - the trigger point - so a hull steering at it lands inside the radius
whatever its momentum overshoots (aimed at a footprint cell's centre beside the anchor, a tank
could round the 2x2 footprint a few pixels outside the radius, pass after pass); from an
entrance cell it names the exit portal's centre, which the tank never reaches because the
trigger fires first. The heuristic stays admissible
(`min(manhattan, nearest portal + hop + nearest exit to goal)`). With exactly two portals the
plan is exact; with three or more it is *optimistic* - the planner assumes the best exit, the
tank lands wherever the draw says and re-plans from there. `Grid::components` unions the
footprints' components, so a room joined to the field only by a portal counts as reachable.

The flow fields (`Grid::add_field`, one per player and for the frog - see `pathfind.rs`'s
module doc) walk the same hub: the Dijkstra outward from the goal, on popping a portal cell,
relaxes every other portal cell at that cost plus the hop (the exit's own price is not
charged, as in A*), and `descend` offers the exits beside the four neighbours from a portal
cell. A target served by a field is therefore reached through a portal exactly when A*
would go through one; `fields_route_through_portals_like_the_search` pins the two agreeing.

## Presentation (`portal.rs`)

`static/portal_sheet.png` (1152 x 288) from `tools/spritegen/gen_portal.py`: twenty-four
96 px frames of a three-arm log spiral (120 degrees periodic, so twenty-four 5 degree steps
are one full visual period), row-major in rows of twelve (`PORTAL_SHEET_COLS`; a single row
would pass the 2048 px width GL ES 2 phones can refuse), and a 32 px builder icon in the
top-left of the cell after the last frame (`PORTAL_ICON_CELL`, col 0 of the third row). The
frames turn the arms against their winding, so the spiral reads as pulling inward. The disc
is about 84 px across on the 96 px cell. Designed at 48 px and doubled, so it sits on the
2 px block grid like the props. Colours are only
`BLACK`, `WHITE` and the four `TEAM_P1` blues - a deliberate off-palette exception, admitted
by `check_sheets.py` for this sheet (docs/PALETTE.md).

The round paints portals at the tail of `Game::paint_floor` - over tracks, scorches and oil,
under everything that stands - so `mapshot` thumbnails inherit them (`portals` is pinned in
`thumbnail.rs`). The frame is `portal_frame(center, time)`: `Game::time` over
`portal_spin_seconds`, phase-shifted by the position hash so two portals never turn in
lockstep; never rotated at draw time. The additive glow block adds `draw_portal_glow` (two
`pixel_disc`s scaled by `portal_glow_strength`), round only. `fx.rs` turns a `Teleported`
event into a blue spark flare at the entrance and a softer flare plus settling embers at the
arrival.

The builder draws every anchor in a pre-pass right after the ground (a portal in the row-sorted
cell loop would cover a wall placed above it), turning on `rl.get_time()`; with fewer than two
placed it ghosts them and outlines the anchor in the gate colour. `Tool::Portal` sits in the
GROUND category after the gate, multi-instance, toggle-erase like every other brush.

## Linter

`portal-alone` (warning): exactly one portal. `portal-blocked` (error): an anchor with no open
nav cell within the trigger radius, or - in an active network - none in the playfield. The
linter's own flood fills (`Cells::neighbours`) step along `Grid::portal_links`, the same
footprint rule the planner routes by, so a portal-only room is playfield and not a
`disconnected-region`, and a frog or pickup there is reachable.

## Maps

- `maps/test/portals.toml`: two rooms split by a full iron column, one portal each, the player
  and frog on one side, three waves rolling in on the other. Every enemy must route through the
  hub; `just probe-fixtures` sweeps it at the shared budgets.
- `maps/portals.toml` (shipped as `portals`): three portals in a triangle around a walled
  centre; the corner ones are shortcuts, the bottom one an escape.

## Tooling

`snapshot` lists `portals`/`portals_active` and each tank's `portal_cooldown` (also settable
through `set_tank`); `terrain` carries the same list; `events` filters `teleported` and
`shot_teleported`. The probe restarts a tank's trail and spin chain on the frame it hops, so a
jump never reads as churn. `portal_shots` off in a `--tuning` patch flies every shot over
the portals, for comparing a sweep.
