//! Touch controls for a keyboard-less device (docs/fullscreen-resolution-
//! research.md section 5, scheme J): nothing drawn at rest, a floating
//! joystick under one thumb and tap-to-fire under the other.
//!
//! The scheme lives on the window in UI points (`hud::UiFrame`,
//! docs/large-maps-follow-camera.md §8), like the HUD: the touches come in
//! as points, every length of the rule is in points, and the stick is drawn
//! at the UI scale - so it is the same size under the thumb whatever scale
//! the world is drawn at, an arena fitted into the window or a followed
//! field map. The area it takes touches from is the window in play (an
//! arena's margins included, where a tablet's thumbs rest), split down its
//! middle.
//!
//! The steering half is the left half - movement under the left thumb, a
//! gamepad's convention - unless the build was made with the
//! `touch-steer-right` cargo feature (`TOUCH_STEER_RIGHT`), which mirrors
//! it: a build-time choice on purpose, so no panel, file or dev tool can
//! flip the side under a player. The first touch that lands there becomes
//! the stick's origin, and the origin then trails the thumb: once the drag
//! is longer than `touch_follow_radius_pt` the origin is pulled along
//! behind it, so the stick only ever measures the last few dozen points of
//! motion and a change of direction costs the same short slide however far
//! the thumb has pushed. The drag from that origin picks one of the four
//! directions by its dominant axis, with a dead zone (`touch_dead_zone_pt`)
//! so a resting thumb does not creep and a hysteresis band (the held axis
//! keeps the direction until the drag is `touch_axis_switch_deg` off it) so
//! a drag near a diagonal does not flicker between axes; lifting stops the
//! tank, which is the game's own stop rule. The three numbers travel as a
//! `StickRule`, read from the tuning table once per frame. Any touch on the
//! other half fires while it is down - a tap is a one-frame press, which is
//! the edge a shell needs, and a hold is what the laser, minigun and
//! flamethrower read. A touch outside the area is never scheme input (the
//! builder takes every touch itself), nor is one on a corner cluster of the
//! HUD (`keep_out`): a touch that lands on one is the HUD's until it lifts,
//! and the clusters' buttons are handled by `app.rs` before the scheme
//! sees a frame.
//!
//! The scheme turns raw touch points into the same `Intent` the keyboard
//! produces, so the simulation never learns a touch screen exists. Only a
//! real touch point drives it; the mouse keeps its own role (the HUD's
//! buttons, the dialogs, the builder) unless `--touch-from-mouse` asks for
//! a desktop stand-in, which is a development aid rather than a control
//! scheme. Presentation-only feedback (`draw`, the one item here that
//! needs raylib, painting `stick_drawing`): a faint base and knob while the
//! stick is held - the base drawn at the origin the rule measures from, so
//! the picture can never point somewhere the tank is not going -, a ripple
//! where a fire tap landed, and a one-time hint the first time a touch is
//! seen.

use crate::math::Vec2;
#[cfg(feature = "render")]
use crate::math::Color;
#[cfg(feature = "render")]
use sola_raylib::prelude::RaylibDraw;

use crate::ai::Intent;
use crate::math::Rectangle;
use crate::tank::Dir;
use crate::tuning::tuning;
use crate::Rect;

/// How far the knob may sit from the base when the origin is pinned
/// (`touch_follow_radius_pt` at 0), in UI points; with a trailing origin
/// the leash is the base's radius, and the knob is at most that far by
/// construction.
pub const KNOB_TRAVEL_PT: f32 = 40.0;
/// The knob's radius, in UI points: 44 across, the least a thumb is given
/// to see under it (`hud::UI_TOUCH_PT`).
pub const KNOB_RADIUS_PT: f32 = 22.0;
/// The least radius the base is drawn at, in UI points: 44 across however
/// short the leash is tuned.
pub const BASE_MIN_RADIUS_PT: f32 = 22.0;
/// A fire ripple's radius when the tap lands and how far it grows over its
/// life, in UI points.
pub const RIPPLE_RADIUS_PT: f32 = 14.0;
pub const RIPPLE_GROWTH_PT: f32 = 12.0;
/// How long a fire ripple stays on screen.
pub const RIPPLE_SECONDS: f32 = 0.25;
/// The hint's text size, in UI points: over the 11 pt Apple holds a
/// phone's text to.
pub const HINT_TEXT_PT: i32 = 20;
/// How long the first-touch hint stays up.
pub const HINT_SECONDS: f32 = 4.0;

/// The three numbers a drag is resolved by: the tuning rows, read once per
/// frame by `StickRule::current` and passed in explicitly, so a test
/// describes the rule it exercises and never touches the global table.
/// Lengths in UI points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StickRule {
    /// A drag shorter than this on both axes is a resting thumb.
    pub dead_zone_pt: f32,
    /// The origin trails the thumb so the drag never exceeds this; 0 pins
    /// it where the thumb landed.
    pub follow_radius_pt: f32,
    /// Degrees off the held axis a drag has to reach before the other axis
    /// takes over.
    pub axis_switch_deg: f32,
}

impl StickRule {
    /// The rule the tuning table holds this frame.
    pub fn current() -> Self {
        let t = tuning();
        StickRule {
            dead_zone_pt: t.touch_dead_zone_pt,
            follow_radius_pt: t.touch_follow_radius_pt,
            axis_switch_deg: t.touch_axis_switch_deg,
        }
    }

    /// The factor the other axis has to exceed the held one by before the
    /// direction switches: the tangent of the switch angle.
    fn switch_ratio(&self) -> f32 {
        self.axis_switch_deg.to_radians().tan()
    }

    /// The leash on a trailing origin - how far the knob may sit from the
    /// base -, in UI points.
    fn radius(&self) -> f32 {
        if self.follow_radius_pt > 0.0 { self.follow_radius_pt } else { KNOB_TRAVEL_PT }
    }
}

/// One touch point this frame, in the space its reader takes it in: UI
/// points for the touch scheme (`hud::UiFrame::to_ui`), bitmap pixels for
/// the builder's gestures (`View::to_bitmap`).
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
    /// The leash this frame (`StickRule::radius`), kept on the stick so
    /// the drawing uses the number the rule did.
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

/// The held stick as `draw` paints it, in UI points: the base round the
/// origin the rule measures from, and the knob where the thumb is, held to
/// the leash.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StickDrawing {
    pub base: Vec2,
    /// The leash, never under `BASE_MIN_RADIUS_PT`.
    pub base_radius: f32,
    pub knob: Vec2,
    /// `KNOB_RADIUS_PT`.
    pub knob_radius: f32,
}

/// The scheme's state across frames.
#[derive(Debug, Default)]
pub struct TouchScheme {
    stick: Option<Stick>,
    /// Touch ids currently held on the fire half.
    fire_ids: Vec<i32>,
    /// Touch ids that pressed a button (`claim`) or landed on the HUD:
    /// neither steering nor firing until they lift.
    claimed: Vec<i32>,
    ripples: Vec<Ripple>,
    /// Seconds of hint left; `None` until the first touch is ever seen.
    hint: Option<f32>,
    /// Whether any touch has been seen this session - the hint fires once.
    seen: bool,
    /// Where a touch landing is nobody's steering or firing, in UI points:
    /// the HUD's corner clusters (`set_keep_out`).
    keep_out: Vec<Rectangle>,
    /// The area the last `update` took touches from, in UI points: where
    /// the hint is set.
    area: Option<Rect>,
}

impl TouchScheme {
    /// The rectangles, in UI points, where a touch that lands neither
    /// steers nor fires until it lifts: the HUD's clusters
    /// (`hud::Corners::keep_out`), which `app.rs` hands over every frame. A
    /// stick dragged across one keeps steering - only where a touch lands
    /// decides whose it is.
    pub fn set_keep_out(&mut self, rects: &[Rectangle]) {
        self.keep_out.clear();
        self.keep_out.extend_from_slice(rects);
    }

    /// Feed this frame's touch points, in UI points, and get the intent
    /// they mean, under the rule the tuning table holds. `area` is where
    /// the scheme takes touches from, in UI points - the window in play,
    /// nothing in the builder - split down its middle; `steer_right` puts
    /// the stick on the right half (the fire half is the other one).
    /// Points outside it and on the HUD's clusters (`set_keep_out`) are
    /// ignored.
    pub fn update(&mut self, points: &[TouchPoint], area: Rect, steer_right: bool, dt: f32) -> Intent {
        self.update_with(points, area, steer_right, dt, &StickRule::current())
    }

    /// `update` under an explicit rule.
    pub fn update_with(&mut self, points: &[TouchPoint], area: Rect, steer_right: bool, dt: f32, rule: &StickRule) -> Intent {
        self.area = Some(area);
        let split = area.x + area.w / 2.0;
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
                    let leash = rule.follow_radius_pt;
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
        self.claimed.retain(|id| points.iter().any(|p| p.id == *id));
        for p in points {
            let claimed = self.stick.map(|s| s.id == p.id).unwrap_or(false)
                || self.fire_ids.contains(&p.id)
                || self.claimed.contains(&p.id);
            if claimed || !area.contains(p.pos) {
                continue;
            }
            if self.keep_out.iter().any(|r| r.contains(p.pos)) {
                // Landed on the HUD: nobody's shot or stick, held or not.
                self.claimed.push(p.id);
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

    /// The touches down now pressed a button over the field - an end
    /// screen's, a dialog's: none of them steers or fires, this frame or
    /// for as long as it stays down, so the tap on `NEXT LEVEL` is not
    /// also the fire press that skips the next level's banner.
    pub fn claim(&mut self, points: &[TouchPoint]) {
        for p in points {
            if !self.claimed.contains(&p.id) {
                self.claimed.push(p.id);
            }
        }
        let claimed = &self.claimed;
        self.fire_ids.retain(|id| !claimed.contains(id));
        if self.stick.is_some_and(|s| claimed.contains(&s.id)) {
            self.stick = None;
        }
    }

    /// Whether a touch is steering right now.
    pub fn steering(&self) -> bool {
        self.stick.is_some()
    }

    /// Whether a touch has been seen this session: thumbs are on the
    /// screen, whatever the build - a phone's browser has a keyboard build.
    pub fn seen(&self) -> bool {
        self.seen
    }

    /// Whether this frame's chrome is laid out for thumbs (`UiFrame::touch`):
    /// on a build with no `keyboard`, under `--touch-from-mouse`
    /// (`from_mouse`), once a touch has been seen - and on the frame a
    /// finger is first `touching`, before `update` has seen it, so the
    /// frame a finger lands on is the frame it lifts from and nothing laid
    /// out by the chrome moves under it.
    pub fn touch_chrome(&self, keyboard: bool, from_mouse: bool, touching: bool) -> bool {
        !keyboard || from_mouse || touching || self.seen
    }

    /// The held stick's origin, the thumb's position and the leash, in UI
    /// points - what a test reads to check the origin trails the thumb.
    pub fn stick_geometry(&self) -> Option<(Vec2, Vec2, f32)> {
        self.stick.map(|s| (s.origin, s.current, s.radius))
    }

    /// The held stick as it is drawn (`StickDrawing`), in UI points: the
    /// base at the origin, the knob along the drag at most a leash from it.
    pub fn stick_drawing(&self) -> Option<StickDrawing> {
        self.stick.map(|s| {
            let dx = s.current.x - s.origin.x;
            let dy = s.current.y - s.origin.y;
            let len = (dx * dx + dy * dy).sqrt();
            let k = if len > s.radius { s.radius / len } else { 1.0 };
            StickDrawing {
                base: s.origin,
                base_radius: s.radius.max(BASE_MIN_RADIUS_PT),
                knob: Vec2::new(s.origin.x + dx * k, s.origin.y + dy * k),
                knob_radius: KNOB_RADIUS_PT,
            }
        })
    }

    /// The scheme's feedback over everything else, in UI points - call
    /// inside a camera at the UI scale (`hud::UiFrame::scale`), never the
    /// world's: the stick while held (`stick_drawing`), fire ripples, the
    /// hint across the area's two halves - while a tap is what the hints
    /// name (`hud::Hints`): a key pressed meanwhile takes it away.
    #[cfg(feature = "render")]
    pub fn draw(&self, d: &mut impl RaylibDraw, steer_right: bool, hints: crate::hud::Hints) {
        if let Some(s) = self.stick_drawing() {
            d.draw_circle_v(s.base, s.base_radius, Color::new(255, 255, 255, 40));
            d.draw_circle_lines_v(s.base, s.base_radius, Color::new(255, 255, 255, 140));
            d.draw_line_ex(s.base, s.knob, 2.0, Color::new(255, 255, 255, 170));
            d.draw_circle_v(s.knob, s.knob_radius, Color::new(255, 255, 255, 110));
        }
        for r in &self.ripples {
            let t = r.age / RIPPLE_SECONDS;
            let radius = RIPPLE_RADIUS_PT + RIPPLE_GROWTH_PT * t;
            let alpha = (200.0 * (1.0 - t)) as u8;
            d.draw_circle_lines_v(r.at, radius, Color::new(255, 170, 120, alpha));
        }
        if let (Some(t), Some(area), crate::hud::Hints::Touch) = (self.hint, self.area, hints) {
            let alpha = ((t / HINT_SECONDS).min(1.0) * 220.0) as u8;
            let y = (area.y + area.h * 0.5) as i32;
            let (steer_x, fire_x) = if steer_right {
                (area.x + area.w * 0.75, area.x + area.w * 0.25)
            } else {
                (area.x + area.w * 0.25, area.x + area.w * 0.75)
            };
            let color = Color::new(255, 255, 255, alpha);
            let t = crate::text::text();
            let steer = t.get(crate::text::keys::TOUCH_STEER);
            let fire = t.get(crate::text::keys::TOUCH_FIRE);
            d.draw_text(&steer, steer_x as i32 - crate::text::width(&steer, HINT_TEXT_PT) / 2, y, HINT_TEXT_PT, color);
            d.draw_text(&fire, fire_x as i32 - crate::text::width(&fire, HINT_TEXT_PT) / 2, y, HINT_TEXT_PT, color);
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
    if ax < rule.dead_zone_pt && ay < rule.dead_zone_pt {
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
    use crate::hud::{Insets, UiFrame, UI_SMALL_TEXT, UI_TOUCH_PT};

    const DT: f32 = 1.0 / 60.0;

    /// The rule the tuning table ships: a 14 pt dead zone, a 40 pt leash
    /// on the origin, the switch 50 degrees off the held axis.
    const RULE: StickRule = StickRule { dead_zone_pt: 14.0, follow_radius_pt: 40.0, axis_switch_deg: 50.0 };
    /// The pinned origin and the wide band a thumb reads as the tank
    /// refusing to turn - what the follow leash and the narrower band are
    /// measured against.
    const PINNED: StickRule = StickRule { dead_zone_pt: 14.0, follow_radius_pt: 0.0, axis_switch_deg: 60.0 };

    /// Where these tests take touches from, in UI points: 960 wide, below
    /// a 32 pt strip that is not the scheme's.
    fn area() -> Rect {
        Rect::new(0.0, 32.0, 960.0, 480.0)
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
        assert_eq!(resolve_dir(RULE.dead_zone_pt, 0.0, None, &RULE), Some(Dir::Right));
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
    /// script. A thumb that has pushed 120 pt left turns DOWN within 48 pt
    /// of slide, because the origin trails it; pinned, the same slide has
    /// not turned it after 200 pt, since the switch would need 208.
    fn slide_down_after_a_push_left(rule: &StickRule) -> (Option<f32>, TouchScheme) {
        let a = area();
        let mut t = TouchScheme::default();
        t.update_with(&[pt(1, 300.0, 300.0)], a, false, DT, rule);
        let mut x = 300.0;
        while x > 180.0 {
            x -= 20.0;
            let i = t.update_with(&[pt(1, x, 300.0)], a, false, DT, rule);
            assert_eq!(i.move_dir, Some(Dir::Left));
        }
        let (mut y, mut slid) = (300.0, 0.0);
        let mut turned = None;
        while slid < 200.0 {
            y += 4.0;
            slid += 4.0;
            let i = t.update_with(&[pt(1, 180.0, y)], a, false, DT, rule);
            if i.move_dir == Some(Dir::Down) {
                turned = Some(slid);
                break;
            }
            assert_eq!(i.move_dir, Some(Dir::Left), "nothing but LEFT before the turn at {slid} pt");
        }
        (turned, t)
    }

    #[test]
    fn a_slow_slide_down_after_a_long_push_left_turns_within_the_leash() {
        let (turned, _) = slide_down_after_a_push_left(&RULE);
        assert!(turned.is_some_and(|d| d <= 48.0), "turned after {turned:?} pt");
        let (turned, _) = slide_down_after_a_push_left(&PINNED);
        assert_eq!(turned, None, "a pinned origin needs 208 pt of slide here");
    }

    #[test]
    fn the_origin_trails_the_thumb_by_at_most_the_leash() {
        let (_, t) = slide_down_after_a_push_left(&RULE);
        let (origin, current, radius) = t.stick_geometry().expect("still held");
        assert_eq!(radius, RULE.follow_radius_pt);
        assert!(origin.distance_to(current) <= RULE.follow_radius_pt + 1e-3, "{origin:?} -> {current:?}");
        let (_, t) = slide_down_after_a_push_left(&PINNED);
        let (origin, _, radius) = t.stick_geometry().expect("still held");
        assert_eq!(origin, Vec2::new(300.0, 300.0), "a pinned origin stays where the thumb landed");
        assert_eq!(radius, KNOB_TRAVEL_PT);
    }

    /// The drawn stick is the offset from the origin, so this is the bound
    /// on how far the picture can point from the direction the tank takes:
    /// never more than the band, whatever the thumb has done.
    #[test]
    fn the_drag_is_never_further_than_the_band_from_the_axis_it_reports() {
        let a = area();
        let mut t = TouchScheme::default();
        t.update_with(&[pt(1, 300.0, 300.0)], a, false, DT, &RULE);
        // A long push left, a diagonal, then a hook back up and right.
        let path: Vec<(f32, f32)> = (1..=12)
            .map(|i| (300.0 - 20.0 * i as f32, 300.0))
            .chain((1..=20).map(|i| (60.0 + 6.0 * i as f32, 300.0 + 6.0 * i as f32)))
            .chain((1..=30).map(|i| (180.0 + 8.0 * i as f32, 420.0 - 9.0 * i as f32)))
            .collect();
        for (x, y) in path {
            let i = t.update_with(&[pt(1, x, y)], a, false, DT, &RULE);
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
        let a = area();
        let mut t = TouchScheme::default();
        // Touch down on the right half, inside the area.
        let i = t.update(&[pt(1, 700.0, 300.0)], a, true, DT);
        assert_eq!(i.move_dir, None);
        assert!(!i.fire);
        // Drag up past the dead zone.
        let i = t.update(&[pt(1, 700.0, 260.0)], a, true, DT);
        assert_eq!(i.move_dir, Some(Dir::Up));
        // Lift: the tank stops.
        let i = t.update(&[], a, true, DT);
        assert_eq!(i.move_dir, None);
        assert!(!t.steering());
    }

    #[test]
    fn a_touch_on_the_fire_half_fires_while_held_and_the_halves_mirror() {
        let a = area();
        let mut t = TouchScheme::default();
        let i = t.update(&[pt(2, 200.0, 300.0)], a, true, DT);
        assert!(i.fire);
        assert_eq!(i.move_dir, None);
        let i = t.update(&[], a, true, DT);
        assert!(!i.fire);
        // Left-handed layout: the same point steers.
        let mut t = TouchScheme::default();
        t.update(&[pt(3, 200.0, 300.0)], a, false, DT);
        assert!(t.steering());
    }

    #[test]
    fn two_thumbs_steer_and_fire_at_once_and_a_touch_outside_the_area_is_ignored() {
        let a = area();
        let mut t = TouchScheme::default();
        t.update(&[pt(1, 700.0, 300.0)], a, true, DT);
        let i = t.update(&[pt(1, 660.0, 300.0), pt(2, 150.0, 200.0)], a, true, DT);
        assert_eq!(i.move_dir, Some(Dir::Left));
        assert!(i.fire);
        // A touch above the area is neither.
        let mut t = TouchScheme::default();
        let i = t.update(&[pt(4, 700.0, 10.0)], a, true, DT);
        assert_eq!(i.move_dir, None);
        assert!(!i.fire);
        assert!(!t.steering());
        // Nor is anything under an empty area: the builder's every touch is
        // its own.
        let i = t.update(&[pt(5, 700.0, 300.0), pt(6, 100.0, 300.0)], Rect::new(0.0, 0.0, 0.0, 0.0), true, DT);
        assert!(!i.fire && i.move_dir.is_none() && !t.steering());
    }

    #[test]
    fn a_second_touch_on_the_steer_half_does_not_steal_the_stick() {
        let a = area();
        let mut t = TouchScheme::default();
        t.update(&[pt(1, 700.0, 300.0)], a, true, DT);
        let i = t.update(&[pt(1, 700.0, 340.0), pt(9, 800.0, 100.0)], a, true, DT);
        assert_eq!(i.move_dir, Some(Dir::Down));
        assert!(!i.fire);
    }

    /// A touch that lands on a corner cluster of the HUD neither steers
    /// nor fires for as long as it is down, on either half; a stick that
    /// started on the field and is dragged across a cluster keeps
    /// steering, since only where a touch lands decides whose it is.
    #[test]
    fn a_touch_on_a_hud_cluster_neither_steers_nor_fires() {
        let a = area();
        let left = Rectangle::new(0.0, 32.0, 300.0, 80.0);
        let right = Rectangle::new(660.0, 32.0, 300.0, 100.0);
        let mut t = TouchScheme::default();
        t.set_keep_out(&[left, right]);
        let i = t.update(&[pt(1, 100.0, 60.0), pt(2, 800.0, 60.0)], a, false, DT);
        assert!(!i.fire && !t.steering(), "a tap on either cluster is the HUD's");
        let i = t.update(&[pt(1, 100.0, 300.0), pt(2, 800.0, 300.0)], a, false, DT);
        assert!(!i.fire && i.move_dir.is_none(), "still the HUD's when dragged off it");
        // Lifted, the field is the scheme's again; a stick that started
        // there keeps steering onto a cluster.
        t.update(&[], a, false, DT);
        t.update(&[pt(3, 200.0, 300.0)], a, false, DT);
        let i = t.update(&[pt(3, 200.0, 60.0)], a, false, DT);
        assert_eq!(i.move_dir, Some(Dir::Up));
        assert!(t.steering());
        assert!(t.update(&[pt(3, 200.0, 60.0), pt(4, 700.0, 300.0)], a, false, DT).fire, "a tap off the clusters fires");
    }

    /// A touch that pressed a button neither fires nor steers while it is
    /// down, whichever half it landed on; once it lifts, the next touch is
    /// the scheme's again.
    #[test]
    fn a_claimed_touch_neither_fires_nor_steers_until_it_lifts() {
        let a = area();
        let mut t = TouchScheme::default();
        let (fire, steer) = (pt(5, 200.0, 300.0), pt(6, 700.0, 300.0));
        t.claim(&[fire, steer]);
        let i = t.update(&[fire, steer], a, true, DT);
        assert!(!i.fire && !t.steering());
        let i = t.update(&[fire, pt(6, 700.0, 240.0)], a, true, DT);
        assert!(!i.fire && i.move_dir.is_none(), "held and dragged, still nobody's");
        // Claimed after the scheme already took it: let go at once.
        let mut t = TouchScheme::default();
        assert!(t.update(&[pt(7, 200.0, 300.0)], a, true, DT).fire);
        t.claim(&[pt(7, 200.0, 300.0)]);
        assert!(!t.update(&[pt(7, 200.0, 300.0)], a, true, DT).fire);
        // Lifted, and a new touch fires as usual.
        t.update(&[], a, true, DT);
        assert!(t.update(&[pt(8, 200.0, 300.0)], a, true, DT).fire);
    }

    /// A phone: a Pixel in landscape, whose window is in device pixels
    /// 2.625 to the point (`UiFrame`'s units per point), and the 59 pt
    /// strips an iPhone's island and corners take on both sides.
    fn phone() -> UiFrame {
        UiFrame::new((2400.0, 1080.0), 2.625, 1.0, Insets { left: 59.0 * 2.625, top: 0.0, right: 59.0 * 2.625, bottom: 21.0 * 2.625 }, true)
    }

    /// A desktop: a 1080p monitor, its window in points.
    fn desktop() -> UiFrame {
        UiFrame::new((1920.0, 1080.0), 1.0, 1.0, Insets::default(), true)
    }

    /// One thumb landing on the steering half and pushing 90 points left
    /// and 10 down on `ui`'s window, the touches arriving in window units
    /// as the platform reports them and taken to points as `app.rs` does.
    fn push_left(ui: &UiFrame) -> (TouchScheme, Intent) {
        let mut t = TouchScheme::default();
        let land = Vec2::new(ui.screen.w * 0.2, ui.screen.h * 0.7);
        let touch = |dx: f32, dy: f32| {
            let window = ui.to_window(Vec2::new(land.x + dx, land.y + dy));
            TouchPoint { id: 1, pos: ui.to_ui(window) }
        };
        t.update_with(&[touch(0.0, 0.0)], ui.screen, false, DT, &RULE);
        let mut intent = Intent::default();
        for i in 1..=9 {
            intent = t.update_with(&[touch(-10.0 * i as f32, i as f32 * 10.0 / 9.0)], ui.screen, false, DT, &RULE);
        }
        (t, intent)
    }

    /// The stick is laid out and drawn in UI points, never at the world's
    /// scale: the same thumb's push on a phone and on a monitor is the
    /// same direction under the same leash, and the stick is the same
    /// size in points - which is the same size on either glass, the
    /// phone's window units being 2.625 to the point - with its base and
    /// knob at least a finger's 44 points across and the hint over the
    /// smallest text a phone is given.
    #[test]
    fn the_stick_is_the_same_size_in_points_on_a_phone_and_a_desktop() {
        let (phone, desk) = (phone(), desktop());
        assert!((phone.scale / desk.scale - 2.625).abs() < 1e-4, "{} vs {}", phone.scale, desk.scale);
        let ((on_phone, i_phone), (on_desk, i_desk)) = (push_left(&phone), push_left(&desk));
        assert_eq!(i_phone.move_dir, Some(Dir::Left));
        assert_eq!(i_desk.move_dir, Some(Dir::Left));
        let (a, b) = (on_phone.stick_drawing().expect("held"), on_desk.stick_drawing().expect("held"));
        assert_eq!((a.base_radius, a.knob_radius), (b.base_radius, b.knob_radius));
        let (throw_a, throw_b) = (a.knob - a.base, b.knob - b.base);
        assert!((throw_a.x - throw_b.x).abs() < 1e-3 && (throw_a.y - throw_b.y).abs() < 1e-3, "{throw_a:?} vs {throw_b:?}");
        assert!(throw_a.length() <= RULE.follow_radius_pt + 1e-3, "the knob is held to the leash: {throw_a:?}");
        for s in [a, b] {
            assert!(2.0 * s.base_radius >= UI_TOUCH_PT && 2.0 * s.knob_radius >= UI_TOUCH_PT, "{s:?}");
        }
        // In window units the phone's stick is 2.625 times the desktop's:
        // the same points, the same size under the thumb.
        assert!((a.knob_radius * phone.scale - 2.625 * b.knob_radius * desk.scale).abs() < 1e-3);
        assert!(HINT_TEXT_PT >= 11 && HINT_TEXT_PT >= UI_SMALL_TEXT);
        // A leash tuned short still draws the base a finger wide.
        let short = StickRule { follow_radius_pt: 8.0, ..RULE };
        let mut t = TouchScheme::default();
        t.update_with(&[pt(1, 100.0, 300.0)], area(), false, DT, &short);
        t.update_with(&[pt(1, 60.0, 300.0)], area(), false, DT, &short);
        let s = t.stick_drawing().expect("held");
        assert_eq!(s.base_radius, BASE_MIN_RADIUS_PT);
        assert!(s.knob.distance_to(s.base) <= 8.0 + 1e-3);
    }

    /// The corners a phone's window lays out are the scheme's keep-outs as
    /// they stand, in the same points: a thumb on the right cluster's
    /// buttons neither fires nor steers, and one just under it fires.
    #[test]
    fn a_phones_corner_clusters_keep_the_thumbs_off_in_points() {
        use crate::hud::{corners, CornerShape, HudLayout};
        let ui = phone();
        let shape = CornerShape {
            layout: HudLayout::One,
            chips: 0,
            level_button: true,
            online: true,
            players: false,
            restart: true,
            build: true,
            leave: false,
            pause: true,
            lines: 1,
            // A phone draws no minimap (`minimap::MinimapRules::is_phone`).
            minimap: None,
            lamp_row: false,
        };
        let c = corners(&ui, &shape);
        let mut t = TouchScheme::default();
        t.set_keep_out(&c.keep_out());
        let right = c.right;
        let on = Vec2::new(right.x + right.width / 2.0, right.y + right.height / 2.0);
        let i = t.update(&[TouchPoint { id: 1, pos: on }], ui.screen, false, DT);
        assert!(!i.fire && !t.steering(), "{on:?} is the HUD's");
        let under = Vec2::new(on.x, right.y + right.height + 20.0);
        assert!(t.update(&[TouchPoint { id: 1, pos: on }, TouchPoint { id: 2, pos: under }], ui.screen, false, DT).fire);
        // The left cluster sits on the steering half: a thumb on it does
        // not steer, one under it does.
        let left = c.left();
        let on_left = Vec2::new(left.x + left.width / 2.0, left.y + left.height / 2.0);
        t.update(&[], ui.screen, false, DT);
        t.update(&[TouchPoint { id: 3, pos: on_left }], ui.screen, false, DT);
        assert!(!t.steering());
        t.update(&[], ui.screen, false, DT);
        t.update(&[TouchPoint { id: 4, pos: Vec2::new(on_left.x, left.y + left.height + 40.0) }], ui.screen, false, DT);
        assert!(t.steering());
    }
}
