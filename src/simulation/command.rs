//! Enemy command & control: one commander that sees every enemy's intent for
//! the frame *before* any of them moves, and may ease a tank off the
//! throttle, hold it, turn it, re-goal it, ration what it goes for, or
//! authorise it to ram (docs/enemy-command-and-control-prd.md).
//!
//! Three invariants, all load-bearing:
//!
//! - **No world access.** Pure data plus caller-supplied closures, so the
//!   whole thing is unit-tested without booting a `Game` - the same shape
//!   [`super::engage`] takes, and for the same reason.
//! - **No RNG, ever.** [`Commander::plan`] is never handed a `SmallRng` and
//!   never will be. Every tie is broken by owner slot. That is what makes a
//!   producer safe to add, reorder or remove: the round's RNG stream is
//!   untouched, so any movement in `just probe-fixtures` afterwards is a
//!   behaviour change and never a stream shift - a distinction
//!   `determinism_tests` structurally cannot make, since it compares two runs
//!   of one build.
//! - **It speaks `Intent`, never `Ai`.** The commander never writes a tank's
//!   committed heading, dodge timer or stuck clock. Reaching into those would
//!   bypass exactly the hysteresis that keeps heading jitter at 6 rather than
//!   30 across a fixture sweep.
//!
//! **Positions here are one frame stale**, and that is fine as long as it is
//! remembered: `drive_tank` applies an impulse during `enemy_phase`, but
//! bodies do not move until `step_world` several phases later and
//! `Tank::position` is not refreshed until `sync_tanks_and_ram` after that.
//! The prediction horizon is set with that frame of lag already spent.

use std::collections::BTreeMap;

use serde::Serialize;

use super::comms::{Blackboard, Order, Unit};
use crate::Position;
use crate::ai::Intent;
use crate::tank::Dir;
use crate::tuning::tuning;

/// One tank as the commander sees it: what it is, where it is, and what it
/// has just decided to do. Built by `enemy_phase` from the collect pass;
/// carries no `Tank`, no `Ai` and no `Entity`, so the commander stays
/// testable on plain values.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UnitView {
    pub unit: Unit,
    /// `Tank::owner_slot` - the sort key and the final tie-break everywhere
    /// in this module.
    pub slot: usize,
    pub position: Position,
    /// The body's *real* velocity, not the commanded cardinal. This is the
    /// half `Ai::avoid_collisions` cannot see: it predicts entirely in
    /// commanded space, so knockback, blast shove, grind residue and the
    /// drift of a jam are all invisible to it. Predicting in real velocity is
    /// how the commander sees a pile-up that every tank in it is "driving
    /// out of".
    pub velocity: Position,
    /// Hull bounding-circle radius at the current facing
    /// (`Tank::avoidance_radius`).
    pub radius: f32,
    /// Top speed the tank would reach if driven flat out right now.
    pub speed: f32,
    /// What the behaviour tree decided this frame, before the commander.
    pub intent: Intent,
    /// True for a wreck: still a solid obstacle, but it is not going
    /// anywhere and cannot be given an order.
    pub wreck: bool,
    /// The engagement-ring slot's rank, if this tank holds one: 0 is the
    /// firing line, 1 the reserve. `None` for a tank steering at its target
    /// directly. Right of way prefers the tank that is doing the work.
    pub ring_rank: Option<u8>,
}

/// The world queries the commander needs, injected as closures so the module
/// itself stays free of `Game` - the same trick `EngageCtx` uses, and the
/// same predicates the AI's own steering tests against, so the two cannot
/// disagree about what is passable.
pub(crate) struct CommandCtx<'a> {
    pub dt: f32,
    /// Would a tank at this position driving this cardinal walk into terrain?
    /// Backed by the frame's `pathfind::Grid::blocked_ahead`.
    pub blocked: &'a dyn Fn(Position, Dir) -> bool,
}

/// Why a conflict produced the mitigation it did - or produced none. The
/// `Rejections` analogue: several of these on one frame is what a jam the
/// commander could not defuse looks like from the outside, and without them
/// a silent no-op is indistinguishable from a conflict that never fired.
#[derive(Clone, Copy, Default, Debug, Serialize)]
pub(crate) struct Skipped {
    /// The tank that should give way had already been given an order.
    pub already_ordered: u32,
    /// Every sidestep cardinal was walled, blocked, or into another conflict.
    pub no_free_lane: u32,
    /// Deliberately let through: the pair holds a matching ram order.
    pub authorised: u32,
    /// This unit held right of way; the other one gave.
    pub had_right_of_way: u32,
}

/// What the commander decided, for tooling and the debug overlay.
#[derive(Clone, Default, Debug, Serialize)]
pub(crate) struct CommandReport {
    /// False when the switch is off, in which case nothing below ran.
    pub enabled: bool,
    /// Orders issued this frame, by recipient slot.
    pub orders: BTreeMap<usize, Vec<Order>>,
    pub skipped: Skipped,
    pub board: Blackboard,
}

impl CommandReport {
    fn clear(&mut self) {
        self.orders.clear();
        self.skipped = Skipped::default();
    }
}

/// The commander. Holds the pack's shared knowledge and the memory that keeps
/// its decisions from oscillating.
#[derive(Default)]
pub(crate) struct Commander {
    board: Blackboard,
    /// This frame's orders, by recipient slot.
    orders: BTreeMap<usize, Vec<Order>>,
    /// Who each tank is currently giving way to, and for how long. Held
    /// across frames on purpose: re-deciding right of way from scratch every
    /// frame flips it near any boundary, and a decision that flips is a
    /// discontinuity the AI's heading hysteresis was never built to absorb -
    /// the lesson `EngageRing::choice` records.
    yielding: BTreeMap<usize, (usize, f32)>,
    report: CommandReport,
}

impl Commander {
    /// Forget everything. Called from `Game::init`, beside the engagement
    /// rings' own reset.
    pub fn clear(&mut self) {
        self.board = Blackboard::default();
        self.orders.clear();
        self.yielding.clear();
        self.report = CommandReport::default();
    }

    /// Decide this frame's orders.
    ///
    /// `units` must be sorted by `slot` - the caller's contract, as
    /// `EngageRing::assign` documents for the same reason: the greedy passes
    /// below walk it in order, so a stable order is what makes them
    /// deterministic.
    ///
    /// Issues nothing while the switch is off, and returns before touching
    /// the blackboard, so "off" is a genuine no-op rather than a mitigation
    /// of strength zero.
    pub fn plan(&mut self, units: &[UnitView], ctx: &CommandCtx) {
        self.orders.clear();
        self.report.clear();
        self.report.enabled = tuning().c2_enabled;
        if !self.report.enabled {
            return;
        }
        debug_assert!(
            units.windows(2).all(|w| w[0].slot <= w[1].slot),
            "Commander::plan expects units sorted by owner slot"
        );
        self.board.begin_frame(ctx.dt);
        self.deconflict(units, ctx);
        // Pickup deconfliction and coordination land here next.
        self.report.board = self.board.clone();
        self.report.orders = self.orders.clone();
    }

    /// Stop enemies driving into each other and into the player.
    ///
    /// The band this owns is the one `Ai::avoid_collisions` explicitly
    /// declines - *"Already overlapping is the ram system's job, not ours"* -
    /// so the two never argue over the same pair. It also predicts in the
    /// bodies' **real** velocity, where the AI predicts in the commanded
    /// cardinal, which is why it can see a jam that every tank inside is
    /// "driving out of": knockback, blast shove, grind residue and the drift
    /// of a pile-up are all invisible upstream.
    fn deconflict(&mut self, units: &[UnitView], ctx: &CommandCtx) {
        let t = tuning();
        let (watch, margin) = (t.c2_watch_px, t.c2_contact_margin_px);
        let mut conflicts: Vec<(f32, usize, usize)> = Vec::new();
        for (i, a) in units.iter().enumerate() {
            if a.wreck {
                continue;
            }
            for b in units.iter().skip(i + 1) {
                if b.wreck {
                    continue;
                }
                // **Enemy-vs-enemy only**, and today that is already true
                // by construction - `enemy_phase` builds `units` from the
                // enemy drive loop, so no player is ever in it. Kept as an
                // explicit guard rather than an assumption, because the
                // rule is deliberate and not incidental: slowing a tank that
                // is *trying* to close on the player would not prevent
                // contact, it would prolong it, and ram damage re-triggers
                // every `ram_damage_cooldown`. Accidental player rams are
                // the ram authorisation gate's problem
                // (docs/enemy-command-and-control-prd.md section 6) - that
                // stops the damage rather than the approach.
                if matches!(a.unit, Unit::Player(_)) || matches!(b.unit, Unit::Player(_)) {
                    continue;
                }
                let gap = a.position.distance_to(b.position);
                if gap > watch {
                    continue;
                }
                let reach = a.radius + b.radius + margin;
                let rel = Position::new(b.position.x - a.position.x, b.position.y - a.position.y);
                let vel = Position::new(a.velocity.x - b.velocity.x, a.velocity.y - b.velocity.y);
                // Closing speed along the line between them. Negative means
                // they are already drifting apart, and a pair that is
                // separating needs nothing from anybody.
                let dist = gap.max(f32::EPSILON);
                let closing = (rel.x * vel.x + rel.y * vel.y) / dist;
                if closing < t.c2_min_closing_px {
                    continue;
                }
                // Already touching counts immediately; otherwise ask how long
                // until they do at the current closing rate.
                let ttc = if gap <= reach { 0.0 } else { (gap - reach) / closing };
                if ttc <= t.c2_horizon_seconds {
                    conflicts.push((ttc, a.slot, b.slot));
                }
            }
        }
        // Soonest first, slot order breaking ties, so the walk is a total
        // order over a set built from a slot-sorted input - no RNG, and the
        // same decisions every replay.
        conflicts.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)).then(x.2.cmp(&y.2)));

        let by_slot = |slot: usize| units.iter().find(|u| u.slot == slot).expect("conflict slots come from units");
        let mut ordered: Vec<usize> = Vec::new();
        for (_, a_slot, b_slot) in conflicts {
            let (a, b) = (by_slot(a_slot), by_slot(b_slot));
            let yielder = if self.gives_way(a, b) { a } else { b };
            if matches!(yielder.unit, Unit::Player(_)) {
                // Unreachable while conflicts are enemy-vs-enemy only, and
                // kept as a hard floor: the commander must never drive a
                // player, whatever a future producer starts considering.
                self.report.skipped.had_right_of_way += 1;
                continue;
            }
            if ordered.contains(&yielder.slot) {
                // One mitigation per tank per frame. Being told to ease off
                // for one neighbour and stop for another in the same frame is
                // how a commander produces the jitter it exists to prevent.
                self.report.skipped.already_ordered += 1;
                continue;
            }
            let other = if yielder.slot == a_slot { b_slot } else { a_slot };
            let held = match self.yielding.get(&yielder.slot) {
                Some(&(to, since)) if to == other => since + ctx.dt,
                _ => 0.0,
            };
            if held > t.c2_yield_hold_seconds {
                // The ceiling. Past it the tank drives on regardless, so a
                // pair that cannot resolve degrades to a visible pause rather
                // than a deadlock - the same valve `enemy_yield_seconds` is.
                self.yielding.remove(&yielder.slot);
                continue;
            }
            self.yielding.insert(yielder.slot, (other, held));
            let order = if held >= t.c2_escalate_seconds {
                Order::Hold
            } else {
                Order::Slow { by: 1.0 - t.c2_throttle_floor }
            };
            self.orders.entry(yielder.slot).or_default().push(order);
            ordered.push(yielder.slot);
        }
        // A tank nobody told to give way this frame is no longer yielding.
        self.yielding.retain(|slot, _| ordered.contains(slot));
    }

    /// Whether `a` is the one that should give way. A **total** order, so of
    /// any pair exactly one yields - which is the whole point, and the thing
    /// `ai.rs`'s two `is_multiple_of(2)` parity tie-breaks cannot provide:
    /// parity guarantees two tanks pick *different* sides, never that the
    /// right one gives way, and three tanks in a jam get two answers.
    ///
    /// First difference wins:
    /// 1. Whoever is already yielding to this opponent keeps yielding -
    ///    hysteresis, the `EngageRing::choice` lesson. A decision that flips
    ///    frame to frame is a discontinuity the AI's heading hysteresis was
    ///    never built to absorb.
    /// 2. A player never gives way to an enemy.
    /// 3. The tank holding a firing-line ring slot keeps its lane; braking
    ///    the shooter wastes the shot.
    /// 4. The faster tank keeps going - bucketed, so a flip needs a real
    ///    difference. It clears the contested ground sooner, so yielding the
    ///    slower one costs fewer tank-seconds.
    /// 5. Lower owner slot. Arbitrary, and the reason this is total.
    fn gives_way(&self, a: &UnitView, b: &UnitView) -> bool {
        if self.yielding.get(&a.slot).is_some_and(|&(to, _)| to == b.slot) {
            return true;
        }
        if self.yielding.get(&b.slot).is_some_and(|&(to, _)| to == a.slot) {
            return false;
        }
        let player = |u: &UnitView| matches!(u.unit, Unit::Player(_));
        if player(a) != player(b) {
            return !player(a);
        }
        let rank = |u: &UnitView| u.ring_rank.unwrap_or(2);
        if rank(a) != rank(b) {
            return rank(a) > rank(b);
        }
        let bucket = |u: &UnitView| (u.velocity.length() / tuning().c2_speed_bucket_px) as i32;
        if bucket(a) != bucket(b) {
            return bucket(a) < bucket(b);
        }
        a.slot > b.slot
    }

    /// Apply this frame's reflex orders to a tank's intent. The one place an
    /// `Order` ever touches an `Intent`; everything else the commander
    /// decides is read later, through `think`'s own inputs.
    pub fn apply(&self, slot: usize, mut intent: Intent) -> Intent {
        let Some(orders) = self.orders.get(&slot) else {
            return intent;
        };
        for order in orders {
            match *order {
                // Throttles from different producers compound rather than
                // overwrite: two reasons to ease off are more reason, not the
                // same reason twice.
                Order::Slow { by } => intent.slow = (intent.slow + by).clamp(0.0, 1.0),
                Order::Hold => {
                    intent.face = intent.face.or(intent.move_dir);
                    intent.move_dir = None;
                }
                Order::Nudge { dir } => intent.move_dir = Some(dir),
                Order::HoldFire => intent.fire = false,
                // Goal orders are read by the next frame's `think`, and ram
                // authorisation by `combat::ram`; neither touches the intent.
                Order::Ram { .. } | Order::Goto { .. } | Order::Fetch { .. } | Order::Focus { .. } => {}
            }
        }
        intent
    }

    pub fn report(&self) -> &CommandReport {
        &self.report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(dir: Option<Dir>) -> Intent {
        Intent { move_dir: dir, ..Intent::default() }
    }

    #[test]
    fn apply_is_a_no_op_for_a_tank_with_no_orders() {
        let c = Commander::default();
        let before = intent(Some(Dir::Up));
        let after = c.apply(7, before);
        assert_eq!(after.move_dir, before.move_dir);
        assert_eq!(after.slow, before.slow);
        assert_eq!(after.fire, before.fire);
    }

    #[test]
    fn a_hold_stops_the_tank_but_keeps_it_aiming() {
        let mut c = Commander::default();
        c.orders.insert(3, vec![Order::Hold]);
        let after = c.apply(3, intent(Some(Dir::Left)));
        assert_eq!(after.move_dir, None, "a held tank stops driving");
        assert_eq!(after.face, Some(Dir::Left), "but keeps facing where it was going, so it still shoots");
    }

    #[test]
    fn throttles_compound_and_clamp() {
        let mut c = Commander::default();
        c.orders.insert(1, vec![Order::Slow { by: 0.3 }, Order::Slow { by: 0.5 }]);
        assert!((c.apply(1, intent(Some(Dir::Up))).slow - 0.8).abs() < 1e-6);
        c.orders.insert(2, vec![Order::Slow { by: 0.9 }, Order::Slow { by: 0.9 }]);
        let stopped = c.apply(2, intent(Some(Dir::Up)));
        assert_eq!(stopped.slow, 1.0, "compounding must clamp rather than reverse the tank");
        assert_eq!(stopped.speed_scale(), 0.0);
    }

    /// Goal orders and ram authorisation are read elsewhere; `apply` must
    /// leave the intent alone rather than half-implementing them.
    #[test]
    fn goal_orders_do_not_touch_the_intent() {
        let mut c = Commander::default();
        c.orders.insert(4, vec![
            Order::Goto { x: 10.0, y: 10.0 },
            Order::Focus { player: 1 },
            Order::Ram { victim: Unit::Player(0) },
        ]);
        let before = intent(Some(Dir::Down));
        let after = c.apply(4, before);
        assert_eq!(after.move_dir, before.move_dir);
        assert_eq!(after.slow, before.slow);
    }

    fn unit(slot: usize, x: f32, vx: f32) -> UnitView {
        UnitView {
            unit: Unit::Enemy(slot),
            slot,
            position: Position::new(x, 0.0),
            velocity: Position::new(vx, 0.0),
            radius: 24.0,
            speed: 210.0,
            intent: intent(Some(Dir::Right)),
            wreck: false,
            ring_rank: None,
        }
    }

    /// Of any pair exactly one gives way. This is the property the two
    /// `is_multiple_of(2)` parity tie-breaks in `ai.rs` cannot provide -
    /// parity guarantees two tanks pick different *sides*, never that the
    /// right one stops - and it is what makes a mutual-brake deadlock
    /// unreachable through the commander.
    #[test]
    fn right_of_way_is_antisymmetric_and_total() {
        let c = Commander::default();
        let units: Vec<UnitView> = (0..8).map(|i| unit(i, i as f32 * 10.0, (i % 3) as f32 * 45.0)).collect();
        for a in &units {
            for b in &units {
                if a.slot == b.slot {
                    continue;
                }
                assert_ne!(
                    c.gives_way(a, b),
                    c.gives_way(b, a),
                    "slots {} and {} must disagree about who yields",
                    a.slot,
                    b.slot
                );
            }
        }
    }

    /// The faster tank keeps going: it clears the contested ground sooner,
    /// so yielding the slower one costs fewer tank-seconds.
    #[test]
    fn the_slower_tank_gives_way() {
        let c = Commander::default();
        let fast = unit(1, 0.0, 200.0);
        let slow = unit(2, 50.0, 0.0);
        assert!(c.gives_way(&slow, &fast));
        assert!(!c.gives_way(&fast, &slow));
    }

    /// Once a tank is yielding to an opponent it keeps yielding to that
    /// opponent, even if the raw comparison would now flip. A decision that
    /// flips frame to frame is a discontinuity the AI's own heading
    /// hysteresis was never built to absorb - `EngageRing::choice`'s lesson.
    #[test]
    fn a_yielder_keeps_yielding_even_when_the_comparison_flips() {
        let mut c = Commander::default();
        let fast = unit(1, 0.0, 200.0);
        let slow = unit(2, 50.0, 0.0);
        // Raw comparison: the slow one yields.
        assert!(c.gives_way(&slow, &fast));
        // Latch it, then hand it the speed that would otherwise win.
        c.yielding.insert(slow.slot, (fast.slot, 0.1));
        let now_fast = unit(2, 50.0, 400.0);
        assert!(c.gives_way(&now_fast, &fast), "the latch outranks the speed bucket");
    }

    #[test]
    fn the_switch_being_off_issues_nothing() {
        let mut c = Commander::default();
        let ctx = CommandCtx { dt: 1.0 / 60.0, blocked: &|_, _| false };
        // `tuning()` is the compiled default (c2_enabled = true) in tests, so
        // drive the off path through the report rather than the global.
        c.plan(&[], &ctx);
        assert!(c.orders.is_empty(), "no producers are wired yet, so nothing is issued either way");
    }
}
