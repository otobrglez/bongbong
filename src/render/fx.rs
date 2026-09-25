//! Drawing the particle layer (`fx.rs` owns the particles and their
//! motion): every particle is a raylib rectangle snapped to the `FX_GRID`
//! block, coloured off the ramps `fx.rs` keeps.

use sola_raylib::prelude::*;

use crate::fx::{Fx, Particle, ParticleKind, EMBER_T, FIRE_T, FX_GRID, STONE_LT, STONE_MD, WHITE_T};
use crate::math::Color;
use crate::tuning::tuning;

/// Draw every particle. Non-additive kinds first in one run, then all
/// the additive ones inside a single blend-mode block: a blend switch
/// breaks raylib's batch, so interleaving them would cost one batch
/// per particle instead of two for the whole layer.
pub fn draw(fx: &Fx, d: &mut impl RaylibDraw) {
    for p in fx.particles().iter().filter(|p| !p.kind.additive()) {
        draw_particle(d, p);
    }
    if fx.particles().iter().any(|p| p.kind.additive()) {
        d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
            for p in fx.particles().iter().filter(|p| p.kind.additive()) {
                draw_particle(&mut bd, p);
            }
        });
    }
}

impl ParticleKind {
    /// Drawn inside the additive blend block: light, not matter.
    fn additive(self) -> bool {
        matches!(self, ParticleKind::Spark | ParticleKind::Ember)
    }
}

const SMOKE_DK: Color = Color::new(0x37, 0x37, 0x37, 255);
const SMOKE_MD: Color = Color::new(0x55, 0x52, 0x4E, 255);
const SMOKE_LT: Color = Color::new(0x7E, 0x7E, 0x7E, 255);
const DEEP_T: Color = Color::new(0x81, 0x2F, 0x27, 255);

/// Fire cools as it ages: white-hot, then yellow, orange, deep red. Four
/// steps is enough to read as a flame and few enough to stay obviously
/// hand-picked rather than interpolated.
const FIRE_RAMP: [Color; 4] = [WHITE_T, FIRE_T, EMBER_T, DEEP_T];
/// Smoke lightens and thins as it rises and cools.
const SMOKE_RAMP: [Color; 3] = [SMOKE_DK, SMOKE_MD, SMOKE_LT];
/// A missile's trail is white where it was just laid and greys as it thins.
const TRAIL_RAMP: [Color; 3] = [WHITE_T, STONE_LT, STONE_MD];

fn snap(v: f32) -> i32 {
    ((v / FX_GRID).floor() * FX_GRID) as i32
}

fn ramp_pick(ramp: &[Color], t: f32) -> Color {
    let i = ((t * ramp.len() as f32) as usize).min(ramp.len() - 1);
    ramp[i]
}

fn draw_particle(d: &mut impl RaylibDraw, p: &Particle) {
    let t = (p.age / p.life).clamp(0.0, 1.0);
    let base = match p.kind {
        ParticleKind::Spark | ParticleKind::Ember => ramp_pick(&FIRE_RAMP, t),
        ParticleKind::Smoke => ramp_pick(&SMOKE_RAMP, t),
        ParticleKind::Trail => ramp_pick(&TRAIL_RAMP, t),
        // A chip or a dust mote keeps the colour of whatever it came off.
        _ => p.tint,
    };
    // Stepped, not smooth: four levels of transparency read as a pixel-art
    // dissolve, where a continuous fade reads as a soft airbrushed blob.
    let levels = 4.0;
    let fade = ((1.0 - t) * levels).ceil() / levels;
    let opacity = match p.kind {
        ParticleKind::Smoke => fade * tuning().smoke_opacity,
        ParticleKind::Trail => fade * tuning().missile_trail_opacity,
        // Fire holds full brightness and dies by stepping down the ramp
        // rather than by dimming.
        ParticleKind::Spark | ParticleKind::Ember => if t < 0.85 { 1.0 } else { 0.5 },
        _ => fade,
    };
    if opacity <= 0.0 {
        return;
    }
    let c = Color::new(base.r, base.g, base.b, (255.0 * opacity) as u8);
    // `z` is a straight y-offset - the game is top-down with no camera, so
    // height is just "further up the screen".
    let blocks = (p.size / FX_GRID).round().max(1.0);
    let side = (blocks * FX_GRID) as i32;
    let x = snap(p.pos.x - blocks * FX_GRID / 2.0);
    let y = snap(p.pos.y - p.z - blocks * FX_GRID / 2.0);
    d.draw_rectangle(x, y, side, side, c);
}

