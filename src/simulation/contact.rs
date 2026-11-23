//! What a hull does as it meets a fixed face, by how hard the face is
//! (`obstacle::Surface`, docs/physics-engine-design.md "Hard and soft").
//!
//! The cushion of a soft face is the solver's (`physics::SurfaceHooks`):
//! it acts inside the step. What is left is judged here, after the step,
//! from each hull's velocity going into it (`Tank::ram_velocity`) and the
//! fixed bodies it ended the step touching (`Physics::fixed_contacts`):
//!
//! - a hard face hit at `hull_bounce_min_speed` or more throws the hull
//!   back - a knock along the face's normal (`combat::knock_hull`), with a
//!   short skid so the throw is seen. A client-owned seat is not told: its
//!   client throws its own hull on the same hit in its sandbox step
//!   (`throw_off_faces`), and the room only allows the pose the speed;
//! - a pane of glass hit at `glass_crack_speed` or more cracks
//!   (`glass_ram_damage`) instead;
//! - a hit at `hull_struck_min_speed` or more is reported
//!   (`Event::HullStruck`), and a tree hit that way shakes its crown
//!   (`TreeShake`).
//!
//! Only the contact's normal is ever touched: a hull sliding along a face
//! is not going into it and nothing here sees it. No RNG; hulls are
//! walked by owner slot.
//!
//! A client-owned hull (docs/online-coop-prd.md §4.14) meets the room's
//! faces at the speed its client's hull met them: the pose carries it
//! (`SeatPose::ram`) and the room steps its copy over the tick at it
//! (`Game::accept_seat_pose`).

use hecs::Entity;
use rapier2d::prelude::RigidBodyHandle;

use crate::knock::Knock;
use crate::map::world_to_cell;
use crate::math::Vec2;
use crate::obstacle::{Material, Obstacle, Surface};
use crate::physics::Physics;
use crate::tank::Tank;
use crate::tuning::{tuning, Tuning};
use crate::Position;

use super::combat::knock_hull;
use super::props::DamageCause;
use super::{ordered, Event, Footing, Frame, Game};

/// A tree's crown swinging after a hull hit its trunk: presentation, kept
/// on the round so the painters read it (`obstacle::draw_tree`'s lean) and
/// set on a replica from the room's `Event::HullStruck`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreeShake {
    /// The cell of the tree.
    pub cell: (i32, i32),
    /// Which way the crown swings first, -1 or 1 along x.
    pub side: f32,
    /// How hard, 1 at `tank_speed`.
    pub strength: f32,
    /// The round clock it was hit at.
    pub at: f32,
}

impl TreeShake {
    /// The sideways lean it adds `now`, px: `tree_shake_px` times its
    /// strength, swinging about three times and dying away over
    /// `tree_shake_seconds`; 0 once over.
    pub fn lean(&self, now: f32, t: &Tuning) -> f32 {
        let age = now - self.at;
        let life = t.tree_shake_seconds.max(0.05);
        if age < 0.0 || age >= life {
            return 0.0;
        }
        let p = age / life;
        let swing = crate::math::sin(p * std::f32::consts::TAU * 3.0 + std::f32::consts::FRAC_PI_2);
        self.side * t.tree_shake_px * self.strength * (1.0 - p) * (1.0 - p) * swing
    }
}

/// The hit `Game::respond_to_contacts` judges one hull's step by.
struct Struck {
    body: RigidBodyHandle,
    surface: Surface,
    normal: Position,
    point: Position,
    speed: f32,
}

impl Struck {
    /// A pane hit hard enough to crack (`glass_crack_speed`).
    fn cracks(&self, t: &Tuning) -> bool {
        self.surface == Surface::Brittle && self.speed >= t.glass_crack_speed
    }

    /// The throw this hit gives a hull that is not resting from its last
    /// one: a knock back along the face's normal at the face's bounce
    /// (`Surface::bounce`) of the hit's speed, with a short skid. `None`
    /// where it cracks a pane instead, the face throws nothing or the hit
    /// was slower than `hull_bounce_min_speed`.
    fn throw(&self, t: &Tuning) -> Option<(Vec2, Knock)> {
        let bounce = self.surface.bounce(t);
        if self.cracks(t) || bounce <= 0.0 || self.speed < t.hull_bounce_min_speed {
            return None;
        }
        let k = Knock { speed: bounce * self.speed, mass_exponent: 0.0, max_speed: t.knockback_max_speed, skid: t.hull_bounce_skid_seconds };
        Some((Vec2::new(-self.normal.x, -self.normal.y), k))
    }
}

/// The slowest hit anything here answers.
fn lowest(t: &Tuning) -> f32 {
    t.hull_struck_min_speed.min(t.hull_bounce_min_speed).min(t.glass_crack_speed).max(1.0)
}

/// The hardest of a hull's hits in the step it went into at `velocity`:
/// of the fixed bodies `body` ended the step touching, the face it went
/// into fastest along the contact's normal, at `low` or more. A `Plain`
/// body (deep water, lava, the bong) is the solver's.
fn hardest_hit(physics: &Physics, body: RigidBodyHandle, velocity: Vec2, low: f32) -> Option<Struck> {
    let mut best: Option<Struck> = None;
    for c in physics.fixed_contacts(body) {
        let speed = velocity.x * c.normal.x + velocity.y * c.normal.y;
        if c.surface == Surface::Plain || speed < low || best.as_ref().is_some_and(|b| b.speed >= speed) {
            continue;
        }
        best = Some(Struck { body: c.other, surface: c.surface, normal: c.normal, point: c.point, speed });
    }
    best
}

/// A client's own hull after its sandbox step (`Game::predict_seat_with`):
/// thrown back off a hard face exactly as `respond_to_contacts` throws it
/// in the room - the same hardest hit, the same knock, the same rest - and
/// nothing else: the slam, a cracked pane and a tree's shake are the
/// room's to report. The room's throw of a client-owned hull goes
/// untold, so the hull is thrown once, on the tick it hit.
pub(super) fn throw_off_faces(physics: &mut Physics, tank: &mut Tank, footing: Footing) {
    let t = tuning();
    let low = lowest(&t);
    let Some(body) = tank.body else { return };
    if tank.is_wreck() || tank.ram_velocity.length() < low || tank.bounce_rest > 0.0 {
        return;
    }
    let Some((dir, k)) = hardest_hit(physics, body, tank.ram_velocity, low).and_then(|hit| hit.throw(&t)) else { return };
    knock_hull(physics, tank, dir, k, footing);
    tank.bounce_rest = t.hull_bounce_rest_seconds;
}

impl Game {
    /// The hard and soft faces' say after the step (the module docs).
    /// Called from `sync_tanks_and_ram` once every hull's position has
    /// been read back, before the rams.
    pub(super) fn respond_to_contacts(&mut self, f: &mut Frame) {
        let t = tuning();
        let low = lowest(&t);
        let hulls: Vec<(Entity, usize)> = ordered(
            self.world.query::<(Entity, &Tank)>().iter().filter(|(_, tank)| tank.body.is_some() && !tank.is_wreck() && tank.ram_velocity.length() >= low),
            |(_, tank)| tank.owner_slot(),
        )
        .into_iter()
        .map(|(e, tank)| (e, tank.owner_slot()))
        .collect();
        let mut cracks: Vec<((i32, i32), Entity)> = Vec::new();
        for (entity, slot) in hulls {
            let (body, velocity) = {
                let tank = self.world.get::<&Tank>(entity).expect("a hull walked this frame exists");
                (tank.body.expect("filtered on a body"), tank.ram_velocity)
            };
            let Some(hit) = hardest_hit(&self.physics, body, velocity, low) else { continue };
            let tile = self.obstacle_of_body(hit.body);
            let material = tile.map(|(_, m)| m);
            let crack = hit.cracks(&t);
            if let (true, Some((tile, _))) = (crack, tile) {
                let cell = self.world.get::<&Obstacle>(tile).map(|o| o.cell()).unwrap_or((0, 0));
                cracks.push((cell, tile));
            }
            // A hull coming back at the face it was just thrown off is not
            // a fresh hit.
            let resting = self.world.get::<&Tank>(entity).is_ok_and(|tank| tank.bounce_rest > 0.0);
            let mut bounced = false;
            if let (false, Some((dir, k))) = (resting, hit.throw(&t)) {
                let footing = self.footing_of(entity);
                let mut tank = self.world.get::<&mut Tank>(entity).expect("a hull walked this frame exists");
                // Untold: a client that owns the hull threw it itself
                // (`throw_off_faces`); the pose validator allows the speed.
                if let Some((dv, skid)) = knock_hull(&mut self.physics, &mut tank, dir, k, footing) {
                    f.shoves.allow_knock(tank.owner(), dv, skid);
                }
                tank.bounce_rest = t.hull_bounce_rest_seconds;
                bounced = true;
            }
            if !resting && hit.speed >= t.hull_struck_min_speed {
                f.events.push(Event::HullStruck {
                    slot,
                    material,
                    surface: hit.surface,
                    x: hit.point.x,
                    y: hit.point.y,
                    nx: hit.normal.x,
                    ny: hit.normal.y,
                    speed: hit.speed,
                    bounced,
                });
                if material.is_some_and(|m| m.is_tree()) {
                    self.shake_tree(hit.point, hit.normal, hit.speed);
                }
            }
        }
        // In cell order: a pane that dies queues what it leaves.
        cracks.sort_by_key(|&((c, r), _)| (r, c));
        cracks.dedup_by_key(|(cell, _)| *cell);
        let damage = t.glass_ram_damage;
        for (_, tile) in cracks {
            if damage > 0.0 {
                self.damage_obstacle(f, tile, damage, DamageCause::Ram);
            }
        }
    }

    /// The live tile whose body is `body`, and its material.
    fn obstacle_of_body(&self, body: rapier2d::prelude::RigidBodyHandle) -> Option<(Entity, Material)> {
        self.world.query::<(Entity, &Obstacle)>().iter().find(|(_, o)| o.body == body && !o.destroyed).map(|(e, o)| (e, o.material))
    }

    /// The ground under a hull (`Footing`), for the skid a bounce leaves.
    fn footing_of(&self, entity: Entity) -> Footing {
        let pos = self.world.get::<&Tank>(entity).map(|t| t.position).unwrap_or(Position::new(0.0, 0.0));
        Footing::at(&self.water, &self.lava, &self.craters, self.weather, pos, self.time)
    }

    /// Set swinging the crown of the tree a hull met at `at` going `way`
    /// (unit) into it at `speed`: the tree in the cell just past the
    /// contact. The one way a shake starts, on the round and on a replica
    /// alike (`Event::HullStruck`).
    pub(crate) fn shake_tree(&mut self, at: Position, way: Position, speed: f32) {
        let t = tuning();
        let now = self.time;
        let life = t.tree_shake_seconds;
        self.tree_shakes.retain(|s| now - s.at < life);
        let cell = world_to_cell(Position::new(at.x + way.x * 4.0, at.y + way.y * 4.0));
        self.tree_shakes.retain(|s| s.cell != cell);
        let side = if way.x < -0.1 { -1.0 } else if way.x > 0.1 { 1.0 } else if (cell.0 + cell.1) % 2 == 0 { 1.0 } else { -1.0 };
        let strength = (speed / t.tank_speed.max(1.0)).clamp(0.3, 1.5);
        self.tree_shakes.push(TreeShake { cell, side, strength, at: now });
    }

    /// The sideways lean a hit's shake adds to the tree at `cell` now, px.
    pub fn tree_shake_lean(&self, cell: (i32, i32), t: &Tuning) -> f32 {
        self.tree_shakes.iter().filter(|s| s.cell == cell).map(|s| s.lean(self.time, t)).sum()
    }
}
