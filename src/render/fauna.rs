//! The raylib half of `fauna.rs` (docs/field-life.md): the field's crows,
//! ducks and fireflies painted in the round's layers - what sits or swims
//! on the floor, what flies and its shadow in the air stages, the
//! fireflies over the lit field.

use sola_raylib::prelude::*;

use crate::fauna::Fauna;
use crate::math::Rectangle;
use crate::pyro::Shape;
use crate::render::canvas::{GpuCanvas, Sheets};
use crate::simulation::Game;
use crate::tuning::tuning;

fn paint<D: RaylibDraw>(d: &mut D, sheets: &impl Sheets, shapes: &[Shape], cull: Option<Rectangle>) {
    if !shapes.is_empty() {
        crate::pyro::draw(&mut GpuCanvas::culled(d, sheets, cull), shapes);
    }
}

/// The crows on the grass and the ducks on the water, over the floor and
/// under everything that stands, within `cull` (`Camera::cull`).
pub fn draw_ground<D: RaylibDraw>(d: &mut D, sheets: &impl Sheets, game: &Game, fauna: &Fauna, cull: Option<Rectangle>) {
    let t = tuning();
    let mut shapes = fauna.ducks().water_shapes(game.water(), game.time, cull, &t);
    shapes.extend(fauna.crows().ground_shapes(game.time, cull));
    paint(d, sheets, &shapes, cull);
}

/// The shadows of the crows and ducks in the air, on the ground under
/// them (`Game::paint_air_shadows`' stage).
pub fn draw_air_shadows<D: RaylibDraw>(d: &mut D, sheets: &impl Sheets, game: &Game, fauna: &Fauna, cull: Option<Rectangle>) {
    if !game.shadows_enabled {
        return;
    }
    let t = tuning();
    let mut shapes = fauna.crows().air_shadows(game.time, cull, &t);
    shapes.extend(fauna.ducks().air_shadows(game.water(), cull, &t));
    paint(d, sheets, &shapes, cull);
}

/// The crows and ducks in the air and the crows in the crowns, over
/// everything that stands (`Game::paint_air`'s stage); a perched crow
/// swings with its tree's shaken crown.
pub fn draw_air<D: RaylibDraw>(d: &mut D, sheets: &impl Sheets, game: &Game, fauna: &Fauna, cull: Option<Rectangle>) {
    let t = tuning();
    let crows = fauna.crows();
    let lean = |tree: usize| {
        let cell = crows.tree_cell(tree);
        game.tree_shakes.iter().filter(|s| s.cell == cell).map(|s| s.lean(game.time, &t)).sum::<f32>()
    };
    let mut shapes = fauna.ducks().air_shapes(game.water(), cull, &t);
    shapes.extend(crows.air_shapes(game.time, cull, &t, lean));
    paint(d, sheets, &shapes, cull);
}

/// The fireflies, over the lit field so they keep their own light in the
/// dark; none while the sky is light (`fireflies::shown`).
pub fn draw_glowing<D: RaylibDraw>(d: &mut D, sheets: &impl Sheets, game: &Game, fauna: &Fauna, cull: Option<Rectangle>) {
    let t = tuning();
    let shown = crate::fireflies::shown(game, &t);
    if shown > 0.0 {
        paint(d, sheets, &fauna.fireflies().shapes(game.time, shown, cull, &t), cull);
    }
}
