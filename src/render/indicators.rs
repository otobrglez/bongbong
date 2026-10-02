//! The raylib half of `indicators.rs`: painting a `Picture`, the blocks
//! and labels `indicators::picture` composed for what a screen cannot see.
//! Drawn after the world layer, with the HUD (`Game::draw_chrome`), so the
//! weather's light never darkens it.

use sola_raylib::prelude::*;

use crate::indicators::{Fill, Picture};

/// Paint `picture`: its world blocks through `world` (the camera that puts
/// the world where the frame shows it - `view::Camera::on_field`, carried
/// onto the window for a followed view), then its screen blocks and labels
/// in the bitmap's own pixels - into the bitmap itself (`bitmap` none), or
/// onto the window through `bitmap`, the camera that puts each bitmap
/// pixel where `View` puts it.
pub fn draw_indicators(d: &mut impl RaylibDraw, picture: &Picture, world: Camera2D, bitmap: Option<Camera2D>) {
    if !picture.world.is_empty() {
        d.draw_mode2D(world, |mut d, _| {
            for f in &picture.world {
                fill(&mut d, f);
            }
        });
    }
    match bitmap {
        None => draw_screen(d, picture),
        Some(b) => d.draw_mode2D(b, |mut d, _| draw_screen(&mut d, picture)),
    }
}

/// The arrows, the folded counts and every label, in bitmap pixels.
fn draw_screen(d: &mut impl RaylibDraw, picture: &Picture) {
    for f in &picture.screen {
        fill(d, f);
    }
    for label in &picture.labels {
        fill(d, &label.plate);
        d.draw_text(&label.text, label.x, label.y, label.size, label.color);
    }
}

fn fill(d: &mut impl RaylibDraw, f: &Fill) {
    d.draw_rectangle(f.x, f.y, f.w, f.h, f.color);
}
