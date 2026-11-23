//! Air support's world half (docs/air-support.md; `air_support.rs` is the
//! call and the aircraft, `air_support/heli.rs` and `jet.rs` how each one
//! flies). A crate calls a sortie (`call_air_support`, from `pickup_phase`);
//! `air_support_phase` brings the aircraft in when its count runs out, shows
//! it the world (`AirView`), steps it (`Aircraft::step`), carries out what
//! it does (`AirAct`) and takes it away once it is off the field. The
//! helicopter fires seeker missiles off its pods (`missile.rs`, every rule
//! of theirs kept); the jet's cannon and bombs are blasts of their own
//! (`air_strike`). No RNG but the blasts' damage rolls, which only a round
//! with an air crate taken ever draws.

use hecs::Entity;

use crate::ai::{Ai, in_sight_box};
use crate::air::AirStrike;
use crate::air_support::{self, AirAct, AirCall, AirFriend, AirMark, AirView, Aircraft, MarkKey, Sortie};
use crate::blast::{BlastFx, BlastKind, BlastShape, Lean, Scorch};
use crate::bong::Side;
use crate::math::Vec2;
use crate::missile::Missile;
use crate::obstacle::Obstacle;
use crate::shell::Owner;
use crate::shockwave::Shockwave;
use crate::tank::Tank;
use crate::tuning::tuning;
use crate::Position;

use super::combat::BlastParams;
use super::{Event, Frame, Game, RollIn, Spectacle};

impl BlastParams {
    /// One burst of the jet's cannon: fixed damage, no roll.
    pub fn air_gun(by: Owner) -> Self {
        let t = tuning();
        BlastParams::ordnance(t.air_jet_gun_radius_px, (t.air_jet_gun_damage, t.air_jet_gun_damage), t.air_jet_gun_knockback, by)
    }

    /// One of the jet's bombs.
    pub fn air_bomb(by: Owner) -> Self {
        let t = tuning();
        BlastParams::ordnance(t.air_jet_bomb_radius_px, (t.air_jet_bomb_damage_min, t.air_jet_bomb_damage_max), t.air_jet_bomb_knockback, by)
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

    /// Where `seat` stands for its aircraft, and whether it is a wreck: its
    /// tank, a wreck included; `None` once the seat has no tank at all.
    fn air_anchor(&self, seat: u8) -> Option<(Position, bool)> {
        let entity = self.seat(seat as usize)?;
        self.world.get::<&Tank>(entity).ok().map(|t| (t.position, t.is_wreck()))
    }

    /// How fast `seat`'s tank moves over the ground (`AirView::seat_velocity`):
    /// what its body does, zero for a wreck or no tank.
    fn air_anchor_velocity(&self, seat: u8) -> Vec2 {
        let Some(entity) = self.seat(seat as usize) else { return Vec2::zero() };
        let Ok(tank) = self.world.get::<&Tank>(entity) else { return Vec2::zero() };
        match tank.body {
            Some(body) if !tank.is_wreck() => self.physics.velocity(body),
            _ => Vec2::zero(),
        }
    }

    /// What an aircraft of the seat standing at `anchor` may attack
    /// (`AirView::marks`), inside the seat's sight box: the enemy tanks live
    /// on the field and the enemy towers still standing. Nearest first; at
    /// the same distance a tank before a tower, tanks by slot and towers by
    /// cell.
    fn air_marks(&self, anchor: Position) -> Vec<AirMark> {
        let mut out: Vec<(f32, AirMark)> = self
            .world
            .query::<&Tank>()
            .with::<&Ai>()
            .without::<&RollIn>()
            .iter()
            .filter(|t| !t.is_wreck() && t.body.is_some() && in_sight_box(anchor, t.position))
            .map(|t| {
                let velocity = t.body.map_or(Vec2::zero(), |body| self.physics.velocity(body));
                (t.position.distance_to(anchor), AirMark { key: MarkKey::Tank(t.owner_slot()), at: t.position, velocity })
            })
            .collect();
        out.extend(
            self.towers
                .iter()
                .filter(|(_, t)| t.side == Side::Enemy && in_sight_box(anchor, t.position))
                .filter(|(_, t)| self.world.get::<&Obstacle>(t.entity).is_ok_and(|o| !o.destroyed))
                .map(|(&cell, t)| (t.position.distance_to(anchor), AirMark { key: MarkKey::Tower(cell), at: t.position, velocity: Vec2::zero() })),
        );
        // `MarkKey` orders a tank before a tower, tanks by slot, towers by cell.
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.key.cmp(&b.1.key)));
        out.into_iter().map(|(_, mark)| mark).collect()
    }

    /// What an aircraft's blasts must not reach (`AirView::friends`):
    /// every seat's live hull on the field, in index order, and the
    /// players' bong while it lives.
    fn air_friends(&self) -> Vec<AirFriend> {
        let mut out: Vec<AirFriend> = self
            .seats_on_field()
            .into_iter()
            .flatten()
            .filter_map(|e| self.world.get::<&Tank>(e).ok().filter(|t| !t.is_wreck()).map(|t| {
                let (at, half) = t.hull_bbox_world();
                AirFriend { at, reach: half.x.max(half.y) }
            }))
            .collect();
        if let Some(bong) = self.bong.and_then(|e| self.world.get::<&crate::bong::Bong>(e).ok().filter(|b| !b.is_dead()).map(|b| b.position)) {
            let (hx, hy) = crate::BONG_COLLIDER_HALF_EXTENT;
            out.push(AirFriend { at: bong, reach: hx.max(hy) });
        }
        out
    }

    /// Air support's frame: bring in every aircraft whose count ran out,
    /// step them all (`Aircraft::step` on what `AirView` shows each), carry
    /// out what they do, and take away the ones gone off the field. `live`
    /// is false on the end screen: no call arrives, an aircraft is shown no
    /// target and whatever it would do is dropped.
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
        let mut acts = Vec::new();
        let friends = self.air_friends();
        for (id, entity) in flying {
            let Ok(mut a) = self.world.get::<&mut Aircraft>(entity).map(|a| *a) else { continue };
            let anchor = self.air_anchor(a.seat);
            let marks = match anchor {
                Some((at, _)) if live => self.air_marks(at),
                _ => Vec::new(),
            };
            let view = AirView {
                seat: anchor.map(|(at, _)| at),
                seat_velocity: self.air_anchor_velocity(a.seat),
                seat_wrecked: anchor.is_some_and(|(_, wreck)| wreck),
                marks: &marks,
                friends: &friends,
                field,
                live,
            };
            acts.clear();
            a.step(&view, f.dt, &t, &mut acts);
            if !live {
                acts.clear();
            }
            let leaving = a.gone(field, &t);
            if let Ok(mut slot) = self.world.get::<&mut Aircraft>(entity) {
                *slot = a;
            }
            for act in acts.drain(..) {
                match act {
                    AirAct::Salvo { from, at } => self.air_salvo(f, &a, from, at),
                    AirAct::Strike { at, dir, bomb } => self.air_strike(f, id, a.owner(), at, dir, bomb, true),
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

    /// Every call whose count ran out, by id: its aircraft comes in - the
    /// helicopter from the edge its id hashes to, in line with its seat; the
    /// jet from the far side of the nearest target (of its seat, with none),
    /// in line with it. `Event::AirSupportArrived::armed` is whether it
    /// found a target in the seat's sight box as it came.
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
            let Some((anchor, _)) = self.air_anchor(call.seat) else {
                // The seat is gone from the round: the call is spent.
                f.events.push(Event::AirSupportLeft { id: call.id });
                continue;
            };
            let marks = self.air_marks(anchor);
            let armed = !marks.is_empty();
            let (edge, toward) = match call.sortie {
                Sortie::Heli => (air_support::entry_edge(call.id), anchor),
                Sortie::Jet => {
                    let toward = marks.first().map_or(anchor, |m| m.at);
                    (air_support::jet_edge(call.id, toward, field), toward)
                }
            };
            let entry = air_support::entry_point(edge, toward, field, t.air_edge_margin_px);
            let aircraft = Aircraft::arrive(call.id, call.seat, call.sortie, entry, toward, &t);
            f.events.push(Event::AirSupportArrived { id: call.id, seat: call.seat, sortie: call.sortie, x: entry.x, y: entry.y, armed });
            self.world.spawn((aircraft,));
        }
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
    /// (`Game::blast`), then the show. `Event::AirStrike`.
    pub(super) fn air_strike(&mut self, f: &mut Frame, id: u32, owner: Owner, at: Position, dir: Vec2, bomb: bool, live: bool) {
        f.events.push(Event::AirStrike { id, bomb, x: at.x, y: at.y });
        if live {
            let params = if bomb { BlastParams::air_bomb(owner) } else { BlastParams::air_gun(owner) };
            self.blast(f, at, &params, &[]);
        }
        let mut show = Spectacle::default();
        self.air_strike_show(&mut show, at, dir, bomb);
        f.stage(show);
    }

    /// The show a cannon burst or a bomb puts on at `center`, the jet flying
    /// `dir`: a burst is a small fireball leaning downrange, the impact
    /// flash and a small scorch; a bomb a big one, a ripple, the screen
    /// flash and a wide scorch, the grass round it flattened. Everything but the damage, so a
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
            show.shocks.push(Shockwave::scaled(center, t.bomb_shock));
            self.flash_screen();
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
        show.shocks.push(Shockwave::scaled(center, crate::tuning::tuning().bomb_shock));
        show.impact_flashes.push(Shockwave::new(center));
        if self.water.depth_at(center) == crate::ground::Depth::Dry {
            show.scorches.push(Scorch::with(center, 1.2, None));
        }
    }

    /// Strike the helicopter `id` with `by` (`Game::strike_air`): a bullet
    /// counts a hit (`Event::AirHit`), `air_heli_hits` of them bring it
    /// down; anything else downs it at once. Down, it is gone - a fireball
    /// where it falls, `Event::AircraftDowned` - and the seat's next call
    /// starts its count. True when it went down.
    pub(super) fn strike_aircraft(&mut self, f: &mut Frame, id: u32, by: AirStrike) -> bool {
        let found = self.world.query::<(Entity, &Aircraft)>().iter().find(|(_, a)| a.id == id && a.sortie == Sortie::Heli).map(|(e, a)| (e, *a));
        let Some((entity, mut a)) = found else { return false };
        if by == AirStrike::Bullet {
            a.hits += 1;
            if a.hits < tuning().air_heli_hits.max(1) as u32 {
                if let Ok(mut slot) = self.world.get::<&mut Aircraft>(entity) {
                    slot.hits = a.hits;
                }
                f.events.push(Event::AirHit { id, aircraft: true, x: a.ground.x, y: a.ground.y });
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
