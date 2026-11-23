//! The game's physics simulation (see docs/physics-engine-design.md).
//!
//! Tanks are rotation-locked dynamic bodies, accelerated toward their
//! commanded velocity by a mass-aware impulse every frame rather than
//! snapped to it (see `simulation::drive_tank`); the battlefield edges,
//! obstacles and the bong are static colliders. Ram/explosion/projectile
//! knockback is a real impulse too (`apply_impulse`). Projectiles have no
//! physics body at all: they are hand-integrated and hit-tested by a swept
//! segment check in `simulation::hits`, so rapier only ever sees solid
//! bodies.
//!
//! Rapier's lengths are the world's pixels. `length_unit` stays 1, so the
//! tolerances rapier scales by it are tuned for metre-sized bodies: contacts
//! are generated 0.02 px ahead and an overlap is pushed out at 3 px/s. The
//! two that matter in a pixel world are set here instead: the speed cap
//! (`PHYSICS_MAX_SPEED`) and every hull's one-step look-ahead
//! (`HULL_LOOK_AHEAD`), which stops a hull flush against whatever it meets
//! however it got there. A larger `length_unit` would raise the cap but not
//! the look-ahead far enough - at 32 a hull knocked at 480 px/s still lands
//! 4 px inside a wall - and it raises the sleep threshold with the rest, so
//! at 100 a heavy hull fell asleep inside one (docs/physics-engine-design.md,
//! "Scale").

use rapier2d::prelude::*;

use crate::obstacle::Surface;
use crate::{PHYSICS_FIXED_DT, PHYSICS_MAX_SPEED, Position, TANK_MOVE_CORNER_RADIUS};

/// How far ahead along its motion a hull's contacts look: its travel in one
/// step at the speed cap, a cell. Rapier's soft continuous collision
/// detection makes predictive contacts against whatever lies within the
/// hull's own travel this step (never further than this), so the solver
/// stops it at a collider's face - driven, knocked, rammed, blasted or
/// pulled - rather than a step past it. Without it a hull driven at a wall
/// sank 2.5 px into it and one knocked at 480 px/s 6 px, each pushed back
/// out over seconds.
const HULL_LOOK_AHEAD: f32 = PHYSICS_MAX_SPEED * PHYSICS_FIXED_DT;

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
/// for the smallest hull in the roster. Public so the `hitboxes` debug
/// overlay (`game.rs::draw_tank_boxes`) can draw the exact rounding the
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

/// How far inside a hull's footprint the shape `Physics::hull_fits` and
/// `Physics::hull_path_clear` ask with stands, px: a hull stopped flush
/// against a face (within 0.3 px of it, docs/physics-engine-design.md
/// "Scale") is clear of it.
const PROBE_SKIN: f32 = 0.5;

/// `tank_move_shape`, `PROBE_SKIN` smaller on every side.
fn probe_shape(half_extents: (f32, f32)) -> SharedShape {
    tank_move_shape(((half_extents.0 - PROBE_SKIN).max(1.0), (half_extents.1 - PROBE_SKIN).max(1.0)))
}

impl Physics {
    pub fn new() -> Self {
        let mut world = PhysicsWorld {
            gravity: to_vector(Position::new(0.0, 0.0)),
            ..PhysicsWorld::default()
        };
        // Scaled by `length_unit`, which is 1: px/s.
        world.integration_parameters.normalized_max_linear_velocity = PHYSICS_MAX_SPEED;
        Self { world }
    }

    /// Advance the simulation by one fixed step. `IntegrationParameters::dt`
    /// defaults to 1/60s (see `PHYSICS_FIXED_DT`), the one `dt`
    /// `Game::update` is ever given, so an update is exactly one step
    /// whatever the render frame rate. The surfaces' cushions
    /// (`SurfaceHooks`) are read from the tuning as the step begins.
    pub fn step(&mut self) {
        let t = crate::tuning::tuning();
        let hooks = SurfaceHooks { decel: Surface::ALL.map(|s| s.cushion(&t)), dt: self.world.integration_parameters.dt };
        self.world.step_with_events(&hooks, &());
    }

    /// Give a fixed body's collider the surface a hull meets on it
    /// (`obstacle::Surface`): kept in the collider's `user_data`, and a
    /// face that cushions asks the solver's contact hook in
    /// (`SurfaceHooks`). Every collider starts `Plain`.
    pub fn set_surface(&mut self, body: RigidBodyHandle, surface: Surface) {
        let handle = self.collider_of(body);
        let collider = self.world.colliders.get_mut(handle).expect("collider handle should always be valid");
        collider.user_data = surface as u128;
        let cushions = matches!(surface, Surface::Firm | Surface::Yielding | Surface::Soft);
        collider.set_active_hooks(if cushions { ActiveHooks::MODIFY_SOLVER_CONTACTS } else { ActiveHooks::empty() });
    }

    /// The surface a body's collider carries (`set_surface`).
    pub fn surface_of(&self, body: RigidBodyHandle) -> Surface {
        let handle = self.collider_of(body);
        self.world.colliders.get(handle).map_or(Surface::Plain, |c| Surface::from_code(c.user_data))
    }

    /// Every fixed body a hull's body is in active contact with after the
    /// last step, in the narrow phase's own order (fixed by the order the
    /// bodies were spawned in): the other body, its surface, the normal of
    /// their deepest contact pointing from the hull into it, and that
    /// contact's point in the world.
    pub fn fixed_contacts(&self, body: RigidBodyHandle) -> Vec<FixedContact> {
        let hull = self.collider_of(body);
        let mut out = Vec::new();
        for pair in self.world.contact_pairs_with(hull) {
            if !pair.has_any_active_contact() {
                continue;
            }
            let other = if pair.collider1 == hull { pair.collider2 } else { pair.collider1 };
            let Some(collider) = self.world.colliders.get(other) else { continue };
            let Some(parent) = collider.parent() else { continue };
            if !self.world.bodies.get(parent).is_some_and(|b| b.is_fixed()) {
                continue;
            }
            let Some((manifold, point)) = pair.find_deepest_contact() else { continue };
            let n = from_vector(manifold.data.normal);
            let normal = if pair.collider1 == hull { n } else { Position::new(-n.x, -n.y) };
            let (local, owner) = if pair.collider1 == hull { (point.local_p1, pair.collider1) } else { (point.local_p2, pair.collider2) };
            let at = self.world.colliders.get(owner).map_or(Position::new(0.0, 0.0), |c| {
                let p = c.position().transform_point(local);
                Position::new(p.x, p.y)
            });
            out.push(FixedContact { other: parent, surface: Surface::from_code(collider.user_data), normal, point: at });
        }
        out
    }

    /// How many bodies and colliders rapier quarantined during the last
    /// `step` (see rapier's `Quarantine`): anything whose pose, velocity or
    /// geometry went non-finite is rolled back to its last valid pose and
    /// disabled, rather than spreading NaN through the rest of the world.
    /// Non-zero means the solver blew up - it is not a state any healthy
    /// round reaches, so `simulation::step_world` turns it straight into an
    /// `Event::PhysicsQuarantine` for the probe's `invariant` anomaly and
    /// the dev server to surface. Counts rather than handles: the caller
    /// already knows which tank is broken from its own snapshot, and this
    /// also covers the wall/obstacle/bong bodies nothing else inspects.
    pub fn quarantined(&self) -> (usize, usize) {
        let q = self.world.quarantine();
        (q.bodies().len(), q.colliders().len())
    }

    /// Spawn a static, fixed-body cuboid collider: the battlefield boundary
    /// (see `battlefield::spawn_walls`) and in-arena obstacles (see
    /// `obstacle::Obstacle`) both reuse this exact same shape - the only
    /// difference is whether the caller ever calls `remove_body` on the
    /// handle later (walls never do; a destroyed obstacle does).
    /// `center` and `half_extents` are in the same pixel space as
    /// `Tank`/`Shell` positions; rapier does no unit conversion here (1
    /// physics unit == 1px). It starts `Plain`; a tile or the field's edge
    /// is given its surface with `set_surface`.
    ///
    /// The bong uses this too, and is then teleported with `set_position`
    /// every frame. That looks wrong - a moving body "should" be
    /// `KinematicPositionBased` so the solver pushes contacts aside rather
    /// than letting depenetration fling them out - and it was **tried and
    /// measured worse**: `maps/test/bong-block.toml`, the fixture built for
    /// exactly this failure class, went from 0 to 10 `clustering` and 0 to
    /// 2 `low-progress` anomalies, breaching its pinned budgets, while the
    /// default-map sweep did not move at all.
    ///
    /// The reason is that a bong *hops*: its position jumps discretely
    /// rather than travelling. A kinematic body asked to be somewhere far
    /// away by the next step drags whatever is in between along with it, so
    /// a hopping bong started shoving tanks across the map. For a discretely
    /// teleporting entity, "fixed body, relocated" is the more faithful
    /// model, not the lazier one. Re-measure with `just probe-fixtures`
    /// before revisiting.
    ///
    /// Every static keeps rapier's default friction (0.5) and restitution
    /// (0.0). How hard a face is (`obstacle::Surface`) acts on the
    /// contact's normal only: a cushion through the contact hook
    /// (`SurfaceHooks`), a bounce as a knock after the step
    /// (`simulation::contact`).
    ///
    /// Per-material surfaces were tried and **measured worse**, twice, so
    /// this deliberately takes no friction parameter. Grippier materials
    /// (sandbags 0.9, brick 0.6) more than doubled the probe's `wall-grind`
    /// count and multiplied `spin` by seven over 30 seeded rounds; a
    /// second attempt running only in the slick direction (glass 0.25,
    /// iron 0.3) came out inside the noise on one seed and slightly worse
    /// on another. The reason is structural rather than a bad choice of
    /// numbers: `TANK_MOVE_CORNER_RADIUS` exists so a tank *slides* past
    /// wall-tile seams instead of snagging on them, and surface friction is
    /// the force that fights exactly that. How walls feel differs instead
    /// along the normal alone - the cushion and the bounce above - and by
    /// contact FX and ram behaviour, never by collider friction. Re-measure
    /// with `just probe-sweep --seed <n>` before trying again.
    pub fn spawn_static(&mut self, center: Position, half_extents: Position) -> RigidBodyHandle {
        let (handle, _) = self.world.insert(
            RigidBodyBuilder::fixed().translation(to_vector(center)),
            ColliderBuilder::cuboid(half_extents.x, half_extents.y),
        );
        handle
    }

    /// Refit a static body's collider (`spawn_static`'s) to the box of
    /// `half_extents` whose centre sits `offset` from the body: a chunked
    /// wall tile (`chunks.rs`) whose standing quadrants no longer fill it.
    pub fn fit_static(&mut self, body: RigidBodyHandle, offset: Position, half_extents: Position) {
        let handle = self.collider_of(body);
        let collider = self.world.colliders.get_mut(handle).expect("collider handle should always be valid");
        collider.set_shape(SharedShape::cuboid(half_extents.x, half_extents.y));
        collider.set_translation_wrt_parent(to_vector(offset));
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
    /// `simulation::drive_tank`. The body looks `HULL_LOOK_AHEAD` ahead for
    /// as long as it lives, wreck or not.
    pub fn spawn_tank(
        &mut self,
        position: Position,
        half_extents: (f32, f32),
        mass: f32,
    ) -> RigidBodyHandle {
        let (handle, _) = self.world.insert(
            RigidBodyBuilder::dynamic()
                .translation(to_vector(position))
                .lock_rotations()
                .soft_ccd_prediction(HULL_LOOK_AHEAD),
            ColliderBuilder::new(tank_move_shape(half_extents)).mass(mass),
        );
        handle
    }

    /// Spawn a loose object's dynamic body (docs/sandbags-barrels-fences.md
    /// "Loose barrels"): a drum standing upright, a ball of `radius` px
    /// with rotation locked - it slides, it does not spin - of `mass`
    /// against the standard chassis's 1, losing `damping` of its speed a
    /// second on the ground and keeping `bounce` of it off whatever it
    /// hits (restitution, the larger of the two sides', so a drum bounces
    /// off a wall and a hull though neither bounces off anything). It
    /// looks a step ahead as a hull does (`HULL_LOOK_AHEAD`), so a thrown
    /// drum stops at a wall's face rather than in it. Its collider is
    /// marked loose (`is_loose`): a hull pressing on one is touching
    /// neither terrain nor a tank (`contact_stats`).
    pub fn spawn_loose(&mut self, center: Position, radius: f32, mass: f32, damping: f32, bounce: f32) -> RigidBodyHandle {
        let (handle, _) = self.world.insert(
            RigidBodyBuilder::dynamic()
                .translation(to_vector(center))
                .lock_rotations()
                .linear_damping(damping)
                .soft_ccd_prediction(HULL_LOOK_AHEAD),
            ColliderBuilder::ball(radius)
                .mass(mass)
                .restitution(bounce)
                .restitution_combine_rule(CoefficientCombineRule::Max)
                .friction(LOOSE_FRICTION)
                .user_data(LOOSE_TAG),
        );
        handle
    }

    /// Whether `body` is a loose object's (`spawn_loose`).
    pub fn is_loose(&self, body: RigidBodyHandle) -> bool {
        self.world.bodies.get(body).and_then(|b| b.colliders().first()).and_then(|&c| self.world.colliders.get(c)).is_some_and(|c| c.user_data == LOOSE_TAG)
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

    /// Whether a hull of overall `half_extents` centred on `at` stands
    /// clear of every fixed body: a tile, the field's edge, deep water,
    /// the bong. Other hulls are not asked - a turn is fitted to the
    /// ground, and two hulls settle between themselves in the solver.
    ///
    /// The shape tested stands `PROBE_SKIN` inside the footprint, so a
    /// hull at rest against a face reads as clear of it. It asks the
    /// broad phase as the last `step` left it: a collider spawned or
    /// moved since is seen from the next step on.
    pub fn hull_fits(&self, at: Position, half_extents: (f32, f32)) -> bool {
        let shape = probe_shape(half_extents);
        self.world
            .query_pipeline_with_filter(QueryFilter::only_fixed())
            .intersect_shape(Pose::translation(at.x, at.y), shape.as_ref())
            .next()
            .is_none()
    }

    /// Whether that hull could travel `distance` from `at` along the unit
    /// vector `dir` without meeting a fixed body: `hull_fits` at `at` and
    /// all the way there.
    pub fn hull_path_clear(&self, at: Position, half_extents: (f32, f32), dir: Position, distance: f32) -> bool {
        let shape = probe_shape(half_extents);
        let options = rapier2d::parry::query::ShapeCastOptions {
            max_time_of_impact: distance,
            target_distance: 0.0,
            stop_at_penetration: true,
            compute_impact_geometry_on_penetration: false,
        };
        self.world
            .cast_shape(&Pose::translation(at.x, at.y), to_vector(dir), shape.as_ref(), options, QueryFilter::only_fixed())
            .is_none()
    }

    /// Move a body by `by`, keeping its velocity: the nudge that fits a
    /// turning hull beside a wall (`simulation::drive_tank`).
    pub fn shift(&mut self, handle: RigidBodyHandle, by: Position) {
        let body = self
            .world
            .bodies
            .get_mut(handle)
            .expect("tank physics body handle should always be valid");
        let at = body.translation();
        body.set_translation(Vector::new(at.x + by.x, at.y + by.y), true);
    }

    /// Teleport a dynamic body (a tank) straight to `position`, bypassing
    /// normal velocity-driven movement entirely - used once, at round init,
    /// to relocate a tank whose rolled spawn point turned out to be
    /// pathfinding-boxed-in (see `battlefield::relocate_unusable_spawns`).
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

    /// Overwrite a dynamic body's linear velocity. The client replica's
    /// one use (`net::apply`): the snapshot's velocity goes on the body
    /// after `set_position`, so `velocity` reads back what the server sent.
    pub fn set_velocity(&mut self, handle: RigidBodyHandle, velocity: Position) {
        let body = self
            .world
            .bodies
            .get_mut(handle)
            .expect("tank physics body handle should always be valid");
        body.set_linvel(to_vector(velocity), true);
    }

    /// The fastest any body moves (px/s): rapier's speed cap
    /// (`IntegrationParameters::max_linear_velocity`, `PHYSICS_MAX_SPEED`).
    pub fn max_speed(&self) -> f32 {
        self.world.integration_parameters.max_linear_velocity()
    }

    /// Settle a tank that has just become a wreck: heavy linear and
    /// angular damping, and more surface friction.
    ///
    /// A wreck keeps its full-mass dynamic body but stops being driven, and
    /// nothing in the world sets damping (rapier's default is zero), so a
    /// shoved hulk used to slide until something else stopped it - braked
    /// only by contact friction, never coming to rest by design. Damping is
    /// what makes a burnt-out tank behave like several tonnes of dead metal
    /// instead of an air-hockey puck.
    ///
    /// Applied on the one frame a tank becomes a wreck (see
    /// `simulation::roll_wreck_col`), because a wreck is created by damage
    /// mid-round rather than spawned as one, so `RigidBodyBuilder`'s
    /// damping is not available to us.
    pub fn settle_wreck(&mut self, handle: RigidBodyHandle, damping: f32, friction: f32) {
        let collider = self.collider_of(handle);
        {
            let body = self
                .world
                .bodies
                .get_mut(handle)
                .expect("tank physics body handle should always be valid");
            body.set_linear_damping(damping);
            body.set_angular_damping(damping);
        }
        let collider = self
            .world
            .colliders
            .get_mut(collider)
            .expect("collider handle should always be valid");
        collider.set_friction(friction);
    }

    /// Remove a body (and any colliders attached to it) from the world -
    /// used once an obstacle is destroyed (see `Game::update`).
    pub fn remove_body(&mut self, handle: RigidBodyHandle) {
        self.world.remove_body(handle);
    }

    /// The sole collider attached to a body (every body here carries
    /// exactly one: a tank's solid movement collider, a wall/obstacle/bong
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

    /// The direction body `a` presses on body `b` along - the unit normal
    /// of their deepest active contact, pointing from `a` into `b` - or
    /// `None` while they do not touch (`touching`).
    pub fn contact_normal(&self, a: RigidBodyHandle, b: RigidBodyHandle) -> Option<Position> {
        let (collider_a, collider_b) = (self.collider_of(a), self.collider_of(b));
        let pair = self.world.contact_pair(collider_a, collider_b)?;
        let (manifold, _) = pair.find_deepest_contact()?;
        if !pair.has_any_active_contact() {
            return None;
        }
        let n = from_vector(manifold.data.normal);
        Some(if pair.collider1 == collider_a { n } else { Position::new(-n.x, -n.y) })
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
    /// obstacles, the bong (all `fixed` bodies -> `touching_static` - the
    /// bong counting as terrain is deliberate, it's the historical
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
            // A loose drum is neither terrain nor a tank: a hull shoving
            // one is pressing on nothing the probe's contact kinds or the
            // parked hold read.
            if self.world.colliders.get(other).is_some_and(|c| c.user_data == LOOSE_TAG) {
                stats.touching_loose = true;
                continue;
            }
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
            let other_body = self.world.colliders.get(other).and_then(|c| c.parent());
            for manifold in pair.manifolds() {
                for point in &manifold.points {
                    if point.data.impulse <= stats.max_impulse {
                        continue;
                    }
                    stats.max_impulse = point.data.impulse;
                    stats.hardest = other_body.map(|b| (b, other_is_fixed));
                    // Where the hardest contact actually is, in world
                    // space. Without it a consumer can only guess - and
                    // putting a shower of sparks at a tank's centre rather
                    // than at the corner that struck the wall is exactly
                    // the sort of thing that reads as wrong without anyone
                    // being able to say why.
                    let local = if pair.collider1 == hull { point.local_p1 } else { point.local_p2 };
                    let owner = if pair.collider1 == hull { pair.collider1 } else { pair.collider2 };
                    stats.contact = self
                        .world
                        .colliders
                        .get(owner)
                        .map(|c| { let p = c.position().transform_point(local); Position::new(p.x, p.y) });
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
    /// or the bong.
    pub touching_static: bool,
    /// Actively contacting another tank's hull (`dynamic` body).
    pub touching_tank: bool,
    /// Actively contacting a loose object (`Physics::spawn_loose`), which
    /// counts as neither of the two above and puts nothing into
    /// `max_impulse`.
    pub touching_loose: bool,
    /// Strongest per-contact-point normal impulse the solver applied this
    /// step across those contacts; 0.0 when nothing is actively touching.
    pub max_impulse: f32,
    /// World-space position of that strongest contact, when there is one.
    pub contact: Option<Position>,
    /// The body that strongest contact is with, and whether it is fixed.
    pub hardest: Option<(RigidBodyHandle, bool)>,
}

/// One fixed body a hull touches (`Physics::fixed_contacts`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedContact {
    pub other: RigidBodyHandle,
    pub surface: Surface,
    /// Unit normal from the hull into the body.
    pub normal: Position,
    /// Where they touch.
    pub point: Position,
}

/// What a loose object's collider carries in its `user_data`
/// (`Physics::spawn_loose`): past every `Surface` code.
const LOOSE_TAG: u128 = 1 << 16;

/// A loose object's surface friction: a drum slides along a wall or a hull
/// rather than catching on it.
const LOOSE_FRICTION: f32 = 0.2;

/// How square to a face's axis a contact's normal has to be for a cushion
/// to act on it: a hull meets a face, not a corner it is driving past.
const CUSHION_AXIS_COS: f32 = 0.95;

/// How much of a hull's velocity has to go into a face for a cushion to
/// act on it: a hull coming at it, not one sliding by.
const CUSHION_HEAD_ON: f32 = 0.7;

/// The solver's contact hook: a cushioning face's give
/// (docs/physics-engine-design.md "Hard and soft").
///
/// Every contact a hull has with a fixed collider within its look-ahead
/// is a speculative one while they are apart: the solver lets the hull
/// close the gap `dist` this step and no more. For a face that cushions
/// (`Surface::cushion` > 0), coming at it head on (`CUSHION_HEAD_ON`,
/// `CUSHION_AXIS_COS`), the hook shortens that to what `v^2 = 2 a d` allows
/// at `d = dist` - so the closer the hull is the slower it may come, and it
/// settles flush with the face over the last few px instead of stopping on
/// it from full speed. Only the gap along the normal is touched: friction,
/// and a hull sliding along the face, are the solver's as for any wall.
/// A pure function of the contact and the hull's velocity: no state, no
/// RNG.
struct SurfaceHooks {
    /// `Surface::cushion` of each surface, by its code.
    decel: [f32; 8],
    dt: f32,
}

impl PhysicsHooks for SurfaceHooks {
    fn modify_solver_contacts(&self, context: &mut ContactModificationContext) {
        let code = |c: ColliderHandle| context.colliders.get(c).map_or(0, |c| c.user_data as usize);
        let (c1, c2) = (code(context.collider1), code(context.collider2));
        let (tile_first, a) = match (self.decel.get(c1).copied().unwrap_or(0.0), self.decel.get(c2).copied().unwrap_or(0.0)) {
            (a, _) if a > 0.0 => (true, a),
            (_, a) if a > 0.0 => (false, a),
            _ => return,
        };
        let hull = if tile_first { context.rigid_body2 } else { context.rigid_body1 };
        let Some(body) = hull.and_then(|h| context.bodies.get(h)) else { return };
        if !body.is_dynamic() {
            return;
        }
        let v = body.linvel();
        let dt = self.dt;
        let Some(rigid) = context.rigid_mut() else { return };
        let n = *rigid.normal;
        // From the hull into the face.
        let into = if tile_first { Vector::new(-n.x, -n.y) } else { n };
        if into.x.abs().max(into.y.abs()) < CUSHION_AXIS_COS {
            return;
        }
        let speed = v.x * into.x + v.y * into.y;
        let total = (v.x * v.x + v.y * v.y).sqrt();
        if speed <= 0.0 || speed < CUSHION_HEAD_ON * total {
            return;
        }
        for contact in rigid.solver_contacts.iter_mut() {
            if contact.dist <= 0.0 {
                continue;
            }
            let allowed = (2.0 * a * contact.dist).sqrt() * dt;
            if allowed < contact.dist {
                contact.dist = allowed;
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A knock at the wire's ceiling (508 px/s) keeps its whole speed:
    /// rapier's own cap, 400 in its units, would have clipped it.
    #[test]
    fn a_knock_past_rapiers_default_cap_keeps_its_speed() {
        let mut physics = Physics::new();
        let hull = physics.spawn_tank(Position::new(0.0, 0.0), (12.0, 12.0), 2.0);
        physics.apply_impulse(hull, Position::new(508.0 * 2.0, 0.0));
        physics.step();
        assert_eq!(physics.velocity(hull).x, 508.0);
        assert!(physics.max_speed() > 508.0);
    }

    /// A hull thrown at a wall a step's travel or less from it at any
    /// speed up to the cap stops at the wall's face, not inside it.
    #[test]
    fn a_hull_thrown_at_a_wall_stops_at_its_face() {
        for speed in [60.0, 210.0, 480.0, 1000.0, PHYSICS_MAX_SPEED] {
            for gap in [0.5, 2.0, 5.0, speed * PHYSICS_FIXED_DT * 0.5] {
                let mut physics = Physics::new();
                // The wall's west face at x 100.
                physics.spawn_static(Position::new(116.0, 0.0), Position::new(16.0, 64.0));
                let hull = physics.spawn_tank(Position::new(100.0 - 12.0 - gap, 0.0), (12.0, 12.0), 1.0);
                physics.apply_impulse(hull, Position::new(speed, 0.0));
                for _ in 0..10 {
                    physics.step();
                    let east = physics.position(hull).x + 12.0;
                    assert!(east <= 100.05, "at {speed} px/s from {gap} px: {} px inside", east - 100.0);
                }
            }
        }
    }
}
