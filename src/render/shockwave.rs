//! The GPU side of everything that bends the picture: the one compiled
//! shader and its uniforms (`shockwave.rs` owns `Shockwave`, the ripple in
//! flight; the muzzle and impact flashes are `Game`'s lists).

use sola_raylib::prelude::*;

use crate::math::Vec2;
use crate::shockwave::{ripple_uv, Shockwave, RIPPLE_FRAME};
use crate::tuning::Tuning;
use crate::view::Camera;
use crate::Position;

/// The muzzle puffs and the impact punches the shader bends the scene by
/// at once, each (`static/shockwave.fs`'s array lengths): the newest win.
pub const FLASH_MAX: usize = 4;

/// The fragment shader's name under `static/` (and `static/web/`), for
/// `target::trouble`.
const SHADER: &str = "shockwave.fs";

/// The four knobs of one kind of bend - a blast's ripple, a muzzle's puff,
/// an impact's punch: in the standard field's heights (`RIPPLE_FRAME`) and
/// seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RippleTuning {
    pub speed: f32,
    pub width: f32,
    pub strength: f32,
    pub duration: f32,
}

impl RippleTuning {
    fn packed(self) -> Vector4 {
        Vector4::new(self.speed, self.width, self.strength, self.duration)
    }
}

/// The knobs of all three kinds of bend, from the tuning table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ripples {
    pub shock: RippleTuning,
    pub puff: RippleTuning,
    pub punch: RippleTuning,
}

impl Ripples {
    /// The `shockwave_*`, `muzzle_flash_*` and `impact_flash_*` rows of
    /// `t`, a ripple's strength scaled by `screen_fx_intensity`.
    pub fn of(t: &Tuning) -> Self {
        Ripples {
            shock: RippleTuning { speed: t.shockwave_speed, width: t.shockwave_width, strength: t.shockwave_strength * t.screen_fx_intensity, duration: t.shockwave_duration },
            puff: RippleTuning { speed: t.muzzle_flash_speed, width: t.muzzle_flash_width, strength: t.muzzle_flash_strength, duration: t.muzzle_flash_duration },
            punch: RippleTuning { speed: t.impact_flash_speed, width: t.impact_flash_width, strength: t.impact_flash_strength, duration: t.impact_flash_duration },
        }
    }
}

/// `static/shockwave.fs` compiled, with its uniform locations: the pass
/// the scene goes through on its way out of the scene target whenever a
/// ripple, a muzzle flash or an impact is live. Every bend is summed there
/// and the scene sampled once, moved by whole 2 px blocks, through the
/// same source rectangle the camera's shake moves - so a muzzle's puff
/// shakes with the picture it bends. The source is in the binary: the
/// GLSL 330 file on the desktop, its GLSL ES 100 twin in `static/web/`
/// where `EMBEDDED` (the web, iOS, Android).
pub struct RippleFx {
    pub shader: Shader,
    /// Array uniforms: every live bend has to resolve in a single pass.
    /// Layering N blits would re-distort the previous pass's output
    /// instead of summing displacements, and would cost N full-screen
    /// samples (see static/shockwave.fs).
    centers_loc: i32,
    times_loc: i32,
    gains_loc: i32,
    /// Where each ring starts and how fast it runs as a multiple of the
    /// speed (`static/shockwave.fs`): 0 and 1 for an outward ring, the
    /// reach and a negative pace for a gravity well's inward one.
    starts_loc: i32,
    signs_loc: i32,
    puffs_loc: i32,
    puff_times_loc: i32,
    puff_loc: i32,
    punches_loc: i32,
    punch_times_loc: i32,
    punch_loc: i32,
    tuning: Ripples,
    speed_loc: i32,
    width_loc: i32,
    strength_loc: i32,
    duration_loc: i32,
    /// The scene target's extent in the ripple frame (`Camera::ripple_view`).
    view_uv_loc: i32,
    view_uv_size_loc: i32,
    target_size_loc: i32,
    /// The world point the scene target's texture coordinate `(0, 0)`
    /// stands on this frame (`set_view`): every centre is measured from it.
    corner: Vec2,
}

impl RippleFx {
    /// Compile the shader and set up the uniforms that never change after
    /// startup - the ripple frame (`shockwave::RIPPLE_FRAME`) - and
    /// `tuning`. A driver that will not compile the source gets raylib's
    /// default shader in its place, which draws the scene as it is: the
    /// round plays without a bend, and the failure is said on screen
    /// (`render::target::trouble`).
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread, tuning: Ripples) -> Self {
        let source = if crate::EMBEDDED { include_str!("../../static/web/shockwave.fs") } else { include_str!("../../static/shockwave.fs") };
        let mut shader = match rl.load_shader_from_memory(thread, None, Some(source)) {
            Ok(shader) => shader,
            Err(e) => {
                crate::render::target::trouble(SHADER, &e.to_string());
                rl.load_shader_from_memory(thread, None, None).expect("raylib's default shader")
            }
        };

        // Resolve as "name[0]": GLSL array uniforms are reported that way
        // by some drivers and as the bare name by others, so ask for the
        // indexed form, which both accept.
        let resolution_loc = shader.get_shader_location("resolution");
        shader.set_shader_value(resolution_loc, Vector2::new(RIPPLE_FRAME.0, RIPPLE_FRAME.1));
        let mut fx = RippleFx {
            centers_loc: shader.get_shader_location("centers[0]"),
            times_loc: shader.get_shader_location("times[0]"),
            gains_loc: shader.get_shader_location("gains[0]"),
            starts_loc: shader.get_shader_location("starts[0]"),
            signs_loc: shader.get_shader_location("signs[0]"),
            puffs_loc: shader.get_shader_location("puffs[0]"),
            puff_times_loc: shader.get_shader_location("puffTimes[0]"),
            puff_loc: shader.get_shader_location("puff"),
            punches_loc: shader.get_shader_location("punches[0]"),
            punch_times_loc: shader.get_shader_location("punchTimes[0]"),
            punch_loc: shader.get_shader_location("punch"),
            tuning,
            speed_loc: shader.get_shader_location("speed"),
            width_loc: shader.get_shader_location("width"),
            strength_loc: shader.get_shader_location("strength"),
            duration_loc: shader.get_shader_location("duration"),
            view_uv_loc: shader.get_shader_location("viewUv"),
            view_uv_size_loc: shader.get_shader_location("viewUvSize"),
            target_size_loc: shader.get_shader_location("targetSize"),
            shader,
            corner: Vec2::new(0.0, RIPPLE_FRAME.1),
        };
        fx.set_tuning(tuning);
        // The standard field until a frame says otherwise (`set_view`); the
        // centres are measured from the target's corner, so `viewUv` stays
        // at the origin.
        fx.shader.set_shader_value(fx.view_uv_loc, Vector2::new(0.0, 0.0));
        fx.shader.set_shader_value(fx.view_uv_size_loc, Vector2::new(1.0, 1.0));
        fx.shader.set_shader_value(fx.target_size_loc, Vector2::new(RIPPLE_FRAME.0, RIPPLE_FRAME.1));
        fx.set_bends(&[], std::iter::empty(), std::iter::empty());
        fx
    }

    /// Point the shader at the part of the field `camera` shows, held in a
    /// scene target of `texels`: that view's extent in the ripple frame
    /// (`Camera::ripple_view`), and the centres `uv_of` gives from here on
    /// are measured from its corner. Set every frame, before the blit.
    pub fn set_view(&mut self, camera: &Camera, texels: (i32, i32)) {
        let (corner, size) = camera.ripple_view();
        self.corner = corner;
        self.shader.set_shader_value(self.view_uv_loc, Vector2::new(0.0, 0.0));
        self.shader.set_shader_value(self.view_uv_size_loc, Vector2::new(size.x, size.y));
        self.shader.set_shader_value(self.target_size_loc, Vector2::new(texels.0.max(1) as f32, texels.1.max(1) as f32));
    }

    /// A bend's centre at the world position `pos`, in the frame
    /// `set_view` last put the shader in (`shockwave::ripple_uv`).
    pub fn uv_of(&self, pos: Position) -> Vector2 {
        ripple_uv(self.corner, pos).into()
    }

    /// Upload this frame's bends, after `set_view`: the round's ripples
    /// (the first `SHOCK_MAX`), and the newest `FLASH_MAX` of the muzzle
    /// `puffs` and of the impact `punches`, each where it is and the
    /// seconds since it started. An unused slot adds nothing: a ripple's
    /// gain is 0, a flash's time is its whole duration.
    pub fn set_bends(&mut self, shocks: &[Shockwave], puffs: impl Iterator<Item = (Position, f32)>, punches: impl Iterator<Item = (Position, f32)>) {
        let mut centers = [Vector2::new(0.0, 0.0); crate::SHOCK_MAX];
        let mut times = [0.0f32; crate::SHOCK_MAX];
        let mut gains = [0.0f32; crate::SHOCK_MAX];
        let mut starts = [0.0f32; crate::SHOCK_MAX];
        let mut signs = [1.0f32; crate::SHOCK_MAX];
        let well = crate::tuning::tuning().well_form_seconds;
        for (i, shock) in shocks.iter().take(crate::SHOCK_MAX).enumerate() {
            centers[i] = self.uv_of(shock.center);
            times[i] = shock.time;
            gains[i] = shock.strength;
            (starts[i], signs[i]) = shock.ring(self.tuning.shock.speed, well);
        }
        self.shader.set_shader_value_v(self.centers_loc, &centers);
        self.shader.set_shader_value_v(self.times_loc, &times);
        self.shader.set_shader_value_v(self.gains_loc, &gains);
        self.shader.set_shader_value_v(self.starts_loc, &starts);
        self.shader.set_shader_value_v(self.signs_loc, &signs);
        let (at, age) = self.newest(puffs, self.tuning.puff.duration);
        self.shader.set_shader_value_v(self.puffs_loc, &at);
        self.shader.set_shader_value_v(self.puff_times_loc, &age);
        let (at, age) = self.newest(punches, self.tuning.punch.duration);
        self.shader.set_shader_value_v(self.punches_loc, &at);
        self.shader.set_shader_value_v(self.punch_times_loc, &age);
    }

    /// The newest `FLASH_MAX` of `flashes` as the shader's two arrays:
    /// centres in the ripple frame, and ages, `duration` in every slot
    /// left over.
    fn newest(&self, flashes: impl Iterator<Item = (Position, f32)>, duration: f32) -> ([Vector2; FLASH_MAX], [f32; FLASH_MAX]) {
        let mut live: Vec<(Position, f32)> = flashes.filter(|&(_, age)| age < duration).collect();
        live.sort_by(|a, b| a.1.total_cmp(&b.1));
        let mut at = [Vector2::new(0.0, 0.0); FLASH_MAX];
        let mut age = [duration; FLASH_MAX];
        for (i, (pos, time)) in live.into_iter().take(FLASH_MAX).enumerate() {
            at[i] = self.uv_of(pos);
            age[i] = time;
        }
        (at, age)
    }

    /// Re-upload the tuning uniforms - called by `app.rs` whenever the
    /// live tuning table changes (`tuning::apply_pending`), so the `fx`
    /// knobs are live like everything else instead of fixed at load.
    pub fn set_tuning(&mut self, tuning: Ripples) {
        self.tuning = tuning;
        self.shader.set_shader_value(self.speed_loc, tuning.shock.speed);
        self.shader.set_shader_value(self.width_loc, tuning.shock.width);
        self.shader.set_shader_value(self.strength_loc, tuning.shock.strength);
        self.shader.set_shader_value(self.duration_loc, tuning.shock.duration);
        self.shader.set_shader_value(self.puff_loc, tuning.puff.packed());
        self.shader.set_shader_value(self.punch_loc, tuning.punch.packed());
    }
}
