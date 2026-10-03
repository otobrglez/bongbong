//! A larger draw batch for the GLES builds (the web, iOS, Android).
//! raylib's rlgl gathers every quad drawn into one vertex buffer and
//! flushes it as a draw call when it fills; built for OpenGL ES 2 it sizes
//! that buffer at 2048 quads, a quarter of the desktop's 8192, so a frame
//! of the lava's tiles, the weather's light fans and the effects'
//! blocks flushes four times as often there. [`RenderBatch::load`] gives
//! the window the desktop's size, which still fits ES 2's 16-bit indices
//! (four vertices a quad, 32768 in all), and costs under a megabyte.
//! The window holds it in its frame closure for as long as it draws.

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
        unsafe { ffi::rlSetRenderBatchActive(&mut *batch) };
        RenderBatch { batch }
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
