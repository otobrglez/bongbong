//! Biomes: what kind of place a battlefield is (docs/biomes.md). A biome is
//! a look and nothing else - no collider, footing, nav cost, cover or RNG
//! draw reads it - so a map plays the same with or without one.
//!
//! A map names its biome with a top-level `biome = "valley"`; absent, it is
//! drawn as it always was. The `biome_override` knob puts a biome over
//! every local round to compare (`in_force`). A biome is separate from the
//! map's `theme`, which also decides whether its trees are dry.
//!
//! What a biome draws today is its **materials**: a layer of 2 px blocks
//! baked once over the floor tiles (`bake`) - damp silt along every bank,
//! where it also covers the tileset's olive shore outline, broad shelves of
//! gravel by the water, packed earth on a road's shoulders, and patches of
//! two neighbouring greens across the open meadow. It is placed by rules
//! read from the tiles themselves (`ground::GroundGrid::floor_px`, the mask
//! `tools/ground_mask.py` generates), hashed from position with no seed and
//! no RNG, so a map draws the same in every round, in the builder and in a
//! thumbnail. `ground::draw` lays it right over the tiles, so the floor
//! shade, the forest floor, the marks and the weather's snow and puddles
//! all fall on top of it. A biome's water colours (`WaterRamp`) are part of
//! its look already; the tiles take them in a later stage (BB-194).

use serde::{Deserialize, Serialize};

use crate::canvas::BlockImage;
use crate::ground::{Deck, FloorPx};
use crate::map::{CellObject, MapFile};
use crate::obstacle::Material;
use crate::math::Color;
use crate::tuning::Tuning;

/// The kinds of place a map can be. See the module comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Biome {
    /// The Soča gravel valley: clear alpine water, gravel by the banks,
    /// earth on the road's shoulders, a meadow in broad patches.
    Valley,
}

impl Biome {
    /// Every biome, in the order the `biome_override` knob counts them
    /// from 1.
    pub const ALL: [Biome; 1] = [Biome::Valley];

    /// The TOML spelling, also the dev server's.
    pub fn name(self) -> &'static str {
        match self {
            Biome::Valley => "valley",
        }
    }

    pub fn parse(s: &str) -> Option<Biome> {
        Biome::ALL.iter().copied().find(|b| b.name() == s)
    }

    /// The biome's own ground tileset (`tools/retint_ground.py`), drawn in
    /// place of the theme's: the same tiles, its water and banks in the
    /// biome's colours.
    pub fn ground_texture_path(self) -> &'static str {
        match self {
            Biome::Valley => "static/punyworld/punyworld-overworld-tileset-valley.png",
        }
    }

    /// What this biome looks like.
    pub fn look(self) -> &'static BiomeLook {
        match self {
            Biome::Valley => &VALLEY,
        }
    }
}

/// The biome a round is drawn in: the map's own, unless `biome_override`
/// says otherwise - `-1` follows the map, `0` draws none, `k` draws
/// `Biome::ALL[k - 1]`.
pub fn in_force(map: Option<Biome>, t: &Tuning) -> Option<Biome> {
    match t.biome_override {
        -1 => map,
        k if k >= 1 => Biome::ALL.get(k as usize - 1).copied().or(map),
        _ => None,
    }
}

/// A biome's water, one step for each role the tileset's water colours
/// play (`tools/ground_mask.py` names the tiles; BB-192 the mapping): the
/// shadow under a bank, the darker and the plain water, its lit part, the
/// ripples and the glints, and the open water no hull enters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterRamp {
    pub edge: Color,
    pub dark: Color,
    pub fill: Color,
    pub light: Color,
    pub ripple: Color,
    pub glint: Color,
    pub deep: Color,
    pub deep_dark: Color,
    /// A pebble on a ford's bed, seen through the water.
    pub bed: Color,
}

/// Everything a biome draws in, and where.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BiomeLook {
    pub water: WaterRamp,
    /// Pale, middle and dark stones of a gravel shelf.
    pub gravel: [Color; 3],
    /// Damp silt along a bank: the step away from the water, then the
    /// darker one beside it.
    pub silt: [Color; 2],
    /// Packed earth on a road's shoulders.
    pub shoulder: [Color; 2],
    /// The meadow's darker and lighter patches: a green laid over the
    /// grass at a low opacity, so the grass tiles' own specks show through.
    pub meadow_dark: Color,
    pub meadow_light: Color,
    /// How far gravel reaches from the water, px: `gravel_reach.0` where a
    /// shelf is thin and up to `.0 + .1` where it is broad.
    pub gravel_reach: (f32, f32),
    /// How far packed earth reaches from a road's dirt, px, the same way.
    pub shoulder_reach: (f32, f32),
    /// A plank deck over water (`ground::Deck`): its rails, the seams
    /// between planks, and the planks' two tones.
    pub deck: [Color; 4],
}

/// The Soča gravel valley, in the steps chosen on the palette card
/// (BB-192).
pub const VALLEY: BiomeLook = BiomeLook {
    water: WaterRamp {
        edge: Color::new(0x14, 0x5F, 0x70, 255),
        dark: Color::new(0x1B, 0x75, 0x84, 255),
        fill: Color::new(0x25, 0x8E, 0x9A, 255),
        light: Color::new(0x38, 0xA5, 0xA9, 255),
        ripple: Color::new(0x6C, 0xC7, 0xC0, 255),
        glint: Color::new(0xB4, 0xE6, 0xDE, 255),
        deep: Color::new(0x16, 0x5A, 0x6A, 255),
        deep_dark: Color::new(0x11, 0x4B, 0x59, 255),
        bed: Color::new(0x5E, 0x9C, 0x92, 255),
    },
    gravel: [Color::new(0xE2, 0xD4, 0xB0, 255), Color::new(0xC9, 0xB9, 0x93, 255), Color::new(0xA9, 0x9A, 0x78, 255)],
    silt: [Color::new(0x6E, 0x5A, 0x44, 255), Color::new(0x55, 0x45, 0x36, 255)],
    shoulder: [Color::new(0xB0, 0x98, 0x68, 255), Color::new(0x97, 0x7F, 0x55, 255)],
    // GREEN_DARKEST and a light leaf green at 12%: the grass fill #619541
    // comes out about #598C3F and #65993F.
    meadow_dark: Color::new(0x1C, 0x4C, 0x33, 31),
    meadow_light: Color::new(0x7F, 0xB1, 0x5A, 38),
    gravel_reach: (8.0, 22.0),
    shoulder_reach: (3.0, 7.0),
    // Weathered timber, greyer than the orange of a timber wall.
    deck: [Color::new(0x3A, 0x2A, 0x1C, 255), Color::new(0x4E, 0x3A, 0x28, 255), Color::new(0x8A, 0x6A, 0x48, 255), Color::new(0x7A, 0x5C, 0x3D, 255)],
};

/// A 32-bit hash of a lattice point and a salt (SplitMix64's finaliser):
/// the same on every machine, no seed.
fn hash(x: i32, y: i32, salt: u32) -> u32 {
    let mut h = (x as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as i64 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9) ^ (salt as u64).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    (h >> 32) as u32
}

/// Value noise in 0..1 at field px (`x`, `y`): hashed lattice points every
/// `scale` px, blended by smoothstep. Only adds and multiplies, so it is
/// the same on every platform.
fn noise(x: f32, y: f32, scale: f32, salt: u32) -> f32 {
    let (fx, fy) = (x / scale, y / scale);
    let (ix, iy) = (fx.floor(), fy.floor());
    let (mut tx, mut ty) = (fx - ix, fy - iy);
    tx = tx * tx * (3.0 - 2.0 * tx);
    ty = ty * ty * (3.0 - 2.0 * ty);
    let (ix, iy) = (ix as i32, iy as i32);
    let at = |dx: i32, dy: i32| hash(ix + dx, iy + dy, salt) as f32 / u32::MAX as f32;
    let (a, b, c, d) = (at(0, 0), at(1, 0), at(0, 1), at(1, 1));
    a + (b - a) * tx + (c - a) * ty + (a - b - c + d) * tx * ty
}

/// How many blocks each block lies from the nearest block `near` says
/// yes to, stepping across and down (so a ring is a diamond); `u16::MAX`
/// where there is none.
fn distance(w: usize, h: usize, near: impl Fn(usize) -> bool) -> Vec<u16> {
    let mut d = vec![u16::MAX; w * h];
    let mut queue = std::collections::VecDeque::new();
    for (i, cell) in d.iter_mut().enumerate() {
        if near(i) {
            *cell = 0;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        let (x, y, next) = (i % w, i / w, d[i].saturating_add(1));
        let mut visit = |j: usize| {
            if d[j] > next {
                d[j] = next;
                queue.push_back(j);
            }
        };
        if x > 0 {
            visit(i - 1);
        }
        if x + 1 < w {
            visit(i + 1);
        }
        if y > 0 {
            visit(i - w);
        }
        if y + 1 < h {
            visit(i + w);
        }
    }
    d
}

/// The materials layer of a field `w` x `h` blocks of 2 px, in `look`: one
/// texel a block, clear where the tiles show through. `class` says what
/// the tiles lay on each block, row by row (`GroundGrid::floor_blocks`).
/// See the module comment for what goes where.
pub fn bake(look: &BiomeLook, w: usize, h: usize, class: &[FloorPx], decks: &[Deck], places: &Places, stamp: u64) -> BlockImage {
    assert_eq!(class.len(), w * h, "a class a block");
    let to_water = distance(w, h, |i| class[i].is_water());
    let deepness = deepness(w, h, class);
    let to_road = distance(w, h, |i| class[i] == FloorPx::Road);
    let clear = Color::new(0, 0, 0, 0);
    let mut texels = vec![clear; w * h];
    for (i, texel) in texels.iter_mut().enumerate() {
        let (bx, by) = ((i % w) as i32, (i / w) as i32);
        let (x, y) = (bx as f32 * 2.0, by as f32 * 2.0);
        let threshold = crate::pyro::bayer(bx, by);
        let wet = to_water[i] as f32 * 2.0;
        let dirt = to_road[i] as f32 * 2.0;
        *texel = match class[i] {
            FloorPx::Road => continue,
            // Open water as a soft body rather than the cells' squares
            // (`deepness`): the tileset paints a deep cell whole, so its
            // edge lightens back to the ford, and a ford beside it darkens.
            // A lone deep cell reads as a round pool.
            FloorPx::Deep => {
                if deepness[i] >= threshold {
                    continue;
                }
                look.water.fill
            }
            // A ford shows a pebble of its bed here and there, away from
            // the open water.
            FloorPx::Water => {
                if deepness[i] > 0.0 && deepness[i] >= threshold {
                    look.water.deep
                } else if deepness[i] <= 0.0 && hash(bx, by, 13) % 23 == 0 {
                    look.water.bed
                } else {
                    continue;
                }
            }
            // The tileset's shore outline, in silt: darker beside the water.
            FloorPx::Bank => look.silt[if to_water[i] <= 1 { 1 } else { 0 }],
            FloorPx::Land => {
                // A seam of damp silt right along the bank.
                if wet <= 4.0 {
                    look.silt[0]
                } else if let Some(stone) = gravel(look, x, y, wet, threshold) {
                    stone
                } else if let Some(earth) = shoulder(look, x, y, dirt, threshold) {
                    earth
                } else {
                    let n = noise(x, y, 220.0, 5);
                    if n < 0.38 && (0.38 - n) * 10.0 >= threshold {
                        look.meadow_dark
                    } else if n > 0.64 && (n - 0.64) * 10.0 >= threshold {
                        look.meadow_light
                    } else {
                        continue;
                    }
                }
            }
        };
    }
    for &(cell, across) in &places.fields {
        lay_furrows(look, cell, across, w, h, class, &mut texels);
    }
    for &(from, to) in &places.races {
        lay_race(look, from, to, w, h, class, &mut texels);
    }
    for deck in decks {
        lay_deck(look, deck, w, h, class, &mut texels);
    }
    BlockImage { width: w, height: h, block: 2, texels, stamp, patches: Vec::new() }
}

/// How far into open water each block of a field `w` x `h` blocks lies,
/// 0 to 1: every deep cell (one whose middle block is `FloorPx::Deep`)
/// adds a cone `DEEP_REACH` px across its centre, full within
/// `DEEP_REACH - DEEP_FADE` px, and the sum is held to 1. A run of deep
/// cells adds up to one body with a soft, rounded edge about where the
/// cells end; a lone cell is a round pool. Only `sqrt` and the basic
/// operations, exact on every platform.
fn deepness(w: usize, h: usize, class: &[FloorPx]) -> Vec<f32> {
    // A cell's corner is 22.6 px from the four centres round it, so a
    // reach past that keeps a lake's middle solid where cells meet; full
    // out to 16 px, a deep cell's own edge.
    const DEEP_REACH: f32 = 26.0;
    const DEEP_FADE: f32 = 10.0;
    let mut out = vec![0.0f32; w * h];
    // A cell is 16 blocks and centred on its world cell, so cell `c`'s
    // middle block is `c * 16`.
    let (cols, rows) = (w as i32 / 16 + 1, h as i32 / 16 + 1);
    let reach = (DEEP_REACH / 2.0).ceil() as i32;
    for r in 0..rows {
        for c in 0..cols {
            let (cx, cy) = (c * 16, r * 16);
            if cx as usize >= w || cy as usize >= h || class[cy as usize * w + cx as usize] != FloorPx::Deep {
                continue;
            }
            for by in (cy - reach).max(0)..(cy + reach + 1).min(h as i32) {
                for bx in (cx - reach).max(0)..(cx + reach + 1).min(w as i32) {
                    // Block centres, in px.
                    let (dx, dy) = ((bx - cx) as f32 * 2.0 + 1.0, (by - cy) as f32 * 2.0 + 1.0);
                    let d2 = dx * dx + dy * dy;
                    if d2 >= DEEP_REACH * DEEP_REACH {
                        continue;
                    }
                    let i = by as usize * w + bx as usize;
                    out[i] = (out[i] + ((DEEP_REACH - d2.sqrt()) / DEEP_FADE).min(1.0)).min(1.0);
                }
            }
        }
    }
    out
}

/// What a biome dresses on a map, found by rule from its cells (`places`):
/// nothing here is authored, so every map with a biome gets them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Places {
    /// Mill races, field px: from the middle of a timber building's wall
    /// cell to the middle of the water cell nearest it.
    pub races: Vec<((f32, f32), (f32, f32))>,
    /// Ploughed cells behind a fence, and whether their furrows run
    /// east-west (along a fence that does).
    pub fields: Vec<((i32, i32), bool)>,
}

/// How far a timber building may stand from the water and still have a
/// race, in cells.
const RACE_REACH: i32 = 3;

/// How far from a fence its field reaches, in cells.
const FIELD_REACH: i32 = 4;

/// The places `map` dresses under a biome:
/// - **a mill race** for every timber building (wood walls touching, by
///   eight neighbours) within `RACE_REACH` cells of water: from the wall
///   cell nearest the water to that water, the nearest pair winning and
///   ties going to the first in cell order;
/// - **a field** of every tall-grass cell within `FIELD_REACH` cells of a
///   fence, its furrows running along the nearest fence - a patch of them,
///   never a lone cell.
pub fn places(map: &MapFile) -> Places {
    use std::collections::{BTreeMap, BTreeSet};
    let cells: BTreeMap<(i32, i32), CellObject> = map.iter_cells().map(|(c, r, o)| ((c, r), *o)).collect();
    let water: Vec<(i32, i32)> = cells.iter().filter(|(_, o)| **o == CellObject::Water).map(|(c, _)| *c).collect();
    let timber: BTreeSet<(i32, i32)> = cells.iter().filter(|(_, o)| o.material() == Some(Material::Wood) && o.is_solid()).map(|(c, _)| *c).collect();
    let chebyshev = |a: (i32, i32), b: (i32, i32)| (a.0 - b.0).abs().max((a.1 - b.1).abs());
    let centre = |(c, r): (i32, i32)| (c as f32 * crate::OBSTACLE_GRID_SIZE, r as f32 * crate::OBSTACLE_GRID_SIZE);
    let mut places = Places::default();
    // Timber buildings, each one race.
    let mut seen = BTreeSet::new();
    for &start in &timber {
        if !seen.insert(start) {
            continue;
        }
        let mut building = vec![start];
        let mut i = 0;
        while i < building.len() {
            let (c, r) = building[i];
            for (dx, dy) in (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (dx, dy))) {
                let n = (c + dx, r + dy);
                if timber.contains(&n) && seen.insert(n) {
                    building.push(n);
                }
            }
            i += 1;
        }
        building.sort();
        let best = building.iter().flat_map(|&wall| water.iter().map(move |&wet| (chebyshev(wall, wet), wall, wet))).min();
        if let Some((d, wall, wet)) = best
            && d <= RACE_REACH
        {
            places.races.push((centre(wall), centre(wet)));
        }
    }
    // Fields behind the fences.
    let fences: Vec<(i32, i32)> = cells.iter().filter(|(_, o)| **o == CellObject::Fence).map(|(c, _)| *c).collect();
    let runs_across = |(c, r): (i32, i32)| fences.contains(&(c - 1, r)) || fences.contains(&(c + 1, r));
    for (&cell, obj) in &cells {
        if *obj != CellObject::TallGrass {
            continue;
        }
        let nearest = fences.iter().map(|&f| (chebyshev(cell, f), f)).min();
        if let Some((d, fence)) = nearest
            && d <= FIELD_REACH
        {
            places.fields.push((cell, runs_across(fence)));
        }
    }
    // A field is a patch, not a lone tuft: a cell keeps its furrows only
    // with two field cells among its eight neighbours.
    let field: BTreeSet<(i32, i32)> = places.fields.iter().map(|(c, _)| *c).collect();
    places.fields.retain(|&((c, r), _)| (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (c + dx, r + dy))).filter(|n| *n != (c, r) && field.contains(n)).count() >= 2);
    places
}

/// Furrows over field cell `cell`: rows of dark furrow and earthen ridge
/// with grass between, every 6 px, running east-west when `across`.
fn lay_furrows(look: &BiomeLook, cell: (i32, i32), across: bool, w: usize, h: usize, class: &[FloorPx], texels: &mut [Color]) {
    let (bx0, by0) = (cell.0 * 16 - 8, cell.1 * 16 - 8);
    for ly in 0..16 {
        for lx in 0..16 {
            let (bx, by) = (bx0 + lx, by0 + ly);
            if bx < 0 || by < 0 || bx as usize >= w || by as usize >= h {
                continue;
            }
            let i = by as usize * w + bx as usize;
            if class[i] != FloorPx::Land {
                continue;
            }
            // Counted on the field's own grid, so neighbouring cells line up.
            let row = if across { by } else { bx };
            texels[i] = match row.rem_euclid(3) {
                0 => look.silt[1],
                1 => look.shoulder[1],
                _ => continue,
            };
        }
    }
}

/// A mill race from `from` to `to` (field px): a channel of water 6 px
/// across lined with stone, cut through the ground and the bank, and a
/// line of weir foam where it meets the river.
fn lay_race(look: &BiomeLook, from: (f32, f32), to: (f32, f32), w: usize, h: usize, class: &[FloorPx], texels: &mut [Color]) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len2 = (dx * dx + dy * dy).max(1.0);
    let reach = 10.0;
    let (x0, x1) = ((from.0.min(to.0) - reach) / 2.0, (from.0.max(to.0) + reach) / 2.0);
    let (y0, y1) = ((from.1.min(to.1) - reach) / 2.0, (from.1.max(to.1) + reach) / 2.0);
    for by in (y0.floor().max(0.0) as usize)..(y1.ceil().max(0.0) as usize).min(h) {
        for bx in (x0.floor().max(0.0) as usize)..(x1.ceil().max(0.0) as usize).min(w) {
            let (x, y) = (bx as f32 * 2.0 + 1.0, by as f32 * 2.0 + 1.0);
            let t = (((x - from.0) * dx + (y - from.1) * dy) / len2).clamp(0.0, 1.0);
            let (px, py) = (from.0 + dx * t, from.1 + dy * t);
            let d = ((x - px) * (x - px) + (y - py) * (y - py)).sqrt();
            let i = by * w + bx;
            match class[i] {
                FloorPx::Land | FloorPx::Bank if d <= 3.0 => texels[i] = look.water.fill,
                FloorPx::Land | FloorPx::Bank if d <= 5.0 => texels[i] = look.gravel[2],
                // Where it falls into the river: foam across its mouth.
                FloorPx::Water if d <= 8.0 && t > 0.0 && texels[i].a == 0 => {
                    if hash(bx as i32, by as i32, 19) % 3 != 0 {
                        texels[i] = if hash(bx as i32, by as i32, 23) % 2 == 0 { look.water.glint } else { look.water.ripple };
                    }
                }
                _ => {}
            }
        }
    }
}

/// The planks of `deck` over its cell: a band 24 px across the road's
/// way, railed on both edges, its planks running across the water - 4 px
/// each, a seam between, their tone hashed - and a strip of the water's
/// shadow along each rail.
fn lay_deck(look: &BiomeLook, deck: &Deck, w: usize, h: usize, class: &[FloorPx], texels: &mut [Color]) {
    let [rail, seam, plank_a, plank_b] = look.deck;
    // The cell's corner in blocks: a cell spans 16 px either side of its
    // centre (`ground` lays its cells centred).
    let (bx0, by0) = (deck.cell.0 * 16 - 8, deck.cell.1 * 16 - 8);
    for ly in 0..16 {
        for lx in 0..16 {
            let (bx, by) = (bx0 + lx, by0 + ly);
            if bx < 0 || by < 0 || bx as usize >= w || by as usize >= h {
                continue;
            }
            let i = by as usize * w + bx as usize;
            // Along the road and across it, in blocks.
            let (along, across) = if deck.across { (lx, ly) } else { (ly, lx) };
            let texel = match across {
                2 | 13 => rail,
                3..=12 if along % 3 == 2 => seam,
                3..=12 => {
                    if hash(deck.cell.0 * 7 + along / 3, deck.cell.1, 17) % 2 == 0 { plank_a } else { plank_b }
                }
                1 | 14 if class[i].is_water() => look.water.edge,
                _ => continue,
            };
            texels[i] = texel;
        }
    }
}

/// The stone of a gravel shelf at field px (`x`, `y`), `wet` px from the
/// water, if one lies there: shelves come and go along a bank in a noise
/// about a cell and a half across, broad where it is high, and fray into
/// the meadow over their last 8 px.
fn gravel(look: &BiomeLook, x: f32, y: f32, wet: f32, threshold: f32) -> Option<Color> {
    if wet > look.gravel_reach.0 + look.gravel_reach.1 {
        return None;
    }
    let shelf = noise(x, y, 44.0, 7);
    let reach = look.gravel_reach.0 + look.gravel_reach.1 * shelf;
    if shelf <= 0.5 || wet > reach || (reach - wet) / 8.0 < threshold {
        return None;
    }
    let grain = noise(x, y, 6.0, 11);
    Some(look.gravel[if grain > 0.72 { 2 } else if grain > 0.3 { 0 } else { 1 }])
}

/// The packed earth on a road's shoulder at field px (`x`, `y`), `dirt` px
/// from the road's dirt, if it reaches there.
fn shoulder(look: &BiomeLook, x: f32, y: f32, dirt: f32, threshold: f32) -> Option<Color> {
    if dirt <= 0.0 || dirt > look.shoulder_reach.0 + look.shoulder_reach.1 {
        return None;
    }
    let wear = noise(x, y, 22.0, 3);
    let reach = look.shoulder_reach.0 + look.shoulder_reach.1 * wear;
    if dirt <= 0.0 || dirt > reach || (reach - dirt) / 4.0 < threshold {
        return None;
    }
    Some(look.shoulder[if wear > 0.6 { 1 } else { 0 }])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CIE 1976 colour difference between two opaque colours, in CIELAB.
    fn delta_e(a: Color, b: Color) -> f32 {
        fn lab(c: Color) -> [f32; 3] {
            let lin = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.04045 { v / 12.92 } else { crate::math::powf((v + 0.055) / 1.055, 2.4) }
            };
            let (r, g, b) = (lin(c.r), lin(c.g), lin(c.b));
            let x = (r * 0.4124 + g * 0.3576 + b * 0.1805) / 0.950_47;
            let y = r * 0.2126 + g * 0.7152 + b * 0.0722;
            let z = (r * 0.0193 + g * 0.1192 + b * 0.9505) / 1.088_83;
            let f = |t: f32| if t > 0.008_856 { crate::math::powf(t, 1.0 / 3.0) } else { 7.787 * t + 16.0 / 116.0 };
            [116.0 * f(y) - 16.0, 500.0 * (f(x) - f(y)), 200.0 * (f(y) - f(z))]
        }
        let (p, q) = (lab(a), lab(b));
        ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
    }

    /// A translucent meadow tone as it lands on the grass fill.
    fn over_grass(c: Color) -> Color {
        let (a, grass) = (c.a as f32 / 255.0, Color::new(0x61, 0x95, 0x41, 255));
        let mix = |g: u8, v: u8| (g as f32 * (1.0 - a) + v as f32 * a).round() as u8;
        Color::new(mix(grass.r, c.r), mix(grass.g, c.g), mix(grass.b, c.b), 255)
    }

    /// The least colour difference a material may have from anything a
    /// player has to pick out on it. Today's road and the ammo crate are
    /// 30 apart; 20 is clearly two colours at a glance.
    const MIN_DELTA_E: f32 = 20.0;

    #[test]
    fn every_material_stands_apart_from_every_seat_pickup_and_shell() {
        let mut things: Vec<(String, Color)> = crate::tank::TEAM_COLORS.iter().enumerate().map(|(i, c)| (format!("seat {}", i + 1), *c)).collect();
        things.extend([
            ("health crate".to_string(), Color::new(0xE4, 0x42, 0x19, 255)),
            ("ammo crate".to_string(), Color::new(0xDE, 0x99, 0x43, 255)),
            ("blue shell".to_string(), Color::new(0x8F, 0xD3, 0xFF, 255)),
            ("orange shell".to_string(), Color::new(0xFF, 0x9F, 0x2F, 255)),
        ]);
        for biome in Biome::ALL {
            let look = biome.look();
            let mut materials: Vec<(&str, Color)> = Vec::new();
            materials.extend(look.gravel.iter().map(|c| ("gravel", *c)));
            materials.extend(look.silt.iter().map(|c| ("silt", *c)));
            materials.extend(look.shoulder.iter().map(|c| ("shoulder", *c)));
            materials.extend(look.deck[2..].iter().map(|c| ("deck plank", *c)));
            materials.push(("dark meadow", over_grass(look.meadow_dark)));
            materials.push(("light meadow", over_grass(look.meadow_light)));
            for (name, material) in &materials {
                for (thing, colour) in &things {
                    let d = delta_e(*material, *colour);
                    assert!(d >= MIN_DELTA_E, "{}: {name} {material:?} is only {d:.1} from the {thing}", biome.name());
                }
            }
        }
    }

    #[test]
    fn every_seat_the_teal_hull_and_glass_stand_apart_from_the_water() {
        let teal_hull = Color::new(0x00, 0xA6, 0x7F, 255);
        let glass = Color::new(0x86, 0xC7, 0xD9, 255);
        for biome in Biome::ALL {
            let w = biome.look().water;
            let mut things: Vec<(String, Color)> = crate::tank::TEAM_COLORS.iter().enumerate().map(|(i, c)| (format!("seat {}", i + 1), *c)).collect();
            things.push(("teal hull".to_string(), teal_hull));
            things.push(("glass".to_string(), glass));
            for (role, water) in [("ford", w.fill), ("open water", w.deep)] {
                for (thing, colour) in &things {
                    let d = delta_e(water, *colour);
                    assert!(d >= MIN_DELTA_E, "{}: the {thing} is only {d:.1} from the {role}", biome.name());
                }
            }
        }
    }

    #[test]
    fn open_water_stands_apart_from_a_ford() {
        for biome in Biome::ALL {
            let w = biome.look().water;
            let d = delta_e(w.fill, w.deep);
            assert!(d >= 10.0, "{}: the ford and the open water are only {d:.1} apart", biome.name());
        }
    }

    #[test]
    fn a_meadow_patch_stays_green() {
        // The de-green rule (docs/PALETTE.md): the meadow stays green -
        // green the strongest channel - and never turns toward khaki.
        for biome in Biome::ALL {
            let look = biome.look();
            for c in [over_grass(look.meadow_dark), over_grass(look.meadow_light)] {
                assert!(c.g > c.r && c.g > c.b && c.r < c.g - 40, "{}: meadow patch {c:?} is not green", biome.name());
            }
        }
    }

    #[test]
    fn the_override_follows_the_map_draws_none_or_puts_a_biome_over_it() {
        let mut t = Tuning::DEFAULT;
        assert_eq!(in_force(Some(Biome::Valley), &t), Some(Biome::Valley));
        assert_eq!(in_force(None, &t), None);
        t.biome_override = 0;
        assert_eq!(in_force(Some(Biome::Valley), &t), None);
        t.biome_override = 1;
        assert_eq!(in_force(None, &t), Some(Biome::Valley));
    }

    /// A field 8 cells square: water down columns 3 and 4, road along
    /// row 5, plain ground elsewhere - by pixel, as a mask would say.
    fn tiny(x: i32, y: i32) -> FloorPx {
        if (96..160).contains(&x) {
            FloorPx::Water
        } else if (160..166).contains(&x) {
            FloorPx::Bank
        } else if (160..176).contains(&y) {
            FloorPx::Road
        } else {
            FloorPx::Land
        }
    }

    /// `tiny` for every block of its 128 x 128 blocks.
    fn tiny_blocks() -> Vec<FloorPx> {
        (0..128 * 128).map(|i| tiny((i % 128) * 2, (i / 128) * 2)).collect()
    }

    #[test]
    fn the_layer_leaves_water_and_road_alone_and_lines_every_bank() {
        let image = bake(&VALLEY, 128, 128, &tiny_blocks(), &[], &Places::default(), 1);
        let at = |x: i32, y: i32| image.texels[(y / 2) as usize * image.width + (x / 2) as usize];
        assert_eq!(at(120, 40).a, 0, "water is the tiles'");
        assert_eq!(at(200, 166).a, 0, "a road's dirt is the tiles'");
        for y in (0..256).step_by(2).filter(|y| !(160..176).contains(y)) {
            let bank = at(162, y);
            assert!(VALLEY.silt.contains(&bank), "the bank at y {y} is {bank:?}, not silt");
        }
    }

    #[test]
    fn the_layer_is_the_same_every_time_it_is_baked() {
        let a = bake(&VALLEY, 128, 128, &tiny_blocks(), &[], &Places::default(), 1);
        let b = bake(&VALLEY, 128, 128, &tiny_blocks(), &[], &Places::default(), 2);
        assert_eq!(a.texels, b.texels);
    }

    #[test]
    fn serpent_river_has_one_race_at_its_watermill_and_fields_by_its_fences() {
        let map = MapFile::load(std::path::Path::new("maps/serpent-river.toml")).unwrap();
        let places = places(&map);
        assert_eq!(places.races.len(), 1, "one timber building stands by the water: {:?}", places.races);
        let (from, _) = places.races[0];
        let mill = crate::map::world_to_cell(crate::Position::new(from.0, from.1));
        assert!((31..=34).contains(&mill.0) && (8..=10).contains(&mill.1), "the race leaves the watermill, not {mill:?}");
        assert!(!places.fields.is_empty());
        for &(cell, across) in &places.fields {
            assert_eq!(map.cell(cell.0, cell.1), Some(&CellObject::TallGrass), "{cell:?} is a field of tall grass");
            assert!(across, "the farm's fences run east-west, so do its furrows");
        }
    }

    #[test]
    fn a_biome_round_trips_through_its_name() {
        for biome in Biome::ALL {
            assert_eq!(Biome::parse(biome.name()), Some(biome));
        }
        assert_eq!(Biome::parse("lagoon"), None);
    }
}
