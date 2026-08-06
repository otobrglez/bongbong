# Physics engine revamp — design doc

Status: all 6 migration phases complete - see "Migration plan" for
per-phase notes, deviations from the original plan (and why), and how each
was verified.

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

**Not done** - flagging this honestly rather than pretending otherwise.
`Owner`-based `if` filtering is still exactly how `Game::update` decides
which shell can hit which tank; phase 5 only replaced *how a hit is
detected* (physics intersection vs. point-in-box), not *who's allowed to
check whom*. Moving that to rapier's `InteractionGroups` (bitmask
membership/filter on each collider) remains a legitimate future cleanup -
it would delete the nested `if shell.owner != Owner::Player` /
`if shell.owner == Owner::Player` branches in the shell-hit loop - but
wasn't necessary for anything this migration needed to work, so it was
left alone rather than done for its own sake.

### AI Decoupling

The AI (`ai.rs`) relies heavily on predicting collisions. Instead of querying the Rapier world (which would couple the AI to the physics state), the AI will continue using its lightweight `Mover` structs for prediction. This ensures AI logic remains fast and abstract, while Rapier is purely an execution layer for physical interactions.

### Per-frame flow (replacing the body of `Game::update`)

1. Read input → write each tank's commanded cardinal velocity onto its body.
2. Apply any queued impulses (ram/explosion/impact knockback) via `apply_impulse`.
3. Accumulate `dt` and step the Rapier pipeline in fixed intervals (e.g., `1/60s`).
4. Drain contact/intersection events: tank-vs-wall requires no game code; tank-vs-tank triggers ram damage on a cooldown; shell-vs-tank triggers damage/detonation.
5. Read resulting position back from the rigid bodies into `Tank`/`Shell` structs for rendering. `Tank`/`Shell` retain game-specific fields (`damage`, `shells_ammo`, etc.).

## Data model sketch

- Below is what was actually built (see "Migration plan" for the reasoning
  behind each deviation from the original sketch above it).
- `src/physics.rs`'s `Physics` wraps rapier's own `PhysicsWorld` convenience
  bundle (already provides `RigidBodySet`/`ColliderSet`/broad+narrow-phase/
  `IntegrationParameters`/etc. in one struct - no need to re-declare them by
  hand) plus small helpers: `spawn_wall`, `spawn_tank`, `add_hit_sensor`,
  `spawn_shell`, `set_velocity`/`velocity`, `apply_impulse`,
  `set_kinematic_position`, `position`, `touching`, `intersecting`,
  `collider_of`, and a `Position <-> glam::Vec2` conversion pair (rapier
  2D/f32 uses glam, not nalgebra, for its `Vector` type).
- `Tank` gains `body: Option<RigidBodyHandle>` (the solid hull collider) and
  `hit_sensor: Option<ColliderHandle>` (a second, sprite-sized sensor
  collider used only for shell hits - see phase 5's note on why one
  collider wasn't enough). `Tank::knockback` is gone (see phase 4: the
  residual is derived fresh each frame instead of stored).
- `Shell` gains `body: Option<RigidBodyHandle>` (a kinematic sensor; see
  phase 5).
- `Game` owns one `Physics` instance plus a `physics_accumulator: f32` for
  the fixed-timestep loop. `ram()`/`explosion_hit()` take `&mut Physics` and
  call `apply_impulse` directly on each tank's own body.

## Platform note: wasm/web target

`sola-raylib`'s web build targets `wasm32-unknown-emscripten`. `rapier2d` is pure Rust and compiles cleanly for JS/WASM. **Crucial constraint**: leave rapier2d's `parallel` (rayon) feature disabled, as the web build operates single-threaded via Emscripten's `ASYNCIFY`. The vendored dependency correctly omits it.

## Migration plan (phased — each phase stays independently playable)

1. **Vendor the engine, no behavior change.** (Completed: `rapier2d` added
   without `parallel`; `src/physics.rs` wraps rapier's own `PhysicsWorld`
   convenience bundle rather than re-declaring its fields by hand, with
   gravity zeroed; `Game` now owns a `Physics` and steps it every unpaused
   frame, still with zero bodies.)
2. **Walls + cardinal-driven bodies, no impulses.** (Completed: every tank is
   a rotation-locked dynamic body with a hull-sized cuboid collider; 4 static
   wall colliders replace `clamp_to_field`; `Ai::deflect_from_walls` and
   `wall_follow` are deleted (`heads_into_wall` stays - predictive dodge
   filtering still wants it); `Game::update` drains a fixed-timestep
   accumulator (`PHYSICS_FIXED_DT`, capped by `PHYSICS_MAX_CATCHUP_SECONDS`)
   into `Physics::step`. One deliberate scope note: giving tanks solid
   colliders forces tank-vs-tank blocking to become physical *right now* -
   two solid dynamic bodies can't be told "collide, but don't actually
   collide yet" - so the old `overlaps()`-revert-and-shove dance in
   `apply_movement_player`/`_enemy` is already gone, ahead of where step 3
   below originally placed it. Knockback is *not* yet a real impulse: it's
   still hand-decayed on `Tank` and folded into the velocity written to the
   body each frame (`velocity + knockback`), exactly matching "no impulses
   yet". Verified in-browser via the wasm build: a temporary debug check
   logging any tank-hull pair found closer than their combined half-hulls
   found zero violations across many rounds of AI combat and player-driven
   ramming, and tanks never render outside the wall-bounded battlefield.)
3. **Tank-vs-tank ram damage as events.** (Completed, with one deliberate
   implementation swap from the original plan: rather than a channel-based
   `CollisionEvent` stream, `Physics::touching(a, b)` queries rapier's own
   narrow-phase directly - `PhysicsWorld::contact_pair(...).has_any_active_contact()`
   - each frame. This achieves the same goal - ram-damage triggering reads
   rapier's authoritative contact state instead of a hand-rolled geometric
   re-check - without extra channel/event-handler plumbing, and it preserves
   the existing "sustained contact re-ticks damage every cooldown" behavior
   exactly, which a one-shot `Started` event would *not* have done on its
   own. `Tank::overlaps` is deleted - dead code once nothing calls it.
   Verified in-browser: clean build, zero console errors/panics across
   several rounds, ram/shell damage and physical blocking both still work as
   in step 2.)
4. **Knockback → impulses.** (Completed, with one important scope correction
   discovered during implementation: `KNOCKBACK_DAMPING` is **not** deleted.
   Ram/explosion knockback is now a real `physics.apply_impulse` call - each
   tank's collider mass is set at spawn (`spawn_tank`'s `mass` param) to match
   `Tank::mass()`, so the impulse/mass division naturally reproduces the old
   hand-rolled mass-weighted split with no separate formula. But letting
   rapier's own `linear_damping` fully own the decay turns out to be
   incompatible with "commanded movement snaps instantly to an exact speed,
   including a dead stop on release, no momentum" (an explicit goal above):
   a single rapier velocity vector has no way to say "this part is
   instantly-reassertable, this part decays" on its own. `Game::drive_tank`
   resolves this by deriving the residual (knockback) velocity fresh every
   frame - `current_body_velocity - what_we_commanded_last_frame` - decaying
   *that* explicitly with `KNOCKBACK_DAMPING`, then adding this frame's fresh
   commanded velocity back on top before writing to the body. `Tank.knockback`
   the *field* is gone (no longer stored - the residual is now derived, not
   tracked), which is the meaningful simplification this step actually
   delivers. Verified in-browser: no console errors across several rounds;
   two consecutive screenshots with no key input showed zero pixel drift on
   any tank, confirming no coasting was introduced; ram/explosion
   damage-and-shove still visibly functioning.)
5. **Shells as sensors.** (Completed, with one scope addition discovered
   during implementation: a tank now carries **two** colliders, not one.
   Reusing the existing hull collider (sized to `Tank::hull_size`, used for
   tank-vs-tank/wall blocking) for shell-hit detection too would have
   quietly shrunk the target - the old `Tank::contains` hit-tested against
   the tank's *full sprite* (`Tank::size`), a deliberately bigger, more
   forgiving hit box. So `physics::Physics::add_hit_sensor` attaches a
   second, sprite-sized *sensor* collider to each tank body specifically for
   shell intersection, leaving the solid hull collider untouched. Shells are
   `RigidBodyBuilder::kinematic_position_based` bodies with `ccd_enabled` and
   a small sensor collider (`SHELL_HIT_HALF_EXTENT`, kept near-point-sized so
   the intersection reads like the old point-in-box check, not a box-vs-box
   one); position is still hand-integrated every frame
   (`Shell::update`/`velocity * dt`) and pushed into the kinematic body via
   `set_kinematic_position` *before* the physics step, so intersection
   queries after that step see this frame's movement - this is why the shell
   movement/animation step moved earlier in `Game::update`, ahead of the
   physics-stepping block, while the hit-detection/damage step stayed where
   it was (after). As in phase 3, intersection is read via a direct
   `PhysicsWorld::intersection_pair` query each frame
   (`Physics::intersecting`) rather than a `CollisionEvent` channel, for the
   same reasons. `Tank::contains` is deleted - dead code once nothing calls
   it. Verified in-browser: no console errors across two full rounds
   including a kill, a round restart (which tears down and rebuilds the
   entire physics world, including every shell's body), and continuous
   shell fire/hit/despawn cycles; shell and ram damage both visibly still
   land (HP dropped from full to 0 in one observed exchange).
6. **Cleanup pass.** Purge dead constants and run a final clippy/fmt hygiene pass.
