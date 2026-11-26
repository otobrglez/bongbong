//! The minigun's tracking turret (docs/BULLETS_SPEC.md §8, BB-262): while
//! a tank carries the minigun, its turret swings up to
//! `minigun_track_cone_deg` off the hull's heading onto the nearest
//! opposing tank it can see, at `minigun_track_slew_deg` a second, and its
//! rounds leave along that swing (`Tank::minigun_aim`). A target is kept
//! while it stays valid; once it has been held `minigun_lock_seconds` the
//! turret is locked on it, and a locked target wrecked starts a kill chain
//! - the turret slews faster onto the next. An enemy's turret swings half
//! as far at half the rate. Driving stays four-way: only the gun turns.
//!
//! The pick reads no RNG and walks seats first, then enemies by slot,
//! ties to the lower slot, so a seed replays the same swing. Who is a
//! target follows the towers' rule (`towers.rs`'s `pick_target`): an
//! opposing tank in reach, not concealed (grass, a woods trail, smoke), in
//! line of sight, and a seat only from inside its sight box.

use super::hits::Terrain;
use super::{Frame, Game, RollIn};
use crate::Position;
use crate::ai::{Ai, in_sight_box};
use crate::math;
use crate::shell::Owner;
use crate::tank::{ActiveWeapon, Tank};
use crate::tuning::{Tuning, tuning};
use hecs::Entity;

/// A tank the turrets may track this tick.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mark {
    pub slot: usize,
    pub owner: Owner,
    pub pos: Position,
    /// Hidden where it stands (`Terrain::conceals`).
    pub concealed: bool,
}

/// A turret's tracking state, before and after a step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TrackState {
    /// Degrees off the hull's heading (`Tank::minigun_track`).
    pub track: f32,
    pub target: Option<usize>,
    pub lock: f32,
    pub chain: f32,
}

impl TrackState {
    pub fn of(tank: &Tank) -> TrackState {
        TrackState { track: tank.minigun_track, target: tank.minigun_target, lock: tank.minigun_lock, chain: tank.minigun_chain }
    }

    pub fn write(self, tank: &mut Tank) {
        tank.minigun_track = self.track;
        tank.minigun_target = self.target;
        tank.minigun_lock = self.lock;
        tank.minigun_chain = self.chain;
    }
}

/// The tracking cone and slew rate of a seat's turret or an enemy's,
/// degrees and degrees a second.
pub(crate) fn reach(t: &Tuning, seat: bool) -> (f32, f32) {
    if seat { (t.minigun_track_cone_deg, t.minigun_track_slew_deg) } else { (t.minigun_enemy_track_cone_deg, t.minigun_enemy_track_slew_deg) }
}

/// `degrees` brought into -180..=180.
pub(crate) fn wrap180(degrees: f32) -> f32 {
    let d = degrees % 360.0;
    if d > 180.0 {
        d - 360.0
    } else if d < -180.0 {
        d + 360.0
    } else {
        d
    }
}

/// The heading (0 = up, clockwise, degrees) from `from` to `to`.
pub(crate) fn bearing(from: Position, to: Position) -> f32 {
    math::atan2(to.x - from.x, -(to.y - from.y)).to_degrees()
}

/// One tick of a turret on a tank of `owner` standing at `pos` with its
/// hull on `heading`: keep the current target while `valid` holds for it,
/// else take the nearest valid mark (ties to the lower slot); count the
/// lock; start a kill chain when a locked target is gone from `marks`
/// (wrecked); slew toward the target's bearing, clamped to the cone, or
/// back to the hull's line with none. `valid` is the caller's sight test
/// (concealment, line of sight, the sight box), asked only of marks in
/// reach and in the cone.
pub(crate) fn step(
    t: &Tuning,
    owner: Owner,
    pos: Position,
    heading: f32,
    prev: TrackState,
    marks: &[Mark],
    valid: impl Fn(&Mark) -> bool,
    dt: f32,
) -> TrackState {
    let (cone, slew) = reach(t, owner.is_player());
    let in_reach = |m: &Mark| {
        !m.owner.same_side(owner)
            && m.pos.distance_to(pos) <= t.minigun_track_range_px
            && wrap180(bearing(pos, m.pos) - heading).abs() <= cone
            && valid(m)
    };
    let mut chain = (prev.chain - dt).max(0.0);
    let held = prev.target.and_then(|slot| marks.iter().find(|m| m.slot == slot));
    if prev.target.is_some() && held.is_none() && prev.lock >= t.minigun_lock_seconds {
        chain = t.minigun_chain_seconds;
    }
    let target = match held.filter(|m| in_reach(m)) {
        Some(m) => Some(*m),
        None => marks
            .iter()
            .filter(|m| in_reach(m))
            .min_by(|a, b| a.pos.distance_to(pos).total_cmp(&b.pos.distance_to(pos)).then(a.slot.cmp(&b.slot)))
            .copied(),
    };
    let lock = match target {
        Some(m) if prev.target == Some(m.slot) => prev.lock + dt,
        _ => 0.0,
    };
    let goal = target.map_or(0.0, |m| wrap180(bearing(pos, m.pos) - heading).clamp(-cone, cone));
    let rate = slew * if chain > 0.0 { t.minigun_chain_slew_factor } else { 1.0 };
    let reach_step = rate * dt;
    let track = (prev.track + (goal - prev.track).clamp(-reach_step, reach_step)).clamp(-cone, cone);
    TrackState { track, target: target.map(|m| m.slot), lock, chain }
}

/// An owned seat's claimed swing (`Tank::minigun_claim`) as the room takes
/// it: inside the cone, and no further from the last swing than the
/// turret slews in `ticks` ticks (a late packet carries more than one) -
/// a claim past that moves the turret only as far as it could have gone.
pub(crate) fn take_claim(t: &Tuning, prev: f32, claim: f32, ticks: u32) -> f32 {
    let (cone, slew) = reach(t, true);
    let most = slew * t.minigun_chain_slew_factor * crate::PHYSICS_FIXED_DT * ticks.max(1) as f32;
    (prev + (claim.clamp(-cone, cone) - prev).clamp(-most, most)).clamp(-cone, cone)
}

impl Game {
    /// An owned seat's claimed turret swing for this tick
    /// (`net::wire::IntentMsg::track`), for `track_minigun_phase` to check
    /// and take; `None` claims nothing and the room tracks the seat itself.
    pub fn claim_minigun_track(&mut self, seat: usize, track: Option<f32>) {
        let Some(entity) = self.seats.get(seat).copied().flatten() else { return };
        if let Ok(mut tank) = self.world.get::<&mut Tank>(entity) {
            tank.minigun_claim = track;
        }
    }

    /// A client's own seat's turret, tracked on the replica: the world as
    /// this client draws it, which is the world the room rewinds the
    /// enemies to when it judges the seat's rounds (lag compensation). Its
    /// swing goes to the room as the seat's claim (`IntentMsg::track`),
    /// aims the predictor's provisional rounds and turns the turret drawn
    /// here at once; the tank is marked `minigun_local`, so a snapshot
    /// leaves its turret be. `None` with no tank for the seat.
    pub fn track_shown_seat(&mut self, seat: usize, dt: f32) -> Option<f32> {
        let entity = self.seats.get(seat).copied().flatten()?;
        let t = tuning();
        let cover = self.cover_cells().into_owned();
        let mut marks: Vec<Mark> = self
            .world
            .query::<&Tank>()
            .iter()
            .filter(|tank| !tank.is_wreck())
            .map(|tank| Mark { slot: tank.owner_slot(), owner: tank.owner(), pos: tank.position, concealed: crate::grass::conceals(&cover, tank.position) })
            .collect();
        marks.sort_unstable_by_key(|m| m.slot);
        let (owner, pos, heading, prev, carrying) = {
            let tank = self.world.get::<&Tank>(entity).ok()?;
            let carrying = tank.active_weapon() == ActiveWeapon::Minigun && tank.disabled <= 0.0 && !tank.is_wreck();
            (tank.owner(), tank.position, tank.rotation, TrackState::of(&tank), carrying)
        };
        // The tiles are built for the sight test only on a tick something
        // stands in the cone to be seen.
        let (cone, _) = reach(&t, owner.is_player());
        let seen = carrying
            && marks.iter().any(|m| {
                !m.owner.same_side(owner)
                    && !m.concealed
                    && m.pos.distance_to(pos) <= t.minigun_track_range_px
                    && wrap180(bearing(pos, m.pos) - heading).abs() <= cone
            });
        let terrain = seen.then(|| {
            let (width, height) = self.map.field_size();
            Terrain::build(&self.world, width, height, &cover, &self.water)
        });
        let next = if carrying {
            step(&t, owner, pos, heading, prev, &marks, |m| !m.concealed && terrain.as_ref().is_some_and(|tr| tr.line_of_sight(pos, m.pos)), dt)
        } else {
            TrackState::default()
        };
        let mut tank = self.world.get::<&mut Tank>(entity).ok()?;
        next.write(&mut tank);
        tank.minigun_local = true;
        Some(next.track)
    }

    /// Put a seat's minigun turret at `track` degrees off its hull: the
    /// predictor's sandbox, whose provisional rounds leave along the swing
    /// this client tracks (`track_shown_seat`).
    pub(crate) fn set_seat_minigun_track(&mut self, seat: usize, track: f32) {
        let Some(entity) = self.seats.get(seat).copied().flatten() else { return };
        if let Ok(mut tank) = self.world.get::<&mut Tank>(entity) {
            tank.minigun_track = track;
        }
    }

    /// Every live tank the turrets may track, seats first then enemies by
    /// slot.
    pub(super) fn minigun_marks(&self, f: &Frame) -> Vec<(Entity, Mark)> {
        let mut order: Vec<Entity> = self.seats_on_field().into_iter().flatten().collect();
        let mut enemies: Vec<(usize, Entity)> =
            self.world.query::<(Entity, &Tank)>().with::<&Ai>().without::<&RollIn>().iter().map(|(e, t)| (t.owner_slot(), e)).collect();
        enemies.sort_unstable_by_key(|&(slot, _)| slot);
        order.extend(enemies.into_iter().map(|(_, e)| e));
        order
            .into_iter()
            .filter_map(|entity| {
                let mut q = self.world.query_one::<&Tank>(entity);
                let tank = q.get().ok()?;
                (!tank.is_wreck()).then(|| {
                    (entity, Mark { slot: tank.owner_slot(), owner: tank.owner(), pos: tank.position, concealed: f.terrain.conceals(tank.position) })
                })
            })
            .collect()
    }

    /// Swing every minigun's turret for this tick, before anyone fires:
    /// each tank carrying the minigun steps its tracker (`step`) against
    /// the marks as they stood after the last step; an owned seat takes
    /// its client's claimed swing in place of the room's (`take_claim`),
    /// keeping the room's target for the lock bracket. A tank whose weapon
    /// in hand is not the minigun, a wreck or one whose electrics are out
    /// lets its turret go back to the hull's line at once.
    pub(super) fn track_minigun_phase(&mut self, f: &mut Frame) {
        let t = tuning();
        let marks = self.minigun_marks(f);
        let plain: Vec<Mark> = marks.iter().map(|&(_, m)| m).collect();
        let mut moves: Vec<(Entity, TrackState)> = Vec::new();
        for &(entity, _) in &marks {
            let mut q = self.world.query_one::<&Tank>(entity);
            let Ok(tank) = q.get() else { continue };
            if tank.active_weapon() != ActiveWeapon::Minigun || tank.disabled > 0.0 {
                if TrackState::of(tank) != TrackState::default() {
                    moves.push((entity, TrackState::default()));
                }
                continue;
            }
            let (owner, pos) = (tank.owner(), tank.position);
            let valid = |m: &Mark| {
                !m.concealed && (!m.owner.is_player() || owner.is_player() || in_sight_box(m.pos, pos)) && f.terrain.line_of_sight(pos, m.pos)
            };
            let mut next = step(&t, owner, pos, tank.rotation, TrackState::of(tank), &plain, valid, f.dt);
            if let Some(claim) = tank.minigun_claim {
                next.track = take_claim(&t, tank.minigun_track, claim, 1);
            }
            moves.push((entity, next));
        }
        for (entity, next) in moves {
            if let Ok(mut tank) = self.world.get::<&mut Tank>(entity) {
                next.write(&mut tank);
                tank.minigun_claim = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = crate::PHYSICS_FIXED_DT;

    /// A seat at the origin facing east (90°).
    const SEAT: Owner = Owner::Player(0);
    const AT: Position = Position::new(0.0, 0.0);
    const EAST: f32 = 90.0;

    fn enemy(slot: usize, x: f32, y: f32) -> Mark {
        Mark { slot, owner: Owner::Enemy(slot), pos: Position::new(x, y), concealed: false }
    }

    fn tick(t: &Tuning, prev: TrackState, marks: &[Mark]) -> TrackState {
        step(t, SEAT, AT, EAST, prev, marks, |m| !m.concealed, DT)
    }

    /// The nearest opposing tank in the cone is taken; between two as
    /// near, the lower slot; a friend, a far one or one out of the cone
    /// never.
    #[test]
    fn the_nearest_is_taken_ties_to_the_lower_slot() {
        let t = Tuning::DEFAULT;
        let friend = Mark { slot: 1, owner: Owner::Player(1), pos: Position::new(50.0, 0.0), concealed: false };
        let wide = enemy(9, 50.0, 60.0);
        let far = enemy(8, t.minigun_track_range_px + 10.0, 0.0);
        let marks = [friend, wide, far, enemy(7, 200.0, 20.0), enemy(5, 200.0, -20.0)];
        assert_eq!(tick(&t, TrackState::default(), &marks).target, Some(5));
        let hidden = [Mark { concealed: true, ..enemy(3, 100.0, 0.0) }, enemy(4, 150.0, 0.0)];
        assert_eq!(tick(&t, TrackState::default(), &hidden).target, Some(4), "a hidden tank is passed over");
    }

    /// A target is kept while it stays valid, even with a nearer one in
    /// the cone; it is dropped the moment it leaves it.
    #[test]
    fn a_target_is_kept_while_it_stays_valid() {
        let t = Tuning::DEFAULT;
        let held = TrackState { target: Some(7), ..TrackState::default() };
        let marks = [enemy(7, 300.0, 0.0), enemy(2, 100.0, 0.0)];
        let next = tick(&t, held, &marks);
        assert_eq!(next.target, Some(7));
        assert!(next.lock > 0.0, "and the lock counts on");
        let gone = [enemy(7, 0.0, 300.0), enemy(2, 100.0, 0.0)];
        let next = tick(&t, held, &gone);
        assert_eq!((next.target, next.lock), (Some(2), 0.0), "out of the cone, the next is taken afresh");
    }

    /// A locked target gone from the marks - wrecked - starts a kill chain,
    /// in which the turret slews `minigun_chain_slew_factor` times faster.
    #[test]
    fn a_locked_target_wrecked_starts_a_chain() {
        let t = Tuning::DEFAULT;
        let locked = TrackState { target: Some(7), lock: t.minigun_lock_seconds, ..TrackState::default() };
        let next_one = [enemy(2, 100.0, 30.0)];
        let next = tick(&t, locked, &next_one);
        assert_eq!(next.target, Some(2));
        assert!(next.chain > 0.0, "the chain runs");
        let step_size = t.minigun_track_slew_deg * t.minigun_chain_slew_factor * DT;
        assert!((next.track - step_size).abs() < 1e-3, "at the chain's slew: {}", next.track);
        let unlocked = TrackState { target: Some(7), ..TrackState::default() };
        assert_eq!(tick(&t, unlocked, &next_one).chain, 0.0, "no chain without a lock");
    }

    /// The swing never passes the cone, nor moves faster than the slew;
    /// with no target it comes back to the hull's line.
    #[test]
    fn the_swing_keeps_to_the_cone_and_the_slew() {
        let t = Tuning::DEFAULT;
        let marks = [enemy(2, 100.0, 50.0)];
        let one = tick(&t, TrackState::default(), &marks);
        assert!((one.track - t.minigun_track_slew_deg * DT).abs() < 1e-3);
        let mut s = one;
        for _ in 0..120 {
            s = tick(&t, s, &marks);
        }
        let bearing = wrap180(bearing(AT, marks[0].pos) - EAST);
        assert!((s.track - bearing).abs() < 1e-3 && s.track <= t.minigun_track_cone_deg, "{} against {bearing}", s.track);
        let back = tick(&t, s, &[]);
        assert!(back.target.is_none() && back.track < s.track, "back toward the line");
    }

    /// An owned seat's claim is held to the cone and to what the turret
    /// could have slewed since the last swing.
    #[test]
    fn a_claim_is_held_to_the_cone_and_the_slew() {
        let t = Tuning::DEFAULT;
        let most = t.minigun_track_slew_deg * t.minigun_chain_slew_factor * DT;
        assert_eq!(take_claim(&t, 0.0, 2.0, 1), 2.0);
        assert!((take_claim(&t, 0.0, 25.0, 1) - most).abs() < 1e-4, "a jump moves only as far as it could");
        assert!((take_claim(&t, 0.0, 25.0, 4) - (most * 4.0).min(25.0)).abs() < 1e-4, "a late packet carries more ticks");
        assert_eq!(take_claim(&t, 29.0, 80.0, 10), t.minigun_track_cone_deg);
    }
}
