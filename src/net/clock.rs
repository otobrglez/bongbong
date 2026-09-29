//! The round trip and the server's clock, measured rather than guessed
//! (docs/online-coop-prd.md §4.15).
//!
//! `interp::ServerClock` reads the server's time off each snapshot's
//! `server_ms` as it arrives, which is right for what it is used for -
//! how deep the snapshot buffer is - and wrong as a clock: every reading
//! is the true time less the packet's one-way delay, so its estimate sits
//! a whole downstream trip behind the server, and it knows nothing of the
//! way up. `RttClock` is the other half: a `Ping` carries the client's own
//! milliseconds, the far end echoes them in a `Pong` with its clock read
//! on the spot, and the client measures the round trip on one clock.
//!
//! The server's time is Cristian's estimate from the **fastest** probe in
//! the window: `server_ms + rtt / 2`, because a probe that took the least
//! time spent the least of it queued behind something, so its midpoint is
//! the tightest bound on when the server read its clock. The round trip
//! itself is reported as the window's median and 95th percentile - what a
//! player feels - and its minimum, the link's floor.
//!
//! No RNG, no `Game`, no raylib, no wall clock: every call is handed the
//! local milliseconds.

use std::collections::VecDeque;

/// How often a probe goes out: four a second, a few bytes each, which is
/// enough to follow a link that changes and nothing a socket notices.
pub const PING_EVERY_MS: i64 = 250;

/// How many answered probes the estimates are read from: ten seconds of
/// them.
pub const PING_WINDOW: usize = 40;

/// A probe unanswered this long is given up on - a dropped connection's
/// last pings, or a room that never answers them.
pub const PING_TIMEOUT_MS: i64 = 5_000;

/// One answered probe.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sample {
    /// The round trip, milliseconds.
    rtt: f64,
    /// Server minus local, milliseconds, by this probe's midpoint.
    offset: f64,
}

/// The measured link: round trip and server time.
#[derive(Clone, Debug, Default)]
pub struct RttClock {
    samples: VecDeque<Sample>,
    /// The last `Pong`'s server stamp as it came off the wire, and that
    /// stamp unwrapped: the server's clock is a `u32` of milliseconds that
    /// wraps every 49.7 days, so each stamp moves the unwrapped time on by
    /// its signed distance from the one before, and every offset in the
    /// window is on one line.
    server_raw: Option<u32>,
    server_ms: f64,
    last_sent: Option<i64>,
    answered: u64,
    sent: u64,
}

/// What `RttClock` has measured, for a status line or a test.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RttReport {
    /// The window's median round trip.
    pub rtt_ms: f64,
    /// Its 95th percentile: what a bad moment on this link costs.
    pub rtt_p95_ms: f64,
    /// Its fastest: the link's floor.
    pub rtt_min_ms: f64,
    /// Server minus local, by the fastest probe.
    pub offset_ms: f64,
    /// Probes answered in the window.
    pub samples: usize,
    /// Probes sent and answered over the connection's life.
    pub sent: u64,
    pub answered: u64,
}

impl RttClock {
    /// Whether a probe is due at local time `now_ms`; if so it is
    /// counted as sent and `now_ms` is what it carries.
    pub fn due(&mut self, now_ms: i64) -> bool {
        if self.last_sent.is_some_and(|at| now_ms - at < PING_EVERY_MS) {
            return false;
        }
        self.last_sent = Some(now_ms);
        self.sent += 1;
        true
    }

    /// The `Pong` for a probe stamped `client_ms` arrived at local time
    /// `now_ms`, with the server's clock reading `server_ms` when it
    /// answered. A stamp from the future or older than the timeout is
    /// not an answer to anything this clock sent and is ignored.
    pub fn observe(&mut self, client_ms: u32, server_ms: u32, now_ms: i64) {
        let server_ms = self.unwrap_server(server_ms);
        // The stamp is the low 32 bits of the local clock, which wraps
        // after 49 days; unwrap it against now.
        let sent = now_ms - (now_ms as u32).wrapping_sub(client_ms) as i64;
        let rtt = (now_ms - sent) as f64;
        if !(0.0..=PING_TIMEOUT_MS as f64).contains(&rtt) {
            return;
        }
        let offset = server_ms + rtt / 2.0 - now_ms as f64;
        self.samples.push_back(Sample { rtt, offset });
        while self.samples.len() > PING_WINDOW {
            self.samples.pop_front();
        }
        self.answered += 1;
    }

    /// A `Pong`'s server stamp on the unwrapped line: the stamp before it
    /// moved on by the signed distance between the two. The first is
    /// taken as it is. Every answer is a reading of the one server clock,
    /// so this runs whether or not the probe it answers is kept.
    fn unwrap_server(&mut self, raw: u32) -> f64 {
        self.server_ms = match self.server_raw {
            Some(last) => self.server_ms + raw.wrapping_sub(last) as i32 as f64,
            None => raw as f64,
        };
        self.server_raw = Some(raw);
        self.server_ms
    }

    /// Server minus local by the window's fastest probe; `None` until one
    /// has been answered.
    pub fn offset_ms(&self) -> Option<f64> {
        self.samples.iter().min_by(|a, b| a.rtt.total_cmp(&b.rtt)).map(|s| s.offset)
    }

    /// What the server's clock reads at local time `local_ms`.
    pub fn server_now(&self, local_ms: i64) -> Option<f64> {
        self.offset_ms().map(|offset| local_ms as f64 + offset)
    }

    /// The window's readings; `None` until a probe has been answered.
    pub fn report(&self) -> Option<RttReport> {
        let offset_ms = self.offset_ms()?;
        let mut rtts: Vec<f64> = self.samples.iter().map(|s| s.rtt).collect();
        rtts.sort_by(f64::total_cmp);
        let at = |q: f64| rtts[((rtts.len() - 1) as f64 * q).round() as usize];
        Some(RttReport {
            rtt_ms: at(0.5),
            rtt_p95_ms: at(0.95),
            rtt_min_ms: rtts[0],
            offset_ms,
            samples: rtts.len(),
            sent: self.sent,
            answered: self.answered,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_go_out_on_a_cadence_and_nothing_is_known_before_an_answer() {
        let mut clock = RttClock::default();
        assert!(clock.due(0));
        assert!(!clock.due(PING_EVERY_MS - 1));
        assert!(clock.due(PING_EVERY_MS));
        assert_eq!(clock.report(), None);
        assert_eq!(clock.server_now(0), None);
    }

    /// A symmetric 40 ms trip against a server 1000 ms ahead: the round
    /// trip is 40, and the midpoint lands the offset on 1000 exactly.
    #[test]
    fn a_round_trip_is_measured_on_one_clock_and_the_midpoint_is_the_servers_time() {
        let mut clock = RttClock::default();
        // Sent at local 100; the server read 1000 + 120 = 1120 on
        // arrival 20 ms later; back at local 140.
        clock.observe(100, 1_120, 140);
        let report = clock.report().expect("an answer");
        assert_eq!(report.rtt_ms, 40.0);
        assert_eq!(report.offset_ms, 1_000.0);
        assert_eq!(clock.server_now(500), Some(1_500.0));
    }

    /// The fastest probe is the one to believe: a slow one spent time
    /// queued somewhere, which biases its midpoint; the median is still
    /// the round trip a player feels.
    #[test]
    fn the_offset_is_the_fastest_probes_and_the_rtt_is_the_windows_median() {
        let mut clock = RttClock::default();
        // Three probes: 40 ms each way, then one delayed 100 ms on the
        // way back (queued behind a snapshot burst), then 40 again.
        clock.observe(0, 1_040, 80);
        clock.observe(250, 1_290, 430);
        clock.observe(500, 1_540, 580);
        let report = clock.report().expect("answers");
        assert_eq!(report.rtt_min_ms, 80.0);
        assert_eq!(report.rtt_ms, 80.0, "median of 80, 180, 80");
        assert_eq!(report.rtt_p95_ms, 180.0);
        assert_eq!(report.offset_ms, 1_000.0, "the slow probe's midpoint would have said 950");
    }

    /// The server's clock wraps at `u32::MAX` between two probes: the
    /// answers after the wrap continue the line of the ones before it, so
    /// the fastest probe's offset - here one after the wrap - is the same
    /// offset the probes before it measured, not four billion off.
    #[test]
    fn the_servers_stamp_is_unwrapped_across_its_u32_wrap() {
        let mut clock = RttClock::default();
        // The server reads `u32::MAX - 100` more than local, on every probe.
        let ahead = u32::MAX as i64 - 100;
        let server_at = |local: i64| ((local + ahead) % (u32::MAX as i64 + 1)) as u32;
        // 40 ms round trips before the wrap, then one of 30 ms after it.
        clock.observe(0, server_at(20), 40);
        clock.observe(50, server_at(70), 90);
        assert!(server_at(515) < 1_000, "the server's clock wrapped between the probes");
        clock.observe(500, server_at(515), 530);
        let report = clock.report().expect("answers");
        assert_eq!(report.rtt_min_ms, 30.0);
        assert_eq!(report.offset_ms, ahead as f64, "the probe after the wrap is on the line of the ones before");
        assert_eq!(clock.server_now(1_000), Some((1_000 + ahead) as f64));
    }

    #[test]
    fn a_stamp_it_never_sent_is_ignored() {
        let mut clock = RttClock::default();
        clock.observe(10_000, 1_000, 100); // a stamp from the future
        clock.observe(0, 1_000, PING_TIMEOUT_MS + 1); // older than the timeout
        assert_eq!(clock.report(), None);
    }
}
