//! The EMP burst's world half (docs/emp-burst.md; `emp.rs` is the pulse,
//! the measures and the drawing): a press queues a pulse, `resolve_emp`
//! puts it on the field once the tank loops are done (and, at night, puts
//! every lamp post out), and `tick_emp_pulses` strikes whatever its front
//! passes, once - tanks disabled (`Tank::disable`), towers offline
//! (`Tower::disable`), missiles dead (`Missile::kill`). Also what the AI is
//! handed: each EMP tank's sense of a pulse (`Game::emp_senses`) and the
//! dangers every enemy keeps out of (`Game::emp_dangers`). No RNG anywhere
//! here.

use std::collections::BTreeMap;

use hecs::Entity;

use crate::ai::{Ai, Danger, DangerShape, EmpSense};
use crate::emp::{self, EmpPulse};
use crate::frog::Side;
use crate::math::Vec2;
use crate::missile::Missile;
use crate::obstacle::Obstacle;
use crate::shell::Owner;
use crate::shockwave::Shockwave;
use crate::tank::{ActiveWeapon, Tank};
use crate::tuning::{Tuning, tuning};
use crate::{OBSTACLE_GRID_SIZE, Position};

use super::{Event, Frame, Game, RollIn, Spectacle};

/// A pulse fired this frame, waiting for `resolve_emp` to put it on the
/// field.
pub(super) struct PendingEmp {
    pub origin: Position,
    pub owner: Owner,
}

/// How far behind the front (px) a thing may be found and still be struck:
/// a hull or a missile coming the other way crosses the front between two
/// frames.
const FRONT_SLACK_PX: f32 = 16.0;

/// Fire one pulse of `tank`'s EMP: its module's pulse cell, its own special
/// offline for `emp_disable_seconds` (the pulse's cost and its reload),
/// `Event::EmpPulse` right after the dispatch's `Fired`, and the pulse
/// queued for `resolve_emp`. No recoil: nothing leaves the hull. No RNG.
pub(super) fn fire_emp(f: &mut Frame, tank: &mut Tank, owner: Owner) {
    let t = tuning();
    tank.kick_emp();
    tank.special_offline = tank.special_offline.max(t.emp_disable_seconds);
    let origin = tank.position;
    f.events.push(Event::EmpPulse { slot: tank.owner_slot(), x: origin.x, y: origin.y });
    f.pending_emp.push(PendingEmp { origin, owner });
}

/// A seat as an EMP-carrying enemy sees it this frame (`Game::emp_senses`,
/// `Game::emp_dangers`).
#[derive(Clone, Copy, Debug)]
pub(super) struct EmpSeat {
    pub seat: u8,
    pub pos: Position,
    /// Its hull box: centre and half extents.
    pub hull: (Position, Vec2),
    /// On the field and not a wreck.
    pub live: bool,
    /// In tall grass that hides it (`Terrain::conceals`).
    pub concealed: bool,
    pub shielded: bool,
    /// Carries a special that fires now.
    pub special_online: bool,
    /// Carries an EMP that fires now: a danger to every enemy near it.
    pub emp_armed: bool,
    pub disabled: bool,
}

impl EmpSeat {
    /// Seat `seat` as `tank` stands, `live` and `concealed` as the enemy
    /// phase read them.
    pub(super) fn of(seat: u8, tank: &Tank, live: bool, concealed: bool) -> EmpSeat {
        let (c, h) = tank.hull_bbox_world();
        EmpSeat {
            seat,
            pos: tank.position,
            hull: (c, Vec2::new(h.x, h.y)),
            live,
            concealed,
            shielded: tank.is_shielded(),
            special_online: tank.special().is_some() && !tank.special_down(),
            emp_armed: tank.active_weapon() == ActiveWeapon::Emp,
            disabled: tank.is_disabled(),
        }
    }
}

/// Half a tower's cell: its box, for the ring's reach.
fn tower_half() -> Vec2 {
    Vec2::new(OBSTACLE_GRID_SIZE * 0.5, OBSTACLE_GRID_SIZE * 0.5)
}

impl Game {
    /// Whether the sky in force is night's (`Weather::Night` or `Storm`,
    /// what `nightfall` turns a round into): when an EMP puts the lamp posts
    /// out, and when a seat's headlights are worth a pulse to the AI.
    pub fn is_night(&self) -> bool {
        matches!(self.weather, crate::map::Weather::Night | crate::map::Weather::Storm)
    }

    /// Seconds every lamp post on the map stays dark (docs/emp-burst.md
    /// "At 11"), 0 while they are lit.
    pub fn lamps_out(&self) -> f32 {
        self.lamps_out
    }

    /// The lamp posts still standing and lit: none while an EMP has them
    /// out (`lamps_out`). What lights the ground and lights a seat up for
    /// an enemy (`Game::is_lit`).
    pub fn lit_lamp_posts(&self) -> Vec<Position> {
        if self.lamps_out > 0.0 { Vec::new() } else { self.lamp_posts() }
    }

    /// The show of a pulse from `origin` by `owner`: its ring on the field
    /// (`live`: one that strikes, the round's own) and a soft ripple. What
    /// the round puts on for a press, a replica for the room's `EmpPulse`
    /// and a client for its own press (`draw_press_show`).
    pub(crate) fn emp_show(&mut self, show: &mut Spectacle, origin: Position, owner: Owner, live: bool) {
        let t = tuning();
        self.emp_pulses.push(EmpPulse::new(origin, owner, live));
        if t.emp_shock > 0.0 {
            show.shocks.push(Shockwave::scaled(origin, t.emp_shock));
        }
    }

    /// Put this frame's pulses on the field; at night each puts every lamp
    /// post out for `emp_lamp_seconds`.
    pub(super) fn resolve_emp(&mut self, f: &mut Frame) {
        let pending: Vec<PendingEmp> = f.pending_emp.drain(..).collect();
        for p in pending {
            let mut show = Spectacle::default();
            self.emp_show(&mut show, p.origin, p.owner, true);
            f.stage(show);
            if self.is_night() {
                self.lamps_out = self.lamps_out.max(tuning().emp_lamp_seconds);
            }
        }
    }

    /// Run every pulse's front on by the frame and strike what it passes;
    /// drop the pulses whose picture is done. `live` is false on the end
    /// screen, where the rings only finish their picture.
    pub(super) fn tick_emp_pulses(&mut self, f: &mut Frame, live: bool) {
        if self.emp_pulses.is_empty() {
            return;
        }
        let t = tuning();
        let mut pulses = std::mem::take(&mut self.emp_pulses);
        for pulse in pulses.iter_mut() {
            pulse.age += f.dt;
            if pulse.spent(&t) {
                continue;
            }
            let (from, to) = (pulse.swept, pulse.front(&t));
            if live && pulse.live {
                self.strike_emp(f, pulse, from, to, &t);
            }
            pulse.swept = to;
        }
        pulses.retain(|p| !p.done(&t));
        pulses.append(&mut self.emp_pulses);
        self.emp_pulses = pulses;
    }

    /// A replica's pulses (docs/emp-burst.md "Wire"): age them and drop the
    /// done ones. A round that simulates its pulses never calls this.
    pub(crate) fn tick_emp_pictures(&mut self, dt: f32) {
        if self.emp_pulses.is_empty() {
            return;
        }
        let t = tuning();
        for pulse in self.emp_pulses.iter_mut() {
            pulse.age += dt;
            pulse.swept = pulse.front(&t);
        }
        self.emp_pulses.retain(|p| !p.done(&t));
    }

    /// Everything `pulse` reaches between the front's radii `from` and
    /// `to`, struck once, in a fixed order (docs/emp-burst.md "What
    /// everything electric means"): the tanks (the seats in index order,
    /// then the enemies by slot), the towers (cell order), the missiles (id
    /// order). The FPV drones and the gravity well add their arms after the
    /// missiles, on the same front.
    fn strike_emp(&mut self, f: &mut Frame, pulse: &mut EmpPulse, from: f32, to: f32, t: &Tuning) {
        let origin = pulse.origin;
        let swept = |d: f32| d <= to && d > from - FRONT_SLACK_PX;
        // Tanks: a hull reached by the nearest point of its box.
        let mut hulls: Vec<(Entity, usize)> =
            self.seats_on_field().into_iter().flatten().map(|e| (e, super::with_tank(&self.world, e, |t| t.owner_slot()))).collect();
        let mut enemies: Vec<(Entity, usize)> =
            self.world.query::<(Entity, &Tank)>().with::<&Ai>().iter().map(|(e, t)| (e, t.owner_slot())).collect();
        enemies.sort_by_key(|&(_, slot)| slot);
        hulls.extend(enemies);
        for (entity, slot) in hulls {
            if pulse.struck.contains(&slot) || self.world.get::<&RollIn>(entity).is_ok() {
                continue;
            }
            let Ok(mut tank) = self.world.get::<&mut Tank>(entity) else { continue };
            if tank.is_wreck() || tank.body.is_none() || tank.owner() == pulse.owner {
                continue;
            }
            let (c, h) = tank.hull_bbox_world();
            if !swept(emp::box_reach(origin, c, Vec2::new(h.x, h.y))) {
                continue;
            }
            pulse.struck.push(slot);
            tank.disable(t.emp_disable_seconds);
            f.events.push(Event::Disabled { slot, x: tank.position.x, y: tank.position.y });
        }
        // Towers, either side: offline for `emp_tower_seconds`.
        let cells: Vec<(i32, i32)> = self.towers.keys().copied().collect();
        for cell in cells {
            let Some(tower) = self.towers.get(&cell) else { continue };
            // A tower stands still: the band alone, no slack.
            let standing = self.world.get::<&Obstacle>(tower.entity).is_ok_and(|o| !o.destroyed);
            let d = emp::box_reach(origin, tower.position, tower_half());
            if !standing || d <= from || d > to {
                continue;
            }
            let at = tower.position;
            if let Some(tower) = self.towers.get_mut(&cell) {
                tower.disable(t.emp_tower_seconds);
            }
            f.events.push(Event::TowerDisabled { x: at.x, y: at.y });
        }
        // Missiles in the air, by id: dead where they are.
        let mut missiles: Vec<(u32, Entity)> = self.world.query::<(Entity, &Missile)>().iter().map(|(e, m)| (m.id, e)).collect();
        missiles.sort_by_key(|&(id, _)| id);
        for (_, entity) in missiles {
            if let Ok(mut m) = self.world.get::<&mut Missile>(entity)
                && !m.is_dead()
                && swept(origin.distance_to(m.position))
            {
                m.kill();
            }
        }
    }

    /// Where one seat's EMP would pulse from right now: the sandbox's pose,
    /// for a client drawing its own press (`net::predict`).
    pub(crate) fn seat_emp(&self, seat: usize) -> Option<Position> {
        let entity = self.seats.get(seat).copied().flatten()?;
        let tank = self.world.get::<&Tank>(entity).ok()?;
        Some(tank.position)
    }

    /// What each enemy carrying an online EMP would get from a pulse where
    /// it stands (`ai::EmpSense`, docs/emp-burst.md "AI"), for `Ai::think`:
    /// built in `enemy_phase` before the tanks think, only when one of them
    /// carries one. `seats` is every seat as the enemies see it this frame,
    /// in index order.
    pub(super) fn emp_senses(&self, seats: &[EmpSeat]) -> BTreeMap<Entity, EmpSense> {
        let t = tuning();
        let mut out = BTreeMap::new();
        let armed: Vec<(Entity, usize, Position, u8)> = self
            .world
            .query::<(Entity, &Tank, &Ai)>()
            .iter()
            .filter(|(_, tank, _)| !tank.is_wreck() && tank.active_weapon() == ActiveWeapon::Emp)
            .map(|(e, tank, ai)| (e, tank.owner_slot(), tank.position, ai.target_player()))
            .collect();
        if armed.is_empty() {
            return out;
        }
        let reach = t.emp_radius_px;
        // Each fellow enemy by its hull box, now and where its motion
        // carries it by the time a pulse decided now would reach it (the
        // crackle and the ring's run out), so one driving into the ring
        // holds it too.
        let lead = ActiveWeapon::Emp.tell_seconds().unwrap_or(0.0) + reach / t.emp_ring_speed.max(1.0);
        let friends: Vec<(usize, Position, Vec2, Position)> = self
            .world
            .query::<(&Tank, Option<&RollIn>)>()
            .with::<&Ai>()
            .iter()
            .filter(|(tank, rolling)| rolling.is_none() && !tank.is_wreck() && tank.body.is_some() && !tank.is_disabled())
            .map(|(tank, _)| {
                let (c, h) = tank.hull_bbox_world();
                let v = tank.body.map_or(Vec2::zero(), |b| self.physics.velocity(b));
                (tank.owner_slot(), c, Vec2::new(h.x, h.y), Position::new(c.x + v.x * lead, c.y + v.y * lead))
            })
            .collect();
        // The standing, online towers: the players' worth a pulse, the
        // enemies' their own side's.
        let towers: Vec<(Position, Side)> = self
            .towers
            .values()
            .filter(|tw| tw.disabled <= 0.0 && self.world.get::<&Obstacle>(tw.entity).is_ok_and(|o| !o.destroyed))
            .map(|tw| (tw.position, tw.side))
            .collect();
        let night = self.is_night();
        let half = t.sight_box_half_px();
        let margin = t.emp_ai_friend_margin_px;
        for &(entity, slot, me, target) in &armed {
            let Ok(ai) = self.world.get::<&Ai>(entity) else { continue };
            let mut sense = EmpSense::default();
            let mut best: Option<(i32, u8)> = None;
            for seat in seats.iter().filter(|s| s.live) {
                if emp::box_reach(me, seat.hull.0, seat.hull.1) > reach {
                    continue;
                }
                if !crate::ai::in_sight_box_of(half, seat.pos, me) {
                    sense.off_box = true;
                    continue;
                }
                if seat.concealed && !ai.is_hit_alerted() {
                    continue;
                }
                let value = emp::seat_value(&t, seat.shielded, seat.special_online, night, seat.disabled);
                sense.value += value;
                if value > 0 && best.is_none_or(|(v, _)| value > v) {
                    best = Some((value, seat.seat));
                }
            }
            sense.at_seat = best.map(|(_, s)| s);
            for &(at, side) in &towers {
                if emp::box_reach(me, at, tower_half()) > reach {
                    continue;
                }
                match side {
                    Side::Player => sense.value += t.emp_ai_tower_value,
                    Side::Enemy => sense.friendly_tower = true,
                }
            }
            sense.friends = friends.iter().any(|&(s, c, h, ahead)| {
                s != slot && (emp::box_reach(me, c, h) <= reach + margin || emp::box_reach(me, ahead, h) <= reach + margin)
            });
            if let Some(seat) = seats.iter().find(|s| s.seat == target && s.live) {
                sense.target_value = emp::seat_value(&t, seat.shielded, seat.special_online, night, seat.disabled);
                sense.target_crowded = friends.iter().any(|&(s, c, h, _)| s != slot && emp::box_reach(seat.pos, c, h) <= reach);
                // The `emp_ai_closers` EMP tanks nearest the seat this one
                // fights close in on it (ties on slot).
                let nearer = armed
                    .iter()
                    .filter(|a| a.3 == target && a.0 != entity)
                    .filter(|a| a.2.distance_to(seat.pos).total_cmp(&me.distance_to(seat.pos)).then(a.1.cmp(&slot)).is_lt())
                    .count();
                sense.closer = (nearer as i32) < t.emp_ai_closers;
            }
            out.insert(entity, sense);
        }
        out
    }

    /// The places every enemy keeps out of this frame because of an EMP
    /// (docs/emp-burst.md "Reacting to the EMP"): round every live seat on
    /// the field carrying an armed EMP that is not hidden in grass, and
    /// round every enemy whose EMP crackle is running, `emp_radius_px +
    /// emp_ai_berth_px` out - seats in index order, then enemies by slot.
    /// Empty when no tank carries an EMP.
    pub(super) fn emp_dangers(&self, seats: &[EmpSeat]) -> Vec<Danger> {
        let t = tuning();
        let radius = t.emp_radius_px + t.emp_ai_berth_px;
        let mut out: Vec<Danger> = seats
            .iter()
            .filter(|s| s.live && s.emp_armed && !s.concealed)
            .map(|s| Danger { shape: DangerShape::Disc { at: s.pos, radius }, owner: Some(s.seat as usize) })
            .collect();
        let mut tells: Vec<(usize, Position)> = self
            .world
            .query::<&Tank>()
            .with::<&Ai>()
            .iter()
            .filter(|tank| !tank.is_wreck() && tank.tell.is_some_and(|tell| tell.weapon == ActiveWeapon::Emp))
            .map(|tank| (tank.owner_slot(), tank.position))
            .collect();
        tells.sort_by_key(|&(slot, _)| slot);
        out.extend(tells.into_iter().map(|(slot, at)| Danger { shape: DangerShape::Disc { at, radius }, owner: Some(slot) }));
        out
    }

    /// The centres of the nav cells inside the seats' dangers (the seat
    /// half of `emp_dangers`, each seat's concealment read off the cover
    /// itself): what the frame's route grid surcharges, so a route goes
    /// round an armed seat's reach rather than through it.
    pub(super) fn danger_route_cells(&self, grid: &crate::pathfind::Grid) -> Vec<Position> {
        let cover = self.cover_cells();
        let seats: Vec<EmpSeat> = self
            .seats_on_field()
            .into_iter()
            .enumerate()
            .filter_map(|(i, e)| e.map(|e| (i, e)))
            .map(|(i, e)| {
                super::with_tank(&self.world, e, |t| EmpSeat::of(i as u8, t, !t.is_wreck(), crate::grass::conceals(&cover, t.position)))
            })
            .collect();
        let (cols, rows, cell) = grid.dims();
        let mut cells = Vec::new();
        let seat_owned = |d: &&Danger| d.owner.is_some_and(|o| seats.iter().any(|s| s.seat as usize == o));
        for danger in self.emp_dangers(&seats).iter().filter(seat_owned) {
            for row in 0..rows {
                for col in 0..cols {
                    let at = Position::new((col as f32 + 0.5) * cell, (row as f32 + 0.5) * cell);
                    if danger.depth(at) > 0.0 {
                        cells.push(at);
                    }
                }
            }
        }
        cells
    }

    /// Whether any tank on the field carries an EMP - what builds the
    /// dangers at all.
    pub(super) fn any_emp(&self) -> bool {
        self.world.query::<&Tank>().iter().any(|t| !t.is_wreck() && t.emp_charges > 0)
    }
}
