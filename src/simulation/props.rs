//! The destructible props' rules (docs/sandbags-barrels-fences.md): the one
//! place an obstacle loses health, a barrel's blast and the chain reaction
//! it starts, fuse timers, and tanks ramming sandbags, fences and barrels.

use crate::tuning::tuning;
use hecs::Entity;
use rand::RngExt;
use rapier2d::prelude::RigidBodyHandle;

use crate::ai::Ai;
use crate::blast::{BlastFx, Scorch};
use crate::decal::Decal;
use crate::frog::Frog;
use crate::obstacle::{Material, Obstacle};
use crate::shockwave::Shockwave;
use crate::tank::Tank;
use crate::{OBSTACLE_GRID_SIZE, Position};

use super::combat::{explosion_hit, BlastParams};
use super::{Event, Frame, Game, SHOCK_BARREL, SHOCK_FROG};

/// Everything `obstacle_died` needs to know about a tile that just died.
/// Bundled rather than passed loose because each death path has the whole
/// `&mut Obstacle` in hand and can fill it in one go, and because a
/// death now carries more than a position: `charred` picks which rubble
/// a burnt-out plank leaves.
pub(super) struct DeadTile {
    pub material: Material,
    pub position: Position,
    /// Another blast set this one off, rather than a shot or a ram.
    pub chained: bool,
    /// Wood only: it burnt out rather than snapping, so it leaves the
    /// charred cell instead of the destroyed one.
    pub charred: bool,
}

/// What is damaging an obstacle - the fence and barrel rules differ by it.
/// `Blast` carries the linear falloff at the victim (1 at the centre, 0 at
/// the radius), which sets how long a chained barrel's fuse burns.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum DamageCause {
    Shot,
    Ram,
    Blast(f32),
}

impl Game {
    /// Apply `amount` of damage to the obstacle `entity` - the only place on
    /// the simulation path an obstacle loses health, so the per-material
    /// rules live here once: a fence is amount-blind (a pristine one dies
    /// outright at `fence_one_shot_chance` odds, else drops to its damaged
    /// stage; a damaged one always dies); a barrel caught in a blast is
    /// always lethal but goes on a fuse - `barrel_fuse_seconds` scaled by
    /// distance, half of it at the blast's centre up to two and a half
    /// times at its edge, so a cluster cascades outward rather than going
    /// off all at once - while
    /// a direct hit or a ram pops it right away through the plain health
    /// path; a tree pushed over by a tank dies outright instead of taking
    /// its flammable fork, since a hull is not something that sets a tree
    /// alight; everything else is `Obstacle::damage`. Returns `true` the
    /// frame the obstacle dies.
    pub(super) fn damage_obstacle(&mut self, f: &mut Frame, entity: Entity, amount: f32, cause: DamageCause) -> bool {
        let (material, pos, died) = {
            let mut q = self.world.query_one::<&mut Obstacle>(entity);
            let Ok(o) = q.get() else { return false };
            if o.destroyed || o.burning || o.fuse.is_some() {
                return false;
            }
            let died = match o.material {
                Material::Fence => {
                    let pristine = o.health >= o.max_health;
                    if pristine && !f.rng.random_bool(tuning().fence_one_shot_chance) {
                        o.health = o.max_health * 0.5;
                        false
                    } else {
                        o.health = 0.0;
                        o.destroyed = true;
                        true
                    }
                }
                Material::Barrel => match cause {
                    DamageCause::Blast(falloff) => {
                        o.health = 0.0;
                        o.fuse = Some(tuning().barrel_fuse_seconds * (0.5 + 2.0 * (1.0 - falloff.clamp(0.0, 1.0))));
                        false
                    }
                    _ => o.damage(amount),
                },
                // A tree shouldered over by a tank goes over, it does not
                // catch fire: fire comes from what is burning at the muzzle
                // end, and a hull pushing a trunk has none. Ignition stays
                // on the shot and blast paths.
                m if m.is_tree() && cause == DamageCause::Ram => {
                    o.health = 0.0;
                    o.destroyed = true;
                    true
                }
                _ => o.damage(amount),
            };
            (o.material, o.position, died)
        };
        if died {
            self.obstacle_died(f, DeadTile { material, position: pos, chained: false, charred: false });
        }
        died
    }

    /// Record an obstacle's death and leave its rubble behind; an
    /// explosive one queues its blast for `explosions` to resolve this
    /// frame. Every tile death in the game funnels through here - a shot,
    /// a ram, a blast, a lit fuse and a burnt-out plank all arrive at this
    /// one place - so it is the only spot that has to know what a dead
    /// tile leaves on the ground.
    fn obstacle_died(&mut self, f: &mut Frame, tile: DeadTile) {
        let DeadTile { material, position: pos, chained, charred } = tile;
        f.events.push(Event::ObstacleDestroyed { material, x: pos.x, y: pos.y });
        // Materials with no terminal sheet cell (Iron, and the props)
        // leave nothing here; props get purpose-drawn wreckage instead.
        if let Some(decal) = Decal::new(material, pos, charred) {
            f.decals.push(decal);
        }
        if material.is_explosive() {
            f.events.push(Event::Blast { x: pos.x, y: pos.y, chained });
            f.pending_blasts.push(pos);
        }
    }

    /// A barrel's detonation at `center`: everyone inside
    /// `barrel_blast_radius` takes linear-falloff damage and a shove -
    /// player and enemies alike, both frogs, every tile - and any other
    /// barrel in range is put on a fuse. `live` is false on the end
    /// screen, where a cascade finishes visually without dealing damage.
    /// Visuals: the full-screen ripple and camera shake (`f.shock`), the
    /// impact-flash quad, the fireball animation and a scorch mark.
    pub(super) fn apply_blast(&mut self, f: &mut Frame, center: Position, live: bool) {
        if live {
            let params = BlastParams::barrel();
            let player = self.player.expect("player entity spawned in init");
            {
                let mut q = self.world.query_one::<&mut Tank>(player);
                let tank = q.get().expect("player entity always has a Tank");
                explosion_hit(tank, center, true, false, &mut self.physics, &mut f.rng, &mut f.kills, &params);
            }
            for tank in self.world.query::<&mut Tank>().with::<&Ai>().iter() {
                explosion_hit(tank, center, true, true, &mut self.physics, &mut f.rng, &mut f.kills, &params);
            }
            let mut dead_frogs = Vec::new();
            for frog in self.world.query::<&mut Frog>().iter() {
                if frog.is_dead() {
                    continue;
                }
                let dist = frog.position.distance_to(center);
                if dist > params.radius {
                    continue;
                }
                frog.damage(params.roll_damage(&mut f.rng) * (1.0 - dist / params.radius));
                if frog.is_dead() {
                    dead_frogs.push(frog.position);
                }
            }
            for pos in dead_frogs {
                f.shocks.push(Shockwave::scaled(pos, SHOCK_FROG));
            }
            // Collect first: `damage_obstacle` needs the world free.
            let hits: Vec<(Entity, Material, f32)> = self
                .world
                .query::<(Entity, &Obstacle)>()
                .iter()
                .filter(|(_, o)| !o.destroyed && o.fuse.is_none())
                .filter_map(|(e, o)| {
                    let dist = o.position.distance_to(center);
                    (dist <= params.radius).then(|| (e, o.material, 1.0 - dist / params.radius))
                })
                .collect();
            for (entity, material, falloff) in hits {
                // Barrels inside always chain - no roll needed.
                let amount = if material.is_explosive() { f32::MAX } else { params.roll_damage(&mut f.rng) * falloff };
                self.damage_obstacle(f, entity, amount, DamageCause::Blast(falloff));
            }
        }
        f.shocks.push(Shockwave::scaled(center, SHOCK_BARREL));
        f.impact_flashes.push(Shockwave::new(center));
        f.blast_fx.push(BlastFx::new(center));
        self.flash_screen();
        f.scorches.push(Scorch::new(center));
    }

    /// Count every lit fuse down; a barrel whose fuse runs out dies and
    /// queues its own blast (`chained`), which `explosions` resolves this
    /// same frame so the cascade keeps going.
    /// Advance every burning wood tile's fire and report the ones that
    /// finish charring. Wood is the only material with a death that does
    /// not go through `damage_obstacle`: `Obstacle::tick_burn` sets
    /// `destroyed` itself once `wood_burn_seconds` are up. Routing that
    /// through `obstacle_died` is what makes a burnt-out tile announce
    /// itself like every other tile death - without it `cleanup_done`
    /// simply despawned it and nothing downstream (the event log, the
    /// dev server, debris) ever heard. Collects first, exactly like
    /// `tick_fuses`, because `obstacle_died` needs the world free.
    pub(super) fn tick_burns(&mut self, f: &mut Frame) {
        let mut charred = Vec::new();
        for o in self.world.query::<&mut Obstacle>().iter() {
            let was_destroyed = o.destroyed;
            o.tick_burn(f.dt);
            if o.destroyed && !was_destroyed {
                charred.push((o.material, o.position));
            }
        }
        for (material, position) in charred {
            self.obstacle_died(f, DeadTile { material, position, chained: false, charred: true });
        }
    }

    pub(super) fn tick_fuses(&mut self, f: &mut Frame) {
        let mut popped = Vec::new();
        for o in self.world.query::<&mut Obstacle>().iter() {
            let Some(t) = o.fuse.as_mut() else { continue };
            *t -= f.dt;
            if *t <= 0.0 && !o.destroyed {
                o.destroyed = true;
                popped.push((o.material, o.position));
            }
        }
        for (material, position) in popped {
            self.obstacle_died(f, DeadTile { material, position, chained: true, charred: false });
        }
    }

    /// Tanks pushing into props and trees. A live tank with a body and a
    /// commanded move counts as a pusher; a tile it is in narrow-phase
    /// contact with
    /// (`Physics::touching`, the same contact state the ram check between
    /// tanks reads) accumulates `ram_timer` and collapses at its
    /// `Material::ram_seconds` - a sandbag slows the tank for a moment,
    /// a fence barely at all - while a barrel takes
    /// `barrel_ram_damage_per_second` and pops in the rammer's face. The
    /// timer decays while nothing pushes, so nudging a sandbag twice
    /// doesn't add up. No RNG.
    pub(super) fn ram_props(&mut self, f: &mut Frame) {
        let pushers: Vec<(RigidBodyHandle, Position)> = self
            .world
            .query::<&Tank>()
            .iter()
            .filter(|t| !t.is_wreck() && (t.velocity.x != 0.0 || t.velocity.y != 0.0))
            .filter_map(|t| t.body.map(|b| (b, t.position)))
            .collect();
        // Coarse distance cull before asking the narrow phase.
        let reach = OBSTACLE_GRID_SIZE * 2.0;
        let mut collapsed = Vec::new();
        let mut barrel_hits = Vec::new();
        for (entity, o) in self.world.query::<(Entity, &mut Obstacle)>().iter() {
            if o.destroyed || o.fuse.is_some() {
                continue;
            }
            // Only what ramming can actually do something to: a prop that
            // collapses, a tree that goes over, or a barrel that pops.
            if o.material.ram_seconds().is_none() && !o.material.is_explosive() {
                continue;
            }
            let pushed = pushers
                .iter()
                .any(|&(body, pos)| pos.distance_to(o.position) < reach && self.physics.touching(body, o.body));
            match o.material.ram_seconds() {
                Some(limit) => {
                    o.ram_timer = if pushed { o.ram_timer + f.dt } else { (o.ram_timer - f.dt).max(0.0) };
                    if pushed && o.ram_timer >= limit {
                        collapsed.push(entity);
                    }
                }
                None if pushed && o.material.is_explosive() => barrel_hits.push(entity),
                None => {}
            }
        }
        for entity in collapsed {
            self.damage_obstacle(f, entity, f32::MAX, DamageCause::Ram);
        }
        let ram_damage = tuning().barrel_ram_damage_per_second * f.dt;
        for entity in barrel_hits {
            self.damage_obstacle(f, entity, ram_damage, DamageCause::Ram);
        }
    }
}
