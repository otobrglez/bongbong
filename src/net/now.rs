//! Where the client's half of a link reads the time: the wall, or a clock
//! a test winds by hand.
//!
//! An `OnlineRound` places render time on its own milliseconds, the
//! `loopback` link stamps each arrival with the instant it became
//! readable, and a `RoomClient` times its pings and its dial against the
//! same instants. On the wall all three are `Instant::now()`, so a test
//! that drives them in real time measures the machine as much as the
//! code: a loaded run sleeps longer, encodes slower and spreads the
//! arrivals out, and the interpolator sizes its delay from that spread.
//! A `Now::manual()` clock handed to all three moves only when the test
//! advances it, so render time, every arrival stamp and every timeout
//! read the same instants on every run.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A source of "now": the wall unless a test has made it a manual one.
/// Clones share one clock, so the round, its client and both ends of a
/// loopback link advance together.
#[derive(Clone, Debug, Default)]
pub struct Now {
    /// The hand-wound instant, `None` on the wall.
    manual: Option<Arc<Mutex<Instant>>>,
}

impl Now {
    /// The wall clock: `Instant::now()`, what every build that plays reads.
    pub fn wall() -> Now {
        Now::default()
    }

    /// A clock that stands at the moment it is made until `advance` moves
    /// it.
    pub fn manual() -> Now {
        Now { manual: Some(Arc::new(Mutex::new(Instant::now()))) }
    }

    /// The instant it is.
    pub fn now(&self) -> Instant {
        match &self.manual {
            Some(at) => *at.lock().expect("nothing panics holding the clock"),
            None => Instant::now(),
        }
    }

    /// How long it has been since `then`, zero for an instant still ahead.
    pub fn since(&self, then: Instant) -> Duration {
        self.now().saturating_duration_since(then)
    }

    /// Move a manual clock on by `by`. The wall moves on its own, so this
    /// does nothing to it.
    pub fn advance(&self, by: Duration) {
        if let Some(at) = &self.manual {
            *at.lock().expect("nothing panics holding the clock") += by;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manual_clock_moves_only_when_advanced_and_its_clones_with_it() {
        let clock = Now::manual();
        let other = clock.clone();
        let start = clock.now();
        std::thread::sleep(Duration::from_millis(2));
        assert_eq!(clock.now(), start, "it stood while the wall moved");
        other.advance(Duration::from_millis(16));
        assert_eq!(clock.since(start), Duration::from_millis(16), "a clone's advance moves it");
        assert_eq!(clock.since(start + Duration::from_secs(1)), Duration::ZERO, "an instant ahead is no time since");
    }

    #[test]
    fn the_wall_clock_is_the_wall() {
        let wall = Now::wall();
        let before = Instant::now();
        wall.advance(Duration::from_secs(60));
        let read = wall.now();
        assert!(read >= before && read <= Instant::now(), "advance moved the wall");
    }
}
