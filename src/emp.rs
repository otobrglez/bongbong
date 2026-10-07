//! The EMP burst (docs/emp-burst.md), its headless half: the pulse a press
//! sends out (`EmpPulse`), the measures its front strikes by (`box_reach`),
//! the AI's pure scoring (`seat_value`), the module's cell, and the
//! composers in the effects language (docs/effects.md) - the ring, the
//! sparks on a disabled hull, an enemy's crackle and the lamp posts going
//! out. The world half is `simulation/emp.rs`. Every choice is hashed from
//! a slot, a pulse or a cell and the clock, never rolled.

use crate::math::Vec2;
use crate::pyro::{self, EMP, Shape};
use crate::shell::Owner;
use crate::tank::{ActiveWeapon, Tank};
use crate::tuning::Tuning;
use crate::Position;

/// The second and third circles of the ring, px inside its front.
pub const RING_INNER_PX: [f32; 2] = [6.0, 12.0];

/// How often the crackle along the ring's front changes (Hz).
pub const CRACKLE_HZ: f32 = 20.0;

/// How often a disabled hull's sparks change within a burst (Hz).
pub const SPARK_HZ: f32 = 12.0;

/// How often an enemy's crackle round its module changes (Hz).
pub const TELL_HZ: f32 = 15.0;

/// How long the lamp posts throw sparks as they go out (s).
pub const LAMP_SPARK_SECONDS: f32 = 0.25;

/// How long a lamp post takes to flicker back on at the end of its outage
/// (s).
pub const LAMP_RETURN_SECONDS: f32 = 0.6;

/// A disabled hull sparks in bursts this long ...
const BURST_SECONDS: f32 = 0.2;

/// ... this often (the Armory scene's rhythm: twice a second).
const BURST_PERIOD: f32 = 0.5;

/// One pulse running out from a hull: what it was fired from and by whom,
/// how far its front has swept, and whether it strikes anything - a room's
/// or a local round's does, a replica's and a client's own drawn press are
/// pictures only.
#[derive(Clone, Debug, PartialEq)]
pub struct EmpPulse {
    /// The hull's centre at the press.
    pub origin: Position,
    /// Who fired it: never struck by it.
    pub owner: Owner,
    /// Seconds since the press.
    pub age: f32,
    /// The radius its front has already swept (`-1` before its first
    /// frame, so a thing at the pivot is struck too).
    pub swept: f32,
    /// It strikes what it reaches (the round's); false for a picture.
    pub live: bool,
    /// Hashed from the pivot: the drawing's variety.
    pub seed: u32,
    /// The tanks it has struck, by owner slot: each once.
    pub struck: Vec<usize>,
}

impl EmpPulse {
    /// A fresh pulse from `origin` by `owner`.
    pub fn new(origin: Position, owner: Owner, live: bool) -> EmpPulse {
        EmpPulse { origin, owner, age: 0.0, swept: -1.0, live, seed: crate::blast::seed_at(origin, 0xE3_9B), struck: Vec::new() }
    }

    /// The front's radius at the pulse's age, held at the ring's reach.
    pub fn front(&self, t: &Tuning) -> f32 {
        (self.age * t.emp_ring_speed).min(t.emp_radius_px)
    }

    /// Whether its front has reached the ring's reach: it strikes nothing
    /// more.
    pub fn spent(&self, t: &Tuning) -> bool {
        self.swept >= t.emp_radius_px
    }

    /// Seconds since its front reached the ring's reach, `None` before.
    pub fn lingered(&self, t: &Tuning) -> Option<f32> {
        let out = t.emp_radius_px / t.emp_ring_speed.max(1.0);
        (self.age >= out).then_some(self.age - out)
    }

    /// Whether its picture is gone too: its front out and `emp_ring_seconds`
    /// more.
    pub fn done(&self, t: &Tuning) -> bool {
        self.lingered(t).is_some_and(|l| l > t.emp_ring_seconds)
    }
}

/// How far from `origin` the nearest point of the box `center` +- `half`
/// lies: where a ring from `origin` reaches it. 0 inside the box.
pub fn box_reach(origin: Position, center: Position, half: Vec2) -> f32 {
    let dx = ((origin.x - center.x).abs() - half.x).max(0.0);
    let dy = ((origin.y - center.y).abs() - half.y).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

/// What one seat in an enemy's ring is worth to it (docs/emp-burst.md
/// "AI"): `emp_ai_seat_value`, plus `emp_ai_shield_value` with a live
/// shield, `emp_ai_special_value` carrying an online special and
/// `emp_ai_night_value` at night; nothing for a seat already disabled.
pub fn seat_value(t: &Tuning, shielded: bool, special_online: bool, night: bool, disabled: bool) -> i32 {
    if disabled {
        return 0;
    }
    let mut value = t.emp_ai_seat_value;
    if shielded {
        value += t.emp_ai_shield_value;
    }
    if special_online {
        value += t.emp_ai_special_value;
    }
    if night {
        value += t.emp_ai_night_value;
    }
    value
}

/// Which side a disabled enemy's turret sags to (`Tank::droop`): +1 or -1,
/// hashed from its slot.
pub fn droop_side(slot: usize) -> f32 {
    if crate::blast::seed_at(Position::new(slot as f32 * 32.0, 0.0), 0x71) & 1 == 0 { 1.0 } else { -1.0 }
}

/// Which of the EMP module's five cells (`tank_modules.png`,
/// `TANK_MODULE_EMP_COL`) `tank` shows at `time`: 3 a pulse
/// (`Tank::emp_flash`), 1 and 2 alternating through its crackle, quicker
/// as it nears, 4 while the special is down (offline or disabled), else 0.
pub fn module_cell(tank: &Tank, time: f32) -> i32 {
    if tank.emp_flash > 0.0 {
        return 3;
    }
    if let Some(tell) = tank.tell.filter(|t| t.weapon == ActiveWeapon::Emp) {
        let hz = 8.0 + 16.0 * tell.progress();
        return 1 + ((time * hz) as i32).rem_euclid(2);
    }
    if tank.special_down() {
        return 4;
    }
    0
}

/// A jagged spark of `segments` straight runs from `from` heading `angle`
/// (radians, from +x, clockwise on the field), `len` px long in all, each
/// run turned by a hashed few tens of degrees: `EMP`'s pale blue with a
/// white head block.
fn zigzag(out: &mut Vec<Shape>, from: Position, angle: f32, len: f32, segments: u32, seed: u32, k: u32) {
    let segments = segments.max(1);
    let step = len / segments as f32;
    let mut at = from;
    let mut a = angle;
    for i in 0..segments {
        let turn = (pyro::unit(seed, k * 16 + i) - 0.5) * 1.6;
        a += turn;
        let to = Position::new(at.x + a.cos() * step, at.y + a.sin() * step);
        out.push(Shape::Line { from: at, to, width: 1.0, head: EMP[3], tail: EMP[3] });
        at = to;
    }
    out.push(Shape::Mark { pos: at, size: 2, color: EMP[4] });
}

/// The ring of `pulse` (the glowing pass: fast and bright): three whole
/// circles - the front two blocks thick in pale blue, a white one block
/// `RING_INNER_PX[0]` inside it and a bright blue one `RING_INNER_PX[1]`
/// inside (none under 8 px) - held at the reach once there and dissolving
/// over the last half of the linger; along the front a hashed crackle of
/// eight short zigzags that changes `CRACKLE_HZ` times a second; one glow
/// over the pivot, gone in its first 0.3 s.
pub fn ring(pulse: &EmpPulse, t: &Tuning) -> Vec<Shape> {
    let mut out = Vec::new();
    let r = pulse.front(t);
    let cover = match pulse.lingered(t) {
        Some(l) => {
            let half = t.emp_ring_seconds * 0.5;
            if l <= half { 1.0 } else { (1.0 - (l - half) / half.max(1e-3)).clamp(0.0, 1.0) }
        }
        None => 1.0,
    };
    if cover <= 0.0 {
        return out;
    }
    let full = std::f32::consts::TAU;
    let c = pulse.origin;
    if r >= pyro::BLOCK {
        out.push(Shape::Arc { center: c, radius: r, width: 2.0 * pyro::BLOCK, from: 0.0, to: full, color: EMP[3], cover });
    }
    let inner = [(RING_INNER_PX[0], EMP[4]), (RING_INNER_PX[1], EMP[2])];
    for (back, color) in inner {
        let rr = r - back;
        if rr >= 8.0 {
            out.push(Shape::Arc { center: c, radius: rr, width: pyro::BLOCK, from: 0.0, to: full, color, cover });
        }
    }
    if r >= 8.0 && cover >= 0.5 {
        let tick = (pulse.age * CRACKLE_HZ) as u32;
        let seed = pulse.seed ^ tick.wrapping_mul(0x9E37_79B9);
        for k in 0..8u32 {
            let a = pyro::unit(seed, k) * full;
            let at = Position::new(c.x + a.cos() * r, c.y + a.sin() * r);
            let outward = a + (pyro::unit(seed, 100 + k) - 0.5) * 1.2;
            zigzag(&mut out, at, outward, 4.0 + 4.0 * pyro::unit(seed, 200 + k), 3, seed, k);
        }
    }
    if pulse.age < 0.3 {
        let k = 1.0 - pulse.age / 0.3;
        out.push(Shape::Glow { pos: c, radius: 40.0 * k, color: pyro::alpha(EMP[3], 0.5 * k) });
    }
    out
}

/// The hash a disabled hull's sparks are phased by: its owner slot - the
/// drawing's and its light's alike.
pub fn spark_seed(slot: usize) -> u32 {
    crate::blast::seed_at(Position::new(slot as f32 * 32.0, 0.0), 0x5B)
}

/// Whether a disabled hull hashed by `seed` is in one of its spark bursts
/// at `time` (`BURST_SECONDS` of every `BURST_PERIOD`, phased by the seed):
/// what its sparks and their light follow.
pub fn sparking(seed: u32, time: f32) -> bool {
    let phase = pyro::unit(seed, 0x5A) * BURST_PERIOD;
    (time + phase).rem_euclid(BURST_PERIOD) < BURST_SECONDS
}

/// The sparks on a disabled hull, its box `center` +- `half`, `left`
/// seconds of its outage to run (the glowing pass): in a burst, four hashed
/// zigzags from points on the box's edge outward 6-16 px, changing
/// `SPARK_HZ` times a second, and one bright glow over the hull; over the
/// outage's last second one zigzag fewer every quarter second. Nothing
/// between bursts.
pub fn sparks(center: Position, half: Vec2, seed: u32, time: f32, left: f32) -> Vec<Shape> {
    let mut out = Vec::new();
    if left <= 0.0 || !sparking(seed, time) {
        return out;
    }
    let count = if left >= 1.0 { 4 } else { (left * 4.0).ceil() as u32 };
    let tick = (time * SPARK_HZ) as u32;
    let seed = seed ^ tick.wrapping_mul(0x85EB_CA6B);
    for k in 0..count {
        let a = pyro::unit(seed, k) * std::f32::consts::TAU;
        let (dx, dy) = (a.cos(), a.sin());
        // The point of the box's edge along that bearing.
        let s = (half.x / dx.abs().max(1e-3)).min(half.y / dy.abs().max(1e-3));
        let from = Position::new(center.x + dx * s, center.y + dy * s);
        let len = 6.0 + 10.0 * pyro::unit(seed, 50 + k);
        let segments = 2 + (pyro::unit(seed, 90 + k) * 2.0) as u32;
        zigzag(&mut out, from, a, len, segments, seed, k);
    }
    out.push(Shape::Glow { pos: center, radius: 20.0, color: pyro::alpha(EMP[2], 0.3) });
    out
}

/// An enemy's crackle before its pulse, round its module's coil at `coil`,
/// `progress` 0..1 through the wind-up (the glowing pass): two to four
/// hashed zigzags off the coil, 4 px growing to 12, changing `TELL_HZ` times
/// a second, and a small glow on the coil pulsing quicker as it nears.
pub fn tell(coil: Position, seed: u32, progress: f32, time: f32) -> Vec<Shape> {
    let mut out = Vec::new();
    let p = progress.clamp(0.0, 1.0);
    let tick = (time * TELL_HZ) as u32;
    let seed = seed ^ tick.wrapping_mul(0xC2B2_AE35);
    let count = 2 + (p * 2.0).round() as u32;
    for k in 0..count {
        let a = pyro::unit(seed, k) * std::f32::consts::TAU;
        let from = Position::new(coil.x + a.cos() * 3.0, coil.y + a.sin() * 3.0);
        zigzag(&mut out, from, a, 4.0 + 8.0 * p, 2, seed, k);
    }
    let pulse = 0.5 + 0.5 * (time * (6.0 + 18.0 * p) * std::f32::consts::TAU).sin();
    out.push(Shape::Glow { pos: coil, radius: 8.0 + 8.0 * p * pulse, color: pyro::alpha(EMP[3], 0.45) });
    out
}

/// A lamp post going out at `at`, `age` seconds into its outage (the
/// glowing pass): a little burst of white and pale blue blocks off its
/// lantern over its first `LAMP_SPARK_SECONDS`, then nothing.
pub fn lamp_sparks(at: Position, age: f32) -> Vec<Shape> {
    let mut out = Vec::new();
    if !(0.0..LAMP_SPARK_SECONDS).contains(&age) {
        return out;
    }
    let k = age / LAMP_SPARK_SECONDS;
    let seed = crate::blast::seed_at(at, 0x1A_77);
    let lantern = Position::new(at.x, at.y - 14.0);
    for i in 0..6u32 {
        let a = pyro::unit(seed, i) * std::f32::consts::TAU;
        let d = 4.0 + 10.0 * k * (0.5 + pyro::unit(seed, 20 + i));
        let pos = Position::new(lantern.x + a.cos() * d, lantern.y + a.sin() * d + 6.0 * k * k);
        out.push(Shape::Mark { pos, size: 2, color: if i % 2 == 0 { EMP[4] } else { EMP[3] } });
    }
    out
}

/// Whether a lamp post at `at` is drawn lit with `left` seconds of the
/// lamps' outage to run: dark while it runs, flickering back in two hashed
/// blinks over its last `LAMP_RETURN_SECONDS`.
pub fn lamp_lit(at: Position, left: f32) -> bool {
    if left <= 0.0 {
        return true;
    }
    if left > LAMP_RETURN_SECONDS {
        return false;
    }
    let k = 1.0 - left / LAMP_RETURN_SECONDS;
    let phase = pyro::unit(crate::blast::seed_at(at, 0x1A_78), 0) * 0.15;
    let blinks = [(0.15 + phase, 0.3 + phase), (0.5 + phase, 0.62 + phase)];
    k >= 0.8 || blinks.iter().any(|&(a, b)| k >= a && k < b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::CpuCanvas;
    use crate::tuning::Tuning;

    #[test]
    fn box_reach_is_the_distance_to_the_nearest_point() {
        let c = Position::new(100.0, 100.0);
        let half = Vec2::new(10.0, 20.0);
        assert_eq!(box_reach(Position::new(100.0, 100.0), c, half), 0.0, "inside");
        assert_eq!(box_reach(Position::new(140.0, 100.0), c, half), 30.0, "beside");
        assert_eq!(box_reach(Position::new(100.0, 50.0), c, half), 30.0, "above");
        assert!((box_reach(Position::new(113.0, 124.0), c, half) - 5.0).abs() < 1e-4, "off a corner");
    }

    #[test]
    fn seat_value_adds_its_parts() {
        let t = Tuning::DEFAULT;
        assert_eq!(seat_value(&t, false, false, false, false), 1);
        assert_eq!(seat_value(&t, true, false, false, false), 2);
        assert_eq!(seat_value(&t, true, true, true, false), 4);
        assert_eq!(seat_value(&t, true, true, true, true), 0, "already disabled");
    }

    #[test]
    fn a_pulse_runs_out_to_its_reach_and_is_gone_after_its_linger() {
        let t = Tuning::DEFAULT;
        let mut p = EmpPulse::new(Position::new(0.0, 0.0), Owner::Player(0), true);
        assert_eq!(p.front(&t), 0.0);
        p.age = 0.1;
        assert!((p.front(&t) - 80.0).abs() < 1e-3);
        p.age = 1.0;
        assert_eq!(p.front(&t), t.emp_radius_px, "held at its reach");
        assert!(!p.done(&t) || p.age > t.emp_radius_px / t.emp_ring_speed + t.emp_ring_seconds);
        p.age = t.emp_radius_px / t.emp_ring_speed + t.emp_ring_seconds + 0.01;
        assert!(p.done(&t));
    }

    /// Paint `shapes`, glows included, and hand back the canvas.
    fn painted(shapes: &[Shape], side: u32) -> CpuCanvas {
        let mut canvas = CpuCanvas::blank(side as usize, side as usize);
        pyro::draw(&mut canvas, shapes);
        pyro::draw_glows(&mut canvas, shapes, 3);
        canvas
    }

    /// Every painted pixel lies in a whole 2 px block of one colour.
    fn on_the_grid(canvas: &CpuCanvas, side: u32) {
        let px = canvas.pixels();
        for y in (0..side).step_by(2) {
            for x in (0..side).step_by(2) {
                let at = |dx: u32, dy: u32| px[((y + dy) * side + x + dx) as usize];
                let a = at(0, 0);
                assert!([at(1, 0), at(0, 1), at(1, 1)].iter().all(|&c| c == a), "block ({x}, {y}) split");
            }
        }
    }

    #[test]
    fn the_ring_is_on_the_grid_in_its_ramp_and_gone_by_its_end() {
        let t = Tuning::DEFAULT;
        let mut pulse = EmpPulse::new(Position::new(200.0, 200.0), Owner::Player(0), true);
        for age in [0.05, 0.12, 0.25, 0.4] {
            pulse.age = age;
            let shapes = ring(&pulse, &t);
            assert!(!shapes.is_empty(), "drawn at {age}");
            for s in &shapes {
                match *s {
                    Shape::Arc { color, .. } | Shape::Mark { color, .. } => assert!(EMP.contains(&color), "{color:?} in the ramp"),
                    // A glow is a ramp step at a fraction of its strength.
                    Shape::Glow { color, .. } => assert!(EMP.iter().any(|e| (e.r, e.g, e.b) == (color.r, color.g, color.b)), "{color:?} in the ramp"),
                    Shape::Line { head, tail, .. } => assert!(EMP.contains(&head) && EMP.contains(&tail)),
                    Shape::Puff(_) => panic!("no puffs in a ring"),
                }
            }
            on_the_grid(&painted(&shapes, 400), 400);
        }
        pulse.age = t.emp_radius_px / t.emp_ring_speed + t.emp_ring_seconds + 0.01;
        assert!(ring(&pulse, &t).is_empty(), "gone");
    }

    #[test]
    fn the_ring_never_draws_past_its_reach() {
        let t = Tuning::DEFAULT;
        let mut pulse = EmpPulse::new(Position::new(200.0, 200.0), Owner::Player(0), true);
        pulse.age = 0.5;
        let canvas = painted(&ring(&pulse, &t), 400);
        let blank = CpuCanvas::blank(400, 400);
        let (px, white) = (canvas.pixels(), blank.pixels());
        let reach = t.emp_radius_px + 2.0 * pyro::BLOCK + 16.0 + 2.0;
        for y in 0..400u32 {
            for x in 0..400u32 {
                if px[(y * 400 + x) as usize] != white[(y * 400 + x) as usize] {
                    let d = Position::new(x as f32, y as f32).distance_to(pulse.origin);
                    assert!(d <= reach, "a pixel {d} px out");
                }
            }
        }
    }

    #[test]
    fn the_sparks_are_hashed_and_thin_at_the_end() {
        let c = Position::new(60.0, 60.0);
        let half = Vec2::new(14.0, 16.0);
        let seed = 77;
        let time = (0..200).map(|i| i as f32 * 0.01).find(|&t| sparking(seed, t)).expect("a burst within two seconds");
        let a = sparks(c, half, seed, time, 3.0);
        assert_eq!(a, sparks(c, half, seed, time, 3.0), "the same inputs, the same sparks");
        let marks = |s: &[Shape]| s.iter().filter(|s| matches!(s, Shape::Mark { .. })).count();
        assert_eq!(marks(&a), 4);
        assert!(marks(&sparks(c, half, seed, time, 0.3)) < 4, "thinning at the end");
        assert!(sparks(c, half, seed, time, 0.0).is_empty(), "none once over");
        on_the_grid(&painted(&a, 120), 120);
    }

    #[test]
    fn the_tell_is_on_the_grid_and_pure() {
        let coil = Position::new(40.0, 40.0);
        let a = tell(coil, 9, 0.5, 1.25);
        assert_eq!(a, tell(coil, 9, 0.5, 1.25));
        assert!(a.iter().any(|s| matches!(s, Shape::Line { .. })));
        on_the_grid(&painted(&a, 80), 80);
    }

    #[test]
    fn lamp_sparks_are_gone_in_their_time_and_the_post_comes_back() {
        let at = Position::new(48.0, 48.0);
        assert!(!lamp_sparks(at, 0.1).is_empty());
        assert!(lamp_sparks(at, LAMP_SPARK_SECONDS + 0.01).is_empty());
        assert!(!lamp_lit(at, 5.0), "dark while the outage runs");
        assert!(lamp_lit(at, 0.0), "lit once it is over");
        assert!(lamp_lit(at, 0.05), "steady over its last moments");
        let flickers = (0..60).filter(|&i| lamp_lit(at, LAMP_RETURN_SECONDS * (1.0 - i as f32 / 60.0))).count();
        assert!(flickers > 0 && flickers < 60, "it flickers back");
    }

    #[test]
    fn the_droop_side_is_hashed_and_both_sides_come_up() {
        let sides: Vec<f32> = (0..16).map(droop_side).collect();
        assert!(sides.contains(&1.0) && sides.contains(&-1.0));
        assert_eq!(droop_side(5), droop_side(5));
    }
}
