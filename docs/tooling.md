# Tooling reference

The dev servers, the probe, the linter, the network lab, the offline rig, thumbnails, the test
suites and the asset pipeline in full. CLAUDE.md keeps the commands and the rules that break
things when forgotten; this is the detail. Update it with the tool.

## Testing & tooling

- **Driving the room server**: `just run-server-dev` starts it with its dev socket and `.mcp.json`
  registers the same `bbmcp` binary a second time, so `mcp__bongbong-rooms__*` (`server_status`,
  `rooms`, `room`, `room_open`, `seat_intent`, `room_set_tank`, `room_spawn_pickup`, `room_tuning`,
  `room_step`, `room_snapshot`, `room_events`) drive the **authoritative** round - the one place a
  co-op bug can actually be observed, since the window's own tools describe a replica it never
  simulates. `just rooms-mcp-call <tool> '{json}'` is the shell path. A scenario needs no browser
  and no second machine: `room_open {seats: 2}`, `seat_intent`, `room_step`, `room_events`.
- **The MCP adapter**: `.mcp.json` starts the built `target/debug/bbmcp` through
  `tools/dev/bbmcp.sh` (never `cargo run`, whose build can outlast the client's connect timeout):
  `just build-mcp` once per checkout or worktree, headless, no raylib. With no window from its own
  checkout on the dev port the adapter hosts a **headless round** itself - private, deterministic,
  stepped only by `step`, `screenshot` the field on the CPU canvas (no HUD, overlays, particles or
  shaders) - so a peer session's game on 4747 is never driven by accident; `attach` reports and
  switches the target. The tool list carries a line or two per tool: `help {tool}` has the reply's
  shape. A parameter a tool does not take is refused by name. `batch` runs a scenario in one call,
  `step {until: [events], until_outcome}` stops at what you are waiting for, `probe {scenario,
  args}` runs the probe, `lint {map}` lints a file, and `reload` rebuilds and restarts the adapter
  in place after a source change (docs/dev-server-design.md sections 3.1 and 4.4).
- **Driving the windowed game**: `just run-dev` starts the dev-tools build with the dev server;
  `.mcp.json` registers `bbmcp`, so the `mcp__bongbong__*` tools (`status`, `restart {seed,
  map_toml, mission, spawn, players, intro, ...}`, `step {frames, move_dir, fire, ...}`, `snapshot`,
  `terrain`, `events`, `history`, `nav_grid`, `field`, `lint`, `overlays`, `screenshot`,
  `teleport`/`set_tank`/`kill`/`spawn_enemy`/`spawn_pickup` (a crate by its map spelling at the
  nearest cell), `rod_call` (a rod called at once on a cell), `tuning_*`, `players`, and the
  mode/builder tools `mode`, `build`, `play`,
  `builder_tool`/`builder_paint`/`builder_undo`/`builder_redo`/`builder_settings`/`builder_map`,
  `click`, `key` - the last two also drive the lobby, through the same `lobby::LobbyInput` `app.rs`
  fills) drive it in lockstep and return PNGs - the way to verify rendering, overlays and feel or to
  reproduce a situation (teleport, step, screenshot). Example session in docs/dev-server-design.md
  §4.1. Shell: `just mcp-call step '{"frames":120}'`. For a pixel-for-pixel screenshot comparison
  set `fx_max_particles` and `fx_max_pieces` to 0 first (`tuning_set`): the particle layer and the
  wall pieces draw from `rand::rng()`, so they differ run to run. A frozen round is drawn as it stands
  (`Game::pose_at` at alpha 1, no texel anchor), so a lockstep screenshot is the step itself; a
  screenshot of a round running in real time is the frame the display got, between two steps
  (`pose.rs`). A builder screenshot differs run to run too: BUILD dresses the
  canvas in the paused round's looks, so a pinned seed pins its ground and its tiles' variants, but
  its water, portals, grass, trees, bongs and sky move on the wall clock - pin the clock in a
  scratch build to compare the builder pixel for pixel. The `camera` tool pins a zoomed view
  (`{x, y, zoom}`; a pin outranks
  the follow camera) and `{reset: true}` returns to the map's class; `status.camera` is the camera
  the window last drew: `view` (whole|follow|pinned), `rect`, `scale`, `target`, `window_field`, and
  for a followed view `seat`, `focus`, `cut`, `lead`, `offset`, `seating`, `framing` (cells, device
  scale, block px, snapped, the tank in pt and mm, bars) and `sight_box` (`in_view`). `click` takes
  window coordinates - aim it from `status.ui` (`scale`, `screen`, `area`, `touch`, the corners'
  `buttons`/`clusters`, and `screen_buttons` for whatever stands over the round: `level_N`, `back`,
  `one`/`two`, `leave`/`stay`, `levels`/`again`/`next`, the lobby's `host`, `join`, `key_K`,
  `start`, `kick_N`) - and replies with the `world` point (`Camera::to_world`). One instance binds
  4747; a second uses `BONGBONG_DEV_PORT`. The MCP adapter caches the tool list, so a new tool needs
  a `/mcp` reconnect.
- **`src/bin/probe.rs`** — headless gameplay probe, the default way to verify mechanics: `cargo run
  --bin probe -- --scenario afk|advance|brake|circle|defend|survive [--enemies N] [--tank K] [--mission ..]
  [--spawn ..] [--frames N] [--log-every N] [--seed S] [-m map] [--rounds N] [--json-out F]
  [--budget kind=N ...] [--heatmap] [--tuning F] [--crate KIND] [--print-rust]`; `--crate` puts that
  pickup in every weapon crate's place (the fixtures carry none). Built on
  `Game::outcome()`/`tank_snapshots()`/`events()`/`bong_position()` - extend those public accessors,
  not raw world access (a bin sees only `pub`). Per-frame **anomaly checks** on enemies print
  greppable `ANOMALY round= seed= frame= kind= tank= pos=` lines, each seed a replay recipe; kinds
  (`ANOMALY_KINDS`, the single list wiring `--budget`/`--json-out`/`--heatmap`/summaries):
  stale-start, stall, border-stuck, jitter, staircase, spin, churn, clustering, wall-grind, bump-rate,
  low-progress, never-arrived, invariant (also players/wrecks and `PhysicsQuarantine`), tank-grind,
  pile-up, offbox-fire (an enemy shot or missile lock on a seat from outside that seat's sight box;
  budgeted at 0 in every recipe; ci.yml runs `just probe-fixtures`, so the justfile holds the only
  copy of the ceilings). A fire tally (`TankSnapshot::shot_at_seat`/`hit_by_seat`) prints beside the
  rams and goes into `--json-out` as `fire`; `seat-hits-offbox` is the reverse unfairness, and
  `enemy-hits-on-enemies` counts enemy tanks damaged by another enemy's shot, beam, flame or blast
  (`TankSnapshot::hit_by_enemy`, one per tank per frame; a ram between the two is left to the ram
  tally). Sweep
  totals sum every kind (`AnomalyTotals::add`). Thresholds are named consts at the top (the
  spin/jitter windows and `signed_quarter_turn` live in `simulation::debug`, shared with the dev
  server's live turn counters); a new kind extends `TankTrack` + `check_anomalies` +
  `ANOMALY_KINDS`. stall/stale-start are muted for a deliberate hold (firing solution, retreat
  recharge, and the `HOLDS` rows: asleep, a tell, a skid, kept out of a danger, a charge, a stand
  for the FPV swarm - `air`); a state the tank's motion is not its own in (`OUT_OF_ITS_HANDS`: an
  EMP's outage) mutes every check and restarts its windows. Four A,B,A turns inside two seconds
  flag a tank, and what the run covered says which kind: each flip is paired with the one two
  before it, and where the pairs' straight lines come to `STAIRCASE_PROGRESS` (0.35) of the ground
  driven between them and a nav cell a round, the turns carried the hull on - a `staircase`, a
  diagonal route or a weave past something - and otherwise it is back where it was or dithering on
  the spot: `jitter`, a flip-flop. A tank is flagged once for each, and the line spells the run's
  legs out (`U144 L144 U98 L0`: heading and px), so most readings need no `--log-every 1`; a run
  that is half a stair and half a shuttle can read either way, and there the trace decides. A
  `churn` window that holds the frame a fleeing tank took a health crate (`act_flee` within half a
  second of the `PickupCollected`) is a trip to heal and back, not churn. Ram counts
  (enemy-pair / into-player) are counters, never budgeted, and saturate through the ram cooldown;
  `tank_contact_seconds` does not. `--rounds N` sweeps seeds base+i and prints only anomalies and
  summaries. A sweep also prints `outcomes:` (won/lost/unfinished, lost-after and won-after mean and
  median), `lulls:` (stretches of at least 3 s with no enemy within reach of a live seat or the bong
  between two frames with one; `lulls` in `--json-out`), `rerolls:` with a trace line per straggler
  re-rolled, `first contact:` (the first engagement, shot at a seat and hit on a seat, mean and
  median) and `timing:` (ms per `Game::update`, flagged in a debug build; with `--features
  dev-tools` a `phases:` line splits it by `frame_stages`' `sim.*` groups), carried in `--json-out`
  as `first_contact`/`ms_per_tick`; a single round's header says `map class=field|arena`. On a field
  map a sleeping tank is a deliberate hold, and a sleeping or leashed one is exempt from
  never-arrived, as is a guard keeping its beat (`TankSnapshot::guarding`); a hold within the last
  half second vetoes stale-start. Recipes: `just probe-sweep` (default map), `just probe-waves`,
  `just probe-fixtures` (every `maps/test/*.toml` at a pinned seed against recorded ceilings -
  deterministic, so an exceedance is a real change; re-baseline consciously after a deliberate
  AI/map/tuning change, never bump a ceiling to go green; the probe is built once, headless, and the
  fixtures run one per core, each one's output printed in order; CI's `check` job runs it after
  `cargo check --workspace`, beside the `test` job's `cargo test --lib`), `just probe-survive` (the
  same fixtures under `--scenario survive`, every round its whole 30 s, against its own ceilings;
  CI runs it), `just probe-fields` (the field-map AI: the study map, longwater and five 56-wide
  levels, AFK, 10 rounds at seed 1000; the ceilings and their history are in the justfile, last
  re-recorded 2026-11-19 for the jitter/staircase split - jitter=7 archipelago, staircase=29
  hedge-maze - with clustering and pile-up brought down to 1; the maps run one per core, over a
  minute in a debug build, and CI runs it), `just probe-defend` (release: the perfect defence on longwater and the study map, ten
  seven-minute rounds each at seed 1000, never-arrived 0). `defend` is a perfect defence: every
  enemy within `--defend-reach` px (400) of a live seat or the players' bong is destroyed through
  `Game::debug_kill`, so a round lasts as long as its last straggler and never-arrived counts one
  still out. `survive` is a seat that cannot be worn down: the seats never move or fire, and after
  every frame the probe mends each live seat's hull (`Game::debug_set_tank`) and the players' bong
  (`Game::debug_mend_bong`), so no round ends 4 to 15 s in the way an AFK one does and the enemies'
  whole fight is read - a round nothing ends runs to `--frames` and counts as unfinished;
  `--mortal-bong` leaves the bong unmended, so a Protect round can still be lost. The
  `maps/test/*.toml` fixtures are 40 x 22.5 but carry `view = "whole"`, so they stay
  arenas and their ceilings measure the arena's AI; `maps/missions/waves-basic.toml` and
  `hunt-basic.toml` are field maps by size. `maps/test/` is the adversarial fixture corpus (u-trap,
  choke, tight-corridors, bong-block, maze, pockets, the `props` playground, plus `portals` (two
  rooms joined only by a portal each)), each with header comments describing intent;
  `maps/test/corridors/` and `maps/test/online/` (`hunt-duel.toml`, the round the co-op tests end in
  under a second) sit in subdirectories on purpose (the glob is `maps/test/*.toml`).
- **`src/maplint.rs`** — the static map linter (`cargo test --lib maplint`): a seeded headless
  `Game::init` read through the same `Game::nav_grid` the AI steers by. Checks: playfield
  connectivity (the playfield is the largest open component; a pickup reachable only through
  destructible walls is a `gated-pickup` warning from an Iron-only flood fill), boxed-in cells,
  spawn-band capacity (Band plans, `enemy_spawn_legal`), planner-vs-physics agreement, single-cell
  corridors, gates (`gate-not-on-edge`/`gate-blocked`/`waves-no-gates`), portals
  (`portal-alone`/`portal-blocked`; the linter's floods step along `Grid::portal_links` like the
  planner), `enemy-bong-unreachable`, `hunt-missing-enemy-bong`, the seat checks
  (`player2-unreachable` names the seat by number, over every seat past player 1). Findings carry
  `cells` (`LintCell::Map`/`Nav`) and, where one edit answers them, a `fix`
  (`LintFix::Move`/`Place`/`Remove`: no start, a penned start, player 2 cut off, starts too close, a
  lone portal, a gate off the edge); `lint_map` with a `LintSetup` is the one entry, shared by the
  builder's CHECK and the dev server's `lint` (`lint_timing_on_the_large_maps`, ignored, times it).
  `supported_maps_no_new_errors` (`SUPPORTED_MAPS`: the two defaults, `towers`, `longwater` and
  every shipped level) permits only each map's `KNOWN_ERROR_KINDS` (default.toml: none, its top
  strip pickups are `gated-pickup` warnings) and fails on any new error class; `maps/test/` fixtures
  assert their intended profiles; other maps are lint-and-print (`--nocapture`).
- **Headless build**: `cargo build --no-default-features --bin probe` and `cargo test --lib
  --no-default-features` (add `--features dev-tools` for the dev server) build and test the crate
  with no raylib, no cmake and no C compiler - `cargo tree --no-default-features -e normal | grep
  sola` is empty. The tests that need raylib (sheet decoding:
  `canvas::every_sheet_loads_from_static`, `thumbnail::{shipped_maps_render_on_the_cpu,
  options_change_the_picture}`; the bit-exact checks against raylib in `math::raylib_tests`; the
  corner and bar layout pins in `render::hud::corner_tests` and `editor::render::bar_tests`) run
  under the default features only.
- **Unit tests**: there is no project-wide suite - rendering and feel are play-tested. `cargo test
  --lib` (add `--features dev-tools` for `capi`/`devserver`) covers tuning, capi, pathfind, `hits`
  sweep geometry, `combat` ram, `engage`, `props_tests`, `flame_tests`, `seat_tests`,
  `mechanics_tests` (headless scenarios on tiny inline maps), `determinism_tests`, `spawn_tests`,
  level, `editor_tests`, `history_tests`, `session_tests`, `hud_tests`, `corner_tests`,
  `chrome_tests`, `lobby_tests`, `qr`, `framing` (the research doc's device table), `follow`,
  `indicator_tests`, `picture_tests`, `minimap_tests`, `shake_tests`, `motion_tests`,
  `establish_tests`, battlefield seams, tank rings/the weapon inventory, ai stuck/separation,
  maplint, devserver (headless dispatch, lockstep, builder tools, socket round-trip), view, touch,
  fx, the effects composers (`pyro`, `fireball`, `burst`, `mushroom`, `damage_stage`: on the grid,
  in their ramps, gone by their end, the same picture from the same inputs); `tests/waves_sanity.rs`
  plays a three-wave round (~10 s); `cargo test --lib a_stroke_across_the_study_map_timing --
  --ignored --nocapture` prints a builder stroke's cost per cell and the texels it uploads.
  `a_large_fill_timing` (add `--release` for a phone's order) prints a large RECT's or FILL's cost
  against making the ground again, and `a_load_list_thumbnail_timing` (render feature) the Load
  list's two thumbnail renderers side by side. No rustfmt/clippy config is checked in; `cargo
  clippy --lib --no-default-features --features dev-tools` has no errors (its warnings are not a
  gate). CI (`.github/workflows/ci.yml`) runs, in `test`, `cargo test --lib` and the headless
  `cargo test --lib --no-default-features --features dev-tools`, and in `check`, `cargo check
  --workspace`, that clippy command (an error fails it), `cargo test -p bongbong-server`, `cargo
  test --test waves_sanity --no-default-features` and `just probe-fixtures`, `probe-survive` and
  `probe-fields`. Dependencies
  build at opt-level 3 in the dev profile (`[profile.dev.package."*"]`), and CI's two jobs build
  the dev profile itself at opt-level 1 with line tables only (`CARGO_PROFILE_DEV_*` in ci.yml): the
  game crate compiles about a minute longer and the suite and probe sweeps run several times faster,
  debug assertions and overflow checks still on.
- **`netlab/`** (netlab/README.md, docs/online-coop-prd.md §4.16): how far networked play is from
  local play, per metric, on a given link - the instrument every co-op feel claim is checked with. A
  workspace crate, headless (`bongbong` with `online` only - no `render`, no `dev-tools` - and
  `bongbong-server` without its dev tools): one process runs the real room server, an impairment
  proxy that models **TCP, not packets** (`link.rs`: delay, jitter that never reorders, loss as a
  hold of that chunk and everything behind it until a retransmit, a Nagle switch), a WebSocket tap
  that decodes every frame through the proxy (`wstap.rs`: one-way delays, the server's hold,
  snapshot gaps, bytes), and two headless clients that are the window's own `OnlineRound` over
  `NativeTransport`, driven by scripts (`drive`, `shoot`, `duel`) at a fixed frame rate - all on one
  process clock, so end-to-end latencies are exact. The same scripts go through a **local twin** (a
  two-seat `Game` stepped the way `app.rs` steps it), the reference every metric is compared with:
  own input latency, remote pacing and lag, the shot ledger (press, drawn, `Fired`, hand-off gap,
  hit), incoming fire's distance at impact, corrections, round trip, frame CPU; a verdict of
  `local`/`close`/`far` per run. Profiles `lan`/`good`/`typical`/`mobile`/`bad` or `custom`;
  `--blackhole-after S` swallows everything both ways from S seconds on without closing a socket (a
  path that died silently - `tests/silent_link.rs` holds both clients to noticing it); `--remote
  URL` points the clients at a real rooms host (a PR preview) with no proxy or tap. `just netlab run
  ...`, `just netlab-suite [--quick]` (each run its own process - the tuning table is process-wide).
  `cargo test -p netlab` covers the link model, the tap's parser and one LAN drive.
- **The offline rig** (`src/net/rig.rs`, docs/online-coop-prd.md §4.13): `cargo run -- --rig --delay
  80 --jitter 20 --loss 0.02` puts an authoritative `Game` on a background thread, a `net::loopback`
  link with those dials between it and the window, and the replica on screen - the whole online
  client with no server, no socket and no port, and the way the feel of a delayed round is judged.
  The thread runs the room server's loop shape (sample the seat's mailbox into an `Input`, `update`
  at `PHYSICS_FIXED_DT`, a snapshot every `SNAPSHOT_EVERY` ticks) with two deliberate differences:
  one seat, welcomed and started in one breath, so `--rig` needs no keypress; and full snapshots
  rather than deltas, because a lost packet breaks a delta chain and the rig has no ack path - a
  loss should be the gap the replica has to ride out, which is what `--loss` is for. The seat's
  intents go through the same `net::mailbox` the server holds and its pose through the same
  `net::authority` rule, so its snapshots carry the same `acked`, `mailbox` and `Placed` readings
  and both the client's lead and a client-owned hull behave as they would against a room. The end of
  a round is the server's: the end screen is ticked out and announced rather than restarted
  (`Lockstep::ended`/`rematch`). `-m`, `--seed`, `--enemies`, `--tank` and the mission/spawn flags
  set the round up; `Rig::authority()` is the truth the picture is checked against. Its tests
  (`cargo test --lib net::rig`) play the lane end to end headlessly: every drawn hull is between two
  authoritative snapshots, the replica only ever stands on a snapshot tick, and the picture moves on
  frames no snapshot arrived on. They run the rig's room and an `OnlineRound` over it on one
  hand-wound clock (`net::now`, the tests' `Wound`): each frame advances the clock a frame and the
  room takes exactly the ticks that fell due in it, so the link's delay and jitter and the
  interpolator replay on every run however loaded the machine; one test
  (`the_rigs_thread_runs_a_round_over_a_lossy_link`) keeps the thread on the wall and asks only what
  no load can change. **`rig::Lockstep`** is the same lane with no thread and no clock,
  for a test or a tool: `start` seats a client and `step {ticks}` runs the authority that many ticks
  and lets the replica catch up to the last snapshot they earned, so a networked round replays the
  way `restart` + `step` replays a local one - the room's `now` advances by exactly
  `PHYSICS_FIXED_DT` a tick, so even `Snapshot::server_ms` comes out the same. Its link is perfect
  and nothing is interpolated: its link and `net::interp` read the wall, so delay, jitter and the
  interpolator are the windowed tests' and the threaded rig's.
- **Native runtime tuning**: `cargo run -- --tuning knobs.json` loads a JSON patch and re-applies it
  whenever the file changes (bad file: fails at startup; bad edit: stderr, ignored). The probe takes
  the same flag (echoed in its header and `--json-out`); `--print-rust` renders it as `tunables!`
  rows to paste into `tuning.rs`.
- **`src/bin/mapshot.rs`** (docs/mapshot-prd.md) — map thumbnails: `mapshot maps/x.toml -o x.png`,
  `--out-dir DIR maps` for a batch (`just thumbnails`), `--renderer cpu|gpu` (cpu default:
  `canvas::CpuCanvas`, no window; gpu: a hidden window through the game's own pass-1 stages),
  `--check` (both, prints their difference; `just mapshot-compare` - a `cargo test` cannot open a
  window on macOS), `--scale`, `--seed` (default `0xB0B5`), `--players`/`--tank`/`--tank2`,
  `--no-tanks`, `--plain`, `--no-shadows`, `--paint-bench FRAMES` (no PNG: the painters' benchmark,
  Performance instruments below). `thumbnail.rs` is the library side: `stage_round` (init +
  **one update**, as the window runs before its first render; no intro, `enemy_count_override =
  Some(0)` under a forced Band plan), `render_cpu`, `compare_pixels`; `render/thumbnail.rs` has
  `load_cpu_sheets`, `GpuSheets`, `render_gpu` and `image_png_bytes`; its tests pin the CPU render
  hash of every `SHIPPED_MAPS` entry (re-baseline consciously after an art/map/tuning change).

## Performance instruments

What the window draws, how its frames are paced and what a step costs, measured rather than read
off the code. All of it is dev tooling: without `dev-tools` every counter and timer is an empty
call (`cargo check --no-default-features` builds none of it).

- **`frame_stages.rs`** keeps four readings in a dev-tools build. *Stages*: where a frame's
  processor time goes (`sim`, `fx`, `pictures`, `lights`, `world`, `post`, `chrome`, `swap`),
  averaged over about half a second, and inside `sim` the groups of phases `Game::update` times
  itself - `sim.terrain` (`Terrain::build`), `sim.nav`, `sim.seats`, `sim.enemies`, `sim.weapons`
  (waves, towers, the special weapons, the flame and laser resolves), `sim.physics`, `sim.rams`
  (`sync_tanks_and_ram` to `ram_props`), `sim.shots`, `sim.fire` (burns, fuses, lava, crates),
  `sim.grass`, `sim.end`. *Draws* (`Draws`): what the last whole frame drew through `GpuCanvas` and
  `render::pyro::Rl` - `blits`, `rects`, `shapes` (discs and rings) and `texture_switches` (how
  often one draw's texture differed from the one before it, rectangles and shapes counting as
  raylib's shapes texture: the least number of draw calls those draws split into) - and the
  `blend_scopes`, `shader_scopes` and `target_scopes` it opened, each two forced flushes (every
  `draw_blend_mode`/`draw_shader_mode`/`draw_texture_mode` call passes its argument through
  `frame_stages::blend`/`shader`/`target`; write a new one the same way). Chrome that calls raylib
  directly (the HUD, the builder's bars) is in the scopes but not in the draws. *Pace* (`Pace`):
  over the last 600 frames, the worst frame, the 99th percentile and how many frames ran 0, 1, 2,
  3 or 4 steps (only frames the play clock paid out for; a lockstep frame is not one). *Totals*:
  each stage's time since the thread started, which the probe prints.
- **Where they are read**: `status.frame_stages` - one key a stage in ms, plus `draws` and `pace`
  (`frames`, `max_ms`, `p99_ms`, `steps`); the two lines under the left cluster with the
  `ui_frame_stages` knob on (`frame 8.5 ms | sim 0.1 ...` and `draws 2067 blits 804 rects 6 shapes
  73 tex | scopes 4 blend 1 shader 2 target | p99 8.7 max 9.4 ms | steps 292/308/0/0/0`); the
  five-second `bongbong: frame avg` log line; and `probe: phases:` (ms a tick a group, `other` the
  rest) when the probe is built with `--features dev-tools`.
- **`just perf-scene`** (`tools/perf/scene.py`; needs a running dev-tools game, `cargo run
  --release --features dev-tools -- --resolution 1280x720`, picked by `BONGBONG_DEV_PORT`): the
  default arena with six enemies and longwater at seed `0xB0B5`, each restarted and stepped eight
  times by 60 frames in lockstep with seat 0 driving and firing, `status.frame_stages.draws` read
  after every step - twice over, and the run fails unless the two passes agree. `fx_max_particles`
  and `fx_max_pieces` are pinned to 0 while the counters are read (`fx.rs` and `pieces.rs` draw
  from `rand::rng()`) and put back for the timed half, where each scene runs twelve seconds in real
  time for its stage averages, pace and one frame's draws. `counters:` lines are the same on every
  run and across restarts of the game at one window size; `timing:` lines are that machine's.
- **`just perf-paint`** (`mapshot --paint-bench FRAMES MAP..`, `thumbnail::paint_bench`): the
  painters with no window - `Game::paint_field` on one `CpuCanvas` over 60 frames of a live seeded
  round (the map's own enemies, nobody at the controls, ten steps between frames), ms a frame and a
  hash of every frame's pixels, the same on every run.
- **`just perf-field`** / **`just perf-field-wasm`**: the probe on `maps/perf/dense-250.toml` - 250
  x 250 cells, the format's limit, 15,004 tiles and deep-water cells, 5,256 cells of tall grass and
  14,539 of road in 31 x 31 town blocks, written by `tools/gen_perf_map.py` (run it again rather
  than editing the file) - three one-minute rounds of `defend` at seed 1000, printing `timing:` and
  `phases:`. The wasm recipe builds the probe for `wasm32-unknown-emscripten` and runs it under
  node (`tools/perf/field_wasm.sh`; `RUSTFLAGS` replaces the game's link arguments with
  `NODERAWFS`, so it builds its own dependency set): V8 on a desktop, which is not Safari and not
  a phone. The fixture is under `maps/perf/` so that no ceiling applies to it (`just
  probe-fixtures` globs `maps/test/*.toml`, `just probe-fields` names its maps); `just thumbnails`
  does render it, as an 8000 x 8000 PNG.
- **`just probe-wasm-parity`** (`tools/perf/wasm_parity.sh`): does a seed play the same round in
  wasm as natively. Both probes (native release, and the `perf-field-wasm` build under node) run
  with **`--state-hash-every 1`**, which prints a `STATE round= seed= frame= rng= tanks= bodies=
  world=` line per frame (and frame 0, after `init`) from `Game::state_hash` - the RNG's next
  draw, every tank's pose/damage/shield/shells in slot order, every body's velocity, and the
  entity count, clock, wrecks and bongs, floats by their bits. The `STATE` and `ANOMALY` lines must
  match; on a difference it prints the first pair, which names the frame and the part. Default:
  600 frames of `defend` at seed 1000 on the default arena, longwater and `maps/perf/dense-250`;
  arguments replace that with one probe run. The flag works on any probe run and with `--rounds`,
  so it also diffs two native builds (a refactor that must not move a round). Not in CI: it needs
  emsdk and a second, wasm dependency build.
- **Baseline, 2026-11-19** (an M3 Pro, release builds, the window 1280x720 on a 120 Hz display
  with vsync; the tree before any render-pipeline work):

  | Reading | default arena | longwater |
  |---|---|---|
  | blits a frame (particles and pieces off) | 2,015 - 2,122 | 1,798 - 2,183 |
  | rects a frame | 804 - 3,111; 21,164 and 23,576 with a blast on screen | 634 - 2,109 |
  | shapes a frame | 6 - 14 | 4 - 6 |
  | texture switches a frame | 73 - 106 | 12 - 24 |
  | blend / shader / target scopes a frame | 4 - 7 / 1 - 5 / 2 | 4 - 5 / 0 - 1 / 2 |
  | one real-time frame, particles and pieces on | 2,431 blits, 31,299 rects, 432 switches | 2,270 blits, 3,680 rects, 24 switches |
  | frame p99 / max, ms (the display's frame is 8.33) | 8.7 - 10.4 / 9.4 - 11.8 | 8.7 - 10.6 / 8.7 - 12.4 |
  | steps a frame, 0/1/2/3/4 of 600 | 292/308/0/0/0 | 290/310/0/0/0 |
  | `world` / `chrome` / `post` stage, ms a frame | 2.4 - 2.8 / 1.3 / 0.15 | 0.7 - 0.8 / 1.6 / 0.12 |
  | `Game::paint_field` on the CPU canvas, ms a frame | 11.3 (1536 x 768 px) | 53.4 (3584 x 2016 px) |

  With the wall pieces on and the particles off the default arena's switches run 324 to 1,304 a
  frame: the pieces alternate with everything around them.

- **After the draw-call stage (BB-164), same day, same machine and scene**, every lockstep
  screenshot byte for byte the baseline's:

  | Reading | before | after |
  |---|---|---|
  | texture switches a frame, default arena (particles and pieces off) | 73 - 106 | 71 - 84 |
  | the same with the wall pieces on | 169 - 1,304 | 72 - 84 |
  | texture switches a frame, longwater | 12 - 24 | 11 - 23 |
  | blend scopes a frame, default / longwater | 4 - 7 / 4 - 5 | 3 - 6 / 3 - 4 |
  | an arena with margins under a clear sky (classic, 1280x900): blits / target scopes | 2,958 / 4 | 2,022 / 2 |
  | `tuning()` calls in one paused frame, default / longwater | 2,376 / 1,075 | 209 / 161 |

  The rectangles draw from a white block in the walls sheet's texture (`render/shapes.rs`), which
  is what took the pieces' alternation away. The sheets are not packed into an atlas: a sheet moved
  into a larger texture has a turned sprite's samples round onto other texels, so the picture
  changes. The switches left are one sheet after another in the y-sorted walk.

  | `Game::update`, ms a tick (`defend`, seed 1000) | native | wasm (node 24) |
  |---|---|---|
  | default arena | 0.10 | 0.19 |
  | longwater (112 x 63) | 0.32 | 0.47 |
  | dense-250, its 16 enemies, one seat | 1.9 - 2.0 | 2.9 - 3.4 |
  | dense-250, 32 enemies | 3.2 | - |
  | dense-250, 32 enemies, eight seats | 3.3 | 5.9 |
  | dense-250, 32 enemies, eight seats, `advance` | 3.8 | 7.4 |

  The dense map's tick by group, native, ms: `terrain` 0.74, `weapons` 0.43 (`resolve_flames`
  builds the set of every standing tile's cell each tick, jets or none), `grass` 0.42 (`grass::tick`
  and the worn floor, per tuft and per mover: 0.9 with 32 enemies), `rams` 0.16 (`ram_props`, per
  hull: 0.9 with eight seats), `enemies` 0.07 (0.64 with 32 enemies awake against eight advancing
  seats; 1.3 in wasm), `fire` 0.06, `nav` 0.03. On longwater the same groups are `terrain` 0.09,
  `enemies` 0.11, `weapons` 0.05, `rams` 0.03. So the per-tile walks are the whole cost of a map at
  the limit and a third of the step on the largest shipped one, and the AI's cost follows the tanks
  awake, not the map. The wasm probe does not play the native round at the same seed (the anomaly
  lines differ on all three maps), so the two columns time rounds of the same kind, not the same
  round.

- **After the scale stage (BB-175), 2026-11-20, same machine**, the window 1280x720: a view of
  part of a field is painted from what stands in it (`tile_index.rs`, `marks.rs`, the grass by
  `partition_point`) instead of by walks of the map. What a frame draws is what it drew - the
  culling already left the same things out - so every draw counter is number for number the
  baseline's (blits, rects, shapes, texture switches and scopes, on the default arena, longwater
  and `maps/perf/dense-250` at seed `0xB0B5`, clear and under storm, snow, night, rain and fog), and
  so is every lockstep screenshot. What moved is processor time, and only on the map at the
  format's limit. Each figure is the median of eight one-second samples of a stage's average while
  a round runs in real time (particles and pieces off); the two builds were run in turn three
  times over, and the three runs are given in order, ms a frame:

  | Reading | before | after |
  |---|---|---|
  | `world`, default arena (shown whole: no rectangle, the plain walk) | 1.01, 1.00, 1.09 | 0.98, 1.00, 1.05 |
  | `world`, longwater (112 x 63) | 0.75, 0.81, 0.81 | (1.46), 0.75, 0.79 |
  | `world`, dense-250 (15,004 tiles) | 0.73, 0.68, 0.71 | 0.71, 0.59, 0.58 |
  | `world`, dense-250 under fog | 1.03, 1.02, 1.06 | 1.05, 0.91, 0.95 |
  | `pictures`, dense-250 (the frame's one tile-by-tile check of the index) | 0.023, 0.021, 0.021 | 0.064, 0.049, 0.050 |
  | `pictures`, longwater | 0.025, 0.027, 0.029 | (0.055), 0.033, 0.033 |
  | frame p99 over the four scenes (the display's frame is 8.33) | 13.5 - 14.3 | 13.6 - 14.3 |

  The first "after" run was taken while another build ran on the machine (its `sim`, which this
  stage does not touch, ran a third to a half over the other runs'); its longwater figures are in
  brackets. So about a tenth of a millisecond less in `world` on dense-250 against three
  hundredths more in `pictures`, and nothing to tell apart on the shipped maps or in the frame
  pace: on a map of a few thousand tiles, reading a view's through the index costs about what
  walking them all did. Not measured: a floor at its mark ceilings (`mark_caps`, about a hundred
  times an arena's on dense-250), which is what `Marks::shown` is for, and the particle store at
  its cap, where a push no longer moves the list.

## Asset pipeline (generated, not hand-drawn)

- Sheets are generated by Pillow scripts in `tools/spritegen/` with fixed seeds: **the tanks by
  `tankdesign/export.py`** (its README; the Vanguard design line) → `scifi_tanks_sheet.png` and its
  light layer `scifi_tanks_glow.png` (33 columns x 60 rows of 40 px cells: four damage tiers of four
  track frames, four wrecks, turrets by tier and recoil pose, the broken turret; the roster five
  times, enemy then players 1-4), `tank_modules.png` + `tank_modules_glow.png` (the weapon modules,
  a row per chassis) and `src/tank_art.rs` - docs/SPRITESHEET_SPEC.md; an unchanged design exports
  byte-identical sheets. `gen_shells.py` → `shells.png`, `gen_walls.py` → `walls_sheet.png`,
  `gen_bullets.py`, `gen_plasma.py`, `gen_props.py` → `props_sheet.png` and the range board's
  `target_sheet.png` (44 px cells, the board drawn 30 % larger than a prop),
  `gen_barrel_explosion.py` → `barrel_explosion.png` (only its scorch row is drawn - every fireball
  is composed, docs/effects.md), `gen_grass.py` → `nature_sheet.png`, `gen_trees.py` →
  `trees_sheet.png`, `gen_missiles.py` → `missile.png` (the flight frames), `gen_portal.py` →
  `portal_sheet.png` (twenty-four three-arm spiral frames in rows of twelve + a bar icon; the spin
  direction and the disc size are the generator's, not the code's; no `snap()`), `gen_towers.py` →
  `towers_sheet.png` (no Pillow: raw PNG bytes; docs/TOWERS_SPEC.md), `gen_crates.py` →
  `crates_sheet.png` and `pickup_glyphs.png` (no Pillow; docs/CRATES_SPEC.md). Run with
  `SPRITE_OUT=static` (the default writes to `assets/sprites`). Specs, one per sheet, in
  `docs/*_SPEC.md` (column/row maps, pivots, palettes).
- **Pillow**: nix's `python3` has no pip. Use `nix-shell -p "python3.withPackages (ps: [ps.pillow])"
  --run "SPRITE_OUT=static python3 tools/spritegen/gen_walls.py"`.
- **One shared palette**: every generator imports `tools/punypalette.py` (sampled from the Puny
  World ground tileset; docs/PALETTE.md). `gen_walls.py` and the tank kit use `PUNY_PALETTE_ALL`
  (extra interpolated greys; the tanks add two deep water/teal steps, `kit.TANK_EXTRA`, and each
  player block its own team ramp, which the check holds block by block). After regenerating run
  **`just check-sheets`**: every sheet on the palette, and no green on anything drawn over the
  ground (walls, props, blasts - green there reads as terrain showing through). Grass and trees are
  the exempt vegetation sheets; `plasma.png`, the crates' symbol inks and `portal_sheet.png` (the P1
  team blues, admitted by `TEAM_SHEETS`) are deliberately off-palette. `snap()` is nearest-colour
  over the whole set, so **shade with an explicit adjacent palette step, never a computed multiple**
  (a darkened sand once snapped to green).
- **Density**: `walls_sheet.png` is authored on a 2 px block grid (16 x 16 effective design pixels
  per 32 px tile - the budget to design against - because tanks draw at 2x and obstacles at 1x;
  `gen_walls.py`'s `px()` writes whole blocks). `gen_props.py` draws each cell at 16 px and upscales
  2x - do not pixelate it again. `nature_sheet.png` is one design pixel per sheet pixel (drawn at
  2.0). `gen_trees.py` uses 48 px cells and expands to 2x2 blocks at the end; `gen_portal.py`
  designs 48 px frames and doubles them to 96.
- **The pickups' crates** are `tools/spritegen/gen_crates.py` → `crates_sheet.png` (a row per kind:
  the crate, four glint frames, damaged, charred) and `pickup_glyphs.png` (the symbols on their
  own), raw PNG bytes, no Pillow (docs/CRATES_SPEC.md); the wood is on the palette, the symbols'
  inks (`PICKUP_INK`) deliberately loud and admitted on those two sheets alone. The tread marks
  have no sheet: they are baked at run time (`wear.rs`, docs/ground-memory.md). A tank's wear is
  the sheet's damage tiers; its smoke and fire are drawn (`damage_stage.rs`).
- **The bongs** are `tools/spritegen/bongs/export.js` → `static/bongs/<kind>/{idle,hurt,hop,attack,
  explosion}.png`, the seven characters drawn by code in `sprites.js` with continuous, keyframed
  animation (docs/BONG_SPEC.md): `node tools/spritegen/bongs/export.js`, Node from the dev shell, no
  packages. Deterministic; off the Puny Palette on purpose, so `check_sheets.py` does not list them.
  Its `CLIPS` frame counts and `lib.rs`'s `BONG_*_FRAMES`/`BONG_*_FPS` must agree.
- **The ground layer is the one third-party, hand-drawn exception**:
  `static/punyworld/punyworld-overworld-tileset.png` and `-desert.png` (see its `SOURCE.md`) are
  retinted copies, one per `map::Theme` - `tools/retint_ground.py` reads the pristine
  `static/punyworld/_original/`, composes the water tiles the pack lacks (`tools/water_tiles.py`)
  and writes both live files, and one per biome made from them (`-valley`, `-floodplain`,
  `-karst`, docs/biomes.md) (idempotent; `BONGBONG_THEME=x` for one), and `gen_grass.py` writes the
  matching `nature_sheet.png`/`nature_sheet_desert.png` the same way (docs/desert-theme.md). Never
  edit a live PNG or `_original/` directly. Everything else was recoloured to match *it*.
  `tools/ground_mask.py` (Pillow) reads the live grass tileset and the tile tables in `src/ground.rs`
  and writes `src/ground_mask.rs`, each road and water tile's pixels as water, bank, dirt or ground
  (docs/biomes.md): run it after a change to those tables or to `water_tiles.py`, never edit its output.
- Before a sweeping art pass, copy the current sheets and generators into
  `static/_backup/<label>-<timestamp>/` and `tools/spritegen/_backup/<label>-<timestamp>/`.
- **Historical, do not run**: `tools/gen_shells.py` (emits the old 3-variant sheet and would regress
  `static/shells.png`), `tools/analyze_turrets.py` and `tools/gen_tanks_candy.py` (read the deleted
  `static/tanks.png`), `tools/resurrect64.py`, `tools/spritegen/gen_ground.py`,
  `tools/spritegen/gen_tanks.py` (the old 32 px tank sheet; refuses to run without
  `GEN_TANKS_OLD_LAYOUT=1`, and would overwrite the shipped one).
