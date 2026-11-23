//! Mallards (docs/field-life.md "Ducks"): a group of three to five on a
//! lake's open water - the deep cells the fish swim in (`fish.rs`) - one
//! group per `duck_group_cells` of it. They paddle slowly from near one
//! deep cell's centre to near a neighbour's with a pale wake falling
//! behind, dip their heads now and then, and paddle off at
//! `duck_paddle_speed` from a hull within `duck_shy_px`. A shot or a hit
//! within `duck_shot_px` or a blast within `duck_blast_px` puts the whole
//! group up: each runs along the surface away from it throwing spray
//! (`duck_run_seconds`), takes off, flies with its shadow on the water to
//! another stretch of open water at least `duck_land_min_px` away - this
//! lake's or another's - and lands there. None under ice.
//!
//! Presentation only, like everything in `fauna.rs`: every choice hashed
//! from the round's seed, the lake and the duck, nothing rolled.

use crate::fauna::{self, Grid, Stir};
use crate::fish::{deep, neighbours, ROAM_PX};
use crate::ground::WaterLayout;
use crate::math::{Color, Rectangle, Vec2};
use crate::pyro::{self, Shape};
use crate::tuning::Tuning;
use crate::Position;

/// The mallard's colours, Puny Palette steps (tools/punypalette.py): the
/// body WOOD_ASH, the back STONE_MD, the head TEAL_DK, the bill
/// GOLD_BRIGHT.
pub const BODY: Color = Color::new(0x73, 0x62, 0x4D, 255);
pub const BACK: Color = Color::new(0x9E, 0x9E, 0x96, 255);
pub const HEAD: Color = Color::new(0x00, 0x7E, 0x53, 255);
pub const BILL: Color = Color::new(0xEE, 0xA3, 0x43, 255);

/// The wake (BLUE_PALE) and the spray thrown on the run (WHITE, then
/// BLUE_PALE).
pub const WAKE: Color = Color::new(0x93, 0xEC, 0xE2, 255);
pub const SPRAY: Color = Color::new(0xFF, 0xFF, 0xFF, 255);

/// Swimming, facing up, 6 x 12 px: bill, head, breast, the back between
/// the flanks and the tail.
const SWIM: [&str; 6] = [".k.", ".H.", "BBB", "BWB", "BWB", ".B."];

/// Dabbling: the head under, a glimpse of it in the breast.
const DIP: [&str; 6] = ["...", "...", "BHB", "BWB", "BWB", ".B."];

/// Flying, wings spread, and swept back.
const WINGS_OUT: [&str; 5] = ["..k..", "..H..", "WWBWW", "W.B.W", "..B.."];
const WINGS_IN: [&str; 5] = ["..k..", "..H..", ".WBW.", ".WBW.", "..B.."];

fn color(ch: u8) -> Option<Color> {
    match ch {
        b'B' => Some(BODY),
        b'W' => Some(BACK),
        b'H' => Some(HEAD),
        b'k' => Some(BILL),
        _ => None,
    }
}

/// The longest a duck flies before it is set down where it was going, s.
const MAX_FLIGHT: f32 = 30.0;

/// How long the spray of a landing lasts, s.
const SPLASH_SECONDS: f32 = 0.35;

/// How far ahead a duck's run must stay over water, px.
const RUN_PX: f32 = 28.0;

/// What a duck is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// On the water.
    Swim,
    /// Running along the surface to take off, `age` seconds in; below 0 it
    /// has not started yet.
    Run { age: f32 },
    /// In the air for `age` seconds.
    Fly { age: f32 },
}

/// One duck.
#[derive(Clone, Debug, PartialEq)]
pub struct Duck {
    /// On the water, where it swims; in the air, the point under it.
    pub pos: Position,
    /// Where it paddles to now: within `ROAM_PX` of a deep cell's centre.
    to: Position,
    pub heading: Vec2,
    speed: f32,
    /// 1 the moment a hull scares it, falling to 0 over `duck_calm_seconds`.
    pub fright: f32,
    /// What it paddles, runs or flies from.
    from: Position,
    /// Seconds of a landing's spray left.
    splash: f32,
    /// This leg is a rest: slowly, to a spot in the same cell.
    idle: bool,
    pub alt: f32,
    pub state: State,
    /// Legs and flights so far: salts each hashed choice.
    leg: u32,
    seed: u32,
    group: usize,
}

/// A group: its lake, the cell it makes for and how often it has flown.
#[derive(Clone, Debug, PartialEq)]
struct Group {
    lake: usize,
    seed: u32,
    flights: u32,
    /// The cell it last landed on, which it makes for until its next
    /// spot comes round; `None` before its first flight.
    landed: Option<(i32, i32)>,
}

/// Every duck of a round, stepped on the round's clock.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ducks {
    lakes: Vec<Vec<(i32, i32)>>,
    /// Open water: deep cells all eight of whose neighbours are deep too,
    /// sorted - where a group lands (every deep cell where no lake has
    /// one).
    landings: Vec<(i32, i32)>,
    groups: Vec<Group>,
    ducks: Vec<Duck>,
    clock: f32,
}

impl Duck {
    fn roll(&self, k: u32) -> f32 {
        pyro::unit(self.seed ^ self.leg.wrapping_mul(0x9E37_79B9), k)
    }

    /// The group it belongs to, for the round: what takes off with it, and
    /// what the sound calls and alarms by (`audio::life`).
    pub fn group(&self) -> usize {
        self.group
    }

    /// Whether it has its head under at the round's clock `time`: calm on
    /// the water, in about one of its hashed windows in seven. The one rule
    /// `water_shapes` draws the dip by and the sound dabbles by.
    pub fn dabbling(&self, time: f32) -> bool {
        let window = (time / 0.9 + pyro::unit(self.seed, 6) * 5.0).floor() as i64 as u32;
        self.state == State::Swim && self.fright == 0.0 && pyro::unit(self.seed ^ window.wrapping_mul(0x9E37_79B9), 8) < 0.15
    }

    /// Pick where to paddle next: away from what scared it while it is
    /// frightened, else toward `target` most of the time, a wander or a
    /// rest otherwise (the fish's legs, `fish::Fish::next_leg`).
    fn next_leg(&mut self, water: &WaterLayout, target: Option<(i32, i32)>) {
        self.leg = self.leg.wrapping_add(1);
        let here = crate::map::world_to_cell(self.pos);
        let options = neighbours(water, here);
        let here_ok = deep(water, here);
        let cell = if self.fright > 0.0 {
            self.idle = false;
            let far = |c: (i32, i32)| fauna::centre(c).distance_to(self.from);
            let mut best = if here_ok { Some(here) } else { None };
            for &c in &options {
                if best.is_none_or(|b| far(c) > far(b)) {
                    best = Some(c);
                }
            }
            best
        } else if options.is_empty() || (here_ok && self.roll(1) < 0.35) {
            self.idle = true;
            here_ok.then_some(here)
        } else {
            self.idle = false;
            match target {
                Some(goal) if self.roll(2) < 0.6 => {
                    let near = |c: (i32, i32)| (c.0 - goal.0).pow(2) + (c.1 - goal.1).pow(2);
                    let mut best = if here_ok { here } else { options[0] };
                    for &c in &options {
                        if near(c) < near(best) {
                            best = c;
                        }
                    }
                    Some(best)
                }
                _ => Some(options[((self.roll(3) * options.len() as f32) as usize).min(options.len() - 1)]),
            }
        };
        let Some(cell) = cell else {
            self.to = self.pos;
            return;
        };
        let offset = Vec2::new((self.roll(4) * 2.0 - 1.0) * ROAM_PX, (self.roll(5) * 2.0 - 1.0) * ROAM_PX);
        self.to = fauna::centre(cell) + offset;
    }

    /// Paddle `dt` seconds toward its leg's end, onto the next leg there.
    fn swim(&mut self, water: &WaterLayout, target: (i32, i32), dt: f32, t: &Tuning) {
        self.fright = (self.fright - dt / t.duck_calm_seconds.max(0.1)).max(0.0);
        self.splash = (self.splash - dt).max(0.0);
        let cruise = t.duck_speed * (0.75 + 0.5 * pyro::unit(self.seed, 7)) * if self.idle { 0.4 } else { 1.0 };
        let want = cruise + (t.duck_paddle_speed - cruise).max(0.0) * self.fright;
        self.speed += (want - self.speed) * (dt * 3.0).min(1.0);
        let mut left = self.speed.max(0.0) * dt;
        for _ in 0..2 {
            let way = self.to - self.pos;
            let dist = way.length();
            if dist > left {
                self.pos += way * (left / dist);
                break;
            }
            self.pos = self.to;
            left -= dist;
            self.next_leg(water, Some(target));
            if left <= 0.0 {
                break;
            }
        }
        if let Some(want) = fauna::unit_of(self.to - self.pos) {
            let k = (dt * 3.0).min(1.0);
            self.heading = fauna::unit_of(self.heading + (want - self.heading) * k).unwrap_or(want);
        }
    }
}

impl Ducks {
    /// The ducks of a round on `water`, hashed from its `seed`: none on
    /// ice.
    pub fn new(water: &WaterLayout, seed: u64, t: &Tuning) -> Ducks {
        let salt = (seed ^ (seed >> 32)) as u32;
        let mut ducks = Ducks::default();
        let most = t.duck_per_group.max(0);
        if water.is_frozen() || !t.duck_enabled || most == 0 {
            return ducks;
        }
        ducks.lakes = crate::fish::lakes(water);
        let mut landings: Vec<(i32, i32)> = ducks
            .lakes
            .iter()
            .flatten()
            .copied()
            .filter(|&(c, r)| (-1..=1).all(|dr| (-1..=1).all(|dc| deep(water, (c + dc, r + dr)))))
            .collect();
        if landings.is_empty() {
            landings = ducks.lakes.iter().flatten().copied().collect();
        }
        landings.sort();
        ducks.landings = landings;
        let fewest = (most + 1) / 2;
        for (li, lake) in ducks.lakes.iter().enumerate() {
            if lake.len() < t.duck_min_lake_cells.max(1) as usize {
                continue;
            }
            let groups = ((lake.len() as f32 / t.duck_group_cells.max(1.0)).round() as usize).max(1);
            for k in 0..groups {
                let gseed = crate::blast::seed_at(fauna::centre(lake[0]), salt ^ (k as u32).wrapping_mul(0x0D0C_C5ED) ^ 0xD0C5);
                let group = ducks.groups.len();
                ducks.groups.push(Group { lake: li, seed: gseed, flights: 0, landed: None });
                let count = (fewest.min(most) + (pyro::unit(gseed, 1) * (most - fewest.min(most) + 1) as f32) as i32).min(most);
                let home = lake[((pyro::unit(gseed, 2) * lake.len() as f32) as usize).min(lake.len() - 1)];
                let near: Vec<(i32, i32)> = std::iter::once(home).chain(neighbours(water, home)).collect();
                for i in 0..count as u32 {
                    let seed = crate::blast::seed_at(fauna::centre(home), gseed ^ i.wrapping_mul(0x2545_F491));
                    let cell = near[((pyro::unit(seed, 1) * near.len() as f32) as usize).min(near.len() - 1)];
                    let pos = fauna::centre(cell) + Vec2::new((pyro::unit(seed, 2) * 2.0 - 1.0) * ROAM_PX, (pyro::unit(seed, 3) * 2.0 - 1.0) * ROAM_PX);
                    ducks.ducks.push(Duck {
                        pos,
                        to: pos,
                        heading: fauna::heading_of((pyro::unit(seed, 4) * 16.0) as usize),
                        speed: 0.0,
                        fright: 0.0,
                        from: pos,
                        splash: 0.0,
                        idle: false,
                        alt: 0.0,
                        state: State::Swim,
                        leg: 0,
                        seed,
                        group,
                    });
                }
            }
        }
        ducks
    }

    pub fn ducks(&self) -> &[Duck] {
        &self.ducks
    }

    /// The cell group `g` makes for at the clock: the cell it landed on
    /// for a while after a flight, else a spot of its lake that changes
    /// every `duck_drift_seconds`.
    fn target(&self, g: usize, t: &Tuning) -> (i32, i32) {
        let s = &self.groups[g];
        let lake = &self.lakes[s.lake];
        let epoch = (self.clock / t.duck_drift_seconds.max(0.5) + pyro::unit(s.seed, 5)).floor() as u32;
        match s.landed {
            Some(cell) if !epoch.is_multiple_of(3) => cell,
            _ => lake[((pyro::unit(s.seed ^ epoch.wrapping_mul(0x27D4_EB2F), 6) * lake.len() as f32) as usize).min(lake.len() - 1)],
        }
    }

    /// This step's `stir`: a hull near a duck on the water sends it
    /// paddling off; a shot, a hit or a blast near any duck of a group on
    /// the water puts the group up.
    pub fn stir(&mut self, stir: &Stir, water: &WaterLayout, t: &Tuning) {
        if self.ducks.is_empty() || water.is_frozen() {
            return;
        }
        let mut causes: Vec<Option<Position>> = vec![None; self.groups.len()];
        for d in self.ducks.iter_mut().filter(|d| d.state == State::Swim) {
            if causes[d.group].is_none() {
                causes[d.group] = Stir::near(&stir.shots, d.pos, t.duck_shot_px).or_else(|| Stir::near(&stir.blasts, d.pos, t.duck_blast_px));
            }
            if let Some(hull) = Stir::near(&stir.hulls, d.pos, t.duck_shy_px) {
                let fresh = d.fright < 0.5;
                let toward = d.to.distance_to(hull) < d.pos.distance_to(hull);
                d.fright = 1.0;
                d.from = hull;
                if fresh || toward {
                    d.next_leg(water, None);
                }
            }
        }
        for (g, cause) in causes.into_iter().enumerate() {
            if let Some(at) = cause {
                self.take_off(g, at, water, t);
            }
        }
    }

    /// Put group `g` up away from `cause`: it picks the open water it
    /// lands on, at least `duck_land_min_px` from the cause where there is
    /// any (hashed among those), and each of its ducks on the water runs
    /// off a hashed moment from now along the heading nearest straight
    /// away that keeps its run over water.
    pub fn take_off(&mut self, g: usize, cause: Position, water: &WaterLayout, t: &Tuning) {
        if self.landings.is_empty() {
            return;
        }
        let group = &mut self.groups[g];
        group.flights = group.flights.wrapping_add(1);
        let roll = pyro::unit(group.seed ^ group.flights.wrapping_mul(0x27D4_EB2F), 1);
        let far: Vec<(i32, i32)> = self.landings.iter().copied().filter(|&c| fauna::centre(c).distance_to(cause) >= t.duck_land_min_px).collect();
        let cell = if far.is_empty() {
            let mut best = self.landings[0];
            for &c in &self.landings {
                if fauna::centre(c).distance_to(cause) > fauna::centre(best).distance_to(cause) {
                    best = c;
                }
            }
            best
        } else {
            far[((roll * far.len() as f32) as usize).min(far.len() - 1)]
        };
        group.landed = Some(cell);
        if let Some(li) = self.lakes.iter().position(|lake| lake.binary_search(&cell).is_ok()) {
            group.lake = li;
        }
        for d in self.ducks.iter_mut().filter(|d| d.group == g && d.state == State::Swim) {
            d.leg = d.leg.wrapping_add(1);
            d.from = cause;
            let away = fauna::unit_of(d.pos - cause).unwrap_or(d.heading);
            d.heading = run_heading(water, d.pos, away);
            d.state = State::Run { age: -d.roll(11) * t.duck_stagger_seconds.max(0.0) };
        }
    }

    /// Move every duck on `dt` seconds. Nothing moves on ice.
    pub fn step(&mut self, water: &WaterLayout, dt: f32, t: &Tuning) {
        self.clock += dt;
        if water.is_frozen() || self.ducks.is_empty() {
            return;
        }
        let targets: Vec<(i32, i32)> = (0..self.groups.len()).map(|g| self.target(g, t)).collect();
        for d in &mut self.ducks {
            let group = &self.groups[d.group];
            match d.state {
                State::Swim => d.swim(water, targets[d.group], dt, t),
                State::Run { age } => {
                    let age = age + dt;
                    if age >= 0.0 {
                        let k = (age / t.duck_run_seconds.max(0.05)).min(1.0);
                        let speed = 12.0 + (t.duck_fly_speed - 12.0).max(0.0) * k;
                        d.pos += d.heading * (speed * dt);
                        // Off the water in the run's last third.
                        d.alt = t.duck_altitude_px * 0.25 * ((k - 0.66) / 0.34).max(0.0);
                    }
                    d.state = if age >= t.duck_run_seconds { State::Fly { age: 0.0 } } else { State::Run { age } };
                }
                State::Fly { age } => {
                    let age = age + dt;
                    let cell = group.landed.unwrap_or_else(|| crate::map::world_to_cell(d.pos));
                    let land = fauna::centre(cell) + Vec2::new((d.roll(4) * 2.0 - 1.0) * ROAM_PX, (d.roll(5) * 2.0 - 1.0) * ROAM_PX);
                    let to_land = land - d.pos;
                    let dist = to_land.length();
                    let toward = fauna::unit_of(to_land).unwrap_or(d.heading);
                    let turn = 2.5 * (48.0 / dist.max(1.0)).max(1.0) * dt;
                    d.heading = fauna::steer(d.heading, toward, turn);
                    let speed = t.duck_fly_speed.max(1.0) * (dist / 64.0).clamp(0.4, 1.0);
                    let goal = if dist < 56.0 { t.duck_altitude_px * dist / 56.0 } else { t.duck_altitude_px };
                    let rate = 30.0 * dt;
                    d.alt = if goal > d.alt { (d.alt + rate).min(goal) } else { (d.alt - rate * 2.0).max(goal) };
                    if dist <= speed * dt + 1.0 || age > MAX_FLIGHT {
                        d.pos = land;
                        d.to = land;
                        d.alt = 0.0;
                        d.speed = 0.0;
                        d.fright = 0.0;
                        d.splash = SPLASH_SECONDS;
                        d.state = State::Swim;
                    } else {
                        d.pos += d.heading * (speed * dt);
                        d.state = State::Fly { age };
                    }
                }
            }
        }
    }

    /// The ducks on the water, to draw over the water's tiles: each one's
    /// wake or spray, then the duck - dabbling now and then
    /// (`Duck::dabbling`) - at the round's clock `time`. Nothing on ice.
    pub fn water_shapes(&self, water: &WaterLayout, time: f32, cull: Option<Rectangle>, t: &Tuning) -> Vec<Shape> {
        let mut out = Vec::new();
        if water.is_frozen() {
            return out;
        }
        let beat = (time * 6.0).floor() as i64;
        for d in &self.ducks {
            if fauna::off_view(cull, d.pos, 24.0) {
                continue;
            }
            let right = Vec2::new(-d.heading.y, d.heading.x);
            match d.state {
                State::Swim => {
                    // A V of blocks falling behind, every other one in
                    // step, fainter as it spreads.
                    let moving = (d.speed / t.duck_speed.max(0.1)).min(1.0);
                    if moving > 0.3 {
                        for k in 1..=4i64 {
                            if (beat + k).rem_euclid(2) == 1 {
                                continue;
                            }
                            let a = (t.duck_wake_opacity - 0.12 * k as f32) * moving;
                            for side in [-1.0, 1.0] {
                                let at = d.pos - d.heading * (6.0 + 3.0 * k as f32) + right * (side * 2.0 * k as f32);
                                if water.depth_at(at).is_wet() {
                                    out.push(Shape::Mark { pos: at, size: 2, color: pyro::alpha(WAKE, a) });
                                }
                            }
                        }
                    }
                    if d.splash > 0.0 {
                        spray(&mut out, water, d, time, 1.0);
                    }
                    fauna::compose_grid(&mut out, d.pos, d.heading, if d.dabbling(time) { &DIP } else { &SWIM }, color);
                }
                State::Run { age } if d.alt < 1.0 => {
                    if age >= 0.0 {
                        spray(&mut out, water, d, time, -1.0);
                    }
                    let grid: &Grid = if age > t.duck_run_seconds * 0.4 { flap(d, age, t) } else { &SWIM };
                    fauna::compose_grid(&mut out, d.pos, d.heading, grid, color);
                }
                _ => {}
            }
        }
        out
    }

    /// The shadows of the ducks in the air, on the water or the ground
    /// under them.
    pub fn air_shadows(&self, water: &WaterLayout, cull: Option<Rectangle>, t: &Tuning) -> Vec<Shape> {
        let mut out = Vec::new();
        if water.is_frozen() {
            return out;
        }
        for d in &self.ducks {
            let age = match d.state {
                State::Fly { age } => age + t.duck_run_seconds,
                State::Run { age } => age,
                State::Swim => continue,
            };
            if d.alt < 1.0 || fauna::off_view(cull, d.pos, 8.0 + d.alt) {
                continue;
            }
            fauna::compose_shadow(&mut out, d.pos, d.alt, d.heading, flap(d, age, t), t.duck_shadow_opacity, t);
        }
        out
    }

    /// The ducks in the air, at their height, flapping.
    pub fn air_shapes(&self, water: &WaterLayout, cull: Option<Rectangle>, t: &Tuning) -> Vec<Shape> {
        let mut out = Vec::new();
        if water.is_frozen() {
            return out;
        }
        for d in &self.ducks {
            let age = match d.state {
                State::Fly { age } => age + t.duck_run_seconds,
                State::Run { age } if d.alt >= 1.0 => age,
                _ => continue,
            };
            let at = Position::new(d.pos.x, d.pos.y - d.alt);
            if !fauna::off_view(cull, at, 8.0) {
                fauna::compose_grid(&mut out, at, d.heading, flap(d, age, t), color);
            }
        }
        out
    }
}

/// The beat of a duck's wings `age` seconds into its take-off.
fn flap(d: &Duck, age: f32, t: &Tuning) -> &'static Grid {
    let beat = ((age + pyro::unit(d.seed, 9)) * t.duck_flap_hz * 2.0).floor() as i64;
    if beat.rem_euclid(2) == 0 { &WINGS_OUT } else { &WINGS_IN }
}

/// Spray round a duck: behind it on the run (`behind` -1), all round as it
/// lands (1) - white at the first block, pale blue after, hashed afresh
/// every twentieth of a second, only where there is water.
fn spray(out: &mut Vec<Shape>, water: &WaterLayout, d: &Duck, time: f32, behind: f32) {
    let window = (time * 20.0).floor() as i64 as u32;
    for k in 0..3u32 {
        let jitter = Vec2::new(pyro::unit(d.seed ^ window, 20 + k) - 0.5, pyro::unit(d.seed ^ window, 30 + k) - 0.5) * 6.0;
        let at = if behind < 0.0 {
            d.pos - d.heading * (4.0 + 4.0 * k as f32) + jitter
        } else {
            d.pos + fauna::turned(d.heading, k as f32 * 2.1) * 6.0 + jitter
        };
        if water.depth_at(at).is_wet() {
            out.push(Shape::Mark { pos: at, size: 2, color: if k == 0 { SPRAY } else { WAKE } });
        }
    }
}

/// The heading a duck at `at` runs along to take off: of the sixteen, the
/// nearest `away` whose run (`RUN_PX` ahead) stays over water; `away`
/// itself if none does.
fn run_heading(water: &WaterLayout, at: Position, away: Vec2) -> Vec2 {
    let mut best: Option<(f32, Vec2)> = None;
    for i in 0..16 {
        let h = fauna::heading_of(i);
        let ok = [0.5, 1.0].iter().all(|&k| water.depth_at(at + h * (RUN_PX * k)).is_wet());
        let score = h.dot(away);
        if ok && best.is_none_or(|(b, _)| score > b + 1e-6) {
            best = Some((score, h));
        }
    }
    best.map_or(away, |(_, h)| h)
}

/// Row `row` (4..8) of `fauna::tests::preview`: a duck swimming,
/// dabbling, wings out, wings in.
#[cfg(test)]
pub fn preview(out: &mut Vec<Shape>, row: u32, at: Position, heading: Vec2) {
    let grids: [&Grid; 4] = [&SWIM, &DIP, &WINGS_OUT, &WINGS_IN];
    if let Some(grid) = row.checked_sub(4).and_then(|r| grids.get(r as usize)) {
        fauna::compose_grid(out, at, heading, grid, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ground::Depth;
    use crate::PHYSICS_FIXED_DT;

    fn world(cells: &[(i32, i32)]) -> Vec<Position> {
        cells.iter().map(|&c| fauna::centre(c)).collect()
    }

    /// Two ponds: a big one west, a small one east.
    fn ponds() -> WaterLayout {
        let mut cells = Vec::new();
        for c in 2..=11 {
            for r in 2..=9 {
                cells.push((c, r));
            }
        }
        for c in 16..=21 {
            for r in 3..=8 {
                cells.push((c, r));
            }
        }
        WaterLayout::build(800.0, 400.0, &[], &world(&cells))
    }

    fn quick() -> Tuning {
        Tuning { duck_group_cells: 24.0, duck_min_lake_cells: 4, ..Tuning::DEFAULT }
    }

    fn run(ducks: &mut Ducks, water: &WaterLayout, seconds: f32, t: &Tuning) {
        let n = (seconds / PHYSICS_FIXED_DT).round() as usize;
        for _ in 0..n {
            ducks.step(water, PHYSICS_FIXED_DT, t);
        }
    }

    #[test]
    fn groups_swim_on_open_water_and_none_on_ice() {
        let t = quick();
        let water = ponds();
        let mut ducks = Ducks::new(&water, 7, &t);
        assert!(ducks.ducks.len() >= 6, "{} ducks", ducks.ducks.len());
        for i in 0..1200 {
            ducks.step(&water, PHYSICS_FIXED_DT, &t);
            if i % 100 == 0 {
                for d in &ducks.ducks {
                    assert!(water.depth_at(d.pos) == Depth::Deep || d.pos.distance_to(d.to) <= 2.0 * ROAM_PX, "{:?}", d.pos);
                }
                for s in &ducks.water_shapes(&water, i as f32 / 60.0, None, &t) {
                    let Shape::Mark { pos, .. } = *s else { panic!() };
                    assert!(water.depth_at(pos).is_wet(), "a block on the bank at {pos:?}");
                }
            }
        }
        let mut ice = ponds();
        ice.freeze();
        assert!(Ducks::new(&ice, 7, &t).ducks.is_empty());
        assert!(ducks.water_shapes(&ice, 1.0, None, &t).is_empty(), "frozen over");
        assert!(Ducks::new(&water, 7, &Tuning { duck_enabled: false, ..t }).ducks.is_empty());
        assert!(Ducks::new(&WaterLayout::build(800.0, 400.0, &[], &[]), 7, &t).ducks.is_empty());
    }

    #[test]
    fn the_same_seed_swims_the_same() {
        let t = quick();
        let water = ponds();
        let go = |seed: u64| {
            let mut ducks = Ducks::new(&water, seed, &t);
            run(&mut ducks, &water, 3.0, &t);
            let at = ducks.ducks[0].pos;
            ducks.stir(&Stir { shots: vec![at + Vec2::new(10.0, 0.0)], ..Stir::default() }, &water, &t);
            run(&mut ducks, &water, 2.0, &t);
            (ducks.water_shapes(&water, 5.0, None, &t), ducks.air_shapes(&water, None, &t), ducks.ducks.clone())
        };
        assert_eq!(go(3), go(3));
        assert_ne!(go(3).2, go(4).2);
    }

    #[test]
    fn a_hull_on_the_bank_sends_a_duck_paddling_off() {
        let t = quick();
        let water = ponds();
        let mut ducks = Ducks::new(&water, 5, &t);
        run(&mut ducks, &water, 2.0, &t);
        let at = ducks.ducks[0].pos + Vec2::new(30.0, 0.0);
        ducks.stir(&Stir { hulls: vec![at], ..Stir::default() }, &water, &t);
        assert_eq!(ducks.ducks[0].fright, 1.0);
        assert_eq!(ducks.ducks[0].state, State::Swim, "a hull does not put it up");
        let before = ducks.ducks[0].pos.distance_to(at);
        run(&mut ducks, &water, 1.5, &t);
        assert!(ducks.ducks[0].pos.distance_to(at) > before + 5.0);
    }

    #[test]
    fn a_shot_puts_the_group_up_and_it_lands_on_other_water() {
        let t = quick();
        let water = ponds();
        let mut ducks = Ducks::new(&water, 9, &t);
        run(&mut ducks, &water, 1.0, &t);
        let g = ducks.ducks[0].group;
        let cause = ducks.ducks[0].pos + Vec2::new(8.0, 0.0);
        ducks.stir(&Stir { shots: vec![cause], ..Stir::default() }, &water, &t);
        let group: Vec<usize> = (0..ducks.ducks.len()).filter(|&i| ducks.ducks[i].group == g).collect();
        assert!(group.iter().all(|&i| matches!(ducks.ducks[i].state, State::Run { .. })));
        let cell = ducks.groups[g].landed.expect("somewhere to land");
        assert!(fauna::centre(cell).distance_to(cause) >= t.duck_land_min_px || ducks.landings.iter().all(|&c| fauna::centre(c).distance_to(cause) < t.duck_land_min_px));
        run(&mut ducks, &water, 0.2, &t);
        assert!(!ducks.water_shapes(&water, 1.2, None, &t).iter().all(|s| !matches!(s, Shape::Mark { color, .. } if *color == SPRAY)), "spray on the run");
        run(&mut ducks, &water, t.duck_run_seconds + 0.6, &t);
        for &i in &group {
            assert!(matches!(ducks.ducks[i].state, State::Fly { .. }), "{:?}", ducks.ducks[i].state);
            assert!(ducks.ducks[i].alt > 4.0);
        }
        assert!(!ducks.air_shadows(&water, None, &t).is_empty());
        run(&mut ducks, &water, 20.0, &t);
        for &i in &group {
            let d = &ducks.ducks[i];
            assert_eq!(d.state, State::Swim, "down again");
            assert_eq!(water.depth_at(d.pos), Depth::Deep, "on open water: {:?}", d.pos);
        }
    }

    #[test]
    fn a_duck_is_whole_blocks_in_its_colours() {
        let t = quick();
        let water = ponds();
        let mut ducks = Ducks::new(&water, 2, &t);
        run(&mut ducks, &water, 2.0, &t);
        let at = ducks.ducks[0].pos;
        ducks.stir(&Stir { blasts: vec![at] , ..Stir::default() }, &water, &t);
        run(&mut ducks, &water, 1.4, &t);
        let mut all = ducks.water_shapes(&water, 3.4, None, &t);
        all.extend(ducks.air_shapes(&water, None, &t));
        assert!(!all.is_empty());
        for s in &all {
            let Shape::Mark { pos, size, color } = *s else { panic!("{s:?}") };
            assert_eq!(size, 2);
            let c = (color.r, color.g, color.b);
            assert!([BODY, BACK, HEAD, BILL, WAKE, SPRAY].iter().any(|k| (k.r, k.g, k.b) == c), "{color:?}");
            let _ = pos;
        }
    }
}
