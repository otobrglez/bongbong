//! The barrel blast's presentation: the one-shot fireball sprite animation,
//! the additive light bloom under it, the pulsing glow on a barrel whose
//! fuse is lit, and the scorch decal a blast leaves on the ground. Pure
//! drawing over `static/barrel_explosion.png` (docs/PROPS_SPEC.md); the
//! simulation (`simulation::props`) only pushes the `BlastFx`/`Scorch`
//! records, and never draws RNG for them - both seeds are a hash of the
//! blast position, so a purely cosmetic field can't shift a seeded replay.

use crate::tuning::tuning;
use sola_raylib::prelude::*;

use crate::{
    BARREL_EXPLOSION_FRAMES,
    BARREL_EXPLOSION_TEXTURE_SIZE,
    Position,
    SCORCH_ROW,
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

/// One in-flight barrel detonation sprite, oldest first in `Game::blast_fx`.
pub struct BlastFx {
    pub center: Position,
    /// Seconds since it went off.
    pub time: f32,
    pub seed: u32,
    /// Multiplier on `blast_anim_scale` and the glow radius - 1.0 for a
    /// barrel or a dying tank, smaller for a cook-off secondary.
    pub scale: f32,
}

impl BlastFx {
    pub fn new(center: Position) -> Self {
        BlastFx { center, time: 0.0, seed: seed_for(center), scale: 1.0 }
    }

    /// A scaled-down fireball for a wreck's ammo cooking off: the same
    /// animation, drawn smaller so a secondary reads as a pop rather than
    /// as a second tank dying.
    pub fn small(center: Position) -> Self {
        BlastFx { center, time: 0.0, seed: seed_for(center), scale: tuning().cookoff_blast_scale }
    }

    /// The frame to show now, clamped so a live change to `blast_anim_fps`
    /// can never index past the sheet.
    pub fn frame(&self) -> i32 {
        ((self.time * tuning().blast_anim_fps) as i32).clamp(0, BARREL_EXPLOSION_FRAMES - 1)
    }

    pub fn done(&self) -> bool {
        self.time >= BARREL_EXPLOSION_FRAMES as f32 / tuning().blast_anim_fps.max(1.0)
    }
}

/// A burn mark left where a barrel went off. Never removed during a round
/// (`Game::init` clears them, `SCORCH_MAX` caps them); `age` only drives
/// the fade-in under the fireball.
pub struct Scorch {
    pub center: Position,
    pub seed: u32,
    pub age: f32,
}

impl Scorch {
    pub fn new(center: Position) -> Self {
        Scorch { center, seed: seed_for(center), age: 0.0 }
    }
}

/// The fireball frame, drawn centred on the blast; mirrored per seed so
/// two chained blasts don't look cloned.
pub fn draw_blast(d: &mut impl RaylibDraw, texture: &Texture2D, b: &BlastFx) {
    let cell = BARREL_EXPLOSION_TEXTURE_SIZE;
    let flip = if b.seed & 1 != 0 { -1.0 } else { 1.0 };
    let src = Rectangle::new(b.frame() as f32 * cell, 0.0, cell * flip, cell);
    let size = cell * tuning().blast_anim_scale * b.scale;
    let dest = Rectangle::new(b.center.x, b.center.y, size, size);
    d.draw_texture_pro(texture, src, dest, Vector2::new(size / 2.0, size / 2.0), 0.0, Color::WHITE);
}

/// The light bloom under a fresh fireball - two flat discs that read as a
/// flash when drawn additively (call inside `draw_blend_mode(BLEND_ADDITIVE)`).
/// Expands as it fades so it doesn't just pop off.
pub fn draw_blast_glow(d: &mut impl RaylibDraw, b: &BlastFx) {
    let seconds = tuning().blast_glow_seconds;
    if seconds <= 0.0 || b.time >= seconds {
        return;
    }
    let k = 1.0 - b.time / seconds;
    let r = tuning().blast_glow_radius * b.scale * (0.6 + 0.8 * (1.0 - k));
    let a = (255.0 * tuning().blast_glow_strength * k) as u8;
    pixel_disc(d, b.center, r, Color::new(255, 150, 60, a));
    pixel_disc(d, b.center, r * 0.45, Color::new(255, 230, 170, a));
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

/// The scorch decal, variant/mirror/quarter-turn picked by the seed (90
/// degree steps keep the pixels crisp), fading in over
/// `scorch_fade_in_seconds` so it appears under the fireball, not before.
pub fn draw_scorch(d: &mut impl RaylibDraw, texture: &Texture2D, s: &Scorch) {
    let cell = BARREL_EXPLOSION_TEXTURE_SIZE;
    let variant = (s.seed % SCORCH_VARIANTS as u32) as f32;
    let flip = if s.seed & 4 != 0 { -1.0 } else { 1.0 };
    let src = Rectangle::new(variant * cell, SCORCH_ROW as f32 * cell, cell * flip, cell);
    let size = cell * tuning().scorch_scale;
    let rotation = ((s.seed >> 3) % 4) as f32 * 90.0;
    let fade = (s.age / tuning().scorch_fade_in_seconds.max(1e-3)).clamp(0.0, 1.0);
    let tint = Color::new(255, 255, 255, (255.0 * tuning().scorch_opacity * fade) as u8);
    let dest = Rectangle::new(s.center.x, s.center.y, size, size);
    d.draw_texture_pro(texture, src, dest, Vector2::new(size / 2.0, size / 2.0), rotation, tint);
}
