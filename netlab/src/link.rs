//! The impairment model: when a chunk that arrived at the proxy is
//! delivered on the other side, as TCP would deliver it.
//!
//! TCP is a byte stream, so the model works on the chunks the proxy reads
//! rather than on packets, and it never reorders: a chunk is delivered at
//! `max(previous delivery, arrival + delay + U[0, jitter])`, so jitter
//! shows as bunching, never as bytes overtaking each other. A lost segment
//! is retransmitted a retransmit timeout later and the receiver hands
//! nothing past the hole to the application until it arrives - so a lost
//! chunk is delivered at `arrival + delay + rto`, and through the `max`
//! everything behind it waits too. That head-of-line hold is the one link
//! effect the client cannot hide, and the reason the model exists.
//!
//! Pure arithmetic on milliseconds, so the tests pin it without a socket.

/// A link's dials, one direction's worth (the proxy runs one `Schedule`
/// per direction with the same dials).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Impairment {
    /// One-way delay, milliseconds.
    pub delay_ms: f64,
    /// Extra delay drawn uniformly from `[0, jitter_ms]` per chunk.
    pub jitter_ms: f64,
    /// The chance a chunk is lost and has to be retransmitted.
    pub loss: f64,
    /// How long a lost chunk waits for its retransmit, past the delay.
    pub rto_ms: f64,
    /// Nagle's algorithm on the sender: a chunk written while earlier data
    /// is still unacknowledged is held and coalesced until that round trip
    /// completes.
    pub nagle: bool,
}

impl Impairment {
    /// No impairment at all: the loopback link.
    pub const NONE: Impairment = Impairment { delay_ms: 0.0, jitter_ms: 0.0, loss: 0.0, rto_ms: 200.0, nagle: false };

    /// The retransmit timeout a real stack would pick for this delay:
    /// Linux's floor of 200 ms, or two round trips' worth past it.
    pub fn default_rto_ms(delay_ms: f64) -> f64 {
        RTO_FLOOR_MS.max(2.0 * delay_ms)
    }

    /// One round trip of this link.
    pub fn rtt_ms(&self) -> f64 {
        2.0 * self.delay_ms
    }
}

/// The minimum retransmit timeout Linux uses (`TCP_RTO_MIN`).
pub const RTO_FLOOR_MS: f64 = 200.0;

/// Where a chunk goes, and why it is as late as it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Delivery {
    /// When the chunk is written out on the far side, milliseconds.
    pub at_ms: f64,
    /// It was lost and waited for its retransmit.
    pub lost: bool,
    /// Nagle held it behind an unacknowledged write.
    pub coalesced: bool,
}

/// SplitMix64: a seeded, portable stream for the jitter and loss draws,
/// so a run's link replays from its seed.
#[derive(Clone, Debug)]
pub struct SplitMix(u64);

impl SplitMix {
    pub fn new(seed: u64) -> SplitMix {
        SplitMix(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// One direction of the link: the dials, the draws and what the last
/// chunk did.
#[derive(Clone, Debug)]
pub struct Schedule {
    link: Impairment,
    rng: SplitMix,
    /// When the previous chunk was delivered; nothing is delivered before it.
    last_delivery: f64,
    /// When the sender last put a segment on the wire (Nagle's clock). In
    /// the future while a coalesced batch is still being held.
    last_send: Option<f64>,
}

impl Schedule {
    pub fn new(link: Impairment, seed: u64) -> Schedule {
        Schedule { link, rng: SplitMix::new(seed), last_delivery: f64::NEG_INFINITY, last_send: None }
    }

    /// The delivery of a chunk that arrived at `arrival_ms`. Calls come in
    /// arrival order, as the proxy reads them.
    pub fn deliver(&mut self, arrival_ms: f64) -> Delivery {
        let (send, coalesced) = self.send_time(arrival_ms);
        let lost = self.link.loss > 0.0 && self.rng.unit() < self.link.loss;
        let extra = if lost {
            self.link.rto_ms
        } else if self.link.jitter_ms > 0.0 {
            self.rng.unit() * self.link.jitter_ms
        } else {
            0.0
        };
        let at_ms = self.last_delivery.max(send + self.link.delay_ms + extra);
        self.last_delivery = at_ms;
        Delivery { at_ms, lost, coalesced }
    }

    /// When the sender actually puts this chunk on the wire: at once, or -
    /// under Nagle, with a write still unacknowledged - with the batch
    /// that goes when the round trip completes.
    fn send_time(&mut self, arrival_ms: f64) -> (f64, bool) {
        if !self.link.nagle {
            return (arrival_ms, false);
        }
        let rtt = self.link.rtt_ms();
        match self.last_send {
            // A batch is still being held: this chunk joins it.
            Some(last) if arrival_ms < last => (last, true),
            // The last segment is still in flight: hold until its ack.
            Some(last) if arrival_ms < last + rtt => {
                let send = last + rtt;
                self.last_send = Some(send);
                (send, true)
            }
            _ => {
                self.last_send = Some(arrival_ms);
                (arrival_ms, false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(delay: f64, jitter: f64, loss: f64) -> Impairment {
        Impairment { delay_ms: delay, jitter_ms: jitter, loss, rto_ms: Impairment::default_rto_ms(delay), nagle: false }
    }

    #[test]
    fn a_clean_link_delivers_each_chunk_one_delay_later() {
        let mut s = Schedule::new(link(40.0, 0.0, 0.0), 1);
        for t in [0.0, 5.0, 16.7, 100.0] {
            let d = s.deliver(t);
            assert_eq!(d.at_ms, t + 40.0);
            assert!(!d.lost && !d.coalesced);
        }
    }

    #[test]
    fn jitter_never_reorders_delivery() {
        let mut s = Schedule::new(link(40.0, 30.0, 0.0), 7);
        let mut last = f64::NEG_INFINITY;
        let mut bunched = 0;
        for i in 0..5000 {
            let arrival = i as f64 * 2.0;
            let d = s.deliver(arrival);
            assert!(d.at_ms >= last, "delivery went backwards at chunk {i}");
            assert!(d.at_ms >= arrival + 40.0, "nothing arrives before the delay");
            if d.at_ms == last {
                bunched += 1;
            }
            last = d.at_ms;
        }
        assert!(bunched > 0, "a jitter wider than the gap between chunks bunches them");
    }

    #[test]
    fn a_lost_chunk_holds_everything_behind_it() {
        // Loss 1.0 for the first draw only: build the schedule, lose one
        // chunk by hand through a link that always loses, then compare.
        let mut s = Schedule::new(link(20.0, 0.0, 1.0), 3);
        let lost = s.deliver(0.0);
        assert!(lost.lost);
        assert_eq!(lost.at_ms, 20.0 + RTO_FLOOR_MS);
        // Switch the link clean for what follows, keeping the schedule's
        // memory of the hold.
        s.link.loss = 0.0;
        for t in [10.0, 50.0, 150.0] {
            let d = s.deliver(t);
            assert_eq!(d.at_ms, lost.at_ms, "chunk at {t} waits behind the retransmit");
        }
        let after = s.deliver(300.0);
        assert_eq!(after.at_ms, 320.0, "the hold is over once the retransmit is through");
    }

    #[test]
    fn the_loss_rate_is_what_was_dialled() {
        let mut s = Schedule::new(link(10.0, 0.0, 0.03), 11);
        let lost = (0..20_000).filter(|i| s.deliver(*i as f64 * 1000.0).lost).count();
        let rate = lost as f64 / 20_000.0;
        assert!((rate - 0.03).abs() < 0.005, "rate {rate}");
    }

    #[test]
    fn the_retransmit_timeout_has_a_floor_and_scales_with_delay() {
        assert_eq!(Impairment::default_rto_ms(15.0), 200.0);
        assert_eq!(Impairment::default_rto_ms(150.0), 300.0);
    }

    #[test]
    fn nagle_coalesces_writes_behind_an_unacknowledged_one() {
        let mut s = Schedule::new(Impairment { nagle: true, ..link(30.0, 0.0, 0.0) }, 5);
        let first = s.deliver(0.0);
        assert_eq!(first.at_ms, 30.0);
        assert!(!first.coalesced);
        // Written 16 ms later, while the first is unacknowledged (rtt 60):
        // held until 60, delivered at 90.
        let second = s.deliver(16.0);
        assert!(second.coalesced);
        assert_eq!(second.at_ms, 90.0);
        // Another write while that batch is still held joins it.
        let third = s.deliver(33.0);
        assert!(third.coalesced);
        assert_eq!(third.at_ms, 90.0);
        // Long after, the pipe is idle again.
        let idle = s.deliver(500.0);
        assert!(!idle.coalesced);
        assert_eq!(idle.at_ms, 530.0);
    }

    #[test]
    fn the_same_seed_replays_the_same_link() {
        let run = |seed| {
            let mut s = Schedule::new(link(60.0, 25.0, 0.01), seed);
            (0..500).map(|i| s.deliver(i as f64 * 16.0).at_ms).collect::<Vec<_>>()
        };
        assert_eq!(run(9), run(9));
        assert_ne!(run(9), run(10));
    }
}
