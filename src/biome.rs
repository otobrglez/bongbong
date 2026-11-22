//! Biomes: what kind of place a battlefield is (docs/biomes.md). A biome is
//! a look and nothing else - no collider, footing, nav cost, cover or RNG
//! draw reads it - so a map plays the same with or without one.
//!
//! A map names its biome with a top-level `biome = "valley"`; absent, it is
//! drawn as it always was. The `biome_override` knob puts a biome over
//! every local round to compare (`in_force`). A biome is separate from the
//! map's `theme`, which also decides whether its trees are dry.
//!
//! There are three - the valley, the floodplain and the karst - each made
//! for one theme (`Biome::theme`): its tileset is that theme's retint with
//! its own water and banks (`tools/retint_ground.py`).
//!
//! Over the tiles a biome draws its **materials**: a layer of 2 px blocks
//! baked once (`bake`) - damp ground along every bank, where it also covers
//! the tileset's shore outline, a band of gravel or sedge by the water,
//! packed earth on a road's shoulders, the karst's limestone plates, and
//! patches of two neighbouring tones across the open ground - and what it
//! dresses by rule (`places`): a mill race, furrows, a floodwork apron. It
//! is placed by rules read from the tiles themselves
//! (`ground::GroundGrid::floor_px`, the mask `tools/ground_mask.py`
//! generates), hashed from position with no seed and no RNG, so a map draws
//! the same in every round, in the builder and in a thumbnail.
//! `ground::draw` lays it right over the tiles, so the floor shade, the
//! forest floor, the marks and the weather's snow and puddles all fall on
//! top of it.

use serde::{Deserialize, Serialize};

use crate::canvas::BlockImage;
use crate::ground::{Deck, FloorPx};
use crate::map::{CellObject, MapFile, Theme};
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
    /// A reclaimed floodplain: slate still water with a lighter submerged
    /// margin, sedge on the wet banks, peat at the shore, an old floodwork
    /// apron of paving round a tower that stands by the water.
    Floodplain,
    /// A dry karst frontier, on the desert's dust: warm hardpan, flat
    /// limestone plates with hairline fissures, dark pools with a damp
    /// seam, dry scrub in the shelter of walls.
    Karst,
}

impl Biome {
    /// Every biome, in the order the `biome_override` knob counts them
    /// from 1.
    pub const ALL: [Biome; 3] = [Biome::Valley, Biome::Floodplain, Biome::Karst];

    /// The TOML spelling, also the dev server's.
    pub fn name(self) -> &'static str {
        match self {
            Biome::Valley => "valley",
            Biome::Floodplain => "floodplain",
            Biome::Karst => "karst",
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
            Biome::Floodplain => "static/punyworld/punyworld-overworld-tileset-floodplain.png",
            Biome::Karst => "static/punyworld/punyworld-overworld-tileset-karst.png",
        }
    }

    /// The theme the biome is made for: its tileset is that theme's
    /// retint (`tools/retint_ground.py`'s `base`), so on a map of another
    /// theme its ground would not match the trees and the grass. The
    /// builder's LOOK row offers only the biomes of the map's theme.
    pub fn theme(self) -> Theme {
        match self {
            Biome::Valley | Biome::Floodplain => Theme::Grass,
            Biome::Karst => Theme::Desert,
        }
    }

    /// What this biome looks like.
    pub fn look(self) -> &'static BiomeLook {
        match self {
            Biome::Valley => &VALLEY,
            Biome::Floodplain => &FLOODPLAIN,
            Biome::Karst => &KARST,
        }
    }

    /// Whether this biome dresses the built tiles (`obstacle::TileLook`):
    /// brick drawn as pale masonry and every fence as post and wire, on
    /// the valley's farms (BB-203). Picture only, like the rest of a look.
    pub fn dresses_tiles(self) -> bool {
        matches!(self, Biome::Valley)
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
    /// The pale, middle and dark tones of the band along the water past
    /// the silt (`shore`): a gravel shelf's stones, or a sedge bed's stems.
    pub gravel: [Color; 3],
    /// What that band is.
    pub shore: Shore,
    /// Damp ground along a bank - silt, peat, a damp seam: the step away
    /// from the water, then the darker one beside it.
    pub silt: [Color; 2],
    /// Packed earth on a road's shoulders.
    pub shoulder: [Color; 2],
    /// The open ground's darker and lighter patches, laid over the floor
    /// at a low opacity so the tiles' own specks show through: on grass a
    /// meadow's two greens, on the karst's dust warm hardpan and pale dust.
    pub meadow_dark: Color,
    pub meadow_light: Color,
    /// How far the shore band reaches from the water, px: `.0` where it is
    /// thin and up to `.0 + .1` where it is broad.
    pub gravel_reach: (f32, f32),
    /// How far packed earth reaches from a road's dirt, px, the same way.
    pub shoulder_reach: (f32, f32),
    /// A plank deck over water (`ground::Deck`): its rails, the seams
    /// between planks, and the planks' two tones.
    pub deck: [Color; 4],
    /// Flat limestone plates across the open ground, where the look has
    /// them.
    pub plates: Option<Plates>,
    /// A floodwork apron's paving round a tower by the water
    /// (`Places::aprons`), where the look lays one.
    pub apron: Option<Apron>,
    /// Whether dry scrub grows in the shelter of a wall
    /// (`woods::Woods::undergrowth`), picture only.
    pub scrub: bool,
    /// Whether the look works the land (`Places`): a mill race from a
    /// timber building by the water, furrows on tall grass by a fence.
    pub farms: bool,
}

/// The band a look lays along its water, past the damp seam.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shore {
    /// Shelves of stones that come and go along the bank.
    Gravel,
    /// A bed of sedge: upright stems a block wide and three tall, lit at
    /// the tip, the grass showing between them.
    Sedge,
}

/// Limestone plates (`BiomeLook::plates`): pale patches of flat stone
/// with hairline fissures across them. Flat ground paint - no edge or
/// line on them may read as a ledge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plates {
    /// The stone's two faces, a broad noise choosing between them.
    pub face: [Color; 2],
    /// A fissure: a broken hairline a block wide, a step darker.
    pub fissure: Color,
}

/// The paving of a floodwork apron (`BiomeLook::apron`): flat slabs in a
/// running bond.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Apron {
    /// A slab's two tones, hashed per slab.
    pub slab: [Color; 2],
    /// The seams between slabs.
    pub seam: Color,
    /// A slab the water beside it has stained.
    pub wet: Color,
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
    shore: Shore::Gravel,
    plates: None,
    apron: None,
    scrub: false,
    farms: true,
};

/// The reclaimed floodplain (BB-198): slate water, sedge, peat, a paved
/// apron. Picked by measurement against the same rules as the valley.
pub const FLOODPLAIN: BiomeLook = BiomeLook {
    water: WaterRamp {
        edge: Color::new(0x26, 0x40, 0x4F, 255),
        dark: Color::new(0x3A, 0x5D, 0x6E, 255),
        fill: Color::new(0x4A, 0x73, 0x83, 255),
        // The submerged margin: a still lighter slate where it shallows.
        light: Color::new(0x6A, 0x93, 0xA0, 255),
        ripple: Color::new(0x9A, 0xB9, 0xBE, 255),
        glint: Color::new(0xCF, 0xE0, 0xDF, 255),
        deep: Color::new(0x33, 0x53, 0x6A, 255),
        deep_dark: Color::new(0x2A, 0x46, 0x5A, 255),
        // Weed on the bed rather than a pebble.
        bed: Color::new(0x5A, 0x7B, 0x6C, 255),
    },
    // Straw sedge: a pale lit tip, the stem, its dark foot.
    gravel: [Color::new(0xC9, 0xB2, 0x7C, 255), Color::new(0xA8, 0x92, 0x5F, 255), Color::new(0x7F, 0x6C, 0x46, 255)],
    shore: Shore::Sedge,
    // Peat.
    silt: [Color::new(0x4C, 0x3C, 0x2E, 255), Color::new(0x3A, 0x2E, 0x24, 255)],
    // Trodden mud.
    shoulder: [Color::new(0x8E, 0x77, 0x59, 255), Color::new(0x73, 0x5F, 0x47, 255)],
    // A damper meadow: the dark patches deeper than the valley's.
    meadow_dark: Color::new(0x1C, 0x4C, 0x33, 46),
    meadow_light: Color::new(0x7F, 0xB1, 0x5A, 30),
    gravel_reach: (8.0, 18.0),
    shoulder_reach: (3.0, 6.0),
    // Wet timber, darker than the valley's.
    deck: [Color::new(0x2E, 0x24, 0x1B, 255), Color::new(0x3E, 0x31, 0x25, 255), Color::new(0x7A, 0x60, 0x46, 255), Color::new(0x6A, 0x53, 0x3D, 255)],
    plates: None,
    apron: Some(Apron {
        slab: [Color::new(0xA3, 0x9E, 0x8E, 255), Color::new(0x95, 0x90, 0x80, 255)],
        seam: Color::new(0x86, 0x82, 0x74, 255),
        wet: Color::new(0x73, 0x70, 0x64, 255),
    }),
    scrub: false,
    farms: false,
};

/// The dry karst frontier (BB-198), on the desert's dust.
pub const KARST: BiomeLook = BiomeLook {
    water: WaterRamp {
        edge: Color::new(0x0F, 0x3F, 0x3E, 255),
        dark: Color::new(0x16, 0x52, 0x4F, 255),
        fill: Color::new(0x1F, 0x6A, 0x64, 255),
        light: Color::new(0x2C, 0x7D, 0x74, 255),
        ripple: Color::new(0x5F, 0xA5, 0x9A, 255),
        glint: Color::new(0xA8, 0xD6, 0xCB, 255),
        deep: Color::new(0x15, 0x4A, 0x49, 255),
        deep_dark: Color::new(0x10, 0x3D, 0x3C, 255),
        bed: Color::new(0x3E, 0x7A, 0x6A, 255),
    },
    // Loose limestone grit round a pool.
    gravel: [Color::new(0xE8, 0xD7, 0xB0, 255), Color::new(0xCD, 0xBB, 0x93, 255), Color::new(0xA8, 0x95, 0x6E, 255)],
    shore: Shore::Gravel,
    // The damp seam, drying outward.
    silt: [Color::new(0x95, 0x7A, 0x57, 255), Color::new(0x7A, 0x62, 0x45, 255)],
    // Worn dust beside a track.
    shoulder: [Color::new(0xB9, 0x9D, 0x72, 255), Color::new(0xA6, 0x8B, 0x62, 255)],
    // Warm hardpan and pale dust over the dust fill #CCB385.
    meadow_dark: Color::new(0xA0, 0x7D, 0x52, 90),
    meadow_light: Color::new(0xE2, 0xCF, 0xA4, 80),
    gravel_reach: (4.0, 10.0),
    shoulder_reach: (3.0, 7.0),
    deck: VALLEY.deck,
    plates: Some(Plates {
        face: [Color::new(0xDE, 0xD0, 0xAE, 255), Color::new(0xD6, 0xC6, 0xA2, 255)],
        fissure: Color::new(0xA8, 0x8F, 0x69, 255),
    }),
    apron: None,
    scrub: true,
    farms: false,
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
    let mut texels = lay(look, (0, 0), w, h, class);
    if look.farms {
        for &(cell, across) in &places.fields {
            lay_furrows(look, cell, across, w, h, class, &mut texels);
        }
        for &(from, to) in &places.races {
            lay_race(look, from, to, w, h, class, &mut texels);
        }
    }
    if let Some(apron) = &look.apron {
        let to_water = distance(w, h, |i| class[i].is_water());
        lay_apron(apron, &places.aprons, w, h, class, &to_water, &mut texels);
    }
    for deck in decks {
        lay_deck(look, deck, w, h, class, &mut texels);
    }
    BlockImage { width: w, height: h, block: 2, texels, stamp, patches: Vec::new() }
}

/// The materials carried past a field (`GroundGrid::beyond`, an arena's
/// margins): `bake`'s silt, gravel, shoulders, meadow and open water over
/// `w` x `h` blocks whose first lies at world block `at`, every choice
/// hashed from the world block as on the field, so the two meet without a
/// seam. No decks or places: those stand on the map's own cells.
pub fn bake_beyond(look: &BiomeLook, at: (i32, i32), w: usize, h: usize, class: &[FloorPx], stamp: u64) -> BlockImage {
    BlockImage { width: w, height: h, block: 2, texels: lay(look, at, w, h, class), stamp, patches: Vec::new() }
}

/// The materials of `w` x `h` blocks whose first is world block `at`, one
/// texel a block (`bake`).
fn lay(look: &BiomeLook, at: (i32, i32), w: usize, h: usize, class: &[FloorPx]) -> Vec<Color> {
    assert_eq!(class.len(), w * h, "a class a block");
    let to_water = distance(w, h, |i| class[i].is_water());
    let deepness = deepness(at, w, h, class);
    let to_road = distance(w, h, |i| class[i] == FloorPx::Road);
    let clear = Color::new(0, 0, 0, 0);
    let mut texels = vec![clear; w * h];
    for (i, texel) in texels.iter_mut().enumerate() {
        let (bx, by) = ((i % w) as i32 + at.0, (i / w) as i32 + at.1);
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
                } else if let Some(stone) = look.plates.as_ref().and_then(|p| plate(p, x, y, wet, dirt)) {
                    stone
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
    texels
}

/// How far into open water each block of `w` x `h` blocks from world
/// block `at` lies, 0 to 1: every deep cell (one whose middle block is `FloorPx::Deep`)
/// adds a cone `DEEP_REACH` px across its centre, full within
/// `DEEP_REACH - DEEP_FADE` px, and the sum is held to 1. A run of deep
/// cells adds up to one body with a soft, rounded edge about where the
/// cells end; a lone cell is a round pool. Only `sqrt` and the basic
/// operations, exact on every platform.
fn deepness(at: (i32, i32), w: usize, h: usize, class: &[FloorPx]) -> Vec<f32> {
    // A cell's corner is 22.6 px from the four centres round it, so a
    // reach past that keeps a lake's middle solid where cells meet; full
    // out to 16 px, a deep cell's own edge.
    const DEEP_REACH: f32 = 26.0;
    const DEEP_FADE: f32 = 10.0;
    let mut out = vec![0.0f32; w * h];
    // A cell is 16 blocks and centred on its world cell, so cell `c`'s
    // middle block is `c * 16` - here `c * 16 - at`.
    let first = |a: i32| a.div_euclid(16) + i32::from(a.rem_euclid(16) != 0);
    let (c0, r0) = (first(at.0), first(at.1));
    let reach = (DEEP_REACH / 2.0).ceil() as i32;
    for r in r0..=(at.1 + h as i32) / 16 {
        for c in c0..=(at.0 + w as i32) / 16 {
            let (cx, cy) = (c * 16 - at.0, r * 16 - at.1);
            if cx < 0 || cy < 0 || cx as usize >= w || cy as usize >= h || class[cy as usize * w + cx as usize] != FloorPx::Deep {
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
    /// The cells of a floodwork apron, in cell order: round every tower
    /// that stands by the water, and joined between two of them that guard
    /// one way in. Only a look with an `Apron` paves them.
    pub aprons: Vec<(i32, i32)>,
}

/// How near the water a tower stands for an apron, in cells.
const APRON_WATER: i32 = 2;

/// How far an apron reaches round its tower, in cells.
const APRON_REACH: i32 = 2;

/// How far apart, on one row or column, two towers by the water may stand
/// for one apron to join them, in cells.
const APRON_PAIR: i32 = 8;

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
    // Floodwork aprons: round each tower by the water, and across the way
    // two of them guard together.
    let towers: Vec<(i32, i32)> = cells.iter().filter(|(_, o)| o.tower().is_some()).map(|(c, _)| *c).filter(|&t| water.iter().any(|&wet| chebyshev(t, wet) <= APRON_WATER)).collect();
    let mut apron = BTreeSet::new();
    let mut pave = |cols: std::ops::RangeInclusive<i32>, rows: std::ops::RangeInclusive<i32>| {
        for r in rows {
            for c in cols.clone() {
                apron.insert((c, r));
            }
        }
    };
    for (k, &(c, r)) in towers.iter().enumerate() {
        pave(c - APRON_REACH..=c + APRON_REACH, r - APRON_REACH..=r + APRON_REACH);
        for &(c2, r2) in &towers[k + 1..] {
            if r2 == r && (c2 - c).abs() <= APRON_PAIR {
                pave(c.min(c2)..=c.max(c2), r - APRON_REACH..=r + APRON_REACH);
            } else if c2 == c && (r2 - r).abs() <= APRON_PAIR {
                pave(c - APRON_REACH..=c + APRON_REACH, r.min(r2)..=r.max(r2));
            }
        }
    }
    places.aprons = apron.into_iter().collect();
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

/// The shore band at field px (`x`, `y`), `wet` px from the water, if it
/// lies there: it comes and goes along a bank in a noise about a cell and
/// a half across, broad where it is high, and frays into the ground over
/// its last 8 px. Gravel is stones of three tones; sedge is upright stems
/// (`Shore::Sedge`) with the grass between.
fn gravel(look: &BiomeLook, x: f32, y: f32, wet: f32, threshold: f32) -> Option<Color> {
    if wet > look.gravel_reach.0 + look.gravel_reach.1 {
        return None;
    }
    match look.shore {
        Shore::Gravel => {
            let shelf = noise(x, y, 44.0, 7);
            let reach = look.gravel_reach.0 + look.gravel_reach.1 * shelf;
            if shelf <= 0.5 || wet > reach || (reach - wet) / 8.0 < threshold {
                return None;
            }
            let grain = noise(x, y, 6.0, 11);
            Some(look.gravel[if grain > 0.72 { 2 } else if grain > 0.3 { 0 } else { 1 }])
        }
        Shore::Sedge => {
            // Three blocks in five a stem, three blocks tall - tip, stem,
            // foot - each column's stems lifted by its own hash so the
            // tips never line up. The bed's extent is read at the stem's
            // tip, so the noise never cuts a stem in two; no dither, which
            // would.
            let (bx, by) = ((x / 2.0) as i32, (y / 2.0) as i32);
            let lifted = by + (hash(bx, 0, 31) % 3) as i32;
            if hash(bx, lifted.div_euclid(3), 29) % 5 >= 3 {
                return None;
            }
            let k = lifted.rem_euclid(3);
            let shelf = noise(x, y - k as f32 * 2.0, 44.0, 7);
            let reach = look.gravel_reach.0 + look.gravel_reach.1 * shelf;
            if shelf <= 0.5 || wet > reach {
                return None;
            }
            Some(look.gravel[k as usize])
        }
    }
}

/// A limestone plate at field px (`x`, `y`), if one lies there (`Plates`):
/// pale patches in a noise about two cells across, kept off the water's
/// grit and a track's shoulder, their rim crisp so a plate reads as a flat
/// face rather than a fade, and hairline fissures - a contour of a finer
/// noise, broken every few blocks - across them.
fn plate(p: &Plates, x: f32, y: f32, wet: f32, dirt: f32) -> Option<Color> {
    if wet < 16.0 || dirt < 8.0 || noise(x, y, 64.0, 37) < 0.64 {
        return None;
    }
    let (bx, by) = ((x / 2.0) as i32, (y / 2.0) as i32);
    let crack = noise(x, y, 20.0, 41);
    if (crack - 0.5).abs() < 0.035 && hash(bx.div_euclid(3), by.div_euclid(3), 43) % 4 != 0 {
        return Some(p.fissure);
    }
    Some(p.face[usize::from(noise(x, y, 140.0, 47) > 0.5)])
}

/// The paving of a floodwork apron over `cells` (`Places::aprons`): slabs
/// 16 x 12 px in a running bond on the field's own grid, so neighbouring
/// cells line up; slabs lifted long ago, where the grass shows - a few in
/// the middle, most at the apron's rim, so it frays into the meadow rather
/// than stopping at a cell's edge; stained by the water within 12 px of it
/// and in damp patches. Only on open ground: the water, the bank and a
/// road keep their tiles.
fn lay_apron(apron: &Apron, cells: &[(i32, i32)], w: usize, h: usize, class: &[FloorPx], to_water: &[u16], texels: &mut [Color]) {
    const SLAB: (i32, i32) = (8, 6);
    let set: std::collections::BTreeSet<(i32, i32)> = cells.iter().copied().collect();
    for &(c, r) in cells {
        // How many of the eight cells round it are apron: fewer at the rim.
        let kept = (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (c + dx, r + dy))).filter(|n| set.contains(n)).count();
        let lifted = match kept {
            9 => 8,
            7 | 8 => 4,
            _ => 2,
        };
        let (bx0, by0) = (c * 16 - 8, r * 16 - 8);
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
                let row = by.div_euclid(SLAB.1);
                let sx = bx + if row.rem_euclid(2) == 1 { SLAB.0 / 2 } else { 0 };
                let col = sx.div_euclid(SLAB.0);
                if hash(col, row, 53) % lifted == 0 {
                    continue;
                }
                let (x, y) = (bx as f32 * 2.0, by as f32 * 2.0);
                texels[i] = if sx.rem_euclid(SLAB.0) == 0 || by.rem_euclid(SLAB.1) == 0 {
                    apron.seam
                } else if to_water[i] <= 6 || noise(x, y, 30.0, 59) > 0.68 {
                    apron.wet
                } else {
                    apron.slab[(hash(col, row, 61) % 2) as usize]
                };
            }
        }
    }
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

/// What a tread mark is pressed into under a biome (`BiomeLook::floor_of`),
/// which picks its colours (`wear.rs`): the meadow's grass, a gravel
/// shelf, damp silt on a bank, or packed earth - a road's shoulder, a
/// furrow, the road's own dirt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Floor {
    Meadow,
    Gravel,
    Silt,
    Earth,
}

impl Floor {
    /// Every floor, in the order `Floor as usize` counts them.
    pub const ALL: [Floor; 4] = [Floor::Meadow, Floor::Gravel, Floor::Silt, Floor::Earth];
}

impl BiomeLook {
    /// The `Floor` a block of the materials layer (`bake`) shows, read back
    /// from its texel: clear, or a meadow patch, is the meadow; a stone,
    /// silt or packed earth is that. Water, the open-water softening and a
    /// deck's planks are not ground a mark is pressed into: `None`. Every
    /// tone of a look is its own (`a_looks_tones_are_each_its_own`), so the
    /// reading is exact.
    pub fn floor_of(&self, texel: Color) -> Option<Floor> {
        if texel.a == 0 || texel == self.meadow_dark || texel == self.meadow_light {
            Some(Floor::Meadow)
        } else if self.gravel.contains(&texel) {
            Some(Floor::Gravel)
        } else if self.silt.contains(&texel) {
            Some(Floor::Silt)
        } else if self.shoulder.contains(&texel) {
            Some(Floor::Earth)
        } else {
            None
        }
    }

    /// The minimap's texel (`minimap::Minimap`) for an open ground cell
    /// centred at field px (`x`, `y`) whose plain colour is `plain`, under
    /// this look: a cell beside water where the bank's gravel shelf is broad
    /// (the noise `gravel` reads) takes the shelf's middle stone, and the
    /// meadow takes its patches (the noise `bake` reads) at their own
    /// opacity, so the map's small picture has the field's large shapes.
    pub fn minimap_ground(&self, plain: Color, x: f32, y: f32, bank: bool) -> Color {
        if bank && noise(x, y, 44.0, 7) > 0.5 {
            return self.gravel[1];
        }
        let n = noise(x, y, 220.0, 5);
        let over = if n < 0.38 {
            self.meadow_dark
        } else if n > 0.64 {
            self.meadow_light
        } else {
            return plain;
        };
        let a = over.a as u32;
        let mix = |p: u8, o: u8| ((p as u32 * (255 - a) + o as u32 * a + 127) / 255) as u8;
        Color::new(mix(plain.r, over.r), mix(plain.g, over.g), mix(plain.b, over.b), 255)
    }
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

    /// A translucent patch tone as it lands on `biome`'s floor: the grass
    /// fill, or the desert's dust.
    fn over_floor(biome: Biome, c: Color) -> Color {
        let floor = match biome.theme() {
            Theme::Grass => Color::new(0x61, 0x95, 0x41, 255),
            Theme::Desert => Color::new(0xCC, 0xB3, 0x85, 255),
        };
        let a = c.a as f32 / 255.0;
        let mix = |g: u8, v: u8| (g as f32 * (1.0 - a) + v as f32 * a).round() as u8;
        Color::new(mix(floor.r, c.r), mix(floor.g, c.g), mix(floor.b, c.b), 255)
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
            materials.push(("dark patch", over_floor(biome, look.meadow_dark)));
            materials.push(("light patch", over_floor(biome, look.meadow_light)));
            if let Some(p) = &look.plates {
                materials.extend(p.face.iter().map(|c| ("plate", *c)));
                materials.push(("fissure", p.fissure));
            }
            if let Some(a) = &look.apron {
                materials.extend(a.slab.iter().map(|c| ("slab", *c)));
                materials.push(("seam", a.seam));
                materials.push(("wet slab", a.wet));
            }
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
    fn a_meadow_patch_stays_green_and_a_patch_of_dust_stays_earth() {
        // The de-green rule (docs/PALETTE.md): large areas stay deep green
        // or earth tan, never olive or yellow-khaki. On grass a patch keeps
        // green its strongest channel; on the desert's dust it stays a warm
        // earth, red over green by a clear step and both over blue.
        for biome in Biome::ALL {
            let look = biome.look();
            for c in [over_floor(biome, look.meadow_dark), over_floor(biome, look.meadow_light)] {
                match biome.theme() {
                    Theme::Grass => assert!(c.g > c.r && c.g > c.b && c.r < c.g - 40, "{}: meadow patch {c:?} is not green", biome.name()),
                    Theme::Desert => assert!(c.r >= c.g + 12 && c.g > c.b + 20, "{}: patch {c:?} is not earth", biome.name()),
                }
            }
        }
    }

    #[test]
    fn every_biome_has_its_own_tileset_made_from_its_theme() {
        for biome in Biome::ALL {
            let path = biome.ground_texture_path();
            assert!(std::path::Path::new(path).exists(), "{}: {path} is missing (tools/retint_ground.py)", biome.name());
            assert_ne!(path, biome.theme().ground_texture_path(), "{} draws its own sheet", biome.name());
        }
        assert_eq!(Biome::Karst.theme(), Theme::Desert, "the karst is the desert's dust");
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
        for (k, biome) in Biome::ALL.into_iter().enumerate() {
            t.biome_override = k as i32 + 1;
            assert_eq!(in_force(Some(Biome::Valley), &t), Some(biome));
        }
        let range = Tuning::SCHEMA.iter().find(|r| r.name == "biome_override").map(|r| r.max);
        assert_eq!(range, Some(Biome::ALL.len() as f64), "the knob reaches every biome");
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
    fn a_sedge_bed_stands_in_stems_with_the_grass_between() {
        let image = bake(&FLOODPLAIN, 128, 128, &tiny_blocks(), &[], &Places::default(), 1);
        let [tip, stem, foot] = FLOODPLAIN.gravel;
        let at = |x: usize, y: usize| image.texels[y * 128 + x];
        let mut tips = 0;
        for y in 0..126 {
            for x in 0..128 {
                if at(x, y) == tip {
                    tips += 1;
                    assert_eq!(at(x, y + 1), stem, "a stem at block ({x}, {y}) stands on its tip");
                }
            }
        }
        assert!(tips > 30, "the banks grow sedge: {tips} stems");
        let band = image.texels.iter().filter(|t| [tip, stem, foot].contains(t)).count();
        assert!(band < 128 * 128 / 10, "a bed, not a mat: {band} blocks");
    }

    #[test]
    fn the_karst_lays_flat_plates_away_from_the_water_and_none_on_the_valley() {
        let image = bake(&KARST, 128, 128, &tiny_blocks(), &[], &Places::default(), 1);
        let p = KARST.plates.unwrap();
        let plate = |c: &Color| p.face.contains(c) || *c == p.fissure;
        let stones: Vec<usize> = (0..image.texels.len()).filter(|&i| plate(&image.texels[i])).collect();
        assert!(stones.len() > 300, "plates on the open ground: {}", stones.len());
        assert!(stones.iter().any(|&i| image.texels[i] == p.fissure), "fissures cross them");
        for &i in &stones {
            // Water down x 96..160 px and its bank to 166: no plate within
            // 16 px of the water, 8 blocks.
            let x = (i % 128) as i32 * 2;
            assert!(!(82..174).contains(&x), "a plate at x {x} px sits in the water's grit");
        }
        let valley = bake(&VALLEY, 128, 128, &tiny_blocks(), &[], &Places::default(), 1);
        assert!(!valley.texels.iter().any(plate), "the valley has no plates");
    }

    #[test]
    fn lotus_lagoons_towers_by_the_water_get_one_apron_between_them() {
        let map = MapFile::load(std::path::Path::new("maps/lotus-lagoon.toml")).unwrap();
        let places = places(&map);
        // The two tesla towers at (21, 13) and (27, 13) guard the way in.
        for cell in [(21, 13), (24, 13), (27, 13), (19, 11), (29, 15)] {
            assert!(places.aprons.contains(&cell), "{cell:?} is paved");
        }
        assert!(!places.aprons.contains(&(18, 13)) && !places.aprons.contains(&(24, 16)), "and no further");
        assert!(places.aprons.windows(2).all(|w| w[0] < w[1]), "in cell order, once each");
        // A look with no apron paves nothing.
        let none = bake(&VALLEY, 64, 64, &vec![FloorPx::Land; 64 * 64], &[], &Places { aprons: vec![(1, 1)], ..Places::default() }, 1);
        let paved = bake(&FLOODPLAIN, 64, 64, &vec![FloorPx::Land; 64 * 64], &[], &Places { aprons: vec![(1, 1)], ..Places::default() }, 1);
        let slab = |c: &Color| FLOODPLAIN.apron.unwrap().slab.contains(c);
        assert!(!none.texels.iter().any(slab));
        assert!(paved.texels.iter().any(slab));
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

    /// A look's ground tones are each its own, so a texel of its layer
    /// reads back as the one material it is (`BiomeLook::floor_of`).
    #[test]
    fn a_looks_tones_are_each_its_own() {
        for biome in Biome::ALL {
            let look = biome.look();
            let w = look.water;
            let mut tones: Vec<Color> = vec![w.edge, w.dark, w.fill, w.light, w.ripple, w.glint, w.deep, w.deep_dark, w.bed, look.meadow_dark, look.meadow_light];
            tones.extend(look.gravel.iter().chain(&look.silt).chain(&look.shoulder).chain(&look.deck));
            for (i, a) in tones.iter().enumerate() {
                assert!(!tones[i + 1..].contains(a), "{}: {a:?} plays two roles", biome.name());
            }
            assert_eq!(look.floor_of(look.gravel[2]), Some(Floor::Gravel));
            assert_eq!(look.floor_of(look.silt[1]), Some(Floor::Silt));
            assert_eq!(look.floor_of(look.shoulder[0]), Some(Floor::Earth));
            assert_eq!(look.floor_of(look.meadow_light), Some(Floor::Meadow));
            assert_eq!(look.floor_of(Color::new(0, 0, 0, 0)), Some(Floor::Meadow));
            assert_eq!(look.floor_of(w.deep), None);
            assert_eq!(look.floor_of(look.deck[2]), None);
        }
    }

    /// The layer carried past a field (`bake_beyond`) is the field's own
    /// wherever it reads the same tiles: every choice is keyed by the world
    /// block, so the two meet without a seam.
    #[test]
    fn the_layer_beyond_a_field_is_the_fields_where_they_meet() {
        let (w, h) = (128, 128);
        let class = tiny_blocks();
        let field = bake(&VALLEY, w, h, &class, &[], &Places::default(), 1);
        // The same blocks laid from world block (-20, -12), with a strip
        // of plain ground before them: far enough from the field's blocks
        // that no distance a block reads changes.
        let (bw, bh) = (w + 20, h + 12);
        let mut wide = vec![FloorPx::Land; bw * bh];
        for y in 0..h {
            wide[(y + 12) * bw + 20..(y + 12) * bw + 20 + w].copy_from_slice(&class[y * w..(y + 1) * w]);
        }
        let beyond = bake_beyond(&VALLEY, (-20, -12), bw, bh, &wide, 2);
        for y in 0..h {
            for x in 0..w {
                assert_eq!(beyond.texels[(y + 12) * bw + x + 20], field.texels[y * w + x], "block ({x}, {y})");
            }
        }
    }

    /// The minimap's open ground under the valley: a broad shelf's stone
    /// on a bank, the meadow's patches elsewhere, plain ground between.
    #[test]
    fn the_minimap_ground_takes_the_shelves_and_the_patches() {
        let plain = Color::new(0x5F, 0x91, 0x4B, 255);
        let cells: Vec<(f32, f32)> = (0..60).flat_map(|r| (0..60).map(move |c| (c as f32 * 32.0, r as f32 * 32.0))).collect();
        let banks: Vec<Color> = cells.iter().map(|&(x, y)| VALLEY.minimap_ground(plain, x, y, true)).collect();
        assert!(banks.contains(&VALLEY.gravel[1]) && banks.iter().any(|c| *c != VALLEY.gravel[1]), "a shelf comes and goes along a bank");
        let open: Vec<Color> = cells.iter().map(|&(x, y)| VALLEY.minimap_ground(plain, x, y, false)).collect();
        assert!(!open.contains(&VALLEY.gravel[1]), "no stone away from the water");
        let kinds: std::collections::BTreeSet<(u8, u8, u8)> = open.iter().map(|c| (c.r, c.g, c.b)).collect();
        assert_eq!(kinds.len(), 3, "plain ground and two patches: {kinds:?}");
        assert!(open.iter().all(|c| c.g > c.r && c.g > c.b), "every patch stays green");
    }

    #[test]
    fn a_biome_round_trips_through_its_name() {
        for biome in Biome::ALL {
            assert_eq!(Biome::parse(biome.name()), Some(biome));
        }
        assert_eq!(Biome::parse("lagoon"), None);
    }
}
