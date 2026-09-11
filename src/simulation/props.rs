//! The destructible props' rules (docs/sandbags-barrels-fences.md,
//! docs/barrel-explosion-variety.md): the one place an obstacle loses
//! health, a barrel's blast and the chain reaction it starts, fuse timers,
//! the fires an oil drum or a lit trail leaves on the ground, a fuel
//! drum's launch, and tanks ramming sandbags, fences and barrels.

use crate::tuning::tuning;
use hecs::Entity;
use rand::RngExt;
use rapier2d::prelude::RigidBodyHandle;
use sola_raylib::core::math::Vector2;

use crate::ai::Ai;
use crate::blast::{BlastFx, BlastKind, BlastShape, Lean, Scorch};
use crate::decal::Decal;
use crate::frog::Frog;
use crate::map::cell_to_world;
use crate::obstacle::{face_toward, Drum, Fuse, Material, Obstacle};
use crate::shockwave::Shockwave;
use crate::tank::Tank;
use crate::{MAX_DAMAGE, OBSTACLE_GRID_SIZE, Position, RUBBLE_ROW_BARREL};

use super::combat::{explosion_hit, BlastParams};
use super::{Event, Frame, Game, SHOCK_BARREL, SHOCK_FROG, SHOCK_FUEL};

/// Everything `obstacle_died` needs to know about a tile that just died.
/// Bundled rather than passed loose because each death path has the whole
/// `&mut Obstacle` in hand and can fill it in one go, and because a
/// death now carries more than a position: `charred` picks which rubble
/// a burnt-out plank leaves, `shape` says what a barrel's blast looks
/// like.
pub(super) struct DeadTile {
    pub material: Material,
    pub variant: i32,
    pub position: Position,
    /// Another blast (or a fire) set this one off, rather than a shot or
    /// a ram.
    pub chained: bool,
    /// Wood only: it burnt out rather than snapping, so it leaves the
    /// charred cell instead of the destroyed one.
    pub charred: bool,
    /// Barrel only: what its blast looks like.
    pub shape: BlastShape,
}

/// What is damaging an obstacle - the fence and barrel rules differ by it.
/// `Blast` carries the linear falloff at the victim (1 at the centre, 0 at
/// the radius), which sets how long a chained barrel's fuse burns, and
/// where the blast was, which a chained drum leans away from.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum DamageCause {
    /// A projectile, travelling along `dir` (unit) when known.
    Shot { dir: Option<Vector2> },
    Ram,
    Blast { falloff: f32, from: Position },
}

/// A barrel detonation waiting for `explosions` to resolve it this frame.
#[derive(Clone, Copy, Debug)]
pub(super) struct PendingBlast {
    pub center: Position,
    pub drum: Drum,
    pub shape: BlastShape,
}

/// A cell of ground on fire: an oil drum's pool or a lit trail cell.
/// Simulation state, not a decal - it hurts what drives over it, lights
/// what stands beside it and blocks the nav grid while it burns.
#[derive(Clone, Copy, Debug)]
pub struct GroundFire {
    pub cell: (i32, i32),
    /// Seconds of burning left.
    pub left: f32,
    /// How long it burns in total (for the presentation's fade).
    pub total: f32,
    /// Round time at which this fire spreads to its oil-trail neighbours,
    /// `None` once it has. One step per cell keeps a lit trail a
    /// deterministic wave with no RNG.
    pub spread_at: Option<f32>,
    /// From a drum's pool rather than a painted trail cell.
    pub pool: bool,
}

impl GroundFire {
    pub fn position(&self) -> Position {
        cell_to_world(self.cell.0, self.cell.1)
    }
}

/// A fuel drum in the air: launched by another blast's fuse, flying to
/// the landing cell the simulation chose, and detonating there when it
/// lands. The flight is cosmetic (`flight`/`height`/`draw_pos` mirror
/// `decal::Decal`'s arc) and cannot move the landing spot.
#[derive(Clone, Copy, Debug)]
pub struct FlyingDrum {
    pub from: Position,
    pub to: Position,
    pub age: f32,
    pub variant: i32,
}

impl FlyingDrum {
    pub fn flight(&self) -> f32 {
        (self.age / tuning().debris_flight_seconds.max(1e-3)).clamp(0.0, 1.0)
    }

    pub fn height(&self) -> f32 {
        let t = self.flight();
        tuning().debris_arc_height * 1.4 * 4.0 * t * (1.0 - t)
    }

    /// The point on the ground it is over right now.
    pub fn ground_pos(&self) -> Position {
        let t = self.flight();
        Position::new(self.from.x + (self.to.x - self.from.x) * t, self.from.y + (self.to.y - self.from.y) * t)
    }

    pub fn draw_pos(&self) -> Position {
        let g = self.ground_pos();
        Position::new(g.x, g.y - self.height())
    }

    pub fn landed(&self) -> bool {
        self.flight() >= 1.0
    }
}

/// The four orthogonal neighbours of a grid cell.
fn neighbours(cell: (i32, i32)) -> [(i32, i32); 4] {
    let (c, r) = cell;
    [(c + 1, r), (c - 1, r), (c, r + 1), (c, r - 1)]
}

/// The grid cell a world position sits in (positions are cell-aligned,
/// so this is the nearest centre).
fn cell_of(pos: Position) -> (i32, i32) {
    ((pos.x / OBSTACLE_GRID_SIZE).round() as i32, (pos.y / OBSTACLE_GRID_SIZE).round() as i32)
}

impl Game {
    /// Apply `amount` of damage to the obstacle `entity` - the only place on
    /// the simulation path an obstacle loses health, so the per-material
    /// rules live here once: a fence is amount-blind (a pristine one dies
    /// outright at `fence_one_shot_chance` odds, else drops to its damaged
    /// stage; a damaged one always dies); a barrel caught in a blast is
    /// always lethal but goes on a fuse - `barrel_fuse_seconds` scaled by
    /// distance, half of it at the blast's centre up to two and a half
    /// times at its edge, then by its drum's own factor (oil smoulders,
    /// fuel cracks first) - so a cluster cascades outward rather than
    /// going off all at once, while a direct hit or a ram pops it right
    /// away through the plain health path; a tree pushed over by a tank
    /// dies outright instead of taking its flammable fork, since a hull is
    /// not something that sets a tree alight; everything else is
    /// `Obstacle::damage`. Returns `true` the frame the obstacle dies.
    pub(super) fn damage_obstacle(&mut self, f: &mut Frame, entity: Entity, amount: f32, cause: DamageCause) -> bool {
        let (material, variant, pos, died) = {
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
                    DamageCause::Blast { falloff, from } => {
                        arm_fuse(o, 0.5 + 2.0 * (1.0 - falloff.clamp(0.0, 1.0)), from);
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
            (o.material, o.variant, o.position, died)
        };
        if died {
            let shape = match cause {
                DamageCause::Shot { dir: Some(d) } => BlastShape::Shot { dir: Lean { x: d.x, y: d.y } },
                DamageCause::Shot { dir: None } => BlastShape::Plain,
                DamageCause::Ram => BlastShape::Ram,
                DamageCause::Blast { .. } => BlastShape::Plain,
            };
            self.obstacle_died(f, DeadTile { material, variant, position: pos, chained: false, charred: false, shape });
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
        let DeadTile { material, variant, position: pos, chained, charred, shape } = tile;
        f.events.push(Event::ObstacleDestroyed { material, x: pos.x, y: pos.y });
        // A barrel's leftovers are thrown by its blast (`apply_blast`);
        // everything else with a rubble row drops it in place. Iron has
        // none and never dies anyway.
        if !material.is_explosive() {
            if let Some(decal) = Decal::new(material, pos, charred) {
                f.decals.push(decal);
            }
        }
        if material.is_explosive() {
            let drum = Drum::from_variant(variant);
            f.events.push(Event::Blast { x: pos.x, y: pos.y, chained, drum });
            f.pending_blasts.push(PendingBlast { center: pos, drum, shape });
        }
    }

    /// A barrel's detonation: everyone inside the drum's blast radius
    /// takes linear-falloff damage and a shove - player and enemies alike,
    /// both frogs, every tile - and any other barrel in range is put on a
    /// fuse. `live` is false on the end screen, where a cascade finishes
    /// visually without dealing damage.
    ///
    /// Beyond the damage, what the blast leaves depends on the drum and on
    /// what set it off: an oil drum's pool of fire, a fuel drum's harder
    /// shake and bigger scorch, a ram's extra lurch, a shot's streak. The
    /// visuals: the full-screen ripple and camera shake (`f.shock`), the
    /// impact-flash quad, the fireball animation, a scorch mark, thrown
    /// drum parts, delayed cook-off pops, flattened grass, burnt-in tread
    /// marks, and any rubble already lying inside the radius thrown again.
    /// Every cosmetic choice hashes the position; the only RNG drawn is
    /// the damage rolls, exactly as before.
    pub(super) fn apply_blast(&mut self, f: &mut Frame, blast: PendingBlast, live: bool) {
        let PendingBlast { center, drum, shape } = blast;
        let t = tuning();
        let mut params = match drum {
            Drum::Oil => BlastParams::barrel(),
            Drum::Fuel => BlastParams::fuel(),
        };
        if shape == BlastShape::Ram {
            // The hull is on top of the drum: it lurches.
            params.knockback *= t.barrel_ram_kick_factor;
        }
        if live {
            // Players first, in index order, then the enemies - the same
            // draw order as a wreck's blast (`Game::apply_explosion`).
            for player in self.players().into_iter().flatten() {
                let mut q = self.world.query_one::<&mut Tank>(player);
                let tank = q.get().expect("player entity always has a Tank");
                explosion_hit(tank, center, true, &mut self.physics, &mut f.rng, &mut f.kills, &params);
            }
            for tank in self.world.query::<&mut Tank>().with::<&Ai>().iter() {
                explosion_hit(tank, center, true, &mut self.physics, &mut f.rng, &mut f.kills, &params);
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
                .query::<(Entity, &mut Obstacle)>()
                .iter()
                .filter(|(_, o)| !o.destroyed && o.fuse.is_none())
                .filter_map(|(e, o)| {
                    let dist = o.position.distance_to(center);
                    if dist > params.radius {
                        return None;
                    }
                    // Soot on whichever face looked at the blast; a
                    // wall's business only, and cosmetic.
                    if o.material.is_wall() {
                        o.scorched |= face_toward(o.position, center);
                    }
                    Some((e, o.material, 1.0 - dist / params.radius))
                })
                .collect();
            for (entity, material, falloff) in hits {
                // Barrels inside always chain - no roll needed.
                let amount = if material.is_explosive() { f32::MAX } else { params.roll_damage(&mut f.rng) * falloff };
                self.damage_obstacle(f, entity, amount, DamageCause::Blast { falloff, from: center });
            }
            // An oil drum leaves a pool of fire; either drum lights any
            // trail cell in range.
            if drum == Drum::Oil && t.oil_pool_radius_cells > 0 {
                let pool = t.oil_pool_chance;
                let leaves = pool >= 1.0 || (pool > 0.0 && f.rng.random_bool(pool));
                if leaves {
                    let (c, r) = cell_of(center);
                    let reach = t.oil_pool_radius_cells;
                    let mut cells = vec![(c, r)];
                    for dc in -reach..=reach {
                        for dr in -reach..=reach {
                            // A plus shape at 1, a diamond at more - fire
                            // spreads out along the ground, not into corners.
                            if (dc, dr) != (0, 0) && dc.abs() + dr.abs() <= reach {
                                cells.push((c + dc, r + dr));
                            }
                        }
                    }
                    for cell in cells {
                        self.light_cell(f, cell, t.oil_pool_seconds, true);
                    }
                }
            }
            let lit: Vec<(i32, i32)> = self
                .oil_cells
                .iter()
                .copied()
                .filter(|&cell| cell_to_world(cell.0, cell.1).distance_to(center) <= params.radius)
                .collect();
            for cell in lit {
                self.light_cell(f, cell, t.oil_trail_burn_seconds, false);
            }
        }

        // --- the show ---
        let kind = match drum {
            Drum::Oil => BlastKind::Oil,
            Drum::Fuel => BlastKind::Fuel,
        };
        f.shocks.push(Shockwave::scaled(center, if drum == Drum::Fuel { SHOCK_FUEL } else { SHOCK_BARREL }));
        f.impact_flashes.push(Shockwave::new(center));
        f.blast_fx.push(BlastFx::shaped(center, kind, shape));
        self.flash_screen();
        let streak = match shape {
            BlastShape::Shot { dir } => Some(dir),
            _ => None,
        };
        let scorch_scale = if drum == Drum::Fuel { t.scorch_fuel_scale } else { 1.0 };
        f.scorches.push(Scorch::with(center, scorch_scale, streak));
        self.scorch_tracks(center);
        crate::grass::flatten(&mut self.grass, center, params.radius * t.blast_grass_flatten);

        // Drum parts thrown in arcs to hashed landing spots (the same
        // machinery a wreck's hull parts use); with none, the plain
        // rubble decal in place.
        let parts = t.barrel_parts.max(0) as u32;
        if parts == 0 {
            if let Some(decal) = Decal::new(Material::Barrel, center, false) {
                f.decals.push(decal);
            }
        }
        for i in 0..parts {
            let h = crate::blast::seed_at(center, 40 + i * 5);
            let angle = (h % 3600) as f32 / 3600.0 * std::f32::consts::TAU;
            let dist = t.barrel_part_throw_px * (0.35 + 0.65 * ((h >> 12) % 100) as f32 / 100.0);
            let to = Position::new(center.x + angle.cos() * dist, center.y + angle.sin() * dist);
            f.decals.push(Decal::thrown(RUBBLE_ROW_BARREL, center, to, 40 + i * 5));
        }

        // Rubble already on the ground inside the blast is picked up and
        // thrown again, so a second blast in the same place visibly moves
        // the scene rather than adding another identical mark. Landing
        // spots are hashed from where each piece lay, so nothing here can
        // move a replay.
        let mut rethrown = 0;
        let cap = t.blast_rethrow_max.max(0);
        for decal in self.decals.iter_mut() {
            if rethrown >= cap {
                break;
            }
            if !decal.landed() {
                continue;
            }
            let dist = decal.center.distance_to(center);
            if dist > params.radius * 0.7 {
                continue;
            }
            rethrown += 1;
            let h = crate::blast::seed_at(decal.center, 60);
            let mut angle = (decal.center.y - center.y).atan2(decal.center.x - center.x);
            angle += ((h % 100) as f32 / 100.0 - 0.5) * 0.8;
            let throw = 16.0 + ((h >> 8) % 24) as f32;
            let to = Position::new(
                (center.x + angle.cos() * (dist + throw)).clamp(0.0, f.width),
                (center.y + angle.sin() * (dist + throw)).clamp(0.0, f.height),
            );
            decal.rethrow(to);
        }

        // Delayed secondaries, hashed 0..=max per blast so a third of
        // drums get none; the same queue a wreck's cook-offs use.
        let max = t.barrel_cookoff_max.max(0) as u32;
        if max > 0 {
            let count = crate::blast::seed_at(center, 90) % (max + 1);
            let window = t.cookoff_window_seconds;
            for i in 0..count {
                let h = crate::blast::seed_at(center, 90 + i * 7);
                let delay = window * (0.15 + 0.85 * (h % 1000) as f32 / 1000.0);
                let off = ((h >> 10) % 21) as f32 - 10.0;
                let off2 = ((h >> 16) % 21) as f32 - 10.0;
                self.cookoffs.push((Position::new(center.x + off, center.y + off2), delay));
            }
        }
    }

    /// Set a ground cell on fire for `seconds`, unless it already is. A
    /// pool cell burns where the drum stood; a trail cell also carries the
    /// fire on to its oil neighbours one step later.
    fn light_cell(&mut self, f: &mut Frame, cell: (i32, i32), seconds: f32, pool: bool) {
        if self.fires.iter().any(|fire| fire.cell == cell) {
            return;
        }
        let pos = cell_to_world(cell.0, cell.1);
        if pos.x < 0.0 || pos.y < 0.0 || pos.x > f.width || pos.y > f.height {
            return;
        }
        let step = 1.0 / tuning().oil_trail_cells_per_second.max(0.5);
        self.fires.push(GroundFire { cell, left: seconds, total: seconds, spread_at: Some(self.time + step), pool });
        f.events.push(Event::FireStarted { x: pos.x, y: pos.y, pool });
    }

    /// Advance every burning ground cell: burn down, spread along oil
    /// trails, hurt the tanks standing in it, light the flammable tiles
    /// and drums beside it, and when it goes out leave the ground darker.
    /// `live` is false on the end screen, where the fire only burns.
    pub(super) fn tick_fires(&mut self, f: &mut Frame, live: bool) {
        if self.fires.is_empty() {
            return;
        }
        let t = tuning();
        let now = self.time;
        // Spread first, so a trail lit this frame does not also spread
        // this frame.
        let mut spread = Vec::new();
        for fire in self.fires.iter_mut() {
            fire.left -= f.dt;
            if let Some(at) = fire.spread_at {
                if now >= at {
                    fire.spread_at = None;
                    for n in neighbours(fire.cell) {
                        if self.oil_cells.contains(&n) {
                            spread.push(n);
                        }
                    }
                }
            }
        }
        for cell in spread {
            self.light_cell(f, cell, t.oil_trail_burn_seconds, false);
        }

        let burning: Vec<(i32, i32)> = self.fires.iter().filter(|fire| fire.left > 0.0).map(|fire| fire.cell).collect();
        if live {
            // A hull over a burning cell burns: players first, in index
            // order, then enemies, like every other blast walk - no RNG,
            // but the kill order is part of the record.
            let dps = t.oil_pool_damage_per_second;
            if dps > 0.0 {
                let half_cell = OBSTACLE_GRID_SIZE * 0.5;
                let mut order: Vec<Entity> = self.players().into_iter().flatten().collect();
                order.extend(self.world.query::<(Entity, &Tank)>().with::<&Ai>().iter().map(|(e, _)| e));
                for entity in order {
                    let mut q = self.world.query_one::<&mut Tank>(entity);
                    let Ok(tank) = q.get() else { continue };
                    if tank.is_wreck() {
                        continue;
                    }
                    let reach = tank.hull_size() * 0.5 + half_cell;
                    let over = burning.iter().any(|&(c, r)| {
                        let p = cell_to_world(c, r);
                        (p.x - tank.position.x).abs() < reach && (p.y - tank.position.y).abs() < reach
                    });
                    if !over {
                        continue;
                    }
                    tank.take_damage(dps * f.dt, MAX_DAMAGE);
                    tank.mark_hit();
                    if tank.is_wreck() {
                        f.kills.push((tank.position, tank.owner()));
                    }
                }
            }
            // Flammable tiles beside the fire catch; drums sitting in it
            // are put on a long fuse.
            let mut fused = Vec::new();
            for o in self.world.query::<&mut Obstacle>().iter() {
                if o.destroyed || o.burning || o.fuse.is_some() {
                    continue;
                }
                let cell = o.cell();
                let Some(&source) =
                    burning.iter().find(|&&b| b == cell || neighbours(b).contains(&cell))
                else {
                    continue;
                };
                if o.material.is_explosive() {
                    fused.push(o.position);
                    arm_fuse(o, t.fire_fuse_factor, cell_to_world(source.0, source.1));
                } else if o.flammable {
                    o.health = 0.0;
                    o.burning = true;
                }
            }
            for _ in fused {
                // Fusing is silent: the drum's own lit column and glow are
                // the tell.
            }
        }

        // Burnt out: the ground stays darker, a trail cell is used up.
        let mut done = Vec::new();
        self.fires.retain(|fire| {
            if fire.left > 0.0 {
                return true;
            }
            done.push(*fire);
            false
        });
        for fire in done {
            let pos = fire.position();
            self.ground.darken_cell(pos, t.ground_burn_darken);
            self.oil_cells.remove(&fire.cell);
            if let Some(decal) = Decal::new(Material::Wood, pos, true) {
                f.decals.push(decal);
            }
        }
    }

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
                charred.push((o.material, o.variant, o.position));
            }
        }
        for (material, variant, position) in charred {
            self.obstacle_died(
                f,
                DeadTile { material, variant, position, chained: false, charred: true, shape: BlastShape::Plain },
            );
        }
    }

    /// Count every lit fuse down; a barrel whose fuse runs out dies and
    /// queues its own blast (`chained`), which `explosions` resolves this
    /// same frame so the cascade keeps going. A fuel drum lit by another
    /// blast does not pop in place: it launches (`launch_drum`), and its
    /// blast lands where it does.
    pub(super) fn tick_fuses(&mut self, f: &mut Frame) {
        let mut popped = Vec::new();
        for (entity, o) in self.world.query::<(Entity, &mut Obstacle)>().iter() {
            let Some(fuse) = o.fuse.as_mut() else { continue };
            fuse.left -= f.dt;
            if fuse.left <= 0.0 && !o.destroyed {
                popped.push((entity, o.material, o.variant, o.position, *fuse));
            }
        }
        for (entity, material, variant, position, fuse) in popped {
            let drum = Drum::from_variant(variant);
            if material.is_explosive() && drum == Drum::Fuel {
                if let Some(from) = fuse.from {
                    if let Some(to) = self.launch_target(f, position, from) {
                        self.launch_drum(f, entity, position, to, variant);
                        continue;
                    }
                }
            }
            {
                let mut q = self.world.query_one::<&mut Obstacle>(entity);
                if let Ok(o) = q.get() {
                    o.destroyed = true;
                }
            }
            let shape = match fuse.from {
                Some(from) => BlastShape::Chained { from, smoulder: fuse.smoulder() },
                None => BlastShape::Fire,
            };
            self.obstacle_died(f, DeadTile { material, variant, position, chained: true, charred: false, shape });
        }
    }

    /// Where a launched fuel drum lands: `fuel_launch_cells_min..=max`
    /// cells (hashed) away from `from`, jittered a little off the line,
    /// snapped to the grid and kept inside the field; a landing cell a
    /// solid tile occupies is walked back toward the drum one cell at a
    /// time. `None` when no launch should happen (the knob is off, the
    /// roll fails, or every cell on the way is blocked), in which case
    /// the drum pops in place like an oil drum.
    fn launch_target(&mut self, f: &mut Frame, at: Position, from: Position) -> Option<Position> {
        let t = tuning();
        let chance = t.fuel_launch_chance;
        if chance <= 0.0 {
            return None;
        }
        if chance < 1.0 && !f.rng.random_bool(chance) {
            return None;
        }
        let h = crate::blast::seed_at(at, 77);
        let dx = at.x - from.x;
        let dy = at.y - from.y;
        let d = (dx * dx + dy * dy).sqrt();
        let base = if d > 0.001 { dy.atan2(dx) } else { (h % 360) as f32 / 360.0 * std::f32::consts::TAU };
        let angle = base + ((h % 100) as f32 / 100.0 - 0.5) * 0.9;
        let (lo, hi) = (t.fuel_launch_cells_min.max(1), t.fuel_launch_cells_max.max(t.fuel_launch_cells_min).max(1));
        let cells = lo + ((h >> 8) % (hi - lo + 1) as u32) as i32;
        let solid: Vec<(i32, i32)> = self
            .world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| !o.destroyed)
            .map(|o| o.cell())
            .collect();
        let start = cell_of(at);
        for n in (1..=cells).rev() {
            let target = Position::new(at.x + angle.cos() * n as f32 * OBSTACLE_GRID_SIZE, at.y + angle.sin() * n as f32 * OBSTACLE_GRID_SIZE);
            let cell = cell_of(target);
            let pos = cell_to_world(cell.0, cell.1);
            let inside = pos.x >= OBSTACLE_GRID_SIZE
                && pos.y >= OBSTACLE_GRID_SIZE
                && pos.x <= f.width - OBSTACLE_GRID_SIZE
                && pos.y <= f.height - OBSTACLE_GRID_SIZE;
            if inside && cell != start && !solid.contains(&cell) {
                return Some(pos);
            }
        }
        None
    }

    /// Send a fuel drum flying: the obstacle goes at once (its body with
    /// it, through the ordinary destroyed path but with no blast), and a
    /// `FlyingDrum` carries the show to the landing spot, where
    /// `tick_launches` sets the real blast off.
    fn launch_drum(&mut self, f: &mut Frame, entity: Entity, from: Position, to: Position, variant: i32) {
        {
            let mut q = self.world.query_one::<&mut Obstacle>(entity);
            if let Ok(o) = q.get() {
                o.destroyed = true;
            }
        }
        f.events.push(Event::ObstacleDestroyed { material: Material::Barrel, x: from.x, y: from.y });
        f.events.push(Event::DrumLaunched { x: from.x, y: from.y, to_x: to.x, to_y: to.y });
        self.flying_drums.push(FlyingDrum { from, to, age: 0.0, variant });
    }

    /// Age the drums in the air; one that lands detonates there - a
    /// chained blast leaning away from where it was launched.
    pub(super) fn tick_launches(&mut self, f: &mut Frame) {
        let mut landed = Vec::new();
        for drum in self.flying_drums.iter_mut() {
            drum.age += f.dt;
            if drum.landed() {
                landed.push(*drum);
            }
        }
        self.flying_drums.retain(|d| !d.landed());
        for drum in landed {
            let kind = Drum::from_variant(drum.variant);
            f.events.push(Event::Blast { x: drum.to.x, y: drum.to.y, chained: true, drum: kind });
            f.pending_blasts.push(PendingBlast {
                center: drum.to,
                drum: kind,
                shape: BlastShape::Chained { from: drum.from, smoulder: 1.0 },
            });
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

    /// Every burning ground cell, with how long it has left and its full
    /// burn - what the renderer and the particle layer sample.
    pub fn burning_cells(&self) -> Vec<(Position, f32, f32)> {
        self.fires.iter().map(|fire| (fire.position(), fire.left, fire.total)).collect()
    }

    /// Every barrel with a lit fuse, for the spark tell.
    pub fn fused_barrels(&self) -> Vec<Position> {
        self.world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| o.fuse.is_some() && !o.destroyed)
            .map(|o| o.position)
            .collect()
    }
}

/// Light a barrel's fuse: `barrel_fuse_seconds` times the caller's factor
/// (distance in a blast, `fire_fuse_factor` from a fire) times the drum's
/// own (`Drum::fuse_factor`). Zeroes its health so it draws as critical
/// under the lit column, and remembers what lit it.
fn arm_fuse(o: &mut Obstacle, factor: f32, from: Position) {
    let drum = Drum::from_variant(o.variant);
    let total = tuning().barrel_fuse_seconds * factor * drum.fuse_factor();
    o.health = 0.0;
    o.fuse = Some(Fuse { left: total, total, from: Some(from) });
}
