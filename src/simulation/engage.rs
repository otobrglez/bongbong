//! Engagement slots: every enemy actually attacking claims a distinct point
//! near the player instead of converging on the same spot. Four cardinal
//! axes through the player, each with two lateral firing slots at
//! ENGAGE_RING_RADIUS (rank 0, both within ENEMY_FIRE_ALIGN_PX of the axis
//! so either can fire) and two reserve slots at ENGAGE_RESERVE_RADIUS (rank
//! 1, beyond attack range so a reserve tank neither fires nor blocks a
//! lane) - 16 points, claimed greedily with mutual exclusion each frame.
//! Pure geometry plus two callbacks; no world access, so it is unit-tested
//! on its own. Every assignment also fills an [`EngageReport`] - which
//! slot each tank holds, why the others were passed over - for tooling.

use std::collections::HashMap;

use hecs::Entity;
use serde::Serialize;

use crate::Position;
use crate::tuning::tuning;

/// One slot identity: cardinal axis (0 = N, 1 = E, 2 = S, 3 = W), rank (0
/// firing line, 1 reserve) and lateral side (-1/+1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct EngageSlot {
    pub axis: u8,
    pub rank: u8,
    pub side: i8,
}

/// Slots in the ring: 4 axes x 2 ranks x 2 sides.
pub(crate) const SLOT_COUNT: usize = 16;

const DIRS: [(f32, f32); 4] = [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)];
const AXIS_NAMES: [&str; 4] = ["up", "right", "down", "left"];

impl EngageSlot {
    /// This slot's position in the 16-entry table: axis-major, then rank,
    /// then side (-1 before +1).
    pub fn index(self) -> usize {
        self.axis as usize * 4 + self.rank as usize * 2 + side_idx(self.side)
    }

    pub fn from_index(i: usize) -> EngageSlot {
        EngageSlot { axis: (i / 4) as u8, rank: ((i / 2) % 2) as u8, side: if i % 2 == 0 { -1 } else { 1 } }
    }

    /// The axis as the direction the slot lies in from the player.
    pub fn axis_name(self) -> &'static str {
        AXIS_NAMES[self.axis as usize]
    }
}

/// Why an enemy is or isn't competing for a slot this frame - the
/// predicate `Game::enemy_phase` applies before calling `assign`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngageStatus {
    Engaged,
    Wreck,
    /// Damage at or past the flee threshold.
    Fleeing,
    /// Backing off to recharge shells.
    Retreating,
    /// Neither within view range nor hit-alerted.
    OutOfRange,
}

/// How many candidate slots one tank's search threw out, by reason, before
/// it settled on one (or gave up). A tank with no slot steers at the
/// player directly, so a high `off_map`/`no_los` count on several tanks
/// at once is what a pile-up looks like from here.
#[derive(Clone, Copy, Default, Debug, Serialize)]
pub struct Rejections {
    pub claimed: u8,
    pub off_map: u8,
    pub unreachable: u8,
    pub no_los: u8,
}

/// One of the 16 slots as this frame's assignment saw it.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct SlotReport {
    /// World point; `None` when it can't be kept inside the battlefield.
    pub point: Option<Position>,
    /// Line of sight from the point to the player - `None` until some
    /// tank's search got as far as checking it.
    pub line_of_sight: Option<bool>,
    pub claimed_by: Option<Entity>,
}

/// One enemy's outcome. `slot`/`target`/`sticky`/`rejected` are only
/// filled for `Engaged` tanks.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EngageTank {
    pub entity: Entity,
    /// `Tank::owner_slot`.
    pub owner: usize,
    pub status: EngageStatus,
    pub slot: Option<EngageSlot>,
    pub target: Option<Position>,
    /// The slot is the one held last frame, kept without a new search.
    pub sticky: bool,
    pub rejected: Rejections,
}

impl EngageTank {
    pub fn new(entity: Entity, owner: usize, status: EngageStatus) -> Self {
        EngageTank { entity, owner, status, slot: None, target: None, sticky: false, rejected: Rejections::default() }
    }
}

/// Everything one enemy phase's assignment decided, kept on `Game` for the
/// `engage` overlay, the debug snapshot and the AI event diff.
#[derive(Default)]
pub(crate) struct EngageReport {
    /// False when fewer than two tanks were engaged and no ring was built.
    pub built: bool,
    pub slots: [SlotReport; SLOT_COUNT],
    pub tanks: Vec<EngageTank>,
}

impl EngageReport {
    pub fn target(&self, entity: Entity) -> Option<Position> {
        self.tanks.iter().find(|t| t.entity == entity).and_then(|t| t.target)
    }

    pub fn slot_of(&self, entity: Entity) -> Option<EngageSlot> {
        self.tanks.iter().find(|t| t.entity == entity).and_then(|t| t.slot)
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Everything `EngageRing::assign` needs about this frame.
pub(super) struct EngageCtx<'a> {
    pub player_pos: Position,
    pub width: f32,
    pub height: f32,
    /// Keep every slot at least this far from the battlefield edge.
    pub margin: f32,
    /// Whether a route exists between two points on this frame's nav grid.
    pub reachable: &'a dyn Fn(Position, Position) -> bool,
    /// Whether a shot from the first point reaches the second unobstructed.
    pub line_of_sight: &'a dyn Fn(Position, Position) -> bool,
}

/// Slot assignment with memory: a tank keeps the slot it already holds for
/// as long as it stays valid and unclaimed, and only searches afresh when
/// it doesn't. Re-choosing from scratch every frame let the pick flip
/// between two equally valid slots near a reachability/line-of-sight
/// boundary, and a flipping *target* is a discontinuity the AI's heading
/// hysteresis was never built to absorb.
#[derive(Default)]
pub(super) struct EngageRing {
    choice: HashMap<Entity, EngageSlot>,
}

/// Rotate a cardinal unit vector 90 degrees - still cardinal, so a lateral
/// offset moves along one axis and the boundary clamp below stays exact.
fn perp_of(d: (f32, f32)) -> (f32, f32) {
    (-d.1, d.0)
}

fn side_idx(side: i8) -> usize {
    if side < 0 { 0 } else { 1 }
}

impl EngageRing {
    pub fn clear(&mut self) {
        self.choice.clear();
    }

    /// Assign one target point to each tank in `engaged` (entity, current
    /// position - callers pass these sorted by entity so the greedy order
    /// is stable frame to frame). A tank prefers the axis it already stands
    /// closest to and the nearer side of it, and any firing slot over any
    /// reserve slot. A slot must be reachable from the tank and have line
    /// of sight to the player; a tank that finds none gets no target (and
    /// falls back to steering at the player directly).
    ///
    /// Results land in `report`, whose `tanks` the caller fills beforehand
    /// (one entry per enemy; an `engaged` entity without one is still
    /// assigned, just not reported). Line of sight depends only on the
    /// slot, so it is checked once per slot per frame and memoised there.
    pub fn assign(&mut self, engaged: &[(Entity, Position)], ctx: &EngageCtx, report: &mut EngageReport) {
        report.built = true;
        for (i, slot) in report.slots.iter_mut().enumerate() {
            slot.point = engage_point(ctx, EngageSlot::from_index(i));
        }
        let mut claimed = [false; SLOT_COUNT];
        for &(entity, my_pos) in engaged {
            let to_me = (my_pos.x - ctx.player_pos.x, my_pos.y - ctx.player_pos.y);
            let bearing_len = (to_me.0 * to_me.0 + to_me.1 * to_me.1).sqrt().max(1.0);
            let bearing = (to_me.0 / bearing_len, to_me.1 / bearing_len);
            let mut axis_order = [0usize, 1, 2, 3];
            axis_order.sort_by(|&a, &b| {
                let da = DIRS[a].0 * bearing.0 + DIRS[a].1 * bearing.1;
                let db = DIRS[b].0 * bearing.0 + DIRS[b].1 * bearing.1;
                db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
            });
            let side_pref = |axis: usize| -> [i8; 2] {
                let perp = perp_of(DIRS[axis]);
                let dot = to_me.0 * perp.0 + to_me.1 * perp.1;
                if dot >= 0.0 { [1, -1] } else { [-1, 1] }
            };
            let mut rejected = Rejections::default();
            let slots = &mut report.slots;
            let mut check = |slot: EngageSlot, rejected: &mut Rejections| -> Option<Position> {
                let i = slot.index();
                if claimed[i] {
                    rejected.claimed += 1;
                    return None;
                }
                let Some(candidate) = slots[i].point else {
                    rejected.off_map += 1;
                    return None;
                };
                if !(ctx.reachable)(my_pos, candidate) {
                    rejected.unreachable += 1;
                    return None;
                }
                let los = *slots[i].line_of_sight.get_or_insert_with(|| (ctx.line_of_sight)(candidate, ctx.player_pos));
                if !los {
                    rejected.no_los += 1;
                    return None;
                }
                Some(candidate)
            };

            let sticky = self.choice.get(&entity).copied().and_then(|slot| check(slot, &mut rejected).map(|p| (slot, p)));
            let mut chosen = sticky;
            if chosen.is_none() {
                'search: for rank in 0u8..2 {
                    for &axis in &axis_order {
                        for side in side_pref(axis) {
                            let slot = EngageSlot { axis: axis as u8, rank, side };
                            if let Some(p) = check(slot, &mut rejected) {
                                chosen = Some((slot, p));
                                break 'search;
                            }
                        }
                    }
                }
            }
            match chosen {
                Some((slot, _)) => {
                    claimed[slot.index()] = true;
                    report.slots[slot.index()].claimed_by = Some(entity);
                    self.choice.insert(entity, slot);
                }
                None => {
                    self.choice.remove(&entity);
                }
            }
            if let Some(t) = report.tanks.iter_mut().find(|t| t.entity == entity) {
                t.slot = chosen.map(|(s, _)| s);
                t.target = chosen.map(|(_, p)| p);
                t.sticky = sticky.is_some();
                t.rejected = rejected;
            }
        }
    }
}

/// The world point for a slot, or `None` if it can't be kept inside the
/// battlefield. A rank-0 slot clamps its forward distance down to fit (but
/// never below ENGAGE_MIN_RADIUS, which keeps it out of the forced-misfire
/// zone); a rank-1 slot never clamps - a clamped reserve would land back in
/// the firing lane and recreate the pile-up it exists to avoid.
fn engage_point(ctx: &EngageCtx, slot: EngageSlot) -> Option<Position> {
    let dir = DIRS[slot.axis as usize];
    let perp = perp_of(dir);
    let lateral = slot.side as f32 * tuning().engage_lateral_offset;
    let (px, py, m) = (ctx.player_pos.x, ctx.player_pos.y, ctx.margin);
    // The lateral coordinate is independent of the forward distance: if
    // it's already against a wall no clamping can save this slot.
    let lat_x = px + perp.0 * lateral;
    let lat_y = py + perp.1 * lateral;
    if lat_x < m || lat_x > ctx.width - m || lat_y < m || lat_y > ctx.height - m {
        return None;
    }
    let mut forward = if slot.rank == 0 { tuning().engage_ring_radius() } else { tuning().engage_reserve_radius() };
    let room = if dir.0 > 0.0 {
        ctx.width - m - px
    } else if dir.0 < 0.0 {
        px - m
    } else if dir.1 > 0.0 {
        ctx.height - m - py
    } else {
        py - m
    };
    if slot.rank == 0 {
        forward = forward.min(room);
        if forward < tuning().engage_min_radius {
            return None;
        }
    } else if forward > room {
        return None;
    }
    Some(Position::new(px + dir.0 * forward + perp.0 * lateral, py + dir.1 * forward + perp.1 * lateral))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_field() -> (Position, f32, f32, f32) {
        (Position::new(640.0, 360.0), 1280.0, 720.0, 40.0)
    }

    fn entity(n: u32) -> Entity {
        Entity::from_bits(((1u64) << 32) | n as u64).expect("valid entity bits")
    }

    /// A report with one `Engaged` entry per tank, owner = its index + 1.
    fn report_for(engaged: &[(Entity, Position)]) -> EngageReport {
        EngageReport {
            tanks: engaged.iter().enumerate().map(|(i, &(e, _))| EngageTank::new(e, i + 1, EngageStatus::Engaged)).collect(),
            ..Default::default()
        }
    }

    fn assign(ring: &mut EngageRing, engaged: &[(Entity, Position)], ctx: &EngageCtx) -> EngageReport {
        let mut report = report_for(engaged);
        ring.assign(engaged, ctx, &mut report);
        report
    }

    #[test]
    fn slot_index_round_trips() {
        for i in 0..SLOT_COUNT {
            assert_eq!(EngageSlot::from_index(i).index(), i);
        }
        assert_eq!(EngageSlot { axis: 3, rank: 1, side: 1 }.index(), 15);
        assert_eq!(EngageSlot { axis: 1, rank: 0, side: -1 }.axis_name(), "right");
    }

    #[test]
    fn opposite_tanks_get_distinct_slots_on_their_own_axes() {
        let (player, w, h, margin) = open_field();
        let yes = |_: Position, _: Position| true;
        let ctx = EngageCtx { player_pos: player, width: w, height: h, margin, reachable: &yes, line_of_sight: &yes };
        let mut ring = EngageRing::default();
        let west = (entity(1), Position::new(200.0, 360.0));
        let east = (entity(2), Position::new(1100.0, 360.0));
        let report = assign(&mut ring, &[west, east], &ctx);
        let tw = report.target(west.0).unwrap();
        let te = report.target(east.0).unwrap();
        assert!(tw.x < player.x, "west tank should be assigned west of the player");
        assert!(te.x > player.x, "east tank should be assigned east of the player");
        assert!((tw.x - player.x).abs() > tuning().engage_min_radius && (te.x - player.x).abs() > tuning().engage_min_radius);
        assert_ne!(tw, te);
        assert!(report.built);
        assert_eq!(report.slot_of(west.0).unwrap().axis_name(), "left");
        assert_eq!(report.slot_of(east.0).unwrap().axis_name(), "right");
        assert_eq!(report.slots.iter().filter(|s| s.claimed_by.is_some()).count(), 2);
        assert!(report.tanks.iter().all(|t| !t.sticky && t.rejected.claimed == 0));
    }

    #[test]
    fn a_held_slot_is_kept_while_it_stays_valid() {
        let (player, w, h, margin) = open_field();
        let yes = |_: Position, _: Position| true;
        let ctx = EngageCtx { player_pos: player, width: w, height: h, margin, reachable: &yes, line_of_sight: &yes };
        let mut ring = EngageRing::default();
        let tank = entity(7);
        let first = assign(&mut ring, &[(tank, Position::new(200.0, 300.0))], &ctx);
        // Drift toward the south-west so a fresh search would now prefer a
        // different axis/side - the held slot must win regardless.
        let second = assign(&mut ring, &[(tank, Position::new(300.0, 500.0))], &ctx);
        assert_eq!(first.target(tank), second.target(tank));
        assert!(!first.tanks[0].sticky);
        assert!(second.tanks[0].sticky);
    }

    #[test]
    fn a_blocked_slot_yields_no_target() {
        let (player, w, h, margin) = open_field();
        let yes = |_: Position, _: Position| true;
        let no = |_: Position, _: Position| false;
        let ctx = EngageCtx { player_pos: player, width: w, height: h, margin, reachable: &no, line_of_sight: &yes };
        let mut ring = EngageRing::default();
        let report = assign(&mut ring, &[(entity(1), Position::new(200.0, 360.0))], &ctx);
        assert!(report.target(entity(1)).is_none());
        // The up/down reserve slots don't fit a 720px field from the centre.
        let r = report.tanks[0].rejected;
        assert_eq!((r.off_map, r.unreachable, r.no_los, r.claimed), (4, 12, 0, 0));
        assert!(report.slots.iter().all(|s| s.line_of_sight.is_none()), "unreachable slots never reach the LOS check");
    }

    #[test]
    fn a_cornered_player_loses_whole_axes_and_the_report_says_so() {
        let (_, w, h, margin) = open_field();
        let yes = |_: Position, _: Position| true;
        let player = Position::new(60.0, 60.0);
        let ctx = EngageCtx { player_pos: player, width: w, height: h, margin, reachable: &yes, line_of_sight: &yes };
        let mut ring = EngageRing::default();
        // From the north-west the tank tries the up and left axes first -
        // both fall outside the battlefield here.
        let tank = (entity(3), Position::new(30.0, 30.0));
        let report = assign(&mut ring, &[tank], &ctx);
        let off_map = report.slots.iter().filter(|s| s.point.is_none()).count();
        assert_eq!(off_map, 8, "the up and left axes are entirely off the map");
        assert_eq!(report.tanks[0].rejected.off_map, 4, "two sides of each of the two preferred axes");
        assert!(report.target(tank.0).is_some_and(|p| p.x > player.x || p.y > player.y));
    }
}
