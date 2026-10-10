//! Ground memory: the tread marks a round's hulls press into the field
//! (docs/ground-memory.md).
//!
//! The field keeps a wear grid of 2 px blocks - the grain everything that
//! lingers on the floor lies on (docs/effects.md) - for each map cell a
//! hull has touched: per block how many passes pressed it, what kind of
//! mark it is (a pad, a grouser's bar, a smear, churned ground, a drop, a
//! mud splat, a burnt-in mark), when it was last pressed and how wet. The
//! simulation stamps it (`simulation/wear.rs`) from hull poses, the ground
//! and the round clock alone: no RNG, every cosmetic choice hashed, so a
//! seeded replay presses the same ground and a replica presses its own
//! from the hulls it is shown. Nothing that plays reads it.
//!
//! What a block looks like is worked out when its cell is baked, from its
//! age and the round's ground and sky: the grousers dissolve into the pad,
//! a single pass thins to half its blocks and holds there for the round,
//! repeated passes deepen into ruts and, on grass, into the tileset's own
//! path, rain fills the ruts and falling snow fills the marks back in. The
//! bake is a `TileAtlas` of the cells the window shows (`refresh_pictures`,
//! the lava layer's pattern), so a room server, which never draws, never
//! pays for a picture, and a mark costs nothing a frame once it is baked.

use std::cell::{Ref, RefCell};
use std::collections::BTreeMap;

use crate::canvas::{TileAtlas, TILE_BLOCKS};
use crate::map::{Theme, Weather};
use crate::math::{Color, Rectangle};
use crate::tuning::Tuning;
use crate::{Position, TreadProfile, OBSTACLE_GRID_SIZE};

/// Field px a side of a block.
pub const BLOCK_PX: f32 = 2.0;

/// Blocks in a cell.
const BLOCKS: usize = TILE_BLOCKS * TILE_BLOCKS;

/// A block's press is kept in sixteenths of a pass.
const PRESS_ONE: f32 = 16.0;

/// The most cells the pictures hold at once: a window never shows more,
/// and it keeps the atlas inside the 4,096 px a WebGL texture is sure of.
const MAX_PICTURE_CELLS: usize = 2048;

/// What a worn block is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    None,
    /// Where a run's pad rolled.
    Pad,
    /// A grouser's bar pressed into it.
    Grouser,
    /// A run sliding sideways: pressed with no pattern.
    Smear,
    /// A pivot's or a spinning start's churned ground.
    Churn,
    /// A drop of ford water off a wet hull.
    Drip,
    /// A mud clod a hull threw.
    Splat,
    /// Burnt in by a wreck or a drum's blast: never thins.
    Char,
}

impl Kind {
    const ALL: [Kind; 8] = [Kind::None, Kind::Pad, Kind::Grouser, Kind::Smear, Kind::Churn, Kind::Drip, Kind::Splat, Kind::Char];

    fn bits(self) -> u8 {
        Kind::ALL.iter().position(|k| *k == self).unwrap_or(0) as u8
    }
}

/// One 2 px block of worn ground, four bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Block {
    /// Passes pressed into it, in sixteenths, saturating. On open water,
    /// the silt in it, of a full brown.
    press: u8,
    /// The kind (bits 0-2), the way the hull faced (3-4: up, right, down,
    /// left), a berm (5) and how wet, in thirds (6-7).
    bits: u8,
    /// The round time it was last pressed, in eighths of a second,
    /// saturating (two hours and a quarter).
    laid: u16,
}

impl Block {
    /// Passes pressed into it.
    pub fn passes(self) -> f32 {
        self.press as f32 / PRESS_ONE
    }

    pub fn kind(self) -> Kind {
        Kind::ALL[(self.bits & 0b111) as usize]
    }

    /// The way the hull faced: 0 up, 1 right, 2 down, 3 left.
    pub fn facing(self) -> u8 {
        (self.bits >> 3) & 0b11
    }

    /// Soil a run pushed up beside it, with no pass of its own.
    pub fn berm(self) -> bool {
        self.bits & 0b10_0000 != 0
    }

    /// How wet it was pressed, 0 to 1.
    pub fn wet(self) -> f32 {
        (self.bits >> 6) as f32 / 3.0
    }

    /// When it was last pressed, round seconds.
    pub fn laid(self) -> f32 {
        self.laid as f32 / 8.0
    }

    /// Silt in an open-water block, 0 to 1.
    pub fn silt(self) -> f32 {
        self.press as f32 / 255.0
    }

    fn is_empty(self) -> bool {
        self.press == 0 && self.bits == 0
    }

    fn set_kind(&mut self, kind: Kind) {
        self.bits = (self.bits & !0b111) | kind.bits();
    }

    fn set_facing(&mut self, facing: u8) {
        self.bits = (self.bits & !0b1_1000) | ((facing & 0b11) << 3);
    }

    fn set_berm(&mut self, berm: bool) {
        self.bits = if berm { self.bits | 0b10_0000 } else { self.bits & !0b10_0000 };
    }

    fn set_wet(&mut self, wet: f32) {
        let q = (wet.clamp(0.0, 1.0) * 3.0).round() as u8;
        self.bits = (self.bits & 0b11_1111) | (q << 6);
    }

    fn set_laid(&mut self, now: f32) {
        self.laid = (now * 8.0).clamp(0.0, u16::MAX as f32) as u16;
    }

    fn add_passes(&mut self, passes: f32) {
        let add = (passes * PRESS_ONE).round().max(1.0);
        self.press = (self.press as f32 + add).min(255.0) as u8;
    }
}

/// What a hull's runs are on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Surface {
    /// The tileset's grass; under the desert theme, its pale dust.
    #[default]
    Grass,
    /// A dirt road - and the floor of a rod's dry crater.
    Road,
    /// A drift of sand.
    Sand,
    /// A ford: wading water that takes no mark, only silt.
    Ford,
    /// Water or lava no hull can enter.
    Deep,
    /// A frozen lake or ford: scratches only.
    Ice,
}

impl Surface {
    pub fn is_water(self) -> bool {
        matches!(self, Surface::Ford | Surface::Deep)
    }
}

/// The part of the round's sky the ground answers to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sky {
    #[default]
    Dry,
    /// Rain or a storm: the ground is mud.
    Rain,
    /// Snow over the field.
    Snow,
}

impl Sky {
    pub fn of(weather: Weather) -> Sky {
        match weather {
            Weather::Rain | Weather::Storm => Sky::Rain,
            Weather::Snow => Sky::Snow,
            _ => Sky::Dry,
        }
    }
}

/// The round's ground and sky, as the marks see them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    pub theme: Theme,
    pub sky: Sky,
}

/// How much a pass on `surface` presses under `look`, before the chassis.
pub fn press_factor(t: &Tuning, look: Look, surface: Surface) -> f32 {
    let ground = match surface {
        Surface::Road => t.wear_press_road,
        Surface::Sand => t.wear_press_sand,
        Surface::Grass if look.theme == Theme::Desert => t.wear_press_dust,
        _ => 1.0,
    };
    let sky = match look.sky {
        Sky::Rain if surface != Surface::Sand => t.wear_press_rain,
        Sky::Snow => t.wear_press_snow,
        _ => 1.0,
    };
    ground * sky
}

/// Whether a run pushes a berm up beside it on `surface`: loose ground.
pub fn is_soft(look: Look, surface: Surface) -> bool {
    match surface {
        Surface::Ford | Surface::Deep | Surface::Ice => false,
        _ if look.sky == Sky::Snow => true,
        Surface::Sand => true,
        Surface::Grass => look.theme == Theme::Desert || look.sky == Sky::Rain,
        Surface::Road => false,
    }
}

/// Whether a sandstorm's gust scours marks off `surface`: sand, desert
/// dust and its roads, anything under snow.
pub fn erodes(look: Look, surface: Surface) -> bool {
    match surface {
        Surface::Ford | Surface::Deep | Surface::Ice => false,
        _ if look.sky == Sky::Snow => true,
        Surface::Sand => true,
        _ => look.theme == Theme::Desert,
    }
}

/// How a hull's runs meet the ground in a step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Roll {
    #[default]
    Rolling,
    /// Moving sideways across its runs: a drift through a turn, a knock,
    /// a well's pull.
    Sliding,
    /// Turning in place, or its tracks spinning on a slippery start.
    Pivoting,
}

/// One hull's runs pressed at one pose.
#[derive(Clone, Copy, Debug)]
pub struct Stamp<'a> {
    pub at: Position,
    /// The hull's drawn heading, degrees, 0 up and clockwise.
    pub heading: f32,
    pub profile: &'a TreadProfile,
    /// `Tank::scale`: tile px to field px.
    pub scale: f32,
    /// Passes one pass of this hull presses.
    pub press: f32,
    pub roll: Roll,
    /// How wet its tracks are, 0 to 1.
    pub wet: f32,
    /// Where its grouser ladder starts, tile px.
    pub phase: f32,
}

/// One map cell's worn blocks.
#[derive(Clone)]
struct Cell {
    blocks: [Block; BLOCKS],
    /// The round time a hull last pressed it: what the cap evicts by.
    pressed: f32,
    /// Bumped whenever a block changes how it looks, so the picture knows
    /// to bake it again.
    version: u32,
    /// Some block holds silt.
    silty: bool,
}

impl Cell {
    fn new(now: f32) -> Box<Cell> {
        Box::new(Cell { blocks: [Block::default(); BLOCKS], pressed: now, version: 0, silty: false })
    }

    fn is_empty(&self) -> bool {
        self.blocks.iter().all(|b| b.is_empty())
    }
}

/// The cell holding field block `(bx, by)`, and the block's place in it.
/// Cell `(c, r)` is centred on `(c, r) * 32`, as every tile's.
fn locate(bx: i32, by: i32) -> ((i32, i32), usize) {
    let half = TILE_BLOCKS as i32 / 2;
    let n = TILE_BLOCKS as i32;
    let (cx, cy) = ((bx + half).div_euclid(n), (by + half).div_euclid(n));
    let (u, v) = ((bx + half).rem_euclid(n), (by + half).rem_euclid(n));
    ((cx, cy), (v * n + u) as usize)
}

/// Field block `i` of cell `cell`.
fn block_of(cell: (i32, i32), i: usize) -> (i32, i32) {
    let n = TILE_BLOCKS as i32;
    let half = n / 2;
    (cell.0 * n - half + (i % TILE_BLOCKS) as i32, cell.1 * n - half + (i / TILE_BLOCKS) as i32)
}

/// The field px at the centre of block `(bx, by)`.
fn block_centre(bx: i32, by: i32) -> Position {
    Position::new((bx as f32 + 0.5) * BLOCK_PX, (by as f32 + 0.5) * BLOCK_PX)
}

/// The block holding field px `at`.
fn block_at(at: Position) -> (i32, i32) {
    ((at.x / BLOCK_PX).floor() as i32, (at.y / BLOCK_PX).floor() as i32)
}

/// The surface of each quarter of a cell (16 px, the grain a sand drift's
/// corners are laid on), top-left, top-right, bottom-left, bottom-right.
fn quarters(cell: (i32, i32), surface: &dyn Fn(Position) -> Surface) -> [Surface; 4] {
    let c = crate::map::cell_to_world(cell.0, cell.1);
    let q = OBSTACLE_GRID_SIZE / 4.0;
    [
        surface(Position::new(c.x - q, c.y - q)),
        surface(Position::new(c.x + q, c.y - q)),
        surface(Position::new(c.x - q, c.y + q)),
        surface(Position::new(c.x + q, c.y + q)),
    ]
}

fn quarter_of(i: usize) -> usize {
    let (u, v) = (i % TILE_BLOCKS, i / TILE_BLOCKS);
    (v / (TILE_BLOCKS / 2)) * 2 + u / (TILE_BLOCKS / 2)
}

/// A unit hash of a block, for the cosmetic choices a bake makes.
fn unit(x: i32, y: i32, salt: u32) -> f32 {
    (crate::blast::seed_at(Position::new(x as f32, y as f32), salt) % 10_000) as f32 / 10_000.0
}

/// Smooth noise over the blocks, a few blocks to a lump: where rain pools
/// along a rut.
fn clump(bx: i32, by: i32) -> f32 {
    let lump = |cell: i32, salt: u32| {
        let (gx, gy) = (bx.div_euclid(cell), by.div_euclid(cell));
        let (fx, fy) = ((bx.rem_euclid(cell) as f32 + 0.5) / cell as f32, (by.rem_euclid(cell) as f32 + 0.5) / cell as f32);
        let (a, b, c, d) = (unit(gx, gy, salt), unit(gx + 1, gy, salt), unit(gx, gy + 1, salt), unit(gx + 1, gy + 1, salt));
        let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
        a + (b - a) * ux + (c - a) * uy + (a - b - c + d) * ux * uy
    };
    0.62 * lump(9, 51) + 0.38 * lump(4, 52)
}

/// The ground's memory of the round's tread marks.
pub struct WearGrid {
    cells: BTreeMap<(i32, i32), Box<Cell>>,
    /// Blocks pressed by a new pass, all round: what a test or the dev
    /// server counts marks by.
    presses: u64,
    max_cells: usize,
    /// Round seconds the silt has not yet drifted.
    silt_due: f32,
    pictures: RefCell<Pictures>,
}

impl Default for WearGrid {
    fn default() -> Self {
        WearGrid { cells: BTreeMap::new(), presses: 0, max_cells: usize::MAX, silt_due: 0.0, pictures: RefCell::new(Pictures::default()) }
    }
}

impl WearGrid {
    /// A fresh round's ground, keeping at most `max_cells` worn cells.
    pub fn new(max_cells: usize) -> WearGrid {
        WearGrid { max_cells: max_cells.max(1), ..WearGrid::default() }
    }

    /// No ground is worn.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Blocks pressed by a new pass so far this round.
    pub fn presses(&self) -> u64 {
        self.presses
    }

    /// Cells with any wear.
    pub fn worn_cells(&self) -> usize {
        self.cells.len()
    }

    /// The block under field px `at`, if it is worn.
    pub fn block(&self, at: Position) -> Option<Block> {
        let (bx, by) = block_at(at);
        let (cell, i) = locate(bx, by);
        self.cells.get(&cell).map(|c| c.blocks[i]).filter(|b| !b.is_empty())
    }

    /// Passes pressed into the block under `at`.
    pub fn passes_at(&self, at: Position) -> f32 {
        self.block(at).map_or(0.0, Block::passes)
    }

    /// Every worn block and where it is (its field px), in cell order.
    pub fn blocks(&self) -> impl Iterator<Item = (Position, Block)> + '_ {
        self.cells.iter().flat_map(|(&cell, c)| {
            c.blocks.iter().enumerate().filter(|(_, b)| !b.is_empty()).map(move |(i, b)| {
                let (bx, by) = block_of(cell, i);
                (block_centre(bx, by), *b)
            })
        })
    }

    /// Blocks burnt in.
    pub fn charred(&self) -> usize {
        self.blocks().filter(|(_, b)| b.kind() == Kind::Char).count()
    }

    /// The cell `cell`, made if it is not worn yet - evicting the cell
    /// pressed longest ago past the cap (the first of them in cell order
    /// on a tie), so a seeded replay evicts the same ones.
    fn cell_mut(&mut self, cell: (i32, i32), now: f32) -> &mut Cell {
        if !self.cells.contains_key(&cell) && self.cells.len() >= self.max_cells {
            let oldest = self.cells.iter().fold(None, |best: Option<((i32, i32), f32)>, (&k, c)| match best {
                Some((_, at)) if at <= c.pressed => best,
                _ => Some((k, c.pressed)),
            });
            if let Some((k, _)) = oldest {
                self.cells.remove(&k);
            }
        }
        self.cells.entry(cell).or_insert_with(|| Cell::new(now))
    }

    /// Press `s`'s runs into the ground at round time `now`; `surface`
    /// says what lies where. Returns the blocks a new pass pressed.
    pub fn press(&mut self, s: &Stamp, look: Look, surface: &dyn Fn(Position) -> Surface, now: f32, t: &Tuning) -> u32 {
        let k = (s.scale * 0.5).max(0.01);
        let (sin, cos) = crate::math::sin_cos(s.heading.to_radians());
        let snap = |v: f32| if v.abs() < 1e-4 { 0.0 } else if (v.abs() - 1.0).abs() < 1e-4 { v.signum() } else { v };
        let (rx, ry) = (snap(cos), snap(sin));
        let (fx, fy) = (ry, -rx);
        let outer = s.profile.runs.first().map_or(0.0, |r| r.0 as f32);
        let (front, rear) = (s.profile.patch.0 as f32, s.profile.patch.1 as f32);
        let reach = crate::math::hypot(-outer * k + 2.0, front.abs().max(rear.abs()) * k + 1.0).ceil() as i32 + 1;
        let (cbx, cby) = (s.at.x / BLOCK_PX, s.at.y / BLOCK_PX);
        let (bx0, by0) = (cbx.floor() as i32 - reach, cby.floor() as i32 - reach);
        let (bx1, by1) = (cbx.floor() as i32 + reach, cby.floor() as i32 + reach);
        let facing = ((s.heading / 90.0).round() as i32).rem_euclid(4) as u8;
        let period = s.profile.period.max(1) as i32;
        let gap = t.wear_pass_gap_seconds;
        let mut pressed = 0;
        let (c0, _) = locate(bx0, by0);
        let (c1, _) = locate(bx1, by1);
        for cy in c0.1..=c1.1 {
            for cx in c0.0..=c1.0 {
                let cell = (cx, cy);
                let mut quads: Option<[Surface; 4]> = None;
                for i in 0..BLOCKS {
                    let (bx, by) = block_of(cell, i);
                    if bx < bx0 || bx > bx1 || by < by0 || by > by1 {
                        continue;
                    }
                    let (dx, dy) = (bx as f32 + 0.5 - cbx, by as f32 + 0.5 - cby);
                    let ty = -(dx * fx + dy * fy) / k;
                    if ty < front || ty >= rear {
                        continue;
                    }
                    let tx = (dx * rx + dy * ry) / k;
                    let in_run = s.profile.runs.iter().any(|&(x0, x1)| (tx >= x0 as f32 && tx < x1 as f32) || (tx >= -(x1 as f32) && tx < -(x0 as f32)));
                    let berm_side = (tx >= outer - 1.0 / k && tx < outer) || (tx >= -outer && tx < -outer + 1.0 / k);
                    if !in_run && !berm_side {
                        continue;
                    }
                    let surf = quads.get_or_insert_with(|| quarters(cell, surface))[quarter_of(i)];
                    if surf.is_water() || (!in_run && (s.roll == Roll::Sliding || !is_soft(look, surf))) {
                        continue;
                    }
                    let along = ((bx as f32 + 0.5) * fx + (by as f32 + 0.5) * fy) / k + s.phase;
                    let grouser = (along.floor() as i32).rem_euclid(period) < period / 2;
                    if !in_run && self.cells.get(&cell).is_some_and(|c| c.blocks[i].press > 0) {
                        continue;
                    }
                    let c = self.cell_mut(cell, now);
                    let old = c.blocks[i];
                    let mut b = old;
                    if !in_run {
                        b.set_berm(true);
                        b.set_laid(now);
                    } else if surf == Surface::Ice {
                        b.press = b.press.max(PRESS_ONE as u8);
                        b.set_kind(match s.roll {
                            Roll::Sliding => Kind::Smear,
                            _ if grouser => Kind::Grouser,
                            _ => Kind::Pad,
                        });
                        b.set_laid(now);
                    } else {
                        let new_pass = b.press == 0 || now - b.laid() > gap;
                        if new_pass {
                            let pivot = if s.roll == Roll::Pivoting { t.wear_press_pivot } else { 1.0 };
                            b.add_passes(s.press * press_factor(t, look, surf) * pivot);
                            pressed += 1;
                        }
                        let kind = match (b.kind(), s.roll) {
                            (Kind::Char, _) => Kind::Char,
                            (Kind::Churn, Roll::Rolling) if now - b.laid() < 1.5 => Kind::Churn,
                            (_, Roll::Sliding) => Kind::Smear,
                            (_, Roll::Pivoting) => Kind::Churn,
                            _ if grouser => Kind::Grouser,
                            _ => Kind::Pad,
                        };
                        b.set_kind(kind);
                        b.set_facing(facing);
                        b.set_wet(s.wet);
                        b.set_berm(false);
                        b.set_laid(now);
                    }
                    if b.press != old.press || b.bits != old.bits || now - old.laid() > 1.0 {
                        c.version = c.version.wrapping_add(1);
                    }
                    c.blocks[i] = b;
                    c.pressed = now;
                }
            }
        }
        self.presses += pressed as u64;
        pressed
    }

    /// Change the block at `at` with `f` if it lies on ground that takes
    /// a mark, making its cell if need be.
    fn touch(&mut self, at: Position, now: f32, surface: &dyn Fn(Position) -> Surface, f: impl FnOnce(&mut Block, Surface)) {
        let surf = surface(at);
        if surf.is_water() || surf == Surface::Ice {
            return;
        }
        let (bx, by) = block_at(at);
        let (cell, i) = locate(bx, by);
        let c = self.cell_mut(cell, now);
        let old = c.blocks[i];
        f(&mut c.blocks[i], surf);
        if c.blocks[i] != old {
            c.version = c.version.wrapping_add(1);
            c.pressed = now;
        }
    }

    /// A drop of ford water off a wet hull, landing at `at`.
    pub fn drip(&mut self, at: Position, now: f32, surface: &dyn Fn(Position) -> Surface) {
        self.touch(at, now, surface, |b, _| {
            if b.passes() < 0.5 {
                b.set_kind(Kind::Drip);
                b.set_wet(1.0);
                b.press = b.press.max(4);
                b.set_laid(now);
            }
        });
    }

    /// A mud clod landing at `at`: a splat two blocks across, over
    /// anything shallower than a rut.
    pub fn splat(&mut self, at: Position, now: f32, surface: &dyn Fn(Position) -> Surface, t: &Tuning) {
        for (dx, dy) in [(0.0, 0.0), (BLOCK_PX, 0.0), (0.0, BLOCK_PX), (BLOCK_PX, BLOCK_PX)] {
            self.touch(Position::new(at.x + dx, at.y + dy), now, surface, |b, _| {
                if b.passes() < t.wear_rut_passes && b.kind() != Kind::Char {
                    b.set_kind(Kind::Splat);
                    b.set_wet(1.0);
                    b.press = b.press.max(6);
                    b.set_laid(now);
                }
            });
        }
    }

    /// Visit every worn block within `radius` of `center` with `f`, which
    /// answers whether it changed.
    fn around(&mut self, center: Position, radius: f32, mut f: impl FnMut(&mut Block, Position) -> bool) {
        if radius <= 0.0 {
            return;
        }
        let (lo, _) = locate(((center.x - radius) / BLOCK_PX).floor() as i32, ((center.y - radius) / BLOCK_PX).floor() as i32);
        let (hi, _) = locate(((center.x + radius) / BLOCK_PX).floor() as i32, ((center.y + radius) / BLOCK_PX).floor() as i32);
        for (&cell, c) in self.cells.range_mut(lo..=(hi.0, hi.1)) {
            if cell.1 < lo.1 || cell.1 > hi.1 {
                continue;
            }
            let mut changed = false;
            for (i, b) in c.blocks.iter_mut().enumerate() {
                if b.is_empty() {
                    continue;
                }
                let (bx, by) = block_of(cell, i);
                let at = block_centre(bx, by);
                if at.distance_to(center) <= radius {
                    changed |= f(b, at);
                }
            }
            if changed {
                c.version = c.version.wrapping_add(1);
            }
        }
    }

    /// Burn the marks within `radius` of `center` into the ground: a
    /// wreck's last tracks, the ground under a drum's blast. Returns the
    /// blocks it burnt.
    pub fn char_around(&mut self, center: Position, radius: f32) -> usize {
        let mut burnt = 0;
        self.around(center, radius, |b, _| {
            if b.press == 0 || b.kind() == Kind::Char {
                return false;
            }
            b.set_kind(Kind::Char);
            b.set_wet(0.0);
            burnt += 1;
            true
        });
        burnt
    }

    /// A gravity well's pull scrubbing the marks within `radius` of
    /// `center`: they lose their pattern to a swirled smear and fade, the
    /// faster the stronger `pull` (0 to 1, by distance from the centre).
    pub fn scrub(&mut self, center: Position, radius: f32, pull: impl Fn(f32) -> f32, dt: f32, t: &Tuning) {
        self.around(center, radius, |b, at| {
            if b.press == 0 || b.kind() == Kind::Char {
                return false;
            }
            let keep = crate::math::exp(-t.wear_well_scrub * pull(at.distance_to(center)) * dt);
            b.press = (b.press as f32 * keep).floor() as u8;
            if b.kind() != Kind::Smear {
                b.set_kind(Kind::Smear);
            }
            true
        });
        self.drop_empty();
    }

    /// A sandstorm's gusts scouring the marks off what `erodes` says
    /// gives: `strength` is the gust at a place, 0 to 1. The grousers go
    /// first and the press after.
    pub fn scour(&mut self, strength: impl Fn(Position) -> f32, erodes: impl Fn(Position) -> bool, dt: f32, t: &Tuning) {
        for (&cell, c) in self.cells.iter_mut() {
            let centre = crate::map::cell_to_world(cell.0, cell.1);
            let s = strength(centre);
            if s <= 0.01 {
                continue;
            }
            let keep = crate::math::exp(-t.wear_gust_scour * s * dt);
            let mut changed = false;
            for (i, b) in c.blocks.iter_mut().enumerate() {
                if b.is_empty() || b.kind() == Kind::Char {
                    continue;
                }
                let (bx, by) = block_of(cell, i);
                if !erodes(block_centre(bx, by)) {
                    continue;
                }
                let press = (b.press as f32 * keep).floor() as u8;
                if press != b.press || b.berm() || matches!(b.kind(), Kind::Grouser | Kind::Churn) {
                    b.press = press;
                    b.set_berm(false);
                    if press == 0 {
                        *b = Block::default();
                    } else if matches!(b.kind(), Kind::Grouser | Kind::Churn) {
                        b.set_kind(Kind::Pad);
                    }
                    changed = true;
                }
            }
            if changed {
                c.version = c.version.wrapping_add(1);
            }
        }
        self.drop_empty();
    }

    /// Silt a wading hull stirs off the bed of the ford under it: a disc of
    /// radius `half` (its hull's half-width), each block gaining up to
    /// `amount` (0 to 1), thinning to nothing at the rim.
    pub fn stir(&mut self, at: Position, half: f32, amount: f32, now: f32, surface: &dyn Fn(Position) -> Surface) {
        let half = half.max(BLOCK_PX);
        let (b0, b1) = (block_at(Position::new(at.x - half, at.y - half)), block_at(Position::new(at.x + half, at.y + half)));
        for by in b0.1..=b1.1 {
            for bx in b0.0..=b1.0 {
                let p = block_centre(bx, by);
                let near = 1.0 - crate::math::hypot(p.x - at.x, p.y - at.y) / half;
                if near <= 0.0 || surface(p) != Surface::Ford {
                    continue;
                }
                let add = (amount * near * 255.0).round().max(1.0);
                let (cell, i) = locate(bx, by);
                let c = self.cell_mut(cell, now);
                let b = &mut c.blocks[i];
                let silt = (b.press as f32 + add).min(255.0) as u8;
                if silt != b.press {
                    b.press = silt;
                    c.version = c.version.wrapping_add(1);
                }
                c.silty = true;
                c.pressed = now;
            }
        }
    }

    /// Let the silt drift down every stream a block at a time and settle,
    /// at 20 steps a second of round time: `flows` says where a current
    /// runs south, `water` where the bed is open water.
    pub fn drift_silt(&mut self, dt: f32, flows: impl Fn(Position) -> bool, water: impl Fn(Position) -> bool, t: &Tuning) {
        const STEP: f32 = 0.05;
        if !self.cells.values().any(|c| c.silty) {
            self.silt_due = 0.0;
            return;
        }
        self.silt_due += dt;
        while self.silt_due >= STEP {
            self.silt_due -= STEP;
            let share = (t.wear_silt_drift * STEP).clamp(0.0, 1.0);
            let keep = crate::math::exp(-STEP / t.wear_silt_seconds.max(0.05));
            // Southmost first, so silt moves one block a step.
            let keys: Vec<(i32, i32)> = self.cells.iter().filter(|(_, c)| c.silty).map(|(k, _)| *k).rev().collect();
            let mut spill: Vec<((i32, i32), usize, f32)> = Vec::new();
            for key in keys {
                let flowing = flows(crate::map::cell_to_world(key.0, key.1));
                let Some(c) = self.cells.get_mut(&key) else { continue };
                let mut any = false;
                for v in (0..TILE_BLOCKS).rev() {
                    for u in 0..TILE_BLOCKS {
                        let i = v * TILE_BLOCKS + u;
                        let silt = c.blocks[i].press as f32;
                        if silt <= 0.0 {
                            continue;
                        }
                        let mut stays = silt;
                        if flowing {
                            let moving = silt * share;
                            stays -= moving;
                            if v + 1 < TILE_BLOCKS {
                                let j = i + TILE_BLOCKS;
                                c.blocks[j].press = (c.blocks[j].press as f32 + moving).min(255.0) as u8;
                            } else {
                                spill.push(((key.0, key.1 + 1), u, moving));
                            }
                        }
                        c.blocks[i].press = (stays * keep).floor() as u8;
                        any |= c.blocks[i].press > 0;
                    }
                }
                c.silty = any;
                c.version = c.version.wrapping_add(1);
            }
            for (cell, u, moving) in spill {
                let (bx, by) = block_of(cell, u);
                if !water(block_centre(bx, by)) {
                    continue;
                }
                let pressed = self.cells.get(&(cell.0, cell.1 - 1)).map_or(0.0, |c| c.pressed);
                let c = self.cell_mut(cell, pressed);
                c.blocks[u].press = (c.blocks[u].press as f32 + moving).min(255.0) as u8;
                c.silty = true;
                c.version = c.version.wrapping_add(1);
            }
        }
        self.drop_empty();
    }

    /// How flat tall grass at `at` stays on a beaten lane, 0 to
    /// `wear_grass_crush_floor`.
    pub fn crush_floor(&self, at: Position, t: &Tuning) -> f32 {
        let passes = self.passes_at(at);
        if passes < t.wear_deep_passes {
            return 0.0;
        }
        let span = (t.wear_path_passes - t.wear_deep_passes).max(0.01);
        ((passes - t.wear_deep_passes) / span).clamp(0.0, 1.0) * t.wear_grass_crush_floor
    }

    fn drop_empty(&mut self) {
        self.cells.retain(|_, c| !c.is_empty());
    }

    /// The pictures as last brought up (`refresh_pictures`).
    pub fn pictures(&self) -> Ref<'_, Pictures> {
        self.pictures.borrow()
    }

    /// Bring the pictures up to `frame`: drop the tiles of cells no longer
    /// worn or out of view, then bake the cells in view that have none,
    /// that changed, or whose age step may have come due - at most
    /// `wear_bakes_per_frame` of them, every one after a jump in the clock
    /// (a new round, a dev server's step). Call before the frame's
    /// textures are uploaded, so nothing changes while it is drawn.
    pub fn refresh_pictures(&self, frame: &PictureFrame, t: &Tuning) {
        let mut guard = self.pictures.borrow_mut();
        let p = &mut *guard;
        if p.look != Some(frame.look) {
            for key in std::mem::take(&mut p.baked).into_keys() {
                p.atlas.remove(key);
            }
            p.look = Some(frame.look);
        }
        let jump = p.at.is_none_or(|at| frame.time < at || frame.time - at > 1.0);
        let shown = |cell: (i32, i32)| {
            frame.views.is_empty() || {
                let c = crate::map::cell_to_world(cell.0, cell.1);
                let s = OBSTACLE_GRID_SIZE;
                frame.views.iter().any(|v| c.x + s >= v.x && c.x - s <= v.x + v.width && c.y + s >= v.y && c.y - s <= v.y + v.height)
            }
        };
        let stale: Vec<(i32, i32)> = p.baked.keys().filter(|k| !self.cells.contains_key(k) || !shown(**k)).copied().collect();
        for key in stale {
            p.atlas.remove(key);
            p.baked.remove(&key);
        }
        let mut due: Vec<(u8, f32, (i32, i32))> = self
            .cells
            .iter()
            .filter(|(k, _)| shown(**k))
            .filter_map(|(&k, c)| match p.baked.get(&k) {
                None => Some((0, 0.0, k)),
                Some(&(version, _)) if version != c.version => Some((1, 0.0, k)),
                Some(&(_, at)) if frame.time - at >= t.wear_rebake_seconds => Some((2, at, k)),
                _ => None,
            })
            .collect();
        due.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
        let budget = if jump { usize::MAX } else { t.wear_bakes_per_frame.max(1) as usize };
        let room = MAX_PICTURE_CELLS.saturating_sub(p.atlas.len());
        let fresh = due.iter().filter(|d| d.0 == 0).count().min(room);
        p.atlas.reserve(p.atlas.len() + fresh);
        let mut added = 0;
        for (kind, _, key) in due.into_iter().take(budget) {
            if kind == 0 {
                if added >= room {
                    continue;
                }
                added += 1;
            }
            let Some(cell) = self.cells.get(&key) else { continue };
            let colours = self.bake(key, cell, frame, t);
            p.atlas.put(key, |u, v| colours[v * TILE_BLOCKS + u]);
            p.atlas.commit();
            p.baked.insert(key, (cell.version, frame.time));
        }
        p.atlas.commit();
        p.at = Some(frame.time);
    }

    /// The colours of cell `key`'s blocks at `frame`, `None` where the
    /// ground shows through.
    fn bake(&self, key: (i32, i32), cell: &Cell, frame: &PictureFrame, t: &Tuning) -> [Option<Color>; BLOCKS] {
        let quads = quarters(key, frame.surface);
        let ramps = quads.map(|s| Ramp::of(frame.look, s));
        let up = self.cells.get(&(key.0, key.1 - 1));
        let left = self.cells.get(&(key.0 - 1, key.1));
        let pressed = |i: usize, du: i32, dv: i32| -> bool {
            let (u, v) = ((i % TILE_BLOCKS) as i32 + du, (i / TILE_BLOCKS) as i32 + dv);
            let n = TILE_BLOCKS as i32;
            let b = if u < 0 {
                left.map(|c| c.blocks[(v * n + n - 1) as usize])
            } else if v < 0 {
                up.map(|c| c.blocks[((n - 1) * n + u) as usize])
            } else {
                Some(cell.blocks[(v * n + u) as usize])
            };
            b.is_some_and(|b| b.passes() >= 0.3)
        };
        let mut out = [None; BLOCKS];
        for (i, b) in cell.blocks.iter().enumerate() {
            if b.is_empty() {
                continue;
            }
            let (bx, by) = block_of(key, i);
            let q = quarter_of(i);
            out[i] = match quads[q] {
                Surface::Ford | Surface::Deep => silt_colour(*b, bx, by),
                Surface::Ice => ice_colour(*b, bx, by, frame.time, t),
                surf => {
                    let rim = !pressed(i, 0, -1) || !pressed(i, -1, 0);
                    ground_colour(*b, bx, by, surf, frame.look, &ramps[q], rim, frame.time, t)
                }
            };
        }
        out
    }
}

/// What a frame brings the pictures up to (`WearGrid::refresh_pictures`).
pub struct PictureFrame<'a> {
    pub time: f32,
    pub look: Look,
    /// The world rectangles the window shows; empty: the whole field.
    pub views: &'a [Rectangle],
    /// What lies where (`Game::surface_at`).
    pub surface: &'a dyn Fn(Position) -> Surface,
}

/// The worn cells baked as tiles, for the painter.
#[derive(Default)]
pub struct Pictures {
    atlas: TileAtlas,
    /// Each baked cell's version and the round time it was baked at.
    baked: BTreeMap<(i32, i32), (u32, f32)>,
    look: Option<Look>,
    at: Option<f32>,
}

impl Pictures {
    pub fn atlas(&self) -> &TileAtlas {
        &self.atlas
    }
}

/// How deep a mark is: a single pass, a rut, a deep rut, a beaten path.
fn level(passes: f32, t: &Tuning) -> usize {
    if passes < t.wear_rut_passes {
        0
    } else if passes < t.wear_deep_passes {
        1
    } else if passes < t.wear_path_passes {
        2
    } else {
        3
    }
}

/// How much of a single pass is still there at `age`: whole until it
/// settles, thinning to half by `wear_fade_seconds`, half after that.
fn presence(age: f32, t: &Tuning) -> f32 {
    let (s, e) = (t.wear_settle_seconds, t.wear_fade_seconds.max(t.wear_settle_seconds + 0.01));
    if age <= s {
        1.0
    } else {
        (1.0 - (age - s) / (e - s)).max(0.5)
    }
}

/// How long a ground holds a grouser's bar, as a multiple of the knobs:
/// sand slumps at once, a dirt road holds.
fn detail_scale(look: Look, surface: Surface) -> f32 {
    if look.sky == Sky::Snow {
        return 1.2;
    }
    match surface {
        Surface::Sand => 0.25,
        Surface::Road => 1.5,
        _ if look.theme == Theme::Desert => 0.7,
        _ if look.sky == Sky::Rain => 0.8,
        _ => 1.0,
    }
}

const fn hex(v: u32) -> Color {
    Color::new((v >> 16) as u8, (v >> 8) as u8, v as u8, 255)
}

/// One ground's mark colours, steps from the Puny palette and the retinted
/// tileset's own pixels (docs/ground-memory.md "Colours"), never blends.
#[derive(Clone, Copy, Debug)]
struct Ramp {
    /// A pad's colour by depth: a single pass, a rut, a deep rut, a path.
    pad: [Color; 4],
    /// A single pass driven down or right, where the ground lies over the
    /// other way (grass's sheen); `None` where it does not.
    pad_back: Option<Color>,
    grouser: [Color; 4],
    grouser_back: Option<Color>,
    /// A rut's top-left inner wall, out of the light.
    rim: [Color; 4],
    berm: Color,
    /// A wet mark: a pad, then a grouser or anything deeper.
    wet: [Color; 2],
    smear: Color,
    churn: [Color; 4],
}

impl Ramp {
    fn of(look: Look, surface: Surface) -> Ramp {
        let base = match (look.theme, surface) {
            (Theme::Desert, Surface::Road) => DESERT_ROAD,
            (Theme::Desert, Surface::Sand) => DESERT_SAND,
            (Theme::Desert, _) => DESERT_DUST,
            (_, Surface::Road) => ROAD,
            (_, Surface::Sand) => SAND,
            _ => GRASS,
        };
        if look.sky != Sky::Snow {
            return base;
        }
        let under = base.wet[1];
        Ramp {
            pad: [hex(0xc6d0dc), hex(0xaebbca), hex(0x93a1b3), under],
            pad_back: None,
            grouser: [hex(0xaebbca), hex(0x93a1b3), under, under],
            grouser_back: None,
            rim: [hex(0xaebbca), hex(0x93a1b3), hex(0x808ea1), hex(0x808ea1)],
            berm: hex(0xfbfdff),
            wet: [hex(0x93a1b3), under],
            smear: hex(0xd3dde8),
            churn: [hex(0xaebbca), hex(0xfbfdff), hex(0xc6d0dc), under],
        }
    }
}

const GRASS: Ramp = Ramp {
    pad: [hex(0x7ea64a), hex(0x84994c), hex(0xa7a559), hex(0xb1a567)],
    pad_back: Some(hex(0x5c8e3d)),
    grouser: [hex(0x5c8e3d), hex(0x73624d), hex(0x84994c), hex(0xa7a559)],
    grouser_back: Some(hex(0x4f7f37)),
    rim: [hex(0x4f7f37), hex(0x5c8e3d), hex(0x73624d), hex(0x84994c)],
    berm: hex(0x73624d),
    wet: [hex(0x4f7f37), hex(0x3d6a33)],
    smear: hex(0x7ea64a),
    churn: [hex(0x84994c), hex(0x73624d), hex(0x5c8e3d), hex(0xa7a559)],
};

const ROAD: Ramp = Ramp {
    pad: [hex(0xa7a559), hex(0x8f8a4c), hex(0x84994c), hex(0x73624d)],
    pad_back: None,
    grouser: [hex(0x84994c), hex(0x73624d), hex(0x73624d), hex(0x67512a)],
    grouser_back: None,
    rim: [hex(0x84994c), hex(0x73624d), hex(0x67512a), hex(0x59341f)],
    berm: hex(0xc9b266),
    wet: [hex(0x73624d), hex(0x67512a)],
    smear: hex(0xa7a559),
    churn: [hex(0x73624d), hex(0xa7a559), hex(0xc9b266), hex(0x84994c)],
};

const SAND: Ramp = Ramp {
    pad: [hex(0xb7a248), hex(0xa99653), hex(0xa08058), hex(0xa08058)],
    pad_back: None,
    grouser: [hex(0xa08058), hex(0x8a6d47), hex(0x8a6d47), hex(0x73624d)],
    grouser_back: None,
    rim: [hex(0xa08058), hex(0x8a6d47), hex(0x73624d), hex(0x67512a)],
    berm: hex(0xd2ba6b),
    wet: [hex(0x8a6d47), hex(0x73624d)],
    smear: hex(0xb7a248),
    churn: [hex(0xa08058), hex(0xd2ba6b), hex(0xb7a248), hex(0x8a6d47)],
};

const DESERT_DUST: Ramp = Ramp {
    pad: [hex(0xbaa277), hex(0xa78761), hex(0xa08058), hex(0xa08058)],
    pad_back: None,
    grouser: [hex(0xa78761), hex(0xa08058), hex(0x8a6d47), hex(0x73624d)],
    grouser_back: None,
    rim: [hex(0xa08058), hex(0x8a6d47), hex(0x73624d), hex(0x67512a)],
    berm: hex(0xdac59b),
    wet: [hex(0x8a6d47), hex(0x73624d)],
    smear: hex(0xbaa277),
    churn: [hex(0xa08058), hex(0xdac59b), hex(0xbaa277), hex(0xa78761)],
};

const DESERT_ROAD: Ramp = Ramp {
    pad: [hex(0x8f7150), hex(0x8a6d47), hex(0x73624d), hex(0x73624d)],
    pad_back: None,
    grouser: [hex(0x73624d), hex(0x73624d), hex(0x67512a), hex(0x59341f)],
    grouser_back: None,
    rim: [hex(0x73624d), hex(0x67512a), hex(0x59341f), hex(0x59341f)],
    berm: hex(0xbfa57a),
    wet: [hex(0x67512a), hex(0x59341f)],
    smear: hex(0x8f7150),
    churn: [hex(0x73624d), hex(0xbfa57a), hex(0xa08058), hex(0x8a6d47)],
};

const DESERT_SAND: Ramp = Ramp {
    pad: [hex(0xb69c72), hex(0xa99068), hex(0xa08058), hex(0xa08058)],
    pad_back: None,
    grouser: [hex(0xa08058), hex(0x8a6d47), hex(0x8a6d47), hex(0x73624d)],
    grouser_back: None,
    rim: [hex(0xa08058), hex(0x8a6d47), hex(0x73624d), hex(0x67512a)],
    berm: hex(0xd3bc90),
    wet: [hex(0x8a6d47), hex(0x73624d)],
    smear: hex(0xb69c72),
    churn: [hex(0xa08058), hex(0xd3bc90), hex(0xb69c72), hex(0x8a6d47)],
};

/// Water standing in a rut, and the odd block of sky caught in it.
const PUDDLE: Color = hex(0x4d5e80);
const PUDDLE_SHEEN: Color = hex(0x7a90b4);

/// Silt in a ford: a muddy teal, then brown where it is thick.
const SILT_LIGHT: Color = hex(0x3f8f86);
const SILT_DARK: Color = hex(0x6b7650);

/// Scratches on ice: a grouser's line, a pad's, a slide's.
const ICE_SCRATCH: Color = hex(0xf6faff);
const ICE_PAD: Color = hex(0xd3e9f7);
const ICE_SMEAR: Color = hex(0xe2f0fa);

/// The weather shader's wet ground, nearly: marks lie over its ground pass,
/// so they take the rain's darkening themselves.
fn rain_tint(c: Color) -> Color {
    Color::new((c.r as f32 * 0.8) as u8, (c.g as f32 * 0.85) as u8, (c.b as f32 * 0.92) as u8, c.a)
}

/// `c` at `eighths` of its opacity.
fn eighths(c: Color, eighths: u32) -> Color {
    Color::new(c.r, c.g, c.b, ((c.a as u32 * eighths.min(8)) / 8) as u8)
}

#[allow(clippy::too_many_arguments)]
fn ground_colour(b: Block, bx: i32, by: i32, surface: Surface, look: Look, ramp: &Ramp, rim: bool, now: f32, t: &Tuning) -> Option<Color> {
    let bay = crate::pyro::bayer;
    let age = (now - b.laid()).max(0.0);
    let kind = b.kind();
    if b.press == 0 {
        let life = t.wear_berm_seconds.max(0.01);
        let c = if look.sky == Sky::Rain { rain_tint(ramp.berm) } else { ramp.berm };
        return (b.berm() && clump(bx + 3, by + 7) < 0.7 * (1.0 - age / life)).then_some(c);
    }
    let mut passes = b.passes();
    if look.sky == Sky::Snow && kind != Kind::Char {
        passes -= t.wear_snow_refill_per_second * age;
        if passes <= 1.0 / 32.0 {
            return None;
        }
    }
    let lvl = level(passes, t);
    let wet = b.wet() * (1.0 - age / t.wear_wet_dry_seconds.max(0.01));
    if kind != Kind::Char && wet > 0.0 && bay(bx + 2, by + 1) < wet {
        let deep = matches!(kind, Kind::Grouser | Kind::Drip | Kind::Splat) || lvl > 0;
        let c = ramp.wet[deep as usize];
        return Some(if look.sky == Sky::Rain { rain_tint(c) } else { c });
    }
    if kind == Kind::Drip {
        return None;
    }
    let mut opacity = 8;
    if lvl == 0 && kind != Kind::Char {
        let there = if kind == Kind::Splat { 1.0 - age / t.wear_splat_seconds.max(0.01) } else { presence(age, t) };
        if there <= 0.06 {
            return None;
        }
        if there < 0.5 {
            if bay(bx, by) >= 0.5 {
                return None;
            }
            opacity = ((there * 16.0).round() as u32).max(1);
        } else if bay(bx, by) >= there {
            return None;
        }
    }
    let ds = detail_scale(look, surface);
    let (fresh, settle) = (t.wear_fresh_seconds * ds, (t.wear_settle_seconds - t.wear_fresh_seconds).max(0.01) * ds);
    let detail = (1.0 - (age - fresh) / settle).clamp(0.0, 1.0);
    let kind = if matches!(kind, Kind::Grouser | Kind::Churn) && bay(bx + 1, by + 3) >= detail { Kind::Pad } else { kind };
    if look.sky == Sky::Rain && lvl >= 1 && surface != Surface::Sand && !matches!(kind, Kind::Smear | Kind::Char) {
        let fill = (age / t.wear_puddle_fill_seconds.max(0.01)).min(1.0) * if lvl >= 2 { 0.62 } else { 0.42 };
        if clump(bx, by) < fill {
            return Some(if unit(bx, by, (now * 1.5) as u32) < 0.05 { PUDDLE_SHEEN } else { PUDDLE });
        }
    }
    let back = matches!(b.facing(), 1 | 2);
    let c = match kind {
        Kind::Char => crate::pyro::CHAR[3 - lvl],
        _ if rim && lvl >= 1 => ramp.rim[lvl],
        Kind::Grouser => match ramp.grouser_back {
            Some(c) if lvl == 0 && back => c,
            _ => ramp.grouser[lvl],
        },
        Kind::Smear if lvl < 2 => ramp.smear,
        Kind::Churn => ramp.churn[(unit(bx, by, 7) * 4.0) as usize % 4],
        Kind::Splat => ramp.grouser[1],
        _ => match ramp.pad_back {
            Some(c) if lvl == 0 && back => c,
            _ => ramp.pad[lvl],
        },
    };
    let c = if look.sky == Sky::Rain { rain_tint(c) } else { c };
    Some(eighths(c, opacity))
}

fn ice_colour(b: Block, bx: i32, by: i32, now: f32, t: &Tuning) -> Option<Color> {
    let age = (now - b.laid()).max(0.0);
    let life = t.wear_ice_scratch_seconds.max(0.01);
    if b.press == 0 || crate::pyro::bayer(bx, by) >= 1.0 - age / life {
        return None;
    }
    Some(match b.kind() {
        Kind::Grouser => ICE_SCRATCH,
        Kind::Smear => ICE_SMEAR,
        _ => ICE_PAD,
    })
}

fn silt_colour(b: Block, bx: i32, by: i32) -> Option<Color> {
    let silt = b.silt();
    (silt > 0.06 && clump(bx + 5, by + 3) < silt).then_some(if silt > 0.7 { SILT_DARK } else { SILT_LIGHT })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuning::Tuning;
    use crate::TREAD_BY_ROW;

    const ASSAULT: usize = 1;
    const TITAN: usize = 10;

    fn look() -> Look {
        Look::default()
    }

    fn grass(_: Position) -> Surface {
        Surface::Grass
    }

    fn stamp(at: Position, heading: f32, row: usize, roll: Roll) -> Stamp<'static> {
        Stamp { at, heading, profile: &TREAD_BY_ROW[row], scale: 2.0, press: 1.0, roll, wet: 0.0, phase: 0.0 }
    }

    /// Drive a hull of chassis `row` straight up from `from` for `px`,
    /// a px a step, starting at round time `t0`.
    fn drive_up(g: &mut WearGrid, row: usize, from: Position, px: i32, t0: f32, t: &Tuning) -> f32 {
        let mut now = t0;
        for k in 0..=px {
            now = t0 + k as f32 / 140.0;
            g.press(&stamp(Position::new(from.x, from.y - k as f32), 0.0, row, Roll::Rolling), look(), &grass, now, t);
        }
        now
    }

    /// The x of every pressed block in the row of blocks through `y`.
    fn pressed_columns(g: &WearGrid, y: f32) -> Vec<i32> {
        let mut xs: Vec<i32> = g.blocks().filter(|(p, b)| (p.y - y).abs() < 1.0 && b.passes() > 0.0).map(|(p, _)| (p.x / BLOCK_PX).floor() as i32).collect();
        xs.sort();
        xs
    }

    #[test]
    fn a_pass_presses_the_two_runs_under_the_hull_and_nothing_between() {
        let t = Tuning::default();
        let mut g = WearGrid::new(64);
        drive_up(&mut g, ASSAULT, Position::new(200.0, 300.0), 120, 0.0, &t);
        // The assault's runs are [-7, -4) and [4, 7) tile px: three blocks
        // either side of the centre column.
        assert_eq!(pressed_columns(&g, 251.0), vec![93, 94, 95, 104, 105, 106]);
    }

    #[test]
    fn a_titan_presses_four_runs() {
        let t = Tuning::default();
        let mut g = WearGrid::new(64);
        drive_up(&mut g, TITAN, Position::new(200.0, 300.0), 120, 0.0, &t);
        let xs = pressed_columns(&g, 251.0);
        let runs = xs.windows(2).filter(|w| w[1] != w[0] + 1).count() + 1;
        assert_eq!(runs, 4, "four runs: {xs:?}");
    }

    #[test]
    fn grousers_are_bars_fixed_to_the_ground() {
        let t = Tuning::default();
        let mut g = WearGrid::new(64);
        drive_up(&mut g, ASSAULT, Position::new(200.0, 300.0), 120, 0.0, &t);
        let column: Vec<Kind> = (0..16).map(|k| g.block(Position::new(187.0, 241.0 + 2.0 * k as f32)).unwrap().kind()).collect();
        // Two blocks of bar, two of pad, down the run.
        for (k, kind) in column.iter().enumerate() {
            assert!(matches!(kind, Kind::Grouser | Kind::Pad));
            assert_eq!(*kind == Kind::Grouser, column[(k + 4) % 16] == Kind::Grouser, "period four: {column:?}");
        }
        assert!(column.contains(&Kind::Grouser) && column.contains(&Kind::Pad));
    }

    #[test]
    fn a_second_pass_deepens_and_a_hull_resting_on_it_does_not() {
        let t = Tuning::default();
        let mut g = WearGrid::new(64);
        let at = Position::new(187.0, 251.0);
        let end = drive_up(&mut g, ASSAULT, Position::new(200.0, 300.0), 120, 0.0, &t);
        let one = g.passes_at(at);
        assert!((one - 1.0).abs() < 0.07, "one pass: {one}");
        // A hull standing where it stopped, pressed again and again, is
        // the same pass.
        let under = Position::new(187.0, 181.0);
        let resting = g.passes_at(under);
        for k in 0..60 {
            g.press(&stamp(Position::new(200.0, 180.0), 0.0, ASSAULT, Roll::Rolling), look(), &grass, end + k as f32 * 0.05, &t);
        }
        assert!((g.passes_at(under) - resting).abs() < 0.01, "resting deepened it");
        drive_up(&mut g, ASSAULT, Position::new(200.0, 300.0), 120, end + 5.0, &t);
        assert!((g.passes_at(at) - 2.0).abs() < 0.13, "two passes: {}", g.passes_at(at));
    }

    #[test]
    fn open_water_takes_no_mark_and_ice_takes_only_a_scratch() {
        let t = Tuning::default();
        for (surface, marked) in [(Surface::Ford, false), (Surface::Deep, false), (Surface::Ice, true)] {
            let mut g = WearGrid::new(64);
            let s = move |_: Position| surface;
            for k in 0..60 {
                g.press(&stamp(Position::new(200.0, 300.0 - k as f32), 0.0, ASSAULT, Roll::Rolling), look(), &s, k as f32 / 60.0, &t);
            }
            assert!(g.presses() == 0, "{surface:?} counts no pass");
            assert_eq!(g.worn_cells() > 0, marked, "{surface:?}");
        }
    }

    #[test]
    fn a_slide_smears_and_a_pivot_churns() {
        let t = Tuning::default();
        let mut g = WearGrid::new(64);
        g.press(&stamp(Position::new(100.0, 100.0), 0.0, ASSAULT, Roll::Sliding), look(), &grass, 0.0, &t);
        assert!(g.blocks().all(|(_, b)| b.kind() == Kind::Smear));
        let mut g = WearGrid::new(64);
        for a in 0..=45 {
            g.press(&stamp(Position::new(100.0, 100.0), a as f32 * 2.0, ASSAULT, Roll::Pivoting), look(), &grass, a as f32 / 100.0, &t);
        }
        assert!(g.blocks().all(|(_, b)| b.kind() == Kind::Churn));
        // A pivot sweeps a ring: nothing under the hull's middle.
        assert!(g.block(Position::new(100.0, 100.0)).is_none());
    }

    #[test]
    fn soft_ground_takes_a_berm_and_firm_ground_none() {
        let t = Tuning::default();
        let sand = |_: Position| Surface::Sand;
        let mut g = WearGrid::new(64);
        g.press(&stamp(Position::new(100.0, 100.0), 0.0, ASSAULT, Roll::Rolling), look(), &sand, 0.0, &t);
        assert!(g.blocks().any(|(_, b)| b.berm()));
        let mut g = WearGrid::new(64);
        g.press(&stamp(Position::new(100.0, 100.0), 0.0, ASSAULT, Roll::Rolling), look(), &grass, 0.0, &t);
        assert!(!g.blocks().any(|(_, b)| b.berm()));
    }

    #[test]
    fn past_the_cap_the_cell_pressed_longest_ago_goes() {
        let mut g = WearGrid::new(4);
        for (k, x) in [40.0, 200.0, 360.0, 520.0, 680.0].iter().enumerate() {
            g.drip(Position::new(*x, 40.0), k as f32, &grass);
        }
        assert_eq!(g.worn_cells(), 4);
        assert!(g.block(Position::new(40.0, 40.0)).is_none(), "the first drip's cell went");
        assert!(g.block(Position::new(680.0, 40.0)).is_some());
    }

    #[test]
    fn a_wreck_burns_in_only_what_is_marked() {
        let t = Tuning::default();
        let mut g = WearGrid::new(64);
        drive_up(&mut g, ASSAULT, Position::new(200.0, 300.0), 120, 0.0, &t);
        let worn = g.blocks().count();
        let burnt = g.char_around(Position::new(200.0, 250.0), 32.0);
        assert!(burnt > 0 && g.charred() == burnt);
        assert_eq!(g.blocks().count(), worn, "no new block is made");
    }

    #[test]
    fn a_gust_scours_sand_and_leaves_the_road() {
        let t = Tuning::default();
        let surface = |p: Position| if p.x < 200.0 { Surface::Sand } else { Surface::Road };
        let mut g = WearGrid::new(64);
        for x in [100.0, 300.0] {
            for k in 0..=80 {
                g.press(&stamp(Position::new(x, 300.0 - k as f32), 0.0, ASSAULT, Roll::Rolling), look(), &surface, k as f32 / 140.0, &t);
            }
        }
        let (sand, road) = (g.passes_at(Position::new(87.0, 251.0)), g.passes_at(Position::new(287.0, 251.0)));
        for _ in 0..120 {
            g.scour(|_| 1.0, |p| erodes(look(), surface(p)), 1.0 / 60.0, &t);
        }
        assert!(g.passes_at(Position::new(87.0, 251.0)) < sand * 0.5, "sand scoured");
        assert_eq!(g.passes_at(Position::new(287.0, 251.0)), road, "the road kept");
    }

    #[test]
    fn silt_drifts_down_a_stream() {
        let t = Tuning::default();
        let ford = |_: Position| Surface::Ford;
        let mut g = WearGrid::new(64);
        g.stir(Position::new(96.0, 96.0), 6.0, 1.0, 0.0, &ford);
        let silt = |p: Position| g.block(p).map_or(0.0, |b| b.silt());
        assert!(silt(Position::new(97.0, 97.0)) > 2.0 * silt(Position::new(101.0, 97.0)), "thinner towards the rim");
        assert_eq!(silt(Position::new(103.0, 103.0)), 0.0, "and none past it");
        let below = Position::new(96.0, 120.0);
        assert!(g.block(below).is_none());
        g.drift_silt(2.0, |_| true, |_| true, &t);
        assert!(g.block(below).is_some_and(|b| b.silt() > 0.0), "silt reached the cell below");
        let mut still = WearGrid::new(64);
        still.stir(Position::new(96.0, 96.0), 6.0, 1.0, 0.0, &ford);
        still.drift_silt(2.0, |_| false, |_| true, &t);
        assert!(still.block(below).is_none(), "a lake keeps its silt where it was stirred");
        still.drift_silt(60.0, |_| false, |_| true, &t);
        assert_eq!(still.worn_cells(), 0, "and it settles");
    }

    /// The colour a fresh assault pass leaves at `at` after `age` seconds
    /// under `look`.
    fn colour_after(g: &WearGrid, at: Position, age: f32, look: Look, t: &Tuning) -> Option<Color> {
        let (bx, by) = block_at(at);
        let (cell, i) = locate(bx, by);
        let c = g.cells.get(&cell).expect("worn");
        let s = move |_: Position| Surface::Grass;
        let frame = PictureFrame { time: c.blocks[i].laid() + age, look, views: &[], surface: &s };
        g.bake(cell, c, &frame, t)[i]
    }

    #[test]
    fn grousers_dissolve_into_the_pad_and_a_single_pass_holds_half_its_blocks() {
        let t = Tuning::default();
        let mut g = WearGrid::new(64);
        drive_up(&mut g, ASSAULT, Position::new(200.0, 300.0), 120, 0.0, &t);
        let run: Vec<Position> = (0..32).flat_map(|k| [187.0, 189.0, 191.0].map(|x| Position::new(x, 221.0 + 2.0 * k as f32))).collect();
        let count = |age: f32, c: Color| run.iter().filter(|p| colour_after(&g, **p, age, look(), &t) == Some(c)).count();
        assert!(count(1.0, GRASS.grouser[0]) > 0, "fresh: bars");
        assert_eq!(count(t.wear_settle_seconds + 1.0, GRASS.grouser[0]), 0, "settled: no bars");
        let shown = |age: f32| run.iter().filter(|p| colour_after(&g, **p, age, look(), &t).is_some()).count();
        assert_eq!(shown(1.0), run.len());
        let late = shown(t.wear_fade_seconds * 10.0);
        assert!(late * 2 >= run.len() - 2 && late * 2 <= run.len() + 2, "half held: {late} of {}", run.len());
    }

    #[test]
    fn falling_snow_fills_a_single_pass_back_in() {
        let t = Tuning::default();
        let snow = Look { sky: Sky::Snow, ..look() };
        let mut g = WearGrid::new(64);
        for k in 0..=120 {
            g.press(&stamp(Position::new(200.0, 300.0 - k as f32), 0.0, ASSAULT, Roll::Rolling), snow, &grass, k as f32 / 140.0, &t);
        }
        let at = Position::new(187.0, 251.0);
        assert!(colour_after(&g, at, 1.0, snow, &t).is_some());
        let filled = t.wear_press_snow / t.wear_snow_refill_per_second + 1.0;
        assert!(colour_after(&g, at, filled, snow, &t).is_none());
    }

    #[test]
    fn rain_fills_a_rut_with_water() {
        let t = Tuning::default();
        let rain = Look { sky: Sky::Rain, ..look() };
        let mut g = WearGrid::new(64);
        for pass in 0..2 {
            for k in 0..=120 {
                g.press(&stamp(Position::new(200.0, 300.0 - k as f32), 0.0, ASSAULT, Roll::Rolling), rain, &grass, pass as f32 * 5.0 + k as f32 / 140.0, &t);
            }
        }
        let run: Vec<Position> = (0..32).flat_map(|k| [187.0, 189.0, 191.0].map(|x| Position::new(x, 221.0 + 2.0 * k as f32))).collect();
        let wet = |age: f32| run.iter().filter(|p| matches!(colour_after(&g, **p, age, rain, &t), Some(c) if c == PUDDLE || c == PUDDLE_SHEEN)).count();
        assert_eq!(wet(0.0), 0, "a fresh rut is dry");
        assert!(wet(t.wear_puddle_fill_seconds * 2.0) > 0, "a rut left in the rain holds water");
    }

    #[test]
    fn the_pictures_bake_only_the_cells_in_view() {
        let t = Tuning::default();
        let mut g = WearGrid::new(64);
        drive_up(&mut g, ASSAULT, Position::new(64.0, 300.0), 40, 0.0, &t);
        drive_up(&mut g, ASSAULT, Position::new(640.0, 300.0), 40, 0.0, &t);
        let s = |_: Position| Surface::Grass;
        let view = [Rectangle::new(0.0, 200.0, 200.0, 200.0)];
        g.refresh_pictures(&PictureFrame { time: 1.0, look: look(), views: &view, surface: &s }, &t);
        let p = g.pictures();
        assert!(p.atlas().len() > 0);
        assert!(p.atlas().cells().all(|(c, _)| c < 10), "only cells near the view");
        drop(p);
        let elsewhere = [Rectangle::new(500.0, 200.0, 300.0, 200.0)];
        g.refresh_pictures(&PictureFrame { time: 1.1, look: look(), views: &elsewhere, surface: &s }, &t);
        assert!(g.pictures().atlas().cells().all(|(c, _)| c > 10), "the cells out of view were dropped");
    }
}
