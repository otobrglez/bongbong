//! The walls sheet's texture, carrying the white block raylib draws its
//! shapes from (`SetShapesTexture`), so a rectangle and a sprite off the
//! walls sheet go out in one batch: a wall piece in flight is a shadow
//! rectangle then its sprite, and every tile's damage, a rectangle after
//! the tile.
//!
//! The texture is the sheet twice as tall, the sheet in its top half at
//! the origin and its width kept. A sprite's texture coordinates are its
//! pixels over the texture's size: over twice the height they are exactly
//! half what they were - a power of two, so no bit is lost - and every
//! sample lands on the texel it did on the sheet alone. No other size
//! and no offset keeps that: a sheet moved inside a larger texture has a
//! turned sprite's samples round onto other texels. The sheet's file is
//! not touched; the texture is built from its pixels at startup.
//!
//! `rounded_outline` is the one way a rounded rectangle's outline is
//! drawn.

use sola_raylib::prelude::*;

/// The white block's side, px, and where its top left sits below the
/// sheet: clear of the two rows that stand in for the wrap (`load`).
const WHITE: i32 = 8;
const WHITE_BELOW: i32 = 64;

/// The walls sheet as the texture described in the module comment, and
/// the rectangle of white inside it shapes are drawn from: the block's
/// middle, white on every side of it.
pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread, path: &str) -> Result<(Texture2D, crate::math::Rectangle), String> {
    let mut image = Image::load_image(path).map_err(|e| format!("{path}: {e}"))?;
    image.set_format(PixelFormat::PIXELFORMAT_UNCOMPRESSED_R8G8B8A8);
    let (w, h) = (image.width, image.height);
    let (first, last): (Vec<Color>, Vec<Color>) = {
        let colors = image.get_image_data();
        (colors[..w as usize].to_vec(), colors[(w * (h - 1)) as usize..].to_vec())
    };
    image.resize_canvas(w, h * 2, 0, 0, Color::BLANK);
    // A sample past the sheet's last row, or before its first, wrapped to
    // the other end of the sheet; here the rows it lands on hold the rows
    // it wrapped to.
    for x in 0..w {
        image.draw_pixel(x, h, first[x as usize]);
        image.draw_pixel(x, h * 2 - 1, last[x as usize]);
    }
    let top = h + WHITE_BELOW;
    for y in top..top + WHITE {
        for x in 0..WHITE.min(w) {
            image.draw_pixel(x, y, Color::WHITE);
        }
    }
    let texture = rl.load_texture_from_image(thread, &image).map_err(|e| format!("{path}: {e}"))?;
    let white = crate::math::Rectangle::new((WHITE / 2 - 1) as f32, (top + WHITE / 2 - 1) as f32, 2.0, 2.0);
    Ok((texture, white))
}

/// Draw every shape from `white` of `texture` from here on, and count its
/// draws against that texture (`frame_stages`).
pub fn draw_from(_window: &mut RaylibHandle, texture: &Texture2D, white: crate::math::Rectangle) {
    // SAFETY: the window is open (`_window`) and the texture is live; the
    // caller keeps it for as long as anything draws.
    unsafe { sola_raylib::ffi::SetShapesTexture(*texture.as_ref(), Rectangle::new(white.x, white.y, white.width, white.height).into()) };
    crate::frame_stages::shapes_texture(texture.id);
}

/// A rounded rectangle's outline as a band `thick` points wide just
/// outside `rec`, so the box it rings keeps its size and a panel's rim
/// stands clear of its fill. raylib draws a positive thickness inside the
/// rectangle and a negative one outside it; every rounded outline the
/// HUD, the lobby, the level select and the builder draw comes through
/// here.
pub fn rounded_outline<D: RaylibDraw>(
    d: &mut D,
    rec: impl Into<sola_raylib::ffi::Rectangle>,
    roundness: f32,
    segments: i32,
    thick: f32,
    color: impl Into<sola_raylib::ffi::Color>,
) {
    d.draw_rectangle_rounded_lines_ex(rec, roundness, segments, -thick.abs(), color);
}

#[cfg(test)]
mod tests {
    /// Over twice the height a row's coordinate is exactly half what it
    /// was, and comes back to the same row: every whole and half pixel of
    /// a sheet as tall as the walls'.
    #[test]
    fn a_sheet_twice_as_tall_keeps_every_sample() {
        let h = 1056.0f32;
        for half in 0..=(2 * 1056) {
            let y = half as f32 / 2.0;
            let (alone, doubled) = (y / h, y / (h * 2.0));
            assert_eq!(doubled, alone / 2.0, "row {y}");
            assert_eq!(doubled * (h * 2.0), alone * h, "row {y}");
        }
    }
}
