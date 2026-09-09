//! Tall grass: cover a tank can sit in, and the only vegetation that is
//! *not* an obstacle.
//!
//! **Why this is not an `Obstacle`.** `Game::nav_grid` feeds every
//! `Obstacle` into pathfinding with no material filter, and `maplint`
//! reasons the same way, so anything that is one is unconditionally
//! impassable to the AI and a wall to the linter. Grass you drive through
//! cannot be that. It is modelled on `decal::Decal` instead - owned by the
//! simulation, seeded from a position hash so it never draws RNG, and
//! rebuilt only when a round starts.
//!
//! **What it looks like.** A map marks whole cells as tall grass; each cell
//! scatters several small tufts at hashed offsets. That scatter is the
//! point: the reference (docs/grass-improvements.md, `grass-hidding.gif`)
//! never hides a unit completely - at worst about 44% of it is covered,
//! bottom-weighted, because the unit walks *between* discrete tufts rather
//! than into a solid block. One sprite per cell would either occlude
//! everything or nothing.
//!
//! **How it moves.** Each tuft leans on a sine of the round clock plus its
//! own hashed phase, so a field ripples instead of swaying in lockstep, and
//! bends away from any tank close enough to push through it. Both are
//! draw-time offsets computed on the CPU - there is no shader, because
//! every shader here needs a hand-ported GLSL ES 100 twin for the web build
//! (see `main.rs`'s `shader_path`) and a sine does not justify that.
//!
//! Note the parting is *our* addition: the reference gif sways ambiently
//! and does not react to the unit at all.

use sola_raylib::prelude::*;

use crate::tuning::tuning;
use crate::{GRASS_SPECIES, GRASS_TEXTURE_SIZE, GRASS_VARIANTS, OBSTACLE_GRID_SIZE, Position};

/// One drawn tuft. Several of these make up a single grass cell.
pub struct GrassTuft {
    /// Where its base sits, in world pixels.
    pub base: Position,
    /// Sheet row (species) and column (variant), both hashed from `base`.
    pub row: i32,
    pub col: i32,
    /// Cosmetic seed - drives the sway phase and the horizontal mirror.
    pub seed: u32,
}

/// The tufts one grass cell scatters. Everything - count, offsets, species,
/// variant, phase - comes out of `blast::seed_at` salted per tuft, so a
/// grass field is identical on replay and costs the round RNG nothing.
pub fn tufts_for_cell(center: Position) -> Vec<GrassTuft> {
    let per_cell = tuning().grass_tufts_per_cell.max(0);
    let half = OBSTACLE_GRID_SIZE / 2.0;
    (0..per_cell)
        .map(|i| {
            let h = crate::blast::seed_at(center, 60 + i as u32 * 5);
            // Spread across the cell, but keep the base inside it so a
            // tuft belongs to the cell that owns it.
            let ox = (h % 1000) as f32 / 1000.0 * OBSTACLE_GRID_SIZE - half;
            let oy = ((h >> 10) % 1000) as f32 / 1000.0 * OBSTACLE_GRID_SIZE - half;
            GrassTuft {
                base: Position::new(center.x + ox, center.y + oy),
                row: ((h >> 20) % GRASS_SPECIES as u32) as i32,
                col: ((h >> 24) % GRASS_VARIANTS as u32) as i32,
                seed: h,
            }
        })
        .collect()
}

/// Is `p` inside a tall-grass cell?
///
/// A point query against the *cells*, not the tufts: cover is a property of
/// the ground a tank is standing on, and testing against scattered sprites
/// would make concealment depend on exactly which way the wind blew.
pub fn conceals(cells: &[Position], p: Position) -> bool {
    let half = OBSTACLE_GRID_SIZE / 2.0;
    cells
        .iter()
        .any(|c| (p.x - c.x).abs() <= half && (p.y - c.y).abs() <= half)
}

/// How far this tuft's tip leans right now, in world px.
///
/// Ambient wind plus a shove from any tank close enough to be pushing
/// through it. Only the *tip* moves - the base is rooted - which is what
/// the vertical skew in `draw_tuft` produces.
fn bend(tuft: &GrassTuft, time: f32, movers: &[Position]) -> f32 {
    let t = tuning();
    // Per-tuft phase, so a field ripples rather than swaying as one sheet.
    let phase = (tuft.seed % 628) as f32 * 0.01;
    let mut lean = (time * t.grass_sway_speed + phase).sin() * t.grass_sway_px;
    let reach = t.grass_part_radius;
    if reach > 0.0 {
        for m in movers {
            let dx = tuft.base.x - m.x;
            let dy = tuft.base.y - m.y;
            let d = (dx * dx + dy * dy).sqrt();
            if d < reach && d > 0.001 {
                // Push away from the tank, hardest at the centre.
                lean += (dx / d) * (1.0 - d / reach) * t.grass_part_px;
            }
        }
    }
    lean
}

/// Draw one tuft, leaning. `movers` is every live tank, so grass parts
/// where one is driving through it.
pub fn draw_tuft(d: &mut impl RaylibDraw, texture: &Texture2D, tuft: &GrassTuft, time: f32, movers: &[Position]) {
    let cell = GRASS_TEXTURE_SIZE;
    let scale = tuning().grass_scale;
    let size = cell * scale;
    let flip = if tuft.seed & 1 != 0 { -1.0 } else { 1.0 };
    let src = Rectangle::new(tuft.col as f32 * cell, tuft.row as f32 * cell, cell * flip, cell);
    // Rotating about the base is what makes the tip move and the root stay
    // put; raylib rotates about `origin`, so the origin sits at the bottom
    // centre of the sprite.
    let lean = bend(tuft, time, movers);
    let rotation = lean.atan2(size).to_degrees();
    let dest = Rectangle::new(tuft.base.x, tuft.base.y, size, size);
    let origin = Vector2::new(size / 2.0, size);
    d.draw_texture_pro(texture, src, dest, origin, rotation, Color::WHITE);
}
