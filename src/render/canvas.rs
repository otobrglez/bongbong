//! The raylib side of `canvas.rs`: `GpuCanvas` over a draw handle and the
//! `Sheets` lookup behind it, PNG decoding into `Pixels` and the CPU
//! canvas's way out as a raylib `Image` or PNG bytes.

use sola_raylib::prelude::*;

use std::collections::BTreeMap;

use crate::canvas::{BlockImage, Canvas, CpuCanvas, Pixels, Sheet};
use crate::frame_stages;
use crate::math::{Color, Rectangle, Vec2};
use crate::render::turn::{Fit, TurnShader};
use crate::turn::Turned;
use crate::Position;

/// Where a [`GpuCanvas`] finds the texture behind a [`Sheet`]:
/// `game::Textures` for the game, `editor::EditorTextures` for the builder.
pub trait Sheets {
    fn texture(&self, sheet: Sheet) -> &Texture2D;

    /// The uploaded copy of the [`BlockImage`] baked under `stamp`, if this
    /// lookup holds it ([`BlockTexture::sync`]). None draws nothing.
    fn blocks_texture(&self, _stamp: u64) -> Option<&Texture2D> {
        None
    }

    /// The shader a sprite turned off the quarters is drawn with
    /// (`render::turn`), if this lookup turns its sprites on their grid.
    /// None turns them the plain way.
    fn turn_shader(&self) -> Option<&TurnShader> {
        None
    }
}

/// The GPU copy of one [`BlockImage`] - the floor shade a `GroundGrid`
/// bakes - uploaded the first time its stamp is seen and kept until
/// another replaces it. Its owner (`app.rs`, the GPU thumbnail) calls
/// [`sync`](Self::sync) before the frame and hands the result to the
/// frame's `Sheets`, so the draw itself never uploads.
#[derive(Default)]
pub struct BlockTexture {
    held: Option<(u64, usize, usize, Texture2D)>,
}

impl BlockTexture {
    /// Make the held texture `image`'s, uploading only when the stamp
    /// changed (a new round, a builder edit) and reusing the texture when
    /// the size did not. A copy the image's patches reach from (a builder
    /// edit baked in place) uploads only the texels they name
    /// (`BlockImage::changed_since`). Returns the stamp and texture for a
    /// `Sheets`.
    pub fn sync(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, image: &BlockImage) -> Option<(u64, &Texture2D)> {
        if image.width == 0 || image.height == 0 || image.texels.len() != image.width * image.height {
            return None;
        }
        let held = self.held.as_ref().map(|(stamp, w, h, _)| (*stamp, *w, *h));
        if held.is_none_or(|(stamp, ..)| stamp != image.stamp) {
            let reuse = held.is_some_and(|(_, w, h)| w == image.width && h == image.height);
            let patch = held.filter(|_| reuse).and_then(|(stamp, ..)| image.changed_since(stamp));
            if !reuse {
                let blank = Image::gen_image_color(image.width as i32, image.height as i32, Color::new(0, 0, 0, 0));
                let texture = rl.load_texture_from_image(thread, &blank).ok()?;
                self.held = Some((0, image.width, image.height, texture));
            }
            // Patches far apart (a tile atlas's scattered slots) go up one
            // by one; close together, as their one bounding rectangle.
            let parts: Vec<crate::canvas::BlockPatch> = match (patch, held.and_then(|(stamp, ..)| image.patches_since(stamp))) {
                (Some(whole), Some(parts)) if parts.len() > 1 && 2 * parts.iter().map(|p| p.width * p.height).sum::<usize>() < whole.width * whole.height => parts.to_vec(),
                (Some(whole), _) => vec![whole],
                (None, _) => Vec::new(),
            };
            let (stamp, _, _, texture) = self.held.as_mut()?;
            match patch {
                Some(_) => {
                    for p in parts.iter().filter(|p| p.width > 0 && p.height > 0) {
                        let bytes: Vec<u8> = (p.y..p.y + p.height)
                            .flat_map(|y| &image.texels[y * image.width + p.x..y * image.width + p.x + p.width])
                            .flat_map(|c| [c.r, c.g, c.b, c.a])
                            .collect();
                        let rect = Rectangle::new(p.x as f32, p.y as f32, p.width as f32, p.height as f32);
                        texture.update_texture_rec(rect, &bytes).ok()?;
                    }
                }
                None => {
                    let bytes: Vec<u8> = image.texels.iter().flat_map(|c| [c.r, c.g, c.b, c.a]).collect();
                    texture.update_texture(&bytes).ok()?;
                }
            }
            *stamp = image.stamp;
        }
        self.held.as_ref().map(|(stamp, _, _, texture)| (*stamp, texture))
    }

    /// The texture held and the stamp of the image it shows, if any.
    pub fn held(&self) -> Option<(u64, &Texture2D)> {
        self.held.as_ref().map(|(stamp, _, _, texture)| (*stamp, texture))
    }
}

/// The GPU copies of a list of [`BlockImage`]s - a round's floor shade,
/// the lava's banks and its tile atlases, the cones - one
/// [`BlockTexture`] per place in the list, so an image that keeps its
/// place keeps its texture and uploads only what changed.
#[derive(Default)]
pub struct BlockTextures {
    held: Vec<BlockTexture>,
}

impl BlockTextures {
    /// Bring every copy up to its image (`BlockTexture::sync`) and hand
    /// out the stamps and textures for a `Sheets`.
    pub fn sync(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, images: &[&BlockImage]) -> Vec<(u64, &Texture2D)> {
        self.held.resize_with(images.len(), BlockTexture::default);
        for (texture, image) in self.held.iter_mut().zip(images) {
            texture.sync(rl, thread, image);
        }
        self.held.iter().filter_map(BlockTexture::held).collect()
    }
}

/// A [`Canvas`] over a raylib draw handle: every call forwards to the
/// raylib call the trait is named after, and a dev-tools build counts it
/// (`frame_stages::Draws`). A sprite turned off the quarters, from a
/// sheet with a design-pixel grid (`turn::block`), is drawn on that grid
/// by the turn shader instead (`turned`); the shader stays open over a
/// run of them and closes before anything else is drawn, and when the
/// canvas is dropped.
pub struct GpuCanvas<'a, D, S: Sheets> {
    d: &'a mut D,
    sheets: &'a S,
    cull: Option<Rectangle>,
    /// What the open turn shader's uniforms hold; `None` while the batch
    /// draws with its default shader.
    turning: Option<Fit>,
}

impl<'a, D: RaylibDraw, S: Sheets> GpuCanvas<'a, D, S> {
    /// A canvas that draws everything it is asked to.
    pub fn new(d: &'a mut D, sheets: &'a S) -> Self {
        Self::culled(d, sheets, None)
    }

    /// A canvas over a camera's view: `cull` is the world rectangle worth
    /// drawing (`view::Camera::cull`), `None` for everything.
    pub fn culled(d: &'a mut D, sheets: &'a S, cull: Option<Rectangle>) -> Self {
        GpuCanvas { d, sheets, cull, turning: None }
    }
}

impl<D, S: Sheets> GpuCanvas<'_, D, S> {
    /// Draw the blit turned on its sheet's grid, if it is one that is:
    /// off the quarters, from a sheet with a grid, with the shader there
    /// and `turn_mode` not the plain one. False leaves it to the plain blit.
    #[allow(clippy::too_many_arguments)]
    fn turned(&mut self, sheet: Sheet, texture: &Texture2D, src: Rectangle, dest: Rectangle, origin: Vec2, rotation: f32, tint: Color, solid: bool) -> bool {
        if crate::turn::is_quarter(rotation) {
            return false;
        }
        let sheets = self.sheets;
        let (Some(block), Some(shader)) = (crate::turn::block(sheet), sheets.turn_shader()) else { return false };
        let t = crate::tuning::tuning();
        if t.turn_mode == 0 {
            return false;
        }
        let Some(turned) = Turned::of(src, dest, origin, rotation, block) else { return false };
        let fit = Fit::of(texture, &turned, block, t.turn_mode == 2, t.turn_corner, solid);
        if self.turning.is_none() {
            shader.open();
            frame_stages::shader(());
        }
        if self.turning != Some(fit) {
            shader.fit(&fit);
            self.turning = Some(fit);
        }
        shader.draw(texture, &turned, tint);
        true
    }

    /// Close the turn shader if a run of turned sprites left it open:
    /// what is drawn next is drawn the plain way.
    fn plain(&mut self) {
        if self.turning.take().is_some()
            && let Some(shader) = self.sheets.turn_shader()
        {
            shader.close();
        }
    }
}

impl<D, S: Sheets> Drop for GpuCanvas<'_, D, S> {
    fn drop(&mut self) {
        self.plain();
    }
}

impl<D: RaylibDraw, S: Sheets> Canvas for GpuCanvas<'_, D, S> {
    fn blit(&mut self, sheet: Sheet, src: Rectangle, dest: Rectangle, origin: Vec2, rotation: f32, tint: Color) {
        let sheets = self.sheets;
        let texture = sheets.texture(sheet);
        frame_stages::blit(texture.id);
        if self.turned(sheet, texture, src, dest, origin, rotation, tint, false) {
            return;
        }
        self.plain();
        self.d.draw_texture_pro(texture, src, dest, origin, rotation, tint);
    }

    fn solid(&mut self, sheet: Sheet, src: Rectangle, dest: Rectangle, origin: Vec2, rotation: f32, color: Color) {
        let sheets = self.sheets;
        let texture = sheets.texture(sheet);
        frame_stages::blit(texture.id);
        // A sprite the turn shader draws on its grid gets its silhouette
        // from the same shader, block for block; any other from the solid
        // shader, a batch of its own. Without either it is the sprite in
        // the colour as a tint.
        if self.turned(sheet, texture, src, dest, origin, rotation, color, true) {
            return;
        }
        self.plain();
        match sheets.turn_shader().and_then(|turn| turn.solid()) {
            Some(shader) => {
                shader.open();
                frame_stages::shader(());
                self.d.draw_texture_pro(texture, src, dest, origin, rotation, color);
                shader.close();
            }
            None => self.d.draw_texture_pro(texture, src, dest, origin, rotation, color),
        }
    }

    fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Color) {
        self.plain();
        frame_stages::rect();
        self.d.draw_rectangle(x, y, width, height, color);
    }

    fn gradient_v(&mut self, x: i32, y: i32, width: i32, height: i32, top: Color, bottom: Color) {
        self.plain();
        frame_stages::rect();
        self.d.draw_rectangle_gradient_v(x, y, width, height, top, bottom);
    }

    fn gradient_h(&mut self, x: i32, y: i32, width: i32, height: i32, left: Color, right: Color) {
        self.plain();
        frame_stages::rect();
        self.d.draw_rectangle_gradient_h(x, y, width, height, left, right);
    }

    fn disc(&mut self, center: Position, radius: f32, color: Color) {
        self.plain();
        frame_stages::shape();
        self.d.draw_circle_v(center, radius, color);
    }

    fn ring(&mut self, center: Position, inner: f32, outer: f32, start_deg: f32, end_deg: f32, segments: i32, color: Color) {
        self.plain();
        frame_stages::shape();
        self.d.draw_ring(center, inner, outer, start_deg, end_deg, segments, color);
    }

    fn blocks(&mut self, image: &BlockImage) {
        let Some(texture) = self.sheets.blocks_texture(image.stamp) else { return };
        let (w, h) = (image.width as f32, image.height as f32);
        let b = image.block as f32;
        self.plain();
        frame_stages::blit(texture.id);
        self.d.draw_texture_pro(texture, Rectangle::new(0.0, 0.0, w, h), Rectangle::new(0.0, 0.0, w * b, h * b), Vec2::new(0.0, 0.0), 0.0, Color::WHITE);
    }

    fn blocks_part(&mut self, image: &BlockImage, src: (usize, usize, usize, usize), at: (i32, i32)) {
        let Some(texture) = self.sheets.blocks_texture(image.stamp) else { return };
        let (x, y, w, h) = (src.0 as f32, src.1 as f32, src.2 as f32, src.3 as f32);
        let b = image.block as f32;
        self.plain();
        frame_stages::blit(texture.id);
        self.d.draw_texture_pro(texture, Rectangle::new(x, y, w, h), Rectangle::new(at.0 as f32, at.1 as f32, w * b, h * b), Vec2::new(0.0, 0.0), 0.0, Color::WHITE);
    }

    fn has_blocks(&self, stamp: u64) -> bool {
        self.sheets.blocks_texture(stamp).is_some()
    }

    fn cull(&self) -> Option<Rectangle> {
        self.cull
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
