//! The raylib half of `fish.rs`: the round's fish painted over the water's
//! tiles, under everything that stands or burns (docs/water.md).

use sola_raylib::prelude::*;

use crate::fish::Shoal;
use crate::math::Rectangle;
use crate::render::canvas::{GpuCanvas, Sheets};
use crate::simulation::Game;

/// Paint `shoal`'s fish in `game`'s water at the round's clock, within
/// `cull` (`Camera::cull`) where there is one. Nothing under ice.
pub fn draw_fish<D: RaylibDraw>(d: &mut D, sheets: &impl Sheets, game: &Game, shoal: &Shoal, cull: Option<Rectangle>) {
    let shapes = shoal.shapes(game.water(), game.time, &crate::tuning::tuning(), cull);
    if !shapes.is_empty() {
        crate::pyro::draw(&mut GpuCanvas::culled(d, sheets, cull), &shapes);
    }
}
