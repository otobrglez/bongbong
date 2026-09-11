# Tap orders — tap where to go, tap whom to attack

Status: exploration, nothing implemented. Follows docs/mobile-and-scaling-
design.md, which gave a touch player a floating stick and a fire zone.
This doc explores the other way to drive a tank with a finger: **tap a
destination and the tank drives itself there; tap an enemy and it goes
and engages it.** With no stick zones to reserve, the touch layout can be
the desktop one (docs/hud-and-builder-layout-design.md's bar over the
field), and the same orders work with a mouse on desktop for free.

## What already exists

Almost all of it, on the enemy side:

- **Routing.** `pathfind::Grid` (A*, rebuilt every frame from the live
  obstacle layout) and `Ai::steer`: the grid's next step toward a target,
  direction commitment (`committed_dir`/`dir_hold`, so the heading does
  not flicker), the obstacle-ahead override, the displacement-based stuck
  escape (`stuck_timer`, `progress_avg`) and `wander` when there is no
  path. An enemy's staircase route across the map is what a routed player
  tank would look like too.
- **Firing solution.** Shots go straight down the barrel and movement is
  four-directional, so "attack" means *get onto the target's row or
  column within range with line of sight, face it, fire*. That is exactly
  the enemy's attack tier: `aim_alignment_at` (perpendicular offset to the
  firing axis, in front), `enemy_fire_align_px`, the `ENEMY_AIM_SETTLE`
  dwell, `Terrain::line_of_sight`, and the friendly-fire check over the
  movers.
- **The player is driven by an `Intent`** (`move_dir`/`face`/`fire`) that
  `player_phase` receives inside `Input` and hands to `drive_tank` and
  `dispatch_fire`. Nothing there cares whether a key or a program produced
  the intent.
- **Inputs are the replay.** Every round is deterministic given its seed
  and per-frame inputs; the dev server's `input`/`step` tools and the
  probe's scenarios inject intents already.

## The model: an order is input, the autopilot produces the intent

```
  tap / click ──► main.rs: screen → field coords ──► Input.order = Some(Order)
                                                            │
  Game::update ──► player_phase:                            ▼
      keys / stick intent non-empty?  ──yes──► drive as today, order cancelled
                  │ no
                  ▼
      Game::player_order (ActiveOrder) ──► Steering ──► Intent ──► drive_tank / dispatch_fire
             ▲                                   (grid, terrain, movers: the enemy's own inputs)
             └── done / failed / replaced ──► Event::Order{Issued,Done,Failed}
```

- `Input.order: Option<Order>` with `Order::MoveTo(Position)`,
  `Order::Attack { target: OwnerSlot, autofire: bool }`,
  `Order::Collect(pickup index)`, `Order::Stop`. It arrives on one frame
  like a key press; `Game::player_order: Option<ActiveOrder>` remembers it
  until it completes, fails, is replaced by a new order, or is overridden
  by a direct movement intent (a key or the stick always wins, and
  cancels the order - the player must be able to jink out of a shell's
  path without first tapping STOP).
- `ActiveOrder` owns a `Steering` - the routing, commitment and stuck-
  escape state factored **out of `Ai`** into its own struct that `Ai`
  then embeds. The behaviour tree, roles, alerts and engage rings stay
  enemy-only; only the "get me to that point through this grid" part is
  shared. `determinism_tests` and the probe's fixture ceilings guard the
  refactor: enemy behaviour must not change.
- **MoveTo**: the goal is snapped to `Grid::nearest_open` when issued;
  done when the tank is within `order_arrive_px` (about half a cell) of
  it; `OrderFailed` if the grid has no route (a sealed pocket), shown as a
  brief red marker instead of driving into a wall.
- **Attack**: two stages that the enemy already runs as one. *Approach*
  steers at the target until it is within `order_attack_range_px` (the
  enemy attack range by default) on a shared row or column with line of
  sight; *hold* stops, sets `face` to the firing axis, and either fires
  (`autofire`) with the shell's per-press edge semantics and a
  `order_aim_settle_seconds` dwell, or waits for the player to press
  FIRE. The target moves, the approach re-plans every frame as the enemy
  does when tracking the player; the target dies, the order is done.
- **Collect**: MoveTo the pickup's slot; done on `PickupCollected`.
- Orders are three or four tuning rows in a new `group orders`
  (`order_arrive_px`, `order_attack_range_px`, `order_aim_settle_seconds`,
  `order_pick_radius_px`), live-tunable like everything else.

## Who pulls the trigger

The `autofire` flag on the attack order is the one real design choice:

- **Autofire**: aligned with line of sight, the autopilot fires. One tap
  per kill, the closest to "RTS-lite", and what a phone wants.
- **Manual**: the autopilot only approaches and faces; the player fires
  with the FIRE button, a tap on their own tank, or Space. Keeps timing in
  the player's hands and keeps shells scarce (`max_shells` matters).

The flag rides on the order rather than on a tuning knob so the frontend
decides per platform: `main.rs` sets it from touch detection (or a
`--tap-autofire` flag), and a mouse click on desktop issues a manual
attack by default. Nothing in the simulation knows which platform it is.

## What a tap means

`Game::pick(point, radius)` is a simulation accessor next to
`debug_snapshot`, so the priority is one function, testable:

1. a **live enemy** within the pick radius → `Attack` (nearest wins);
2. the **player's own tank** → `Stop` (a second tap while stopped: fire
   once, in manual mode);
3. a **pickup** within the radius → `Collect`;
4. the **frog** → `MoveTo` the nearest open cell beside it (the Protect
   mission's "go guard it" in one tap);
5. anything else → `MoveTo` the tapped point.

The radius is 44 pt converted to field pixels through the `Layout` scale
(88 px on an iPhone at 0.5, 55 px on an iPad at 0.8), because a 40 px tank
at half scale is a 20 pt target and Apple's minimum is 44.

**Tap and drag coexist.** A touch that ends within the dead zone and
~250 ms is a tap and becomes an order; one that moves past the dead zone
is the floating stick from the mobile doc and drives directly (cancelling
any order). So the stick needs no reserved zone any more: it is wherever
the finger lands, and the field can take the whole screen. A mouse gets
the same split: click = order, keys = direct.

## Feedback on screen (`game.rs`, release builds)

- **Destination**: a pulsing ground ring at the goal (one more
  `RingStyle` of `draw_ground_ring`), and the route as faint dots along
  the grid path - the dev overlay already draws enemies' waypoints, the
  same drawing serves the player's route in release.
- **Attack**: a reticle ring on the target and, once aligned, the firing
  axis drawn as a short dashed line so the player sees why the tank
  stopped where it did.
- **Failed**: the ring flashes red and fades.
- The bar's slack shows the order in words: `→ 12,7`, `⚔ ENEMY`, `STOP`.

## Layout consequences

No control zones to reserve, so the touch layout is the desktop one:

| Screen | Field fit | Panel |
| --- | --- | --- |
| iPhone 852x393 | 0.546, 699x393 (or 0.5 with 16 pt bands) | 76 pt columns: readouts stacked left, wave/pips right, FIRE and STOP as 44 pt buttons at the bottom of the columns |
| iPad 1024x768 | 0.8, 1024x576 | the 48 pt bar; FIRE, STOP, PAUSE as 44 pt buttons in the bar's slack, no control band needed |
| Desktop | 1.0 | variant A unchanged; click-to-move works with the mouse |

## Feel risks, and what answers them

- **It is not the arcade twitch the game is built around.** A routed tank
  cannot jink. Answer: direct input always overrides an order, and the
  drag-stick coexists with taps, so the player chooses per moment.
- **Auto-aim is strong.** Answer: manual fire is a flag away; the
  `order_aim_settle_seconds` dwell is the same handicap the enemies carry.
- **The autopilot is naive about danger.** It will route through a
  barrel's blast radius or across an enemy's firing axis. Later: weight
  those cells in the A* cost (`Grid::path_cost` is there) - not needed
  for a first version, and a human on a stick is naive too.
- **Staircase routes look mechanical.** They look exactly like the
  enemies' movement, which the player watches all round; consistency
  helps here.

## Testing, in the codebase's own terms

- `mechanics_tests`: `MoveTo` across a tiny inline map ends in
  `OrderDone` within N frames; `Attack` on an enemy behind a brick wall
  approaches, aligns and (autofire) lands a `Hit`; a goal in a sealed
  pocket yields `OrderFailed` on the issue frame; a key press mid-order
  cancels it the same frame.
- The probe gets `goto` and `hunt` scenarios, and its anomaly checks
  (stall, spin, churn, wall-grind, never-arrived) apply to the player
  while under an order - the same sweeps that hardened the enemy steering
  then harden the autopilot.
- The dev server gets an `order` tool (`{kind, x, y | target, autofire}`)
  and `status.order`, so a tap can be reproduced and screenshotted in
  lockstep.

## What changes

| Where | Change |
| --- | --- |
| `simulation/orders.rs` (new) | `Order`, `ActiveOrder`, `Game::pick`, the per-frame resolve into an `Intent`. |
| `ai.rs` | `Steering` extracted (`committed_dir`, `dir_hold`, `last_position`, `progress_avg`, `stuck_timer`, `escapes`, `steer`, `wander`); `Ai` embeds it, behaviour unchanged. |
| `simulation/mod.rs` | `Input.order`, `Game.player_order`, `player_phase` asks the autopilot when the direct intent is empty; `Event::Order*`. |
| `main.rs` | tap-vs-drag classifier, screen → field through `Layout`, mouse click = order on desktop, `autofire` from touch detection. |
| `game.rs` | destination ring, route dots, reticle, firing-axis line, order text in the bar. |
| `tuning.rs` | `group orders`: four rows. |
| `devserver.rs`, `bin/probe.rs` | `order` tool; `goto`/`hunt` scenarios. |

Order of work: `Steering` extraction first (pure refactor, guarded by
the existing tests and fixture sweeps), then `MoveTo` with the marker,
then `Attack`, then the touch classifier. Each step is playable on
desktop with a mouse before a phone ever sees it.
