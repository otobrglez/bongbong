# Free steering (BB-128)

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

## The model

`tank::Steer` says how a hull steers. `Game::steer` is a pre-init setting like
`Game::players`, kept across restarts. The window sets it to `Steer::BUILD` on
its local round: `Free` under the feature, `Cardinal` without it. A seat's tank
takes it when it is made (`Tank::steer`) and keeps it through a wave's roll-in
and a training respawn. Enemies are always `Cardinal`.

The input fills both `Intent::move_dir` (four ways, as before) and
`Intent::heading` (degrees, 0 up, clockwise), so the input never needs to know
how the round steers. A `Cardinal` hull never reads `heading`; a `Free` hull
reads `heading`, or `move_dir`'s angle when there is none (the dev server's
`move_dir`, a script).

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
seat's body comes through `simulation::spawn_hull`, which picks the shape from
`Tank::steer`.

- **Why round.** A box turned with the hull reaches into a wall it pivots
  beside, and rapier pushes an overlap out at 3 px/s
  (docs/physics-engine-design.md "Scale"). A circle turns in place with
  nothing to refit, and slides off walls and corners at any angle.
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
- **The hit test** meets a turned seat on its true boxes:
  `hits::tank_entries` turns the shot's segment into the hull's own frame
  (`Tank::to_local`) and tests the hull and turret boxes there
  (`Tank::local_boxes`). On the four ways it is the axis-aligned test box for
  box, so a `Cardinal` round's hits are unchanged. A gauss slug's pierce list
  takes the same test for a turned seat.
- **Boxes for everything else** (blasts, the EMP, a well, a tower's ooze):
  `hull_bbox_world`/`turret_bbox_world` answer the axis-aligned box around the
  turned box.
- `Tank::forward` is the facing as a unit vector: `Dir::vec` exactly on the
  four ways, sine and cosine between. The gauss rail, the well's orb, the lamp
  and the specials' fallbacks use it. `Tank::facing` is the nearest of the four
  (`Dir::nearest`), for what only knows four ways: the sonic hammer's cone, a
  wind-up tell, the rod's first reticle, the AI's read of a seat's rail lane
  and well facing.

## What stays four-way

- **Enemies.** The AI routes on the cell grid and commands four ways; it keeps
  doing so.
- **Online.** A room's seats steer four ways: `heading` never travels
  (`IntentMsg` carries `move_dir`), the room server builds its `Game` with the
  default `Cardinal`, and so does every replica and prediction sandbox. The
  wire's pose carries a four-way `dir`, so free steering online would need a
  protocol change (a `PROTOCOL_VERSION` bump).
- **The follow camera** leads the nearest axis (`follow::cardinal`).
- **The AI's lane pricing** (`route_grid_on`) prices a seat's firing lane only
  while it faces one of the four ways.

## Knobs

`tank_free_turn_speed_deg` (540) and `tank_free_pivot_deg` (90), in the
movement group. Every other driving knob (`tank_speed`, `tank_accel_force`,
`tank_turn_grip_force`, `tank_decel_curve_rate`) is shared with the four-way
model.

## Tests

`simulation/steer_tests.rs`: the turn rate and the short way round, driving
any way at top speed, the reversal's pivot, `move_dir` alone, a `Cardinal`
seat ignoring the heading, the round collider, head-on contact equal to the
box, the wall slide never past top speed, the hit test on a turned hull (and
box for box on the four ways), and a free round replaying bit for bit. The
pinned one- and two-seat streams (`determinism_tests`) and the probe fixtures
run the `Cardinal` round, so they hold the default unchanged. Also
`touch_tests::the_stick_gives_a_free_hull_the_drags_own_angle` and
`app::tests::the_keys_held_steer_eight_ways`.

## Follow-ups

- Free-steering enemies: an any-angle path follower over the A* route.
- Online: a heading on the wire and an angle in the pose.
- The follow camera's lead and the lane pricing along a turned facing.
- The sonic hammer's cone and the wind-up tell along the true facing.
