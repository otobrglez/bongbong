# Physics engine revamp — design doc

Status: approved for implementation.

## Goals

- Replace the hand-rolled collision/knockback math in `game.rs`/`tank.rs` with
  a real 2D physics engine (**rapier2d**) as the single source of truth for
  body-body interaction: contacts, impulses, mass, restitution.
- Preserve the current *feel* of driving exactly: player and AI movement stays
  strictly cardinal (`Dir::Up/Down/Left/Right`), constant speed while a
  direction is held, instant stop on release, no acceleration/inertia on
  commanded movement.
- Make all *non-commanded* motion — ram knockback, explosion knockback, and
  future projectile impacts / environmental pushes — real physics
  impulses/forces resolved by the engine's solver, instead of the current
  hand-written formulas in `Game::ram`/`explosion_hit`/`Tank::apply_knockback`.
- Build a robust, scalable foundation general enough to support planned future mechanics
  (obstacles/terrain, projectile arcs/bounces, more entity types) without
  another rewrite.
- Ensure deterministic and consistent simulation irrespective of rendering framerates
  by introducing a strict fixed-timestep accumulator.

## Non-goals (this pass)

- Free-angle/analog driving. Explicitly staying cardinal-only for commanded
  movement — see "Layering" below for how that coexists with a real physics
  body.
- Networked/lockstep determinism. Not needed now; rapier's float solver isn't
  cross-platform-deterministic without extra work.
- Touching the AI's decision-making (`ai.rs`/`bt.rs`). Some of its hand-rolled
  collision/wall-avoidance code should get *deleted* as a side effect (see
  Migration), but the behavior tree and steering targets aren't being redesigned.

## Current state (why this is worth doing)

Everything today is per-frame hand math, spread across `Tank` and `Game`:

- `Tank::control()` sets `rotation`/`position` directly from a cardinal `Dir` —
  no acceleration, matches the goal above already.
- `Tank::knockback` is a second velocity-like field, integrated and decayed
  by hand in `apply_knockback` (`KNOCKBACK_DAMPING`).
- Collision is an AABB overlap check (`Tank::overlaps`, cheap because hulls
  are always axis-aligned squares — tanks only ever face cardinal). On
  overlap, `Game::apply_movement_player/_enemy` just **reverts position to
  before the move** and calls `ram()`, which hand-computes a mass-weighted
  push into `knockback`.
- Explosions (`explosion_hit`) duplicate that same falloff-and-push math
  independently.
- Screen bounds are a manual `clamp_to_field`; the AI additionally hand-rolls
  wall-sliding (`Ai::deflect_from_walls`, `wall_follow`) entirely in
  `ai.rs` to route around the fact that physics doesn't do it for them.
- Shells are pure `position += velocity * dt` with a point-in-square hit test
  (`Tank::contains`) — no engine involvement at all.
- Variable framerate integration: `rl.get_frame_time()` is currently used directly.

This works at current scope, but every one of those is a dead end for the
features you want next: it doesn't generalize to obstacles, non-square hulls,
projectile arcs, or bounces — and the collision response (revert-and-shove)
already visibly doesn't slide, it just stops. Further, variable timesteps cause
subtle visual stutter and non-deterministic physics.

## Why rapier2d

- Pure Rust, no C bindings — fits the project's minimal-dependency style
  (`Cargo.toml` currently has 4 deps).
- Gives us broad/narrow-phase collision detection, a real contact/impulse
  solver, rigid body types (dynamic/kinematic/fixed), shaped colliders,
  collision groups/filters, contact & intersection event queues, and a CCD
  (Continuous Collision Detection) solver for fast movers (shells) — all things currently hand-rolled or missing.
- Pulls in `nalgebra` for its math types, which don't match `sola_raylib`'s
  `Vector2` (`Position`'s current alias) — needs a small conversion shim, not
  a new abstraction layer.

## Proposed architecture

### Layering: commanded movement vs. physics-driven displacement

This is the crux of "cardinal driving stays, but interactions should be free
physics." Two velocity contributions feed into the *same* rigid body each frame:

1. **Commanded velocity** — from `Tank::control()`'s Dir input, unchanged
   logic: full `effective_speed()` on that axis, zero otherwise. Written
   directly onto the rigid body's `linvel` each frame, overwriting whatever
   was there — this is the "the player/AI is actively steering" signal, and
   it's why driving still feels identical.
2. **Physics-driven displacement** — ram knockback, explosion knockback, and
   any future impact push, applied as **impulses** (`apply_impulse`) to that
   same body. The solver integrates these and — this is the actual payoff —
   resolves them against contacts with other bodies and walls automatically,
   so a knock into another tank or a wall gets naturally blocked/redirected
   by the solver instead of hand-written wall-reflect code.

Net effect: same mental model as today (control-velocity + knockback,
summed), but the engine now owns collision resolution and impulse math
instead of `Game::ram`/`explosion_hit`/`apply_knockback`.

### Timestep Accumulator (New Requirement)

Feeding variable framerates (`rl.get_frame_time()`) directly into Rapier creates inconsistent collision responses. We must introduce a **fixed timestep accumulator** (e.g., `1/60` of a second). `Game::update` will accumulate real-time `dt` and consume it in fixed chunks by stepping the physics pipeline.

### Entities

- **Tank** (player + each enemy): one **dynamic** rigid body, cuboid collider
  sized from `hull_size()` (matches today's square hull). **Rotation locked**: 
  driving stays cardinal, so the body itself never spins physically. This prevents floating-point micro-rotations from causing axis-aligned sprites to jitter. Sprite facing is managed via the existing cosmetic `Dir::rotation()`.
- **Shell**: a **sensor** collider per in-flight shell. Shells do not get knocked around and they do not push tanks. They generate intersection *events* that trigger the existing damage/`detonate()` logic. Because shells move fast, they must have **CCD (Continuous Collision Detection) enabled** to avoid tunneling through walls or tanks in a single frame. Position is still managed as kinematic data.
- **Battlefield bounds**: 4 static wall colliders at the screen edges,
  replacing `clamp_to_field` — tanks colliding with them get real sliding for
  free, allowing deletion of `deflect_from_walls`/`wall_follow`.
- **(future) Obstacles/terrain**: more static colliders.

### Collision groups (replacing `Owner`-based filtering)

Define membership groups using Rapier's `InteractionGroups` — `Player`, `Enemy`, `PlayerShell`, `EnemyShell`, `Wall`. Filter masks will ensure, for example, an `EnemyShell` only triggers intersections against `Player` bodies and `Wall`s. This removes nested `owner != Owner::Player` if-statements.

### AI Decoupling

The AI (`ai.rs`) relies heavily on predicting collisions. Instead of querying the Rapier world (which would couple the AI to the physics state), the AI will continue using its lightweight `Mover` structs for prediction. This ensures AI logic remains fast and abstract, while Rapier is purely an execution layer for physical interactions.

### Per-frame flow (replacing the body of `Game::update`)

1. Read input → write each tank's commanded cardinal velocity onto its body.
2. Apply any queued impulses (ram/explosion/impact knockback) via `apply_impulse`.
3. Accumulate `dt` and step the Rapier pipeline in fixed intervals (e.g., `1/60s`).
4. Drain contact/intersection events: tank-vs-wall requires no game code; tank-vs-tank triggers ram damage on a cooldown; shell-vs-tank triggers damage/detonation.
5. Read resulting position back from the rigid bodies into `Tank`/`Shell` structs for rendering. `Tank`/`Shell` retain game-specific fields (`damage`, `shells_ammo`, etc.).

## Data model sketch

- New module `src/physics.rs`: owns the rapier `PhysicsPipeline`,
  `RigidBodySet`, `ColliderSet`, `IslandManager`, `BroadPhase`, `NarrowPhase`, `ImpulseJointSet`, `MultibodyJointSet`, `CCDSolver`, and `IntegrationParameters`. It exposes small helpers (`spawn_tank_body`, `spawn_shell_sensor`, `spawn_wall`) and a small `Position <-> nalgebra::Vector2` converter.
- `Tank`/`Shell` gain a `body: RigidBodyHandle` (`collider: ColliderHandle` for shells). `Tank::knockback` goes away.
- `Game` owns one `Physics` instance. `ram()` and `apply_explosion()` simply apply impulses to handles.

## Platform note: wasm/web target

`sola-raylib`'s web build targets `wasm32-unknown-emscripten`. `rapier2d` is pure Rust and compiles cleanly for JS/WASM. **Crucial constraint**: leave rapier2d's `parallel` (rayon) feature disabled, as the web build operates single-threaded via Emscripten's `ASYNCIFY`. The vendored dependency correctly omits it.

## Migration plan (phased — each phase stays independently playable)

1. **Vendor the engine, no behavior change.** (Completed: `rapier2d` added
   without `parallel`; `src/physics.rs` wraps rapier's own `PhysicsWorld`
   convenience bundle rather than re-declaring its fields by hand, with
   gravity zeroed; `Game` now owns a `Physics` and steps it every unpaused
   frame, still with zero bodies.)
2. **Walls + cardinal-driven bodies, no impulses.** Tanks become rotation-locked dynamic bodies. Replace `clamp_to_field` with static colliders. Delete AI wall-following hacks. Ensure `Game::update` operates on a fixed timestep accumulator.
3. **Tank-vs-tank contact + ram damage.** Hook up Rapier's `CollisionEvent` queue. Let the solver push tanks apart naturally while applying ram damage. Delete hand-rolled `overlaps()` revert logic.
4. **Knockback → impulses.** Convert ram and explosion knockbacks to `apply_impulse`. Delete manual decay constants (`KNOCKBACK_DAMPING`).
5. **Shells as sensors.** Convert shells to kinematic CCD sensors. Hook intersection events to detonate logic. Drop `Tank::contains`.
6. **Cleanup pass.** Purge dead constants and run a final clippy/fmt hygiene pass.
