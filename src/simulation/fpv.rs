//! The FPV swarm's world half (docs/fpv-swarm.md; `fpv.rs` is the drone,
//! its flight and its picture). A press queues a launch (`fire_fpv`);
//! `launch_drones` puts the drone in the world once the tank loops are
//! done, locked on what it is after; `guide_drones` keeps each aim on its
//! lock and drops a lock that is lost; `step_world` flies them; and
//! `resolve_drones` bursts the ones that came down on their point and
//! takes away the ones that crashed. Also the first **air targets**
//! (`air.rs`): the list every reader takes (`Game::air_targets`) and the
//! one thing that changes one (`Game::strike_air`). No RNG anywhere here
//! but a burst's own rules - a fence's odds, a drum's fuse - where it lands.

use hecs::Entity;

use crate::ai::{Ai, in_sight_box};
use crate::air::{AirKey, AirStrike, AirTarget};
use crate::blast::{BlastFx, BlastKind, BlastShape, Lean, Scorch};
use crate::fpv::{self, AirWant, Drone, DroneLock, DroneStage, LockCandidate};
use crate::frog::{Frog, Side};
use crate::math::Vec2;
use crate::obstacle::Obstacle;
use crate::shell::Owner;
use crate::shockwave::Shockwave;
use crate::tank::Tank;
use crate::tuning::tuning;
use crate::{OBSTACLE_GRID_SIZE, Position};

use super::combat::BlastParams;
use super::props::DamageCause;
use super::{Event, Frame, Game, RollIn, Spectacle};

/// A drone launched this frame, waiting for `launch_drones` to put it in
/// the world.
pub(super) struct PendingDrone {
    /// The halo slot it leaves, that slot's ground point and outward
    /// bearing.
    pub slot: u8,
    pub origin: Position,
    pub out: Vec2,
    pub owner: Owner,
    /// Where its launcher stood and the way its hull faced: what its lock
    /// is measured from and where its aim point lies.
    pub from: Position,
    pub facing: Vec2,
    pub want: AirWant,
}

impl BlastParams {
    /// One FPV drone's burst, launched by `by`: fixed damage, no roll.
    pub fn drone(by: Owner) -> Self {
        let t = tuning();
        BlastParams { radius: t.fpv_blast_radius_px, damage: (t.fpv_damage, t.fpv_damage), knockback: t.fpv_knockback_speed, by: Some(by) }
    }
}

/// Fire one drone of `tank`'s halo: the top slot's (`fpv_drones - 1`
/// before the press spends it), its module's launch cell, and the launch
/// queued for `launch_drones` with what the tank's AI asked it to lock
/// (`Tank::fpv_want`, taken). No recoil: a quadcopter lifts itself. No RNG.
pub(super) fn fire_fpv(f: &mut Frame, tank: &mut Tank, owner: Owner) {
    let slots = tuning().fpv_drones_per_pickup.max(1) as usize;
    let slot = (tank.fpv_drones.max(1) as usize - 1).min(slots - 1);
    let (origin, out) = fpv::halo_slot(tank.position, tank.sprite_size(), slot, slots);
    let r = tank.rotation.to_radians();
    tank.kick_fpv();
    f.pending_drones.push(PendingDrone {
        slot: slot as u8,
        origin,
        out,
        owner,
        from: tank.position,
        facing: Vec2::new(r.sin(), -r.cos()),
        want: tank.fpv_want.take().unwrap_or(AirWant::Nearest),
    });
}

impl Game {
    /// The standing trees' cells' centres, in the world's tile order: what
    /// a crown is measured round (`fpv::crown_box`).
    pub(crate) fn standing_trees(&self) -> Vec<(Entity, Position)> {
        let mut trees: Vec<(Entity, Position, (i32, i32))> = self
            .world
            .query::<(Entity, &Obstacle)>()
            .iter()
            .filter(|(_, o)| !o.destroyed && o.material.is_tree())
            .map(|(e, o)| (e, o.position, o.cell()))
            .collect();
        trees.sort_by_key(|&(_, _, c)| (c.1, c.0));
        trees.into_iter().map(|(e, p, _)| (e, p)).collect()
    }

    /// The tree whose crown `hull` (centre, half extents) overlaps - what
    /// is under canopy (docs/fpv-swarm.md "Trees") - the nearest tree first,
    /// ties to cell order.
    pub(crate) fn canopy_over(trees: &[(Entity, Position)], hull: (Position, Vec2)) -> Option<(Entity, Position)> {
        let canopy = tuning().fpv_canopy_px;
        trees
            .iter()
            .filter(|&&(_, p)| fpv::boxes_overlap(hull, fpv::crown_box(p, canopy)))
            .min_by(|a, b| a.1.distance_to(hull.0).total_cmp(&b.1.distance_to(hull.0)))
            .copied()
    }

    /// The tree whose crown holds the point `p`, nearest first.
    fn crown_at(trees: &[(Entity, Position)], p: Position) -> Option<(Entity, Position)> {
        let canopy = tuning().fpv_canopy_px;
        trees
            .iter()
            .filter(|&&(_, c)| fpv::box_holds(fpv::crown_box(c, canopy), p))
            .min_by(|a, b| a.1.distance_to(p).total_cmp(&b.1.distance_to(p)))
            .copied()
    }

    /// A tank's hull box as `fpv`'s geometry takes it.
    fn hull_of(tank: &Tank) -> (Position, Vec2) {
        let (c, h) = tank.hull_bbox_world();
        (c, Vec2::new(h.x, h.y))
    }

    /// A frog's box as the crown is measured against it: its centre alone.
    fn frog_box(frog: &Frog) -> (Position, Vec2) {
        (frog.position, Vec2::zero())
    }

    /// Put this frame's launches in the world, each locked on what it is
    /// after (docs/fpv-swarm.md "Which target") and given the round's next
    /// projectile id; `Event::DroneLaunched` after its `Fired`.
    pub(super) fn launch_drones(&mut self, f: &mut Frame) {
        if f.pending_drones.is_empty() {
            return;
        }
        let trees = self.standing_trees();
        let field = (f.width, f.height);
        let pending: Vec<PendingDrone> = f.pending_drones.drain(..).collect();
        for p in pending {
            let (lock, aim) = self.pick_lock(&p, &trees, field);
            let mut drone = Drone::launch(p.origin, p.out, p.slot, p.owner, lock, aim);
            drone.id = self.take_shot_id();
            let (target, frog) = match lock {
                DroneLock::Tank { slot, .. } => (Some(slot), false),
                DroneLock::Frog { .. } => (None, true),
                DroneLock::None => (None, false),
            };
            f.events.push(Event::DroneLaunched { id: drone.id, slot: p.owner.slot(), x: p.origin.x, y: p.origin.y, target, frog });
            self.world.spawn((drone,));
        }
        self.count_drones_out();
    }

    /// What a launch locks and its first aim: a seat's drone the nearest
    /// enemy inside the seat's sight box not under canopy; an enemy's what
    /// its AI asked, checked again here; with no lock the aim point
    /// `fpv_aim_px` ahead of the launcher, or the enemy's chosen point.
    fn pick_lock(&self, p: &PendingDrone, trees: &[(Entity, Position)], field: (f32, f32)) -> (DroneLock, Position) {
        let t = tuning();
        let margin = OBSTACLE_GRID_SIZE * 0.5;
        let ahead = fpv::clamp_inside(p.from + p.facing * t.fpv_aim_px, field, margin);
        let enemies = || {
            let mut out: Vec<(Entity, usize, Position)> = self
                .world
                .query::<(Entity, &Tank)>()
                .with::<&Ai>()
                .without::<&RollIn>()
                .iter()
                .filter(|(_, tank)| !tank.is_wreck() && tank.body.is_some() && Self::canopy_over(trees, Self::hull_of(tank)).is_none())
                .map(|(e, tank)| (e, tank.owner_slot(), tank.position))
                .collect();
            out.sort_by_key(|&(_, slot, _)| slot);
            out
        };
        let seat_ok = |seat: u8| -> Option<(Entity, Position)> {
            let entity = self.seats_on_field().get(seat as usize).copied().flatten()?;
            let tank = self.world.get::<&Tank>(entity).ok()?;
            let ok = !tank.is_wreck() && in_sight_box(tank.position, p.from) && Self::canopy_over(trees, Self::hull_of(&tank)).is_none();
            ok.then_some((entity, tank.position))
        };
        match (p.owner, p.want) {
            (Owner::Player(_), _) => {
                let cands: Vec<(Entity, usize, Position)> = enemies().into_iter().filter(|&(_, _, pos)| in_sight_box(p.from, pos)).collect();
                let picks: Vec<LockCandidate> = cands.iter().map(|&(_, slot, pos)| LockCandidate { key: slot, pos }).collect();
                match fpv::pick_nearest(p.from, &picks) {
                    Some(i) => (DroneLock::Tank { entity: cands[i].0, slot: cands[i].1 }, cands[i].2),
                    None => (DroneLock::None, ahead),
                }
            }
            (_, AirWant::Seat(seat)) => match seat_ok(seat) {
                Some((entity, pos)) => (DroneLock::Tank { entity, slot: seat as usize }, pos),
                None => (DroneLock::None, ahead),
            },
            (_, AirWant::Frog) => {
                let frog = self.frog.and_then(|e| {
                    let fr = self.world.get::<&Frog>(e).ok()?;
                    (!fr.is_dead() && Self::crown_at(trees, fr.position).is_none() && Self::canopy_over(trees, Self::frog_box(&fr)).is_none())
                        .then_some((e, fr.position))
                });
                match frog {
                    Some((entity, pos)) => (DroneLock::Frog { entity, side: Side::Player }, pos),
                    None => (DroneLock::None, ahead),
                }
            }
            (_, AirWant::Point(at)) => (DroneLock::None, fpv::clamp_inside(at, field, margin)),
            (_, AirWant::Nearest) => {
                let seats: Vec<(Entity, usize, Position)> = (0..self.players.count() as u8)
                    .filter_map(|s| seat_ok(s).map(|(e, pos)| (e, s as usize, pos)))
                    .collect();
                let picks: Vec<LockCandidate> = seats.iter().map(|&(_, slot, pos)| LockCandidate { key: slot, pos }).collect();
                match fpv::pick_nearest(p.from, &picks) {
                    Some(i) => (DroneLock::Tank { entity: seats[i].0, slot: seats[i].1 }, seats[i].2),
                    None => (DroneLock::None, ahead),
                }
            }
        }
    }

    /// Keep every tracking drone's aim on its lock, and drop a lock that
    /// is lost - its tank a wreck or gone, under a tree's crown, or through
    /// a portal this frame - leaving the aim where it last was
    /// (`Event::DroneLockLost`). Before the step, like `guide_missiles`.
    pub(super) fn guide_drones(&mut self, f: &mut Frame) {
        let any = self.world.query::<&Drone>().iter().any(|d| d.tracking());
        if !any {
            return;
        }
        let trees = self.standing_trees();
        let teleported: Vec<usize> = f
            .events
            .iter()
            .filter_map(|e| match *e {
                Event::Teleported { slot, .. } => Some(slot),
                _ => None,
            })
            .collect();
        // Where each lock stands, or why it is lost.
        let mut drones: Vec<(u32, Entity, DroneLock)> =
            self.world.query::<(Entity, &Drone)>().iter().filter(|(_, d)| d.tracking()).map(|(e, d)| (d.id, e, d.lock)).collect();
        drones.sort_by_key(|&(id, _, _)| id);
        for (id, entity, lock) in drones {
            let found: Result<Position, &'static str> = match lock {
                DroneLock::Tank { entity: target, slot } => match self.world.get::<&Tank>(target) {
                    Err(_) => Err("gone"),
                    Ok(tank) if tank.is_wreck() => Err("wreck"),
                    Ok(_) if teleported.contains(&slot) => Err("teleport"),
                    Ok(_) if self.world.get::<&RollIn>(target).is_ok() => Err("gone"),
                    Ok(tank) if Self::canopy_over(&trees, Self::hull_of(&tank)).is_some() => Err("canopy"),
                    Ok(tank) => Ok(tank.position),
                },
                DroneLock::Frog { entity: target, .. } => match self.world.get::<&Frog>(target) {
                    Err(_) => Err("gone"),
                    Ok(fr) if fr.is_dead() => Err("wreck"),
                    Ok(fr) if Self::canopy_over(&trees, Self::frog_box(&fr)).is_some() || Self::crown_at(&trees, fr.position).is_some() => Err("canopy"),
                    Ok(fr) => Ok(fr.position),
                },
                DroneLock::None => continue,
            };
            let Ok(mut d) = self.world.get::<&mut Drone>(entity) else { continue };
            match found {
                Ok(at) => d.aim = fpv::clamp_inside(at, (f.width, f.height), 0.0),
                Err(why) => {
                    d.lock = DroneLock::None;
                    f.events.push(Event::DroneLockLost { id, why });
                }
            }
        }
    }

    /// One fixed step of every drone in the air (from `step_world`): the
    /// wind a sandstorm's gust carries it by, past its climb.
    pub(super) fn advance_drones(&mut self, dt: f32, field: (f32, f32)) {
        let t = tuning();
        let (sky, time) = (self.weather, self.time);
        for d in self.world.query_mut::<&mut Drone>() {
            let wind = if matches!(d.stage, DroneStage::Cruise | DroneStage::Dive) && t.fpv_gust_factor > 0.0 {
                crate::weather::gust_at(sky, d.ground, time, &t) * t.fpv_gust_factor
            } else {
                Vec2::zero()
            };
            d.advance(dt, wind, field);
        }
    }

    /// Burst every drone whose dive came down on its point this frame -
    /// in a tree's crown, or on the ground - and take away every downed
    /// one that reached the ground, a dud. `live` is false on the end
    /// screen, where a burst hurts nobody.
    pub(super) fn resolve_drones(&mut self, f: &mut Frame, live: bool) {
        let mut landed: Vec<(u32, Entity, Position, Owner, Vec2, bool)> = self
            .world
            .query::<(Entity, &Drone)>()
            .iter()
            .filter(|(_, d)| d.landed)
            .map(|(e, d)| (d.id, e, d.ground, d.owner, d.velocity(), d.stage == DroneStage::Falling))
            .collect();
        if landed.is_empty() {
            return;
        }
        landed.sort_by_key(|&(id, ..)| id);
        let trees = self.standing_trees();
        for (id, entity, at, owner, dir, crashed) in landed {
            self.world.despawn(entity).ok();
            if crashed {
                f.events.push(Event::DroneCrashed { id, x: at.x, y: at.y });
                continue;
            }
            let crown = Self::crown_at(&trees, at);
            f.events.push(Event::DroneBurst { id, slot: owner.slot(), x: at.x, y: at.y, crown: crown.is_some() });
            if live {
                match crown {
                    Some((tree, _)) => {
                        let amount = tuning().fpv_tree_damage;
                        if amount > 0.0 {
                            self.damage_obstacle(f, tree, amount, DamageCause::Blast { falloff: 1.0, from: at });
                        }
                    }
                    None => {
                        let spared = self.spared_by_canopy(&trees);
                        self.side_blast_sparing(f, at, owner, &BlastParams::drone(owner), &spared);
                    }
                }
            }
            let mut show = Spectacle::default();
            self.drone_show(&mut show, at, dir, crown.is_some());
            f.stage(show);
        }
        self.count_drones_out();
    }

    /// The tanks and frogs under a tree's crown, which a drone's burst
    /// leaves out whole.
    fn spared_by_canopy(&self, trees: &[(Entity, Position)]) -> Vec<Entity> {
        if trees.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<Entity> = self
            .world
            .query::<(Entity, &Tank)>()
            .iter()
            .filter(|(_, tank)| !tank.is_wreck() && Self::canopy_over(trees, Self::hull_of(tank)).is_some())
            .map(|(e, _)| e)
            .collect();
        out.extend(
            self.world
                .query::<(Entity, &Frog)>()
                .iter()
                .filter(|(_, fr)| Self::canopy_over(trees, Self::frog_box(fr)).is_some() || Self::crown_at(trees, fr.position).is_some())
                .map(|(e, _)| e),
        );
        out
    }

    /// The show a drone bursting at `center`, coming down along `dir`,
    /// puts on: the missile's burst, smaller - a fireball leaning down the
    /// dive, a weak ripple, the impact flash, and on dry open ground a
    /// small scorch and the grass round it flattened; in a crown no scorch.
    /// Everything but the damage, which is why a replica can call it off
    /// `Event::DroneBurst`. No RNG.
    pub(crate) fn drone_show(&mut self, show: &mut Spectacle, center: Position, dir: Vec2, crown: bool) {
        let t = tuning();
        let len = dir.length();
        let lean = if len > 1e-3 { Lean { x: dir.x / len, y: dir.y / len } } else { Lean { x: 0.0, y: 0.0 } };
        let mut fx = BlastFx::shaped(center, BlastKind::Oil, BlastShape::Shot { dir: lean });
        fx.scale *= t.fpv_blast_fx_scale;
        show.blast_fx.push(fx);
        if t.fpv_shock > 0.0 {
            show.shocks.push(Shockwave::scaled(center, t.fpv_shock));
        }
        show.impact_flashes.push(Shockwave::new(center));
        if !crown {
            if self.water.depth_at(center) == crate::ground::Depth::Dry {
                show.scorches.push(Scorch::with(center, t.fpv_blast_fx_scale, None));
            }
            crate::grass::flatten(&mut self.grass, center, t.fpv_blast_radius_px * t.blast_grass_flatten);
        }
    }

    /// Every tank's drones in the air, counted from the world
    /// (`Tank::fpv_out`): the module's link cell and the enemy rule's one
    /// at a time read it.
    pub(crate) fn count_drones_out(&mut self) {
        let mut out: std::collections::BTreeMap<usize, u8> = std::collections::BTreeMap::new();
        for d in self.world.query::<&Drone>().iter() {
            if d.in_air() {
                *out.entry(d.owner.slot()).or_insert(0) += 1;
            }
        }
        for tank in self.world.query_mut::<&mut Tank>() {
            tank.fpv_out = out.get(&tank.owner_slot()).copied().unwrap_or(0);
        }
    }

    /// Every air target - every drone in its launch, cruise or dive - by
    /// key (`air.rs`).
    pub fn air_targets(&self) -> Vec<AirTarget> {
        let mut out: Vec<AirTarget> = self.world.query::<&Drone>().iter().filter(|d| d.in_air()).map(Drone::as_target).collect();
        out.sort_by_key(|a| a.key);
        out
    }

    /// The drones in the world, by id: what the presentation, the wire and
    /// the tools read.
    pub fn drones(&self) -> Vec<Drone> {
        let mut out: Vec<Drone> = self.world.query::<&Drone>().iter().cloned().collect();
        out.sort_by_key(|d| d.id);
        out
    }

    /// Strike the air target `key` with `by` (docs/fpv-swarm.md
    /// "Shot down"): a bullet counts a hit, `fpv_drone_hits` of them bring
    /// a drone down; anything else downs it at once. `Event::DroneDowned`
    /// when it goes down. True when it did. The one thing that changes an
    /// air target.
    pub(super) fn strike_air(&mut self, f: &mut Frame, key: AirKey, by: AirStrike) -> bool {
        let AirKey::Drone(id) = key;
        let entity = self.world.query::<(Entity, &Drone)>().iter().find(|(_, d)| d.id == id).map(|(e, _)| e);
        let Some(entity) = entity else { return false };
        let Ok(mut d) = self.world.get::<&mut Drone>(entity) else { return false };
        if !d.in_air() {
            return false;
        }
        if by == AirStrike::Bullet {
            d.hits += 1;
            if d.hits < tuning().fpv_drone_hits.max(1) {
                return false;
            }
        }
        d.down(by);
        let (ground, height) = (d.ground, d.height);
        drop(d);
        f.events.push(Event::DroneDowned { id, x: ground.x, y: ground.y, height, by: by.name() });
        self.count_drones_out();
        true
    }

    /// Where `seat`'s next drone would leave its halo right now - its slot,
    /// that slot's ground point and outward bearing - from the sandbox's
    /// pose, for a client drawing its own launch (`net::predict`).
    pub(crate) fn seat_drone_slot(&self, seat: usize, owed: i32) -> Option<(u8, Position, Vec2)> {
        let entity = self.seats.get(seat).copied().flatten()?;
        let tank = self.world.get::<&Tank>(entity).ok()?;
        let slots = tuning().fpv_drones_per_pickup.max(1) as usize;
        let left = tank.fpv_drones - owed;
        if left <= 0 {
            return None;
        }
        let slot = (left as usize - 1).min(slots - 1);
        let (origin, out) = fpv::halo_slot(tank.position, tank.sprite_size(), slot, slots);
        Some((slot as u8, origin, out))
    }
}

/// A seat as an FPV-carrying enemy sees it this frame (`Game::fpv_senses`).
#[derive(Clone, Copy, Debug)]
pub(super) struct FpvSeat {
    pub seat: u8,
    pub entity: Entity,
    pub pos: Position,
    /// On the field and not a wreck.
    pub live: bool,
    /// In tall grass that hides it (`Terrain::conceals`).
    pub concealed: bool,
    /// How far an enemy sees it (`Game::sight_on`).
    pub sight: f32,
}

impl Game {
    /// Whether some live enemy carries an online FPV swarm.
    pub(super) fn any_fpv(&self) -> bool {
        self.world.query::<&Tank>().with::<&Ai>().iter().any(|t| !t.is_wreck() && t.active_weapon() == crate::tank::ActiveWeapon::FpvSwarm)
    }

    /// What each enemy carrying an online FPV swarm measures for its rule
    /// (`ai::FpvSense`, docs/fpv-swarm.md "AI"): the seat it would launch
    /// at, the seat hiding under a crown, a hunter's quarry, whether it
    /// stands in the open, where to back off to and its cover spot, kept on
    /// its `Ai` (`Ai::cover_spot`). Built before the tanks think, only when
    /// one carries the swarm. No RNG; ties on seat and cell.
    pub(super) fn fpv_senses(
        &mut self,
        f: &Frame,
        seats: &[FpvSeat],
        grid: &crate::pathfind::Grid,
        dangers: &[crate::ai::Danger],
    ) -> std::collections::BTreeMap<Entity, crate::ai::FpvSense> {
        let t = tuning();
        let trees = self.standing_trees();
        let field = (f.width, f.height);
        let hulls: Vec<(Entity, (Position, Vec2))> =
            seats.iter().map(|s| (s.entity, self.world.get::<&Tank>(s.entity).map(|tk| Self::hull_of(&tk)).unwrap_or((s.pos, Vec2::zero())))).collect();
        let quarry = self.frog.and_then(|e| {
            let fr = self.world.get::<&Frog>(e).ok()?;
            let hidden = Self::canopy_over(&trees, Self::frog_box(&fr)).is_some() || Self::crown_at(&trees, fr.position).is_some();
            (!fr.is_dead() && !hidden).then_some(fr.position)
        });
        let view = self.enemy_sight();
        let mut out = std::collections::BTreeMap::new();
        let mut q = self.world.query::<(Entity, &Tank, &mut Ai)>();
        for (entity, tank, ai) in q.iter() {
            if tank.is_wreck() || tank.active_weapon() != crate::tank::ActiveWeapon::FpvSwarm {
                continue;
            }
            let me = tank.position;
            let mine = tank.owner_slot();
            let safe = |p: Position| !dangers.iter().any(|d| d.owner != Some(mine) && d.depth(p) > 0.0);
            // The seats it knows of and whose sight box it stands in.
            let known: Vec<(&FpvSeat, bool)> = seats
                .iter()
                .zip(&hulls)
                .filter(|(s, _)| s.live && in_sight_box(s.pos, me) && me.distance_to(s.pos) <= s.sight && (!s.concealed || ai.is_hit_alerted()))
                .map(|(s, (_, hull))| (s, Self::canopy_over(&trees, *hull).is_some()))
                .collect();
            let nearest = |covered: bool| {
                known
                    .iter()
                    .filter(|(_, c)| *c == covered)
                    .min_by(|a, b| me.distance_to(a.0.pos).total_cmp(&me.distance_to(b.0.pos)).then(a.0.seat.cmp(&b.0.seat)))
                    .map(|(s, _)| **s)
            };
            let open_seat = nearest(false);
            let covered_seat = nearest(true);
            let canopy = covered_seat.and_then(|s| {
                let (_, hull) = hulls[seats.iter().position(|x| x.seat == s.seat)?];
                Self::canopy_over(&trees, hull).map(|(_, tree)| (s.seat, tree))
            });
            let target = open_seat.or(covered_seat);
            let exposed = target.is_some_and(|s| f.terrain.line_of_sight(s.pos, me));
            // Too close: the nearest it knows of that sees it.
            let back_off = known
                .iter()
                .filter(|(s, _)| me.distance_to(s.pos) < t.fpv_ai_min_range_px && f.terrain.line_of_sight(s.pos, me))
                .min_by(|a, b| me.distance_to(a.0.pos).total_cmp(&me.distance_to(b.0.pos)).then(a.0.seat.cmp(&b.0.seat)))
                .map(|(s, _)| {
                    let away = me - s.pos;
                    let len = away.length();
                    let dir = if len > 1e-3 { away * (1.0 / len) } else { crate::tank::Dir::from_rotation(tank.rotation).unwrap_or(crate::tank::Dir::Up).vec() };
                    fpv::clamp_inside(s.pos + dir * (t.fpv_ai_min_range_px + OBSTACLE_GRID_SIZE), field, OBSTACLE_GRID_SIZE * 0.5)
                })
                .filter(|&p| safe(p));
            // The cover spot, kept while it still hides it from the seat.
            let cover = match (exposed, target) {
                (true, Some(s)) => {
                    let keeps = |p: Position| {
                        in_sight_box(s.pos, p) && p.distance_to(s.pos) >= t.fpv_ai_min_range_px && safe(p) && !f.terrain.line_of_sight(s.pos, p)
                    };
                    let kept = ai.cover_spot.filter(|&(p, age)| age < t.fpv_ai_cover_seconds && keeps(p) && grid.connected(me, p));
                    let spot = match kept {
                        Some((p, age)) => Some((p, age + f.dt)),
                        None => fpv_cover_spot(me, grid, &keeps).map(|p| (p, 0.0)),
                    };
                    ai.cover_spot = spot;
                    spot.map(|(p, _)| p)
                }
                // In cover from the seat it would launch at: the patience
                // starts over. With no seat it keeps what it had - a cover
                // whose way out of the box loses the seat for a moment is
                // still the cover it cannot get to.
                (false, Some(_)) => {
                    ai.cover_spot = None;
                    ai.cover_waited = 0.0;
                    None
                }
                _ => {
                    ai.cover_spot = None;
                    None
                }
            };
            let hunter = ai.role == crate::ai::Role::Hunter;
            let quarry = quarry.filter(|&p| hunter && me.distance_to(p) <= view);
            out.insert(
                entity,
                crate::ai::FpvSense {
                    at_seat: open_seat.map(|s| (s.seat, s.pos)),
                    canopy,
                    quarry,
                    exposed,
                    cover,
                    back_off,
                    in_air: tank.fpv_out,
                },
            );
        }
        out
    }

    /// Every enemy a seat's drone is coming at - locked on it, or going for
    /// a point whose blast would reach it - and what it can do about it
    /// (`ai::AirThreat`, docs/fpv-swarm.md "Reacting to a seat's swarm"):
    /// the nearest such drone past its climb within the tank's sight,
    /// whether the tank stands inside that seat's box (it may fire at it),
    /// whether its hull is under a crown already and the nearest tree it
    /// can get under. Empty with no seat's drone in the air. No RNG; ties on
    /// id and cell.
    pub(super) fn air_threats(&self, grid: &crate::pathfind::Grid) -> std::collections::BTreeMap<Entity, crate::ai::AirThreat> {
        let mut out = std::collections::BTreeMap::new();
        let drones: Vec<Drone> = self
            .drones()
            .into_iter()
            .filter(|d| d.owner.is_player() && matches!(d.stage, DroneStage::Cruise | DroneStage::Dive))
            .collect();
        if drones.is_empty() {
            return out;
        }
        let t = tuning();
        let trees = self.standing_trees();
        let view = self.enemy_sight();
        let seats = self.players();
        let reach = t.fpv_blast_radius_px + OBSTACLE_GRID_SIZE * 0.5;
        let enemies: Vec<(Entity, Position)> =
            self.world.query::<(Entity, &Tank)>().with::<&Ai>().iter().filter(|(_, tk)| !tk.is_wreck()).map(|(e, tk)| (e, tk.position)).collect();
        let pairs = drones.iter().flat_map(|d| {
            let locked = match d.lock {
                DroneLock::Tank { entity, .. } => Some(entity),
                _ => None,
            };
            enemies.iter().filter(move |&&(e, p)| locked == Some(e) || (locked.is_none() && p.distance_to(d.aim) <= reach)).map(move |&(e, _)| (d, e))
        });
        for (d, entity) in pairs {
            let Ok(tank) = self.world.get::<&Tank>(entity) else { continue };
            let me = tank.position;
            let dist = me.distance_to(d.ground);
            if dist > view {
                continue;
            }
            let better = out.get(&entity).is_none_or(|old: &crate::ai::AirThreat| dist < me.distance_to(old.drone));
            if !better {
                continue;
            }
            let may_shoot = match d.owner {
                Owner::Player(seat) => seats
                    .get(seat as usize)
                    .copied()
                    .flatten()
                    .and_then(|e| self.world.get::<&Tank>(e).ok().map(|s| in_sight_box(s.position, me)))
                    .unwrap_or(true),
                _ => false,
            };
            let speed = d.velocity().length().max(1.0);
            let covered = Self::canopy_over(&trees, Self::hull_of(&tank)).is_some();
            let tree = if trees.is_empty() || covered { None } else { fpv_tree_spot(me, grid, &trees, t.fpv_ai_tree_px) };
            out.insert(entity, crate::ai::AirThreat { drone: d.ground, eta: dist / speed, may_shoot, covered, tree });
        }
        out
    }
}

/// The nearest nav cell within `fpv_ai_cover_px` of `me` (its centre) that
/// is open, joined to where it stands and `keeps` - hidden from the seat,
/// inside its box, out of its face - ties to the lower cell index.
fn fpv_cover_spot(me: Position, grid: &crate::pathfind::Grid, keeps: &dyn Fn(Position) -> bool) -> Option<Position> {
    let reach = (tuning().fpv_ai_cover_px / OBSTACLE_GRID_SIZE).ceil() as i32;
    nearest_cell(me, reach, grid, |p| grid.connected(me, p) && keeps(p))
}

/// The nearest tree within `reach_px` of `me` (ties to cell order) that has
/// an open cell joined to where it stands within two cells of it: that cell
/// - the one nearest the tree, then nearest `me`, then the lower cell index
/// - and the tree's centre. The cells round a tree are mostly inside the
/// nav grid's clearance, so the hull gets under the crown by driving at the
/// tree from there.
fn fpv_tree_spot(me: Position, grid: &crate::pathfind::Grid, trees: &[(Entity, Position)], reach_px: f32) -> Option<(Position, Position)> {
    let mut near: Vec<&(Entity, Position)> = trees.iter().filter(|(_, p)| p.distance_to(me) <= reach_px).collect();
    near.sort_by(|a, b| a.1.distance_to(me).total_cmp(&b.1.distance_to(me)));
    near.into_iter().find_map(|&(_, tree)| {
        let (cx, cy) = crate::map::world_to_cell(tree);
        let mut best: Option<((f32, f32, (i32, i32)), Position)> = None;
        for row in cy - 2..=cy + 2 {
            for col in cx - 2..=cx + 2 {
                if col < 0 || row < 0 {
                    continue;
                }
                let p = crate::map::cell_to_world(col, row);
                if !grid.usable(p) || !grid.connected(me, p) {
                    continue;
                }
                let key = (p.distance_to(tree), p.distance_to(me), (row, col));
                if best.is_none_or(|(k, _)| key < k) {
                    best = Some((key, p));
                }
            }
        }
        best.map(|(_, spot)| (spot, tree))
    })
}

/// The nearest usable cell centre within `reach` cells of `me` for which
/// `ok` holds; ties to the lower cell index (row, then column).
fn nearest_cell(me: Position, reach: i32, grid: &crate::pathfind::Grid, ok: impl Fn(Position) -> bool) -> Option<Position> {
    let (cx, cy) = crate::map::world_to_cell(me);
    let mut best: Option<(f32, (i32, i32), Position)> = None;
    for row in cy - reach..=cy + reach {
        for col in cx - reach..=cx + reach {
            if col < 0 || row < 0 {
                continue;
            }
            let p = crate::map::cell_to_world(col, row);
            if !grid.usable(p) || !ok(p) {
                continue;
            }
            let d = me.distance_to(p);
            if best.is_none_or(|(bd, bc, _)| (d, (row, col)) < (bd, bc)) {
                best = Some((d, (row, col), p));
            }
        }
    }
    best.map(|(_, _, p)| p)
}
