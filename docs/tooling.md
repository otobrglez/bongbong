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
  set `fx_max_particles` to 0 first (`tuning_set`): the particle layer draws from `rand::rng()`, so
  particles differ run to run. A builder screenshot differs run to run too (`ground_seed` is drawn
  per session; water and portals animate on the wall clock): pin both in a scratch build to compare
  the builder pixel for pixel. The `camera` tool pins a zoomed view (`{x, y, zoom}`; a pin outranks
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
  --bin probe -- --scenario afk|advance|brake|circle|defend [--enemies N] [--tank K] [--mission ..]
  [--spawn ..] [--frames N] [--log-every N] [--seed S] [-m map] [--rounds N] [--json-out F]
  [--budget kind=N ...] [--heatmap] [--tuning F] [--crate KIND] [--print-rust]`; `--crate` puts that
  pickup in every weapon crate's place (the fixtures carry none). Built on
  `Game::outcome()`/`tank_snapshots()`/`events()`/`frog_position()` - extend those public accessors,
  not raw world access (a bin sees only `pub`). Per-frame **anomaly checks** on enemies print
  greppable `ANOMALY round= seed= frame= kind= tank= pos=` lines, each seed a replay recipe; kinds
  (`ANOMALY_KINDS`, the single list wiring `--budget`/`--json-out`/`--heatmap`/summaries):
  stale-start, stall, border-stuck, jitter, spin, churn, clustering, wall-grind, bump-rate,
  low-progress, never-arrived, invariant (also players/wrecks and `PhysicsQuarantine`), tank-grind,
  pile-up, offbox-fire (an enemy shot or missile lock on a seat from outside that seat's sight box;
  budgeted at 0 in every recipe; ci.yml runs `just probe-fixtures`, so the justfile holds the only
  copy of the ceilings). A fire tally (`TankSnapshot::shot_at_seat`/`hit_by_seat`) prints beside the
  rams and goes into `--json-out` as `fire`; `seat-hits-offbox` is the reverse unfairness. Sweep
  totals sum every kind (`AnomalyTotals::add`). Thresholds are named consts at the top (the
  spin/jitter windows and `signed_quarter_turn` live in `simulation::debug`, shared with the dev
  server's live turn counters); a new kind extends `TankTrack` + `check_anomalies` +
  `ANOMALY_KINDS`. stall/stale-start are muted for a deliberate hold (firing solution, retreat
  recharge, and the `HOLDS` rows: asleep, a tell, a skid, kept out of a danger, a charge, a stand
  for the FPV swarm - `air`); a state the tank's motion is not its own in (`OUT_OF_ITS_HANDS`: an
  EMP's outage) mutes every check and restarts its windows. `jitter` cannot tell flip-flopping from
  legitimate weaving - corroborate with `--log-every 1` before calling it a regression. Ram counts
  (enemy-pair / into-player) are counters, never budgeted, and saturate through the ram cooldown;
  `tank_contact_seconds` does not. `--rounds N` sweeps seeds base+i and prints only anomalies and
  summaries. A sweep also prints `outcomes:` (won/lost/unfinished, lost-after and won-after mean and
  median), `lulls:` (stretches of at least 3 s with no enemy within reach of a live seat or the frog
  between two frames with one; `lulls` in `--json-out`), `rerolls:` with a trace line per straggler
  re-rolled, `first contact:` (the first engagement, shot at a seat and hit on a seat, mean and
  median) and `timing:` (ms per `Game::update`, flagged in a debug build), carried in `--json-out`
  as `first_contact`/`ms_per_tick`; a single round's header says `map class=field|arena`. On a field
  map a sleeping tank is a deliberate hold, and a sleeping or leashed one is exempt from
  never-arrived, as is a guard keeping its beat (`TankSnapshot::guarding`); a hold within the last
  half second vetoes stale-start. Recipes: `just probe-sweep` (default map), `just probe-waves`,
  `just probe-fixtures` (every `maps/test/*.toml` at a pinned seed against recorded ceilings -
  deterministic, so an exceedance is a real change; re-baseline consciously after a deliberate
  AI/map/tuning change, never bump a ceiling to go green; the probe is built once and the fixtures
  run one per core, each one's output printed in order; CI runs it beside `cargo test --lib`), `just
  probe-fields` (the field-map AI: the study map, longwater and five 56-wide levels, AFK, 10 rounds
  at seed 1000 against ceilings recorded 2026-10-02 and re-baselined the same day for lanes, for the
  margin's window (jitter=30, hedge-maze) and for the shipped maps' growth by 40 % a side
  (border-stuck=8 castle-moat, spin=9 and churn=39 hedge-maze, wall-grind=1 harbor-lights) and once
  more for the play-test pass's armour, shield and rockets (spin=12, churn=43, clustering=11
  hedge-maze, pile-up=8 archipelago, tank-grind=1); not in CI - about four minutes in a debug
  build), `just probe-defend` (release: the perfect defence on longwater and the study map, ten
  seven-minute rounds each at seed 1000, never-arrived 0). `defend` is a perfect defence: every
  enemy within `--defend-reach` px (400) of a live seat or the players' frog is destroyed through
  `Game::debug_kill`, so a round lasts as long as its last straggler and never-arrived counts one
  still out. The `maps/test/*.toml` fixtures are 40 x 22.5 but carry `view = "whole"`, so they stay
  arenas and their ceilings measure the arena's AI; `maps/missions/waves-basic.toml` and
  `hunt-basic.toml` are field maps by size. `maps/test/` is the adversarial fixture corpus (u-trap,
  choke, tight-corridors, frog-block, maze, pockets, the `props` playground, plus `portals` (two
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
  planner), `enemy-frog-unreachable`, `hunt-missing-enemy-frog`, the seat checks
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
  list's two thumbnail renderers side by side. No rustfmt/clippy config is checked in. Dependencies
  build at opt-level 3 in the dev profile (`[profile.dev.package."*"]`), and CI's test job builds
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
  frames no snapshot arrived on. **`rig::Lockstep`** is the same lane with no thread and no clock,
  for a test or a tool: `start` seats a client and `step {ticks}` runs the authority that many ticks
  and lets the replica catch up to the last snapshot they earned, so a networked round replays the
  way `restart` + `step` replays a local one - the room's `now` advances by exactly
  `PHYSICS_FIXED_DT` a tick, so even `Snapshot::server_ms` comes out the same. Its link is perfect
  and nothing is interpolated: delay, jitter and `net::interp` are all dials on *real* time, so they
  stay the threaded rig's.
- **Native runtime tuning**: `cargo run -- --tuning knobs.json` loads a JSON patch and re-applies it
  whenever the file changes (bad file: fails at startup; bad edit: stderr, ignored). The probe takes
  the same flag (echoed in its header and `--json-out`); `--print-rust` renders it as `tunables!`
  rows to paste into `tuning.rs`.
- **`src/bin/mapshot.rs`** (docs/mapshot-prd.md) — map thumbnails: `mapshot maps/x.toml -o x.png`,
  `--out-dir DIR maps` for a batch (`just thumbnails`), `--renderer cpu|gpu` (cpu default:
  `canvas::CpuCanvas`, no window; gpu: a hidden window through the game's own pass-1 stages),
  `--check` (both, prints their difference; `just mapshot-compare` - a `cargo test` cannot open a
  window on macOS), `--scale`, `--seed` (default `0xB0B5`), `--players`/`--tank`/`--tank2`,
  `--no-tanks`, `--plain`, `--no-shadows`. `thumbnail.rs` is the library side: `stage_round` (init +
  **one update**, as the window runs before its first render; no intro, `enemy_count_override =
  Some(0)` under a forced Band plan), `render_cpu`, `compare_pixels`; `render/thumbnail.rs` has
  `load_cpu_sheets`, `GpuSheets`, `render_gpu` and `image_png_bytes`; its tests pin the CPU render
  hash of every `SHIPPED_MAPS` entry (re-baseline consciously after an art/map/tuning change).

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
  inks (`PICKUP_INK`) deliberately loud and admitted on those two sheets alone. `static/tracks.png`
  is static. A tank's wear is the sheet's damage tiers; its smoke and fire are drawn
  (`damage_stage.rs`).
- **The ground layer is the one third-party, hand-drawn exception**:
  `static/punyworld/punyworld-overworld-tileset.png` and `-desert.png` (see its `SOURCE.md`) are
  retinted copies, one per `map::Theme` - `tools/retint_ground.py` reads the pristine
  `static/punyworld/_original/`, composes the water tiles the pack lacks (`tools/water_tiles.py`)
  and writes both live files (idempotent; `BONGBONG_THEME=x` for one), and `gen_grass.py` writes the
  matching `nature_sheet.png`/`nature_sheet_desert.png` the same way (docs/desert-theme.md). Never
  edit a live PNG or `_original/` directly. Everything else was recoloured to match *it*.
- Before a sweeping art pass, copy the current sheets and generators into
  `static/_backup/<label>-<timestamp>/` and `tools/spritegen/_backup/<label>-<timestamp>/`.
- **Historical, do not run**: `tools/gen_shells.py` (emits the old 3-variant sheet and would regress
  `static/shells.png`), `tools/analyze_turrets.py` and `tools/gen_tanks_candy.py` (read the deleted
  `static/tanks.png`), `tools/resurrect64.py`, `tools/spritegen/gen_ground.py`,
  `tools/spritegen/gen_tanks.py` (the old 32 px tank sheet; refuses to run without
  `GEN_TANKS_OLD_LAYOUT=1`, and would overwrite the shipped one).
