//! A seat's intents between the socket and the tick (docs/online-coop-prd.md
//! §4.1, §4.12, §4.14, §4.16). One type for every room there is - the
//! room server's (`bongbong_server::room`), the rig's thread and
//! `rig::Lockstep` - so a seat's input reaches the round by one rule
//! whichever it is playing against. A tick reads the mailbox once; what
//! the read takes depends on what the client is sending.
//!
//! **A server-driven seat (stage 2) is an ordered jitter buffer**: its
//! intents are held by the client's own tick and applied **one per tick,
//! in order**. A client predicting its own hull replays the inputs the
//! server has not acknowledged yet, so the two sides have to apply the
//! same inputs in the same sequence or the replay lands somewhere the
//! server never was; holding only the newest would drop an input
//! whenever two packets arrive inside one tick, and jitter guarantees
//! that happens. The buffer never holds an intent back - each tick takes
//! the oldest waiting - so depth is the client's to manage, and
//! `wire_state` is how it finds out.
//!
//! **A client-owned seat (stage 3 and on, `IntentMsg::owned`) is played
//! out on the room's own clock.** The mailbox keeps a *play point*, the
//! client tick the room is applying, and moves it on one tick a read. A
//! read takes every intent at or before the play point - pose,
//! `move_dir`, `face` and the view (`view_tick`) from the newest of them,
//! the trigger merged press for press - and leaves the later ones for
//! their own ticks, so the room's copy of the hull walks the client's
//! own track one client tick per room tick however the packets bunched
//! on the way. Applying the newest intent waiting instead makes that
//! track stall and double whenever arrivals straddle a tick - one tick in
//! ten on a LAN - and every other player draws the room's track. There
//! are no inputs to replay, so a read that finds several waiting at or
//! before its point takes them all and the ack is the newest taken.
//!
//! The first read takes whatever is waiting. From there every
//! `PLAYOUT_WINDOW_TICKS` reads are a window: `PLAYOUT_WIDEN_MISSES` or
//! more reads that found nothing at or past their tick arrived hold the
//! point for a tick, widening the margin by one, and a window in which
//! every read had at least `PLAYOUT_NARROW_AHEAD` ticks to spare moves it
//! on two, narrowing it by one - so the point settles a tick or so behind
//! the latest arrival the link actually delivers, costing the room's copy
//! that much and no more, and follows a link that changes. A tick lost on
//! the way while later ones have arrived is no miss: waiting longer would
//! not bring it. The margin never widens past `BUFFER_MAX - 1`: a window
//! in which any read already stood that far behind the newest arrival is
//! not held, since one tick more is the cap.
//!
//! Two things move the point by more than the controller does. A point
//! `BUFFER_MAX` or more behind the newest arrival has lost the tick it
//! would take next (`post` keeps only the newest `BUFFER_MAX`): it starts
//! again one tick behind the newest when the room's own time since the
//! last pose covers that much driving - a room that stood still while its
//! client drove on - and otherwise steps over only what was dropped, onto
//! the oldest intent still waiting, so the validator is never handed a
//! jump it must refuse. And a point more than `PLAYOUT_NARROW_AHEAD`
//! past the newest intent a read took goes back to that intent: the
//! client's clock slipped behind the room's - a frame hitch longer than
//! it catches up, a hidden tab - and its ticks now arrive at or before
//! the point, where every read would take whatever came. A burst held up
//! on the way is not that: it runs up to the client's current tick, at
//! or past the point. A seat that coasted out (`INTENT_COAST`) starts
//! its point again, as on its first read.
//!
//! **How far an owned pose is believed is the room's own time**
//! (`Mailbox::pose_reach_ticks`, which a room hands
//! `net::authority::take_pose` and the validator scales its reach by):
//! the ticks of the client's driving from the intent the last read
//! applied to the one this read applied, plus the ticks the room
//! dead-reckoned the hull meanwhile - the guess may have gone the other
//! way - but never more than the room's own ticks since that last pose
//! (its reads, or the wall clock's ticks where the room stood still)
//! plus `REACH_SPARE_TICKS`, and at most `REACH_TICKS_MAX`. An ordinary
//! read covers one tick, which the validator's floor
//! (`simulation::POSE_REACH_TICKS`) already allows for; the read that
//! ends a stall covers the whole stall, and refusing it would yank an
//! honest client back with a `Placed`; a client stamping its packets
//! further apart than it drives gains nothing by it.
//!
//! **The trigger survives the merge press for press.** Shells and plasma
//! are edge-triggered - the round fires one on the trigger going down
//! between two ticks and never again while it stays down - so what a read
//! delivers is chosen to keep every press the client made a press the
//! round sees:
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
//! intent it merged (`net::authority::stamp_presses`), and the client
//! pairs the shot it drew with the room's by that tick.
//!
//! **A starved tick** - nothing waiting - counts a starvation and repeats
//! the last intent, for at most `INTENT_COAST`; a seat with nobody
//! connected reads as no input (`clear`). For a server-driven seat the
//! repeat is exact. For an owned seat the pose is dead-reckoned along its
//! reported velocity by the ticks since it was taken, for at most
//! `DEAD_RECKON_TICKS`, then held still with no velocity - so the other
//! players see a late packet as a hull that keeps going rather than one
//! that freezes a tick and then jumps.
//!
//! **A starved owned tick repeats the client's own last trigger** - not
//! the merge the last read delivered, and not a trigger cleared for want
//! of news. Shells are edge-triggered, so the trigger a starved tick hands
//! the round decides whether the next real packet reads as a press. A
//! tap merged into the last read is let go, since holding the merge down
//! would swallow the next press's edge; and a trigger the client is still
//! holding stays down, since clearing it for the starved tick would let
//! it up, and the next packet - still held - would put it down again and
//! fire a shell the player never pressed for.
//!
//! `wire_state` keeps one meaning for both: the depth left after the
//! tick's read and whether that read starved, carried in
//! `Snapshot::mailbox`.
//!
//! **The hold report** (`Mailbox::hold_ticks`, docs/gauss-rail.md "The hold
//! report"): how many of the client's own ticks its trigger has been held,
//! counted along every intent taken in tick order - dropped ones folded in
//! - so a charge-and-hold trigger counts the client's ticks rather than the
//! room's reads, which a merge, a starved tick or a held play point make
//! differ. A read that delivers the trigger down reports the hold running
//! (or, for a tap merged into one read, the one that just ended); one that
//! delivers it up reports the hold that last ended. A starved read reports
//! nothing, and the round counts its own tick.

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

/// The most ticks of driving one owned read vouches for
/// (`Mailbox::pose_reach_ticks`): `INTENT_COAST`, the longest silence a
/// seat is still driven through. A longer stall ends with the seat
/// coasted out, and the validator answers the pose after it with
/// `Placed`.
pub const REACH_TICKS_MAX: u32 = 30;

/// Ticks of driving an owned read may vouch for beyond the room's own
/// ticks since the pose before it: a packet stamped a tick early, and the
/// play point moving two in a narrowing read.
pub const REACH_SPARE_TICKS: u32 = 2;

/// Reads in one window of the owned play point's controller: a second.
pub const PLAYOUT_WINDOW_TICKS: u32 = 60;

/// Reads in a window that found nothing at or past their tick arrived
/// before the play point is held a tick to widen its margin. One is a
/// hiccup the dead reckoning rides out; two a second is a link that
/// needs the room.
pub const PLAYOUT_WIDEN_MISSES: u32 = 2;

/// Ticks every read of a window had to spare - the newest arrival that
/// far past the play point - before the point is moved on two to narrow
/// its margin; and how far past the newest intent a read took the point
/// may run before it is put back on the client's stream.
pub const PLAYOUT_NARROW_AHEAD: i64 = 2;

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
    /// The ticks of the client's driving the last read's pose covers
    /// (`Mailbox::pose_reach_ticks`).
    reach: u32,
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
    /// The newest client tick ever posted.
    newest_posted: Option<u32>,
    /// An owned seat's play point: the client tick the last read applied
    /// up to. `None` until the first owned read, and again once the seat
    /// has coasted out.
    play: Option<u32>,
    /// How far the next owned read moves the play point: one, or the
    /// controller's hold (0) or catch-up (2) for one read.
    next_step: u32,
    /// Reads in the controller's current window, the ones among them that
    /// found nothing at or past their tick arrived, and the fewest and
    /// the most ticks any of them had to spare.
    window_reads: u32,
    window_misses: u32,
    window_ahead: i64,
    window_ahead_max: i64,
    /// Reads since an intent was last applied, and when that was: the
    /// room's own time a pose may vouch for.
    reads_since_apply: u32,
    applied_at: Option<Instant>,
    /// The client tick the trigger hold running started on, and the newest
    /// tick it was still held on (`Inner::track_hold`).
    held_since: Option<u32>,
    held_through: u32,
    /// The length in client ticks of the hold that last ended.
    held_finished: Option<u32>,
    /// The last read's hold report (`Mailbox::hold_ticks`).
    hold: Option<u32>,
}

impl Inner {
    /// The client's own trigger as of the last intent applied.
    fn own_down(&self) -> bool {
        self.last.is_some_and(|m| m.fire)
    }

    /// One intent's trigger into the hold count, in tick order.
    fn track_hold(&mut self, msg: &IntentMsg) {
        if msg.fire {
            self.held_since.get_or_insert(msg.tick);
            self.held_through = msg.tick;
        } else if let Some(since) = self.held_since.take() {
            self.held_finished = Some(self.held_through.saturating_sub(since) + 1);
        }
    }

    /// The hold report for a read that delivered the trigger `fire`: the
    /// hold running while it is down, else the one that last ended.
    fn report_hold(&mut self, fire: bool) {
        self.hold = match self.held_since {
            Some(since) if fire => Some(self.held_through.saturating_sub(since) + 1),
            _ => self.held_finished,
        };
    }

    /// An owned seat's read: the play point moved on a tick (or as the
    /// controller asked), every intent at or before it merged into the
    /// newest of them, and a pose dead-reckoned when none has arrived.
    fn read_owned(&mut self, now: Instant) -> Option<IntentMsg> {
        let waiting = self.queue.keys().next_back().copied();
        let step = std::mem::replace(&mut self.next_step, 1);
        let mut play = match (self.play, waiting) {
            (Some(play), _) => play.saturating_add(step),
            // The first owned read takes whatever is waiting.
            (None, Some(newest)) => newest,
            (None, None) => return self.starve(),
        };
        let newest = self.newest_posted.unwrap_or(play);
        // `BUFFER_MAX` behind, the point's own tick is one `post` dropped
        // to make room, and every tick after it would be too. The point
        // starts again one tick behind the newest when the room's own
        // time since the last pose covers that much driving - a room that
        // stood still while its client drove on - and otherwise steps
        // over only what was dropped, onto the oldest still waiting, so
        // the validator is never handed a jump it has to refuse.
        if newest >= play.saturating_add(BUFFER_MAX as u32) {
            let restart = newest - 1;
            let covered = match (self.applied, self.applied_at) {
                (Some(applied), Some(at)) => {
                    let room = self.reads_since_apply.max(wall_ticks(now, at)).saturating_add(1 + REACH_SPARE_TICKS);
                    room >= restart.saturating_sub(applied)
                }
                _ => true,
            };
            play = if covered { restart } else { self.queue.keys().next().copied().map_or(restart, |oldest| oldest.min(restart)) };
        }
        self.play = Some(play);
        self.reads_since_apply = self.reads_since_apply.saturating_add(1);
        let later = self.queue.split_off(&play.saturating_add(1));
        let taken = std::mem::replace(&mut self.queue, later);
        let ahead = newest as i64 - play as i64;
        // A miss is a tick late: nothing at or past the point has
        // arrived. A tick lost while later ones are here is not one, and
        // a held point finds its tick already applied.
        let missed = taken.is_empty() && step > 0 && ahead < 0;
        self.watch_playout(ahead, missed);
        if taken.is_empty() {
            return self.starve();
        }
        let mut scan = self.folded.take().unwrap_or_else(|| Scan::from(self.own_down()));
        for msg in taken.values() {
            scan.step(msg);
            self.track_hold(msg);
        }
        let oldest = *taken.values().next()?;
        let newest_taken = *taken.values().next_back()?;
        // The newest the client has sent is well behind the point, so
        // its clock slipped: back onto its stream, from where the next
        // read takes the tick after this one.
        if (newest_taken.tick as i64) + PLAYOUT_NARROW_AHEAD < play as i64 {
            self.play = Some(newest_taken.tick);
        }
        // The client's driving from the intent last applied to the newest
        // taken - with none applied yet, from just before the oldest -
        // and the ticks the room guessed meanwhile, held to the room's
        // own time since that last pose.
        let driven = match self.applied {
            Some(applied) => newest_taken.tick.saturating_sub(applied),
            None => newest_taken.tick.saturating_sub(oldest.tick).saturating_add(1),
        };
        let guessed = self.starved_run.min(DEAD_RECKON_TICKS);
        let elapsed = match self.applied_at {
            Some(at) => self.reads_since_apply.max(wall_ticks(now, at)).saturating_add(REACH_SPARE_TICKS),
            None => REACH_TICKS_MAX,
        };
        self.reach = driven.saturating_add(guessed).min(elapsed).clamp(1, REACH_TICKS_MAX);
        self.applied = Some(newest_taken.tick);
        self.applied_at = Some(now);
        self.reads_since_apply = 0;
        self.last = Some(newest_taken);
        self.last_starved = false;
        self.starved_run = 0;
        let fire = self.deliver(scan.press, newest_taken.fire);
        self.report_hold(fire);
        // The lamp key is read as held if any intent taken held it, so a
        // press inside a merge still reaches the round; its edge is the
        // round's to find (docs/volcano.md).
        let lamp = taken.values().any(|m| m.lamp);
        Some(IntentMsg { fire, lamp, ..newest_taken })
    }

    /// An owned read with nothing to apply: a starvation, the last pose
    /// carried on.
    fn starve(&mut self) -> Option<IntentMsg> {
        self.starvations += 1;
        self.last_starved = true;
        self.hold = None;
        self.starved_run = self.starved_run.saturating_add(1);
        let last = self.last?;
        Some(self.reckon(last))
    }

    /// One owned read's reading for the play point's controller: how many
    /// ticks the newest arrival stood past the point, and whether the
    /// read was late - nothing at or past the point arrived. At the end of
    /// a window the point is held a tick if the window missed too often,
    /// unless a read already stood `BUFFER_MAX - 1` behind the newest
    /// arrival (one more and it starts again), or moved on two if every
    /// read had room to spare.
    fn watch_playout(&mut self, ahead: i64, missed: bool) {
        let first = self.window_reads == 0;
        self.window_ahead = if first { ahead } else { self.window_ahead.min(ahead) };
        self.window_ahead_max = if first { ahead } else { self.window_ahead_max.max(ahead) };
        self.window_reads += 1;
        self.window_misses += missed as u32;
        if self.window_reads < PLAYOUT_WINDOW_TICKS {
            return;
        }
        if self.window_misses >= PLAYOUT_WIDEN_MISSES {
            if self.window_ahead_max < BUFFER_MAX as i64 - 1 {
                self.next_step = 0;
            }
        } else if self.window_ahead >= PLAYOUT_NARROW_AHEAD {
            self.next_step = 2;
        }
        self.window_reads = 0;
        self.window_misses = 0;
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
    /// along its velocity for up to `DEAD_RECKON_TICKS`, then held - one
    /// tick on from the guess the tick before, or none.
    fn reckon(&mut self, last: IntentMsg) -> IntentMsg {
        self.reach = 1;
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

/// Whole room ticks of wall time from `at` to `now`, rounded up.
fn wall_ticks(now: Instant, at: Instant) -> u32 {
    (now.saturating_duration_since(at).as_secs_f32() / PHYSICS_FIXED_DT).ceil() as u32
}

/// One seat's intents: ordered for a server-driven seat, played out on
/// the room's clock for an owned one.
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
        inner.newest_posted = Some(inner.newest_posted.map_or(msg.tick, |n| n.max(msg.tick)));
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
                inner.track_hold(&oldest);
            }
        }
    }

    /// The intent this tick applies. A server-driven seat's is the oldest
    /// one waiting; an owned seat's (the newest waiting carries a pose) is
    /// every one at or before the play point, merged; a starved tick
    /// repeats the last one while it is younger than `INTENT_COAST`, an
    /// owned pose reckoned forward. `None` when nothing was ever posted, the seat has coasted
    /// out, or the mailbox was cleared.
    pub fn read(&self, now: Instant) -> Option<IntentMsg> {
        let mut inner = self.inner.lock().expect("mailbox poisoned");
        let posted = inner.posted?;
        if now.saturating_duration_since(posted) > INTENT_COAST {
            // Coasted out: the seat's next owned read starts its play
            // point afresh, like its first.
            inner.play = None;
            return None;
        }
        // Owned when the newest intent waiting says so, or - with none
        // waiting - when the last one applied did.
        let owned = inner.queue.values().next_back().or(inner.last.as_ref()).map(|m| m.owned);
        if owned == Some(true) {
            return inner.read_owned(now);
        }
        if let Some((tick, msg)) = inner.queue.pop_first() {
            inner.applied = Some(tick);
            inner.last = Some(msg);
            inner.last_starved = false;
            inner.starved_run = 0;
            inner.reach = 1;
            // The ordered path hands every intent over as sent; the owned
            // path's trigger bookkeeping and play point start again from it.
            inner.folded = None;
            inner.owed_press = None;
            inner.play = None;
            inner.press = (msg.fire && !inner.delivered).then_some(tick);
            inner.delivered = msg.fire;
            inner.track_hold(&msg);
            inner.report_hold(msg.fire);
            return Some(msg);
        }
        inner.starvations += 1;
        inner.last_starved = true;
        inner.starved_run = inner.starved_run.saturating_add(1);
        inner.press = None;
        inner.reach = 1;
        inner.hold = None;
        inner.last
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

    /// How many ticks of the client's own driving the pose the last read
    /// delivered is believed for, which is how far from the hull the
    /// round has the validator lets that pose stand
    /// (`net::authority::take_pose`, `Game::accept_seat_pose`). An owned
    /// read that applied an intent covers the ticks from the intent
    /// applied before it, plus the ticks the room dead-reckoned in
    /// between, held to the room's own ticks since that pose plus
    /// `REACH_SPARE_TICKS` and to `REACH_TICKS_MAX`: one for the packet a
    /// tick of a steady link, the whole stall for the read that ends one.
    /// Any other read covers one - a starved read's guess moves a tick at
    /// most. 0 before any read.
    pub fn pose_reach_ticks(&self) -> u32 {
        self.inner.lock().expect("mailbox poisoned").reach
    }

    /// The last read's hold report (docs/gauss-rail.md "The hold report"):
    /// how many of the client's own ticks the trigger the read delivered
    /// has been held - while it is down the hold running, once it is up the
    /// hold that last ended - which a room hands the round
    /// (`net::authority::take_hold`) for a charge to count by. `None` for a
    /// starved read, before any read, and before any hold.
    pub fn hold_ticks(&self) -> Option<u32> {
        self.inner.lock().expect("mailbox poisoned").hold
    }

    /// How many ticks have found this seat's buffer empty. `/metrics`
    /// reports the total; the client reads each one as it happens off
    /// `wire_state`.
    pub fn starvations(&self) -> u64 {
        self.inner.lock().expect("mailbox poisoned").starvations
    }

    /// How many intents are waiting. Deep means a server-driven seat's
    /// client is stamping further ahead than it needs to; an owned seat's
    /// is the margin its play point keeps behind the newest arrival.
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

    /// **An owned burst at or before the play point is one read.** Here it
    /// is the first of the seat's stream, which the play point starts
    /// from: the newest pose, the ack at the newest tick, nothing left
    /// behind.
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

    /// The hold report counts the client's ticks of trigger held, however
    /// the reads took them: one at a time, merged, the release's read
    /// reporting the hold it ended; a starved read reports nothing.
    #[test]
    fn the_hold_report_counts_the_clients_ticks_not_the_reads() {
        let now = Instant::now();
        let ordered = Mailbox::new();
        let fire = |tick: u32, fire: bool| IntentMsg { tick, fire, ..IntentMsg::default() };
        assert_eq!(ordered.hold_ticks(), None, "nothing read");
        for tick in 0..5 {
            ordered.post(fire(tick, true), now);
            ordered.read(now);
            assert_eq!(ordered.hold_ticks(), Some(tick + 1));
        }
        ordered.read(now + Duration::from_millis(20));
        assert_eq!(ordered.hold_ticks(), None, "a starved read reports nothing");
        ordered.post(fire(5, false), now);
        ordered.read(now);
        assert_eq!(ordered.hold_ticks(), Some(5), "the release reports the hold it ended");

        let merged = Mailbox::new();
        for tick in 10..13 {
            merged.post(owned(tick, 100.0, 0.0, true), now);
        }
        merged.read(now);
        assert_eq!(merged.hold_ticks(), Some(3), "three ticks held in one read");
        merged.post(owned(13, 100.0, 0.0, true), now);
        merged.read(now);
        assert_eq!(merged.hold_ticks(), Some(4));
        merged.post(owned(14, 100.0, 0.0, false), now);
        merged.read(now);
        assert_eq!(merged.hold_ticks(), Some(4), "released after four");

        let tap = Mailbox::new();
        tap.post(owned(1, 100.0, 0.0, true), now);
        tap.post(owned(2, 100.0, 0.0, false), now);
        assert!(tap.read(now).unwrap().fire, "the tap is delivered");
        assert_eq!(tap.hold_ticks(), Some(1), "a tick's hold");
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
    /// inside a merge is a release - neither makes a press of its own.
    #[test]
    fn an_owned_hold_is_a_hold_and_its_release_a_release() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        mailbox.post(owned(1, 100.0, 0.0, true), now);
        assert!(mailbox.read(now).unwrap().fire);
        // Tick 2 is late: the read for it starves, and the next takes
        // ticks 2 and 3 together.
        assert!(mailbox.read(now).unwrap().fire, "a starved tick keeps the hold");
        mailbox.post(owned(2, 100.0, 0.0, true), now);
        mailbox.post(owned(3, 100.0, 0.0, true), now);
        assert!(mailbox.read(now).unwrap().fire, "held across a merge");
        assert_eq!(mailbox.acked_tick(), 3, "both taken");
        assert_eq!(mailbox.press_tick(), 3, "no new press");
        mailbox.read(now);
        mailbox.post(owned(4, 100.0, 0.0, true), now);
        mailbox.post(owned(5, 100.0, 0.0, false), now);
        assert!(!mailbox.read(now).unwrap().fire, "a hold let go inside a merge is let go");
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

    /// **An owned read vouches for the driving it covers, and for no
    /// more than the room's own time.** An ordinary read covers one tick,
    /// and so does a starved one; the read that ends a stall covers every
    /// tick since the intent last applied plus the ticks the room guessed
    /// while it waited; a client stamping its packets further apart than
    /// the room's ticks is believed for the room's ticks alone.
    #[test]
    fn an_owned_reads_reach_covers_the_stall_it_ends() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        assert_eq!(mailbox.pose_reach_ticks(), 0, "nothing read yet");
        mailbox.post(owned(10, 100.0, 60.0, false), now);
        mailbox.read(now);
        assert_eq!(mailbox.pose_reach_ticks(), 1, "a first read of one intent covers its tick");
        mailbox.post(owned(11, 101.0, 60.0, false), now);
        mailbox.read(now);
        assert_eq!(mailbox.pose_reach_ticks(), 1, "one intent a tick is one tick a read");
        // Four starved ticks, each guessing one tick on (the reckoning
        // stops at `DEAD_RECKON_TICKS`).
        for _ in 0..4 {
            mailbox.read(now);
            assert_eq!(mailbox.pose_reach_ticks(), 1, "a guess moves a tick at most");
        }
        // The stall ends with the five late intents at once, all at or
        // before the play point (tick 16).
        for tick in 12..=16 {
            mailbox.post(owned(tick, 100.0 + tick as f32, 60.0, false), now);
        }
        mailbox.read(now);
        assert_eq!(mailbox.acked_tick(), 16);
        // Five ticks driven and three guessed, held to the room's own five
        // ticks since the last pose and the spare.
        assert_eq!(mailbox.pose_reach_ticks(), 5 + REACH_SPARE_TICKS);
        // A gap in the ticks is driving the room never heard about: a
        // lost packet still moved the hull.
        mailbox.read(now);
        mailbox.post(owned(18, 118.0, 60.0, false), now);
        mailbox.read(now);
        assert_eq!(mailbox.acked_tick(), 18);
        assert_eq!(mailbox.pose_reach_ticks(), 2 + 1, "two ticks driven, one guessed");
        // A packet stamped a hundred ticks on: the play point starts again
        // one tick behind it, and the pose is believed for the room's own
        // ticks since the last one, not the hundred it claims.
        mailbox.post(owned(118, 218.0, 60.0, false), now);
        mailbox.read(now);
        mailbox.read(now);
        assert_eq!(mailbox.acked_tick(), 118);
        assert_eq!(mailbox.pose_reach_ticks(), 2 + REACH_SPARE_TICKS, "the room's two ticks and the spare");
        // A server-driven read vouches for nothing past its one tick.
        mailbox.post(intent(141, 1), now);
        mailbox.post(intent(142, 1), now);
        mailbox.read(now);
        assert_eq!(mailbox.pose_reach_ticks(), 1);
    }

    /// Owned intents from the tick the play point stands on at each
    /// read, `wobble` of them arriving per read around a steady one: the
    /// arrivals bunch, the room's track does not.
    fn play_out(wobble: &[usize], reads: usize) -> (Mailbox, Vec<f32>) {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        let mut next = 0u32;
        let mut xs = Vec::new();
        for i in 0..reads {
            for _ in 0..wobble[i % wobble.len()] {
                mailbox.post(owned(next, 100.0 + next as f32, 60.0, false), now);
                next += 1;
            }
            if let Some(read) = mailbox.read(now) {
                xs.push(dequantise_pos(read.x));
            }
        }
        (mailbox, xs)
    }

    /// **The room walks the client's track one tick a read** however the
    /// packets bunch: arrivals of one, none and two a tick around a
    /// steady stream give a hull that moves exactly one tick's driving on
    /// every read - a late tick carried by the dead reckoning, the pair
    /// after it taken one at a time - where applying the newest waiting
    /// stalls on one read and doubles on the next.
    #[test]
    fn the_owned_play_point_walks_the_clients_track_one_tick_a_read() {
        // 60 px/s is one pixel a tick.
        let (mailbox, xs) = play_out(&[1, 0, 2, 1, 2, 0, 1], 600);
        let steps: Vec<f32> = xs.windows(2).map(|w| w[1] - w[0]).collect();
        // The controller may move the point once a window; count those.
        let off: Vec<(usize, f32)> = steps.iter().copied().enumerate().filter(|&(_, d)| (d - 1.0).abs() > 0.01).collect();
        assert!(off.len() <= 2, "the track stalled or doubled: {off:?}");
        assert!(mailbox.depth() <= 3, "the point keeps a small margin: depth {}", mailbox.depth());
    }

    /// An intent that arrives before its tick waits for the play point:
    /// it is the client's future, not a pose to jump to.
    #[test]
    fn an_owned_intent_ahead_of_the_play_point_waits_its_tick() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        mailbox.post(owned(1, 101.0, 60.0, false), now);
        assert_eq!(mailbox.read(now).unwrap().tick, 1);
        mailbox.post(owned(2, 102.0, 60.0, false), now);
        mailbox.post(owned(3, 103.0, 60.0, false), now);
        assert_eq!(mailbox.read(now).unwrap().tick, 2, "tick 3 waits");
        assert_eq!(unpack(mailbox.wire_state()), (1, false), "one waiting behind the point");
        assert_eq!(mailbox.read(now).unwrap().tick, 3, "and is applied on its own read");
    }

    /// **The margin follows the link.** Late ticks twice in a window hold
    /// the play point a tick, which ends the misses; a window with room
    /// to spare on every read moves it on two, which takes the margin
    /// back.
    #[test]
    fn misses_widen_the_owned_margin_and_spare_narrows_it() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        let mut next = 0u32;
        let mut post = |n: usize| {
            for _ in 0..n {
                mailbox.post(owned(next, 100.0 + next as f32, 60.0, false), now);
                next += 1;
            }
        };
        // A link that is a tick late every tenth read: two misses a
        // window, until the point is held a tick.
        let mut misses = Vec::new();
        for window in 0..4 {
            let before = mailbox.starvations();
            for i in 0..PLAYOUT_WINDOW_TICKS {
                post(match i % 10 { 4 => 0, 5 => 2, _ => 1 });
                mailbox.read(now);
            }
            misses.push((window, mailbox.starvations() - before));
        }
        assert!(misses[0].1 >= PLAYOUT_WIDEN_MISSES as u64, "the fixture misses: {misses:?}");
        assert_eq!(misses[3].1, 0, "a tick of margin ends them: {misses:?}");
        let widened = mailbox.depth();
        // The link then runs two ticks early for a whole window: the point
        // moves on and the margin comes back down.
        post(2);
        for _ in 0..(2 * PLAYOUT_WINDOW_TICKS) {
            post(1);
            mailbox.read(now);
        }
        assert!(mailbox.depth() < widened + 2, "the spare was taken back: {} vs {widened}", mailbox.depth());
    }

    /// A standing owned pose at `100 + tick` px: no velocity, so a starved
    /// read holds the hull still and a gap in the track shows as a step
    /// the dead reckoning cannot fill.
    fn standing(tick: u32) -> IntentMsg {
        owned(tick, 100.0 + tick as f32, 0.0, false)
    }

    /// **A point `BUFFER_MAX` behind the newest arrival starts again.**
    /// `post` keeps only the newest `BUFFER_MAX`, so at that margin the
    /// point's own tick is one it dropped, and so is every tick after it
    /// while the client keeps sending: left there, every read would
    /// starve with a full buffer behind it.
    #[test]
    fn an_owned_point_at_the_cap_starts_again_rather_than_starving() {
        let mailbox = Mailbox::new();
        let tick = Duration::from_secs_f32(PHYSICS_FIXED_DT);
        let mut now = Instant::now();
        mailbox.post(standing(0), now);
        mailbox.read(now);
        // The room stands still while the client drives on.
        let mut next = 1u32;
        for _ in 0..=BUFFER_MAX {
            now += tick;
            mailbox.post(standing(next), now);
            next += 1;
        }
        let starvations = mailbox.starvations();
        let mut acked = Vec::new();
        for _ in 0..PLAYOUT_WINDOW_TICKS {
            mailbox.read(now);
            acked.push(mailbox.acked_tick());
            now += tick;
            mailbox.post(standing(next), now);
            next += 1;
        }
        assert_eq!(mailbox.starvations(), starvations, "a read starved with the buffer full: acked {acked:?}");
        assert_eq!(acked[0], BUFFER_MAX as u32, "the first read starts again a tick behind the newest");
        assert!(acked.windows(2).all(|w| w[1] == w[0] + 1), "and every read after it takes the next tick: {acked:?}");
    }

    /// **A point at the cap on a room that never stalled steps over only
    /// what was dropped.** The room has read every tick, so its own time
    /// since the last pose covers a tick or two of driving, not the whole
    /// margin: starting again a tick behind the newest would hand the
    /// validator a jump it refuses. The point moves onto the oldest intent
    /// still waiting instead, and every read vouches for the driving it
    /// applies.
    #[test]
    fn an_owned_point_at_the_cap_on_a_ticking_room_steps_over_only_the_dropped() {
        let mailbox = Mailbox::new();
        let tick = Duration::from_secs_f32(PHYSICS_FIXED_DT);
        let mut now = Instant::now();
        mailbox.post(standing(0), now);
        mailbox.read(now);
        // A margin of seven: ticks 1..=8 arrive together, then one a tick.
        let mut next = 1u32;
        for _ in 0..8 {
            mailbox.post(standing(next), now);
            next += 1;
        }
        let mut steps = Vec::new();
        for read in 0..40 {
            now += tick;
            let before = mailbox.acked_tick();
            mailbox.read(now);
            steps.push((mailbox.acked_tick() - before, mailbox.pose_reach_ticks()));
            // Two in one tick now and then, which puts the point at the cap.
            let arrivals = if read % 10 == 5 { 2 } else { 1 };
            for _ in 0..arrivals {
                mailbox.post(standing(next), now);
                next += 1;
            }
        }
        assert!(
            steps.iter().all(|&(driven, reach)| driven <= reach.max(1)),
            "a read applied more driving than it vouched for: {steps:?}"
        );
    }

    /// **The margin never widens into the cap.** A link that delivers in
    /// clumps of `BUFFER_MAX + 1` needs more margin than the buffer holds,
    /// so the read before each clump misses however wide the point sits.
    /// Holding the point for those misses once it stands `BUFFER_MAX - 1`
    /// behind a clump would put it at the cap on the next and start it
    /// again a tick behind - the margin thrown away and the room's track
    /// jumping a clump. It widens that far and stays: one tick a read, two
    /// after each miss.
    #[test]
    fn a_clumped_link_widens_the_owned_margin_short_of_the_cap() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        let clump = BUFFER_MAX as u32 + 1;
        let mut next = 0u32;
        let mut acked = Vec::new();
        let mut depths = Vec::new();
        for read in 0..(30 * PLAYOUT_WINDOW_TICKS) {
            if read % clump == 0 {
                for _ in 0..clump {
                    mailbox.post(standing(next), now);
                    next += 1;
                }
            }
            mailbox.read(now);
            acked.push(mailbox.acked_tick());
            depths.push(mailbox.depth());
        }
        // A window widens it a tick, so it is at its widest well before
        // the twelfth.
        let from = (12 * PLAYOUT_WINDOW_TICKS) as usize;
        let jumps: Vec<(usize, u32)> =
            acked[from..].windows(2).map(|w| w[1] - w[0]).enumerate().filter(|&(_, d)| d > 2).collect();
        assert!(jumps.is_empty(), "the point restarted: {jumps:?}");
        let widest = depths[from..].iter().copied().max().unwrap_or(0);
        assert_eq!(widest, BUFFER_MAX - 1, "the margin behind a clump");
    }

    /// **A stream behind the point is followed back.** A client whose
    /// clock slipped behind the room's - a frame hitch longer than it
    /// catches up - goes on from its own next tick, which the point passed
    /// while it was silent, and a point that stayed ahead would take
    /// whatever arrived on every read, the room's track stalling and
    /// doubling with the bunching. It goes back to the stream on the first
    /// read that takes from it, and walks it one tick a read again once
    /// the controller has found the link's margin.
    #[test]
    fn an_owned_point_ahead_of_a_slipped_stream_goes_back_to_it() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        let mut next = 0u32;
        let mut post = |n: usize| {
            for _ in 0..n {
                mailbox.post(standing(next), now);
                next += 1;
            }
        };
        let mut xs = Vec::new();
        let read = |xs: &mut Vec<f32>| {
            if let Some(read) = mailbox.read(now) {
                xs.push(dequantise_pos(read.x));
            }
        };
        // A steady stream a tick ahead of the point.
        post(1);
        read(&mut xs);
        post(2);
        for _ in 0..PLAYOUT_WINDOW_TICKS {
            read(&mut xs);
            post(1);
        }
        // The client's clock stops for twenty ticks while the room's runs.
        for _ in 0..20 {
            read(&mut xs);
        }
        // It goes on from its next tick, bunched as a LAN bunches it.
        let resumed = xs.len();
        let wobble = [1, 0, 2, 1, 2, 0, 1];
        for i in 0..(6 * PLAYOUT_WINDOW_TICKS) as usize {
            post(wobble[i % wobble.len()]);
            read(&mut xs);
        }
        // Two windows to find the margin, then four to walk.
        let walked = &xs[resumed + 2 * PLAYOUT_WINDOW_TICKS as usize..];
        let off: Vec<(usize, f32)> =
            walked.windows(2).map(|w| w[1] - w[0]).enumerate().filter(|&(_, d)| (d - 1.0).abs() > 0.01).collect();
        // The controller may move the point once a window; count those.
        assert!(off.len() <= 4, "the track stalled or doubled: {off:?}");
    }

    /// **A seat heard from again after coasting out starts afresh**: its
    /// first read takes whatever is waiting, as a seat's first read does,
    /// rather than walk on from the point the silence left behind.
    #[test]
    fn an_owned_seat_back_from_coasting_out_starts_its_point_afresh() {
        let mailbox = Mailbox::new();
        let dt = Duration::from_secs_f32(PHYSICS_FIXED_DT);
        let mut now = Instant::now();
        for tick in 0..10 {
            mailbox.post(standing(tick), now);
            mailbox.read(now);
            now += dt;
        }
        // The link goes quiet for a second: the hull is carried a while,
        // then the seat coasts out.
        let mut coasted = 0;
        for _ in 0..60 {
            coasted += mailbox.read(now).is_none() as u32;
            now += dt;
        }
        assert!(coasted > 0, "the silence outlasted INTENT_COAST");
        // It comes back with its clock forty ticks on.
        mailbox.post(standing(50), now);
        let read = mailbox.read(now).expect("the seat is back");
        assert_eq!(read.tick, 50, "the first read back takes what is waiting");
        assert_eq!(mailbox.acked_tick(), 50);
    }

    /// **A lost tick is not a late one.** A packet that never arrives
    /// leaves its read with nothing, but with later ticks already waiting
    /// holding the point would not bring it back; only a read with
    /// nothing at or past its tick counts toward widening the margin. One
    /// tick in twenty lost leaves the margin where the link's timing puts
    /// it.
    #[test]
    fn a_lost_owned_tick_does_not_widen_the_margin() {
        let mailbox = Mailbox::new();
        let now = Instant::now();
        let mut depths = Vec::new();
        for tick in 0..(6 * PLAYOUT_WINDOW_TICKS) {
            if tick % 20 != 10 {
                mailbox.post(standing(tick), now);
            }
            mailbox.read(now);
            depths.push(mailbox.depth());
        }
        let widest = depths.iter().copied().max().unwrap_or(0);
        assert!(widest <= 2, "the margin ratcheted up: widest {widest}, last {:?}", depths.last());
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
