# Gameplay verification & stuck-detection tooling — design doc

Status: ALL SIX PHASES LANDED (2026-08-27). See each phase's "Landed"
notes for deviations, and the wave-5 close-out at the end of Phase 6 for
the definitive standing baselines, the budget gate, and the burn-down
list of real findings this tooling produced on day one. Six phases, each independently
landable and verified on its own; Phase 1 (determinism) is the enabler
for everything else and landed first. When a phase lands, update
CLAUDE.md's "Testing & tooling" section and flip that phase's checkbox
here.

- [x] Phase 1 — Seeded, replayable rounds (+ frame invariants) — landed 2026-08-27, see the "Landed" notes at the end of its section
- [x] Phase 2 — `--map` for the probe + adversarial fixture maps — landed 2026-08-27, see its "Landed" notes
- [x] Phase 3 — Static map linter (exhaustive, in `cargo test`) — landed 2026-08-27, see its "Landed" notes
- [x] Phase 4 — Contact-level metrics (wall-grind / bump-rate / low-progress) — landed 2026-08-27, see its "Landed" notes
- [x] Phase 5 — Navigation e2e (path-stretch / never-arrived) — landed 2026-08-27 (§5.1 early, §5.2 same day), see its "Landed" notes
- [x] Phase 6 — Sweep ergonomics (JSONL, budgets, heatmaps, CI) — landed 2026-08-27; see its "Landed" notes and the wave-5 close-out

## Problem

Two recurring gameplay-quality problems have no systematic detection today:

1. **Tanks getting stuck** — against obstacle clusters, the frog, walls, or
   each other; every historical instance (boxed-in spawn spinning, frog-cell
   routing, corner heading flip-flop, engagement pile-ups) was found by
   probe traces *after* someone noticed something looked wrong in play.
2. **Tanks hitting obstacles too much** — scraping/grinding along terrain
   while driving. Today this is literally invisible to tooling: every probe
   anomaly check reasons from kinematics (speed ≈ 0, position near border),
   so a tank grinding along a wall at half speed passes every check.

The existing probe harness (`src/bin/probe.rs`) is the right foundation —
headless, scripted, anomaly-checked, sweepable — but it has five structural
gaps: findings aren't reproducible (no RNG seed), obstacle contact isn't
measured, hand-authored maps (now the *only* terrain source) have no static
verification, no scenario asserts navigation as a goal, and sweep output
isn't machine-readable or budget-gated for CI.

## Goals

- **Reproducibility**: any flagged round is replayable exactly — headless
  for frame-level inspection, and in the windowed game to watch the same
  layout. A probe finding becomes a permanent `(map, scenario, seed)` repro.
- **Ground truth over symptoms**: measure actual tank↔terrain contact from
  rapier's narrow phase, plus commanded-vs-real velocity, so "hitting
  obstacles too much" and "trying to move but failing" are direct
  observations, not inferences.
- **Exhaustive static verification**: for every supported map, *prove* (not
  sample) connectivity, reachability of pickups/frog, spawn-band viability,
  and consistency between the pathfinding grid and the physics colliders —
  in `cargo test`, in milliseconds.
- **Task-level e2e**: assert "an enemy that has a path to the player
  actually gets there in bounded time", not just "it didn't sit still".
- **CI-able sweeps**: machine-readable per-round results, explicit anomaly
  budgets with a nonzero exit on breach, and heatmap triage that turns
  "tanks sometimes stick" into "cell (14,3) accounts for 80% of stalls".
- Preserve the architecture principles that made the probe possible:
  `simulation.rs` stays renderer-free, the probe sees only `pub` API
  (`Outcome`/`TankSnapshot`-style accessors, never `world`/`Entity`), and
  the AI stays snapshot-only (docs/physics-engine-design.md).

## Non-goals (this pass)

- **Cross-platform / cross-build bit determinism.** Seeded replay promises
  "same binary, same platform, same inputs → same round". rapier's float
  solver isn't cross-platform-deterministic without its
  `enhanced-determinism` feature and we don't need it for a local repro
  workflow. Likewise probe↔windowed runs are *not* frame-exact twins (the
  windowed game steps with variable render dt; see Phase 1's "what a seed
  does and doesn't promise").
- **Testing rendering, shaders, or feel.** That stays on playtesting and
  the `run` skill; this doc is logic/behavior only, same as the probe.
- **A general input-recording/replay system.** Scripted `Scenario`s plus a
  seed make one unnecessary — the entire input stream is already a pure
  function of `(scenario, frame)`.
- **Redesigning AI behavior.** The tooling observes and flags; fixes to
  `ai.rs` happen as their own changes, pinned afterwards by the regression
  path below.
- **New dependencies.** No `serde_json` (JSONL is hand-emitted — the schema
  is flat numbers/short strings), no test frameworks, no rayon. Everything
  builds on `clap`, `rand`, `rapier2d`, `hecs` already in Cargo.toml.

## Current state (what exists, what's missing)

Working today, and load-bearing for this design:

- `simulation.rs` is fully renderer-independent; `Game::update(Input, dt,
  w, h)` drives a whole round headlessly. The probe (`src/bin/probe.rs`)
  scripts input per frame, runs five per-frame anomaly checks per enemy
  (`stale-start`, `stall`, `border-stuck`, `jitter`, `clustering`), prints
  greppable `ANOMALY` lines, and sweeps N independent rounds with
  `--rounds`. Multiple fixed bugs cite it as the discovery tool (see
  `pathfind::Grid::boxed_in`, `Grid::same_cell`, `nearest_open`,
  `simulation::engage::EngageRing`'s doc comments).
- `pathfind.rs` has real unit tests and the stuck-adjacent helpers
  (`boxed_in`, `blocked_ahead`, `nearest_open`);
  `battlefield::relocate_unusable_spawns` already audits spawns against the
  finished layout once per round.
- `physics.rs` wraps rapier's `PhysicsWorld`, which exposes everything the
  contact metrics need (`contact_pairs_with`, `ContactPair::
  has_any_active_contact`, per-contact solver `impulse`) — `touching()`
  already uses the pair API for ram damage.
- Precedent for simulation-level tests exists (`shell_sweep_tests` in
  simulation.rs; `tile_seam_tests` in battlefield.rs).

The five gaps, with the evidence:

1. **No seed.** `Game::init` and `Game::update` each open a fresh
   `rand::rng()` (`ThreadRng`) and thread `&mut ThreadRng` through every
   helper. A flagged sweep round is gone the moment it prints. The probe's
   own doc comment says it: "not a reproducible test oracle".
2. **No contact data.** `TankSnapshot` carries position/velocity/damage/
   ammo/wreck — nothing about touching terrain, and not the *commanded*
   velocity either, so intent-vs-outcome can't be computed externally.
3. **No static map verification.** Since the procedural battlefield was
   removed, `maps/*.toml` is the only terrain source, authored by hand in
   the editor — and nothing checks that a map is connected, that pickups/
   frog are reachable, or that the pathfind grid and the physics colliders
   agree about what fits where. The grid/physics mismatch class is exactly
   what produced the "stuck near the frog" bug (see the grid-build comment
   in `Game::update`).
4. **No navigation assertion.** All scenarios exercise combat emergently;
   nothing fails when an enemy with a perfectly good path simply never
   arrives (it only fails if the tank also happens to sit still long enough
   to trip `stall`).
5. **Sweep output is human-only.** Totals print as prose; there's no
   per-round record, no budgets, no exit code, no way to hand results to a
   script or CI gate, and triage still means reading `ANOMALY` lines one at
   a time.

---

## Phase 1 — Seeded, replayable rounds

The single highest-leverage change: convert the probe from an exploration
tool into a test oracle, and make every future phase's findings replayable.

### 1.1 Where the RNG lives

`Game` gains three things (simulation.rs):

```rust
/// CLI-provided fixed seed (`--seed`, main.rs / probe.rs). When `None`,
/// `init` draws a fresh random seed each round. Same "set once before the
/// first `init`, persists across restarts" convention as
/// `enemy_count_override` — so a seeded windowed game replays the *same*
/// round on every restart, which is exactly the repro loop we want.
pub seed_override: Option<u64>,
/// The seed this round actually ran with (whether overridden or drawn
/// fresh), so callers can report/replay it — see `round_seed()`.
round_seed: u64,
/// The round's single RNG stream, seeded in `init`. `None` only before
/// the first `init` call — same convention as `player`/`frog`. Held as an
/// Option so `update` can `take()` it into a local and put it back at the
/// end, keeping every existing `&mut rng` call site untouched and
/// sidestepping partial-borrow conflicts entirely.
rng: Option<SmallRng>,
```

plus `pub fn round_seed(&self) -> u64`.

`init` starts with:

```rust
let seed = self.seed_override.unwrap_or_else(|| rand::rng().random());
self.round_seed = seed;
let mut rng = SmallRng::seed_from_u64(seed);
// ... existing body, unchanged, using `rng` ...
self.rng = Some(rng);   // at the end
```

`update` replaces its `let mut rng = rand::rng();` with
`let mut rng = self.rng.take().expect("rng seeded in init");` and restores
`self.rng = Some(rng);` at the end. The three early `return`s in `update`
(pause, paused-frame, post-round restart countdown) all happen *before* the
RNG is currently created, so take/put-back brackets the rest of the body
cleanly — but the put-back must be audited if anyone later adds an early
return below that point (a `debug_assert!(self.rng.is_some())` at the top
of `update` catches a missed restore on the very next frame).

Notes:

- `SmallRng` is `rand::rngs::SmallRng` (unconditionally exported in rand
  0.10; `SeedableRng::seed_from_u64` runs SplitMix64 internally, so even
  adjacent seeds like `base + round` are well-decorrelated).
- The one remaining `ThreadRng` use inside the game is the entropy draw for
  an unseeded round's seed — one `u64`, after which everything flows from
  the `SmallRng` stream.
- `ground::build` already takes `seed: u64` and is internally deterministic
  (hash-per-cell); `init` draws that seed from `rng`, so the ground layer
  becomes seed-stable for free.

### 1.2 Signature migration

Every `&mut rand::rngs::ThreadRng` parameter flips to
`&mut rand::rngs::SmallRng` — a mechanical type swap, no logic changes.
Current inventory (re-grep before starting: `grep -rn "ThreadRng" src/`):

- `battlefield.rs`: `sample_clear_position`, `spawn_from_map`
- `ai.rs`: `Ai::think`, the two act/wander helpers taking `rng`, and the
  `Brain.rng: &'a mut ThreadRng` field
- `simulation.rs`: the ~10 free helpers (`frog_hop_target`,
  `respawn_from_slots`, `roll_track_distortion`, `explosion_hit_obstacle`,
  the fire/spawn helpers, …)

Deliberately a concrete type, not `&mut impl Rng`: `Brain` stores the
reference in a struct field, so a generic would push a type parameter
through `Brain` and every behavior-tree function for zero benefit — there
will only ever be one RNG type in the game.

**One stray inline call must be fixed or replay silently breaks:**
`roll_wreck_col` (simulation.rs) calls `rand::rng().random_range(..)`
inline. It gains an `rng: &mut SmallRng` parameter like its neighbors.
Post-migration invariant, worth a comment and a grep in review:
`rand::rng()` appears exactly twice in `src/` — the seed draw in
`Game::init` and the editor's cosmetic `ground_seed` (`editor.rs`, dev-only
feature, deliberately outside the game's seeded stream).

### 1.3 The iteration-order hazard (this is the subtle part)

Seeding the RNG is *not sufficient* for replay. Two ordering sources must
also be pinned:

1. **`MapFile::cells` is a `HashMap<String, CellObject>`**, and
   `battlefield::spawn_from_map` iterates `map.iter_cells()` while (a)
   consuming RNG per Wood tile (`OBSTACLE_WOOD_FLAMMABLE_CHANCE` roll) and
   (b) spawning obstacle entities. `HashMap` iteration order varies per
   process (randomized hasher), so both the RNG stream *and* the hecs
   entity-creation order currently differ run-to-run even with a fixed
   seed.

   Fix at the source so every consumer is covered at once:
   `MapFile::iter_cells` collects and sorts by `(row, col)` before
   yielding. Maps are a few hundred cells; the collect+sort cost is
   irrelevant, and it also makes `frog_cell`/`start_cell`
   ("first match wins" over a hand-edited file with duplicates) and
   `spawn_from_map`'s `road_cells` order stable. Do **not** switch `cells`
   to `BTreeMap` instead — the keys are `"col,row"` strings and would sort
   lexicographically (`"10,2" < "2,3"`), which is a trap; sorting the
   parsed tuples is the correct order.

2. **hecs iteration order** is deterministic *given the same sequence of
   spawn/despawn operations* — which (1) restores. After that, every
   per-frame `world.query()` loop that consumes RNG per entity (the enemy
   `Ai::think` loop, track distortion, frog logic) draws in a reproducible
   order.

Audit of the remaining `HashMap`/`HashSet` uses on the simulation path,
for the record (all safe today, listed so a future change knows the rule):
`enemy_indices` is sorted into `engaged` before any order-dependent use
(the comment on that sort already explains why); `excluded`/`hit_alerted`/
`wall_cells`/`material_variant`/`EngageRing::choice` are lookup-only;
`material_variant` is *built* by iterating the `MATERIALS` const slice
(deterministic). **The rule going forward: never iterate a HashMap/HashSet
on the simulation path where the loop body consumes RNG, spawns entities,
or breaks ties — sort first.**

### 1.4 What a seed does and doesn't promise

- probe ↔ probe, same build: **bit-exact** — same layout, same AI
  decisions, same outcome, byte-identical trace. The probe steps `update`
  with a constant `DT = 1/60` equal to `PHYSICS_FIXED_DT`, so the
  accumulator fires exactly one physics step per frame.
- windowed ↔ probe, same seed: **identical round setup** (map terrain,
  ground cosmetics, spawn positions, chassis/speed rolls, frog placement)
  but divergent evolution over time, because the windowed game feeds real
  render `dt` into `update` and AI timers/physics stepping quantize
  differently. This is still the payoff that matters: watch the exact
  layout that flagged, with the same starting conditions.
- across rustc/dependency upgrades: no promise (float codegen and `rand`
  algorithm changes can shift streams). Pinned regression tests (below)
  assert *behavioral* facts ("reaches the player", "no stall anomaly"),
  not exact trajectories, so they survive benign drift.

### 1.5 CLI surface

- `probe`: `--seed <u64>`. Single round: uses it directly. Sweep
  (`--rounds N`): round `i` runs with `base.wrapping_add(i)`; unseeded
  sweeps draw a random base once and derive the same way. **Every round
  is replayable either way** because the effective seed is printed:
  `run_round` reports it in the round-result line and `report()` includes
  `seed=0x{:016x}` in every `ANOMALY` line (add the round seed to
  `report`'s parameters). The final summary repeats the seeds of flagged
  rounds so they can be replayed without scrolling.
- `bongbong` (main.rs): `--seed <u64>` → `game.seed_override`. With the
  override set, the R-key/auto restart replays the identical round —
  that's the intended debugging loop, and the flag's doc comment should
  say so.

### 1.6 Determinism smoke test (the guard that keeps this from rotting)

A `#[cfg(test)]` in simulation.rs — this is the important deliverable of
the phase, because iteration-order regressions are silent otherwise:

```text
determinism_two_runs_agree:
  for each of 2 seeds:
    run Game::init + 600 frames of update twice (fixed dt = 1/60,
    default Input), collecting tank_snapshots() every 60 frames
  assert both runs' snapshot sequences are identical
  (positions bit-equal, ammo/damage/wreck equal)
```

600 frames crosses spawn, patrol, alert, engagement, and firing — enough
to sweep every RNG consumer. If someone later adds an unsorted-HashMap
iteration or a stray `rand::rng()`, this fails within one CI run instead
of quietly poisoning every future sweep. (Measured: ~6.5s in a debug
build — the doc's original "well under a second" guess was wrong; 2400
simulated frames of debug-profile physics + per-frame A* grid rebuilds
dominate. Accepted as the price of the guard rather than trimmed.)

### 1.7 Frame invariants (small add-on while we're in the probe)

Per-frame sanity assertions in `check_anomalies`, reported as
`kind=invariant` ANOMALY lines — cheap, and with seeds every violation is
a perfect repro of a hard bug:

- `position.x/y` or `velocity.x/y` is NaN/infinite
- position outside `[-50, width+50] × [-50, height+50]` (escaped the
  walls — tunneled or blasted through)
- speed above `INVARIANT_SPEED_MAX` (a named const ≈ 800 px/s: comfortably
  above `TANK_SPEED` 220 × `SPEED_BOOST_MULTIPLIER` plus any legitimate
  knockback spike, so it only trips on solver explosions)

These apply to the player too (unlike behavior anomalies — physics sanity
isn't explained by the script).

**Phase 1 verified by:** the smoke test above; running the same
`--seed`ed probe command twice and diffing stdout (identical); running
`bongbong --seed` and confirming restart reproduces the layout; a 30-round
sweep confirming per-round seeds print and replay.

**Landed (2026-08-27).** All of 1.1–1.7 as designed, plus verification:
the smoke test passes; two same-`--seed` probe runs diff byte-identical;
an *unseeded* run replayed byte-identically from its printed seed; and a
flagged sweep round (round 28, seed `0x404`) replayed with `--rounds 1
--seed 0x404` reproduced both its anomalies at the same frames and
positions to the decimal. Deviations from the plan, for the record:

- The take/put-back sketch missed that `update`'s post-round early-return
  branch itself consumes RNG (`roll_wreck_col` while wrecks burn on the
  end screen). That branch borrows `self.rng.as_mut()` in place instead
  of taking — field-disjoint from the `world` query borrow, and NLL ends
  it before the branch's own `self.init(..)` restart call.
- `roll_wreck_col` (the stray-inline-`rand::rng()` fix) is called from
  both that branch and the main flow, so it gained the `rng` parameter as
  planned but with two differently-sourced call sites.
- The smoke test costs ~6.5s (debug build), not the predicted sub-second
  — noted at §1.6, kept as-is.
- First seeded baseline sweep (`afk --enemies 4 --frames 900 --rounds 30
  --seed 1000`): 22/30 rounds flagged — stale-start=8 stall=11
  border-stuck=1 jitter=19 clustering=22 invariant=0. That is the Phase 6
  budget baseline to beat, and `--seed 1000` makes this exact sweep
  re-runnable for comparison after any AI change.

---

## Phase 2 — `--map` for the probe + adversarial fixture maps

### 2.1 `--map`

`probe.rs` currently hardcodes the embedded default map. Add
`#[arg(short = 'm', long = "map", value_parser = parse_map)]` exactly like
main.rs (load eagerly, fail fast), defaulting to the embedded
`default.toml` as today. Print the map name in the run header and carry it
into Phase 6's JSONL. While in there, move main.rs's bin-private
`DEFAULT_SCREEN_WIDTH`/`DEFAULT_SCREEN_HEIGHT` statics into lib.rs and use
them from both main.rs and the probe (replacing probe's own duplicate
`WIDTH`/`HEIGHT` consts) so the two can't drift.

### 2.2 The fixture corpus: `maps/test/`

Hand-authored in the existing editor (`cargo run --features map-editor --
--editor`), committed, one map per known failure *class* — each is a trap
built from a bug we've already fixed once, so regressions walk straight
into it:

| map | provokes | historically |
|---|---|---|
| `u-trap.toml` | a cul-de-sac opening toward the enemy spawn band | stuck-escape / committed-heading overrides |
| `choke.toml` | one corridor (barely above clearance) dividing the field | funneling, pile-ups, `clustering` |
| `tight-corridors.toml` | long corridors exactly at clearance width | wall-grind (Phase 4's target metric) |
| `frog-block.toml` | frog parked in a corridor mouth | the grid-didn't-know-about-the-frog stuck bug |
| `maze.toml` | dense right-angle maze | heading jitter vs. legitimate weaving (the known `jitter` blind spot) |
| `pockets.toml` | almost-sealed pockets in the spawn band | `boxed_in` spawns / `relocate_unusable_spawns` |

Conventions: every fixture places an explicit `Start` cell and a `Frog`
cell (so nothing about its terrain depends on random fallback placement),
and sets the map's `tanks` count so runs are comparable. `maps/` is not
bundled into releases (`dist-workspace.toml` includes only `static/`;
the shipped default is `include_str!`-embedded), so fixtures cost end
users nothing.

**Phase 2 verified by:** `probe --map maps/test/<each>.toml --scenario afk
--rounds 10` runs clean end-to-end; fixture-by-fixture baseline numbers
recorded for Phase 6's budgets.

**Landed (2026-08-27).** `--map` (+ `NamedMap` so the header names the
battlefield), `DEFAULT_SCREEN_WIDTH/HEIGHT` moved to lib.rs, and all six
fixtures committed. Verified: every fixture loads, spawns, and fights
(tight-corridors traced end-to-end: enemies entered the tube from both
ends, drove its single open grid row, and finished an AFK player at frame
237); the u-trap start cell places the player at exactly (640, 512);
`cargo test --lib` stays green. Deviations and findings:

- Fixtures were emitted by a one-off generator script (scratchpad-only,
  not committed) rather than clicked out in the editor — the k≥6
  corridor-clearance arithmetic was much safer done programmatically.
  The files are inline-table TOML with explanatory `#` header comments;
  an editor re-save would drop those comments, so edit fixtures by hand
  (or regenerate), don't round-trip them through the editor.
- The probe caught its first fixture defect before the corpus even
  shipped: maze's rails at rows 4/18 left the only route north of the
  top rail inside the 40px border margin, flagging `border-stuck` ×10 as
  pure routing artifact. Rails moved to rows 5/17 (which also tightens
  both lanes to exactly k=6): border-stuck 10 → 0, jitter/clustering
  preserved. Fixture design lesson recorded: keep at least one legal
  route outside `BORDER_MARGIN` of every boundary, or the border-stuck
  check measures the map, not the AI.
- Baselines (afk, map-default tank counts, 1800 frames, 10 rounds,
  `--seed 1000` — rerunnable verbatim). Measured twice the day they
  landed: first on the six-kind detector set, then again after the
  `spin`/`churn` detectors (and the `circle` scenario) were added
  concurrently in probe.rs — every original-kind count replayed
  *identically* across the two builds, a live cross-check that detectors
  are pure observers of a seed-determined round. Current (eight-kind)
  numbers:

  | fixture | flagged | signature |
  |---|---|---|
  | u-trap | 6/10 | stale-start=4 stall=1 jitter=3 churn=1 clustering=1 |
  | choke | 6/10 | stale-start=3 border-stuck=1 jitter=2 churn=4 |
  | tight-corridors | 2/10 | churn=1 clustering=6 |
  | frog-block | 5/10 | stale-start=1 jitter=2 churn=2 clustering=6 |
  | maze | 8/10 | stall=1 jitter=9 spin=3 churn=17 clustering=17 |
  | pockets | 6/10 | stale-start=4 jitter=2 churn=2 |

  Zero `invariant` hits anywhere. Each fixture provokes a distinct
  signature matching its intent (maze → weaving jitter/spin/churn plus
  entrance funneling — it's also the loudest map for the two new
  detectors; tight-corridors/frog-block → clustering at the choke;
  u-trap/pockets → spawn-adjacent stale-starts). The stale-starts and
  stalls across u-trap/choke/pockets are the standing AI findings to
  chase with these seeds, not fixture defects.

---

## Phase 3 — Static map linter (exhaustive verification)

Terrain is static and finite (~27×15 cells at `PATHFIND_CELL_SIZE` = 48px
on a 1280×720 field), so unlike AI behavior it can be verified
*exhaustively* — every claim below is a proof over the whole grid, not a
sample. This is the "mathematical" layer of the tooling.

### 3.1 Same code path, or it verifies the wrong thing

The linter must see exactly the grid the AI steers by. Extract the grid
construction in `Game::update` (the `Grid::build(width, height,
PATHFIND_CELL_SIZE, max_tank_avoidance_radius(), obstacles ⧺ frog)` call)
into `pub(crate) fn Game::nav_grid(&self, width: f32, height: f32) ->
Grid`, called by `update` each frame and by the linter once. A parallel
reimplementation in the linter is explicitly rejected — it would drift.

Lint setup is simply a headless round: `Game::default()` + `game.map = …`
+ `seed_override = Some(FIXED)` (Phase 1) + `init(1280, 720)`. Init is
cheap (the probe does this per round already), and using real init means
the linter sees seam-widened wall colliders (`tile_hull_half_extent`),
the frog's actual placed cell, and map-driven pickup slots without
duplicating any spawn logic.

### 3.2 The checks

New module `src/maplint.rs`, `pub fn lint(game: &Game, width, height) ->
Vec<LintFinding>` (`pub` so a later probe `--lint` flag can call it across
the bin/lib boundary), `LintFinding` an enum with severity (`Error` /
`Warning` / `Info`) and `Display`:

1. **Connectivity** (flood fill over open cells): the component containing
   the player start cell (or the center-fallback cell when the map places
   no `Start`) is the *playfield*. `Error` if the frog cell or any pickup
   slot's cell is outside it; `Warning` for any other open component of
   ≥ 4 cells (likely authoring mistake; smaller enclosed slivers are
   `Info` — sometimes decorative).
2. **No boxed-in open cells**: `Error` for any open cell where
   `grid.boxed_in(center)` — technically open, unusable in practice, and
   a magnet for knocked-back tanks (`nearest_open` would just relocate
   the problem).
3. **Spawn-band capacity**: enemies spawn by rejection sampling in the
   border band (`ENEMY_SPAWN_MARGIN_MIN`/`MAX`, the clearance predicate in
   `Game::init`'s enemy loop). Count band cells that are open, in the
   playfield component, and clear of obstacles per that same predicate;
   `Error` if fewer than the map's `tanks` count (or `ENEMY_COUNT_MIN`),
   `Warning` if under `ENEMY_COUNT_MAX` — the sampler would degrade into
   its attempt cap and cram tanks in anyway (its documented worst case).
4. **Planner/physics agreement** (the stuck-tank generator): for every
   *open* cell, test a worst-case tank square (half-extent =
   `max_tank_avoidance_radius()`) centered on the cell center for AABB
   overlap against every obstacle collider (position ±
   `tile_hull_half_extent`, i.e. the seam-widened extents physics really
   uses) and the frog's collider. Overlap ⇒ `Error`: the grid tells the
   AI "drive here" and the solver says no — precisely the mismatch class
   behind the frog bug. (The converse — blocked cell that's physically
   fine — is expected conservatism from the margin; not reported.)
5. **Narrow corridors**: `Info` for open playfield cells none of whose
   open neighbors have another open neighbor on the same axis (i.e.
   single-cell-wide passages) — legal, but scrape-prone; Phase 4's
   wall-grind numbers on these maps get read with that in mind.

### 3.3 Where it runs

`#[cfg(test)] mod map_lint_tests` (in maplint.rs): iterate
`maps/*.toml` + `maps/test/*.toml` from disk (`cargo test` runs at the
crate root). Two tiers, because `maps/` contains scratch files:

- A `SUPPORTED_MAPS` const list (`default.toml` + whatever is considered
  shipped) must produce **zero `Error` findings** — test fails otherwise.
- Everything else is lint-and-print only (visible with `--nocapture`),
  so a half-finished editor session can't break the build.
- Fixture maps assert their *intended* profile (e.g. `choke.toml` must
  contain a `NarrowCorridor` finding — a fixture that stops provoking is
  itself a bug in the fixture).

**Phase 3 verified by:** seeding a deliberately broken map (sealed pickup,
boxed-in cell, tank-count 10 on a nearly-full map) and watching each check
fire; `cargo test` green on `SUPPORTED_MAPS`; runtime for the whole maps
directory under a second.

**Landed (2026-08-27).** Implemented by a subagent fork (which stalled at
8/11 tests mid-diagnosis and was finished inline); its wave partner
delivered §5.1's `Grid::path_cost` early. Architecture exactly as §3.1
demanded — and that choice immediately paid off, twice:

- **The linter cannot disagree with the AI, so it caught the AI's world
  changing mid-landing.** The parallel AI-mechanics session changed
  `Grid::build`'s rasterization from any-overlap to *cell-center-inside-
  reach* (killing a hidden extra ~half-cell of margin per side — the root
  cause of the spinning/dancing tanks: sealed pockets → failed
  reachability → per-frame wander re-rolls; their before/after at seed
  0x5eed0001, 30 rounds: advance spin 34→5 churn 96→19, afk spin 5→2
  churn 44→25, circle spin 19→14 churn 83→58). The lint tests failed the
  moment the rule changed, and the failure dumps were what root-caused it
  here independently before the other session's heads-up arrived.
  Absorbed: `sealed_ring` resized to a 5×5 ring (exactly one open center
  per axis ⇒ a genuine single boxed cell), a wider `sealed_vault` kept
  for the pickup/frog burial tests (whose ~128/~150px approach reaches
  need the deeper interior), **maps/test/pockets.toml regenerated** with
  the small ring so the on-disk fixture still carries the boxed-cell
  pathology, and all fixture headers/generator prose re-derived
  (guaranteed grid-passable is now ≥5 cells of wall separation; 3–4 is
  48px-alignment-dependent; choke/tight-corridors/frog-block lanes are
  now two cells wide, so `NarrowCorridor` rightly stays silent for them
  and their provocations are recorded as behavioral).
- **Day-one real findings in the shipped map.** `default.toml` carries a
  strip of pickups along its top edge that no playfield cell comes
  within approach reach of, and — the big one — a border band with
  **zero** cells passing the enemy-spawn legality predicate: every enemy
  spawn on the default map degrades to `sample_clear_position`'s
  documented attempt-cap fallback, a very plausible driver of that map's
  stale-start/stall baseline. Real map debt, recorded, not "fixed" by
  loosening the checks. *(Resolved 2026-09-03: the distance-based
  predicate was replaced by `battlefield::enemy_spawn_legal` - nav-grid
  usability plus a tank-box-vs-wall-box separation - shared by the
  sampler and the linter; the sampler no longer returns a rejected
  sample on its attempt cap, and `relocate_unusable_spawns` audits
  blocked cells too. Root cause of enemies spawning inside walls.)*

Deviations from the plan:

- The zero-error `SUPPORTED_MAPS` gate became
  `supported_maps_no_new_errors` ratcheting against `KNOWN_ERROR_KINDS` —
  an allowlist of error *kinds* (not counts): default.toml is under
  active hand-editing (its unreachable-pickup count grew from 5 to 12
  while the gate was being written), and count-ratcheting a live canvas
  just flaps. A brand-new error class (boxed-in cell, planner/physics
  mismatch, unreachable frog) still fails instantly. Tighten back to
  per-kind counts once the map settles.
- `tight-corridors.toml`'s profile now *includes* `SpawnBandTooTight`
  (its 21-tile rails leave ~3 legal band cells for 4 tanks) — spawn
  pressure is part of that fixture's identity, asserted as the only
  error kind it may carry.
- Suite at this point: **36 tests green** (linter + path_cost + the
  AI-session's weapon-queue tests). Standing caveat: the Phase 2
  baseline table predates both the center-rule change and the probe's
  new deliberate-hold suppression (stall/stale-start muted for tanks
  that are firing/aiming/recharge-waiting), so every recorded anomaly
  baseline is stale until the wave-5 re-baseline on the final binary.

---

## Phase 4 — Contact-level metrics

### 4.1 Physics: expose narrow-phase truth

`physics.rs` gains one read-only query, built on the same pair API
`touching()` already uses:

```rust
pub struct ContactStats {
    pub touching_static: bool,   // wall / obstacle / frog (fixed bodies)
    pub touching_tank: bool,     // another tank's hull (dynamic bodies)
    pub max_impulse: f32,        // strongest solver impulse among those contacts
}
pub fn contact_stats(&self, body: RigidBodyHandle) -> ContactStats
```

Implementation: take the hull collider (`collider_of(body)` — the first
collider is the solid hull by construction), iterate
`world.contact_pairs_with(hull)`, keep pairs with
`has_any_active_contact()`, classify the *other* collider's parent body
(`fixed` vs `dynamic`), and fold the max of each manifold point's
`data.impulse`. Hit sensors never appear here — sensors produce
intersection pairs, not solver contacts — so this is exactly "solid hull
against solid world". The frog counting as static contact is deliberate:
pushing against the frog is the historical stuck case.

### 4.2 Snapshot: instantaneous facts only, windowing stays in the probe

`TankSnapshot` gains four fields; **no accumulators are added to `Tank` or
`Game`** — the simulation reports per-frame facts and the probe owns
thresholds/windows, exactly like the existing anomaly checks:

```rust
pub commanded_velocity: Position, // Tank::velocity — the target drive_tank chases
pub top_speed: f32,               // Tank::speed — this tank's rolled base top speed
pub touching_static: bool,        // from Physics::contact_stats
pub contact_impulse: f32,         //   "
```

(`tank_snapshots` already reads the physics body for real velocity; one
more read per tank per snapshot is nothing.) Sampling cadence: snapshots
reflect the narrow phase after the last physics step of that `update` —
in the probe that's exactly one step per frame (Phase 1.4), so per-frame
booleans are well-defined.

### 4.3 New probe anomaly kinds (same `TankTrack`/`check_anomalies` pattern)

- **`wall-grind`** — `touching_static` && commanded speed > ε,
  sustained `GRIND_FRAMES` (start: 120 = 2s) consecutive frames. The
  failure `stall` can't see: driving *into* terrain, possibly still moving
  (sliding along it).
- **`bump-rate`** — rising edges of `touching_static` per trailing 60s
  window above `BUMP_RATE_MAX`. This is "hitting obstacles too much" made
  literal. Threshold comes from Phase 6's baseline pass, not guessed.
- **`low-progress`** — commanded speed > 0.5 × `top_speed` while real
  speed < 0.3 × commanded, sustained `PROGRESS_FRAMES` (start: 180 = 3s).
  The principled stuck definition: intent vs. outcome. Catches half-speed
  grinding that clears `STALL_SPEED_EPS`. Note the relationship to the
  AI's own escape hatch (`STUCK_ESCAPE_SECONDS` = 0.75s at < 8px/s): the
  anomaly window is deliberately ~4× longer — it fires only when the
  escape mechanism is *failing repeatedly*, not on each normal trigger.

All thresholds are named consts at the top of probe.rs with the tuning
rationale in comments, per the existing convention there.

**Phase 4 verified by:** `tight-corridors.toml` + an `advance` run showing
nonzero grind/bump numbers while the open `default.toml` afk baseline stays
near zero; manually wedging the player into a wall in a windowed `--seed`
run and confirming the same frames flag in the probe replay; the
determinism smoke test still green (contact reads are pure queries).

**Landed (2026-08-27, subagent fork, whole phase).** As designed:
`Physics::contact_stats` (pair iteration on the hull collider, fixed ⇒
static incl. walls/frog, max per-point solver impulse), four snapshot
fields slotted beside the AI-session's `laser_charges`, three probe kinds
wired through every total/summary. No accumulators in the sim — §4.2's
instantaneous-facts split held. Thresholds were **measured, then set**
(temporary rising-edge instrumentation, added and removed): per-tank
bump-window maxima of 7/13/12 per minute on the default map
(afk/advance/circle) and 0–5 on every fixture ⇒ `BUMP_RATE_MAX = 30`,
~2.3× the worst legitimate peak and below collide-thrash rates; the
measurement table lives in the const's comment. Suite 36/36 green,
independently re-verified.

The interesting part: **the verified-by prediction above was wrong in
direction, for a good reason.** Post-rasterization-fix routing is clean —
`tight-corridors` logged *zero* contact events across 10 afk rounds; the
snug tube is driven without a scrape. It's the dense shipped
`default.toml` that carries the real contact population: wall-grind 4/3/3
and low-progress 8/9/6 (afk/advance/circle, 10 rounds each, `--seed
1000`), zero `invariant` anywhere. Every hit is a seeded repro; the
canonical specimen — seed `0x3e9`, frame 120, an enemy at (960, 374)
commanding 173px/s while achieving 0 against ~2040 impulse, flagged by
`wall-grind` and then `low-progress` 70 frames later — looks like a
genuine stuck spot on the current default map, queued for the same
burn-down list as its lint debt. Two small deviations: `touching_tank`
exists in `ContactStats` but isn't yet a snapshot field (ready for future
ram/pile-up metrics); the windowed wedge-the-player check was superseded
by that seeded specimen.

---

## Phase 5 — Navigation e2e (path-stretch)

### 5.1 `Grid::path_cost`

`pathfind.rs` grows `pub fn path_cost(&self, from: Position, to: Position)
-> Option<u32>` — cardinal step count of the shortest path (`None` if
unreachable; `Some(0)` for same-cell). Implemented by refactoring the A*
body into an internal search shared with `next_step` (both need
`came_from`/`g_score`; `next_step`'s public behavior is unchanged). Unit
tests alongside the existing ones: open-field cost equals Manhattan cell
distance; a detour costs more; sealed goal returns `None`.

*Landed 2026-08-27 (early, alongside Phase 3):* the shared internal
`search` returns `SearchHit { first_step, cost }`; `Some(0)` for
same-cell is deliberate and documented against `next_step`'s conflating
`None`. Four new tests; `next_step`'s pre-existing tests pass untouched.

### 5.2 The metric, on `pub` API only

`Game` gains one accessor so the probe never touches `world`:

```rust
/// Shortest-path cost in grid cells between two points on this round's
/// nav grid (see `nav_grid`) — for external tooling (probe path-stretch).
pub fn nav_path_cells(&self, from: Position, to: Position, width: f32, height: f32) -> Option<u32>
```

At round start (frame 0) the probe computes, per enemy:
`ideal_seconds = path_cells × PATHFIND_CELL_SIZE / top_speed` from its
spawn to the player's position. During the run it records
`time_to_engage`: the first `t` at which that enemy comes within
`ENEMY_ATTACK_RANGE` (340px) of the player. Engagement range — not
contact — is the correct goal: reaching it is exactly when the AI's
Attack behavior takes over and *stops approaching* (ring slots, reserve
ranks, retreat are all by design from there).

- **`never-arrived`** anomaly: round hits the frame cap (or ends) with a
  live, never-engaged enemy whose spawn-time `path_cells` was `Some`, and
  elapsed > `NAV_GRACE_SECONDS` (start: 10s — patrol/alert latency
  allowance) + `NAV_STRETCH_MAX` (start: 4.0) × its `ideal_seconds`. The
  detail string reports the achieved stretch so near-misses are visible
  in sweeps before they become failures.
- Per-enemy stretch values go into Phase 6's JSONL either way — the
  distribution over a sweep is the real health signal; the anomaly is
  just its tail.

Known limits, accepted: an enemy that legitimately never acquires the
player (patrols a far corner of a huge map for the whole cap, view range
notwithstanding — `ENEMY_VIEW_RANGE` is 800px, so rare at 1280×720) shows
up as grace-time pressure, which is what the constant is for; a retreating
enemy that already engaged once is out of scope (first crossing only).

**Phase 5 verified by:** `path_cost` unit tests; afk sweeps on
`default.toml` (stretch distribution ≈ 1–2, zero `never-arrived`) vs
`u-trap.toml`/`maze.toml` (higher stretch, still arriving) vs a
deliberately-sealed throwaway map (`path_cells = None` ⇒ excluded, no
false flag).

**Landed (2026-08-27, subagent fork; §5.1 had landed earlier).**
`Game::nav_path_cells` beside `round_seed()`; `NAV_GRACE_SECONDS = 10` /
`NAV_STRETCH_MAX = 4.0`; `TankTrack` carries route/ideal/time-to-engage
and the verdict runs once at round end — deliberately *outside*
`check_anomalies`' still-Playing gate, since "never arrived" is only
decidable when arriving can no longer happen. Trace mode prints a
per-enemy stretch table. At production thresholds: `never-arrived` = 0
and `invariant` = 0 across default/maze/u-trap/tight-corridors/
frog-block (10 seeded rounds each); since nothing fired naturally, the
wiring was proven by a temporary threshold-floor positive control (maze
seed `0x3ea`: "route existed (15 cells, ideal 4.3s) but no engagement in
13.1s") and then reverted to byte-identical sweep output. Suite 36/36.

Reality vs the prediction above, and two recorded caveats:

- Stretch distributions run **sub-1.0** (u-trap 0.0–0.6, maze 0.0–1.0),
  not the predicted 1–2: "engaged" fires `ENEMY_ATTACK_RANGE` (340px)
  short of the player while `ideal` prices the full route. The health
  signal is the tail (≫1 or never), not the mean — the trace table shows
  the distribution for exactly this reason.
- **Euclidean engagement blind spot**: maze enemies with 34–37-cell
  routes log `engaged=0.0s` — within 340px of the player *through the
  walls*. Faithful to §5.2 as specified (and to the AI itself, whose
  attack gate is equally Euclidean); a future refinement could gate
  arrival on line-of-sight or route distance.
- **`route=none` on the shipped map**: a default.toml round (seed 1000)
  spawned an enemy with *no route to the player at all* — independent
  corroboration of the linter's spawn-band finding (attempt-cap fallback
  spawns landing in grid-sealed areas). Excluded from `never-arrived` by
  design; currently visible only in the trace table, so a
  `no-route-at-spawn` anomaly kind is a candidate promotion.

---

## Phase 6 — Sweep ergonomics: JSONL, budgets, heatmaps, CI

### 6.1 Machine-readable rounds: `--json-out <path>`

One JSON object per line per round, hand-emitted (`format!` — the schema
is flat; the only strings are the map path, scenario, and outcome, escaped
minimally). Human output on stdout is unchanged.

```json
{"v":1,"round":3,"seed":"0x1a2b3c4d5e6f7788","map":"maps/test/choke.toml",
 "scenario":"afk","enemies":4,"frames_run":900,"outcome":"Playing",
 "anomalies":{"stale_start":0,"stall":1,"border_stuck":0,"jitter":0,
              "clustering":0,"wall_grind":2,"bump_rate":0,"low_progress":1,
              "never_arrived":0,"invariant":0},
 "tanks":[{"label":"ENEMY#0","time_to_engage":6.4,"stretch":1.3,
           "contact_events":11,"grind_seconds":0.0,
           "distance_travelled":1480.2}]}
```

`"v":1` so later schema changes don't strand scripts. Everything a script
needs to rank worst seeds, plot stretch distributions, or diff two
branches' sweeps is in the file; replay is one copy-paste of `seed`.

### 6.2 Budgets: `--budget <kind>=<max>` (repeatable) 

Kinds are the anomaly kinds plus `total`. After a sweep, any budget
exceeded prints a `BUDGET EXCEEDED kind=… count=… max=…` line and the
process exits 1. No budgets given = report-only, today's behavior. Policy
(this is process, and it matters more than the mechanism): **run the
baseline sweep first, set budgets at observed reality, then ratchet.**
Hard zeros on day one make the gate flaky and get it deleted; the two
that should genuinely start at 0 are `invariant` and (per current
observed behavior on `default.toml`) `stall`.

### 6.3 Heatmap triage: `--heatmap`

At sweep end, for each anomaly kind with ≥ 1 hit, print a
`PATHFIND_CELL_SIZE`-granularity ASCII grid (27×15 at 1280×720) of
flag positions (`·` none, `1–9`, `#` for 10+), plus the map name and a
scale line. Anomaly positions are already collected; this is bucketing
plus a print. It converts a sweep from "7 stalls somewhere" into "the
stalls are all in the two cells beside the choke mouth" — usually the
whole diagnosis.

### 6.4 Recipes & CI

Justfile:

```
probe-sweep:        # the default health check
    cargo run --bin probe -- --scenario afk --enemies 4 --frames 1800 --rounds 30 --heatmap
probe-fixtures:     # every fixture map, budgeted
    for m in maps/test/*.toml; do cargo run --bin probe -- --map $m --rounds 10 --budget invariant=0 --budget stall=0 || exit 1; done
```

CI: a new `.github/workflows/ci.yml` alongside the existing deploy
workflows (this repo has no test workflow yet — release.yml is
cargo-dist-generated, don't touch it): `cargo test` (pathfind + shell
sweep + determinism smoke + map lint + pinned regressions) on every PR,
plus the budgeted fixture sweep. Native build without a display is fine —
probe and tests never open a window; only `cargo run` (the game bin)
would. Sweep cost is seconds; the build dominates. No secrets needed.

**Phase 6 verified by:** JSONL parsed by a throwaway `python -c
json.loads` loop over the file; a deliberately-tight budget exiting 1; the
workflow green on a clean branch and red on a branch with a seeded,
known-bad regression (e.g. temporarily reverting the frog-in-grid fix).

**Landed (2026-08-27, subagent fork + wave-5 finishing pass).** As
designed, with an `ANOMALY_KINDS` registry (12 tags in reporting order)
as the single source of truth wiring every kind into JSONL, budgets, and
heatmaps at once. `--json-out` writes each round's record as it finishes
(an interrupted sweep still leaves valid JSONL) and reports the *actual*
spawned enemy count; `path_cells: null` per tank makes the
no-route-at-spawn population machine-visible. `--budget` parses strictly
(unknown kinds are clap errors listing the valid set) and exits 1 after
all other output on a breach. The first heatmap render localized the
default map's stalls/jitter to the row-3 band beside its walled top
strip — converging with the lint debt, independently. All verified twice
(fork, then parent session): suite 36/36, JSONL `json.loads`-clean,
budget exit codes 0/1, `just probe-fixtures` end-to-end. ci.yml's apt
list is a verbatim copy of `dist-workspace.toml`'s; its first real
validation happens on push, by design.

## Wave-5 close-out: the standing baselines (2026-08-27, final binary)

All phases landed; these are the definitive seeded baselines every
recorded number in earlier landed notes is superseded by (`--seed 1000`,
afk, 1800 frames; default 30 rounds `--enemies 4`, fixtures 10 rounds at
map tank counts):

| map | flagged | totals (nonzero kinds) |
|---|---|---|
| default.toml | 27/30 | stale-start=5 stall=5 border-stuck=5 jitter=20 spin=1 churn=37 clustering=15 wall-grind=7 low-progress=38 **never-arrived=1** |
| u-trap | 4/10 | jitter=1 spin=2 churn=1 |
| choke | 3/10 | jitter=1 churn=2 |
| tight-corridors | 3/10 | jitter=1 spin=2 clustering=3 |
| frog-block | 3/10 | jitter=1 clustering=4 |
| maze | 8/10 | jitter=8 spin=1 churn=5 clustering=6 low-progress=3 |
| pockets | 1/10 | churn=2 |

`invariant` and `bump-rate` are zero everywhere. The fixture gate
(`just probe-fixtures`, mirrored in ci.yml) pins the seed and holds each
kind to its cross-fixture maximum exactly — deterministic, so an
exceedance is a real behavior change; the justfile comment carries the
re-baselining policy.

### Re-baseline 2026-09-03: nav-grid spawn legality

Enemy spawn placement changed (`battlefield::enemy_spawn_legal` replaced
the `enemy_clear + OBSTACLE_CLEAR` distance term - which no cell of the
default map's band could satisfy, so every enemy there was an attempt-cap
fallback, ~9% of them with their center inside a wall tile - with
nav-grid usability plus a tank-box-vs-wall-box separation; the sampler
no longer hands back a rejected sample, and `relocate_unusable_spawns`
also relocates blocked cells). Every round's RNG stream shifts with it,
so the pinned-seed numbers were re-read (same sweep parameters as above):

| map | flagged | totals (nonzero kinds) |
|---|---|---|
| default.toml | 27/30 | stall=1 border-stuck=4 jitter=15 spin=2 churn=48 clustering=6 wall-grind=1 low-progress=30 |
| u-trap | 2/10 | jitter=1 spin=1 |
| choke | 4/10 | jitter=2 churn=2 |
| tight-corridors | 2/10 | jitter=1 spin=1 churn=1 |
| frog-block | 2/10 | churn=3 |
| maze | 8/10 | jitter=7 spin=2 churn=5 clustering=16 low-progress=7 |
| pockets | 4/10 | jitter=5 churn=3 |

The default map's stall/wall-grind/stale-start counts fell several-fold
(the spawn fallback was seeding tanks in or against walls); `never-arrived`
is gone (its one case was a fallback spawn in a sealed area). The maze's
clustering/low-progress rise is a pinned-seed artifact: a 30-round sweep
at `--seed 2000` reads clustering=26 jitter=16 low-progress=6 after versus
clustering=34 jitter=24 low-progress=11 before. `stale-start` now judges
the farthest a tank got from spawn inside the window instead of where it
stands when the window closes (its two remaining hits were tanks driving
at full speed that happened to pass back through spawn at the 2s mark), so
it reads zero everywhere again. In the same pass the linter learned to
tell gated loot from sealed loot: a pickup approachable only after
destructible walls are gone is a `gated-pickup` warning. default.toml's
28 formerly-unreachable slots (the top-edge strip and the bottom-right
rows; the three top-center health packs sit behind a permanent iron gate
breachable only through the brick columns beside it) are all gated, so
the map carries no lint errors any more. Ceilings in `just probe-fixtures`/ci.yml are the
new cross-fixture maxima.

### Re-baseline 2026-09-04: QA'd tuning defaults + progress-based stuck detection

The QA'd knob set from the web panel became `tuning.rs`'s defaults
(player 220->210 px/s, enemies 150->160, 12 shells, minigun bursts of 6
at 570 px/s dealing 3-6, ammo crates +10, fragile brick/wood/glass walls,
plus shadow/HUD/shockwave cosmetics). Speeds and ammo shift every round's
RNG stream, and the first re-read surfaced the first fixture
`border-stuck` ever: maze round 6 (`--seed 0x3ee`), ENEMY#0 reaching the
right border wall at ~15s and sitting there nudging it at ~8px/s. A
per-frame AI trace showed a three-tank jam in the open ground east of the
maze: two chasers with correct but opposed first legs (Down and Up, their
engagement slots one nav row apart) plus a third committed Right while
its route said Left. Pressed together, the rounded tank colliders slid the
whole jam east at up to 100px/s until the wall stopped it - and because
every tank in it *was* moving, `Ai`'s stuck escape (gated on real speed
under `stuck_speed_eps`) never fired, while predictive avoidance skips
tanks already in contact. `Ai::think` now takes the real velocity vector
and projects it onto the heading it commanded last tick, so being carried
sideways or backwards counts as stuck; the escape prefers an unblocked
perpendicular and backs straight out as a last resort (`ai.rs`'s
`stuck_tests`). Letting the commitment gate flip on a straight reversal
was tried alongside and rejected: `act_chase` never stops at its slot, so
a reversible tank overshoots, flips, overshoots again, and the default
map's 30-round jitter went 6->30. The pinned-seed numbers (same sweep
parameters as above) with both changes in:

| map | flagged | totals (nonzero kinds) |
|---|---|---|
| default.toml | 25/30 | border-stuck=1 jitter=6 spin=1 churn=35 clustering=3 low-progress=4 |
| u-trap | 2/10 | jitter=1 churn=2 |
| choke | 3/10 | churn=5 |
| tight-corridors | 6/10 | jitter=1 churn=5 clustering=1 low-progress=2 |
| frog-block | 1/10 | clustering=3 |
| maze | 8/10 | jitter=4 spin=3 churn=14 clustering=17 |
| pockets | 2/10 | jitter=1 churn=3 |

The default map improved on every kind versus the 2026-09-03 read (stall
and wall-grind now zero, jitter 15->6, churn 48->35, low-progress 30->4).
The maze's churn rise is a pinned-seed artifact: a 30-round sweep at
`--seed 5000` reads jitter=10 spin=3 churn=21 clustering=22
low-progress=2 after versus jitter=18 spin=6 churn=25 clustering=26
low-progress=7 before. The fixtures produce no `border-stuck` at all
again; the default map's one remaining hit predates both changes and is
untouched by them. Ceilings in `just probe-fixtures`/ci.yml are the new
cross-fixture maxima. One later shift: the rainbow-shield spawn roll
(one RNG draw per tank in `Game::init`, 2026-09-04) moved every pinned
stream and the maze now reads jitter=5 spin=2 churn=6 clustering=7
low-progress=2 at the same seed - identical with the shield knobs zeroed,
so the jitter ceiling went 4->5 as a stream artifact, not a behaviour
change (the maze has no health cells, so no shield ever spawns there).

**The burn-down list**, where every instrument now points at the same
place — the default map, especially its walled top strip:

1. `default.toml` lint debt (`KNOWN_ERROR_KINDS`): the unreachable
   top-strip pickups and the zero-legal-cell spawn band.
2. The natural `never-arrived`: seed `0x3fd`, an enemy at (507, 170) —
   nav-grid row ~3, beside that same strip — alive with a 9-cell route
   (ideal 2.3s) and no engagement in 26.8s.
3. The wall-grind family: seed `0x3e9`, (960, 374), commanded 173px/s
   achieving 0.
4. The heatmap's stall/jitter concentration in the row-3 band.

Fixing the map's top strip (or the spawn band predicate) and replaying
those seeds is the intended next use of this tooling.

---

## Promotion path: from sweep finding to pinned regression

The lifecycle every finding follows (this is the "e2e tests" the tooling
exists to feed):

1. Sweep flags `(map, scenario, seed)` — reproducible forever (Phase 1).
2. Diagnose: `probe --seed … --log-every 1` for the frame trace;
   `bongbong --seed … -m …` to watch it; heatmap to localize.
3. Fix in `ai.rs`/`simulation.rs`/the map, as its own change.
4. **Pin it**: a `#[cfg(test)]` in simulation.rs (precedent:
   `shell_sweep_tests`) that seeds that exact round, runs N frames
   headlessly, and asserts the *behavioral* claim — "every enemy engages
   within its stretch budget", "no tank's real speed stays under
   8px/s for 3s while commanded to move" — via `tank_snapshots`, not
   exact positions (survives benign RNG-stream drift across toolchain
   bumps; if the stream shifts, the test's seed gets re-rolled to a new
   equivalent repro, and the behavioral assertion is what's protected).
5. Where the failure was map-shaped, also add/extend a fixture map so the
   *class* stays trapped, not just the instance.

Heavy statistical sweeps stay out of `cargo test` (they're the probe's
job, on demand and in CI); the test suite holds only fast, deterministic
proofs: unit tests, the determinism smoke, map lint, pinned regressions.

## Threshold appendix (initial values, all named consts, all provisional)

| const | start | why |
|---|---|---|
| `GRIND_FRAMES` | 120 (2s) | matches `STALE_START_FRAMES` scale; > any legitimate corner brush |
| `BUMP_RATE_MAX` | 30/min | measured maxima: 7/13/12 per min on default (afk/advance/circle), 0–5 on fixtures; ≈2.3× worst legitimate peak, below collide-thrash rates |
| `PROGRESS_FRAMES` | 180 (3s) | matches `STALL_FRAMES_THRESHOLD`; ~4× the AI's own `STUCK_ESCAPE_SECONDS` |
| `NAV_STRETCH_MAX` | 4.0 | dodging/commitment/engagement detours legitimately cost 2–3× |
| `NAV_GRACE_SECONDS` | 10 | patrol wander + alert propagation before pursuit starts |
| `INVARIANT_SPEED_MAX` | 800 px/s | > TANK_SPEED(220) × boost, with knockback headroom |

## Open questions / future work

- **`jitter` upgrade**: with Phase 4 data, a compound "jitter while zero
  net progress" check could finally separate wasteful flip-flopping from
  legitimate maze weaving (the documented blind spot). Deferred until the
  contact metrics have baselines.
- **Sweep parallelism**: rounds are independent once seeded;
  `std::thread::scope` over chunks if thousand-round sweeps ever feel
  slow. Not now — 30 rounds ≈ 1s.
- **Player-behavior fuzzing**: scenarios are fixed scripts; a seeded
  random-walk player scenario (`--scenario fuzz`) would widen coverage
  cheaply once determinism makes its findings replayable. Small,
  worthwhile follow-up.
- **`--lint` probe flag**: `maplint::lint` is `pub` specifically so the
  probe can grow a human-facing lint mode for editor sessions; the tests
  are what gate CI, so this is convenience, not required.
- **Cross-platform determinism** (rapier `enhanced-determinism`, wasm):
  only if seed-sharing between machines ever matters.
