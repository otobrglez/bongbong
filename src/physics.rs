//! The game's physics simulation (see docs/physics-engine-design.md).
//!
//! Tanks are rotation-locked dynamic bodies, accelerated toward their
//! commanded velocity by a mass-aware impulse every frame rather than
//! snapped to it (see `Game`'s `drive_tank`); the battlefield edges are
//! static wall colliders. Ram/explosion/shell-impact knockback is a real
//! impulse too (`apply_impulse`). Shells are kinematic-position-based sensor bodies
//! (`spawn_shell`) - their position is still hand-moved every frame (see
//! `Shell::update`), not driven by velocity; the physics engine's only job
//! for them is precise intersection detection against tanks.

use rapier2d::prelude::*;

use crate::{Position, TANK_MOVE_CORNER_RADIUS};

/// Owns the rapier simulation state. Wraps rapier's own `PhysicsWorld`
/// convenience bundle (rigid bodies, colliders, broad/narrow-phase, solver,
/// `IntegrationParameters`, ...) rather than re-declaring those fields by
/// hand, with gravity zeroed out - bongbong is top-down, so there's no axis
/// for anything to fall along.
pub struct Physics {
    world: PhysicsWorld,
}

/// The collision-filtering group for a tank "owner" slot - slot 0 is the
/// player, slot `n` (n >= 1) is `Owner::Enemy(n - 1)`. Used to tell rapier
/// "this shell's sensor shouldn't intersect that tank's hit sensor" via
/// `InteractionGroups` (see `add_hit_sensor`/`spawn_shell`), replacing the
/// old manual `if shell.owner == ...` self-exclusion checks in
/// `Game::update`'s shell-hit loop. `ENEMY_COUNT_MAX` (10) plus the player
/// leaves 21 of `Group`'s 32 bits unused for future owner-like filtering.
pub fn owner_group(slot: usize) -> Group {
    debug_assert!(slot < 32, "owner slot must fit in rapier's 32 group bits");
    Group::from_bits_retain(1 << slot)
}

/// The corner rounding actually applied to a tank movement collider with
/// these overall `half_extents`: TANK_MOVE_CORNER_RADIUS, clamped to half
/// the smaller half-extent so the collider's core box (which
/// `tank_move_shape` shrinks by this radius) always keeps real area even
/// for the smallest hull in the roster. Public so the "I"-key inspect
/// overlay (`game.rs::draw_tank_inspect`) can draw the exact rounding the
/// physics body carries rather than re-deriving (and possibly drifting
/// from) this clamp.
pub fn tank_corner_radius(half_extents: (f32, f32)) -> f32 {
    TANK_MOVE_CORNER_RADIUS.min(half_extents.0.min(half_extents.1) * 0.5)
}

/// The shared shape for a tank's solid movement collider: a round cuboid
/// whose *overall* footprint (core box dilated by the border radius -
/// that's parry's `RoundShape` semantics, the radius grows the shape
/// outward) is exactly the given `half_extents`, with
/// `tank_corner_radius`-rounded corners. Rounded rather than sharp because
/// box-vs-box corners catch on the internal seams between adjacent
/// wall/obstacle cell colliders (tanks visibly snagged mid-slide along a
/// flat wall run) - a rounded corner slides past instead. Used by both
/// `spawn_tank` and `resize_collider` so spawn and facing-change resize can
/// never disagree about the shape.
fn tank_move_shape(half_extents: (f32, f32)) -> SharedShape {
    let r = tank_corner_radius(half_extents);
    SharedShape::round_cuboid(half_extents.0 - r, half_extents.1 - r, r)
}

impl Physics {
    pub fn new() -> Self {
        Self {
            world: PhysicsWorld {
                gravity: to_vector(Position::new(0.0, 0.0)),
                ..PhysicsWorld::default()
            },
        }
    }

    /// Advance the simulation by one fixed step. `IntegrationParameters::dt`
    /// defaults to 1/60s (see `PHYSICS_FIXED_DT`); `Game::update` drives this
    /// from a fixed-timestep accumulator so contact resolution stays
    /// consistent regardless of the render frame rate.
    pub fn step(&mut self) {
        self.world.step();
    }

    /// Spawn a static, fixed-body cuboid collider: the battlefield boundary
    /// (see `battlefield::spawn_walls`) and in-arena obstacles (see
    /// `obstacle::Obstacle`) both reuse this exact same shape - the only
    /// difference is whether the caller ever calls `remove_body` on the
    /// handle later (walls never do; a destroyed `Crate` obstacle does).
    /// `center` and `half_extents` are in the same pixel space as
    /// `Tank`/`Shell` positions; rapier does no unit conversion here (1
    /// physics unit == 1px).
    pub fn spawn_static(&mut self, center: Position, half_extents: Position) -> RigidBodyHandle {
        let (handle, _) = self.world.insert(
            RigidBodyBuilder::fixed().translation(to_vector(center)),
            ColliderBuilder::cuboid(half_extents.x, half_extents.y),
        );
        handle
    }

    /// Spawn a rotation-locked dynamic tank body: a corner-rounded
    /// rectangular collider (`tank_move_shape`) of overall `half_extents`
    /// (x, y) per side - callers pass `Tank::move_half_extents`, the
    /// shrunken movement box, not the full hull damage box - centered at
    /// `position`, with mass set to `mass` (matching `Tank::mass()`) so
    /// impulses applied later (see `apply_impulse`) produce the exact
    /// velocity change the caller intended rather than whatever rapier's
    /// default density would give. Rotation is locked because driving stays
    /// cardinal (see docs/physics-engine-design.md) - the body only ever
    /// needs to translate, never spin; sprite facing stays the existing
    /// cosmetic `Tank::rotation`/`Dir::rotation`. Since the *collider*
    /// itself never rotates either, a non-square tank's `half_extents` need
    /// to be reoriented by hand whenever its facing changes between an
    /// X-axis and Y-axis cardinal direction - see `resize_collider` and
    /// `simulation::drive_tank`.
    pub fn spawn_tank(
        &mut self,
        position: Position,
        half_extents: (f32, f32),
        mass: f32,
    ) -> RigidBodyHandle {
        let (handle, _) = self.world.insert(
            RigidBodyBuilder::dynamic()
                .translation(to_vector(position))
                .lock_rotations(),
            ColliderBuilder::new(tank_move_shape(half_extents)).mass(mass),
        );
        handle
    }

    /// Resize a tank's solid movement collider (the one `spawn_tank`
    /// attaches) to new overall `half_extents` (`Tank::move_half_extents`,
    /// same as at spawn; the corner rounding is re-derived via
    /// `tank_move_shape`) - used when a tank's facing swaps between an
    /// X-axis and Y-axis cardinal direction, so its hitbox stays oriented
    /// to match its sprite (see `simulation::drive_tank`, which calls this
    /// only when `Tank::rotation` actually changes). `set_shape` only marks
    /// the collider's geometry dirty - it doesn't touch the collider's
    /// explicit `.mass(mass)` override from `spawn_tank`, so resizing never
    /// perturbs ram/explosion knockback math.
    pub fn resize_collider(&mut self, handle: ColliderHandle, half_extents: (f32, f32)) {
        let collider = self
            .world
            .colliders
            .get_mut(handle)
            .expect("collider handle should always be valid");
        collider.set_shape(tank_move_shape(half_extents));
    }

    /// Attach an extra sensor collider to an existing tank body, used only to
    /// detect shell hits - a separate collider from what blocks movement
    /// (the solid movement collider `spawn_tank` attaches), because a shell's
    /// owner-exclusion `InteractionGroups` filter (see `group` below) can't
    /// share a collider with movement collision, which needs to interact
    /// with *everything* (every other tank, every wall/obstacle) regardless
    /// of ownership. A tank carries two of these - one sized/positioned to
    /// the hull box, one to the turret+barrel box (`Tank::hull_bbox_world`/
    /// `turret_bbox_world`) - so a shot registers across the tank's actual
    /// visible silhouette without also counting the sprite tile's
    /// transparent padding as a hit. `half_extents`/`offset` are in the
    /// body's own frame; since a tank body's rotation is locked (see
    /// `spawn_tank`), that frame never itself rotates, so `offset` is
    /// exactly the world-space offset from the tank's position at the
    /// moment this is called - `resize_hit_sensor` updates both when the
    /// tank's facing changes. Returns the new collider's handle - callers
    /// need it directly for `intersecting`, since a tank body now has three
    /// colliders total and `collider_of`'s "the first one" convention only
    /// ever refers to the original solid movement collider `spawn_tank`
    /// attaches.
    ///
    /// Explicitly zero-mass: rapier still folds a collider's density-based
    /// mass into its body's total mass even when `.sensor(true)` - being a
    /// sensor only exempts it from collision *response*, not mass-property
    /// aggregation. Left at the default density (1.0), a sensor's area would
    /// silently perturb the body's total mass on top of the hull's own
    /// explicit mass (`Tank::mass`) - which is exactly what was making tanks
    /// crawl back when this sensor covered the tank's full sprite tile:
    /// `Game::drive_tank`'s impulse is sized for `Tank::mass()`, but rapier
    /// was dividing by the real (much larger) body mass instead.
    ///
    /// `group` (see `owner_group`) is this tank's own membership bit - a
    /// shell fired by this same tank sets its filter to exclude that bit
    /// (see `spawn_shell`), so `intersecting` naturally reports no hit
    /// against its own shooter without any owner-equality check on the
    /// call site.
    pub fn add_hit_sensor(
        &mut self,
        body: RigidBodyHandle,
        half_extents: (f32, f32),
        offset: Position,
        group: Group,
    ) -> ColliderHandle {
        self.world.insert_collider(
            ColliderBuilder::cuboid(half_extents.0, half_extents.1)
                .translation(to_vector(offset))
                .sensor(true)
                .mass(0.0)
                .collision_groups(InteractionGroups::new(
                    group,
                    Group::ALL,
                    InteractionTestMode::And,
                )),
            Some(body),
        )
    }

    /// Resize and reposition an existing hit-sensor collider (see
    /// `add_hit_sensor`) - called whenever a tank's facing crosses between
    /// an X-axis and Y-axis cardinal direction, the same trigger
    /// `resize_collider` reacts to for the solid hull collider (see
    /// `simulation::drive_tank`). `set_shape` only marks the geometry dirty,
    /// same as `resize_collider`; `set_translation_wrt_parent` moves the
    /// sensor's offset within the (non-rotating) body frame to match.
    pub fn resize_hit_sensor(&mut self, handle: ColliderHandle, half_extents: (f32, f32), offset: Position) {
        let collider = self
            .world
            .colliders
            .get_mut(handle)
            .expect("collider handle should always be valid");
        collider.set_shape(SharedShape::cuboid(half_extents.0, half_extents.1));
        collider.set_translation_wrt_parent(to_vector(offset));
    }

    /// Spawn a kinematic-position-based sensor body for a shell: a square
    /// sensor `half_extent` per side. A sensor never gets physically pushed
    /// and never pushes anything else - it only ever reports whether it's
    /// intersecting something (see `intersecting`).
    ///
    /// `.ccd_enabled(true)` is set but doesn't actually protect this body:
    /// per rapier's own docs (`RigidBody::enable_ccd`), CCD sweeping only
    /// applies to **dynamic** bodies moving fast under velocity integration.
    /// A kinematic-position-based body's motion is a discrete teleport via
    /// `set_kinematic_position` each frame, which CCD never sweeps - so a
    /// shell whose per-frame movement is ever large enough to fully clear a
    /// thin target in one step (e.g. under a frame-rate hitch) can still
    /// tunnel through it undetected by *this* check. In practice shells move
    /// ~8px/frame at SHELL_SPEED and everything they can hit is comfortably
    /// wider than that, so this was long treated as a latent edge case, not
    /// the thing `.active_collision_types` below fixes - but it did show up
    /// in practice (reported as shells occasionally flying straight over an
    /// unbroken Glass obstacle, the fastest-dying material). The actual fix
    /// is `simulation::swept_shell_target` - a hand-rolled segment-vs-box
    /// sweep checked as a fallback whenever this discrete end-of-frame
    /// check finds nothing, covering the shell's whole path for the frame
    /// instead of just where it ended up. `.ccd_enabled(true)` is left here
    /// as a harmless no-op rather than removed, in case rapier ever extends
    /// real CCD to kinematic bodies.
    ///
    /// `.active_collision_types` explicitly re-enables `KINEMATIC_FIXED`:
    /// rapier's own default (`ActiveCollisionTypes::default()`) is
    /// DYNAMIC_DYNAMIC | DYNAMIC_KINEMATIC | DYNAMIC_FIXED only - kinematic
    /// vs. fixed pairs are excluded ("platforms don't collide with walls"),
    /// which silently broke shell-vs-obstacle and would-be shell-vs-wall
    /// intersection entirely: obstacles are `fixed` bodies (`spawn_static`),
    /// so without this a shell's `kinematic_position_based` sensor never
    /// even formed a broad-phase pair with one, regardless of how deep the
    /// geometric overlap was - `intersecting` always returned `false`, so
    /// shells flew straight through every obstacle. Tanks were unaffected
    /// (they're `dynamic`, and DYNAMIC_KINEMATIC *is* a default). Verified
    /// live (web/wasm build, browser-driven): before this flag, a shell's
    /// distance-to-obstacle log showed `intersecting=false` all the way
    /// through a dead-center pass (down to ~3.6px from the obstacle's own
    /// center, well inside its collision half-extent); after adding the
    /// flag, the same shot correctly reports `intersecting=true` and
    /// detonates.
    ///
    /// `shooter_group` (see `owner_group`) is excluded from this shell's
    /// filter, so it can never register an intersection against the hit
    /// sensor of the tank that fired it - it still intersects every other
    /// tank's hit sensor normally.
    pub fn spawn_shell(
        &mut self,
        position: Position,
        half_extent: f32,
        shooter_group: Group,
    ) -> RigidBodyHandle {
        let (handle, _) = self.world.insert(
            RigidBodyBuilder::kinematic_position_based()
                .translation(to_vector(position))
                .ccd_enabled(true),
            ColliderBuilder::cuboid(half_extent, half_extent)
                .sensor(true)
                .collision_groups(InteractionGroups::new(
                    Group::ALL,
                    !shooter_group,
                    InteractionTestMode::And,
                ))
                .active_collision_types(
                    ActiveCollisionTypes::default() | ActiveCollisionTypes::KINEMATIC_FIXED,
                ),
        );
        handle
    }

    /// Move a kinematic body (a shell) to `position` ahead of the next step -
    /// see `spawn_shell`. Unlike `set_velocity`, this is a direct position
    /// write: the shell's own `velocity * dt` integration (see
    /// `Shell::update`) is still what actually decides where it goes.
    pub fn set_kinematic_position(&mut self, handle: RigidBodyHandle, position: Position) {
        let body = self
            .world
            .bodies
            .get_mut(handle)
            .expect("shell physics body handle should always be valid");
        body.set_next_kinematic_translation(to_vector(position));
    }

    /// Teleport a dynamic body (a tank) straight to `position`, bypassing
    /// normal velocity-driven movement entirely - used once, at round init,
    /// to relocate a tank whose rolled spawn point turned out to be
    /// pathfinding-boxed-in (see `battlefield::relocate_boxed_in_tanks`).
    /// Zeroes velocity too, so it doesn't arrive already carrying whatever
    /// momentum it had at the old spot.
    pub fn set_position(&mut self, handle: RigidBodyHandle, position: Position) {
        let body = self
            .world
            .bodies
            .get_mut(handle)
            .expect("tank physics body handle should always be valid");
        body.set_translation(to_vector(position), true);
        body.set_linvel(Vector::new(0.0, 0.0), true);
    }

    /// Remove a body (and any colliders attached to it) from the world -
    /// used once a shell finishes its lifecycle (see `Game::update`).
    pub fn remove_body(&mut self, handle: RigidBodyHandle) {
        self.world.remove_body(handle);
    }

    /// True if colliders `a` and `b` currently intersect. Used for shell-hit
    /// detection: one side is typically a shell's sensor, the other a tank's
    /// hit sensor (see `add_hit_sensor`).
    pub fn intersecting(&self, a: ColliderHandle, b: ColliderHandle) -> bool {
        self.world.intersection_pair(a, b).unwrap_or(false)
    }

    /// The first collider attached to a body - the sole collider for a
    /// single-collider body (walls, shells), or specifically the solid
    /// movement collider for a tank body (`spawn_tank` always inserts it
    /// first; any hit sensor `add_hit_sensor` adds later comes second).
    pub fn collider_of(&self, body: RigidBodyHandle) -> ColliderHandle {
        self.world
            .bodies
            .get(body)
            .expect("physics body handle should always be valid")
            .colliders()[0]
    }

    /// A body's current linear velocity.
    pub fn velocity(&self, handle: RigidBodyHandle) -> Position {
        let body = self
            .world
            .bodies
            .get(handle)
            .expect("tank physics body handle should always be valid");
        from_vector(body.linvel())
    }

    /// Apply an instantaneous impulse to a body: an immediate change in
    /// momentum, i.e. its velocity changes by `impulse / mass`. Used for ram
    /// and explosion knockback (see `Game`'s `ram`/`explosion_hit`) - the
    /// mass division means a lighter tank (see `spawn_tank`'s `mass`) gets
    /// shoved further by the same impulse, automatically, with no separate
    /// hand-rolled mass-weighting formula needed.
    pub fn apply_impulse(&mut self, handle: RigidBodyHandle, impulse: Position) {
        let body = self
            .world
            .bodies
            .get_mut(handle)
            .expect("tank physics body handle should always be valid");
        body.apply_impulse(to_vector(impulse), true);
    }

    /// Current position of a body, read back after stepping.
    pub fn position(&self, handle: RigidBodyHandle) -> Position {
        let body = self
            .world
            .bodies
            .get(handle)
            .expect("tank physics body handle should always be valid");
        from_vector(body.translation())
    }

    /// True if the bodies `a` and `b` currently have an active contact - i.e.
    /// are actually touching right now, not merely close on the broad phase.
    /// Backed by rapier's own narrow-phase contact state
    /// (`ContactPair::has_any_active_contact`) rather than a hand-rolled
    /// geometric re-check, so ram-damage triggering matches what the solver
    /// itself just resolved this step.
    pub fn touching(&self, a: RigidBodyHandle, b: RigidBodyHandle) -> bool {
        let (collider_a, collider_b) = (self.collider_of(a), self.collider_of(b));
        self.world
            .contact_pair(collider_a, collider_b)
            .is_some_and(|pair| pair.has_any_active_contact())
    }
}

impl Default for Physics {
    fn default() -> Self {
        Self::new()
    }
}

fn to_vector(p: Position) -> Vector {
    Vector::new(p.x, p.y)
}

fn from_vector(v: Vector) -> Position {
    Position::new(v.x, v.y)
}
