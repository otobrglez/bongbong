//! The gravity well (docs/gravity-well.md), its headless half: the orb in
//! flight (`Orb`), a well standing on the field (`WellZone`, a `Zone`'s
//! kind), **the well field** - the pull every mover reads, a pure function
//! of the wells pulling at a round tick and a point (`WellField`) - what a
//! well holds (drums circling its ring, `HeldDrum`; grenades in orbit,
//! `GrenadeOrbit`), the AI's measures and the pictures, composed in the
//! effects language (docs/effects.md) as pure functions of what they draw
//! and its age, hashed, never rolled. The world half is
//! `simulation/well.rs`.
//!
//! The field is arithmetic only - square roots and divisions, and a hull's
//! mass factor raised to its exponent - summed in source id order, so the
//! room, a client's sandbox, its provisional shots and its incoming fire
//! all read the same pull on the same tick.

use crate::math::{Color, Vec2};
use crate::obstacle::Drum;
use crate::pyro::{self, Shape, SMOKE, VOID};
use crate::shell::Owner;
use crate::tank::{ActiveWeapon, Dir, Tank};
use crate::tuning::Tuning;
use crate::zone::{Zone, ZoneKind};
use crate::{PHYSICS_FIXED_DT, Position};

/// The radius of the orb's outer disc (px).
const ORB_PX: f32 = 6.0;

/// How long a fresh orb takes to swell to its size at the muzzle (s).
pub const ORB_SWELL_SECONDS: f32 = 0.1;

/// The accretion ring's radius round the core (px) and how fast it turns
/// (rad/s, clockwise as drawn).
const RING_PX: f32 = 13.0;
const RING_SPIN: f32 = 4.0;

/// The snap's ring: from how far out it closes onto the anchor (px) and how
/// long it takes (s).
const SNAP_FROM_PX: f32 = 32.0;
const SNAP_SECONDS: f32 = 0.12;

/// The collapse's two rings (px/s) and how long they run (s); the
/// implosion's flash (s).
const COLLAPSE_SPEEDS: (f32, f32) = (220.0, 160.0);
const COLLAPSE_SECONDS: f32 = 0.6;
const IMPLODE_SECONDS: f32 = 0.06;

/// How long a swallow's pop runs (s).
pub const SWALLOW_SECONDS: f32 = 0.15;

/// How high a held drum is drawn lifted over its shadow (px).
const HELD_LIFT_PX: f32 = 10.0;

/// The orb flying off the gun line toward where it anchors: a shot (on the
/// wire, `ShotKind::Orb`), integrated in the fixed-step loop, bent by other
/// wells, swept every tick (`simulation::well`). It hurts nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orb {
    /// From the round's projectile counter (`Game::spawn_pending`): its
    /// wire key, and the id of the well it becomes.
    pub id: u32,
    pub owner: Owner,
    pub position: Position,
    /// Where it stood at the start of this tick: the stretch its sweep
    /// checks.
    pub prev_position: Position,
    pub velocity: Vec2,
    /// Its heading in degrees, 0 = up, turned by a bend.
    pub rotation: f32,
    /// How far it has flown (px): it anchors itself at
    /// `well_orb_range_px`.
    pub flown: f32,
    /// Seconds since it left the muzzle (the swell, the circling motes).
    pub age: f32,
    /// How many ticks behind the room its seat's client drew the world when
    /// it was fired (`Game::seat_rewind`): its sweep meets enemies where
    /// they were drawn, as every seat's shot does. 0 for an enemy's.
    pub rewind: u8,
}

impl Orb {
    /// An orb leaving `muzzle` along the unit `dir` at `well_orb_speed`.
    pub fn launch(muzzle: Position, dir: Vec2, owner: Owner, t: &Tuning) -> Orb {
        let velocity = dir * t.well_orb_speed;
        Orb { id: 0, owner, position: muzzle, prev_position: muzzle, velocity, rotation: heading_deg(velocity), flown: 0.0, age: 0.0, rewind: 0 }
    }

    /// One fixed step of flight: straight along its velocity.
    pub fn advance(&mut self, dt: f32) {
        self.position = self.position + self.velocity * dt;
        self.flown += self.velocity.length() * dt;
        self.age += dt;
    }

    /// Turn its velocity by `accel` over `dt`, keeping its speed
    /// (`bend`), and its heading with it.
    pub fn bend(&mut self, accel: Vec2, dt: f32) {
        self.velocity = bend(self.velocity, accel, dt);
        self.rotation = heading_deg(self.velocity);
    }
}

/// The heading of `v` in degrees, 0 = up, clockwise as drawn.
pub fn heading_deg(v: Vec2) -> f32 {
    v.x.atan2(-v.y).to_degrees()
}

/// What stage a well stands in (`WellZone::stage`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WellStage {
    /// The snap and the ring forming: nothing pulled yet.
    Forming,
    /// It pulls (`WellField`); `Zone::until` is its collapse.
    Pulling,
}

/// How a well was anchored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnchorBy {
    /// Its shooter's second press.
    Press,
    /// Its orb met a tile, a hull, a frog or the field's edge.
    Contact,
    /// Its orb flew `well_orb_range_px`.
    Range,
}

impl AnchorBy {
    /// The wire's byte (`WireEvent::WellAnchored::by`).
    pub fn byte(self) -> u8 {
        match self {
            AnchorBy::Press => 0,
            AnchorBy::Contact => 1,
            AnchorBy::Range => 2,
        }
    }

    pub fn from_byte(b: u8) -> AnchorBy {
        match b {
            1 => AnchorBy::Contact,
            2 => AnchorBy::Range,
            _ => AnchorBy::Press,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            AnchorBy::Press => "press",
            AnchorBy::Contact => "contact",
            AnchorBy::Range => "range",
        }
    }
}

/// A well standing on the field (`zone::ZoneKind::Well`): its stage, how
/// it was anchored, the seat that fired it, and whether an EMP's ring has
/// called its collapse this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WellZone {
    pub stage: WellStage,
    pub by: AnchorBy,
    pub seat: Option<u8>,
    /// An EMP's ring reached its centre: `Game::well_phase` collapses it
    /// before anything else this tick.
    pub emp_collapse: bool,
}

/// Whether a zone's `until` has come at round time `now`: within half a
/// tick, the rod's rule (`Game::resolve_zones`), so a room and a client
/// agree on the tick.
pub fn due(until: f32, now: f32) -> bool {
    until <= now + PHYSICS_FIXED_DT * 0.5
}

/// Whether `zone` is a well pulling at round time `now`, its stage read off
/// the clock: a forming well whose `until` has come pulls, a pulling one
/// whose `until` has come does not - its collapse is due.
pub fn pulls_at(zone: &Zone, now: f32) -> bool {
    match zone.kind {
        ZoneKind::Well(w) => match w.stage {
            WellStage::Forming => due(zone.until, now),
            WellStage::Pulling => !due(zone.until, now),
        },
        _ => false,
    }
}

/// The pull's strength `d` px from a well's centre, 0..1 (docs/gravity-
/// well.md "The strength"): nothing past the reach, rising linearly to 1 at
/// the core's rim, down to nothing at the centre itself, so a pulled thing
/// settles on it rather than overshooting.
pub fn strength(d: f32, t: &Tuning) -> f32 {
    let r = t.well_radius_px.max(1.0);
    let c = t.well_core_px.clamp(1e-3, r - 1e-3);
    if d >= r {
        0.0
    } else if d >= c {
        (r - d) / (r - c)
    } else {
        (d / c).max(0.0)
    }
}

/// What the wells do to one hull this tick, in the world's axes, already
/// scaled by its mass (`WellField::hull_pull`): the drive splits them by
/// its axis (`simulation::drive_tank_with`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HullPull {
    /// The current (px/s) its tracks roll in, along their axis.
    pub current: Vec2,
    /// The side pull (px/s²) its tracks' grip holds, across their axis.
    pub side: Vec2,
}

impl HullPull {
    pub fn is_zero(&self) -> bool {
        self.current.x == 0.0 && self.current.y == 0.0 && self.side.x == 0.0 && self.side.y == 0.0
    }
}

/// The wells pulling at one round tick: their ids and centres, in id order.
/// Built from `Game::zones` by `at`, so a room, a replica and a sandbox
/// build the same field from the same zones.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WellField {
    pub sources: Vec<(u32, Position)>,
}

impl WellField {
    /// The field at round time `now` from `zones` (`pulls_at`).
    pub fn at(zones: &[Zone], now: f32) -> WellField {
        WellField { sources: zones.iter().filter(|z| pulls_at(z, now)).map(|z| (z.id, z.centre)).collect() }
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// The sum over the sources of the unit vector toward each centre times
    /// its `strength`: zero outside every well. Summed in id order.
    pub fn pull(&self, p: Position, t: &Tuning) -> Vec2 {
        let mut sum = Vec2::zero();
        for &(_, c) in &self.sources {
            let to = c - p;
            let d = to.length();
            if d <= 1e-4 {
                continue;
            }
            let s = strength(d, t);
            if s > 0.0 {
                sum = sum + to * (s / d);
            }
        }
        sum
    }

    /// A hull's current and side pull at `p` for a chassis of mass factor
    /// `mass_factor`: `pull` times `well_current_speed` and
    /// `well_side_pull`, over the factor to `well_mass_exponent`.
    pub fn hull_pull(&self, p: Position, mass_factor: f32, t: &Tuning) -> HullPull {
        if self.sources.is_empty() {
            return HullPull::default();
        }
        let pull = self.pull(p, t);
        if pull.x == 0.0 && pull.y == 0.0 {
            return HullPull::default();
        }
        let resist = mass_factor.max(0.05).powf(t.well_mass_exponent);
        HullPull { current: pull * (t.well_current_speed / resist), side: pull * (t.well_side_pull / resist) }
    }

    /// A shot's acceleration at `p` (px/s²): `pull` times `well_shot_pull`.
    pub fn shot_accel(&self, p: Position, t: &Tuning) -> Vec2 {
        self.pull(p, t) * t.well_shot_pull
    }

    /// The source whose core the stretch `p0..p1` first comes within
    /// `well_core_px` of, and how far along it (0..1): a swallow. Ties to
    /// the lower id.
    pub fn core_hit(&self, p0: Position, p1: Position, t: &Tuning) -> Option<(u32, f32)> {
        let mut best: Option<(u32, f32)> = None;
        for &(id, c) in &self.sources {
            if let Some(f) = segment_enters_disc(p0, p1, c, t.well_core_px)
                && best.is_none_or(|(_, b)| f < b)
            {
                best = Some((id, f));
            }
        }
        best
    }

    /// The source pulling hardest at `p` (its strength there), ties to the
    /// lower id: the well an enemy escapes.
    pub fn strongest(&self, p: Position, t: &Tuning) -> Option<(u32, Position, f32)> {
        let mut best: Option<(u32, Position, f32)> = None;
        for &(id, c) in &self.sources {
            let s = strength(p.distance_to(c), t);
            if s > 0.0 && best.is_none_or(|(_, _, b)| s > b) {
                best = Some((id, c, s));
            }
        }
        best
    }
}

/// How far along `p0..p1` (0..1) it first comes within `radius` of `c`; 0
/// when it starts inside, `None` when it never does.
pub fn segment_enters_disc(p0: Position, p1: Position, c: Position, radius: f32) -> Option<f32> {
    let d = p1 - p0;
    let f = p0 - c;
    let cc = f.x * f.x + f.y * f.y - radius * radius;
    if cc <= 0.0 {
        return Some(0.0);
    }
    let a = d.x * d.x + d.y * d.y;
    if a <= 1e-9 {
        return None;
    }
    let b = 2.0 * (f.x * d.x + f.y * d.y);
    let disc = b * b - 4.0 * a * cc;
    if disc < 0.0 {
        return None;
    }
    let t0 = (-b - disc.sqrt()) / (2.0 * a);
    (0.0..=1.0).contains(&t0).then_some(t0)
}

/// `velocity` turned by `accel` over `dt` and set back to its own speed.
pub fn bend(velocity: Vec2, accel: Vec2, dt: f32) -> Vec2 {
    let speed = velocity.length();
    let turned = velocity + accel * dt;
    let len = turned.length();
    if speed <= 1e-6 || len <= 1e-6 {
        return velocity;
    }
    turned * (speed / len)
}

/// Whether a tank can anchor its orb with this press: it has one in flight,
/// its special is up (an EMP'd tank's press fires a shell, as every offline
/// special's does), and its trigger is still the well's - its wells, or
/// none left at all (the last orb is still pressed down).
pub fn anchor_press(tank: &Tank) -> bool {
    tank.orb.is_some() && !tank.special_down() && matches!(tank.special(), None | Some(ActiveWeapon::GravityWell))
}

/// A drum a well lifted out of its cell (`Game::held_drums`, by id): what
/// it was, where it came from, and when.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeldDrum {
    pub id: u32,
    /// The well's zone id.
    pub well: u32,
    pub cell: (i32, i32),
    pub drum: Drum,
    /// Its fuse's seconds left, if one was burning when it was lifted.
    pub fuse: Option<f32>,
    /// The round clock it was lifted at.
    pub lifted_at: f32,
    /// Its well's centre, which it circles: kept with it, so it is where
    /// it goes off at its well's collapse whatever stands then.
    pub centre: Position,
}

/// Where a held drum is at round time `now` round its well's centre: from
/// its cell spiralling in to the ring over `well_capture_seconds` (eased
/// out), turning clockwise at `well_orbit_speed` from the bearing it was
/// lifted on; and the height it is drawn lifted by.
pub fn held_at(drum: &HeldDrum, centre: Position, now: f32, t: &Tuning) -> (Position, f32) {
    let from = crate::map::cell_to_world(drum.cell.0, drum.cell.1);
    let off = from - centre;
    let r0 = off.length();
    let bearing = off.y.atan2(off.x);
    let age = (now - drum.lifted_at).max(0.0);
    let k = pyro::ease_out(age / t.well_capture_seconds.max(1e-3));
    let r = r0 + (t.well_ring_px - r0) * k;
    let a = bearing + t.well_orbit_speed * age;
    (Position::new(centre.x + a.cos() * r, centre.y + a.sin() * r), HELD_LIFT_PX * pyro::ease_out(age * 4.0))
}

/// A grenade circling a well's ring (`grenade::Grenade::orbit`): the well,
/// the bearing it was caught at and when.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrenadeOrbit {
    pub well: u32,
    pub bearing: f32,
    pub since: f32,
}

/// Where a grenade on the ring is at round time `now`: the ring's radius
/// from the centre, turned clockwise at `well_orbit_speed` from its catch.
pub fn orbit_at(orbit: &GrenadeOrbit, centre: Position, now: f32, t: &Tuning) -> Position {
    let a = orbit.bearing + t.well_orbit_speed * (now - orbit.since).max(0.0);
    Position::new(centre.x + a.cos() * t.well_ring_px, centre.y + a.sin() * t.well_ring_px)
}

/// How many stretches a tracer is drawn curved in through a pull
/// (`curved_streak`).
pub const STREAK_STEPS: usize = 4;

/// The path a shot at `position` flying `velocity` came by over its last
/// `length` px, stepped back through `field` in `STREAK_STEPS` stretches -
/// the curve its tracer is drawn along inside a pull (docs/gravity-well.md
/// "Bent shots"). `None` where the field does not bend it, so the tracer
/// stays the straight one. Head first.
pub fn curved_streak(position: Position, velocity: Vec2, length: f32, field: &WellField, t: &Tuning) -> Option<[Position; STREAK_STEPS + 1]> {
    if field.is_empty() {
        return None;
    }
    let speed = velocity.length();
    if speed <= 1e-3 {
        return None;
    }
    let dt = length / STREAK_STEPS as f32 / speed;
    let mut points = [position; STREAK_STEPS + 1];
    let (mut p, mut v) = (position, velocity);
    let mut bent = false;
    for point in points.iter_mut().skip(1) {
        p = p - v * dt;
        let accel = field.shot_accel(p, t);
        if accel.x != 0.0 || accel.y != 0.0 {
            bent = true;
            v = bend(v, accel * -1.0, dt);
        }
        *point = p;
    }
    bent.then_some(points)
}

/// Whether a hull of mass factor `mass_factor` standing broadside to the
/// pull at `p` holds there: the field's side pull on it (`hull_pull`) is no
/// more than its tracks' grip, `grip` px/s² - `tank_turn_grip_force` times
/// the ground's grip over its mass, the drive's own numbers.
pub fn holds_broadside(field: &WellField, p: Position, mass_factor: f32, grip: f32, t: &Tuning) -> bool {
    field.hull_pull(p, mass_factor, t).side.length() <= grip
}

/// The cardinal an enemy escapes a well along (docs/gravity-well.md "The
/// `pull` tier"): perpendicular to its bearing from the core - the larger
/// offset's axis turned a quarter - on its own side of the core (the
/// smaller offset's sign; on the axis itself the clockwise turn), so its
/// chord runs away from the core; the other perpendicular if that way is
/// shut; straight away if both are; `None` if every one is.
pub fn escape_dir(me: Position, core: Position, open: impl Fn(Dir) -> bool) -> Option<Dir> {
    let off = me - core;
    let (first, second, away) = if off.x.abs() >= off.y.abs() {
        // The core lies along x: across is up or down.
        let away = if off.x >= 0.0 { Dir::Right } else { Dir::Left };
        let toward_own = if off.y > 0.0 {
            Dir::Down
        } else if off.y < 0.0 {
            Dir::Up
        } else if away == Dir::Right {
            Dir::Down
        } else {
            Dir::Up
        };
        (toward_own, toward_own.opposite(), away)
    } else {
        let away = if off.y >= 0.0 { Dir::Down } else { Dir::Up };
        let toward_own = if off.x > 0.0 {
            Dir::Right
        } else if off.x < 0.0 {
            Dir::Left
        } else if away == Dir::Down {
            Dir::Left
        } else {
            Dir::Right
        };
        (toward_own, toward_own.opposite(), away)
    };
    [first, second, away].into_iter().find(|&d| open(d))
}

/// The broadside cardinal across the pull from `core` nearer `prefer` (a
/// bearing to the seat a bracing tank fights), ties to the one nearer its
/// facing.
pub fn brace_dir(me: Position, core: Position, prefer: Option<Position>, facing: Dir) -> Dir {
    let off = me - core;
    let (a, b) = if off.x.abs() >= off.y.abs() { (Dir::Up, Dir::Down) } else { (Dir::Left, Dir::Right) };
    if let Some(p) = prefer {
        let to = p - me;
        let da = a.vec().x * to.x + a.vec().y * to.y;
        let db = b.vec().x * to.x + b.vec().y * to.y;
        if da > db {
            return a;
        }
        if db > da {
            return b;
        }
    }
    if facing == b { b } else { a }
}

/// The farthest point along the line from `centre` through `from`, up to
/// `cells` cells out from `from`, where a frog `fits`, walked back a cell at
/// a time toward `from`; `None` if none fits (docs/gravity-well.md "The
/// collapse": a frog hops out with no jitter).
pub fn frog_fling_target(centre: Position, from: Position, cells: i32, fits: impl Fn(Position) -> bool) -> Option<Position> {
    let off = from - centre;
    let len = off.length();
    let dir = if len > 1e-3 { off / len } else { Vec2::new(0.0, -1.0) };
    (1..=cells.max(1)).rev().map(|k| from + dir * (k as f32 * crate::OBSTACLE_GRID_SIZE)).find(|&p| fits(p))
}

/// The well module's cell (`tank::module_cols`): 1 while it shows a launch,
/// 2 while it shows an anchor, 3 while its special is down, else 0.
pub fn module_cell(tank: &Tank) -> i32 {
    if tank.well_flash > 0.0 {
        return if tank.well_anchor_flash { 2 } else { 1 };
    }
    if tank.special_down() {
        return 3;
    }
    0
}

/// The orb as it is drawn (glowing pass): a disc of `VOID[1]` with a
/// `VOID[3]` disc inside and a `VOID[4]` glint, two motes circling it, a
/// trail of four blocks behind it dissolving, one glow; swelling from the
/// muzzle over its first tenth of a second.
pub fn compose_orb(out: &mut Vec<Shape>, at: Position, velocity: Vec2, age: f32, id: u32) {
    let swell = (age / ORB_SWELL_SECONDS).clamp(0.0, 1.0);
    let r = 2.0 + (ORB_PX - 2.0) * swell;
    let speed = velocity.length();
    let back = if speed > 1e-3 { velocity * (-1.0 / speed) } else { Vec2::new(0.0, 1.0) };
    for k in 0..4 {
        let d = r + 3.0 + k as f32 * 3.0;
        let cover = [1.0, 0.8, 0.5, 0.3][k];
        out.push(Shape::Disc { center: at + back * d, radius: pyro::BLOCK * 0.6, color: VOID[(2usize).saturating_sub(k / 2)], cover });
    }
    out.push(Shape::Disc { center: at, radius: r, color: VOID[1], cover: 1.0 });
    out.push(Shape::Disc { center: at, radius: r * 0.66, color: VOID[3], cover: 1.0 });
    out.push(Shape::Mark { pos: at + Vec2::new(-r * 0.4, -r * 0.4), size: 2, color: VOID[4] });
    let phase = pyro::unit(id, 1) * std::f32::consts::TAU;
    for m in 0..2 {
        let a = phase + age * std::f32::consts::TAU * 2.0 + m as f32 * std::f32::consts::PI;
        out.push(Shape::Mark { pos: at + Vec2::new(a.cos(), a.sin()) * (r + 2.0), size: 2, color: VOID[3] });
    }
    out.push(Shape::Glow { pos: at, radius: 14.0 * (0.5 + 0.5 * swell), color: VOID[1] });
}

/// The snap at an anchor (glowing pass), `age` seconds after it: a ring of
/// `VOID[4]` blocks closing from 32 px onto the point and a white block at
/// it. Gone after `SNAP_SECONDS`.
pub fn compose_snap(out: &mut Vec<Shape>, at: Position, age: f32) {
    if !(0.0..SNAP_SECONDS).contains(&age) {
        return;
    }
    let k = age / SNAP_SECONDS;
    let r = SNAP_FROM_PX * (1.0 - k) + 4.0;
    out.push(Shape::Arc { center: at, radius: r, width: pyro::BLOCK, from: 0.0, to: std::f32::consts::TAU, color: VOID[4], cover: 1.0 - 0.5 * k });
    out.push(Shape::Mark { pos: at, size: 2, color: crate::math::Color::WHITE });
}

/// A well as it is drawn (glowing pass) at round time `now`: by its stage,
/// the core (`SMOKE[0]` inside `VOID[4]`), the accretion ring's two arcs
/// turning clockwise, the swirl of blocks spiralling in, the dotted rim at
/// its reach and one glow; forming, the core grows and the ring dissolves
/// in; over its last second the core beats and the swirl quickens.
pub fn compose_well(out: &mut Vec<Shape>, zone: &Zone, now: f32, t: &Tuning) {
    let ZoneKind::Well(w) = zone.kind else { return };
    let c = zone.centre;
    let (grow, left) = match w.stage {
        WellStage::Forming => {
            let total = t.well_form_seconds.max(1e-3);
            (1.0 - (zone.until - now).clamp(0.0, total) / total, f32::INFINITY)
        }
        WellStage::Pulling => (1.0, (zone.until - now).max(0.0)),
    };
    let warn = left < 1.0;
    let beat = if warn && ((now * 12.0) as i32).rem_euclid(2) == 0 { 1.0 } else { 0.0 };
    let core = (7.0 + beat) * grow.max(0.15);
    let cover = 0.3 + 0.7 * grow;
    compose_swirl(out, c, zone.id, now * if warn { 1.5 } else { 1.0 }, t.well_particles.max(0) as u32, t.well_radius_px, cover);
    let turn = now * RING_SPIN;
    out.push(Shape::Arc { center: c, radius: RING_PX, width: pyro::BLOCK * 2.0, from: turn, to: turn + 4.6, color: VOID[2], cover });
    out.push(Shape::Arc { center: c, radius: RING_PX, width: pyro::BLOCK * 2.0, from: turn + 4.6, to: turn + std::f32::consts::TAU, color: VOID[1], cover });
    out.push(Shape::Disc { center: c, radius: core + 3.0, color: VOID[4], cover: 1.0 });
    // The rim: a dot every 12 px of arc at the reach, turning a turn every
    // eight seconds.
    let r = t.well_radius_px;
    let dots = ((std::f32::consts::TAU * r) / 12.0).max(1.0) as u32;
    let spin = now * std::f32::consts::TAU / 8.0;
    for i in 0..dots {
        let a = spin + i as f32 * std::f32::consts::TAU / dots as f32;
        out.push(Shape::Disc { center: c + Vec2::new(a.cos(), a.sin()) * r, radius: pyro::BLOCK * 0.6, color: VOID[2], cover: 0.6 * grow });
    }
    out.push(Shape::Glow { pos: c, radius: 26.0 * grow, color: VOID[1] });
}

/// A well's core (`compose_well`'s hole), drawn after every light so no
/// glow brightens it: black on any ground under any sky.
pub fn compose_core(out: &mut Vec<Shape>, zone: &Zone, now: f32, t: &Tuning) {
    let ZoneKind::Well(w) = zone.kind else { return };
    let grow = match w.stage {
        WellStage::Forming => {
            let total = t.well_form_seconds.max(1e-3);
            1.0 - (zone.until - now).clamp(0.0, total) / total
        }
        WellStage::Pulling => 1.0,
    };
    let warn = w.stage == WellStage::Pulling && zone.until - now < 1.0;
    let beat = if warn && ((now * 12.0) as i32).rem_euclid(2) == 0 { 1.0 } else { 0.0 };
    out.push(Shape::Disc { center: zone.centre, radius: (7.0 + beat) * grow.max(0.15), color: SMOKE[0], cover: 1.0 });
}

/// The swirl: `count` blocks spiralling clockwise into a well at `c` - each
/// at phase `(0.7 t + hash) mod 1`, its radius falling from 0.6 of the
/// reach to 8 px, the vertical squashed to 0.8 (the ground's tilt) - paler
/// as it falls in.
pub fn compose_swirl(out: &mut Vec<Shape>, c: Position, id: u32, time: f32, count: u32, reach: f32, cover: f32) {
    for i in 0..count {
        let ph = (time * 0.7 + pyro::unit(id, 100 + i)).rem_euclid(1.0);
        let r = 0.6 * reach * (1.0 - ph) + 8.0;
        let a = pyro::unit(id, 300 + 3 * i) * std::f32::consts::TAU + ph * 5.0;
        let color = if ph > 0.7 {
            VOID[4]
        } else if ph > 0.4 {
            VOID[3]
        } else {
            VOID[2]
        };
        let pos = Position::new(c.x + a.cos() * r, c.y + a.sin() * r * 0.8);
        if cover >= 0.999 || pyro::unit(id, 500 + i) < cover {
            out.push(Shape::Mark { pos, size: 2, color });
        }
    }
}

/// The collapse as it is drawn (glowing pass), `age` seconds after it: the
/// implosion's flash, then two rings racing out dissolving, and one glow.
pub fn compose_collapse(out: &mut Vec<Shape>, at: Position, age: f32) {
    if age < IMPLODE_SECONDS {
        out.push(Shape::Disc { center: at, radius: 24.0, color: VOID[4], cover: 1.0 });
        out.push(Shape::Disc { center: at, radius: 10.0, color: crate::math::Color::WHITE, cover: 1.0 });
    }
    if age < COLLAPSE_SECONDS {
        let k = age / COLLAPSE_SECONDS;
        out.push(Shape::Arc { center: at, radius: age * COLLAPSE_SPEEDS.0, width: pyro::BLOCK * 2.0, from: 0.0, to: std::f32::consts::TAU, color: VOID[2], cover: 1.0 - k });
        out.push(Shape::Arc { center: at, radius: age * COLLAPSE_SPEEDS.1, width: pyro::BLOCK, from: 0.0, to: std::f32::consts::TAU, color: VOID[0], cover: 1.0 - k });
    }
    if age < 1.0 / 3.0 {
        out.push(Shape::Glow { pos: at, radius: 80.0 * (1.0 - age * 3.0), color: VOID[1] });
    }
}

/// How long the collapse's dust hangs (s).
const COLLAPSE_DUST_SECONDS: f32 = 0.8;

/// The collapse's dust (lit pass), `age` seconds after it: ten shaded
/// puffs thrown out from half the reach to the reach, leaning `lean` with
/// the wind, thinning out; hashed from the point. Gone after
/// `COLLAPSE_DUST_SECONDS`.
pub fn compose_collapse_dust(out: &mut Vec<Shape>, at: Position, age: f32, lean: f32, t: &Tuning) {
    if !(0.0..COLLAPSE_DUST_SECONDS).contains(&age) {
        return;
    }
    let k = age / COLLAPSE_DUST_SECONDS;
    let seed = crate::blast::seed_at(at, 0x9E11);
    for i in 0..10u32 {
        let a = (i as f32 + pyro::unit(seed, i)) * std::f32::consts::TAU / 10.0;
        let reach = t.well_radius_px * (0.5 + 0.5 * pyro::ease_out(k)) * (0.8 + 0.2 * pyro::unit(seed, 20 + i));
        let pos = Position::new(at.x + a.cos() * reach + lean * age * 20.0, at.y + a.sin() * reach * 0.8 - age * 6.0);
        out.push(Shape::Puff(pyro::dust_puff(pos, 6.0 + 4.0 * k, k, 1.0 - k)));
    }
}

/// How long the collapse's picture runs (s).
pub fn collapse_seconds() -> f32 {
    COLLAPSE_SECONDS
}

/// A swallow's pop (glowing pass), `age` seconds after it: a ring of
/// `VOID[3]` blocks closing onto the point and one `VOID[4]` block.
pub fn compose_swallow(out: &mut Vec<Shape>, at: Position, age: f32) {
    if !(0.0..SWALLOW_SECONDS).contains(&age) {
        return;
    }
    let k = age / SWALLOW_SECONDS;
    out.push(Shape::Arc { center: at, radius: 12.0 * (1.0 - k) + 2.0, width: pyro::BLOCK, from: 0.0, to: std::f32::consts::TAU, color: VOID[3], cover: 1.0 });
    out.push(Shape::Mark { pos: at, size: 2, color: VOID[4] });
}

/// The colour a held drum's lift light is drawn in.
pub fn held_glow() -> Color {
    VOID[2]
}

/// What reached a well's core and was swallowed (`Event::Swallowed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Swallow {
    Shell,
    Bullet,
    Plasma,
    Orb,
    Missile,
    Drone,
}

impl Swallow {
    pub const ALL: [Swallow; 6] = [Swallow::Shell, Swallow::Bullet, Swallow::Plasma, Swallow::Orb, Swallow::Missile, Swallow::Drone];

    pub fn name(self) -> &'static str {
        match self {
            Swallow::Shell => "shell",
            Swallow::Bullet => "bullet",
            Swallow::Plasma => "plasma",
            Swallow::Orb => "orb",
            Swallow::Missile => "missile",
            Swallow::Drone => "drone",
        }
    }
}

/// A well's moment as it is drawn - the snap at an anchor, the collapse, a
/// swallow's pop - aged in `tick_effects`, the same on the room and on a
/// replica (staged by their events).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WellFx {
    pub kind: WellFxKind,
    pub at: Position,
    pub age: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WellFxKind {
    Snap,
    Collapse,
    Swallow,
}

impl WellFx {
    /// Whether its picture is over.
    pub fn done(&self) -> bool {
        self.age
            > match self.kind {
                WellFxKind::Snap => 0.2,
                WellFxKind::Collapse => collapse_seconds().max(COLLAPSE_DUST_SECONDS),
                WellFxKind::Swallow => SWALLOW_SECONDS,
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuning::Tuning;

    fn well_at(id: u32, at: Position, stage: WellStage, until: f32) -> Zone {
        Zone {
            id,
            kind: ZoneKind::Well(WellZone { stage, by: AnchorBy::Press, seat: Some(0), emp_collapse: false }),
            owner: Owner::Player(0),
            centre: at,
            until,
        }
    }

    #[test]
    fn strength_peaks_at_the_cores_rim_and_falls_to_the_edge() {
        let t = Tuning::DEFAULT;
        assert_eq!(strength(t.well_core_px, &t), 1.0);
        assert_eq!(strength(t.well_radius_px, &t), 0.0);
        assert_eq!(strength(t.well_radius_px + 10.0, &t), 0.0);
        assert_eq!(strength(0.0, &t), 0.0);
        assert!(strength(t.well_core_px * 0.5, &t) > 0.4 && strength(t.well_core_px * 0.5, &t) < 0.6);
        let mid = (t.well_core_px + t.well_radius_px) * 0.5;
        assert!((strength(mid, &t) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn the_field_reads_the_stage_off_the_clock_and_sums_in_id_order() {
        let t = Tuning::DEFAULT;
        let forming = well_at(2, Position::new(200.0, 200.0), WellStage::Forming, 1.0);
        let pulling = well_at(5, Position::new(260.0, 200.0), WellStage::Pulling, 3.0);
        assert!(WellField::at(&[forming], 0.5).is_empty(), "forming pulls nothing");
        assert_eq!(WellField::at(&[forming], 1.0).sources.len(), 1, "its pull starts on its until");
        assert!(WellField::at(&[pulling], 3.0).is_empty(), "a due collapse pulls nothing");
        let field = WellField::at(&[forming, pulling], 2.0);
        assert_eq!(field.sources.iter().map(|s| s.0).collect::<Vec<_>>(), vec![2, 5]);
        // Between the two, the pulls cancel; outside both, nothing.
        let between = field.pull(Position::new(230.0, 200.0), &t);
        assert!(between.length() < 1e-4, "{between:?}");
        assert_eq!(field.pull(Position::new(900.0, 900.0), &t), Vec2::zero());
    }

    #[test]
    fn bend_keeps_the_speed_and_turns_toward_the_pull() {
        let v = Vec2::new(500.0, 0.0);
        let b = bend(v, Vec2::new(0.0, 3600.0), 1.0 / 60.0);
        assert!((b.length() - 500.0).abs() < 1e-3);
        assert!(b.y > 0.0 && b.x > 0.0);
        assert_eq!(bend(v, Vec2::zero(), 1.0 / 60.0), v);
    }

    #[test]
    fn hull_pull_scales_by_the_mass() {
        let t = Tuning::DEFAULT;
        let field = WellField { sources: vec![(1, Position::new(100.0, 100.0))] };
        let p = Position::new(100.0 + t.well_core_px, 100.0);
        let std = field.hull_pull(p, 1.0, &t);
        assert!((std.current.x + t.well_current_speed).abs() < 1e-3, "{std:?}");
        let heavy = field.hull_pull(p, 2.0, &t);
        assert!((heavy.current.x * 4.0 - std.current.x).abs() < 1e-3);
        assert!(WellField::default().hull_pull(p, 1.0, &t).is_zero());
    }

    #[test]
    fn core_hit_meets_the_core_between_ticks() {
        let t = Tuning::DEFAULT;
        let field = WellField { sources: vec![(1, Position::new(100.0, 100.0))] };
        let hit = field.core_hit(Position::new(80.0, 100.0), Position::new(120.0, 101.0), &t).expect("through the core");
        assert!(hit.1 > 0.0 && hit.1 < 0.5);
        assert!(field.core_hit(Position::new(80.0, 140.0), Position::new(120.0, 140.0), &t).is_none());
    }

    #[test]
    fn escape_dir_is_across_on_the_hulls_side() {
        let core = Position::new(0.0, 0.0);
        assert_eq!(escape_dir(Position::new(40.0, 10.0), core, |_| true), Some(Dir::Down));
        assert_eq!(escape_dir(Position::new(40.0, -10.0), core, |_| true), Some(Dir::Up));
        assert_eq!(escape_dir(Position::new(-5.0, -40.0), core, |_| true), Some(Dir::Left));
        assert_eq!(escape_dir(Position::new(40.0, 10.0), core, |d| d != Dir::Down), Some(Dir::Up));
        assert_eq!(escape_dir(Position::new(40.0, 10.0), core, |d| d == Dir::Right), Some(Dir::Right));
        assert_eq!(escape_dir(Position::new(40.0, 10.0), core, |_| false), None);
    }

    #[test]
    fn curved_streak_follows_the_bend() {
        let t = Tuning::DEFAULT;
        let empty = WellField::default();
        let at = Position::new(300.0, 300.0);
        let v = Vec2::new(500.0, 0.0);
        assert!(curved_streak(at, v, 40.0, &empty, &t).is_none(), "no well, the straight tracer");
        let field = WellField { sources: vec![(1, Position::new(300.0, 340.0))] };
        let path = curved_streak(at, v, 40.0, &field, &t).expect("bent in the pull");
        assert_eq!(path[0], at, "head first");
        assert!(path[STREAK_STEPS].x < at.x - 30.0, "it came from behind");
        assert!(path.iter().skip(1).any(|p| (p.y - at.y).abs() > 0.01), "off the straight line");
        let far = WellField { sources: vec![(1, Position::new(900.0, 900.0))] };
        assert!(curved_streak(at, v, 40.0, &far, &t).is_none(), "out of the reach, straight");
    }

    #[test]
    fn held_at_spirals_in_then_circles_on_the_ring() {
        let t = Tuning::DEFAULT;
        let centre = Position::new(320.0, 320.0);
        let drum = HeldDrum { id: 4, well: 1, cell: crate::map::world_to_cell(Position::new(384.0, 320.0)), drum: Drum::Oil, fuse: None, lifted_at: 1.0, centre };
        let (start, h0) = held_at(&drum, centre, 1.0, &t);
        assert!(start.distance_to(centre) > 50.0 && h0 == 0.0);
        let (later, _) = held_at(&drum, centre, 1.0 + t.well_capture_seconds + 0.5, &t);
        assert!((later.distance_to(centre) - t.well_ring_px).abs() < 0.01);
        let orbit = GrenadeOrbit { well: 1, bearing: 0.0, since: 2.0 };
        assert!((orbit_at(&orbit, centre, 3.0, &t).distance_to(centre) - t.well_ring_px).abs() < 1e-3);
    }

    #[test]
    fn frog_fling_target_walks_back_to_a_fit() {
        let c = Position::new(0.0, 0.0);
        let from = Position::new(32.0, 0.0);
        assert_eq!(frog_fling_target(c, from, 3, |_| true), Some(Position::new(128.0, 0.0)));
        assert_eq!(frog_fling_target(c, from, 3, |p| p.x < 100.0), Some(Position::new(96.0, 0.0)));
        assert_eq!(frog_fling_target(c, from, 3, |_| false), None);
    }

    #[test]
    fn the_pictures_are_on_the_grid_in_the_void_ramp_and_gone_by_their_end() {
        let t = Tuning::DEFAULT;
        let zone = well_at(3, Position::new(200.0, 200.0), WellStage::Pulling, 5.0);
        let mut shapes = Vec::new();
        compose_well(&mut shapes, &zone, 3.0, &t);
        assert!(!shapes.is_empty());
        let mut again = Vec::new();
        compose_well(&mut again, &zone, 3.0, &t);
        assert_eq!(shapes, again, "pure");
        let mut gone = Vec::new();
        compose_collapse(&mut gone, Position::new(1.0, 1.0), 1.0);
        compose_snap(&mut gone, Position::new(1.0, 1.0), 1.0);
        compose_swallow(&mut gone, Position::new(1.0, 1.0), 1.0);
        assert!(gone.is_empty());
        let mut orb = Vec::new();
        compose_orb(&mut orb, Position::new(50.0, 50.0), Vec2::new(150.0, 0.0), 0.5, 7);
        for s in &orb {
            if let Shape::Disc { color, .. } | Shape::Mark { color, .. } = s {
                assert!(VOID.contains(color), "{color:?} is a VOID step");
            }
        }
    }
}
