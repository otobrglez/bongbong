//! The GPU side of the ripple effects: the compiled shader and its
//! uniforms (`shockwave.rs` owns `Shockwave`, the ripple in flight).

use sola_raylib::prelude::*;

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
    speed_loc: i32,
    width_loc: i32,
    strength_loc: i32,
    duration_loc: i32,
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

impl RippleFx {
    /// Compile `shader_path` and set up the uniforms that never change after
    /// startup: the screen resolution and this instance's `tuning`. Every
    /// ripple effect (kill shockwave, muzzle flash, shell impact, ...) is its
    /// own `RippleFx`, and each may point at its own fragment shader file as
    /// well as its own tuning, so a new effect can look genuinely different
    /// rather than just differently timed.
    pub fn load(
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        shader_path: &str,
        screen_width: i32,
        screen_height: i32,
        tuning: RippleTuning,
    ) -> Self {
        #[cfg(not(target_os = "android"))]
        let mut shader = rl
            .load_shader(thread, None, Some(shader_path))
            .expect("failed loading ripple shader");
        // Android keeps its assets inside the APK, where raylib's own
        // loaders see them but the wrapper's `load_shader` does not (it
        // checks the path with `std::fs` first); the three ripple shaders
        // are small, so the GLSL ES 100 ports travel in the binary instead.
        #[cfg(target_os = "android")]
        let mut shader = {
            let source = match shader_path.rsplit('/').next() {
                Some("shockwave.fs") => include_str!("../../static/web/shockwave.fs"),
                Some("muzzle_flash.fs") => include_str!("../../static/web/muzzle_flash.fs"),
                Some("impact.fs") => include_str!("../../static/web/impact.fs"),
                other => panic!("no embedded ripple shader for {other:?}"),
            };
            rl.load_shader_from_memory(thread, None, Some(source))
                .expect("failed compiling the embedded ripple shader")
        };

        let center_loc = shader.get_shader_location("center");
        let time_loc = shader.get_shader_location("time");
        // Resolve as "name[0]": GLSL array uniforms are reported that way
        // by some drivers and as the bare name by others, so ask for the
        // indexed form, which both accept.
        let centers_loc = shader.get_shader_location("centers[0]");
        let times_loc = shader.get_shader_location("times[0]");
        let gains_loc = shader.get_shader_location("gains[0]");
        let resolution_loc = shader.get_shader_location("resolution");
        let speed_loc = shader.get_shader_location("speed");
        let width_loc = shader.get_shader_location("width");
        let strength_loc = shader.get_shader_location("strength");
        let duration_loc = shader.get_shader_location("duration");

        shader.set_shader_value(
            resolution_loc,
            Vector2::new(screen_width as f32, screen_height as f32),
        );
        shader.set_shader_value(speed_loc, tuning.speed);
        shader.set_shader_value(width_loc, tuning.width);
        shader.set_shader_value(strength_loc, tuning.strength);
        shader.set_shader_value(duration_loc, tuning.duration);

        RippleFx {
            shader,
            center_loc,
            time_loc,
            centers_loc,
            times_loc,
            gains_loc,
            speed_loc,
            width_loc,
            strength_loc,
            duration_loc,
        }
    }

    /// Re-upload the tuning uniforms - called by `main.rs` whenever the
    /// live tuning table changes (`tuning::apply_pending`), so the `fx`
    /// knobs are live like everything else instead of fixed at load.
    pub fn set_tuning(&mut self, tuning: RippleTuning) {
        self.shader.set_shader_value(self.speed_loc, tuning.speed);
        self.shader.set_shader_value(self.width_loc, tuning.width);
        self.shader.set_shader_value(self.strength_loc, tuning.strength);
        self.shader.set_shader_value(self.duration_loc, tuning.duration);
    }
}

/// Convert a screen-space pixel position into the UV space the ripple shader
/// actually samples in. raylib stores a `RenderTexture2D`'s pixels vertically
/// flipped relative to a loaded image, so `Game::render` blits `scene_target`
/// through a negative-height source rect to undo that - which also flips the
/// fragment shader's `fragTexCoord` relative to plain top-down screen space.
/// Any ripple `center` has to be flipped the same way to land on the point on
/// screen that `pos` actually names.
pub fn screen_to_ripple_uv(pos: Position, screen_width: f32, screen_height: f32) -> Vector2 {
    Vector2::new(pos.x / screen_width, 1.0 - pos.y / screen_height)
}
