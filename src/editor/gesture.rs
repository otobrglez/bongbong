//! The builder's touch gestures, read from the raw touch points
//! (docs/large-maps-follow-camera.md §9). raylib's gesture module reads
//! only two points and decides with 0.3 s thresholds, so the builder
//! follows the fingers itself: touch points in, `GestureEvent`s out, no
//! camera and no map - `MapEditor` turns the events into strokes and
//! camera moves. Headless and pure, so every gesture is a test.
//!
//! - **One finger paints** once it has moved past the touch slop: a
//!   finger resting on the glass paints nothing, and a stroke starts from
//!   where the finger landed. A quick touch that never left the slop is a
//!   tap - one cell. Where the editor says a drag does not paint (under
//!   the paint threshold, with somewhere to pan to), it pans instead, and
//!   the editor decides what a tap does.
//! - **Two fingers pan and pinch** about their middle; nothing moves until
//!   they have moved past the slop between them, and then everything since
//!   they landed applies at once. A second finger landing on a stroke takes
//!   the stroke back. When fewer than two remain the view settles
//!   (`Settle`), and a finger left behind does nothing until every finger
//!   has lifted.
//! - **A two-finger tap undoes and a three-finger tap redoes** (Procreate,
//!   Pixaki): the fingers down at once, lifted within `builder_tap_seconds`
//!   of the first landing, none past the slop.
//! - A finger that landed anywhere but the canvas - the bar, an open popup
//!   - is never part of a gesture (`update`'s `adopt`).

use crate::math::Vec2;
use crate::touch::TouchPoint;

/// The two numbers a gesture is told apart by, in the units of the touch
/// points (bitmap pixels) and seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GestureRules {
    /// How far a finger, or the middle and the spread of two, moves
    /// before a touch is a drag rather than a tap.
    pub slop: f32,
    /// How long a tap may last, from the first finger landing.
    pub tap_seconds: f32,
}

/// What a frame of touches means to the canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GestureEvent {
    /// One finger tapped where it landed: paint that cell, or zoom in on
    /// it under the paint threshold.
    Tap(Vec2),
    /// A one-finger stroke began where the finger landed ...
    StrokeBegin(Vec2),
    /// ... and has reached this point ...
    StrokeTo(Vec2),
    /// ... and the finger lifted.
    StrokeEnd,
    /// A second finger landed on a stroke: take it back.
    StrokeCancel,
    /// Move the view: the world under `from` comes to lie under `to`,
    /// magnified `factor` times about it - a one-finger pan is factor 1.
    Move { from: Vec2, to: Vec2, factor: f32 },
    /// Two fingers that moved the view have parted company at `at`: a
    /// coarse screen's zoom comes to rest on whole blocks there.
    Settle(Vec2),
    Undo,
    Redo,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum State {
    #[default]
    Idle,
    /// One finger down, still inside the slop: a tap, the start of a drag
    /// or the first finger of a two-finger gesture.
    Pending { id: i32, start: Vec2, age: f32 },
    /// One finger painting a stroke.
    Painting { id: i32 },
    /// One finger panning, under the paint threshold.
    Panning { id: i32, last: Vec2 },
    /// Two fingers or more: the pair `ids` pans and pinches from where it
    /// was at `mid`/`spread` - its landing until it moved, then the last
    /// frame - and `most` is the most fingers down at once, for the taps.
    Multi { ids: [i32; 2], start_mid: Vec2, start_spread: f32, mid: Vec2, spread: f32, age: f32, moved: bool, most: usize },
    /// A gesture ended with a finger still down: nothing until all lift.
    Draining,
}

/// The gesture state across frames.
#[derive(Clone, Debug, Default)]
pub struct Gestures {
    state: State,
    /// Every finger down that landed on the canvas, in landing order.
    adopted: Vec<i32>,
    /// Fingers down that landed anywhere else.
    ignored: Vec<i32>,
}

impl Gestures {
    /// No gesture, and the fingers `down` ignored until they lift: they
    /// landed on something else - the canvas came up under them.
    pub fn ignoring(down: impl IntoIterator<Item = i32>) -> Gestures {
        Gestures { ignored: down.into_iter().collect(), ..Gestures::default() }
    }

    /// Whether a gesture is under way or a canvas finger is down - the
    /// canvas belongs to the fingers until it is over.
    pub fn active(&self) -> bool {
        self.state != State::Idle || !self.adopted.is_empty()
    }

    /// Whether one finger is painting a stroke.
    pub fn painting(&self) -> bool {
        matches!(self.state, State::Painting { .. })
    }

    /// Read one frame of touch points - every finger down this frame, in
    /// bitmap pixels - `dt` seconds after the last. `adopt` says whether a
    /// finger seen for the first time landed on the canvas; `paints`
    /// whether one finger dragging paints (or pans, under the paint
    /// threshold).
    pub fn update(&mut self, touches: &[TouchPoint], adopt: impl Fn(&TouchPoint) -> bool, paints: bool, dt: f32, rules: &GestureRules) -> Vec<GestureEvent> {
        self.adopted.retain(|id| touches.iter().any(|t| t.id == *id));
        self.ignored.retain(|id| touches.iter().any(|t| t.id == *id));
        for t in touches {
            if !self.adopted.contains(&t.id) && !self.ignored.contains(&t.id) {
                if adopt(t) {
                    self.adopted.push(t.id);
                } else {
                    self.ignored.push(t.id);
                }
            }
        }
        let down: Vec<TouchPoint> = self.adopted.iter().filter_map(|id| touches.iter().find(|t| t.id == *id).copied()).collect();
        let at = |id: i32| down.iter().find(|t| t.id == id).map(|t| t.pos);
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        let mut events = Vec::new();
        self.state = match self.state {
            State::Idle => match down.len() {
                0 => State::Idle,
                1 => State::Pending { id: down[0].id, start: down[0].pos, age: 0.0 },
                _ => multi(&down, 0.0, false),
            },
            State::Pending { id, start, age } => {
                let age = age + dt;
                if down.len() >= 2 {
                    multi(&down, age, false)
                } else {
                    match at(id) {
                        None => {
                            if age <= rules.tap_seconds {
                                events.push(GestureEvent::Tap(start));
                            }
                            // Another finger may have landed as this one lifted.
                            match down.first() {
                                Some(t) => State::Pending { id: t.id, start: t.pos, age: 0.0 },
                                None => State::Idle,
                            }
                        }
                        Some(pos) if pos.distance_to(start) > rules.slop => {
                            if paints {
                                events.push(GestureEvent::StrokeBegin(start));
                                events.push(GestureEvent::StrokeTo(pos));
                                State::Painting { id }
                            } else {
                                events.push(GestureEvent::Move { from: start, to: pos, factor: 1.0 });
                                State::Panning { id, last: pos }
                            }
                        }
                        Some(_) => State::Pending { id, start, age },
                    }
                }
            }
            State::Painting { id } => {
                if down.len() >= 2 {
                    events.push(GestureEvent::StrokeCancel);
                    multi(&down, rules.tap_seconds, true)
                } else {
                    match at(id) {
                        None => {
                            events.push(GestureEvent::StrokeEnd);
                            if down.is_empty() { State::Idle } else { State::Draining }
                        }
                        Some(pos) => {
                            events.push(GestureEvent::StrokeTo(pos));
                            State::Painting { id }
                        }
                    }
                }
            }
            State::Panning { id, last } => {
                if down.len() >= 2 {
                    multi(&down, rules.tap_seconds, true)
                } else {
                    match at(id) {
                        None if down.is_empty() => State::Idle,
                        None => State::Draining,
                        Some(pos) => {
                            if pos != last {
                                events.push(GestureEvent::Move { from: last, to: pos, factor: 1.0 });
                            }
                            State::Panning { id, last: pos }
                        }
                    }
                }
            }
            State::Multi { ids, start_mid, start_spread, mid, spread, age, moved, most } => {
                let age = age + dt;
                let most = most.max(down.len());
                if down.len() < 2 && moved {
                    events.push(GestureEvent::Settle(mid));
                    if down.is_empty() { State::Idle } else { State::Draining }
                } else if down.is_empty() {
                    // A tap's fingers seldom lift in the same frame: it is
                    // decided when the last one does.
                    if age <= rules.tap_seconds {
                        events.push(if most >= 3 { GestureEvent::Redo } else { GestureEvent::Undo });
                    }
                    State::Idle
                } else if down.len() == 1 {
                    if age <= rules.tap_seconds {
                        State::Multi { ids, start_mid, start_spread, mid, spread, age, moved, most }
                    } else {
                        State::Draining
                    }
                } else {
                    let (now_mid, now_spread) = pair(&down);
                    if [down[0].id, down[1].id] != ids {
                        // Another pair: it carries on from where it is.
                        State::Multi {
                            ids: [down[0].id, down[1].id],
                            start_mid: now_mid,
                            start_spread: now_spread,
                            mid: now_mid,
                            spread: now_spread,
                            age,
                            moved,
                            most,
                        }
                    } else {
                        let moved = moved || now_mid.distance_to(start_mid) > rules.slop || (now_spread - start_spread).abs() > rules.slop;
                        if moved {
                            events.push(GestureEvent::Move { from: mid, to: now_mid, factor: now_spread / spread });
                            State::Multi { ids, start_mid, start_spread, mid: now_mid, spread: now_spread, age, moved, most }
                        } else {
                            State::Multi { ids, start_mid, start_spread, mid, spread, age, moved, most }
                        }
                    }
                }
            }
            State::Draining => match down.len() {
                0 => State::Idle,
                1 => State::Draining,
                _ => multi(&down, rules.tap_seconds, true),
            },
        };
        events
    }
}

/// A gesture of the first two fingers in `down` (at least two), `age`
/// seconds in, having `moved` the view already or not.
fn multi(down: &[TouchPoint], age: f32, moved: bool) -> State {
    let (mid, spread) = pair(down);
    State::Multi { ids: [down[0].id, down[1].id], start_mid: mid, start_spread: spread, mid, spread, age, moved, most: down.len() }
}

/// The middle of the first two fingers and how far apart they are (never
/// under a pixel, so a pinch's ratio stays finite).
fn pair(down: &[TouchPoint]) -> (Vec2, f32) {
    let (a, b) = (down[0].pos, down[1].pos);
    (Vec2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0), a.distance_to(b).max(1.0))
}

#[cfg(test)]
mod gesture_tests {
    use super::*;

    const RULES: GestureRules = GestureRules { slop: 10.0, tap_seconds: 0.35 };
    const DT: f32 = 1.0 / 60.0;

    fn t(id: i32, x: f32, y: f32) -> TouchPoint {
        TouchPoint { id, pos: Vec2::new(x, y) }
    }

    /// Run `frames` through a fresh recogniser, every finger the canvas's.
    fn run(frames: &[Vec<TouchPoint>], paints: bool) -> Vec<GestureEvent> {
        let mut g = Gestures::default();
        let mut out = Vec::new();
        for f in frames {
            out.extend(g.update(f, |_| true, paints, DT, &RULES));
        }
        assert!(!g.active() || !frames.last().is_some_and(|f| f.is_empty()), "every finger lifted ends the gesture");
        out
    }

    #[test]
    fn a_resting_finger_paints_nothing_and_a_quick_one_is_a_tap() {
        // Within the slop for a while, then lifted late: nothing at all.
        let mut frames: Vec<Vec<TouchPoint>> = (0..40).map(|i| vec![t(1, 100.0 + (i % 3) as f32, 100.0)]).collect();
        frames.push(vec![]);
        assert_eq!(run(&frames, true), vec![]);
        // Lifted quickly: a tap where it landed.
        let frames = vec![vec![t(1, 100.0, 100.0)], vec![t(1, 103.0, 101.0)], vec![]];
        assert_eq!(run(&frames, true), vec![GestureEvent::Tap(Vec2::new(100.0, 100.0))]);
    }

    #[test]
    fn one_finger_past_the_slop_strokes_from_where_it_landed_or_pans() {
        let frames = vec![vec![t(1, 100.0, 100.0)], vec![t(1, 106.0, 100.0)], vec![t(1, 140.0, 100.0)], vec![t(1, 180.0, 100.0)], vec![]];
        assert_eq!(
            run(&frames, true),
            vec![
                GestureEvent::StrokeBegin(Vec2::new(100.0, 100.0)),
                GestureEvent::StrokeTo(Vec2::new(140.0, 100.0)),
                GestureEvent::StrokeTo(Vec2::new(180.0, 100.0)),
                GestureEvent::StrokeEnd,
            ]
        );
        assert_eq!(
            run(&frames, false),
            vec![
                GestureEvent::Move { from: Vec2::new(100.0, 100.0), to: Vec2::new(140.0, 100.0), factor: 1.0 },
                GestureEvent::Move { from: Vec2::new(140.0, 100.0), to: Vec2::new(180.0, 100.0), factor: 1.0 },
            ]
        );
    }

    #[test]
    fn two_fingers_pinch_and_pan_about_their_middle_once_past_the_slop() {
        let frames = vec![
            vec![t(1, 100.0, 100.0)],
            vec![t(1, 100.0, 100.0), t(2, 200.0, 100.0)],
            vec![t(1, 97.0, 100.0), t(2, 203.0, 100.0)],
            vec![t(1, 80.0, 100.0), t(2, 220.0, 100.0)],
            vec![t(1, 90.0, 130.0), t(2, 230.0, 130.0)],
            vec![],
        ];
        let events = run(&frames, true);
        assert_eq!(events.len(), 3, "{events:?}");
        // The first move carries everything since the second finger landed.
        assert_eq!(events[0], GestureEvent::Move { from: Vec2::new(150.0, 100.0), to: Vec2::new(150.0, 100.0), factor: 1.4 });
        assert_eq!(events[1], GestureEvent::Move { from: Vec2::new(150.0, 100.0), to: Vec2::new(160.0, 130.0), factor: 1.0 });
        assert_eq!(events[2], GestureEvent::Settle(Vec2::new(160.0, 130.0)));
    }

    #[test]
    fn two_finger_taps_undo_and_three_finger_taps_redo() {
        let two = vec![vec![t(1, 100.0, 100.0)], vec![t(1, 100.0, 100.0), t(2, 160.0, 100.0)], vec![t(2, 160.0, 100.0)], vec![]];
        assert_eq!(run(&two, true), vec![GestureEvent::Undo]);
        let three = vec![
            vec![t(1, 100.0, 100.0), t(2, 160.0, 100.0)],
            vec![t(1, 100.0, 100.0), t(2, 160.0, 100.0), t(3, 130.0, 160.0)],
            vec![t(3, 130.0, 160.0)],
            vec![],
        ];
        assert_eq!(run(&three, true), vec![GestureEvent::Redo]);
        // Held too long, or moved: no tap.
        let mut slow = vec![vec![t(1, 100.0, 100.0), t(2, 160.0, 100.0)]; 40];
        slow.push(vec![]);
        assert_eq!(run(&slow, true), vec![]);
    }

    #[test]
    fn a_second_finger_on_a_stroke_takes_it_back_and_the_last_finger_does_nothing() {
        let frames = vec![
            vec![t(1, 100.0, 100.0)],
            vec![t(1, 150.0, 100.0)],
            vec![t(1, 150.0, 100.0), t(2, 250.0, 100.0)],
            vec![t(1, 140.0, 100.0), t(2, 240.0, 100.0)],
            vec![t(2, 240.0, 100.0)],
            vec![t(2, 300.0, 140.0)],
            vec![],
        ];
        let events = run(&frames, true);
        assert_eq!(&events[..3], &[GestureEvent::StrokeBegin(Vec2::new(100.0, 100.0)), GestureEvent::StrokeTo(Vec2::new(150.0, 100.0)), GestureEvent::StrokeCancel]);
        assert!(matches!(events[3], GestureEvent::Move { .. }), "{events:?}");
        assert_eq!(events[4], GestureEvent::Settle(Vec2::new(190.0, 100.0)));
        assert_eq!(events.len(), 5, "the finger left behind neither paints nor pans: {events:?}");
    }

    #[test]
    fn a_finger_that_landed_off_the_canvas_is_no_part_of_a_gesture() {
        let mut g = Gestures::default();
        let on_canvas = |p: &TouchPoint| p.pos.y > 32.0;
        let mut events = g.update(&[t(1, 100.0, 10.0)], on_canvas, true, DT, &RULES);
        events.extend(g.update(&[t(1, 300.0, 200.0)], on_canvas, true, DT, &RULES));
        events.extend(g.update(&[], on_canvas, true, DT, &RULES));
        assert_eq!(events, vec![]);
        assert!(!g.active());
    }
}
