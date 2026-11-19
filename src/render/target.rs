//! Render targets and sheets made so that a failure shows rather than
//! ends the game: `load` is the one place a render target is made (its
//! wrap clamped, so a blit that reads past its edge - a shaken scene, a
//! ripple's bend - takes the edge texel on every platform, where the
//! default repeats on the desktop and clamps on GL ES 2), `sheet` the one
//! place a sheet is loaded (a missing or unreadable one is a checkered
//! placeholder), and `trouble` keeps what went wrong for the frame to say
//! on screen (`draw_trouble`) as well as on stderr.
//!
//! A target the window sizes is asked for in steps (`stepped`): a window
//! dragged by its edge then makes it again once every `STEP` pixels, not
//! every frame, and whoever draws it reads only the part it filled.

use std::sync::Mutex;

use sola_raylib::prelude::*;

use crate::math::Color;
use crate::text::keys;

/// The step a window-sized target's sides are rounded up to (`stepped`).
pub const STEP: i32 = 64;

/// `size` with each side rounded up to a whole `STEP`: what a target is
/// made at to hold a picture `size` pixels, the picture in its top-left
/// corner.
pub fn stepped(size: (i32, i32)) -> (i32, i32) {
    let side = |v: i32| ((v.max(1) + STEP - 1) / STEP) * STEP;
    (side(size.0), side(size.1))
}

/// A render target `size` texels, its wrap clamped. An error names the
/// size; whoever asked decides what the frame does without it.
pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread, size: (i32, i32)) -> Result<RenderTexture2D, String> {
    let (w, h) = (size.0.max(1), size.1.max(1));
    let target = rl.load_render_texture(thread, w as u32, h as u32).map_err(|e| format!("render target {w}x{h}: {e}"))?;
    target.set_texture_wrap(thread, TextureWrap::TEXTURE_WRAP_CLAMP);
    Ok(target)
}

/// The texels of a target `held` tall that a picture `size` pixels drawn
/// into its top-left corner fills, as a blit's source: a render texture
/// reads back bottom-up, so the rows count from the bottom and the
/// negative height flips them on the way out.
pub fn filled(held: &RenderTexture2D, size: (f32, f32)) -> Rectangle {
    Rectangle::new(0.0, held.texture.height as f32 - size.1, size.0, -size.1)
}

/// The sheet at `path`, or - where the file is missing or will not decode
/// - a checkered placeholder that tiles, with the failure kept for the
/// screen (`trouble`). A draw from a missing sheet then shows checks where
/// its sprite would be, and the round plays on.
pub fn sheet(rl: &mut RaylibHandle, thread: &RaylibThread, path: &str) -> Texture2D {
    match rl.load_texture(thread, path) {
        Ok(texture) => texture,
        Err(e) => sheet_failed(rl, thread, path, &e.to_string()),
    }
}

/// What stands in for the sheet at `path` that failed to load with
/// `detail`: the placeholder, the failure kept for the screen.
pub fn sheet_failed(rl: &mut RaylibHandle, thread: &RaylibThread, path: &str, detail: &str) -> Texture2D {
    trouble(path, detail);
    placeholder(rl, thread)
}

/// The placeholder a missing sheet is drawn as: `PLACEHOLDER_PX` square,
/// a power of two so that it may repeat on GL ES 2 as well, checked in two
/// colours no sheet uses.
fn placeholder(rl: &mut RaylibHandle, thread: &RaylibThread) -> Texture2D {
    let checks = PLACEHOLDER_PX / 8;
    let image = Image::gen_image_checked(PLACEHOLDER_PX, PLACEHOLDER_PX, checks, checks, Color::new(255, 0, 220, 255), Color::new(24, 24, 28, 255));
    let texture = rl.load_texture_from_image(thread, &image).expect("a 64 px texture can be made wherever the window was");
    texture.set_texture_wrap(thread, TextureWrap::TEXTURE_WRAP_REPEAT);
    texture
}

const PLACEHOLDER_PX: i32 = 64;

/// What could not be made or loaded since the window opened, each thing
/// once, oldest first.
static TROUBLE: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The most lines `draw_trouble` shows.
const TROUBLE_LINES: usize = 4;

/// Note that `what` - a sheet's path, a render target's use - failed with
/// `detail`: said once on stderr with the detail, and kept by name for
/// the screen.
pub fn trouble(what: &str, detail: &str) {
    let mut kept = TROUBLE.lock().unwrap_or_else(|e| e.into_inner());
    if !kept.iter().any(|k| k == what) {
        eprintln!("[render] {what}: {detail}");
        kept.push(what.to_string());
    }
}

/// `what` works again: its line leaves the screen.
pub fn recovered(what: &str) {
    TROUBLE.lock().unwrap_or_else(|e| e.into_inner()).retain(|k| k != what);
}

/// Whether anything is in trouble.
pub fn any_trouble() -> bool {
    !TROUBLE.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
}

/// The lines `draw_trouble` writes, in the language on screen.
pub fn trouble_lines() -> Vec<String> {
    let kept = TROUBLE.lock().unwrap_or_else(|e| e.into_inner());
    let t = crate::text::text();
    kept.iter()
        .take(TROUBLE_LINES)
        .map(|what| {
            // A path says more by its file's name than by its folders.
            let name = what.rsplit('/').next().unwrap_or(what);
            t.fmt(keys::RENDER_TROUBLE, &[("name", name.into())])
        })
        .collect()
}

/// What is in trouble, written up from the bottom-left corner of an area
/// `size` across in the units `d` draws in, the text `px` tall: nothing
/// while everything loaded. Over whatever the frame drew, so a missing
/// sheet or a target that could not be made is said where the player
/// looks.
pub fn draw_trouble(d: &mut impl RaylibDraw, size: (f32, f32), px: f32) {
    let lines = trouble_lines();
    if lines.is_empty() {
        return;
    }
    let px = px.round().max(8.0) as i32;
    let pad = px / 2;
    let line = px + pad / 2;
    let top = size.1 as i32 - pad - line * lines.len() as i32;
    for (i, text) in lines.iter().enumerate() {
        let text = crate::text::fold(text);
        let (x, y) = (pad, top + line * i as i32);
        let w = crate::text::width(&text, px);
        d.draw_rectangle(x - pad / 2, y - pad / 4, w + pad, line, Color::new(20, 20, 24, 255));
        d.draw_text(&text, x, y, px, Color::new(255, 96, 96, 255));
    }
}

/// The whole frame when the world cannot be drawn at all - a scene target
/// that could not be made: the backdrop and the trouble's lines, so the
/// window says why it is empty.
pub fn draw_trouble_alone(d: &mut impl RaylibDraw, size: (f32, f32), px: f32, backdrop: Color) {
    d.clear_background(backdrop);
    draw_trouble(d, size, px);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stepped_size_holds_the_picture_and_moves_only_every_step() {
        assert_eq!(stepped((1, 1)), (STEP, STEP));
        assert_eq!(stepped((STEP, STEP + 1)), (STEP, 2 * STEP));
        // A drag of a window's edge by a pixel a frame makes the target
        // again once in `STEP` frames.
        let made = (1281..=1280 + 4 * STEP).map(|w| stepped((w, 720))).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(made.len(), 4);
        for w in [1, 63, 64, 65, 1279, 2560] {
            let (held, _) = stepped((w, 1));
            assert!(held >= w && held - w < STEP && held % STEP == 0, "{w} -> {held}");
        }
    }

    #[test]
    fn trouble_is_kept_once_and_leaves_when_it_recovers() {
        trouble("tests/a_target_no_frame_makes", "out of memory");
        trouble("tests/a_target_no_frame_makes", "out of memory");
        let kept = || TROUBLE.lock().unwrap().iter().filter(|k| k.as_str() == "tests/a_target_no_frame_makes").count();
        assert_eq!(kept(), 1);
        assert!(any_trouble());
        recovered("tests/a_target_no_frame_makes");
        assert_eq!(kept(), 0);
    }
}
