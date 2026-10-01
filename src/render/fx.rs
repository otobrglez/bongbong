//! Drawing the particle layer (`fx.rs` owns the particles and their
//! motion), in the effects language (`pyro.rs`, docs/effects.md): every
//! particle is whole 2 px blocks on the field's grid and every colour a
//! ramp step. Light (sparks, embers) is a hot block cooling down the fire
//! ramp, a fast spark dragging a short tail of blocks; air (smoke, dust, a
//! missile's trail) is a shaded puff - a shadow step, the body, a lit side
//! up and left - stepping paler and thinner as it rises; matter (chips,
//! spray) is a small solid square.

use sola_raylib::prelude::*;

use crate::fx::{Fx, Particle, ParticleKind, EMBER_T, FIRE_T, SOOT_T, WHITE_T};
use crate::math::{Color, Rectangle, Vec2};
use crate::view::culled;
use crate::pyro::{self, Puff, BLOCK, FIRE, SMOKE};
use crate::render::pyro::Rl;
use crate::tuning::tuning;

/// Draw every particle `cull` keeps (`view::Camera::cull`; `None` keeps
/// them all). Non-additive kinds first in one run, then all the additive
/// ones inside a single blend-mode block: a blend switch breaks raylib's
/// batch, so interleaving them would cost one batch per particle instead
/// of two for the whole layer.
pub fn draw(fx: &Fx, d: &mut impl RaylibDraw, cull: Option<Rectangle>) {
    let shown = |p: &&Particle| !culled(cull, p.pos);
    for p in fx.particles().iter().filter(|p| !p.kind.additive()).filter(shown) {
        draw_particle(d, p);
    }
    if fx.particles().iter().filter(shown).any(|p| p.kind.additive()) {
        d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
            for p in fx.particles().iter().filter(|p| p.kind.additive()).filter(shown) {
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

/// Fire cools as it ages: white, pale gold, gold, red, deep red.
const FIRE_RAMP: [Color; 5] = [FIRE[7], FIRE[6], FIRE[5], FIRE[3], FIRE[2]];
/// Smoke pales and thins as it rises and cools.
const SMOKE_RAMP: [Color; 3] = [SMOKE[2], SMOKE[3], SMOKE[4]];
/// Black smoke off burning oil, a burning deck or a wreck: the same ramp a
/// step darker.
const SOOT_RAMP: [Color; 3] = [SMOKE[1], SMOKE[2], SMOKE[3]];
/// A missile's trail is near white where it was just laid and greys as it
/// thins.
const TRAIL_RAMP: [Color; 4] = [SMOKE[6], SMOKE[5], SMOKE[4], SMOKE[3]];

/// The step `t` (0..1) of the way down `ramp`: never a blend of two.
fn ramp_at(ramp: &[Color], t: f32) -> Color {
    let i = (t.clamp(0.0, 1.0) * ramp.len() as f32) as usize;
    ramp[i.min(ramp.len() - 1)]
}

/// The steps a tinted light (a portal's blue, a plasma bolt's teal, a
/// laser's red) runs through: a white flash, then its own colour, then
/// its own colour dimmed.
fn tinted(tint: Color, t: f32) -> Color {
    match (t * 3.0) as i32 {
        0 => WHITE_T,
        1 => tint,
        _ => Color::new(tint.r, tint.g, tint.b, 150),
    }
}

fn draw_particle(d: &mut impl RaylibDraw, p: &Particle) {
    let t = (p.age / p.life).clamp(0.0, 1.0);
    // `z` is a straight y-offset - the game looks straight down with no
    // perspective, so height is just "further up the screen".
    let at = Vec2::new(p.pos.x, p.pos.y - p.z);
    let mut b = Rl(d);
    match p.kind {
        // Light: one hot block, full strength through most of its life and
        // stepping out over the end; a fast spark drags a tail of blocks
        // stepping down its ramp behind it.
        ParticleKind::Spark | ParticleKind::Ember => {
            let base = if is_fire(p.tint) { ramp_at(&FIRE_RAMP, t) } else { tinted(p.tint, t) };
            let k = if t < 0.6 { 1.0 } else if t < 0.8 { 0.66 } else { 0.33 };
            let c = pyro::alpha(base, k);
            // An ember flickers: it drops out for a beat now and then.
            if p.kind == ParticleKind::Ember && ((p.age * 23.0 + p.pos.x * 0.37).sin() < -0.72) {
                return;
            }
            let speed = p.vel.length();
            if p.kind == ParticleKind::Spark && speed > SMEAR_SPEED {
                let dir = p.vel * (1.0 / speed);
                let len = (speed * 0.03).min(12.0);
                let tail = Vec2::new(at.x - dir.x * len, at.y - dir.y * len);
                let cool = pyro::alpha(ramp_at(&FIRE_RAMP, (t + 0.4).min(1.0)), 0.5 * k);
                pyro::block_line(&mut b, at, tail, 1, |s| if s < 0.34 { c } else { cool });
            } else {
                pyro::mark(&mut b, at, BLOCK as i32, c);
            }
        }
        // Air: a shaded puff, paler as it rises and thinner as it goes,
        // the whole puff one translucent step (eighths), so a column of
        // them builds up where it is thick.
        ParticleKind::Smoke | ParticleKind::Trail | ParticleKind::Dust => {
            let (opacity, body, lit, shadow) = match p.kind {
                ParticleKind::Smoke => {
                    let soot = p.tint.r == SOOT_T.r && p.tint.g == SOOT_T.g && p.tint.b == SOOT_T.b;
                    let body = ramp_at(if soot { &SOOT_RAMP } else { &SMOKE_RAMP }, t);
                    (tuning().smoke_opacity * 1.15, body, lighter(&SMOKE, body), None)
                }
                ParticleKind::Trail => {
                    let body = ramp_at(&TRAIL_RAMP, t);
                    (tuning().missile_trail_opacity, body, lighter(&SMOKE, body), darker(&SMOKE, body))
                }
                // Dust keeps the colour of whatever it came off.
                _ => (0.9, p.tint, None, None),
            };
            let k = (1.0 - t * t) * opacity;
            if k <= 0.05 {
                return;
            }
            let fade = |c: Color| pyro::alpha(c, k.min(1.0));
            let puff = Puff {
                pos: at,
                radius: (p.size * 0.5 + 0.5).max(BLOCK),
                body: fade(body),
                shadow: shadow.map(fade),
                lit: lit.map(fade),
                core: None,
                cover: 1.0,
            };
            pyro::draw_puff(&mut b, &puff);
        }
        // Matter: a chip of something or a drop of water, a small solid
        // square on the block grid like the sprites it came off, stepping
        // out in quarters over its life.
        _ => {
            let fade_step = ((1.0 - t) * 4.0).ceil() / 4.0;
            let blocks = (p.size / BLOCK).round().max(1.0) as i32;
            pyro::mark(&mut b, at, blocks * BLOCK as i32, pyro::alpha(p.tint, fade_step));
        }
    }
}

/// The step after `c` in `ramp`, if there is one.
fn lighter(ramp: &[Color], c: Color) -> Option<Color> {
    ramp.iter().position(|r| r.r == c.r && r.g == c.g && r.b == c.b).and_then(|i| ramp.get(i + 1).copied())
}

/// The step before `c` in `ramp`, if there is one.
fn darker(ramp: &[Color], c: Color) -> Option<Color> {
    ramp.iter().position(|r| r.r == c.r && r.g == c.g && r.b == c.b).and_then(|i| i.checked_sub(1)).map(|i| ramp[i])
}

/// Speed (px/s) above which a spark drags a tail.
const SMEAR_SPEED: f32 = 90.0;

/// Is `c` one of the fire colours a spark or ember is thrown in? Those
/// cool down `FIRE_RAMP`; anything else keeps its own colour.
fn is_fire(c: Color) -> bool {
    [WHITE_T, FIRE_T, EMBER_T].iter().any(|f| f.r == c.r && f.g == c.g && f.b == c.b)
}
