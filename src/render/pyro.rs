//! The raylib side of `pyro.rs`: the draw handle as a `pyro::Blocks`, so
//! the block painters run inside `Game::render`'s additive blend blocks
//! with the same code the CPU tests paint through.

use sola_raylib::prelude::*;

use crate::math::Color;
use crate::pyro::Blocks;

/// A raylib draw handle (or a blend-mode scope of one) as `Blocks`.
pub struct Rl<'a, D: RaylibDraw>(pub &'a mut D);

impl<D: RaylibDraw> Blocks for Rl<'_, D> {
    fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Color) {
        self.0.draw_rectangle(x, y, width, height, color);
    }
}
