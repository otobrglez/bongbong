//! The GPU side of the ripple effects: the compiled shader and its
//! uniforms (`shockwave.rs` owns `Shockwave`, the ripple in flight).

use sola_raylib::prelude::*;

use crate::math::Vec2;
use crate::shockwave::{ripple_uv, RIPPLE_FRAME};
use crate::view::Camera;
use crate::Position;

/// The GPU side of a ripple effect: `static/shockwave.fs` compiled with its
/// own tuning uniforms (speed/width/strength/duration, set once at load since
/// each instance keeps a fixed look for the whole run) plus the two uniform
/// locations that change on every draw. The kill shockwave and the muzzle
/// flash are each their own `RippleFx` - same shader, different tuning and
/// (in `Game::render`) different quad size.
pub struct RippleFx {
    pub shader: Shader,
    pub center_loc: i32,
    pub time_loc: i32,
    /// Array uniforms, used only by the whole-screen shock instance: it is
    /// the one effect that can have several live at once, and they have to
    /// resolve in a single pass. Layering N blits would re-distort the
    /// previous pass's output instead of summing displacements, and would
    /// cost N full-screen samples; the shader accumulates offsets and
    /// samples exactly once (see static/shockwave.fs).
    pub centers_loc: i32,
    pub times_loc: i32,
    pub gains_loc: i32,
    /// Where each ring starts and how fast it runs as a multiple of the
    /// speed (`static/shockwave.fs`): 0 and 1 for an outward ring, the
    /// reach and a negative pace for a gravity well's inward one.
    pub starts_loc: i32,
    pub signs_loc: i32,
    speed: f32,
    speed_loc: i32,
    width_loc: i32,
    strength_loc: i32,
    duration_loc: i32,
    /// The scene target's extent in the ripple frame (`Camera::ripple_view`).
    view_uv_loc: i32,
    view_uv_size_loc: i32,
    /// The world point the scene target's texture coordinate `(0, 0)`
    /// stands on this frame (`set_view`): every centre is measured from it.
    corner: Vec2,
}

/// The tuning knobs for one `RippleFx` instance, set at load and re-uploaded
/// by `set_tuning` whenever the live tuning table changes. Bundled into one struct (rather than four loose params) since every
/// effect - kill shockwave, muzzle flash, shell impact - supplies all four
/// together from its own block of constants in `lib.rs`.
pub struct RippleTuning {
    pub speed: f32,
    pub width: f32,
    pub strength: f32,
    pub duration: f32,
}

/// The fragment shader a `RippleFx` bends the scene with. Its source is
/// in the binary: the GLSL 330 file on the desktop, its GLSL ES 100 twin
/// in `static/web/` where `EMBEDDED` (the web, iOS, Android).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RippleShader {
    Shockwave,
    MuzzleFlash,
    Impact,
}

impl RippleShader {
    /// The file under `static/` (and `static/web/`).
    pub fn name(self) -> &'static str {
        match self {
            RippleShader::Shockwave => "shockwave.fs",
            RippleShader::MuzzleFlash => "muzzle_flash.fs",
            RippleShader::Impact => "impact.fs",
        }
    }

    fn source(self) -> &'static str {
        match (self, crate::EMBEDDED) {
            (RippleShader::Shockwave, false) => include_str!("../../static/shockwave.fs"),
            (RippleShader::Shockwave, true) => include_str!("../../static/web/shockwave.fs"),
            (RippleShader::MuzzleFlash, false) => include_str!("../../static/muzzle_flash.fs"),
            (RippleShader::MuzzleFlash, true) => include_str!("../../static/web/muzzle_flash.fs"),
            (RippleShader::Impact, false) => include_str!("../../static/impact.fs"),
            (RippleShader::Impact, true) => include_str!("../../static/web/impact.fs"),
        }
    }
}

impl RippleFx {
    /// A ripple's start (ripple units) and pace as a multiple of this
    /// effect's speed, for `static/shockwave.fs` (`Shockwave::ring`).
    pub fn start_and_sign(&self, shock: &crate::shockwave::Shockwave) -> (f32, f32) {
        shock.ring(self.speed, crate::tuning::tuning().well_form_seconds)
    }

    /// Compile `which` and set up the uniforms that never change after
    /// startup: the ripple frame (`shockwave::RIPPLE_FRAME`) and this
    /// instance's `tuning`. Every
    /// ripple effect (kill shockwave, muzzle flash, shell impact, ...) is its
    /// own `RippleFx`, and each may name its own fragment shader as
    /// well as its own tuning, so a new effect can look genuinely different
    /// rather than just differently timed. A driver that will not compile
    /// the source gets raylib's default shader in its place, which draws
    /// the scene as it is: the round plays without that ripple's bend, and
    /// the failure is said on screen (`render::target::trouble`).
    pub fn load(
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        which: RippleShader,
        tuning: RippleTuning,
    ) -> Self {
        let mut shader = match rl.load_shader_from_memory(thread, None, Some(which.source())) {
            Ok(shader) => shader,
            Err(e) => {
                crate::render::target::trouble(which.name(), &e.to_string());
                rl.load_shader_from_memory(thread, None, None).expect("raylib's default shader")
            }
        };

        let center_loc = shader.get_shader_location("center");
        let time_loc = shader.get_shader_location("time");
        // Resolve as "name[0]": GLSL array uniforms are reported that way
        // by some drivers and as the bare name by others, so ask for the
        // indexed form, which both accept.
        let centers_loc = shader.get_shader_location("centers[0]");
        let times_loc = shader.get_shader_location("times[0]");
        let gains_loc = shader.get_shader_location("gains[0]");
        let starts_loc = shader.get_shader_location("starts[0]");
        let signs_loc = shader.get_shader_location("signs[0]");
        let resolution_loc = shader.get_shader_location("resolution");
        let speed_loc = shader.get_shader_location("speed");
        let width_loc = shader.get_shader_location("width");
        let strength_loc = shader.get_shader_location("strength");
        let duration_loc = shader.get_shader_location("duration");
        let view_uv_loc = shader.get_shader_location("viewUv");
        let view_uv_size_loc = shader.get_shader_location("viewUvSize");

        shader.set_shader_value(resolution_loc, Vector2::new(RIPPLE_FRAME.0, RIPPLE_FRAME.1));
        shader.set_shader_value(speed_loc, tuning.speed);
        shader.set_shader_value(width_loc, tuning.width);
        shader.set_shader_value(strength_loc, tuning.strength);
        shader.set_shader_value(duration_loc, tuning.duration);
        // The standard field until a frame says otherwise (`set_view`); the
        // centres are measured from the target's corner, so `viewUv` stays
        // at the origin.
        shader.set_shader_value(view_uv_loc, Vector2::new(0.0, 0.0));
        shader.set_shader_value(view_uv_size_loc, Vector2::new(1.0, 1.0));
        // Every ring outward until a frame says otherwise.
        if starts_loc >= 0 && signs_loc >= 0 {
            shader.set_shader_value_v(starts_loc, &[0.0f32; crate::SHOCK_MAX]);
            shader.set_shader_value_v(signs_loc, &[1.0f32; crate::SHOCK_MAX]);
        }

        RippleFx {
            shader,
            center_loc,
            time_loc,
            centers_loc,
            times_loc,
            gains_loc,
            starts_loc,
            signs_loc,
            speed: tuning.speed,
            speed_loc,
            width_loc,
            strength_loc,
            duration_loc,
            view_uv_loc,
            view_uv_size_loc,
            corner: Vec2::new(0.0, RIPPLE_FRAME.1),
        }
    }

    /// Point the ripple at the part of the field `camera` shows: the scene
    /// target it samples is that view, its extent in the ripple frame
    /// (`Camera::ripple_view`), and the centres `uv_of` gives from here on
    /// are measured from its corner. Set every frame, before the blit.
    pub fn set_view(&mut self, camera: &Camera) {
        let (corner, size) = camera.ripple_view();
        self.corner = corner;
        self.shader.set_shader_value(self.view_uv_loc, Vector2::new(0.0, 0.0));
        self.shader.set_shader_value(self.view_uv_size_loc, Vector2::new(size.x, size.y));
    }

    /// A ripple's centre at the world position `pos`, in the frame
    /// `set_view` last put the shader in (`shockwave::ripple_uv`).
    pub fn uv_of(&self, pos: Position) -> Vector2 {
        ripple_uv(self.corner, pos).into()
    }

    /// Re-upload the tuning uniforms - called by `main.rs` whenever the
    /// live tuning table changes (`tuning::apply_pending`), so the `fx`
    /// knobs are live like everything else instead of fixed at load.
    pub fn set_tuning(&mut self, tuning: RippleTuning) {
        self.speed = tuning.speed;
        self.shader.set_shader_value(self.speed_loc, tuning.speed);
        self.shader.set_shader_value(self.width_loc, tuning.width);
        self.shader.set_shader_value(self.strength_loc, tuning.strength);
        self.shader.set_shader_value(self.duration_loc, tuning.duration);
    }
}

