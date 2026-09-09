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
//! own hashed phase, so a field ripples instead of swaying in lockstep.
//! That is a draw-time offset computed on the CPU - there is no shader,
//! because every shader here needs a hand-ported GLSL ES 100 twin for the
//! web build (see `main.rs`'s `shader_path`) and a sine does not justify
//! that.
//!
//! **How a tank goes through it.** `tick` carries two more values per tuft,
//! both pure functions of where the tanks are - no RNG, so a seeded replay
//! is unaffected:
//!
//! - `crush`, how flat the tuft is lying. It goes to 1 under a hull and
//!   recovers over `grass_crush_recover_seconds`, which is the whole trail
//!   effect: grass stays matted behind a tank and stands back up, the same
//!   shape `track.rs` gives a tread mark. Persistence is also *why* the
//!   wake does not need to be shaped by the direction of travel - the trail
//!   is left behind by time, not by geometry.
//! - `push`, the sideways shove. It is stronger for grass *ahead* of a
//!   moving tank than behind it, so a hull drives a bow wave rather than
//!   a symmetric ring.
//!
//! Both are additions of ours: the reference gif sways ambiently and does
//! not react to the unit at all.
//!
//! **Depth.** `game.rs` draws tufts interleaved with tanks by ground-contact
//! y, so a tuft rooted behind a tank is drawn behind it. Drawing all grass
//! last - which is what it used to do - buries a tank in grass that is
//! rooted well past it.

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
    /// How flat this tuft is lying: 0 standing, 1 fully matted. Driven to 1
    /// under a hull and recovered over `grass_crush_recover_seconds` - see
    /// the module comment on why that recovery is the trail.
    pub crush: f32,
    /// Sideways shove from a passing hull, in px of travel at the tip.
    /// Signed; positive is to the right. Added to the ambient sway.
    pub push: f32,
}

/// A tank as the grass sees it. Velocity is the *body's*, not
/// `Tank::velocity` - that one is the commanded cardinal vector and reads
/// as zero the moment the driver lets go, while the hull is still moving.
pub struct Mover {
    pub position: Position,
    pub velocity: Position,
    /// Half-width of the hull's footprint. Flattening is measured from this
    /// *box*, not from the tank's centre: a radial falloff from the centre
    /// leaves the grass under the tracks standing, because the hull is
    /// wider than any radius small enough to look right.
    pub half: f32,
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
                crush: 0.0,
                push: 0.0,
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

/// Advance every tuft against the tanks driving through it.
///
/// Called once per simulation frame (`Game::tick_grass`) rather than at
/// draw time, so the crush recovery is frame-rate independent and the trail
/// behind a tank is the same length however fast the game renders. Pure in
/// the tank positions - it draws no RNG and touches nothing a seeded replay
/// can see.
pub fn tick(tufts: &mut [GrassTuft], movers: &[Mover], dt: f32) {
    let t = tuning();
    let (crush_r, wake_r) = (t.grass_crush_radius, t.grass_part_radius);
    let recover = t.grass_crush_recover_seconds.max(0.05);
    for tuft in tufts.iter_mut() {
        let mut target = 0.0f32;
        let mut push = 0.0f32;
        for m in movers {
            let dx = tuft.base.x - m.position.x;
            let dy = tuft.base.y - m.position.y;
            let d = (dx * dx + dy * dy).sqrt();
            // Distance to the hull box rather than to its centre: zero
            // anywhere under the tank, then growing outward.
            let outside = {
                let ox = (dx.abs() - m.half).max(0.0);
                let oy = (dy.abs() - m.half).max(0.0);
                (ox * ox + oy * oy).sqrt()
            };
            if crush_r > 0.0 && outside < crush_r {
                // Smoothstepped so the flattened patch has no hard rim.
                let f = 1.0 - outside / crush_r;
                target = target.max(f * f * (3.0 - 2.0 * f));
            }
            if wake_r > 0.0 && d < wake_r && d > 0.001 {
                let speed = (m.velocity.x * m.velocity.x + m.velocity.y * m.velocity.y).sqrt();
                // How far in front of the hull this tuft is, 0 behind to 1
                // dead ahead: a moving tank drives a bow wave, a parked one
                // just parts what it is sitting in.
                let ahead = if speed > 8.0 {
                    ((dx * m.velocity.x + dy * m.velocity.y) / (d * speed)).max(0.0)
                } else {
                    0.5
                };
                push += (dx / d) * (1.0 - d / wake_r) * t.grass_part_px * (0.45 + 0.75 * ahead);
            }
        }
        // Flattening is immediate - a hull does not ease grass down - but
        // standing back up takes the whole recovery, which is the trail.
        tuft.crush = if target > tuft.crush { target } else { (tuft.crush - dt / recover).max(0.0) };
        tuft.push = push;
    }
}

/// How far this tuft's tip leans right now, in world px: ambient wind plus
/// whatever `tick` last recorded. Matted grass barely waves, so the wind is
/// scaled down by the crush.
fn bend(tuft: &GrassTuft, time: f32) -> f32 {
    let t = tuning();
    // Per-tuft phase, so a field ripples rather than swaying as one sheet.
    let phase = (tuft.seed % 628) as f32 * 0.01;
    let wind = (time * t.grass_sway_speed + phase).sin() * t.grass_sway_px;
    wind * (1.0 - tuft.crush) + tuft.push
}

/// Draw one tuft, leaning and squashed by however flat it is lying.
pub fn draw_tuft(d: &mut impl RaylibDraw, texture: &Texture2D, tuft: &GrassTuft, time: f32) {
    let cell = GRASS_TEXTURE_SIZE;
    let t = tuning();
    let size = cell * t.grass_scale;
    let flip = if tuft.seed & 1 != 0 { -1.0 } else { 1.0 };
    let src = Rectangle::new(tuft.col as f32 * cell, tuft.row as f32 * cell, cell * flip, cell);
    // Matted grass is *foreshortened*, not cropped: the blades bend over
    // rather than being mown, so the sprite squashes toward its own root.
    // Never all the way - `grass_crush_flatten` leaves a stub, because a
    // tuft that vanishes entirely reads as a hole in the field.
    let height = size * (1.0 - tuft.crush * t.grass_crush_flatten);
    // Rotating about the base is what makes the tip move and the root stay
    // put; raylib rotates about `origin`, so the origin sits at the bottom
    // centre of the sprite.
    let rotation = bend(tuft, time).atan2(size).to_degrees();
    let dest = Rectangle::new(tuft.base.x, tuft.base.y, size, height);
    let origin = Vector2::new(size / 2.0, height);
    d.draw_texture_pro(texture, src, dest, origin, rotation, Color::WHITE);
}
