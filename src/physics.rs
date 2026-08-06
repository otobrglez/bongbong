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

use crate::Position;

/// Owns the rapier simulation state. Wraps rapier's own `PhysicsWorld`
/// convenience bundle (rigid bodies, colliders, broad/narrow-phase, solver,
/// `IntegrationParameters`, ...) rather than re-declaring those fields by
/// hand, with gravity zeroed out - bongbong is top-down, so there's no axis
/// for anything to fall along.
pub struct Physics {
    world: PhysicsWorld,
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

    /// Spawn a static wall collider - a fixed, immovable battlefield boundary
    /// segment. `center` and `half_extents` are in the same pixel space as
    /// `Tank`/`Shell` positions; rapier does no unit conversion here (1
    /// physics unit == 1px).
    pub fn spawn_wall(&mut self, center: Position, half_extents: Position) {
        self.world.insert(
            RigidBodyBuilder::fixed().translation(to_vector(center)),
            ColliderBuilder::cuboid(half_extents.x, half_extents.y),
        );
    }

    /// Spawn a rotation-locked dynamic tank body: a square collider
    /// `half_extent` per side, centered at `position`, with mass set to
    /// `mass` (matching `Tank::mass()`) so impulses applied later (see
    /// `apply_impulse`) produce the exact velocity change the caller
    /// intended rather than whatever rapier's default density would give.
    /// Rotation is locked because driving stays cardinal (see
    /// docs/physics-engine-design.md) - the body only ever needs to
    /// translate, never spin; sprite facing stays the existing cosmetic
    /// `Tank::rotation`/`Dir::rotation`.
    pub fn spawn_tank(
        &mut self,
        position: Position,
        half_extent: f32,
        mass: f32,
    ) -> RigidBodyHandle {
        let (handle, _) = self.world.insert(
            RigidBodyBuilder::dynamic()
                .translation(to_vector(position))
                .lock_rotations(),
            ColliderBuilder::cuboid(half_extent, half_extent).mass(mass),
        );
        handle
    }

    /// Attach an extra sensor collider to an existing tank body, used only to
    /// detect shell hits - a separate, larger hit box (typically the tank's
    /// full sprite size, see `Tank::size` vs the solid hull collider's
    /// `Tank::hull_size`) than what blocks movement, so switching shell hit
    /// detection to physics doesn't shrink what counts as "in range" of a
    /// shell. Returns the new collider's handle - callers need it directly
    /// for `intersecting`, since a tank body now has two colliders and
    /// `collider_of`'s "the first one" convention only ever refers to the
    /// original solid hull collider `spawn_tank` attaches.
    ///
    /// Explicitly zero-mass: rapier still folds a collider's density-based
    /// mass into its body's total mass even when `.sensor(true)` - being a
    /// sensor only exempts it from collision *response*, not mass-property
    /// aggregation. Left at the default density (1.0), this sensor's full
    /// tank-sized area would silently outweigh the hull's own explicit mass
    /// (`Tank::mass`, ~4) by three orders of magnitude, since it's the
    /// bigger of the two colliders - which is exactly what was making tanks
    /// crawl: `Game::drive_tank`'s impulse is sized for `Tank::mass()`, but
    /// rapier was dividing by the real (much larger) body mass instead.
    pub fn add_hit_sensor(&mut self, body: RigidBodyHandle, half_extent: f32) -> ColliderHandle {
        self.world.insert_collider(
            ColliderBuilder::cuboid(half_extent, half_extent)
                .sensor(true)
                .mass(0.0),
            Some(body),
        )
    }

    /// Spawn a kinematic-position-based sensor body for a shell: a square
    /// sensor `half_extent` per side. A sensor never gets physically pushed
    /// and never pushes anything else - it only ever reports whether it's
    /// intersecting something (see `intersecting`). CCD is enabled so a
    /// fast-moving shell can't tunnel past a tank within a single step
    /// without the intersection registering.
    pub fn spawn_shell(&mut self, position: Position, half_extent: f32) -> RigidBodyHandle {
        let (handle, _) = self.world.insert(
            RigidBodyBuilder::kinematic_position_based()
                .translation(to_vector(position))
                .ccd_enabled(true),
            ColliderBuilder::cuboid(half_extent, half_extent).sensor(true),
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
    /// single-collider body (walls, shells), or specifically the solid hull
    /// collider for a tank body (`spawn_tank` always inserts it first; any
    /// hit sensor `add_hit_sensor` adds later comes second).
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
