//! Portals: the turning blue spiral a tank drives into to come out beside
//! another one (docs/teleporting.md). The mechanic is `Game::portal_phase`;
//! this module is only how a portal looks. `PORTAL_FRAMES` baked frames from
//! `static/portal_sheet.png` cycle on the round clock, offset per portal by
//! a position hash so two portals never turn in lockstep, and the frame is
//! blitted unrotated - a rotated blit would smear the 2px blocks. Under an
//! active portal the round adds a soft additive glow built from the same
//! 2px blocks as every other glow (`blast::pixel_disc`).

use sola_raylib::prelude::*;

use crate::blast::{pixel_disc, seed_at};
use crate::canvas::{Canvas, Sheet};
use crate::tuning::tuning;
use crate::{Position, PORTAL_FRAMES, PORTAL_ICON_CELL, PORTAL_ICON_SIZE, PORTAL_SHEET_COLS, PORTAL_TEXTURE_SIZE};

/// The P1 team blues (`tools/punypalette.py`'s `TEAM_P1`, `tank::TEAM_COLORS[0]`):
/// deliberately off the ground palette, so a hole in the ground reads as
/// not-terrain the way a player's ring does - docs/PALETTE.md.
const PORTAL_MID: Color = Color::new(0x4D, 0x65, 0xB4, 255);
const PORTAL_LIGHT: Color = Color::new(0x8F, 0xD3, 0xFF, 255);

/// Which of the `PORTAL_FRAMES` frames a portal centred at `center` shows at `time`:
/// one full cycle per `portal_spin_seconds`, phase-shifted by the position
/// hash so neighbouring portals turn out of step.
pub fn portal_frame(center: Position, time: f32) -> i32 {
    let period = tuning().portal_spin_seconds.max(0.05);
    let step = ((time / period) * PORTAL_FRAMES as f32) as i64;
    let offset = (seed_at(center, 101) % PORTAL_FRAMES as u32) as i64;
    (step + offset).rem_euclid(PORTAL_FRAMES as i64) as i32
}

/// Top-left corner of sheet cell `cell`, counted row-major in rows of
/// `PORTAL_SHEET_COLS`.
fn cell_origin(cell: i32) -> (f32, f32) {
    ((cell % PORTAL_SHEET_COLS) as f32 * PORTAL_TEXTURE_SIZE, (cell / PORTAL_SHEET_COLS) as f32 * PORTAL_TEXTURE_SIZE)
}

/// Source rectangle of rotation frame `frame` on the portal sheet.
pub fn portal_source_rec(frame: i32) -> Rectangle {
    let (x, y) = cell_origin(frame);
    Rectangle::new(x, y, PORTAL_TEXTURE_SIZE, PORTAL_TEXTURE_SIZE)
}

/// Source rectangle of the 32px builder icon in the cell after the last
/// frame.
pub fn portal_icon_source_rec() -> Rectangle {
    let (x, y) = cell_origin(PORTAL_ICON_CELL);
    Rectangle::new(x, y, PORTAL_ICON_SIZE, PORTAL_ICON_SIZE)
}

/// Draw one portal centred on `center` - the frame for `time`, tinted
/// `tint` (the builder ghosts an inactive one this way). Floor level: the
/// round paints these at the tail of `Game::paint_floor`, over tracks and
/// scorches and under everything that stands.
pub fn draw_portal(c: &mut impl Canvas, center: Position, time: f32, tint: Color) {
    let size = PORTAL_TEXTURE_SIZE;
    let dest = Rectangle::new(center.x, center.y, size, size);
    c.blit(Sheet::Portal, portal_source_rec(portal_frame(center, time)), dest, Vector2::new(size / 2.0, size / 2.0), 0.0, tint);
}

/// The additive glow under an active portal: a wide dim disc and a small
/// bright one, breathing slowly. `portal_glow_strength` 0 draws nothing.
pub fn draw_portal_glow(d: &mut impl RaylibDraw, center: Position, time: f32) {
    let strength = tuning().portal_glow_strength;
    if strength <= 0.0 {
        return;
    }
    let breathe = 0.85 + 0.15 * (time * 0.8 + seed_at(center, 102) as f32 * 0.01).sin();
    let alpha = |base: f32| (base * strength * breathe * 255.0).round().clamp(0.0, 255.0) as u8;
    pixel_disc(d, center, 40.0, Color::new(PORTAL_MID.r, PORTAL_MID.g, PORTAL_MID.b, alpha(0.6)));
    pixel_disc(d, center, 16.0, Color::new(PORTAL_LIGHT.r, PORTAL_LIGHT.g, PORTAL_LIGHT.b, alpha(0.9)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Sheet;

    #[test]
    fn frames_cycle_and_the_icon_sits_after_the_last_frame() {
        let at = Position::new(320.0, 160.0);
        let f0 = portal_frame(at, 0.0);
        assert!((0..PORTAL_FRAMES).contains(&f0));
        // A whole period later the same frame shows again; one frame's
        // share of a period later the next frame does.
        let period = tuning().portal_spin_seconds;
        assert_eq!(portal_frame(at, period), f0);
        assert_eq!(portal_frame(at, period / PORTAL_FRAMES as f32 + 0.001), (f0 + 1) % PORTAL_FRAMES);
        // Frames wrap onto the next sheet row after `PORTAL_SHEET_COLS`.
        let first = portal_source_rec(0);
        let wrapped = portal_source_rec(PORTAL_SHEET_COLS);
        assert_eq!((first.x, first.y), (0.0, 0.0));
        assert_eq!((wrapped.x, wrapped.y), (0.0, PORTAL_TEXTURE_SIZE));
        let last = portal_source_rec(PORTAL_FRAMES - 1);
        assert_eq!((last.x, last.y), (((PORTAL_FRAMES - 1) % PORTAL_SHEET_COLS) as f32 * PORTAL_TEXTURE_SIZE, ((PORTAL_FRAMES - 1) / PORTAL_SHEET_COLS) as f32 * PORTAL_TEXTURE_SIZE));
        let icon = portal_icon_source_rec();
        assert_eq!((icon.x, icon.y, icon.width, icon.height), (0.0, (PORTAL_FRAMES / PORTAL_SHEET_COLS) as f32 * PORTAL_TEXTURE_SIZE, 32.0, 32.0));
        assert_eq!(Sheet::Portal.path(), "static/portal_sheet.png");
    }
}
