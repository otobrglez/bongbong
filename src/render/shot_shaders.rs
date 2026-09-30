//! What is drawn by fragment shader rather than from blocks or sprites:
//! the plasma bolt's orb (`static/plasma_orb.fs`) and the flamethrower's
//! jet of burning fuel (`static/flame_jet.fs`), each on one textured quad.
//! The sources are compiled into the binary (the GLSL ES 100 twins in
//! `static/web/` on the embedded builds), so no platform needs the files
//! on disk. Every hit's burst is composed in blocks instead (`burst.rs`).
//!
//! Every uniform is set per draw from the shot and the round clock alone -
//! nothing is kept between frames - so a replica, a paused frame and a
//! re-render all draw the same picture.

use sola_raylib::prelude::*;

use crate::math::{Color, Vec2};
use crate::plasma::{Plasma, PlasmaVariant};
use crate::tuning::tuning;
use crate::Position;

/// How far the orb's glow reaches ahead of and beside its centre, in orb
/// radii - the quad's half width and the part of it ahead of the centre.
const ORB_REACH: f32 = 2.3;

struct OrbLocs {
    orb_pos: i32,
    radius_px: i32,
    field_height: i32,
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
    origin: i32,
    dir: i32,
    half_width: i32,
    field_height: i32,
}

/// The compiled shaders, their uniform locations and the 1x1 white texture
/// every quad is drawn with (so the fragment shader's texture coordinates
/// run 0..1 across the quad).
pub struct ShotShaders {
    orb: Shader,
    orb_locs: OrbLocs,
    flame: Shader,
    flame_locs: FlameLocs,
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
        let (orb_src, flame_src) = if crate::EMBEDDED {
            (include_str!("../../static/web/plasma_orb.fs"), include_str!("../../static/web/flame_jet.fs"))
        } else {
            (include_str!("../../static/plasma_orb.fs"), include_str!("../../static/flame_jet.fs"))
        };
        let orb = compile(rl, thread, "plasma_orb.fs", orb_src)?;
        let flame = compile(rl, thread, "flame_jet.fs", flame_src)?;
        let white = Image::gen_image_color(1, 1, Color::WHITE);
        let quad = rl.load_texture_from_image(thread, &white).map_err(|e| format!("shot quad: {e}"))?;
        let orb_locs = OrbLocs {
            orb_pos: orb.get_shader_location("orbPos"),
            radius_px: orb.get_shader_location("radiusPx"),
            field_height: orb.get_shader_location("fieldHeight"),
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
            origin: flame.get_shader_location("origin"),
            dir: flame.get_shader_location("dir"),
            half_width: flame.get_shader_location("halfWidth"),
            field_height: flame.get_shader_location("fieldHeight"),
        };
        Ok(ShotShaders { orb, orb_locs, flame, flame_locs, quad })
    }

    /// Draw a flying plasma bolt as its orb: the ball, its glow, its
    /// style's outer effect (teal's lightning, purple's spiral arms and
    /// stars) and its comet tail, on one quad turned to face travel with
    /// the orb `ORB_REACH` radii from its front edge, into a target
    /// `field_height` px tall (the shader works the bolt out per 2 px block
    /// of it).
    pub fn draw_orb<D: RaylibDraw + RaylibShaderModeExt>(&mut self, d: &mut D, plasma: &Plasma, time: f32, field_height: f32) {
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
        s.set_shader_value(l.orb_pos, Vector2::new(plasma.position.x, plasma.position.y));
        s.set_shader_value(l.radius_px, r);
        s.set_shader_value(l.field_height, field_height);
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
    /// end, into a target `field_height` px tall (the field: the shader
    /// works the stream out per 2 px block of it). `seed` keeps two
    /// streams from rolling in step.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_flame<D: RaylibDraw + RaylibShaderModeExt>(&mut self, d: &mut D, origin: Position, dir: Vec2, reach: f32, time: f32, seed: f32, field_height: f32) {
        if reach <= 4.0 {
            return;
        }
        let half = reach * tuning().flame_half_angle_deg.to_radians().tan() + 6.0;
        let l = &self.flame_locs;
        let s = &mut self.flame;
        s.set_shader_value(l.time, time);
        s.set_shader_value(l.seed, seed);
        s.set_shader_value(l.reach, reach);
        s.set_shader_value(l.origin, [origin.x, origin.y]);
        s.set_shader_value(l.dir, [dir.x, dir.y]);
        s.set_shader_value(l.half_width, half);
        s.set_shader_value(l.field_height, field_height);
        let angle = dir.y.atan2(dir.x).to_degrees() - 90.0;
        let dest = Rectangle::new(origin.x, origin.y, half * 2.0, reach);
        let quad = &self.quad;
        d.draw_shader_mode(s, |mut sd| {
            sd.draw_texture_pro(quad, Rectangle::new(0.0, 0.0, 1.0, 1.0), dest, Vector2::new(half, 0.0), angle, Color::WHITE);
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

    /// The GLSL ES 100 twins in `static/web/` are ported by hand: each
    /// declares every uniform its desktop shader does, so the web build
    /// never draws with a zero where a value should be, and writes
    /// `gl_FragColor`.
    #[test]
    fn the_web_twins_declare_the_desktop_uniforms() {
        fn uniforms(source: &str) -> Vec<String> {
            let mut out: Vec<String> = source
                .lines()
                .filter_map(|line| line.trim().strip_prefix("uniform "))
                .map(|rest| rest.split(';').next().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" "))
                .collect();
            out.sort();
            out
        }
        let pairs = [
            (include_str!("../../static/plasma_orb.fs"), include_str!("../../static/web/plasma_orb.fs")),
            (include_str!("../../static/flame_jet.fs"), include_str!("../../static/web/flame_jet.fs")),
            (include_str!("../../static/impact.fs"), include_str!("../../static/web/impact.fs")),
            (include_str!("../../static/muzzle_flash.fs"), include_str!("../../static/web/muzzle_flash.fs")),
            (include_str!("../../static/shockwave.fs"), include_str!("../../static/web/shockwave.fs")),
        ];
        for (desktop, web) in pairs {
            assert!(desktop.starts_with("#version 330") && web.starts_with("#version 100"));
            assert_eq!(uniforms(desktop), uniforms(web));
            assert!(web.contains("gl_FragColor") && !web.contains("finalColor ="), "the twin writes gl_FragColor");
        }
    }
}
