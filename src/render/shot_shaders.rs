//! What is drawn by fragment shader rather than from blocks or sprites:
//! the plasma bolt's orb (`static/plasma_orb.fs`), the flamethrower's jet
//! of burning fuel (`static/flame_jet.fs`) and every hit's burst
//! (`static/impact_burst.fs`), each on one textured quad. Both sources are compiled into the binary
//! (the GLSL ES 100 twins in `static/web/` on the embedded builds), so no
//! platform needs the files on disk.
//!
//! Every uniform is set per draw from the shot and the round clock alone -
//! nothing is kept between frames - so a replica, a paused frame and a
//! re-render all draw the same picture.

use sola_raylib::prelude::*;

use crate::fx::{Impact, ImpactKind};
use crate::math::{Color, Vec2};
use crate::plasma::{Plasma, PlasmaVariant};
use crate::tuning::tuning;
use crate::Position;

/// How far the orb's glow reaches ahead of and beside its centre, in orb
/// radii - the quad's half width and the part of it ahead of the centre.
const ORB_REACH: f32 = 2.3;

struct OrbLocs {
    quad_size: i32,
    center: i32,
    time: i32,
    spin: i32,
    tilt: i32,
    seed: i32,
    style: i32,
    rotation: i32,
    trail: i32,
    breathe: i32,
    deep: i32,
    body: i32,
    bright: i32,
    hot: i32,
}

struct FlameLocs {
    time: i32,
    seed: i32,
    reach: i32,
}

struct ImpactLocs {
    t: i32,
    style: i32,
    dir: i32,
    seed: i32,
    bright: i32,
    body: i32,
}

/// The compiled shaders, their uniform locations and the 1x1 white texture
/// every quad is drawn with (so the fragment shader's texture coordinates
/// run 0..1 across the quad).
pub struct ShotShaders {
    orb: Shader,
    orb_locs: OrbLocs,
    flame: Shader,
    flame_locs: FlameLocs,
    impact: Shader,
    impact_locs: ImpactLocs,
    quad: Texture2D,
}

/// What makes one bolt look unlike the next, all hashed from its id: a
/// pattern offset, a spin speed factor and direction, and the lean of the
/// axis it spins about.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrbLook {
    pub seed: f32,
    pub spin_factor: f32,
    pub tilt: f32,
    pub phase: f32,
}

impl OrbLook {
    pub fn of(id: u32) -> Self {
        let h = id.wrapping_mul(2_654_435_761) ^ 0x9E37_79B9;
        let unit = |shift: u32| ((h >> shift) & 0xff) as f32 / 255.0;
        let sign = if h & 1 == 0 { 1.0 } else { -1.0 };
        OrbLook {
            seed: unit(0) * 97.0 + unit(8) * 3.0,
            spin_factor: sign * (0.7 + 0.8 * unit(16)),
            tilt: (unit(24) - 0.5) * 1.8,
            phase: unit(4) * std::f32::consts::TAU,
        }
    }
}

fn rgb(c: Color) -> [f32; 3] {
    [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0]
}

fn compile(rl: &mut RaylibHandle, thread: &RaylibThread, name: &str, source: &str) -> Result<Shader, String> {
    rl.load_shader_from_memory(thread, None, Some(source)).map_err(|e| format!("{name}: {e}"))
}

impl ShotShaders {
    /// Compile both shaders. A failure is returned rather than fatal: the
    /// game then flies the baked plasma sprite and draws the flamethrower
    /// from its particles alone.
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Result<Self, String> {
        let (orb_src, flame_src, impact_src) = if crate::EMBEDDED {
            (
                include_str!("../../static/web/plasma_orb.fs"),
                include_str!("../../static/web/flame_jet.fs"),
                include_str!("../../static/web/impact_burst.fs"),
            )
        } else {
            (include_str!("../../static/plasma_orb.fs"), include_str!("../../static/flame_jet.fs"), include_str!("../../static/impact_burst.fs"))
        };
        let orb = compile(rl, thread, "plasma_orb.fs", orb_src)?;
        let flame = compile(rl, thread, "flame_jet.fs", flame_src)?;
        let impact = compile(rl, thread, "impact_burst.fs", impact_src)?;
        let white = Image::gen_image_color(1, 1, Color::WHITE);
        let quad = rl.load_texture_from_image(thread, &white).map_err(|e| format!("shot quad: {e}"))?;
        let orb_locs = OrbLocs {
            quad_size: orb.get_shader_location("quadSize"),
            center: orb.get_shader_location("center"),
            time: orb.get_shader_location("time"),
            spin: orb.get_shader_location("spin"),
            tilt: orb.get_shader_location("tilt"),
            seed: orb.get_shader_location("seed"),
            style: orb.get_shader_location("style"),
            rotation: orb.get_shader_location("rotation"),
            trail: orb.get_shader_location("trail"),
            breathe: orb.get_shader_location("breathe"),
            deep: orb.get_shader_location("cDeep"),
            body: orb.get_shader_location("cBody"),
            bright: orb.get_shader_location("cBright"),
            hot: orb.get_shader_location("cHot"),
        };
        let flame_locs = FlameLocs {
            time: flame.get_shader_location("time"),
            seed: flame.get_shader_location("seed"),
            reach: flame.get_shader_location("reach"),
        };
        let impact_locs = ImpactLocs {
            t: impact.get_shader_location("t"),
            style: impact.get_shader_location("style"),
            dir: impact.get_shader_location("dir"),
            seed: impact.get_shader_location("seed"),
            bright: impact.get_shader_location("cA"),
            body: impact.get_shader_location("cB"),
        };
        Ok(ShotShaders { orb, orb_locs, flame, flame_locs, impact, impact_locs, quad })
    }

    /// Draw a flying plasma bolt as its orb: the ball, its glow, its
    /// style's outer effect (teal's lightning, purple's spiral arms and
    /// stars) and its comet tail, on one quad turned to face travel with
    /// the orb `ORB_REACH` radii from its front edge.
    pub fn draw_orb<D: RaylibDraw + RaylibShaderModeExt>(&mut self, d: &mut D, plasma: &Plasma, time: f32) {
        let t = tuning();
        let look = OrbLook::of(plasma.id);
        let pulse = 0.5 + 0.5 * (time * t.plasma_pulse_hz * std::f32::consts::TAU + look.phase).sin();
        let breathe = t.plasma_pulse_min_scale + (t.plasma_pulse_max_scale - t.plasma_pulse_min_scale) * pulse;
        let r = t.plasma_orb_radius * (0.96 + 0.08 * pulse);
        let trail = (t.plasma_trail_length / r).max(0.0);
        let (w, h) = (ORB_REACH * 2.0, ORB_REACH + trail.max(0.4));
        let spin = time * t.plasma_orb_spin_hz * std::f32::consts::TAU * look.spin_factor + look.phase;
        let [deep, body, bright, hot] = plasma.variant.orb_colors();
        let style = match plasma.variant {
            PlasmaVariant::Teal => 0.0f32,
            PlasmaVariant::Purple => 1.0,
        };
        let l = &self.orb_locs;
        let s = &mut self.orb;
        s.set_shader_value(l.quad_size, Vector2::new(w, h));
        s.set_shader_value(l.center, Vector2::new(0.5, ORB_REACH / h));
        s.set_shader_value(l.time, time);
        s.set_shader_value(l.spin, spin);
        s.set_shader_value(l.tilt, look.tilt);
        s.set_shader_value(l.seed, look.seed);
        s.set_shader_value(l.style, style);
        s.set_shader_value(l.rotation, plasma.rotation.to_radians());
        s.set_shader_value(l.trail, trail);
        s.set_shader_value(l.breathe, breathe);
        s.set_shader_value(l.deep, rgb(deep));
        s.set_shader_value(l.body, rgb(body));
        s.set_shader_value(l.bright, rgb(bright));
        s.set_shader_value(l.hot, rgb(hot));
        let dest = Rectangle::new(plasma.position.x, plasma.position.y, w * r, h * r);
        let origin = Vector2::new(ORB_REACH * r, ORB_REACH * r);
        let quad = &self.quad;
        d.draw_shader_mode(s, |mut sd| {
            sd.draw_texture_pro(quad, Rectangle::new(0.0, 0.0, 1.0, 1.0), dest, origin, plasma.rotation, Color::WHITE);
        });
    }

    /// Draw the flamethrower's jet from `origin` along `dir` (a unit
    /// vector) for `reach` px, on a quad as wide as the cone at its far
    /// end. `seed` keeps two streams from rolling in step.
    pub fn draw_flame<D: RaylibDraw + RaylibShaderModeExt>(&mut self, d: &mut D, origin: Position, dir: Vec2, reach: f32, time: f32, seed: f32) {
        if reach <= 4.0 {
            return;
        }
        let half = reach * tuning().flame_half_angle_deg.to_radians().tan() + 6.0;
        let l = &self.flame_locs;
        let s = &mut self.flame;
        s.set_shader_value(l.time, time);
        s.set_shader_value(l.seed, seed);
        s.set_shader_value(l.reach, reach);
        let angle = dir.y.atan2(dir.x).to_degrees() - 90.0;
        let dest = Rectangle::new(origin.x, origin.y, half * 2.0, reach);
        let quad = &self.quad;
        d.draw_shader_mode(s, |mut sd| {
            sd.draw_texture_pro(quad, Rectangle::new(0.0, 0.0, 1.0, 1.0), dest, Vector2::new(half, 0.0), angle, Color::WHITE);
        });
    }
}

impl ShotShaders {
    /// Draw one hit's burst, centred where the shot landed, on a square
    /// quad as wide as its look reaches (`static/impact_burst.fs`).
    pub fn draw_impact<D: RaylibDraw + RaylibShaderModeExt>(&mut self, d: &mut D, impact: &Impact) {
        let (style, half, bright, body) = match impact.kind {
            ImpactKind::Shell => (0.0f32, 46.0f32, Color::WHITE, Color::WHITE),
            ImpactKind::Bullet => (1.0, 26.0, Color::WHITE, Color::WHITE),
            ImpactKind::Plasma(v) => {
                let [_, body, bright, _] = v.orb_colors();
                let style = if v == PlasmaVariant::Purple { 3.0 } else { 2.0 };
                (style, 44.0, bright, body)
            }
            ImpactKind::Laser(blue) => {
                let (bright, body) = if blue {
                    (Color::new(150, 200, 255, 255), Color::new(40, 110, 255, 255))
                } else {
                    (Color::new(255, 170, 150, 255), Color::new(255, 50, 40, 255))
                };
                (4.0, 30.0, bright, body)
            }
        };
        let half = half * tuning().hit_fx_scale;
        let l = &self.impact_locs;
        let s = &mut self.impact;
        s.set_shader_value(l.t, impact.progress());
        s.set_shader_value(l.style, style);
        s.set_shader_value(l.dir, Vector2::new(impact.dir.x, impact.dir.y));
        s.set_shader_value(l.seed, impact.seed);
        s.set_shader_value(l.bright, rgb(bright));
        s.set_shader_value(l.body, rgb(body));
        let dest = Rectangle::new(impact.pos.x - half, impact.pos.y - half, half * 2.0, half * 2.0);
        let quad = &self.quad;
        d.draw_shader_mode(s, |mut sd| {
            sd.draw_texture_pro(quad, Rectangle::new(0.0, 0.0, 1.0, 1.0), dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::OrbLook;

    #[test]
    fn every_bolt_looks_its_own_and_stays_in_range() {
        let looks: Vec<OrbLook> = (0..64).map(OrbLook::of).collect();
        for l in &looks {
            assert!((0.7..=1.5).contains(&l.spin_factor.abs()), "spin {l:?}");
            assert!(l.tilt.abs() <= 0.9 + 1e-6, "tilt {l:?}");
            assert!((0.0..=100.0).contains(&l.seed), "seed {l:?}");
        }
        let mut seeds: Vec<i64> = looks.iter().map(|l| (l.seed * 1000.0) as i64).collect();
        seeds.sort();
        seeds.dedup();
        assert!(seeds.len() > 48, "ids should spread over many patterns, got {}", seeds.len());
        assert!(looks.iter().any(|l| l.spin_factor < 0.0) && looks.iter().any(|l| l.spin_factor > 0.0), "both spin directions");
        assert_eq!(OrbLook::of(7), OrbLook::of(7), "a bolt keeps its look");
    }
}
