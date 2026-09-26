//! A seat's jitter buffer (docs/online-coop-prd.md §4.1, §4.12): the
//! intents a connection has dropped in, held by the client's own tick and
//! applied **one per tick, in order**. One type for every room there is -
//! the room server's (`bongbong_server::room`), the rig's thread and
//! `rig::Lockstep` - so a client's lead is steered by the same reading
//! whichever it is playing against.
//!
//! The order is what stage 2 needs. A client predicts its own hull by
//! replaying the inputs the server has not acknowledged yet, so the two
//! sides have to apply the same inputs in the same sequence or the replay
//! lands somewhere the server never was. Holding only the newest drops an
//! input whenever two packets arrive inside one tick, and jitter
//! guarantees that happens: the client replays an input the server threw
//! away, and the difference shows up as a correction that looks like
//! network error but is not.
//!
//! A tick that finds the buffer empty repeats the last intent and counts
//! a starvation, so a hiccup coasts rather than stops - for at most
//! `INTENT_COAST` - and a seat with nobody connected reads as no input
//! (`clear`).
//!
//! **The buffer never holds an intent back.** Each tick takes the oldest
//! one waiting, so nothing here adds latency; depth is the client's to
//! manage. `wire_state` is how it finds out: the depth left after the
//! tick's read and whether that read starved travel in
//! `Snapshot::mailbox`, and `net::round` adds a packet on a starvation and
//! skips one when the depth sits high (§4.12, "The lead"). What the buffer
//! buys is that a burst is spread over the ticks that follow instead of
//! being thrown away.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::net::wire::IntentMsg;

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
/// past the jitter any playable link has, and past the depth the client's
/// lead ever asks for.
pub const BUFFER_MAX: usize = 8;

/// The bit of `wire_state` that says the last read found nothing waiting.
pub const STARVED_BIT: u8 = 0x80;

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
    /// Whether the most recent read found the buffer empty: the bit the
    /// snapshot carries to the client.
    last_starved: bool,
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
                inner.last_starved = false;
                Some(msg)
            }
            None => {
                inner.starvations += 1;
                inner.last_starved = true;
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

    /// How many ticks have found this seat's buffer empty. `/metrics`
    /// reports the total; the client reads each one as it happens off
    /// `wire_state`.
    pub fn starvations(&self) -> u64 {
        self.inner.lock().expect("mailbox poisoned").starvations
    }

    /// How many intents are waiting. Deep means the client is stamping
    /// further ahead than it needs to.
    pub fn depth(&self) -> usize {
        self.inner.lock().expect("mailbox poisoned").queue.len()
    }

    /// The reading `Snapshot::mailbox` carries for this seat: the depth
    /// left after the tick's read in the low seven bits and `STARVED_BIT`
    /// if that read found nothing. Cut after the read, so a client sees
    /// what the tick it is being told about actually found.
    pub fn wire_state(&self) -> u8 {
        let inner = self.inner.lock().expect("mailbox poisoned");
        let depth = inner.queue.len().min((STARVED_BIT - 1) as usize) as u8;
        if inner.last_starved { depth | STARVED_BIT } else { depth }
    }

    /// Forget everything: the seat reads as no input from the next tick.
    pub fn clear(&self) {
        *self.inner.lock().expect("mailbox poisoned") = Inner::default();
    }
}

/// The two halves of a `wire_state` byte.
pub fn unpack(state: u8) -> (u8, bool) {
    (state & !STARVED_BIT, state & STARVED_BIT != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(tick: u32, move_dir: u8) -> IntentMsg {
        IntentMsg { tick, move_dir, face: 0, fire: false }
    }

    #[test]
    fn a_tick_without_a_new_intent_repeats_the_last_one_then_coasts_out() {
        let mailbox = Mailbox::new();
        let t0 = Instant::now();
        assert_eq!(mailbox.read(t0), None, "nothing posted reads as no input");
        mailbox.post(intent(5, 1), t0);
        assert_eq!(mailbox.read(t0), Some(intent(5, 1)));
        assert_eq!(unpack(mailbox.wire_state()), (0, false), "one in, one out, nothing left and nothing starved");
        let later = t0 + Duration::from_millis(100);
        assert_eq!(mailbox.read(later), Some(intent(5, 1)), "a missed tick repeats the last intent");
        assert_eq!(mailbox.starvations(), 1, "and counts as a starvation");
        assert_eq!(unpack(mailbox.wire_state()), (0, true), "which the wire says");
        assert_eq!(mailbox.read(later + INTENT_COAST + Duration::from_millis(1)), None, "past the coast the seat stops");
        assert_eq!(mailbox.acked_tick(), 5, "the ack names the intent that was applied");
    }

    /// **The reason this is a buffer.** Two packets inside one tick is
    /// what jitter does, and both have to be applied in order - a client
    /// replaying its own inputs against the server's answer diverges by
    /// exactly the one that was thrown away.
    #[test]
    fn a_burst_is_applied_in_order_rather_than_collapsed() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        mailbox.post(intent(1, 1), now);
        mailbox.post(intent(2, 4), now);
        mailbox.post(intent(3, 2), now);
        assert_eq!(mailbox.depth(), 3);
        assert_eq!(mailbox.read(now), Some(intent(1, 1)), "oldest first");
        assert_eq!(mailbox.acked_tick(), 1, "the ack names what was applied, not what arrived");
        assert_eq!(unpack(mailbox.wire_state()), (2, false), "two still waiting after the read");
        assert_eq!(mailbox.read(now), Some(intent(2, 4)));
        assert_eq!(mailbox.read(now), Some(intent(3, 2)));
        assert_eq!(mailbox.acked_tick(), 3);
        assert_eq!(mailbox.starvations(), 0, "a full buffer never starves");
    }

    /// Packets arrive out of order and get retransmitted; neither may
    /// move the hull backwards.
    #[test]
    fn out_of_order_is_sorted_and_a_straggler_is_dropped() {
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
    #[test]
    fn a_runaway_client_is_capped_at_the_freshest_intents() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        for tick in 0..(BUFFER_MAX as u32 + 4) {
            mailbox.post(intent(tick, 1), now);
        }
        assert_eq!(mailbox.depth(), BUFFER_MAX);
        // The four oldest went; the next applied is the fifth.
        assert_eq!(mailbox.read(now), Some(intent(4, 1)));
        assert_eq!(unpack(mailbox.wire_state()), (BUFFER_MAX as u8 - 1, false));
    }

    #[test]
    fn clear_reads_as_no_input_and_forgets_the_ack() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        mailbox.post(intent(2, 4), now);
        assert_eq!(mailbox.read(now), Some(intent(2, 4)));
        assert_eq!(mailbox.acked_tick(), 2);
        mailbox.clear();
        assert_eq!(mailbox.read(now), None);
        assert_eq!(mailbox.acked_tick(), 0);
        assert_eq!(mailbox.depth(), 0);
        assert_eq!(mailbox.wire_state(), 0);
    }

    /// Tick 0 is a real tick - the client's counter starts there - so it
    /// must not read as "nothing applied yet".
    #[test]
    fn tick_zero_is_an_intent_like_any_other() {
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
