//! Breakable crates (docs/CRATES_SPEC.md, the `crate_breakable` knob): a
//! blast or fire breaks a pickup's crate. Shells and bullets still fly over
//! it - a crate is low, and making it a target would change every
//! firefight, the AI's line of sight and the client's prediction - so what
//! reaches one is what reaches the ground: a drum, a missile's burst, a
//! dying tank, another crate cooking off, the flamethrower and fire.
//!
//! A crate breaks by what is inside (`PickupKind::cooks_off`): ordnance and
//! energy cook off in a blast of their own, a fraction of an oil drum's
//! (`crate_cookoff_scale`; the flamethrower's fuel leaves a pool of fire),
//! and the rest spill - the pickup lies loose on its cell for
//! `crate_spill_seconds`, drawn as its bare symbol, and is taken like any
//! other until its clock runs out.
//!
//! A blast takes its mid damage times its falloff off a crate - no roll -
//! and a crate's fire runs on the round clock, so this draws no RNG until
//! a crate cooks off; a cook-off rolls its damage on tanks, bongs and tiles
//! as a drum's blast does. With the knob off nothing here touches a crate,
//! and every round replays as it did.

use hecs::Entity;
use std::collections::HashSet;

use crate::blast::{BlastFx, BlastKind, BlastShape, Scorch};
use crate::decal::Decal;
use crate::map::world_to_cell;
use crate::obstacle::Material;
use crate::pickup::{Pickup, PickupKind};
use crate::shockwave::Shockwave;
use crate::tuning::tuning;
use crate::Position;

use super::combat::BlastParams;
use super::{Event, Frame, Game, Spectacle};

impl Game {
    /// A blast at `center` reaching the crates: each whole crate within
    /// `params.radius` loses the blast's mid damage times its falloff, and
    /// one left with none is queued to break (`explosions`). Nothing while
    /// crates are unbreakable.
    pub(super) fn blast_crates(&mut self, f: &mut Frame, center: Position, params: &BlastParams) {
        if !self.crates_breakable {
            return;
        }
        let mid = (params.damage.0 + params.damage.1) * 0.5;
        let mut broken: Vec<(Position, Entity)> = Vec::new();
        for (entity, pickup) in self.world.query::<(Entity, &mut Pickup)>().iter() {
            if pickup.loose.is_some() || pickup.health <= 0.0 {
                continue;
            }
            let dist = pickup.position.distance_to(center);
            if dist > params.radius {
                continue;
            }
            pickup.health -= mid * (1.0 - dist / params.radius);
            if pickup.health <= 0.0 {
                broken.push((pickup.position, entity));
            }
        }
        queue_breaks(f, broken);
    }

    /// A blast at `center` of `radius` tossing the supply crates round it
    /// (docs/CRATES_SPEC.md "Tossed by a blast"): every pickup on the
    /// ground within `crate_toss_reach` of the radius is thrown straight
    /// away from the centre, `crate_toss_cells` at the centre falling
    /// linearly to none at the reach, and lands at the last clear spot on
    /// that line (`crate_lands_clear`) - short of deep water, lava, a tile
    /// or the field's edge. It keeps its slot and lies at its new place
    /// from now on (`Pickup::drift`, what the seeks and the collection
    /// read), but nobody takes it while it is in the air
    /// (`Pickup::airborne`). `Event::CrateTossed` says so; the arc is a
    /// `CrateToss` the painters read. In position order; no RNG.
    pub(super) fn toss_crates(&mut self, f: &mut Frame, center: Position, radius: f32) {
        let t = tuning();
        let reach = radius * t.crate_toss_reach;
        let most = t.crate_toss_cells * crate::OBSTACLE_GRID_SIZE;
        if reach <= 0.0 || most <= 0.0 {
            return;
        }
        let mut tossed: Vec<(Entity, Position, Position)> = Vec::new();
        for (entity, pickup) in self.world.query::<(Entity, &Pickup)>().iter() {
            if pickup.airborne > 0.0 {
                continue;
            }
            let from = pickup.at();
            let line = from - center;
            let dist = line.length();
            if dist > reach {
                continue;
            }
            let dir = if dist > 0.001 { line / dist } else { crate::math::Vec2::new(0.0, -1.0) };
            let want = most * (1.0 - dist / reach);
            let to = self.crate_landing(f, from, dir, want);
            if to.distance_to(from) >= CRATE_TOSS_MIN_PX {
                tossed.push((entity, from, to));
            }
        }
        tossed.sort_by(|a, b| a.1.y.total_cmp(&b.1.y).then(a.1.x.total_cmp(&b.1.x)));
        for (entity, from, to) in tossed {
            let Ok(mut pickup) = self.world.get::<&mut Pickup>(entity) else { continue };
            pickup.drift = pickup.drift + (to - from);
            pickup.slide = crate::math::Vec2::zero();
            pickup.airborne = t.crate_toss_seconds;
            let kind = pickup.kind;
            drop(pickup);
            f.events.push(Event::CrateTossed { kind, x: from.x, y: from.y, to_x: to.x, to_y: to.y });
            self.crate_in_air(kind, from, to);
        }
    }

    /// Put a tossed crate's arc up, from `from` to `to`: the round as it
    /// tosses one, a replica from `Event::CrateTossed`.
    pub(crate) fn crate_in_air(&mut self, kind: PickupKind, from: Position, to: Position) {
        self.crate_tosses.retain(|c| c.flying());
        self.crate_tosses.push(crate::pickup::CrateToss { kind, from, to, age: 0.0, total: tuning().crate_toss_seconds });
    }

    /// Age the tossed crates' arcs by `dt`, dropping the ones that landed.
    pub(crate) fn age_crate_tosses(&mut self, dt: f32) {
        for toss in &mut self.crate_tosses {
            toss.age += dt;
        }
        self.crate_tosses.retain(|c| c.flying());
    }

    /// The arc of the crate in the air landing at `at`, if one is.
    pub fn crate_toss_to(&self, at: Position) -> Option<crate::pickup::CrateToss> {
        self.crate_tosses.iter().rev().find(|c| c.flying() && c.to.distance_to(at) < 0.5).copied()
    }

    /// Where a crate tossed from `from` along the unit `dir` for `want` px
    /// comes down: the furthest point on that line, in `CRATE_TOSS_STEP_PX`
    /// steps, that it reaches with every step before it clear
    /// (`crate_lands_clear`); `from` if the first step is not.
    fn crate_landing(&self, f: &Frame, from: Position, dir: crate::math::Vec2, want: f32) -> Position {
        let solid: HashSet<(i32, i32)> =
            self.world.query::<&crate::obstacle::Obstacle>().iter().filter(|o| !o.destroyed && !o.material.is_loose()).map(|o| o.cell()).collect();
        let steps = (want / CRATE_TOSS_STEP_PX).floor() as i32;
        let mut landed = from;
        for k in 1..=steps {
            let at = from + dir * (k as f32 * CRATE_TOSS_STEP_PX);
            if !self.crate_lands_clear(&solid, at, f.width, f.height) {
                break;
            }
            landed = at;
        }
        landed
    }

    /// Whether a crate may come down at `at`: inside the field by half a
    /// cell, on no fixed tile's cell, in no deep water and on no lava.
    fn crate_lands_clear(&self, solid: &HashSet<(i32, i32)>, at: Position, width: f32, height: f32) -> bool {
        let inset = crate::OBSTACLE_GRID_SIZE * 0.5;
        (inset..=width - inset).contains(&at.x)
            && (inset..=height - inset).contains(&at.y)
            && !solid.contains(&world_to_cell(at))
            && self.water.depth_at(at) != crate::ground::Depth::Deep
            && self.lava.depth_at(at) == crate::ground::Depth::Dry
    }

    /// Once a frame, before `explosions`: the crates' fire and the loose
    /// pickups' clocks. A crate catches under the flamethrower once its
    /// cell has had `crate_ignite_seconds` of heat, or at once on a
    /// burning ground cell, and falls in `crate_burn_seconds` later; a
    /// loose pickup is gone when its clock runs out.
    pub(super) fn tick_crates(&mut self, f: &mut Frame) {
        let t = tuning();
        let breakable = self.crates_breakable;
        let burning: HashSet<(i32, i32)> = if breakable { self.fires.iter().map(|fire| fire.cell).collect() } else { HashSet::new() };
        let mut gone: Vec<Entity> = Vec::new();
        let mut broken: Vec<(Position, Entity)> = Vec::new();
        for (entity, pickup) in self.world.query::<(Entity, &mut Pickup)>().iter() {
            pickup.airborne = (pickup.airborne - f.dt).max(0.0);
            if let Some(left) = pickup.loose.as_mut() {
                *left -= f.dt;
                if *left <= 0.0 {
                    gone.push(entity);
                }
                continue;
            }
            if !breakable || pickup.health <= 0.0 {
                continue;
            }
            match pickup.burn.as_mut() {
                Some(left) => {
                    *left -= f.dt;
                    if *left <= 0.0 {
                        pickup.health = 0.0;
                        broken.push((pickup.position, entity));
                    }
                }
                None => {
                    let cell = world_to_cell(pickup.position);
                    let heat = self.heat.get(&cell).copied().unwrap_or(0.0);
                    if burning.contains(&cell) || heat >= t.crate_ignite_seconds {
                        pickup.burn = Some(t.crate_burn_seconds);
                    }
                }
            }
        }
        for entity in gone {
            self.world.despawn(entity).ok();
        }
        queue_breaks(f, broken);
        self.age_crate_tosses(f.dt);
    }

    /// Break the crate `entity` (from `explosions`' worklist): it cooks off
    /// or spills, by what is inside. `live` false - the end screen - still
    /// breaks it, the scene finishing, but its blast hurts nobody.
    pub(super) fn break_crate(&mut self, f: &mut Frame, entity: Entity, live: bool) {
        let t = tuning();
        let (kind, at) = {
            let Ok(mut pickup) = self.world.get::<&mut Pickup>(entity) else { return };
            if pickup.loose.is_some() {
                return;
            }
            pickup.health = 0.0;
            pickup.burn = None;
            if !pickup.kind.cooks_off() {
                pickup.loose = Some(t.crate_spill_seconds);
            }
            (pickup.kind, pickup.position)
        };
        let cooked = kind.cooks_off();
        f.events.push(Event::CrateBroken { kind, x: at.x, y: at.y, cooked });
        if cooked {
            self.world.despawn(entity).ok();
            self.crate_cookoff(f, at, kind, live);
        }
    }

    /// A crate's contents going off at `center`: an oil drum's blast scaled
    /// by `crate_cookoff_scale` on both sides, the bongs, the tiles and the
    /// other crates, and for the flamethrower's fuel a pool of fire on its
    /// cell. The damage rolls are the round's RNG, drawn in a drum's order:
    /// the seats, then the enemies, the bongs, the tiles.
    fn crate_cookoff(&mut self, f: &mut Frame, center: Position, kind: PickupKind, live: bool) {
        let t = tuning();
        let params = cookoff_params();
        if live && params.radius > 0.0 {
            self.blast(f, center, &params, &[]);
            if kind == PickupKind::Flamethrower {
                self.light_cell(f, world_to_cell(center), t.oil_pool_seconds, true);
            }
        }
        let mut show = Spectacle::default();
        self.crate_cookoff_show(&mut show, center);
        f.stage(show);
    }

    /// The show a crate cooking off at `center` puts on: a ripple, the
    /// impact flash, a fireball the cook-off's size, a scorch on dry
    /// ground, the crate's charred boards and the grass flattened. All of
    /// it but the damage, which is why a replica can call it off
    /// `Event::CrateBroken`. Every choice hashes the position; no RNG.
    pub(crate) fn crate_cookoff_show(&mut self, show: &mut Spectacle, center: Position) {
        let t = tuning();
        let scale = t.crate_cookoff_scale;
        show.shocks.push(Shockwave::scaled(center, crate::tuning::tuning().barrel_shock * scale));
        show.impact_flashes.push(Shockwave::new(center));
        let mut fx = BlastFx::shaped(center, BlastKind::Oil, BlastShape::Plain);
        fx.scale *= scale.max(0.2);
        show.blast_fx.push(fx);
        if self.water.depth_at(center) == crate::ground::Depth::Dry {
            show.scorches.push(Scorch::with(center, 0.7 * scale.max(0.2), None));
        }
        if let Some(decal) = Decal::new(Material::Wood, center, true) {
            show.decals.push(decal);
        }
        crate::grass::flatten(&mut self.grass, center, cookoff_params().radius * t.blast_grass_flatten);
    }
}

/// How far a toss must carry a crate for it to leave the ground, px.
const CRATE_TOSS_MIN_PX: f32 = 4.0;

/// The step a toss's landing is searched along its line in, px.
const CRATE_TOSS_STEP_PX: f32 = 2.0;

/// A crate cooking off: an oil drum's blast scaled by `crate_cookoff_scale`.
fn cookoff_params() -> BlastParams {
    let scale = tuning().crate_cookoff_scale.max(0.0);
    let mut params = BlastParams::barrel();
    params.radius *= scale;
    params.damage = (params.damage.0 * scale, params.damage.1 * scale);
    params.knockback *= scale;
    params.lanterns = false;
    params
}

/// Queue the crates a blast or fire broke for `explosions`, in a fixed
/// order - top to bottom, left to right - so the cook-offs they set off
/// draw the round's RNG in the same order every replay.
fn queue_breaks(f: &mut Frame, mut broken: Vec<(Position, Entity)>) {
    broken.sort_by(|a, b| a.0.y.total_cmp(&b.0.y).then(a.0.x.total_cmp(&b.0.x)));
    for (_, entity) in broken {
        if !f.crate_breaks.contains(&entity) {
            f.crate_breaks.push(entity);
        }
    }
}
