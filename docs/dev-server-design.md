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
  advertises, so the two cannot drift. `before_frame` and the dispatcher
  take the whole `mode::Session` (the round, the map builder and which of
  the two is live), so the mode switch and the builder are tools too
  (section 4.1); `advance`/`after_render` still take the bare `Game`.
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
  the inspect readout (tank hitboxes + stats), blocked nav cells, AI
  waypoint/heading/last action, projectile hit boxes, engagement targets,
  pickup radii. Screen-space, post-composite. The I key cycles the presets
  off -> inspect -> all (`Overlays::next_preset`, via
  `Input::cycle_overlays_pressed`); the drawing code is dev-only
  (`#[cfg(feature = "dev-tools")]`), so a release build has none of it.
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
read keyboard       → Input (play mode) | BuilderInput (build mode)
shape_input         injected intent replaces the keyboard for N frames
advance             real-time update | n lockstep updates | nothing (frozen, or the builder is live)
render              overlays drawn from Game::debug_overlays; the builder draws itself in build mode
after_render        pending screenshot captured (one frame after arming: the read-back lags a present)
```

In build mode, or while the leave-round dialog is up, `main.rs` never
calls `advance` (`Session::playing`), so the round is frozen by
construction; every request still lands in `before_frame`.

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
| `status` | - | seed, frame, time, outcome, `mission`, `spawn` (the resolved plan), `intro_seconds_left`, paused, lockstep, tank counts, overlay flags, `map` (name/cells/tanks), `mode` (play\|build), `dialog_open`, `builder` (dirty, tool), history depth |
| `snapshot` | `detail: compact\|full` | tanks (slot, chassis, x/y, velocity, damage/hp, ammo, weapon, shield, boost, `ring` (the health ring's opacity, 0..1), `nearest_ally_px`; `full` adds `ai`), projectiles (cap 64), pickups, frog, `engage` (per enemy: status `engaged\|wreck\|fleeing\|retreating\|out_of_range`, `ring` slot index or null, target x/y, `sticky`; `full` adds the rejection tally and the 16-slot table), `clusters` (live enemies within 90 px, as slot groups) |
| `events` | `since`, `limit`, `kinds`, `exclude` | ring of `{seq, frame, event, ...}` (cap 4096); `kinds`/`exclude` filter by event name |
| `step` | `frames`, `move_dir`, `face`, `fire`, `p2_move_dir`, `p2_face`, `p2_fire` (player 2, two-player rounds), `fire_every`, `snapshot`, `detail`, `kinds`, `exclude` | frame, time, outcome, restarted, events of the step (filtered like `events`), snapshot |
| `input` | `move_dir`, `face`, `fire`, `p2_move_dir`, `p2_face`, `p2_fire`, `frames`, `cycle_overlays` | override the keyboard for N real-time frames; `cycle_overlays: true` presses the I key once |
| `pause` / `resume` | - | status |
| `restart` | `seed`, `enemies`, `tank_row`, `players` (1\|2, kept for later restarts), `tank2_row`, `map` (path) or `map_toml` (inline TOML), `mission` (protect\|hunt\|destroy), `spawn` (band\|waves) with `waves`/`wave_size`/`wave_growth`/`tier_start`/`tier_end`, `intro` (start frozen behind the mission banner; default false) | status; neither map param keeps the current map; level params override the map's own tables and stay pinned. Always returns to play mode; a new map replaces the builder's canvas and baseline too |
| `map_get` | - | the current map: name, cells, tanks, `toml` (edit and hand back via `restart {map_toml}`) |
| `history` | `slot`, `last` (600), `every` (10) | sampled per-tank rows (frame, x/y, action, ring, stuck, touching) over the last N frames plus per-tank aggregates: frames, distance, net, cluster_frames, stuck_frames, no_ring_frames, touching_frames |
| `screenshot` | `scale` (0.5), `source: screen\|scene`, `overlays` | PNG (base64) + path under `target/devshots/` |
| `overlays` | `nav_grid`, `ai`, `projectiles`, `engage`, `pickups`, `inspect` (individual flags; the I key cycles presets off -> inspect -> all) | current flags |
| `nav_grid` | - | ASCII grid with tanks/frog/pickups marked |
| `teleport` | `slot`, `x`, `y`, `facing` | - |
| `set_tank` | `slot`, `damage`, `*_ammo`, `laser_charges`, `shield_timer`, `speed_boost_timer` | the tank |
| `kill` | `slot` | applied on the next simulated frame through the normal kill path |
| `spawn_enemy` | `x`, `y`, `row` | new slot |
| `tuning_get` / `tuning_set` / `tuning_reset` / `tuning_schema` | `diff_only` / `patch` / - / `group`, `name_contains` | see docs/runtime-tuning-design.md |

Slots are `Tank::owner_slot`: 0 = player, enemies from 1. Positions are
screen pixels (1280x720 by default), y down, rotation 0 = up.

### 4.1 The two modes and the map builder

The game has a play mode and a build mode in one window
(docs/game-editor-fusion.md); `mode::Session` owns the `Game`, the
`MapEditor` and the switch, and both the window and these tools go through
its methods (`press_build`, `answer_dialog`, `play`, `replace_map`,
`update_builder`), so a tool and a click are the same path. Field cells are
`[col, row]` on the 32 px grid (40 x 23 at 1280x720); window positions for
`click` include the 32 px HUD bar, so the field starts at y = 32.

| tool | params | does |
|---|---|---|
| `mode` | - | `mode` (play\|build), `dialog_open`, and the builder's `dirty`, `map_name`, `tool`, `category`, `open_menu` (a category name, `map` or `save`, else null), `undo_depth`, `redo_depth`. Cheap |
| `build` | `answer` (leave\|stay) | presses BUILD: opens the leave-round dialog mid-round (the round freezes), switches at once on the end screen; `answer` answers an open dialog instead. Replies like `mode` |
| `players` | `count?` (1\|2) | presses the players button in play mode: without `count` opens/closes the "How many players?" dialog (the round freezes); with `count` answers it - a different count restarts the round in that mode, frozen in lockstep (slot 1 becomes player 2, enemies count from 2); the same count just closes. Refused in build mode. Replies like `mode` |
| `play` | `intro` | presses PLAY in build mode: the builder's map becomes the round's, a fresh round starts frozen in lockstep like `restart` (a pinned seed stays pinned; `intro: true` for the banner). Replies with `status`. An error in play mode |
| `builder_tool` | `tool` | selects a brush by name (the 23 cell tools - `start2` is player 2's start - or `eraser`) through the category's own path; without `tool`, reports the active tool and every category's current tool and list |
| `builder_paint` | `cells`, `tool`, `button` (left\|right) | one stroke: press on `cells[0]`, drag through the rest, release - the toggle-erase rule, singleton moves and one-undo-step-per-stroke apply as for a mouse; `right` erases. Replies with each changed cell's `before`/`after` (the map's own `{kind, ...}` shape, null = empty) and `undo_depth` |
| `builder_undo` / `builder_redo` | `steps` (1) | undo/redo that many steps; replies `undone`/`redone`, both depths, and the `cells` of the last step |
| `builder_settings` | `tanks`, `tank`, `tank2` (player 2's chassis), `mission`, `spawn`, `waves`, `wave_size`, `wave_growth`, `tier_start`, `tier_end`, `reset` | the map's level keys: an absent field is untouched, `null` means auto (mission/spawn fall back to protect/band); each changed field is one undo step, `reset: true` then reverts cells and settings to the baseline. Replies with the current values and `cli_overrides` (which fields `Game`'s CLI/`restart` values outrank at PLAY) |
| `builder_map` | `name`, `map_toml` or `map` | without any: the builder's map as `toml`, `name`, `dirty`, `diff` (added/removed/changed cells, changed settings); with one: loads it (a Load-list name, inline TOML, or a path) into the canvas as one undo step and the new baseline - the round keeps its map until `play`. `map_get` keeps answering with the round's map |
| `builder_files` | | what FILE > LOAD offers: every loadable map (`on_disk` for `maps/*.toml`, else shipped in the binary) and `can_save` (native only) |
| `builder_save` | `name?` | FILE > SAVE / SAVE AS: writes `maps/<name>.toml` (defaults to the map's name), makes it the baseline; replies like `builder_map` |
| `click` | `x`, `y`, `button`, `drag_to` | a raw press at a window position on the same hit-tests the mouse gets: in play, the BUILD button, the players button beside it, either dialog's buttons (outside a panel closes it); in build, the bar's buttons (PLAY starts the round), a dropdown row, a stepper, a field cell; `drag_to` drags in 8 px steps and releases. Replies like `mode` |
| `key` | `key` (tab\|escape\|enter\|undo\|redo\|backspace\|1\|2), `text` | one key for one frame: in play, Tab opens/closes the leave dialog, Esc keeps playing / closes the players dialog, Enter leaves (or, in the players dialog, switches to the other count), 1/2 answer the players dialog; in build, Tab is PLAY and the rest go through `BuilderInput` (`text` types into an open prompt). Replies like `mode` |

**The refusal rule.** In build mode the tools that read or drive the
round - `GAME_ONLY_TOOLS`: `snapshot`, `events`, `step`, `input`,
`pause`, `resume`, `history`, `nav_grid`, `teleport`, `set_tank`, `kill`,
`spawn_enemy` - return an error naming `play` rather than touching a frozen
game. Everything else works in both modes: `status`, `mode`, `screenshot`
and `overlays` (the presented frame is the builder), `map_get` (the round's
map), the `tuning_*` tools, `restart` (returns to play, and with a map
replaces the builder's canvas as well), and all the builder tools (the
canvas is only *shown* in build mode). A `play`, or a `click`/`key {tab}`
that presses PLAY, does what `restart` does afterwards: banks the
`round_started` event, restarts the history ring and enters lockstep, so
the `step`/`screenshot` loop carries over unchanged.

A session, from PRD section 11:

```
mode                                   -> play
build                                  -> dialog open
screenshot                             -> the dimmed field and the dialog
build {answer: "leave"}                -> build, tool wall/brick
click {x: 340, y: 16}                  -> WALL dropdown open (the caret)
screenshot
click {x: 340, y: 120}                 -> picked iron from the list
builder_paint {cells: [[10,5],[11,5],[12,5]]}
builder_paint {cells: [[11,5]]}        -> toggle-erased the middle iron
builder_undo
builder_settings {tanks: 3, mission: "hunt"}
builder_map                            -> TOML + diff: 3 cells added, 2 settings
play                                   -> frozen round on the edited map
step {frames: 300}
screenshot
```

Payload discipline: snapshot numbers are rounded to 0.1, projectiles are
capped at 64 (`projectiles_total` has the real count), a full snapshot of a
5-tank round is about 12 KB (compact under 6 KB - the slot table and the
rejection tallies are `full` only), screenshots default to half size, a
`history` reply returns at most 2000 rows.

### Debugging enemy clustering

The engagement ring (`simulation/engage.rs`) fills an `EngageReport` every
enemy phase: each enemy's status, the ring slot it holds (`EngageSlot::index`,
0-15: axis-major up/right/down/left, then rank 0 firing / 1 reserve, then
side -1/+1), whether it kept last frame's slot, and how many candidates its
search passed over as `claimed`, `off_map` (the slot falls outside the
battlefield for this player position - a cornered player loses whole axes),
`unreachable` or `no_los`. An *engaged* enemy with `ring: null` steers at the
player's own position (`Brain::engage_point`'s fallback), so several of those
at once is what a pile-up looks like in the data. `snapshot.clusters` and
`history`'s `cluster_frames` measure the symptom (live enemies within
`debug::CLUSTER_RADIUS_PX`, the probe's clustering radius); the AI events
(`engage_slot`, `ai_action`, `stuck_escape`, `breach`, `retreat`, `alert`)
show the decisions in between. Those events are diffs of each enemy's
`AiSnapshot` around its `think`, recorded only while `Game::trace_ai` is set
(the server sets it in `before_frame`; the simulation never reads it), so
tests and the probe see the same event log as before.

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
full/compact snapshot shape and size budgets, seeded restart replay, an
inline `map_toml` restart and `map_get` round trip, `history` rows and
aggregates, `events`/`step` kind filters, tuning errors, base64 vectors,
every tool schema is an object schema, and the mode/builder tools: `build`
asks mid-round and skips the dialog on the end screen, the game-only
refusal in build mode and the tools that must still answer, a stroke's
toggle-erase with undo/redo, settings `null` = auto and `cli_overrides`,
`builder_map`'s diff and load, `play` on the edited map leaving lockstep
on, `click`/`key` through the dialog, the bar and a drag on the field, and
`restart` from build mode) plus one real socket round-trip on an ephemeral
port. `simulation::engage::tests` covers the report (slot
indices, sticky flag, rejection tallies); `simulation::mechanics_tests` the
event log; `pathfind::dims_tests` the grid accessors. Rendering, overlays
and screenshots are verified by running `just run-dev` and reading the PNG.
