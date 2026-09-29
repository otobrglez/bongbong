//! The blast layers a live round draws with raylib directly: the fireball
//! frame, the ground-fire loop and the block-built glows (`pixel_disc`),
//! which `Game::render` draws inside an additive blend block. The scorch
//! decal, generic over `Canvas`, stays in `blast.rs`.

use sola_raylib::prelude::*;

use crate::blast::{seed_at, BlastFx, BlastKind};
use crate::math::{Color, Rectangle, Vec2};
use crate::tuning::tuning;
use crate::{Position, BARREL_EXPLOSION_TEXTURE_SIZE, FIRE_LOOP_COL, FIRE_LOOP_FRAMES, SCORCH_ROW};

impl BlastFx {
    /// Where the sprite is drawn: the centre plus the cause's lean.
    fn draw_center(&self) -> Position {
        Position::new(self.center.x + self.offset.x, self.center.y + self.offset.y)
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

/// The flamethrower's nozzle glow while it fires (additive): a hot disc
/// at the muzzle and a fainter, larger one part way down the stream,
/// both flickering off the round clock. The stream itself is particles
/// (`fx.rs`); this is the light they throw on the ground.
pub fn draw_flame_glow(d: &mut impl RaylibDraw, origin: Position, dir: Vec2, reach: f32, time: f32) {
    let flicker = 0.75 + 0.25 * (time * 47.0).sin();
    pixel_disc(d, origin, 10.0, Color::new(255, 200, 90, (150.0 * flicker) as u8));
    let mid = Position::new(origin.x + dir.x * reach * 0.4, origin.y + dir.y * reach * 0.4);
    pixel_disc(d, mid, (reach * 0.22).max(6.0), Color::new(255, 120, 40, (70.0 * flicker) as u8));
}

/// One slice of the flamethrower's stream at step `i` of `steps`: where
/// its centre sits (swaying sideways off the round clock), its half width
/// (licking in and out) and how far along the stream it is (0..1).
fn flame_slice(origin: Position, dir: Vec2, reach: f32, time: f32, i: i32, steps: i32) -> (Position, f32, f32, f32) {
    let spread = tuning().flame_half_angle_deg.to_radians().tan();
    let t = i as f32 / steps as f32;
    let along = reach * 0.8 * t;
    let lick = 0.5 + 0.5 * (time * 31.0 - i as f32 * 0.9).sin();
    let sway = (time * 19.0 + i as f32 * 0.7).sin() * along * 0.07;
    let at = Position::new(origin.x + dir.x * along - dir.y * sway, origin.y + dir.y * along + dir.x * sway);
    (at, 2.0 + along * spread * (0.4 + 0.25 * lick), t, lick)
}

/// Steps along the stream: one slice every 4 px of the drawn length.
fn flame_steps(reach: f32) -> i32 {
    (reach * 0.8 / 4.0).ceil() as i32
}

/// The flamethrower's stream body (normal blend, drawn over the tanks):
/// a tapering tongue of block discs, yellow at the root through orange to
/// a dull red tip, each slice's width and sideways sway flickering so the
/// stream licks and rolls. Drawn as paint rather than light so the orange
/// stays orange over grass; `draw_flame_core` is the light inside it and
/// the motes (`fx.rs`) ride over both.
pub fn draw_flame_body(d: &mut impl RaylibDraw, origin: Position, dir: Vec2, reach: f32, time: f32) {
    let strength = tuning().shot_glow_strength;
    if strength <= 0.0 || reach <= 4.0 {
        return;
    }
    let steps = flame_steps(reach);
    for i in (0..steps).rev() {
        let (at, half, t, lick) = flame_slice(origin, dir, reach, time, i, steps);
        let color = if t < 0.3 {
            Color::new(255, 206, 84, 255)
        } else if t < 0.6 {
            Color::new(246, 128, 38, 255)
        } else {
            Color::new(196, 56, 26, 255)
        };
        let a = (strength * (1.0 - t).powf(0.5) * (0.45 + 0.2 * lick)).clamp(0.0, 1.0);
        pixel_disc(d, at, half, Color::new(color.r, color.g, color.b, (255.0 * a) as u8));
    }
}

/// The white-hot light inside the stream (additive, over the body): a
/// narrower core through the first two thirds of it.
pub fn draw_flame_core(d: &mut impl RaylibDraw, origin: Position, dir: Vec2, reach: f32, time: f32) {
    let strength = tuning().shot_glow_strength;
    if strength <= 0.0 || reach <= 4.0 {
        return;
    }
    let steps = flame_steps(reach);
    for i in 0..steps {
        let (at, half, t, lick) = flame_slice(origin, dir, reach, time, i, steps);
        if t > 0.65 {
            break;
        }
        let a = (strength * (1.0 - t / 0.65) * (0.4 + 0.25 * lick)).clamp(0.0, 1.0);
        pixel_disc(d, at, half * 0.45, Color::new(255, 240, 190, (255.0 * a) as u8));
    }
}

/// The glow on a hull the flamethrower set alight (additive): a flickering
/// disc that shrinks as the afterburn runs out, `left` seconds to go.
pub fn draw_burning_hull_glow(d: &mut impl RaylibDraw, center: Position, time: f32, left: f32) {
    let phase = (seed_at(center, 29) % 100) as f32 / 100.0 * std::f32::consts::TAU;
    let flicker = 0.7 + 0.3 * (time * 37.0 + phase).sin();
    let dying = (left / tuning().flame_afterburn_seconds.max(0.1)).clamp(0.2, 1.0);
    pixel_disc(d, center, 12.0 + 8.0 * dying, Color::new(255, 130, 40, (110.0 * flicker * dying) as u8));
}

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
