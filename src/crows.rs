//! Crows (docs/field-life.md "Crows"): a flock of five to eight on each of
//! a handful of open stretches of grass, and the odd few up in a tree's
//! crown. On the grass they peck, hop and turn about; a hull coming within
//! `crow_flush_px`, a shot or a hit within `crow_shot_px` or a blast within
//! `crow_blast_px` of any of them flushes the whole flock. Each bird lifts
//! off a moment after the last (`crow_stagger_seconds`), flies away from
//! the cause climbing to `crow_altitude_px`, swings round over the stretch
//! of grass the flock settles on next - `crow_land_min_px` to
//! `crow_land_max_px` from the cause - and comes down on it. A crow in a
//! crown flushes when its tree is struck, hit or felled, or a shot or a
//! blast comes near; it settles on the grass.
//!
//! Presentation only, like everything in `fauna.rs`: every choice hashed
//! from the round's seed, the cell and the crow, nothing rolled.

use crate::fauna::{self, Grid, Ground, Stir};
use crate::math::{Color, Rectangle, Vec2};
use crate::pyro::{self, Shape};
use crate::tuning::Tuning;
use crate::{OBSTACLE_GRID_SIZE, Position};

/// The crow's colours, Puny Palette steps (tools/punypalette.py): the body
/// STONE_DARKEST, the wings and the head BLACK, the beak STONE_DK.
pub const BODY: Color = Color::new(0x37, 0x37, 0x37, 255);
pub const WING: Color = Color::new(0x25, 0x25, 0x25, 255);
pub const HEAD: Color = Color::new(0x25, 0x25, 0x25, 255);
pub const BEAK: Color = Color::new(0x7E, 0x7E, 0x7E, 255);

/// Sitting, facing up: the beak, the head, the body between its folded
/// wings and the tail.
const SITTING: [&str; 5] = [".k.", ".H.", "BWB", "BWB", ".W."];

/// Pecking: the head down a block, the tail hidden.
const PECKING: [&str; 5] = ["...", ".k.", ".H.", "BWB", "BWB"];

/// Flying, wings spread, and swept back: the two beats of a flap.
const WINGS_OUT: [&str; 5] = ["..k..", "..H..", "WWBWW", "W.B.W", ".W.W."];
const WINGS_IN: [&str; 5] = ["..k..", "..H..", ".WBW.", ".WBW.", ".W.W."];

fn color(ch: u8) -> Option<Color> {
    match ch {
        b'B' => Some(BODY),
        b'W' => Some(WING),
        b'H' => Some(HEAD),
        b'k' => Some(BEAK),
        _ => None,
    }
}

/// A crow's place in its flock: the flock stands in two loose rows of
/// four, `SLOT_PX` apart across and `ROW_PX` down, the second row half a
/// place over, each bird a hashed `JITTER_PX` off its place; it wanders up
/// to `WANDER_PX` about it as it hops.
const SLOT_PX: f32 = 17.0;
const ROW_PX: f32 = 13.0;
const JITTER_PX: f32 = 3.0;
const WANDER_PX: (f32, f32) = (7.0, 5.0);

/// How far from its flock's spot a crow on the grass ever sits, px.
pub const SPREAD_PX: f32 = 2.0 * SLOT_PX + JITTER_PX + WANDER_PX.0;

/// How far from its crown's centre a perched crow sits, px across and
/// down.
const PERCH_PX: (f32, f32) = (8.0, 6.0);

/// A hop: how long it takes, s, and how far it goes at most, px.
const HOP_SECONDS: f32 = 0.2;
const HOP_PX: f32 = 6.0;

/// The longest a crow flies before it is set down where it was going, s.
const MAX_FLIGHT: f32 = 30.0;

/// What a crow is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// On the grass: pecking, turning, hopping now and then.
    Ground,
    /// Up in the crown of `Ground::trees[i]`.
    Perched(usize),
    /// Startled: it lifts off in `left` seconds.
    Startled { left: f32 },
    /// In the air for `age` seconds.
    Flying { age: f32 },
}

/// One crow.
#[derive(Clone, Debug, PartialEq)]
pub struct Crow {
    /// On the ground, where it stands; in the air, the point on the ground
    /// under it; in a crown, its place there.
    pub pos: Position,
    /// How high it flies, px.
    pub alt: f32,
    /// The way it faces, or flies.
    pub heading: Vec2,
    pub state: State,
    seed: u32,
    flock: usize,
    /// Its place about its flock's spot (`slot`).
    slot: Vec2,
    /// A hop under way: from, to and how far along (0..1).
    hop: Option<(Position, Position, f32)>,
    /// Seconds to its next hop or turn.
    rest: f32,
    /// What it flies from.
    from: Position,
    /// Hops, turns and flights so far: salts each hashed choice.
    leg: u32,
}

/// A flock: where it sits now and how often it has been flushed.
#[derive(Clone, Debug, PartialEq)]
struct Flock {
    spot: Position,
    /// Its tree while it roosts in one.
    tree: Option<usize>,
    seed: u32,
    flights: u32,
}

/// Every crow of a round.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Crows {
    flocks: Vec<Flock>,
    crows: Vec<Crow>,
    /// Open grass whose eight neighbours are open grass too, sorted: where
    /// a flock sits.
    stretches: Vec<(i32, i32)>,
    /// The trees, sorted by cell (`Ground::trees`).
    trees: Vec<((i32, i32), Position)>,
}

impl Crow {
    fn roll(&self, k: u32) -> f32 {
        pyro::unit(self.seed ^ self.leg.wrapping_mul(0x9E37_79B9), k)
    }
}

/// The place of bird `i` (of a flock) about its flock's spot, jittered by
/// its `seed`.
fn slot(i: u32, seed: u32) -> Vec2 {
    let (col, row) = ((i % 4) as f32, ((i / 4) % 2) as f32);
    let x = (col - 1.5) * SLOT_PX + (row - 0.5) * SLOT_PX * 0.5 + (pyro::unit(seed, 21) * 2.0 - 1.0) * JITTER_PX;
    let y = (row - 0.5) * ROW_PX + (pyro::unit(seed, 22) * 2.0 - 1.0) * JITTER_PX;
    Vec2::new(x, y)
}

/// A hashed point up to `WANDER_PX` about a crow's place.
fn wander(seed: u32, k: u32) -> Vec2 {
    Vec2::new((pyro::unit(seed, k) * 2.0 - 1.0) * WANDER_PX.0, (pyro::unit(seed, k + 1) * 2.0 - 1.0) * WANDER_PX.1)
}

impl Crows {
    /// The crows of a round on `ground`, hashed from its `seed`.
    pub fn new(ground: &Ground, seed: u64, t: &Tuning) -> Crows {
        let salt = (seed ^ (seed >> 32)) as u32;
        let open = &ground.open;
        let is_open = |c: (i32, i32)| open.binary_search(&c).is_ok();
        let stretches: Vec<(i32, i32)> =
            open.iter().copied().filter(|&(c, r)| (-1..=1).all(|dr| (-1..=1).all(|dc| is_open((c + dc, r + dr))))).collect();
        let mut crows = Crows { stretches, trees: ground.trees.clone(), ..Crows::default() };
        let most = t.crow_per_flock.max(0);
        if !t.crow_enabled || most == 0 {
            return crows;
        }
        let fewest = (most + 2) / 2;
        let wanted = ((open.len() as f32 / t.crow_flock_cells.max(1.0)).round() as usize).max(usize::from(!crows.stretches.is_empty()));
        let cap = t.crow_max_flocks.max(0) as usize;
        // Homes: the stretches in a hashed order, each kept if it is far
        // enough from those kept before it.
        let mut order: Vec<(f32, (i32, i32))> = crows.stretches.iter().map(|&c| (pyro::unit(crate::blast::seed_at(fauna::centre(c), salt), 1), c)).collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let apart = t.crow_spacing_cells * OBSTACLE_GRID_SIZE;
        let mut homes: Vec<Position> = Vec::new();
        for &(_, cell) in &order {
            if homes.len() >= wanted.min(cap) {
                break;
            }
            let at = fauna::centre(cell);
            if homes.iter().all(|h| h.distance_to(at) >= apart) {
                homes.push(at);
            }
        }
        for home in homes {
            let fseed = crate::blast::seed_at(home, salt ^ 0xC20E_5EED);
            let count = (fewest + (pyro::unit(fseed, 1) * (most - fewest + 1) as f32) as i32).min(most);
            crows.add_flock(Flock { spot: home, tree: None, seed: fseed, flights: 0 }, count as u32);
        }
        // Roosts: now and then a tree with a crow or three in its crown.
        let roosts = crows.trees.clone();
        let mut kept = 0;
        for (i, &(cell, crown)) in roosts.iter().enumerate() {
            if kept >= cap {
                break;
            }
            let rseed = crate::blast::seed_at(fauna::centre(cell), salt ^ 0x0700_57ED);
            if pyro::unit(rseed, 1) >= t.crow_roost_share {
                continue;
            }
            kept += 1;
            let count = 1 + (pyro::unit(rseed, 2) * 3.0) as u32;
            crows.add_flock(Flock { spot: crown, tree: Some(i), seed: rseed, flights: 0 }, count.min(3));
        }
        crows
    }

    fn add_flock(&mut self, flock: Flock, count: u32) {
        let fi = self.flocks.len();
        for i in 0..count {
            let seed = crate::blast::seed_at(flock.spot, flock.seed ^ i.wrapping_mul(0x2545_F491));
            let (pos, state) = match flock.tree {
                Some(tree) => {
                    let off = Vec2::new((pyro::unit(seed, 1) * 2.0 - 1.0) * PERCH_PX.0, (pyro::unit(seed, 2) * 2.0 - 1.0) * PERCH_PX.1);
                    (flock.spot + off, State::Perched(tree))
                }
                None => (flock.spot + slot(i, seed), State::Ground),
            };
            self.crows.push(Crow {
                pos,
                alt: 0.0,
                heading: match flock.tree {
                    // Up in a crown a crow faces one of the four ways, the
                    // shape that reads against the leaves.
                    Some(_) => fauna::heading_of(4 * (pyro::unit(seed, 4) * 4.0) as usize),
                    None => fauna::heading_of((pyro::unit(seed, 4) * 16.0) as usize),
                },
                state,
                seed,
                flock: fi,
                slot: slot(i, seed),
                hop: None,
                rest: 0.5 + 3.0 * pyro::unit(seed, 5),
                from: pos,
                leg: 0,
            });
        }
        self.flocks.push(flock);
    }

    pub fn crows(&self) -> &[Crow] {
        &self.crows
    }

    /// The cell of tree `i` (`State::Perched`).
    pub fn tree_cell(&self, i: usize) -> (i32, i32) {
        self.trees[i].0
    }

    /// Flush every flock one of whose birds this step's `stir` came near:
    /// a hull, a shot or a blast for one on the grass; its tree struck, a
    /// shot or a blast for one in a crown. The first cause found, in the
    /// crows' order, is the one the flock flies from.
    pub fn stir(&mut self, stir: &Stir, t: &Tuning) {
        let mut causes: Vec<Option<Position>> = vec![None; self.flocks.len()];
        for c in &self.crows {
            if causes[c.flock].is_some() {
                continue;
            }
            let near = |points: &[Position], at: Position, reach: f32| Stir::near(points, at, reach);
            causes[c.flock] = match c.state {
                State::Ground => near(&stir.hulls, c.pos, t.crow_flush_px)
                    .or_else(|| near(&stir.shots, c.pos, t.crow_shot_px))
                    .or_else(|| near(&stir.blasts, c.pos, t.crow_blast_px)),
                State::Perched(tree) => near(&stir.trees, fauna::centre(self.trees[tree].0), t.crow_tree_px)
                    .or_else(|| near(&stir.shots, c.pos, t.crow_shot_px))
                    .or_else(|| near(&stir.blasts, c.pos, t.crow_blast_px)),
                _ => None,
            };
        }
        for (fi, cause) in causes.into_iter().enumerate() {
            if let Some(at) = cause {
                self.flush(fi, at, t);
            }
        }
    }

    /// Flush flock `fi` away from `cause`: it picks the stretch of grass it
    /// settles on next, and each of its birds still down lifts off a
    /// hashed moment from now.
    pub fn flush(&mut self, fi: usize, cause: Position, t: &Tuning) {
        let flock = &mut self.flocks[fi];
        flock.flights = flock.flights.wrapping_add(1);
        let roll = |k: u32| pyro::unit(flock.seed ^ flock.flights.wrapping_mul(0x27D4_EB2F), k);
        if !self.stretches.is_empty() {
            let ring: Vec<(i32, i32)> = self
                .stretches
                .iter()
                .copied()
                .filter(|&c| {
                    let d = fauna::centre(c).distance_to(cause);
                    d >= t.crow_land_min_px && d <= t.crow_land_max_px.max(t.crow_land_min_px)
                })
                .collect();
            let cell = if ring.is_empty() {
                // Nowhere in reach: the farthest stretch from the cause.
                let mut best = self.stretches[0];
                for &c in &self.stretches {
                    if fauna::centre(c).distance_to(cause) > fauna::centre(best).distance_to(cause) {
                        best = c;
                    }
                }
                best
            } else {
                ring[((roll(1) * ring.len() as f32) as usize).min(ring.len() - 1)]
            };
            flock.spot = fauna::centre(cell);
            flock.tree = None;
        }
        for c in self.crows.iter_mut().filter(|c| c.flock == fi) {
            if matches!(c.state, State::Ground | State::Perched(_)) {
                c.leg = c.leg.wrapping_add(1);
                c.hop = None;
                c.from = cause;
                c.state = State::Startled { left: c.roll(11) * t.crow_stagger_seconds.max(0.0) };
                if let Some(away) = fauna::unit_of(c.pos - cause) {
                    c.heading = away;
                }
            }
        }
    }

    /// Move every crow on `dt` seconds.
    pub fn step(&mut self, dt: f32, t: &Tuning) {
        for i in 0..self.crows.len() {
            let fi = self.crows[i].flock;
            let (spot, tree, fseed, flights) = {
                let f = &self.flocks[fi];
                (f.spot, f.tree, f.seed, f.flights)
            };
            let c = &mut self.crows[i];
            match c.state {
                State::Ground => sit(c, spot, dt, t),
                State::Perched(_) => {
                    c.rest -= dt;
                    if c.rest <= 0.0 {
                        c.leg = c.leg.wrapping_add(1);
                        c.heading = fauna::heading_of(4 * (c.roll(4) * 4.0) as usize);
                        c.rest = t.crow_hop_seconds.max(0.2) * (1.0 + 2.0 * c.roll(5));
                    }
                }
                State::Startled { left } => {
                    let left = left - dt;
                    c.state = if left <= 0.0 { State::Flying { age: 0.0 } } else { State::Startled { left } };
                    if left <= 0.0 {
                        // Off at a hashed slant from straight away.
                        let away = fauna::unit_of(c.pos - c.from).unwrap_or(c.heading);
                        c.heading = fauna::turned(away, (c.roll(12) - 0.5) * 1.0);
                    }
                }
                State::Flying { age } => fly(c, age + dt, spot, tree, fseed, flights, dt, t),
            }
        }
    }

    /// The crows on the grass, to draw on the floor: pecking now and then
    /// (hashed on the clock `time`), lifted a little mid-hop.
    pub fn ground_shapes(&self, time: f32, cull: Option<Rectangle>) -> Vec<Shape> {
        let mut out = Vec::new();
        for c in self.crows.iter().filter(|c| matches!(c.state, State::Ground | State::Startled { .. })) {
            if fauna::off_view(cull, c.pos, 8.0) {
                continue;
            }
            let lift = c.hop.map_or(0.0, |(_, _, k)| 3.0 * crate::math::sin(std::f32::consts::PI * k));
            let at = Position::new(c.pos.x, c.pos.y - lift);
            fauna::compose_grid(&mut out, at, c.heading, sitting(c, time), color);
        }
        out
    }

    /// The shadows of the crows in the air, on the ground under them.
    pub fn air_shadows(&self, time: f32, cull: Option<Rectangle>, t: &Tuning) -> Vec<Shape> {
        let mut out = Vec::new();
        for c in &self.crows {
            let State::Flying { age } = c.state else { continue };
            if c.alt < 1.0 || fauna::off_view(cull, c.pos, 8.0 + c.alt) {
                continue;
            }
            fauna::compose_shadow(&mut out, c.pos, c.alt, c.heading, wings(c, age, time, t), t.crow_shadow_opacity, t);
        }
        out
    }

    /// The crows in the air and in the crowns, to draw over everything
    /// that stands: a flier at its height flapping, a perched crow on its
    /// crown, swung with it by `lean` (its tree's index to px across).
    pub fn air_shapes(&self, time: f32, cull: Option<Rectangle>, t: &Tuning, lean: impl Fn(usize) -> f32) -> Vec<Shape> {
        let mut out = Vec::new();
        for c in &self.crows {
            match c.state {
                State::Flying { age } => {
                    let at = Position::new(c.pos.x, c.pos.y - c.alt);
                    if !fauna::off_view(cull, at, 8.0) {
                        fauna::compose_grid(&mut out, at, c.heading, wings(c, age, time, t), color);
                    }
                }
                State::Perched(tree) => {
                    let at = Position::new(c.pos.x + lean(tree), c.pos.y);
                    if !fauna::off_view(cull, at, 8.0) {
                        fauna::compose_grid(&mut out, at, c.heading, sitting(c, time), color);
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// The grid a crow that is down shows at `time`: pecking in about a third
/// of its hashed windows, never mid-hop or startled.
fn sitting(c: &Crow, time: f32) -> &'static Grid {
    if c.hop.is_some() || matches!(c.state, State::Startled { .. }) {
        return &SITTING;
    }
    let window = (time * 3.0 + pyro::unit(c.seed, 6) * 7.0).floor() as i64 as u32;
    if pyro::unit(c.seed ^ window.wrapping_mul(0x9E37_79B9), 7) < 0.3 { &PECKING } else { &SITTING }
}

/// The grid a flying crow shows: a beat of its wings at `crow_flap_hz`,
/// held spread as it glides in low.
fn wings(c: &Crow, age: f32, _time: f32, t: &Tuning) -> &'static Grid {
    if c.alt < t.crow_altitude_px * 0.35 && age > 1.0 {
        return &WINGS_OUT;
    }
    let beat = ((age + pyro::unit(c.seed, 8)) * t.crow_flap_hz * 2.0).floor() as i64;
    if beat.rem_euclid(2) == 0 { &WINGS_OUT } else { &WINGS_IN }
}

/// A crow on the grass for `dt` more seconds: finishing a hop, or once its
/// rest is up turning to a hashed heading and, most times, hopping a
/// little toward a hashed point near its flock's spot.
fn sit(c: &mut Crow, spot: Position, dt: f32, t: &Tuning) {
    if let Some((from, to, k)) = c.hop {
        let k = k + dt / HOP_SECONDS;
        if k >= 1.0 {
            c.pos = to;
            c.hop = None;
        } else {
            c.pos = from + (to - from) * k;
            c.hop = Some((from, to, k));
        }
        return;
    }
    c.rest -= dt;
    if c.rest > 0.0 {
        return;
    }
    c.leg = c.leg.wrapping_add(1);
    c.rest = t.crow_hop_seconds.max(0.2) * (0.5 + c.roll(5));
    let goal = spot + c.slot + wander(c.seed ^ c.leg.wrapping_mul(0x51ED_270B), 1);
    let way = goal - c.pos;
    match fauna::unit_of(way) {
        Some(dir) if c.roll(3) < 0.7 => {
            c.heading = dir;
            let to = c.pos + dir * way.length().min(HOP_PX);
            c.hop = Some((c.pos, to, 0.0));
        }
        _ => c.heading = fauna::heading_of((c.roll(4) * 16.0) as usize),
    }
}

/// A crow in the air `age` seconds, on for `dt`: away from what flushed
/// it, then circling the flock's new spot, then in to land on it (or in
/// its tree, while the flock roosts), climbing to `crow_altitude_px` and
/// coming down on the way in.
#[allow(clippy::too_many_arguments)]
fn fly(c: &mut Crow, age: f32, spot: Position, tree: Option<usize>, fseed: u32, flights: u32, dt: f32, t: &Tuning) {
    let land = match tree {
        Some(_) => spot + Vec2::new((c.roll(1) * 2.0 - 1.0) * PERCH_PX.0, (c.roll(2) * 2.0 - 1.0) * PERCH_PX.1),
        None => spot + c.slot + wander(c.seed ^ flights.wrapping_mul(0x68E3_1DA4), 1),
    };
    let scatter = t.crow_scatter_seconds.max(0.0) * (0.8 + 0.4 * c.roll(13));
    let circle = scatter + t.crow_circle_seconds.max(0.0) * (0.75 + 0.5 * pyro::unit(fseed ^ flights, 3));
    let to_land = land - c.pos;
    let dist = to_land.length();
    let toward = fauna::unit_of(to_land).unwrap_or(c.heading);
    let want = if age < scatter {
        fauna::turned(fauna::unit_of(c.pos - c.from).unwrap_or(c.heading), (c.roll(12) - 0.5) * 1.0)
    } else if age < circle {
        // Round the spot, the flock all one way.
        let side = if pyro::unit(fseed, 9) < 0.5 { 1.0 } else { -1.0 };
        let tangent = Vec2::new(-toward.y * side, toward.x * side);
        fauna::unit_of(tangent + toward * 0.35).unwrap_or(toward)
    } else {
        toward
    };
    let turn = t.crow_turn_deg.to_radians() * (48.0 / dist.max(1.0)).max(1.0) * dt;
    c.heading = fauna::steer(c.heading, want, turn);
    let coming_in = age >= circle;
    let mut speed = t.crow_fly_speed.max(1.0) * (0.9 + 0.2 * pyro::unit(c.seed, 10));
    if coming_in {
        speed *= (dist / 64.0).clamp(0.3, 1.0);
    }
    let goal = if coming_in && dist < 48.0 { t.crow_altitude_px * dist / 48.0 } else { t.crow_altitude_px };
    let rate = t.crow_climb_speed.max(1.0) * dt;
    c.alt = if goal > c.alt { (c.alt + rate).min(goal) } else { (c.alt - rate * 2.0).max(goal) };
    if (coming_in && dist <= speed * dt + 1.0) || age > MAX_FLIGHT {
        c.pos = land;
        c.alt = 0.0;
        c.hop = None;
        c.leg = c.leg.wrapping_add(1);
        c.rest = t.crow_hop_seconds.max(0.2) * (0.5 + c.roll(5));
        c.state = match tree {
            Some(i) => State::Perched(i),
            None => State::Ground,
        };
        return;
    }
    c.pos += c.heading * (speed * dt);
    c.state = State::Flying { age };
}

/// Row `row` (0..4) of `fauna::tests::preview`: a crow sitting, pecking,
/// wings out, wings in.
#[cfg(test)]
pub fn preview(out: &mut Vec<Shape>, row: u32, at: Position, heading: Vec2) {
    let grids: [&Grid; 4] = [&SITTING, &PECKING, &WINGS_OUT, &WINGS_IN];
    if let Some(grid) = grids.get(row as usize) {
        fauna::compose_grid(out, at, heading, grid, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PHYSICS_FIXED_DT;

    /// A meadow: every cell of a 30 x 16 field open but a wall line down
    /// column 15, and two trees.
    fn meadow() -> Ground {
        let mut open = Vec::new();
        for r in 1..16 {
            for c in 1..30 {
                if c != 15 {
                    open.push((c, r));
                }
            }
        }
        open.retain(|&c| c != (4, 4) && c != (25, 12));
        open.sort();
        let trees = vec![((4, 4), fauna::centre((4, 4))), ((25, 12), fauna::centre((25, 12)))];
        Ground { open, trees, glades: Vec::new() }
    }

    fn quick() -> Tuning {
        Tuning { crow_flock_cells: 120.0, crow_roost_share: 1.0, ..Tuning::DEFAULT }
    }

    fn run(crows: &mut Crows, seconds: f32, t: &Tuning) {
        let n = (seconds / PHYSICS_FIXED_DT).round() as usize;
        for _ in 0..n {
            crows.step(PHYSICS_FIXED_DT, t);
        }
    }

    #[test]
    fn flocks_sit_on_open_grass_and_roosts_in_trees() {
        let t = quick();
        let g = meadow();
        let crows = Crows::new(&g, 7, &t);
        let ground: Vec<&Crow> = crows.crows.iter().filter(|c| c.state == State::Ground).collect();
        assert!(ground.len() >= 10, "{} crows on the grass", ground.len());
        assert!(crows.flocks.iter().filter(|f| f.tree.is_none()).count() >= 2);
        for c in &ground {
            let cell = crate::map::world_to_cell(c.pos);
            assert!(g.open.binary_search(&cell).is_ok(), "a crow at {:?} off open grass", c.pos);
            // Never beside the wall line.
            assert!((c.pos.x - 15.0 * 32.0).abs() > 16.0);
        }
        let perched = crows.crows.iter().filter(|c| matches!(c.state, State::Perched(_))).count();
        assert!((2..=6).contains(&perched), "{perched} crows in the crowns");
        let none = Tuning { crow_enabled: false, ..t };
        assert!(Crows::new(&g, 7, &none).crows.is_empty());
        assert!(Crows::new(&Ground::default(), 7, &t).crows.is_empty());
    }

    #[test]
    fn the_same_seed_places_the_same_crows() {
        let t = quick();
        let g = meadow();
        assert_eq!(Crows::new(&g, 3, &t), Crows::new(&g, 3, &t));
        assert_ne!(Crows::new(&g, 3, &t).crows, Crows::new(&g, 4, &t).crows);
        let replay = || {
            let mut crows = Crows::new(&g, 3, &t);
            run(&mut crows, 2.0, &t);
            let at = crows.crows[0].pos;
            crows.stir(&Stir { blasts: vec![at + Vec2::new(30.0, 0.0)], ..Stir::default() }, &t);
            run(&mut crows, 3.0, &t);
            (crows.ground_shapes(5.0, None), crows.air_shapes(5.0, None, &t, |_| 0.0))
        };
        assert_eq!(replay(), replay());
    }

    #[test]
    fn a_blast_flushes_the_flock_which_lands_on_other_grass() {
        let t = quick();
        let g = meadow();
        let mut crows = Crows::new(&g, 11, &t);
        let fi = crows.crows.iter().find(|c| c.state == State::Ground).unwrap().flock;
        let cause = crows.flocks[fi].spot + Vec2::new(40.0, 0.0);
        crows.stir(&Stir { blasts: vec![cause], ..Stir::default() }, &t);
        let flock: Vec<usize> = (0..crows.crows.len()).filter(|&i| crows.crows[i].flock == fi).collect();
        assert!(flock.iter().all(|&i| matches!(crows.crows[i].state, State::Startled { .. })));
        let spot = crows.flocks[fi].spot;
        assert!(spot.distance_to(cause) >= t.crow_land_min_px - 1.0, "it settles away from the blast");
        run(&mut crows, 1.0, &t);
        for &i in &flock {
            let c = &crows.crows[i];
            assert!(matches!(c.state, State::Flying { .. }), "{:?}", c.state);
            assert!(c.alt > 10.0, "up: {}", c.alt);
        }
        assert!(!crows.air_shadows(1.0, None, &t).is_empty(), "a shadow under each");
        run(&mut crows, 25.0, &t);
        for &i in &flock {
            let c = &crows.crows[i];
            assert_eq!(c.state, State::Ground, "down again");
            assert_eq!(c.alt, 0.0);
            assert!(c.pos.distance_to(spot) <= SPREAD_PX + HOP_PX * 40.0);
            assert!(g.open.binary_search(&crate::map::world_to_cell(c.pos)).is_ok(), "on open grass: {:?}", c.pos);
        }
    }

    #[test]
    fn a_hull_flushes_a_flock_only_within_reach() {
        let t = quick();
        let g = meadow();
        let mut crows = Crows::new(&g, 5, &t);
        let c0 = crows.crows.iter().find(|c| c.state == State::Ground).unwrap().clone();
        let far = c0.pos + Vec2::new(t.crow_flush_px + 200.0, 0.0);
        let before = crows.clone();
        let others_near = crows.crows.iter().any(|c| c.state == State::Ground && c.pos.distance_to(far) < t.crow_flush_px);
        crows.stir(&Stir { hulls: vec![far], ..Stir::default() }, &t);
        if !others_near {
            assert_eq!(crows, before, "a hull out of reach flushes nothing");
        }
        crows.stir(&Stir { hulls: vec![c0.pos + Vec2::new(20.0, 0.0)], ..Stir::default() }, &t);
        assert!(crows.crows.iter().filter(|c| c.flock == c0.flock).all(|c| matches!(c.state, State::Startled { .. })));
    }

    #[test]
    fn a_struck_tree_flushes_its_crows() {
        let t = quick();
        let g = meadow();
        let mut crows = Crows::new(&g, 9, &t);
        let Some(perched) = crows.crows.iter().find(|c| matches!(c.state, State::Perched(_))).cloned() else { panic!("a roost") };
        let State::Perched(tree) = perched.state else { unreachable!() };
        // A hull going by does not move it.
        crows.stir(&Stir { hulls: vec![perched.pos], ..Stir::default() }, &t);
        assert_eq!(crows.crows.iter().find(|c| c.seed == perched.seed).unwrap().state, perched.state);
        crows.stir(&Stir { trees: vec![fauna::centre(g.trees[tree].0) + Vec2::new(10.0, 0.0)], ..Stir::default() }, &t);
        assert!(matches!(crows.crows.iter().find(|c| c.seed == perched.seed).unwrap().state, State::Startled { .. }));
        run(&mut crows, 30.0, &t);
        let c = crows.crows.iter().find(|c| c.seed == perched.seed).unwrap();
        assert_eq!(c.state, State::Ground, "it settles on the grass");
    }

    #[test]
    fn a_crow_is_whole_blocks_in_its_colours() {
        let t = quick();
        let mut crows = Crows::new(&meadow(), 2, &t);
        run(&mut crows, 3.0, &t);
        let at = crows.crows[0].pos;
        crows.stir(&Stir { shots: vec![at], ..Stir::default() }, &t);
        run(&mut crows, 1.5, &t);
        let mut all = crows.ground_shapes(4.5, None);
        all.extend(crows.air_shapes(4.5, None, &t, |_| 2.0));
        assert!(!all.is_empty());
        for s in &all {
            let Shape::Mark { pos, size, color } = *s else { panic!("{s:?}") };
            assert_eq!(size, 2);
            assert!(pos.x % 2.0 == 0.0 && pos.y % 2.0 == 0.0, "{pos:?}");
            assert!([BODY, WING, HEAD, BEAK].contains(&color), "{color:?}");
        }
        for s in &crows.air_shadows(4.5, None, &t) {
            let Shape::Mark { color, .. } = *s else { panic!() };
            assert_eq!((color.r, color.g, color.b), (0, 0, 0));
        }
        // A view far away draws none.
        let far = Some(Rectangle::new(5000.0, 5000.0, 100.0, 100.0));
        assert!(crows.ground_shapes(4.5, far).is_empty());
        assert!(crows.air_shapes(4.5, far, &t, |_| 0.0).is_empty());
    }
}
