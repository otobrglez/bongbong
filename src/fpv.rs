//! The FPV swarm (docs/fpv-swarm.md), its headless half: the drone and its
//! flight, the halo's geometry, and the drawing in the effects language.
//! `simulation/fpv.rs` is the world half - the launch, the locks, the
//! bursts, what strikes a drone in the air and what the AI is handed.
//!
//! A crate loads six drones into a halo over the tank. The halo is the
//! stock, drawn - `Tank::fpv_drones` of `full_load` fixed slots round the
//! hull (`halo_slot`) - and nothing in it is in the world. A press sends the
//! top slot's drone up: it climbs out along its slot's bearing (`Launch`,
//! the same whatever it is after - `launch_path`), cruises over everything
//! toward its aim (`Cruise`, tracking its lock), commits within
//! `fpv_commit_px` and comes down on the point it committed to (`Dive`),
//! and bursts there. Struck in the air it falls (`Falling`) and lands a
//! dud. Like a missile it works on a ground point and a height, the drawing
//! lifting it over its shadow. No RNG anywhere: the flight is a pure
//! function of its state, its aim, the wind and the step.

use crate::air::{AirKey, AirStrike, AirTarget};
use crate::math::{Color, Vec2};
use crate::pyro::{self, Shape};
use crate::shell::Owner;
use crate::tank::Tank;
use crate::tuning::{Tuning, tuning};
use crate::{PHYSICS_FIXED_DT, Position};

/// The bearing of the halo's first slot (degrees, 0 = up, clockwise): the
/// slots stand at this plus `k * 360 / n`, so none sits on the gun line.
pub const FPV_HALO_START_DEG: f32 = 30.0;

/// How far from the hull's centre the halo's ring stands, against the
/// tank's sprite size: 44 px for a standard chassis, outside the hull.
pub const FPV_HALO_RADIUS_FRACTION: f32 = 0.55;

/// How high a halo drone hovers (px), and where a drone's climb starts.
pub const FPV_HALO_HEIGHT_PX: f32 = 14.0;

/// How long a wrecked tank's halo takes to fall to the ground (seconds).
pub const FPV_HALO_FALL_SECONDS: f32 = 0.5;

/// How long a rotor blade holds one of its two blocks (seconds).
pub const FPV_ROTOR_FRAME_SECONDS: f32 = 0.05;

/// Below this height (px) a drone in the air throws rotor wash off the
/// ground under it (`fx.rs`).
pub const FPV_WASH_HEIGHT_PX: f32 = 24.0;

/// The side of a drone's quad, in 2 px blocks.
const QUAD_BLOCKS: i32 = 4;

/// Where a drone is in its flight - see this module's doc comment. The
/// variant order is the wire's encoding (`net::wire::drone_stage`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DroneStage {
    Launch,
    Cruise,
    Dive,
    Falling,
}

impl DroneStage {
    pub const ALL: [DroneStage; 4] = [DroneStage::Launch, DroneStage::Cruise, DroneStage::Dive, DroneStage::Falling];

    /// The byte it travels as.
    pub fn code(self) -> u8 {
        DroneStage::ALL.iter().position(|&s| s == self).expect("every stage is in ALL") as u8
    }

    /// Inverse of `code`; `None` past the last.
    pub fn from_code(code: u8) -> Option<DroneStage> {
        DroneStage::ALL.get(code as usize).copied()
    }

    /// Lower-case name for tooling.
    pub fn name(self) -> &'static str {
        match self {
            DroneStage::Launch => "launch",
            DroneStage::Cruise => "cruise",
            DroneStage::Dive => "dive",
            DroneStage::Falling => "falling",
        }
    }
}

/// What a drone is locked on: picked at its launch and kept until it is
/// lost (docs/fpv-swarm.md "Which target").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DroneLock {
    /// Nothing: it dives on its aim point.
    None,
    /// A tank, by its entity and owner slot.
    Tank { entity: hecs::Entity, slot: usize },
    /// A frog: the players' (`Side::Player`) or the Hunt mission's enemy
    /// frog.
    Frog { entity: hecs::Entity, side: crate::frog::Side },
}

/// `DroneLock::code` of a drone locked on nothing.
pub const LOCK_NONE: u16 = u16::MAX;
/// `DroneLock::code` of a drone locked on the players' frog.
pub const LOCK_FROG: u16 = u16::MAX - 1;
/// `DroneLock::code` of a drone locked on the Hunt mission's enemy frog.
pub const LOCK_ENEMY_FROG: u16 = u16::MAX - 2;

impl DroneLock {
    /// The owner slot of a locked tank.
    pub fn slot(self) -> Option<usize> {
        match self {
            DroneLock::Tank { slot, .. } => Some(slot),
            _ => None,
        }
    }

    /// What it is locked on as one number, the wire's
    /// (`net::wire::DroneState::lock`): a tank's owner slot, `LOCK_FROG`,
    /// `LOCK_ENEMY_FROG` or `LOCK_NONE`.
    pub fn code(self) -> u16 {
        match self {
            DroneLock::None => LOCK_NONE,
            DroneLock::Tank { slot, .. } => slot.min(LOCK_ENEMY_FROG as usize - 1) as u16,
            DroneLock::Frog { side: crate::frog::Side::Player, .. } => LOCK_FROG,
            DroneLock::Frog { side: crate::frog::Side::Enemy, .. } => LOCK_ENEMY_FROG,
        }
    }
}

/// What an enemy's drone is to lock (`ai::SpecialUse::Launch`): the seat
/// its rule chose, the players' frog, or a point - a tree's crown - with no
/// lock. A seat's press asks for `Nearest`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AirWant {
    Nearest,
    Seat(u8),
    Frog,
    Point(Position),
}

/// One drone in the air.
#[derive(Clone, Debug)]
pub struct Drone {
    /// Per-round id from `Game::take_shot_id`, the one projectile counter:
    /// its wire key and its walk order.
    pub id: u32,
    /// Who launched it: its side, its lamp's colour, its kills.
    pub owner: Owner,
    pub stage: DroneStage,
    /// The point on the ground under it.
    pub ground: Position,
    /// Its height over that point (px).
    pub height: f32,
    /// Unit heading over the ground.
    pub heading: Vec2,
    /// Ground speed (px/s).
    pub speed: f32,
    /// Seconds since its launch.
    pub age: f32,
    /// Seconds in the current stage.
    pub stage_time: f32,
    /// The halo slot's ground point it climbed out of, and that slot's
    /// outward bearing: its launch path (`launch_path`).
    pub origin: Position,
    pub out: Vec2,
    /// Which halo slot it left: a salt for its rotors and lamp.
    pub slot: u8,
    pub lock: DroneLock,
    /// Where it is going: its lock's centre while it tracks one, the point
    /// it committed to once diving.
    pub aim: Position,
    /// The dive: where it started, how long it takes, the height it started
    /// at and how far the wind has carried it since.
    pub dive_from: Position,
    pub dive_seconds: f32,
    pub dive_height: f32,
    pub drift: Vec2,
    /// Bullets it has taken (`fpv_drone_hits` bring it down).
    pub hits: i32,
    /// A falling drone's ground velocity and its fall speed.
    pub fall_velocity: Vec2,
    pub fall_speed: f32,
    /// What brought it down, once something has.
    pub downed_by: Option<AirStrike>,
    /// Set the step it reaches the ground: its burst (a dive's end) or its
    /// crash (a fall's); the frame's `resolve_drones` takes it away.
    pub landed: bool,
}

impl Drone {
    /// A drone leaving halo slot `slot` (its ground point `origin`, its
    /// outward bearing `out`), locked on `lock` with `aim` its first aim.
    pub fn launch(origin: Position, out: Vec2, slot: u8, owner: Owner, lock: DroneLock, aim: Position) -> Drone {
        let t = tuning();
        Drone {
            id: 0,
            owner,
            stage: DroneStage::Launch,
            ground: origin,
            height: FPV_HALO_HEIGHT_PX,
            heading: out,
            speed: t.fpv_launch_speed,
            age: 0.0,
            stage_time: 0.0,
            origin,
            out,
            slot,
            lock,
            aim,
            dive_from: origin,
            dive_seconds: 0.0,
            dive_height: 0.0,
            drift: Vec2::zero(),
            hits: 0,
            fall_velocity: Vec2::zero(),
            fall_speed: 0.0,
            downed_by: None,
            landed: false,
        }
    }

    /// Its heading over the ground as degrees clockwise from up (0 = up):
    /// the wire's spelling.
    pub fn heading_degrees(&self) -> f32 {
        self.heading.x.atan2(-self.heading.y).to_degrees()
    }

    /// The unit heading `heading_degrees` names.
    pub fn heading_of(degrees: f32) -> Vec2 {
        let rad = degrees.to_radians();
        Vec2::new(rad.sin(), -rad.cos())
    }

    /// Run a replica's copy's clocks on by `dt` (`Game::tick_presentation`):
    /// its rotors and its lamp turn on them. The room's snapshot is what
    /// moves it.
    pub fn age_by(&mut self, dt: f32) {
        self.age += dt;
        self.stage_time += dt;
    }

    /// In the air and strikable: launching, cruising or diving.
    pub fn in_air(&self) -> bool {
        self.stage != DroneStage::Falling && !self.landed
    }

    /// Still following its lock: launching or cruising with one.
    pub fn tracking(&self) -> bool {
        matches!(self.stage, DroneStage::Launch | DroneStage::Cruise) && self.lock != DroneLock::None
    }

    /// Its ground velocity (px/s) this step.
    pub fn velocity(&self) -> Vec2 {
        match self.stage {
            DroneStage::Launch | DroneStage::Cruise => self.heading * self.speed,
            DroneStage::Dive => {
                if self.dive_seconds > 0.0 { (self.aim - self.dive_from) * (1.0 / self.dive_seconds) } else { Vec2::zero() }
            }
            DroneStage::Falling => self.fall_velocity,
        }
    }

    /// This drone as an air target.
    pub fn as_target(&self) -> AirTarget {
        AirTarget {
            key: AirKey::Drone(self.id),
            owner: self.owner,
            ground: self.ground,
            height: self.height,
            velocity: self.velocity(),
            half: tuning().fpv_hit_half_px,
        }
    }

    /// Where it is drawn: the ground point lifted by its height.
    pub fn drawn(&self) -> Position {
        Position::new(self.ground.x, self.ground.y - self.height)
    }

    /// Stop tracking and come down on `at`: the dive takes the distance
    /// over `fpv_dive_speed`, at least a tick, its height falling to 0.
    pub fn commit(&mut self, at: Position) {
        let t = tuning();
        self.aim = at;
        self.dive_from = self.ground;
        self.dive_seconds = (self.ground.distance_to(at) / t.fpv_dive_speed.max(1.0)).max(PHYSICS_FIXED_DT);
        self.dive_height = self.height;
        self.drift = Vec2::zero();
        self.enter(DroneStage::Dive);
    }

    /// Struck in the air: it falls from where it is, keeping its ground
    /// motion, no longer an air target.
    pub fn down(&mut self, by: AirStrike) {
        if self.stage == DroneStage::Falling {
            return;
        }
        self.fall_velocity = self.velocity();
        self.fall_speed = 0.0;
        self.downed_by = Some(by);
        self.enter(DroneStage::Falling);
    }

    fn enter(&mut self, stage: DroneStage) {
        self.stage = stage;
        self.stage_time = 0.0;
    }

    /// One fixed step: `wind` is the ground flow a gust carries it by
    /// (px/s, `fpv_gust_factor` already applied; zero but in a sandstorm),
    /// `field` the field's size its ground point is held inside. Pure.
    pub fn advance(&mut self, dt: f32, wind: Vec2, field: (f32, f32)) {
        if self.landed {
            return;
        }
        let t = tuning();
        self.age += dt;
        self.stage_time += dt;
        match self.stage {
            DroneStage::Launch => {
                let (ground, height) = launch_path_with(&t, self.origin, self.out, self.stage_time);
                self.ground = ground;
                self.height = height;
                self.heading = self.out;
                self.speed = t.fpv_launch_speed;
                if self.stage_time >= t.fpv_launch_seconds {
                    self.enter(DroneStage::Cruise);
                    if self.ground.distance_to(self.aim) <= t.fpv_commit_px {
                        self.commit(self.aim);
                    }
                }
            }
            DroneStage::Cruise => {
                self.speed = (self.speed + t.fpv_accel * dt).min(t.fpv_speed);
                self.turn_toward_aim(t.fpv_turn_rate_deg, dt);
                self.ground = self.ground + self.heading * (self.speed * dt) + wind * dt;
                self.height = t.fpv_cruise_height;
                if self.ground.distance_to(self.aim) <= t.fpv_commit_px {
                    self.commit(self.aim);
                } else if self.age >= t.fpv_max_flight_seconds {
                    let ahead = self.ground + self.heading * t.fpv_commit_px;
                    self.commit(clamp_inside(ahead, field, crate::OBSTACLE_GRID_SIZE * 0.5));
                }
            }
            DroneStage::Dive => {
                self.drift = self.drift + wind * dt;
                let k = (self.stage_time / self.dive_seconds.max(1e-4)).min(1.0);
                self.ground = self.dive_from + (self.aim - self.dive_from) * k + self.drift;
                self.height = self.dive_height * (1.0 - k);
                if k >= 1.0 {
                    self.height = 0.0;
                    self.landed = true;
                }
            }
            DroneStage::Falling => {
                self.fall_speed += t.fpv_fall_gravity * dt;
                self.height -= self.fall_speed * dt;
                self.ground = self.ground + self.fall_velocity * dt;
                self.fall_velocity = self.fall_velocity * (-t.fpv_fall_drag * dt).exp();
                if self.height <= 0.0 {
                    self.height = 0.0;
                    self.landed = true;
                }
            }
        }
        self.ground = clamp_inside(self.ground, field, 0.0);
    }

    /// Rotate `heading` toward `aim` by at most `rate_deg` degrees a
    /// second.
    fn turn_toward_aim(&mut self, rate_deg: f32, dt: f32) {
        let to = self.aim - self.ground;
        if to.length() < 1e-3 {
            return;
        }
        let want = to.y.atan2(to.x);
        let have = self.heading.y.atan2(self.heading.x);
        let mut delta = want - have;
        while delta > std::f32::consts::PI {
            delta -= std::f32::consts::TAU;
        }
        while delta < -std::f32::consts::PI {
            delta += std::f32::consts::TAU;
        }
        let max = rate_deg.to_radians() * dt;
        let turned = have + delta.clamp(-max, max);
        self.heading = Vec2::new(turned.cos(), turned.sin());
    }
}

/// `p` held `margin` px inside a field of `field`'s size.
pub fn clamp_inside(p: Position, (w, h): (f32, f32), margin: f32) -> Position {
    Position::new(p.x.clamp(margin, (w - margin).max(margin)), p.y.clamp(margin, (h - margin).max(margin)))
}

/// A unit vector along `bearing` (degrees, 0 = up, clockwise).
fn bearing_dir(bearing: f32) -> Vec2 {
    let r = bearing.to_radians();
    Vec2::new(r.sin(), -r.cos())
}

/// Halo slot `k` of `n` round a hull at `centre` whose sprite is
/// `sprite_size` across: its ground point and its outward bearing. Fixed in
/// the world - neither orbiting nor turning with the hull.
pub fn halo_slot(centre: Position, sprite_size: f32, k: usize, n: usize) -> (Position, Vec2) {
    let n = n.max(1);
    let out = bearing_dir(FPV_HALO_START_DEG + k as f32 * 360.0 / n as f32);
    (centre + out * (sprite_size * FPV_HALO_RADIUS_FRACTION), out)
}

/// How far into its climb (s) a drone a client drew on its own press is
/// handed to the room's copy (docs/fpv-swarm.md "Wire"): never past
/// `fpv_launch_seconds`, so the two meet on the climb's one path.
pub const FPV_LAUNCH_HANDOVER_SECONDS: f32 = 0.25;

/// Where a drone `age` seconds into its climb stands: out from its slot's
/// ground point `origin` along `out` at `fpv_launch_speed`, its height
/// easing out from the halo's to `fpv_cruise_height` - the same whatever
/// it is after, which is what lets a client draw a launch on the press
/// (docs/fpv-swarm.md "Wire").
pub fn launch_path(origin: Position, out: Vec2, age: f32) -> (Position, f32) {
    launch_path_with(&tuning(), origin, out, age)
}

fn launch_path_with(t: &Tuning, origin: Position, out: Vec2, age: f32) -> (Position, f32) {
    let span = t.fpv_launch_seconds.max(1e-3);
    let age = age.clamp(0.0, span);
    let k = pyro::ease_out(age / span);
    (origin + out * (t.fpv_launch_speed * age), FPV_HALO_HEIGHT_PX + (t.fpv_cruise_height - FPV_HALO_HEIGHT_PX) * k)
}

/// A candidate lock: an owner slot (or a frog's tie key) and where it
/// stands.
#[derive(Clone, Copy, Debug)]
pub struct LockCandidate {
    pub key: usize,
    pub pos: Position,
}

/// The nearest candidate to `from`, ties to the lower key - the rule every
/// lock and the AI's sense pick by. Its index in `candidates`.
pub fn pick_nearest(from: Position, candidates: &[LockCandidate]) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| from.distance_to(a.pos).total_cmp(&from.distance_to(b.pos)).then(a.key.cmp(&b.key)))
        .map(|(i, _)| i)
}

/// A tree's crown as a box (centre, half extents): its cell grown by
/// `fpv_canopy_px` on every side - the crown it is drawn with, and a few
/// pixels more.
pub fn crown_box(tree: Position, canopy_px: f32) -> (Position, Vec2) {
    let half = crate::OBSTACLE_GRID_SIZE * 0.5 + canopy_px;
    (tree, Vec2::new(half, half))
}

/// Whether two boxes (centre, half extents) overlap.
pub fn boxes_overlap(a: (Position, Vec2), b: (Position, Vec2)) -> bool {
    (a.0.x - b.0.x).abs() < a.1.x + b.1.x && (a.0.y - b.0.y).abs() < a.1.y + b.1.y
}

/// Whether `p` lies inside the box (centre, half extents).
pub fn box_holds(b: (Position, Vec2), p: Position) -> bool {
    (p.x - b.0.x).abs() <= b.1.x && (p.y - b.0.y).abs() <= b.1.y
}

/// A drone's lamp colour: a seat's team colour (each seat its own in
/// co-op), an enemy's hostile red.
pub fn lamp_color(owner: Owner) -> Color {
    match owner {
        Owner::Player(seat) => crate::tank::team_color(seat),
        _ => crate::indicators::HOSTILE,
    }
}

/// Whether a lamp blinking at `hz` with `phase` (0..1) is on at `time`.
pub fn lamp_on(time: f32, hz: f32, phase: f32) -> bool {
    (time * hz + phase).rem_euclid(1.0) < 0.5
}

/// The FPV relay module's cell (`tank::module_cols`, offset from
/// `TANK_MODULE_FPV_COL`) `tank` shows at `time`: 2 a launch, 3 offline,
/// 1 and 0 alternating at 4 Hz while one of its drones is in the air, else
/// 0.
pub fn module_cell(tank: &Tank, time: f32) -> i32 {
    if tank.fpv_flash > 0.0 {
        return 2;
    }
    if tank.special_down() {
        return 3;
    }
    if tank.fpv_out > 0 {
        return ((time * 4.0 * 2.0) as i32).rem_euclid(2);
    }
    0
}

/// A drone's picture as plain blocks: its body (alpha-blended, the lit
/// pass) and its lamp (drawn in the glowing pass, so it reads at night).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DronePicture {
    pub body: Vec<Shape>,
    pub lamps: Vec<Shape>,
}

/// How a drone is drawn this frame (`compose_drone`).
#[derive(Clone, Copy, Debug)]
pub struct DroneLook {
    /// Where it is drawn (its ground point lifted by its height).
    pub at: Position,
    /// Its hash salt: the drone's id, or the halo slot's.
    pub seed: u32,
    /// The clock its rotors and lamp run on.
    pub time: f32,
    /// Its lamp's colour while on; `None` dark.
    pub lamp: Option<Color>,
    /// The lamp's blink rate (Hz).
    pub lamp_hz: f32,
    /// The rotors turn.
    pub rotors: bool,
    /// Diving along this heading: two blocks streak behind it.
    pub dive: Option<Vec2>,
    /// Falling: its silhouette turns between an X and a + at this rate.
    pub spin_hz: Option<f32>,
    /// Its blocks' side (px): `pyro::BLOCK` in a halo and low, twice that
    /// up at cruise height (`drone_block`), where it is nearer the eye.
    pub block: f32,
}

/// The side (px) of an airborne drone's blocks at `height`: twice
/// `pyro::BLOCK` above `FPV_WASH_HEIGHT_PX` - up where it cruises it is
/// nearer the eye, as a missile's sprite grows with its height, and a
/// drone must read as the threat it is - else `pyro::BLOCK`, a halo's.
pub fn drone_block(height: f32) -> f32 {
    if height > FPV_WASH_HEIGHT_PX { pyro::BLOCK * 2.0 } else { pyro::BLOCK }
}

const FRAME: Color = pyro::SMOKE[0];
const BODY: Color = pyro::SMOKE[2];
const BODY_LIT: Color = pyro::SMOKE[3];
const ROTOR: Color = pyro::SMOKE[5];
const LAMP_OFF: Color = pyro::SMOKE[1];
const STREAK: Color = pyro::SMOKE[4];

/// One drone: a 4 x 4-block quad on the 2 px grid - its frame the two
/// diagonals (the corner blocks and the middle four) in the tanks' outline,
/// its body the middle 2 x 2 with its top-left block lit, a rotor blade at
/// each corner stepping between the two blocks beside it, its lamp the
/// body's bottom-right block blinking in `look.lamp`. A falling drone turns
/// between that X and a + silhouette with its rotors still. Pure.
pub fn compose_drone(look: &DroneLook) -> DronePicture {
    let mut pic = DronePicture::default();
    let block = look.block.max(pyro::BLOCK);
    let (bx, by) = ((look.at.x / block).floor() as i32, (look.at.y / block).floor() as i32);
    let (x0, y0) = (bx - QUAD_BLOCKS / 2, by - QUAD_BLOCKS / 2);
    let at = |i: i32, j: i32| Position::new(((x0 + i) as f32 + 0.5) * block, ((y0 + j) as f32 + 0.5) * block);
    let size = block as i32;
    let mark = |pos: Position, color: Color| Shape::Mark { pos, size, color };
    let plus = look.spin_hz.is_some_and(|hz| ((look.time * hz) as i32).rem_euclid(2) == 1);
    let arms: [(i32, i32); 4] = if plus { [(1, 0), (0, 2), (3, 1), (2, 3)] } else { [(0, 0), (3, 0), (0, 3), (3, 3)] };
    for (i, j) in arms {
        pic.body.push(mark(at(i, j), FRAME));
    }
    pic.body.push(mark(at(1, 1), BODY_LIT));
    pic.body.push(mark(at(2, 1), BODY));
    pic.body.push(mark(at(1, 2), BODY));
    if look.rotors && !plus {
        // Each corner's blade steps between the blocks either side of it,
        // the four a quarter turn apart and every drone on its own phase.
        let frame = (look.time / FPV_ROTOR_FRAME_SECONDS) as i32 + (pyro::unit(look.seed, 1) * 4.0) as i32;
        let blades = [((1, 0), (0, 1)), ((2, 0), (3, 1)), ((3, 2), (2, 3)), ((0, 2), (1, 3))];
        for (k, (a, b)) in blades.into_iter().enumerate() {
            let (i, j) = if (frame + k as i32).rem_euclid(2) == 0 { a } else { b };
            pic.body.push(mark(at(i, j), ROTOR));
        }
    }
    let phase = pyro::unit(look.seed, 2);
    match look.lamp {
        Some(color) if lamp_on(look.time, look.lamp_hz, phase) => pic.lamps.push(mark(at(2, 2), color)),
        _ => pic.body.push(mark(at(2, 2), LAMP_OFF)),
    }
    if let Some(dir) = look.dive {
        let len = dir.length();
        if len > 1e-3 {
            let d = dir * (1.0 / len);
            pic.body.push(mark(look.at - d * (3.0 * block), STREAK));
            pic.body.push(mark(look.at - d * (5.0 * block), STREAK));
        }
    }
    pic
}

/// A drone's shadow on the ground under it: a 3 x 3-block square of black
/// at `opacity`, a block smaller from 24 px up, shifted 4 px along
/// `shadow_dir` (the missiles' rule).
pub fn compose_shadow(ground: Position, height: f32, opacity: f32, shadow_dir: Vec2) -> Vec<Shape> {
    let a = (opacity.clamp(0.0, 1.0) * 255.0) as u8;
    if a == 0 {
        return Vec::new();
    }
    let at = ground + shadow_dir * 4.0;
    let size = if height >= FPV_WASH_HEIGHT_PX { 4 } else { 6 };
    vec![Shape::Mark { pos: at, size, color: Color::new(0, 0, 0, a) }]
}

/// How the halo of one tank is drawn (`compose_halo`).
#[derive(Clone, Copy, Debug)]
pub struct HaloLook {
    pub centre: Position,
    pub sprite_size: f32,
    /// Drones left in it, and its slots (`full_load`).
    pub drones: usize,
    pub slots: usize,
    pub time: f32,
    /// The lamps' colour.
    pub lamp: Color,
    /// 0 hovering, 1 settled on the ground (a disabled tank's).
    pub settle: f32,
    /// Seconds since its tank was wrecked: the halo falls and lies dark.
    pub wreck_age: Option<f32>,
    /// A salt: the tank's owner slot.
    pub seed: u32,
}

/// The halo: each occupied slot a drone hovering at `FPV_HALO_HEIGHT_PX`,
/// bobbing a block on a 1.3 s cycle phased by slot, rotors turning, lamps
/// blinking at `fpv_lamp_hz` phased by slot, its shadow under it. Settled,
/// they lie at their slots, still and dark; a wreck's tumble to the ground
/// over `FPV_HALO_FALL_SECONDS` and lie dark. Pure.
pub fn compose_halo(look: &HaloLook, t: &Tuning) -> (Vec<Shape>, DronePicture) {
    let mut shadows = Vec::new();
    let mut pic = DronePicture::default();
    let shadow_dir = Vec2::new(t.shadow_dir_x, t.shadow_dir_y);
    for k in 0..look.drones.min(look.slots) {
        let (ground, _) = halo_slot(look.centre, look.sprite_size, k, look.slots);
        let seed = look.seed.wrapping_mul(31).wrapping_add(k as u32);
        let bob = if ((look.time / 1.3 + k as f32 / look.slots.max(1) as f32).rem_euclid(1.0)) < 0.5 { 0.0 } else { pyro::BLOCK };
        let (height, falling, live) = match look.wreck_age {
            Some(age) => {
                let k = (age / FPV_HALO_FALL_SECONDS).clamp(0.0, 1.0);
                (FPV_HALO_HEIGHT_PX * (1.0 - k * k), k < 1.0, false)
            }
            None => ((FPV_HALO_HEIGHT_PX + bob) * (1.0 - look.settle.clamp(0.0, 1.0)), false, look.settle <= 0.0),
        };
        shadows.extend(compose_shadow(ground, height, t.fpv_shadow_opacity, shadow_dir));
        let drone = compose_drone(&DroneLook {
            at: Position::new(ground.x, ground.y - height),
            seed,
            time: look.time,
            lamp: live.then_some(look.lamp),
            lamp_hz: t.fpv_lamp_hz,
            rotors: live,
            dive: None,
            spin_hz: falling.then_some(t.fpv_fall_spin_hz),
            block: pyro::BLOCK,
        });
        pic.body.extend(drone.body);
        pic.lamps.extend(drone.lamps);
    }
    (shadows, pic)
}

/// How a drone in the air looks this frame: its quad and lamp (`DroneLook`)
/// from its state.
pub fn look_of(drone: &Drone, time: f32, t: &Tuning) -> DroneLook {
    let falling = drone.stage == DroneStage::Falling;
    DroneLook {
        at: drone.drawn(),
        seed: drone.id,
        time,
        lamp: (!falling).then(|| lamp_color(drone.owner)),
        lamp_hz: if drone.stage == DroneStage::Dive { t.fpv_dive_lamp_hz } else { t.fpv_lamp_hz },
        rotors: !falling,
        dive: (drone.stage == DroneStage::Dive).then(|| drone.velocity()),
        spin_hz: falling.then_some(t.fpv_fall_spin_hz),
        block: drone_block(drone.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIELD: (f32, f32) = (2000.0, 1200.0);

    fn fly(d: &mut Drone, max: usize) -> Vec<DroneStage> {
        let mut stages = vec![d.stage];
        for _ in 0..max {
            d.advance(PHYSICS_FIXED_DT, Vec2::zero(), FIELD);
            if stages.last() != Some(&d.stage) {
                stages.push(d.stage);
            }
            if d.landed {
                break;
            }
        }
        stages
    }

    #[test]
    fn halo_slots_are_fixed_round_the_hull() {
        let centre = Position::new(500.0, 500.0);
        let (a, out) = halo_slot(centre, 80.0, 0, 6);
        assert!((a.distance_to(centre) - 44.0).abs() < 1e-3, "{a:?}");
        assert!((out.length() - 1.0).abs() < 1e-4);
        let bearings: Vec<f32> = (0..6)
            .map(|k| {
                let (p, _) = halo_slot(centre, 80.0, k, 6);
                (p.x - centre.x).atan2(-(p.y - centre.y)).to_degrees().rem_euclid(360.0)
            })
            .collect();
        for (k, b) in bearings.iter().enumerate() {
            assert!((b - (30.0 + 60.0 * k as f32)).abs() < 1e-2, "{bearings:?}");
        }
    }

    #[test]
    fn the_launch_path_is_the_same_whatever_the_lock() {
        let origin = Position::new(300.0, 300.0);
        let out = Vec2::new(0.0, -1.0);
        let mut near = Drone::launch(origin, out, 0, Owner::Player(0), DroneLock::None, Position::new(900.0, 900.0));
        let mut far = Drone::launch(origin, out, 0, Owner::Player(0), DroneLock::None, Position::new(10.0, 10.0));
        let t = tuning();
        let ticks = (t.fpv_launch_seconds / PHYSICS_FIXED_DT) as usize - 1;
        for _ in 0..ticks {
            near.advance(PHYSICS_FIXED_DT, Vec2::zero(), FIELD);
            far.advance(PHYSICS_FIXED_DT, Vec2::zero(), FIELD);
            assert_eq!((near.ground, near.height), (far.ground, far.height));
            let (p, h) = launch_path(origin, out, near.stage_time);
            assert!(p.distance_to(near.ground) < 1e-3 && (h - near.height).abs() < 1e-3);
        }
    }

    #[test]
    fn a_drone_climbs_cruises_dives_and_arrives() {
        let aim = Position::new(700.0, 300.0);
        let mut d = Drone::launch(Position::new(300.0, 300.0), Vec2::new(0.0, -1.0), 0, Owner::Player(0), DroneLock::None, aim);
        let stages = fly(&mut d, 2000);
        assert_eq!(stages, vec![DroneStage::Launch, DroneStage::Cruise, DroneStage::Dive]);
        assert!(d.landed && d.height == 0.0);
        assert!(d.ground.distance_to(aim) < 1e-2, "{:?}", d.ground);
    }

    #[test]
    fn the_turning_circle_is_inside_the_commit() {
        // A target behind the climb: the drone turns round without
        // circling it, and arrives well before its battery runs out.
        let t = tuning();
        let radius = t.fpv_speed / t.fpv_turn_rate_deg.to_radians();
        assert!(radius < t.fpv_commit_px, "turning radius {radius} vs commit {}", t.fpv_commit_px);
        let aim = Position::new(300.0, 420.0);
        let mut d = Drone::launch(Position::new(300.0, 300.0), Vec2::new(0.0, -1.0), 0, Owner::Player(0), DroneLock::None, aim);
        fly(&mut d, 2000);
        assert!(d.landed && d.age < t.fpv_max_flight_seconds, "age {}", d.age);
        assert!(d.ground.distance_to(aim) < 1e-2);
    }

    #[test]
    fn a_downed_drone_falls_and_lands() {
        let mut d = Drone::launch(Position::new(300.0, 300.0), Vec2::new(1.0, 0.0), 0, Owner::Enemy(4), DroneLock::None, Position::new(900.0, 300.0));
        for _ in 0..40 {
            d.advance(PHYSICS_FIXED_DT, Vec2::zero(), FIELD);
        }
        assert!(d.in_air());
        d.down(AirStrike::Bullet);
        assert!(!d.in_air());
        let stages = fly(&mut d, 600);
        assert_eq!(stages, vec![DroneStage::Falling]);
        assert!(d.landed && d.height == 0.0);
    }

    #[test]
    fn a_gust_carries_a_dive_off_its_point() {
        let aim = Position::new(500.0, 300.0);
        let mut d = Drone::launch(Position::new(300.0, 300.0), Vec2::new(1.0, 0.0), 0, Owner::Player(0), DroneLock::None, aim);
        for _ in 0..2000 {
            d.advance(PHYSICS_FIXED_DT, Vec2::new(0.0, 40.0), FIELD);
            if d.landed {
                break;
            }
        }
        assert!(d.landed);
        assert!(d.ground.y > aim.y + 2.0, "carried downwind: {:?}", d.ground);
    }

    fn look(time: f32, spin: Option<f32>) -> DroneLook {
        DroneLook {
            at: Position::new(101.0, 57.0),
            seed: 7,
            time,
            lamp: Some(Color::new(255, 0, 0, 255)),
            lamp_hz: 3.0,
            rotors: true,
            dive: None,
            spin_hz: spin,
            block: pyro::BLOCK,
        }
    }

    /// Up at cruise height a drone is drawn in blocks twice the size, on
    /// the grid of its own block; low it is a halo's.
    #[test]
    fn a_drone_up_high_is_drawn_twice_the_size() {
        assert_eq!(drone_block(FPV_HALO_HEIGHT_PX), pyro::BLOCK);
        assert_eq!(drone_block(36.0), pyro::BLOCK * 2.0);
        let big = compose_drone(&DroneLook { block: 4.0, ..look(0.0, None) });
        for shape in big.body.iter().chain(&big.lamps) {
            let Shape::Mark { pos, size, .. } = *shape else { continue };
            assert_eq!(size, 4);
            let off = ((pos.x - 2.0) / 4.0).fract().abs() + ((pos.y - 2.0) / 4.0).fract().abs();
            assert!(off < 1e-4, "on its own grid: {pos:?}");
        }
    }

    fn marks(shapes: &[Shape]) -> Vec<(i32, i32)> {
        shapes
            .iter()
            .filter_map(|s| match *s {
                Shape::Mark { pos, .. } => Some(pyro::block_of(pos.x, pos.y)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_drone_is_on_the_grid_and_pure() {
        let a = compose_drone(&look(0.31, None));
        assert_eq!(a, compose_drone(&look(0.31, None)));
        for s in a.body.iter().chain(&a.lamps) {
            let Shape::Mark { pos, size, .. } = *s else { panic!("a drone is made of marks") };
            assert_eq!(size, 2);
            assert_eq!(((pos.x - 1.0) % 2.0, (pos.y - 1.0) % 2.0), (0.0, 0.0), "{pos:?}");
        }
        let cells = marks(&a.body);
        let (bx, by) = pyro::block_of(101.0, 57.0);
        assert!(cells.iter().all(|&(x, y)| (bx - 2..bx + 2).contains(&x) && (by - 2..by + 2).contains(&y)), "{cells:?}");
    }

    #[test]
    fn the_lamp_blinks_at_its_rate() {
        let lit: Vec<bool> = (0..60).map(|i| !compose_drone(&look(i as f32 / 60.0, None)).lamps.is_empty()).collect();
        let flips = lit.windows(2).filter(|w| w[0] != w[1]).count();
        assert!((5..=7).contains(&flips), "3 Hz is six flips a second: {flips}");
    }

    #[test]
    fn the_rotors_turn() {
        let frames: Vec<Vec<(i32, i32)>> = (0..4).map(|i| marks(&compose_drone(&look(i as f32 * FPV_ROTOR_FRAME_SECONDS, None)).body)).collect();
        assert_ne!(frames[0], frames[1]);
        assert_eq!(frames[0], frames[2]);
    }

    #[test]
    fn a_falling_drone_alternates_its_silhouettes() {
        let x = marks(&compose_drone(&look(0.0, Some(6.0))).body);
        let plus = marks(&compose_drone(&look(1.0 / 6.0 + 0.01, Some(6.0))).body);
        assert_ne!(x, plus);
        assert_eq!(x, marks(&compose_drone(&look(2.0 / 6.0 + 0.01, Some(6.0))).body));
    }

    #[test]
    fn the_halo_settles_when_disabled_and_falls_on_a_wreck() {
        let t = tuning();
        let base = HaloLook {
            centre: Position::new(400.0, 400.0),
            sprite_size: 80.0,
            drones: 6,
            slots: 6,
            time: 0.4,
            lamp: Color::new(255, 0, 0, 255),
            settle: 0.0,
            wreck_age: None,
            seed: 0,
        };
        let (_, hover) = compose_halo(&base, &t);
        let (_, settled) = compose_halo(&HaloLook { settle: 1.0, ..base }, &t);
        assert!(settled.lamps.is_empty(), "settled lamps are dark");
        assert_ne!(marks(&hover.body), marks(&settled.body), "settled lower");
        let (_, lying) = compose_halo(&HaloLook { wreck_age: Some(5.0), ..base }, &t);
        assert!(lying.lamps.is_empty());
        let (_, fewer) = compose_halo(&HaloLook { drones: 2, ..base }, &t);
        assert!(fewer.body.len() < hover.body.len());
    }

    #[test]
    fn pick_nearest_ties_on_the_lower_key() {
        let from = Position::new(0.0, 0.0);
        let c = [
            LockCandidate { key: 9, pos: Position::new(30.0, 40.0) },
            LockCandidate { key: 4, pos: Position::new(-50.0, 0.0) },
            LockCandidate { key: 6, pos: Position::new(0.0, 80.0) },
        ];
        assert_eq!(pick_nearest(from, &c), Some(1));
        assert_eq!(pick_nearest(from, &[]), None);
    }
}
