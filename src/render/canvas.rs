//! The raylib side of `canvas.rs`: `GpuCanvas` over a draw handle and the
//! `Sheets` lookup behind it, PNG decoding into `Pixels` and the CPU
//! canvas's way out as a raylib `Image` or PNG bytes.

use sola_raylib::prelude::*;

use std::collections::BTreeMap;

use crate::canvas::{Canvas, CpuCanvas, Pixels, Sheet};
use crate::math::{Color, Rectangle, Vec2};
use crate::Position;

/// Where a [`GpuCanvas`] finds the texture behind a [`Sheet`]:
/// `game::Textures` for the game, `editor::EditorTextures` for the builder.
pub trait Sheets {
    fn texture(&self, sheet: Sheet) -> &Texture2D;
}

/// A [`Canvas`] over a raylib draw handle: every call forwards to the
/// raylib call the trait is named after.
pub struct GpuCanvas<'a, D, S> {
    d: &'a mut D,
    sheets: &'a S,
}

impl<'a, D: RaylibDraw, S: Sheets> GpuCanvas<'a, D, S> {
    pub fn new(d: &'a mut D, sheets: &'a S) -> Self {
        GpuCanvas { d, sheets }
    }
}

impl<D: RaylibDraw, S: Sheets> Canvas for GpuCanvas<'_, D, S> {
    fn blit(&mut self, sheet: Sheet, src: Rectangle, dest: Rectangle, origin: Vec2, rotation: f32, tint: Color) {
        self.d.draw_texture_pro(self.sheets.texture(sheet), src, dest, origin, rotation, tint);
    }

    fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Color) {
        self.d.draw_rectangle(x, y, width, height, color);
    }

    fn gradient_v(&mut self, x: i32, y: i32, width: i32, height: i32, top: Color, bottom: Color) {
        self.d.draw_rectangle_gradient_v(x, y, width, height, top, bottom);
    }

    fn gradient_h(&mut self, x: i32, y: i32, width: i32, height: i32, left: Color, right: Color) {
        self.d.draw_rectangle_gradient_h(x, y, width, height, left, right);
    }

    fn disc(&mut self, center: Position, radius: f32, color: Color) {
        self.d.draw_circle_v(center, radius, color);
    }

    fn ring(&mut self, center: Position, inner: f32, outer: f32, start_deg: f32, end_deg: f32, segments: i32, color: Color) {
        self.d.draw_ring(center, inner, outer, start_deg, end_deg, segments, color);
    }
}
impl Pixels {
    /// Decode a PNG through raylib's CPU image loader (stb_image). Needs no
    /// window. Any pixel format is converted to RGBA by `LoadImageColors`.
    pub fn load(path: &str) -> Result<Self, String> {
        let image = Image::load_image(path).map_err(|e| format!("{path}: {e}"))?;
        let (width, height) = (image.width().max(0) as usize, image.height().max(0) as usize);
        let colors = image.get_image_data();
        Ok(Pixels { width, height, data: colors.iter().map(|&c| c.into()).collect() })
    }
}

impl CpuCanvas {
    /// A white canvas with every sheet in [`Sheet::all`] decoded from
    /// `static/` under the working directory. Fails naming the first file
    /// that would not load.
    pub fn load(width: usize, height: usize) -> Result<Self, String> {
        let mut sheets = BTreeMap::new();
        for sheet in Sheet::all() {
            sheets.insert(sheet, Pixels::load(&sheet.path())?);
        }
        Ok(Self::new(width, height, sheets))
    }

    /// The canvas as a raylib `Image` (RGBA8), for encoding or resizing.
    pub fn to_image(&self) -> Image {
        let mut image = Image::gen_image_color(self.width() as i32, self.height() as i32, Color::WHITE);
        for y in 0..self.height() {
            for x in 0..self.width() {
                image.draw_pixel(x as i32, y as i32, self.pixel(x, y));
            }
        }
        image
    }

    /// PNG bytes of the canvas, scaled up `scale` times with whole pixels.
    pub fn png_bytes(&self, scale: u32) -> Result<Vec<u8>, String> {
        let mut image = self.to_image();
        if scale > 1 {
            image.resize_nn((self.width() as u32 * scale) as i32, (self.height() as u32 * scale) as i32);
        }
        image.export_image_to_memory(".png").map_err(|e| e.to_string())
    }

    /// Write the canvas to `path` as a PNG, scaled up `scale` times.
    pub fn write_png(&self, path: &std::path::Path, scale: u32) -> Result<(), String> {
        let bytes = self.png_bytes(scale)?;
        std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
    }
}
