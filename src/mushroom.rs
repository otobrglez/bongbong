//! The mushrooms of the mushroom hunt (docs/mushroom-hunt-prd.md): what
//! one is, and its drawing, generic over `canvas::Canvas` so a thumbnail
//! and the round draw it alike. The rules are `simulation/forage.rs`.
//!
//! A mushroom is map data (`map::CellObject::Mushroom`) turned into a plain
//! value at `init`, not an `Obstacle`: no collider, no nav-grid cell, no
//! cover - the nav grid and the linter treat every obstacle as impassable,
//! and a tank has to drive into a mushroom to take it. A seat's hull box
//! touching the cell's square, grown by `mushroom_collect_pad_px`, takes it
//! (`Mushroom::in_reach`).
//!
//! **The look is the picks' (the "Mushroom picks" artifact):** M1, the fly
//! agaric - a domed cap with white spots, a skirt on the stem - at S2, 64 px
//! across (32 design pixels of 2 px blocks), so it stands taller and wider
//! than a tank's 36 px hull; C1, the cap stepping through six loud ramps,
//! one step every `mushroom_cycle_seconds`, each mushroom from its own
//! hashed place in the cycle; P1, the pop (`pop`). The caps' ramps are
//! deliberately off the Puny palette, like the crates' symbols: a mushroom
//! has to be spotted from across the field. The stem, the spots and the
//! outline are on it. Colours step from ramp to ramp and never blend.
//!
//! The art is worked out once (`raster`), as cells of a material and a
//! ramp step, then turned into horizontal runs per cap ramp (`runs`), so a
//! frame draws a mushroom as a few dozen rectangles. No RNG anywhere; every
//! choice is hashed from the cell.

use std::sync::OnceLock;

use crate::canvas::Canvas;
use crate::math::{Color, Vec2};
use crate::pyro::bayer;
use crate::{Position, OBSTACLE_GRID_SIZE};

/// The most mushrooms a map holds: what the wire's one-bit-each mask
/// carries (`net::wire::Snapshot::mushrooms`). A map's cells past it are
/// left out, in `MapFile::iter_cells` order, and the linter says so.
pub const MUSHROOM_MAX: usize = 64;

/// Design pixels a side, each a 2 px block: 64 px of art.
pub const DESIGN: i32 = 32;

/// A design pixel on the field.
pub const BLOCK: i32 = 2;

/// How far below its cell's centre the art stands: the stem's foot sits
/// near the cell's bottom edge, so the cap rises over the cells north of
/// it the way a tree's crown does.
pub const BASE_DROP: f32 = 10.0;

/// The six cap ramps, light to dark: red, gold, teal, blue, magenta,
/// violet. Off the Puny palette on purpose (module docs).
pub const RAMPS: [[Color; 5]; 6] = [
    [rgb(0xff, 0xb3, 0x8a), rgb(0xff, 0x5a, 0x3c), rgb(0xe8, 0x34, 0x2a), rgb(0xa8, 0x23, 0x2a), rgb(0x5c, 0x17, 0x24)],
    [rgb(0xff, 0xf1, 0xa0), rgb(0xff, 0xd9, 0x3d), rgb(0xf0, 0xa0, 0x20), rgb(0xb5, 0x6a, 0x18), rgb(0x5c, 0x34, 0x10)],
    [rgb(0xb8, 0xff, 0xf0), rgb(0x3d, 0xff, 0xd0), rgb(0x00, 0xc8, 0x90), rgb(0x00, 0x7e, 0x60), rgb(0x00, 0x3a, 0x2c)],
    [rgb(0xbf, 0xe4, 0xff), rgb(0x5a, 0xb4, 0xff), rgb(0x2a, 0x6f, 0xf0), rgb(0x1c, 0x3f, 0xa8), rgb(0x14, 0x21, 0x5c)],
    [rgb(0xff, 0xc4, 0xf4), rgb(0xff, 0x5a, 0xd8), rgb(0xd4, 0x2a, 0xb0), rgb(0x8a, 0x1a, 0x78), rgb(0x45, 0x10, 0x3e)],
    [rgb(0xe2, 0xc8, 0xff), rgb(0xb0, 0x7a, 0xff), rgb(0x7a, 0x3e, 0xf0), rgb(0x4a, 0x22, 0xa0), rgb(0x24, 0x12, 0x4f)],
];

/// The stem and the spots, on the Puny palette's neutral greys.
const STEM: [Color; 5] = [rgb(0xf0, 0xf0, 0xf0), rgb(0xda, 0xda, 0xda), rgb(0xc1, 0xc1, 0xc1), rgb(0x9e, 0x9e, 0x96), rgb(0x7e, 0x7e, 0x7e)];
const SPOT: [Color; 3] = [rgb(0xff, 0xff, 0xff), rgb(0xf0, 0xf0, 0xf0), rgb(0xc1, 0xc1, 0xc1)];
const OUTLINE: Color = rgb(0x25, 0x25, 0x25);
/// The drop shadow, an obstacle's.
const SHADOW: Color = Color::new(0x14, 0x1e, 0x14, 82);

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::new(r, g, b, 255)
}

/// One mushroom of the round: its cell, the cell's centre, whether a seat
/// has taken it, and the round time it was taken at - what its pop
/// (`draw_pop`) plays from, so the pop is drawn straight off the round,
/// the replica's as much as the room's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mushroom {
    pub col: i32,
    pub row: i32,
    pub at: Position,
    pub taken: bool,
    pub taken_at: Option<f32>,
}

impl Mushroom {
    pub fn new(col: i32, row: i32) -> Mushroom {
        Mushroom { col, row, at: crate::map::cell_to_world(col, row), taken: false, taken_at: None }
    }

    /// Take it at round time `time`.
    pub fn take(&mut self, time: f32) {
        self.taken = true;
        self.taken_at = Some(time);
    }

    /// Where the art stands on the ground: its foot, the key the standing
    /// walk sorts it by.
    pub fn base(&self) -> Position {
        Vec2::new(self.at.x, self.at.y + BASE_DROP)
    }

    /// Whether a hull box centred at `hull_center` with half-extents
    /// `hull_half` touches this mushroom's cell grown by `pad` on every
    /// side - `Pickup::in_reach`'s box test on the cell, whatever the
    /// art's size, so the big cap never collects from two cells away.
    pub fn in_reach(&self, hull_center: Position, hull_half: Position, pad: f32) -> bool {
        let half = OBSTACLE_GRID_SIZE * 0.5 + pad;
        (hull_center.x - self.at.x).abs() <= hull_half.x + half && (hull_center.y - self.at.y).abs() <= hull_half.y + half
    }

    /// The cap's ramp at round time `time`: its own hashed place in the
    /// cycle, stepped on by one ramp every `cycle` seconds.
    pub fn ramp(&self, time: f32, cycle: f32) -> usize {
        ramp_at(self.col, self.row, time, cycle)
    }
}

/// The cap ramp of the mushroom on cell (`col`, `row`) at `time`.
pub fn ramp_at(col: i32, row: i32, time: f32, cycle: f32) -> usize {
    let seed = (crate::pyro::unit(col as u32 ^ (row as u32).wrapping_mul(0x9E37_79B9), 0x6d75) * RAMPS.len() as f32) as usize;
    let step = if cycle > 0.0 { (time.max(0.0) / cycle) as usize } else { 0 };
    (seed + step) % RAMPS.len()
}

/// The cap's bright step on (`col`, `row`) at `time`: the colour a mark
/// standing for this mushroom wears (the HUD's glyph, the minimap, the
/// off-screen arrow).
pub fn cap_color(col: i32, row: i32, time: f32, cycle: f32) -> Color {
    RAMPS[ramp_at(col, row, time, cycle)][1]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mat {
    Cap,
    Spot,
    Stem,
    Gills,
    Outline,
}

/// One design pixel of the art: what it is and its ramp step (0 lit .. 4
/// dark).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Px {
    mat: Mat,
    step: u8,
}

/// The art: `DESIGN` x `DESIGN` design pixels, row-major, and the lowest
/// row anything stands on.
struct Raster {
    cells: Vec<Option<Px>>,
    base: i32,
}

fn in_ellipse(u: f32, v: f32, cx: f32, cy: f32, rx: f32, ry: f32) -> bool {
    ((u - cx) / rx).powi(2) + ((v - cy) / ry).powi(2) <= 1.0
}

/// A cap's step at (`u`, `v`): lit from the top left in five bands with
/// dithered edges, its rim darkened.
fn cap_step(u: f32, v: f32, cx: f32, cy: f32, rx: f32, ry: f32, x: i32, y: i32) -> u8 {
    let (nx, ny) = ((u - cx) / rx, (v - cy) / ry);
    let b = 0.6 * -nx + 0.8 * -ny + (bayer(x, y) - 0.5) * 0.22;
    let mut s = if b > 0.62 {
        0
    } else if b > 0.18 {
        1
    } else if b > -0.28 {
        2
    } else if b > -0.7 {
        3
    } else {
        4
    };
    if nx * nx + ny * ny > 0.86 && s < 3 && ny > -0.2 {
        s = 3;
    }
    s
}

/// A stem's step: lit from the left.
fn stem_step(u: f32, cx: f32, hw: f32, x: i32, y: i32) -> u8 {
    let sx = (u - cx) / hw + (bayer(x, y) - 0.5) * 0.3;
    if sx < -0.3 {
        0
    } else if sx < 0.4 {
        1
    } else {
        2
    }
}

/// The fly agaric at (`u`, `v`) in the unit square, or nothing.
fn agaric(u: f32, v: f32, x: i32, y: i32) -> Option<Px> {
    const SPOTS: [(f32, f32, f32); 6] =
        [(0.32, 0.30, 0.065), (0.56, 0.19, 0.07), (0.71, 0.37, 0.055), (0.45, 0.41, 0.05), (0.21, 0.44, 0.04), (0.81, 0.47, 0.035)];
    let (cx, cy, rx, ry) = (0.5, 0.47, 0.44, 0.37);
    if v <= 0.5 && in_ellipse(u, v, cx, cy, rx, ry) {
        let step = cap_step(u, v, cx, cy, rx, ry, x, y);
        if SPOTS.iter().any(|&(sx, sy, r)| (u - sx).powi(2) + (v - sy).powi(2) <= r * r) {
            let spot = if step > 2 {
                2
            } else if step > 0 {
                1
            } else {
                0
            };
            return Some(Px { mat: Mat::Spot, step: spot });
        }
        return Some(Px { mat: Mat::Cap, step });
    }
    if v > 0.5 && v <= 0.555 && in_ellipse(u, v, 0.5, 0.5, 0.37, 0.055) {
        return Some(Px { mat: Mat::Gills, step: 4 });
    }
    // The skirt round the stem.
    if (0.6..=0.655).contains(&v) && (u - 0.5).abs() < 0.155 {
        return Some(Px { mat: Mat::Stem, step: if v < 0.625 { 0 } else { 3 } });
    }
    let hw = 0.1 + (v - 0.55) * 0.07;
    if v > 0.5 && v <= 0.9 && (u - 0.5).abs() < hw {
        return Some(Px { mat: Mat::Stem, step: if v < 0.6 { 3 } else { stem_step(u, 0.5, hw, x, y) } });
    }
    if in_ellipse(u, v, 0.5, 0.885, 0.15, 0.06) {
        return Some(Px { mat: Mat::Stem, step: stem_step(u, 0.5, 0.15, x, y) });
    }
    None
}

fn raster() -> &'static Raster {
    static RASTER: OnceLock<Raster> = OnceLock::new();
    RASTER.get_or_init(|| {
        let n = DESIGN;
        let at = |x: i32, y: i32| agaric((x as f32 + 0.5) / n as f32, (y as f32 + 0.5) / n as f32, x, y);
        let shape: Vec<Option<Px>> = (0..n * n).map(|i| at(i % n, i / n)).collect();
        let filled = |x: i32, y: i32| x >= 0 && y >= 0 && x < n && y < n && shape[(y * n + x) as usize].is_some();
        // A one-pixel outline round the whole of it, the tanks' rule.
        let cells: Vec<Option<Px>> = (0..n * n)
            .map(|i| {
                let (x, y) = (i % n, i / n);
                shape[i as usize].or_else(|| {
                    [(1, 0), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .any(|&(dx, dy)| filled(x + dx, y + dy))
                        .then_some(Px { mat: Mat::Outline, step: 0 })
                })
            })
            .collect();
        let base = (0..n * n).filter(|&i| cells[i as usize].is_some()).map(|i| i / n).max().unwrap_or(n - 1);
        Raster { cells, base }
    })
}

/// A horizontal run of one colour: design-pixel row and column, length.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Run {
    x: i32,
    y: i32,
    len: i32,
    color: Color,
}

fn color_of(px: Px, ramp: usize) -> Color {
    match px.mat {
        Mat::Cap => RAMPS[ramp][px.step as usize],
        Mat::Spot => SPOT[px.step.min(2) as usize],
        Mat::Stem => STEM[px.step as usize],
        Mat::Gills => RAMPS[ramp][4],
        Mat::Outline => OUTLINE,
    }
}

/// The art as runs, once per cap ramp.
fn runs(ramp: usize) -> &'static [Run] {
    static RUNS: OnceLock<Vec<Vec<Run>>> = OnceLock::new();
    &RUNS.get_or_init(|| {
        let r = raster();
        (0..RAMPS.len())
            .map(|ramp| {
                let mut out = Vec::new();
                for y in 0..DESIGN {
                    let mut x = 0;
                    while x < DESIGN {
                        let Some(px) = r.cells[(y * DESIGN + x) as usize] else {
                            x += 1;
                            continue;
                        };
                        let color = color_of(px, ramp);
                        let start = x;
                        while x < DESIGN && r.cells[(y * DESIGN + x) as usize].is_some_and(|p| color_of(p, ramp) == color) {
                            x += 1;
                        }
                        out.push(Run { x: start, y, len: x - start, color });
                    }
                }
                out
            })
            .collect()
    })[ramp]
}

/// The art's top-left corner in field px for a mushroom standing at
/// `base`, on the 2 px grid.
fn origin(base: Position) -> (i32, i32) {
    let snap = |v: f32| ((v / BLOCK as f32).floor() as i32) * BLOCK;
    (snap(base.x) - DESIGN * BLOCK / 2, snap(base.y) - (raster().base + 1) * BLOCK)
}

/// The drop shadow under a mushroom standing at `base`, leaning the way
/// every shadow on the field does.
pub fn draw_shadow(c: &mut impl Canvas, base: Position) {
    let (rx, ry) = (DESIGN * 34 / 100, (DESIGN * 9 / 100).max(2));
    let (bx, by) = ((base.x / BLOCK as f32).floor() as i32 + rx / 4, (base.y / BLOCK as f32).floor() as i32);
    for dy in -ry..=ry {
        let half = (rx as f32 * (1.0 - (dy as f32 / ry as f32).powi(2)).max(0.0).sqrt()).round() as i32;
        if half > 0 {
            c.fill_rect((bx - half) * BLOCK, (by + dy) * BLOCK, half * 2 * BLOCK, BLOCK, SHADOW);
        }
    }
}

/// A mushroom standing at `base` with its cap in ramp `ramp`.
pub fn draw_at(c: &mut impl Canvas, base: Position, ramp: usize, shadow: bool) {
    if shadow {
        draw_shadow(c, base);
    }
    let (ox, oy) = origin(base);
    for run in runs(ramp % RAMPS.len()) {
        c.fill_rect(ox + run.x * BLOCK, oy + run.y * BLOCK, run.len * BLOCK, BLOCK, run.color);
    }
}

/// A mushroom on the field at round time `time`: standing while it is
/// out, its pop for `POP_SECONDS` after it is taken, then nothing.
pub fn draw_mushroom(c: &mut impl Canvas, m: &Mushroom, time: f32, cycle: f32, shadow: bool) {
    match (m.taken, m.taken_at) {
        (false, _) => draw_at(c, m.base(), m.ramp(time, cycle), shadow),
        (true, Some(at)) if (0.0..POP_SECONDS).contains(&(time - at)) => {
            draw_pop(c, m.base(), m.col, m.row, m.ramp(at, cycle), time - at);
        }
        _ => {}
    }
}

/// The HUD's mushroom: 7 x 7 blocks, `1` the outline, `2` the cap, `3`
/// the stem and spots.
pub const GLYPH: [&str; 7] = ["0011100", "0122210", "1232321", "1111111", "0013100", "0013100", "0113110"];

/// The glyph's blocks as (column, row, colour), its cap in `cap`.
pub fn glyph_blocks(cap: Color) -> impl Iterator<Item = (i32, i32, Color)> {
    GLYPH.iter().enumerate().flat_map(move |(y, row)| {
        row.bytes().enumerate().filter_map(move |(x, b)| {
            let color = match b {
                b'1' => OUTLINE,
                b'2' => cap,
                b'3' => STEM[0],
                _ => return None,
            };
            Some((x as i32, y as i32, color))
        })
    })
}

/// How long the pop (`pop`) plays, in seconds.
pub const POP_SECONDS: f32 = 1.2;

/// The pop a mushroom goes in the moment a seat takes it (the P1 pick),
/// `age` seconds on: the art squashed for a tenth of a second, then a ring
/// racing out and a shower of spores in the cap's ramp and white, falling
/// a little and dithering away. Pure in its inputs, every spore hashed
/// from the cell, so a replica pops the same.
pub fn draw_pop(c: &mut impl Canvas, base: Position, col: i32, row: i32, ramp: usize, age: f32) {
    let ramp = ramp % RAMPS.len();
    let ramp_colors = RAMPS[ramp];
    let mid = Vec2::new(base.x, base.y - DESIGN as f32 * BLOCK as f32 * 0.45);
    if age < 0.1 {
        // Squashed: wider and lower about the foot, drawn run by run.
        let (ox, oy) = origin(base);
        let r = raster();
        let tall = (r.base + 1) as f32;
        for run in runs(ramp) {
            let y = oy as f32 + (tall - (tall - run.y as f32) * 0.72) * BLOCK as f32;
            let x = base.x + ((ox + run.x * BLOCK) as f32 - base.x) * 1.18;
            c.fill_rect(x.round() as i32, y.round() as i32, ((run.len * BLOCK) as f32 * 1.18).ceil() as i32, BLOCK, run.color);
        }
        return;
    }
    let age = age - 0.1;
    let seed = (col as u32).wrapping_mul(73_856_093) ^ (row as u32).wrapping_mul(19_349_663);
    let block = |c: &mut dyn FnMut(i32, i32, Color), x: f32, y: f32, color: Color, keep: f32| {
        let (bx, by) = crate::pyro::block_of(x, y);
        if bayer(bx, by) < keep {
            c(bx, by, color);
        }
    };
    let mut put = |bx: i32, by: i32, color: Color| c.fill_rect(bx * BLOCK, by * BLOCK, BLOCK, BLOCK, color);
    // The ring.
    if age < 0.32 {
        let r = 6.0 + age / 0.32 * DESIGN as f32 * BLOCK as f32 * 0.55;
        for k in 0..48 {
            let a = k as f32 / 48.0 * std::f32::consts::TAU;
            block(&mut put, mid.x + crate::trig::cos(a) * r, mid.y + crate::trig::sin(a) * r * 0.7, ramp_colors[1], 1.0 - age / 0.32);
        }
    }
    // The spores.
    for k in 0..28u32 {
        let life = 0.6 + crate::pyro::unit(seed, k * 3 + 1) * 0.5;
        if age > life {
            continue;
        }
        let a = crate::pyro::unit(seed, k * 3 + 2) * std::f32::consts::TAU;
        let speed = 36.0 + crate::pyro::unit(seed, k * 3 + 3) * 68.0;
        let d = speed * (1.0 - (-age * 3.0).exp()) / 3.0 * 3.0;
        let x = mid.x + crate::trig::cos(a) * d;
        let y = mid.y + crate::trig::sin(a) * d * 0.75 + age * age * 28.0;
        let color = if k % 4 == 0 { SPOT[0] } else { ramp_colors[(k % 3) as usize] };
        block(&mut put, x, y, color, 1.15 - age / life);
        if k % 3 == 0 {
            block(&mut put, x + BLOCK as f32, y, color, 1.15 - age / life);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The art stands on its foot, inside its square, bigger than a tank.
    #[test]
    fn the_art_is_bigger_than_a_tank_and_fits_its_square() {
        let r = raster();
        let (mut x0, mut x1, mut y0) = (DESIGN, 0, DESIGN);
        for (i, c) in r.cells.iter().enumerate() {
            if c.is_some() {
                let (x, y) = (i as i32 % DESIGN, i as i32 / DESIGN);
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
            }
        }
        let wide = (x1 - x0 + 1) * BLOCK;
        let tall = (r.base - y0 + 1) * BLOCK;
        assert!(wide > 40 && tall > 40, "a mushroom is {wide} x {tall} px, no bigger than a tank's sprite");
        assert!(x0 > 0 && x1 < DESIGN - 1, "the outline fits the square");
    }

    /// Every run is one colour from the art's palette, and the runs of a
    /// ramp cover the art exactly once.
    #[test]
    fn the_runs_cover_the_art_once() {
        let r = raster();
        for ramp in 0..RAMPS.len() {
            let mut seen = vec![0u8; (DESIGN * DESIGN) as usize];
            for run in runs(ramp) {
                for x in run.x..run.x + run.len {
                    seen[(run.y * DESIGN + x) as usize] += 1;
                }
            }
            for (i, c) in r.cells.iter().enumerate() {
                assert_eq!(seen[i], u8::from(c.is_some()), "pixel {i} of ramp {ramp}");
            }
        }
    }

    /// The cap steps through every ramp on the cycle, a mushroom from its
    /// own place, and stands still with no cycle.
    #[test]
    fn the_cap_steps_through_the_ramps() {
        let seen: std::collections::BTreeSet<usize> = (0..12).map(|k| ramp_at(3, 4, k as f32 * 0.35 + 0.01, 0.35)).collect();
        assert_eq!(seen.len(), RAMPS.len());
        assert_eq!(ramp_at(3, 4, 0.0, 0.35), ramp_at(3, 4, 0.34, 0.35));
        assert_ne!(ramp_at(3, 4, 0.0, 0.35), ramp_at(3, 4, 0.36, 0.35));
        assert_eq!(ramp_at(3, 4, 0.0, 0.0), ramp_at(3, 4, 99.0, 0.0));
        let starts: std::collections::BTreeSet<usize> = (0..20).map(|c| ramp_at(c, 7, 0.0, 0.35)).collect();
        assert!(starts.len() > 1, "every mushroom starts the cycle at the same place");
    }

    /// The collect box is the cell's, grown by the pad.
    #[test]
    fn it_is_taken_by_touch_on_its_cell() {
        let m = Mushroom::new(4, 4);
        let half = Vec2::new(16.0, 16.0);
        let reach = 16.0 + 16.0 + 6.0;
        assert!(m.in_reach(Vec2::new(m.at.x + reach, m.at.y), half, 6.0));
        assert!(!m.in_reach(Vec2::new(m.at.x + reach + 1.0, m.at.y), half, 6.0));
        assert!(!m.in_reach(Vec2::new(m.at.x, m.at.y - 64.0), half, 6.0), "the cap overhead collects nothing");
    }
}
