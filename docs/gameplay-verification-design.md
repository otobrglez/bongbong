# Gameplay verification & stuck-detection tooling — design doc

Status: design only, nothing implemented yet. Six phases, each independently
landable and verified on its own; Phase 1 (determinism) is the enabler for
everything else and must land first. When a phase lands, update CLAUDE.md's
"Testing & tooling" section and flip that phase's checkbox here.

- [ ] Phase 1 — Seeded, replayable rounds (+ frame invariants)
- [ ] Phase 2 — `--map` for the probe + adversarial fixture maps
- [ ] Phase 3 — Static map linter (exhaustive, in `cargo test`)
- [ ] Phase 4 — Contact-level metrics (wall-grind / bump-rate / low-progress)
- [ ] Phase 5 — Navigation e2e (path-stretch / never-arrived)
- [ ] Phase 6 — Sweep ergonomics (JSONL, budgets, heatmaps, CI)

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
  `Game::engage_slot_choice`'s doc comments).
- `pathfind.rs` has real unit tests and the stuck-adjacent helpers
  (`boxed_in`, `blocked_ahead`, `nearest_open`);
  `battlefield::relocate_boxed_in_tanks` already audits spawns against the
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
`wall_cells`/`material_variant`/`engage_slot_choice` are lookup-only;
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
of quietly poisoning every future sweep. Runs in well under a second.

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
| `pockets.toml` | almost-sealed pockets in the spawn band | `boxed_in` spawns / `relocate_boxed_in_tanks` |

Conventions: every fixture places an explicit `Start` cell and a `Frog`
cell (so nothing about its terrain depends on random fallback placement),
and sets the map's `tanks` count so runs are comparable. `maps/` is not
bundled into releases (`dist-workspace.toml` includes only `static/`;
the shipped default is `include_str!`-embedded), so fixtures cost end
users nothing.

**Phase 2 verified by:** `probe --map maps/test/<each>.toml --scenario afk
--rounds 10` runs clean end-to-end; fixture-by-fixture baseline numbers
recorded for Phase 6's budgets.

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
| `BUMP_RATE_MAX` | from baseline | set after Phase 6 baseline pass, per map class |
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
