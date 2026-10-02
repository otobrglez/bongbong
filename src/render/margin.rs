//! The raylib half of `margin.rs` (docs/large-maps-follow-camera.md §13
//! item 7): the world an arena shows past its field, drawn into a target
//! of its own, a texel per world pixel, for `Game::render` to put round the
//! field on the window (`render::view::present_into`). The round's ground
//! carried on out of the field (`Margin::ground`) and the shade that takes
//! it off the playfield (`Margin::shade`), then the round's sky the way the
//! field takes it: the ground, light and sky passes of `render::weather`
//! over this target's own world rectangle, under a light map of the
//! ambient alone - no lamp of the round reaches past its boundary - or,
//! where the passes would not compile, the plain sky's blocks and a
//! multiply, as the field draws it then.
//!
//! The target covers the whole window's world, but only its margins are
//! drawn and passed through the sky - the parts past the field and a strip
//! under its edge (`MarginFrame::parts`) - since the field's bitmap covers
//! the rest: on a phone's arena the bars beside it are a tenth of the
//! window, and the passes are full-screen shaders on a GPU that is short of
//! fill.

use sola_raylib::prelude::*;

use crate::ground::{self, MarginShade, SHADE_BLOCK};
use crate::margin::{around, overlap, Margin, MarginFrame};
use crate::math::{Color, Rectangle};
use crate::render::canvas::{BlockTexture, GpuCanvas};
use crate::render::game::Textures;
use crate::render::weather::{self, CellMask, MaskTexture, PassView, WeatherFrame, WeatherFx};
use crate::simulation::Game;
use crate::tuning::tuning;

/// How far into the field, in world px, the margins are drawn under the
/// field's bitmap: the heat haze shifts what the sky pass reads by up to
/// 8 px (`haze_amplitude_px` at its top), so a margin's inner edge reads
/// ground drawn for it.
const UNDER_FIELD_PX: f32 = 16.0;

/// The targets the margins are drawn through, the frame's size.
struct Targets {
    size: (i32, i32),
    /// The picture, and the stage before it under a sky.
    a: RenderTexture2D,
    b: RenderTexture2D,
    /// The bare ground the ground pass reads.
    ground: RenderTexture2D,
    /// The picture as the window takes it (`draw`'s last step).
    out: RenderTexture2D,
}

/// The margins' GPU state for the whole session. `app.rs` owns it and
/// hands it to `Game::render` through `Effects::margins`; `None` there
/// draws the flat bars.
#[derive(Default)]
pub struct MarginFx {
    /// The ground and the shade, made for a round's floor and a reach.
    margin: Option<Margin>,
    /// The shade's image on the GPU.
    shade: BlockTexture,
    targets: Option<Targets>,
    /// A light map of one texel, the ambient.
    light: Option<RenderTexture2D>,
    mask: MaskTexture,
    /// Set by the first target that could not be made: said once on
    /// stderr, and the bars drawn instead.
    warned: bool,
}

impl MarginFx {
    /// Draw the world `frame` shows round `game`'s field into the target
    /// this keeps and hand the target back: the ground, its shade, and the
    /// sky `sky` is - the frame's weather and its passes - when the round
    /// has one, over the margins (`MarginFrame::parts`). `backdrop` is what
    /// lies past the ground's reach. `None` where a target could not be
    /// made, which leaves the bars.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        game: &Game,
        frame: &MarginFrame,
        textures: &Textures,
        backdrop: Color,
        sky: Option<(&mut WeatherFx, &WeatherFrame)>,
    ) -> Option<&RenderTexture2D> {
        let size = (frame.rect.width as i32, frame.rect.height as i32);
        if let Err(e) = self.ensure_targets(rl, thread, size) {
            if !self.warned {
                eprintln!("[margin] {e}; drawing the margins flat");
                self.warned = true;
            }
            return None;
        }
        if !self.margin.as_ref().is_some_and(|m| m.fits(&game.ground, frame.cells)) {
            self.margin = Some(Margin::new(&game.ground, game.map.field_size(), game.map.theme, frame.cells, &tuning()));
        }
        let MarginFx { margin, shade, targets, light, mask, .. } = self;
        let margin = margin.as_ref().expect("made above");
        let shade_texture = shade.sync(rl, thread, &margin.shade.image).map(|(_, texture)| texture);
        let Targets { a, b, ground: bare, out, .. } = targets.as_mut().expect("made above");
        let rect = frame.rect;
        let world = Camera2D { offset: Vector2::new(0.0, 0.0), target: Vector2::new(rect.x, rect.y), rotation: 0.0, zoom: 1.0 };
        // What is drawn: the margins and a strip under the field's edge, in
        // world pixels and in the targets' own.
        let parts = frame.parts(UNDER_FIELD_PX);
        let local: Vec<Rectangle> = parts.iter().map(|p| Rectangle::new(p.x - rect.x, p.y - rect.y, p.width, p.height)).collect();
        let reach = parts.iter().copied().reduce(bounds).unwrap_or(rect);
        let floor = Floor { margin, shade: shade_texture, textures, parts: &parts, reach, time: game.time };
        let ground_and_shade = |rl: &mut RaylibHandle, a: &mut RenderTexture2D| {
            rl.draw_texture_mode(thread, a, |mut d| {
                d.clear_background(backdrop);
                d.draw_mode2D(world, |mut d, _| {
                    floor.ground(&mut d);
                    floor.shade(&mut d);
                });
            });
        };

        // Which of the two targets the picture ends in.
        let in_a = match sky {
            None => {
                ground_and_shade(rl, a);
                true
            }
            Some((_, sky)) if sky.plain => {
                // The sky without its shaders, as the field draws it then:
                // the snow on the ground under the shade, the ambient
                // multiplied in, then the air as plain blocks and
                // lightning's white.
                let plan = sky.plan;
                let mut blocks = Vec::new();
                rl.draw_texture_mode(thread, a, |mut d| {
                    d.clear_background(backdrop);
                    d.draw_mode2D(world, |mut d, _| {
                        floor.ground(&mut d);
                        sky.plain_snow_cover_over(reach, &mut blocks);
                        weather::draw_blocks(&mut d, &blocks);
                        floor.shade(&mut d);
                    });
                    if plan.lit {
                        weather::multiply_ambient(&mut d, sky, size);
                    }
                    if plan.sky {
                        blocks.clear();
                        sky.plain_air_over(reach, &mut blocks);
                        d.draw_mode2D(world, |mut d, _| weather::draw_blocks(&mut d, &blocks));
                        weather::draw_flash_over(&mut d, sky, size);
                    }
                });
                true
            }
            Some((fx, sky)) => {
                // The passes in the order pass 1 runs them on the field,
                // over this target's world under a light map of the
                // ambient alone; each stage draws from the one before into
                // the other target.
                let plan = sky.plan;
                match fx.shader_passes() {
                    None => {
                        ground_and_shade(rl, a);
                        true
                    }
                    Some(mut passes) => {
                        let light = light.as_mut().expect("made with the targets");
                        rl.draw_texture_mode(thread, light, |mut d| d.clear_background(sky.ambient_texel()));
                        let at = PassView { origin: crate::math::Vec2::new(rect.x, rect.y), size, light: *light.as_ref(), parts: &local };
                        if plan.ground {
                            let made = weather::MaskFor { round: crate::minimap::RoundKey::of(game), origin: margin.ground.origin() };
                            let Ok((texture, (cols, rows))) =
                                mask.sync_for(rl, thread, made, game.frame(), || weather::margin_mask_bytes(game, &margin.ground))
                            else {
                                return None;
                            };
                            let cells = CellMask { texture: *texture.as_ref(), cells: (cols, rows), origin: margin.ground.origin() };
                            rl.draw_texture_mode(thread, bare, |mut d| {
                                d.clear_background(backdrop);
                                d.draw_mode2D(world, |mut d, _| floor.ground(&mut d));
                            });
                            rl.draw_texture_mode(thread, a, |mut d| {
                                d.clear_background(backdrop);
                                passes.ground(&mut d, bare, sky, &at, &cells);
                                d.draw_mode2D(world, |mut d, _| floor.shade(&mut d));
                            });
                        } else {
                            ground_and_shade(rl, a);
                        }
                        let mut in_a = true;
                        if plan.lit {
                            rl.draw_texture_mode(thread, b, |mut d| passes.lit(&mut d, a, sky, &at));
                            in_a = false;
                        }
                        if plan.sky {
                            if in_a {
                                rl.draw_texture_mode(thread, b, |mut d| passes.sky(&mut d, a, sky, &at));
                            } else {
                                rl.draw_texture_mode(thread, a, |mut d| passes.sky(&mut d, b, sky, &at));
                            }
                            in_a = !in_a;
                        }
                        in_a
                    }
                }
            }
        };
        // The step the field's scene takes into its bitmap (`Game::render`'s
        // pass 2): onto black, alpha-blended - where a translucent draw left
        // the alpha under one, that darkens the picture as it darkens the
        // field, and `present_into` lays the two on the backdrop alike.
        let picture: &RenderTexture2D = if in_a { a } else { b };
        rl.draw_texture_mode(thread, out, |mut d| {
            d.clear_background(Color::BLACK);
            for part in &local {
                // Read the right way up: a render texture is stored
                // bottom-up.
                let rows = Rectangle::new(part.x, size.1 as f32 - part.y - part.height, part.width, -part.height);
                d.draw_texture_rec(picture, rows, Vector2::new(part.x, part.y), Color::WHITE);
            }
        });
        Some(&*out)
    }

    fn ensure_targets(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, size: (i32, i32)) -> Result<(), String> {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return Ok(());
        }
        let make = |rl: &mut RaylibHandle| rl.load_render_texture(thread, size.0.max(1) as u32, size.1.max(1) as u32).map_err(|e| format!("margin target: {e}"));
        let a = make(rl)?;
        let b = make(rl)?;
        let ground = make(rl)?;
        let out = make(rl)?;
        if self.light.is_none() {
            self.light = Some(rl.load_render_texture(thread, 1, 1).map_err(|e| format!("margin light map: {e}"))?);
        }
        self.targets = Some(Targets { size, a, b, ground, out });
        Ok(())
    }
}

/// The smallest rectangle holding both.
fn bounds(a: Rectangle, b: Rectangle) -> Rectangle {
    let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
    let (x1, y1) = ((a.x + a.width).max(b.x + b.width), (a.y + a.height).max(b.y + b.height));
    Rectangle::new(x0, y0, x1 - x0, y1 - y0)
}

/// What the margins' floor is painted from, in world pixels.
struct Floor<'a, 't> {
    margin: &'a Margin,
    /// The shade's image on the GPU; `None` paints the plateau alone.
    shade: Option<&'a Texture2D>,
    textures: &'a Textures<'t>,
    /// The parts drawn (`MarginFrame::parts`), none overlapping, and the
    /// world they reach across.
    parts: &'a [Rectangle],
    reach: Rectangle,
    time: f32,
}

impl Floor<'_, '_> {
    /// The round's ground carried past its field, the cells the parts
    /// reach across: opaque tiles, drawn once each.
    fn ground(&self, d: &mut impl RaylibDraw) {
        ground::draw(&mut GpuCanvas::culled(d, self.textures, Some(self.reach)), &self.margin.ground, self.margin.theme, self.time);
    }

    /// The shade, part by part so no block of it is laid twice: its image
    /// round the field, each texel a block, and the plateau past it.
    fn shade(&self, d: &mut impl RaylibDraw) {
        let MarginShade { image, origin, plateau } = &self.margin.shade;
        let block = SHADE_BLOCK as f32;
        let at = Rectangle::new(origin.0 as f32 * block, origin.1 as f32 * block, image.width as f32 * block, image.height as f32 * block);
        for part in self.parts {
            if let (Some(texture), Some(on)) = (self.shade, overlap(*part, at)) {
                let texels = Rectangle::new((on.x - at.x) / block, (on.y - at.y) / block, on.width / block, on.height / block);
                d.draw_texture_pro(texture, texels, on, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
            }
            for band in around(*part, at) {
                if let Some(band) = overlap(band, *part) {
                    d.draw_rectangle_rec(band, *plateau);
                }
            }
        }
    }
}
