//! The flamethrower's stream (docs/flamethrower-prd.md): what one frame
//! of a held trigger does to the tanks, the frogs and the ground and
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

use hecs::Entity;
use sola_raylib::core::math::Vector2;

use super::props::{arm_fuse, DamageCause};
use super::weapons::FlameJet;
use super::{Event, Frame, Game, HitTarget};
use crate::ai::Ai;
use crate::frog::Frog;
use crate::map::cell_to_world;
use crate::obstacle::{face_toward, Obstacle};
use crate::shockwave::Shockwave;
use crate::tank::Tank;
use crate::tuning::tuning;
use crate::{Position, MAX_DAMAGE, OBSTACLE_GRID_SIZE};

/// The stream starts sampling the ground this far past the muzzle, so a
/// standing tank never lights the cell it is sitting in.
const SAMPLE_START_PX: f32 = OBSTACLE_GRID_SIZE / 4.0;
/// Spacing of the exposure samples along the centre line - two per cell,
/// so no cell the line crosses is skipped.
const SAMPLE_STEP_PX: f32 = OBSTACLE_GRID_SIZE / 2.0;
/// Lateral slack for a hull: the stream counts as on a tank whose centre
/// is within a quarter hull of the cone, since a jet that visibly washes
/// over a hull's flank and does nothing reads as a miss.
const HULL_SLACK_FRACTION: f32 = 0.25;

/// One jet's cone this frame, with the range already capped by the first
/// solid tile on its centre line.
struct Cone {
    origin: Position,
    dir: Vector2,
    /// Effective reach: `flame_range`, or where the centre line met a tile.
    reach: f32,
    /// tan of the half angle: half width at distance `d` is `d * spread`.
    spread: f32,
}

impl Cone {
    fn new(jet: &FlameJet, reach: f32) -> Self {
        Cone { origin: jet.origin, dir: jet.dir, reach, spread: tuning().flame_half_angle_deg.to_radians().tan() }
    }

    /// Is `p` inside the cone, widened by `pad` on every side?
    fn contains(&self, p: Position, pad: f32) -> bool {
        let dx = p.x - self.origin.x;
        let dy = p.y - self.origin.y;
        let along = dx * self.dir.x + dy * self.dir.y;
        let lateral = (dx * self.dir.y - dy * self.dir.x).abs();
        along >= -pad && along <= self.reach + pad && lateral <= along.max(0.0) * self.spread + pad
    }

    /// The ground cells the stream heats this frame: the centre line
    /// sampled from a quarter cell past the muzzle to the reach, and the
    /// cone's width sampled across its outer half. Sorted and unique.
    fn cells(&self) -> Vec<(i32, i32)> {
        let mut cells = Vec::new();
        let mut along = SAMPLE_START_PX;
        while along <= self.reach {
            let cx = self.origin.x + self.dir.x * along;
            let cy = self.origin.y + self.dir.y * along;
            cells.push(cell_of(Position::new(cx, cy)));
            if along >= self.reach * 0.5 {
                let half_w = along * self.spread;
                // Perpendicular to the facing.
                let (px, py) = (-self.dir.y * half_w, self.dir.x * half_w);
                cells.push(cell_of(Position::new(cx + px, cy + py)));
                cells.push(cell_of(Position::new(cx - px, cy - py)));
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
    /// frogs, ground and tiles; tick the afterburn on every burning hull;
    /// cool everything the stream is not on. Runs only while the round
    /// is live - on the end screen nothing fires and nothing burns down.
    pub(super) fn resolve_flames(&mut self, f: &mut Frame) {
        let jets = std::mem::take(&mut f.flame_jets);
        let t = tuning();
        let decay = t.flame_heat_decay.max(0.0) * f.dt;

        let mut cones: Vec<(Cone, &FlameJet)> = Vec::with_capacity(jets.len());
        let mut kept = Vec::with_capacity(jets.len());
        for jet in &jets {
            let end = Position::new(jet.origin.x + jet.dir.x * jet.range, jet.origin.y + jet.dir.y * jet.range);
            let reach = f.terrain.first_solid_along(jet.origin, end).map_or(jet.range, |t| t * jet.range);
            cones.push((Cone::new(jet, reach), jet));
            kept.push(FlameJet { reach, ..*jet });
        }

        // --- Tanks and frogs in a cone. ---
        let mut touched: Vec<Entity> = Vec::new();
        for (cone, jet) in &cones {
            let targets: Vec<Entity> = self
                .world
                .query::<(Entity, &Tank)>()
                .iter()
                .filter(|(e, tank)| *e != jet.shooter && !tank.is_wreck() && tank.body.is_some())
                .filter(|(_, tank)| cone.contains(tank.position, tank.hull_size() * HULL_SLACK_FRACTION))
                .map(|(e, _)| e)
                .collect();
            for entity in targets {
                let (killed, pos, owner, entered) = {
                    let mut q = self.world.query_one::<&mut Tank>(entity);
                    let tank = q.get().expect("cone target always has a Tank");
                    let mut d = t.flame_damage_per_second * f.dt;
                    if jet.owner.is_player() && tank.owner().is_player() {
                        d *= t.friendly_fire_damage_factor;
                    }
                    let entered = !self.flame_contacts.contains(&entity);
                    tank.take_damage(d, MAX_DAMAGE);
                    tank.mark_hit();
                    tank.burn_timer = t.flame_afterburn_seconds;
                    (tank.is_wreck(), tank.position, tank.owner(), entered)
                };
                touched.push(entity);
                if entered {
                    let target = match owner {
                        crate::shell::Owner::Player(player) => HitTarget::Player { player },
                        crate::shell::Owner::Enemy(slot) => HitTarget::Enemy { slot },
                    };
                    f.events.push(Event::Hit { target, damage: 0.0, killed, x: pos.x, y: pos.y });
                }
                if killed {
                    f.kills.push((pos, owner));
                } else {
                    let mut q = self.world.query_one::<&mut Ai>(entity);
                    if let Ok(ai) = q.get() {
                        ai.notify_hit();
                    }
                }
            }
            if t.flame_frog_damage_per_second > 0.0 {
                let mut dead_at = Vec::new();
                for (entity, frog) in self.world.query::<(Entity, &mut Frog)>().iter() {
                    if frog.is_dead() || !cone.contains(frog.position, OBSTACLE_GRID_SIZE * 0.25) {
                        continue;
                    }
                    frog.damage(t.flame_frog_damage_per_second * f.dt);
                    let entered = !self.flame_contacts.contains(&entity);
                    touched.push(entity);
                    if entered || frog.is_dead() {
                        let target = HitTarget::Frog { side: frog.side };
                        f.events.push(Event::Hit { target, damage: 0.0, killed: frog.is_dead(), x: frog.position.x, y: frog.position.y });
                    }
                    if frog.is_dead() {
                        dead_at.push(frog.position);
                    }
                }
                for pos in dead_at {
                    f.shocks.push(Shockwave::scaled(pos, super::SHOCK_FROG));
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
            order.extend(self.world.query::<(Entity, &Tank)>().with::<&Ai>().iter().map(|(e, _)| e));
            for entity in order {
                if touched.contains(&entity) {
                    continue;
                }
                let mut q = self.world.query_one::<&mut Tank>(entity);
                let Ok(tank) = q.get() else { continue };
                if tank.is_wreck() || tank.burn_timer <= 0.0 {
                    continue;
                }
                tank.take_damage(t.flame_afterburn_dps * f.dt, MAX_DAMAGE);
                if tank.is_wreck() {
                    f.kills.push((tank.position, tank.owner()));
                }
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
        let solid: std::collections::HashSet<(i32, i32)> =
            self.world.query::<&Obstacle>().iter().filter(|o| !o.destroyed).map(|o| o.cell()).collect();
        for cell in &heated {
            if solid.contains(cell) {
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
        for (entity, o) in self.world.query::<(Entity, &mut Obstacle)>().iter() {
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
            } else if m.is_tree() || m == crate::obstacle::Material::Wood {
                if o.heat >= t.flame_ignite_seconds {
                    // Whatever the tile rolled: the one thing that lights
                    // a plank that was rolled to break.
                    o.health = 0.0;
                    o.burning = true;
                    ignited.push((o.position, if m.is_tree() { "tree" } else { "wood" }));
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
                f.muzzle_flashes.push(Shockwave::new(jet.origin));
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
