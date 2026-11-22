//! What a hull does as it meets a fixed face, by how hard the face is
//! (`obstacle::Surface`, docs/physics-engine-design.md "Hard and soft").
//!
//! The cushion of a soft face is the solver's (`physics::SurfaceHooks`):
//! it acts inside the step. What is left is judged here, after the step,
//! from each hull's velocity going into it (`Tank::ram_velocity`) and the
//! fixed bodies it ended the step touching (`Physics::fixed_contacts`):
//!
//! - a hard face hit at `hull_bounce_min_speed` or more throws the hull
//!   back - a knock along the face's normal (`combat::knock`, so a
//!   client-owned seat is told), with a short skid so the throw is seen;
//! - a pane of glass hit at `glass_crack_speed` or more cracks
//!   (`glass_ram_damage`) instead;
//! - a hit at `hull_struck_min_speed` or more is reported
//!   (`Event::HullStruck`), and a tree hit that way shakes its crown
//!   (`TreeShake`).
//!
//! Only the contact's normal is ever touched: a hull sliding along a face
//! is not going into it and nothing here sees it. No RNG; hulls are
//! walked by owner slot.

use hecs::Entity;

use crate::knock::Knock;
use crate::map::world_to_cell;
use crate::math::Vec2;
use crate::obstacle::{Material, Obstacle, Surface};
use crate::tank::Tank;
use crate::tuning::{tuning, Tuning};
use crate::Position;

use super::combat::knock;
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
    body: rapier2d::prelude::RigidBodyHandle,
    surface: Surface,
    normal: Position,
    point: Position,
    speed: f32,
}

impl Game {
    /// The hard and soft faces' say after the step (the module docs).
    /// Called from `sync_tanks_and_ram` once every hull's position has
    /// been read back, before the rams.
    pub(super) fn respond_to_contacts(&mut self, f: &mut Frame) {
        let t = tuning();
        let low = t.hull_struck_min_speed.min(t.hull_bounce_min_speed).min(t.glass_crack_speed).max(1.0);
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
            // The hardest of its hits: the face it went into fastest. A
            // `Plain` body (deep water, lava, the bong) is the solver's.
            let mut best: Option<Struck> = None;
            for c in self.physics.fixed_contacts(body) {
                let speed = velocity.x * c.normal.x + velocity.y * c.normal.y;
                if c.surface == Surface::Plain || speed < low || best.as_ref().is_some_and(|b| b.speed >= speed) {
                    continue;
                }
                best = Some(Struck { body: c.other, surface: c.surface, normal: c.normal, point: c.point, speed });
            }
            let Some(hit) = best else { continue };
            let tile = self.obstacle_of_body(hit.body);
            let material = tile.map(|(_, m)| m);
            let crack = hit.surface == Surface::Brittle && hit.speed >= t.glass_crack_speed;
            if let (true, Some((tile, _))) = (crack, tile) {
                let cell = self.world.get::<&Obstacle>(tile).map(|o| o.cell()).unwrap_or((0, 0));
                cracks.push((cell, tile));
            }
            // A hull coming back at the face it was just thrown off is not
            // a fresh hit.
            let resting = self.world.get::<&Tank>(entity).is_ok_and(|tank| tank.bounce_rest > 0.0);
            let mut bounced = false;
            let bounce = hit.surface.bounce(&t);
            if !crack && !resting && bounce > 0.0 && hit.speed >= t.hull_bounce_min_speed {
                let footing = self.footing_of(entity);
                let mut tank = self.world.get::<&mut Tank>(entity).expect("a hull walked this frame exists");
                let k = Knock { speed: bounce * hit.speed, mass_exponent: 0.0, max_speed: t.knockback_max_speed, skid: t.hull_bounce_skid_seconds };
                knock(&mut self.physics, &mut f.shoves, &mut tank, Vec2::new(-hit.normal.x, -hit.normal.y), k, footing);
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
