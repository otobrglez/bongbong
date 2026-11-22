# Physics

How bongbong uses **rapier2d**: what is a body, how a hull is driven, what
the solver decides and what it does not. The wrapper is `src/physics.rs`
(`Physics`); the code that drives it is `src/simulation/` (`drive_tank`,
`step_world`, `sync_tanks_and_ram`, `props.rs`).

## What rapier owns

Rapier is the single source of truth for **solid bodies touching**: a hull
against a hull, a tile, the bong, deep water or the field's edge. It
resolves the contact, splits a shove by mass and stops a hull at a face.
Everything else is the simulation's own:

| Thing | In rapier | Why |
|---|---|---|
| Tank (seat, enemy, wreck) | dynamic body, rotation locked | driven and knocked by impulses |
| Tank rolling in through a gate | none until it arrives | `rollin_phase` moves it by hand and spawns the body on arrival |
| Field boundary | 4 fixed cuboids | `battlefield::spawn_walls` |
| Obstacle tile (wall, prop, tree, tower, cone, door, lamp post) | 1 fixed cuboid each, with its surface ("Hard and soft") | removed when the tile dies |
| Deep water, deep lava | 1 fixed cuboid a cell | a hull cannot enter; a shot is never stopped by them |
| Bong | fixed cuboid, relocated | see "The bong" |
| Shell, bullet, plasma, missile, grenade, drone, orb, laser, rail | **none** | hand-integrated; hit by `simulation::hits` |
| Pickup, portal, fire, ooze, grass | none | overlap tests on positions |

Every body carries exactly one collider (`Physics::collider_of`). There are
no sensors, no collision groups and no joints. Gravity is zero: the game is
top-down. The one physics hook is the cushion of a soft face
(`SurfaceHooks`, "Hard and soft").

Projectiles have no body because a body can only say where a shot *ended
up*. `hits::Terrain::sweep` sweeps the whole segment a shot travelled this
step against a snapshot of the terrain and the hulls' boxes and takes the
nearest entry - the only projectile hit test in the game.

## The step

`Game::update` is given `PHYSICS_FIXED_DT` (1/60 s) and nothing else; a
debug build asserts it. One update is one step:

1. The phases before `step_world` write impulses onto bodies: each seat's
   and each enemy's drive (`drive_tank`), a shot's recoil, a knock.
2. `step_world` advances every projectile by the step, then calls
   `Physics::step` once. On the end screen the projectiles still fly and
   the physics does not step, so nothing drifts.
3. `sync_tanks_and_ram` reads each body's position back into
   `Tank::position`, lets the faces have their say (`respond_to_contacts`:
   the bounce, a crack, a hit's report - "Hard and soft"), lays tread marks
   along the travel and resolves rams.

The windowed game keeps real time and the step apart in `app.rs`'s
`StepClock`: it owes the round whole steps, runs at most
`SIM_MAX_STEPS_PER_FRAME` of them a frame and carries a frame with no step
on. The probe, the tests, the dev server's `step`, the room server and the
rig each call `update` once per step. That one fixed `dt` is why a seed
replays bit for bit at any refresh rate (docs/gameplay-verification-design.md).

After a step `Physics::quarantined` says whether rapier had to disable a
body or collider whose state went non-finite; `step_world` turns a non-zero
count into `Event::PhysicsQuarantine`, which the probe counts as an
`invariant` anomaly.

## Scale

Rapier's lengths are the world's pixels, and `IntegrationParameters::length_unit`
stays 1. Rapier scales its tolerances by it, tuned for metre-sized bodies, so
in a pixel world contacts are generated 0.02 px ahead and an overlap is pushed
out at 3 px/s. Two of those limits matter here, and `physics.rs` sets them
itself:

- **The speed cap** is `PHYSICS_MAX_SPEED` (lib.rs): one cell a step, 1920
  px/s, set as `normalized_max_linear_velocity`. It sits above every
  legitimate speed - a knock at the wire's 508 px/s on top of a hull's own, a
  light hull carried by a gravity well's current at full throttle (about 750)
  - so it never clips a knock the tuning asks for. `Physics::max_speed` reads
  it, and the pose validator's well allowance never passes it
  (docs/gravity-well.md "Online").
- **Every hull looks a step ahead** (`HULL_LOOK_AHEAD`, physics.rs). Rapier's
  soft continuous collision detection is set on the body at spawn, for its
  whole life, wreck or not. Each step it makes predictive contacts against
  whatever lies within the hull's own travel (never more than a cell), so the
  solver stops the hull at a collider's face, however it got there. Measured
  on a scout, an assault and a leviathan driven or knocked north into an iron
  row and the field's top edge, from gaps of 0 to 34 px:

  | Moved by | Without the look-ahead (deepest in) | With it |
  |---|---|---|
  | its own drive (210 px/s) | 2.5 px, out within a second | 0.28 px (scout), flush at rest |
  | a ram's or blast's knock (60 px/s cap) | 0.15-0.35 px | 0 |
  | a 300 px/s knock, no skid | 2.8-3.7 px, out over 1-1.2 s | 0 |
  | a 480 px/s knock, no skid | 4.7-6.2 px, out over 1.6-2.4 s | 0 |

  A predictive contact counts as touching, so a ram and the probe's contact
  kinds register on the step two hulls meet rather than the step after, when
  they would overlap.

Rejected: a larger `length_unit`. It raises the prediction distance with the
rest, but at these speeds not far enough: at 32 a 480 px/s knock still lands
4-5 px inside a wall. It also scales the sleep threshold, so at 100 a
leviathan fell asleep 0.7 px inside one. Also rejected: a 400 px/s cap with
the knocks clamped under it - the tuning table, the design docs and the wire
all mean knocks to go past 400.

## A tank's body

`Physics::spawn_tank` makes a dynamic body with rotation locked and one
collider:

- **Shape**: a round cuboid whose overall footprint is
  `Tank::move_half_extents` - the hull's damage box scaled down by
  `TANK_MOVE_BBOX_FRACTION` - with `TANK_MOVE_CORNER_RADIUS` corners
  (`physics::tank_corner_radius` clamps the radius for small hulls). The
  rounding is what lets a hull slide along a run of tile colliders instead
  of catching on the edge between two of them. The projectile hit test
  uses the bigger damage box (`Tank::hull_bbox_world`), never this one.
- **Mass**: `Tank::mass()`, set on the collider, so an impulse divided by
  mass is exactly the velocity change the caller meant and a ram splits by
  weight with no formula of its own.
- **Orientation**: the body never rotates. A hull that is not square has its
  collider reshaped (`Physics::resize_collider`) whenever its facing swaps
  between the x and the y axis; `drive_tank` does it on the turn, standing
  the hull where its new shape fits first (`fit_turn`, "The turn" below),
  and `Game::place_tank` on a jump.
- **Friction and damping**: rapier's defaults (friction 0.5, no damping)
  while it lives. The one place either changes is `Physics::settle_wreck`,
  on the frame a tank becomes a wreck: `wreck_linear_damping` times the
  chassis's mass factor, and `wreck_friction` ("Wrecks" below).

### Driving

Movement is four-directional with momentum. `Tank::control` only records
the command: the facing and `Tank::velocity`, the *commanded* cardinal
vector at the hull's damage-scaled top speed. `drive_tank` is the whole
locomotion model, one impulse a step on the body's real velocity
(`Physics::velocity`):

- **Along the tracks** it closes the gap to the commanded speed. Speeding
  up, the change is capped by `tank_accel_force` over the hull's mass.
  Slowing or reversing, it closes a fraction of the gap each step
  (`tank_decel_curve_rate`, an exponential) and snaps the last
  `tank_decel_snap_px`, so letting go stops a hull in a short, heavy slide.
- **Across the tracks** it kills the sideways slide, capped by
  `tank_turn_grip_force` over mass: a turn at speed drifts a little.
- **Weight**: the drive's force is multiplied by the chassis's mass factor
  to the power `tank_drive_mass_exponent`, the brake's rate by the factor to
  `tank_brake_mass_exponent` (both 0.5; both exactly 1 on the standard
  chassis and at 0). A heavy hull still gathers speed and stops slower
  than a light one - the force grows slower than the mass - but it pushes
  harder: nose to nose the heavy one wins, and it shoves a light hull out
  of its way.
- **A parked hull holds its ground** (`tank_park_hold_force`): with no
  drive commanded and another hull against it
  (`ContactStats::touching_tank`), its tracks take that much more force
  off its motion, times its mass factor, along them on top of the brake
  and across them on top of the grip. A hull of its own weight then creeps
  it a step's shove at a time (about 18 px/s), a lighter one hardly moves
  it and a heavier one shoves it aside; a hull nothing touches stops and
  slides exactly as without it. At 0 any hull pushes any parked hull
  sideways at its own top speed.
- **Footing** scales all of it (`Footing`): a ford's pace, drive and grip
  and its current, ice, rubble, ooze, and a gravity well's pull - the model
  runs in the frame of whatever carries the hull.
- **A skid** (`Tank::skid`, a sonic blast or a hard knock) turns the drive
  off: the hull only loses speed to friction until it is nearly still.
- An enemy's drive is deferred to the end of `enemy_phase`
  (`drive_tank_with`), with the velocity and facing captured before it
  fired, so the command layer sees every intent before any hull moves.

A seat whose client owns its hull online is not driven here at all: the
room accepts the pose the client sends, inside limits
(`Game::accept_seat_pose`, docs/online-coop-prd.md section 4.14).

### The turn

A hull is longer than it is wide, and a turn swaps the two under it. Three
things make a turn land where the player meant it. All three read the hull,
its body's velocity and the *fixed* bodies around it (tiles, the field's
edge, deep water, the bong; `Physics::hull_fits`, `hull_path_clear`) and
keep no state, so a client's prediction replays them from the same pose;
other hulls are never asked. Seats and enemies go through the same code.

- **The fit** (`fit_turn`, `tank_turn_fit_px` 10): before the collider is
  reshaped, where the new shape would reach into a fixed body the hull is
  moved to the nearest spot it fits, velocity kept - along its new length
  first (ahead, then back), then along the heading it turned from as well
  (back before on), a px at a time out to the knob. Where nothing in reach
  fits it turns where it stands and the solver pushes it out, as it does
  for every turn at 0. The move is at most the knob in each axis, inside
  what `accept_seat_pose` lets a client's hull move in a tick.
- **The buffer** (`buffer_turn`, `turn_buffer_px` 16): a turn across the
  heading pressed just short of an opening is held - the hull told to drive
  on - while the new heading is walled off within `lane_look_px` both where
  the hull stands and where its slide would end, and is clear from some
  point within the buffer's distance ahead. It runs before `Tank::control`:
  the seats' in `drive_tank`, the enemies' in `enemy_phase`'s collect pass.
  A stopped hull, one in a skid or a spin, and one in a well's pull turn at
  once.
- **The assist** (`lane_guide`, `lane_assist_px` 12): a driving hull whose
  way ahead is walled off within `lane_look_px`, with the way clear a
  little to one side, is slid across to it by its tracks - at up to
  `lane_assist_grip` times their grip, no faster than the hull drives and
  no faster than that grip can stop in what is left to go. It looks from
  where the hull's sideways slide would end, so a hull that will come to
  rest in the clear is left to; the side against the slide is tried first
  and reaches further, by what the guided grip needs to stop the slide.
  That is what brings a turn pressed late back to its lane, and what
  guides a hull a few px off a lane's centre into it instead of stopping
  it on the lane's corner.

An enemy times its own turns one slide short of a lane's centre
(`Ai::lane_turn`), so its way is clear where its slide ends and the buffer
and the assist leave it alone; they act on it where it would have caught a
corner. Each costs a driving hull a handful of shape casts a step, and only
while its way is walled off.

### Wrecks

A wreck is no longer driven: what holds it is the damping
`Physics::settle_wreck` puts on its body, `wreck_linear_damping` (30) times
its chassis's mass factor. Damping takes a share of the hulk's speed off
every second, so the speed a pusher's drive holds it at falls with the
hulk's weight twice over: an assault hull shoves a scout's wreck at about
70 px/s, its like's at about 50 and a titan's at about 20, where at a
damping of 4 any of them went at the pusher's own pace. A blast still
throws a wreck (`knock::Knock`), a few px.

### Knocks

Everything that shoves a hull is `Physics::apply_impulse`: a ram, a blast's
knockback, a shot's push, recoil. Nothing stores a knock - the body's
velocity carries it and the drive above wears it off, which is the
"momentum" in the model. Rams read rapier's own contact state
(`Physics::touching`, an active contact pair) rather than a geometric
test, once per pair a step in a fixed order: each enemy against each seat,
the seats against each other, then enemy pairs by owner slot.

### Jumps

`Game::place_tank` is the one way a hull is put somewhere: the position
goes on the tank and the body (`Physics::set_position`, which also zeroes
the velocity), the skid and tread state are cleared, and the collider is
reshaped for the facing. A portal and the dev server's `teleport` both go
through it, before `step_world`, so the jump lays no tread marks.

## Static bodies

`Physics::spawn_static` makes a fixed cuboid with rapier's default surface.
Per-material friction was tried twice and measured worse (more `wall-grind`
and `spin` in the probe): the rounded hull exists to slide past tiles, and
friction fights exactly that. How hard a tile is acts on the contact's
normal instead ("Hard and soft" below).

### Hard and soft

Every fixed collider carries a surface (`obstacle::Surface`, set with
`Physics::set_surface` and kept in the collider's `user_data`), which
`Material::surface` names for every tile:

| Surface | Tiles | Head on, a hull |
|---|---|---|
| Hardest | the towers, a volcano's cone | stops flush, thrown back hardest (`hardest_bounce` 0.5), the strongest jolt |
| Hard | iron, concrete, brick, a training door, the field's edge | stops flush, thrown back (`hard_bounce` 0.38) |
| Brittle | glass | as hard, but a hit at `glass_crack_speed` (170 px/s) cracks the pane (`glass_ram_damage`, half its health) instead |
| Firm | a timber wall, broadleaf, pine, spruce, Scots pine, fir, palm, a drum | slows over the last ~2 px (`firm_cushion_decel`) |
| Yielding | birch, willow, snag | slows over the last ~4 px (`yielding_cushion_decel`) |
| Soft | sandbags, a fence, a lamp post, a range board | slows over the last ~10 px (`soft_cushion_decel`) |
| Plain | deep water, lava, the bong | the solver's own stop |

**The cushion is the solver's.** While a hull is within its look-ahead of
a collider (`HULL_LOOK_AHEAD`), their contact is a speculative one: the
solver lets the hull close the gap `d` this step and no more. A contact
hook (`physics::SurfaceHooks`, rapier's `modify_solver_contacts`, asked only
by the colliders that cushion) shortens that, for a hull coming at the face
head on, to what `v^2 = 2 a d` allows at `d`. The nearer the hull is the
slower it may come, so it settles flush against the face over the last few
px - 3 to 5 steps against sandbags at full speed - instead of stopping on
it from full speed. Head on means a normal within `CUSHION_AXIS_COS` of an
axis and at least `CUSHION_HEAD_ON` of the hull's velocity going into it:
a hull sliding along a face, or driving past a corner, is not slowed. It
stays a stop at the face, never a sink into the tile: a hull pressed into
a run of tiles would meet the next tile's side face at the seam and snag,
which is the failure the rounded hull exists to avoid. Rapier never
recycles the contacts of a hooked pair, so the cushion is recomputed every
step, a pure function of the gap and the hull's velocity.

**The bounce is a knock** (`simulation::contact`, after the step, before the
rams). A hull whose body went into the step at `hull_bounce_min_speed` (130
px/s) or more along its contact with a hard face is thrown back along the
normal at that share of the speed, with a skid of
`hull_bounce_skid_seconds` so the drive does not take the throw back in a
frame (`combat::knock`, so a client-owned seat is told through
`Event::Shoved`). 130 px/s is over a driven hull's own push into a face it
rests on - one step of `tank_accel_force` is 70 to 90 px/s - so leaning on
a wall never bounces; after a throw `hull_bounce_rest_seconds` passes
before the same hull is thrown or reports a hit again, since it comes back
at the face under its own drive. Rapier's restitution is not used: it
cannot tell a hit from a hull leaning on the face at the drive's push.

**A hit is reported** at `hull_struck_min_speed` (120 px/s):
`Event::HullStruck` carries the slot, the tile's material (none for the
field's edge), the surface, the contact, the way in, the speed and whether
it bounced. `fx.rs` draws it by surface - a hard face slams (sparks, dust,
the hull's flash and a jolt), the rest throw their own chips and dust
(`tile_chip`: a timber wall's splinters, a sandbag's sand, a tree's leaves)
with a smaller jolt (`hardest_jolt` .. `soft_jolt`); a tree's crown shakes
(`contact::TreeShake`, `tree_shake_px`, the round's presentation, set on a
replica from the event).

Around the faces, by the same request: a lamp post snaps
(`lamp_ram_seconds`) and a range board tips over (`target_ram_seconds`) under
a ram, as a fence does; a training door stays hard. Tall grass and reeds keep
`grass_pace` (0.95) of a hull's pace and a bush `bush_pace` (0.88)
(`Footing::in_brush`), on top of the sway and the flattening they always
had.

**Measured** (2026-11-22, BB-205): with the brush's pace at 1, the faces
move no round of the nine fixtures, the seven field maps or the pinned
default-map stream. The probe's default-map sweep over four seed bases
(120 rounds) reads wall-grind 0 before and after, spin 7 -> 6, stall 4 -> 3,
bump-rate 3 -> 2; the fixtures read exactly as before; the field maps'
total fell from 204 to 199, hedge-maze's jitter rising with its bushes
(justfile, `field_ceilings`).

### Tiles and their seams

A tile's own box is `OBSTACLE_HULL_FRACTION` of its cell: 24 px in a 32 px
cell. Two neighbours would leave an 8 px seam a shell could thread, so a
tile of a *structure* is widened to the full half cell on each axis where
it has a neighbour (`battlefield::tile_half_extent`). Trees and lamp posts
neither widen nor count as a neighbour: a grove is trunks with gaps.

The same rule sizes a tile in three places. They agree at every moment of
a round, but for one neighbour: the hit test and the nav grid count a lamp
post as one, the collider does not.

- the collider, set when the tile is spawned and refitted when it changes;
- the projectile hit test, rebuilt every frame (`hits::Terrain::build`);
- the nav grid (`simulation::nav`).

A collider changes twice in a tile's life:

- **A chunk breaks** (`chunks.rs`): `Game::fit_tile_body` cuts the box down
  to the quadrants still standing.
- **A neighbour goes**: every tile death goes through
  `Game::obstacle_died`, which refits the four tiles beside the dead one
  (`refit_neighbours`); a training door opening, a drum a gravity well
  lifts and a replica taking a tile out call it too. A tile that just lost its only neighbour on an axis
  goes back to its own 24 px there, so the breach is as wide for a hull as
  it is for a shot and for the router, and a hull driving through does not
  catch on a box that still reaches for a wall that is gone.

The dead tile's own body is removed at the end of the frame
(`cleanup_done`).

### The bong

A bong comes to rest clear of every hull, standing or wrecked: a hop's
landing (`Terrain::bong_fits`) and a gravity well's slide
(`Terrain::bong_slides`) both test the hulls' movement boxes, grown by
`bong_hull_clearance_px`, as they stood at the start of the step. A hull
that drives under a hop already in the air is not seen.

A bong is a fixed body relocated with `set_position` every frame, not a
kinematic one. A bong *hops*: a kinematic body asked to be far away by the
next step drags whatever is between along with it, and measured on
`maps/test/bong-block.toml` it shoved tanks across the map. For something
that teleports, "fixed, relocated" is the faithful model.

## What the AI sees

The AI never queries rapier. It plans on the nav grid and on a per-frame
snapshot of movers (`Game::motion_snapshot`), and it detects being stuck
from displacement along its commanded heading, never from a body's
velocity. Rapier executes; it is not asked for advice.

The probe does read rapier, through `Physics::contact_stats`: what a hull
is pressing against after the step (a fixed body, another tank) and how
hard. Those are the `wall-grind`, `bump-rate`, `tank-grind` and `pile-up`
anomaly kinds.

## Determinism

- Bodies are spawned in a fixed order - walls, tiles row by row, deep
  cells row by row, seats, enemies by slot - so a seed hands rapier the
  same handles on every run. A wave tank's body is spawned on arrival, in
  slot order within a frame.
- Rapier's `parallel` feature is off: the web build is single-threaded and
  the solver's order must not depend on a thread pool.
- The round's math around the solver is portable (`math::sin` and friends,
  `libm`), and rapier itself runs the same float operations on every
  platform, so a Mac, a Linux runner and wasm agree on a seed.
