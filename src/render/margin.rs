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

use sola_raylib::prelude::*;

use crate::ground::{self, MarginShade, SHADE_BLOCK};
use crate::margin::{Margin, MarginFrame};
use crate::math::{Color, Rectangle};
use crate::render::canvas::{BlockTexture, GpuCanvas};
use crate::render::game::Textures;
use crate::render::weather::{self, CellMask, MaskTexture, PassView, WeatherFrame, WeatherFx};
use crate::simulation::Game;
use crate::tuning::tuning;

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
    /// has one. `backdrop` is what lies past the ground's reach. `None`
    /// where a target could not be made, which leaves the bars.
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
        let floor = Floor { margin, shade: shade_texture, textures, rect, time: game.time };
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
                        sky.plain_snow_cover_over(rect, &mut blocks);
                        weather::draw_blocks(&mut d, &blocks);
                        floor.shade(&mut d);
                    });
                    if plan.lit {
                        weather::multiply_ambient(&mut d, sky, size);
                    }
                    if plan.sky {
                        blocks.clear();
                        sky.plain_air_over(rect, &mut blocks);
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
                        let at = PassView { origin: crate::math::Vec2::new(rect.x, rect.y), size, light: *light.as_ref() };
                        if plan.ground {
                            let (cols, rows, bytes) = weather::margin_mask_bytes(game, &margin.ground);
                            let Ok(texture) = mask.sync(rl, thread, cols, rows, bytes) else {
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
            d.draw_texture_rec(picture, Rectangle::new(0.0, 0.0, size.0 as f32, -(size.1 as f32)), Vector2::new(0.0, 0.0), Color::WHITE);
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

/// What the margins' floor is painted from, in world pixels.
struct Floor<'a, 't> {
    margin: &'a Margin,
    /// The shade's image on the GPU; `None` paints the plateau alone.
    shade: Option<&'a Texture2D>,
    textures: &'a Textures<'t>,
    /// The world the target holds.
    rect: Rectangle,
    time: f32,
}

impl Floor<'_, '_> {
    /// The round's ground carried past its field, culled to the target.
    fn ground(&self, d: &mut impl RaylibDraw) {
        ground::draw(&mut GpuCanvas::culled(d, self.textures, Some(self.rect)), &self.margin.ground, self.margin.theme, self.time);
    }

    /// The shade: its image round the field, each texel a block, and the
    /// plateau on every block of the target past it.
    fn shade(&self, d: &mut impl RaylibDraw) {
        let MarginShade { image, origin, plateau } = &self.margin.shade;
        let block = SHADE_BLOCK as f32;
        let (w, h) = (image.width as f32, image.height as f32);
        let at = Rectangle::new(origin.0 as f32 * block, origin.1 as f32 * block, w * block, h * block);
        if let Some(texture) = self.shade {
            d.draw_texture_pro(texture, Rectangle::new(0.0, 0.0, w, h), at, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        for band in around(self.rect, at).into_iter().filter(|r| r.width > 0.0 && r.height > 0.0) {
            d.draw_rectangle_rec(band, *plateau);
        }
    }
}

/// The parts of `outer` that `inner` does not cover: the bands over and
/// under it the whole width, and the two beside it.
fn around(outer: Rectangle, inner: Rectangle) -> [Rectangle; 4] {
    let (right, bottom) = (outer.x + outer.width, outer.y + outer.height);
    let (x0, y0) = (inner.x.clamp(outer.x, right), inner.y.clamp(outer.y, bottom));
    let (x1, y1) = ((inner.x + inner.width).clamp(x0, right), (inner.y + inner.height).clamp(y0, bottom));
    [
        Rectangle::new(outer.x, outer.y, outer.width, y0 - outer.y),
        Rectangle::new(outer.x, y1, outer.width, bottom - y1),
        Rectangle::new(outer.x, y0, x0 - outer.x, y1 - y0),
        Rectangle::new(x1, y0, right - x1, y1 - y0),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bands round a rectangle cover what it does not, once.
    #[test]
    fn the_bands_round_the_image_cover_the_rest_of_the_target() {
        let outer = Rectangle::new(-100.0, -40.0, 400.0, 200.0);
        for inner in [Rectangle::new(0.0, 0.0, 100.0, 50.0), Rectangle::new(-150.0, 10.0, 600.0, 20.0), Rectangle::new(-200.0, -200.0, 900.0, 900.0)] {
            let bands = around(outer, inner);
            for y in (-40..160).step_by(7) {
                for x in (-100..300).step_by(7) {
                    let p = crate::math::Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let covered = bands.iter().filter(|b| b.width > 0.0 && b.height > 0.0 && b.contains(p)).count();
                    assert_eq!(covered, usize::from(!inner.contains(p)), "{inner:?} at {p:?}");
                }
            }
        }
    }
}
