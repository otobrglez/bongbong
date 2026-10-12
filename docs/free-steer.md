# Free steering (BB-128, BB-129, BB-130)

Status: a play-test build behind the `free-steer` cargo feature. The shipped
game is unchanged: without the feature every round steers four ways, byte for
byte.

## Why

Three play-testers asked, separately, for the tanks to move freely rather than
only up, down, left and right. This build lets that be tried against the
shipped feel without committing the game to it.

## How to play it

- Native: `just run-free` (the dev-tools build plus `free-steer`, so the dev
  server and its tools work as with `just run-dev`).
- Web: `just serve-web-free` builds the dev-tools page with the feature and
  previews it at http://localhost:4321, with the tuning panel.
- Keys: the arrows (and WASD for the couch's second seat) give eight ways, the
  keys held summed, a pair of opposites cancelling (`app.rs` `key_heading`).
  Touch: the stick's own angle, any way at all, past the same dead zone
  (`touch::resolve_heading`).
- The probe plays a free round with `--free-steer`.

## The model

`tank::Steer` says how a hull steers. `Game::steer` is a pre-init setting like
`Game::players`, kept across restarts. The window sets it to `Steer::BUILD` on
its local round: `Free` under the feature, `Cardinal` without it. Every tank of
the round takes it when it is made (`Tank::steer`), seat or enemy, and a seat
keeps it through a wave's roll-in and a training respawn.

The input fills both `Intent::move_dir` (four ways, as before) and
`Intent::heading` (degrees, 0 up, clockwise), so the input never needs to know
how the round steers. A `Cardinal` hull never reads `heading`; a `Free` hull
reads `heading`, or `move_dir`'s angle when there is none (the dev server's
`move_dir`, a script, an AI sidestep). `Intent::aim` is an angle to turn to
without driving, ahead of `face`.

`Tank::steer_free` is a `Free` hull's `Tank::control`:

- **Turn.** The facing (`Tank::rotation`, aim and drive with it) turns toward
  the heading the short way round, at most `tank_free_turn_speed_deg` (540
  degrees a second) a step, and lands on it exactly once it is within a step.
  The sprite's eased angles follow as they always have.
- **Drive.** The commanded velocity is the hull's top speed along where it
  faces, times `tank::free_throttle` of the turn still left. That is the cosine
  of the turn, stretched to reach nothing at `tank_free_pivot_deg` (90), and
  nothing past it. A nudge arcs at speed. A turn wider than the pivot angle (a
  reversal) pivots in place before it drives.
- **Momentum.** `drive_tank_with` runs the same model it always has (the
  accel force along the tracks, the decel curve, the grip across them, fords,
  ooze, wells, skids), in the hull's own frame (`Tracks`): the world's axes for
  a hull facing one of the four ways, its unit facing and the perpendicular for
  one turned between them. One addition, for a `Free` hull only: its tracks
  never drive it past its commanded speed in all. Without that, a hull pressed
  into a wall on a slant slid along it at about 250 px/s, faster than its top
  speed of 210, because the drive along the tracks outran the grip across them.

## The collider

A `Free` hull stands on a round collider (`Physics::spawn_round_tank`). Every
tank's body - a seat's, a band's, a wave's on arrival, the dev server's - comes
through `simulation::spawn_hull`, which picks the shape from `Tank::steer`.

- **Why round.** A box turned with the hull reaches into a wall it pivots
  beside, and rapier pushes an overlap out at 3 px/s
  (docs/physics-engine-design.md "Scale"). A circle turns in place with
  nothing to refit, and slides off walls, corners and other hulls at any angle.
- **Its radius** is the movement box's half-length (`Tank::move_radius`). Head
  on, it stops exactly where a `Cardinal` hull's box does (`steer_tests` pins
  it). The cost: about 2 px more room at the sides than the box, and a
  32 px (one-cell) gap closes to most chassis. The maps are built on two-cell
  corridors.
- **Frictionless** (`friction` 0, the `Min` combine rule). At the statics'
  0.5, a hull driven at a wall on a slant pressed into it and crawled along at
  about 11 px/s. With none, it slides at about 199 px/s, with a bump of about
  a pixel where it crosses the seam between two wall tiles.
- `resize_collider` is never called on it: the drive's facing check, a gauss
  rail's overcharge spin and the dev server's teleport all skip a `Free` hull.

## What reads the facing

- **Shots** leave along `Tank::rotation` by trigonometry, as they always have,
  so every gun fires where a turned hull faces.
- **The hit test** meets a turned tank, seat or enemy, on its true boxes:
  `hits::tank_entries` turns the shot's segment into the hull's own frame
  (`Tank::to_local`) and tests the hull and turret boxes there
  (`Tank::local_boxes`). On the four ways it is the axis-aligned test box for
  box, so a `Cardinal` round's hits are unchanged. A gauss slug's pierce list
  takes the same test. Lag compensation's rewound boxes (online only) stay
  axis-aligned.
- **Boxes for everything else** (blasts, the EMP, a well, a tower's ooze):
  `hull_bbox_world`/`turret_bbox_world` answer the axis-aligned box around the
  turned box.
- `Tank::forward` is the facing as a unit vector: `Dir::vec` exactly on the
  four ways, sine and cosine between. The gauss rail, the well's orb, the lamp
  and the specials' fallbacks use it. `Tank::facing` is the nearest of the four
  (`Dir::nearest`), for what only knows four ways: the sonic hammer's cone, a
  wind-up tell, the rod's first reticle, every read the AI makes of its own
  facing, and its read of a seat's rail lane and well facing.

## Enemies

An enemy of a free round is a `Free` hull too. The behaviour tree decides as it
always has; two of its outputs change.

### Following a route (BB-130)

`Ai::steer_toward` hands a free hull to `steer_toward_free`, which drives
straight at:

1. the target itself, while the line to it crosses only plain ground
   (`Grid::plain_line`, every cell at step cost 1) up to a cell short of it -
   a line as cheap as any route. Short of it, because what stands around the
   target (a seat's own firing lane) prices whichever way it is reached;
2. else the farthest cell of its route ahead it has a clear line to
   (`Grid::clear_line`), so the route's corners are cut and its costs kept. A
   searched route is read past its first step for this
   (`Grid::route_ahead_far`, `ROUTE_AHEAD_CELLS` cells), staircase or not;
3. else its first step.

The heading travels as `Intent::heading`; `move_dir` is its nearest way, what
the rest of the tree reads. `clear_line` does not hold the hull's own cell
against it: a hull pressed against a wall stands in the wall's margin.

What the four-way follower does that a free one does not:

- **No hold, no margin, no lanes.** A heading that turns a little each think
  is how a free hull drives.
- **No collision sidestep** (`avoid_collisions`). A quarter-turn is instant for
  a four-way hull and a pivot for a free one, so sidesteps came faster than
  the hull could take them and it turned on in place beside the tank it was
  dodging - the probe's `spin`, ten of them over the maze fixture's ten
  rounds. Round colliders slide off each other, and the yield brake still
  holds a hull short of one ahead, facing as it is (`Intent::aim`).

What it adds:

- **Arriving.** A hull turns on a circle of its speed over its turn rate (about
  17 px at 160 px/s and 540 degrees a second), and one at speed orbits a point
  inside that circle rather than reaching it. Within two of those radii of the
  point it steers at it eases off with the distance left (`Intent::slow`); within
  `FREE_ARRIVED_PX` (a quarter cell) it stands, facing as it is - a line a few
  pixels long points anywhere.
- **The stuck clock** waits while the hull is still turning onto its heading
  (more than `FREE_STUCK_TURN_DEG`, 30, off it) and while it eases off to
  arrive: a pivot is not a jam, and an escape fired at one sent the hull
  turning onto another, round and round. It measures progress along the
  heading driven (`Ai::last_heading`).

The stuck escape itself is the four-way follower's (`Ai::stuck_escape`) and
drives four ways.

### Aiming (BB-129)

A four-way enemy fires once a seat stands within `enemy_fire_align_px` of one
of its four lines. A free one has no lines to wait for:

- **The attack tier** (`act_attack_free`): with a clear line to its target and,
  at a seat, inside that seat's sight box (`may_fire_at_seat`, unchanged), it
  holds where it stands and lays its gun (`Brain::hold_and_fire_at`); otherwise
  it repositions as a four-way tank does. A hunter's snipe needs no line-up
  either.
- **The lead** (`Brain::lead_aim`): at a seat, with a weapon whose shot flies
  (shells, minigun bullets, plasma - `shot_speed`), it aims where the seat's
  motion carries it over the shot's flight, times `enemy_free_lead` (0.8). The
  motion is the seat's displacement between thinks, eased over 0.2 s
  (`Ai::seat_motion`), not what its driver asks for: a seat pressed against a
  wall asks to move and stands still. Led by the driver's ask, a free round's
  enemies fired twice the shots for fewer hits (the probe's `advance`, 508
  shots and 93 hits against 241 and 120 four-way); led by the motion, 313 and
  124.
- **The trigger** waits until the hull faces within
  `enemy_free_aim_tolerance_deg` (3) of the aim for `enemy_aim_settle`, then
  goes as a four-way enemy's does: paced by its fire timer, held (mostly) when
  a friend stands on the line (`friendly_blocks_line`), a misfire rolled at
  point blank.

A free hull turns in the apply pass alone: the collect pass's `Tank::control`,
which snaps a four-way hull's facing before it fires, is skipped for it, and it
fires along the facing it has.

## What stays four-way

- **Online.** A room's tanks steer four ways: `heading` and `aim` never travel
  (`IntentMsg` carries `move_dir`), the room server builds its `Game` with the
  default `Cardinal`, and so does every replica and prediction sandbox. The
  wire's pose carries a four-way `dir`, so free steering online would need a
  protocol change (a `PROTOCOL_VERSION` bump).
- **The follow camera** leads the nearest axis (`follow::cardinal`).
- **The AI's lane pricing** (`route_grid_on`) prices a seat's firing lane only
  while it faces one of the four ways.
- **The specials' rules** (hammer, EMP, rail, rod, well, swarm) choose among
  the four ways and the hull turns to them; a grudge shot at a tower and a
  breach line up four ways too.

## The probe's turn rules

The probe's `jitter` and `spin` and the dev server's turn counters were written
for hulls whose facing is one of four exact values (`simulation::debug`). A
free hull's facing is read so they measure what they meant to:

- **jitter** reads `turn_heading`: a four-way hull's own facing, a free hull's
  nearest way, held until it turns `TURN_HYSTERESIS_DEG` (10) past the
  half-way line. Read raw, a free enemy laying its gun on a moving seat a
  fraction of a degree at a time counted as flip-flops.
- **spin** sums `spin_step`: the angle really turned, a reversal breaking the
  chain - a four-way hull's quarter turns exactly as before. Read on the
  nearest ways, a hull that turned 284 degrees counted a full circle.

## Measured

`just probe-fixtures`' ceilings, ten rounds a map at seed 1000, with
`--free-steer`: every fixture within them, and cleaner than four-way - jitter
0 to 5 a map against 2 to 8, no spin against up to 1, no churn against up to 8
(maze 8 four-way). `just probe-fields`' seven field maps likewise: hedge-maze's
churn 9 against 38, archipelago's jitter 10 against 23, harbor-lights' churn 1
against 23. Four-way rounds measure exactly what they did.

## Knobs

`tank_free_turn_speed_deg` (540) and `tank_free_pivot_deg` (90) in the movement
group, `enemy_free_lead` (0.8) and `enemy_free_aim_tolerance_deg` (3) in the
enemy group. Every other driving knob (`tank_speed`, `tank_accel_force`,
`tank_turn_grip_force`, `tank_decel_curve_rate`) is shared with the four-way
model.

## Tests

`simulation/steer_tests.rs`: the turn rate and the short way round, driving
any way at top speed, the reversal's pivot, `move_dir` alone, a `Cardinal`
seat ignoring the heading, the round collider, head-on contact equal to the
box, the wall slide never past top speed, the hit test on a turned hull (and
box for box on the four ways), a free enemy's hull and collider, its route
driven at an angle, its shot at a seat off its four lines and its lead on a
moving one, the probe's turn rules, and a free round replaying bit for bit.
`pathfind::tests::a_clear_line_needs_every_cell_it_crosses_open` holds the
line tests. The pinned one- and two-seat streams (`determinism_tests`) and
`just probe-fixtures` run the `Cardinal` round, so they hold the default
unchanged. Also `touch_tests::the_stick_gives_a_free_hull_the_drags_own_angle`
and `app::tests::the_keys_held_steer_eight_ways`.

## Follow-ups

- Online: a heading on the wire and an angle in the pose.
- The follow camera's lead and the lane pricing along a turned facing.
- The sonic hammer's cone and the wind-up tell along the true facing.
- BB-133: an enemy flipping between chasing the seat and fetching a crate every
  few frames dances on the spot four-way and loops free.
