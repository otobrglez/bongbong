//! The picture between two snapshots (docs/online-coop-prd.md §4.5): the
//! clock that says what "now" is on the server, the ring of snapshots
//! that have arrived, and the blend of two of them the replica is written
//! from each rendered frame.
//!
//! A room sends twenty snapshots a second. Writing each one into the
//! replica as it arrives would draw a 20 fps round on a 60 or 120 Hz
//! screen, so the replica draws the *past* instead: render time is the
//! server's clock minus `online_interpolation_delay_ms`, which keeps two
//! snapshots bracketing it, and every frame lands somewhere between them.
//!
//! What blends and what does not:
//!
//! - Positions - hulls, shots, frogs - move linearly between the two ends
//!   of the bracket, on the wire's own quarter-pixel grid.
//! - A hull's facing does not: movement is four-directional (`Tank::control`
//!   snaps `rotation`), so a heading is a fact about a tick, not a value to
//!   average. The bracket's near end owns it, and the *drawn* angle swings
//!   across in `Game::tick_presentation` exactly as it does in a local
//!   round. A shot's heading is the same: it only ever changes on a
//!   ricochet, where a snap is the truth.
//! - Everything discrete - tiles, fires, pickups, the round's scalars, the
//!   flags on a hull - is the near end's, so the picture never shows a
//!   state the server has not reached yet.
//! - Events ride the near end and are handed over exactly once, on the
//!   frame render time reaches the snapshot they were cut with, so a
//!   fireball appears when the hull it belongs to is drawn where it died.
//!   `Game::frame` is that snapshot's tick, which is what makes
//!   `Fx::observe` fire once per snapshot rather than once per frame.
//!
//! With nothing newer than the near end - a lost snapshot, a stalled link
//! - the hulls carry on at their last velocity for at most
//! `EXTRAPOLATION_INTERVALS` intervals and then hold. Shots and frogs hold
//! at once: the wire gives them no velocity, and a shell guessed onward
//! through a wall would have to be taken back.
//!
//! No RNG, no `Game`, no raylib: a `Snapshot` in, a `Snapshot` out.

use std::collections::VecDeque;

use crate::net::events::WireEvent;
use crate::net::wire::{Snapshot, dequantise_velocity, quantise_pos};
use crate::tuning::tuning;

/// The gap between two snapshots a room with nothing to say would show:
/// two ticks at 60 Hz, `bongbong_server::room::SNAPSHOT_EVERY`. The
/// starting guess for `Interpolator::interval_ms`, which then follows
/// what actually arrives.
pub const SNAPSHOT_INTERVAL_MS: f64 = 50.0;

/// How far past the newest snapshot the hulls are carried on their last
/// velocity before the picture holds still. Two intervals is a hundred
/// milliseconds of gap ridden out; beyond that the guess is worse than a
/// freeze.
pub const EXTRAPOLATION_INTERVALS: f64 = 2.0;

/// How much of each reading the clock offset takes: a tenth, so a
/// jittered arrival moves render time by a tenth of its lateness and the
/// estimate settles within a second of snapshots.
pub const CLOCK_GAIN: f64 = 0.1;

/// A reading this far from the estimate is not jitter but another clock -
/// a rejoin, a room restarted under the same socket - and is taken whole
/// instead of smoothed toward over a minute.
pub const CLOCK_SNAP_MS: f64 = 500.0;

/// How many snapshots are kept. Render time sits one interval or so
/// behind the newest, so this is room for four intervals of jitter and
/// loss before the oldest is dropped unseen.
const BUFFERED_SNAPSHOTS: usize = 8;

/// The plausible range for a measured interval; anything outside is a
/// gap or a hiccup rather than the room's cadence.
const INTERVAL_RANGE_MS: std::ops::RangeInclusive<f64> = 5.0..=500.0;

/// What the server's clock reads here, from the `server_ms` stamps that
/// have arrived.
///
/// The offset (server minus local) is a plain exponential average: every
/// reading is the true offset minus that packet's travel time, so the
/// estimate settles a little behind the server and jitter moves it by a
/// tenth of itself. The interpolation delay is what absorbs the rest,
/// which is why this does not try to be an NTP.
#[derive(Clone, Debug, Default)]
pub struct ServerClock {
    offset_ms: Option<f64>,
}

impl ServerClock {
    /// A snapshot stamped `server_ms` arrived at local time `local_ms`.
    pub fn observe(&mut self, server_ms: u32, local_ms: i64) {
        let raw = server_ms as f64 - local_ms as f64;
        self.offset_ms = Some(match self.offset_ms {
            Some(estimate) if (raw - estimate).abs() < CLOCK_SNAP_MS => {
                estimate + (raw - estimate) * CLOCK_GAIN
            }
            _ => raw,
        });
    }

    /// What the server's clock reads at local time `local_ms`; `None`
    /// until a first snapshot has been seen.
    pub fn now(&self, local_ms: i64) -> Option<f64> {
        self.offset_ms.map(|offset| local_ms as f64 + offset)
    }

    /// The estimate itself, for a status line.
    pub fn offset_ms(&self) -> Option<f64> {
        self.offset_ms
    }
}

/// The replica's view of one rendered frame: the snapshot to write into
/// it, and how far past that snapshot's tick the picture stands.
#[derive(Clone, Debug)]
pub struct Frame {
    /// The blend, to be handed to `net::apply::snapshot`. Its `tick` is
    /// the bracket's near end and its `events` are empty except on the
    /// frame that end was reached.
    pub snapshot: Snapshot,
    /// Seconds past `snapshot.tick`: the round clock the replica draws
    /// its water, fires and sprite loops from is
    /// `tick * PHYSICS_FIXED_DT + ahead`.
    pub ahead: f32,
    /// The picture is running past the newest snapshot on last-known
    /// velocities rather than between two of them.
    pub extrapolated: bool,
}

/// The snapshots that have arrived and the clock that orders them.
#[derive(Clone, Debug)]
pub struct Interpolator {
    buffer: VecDeque<Snapshot>,
    clock: ServerClock,
    /// The newest tick whose events have been handed to the replica.
    released: Option<u32>,
    /// The room's measured cadence.
    interval_ms: f64,
}

impl Default for Interpolator {
    fn default() -> Interpolator {
        Interpolator {
            buffer: VecDeque::new(),
            clock: ServerClock::default(),
            released: None,
            interval_ms: SNAPSHOT_INTERVAL_MS,
        }
    }
}

impl Interpolator {
    /// Start again from `baseline`, the snapshot inside a `Welcome`:
    /// `net::apply::welcome` has already written it into the replica,
    /// events included, so it is the near end and nothing of it is
    /// released twice. The clock estimate survives - the room's stamps
    /// are the same clock - and a round restarting under it rewinds the
    /// tick, which the release rule reads as a fresh round.
    pub fn restart(&mut self, baseline: &Snapshot, local_ms: i64) {
        self.buffer.clear();
        self.released = Some(baseline.tick);
        self.clock.observe(baseline.server_ms, local_ms);
        self.buffer.push_back(baseline.clone());
    }

    /// A whole snapshot from the room, at local time `local_ms`.
    ///
    /// The client hands snapshots over in tick order, so a repeat of the
    /// newest is dropped and a tick *behind* it is the round having
    /// started over (the server's counter rewinds on `Game::init`):
    /// nothing buffered describes that world any more, so the buffer
    /// starts again from it.
    pub fn accept(&mut self, snapshot: Snapshot, local_ms: i64) {
        self.clock.observe(snapshot.server_ms, local_ms);
        match self.buffer.back() {
            Some(newest) if snapshot.tick == newest.tick => return,
            Some(newest) if snapshot.tick < newest.tick => {
                self.buffer.clear();
                self.released = None;
            }
            Some(newest) => {
                let gap = snapshot.server_ms as f64 - newest.server_ms as f64;
                if INTERVAL_RANGE_MS.contains(&gap) {
                    self.interval_ms += (gap - self.interval_ms) * CLOCK_GAIN;
                }
            }
            None => {}
        }
        self.buffer.push_back(snapshot);
        while self.buffer.len() > BUFFERED_SNAPSHOTS {
            self.buffer.pop_front();
        }
    }

    /// The frame to draw at local time `local_ms`, or `None` before the
    /// first snapshot has arrived.
    ///
    /// Advances the near end past every snapshot render time has reached,
    /// handing over the events of each exactly once, and blends toward
    /// the far end.
    pub fn sample(&mut self, local_ms: i64) -> Option<Frame> {
        let render = self.render_ms(local_ms)?;
        let mut events: Vec<WireEvent> = Vec::new();
        while self.buffer.len() > 1 && self.buffer[1].server_ms as f64 <= render {
            let passed = self.buffer.pop_front().expect("the buffer holds two");
            // A frame long enough to step over a whole snapshot leaves one
            // that was never the near end; its events are still owed.
            if self.owed(passed.tick) {
                events.extend(passed.events);
            }
        }
        let front_tick = self.buffer.front()?.tick;
        if self.owed(front_tick) {
            events.extend(self.buffer[0].events.iter().cloned());
        }
        let from = &self.buffer[0];
        let ahead_ms = (render - from.server_ms as f64).max(0.0);
        let mut frame = match self.buffer.get(1) {
            Some(to) => {
                let span = (to.server_ms as f64 - from.server_ms as f64).max(1.0);
                let alpha = (ahead_ms / span).clamp(0.0, 1.0);
                Frame {
                    snapshot: blend(from, to, alpha as f32),
                    ahead: (ahead_ms.min(span) / 1000.0) as f32,
                    extrapolated: false,
                }
            }
            None => {
                let held = ahead_ms.min(EXTRAPOLATION_INTERVALS * self.interval_ms);
                Frame {
                    snapshot: extrapolate(from, (held / 1000.0) as f32),
                    ahead: (held / 1000.0) as f32,
                    extrapolated: held > 0.0,
                }
            }
        };
        frame.snapshot.events = events;
        Some(frame)
    }

    /// Server time the picture is drawn at: the clock less the delay.
    fn render_ms(&self, local_ms: i64) -> Option<f64> {
        let delay = tuning().online_interpolation_delay_ms as f64;
        Some(self.clock.now(local_ms)? - delay)
    }

    /// Whether `tick`'s events still have to be handed over, marking them
    /// handed over if they do. A tick at or before the last released one
    /// has already been drawn; a rewind clears the mark in `accept`,
    /// since that is a new round.
    fn owed(&mut self, tick: u32) -> bool {
        if self.released.is_some_and(|released| tick <= released) {
            return false;
        }
        self.released = Some(tick);
        true
    }

    /// The room's measured cadence in milliseconds.
    pub fn interval_ms(&self) -> f64 {
        self.interval_ms
    }

    /// How many snapshots are waiting to be drawn.
    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }

    /// The clock the render time is taken from.
    pub fn clock(&self) -> &ServerClock {
        &self.clock
    }

    /// The newest snapshot's tick, whether or not it has been drawn yet.
    pub fn newest_tick(&self) -> Option<u32> {
        self.buffer.back().map(|s| s.tick)
    }

    /// How far ahead of the picture the newest snapshot stands, in
    /// milliseconds: the depth of the buffer the delay is buying. It
    /// hovers around the delay on a steady link, dips toward zero when
    /// packets are late, and goes negative while the picture runs on
    /// extrapolation.
    pub fn lead_ms(&self, local_ms: i64) -> Option<f64> {
        let render = self.render_ms(local_ms)?;
        Some(self.buffer.back()?.server_ms as f64 - render)
    }
}

/// `from`, with every position moved `alpha` of the way toward `to`.
fn blend(from: &Snapshot, to: &Snapshot, alpha: f32) -> Snapshot {
    let mut out = from.clone();
    for tank in &mut out.tanks {
        if let Ok(i) = to.tanks.binary_search_by_key(&tank.id, |t| t.id) {
            tank.x = lerp(tank.x, to.tanks[i].x, alpha);
            tank.y = lerp(tank.y, to.tanks[i].y, alpha);
        }
    }
    for shot in &mut out.shots {
        if let Ok(i) = to.shots.binary_search_by_key(&shot.id, |s| s.id) {
            shot.x = lerp(shot.x, to.shots[i].x, alpha);
            shot.y = lerp(shot.y, to.shots[i].y, alpha);
        }
    }
    for frog in &mut out.frogs {
        if let Some(next) = to.frogs.iter().find(|f| f.side == frog.side) {
            frog.x = lerp(frog.x, next.x, alpha);
            frog.y = lerp(frog.y, next.y, alpha);
        }
    }
    out
}

/// `from`, with every hull carried `ahead` seconds along the velocity it
/// was last seen at. Nothing else moves: a shot has no velocity on the
/// wire and a frog's hop is a scripted arc, so both hold where they are.
fn extrapolate(from: &Snapshot, ahead: f32) -> Snapshot {
    let mut out = from.clone();
    if ahead <= 0.0 {
        return out;
    }
    for tank in &mut out.tanks {
        tank.x = offset(tank.x, dequantise_velocity(tank.vx) * ahead);
        tank.y = offset(tank.y, dequantise_velocity(tank.vy) * ahead);
    }
    out
}

/// Two quantised positions, `alpha` of the way from one to the other.
fn lerp(a: i16, b: i16, alpha: f32) -> i16 {
    let v = a as f32 + (b as f32 - a as f32) * alpha;
    v.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

/// A quantised position moved by `px` pixels.
fn offset(a: i16, px: f32) -> i16 {
    (a as i32 + quantise_pos(px) as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frog::Side;
    use crate::net::wire::{FrogState, TankState, quantise_velocity};

    /// A snapshot at `tick`, stamped as the room would at 20 Hz, with one
    /// hull moving right at 200 px/s.
    fn snapshot(tick: u32) -> Snapshot {
        let ms = tick * 1000 / 60;
        Snapshot {
            tick,
            server_ms: ms,
            tanks: vec![TankState {
                id: 0,
                x: 400 + (tick as i16) * 13,
                y: 800,
                vx: quantise_velocity(200.0),
                dir: 1,
                hp: 100,
                ..Default::default()
            }],
            frogs: vec![FrogState { side: Side::Player, x: 100, y: 100, hp: 100, state: 0, phase: 0, hop_x: 0, hop_y: 0 }],
            ..Default::default()
        }
    }

    /// An event to count, at `x`.
    fn blast(x: i16) -> WireEvent {
        WireEvent::Blast { x, y: 0, chained: false, drum: crate::obstacle::Drum::Oil }
    }

    /// An interpolator whose clock reads the server's exactly: a
    /// snapshot stamped `t` arrived at local `t`.
    fn fed(ticks: &[u32]) -> Interpolator {
        let mut interp = Interpolator::default();
        for &tick in ticks {
            let at = snapshot(tick).server_ms as i64;
            interp.accept(snapshot(tick), at);
        }
        interp
    }

    /// The local time at which render time lands on `server_ms`, with the
    /// clock reading true.
    fn at(server_ms: f64) -> i64 {
        (server_ms + tuning().online_interpolation_delay_ms as f64).round() as i64
    }

    #[test]
    fn render_time_sits_between_the_two_snapshots_that_bracket_it() {
        let mut interp = fed(&[0, 3, 6, 9]);
        // Halfway between the snapshots at tick 3 (50 ms) and 6 (100 ms).
        let frame = interp.sample(at(75.0)).expect("a frame");
        assert_eq!(frame.snapshot.tick, 3, "the near end is the snapshot render time has passed");
        assert!(!frame.extrapolated);
        let (a, b) = (snapshot(3).tanks[0].x, snapshot(6).tanks[0].x);
        assert!(
            (frame.snapshot.tanks[0].x - (a + b) / 2).abs() <= 1,
            "half of the way between {a} and {b}, got {}",
            frame.snapshot.tanks[0].x
        );
        assert!((frame.ahead - 0.025).abs() < 0.002, "25 ms past the near end, got {}", frame.ahead);

        // A frame later, with no snapshot in between, the picture has
        // moved on: this is the whole point of the delay.
        let later = interp.sample(at(91.0)).expect("a frame");
        assert!(later.snapshot.tanks[0].x > frame.snapshot.tanks[0].x, "a frame with no snapshot still moves");
        assert!(later.snapshot.tanks[0].x < b, "and never past the far end");
    }

    #[test]
    fn a_hulls_facing_snaps_at_its_tick_instead_of_blending() {
        let mut interp = Interpolator::default();
        interp.accept(snapshot(3), snapshot(3).server_ms as i64);
        let mut turned = snapshot(6);
        turned.tanks[0].dir = 2;
        interp.accept(turned, snapshot(6).server_ms as i64);
        // Nine tenths of the way toward the snapshot that turned.
        let frame = interp.sample(at(95.0)).expect("a frame");
        assert_eq!(frame.snapshot.tanks[0].dir, 1, "the near end owns the facing");
        // Render time reaches the turn: it snaps, whole.
        let frame = interp.sample(at(105.0)).expect("a frame");
        assert_eq!(frame.snapshot.tanks[0].dir, 2);
    }

    #[test]
    fn a_gap_runs_on_the_last_velocity_for_two_intervals_and_then_holds() {
        let mut interp = fed(&[0, 3]);
        let newest = snapshot(3);
        let at_newest = newest.server_ms as f64;
        let held = interp.sample(at(at_newest)).expect("a frame");
        assert_eq!(held.snapshot.tanks[0].x, newest.tanks[0].x, "on the snapshot itself, nothing is guessed");
        assert!(!held.extrapolated);

        // One interval past it: 50 ms at 200 px/s is 10 px, 40 quarters.
        let one = interp.sample(at(at_newest + 50.0)).expect("a frame");
        assert!(one.extrapolated);
        assert_eq!(one.snapshot.tanks[0].x, newest.tanks[0].x + 40);
        assert_eq!(one.snapshot.frogs[0].x, newest.frogs[0].x, "a frog holds: the wire gives it no velocity");

        // Two intervals is the cap, and a third changes nothing.
        let two = interp.sample(at(at_newest + 100.0)).expect("a frame");
        assert_eq!(two.snapshot.tanks[0].x, newest.tanks[0].x + 80);
        let three = interp.sample(at(at_newest + 150.0)).expect("a frame");
        assert_eq!(three.snapshot.tanks[0].x, two.snapshot.tanks[0].x, "past the cap the picture holds");
        assert!((three.ahead - 0.1).abs() < 0.001, "the round clock stops with it");
    }

    #[test]
    fn the_clock_offset_smooths_toward_a_reading_and_snaps_on_a_jump() {
        let mut clock = ServerClock::default();
        assert_eq!(clock.now(0), None, "nothing is known before the first snapshot");
        // The first reading is taken whole: server 1000 ms at local 0 ms.
        clock.observe(1_000, 0);
        assert_eq!(clock.offset_ms(), Some(1_000.0));
        assert_eq!(clock.now(500), Some(1_500.0));
        // A packet 100 ms late moves the estimate by a tenth of that.
        clock.observe(1_100, 200);
        assert!((clock.offset_ms().unwrap() - 990.0).abs() < 0.001, "{:?}", clock.offset_ms());
        // Steady readings pull it back within a second of snapshots.
        for i in 1..=20 {
            clock.observe(1_100 + i * 50, (100 + i * 50) as i64);
        }
        assert!((clock.offset_ms().unwrap() - 1_000.0).abs() < 2.0, "{:?}", clock.offset_ms());
        // Another clock entirely is taken whole rather than crawled to.
        clock.observe(9_000, 100);
        assert!((clock.offset_ms().unwrap() - 8_900.0).abs() < 0.001, "{:?}", clock.offset_ms());
    }

    #[test]
    fn a_snapshots_events_are_handed_over_once_when_render_time_reaches_it() {
        let mut interp = Interpolator::default();
        let mut with_event = snapshot(3);
        with_event.events = vec![blast(10)];
        interp.accept(snapshot(0), 0);
        interp.accept(with_event, snapshot(3).server_ms as i64);
        interp.accept(snapshot(6), snapshot(6).server_ms as i64);
        // Render time is still short of the snapshot that carries it.
        let frame = interp.sample(at(20.0)).expect("a frame");
        assert!(frame.snapshot.events.is_empty(), "nothing of a tick not yet drawn");
        // It reaches it: the event goes over with the tick it belongs to.
        let frame = interp.sample(at(60.0)).expect("a frame");
        assert_eq!(frame.snapshot.tick, 3);
        assert_eq!(frame.snapshot.events.len(), 1);
        // And never again.
        let frame = interp.sample(at(70.0)).expect("a frame");
        assert!(frame.snapshot.events.is_empty());
    }

    #[test]
    fn a_frame_that_steps_over_a_whole_snapshot_still_hands_its_events_over() {
        let mut interp = Interpolator::default();
        interp.accept(snapshot(0), 0);
        for tick in [3u32, 6, 9] {
            let mut s = snapshot(tick);
            s.events = vec![blast(tick as i16)];
            interp.accept(s, snapshot(tick).server_ms as i64);
        }
        // One long frame, straight past ticks 3 and 6 onto 9.
        let frame = interp.sample(at(160.0)).expect("a frame");
        assert_eq!(frame.snapshot.tick, 9);
        assert_eq!(
            frame.snapshot.events.len(),
            3,
            "every snapshot passed owes its events: {:?}",
            frame.snapshot.events
        );
    }

    #[test]
    fn a_repeat_is_dropped_a_rewind_starts_over_and_a_welcome_is_the_baseline() {
        let mut interp = fed(&[0, 3, 6]);
        assert_eq!(interp.buffered(), 3);
        interp.accept(snapshot(6), 100);
        assert_eq!(interp.buffered(), 3, "the newest again is not news");
        assert_eq!(interp.newest_tick(), Some(6));
        // The round started over: the buffer holds a world that is gone.
        let mut restarted = snapshot(0);
        restarted.server_ms = 5_000;
        restarted.events = vec![WireEvent::RoundStarted {
            seed: 7,
            enemies: 4,
            mission: crate::level::Mission::Protect,
            spawn: crate::level::SpawnKind::Band,
        }];
        interp.accept(restarted, 5_000);
        assert_eq!(interp.buffered(), 1);
        let frame = interp.sample(at(5_000.0)).expect("a frame");
        assert_eq!(frame.snapshot.tick, 0);
        assert_eq!(frame.snapshot.events.len(), 1, "the new round's own events are drawn");
        // A welcome's snapshot is the new baseline and is not drawn twice.
        let mut baseline = snapshot(0);
        baseline.events = vec![blast(1)];
        interp.restart(&baseline, 0);
        assert_eq!(interp.buffered(), 1);
        let frame = interp.sample(at(0.0)).expect("a frame");
        assert!(frame.snapshot.events.is_empty(), "the welcome applied its own events");
    }

    #[test]
    fn the_measured_interval_follows_the_rooms_cadence() {
        let mut interp = Interpolator::default();
        assert_eq!(interp.interval_ms(), SNAPSHOT_INTERVAL_MS);
        // A room on a 100 ms cadence pulls the estimate up.
        for tick in 1..=60u32 {
            let mut s = snapshot(tick);
            s.server_ms = tick * 100;
            interp.accept(s, (tick * 100) as i64);
        }
        assert!((interp.interval_ms() - 100.0).abs() < 1.0, "{}", interp.interval_ms());
    }
}
