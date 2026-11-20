//! Drawing the particle layer (`fx.rs` owns the particles and their
//! motion), in the effects language (`pyro.rs`, docs/effects.md): every
//! particle is whole 2 px blocks on the field's grid and every colour a
//! ramp step. Light (sparks, embers) is a hot block cooling down the fire
//! ramp, a fast spark dragging a short tail of blocks; air (smoke, dust, a
//! missile's trail) is a shaded puff - a shadow step, the body, a lit side
//! up and left - stepping paler and thinner as it rises; matter (chips,
//! spray) is a small solid square. The layer is drawn in three places
//! (docs/effects.md, "Painting"): what lies on the ground with the floor
//! (`draw_ground`), air and matter at the end of the lit field
//! (`draw_air`), so the night and the fog take them like everything else
//! that only reflects light, and the light itself in the glowing pass
//! (`draw_light`). A particle's ramp is its tint's (`fx::Ramp`).

use sola_raylib::prelude::*;

use crate::fx::{Fx, Particle, ParticleKind, Ramp, KICK_DESERT, KICK_DUST, KICK_SNOW};
use crate::math::{Color, Rectangle, Vec2};
use crate::view::culled;
use crate::pyro::{self, Puff, BLOCK, FIRE, SMOKE};
use crate::render::pyro::Rl;
use crate::tuning::{tuning, Tuning};

/// Draw the air and the matter `cull` keeps (`view::Camera::cull`; `None`
/// keeps them all) - smoke, a missile's trail, a grenade's plume, dust,
/// chips and spray - over everything standing in the lit field
/// (`paint_field_lit`): they reflect light and give none, so a weathered
/// frame darkens and fogs them with the ground they hang over.
pub fn draw_air(fx: &Fx, d: &mut impl RaylibDraw, cull: Option<Rectangle>) {
    let t = tuning();
    for p in fx.particles().iter().filter(|p| p.kind.in_air() && !culled(cull, p.pos)) {
        draw_particle(d, p, &t);
    }
}

/// Draw the light `cull` keeps - sparks and embers - in the glowing pass
/// (`paint_field_glowing`), inside a single additive block: a blend switch
/// breaks raylib's batch, so the whole layer's light is one.
pub fn draw_light(fx: &Fx, d: &mut impl RaylibDraw, cull: Option<Rectangle>) {
    let shown = |p: &&Particle| p.kind.additive() && !culled(cull, p.pos);
    if !fx.particles().iter().any(|p| shown(&p)) {
        return;
    }
    let t = tuning();
    d.draw_blend_mode(crate::frame_stages::blend(BlendMode::BLEND_ADDITIVE), |mut bd| {
        for p in fx.particles().iter().filter(shown) {
            draw_particle(&mut bd, p, &t);
        }
    });
}

/// Draw what lies on the ground - a hull's dust and powder, a ford's
/// foam - in the lit field (`paint_field_lit`), under the hulls and the
/// light pass, so it darkens at night and shows in a headlight's beam.
pub fn draw_ground(fx: &Fx, d: &mut impl RaylibDraw, cull: Option<Rectangle>) {
    let t = tuning();
    for p in fx.particles().iter().filter(|p| p.kind.on_ground() && !culled(cull, p.pos)) {
        draw_particle(d, p, &t);
    }
}

impl ParticleKind {
    /// Drawn inside the additive blend block: light, not matter.
    fn additive(self) -> bool {
        matches!(self, ParticleKind::Spark | ParticleKind::Ember)
    }

    /// Drawn with the floor in the lit field (`draw_ground`), not over
    /// the scene.
    fn on_ground(self) -> bool {
        matches!(self, ParticleKind::Kick | ParticleKind::Foam)
    }

    /// Drawn over everything standing in the lit field (`draw_air`): air
    /// and matter, whatever is neither light nor lying on the ground.
    fn in_air(self) -> bool {
        !self.additive() && !self.on_ground()
    }
}

/// The shadow and lit steps of a hull's dust on `ramp`; none for a tint
/// that is no ground's.
fn kick_shades(ramp: Ramp) -> (Option<Color>, Option<Color>) {
    let steps = match ramp {
        Ramp::Dust => KICK_DUST,
        Ramp::Desert => KICK_DESERT,
        Ramp::Snow => KICK_SNOW,
        _ => return (None, None),
    };
    (Some(steps[0].color), Some(steps[2].color))
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

/// A grenade's plume: near-white body paling to the lightest smoke, lit in
/// white and shaded in the stone's mid step (STONE_PALE, STONE_HI,
/// STONE_LT; WHITE; STONE_MID).
const PLUME_RAMP: [Color; 3] = [Color::new(0xF0, 0xF0, 0xF0, 255), SMOKE[6], SMOKE[5]];
const PLUME_LIT: Color = Color::new(0xFF, 0xFF, 0xFF, 255);
const PLUME_SHADE: Color = Color::new(0xB0, 0xB0, 0xB0, 255);

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
        0 => Color::WHITE,
        1 => tint,
        _ => Color::new(tint.r, tint.g, tint.b, 150),
    }
}

fn draw_particle(d: &mut impl RaylibDraw, p: &Particle, knobs: &Tuning) {
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
            let base = if p.tint.ramp == Ramp::Fire { ramp_at(&FIRE_RAMP, t) } else { tinted(p.tint.color, t) };
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
        ParticleKind::Smoke | ParticleKind::Trail | ParticleKind::Plume | ParticleKind::Dust => {
            let (opacity, body, lit, shadow) = match p.kind {
                ParticleKind::Smoke => {
                    let body = ramp_at(if p.tint.ramp == Ramp::Soot { &SOOT_RAMP } else { &SMOKE_RAMP }, t);
                    (knobs.smoke_opacity * 1.15, body, lighter(&SMOKE, body), None)
                }
                ParticleKind::Trail => {
                    let body = ramp_at(&TRAIL_RAMP, t);
                    (knobs.missile_trail_opacity, body, lighter(&SMOKE, body), darker(&SMOKE, body))
                }
                ParticleKind::Plume => (1.0, ramp_at(&PLUME_RAMP, t), Some(PLUME_LIT), Some(PLUME_SHADE)),
                // Dust keeps the colour of whatever it came off.
                _ => (0.9, p.tint.color, None, None),
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
        // Ground thrown up: a shaded puff that dissolves through the
        // Bayer pattern to half, then thins in eighths; foam on the water
        // is one colour and dissolves the same way.
        ParticleKind::Kick | ParticleKind::Foam => {
            let cover = 1.0 - t;
            let k = if cover < 0.5 { cover * 2.0 } else { 1.0 };
            let (shadow, lit) = if p.kind == ParticleKind::Kick { kick_shades(p.tint.ramp) } else { (None, None) };
            let fade = |c: Color| pyro::alpha(c, k);
            let puff = Puff {
                pos: at,
                radius: (p.size * 0.5).max(BLOCK),
                body: fade(p.tint.color),
                shadow: shadow.map(fade),
                lit: lit.map(fade),
                core: None,
                cover: cover.max(0.5),
            };
            pyro::draw_puff(&mut b, &puff);
        }
        // Matter: a chip of something or a drop of water, a small solid
        // square on the block grid like the sprites it came off, stepping
        // out in quarters over its life.
        _ => {
            let fade_step = ((1.0 - t) * 4.0).ceil() / 4.0;
            let blocks = (p.size / BLOCK).round().max(1.0) as i32;
            pyro::mark(&mut b, at, blocks * BLOCK as i32, pyro::alpha(p.tint.color, fade_step));
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
