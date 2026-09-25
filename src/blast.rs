//! The barrel blast's presentation: the one-shot fireball sprite animation,
//! the additive light bloom under it, the pulsing glow on a barrel whose
//! fuse is lit, and the scorch decal a blast leaves on the ground. Pure
//! drawing over `static/barrel_explosion.png` (docs/PROPS_SPEC.md); the
//! simulation (`simulation::props`) only pushes the `BlastFx`/`Scorch`
//! records, and never draws RNG for them - every seed is a hash of the
//! blast position, so a purely cosmetic field can't shift a seeded replay.
//! The records and the scorch (painted over `Canvas`) are here; the
//! fireball, the ground fire and the glows draw with raylib in
//! `render::blast`.
//!
//! No two blasts look alike on purpose: the sheet holds four fireball
//! shapes, each blast picks one plus a mirror, a quarter-turn, a playback
//! rate and a size from its hash, and the simulation shapes it further by
//! what set it off (`BlastShape`) - a shot leans the fire downrange, a ram
//! goes up as a column, a chained drum leans away from the blast that lit
//! it and grows with how long it smouldered.

use crate::canvas::{Canvas, Sheet};
use crate::mushroom::Cloud;
use crate::tuning::tuning;
use crate::math::{Color, Rectangle, Vec2};

use crate::{
    BARREL_EXPLOSION_FRAMES,
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

/// One in-flight barrel detonation sprite, oldest first in `Game::blast_fx`.
pub struct BlastFx {
    pub center: Position,
    /// Seconds since it went off.
    pub time: f32,
    pub seed: u32,
    /// Multiplier on `blast_anim_scale` and the glow radius - 1.0 for a
    /// barrel or a dying tank, smaller for a cook-off secondary.
    pub scale: f32,
    /// A cook-off pop rather than a kill or a barrel: local only, so it
    /// gets no screen-level effect (see `Game::tick_cookoffs`).
    pub secondary: bool,
    /// Sheet row of the fireball shape (`BLAST_SHAPE_ROWS`).
    pub row: i32,
    /// Quarter-turns applied to the fire frames (the smoke frames keep
    /// their rise, so they only take the mirror).
    pub turn: i32,
    /// Multiplier on `blast_anim_fps`, hashed per blast.
    pub fps_scale: f32,
    /// Draw offset from `center` (px): the lean a cause gives the fire.
    pub offset: Lean,
    pub kind: BlastKind,
    /// A dying tank's mushroom cloud, composed at draw time
    /// (`mushroom.rs`), in place of the sheet's fireball.
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

    /// Playback rate for this blast: the knob times its hashed jitter,
    /// and a fuel drum's fire is a little quicker.
    pub fn fps(&self) -> f32 {
        let base = tuning().blast_anim_fps * self.fps_scale;
        (if self.kind == BlastKind::Fuel { base * 1.15 } else { base }).max(1.0)
    }

    /// The frame to show now, clamped so a live change to `blast_anim_fps`
    /// can never index past the sheet.
    pub fn frame(&self) -> i32 {
        ((self.time * self.fps()) as i32).clamp(0, BARREL_EXPLOSION_FRAMES - 1)
    }

    pub fn done(&self) -> bool {
        if let Some(cloud) = &self.cloud {
            return cloud.done(self.time);
        }
        self.time >= BARREL_EXPLOSION_FRAMES as f32 / self.fps()
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
}

impl Scorch {
    pub fn new(center: Position) -> Self {
        Scorch { center, seed: seed_for(center), age: 0.0, scale: 1.0, streak: None }
    }

    pub fn with(center: Position, scale: f32, streak: Option<Lean>) -> Self {
        Scorch { center, seed: seed_for(center), age: 0.0, scale, streak }
    }
}

/// The scorch decal, variant/mirror/quarter-turn picked by the seed (90
/// degree steps keep the pixels crisp), fading in over
/// `scorch_fade_in_seconds` so it appears under the fireball, not before.
/// A shot's scorch also gets the streak cell, turned to the nearest
/// quarter toward the shot.
pub fn draw_scorch(c: &mut impl Canvas, s: &Scorch) {
    let cell = BARREL_EXPLOSION_TEXTURE_SIZE;
    let variant = (s.seed % SCORCH_VARIANTS as u32) as f32;
    let flip = if s.seed & 4 != 0 { -1.0 } else { 1.0 };
    let src = Rectangle::new(variant * cell, SCORCH_ROW as f32 * cell, cell * flip, cell);
    let size = cell * tuning().scorch_scale * s.scale;
    let rotation = ((s.seed >> 3) % 4) as f32 * 90.0;
    let fade = (s.age / tuning().scorch_fade_in_seconds.max(1e-3)).clamp(0.0, 1.0);
    let tint = Color::new(255, 255, 255, (255.0 * tuning().scorch_opacity * fade) as u8);
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
