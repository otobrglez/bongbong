//! The shader that draws a sprite's shape in one flat colour
//! (`static/solid.fs`, its GLSL ES 100 twin in `static/web/` on the
//! embedded builds): what `Canvas::solid` draws with on the GPU. A sprite
//! turned on its grid takes the same rule from the turn shader's `solid`
//! uniform (`render::turn`), so a silhouette covers exactly the sprite
//! under it.

use sola_raylib::ffi;
use sola_raylib::prelude::*;

/// The compiled shader.
pub struct SolidShader {
    shader: Shader,
}

/// What the shader is called when it cannot be made (`target::trouble`).
const SOLID_SHADER: &str = "solid shader";

impl SolidShader {
    /// Compile the shader. A driver that cannot is told on screen
    /// (`target::trouble`) and a silhouette is drawn as the sprite itself
    /// in the tint.
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Option<Self> {
        let source = if crate::EMBEDDED { include_str!("../../static/web/solid.fs") } else { include_str!("../../static/solid.fs") };
        match rl.load_shader_from_memory(thread, None, Some(source)) {
            Ok(shader) => Some(SolidShader { shader }),
            Err(e) => {
                crate::render::target::trouble(SOLID_SHADER, &e.to_string());
                None
            }
        }
    }

    /// Make this the shader the batch draws with, flushing what was
    /// queued. `close` must follow before anything else is drawn.
    pub fn open(&self) {
        // SAFETY: a plain raylib call on the render thread, which holds
        // the only `SolidShader`; the shader outlives the scope.
        unsafe { ffi::BeginShaderMode(*self.shader.as_ref()) }
    }

    /// Give the batch its default shader back, flushing the silhouettes
    /// queued.
    pub fn close(&self) {
        // SAFETY: closes the scope `open` opened.
        unsafe { ffi::EndShaderMode() }
    }
}
