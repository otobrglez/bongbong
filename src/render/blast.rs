//! The glows a live round draws with raylib directly - a lit fuse, a
//! burning cell, the flamethrower's nozzle - as stepped block discs
//! (`render::shot_fx::glow`) inside an additive blend block. The blasts
//! themselves are composed in `fireball.rs` and `mushroom.rs`, the flames
//! in `pyro::tongues` (a burning hull's and tile's carry their own light,
//! `damage_stage::flames`, `game::tile_flames`); the scorch decal, generic
//! over `Canvas`, stays in `blast.rs`.

use sola_raylib::prelude::*;

use crate::blast::seed_at;
use crate::math::{Color, Vec2};
use crate::render::shot_fx::glow;
use crate::tuning::tuning;
use crate::Position;

/// A filled disc of whole 2 px blocks on the field's grid
/// (`pyro::block_disc`), for glows that pulse rather than fall off: a lit
/// fuse, a portal's core.
pub fn pixel_disc(d: &mut impl RaylibDraw, center: Position, radius: f32, color: Color) {
    crate::pyro::block_disc(&mut crate::render::pyro::Rl(d), center, radius, color);
}

/// The flamethrower's nozzle glow while it fires (additive): a hot soft
/// pool at the muzzle and a fainter, larger one part way down the stream,
/// both flickering off the round clock - the light the stream (the jet
/// shader and the motes in `fx.rs`) throws on the ground.
pub fn draw_flame_glow(d: &mut impl RaylibDraw, origin: Position, dir: Vec2, reach: f32, time: f32) {
    let flicker = 0.75 + 0.25 * (time * 47.0).sin();
    let hot = Color::new(255, 200, 90, (150.0 * flicker) as u8);
    glow(d, origin, 12.0, hot);
    let mid = Position::new(origin.x + dir.x * reach * 0.4, origin.y + dir.y * reach * 0.4);
    let warm = Color::new(255, 120, 40, (45.0 * flicker) as u8);
    glow(d, mid, (reach * 0.22).max(8.0), warm);
}

/// The pulsing glow on a barrel whose fuse is lit (additive, like the
/// bloom). `time` is the round clock, only used to phase the pulse.
pub fn draw_fuse_glow(d: &mut impl RaylibDraw, center: Position, time: f32) {
    let pulse = 0.5 + 0.5 * (time * 50.0).sin();
    let a = (255.0 * tuning().barrel_fuse_glow_strength * pulse) as u8;
    pixel_disc(d, center, 22.0, Color::new(255, 120, 40, a));
}

/// The glow of a burning ground cell (an oil pool or a lit trail): a soft
/// additive pool of light, flickering at its own hashed phase, dying down
/// over the last part of `left`.
pub fn draw_fire_glow(d: &mut impl RaylibDraw, center: Position, time: f32, left: f32) {
    let phase = (seed_at(center, 23) % 100) as f32 / 100.0 * std::f32::consts::TAU;
    let flicker = 0.7 + 0.3 * (time * 31.0 + phase).sin();
    let dying = (left / 0.6).clamp(0.0, 1.0);
    let a = (255.0 * 0.3 * flicker * dying) as u8;
    glow(d, center, 26.0, Color::new(255, 130, 40, a));
}
