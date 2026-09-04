# Maps to levels: missions, enemy spawn plans, waves

Status: **being implemented** (interviewed and decided 2026-09-04;
the "Decisions" section records what was chosen and why). Phases below are
ordered so every phase leaves the game playable and `cargo test --lib`
green.

## Purpose

Today every map plays the same round: a random band of enemies, one frog to
protect, the round ends when the player or the frog dies or every enemy is a
wreck. This design turns a map into a *level*: each map states its
**mission** (what ends the round) and its **spawn plan** (how enemies arrive),
both overridable from the CLI and tunable at runtime. "Proceed to the next
level" stays out of scope: a finished round restarts the same map, exactly as
now.

## Decisions (from the 2026-09-04 interview)

| Topic | Decision |
|---|---|
| Hunt-mission AI | Each enemy is rolled **hunter** (drives to and shoots the player's frog, fights the player only when in the way) or **guard** (stays leashed near its own frog, engages the player as today). The split is a knob. |
| Protect-mission AI | Same hunter roll with a lower default share, so the frog is a real target rather than collateral. Enemies not rolled hunter behave exactly as today. |
| Destroy mission | No frog is spawned at all. Win = every enemy wrecked and no more arriving. |
| Wave pacing | Next wave when live enemies drop to `wave_next_when_alive` (default 0) **or** `wave_timeout_seconds` elapses, whichever first, then a `wave_gap_seconds` breather with a "Wave N" banner. |
| Tank class | Four named **tiers**: `light` (scout, wraith, flak, glacier), `medium` (assault, warden), `heavy` (longbow, obelisk, breaker, ravager), `super` (titan, leviathan). A wave plan gives a start and end tier; waves interpolate between them. |
| Roll-in gates | Gates are found automatically each wave: an edge nav cell with `wave_gate_inward_cells` open cells inward. A map may also place explicit `kind = "gate"` cells, which win when present. |
| Wrecks in wave rounds | A wreck despawns (fades) after `wave_wreck_despawn_seconds`; owner slots are monotonic and never reused (the 31 is a cap on *live* enemies, not on a round's total). Non-wave rounds keep wrecks forever, as today. |
| Mission banner | The round starts frozen for `mission_banner_seconds` with big white mission text over a dim overlay, then unfreezes and the text fades. Shown on every restart; move or fire skips it. Headless callers (probe, tests, dev server) start with the intro off. |
| Enemy frog | New `kind = "enemy_frog"` map cell (plus editor tool). A hunt map without one gets a procedural spot in the enemy spawn band. Same `Frog` entity as the player's (bites, hops, health), told apart by a red ground ring; the player's frog gets a white one. |
| Hunt win rule | Only the enemy frog's death wins. Wrecking every enemy makes the field safe but does not end the round. *(Assumption made while writing this plan; flag if you want "all enemies dead" to count as a win too.)* |
| Frog damage sides | Any shot damages any frog, exactly as the player's own shells damage the frog today. Keeps the "watch your fire" tension symmetric. |

## Concepts

### Mission

```rust
pub enum Mission { Protect, Hunt, Destroy }   // default: Protect
```

| Mission | Frogs spawned | Lose when | Win when | Banner |
|---|---|---|---|---|
| Protect | player frog | player wreck or player frog dead | all enemies wrecked and spawn plan finished | `PROTECT THE FROG!` |
| Hunt | player frog + enemy frog | player wreck or player frog dead | enemy frog dead | `HUNT THE FROG!` |
| Destroy | none | player wreck | all enemies wrecked and spawn plan finished | `DESTROY!` |

"Spawn plan finished" = no wave pending and no tank still rolling in. Losing
takes precedence over winning on the same frame (as today).

### Spawn plan

```rust
pub enum SpawnPlan {
    /// Today's behaviour: `count` enemies placed in the spawn band at init.
    Band { count: Option<usize> },
    /// Enemies arrive in waves through edge gates.
    Waves { waves: u32, size: u32, growth: u32, tier_start: Tier, tier_end: Tier },
}
```

`Band` is the default. Its count resolves exactly as now: `--enemies`, then
the map's `tanks`, then a random `enemy_count_min..=enemy_count_max` roll,
clamped to `1..=31`.

`Waves`: wave `i` (0-based) brings `size + i * growth` tanks, capped so live
enemies never exceed 31 (surplus tanks queue for the next opening). Its tier
is `tier_start + round((tier_end - tier_start) * i / max(waves - 1, 1))`;
each tank rolls a chassis from that tier's rows, with `wave_tier_mix` odds of
one tier lower for variety. Special weapon and shield spawn rolls apply
exactly as at init.

### Tier ladder

`TANK_TIER_BY_ROW: [Tier; 12]` in `lib.rs` next to the chassis class table
(roster classification, not a feel knob): rows 0 scout, 5 wraith, 4 flak,
8 glacier → `light`; 1 assault, 6 warden → `medium`; 3 longbow, 9 obelisk,
2 breaker, 7 ravager → `heavy`; 10 titan, 11 leviathan → `super`.

### Gates and roll-in

A **gate** is a nav-grid edge cell (col 0, col `cols-1`, row 0 or row
`rows-1`) whose `wave_gate_inward_cells` cells toward the interior are all
`Grid::usable`, and which lies at least `wave_gate_min_player_dist` px from
the player and the player's frog. Candidates are recomputed from the live
nav grid at the start of each wave (walls may have been shot away), sorted,
and drawn with the round RNG. Explicit `kind = "gate"` cells replace the
scan entirely when a map has any; a gate cell that is not on an edge or is
blocked inward is a lint error.

**Roll-in** is kinematic: a wave tank is spawned *outside* the boundary at
the gate's edge point minus one tank length, with `Tank::body == None` and a
`RollIn { to: Position }` component, facing inward. Each frame
`rollin_phase` moves it straight toward `to` (the gate's innermost open
cell center) at `tank_speed * speed_scale * wave_rollin_speed_factor`, with
treads animating and tracks laid. While rolling in the tank is not in the
hit sweep, the ram check, the engage ring, the AI phase or the "all enemies
wrecked" test. On arrival the component is removed, the physics body spawned
(`Physics::spawn_tank`) and `Ai::default()` takes over; `Event::TankEntered`
is pushed. Tanks of one wave are staggered `wave_stagger_seconds` apart and
spread over distinct gates so two never overlap in one lane.

### AI roles

```rust
pub enum Role { Player, Hunter, Guard }   // Ai::role, rolled at spawn
```

- `Player` (today's behaviour, unchanged): target is the player, engage
  ring around the player.
- `Hunter`: target is the player's frog. Steers via the grid to a second
  engage ring built around the frog (`Game::engage_frog`), fire gate is line
  of sight to the frog. Opportunistic: if the player is within
  `enemy_attack_range` with line of sight, shoot the player this tick
  instead. Reverts to `Player` behaviour when the frog is dead.
- `Guard` (Hunt only): leashed to its own frog. Engages the player as
  `Player` does while the player is within `guard_leash_px` of the enemy
  frog; otherwise wanders inside the leash and returns when outside it.

`Ai::think` gains a `target: Position` (replacing the implicit player) and a
`frog_target: Option<Position>`; it stays snapshot-only. The role share
rolls are `enemy_hunter_share_protect` (Protect) and
`enemy_hunter_share_hunt` (Hunt; the rest are guards). Destroy rolls nothing.

## Data model

### Map TOML (`map.rs`)

```toml
version = 1
tanks = 6                        # unchanged: Band plan default count

[mission]
kind = "hunt"                    # protect (default) | hunt | destroy

[spawn]
kind = "waves"                   # band (default) | waves
waves = 5
size = 3                         # first wave size
growth = 1                       # tanks added per wave
tier_start = "light"
tier_end = "super"

cells."3,5"  = { kind = "enemy_frog" }   # hunt: one, singleton like frog
cells."0,11" = { kind = "gate" }         # waves: optional, edge cells only
```

Both tables are `#[serde(default)]`, so every existing map parses unchanged
as Protect + Band. **Write them as dotted keys** (`mission.kind = "hunt"`,
`spawn.kind = "waves"`, `spawn.waves = 5`, ...) rather than `[mission]` /
`[spawn]` headers: a table header swallows every `cells."c,r" = ...` line
after it. `MapFile::to_toml_string` emits the tables after `cells`, so
editor-saved files are fine either way. `MapFile` gains `mission: MissionConfig` and
`spawn: SpawnConfig` (plain serde structs with the defaults above; the
`SpawnPlan` enum is resolved in `Game::init` after CLI precedence), plus
`enemy_frog_cell()` and `gate_cells()`. The editor round-trips the new
fields untouched (it saves the loaded `MapFile`). `nearest_free_cell` and
`enemy_spawn_legal` treat `enemy_frog`/`gate` like other non-wall cells.

### CLI (`main.rs` and `bin/probe.rs`, identical flags)

```
--mission protect|hunt|destroy
--spawn band|waves
--waves N  --wave-size N  --wave-growth N  --tier-start T  --tier-end T
```

Precedence for every field: CLI flag > map table > tuning/plan default.
`--enemies` keeps its meaning for the Band plan; with `--spawn waves` it is
an error to avoid silent surprises.

### Tuning (`tuning.rs`)

New group `mission`:

| knob | default | applies |
|---|---|---|
| `mission_banner_seconds` | 2.0 | Live |
| `enemy_hunter_share_protect` | 0.25 | Spawn |
| `enemy_hunter_share_hunt` | 0.6 | Spawn |
| `guard_leash_px` | 260 | Live |
| `enemy_frog_spawn_min_dist` | 400 (from the player's frog) | Restart |

New group `waves`:

| knob | default | applies |
|---|---|---|
| `wave_size_default` | 3 | Restart |
| `wave_growth_default` | 1 | Restart |
| `wave_count_default` | 5 | Restart |
| `wave_gap_seconds` | 4.0 | Live |
| `wave_timeout_seconds` | 60.0 | Live |
| `wave_next_when_alive` | 0 | Live |
| `wave_stagger_seconds` | 0.8 | Live |
| `wave_rollin_speed_factor` | 0.8 | Live |
| `wave_gate_inward_cells` | 3 | Live |
| `wave_gate_min_player_dist` | 300 | Live |
| `wave_tier_mix` | 0.25 | Live |
| `wave_wreck_despawn_seconds` | 20.0 (wave rounds only) | Live |
| `wave_max_alive` | 31 in 1..=31 | Live |

The mission and plan *kind* are level data (map/CLI), not knobs.

### Game state (`simulation/mod.rs`)

```rust
pub struct Game {
    pub mission: Mission,                 // resolved at init
    pub spawn_plan: SpawnPlan,            // resolved at init
    pub(crate) enemy_frog: Option<Entity>,
    pub(crate) engage_frog: EngageRing,   // hunters' ring
    intro_timer: f32,                     // mission banner freeze
    pub show_intro: bool,                 // false for headless callers
    wave: WaveState,                      // index, pending queue, gap/timeout timers, next slot
    // overrides, one per CLI flag: mission_override, spawn_override ...
}
```

`Frog` gains `side: Side` (`Player`/`Enemy`); `HitTarget::Frog` and
`Event::FrogBite`/`Hit` carry it. `Event` gains `WaveStarted { wave, size,
tier }`, `TankEntered { slot }`, `WreckRemoved { slot }`, and
`RoundStarted` gains `mission` and `spawn` fields.

`Game::update` phase order becomes: intro freeze check → timers → frog phase
(both frogs) → pickups → player → **rollin_phase** → enemies → **wave_phase**
(schedules/spawns) → … → cleanup (**wreck_despawn**) → round end (per
mission). Wave spawning draws from the round RNG inside the fixed-step
frame, so a seeded round replays bit-for-bit including waves.

## Rendering (`game.rs`)

- Mission banner: 72 px white title centred over a `Color::new(0,0,0,120)`
  overlay while `intro_timer > 0`, then a 0.5 s alpha fade. Same code
  path draws `WAVE N` (with `FINAL WAVE` on the last) during the wave gap,
  smaller and without the overlay.
- Enemy frog: `draw_ground_ring` in red under the hull (`RingStyle::Enemy`);
  player frog gets the same ring in white. Both frogs' overhead health bars
  unchanged.
- HUD: `WAVE i/N` and live-enemy count on the right of the existing HUD line
  in wave rounds; `MISSION: …` is not shown (the banner covers it).
- Rolling-in tanks are drawn as normal tanks (they are partly off-screen by
  construction); wrecks fade alpha over the last second before despawn.

## Tooling

- **Dev server / MCP** (`devserver.rs`, `bin/bbmcp.rs`): `restart` gains
  `mission`, `spawn`, `waves`, `wave_size`, `wave_growth`, `tier_start`,
  `tier_end`, `intro` (default false). `status` reports `mission`, `spawn`
  and `wave { index, total, alive, pending, next_in }`. `snapshot.frogs`
  becomes a list with `side`; `tanks[]` gains `role` and `entering`.
  `spawn_enemy` gains an optional `role`. `map_get`'s description lists the
  two new cell kinds and tables.
- **Probe** (`bin/probe.rs`): the CLI flags above; the header and JSONL
  record carry `mission`/`spawn`; anomaly checks skip tanks that are still
  rolling in and measure `stale-start`/`never-arrived` from the arrival
  frame and position; a new `just probe-waves` recipe sweeps
  `maps/missions/waves-basic.toml`. Hunt rounds against an AFK player are
  expected to be lost quickly; the sweep uses `--mission destroy`/`protect`
  for AI-health checks and a dedicated `hunt` run only for the frog-race
  outcome distribution.
- **Map linter** (`maplint.rs`): new kinds `gate-not-on-edge` (error),
  `gate-blocked` (error), `waves-no-gates` (error: waves plan and zero
  candidates), `hunt-missing-enemy-frog` (warning: fallback used),
  `enemy-frog-unreachable` (error: not in the player start's component).
  The spawn-band capacity check runs only for the Band plan.
- **Editor** (`editor.rs`): two new tools, `EnemyFrog` (singleton, frog idle
  sprite with a red ring) and `Gate` (multi-place, drawn as an arrow chevron
  on the edge). Mission/spawn tables stay TOML-only for now; the editor
  preserves them on save. docs/map-editor-design.md's cell table gets the
  two rows.
- **Docs**: CLAUDE.md module map (map.rs, simulation, ai.rs, game.rs,
  probe/devserver bullets), dev-server-design.md tool list.

## Implementation phases

Each phase ends with `cargo test --lib`, `cargo test --lib --features
dev-tools`, `just probe-fixtures` green and a windowed check through the
MCP tools (`just run-dev`, `restart {...}`, `step`, `screenshot`).

### Phase 1: Mission plumbing, Destroy, banner

1. `Mission` enum + `[mission]` table in `map.rs`; `--mission` on both
   binaries; `Game::mission` with override precedence.
2. Round end per mission in `check_round_end`; `Game::frog` becomes truly
   optional (Destroy spawns none; every `expect("frog…")` becomes a guard).
3. `intro_timer`/`show_intro` freeze, banner drawing, skip on input;
   `mission_banner_seconds` knob; dev server `restart {intro}`.
4. Tests: map round-trip with/without `[mission]`; mechanics: Destroy has
   no frog and ends Won on the last wreck; Protect unchanged; intro freezes
   exactly `ceil(seconds / dt)` frames and a move input skips it.

### Phase 2: AI roles and the frog as a target

1. `Role` on `Ai`, `target`/`frog_target` parameters on `think`, second
   engage ring, hunter fire gate on frog line of sight, opportunistic
   player shots, revert when the frog dies.
2. Protect rolls hunters with `enemy_hunter_share_protect`.
3. Tests (`ai.rs` role tests, `mechanics_tests`): a hunter's distance to
   the frog decreases monotonically on an open map; a hunter with the player
   in range and LOS fires at the player; the existing `probe-fixtures`
   ceilings hold with the default share (re-baseline consciously if not;
   hunters change every seeded stream).

### Phase 3: Hunt mission

1. `enemy_frog` cell kind, `Frog::side`, spawn (map cell or band fallback),
   red/white ground rings, `enemy_frog_spawn_min_dist`.
2. Guards and the leash; Hunt role roll.
3. Win/lose rules; events carry `side`.
4. Editor tool, linter kinds, `maps/missions/hunt-basic.toml` fixture.
5. Tests: Hunt ends Won when the enemy frog dies and Lost when the player's
   does; a guard never leaves its leash on an open map; determinism test
   covers a Hunt round; lint fixture profiles.

### Phase 4: Waves

1. `SpawnPlan`, `[spawn]` table, CLI flags, `TANK_TIER_BY_ROW`, the
   `waves` tuning group.
2. `WaveState` scheduler (`wave_phase`): first wave at intro end, next on
   cleared-or-timeout, gap banner, cap on live enemies, tier interpolation.
3. Gates: auto scan + `gate` cells; `RollIn` component and `rollin_phase`;
   exclusion from hits/ram/engage/AI/round-end while entering; events.
4. Wreck despawn with fade, `remove_body`, `WreckRemoved`.
5. HUD wave counter, `WAVE N` banner, editor `Gate` tool, linter kinds,
   `maps/missions/waves-basic.toml`, `just probe-waves`.
6. Tests: wave 2 spawns only after wave 1 is cleared (and after timeout
   with one alive); a rolling-in tank has no body until arrival and arrives
   inside the bounds at a usable cell; the live cap holds with an oversized
   wave; a wreck is gone after the knob's seconds; a seeded waves round
   replays bit-for-bit; every tank of wave `i` belongs to its interpolated
   tier (or one lower).

### Phase 5: Tooling and docs

Dev server/MCP fields, probe flags and record fields, snapshot/status
additions, CLAUDE.md and design-doc updates, `.mcp.json` unchanged. Verify
end-to-end through the MCP tools: `restart {mission:"hunt",
spawn:"waves", intro:true}` → `screenshot` shows the banner → `step` past it
→ `events` show `wave_started`/`tank_entered` → `screenshot` shows a tank
rolling in through an edge gate.

## Out of scope

Level progression ("next level" after a win), per-wave explicit chassis
lists, editor UI for the mission/spawn tables, enemy frog art variants
beyond the ring, audio.
