use sola_raylib::prelude::*;

use crate::Position;

/// A ripple effect in flight: a radial-distortion ring expanding from
/// `center`, `time` seconds after it started. Shared shape for both ripple
/// effects in the game: the full-screen kill shockwave (`Game::shock`, at
/// most one at a time) and the small, split-second muzzle-flash heat haze
/// (`Game::muzzle_flashes`, one per shot fired). See `Game::render`.
pub struct Shockwave {
    /// Hit point in world/screen pixels (the game has no camera transform, so
    /// world space and screen space are the same thing).
    pub center: Position,
    /// Seconds since the ripple was triggered.
    pub time: f32,
    /// How hard this one hits, as a multiple of `shockwave_strength` and
    /// `camera_shake_magnitude`. 1.0 is a tank dying; a fence collapsing
    /// has no business shaking the screen as hard as that.
    pub strength: f32,
}

impl Shockwave {
    /// A ripple at full strength.
    pub fn new(center: Position) -> Self {
        Shockwave { center, time: 0.0, strength: 1.0 }
    }

    /// A ripple scaled against a tank kill, which is the 1.0 reference.
    pub fn scaled(center: Position, strength: f32) -> Self {
        Shockwave { center, time: 0.0, strength }
    }

    /// Punch left in it: strength faded by how much of its life is gone.
    /// `Game::finish_frame` evicts by this rather than by age, so a barrel
    /// cascade's little fuse pops cannot shove out the tank explosion that
    /// set them off.
    pub fn remaining(&self) -> f32 {
        let left = 1.0 - (self.time / crate::tuning::tuning().shockwave_duration).clamp(0.0, 1.0);
        self.strength * left
    }
}

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
        let mut shader = rl
            .load_shader(thread, None, Some(shader_path))
            .expect("failed loading ripple shader");

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
