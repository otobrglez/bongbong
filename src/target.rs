//! The range board's fire, headless (docs/range-target-prd.md): what is
//! drawn over the board's sheet cell while a flame works on it, in the
//! effects language (docs/effects.md, `pyro.rs`). The sheet carries the
//! char - the three burn columns `Obstacle::col` steps through by how far
//! the fire has got - and the flames standing on it are every burning
//! tile's (`game::tile_flames`); this adds the two things that move on
//! the board itself:
//!
//! - **Scorching**: under the flamethrower's stream, before it catches,
//!   soot creeps in from the rim as the board's heat rises toward
//!   `flame_ignite_seconds`, dithered through the field's Bayer pattern.
//! - **Embers**: once it burns, blocks of the char glow and fade on their
//!   own hashed cadences, more of them and hotter as the fire spreads,
//!   fewer and duller as the board chars out.
//!
//! A pure function of the obstacle: no RNG, nothing kept.
//! The embers run on `Obstacle::burn_shown`, so a client replica, which
//! is told only that the board burns, animates them on its own clock. The
//! scorch reads `Obstacle::heat`, which only the simulating round has.

use crate::math::Color;
use crate::obstacle::{Material, Obstacle};
use crate::pyro::{self, Shape, BLOCK, CHAR, FIRE};
use crate::Position;

/// Where the board's face sits on its cell, in px from the cell's centre:
/// the generator's disc (`gen_props.py`'s `TGT_CX`/`TGT_CY`) one pixel
/// above the middle, its radius `TGT_R` macro pixels at two px each.
const FACE_DY: f32 = -1.0;
const FACE_RADIUS: f32 = 12.8;

/// Most embers a board shows at the height of its fire.
const MAX_EMBERS: u32 = 9;

/// Everything drawn over a range board this frame for its fire: scorch
/// marks while it heats, ember marks while it burns. Empty for anything
/// that is not a board, and for a cold one.
pub fn fire_shapes(obstacle: &Obstacle, ignite_seconds: f32) -> Vec<Shape> {
    let mut out = Vec::new();
    if obstacle.material != Material::Target || obstacle.destroyed {
        return out;
    }
    let centre = Position::new(obstacle.position.x, obstacle.position.y + FACE_DY + obstacle.burn_sag());
    if obstacle.burning {
        embers(&mut out, centre, obstacle);
    } else if obstacle.heat > 0.0 && ignite_seconds > 0.0 {
        scorch(&mut out, centre, (obstacle.heat / ignite_seconds).clamp(0.0, 1.0));
    }
    out
}

/// Soot creeping in from the rim: a block of the face is sooted once the
/// heat has reached its depth, the front of it dithered so it creeps
/// rather than steps. `heat` is 0 cold to 1 at the moment it catches.
fn scorch(out: &mut Vec<Shape>, centre: Position, heat: f32) {
    let reach = (FACE_RADIUS / BLOCK).ceil() as i32;
    let (cx, cy) = pyro::block_of(centre.x, centre.y);
    for by in cy - reach..=cy + reach {
        for bx in cx - reach..=cx + reach {
            let at = Position::new((bx as f32 + 0.5) * BLOCK, (by as f32 + 0.5) * BLOCK);
            let d = at.distance_to(centre) / FACE_RADIUS;
            if d > 1.0 {
                continue;
            }
            // The rim first: depth 0 at the edge, 1 at the middle. The
            // heat reaches two thirds of the way in by the time it lights.
            let depth = 1.0 - d;
            let front = heat * 0.7 - depth;
            if front <= 0.0 || pyro::bayer(bx, by) > (front * 4.0).min(1.0) {
                continue;
            }
            let colour = if front > 0.25 { CHAR[0] } else { CHAR[1] };
            out.push(Shape::Mark { pos: at, size: 2, color: pyro::alpha(colour, 0.85) });
        }
    }
}

/// The burning board's embers: up to `MAX_EMBERS` blocks of the face, each
/// at a spot and on a cadence hashed from the board's position and its
/// number, glowing up the fire ramp and fading back. The count swells to
/// the middle of the burn and falls off as the board chars out; the
/// hottest step a spot reaches falls with it.
fn embers(out: &mut Vec<Shape>, centre: Position, obstacle: &Obstacle) {
    let progress = obstacle.burn_progress();
    let swell = (progress * 3.0).min(1.0) * (1.0 - ((progress - 0.6).max(0.0) / 0.4)).max(0.25);
    let count = (MAX_EMBERS as f32 * swell).round().max(2.0) as u32;
    let seed = crate::blast::seed_at(obstacle.position, 47);
    let hottest = if progress < 0.7 { 5 } else { 3 };
    for i in 0..count {
        let angle = pyro::unit(seed, i * 3) * std::f32::consts::TAU;
        let r = FACE_RADIUS * 0.85 * pyro::unit(seed, i * 3 + 1).sqrt();
        let at = Position::new(centre.x + angle.cos() * r, centre.y + angle.sin() * r);
        let rate = 1.5 + 2.5 * pyro::unit(seed, i * 3 + 2);
        let phase = pyro::unit(seed ^ 0x9e37, i) * std::f32::consts::TAU;
        let glow = 0.5 + 0.5 * (obstacle.burn_shown * rate * std::f32::consts::TAU / 2.0 + phase).sin();
        // Below a third the ember is out: only the char shows.
        if glow < 0.33 {
            continue;
        }
        let step = 1 + ((glow - 0.33) / 0.67 * (hottest as f32 - 1.0)).round() as usize;
        out.push(Shape::Mark { pos: at, size: 2, color: ember(step) });
    }
}

/// The fire ramp's `step`, dark red up to gold.
fn ember(step: usize) -> Color {
    FIRE[step.min(FIRE.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rapier2d::prelude::RigidBodyHandle;

    fn board() -> Obstacle {
        Obstacle::new(Material::Target, 0, Position::new(160.0, 96.0), false, RigidBodyHandle::invalid())
    }

    fn marks(shapes: &[Shape]) -> Vec<(Position, Color)> {
        shapes
            .iter()
            .filter_map(|s| match *s {
                Shape::Mark { pos, color, .. } => Some((pos, color)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_cold_board_draws_nothing() {
        assert!(fire_shapes(&board(), 0.35).is_empty());
    }

    #[test]
    fn only_a_board_gets_the_animation() {
        let mut plank = Obstacle::new(Material::Wood, 0, Position::new(160.0, 96.0), true, RigidBodyHandle::invalid());
        plank.burning = true;
        plank.heat = 0.2;
        assert!(fire_shapes(&plank, 0.35).is_empty());
    }

    #[test]
    fn the_scorch_creeps_in_from_the_rim_as_it_heats() {
        let mut b = board();
        b.heat = 0.1;
        let early = marks(&fire_shapes(&b, 0.35));
        b.heat = 0.34;
        let late = marks(&fire_shapes(&b, 0.35));
        assert!(!early.is_empty() && late.len() > early.len(), "{} then {}", early.len(), late.len());
        let centre = Position::new(160.0, 95.0);
        let nearest = |m: &[(Position, Color)]| m.iter().map(|(p, _)| p.distance_to(centre)).fold(f32::MAX, f32::min);
        assert!(nearest(&late) < nearest(&early), "the soot reaches further in");
        // Never the middle: the gold still shows as it catches.
        assert!(nearest(&late) > 2.0);
        for (p, c) in &late {
            assert!(CHAR.iter().any(|k| k.r == c.r && k.g == c.g && k.b == c.b), "char steps only");
            assert_eq!(p.x.rem_euclid(BLOCK), 1.0, "on the block grid");
        }
    }

    #[test]
    fn embers_glow_on_the_face_and_move_with_the_fire() {
        let mut b = board();
        b.burning = true;
        let mut seen = std::collections::BTreeSet::new();
        for i in 0..40 {
            b.burn_shown = i as f32 * 0.05;
            let m = marks(&fire_shapes(&b, 0.35));
            for (p, c) in &m {
                assert!(p.distance_to(Position::new(160.0, 95.0)) <= FACE_RADIUS + BLOCK, "on the face");
                assert!(FIRE.iter().any(|k| k == c), "fire steps only");
            }
            seen.insert(m.len());
        }
        assert!(seen.len() > 2, "the embers pulse: {seen:?}");
    }

    #[test]
    fn the_board_chars_in_order_and_sags_at_the_end_on_any_clock() {
        // `tick_burn_frame` is all a replica runs, so the drawn burn has to
        // move on it alone.
        let mut b = board();
        b.burning = true;
        let burn = Material::Target.burn_seconds();
        let mut last = 0.0;
        let mut sagged_at = None;
        let steps = (burn * 60.0) as usize;
        for i in 0..steps {
            b.tick_burn_frame(1.0 / 60.0);
            let p = b.burn_progress();
            assert!(p >= last, "the char never goes back");
            last = p;
            if b.burn_sag() > 0.0 && sagged_at.is_none() {
                sagged_at = Some(i as f32 / steps as f32);
            }
        }
        assert!(last > 0.95, "the drawn burn reaches the end: {last}");
        let sagged_at = sagged_at.expect("the stand gives way");
        assert!((0.75..0.85).contains(&sagged_at), "in the last fifth: {sagged_at}");
        assert_eq!(b.burn_sag(), BLOCK, "one whole block");
        assert_eq!(b.burn_elapsed, 0.0, "the drawn clock is not the simulation's");
    }

    #[test]
    fn the_same_inputs_give_the_same_picture() {
        let mut b = board();
        b.burning = true;
        b.burn_shown = 1.3;
        assert_eq!(marks(&fire_shapes(&b, 0.35)), marks(&fire_shapes(&b, 0.35)));
    }
}
