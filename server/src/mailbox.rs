//! A seat's jitter buffer (docs/online-coop-prd.md §4.1, §4.12): the
//! intents a connection has dropped in, held by the client's own tick and
//! applied **one per tick, in order**.
//!
//! The order is what stage 2 needs. A client predicts its own hull by
//! replaying the inputs the server has not acknowledged yet, so the two
//! sides have to apply the same inputs in the same sequence or the replay
//! lands somewhere the server never was. Holding only the newest - which
//! is all stage 1 needed - drops an input whenever two packets arrive
//! inside one tick, and jitter guarantees that happens: the client
//! replays an input the server threw away, and the difference shows up as
//! a correction that looks like network error but is not.
//!
//! A tick that finds the buffer empty repeats the last intent and counts
//! a starvation, so a hiccup coasts rather than stops - for at most
//! `INTENT_COAST` - and a seat with nobody connected reads as no input
//! (`clear`).
//!
//! **The buffer never holds an intent back.** Each tick takes the oldest
//! one waiting, so nothing here adds latency; depth is the client's to
//! manage by how far ahead it stamps (§4.12's adaptive lead, which reads
//! `starvations` to decide). What the buffer buys is that a burst is
//! spread over the ticks that follow instead of being thrown away.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use bongbong::net::wire::IntentMsg;
use tokio::time::Instant;

/// How long a tick keeps repeating a seat's last intent with nothing
/// newer arrived. The client sends every tick (16.7 ms), so half a second
/// covers a hiccup and stops a tank whose client went silent.
pub const INTENT_COAST: Duration = Duration::from_millis(500);

/// The most intents held for one seat.
///
/// Small on purpose. The buffer exists to survive jitter, not to let a
/// client run arbitrarily far ahead: past this the oldest waiting intents
/// are dropped, which bounds both the memory one seat can cost and how
/// stale an applied input can be. Eight ticks is 133 ms of slack, well
/// past the jitter any playable link has.
pub const BUFFER_MAX: usize = 8;

#[derive(Default)]
struct Inner {
    /// Waiting to be applied, keyed by the client's tick. A `BTreeMap`
    /// rather than a queue because packets arrive out of order and a
    /// duplicate has to collapse onto the one already held.
    queue: BTreeMap<u32, IntentMsg>,
    /// The last intent applied, repeated on a starved tick.
    last: Option<IntentMsg>,
    /// The tick of the last intent applied - what the snapshot reports as
    /// `acked`, and what a client measures its replay from. `None` until
    /// one has been applied, which is not the same as tick 0: the
    /// client's first intent carries tick 0.
    applied: Option<u32>,
    /// When the newest packet arrived, for `INTENT_COAST`.
    posted: Option<Instant>,
    /// Ticks that found the buffer empty (§4.12 asks for this per seat).
    starvations: u64,
}

/// One seat's intents, ordered.
#[derive(Default)]
pub struct Mailbox {
    inner: Mutex<Inner>,
}

impl Mailbox {
    pub fn new() -> Mailbox {
        Mailbox::default()
    }

    /// Take an intent off the wire.
    ///
    /// An intent for a tick already applied is dropped - it is a
    /// duplicate or a straggler, and applying it would move the hull
    /// backwards - but it still counts as the seat being heard from, so a
    /// client whose packets are all late does not read as silent.
    pub fn post(&self, msg: IntentMsg, now: Instant) {
        let mut inner = self.inner.lock().expect("mailbox poisoned");
        inner.posted = Some(now);
        if inner.applied.is_some_and(|applied| msg.tick <= applied) {
            return;
        }
        inner.queue.insert(msg.tick, msg);
        // Past the cap the oldest go: a client that has run far ahead is
        // better served by the freshest inputs than by working through a
        // backlog the round has left behind.
        while inner.queue.len() > BUFFER_MAX {
            let Some(&oldest) = inner.queue.keys().next() else { break };
            inner.queue.remove(&oldest);
        }
    }

    /// The intent this tick applies: the oldest one waiting, or the last
    /// one again while it is younger than `INTENT_COAST`. `None` when
    /// nothing was ever posted, the seat has coasted out, or the mailbox
    /// was cleared.
    pub fn read(&self, now: Instant) -> Option<IntentMsg> {
        let mut inner = self.inner.lock().expect("mailbox poisoned");
        let posted = inner.posted?;
        if now.saturating_duration_since(posted) > INTENT_COAST {
            return None;
        }
        let next = inner.queue.keys().next().copied();
        match next {
            Some(tick) => {
                let msg = inner.queue.remove(&tick).expect("just looked it up");
                inner.applied = Some(tick);
                inner.last = Some(msg);
                Some(msg)
            }
            None => {
                inner.starvations += 1;
                inner.last
            }
        }
    }

    /// The tick of the last intent **applied**, which is what a snapshot
    /// reports as this seat's `acked`.
    ///
    /// Deliberately not the newest intent *received*: a client measures
    /// its replay from here, and naming an input still sitting in the
    /// buffer would have it drop inputs the server has not run yet.
    pub fn acked_tick(&self) -> u32 {
        self.inner.lock().expect("mailbox poisoned").applied.unwrap_or(0)
    }

    /// How many ticks have found this seat's buffer empty. A client
    /// widens its lead on these (§4.12) and `/metrics` reports them.
    pub fn starvations(&self) -> u64 {
        self.inner.lock().expect("mailbox poisoned").starvations
    }

    /// How many intents are waiting. Deep means the client is stamping
    /// further ahead than it needs to.
    pub fn depth(&self) -> usize {
        self.inner.lock().expect("mailbox poisoned").queue.len()
    }

    /// Forget everything: the seat reads as no input from the next tick.
    pub fn clear(&self) {
        *self.inner.lock().expect("mailbox poisoned") = Inner::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(tick: u32, move_dir: u8) -> IntentMsg {
        IntentMsg { tick, move_dir, face: 0, fire: false }
    }

    #[tokio::test(start_paused = true)]
    async fn a_tick_without_a_new_intent_repeats_the_last_one_then_coasts_out() {
        let mailbox = Mailbox::new();
        let t0 = Instant::now();
        assert_eq!(mailbox.read(t0), None, "nothing posted reads as no input");
        mailbox.post(intent(5, 1), t0);
        assert_eq!(mailbox.read(t0), Some(intent(5, 1)));
        tokio::time::advance(Duration::from_millis(100)).await;
        assert_eq!(mailbox.read(Instant::now()), Some(intent(5, 1)), "a missed tick repeats the last intent");
        assert_eq!(mailbox.starvations(), 1, "and counts as a starvation");
        tokio::time::advance(INTENT_COAST).await;
        assert_eq!(mailbox.read(Instant::now()), None, "past the coast the seat stops");
        assert_eq!(mailbox.acked_tick(), 5, "the ack names the intent that was applied");
    }

    /// **The reason this is a buffer.** Two packets inside one tick is
    /// what jitter does, and both have to be applied in order - a client
    /// replaying its own inputs against the server's answer diverges by
    /// exactly the one that was thrown away.
    #[tokio::test(start_paused = true)]
    async fn a_burst_is_applied_in_order_rather_than_collapsed() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        mailbox.post(intent(1, 1), now);
        mailbox.post(intent(2, 4), now);
        mailbox.post(intent(3, 2), now);
        assert_eq!(mailbox.depth(), 3);
        assert_eq!(mailbox.read(now), Some(intent(1, 1)), "oldest first");
        assert_eq!(mailbox.acked_tick(), 1, "the ack names what was applied, not what arrived");
        assert_eq!(mailbox.read(now), Some(intent(2, 4)));
        assert_eq!(mailbox.read(now), Some(intent(3, 2)));
        assert_eq!(mailbox.acked_tick(), 3);
        assert_eq!(mailbox.starvations(), 0, "a full buffer never starves");
    }

    /// Packets arrive out of order and get retransmitted; neither may
    /// move the hull backwards.
    #[tokio::test(start_paused = true)]
    async fn out_of_order_is_sorted_and_a_straggler_is_dropped() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        mailbox.post(intent(3, 2), now);
        mailbox.post(intent(1, 1), now);
        mailbox.post(intent(1, 1), now); // a duplicate collapses
        assert_eq!(mailbox.depth(), 2);
        assert_eq!(mailbox.read(now), Some(intent(1, 1)));
        assert_eq!(mailbox.read(now), Some(intent(3, 2)));
        // Tick 2 finally turns up, after 3 has been applied.
        mailbox.post(intent(2, 4), now);
        assert_eq!(mailbox.depth(), 0, "a straggler for an applied tick is dropped");
        assert_eq!(mailbox.read(now), Some(intent(3, 2)), "and the last applied one repeats");
        assert_eq!(mailbox.acked_tick(), 3, "the ack never goes backwards");
    }

    /// A client that runs far ahead is held to `BUFFER_MAX`, so one seat
    /// cannot grow without bound or drag the round through a backlog.
    #[tokio::test(start_paused = true)]
    async fn a_runaway_client_is_capped_at_the_freshest_intents() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        for tick in 0..(BUFFER_MAX as u32 + 4) {
            mailbox.post(intent(tick, 1), now);
        }
        assert_eq!(mailbox.depth(), BUFFER_MAX);
        // The four oldest went; the next applied is the fifth.
        assert_eq!(mailbox.read(now), Some(intent(4, 1)));
    }

    #[tokio::test(start_paused = true)]
    async fn clear_reads_as_no_input_and_forgets_the_ack() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        mailbox.post(intent(2, 4), now);
        assert_eq!(mailbox.read(now), Some(intent(2, 4)));
        assert_eq!(mailbox.acked_tick(), 2);
        mailbox.clear();
        assert_eq!(mailbox.read(now), None);
        assert_eq!(mailbox.acked_tick(), 0);
        assert_eq!(mailbox.depth(), 0);
    }

    /// Tick 0 is a real tick - the client's counter starts there - so it
    /// must not read as "nothing applied yet".
    #[tokio::test(start_paused = true)]
    async fn tick_zero_is_an_intent_like_any_other() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        mailbox.post(intent(0, 1), now);
        assert_eq!(mailbox.read(now), Some(intent(0, 1)));
        assert_eq!(mailbox.acked_tick(), 0);
        // And a second intent for tick 0 is now a straggler.
        mailbox.post(intent(0, 4), now);
        assert_eq!(mailbox.depth(), 0);
    }
}
