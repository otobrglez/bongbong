//! The gauss rail's world half (docs/gauss-rail.md; `gauss.rs` is the
//! picture and the measures). The charge-and-hold pattern's room side,
//! shared by a seat and an enemy (`charge_trigger`): a charge built on the
//! held trigger, fired on its release (`fire_charge`) as a slug queued for
//! `resolve_rails`, which traces it leg by leg across the field once the
//! tank loops are done, through everything in its lane up to the first
//! stopper (`Terrain::pierce_rewound`), knocks the shooter back and puts
//! its picture on (`rail_show`). Also what the AI is handed - the lanes of
//! every charging rail (`Game::rail_lanes`) - and the hold report a room
//! sets on a seat (`Game::set_seat_hold`). No RNG anywhere here but a
//! portal's exit, drawn only where a slug goes into one.

use hecs::Entity;

use crate::frog::Frog;
use crate::gauss::{self, ChargeEndFx, Pierce, Pierced, RailSlug};
use crate::math::Vec2;
use crate::obstacle::{Material, Obstacle};
use crate::shell::Owner;
use crate::shockwave::Shockwave;
use crate::tank::{ActiveWeapon, ChargeEdge, ChargeEnd, ChargeStage, Dir, Tank};
use crate::tuning::tuning;
use crate::{MAX_DAMAGE, Position};

use super::hits::{self, ShellTarget};
use super::props::DamageCause;
use super::{Event, Footing, Frame, Game, HitCause, HitTarget, SHOCK_FROG, Spectacle, laser_reach, portals};

/// A slug released this frame, waiting for `resolve_rails` to trace it.
pub(super) struct PendingRail {
    /// The tank that fired it: knocked back by its recoil.
    pub shooter: Entity,
    pub owner: Owner,
    /// Where it is judged from: the gun line's muzzle.
    pub start: Position,
    /// Where it is drawn from: the rail module's muzzle.
    pub muzzle: Position,
    /// The cardinal it flies along.
    pub dir: Vec2,
    pub overcharged: bool,
}

/// A charging rail's lane this frame (`Game::rail_lanes`): from its gun
/// line's muzzle along its facing to where its slug would stop, as it
/// stands - what the AI keeps out of and routes round.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RailLane {
    /// The charging tank's owner slot.
    pub slot: usize,
    /// The charging hull.
    pub at: Position,
    pub from: Position,
    pub dir: Dir,
    pub length: f32,
    /// How far across its line a hull's centre may stand and be pierced.
    pub half_width: f32,
    pub seat: Option<u8>,
}

impl RailLane {
    /// How far `p` stands inside it, across its line; negative outside -
    /// beside it, behind its muzzle or past its stop.
    pub fn depth(&self, p: Position) -> f32 {
        let d = self.dir.vec();
        let rel = p - self.from;
        let along = rel.x * d.x + rel.y * d.y;
        let across = (rel.x * d.y - rel.y * d.x).abs();
        if along < 0.0 {
            return along;
        }
        if along > self.length {
            return self.length - along;
        }
        self.half_width - across
    }
}

/// A seat as a rail-carrying enemy sees it this frame (`Game::gauss_senses`).
#[derive(Clone, Copy, Debug)]
pub(super) struct GaussSeat {
    pub seat: u8,
    pub entity: Entity,
    pub pos: Position,
    /// On the field and not a wreck.
    pub live: bool,
    /// In tall grass that hides it.
    pub concealed: bool,
    /// How far an enemy sees it under the sky (`Game::sight_on`).
    pub sight: f32,
}

/// The room side of a charge-and-hold trigger (docs/gauss-rail.md "The
/// charge-and-hold pattern"), one tick of it for the tank `entity`: the
/// charge stepped (`Tank::step_charge`, the gate its `fire_cooldown`), and
/// what the step did acted on - a start logged, a release fired
/// (`fire_charge`), a vent cooling the weapon, an end logged and drawn.
#[allow(clippy::too_many_arguments)]
pub(super) fn charge_trigger(f: &mut Frame, entity: Entity, tank: &mut Tank, owner: Owner, fire: bool, pressed: bool, report: Option<u32>) {
    let weapon = tank.charge.map_or_else(|| tank.active_weapon(), |c| c.weapon);
    let open = tank.fire_cooldown <= 0.0;
    match tank.step_charge(fire, pressed, f.dt, open, report) {
        ChargeEdge::None | ChargeEdge::Held => {}
        ChargeEdge::Started => f.events.push(Event::ChargeStarted { slot: tank.owner_slot(), weapon: weapon.name() }),
        ChargeEdge::Released(stage) => fire_charge(f, entity, tank, owner, weapon, stage),
        ChargeEdge::Ended(end) => {
            if end == ChargeEnd::Vented
                && let Some(rule) = weapon.charge_rule()
            {
                tank.fire_cooldown = tank.fire_cooldown.max(rule.vent_cooldown);
            }
            f.events.push(Event::ChargeEnded { slot: tank.owner_slot(), weapon: weapon.name(), end });
            if end != ChargeEnd::Lapsed {
                f.charge_ends.push(ChargeEndFx::new(crate::gauss::muzzle(tank), end));
            }
        }
    }
}

/// Fire a released charge of `weapon` at `stage`: for the gauss rail one
/// slug spent, `gauss_reload_seconds` before the next charge, the module's
/// shot cell, `Event::Fired` and the slug queued for `resolve_rails` along
/// the hull's facing - never off-aim. No RNG.
pub(super) fn fire_charge(f: &mut Frame, entity: Entity, tank: &mut Tank, owner: Owner, weapon: ActiveWeapon, stage: ChargeStage) {
    if weapon != ActiveWeapon::GaussRail || tank.gauss_slugs <= 0 {
        return;
    }
    tank.gauss_slugs -= 1;
    tank.fire_cooldown = tuning().gauss_reload_seconds;
    tank.kick_rail();
    f.events.push(Event::Fired { slot: tank.owner_slot(), weapon: weapon.name() });
    let dir = Dir::from_rotation(tank.rotation).unwrap_or(Dir::Up).vec();
    f.pending_rails.push(PendingRail {
        shooter: entity,
        owner,
        start: tank.gun_line_muzzle(dir),
        muzzle: crate::gauss::muzzle(tank),
        dir,
        overcharged: stage == ChargeStage::Overcharged,
    });
}

/// A slug's recoil on the hull that fired it along `dir`
/// (`gauss::recoil_speed`): a knock back the other way - a skid, the same
/// slide whichever way the hull faces - and, for an overcharged one, the
/// hull spun round, its facing turned half a turn and the stick locked
/// for as long as it skids (`Tank::spin`). The velocity change and the
/// skid. The room's (`Game::rail_recoil`) and a client's sandbox's on its
/// own release (`Game::predict_seat_with`) alike.
pub(super) fn recoil_hull(physics: &mut crate::physics::Physics, tank: &mut Tank, dir: Vec2, overcharged: bool, footing: Footing) -> Option<(Vec2, f32)> {
    let t = tuning();
    let mass = t.tank_mass_factor[tank.row.clamp(0, 11) as usize];
    let speed = gauss::recoil_speed(&t, mass, overcharged, crate::sonic::skid_friction(&t, 1.0));
    let knocked = super::sonic::knock_hull(physics, tank, dir * -1.0, speed, footing)?;
    if overcharged {
        tank.rotation = (tank.rotation + 180.0).rem_euclid(360.0);
        tank.spin = tank.spin.max(tank.skid);
        if let Some(handle) = tank.body {
            let half = tank.move_half_extents(tank.facing_along_x());
            physics.resize_collider(physics.collider_of(handle), half);
        }
    }
    Some(knocked)
}

/// What one predicted tick of a seat's charge did
/// (`Game::predict_seat_with`): the step's edge, and where the slug would
/// leave from as the hull stood before its recoil - judged from (`start`),
/// drawn from (`muzzle`), along `dir`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SeatCharge {
    pub edge: ChargeEdge,
    pub start: Position,
    pub muzzle: Position,
    pub dir: Vec2,
}

impl Game {
    /// Trace this frame's slugs, leg by leg: everything each leg goes
    /// through takes its share in order (`pierce_hit`), the leg is logged
    /// (`Event::RailSlug`) and drawn (`rail_show`), and a leg ending in a
    /// portal goes on from another, the exit one draw from the round's RNG
    /// (`draw_shot_exit`); then the shooter is knocked back (and spun round
    /// by an overcharged one). Also puts this frame's charge ends on the
    /// field. A seat's slug is judged against the enemies as its client
    /// drew them (`seat_rewind`), as the laser is.
    pub(super) fn resolve_rails(&mut self, f: &mut Frame) {
        for fx in f.charge_ends.drain(..) {
            self.charge_ends.push(fx);
        }
        let shots = std::mem::take(&mut f.pending_rails);
        if shots.is_empty() {
            return;
        }
        let t = tuning();
        let players = self.seats_on_field();
        let reach = laser_reach(self.map.field_size());
        let (portal_radius, max_passes) = (t.portal_shot_radius, t.portal_shot_max_passes.max(0) as u8);
        let half = t.gauss_half_width;
        for shot in shots {
            let rewind = self.seat_rewind(shot.owner);
            let full_damage = gauss::damage(shot.owner, &t);
            let mut damage = full_damage;
            let (mut from, mut drawn, mut left) = (shot.start, shot.muzzle, reach);
            let mut leaving = portals::inside(from, self.shot_portals(), portal_radius);
            let mut leg = 0u8;
            loop {
                let end = from + shot.dir * left;
                let entry = if leg < max_passes { portals::shot_entry(from, end, self.shot_portals(), portal_radius, leaving) } else { None };
                let stop = entry.map_or(end, |(_, at)| at);
                let hits = {
                    let past = self.rewound_boxes(rewind);
                    f.terrain.pierce_rewound(&self.world, players, shot.owner, from, stop, half, past, !shot.overcharged)
                };
                let mut pierced: Vec<Pierce> = Vec::new();
                let mut stopped: Option<Position> = None;
                for (target, tt) in hits {
                    let at = from + (stop - from) * tt;
                    if self.stops_slug(f, target, shot.overcharged) {
                        let hit = match target {
                            ShellTarget::Obstacle(e) => f.terrain.obstacle(e).map(|b| HitTarget::Obstacle { material: b.material }),
                            _ => Some(HitTarget::Wall),
                        };
                        if let Some(target) = hit {
                            f.events.push(Event::Hit { target, damage: 0.0, killed: false, x: at.x, y: at.y, cause: HitCause::Rail });
                        }
                        stopped = Some(at);
                        break;
                    }
                    if let Some(what) = self.pierce_hit(f, target, at, &mut damage, full_damage, shot.owner, shot.dir) {
                        pierced.push(Pierce { at, what });
                    }
                }
                let through = stopped.is_none() && entry.is_some();
                let leg_end = stopped.unwrap_or(stop);
                f.events.push(Event::RailSlug {
                    slot: shot.owner.slot(),
                    seat: crate::net::encode::owner_seat(shot.owner),
                    leg,
                    x0: drawn.x,
                    y0: drawn.y,
                    x1: leg_end.x,
                    y1: leg_end.y,
                    portal: through,
                    overcharged: shot.overcharged,
                    pierced: pierced.iter().map(|p| (p.at.x, p.at.y, p.what)).collect(),
                });
                let mut show = Spectacle::default();
                self.rail_show(&mut show, RailSlug::new(drawn, leg_end, through, shot.overcharged, pierced), leg == 0);
                f.stage(show);
                if let (true, Some((entrance, at))) = (through, entry) {
                    let exit = self.draw_shot_exit(f, entrance);
                    let out = portals::exit_point(self.portals[entrance], self.portals[exit], at, shot.dir);
                    f.events.push(Event::ShotTeleported { id: None, x: at.x, y: at.y, to_x: out.x, to_y: out.y });
                    left -= (at - from).length();
                    (from, drawn) = (out, out);
                    leaving = Some(exit);
                    leg += 1;
                    continue;
                }
                break;
            }
            self.rail_recoil(f, shot.shooter, shot.dir, shot.overcharged);
        }
    }

    /// Whether `target` stops a slug: the field's edge, or a permanent tile
    /// (a volcano's cone, a training door, and iron unless the slug is
    /// overcharged).
    fn stops_slug(&self, f: &Frame, target: ShellTarget, overcharged: bool) -> bool {
        match target {
            ShellTarget::Wall => true,
            ShellTarget::Obstacle(e) => {
                f.terrain.obstacle(e).is_some_and(|b| b.material.is_permanent() && (!overcharged || b.material != Material::Iron))
            }
            ShellTarget::Tank(_) | ShellTarget::Frog(_) => false,
        }
    }

    /// One thing a slug goes through, at `at`, with the slug's `damage` so
    /// far (`full` at its start): a tank takes it whole through
    /// `take_damage` - a seat's slug through a teammate scaled by
    /// `friendly_fire_damage_factor`, a rainbow shield soaking it - and the
    /// slug keeps `gauss_pierce_keep` of it; a frog takes
    /// `gauss_frog_damage` times what the slug has kept, and the same;
    /// a tower takes it (`DamageCause::Pierce`), and the same; every other
    /// tile dies outright and the slug keeps `gauss_tile_keep` (iron an
    /// overcharged slug cuts is untouched; a tile already burning or with
    /// a fuse lit is passed through untouched). No knockback, no hop, no
    /// roll. What it was, for the picture; `None` for a wreck, a dead frog
    /// and a tile already gone.
    #[allow(clippy::too_many_arguments)]
    fn pierce_hit(&mut self, f: &mut Frame, target: ShellTarget, at: Position, damage: &mut f32, full: f32, shooter: Owner, dir: Vec2) -> Option<Pierced> {
        let t = tuning();
        match target {
            ShellTarget::Tank(entity) => {
                let (what, survived) = {
                    let mut q = self.world.query_one::<&mut Tank>(entity);
                    let tank = q.get().ok()?;
                    if tank.is_wreck() {
                        return None;
                    }
                    let what = if tank.is_shielded() { Pierced::Shield } else { Pierced::Tank };
                    let mut d = *damage;
                    if shooter.is_player() && tank.owner().is_player() {
                        d *= t.friendly_fire_damage_factor;
                    }
                    let landed = tank.take_damage(d, MAX_DAMAGE);
                    tank.mark_hit();
                    tank.credit(shooter);
                    let killed = tank.is_wreck();
                    let hit_target = match tank.owner() {
                        Owner::Player(player) => HitTarget::Player { player },
                        Owner::Enemy(slot) => HitTarget::Enemy { slot },
                        Owner::Tower { .. } => unreachable!("no tank is owned by a tower"),
                    };
                    f.events.push(Event::Hit { target: hit_target, damage: landed, killed, x: at.x, y: at.y, cause: HitCause::Rail });
                    if killed {
                        f.kills.push((tank.position, tank.owner()));
                    }
                    (what, !killed)
                };
                if survived && let Ok(ai) = self.world.query_one_mut::<&mut crate::ai::Ai>(entity) {
                    ai.notify_hit();
                }
                *damage *= t.gauss_pierce_keep;
                Some(what)
            }
            ShellTarget::Frog(entity) => {
                let keep = if full > 0.0 { *damage / full } else { 0.0 };
                let (dead, pos) = {
                    let mut q = self.world.query_one::<&mut Frog>(entity);
                    let frog = q.get().ok()?;
                    if frog.is_dead() {
                        return None;
                    }
                    let d = t.gauss_frog_damage * keep;
                    frog.damage(d);
                    f.events.push(Event::Hit { target: HitTarget::Frog { side: frog.side }, damage: d, killed: frog.is_dead(), x: at.x, y: at.y, cause: HitCause::Rail });
                    (frog.is_dead(), frog.position)
                };
                if dead {
                    f.shocks.push(Shockwave::scaled(pos, SHOCK_FROG));
                }
                *damage *= t.gauss_pierce_keep;
                Some(Pierced::Frog)
            }
            ShellTarget::Obstacle(entity) => {
                let (material, standing) = {
                    let o = self.world.get::<&Obstacle>(entity).ok()?;
                    if o.destroyed {
                        return None;
                    }
                    (o.material, !o.burning && o.fuse.is_none())
                };
                if material == Material::Iron {
                    f.events.push(Event::Hit { target: HitTarget::Obstacle { material }, damage: 0.0, killed: false, x: at.x, y: at.y, cause: HitCause::Rail });
                    return Some(Pierced::Tile(material));
                }
                if !standing {
                    return Some(Pierced::Tile(material));
                }
                let mark = f.events.len();
                let d = *damage;
                let killed = self.damage_obstacle(f, entity, d, DamageCause::Pierce { dir });
                f.events.insert(mark, Event::Hit { target: HitTarget::Obstacle { material }, damage: d, killed, x: at.x, y: at.y, cause: HitCause::Rail });
                *damage *= if material.is_tower() { t.gauss_pierce_keep } else { t.gauss_tile_keep };
                Some(Pierced::Tile(material))
            }
            ShellTarget::Wall => None,
        }
    }

    /// Knock `shooter` back from its slug along `-dir`
    /// (`gauss::recoil_speed`, a skid: the same slide whichever way the
    /// hull faces), and spin an overcharged one round - its facing turned
    /// half a turn, the stick locked for as long as it skids. A seat's own
    /// recoil is not sent as a `Shoved` - its client kicks it on the
    /// release - but the pose validator still allows the knock's speed.
    fn rail_recoil(&mut self, f: &mut Frame, shooter: Entity, dir: Vec2, overcharged: bool) {
        let (water, lava, weather, time) = (&self.water, &self.lava, self.weather, self.time);
        let Ok(mut tank) = self.world.get::<&mut Tank>(shooter) else { return };
        if tank.is_wreck() {
            return;
        }
        let footing = Footing::at(water, lava, weather, tank.position, time);
        if let Some((dv, skid)) = recoil_hull(&mut self.physics, &mut tank, dir, overcharged, footing) {
            f.shoves.allow_knock(tank.owner(), dv, skid);
        }
    }

    /// One leg of a slug as it is drawn - a round's, a replica's off
    /// `Event::RailSlug`, a client's own release: the leg on
    /// `rail_slugs`, the tall grass along it laid flat, and on the first
    /// leg the ripple and the module's flash at the muzzle. The shows of
    /// what it went through are its own picture's (`gauss::compose_slug`).
    pub(crate) fn rail_show(&mut self, show: &mut Spectacle, slug: RailSlug, first_leg: bool) {
        let t = tuning();
        if first_leg {
            show.muzzle_flashes.push(Shockwave::new(slug.start));
            if t.gauss_shock > 0.0 {
                show.shocks.push(Shockwave::scaled(slug.start, t.gauss_shock));
            }
        }
        crate::grass::flatten_along(&mut self.grass, slug.start, slug.end, gauss::GRASS_REACH_PX);
        self.rail_slugs.push(slug);
    }

    /// A charge's end as it is drawn at `at` (a replica's off
    /// `Event::ChargeEnded`, a client's own): the fizzle or the vent. A
    /// lapse draws nothing.
    pub(crate) fn charge_end_show(&mut self, at: Position, end: ChargeEnd) {
        if end != ChargeEnd::Lapsed {
            self.charge_ends.push(ChargeEndFx::new(at, end));
        }
    }

    /// Whether some live tank charges a gauss rail: what the lanes, their
    /// dangers and their surcharge are built for.
    pub(crate) fn any_rail_charging(&self) -> bool {
        self.world.query::<&Tank>().iter().any(|t| !t.is_wreck() && t.charge.is_some_and(|c| c.weapon == ActiveWeapon::GaussRail))
    }

    /// The lane of every live tank on the field charging a rail, seats in
    /// index order then enemies by slot: from its gun line's muzzle along
    /// its facing to where its slug would stop with the stage it has now -
    /// the first cell holding a permanent tile (iron only while it is not
    /// overcharged) or the field's edge - as wide as any hull centred in
    /// it could be pierced. Empty when nothing charges.
    pub(crate) fn rail_lanes(&self) -> Vec<RailLane> {
        let t = tuning();
        let (width, height) = self.map.field_size();
        let half_width = crate::battlefield::max_tank_clearance_half_extent() + t.gauss_half_width;
        let permanent: std::collections::BTreeMap<(i32, i32), Material> = self
            .world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| !o.destroyed && o.material.is_permanent())
            .map(|o| (o.cell(), o.material))
            .collect();
        let mut lanes: Vec<RailLane> = self
            .world
            .query::<&Tank>()
            .iter()
            .filter(|tank| tank.charge.is_some_and(|c| c.weapon == ActiveWeapon::GaussRail) && !tank.is_wreck() && tank.body.is_some())
            .map(|tank| {
                let dir = Dir::from_rotation(tank.rotation).unwrap_or(Dir::Up);
                let from = tank.gun_line_muzzle(dir.vec());
                let overcharged = tank.charge.is_some_and(|c| c.stage() == ChargeStage::Overcharged);
                let step = crate::OBSTACLE_GRID_SIZE * 0.25;
                let mut length = 0.0;
                loop {
                    let p = from + dir.vec() * length;
                    if p.x < 0.0 || p.y < 0.0 || p.x > width || p.y > height {
                        break;
                    }
                    if permanent.get(&crate::map::world_to_cell(p)).is_some_and(|&m| !overcharged || m != Material::Iron) {
                        break;
                    }
                    length += step;
                }
                RailLane { slot: tank.owner_slot(), at: tank.position, from, dir, length, half_width, seat: tank.player_index() }
            })
            .collect();
        lanes.sort_by_key(|l| (l.seat.is_none(), l.slot));
        lanes
    }

    /// The dangers every enemy keeps out of because a rail charges
    /// (docs/gauss-rail.md "Reacting to a rail"): a lane each, owned by the
    /// charging tank - a seat's every enemy steps out of, an enemy's its
    /// allies do.
    pub(super) fn rail_dangers(&self) -> Vec<crate::ai::Danger> {
        self.rail_lanes()
            .into_iter()
            .map(|l| crate::ai::Danger {
                shape: crate::ai::DangerShape::Lane { at: l.at, from: l.from, dir: l.dir, length: l.length, half_width: l.half_width },
                owner: Some(l.slot),
                slack: 0.0,
            })
            .collect()
    }

    /// What a slug of each rail-carrying enemy's own would go through each
    /// way it could face (docs/gauss-rail.md "What it is handed"), from the
    /// gun line's muzzle that facing gives, by the room's own trace
    /// (`Terrain::pierce_rewound`, iron stopping it), for the tanks that
    /// think this tick (`thinks`) - no other reads its sense. A seat counts
    /// only where this tank stands in its sight box, it is within this
    /// tank's sight under the sky and not hidden from it.
    ///
    /// A lane is traced only where a target could be on it: one holding
    /// neither a seat that counts nor, for a hunter, the players' frog is
    /// worth nothing whatever else it would go through (`GaussLane::counts`),
    /// so it is left `GaussLane::default` unless the box of one of those
    /// reaches its band (`SlugBand`, the test the trace itself holds every
    /// box to first), and a tank with none of them anywhere traces nothing.
    pub(super) fn gauss_senses(
        &self,
        terrain: &hits::Terrain,
        seats: &[GaussSeat],
        thinks: impl Fn(&Tank, &crate::ai::Ai) -> bool,
    ) -> std::collections::BTreeMap<Entity, crate::ai::GaussSense> {
        use crate::ai::{Ai, GaussLane, GaussSense};
        let t = tuning();
        let mut out = std::collections::BTreeMap::new();
        let reach = laser_reach(self.map.field_size());
        let players = self.seats_on_field();
        let (half_w, half_h) = t.sight_box_half_px();
        let pad = Position::new(t.gauss_half_width, t.gauss_half_width);
        // Each seat's boxes as a trace grows them, read once for every tank.
        let seat_boxes: Vec<[(Position, Position); 2]> =
            seats.iter().map(|s| super::with_tank(&self.world, s.entity, |tank| hits::slug_boxes(tank, pad))).collect();
        let quarry_box = self.frog.and_then(|frog| terrain.frog_box(frog, t.gauss_half_width));
        let armed: Vec<(Entity, Owner, Position)> = self
            .world
            .query::<(Entity, &Tank, &Ai)>()
            .iter()
            .filter(|(_, tank, ai)| !tank.is_wreck() && tank.body.is_some() && tank.active_weapon() == ActiveWeapon::GaussRail && thinks(tank, ai))
            .map(|(e, tank, _)| (e, tank.owner(), tank.position))
            .collect();
        for (entity, owner, me) in armed {
            let Ok(ai) = self.world.get::<&Ai>(entity) else { continue };
            let hunter = ai.role == crate::ai::Role::Hunter;
            let Ok(tank) = self.world.get::<&Tank>(entity) else { continue };
            // Whether a seat counts for this tank does not hang on the lane.
            let counts = |seat: &GaussSeat| {
                seat.live
                    && !ai.frog_only
                    && crate::ai::in_sight_box_of((half_w, half_h), seat.pos, me)
                    && me.distance_to(seat.pos) <= seat.sight
                    && !(seat.concealed && !ai.is_hit_alerted())
            };
            let targets: Vec<(Position, Position)> = seats
                .iter()
                .zip(&seat_boxes)
                .filter(|(seat, _)| counts(seat))
                .flat_map(|(_, boxes)| boxes.iter().copied())
                .chain(quarry_box.filter(|_| hunter))
                .collect();
            let mut sense = GaussSense::default();
            if targets.is_empty() {
                out.insert(entity, sense);
                continue;
            }
            for dir in Dir::ALL {
                let from = tank.gun_line_muzzle(dir.vec());
                let to = from + dir.vec() * reach;
                let band = hits::SlugBand::of(from, to, t.gauss_half_width);
                if !targets.iter().any(|&(c, h)| band.reaches(c, h)) {
                    continue;
                }
                let mut lane = GaussLane::default();
                for (target, tt) in terrain.pierce_rewound(&self.world, players, owner, from, to, t.gauss_half_width, None, true) {
                    let along = tt * reach;
                    match target {
                        ShellTarget::Tank(e) => match seats.iter().find(|s| s.entity == e) {
                            Some(seat) => {
                                if counts(seat) {
                                    lane.seats += 1;
                                    if lane.at_seat.is_none() {
                                        lane.at_seat = Some((seat.seat, along));
                                        let m = t.gauss_ai_box_margin_px;
                                        lane.settled = crate::ai::in_sight_box_of((half_w - m, half_h - m), seat.pos, me)
                                            && me.distance_to(seat.pos) <= seat.sight - m;
                                    }
                                }
                            }
                            None => {
                                lane.friend.get_or_insert(along);
                            }
                        },
                        ShellTarget::Frog(e) => {
                            if Some(e) == self.enemy_frog {
                                lane.friend.get_or_insert(along);
                            } else if hunter && Some(e) == self.frog && lane.quarry.is_none() {
                                lane.quarry = Some(along);
                            }
                        }
                        ShellTarget::Obstacle(e) => {
                            if let Ok(o) = self.world.get::<&Obstacle>(e)
                                && o.material.is_tower()
                                && !o.destroyed
                            {
                                if crate::tower::side_of_variant(o.variant) == crate::frog::Side::Player {
                                    lane.towers += 1;
                                } else {
                                    lane.friend.get_or_insert(along);
                                }
                            }
                        }
                        ShellTarget::Wall => {}
                    }
                }
                sense.lanes[dir.index()] = lane;
            }
            out.insert(entity, sense);
        }
        out
    }

    /// A room's count of the ticks a seat's client has held its trigger
    /// (`net::mailbox::Mailbox::hold_ticks`), for this update alone: the
    /// charge counts the client's ticks, held to its own within
    /// `CHARGE_HOLD_SPARE_TICKS` (docs/gauss-rail.md "The hold report").
    pub fn set_seat_hold(&mut self, seat: usize, ticks: Option<u32>) {
        if let Some(slot) = self.seat_hold.get_mut(seat) {
            *slot = ticks.map(|n| (self.frame + 1, n));
        }
    }

    /// This update's hold report for `seat`, if a room set one.
    pub(super) fn seat_hold_report(&self, seat: usize) -> Option<u32> {
        self.seat_hold.get(seat).copied().flatten().filter(|&(frame, _)| frame == self.frame).map(|(_, n)| n)
    }
}
