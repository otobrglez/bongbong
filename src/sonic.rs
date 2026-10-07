//! The sonic hammer (docs/sonic-hammer.md), its headless half: the cone a
//! blast is cast as, the wave that runs out along it, the numbers a shove
//! and a skid come to (which the AI predicts with), where a thrown drum
//! lands, and the drawing - the wave's arcs, its dust and shards, an
//! enemy's wind-up, a tree swaying as the front passes and the dish's
//! cell. `simulation/sonic.rs` is the world half.
//!
//! **The cone is cast once, at the press**, as a fan of rays walked cell by
//! cell over the map's cells (centres on multiples of `OBSTACLE_GRID_SIZE`):
//! a ray stops on entering the first cell that blocks sound
//! (`Material::blocks_sound`) or at the field's edge. What the rays reach
//! is what the wave reaches and what the arcs are drawn over, so the
//! picture is the rule. Nothing here draws RNG; every cosmetic choice
//! hashes the wave's origin.

use crate::math::{Color, Vec2};
use crate::pyro::{self, Puff, Shape};
use crate::shell::Owner;
use crate::tank::{Dir, Tank};
use crate::tuning::Tuning;
use crate::{OBSTACLE_GRID_SIZE, Position};

/// Px of arc, at the cone's full reach, between two of its rays: fine
/// enough that no cell between two rays goes unseen.
pub const SONIC_RAY_ARC_PX: f32 = 4.0;

/// Px between two points the AI samples along a predicted slide.
pub const SONIC_TROUBLE_STEP_PX: f32 = 8.0;

/// No arc is drawn tighter than this round the pivot (px): inside it is
/// the dish.
const MIN_ARC_RADIUS: f32 = 12.0;

/// How long a tree's crown leans after the front passes it (seconds).
const TREE_SWAY_SECONDS: f32 = 0.25;

/// The stone ramp the wave is drawn in, darkest first.
pub const STONE: [Color; 4] = [
    Color::new(0x9E, 0x9E, 0x96, 255),
    Color::new(0xC1, 0xC1, 0xC1, 255),
    Color::new(0xDA, 0xDA, 0xDA, 255),
    Color::new(0xF0, 0xF0, 0xF0, 255),
];

/// A shard of glass, and the spray off water.
const BLUE_PALE: Color = Color::new(0x93, 0xEC, 0xE2, 255);

/// What lies under a cell the wave crosses, for the drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Floor {
    Dry,
    Water,
    Lava,
}

/// How a cell answers a ray (`SonicCone::cast`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    /// Sound passes.
    Open,
    /// A wall, a tower, a cone, a door: the ray stops at its face.
    Wall,
    /// A pane: the ray stops at it and it shatters.
    Glass,
}

/// One cell some ray of a cone entered: where, how far from the pivot the
/// first ray entered it, and what lies under it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConeCell {
    pub cell: (i32, i32),
    pub at: f32,
    pub floor: Floor,
}

/// A blast's cone, cast once at the press.
#[derive(Clone, Debug, PartialEq)]
pub struct SonicCone {
    /// The pivot it is judged and drawn from.
    pub origin: Position,
    pub facing: Dir,
    /// The half angle either side of the facing (radians).
    pub half_angle: f32,
    /// The reach it was cast to (px).
    pub reach: f32,
    /// Each ray's reach, from `-half_angle` to `+half_angle` in even steps.
    pub rays: Vec<f32>,
    /// Every cell a ray entered (the pivot's own included), by entry
    /// distance and then cell.
    pub cells: Vec<ConeCell>,
    /// The panes rays stopped at, by entry distance and then cell: what
    /// shatters as the front reaches them.
    pub panes: Vec<ConeCell>,
}

/// The rotation (degrees, 0 up, clockwise) of `facing`.
fn facing_rad(facing: Dir) -> f32 {
    facing.rotation().to_radians()
}

/// The unit vector of rotation `rad` (0 up, clockwise on the y-down field).
fn along(rad: f32) -> Vec2 {
    Vec2::new(rad.sin(), -rad.cos())
}

/// `v` at unit length, or straight up for a zero vector.
fn unit(v: Vec2) -> Vec2 {
    let l = v.length();
    if l > 1e-4 { v / l } else { Vec2::new(0.0, -1.0) }
}

/// The map cell (`map::world_to_cell`) the point lies in.
fn cell_of(p: Position) -> (i32, i32) {
    crate::map::world_to_cell(p)
}

impl SonicCone {
    /// Cast a cone from `origin` along `facing`: `reach` px out, `half_angle`
    /// radians either side, over a field `field` px across, each ray
    /// stopping where `block` says a cell stops sound and at the field's
    /// edge. `floor` names what lies under a cell, for the drawing.
    pub fn cast(
        origin: Position,
        facing: Dir,
        reach: f32,
        half_angle: f32,
        field: (f32, f32),
        block: impl Fn((i32, i32)) -> Block,
        floor: impl Fn((i32, i32)) -> Floor,
    ) -> SonicCone {
        let span = 2.0 * half_angle;
        let n = ((span * reach / SONIC_RAY_ARC_PX).ceil() as usize).max(2) + 1;
        let base = facing_rad(facing);
        let mut entered: std::collections::BTreeMap<(i32, i32), f32> = std::collections::BTreeMap::new();
        let mut panes: std::collections::BTreeMap<(i32, i32), f32> = std::collections::BTreeMap::new();
        let mut rays = Vec::with_capacity(n);
        for i in 0..n {
            let a = base - half_angle + span * i as f32 / (n - 1) as f32;
            rays.push(walk(origin, along(a), reach, field, &block, &mut entered, &mut panes));
        }
        let mut cells: Vec<ConeCell> = entered.into_iter().map(|(cell, at)| ConeCell { cell, at, floor: floor(cell) }).collect();
        cells.sort_by(|a, b| a.at.total_cmp(&b.at).then(a.cell.cmp(&b.cell)));
        let mut panes: Vec<ConeCell> = panes.into_iter().map(|(cell, at)| ConeCell { cell, at, floor: Floor::Dry }).collect();
        panes.sort_by(|a, b| a.at.total_cmp(&b.at).then(a.cell.cmp(&b.cell)));
        SonicCone { origin, facing, half_angle, reach, rays, cells, panes }
    }

    /// The longest ray's reach: past it the wave has reached all it will.
    pub fn longest(&self) -> f32 {
        self.rays.iter().copied().fold(0.0, f32::max)
    }

    /// The reach along the bearing `offset` radians off the facing: the
    /// lesser of the two rays either side of it. `None` outside the half
    /// angle.
    pub fn reach_at(&self, offset: f32) -> Option<f32> {
        if offset.abs() > self.half_angle + 1e-4 || self.rays.is_empty() {
            return None;
        }
        let last = self.rays.len() - 1;
        let k = ((offset + self.half_angle) / (2.0 * self.half_angle)).clamp(0.0, 1.0) * last as f32;
        let (lo, hi) = (k.floor() as usize, (k.ceil() as usize).min(last));
        Some(self.rays[lo].min(self.rays[hi]))
    }

    /// The bearing of `p` off the facing (radians, wrapped to -pi..=pi).
    pub fn offset_of(&self, p: Position) -> f32 {
        let d = p - self.origin;
        let rot = d.x.atan2(-d.y);
        let pi = std::f32::consts::PI;
        (rot - facing_rad(self.facing) + pi).rem_euclid(2.0 * pi) - pi
    }

    /// How far `p` is from the pivot if the cone reaches it, `None` if it
    /// does not: within the half angle and no further than the rays either
    /// side of its bearing.
    pub fn reaches(&self, p: Position) -> Option<f32> {
        let d = self.origin.distance_to(p);
        if d > self.reach + 1e-3 {
            return None;
        }
        if d < 1.0 {
            return Some(d);
        }
        let reach = self.reach_at(self.offset_of(p))?;
        (d <= reach + 1e-3).then_some(d)
    }

    /// The distance a ray first entered `cell` at, if any did.
    pub fn entered(&self, cell: (i32, i32)) -> Option<f32> {
        self.cells.iter().find(|c| c.cell == cell).map(|c| c.at)
    }

    /// The nearest of `points` the cone reaches, and how far it is - what a
    /// hull is reached at (its centre and its box's corners).
    pub fn nearest_reached(&self, points: &[Position]) -> Option<f32> {
        points.iter().filter_map(|&p| self.reaches(p)).min_by(|a, b| a.total_cmp(b))
    }
}

/// One ray of a cast: from `origin` along the unit `dir` for up to `reach`
/// px, cell by cell (Amanatides-Woo on the map's cells, which span
/// `[32c - 16, 32c + 16)`), noting the distance each cell is entered at.
/// Stops at the field's edge and on entering a cell that blocks, whose
/// entry is its reach (a pane is noted as one). Returns the ray's reach.
fn walk(
    origin: Position,
    dir: Vec2,
    reach: f32,
    (width, height): (f32, f32),
    block: &impl Fn((i32, i32)) -> Block,
    entered: &mut std::collections::BTreeMap<(i32, i32), f32>,
    panes: &mut std::collections::BTreeMap<(i32, i32), f32>,
) -> f32 {
    let g = OBSTACLE_GRID_SIZE;
    // How far the ray runs before it leaves the field.
    let exit = |o: f32, d: f32, hi: f32| {
        if d > 1e-6 {
            (hi - o) / d
        } else if d < -1e-6 {
            -o / d
        } else {
            f32::INFINITY
        }
    };
    let cap = reach.min(exit(origin.x, dir.x, width)).min(exit(origin.y, dir.y, height)).max(0.0);
    let (ux, uy) = ((origin.x + g * 0.5) / g, (origin.y + g * 0.5) / g);
    let (mut cx, mut cy) = (ux.floor() as i32, uy.floor() as i32);
    let note = |entered: &mut std::collections::BTreeMap<(i32, i32), f32>, cell: (i32, i32), at: f32| {
        let e = entered.entry(cell).or_insert(at);
        if at < *e {
            *e = at;
        }
    };
    match block((cx, cy)) {
        Block::Open => note(entered, (cx, cy), 0.0),
        // The pivot inside a solid cell: nothing leaves it.
        Block::Wall | Block::Glass => return 0.0,
    }
    let (step_x, step_y) = (if dir.x >= 0.0 { 1 } else { -1 }, if dir.y >= 0.0 { 1 } else { -1 });
    let next = |u: f32, c: i32, d: f32| {
        if d > 1e-6 {
            ((c + 1) as f32 - u) * g / d
        } else if d < -1e-6 {
            (u - c as f32) * g / -d
        } else {
            f32::INFINITY
        }
    };
    let mut t_x = next(ux, cx, dir.x);
    let mut t_y = next(uy, cy, dir.y);
    let dt_x = if dir.x.abs() > 1e-6 { g / dir.x.abs() } else { f32::INFINITY };
    let dt_y = if dir.y.abs() > 1e-6 { g / dir.y.abs() } else { f32::INFINITY };
    loop {
        let t = if t_x < t_y {
            cx += step_x;
            let t = t_x;
            t_x += dt_x;
            t
        } else {
            cy += step_y;
            let t = t_y;
            t_y += dt_y;
            t
        };
        if t > cap {
            return cap;
        }
        match block((cx, cy)) {
            Block::Open => note(entered, (cx, cy), t),
            Block::Wall => return t,
            Block::Glass => {
                note(panes, (cx, cy), t);
                return t;
            }
        }
    }
}

/// A blast's wave: its cone, who fired it and how far its front has run.
/// The simulation strikes what the front passes (`Game::tick_sonic_waves`);
/// a replica's copy only draws. A wave's picture is pure in it and its age.
#[derive(Clone, Debug, PartialEq)]
pub struct SonicWave {
    pub cone: SonicCone,
    /// Who fired it: never reached by it.
    pub owner: Owner,
    /// Seconds since the press.
    pub age: f32,
    /// The front's radius already swept - by the rules on the round that
    /// simulates it, by the cosmetic sweep on a replica.
    pub swept: f32,
    /// The hulls it has struck, by owner slot: each once.
    pub struck: Vec<usize>,
    /// Hashed from the pivot: the drawing's variety.
    pub seed: u32,
}

impl SonicWave {
    /// A fresh wave on `cone`, fired by `owner`.
    pub fn new(cone: SonicCone, owner: Owner) -> SonicWave {
        let seed = crate::blast::seed_at(cone.origin, 0x50_41C);
        SonicWave { cone, owner, age: 0.0, swept: 0.0, struck: Vec::new(), seed }
    }

    /// The front's radius at the wave's age.
    pub fn front(&self, t: &Tuning) -> f32 {
        self.age * t.sonic_wave_speed
    }

    /// Whether its front is past everything it reaches.
    pub fn spent(&self, t: &Tuning) -> bool {
        self.front(t) > self.cone.longest()
    }

    /// Whether its picture is gone too: its front out, and
    /// `sonic_wave_seconds` more.
    pub fn done(&self, t: &Tuning) -> bool {
        self.age > self.cone.longest() / t.sonic_wave_speed.max(1.0) + t.sonic_wave_seconds
    }

    /// Seconds since the front passed `distance`, `None` before it has.
    fn since(&self, distance: f32, t: &Tuning) -> Option<f32> {
        let dt = self.age - distance / t.sonic_wave_speed.max(1.0);
        (dt >= 0.0).then_some(dt)
    }
}

/// The share of a blast's effect left at `d` px from the pivot: 1 there,
/// `sonic_edge_falloff` at the reach, linear between.
pub fn falloff(t: &Tuning, d: f32) -> f32 {
    let k = (d / t.sonic_reach_px.max(1.0)).clamp(0.0, 1.0);
    1.0 - (1.0 - t.sonic_edge_falloff) * k
}

/// The shove a hull of chassis mass factor `mass_factor` takes at `d` px
/// from the pivot (px/s).
pub fn shove_speed(t: &Tuning, mass_factor: f32, d: f32) -> f32 {
    let resist = mass_factor.max(0.05).powf(t.sonic_mass_exponent);
    (t.sonic_shove_speed * falloff(t, d) / resist).min(t.sonic_shove_max_speed)
}

/// A skidding hull's friction on ground of `grip` (px/s^2).
pub fn skid_friction(t: &Tuning, grip: f32) -> f32 {
    t.sonic_skid_decel * grip.max(t.sonic_skid_grip_floor)
}

/// How far a hull knocked to `speed` slides on ground of `grip`, if
/// nothing stops it (px): the skid's own answer.
pub fn slide(t: &Tuning, speed: f32, grip: f32) -> f32 {
    let f = skid_friction(t, grip);
    speed * speed / (2.0 * f)
}

/// How long a hull knocked to `speed` skids on ground of `grip`.
pub fn skid_seconds(t: &Tuning, speed: f32, grip: f32) -> f32 {
    (speed / skid_friction(t, grip)).min(t.sonic_skid_max_seconds)
}

/// Where a drum at `drum`, reached `falloff` of the way, lands when the
/// wave from `origin` throws it: `sonic_drum_throw_cells * falloff` cells
/// along the line from the pivot through the drum (at least one) - or onto
/// the nearest of `tanks` (owner slot, position) standing beyond the drum
/// within `sonic_drum_aim_deg` of that line and the throw plus a cell,
/// ties on slot - walked back toward the drum a cell at a time off any
/// cell `solid` names and off the field's border cells. `None` when no
/// cell on the way will take it.
pub fn drum_landing(
    t: &Tuning,
    drum: Position,
    origin: Position,
    falloff: f32,
    tanks: &[(usize, Position)],
    field: (f32, f32),
    solid: impl Fn((i32, i32)) -> bool,
) -> Option<Position> {
    let g = OBSTACLE_GRID_SIZE;
    let line = drum - origin;
    let len = line.length();
    let u = if len > 1e-3 { line / len } else { Vec2::new(0.0, -1.0) };
    let cells = (t.sonic_drum_throw_cells * falloff).round().max(1.0);
    let throw = cells * g;
    let cos_aim = t.sonic_drum_aim_deg.to_radians().cos();
    let onto = tanks
        .iter()
        .filter_map(|&(slot, at)| {
            let v = at - drum;
            let d = v.length();
            let ahead = v.x * u.x + v.y * u.y;
            (d > 1e-3 && ahead > 0.0 && ahead / d >= cos_aim && d <= throw + g).then_some((d, slot, at))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
        .map(|(_, _, at)| at);
    let target = onto.unwrap_or(drum + u * throw);
    let way = target - drum;
    let steps = (way.length() / g).round().max(1.0) as i32;
    let start = cell_of(drum);
    for n in (1..=steps).rev() {
        let p = drum + way * (n as f32 / steps as f32);
        let cell = cell_of(p);
        let at = crate::map::cell_to_world(cell.0, cell.1);
        let inside = at.x >= g && at.y >= g && at.x <= field.0 - g && at.y <= field.1 - g;
        if inside && cell != start && !solid(cell) {
            return Some(at);
        }
    }
    None
}

/// Which of the sonic hammer module's four cells (`tank_modules.png`,
/// `TANK_MODULE_SONIC_COL`) `tank` shows at `time`: 3 a blast
/// (`Tank::sonic_flash`), 1 and 2 alternating through a wind-up, quicker as
/// it nears, else 0.
pub fn module_cell(tank: &Tank, time: f32) -> i32 {
    if tank.sonic_flash > 0.0 {
        return 3;
    }
    match tank.tell {
        Some(tell) if tell.weapon == crate::tank::ActiveWeapon::SonicHammer => {
            let hz = 4.0 + 10.0 * tell.progress();
            1 + ((time * hz) as i32).rem_euclid(2)
        }
        _ => 0,
    }
}

/// The arcs of `wave` (the glowing pass: fast and bright): its front and
/// the `sonic_wave_rings - 1` behind it, each split into the runs of rays
/// that reach its radius, so an arc stops at a wall's face. In the stone
/// ramp, leading two blocks thick, thinning as it spreads and over its
/// last third.
pub fn wave_arcs(wave: &SonicWave, t: &Tuning) -> Vec<Shape> {
    let mut out = Vec::new();
    let cone = &wave.cone;
    let longest = cone.longest();
    if longest <= 0.0 || cone.rays.len() < 2 {
        return out;
    }
    let life = longest / t.sonic_wave_speed.max(1.0) + t.sonic_wave_seconds;
    let fade = (1.0 - ((wave.age / life - 2.0 / 3.0) * 3.0).clamp(0.0, 1.0)).clamp(0.0, 1.0);
    let front = wave.front(t);
    let base = facing_rad(cone.facing) - std::f32::consts::FRAC_PI_2;
    let last = cone.rays.len() - 1;
    let step = 2.0 * cone.half_angle / last as f32;
    for k in 0..t.sonic_wave_rings.max(1) {
        let r = front - k as f32 * t.sonic_ring_gap_px;
        if r < MIN_ARC_RADIUS || r > longest {
            continue;
        }
        let color = STONE[(3 - k.min(2)) as usize];
        let width = if k == 0 { pyro::BLOCK * 2.0 } else { pyro::BLOCK };
        let cover = (1.0 - 0.6 * (r / cone.reach.max(1.0)).powi(2)) * fade;
        if cover <= 0.0 {
            continue;
        }
        // Runs of rays reaching `r`, as angle spans from +x (clockwise).
        let mut run: Option<usize> = None;
        for i in 0..=cone.rays.len() {
            let open = i < cone.rays.len() && cone.rays[i] >= r;
            match (open, run) {
                (true, None) => run = Some(i),
                (false, Some(s)) => {
                    let from = base - cone.half_angle + step * (s as f32 - 0.5).max(0.0);
                    let to = base - cone.half_angle + step * ((i - 1) as f32 + 0.5).min(last as f32);
                    out.push(Shape::Arc { center: cone.origin, radius: r, width, from, to, color, cover });
                    run = None;
                }
                _ => {}
            }
        }
    }
    out
}

/// The dust the front lifts off the cells it crosses and the shards of the
/// panes it shatters (the lit pass: lingering, shaded): off dry ground
/// `DUST` puffs rising and leaning with the wind, off water pale chop, off
/// lava `SMOKE` puffs; every `sonic_dust_spacing_px` along the front at
/// hashed offsets, gone in `sonic_dust_seconds`. `time` is the round clock,
/// for the wind.
pub fn wave_dust(wave: &SonicWave, t: &Tuning, time: f32) -> Vec<Shape> {
    let mut out = Vec::new();
    let life = t.sonic_dust_seconds.max(0.05);
    if t.sonic_dust_spacing_px > 0.0 {
        let per_cell = ((OBSTACLE_GRID_SIZE / t.sonic_dust_spacing_px).round() as u32).max(1);
        for c in &wave.cone.cells {
            if c.at < MIN_ARC_RADIUS {
                continue;
            }
            let Some(age) = wave.since(c.at, t).filter(|&a| a < life) else { continue };
            let k = age / life;
            let centre = crate::map::cell_to_world(c.cell.0, c.cell.1);
            let seed = wave.seed ^ crate::blast::seed_at(centre, 0x5D);
            for i in 0..per_cell {
                let jx = (pyro::unit(seed, i * 2) - 0.5) * OBSTACLE_GRID_SIZE;
                let jy = (pyro::unit(seed, i * 2 + 1) - 0.5) * OBSTACLE_GRID_SIZE;
                let at = Position::new(centre.x + jx, centre.y + jy);
                if wave.cone.reaches(at).is_none() {
                    continue;
                }
                let away = unit(at - wave.cone.origin);
                match c.floor {
                    Floor::Dry => {
                        let rise = 8.0 * k;
                        let lean = pyro::smoke_lean(t, at, time) * rise;
                        let pos = Position::new(at.x + away.x * 6.0 * k + lean, at.y + away.y * 6.0 * k - rise);
                        out.push(Shape::Puff(pyro::dust_puff(pos, 3.0 + 4.0 * k, 1.0 - k, 1.0 - k * k)));
                    }
                    Floor::Water => {
                        if k < 0.5 {
                            let color = if i % 2 == 0 { BLUE_PALE } else { STONE[3] };
                            out.push(Shape::Mark { pos: Position::new(at.x, at.y - 6.0 * k), size: 2, color });
                        }
                    }
                    Floor::Lava => {
                        let pos = Position::new(at.x, at.y - 10.0 * k);
                        let mut puff = Puff::plain(pos, 3.0 + 3.0 * k, pyro::step(&pyro::SMOKE, 0.4 - 0.2 * k));
                        puff.cover = 1.0 - k;
                        out.push(Shape::Puff(puff));
                    }
                }
            }
        }
    }
    for pane in &wave.cone.panes {
        let Some(age) = wave.since(pane.at, t).filter(|&a| a < life) else { continue };
        let k = age / life;
        let centre = crate::map::cell_to_world(pane.cell.0, pane.cell.1);
        let away = unit(centre - wave.cone.origin);
        let seed = wave.seed ^ crate::blast::seed_at(centre, 0x9A5);
        for i in 0..8u32 {
            let turn = (pyro::unit(seed, i) - 0.5) * 80f32.to_radians();
            let (s, c) = turn.sin_cos();
            let dir = Vec2::new(away.x * c - away.y * s, away.x * s + away.y * c);
            let reach = 16.0 + 24.0 * pyro::unit(seed, i + 8);
            let pos = Position::new(centre.x + dir.x * reach * k, centre.y + dir.y * reach * k + 30.0 * k * k);
            let color = pyro::alpha(if i % 2 == 0 { BLUE_PALE } else { STONE[3] }, 1.0 - k);
            out.push(Shape::Mark { pos, size: 2, color });
        }
    }
    out
}

/// An enemy's wind-up at the dish `dish`, facing `facing`, `progress` of
/// the way through (the glowing pass, so it reads at night): three arcs
/// within the cone's angles closing in on the dish from 40 px once every
/// 0.2 s, darker far and paler near, dissolving in as the wind-up builds.
pub fn tell_arcs(dish: Position, facing: Dir, progress: f32, time: f32, t: &Tuning) -> Vec<Shape> {
    let mut out = Vec::new();
    let half = t.sonic_half_angle_deg.to_radians();
    let base = facing_rad(facing) - std::f32::consts::FRAC_PI_2;
    let cover = 0.3 + 0.7 * progress.clamp(0.0, 1.0);
    for k in 0..3 {
        let phase = (time / 0.2 + k as f32 / 3.0).rem_euclid(1.0);
        let r = 6.0 + 34.0 * (1.0 - phase);
        let color = pyro::step(&STONE[1..], 1.0 - (r - 6.0) / 34.0);
        out.push(Shape::Arc { center: dish, radius: r, width: pyro::BLOCK, from: base - half, to: base + half, color, cover });
    }
    out
}

/// How far (px, whole blocks) a tree at `tree` leans away from the waves'
/// pivots right now: `sonic_tree_lean_px` as the front passes its cell,
/// swinging back over a quarter second. Sideways only, as a ram's lean.
pub fn tree_push(tree: Position, waves: &[SonicWave], t: &Tuning) -> f32 {
    let cell = cell_of(tree);
    let mut lean = 0.0;
    for w in waves {
        let Some(at) = w.cone.entered(cell) else { continue };
        let Some(dt) = w.since(at, t).filter(|&dt| dt < TREE_SWAY_SECONDS) else { continue };
        let side = (tree.x - w.cone.origin.x).signum();
        lean += side * t.sonic_tree_lean_px * (1.0 - dt / TREE_SWAY_SECONDS);
    }
    (lean / pyro::BLOCK).round() * pyro::BLOCK
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::CpuCanvas;

    const FIELD: (f32, f32) = (34.0 * 32.0, 17.0 * 32.0);

    fn open(_: (i32, i32)) -> Block {
        Block::Open
    }

    fn dry(_: (i32, i32)) -> Floor {
        Floor::Dry
    }

    fn cone_at(origin: Position, facing: Dir, block: impl Fn((i32, i32)) -> Block) -> SonicCone {
        let t = Tuning::DEFAULT;
        SonicCone::cast(origin, facing, t.sonic_reach_px, t.sonic_half_angle_deg.to_radians(), FIELD, block, dry)
    }

    #[test]
    fn the_cone_reaches_its_reach_on_open_ground() {
        let t = Tuning::DEFAULT;
        let c = cone_at(Position::new(320.0, 288.0), Dir::Right, open);
        assert!(c.rays.iter().all(|&r| (r - t.sonic_reach_px).abs() < 1e-3), "{:?}", c.rays);
        assert!(c.reaches(Position::new(320.0 + 150.0, 288.0)).is_some(), "straight ahead");
        assert!(c.reaches(Position::new(320.0 + 170.0, 288.0)).is_none(), "past the reach");
        assert!(c.reaches(Position::new(320.0, 288.0 + 100.0)).is_none(), "beside it");
        assert!(c.reaches(Position::new(320.0 - 60.0, 288.0)).is_none(), "behind it");
        // The edge of the cone: 35 degrees in, 40 out.
        let at = |deg: f32| Position::new(320.0 + 100.0 * deg.to_radians().cos(), 288.0 + 100.0 * deg.to_radians().sin());
        assert!(c.reaches(at(34.0)).is_some() && c.reaches(at(-34.0)).is_some());
        assert!(c.reaches(at(40.0)).is_none() && c.reaches(at(-40.0)).is_none());
        assert!(c.rays.len() >= 50, "{} rays", c.rays.len());
    }

    #[test]
    fn a_blocking_cell_cuts_the_rays_that_meet_it() {
        // A wall cell three cells ahead of the pivot.
        let wall = |c: (i32, i32)| if c == (13, 9) { Block::Wall } else { Block::Open };
        let c = cone_at(Position::new(320.0, 288.0), Dir::Right, wall);
        let behind = Position::new(13.0 * 32.0 + 40.0, 288.0);
        assert!(c.reaches(behind).is_none(), "the wall shadows what stands behind it");
        assert!(c.reaches(Position::new(320.0 + 70.0, 288.0)).is_some(), "short of the wall");
        assert!(c.reaches(Position::new(320.0 + 120.0, 288.0 + 64.0)).is_some(), "beside the shadow");
        assert_eq!(c.entered((13, 9)), None, "a wall cell is never entered");
        let straight = c.reach_at(0.0).expect("on the axis");
        assert!((straight - (13.0 * 32.0 - 16.0 - 320.0)).abs() < 0.05, "it stops at the wall's face: {straight}");
    }

    #[test]
    fn glass_is_struck_and_stops_its_rays() {
        let glass = |c: (i32, i32)| if c == (12, 9) { Block::Glass } else { Block::Open };
        let c = cone_at(Position::new(320.0, 288.0), Dir::Right, glass);
        assert_eq!(c.panes.len(), 1);
        assert_eq!(c.panes[0].cell, (12, 9));
        assert!((c.panes[0].at - (12.0 * 32.0 - 16.0 - 320.0)).abs() < 0.05);
        assert!(c.reaches(Position::new(13.0 * 32.0 + 8.0, 288.0)).is_none(), "the pane shadows what is behind it");
    }

    #[test]
    fn the_cone_never_leaves_the_field() {
        let c = cone_at(Position::new(64.0, 288.0), Dir::Left, open);
        assert!(c.rays.iter().all(|&r| r <= 64.0 / (35f32.to_radians().cos()) + 1e-3), "{:?}", c.rays);
        assert!(c.cells.iter().all(|cell| cell.cell.0 >= 0));
    }

    #[test]
    fn cells_are_in_entry_order() {
        let c = cone_at(Position::new(320.0, 288.0), Dir::Up, open);
        assert!(c.cells.windows(2).all(|w| w[0].at <= w[1].at));
        assert_eq!(c.cells[0].cell, (10, 9), "the pivot's own cell, at 0");
        assert_eq!(c.cells[0].at, 0.0);
    }

    #[test]
    fn drum_landing_takes_the_nearest_tank_behind_then_the_slot() {
        let t = Tuning::DEFAULT;
        let drum = Position::new(320.0, 288.0);
        let origin = Position::new(256.0, 288.0);
        let tanks = [(7, Position::new(416.0, 300.0)), (3, Position::new(416.0, 276.0)), (2, Position::new(256.0, 200.0))];
        let at = drum_landing(&t, drum, origin, 1.0, &tanks, FIELD, |_| false).expect("a landing");
        assert_eq!(at, Position::new(416.0, 288.0), "onto the nearer two's cell, the tie to the lower slot's, snapped");
        let alone = drum_landing(&t, drum, origin, 1.0, &[], FIELD, |_| false).expect("a landing");
        assert_eq!(alone, Position::new(320.0 + 4.0 * 32.0, 288.0), "four cells along the line");
        let weak = drum_landing(&t, drum, origin, 0.1, &[], FIELD, |_| false).expect("a landing");
        assert_eq!(weak, Position::new(352.0, 288.0), "at least a cell");
    }

    #[test]
    fn drum_landing_walks_back_off_a_tile() {
        let t = Tuning::DEFAULT;
        let drum = Position::new(320.0, 288.0);
        let at = drum_landing(&t, drum, Position::new(256.0, 288.0), 1.0, &[], FIELD, |c| c.0 >= 13).expect("a landing");
        assert_eq!(at, Position::new(384.0, 288.0));
        assert_eq!(drum_landing(&t, drum, Position::new(256.0, 288.0), 1.0, &[], FIELD, |c| c.0 >= 11), None);
    }

    #[test]
    fn the_skid_is_v_squared_over_twice_the_friction() {
        let t = Tuning::DEFAULT;
        let v = shove_speed(&t, 1.0, 0.0);
        assert_eq!(v, t.sonic_shove_speed);
        assert!((slide(&t, v, 1.0) - v * v / (2.0 * t.sonic_skid_decel)).abs() < 1e-3);
        assert!(slide(&t, v, 0.15) <= slide(&t, v, 1.0) / t.sonic_skid_grip_floor + 1e-3, "the grip floor caps an icy slide");
        let titan = shove_speed(&t, 528.0 / 280.0, 0.0);
        let scout = shove_speed(&t, 216.0 / 280.0, 0.0);
        assert!(slide(&t, titan, 1.0) * 4.0 < slide(&t, scout, 1.0), "a heavy chassis barely slides");
        assert!(shove_speed(&t, 1.0, t.sonic_reach_px) < v * 0.4, "it falls off to the rim");
    }

    /// Every shape on the grid, in the stone ramp or a puff, gone by its end,
    /// and the same picture from the same wave.
    #[test]
    fn the_wave_is_on_the_grid_in_its_ramp_and_gone_by_its_end() {
        let t = Tuning::DEFAULT;
        let mut wave = SonicWave::new(cone_at(Position::new(320.0, 288.0), Dir::Right, open), Owner::Player(0));
        let mut drew = 0;
        while !wave.done(&t) {
            let arcs = wave_arcs(&wave, &t);
            for s in &arcs {
                let Shape::Arc { color, .. } = s else { panic!("only arcs: {s:?}") };
                assert!(STONE.iter().any(|c| c.r == color.r && c.g == color.g && c.b == color.b), "{color:?}");
            }
            assert_eq!(arcs, wave_arcs(&wave, &t), "pure");
            let mut canvas = CpuCanvas::new(1088, 544, Default::default());
            pyro::draw(&mut canvas, &arcs);
            pyro::draw(&mut canvas, &wave_dust(&wave, &t, 1.0));
            drew += arcs.len();
            wave.age += 1.0 / 60.0;
        }
        assert!(drew > 0, "the wave drew nothing");
        assert!(wave_arcs(&wave, &t).is_empty() && wave_dust(&wave, &t, 1.0).is_empty(), "gone by its end");
    }

    #[test]
    fn the_wave_stops_at_a_walls_face() {
        let t = Tuning::DEFAULT;
        let wall = |c: (i32, i32)| if c.0 == 13 { Block::Wall } else { Block::Open };
        let mut wave = SonicWave::new(cone_at(Position::new(320.0, 288.0), Dir::Right, wall), Owner::Player(0));
        // The face is 80 px ahead, 98 px at the cone's edge: every arc of
        // a front at 140 px lies past it.
        wave.age = 140.0 / t.sonic_wave_speed;
        assert!(wave_arcs(&wave, &t).is_empty(), "no arc past the wall's face");
        wave.age = 60.0 / t.sonic_wave_speed;
        assert!(!wave_arcs(&wave, &t).is_empty());
    }

    #[test]
    fn the_tell_is_on_the_grid_and_pure() {
        let t = Tuning::DEFAULT;
        let a = tell_arcs(Position::new(200.0, 200.0), Dir::Down, 0.5, 3.1, &t);
        assert_eq!(a.len(), 3);
        assert_eq!(a, tell_arcs(Position::new(200.0, 200.0), Dir::Down, 0.5, 3.1, &t));
        let mut canvas = CpuCanvas::new(400, 400, Default::default());
        pyro::draw(&mut canvas, &a);
    }

    /// A whole turn is every block of the ring once: no seam where it
    /// starts and ends, no block drawn twice.
    #[test]
    fn block_arc_draws_a_whole_turn_without_a_seam() {
        struct Blocks(std::collections::BTreeMap<(i32, i32), u32>);
        impl pyro::Blocks for Blocks {
            fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, _: Color) {
                for bx in (x..x + w).step_by(2) {
                    for by in (y..y + h).step_by(2) {
                        *self.0.entry((bx, by)).or_insert(0) += 1;
                    }
                }
            }
        }
        let c = Position::new(100.0, 100.0);
        let mut whole = Blocks(Default::default());
        pyro::block_arc(&mut whole, c, 30.0, 2.0, 1.0, 1.0 + std::f32::consts::TAU, STONE[3], 1.0);
        assert!(whole.0.values().all(|&n| n == 1), "a block drawn twice");
        let mut ring = Blocks(Default::default());
        pyro::block_arc(&mut ring, c, 30.0, 2.0, -std::f32::consts::PI, std::f32::consts::PI, STONE[3], 1.0);
        assert_eq!(whole.0.keys().collect::<Vec<_>>(), ring.0.keys().collect::<Vec<_>>(), "the same ring wherever it starts");
        // Every block of the ring is there: one on each side of the centre.
        for (dx, dy) in [(30, 0), (-30, 0), (0, 30), (0, -30)] {
            let at = (((c.x as i32 + dx) / 2) * 2, ((c.y as i32 + dy) / 2) * 2);
            assert!(whole.0.contains_key(&at) || whole.0.contains_key(&(at.0 - 2, at.1)) || whole.0.contains_key(&(at.0, at.1 - 2)), "{at:?}");
        }
    }

    #[test]
    fn block_arc_stays_on_the_grid_and_within_its_angles() {
        struct Rects(Vec<(i32, i32, i32, i32)>);
        impl pyro::Blocks for Rects {
            fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, _: Color) {
                self.0.push((x, y, w, h));
            }
        }
        let mut r = Rects(Vec::new());
        let c = Position::new(101.0, 99.0);
        pyro::block_arc(&mut r, c, 40.0, 2.0, -0.5, 0.5, STONE[3], 1.0);
        assert!(!r.0.is_empty());
        for &(x, y, w, h) in &r.0 {
            assert!(x % 2 == 0 && y % 2 == 0 && w % 2 == 0 && h == 2, "on the grid: {x},{y} {w}x{h}");
            for bx in (x..x + w).step_by(2) {
                let (dx, dy) = (bx as f32 + 1.0 - c.x, y as f32 + 1.0 - c.y);
                let a = dy.atan2(dx);
                assert!((-0.5 - 1e-3..=0.5 + 1e-3).contains(&a), "{a}");
                assert!(((dx * dx + dy * dy).sqrt() - 40.0).abs() <= 1.0 + 1e-3);
            }
        }
    }
}
