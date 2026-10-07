//! The rod from god (docs/rod-from-god.md), its headless half: the reticle a
//! held trigger puts up and the stick steers (`Reticle`, `step_reticle`), the
//! charge rule it is held under (`rule`), the impact's measures (`shove_speed`,
//! `cell_reach`), the crater it leaves (`Craters`, `crater_cells`), the
//! uplink module's cell, and the pictures - the reticle, the call's beam,
//! circle and countdown, the white column, the dust ring and the debris, the
//! crater - composed in the effects language (docs/effects.md) as pure
//! functions of what they draw and its age, hashed, never rolled. The world
//! half is `simulation/rod.rs`.

use std::collections::BTreeSet;

use crate::canvas::Canvas;
use crate::map::{cell_to_world, world_to_cell};
use crate::math::{Color, Vec2};
use crate::pyro::{self, Shape, DUST, LASER_RED, SMOKE};
use crate::tank::{ActiveWeapon, ChargeRule, Dir, Stick, Tank};
use crate::tuning::Tuning;
use crate::{OBSTACLE_GRID_SIZE, Position};

/// How fast a call's beam flickers through its first seconds (Hz): drawn
/// on two of every three of these frames.
pub const ROD_BEAM_HZ: f32 = 20.0;

/// How long a filled crater's water takes to rise in the picture after the
/// impact (s). Drawn only: the rules made it a ford at once.
pub const ROD_FILL_SECONDS: f32 = 1.0;

/// Water in a filled crater and over a lake struck: the palette's deep,
/// dark and pale blues (`pyro::EMP`'s first steps).
const WATER: [Color; 3] = [pyro::EMP[0], pyro::EMP[1], pyro::EMP[3]];

/// Arcs in a reticle's or a call's circle.
const ARCS: u32 = 8;

/// How fast the circle's arcs turn (rad/s).
const SPIN: f32 = 1.5;

/// How far outside the kill radius a moving reticle's arcs stand, closing
/// in as it comes to rest (px).
const SETTLE_PX: f32 = 8.0;

/// How long a reticle at rest takes to close its arcs (s).
const SETTLE_SECONDS: f32 = 0.3;

/// Where the countdown's digits stand from the circle's centre (px).
const COUNT_AT: (f32, f32) = (38.0, -32.0);

/// The dust rings' speeds (px/s): the pale front and the white echo.
const RING_SPEEDS: (f32, f32) = (230.0, 180.0);

/// How long the dust rings run (s).
const RING_SECONDS: f32 = 0.9;

/// The ground's tilt the rings and the dust are squashed by.
const SQUASH: f32 = 0.7;

/// How high the spray a rod throws up out of water rises (px), and how long
/// it takes to fall back (s).
const SPLASH_PX: f32 = 40.0;
const SPLASH_SECONDS: f32 = 0.6;

/// A rod's call (`zone::ZoneKind::Rod`): the cell it lands on and the seat
/// that called it, if a seat did - the kill credit's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RodCall {
    pub cell: (i32, i32),
    pub seat: Option<u8>,
}

/// The span a seat's velocity is averaged over (`SeatStill::velocity`), s.
pub const SEAT_MOTION_SECONDS: f32 = 1.0;

/// Slower than this (px/s) a seat that is not still is not led either: it
/// is standing, as good as.
pub const SEAT_LEAD_MIN_SPEED: f32 = 4.0;

/// How a seat has been moving (`Game::seat_still`, docs/rod-from-god.md
/// "AI"): measured each live tick from its hull's centre, no RNG - what an
/// enemy with a rod calls on a standing seat by.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeatStill {
    /// Where it last moved off from: reset to its centre whenever the centre
    /// is more than `rod_ai_still_px` from it.
    pub anchor: Position,
    /// Seconds its centre has stayed within `rod_ai_still_px` of `anchor`.
    pub still: f32,
    /// Its velocity (px/s) averaged over about `SEAT_MOTION_SECONDS`.
    pub velocity: Vec2,
    /// Its centre last tick; `None` for a seat not on the field.
    pub last: Option<Position>,
}

impl SeatStill {
    /// One tick: `at` is the hull's centre, `None` for a wreck or a seat off
    /// the field, which reads standing nowhere and moving at nothing.
    pub fn step(&mut self, at: Option<Position>, dt: f32, t: &Tuning) {
        let Some(at) = at else {
            *self = SeatStill::default();
            return;
        };
        match self.last {
            None => {
                *self = SeatStill { anchor: at, still: 0.0, velocity: Vec2::zero(), last: Some(at) };
            }
            Some(last) => {
                let step = (at - last) / dt.max(1e-6);
                let k = (dt / SEAT_MOTION_SECONDS).min(1.0);
                self.velocity = self.velocity + (step - self.velocity) * k;
                if at.distance_to(self.anchor) > t.rod_ai_still_px {
                    self.anchor = at;
                    self.still = 0.0;
                } else {
                    self.still += dt;
                }
                self.last = Some(at);
            }
        }
    }

    /// Its averaged speed (px/s).
    pub fn speed(&self) -> f32 {
        self.velocity.length()
    }
}

/// What steers a reticle this tick (`Tank::step_trigger`): a seat's stick,
/// an enemy's aim (`ai::Intent::aim_cell`), or a room's report of where the
/// client has it (`Game::seat_reticle_report`), which outranks the other
/// two.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Steer {
    pub stick: Option<Dir>,
    pub aim: Option<(i32, i32)>,
    pub report: Option<(i32, i32)>,
}

/// The cells a reticle may stand on, round a hull: inside the box of the
/// caller's sight (`sight_box_half_cols - 0.5` sideways,
/// `sight_box_half_rows - 0.5` up and down of the hull's cell), so the
/// circle is on the caller's screen, and inside the field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub hull: (i32, i32),
    pub half: (i32, i32),
    pub cols: i32,
    pub rows: i32,
}

impl Range {
    /// The range round a hull at `hull` in a field of `field` px.
    pub fn of(hull: Position, field: (f32, f32), t: &Tuning) -> Range {
        let half = ((t.sight_box_half_cols - 0.5).floor().max(0.0) as i32, (t.sight_box_half_rows - 0.5).floor().max(0.0) as i32);
        let cols = ((field.0 / OBSTACLE_GRID_SIZE).floor() as i32).max(1);
        let rows = ((field.1 / OBSTACLE_GRID_SIZE).floor() as i32).max(1);
        Range { hull: world_to_cell(hull), half, cols, rows }
    }

    /// `cell` held inside the range.
    pub fn hold(&self, cell: (i32, i32)) -> (i32, i32) {
        let (hx, hy) = self.hull;
        let x = cell.0.clamp(hx - self.half.0, hx + self.half.0).clamp(0, self.cols - 1);
        let y = cell.1.clamp(hy - self.half.1, hy + self.half.1).clamp(0, self.rows - 1);
        (x, y)
    }

    /// Whether `cell` is inside it.
    pub fn holds(&self, cell: (i32, i32)) -> bool {
        self.hold(cell) == cell
    }
}

/// A rod's reticle (docs/rod-from-god.md "The reticle"): the cell it stands
/// on, the stick that moves it and when that next steps it, and how long it
/// has stood on its cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reticle {
    pub cell: (i32, i32),
    /// The direction held last tick, `None` with the stick at rest.
    pub held: Option<Dir>,
    /// Seconds to the held stick's next step.
    pub repeat: f32,
    /// Seconds on this cell: what an enemy waits on before it calls, and
    /// the drawing's settle.
    pub rest: f32,
}

impl Reticle {
    pub fn new(cell: (i32, i32)) -> Reticle {
        Reticle { cell, held: None, repeat: 0.0, rest: 0.0 }
    }

    /// The centre of its cell: where a call lands.
    pub fn centre(&self) -> Position {
        cell_to_world(self.cell.0, self.cell.1)
    }

    /// Move to `cell`, if it is another one: its rest starts over.
    fn go(&mut self, cell: (i32, i32)) {
        if cell != self.cell {
            self.cell = cell;
            self.rest = 0.0;
        }
    }
}

/// Where a reticle appears: `rod_reticle_start_cells` cells ahead of the
/// hull's cell along its facing, held in range.
pub fn reticle_start(hull: Position, facing: Dir, range: &Range, t: &Tuning) -> (i32, i32) {
    let (c, r) = world_to_cell(hull);
    let v = facing.vec();
    let n = t.rod_reticle_start_cells.max(0);
    range.hold((c + v.x.round() as i32 * n, r + v.y.round() as i32 * n))
}

/// One tick of a reticle, pure: a room's report puts it on the reported
/// cell; else a stick pressed this tick steps it a cell at once and,
/// held, again after `rod_reticle_delay_seconds` and then every
/// `rod_reticle_repeat_seconds`; else with an aim (an enemy's) it steps
/// toward the aim a cell every `rod_reticle_repeat_seconds`, along the axis
/// with the larger offset first (ties across). Every step is held to
/// `range`; a hull that moved pulls it back in, which counts as a step.
pub fn step_reticle(r: &mut Reticle, steer: Steer, range: &Range, dt: f32, t: &Tuning) {
    r.rest += dt;
    let held = range.hold(r.cell);
    r.go(held);
    if let Some(cell) = steer.report {
        let cell = range.hold(cell);
        r.go(cell);
        r.held = None;
        return;
    }
    if let Some(dir) = steer.stick {
        if r.held != Some(dir) {
            r.held = Some(dir);
            r.repeat = t.rod_reticle_delay_seconds;
            step_cell(r, dir, range);
            return;
        }
        r.repeat -= dt;
        while r.repeat <= 0.0 {
            r.repeat += t.rod_reticle_repeat_seconds.max(0.01);
            step_cell(r, dir, range);
        }
        return;
    }
    r.held = None;
    let Some(aim) = steer.aim else {
        r.repeat = 0.0;
        return;
    };
    let aim = range.hold(aim);
    if aim == r.cell {
        r.repeat = 0.0;
        return;
    }
    r.repeat -= dt;
    while r.repeat <= 0.0 && aim != r.cell {
        r.repeat += t.rod_reticle_repeat_seconds.max(0.01);
        let (dx, dy) = (aim.0 - r.cell.0, aim.1 - r.cell.1);
        let dir = if dx.abs() >= dy.abs() {
            if dx > 0 { Dir::Right } else { Dir::Left }
        } else if dy > 0 {
            Dir::Down
        } else {
            Dir::Up
        };
        step_cell(r, dir, range);
    }
}

/// One cell along `dir`, held to `range`.
fn step_cell(r: &mut Reticle, dir: Dir, range: &Range) {
    let v = dir.vec();
    let next = range.hold((r.cell.0 + v.x.round() as i32, r.cell.1 + v.y.round() as i32));
    r.go(next);
}

/// The rod's charge rule (docs/rod-from-god.md): full once settled, never
/// overcharged, timing out `rod_hold_seconds` later; the hull stands and the
/// stick steers the reticle.
pub fn rule(t: &Tuning) -> ChargeRule {
    ChargeRule {
        full: t.rod_settle_seconds,
        overcharge: None,
        vent: t.rod_settle_seconds + t.rod_hold_seconds,
        crawl: 0.0,
        vent_cooldown: t.rod_vent_cooldown_seconds,
        stick: Stick::Aim,
    }
}

/// How far `p` stands from the box of map cell `cell` (0 inside it): what
/// the break radius is measured by.
pub fn cell_reach(p: Position, cell: (i32, i32)) -> f32 {
    crate::emp::box_reach(p, cell_to_world(cell.0, cell.1), Vec2::new(OBSTACLE_GRID_SIZE * 0.5, OBSTACLE_GRID_SIZE * 0.5))
}

/// The shove ring's falloff at `d` px from the struck cell's centre: 1 at
/// the circle's edge, 0 at the shove radius.
pub fn falloff(t: &Tuning, d: f32) -> f32 {
    let span = (t.rod_shove_radius_px - t.rod_kill_radius_px).max(1.0);
    ((t.rod_shove_radius_px - d) / span).clamp(0.0, 1.0)
}

/// The shove a hull of `mass_factor` gets `d` px out (px/s).
pub fn shove_speed(t: &Tuning, mass_factor: f32, d: f32) -> f32 {
    let resist = mass_factor.max(0.05).powf(t.rod_mass_exponent);
    (t.rod_shove_speed * falloff(t, d) / resist).min(t.rod_shove_max_speed)
}

/// A crater's cells: those within `reach` steps of `cell` (a plus at 1), in
/// the field, where `dry` says the ground is dry (no water, no lava, no
/// volcano's cone). Sorted.
pub fn crater_cells(cell: (i32, i32), reach: i32, cols: i32, rows: i32, dry: impl Fn((i32, i32)) -> bool) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for dr in -reach..=reach {
        for dc in -reach..=reach {
            let c = (cell.0 + dc, cell.1 + dr);
            if dc.abs() + dr.abs() <= reach && c.0 >= 0 && c.1 >= 0 && c.0 < cols && c.1 < rows && dry(c) {
                out.push(c);
            }
        }
    }
    out.sort_unstable();
    out
}

/// One crater: the cell the rod struck and the round time it struck.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crater {
    pub cell: (i32, i32),
    pub at: f32,
}

/// The round's craters (`Game::craters`): every crater cell, and the craters
/// in the order they were made, for the drawing. A cell already a crater
/// stays one; craters that touch are one pit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Craters {
    cells: BTreeSet<(i32, i32)>,
    list: Vec<Crater>,
}

impl Craters {
    /// None at all: every rule that reads them is passed by.
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Add a crater struck at `cell` at round time `at`, over `cells`. A
    /// crater already struck there is kept as it was.
    pub fn add(&mut self, cell: (i32, i32), at: f32, cells: &[(i32, i32)]) -> bool {
        if self.list.iter().any(|c| c.cell == cell) {
            return false;
        }
        self.list.push(Crater { cell, at });
        self.cells.extend(cells.iter().copied());
        true
    }

    pub fn clear(&mut self) {
        self.cells.clear();
        self.list.clear();
    }

    /// Whether map cell `cell` is a crater's.
    pub fn holds(&self, cell: (i32, i32)) -> bool {
        self.cells.contains(&cell)
    }

    /// Whether the cell under a world position is a crater's.
    pub fn under(&self, pos: Position) -> bool {
        !self.cells.is_empty() && self.cells.contains(&world_to_cell(pos))
    }

    /// Every crater cell, sorted.
    pub fn cells(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.cells.iter().copied()
    }

    /// The craters in the order they were made.
    pub fn list(&self) -> &[Crater] {
        &self.list
    }
}

/// Where the uplink's designator lens is in the world: the line to the
/// reticle is drawn from it.
pub fn lens(tank: &Tank) -> Position {
    tank.turret_point(crate::tank_art::ROD_LENS[tank.row.clamp(0, 11) as usize])
}

/// Every reticle up on the field as it is drawn (`compose_reticle` and
/// `compose_designator`): a seat's in its team colour, an enemy's in the
/// designator's red, the cancel's X while it stands on its tank's own cell.
pub fn compose_reticles<'a>(out: &mut Vec<Shape>, tanks: impl Iterator<Item = &'a Tank>, t: &Tuning, time: f32) {
    for tank in tanks {
        let Some(r) = tank.reticle.filter(|_| !tank.is_wreck()) else { continue };
        let color = match tank.owner() {
            crate::shell::Owner::Player(seat) => crate::tank::team_color(seat),
            _ => LASER_RED[2],
        };
        let cancel = r.cell == world_to_cell(tank.position);
        compose_designator(out, lens(tank), r.centre(), color);
        compose_reticle(out, &r, cancel, color, t, time);
    }
}

/// Every call standing as it is drawn: its ring closing in on its first
/// moments, then the circle, the beam and the count. `clock` is the time
/// the countdowns are read on (the round's, plus a client's lead to its
/// present).
pub fn compose_calls(out: &mut Vec<Shape>, zones: &[crate::zone::Zone], clock: f32, view_top: f32, t: &Tuning, time: f32) {
    for z in zones {
        if z.rod().is_none() {
            continue;
        }
        let left = z.left(clock);
        compose_call_ring(out, z.centre, t.rod_countdown_seconds - left, t);
        compose_call(out, z.centre, left, view_top, z.id, t, time);
    }
}

/// The uplink module's cell (`tank::module_cols`): 3 a call's uplink while
/// `Tank::rod_flash` runs, 4 offline (an EMP), 1 and 2 alternating at 6 Hz
/// while a reticle is up, else 0.
pub fn module_cell(tank: &Tank, time: f32) -> i32 {
    if tank.rod_flash > 0.0 {
        return 3;
    }
    if tank.special_down() {
        return 4;
    }
    if tank.charge.is_some_and(|c| c.weapon == ActiveWeapon::RodFromGod) {
        return 1 + ((time * 6.0) as i32).rem_euclid(2);
    }
    0
}

/// The circle's eight arcs round `at`, `radius` out, turned by `time`, one
/// block wide: each an eighth of a turn less `gap` (rad) - none for a
/// closed ring.
fn arcs(out: &mut Vec<Shape>, at: Position, radius: f32, time: f32, gap: f32, color: Color, cover: f32) {
    let step = std::f32::consts::TAU / ARCS as f32;
    let turn = time * SPIN;
    for i in 0..ARCS {
        let from = turn + i as f32 * step;
        out.push(Shape::Arc { center: at, radius, width: pyro::BLOCK, from, to: from + step - gap, color, cover });
    }
}

/// A reticle as it is drawn (docs/rod-from-god.md "Drawing"): eight arcs
/// of a circle round its cell's centre turning, `rod_kill_radius_px` out
/// and `SETTLE_PX` more while it moves, closing in over its first
/// `SETTLE_SECONDS` at rest; a cross of four two-block arms at the centre.
/// On the caller's own cell (`cancel`) the arcs dissolve to half and the
/// cross is an X. In `color`: a seat's team colour, an enemy's designator
/// red.
pub fn compose_reticle(out: &mut Vec<Shape>, r: &Reticle, cancel: bool, color: Color, t: &Tuning, time: f32) {
    let at = r.centre();
    let settle = (r.rest / SETTLE_SECONDS).clamp(0.0, 1.0);
    let radius = t.rod_kill_radius_px + SETTLE_PX * (1.0 - pyro::ease_out(settle));
    arcs(out, at, radius, time, 0.18, color, if cancel { 0.5 } else { 1.0 });
    let arm = 6.0;
    let lines: [(f32, f32); 4] = if cancel { [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] } else { [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] };
    for (dx, dy) in lines {
        let from = Position::new(at.x + dx * 2.0, at.y + dy * 2.0);
        let to = Position::new(at.x + dx * arm, at.y + dy * arm);
        out.push(Shape::Line { from, to, width: 1.0, head: color, tail: color });
    }
}

/// The designator's dotted line (docs/rod-from-god.md "Drawing"): a block
/// every 6 px from the module's lens to the reticle's centre, in its colour,
/// alternate blocks only - half its cover.
pub fn compose_designator(out: &mut Vec<Shape>, lens: Position, to: Position, color: Color) {
    let d = to - lens;
    let len = d.length();
    if len < 8.0 {
        return;
    }
    let n = (len / 6.0) as i32;
    for i in 1..n {
        if i % 2 == 1 {
            continue;
        }
        let p = lens + d * (i as f32 * 6.0 / len);
        out.push(Shape::Mark { pos: p, size: 2, color });
    }
}

/// A call as it is drawn for `left` seconds to its impact
/// (docs/rod-from-god.md "Drawing"): the circle's arcs in the designator's
/// red, closing and beating at 4 Hz over the last second; the beam, one
/// block wide from the circle's centre straight up to `view_top`, flickering
/// for the first three seconds and steady, two blocks wide with a white core,
/// over the last; a glow at its foot; the whole seconds left beside the
/// circle in block digits, white in the last second. `seed` phases the
/// flicker per call.
pub fn compose_call(out: &mut Vec<Shape>, at: Position, left: f32, view_top: f32, seed: u32, t: &Tuning, time: f32) {
    let last = left <= 1.0;
    let beat = if last { if (time * 8.0) as i32 % 2 == 0 { 1.0 } else { 0.7 } } else { 1.0 };
    arcs(out, at, t.rod_kill_radius_px, time, if last { 0.0 } else { 0.22 }, LASER_RED[2], beat);
    let frame = (time * ROD_BEAM_HZ + (seed % 3) as f32) as i32;
    let flicker_on = left <= 1.0 || frame.rem_euclid(3) != 0;
    let top = view_top.min(at.y - 8.0);
    if flicker_on && left <= t.rod_countdown_seconds + 0.5 {
        if last {
            out.push(Shape::Line { from: Position::new(at.x - 1.0, top), to: Position::new(at.x - 1.0, at.y), width: 1.0, head: LASER_RED[2], tail: LASER_RED[2] });
            out.push(Shape::Line { from: Position::new(at.x + 1.0, top), to: Position::new(at.x + 1.0, at.y), width: 1.0, head: LASER_RED[4], tail: LASER_RED[3] });
        } else {
            out.push(Shape::Line { from: Position::new(at.x, top), to: Position::new(at.x, at.y), width: 1.0, head: LASER_RED[2], tail: LASER_RED[2] });
            // A brighter block running down it every 8 px.
            let run = ((time * 120.0) as i32).rem_euclid(8) as f32;
            let mut y = at.y - run;
            while y > top {
                out.push(Shape::Mark { pos: Position::new(at.x, y), size: 2, color: LASER_RED[3] });
                y -= 8.0;
            }
        }
    }
    out.push(Shape::Glow { pos: at, radius: 16.0, color: pyro::alpha(LASER_RED[2], 0.5 + 0.5 * beat) });
    let n = left.ceil().max(1.0) as u32;
    let digit = if last { LASER_RED[4] } else { LASER_RED[3] };
    pyro::digits(out, n, Position::new(at.x + COUNT_AT.0, at.y + COUNT_AT.1), 2, digit, SMOKE[0]);
}

/// The ring a call puts on as it is made (`rod_call_show`): designator-red
/// marks closing from 32 px outside the circle onto it over 0.15 s.
pub fn compose_call_ring(out: &mut Vec<Shape>, at: Position, age: f32, t: &Tuning) {
    let k = (age / 0.15).clamp(0.0, 1.0);
    if k >= 1.0 {
        return;
    }
    let radius = t.rod_kill_radius_px + 32.0 * (1.0 - k);
    out.push(Shape::Arc { center: at, radius, width: pyro::BLOCK, from: 0.0, to: std::f32::consts::TAU, color: LASER_RED[3], cover: 1.0 - k * 0.5 });
}

/// What the impact threw up as it is drawn, aged in `tick_effects` until it
/// is done (`Game::rod_impacts`): where, how long ago, the ground's dust it
/// raised and the seed every choice hashes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RodImpactFx {
    pub at: Position,
    pub age: f32,
    pub ground: Ground,
    pub seed: u32,
}

/// What the rod struck, for its dust: dry ground, water, lava, or ice and
/// snow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ground {
    Dry,
    Water,
    Lava,
    Snow,
}

impl RodImpactFx {
    pub fn new(at: Position, ground: Ground) -> RodImpactFx {
        RodImpactFx { at, age: 0.0, ground, seed: crate::blast::seed_at(at, 0x52_4F_44) }
    }

    /// Whether its last puff and debris have gone.
    pub fn done(&self, t: &Tuning) -> bool {
        self.age > t.rod_dust_seconds.max(RING_SECONDS).max(t.rod_column_seconds)
    }
}

/// The white column (docs/rod-from-god.md "Drawing"): for
/// `rod_column_seconds`, six blocks wide of white from `view_top` down to
/// the strike, a designator-red block either side. Glowing pass.
pub fn compose_column(out: &mut Vec<Shape>, fx: &RodImpactFx, view_top: f32, t: &Tuning) {
    if fx.age > t.rod_column_seconds {
        return;
    }
    let at = fx.at;
    let top = view_top.min(at.y - 8.0);
    for i in -3..3 {
        let x = at.x + i as f32 * pyro::BLOCK + 1.0;
        out.push(Shape::Line { from: Position::new(x, top), to: Position::new(x, at.y), width: 1.0, head: Color::WHITE, tail: Color::WHITE });
    }
    for x in [at.x - 7.0, at.x + 7.0] {
        out.push(Shape::Line { from: Position::new(x, top), to: Position::new(x, at.y), width: 1.0, head: LASER_RED[3], tail: LASER_RED[3] });
    }
    out.push(Shape::Glow { pos: at, radius: 48.0, color: Color::WHITE });
}

/// The dust the impact raises (docs/rod-from-god.md "Drawing"), lit pass:
/// two rings racing out squashed to the ground's tilt - a two-block one in
/// the ground's pale step and a one-block white echo - dissolving over
/// `RING_SECONDS`; ten shaded puffs running out from 30 to 150 px, growing,
/// leaning with the wind (`lean`, px per second), gone in
/// `rod_dust_seconds`; out of water a column of spray rising `SPLASH_PX`
/// and falling back over `SPLASH_SECONDS`; twelve debris blocks thrown out
/// on hashed bearings in arcs, gone where they land.
pub fn compose_impact(out: &mut Vec<Shape>, fx: &RodImpactFx, lean: f32, t: &Tuning) {
    let k = fx.age;
    let at = fx.at;
    let (pale, mid, dark) = match fx.ground {
        Ground::Dry => (DUST[4], DUST[2], DUST[1]),
        Ground::Water => (WATER[2], Color::WHITE, WATER[2]),
        Ground::Lava => (SMOKE[4], SMOKE[3], SMOKE[2]),
        Ground::Snow => (SMOKE[6], Color::WHITE, SMOKE[5]),
    };
    if k < RING_SECONDS {
        let fade = 1.0 - k / RING_SECONDS;
        ellipse_ring(out, at, k * RING_SPEEDS.0, 2.0, pale, fade);
        ellipse_ring(out, at, k * RING_SPEEDS.1, 1.0, Color::WHITE, fade);
    }
    let life = t.rod_dust_seconds.max(0.1);
    if k < life {
        let p = k / life;
        for i in 0..10u32 {
            let a = i as f32 / 10.0 * std::f32::consts::TAU + 0.3 + (pyro::unit(fx.seed, i) - 0.5) * 0.3;
            let d = 30.0 + 120.0 * pyro::ease_out(p.min(1.0));
            let pos = Position::new(at.x + a.cos() * d + lean * k, at.y + a.sin() * d * SQUASH - 6.0 * p);
            let radius = 8.0 + 8.0 * p;
            let cover = (1.0 - p).clamp(0.0, 1.0);
            out.push(Shape::Puff(pyro::Puff { pos, radius, body: mid, shadow: Some(dark), lit: Some(pale), core: None, cover }));
        }
    }
    // Out of water, a column of spray thrown up over the strike and falling
    // back: white blocks with a pale blue one in three.
    if fx.ground == Ground::Water && k < SPLASH_SECONDS {
        let p = k / SPLASH_SECONDS;
        let height = SPLASH_PX * 4.0 * p * (1.0 - p);
        for i in 0..12u32 {
            let x = at.x + (pyro::unit(fx.seed, 120 + i) - 0.5) * 20.0;
            let y = at.y - height * (0.3 + 0.7 * pyro::unit(fx.seed, 140 + i));
            let color = if i % 3 == 0 { WATER[2] } else { Color::WHITE };
            out.push(Shape::Mark { pos: Position::new(x, y), size: 2, color });
        }
    }
    if k < 1.0 {
        for i in 0..12u32 {
            let a = pyro::unit(fx.seed, 40 + i) * std::f32::consts::TAU;
            let reach = 140.0 * (0.5 + pyro::unit(fx.seed, 60 + i));
            let d = reach * k;
            let height = 60.0 * k - 120.0 * k * k;
            if height < -2.0 {
                continue;
            }
            let color = if i % 2 == 0 { SMOKE[1] } else { DUST[0] };
            out.push(Shape::Mark { pos: Position::new(at.x + a.cos() * d, at.y + a.sin() * d * SQUASH - height.max(0.0)), size: 2, color });
        }
    }
}

/// A ring of blocks round `at`, `radius` across and squashed to the ground's
/// tilt, `width` blocks wide, its blocks dropping through the Bayer pattern
/// as `cover` falls.
fn ellipse_ring(out: &mut Vec<Shape>, at: Position, radius: f32, width: f32, color: Color, cover: f32) {
    if radius < 4.0 || cover <= 0.0 {
        return;
    }
    let n = ((radius * std::f32::consts::TAU / pyro::BLOCK) as u32).max(8);
    for i in 0..n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let p = Position::new(at.x + a.cos() * radius, at.y + a.sin() * radius * SQUASH);
        let (bx, by) = pyro::block_of(p.x, p.y);
        if pyro::bayer(bx, by) > cover {
            continue;
        }
        out.push(Shape::Mark { pos: p, size: (width * pyro::BLOCK) as i32, color });
    }
}

/// The smoke a fresh crater gives off (docs/rod-from-god.md "Drawing"): for
/// `rod_crater_smoke_seconds` after its impact, a thin puff every 0.3 s from
/// the pit rising and leaning with the wind (`lean`), the last third
/// thinning out. `age` is the crater's.
pub fn compose_crater_smoke(out: &mut Vec<Shape>, at: Position, age: f32, lean: f32, t: &Tuning) {
    let life = t.rod_crater_smoke_seconds;
    if life <= 0.0 || age > life + 1.2 || age < 0.0 {
        return;
    }
    let seed = crate::blast::seed_at(at, 0x534D);
    let first = ((age - 1.2) / 0.3).ceil().max(0.0) as u32;
    let last = (age.min(life) / 0.3) as u32;
    for i in first..=last {
        let born = i as f32 * 0.3;
        let k = age - born;
        if !(0.0..=1.2).contains(&k) {
            continue;
        }
        let thin = if born > life * 2.0 / 3.0 { 1.0 - (born - life * 2.0 / 3.0) / (life / 3.0) } else { 1.0 };
        let off = (pyro::unit(seed, i) - 0.5) * 16.0;
        let pos = Position::new(at.x + off + lean * k, at.y - 4.0 - 24.0 * k);
        let cover = ((1.0 - k / 1.2) * thin).clamp(0.0, 1.0);
        out.push(Shape::Puff(pyro::Puff { pos, radius: 5.0 + 5.0 * k, body: SMOKE[3], shadow: None, lit: Some(SMOKE[4]), core: None, cover }));
    }
}

/// How a crater's water looks: dry, filled, or filled and frozen over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CraterWater {
    Dry,
    Filled,
    Frozen,
}

/// A crater as the floor shows it (docs/rod-from-god.md "Drawing"), on any
/// canvas: discs of blocks round the struck cell's centre - the dark step
/// to 34 px, black to 26 px and a char core 18 px a little down and right,
/// its shadow side - each block's edge pushed in or out a block by a hash of
/// where it lies, so the rim is ragged; a rim of thrown earth at 36 px.
/// Filled, water inside 30 px rising from the centre over
/// `ROD_FILL_SECONDS` after the impact; frozen, ice. `age` is seconds since
/// the impact, `rain` how hard it rains (rings on the water), `time` the
/// round clock.
pub fn draw_crater(c: &mut impl Canvas, crater: &Crater, age: f32, water: CraterWater, rain: f32, time: f32) {
    let at = cell_to_world(crater.cell.0, crater.cell.1);
    let seed = crate::blast::seed_at(at, 0x4352);
    let ragged = |x: i32, y: i32| (pyro::unit(seed ^ (x as u32).wrapping_mul(73_856_093) ^ (y as u32).wrapping_mul(19_349_663), 0) - 0.5) * 2.0 * pyro::BLOCK;
    let b = pyro::BLOCK as i32;
    let reach = 40;
    let (cx, cy) = (pyro::snap(at.x), pyro::snap(at.y));
    for y in (cy - reach..=cy + reach).step_by(b as usize) {
        for x in (cx - reach..=cx + reach).step_by(b as usize) {
            let (fx, fy) = (x as f32 + 1.0 - at.x, (y as f32 + 1.0 - at.y) / 0.92);
            let d = (fx * fx + fy * fy).sqrt() + ragged(x, y);
            let core = ((x as f32 + 1.0 - at.x - 4.0).powi(2) + (y as f32 + 1.0 - at.y - 4.0).powi(2)).sqrt();
            let color = if d <= 18.0 && core <= 18.0 {
                pyro::CHAR[0]
            } else if d <= 26.0 {
                SMOKE[0]
            } else if d <= 34.0 {
                SMOKE[1]
            } else if d <= 38.0 {
                if (x / b + y / b).rem_euclid(2) == 0 { DUST[0] } else { DUST[1] }
            } else {
                continue;
            };
            c.fill_rect(x, y, b, b, color);
        }
    }
    if water == CraterWater::Dry {
        return;
    }
    let rise = (age / ROD_FILL_SECONDS).clamp(0.0, 1.0);
    let surface = 30.0 * rise;
    for y in (cy - reach..=cy + reach).step_by(b as usize) {
        for x in (cx - reach..=cx + reach).step_by(b as usize) {
            let (fx, fy) = (x as f32 + 1.0 - at.x, (y as f32 + 1.0 - at.y) / 0.92);
            let d = (fx * fx + fy * fy).sqrt() + ragged(x, y) * 0.5;
            if d > surface {
                continue;
            }
            let color = match water {
                CraterWater::Frozen => {
                    if pyro::unit(seed, (x * 7 + y * 13) as u32) < 0.08 { WATER[2] } else if fx + fy < -8.0 { Color::WHITE } else { SMOKE[6] }
                }
                _ => {
                    let window = (time / 0.4) as u32;
                    if pyro::unit(seed ^ window.wrapping_mul(0x9E37_79B9), (x * 7 + y * 13) as u32) < 0.03 {
                        WATER[2]
                    } else if fx + fy < -surface * 0.9 {
                        WATER[1]
                    } else {
                        WATER[0]
                    }
                }
            };
            c.fill_rect(x, y, b, b, color);
        }
    }
    if water == CraterWater::Filled && rain > 0.0 && rise >= 1.0 {
        // A raindrop's ring now and then, opening from 2 to 6 px.
        let window = (time / 0.4).floor();
        let k = (time / 0.4) - window;
        let w = window as u32;
        for i in 0..2u32 {
            if pyro::unit(seed ^ w.wrapping_mul(0x85EB_CA6B), 90 + i) > rain {
                continue;
            }
            let a = pyro::unit(seed ^ w, 70 + i) * std::f32::consts::TAU;
            let r0 = 18.0 * pyro::unit(seed ^ w, 80 + i);
            let p = Position::new(at.x + a.cos() * r0, at.y + a.sin() * r0);
            let radius = 2.0 + 4.0 * k;
            let n = 8;
            for j in 0..n {
                let aa = j as f32 / n as f32 * std::f32::consts::TAU;
                let q = Position::new(p.x + aa.cos() * radius, p.y + aa.sin() * radius * 0.8);
                let (bx, by) = pyro::block_of(q.x, q.y);
                c.fill_rect(bx * b, by * b, b, b, WATER[2]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Tuning {
        Tuning::DEFAULT
    }

    fn range() -> Range {
        Range::of(cell_to_world(10, 8), (34.0 * 32.0, 17.0 * 32.0), &t())
    }

    #[test]
    fn the_range_is_the_sight_box_less_half_a_cell_inside_the_field() {
        let r = range();
        assert_eq!(r.half, (11, 7));
        assert_eq!(r.hold((40, 8)), (21, 8));
        assert_eq!(r.hold((10, -5)), (10, 1));
        assert_eq!(r.hold((-30, 30)), (0, 15), "the field's edge outranks the box");
        assert!(r.holds((15, 4)));
    }

    #[test]
    fn reticle_start_is_ahead_and_in_range() {
        let r = range();
        assert_eq!(reticle_start(cell_to_world(10, 8), Dir::Right, &r, &t()), (14, 8));
        assert_eq!(reticle_start(cell_to_world(10, 8), Dir::Up, &r, &t()), (10, 4));
        let edge = Range::of(cell_to_world(1, 8), (34.0 * 32.0, 17.0 * 32.0), &t());
        assert_eq!(reticle_start(cell_to_world(1, 8), Dir::Left, &edge, &t()), (0, 8));
    }

    #[test]
    fn the_stick_steps_once_then_repeats() {
        let t = t();
        let r0 = range();
        let mut r = Reticle::new((14, 8));
        let dt = 1.0 / 60.0;
        let right = Steer { stick: Some(Dir::Right), ..Steer::default() };
        step_reticle(&mut r, right, &r0, dt, &t);
        assert_eq!(r.cell, (15, 8), "a press steps at once");
        let mut cells = vec![r.cell];
        for _ in 0..60 {
            step_reticle(&mut r, right, &r0, dt, &t);
            cells.push(r.cell);
        }
        let first_repeat = cells.iter().position(|&c| c == (16, 8)).unwrap();
        assert!((first_repeat as f32 * dt - t.rod_reticle_delay_seconds).abs() < 2.0 * dt, "{first_repeat}");
        assert_eq!(r.cell, (21, 8), "held, it runs to the box's edge and stops");
        // Released, it stays; a tap is one cell.
        step_reticle(&mut r, Steer::default(), &r0, dt, &t);
        step_reticle(&mut r, Steer { stick: Some(Dir::Left), ..Steer::default() }, &r0, dt, &t);
        step_reticle(&mut r, Steer::default(), &r0, dt, &t);
        assert_eq!(r.cell, (20, 8));
    }

    #[test]
    fn an_aim_walks_the_reticle_larger_offset_first() {
        let t = t();
        let r0 = range();
        let mut r = Reticle::new((10, 8));
        let aim = Steer { aim: Some((14, 6)), ..Steer::default() };
        let mut path = vec![r.cell];
        for _ in 0..200 {
            step_reticle(&mut r, aim, &r0, 1.0 / 60.0, &t);
            if path.last() != Some(&r.cell) {
                path.push(r.cell);
            }
        }
        assert_eq!(r.cell, (14, 6));
        assert_eq!(path[1], (11, 8), "across first, the larger offset");
        assert!(r.rest > 2.0, "it rests once there");
    }

    #[test]
    fn a_report_puts_it_on_the_reported_cell_held_in_range() {
        let t = t();
        let r0 = range();
        let mut r = Reticle::new((14, 8));
        step_reticle(&mut r, Steer { report: Some((3, 3)), stick: Some(Dir::Right), ..Steer::default() }, &r0, 1.0 / 60.0, &t);
        assert_eq!(r.cell, (3, 3));
        step_reticle(&mut r, Steer { report: Some((99, 3)), ..Steer::default() }, &r0, 1.0 / 60.0, &t);
        assert_eq!(r.cell, (21, 3));
    }

    #[test]
    fn crater_cells_are_a_plus_of_dry_cells_in_the_field() {
        assert_eq!(crater_cells((5, 5), 1, 34, 17, |_| true), vec![(4, 5), (5, 4), (5, 5), (5, 6), (6, 5)]);
        assert_eq!(crater_cells((0, 0), 1, 34, 17, |_| true), vec![(0, 0), (0, 1), (1, 0)]);
        assert_eq!(crater_cells((5, 5), 1, 34, 17, |c| c != (5, 6)), vec![(4, 5), (5, 4), (5, 5), (6, 5)]);
    }

    #[test]
    fn the_shove_falls_off_to_nothing_at_its_radius() {
        let t = t();
        assert_eq!(shove_speed(&t, 1.0, t.rod_kill_radius_px), t.rod_shove_speed);
        assert_eq!(shove_speed(&t, 1.0, t.rod_shove_radius_px), 0.0);
        assert!(shove_speed(&t, 2.0, 60.0) < shove_speed(&t, 1.0, 60.0), "a heavy chassis resists");
    }

    #[test]
    fn cell_reach_is_the_distance_to_the_cells_box() {
        assert_eq!(cell_reach(cell_to_world(3, 3), (3, 3)), 0.0);
        assert_eq!(cell_reach(cell_to_world(3, 3), (5, 3)), 48.0);
    }

    #[test]
    fn the_reticle_is_on_the_grid_and_pure() {
        let t = t();
        let r = Reticle { cell: (10, 8), held: None, repeat: 0.0, rest: 0.1 };
        let mut a = Vec::new();
        compose_reticle(&mut a, &r, false, LASER_RED[2], &t, 1.25);
        let mut b = Vec::new();
        compose_reticle(&mut b, &r, false, LASER_RED[2], &t, 1.25);
        assert_eq!(a, b);
        assert!(a.iter().any(|s| matches!(s, Shape::Arc { .. })));
    }

    #[test]
    fn the_call_draws_its_countdown_and_its_beam_holds_in_the_last_second() {
        let t = t();
        let at = cell_to_world(10, 8);
        let lines = |left: f32, time: f32| {
            let mut out = Vec::new();
            compose_call(&mut out, at, left, 0.0, 1, &t, time);
            out.iter().filter(|s| matches!(s, Shape::Line { .. })).count()
        };
        let flickers: Vec<usize> = (0..6).map(|i| lines(3.0, i as f32 / ROD_BEAM_HZ)).collect();
        assert!(flickers.contains(&0) && flickers.iter().any(|&n| n > 0), "{flickers:?}");
        assert!((0..6).all(|i| lines(0.5, i as f32 / ROD_BEAM_HZ) == 2), "steady and doubled in the last second");
    }

    #[test]
    fn the_column_lasts_its_frames_and_the_impact_is_gone_by_its_end() {
        let t = t();
        let mut fx = RodImpactFx::new(cell_to_world(10, 8), Ground::Dry);
        let mut out = Vec::new();
        compose_column(&mut out, &fx, 0.0, &t);
        assert!(!out.is_empty());
        fx.age = t.rod_column_seconds + 0.01;
        out.clear();
        compose_column(&mut out, &fx, 0.0, &t);
        assert!(out.is_empty());
        fx.age = 0.4;
        compose_impact(&mut out, &fx, 0.0, &t);
        assert!(!out.is_empty());
        fx.age = 10.0;
        assert!(fx.done(&t));
        out.clear();
        compose_impact(&mut out, &fx, 0.0, &t);
        assert!(out.is_empty());
    }

    #[test]
    fn a_strike_in_water_throws_up_spray_that_falls_back() {
        let t = t();
        let marks_above = |ground: Ground, age: f32| {
            let mut fx = RodImpactFx::new(cell_to_world(10, 8), ground);
            fx.age = age;
            let mut out = Vec::new();
            compose_impact(&mut out, &fx, 0.0, &t);
            out.iter().filter(|s| matches!(s, Shape::Mark { pos, .. } if pos.y < fx.at.y - 12.0 && (pos.x - fx.at.x).abs() <= 10.0)).count()
        };
        assert!(marks_above(Ground::Water, SPLASH_SECONDS * 0.5) >= 6, "a column of spray at its height");
        assert!(marks_above(Ground::Water, SPLASH_SECONDS * 0.5) > marks_above(Ground::Dry, SPLASH_SECONDS * 0.5), "only out of water");
        let end = SPLASH_SECONDS + 0.01;
        assert_eq!(marks_above(Ground::Water, end), marks_above(Ground::Dry, end), "fallen back by its end");
    }

    #[test]
    fn a_crater_draws_the_same_on_any_canvas_and_a_filled_one_draws_water() {
        use crate::canvas::CpuCanvas;
        let crater = Crater { cell: (3, 3), at: 0.0 };
        let mut a = CpuCanvas::blank(192, 192);
        draw_crater(&mut a, &crater, 5.0, CraterWater::Dry, 0.0, 5.0);
        let mut b = CpuCanvas::blank(192, 192);
        draw_crater(&mut b, &crater, 5.0, CraterWater::Dry, 0.0, 5.0);
        assert_eq!(a.pixels(), b.pixels());
        let at = |c: &CpuCanvas| c.pixel(96, 96);
        assert_eq!(at(&a), pyro::CHAR[0]);
        let mut w = CpuCanvas::blank(192, 192);
        draw_crater(&mut w, &crater, 5.0, CraterWater::Filled, 0.0, 5.0);
        assert!(matches!(at(&w), c if c == WATER[0] || c == WATER[1] || c == WATER[2]), "{:?}", at(&w));
    }
}
