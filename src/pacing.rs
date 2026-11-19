//! Frame pacing: the display's refresh period read off the frame times,
//! a frame time snapped to it, and the clock that pays frame time out in
//! simulation steps. Headless and pure; `app.rs` owns one of each.
//!
//! A display paced by vsync shows frames a whole number of refresh
//! periods apart, but the time measured between two frames wobbles
//! round that by a fraction of a millisecond, and a "60 Hz" panel is
//! often 59.94. Fed raw to a 60 Hz step clock, either makes a frame now
//! and then run no step or two. `Refresh` finds the period, `snap` puts a
//! frame time on the nearest whole number of them when it is close, and
//! `StepClock` pays the result out in steps and says how far into the
//! next step the frame stands (`alpha`, which `Game::pose_at` draws at).

use crate::{PHYSICS_FIXED_DT, SIM_MAX_STEPS_PER_FRAME};

/// How far a frame time may stand from a whole number of refresh periods
/// and still be snapped onto it, as a share of one period: 0.83 ms at
/// 60 Hz. Wider than a vsynced frame's wobble, far narrower than the gap
/// to a frame that really was paced differently.
pub const SNAP_TOLERANCE: f32 = 0.05;

/// The refresh rates a measured period is matched against, Hz. A panel
/// at a rate that is not here is not snapped to.
const NOMINAL_HZ: [f32; 12] = [24.0, 30.0, 48.0, 50.0, 60.0, 72.0, 75.0, 90.0, 100.0, 120.0, 144.0, 240.0];

/// How far a measured period may stand from a nominal one and be taken
/// for it, as a share of the period: 59.94 Hz is 60, 119.88 is 120.
const NOMINAL_TOLERANCE: f32 = 0.02;

/// The frame times `Refresh` keeps.
const REFRESH_WINDOW: usize = 15;

/// The frame times it needs before it names a period.
const REFRESH_MIN_FRAMES: usize = 8;

/// `dt` on the nearest whole number of `period`s (one at least) when it
/// stands within `tolerance` of a period from it, else `dt` as it is.
/// A `period` that is not a positive number snaps nothing.
pub fn snap(dt: f32, period: f32, tolerance: f32) -> f32 {
    if !(period.is_finite() && period > 0.0 && dt.is_finite() && dt > 0.0) {
        return dt;
    }
    let whole = (dt / period).round();
    if whole < 1.0 {
        return dt;
    }
    let snapped = whole * period;
    if (dt - snapped).abs() <= tolerance * period { snapped } else { dt }
}

/// The nominal refresh period a measured one stands for, seconds: 1/60
/// for 16.68 ms. `None` when it is near none of `NOMINAL_HZ`.
pub fn nominal_period(measured: f32) -> Option<f32> {
    if !(measured.is_finite() && measured > 0.0) {
        return None;
    }
    NOMINAL_HZ.iter().map(|hz| 1.0 / hz).find(|period| (measured - period).abs() <= NOMINAL_TOLERANCE * period)
}

/// The display's refresh period, measured: the median of the last few
/// frame times, taken for the nominal period it is near. The median
/// shrugs off a hitch, and a display that drops every other frame reads
/// as one of half the rate, whose multiples are still the real ones.
#[derive(Clone, Debug, Default)]
pub struct Refresh {
    recent: [f32; REFRESH_WINDOW],
    seen: usize,
}

impl Refresh {
    /// Note a frame's raw time.
    pub fn observe(&mut self, dt: f32) {
        if dt.is_finite() && dt > 0.0 {
            self.recent[self.seen % REFRESH_WINDOW] = dt;
            self.seen += 1;
        }
    }

    /// The refresh period, seconds: `None` before enough frames, and
    /// while they are paced at no rate it knows.
    pub fn period(&self) -> Option<f32> {
        let n = self.seen.min(REFRESH_WINDOW);
        if n < REFRESH_MIN_FRAMES {
            return None;
        }
        let mut sorted = self.recent;
        let sorted = &mut sorted[..n];
        sorted.sort_by(f32::total_cmp);
        nominal_period(sorted[n / 2])
    }

    /// Note `dt` and answer it snapped to the period as measured so far.
    pub fn pace(&mut self, dt: f32) -> f32 {
        self.observe(dt);
        match self.period() {
            Some(period) => snap(dt, period, SNAP_TOLERANCE),
            None => dt,
        }
    }
}

/// Frames in a row that must each pay for a whole number of steps before
/// the clock takes the display to be in step with the simulation.
const IN_STEP_AFTER: u32 = 30;

/// How near a whole number of steps a frame time must be to count as one,
/// as a share of a step.
const WHOLE_STEP_TOLERANCE: f32 = 1e-3;

/// The play clock: real frame time paid out in whole simulation steps of
/// `PHYSICS_FIXED_DT`, so `Game::update` always sees the step the dev
/// server, the probe and a replay see (docs/online-coop-prd.md §4.1). A
/// rendered frame runs the steps it has accumulated - none on every other
/// frame of a 120 Hz display, one per frame at 60 Hz, two after a hitch -
/// and at most `SIM_MAX_STEPS_PER_FRAME`: a frame owing more runs the cap
/// and forgets the rest, so a stall costs the round real time, never a
/// spiral of catch-up steps.
///
/// What is left over is how far the frame stands into the next step:
/// `alpha`, the pose the frame draws (`Game::pose_at`). Drawing between
/// two steps shows the round one step late, which buys nothing on a
/// display whose every frame is a whole number of steps (60 Hz, 30 Hz):
/// there every frame draws the round as it stands, as it would with no
/// interpolation at all. The clock takes the display to be in step after
/// `IN_STEP_AFTER` such frames in a row and stops at the first that is
/// not - a hitch, the slowed end of a round - which is drawn between its
/// steps like every frame after it. A frame of no time at all (the held
/// blow) changes nothing.
///
/// Going in or out of step moves the picture between "one step late" and
/// "as it stands". So that it never jumps a whole step, a clock in step
/// keeps between half a step and a step and a half owed (`keeps`): going
/// in, a clock under half a step holds one step back; going out, what it
/// held is paid. Either way the picture is off by half a step at most,
/// once.
#[derive(Clone, Debug, Default)]
pub struct StepClock {
    /// Real seconds not yet paid out as a step: below one step after
    /// `advance`, or half a step to a step and a half on a clock in step.
    owed: f32,
    /// Frames in a row that paid for a whole number of steps.
    whole: u32,
    in_step: bool,
}

impl StepClock {
    /// What stays owed after a frame's steps: less than this.
    fn keeps(&self) -> f32 {
        if self.in_step { 1.5 * PHYSICS_FIXED_DT } else { PHYSICS_FIXED_DT }
    }

    /// Add a rendered frame's `dt` and take the steps it pays for.
    pub fn advance(&mut self, dt: f32) -> u32 {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        if dt == 0.0 {
            return 0;
        }
        let paid = (dt / PHYSICS_FIXED_DT).round();
        let exact = paid >= 1.0 && (dt - paid * PHYSICS_FIXED_DT).abs() <= WHOLE_STEP_TOLERANCE * PHYSICS_FIXED_DT;
        if exact {
            self.whole = self.whole.saturating_add(1);
            if self.whole >= IN_STEP_AFTER {
                self.in_step = true;
            }
        } else {
            self.whole = 0;
            self.in_step = false;
        }
        let keeps = self.keeps();
        // In step and holding its share: the frame's steps are the ones it
        // paid for, and what is owed is not touched, so no rounding ever
        // tips a frame into none or two.
        if self.in_step && exact && self.owed >= keeps - PHYSICS_FIXED_DT && self.owed < keeps && paid <= SIM_MAX_STEPS_PER_FRAME as f32 {
            return paid as u32;
        }
        let cap = SIM_MAX_STEPS_PER_FRAME as f32 * PHYSICS_FIXED_DT;
        self.owed = (self.owed + dt).min(cap + keeps - PHYSICS_FIXED_DT);
        let mut steps = 0;
        while self.owed >= keeps && steps < SIM_MAX_STEPS_PER_FRAME {
            self.owed -= PHYSICS_FIXED_DT;
            steps += 1;
        }
        if steps == SIM_MAX_STEPS_PER_FRAME {
            self.owed = keeps - PHYSICS_FIXED_DT;
        }
        steps
    }

    /// How far the frame stands into the next step, 0..=1: what
    /// `Game::pose_at` draws at. 1 - the round as it stands - on a display
    /// in step with the simulation.
    pub fn alpha(&self) -> f32 {
        if self.in_step { 1.0 } else { (self.owed / PHYSICS_FIXED_DT).clamp(0.0, 1.0) }
    }

    /// Forget what is owed: the dev server froze the round, which then
    /// resumes on a fresh step rather than on the rest of an old one.
    pub fn reset(&mut self) {
        self.owed = self.keeps() - PHYSICS_FIXED_DT;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: f32 = PHYSICS_FIXED_DT;

    #[test]
    fn a_frame_time_near_a_whole_number_of_periods_lands_on_it() {
        let period = 1.0 / 60.0;
        assert_eq!(snap(0.016_683, period, SNAP_TOLERANCE), period, "59.94 Hz");
        assert_eq!(snap(0.016_2, period, SNAP_TOLERANCE), period);
        assert_eq!(snap(0.017_3, period, SNAP_TOLERANCE), period);
        assert_eq!(snap(0.033_9, period, SNAP_TOLERANCE), 2.0 * period, "a dropped frame is two periods");
        assert_eq!(snap(0.05, period, SNAP_TOLERANCE), 3.0 * period);
    }

    #[test]
    fn a_frame_time_far_from_one_is_left_alone() {
        let period = 1.0 / 60.0;
        for dt in [0.015, 0.018, 0.025, 0.008, 0.001, 0.0] {
            assert_eq!(snap(dt, period, SNAP_TOLERANCE), dt, "{dt}");
        }
        // Half a period rounds to one and is nowhere near it.
        assert_eq!(snap(period / 2.0, period, SNAP_TOLERANCE), period / 2.0);
        // No period, no snap.
        for period in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(snap(0.016, period, SNAP_TOLERANCE), 0.016);
        }
        assert!(snap(f32::NAN, 1.0 / 60.0, SNAP_TOLERANCE).is_nan());
    }

    #[test]
    fn a_measured_period_is_taken_for_the_nominal_one() {
        assert_eq!(nominal_period(1.0 / 59.94), Some(1.0 / 60.0));
        assert_eq!(nominal_period(1.0 / 119.88), Some(1.0 / 120.0));
        assert_eq!(nominal_period(0.020), Some(1.0 / 50.0));
        assert_eq!(nominal_period(1.0 / 144.0), Some(1.0 / 144.0));
        assert_eq!(nominal_period(1.0 / 165.0), None, "a rate it does not know");
        assert_eq!(nominal_period(0.0), None);
        assert_eq!(nominal_period(f32::NAN), None);
    }

    #[test]
    fn the_refresh_period_is_the_median_frame_time() {
        let mut refresh = Refresh::default();
        for _ in 0..REFRESH_MIN_FRAMES - 1 {
            refresh.observe(0.0167);
            assert_eq!(refresh.period(), None, "not before it has seen enough");
        }
        refresh.observe(0.0166);
        assert_eq!(refresh.period(), Some(1.0 / 60.0));
        // A hitch does not move it.
        refresh.observe(0.120);
        refresh.observe(0.045);
        assert_eq!(refresh.period(), Some(1.0 / 60.0));
        // A display at another rate does, once most of the window is its.
        for _ in 0..REFRESH_WINDOW {
            refresh.observe(0.00834);
        }
        assert_eq!(refresh.period(), Some(1.0 / 120.0));
        // Frames paced at no rate it knows: nothing to snap to.
        for _ in 0..REFRESH_WINDOW {
            refresh.observe(0.0123);
        }
        assert_eq!(refresh.period(), None);
    }

    #[test]
    fn a_wobbling_sixty_hertz_display_steps_once_every_frame() {
        // 59.94 Hz with a wobble: raw, the clock would now and then run no
        // step or two; paced, every frame is one.
        let mut refresh = Refresh::default();
        let mut clock = StepClock::default();
        let mut raw = StepClock::default();
        let (mut odd, mut raw_odd) = (0, 0);
        for frame in 0..4000 {
            let wobble = [0.0003, -0.0004, 0.0001, -0.0002, 0.0005, -0.0003][frame % 6];
            let dt = 1.0 / 59.94 + wobble;
            let steps = clock.advance(refresh.pace(dt));
            // Once the period is measured and the clock has fallen in
            // step (which may hold one step back, once).
            let settled = frame >= 2 * IN_STEP_AFTER as usize;
            if settled && steps != 1 {
                odd += 1;
            }
            if raw.advance(dt) != 1 && settled {
                raw_odd += 1;
            }
        }
        assert_eq!(odd, 0, "every paced frame runs one step");
        assert!(raw_odd > 0, "the raw clock does not: the test would prove nothing otherwise");
    }

    #[test]
    fn the_step_clock_pays_frame_time_out_in_whole_steps() {
        // 60 Hz: one step a frame.
        let mut clock = StepClock::default();
        assert_eq!((0..10).map(|_| clock.advance(STEP)).sum::<u32>(), 10);
        // 120 Hz: a step every other frame.
        let mut clock = StepClock::default();
        let steps: Vec<u32> = (0..6).map(|_| clock.advance(STEP / 2.0)).collect();
        assert_eq!(steps, [0, 1, 0, 1, 0, 1]);
        // 30 Hz: two a frame.
        let mut clock = StepClock::default();
        assert_eq!(clock.advance(2.0 * STEP), 2);
        // A stall runs the cap and forgets the rest.
        let mut clock = StepClock::default();
        assert_eq!(clock.advance(1.0), SIM_MAX_STEPS_PER_FRAME);
        assert_eq!(clock.advance(0.2 * STEP), 0, "nothing is owed after a stall");
        // A reset forgets the time owed.
        let mut clock = StepClock::default();
        assert_eq!(clock.advance(0.9 * STEP), 0);
        clock.reset();
        assert_eq!(clock.advance(0.2 * STEP), 0);
        // Time that is not a number, or runs backwards, pays for nothing.
        let mut clock = StepClock::default();
        assert_eq!(clock.advance(f32::NAN), 0);
        assert_eq!(clock.advance(-1.0), 0);
        assert_eq!(clock.alpha(), 0.0);
    }

    #[test]
    fn alpha_is_how_far_the_frame_stands_into_the_next_step() {
        // 120 Hz: a frame half way to the next step, then one on it.
        let mut clock = StepClock::default();
        clock.advance(STEP / 2.0);
        assert!((clock.alpha() - 0.5).abs() < 1e-4);
        clock.advance(STEP / 2.0);
        assert!(clock.alpha() < 1e-4);
        // 144 Hz: it walks through the step and never leaves 0..1.
        let mut clock = StepClock::default();
        let mut last = 0.0;
        let mut total = 0.0;
        for _ in 0..500 {
            let steps = clock.advance(1.0 / 144.0);
            let alpha = clock.alpha();
            assert!((0.0..1.0).contains(&alpha));
            // The time shown - steps run plus the share of the next - moves
            // on by the frame's own time, every frame.
            let shown = steps as f32 + alpha - last;
            assert!((shown * STEP - 1.0 / 144.0).abs() < 1e-5, "{shown}");
            total += shown;
            last = alpha;
        }
        assert!((total * STEP - 500.0 / 144.0).abs() < 1e-3);
    }

    #[test]
    fn a_display_in_step_with_the_simulation_draws_the_round_as_it_stands() {
        let mut clock = StepClock::default();
        for _ in 0..IN_STEP_AFTER - 1 {
            assert_eq!(clock.advance(STEP), 1);
            assert_eq!(clock.alpha(), 0.0, "not yet: one step late");
        }
        clock.advance(STEP);
        assert_eq!(clock.alpha(), 1.0, "in step: no step of latency for nothing");
        assert!((0..600).all(|_| clock.advance(STEP) == 1 && clock.alpha() == 1.0), "one step a frame, for good");
        assert!((0..60).all(|_| clock.advance(2.0 * STEP) == 2), "and two on a dropped frame");
        // A frame of no time (the held blow) changes nothing ...
        assert_eq!(clock.advance(0.0), 0);
        assert_eq!(clock.alpha(), 1.0);
        // ... an uneven one (a hitch, the slowed end of a round) ends it,
        // and it is back after as many even frames as it took.
        clock.advance(0.3 * STEP);
        assert!(clock.alpha() < 1.0);
        for _ in 0..IN_STEP_AFTER - 1 {
            clock.advance(STEP);
            assert!(clock.alpha() < 1.0);
        }
        clock.advance(STEP);
        assert_eq!(clock.alpha(), 1.0);
        // And a 120 Hz display is never in step: it needs the frames
        // between.
        let mut clock = StepClock::default();
        for _ in 0..200 {
            clock.advance(STEP / 2.0);
        }
        assert!(clock.alpha() < 1.0);
    }

    /// The time a frame shows, in steps: the steps run, less the one a
    /// frame between two steps is behind by, plus its share of the next.
    fn shown(steps_run: u32, clock: &StepClock) -> f32 {
        steps_run as f32 - 1.0 + clock.alpha()
    }

    #[test]
    fn going_in_and_out_of_step_never_jumps_the_picture_a_whole_step() {
        // Whatever the clock holds when the display falls in step, and
        // through a slowed stretch and back, the picture moves on by the
        // frame's own time give or take half a step - and by exactly that
        // time on every frame but the few where it goes in or out.
        for start in [0.0, 0.2, 0.49, 0.5, 0.7, 0.95] {
            let mut clock = StepClock::default();
            clock.advance(start * STEP);
            let mut run = 0;
            let mut last = shown(run, &clock);
            let (mut off, mut time, mut picture) = (0, 0.0, 0.0);
            let frames = std::iter::repeat_n(1.0, 90).chain(std::iter::repeat_n(0.3, 40)).chain(std::iter::repeat_n(1.0, 90)).chain([1.6, 1.0, 1.0, 0.4]);
            for (i, d) in frames.enumerate() {
                run += clock.advance(d * STEP);
                let now = shown(run, &clock);
                let error = (now - last) - d;
                assert!(error.abs() <= 0.5 + 1e-3, "start {start}, frame {i}: the picture jumped {error} steps");
                if error.abs() > 1e-3 {
                    off += 1;
                }
                time += d;
                picture += now - last;
                last = now;
            }
            assert!(off <= 4, "start {start}: {off} frames off their own time");
            assert!((picture - time).abs() <= 1.0 + 1e-3, "start {start}: the picture keeps up with the clock");
        }
    }

}
