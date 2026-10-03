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
//! a crate cooks off; a cook-off rolls its damage on tanks, frogs and tiles
//! as a drum's blast does. With the knob off nothing here touches a crate,
//! and every round replays as it did.

use hecs::Entity;
use std::collections::HashSet;

use crate::blast::{BlastFx, BlastKind, BlastShape, Scorch};
use crate::decal::Decal;
use crate::frog::Frog;
use crate::map::world_to_cell;
use crate::obstacle::{face_toward, Material, Obstacle};
use crate::pickup::{Pickup, PickupKind};
use crate::shockwave::Shockwave;
use crate::tank::Tank;
use crate::tuning::tuning;
use crate::Position;
use crate::ai::Ai;

use super::combat::{explosion_hit, BlastParams};
use super::props::DamageCause;
use super::{Event, Frame, Game, Spectacle, SHOCK_BARREL, SHOCK_FROG};

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
    /// by `crate_cookoff_scale` on both sides, the frogs, the tiles and the
    /// other crates, and for the flamethrower's fuel a pool of fire on its
    /// cell. The damage rolls are the round's RNG, drawn in a drum's order:
    /// the seats, then the enemies, the frogs, the tiles.
    fn crate_cookoff(&mut self, f: &mut Frame, center: Position, kind: PickupKind, live: bool) {
        let t = tuning();
        let params = cookoff_params();
        if live && params.radius > 0.0 {
            for player in self.seats_on_field().into_iter().flatten() {
                let mut q = self.world.query_one::<&mut Tank>(player);
                let tank = q.get().expect("player entity always has a Tank");
                if let Some(dv) = explosion_hit(tank, center, true, &mut self.physics, &mut f.rng, &mut f.kills, &params) {
                    f.shoves.push(tank.owner(), dv);
                }
            }
            for tank in self.world.query::<&mut Tank>().with::<&Ai>().iter() {
                explosion_hit(tank, center, true, &mut self.physics, &mut f.rng, &mut f.kills, &params);
            }
            let mut dead_frogs = Vec::new();
            for frog in self.world.query::<&mut Frog>().iter() {
                let dist = frog.position.distance_to(center);
                if frog.is_dead() || dist > params.radius {
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
                    if o.material.is_wall() {
                        o.scorched |= face_toward(o.position, center);
                    }
                    Some((e, o.material, 1.0 - dist / params.radius))
                })
                .collect();
            for (entity, material, falloff) in hits {
                let amount = if material.is_explosive() { f32::MAX } else { params.roll_damage(&mut f.rng) * falloff };
                self.damage_obstacle(f, entity, amount, DamageCause::Blast { falloff, from: center });
            }
            self.blast_crates(f, center, &params);
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
        show.shocks.push(Shockwave::scaled(center, SHOCK_BARREL * scale));
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

/// A crate cooking off: an oil drum's blast scaled by `crate_cookoff_scale`.
fn cookoff_params() -> BlastParams {
    let scale = tuning().crate_cookoff_scale.max(0.0);
    let mut params = BlastParams::barrel();
    params.radius *= scale;
    params.damage = (params.damage.0 * scale, params.damage.1 * scale);
    params.knockback *= scale;
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
