//! The screen a local round ended on, dissolving into the next round's
//! (docs/effects.md, "Between rounds"): kept in a render texture on every
//! frame the end screen - or the level select over a round - stands
//! (`Session::keeps_picture`), then drawn over everything on the new
//! round's first frames through `static/dissolve.fs`, which drops it block
//! by block through the 4x4 Bayer pattern (`pyro::bayer`'s matrix) as
//! `Session::dissolve` runs. Both screens carry the same dim - the end
//! screen's under its lifted verdict, the mission banner's - so the
//! brightness holds and nothing goes black. The source is compiled into
//! the binary (its GLSL ES 100 twin in `static/web/` on the embedded
//! builds). Without the shader, or with nothing kept on the frame before,
//! the new round cuts in.

use sola_raylib::prelude::*;

use crate::math::{Color, Rectangle};

/// The dissolve's shader and the kept screen.
pub struct Dissolve {
    /// The compiled shader and its `progress` and `blockPx` locations;
    /// `None` where the driver refused it.
    shader: Option<(Shader, i32, i32)>,
    /// The kept screen and its size, the framebuffer's, in a target that
    /// holds it (`render::target::stepped`): made on the first keep and
    /// remade when the window's size leaves the target's step.
    kept: Option<(RenderTexture2D, (i32, i32))>,
    /// `kept` holds the screen of the frame before: set by a keeping
    /// frame, cleared by one that neither keeps nor dissolves, so a round
    /// that starts long after the last end screen never dissolves from it.
    fresh: bool,
}

impl Dissolve {
    /// Compile the shader. A failure is not fatal: every new round then
    /// cuts in.
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        let source = if crate::EMBEDDED { include_str!("../../static/web/dissolve.fs") } else { include_str!("../../static/dissolve.fs") };
        let shader = match rl.load_shader_from_memory(thread, None, Some(source)) {
            Ok(s) => {
                let (progress, block) = (s.get_shader_location("progress"), s.get_shader_location("blockPx"));
                Some((s, progress, block))
            }
            Err(e) => {
                eprintln!("[render] dissolve.fs unavailable, new rounds cut in: {e}");
                None
            }
        };
        Dissolve { shader, kept: None, fresh: false }
    }

    /// The texture to keep this frame's screen in, in the top-left `size`
    /// framebuffer pixels of it, made or remade to hold them; `None`
    /// without the shader, which would leave nothing to draw it with.
    pub fn keep(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, size: (i32, i32)) -> Option<&mut RenderTexture2D> {
        self.shader.as_ref()?;
        let size = (size.0.max(1), size.1.max(1));
        let held = crate::render::target::stepped(size);
        let fits = self.kept.as_ref().is_some_and(|(t, _)| (t.texture.width, t.texture.height) == held);
        if !fits {
            self.kept = match crate::render::target::load(rl, thread, held) {
                Ok(target) => Some((target, size)),
                Err(e) => {
                    eprintln!("[render] dissolve: no texture to keep the screen in: {e}");
                    None
                }
            };
        }
        if let Some((_, kept)) = self.kept.as_mut() {
            *kept = size;
        }
        self.fresh = self.kept.is_some();
        self.kept.as_mut().map(|(target, _)| target)
    }

    /// A frame that neither keeps the screen nor dissolves: what is kept
    /// is no longer the screen a new round starts from.
    pub fn forget(&mut self) {
        self.fresh = false;
    }

    /// The kept screen over everything on the window, `progress` (0 to 1)
    /// of the way dissolved, in blocks `block_px` framebuffer pixels a
    /// side; `window` is the window in its own units, the space a direct
    /// draw is in (raylib scales it onto the framebuffer). Nothing unless a
    /// screen was kept on the frame before the round started.
    pub fn draw<D: RaylibDraw + RaylibShaderModeExt>(&mut self, d: &mut D, progress: f32, block_px: f32, window: (f32, f32)) {
        let (Some((shader, at_progress, at_block)), Some((kept, size))) = (self.shader.as_mut(), self.kept.as_ref()) else { return };
        if !self.fresh {
            return;
        }
        shader.set_shader_value(*at_progress, progress.clamp(0.0, 1.0));
        shader.set_shader_value(*at_block, block_px.max(1.0));
        let source = crate::render::target::filled(kept, (size.0 as f32, size.1 as f32));
        let dest = Rectangle::new(0.0, 0.0, window.0, window.1);
        d.draw_shader_mode(crate::frame_stages::shader(shader), |mut s| s.draw_texture_pro(kept, source, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE));
    }
}

#[cfg(test)]
mod tests {
    /// The shader's threshold is `pyro::bayer`'s: its 2x2 recursion,
    /// written out here as the GLSL computes it, gives the same 4x4 matrix.
    #[test]
    fn the_shader_dissolves_through_pyros_bayer_pattern() {
        let bayer2 = |x: f32, y: f32| 2.0 * x + 3.0 * y - 4.0 * x * y;
        for by in 0..4 {
            for bx in 0..4 {
                let (x, y) = (bx as f32, by as f32);
                let m = 4.0 * bayer2(x % 2.0, y % 2.0) + bayer2((x / 2.0).floor() % 2.0, (y / 2.0).floor() % 2.0);
                assert_eq!((m + 0.5) / 16.0, crate::pyro::bayer(bx, by), "block ({bx}, {by})");
            }
        }
    }
}
