//! An in-process link between two `Transport` ends, with delay, jitter
//! and loss on a dial (docs/online-coop-prd.md §4.6, §4.13): the offline
//! rig's wire. An authoritative `Game` runs on a thread with one end and
//! the replica in the window holds the other, so the whole client half
//! of online play - the encoder, the delta chain, the replica, the feel
//! at 80 ms and 2 % loss - is exercised with no server, no socket and no
//! port.
//!
//! What the link models is a WebSocket, not a datagram: delivery is in
//! order and head-of-line, so a parcel held back by jitter holds back
//! everything behind it. Loss is the one thing TCP would not do, and it
//! is here because the rig's job is to make the replica's tolerance
//! visible - the PRD's `--loss 0.02`.
//!
//! Every roll comes from a seeded `SmallRng`, one per direction, so a
//! rig run replays: the same seed, the same link, the same parcels
//! arriving on the same frames. `rand::rng()` is never called here.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rand::rngs::SmallRng;
use rand::{RngExt, SeedableRng};

use crate::net::codec::{self, Msg};
use crate::net::transport::{Closed, ConnState, Transport};

/// How bad the link is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkQuality {
    /// One-way delay before a message is handed over.
    pub delay: Duration,
    /// Spread around `delay`, uniform in `-jitter ..= jitter` and never
    /// pushing an arrival before the one in front of it.
    pub jitter: Duration,
    /// The share of messages that never arrive, 0.0 to 1.0.
    pub loss: f64,
}

impl LinkQuality {
    /// A link that delivers everything, at once.
    pub const PERFECT: LinkQuality =
        LinkQuality { delay: Duration::ZERO, jitter: Duration::ZERO, loss: 0.0 };

    /// The rig's dials as the command line spells them: `--delay 80
    /// --jitter 20 --loss 0.02`.
    pub fn new(delay_ms: u64, jitter_ms: u64, loss: f64) -> LinkQuality {
        LinkQuality {
            delay: Duration::from_millis(delay_ms),
            jitter: Duration::from_millis(jitter_ms),
            loss: loss.clamp(0.0, 1.0),
        }
    }
}

impl Default for LinkQuality {
    fn default() -> LinkQuality {
        LinkQuality::PERFECT
    }
}

/// A message in flight and the instant it becomes readable.
struct Parcel {
    due: Instant,
    bytes: Vec<u8>,
}

/// One direction of the link.
#[derive(Default)]
struct Pipe {
    queue: VecDeque<Parcel>,
    /// The last arrival handed to this pipe, so a parcel never overtakes
    /// the one in front of it.
    last_due: Option<Instant>,
    /// Somebody hung up; what is already queued still arrives.
    closed: Option<Closed>,
}

/// One end of an in-process link.
///
/// Both ends are ordinary `Transport`s, so the rig's replica cannot tell
/// this from a socket. `Send`, so the authoritative half lives on its own
/// thread.
pub struct Loopback {
    /// What this end reads.
    inbox: Arc<Mutex<Pipe>>,
    /// What this end writes.
    outbox: Arc<Mutex<Pipe>>,
    quality: LinkQuality,
    rng: SmallRng,
    /// Set once the inbox has closed and run dry.
    finished: Option<Closed>,
}

/// The two ends of a fresh link, `(a, b)`: what `a` sends, `b` drains.
///
/// Each direction rolls its delay and loss from its own stream off
/// `seed`, so the two never interleave and a run replays exactly.
pub fn pair(quality: LinkQuality, seed: u64) -> (Loopback, Loopback) {
    let a_to_b: Arc<Mutex<Pipe>> = Arc::default();
    let b_to_a: Arc<Mutex<Pipe>> = Arc::default();
    let a = Loopback {
        inbox: b_to_a.clone(),
        outbox: a_to_b.clone(),
        quality,
        rng: SmallRng::seed_from_u64(seed),
        finished: None,
    };
    let b = Loopback {
        inbox: a_to_b,
        outbox: b_to_a,
        quality,
        // The other direction's stream: the odd-word mix of the golden
        // ratio, the same constant the hashes elsewhere are salted with.
        rng: SmallRng::seed_from_u64(seed ^ 0x9E37_79B9_7F4A_7C15),
        finished: None,
    };
    (a, b)
}

impl Loopback {
    /// This end's link quality; the two directions can differ.
    pub fn set_quality(&mut self, quality: LinkQuality) {
        self.quality = quality;
    }

    /// `send` at a given moment, the seam the tests drive so they need
    /// no sleeping.
    pub fn send_at(&mut self, now: Instant, bytes: &[u8]) {
        let outbox = Arc::clone(&self.outbox);
        let mut pipe = outbox.lock().expect("the link's other end did not panic");
        if pipe.closed.is_some() {
            return;
        }
        if self.quality.loss > 0.0 && self.rng.random_bool(self.quality.loss) {
            return;
        }
        let delay = self.roll_delay();
        let due = (now + delay).max(pipe.last_due.unwrap_or(now));
        pipe.last_due = Some(due);
        pipe.queue.push_back(Parcel { due, bytes: bytes.to_vec() });
    }

    /// `drain` at a given moment: everything due by `now`, in order.
    pub fn drain_at(&mut self, now: Instant, out: &mut Vec<Msg>) {
        let inbox = Arc::clone(&self.inbox);
        let mut pipe = inbox.lock().expect("the link's other end did not panic");
        while pipe.queue.front().is_some_and(|p| p.due <= now) {
            let parcel = pipe.queue.pop_front().expect("just peeked");
            if let Ok(msg) = codec::decode(&parcel.bytes) {
                out.push(msg);
            }
        }
        if pipe.queue.is_empty() {
            self.finished = pipe.closed.clone();
        }
    }

    /// The delay one parcel takes: the dialled delay, spread by the
    /// jitter and never negative.
    fn roll_delay(&mut self) -> Duration {
        let jitter = self.quality.jitter.as_nanos() as i64;
        if jitter == 0 {
            return self.quality.delay;
        }
        let offset = self.rng.random_range(-jitter..=jitter);
        let nanos = self.quality.delay.as_nanos() as i64 + offset;
        Duration::from_nanos(nanos.max(0) as u64)
    }

    /// Hang up both directions with `reason`; what is queued still
    /// arrives, which is how a room's last words reach a client.
    fn hang_up(&mut self, reason: Closed) {
        for pipe in [&self.inbox, &self.outbox] {
            let mut pipe = pipe.lock().expect("the link's other end did not panic");
            if pipe.closed.is_none() {
                pipe.closed = Some(reason.clone());
            }
        }
    }
}

impl Transport for Loopback {
    fn state(&self) -> ConnState {
        match &self.finished {
            Some(closed) => ConnState::Closed(closed.clone()),
            None => ConnState::Open,
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        self.send_at(Instant::now(), bytes);
    }

    fn drain(&mut self, out: &mut Vec<Msg>) {
        self.drain_at(Instant::now(), out);
    }

    fn close(&mut self) {
        self.hang_up(Closed::by_us());
        self.finished = Some(Closed::by_us());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::wire::IntentMsg;

    fn intent(tick: u32) -> Vec<u8> {
        codec::encode(&Msg::Intent(IntentMsg { tick, move_dir: 1, face: 1, fire: false }))
    }

    fn ticks(msgs: &[Msg]) -> Vec<u32> {
        msgs.iter()
            .map(|m| match m {
                Msg::Intent(i) => i.tick,
                other => panic!("not an intent: {other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_perfect_link_hands_everything_over_at_once_and_in_order() {
        let (mut a, mut b) = pair(LinkQuality::PERFECT, 7);
        let now = Instant::now();
        for tick in 0..4 {
            a.send_at(now, &intent(tick));
        }
        let mut out = Vec::new();
        b.drain_at(now, &mut out);
        assert_eq!(ticks(&out), [0, 1, 2, 3]);
        out.clear();
        b.drain_at(now, &mut out);
        assert!(out.is_empty(), "a drained pipe has nothing left");
        // The link is duplex and the directions are independent.
        b.send_at(now, &intent(9));
        let mut back = Vec::new();
        a.drain_at(now, &mut back);
        assert_eq!(ticks(&back), [9]);
    }

    #[test]
    fn delay_holds_a_message_back_by_exactly_the_dial() {
        let (mut a, mut b) = pair(LinkQuality::new(80, 0, 0.0), 1);
        let now = Instant::now();
        a.send_at(now, &intent(1));
        let mut out = Vec::new();
        b.drain_at(now + Duration::from_millis(79), &mut out);
        assert!(out.is_empty(), "not due yet");
        b.drain_at(now + Duration::from_millis(80), &mut out);
        assert_eq!(ticks(&out), [1]);
    }

    #[test]
    fn jitter_spreads_arrivals_within_the_dial_and_never_reorders_them() {
        let (mut a, mut b) = pair(LinkQuality::new(80, 20, 0.0), 0xB0B5);
        let now = Instant::now();
        for tick in 0..60 {
            // One send per tick of the intent stream.
            a.send_at(now + Duration::from_millis(tick as u64 * 16), &intent(tick));
        }
        let mut seen: Vec<(u32, u64)> = Vec::new();
        for ms in 0..2000u64 {
            let mut out = Vec::new();
            b.drain_at(now + Duration::from_millis(ms), &mut out);
            seen.extend(ticks(&out).into_iter().map(|t| (t, ms)));
        }
        assert_eq!(seen.len(), 60, "nothing is lost on a lossless link");
        assert_eq!(
            seen.iter().map(|(t, _)| *t).collect::<Vec<_>>(),
            (0..60).collect::<Vec<_>>(),
            "order is the link's, not the jitter's"
        );
        let lag: Vec<i64> = seen.iter().map(|(t, ms)| *ms as i64 - *t as i64 * 16).collect();
        // The one extra millisecond is the drain loop's own step: a
        // parcel due mid-millisecond is read on the next one.
        assert!(lag.iter().all(|l| (60..=101).contains(l)), "every arrival within 80 +/- 20 ms: {lag:?}");
        assert!(lag.iter().any(|&l| l < 78) && lag.iter().any(|&l| l > 82), "the jitter actually spreads: {lag:?}");
    }

    #[test]
    fn loss_drops_about_the_dialled_share_and_the_seed_replays_it() {
        let arrivals = |seed: u64| {
            let (mut a, mut b) = pair(LinkQuality::new(0, 0, 0.1), seed);
            let now = Instant::now();
            for tick in 0..1000 {
                a.send_at(now, &intent(tick));
            }
            let mut out = Vec::new();
            b.drain_at(now, &mut out);
            ticks(&out)
        };
        let first = arrivals(4242);
        assert!((850..=950).contains(&first.len()), "{} of 1000 arrived at 10 % loss", first.len());
        assert!(first.windows(2).all(|w| w[0] < w[1]), "what survives keeps its order");
        assert_eq!(first, arrivals(4242), "the same seed loses the same messages");
        assert_ne!(first, arrivals(4243), "another seed loses others");
    }

    #[test]
    fn the_two_directions_are_independent_streams() {
        // Were both ends rolling the same stream, a symmetric run would
        // lose the same message in both directions.
        let (mut a, mut b) = pair(LinkQuality::new(0, 0, 0.3), 99);
        let now = Instant::now();
        for tick in 0..200 {
            a.send_at(now, &intent(tick));
            b.send_at(now, &intent(tick));
        }
        let (mut to_b, mut to_a) = (Vec::new(), Vec::new());
        b.drain_at(now, &mut to_b);
        a.drain_at(now, &mut to_a);
        assert_ne!(ticks(&to_b), ticks(&to_a));
    }

    #[test]
    fn hanging_up_still_delivers_what_was_in_flight() {
        let (mut a, mut b) = pair(LinkQuality::new(50, 0, 0.0), 3);
        let now = Instant::now();
        a.send_at(now, &intent(1));
        a.close();
        assert_eq!(a.state(), ConnState::Closed(Closed::by_us()));
        let mut out = Vec::new();
        b.drain_at(now, &mut out);
        assert!(out.is_empty());
        assert_eq!(b.state(), ConnState::Open, "the last word has not been read yet");
        b.drain_at(now + Duration::from_millis(50), &mut out);
        assert_eq!(ticks(&out), [1], "what was in flight arrives");
        assert!(matches!(b.state(), ConnState::Closed(_)), "and then the end is the end");
        a.send_at(now, &intent(2));
        out.clear();
        b.drain_at(now + Duration::from_secs(1), &mut out);
        assert!(out.is_empty(), "a closed link takes nothing more");
    }

    #[test]
    fn a_message_that_does_not_decode_is_skipped_not_fatal() {
        let (mut a, mut b) = pair(LinkQuality::PERFECT, 0);
        let now = Instant::now();
        a.send_at(now, &[200, 1, 2, 3]);
        a.send_at(now, &intent(5));
        let mut out = Vec::new();
        b.drain_at(now, &mut out);
        assert_eq!(ticks(&out), [5]);
        assert_eq!(b.state(), ConnState::Open);
    }
}
