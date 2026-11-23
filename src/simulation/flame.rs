//! The flamethrower's stream (docs/flamethrower-prd.md): what one frame
//! of a held trigger does to the tanks, the bongs and the ground and
//! tiles inside its cone, plus the afterburn it leaves on a hull.
//!
//! Kept out of `combat.rs` and `props.rs` because it is the one weapon
//! that damages by *rate* rather than by hit: nothing here rolls a
//! damage range, so **the whole phase draws no RNG** - a round with no
//! flamethrower pickup replays exactly as it did before the weapon
//! existed, and one with it replays exactly however the player waved the
//! nozzle.
//!
//! Everything in the world reacts through one mechanism, **exposure**:
//! ground cells (`Game::heat`) and tiles (`Obstacle::heat`) accumulate
//! seconds under the stream and lose them at `flame_heat_decay` when it
//! moves on, and each thing catches at its own threshold. A quick sweep
//! therefore soots without lighting; holding on a spot sets it alight.
//! What a lit cell then does - hurt what drives over it, spread along an
//! oil trail, light the tiles beside it, char the grass in it - is the
//! ground-fire machinery in `props.rs`, reused untouched.

use crate::math;
use hecs::Entity;
use crate::math::Vec2;

use super::combat::Hurt;
use super::hits::Terrain;
use super::props::{arm_fuse, DamageCause};
use super::weapons::FlameJet;
use super::{Event, Frame, Game, HitTarget};
use crate::bong::Bong;
use crate::map::cell_to_world;
use crate::obstacle::{face_toward, Obstacle};
use crate::shockwave::Shockwave;
use crate::tank::Tank;
use crate::tuning::tuning;
use crate::{Position, OBSTACLE_GRID_SIZE};

/// The stream starts sampling the ground this far past the muzzle, so a
/// standing tank never lights the cell it is sitting in.
const SAMPLE_START_PX: f32 = OBSTACLE_GRID_SIZE / 4.0;
/// Spacing of the exposure samples along the centre line - two per cell,
/// so no cell the line crosses is skipped.
const SAMPLE_STEP_PX: f32 = OBSTACLE_GRID_SIZE / 2.0;
/// The cone is cast as rays from the nozzle, one at each of these
/// fractions of its half angle: each is stopped on its own by the first
/// tile it meets and then by the first hull, so a flank does not burn
/// round a wall's corner and nothing burns behind a tank.
const RAYS: [f32; 5] = [-1.0, -0.5, 0.0, 0.5, 1.0];
/// The ray down the cone's axis.
const CENTRE: usize = 2;

/// One jet's cone this frame, each of its rays capped by what it met.
struct Cone {
    origin: Position,
    dir: Vec2,
    /// The centre ray's reach to the first tile, or `flame_range`: what
    /// the jet is drawn to.
    reach: f32,
    /// tan of the half angle: half width at distance `d` is `d * spread`.
    spread: f32,
    /// How far along the axis each of `RAYS` gets, px.
    rays: [f32; RAYS.len()],
}

impl Cone {
    /// The cone of `jet`, each ray capped at the first tile a shot would
    /// meet on it (`Terrain::first_shot_along`: every tile, a chunked one
    /// by its standing chunks).
    fn new(jet: &FlameJet, terrain: &Terrain) -> Self {
        let spread = math::tan(tuning().flame_half_angle_deg.to_radians());
        let mut cone = Cone { origin: jet.origin, dir: jet.dir, reach: jet.range, spread, rays: [jet.range; RAYS.len()] };
        for i in 0..RAYS.len() {
            let end = cone.ray_end(i, jet.range);
            cone.rays[i] = terrain.first_shot_along(jet.origin, end, 0.0).map_or(jet.range, |t| t * jet.range);
        }
        cone.reach = cone.rays[CENTRE];
        cone
    }

    /// The point `along` px down the axis on ray `i`.
    fn ray_end(&self, i: usize, along: f32) -> Position {
        let side = along * self.spread * RAYS[i];
        Position::new(self.origin.x + self.dir.x * along - self.dir.y * side, self.origin.y + self.dir.y * along + self.dir.x * side)
    }

    /// The ray nearest a point `along` px down the axis and `side` px off
    /// it (signed, toward `ray_end`'s positive side).
    fn ray_near(&self, along: f32, side: f32) -> usize {
        let half = along * self.spread;
        if half <= f32::EPSILON {
            return CENTRE;
        }
        let k = (side / half).clamp(-1.0, 1.0);
        (((k + 1.0) * 0.5) * (RAYS.len() - 1) as f32).round() as usize
    }

    /// Is `p` in the stream, widened by `pad` on every side: inside the
    /// cone and no further out than the ray nearest it gets?
    fn contains(&self, p: Position, pad: f32) -> bool {
        let dx = p.x - self.origin.x;
        let dy = p.y - self.origin.y;
        let along = dx * self.dir.x + dy * self.dir.y;
        let side = dy * self.dir.x - dx * self.dir.y;
        along >= -pad && side.abs() <= along.max(0.0) * self.spread + pad && along <= self.rays[self.ray_near(along.max(0.0), side)] + pad
    }

    /// The ground cells the stream heats this frame: the centre ray
    /// sampled from a quarter cell past the muzzle to its reach, and the
    /// two edge rays across the cone's outer half, each to its own.
    /// Sorted and unique.
    fn cells(&self) -> Vec<(i32, i32)> {
        let mut cells = Vec::new();
        let mut along = SAMPLE_START_PX;
        let far = self.rays.iter().copied().fold(0.0, f32::max);
        while along <= far {
            if along <= self.rays[CENTRE] {
                cells.push(cell_of(self.ray_end(CENTRE, along)));
            }
            if along >= self.reach * 0.5 {
                for edge in [0, RAYS.len() - 1] {
                    if along <= self.rays[edge] {
                        cells.push(cell_of(self.ray_end(edge, along)));
                    }
                }
            }
            along += SAMPLE_STEP_PX;
        }
        cells.sort_unstable();
        cells.dedup();
        cells
    }
}

/// The grid cell a world position sits in.
fn cell_of(p: Position) -> (i32, i32) {
    ((p.x / OBSTACLE_GRID_SIZE).round() as i32, (p.y / OBSTACLE_GRID_SIZE).round() as i32)
}

impl Game {
    /// The flamethrower phase: resolve this frame's jets against tanks,
    /// bongs, ground and tiles; tick the afterburn on every burning hull;
    /// cool everything the stream is not on. Runs only while the round
    /// is live - on the end screen nothing fires and nothing burns down.
    pub(super) fn resolve_flames(&mut self, f: &mut Frame) {
        let jets = std::mem::take(&mut f.flame_jets);
        let t = tuning();
        let decay = t.flame_heat_decay.max(0.0) * f.dt;

        let mut cones: Vec<(Cone, &FlameJet)> = Vec::with_capacity(jets.len());
        let mut kept = Vec::with_capacity(jets.len());
        for jet in &jets {
            let cone = Cone::new(jet, &f.terrain);
            kept.push(FlameJet { reach: cone.reach, ..*jet });
            cones.push((cone, jet));
        }

        // --- Tanks and bongs in a cone. ---
        let mut touched: Vec<Entity> = Vec::new();
        for (cone, jet) in &mut cones {
            // By owner slot: the seats, then the enemies. A wreck is no
            // more in a stream's way than in a shell's.
            let mut hulls: Vec<(usize, Entity, crate::shape::TargetShape)> = self
                .world
                .query::<(Entity, &Tank)>()
                .iter()
                .filter(|(e, tank)| *e != jet.shooter && !tank.is_wreck() && tank.body.is_some())
                .map(|(e, tank)| (tank.owner_slot(), e, tank.target_shape()))
                .collect();
            hulls.sort_unstable_by_key(|&(slot, ..)| slot);
            let bongs: Vec<(Entity, Position)> = super::ordered(self.world.query::<(Entity, &Bong)>().iter().filter(|(_, b)| !b.is_dead()), |(_, b)| super::bong_order(b))
                .into_iter()
                .map(|(e, b)| (e, b.position))
                .collect();
            // Each ray burns the first hull or bong it meets (`TargetShape::
            // swept`, the boxes a shot is tested against) and stops there.
            let mut reached: Vec<Entity> = Vec::new();
            let bong_half = Position::new(crate::BONG_COLLIDER_HALF_EXTENT.0, crate::BONG_COLLIDER_HALF_EXTENT.1);
            for i in 0..RAYS.len() {
                let end = cone.ray_end(i, cone.rays[i]);
                let mut first: Option<(f32, Entity)> = None;
                let entries = hulls
                    .iter()
                    .map(|&(_, e, shape)| (e, shape.swept(cone.origin, end, Vec2::zero())))
                    .chain(bongs.iter().map(|&(e, at)| (e, super::hits::segment_hits_aabb(cone.origin, end, at, bong_half))));
                for (e, entry) in entries {
                    if let Some(t) = entry
                        && first.is_none_or(|(best, _)| t < best)
                    {
                        first = Some((t, e));
                    }
                }
                if let Some((t, e)) = first {
                    cone.rays[i] *= t;
                    reached.push(e);
                }
            }
            let targets: Vec<Entity> = hulls.iter().map(|&(_, e, _)| e).filter(|e| reached.contains(e)).collect();
            for entity in targets {
                let Some((at, seat, takes_heat)) = self.world.get::<&Tank>(entity).ok().map(|tank| (tank.position, tank.owner().is_player(), tank.takes_heat())) else { continue };
                let mut d = t.flame_damage_per_second * f.dt;
                if jet.owner.is_player() && seat {
                    d *= t.friendly_fire_damage_factor;
                }
                // A heat shield turns the stream: no burn, no afterburn -
                // the contact is still reported.
                self.hurt_tank(f, entity, Hurt::contact(if takes_heat { d } else { 0.0 }, super::HitCause::Flame, Some(jet.owner), at));
                if takes_heat {
                    super::with_tank_mut(&self.world, entity, |tank| tank.burn_timer = t.flame_afterburn_seconds);
                }
                touched.push(entity);
            }
            if t.flame_bong_damage_per_second > 0.0 {
                let mut dead_at = Vec::new();
                for (entity, bong) in super::ordered(self.world.query::<(Entity, &mut Bong)>().iter(), |(_, b)| super::bong_order(b)) {
                    if bong.is_dead() || !reached.contains(&entity) {
                        continue;
                    }
                    bong.hurt_by(t.flame_bong_damage_per_second * f.dt, Some(jet.owner));
                    let entered = !self.flame_contacts.contains(&entity);
                    touched.push(entity);
                    if entered || bong.is_dead() {
                        let target = HitTarget::Bong { side: bong.side };
                        f.events.push(Event::Hit { target, damage: 0.0, killed: bong.is_dead(), x: bong.position.x, y: bong.position.y, cause: super::HitCause::Flame, by: Some(jet.owner.slot()) });
                    }
                    if bong.is_dead() {
                        dead_at.push(bong.position);
                    }
                }
                for pos in dead_at {
                    f.shocks.push(Shockwave::scaled(pos, crate::tuning::tuning().bong_shock));
                }
            }
        }
        touched.sort_unstable();
        touched.dedup();

        // Afterburn: a hull the stream is *not* on right now keeps
        // burning at the lower rate - contact already charges the full
        // rate, so the two never stack. Players in index order, then the
        // enemies, like every other damage walk.
        if t.flame_afterburn_dps > 0.0 {
            let mut order: Vec<Entity> = self.players().into_iter().flatten().collect();
            order.extend(self.enemies_by_slot());
            for entity in order {
                if touched.contains(&entity) {
                    continue;
                }
                let Some((at, takes_heat)) = self.world.get::<&Tank>(entity).ok().filter(|tank| !tank.is_wreck() && tank.burn_timer > 0.0).map(|tank| (tank.position, tank.takes_heat())) else { continue };
                // Wading puts the fire out (docs/water.md); ice does not.
                // A heat shield puts it out too.
                if self.water.depth_at(at).is_wet() || !takes_heat {
                    super::with_tank_mut(&self.world, entity, |tank| tank.burn_timer = 0.0);
                    continue;
                }
                // In the lava itself the lava charges the full rate
                // (`Game::lava_phase`); the afterburn is for after.
                if self.lava.depth_at(at) != crate::ground::Depth::Dry {
                    continue;
                }
                self.hurt_tank(f, entity, Hurt::contact(t.flame_afterburn_dps * f.dt, super::HitCause::Fire, None, at));
            }
        }
        self.flame_contacts = touched;

        // --- Ground exposure. ---
        // A cell the shooter's own hull is over or touching never heats:
        // the same box `tick_fires` burns a hull from, so a standing tank
        // cannot light the ground it would be burnt by. Everything past
        // that is fair game - driving forward into the strip you laid is
        // the weapon's cost.
        let mut heated: Vec<(i32, i32)> = Vec::new();
        for (cone, jet) in &cones {
            let (at, reach) = super::with_tank(&self.world, jet.shooter, |t| (t.position, t.hull_size() * 0.5 + OBSTACLE_GRID_SIZE * 0.5));
            heated.extend(cone.cells().into_iter().filter(|&(c, r)| {
                let p = cell_to_world(c, r);
                (p.x - at.x).abs() >= reach || (p.y - at.y).abs() >= reach
            }));
        }
        heated.sort_unstable();
        heated.dedup();
        // A cell under a standing tile never lights as ground: the tile
        // takes the heat instead (below).
        let solid: std::collections::HashSet<(i32, i32)> = if heated.is_empty() {
            std::collections::HashSet::new()
        } else {
            self.world.query::<&Obstacle>().iter().filter(|o| !o.destroyed).map(|o| o.cell()).collect()
        };
        for cell in &heated {
            if solid.contains(cell) {
                continue;
            }
            // Water takes no heat (docs/water.md).
            if self.water.depth_at(cell_to_world(cell.0, cell.1)) != crate::ground::Depth::Dry {
                continue;
            }
            *self.heat.entry(*cell).or_insert(0.0) += f.dt;
        }
        let mut lit = Vec::new();
        self.heat.retain(|cell, heat| {
            if !heated.contains(cell) {
                *heat -= decay;
            }
            if *heat >= t.flame_ignite_seconds {
                lit.push(*cell);
                return false;
            }
            *heat > 0.0
        });
        for cell in lit {
            if self.oil_cells.contains(&cell) {
                self.light_cell(f, cell, t.oil_trail_burn_seconds, false);
                let p = cell_to_world(cell.0, cell.1);
                f.events.push(Event::Ignited { x: p.x, y: p.y, what: "oil" });
            } else {
                self.light_cell(f, cell, t.flame_ground_seconds, true);
                let p = cell_to_world(cell.0, cell.1);
                f.events.push(Event::Ignited { x: p.x, y: p.y, what: "ground" });
            }
        }

        // --- Tile exposure. ---
        let half = OBSTACLE_GRID_SIZE * 0.5;
        let mut collapse: Vec<(Entity, Position, &'static str)> = Vec::new();
        let mut ignited: Vec<(Position, &'static str)> = Vec::new();
        if cones.is_empty() {
            // No stream: every tile's heat only cools, each on its own, so
            // the order the tiles are met in shows nowhere.
            for o in self.world.query::<&mut Obstacle>().iter() {
                if !(o.destroyed || o.burning || o.fuse.is_some()) {
                    o.heat = (o.heat - decay).max(0.0);
                }
            }
        }
        // In cell order: what a tile catching or giving way sets off.
        let streams = if cones.is_empty() { 0 } else { usize::MAX };
        for (entity, o) in super::ordered(self.world.query::<(Entity, &mut Obstacle)>().iter().take(streams), |(_, o)| super::cell_order(o.position)) {
            if o.destroyed || o.burning || o.fuse.is_some() {
                continue;
            }
            let hit = cones.iter().find(|(cone, _)| cone.contains(o.position, half));
            let Some((cone, _)) = hit else {
                o.heat = (o.heat - decay).max(0.0);
                continue;
            };
            if o.heat <= 0.0 && o.material.is_wall() {
                // First contact soots the face toward the nozzle.
                o.scorched |= face_toward(o.position, cone.origin);
            }
            o.heat += f.dt;
            let m = o.material;
            if m.is_explosive() {
                if o.heat >= t.flame_ignite_seconds {
                    arm_fuse(o, t.fire_fuse_factor, cone.origin);
                    ignited.push((o.position, "drum"));
                }
            } else if m.is_tree() || m == crate::obstacle::Material::Wood || m.catches_fire() {
                if o.heat >= t.flame_ignite_seconds {
                    // Whatever the tile rolled: the one thing that lights
                    // a plank that was rolled to break, and a range board.
                    o.health = 0.0;
                    o.burning = true;
                    ignited.push((o.position, if m.is_tree() { "tree" } else if m.catches_fire() { "target" } else { "wood" }));
                }
            } else if m == crate::obstacle::Material::Sandbag {
                if o.heat >= t.flame_sandbag_seconds {
                    collapse.push((entity, o.position, "sandbag"));
                }
            } else if m == crate::obstacle::Material::Fence {
                if o.heat >= t.flame_fence_seconds {
                    collapse.push((entity, o.position, "fence"));
                }
            }
        }
        for (pos, what) in ignited {
            f.events.push(Event::Ignited { x: pos.x, y: pos.y, what });
        }
        for (entity, pos, what) in collapse {
            f.events.push(Event::Ignited { x: pos.x, y: pos.y, what });
            self.damage_obstacle(f, entity, f32::MAX, DamageCause::Fire);
        }

        // --- Presentation hooks. ---
        let every = t.flame_shimmer_every_frames.max(1) as u64;
        if self.frame % every == 0 {
            for jet in &kept {
                f.muzzle_flashes.push(Shockwave::new(jet.nozzle));
            }
        }
        self.flame_jets = kept;
    }

    /// This frame's flame jets, reach already capped by terrain - what
    /// `fx.rs` draws the stream from.
    pub fn flames(&self) -> &[FlameJet] {
        &self.flame_jets
    }

    /// Every live tank with afterburn on it: position and seconds left.
    /// A state the particle layer samples, like `burning_wrecks`.
    pub fn burning_tanks(&self) -> Vec<(Position, f32)> {
        self.world
            .query::<&Tank>()
            .iter()
            .filter(|t| !t.is_wreck() && t.burn_timer > 0.0)
            .map(|t| (t.position, t.burn_timer))
            .collect()
    }

    /// Flame exposure of a ground cell, in seconds, for the overlays and
    /// tests. 0 for a cell the stream has not touched.
    pub fn cell_heat(&self, cell: (i32, i32)) -> f32 {
        self.heat.get(&cell).copied().unwrap_or(0.0)
    }
}
