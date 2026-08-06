//! The game's physics simulation (see docs/physics-engine-design.md).
//!
//! Tanks are rotation-locked dynamic bodies driven by a commanded velocity
//! written every frame (see `Game`'s `drive_tank`); the battlefield edges are
//! static wall colliders. Real impulse-based knockback and shell colliders
//! land in later phases.

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
    /// `half_extent` per side, centered at `position`. Rotation is locked
    /// because driving stays cardinal (see docs/physics-engine-design.md) -
    /// the body only ever needs to translate, never spin; sprite facing
    /// stays the existing cosmetic `Tank::rotation`/`Dir::rotation`.
    pub fn spawn_tank(&mut self, position: Position, half_extent: f32) -> RigidBodyHandle {
        let (handle, _) = self.world.insert(
            RigidBodyBuilder::dynamic()
                .translation(to_vector(position))
                .lock_rotations(),
            ColliderBuilder::cuboid(half_extent, half_extent),
        );
        handle
    }

    /// Set a body's linear velocity directly (not an impulse). Used for the
    /// commanded cardinal movement plus any residual knockback drift, which
    /// together are still computed by hand each frame (see `Game::drive_tank`)
    /// rather than as a real physics impulse - that lands in a later phase.
    pub fn set_velocity(&mut self, handle: RigidBodyHandle, velocity: Position) {
        let body = self
            .world
            .bodies
            .get_mut(handle)
            .expect("tank physics body handle should always be valid");
        body.set_linvel(to_vector(velocity), true);
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
