//! Drawing the particle layer (`fx.rs` owns the particles and their
//! motion): light (sparks, embers) as a hot core in a soft halo with a
//! smear behind the fast ones, air (smoke, dust, trail) as soft round
//! puffs, matter (chips, spray) as small squares on the `FX_GRID` block -
//! all coloured off the ramps, blended smoothly between their steps.

use sola_raylib::prelude::*;

use crate::fx::{Fx, Particle, ParticleKind, EMBER_T, FIRE_T, FX_GRID, STONE_LT, STONE_MD, WHITE_T};
use crate::math::{Color, Vec2};
use crate::render::shot_fx::{fade, glow, streak};
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

/// Fire cools as it ages: white-hot, then yellow, orange, deep red.
const FIRE_RAMP: [Color; 4] = [WHITE_T, FIRE_T, EMBER_T, DEEP_T];
/// Smoke lightens and thins as it rises and cools.
const SMOKE_RAMP: [Color; 3] = [SMOKE_DK, SMOKE_MD, SMOKE_LT];
/// A missile's trail is white where it was just laid and greys as it thins.
const TRAIL_RAMP: [Color; 3] = [WHITE_T, STONE_LT, STONE_MD];

fn snap(v: f32) -> i32 {
    ((v / FX_GRID).floor() * FX_GRID) as i32
}

/// The colour `t` (0..1) of the way along `ramp`, blended smoothly
/// between its steps.
fn ramp_at(ramp: &[Color], t: f32) -> Color {
    let x = t.clamp(0.0, 1.0) * (ramp.len() - 1) as f32;
    let i = (x as usize).min(ramp.len() - 2);
    let f = x - i as f32;
    let (a, b) = (ramp[i], ramp[i + 1]);
    let l = |p: u8, q: u8| (p as f32 + (q as f32 - p as f32) * f).round() as u8;
    Color::new(l(a.r, b.r), l(a.g, b.g), l(a.b, b.b), 255)
}

fn draw_particle(d: &mut impl RaylibDraw, p: &Particle) {
    let t = (p.age / p.life).clamp(0.0, 1.0);
    let base = match p.kind {
        // Fire cools down its ramp; light of any other colour (a portal's
        // blue, a plasma bolt's teal, a laser's red) flashes white and
        // then burns in its own colour.
        ParticleKind::Spark | ParticleKind::Ember if is_fire(p.tint) => ramp_at(&FIRE_RAMP, t),
        ParticleKind::Spark | ParticleKind::Ember => ramp_at(&[WHITE_T, p.tint, p.tint], t),
        ParticleKind::Smoke => ramp_at(&SMOKE_RAMP, t),
        ParticleKind::Trail => ramp_at(&TRAIL_RAMP, t),
        // A chip or a dust mote keeps the colour of whatever it came off.
        _ => p.tint,
    };
    // `z` is a straight y-offset - the game is top-down with no camera, so
    // height is just "further up the screen".
    let at = Vec2::new(p.pos.x, p.pos.y - p.z);
    match p.kind {
        // Light: a hot core in a soft halo, and a fast spark smears into a
        // short tapering streak behind it. Holds full brightness through
        // most of its life and fades out over the end.
        ParticleKind::Spark | ParticleKind::Ember => {
            let k = 1.0 - ((t - 0.6) / 0.4).clamp(0.0, 1.0);
            let c = fade(base, k);
            let core = (p.size * 0.45).max(0.9);
            glow(d, at, p.size * 1.9, fade(base, 0.45 * k));
            d.draw_circle_v(at, core, c);
            let speed = p.vel.length();
            if p.kind == ParticleKind::Spark && speed > SMEAR_SPEED {
                let dir = p.vel * (1.0 / speed);
                streak(d, at, dir, (speed * 0.025).min(14.0), core * 2.0, c, Color::new(base.r, base.g, base.b, 0));
            }
        }
        // Air: a soft round puff, thinning smoothly as it spreads.
        ParticleKind::Smoke | ParticleKind::Trail | ParticleKind::Dust => {
            let opacity = match p.kind {
                ParticleKind::Smoke => tuning().smoke_opacity,
                ParticleKind::Trail => tuning().missile_trail_opacity,
                _ => 1.0,
            };
            let k = (1.0 - t) * (1.0 - t * 0.3) * opacity * 1.25;
            glow(d, at, p.size * 1.1, fade(base, k.clamp(0.0, 1.0)));
        }
        // Matter: a chip of something or a drop of water, a small solid
        // square on the block grid like the sprites it came off.
        _ => {
            let levels = 4.0;
            let fade_step = ((1.0 - t) * levels).ceil() / levels;
            let c = Color::new(base.r, base.g, base.b, (255.0 * fade_step) as u8);
            let blocks = (p.size / FX_GRID).round().max(1.0);
            let side = (blocks * FX_GRID) as i32;
            let x = snap(at.x - blocks * FX_GRID / 2.0);
            let y = snap(at.y - blocks * FX_GRID / 2.0);
            d.draw_rectangle(x, y, side, side, c);
        }
    }
}

/// Speed (px/s) above which a spark smears into a streak.
const SMEAR_SPEED: f32 = 90.0;

/// Is `c` one of the fire colours a spark or ember is thrown in? Those
/// cool down `FIRE_RAMP`; anything else keeps its own colour.
fn is_fire(c: Color) -> bool {
    [WHITE_T, FIRE_T, EMBER_T].iter().any(|f| f.r == c.r && f.g == c.g && f.b == c.b)
}
