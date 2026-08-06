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
}

/// The tuning knobs for one `RippleFx` instance, set once at load and fixed for
/// the run. Bundled into one struct (rather than four loose params) since every
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
        }
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
