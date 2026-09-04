# Dev server + MCP adapter

A local-only debug server embedded in the native game (`--features dev-tools`)
and a thin stdio MCP adapter (`bbmcp`) so an AI coding agent - or a shell
script - can drive, inspect and screenshot the *running windowed game*.
The headless probe (`src/bin/probe.rs`) covers logic; this covers the rest:
the rendered frame, frame-by-frame stepping with the numbers and the pixels
side by side, direct scenario setup, and live tuning.

Never in a release build: `src/devserver.rs` is gated on
`all(feature = "dev-tools", not(target_os = "emscripten"))`, `bbmcp` has
`required-features = ["dev-tools"]`, and cargo-dist builds without features.

## 1. Pieces

```
Claude Code ──stdio JSON-RPC (MCP)──> bbmcp (src/bin/bbmcp.rs)
                                          │ TCP 127.0.0.1:4747, one JSON line per request/reply
                                          ▼
                                  DevServer (src/devserver.rs)  socket threads ──mpsc──> main loop
                                          │ serviced between frames in main.rs's closure
                                          ▼
                                  Game (simulation) · overlays (game.rs) · screenshot (raylib)
```

- `src/devserver.rs` - `DevServer`: the listener, the request queue, lockstep
  state, the tool dispatcher, screenshot capture, the event ring. Also the
  tool table `TOOLS` (name, description, JSON schema) that the adapter
  advertises, so the two cannot drift.
- `src/bin/bbmcp.rs` - MCP over stdio (`initialize`, `ping`, `tools/list`,
  `tools/call`); each call is one request line to the game. Also
  `bbmcp call <tool> [json]` for the shell (`just mcp-call`).
- `src/simulation/debug.rs` - the read/mutate surface on `Game` the server
  uses: `debug_snapshot`, `debug_teleport`, `debug_set_tank`, `debug_kill`,
  `debug_spawn_enemy`, `nav_grid_ascii`. Always compiled (cheap, unit-tested
  without a socket).
- `Game::events` / `simulation::Event` - the per-frame event log (fired, hit,
  wreck, ram, shield deflection, shell-vs-shell cancel, frog bite, pickups,
  round start/end) the phases append to and the server
  streams. `Game::frame` counts `update` calls per round.
- `Game::debug_overlays` (`simulation::Overlays`) - flags `game.rs` draws:
  blocked nav cells, AI waypoint/heading/last action, projectile hit boxes,
  engagement targets, pickup radii. Screen-space, post-composite, like the
  I-key inspect overlay.
- `.mcp.json` - registers the `bongbong` MCP server for Claude Code
  (`cargo run -q --features dev-tools --bin bbmcp`).
- `justfile`: `run-dev` (game with the server), `watch-dev`, `mcp-call`.

## 2. Wire protocol (game side)

Newline-delimited JSON on `127.0.0.1:<port>` (`--dev-port`, else
`BONGBONG_DEV_PORT`, else 4747):

```
→ {"id": 1, "method": "step", "params": {"frames": 60, "move_dir": "up"}}
← {"id": 1, "result": {...}}          or          {"id": 1, "error": "message"}
```

Methods are exactly the `TOOLS` names. Any client works:

```
printf '{"id":1,"method":"status","params":{}}\n' | nc 127.0.0.1 4747
```

### Threading

Socket threads never touch the game. Each request becomes a `Request`
(method, params, a one-shot reply channel) on an `mpsc` queue; the main loop
drains the queue at the frame boundary (`DevServer::before_frame`, before
`tuning::apply_pending`) - the same staging shape `tuning.rs` uses - so every
read and write happens while no `update` is running and the round RNG sits
in `Game::rng`. The socket thread waits up to 120 s for the reply and drops
the connection after 300 s idle. No mutexes anywhere.

### Frame order in `main.rs`

```
before_frame        drain requests: answer immediates, arm step/screenshot, stage tuning
apply_pending       tuning patches land (dev panel, --tuning watch, tuning_set alike)
read keyboard       → Input
shape_input         injected intent replaces the keyboard for N frames
advance             real-time update | n lockstep updates | nothing (frozen)
render              overlays drawn from Game::debug_overlays
after_render        pending screenshot captured (one frame after arming: the read-back lags a present)
```

## 3. Lockstep and determinism

- `step {frames}` puts the server in **lockstep**: the main loop stops
  advancing the game on its own. The step runs its `frames` updates
  back-to-back inside one rendered frame at `PHYSICS_FIXED_DT` (1/60 s) -
  the same cadence the probe and the headless tests use - so it is fast
  (thousands of frames in milliseconds) and replayable.
- `pause` enters lockstep without stepping (no PAUSED overlay, so
  screenshots stay clean); `resume` leaves it and clears the P-key pause.
- `restart {seed}` pins the seed and leaves the new round frozen in
  lockstep, so no wall-clock frames slip in before the first `step`;
  `restart` + the same `step`s replays bit-for-bit (`devserver::tests::restart_with_a_seed_replays_identically`,
  and verified through the windowed game).
- Real-time frames use wall-clock dt, so a round that ran in real time is
  not the seeded replay; `step` from a fresh `restart` is the repro loop.
- Mid-round mutators (`teleport`, `set_tank`, `kill`) diverge the round from
  its seeded replay from that point on; `spawn_enemy` additionally draws
  from the round RNG. Nothing else here consumes RNG, so the probe's
  fixture baselines and `determinism_tests` are unaffected by the feature.
- `frame` counts `update` calls this round (paused frames included) and
  resets on `init`; a `step` reply's `restarted: true` means the round
  restarted inside that step (R key, or the end-screen countdown ran out).
  `time` does not advance while paused.
- A screenshot shows the state after the most recent step. Reading the
  screen returns the frame presented before the current one (raylib reads
  back after the buffer swap), so the capture happens one rendered frame
  after the request; in lockstep that frame is identical and already
  carries overlay flags passed with the request.

## 4. Tools

| tool | params | reply |
|---|---|---|
| `status` | - | seed, frame, time, outcome, paused, lockstep, tank counts, overlay flags |
| `snapshot` | `detail: compact\|full` | tanks (slot, chassis, x/y, velocity, damage/hp, ammo, weapon, shield, boost; `full` adds `ai`), projectiles (cap 64), pickups, frog, engage targets |
| `events` | `since`, `limit` | ring of `{seq, frame, event, ...}` (cap 1024) |
| `step` | `frames`, `move_dir`, `face`, `fire`, `snapshot`, `detail` | frame, time, outcome, restarted, events of the step, snapshot |
| `input` | `move_dir`, `face`, `fire`, `frames` | override the keyboard for N real-time frames |
| `pause` / `resume` | - | status |
| `restart` | `seed`, `enemies`, `tank_row`, `map` | status |
| `screenshot` | `scale` (0.5), `source: screen\|scene`, `overlays` | PNG (base64) + path under `target/devshots/` |
| `overlays` | `nav_grid`, `ai`, `projectiles`, `engage`, `pickups`, `inspect` | current flags |
| `nav_grid` | - | ASCII grid with tanks/frog/pickups marked |
| `teleport` | `slot`, `x`, `y`, `facing` | - |
| `set_tank` | `slot`, `damage`, `*_ammo`, `laser_charges`, `shield_timer`, `speed_boost_timer` | the tank |
| `kill` | `slot` | applied on the next simulated frame through the normal kill path |
| `spawn_enemy` | `x`, `y`, `row` | new slot |
| `tuning_get` / `tuning_set` / `tuning_reset` / `tuning_schema` | `diff_only` / `patch` / - / `group`, `name_contains` | see docs/runtime-tuning-design.md |

Slots are `Tank::owner_slot`: 0 = player, enemies from 1. Positions are
screen pixels (1280x720 by default), y down, rotation 0 = up.

Payload discipline: snapshot numbers are rounded to 0.1, projectiles are
capped at 64 (`projectiles_total` has the real count), a full snapshot of a
5-tank round is about 10 KB, screenshots default to half size.

## 5. Using it from Claude Code

1. `just run-dev` (or `just watch-dev`) - the game prints
   `[dev] listening on 127.0.0.1:4747`.
2. Approve the project's `.mcp.json` server when Claude Code asks; `/mcp`
   lists `bongbong` and the tools appear as `mcp__bongbong__<tool>`.
3. Typical loop: `restart {seed}` → `overlays {...}` → `step {frames}` →
   `snapshot`/`events`/`nav_grid` → `screenshot` → change code → the game
   relaunches (`watch-dev`) → repeat. With the game closed every tool
   returns an `isError` result naming `just run-dev`.

The adapter is started through `cargo run`, which blocks on cargo's build
lock while `cargo watch` is compiling; `./target/debug/bbmcp` works as the
`command` too once built.

## 6. Tests

`cargo test --lib --features dev-tools`: `devserver::tests` drive a headless
`DevServer` through its request channel (status shape, `step` equals manual
fixed-dt updates bit-for-bit, teleport/kill/spawn/set_tank, nav grid and
full-snapshot shape and size, seeded restart replay, tuning errors, base64
vectors, every tool schema is an object schema) plus one real socket
round-trip on an ephemeral port. `simulation::mechanics_tests` covers the
event log; `pathfind::dims_tests` the grid accessors. Rendering, overlays
and screenshots are verified by running `just run-dev` and reading the PNG.
