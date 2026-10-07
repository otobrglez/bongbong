//! The volcano (docs/volcano.md): a cone seen from straight above, its
//! crater the map's `volcano` cell. This file is the data - the footprint,
//! the cycle, a bomb in flight - that the simulation
//! (`simulation/volcano.rs`) and the drawing share.
//!
//! **The cycle is a pure function of the round clock** (`phase`): asleep,
//! a rumble that warns, the eruption, the cooling, then asleep again until
//! the next period. A replica's clock stands on the room's tick, so it
//! rumbles and erupts on the same tick with nothing on the wire; only the
//! bombs, whose aim reads where the seats are, travel as events.

use crate::map::cell_to_world;
use crate::tuning::Tuning;
use crate::Position;

/// Where a volcano is in its cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Asleep,
    /// The warning: the crater brightens, the plume darkens, the ground
    /// trembles.
    Rumble,
    /// The fountain, the shock ring and the bombs.
    Erupt,
    /// The surge dying down the rivers.
    Cool,
}

/// A volcano's state at one moment of the round clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Phase {
    pub stage: Stage,
    /// Which eruption this period leads up to or follows (0 the first);
    /// -1 before the first rumble.
    pub eruption: i64,
    /// Seconds into the stage.
    pub age: f32,
    /// How far through the stage, 0..1.
    pub progress: f32,
    /// How hot the crater burns, 0.15 asleep to 1 erupting.
    pub heat: f32,
    /// The rivers' surge, 0 calm to 1 at the eruption's height.
    pub surge: f32,
}

impl Phase {
    /// Erupting or about to: the off-screen arrow and the minimap pulse.
    pub fn is_warning(&self) -> bool {
        matches!(self.stage, Stage::Rumble | Stage::Erupt)
    }
}

/// The stage lengths a period of `period` seconds holds, shrunk together
/// where the knobs ask for more than the period has.
fn stage_lengths(t: &Tuning) -> (f32, f32, f32, f32) {
    let period = t.volcano_period_seconds.max(1.0);
    let (rumble, erupt, cool) = (t.volcano_rumble_seconds.max(0.1), t.volcano_erupt_seconds.max(0.1), t.volcano_cool_seconds.max(0.1));
    let sum = rumble + erupt + cool;
    let k = if sum > period { period / sum } else { 1.0 };
    (period, rumble * k, erupt * k, cool * k)
}

/// The volcano whose clock is offset by `offset` seconds, at round time
/// `time`.
pub fn phase(time: f32, offset: f32, t: &Tuning) -> Phase {
    let (period, rumble, erupt, cool) = stage_lengths(t);
    let u = time - t.volcano_first_rumble_seconds - offset;
    let asleep = |eruption: i64, age: f32| Phase { stage: Stage::Asleep, eruption, age, progress: 0.0, heat: 0.15, surge: 0.0 };
    if u < 0.0 {
        return asleep(-1, time);
    }
    let n = (u / period).floor();
    let w = u - n * period;
    let n = n as i64;
    if w < rumble {
        let k = w / rumble;
        let flicker = (crate::lava::hash3((time * 12.0) as i32, n as i32, 3) - 0.5) * 0.12;
        return Phase { stage: Stage::Rumble, eruption: n, age: w, progress: k, heat: 0.18 + 0.5 * k + flicker, surge: 0.0 };
    }
    let w = w - rumble;
    if w < erupt {
        return Phase { stage: Stage::Erupt, eruption: n, age: w, progress: w / erupt, heat: 1.0, surge: (w / 0.8).min(1.0) };
    }
    let w = w - erupt;
    if w < cool {
        let c = w / cool;
        return Phase { stage: Stage::Cool, eruption: n, age: w, progress: c, heat: (1.0 - c * 1.1).max(0.15), surge: (1.0 - c * 1.3).max(0.0) };
    }
    asleep(n, w - cool)
}

/// When eruption `n` of the volcano offset by `offset` starts, on the round
/// clock.
pub fn eruption_start(n: i64, offset: f32, t: &Tuning) -> f32 {
    let (period, rumble, _, _) = stage_lengths(t);
    t.volcano_first_rumble_seconds + offset + n as f32 * period + rumble
}

/// When bomb `k` of an eruption that started at `start` leaves the crater:
/// spread over the first nine tenths of the eruption.
pub fn bomb_launch(start: f32, k: i32, count: i32, t: &Tuning) -> f32 {
    let (_, _, erupt, _) = stage_lengths(t);
    start + erupt * 0.9 * (k as f32 + 0.5) / count.max(1) as f32
}

/// A volcano on the field: its crater and the clock offset hashed from it,
/// so two volcanoes on one map never erupt together.
#[derive(Clone, Debug)]
pub struct Volcano {
    pub cell: (i32, i32),
    /// The clock offset its cycle runs on: `base` moved by `shift` ticks.
    pub offset: f32,
    /// The offset hashed from its crater, which the cycle runs on until a
    /// rod sets it off.
    pub base: f32,
    /// Ticks a rod from god moved its cycle by (`set_off_shift`,
    /// docs/rod-from-god.md), 0 until one does: the round state carries it,
    /// so a replica's cycle runs on the room's.
    pub shift: i32,
    /// The directions (radians, 0 east, clockwise as the field's y runs
    /// down) the lava leaves the cone in: where its gullies run.
    pub outlets: Vec<f32>,
}

impl Volcano {
    /// The volcano on crater `cell`, its gullies cut toward `outlets`.
    pub fn new(cell: (i32, i32), outlets: Vec<f32>, t: &Tuning) -> Volcano {
        let h = crate::lava::hash3(cell.0, cell.1, 71);
        let base = h * 0.25 * t.volcano_period_seconds.max(1.0);
        Volcano { cell, offset: base, base, shift: 0, outlets }
    }

    /// Move its cycle by `shift` ticks from its hashed offset.
    pub fn set_shift(&mut self, shift: i32) {
        self.shift = shift;
        self.offset = self.base + shift as f32 * crate::PHYSICS_FIXED_DT;
    }

    /// The shift (whole ticks) that starts this volcano's next eruption on
    /// the tick after round time `now` (docs/rod-from-god.md "A volcano"),
    /// and that eruption's number: from asleep or cooling the one after the
    /// last, from a rumble that rumble's own. `None` while it erupts. The
    /// cycle runs on from there a period at a time - still a pure function
    /// of the round clock and the shift.
    pub fn set_off_shift(&self, now: f32, t: &Tuning) -> Option<(i32, i64)> {
        let p = self.phase(now, t);
        let n = match p.stage {
            Stage::Erupt => return None,
            Stage::Rumble => p.eruption,
            Stage::Asleep | Stage::Cool => p.eruption + 1,
        };
        let (period, rumble, _, _) = stage_lengths(t);
        let dt = crate::PHYSICS_FIXED_DT;
        let want = now + dt - t.volcano_first_rumble_seconds - n as f32 * period - rumble;
        let shift = ((want - self.base) / dt).floor() as i32;
        Some((shift, n))
    }

    /// The crater's centre on the field.
    pub fn centre(&self) -> Position {
        cell_to_world(self.cell.0, self.cell.1)
    }

    pub fn phase(&self, time: f32, t: &Tuning) -> Phase {
        phase(time, self.offset, t)
    }
}

/// A lava bomb in the air: thrown from the crater at `from`, landing at
/// `to` after `volcano_bomb_flight_seconds`, where it bursts and splashes
/// lava. Its landing spot is decided at launch, so its ring warns where
/// it will come down the whole way; its arc is derived from its age, so
/// nothing integrates it (`props::FlyingDrum`'s convention).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LavaBomb {
    pub from: Position,
    pub to: Position,
    pub age: f32,
    /// Hashed from where it lands: its spin and its trail.
    pub seed: u32,
}

impl LavaBomb {
    pub fn new(from: Position, to: Position) -> LavaBomb {
        LavaBomb { from, to, age: 0.0, seed: crate::blast::seed_at(to, 73) }
    }

    /// How far through its flight, 0..1.
    pub fn flight(&self, t: &Tuning) -> f32 {
        (self.age / t.volcano_bomb_flight_seconds.max(1e-3)).clamp(0.0, 1.0)
    }

    /// Seconds until it lands.
    pub fn left(&self, t: &Tuning) -> f32 {
        (t.volcano_bomb_flight_seconds - self.age).max(0.0)
    }

    /// How high above the ground it is (px): a parabola peaking at
    /// `volcano_bomb_arc_px`, starting a little up off the crater's rim.
    pub fn height(&self, t: &Tuning) -> f32 {
        let p = self.flight(t);
        24.0 * (1.0 - p) + t.volcano_bomb_arc_px * 4.0 * p * (1.0 - p)
    }

    /// The point on the ground it is over now.
    pub fn ground_pos(&self, t: &Tuning) -> Position {
        let p = self.flight(t);
        Position::new(self.from.x + (self.to.x - self.from.x) * p, self.from.y + (self.to.y - self.from.y) * p)
    }

    pub fn landed(&self, t: &Tuning) -> bool {
        self.age >= t.volcano_bomb_flight_seconds
    }
}

/// How far the cone reaches from its crater, in cells: every cell whose
/// centre lies within this radius of the crater's is under the cone - a
/// 5 x 5 block less its corners, 21 cells.
pub const FOOTPRINT_RADIUS2: i32 = 5;

/// The cone's reach from the crater in whole cells either way.
pub const FOOTPRINT_REACH: i32 = 2;

/// Whether the cell `(col, row)` stands under the cone of the volcano
/// whose crater is `(cc, cr)`.
pub fn in_footprint(cc: i32, cr: i32, col: i32, row: i32) -> bool {
    let (dc, dr) = (col - cc, row - cr);
    dc * dc + dr * dr <= FOOTPRINT_RADIUS2
}

/// Every cell under the cone of the volcano whose crater is `(cc, cr)`,
/// row by row.
pub fn footprint(cc: i32, cr: i32) -> impl Iterator<Item = (i32, i32)> {
    (-FOOTPRINT_REACH..=FOOTPRINT_REACH).flat_map(move |dr| {
        (-FOOTPRINT_REACH..=FOOTPRINT_REACH).filter_map(move |dc| {
            (dc * dc + dr * dr <= FOOTPRINT_RADIUS2).then_some((cc + dc, cr + dr))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cycle_sleeps_rumbles_erupts_and_cools() {
        let t = Tuning::DEFAULT;
        let start = t.volcano_first_rumble_seconds;
        assert_eq!(phase(start - 1.0, 0.0, &t).stage, Stage::Asleep);
        assert_eq!(phase(start - 1.0, 0.0, &t).eruption, -1);
        assert_eq!(phase(start + 0.5, 0.0, &t).stage, Stage::Rumble);
        let erupting = phase(start + t.volcano_rumble_seconds + 0.5, 0.0, &t);
        assert_eq!(erupting.stage, Stage::Erupt);
        assert_eq!(erupting.heat, 1.0);
        assert_eq!(eruption_start(0, 0.0, &t), start + t.volcano_rumble_seconds);
        let cooling = phase(start + t.volcano_rumble_seconds + t.volcano_erupt_seconds + 0.5, 0.0, &t);
        assert_eq!(cooling.stage, Stage::Cool);
        assert!(cooling.surge > 0.0 && cooling.surge < 1.0);
        assert_eq!(phase(start + t.volcano_period_seconds - 0.5, 0.0, &t).stage, Stage::Asleep);
        let second = phase(start + t.volcano_period_seconds + 0.5, 0.0, &t);
        assert_eq!((second.stage, second.eruption), (Stage::Rumble, 1));
    }

    #[test]
    fn set_off_shift_starts_the_eruption_on_the_next_tick_and_leaves_an_eruption_alone() {
        let t = Tuning::DEFAULT;
        let dt = crate::PHYSICS_FIXED_DT;
        for now in [3.0_f32, 14.5, 17.0, 26.0, 40.0] {
            let mut v = Volcano::new((10, 10), Vec::new(), &t);
            let before = v.phase(now, &t);
            let Some((shift, n)) = v.set_off_shift(now, &t) else {
                assert_eq!(before.stage, Stage::Erupt, "{now}");
                continue;
            };
            v.set_shift(shift);
            let next = v.phase(now + dt, &t);
            assert_eq!((next.stage, next.eruption), (Stage::Erupt, n), "{now}: erupting on the next tick");
            assert!(v.phase(now, &t).stage != Stage::Erupt, "{now}: not yet on this one");
            assert!(n >= before.eruption.max(0), "{now}: the count never goes back");
            let again = v.phase(now + dt + t.volcano_period_seconds, &t);
            assert_eq!((again.stage, again.eruption), (Stage::Erupt, n + 1), "{now}: the next a period later");
        }
    }

    #[test]
    fn the_cycle_is_a_function_of_the_clock_alone() {
        let t = Tuning::DEFAULT;
        for i in 0..400 {
            let time = i as f32 * 0.37;
            assert_eq!(phase(time, 2.5, &t), phase(time, 2.5, &t));
        }
    }

    #[test]
    fn a_bomb_lands_where_it_was_aimed_after_its_flight() {
        let t = Tuning::DEFAULT;
        let mut bomb = LavaBomb::new(Position::new(100.0, 100.0), Position::new(300.0, 160.0));
        assert!(!bomb.landed(&t));
        bomb.age = t.volcano_bomb_flight_seconds;
        assert!(bomb.landed(&t));
        assert_eq!(bomb.ground_pos(&t), Position::new(300.0, 160.0));
        assert!(bomb.height(&t).abs() < 1e-3);
    }

    #[test]
    fn the_footprint_is_a_block_of_five_less_its_corners() {
        let cells: Vec<_> = footprint(10, 10).collect();
        assert_eq!(cells.len(), 21);
        assert!(cells.contains(&(10, 10)));
        assert!(cells.contains(&(12, 11)));
        assert!(!cells.contains(&(12, 12)));
        assert!(cells.iter().all(|&(c, r)| in_footprint(10, 10, c, r)));
    }
}

// ---------------------------------------------------------------------------
// The picture (docs/volcano.md): the cone seen straight from above, composed
// once per set of outlets in the effects language and kept; the crater and
// the gullies' molten pixels are coloured each frame from the cycle.

use crate::canvas::{BlockImage, Canvas};
use crate::math::Color;
use crate::pyro::{Puff, Shape, FIRE, SMOKE};
use std::sync::{Arc, Mutex};
use crate::trig;

/// Cells across the cone's picture: a little wider than its footprint, so
/// the slope overhangs the outer cells' edges the way a tree's crown does.
pub const CONE_CELLS: i32 = 5;

/// Design pixels across the picture (each one a 2 px block on the field).
pub const CONE_DESIGN: i32 = CONE_CELLS * 16 + 8;

const RUST: Color = Color::new(0x8D, 0x4A, 0x25, 255);
const RUST_DK: Color = Color::new(0x59, 0x34, 0x1F, 255);
const WOOD_ASH: Color = Color::new(0x73, 0x62, 0x4D, 255);
const WOOD_LT: Color = Color::new(0x99, 0x65, 0x24, 255);

/// The cone's lower slopes, dark to light: black, stone, ash.
const ASH: [Color; 5] = [SMOKE[0], SMOKE[1], SMOKE[2], WOOD_ASH, SMOKE[3]];

/// The rim's scoria, dark to light: black through rust.
const SCORIA: [Color; 5] = [SMOKE[0], FIRE[0], RUST_DK, RUST, WOOD_LT];

/// A pixel of the picture that molten rock fills.
#[derive(Clone, Copy, Debug)]
struct Molten {
    x: i16,
    y: i16,
    /// The crater (true) or a gully running down to an outlet.
    crater: bool,
    /// How far out it lies: 0 at the crater's middle or a gully's top, 1
    /// at the crater's rim or the gully's foot.
    d: f32,
    /// A hash, 0..1, for its own flicker.
    h: f32,
}

/// The cone's picture, made once per set of outlets (`cone`).
#[derive(Debug)]
pub struct ConePicture {
    /// Runs of one colour in design pixels: (row, first column, end
    /// column, colour).
    runs: Vec<(i16, i16, i16, Color)>,
    /// The silhouette for the shadow, as runs.
    shadow: Vec<(i16, i16, i16)>,
    /// The cinder skirt round the foot, as runs in a picture eight design
    /// pixels wider each way.
    skirt: Vec<(i16, i16, i16, Color)>,
    molten: Vec<Molten>,
    /// The runs above baked as images the first time they are asked for
    /// (`images`), so a round draws each as one quad.
    images: std::sync::OnceLock<ConeImages>,
}

/// What of a cone's picture never changes, as images a texel per design
/// pixel: the slopes (`CONE_DESIGN` across), the shadow's silhouette and
/// the cinder skirt (eight design pixels wider each way).
#[derive(Debug)]
pub struct ConeImages {
    pub body: BlockImage,
    pub shadow: BlockImage,
    pub skirt: BlockImage,
}

/// An image `size` texels square of `runs`, transparent elsewhere.
fn image_of(runs: impl Iterator<Item = (i16, i16, i16, Color)>, size: i32) -> BlockImage {
    let size = size.max(1) as usize;
    let mut texels = vec![Color::new(0, 0, 0, 0); size * size];
    for (y, x0, x1, color) in runs {
        for x in x0..x1 {
            texels[y as usize * size + x as usize] = color;
        }
    }
    BlockImage { width: size, height: size, block: 2, texels, stamp: crate::ground::next_block_stamp(), patches: Vec::new() }
}

impl ConePicture {
    /// The fixed parts as images, baked on first use.
    pub fn images(&self) -> &ConeImages {
        self.images.get_or_init(|| ConeImages {
            body: image_of(self.runs.iter().copied(), CONE_DESIGN),
            shadow: image_of(self.shadow.iter().map(|&(y, x0, x1)| (y, x0, x1, SHADOW)), CONE_DESIGN),
            skirt: image_of(self.skirt.iter().copied(), CONE_DESIGN + 16),
        })
    }
}

/// The cone's shadow's shade.
const SHADOW: Color = Color::new(0, 0, 0, 90);

fn noise_a(th: f32, s: i32) -> f32 {
    use crate::lava::hash3;
    use crate::trig::sin;
    0.5 + 0.25 * sin(3.0 * th + hash3(s, 1, 1) * 6.28) + 0.15 * sin(5.0 * th + hash3(s, 2, 1) * 6.28) + 0.1 * sin(7.0 * th + hash3(s, 3, 1) * 6.28)
}

/// `exp(-u)` for `u >= 0` as the reciprocal of its series to the fourth
/// power: IEEE arithmetic only, so the cone's picture - which a pinned
/// thumbnail holds - comes out the same on every platform's libm.
fn bell(u: f32) -> f32 {
    1.0 / (1.0 + u + u * u / 2.0 + u * u * u / 6.0 + u * u * u * u / 24.0)
}

fn value_noise(x: f32, y: f32, s: i32) -> f32 {
    use crate::lava::hash3;
    let (xi, yi) = (x.floor(), y.floor());
    let (xf, yf) = (x - xi, y - yi);
    let (u, v) = (xf * xf * (3.0 - 2.0 * xf), yf * yf * (3.0 - 2.0 * yf));
    let (xi, yi) = (xi as i32, yi as i32);
    let a = hash3(xi, yi, s) + (hash3(xi + 1, yi, s) - hash3(xi, yi, s)) * u;
    let b = hash3(xi, yi + 1, s) + (hash3(xi + 1, yi + 1, s) - hash3(xi, yi + 1, s)) * u;
    a + (b - a) * v
}

fn angle_between(a: f32, b: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let d = (a - b).abs() % tau;
    if d > std::f32::consts::PI { tau - d } else { d }
}

/// Runs of equal colour along each row of a `size` x `size` grid.
fn runs_of(grid: &[Option<Color>], size: i32) -> Vec<(i16, i16, i16, Color)> {
    let mut out = Vec::new();
    for y in 0..size {
        let mut x = 0;
        while x < size {
            let Some(c) = grid[(y * size + x) as usize] else {
                x += 1;
                continue;
            };
            let start = x;
            while x < size && grid[(y * size + x) as usize] == Some(c) {
                x += 1;
            }
            out.push((y as i16, start as i16, x as i16, c));
        }
    }
    out
}

/// Compose the cone whose gullies run toward `outlets` (radians). Pure:
/// the same outlets give the same picture.
fn compose_cone(outlets: &[f32]) -> ConePicture {
    use crate::lava::hash3;
    use std::f32::consts::{PI, TAU};
    let cells = CONE_CELLS;
    let d = CONE_DESIGN;
    let (cx, cy) = (d as f32 / 2.0, d as f32 / 2.0);
    let r_big = cells as f32 * 8.0 + 1.5;
    let (rc, rr) = (r_big * 0.24, r_big * 0.37);
    let gullies_n = 4 + cells * 2;
    let mut gullies: Vec<f32> = (0..gullies_n).map(|k| (k as f32 + (hash3(k, cells, 31) - 0.5) * 0.5) / gullies_n as f32 * TAU).collect();
    gullies.extend_from_slice(outlets);
    let edge_r = |th: f32| r_big * (1.0 + 0.05 * (noise_a(th, 7) * 2.0 - 1.0));
    let n = (d * d) as usize;
    let mut hf = vec![-1.0f32; n];
    for y in 0..d {
        for x in 0..d {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let r = (dx * dx + dy * dy).sqrt();
            let th = trig::atan2(dy, dx);
            let re = edge_r(th);
            if r > re {
                continue;
            }
            let mut h = if r < rc {
                0.5
            } else if r < rr {
                0.5 + 0.5 * trig::sin((r - rc) / (rr - rc) * PI / 2.0)
            } else {
                // x^1.25, as x times its fourth root.
                let x = (1.0 - (r - rr) / (re - rr)).max(0.0);
                x * x.sqrt().sqrt()
            };
            if r > rr {
                for &a in &gullies {
                    let w = angle_between(th, a) * r;
                    h -= 0.08 * bell((w / 1.4) * (w / 1.4)) * ((r - rr) / 3.0).clamp(0.0, 1.0);
                }
            }
            h += (value_noise(x as f32 * 0.5, y as f32 * 0.5, 11) - 0.5) * 0.05;
            hf[(y * d + x) as usize] = h.max(0.002);
        }
    }
    let height_at = |x: i32, y: i32| if x < 0 || y < 0 || x >= d || y >= d { 0.0 } else { hf[(y * d + x) as usize].max(0.0) };
    let lv = {
        let v = [-0.6f32, -0.5, 0.63];
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / l, v[1] / l, v[2] / l]
    };
    let slope = r_big * 0.6;
    let mut grid: Vec<Option<Color>> = vec![None; n];
    let mut molten = Vec::new();
    for y in 0..d {
        for x in 0..d {
            if hf[(y * d + x) as usize] < 0.0 {
                continue;
            }
            let hx = (height_at(x + 1, y) - height_at(x - 1, y)) / 2.0;
            let hy = (height_at(x, y + 1) - height_at(x, y - 1)) / 2.0;
            let nv = [-hx * slope, -hy * slope, 1.0];
            let nl = (nv[0] * nv[0] + nv[1] * nv[1] + nv[2] * nv[2]).sqrt();
            let lum = (nv[0] * lv[0] + nv[1] * lv[1] + nv[2] * lv[2]) / nl;
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let r = (dx * dx + dy * dy).sqrt();
            let th = trig::atan2(dy, dx);
            if r < rc {
                grid[(y * d + x) as usize] = Some(SMOKE[0]);
                molten.push(Molten { x: x as i16, y: y as i16, crater: true, d: r / rc, h: hash3(x, y, 9) });
                continue;
            }
            let t = (r - rr) / (edge_r(th) - rr);
            let scoria = r < rr || t < 0.5 + (crate::pyro::bayer(x, y) - 0.5) * 0.35;
            let mut i = ((lum * 5.2 - 0.3 + (hash3(x, y, 2) - 0.5) * 0.7).floor() as i32).clamp(0, 4);
            if (r - rr).abs() < 0.8 && dx + dy < 0.0 && i < 4 {
                i += 1;
            }
            grid[(y * d + x) as usize] = Some(if scoria { SCORIA[i as usize] } else { ASH[i as usize] });
        }
    }
    // The gullies the lava leaves by: molten lines from the rim to the foot.
    for (ci, &a) in outlets.iter().enumerate() {
        let mut seen = std::collections::BTreeSet::new();
        let mut r = rr + 0.6;
        while r < r_big + 0.6 {
            for w in [-0.5f32, 0.5] {
                let aa = a + w / r.max(1.0) + 0.03 * trig::sin(r * 0.8 + ci as f32 * 2.0);
                let (sa, ca) = trig::sin_cos(aa);
                let (x, y) = ((cx + ca * r).floor() as i32, (cy + sa * r).floor() as i32);
                if x < 0 || y < 0 || x >= d || y >= d || grid[(y * d + x) as usize].is_none() || !seen.insert((x, y)) {
                    continue;
                }
                grid[(y * d + x) as usize] = Some(SMOKE[0]);
                molten.push(Molten { x: x as i16, y: y as i16, crater: false, d: (r - rr) / (r_big - rr), h: hash3(x, y, 4) });
            }
            r += 0.4;
        }
    }
    // A dark line round the foot, where the slope meets the ground.
    let solid = grid.clone();
    for y in 0..d {
        for x in 0..d {
            if solid[(y * d + x) as usize].is_some() {
                continue;
            }
            let near = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(ox, oy)| {
                let (nx, ny) = (x + ox, y + oy);
                nx >= 0 && ny >= 0 && nx < d && ny < d && solid[(ny * d + nx) as usize].is_some()
            });
            if near {
                grid[(y * d + x) as usize] = Some(SMOKE[0]);
            }
        }
    }
    let runs = runs_of(&grid, d);
    let shadow = runs.iter().map(|&(y, x0, x1, _)| (y, x0, x1)).collect();
    // The cinder skirt: dark grit thinning out past the foot.
    let sd = d + 16;
    let sc = sd as f32 / 2.0;
    let mut skirt: Vec<Option<Color>> = vec![None; (sd * sd) as usize];
    for y in 0..sd {
        for x in 0..sd {
            let e = ((x as f32 + 0.5 - sc).powi(2) + (y as f32 + 0.5 - sc).powi(2)).sqrt() / r_big;
            if e < 0.92 {
                continue;
            }
            let t = (e - 0.92) * r_big / 7.0;
            if t > 1.0 || crate::pyro::bayer(x, y) < t * 1.05 + 0.15 {
                continue;
            }
            let hh = hash3(x, y, 43);
            skirt[(y * sd + x) as usize] = Some(if hh < 0.35 {
                SMOKE[0]
            } else if hh < 0.6 {
                SMOKE[1]
            } else if hh < 0.8 {
                WOOD_ASH
            } else {
                FIRE[0]
            });
        }
    }
    ConePicture { runs, shadow, skirt: runs_of(&skirt, sd), molten, images: std::sync::OnceLock::new() }
}

/// The picture of a cone whose gullies run toward `outlets`, made once and
/// kept for the session (a map has a handful of volcanoes at most).
pub fn cone(outlets: &[f32]) -> Arc<ConePicture> {
    static CONES: Mutex<Vec<(Vec<i32>, Arc<ConePicture>)>> = Mutex::new(Vec::new());
    let key: Vec<i32> = outlets.iter().map(|a| (a.to_degrees().round() as i32).rem_euclid(360)).collect();
    let mut cones = CONES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, picture)) = cones.iter().find(|(k, _)| *k == key) {
        return picture.clone();
    }
    let picture = Arc::new(compose_cone(outlets));
    cones.push((key, picture.clone()));
    picture
}

/// How far down its gullies the molten rock reaches, 0..1, at `phase`.
fn reach(phase: &Phase) -> f32 {
    match phase.stage {
        Stage::Asleep => 0.0,
        Stage::Rumble => 0.3 * phase.progress,
        Stage::Erupt => (0.3 + phase.age / 0.9).min(1.0),
        Stage::Cool => 1.0 - ((phase.progress - 0.75) / 0.25).max(0.0),
    }
}

fn molten_color(m: &Molten, phase: &Phase, time: f32) -> Color {
    use crate::lava::hash3;
    if m.crater {
        let bub = hash3(m.x as i32, m.y as i32, (time * 3.0 + m.h * 5.0).floor() as i32);
        if phase.heat < 0.22 {
            return if bub < 0.16 + phase.heat * 0.5 {
                if bub < 0.05 { FIRE[2] } else { FIRE[1] }
            } else if m.h < 0.55 {
                SMOKE[0]
            } else {
                SMOKE[1]
            };
        }
        let v = phase.heat * 0.9 + (bub - 0.5) * 0.35 - m.d * 0.15;
        return FIRE[((1.5 + v * 4.6).round() as i32).clamp(1, 7) as usize];
    }
    let reach = reach(phase).max(0.55);
    if m.d > reach {
        return if m.h < 0.3 { FIRE[0] } else { SMOKE[1] };
    }
    let hot = phase.heat.max(0.35) * (1.0 - 0.35 * m.d);
    let pulse = (m.d * 7.0 - time * 2.2).rem_euclid(1.0);
    let v = hot + if pulse < 0.35 { 0.12 } else { -0.05 } + (m.h - 0.5) * 0.1;
    FIRE[((v * 6.0).round() as i32).clamp(1, 6) as usize]
}

/// The picture's top-left corner on the field for a crater at `centre`.
fn cone_origin(centre: Position) -> (i32, i32) {
    let half = CONE_DESIGN; // design px are 2 px: the picture is 2 x CONE_DESIGN wide
    (crate::pyro::snap(centre.x) - half, crate::pyro::snap(centre.y) - half)
}

/// The cinder skirt round a cone's foot: floor art, under its shadow.
pub fn draw_skirt(c: &mut impl Canvas, centre: Position, picture: &ConePicture) {
    let (ox, oy) = cone_origin(centre);
    let image = &picture.images().skirt;
    if c.has_blocks(image.stamp) {
        c.blocks_part(image, (0, 0, image.width, image.height), (ox - 16, oy - 16));
        return;
    }
    for &(y, x0, x1, color) in &picture.skirt {
        c.fill_rect(ox - 16 + x0 as i32 * 2, oy - 16 + y as i32 * 2, (x1 - x0) as i32 * 2, 2, color);
    }
}

/// The cone's shadow: its silhouette at a third, cast `shadow_px` along the
/// field's shadow direction.
pub fn draw_cone_shadow(c: &mut impl Canvas, centre: Position, picture: &ConePicture, shadow_dir: (f32, f32)) {
    let (ox, oy) = cone_origin(centre);
    let (sx, sy) = (((shadow_dir.0 * 12.0) / 2.0).round() as i32 * 2, ((shadow_dir.1 * 12.0) / 2.0).round() as i32 * 2);
    let image = &picture.images().shadow;
    if c.has_blocks(image.stamp) {
        c.blocks_part(image, (0, 0, image.width, image.height), (ox + sx, oy + sy));
        return;
    }
    for &(y, x0, x1) in &picture.shadow {
        c.fill_rect(ox + sx + x0 as i32 * 2, oy + sy + y as i32 * 2, (x1 - x0) as i32 * 2, 2, SHADOW);
    }
}

/// The cone, its crater and gullies coloured for `phase` at `time`.
pub fn draw_cone(c: &mut impl Canvas, centre: Position, picture: &ConePicture, phase: &Phase, time: f32) {
    draw_cone_with(c, centre, picture, None, phase, time);
}

/// `draw_cone` with the molten pixels baked (`bake_molten`) where a round
/// keeps them: its slopes and crater as two quads where the canvas has
/// them uploaded, pixel by pixel where it has not.
pub fn draw_cone_with(c: &mut impl Canvas, centre: Position, picture: &ConePicture, molten: Option<&BlockImage>, phase: &Phase, time: f32) {
    let (ox, oy) = cone_origin(centre);
    let body = &picture.images().body;
    if c.has_blocks(body.stamp) {
        c.blocks_part(body, (0, 0, body.width, body.height), (ox, oy));
    } else {
        for &(y, x0, x1, color) in &picture.runs {
            c.fill_rect(ox + x0 as i32 * 2, oy + y as i32 * 2, (x1 - x0) as i32 * 2, 2, color);
        }
    }
    if let Some(image) = molten.filter(|image| c.has_blocks(image.stamp)) {
        c.blocks_part(image, (0, 0, image.width, image.height), (ox, oy));
        return;
    }
    for m in &picture.molten {
        c.fill_rect(ox + m.x as i32 * 2, oy + m.y as i32 * 2, 2, 2, molten_color(m, phase, time));
    }
}

/// Bake `picture`'s molten pixels at `phase` and `time`: every one into
/// `body`, the ones bright enough to shine into `glow` (what
/// `draw_cone_glow` draws), both images of the picture's size under a new
/// stamp.
pub fn bake_molten(picture: &ConePicture, phase: &Phase, time: f32, body: &mut BlockImage, glow: &mut BlockImage) {
    let size = CONE_DESIGN as usize;
    for image in [&mut *body, &mut *glow] {
        if image.width != size || image.height != size {
            *image = BlockImage { width: size, height: size, block: 2, texels: vec![Color::new(0, 0, 0, 0); size * size], stamp: 0, patches: Vec::new() };
        }
    }
    for m in &picture.molten {
        let color = molten_color(m, phase, time);
        let at = m.y as usize * size + m.x as usize;
        body.texels[at] = color;
        glow.texels[at] = if FIRE[3..].contains(&color) { color } else { Color::new(0, 0, 0, 0) };
    }
    body.stamp = crate::ground::next_block_stamp();
    glow.stamp = crate::ground::next_block_stamp();
}

/// The crater's molten pixels alone, those bright enough to shine: what
/// the glowing pass draws again over a darkened field.
pub fn draw_cone_glow(c: &mut impl Canvas, centre: Position, picture: &ConePicture, phase: &Phase, time: f32, alpha: f32) {
    draw_cone_glow_with(c, centre, picture, None, phase, time, alpha);
}

/// `draw_cone_glow` from the baked bright pixels (`bake_molten`) where the
/// canvas has them uploaded.
pub fn draw_cone_glow_with(c: &mut impl Canvas, centre: Position, picture: &ConePicture, glow: Option<&BlockImage>, phase: &Phase, time: f32, alpha: f32) {
    let (ox, oy) = cone_origin(centre);
    if let Some(image) = glow.filter(|image| alpha >= 1.0 && c.has_blocks(image.stamp)) {
        c.blocks_part(image, (0, 0, image.width, image.height), (ox, oy));
        return;
    }
    for m in &picture.molten {
        let color = molten_color(m, phase, time);
        if FIRE[3..].contains(&color) {
            c.fill_rect(ox + m.x as i32 * 2, oy + m.y as i32 * 2, 2, 2, Color::new(color.r, color.g, color.b, (255.0 * alpha.clamp(0.0, 1.0)) as u8));
        }
    }
}

// ---------------------------------------------------------------------------
// What moves above it, composed as `pyro::Shape`s.

/// How often the plume puts up a puff (seconds) and how long one lives.
const PUFF_EVERY: f32 = 0.12;
const PUFF_LIFE: f32 = 3.6;

/// The plume over the crater at `time`: steam while it sleeps, dark ash
/// from the rumble through the cooling, leaning with the wind. Each puff
/// is born on a fixed tick and hashed, so the plume is the same picture
/// for the same clock.
pub fn plume(v: &Volcano, time: f32, t: &Tuning, out: &mut Vec<Shape>) {
    use crate::lava::hash3;
    let centre = v.centre();
    let seed = v.cell.0 * 97 + v.cell.1;
    let first = ((time - PUFF_LIFE) / PUFF_EVERY).floor() as i64;
    let last = (time / PUFF_EVERY).floor() as i64;
    for k in first..=last {
        let born = k as f32 * PUFF_EVERY;
        let age = time - born;
        if age < 0.0 {
            continue;
        }
        let at_birth = v.phase(born, t);
        let rate = match at_birth.stage {
            Stage::Asleep => 0.3,
            Stage::Rumble => 0.85,
            Stage::Erupt => 1.0,
            Stage::Cool => 0.55,
        };
        let ki = k as i32;
        if hash3(ki, seed, 1) > rate {
            continue;
        }
        let ash = matches!(at_birth.stage, Stage::Rumble | Stage::Erupt);
        let life = PUFF_LIFE * (0.65 + 0.35 * hash3(ki, seed, 2));
        let f = age / life;
        if f >= 1.0 {
            continue;
        }
        let rise = age * if ash { 30.0 } else { 18.0 };
        let lean = crate::pyro::smoke_lean(t, centre, born);
        let x = centre.x + (hash3(ki, seed, 3) - 0.5) * 8.0 + lean * rise + (age * 2.0 + k as f32).sin() * 1.5;
        let y = centre.y - 6.0 - rise;
        let r = (if ash { 5.0 } else { 3.5 }) + age * if ash { 8.0 } else { 5.0 };
        let r = r * (1.0 - (f - 0.6).max(0.0) * 0.5);
        let cover = if f < 0.7 { 1.0 } else { 1.0 - (f - 0.7) / 0.3 };
        let (body, shadow, lit) = if ash { (SMOKE[1], SMOKE[0], SMOKE[2]) } else { (SMOKE[3], SMOKE[2], SMOKE[5]) };
        out.push(Shape::Puff(Puff { pos: Position::new(x, y), radius: r, body, shadow: Some(shadow), lit: Some(lit), core: None, cover }));
    }
}

/// The eruption's show over the crater: the shock ring of light racing
/// out, the flash with its rays, three tongues of fire, molten drops
/// thrown up and falling back, violet lightning in the plume, and one
/// light over the crater. Empty unless erupting.
pub fn eruption(v: &Volcano, time: f32, t: &Tuning, out: &mut Vec<Shape>) {
    use crate::lava::hash3;
    let phase = v.phase(time, t);
    if phase.stage != Stage::Erupt {
        return;
    }
    let centre = v.centre();
    let a = phase.age;
    let n = phase.eruption as i32;
    let tick = (time * 10.0).floor() as i32;
    // The shock ring.
    if a < 0.8 {
        let k = 1.0 - a / 0.8;
        let rr = a / 0.8 * 300.0;
        let count = (rr * 0.5) as i32 + 8;
        for i in 0..count {
            let ang = i as f32 / count as f32 * std::f32::consts::TAU;
            if crate::pyro::bayer(i, tick) < k + 0.2 {
                let p = Position::new(centre.x + ang.cos() * rr, centre.y + ang.sin() * rr);
                out.push(Shape::Mark { pos: p, size: 2, color: Color::new(FIRE[6].r, FIRE[6].g, FIRE[6].b, (220.0 * k) as u8) });
            }
        }
    }
    // Molten drops thrown up and falling back.
    for k in 0..26 {
        let launch = hash3(k, n, 1) * 2.0;
        let tk = a - launch;
        if tk < 0.0 {
            continue;
        }
        let vz = 80.0 + hash3(k, n, 4) * 90.0;
        let z = vz * tk - 80.0 * tk * tk;
        if z < 0.0 {
            continue;
        }
        let dir = hash3(k, n, 5) * std::f32::consts::TAU;
        let sp = 10.0 + hash3(k, n, 6) * 24.0;
        let g = Position::new(centre.x + dir.cos() * sp * tk, centre.y + dir.sin() * sp * tk * 0.8);
        let step = (6 - (tk * 3.0).floor() as i32).clamp(3, 6) as usize;
        out.push(Shape::Mark { pos: Position::new(g.x, g.y - z), size: if hash3(k, n, 7) < 0.4 { 4 } else { 2 }, color: FIRE[step] });
    }
    // The fountain: three tongues of fire out of the crater.
    let lean = crate::pyro::smoke_lean(t, centre, time) * 0.4;
    let grow = (a / 0.25 + 0.2).min(1.0);
    crate::pyro::tongues(out, Position::new(centre.x, centre.y + 4.0), 22.0, 46.0 * grow, 3, crate::blast::seed_at(centre, 81), time, lean, 1.0);
    // The flash: a white disc and rays at the start.
    if a < 0.45 {
        let k = 1.0 - a / 0.45;
        for i in 0..12 {
            let ang = (i as f32 + hash3(i, 9, 3) * 0.6) / 12.0 * std::f32::consts::TAU;
            let len = (24.0 + hash3(i, 8, 3) * 44.0) * (0.4 + 0.6 * (1.0 - k));
            let from = Position::new(centre.x + ang.cos() * 12.0, centre.y + ang.sin() * 12.0);
            let to = Position::new(centre.x + ang.cos() * len, centre.y + ang.sin() * len);
            out.push(Shape::Line { from, to, width: 1.0, head: FIRE[6], tail: FIRE[5] });
        }
        out.push(Shape::Puff(Puff::plain(centre, 18.0 * k + 4.0, Color::new(255, 255, 255, 230))));
    }
    // Volcanic lightning in the plume.
    for k in 0..5 {
        let tb = 0.3 + hash3(k, n, 41) * 2.0;
        if a < tb || a > tb + 0.14 {
            continue;
        }
        let (mut x, mut y) = (centre.x + 10.0 + hash3(k, 2, 41) * 30.0, centre.y - 40.0 - hash3(k, 3, 41) * 30.0);
        for s in 0..6 {
            let from = Position::new(x, y);
            x += 4.0 + hash3(k, s, 42) * 10.0;
            y += (hash3(k, s, 43) - 0.6) * 18.0;
            out.push(Shape::Line { from, to: Position::new(x, y), width: 1.0, head: crate::pyro::TESLA[4], tail: crate::pyro::TESLA[3] });
            out.push(Shape::Glow { pos: Position::new(x, y), radius: 12.0, color: Color::new(0x8C, 0x40, 0xFF, 90) });
        }
    }
    // One light over the crater: a flare as it opens, then the fountain's
    // steady glow.
    let flare = (1.0 - a / 0.5).max(0.0);
    out.push(Shape::Glow { pos: centre, radius: 70.0 + 90.0 * flare, color: Color::new(255, 150, 60, (50.0 + 110.0 * flare) as u8) });
}

/// A bomb in the air: its smoke trail, its shadow on the ground, the rock
/// with its glowing cracks turning as it flies.
pub fn bomb(b: &LavaBomb, time: f32, t: &Tuning, out: &mut Vec<Shape>) {
    let at = |age: f32| {
        let mut past = *b;
        past.age = age.max(0.0);
        (past.ground_pos(t), past.height(t))
    };
    let (ground, h) = at(b.age);
    // The trail: puffs where it was a moment ago.
    for k in 2..=7 {
        let back = b.age - k as f32 * 0.035;
        if back <= 0.0 {
            break;
        }
        let (g, hh) = at(back);
        let cover = 1.0 - k as f32 / 7.0 * 0.9;
        out.push(Shape::Puff(Puff { pos: Position::new(g.x + k as f32 * 0.6, g.y - hh - k as f32 * 0.5), radius: 1.5 + k as f32 * 0.8, body: SMOKE[2], shadow: Some(SMOKE[1]), lit: Some(SMOKE[3]), core: None, cover }));
    }
    // Its shadow: smaller and fainter the higher it flies.
    let lift = (h / t.volcano_bomb_arc_px.max(1.0)).clamp(0.0, 1.0);
    out.push(Shape::Puff(Puff::plain(ground, 6.0 - 3.0 * lift, Color::new(0, 0, 0, (110.0 - 60.0 * lift) as u8))));
    let rock = Position::new(ground.x, ground.y - h);
    out.push(Shape::Puff(Puff { pos: rock, radius: 5.0, body: SMOKE[1], shadow: Some(SMOKE[0]), lit: Some(SMOKE[2]), core: None, cover: 1.0 }));
    let spin = ((time * 8.0) as i32 + (b.seed % 4) as i32).rem_euclid(4) as usize;
    const CRACKS: [(f32, f32); 4] = [(-2.0, 0.0), (0.0, 2.0), (2.0, -2.0), (0.0, -2.0)];
    for i in 0..3 {
        let (cx, cy) = CRACKS[(i + spin) % 4];
        out.push(Shape::Mark { pos: Position::new(rock.x + cx, rock.y + cy), size: 2, color: if i == 0 { FIRE[6] } else { FIRE[5] } });
    }
    out.push(Shape::Glow { pos: rock, radius: 14.0, color: Color::new(255, 140, 50, 140) });
}

/// The warning on the ground where a bomb will land: a faint red disc the
/// blast's size, a ring of blocks turning round it that blinks faster as
/// the bomb comes down, and a cross at its heart.
pub fn bomb_ring(b: &LavaBomb, time: f32, t: &Tuning, out: &mut Vec<Shape>) {
    let r = t.volcano_bomb_radius_px;
    let k = (b.flight(t)).clamp(0.0, 1.0);
    let blink = ((time * (4.0 + k * 10.0)).floor() as i32) % 2 == 0;
    out.push(Shape::Puff(Puff { pos: b.to, radius: r, body: Color::new(FIRE[3].r, FIRE[3].g, FIRE[3].b, (30.0 + 60.0 * k) as u8), shadow: None, lit: None, core: None, cover: 0.5 }));
    let count = (r * std::f32::consts::TAU / 8.0) as i32;
    let color = if blink { FIRE[6] } else { FIRE[4] };
    for i in 0..count {
        let ang = i as f32 / count as f32 * std::f32::consts::TAU + time;
        out.push(Shape::Mark { pos: Position::new(b.to.x + ang.cos() * r, b.to.y + ang.sin() * r), size: 2, color });
    }
    out.push(Shape::Line { from: Position::new(b.to.x, b.to.y - 6.0), to: Position::new(b.to.x, b.to.y + 6.0), width: 1.0, head: FIRE[6], tail: FIRE[6] });
    out.push(Shape::Line { from: Position::new(b.to.x - 6.0, b.to.y), to: Position::new(b.to.x + 6.0, b.to.y), width: 1.0, head: FIRE[6], tail: FIRE[6] });
}
