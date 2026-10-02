//! Fair fire (docs/large-maps-follow-camera.md §5): an enemy shoots at a
//! player only from where that player can see it. On a field map each
//! seat's screen shows a part of the battlefield round its tank
//! (`camera.rs`); the round is told how much (`Game::set_seat_screen`,
//! field pixels, from the window or from the seat's packets) and keeps
//! every enemy's fire at that seat inside the *fire box*: the part of the
//! screen that stays on it wherever the camera's dead zone and look-ahead
//! put the view, less a hull's width so the shooter is wholly in sight.
//! A seat with no screen - a map drawn whole, the probe, a bot - has no
//! box, and the rule is off for it.
//!
//! The look-ahead's room is this module's (`lead_room`), read by the
//! camera too, so the box can never be larger than the view that
//! follows the same numbers. Pure arithmetic on the tuning table: no
//! RNG, no world.

use crate::tuning::Tuning;
use crate::{Position, OBSTACLE_GRID_SIZE};

/// How much of its half-width a view that cannot hold the sight box (a
/// zoomed phone) may lead the tank by.
pub const LEAD_SHARE_ZOOMED: f32 = 0.3;

/// What the fire box keeps clear of the screen's edge beyond the camera's
/// own play: half a hull and a block, so a shooter is on screen whole.
const FIRE_BOX_EDGE_PX: f32 = 22.0;

/// The smallest half-extent a fire box shrinks to, so a tiny screen still
/// lets a tank alongside fire.
const FIRE_BOX_MIN_HALF_PX: f32 = 2.0 * OBSTACLE_GRID_SIZE;

/// How far the camera may lead the tank along one axis of a view `half`
/// wide (half the view's extent) with a sight box `sight` (its half-extent)
/// on that axis: the room outside the sight box, or a share of the view
/// when the view is narrower than the box.
pub fn lead_room(half: f32, sight: f32) -> f32 {
    let margin = OBSTACLE_GRID_SIZE / 4.0;
    if half >= sight + margin { half - sight - margin } else { half * LEAD_SHARE_ZOOMED }
}

/// The fire box's half-extents for a screen of `screen` field pixels: what
/// stays on screen round the tank at the camera's furthest lead and the
/// edge of its dead zone.
pub fn fire_box_half(screen: (f32, f32), t: &Tuning) -> (f32, f32) {
    let cell = OBSTACLE_GRID_SIZE;
    let reach = t.camera_look_ahead_cells * cell;
    let dead = t.camera_dead_zone_cells * cell;
    let axis = |extent: f32, sight_cells: f32| {
        let half = extent / 2.0;
        let lead = reach.min(lead_room(half, sight_cells * cell));
        (half - lead - dead - FIRE_BOX_EDGE_PX).max(FIRE_BOX_MIN_HALF_PX)
    };
    (axis(screen.0, t.camera_sight_x_cells), axis(screen.1, t.camera_sight_y_cells))
}

/// Whether `shooter` stands in the fire box of a seat at `seat` with a
/// screen of `screen` field pixels; always for a seat with none.
pub fn in_fire_box(screen: Option<(f32, f32)>, seat: Position, shooter: Position, t: &Tuning) -> bool {
    let Some(screen) = screen else { return true };
    let (hx, hy) = fire_box_half(screen, t);
    (shooter.x - seat.x).abs() <= hx && (shooter.y - seat.y).abs() <= hy
}

/// Whether the shared alert raised at `alert` reaches an enemy at `enemy`:
/// within `enemy_alert_radius_cells`, or anywhere with the radius at 0.
pub fn alert_reaches(alert: Position, enemy: Position, t: &Tuning) -> bool {
    let radius = t.enemy_alert_radius_cells * OBSTACLE_GRID_SIZE;
    radius <= 0.0 || enemy.distance_to(alert) <= radius
}

#[cfg(test)]
mod sight_tests {
    use super::*;

    /// The box sits inside the screen however the camera leads: half the
    /// screen less the furthest lead and the dead zone.
    #[test]
    fn the_fire_box_is_what_stays_on_screen() {
        let t = Tuning::DEFAULT;
        let desk = (1133.0, 522.0);
        let (hx, hy) = fire_box_half(desk, &t);
        let lead_x = (t.camera_look_ahead_cells * 32.0).min(lead_room(desk.0 / 2.0, t.camera_sight_x_cells * 32.0));
        let lead_y = (t.camera_look_ahead_cells * 32.0).min(lead_room(desk.1 / 2.0, t.camera_sight_y_cells * 32.0));
        let dead = t.camera_dead_zone_cells * 32.0;
        assert!(hx + lead_x + dead < desk.0 / 2.0 && hy + lead_y + dead < desk.1 / 2.0, "({hx}, {hy})");
        assert!(hx > 10.0 * 32.0 && hy > 5.0 * 32.0, "most of a desktop screen: ({hx}, {hy})");
        // A phone's box is smaller, but a tank alongside can still fire.
        let (px, py) = fire_box_half((852.0, 393.0), &t);
        assert!(px < hx && py < hy && py >= FIRE_BOX_MIN_HALF_PX);
    }

    /// The alert reaches across an arena corner to corner, not across a
    /// field map; at 0 it reaches everywhere.
    #[test]
    fn the_alert_reaches_a_radius() {
        let t = Tuning::DEFAULT;
        let alert = Position::new(0.0, 0.0);
        assert!(alert_reaches(alert, Position::new(36.0 * 32.0, 18.0 * 32.0), &t), "an arena's diagonal");
        assert!(!alert_reaches(alert, Position::new(60.0 * 32.0, 30.0 * 32.0), &t), "across a field map");
        let everywhere = Tuning { enemy_alert_radius_cells: 0.0, ..Tuning::DEFAULT };
        assert!(alert_reaches(alert, Position::new(1.0e5, 1.0e5), &everywhere));
    }

    #[test]
    fn a_seat_with_no_screen_has_no_box() {
        let t = Tuning::DEFAULT;
        let seat = Position::new(0.0, 0.0);
        assert!(in_fire_box(None, seat, Position::new(5000.0, 5000.0), &t));
        assert!(in_fire_box(Some((1133.0, 522.0)), seat, Position::new(100.0, 100.0), &t));
        assert!(!in_fire_box(Some((1133.0, 522.0)), seat, Position::new(600.0, 0.0), &t));
        assert!(!in_fire_box(Some((1133.0, 522.0)), seat, Position::new(0.0, 300.0), &t));
    }
}
