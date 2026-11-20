//! The blast's records - `BlastFx`, one explosion playing, and `Scorch`,
//! the mark it leaves - and the scorch decal, painted over `Canvas` from
//! `static/barrel_explosion.png` (docs/PROPS_SPEC.md). The explosion itself
//! is composed at draw time in the effects language: `fireball.rs` for
//! every blast, `mushroom.rs` for the share of kills that go up as a
//! cloud (docs/effects.md). The simulation (`simulation::props`) only
//! pushes the records, and never draws RNG for them - every seed is a hash
//! of the blast position, so a purely cosmetic field can't shift a seeded
//! replay.
//!
//! No two blasts look alike on purpose: each blast picks one of four
//! shapes (`BLAST_SHAPE_ROWS`), a pace and a size from its hash, and the
//! simulation shapes it further by what set it off (`BlastShape`) - a shot
//! throws the fire downrange, a ram goes up as a column, a chained drum
//! leans away from the blast that lit it and grows with how long it
//! smouldered.

use crate::canvas::{Canvas, Sheet};
use crate::mushroom::Cloud;
use crate::tuning::tuning;
use crate::math::{Color, Rectangle, Vec2};

use crate::{
    BARREL_EXPLOSION_TEXTURE_SIZE,
    BLAST_ROW_DOUBLE,
    BLAST_ROW_FLAT,
    BLAST_ROW_MUSHROOM,
    BLAST_ROW_TALL,
    BLAST_SHAPE_ROWS,
    Position,
    SCORCH_ROW,
    SCORCH_STREAK_COL,
    SCORCH_VARIANTS,
};

/// A blast a seat owns that is about to go off (`Game::blast_rings`):
/// where, how far it reaches, and the seconds left before it does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlastRing {
    pub at: Position,
    pub radius: f32,
    pub left: f32,
}

/// The ring a blast that is about to go off draws on the ground at its
/// reach (docs/grenade-launcher.md "The ring"): one block wide, in the
/// grenade band's flash ramp (`grenade::BAND_LIT`), coming in through the
/// field's Bayer pattern over the first third of `blast_ring_seconds` and
/// burning from red to white as `left` runs out. Everything inside it will
/// be hurt, whichever side it is on.
pub fn draw_blast_ring(c: &mut impl Canvas, ring: &BlastRing, t: &crate::tuning::Tuning) {
    let window = t.blast_ring_seconds;
    if window <= 0.0 || ring.radius <= 0.0 {
        return;
    }
    let shown = ((window - ring.left) / window).clamp(0.0, 1.0);
    let cover = (shown * 3.0).clamp(0.0, 1.0);
    let color = crate::pyro::step(&crate::grenade::BAND_LIT, 0.2 + 0.8 * shown);
    crate::pyro::block_arc(c, ring.at, ring.radius, crate::pyro::BLOCK, 0.0, std::f32::consts::TAU, color, cover.max(0.25));
}

/// A cosmetic seed for the blast at `center`: picks flips, rotations and
/// the scorch variant. Hashed from the position rather than rolled.
pub fn seed_for(center: Position) -> u32 {
    seed_at(center, 0)
}

/// `seed_for` with a salt, so several cosmetic choices made at one
/// position get independent seeds without any of them drawing RNG - a
/// dying tile picks its rubble variant, its mirror and its quarter-turn
/// from three different salts at the same centre.
pub fn seed_at(center: Position, salt: u32) -> u32 {
    let h = (center.x as i32 as u32)
        .wrapping_mul(73_856_093)
        .wrapping_add(salt.wrapping_mul(83_492_791))
        .wrapping_add(1)
        ^ (center.y as i32 as u32).wrapping_mul(19_349_663);
    avalanche(h)
}

/// A cosmetic value in `0.0..1.0` for choice `k` of whatever `seed`
/// belongs to: independent per `k`, no RNG drawn.
pub fn hash_unit(seed: u32, k: u32) -> f32 {
    (avalanche(seed ^ k.wrapping_mul(0x9e37_79b9)) >> 8) as f32 / (1u32 << 24) as f32
}

/// Spread a hash's entropy over all 32 bits. Everything here sits on a
/// 32px grid, so both coordinates are multiples of 32 and the products
/// above have five zero low bits - which is exactly where callers look
/// when they take `seed & 1` for a mirror or `seed % N` for a variant.
/// Without this every grid-aligned decal in a round picked the same
/// mirror, the same quarter-turn and the same cell.
fn avalanche(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h
}

/// `seed_at` salt for the wreck's mushroom-cloud pick, clear of the
/// other salts hashed at a kill position (parts, rubble, particles).
const WRECK_MUSHROOM_SALT: u32 = 211;

/// A unit-ish direction in the plane, for the cosmetic lean of a blast.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lean {
    pub x: f32,
    pub y: f32,
}

/// What set a blast off, as far as its look is concerned. Decided in the
/// simulation from data it already holds at the moment the drum dies, so
/// no RNG is drawn for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlastShape {
    /// A shot: the fire leans `dir` (the projectile's travel), and the
    /// scorch gets a streak pointing downrange.
    Shot { dir: Lean },
    /// A tank drove into it: a tall column, and the hull lurches.
    Ram,
    /// Another blast's fuse: leans away from `from`; `smoulder` (0..1)
    /// is how long the fuse burned relative to the shortest one, and the
    /// fireball grows with it.
    Chained { from: Position, smoulder: f32 },
    /// A burning cell reached it: a column, like a ram.
    Fire,
    /// No cause recorded - the plain hashed pick.
    Plain,
}

/// Which drum went off, for the numbers the presentation reads (size,
/// pace, bloom colour). Mirrors `obstacle::Drum` without importing it,
/// so a wreck's fireball (`Oil`, the reference look) needs no drum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlastKind {
    Oil,
    Fuel,
}

/// One explosion playing, oldest first in `Game::blast_fx`.
pub struct BlastFx {
    pub center: Position,
    /// Seconds since it went off.
    pub time: f32,
    pub seed: u32,
    /// Multiplier on `blast_fireball_px` and the glow radius - 1.0 for a
    /// barrel or a dying tank, smaller for a cook-off secondary.
    pub scale: f32,
    /// A cook-off pop rather than a kill or a barrel: local only, so it
    /// gets no screen-level effect (see `Game::tick_cookoffs`).
    pub secondary: bool,
    /// The fireball's shape (`BLAST_SHAPE_ROWS`: round, a column, a flat
    /// burst thrown downrange, a double core; `fireball::compose`).
    pub row: i32,
    /// Quarter-turns of the fireball's hashed pattern: which way its puffs
    /// and flash rays are laid out.
    pub turn: i32,
    /// How fast this blast plays, hashed per blast: its life is
    /// `fireball::seconds` divided by it.
    pub fps_scale: f32,
    /// Draw offset from `center` (px): the lean a cause gives the fire.
    pub offset: Lean,
    pub kind: BlastKind,
    /// A dying tank's mushroom cloud, composed at draw time
    /// (`mushroom.rs`), in place of the fireball.
    pub cloud: Option<Cloud>,
}

impl BlastFx {
    /// The reference fireball at `center`: the hashed shape, mirror,
    /// turn and jitter with no cause-driven lean.
    pub fn new(center: Position) -> Self {
        Self::shaped(center, BlastKind::Oil, BlastShape::Plain)
    }

    /// A dying tank's fireball: the mushroom cloud in
    /// `wreck_mushroom_chance` of kills, the reference fireball otherwise.
    pub fn wreck(center: Position) -> Self {
        Self::wreck_with(center, tuning().wreck_mushroom_chance)
    }

    /// `wreck` with the chance passed in. The pick is a salted position
    /// hash, independent of the one that picks the plain row, so no RNG
    /// is drawn; the cloud's shape hashes from the blast's seed.
    pub fn wreck_with(center: Position, mushroom_chance: f32) -> Self {
        let mut fx = Self::new(center);
        let roll = (seed_at(center, WRECK_MUSHROOM_SALT) % 10_000) as f32 / 10_000.0;
        if roll < mushroom_chance {
            fx.cloud = Some(Cloud::new(fx.seed, fx.scale));
        }
        fx
    }

    /// A barrel's fireball, shaped by what set it off.
    pub fn shaped(center: Position, kind: BlastKind, shape: BlastShape) -> Self {
        let t = tuning();
        let seed = seed_for(center);
        let hashed = |shift: u32| ((seed >> shift) % 100) as f32 / 100.0;
        let mut row = BLAST_SHAPE_ROWS[((seed >> 8) % BLAST_SHAPE_ROWS.len() as u32) as usize];
        let turn = ((seed >> 3) % 4) as i32;
        let fps_scale = 1.0 - t.barrel_fps_jitter + 2.0 * t.barrel_fps_jitter * hashed(12);
        let mut scale = 1.0 - t.barrel_scale_jitter + 2.0 * t.barrel_scale_jitter * hashed(20);
        let mut offset = Lean { x: 0.0, y: 0.0 };
        match shape {
            BlastShape::Shot { dir } => {
                // Flat, or now and then the double core; leaning downrange.
                row = if (seed >> 8) % 3 == 0 { BLAST_ROW_DOUBLE } else { BLAST_ROW_FLAT };
                offset = Lean { x: (dir.x * 10.0 / 2.0).round() * 2.0, y: (dir.y * 10.0 / 2.0).round() * 2.0 };
            }
            BlastShape::Ram | BlastShape::Fire => row = BLAST_ROW_TALL,
            BlastShape::Chained { from, smoulder } => {
                let dx = center.x - from.x;
                let dy = center.y - from.y;
                let d = (dx * dx + dy * dy).sqrt();
                if d > 0.001 {
                    offset = Lean { x: (dx / d * 6.0 / 2.0).round() * 2.0, y: (dy / d * 6.0 / 2.0).round() * 2.0 };
                }
                scale *= 0.9 + 0.35 * smoulder.clamp(0.0, 1.0);
            }
            BlastShape::Plain => {}
        }
        if kind == BlastKind::Fuel {
            // Bigger, faster and always the column: a fuel drum goes up.
            row = BLAST_ROW_TALL;
            scale *= 1.15;
        }
        BlastFx { center, time: 0.0, seed, scale, secondary: false, row, turn, fps_scale, offset, kind, cloud: None }
    }

    /// A scaled-down fireball for a wreck's ammo cooking off: the same
    /// animation, drawn smaller so a secondary reads as a pop rather than
    /// as a second tank dying.
    pub fn small(center: Position) -> Self {
        BlastFx {
            center,
            time: 0.0,
            seed: seed_for(center),
            scale: tuning().cookoff_blast_scale,
            secondary: true,
            row: BLAST_ROW_MUSHROOM,
            turn: 0,
            fps_scale: 1.0,
            offset: Lean { x: 0.0, y: 0.0 },
            kind: BlastKind::Oil,
            cloud: None,
        }
    }

    /// Has its last smoke gone? The mushroom cloud's own life, or the
    /// fireball's (`fireball::seconds`).
    pub fn done(&self) -> bool {
        if let Some(cloud) = &self.cloud {
            return cloud.done(self.time);
        }
        crate::fireball::done(self)
    }

}

/// A burn mark left where a barrel went off. Never removed during a round
/// (`Game::init` clears them, `SCORCH_MAX` caps them); `age` only drives
/// the fade-in under the fireball.
pub struct Scorch {
    pub center: Position,
    pub seed: u32,
    pub age: f32,
    /// Multiplier on `scorch_scale`: a fuel drum's mark is bigger.
    pub scale: f32,
    /// A streak pointing downrange when a shot set the drum off (drawn
    /// on top of the blot, quarter-turned toward `dir`).
    pub streak: Option<Lean>,
    /// The scar a ground fire left: an ember bed, glowing for
    /// `ember_bed_seconds` of its `age` before it is only the mark
    /// (`smoulder.rs`).
    pub ember: bool,
}

impl Scorch {
    pub fn new(center: Position) -> Self {
        Scorch { center, seed: seed_for(center), age: 0.0, scale: 1.0, streak: None, ember: false }
    }

    pub fn with(center: Position, scale: f32, streak: Option<Lean>) -> Self {
        Scorch { center, seed: seed_for(center), age: 0.0, scale, streak, ember: false }
    }

    /// The scar a ground fire leaves where it burnt out: a smaller mark,
    /// its variant and turn hashed apart from a blast's at the same spot,
    /// so a burnt trail reads as one scorched streak rather than a row of
    /// the same blot.
    pub fn burn(center: Position) -> Self {
        Scorch { center, seed: seed_at(center, 57), age: 0.0, scale: 0.6, streak: None, ember: true }
    }
}

/// A sprite scale of about `k` at which every texel is the same whole
/// number of pixels, or every drawn pixel the same whole number of texels:
/// the nearest whole number from 1 up, the nearest whole fraction under
/// it. A scorch drawn at 1.2 texels a pixel would show some of its texels
/// two pixels wide and some one.
pub fn texel_scale(k: f32) -> f32 {
    if k >= 1.0 {
        k.round()
    } else {
        1.0 / (1.0 / k.max(1.0 / 8.0)).round()
    }
}

/// Burning ground cells that touch, as one fire: what one daylight glow
/// is drawn for, so a lit pool reads as a pool of fire and not as a
/// grid of discs.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FireGroup {
    /// The middle of its cells.
    pub centre: Position,
    /// How far its furthest cell's centre lies from `centre`, px.
    pub reach: f32,
    /// How many cells burn in it.
    pub cells: usize,
    /// The longest any of its cells has left to burn, seconds.
    pub left: f32,
    /// The largest share of its burn any of its cells has left, 0 to 1.
    pub share: f32,
}

/// The fires among burning `cells` (each its centre, the seconds it has
/// left and its whole burn, `Game::burning_cells`): cells joined edge to
/// edge on the obstacle grid are one. In the order of each fire's first
/// cell, top row first, whatever order the cells came in.
pub fn fire_groups(cells: &[(Position, f32, f32)]) -> Vec<FireGroup> {
    use std::collections::BTreeMap;
    let grid = crate::OBSTACLE_GRID_SIZE;
    let key = |at: Position| ((at.y / grid).round() as i32, (at.x / grid).round() as i32);
    let mut open: BTreeMap<(i32, i32), (Position, f32, f32)> = BTreeMap::new();
    for &(at, left, total) in cells {
        if left > 0.0 {
            open.insert(key(at), (at, left, total));
        }
    }
    let mut out = Vec::new();
    while let Some((&first, _)) = open.iter().next() {
        let mut members: Vec<(Position, f32, f32)> = Vec::new();
        let mut stack = vec![first];
        while let Some(k) = stack.pop() {
            let Some(cell) = open.remove(&k) else { continue };
            members.push(cell);
            stack.extend([(k.0 - 1, k.1), (k.0 + 1, k.1), (k.0, k.1 - 1), (k.0, k.1 + 1)].into_iter().filter(|n| open.contains_key(n)));
        }
        let n = members.len() as f32;
        let centre = Position::new(members.iter().map(|m| m.0.x).sum::<f32>() / n, members.iter().map(|m| m.0.y).sum::<f32>() / n);
        out.push(FireGroup {
            centre,
            reach: members.iter().map(|m| m.0.distance_to(centre)).fold(0.0, f32::max),
            cells: members.len(),
            left: members.iter().map(|m| m.1).fold(0.0, f32::max),
            share: members.iter().map(|m| (m.1 / m.2.max(0.01)).clamp(0.0, 1.0)).fold(0.0, f32::max),
        });
    }
    out
}

/// The scorch decal, variant/mirror/quarter-turn picked by the seed (90
/// degree steps keep the pixels crisp), at a whole-texel size
/// (`texel_scale`), stepping in over
/// `scorch_fade_in_seconds` so it appears under the fireball, not before.
/// A shot's scorch also gets the streak cell, turned to the nearest
/// quarter toward the shot.
pub fn draw_scorch(c: &mut impl Canvas, s: &Scorch) {
    draw_scorch_kept(c, s, 1.0);
}

/// `draw_scorch` with `keep` (0 to 1) of its strength, in eighths: what
/// snow lying over it leaves showing (docs/aftermath.md).
pub fn draw_scorch_kept(c: &mut impl Canvas, s: &Scorch, keep: f32) {
    let cell = BARREL_EXPLOSION_TEXTURE_SIZE;
    let variant = (s.seed % SCORCH_VARIANTS as u32) as f32;
    let flip = if s.seed & 4 != 0 { -1.0 } else { 1.0 };
    let src = Rectangle::new(variant * cell, SCORCH_ROW as f32 * cell, cell * flip, cell);
    let size = cell * texel_scale(tuning().scorch_scale * s.scale);
    let rotation = ((s.seed >> 3) % 4) as f32 * 90.0;
    let fade = crate::pyro::eighths(s.age / tuning().scorch_fade_in_seconds.max(1e-3));
    let keep = crate::pyro::eighths(keep);
    if keep <= 0.0 {
        return;
    }
    let tint = Color::new(255, 255, 255, (255.0 * tuning().scorch_opacity * fade * keep) as u8);
    let dest = Rectangle::new(s.center.x, s.center.y, size, size);
    c.blit(Sheet::BarrelExplosion, src, dest, Vec2::new(size / 2.0, size / 2.0), rotation, tint);
    if let Some(dir) = s.streak {
        // The streak cell points right; turn it to the quarter nearest
        // the shot's travel.
        let angle = dir.y.atan2(dir.x).to_degrees();
        let quarter = ((angle / 90.0).round() * 90.0).rem_euclid(360.0);
        let src = Rectangle::new(SCORCH_STREAK_COL as f32 * cell, SCORCH_ROW as f32 * cell, cell, cell);
        c.blit(Sheet::BarrelExplosion, src, dest, Vec2::new(size / 2.0, size / 2.0), quarter, tint);
    }
}

#[cfg(test)]
mod conformance_tests {

    /// The ring lies on the blast's reach, one block wide, nothing inside
    /// or outside it, and is drawn only within its window.
    #[test]
    fn the_ring_is_drawn_on_the_reach_and_only_in_its_window() {
        let t = crate::tuning::Tuning::DEFAULT;
        let ring = BlastRing { at: Position::new(100.0, 100.0), radius: 40.0, left: 0.0 };
        // On black: the blank canvas is white, as is the ring's last step.
        let black = || {
            let mut c = crate::canvas::CpuCanvas::blank(200, 200);
            Canvas::fill_rect(&mut c, 0, 0, 200, 200, Color::BLACK);
            c
        };
        let mut c = black();
        draw_blast_ring(&mut c, &ring, &t);
        let clear = black();
        let lit = |c: &crate::canvas::CpuCanvas, x: usize, y: usize| c.pixel(x, y) != clear.pixel(x, y);
        assert!(lit(&c, 140, 100) || lit(&c, 139, 100), "on the reach to the east");
        assert!(lit(&c, 100, 60) || lit(&c, 100, 59), "and to the north");
        assert!(!lit(&c, 100, 100) && !lit(&c, 120, 100), "nothing inside it");
        assert!(!lit(&c, 150, 100), "nor outside");
        let mut none = black();
        draw_blast_ring(&mut none, &ring, &crate::tuning::Tuning { blast_ring_seconds: 0.0, ..t });
        assert!(none.pixels() == clear.pixels(), "0 seconds draws none");
    }

    use super::*;

    #[test]
    fn a_scorch_is_drawn_at_whole_texels() {
        assert_eq!(texel_scale(2.0), 2.0);
        assert_eq!(texel_scale(1.2), 1.0, "a burn's 0.6 of the default 2 is one pixel a texel");
        assert_eq!(texel_scale(1.6), 2.0);
        assert_eq!(texel_scale(3.4), 3.0);
        assert_eq!(texel_scale(0.6), 0.5, "under 1 it is a whole number of texels a pixel");
        assert_eq!(texel_scale(0.3), 1.0 / 3.0);
        assert!(texel_scale(0.0) > 0.0, "and never nothing");
        for k in 1..80 {
            let s = texel_scale(k as f32 * 0.05);
            assert!(s.fract() == 0.0 || (1.0 / s - (1.0 / s).round()).abs() < 1e-5, "{k}: {s}");
        }
    }

    fn cell(c: i32, r: i32, left: f32) -> (Position, f32, f32) {
        (Position::new(c as f32 * crate::OBSTACLE_GRID_SIZE, r as f32 * crate::OBSTACLE_GRID_SIZE), left, 4.0)
    }

    #[test]
    fn burning_cells_that_touch_are_one_fire() {
        // An L of three, a cell touching it only by a corner, and one far
        // off; a burnt-out cell between two that would join them.
        let cells = [cell(3, 3, 2.0), cell(4, 3, 4.0), cell(4, 4, 1.0), cell(5, 5, 3.0), cell(10, 3, 0.5), cell(11, 3, 0.0), cell(12, 3, 2.0)];
        let fires = fire_groups(&cells);
        assert_eq!(fires.iter().map(|f| f.cells).collect::<Vec<_>>(), [3, 1, 1, 1], "top row first, the corner cell its own");
        let l = fires[0];
        let g = crate::OBSTACLE_GRID_SIZE;
        assert!((l.centre.x - 11.0 / 3.0 * g).abs() < 1e-3 && (l.centre.y - 10.0 / 3.0 * g).abs() < 1e-3, "{:?}", l.centre);
        assert!(l.reach > 0.5 * g && l.reach < g, "{}", l.reach);
        assert_eq!((l.left, l.share), (4.0, 1.0), "as long as its longest cell");
        assert_eq!(fires[3].reach, 0.0);
        // Whatever order they come in.
        let mut shuffled = cells;
        shuffled.reverse();
        assert_eq!(fire_groups(&shuffled), fires);
        assert!(fire_groups(&[]).is_empty());
    }
}
