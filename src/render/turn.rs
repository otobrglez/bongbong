//! The raylib side of `turn.rs`: the shader that draws a sprite turned on
//! its design-pixel grid (`static/turn.vs`, `static/turn.fs`, the GLSL ES
//! 100 twins in `static/web/` on the embedded builds) and the quad it is
//! drawn on.
//!
//! `GpuCanvas` opens the shader for the first turned sprite it is handed
//! and closes it before anything else is drawn, so a run of turned
//! sprites is one shader scope. What differs from sprite to sprite - the
//! cell, the angle - rides in the quad's vertices (the batch's normal), so
//! the sprites of one sheet in a run are one draw call; only what a sheet
//! fixes is a uniform (`Fit`), set when the run moves to another sheet.

use sola_raylib::ffi;
use sola_raylib::prelude::*;

use crate::math::{Color, Vec2};
use crate::turn::Turned;

/// What the shader holds as uniforms: the same for every sprite of one
/// sheet drawn at one size about one pivot. A run of turned sprites is
/// flushed when it changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    texture: u32,
    /// One design pixel in texture coordinates.
    texel: Vec2,
    cell: Vec2,
    pivot: Vec2,
    corner: f32,
    shear: bool,
}

impl Fit {
    /// The uniforms for `turned` drawn from `texture`, a sheet of `block`
    /// texels to the design pixel: sheared, or sampled with its corners
    /// cut `corner` of the way.
    pub fn of(texture: &Texture2D, turned: &Turned, block: f32, shear: bool, corner: f32) -> Fit {
        Fit {
            texture: texture.id,
            texel: Vec2::new(block / texture.width.max(1) as f32, block / texture.height.max(1) as f32),
            cell: turned.cell,
            pivot: turned.pivot,
            corner,
            shear,
        }
    }
}

/// The compiled shader and its uniform locations.
pub struct TurnShader {
    shader: Shader,
    texel: i32,
    cell: i32,
    pivot: i32,
    corner: i32,
    shear: i32,
}

/// What the shader is called when it cannot be made (`target::trouble`).
const TURN_SHADER: &str = "turn shader";

impl TurnShader {
    /// Compile the shader. A driver that cannot is told on screen
    /// (`target::trouble`) and every sprite turns the plain way.
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Option<Self> {
        let (vs, fs) = if crate::EMBEDDED {
            (include_str!("../../static/web/turn.vs"), include_str!("../../static/web/turn.fs"))
        } else {
            (include_str!("../../static/turn.vs"), include_str!("../../static/turn.fs"))
        };
        let made = rl.load_shader_from_memory(thread, Some(vs), Some(fs)).map_err(|e| e.to_string()).and_then(|shader| {
            let loc = |name: &str| shader.get_shader_location(name);
            let (texel, cell, pivot, corner, shear) = (loc("texel"), loc("cell"), loc("pivot"), loc("corner"), loc("shear"));
            // raylib hands back its default shader's locations for a
            // source that did not compile: none of these is there.
            if texel < 0 || cell < 0 || pivot < 0 || corner < 0 || shear < 0 {
                return Err("did not compile".to_string());
            }
            Ok(TurnShader { shader, texel, cell, pivot, corner, shear })
        });
        match made {
            Ok(shader) => Some(shader),
            Err(e) => {
                crate::render::target::trouble(TURN_SHADER, &e);
                None
            }
        }
    }

    /// Make this the shader the batch draws with, flushing what was
    /// queued. `close` must follow before anything but `draw` is drawn.
    pub fn open(&self) {
        // SAFETY: a plain raylib call on the render thread, which holds
        // the only `TurnShader`; the shader outlives the scope.
        unsafe { ffi::BeginShaderMode(*self.shader.as_ref()) }
    }

    /// Give the batch its default shader back, flushing the turned
    /// sprites queued.
    pub fn close(&self) {
        // SAFETY: closes the scope `open` opened.
        unsafe { ffi::EndShaderMode() }
    }

    /// Set what `fit` fixes, the queued sprites drawn first: they were
    /// queued under the uniforms before.
    pub fn fit(&self, fit: &Fit) {
        let raw = *self.shader.as_ref();
        let vec2 = ffi::ShaderUniformDataType::SHADER_UNIFORM_VEC2 as i32;
        let float = ffi::ShaderUniformDataType::SHADER_UNIFORM_FLOAT as i32;
        let pair = |v: Vec2| [v.x, v.y];
        // SAFETY: each pointer is to a local of the type the uniform
        // names, read before the call returns.
        unsafe {
            ffi::rlDrawRenderBatchActive();
            ffi::SetShaderValue(raw, self.texel, pair(fit.texel).as_ptr().cast(), vec2);
            ffi::SetShaderValue(raw, self.cell, pair(fit.cell).as_ptr().cast(), vec2);
            ffi::SetShaderValue(raw, self.pivot, pair(fit.pivot).as_ptr().cast(), vec2);
            ffi::SetShaderValue(raw, self.corner, [fit.corner].as_ptr().cast(), float);
            ffi::SetShaderValue(raw, self.shear, [if fit.shear { 1.0f32 } else { 0.0 }].as_ptr().cast(), float);
        }
    }

    /// Queue `turned`'s quad from `texture` in `tint`, inside `open` and
    /// under its sheet's `fit`.
    pub fn draw(&self, texture: &Texture2D, turned: &Turned, tint: Color) {
        // SAFETY: rlgl's immediate mode between the frame's begin and end,
        // the way `DrawTexturePro` queues its own quad.
        unsafe {
            ffi::rlSetTexture(texture.id);
            ffi::rlBegin(ffi::RL_QUADS as i32);
            ffi::rlColor4ub(tint.r, tint.g, tint.b, tint.a);
            ffi::rlNormal3f(turned.cell_corner.x, turned.cell_corner.y, turned.radians);
            for (world, design) in turned.corners {
                ffi::rlTexCoord2f(design.x, design.y);
                ffi::rlVertex2f(world.x, world.y);
            }
            ffi::rlEnd();
            ffi::rlSetTexture(0);
        }
    }
}
