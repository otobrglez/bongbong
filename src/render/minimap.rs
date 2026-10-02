//! The raylib half of `minimap.rs`: painting a minimap - its plate, the
//! image's GPU copy (`render::canvas::BlockTexture`, uploaded by its owner
//! before the frame) scaled into its rectangle a texel to a cell, and the
//! marks `minimap::picture` composed over it. The play minimap is drawn in
//! UI points with the right corner cluster (`render::hud::draw_corners`),
//! the builder's navigator in the bitmap's pixels with the builder's chrome
//! (`editor/render.rs`).

use sola_raylib::prelude::*;

use crate::hud::Corners;
use crate::indicators::Fill;
use crate::math::{Color, Rectangle};
use crate::minimap::{source, Marks, Picture};

/// What a frame draws the play minimap from (`Effects::minimap`): the
/// GPU copy of the round's image, the field it covers and this frame's
/// marks. The picture is composed where the corners put the minimap.
pub struct MinimapLayer<'a> {
    pub texture: &'a Texture2D,
    pub field: (f32, f32),
    pub marks: &'a Marks,
}

/// Paint a minimap in `rect` - on its plate, its image `texture` covering
/// a field of `field` world pixels (`minimap::source`), `picture` over it
/// - everything at `alpha` (the right cluster's fade). The texture is
/// sampled nearest, so every cell is a block of one colour.
pub fn draw_minimap(d: &mut impl RaylibDraw, rect: Rectangle, texture: &Texture2D, field: (f32, f32), picture: &Picture, alpha: f32) {
    crate::render::hud::draw_plate(d, Corners::plate(rect), crate::render::hud::PLATE_EDGE, alpha);
    let tint = Color::new(255, 255, 255, (255.0 * alpha.clamp(0.0, 1.0)).round() as u8);
    d.draw_texture_pro(texture, source(field), rect, Vector2::new(0.0, 0.0), 0.0, tint);
    for f in &picture.fills {
        fill(d, f, alpha);
    }
    for label in &picture.labels {
        fill(d, &label.plate, alpha);
        d.draw_text(&label.text, label.x, label.y, label.size, faded(label.color, alpha));
    }
}

fn fill(d: &mut impl RaylibDraw, f: &Fill, alpha: f32) {
    d.draw_rectangle(f.x, f.y, f.w, f.h, faded(f.color, alpha));
}

fn faded(c: Color, alpha: f32) -> Color {
    Color::new(c.r, c.g, c.b, (c.a as f32 * alpha.clamp(0.0, 1.0)).round() as u8)
}
