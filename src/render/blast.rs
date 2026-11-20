//! The glows a live round draws with raylib directly - a lit fuse, a
//! burning cell, the flamethrower's nozzle - as stepped block discs
//! (`render::shot_fx::glow`) inside an additive blend block. The blasts
//! themselves are composed in `fireball.rs` and `mushroom.rs`, the flames
//! in `pyro::tongues` (a burning hull's and tile's carry their own light,
//! `damage_stage::flames`, `game::tile_flames`); the scorch decal, generic
//! over `Canvas`, stays in `blast.rs`.

use sola_raylib::prelude::*;

use crate::blast::{seed_at, FireGroup};
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
/// bloom). `time` is the round clock, only used to phase the pulse, which
/// reduced motion slows to a held beat (`motion::beat`).
pub fn draw_fuse_glow(d: &mut impl RaylibDraw, center: Position, time: f32) {
    let pulse = crate::motion::beat(time, 0.5 + 0.5 * (time * 50.0).sin(), crate::motion::reduced());
    let lit = crate::pyro::LIGHT.fuse;
    let a = (255.0 * tuning().barrel_fuse_glow_strength * pulse) as u8;
    pixel_disc(d, center, 22.0, Color::new(lit.r, lit.g, lit.b, a));
}

/// The glow of burning ground (an oil pool or a lit trail), one for the
/// whole of a fire however many cells burn in it (`blast::fire_groups`):
/// a soft additive pool of light as wide as the fire, flickering at its
/// own hashed phase, dying down as its last cell does, as strong as
/// `fire_glow_strength`.
pub fn draw_fire_glow(d: &mut impl RaylibDraw, fire: &FireGroup, time: f32) {
    let phase = (seed_at(fire.centre, 23) % 100) as f32 / 100.0 * std::f32::consts::TAU;
    let flicker = 0.7 + 0.3 * (time * 31.0 + phase).sin();
    let dying = (fire.left / 0.6).clamp(0.0, 1.0);
    let lit = crate::pyro::LIGHT.fire;
    let a = (255.0 * tuning().fire_glow_strength * flicker * dying) as u8;
    glow(d, fire.centre, FIRE_GLOW_RADIUS + fire.reach, Color::new(lit.r, lit.g, lit.b, a));
}

/// How far one burning cell's glow reaches from its centre, px; a fire of
/// several reaches that far past its furthest cell.
const FIRE_GLOW_RADIUS: f32 = 26.0;
