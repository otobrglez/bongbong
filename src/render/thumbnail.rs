//! The raylib side of `thumbnail.rs`: the sheets decoded for the CPU
//! canvas, the GPU renderer through a hidden window and the PNG encoder.

use sola_raylib::prelude::*;
use std::collections::BTreeMap;

use crate::canvas::{Pixels, Sheet};
use crate::math::Color;
use crate::render::canvas::{GpuCanvas, Sheets};
use crate::simulation::Game;
use crate::thumbnail::{field_pixels, PAINT};

/// Every sheet decoded for the CPU canvas. Load once per batch; each
/// render clones the map (a few megabytes) into its canvas.
pub fn load_cpu_sheets() -> Result<BTreeMap<Sheet, Pixels>, String> {
    let mut sheets = BTreeMap::new();
    for sheet in Sheet::all() {
        sheets.insert(sheet, Pixels::load(&sheet.path())?);
    }
    Ok(sheets)
}

/// The GPU side's `Sheet` lookup: one texture per sheet, loaded by path.
pub struct GpuSheets(pub BTreeMap<Sheet, Texture2D>);

impl GpuSheets {
    /// Load every sheet as a texture. Needs a window (hidden is fine).
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Result<Self, String> {
        let mut map = BTreeMap::new();
        for sheet in Sheet::all() {
            let path = sheet.path();
            let texture = rl.load_texture(thread, &path).map_err(|e| format!("{path}: {e}"))?;
            map.insert(sheet, texture);
        }
        Ok(GpuSheets(map))
    }
}

impl Sheets for GpuSheets {
    fn texture(&self, sheet: Sheet) -> &Texture2D {
        self.0.get(&sheet).unwrap_or_else(|| panic!("{sheet:?} was not loaded"))
    }
}

/// Paint `game`'s field into a render texture through the same `Canvas`
/// stages the game's own pass 1 runs, and read it back as an `Image`
/// (top row first, like the CPU canvas).
pub fn render_gpu(rl: &mut RaylibHandle, thread: &RaylibThread, sheets: &GpuSheets, game: &Game) -> Result<Image, String> {
    let (w, h) = field_pixels(game);
    let mut scene = rl.load_render_texture(thread, w as u32, h as u32).map_err(|e| e.to_string())?;
    rl.draw_texture_mode(thread, &mut scene, |mut d| {
        d.clear_background(Color::WHITE);
        game.paint_field(&mut GpuCanvas::new(&mut d, sheets), PAINT);
    });
    let mut image = scene.load_image().map_err(|e| e.to_string())?;
    image.flip_vertical();
    Ok(image)
}

/// PNG bytes of a read-back `Image`, scaled up `scale` times with whole
/// pixels.
pub fn image_png_bytes(mut image: Image, scale: u32) -> Result<Vec<u8>, String> {
    if scale > 1 {
        image.resize_nn(image.width() * scale as i32, image.height() * scale as i32);
    }
    image.export_image_to_memory(".png").map_err(|e| e.to_string())
}
