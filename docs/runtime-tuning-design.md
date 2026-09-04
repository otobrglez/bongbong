# Runtime tuning: live-editable game parameters (design proposal)

Status: **implemented 2026-09-02** (phases 1-4 of §11 landed in one pass:
`src/tuning.rs`, `src/capi.rs`, `build.rs`, the `dev-tools` feature, the
Astro panel, `just build-web-dev`, the CI `cargo-features` input, `--tuning`
on both binaries with the native mtime watch). Phase 5 (localhost HTTP/MCP,
live-stats JSON) is still open. Where the implementation diverged from the
text below it's noted inline as *[impl]*.

## 1. Goal

Make as many gameplay/feel parameters as possible (speed, traction, damage,
fire intervals, AI ranges, shadow/track cosmetics, shader ripple tuning)
editable **while the game is running**, in a development/QA build, so
fine-tuning is a slider drag instead of an edit-recompile-restart loop.

The parameters are declared once, in a table, by a macro. From that one
declaration the build derives:

- the typed Rust struct the simulation reads every frame,
- the defaults (today's `lib.rs` constant values),
- a machine-readable **schema** (name, group, type, doc, min/max, when a
  change takes effect) as JSON,
- JSON get/set (serde) for the values themselves,
- a tiny **C ABI** (`extern "C"`) so the Astro page can render a tuning
  table under the canvas and push edits into the running wasm.

Web first. The same core drives native later (file watch, then a local
HTTP/MCP endpoint), and the probe gets a `--tuning <file.json>` flag so a
tuned set can be swept headlessly.

## 2. Where the knobs live today, and which ones qualify

`lib.rs` holds 270 `pub const`s (221 `f32`, 27 `i32`, a few `u32`/`usize`,
and ~15 arrays/tuples). They split cleanly into two populations:

| Population | Examples | Runtime-tunable? |
| --- | --- | --- |
| **Tuning knobs** (scalars whose value is a design decision) | `TANK_SPEED`, `TANK_ACCEL_FORCE`, `TANK_TURN_GRIP_FORCE`, `SHELL_SPEED`, `MAX_SHELLS`, `SHELL_RECHARGE_SECONDS`, `PLAYER_DAMAGE_MIN/MAX`, `ENEMY_*_RANGE`, `ENEMY_FIRE_INTERVAL`, `AVOID_*`, `AI_DIR_HOLD_SECONDS`, `TRACK_*`, `SHOCKWAVE_*`, `*_SHADOW_*` | **Yes.** Roughly 150-180 of the 270. |
| **Layout constants** (values that describe an asset or a data structure) | `*_TEXTURE_SIZE`, `*_COL`, `*_VARIANTS`, `*_BY_ROW` tables, `DEFAULT_SCREEN_*`, `OBSTACLE_GRID_SIZE`, `PATHFIND_CELL_SIZE`, `WALL_THICKNESS` | **No.** They stay `pub const`. Changing them at runtime would desync sprite slicing, collider sizes, the nav grid and the map format. |

Some `*_BY_ROW` tables are tuning knobs indexed by chassis (mass factor,
damage factor, muzzle/barrel offsets, track weight) rather than layout;
those become **array rows** in the table, see §3.1. The ones that describe
colliders or art mapping (`TANK_HULL_BBOX_BY_ROW`, `TANK_TURRET_BBOX_BY_ROW`,
`TANK_SHELL_VARIANT_BY_ROW`) stay `const`.

The 27 module-local `const`s outside `lib.rs` (`ai.rs`, `maplint.rs`,
`obstacle.rs`, ...) are handled the same way, case by case: knob or layout.

## 3. The macro: `tunables!` in a new `src/tuning.rs`

Yes, a macro is the right tool, and a **declarative `macro_rules!`** is
enough. No proc-macro crate, no `paste`, no build-time codegen. The grammar
below was compile-checked with `rustc --edition 2024` before this doc was
written (doc-comment capture, negative literals, per-row `@` markers, the
`get`/`set` dispatch all work).

```rust
tunables! {
    group movement {
        /// Player top speed (px/s).
        tank_speed: f32 = 220.0 in 50.0 ..= 600.0 @ Spawn;
        /// Force that reaches TANK_SPEED in well under a second.
        tank_accel_force: f32 = 4200.0 in 500.0 ..= 20000.0;
        /// Sideways grip while turning - the "traction" knob.
        tank_turn_grip_force: f32 = 2200.0 in 0.0 ..= 10000.0;
    }
    group weapons {
        /// Shell magazine size.
        max_shells: i32 = 10 in 1 ..= 50;
        /// Ricochets before a shell is spent.
        shell_ricochet_bounces: u32 = 1 in 0 ..= 5;
    }
    group ai { /* ... */ }
    group cosmetics { /* ... */ }
    group fx { /* shockwave / muzzle / impact ripple tuning */ }
}
```

One row per knob: `name: type = default in min ..= max [@ Applies];`.
`@ Live` is the default and may be omitted. The `///` doc comment is the
same text that sits on today's constant, and it is **captured**, not just
forwarded, so the UI shows it as the row's tooltip. The rich comments in
`lib.rs` therefore survive the move intact, as the project convention
requires.

What the macro expands to:

```rust
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]                     // partial JSON = patch, see §5
pub struct Tuning { pub tank_speed: f32, /* ... every row ... */ }

impl Tuning {
    pub const DEFAULT: Tuning = Tuning { tank_speed: 220.0, /* ... */ };
    /// One entry per row, in declaration order: the table the UI renders.
    pub const SCHEMA: &'static [ParamMeta] = &[ /* ... */ ];
    pub fn get(&self, name: &str) -> Option<f64>;
    pub fn set(&mut self, name: &str, v: f64) -> Result<(), String>; // range-checked
}

pub struct ParamMeta {
    pub name: &'static str,   // "tank_speed"
    pub group: &'static str,  // "movement"
    pub kind: Kind,           // F32 | I32 | U32 | Usize | Bool
    pub doc: &'static str,
    pub min: f64, pub max: f64,
    pub applies: Applies,     // Live | Spawn | Restart
}
```

`Default` is implemented as `Tuning::DEFAULT`. `get`/`set` go through a
small `Knob` trait (`f32`/`i32`/`u32`/`usize`/`bool` -> `f64`), which is
what lets one JSON number type cover every row.

Naming: fields are `snake_case`, so `TANK_SPEED` becomes
`tuning().tank_speed`. `macro_rules!` cannot change the identifier case, and
keeping both spellings would need the `paste` crate for no real gain. The
migration is a mechanical `sed` per constant plus `cargo check`.

### Why not `#[derive(JsonSchema)]` (schemars) instead of a custom macro?

It was considered. A plain struct with serde + schemars derives (proc
macros, but off-the-shelf) gets a JSON Schema with doc strings for free,
and nested structs would give groups. It loses the two things this feature
actually needs per row, `min..=max` in a form the game can enforce, and
`Applies`, and it makes the UI depend on JSON Schema's shape. The
`tunables!` table is ~100 lines, owns its metadata, and is the flat
"table of parameters" the feature is about. Custom macro it is.

### 3.1 Variants: weapons, walls, ammo and tank models

The knob population has two shapes, and they are represented differently.

**Heterogeneous kinds -> one `group` each.** The four weapons (`ActiveWeapon`:
Shell, Minigun, Laser, Plasma) share a few knob *names* (speed, damage
min/max, recoil, hit half-extent, ammo per pickup) but each also has knobs
the others don't: minigun burst size/spread/cycle, laser charges/beam width,
plasma damage factor, shell ricochet bounces and recharge. Forcing those
into one matrix would leave holes, so each weapon is a group, and its ammo
knobs live *in* that group rather than in a separate "ammo" table, because
ammo is per weapon (`max_shells` + `shell_recharge_seconds`,
`minigun_ammo_per_pickup`, `laser_charges_per_pickup`,
`plasma_ammo_per_pickup`). The generic pickup knobs (`pickup_ammo_amount`,
`pickup_heal_amount`, respawn/collect radius) form a `pickups` group. The
UI shows a tab per group, so "Shell / Minigun / Laser / Plasma" reads like a
weapon spec sheet.

**Homogeneous kinds -> array rows, labelled by the enum.** Where the *same*
knob exists once per variant of an enum, the row's type is `[T; N]` and the
row names the label set, which is the enum's declaration order:

```rust
/// Chassis names in scifi_tanks_sheet.png row order (moves here from
/// main.rs's `TankKind`, which then derives its `--tank` names from it).
pub const TANK_NAMES: [&str; 12] = ["scout", "assault", /* ... */ "leviathan"];
/// `obstacle::MATERIALS` order.
pub const MATERIAL_NAMES: [&str; 4] = ["brick", "iron", "wood", "glass"];

tunables! {
    group tank_models {
        /// Chassis mass multiplier - heavier tanks push lighter ones around.
        tank_mass_factor: [f32; 12] = [0.7, 0.9, /* ... */] in 0.2 ..= 5.0 labels TANK_NAMES @ Spawn;
        /// Damage multiplier on everything this chassis fires.
        tank_damage_factor: [f32; 12] = [/* ... */] in 0.2 ..= 5.0 labels TANK_NAMES;
        tank_muzzle_forward_offset: [f32; 12] = [/* ... */] in 0.0 ..= 32.0 labels TANK_NAMES;
        track_weight_opacity: [f32; 12] = [/* ... */] in 0.0 ..= 1.0 labels TANK_NAMES;
    }
    group walls {
        /// Hit points per material; iron plateaus at rust, never destroyed.
        wall_max_health: [f32; 4] = [70.0, 220.0, 35.0, 20.0] in 1.0 ..= 1000.0 labels MATERIAL_NAMES @ Restart;
        wood_flammable_chance: f32 = 0.5 in 0.0 ..= 1.0 @ Restart;
    }
    group laser {
        /// Damage multiplier per `LaserVariant` (Red, Blue).
        laser_damage_factor: [f32; 2] = [1.0, 1.2] in 0.1 ..= 5.0 labels LASER_VARIANT_NAMES;
    }
}
```

Code reads an element by the enum's discriminant, which is what the
existing `match` arms in `obstacle.rs`/`laser.rs`/`plasma.rs` collapse
into: `tuning().wall_max_health[material as usize]`,
`tuning().tank_mass_factor[tank.row as usize]`. A row's type may be `T` or
`[T; N]` with no other change to the grammar. This was compile-checked
alongside the scalar form: the default is taken as one token tree (a
literal or a bracketed list), and an optional `labels NAME` marker selects
the array behaviour for `set`/schema. The one wart: a negative scalar
default must be parenthesised, `(-1.0)`, because `-1.0` is two tokens.

The schema entry for an array row carries `labels` and the element
`kind`; `min`/`max`/`applies` apply to every element. Addressing an
element is `name.label` or `name.index`: `tank_mass_factor.titan`,
`wall_max_health.2`. Setting an array row without an element, or a scalar
with one, is an error.

**What the UI makes of it.** Every array row whose `labels` is the same set
is pivoted into one grid: for `TANK_NAMES` that is a 12-row "Tank models"
table with one column per knob (mass, damage, muzzle offset, track weight),
for `MATERIAL_NAMES` a 4-row "Walls" table. That is the per-model spec sheet
the feature is really for, and the game's own `--tank titan` naming carries
over unchanged. Rows with a label set nobody else uses just render as a
short vertical list inside their group.

**Why not a struct per variant** (`TankModel { mass, damage, ... }` x 12)?
It reads nicer in Rust, but it inverts the table: a knob then belongs to a
model instead of a model belonging to a knob, the macro would need nested
grammars, and the JSON diff of "titan's mass" would be a nested path. The
labelled-array form keeps one flat namespace, one grammar, and the pivot is
a UI concern.

*[impl]* Landed as above, with two small additions: `f64` is also a `Knob`
(for `wood_flammable_chance`), and the schema JSON carries each row's
`default` value alongside the `ParamMeta` fields so the panel can reset a
row without a second call.

**Picking one variant, not tuning all of them: a scalar row with an "unset"
sentinel.** `player_tank` (group `round`) is the first knob whose value *is*
a variant rather than a number per variant: it's an `i32` row over
`-1 ..= 11`, where `0..=11` is a `TANK_NAMES` row index and `-1` means
"unset", deferring to the loaded map's `tank` key and then to `Game::init`'s
random roll (`--tank` outranks the knob either way - see
`simulation::resolve_player_row`). A label set is deliberately *not* used:
`labels` marks a row as an array whose elements are addressed
`name.<label>`, which is the opposite shape - twelve values, one per chassis
- so the panel renders `player_tank` as an integer slider and the doc
comment carries the row->name legend. The sentinel lives in the range rather
than in a second "enabled" bool so the whole choice stays one JSON key, one
slider, and one `tunables!` row.

## 4. Runtime storage and the read path

```rust
// tuning.rs
static TUNING: RwLock<Tuning> = RwLock::new(Tuning::DEFAULT);

#[inline]
pub fn tuning() -> impl Deref<Target = Tuning> { TUNING.read().unwrap() }
```

Call sites read `tuning().tank_speed`. In a hot loop bind once:
`let t = tuning();`. An uncontended `RwLock` read is a few nanoseconds and
the game is single-threaded on both native and wasm, so this is not a
measurable cost.

**Writes happen only at the frame boundary.** Every transport (C API, file
watcher, later HTTP) pushes into `static PENDING: Mutex<Vec<Patch>>`.
`main.rs`'s loop closure (and the probe's loop) calls
`tuning::apply_pending()` **before** `Game::update`. Consequences:

- A frame never observes two values of one knob.
- Nothing inside `simulation/` ever mutates tuning, which keeps the
  "simulation takes plain values" rule and `game.rs`'s "never mutates"
  rule intact.
- The C API needs no pointer to `Game` at all.

Why a global rather than a `Tuning` field on `Game` threaded down as
`&Tuning`? Threading is the purer design, but lib.rs constants are read at
about 400 sites across 20 modules, many in free functions and entity
constructors (`Tank::new`, `Shell` helpers, `ai::steer`) that have no
`Game` in scope. A global accessor makes the migration a is renamed; threading
makes it a signature change on most of the crate. The frame-boundary rule
above recovers the determinism property that threading would have given.

Restart from the panel: the C API pushes a `Command::Restart` onto the same
pending queue, and `main.rs` ORs it into `Input::restart_pressed`. The
simulation stays unaware that a browser exists.

## 5. JSON contract

Three documents, all produced by `tuning.rs` (transport-agnostic):

1. **Schema** (`schema_json()`): `Tuning::SCHEMA` serialized, once, static.
   ```json
   [{"name":"tank_speed","group":"movement","kind":"f32",
     "doc":"Player top speed (px/s).","min":50,"max":600,"applies":"spawn"}, ...]
   ```
2. **Values** (`current_json()`): the whole `Tuning` as a flat object,
   `{"tank_speed":220.0,"max_shells":10,...}`. Also `diff_json()`, only the
   keys that differ from `DEFAULT`, which is what gets saved/shared.
3. **Patch** (`apply_json(&str) -> Result<usize, String>`): a flat object
   with any subset of keys. Unknown keys and out-of-range values are
   rejected as a whole (no partial apply) with a message naming the key.

Array rows (§3.1) serialize as arrays in the values document,
`"wall_max_health":[70,220,35,20]`, and are patched by element with the
dotted key form: `{"tank_mass_factor.titan": 2.5, "wall_max_health.wood": 40}`.
A whole-array replacement (`"wall_max_health":[...]`, exact length) is
accepted too, so a saved diff round-trips without the UI having to expand
it.

`serde_json` becomes a dependency (serde is already there for TOML maps).

A "Copy as Rust" export emits `tank_speed: f32 = 99.0 in ...;` rows for the
diff, so a value found in the browser can be pasted straight back into the
`tunables!` table. That closes the loop: the table is the source of truth,
the browser is a scratchpad.

*[impl]* `tuning()` returns the `RwLockReadGuard` directly. Staging is a
`Mutex<Option<Tuning>>` holding the *next whole table* rather than a queue of
patches: each `submit_json` applies its patch to a copy of whatever is staged
(or live), so a rejected patch never half-applies and a burst of submits
between two frames lands together. Restart is an `AtomicBool`.
`Tank::speed` became `Tank::speed_scale` so the speed knobs are `Live`, not
`Spawn`; `RippleFx::set_tuning` re-uploads shader uniforms when
`apply_pending` reports a change.

## 6. C API (`src/capi.rs`, feature `dev-tools`)

```rust
#[unsafe(no_mangle)] pub extern "C" fn bb_tuning_schema_json() -> *const c_char;
#[unsafe(no_mangle)] pub extern "C" fn bb_tuning_current_json() -> *const c_char;
#[unsafe(no_mangle)] pub extern "C" fn bb_tuning_diff_json() -> *const c_char;
#[unsafe(no_mangle)] pub extern "C" fn bb_tuning_apply_json(json: *const c_char) -> i32; // >=0 keys applied, -1 error
#[unsafe(no_mangle)] pub extern "C" fn bb_last_error() -> *const c_char;
#[unsafe(no_mangle)] pub extern "C" fn bb_tuning_reset();
#[unsafe(no_mangle)] pub extern "C" fn bb_game_restart();
```

- **String ownership**: returned pointers point into a thread-local
  `CString` scratch buffer that stays valid until the next `bb_*` call. No
  `free` dance across the boundary; this is a dev tool, one call at a time.
- **Feature-gated**: `dev-tools = []` in `Cargo.toml`, same convention as
  `map-editor`. Release/production builds do not export anything. The
  *read* side (`tuning.rs`, `tuning()`) is always compiled, it is just the
  defaults with no writer in a release build.
- **Emscripten export**: a `#[no_mangle]` function in a bin crate is dead-
  stripped unless the linker is told to keep it. Add a small `build.rs`
  that, when `CARGO_FEATURE_DEV_TOOLS` is set and the target is
  `wasm32-unknown-emscripten`, emits
  `cargo:rustc-link-arg-bins=-sEXPORTED_FUNCTIONS=_main,_bb_tuning_schema_json,...`.
  This keeps `.cargo/config.toml` static and lets the non-dev wasm build
  link without referencing symbols it doesn't have. (`_main` must be listed
  explicitly whenever `EXPORTED_FUNCTIONS` is set.) `ccall` is already in
  `EXPORTED_RUNTIME_METHODS`; its `'string'` argument/return kinds do the
  UTF-8 marshalling, so the JS side is one line per call.
- Native builds compile the same `extern "C"` functions; they are simply
  unused until a native transport (§8) calls them, or can be exercised
  from a test.

*[impl]* Eight entry points rather than seven (`bb_tuning_diff_rust` and
`bb_last_error` were added, `bb_tuning_reset`/`bb_game_restart` as planned).
The keep-alive is `capi::keep_alive()` called from `main`, not a `#[used]`
static (a `*const ()` table isn't `Sync`). `build.rs` uses
`cargo:rustc-link-arg-bin=bongbong=...` so `probe`'s wasm link (also built by
a bare `cargo build --target wasm32-unknown-emscripten`) isn't asked to
export symbols it never references.

## 7. Web panel (Astro, `site/src/pages/index.astro`)

Under the canvas, a `<details>` "Tuning" panel, rendered entirely from the
schema at runtime (no generated HTML, so a new row in `tunables!` shows up
with zero site changes):

- One `<table>` per `group`; one row per knob: name (doc as tooltip), a
  range slider plus a number input bound to `min..max` (`step` from kind:
  integers 1, floats `(max-min)/1000`), current value, a per-row reset,
  and a badge for `applies = spawn|restart`.
- On change (debounced ~50 ms) -> `Module.ccall('bb_tuning_apply_json',
  'number', ['string'], [JSON.stringify({[name]: value})])`.
- Toolbar: Reset all, Restart round, Copy JSON (diff), Paste JSON, Copy as
  Rust. The diff also persists to `localStorage` and is re-applied in
  `Module.onRuntimeInitialized`, so a reload keeps the tuning.
- Visibility: only when the loaded wasm actually exports the API
  (`typeof Module._bb_tuning_schema_json === 'function'`). The production
  wasm doesn't, so the panel never appears on bongbong.io; no `?dev=1`
  secret handshake needed.
- **PR previews become the QA surface.** `just build-web` grows a
  `just build-web-dev` (`--features dev-tools`), and
  `.github/actions/build-web` gets a `features` input; `pr-preview.yml`
  passes `dev-tools`, `cloudflare-deploy.yml` doesn't. Every
  `pr-<N>.preview.bongbong.io` then has the panel, production never does.

*[impl]* As designed, plus a filter box (185 rows is a lot to scroll) and an
Import textarea instead of a `prompt()` dialog. Verified in Chrome against
the real `just build-web-dev` output: a slider edit reaches the live table at
the next frame, the 12-by-6 tank-model grid and the 4-row walls grid pivot
from the labels, and a reload restores the saved diff.

## 8. Native transports (after web works)

Same `tuning.rs` core, different writer:

1. `--tuning <file.json>` on both `bongbong` and `probe`: apply a diff at
   startup. Trivial, and it gives the probe tuned sweeps immediately.
2. File watch: with `--tuning`, the game polls the file's mtime every
   0.5 s and re-applies it (frame-boundary, as always). Edit the JSON in
   any editor, watch the game react. Zero dependencies, and it composes
   with the existing `cargo watch` habit.
3. Local HTTP/MCP: a `--tuning-port` that serves the same three JSON
   documents over localhost, which is exactly the surface an MCP server
   needs (`get_schema`, `get_tuning`, `set_tuning`, `restart`). Deferred;
   the JSON contract in §5 is designed so this is purely additive.

*[impl]* Items 1 and 2 landed (`--tuning` on both binaries; the game polls
the file's mtime every 30 frames). Item 3 is still open.

## 9. Determinism, the probe, and "applies"

- `determinism_tests` and the fixture baselines run at `Tuning::DEFAULT`
  with no writer, so they are unaffected.
- A tuned run is still reproducible: `(seed, tuning diff)` is the replay
  pair. The probe prints a hash of the active diff in its header and writes
  the diff into each `--json-out` record next to the seed.
- Mid-round edits are, by definition, a divergence from the seeded replay.
  That is fine for hand tuning and is why the panel's "Restart round"
  button exists: tune, restart, watch the whole round under the new values.

Three `Applies` classes, chosen per row:

| Class | Meaning | Examples |
| --- | --- | --- |
| `Live` (default) | Read fresh every frame or on every new entity/shot. Change is felt immediately. | fire intervals, damage ranges, AI ranges, cosmetics, ripple FX |
| `Spawn` | Baked into an entity at spawn (`Tank::speed` is set from `TANK_SPEED`/`ENEMY_SPEED` in `Tank::new`/`Game::init`). Affects new spawns and the next restart. | tank speeds, `max_shells` |
| `Restart` | Consumed by `Game::init` only. | enemy count bounds, spawn margins, frog spawn distances |

For the knobs you most want to *feel* (player/enemy speed), phase 2 turns
`Spawn` into `Live` by storing a per-tank **factor** (the `ENEMY_SPEED_VARIANCE`
roll) instead of an absolute speed, and computing `factor * tuning().enemy_speed`
in `effective_speed`. The badge in the UI tells you which knobs still lag
until restart, so nothing silently "doesn't work".

## 10. Migration hazards (found while surveying the call sites)

- **Derived constants**: `ENGAGE_RING_RADIUS = ENEMY_ATTACK_RANGE * 0.8`,
  `ENGAGE_RESERVE_RADIUS`, `ENEMY_RETREAT_RANGE`. They must stay derived, or
  dragging `enemy_attack_range` silently breaks the engagement ring. They
  become methods on `Tuning` (`fn engage_ring_radius(&self) -> f32`),
  declared in a `derived { ... }` block the macro forwards verbatim.
- **Divide-by-the-constant-that-made-it**: a projectile's velocity is
  built as `dir * SHELL_SPEED` (likewise `PLASMA_SPEED`,
  `MINIGUN_BULLET_SPEED`) at spawn. Any later code that recovers the
  direction by dividing by the same constant is wrong the moment
  `shell_speed` changes with that projectile in flight. The old
  `simulation.rs` had one such site; the simulation/ split (2026-09-02)
  removed it, but the rule stands for migration: normalise by the stored
  vector's own length, never by the constant. `grep` for `/ *[A-Z_]*SPEED`
  before each group lands.
- **The simulation is now a directory module** (`src/simulation/{mod,
  weapons,hits,combat,engage}.rs`), each importing its own constants, so
  the rename touches those files individually plus `tank.rs`/`ai.rs`.
  `lib.rs` also gained `RAM_DAMAGE_MIN/MAX` (a `combat` group knob).
- **Const contexts**: a tunable cannot be used as an array length, in a
  `match` pattern, or to initialise another `const`. `cargo check` catches
  every one of these after the rename; the survey found none among the
  knob population (the `match` hits are arm bodies, not patterns).
- **`RippleTuning` copies**: `main.rs` builds the three `RippleFx` once with
  values from `lib.rs`. Re-copy from `tuning()` each frame (three tiny
  structs) so the `fx` group is live.
- Comments that cite a constant by its old upper-case name are fine to
  leave; comments that *restate* its value should keep pointing at the
  table rather than the number, per the existing lib.rs convention.

## 11. Phasing

1. **Spike (web, end to end, ~10 knobs).** `tuning.rs` + `tunables!`,
   `serde_json`, `capi.rs` behind `dev-tools`, `build.rs` export list,
   `apply_pending()` in `main.rs`, the Astro panel, `just build-web-dev`.
   Knobs: player/enemy speed, accel, turn grip, shell speed, damage ranges,
   fire intervals, `max_shells`. Proves the whole pipe before the big rename.
2. **Migrate the rest** of the knob population (~150 rows), derived
   constants -> methods, `Spawn` -> `Live` for speeds, `RippleTuning`
   re-copy, module-local consts. Mostly mechanical; do it group by group
   with `cargo check` + `cargo test --lib` + `just probe-fixtures` green
   after each group (baselines must not move at `DEFAULT`). The per-weapon
   groups, the `walls` array row and the `tank_models` array rows (§3.1)
   are part of this phase, not deferred: the array grammar is already
   verified and the `match`-to-index collapse in `obstacle.rs`/`laser.rs`/
   `plasma.rs` is a few lines each. The UI pivot for labelled rows lands
   with them.
3. **CI**: `features` input on the composite action; PR previews build
   with `dev-tools`.
4. **Native**: `--tuning` on both binaries, file watch, probe header/JSON
   hash.
5. **Later**: localhost HTTP + MCP; a read-only "live stats" JSON (fps,
   outcome, tank snapshots) over the same C API for the panel to display.

## 12. Open questions

- Should `dev-tools` wasm be a separate artifact name (`bongbong-dev.wasm`)
  so a PR preview can host both the QA build and the exact production
  build? Cheap to add; default answer is no until someone wants it.
- Bool knobs (e.g. `shadows_enabled`) currently live on `Game`, not in
  `lib.rs`. Moving them into the table makes them tunable from the panel
  but duplicates the `L` key toggle; probably worth it, decide in phase 2.
