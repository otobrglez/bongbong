//! A larger draw batch for the GLES builds (the web, iOS, Android).
//! raylib's rlgl gathers every quad drawn into one vertex buffer and
//! flushes it as a draw call when it fills; built for OpenGL ES 2 it sizes
//! that buffer at 2048 quads, a quarter of the desktop's 8192, so a frame
//! of the lava's tiles, the weather's light fans and the effects'
//! blocks flushes four times as often there. [`RenderBatch::load`] gives
//! the window the desktop's size, which still fits ES 2's 16-bit indices
//! (four vertices a quad, 32768 in all), and costs under a megabyte.
//! The window holds it (`app.rs`'s `App`) for as long as it draws.

use sola_raylib::ffi;

/// The quads one batch holds: the desktop build's own default.
pub const BATCH_QUADS: i32 = 8192;

/// A render batch rlgl draws through in place of its own while this is
/// held. Dropping it flushes it, gives rlgl its default back and frees
/// it, so it must go before the window closes.
pub struct RenderBatch {
    batch: Box<ffi::rlRenderBatch>,
}

impl RenderBatch {
    /// Load a batch of [`BATCH_QUADS`] and make it the one rlgl draws
    /// through. Call with the window open.
    pub fn load() -> Self {
        // rlgl keeps a pointer to the active batch, so it lives in a box
        // that does not move.
        let mut batch = Box::new(unsafe { ffi::rlLoadRenderBatch(1, BATCH_QUADS) });
        // SAFETY: the batch just loaded holds `bufferCount` (1) vertex
        // buffers, each with its vertex array and its normals' buffer.
        unsafe {
            for i in 0..batch.bufferCount.max(0) as usize {
                bind_normals(&*batch.vertexBuffer.add(i));
            }
            ffi::rlSetRenderBatchActive(&mut *batch);
        }
        RenderBatch { batch }
    }
}

/// The vertex attribute rlgl binds a batch's normals to
/// (`RL_DEFAULT_SHADER_ATTRIB_LOCATION_NORMAL`).
const NORMAL_ATTRIBUTE: u32 = 2;

/// Point `buffer`'s vertex array at its normals. rlgl binds a batch's
/// normals to the location the current shader gives them, and outside
/// its own start-up that is the default shader's none, so a batch loaded
/// later carries normals no shader can read - and a turned sprite's cell
/// and angle ride in them (`render::turn`). A driver without vertex
/// arrays binds every attribute at each draw, by the drawing shader's
/// locations, and needs nothing here.
///
/// # Safety
/// `buffer` is a vertex buffer of a batch rlgl loaded, on the render
/// thread.
unsafe fn bind_normals(buffer: &ffi::rlVertexBuffer) {
    unsafe {
        if !ffi::rlEnableVertexArray(buffer.vaoId) {
            return;
        }
        ffi::rlEnableVertexBuffer(buffer.vboId[2]);
        ffi::rlSetVertexAttribute(NORMAL_ATTRIBUTE, 3, ffi::RL_FLOAT as i32, false, 0, 0);
        ffi::rlEnableVertexAttribute(NORMAL_ATTRIBUTE);
        ffi::rlDisableVertexBuffer();
        ffi::rlDisableVertexArray();
    }
}

impl Drop for RenderBatch {
    fn drop(&mut self) {
        unsafe {
            ffi::rlSetRenderBatchActive(std::ptr::null_mut());
            ffi::rlUnloadRenderBatch(*self.batch);
        }
    }
}
