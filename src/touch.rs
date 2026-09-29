//! Touch controls for a keyboard-less device (docs/fullscreen-resolution-
//! research.md section 5, scheme J): nothing drawn at rest, a floating
//! joystick under one thumb and tap-to-fire under the other.
//!
//! The steering half of the field is the left half - movement under the
//! left thumb, a gamepad's convention - unless the build was made with the
//! `touch-steer-right` cargo feature (`TOUCH_STEER_RIGHT`), which mirrors
//! it: a build-time choice on purpose, so no panel, file or dev tool can
//! flip the side under a player. The first touch that lands there becomes
//! the stick's origin, and the origin then trails the thumb: once the drag is longer
//! than `touch_follow_radius_px` the origin is pulled along behind it, so
//! the stick only ever measures the last few dozen pixels of motion and a
//! change of direction costs the same short slide however far the thumb
//! has pushed. The drag from that origin picks one of the four directions
//! by its dominant axis, with a dead zone (`touch_dead_zone_px`) so a
//! resting thumb does not creep and a hysteresis band (the held axis keeps
//! the direction until the drag is `touch_axis_switch_deg` off it) so a
//! drag near a diagonal does not flicker between axes; lifting stops the
//! tank, which is the game's own stop rule. The three numbers travel as a
//! `StickRule`, read from the tuning table once per frame. Any touch on
//! the other half fires while it is down - a tap is a one-frame press,
//! which is the edge a shell needs, and a hold is what the laser, minigun
//! and flamethrower read. The bar above the field is never scheme input;
//! its buttons are handled by `app.rs` before the scheme sees a frame.
//!
//! The scheme turns raw touch points into the same `Intent` the keyboard
//! produces, so the simulation never learns a touch screen exists. Only a
//! real touch point drives it; the mouse keeps its own role (the bar's
//! buttons, the dialogs, the builder) unless `--touch-from-mouse` asks for
//! a desktop stand-in, which is a development aid rather than a control
//! scheme. Presentation-only feedback (`draw`, the one item here that
//! needs raylib): a faint base and knob while the stick is held - the base
//! drawn at the origin the rule measures from, so the picture can never
//! point somewhere the tank is not going -, a ripple where a fire tap
//! landed, and a one-time hint the first time a touch is seen.

use crate::math::Vec2;
#[cfg(feature = "render")]
use crate::math::Color;
#[cfg(feature = "render")]
use sola_raylib::prelude::RaylibDraw;

use crate::ai::Intent;
use crate::tank::Dir;
use crate::tuning::tuning;
use crate::Layout;

/// How far the drawn knob may sit from the base when the origin is pinned
/// (`touch_follow_radius_px` at 0); with a trailing origin the leash is the
/// base's radius, and the knob is at most that far by construction.
pub const KNOB_TRAVEL_PX: f32 = 40.0;
/// How long a fire ripple stays on screen.
pub const RIPPLE_SECONDS: f32 = 0.25;
/// How long the first-touch hint stays up.
pub const HINT_SECONDS: f32 = 4.0;

/// The three numbers a drag is resolved by: the tuning rows, read once per
/// frame by `StickRule::current` and passed in explicitly, so a test
/// describes the rule it exercises and never touches the global table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StickRule {
    /// A drag shorter than this on both axes is a resting thumb.
    pub dead_zone_px: f32,
    /// The origin trails the thumb so the drag never exceeds this; 0 pins
    /// it where the thumb landed.
    pub follow_radius_px: f32,
    /// Degrees off the held axis a drag has to reach before the other axis
    /// takes over.
    pub axis_switch_deg: f32,
}

impl StickRule {
    /// The rule the tuning table holds this frame.
    pub fn current() -> Self {
        let t = tuning();
        StickRule {
            dead_zone_px: t.touch_dead_zone_px,
            follow_radius_px: t.touch_follow_radius_px,
            axis_switch_deg: t.touch_axis_switch_deg,
        }
    }

    /// The factor the other axis has to exceed the held one by before the
    /// direction switches: the tangent of the switch angle.
    fn switch_ratio(&self) -> f32 {
        self.axis_switch_deg.to_radians().tan()
    }

    /// The leash on a trailing origin, and the radius of the drawn base.
    fn radius(&self) -> f32 {
        if self.follow_radius_px > 0.0 { self.follow_radius_px } else { KNOB_TRAVEL_PX }
    }
}

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
    /// The base's radius this frame (`StickRule::radius`), kept on the
    /// stick so the drawing uses the number the rule did.
    radius: f32,
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
    /// Feed this frame's touch points and get the intent they mean, under
    /// the rule the tuning table holds. `steer_right` puts the stick on
    /// the right half of the field (the fire half is the other one).
    /// Points on the bar are ignored.
    pub fn update(&mut self, points: &[TouchPoint], layout: &Layout, steer_right: bool, dt: f32) -> Intent {
        self.update_with(points, layout, steer_right, dt, &StickRule::current())
    }

    /// `update` under an explicit rule.
    pub fn update_with(&mut self, points: &[TouchPoint], layout: &Layout, steer_right: bool, dt: f32, rule: &StickRule) -> Intent {
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

        // The stick follows its own touch id while that id is still down,
        // and its origin trails the thumb on a leash of the follow radius:
        // past it the origin is pulled to exactly that far behind, along
        // the drag, so the offset the direction is read from is always the
        // thumb's last stretch of motion rather than everything since it
        // landed.
        if let Some(stick) = self.stick.as_mut() {
            match points.iter().find(|p| p.id == stick.id) {
                Some(p) => {
                    stick.current = p.pos;
                    stick.radius = rule.radius();
                    let leash = rule.follow_radius_px;
                    if leash > 0.0 {
                        let dx = stick.current.x - stick.origin.x;
                        let dy = stick.current.y - stick.origin.y;
                        let len = (dx * dx + dy * dy).sqrt();
                        if len > leash {
                            let k = leash / len;
                            stick.origin = Vec2::new(stick.current.x - dx * k, stick.current.y - dy * k);
                        }
                    }
                }
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
                    self.stick = Some(Stick { id: p.id, origin: p.pos, current: p.pos, radius: rule.radius(), dir: None });
                }
            } else {
                self.fire_ids.push(p.id);
                self.ripples.push(Ripple { at: p.pos, age: 0.0 });
            }
        }

        let move_dir = self.stick.as_mut().and_then(|s| {
            s.dir = resolve_dir(s.current.x - s.origin.x, s.current.y - s.origin.y, s.dir, rule);
            s.dir
        });
        Intent { move_dir, fire: !self.fire_ids.is_empty(), ..Intent::default() }
    }

    /// Whether a touch is steering right now.
    pub fn steering(&self) -> bool {
        self.stick.is_some()
    }

    /// The held stick's origin, the thumb's position and the base radius
    /// - what `draw` paints, and what a test reads to check the origin
    /// trails the thumb.
    pub fn stick_geometry(&self) -> Option<(Vec2, Vec2, f32)> {
        self.stick.map(|s| (s.origin, s.current, s.radius))
    }

    /// The scheme's feedback, drawn in bitmap space over the field after
    /// everything else: the stick while held, fire ripples, the hint.
    #[cfg(feature = "render")]
    pub fn draw(&self, d: &mut impl RaylibDraw, layout: &Layout, steer_right: bool) {
        if let Some(s) = &self.stick {
            let dx = s.current.x - s.origin.x;
            let dy = s.current.y - s.origin.y;
            let len = (dx * dx + dy * dy).sqrt();
            let k = if len > s.radius { s.radius / len } else { 1.0 };
            let knob = Vec2::new(s.origin.x + dx * k, s.origin.y + dy * k);
            d.draw_circle_v(s.origin, s.radius, Color::new(255, 255, 255, 40));
            d.draw_circle_lines(s.origin.x as i32, s.origin.y as i32, s.radius, Color::new(255, 255, 255, 140));
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
            let t = crate::text::text();
            let steer = t.get(crate::text::keys::TOUCH_STEER);
            let fire = t.get(crate::text::keys::TOUCH_FIRE);
            d.draw_text(&steer, steer_x as i32 - crate::text::width(&steer, size) / 2, y, size, color);
            d.draw_text(&fire, fire_x as i32 - crate::text::width(&fire, size) / 2, y, size, color);
        }
    }
}

/// The direction a drag of (`dx`, `dy`) from the origin means under
/// `rule`, given the direction it meant last frame: none inside the dead
/// zone, else the dominant axis, and the previous axis is kept until the
/// drag is `axis_switch_deg` off it, which is where the other axis wins by
/// the tangent of that angle.
pub fn resolve_dir(dx: f32, dy: f32, previous: Option<Dir>, rule: &StickRule) -> Option<Dir> {
    let (ax, ay) = (dx.abs(), dy.abs());
    if ax < rule.dead_zone_px && ay < rule.dead_zone_px {
        return None;
    }
    let ratio = rule.switch_ratio();
    let horizontal = match previous {
        Some(Dir::Left) | Some(Dir::Right) => ay < ax * ratio,
        Some(Dir::Up) | Some(Dir::Down) => ax >= ay * ratio,
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

    const DT: f32 = 1.0 / 60.0;

    /// The rule the tuning table ships: a 14 px dead zone, a 40 px leash
    /// on the origin, the switch 50 degrees off the held axis.
    const RULE: StickRule = StickRule { dead_zone_px: 14.0, follow_radius_px: 40.0, axis_switch_deg: 50.0 };
    /// The pinned origin and the wide band a thumb reads as the tank
    /// refusing to turn - what the follow leash and the narrower band are
    /// measured against.
    const PINNED: StickRule = StickRule { dead_zone_px: 14.0, follow_radius_px: 0.0, axis_switch_deg: 60.0 };

    fn layout() -> Layout {
        Layout::for_field(960.0, 480.0)
    }

    fn pt(id: i32, x: f32, y: f32) -> TouchPoint {
        TouchPoint { id, pos: Vec2::new(x, y) }
    }

    #[test]
    fn the_tuning_table_ships_the_rule_these_tests_describe() {
        assert_eq!(StickRule::current(), RULE);
    }

    #[test]
    fn a_resting_thumb_inside_the_dead_zone_is_no_direction() {
        assert_eq!(resolve_dir(5.0, -8.0, None, &RULE), None);
        assert_eq!(resolve_dir(RULE.dead_zone_px, 0.0, None, &RULE), Some(Dir::Right));
    }

    #[test]
    fn the_dominant_axis_wins_and_the_held_axis_is_sticky_only_inside_the_band() {
        assert_eq!(resolve_dir(30.0, -20.0, None, &RULE), Some(Dir::Right));
        assert_eq!(resolve_dir(-20.0, 30.0, None, &RULE), Some(Dir::Down));
        // 42 degrees off horizontal is 48 off the held vertical axis, so a
        // stick already vertical stays vertical; 38 off horizontal is 52
        // off vertical, past the band, and the switch happens.
        assert_eq!(resolve_dir(30.0, 27.0, None, &RULE), Some(Dir::Right));
        assert_eq!(resolve_dir(30.0, 27.0, Some(Dir::Down), &RULE), Some(Dir::Down));
        assert_eq!(resolve_dir(30.0, 23.4, Some(Dir::Down), &RULE), Some(Dir::Right));
        // The wide band keeps the old axis a further ten degrees.
        assert_eq!(resolve_dir(30.0, 25.0, Some(Dir::Down), &PINNED), Some(Dir::Down));
        assert_eq!(resolve_dir(60.0, 25.0, Some(Dir::Down), &PINNED), Some(Dir::Right));
        // A sign flip along the held axis is immediate.
        assert_eq!(resolve_dir(-30.0, 5.0, Some(Dir::Right), &RULE), Some(Dir::Left));
    }

    /// Drive left, then slide the thumb slowly straight down: the tester's
    /// script. A thumb that has pushed 120 px left turns DOWN within 48 px
    /// of slide, because the origin trails it; pinned, the same slide has
    /// not turned it after 200 px, since the switch would need 208.
    fn slide_down_after_a_push_left(rule: &StickRule) -> (Option<f32>, TouchScheme) {
        let l = layout();
        let mut t = TouchScheme::default();
        t.update_with(&[pt(1, 300.0, 300.0)], &l, false, DT, rule);
        let mut x = 300.0;
        while x > 180.0 {
            x -= 20.0;
            let i = t.update_with(&[pt(1, x, 300.0)], &l, false, DT, rule);
            assert_eq!(i.move_dir, Some(Dir::Left));
        }
        let (mut y, mut slid) = (300.0, 0.0);
        let mut turned = None;
        while slid < 200.0 {
            y += 4.0;
            slid += 4.0;
            let i = t.update_with(&[pt(1, 180.0, y)], &l, false, DT, rule);
            if i.move_dir == Some(Dir::Down) {
                turned = Some(slid);
                break;
            }
            assert_eq!(i.move_dir, Some(Dir::Left), "nothing but LEFT before the turn at {slid} px");
        }
        (turned, t)
    }

    #[test]
    fn a_slow_slide_down_after_a_long_push_left_turns_within_the_leash() {
        let (turned, _) = slide_down_after_a_push_left(&RULE);
        assert!(turned.is_some_and(|d| d <= 48.0), "turned after {turned:?} px");
        let (turned, _) = slide_down_after_a_push_left(&PINNED);
        assert_eq!(turned, None, "a pinned origin needs 208 px of slide here");
    }

    #[test]
    fn the_origin_trails_the_thumb_by_at_most_the_leash() {
        let (_, t) = slide_down_after_a_push_left(&RULE);
        let (origin, current, radius) = t.stick_geometry().expect("still held");
        assert_eq!(radius, RULE.follow_radius_px);
        assert!(origin.distance_to(current) <= RULE.follow_radius_px + 1e-3, "{origin:?} -> {current:?}");
        let (_, t) = slide_down_after_a_push_left(&PINNED);
        let (origin, _, radius) = t.stick_geometry().expect("still held");
        assert_eq!(origin, Vec2::new(300.0, 300.0), "a pinned origin stays where the thumb landed");
        assert_eq!(radius, KNOB_TRAVEL_PX);
    }

    /// The drawn stick is the offset from the origin, so this is the bound
    /// on how far the picture can point from the direction the tank takes:
    /// never more than the band, whatever the thumb has done.
    #[test]
    fn the_drag_is_never_further_than_the_band_from_the_axis_it_reports() {
        let l = layout();
        let mut t = TouchScheme::default();
        t.update_with(&[pt(1, 300.0, 300.0)], &l, false, DT, &RULE);
        // A long push left, a diagonal, then a hook back up and right.
        let path: Vec<(f32, f32)> = (1..=12)
            .map(|i| (300.0 - 20.0 * i as f32, 300.0))
            .chain((1..=20).map(|i| (60.0 + 6.0 * i as f32, 300.0 + 6.0 * i as f32)))
            .chain((1..=30).map(|i| (180.0 + 8.0 * i as f32, 420.0 - 9.0 * i as f32)))
            .collect();
        for (x, y) in path {
            let i = t.update_with(&[pt(1, x, y)], &l, false, DT, &RULE);
            let (origin, current, _) = t.stick_geometry().expect("held");
            let (dx, dy) = ((current.x - origin.x).abs(), (current.y - origin.y).abs());
            let off_axis = match i.move_dir {
                Some(Dir::Left) | Some(Dir::Right) => dy.atan2(dx).to_degrees(),
                Some(Dir::Up) | Some(Dir::Down) => dx.atan2(dy).to_degrees(),
                None => 0.0,
            };
            assert!(off_axis <= RULE.axis_switch_deg + 0.01, "{off_axis} degrees off {:?} at ({x}, {y})", i.move_dir);
        }
    }

    #[test]
    fn a_drag_on_the_steer_half_moves_and_lifting_stops() {
        let l = layout();
        let mut t = TouchScheme::default();
        // Touch down on the right half, inside the field.
        let i = t.update(&[pt(1, 700.0, 300.0)], &l, true, DT);
        assert_eq!(i.move_dir, None);
        assert!(!i.fire);
        // Drag up past the dead zone.
        let i = t.update(&[pt(1, 700.0, 260.0)], &l, true, DT);
        assert_eq!(i.move_dir, Some(Dir::Up));
        // Lift: the tank stops.
        let i = t.update(&[], &l, true, DT);
        assert_eq!(i.move_dir, None);
        assert!(!t.steering());
    }

    #[test]
    fn a_touch_on_the_fire_half_fires_while_held_and_the_halves_mirror() {
        let l = layout();
        let mut t = TouchScheme::default();
        let i = t.update(&[pt(2, 200.0, 300.0)], &l, true, DT);
        assert!(i.fire);
        assert_eq!(i.move_dir, None);
        let i = t.update(&[], &l, true, DT);
        assert!(!i.fire);
        // Left-handed layout: the same point steers.
        let mut t = TouchScheme::default();
        t.update(&[pt(3, 200.0, 300.0)], &l, false, DT);
        assert!(t.steering());
    }

    #[test]
    fn two_thumbs_steer_and_fire_at_once_and_the_bar_is_ignored() {
        let l = layout();
        let mut t = TouchScheme::default();
        t.update(&[pt(1, 700.0, 300.0)], &l, true, DT);
        let i = t.update(&[pt(1, 660.0, 300.0), pt(2, 150.0, 200.0)], &l, true, DT);
        assert_eq!(i.move_dir, Some(Dir::Left));
        assert!(i.fire);
        // A touch on the bar (above the field) is neither.
        let mut t = TouchScheme::default();
        let i = t.update(&[pt(4, 700.0, 10.0)], &l, true, DT);
        assert_eq!(i.move_dir, None);
        assert!(!i.fire);
        assert!(!t.steering());
    }

    #[test]
    fn a_second_touch_on_the_steer_half_does_not_steal_the_stick() {
        let l = layout();
        let mut t = TouchScheme::default();
        t.update(&[pt(1, 700.0, 300.0)], &l, true, DT);
        let i = t.update(&[pt(1, 700.0, 340.0), pt(9, 800.0, 100.0)], &l, true, DT);
        assert_eq!(i.move_dir, Some(Dir::Down));
        assert!(!i.fire);
    }
}
