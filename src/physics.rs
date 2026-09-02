//! The game's physics simulation (see docs/physics-engine-design.md).
//!
//! Tanks are rotation-locked dynamic bodies, accelerated toward their
//! commanded velocity by a mass-aware impulse every frame rather than
//! snapped to it (see `simulation::drive_tank`); the battlefield edges,
//! obstacles and the frog are static colliders. Ram/explosion/projectile
//! knockback is a real impulse too (`apply_impulse`). Projectiles have no
//! physics body at all: they are hand-integrated and hit-tested by a swept
//! segment check in `simulation::hits`, so rapier only ever sees solid
//! bodies.

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
    /// handle later (walls never do; a destroyed obstacle does).
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
    /// used once an obstacle is destroyed (see `Game::update`).
    pub fn remove_body(&mut self, handle: RigidBodyHandle) {
        self.world.remove_body(handle);
    }

    /// The sole collider attached to a body (every body here carries
    /// exactly one: a tank's solid movement collider, a wall/obstacle/frog
    /// cuboid).
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
    /// and explosion knockback (see `simulation::combat`) - the
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

    /// What a tank body's solid hull is pressing against *right now*, read
    /// straight out of rapier's narrow phase as of the last step - the
    /// ground truth behind the probe's contact anomaly kinds (wall-grind/
    /// bump-rate, see docs/gameplay-verification-design.md §4), which the
    /// kinematic checks could only ever infer from symptoms. Same
    /// `has_any_active_contact` basis as `touching` above, but fanned out
    /// over every pair the hull is in (`contact_pairs_with`) instead of
    /// one known opponent, and classified by what's on the other side.
    ///
    /// Every body here is solid (projectiles have no physics body at all),
    /// so the pairs iterated are exactly the hull against walls,
    /// obstacles, the frog (all `fixed` bodies -> `touching_static` - the
    /// frog counting as terrain is deliberate, it's the historical
    /// stuck-against case) and other tanks' hulls (`dynamic` ->
    /// `touching_tank`). `max_impulse` is the strongest per-point normal
    /// impulse rapier's solver applied among those active contacts this
    /// step - 0.0 while merely resting in broad-phase proximity,
    /// mass-scaled (see `spawn_tank`), so a ram spike dwarfs a scrape.
    pub fn contact_stats(&self, body: RigidBodyHandle) -> ContactStats {
        let hull = self.collider_of(body);
        let mut stats = ContactStats::default();
        for pair in self.world.contact_pairs_with(hull) {
            if !pair.has_any_active_contact() {
                continue;
            }
            let other = if pair.collider1 == hull { pair.collider2 } else { pair.collider1 };
            // A live pair's colliders/bodies always resolve; the chained
            // lookups are just totality, not an expected failure path.
            let other_is_fixed = self
                .world
                .colliders
                .get(other)
                .and_then(|c| c.parent())
                .and_then(|b| self.world.bodies.get(b))
                .is_some_and(|b| b.is_fixed());
            if other_is_fixed {
                stats.touching_static = true;
            } else {
                stats.touching_tank = true;
            }
            for manifold in &pair.manifolds {
                for point in &manifold.points {
                    stats.max_impulse = stats.max_impulse.max(point.data.impulse);
                }
            }
        }
        stats
    }
}

/// Result of `Physics::contact_stats`: what a tank's solid hull currently
/// has active solver contacts with, plus the strongest of their impulses.
/// Plain instantaneous facts - any windowing/accumulation lives in the
/// consumer (the probe), per the design doc's §4.2.
#[derive(Default, Clone, Copy)]
pub struct ContactStats {
    /// Actively contacting any `fixed` body: boundary wall, obstacle tile,
    /// or the frog.
    pub touching_static: bool,
    /// Actively contacting another tank's hull (`dynamic` body).
    pub touching_tank: bool,
    /// Strongest per-contact-point normal impulse the solver applied this
    /// step across those contacts; 0.0 when nothing is actively touching.
    pub max_impulse: f32,
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
