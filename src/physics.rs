//! The game's physics simulation (see docs/physics-engine-design.md).
//!
//! Phase 1: an empty, stepped world with no bodies wired in yet. This exists
//! to prove rapier2d builds and integrates into the game loop before any
//! gameplay code depends on it - walls, tank bodies, and impulses land in
//! later phases.

use rapier2d::prelude::*;

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
                gravity: vector![0.0, 0.0].into(),
                ..PhysicsWorld::default()
            },
        }
    }

    /// Advance the simulation by one step. `IntegrationParameters::dt`
    /// defaults to 1/60s, matching `set_target_fps(60)` in `main.rs`; a real
    /// fixed-timestep accumulator so this stays correct under variable frame
    /// rates lands in phase 2, alongside the first bodies.
    pub fn step(&mut self) {
        self.world.step();
    }
}

impl Default for Physics {
    fn default() -> Self {
        Self::new()
    }
}
