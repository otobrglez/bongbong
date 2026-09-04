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
    (center.x as i32 as u32)
        .wrapping_mul(73_856_093)
        .wrapping_add(1)
        ^ (center.y as i32 as u32).wrapping_mul(19_349_663)
}

/// One in-flight barrel detonation sprite, oldest first in `Game::blast_fx`.
pub struct BlastFx {
    pub center: Position,
    /// Seconds since it went off.
    pub time: f32,
    pub seed: u32,
}

impl BlastFx {
    pub fn new(center: Position) -> Self {
        BlastFx { center, time: 0.0, seed: seed_for(center) }
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
    let size = cell * tuning().blast_anim_scale;
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
    let r = tuning().blast_glow_radius * (0.6 + 0.8 * (1.0 - k));
    let a = (255.0 * tuning().blast_glow_strength * k) as u8;
    d.draw_circle_v(b.center, r, Color::new(255, 150, 60, a));
    d.draw_circle_v(b.center, r * 0.45, Color::new(255, 230, 170, a));
}

/// The pulsing glow on a barrel whose fuse is lit (additive, like the
/// bloom). `time` is the round clock, only used to phase the pulse.
pub fn draw_fuse_glow(d: &mut impl RaylibDraw, center: Position, time: f32) {
    let pulse = 0.5 + 0.5 * (time * 50.0).sin();
    let a = (255.0 * tuning().barrel_fuse_glow_strength * pulse) as u8;
    d.draw_circle_v(center, 22.0, Color::new(255, 120, 40, a));
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
