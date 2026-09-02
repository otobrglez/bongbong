//! Engagement slots: every enemy actually attacking claims a distinct point
//! near the player instead of converging on the same spot. Four cardinal
//! axes through the player, each with two lateral firing slots at
//! ENGAGE_RING_RADIUS (rank 0, both within ENEMY_FIRE_ALIGN_PX of the axis
//! so either can fire) and two reserve slots at ENGAGE_RESERVE_RADIUS (rank
//! 1, beyond attack range so a reserve tank neither fires nor blocks a
//! lane) - 16 points, claimed greedily with mutual exclusion each frame.
//! Pure geometry plus two callbacks; no world access, so it is unit-tested
//! on its own.

use crate::tuning::tuning;
use std::collections::HashMap;

use hecs::Entity;

use crate::{Position};

/// One slot identity: cardinal axis (0 = N, 1 = E, 2 = S, 3 = W), rank (0
/// firing line, 1 reserve) and lateral side (-1/+1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EngageSlot {
    axis: u8,
    rank: u8,
    side: i8,
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

const DIRS: [(f32, f32); 4] = [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)];

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
    /// of sight to the player; a tank that finds none gets no entry (and
    /// falls back to steering at the player directly).
    pub fn assign(&mut self, engaged: &[(Entity, Position)], ctx: &EngageCtx) -> HashMap<Entity, Position> {
        let mut targets = HashMap::new();
        let mut claimed = [[[false; 2]; 2]; 4];
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
            let valid = |slot: EngageSlot| -> Option<Position> {
                if claimed[slot.axis as usize][slot.rank as usize][side_idx(slot.side)] {
                    return None;
                }
                let candidate = engage_point(ctx, slot)?;
                ((ctx.reachable)(my_pos, candidate) && (ctx.line_of_sight)(candidate, ctx.player_pos))
                    .then_some(candidate)
            };

            let sticky = self.choice.get(&entity).and_then(|&slot| valid(slot).map(|p| (slot, p)));
            let chosen = sticky.or_else(|| {
                (0u8..2).find_map(|rank| {
                    axis_order.iter().find_map(|&axis| {
                        side_pref(axis).into_iter().find_map(|side| {
                            let slot = EngageSlot { axis: axis as u8, rank, side };
                            valid(slot).map(|p| (slot, p))
                        })
                    })
                })
            });
            match chosen {
                Some((slot, point)) => {
                    claimed[slot.axis as usize][slot.rank as usize][side_idx(slot.side)] = true;
                    self.choice.insert(entity, slot);
                    targets.insert(entity, point);
                }
                None => {
                    self.choice.remove(&entity);
                }
            }
        }
        targets
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

    #[test]
    fn opposite_tanks_get_distinct_slots_on_their_own_axes() {
        let (player, w, h, margin) = open_field();
        let yes = |_: Position, _: Position| true;
        let ctx = EngageCtx { player_pos: player, width: w, height: h, margin, reachable: &yes, line_of_sight: &yes };
        let mut ring = EngageRing::default();
        let west = (entity(1), Position::new(200.0, 360.0));
        let east = (entity(2), Position::new(1100.0, 360.0));
        let targets = ring.assign(&[west, east], &ctx);
        let tw = targets[&west.0];
        let te = targets[&east.0];
        assert!(tw.x < player.x, "west tank should be assigned west of the player");
        assert!(te.x > player.x, "east tank should be assigned east of the player");
        assert!((tw.x - player.x).abs() > tuning().engage_min_radius && (te.x - player.x).abs() > tuning().engage_min_radius);
        assert_ne!(tw, te);
    }

    #[test]
    fn a_held_slot_is_kept_while_it_stays_valid() {
        let (player, w, h, margin) = open_field();
        let yes = |_: Position, _: Position| true;
        let ctx = EngageCtx { player_pos: player, width: w, height: h, margin, reachable: &yes, line_of_sight: &yes };
        let mut ring = EngageRing::default();
        let tank = entity(7);
        let first = ring.assign(&[(tank, Position::new(200.0, 300.0))], &ctx)[&tank];
        // Drift toward the south-west so a fresh search would now prefer a
        // different axis/side - the held slot must win regardless.
        let second = ring.assign(&[(tank, Position::new(300.0, 500.0))], &ctx)[&tank];
        assert_eq!(first, second);
    }

    #[test]
    fn a_blocked_slot_yields_no_target() {
        let (player, w, h, margin) = open_field();
        let yes = |_: Position, _: Position| true;
        let no = |_: Position, _: Position| false;
        let ctx = EngageCtx { player_pos: player, width: w, height: h, margin, reachable: &no, line_of_sight: &yes };
        let mut ring = EngageRing::default();
        let targets = ring.assign(&[(entity(1), Position::new(200.0, 360.0))], &ctx);
        assert!(targets.is_empty());
    }
}
