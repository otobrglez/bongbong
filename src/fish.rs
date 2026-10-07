//! Fish in the water (docs/water.md, "Presentation"): small schools in the
//! lakes and the odd fish carried down a stream. They are scenery that
//! reacts: a fish darts away from a hull on the bank, from a shot flying
//! over or landing beside the water, and from a blast.
//!
//! **Presentation only.** Nothing in the simulation reads a fish, nothing
//! travels on the wire, and no fish draws the round's RNG: every choice is
//! hashed (`pyro::unit`) from the round's seed, the lake's cells, the fish
//! and its leg. A school's state lives in `Shoal`, which `fx::Fx` owns
//! (so `app.rs` owns it, as it owns the particles) and steps on the round's
//! clock after every simulation step (`Shoal::observe`). The same steps
//! from the same seed give the same fish, so a dev-server lockstep replays
//! them, and a replica reads the same `Game` a local round does.
//!
//! **Where they swim.** A lake's open water - its `Depth::Deep` cells,
//! flat water edge to edge - in the lake they were seeded in. A fish swims
//! from a point near one deep cell's centre to a point near a neighbouring
//! deep cell's (a diagonal only where both cells beside it are deep too),
//! so its whole body stays over water: every neighbour of a deep cell is
//! painted water (a wet corner needs all four cells round it wet). A
//! school makes for one cell of its lake for `fish_school_seconds`, then
//! another; each fish mostly steps toward it, sometimes wanders or idles.
//! Streams carry a fish down the map now and then: a pure function of the
//! clock along each column the current runs down (`stream_fish`), clipped
//! to those cells. Under snow the water is ice and no fish shows.
//!
//! **What they look like** (the effects language, docs/effects.md): whole
//! 2 px blocks on the field's grid in sixteen headings - a dark silhouette
//! (`compose`), a fainter tail fin that swings a block to either side as it
//! beats, and a pale glint on the flank that catches the light now and
//! then (always while it darts). Drawn under the surface at
//! `fish_opacity`, stepped to eighths, over the water's tiles and under
//! everything else (`render::fish::draw_fish`).

use std::collections::BTreeSet;

use crate::ground::{Depth, WaterLayout};
use crate::math::{Color, Rectangle, Vec2};
use crate::pyro::{self, Shape};
use crate::simulation::{Event, Game};
use crate::tuning::{tuning, Tuning};
use crate::{OBSTACLE_GRID_SIZE, PHYSICS_FIXED_DT, Position};

/// The fish's colours: the body (`BLUE_DEEP`, the tank kit's deep water
/// step) and the glint (`BLUE_PALE`) - steps of the Puny palette's blue
/// family (tools/punypalette.py, docs/PALETTE.md). The fin is the body's
/// step, fainter.
pub const FISH: [Color; 2] = [Color::new(0x1D, 0x60, 0x71, 255), Color::new(0x93, 0xEC, 0xE2, 255)];

/// How far a waypoint lies from its cell's centre at most, px: with
/// `REACH_PX`, a fish at a waypoint reaches at most 2 px past its cell,
/// where the water still is.
pub const ROAM_PX: f32 = 6.0;

/// How long the glint's flicker holds one state, s.
const GLINT_SECONDS: f32 = 0.35;

/// The longest step a fish is moved in one go: time that arrives in a
/// lump (a replica's frame) is split into ticks, so a school swims alike
/// however the round's clock reaches it.
const MAX_STEP: f32 = PHYSICS_FIXED_DT;

/// The longest gap in the round's clock a school is stepped across; a
/// longer one (a stall) is taken as this.
const MAX_GAP: f32 = 0.5;

/// The eight neighbours a fish can swim to, orthogonals first.
const STEPS: [(i32, i32); 8] = [(0, -1), (1, 0), (0, 1), (-1, 0), (1, -1), (1, 1), (-1, 1), (-1, -1)];

/// One fish in a lake.
#[derive(Clone, Debug, PartialEq)]
pub struct Fish {
    pub pos: Position,
    /// Where it swims now: within `ROAM_PX` of a deep cell's centre.
    to: Position,
    /// The unit vector it faces, eased toward its way; drawn in sixteen
    /// headings.
    pub heading: Vec2,
    speed: f32,
    /// 1 the moment something scares it, falling to 0 over
    /// `fish_calm_seconds`.
    pub fright: f32,
    /// What it is fleeing while `fright` is up.
    flee: Position,
    /// The tail's beat, in beats.
    tail: f32,
    /// Legs swum so far: salts each hashed choice.
    leg: u32,
    /// This leg is a rest: slowly, to a spot in the same cell.
    idle: bool,
    seed: u32,
    /// Its school (`Shoal::schools`).
    school: usize,
    /// Thrown onto the bank by a sonic hammer's wave (the hammer's "at 11",
    /// docs/sonic-hammer.md): where it lay in the water, the spot on the
    /// bank and the seconds since. `None` in the water.
    pub flop: Option<Flop>,
}

/// A fish on the bank (`Fish::flop`): it hops out of the water to `bank`,
/// flops there `sonic_fish_flop_seconds`, and hops back to `from`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flop {
    pub from: Position,
    pub bank: Position,
    pub age: f32,
}

/// Seconds a fish's hop onto the bank, or back, takes.
const FLOP_HOP_SECONDS: f32 = 0.3;

/// Seconds a fish on the bank holds one heading before it flips over.
const FLOP_FLIP_SECONDS: f32 = 0.15;

impl Flop {
    /// Where the flopping fish is drawn at its age, and how high off the
    /// ground (px): hopping out, lying on the bank flipping a block up and
    /// down, hopping back.
    fn place(&self, seconds: f32) -> (Position, f32) {
        let hop = |from: Position, to: Position, k: f32| (from + (to - from) * k, 10.0 * (std::f32::consts::PI * k).sin());
        if self.age < FLOP_HOP_SECONDS {
            return hop(self.from, self.bank, self.age / FLOP_HOP_SECONDS);
        }
        let back = FLOP_HOP_SECONDS + seconds;
        if self.age >= back {
            return hop(self.bank, self.from, ((self.age - back) / FLOP_HOP_SECONDS).min(1.0));
        }
        let up = ((self.age / FLOP_FLIP_SECONDS) as i32).rem_euclid(2) == 0;
        (self.bank, if up { 2.0 } else { 0.0 })
    }

    /// Whether it is back in the water.
    fn done(&self, seconds: f32) -> bool {
        self.age >= 2.0 * FLOP_HOP_SECONDS + seconds
    }
}

/// A school: a lake and the seed its fish and its wandering hash from.
#[derive(Clone, Debug, PartialEq)]
struct School {
    lake: usize,
    seed: u32,
}

/// Something a fish darts away from, and how near it has to be.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scare {
    pub at: Position,
    pub radius: f32,
}

/// Every fish of a round: its lakes, schools and fish, stepped on the
/// round's clock. Rebuilt when a new round starts.
#[derive(Clone, Debug, Default)]
pub struct Shoal {
    /// The round it was made for (`Game::round_seed`), `None` before the
    /// first.
    round: Option<u64>,
    /// The last `Game::frame` observed: a frame counter going back is a
    /// new round.
    last_frame: u64,
    /// Each lake's deep cells, sorted.
    lakes: Vec<Vec<(i32, i32)>>,
    schools: Vec<School>,
    fish: Vec<Fish>,
    /// The cells the current runs down, sorted: where `stream_fish` swim.
    streams: Vec<(i32, i32)>,
    /// The round's clock at the last step.
    clock: f32,
    /// The round's seed, salting the streams' fish.
    salt: u32,
    /// The fish each sonic wave on the field has thrown onto the bank so
    /// far, by the wave's seed (`SonicWave::seed`): at most
    /// `sonic_fish_throw_max` a wave.
    thrown: Vec<(u32, u32)>,
}

fn centre((col, row): (i32, i32)) -> Position {
    Position::new(col as f32 * OBSTACLE_GRID_SIZE, row as f32 * OBSTACLE_GRID_SIZE)
}

fn cell_of(p: Position) -> (i32, i32) {
    crate::map::world_to_cell(p)
}

fn unit_of(v: Vec2) -> Option<Vec2> {
    let l = v.length();
    (l > 1e-4).then(|| v / l)
}

fn deep(water: &WaterLayout, (col, row): (i32, i32)) -> bool {
    water.depth_of_cell(col, row) == Depth::Deep
}

/// The deep cells a fish in `here` can swim to next, in `STEPS` order: a
/// diagonal only where both cells beside it are deep, so the way there
/// never crosses dry ground.
fn neighbours(water: &WaterLayout, here: (i32, i32)) -> Vec<(i32, i32)> {
    STEPS
        .iter()
        .filter(|&&(dx, dy)| {
            let to = (here.0 + dx, here.1 + dy);
            deep(water, to) && (dx == 0 || dy == 0 || (deep(water, (here.0 + dx, here.1)) && deep(water, (here.0, here.1 + dy))))
        })
        .map(|&(dx, dy)| (here.0 + dx, here.1 + dy))
        .collect()
}

impl Fish {
    /// A hashed choice for this leg.
    fn roll(&self, k: u32) -> f32 {
        pyro::unit(self.seed ^ self.leg.wrapping_mul(0x9E37_79B9), k)
    }

    /// Pick where to swim next: away from what scared it while it is
    /// frightened, else toward `target` (its school's spot) most of the
    /// time, a wander or a rest otherwise.
    fn next_leg(&mut self, water: &WaterLayout, target: Option<(i32, i32)>) {
        self.leg = self.leg.wrapping_add(1);
        let here = cell_of(self.pos);
        let options = neighbours(water, here);
        let here_ok = deep(water, here);
        let cell = if self.fright > 0.0 {
            self.idle = false;
            // The farthest from the threat, here included; the first on
            // a tie.
            let far = |c: (i32, i32)| centre(c).distance_to(self.flee);
            let mut best = if here_ok { Some(here) } else { None };
            for &c in &options {
                if best.is_none_or(|b| far(c) > far(b)) {
                    best = Some(c);
                }
            }
            best
        } else if options.is_empty() || (here_ok && self.roll(1) < 0.2) {
            self.idle = true;
            here_ok.then_some(here)
        } else {
            self.idle = false;
            match target {
                Some(goal) if self.roll(2) < 0.7 => {
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
        let offset = if self.fright > 0.0 && cell == here {
            // Cornered: to the side of the cell away from the threat.
            let away = unit_of(centre(cell) - self.flee).unwrap_or(self.heading);
            Vec2::new((away.x * ROAM_PX).clamp(-ROAM_PX, ROAM_PX), (away.y * ROAM_PX).clamp(-ROAM_PX, ROAM_PX))
        } else {
            Vec2::new((self.roll(4) * 2.0 - 1.0) * ROAM_PX, (self.roll(5) * 2.0 - 1.0) * ROAM_PX)
        };
        self.to = centre(cell) + offset;
    }

    /// Swim `dt` seconds: settle, pick up or lose speed, cover the
    /// distance (onto the next leg if it reaches this one's end), turn
    /// toward the way it goes and beat the tail.
    fn step(&mut self, water: &WaterLayout, target: (i32, i32), dt: f32, t: &Tuning) {
        self.fright = (self.fright - dt / t.fish_calm_seconds.max(0.1)).max(0.0);
        let cruise = t.fish_speed * (0.75 + 0.5 * pyro::unit(self.seed, 7)) * if self.idle { 0.4 } else { 1.0 };
        let want = cruise + (t.fish_dart_speed - cruise).max(0.0) * self.fright;
        self.speed += (want - self.speed) * (dt * 5.0).min(1.0);
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
        if let Some(want) = unit_of(self.to - self.pos) {
            let k = (dt * 6.0).min(1.0);
            self.heading = unit_of(self.heading + (want - self.heading) * k).unwrap_or(want);
        }
        self.tail += dt * t.fish_tail_hz * (1.0 + 2.0 * self.fright);
    }
}

impl Shoal {
    /// The fish of a round on `water`, hashed from its `seed`.
    pub fn new(water: &WaterLayout, seed: u64, t: &Tuning) -> Shoal {
        let salt = (seed ^ (seed >> 32)) as u32;
        let streams: Vec<(i32, i32)> = {
            let mut s: Vec<(i32, i32)> = water.current_grid_cells().collect();
            s.sort();
            s
        };
        let mut shoal = Shoal { round: Some(seed), salt, streams, ..Shoal::default() };
        if water.is_frozen() {
            return shoal;
        }
        // The lakes: the deep cells joined edge to edge, in cell order.
        let deep_cells: BTreeSet<(i32, i32)> = water.deep_grid_cells().collect();
        let mut seen: BTreeSet<(i32, i32)> = BTreeSet::new();
        for &start in &deep_cells {
            if !seen.insert(start) {
                continue;
            }
            let mut lake = vec![start];
            let mut i = 0;
            while i < lake.len() {
                let (c, r) = lake[i];
                for (dc, dr) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                    let n = (c + dc, r + dr);
                    if deep_cells.contains(&n) && seen.insert(n) {
                        lake.push(n);
                    }
                }
                i += 1;
            }
            lake.sort();
            shoal.lakes.push(lake);
        }
        let most = t.fish_per_school.max(0);
        if most == 0 {
            return shoal;
        }
        for (li, lake) in shoal.lakes.iter().enumerate() {
            if lake.len() < t.fish_min_lake_cells.max(1) as usize {
                continue;
            }
            let schools = ((lake.len() as f32 / t.fish_school_cells.max(1.0)).round() as usize).max(1);
            for k in 0..schools {
                let seed = crate::blast::seed_at(centre(lake[0]), salt ^ (k as u32).wrapping_mul(0x51ED_270B));
                let school = shoal.schools.len();
                shoal.schools.push(School { lake: li, seed });
                let fewest = (most + 1) / 2;
                let count = (fewest + (pyro::unit(seed, 1) * (most - fewest + 1) as f32) as i32).min(most);
                let home = lake[((pyro::unit(seed, 2) * lake.len() as f32) as usize).min(lake.len() - 1)];
                let near: Vec<(i32, i32)> = std::iter::once(home).chain(neighbours(water, home)).collect();
                for i in 0..count as u32 {
                    let fseed = crate::blast::seed_at(centre(home), seed ^ i.wrapping_mul(0x2545_F491));
                    let cell = near[((pyro::unit(fseed, 1) * near.len() as f32) as usize).min(near.len() - 1)];
                    let pos = centre(cell)
                        + Vec2::new((pyro::unit(fseed, 2) * 2.0 - 1.0) * ROAM_PX, (pyro::unit(fseed, 3) * 2.0 - 1.0) * ROAM_PX);
                    let (dx, dy) = STEPS[((pyro::unit(fseed, 4) * 8.0) as usize).min(7)];
                    shoal.fish.push(Fish {
                        pos,
                        to: pos,
                        heading: unit_of(Vec2::new(dx as f32, dy as f32)).unwrap_or(Vec2::new(1.0, 0.0)),
                        speed: 0.0,
                        fright: 0.0,
                        flee: pos,
                        tail: pyro::unit(fseed, 5),
                        leg: 0,
                        idle: false,
                        seed: fseed,
                        school,
                        flop: None,
                    });
                }
            }
        }
        shoal
    }

    pub fn fish(&self) -> &[Fish] {
        &self.fish
    }

    /// The spot of its lake a school makes for at `clock`.
    fn target(&self, school: usize, t: &Tuning) -> (i32, i32) {
        let s = &self.schools[school];
        let lake = &self.lakes[s.lake];
        let epoch = (self.clock / t.fish_school_seconds.max(0.5) + pyro::unit(s.seed, 5)).floor() as u32;
        lake[((pyro::unit(s.seed ^ epoch.wrapping_mul(0x27D4_EB2F), 6) * lake.len() as f32) as usize).min(lake.len() - 1)]
    }

    /// Scare the fish within `s.radius` of `s.at`: each turns away at once
    /// unless it is already fleeing and swimming away from it.
    pub fn scare(&mut self, water: &WaterLayout, s: Scare) {
        if s.radius <= 0.0 {
            return;
        }
        for f in &mut self.fish {
            let d = f.pos.distance_to(s.at);
            if d >= s.radius {
                continue;
            }
            let fresh = f.fright < 0.5;
            let toward = f.to.distance_to(s.at) < d;
            f.fright = 1.0;
            f.flee = s.at;
            if fresh || toward {
                f.next_leg(water, None);
            }
        }
    }

    /// Swim every fish on to round time `clock`, in steps of at most a
    /// tick. A clock that went back starts the count again from there.
    pub fn advance(&mut self, water: &WaterLayout, clock: f32, t: &Tuning) {
        let mut gap = (clock - self.clock).clamp(0.0, MAX_GAP);
        self.clock = clock - gap;
        if water.is_frozen() {
            self.clock = clock;
            return;
        }
        while gap > 1e-6 {
            let dt = gap.min(MAX_STEP);
            gap -= dt;
            self.clock += dt;
            let targets: Vec<(i32, i32)> = (0..self.schools.len()).map(|s| self.target(s, t)).collect();
            for f in &mut self.fish {
                // A fish on the bank flops there and swims on once it is
                // back where it was thrown from.
                if let Some(flop) = f.flop.as_mut() {
                    flop.age += dt;
                    if flop.done(t.sonic_fish_flop_seconds) {
                        f.pos = flop.from;
                        f.flop = None;
                    }
                    continue;
                }
                f.step(water, targets[f.school], dt, t);
            }
        }
        self.clock = clock;
    }

    /// One simulation step of `game` seen: rebuild for a new round, scare
    /// the fish near this step's hulls, shots and blasts, and swim them on
    /// to the round's clock. A frame already seen does nothing, so a
    /// frozen round's fish hold still.
    pub fn observe(&mut self, game: &Game) {
        let frame = game.frame();
        let restarted = game.events().iter().any(|e| matches!(e, Event::RoundStarted { .. }));
        if self.round != Some(game.round_seed()) || frame < self.last_frame || (restarted && frame != self.last_frame) {
            let t = tuning();
            *self = Shoal::new(game.water(), game.round_seed(), &t);
            self.clock = game.time;
            self.last_frame = frame;
            return;
        }
        if frame == self.last_frame {
            return;
        }
        self.last_frame = frame;
        let t = tuning();
        let water = game.water();
        if self.fish.is_empty() || water.is_frozen() {
            self.clock = game.time;
            return;
        }
        for s in scares(game, &t) {
            self.scare(water, s);
        }
        let step = (game.time - self.clock).clamp(0.0, MAX_GAP);
        self.throw_onto_banks(&game.sonic_waves, step, water, &t);
        for e in game.events() {
            match *e {
                Event::RodImpact { cell, .. } => {
                    self.throw_from(crate::map::cell_to_world(cell.0, cell.1), t.rod_fish_reach_px, t.rod_fish_throw_max, water, &t);
                }
                // A gravity well's collapse throws the fish in its reach
                // out onto the banks (docs/gravity-well.md "The collapse").
                Event::WellCollapsed { x, y, .. } => self.throw_from(Position::new(x, y), t.well_radius_px, t.well_fish_throw_max, water, &t),
                _ => {}
            }
        }
        self.advance(water, game.time, &t);
    }

    /// The hammer's "at 11" (docs/sonic-hammer.md): a fish a sonic wave's
    /// front passes this step, whose throw along the wave's line
    /// (`sonic_fish_throw_px`) lands on dry ground on the map - never past
    /// its edge, where a lake painted to it runs on - is thrown onto the bank
    /// - at most `sonic_fish_throw_max` a wave, the ones nearest the pivot
    /// first, ties on their order. Hashed nowhere and drawn only: a replica
    /// throws the same fish off the same wave. `step` is the round time
    /// this step covers.
    fn throw_onto_banks(&mut self, waves: &[crate::sonic::SonicWave], step: f32, water: &WaterLayout, t: &Tuning) {
        self.thrown.retain(|(seed, _)| waves.iter().any(|w| w.seed == *seed));
        if t.sonic_fish_throw_max <= 0 || t.sonic_fish_throw_px <= 0.0 {
            return;
        }
        for wave in waves {
            let front = wave.front(t);
            let before = (wave.age - step) * t.sonic_wave_speed;
            let mut hit: Vec<(f32, usize, Position)> = Vec::new();
            for (i, f) in self.fish.iter().enumerate().filter(|(_, f)| f.flop.is_none()) {
                let Some(d) = wave.cone.reaches(f.pos).filter(|&d| d > before && d <= front) else { continue };
                let away = f.pos - wave.cone.origin;
                let Some(dir) = unit_of(away) else { continue };
                let bank = f.pos + dir * t.sonic_fish_throw_px;
                if water.contains(bank) && water.depth_at(bank) == Depth::Dry {
                    hit.push((d, i, bank));
                }
            }
            hit.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let count = match self.thrown.iter_mut().find(|(s, _)| *s == wave.seed) {
                Some(entry) => entry,
                None => {
                    self.thrown.push((wave.seed, 0));
                    self.thrown.last_mut().expect("just pushed")
                }
            };
            for (_, i, bank) in hit {
                if count.1 >= t.sonic_fish_throw_max as u32 {
                    break;
                }
                count.1 += 1;
                let f = &mut self.fish[i];
                f.flop = Some(Flop { from: f.pos, bank, age: 0.0 });
            }
        }
    }

    /// A rod's impact at `at` (docs/rod-from-god.md "At 11"): every fish
    /// within `reach` whose throw straight away from `at`
    /// (`sonic_fish_throw_px`, the hammer's throw) lands on dry ground on
    /// the map is thrown onto the bank - at most `max`, the nearest first,
    /// ties on their order. Drawn only, and a replica throws the same fish
    /// off the same impact.
    pub fn throw_from(&mut self, at: Position, reach: f32, max: i32, water: &WaterLayout, t: &Tuning) {
        if max <= 0 || reach <= 0.0 || t.sonic_fish_throw_px <= 0.0 {
            return;
        }
        let mut hit: Vec<(f32, usize, Position)> = Vec::new();
        for (i, f) in self.fish.iter().enumerate().filter(|(_, f)| f.flop.is_none()) {
            let d = f.pos.distance_to(at);
            if d > reach {
                continue;
            }
            let Some(dir) = unit_of(f.pos - at) else { continue };
            let bank = f.pos + dir * t.sonic_fish_throw_px;
            if water.contains(bank) && water.depth_at(bank) == Depth::Dry {
                hit.push((d, i, bank));
            }
        }
        hit.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, i, bank) in hit.into_iter().take(max as usize) {
            let f = &mut self.fish[i];
            f.flop = Some(Flop { from: f.pos, bank, age: 0.0 });
        }
    }

    /// The fish to draw at the round's clock `time` (the lakes' from their
    /// state, the streams' from the clock), within `cull` where there is
    /// one. Nothing under ice.
    pub fn shapes(&self, water: &WaterLayout, time: f32, t: &Tuning, cull: Option<Rectangle>) -> Vec<Shape> {
        let mut out = Vec::new();
        if water.is_frozen() || t.fish_opacity <= 0.0 {
            return out;
        }
        self.lake_shapes(&mut out, time, t, cull);
        for (pos, beat, seed) in stream_fish(&self.streams, self.salt, time, t) {
            if !crate::view::culled(cull, pos) {
                compose(&mut out, pos, Vec2::new(0.0, 1.0), beat, false, seed, t.fish_opacity);
            }
        }
        out
    }

    /// The lakes' fish (`shapes` without the streams').
    fn lake_shapes(&self, out: &mut Vec<Shape>, time: f32, t: &Tuning, cull: Option<Rectangle>) {
        let window = (time / GLINT_SECONDS).floor() as i32 as u32;
        for f in self.fish.iter().filter(|f| !crate::view::culled(cull, f.pos)) {
            if let Some(flop) = f.flop {
                // Out of the water: drawn whole, flipping over on the bank.
                let (at, lift) = flop.place(t.sonic_fish_flop_seconds);
                let flip = ((flop.age / FLOP_FLIP_SECONDS) as i32).rem_euclid(2) == 0;
                let heading = if flip { Vec2::new(1.0, 0.0) } else { Vec2::new(-1.0, 0.0) };
                compose(out, Position::new(at.x, at.y - lift), heading, flop.age * 4.0, true, f.seed, 1.0);
                continue;
            }
            let glint = f.fright > 0.4 || pyro::unit(f.seed ^ window.wrapping_mul(0x9E37_79B9), 8) < 0.18;
            compose(out, f.pos, f.heading, f.tail, glint, f.seed, t.fish_opacity);
        }
    }
}

/// What a fish darts away from this step: every live hull
/// (`fish_scatter_px`), every shell, bullet and plasma bolt in the air and
/// every hit, ricochet and laser beam (`fish_shot_scatter_px`), every
/// blast, missile burst and wreck (`fish_blast_scatter_px`), and a rod's
/// impact (twice that).
pub fn scares(game: &Game, t: &Tuning) -> Vec<Scare> {
    let mut out = Vec::new();
    let hull = t.fish_scatter_px;
    let shot = t.fish_shot_scatter_px;
    let blast = t.fish_blast_scatter_px;
    // A sonic wave's front, while it runs out: a scare every cell along it.
    for w in &game.sonic_waves {
        if w.spent(t) {
            continue;
        }
        let front = w.front(t);
        let rays = &w.cone.rays;
        let every = ((OBSTACLE_GRID_SIZE / crate::sonic::SONIC_RAY_ARC_PX) as usize).max(1);
        for (i, &reach) in rays.iter().enumerate().step_by(every) {
            let offset = -w.cone.half_angle + 2.0 * w.cone.half_angle * i as f32 / (rays.len().max(2) - 1) as f32;
            let rot = w.cone.facing.rotation().to_radians() + offset;
            let r = front.min(reach);
            out.push(Scare { at: w.cone.origin + Vec2::new(rot.sin(), -rot.cos()) * r, radius: blast });
        }
    }
    for tank in game.world.query::<&crate::tank::Tank>().iter().filter(|tank| !tank.is_dead()) {
        out.push(Scare { at: tank.position, radius: hull });
    }
    for s in game.world.query::<&crate::shell::Shell>().iter() {
        out.push(Scare { at: s.position, radius: shot });
    }
    for b in game.world.query::<&crate::bullet::Bullet>().iter() {
        out.push(Scare { at: b.position, radius: shot });
    }
    for p in game.world.query::<&crate::plasma::Plasma>().iter() {
        out.push(Scare { at: p.position, radius: shot });
    }
    // A gravity well's orbs, as a shot scares them.
    for o in game.orbs() {
        out.push(Scare { at: o.position, radius: shot });
    }
    for e in game.events() {
        match *e {
            Event::Blast { x, y, .. } | Event::MissileBlast { x, y, .. } | Event::Wreck { x, y, .. } => {
                out.push(Scare { at: Position::new(x, y), radius: blast });
            }
            Event::RodImpact { cell, .. } => out.push(Scare { at: crate::map::cell_to_world(cell.0, cell.1), radius: blast * 2.0 }),
            Event::Hit { x, y, .. } | Event::Ricochet { x, y, .. } => out.push(Scare { at: Position::new(x, y), radius: shot }),
            // A drone's burst and a downed one's crash: small as a shot's.
            Event::DroneBurst { x, y, .. } | Event::DroneCrashed { x, y, .. } => out.push(Scare { at: Position::new(x, y), radius: shot }),
            Event::LaserBeam { x0, y0, x1, y1, .. } | Event::RailSlug { x0, y0, x1, y1, .. } => {
                // Along the beam or the slug, a scare every cell, its end
                // included.
                let (a, b) = (Position::new(x0, y0), Position::new(x1, y1));
                let n = (a.distance_to(b) / OBSTACLE_GRID_SIZE).ceil().max(1.0) as u32;
                for i in 1..=n {
                    out.push(Scare { at: a + (b - a) * (i as f32 / n as f32), radius: shot });
                }
            }
            _ => {}
        }
    }
    out
}

/// The fish a stream carries down the map at `time`: along each column of
/// the current's cells (`streams`, sorted), one fish every
/// `fish_stream_seconds` on average, drifting south a little faster than
/// the water (`water_current_speed`), drawn only while its whole body is
/// over a cell the current runs down. Pure in its inputs: a lane is keyed
/// by its column and the round's `salt`. Each is its position, its tail's
/// beat and a seed.
pub fn stream_fish(streams: &[(i32, i32)], salt: u32, time: f32, t: &Tuning) -> Vec<(Position, f32, u32)> {
    let mut out = Vec::new();
    if t.fish_stream_seconds <= 0.0 || t.fish_per_school <= 0 {
        return out;
    }
    let half = OBSTACLE_GRID_SIZE / 2.0;
    let current = |c: (i32, i32)| streams.binary_search(&c).is_ok();
    for &(col, row) in streams {
        let seed = crate::blast::seed_at(centre((col, 0)), salt ^ 0x5157_E41D);
        let speed = (t.water_current_speed * (1.1 + 0.4 * pyro::unit(seed, 1))).max(1.0);
        let gap = (t.fish_stream_seconds * (0.7 + 0.6 * pyro::unit(seed, 2)) * speed).max(OBSTACLE_GRID_SIZE * 2.0);
        let x = col as f32 * OBSTACLE_GRID_SIZE + if pyro::unit(seed, 3) < 0.5 { -2.0 } else { 2.0 };
        let top = row as f32 * OBSTACLE_GRID_SIZE - half;
        // The fish of this lane whose position falls in this cell.
        let phase = (pyro::unit(seed, 4) * gap + time * speed).rem_euclid(gap);
        let first = ((top - phase) / gap).ceil();
        let y = phase + first * gap;
        if y >= top + OBSTACLE_GRID_SIZE {
            continue;
        }
        // Its blocks reach 8 px either way along the stream.
        if !current(cell_of(Position::new(x, y - 8.0))) || !current(cell_of(Position::new(x, y + 8.0))) {
            continue;
        }
        let k = (first as i64 as u32).wrapping_mul(0x9E37_79B9);
        out.push((Position::new(x, y), time * t.fish_tail_hz, seed ^ k));
    }
    out
}

/// The sixteen headings a fish is drawn in, as (cos, sin) clockwise from
/// +x on the y-down field: written out so the picture is the same on
/// every platform.
const HEADINGS: [(f32, f32); 16] = [
    (1.0, 0.0),
    (0.923_879_5, 0.382_683_43),
    (0.707_106_77, 0.707_106_77),
    (0.382_683_43, 0.923_879_5),
    (0.0, 1.0),
    (-0.382_683_43, 0.923_879_5),
    (-0.707_106_77, 0.707_106_77),
    (-0.923_879_5, 0.382_683_43),
    (-1.0, 0.0),
    (-0.923_879_5, -0.382_683_43),
    (-0.707_106_77, -0.707_106_77),
    (-0.382_683_43, -0.923_879_5),
    (0.0, -1.0),
    (0.382_683_43, -0.923_879_5),
    (0.707_106_77, -0.707_106_77),
    (0.923_879_5, -0.382_683_43),
];

/// How far a fish's blocks reach from its position at most, px.
pub const REACH_PX: f32 = 12.0;

/// The nearest of the sixteen headings to `v` (`HEADINGS`); east for a
/// zero vector.
pub fn heading_index(v: Vec2) -> usize {
    let mut best = 0;
    let mut most = f32::MIN;
    for (i, &(c, s)) in HEADINGS.iter().enumerate() {
        let d = v.x * c + v.y * s;
        if d > most + 1e-6 {
            most = d;
            best = i;
        }
    }
    best
}

/// One fish at `pos` facing `heading`: a body rasterised onto the field's
/// 2 px blocks from an ellipse along the nearest of sixteen headings,
/// anchored to the grid vertex nearest `pos` so it moves a whole block at
/// a time and keeps its shape; behind it a tail fin, fainter, that swings
/// a block to either side as `tail` beats; a pale glint on its flank near
/// the head when `glint`. A fish hashed small (from `seed`) is two blocks
/// shorter. Everything at `opacity` stepped to eighths, in the `FISH`
/// steps; nothing past `REACH_PX`.
pub fn compose(out: &mut Vec<Shape>, pos: Position, heading: Vec2, tail: f32, glint: bool, seed: u32, opacity: f32) {
    let (c, s) = HEADINGS[heading_index(heading)];
    let (cx, cy) = (pyro::snap(pos.x), pyro::snap(pos.y));
    let small = pyro::unit(seed, 9) < 0.35;
    let (half_len, half_w) = if small { (4.4, 2.0) } else { (6.2, 2.4) };
    // The tail's swing 4 px behind the body, a block either way.
    let swing = [0.0, 2.0, 0.0, -2.0][((tail * 4.0).floor() as i64).rem_euclid(4) as usize];
    let body = pyro::alpha(FISH[0], opacity);
    let fin = pyro::alpha(FISH[0], opacity * 0.625);
    let shine = pyro::alpha(FISH[1], opacity);
    let n = (REACH_PX / pyro::BLOCK) as i32;
    let first = out.len();
    // The glint's block: the body's foremost on its right flank.
    let mut shine_at: Option<(usize, f32)> = None;
    for by in -n..n {
        for bx in -n..n {
            // The block's centre about the anchor, in the fish's frame:
            // `u` ahead, `v` to its right.
            let (x, y) = (bx as f32 * 2.0 + 1.0, by as f32 * 2.0 + 1.0);
            let (u, v) = (x * c + y * s, -x * s + y * c);
            let color = if (u / half_len).powi(2) + (v / half_w).powi(2) <= 1.0 {
                if v > 0.0 && shine_at.is_none_or(|(_, best)| u + 0.5 * v > best + 1e-4) {
                    shine_at = Some((out.len(), u + 0.5 * v));
                }
                body
            } else {
                // From a little inside the body's tip, so the fin joins
                // it whatever the heading.
                let behind = -(u + half_len);
                let back = behind.max(0.0);
                if (-1.5..=4.5).contains(&behind) && (v - swing * back / 4.0).abs() <= 1.2 + 0.35 * back {
                    fin
                } else {
                    continue;
                }
            };
            out.push(Shape::Mark { pos: Position::new((cx + bx * 2) as f32, (cy + by * 2) as f32), size: 2, color });
        }
    }
    if let (true, Some((i, _))) = (glint, shine_at) {
        if let Shape::Mark { color, .. } = &mut out[i] {
            *color = shine;
        }
    }
    debug_assert!(out.len() > first);
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 640.0;
    const H: f32 = 480.0;

    fn world(cells: &[(i32, i32)]) -> Vec<Position> {
        cells.iter().map(|&c| centre(c)).collect()
    }

    /// A pond: an L of painted water whose middle is deep, and a
    /// north/south stream feeding it.
    fn pond() -> WaterLayout {
        let mut cells = Vec::new();
        for c in 2..=11 {
            for r in 2..=7 {
                cells.push((c, r));
            }
        }
        for c in 2..=6 {
            for r in 8..=12 {
                cells.push((c, r));
            }
        }
        for r in 0..2 {
            cells.push((14, r));
        }
        for r in 0..=13 {
            cells.push((16, r));
        }
        WaterLayout::build(W, H, &[], &world(&cells))
    }

    fn quick() -> Tuning {
        Tuning { fish_per_school: 6, fish_school_cells: 12.0, ..Tuning::DEFAULT }
    }

    /// Swim `shoal` on from round time `from` to `to` a tick at a time,
    /// as a round's steps would.
    fn swim(shoal: &mut Shoal, water: &WaterLayout, from: f32, to: f32, t: &Tuning) {
        let mut clock = from;
        while clock < to - 1e-4 {
            clock = (clock + PHYSICS_FIXED_DT).min(to);
            shoal.advance(water, clock, t);
        }
    }

    /// Every block of every fish lies on painted water and within
    /// `REACH_PX` of a deep cell - short of the middle of a shore cell,
    /// where a lake's shoreline is drawn - so over water in the picture.
    fn assert_in_water(water: &WaterLayout, shapes: &[Shape]) {
        for s in shapes {
            let Shape::Mark { pos, size, .. } = *s else { panic!("a fish is marks: {s:?}") };
            assert_eq!(size, 2);
            for (cx, cy) in [(pos.x, pos.y), (pos.x + 1.99, pos.y + 1.99)] {
                let p = Position::new(cx, cy);
                assert_ne!(water.depth_at(p), Depth::Dry, "a block on dry ground at {p:?}");
            }
            let c = cell_of(pos + Vec2::new(1.0, 1.0));
            let near = (-1..=1).any(|dc| {
                (-1..=1).any(|dr| {
                    let d = (c.0 + dc, c.1 + dr);
                    deep(water, d) && {
                        let o = centre(d);
                        (pos.x + 1.0 - o.x).abs() <= 16.0 + REACH_PX && (pos.y + 1.0 - o.y).abs() <= 16.0 + REACH_PX
                    }
                })
            });
            assert!(near, "a block at {pos:?} is not over a lake's open water");
        }
    }

    #[test]
    fn a_pond_holds_schools_and_a_dry_map_none() {
        let t = quick();
        let shoal = Shoal::new(&pond(), 7, &t);
        assert!(!shoal.lakes.is_empty());
        assert!(shoal.fish.len() >= 3, "{} fish", shoal.fish.len());
        assert!(shoal.schools.len() >= 2, "a big pond holds more than one school");
        let dry = WaterLayout::build(W, H, &[], &[]);
        assert!(Shoal::new(&dry, 7, &t).fish.is_empty());
        let none = Tuning { fish_per_school: 0, ..t };
        assert!(Shoal::new(&pond(), 7, &none).fish.is_empty());
    }

    #[test]
    fn fish_stay_over_open_water_on_the_grid() {
        let t = quick();
        let water = pond();
        let mut shoal = Shoal::new(&water, 11, &t);
        for i in 1..=3600 {
            let clock = i as f32 * PHYSICS_FIXED_DT;
            // A hull driving round the pond's edge, and a blast now and then.
            let a = clock * 0.7;
            let hull = Position::new(220.0 + 200.0 * a.cos(), 160.0 + 130.0 * a.sin());
            shoal.scare(&water, Scare { at: hull, radius: t.fish_scatter_px });
            if i % 400 == 0 {
                shoal.scare(&water, Scare { at: centre((4 + i / 400 % 5, 5)), radius: t.fish_blast_scatter_px });
            }
            shoal.advance(&water, clock, &t);
            for s in &shoal.shapes(&water, clock, &t, None) {
                let Shape::Mark { pos, .. } = *s else { unreachable!() };
                assert!(pos.x % 2.0 == 0.0 && pos.y % 2.0 == 0.0, "off the 2 px grid: {pos:?}");
            }
            let mut lake = Vec::new();
            shoal.lake_shapes(&mut lake, clock, &t, None);
            assert_in_water(&water, &lake);
        }
    }

    #[test]
    fn the_same_round_swims_the_same() {
        let t = quick();
        let water = pond();
        let run = || {
            let mut shoal = Shoal::new(&water, 99, &t);
            for i in 1..=600 {
                if i == 200 {
                    shoal.scare(&water, Scare { at: centre((6, 4)), radius: t.fish_blast_scatter_px });
                }
                shoal.advance(&water, i as f32 * PHYSICS_FIXED_DT, &t);
            }
            shoal.shapes(&water, 10.0, &t, None)
        };
        assert_eq!(run(), run());
        // Time in one lump swims as the same time in ticks.
        let mut lump = Shoal::new(&water, 99, &t);
        let mut ticks = lump.clone();
        lump.advance(&water, 0.25, &t);
        for i in 1..=15 {
            ticks.advance(&water, i as f32 * PHYSICS_FIXED_DT, &t);
        }
        for (a, b) in lump.fish.iter().zip(&ticks.fish) {
            assert!(a.pos.distance_to(b.pos) < 0.01, "{:?} vs {:?}", a.pos, b.pos);
        }
        // Another seed, other fish.
        assert_ne!(Shoal::new(&water, 99, &t).fish, Shoal::new(&water, 100, &t).fish);
    }

    /// A sonic wave scares fish all along its front as it runs out, and
    /// none once it is spent.
    #[test]
    fn a_wave_scares_the_fish_it_passes() {
        let t = quick();
        let origin = Position::new(200.0, 128.0);
        let cone = crate::sonic::SonicCone::cast(
            origin,
            crate::tank::Dir::Right,
            t.sonic_reach_px,
            t.sonic_half_angle_deg.to_radians(),
            (W, H),
            |_| crate::sonic::Block::Open,
            |_| crate::sonic::Floor::Water,
        );
        let mut game = Game::default();
        let mut wave = crate::sonic::SonicWave::new(cone, crate::shell::Owner::Player(0));
        wave.age = 80.0 / t.sonic_wave_speed;
        game.sonic_waves.push(wave.clone());
        let along: Vec<Scare> = scares(&game, &t).into_iter().filter(|s| (s.at.distance_to(origin) - 80.0).abs() < 0.5).collect();
        assert!(along.len() >= 3, "scares along the front: {along:?}");
        assert!(along.iter().all(|s| s.at.x > origin.x), "ahead of the pivot");
        wave.age = 10.0;
        game.sonic_waves = vec![wave];
        assert!(scares(&game, &t).is_empty(), "a spent wave scares nothing");
    }

    /// The hammer's "at 11": a fish the wave's front passes, whose throw
    /// along the wave's line lands on dry ground, is thrown onto the bank,
    /// flops there and hops back to where it was; one whose throw lands in
    /// the water stays in it.
    #[test]
    fn a_shout_at_the_shore_throws_a_fish_onto_the_bank_and_back() {
        let t = quick();
        let water = pond();
        let mut shoal = Shoal::new(&water, 5, &t);
        assert!(shoal.fish.len() >= 2);
        // One fish by the east shore, one in the middle of the lake.
        shoal.fish[0].pos = Position::new(340.0, 128.0);
        shoal.fish[1].pos = Position::new(200.0, 160.0);
        let cone = crate::sonic::SonicCone::cast(
            Position::new(200.0, 128.0),
            crate::tank::Dir::Right,
            t.sonic_reach_px,
            t.sonic_half_angle_deg.to_radians(),
            (W, H),
            |_| crate::sonic::Block::Open,
            |_| crate::sonic::Floor::Water,
        );
        let mut wave = crate::sonic::SonicWave::new(cone, crate::shell::Owner::Player(0));
        let mut thrown = false;
        for _ in 0..30 {
            wave.age += PHYSICS_FIXED_DT;
            shoal.throw_onto_banks(std::slice::from_ref(&wave), PHYSICS_FIXED_DT, &water, &t);
            thrown |= shoal.fish[0].flop.is_some();
        }
        assert!(thrown, "the fish by the shore is thrown onto the bank");
        assert!(shoal.fish[1].flop.is_none(), "the one in the middle stays in the water");
        let flop = shoal.fish[0].flop.expect("on the bank");
        assert_eq!(water.depth_at(flop.bank), Depth::Dry);
        // It flops, then hops back.
        let clock = shoal.clock;
        swim(&mut shoal, &water, clock, clock + t.sonic_fish_flop_seconds + 1.0, &t);
        assert!(shoal.fish[0].flop.is_none(), "back in the water");
        assert!(water.depth_at(shoal.fish[0].pos).is_wet(), "{:?}", shoal.fish[0].pos);
    }

    /// A gravity well's orb scares the fish it passes as a shot does
    /// (docs/gravity-well.md).
    #[test]
    fn an_orb_scares_the_fish() {
        let t = quick();
        let mut game = Game::default();
        let at = Position::new(200.0, 128.0);
        game.orbs.push(crate::well::Orb::launch(at, Vec2::new(1.0, 0.0), crate::shell::Owner::Player(0), &t));
        assert!(scares(&game, &t).iter().any(|s| s.at == at && s.radius == t.fish_shot_scatter_px));
    }

    /// A rod's impact throws the fish within its reach whose throw away
    /// from it lands on dry ground onto the bank - at most its count,
    /// nearest first - and leaves the rest in the water
    /// (docs/rod-from-god.md).
    #[test]
    fn a_rods_impact_throws_the_fish_by_the_shore_onto_the_bank() {
        let t = quick();
        let water = pond();
        let mut shoal = Shoal::new(&water, 5, &t);
        assert!(shoal.fish.len() >= 3);
        // By the east shore, by the west shore, and in the middle.
        shoal.fish[0].pos = Position::new(340.0, 128.0);
        shoal.fish[1].pos = Position::new(76.0, 128.0);
        shoal.fish[2].pos = Position::new(240.0, 150.0);
        for f in shoal.fish.iter_mut().skip(3) {
            f.pos = Position::new(240.0, 128.0);
        }
        shoal.throw_from(Position::new(240.0, 128.0), 200.0, 8, &water, &t);
        for i in [0, 1] {
            let flop = shoal.fish[i].flop.unwrap_or_else(|| panic!("fish {i} by the shore is thrown"));
            assert_eq!(water.depth_at(flop.bank), Depth::Dry);
        }
        assert!(shoal.fish[2].flop.is_none(), "the one in the middle stays in the water");
        let mut capped = Shoal::new(&water, 5, &t);
        capped.fish[0].pos = Position::new(340.0, 128.0);
        capped.fish[1].pos = Position::new(76.0, 128.0);
        capped.throw_from(Position::new(240.0, 128.0), 200.0, 1, &water, &t);
        assert_eq!(capped.fish.iter().filter(|f| f.flop.is_some()).count(), 1, "at most its count");
    }

    /// A lake painted to the map's edge runs on past it, so a fish the
    /// wave would throw past the edge stays in the water: no fish lands off
    /// the map.
    #[test]
    fn no_fish_is_thrown_off_the_map() {
        let t = quick();
        let mut cells = Vec::new();
        for c in 4..=12 {
            for r in 9..=14 {
                cells.push((c, r));
            }
        }
        let water = WaterLayout::build(W, H, &[], &world(&cells));
        let mut shoal = Shoal::new(&water, 5, &t);
        assert!(!shoal.fish.is_empty());
        shoal.fish[0].pos = Position::new(256.0, H - 12.0);
        let cone = crate::sonic::SonicCone::cast(
            Position::new(256.0, H - 140.0),
            crate::tank::Dir::Down,
            t.sonic_reach_px,
            t.sonic_half_angle_deg.to_radians(),
            (W, H),
            |_| crate::sonic::Block::Open,
            |_| crate::sonic::Floor::Water,
        );
        let mut wave = crate::sonic::SonicWave::new(cone, crate::shell::Owner::Player(0));
        for _ in 0..30 {
            wave.age += PHYSICS_FIXED_DT;
            shoal.throw_onto_banks(std::slice::from_ref(&wave), PHYSICS_FIXED_DT, &water, &t);
        }
        assert!(shoal.fish[0].flop.is_none(), "the fish by the edge stays in the water");
    }

    #[test]
    fn a_scare_sends_fish_darting_away() {
        let t = quick();
        let water = pond();
        let mut shoal = Shoal::new(&water, 5, &t);
        swim(&mut shoal, &water, 0.0, 2.0, &t);
        let f = shoal.fish[0].clone();
        // Just beside it, on its east side.
        let at = f.pos + Vec2::new(12.0, 0.0);
        shoal.scare(&water, Scare { at, radius: t.fish_scatter_px });
        assert_eq!(shoal.fish[0].fright, 1.0);
        assert!(shoal.fish[0].to.distance_to(at) >= f.pos.distance_to(at), "it turns away");
        let before = shoal.fish[0].pos.distance_to(at);
        swim(&mut shoal, &water, 2.0, 2.6, &t);
        let after = shoal.fish[0].pos.distance_to(at);
        assert!(after > before + 10.0, "it darts off: {before} -> {after}");
        assert!(shoal.fish[0].speed > t.fish_speed * 1.5, "fast: {}", shoal.fish[0].speed);
        // Out of reach, nothing happens.
        let calm = shoal.fish.iter().position(|g| g.pos.distance_to(at) > t.fish_scatter_px + 40.0 && g.fright == 0.0);
        if let Some(i) = calm {
            let before = shoal.fish[i].clone();
            shoal.scare(&water, Scare { at, radius: t.fish_scatter_px });
            assert_eq!(shoal.fish[i], before);
        }
        // And it settles: the fright runs out over the calm time.
        swim(&mut shoal, &water, 2.6, 2.6 + t.fish_calm_seconds + 0.1, &t);
        assert_eq!(shoal.fish[0].fright, 0.0);
    }

    #[test]
    fn ice_holds_no_fish() {
        let t = quick();
        let mut water = pond();
        let mut shoal = Shoal::new(&water, 3, &t);
        assert!(!shoal.shapes(&water, 1.0, &t, None).is_empty());
        water.freeze();
        assert!(shoal.shapes(&water, 1.0, &t, None).is_empty());
        let before = shoal.fish.clone();
        shoal.advance(&water, 4.0, &t);
        assert_eq!(shoal.fish, before, "frozen fish do not swim");
        assert!(Shoal::new(&water, 3, &t).fish.is_empty());
    }

    #[test]
    fn a_stream_carries_a_fish_down_its_current() {
        let t = Tuning { fish_stream_seconds: 4.0, ..quick() };
        let water = pond();
        let shoal = Shoal::new(&water, 1, &t);
        assert!(!shoal.streams.is_empty());
        let mut seen = 0;
        let mut last: Option<f32> = None;
        for i in 0..400 {
            let time = i as f32 * 0.05;
            let fish = stream_fish(&shoal.streams, shoal.salt, time, &t);
            for &(pos, _, _) in &fish {
                assert!(water.pushes_south(pos), "{pos:?} is in the current");
                assert!(water.pushes_south(pos - Vec2::new(0.0, 8.0)) && water.pushes_south(pos + Vec2::new(0.0, 8.0)));
            }
            if let Some(&(pos, ..)) = fish.iter().find(|(p, ..)| p.x > 14.5 * 32.0) {
                if let Some(y) = last {
                    if pos.y > y {
                        seen += 1;
                    }
                }
                last = Some(pos.y);
            }
        }
        assert!(seen > 20, "fish drift south along the long stream: {seen}");
        // The short stream at column 14 is too short for a whole fish
        // only at its ends; nothing shows off the current.
        let none = Tuning { fish_stream_seconds: 0.0, ..t };
        assert!(stream_fish(&shoal.streams, shoal.salt, 3.0, &none).is_empty());
    }

    #[test]
    fn a_fish_is_on_the_grid_in_its_ramp() {
        let at = Position::new(101.3, 57.9);
        let anchor = Position::new(102.0, 58.0);
        for k in 0..16u32 {
            let (c, s) = HEADINGS[k as usize];
            let heading = Vec2::new(c, s);
            assert_eq!(heading_index(heading), k as usize);
            for (seed, glint) in [(k, false), (k + 100, true)] {
                let mut out = Vec::new();
                compose(&mut out, at, heading, k as f32 * 0.3, glint, seed, 0.75);
                assert!(out.len() >= 8, "a fish of {} blocks", out.len());
                for sh in &out {
                    let Shape::Mark { pos, size, color } = *sh else { panic!("{sh:?}") };
                    assert_eq!(size, 2);
                    assert!(pos.x % 2.0 == 0.0 && pos.y % 2.0 == 0.0, "off the grid: {pos:?}");
                    assert!(FISH.iter().any(|f| (f.r, f.g, f.b) == (color.r, color.g, color.b)), "{color:?}");
                    assert!([pyro::alpha(FISH[0], 0.75).a, pyro::alpha(FISH[0], 0.75 * 0.625).a].contains(&color.a), "{}", color.a);
                    assert!(Position::new(pos.x + 1.0, pos.y + 1.0).distance_to(anchor) <= REACH_PX, "{pos:?} too far");
                }
                let mut again = Vec::new();
                compose(&mut again, at, heading, k as f32 * 0.3, glint, seed, 0.75);
                assert_eq!(out, again);
                let shines = out.iter().filter(|sh| matches!(sh, Shape::Mark { color, .. } if color.r == FISH[1].r)).count();
                assert_eq!(shines > 0, glint, "a glint only when it glints");
            }
        }
        assert_eq!(heading_index(Vec2::new(0.0, 0.0)), 0);
        assert_eq!(heading_index(Vec2::new(0.1, -1.0)), 12);
    }

    /// Every heading and tail beat on lake water, to look at:
    /// `cargo test --lib fish::tests::preview -- --ignored` writes
    /// target/fish-preview.png.
    #[test]
    #[ignore]
    #[cfg(feature = "render")] // write_png is raylib's PNG encoder
    fn preview() {
        use crate::canvas::{Canvas, CpuCanvas};
        let mut c = CpuCanvas::blank(16 * 32, 4 * 32);
        c.fill_rect(0, 0, 16 * 32, 4 * 32, Color::new(0x04, 0xA0, 0xB4, 255));
        for (k, &(hc, hs)) in HEADINGS.iter().enumerate() {
            for row in 0..4u32 {
                let mut out = Vec::new();
                let at = Position::new(k as f32 * 32.0 + 16.0, row as f32 * 32.0 + 16.0);
                compose(&mut out, at, Vec2::new(hc, hs), row as f32 * 0.25, row == 3, row * 7 + 1, 0.75);
                pyro::draw(&mut c, &out);
            }
        }
        c.write_png(std::path::Path::new("target/fish-preview.png"), 4).unwrap();
    }
}
