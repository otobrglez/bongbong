//! The picture between two snapshots (docs/online-coop-prd.md §4.5): the
//! clock that says what "now" is on the server, the ring of snapshots
//! that have arrived, and the blend of two of them the replica is written
//! from each rendered frame.
//!
//! A room sends a snapshot every tick, sixty a second. Writing each one
//! into the replica as it arrives would still draw the round at the
//! room's cadence on a 120 Hz screen - and at any cadence a packet can be
//! late or lost - so the replica draws the *past* instead: render time is
//! the server's clock minus a delay, which keeps two snapshots bracketing
//! it, and every frame lands somewhere between them.
//!
//! **The delay is adaptive** (docs/online-coop-prd.md §4.5, decision 8).
//! Its floor is `online_interpolation_delay_ms`, two intervals at the
//! room's cadence; on top of it goes the link's own jitter - the smoothed
//! deviation of each snapshot's arrival from the clock's estimate
//! (`ServerClock::jitter_ms`) - times `online_interpolation_jitter_factor`,
//! capped at `online_interpolation_delay_max_ms`. A clean link pays the
//! floor; a jittery one pays what its late packets need, and no more. The
//! delay in force never jumps to its target: it slews at
//! `DELAY_WIDEN_PER_MS` / `DELAY_NARROW_PER_MS`, so a jittery spell
//! stretches the picture's time by a few per cent rather than rewinding
//! it, and a link that settles gives the milliseconds back the same way.
//! `online_interpolation_adaptive` off pins the delay at the floor.
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
pub const SNAPSHOT_INTERVAL_MS: f64 = 1000.0 / 60.0;

/// How far past the newest snapshot the hulls are carried on their last
/// velocity before the picture holds still.
///
/// Counted in *intervals*, so it shrinks in wall-clock terms whenever the
/// cadence goes up: at 20 Hz two intervals rode out 100 ms, at 60 Hz they
/// would ride out 33. Four keeps roughly the old 66 ms of gap coverage at
/// the new cadence, which is what a dropped packet or a stalled link
/// actually needs; beyond that the guess is worse than a freeze.
pub const EXTRAPOLATION_INTERVALS: f64 = 4.0;

/// How much of each reading the clock offset takes: a tenth, so a
/// jittered arrival moves render time by a tenth of its lateness and the
/// estimate settles within a second of snapshots.
pub const CLOCK_GAIN: f64 = 0.1;

/// A reading this far from the estimate is not jitter but another clock -
/// a rejoin, a room restarted under the same socket - and is taken whole
/// instead of smoothed toward over a minute.
pub const CLOCK_SNAP_MS: f64 = 500.0;

/// How fast the delay in force may grow toward a larger target, in
/// milliseconds of delay per millisecond of real time: a tenth, so
/// widening by 20 ms takes a fifth of a second, during which the picture
/// runs at nine tenths speed. Growing is the urgent direction - a late
/// packet is already being ridden out on extrapolation - so it is five
/// times the shrink rate.
pub const DELAY_WIDEN_PER_MS: f64 = 0.1;

/// How fast the delay in force may shrink toward a smaller target: a
/// fiftieth, so a link that settles gives 20 ms back over a second at
/// two per cent over speed, which nothing notices.
pub const DELAY_NARROW_PER_MS: f64 = 0.02;

/// How many snapshots are kept. Render time sits one interval or so
/// behind the newest, so this is room for jitter and loss before the
/// oldest is dropped unseen.
///
/// A count, not a duration, so it halves in wall-clock terms every time
/// the cadence doubles: eight covered 266 ms at 30 Hz and would cover
/// 133 at 60. Sixteen keeps the window a quarter of a second, which is
/// what the number was really buying.
const BUFFERED_SNAPSHOTS: usize = 16;

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
    /// The smoothed distance of each reading from the estimate: how far
    /// a snapshot's arrival strays from where the clock expected it, the
    /// jitter the adaptive delay covers. Zero on a perfect link.
    jitter_ms: f64,
}

impl ServerClock {
    /// A snapshot stamped `server_ms` arrived at local time `local_ms`.
    pub fn observe(&mut self, server_ms: u32, local_ms: i64) {
        let raw = server_ms as f64 - local_ms as f64;
        self.offset_ms = Some(match self.offset_ms {
            Some(estimate) if (raw - estimate).abs() < CLOCK_SNAP_MS => {
                let deviation = (raw - estimate).abs();
                self.jitter_ms += (deviation - self.jitter_ms) * CLOCK_GAIN;
                estimate + (raw - estimate) * CLOCK_GAIN
            }
            _ => {
                // Another clock: nothing measured against the old one
                // says anything about this link.
                self.jitter_ms = 0.0;
                raw
            }
        });
    }

    /// The link's jitter as the clock sees it, in milliseconds.
    pub fn jitter_ms(&self) -> f64 {
        self.jitter_ms
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

/// What the interpolator is doing, for `status.round.interpolation` and
/// the rig's tests (docs/online-coop-prd.md §4.12, "Measured").
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InterpReport {
    /// The delay in force, milliseconds behind the server's clock.
    pub delay_ms: f64,
    /// Where the delay is heading: the floor plus the jitter margin.
    pub target_ms: f64,
    /// The link's jitter as the clock sees it.
    pub jitter_ms: f64,
    /// The room's measured cadence.
    pub interval_ms: f64,
    /// Snapshots waiting to be drawn.
    pub buffered: usize,
    /// Frames drawn past the newest snapshot on last-known velocities:
    /// each one is a packet the delay did not cover.
    pub extrapolated_frames: u64,
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
    /// The delay in force; `None` until the first frame is sampled, when
    /// it starts at its target rather than slewing up from nothing.
    delay_ms: Option<f64>,
    /// When the delay was last slewed, so the rate is per real time.
    slewed_at: Option<i64>,
    /// Frames that ran on extrapolation, for the report.
    extrapolated_frames: u64,
}

impl Default for Interpolator {
    fn default() -> Interpolator {
        Interpolator {
            buffer: VecDeque::new(),
            clock: ServerClock::default(),
            released: None,
            interval_ms: SNAPSHOT_INTERVAL_MS,
            delay_ms: None,
            slewed_at: None,
            extrapolated_frames: 0,
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
        self.slew_delay(local_ms);
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
                if held > 0.0 {
                    self.extrapolated_frames += 1;
                }
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
        Some(self.clock.now(local_ms)? - self.delay_ms())
    }

    /// Where the delay is heading: the floor, or two measured intervals
    /// if the room's cadence is slower than the floor assumes, plus the
    /// jitter margin, under the cap. With `online_interpolation_adaptive`
    /// off it is the floor alone.
    pub fn target_delay_ms(&self) -> f64 {
        let t = tuning();
        let floor = t.online_interpolation_delay_ms as f64;
        if !t.online_interpolation_adaptive {
            return floor;
        }
        let base = (2.0 * self.interval_ms).max(floor);
        let margin = t.online_interpolation_jitter_factor as f64 * self.clock.jitter_ms();
        (base + margin).min(t.online_interpolation_delay_max_ms as f64).max(floor)
    }

    /// The delay in force, milliseconds: the target until a frame has
    /// been sampled, then wherever the slew has got to.
    pub fn delay_ms(&self) -> f64 {
        self.delay_ms.unwrap_or_else(|| self.target_delay_ms())
    }

    /// Move the delay in force toward its target, no faster than the
    /// slew rates allow over the real time since the last frame.
    fn slew_delay(&mut self, local_ms: i64) {
        let target = self.target_delay_ms();
        let elapsed = self.slewed_at.map_or(0, |at| (local_ms - at).max(0)) as f64;
        self.slewed_at = Some(local_ms);
        self.delay_ms = Some(match self.delay_ms {
            None => target,
            Some(now) if target > now => (now + elapsed * DELAY_WIDEN_PER_MS).min(target),
            Some(now) => (now - elapsed * DELAY_NARROW_PER_MS).max(target),
        });
    }

    /// The readings a status line or a test wants.
    pub fn report(&self) -> InterpReport {
        InterpReport {
            delay_ms: self.delay_ms(),
            target_ms: self.target_delay_ms(),
            jitter_ms: self.clock.jitter_ms(),
            interval_ms: self.interval_ms,
            buffered: self.buffer.len(),
            extrapolated_frames: self.extrapolated_frames,
        }
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
    /// clock reading true and the delay wherever the interpolator has it
    /// (the floor on these exact clocks, plus what two measured intervals
    /// add once the 50 ms cadence of `snapshot` has been seen).
    fn at(interp: &Interpolator, server_ms: f64) -> i64 {
        (server_ms + interp.delay_ms()).round() as i64
    }

    #[test]
    fn render_time_sits_between_the_two_snapshots_that_bracket_it() {
        let mut interp = fed(&[0, 3, 6, 9]);
        // Halfway between the snapshots at tick 3 (50 ms) and 6 (100 ms).
        let frame = interp.sample(at(&interp, 75.0)).expect("a frame");
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
        let later = interp.sample(at(&interp, 91.0)).expect("a frame");
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
        let frame = interp.sample(at(&interp, 95.0)).expect("a frame");
        assert_eq!(frame.snapshot.tanks[0].dir, 1, "the near end owns the facing");
        // Render time reaches the turn: it snaps, whole.
        let frame = interp.sample(at(&interp, 105.0)).expect("a frame");
        assert_eq!(frame.snapshot.tanks[0].dir, 2);
    }

    #[test]
    fn a_gap_runs_on_the_last_velocity_for_four_intervals_and_then_holds() {
        let mut interp = fed(&[0, 3]);
        let newest = snapshot(3);
        let at_newest = newest.server_ms as f64;
        let held = interp.sample(at(&interp, at_newest)).expect("a frame");
        assert_eq!(held.snapshot.tanks[0].x, newest.tanks[0].x, "on the snapshot itself, nothing is guessed");
        assert!(!held.extrapolated);

        // Measured in *intervals*, so the arithmetic is written against
        // the interval the interpolator has actually settled on rather
        // than against a number baked in here - `SNAPSHOT_INTERVAL_MS`
        // only seeds it, and a test that pinned the seed's value broke
        // the moment the room's cadence changed.
        let iv = interp.interval_ms();
        // 200 px/s for one interval, in quarter pixels.
        let per_interval = (200.0 * iv / 1000.0 * 4.0).round() as i16;

        let one = interp.sample(at(&interp, at_newest + iv)).expect("a frame");
        assert!(one.extrapolated);
        assert_eq!(one.snapshot.tanks[0].x, newest.tanks[0].x + per_interval);
        assert_eq!(one.snapshot.frogs[0].x, newest.frogs[0].x, "a frog holds: the wire gives it no velocity");

        let two = interp.sample(at(&interp, at_newest + 2.0 * iv)).expect("a frame");
        assert_eq!(two.snapshot.tanks[0].x, newest.tanks[0].x + 2 * per_interval);

        // `EXTRAPOLATION_INTERVALS` is the cap - four, the count that
        // keeps roughly 66 ms of gap covered now the room cuts a snapshot
        // every tick.
        let cap = EXTRAPOLATION_INTERVALS;
        let at_cap = interp.sample(at(&interp, at_newest + cap * iv)).expect("a frame");
        assert_eq!(at_cap.snapshot.tanks[0].x, newest.tanks[0].x + cap as i16 * per_interval, "the cap is {cap} intervals");
        let past = interp.sample(at(&interp, at_newest + (cap + 1.0) * iv)).expect("a frame");
        assert_eq!(past.snapshot.tanks[0].x, at_cap.snapshot.tanks[0].x, "past the cap the picture holds");
        let stopped = (cap * iv / 1000.0) as f32;
        assert!((past.ahead - stopped).abs() < 0.001, "the round clock stops with it: {} vs {stopped}", past.ahead);
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
        let frame = interp.sample(at(&interp, 20.0)).expect("a frame");
        assert!(frame.snapshot.events.is_empty(), "nothing of a tick not yet drawn");
        // It reaches it: the event goes over with the tick it belongs to.
        let frame = interp.sample(at(&interp, 60.0)).expect("a frame");
        assert_eq!(frame.snapshot.tick, 3);
        assert_eq!(frame.snapshot.events.len(), 1);
        // And never again.
        let frame = interp.sample(at(&interp, 70.0)).expect("a frame");
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
        let frame = interp.sample(at(&interp, 160.0)).expect("a frame");
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
        let frame = interp.sample(at(&interp, 5_000.0)).expect("a frame");
        assert_eq!(frame.snapshot.tick, 0);
        assert_eq!(frame.snapshot.events.len(), 1, "the new round's own events are drawn");
        // A welcome's snapshot is the new baseline and is not drawn twice.
        let mut baseline = snapshot(0);
        baseline.events = vec![blast(1)];
        interp.restart(&baseline, 0);
        assert_eq!(interp.buffered(), 1);
        let frame = interp.sample(at(&interp, 0.0)).expect("a frame");
        assert!(frame.snapshot.events.is_empty(), "the welcome applied its own events");
    }

    /// The delay is the floor on a clean link and widens by the jitter on
    /// a dirty one - and slews there rather than jumping, since a jump in
    /// the delay is a jump in render time, which is the picture rewinding.
    #[test]
    fn the_delay_widens_with_the_links_jitter_and_slews_rather_than_jumps() {
        let floor = tuning().online_interpolation_delay_ms as f64;
        // A perfect link at the room's own cadence: the target is the
        // floor (two intervals of 16.7 ms come to the same number).
        let mut clean = Interpolator::default();
        for tick in 0..120u32 {
            let mut s = snapshot(tick);
            s.server_ms = tick * 1000 / 60;
            clean.accept(s, (tick * 1000 / 60) as i64);
        }
        let report = clean.report();
        assert!(report.jitter_ms < 0.5, "an exact clock has no jitter: {report:?}");
        assert!((report.target_ms - floor).abs() < 1.0, "a clean link pays the floor: {report:?}");

        // The same room over a link whose packets land up to 30 ms late
        // in a pattern: the clock's estimate strays, the jitter reading
        // grows, and the target with it.
        let mut dirty = Interpolator::default();
        for tick in 0..120u32 {
            let mut s = snapshot(tick);
            let sent = tick * 1000 / 60;
            s.server_ms = sent;
            let late = [0, 30, 5, 25, 10, 20][(tick % 6) as usize];
            dirty.accept(s, (sent + late) as i64);
        }
        let report = dirty.report();
        assert!(report.jitter_ms > 5.0, "the late packets read as jitter: {report:?}");
        assert!(
            report.target_ms > floor + 15.0 && report.target_ms <= tuning().online_interpolation_delay_max_ms as f64,
            "the target widened by the margin and stayed under the cap: {report:?}"
        );

        // The delay in force starts at the target on the first frame,
        // and afterwards follows a moved target at the slew rate.
        let first = dirty.sample(2_100).expect("a frame");
        let _ = first;
        let in_force = dirty.delay_ms();
        assert!((in_force - report.target_ms).abs() < 0.01, "the first frame takes the target whole");
        // Ten more perfectly timed snapshots pull the jitter down and the
        // target with it; one frame 16 ms later has moved the delay by
        // at most the narrow rate times sixteen.
        for tick in 120..130u32 {
            let mut s = snapshot(tick);
            s.server_ms = tick * 1000 / 60;
            dirty.accept(s, (tick * 1000 / 60) as i64);
        }
        let lower = dirty.target_delay_ms();
        assert!(lower < in_force, "a settling link lowers the target");
        dirty.sample(2_116).expect("a frame");
        let moved = in_force - dirty.delay_ms();
        assert!(moved > 0.0 && moved <= 16.0 * DELAY_NARROW_PER_MS + 1e-9, "slewed by {moved} ms, not jumped");
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
