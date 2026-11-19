//! Damage and knockback: applying a resolved hit to whatever it struck,
//! ram contact between the player and an enemy, a wreck's explosion, and
//! the bong's evasive hop.

use crate::math;
use crate::tuning::tuning;
use hecs::Entity;
use rand::RngExt;
use crate::RoundRng;
use crate::math::Vec2;

use crate::ai::Ai;
use crate::bong::{Bong, Side};
use crate::knock::Knock;
use crate::obstacle::{face_toward, Material, Obstacle};
use crate::physics::Physics;
use crate::shell::Owner;
use crate::shockwave::Shockwave;
use crate::tank::Tank;
use crate::{
    BONG_HOP_ANGLE_FAN_DEG,
    BONG_HOP_DISTANCE_STEPS,
    MAX_DAMAGE,
    Position,
};

use super::hits::{ShellTarget, Terrain};
use super::props::DamageCause;
use super::{with_bong_mut, Event, Footing, Frame, Game, HitCause, HitTarget, Shoves};

/// Whose hulls a blast hurts. It shoves every live hull it reaches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum BlastSpares {
    Nobody,
    /// The side of this owner is shoved and not hurt: a wreck's own side,
    /// the side that fired a missile or threw a grenade.
    SideOf(Owner),
}

/// Which bongs a blast hurts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum BlastBongs {
    None,
    All,
    /// Only the bong of the side opposing this owner.
    Opposing(Owner),
}

/// One explosion's numbers and what it touches, for `Game::blast` - the
/// one blast every source goes through: a tank wreck's, a drum's, a lava
/// bomb's, a crate cooking off, a missile's, a grenade's, a drone's, a
/// jet's.
pub(super) struct BlastParams {
    pub radius: f32,
    pub damage: (f32, f32),
    pub knockback: f32,
    /// Who set the blast off, for the kill credit (`Tank::credit`): a
    /// missile's shooter, the seat that wrecked the tank now exploding.
    /// `None` for a drum's.
    pub by: Option<Owner>,
    pub spares: BlastSpares,
    pub bongs: BlastBongs,
    /// Lanterns on the ground within `LANTERN_REACH` of the radius are
    /// knocked out.
    pub lanterns: bool,
    /// Grenades on the ground in the radius go off with it.
    pub grenades: bool,
    /// A drum's way with tiles: an explosive tile in the radius goes off
    /// with no roll, and one already on a fuse is left to it. Without it
    /// every tile takes the rolled damage.
    pub chains: bool,
}

/// How much of a blast's radius knocks a lantern out.
const LANTERN_REACH: f32 = 0.6;

impl BlastParams {
    /// A blast of the world's own - a drum, a lava bomb: everyone in the
    /// radius, both bongs, the lanterns, the drums chained.
    pub fn world(radius: f32, damage: (f32, f32), knockback: f32) -> Self {
        BlastParams { radius, damage, knockback, by: None, spares: BlastSpares::Nobody, bongs: BlastBongs::All, lanterns: true, grenades: false, chains: true }
    }

    /// Ordnance fired by `by` going off - a missile, a grenade, a drone, a
    /// jet's burst or bomb: the side opposing `by` and its bong.
    pub fn ordnance(radius: f32, damage: (f32, f32), knockback: f32, by: Owner) -> Self {
        BlastParams {
            radius,
            damage,
            knockback,
            by: Some(by),
            spares: BlastSpares::SideOf(by),
            bongs: BlastBongs::Opposing(by),
            lanterns: false,
            grenades: false,
            chains: false,
        }
    }

    /// A tank's death (`Game::apply_explosion`): a chip to the side
    /// opposing `victim`, credited to `by`, and no bong.
    pub fn tank_wreck(victim: Owner, by: Option<Owner>) -> Self {
        BlastParams {
            radius: tuning().explosion_radius,
            damage: (tuning().explosion_damage_min, tuning().explosion_damage_max),
            knockback: tuning().explosion_knockback_speed,
            by,
            spares: BlastSpares::SideOf(victim),
            bongs: BlastBongs::None,
            lanterns: false,
            grenades: false,
            chains: false,
        }
    }

    /// The red oil drum's blast.
    pub fn barrel() -> Self {
        let t = tuning();
        BlastParams::world(t.barrel_blast_radius, (t.barrel_blast_damage_min, t.barrel_blast_damage_max), t.barrel_blast_knockback_speed)
    }

    /// The grey fuel drum's: bigger, sharper, shorter.
    pub fn fuel() -> Self {
        let t = tuning();
        BlastParams::world(t.fuel_blast_radius, (t.fuel_blast_damage_min, t.fuel_blast_damage_max), t.fuel_blast_knockback_speed)
    }

    /// A volcano's lava bomb landing (docs/volcano.md).
    pub fn lava_bomb() -> Self {
        let t = tuning();
        BlastParams::world(t.volcano_bomb_radius_px, (t.volcano_bomb_damage_min, t.volcano_bomb_damage_max), t.volcano_bomb_knockback)
    }

    /// Centre damage before falloff (`roll_range`).
    pub fn roll_damage(&self, rng: &mut RoundRng) -> f32 {
        roll_range(rng, self.damage.0, self.damage.1)
    }
}

/// A uniform roll in `min..max`: one draw. A tuned range that is empty or
/// inverted (`min >= max`) is `min` and draws nothing, so a knob pair set
/// the wrong way round plays instead of panicking.
pub(super) fn roll_range(rng: &mut RoundRng, min: f32, max: f32) -> f32 {
    if max > min { rng.random_range(min..max) } else { min }
}

/// How a hurt is told to the round's feed (`Event::Hit`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Report {
    /// One landed thing - a shot, a slug, a wave, a blast's share: an
    /// `Event::Hit` with what the hull took.
    Hit(HitCause),
    /// A source that burns for as long as it touches - a flame stream,
    /// lava, a burning cell, ooze: an `Event::Hit` with damage 0 on the
    /// first tick of the contact (`Tank::touch`) and on the tick that
    /// wrecks the hull, never one a frame.
    Contact(HitCause),
    /// The source logs its own event (`Event::Ram`, `Event::BongBite`).
    Own,
}

/// One hurt on its way to a tank through `hurt`.
#[derive(Clone, Copy, Debug)]
pub(super) struct Hurt {
    /// Damage before the shield and a seat's armour (`Tank::take_damage`).
    /// Zero lands nothing and credits nobody, but is still reported.
    pub amount: f32,
    /// Where the hull's damage stops: `MAX_DAMAGE`, or one below it for a
    /// bite that never finishes a seat off.
    pub cap: f32,
    /// Who did it, for the kill credit (`Tank::credit`) and a tower's
    /// grudge. `None` for the world's own hurts.
    pub by: Option<Owner>,
    /// Where it landed, for the event.
    pub at: Position,
    pub report: Report,
    /// A rod from god's: the hull is wrecked whole, past the shield, the
    /// armour and the amount.
    pub crush: bool,
}

impl Hurt {
    pub fn hit(amount: f32, cause: HitCause, by: Option<Owner>, at: Position) -> Self {
        Hurt { amount, cap: MAX_DAMAGE, by, at, report: Report::Hit(cause), crush: false }
    }

    pub fn contact(amount: f32, cause: HitCause, by: Option<Owner>, at: Position) -> Self {
        Hurt { report: Report::Contact(cause), ..Hurt::hit(amount, cause, by, at) }
    }

    pub fn own(amount: f32, by: Option<Owner>, at: Position) -> Self {
        Hurt { amount, cap: MAX_DAMAGE, by, at, report: Report::Own, crush: false }
    }
}

/// What a hurt did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Landed {
    /// What the hull took: 0 where a shield took it instead.
    pub landed: f32,
    pub killed: bool,
    pub owner: Owner,
    pub position: Position,
}

/// **The one way a tank loses health.** Every source - a shot, a beam, a
/// stream, a blast, a ram, a bite, lava, fire, ooze, a rod - lands here:
/// the damage goes on through `Tank::take_damage` (or the rod's crush),
/// the hull is marked hit and `hurt.by` credited, the hurt is reported as
/// its `Report` says, and a hull it wrecks goes on `kills`. `None` for a
/// wreck, which nothing hurts further. Draws no RNG: a source rolls its
/// own damage first. `Game::hurt_tank` is this plus the AI's alert.
pub(super) fn hurt(tank: &mut Tank, hurt: &Hurt, events: &mut Vec<Event>, kills: &mut Vec<(Position, Owner)>) -> Option<Landed> {
    if tank.is_wreck() {
        return None;
    }
    let landed = if hurt.crush {
        tank.crush();
        MAX_DAMAGE
    } else if hurt.amount > 0.0 {
        tank.take_damage(hurt.amount, hurt.cap)
    } else {
        0.0
    };
    if hurt.crush || hurt.amount > 0.0 {
        tank.mark_hit();
        if let Some(by) = hurt.by {
            tank.credit(by);
        }
    }
    let killed = tank.is_wreck();
    let owner = tank.owner();
    let target = match owner {
        Owner::Player(player) => HitTarget::Player { player },
        Owner::Enemy(slot) => HitTarget::Enemy { slot },
        Owner::Tower { .. } => unreachable!("no tank is owned by a tower"),
    };
    match hurt.report {
        Report::Hit(cause) => events.push(Event::Hit { target, damage: landed, killed, x: hurt.at.x, y: hurt.at.y, cause }),
        Report::Contact(cause) => {
            if tank.touch(cause) || killed {
                events.push(Event::Hit { target, damage: 0.0, killed, x: hurt.at.x, y: hurt.at.y, cause });
            }
        }
        Report::Own => {}
    }
    if killed {
        kills.push((tank.position, owner));
    }
    Some(Landed { landed, killed, owner, position: tank.position })
}

/// What a hit does beyond damage: an optional shove (unit travel direction,
/// speed in px/s) for a surviving tank, and whether the bong tries to hop
/// away (along the given direction) when it survives.
pub(super) struct HitEffects {
    pub knockback: Option<(Vec2, f32)>,
    pub bong_hop: Option<Vec2>,
    /// The projectile's unit travel direction, whatever it hit: a barrel
    /// it pops leans its fire this way (`BlastShape::Shot`). `None` for a
    /// beam, which has no travel to speak of.
    pub travel: Option<Vec2>,
    /// What landed, for the `Event::Hit`.
    pub cause: HitCause,
}

impl HitEffects {
    /// A hit by `cause` that shoves nothing and has no travel.
    pub fn none(cause: HitCause) -> Self {
        HitEffects {
            knockback: None,
            bong_hop: None,
            travel: None,
            cause,
        }
    }
}

impl Game {
    /// `hurt` on the tank `entity`, and the alert: an enemy that survives
    /// a hurt that landed is told it was hit (`Ai::notify_hit`; a seat
    /// carries no `Ai`), and holds a grudge against the tower that did it.
    /// `None` for a wreck or an entity with no tank.
    pub(super) fn hurt_tank(&mut self, f: &mut Frame, entity: Entity, h: Hurt) -> Option<Landed> {
        let landed = {
            let mut q = self.world.query_one::<&mut Tank>(entity);
            let tank = q.get().ok()?;
            hurt(tank, &h, &mut f.events, &mut f.kills)?
        };
        if !landed.killed && (h.amount > 0.0 || h.crush) {
            let tower = h.by.and_then(|by| self.tower_by_owner(by));
            if let Ok(mut ai) = self.world.get::<&mut Ai>(entity) {
                match tower {
                    Some(at) => ai.notify_tower_hit(at),
                    None => ai.notify_hit(),
                }
            }
        }
        Some(landed)
    }

    /// Apply one resolved projectile/beam hit to `target`: roll damage in
    /// `dmg`, then per target kind - a tank is marked hit, either killed
    /// (recorded in `f.kills` for its shockwave/explosion) or shoved; an
    /// enemy that survives is told it was shot (`Ai::notify_hit`); a bong
    /// (either side - any shot damages any bong) takes damage and either
    /// dies (shockwave) or hops away; an obstacle
    /// takes damage; a wall absorbs the shot. Wrecks and dead bongs ignore
    /// further hits. `at` is the impact point, recorded in the `Event::Hit`
    /// every landed hit appends. `shooter` is who fired: a player hitting
    /// the other player deals `friendly_fire_damage_factor` of the roll.
    pub(super) fn apply_hit(
        &mut self,
        f: &mut Frame,
        target: ShellTarget,
        at: Position,
        dmg: (f32, f32),
        effects: HitEffects,
        shooter: Owner,
    ) {
        match target {
            ShellTarget::Tank(entity) => {
                let Some(seat) = self.world.get::<&Tank>(entity).ok().filter(|t| !t.is_wreck()).map(|t| t.owner().is_player()) else { return };
                // Scaled after the roll, so the RNG draw count never
                // depends on who shot whom.
                let mut d = roll_range(&mut f.rng, dmg.0, dmg.1);
                if shooter.is_player() && seat {
                    d *= tuning().friendly_fire_damage_factor;
                }
                // A shield takes the hit instead and what landed is 0,
                // which is what the event reports.
                let landed = self.hurt_tank(f, entity, Hurt::hit(d, effects.cause, Some(shooter), at));
                if let (Some(Landed { killed: false, .. }), Some((dir, speed))) = (landed, effects.knockback) {
                    let mut q = self.world.query_one::<&mut Tank>(entity);
                    let tank = q.get().expect("hit target always has a Tank");
                    knock(&mut self.physics, &mut f.shoves, tank, dir, Knock::impact(&tuning(), speed), Footing::DRY);
                }
            }
            ShellTarget::Bong(entity) => {
                let (dead, pos, can_hop, hop_distance) = {
                    let mut q = self.world.query_one::<&mut Bong>(entity);
                    let bong = q.get().expect("hit target always has a Bong");
                    if bong.is_dead() {
                        (true, bong.position, false, 0.0)
                    } else {
                        let d = roll_range(&mut f.rng, dmg.0, dmg.1);
                        bong.damage(d);
                        let target = HitTarget::Bong { side: bong.side };
                        f.events.push(Event::Hit { target, damage: d, killed: bong.is_dead(), x: at.x, y: at.y, cause: effects.cause });
                        (bong.is_dead(), bong.position, bong.can_hop(), bong.hop_distance())
                    }
                };
                if dead {
                    f.shocks.push(Shockwave::scaled(pos, crate::tuning::tuning().bong_shock));
                } else if let (true, Some(away)) = (can_hop, effects.bong_hop) {
                    let landing = bong_hop_target(&mut f.rng, pos, away, hop_distance, &f.terrain, f.width, f.height);
                    if let Some(new_pos) = landing {
                        with_bong_mut(&self.world, entity, |fr| fr.start_hop(new_pos));
                    }
                }
            }
            ShellTarget::Obstacle(entity) => {
                let d = roll_range(&mut f.rng, dmg.0, dmg.1);
                // Read the material before the damage lands: a killing blow
                // despawns the tile, and the event has to say what it was.
                let material = {
                    let mut q = self.world.query_one::<&Obstacle>(entity);
                    q.get().expect("a resolved obstacle hit names a live Obstacle").material
                };
                // The hit is recorded ahead of whatever the damage causes
                // (a destroyed tile, a blast), since it happened first.
                let mark = f.events.len();
                let killed = self.damage_obstacle(f, entity, d, DamageCause::Shot { dir: effects.travel, at: Some(at), by: Some(shooter) });
                f.events.insert(mark, Event::Hit { target: HitTarget::Obstacle { material }, damage: d, killed, x: at.x, y: at.y, cause: effects.cause });
            }
            ShellTarget::Wall => {
                f.events.push(Event::Hit { target: HitTarget::Wall, damage: 0.0, killed: false, x: at.x, y: at.y, cause: effects.cause });
            }
        }
    }

    /// A tank's death: an outward shove that fades linearly to nothing at
    /// `explosion_radius`, reaching every live tank regardless of side,
    /// plus a chip of damage only to the side opposing whoever died,
    /// credited to the seat the wreck itself is credited to. Obstacles in
    /// range crack too - a barrel in range goes off
    /// (`Game::damage_obstacle`). No bong is hurt. A chip that finishes
    /// off a tank pushes it onto `f.kills`, so it gets its own blast in
    /// turn.
    pub(super) fn apply_explosion(&mut self, f: &mut Frame, center: Position, victim: Owner) {
        let by = self.world.query::<&Tank>().iter().find(|t| t.owner() == victim).and_then(|t| t.last_hit_by).map(Owner::Player);
        self.blast(f, center, &BlastParams::tank_wreck(victim, by), &[]);
    }

    /// **The one blast.** Everything `params.radius` reaches from `center`
    /// takes its share, falling off linearly to nothing at the radius:
    ///
    /// - every live hull on the field is shoved out from the centre, and
    ///   hurt unless `params.spares` its side - reached at the nearest
    ///   point of its hull (`TargetShape::reach_from`), so a wide hull is
    ///   not hit from further inside itself than a narrow one;
    /// - the bongs `params.bongs` names are hurt;
    /// - a hull or a bong with a permanent tile between it and the centre
    ///   (`Terrain::line_of_fire` to where it stands - iron, a volcano's
    ///   cone, a training door) is in cover: neither hurt nor shoved;
    /// - lanterns and grenades as the flags say; every tile cracks
    ///   (`damage_obstacle`), every crate in reach breaks (`blast_crates`),
    ///   and a gravity well's held drums go off.
    ///
    /// `spared` are left out whole. One damage roll for each hull hurt,
    /// each bong and each tile, in that order - the seats in index order,
    /// the enemies by slot, the bongs, the tiles by cell - and the roll is
    /// drawn for one in cover too, so cover never moves the stream. A tank
    /// still rolling in through a gate is out of the fight: a seat by
    /// `seats_on_field`, an enemy by having no `Ai` yet.
    pub(super) fn blast(&mut self, f: &mut Frame, center: Position, params: &BlastParams, spared: &[Entity]) {
        self.chain_held_drums(f, center, params.radius);
        let mut hulls: Vec<Entity> = self.seats_on_field().into_iter().flatten().collect();
        hulls.extend(super::ordered(self.world.query::<(Entity, &Tank)>().with::<&Ai>().iter(), |(_, t)| t.owner_slot()).into_iter().map(|(e, _)| e));
        for entity in hulls {
            if spared.contains(&entity) {
                continue;
            }
            let (dist, at, owner, footing) = {
                let Ok(tank) = self.world.get::<&Tank>(entity) else { continue };
                if tank.is_wreck() {
                    continue;
                }
                let footing = Footing::at(&self.water, &self.lava, &self.craters, self.weather, tank.position, self.time);
                let dist = tank.target_shape().reach_from(center);
                (dist, tank.position, tank.owner(), footing)
            };
            if dist > params.radius {
                continue;
            }
            let falloff = 1.0 - dist / params.radius;
            let covered = !f.terrain.line_of_fire(center, at);
            let hurts = match params.spares {
                BlastSpares::Nobody => true,
                BlastSpares::SideOf(side) => !side.same_side(owner),
            };
            if hurts {
                let dmg = params.roll_damage(&mut f.rng) * falloff;
                if !covered && self.hurt_tank(f, entity, Hurt::hit(dmg, HitCause::Blast, params.by, center)).is_some_and(|l| l.killed) {
                    continue;
                }
            }
            if covered {
                continue;
            }
            let line = at - center;
            let len = line.length();
            // Sitting exactly on the blast's centre: any direction beats none.
            let dir = if len > 0.001 { line / len } else { Vec2::new(1.0, 0.0) };
            let mut q = self.world.query_one::<&mut Tank>(entity);
            let tank = q.get().expect("a hull in the blast's walk has a Tank");
            knock(&mut self.physics, &mut f.shoves, tank, dir, Knock::blast(&tuning(), params.knockback * falloff), footing);
        }
        if params.bongs != BlastBongs::None {
            let mut dead_bongs = Vec::new();
            for (entity, bong) in super::ordered(self.world.query::<(Entity, &mut Bong)>().iter(), |(_, b)| super::bong_order(b)) {
                if bong.is_dead() || spared.contains(&entity) {
                    continue;
                }
                if let BlastBongs::Opposing(owner) = params.bongs
                    && bong.side == (if owner.is_player() { Side::Player } else { Side::Enemy })
                {
                    continue;
                }
                let dist = bong.position.distance_to(center);
                if dist > params.radius {
                    continue;
                }
                let dmg = params.roll_damage(&mut f.rng) * (1.0 - dist / params.radius);
                if !f.terrain.line_of_fire(center, bong.position) {
                    continue;
                }
                bong.damage(dmg);
                if bong.is_dead() {
                    dead_bongs.push(bong.position);
                }
            }
            for pos in dead_bongs {
                f.shocks.push(Shockwave::scaled(pos, tuning().bong_shock));
            }
        }
        if params.lanterns {
            self.break_lanterns(f, center, params.radius * LANTERN_REACH);
        }
        if params.grenades {
            self.set_off_grenades(center, params.radius);
        }
        // Collect first: `damage_obstacle` needs the world free.
        let hits: Vec<(Entity, Material, f32)> = super::tiles_near(self.world.query::<(Entity, &mut Obstacle)>().iter(), center, params.radius)
            .into_iter()
            .filter(|(_, o)| !o.destroyed && !(params.chains && o.fuse.is_some()))
            .filter_map(|(e, o)| {
                let dist = o.position.distance_to(center);
                if dist > params.radius {
                    return None;
                }
                // Soot on whichever face looked at the blast; a wall's
                // business only, and cosmetic.
                if o.material.is_wall() {
                    o.scorched |= face_toward(o.position, center);
                }
                Some((e, o.material, 1.0 - dist / params.radius))
            })
            .collect();
        for (entity, material, falloff) in hits {
            // A drum's blast sets every other drum inside off - no roll.
            let amount = if params.chains && material.is_explosive() { f32::MAX } else { params.roll_damage(&mut f.rng) * falloff };
            self.damage_obstacle(f, entity, amount, DamageCause::Blast { falloff, from: center });
        }
        self.blast_crates(f, center, params);
    }

    /// Knock out every lantern on the ground within `reach` of `center`
    /// (`Event::LanternBroken`).
    pub(super) fn break_lanterns(&mut self, f: &mut Frame, center: Position, reach: f32) {
        let mut broken = Vec::new();
        self.lanterns.retain(|l| {
            let hit = l.position.distance_to(center) <= reach;
            if hit {
                broken.push(l.position);
            }
            !hit
        });
        for at in broken {
            f.events.push(Event::LanternBroken { x: at.x, y: at.y });
        }
    }

    /// Run every grenade within `reach` of `center` out of fuse: each goes
    /// off on its own next resolve, in id order.
    pub(super) fn set_off_grenades(&mut self, center: Position, reach: f32) {
        for grenade in self.world.query_mut::<&mut crate::grenade::Grenade>() {
            if grenade.position.distance_to(center) <= reach {
                grenade.fuse = 0.0;
            }
        }
    }
}

/// Shove `tank` along the unit `dir` as `k` says (`Knock::velocity`, the
/// one mass rule): a real impulse sized by the tank's own mass, so the
/// velocity change is exact. Returns that change; `None` for a hull with
/// no body or a knock that comes to nothing. No skid - `knock_hull` is
/// this and the skid.
pub(super) fn shove(physics: &mut Physics, tank: &Tank, dir: Vec2, k: Knock) -> Option<Vec2> {
    let handle = tank.body?;
    let v = k.velocity(tank.mass_factor());
    if v <= 0.0 {
        return None;
    }
    let dv = dir * v;
    physics.apply_impulse(handle, Position::new(dv.x * tank.mass(), dv.y * tank.mass()));
    Some(dv)
}

/// **The one knock**, with nobody told: `shove`, then - for a knock with a
/// skid - the hull off its tracks (`Tank::skid`, docs/sonic-hammer.md "The
/// knock") for as long as the skid's friction takes to stop it on its
/// ground (`footing`), `k.skid` at most, and the speed it was left at for
/// the pose validator. The velocity change and the skid. What a client's
/// sandbox kicks its own hull with too (`Game::predict_seat_with`).
pub(super) fn knock_hull(physics: &mut Physics, tank: &mut Tank, dir: Vec2, k: Knock, footing: Footing) -> Option<(Vec2, f32)> {
    let dv = shove(physics, tank, dir, k)?;
    if k.skid <= 0.0 {
        return Some((dv, 0.0));
    }
    let handle = tank.body?;
    let v = physics.velocity(handle);
    let rel = Vec2::new(v.x - footing.flow.x, v.y - footing.flow.y).length();
    let skid = (rel / crate::sonic::skid_friction(&tuning(), footing.grip)).min(k.skid);
    tank.skid = tank.skid.max(skid);
    tank.skid_speed = rel;
    Some((dv, skid))
}

/// `knock_hull`, and a client-owned seat told of it (`Event::Shoved` with
/// its skid, through `Shoves`). Every shove in the round goes through
/// here: a shot's impact, a blast, a ram, a hammer's
/// wave, a rod's ring, a well's fling. Returns the velocity change.
pub(super) fn knock(physics: &mut Physics, shoves: &mut Shoves, tank: &mut Tank, dir: Vec2, k: Knock, footing: Footing) -> Vec2 {
    let Some((dv, skid)) = knock_hull(physics, tank, dir, k, footing) else { return Vec2::zero() };
    shoves.push_knock(tank.owner(), dv, skid);
    dv
}

/// Ram contact between two live tanks on opposing sides: both take
/// RAM_DAMAGE_MIN..MAX once per RAM_DAMAGE_COOLDOWN (either tank still
/// cooling down blocks the whole exchange), and both are shoved apart along
/// the line between their centers, harder the faster they were closing,
/// split by mass so the lighter one flies further. A wreck on either side
/// means no exchange at all - a burning hulk neither deals nor takes ram
/// damage. Called for every touching pair: each enemy against each player,
/// enemy against enemy, and the two players against each other. The roll
/// is scaled by `damage_scale` after it is drawn (1 for opposing sides,
/// `friendly_fire_damage_factor` for the two players). A tank this kills
/// is recorded in `kills` with its owner, and each shove goes on `shoves`
/// (kept only for a client-owned seat). Returns the damage dealt, or
/// `None` when nothing was exchanged.
///
/// `physics_velocity`: a tank's actual body velocity, or zero if it has no
/// body yet (a wave tank still rolling in through a gate).
fn physics_velocity(physics: &Physics, tank: &Tank) -> Vec2 {
    match tank.body {
        Some(body) => {
            let v = physics.velocity(body);
            Vec2::new(v.x, v.y)
        }
        None => Vec2::new(0.0, 0.0),
    }
}

pub(super) fn ram(
    a: &mut Tank,
    b: &mut Tank,
    physics: &mut Physics,
    rng: &mut RoundRng,
    kills: &mut Vec<(Position, Owner)>,
    shoves: &mut Shoves,
    damage_scale: f32,
) -> Option<f32> {
    if a.is_wreck() || b.is_wreck() || a.ram_cooldown > 0.0 || b.ram_cooldown > 0.0 {
        return None;
    }
    let dmg = roll_range(rng, tuning().ram_damage_min, tuning().ram_damage_max) * damage_scale;
    // `Event::Ram` is the exchange's report (`Report::Own`).
    let mut unreported = Vec::new();
    let (at_a, at_b) = (a.position, b.position);
    hurt(a, &Hurt::own(dmg, Some(b.owner()), at_a), &mut unreported, kills);
    hurt(b, &Hurt::own(dmg, Some(a.owner()), at_b), &mut unreported, kills);
    a.ram_cooldown = tuning().ram_damage_cooldown;
    b.ram_cooldown = tuning().ram_damage_cooldown;

    let dx = a.position.x - b.position.x;
    let dy = a.position.y - b.position.y;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist <= 0.001 {
        return Some(dmg);
    }
    let axis = Vec2::new(dx / dist, dy / dist);
    // Real closing speed off the bodies, not the commanded `Tank::velocity`.
    // The commanded value is a fixed-magnitude cardinal vector, so it says
    // "both are driving" rather than "how hard they hit": a tank shoved
    // backwards into another by a blast read as not closing at all, and one
    // grinding into a wall read as travelling at full speed. Only the
    // component along the contact axis counts - two tanks sliding past each
    // other are not colliding.
    let (av, bv) = (physics_velocity(physics, a), physics_velocity(physics, b));
    let closing = (av.x - bv.x) * -axis.x + (av.y - bv.y) * -axis.y;
    let impact_speed = closing.max(0.0);
    let push = (impact_speed * tuning().knockback_strength).min(tuning().knockback_max_speed);
    let total_mass = a.mass() + b.mass();
    // A tank this very hit just killed stays put, like any wreck.
    let t = tuning();
    if !a.is_wreck() {
        knock(physics, shoves, a, axis, Knock::ram(&t, push * 2.0 * b.mass() / total_mass), Footing::DRY);
    }
    if !b.is_wreck() {
        knock(physics, shoves, b, Vec2::new(-axis.x, -axis.y), Knock::ram(&t, push * 2.0 * a.mass() / total_mass), Footing::DRY);
    }
    Some(dmg)
}

/// A landing spot for the bong's evasive hop, up to `distance` px away
/// along `away_dir` (only its angle matters).
///
/// The search is a ladder, widest leap first: for each length in
/// BONG_HOP_DISTANCE_STEPS, every offset in BONG_HOP_ANGLE_FAN_DEG off the
/// ideal dead-away angle (plus a little jitter so hops don't all look
/// mechanically identical), taking the first candidate inside the
/// battlefield (`bong_hop_bounds_margin`) that the bong's own hull fits at
/// (`Terrain::bong_fits`). So a cornered bong shortens and angles its hop
/// rather than sitting still, and the fan stops at a quarter turn either
/// way - past that a "hop away" would carry it back toward the threat.
/// `None` only when the bong really is boxed in; the hop is best-effort.
///
/// Bongs love water (docs/water.md): the ladder runs twice, first taking
/// only a landing in water, then any landing. One jitter draw either way,
/// so a map without water hops exactly as before.
pub(super) fn bong_hop_target(
    rng: &mut RoundRng,
    bong_pos: Position,
    away_dir: Vec2,
    distance: f32,
    terrain: &Terrain,
    width: f32,
    height: f32,
) -> Option<Position> {
    let jitter = roll_range(rng, -tuning().bong_hop_angle_jitter_deg, tuning().bong_hop_angle_jitter_deg).to_radians();
    let base_angle = math::atan2(away_dir.y, away_dir.x) + jitter;
    let margin = tuning().bong_hop_bounds_margin;
    for wet_only in [true, false] {
        for step in BONG_HOP_DISTANCE_STEPS {
            for offset_deg in BONG_HOP_ANGLE_FAN_DEG {
                let angle = base_angle + offset_deg.to_radians();
                let reach = distance * step;
                let candidate = Position::new(bong_pos.x + math::cos(angle) * reach, bong_pos.y + math::sin(angle) * reach);
                let in_bounds = candidate.x >= margin
                    && candidate.x <= width - margin
                    && candidate.y >= margin
                    && candidate.y <= height - margin;
                let wet = terrain.depth_at(candidate).is_wet();
                if in_bounds && (wet || !wet_only) && terrain.bong_fits(candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod ram_tests {
    use super::*;
    use rand::SeedableRng;

    fn tank_at(x: f32, damage: f32, physics: &mut Physics) -> Tank {
        let mut tank = Tank {
            position: Position::new(x, 100.0),
            damage,
            ..Tank::default()
        };
        tank.body = Some(physics.spawn_tank(tank.position, tank.move_half_extents(false), tank.mass()));
        tank
    }

    #[test]
    fn the_bong_hops_into_water_when_the_fan_reaches_any() {
        let world = hecs::World::new();
        let (w, h) = (1280.0, 720.0);
        // A pond east of the bong, just inside the widest leap.
        let pond: Vec<Position> = (14..=16).flat_map(|c| (10..=12).map(move |r| crate::map::cell_to_world(c, r))).collect();
        let water = crate::ground::WaterLayout::build(w, h, &[], &pond);
        let terrain = Terrain::build(&world, w, h, &[], &water);
        let bong = crate::map::cell_to_world(11, 11);
        let away = Vec2::new(1.0, 0.0);
        let mut rng = RoundRng::seed_from_u64(3);
        let landing = bong_hop_target(&mut rng, bong, away, 96.0, &terrain, w, h).expect("a landing");
        assert_ne!(water.depth_at(landing), crate::ground::Depth::Dry, "landed in the pond: {landing:?}");
        // Threat from the east: hopping west finds no water in the fan, so
        // it lands on dry ground as before.
        let mut rng = RoundRng::seed_from_u64(3);
        let landing = bong_hop_target(&mut rng, bong, Vec2::new(-1.0, 0.0), 96.0, &terrain, w, h).expect("a landing");
        assert_eq!(water.depth_at(landing), crate::ground::Depth::Dry);
        assert!(landing.x < bong.x);
    }

    #[test]
    fn roll_range_is_min_and_draws_nothing_on_an_empty_or_inverted_range() {
        for (min, max) in [(6.0_f32, 2.0_f32), (4.0, 4.0), (0.0, 0.0), (-0.0, 0.0)] {
            let mut rng = RoundRng::seed_from_u64(9);
            let mut untouched = rng.clone();
            assert_eq!(roll_range(&mut rng, min, max), min, "{min}..{max}");
            assert_eq!(rng.random_range(0..u32::MAX), untouched.random_range(0..u32::MAX), "{min}..{max} drew");
        }
        let blast = BlastParams::world(10.0, (8.0, 3.0), 0.0);
        assert_eq!(blast.roll_damage(&mut RoundRng::seed_from_u64(9)), 8.0);
    }

    #[test]
    fn roll_range_is_the_one_plain_draw_on_a_valid_range() {
        for (seed, min, max) in [(1_u64, 2.0_f32, 6.0_f32), (2, 0.0, 100.0), (0xB0B5, -10.0, 10.0), (4, 3.0, 3.5)] {
            let mut rng = RoundRng::seed_from_u64(seed);
            let mut plain = rng.clone();
            for _ in 0..32 {
                assert_eq!(roll_range(&mut rng, min, max).to_bits(), plain.random_range(min..max).to_bits(), "seed {seed}, {min}..{max}");
            }
            assert_eq!(rng.random_range(0..u32::MAX), plain.random_range(0..u32::MAX), "the streams stay in step");
        }
    }

    /// `bong_hop_angle_jitter_deg` at 0 is a hop straight along `away`.
    #[cfg(feature = "dev-tools")]
    #[test]
    fn a_hop_with_no_angle_jitter_still_lands() {
        let table = std::sync::Arc::new(crate::tuning::Tuning { bong_hop_angle_jitter_deg: 0.0, ..crate::tuning::Tuning::DEFAULT });
        let world = hecs::World::new();
        let (w, h) = (1280.0, 720.0);
        let water = crate::ground::WaterLayout::build(w, h, &[], &[]);
        let terrain = Terrain::build(&world, w, h, &[], &water);
        let bong = crate::map::cell_to_world(11, 11);
        let mut rng = RoundRng::seed_from_u64(3);
        let landing = crate::tuning::in_force(Some(&table), || bong_hop_target(&mut rng, bong, Vec2::new(1.0, 0.0), 96.0, &terrain, w, h));
        assert!(landing.is_some_and(|p| p.x > bong.x), "{landing:?}");
    }

    #[test]
    fn a_wreck_neither_deals_nor_takes_ram_damage() {
        let mut physics = Physics::new();
        let mut wreck = tank_at(100.0, MAX_DAMAGE, &mut physics);
        let mut live = tank_at(130.0, 10.0, &mut physics);
        let mut rng = RoundRng::seed_from_u64(1);
        let mut kills = Vec::new();
        ram(&mut wreck, &mut live, &mut physics, &mut rng, &mut kills, &mut Shoves::default(), 1.0);
        assert_eq!(live.damage, 10.0);
        assert_eq!(live.ram_cooldown, 0.0);
        assert!(kills.is_empty());
    }

    #[test]
    fn live_tanks_exchange_damage_once_per_cooldown() {
        let mut physics = Physics::new();
        let mut a = tank_at(100.0, 0.0, &mut physics);
        let mut b = tank_at(130.0, 0.0, &mut physics);
        let mut rng = RoundRng::seed_from_u64(1);
        let mut kills = Vec::new();
        ram(&mut a, &mut b, &mut physics, &mut rng, &mut kills, &mut Shoves::default(), 1.0);
        assert!(a.damage >= tuning().ram_damage_min && a.damage < tuning().ram_damage_max);
        assert_eq!(a.damage, b.damage);
        assert_eq!(a.ram_cooldown, tuning().ram_damage_cooldown);
        let before = b.damage;
        ram(&mut a, &mut b, &mut physics, &mut rng, &mut kills, &mut Shoves::default(), 1.0);
        assert_eq!(b.damage, before, "second contact inside the cooldown must not re-damage");
    }

    #[test]
    fn a_ram_on_an_owned_seat_logs_its_push_and_nobody_elses() {
        let mut physics = Physics::new();
        let mut seat = tank_at(100.0, 0.0, &mut physics);
        seat.owner = Owner::Player(0);
        let mut enemy = tank_at(130.0, 0.0, &mut physics);
        enemy.owner = Owner::Enemy(3);
        // The seat drives east into the enemy.
        physics.set_velocity(seat.body.expect("a body"), Position::new(120.0, 0.0));
        let mut shoves = Shoves::default();
        shoves.owned[0] = true;
        let mut rng = RoundRng::seed_from_u64(1);
        let mut kills = Vec::new();
        ram(&mut seat, &mut enemy, &mut physics, &mut rng, &mut kills, &mut shoves, 1.0).expect("an exchange");
        assert_eq!(shoves.log.len(), 1, "the enemy's push is nobody's to be told: {:?}", shoves.log);
        let (who, dv, _) = shoves.log[0];
        assert_eq!(who, 0);
        assert!(dv.x < 0.0 && dv.y == 0.0, "the seat is pushed back west, away from the enemy: {dv:?}");
        assert!(-dv.x <= tuning().knockback_max_speed);
    }
}
