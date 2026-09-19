//! A seat's mailbox (docs/online-coop-prd.md §4.1): the newest `IntentMsg`
//! the connection dropped in, sampled by every tick. A tick that finds no
//! newer intent repeats the last one, so a hiccup coasts rather than
//! stops - for at most `INTENT_COAST` - and a seat with nobody connected
//! reads as no input (`clear`).

use std::sync::Mutex;
use std::time::Duration;

use bongbong::net::wire::IntentMsg;
use tokio::time::Instant;

/// How long a tick keeps repeating a seat's last intent with nothing
/// newer arrived. The client sends every tick (16.7 ms), so half a second
/// covers a hiccup and stops a tank whose client went silent.
pub const INTENT_COAST: Duration = Duration::from_millis(500);

/// The newest intent posted and when.
#[derive(Default)]
pub struct Mailbox {
    slot: Mutex<Option<(IntentMsg, Instant)>>,
}

impl Mailbox {
    pub fn new() -> Mailbox {
        Mailbox::default()
    }

    /// Replace the newest intent; the tick never sees an older one.
    pub fn post(&self, msg: IntentMsg, now: Instant) {
        *self.slot.lock().expect("mailbox poisoned") = Some((msg, now));
    }

    /// The intent this tick samples: the newest one, repeated while it is
    /// younger than `INTENT_COAST`; `None` when nothing was posted, the
    /// last one has coasted out, or the mailbox was cleared.
    pub fn read(&self, now: Instant) -> Option<IntentMsg> {
        let slot = self.slot.lock().expect("mailbox poisoned");
        let (msg, posted) = (*slot)?;
        (now.saturating_duration_since(posted) <= INTENT_COAST).then_some(msg)
    }

    /// The tick of the newest intent, whether or not it still coasts: the
    /// snapshot's `acked` entry for the seat. 0 with nothing posted.
    pub fn last_tick(&self) -> u32 {
        self.slot.lock().expect("mailbox poisoned").map_or(0, |(m, _)| m.tick)
    }

    /// Forget the intent: the seat reads as no input from the next tick.
    pub fn clear(&self) {
        *self.slot.lock().expect("mailbox poisoned") = None;
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
        tokio::time::advance(INTENT_COAST).await;
        assert_eq!(mailbox.read(Instant::now()), None, "past the coast the seat stops");
        assert_eq!(mailbox.last_tick(), 5, "the ack still names the newest intent");
    }

    #[tokio::test(start_paused = true)]
    async fn the_newest_intent_wins_and_clear_reads_as_no_input() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        mailbox.post(intent(1, 1), now);
        mailbox.post(intent(2, 4), now);
        assert_eq!(mailbox.read(now), Some(intent(2, 4)));
        assert_eq!(mailbox.last_tick(), 2);
        mailbox.clear();
        assert_eq!(mailbox.read(now), None);
        assert_eq!(mailbox.last_tick(), 0);
    }
}
