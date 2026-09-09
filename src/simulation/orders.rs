//! Tap-to-command: the player's standing order, and the `Intent` it drives
//! the tank with (docs/tap-navigation.md).
//!
//! A tap is an *order*, not a nudge: it survives until the target dies, the
//! destination is reached, the route turns out not to exist, or the player
//! touches an arrow key. Keys always win - `player_phase` clears the order
//! before anything else - so the keyboard is never fighting the assist for
//! the same frame.
//!
//! **Why the tap arrives in `Input` rather than being read here.** A round
//! has to replay bit-for-bit from `(seed, inputs)` (CLAUDE.md's
//! seeded-replay rule). Reading the mouse inside the simulation, or keeping
//! the order outside the input stream, breaks both replay and the probe.
//! Everything in this module is a pure function of the world plus that one
//! `Position`.
//!
//! **The hard part is not pathfinding, it is shooting at something that
//! moves.** Movement is 4-direction, so the assist has to stand on the
//! target's row or column to fire down it - and a shell is in flight for
//! `range / shell_speed` while the target keeps going. At full
//! `enemy_attack_range` that is about 0.68 s, in which an enemy at
//! `enemy_speed` crosses ~109 px against a `enemy_fire_align_px` window of
//! 24. The alignment test is about the moment of *firing*; the hit depends
//! on the moment of *arrival*. Three things follow, and all three are in
//! `engage` below:
//!
//! 1. A shot whose target will have left the window by the time the shell
//!    lands is not taken at all. Ammo is 20 shells; spending one on a
//!    geometrically impossible shot is worse than waiting.
//! 2. The firing position is chosen *along* the target's travel where
//!    possible - in front of or behind a mover, where drift during flight is
//!    near zero - rather than across it.
//! 3. Alignment is hysteretic and the sidestep direction is committed for a
//!    beat, so the tank does not shuffle as the target crosses the boundary.
//!
//! Deliberately absent: leading the shot. Firing at the intercept would hit
//! movers reliably and would make assisted play better than both the AI and
//! a hand-aiming human, which is the one thing the accuracy decision rules
//! out.

use hecs::Entity;
use sola_raylib::prelude::Vector2;

use super::{Game, Frame, with_frog, with_tank};
use crate::ai::{Intent, axis_offsets, opposite, perpendicular};
use crate::obstacle::Obstacle;
use crate::pickup::Pickup;
use crate::pathfind::{Components, Grid};
use crate::tank::{Dir, Tank};
use crate::tuning::tuning;
use crate::frog::Frog;
use crate::Position;

/// What a tap landed on. Only used to colour the order ring and to read the
/// order back out in tooling - the driving code treats every target the
/// same way.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderTarget {
    Tank,
    Frog,
    Tile,
    Pickup,
}

/// The player's standing order.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Order {
    /// Drive to a point and stop.
    Move { to: Position },
    /// Close on something and shoot it until it is gone.
    Engage { target: Entity, kind: OrderTarget },
    /// Drive onto a pickup and take it. Separate from `Move` because the
    /// end condition is the pickup being *gone*, not the tank arriving:
    /// collection is a proximity test (`pickup_collect_radius`) that a
    /// move order's own arrival slop could stop just short of.
    Collect { pickup: Entity },
}

/// The order plus the small amount of memory driving it needs. Kept on
/// `Game` so it is part of the simulation state a replay reproduces.
pub struct PlayerOrders {
    pub current: Option<Order>,
    /// Seconds the aim has been settled on this axis, gating the shot the
    /// same way `enemy_aim_settle` gates the AI's.
    aim_settle: f32,
    /// Whether the last frame counted as aligned. The alignment test is
    /// hysteretic - it takes a wider miss to *lose* alignment than to gain
    /// it - so a target hovering on the boundary does not make the tank
    /// stutter between lining up and sidestepping.
    aligned: bool,
    /// The sidestep direction currently committed to, and for how long.
    /// Same idea as `Ai`'s `committed_dir`/`dir_hold`.
    committed: Option<Dir>,
    dir_hold: f32,
    /// Where the tank was last frame and how far it got since, measured
    /// once per frame in `order_intent` so every branch sees the same
    /// number - the aligned and creep branches never reach `commit_dir`,
    /// and reading a stale `last_pos` from them made both the stuck escape
    /// and the give-up below fire on invented displacements.
    ///
    /// Displacement, never a physics velocity: a tank pressed against
    /// something is handed a velocity along its heading every frame without
    /// going anywhere.
    last_pos: Option<Position>,
    moved: f32,
    stuck_timer: f32,
    /// The closest to a firing line this engagement has managed, how far
    /// the tank has driven since it last got any closer, and how long that
    /// has been going on. An engagement that neither improves its aim nor
    /// covers any ground has stopped working - see `drive_engage`.
    best_off_axis: f32,
    give_up_travel: f32,
    give_up: f32,
}

impl Default for PlayerOrders {
    fn default() -> Self {
        Self {
            current: None,
            aim_settle: 0.0,
            aligned: false,
            committed: None,
            dir_hold: 0.0,
            last_pos: None,
            moved: 0.0,
            stuck_timer: 0.0,
            // No firing line has been managed yet, so anything counts as an
            // improvement on it.
            best_off_axis: f32::MAX,
            give_up_travel: 0.0,
            give_up: 0.0,
        }
    }
}

impl PlayerOrders {
    pub fn clear(&mut self) {
        *self = PlayerOrders::default();
    }

    pub fn is_active(&self) -> bool {
        self.current.is_some()
    }

    /// What the order ring should show: `None` with no order running,
    /// `Some(false)` for a move, `Some(true)` for an engagement.
    pub fn ring_engage(&self) -> Option<bool> {
        match self.current {
            None => None,
            Some(Order::Move { .. }) | Some(Order::Collect { .. }) => Some(false),
            Some(Order::Engage { .. }) => Some(true),
        }
    }
}

impl Game {
    /// Turn a tap into an order. Anything shootable under the point becomes
    /// `Engage`; everything else is a `Move` to the nearest cell a tank can
    /// actually stand in.
    pub(super) fn resolve_tap(&self, at: Position, grid: &Grid, comps: &Components) -> Option<Order> {
        let player = self.player?;
        let me = with_tank(&self.world, player, |t| t.position);
        // A live enemy hull first: the most likely thing anyone is aiming
        // at, and the only one worth a generous hit box.
        let on_tank = self
            .world
            .query::<(Entity, &Tank)>()
            .with::<&crate::ai::Ai>()
            .iter()
            .filter(|(e, t)| *e != player && !t.is_wreck())
            .find(|(_, t)| within(at, t.position, t.size() * 0.5))
            .map(|(e, _)| e);
        if let Some(target) = on_tank {
            return Some(Order::Engage { target, kind: OrderTarget::Tank });
        }
        // The enemy frog is the Hunt mission's objective - without this a
        // touch player could never finish one.
        if let Some(frog) = self.enemy_frog {
            let hit = with_frog(&self.world, frog, |fr| {
                !fr.is_dead() && within(at, fr.position, crate::FROG_COLLIDER_HALF_EXTENT.0.max(crate::FROG_COLLIDER_HALF_EXTENT.1))
            });
            if hit {
                return Some(Order::Engage { target: frog, kind: OrderTarget::Frog });
            }
        }
        // A pickup: an explicit "go and get that" order. It cannot just
        // fall out as a `Move` - the move would snap to the nearest
        // *reachable cell*, which is routinely the cell next door, and the
        // tank would park beside the health pack without ever touching it.
        let on_pickup = self
            .world
            .query::<(Entity, &Pickup)>()
            .iter()
            .find(|(_, p)| within(at, p.position, p.size() * 0.5))
            .map(|(e, _)| e);
        if let Some(pickup) = on_pickup {
            return Some(Order::Collect { pickup });
        }
        // A destructible tile: tap a barrel to pop it, a wall or a tree to
        // breach it. Iron never dies, so tapping it is a move order and the
        // tank paths up beside it instead of grinding away at it forever.
        let on_tile = self
            .world
            .query::<(Entity, &Obstacle)>()
            .iter()
            .filter(|(_, o)| !o.destroyed && !o.material.is_permanent())
            .find(|(_, o)| within(at, o.position, o.size() * 0.5))
            .map(|(e, _)| e);
        if let Some(target) = on_tile {
            return Some(Order::Engage { target, kind: OrderTarget::Tile });
        }
        // Best effort, not all-or-nothing: head for the closest point to
        // the tap that is actually reachable from here. A strict "route to
        // the exact spot or nothing" left 81% of taps on open ground doing
        // nothing at all on the shipped map, because `nearest_open` happily
        // snaps into a pocket with no route to it.
        grid.nearest_reachable(at, me, comps).map(|to| Order::Move { to })
    }

    /// The `Intent` the standing order wants this frame, or `None` when
    /// there is no order (or it just finished, in which case it is cleared
    /// here and the player coasts).
    pub(super) fn order_intent(&mut self, f: &mut Frame, grid: &Grid, comps: &Components) -> Option<Intent> {
        let player = self.player?;
        let order = self.orders.current?;
        let me = with_tank(&self.world, player, |t| MeSnapshot {
            position: t.position,
            wreck: t.is_wreck(),
        });
        if me.wreck {
            self.orders.clear();
            return None;
        }
        self.orders.moved = self.orders.last_pos.map_or(0.0, |p| p.distance_to(me.position));
        self.orders.last_pos = Some(me.position);
        match order {
            Order::Move { to } => self.drive_move(f, grid, me.position, to),
            Order::Engage { target, .. } => self.drive_engage(f, grid, comps, me.position, target),
            Order::Collect { pickup } => self.drive_collect(f, grid, comps, me.position, pickup),
        }
    }

    /// Drive onto a pickup. Ends when the pickup is gone - collected by
    /// this tank, taken by someone else, or despawned - rather than on
    /// arrival, so the order cannot finish a few px short of the
    /// `pickup_collect_radius` that actually picks it up.
    fn drive_collect(&mut self, f: &mut Frame, grid: &Grid, comps: &Components, from: Position, pickup: Entity) -> Option<Intent> {
        let at = {
            let mut q = self.world.query_one::<&Pickup>(pickup);
            match q.get() {
                Ok(p) => p.position,
                Err(_) => {
                    self.orders.clear();
                    return None;
                }
            }
        };
        // Best effort again, and this is the case that needed it most: a
        // pickup tucked against a wall sits in a nav-blocked cell, so
        // pathing to the pickup's own position finds no route and the order
        // died on its first frame. Every tap on a health or ammo pack did
        // nothing at all until this aimed at the reachable cell beside it.
        let goal = grid.nearest_reachable(at, from, comps).unwrap_or(at);
        match grid.next_step(from, goal) {
            Some(step) => Some(Intent { move_dir: Some(self.commit_dir(f, grid, from, step)), ..Intent::default() }),
            // In the pickup's own cell but not yet inside
            // `pickup_collect_radius`: close the last few px straight at it.
            None if grid.same_cell(from, at) => {
                Some(Intent { move_dir: Some(Dir::toward(from, at)), ..Intent::default() })
            }
            // Standing at the closest reachable point and the pickup is
            // still there - it is walled off (the map linter calls these
            // `gated-pickup`). Stop rather than grind.
            None => {
                self.orders.clear();
                None
            }
        }
    }

    fn drive_move(&mut self, f: &mut Frame, grid: &Grid, from: Position, to: Position) -> Option<Intent> {
        if grid.same_cell(from, to) || from.distance_to(to) <= tuning().order_arrive_px {
            self.orders.clear();
            return None;
        }
        match grid.next_step(from, to) {
            Some(step) => Some(Intent { move_dir: Some(self.commit_dir(f, grid, from, step)), ..Intent::default() }),
            // `next_step` conflates "arrived" and "no route" into `None`,
            // and the same-cell case is already handled above - so this is
            // genuinely unreachable. End the order rather than spin.
            None => {
                self.orders.clear();
                None
            }
        }
    }

    fn drive_engage(&mut self, f: &mut Frame, grid: &Grid, comps: &Components, from: Position, target: Entity) -> Option<Intent> {
        let Some(state) = self.target_state(target) else {
            self.orders.clear();
            return None;
        };
        let t = tuning();
        let range = from.distance_to(state.position);
        if range > t.enemy_attack_range {
            // Too far to shoot: just close the distance.
            self.orders.aim_settle = 0.0;
            self.orders.aligned = false;
            // Best effort, as with a move: a wall buried inside a structure
            // has no route *to* it, but you do not need one - you need to
            // get within `enemy_attack_range` and shoot. Pathing to the
            // target's own cell instead simply cleared the order, which is
            // why tapping most solid tiles used to do nothing.
            let approach = grid.nearest_reachable(state.position, from, comps).unwrap_or(state.position);
            return match grid.next_step(from, approach) {
                Some(step) => self.drive(f, grid, from, step),
                None => {
                    self.orders.clear();
                    None
                }
            };
        }

        let dir = Dir::toward(from, state.position);
        let (off_axis, forward) = axis_offsets(from, state.position, dir);
        let window = hit_window(&state, dir);
        // Hysteresis: it takes a wider miss to lose alignment than to gain
        // it, so a target sitting on the boundary cannot make the tank
        // stutter between holding and sidestepping.
        let lose = window + t.order_align_hysteresis_px;
        let aligned = forward > 0.0 && off_axis <= if self.orders.aligned { lose } else { window };
        self.orders.aligned = aligned;

        if aligned && self.terrain_sees(f, from, near_side(from, &state)) {
            self.orders.give_up = 0.0;
            self.orders.give_up_travel = 0.0;
            self.orders.best_off_axis = f32::MAX;
            self.orders.aim_settle += f.dt;
            let mut intent = Intent { face: Some(dir), ..Intent::default() };
            if self.orders.aim_settle >= t.enemy_aim_settle && can_land(range, off_axis, state.velocity, dir, window) {
                intent.fire = true;
                // Release the trigger by resetting the settle timer. Shells
                // and plasma are edge-triggered in `player_phase` - a held
                // `fire` fires exactly once and then never again - so the
                // assist has to pulse. Re-settling is also the cadence: one
                // shot per `enemy_aim_settle`, still floored by the weapon's
                // own `fire_cooldown`.
                self.orders.aim_settle = 0.0;
            }
            return Some(intent);
        }

        self.orders.aim_settle = 0.0;
        // Give up on an engagement that is neither improving its aim nor
        // going anywhere - a target with no reachable firing line at all,
        // which used to mean grinding into a wall for as long as the player
        // let it.
        //
        // Both halves are load-bearing, and each was measured. A plain "no
        // shot in N seconds" clock cost a quarter of the kills, because an
        // engagement that still needs a long sidestep is working fine. Aim
        // progress alone is not enough either: chasing a live enemy across
        // open ground, `off_axis` swings around with the target and
        // regularly fails to beat its own best for seconds at a time while
        // the tank is very much still in the fight. Only a tank that has
        // stopped closing *and* stopped moving has actually given up.
        self.orders.give_up_travel += self.orders.moved;
        if off_axis < self.orders.best_off_axis - 1.0 {
            self.orders.best_off_axis = off_axis;
        }
        if off_axis <= self.orders.best_off_axis || self.orders.give_up_travel >= t.order_engage_give_up_px {
            self.orders.give_up = 0.0;
            self.orders.give_up_travel = 0.0;
        } else {
            self.orders.give_up += f.dt;
            if self.orders.give_up >= t.order_engage_give_up_seconds {
                self.orders.clear();
                return None;
            }
        }

        // Sidestep onto a firing line. Of the two axes, take the first one
        // that is actually *standable* - `next_step`'s A* treats its goal
        // cell as open whether or not it is, so aiming at a firing spot
        // buried in a wall produced a route that ended by driving into the
        // wall and staying there.
        let stand = firing_spots(from, &state)
            .into_iter()
            .find(|&spot| grid.usable(spot) && comps.connected(grid, from, spot));

        match stand {
            // The sidestep is inside the tank's own nav cell. The grid
            // cannot express it - `next_step` returns `None` for a goal in
            // the start cell - so this is where the last few px of
            // alignment have to come from, driven straight rather than
            // through the router. Without it the tank fell through to
            // "close on the target" and charged instead of lining up,
            // which is why a tapped enemy was shot at from a diagonal.
            Some(spot) if grid.same_cell(from, spot) => {
                let creep = Dir::toward(from, spot);
                // Half the window is a deadzone: the tank decelerates on a
                // curve, so releasing exactly on the line overshoots it.
                // Stopping short leaves it inside the window with the
                // remaining drift still heading the right way.
                if off_axis <= window * 0.5 || grid.blocked_ahead(from, creep.vec()) {
                    Some(Intent { face: Some(dir), ..Intent::default() })
                } else {
                    Some(Intent { move_dir: Some(creep), ..Intent::default() })
                }
            }
            Some(spot) => match grid.next_step(from, spot) {
                Some(step) => self.drive(f, grid, from, step),
                None => self.close_on(f, grid, from, state.position),
            },
            // Neither firing line is reachable - close on the target, which
            // is what the AI does when its slot is untenable.
            None => self.close_on(f, grid, from, state.position),
        }
    }

    /// Drive one committed step toward `step`, the shape every movement
    /// branch above ends in.
    fn drive(&mut self, f: &mut Frame, grid: &Grid, from: Position, step: Position) -> Option<Intent> {
        Some(Intent { move_dir: Some(self.commit_dir(f, grid, from, step)), ..Intent::default() })
    }

    fn close_on(&mut self, f: &mut Frame, grid: &Grid, from: Position, target: Position) -> Option<Intent> {
        match grid.next_step(from, target) {
            Some(step) => self.drive(f, grid, from, step),
            // Already in the target's cell and still not aligned: nudge
            // straight at it rather than standing still.
            None => Some(Intent { move_dir: Some(Dir::toward(from, target)), ..Intent::default() }),
        }
    }

    /// Whether a shot fired now can still be inside the alignment window when
    /// it arrives.
    ///
    /// A shell is in flight for `range / shell_speed`; a target crossing the
    /// firing line keeps moving for all of it. Taking the shot anyway spends
    /// one of twenty shells on something geometrically impossible, so this
    /// holds fire instead - the tank keeps its aim and shoots the moment the
    /// geometry works.
    fn terrain_sees(&self, f: &Frame, from: Position, to: Position) -> bool {
        f.terrain.line_of_sight(from, to)
    }

    /// Where the target is, how big it is and how fast it is going - the
    /// three things `drive_engage` needs, for whichever kind of thing was
    /// tapped.
    fn target_state(&self, target: Entity) -> Option<TargetState> {
        {
            let mut q = self.world.query_one::<&Tank>(target);
            if let Ok(tank) = q.get() {
                if tank.is_wreck() {
                    return None;
                }
                let velocity = tank.body.map(|b| self.physics.velocity(b)).unwrap_or(tank.velocity);
                // The real per-row damage box the projectile sweep checks,
                // oriented the way the tank is currently facing - not the
                // `hull_size` average, which is up to 8px out on either
                // axis and so lets the fire gate approve shots that sail
                // past the hull.
                let (hx, hy) = tank.hull_half_extents(tank.facing_along_x());
                return Some(TargetState { position: tank.position, velocity, half_x: hx, half_y: hy });
            }
        }
        {
            let mut q = self.world.query_one::<&Frog>(target);
            if let Ok(frog) = q.get() {
                return (!frog.is_dead()).then_some(TargetState {
                    position: frog.position,
                    velocity: Position::new(0.0, 0.0),
                    half_x: crate::FROG_COLLIDER_HALF_EXTENT.0,
                    half_y: crate::FROG_COLLIDER_HALF_EXTENT.1,
                });
            }
        }
        {
            let mut q = self.world.query_one::<&Obstacle>(target);
            if let Ok(o) = q.get() {
                let half = o.hull_size() * 0.5;
                return (!o.destroyed).then_some(TargetState {
                    position: o.position,
                    velocity: Position::new(0.0, 0.0),
                    half_x: half,
                    half_y: half,
                });
            }
        }
        None
    }

    /// Pick the heading to drive this frame, with the two safety nets
    /// `Ai::steer` has and the first cut of this did not.
    ///
    /// Direction commitment on its own (hold a heading for
    /// `order_dir_hold_seconds` so a diagonal route does not make the tank
    /// alternate every frame) has one bad failure: a held heading that
    /// walks into a doorway jamb keeps walking into it. So:
    ///
    /// - **Obstacle ahead** drops the commitment early. Continuing into a
    ///   cell the grid already calls blocked is a hard geometric fact, not
    ///   the wobbling-route case the hold exists to filter.
    /// - **Stuck escape** catches what that cannot: commanded to move,
    ///   going nowhere for `stuck_escape_seconds` (wedged on a corner the
    ///   coarse grid thinks is clear). Turn away from whatever heading has
    ///   been failing - a perpendicular, then the other, then back out.
    ///
    /// Both are safety nets and are worth having, but neither was the cause
    /// of the "the tank will not go down a corridor" report they were built
    /// for: adding them moved arrivals on the shipped map from 72% to 74%
    /// and left the 280 px misses exactly where they were. Tracing one
    /// showed the tank driving into a wall and grinding there for twenty
    /// seconds under an *engagement* - `next_step`'s A* treats its goal cell
    /// as open whether or not it is, so a firing spot buried inside a
    /// structure produced a perfectly good route that ended in the
    /// structure. `drive_engage` picking a firing spot it can actually
    /// stand on is what fixed that; see the `firing_spots` call there.
    fn commit_dir(&mut self, f: &mut Frame, grid: &Grid, from: Position, step: Position) -> Dir {
        let t = tuning();
        let fresh = Dir::toward(from, step);
        let blocked = |d: Dir| grid.blocked_ahead(from, d.vec());

        if self.orders.moved < t.stuck_speed_eps * f.dt {
            self.orders.stuck_timer += f.dt;
        } else {
            self.orders.stuck_timer = 0.0;
        }

        if self.orders.stuck_timer >= t.stuck_escape_seconds {
            self.orders.stuck_timer = 0.0;
            let failing = self.orders.committed.unwrap_or(fresh);
            let out = [perpendicular(failing, true), perpendicular(failing, false), opposite(failing)]
                .into_iter()
                .find(|&d| !blocked(d))
                .unwrap_or(fresh);
            self.orders.committed = Some(out);
            self.orders.dir_hold = 0.0;
            return out;
        }

        self.orders.dir_hold += f.dt;
        match self.orders.committed {
            Some(held)
                if held != fresh
                    && self.orders.dir_hold < t.order_dir_hold_seconds
                    && !blocked(held) =>
            {
                held
            }
            _ => {
                if self.orders.committed != Some(fresh) {
                    self.orders.dir_hold = 0.0;
                }
                self.orders.committed = Some(fresh);
                fresh
            }
        }
    }
}

struct MeSnapshot {
    position: Position,
    wreck: bool,
}

struct TargetState {
    position: Position,
    velocity: Position,
    /// Half the target's footprint on each axis, in world px. Per-axis
    /// rather than one radius because tanks are appreciably longer than
    /// they are wide (a `scout` is 28x38), so how much of it a shot has to
    /// hit depends on which way the shot is travelling - see
    /// `half_across`.
    half_x: f32,
    half_y: f32,
}

impl TargetState {
    /// Half the target's width *across* a shot travelling along `dir` -
    /// i.e. how far off the target's centre line a shell may pass and
    /// still strike it. This is the number the fire gate is actually
    /// about; `enemy_fire_align_px` is a heading tolerance, which is a
    /// different quantity that happens to share units.
    fn half_across(&self, dir: Dir) -> f32 {
        match dir {
            Dir::Up | Dir::Down => self.half_x,
            Dir::Left | Dir::Right => self.half_y,
        }
    }

    /// The radius sighting has to stop short of. A *solid* target blocks
    /// the very ray aimed at it: `Terrain::line_of_sight` to a barrel's
    /// centre is always false, since the barrel is in the way of itself.
    fn sight_half(&self) -> f32 {
        self.half_x.max(self.half_y)
    }
}

/// How far off the target's centre line this shot may pass and still hit,
/// in world px: the target's own half-width *across* the shot, less
/// `order_align_margin_px` for the shell's girth, floored so a very narrow
/// target still gets shot at rather than stared at.
///
/// **This is a hit test, not a heading tolerance**, and the difference is
/// the whole point. The gate used to be `enemy_fire_align_px` - 24 px flat,
/// borrowed from the AI, where it is fine because an enemy closes to
/// point-blank and gets many shots. As a *player* fire gate it approves
/// shots that cannot land. Chassis are 28-56 px wide, so half of one is
/// 14-28 px: lined up at the old window's edge, a shot at a `scout` passed
/// 24 px off a hull with 14 px of half-width and sailed by. Measured over a
/// spread of bearings, the twin-barrel rows landed 47% and 46% of their
/// shells; sizing the window to the target takes them to 63% and 56%, and
/// the single-barrel row - which was already inside its own window most of
/// the time - stays where it was at 88%.
///
/// **The shooter's barrel offset is deliberately not charged against this**,
/// though it is real: a twin-barrel hull fires from barrels 6-10 px either
/// side of its centre line, so its outboard shell is that much further out
/// than this window admits. Subtracting it was tried and measured worse.
/// Both shells land only within `half - lateral` of dead centre, which for
/// a `titan` shooting a `scout` is +/-4 px - a window so tight the tank
/// almost never took the shot at all (a quarter as many shells fired, and
/// 38% of those landing, because anything that tight is at the mercy of the
/// target's own movement). Firing a volley where one shell hits and the
/// other misses beats standing there holding a perfect firing line that
/// never arrives.
fn hit_window(state: &TargetState, dir: Dir) -> f32 {
    let t = tuning();
    (state.half_across(dir) - t.order_align_margin_px).max(t.order_align_min_px)
}

/// Whether a shot fired now can still be inside `window` when it arrives.
///
/// A shell is in flight for `range / shell_speed`; a target crossing the
/// firing line keeps moving for all of it. Taking the shot anyway spends
/// one of twenty shells on something geometrically impossible, so this
/// holds fire instead - the tank keeps its aim and shoots the moment the
/// geometry works.
fn can_land(range: f32, off_axis: f32, target_vel: Position, dir: Dir, window: f32) -> bool {
    let speed = tuning().shell_speed.max(1.0);
    let flight = range / speed;
    // Only the component across the firing axis matters; closing or
    // opening along it just changes where on the line the shell lands.
    let across = match dir {
        Dir::Up | Dir::Down => target_vel.x,
        Dir::Left | Dir::Right => target_vel.y,
    };
    off_axis + (across * flight).abs() <= window
}

/// The point to shoot from: a **sidestep** onto one of the target's two
/// axes, holding the current standoff.
///
/// Deliberately not a fixed distance from the target. An earlier version
/// stood at `range * 0.8`, which made a tank already lined up at 190px drive
/// *backwards* to 272px before it would shoot - the shortest path to a
/// firing line is sideways onto it, not out to some canonical radius.
///
/// Of the two axes, prefer the one the target is *travelling along* - in
/// front of or behind a mover rather than across it. A shell fired down the
/// line a target is running on stays lined up for its whole flight; one
/// fired across that line has to beat the drift, and at any real range it
/// cannot (see the module comment).
/// Both spots, preferred first - the caller takes the first one it can
/// actually stand on, because the preferred axis is regularly a wall.
fn firing_spots(from: Position, state: &TargetState) -> [Position; 2] {
    let (vx, vy) = (state.velocity.x.abs(), state.velocity.y.abs());
    let vertical = if vx.max(vy) > 12.0 {
        // Moving: stand on the axis it is moving along.
        vy > vx
    } else {
        // Still: take whichever axis needs the shorter sidestep.
        (from.x - state.position.x).abs() < (from.y - state.position.y).abs()
    };
    // Shooting up or down the target's column means matching its x; along
    // its row means matching its y. Either way the standoff is unchanged -
    // the shortest path onto a firing line is sideways onto it, not out to
    // some canonical radius.
    let column = Position::new(state.position.x, from.y);
    let row = Position::new(from.x, state.position.y);
    if vertical { [column, row] } else { [row, column] }
}

/// The point on the near face of the target, along the line from `from` -
/// what sighting aims at, so a solid target does not block its own ray.
fn near_side(from: Position, state: &TargetState) -> Position {
    let (dx, dy) = (state.position.x - from.x, state.position.y - from.y);
    let d = (dx * dx + dy * dy).sqrt();
    if d <= state.sight_half() + 1.0 {
        return from;
    }
    let back = state.sight_half() + 2.0;
    Position::new(state.position.x - dx / d * back, state.position.y - dy / d * back)
}

fn within(point: Position, center: Position, half: f32) -> bool {
    (point.x - center.x).abs() <= half && (point.y - center.y).abs() <= half
}

/// `Vector2` is not `Serialize`, so the dev server's snapshot takes plain
/// numbers - the same shape `debug::DebugSnapshot` uses.
#[derive(serde::Serialize)]
pub struct OrderReadout {
    pub kind: &'static str,
    pub target: Option<OrderTarget>,
    pub x: f32,
    pub y: f32,
    /// Half the marked thing's on-screen footprint, so the selection
    /// brackets frame a titan and a health pack alike without either
    /// knowing about the other. Zero for a move, which marks a point.
    pub half: f32,
}

impl Game {
    /// The standing order as plain values, for `snapshot` and the overlay.
    pub fn player_order(&self) -> Option<OrderReadout> {
        let order = self.orders.current?;
        Some(match order {
            Order::Move { to } => OrderReadout { kind: "move", target: None, x: to.x, y: to.y, half: 0.0 },
            Order::Engage { target, kind } => {
                let state = self.target_state(target);
                let at = state.as_ref().map(|s| s.position).unwrap_or(Vector2::new(0.0, 0.0));
                // The wider of the two halves: the bracket frames the whole
                // silhouette rather than cutting the long axis off.
                let half = state.as_ref().map(|s| s.half_x.max(s.half_y)).unwrap_or(0.0);
                OrderReadout { kind: "engage", target: Some(kind), x: at.x, y: at.y, half }
            }
            Order::Collect { pickup } => {
                let mut q = self.world.query_one::<&Pickup>(pickup);
                let (at, half) = q
                    .get()
                    .map(|p| (p.position, p.size() * 0.5))
                    .unwrap_or((Vector2::new(0.0, 0.0), 0.0));
                OrderReadout { kind: "collect", target: Some(OrderTarget::Pickup), x: at.x, y: at.y, half }
            }
        })
    }
}

/// The route the standing order is currently walking, as world points -
/// the `orders` debug overlay draws it, and it is also the honest answer to
/// "is the pathfinding doing something sane" without reading a screenshot.
///
/// Walks `Grid::next_step` forward from the player, capped, because that is
/// literally the route the tank will drive; a fresh A* to the goal could
/// disagree with what the tank does next.
impl Game {
    pub fn order_route(&self, grid: &Grid, cap: usize) -> Vec<Position> {
        let Some(order) = self.orders.current else { return Vec::new() };
        let Some(player) = self.player else { return Vec::new() };
        let goal = match order {
            Order::Move { to } => to,
            Order::Engage { target, .. } => match self.target_state(target) {
                Some(s) => s.position,
                None => return Vec::new(),
            },
            Order::Collect { pickup } => {
                let mut q = self.world.query_one::<&Pickup>(pickup);
                match q.get() {
                    Ok(p) => p.position,
                    Err(_) => return Vec::new(),
                }
            }
        };
        let mut at = with_tank(&self.world, player, |t| t.position);
        let mut route = Vec::new();
        for _ in 0..cap {
            match grid.next_step(at, goal) {
                Some(step) => {
                    route.push(step);
                    at = step;
                }
                None => break,
            }
        }
        route
    }
}
