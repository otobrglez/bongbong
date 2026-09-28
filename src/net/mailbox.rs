//! A seat's intents between the socket and the tick (docs/online-coop-prd.md
//! §4.1, §4.12, §4.16). One type for every room there is - the room
//! server's (`bongbong_server::room`), the rig's thread and
//! `rig::Lockstep` - so a seat's input reaches the round by one rule
//! whichever it is playing against. A tick reads the mailbox once; what
//! the read takes depends on what the client is sending.
//!
//! **A server-driven seat (stage 2) is a jitter buffer**: its intents are
//! held by the client's own tick and applied **one per tick, in order**.
//! A client predicting its own hull replays the inputs the server has not
//! acknowledged yet, so the two sides have to apply the same inputs in
//! the same sequence or the replay lands somewhere the server never was;
//! holding only the newest would drop an input whenever two packets
//! arrive inside one tick, and jitter guarantees that happens. The buffer
//! never holds an intent back - each tick takes the oldest waiting - so
//! depth is the client's to manage, and `wire_state` is how it finds out.
//!
//! **A client-owned seat (stage 3 and on, `IntentMsg::owned`) is
//! newest-wins**: a tick whose newest waiting intent carries a pose takes
//! *every* intent waiting. The pose, `move_dir`, `face` and the view
//! (`view_tick`, the world the client was drawing) are the newest's - the
//! client has already driven the hull there, so the ticks in between have
//! nothing left to replay - and the ack is the newest's tick, so depth is
//! 0 after every such read and nothing an owned seat does on the server
//! runs a tick later than the packet that asked for it. An ordered queue
//! would instead keep every tick a starvation ever cost as standing
//! depth, with nothing to drain it.
//!
//! **The trigger survives the merge press for press.** Shells and plasma
//! fire on a press - the round sees the trigger go down between two
//! ticks - so what a read delivers is chosen to keep every press the
//! client made a press the round sees:
//! - the intents taken are scanned in order from the client's own last
//!   reported trigger, looking for a press (down after up);
//! - no press among them: the newest's own trigger, a hold held and a
//!   release released;
//! - a press, and the round last saw the trigger up: down, on this tick -
//!   a tap that went down and up between two ticks still fires;
//! - a press, and the round last saw it down (the release hid inside an
//!   earlier merge): up now and down on the next read, whatever that read
//!   finds, so the round sees the release and the new press.
//!
//! What is lost is a second press inside one read's intents - a release
//! and a new press between two ticks, which a client holding each tap for
//! `net::client::FIRE_HOLD_TICKS` only produces behind a stall of several
//! ticks. `press_tick` names the intent the press came from, so the room
//! stamps `Fired::input_tick` with the press rather than with the newest
//! intent it merged.
//!
//! **A starved tick** - nothing waiting - counts a starvation and repeats
//! the last intent, for at most `INTENT_COAST`; a seat with nobody
//! connected reads as no input (`clear`). For a server-driven seat the
//! repeat is exact. For an owned seat the pose is dead-reckoned along its
//! reported velocity by the ticks since it was taken, for at most
//! `DEAD_RECKON_TICKS`, then held still with no velocity - so the other
//! players see a late packet as a hull that keeps going rather than one
//! that freezes a tick and then jumps. The trigger repeats the client's
//! own last report, never a merge's: a released tap stays released, and a
//! held trigger stays held, since letting go of it for one tick would read
//! as a fresh press on the next and fire a shell nobody asked for.
//!
//! `wire_state` keeps one meaning for both: the depth left after the
//! tick's read and whether that read starved, carried in
//! `Snapshot::mailbox`.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::PHYSICS_FIXED_DT;
use crate::net::wire::{IntentMsg, dequantise_pos, dequantise_velocity, quantise_pos};

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
/// lead ever asks for. An owned seat's dropped intents give up their pose
/// but not their trigger, which is folded into the next read's.
pub const BUFFER_MAX: usize = 8;

/// How many starved ticks in a row an owned seat's pose is carried along
/// its last reported velocity before it holds still: 50 ms, the late
/// packet a jittery link delivers rather than the silence of one that
/// dropped. Past it the hull waits where the reckoning left it, since a
/// guess that runs on is a hull the next real pose has to be pulled back
/// from.
pub const DEAD_RECKON_TICKS: u32 = 3;

/// The bit of `wire_state` that says the last read found nothing waiting.
pub const STARVED_BIT: u8 = 0x80;

/// The client's trigger across a run of its intents, read in tick order:
/// where it ended, and the first press (down after up) along the way.
#[derive(Clone, Copy, Debug)]
struct Scan {
    /// The trigger as of the last intent scanned.
    down: bool,
    /// The tick of the first press, if there was one.
    press: Option<u32>,
    /// The newest tick scanned.
    through: Option<u32>,
}

impl Scan {
    fn from(down: bool) -> Scan {
        Scan { down, press: None, through: None }
    }

    fn step(&mut self, msg: &IntentMsg) {
        if msg.fire && !self.down && self.press.is_none() {
            self.press = Some(msg.tick);
        }
        self.down = msg.fire;
        self.through = Some(msg.tick);
    }
}

#[derive(Default)]
struct Inner {
    /// Waiting to be applied, keyed by the client's tick. A `BTreeMap`
    /// rather than a queue because packets arrive out of order and a
    /// duplicate has to collapse onto the one already held.
    queue: BTreeMap<u32, IntentMsg>,
    /// The newest intent applied, as the client sent it: what a starved
    /// tick repeats.
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
    /// Starved reads in a row since an intent was last taken: how far an
    /// owned pose has been dead-reckoned.
    starved_run: u32,
    /// The trigger the last read delivered - what the round last saw.
    delivered: bool,
    /// A press the last read could not deliver (the round still had the
    /// trigger down), owed to the next read with its intent's tick.
    owed_press: Option<u32>,
    /// The tick of the press the last read delivered, if it delivered one.
    press: Option<u32>,
    /// The trigger of the owned intents dropped over `BUFFER_MAX` since
    /// the last read, so a press in a backlog is not lost with its pose.
    folded: Option<Scan>,
}

impl Inner {
    /// The client's own trigger as of the last intent applied.
    fn own_down(&self) -> bool {
        self.last.is_some_and(|m| m.fire)
    }

    /// An owned seat's read: every intent waiting, merged into the newest.
    fn take_all(&mut self) -> Option<IntentMsg> {
        let mut scan = self.folded.take().unwrap_or_else(|| Scan::from(self.own_down()));
        let queue = std::mem::take(&mut self.queue);
        for msg in queue.values() {
            scan.step(msg);
        }
        let newest = *queue.values().next_back()?;
        self.applied = Some(newest.tick);
        self.last = Some(newest);
        self.last_starved = false;
        self.starved_run = 0;
        let fire = self.deliver(scan.press, newest.fire);
        Some(IntentMsg { fire, ..newest })
    }

    /// The trigger an owned read hands the round: `press` is the client's
    /// first press among the intents read, `own` its trigger as of the
    /// newest. The rule is the module doc's.
    fn deliver(&mut self, press: Option<u32>, own: bool) -> bool {
        let (fire, delivered_press) = match (self.owed_press.take(), press) {
            // Owed from the last read, which let the trigger up for it:
            // down now, whatever this read found.
            (Some(owed), _) => (true, Some(owed)),
            (None, Some(press)) if !self.delivered => (true, Some(press)),
            (None, Some(press)) => {
                // The round still has the trigger down, so this press
                // would not read as one: let go now, press next tick.
                self.owed_press = Some(press);
                (false, None)
            }
            (None, None) => (own, None),
        };
        self.delivered = fire;
        self.press = delivered_press;
        fire
    }

    /// An owned seat's starved read: the last intent, its pose carried
    /// along its velocity for up to `DEAD_RECKON_TICKS`, then held.
    fn reckon(&mut self, last: IntentMsg) -> IntentMsg {
        let ticks = self.starved_run.min(DEAD_RECKON_TICKS);
        let dt = PHYSICS_FIXED_DT * ticks as f32;
        let x = quantise_pos(dequantise_pos(last.x) + dequantise_velocity(last.vx) * dt);
        let y = quantise_pos(dequantise_pos(last.y) + dequantise_velocity(last.vy) * dt);
        let holding = self.starved_run > DEAD_RECKON_TICKS;
        let fire = self.deliver(None, last.fire);
        IntentMsg {
            x,
            y,
            vx: if holding { 0 } else { last.vx },
            vy: if holding { 0 } else { last.vy },
            fire,
            ..last
        }
    }
}

/// One seat's intents: ordered for a server-driven seat, newest-wins for
/// an owned one.
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
        // Older than an owned intent already folded away: its pose is
        // superseded and its trigger would be read out of order.
        if inner.folded.and_then(|f| f.through).is_some_and(|through| msg.tick <= through) {
            return;
        }
        inner.queue.insert(msg.tick, msg);
        // Past the cap the oldest go: a client that has run far ahead is
        // better served by the freshest inputs than by working through a
        // backlog the round has left behind. An owned intent's trigger
        // stays, folded into the next read.
        while inner.queue.len() > BUFFER_MAX {
            let Some((_, oldest)) = inner.queue.pop_first() else { break };
            if oldest.owned {
                let down = inner.own_down();
                inner.folded.get_or_insert_with(|| Scan::from(down)).step(&oldest);
            }
        }
    }

    /// The intent this tick applies. A server-driven seat's is the oldest
    /// one waiting; an owned seat's (the newest waiting carries a pose) is
    /// every one waiting, merged; a starved tick repeats the last one
    /// while it is younger than `INTENT_COAST`, an owned pose reckoned
    /// forward. `None` when nothing was ever posted, the seat has coasted
    /// out, or the mailbox was cleared.
    pub fn read(&self, now: Instant) -> Option<IntentMsg> {
        let mut inner = self.inner.lock().expect("mailbox poisoned");
        let posted = inner.posted?;
        if now.saturating_duration_since(posted) > INTENT_COAST {
            return None;
        }
        let newest_owned = inner.queue.values().next_back().map(|m| m.owned);
        match newest_owned {
            Some(true) => inner.take_all(),
            Some(false) => {
                let (tick, msg) = inner.queue.pop_first().expect("the queue has a newest, so a first");
                inner.applied = Some(tick);
                inner.last = Some(msg);
                inner.last_starved = false;
                inner.starved_run = 0;
                // The ordered path hands every intent over as sent; the
                // owned path's trigger bookkeeping starts again from it.
                inner.folded = None;
                inner.owed_press = None;
                inner.press = (msg.fire && !inner.delivered).then_some(tick);
                inner.delivered = msg.fire;
                Some(msg)
            }
            None => {
                inner.starvations += 1;
                inner.last_starved = true;
                inner.starved_run = inner.starved_run.saturating_add(1);
                let last = inner.last?;
                if last.owned {
                    Some(inner.reckon(last))
                } else {
                    inner.press = None;
                    Some(last)
                }
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

    /// The input tick a shot fired on this tick answers, which the room
    /// stamps `Fired::input_tick` with: the intent the trigger was pressed
    /// on when the last read delivered a press - an owned read merges
    /// several, so the press can be older than the ack - and the ack
    /// otherwise, which is what a held full-auto trigger fires on.
    pub fn press_tick(&self) -> u32 {
        let inner = self.inner.lock().expect("mailbox poisoned");
        inner.press.or(inner.applied).unwrap_or(0)
    }

    /// How many ticks have found this seat's buffer empty. `/metrics`
    /// reports the total; the client reads each one as it happens off
    /// `wire_state`.
    pub fn starvations(&self) -> u64 {
        self.inner.lock().expect("mailbox poisoned").starvations
    }

    /// How many intents are waiting. Deep means a server-driven seat's
    /// client is stamping further ahead than it needs to; an owned seat's
    /// is 0 after every read.
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
    use crate::net::wire::quantise_velocity;

    fn intent(tick: u32, move_dir: u8) -> IntentMsg {
        IntentMsg { tick, move_dir, face: 0, fire: false, ..IntentMsg::default() }
    }

    /// An owned intent: the hull at (`x`, 100) px moving `vx` px/s, the
    /// trigger as given.
    fn owned(tick: u32, x: f32, vx: f32, fire: bool) -> IntentMsg {
        IntentMsg {
            tick,
            move_dir: 4,
            fire,
            owned: true,
            x: quantise_pos(x),
            y: quantise_pos(100.0),
            vx: quantise_velocity(vx),
            ..IntentMsg::default()
        }
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

    /// A server-driven seat's trigger goes through untouched, one intent
    /// a tick, and a starved tick repeats it exactly - held included.
    #[test]
    fn a_server_driven_seats_trigger_is_applied_as_sent() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        let fired = |tick| IntentMsg { fire: true, ..intent(tick, 0) };
        mailbox.post(fired(1), now);
        mailbox.post(fired(2), now);
        mailbox.post(intent(3, 0), now);
        assert_eq!(mailbox.read(now), Some(fired(1)));
        assert_eq!(mailbox.press_tick(), 1, "the press is the intent it came on");
        assert_eq!(mailbox.read(now), Some(fired(2)));
        assert_eq!(mailbox.press_tick(), 2, "a held trigger fires on the ack");
        assert_eq!(mailbox.read(now), Some(intent(3, 0)));
        mailbox.post(fired(4), now);
        assert_eq!(mailbox.read(now), Some(fired(4)));
        assert_eq!(mailbox.read(now), Some(fired(4)), "a starved tick repeats the intent exactly, trigger and all");
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

    /// **Owned poses are newest-wins.** A burst of owned intents is one
    /// tick's work: the newest pose, the ack at the newest tick, nothing
    /// left behind - so a burst never becomes standing depth.
    #[test]
    fn an_owned_burst_is_taken_whole_by_one_tick() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        mailbox.post(owned(10, 100.0, 60.0, false), now);
        mailbox.post(owned(11, 101.0, 60.0, false), now);
        mailbox.post(owned(12, 102.0, 60.0, false), now);
        assert_eq!(mailbox.depth(), 3);
        let read = mailbox.read(now).expect("an intent");
        assert_eq!(read, owned(12, 102.0, 60.0, false), "the newest pose, whole");
        assert_eq!(mailbox.acked_tick(), 12, "acked at the newest intent taken");
        assert_eq!(unpack(mailbox.wire_state()), (0, false), "nothing left waiting");
        assert_eq!(mailbox.depth(), 0);
    }

    /// A tap that went down and up between two ticks still fires: the
    /// merged trigger is down if any intent pressed it, and the press is
    /// named by the intent it came on.
    #[test]
    fn an_owned_merge_ors_the_trigger_so_a_tap_is_never_lost() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        mailbox.post(owned(1, 100.0, 0.0, false), now);
        mailbox.post(owned(2, 100.0, 0.0, true), now);
        mailbox.post(owned(3, 100.0, 0.0, false), now);
        let read = mailbox.read(now).expect("an intent");
        assert!(read.fire, "the tap inside the burst reaches the tick");
        assert_eq!(mailbox.press_tick(), 2, "and names the intent it was pressed on");
        assert_eq!(mailbox.acked_tick(), 3, "while the ack is the newest");
        // Released since: the next tick lets go.
        mailbox.post(owned(4, 100.0, 0.0, false), now);
        assert!(!mailbox.read(now).unwrap().fire, "the release follows");
        assert_eq!(mailbox.press_tick(), 4, "no press, so the ack");
    }

    /// The round only fires a shell on the trigger going down. A release
    /// hidden inside one merge and a new press in the next would read to
    /// it as one long hold, so the mailbox lets go for a tick and presses
    /// on the one after.
    #[test]
    fn a_press_after_a_hidden_release_is_delivered_up_then_down() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        // Down and up between two ticks: the round sees it down.
        mailbox.post(owned(1, 100.0, 0.0, true), now);
        mailbox.post(owned(2, 100.0, 0.0, false), now);
        assert!(mailbox.read(now).unwrap().fire);
        // Pressed again before the next tick: up now...
        mailbox.post(owned(3, 100.0, 0.0, true), now);
        assert!(!mailbox.read(now).unwrap().fire, "the release the last merge hid");
        // ...and down on the next, even with nothing new arrived.
        let starved = mailbox.read(now).unwrap();
        assert!(starved.fire, "the owed press");
        assert_eq!(mailbox.press_tick(), 3, "named by the intent it came on");
        // Held from here: no more presses.
        mailbox.post(owned(4, 100.0, 0.0, true), now);
        assert!(mailbox.read(now).unwrap().fire, "held");
        assert_eq!(mailbox.press_tick(), 4, "a hold fires on the ack");
    }

    /// A held trigger stays held through the merge, and a hold's release
    /// inside a burst is a release - neither makes a press of its own.
    #[test]
    fn an_owned_hold_is_a_hold_and_its_release_a_release() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        mailbox.post(owned(1, 100.0, 0.0, true), now);
        assert!(mailbox.read(now).unwrap().fire);
        mailbox.post(owned(2, 100.0, 0.0, true), now);
        mailbox.post(owned(3, 100.0, 0.0, true), now);
        assert!(mailbox.read(now).unwrap().fire, "held across a burst");
        assert_eq!(mailbox.press_tick(), 3, "no new press");
        mailbox.post(owned(4, 100.0, 0.0, true), now);
        mailbox.post(owned(5, 100.0, 0.0, false), now);
        assert!(!mailbox.read(now).unwrap().fire, "a hold let go inside a burst is let go");
    }

    /// **A late packet keeps the hull going.** A starved tick carries an
    /// owned pose along its velocity for `DEAD_RECKON_TICKS`, then holds
    /// it still with no velocity; the ack does not move.
    #[test]
    fn a_starved_owned_tick_dead_reckons_then_holds() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        // 60 px/s is exactly one pixel a tick.
        mailbox.post(owned(7, 100.0, 60.0, false), now);
        assert_eq!(dequantise_pos(mailbox.read(now).unwrap().x), 100.0);
        for n in 1..=DEAD_RECKON_TICKS {
            let read = mailbox.read(now).unwrap();
            assert_eq!(dequantise_pos(read.x), 100.0 + n as f32, "starved tick {n} reckons {n} px on");
            assert_eq!(dequantise_velocity(read.vx), 60.0, "and keeps going");
            assert_eq!(dequantise_pos(read.y), 100.0);
        }
        for _ in 0..3 {
            let read = mailbox.read(now).unwrap();
            assert_eq!(dequantise_pos(read.x), 100.0 + DEAD_RECKON_TICKS as f32, "past the cap it holds");
            assert_eq!((read.vx, read.vy), (0, 0), "and holds still");
        }
        assert_eq!(mailbox.acked_tick(), 7, "a reckoned tick acks nothing");
        assert_eq!(mailbox.starvations(), DEAD_RECKON_TICKS as u64 + 3);
        assert_eq!(unpack(mailbox.wire_state()), (0, true));
        // The next real pose replaces the guess and starts the count again.
        mailbox.post(owned(8, 104.0, 60.0, false), now);
        assert_eq!(dequantise_pos(mailbox.read(now).unwrap().x), 104.0);
        assert_eq!(dequantise_pos(mailbox.read(now).unwrap().x), 105.0, "reckoned from the new pose");
    }

    /// A starved tick never repeats a press: a tap merged into the last
    /// read is released, while a trigger the client still holds stays
    /// down - letting it up would make the next packet a fresh press.
    #[test]
    fn a_starved_owned_tick_repeats_the_clients_trigger_not_a_press() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        mailbox.post(owned(1, 100.0, 0.0, true), now);
        mailbox.post(owned(2, 100.0, 0.0, false), now);
        assert!(mailbox.read(now).unwrap().fire, "the tap");
        assert!(!mailbox.read(now).unwrap().fire, "a starved tick does not press it again");

        let held = Mailbox::new();
        held.post(owned(1, 100.0, 0.0, true), now);
        assert!(held.read(now).unwrap().fire);
        assert!(held.read(now).unwrap().fire, "a held trigger stays held through a starved tick");
        held.post(owned(2, 100.0, 0.0, true), now);
        assert!(held.read(now).unwrap().fire);
        assert_eq!(held.press_tick(), 2, "and the packet after it is no new press");
    }

    /// Over the cap an owned intent gives up its pose, never its trigger:
    /// a press in a backlog longer than `BUFFER_MAX` still fires.
    #[test]
    fn an_owned_backlog_over_the_cap_keeps_its_press() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        mailbox.post(owned(0, 100.0, 0.0, true), now);
        mailbox.post(owned(1, 100.0, 0.0, false), now);
        for tick in 2..(BUFFER_MAX as u32 + 6) {
            mailbox.post(owned(tick, 100.0 + tick as f32, 60.0, false), now);
        }
        assert_eq!(mailbox.depth(), BUFFER_MAX);
        // A straggler behind what was folded away is dropped.
        mailbox.post(owned(1, 100.0, 0.0, false), now);
        assert_eq!(mailbox.depth(), BUFFER_MAX);
        let read = mailbox.read(now).unwrap();
        let newest = BUFFER_MAX as u32 + 5;
        assert_eq!(read.tick, newest);
        assert_eq!(dequantise_pos(read.x), 100.0 + newest as f32, "the newest pose");
        assert!(read.fire, "the press at tick 0 survived the cap");
        assert_eq!(mailbox.press_tick(), 0);
        assert_eq!(mailbox.depth(), 0);
    }
}
