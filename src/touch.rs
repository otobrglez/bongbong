//! Touch controls for a keyboard-less device (docs/fullscreen-resolution-
//! research.md section 5, scheme J): nothing drawn at rest, a floating
//! joystick under one thumb and tap-to-fire under the other.
//!
//! The steering half of the field is the right half by default (the
//! `touch_steer_side` knob mirrors it). The first touch that lands there
//! becomes the stick's origin; the drag from that origin picks one of the
//! four directions by its dominant axis, with a dead zone so a resting
//! thumb does not creep and a hysteresis band around the diagonals so a
//! drag near 45 degrees does not flicker between axes; lifting stops the
//! tank, which is the game's own stop rule. Any touch on the other half
//! fires while it is down - a tap is a one-frame press, which is the edge
//! a shell needs, and a hold is what the laser, minigun and flamethrower
//! read. The bar above the field is never scheme input; its buttons are
//! handled by `main.rs` before the scheme sees a frame.
//!
//! The scheme turns raw touch points into the same `Intent` the keyboard
//! produces, so the simulation never learns a touch screen exists. Only a
//! real touch point drives it; the mouse keeps its own role (the bar's
//! buttons, the dialogs, the builder) unless `--touch-from-mouse` asks for
//! a desktop stand-in, which is a development aid rather than a control
//! scheme. Presentation-only feedback (`draw`, the one item here that
//! needs raylib): a faint base and knob while the stick is held, a ripple
//! where a fire tap landed, and a one-time hint the first time a touch is
//! seen.

use crate::math::Vec2;
#[cfg(feature = "render")]
use crate::math::Color;
#[cfg(feature = "render")]
use sola_raylib::prelude::RaylibDraw;

use crate::ai::Intent;
use crate::tank::Dir;
use crate::Layout;

/// Movement below this many bitmap pixels from the origin is a resting
/// thumb, not a direction.
pub const DEAD_ZONE_PX: f32 = 14.0;
/// Once an axis is chosen, the other axis has to exceed it by this factor
/// before the direction switches axes: tan(60 degrees), i.e. a 15 degree
/// band either side of each diagonal.
pub const AXIS_SWITCH_RATIO: f32 = 1.732;
/// How far the drawn knob may sit from the base (the drag itself is not
/// clamped: the direction only cares about the sign and the axis).
pub const KNOB_TRAVEL_PX: f32 = 40.0;
/// How long a fire ripple stays on screen.
pub const RIPPLE_SECONDS: f32 = 0.25;
/// How long the first-touch hint stays up.
pub const HINT_SECONDS: f32 = 4.0;

/// One touch point this frame, in bitmap pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchPoint {
    pub id: i32,
    pub pos: Vec2,
}

#[derive(Clone, Copy, Debug)]
struct Stick {
    id: i32,
    origin: Vec2,
    current: Vec2,
    /// The direction the stick last resolved to, kept for the hysteresis.
    dir: Option<Dir>,
}

#[derive(Clone, Copy, Debug)]
struct Ripple {
    /// Where the tap landed; only the overlay reads it.
    #[cfg_attr(not(feature = "render"), allow(dead_code))]
    at: Vec2,
    age: f32,
}

/// The scheme's state across frames.
#[derive(Debug, Default)]
pub struct TouchScheme {
    stick: Option<Stick>,
    /// Touch ids currently held on the fire half.
    fire_ids: Vec<i32>,
    ripples: Vec<Ripple>,
    /// Seconds of hint left; `None` until the first touch is ever seen.
    hint: Option<f32>,
    /// Whether any touch has been seen this session - the hint fires once.
    seen: bool,
}

impl TouchScheme {
    /// Feed this frame's touch points and get the intent they mean.
    /// `steer_right` puts the stick on the right half of the field (the
    /// fire half is the other one). Points on the bar are ignored.
    pub fn update(&mut self, points: &[TouchPoint], layout: &Layout, steer_right: bool, dt: f32) -> Intent {
        let field = layout.field;
        let split = field.x + field.w / 2.0;
        let on_field = |p: Vec2| p.y >= field.y && p.y < field.y + field.h && p.x >= field.x && p.x < field.x + field.w;
        let on_steer = |p: Vec2| if steer_right { p.x >= split } else { p.x < split };

        if !points.is_empty() && !self.seen {
            self.seen = true;
            self.hint = Some(HINT_SECONDS);
        }
        if let Some(t) = self.hint.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                self.hint = None;
            }
        }
        for r in self.ripples.iter_mut() {
            r.age += dt;
        }
        self.ripples.retain(|r| r.age < RIPPLE_SECONDS);

        // The stick follows its own touch id while that id is still down.
        if let Some(stick) = self.stick.as_mut() {
            match points.iter().find(|p| p.id == stick.id) {
                Some(p) => stick.current = p.pos,
                None => self.stick = None,
            }
        }
        // Fire ids that lifted are gone; new touches are classified by
        // where they landed.
        self.fire_ids.retain(|id| points.iter().any(|p| p.id == *id));
        for p in points {
            let claimed = self.stick.map(|s| s.id == p.id).unwrap_or(false) || self.fire_ids.contains(&p.id);
            if claimed || !on_field(p.pos) {
                continue;
            }
            if on_steer(p.pos) {
                if self.stick.is_none() {
                    self.stick = Some(Stick { id: p.id, origin: p.pos, current: p.pos, dir: None });
                }
            } else {
                self.fire_ids.push(p.id);
                self.ripples.push(Ripple { at: p.pos, age: 0.0 });
            }
        }

        let move_dir = self.stick.as_mut().and_then(|s| {
            s.dir = resolve_dir(s.current.x - s.origin.x, s.current.y - s.origin.y, s.dir);
            s.dir
        });
        Intent { move_dir, fire: !self.fire_ids.is_empty(), ..Intent::default() }
    }

    /// Whether a touch is steering right now.
    pub fn steering(&self) -> bool {
        self.stick.is_some()
    }

    /// The scheme's feedback, drawn in bitmap space over the field after
    /// everything else: the stick while held, fire ripples, the hint.
    #[cfg(feature = "render")]
    pub fn draw(&self, d: &mut impl RaylibDraw, layout: &Layout, steer_right: bool) {
        if let Some(s) = &self.stick {
            let dx = s.current.x - s.origin.x;
            let dy = s.current.y - s.origin.y;
            let len = (dx * dx + dy * dy).sqrt();
            let k = if len > KNOB_TRAVEL_PX { KNOB_TRAVEL_PX / len } else { 1.0 };
            let knob = Vec2::new(s.origin.x + dx * k, s.origin.y + dy * k);
            d.draw_circle_v(s.origin, KNOB_TRAVEL_PX, Color::new(255, 255, 255, 40));
            d.draw_circle_lines(s.origin.x as i32, s.origin.y as i32, KNOB_TRAVEL_PX, Color::new(255, 255, 255, 140));
            d.draw_line_ex(s.origin, knob, 2.0, Color::new(255, 255, 255, 170));
            d.draw_circle_v(knob, 16.0, Color::new(255, 255, 255, 110));
        }
        for r in &self.ripples {
            let t = r.age / RIPPLE_SECONDS;
            let radius = 14.0 + 12.0 * t;
            let alpha = (200.0 * (1.0 - t)) as u8;
            d.draw_circle_lines(r.at.x as i32, r.at.y as i32, radius, Color::new(255, 170, 120, alpha));
        }
        if let Some(t) = self.hint {
            let alpha = ((t / HINT_SECONDS).min(1.0) * 220.0) as u8;
            let field = layout.field;
            let y = (field.y + field.h * 0.5) as i32;
            let (steer_x, fire_x) = if steer_right {
                (field.x + field.w * 0.75, field.x + field.w * 0.25)
            } else {
                (field.x + field.w * 0.25, field.x + field.w * 0.75)
            };
            let color = Color::new(255, 255, 255, alpha);
            let size = 20;
            let steer = "DRAG TO STEER";
            let fire = "TAP TO FIRE";
            d.draw_text(steer, steer_x as i32 - steer.len() as i32 * 5, y, size, color);
            d.draw_text(fire, fire_x as i32 - fire.len() as i32 * 5, y, size, color);
        }
    }
}

/// The direction a drag of (`dx`, `dy`) from the origin means, given the
/// direction it meant last frame: none inside the dead zone, else the
/// dominant axis, and the previous axis is kept until the other one
/// clearly wins (`AXIS_SWITCH_RATIO`).
pub fn resolve_dir(dx: f32, dy: f32, previous: Option<Dir>) -> Option<Dir> {
    let (ax, ay) = (dx.abs(), dy.abs());
    if ax < DEAD_ZONE_PX && ay < DEAD_ZONE_PX {
        return None;
    }
    let horizontal = match previous {
        Some(Dir::Left) | Some(Dir::Right) => ay < ax * AXIS_SWITCH_RATIO,
        Some(Dir::Up) | Some(Dir::Down) => ax >= ay * AXIS_SWITCH_RATIO,
        None => ax >= ay,
    };
    Some(if horizontal {
        if dx >= 0.0 { Dir::Right } else { Dir::Left }
    } else if dy >= 0.0 {
        Dir::Down
    } else {
        Dir::Up
    })
}

#[cfg(test)]
mod touch_tests {
    use super::*;

    fn layout() -> Layout {
        Layout::for_field(960.0, 480.0)
    }

    fn pt(id: i32, x: f32, y: f32) -> TouchPoint {
        TouchPoint { id, pos: Vec2::new(x, y) }
    }

    #[test]
    fn a_resting_thumb_inside_the_dead_zone_is_no_direction() {
        assert_eq!(resolve_dir(5.0, -8.0, None), None);
        assert_eq!(resolve_dir(DEAD_ZONE_PX, 0.0, None), Some(Dir::Right));
    }

    #[test]
    fn the_dominant_axis_wins_and_the_previous_axis_is_sticky_near_the_diagonal() {
        assert_eq!(resolve_dir(30.0, -20.0, None), Some(Dir::Right));
        assert_eq!(resolve_dir(-20.0, 30.0, None), Some(Dir::Down));
        // 40 degrees off horizontal: a fresh drag says horizontal, and a
        // stick already vertical stays vertical.
        assert_eq!(resolve_dir(30.0, 25.0, None), Some(Dir::Right));
        assert_eq!(resolve_dir(30.0, 25.0, Some(Dir::Down)), Some(Dir::Down));
        // Past the band the switch happens.
        assert_eq!(resolve_dir(60.0, 25.0, Some(Dir::Down)), Some(Dir::Right));
        // A sign flip along the held axis is immediate.
        assert_eq!(resolve_dir(-30.0, 5.0, Some(Dir::Right)), Some(Dir::Left));
    }

    #[test]
    fn a_drag_on_the_steer_half_moves_and_lifting_stops() {
        let l = layout();
        let mut t = TouchScheme::default();
        // Touch down on the right half, inside the field.
        let i = t.update(&[pt(1, 700.0, 300.0)], &l, true, 1.0 / 60.0);
        assert_eq!(i.move_dir, None);
        assert!(!i.fire);
        // Drag up past the dead zone.
        let i = t.update(&[pt(1, 700.0, 260.0)], &l, true, 1.0 / 60.0);
        assert_eq!(i.move_dir, Some(Dir::Up));
        // Lift: the tank stops.
        let i = t.update(&[], &l, true, 1.0 / 60.0);
        assert_eq!(i.move_dir, None);
        assert!(!t.steering());
    }

    #[test]
    fn a_touch_on_the_fire_half_fires_while_held_and_the_halves_mirror() {
        let l = layout();
        let mut t = TouchScheme::default();
        let i = t.update(&[pt(2, 200.0, 300.0)], &l, true, 1.0 / 60.0);
        assert!(i.fire);
        assert_eq!(i.move_dir, None);
        let i = t.update(&[], &l, true, 1.0 / 60.0);
        assert!(!i.fire);
        // Left-handed: the same point steers.
        let mut t = TouchScheme::default();
        t.update(&[pt(3, 200.0, 300.0)], &l, false, 1.0 / 60.0);
        assert!(t.steering());
    }

    #[test]
    fn two_thumbs_steer_and_fire_at_once_and_the_bar_is_ignored() {
        let l = layout();
        let mut t = TouchScheme::default();
        t.update(&[pt(1, 700.0, 300.0)], &l, true, 1.0 / 60.0);
        let i = t.update(&[pt(1, 660.0, 300.0), pt(2, 150.0, 200.0)], &l, true, 1.0 / 60.0);
        assert_eq!(i.move_dir, Some(Dir::Left));
        assert!(i.fire);
        // A touch on the bar (above the field) is neither.
        let mut t = TouchScheme::default();
        let i = t.update(&[pt(4, 700.0, 10.0)], &l, true, 1.0 / 60.0);
        assert_eq!(i.move_dir, None);
        assert!(!i.fire);
        assert!(!t.steering());
    }

    #[test]
    fn a_second_touch_on_the_steer_half_does_not_steal_the_stick() {
        let l = layout();
        let mut t = TouchScheme::default();
        t.update(&[pt(1, 700.0, 300.0)], &l, true, 1.0 / 60.0);
        let i = t.update(&[pt(1, 700.0, 340.0), pt(9, 800.0, 100.0)], &l, true, 1.0 / 60.0);
        assert_eq!(i.move_dir, Some(Dir::Down));
        assert!(!i.fire);
    }
}
