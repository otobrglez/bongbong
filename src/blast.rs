//! The barrel blast's presentation: the one-shot fireball sprite animation,
//! the additive light bloom under it, the pulsing glow on a barrel whose
//! fuse is lit, and the scorch decal a blast leaves on the ground. Pure
//! drawing over `static/barrel_explosion.png` (docs/PROPS_SPEC.md); the
//! simulation (`simulation::props`) only pushes the `BlastFx`/`Scorch`
//! records, and never draws RNG for them - every seed is a hash of the
//! blast position, so a purely cosmetic field can't shift a seeded replay.
//!
//! No two blasts look alike on purpose: the sheet holds four fireball
//! shapes, each blast picks one plus a mirror, a quarter-turn, a playback
//! rate and a size from its hash, and the simulation shapes it further by
//! what set it off (`BlastShape`) - a shot leans the fire downrange, a ram
//! goes up as a column, a chained drum leans away from the blast that lit
//! it and grows with how long it smouldered.

use crate::tuning::tuning;
use sola_raylib::prelude::*;

use crate::{
    BARREL_EXPLOSION_FRAMES,
    BARREL_EXPLOSION_TEXTURE_SIZE,
    BLAST_ROW_DOUBLE,
    BLAST_ROW_FLAT,
    BLAST_ROW_MUSHROOM,
    BLAST_ROW_TALL,
    BLAST_SHAPE_ROWS,
    FIRE_LOOP_COL,
    FIRE_LOOP_FRAMES,
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

/// A unit-ish direction in the plane, for the cosmetic lean of a blast.
/// Plain fields rather than a `Vector2` so the simulation can hand one
/// over without a drawing type in its signature.
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
}

impl BlastFx {
    /// The reference fireball at `center`: the hashed shape, mirror,
    /// turn and jitter with no cause-driven lean. What a dying tank uses.
    pub fn new(center: Position) -> Self {
        Self::shaped(center, BlastKind::Oil, BlastShape::Plain)
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
        BlastFx { center, time: 0.0, seed, scale, secondary: false, row, turn, fps_scale, offset, kind }
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
        self.time >= BARREL_EXPLOSION_FRAMES as f32 / self.fps()
    }

    /// Where the sprite is drawn: the centre plus the cause's lean.
    fn draw_center(&self) -> Position {
        Position::new(self.center.x + self.offset.x, self.center.y + self.offset.y)
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

/// The fireball frame, drawn centred on the blast; mirrored per seed so
/// two chained blasts don't look cloned, and quarter-turned while the fire
/// is still a fire - the smoke frames rise, so those keep their way up.
pub fn draw_blast(d: &mut impl RaylibDraw, texture: &Texture2D, b: &BlastFx) {
    let cell = BARREL_EXPLOSION_TEXTURE_SIZE;
    let flip = if b.seed & 1 != 0 { -1.0 } else { 1.0 };
    let frame = b.frame();
    let src = Rectangle::new(frame as f32 * cell, b.row as f32 * cell, cell * flip, cell);
    let size = cell * tuning().blast_anim_scale * b.scale;
    let at = b.draw_center();
    let dest = Rectangle::new(at.x, at.y, size, size);
    let rotation = if frame <= 3 { b.turn as f32 * 90.0 } else { 0.0 };
    d.draw_texture_pro(texture, src, dest, Vector2::new(size / 2.0, size / 2.0), rotation, Color::WHITE);
}

/// The light bloom under a fresh fireball - two flat discs that read as a
/// flash when drawn additively (call inside `draw_blend_mode(BLEND_ADDITIVE)`).
/// Expands as it fades so it doesn't just pop off. A fuel drum's is whiter.
pub fn draw_blast_glow(d: &mut impl RaylibDraw, b: &BlastFx) {
    let seconds = tuning().blast_glow_seconds;
    if seconds <= 0.0 || b.time >= seconds {
        return;
    }
    let k = 1.0 - b.time / seconds;
    let r = tuning().blast_glow_radius * b.scale * (0.6 + 0.8 * (1.0 - k));
    let a = (255.0 * tuning().blast_glow_strength * k) as u8;
    let at = b.draw_center();
    let outer = match b.kind {
        BlastKind::Oil => Color::new(255, 150, 60, a),
        BlastKind::Fuel => Color::new(255, 220, 170, a),
    };
    pixel_disc(d, at, r, outer);
    pixel_disc(d, at, r * 0.45, Color::new(255, 230, 170, a));
}

/// A filled disc built out of whole `GLOW_BLOCK` blocks, one scanline of
/// blocks at a time.
///
/// Every sprite in the game lands on a 2-screen-pixel block (tanks draw a
/// 32px tile at scale 2; the wall sheet bakes the same chunkiness in), so
/// a smooth `draw_circle_v` here reads as a different, softer game layered
/// over this one - and the blast glow is the largest thing on screen when
/// it plays. Stepping the edge is the whole point: the visible staircase
/// is what makes it look drawn.
pub fn pixel_disc(d: &mut impl RaylibDraw, center: Position, radius: f32, color: Color) {
    if radius < GLOW_BLOCK {
        return;
    }
    // Snap the centre too, so a disc does not shimmer between block
    // alignments as whatever it is attached to moves.
    let cx = (center.x / GLOW_BLOCK).round() * GLOW_BLOCK;
    let cy = (center.y / GLOW_BLOCK).round() * GLOW_BLOCK;
    let rows = (radius / GLOW_BLOCK).floor() as i32;
    for row in -rows..=rows {
        let dy = row as f32 * GLOW_BLOCK;
        let half = (radius * radius - dy * dy).max(0.0).sqrt();
        let cols = (half / GLOW_BLOCK).floor() as i32;
        if cols <= 0 {
            continue;
        }
        let w = (cols * 2 + 1) as f32 * GLOW_BLOCK;
        d.draw_rectangle(
            (cx - cols as f32 * GLOW_BLOCK - GLOW_BLOCK / 2.0) as i32,
            (cy + dy - GLOW_BLOCK / 2.0) as i32,
            w as i32,
            GLOW_BLOCK as i32,
            color,
        );
    }
}

/// The block size every glow here quantises to - the same 2 screen pixels
/// one source pixel of every sprite in the game covers.
const GLOW_BLOCK: f32 = 2.0;

/// The pulsing glow on a barrel whose fuse is lit (additive, like the
/// bloom). `time` is the round clock, only used to phase the pulse.
pub fn draw_fuse_glow(d: &mut impl RaylibDraw, center: Position, time: f32) {
    let pulse = 0.5 + 0.5 * (time * 50.0).sin();
    let a = (255.0 * tuning().barrel_fuse_glow_strength * pulse) as u8;
    pixel_disc(d, center, 22.0, Color::new(255, 120, 40, a));
}

/// The glow of a burning ground cell (an oil pool or a lit trail):
/// additive like the fuse, flickering at its own hashed phase, dying down
/// over the last part of `left`.
pub fn draw_fire_glow(d: &mut impl RaylibDraw, center: Position, time: f32, left: f32) {
    let phase = (seed_at(center, 23) % 100) as f32 / 100.0 * std::f32::consts::TAU;
    let flicker = 0.7 + 0.3 * (time * 31.0 + phase).sin();
    let dying = (left / 0.6).clamp(0.0, 1.0);
    let a = (255.0 * 0.35 * flicker * dying) as u8;
    pixel_disc(d, center, 20.0, Color::new(255, 130, 40, a));
}

/// A burning ground cell's flames (`Game::fires`): the three-frame loop
/// on the scorch row, drawn at `blast_anim_scale`-independent scale 2 so
/// its design pixels match everything else, cycling at the wood burn
/// cadence with a phase hashed from the cell so a trail flickers out of
/// step with itself. Mirrored per cell, and it fades over the last part
/// of the burn instead of vanishing.
pub fn draw_ground_fire(d: &mut impl RaylibDraw, texture: &Texture2D, center: Position, time: f32, left: f32, total: f32) {
    let cell = BARREL_EXPLOSION_TEXTURE_SIZE;
    let seed = seed_at(center, 23);
    let cadence = tuning().wood_burn_frame_seconds.max(0.02);
    let frame = (((time / cadence) as i64 + (seed % FIRE_LOOP_FRAMES as u32) as i64).rem_euclid(FIRE_LOOP_FRAMES as i64)) as i32;
    let flip = if seed & 2 != 0 { -1.0 } else { 1.0 };
    let src = Rectangle::new((FIRE_LOOP_COL + frame) as f32 * cell, SCORCH_ROW as f32 * cell, cell * flip, cell);
    let size = cell * 2.0;
    let dying = (left / 0.6).clamp(0.0, 1.0);
    let catching = ((total - left) / 0.25).clamp(0.0, 1.0);
    let a = (255.0 * dying.min(catching)) as u8;
    // The flames sit in the lower half of the cell art, so the sprite is
    // centred a little above the ground cell and the tongues rise past it.
    let dest = Rectangle::new(center.x, center.y - 8.0, size, size);
    d.draw_texture_pro(texture, src, dest, Vector2::new(size / 2.0, size / 2.0), 0.0, Color::new(255, 255, 255, a));
}

/// The scorch decal, variant/mirror/quarter-turn picked by the seed (90
/// degree steps keep the pixels crisp), fading in over
/// `scorch_fade_in_seconds` so it appears under the fireball, not before.
/// A shot's scorch also gets the streak cell, turned to the nearest
/// quarter toward the shot.
pub fn draw_scorch(d: &mut impl RaylibDraw, texture: &Texture2D, s: &Scorch) {
    let cell = BARREL_EXPLOSION_TEXTURE_SIZE;
    let variant = (s.seed % SCORCH_VARIANTS as u32) as f32;
    let flip = if s.seed & 4 != 0 { -1.0 } else { 1.0 };
    let src = Rectangle::new(variant * cell, SCORCH_ROW as f32 * cell, cell * flip, cell);
    let size = cell * tuning().scorch_scale * s.scale;
    let rotation = ((s.seed >> 3) % 4) as f32 * 90.0;
    let fade = (s.age / tuning().scorch_fade_in_seconds.max(1e-3)).clamp(0.0, 1.0);
    let tint = Color::new(255, 255, 255, (255.0 * tuning().scorch_opacity * fade) as u8);
    let dest = Rectangle::new(s.center.x, s.center.y, size, size);
    d.draw_texture_pro(texture, src, dest, Vector2::new(size / 2.0, size / 2.0), rotation, tint);
    if let Some(dir) = s.streak {
        // The streak cell points right; turn it to the quarter nearest
        // the shot's travel.
        let angle = dir.y.atan2(dir.x).to_degrees();
        let quarter = ((angle / 90.0).round() * 90.0).rem_euclid(360.0);
        let src = Rectangle::new(SCORCH_STREAK_COL as f32 * cell, SCORCH_ROW as f32 * cell, cell, cell);
        d.draw_texture_pro(texture, src, dest, Vector2::new(size / 2.0, size / 2.0), quarter, tint);
    }
}
