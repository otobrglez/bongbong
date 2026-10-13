//! Air support's world half (docs/air-support.md; `air_support.rs` is the
//! call, the aircraft and their geometry). A crate calls a sortie
//! (`call_air_support`, from `pickup_phase`); `air_support_phase` brings the
//! aircraft in when its count runs out, flies it, fires it and takes it away
//! once it is off the field. The helicopter fires seeker missiles off its
//! pods (`missile.rs`, every rule of theirs kept); the jet's cannon and bombs
//! are blasts of their own (`air_strike`). No RNG but the blasts' damage
//! rolls, which only a round with an air crate taken ever draws.

use hecs::Entity;

use crate::ai::{Ai, in_sight_box};
use crate::air::AirStrike;
use crate::air_support::{self, AirCall, AirStage, Aircraft, Sortie};
use crate::blast::{BlastFx, BlastKind, BlastShape, Lean, Scorch};
use crate::bong::Side;
use crate::math::Vec2;
use crate::missile::Missile;
use crate::obstacle::Obstacle;
use crate::shell::Owner;
use crate::shockwave::Shockwave;
use crate::tank::Tank;
use crate::tuning::tuning;
use crate::{OBSTACLE_GRID_SIZE, Position};

use super::combat::BlastParams;
use super::{Event, Frame, Game, RollIn, Spectacle};

/// How hard a bomb shakes the screen, relative to a tank dying.
const SHOCK_BOMB: f32 = 0.6;

/// What an aircraft does this step, decided while the world is read and
/// done once it is free.
enum Act {
    /// The heli's salvo: rockets from its pods at `at`.
    Salvo { from: Position, at: Position },
    /// One of the jet's cannon bursts, or (`bomb`) one of its bombs.
    Strike { at: Position, dir: Vec2, bomb: bool },
}

impl BlastParams {
    /// One burst of the jet's cannon: fixed damage, no roll.
    pub fn air_gun(by: Owner) -> Self {
        let t = tuning();
        BlastParams { radius: t.air_jet_gun_radius_px, damage: (t.air_jet_gun_damage, t.air_jet_gun_damage), knockback: t.air_jet_gun_knockback, by: Some(by) }
    }

    /// One of the jet's bombs.
    pub fn air_bomb(by: Owner) -> Self {
        let t = tuning();
        BlastParams {
            radius: t.air_jet_bomb_radius_px,
            damage: (t.air_jet_bomb_damage_min, t.air_jet_bomb_damage_max),
            knockback: t.air_jet_bomb_knockback,
            by: Some(by),
        }
    }
}

impl Game {
    /// Seat `seat` took an air support crate: put its call on
    /// `air_calls`, counting from now, or waiting behind the seat's sortie
    /// already called or in the air (`air_support.rs` rule 3).
    pub(super) fn call_air_support(&mut self, f: &mut Frame, seat: u8, sortie: Sortie) {
        let id = self.take_shot_id();
        let busy = self.seat_air_busy(seat);
        let arrive_at = (!busy).then(|| self.time + tuning().air_support_delay_seconds);
        self.air_calls.push(AirCall { id, seat, sortie, arrive_at });
        f.events.push(Event::AirSupportCalled { id, seat, sortie, arrive: arrive_at });
    }

    /// Whether `seat` has a sortie counting down or in the air.
    fn seat_air_busy(&self, seat: u8) -> bool {
        self.air_calls.iter().any(|c| c.seat == seat && c.arrive_at.is_some()) || self.world.query::<&Aircraft>().iter().any(|a| a.seat == seat)
    }

    /// Start the count of `seat`'s next waiting call, if it has one and
    /// nothing else of its is counting or in the air.
    fn start_next_call(&mut self, seat: u8) {
        if self.seat_air_busy(seat) {
            return;
        }
        let at = self.time + tuning().air_support_delay_seconds;
        if let Some(call) = self.air_calls.iter_mut().find(|c| c.seat == seat && c.arrive_at.is_none()) {
            call.arrive_at = Some(at);
        }
    }

    /// Where `seat` stands for its aircraft: its tank, a wreck included;
    /// `None` once the seat has no tank at all.
    fn air_anchor(&self, seat: u8) -> Option<Position> {
        let entity = self.seat(seat as usize)?;
        self.world.get::<&Tank>(entity).ok().map(|t| t.position)
    }

    /// What an aircraft of the seat standing at `anchor` may attack, inside
    /// the seat's sight box: the enemy tanks live on the field and the enemy
    /// towers still standing. Nearest first; at the same distance a tank
    /// before a tower, tanks by slot and towers by cell.
    fn air_targets_near(&self, anchor: Position) -> Vec<Position> {
        let mut out: Vec<(f32, u8, usize, (i32, i32), Position)> = self
            .world
            .query::<&Tank>()
            .with::<&Ai>()
            .without::<&RollIn>()
            .iter()
            .filter(|t| !t.is_wreck() && t.body.is_some() && in_sight_box(anchor, t.position))
            .map(|t| (t.position.distance_to(anchor), 0, t.owner_slot(), (0, 0), t.position))
            .collect();
        out.extend(
            self.towers
                .iter()
                .filter(|(_, t)| t.side == Side::Enemy && in_sight_box(anchor, t.position))
                .filter(|(_, t)| self.world.get::<&Obstacle>(t.entity).is_ok_and(|o| !o.destroyed))
                .map(|(&cell, t)| (t.position.distance_to(anchor), 1, 0, cell, t.position)),
        );
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)).then(a.3.cmp(&b.3)));
        out.into_iter().map(|(.., p)| p).collect()
    }

    /// Air support's frame: bring in every aircraft whose count ran out,
    /// fly them all a step, let them fire, and take away the ones gone off
    /// the field. `live` is false on the end screen: no call arrives and an
    /// aircraft in its attack breaks off and leaves, firing nothing.
    pub(super) fn air_support_phase(&mut self, f: &mut Frame, live: bool) {
        if self.air_calls.is_empty() && self.world.query::<&Aircraft>().iter().next().is_none() {
            return;
        }
        if live {
            self.air_arrivals(f);
        }
        let t = tuning();
        let field = (f.width, f.height);
        let mut flying: Vec<(u32, Entity)> = self.world.query::<(Entity, &Aircraft)>().iter().map(|(e, a)| (a.id, e)).collect();
        flying.sort_by_key(|&(id, _)| id);
        let mut gone = Vec::new();
        for (id, entity) in flying {
            let Ok(mut a) = self.world.get::<&mut Aircraft>(entity).map(|a| *a) else { continue };
            a.advance(f.dt, &t);
            if !live && a.stage == AirStage::Attack && a.sortie == Sortie::Heli {
                a.stage = AirStage::Egress;
            }
            let acts = if live { self.air_acts(&mut a, &t) } else { Vec::new() };
            if a.sortie == Sortie::Jet && a.stage != AirStage::Egress && a.flown >= (a.mark - a.entry).length() {
                a.stage = AirStage::Egress;
            }
            let leaving = a.gone(field, &t);
            if let Ok(mut slot) = self.world.get::<&mut Aircraft>(entity) {
                *slot = a;
            }
            for act in acts {
                match act {
                    Act::Salvo { from, at } => self.air_salvo(f, &a, from, at),
                    Act::Strike { at, dir, bomb } => self.air_strike(f, id, a.owner(), at, dir, bomb, true),
                }
            }
            if leaving {
                gone.push((id, entity, a.seat));
            }
        }
        for (id, entity, seat) in gone {
            self.world.despawn(entity).ok();
            f.events.push(Event::AirSupportLeft { id });
            self.start_next_call(seat);
        }
    }

    /// Every call whose count ran out, by id: its aircraft comes in.
    fn air_arrivals(&mut self, f: &mut Frame) {
        let now = self.time;
        let due: Vec<AirCall> = self.air_calls.iter().filter(|c| c.arrive_at.is_some_and(|at| at <= now)).copied().collect();
        if due.is_empty() {
            return;
        }
        self.air_calls.retain(|c| !c.arrive_at.is_some_and(|at| at <= now));
        let t = tuning();
        let field = (f.width, f.height);
        for call in due {
            let anchor = self.air_anchor(call.seat);
            let targets = anchor.map(|p| self.air_targets_near(p)).unwrap_or_default();
            let Some(anchor) = anchor else {
                // The seat is gone from the round: the call is spent.
                f.events.push(Event::AirSupportLeft { id: call.id });
                continue;
            };
            let armed = !targets.is_empty();
            let edge = air_support::entry_edge(call.id);
            let (entry, mark) = match call.sortie {
                Sortie::Heli => {
                    let first = targets.first().copied().unwrap_or(anchor);
                    let entry = air_support::entry_point(edge, first, field, t.air_edge_margin_px);
                    let standoff = if armed { t.air_heli_standoff_px } else { 0.0 };
                    (entry, air_support::hover_point(entry, first, standoff, field, OBSTACLE_GRID_SIZE))
                }
                Sortie::Jet => {
                    let mark = air_support::run_mark(&targets, t.air_jet_targets.max(1) as usize).unwrap_or(anchor);
                    (air_support::entry_point(air_support::jet_edge(call.id, mark, field), mark, field, t.air_edge_margin_px), mark)
                }
            };
            let aircraft = Aircraft::arrive(call.id, call.seat, call.sortie, entry, mark, armed, &t);
            f.events.push(Event::AirSupportArrived { id: call.id, seat: call.seat, sortie: call.sortie, x: entry.x, y: entry.y, armed });
            self.world.spawn((aircraft,));
        }
    }

    /// What aircraft `a` fires this step, its stage moved on as it does.
    fn air_acts(&self, a: &mut Aircraft, t: &crate::tuning::Tuning) -> Vec<Act> {
        let mut acts = Vec::new();
        match a.sortie {
            Sortie::Heli => {
                if a.stage != AirStage::Attack {
                    return acts;
                }
                if !a.armed {
                    if a.stage_time >= t.air_heli_linger_seconds {
                        a.stage = AirStage::Egress;
                    }
                    return acts;
                }
                let next = t.air_heli_salvo_seconds * (a.fired as f32 + 1.0);
                if a.stage_time < next {
                    return acts;
                }
                let anchor = self.air_anchor(a.seat).unwrap_or(a.mark);
                match self.air_targets_near(anchor).first().copied() {
                    Some(at) => {
                        if let Some(dir) = air_support::unit(at - a.ground) {
                            a.heading = dir;
                        }
                        acts.push(Act::Salvo { from: a.drawn(), at });
                        a.fired = a.fired.saturating_add(1);
                        if a.fired as i32 >= t.air_heli_salvos {
                            a.stage = AirStage::Egress;
                        }
                    }
                    None => a.stage = AirStage::Egress,
                }
            }
            Sortie::Jet => {
                if !a.armed {
                    return acts;
                }
                let n = t.air_jet_gun_bursts.clamp(1, 255) as u8;
                let run = a.run();
                while a.fired < n {
                    let (at, when) = air_support::gun_burst(a.entry, a.mark, a.fired, n, t.air_jet_gun_len_px, t.air_jet_gun_ahead_px, t.air_edge_margin_px);
                    if a.flown < when {
                        break;
                    }
                    acts.push(Act::Strike { at, dir: run, bomb: false });
                    a.fired += 1;
                    a.stage = AirStage::Attack;
                }
                if a.fired == n {
                    let (drops, when) = air_support::bombs(a.entry, a.mark, t.air_jet_bomb_spacing_px, t.air_jet_bomb_lead_px, t.air_edge_margin_px);
                    if a.flown >= when {
                        for at in drops {
                            acts.push(Act::Strike { at, dir: run, bomb: true });
                        }
                        a.fired += 1;
                    }
                }
            }
        }
        acts
    }

    /// The heli's salvo: `air_heli_salvo_rockets` seeker missiles off its
    /// pods at `from`, aimed at `at` (their seek, their blast, every rule of
    /// theirs is a missile's). `Event::AirSalvo`.
    fn air_salvo(&mut self, f: &mut Frame, a: &Aircraft, from: Position, at: Position) {
        let dir = air_support::unit(at - from).unwrap_or(a.heading);
        for tube in 0..tuning().air_heli_salvo_rockets.clamp(1, 8) as u8 {
            f.pending_missiles.push(Missile::spawn(from, dir, a.owner(), tube % 4, at));
        }
        f.muzzle_flashes.push(Shockwave::new(from));
        f.events.push(Event::AirSalvo { id: a.id, x: at.x, y: at.y });
    }

    /// One of the jet's cannon bursts (`bomb` false) or bombs landing at
    /// `at`, fired by `owner` flying `dir`: the damage when `live`
    /// (`side_blast`), then the show. `Event::AirStrike`.
    pub(super) fn air_strike(&mut self, f: &mut Frame, id: u32, owner: Owner, at: Position, dir: Vec2, bomb: bool, live: bool) {
        f.events.push(Event::AirStrike { id, bomb, x: at.x, y: at.y });
        if live {
            let params = if bomb { BlastParams::air_bomb(owner) } else { BlastParams::air_gun(owner) };
            self.side_blast(f, at, owner, &params);
        }
        let mut show = Spectacle::default();
        self.air_strike_show(&mut show, at, dir, bomb);
        f.stage(show);
    }

    /// The show a cannon burst or a bomb puts on at `center`, the jet flying
    /// `dir`: a burst is a small fireball leaning downrange, the impact
    /// flash and a small scorch; a bomb a big one, a ripple and a wide
    /// scorch, the grass round it flattened. Everything but the damage, so a
    /// replica calls it off `Event::AirStrike`. No RNG.
    pub(crate) fn air_strike_show(&mut self, show: &mut Spectacle, center: Position, dir: Vec2, bomb: bool) {
        let t = tuning();
        let lean = Lean { x: dir.x, y: dir.y };
        let (scale, radius) = if bomb { (1.6, t.air_jet_bomb_radius_px) } else { (0.45, t.air_jet_gun_radius_px) };
        let kind = if bomb { BlastKind::Fuel } else { BlastKind::Oil };
        let mut fx = BlastFx::shaped(center, kind, BlastShape::Shot { dir: lean });
        fx.scale *= scale;
        show.blast_fx.push(fx);
        if bomb {
            show.shocks.push(Shockwave::scaled(center, SHOCK_BOMB));
        }
        show.impact_flashes.push(Shockwave::new(center));
        if self.water.depth_at(center) == crate::ground::Depth::Dry {
            show.scorches.push(Scorch::with(center, scale, Some(lean)));
        }
        crate::grass::flatten(&mut self.grass, center, radius * t.blast_grass_flatten);
    }

    /// The show a downed helicopter puts on where it comes down: the
    /// wreck's fireball and a ripple. Cosmetic: it hurts nothing.
    pub(crate) fn aircraft_crash_show(&mut self, show: &mut Spectacle, center: Position) {
        show.blast_fx.push(BlastFx::shaped(center, BlastKind::Fuel, BlastShape::Fire));
        show.shocks.push(Shockwave::scaled(center, SHOCK_BOMB));
        show.impact_flashes.push(Shockwave::new(center));
        if self.water.depth_at(center) == crate::ground::Depth::Dry {
            show.scorches.push(Scorch::with(center, 1.2, None));
        }
    }

    /// Strike the helicopter `id` with `by` (`Game::strike_air`): a bullet
    /// counts a hit, `air_heli_hits` of them bring it down; anything else
    /// downs it at once. Down, it is gone - a fireball where it falls,
    /// `Event::AircraftDowned` - and the seat's next call starts its count.
    /// True when it went down.
    pub(super) fn strike_aircraft(&mut self, f: &mut Frame, id: u32, by: AirStrike) -> bool {
        let found = self.world.query::<(Entity, &Aircraft)>().iter().find(|(_, a)| a.id == id && a.sortie == Sortie::Heli).map(|(e, a)| (e, *a));
        let Some((entity, mut a)) = found else { return false };
        if by == AirStrike::Bullet {
            a.hits += 1;
            if a.hits < tuning().air_heli_hits.max(1) as u32 {
                if let Ok(mut slot) = self.world.get::<&mut Aircraft>(entity) {
                    slot.hits = a.hits;
                }
                return false;
            }
        }
        self.world.despawn(entity).ok();
        f.events.push(Event::AircraftDowned { id, x: a.ground.x, y: a.ground.y, height: a.height, by: by.name() });
        let mut show = Spectacle::default();
        self.aircraft_crash_show(&mut show, a.ground);
        f.stage(show);
        self.start_next_call(a.seat);
        true
    }

    /// The aircraft on the field, by id: what the presentation, the wire and
    /// the tools read.
    pub fn aircraft(&self) -> Vec<Aircraft> {
        let mut out: Vec<Aircraft> = self.world.query::<&Aircraft>().iter().copied().collect();
        out.sort_by_key(|a| a.id);
        out
    }

    /// The air support calls waiting to arrive, by id.
    pub fn air_calls(&self) -> &[AirCall] {
        &self.air_calls
    }
}
