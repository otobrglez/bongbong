//! Bounded AI and pacing on field maps (docs/large-maps-follow-camera.md
//! sections 5, 12 and 13 item 5): what replaces the arena's assumptions
//! once a map is bigger than a screen. A field map is one the camera
//! follows - bigger than an arena, or `view = "follow"`
//! (`MapFile::class`); `Game::field_map` carries the answer from `init`,
//! and on an arena nothing in here runs, so every arena replays exactly as
//! it did.
//!
//! - **Chained alerts** (`Game::field_alerts`): an enemy that sees a seat
//!   alerts the enemies within `enemy_alert_chain_px` of itself, and they
//!   pass it on the same way - each tank carries its own alert
//!   (`ai::FieldMind`) instead of the arena's one shared alert, and the
//!   far side of the map does not hear. Like the shared alert, a pure
//!   distance test with no line of sight (see `grass.rs`'s reasoning).
//! - **Leashes**: each enemy keeps a home - where it first stood on the
//!   field - and with nothing to fight it stays within `enemy_leash_px` of
//!   it (`ai::Brain::home_leash`).
//! - **A wave is called to the fight**: a tank that arrives through a gate
//!   routes at the seat it fights until it first comes within sight range
//!   of a seat or takes a hit (`FieldMind::called`) - the walk the gate was
//!   chosen for.
//! - **Far enemies think less** (`mind`): farther than `enemy_far_px` from
//!   every live seat and the players' frog, a tank thinks every
//!   `enemy_far_think_ticks` ticks, staggered by owner slot, and coasts on
//!   its last intent in between; one nothing has woken yet does not think
//!   at all.
//! - **Spawns and gates by walk, out of sight** (`spawn_cells`,
//!   `prefer_gates`): outside every seat's sight box (`ai::in_sight_box`),
//!   preferring a path from the nearest seat whose walk is about
//!   `field_walk_seconds` where the map offers one; a wrecked seat comes
//!   back through the gate nearest the living seats (`waves.rs`).
//!
//! Everything is a pure function of positions, slots and the round clock:
//! no RNG is drawn here but the one pick among the spawn cells, from the
//! round's own stream, and ties break on owner slot or cell order.

use std::collections::{BTreeMap, HashMap};

use hecs::Entity;
use rand::RngExt;
use rand::rngs::SmallRng;

use crate::ai::{Ai, Intent, in_sight_box_of};
use crate::battlefield::{self, Gate};
use crate::pathfind::{Grid, WalkCosts};
use crate::tank::Tank;
use crate::tuning::{Tuning, tuning};
use crate::{OBSTACLE_GRID_SIZE, PATHFIND_CELL_SIZE, Position};

use super::{Event, Frame, Game, with_tank};

/// How a tank on a field map spends this tick (`mind`).
#[derive(Clone, Copy, Debug)]
pub(super) enum Mind {
    /// It thinks, its timers covering this many seconds - the tick plus
    /// every tick it skipped since its last think.
    Think(f32),
    /// Far, so it keeps driving this intent - its last, the trigger
    /// released - without thinking.
    Coast(Intent),
    /// Far and never woken: it holds still and thinks nothing.
    Sleep,
}

/// How the tank at `position` in owner slot `slot` spends tick `frame`:
/// a near tank - within `enemy_far_px` of one of `anchors`, the live seats
/// and the players' frog - thinks every tick; a far one thinks when
/// `frame + slot` falls on `enemy_far_think_ticks` and coasts otherwise;
/// a far one nothing has woken sleeps. `hunting` is a hunter with a frog
/// to hunt, which an objective keeps awake wherever it is.
///
/// Also where the tank's home is set, the first tick it stands on the
/// field, and where it wakes: an alert, a hit, a call to the fight, a
/// frog to hunt or a seat within `enemy_far_px` - and once awake it stays
/// so. And where a wave tank counts how long it has been lost to the
/// fight (`FieldMind::lost`): no anchor within `sight`, what it sees.
#[allow(clippy::too_many_arguments)] // one tank's perception, passed by value
pub(super) fn mind(
    ai: &mut Ai,
    position: Position,
    slot: usize,
    anchors: &[Position],
    sight: f32,
    hunting: bool,
    frame: u64,
    dt: f32,
) -> Mind {
    let t = tuning();
    let near = anchors.iter().any(|a| a.distance_to(position) <= t.enemy_far_px);
    let hit = ai.is_hit_alerted();
    let mind = &mut ai.field;
    if mind.home.is_none() {
        mind.home = Some(position);
    }
    if mind.wave {
        let in_sight = anchors.iter().any(|a| a.distance_to(position) <= sight);
        mind.lost = if in_sight { 0.0 } else { mind.lost + dt };
    }
    if near || hunting || hit || mind.called || mind.alert.is_some() {
        mind.awake = true;
    }
    let think = near || (mind.awake && (frame + slot as u64).is_multiple_of(t.enemy_far_think_ticks.max(1) as u64));
    if think {
        let covered = dt + mind.think_debt;
        mind.think_debt = 0.0;
        return Mind::Think(covered);
    }
    if !mind.awake {
        return Mind::Sleep;
    }
    mind.think_debt += dt;
    Mind::Coast(Intent { fire: false, fire_aim_offset: 0.0, ..ai.last_intent() })
}

/// The point a called wave tank at `position` routes at: the nearest of
/// the live seats on the field, ties to the lower seat. `None` with
/// nobody to go to.
pub(super) fn call_target(live_seats: &[Position], position: Position) -> Option<Position> {
    live_seats.iter().copied().fold(None, |best: Option<Position>, s| match best {
        Some(b) if b.distance_to(position) <= s.distance_to(position) => Some(b),
        _ => Some(s),
    })
}

/// A path's walk in seconds at the baseline `enemy_speed`, from its cost
/// in nav steps.
fn walk_seconds(cost: u32, t: &Tuning) -> f32 {
    cost as f32 * PATHFIND_CELL_SIZE / t.enemy_speed.max(1.0)
}

/// Whether a walk of `walk` seconds is about `field_walk_seconds`: within
/// `field_walk_slack_seconds` of it either way.
fn about_the_walk(walk: f32, t: &Tuning) -> bool {
    (walk - t.field_walk_seconds).abs() <= t.field_walk_slack_seconds
}

/// A cell a field map's band may spawn an enemy on (`spawn_cells`), and
/// whether its walk to the nearest seat is about `field_walk_seconds`.
#[derive(Clone, Copy, Debug)]
pub(super) struct SpawnCell {
    pub(super) at: Position,
    pub(super) about_the_walk: bool,
}

/// The cells a field map's band may spawn an enemy on
/// (docs/large-maps-follow-camera.md sections 5 and 12): the centre of
/// every nav cell a hull can stand in (`Grid::usable`) that keeps a tile
/// and a hull clear of every one of `walls` (the band's own separation,
/// `battlefield::enemy_spawn_legal`), stands outside every seat's sight
/// box and has a route to a seat, each marked with whether its walk to the
/// nearest of `seats` is about `field_walk_seconds` - which `pick_spawn`
/// prefers. In cell order, so a draw from it is a pure function of the
/// round's seed. Empty when no cell qualifies, and the caller falls back
/// to the band.
pub(super) fn spawn_cells(grid: &Grid, seats: &[Position], walls: &[Position]) -> Vec<SpawnCell> {
    if seats.is_empty() {
        return Vec::new();
    }
    let (cols, rows, cell) = grid.dims();
    let walk = grid.walk_costs(seats);
    // The walls bucketed by tile, so the separation test reads the few
    // around a cell rather than all of them: a wall farther than one
    // tile and a hull can never fail it.
    let separation = OBSTACLE_GRID_SIZE * 0.5 + battlefield::max_tank_clearance_half_extent();
    let bucket = |p: Position| ((p.x / OBSTACLE_GRID_SIZE).floor() as i32, (p.y / OBSTACLE_GRID_SIZE).floor() as i32);
    let mut buckets: HashMap<(i32, i32), Vec<Position>> = HashMap::new();
    for &w in walls {
        buckets.entry(bucket(w)).or_default().push(w);
    }
    let reach = (separation / OBSTACLE_GRID_SIZE).ceil() as i32;
    let clear_of_walls = |p: Position| {
        let (bx, by) = bucket(p);
        (bx - reach..=bx + reach).all(|x| {
            (by - reach..=by + reach).all(|y| {
                buckets.get(&(x, y)).is_none_or(|ws| {
                    ws.iter().all(|w| (p.x - w.x).abs() >= separation || (p.y - w.y).abs() >= separation)
                })
            })
        })
    };
    let t = tuning();
    let half = t.sight_box_half_px();
    let mut cells = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let p = Position::new((col as f32 + 0.5) * cell, (row as f32 + 0.5) * cell);
            if !grid.usable(p) || seats.iter().any(|&s| in_sight_box_of(half, s, p)) {
                continue;
            }
            let Some(cost) = walk.at(p) else { continue };
            if clear_of_walls(p) {
                cells.push(SpawnCell { at: p, about_the_walk: about_the_walk(walk_seconds(cost, &t), &t) });
            }
        }
    }
    cells
}

/// One of `cells` that passes `ok`, drawn from the round's RNG: of
/// `field_spawn_spread_candidates` uniform draws, a cell whose walk is
/// about `field_walk_seconds` before one that is not, then the one
/// standing farthest from every tank in `placed` (the first such on a
/// tie) - so a band leans toward the walk the map is paced for without
/// piling into the sliver of it a small map has, and spreads over its
/// region instead of starting in a heap. `None`, drawing nothing, when no
/// cell passes.
pub(super) fn pick_spawn(cells: &[SpawnCell], rng: &mut SmallRng, placed: &[Position], ok: impl Fn(Position) -> bool) -> Option<Position> {
    SpawnPool::new(cells, ok).pick(rng, placed)
}

/// The cells a band's enemies are drawn from one after another
/// (`Game::init`), kept as `pick_spawn` would filter them for each: the
/// cells a fixed test allows, less, as each enemy goes down, the ones too
/// near it (`place`). The same cells in the same order as filtering them
/// all again for every enemy, so every draw lands where it would - at a
/// cost that grows with the cells, not with the cells times the band.
pub(super) struct SpawnPool {
    free: Vec<SpawnCell>,
}

impl SpawnPool {
    /// The cells of `cells` that `ok` allows, in order.
    pub(super) fn new(cells: &[SpawnCell], ok: impl Fn(Position) -> bool) -> SpawnPool {
        SpawnPool { free: cells.iter().copied().filter(|c| ok(c.at)).collect() }
    }

    /// `pick_spawn`'s draw over the pool, `placed` the tanks already down.
    pub(super) fn pick(&self, rng: &mut SmallRng, placed: &[Position]) -> Option<Position> {
        let free = &self.free;
        if free.is_empty() {
            return None;
        }
        let mut best: Option<(SpawnCell, f32)> = None;
        for _ in 0..tuning().field_spawn_spread_candidates.max(1) {
            let cell = free[rng.random_range(0..free.len())];
            let spread = placed.iter().map(|&q| cell.at.distance_to(q)).fold(f32::INFINITY, f32::min);
            let better = |(b, s): (SpawnCell, f32)| (cell.about_the_walk, spread) > (b.about_the_walk, s);
            if best.is_none_or(better) {
                best = Some((cell, spread));
            }
        }
        best.map(|(cell, _)| cell.at)
    }

    /// A tank went down at `at`: the cells nearer it than `apart` leave
    /// the pool.
    pub(super) fn place(&mut self, at: Position, apart: f32) {
        self.free.retain(|c| c.at.distance_to(at) >= apart);
    }
}

/// A field map's preference among the lanes a wave may roll in through:
/// the gates whose inside point stands outside every one of `seats`'
/// sight boxes, then of those the ones whose walk to the nearest seat is
/// about `field_walk_seconds` (`about_the_walk`). Either step that would
/// leave no gate leaves the list as it was: a map whose every gate sits
/// by a seat still uses them, and one with no gate that far out (a map
/// about 40 cells across) only keeps its waves out of sight. Order is
/// kept.
pub(super) fn prefer_gates(gates: Vec<Gate>, grid: &Grid, seats: &[Position]) -> Vec<Gate> {
    if seats.is_empty() {
        return gates;
    }
    let t = tuning();
    let half = t.sight_box_half_px();
    let outside: Vec<Gate> = gates.iter().copied().filter(|g| seats.iter().all(|&s| !in_sight_box_of(half, s, g.inside))).collect();
    let gates = if outside.is_empty() { gates } else { outside };
    let walk = grid.walk_costs(seats);
    let paced: Vec<Gate> =
        gates.iter().copied().filter(|g| walk.at(g.inside).is_some_and(|cost| about_the_walk(walk_seconds(cost, &t), &t))).collect();
    if paced.is_empty() { gates } else { paced }
}

/// The gates in the order a wrecked seat coming back prefers them: by
/// path to the nearest of the living `seats` (`walk`), unreachable last,
/// ties in the given order.
pub(super) fn nearest_gates(mut gates: Vec<Gate>, walk: &WalkCosts) -> Vec<Gate> {
    gates.sort_by_key(|g| walk.at(g.inside).unwrap_or(u32::MAX));
    gates
}

/// Whether a walk of `cost` nav steps (`None`: no walk at all) is far: a
/// straggler's (`Game::reroll_stragglers`) - longer than the nearest a
/// wave's gate is paced for, `field_walk_seconds` less
/// `field_walk_slack_seconds`.
pub(super) fn far_by_walk(cost: Option<u32>) -> bool {
    let t = tuning();
    cost.is_none_or(|cost| walk_seconds(cost, &t) > t.field_walk_seconds - t.field_walk_slack_seconds)
}

/// Whether no screen following the seat at `seat` can show `at`, for a
/// tank that may only leave the field unseen (`Game::reroll_stragglers`).
/// A screen shows the seat's whole sight box (`Tuning::sight_box_half_px`)
/// in a view of at most `view_local_max_cells` cells - more than a room's
/// `view_area_cells` - whose shape lies between `view_aspect_min` and
/// `view_aspect_max` (`framing`). So `at` is on some screen exactly when a
/// view that size and shape can hold both it and the box: with `x` and
/// `y` its offsets from the seat plus the box's half extents, when
/// `x * y` is within the area and neither side outgrows the widest and
/// tallest outline. A small low-density desktop window that snaps
/// outward to a whole-block zoom can show a little more than the area;
/// the straggler's own terms - long out of its sight, a long walk away -
/// keep the tanks this takes off well past that in practice.
pub(super) fn beyond_every_screen(seat: Position, at: Position, t: &Tuning) -> bool {
    let cell = OBSTACLE_GRID_SIZE;
    let area = t.view_local_max_cells.max(t.view_area_cells) * cell * cell;
    let (box_w, box_h) = t.sight_box_half_px();
    let x = (at.x - seat.x).abs() + box_w;
    let y = (at.y - seat.y).abs() + box_h;
    x * y > area || x > (area * t.view_aspect_max).sqrt() || y > (area / t.view_aspect_min.max(f32::EPSILON)).sqrt()
}

/// The lanes a straggler may come back through, from the ones a wave may
/// take (`gates`, in order): those whose inside point stands outside
/// every one of `seats`' sight boxes and whose walk to the fight (`walk`)
/// is shorter than the straggler's own (`from`), then of those the ones
/// about `field_walk_seconds` where there are any. Order is kept; empty
/// when no lane is nearer.
pub(super) fn reroll_gates(gates: Vec<Gate>, walk: &WalkCosts, seats: &[Position], from: Option<u32>) -> Vec<Gate> {
    let t = tuning();
    let half = t.sight_box_half_px();
    let nearer: Vec<Gate> = gates
        .into_iter()
        .filter(|g| seats.iter().all(|&s| !in_sight_box_of(half, s, g.inside)))
        .filter(|g| walk.at(g.inside).is_some_and(|cost| from.is_none_or(|from| cost < from)))
        .collect();
    let paced: Vec<Gate> =
        nearer.iter().copied().filter(|g| walk.at(g.inside).is_some_and(|cost| about_the_walk(walk_seconds(cost, &t), &t))).collect();
    if paced.is_empty() { nearer } else { paced }
}

impl Game {
    /// Whether this round is fought on a field map - see the module doc.
    pub fn field_map(&self) -> bool {
        self.field_map
    }

    /// Where every live seat on the field stands, in seat order: what a
    /// field map's spawns, gates and re-entries are measured from.
    pub(super) fn live_seat_positions(&self) -> Vec<Position> {
        self.seats_on_field()
            .into_iter()
            .flatten()
            .filter_map(|e| with_tank(&self.world, e, |t| (!t.is_wreck()).then_some(t.position)))
            .collect()
    }

    /// The field map's alert pass, run by `enemy_phase` in place of the
    /// arena's shared alert: every enemy's own alert ages by the frame;
    /// an enemy that sees a seat - one of `seen`, the live unconcealed
    /// seats on the field, each within the range it is seen at
    /// (`Game::sight_on`) - takes a fresh one on the
    /// nearest; then each group of enemies chained within
    /// `enemy_alert_chain_px` of one another shares the best alert any of
    /// them holds (the freshest, then the nearest sighting, then the
    /// lower slot), which is how a sighting travels down the chain and no
    /// further. A wreck, and a tank an EMP disabled, neither sees nor passes
    /// anything on. A called
    /// wave tank's call ends here too, the first frame it is within
    /// `view_range` of one of the `live` seats or has been hit.
    pub(super) fn field_alerts(&mut self, seen: &[(Position, f32)], live: &[Position], view_range: f32, f: &mut Frame) {
        let (hold, chain) = {
            let t = tuning();
            (t.enemy_alert_hold_seconds, t.enemy_alert_chain_px)
        };
        let mut tanks: Vec<(Entity, usize, Position, bool)> = self
            .world
            .query::<(Entity, &Tank)>()
            .with::<&Ai>()
            .iter()
            // A disabled tank (an EMP) is as deaf and blind as a wreck: it
            // neither sees, relays nor receives, its own alert ageing on.
            .map(|(e, t)| (e, t.owner_slot(), t.position, t.is_wreck() || t.is_disabled()))
            .collect();
        tanks.sort_by_key(|&(_, slot, _, _)| slot);
        let alerted = |world: &hecs::World| -> Option<Position> {
            tanks.iter().find_map(|&(e, ..)| world.get::<&Ai>(e).ok().and_then(|ai| ai.field.alert))
        };
        let before = self.trace_ai.then(|| alerted(&self.world)).flatten();

        // Age, then refresh every spotter. `sighting` is how far a spotter
        // stood from the seat it saw, for the group's choice below.
        let mut sighting: Vec<Option<f32>> = vec![None; tanks.len()];
        for (i, &(entity, _, position, wreck)) in tanks.iter().enumerate() {
            let mut ai = self.world.get::<&mut Ai>(entity).expect("queried with an Ai");
            let hit = ai.is_hit_alerted();
            let mind = &mut ai.field;
            mind.alert_timer = (mind.alert_timer - f.dt).max(0.0);
            if mind.alert_timer <= 0.0 {
                mind.alert = None;
            }
            if !wreck {
                // The nearest seat it sees, ties to the lower seat.
                let nearest = seen.iter().map(|&(s, sight)| (position.distance_to(s), s, sight)).filter(|&(d, _, sight)| d <= sight).map(|(d, s, _)| (d, s)).fold(
                    None,
                    |best: Option<(f32, Position)>, (d, s)| if best.is_some_and(|(bd, _)| bd <= d) { best } else { Some((d, s)) },
                );
                if let Some((d, s)) = nearest {
                    mind.alert = Some(s);
                    mind.alert_timer = hold;
                    sighting[i] = Some(d);
                }
            }
            if mind.called && (hit || live.iter().any(|&s| s.distance_to(position) <= view_range)) {
                mind.called = false;
            }
        }

        // The chain: live enemies within `chain` px of one another, joined
        // transitively (a union-find over every pair, slot order).
        let mut group: Vec<usize> = (0..tanks.len()).collect();
        fn root(group: &mut [usize], mut i: usize) -> usize {
            while group[i] != i {
                group[i] = group[group[i]];
                i = group[i];
            }
            i
        }
        for i in 0..tanks.len() {
            for j in i + 1..tanks.len() {
                let (a, b) = (&tanks[i], &tanks[j]);
                if a.3 || b.3 || a.2.distance_to(b.2) > chain {
                    continue;
                }
                let (ra, rb) = (root(&mut group, i), root(&mut group, j));
                if ra != rb {
                    group[ra.max(rb)] = ra.min(rb);
                }
            }
        }
        // Each group's best alert, then handed to every live member.
        let mut best: BTreeMap<usize, (f32, f32, usize, Position)> = BTreeMap::new();
        for (i, &(entity, slot, _, wreck)) in tanks.iter().enumerate() {
            if wreck {
                continue;
            }
            let ai = self.world.get::<&Ai>(entity).expect("queried with an Ai");
            let Some(at) = ai.field.alert else { continue };
            let key = (ai.field.alert_timer, sighting[i].unwrap_or(f32::INFINITY), slot, at);
            let r = root(&mut group, i);
            let better = |old: &(f32, f32, usize, Position)| {
                key.0 > old.0 || (key.0 == old.0 && (key.1 < old.1 || (key.1 == old.1 && key.2 < old.2)))
            };
            if best.get(&r).is_none_or(better) {
                best.insert(r, key);
            }
        }
        for (i, &(entity, _, _, wreck)) in tanks.iter().enumerate() {
            if wreck {
                continue;
            }
            let Some(&(timer, _, _, at)) = best.get(&root(&mut group, i)) else { continue };
            let mut ai = self.world.get::<&mut Ai>(entity).expect("queried with an Ai");
            ai.field.alert = Some(at);
            ai.field.alert_timer = timer;
        }

        if self.trace_ai {
            let after = alerted(&self.world);
            if before.is_some() != after.is_some() {
                let p = after.or(before).unwrap_or_default();
                f.events.push(Event::Alert { on: after.is_some(), x: p.x, y: p.y });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No screen shows past the widest outline a view of the local round's
    /// area takes, nor past its tallest, nor into a corner no outline of
    /// that area reaches; everything nearer is on some screen.
    #[test]
    fn a_point_is_beyond_every_screen_only_past_every_outline_the_view_can_take() {
        let t = Tuning::DEFAULT;
        let seat = Position::new(4000.0, 4000.0);
        let at = |dx: f32, dy: f32| Position::new(seat.x + dx, seat.y + dy);
        let cell = OBSTACLE_GRID_SIZE;
        let area = t.view_local_max_cells * cell * cell;
        let (box_w, box_h) = t.sight_box_half_px();
        let widest = (area * t.view_aspect_max).sqrt() - box_w;
        let tallest = (area / t.view_aspect_min).sqrt() - box_h;
        // The sight box itself, and a monitor's 40 x 22.5 cells round it.
        assert!(!beyond_every_screen(seat, at(box_w, box_h), &t));
        assert!(!beyond_every_screen(seat, at(-20.0 * cell, 0.0), &t));
        // Straight out to either side, up and down: the widest and the
        // tallest outline are the edge.
        assert!(!beyond_every_screen(seat, at(widest - 1.0, 0.0), &t));
        assert!(beyond_every_screen(seat, at(widest + 1.0, 0.0), &t));
        assert!(!beyond_every_screen(seat, at(0.0, -(tallest - 1.0)), &t));
        assert!(beyond_every_screen(seat, at(0.0, -(tallest + 1.0)), &t));
        // A corner inside both, but past what any one outline of the area
        // holds at once.
        let corner = at(widest * 0.9, tallest * 0.9);
        assert!(beyond_every_screen(seat, corner, &t), "{corner:?}");
    }

    #[test]
    fn a_walk_is_far_past_the_nearest_a_wave_is_paced_for() {
        let t = Tuning::DEFAULT;
        let steps = |seconds: f32| (seconds * t.enemy_speed / PATHFIND_CELL_SIZE).round() as u32;
        let near = t.field_walk_seconds - t.field_walk_slack_seconds;
        assert!(!far_by_walk(Some(steps(near - 1.0))));
        assert!(far_by_walk(Some(steps(near + 1.0))));
        assert!(far_by_walk(None), "no walk at all is the farthest");
    }
}
