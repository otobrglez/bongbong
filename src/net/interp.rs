//! The picture between two snapshots (docs/online-coop-prd.md §4.5,
//! §4.16): the clock that says what "now" is on the server, the ring of
//! snapshots that have arrived, the playout clock that says which instant
//! of the room's past is drawn, and the blend of two snapshots the
//! replica is written from each rendered frame.
//!
//! A room sends a snapshot every tick, sixty a second. Writing each one
//! into the replica as it arrives would draw the round at the room's
//! cadence on a 120 Hz screen - and at any cadence a packet can be late
//! or lost - so the replica draws the *past* instead: render time is the
//! server's time less a delay, which keeps two snapshots bracketing it,
//! and every frame lands somewhere between them.
//!
//! **The picture is placed in ticks, the link is measured on the wall.**
//! A snapshot's place in the round is `tick * TICK_MS`, exact, and is
//! what render time and every blend run on. Its `server_ms` is the room's
//! wall clock when it was sent, which is what the link is measured with:
//! the two differ by the round's **anchor** - `server_ms - tick time` -
//! which only moves when the room's schedule does. A room owing more
//! ticks than it catches up restarts its schedule, a paused room resumes,
//! a round starts or a welcome arrives from a waiting room at tick 0, and
//! in every case the tick numbers shift against the wall while the link
//! has not changed at all. So the clock (`ServerClock`) takes `server_ms`
//! readings, and the anchor - the lowest of the last `ANCHOR_SNAPSHOTS`,
//! which reaches past a catch-up burst's late ticks to an on-time one -
//! turns its estimate into tick time. A welcome starts the anchor again,
//! and the link's clock only when its stamp is another server's or the
//! link has been silent for longer than the clock's window (a lobby, the
//! wait for a rematch); an anchor that jumps forward holds the picture
//! for as long as the room stood still, which is what happened.
//!
//! **The clock is the lower envelope of arrivals** (`ServerClock`). Every
//! arrival reads the server's time less that packet's transit; the
//! fastest arrivals in the last `CLOCK_WINDOW_MS` are the ones that spent
//! least time queued, so the estimate is the largest reading in the
//! window - up or down, so a path that got slower for good is followed
//! the moment its last fast reading leaves the window. The one exception
//! is the aftermath of an arrival gap: once a stall outlasts the window,
//! the window holds nothing but the stall's late burst, which says
//! nothing about the path, so for `CLOCK_SETTLE_MS` after the gap - after
//! the last of them, for a backlog that comes back in chunks, and never
//! more than `CLOCK_SETTLE_MAX_MS` after the first - the estimate falls
//! toward it at most `CLOCK_FALL_PER_MS`, and only for time during which
//! snapshots were actually arriving. A reading far *above* the estimate is
//! a new clock and is taken whole. The room's stamps are a `u32` of
//! milliseconds that wraps every 49.7 days; the interpolator unwraps each
//! against the one before, so a round that spans the wrap reads one clock.
//!
//! **The picture runs on its own clock** (the playout clock). Render time
//! is a place on the tick schedule, which no clock estimate moves. It
//! advances at 1.0x real time and is steered toward its target - the
//! clock less the delay - only through a bounded rate: `RATE_NEAR` (3 %)
//! for small errors, up to `RATE_FAR` (10 %) when far off, nothing inside
//! `RATE_DEADBAND_MS`. A picture running past the newest snapshot with its
//! target behind it is a guess, so that error is closed bounded by
//! `RATE_FAR` rather than the error-ramped bound, and without the
//! deadband. It never steps backwards: a target more than
//! `RENDER_SNAP_MS` ahead is taken whole, and a target behind it - a new
//! clock, or more than `RENDER_SNAP_MS` behind - holds it where it stands
//! until the target catches up. Only a round starting over (the tick
//! counter going back) or a `Welcome` starts it afresh. Jittered arrivals
//! move neither the envelope nor the target, so on a live link the
//! picture's speed is exactly 1.0 on almost every frame.
//!
//! **The delay is sized from lateness** (decision 8). Each arrival's
//! lateness is how far it came in behind the envelope as it stood then,
//! or as it stands now if it has since fallen: when the envelope falls to
//! a slower path, the lateness the delay was paying for falls with it and
//! the render target stays where it was. To that comes how far behind the
//! anchor the room itself sent the snapshot - a tick run late and caught
//! up - since the picture has to wait for that too.
//! The target is one snapshot interval (the bracket's own width) plus one
//! 60 Hz frame plus the 95th percentile of lateness over
//! `LATENESS_WINDOW_MS` - floored at
//! `online_interpolation_delay_ms`, capped at
//! `online_interpolation_delay_max_ms`, pinned to the floor with
//! `online_interpolation_adaptive` off. Head-of-line stalls - lateness
//! past `STALL_INTERVALS` intervals arriving bunched, a lost TCP segment
//! holding everything behind it and releasing it at once - are left out
//! of the percentile while they are isolated (under `STALL_SHARE_MAX` of
//! the window): sizing the delay for them would pay a stall's worth of
//! latency on every frame to hide one frame in a few seconds, so they are
//! ridden out on extrapolation instead. Lateness that arrives on the
//! room's cadence is the path's and always counts: a route that got
//! slower is covered within a twentieth of the window, before the clock
//! has followed it.
//!
//! What blends and what does not:
//!
//! - Positions - hulls, shots, missiles, grenades, frogs - move linearly
//!   between the two ends of the bracket, on the wire's own quarter-pixel
//!   grid; a missile's and a grenade's height too, and a missile's two
//!   headings by the shortest arc.
//! - A hull's facing does not: movement is four-directional
//!   (`Tank::control` snaps `rotation`), so a heading is a fact about a
//!   tick, not a value to average. The bracket's near end owns it, and the
//!   *drawn* angle swings across in `Game::tick_presentation` exactly as
//!   it does in a local round. A shot's heading is the same: it only ever
//!   changes on a ricochet, where a snap is the truth.
//! - A hull that moved further between the two ends than it could have
//!   driven, or whose far end carries its `Teleported` or `Placed`, is not
//!   lerped across the map: it holds at the near end and is drawn at the
//!   far end the frame render time reaches it, and that frame lists it in
//!   `Frame::snapped` so the caller can lift its tread marks. A
//!   `TankEntered` is no such cue: it marks a rolling-in hull reaching the
//!   inside of its gate at the end of a drive the snapshots show whole.
//!   A shot whose far end carries its `ShotTeleported` - it went through
//!   a portal - holds the same way, and its correction offset is dropped
//!   the frame it is drawn at the exit.

//! - Everything discrete - tiles, fires, pickups, the round's scalars, the
//!   flags on a hull - is the near end's, so the picture never shows a
//!   state the server has not reached yet.
//! - Events ride the near end and are handed over exactly once, on the
//!   frame render time reaches the snapshot they were cut with, so a
//!   fireball appears when the hull it belongs to is drawn where it died.
//!   `Game::frame` is that snapshot's tick, which is what makes
//!   `Fx::observe` fire once per snapshot rather than once per frame. A
//!   snapshot the picture never showed - dropped from a full buffer, or
//!   stepped over by a frame that came after a hidden tab - keeps only the
//!   events the replica needs as state (`carries_state`) once it is more
//!   than `EVENT_STALE_MS` older than the delay would have shown it; its
//!   cosmetics would all land on one frame, a mass burst of things long
//!   over. The near end on screen always hands over everything it carries.
//!
//! **Past the newest snapshot** - a lost segment, a stalled link - hulls
//! carry on at their last velocity for `HULL_EXTRAPOLATE_MS` and then
//! dead-blend to a stop, the velocity halving every
//! `HULL_BLEND_HALF_LIFE_MS`, rather than stopping dead. A flying shot
//! carries on along its heading at its kind's speed (`shell_speed`,
//! `minigun_bullet_speed`, `plasma_speed`) and a missile along its ground
//! heading at the speed its last two snapshots show, both for
//! `SHOT_EXTRAPOLATE_MS`; a shot at the muzzle or in its impact frames
//! holds. Frogs hold: a hop is a scripted arc the wire describes whole.
//!
//! **Corrections ease** (`Interpolator::offsets`). When the snapshots that
//! arrive after an extrapolated frame place an entity somewhere other
//! than where it was drawn, the difference is kept as a per-entity error
//! offset - last drawn less the new truth - and halved every
//! `ERROR_HALF_LIFE_MS`, so a late packet is a glide rather than a jump.
//! Offsets are keyed by tank, shot or missile id, dropped with the
//! entity, and never carried across a snap; a correction under
//! `ERROR_EASE_MIN_PX` (finer than the art can show) or past
//! `ERROR_SNAP_PX` (a jump, not a drift) is drawn whole.
//!
//! No RNG, no `Game`, no raylib: a `Snapshot` in, a `Snapshot` out.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::bullet::BulletState;
use crate::net::events::WireEvent;
use crate::net::wire::{
    MissileState, ShotKind, ShotState, Snapshot, TankState, dequantise_heading, dequantise_velocity, quantise_pos,
};
use crate::plasma::PlasmaState;
use crate::shell::ShellState;
use crate::tuning::tuning;

/// One room tick in milliseconds: the room runs at 60 Hz, and a
/// snapshot's time is its tick times this, exactly.
pub const TICK_MS: f64 = 1000.0 / 60.0;

/// The gap between two snapshots of a room that sends one every tick
/// (`bongbong_server::room::SNAPSHOT_EVERY` is 1). The starting guess for
/// `Interpolator::interval_ms`, which then follows the ticks that
/// actually arrive.
pub const SNAPSHOT_INTERVAL_MS: f64 = TICK_MS;

/// One rendered frame at 60 Hz, part of the delay: render time is
/// sampled once a frame, so a snapshot can arrive just after the frame
/// that needed it.
pub const FRAME_MS: f64 = 1000.0 / 60.0;

/// A reading this far *above* the clock's estimate - a snapshot that
/// arrived half a second earlier than any before it could have - or a
/// jump this large in `server_ms - tick time` is not jitter but another
/// clock: a rejoin, a room restarted or resumed under the same socket. It
/// is taken whole.
pub const CLOCK_SNAP_MS: f64 = 500.0;

/// Snapshots the anchor (`server_ms - tick time`) is the lowest of: one
/// more than a catch-up burst's run of late ticks
/// (`net::authority::CATCH_UP_TICKS`) and the on-time one after it, so a
/// burst never moves the anchor and a restarted schedule moves it within
/// a tenth of a second.
pub const ANCHOR_SNAPSHOTS: usize = crate::net::authority::CATCH_UP_TICKS as usize + 2;

/// An anchor that moves forward further than this is a room that fell
/// behind its own schedule and restarted it: the picture holds for as
/// long rather than playing on through time the room never ran.
pub const ANCHOR_HOLD_MS: f64 = 2.0 * TICK_MS;

/// Snapshots in a row that agree on a later anchor, within
/// `ANCHOR_AGREE_MS`, before it is taken without waiting out
/// `ANCHOR_SNAPSHOTS`: a restarted schedule stamps every tick after it
/// the same amount late, where a catch-up burst's late ticks each come a
/// whole tick less late than the one before - two tell them apart.
pub const ANCHOR_RUN: usize = 2;

/// How closely `ANCHOR_RUN` snapshots have to agree.
pub const ANCHOR_AGREE_MS: f64 = 3.0;

/// How long the clock remembers an arrival: the envelope is the fastest
/// arrival of the last two seconds, long enough that one lucky packet
/// sets it for a while and short enough that a path that got slower is
/// followed two seconds later.
pub const CLOCK_WINDOW_MS: i64 = 2_000;

/// How long after an arrival gap past `STALL_INTERVALS` intervals the
/// estimate may only fall at `CLOCK_FALL_PER_MS` - and only while the
/// window holds nothing but readings from after the gap, which is a stall
/// that outlasted the window: its late burst says nothing about the path,
/// and half a second of the stream that follows it does. A stall's
/// backlog comes back in chunks a round trip apart, each behind a gap of
/// its own, and each of those gaps extends a settling already in effect by
/// as much again, up to `CLOCK_SETTLE_MAX_MS` from the first; a gap on a
/// link that is merely gappy extends nothing, since the window there never
/// empties of what came before.
pub const CLOCK_SETTLE_MS: i64 = 500;

/// The longest one settling lasts from the gap that started it, however
/// many later gaps extend it: the clock's window. By then the window holds
/// nothing from before the stall, and a link that keeps arriving in bursts
/// is simply its path.
pub const CLOCK_SETTLE_MAX_MS: i64 = CLOCK_WINDOW_MS;

/// How fast the clock's estimate may fall toward a slower envelope while
/// it settles after a gap (`CLOCK_SETTLE_MS`), in milliseconds per
/// millisecond of arrivals: 50 ms a second, so a burst of late packets
/// moves it by next to nothing.
pub const CLOCK_FALL_PER_MS: f64 = 0.05;

/// The most one arrival gap counts toward `CLOCK_FALL_PER_MS`: a stall of
/// any length earns the clock a tenth of a second of fall, never the
/// stall's own length - after a stall every packet is late, and the
/// envelope has to wait for the stream to resume to say where it is.
pub const CLOCK_FALL_CREDIT_MS: f64 = 100.0;

/// How much of each arrival's lateness the clock's smoothed jitter
/// reading takes: a tenth, so it settles within a second of snapshots.
pub const JITTER_GAIN: f64 = 0.1;

/// How long arrival lateness is remembered for the delay's percentile.
pub const LATENESS_WINDOW_MS: i64 = 3_000;

/// The lateness percentile the delay covers.
pub const LATENESS_PERCENTILE: f64 = 0.95;

/// Lateness past this many snapshot intervals, on an arrival that ended a
/// gap this wide or came in the burst behind one, is a head-of-line stall
/// rather than jitter; an arrival gap this wide is counted as one.
pub const STALL_INTERVALS: f64 = 3.0;

/// An arrival less than this many snapshot intervals after the one before
/// it came in a burst - what a stall releases at once. Lateness that
/// arrives on the room's cadence instead is the path's own, however
/// large: a route that got slower, which the delay has to cover until the
/// clock's window has followed it.
pub const BURST_INTERVALS: f64 = 0.5;

/// While stalls are at most this share of the lateness window they are
/// isolated and left out of the percentile; past it they are simply the
/// link, and the delay pays for them.
pub const STALL_SHARE_MAX: f64 = 0.2;

/// A smaller delay target is only taken once it is this much smaller, so
/// a percentile sliding by a millisecond does not steer the picture's
/// speed; a larger one is taken at once.
pub const DELAY_NARROW_HYSTERESIS_MS: f64 = 5.0;

/// Render time this far behind its target is taken whole instead of
/// steered toward, and this far ahead of it is held until the target
/// catches up: a quarter of a second at 10 % would be two and a half
/// seconds of fast or slow motion.
pub const RENDER_SNAP_MS: f64 = 250.0;

/// A snapshot the picture never showed - dropped from a full buffer or
/// stepped over by a long frame - whose events reach the replica more
/// than this much later than the delay would have shown them keeps only
/// the ones the replica needs as state (`carries_state`). A quarter of a
/// second is how far render time may fall behind before it jumps, and
/// about as long as a fireball lasts: a lost segment's burst is still
/// shown late, a hidden tab's backlog is not shown at all.
pub const EVENT_STALE_MS: f64 = RENDER_SNAP_MS;

/// Inside this error the picture runs at exactly 1.0x - except past the
/// newest snapshot, where a target behind render time is closed however
/// small the error, since every frame there is a guess.
pub const RATE_DEADBAND_MS: f64 = 1.0;

/// The most the rate strays from 1.0 for a small error: 3 %, below what
/// anyone notices in motion.
pub const RATE_NEAR: f64 = 0.03;

/// The most the rate strays from 1.0 when far off: 10 %.
pub const RATE_FAR: f64 = 0.10;

/// Errors up to this size are steered at no more than `RATE_NEAR`.
pub const RATE_NEAR_MS: f64 = 30.0;

/// Errors from this size are steered at up to `RATE_FAR`; between the two
/// the bound ramps, so the rate never jumps with the error.
pub const RATE_FAR_MS: f64 = 100.0;

/// The rate's gain: the error is closed at `error / RATE_TIME_CONSTANT_MS`
/// per unit of real time, within the bounds above.
pub const RATE_TIME_CONSTANT_MS: f64 = 250.0;

/// How long past the newest snapshot a hull carries on at its last
/// velocity before it dead-blends: a lost segment is ridden out at full
/// speed for about one retransmit.
pub const HULL_EXTRAPOLATE_MS: f64 = 150.0;

/// The dead-blend's half-life: past `HULL_EXTRAPOLATE_MS` a hull's
/// velocity halves every this many milliseconds, so it coasts to a stop a
/// hundred milliseconds later instead of stopping dead.
pub const HULL_BLEND_HALF_LIFE_MS: f64 = 60.0;

/// How long past the newest snapshot a flying shot or a missile carries
/// on along its heading before it holds. Nothing here knows the walls, so
/// the guess is kept to the length of a stall a delay does not cover.
pub const SHOT_EXTRAPOLATE_MS: f64 = 150.0;

/// A correction's error offset halves every this many milliseconds of
/// real time.
pub const ERROR_HALF_LIFE_MS: f64 = 100.0;

/// A correction larger than this is not eased but drawn whole: past a
/// hull's length a glide is a tank sliding across the field.
pub const ERROR_SNAP_PX: f32 = 96.0;

/// A correction smaller than this is drawn whole rather than eased: two
/// pixels is one design pixel of a hull drawn at 2x, so the art cannot
/// show anything finer, and an offset that small would only keep the
/// picture off the room's word for no visible gain.
pub const ERROR_EASE_MIN_PX: f32 = 2.0;

/// The distance a hull may cover between two snapshots beyond twice its
/// top speed before the move is a teleport.
pub const TELEPORT_SLACK_PX: f32 = 16.0;

/// How many snapshots are kept. Render time sits the delay behind the
/// newest - up to `online_interpolation_delay_max_ms` - and a burst after
/// a stall lands several at once, so this is room for both before the
/// oldest is dropped unseen (what of its events is still owed as state is
/// handed over).
const BUFFERED_SNAPSHOTS: usize = 32;

/// The plausible range for a measured interval; anything outside is a
/// gap or a hiccup rather than the room's cadence.
const INTERVAL_RANGE_MS: std::ops::RangeInclusive<f64> = 5.0..=500.0;

/// Arrivals are stamped in whole milliseconds, so this much of any
/// lateness reading is rounding rather than the link.
const ARRIVAL_RESOLUTION_MS: f64 = 1.0;

/// How many tick gaps the measured interval is the median of.
const GAP_HISTORY: usize = 32;

/// How many corrections the error percentile is taken over.
const CORRECTION_HISTORY: usize = 256;

/// An error offset under this, in quarter pixels, rounds to nothing and
/// is dropped.
const ERROR_DROP_QPX: f32 = 0.5;

/// A tick's time on the room's schedule, milliseconds.
pub fn tick_ms(tick: u32) -> f64 {
    tick as f64 * TICK_MS
}

/// What the server's clock reads here, from the arrivals of its stamped
/// messages (for the interpolator, snapshot tick times).
///
/// Each reading is the server's time less the local arrival time: the
/// true offset less that packet's transit. The estimate is the lower
/// envelope of transit - the largest reading of the last
/// `CLOCK_WINDOW_MS` - so it is set by the packets that spent least time
/// queued and a late one moves nothing. The link's time on the wire is
/// part of the delay's floor, not something this tries to measure (that
/// is `net::clock`'s round trip).
///
/// The estimate follows the window's largest reading both ways, except
/// while it settles after an arrival gap (`CLOCK_SETTLE_MS`): a stall
/// longer than the window leaves nothing in it but the stall's late
/// burst, and that may only pull the estimate down at `CLOCK_FALL_PER_MS`.
///
/// Readings are plain milliseconds on a line that does not wrap: the
/// caller unwraps the room's `u32` stamps (`Interpolator` does).
#[derive(Clone, Debug)]
pub struct ServerClock {
    offset_ms: Option<f64>,
    /// The readings in the window, oldest first, with their arrival time.
    window: VecDeque<(i64, f64)>,
    /// The newest arrival, for the fall credit and the gap.
    last_local: Option<i64>,
    /// The arrival that ended the gap the settling in progress started
    /// on: the window holds nothing from before the stall once its front
    /// is at or past it.
    resumed_at: Option<i64>,
    /// The settling lasts until this local time: `CLOCK_SETTLE_MS` past
    /// the latest gap of a backlog, never more than `CLOCK_SETTLE_MAX_MS`
    /// past `resumed_at`.
    settle_until: Option<i64>,
    /// An arrival gap longer than this is a stall.
    gap_ms: f64,
    /// The smoothed lateness of each reading behind the estimate: how far
    /// arrivals stray from the fastest. Zero on a perfect link.
    jitter_ms: f64,
}

impl Default for ServerClock {
    fn default() -> ServerClock {
        ServerClock {
            offset_ms: None,
            window: VecDeque::new(),
            last_local: None,
            resumed_at: None,
            settle_until: None,
            gap_ms: STALL_INTERVALS * SNAPSHOT_INTERVAL_MS,
            jitter_ms: 0.0,
        }
    }
}

impl ServerClock {
    /// A reading of the server's time `server_ms` at local arrival time
    /// `local_ms`. True when it was taken whole as a new clock.
    pub fn observe_ms(&mut self, server_ms: f64, local_ms: i64) -> bool {
        let reading = server_ms - local_ms as f64;
        let Some(offset) = self.offset_ms else {
            self.take_whole(reading, local_ms);
            return true;
        };
        if reading - offset > CLOCK_SNAP_MS {
            // Earlier than any packet could have come: another clock, and
            // nothing measured against the old one says anything.
            self.take_whole(reading, local_ms);
            return true;
        }
        let gap = self.last_local.map_or(0, |at| (local_ms - at).max(0));
        let credit = (gap as f64).min(CLOCK_FALL_CREDIT_MS);
        self.last_local = Some(self.last_local.map_or(local_ms, |at| at.max(local_ms)));
        self.window.push_back((local_ms, reading));
        let horizon = local_ms - CLOCK_WINDOW_MS;
        while self.window.len() > 1 && self.window.front().is_some_and(|&(at, _)| at < horizon) {
            self.window.pop_front();
        }
        // A stall's backlog comes back in chunks a round trip apart, each
        // behind a gap of its own: the first gap starts a settling, and a
        // later one extends it without moving its start - but only once it
        // is in effect, the window holding nothing from before that first
        // gap. On a link that is merely gappy the window never empties, the
        // pending settling runs out, and nothing settles.
        if gap as f64 > self.gap_ms {
            let pending = self.settle_until.is_some_and(|until| local_ms < until);
            let in_effect = self.resumed_at.is_some_and(|at| self.window.front().is_some_and(|&(first, _)| first >= at));
            match (pending, self.resumed_at) {
                (true, Some(start)) if in_effect => {
                    self.settle_until = Some((local_ms + CLOCK_SETTLE_MS).min(start + CLOCK_SETTLE_MAX_MS));
                }
                (true, _) => {}
                (false, _) => {
                    self.resumed_at = Some(local_ms);
                    self.settle_until = Some(local_ms + CLOCK_SETTLE_MS);
                }
            }
        }
        let envelope = self.window.iter().map(|&(_, r)| r).fold(f64::NEG_INFINITY, f64::max);
        // Settling: the window holds only what came after a gap, and the
        // gap is recent. Otherwise the window's best is the path.
        let settling = self.settling(local_ms);
        let next = if envelope >= offset || !settling {
            envelope
        } else {
            (offset - credit * CLOCK_FALL_PER_MS).max(envelope)
        };
        self.offset_ms = Some(next);
        let late = (next - reading).max(0.0);
        self.jitter_ms += (late - self.jitter_ms) * JITTER_GAIN;
        false
    }

    /// Whether a settling is in effect at `local_ms`: one is pending and
    /// the window holds nothing from before the gap that started it.
    fn settling(&self, local_ms: i64) -> bool {
        self.settle_until.is_some_and(|until| local_ms < until)
            && self.resumed_at.is_some_and(|at| self.window.front().is_some_and(|&(first, _)| first >= at))
    }

    fn take_whole(&mut self, reading: f64, local_ms: i64) {
        self.offset_ms = Some(reading);
        self.window.clear();
        self.window.push_back((local_ms, reading));
        self.last_local = Some(local_ms);
        self.resumed_at = None;
        self.settle_until = None;
        self.jitter_ms = 0.0;
    }

    /// The arrival gap past which the readings that follow are a stall's
    /// burst (`STALL_INTERVALS` of the room's measured cadence).
    pub fn set_gap_ms(&mut self, gap_ms: f64) {
        self.gap_ms = gap_ms;
    }

    /// Forget every reading: the next one is taken whole.
    pub fn reset(&mut self) {
        *self = ServerClock { gap_ms: self.gap_ms, ..ServerClock::default() };
    }

    /// How late a reading of `server_ms` arriving at `local_ms` is behind
    /// the envelope, milliseconds; 0 before a first reading.
    pub fn lateness(&self, server_ms: f64, local_ms: i64) -> f64 {
        self.offset_ms.map_or(0.0, |offset| (offset - (server_ms - local_ms as f64)).max(0.0))
    }

    /// The smoothed lateness of arrivals behind the envelope, in
    /// milliseconds.
    pub fn jitter_ms(&self) -> f64 {
        self.jitter_ms
    }

    /// What the server's clock reads at local time `local_ms`; `None`
    /// until a first reading has been seen.
    pub fn now(&self, local_ms: i64) -> Option<f64> {
        self.offset_ms.map(|offset| local_ms as f64 + offset)
    }

    /// The estimate itself, for a status line.
    pub fn offset_ms(&self) -> Option<f64> {
        self.offset_ms
    }

    /// The local time of the newest reading; `None` before a first one.
    pub fn last_local(&self) -> Option<i64> {
        self.last_local
    }
}

/// The replica's view of one rendered frame: the snapshot to write into
/// it, and how far past that snapshot's tick the picture stands.
#[derive(Clone, Debug)]
pub struct Frame {
    /// The blend, to be handed to `net::apply::snapshot`. Its `tick` is
    /// the bracket's near end and its `events` are empty except on the
    /// frame that end was reached.
    pub snapshot: Snapshot,
    /// Seconds past `snapshot.tick`: the round clock the replica draws
    /// its water, fires and sprite loops from is
    /// `tick * PHYSICS_FIXED_DT + ahead`.
    pub ahead: f32,
    /// The picture is running past the newest snapshot on last-known
    /// velocities rather than between two of them.
    pub extrapolated: bool,
    /// Tanks (by id) that jumped rather than drove onto this frame's near
    /// end - a portal, a `Placed`, a move longer than driving allows - and
    /// are drawn there whole: the caller lifts their tread marks rather
    /// than pressing a line across the field.
    pub snapped: Vec<u16>,
}

/// What the interpolator is doing, for `status.round.interpolation` and
/// the rig's tests (docs/online-coop-prd.md §4.12, "Measured", §4.16).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InterpReport {
    /// The delay in force, milliseconds behind the server's clock.
    pub delay_ms: f64,
    /// Where the delay is heading: an interval, a frame and the lateness
    /// percentile, between the floor and the cap.
    pub target_ms: f64,
    /// The smoothed lateness of arrivals behind the clock's envelope.
    pub jitter_ms: f64,
    /// The room's measured cadence.
    pub interval_ms: f64,
    /// Snapshots waiting to be drawn.
    pub buffered: usize,
    /// Frames drawn past the newest snapshot on last-known velocities:
    /// each one is a packet the delay did not cover.
    pub extrapolated_frames: u64,
    /// The median lateness behind the envelope, stalls left out.
    pub lateness_p50_ms: f64,
    /// The 95th percentile of lateness, stalls left out while isolated:
    /// the term the delay covers.
    pub lateness_p95_ms: f64,
    /// Arrival gaps wider than `STALL_INTERVALS` intervals: head-of-line
    /// stalls ridden out on extrapolation.
    pub stalls: u64,
    /// The playout clock's speed on the last frame, 1.0 on a settled link
    /// and 0 while render time holds for its target to catch up.
    pub rate: f64,
    /// Corrections eased by an error offset.
    pub corrections: u64,
    /// The 95th percentile of those corrections' size, pixels.
    pub error_p95_px: f64,
    /// Cosmetic events left out of snapshots handed over too late to show
    /// (`EVENT_STALE_MS`): a hidden tab's backlog, a buffer that overflowed.
    pub stale_events: u64,
}

/// One arrival's reading of the clock, for the lateness percentile.
///
/// Its lateness is how far it fell short of the clock's estimate: the
/// estimate when it arrived, or the estimate now if that has since fallen.
/// A path that got slower brings the estimate down and every reading that
/// looked late against the old path with it; a faster reading raising the
/// estimate later does not make the earlier ones late after the fact.
#[derive(Clone, Copy, Debug)]
struct Reading {
    /// Local arrival time.
    at_ms: i64,
    /// The room's send time (`server_ms`) less arrival time.
    reading: f64,
    /// The clock's estimate just after this reading was taken.
    estimate: f64,
    /// The snapshot's own `server_ms - tick time`: past the anchor, the
    /// room sent it late.
    epoch: f64,
    /// It ended an arrival gap of `STALL_INTERVALS` or came less than
    /// `BURST_INTERVALS` behind the arrival before it: the shape of a
    /// head-of-line stall, so its lateness may be one.
    bunched: bool,
}

/// Which entity an error offset belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Tank(u16),
    Shot(u16),
    Missile(u16),
}

/// The snapshots that have arrived and the clocks that order them.
#[derive(Clone, Debug)]
pub struct Interpolator {
    buffer: VecDeque<Snapshot>,
    clock: ServerClock,
    /// The last `server_ms` stamp as it came off the wire, and that stamp
    /// unwrapped: the room's clock is a `u32` of milliseconds that wraps
    /// every 49.7 days, so each stamp moves the unwrapped time on by its
    /// signed distance from the one before (`unwrap_stamp`).
    last_raw: Option<u32>,
    extended: f64,
    /// The newest tick whose events have been handed to the replica.
    released: Option<u32>,
    /// Events of snapshots dropped from a full buffer before render time
    /// reached them, owed to the next frame - only the ones the replica
    /// needs as state, since such a snapshot is long past.
    orphaned: Vec<WireEvent>,
    /// The seat this client plays: its `Fired` confirm its presses, so a
    /// stale snapshot keeps them where it drops everyone else's.
    seat: Option<u8>,
    /// Cosmetic events left out of stale snapshots, for the report.
    stale_events: u64,
    /// The last near end render time left behind: the second point of
    /// every velocity estimate past the newest snapshot.
    previous: Option<Snapshot>,
    /// `server_ms - tick time` of the last `ANCHOR_SNAPSHOTS` snapshots,
    /// and the lowest of them: the round's anchor, which turns the
    /// clock's wall time into tick time.
    epochs: VecDeque<f64>,
    anchor: Option<f64>,
    /// Recent tick gaps between consecutive snapshots.
    gaps: VecDeque<u32>,
    /// The room's measured cadence: the median gap, in milliseconds.
    interval_ms: f64,
    /// Each recent arrival's reading of the clock (its tick time less its
    /// arrival time), for the lateness percentile (`Reading`).
    readings: VecDeque<Reading>,
    lateness_p50: f64,
    lateness_p95: f64,
    /// The delay the lateness asks for before the floor and the cap, with
    /// the narrowing hysteresis applied.
    want_ms: f64,
    /// The newest arrival, for the stall count.
    last_arrival: Option<i64>,
    stalls: u64,
    /// Render time on the room's tick schedule; `None` until the first
    /// frame, and again after a round starts over or a welcome.
    render: Option<f64>,
    /// Render time stands still until its target catches up with it: a
    /// new clock, or a target that fell more than `RENDER_SNAP_MS` behind.
    holding: bool,
    /// The local time of the last frame.
    rendered_at: Option<i64>,
    /// The playout clock's speed on the last frame.
    rate: f64,
    /// The delay in force on the last frame.
    delay_ms: Option<f64>,
    /// The near end's tick on the last frame, if that frame ran past the
    /// newest snapshot: the cue that the next arrivals may correct it.
    extrapolated_from: Option<u32>,
    /// Frames that ran on extrapolation, for the report.
    extrapolated_frames: u64,
    /// Where each entity was drawn last frame, quarter pixels.
    drawn: BTreeMap<Key, (f32, f32)>,
    /// Each entity's error offset, quarter pixels.
    offsets: BTreeMap<Key, (f32, f32)>,
    /// The size of recent corrections, pixels.
    correction_px: VecDeque<f32>,
    corrections: u64,
}

impl Default for Interpolator {
    fn default() -> Interpolator {
        Interpolator {
            buffer: VecDeque::new(),
            clock: ServerClock::default(),
            last_raw: None,
            extended: 0.0,
            released: None,
            orphaned: Vec::new(),
            seat: None,
            stale_events: 0,
            previous: None,
            epochs: VecDeque::new(),
            anchor: None,
            gaps: VecDeque::new(),
            interval_ms: SNAPSHOT_INTERVAL_MS,
            readings: VecDeque::new(),
            lateness_p50: 0.0,
            lateness_p95: 0.0,
            want_ms: SNAPSHOT_INTERVAL_MS + FRAME_MS,
            last_arrival: None,
            stalls: 0,
            render: None,
            holding: false,
            rendered_at: None,
            rate: 1.0,
            delay_ms: None,
            extrapolated_from: None,
            extrapolated_frames: 0,
            drawn: BTreeMap::new(),
            offsets: BTreeMap::new(),
            correction_px: VecDeque::new(),
            corrections: 0,
        }
    }
}

impl Interpolator {
    /// Start again from `baseline`, the snapshot inside a `Welcome`:
    /// `net::apply::welcome` has already written it into the replica,
    /// events included, so it is the near end and nothing of it is
    /// released twice. The link's clock survives unless the stamp says it
    /// is another server or its newest reading is older than
    /// `CLOCK_WINDOW_MS` - a lobby, the wait for a rematch - which says
    /// nothing of the path now; the anchor starts again, and so does the
    /// picture, at its target.
    ///
    /// `local_ms` is when the welcome came off the socket, not the frame
    /// that read it: the clock is the link's.
    pub fn restart(&mut self, baseline: &Snapshot, local_ms: i64) {
        self.buffer.clear();
        self.orphaned.clear();
        self.previous = None;
        self.released = Some(baseline.tick);
        let tick_time = tick_ms(baseline.tick);
        let server_ms = self.unwrap_stamp(baseline.server_ms);
        // A welcome is a round the room may not have started yet (a
        // waiting room's is tick 0 of a round that begins later) or one on
        // another schedule: the anchor starts again from it. The link is
        // the same link - unless the stamp says another server, or the
        // link has been silent for longer than the clock remembers.
        self.epochs.clear();
        self.anchor = None;
        self.note_anchor(server_ms - tick_time);
        let reading = server_ms - local_ms as f64;
        let elsewhere = self.clock.offset_ms().is_some_and(|offset| (reading - offset).abs() > CLOCK_SNAP_MS);
        let stale = self.clock.last_local().is_some_and(|at| local_ms - at > CLOCK_WINDOW_MS);
        if elsewhere || stale {
            self.clock.reset();
        }
        self.observe(server_ms, local_ms);
        self.reset_picture();
        self.buffer.push_back(baseline.clone());
    }

    /// `raw`, a room's `server_ms` stamp, on a line that does not wrap:
    /// the unwrapped time of the stamp before it moved on by the signed
    /// distance between the two, so the hub clock's wrap at `u32::MAX` is
    /// a step of a millisecond like any other. The first stamp is taken as
    /// it is.
    fn unwrap_stamp(&mut self, raw: u32) -> f64 {
        self.extended = match self.last_raw {
            Some(last) => self.extended + raw.wrapping_sub(last) as i32 as f64,
            None => raw as f64,
        };
        self.last_raw = Some(raw);
        self.extended
    }

    /// The seat this client plays, whose `Fired` a stale snapshot keeps.
    pub fn set_seat(&mut self, seat: Option<u8>) {
        self.seat = seat;
    }

    /// A whole snapshot from the room, which came off the socket at local
    /// time `at_ms`.
    ///
    /// The client hands snapshots over in tick order, so a repeat of the
    /// newest is dropped and a tick *behind* it is the round having
    /// started over (the server's counter rewinds on `Game::init`):
    /// nothing buffered describes that world any more, so the buffer, the
    /// anchor and the picture start again from it.
    pub fn accept(&mut self, snapshot: Snapshot, at_ms: i64) {
        match self.buffer.back() {
            Some(newest) if snapshot.tick == newest.tick => return,
            Some(newest) if snapshot.tick < newest.tick => {
                self.buffer.clear();
                self.orphaned.clear();
                self.previous = None;
                self.released = None;
                self.epochs.clear();
                self.anchor = None;
                self.readings.clear();
                self.reset_picture();
            }
            Some(newest) => {
                let gap = snapshot.tick - newest.tick;
                if INTERVAL_RANGE_MS.contains(&(gap as f64 * TICK_MS)) {
                    self.gaps.push_back(gap);
                    while self.gaps.len() > GAP_HISTORY {
                        self.gaps.pop_front();
                    }
                    let mut sorted: Vec<u32> = self.gaps.iter().copied().collect();
                    sorted.sort_unstable();
                    self.interval_ms = sorted[sorted.len() / 2] as f64 * TICK_MS;
                }
            }
            None => {}
        }
        let tick_time = tick_ms(snapshot.tick);
        let server_ms = self.unwrap_stamp(snapshot.server_ms);
        let epoch = server_ms - tick_time;
        self.note_anchor(epoch);
        let gap = self.last_arrival.map(|last| (at_ms - last) as f64);
        let stalled = gap.is_some_and(|gap| gap > STALL_INTERVALS * self.interval_ms);
        if stalled {
            self.stalls += 1;
        }
        let bunched = stalled || gap.is_some_and(|gap| gap < BURST_INTERVALS * self.interval_ms);
        self.last_arrival = Some(self.last_arrival.map_or(at_ms, |last| last.max(at_ms)));
        self.clock.set_gap_ms(STALL_INTERVALS * self.interval_ms);
        self.observe(server_ms, at_ms);
        let reading = server_ms - at_ms as f64;
        let estimate = self.clock.offset_ms().unwrap_or(reading);
        self.readings.push_back(Reading { at_ms, reading, estimate, epoch, bunched });
        let horizon = at_ms - LATENESS_WINDOW_MS;
        while self.readings.front().is_some_and(|r| r.at_ms < horizon) {
            self.readings.pop_front();
        }
        self.refresh_lateness();
        self.buffer.push_back(snapshot);
        let cutoff = self.stale_before(self.server_now(at_ms));
        let mut owed = std::mem::take(&mut self.orphaned);
        while self.buffer.len() > BUFFERED_SNAPSHOTS {
            let dropped = self.buffer.pop_front().expect("the buffer is over its cap");
            if self.owed(dropped.tick) {
                self.stale_events += hand_over(&dropped, cutoff, self.seat, &mut owed);
            }
            self.previous = Some(dropped);
        }
        self.orphaned = owed;
    }

    /// One reading of the link's clock, at the room's send time
    /// `server_ms` (unwrapped); a new clock starts the lateness over and
    /// holds the picture.
    fn observe(&mut self, server_ms: f64, at_ms: i64) {
        if self.clock.observe_ms(server_ms, at_ms) {
            self.new_clock();
        }
    }

    /// A snapshot's `server_ms - tick time` into the anchor: the lowest of
    /// the last `ANCHOR_SNAPSHOTS`, so a catch-up burst's late ticks never
    /// move it. One past `CLOCK_SNAP_MS` from it - a room that paused and
    /// resumed - is taken at once, and so is a run of `ANCHOR_RUN` equally
    /// late ones - a schedule that restarted; either way an anchor that
    /// moves forward by more than `ANCHOR_HOLD_MS` holds the picture until
    /// the target behind it catches up, the time the room stood still.
    fn note_anchor(&mut self, epoch: f64) {
        let Some(anchor) = self.anchor else {
            self.epochs.push_back(epoch);
            self.anchor = Some(epoch);
            return;
        };
        if (epoch - anchor).abs() > CLOCK_SNAP_MS {
            self.epochs.clear();
        }
        self.epochs.push_back(epoch);
        while self.epochs.len() > ANCHOR_SNAPSHOTS {
            self.epochs.pop_front();
        }
        let mut low = self.epochs.iter().copied().fold(f64::INFINITY, f64::min);
        // A run of equally late ticks is a schedule that restarted: take
        // it now, not once the on-time ones have left the window.
        let run: Vec<f64> = self.epochs.iter().rev().take(ANCHOR_RUN).copied().collect();
        if run.len() == ANCHOR_RUN {
            let (lo, hi) = run.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &e| (lo.min(e), hi.max(e)));
            if hi - lo <= ANCHOR_AGREE_MS && lo - anchor > ANCHOR_HOLD_MS {
                low = lo;
                let keep = self.epochs.len().saturating_sub(ANCHOR_RUN);
                self.epochs.drain(..keep);
            }
        }
        if low - anchor > ANCHOR_HOLD_MS {
            self.holding = true;
        }
        self.anchor = Some(low);
    }

    /// The room's tick time at local time `local_ms`: the link's clock
    /// less the round's anchor. `None` before both are known.
    fn server_now(&self, local_ms: i64) -> Option<f64> {
        Some(self.clock.now(local_ms)? - self.anchor?)
    }

    /// Another clock: lateness measured against the old one means nothing.
    /// Render time is a place on the tick schedule, which the new clock
    /// does not move, so it is kept - held until the new target reaches
    /// it rather than rewound to a target behind it, or taken forward to
    /// one ahead.
    fn new_clock(&mut self) {
        self.readings.clear();
        self.holding = true;
    }

    /// The picture starts again - the round started over, or a welcome
    /// built a new replica: render time is taken whole on the next frame
    /// and nothing drawn before is eased from.
    fn reset_picture(&mut self) {
        self.render = None;
        self.rendered_at = None;
        self.rate = 1.0;
        self.holding = false;
        self.drop_corrections();
    }

    /// Nothing drawn before is eased from: the picture jumped.
    fn drop_corrections(&mut self) {
        self.extrapolated_from = None;
        self.drawn.clear();
        self.offsets.clear();
    }

    /// The tick time before which a snapshot handed over at server time
    /// `now` is stale (`EVENT_STALE_MS`); `None` before the clock has a
    /// reading, when nothing is.
    fn stale_before(&self, now: Option<f64>) -> Option<f64> {
        now.map(|now| now - self.target_delay_ms() - EVENT_STALE_MS)
    }

    /// The lateness percentiles and the delay they ask for. A reading is a
    /// stall's when it is later than `STALL_INTERVALS` intervals and came
    /// bunched - at the head of a gap or in the burst behind it; while
    /// those are isolated they are left out.
    fn refresh_lateness(&mut self) {
        let Some(offset) = self.clock.offset_ms() else { return };
        let anchor = self.anchor.unwrap_or(0.0);
        let limit = STALL_INTERVALS * self.interval_ms;
        let late: Vec<(f64, bool)> = self
            .readings
            .iter()
            .map(|r| {
                let sent_late = (r.epoch - anchor).max(0.0);
                let late = (r.estimate.min(offset) - r.reading + sent_late - ARRIVAL_RESOLUTION_MS).max(0.0);
                (late, r.bunched && late > limit)
            })
            .collect();
        let stalled = late.iter().filter(|&&(_, stall)| stall).count();
        let isolated = (stalled as f64) <= STALL_SHARE_MAX * late.len() as f64;
        let mut values: Vec<f64> =
            late.iter().filter(|&&(_, stall)| !(stall && isolated)).map(|&(late, _)| late).collect();
        values.sort_by(f64::total_cmp);
        self.lateness_p50 = percentile(&values, 0.5);
        self.lateness_p95 = percentile(&values, LATENESS_PERCENTILE);
        let want = self.interval_ms + FRAME_MS + self.lateness_p95;
        if want > self.want_ms || self.want_ms - want > DELAY_NARROW_HYSTERESIS_MS {
            self.want_ms = want;
        }
    }

    /// The frame to draw at local time `local_ms`, or `None` before the
    /// first snapshot has arrived.
    ///
    /// Advances the playout clock, eases any correction the newest
    /// arrivals made to what was drawn last frame, moves the near end past
    /// every snapshot render time has reached - handing over the events of
    /// each exactly once, only the ones kept as state from a snapshot it
    /// stepped over that is long past (`EVENT_STALE_MS`) - and blends
    /// toward the far end.
    pub fn sample(&mut self, local_ms: i64) -> Option<Frame> {
        if self.buffer.is_empty() {
            return None;
        }
        let now = self.server_now(local_ms)?;
        let elapsed = self.rendered_at.map_or(0.0, |at| (local_ms - at).max(0) as f64);
        let previous_render = self.render;
        let target = now - self.target_delay_ms();
        let (render, fresh) = self.advance(target, elapsed);
        self.render = Some(render);
        self.rendered_at = Some(local_ms);
        self.delay_ms = Some(now - render);

        // Newer snapshots have come in since a frame that ran past the
        // newest one: what they say about that frame's instant replaces
        // what was guessed, and the difference is eased off.
        if !fresh
            && let (Some(from), Some(then)) = (self.extrapolated_from, previous_render)
            && self.buffer.back().is_some_and(|newest| newest.tick > from)
        {
            self.correct(then);
        }
        let fade = 0.5f32.powf((elapsed / ERROR_HALF_LIFE_MS) as f32);
        self.offsets.retain(|_, (x, y)| {
            *x *= fade;
            *y *= fade;
            x.hypot(*y) >= ERROR_DROP_QPX
        });

        let mut events: Vec<WireEvent> = std::mem::take(&mut self.orphaned);
        let mut snapped: BTreeSet<u16> = BTreeSet::new();
        let cutoff = self.stale_before(Some(now));
        while self.buffer.len() > 1 && tick_ms(self.buffer[1].tick) <= render {
            let passed = self.buffer.pop_front().expect("the buffer holds two");
            // A frame long enough to step over a whole snapshot leaves one
            // that was never the near end; its events are still owed.
            if self.owed(passed.tick) {
                self.stale_events += hand_over(&passed, cutoff, self.seat, &mut events);
            }
            snapped.extend(teleported(&passed, &self.buffer[0]));
            // A shot that came out of a portal starts afresh at the exit:
            // no correction carried over from the far side of the field.
            for id in shots_teleported(&self.buffer[0].events) {
                self.offsets.remove(&Key::Shot(id));
            }
            self.previous = Some(passed);
        }
        // The near end is the snapshot on screen, however far back its
        // tick: its events are this frame's.
        let front_tick = self.buffer.front()?.tick;
        if self.owed(front_tick) {
            events.extend(self.buffer[0].events.iter().cloned());
        }
        let (mut snapshot, extrapolated, ahead_ms) = self.scene(render);
        if extrapolated {
            self.extrapolated_frames += 1;
            self.extrapolated_from = Some(front_tick);
        } else {
            self.extrapolated_from = None;
        }
        for id in &snapped {
            self.offsets.remove(&Key::Tank(*id));
        }
        self.apply_offsets(&mut snapshot);
        snapshot.events = events;
        Some(Frame {
            snapshot,
            ahead: (ahead_ms / 1000.0) as f32,
            extrapolated,
            snapped: snapped.into_iter().collect(),
        })
    }

    /// Render time `elapsed` milliseconds after the last frame, steered
    /// toward `target`, and whether the picture starts afresh on it.
    ///
    /// Never backwards: a target behind render time - after a new clock,
    /// or more than `RENDER_SNAP_MS` behind - holds it where it stands
    /// until the target catches up, and a target more than
    /// `RENDER_SNAP_MS` ahead is jumped to. In between, the bounded rate;
    /// while the picture is past the newest snapshot and the target behind
    /// it, the error is closed bounded by `RATE_FAR` rather than the
    /// error-ramped bound, and without the deadband.
    fn advance(&mut self, target: f64, elapsed: f64) -> (f64, bool) {
        let Some(render) = self.render else {
            self.rate = 1.0;
            self.holding = false;
            return (target, true);
        };
        let err = target - (render + elapsed);
        if self.holding || err < -RENDER_SNAP_MS {
            if target < render {
                self.holding = true;
                self.rate = 0.0;
                return (render, false);
            }
            self.holding = false;
        } else if err <= RENDER_SNAP_MS {
            // Past the newest snapshot the picture is a guess, so a target
            // behind it is closed in proportion to the error bounded by
            // `RATE_FAR` rather than the error-ramped bound, and without the
            // deadband: every frame saved is a frame less guessed.
            let guessing = self.buffer.back().is_some_and(|newest| render > tick_ms(newest.tick));
            self.rate = if guessing && err < 0.0 {
                (1.0 + err / RATE_TIME_CONSTANT_MS).max(1.0 - RATE_FAR)
            } else {
                rate_for(err)
            };
            return (render + elapsed * self.rate, false);
        }
        // Caught up with a held picture, or far behind the target: taken
        // whole, and a jump drops what was being eased.
        self.rate = 1.0;
        let jumped = target - render > RENDER_SNAP_MS;
        if jumped {
            self.drop_corrections();
        }
        (target, jumped)
    }

    /// The picture at render time `t` from what is buffered now, without
    /// moving anything: the blend of the bracket around `t`, or the newest
    /// snapshot carried past its tick. Returns the snapshot, whether it is
    /// extrapolated, and how far past the near end `t` is, milliseconds.
    fn scene(&self, t: f64) -> (Snapshot, bool, f64) {
        let i = self.buffer.iter().rposition(|s| tick_ms(s.tick) <= t).unwrap_or(0);
        let from = &self.buffer[i];
        let ahead = (t - tick_ms(from.tick)).max(0.0);
        match self.buffer.get(i + 1) {
            Some(to) => {
                let span = (tick_ms(to.tick) - tick_ms(from.tick)).max(1.0);
                let alpha = (ahead / span).clamp(0.0, 1.0);
                (blend(from, to, alpha as f32), false, ahead.min(span))
            }
            None => {
                let before = if i > 0 { self.buffer.get(i - 1) } else { self.previous.as_ref() };
                (extrapolate(from, before, ahead), ahead > 0.0, ahead)
            }
        }
    }

    /// The frame at render time `then` was drawn from a guess; set each
    /// entity's error offset to where it was drawn less where the newer
    /// snapshots put it at that instant.
    fn correct(&mut self, then: f64) {
        let (truth, _, _) = self.scene(then);
        let mut fresh: BTreeMap<Key, (f32, f32)> = BTreeMap::new();
        let mut consider = |key: Key, x: i16, y: i16, drawn: &BTreeMap<Key, (f32, f32)>| {
            if let Some(&(dx, dy)) = drawn.get(&key) {
                fresh.insert(key, (dx - x as f32, dy - y as f32));
            }
        };
        for t in &truth.tanks {
            consider(Key::Tank(t.id), t.x, t.y, &self.drawn);
        }
        for s in truth.shots.iter().filter(|s| flying(s)) {
            consider(Key::Shot(s.id), s.x, s.y, &self.drawn);
        }
        for m in &truth.missiles {
            consider(Key::Missile(m.id), m.x, m.y, &self.drawn);
        }
        let steps = crate::net::wire::POSITION_STEPS_PER_PX;
        let (ease_qpx, snap_qpx) = (ERROR_EASE_MIN_PX * steps, ERROR_SNAP_PX * steps);
        for (key, (x, y)) in fresh {
            let size = x.hypot(y);
            if size < ease_qpx || size > snap_qpx {
                self.offsets.remove(&key);
                continue;
            }
            self.offsets.insert(key, (x, y));
            self.corrections += 1;
            self.correction_px.push_back(size / steps);
            while self.correction_px.len() > CORRECTION_HISTORY {
                self.correction_px.pop_front();
            }
        }
    }

    /// Add each entity's error offset to its position, forget the offsets
    /// of entities that are gone (and of shots out of flight), and
    /// remember where everything was drawn.
    fn apply_offsets(&mut self, snapshot: &mut Snapshot) {
        let mut live: BTreeSet<Key> = BTreeSet::new();
        self.drawn.clear();
        for t in &mut snapshot.tanks {
            let key = Key::Tank(t.id);
            live.insert(key);
            shift(&self.offsets, key, &mut t.x, &mut t.y);
            self.drawn.insert(key, (t.x as f32, t.y as f32));
        }
        for s in &mut snapshot.shots {
            let key = Key::Shot(s.id);
            if flying(s) {
                live.insert(key);
                shift(&self.offsets, key, &mut s.x, &mut s.y);
            }
            self.drawn.insert(key, (s.x as f32, s.y as f32));
        }
        for m in &mut snapshot.missiles {
            let key = Key::Missile(m.id);
            live.insert(key);
            shift(&self.offsets, key, &mut m.x, &mut m.y);
            self.drawn.insert(key, (m.x as f32, m.y as f32));
        }
        self.offsets.retain(|key, _| live.contains(key));
    }

    /// Where the delay is heading: one interval, one frame and the
    /// lateness percentile (stalls left out while isolated), between
    /// `online_interpolation_delay_ms` and
    /// `online_interpolation_delay_max_ms`. With
    /// `online_interpolation_adaptive` off it is the floor alone.
    pub fn target_delay_ms(&self) -> f64 {
        let t = tuning();
        let floor = t.online_interpolation_delay_ms as f64;
        if !t.online_interpolation_adaptive {
            return floor;
        }
        self.want_ms.min(t.online_interpolation_delay_max_ms as f64).max(floor)
    }

    /// The delay in force, milliseconds: the target until a frame has
    /// been sampled, then how far behind the clock the last frame stood.
    pub fn delay_ms(&self) -> f64 {
        self.delay_ms.unwrap_or_else(|| self.target_delay_ms())
    }

    /// Render time at local time `local_ms`: the last frame's, run on at
    /// its rate, or the target before a first frame.
    fn render_at(&self, local_ms: i64) -> Option<f64> {
        match (self.render, self.rendered_at) {
            (Some(render), Some(at)) => Some(render + (local_ms - at).max(0) as f64 * self.rate),
            _ => Some(self.server_now(local_ms)? - self.target_delay_ms()),
        }
    }

    /// The readings a status line or a test wants.
    pub fn report(&self) -> InterpReport {
        let mut sizes: Vec<f64> = self.correction_px.iter().map(|&s| s as f64).collect();
        sizes.sort_by(f64::total_cmp);
        InterpReport {
            delay_ms: self.delay_ms(),
            target_ms: self.target_delay_ms(),
            jitter_ms: self.clock.jitter_ms(),
            interval_ms: self.interval_ms,
            buffered: self.buffer.len(),
            extrapolated_frames: self.extrapolated_frames,
            lateness_p50_ms: self.lateness_p50,
            lateness_p95_ms: self.lateness_p95,
            stalls: self.stalls,
            rate: self.rate,
            corrections: self.corrections,
            error_p95_px: percentile(&sizes, 0.95),
            stale_events: self.stale_events,
        }
    }

    /// Whether the last frame ran past the newest snapshot on last-known
    /// velocities: a picture that is a guess, not the room's word.
    pub fn extrapolating(&self) -> bool {
        self.extrapolated_from.is_some()
    }

    /// Whether tank `id` was drawn off its snapshots on the last frame by
    /// an easing correction (`ERROR_HALF_LIFE_MS`).
    pub fn easing(&self, id: u16) -> bool {
        self.offsets.contains_key(&Key::Tank(id))
    }

    /// Whether `tick`'s events still have to be handed over, marking them
    /// handed over if they do. A tick at or before the last released one
    /// has already been drawn; a rewind clears the mark in `accept`,
    /// since that is a new round.
    fn owed(&mut self, tick: u32) -> bool {
        if self.released.is_some_and(|released| tick <= released) {
            return false;
        }
        self.released = Some(tick);
        true
    }

    /// The room's measured cadence in milliseconds.
    pub fn interval_ms(&self) -> f64 {
        self.interval_ms
    }

    /// How many snapshots are waiting to be drawn.
    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }

    /// The clock the render time is steered by.
    pub fn clock(&self) -> &ServerClock {
        &self.clock
    }

    /// The newest snapshot's tick, whether or not it has been drawn yet.
    pub fn newest_tick(&self) -> Option<u32> {
        self.buffer.back().map(|s| s.tick)
    }

    /// The newest snapshot itself: the room's most recent word.
    pub fn newest(&self) -> Option<&Snapshot> {
        self.buffer.back()
    }

    /// How far ahead of the picture the newest snapshot stands, in
    /// milliseconds: the depth of the buffer the delay is buying. It
    /// hovers around the delay on a steady link, dips toward zero when
    /// packets are late, and goes negative while the picture runs on
    /// extrapolation.
    pub fn lead_ms(&self, local_ms: i64) -> Option<f64> {
        let render = self.render_at(local_ms)?;
        Some(tick_ms(self.buffer.back()?.tick) - render)
    }
}

/// The playout clock's speed for an error of `err_ms` (target less where
/// render time would be at 1.0x): 1.0 inside the deadband, otherwise
/// proportional, bounded by `RATE_NEAR` for small errors and ramping to
/// `RATE_FAR` for large ones.
fn rate_for(err_ms: f64) -> f64 {
    if err_ms.abs() <= RATE_DEADBAND_MS {
        return 1.0;
    }
    let reach = ((err_ms.abs() - RATE_NEAR_MS) / (RATE_FAR_MS - RATE_NEAR_MS)).clamp(0.0, 1.0);
    let bound = RATE_NEAR + (RATE_FAR - RATE_NEAR) * reach;
    1.0 + (err_ms / RATE_TIME_CONSTANT_MS).clamp(-bound, bound)
}

/// The nearest-rank `q` percentile of sorted `values`; 0 when empty.
fn percentile(values: &[f64], q: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values[((values.len() - 1) as f64 * q).round() as usize]
}

/// Append `snapshot`'s events to `out`: all of them while it is on time,
/// only the ones `carries_state` keeps once its tick is before `cutoff`
/// (`EVENT_STALE_MS`). Returns how many were left out.
fn hand_over(snapshot: &Snapshot, cutoff: Option<f64>, seat: Option<u8>, out: &mut Vec<WireEvent>) -> u64 {
    if cutoff.is_none_or(|cutoff| tick_ms(snapshot.tick) >= cutoff) {
        out.extend(snapshot.events.iter().cloned());
        return 0;
    }
    let before = out.len();
    out.extend(snapshot.events.iter().filter(|e| carries_state(e, seat)).cloned());
    (snapshot.events.len() - (out.len() - before)) as u64
}

/// Whether the replica needs `event` as state however late it comes, as
/// opposed to a cosmetic that is only worth showing on time.
///
/// Kept: a round starting over (`net::apply` re-inits on it) or ending; a
/// tile's death (the tile family says nothing of a tile that is gone); a
/// fuel drum's launch (its flight is no snapshot family); a hull moved by
/// the room - `Placed`, `Teleported`, `TankEntered` - and a `Shoved`, which
/// the own hull takes as state; and this seat's `Fired`, which confirms a
/// press the client is holding a provisional shot for - dropping it would
/// strand the press. Everything else - blasts, wrecks, hits, flashes,
/// other seats' and enemies' muzzles, beams - is how something looked at
/// the moment, and the moment is gone. With no seat known, every `Fired`
/// is kept.
pub fn carries_state(event: &WireEvent, seat: Option<u8>) -> bool {
    match *event {
        WireEvent::RoundStarted { .. }
        | WireEvent::RoundEnded { .. }
        | WireEvent::ObstacleDestroyed { .. }
        | WireEvent::DrumLaunched { .. }
        | WireEvent::LavaBombLaunched { .. }
        | WireEvent::Placed { .. }
        | WireEvent::Teleported { .. }
        | WireEvent::TankEntered { .. }
        | WireEvent::Shoved { .. } => true,
        WireEvent::Fired { slot, .. } => seat.is_none_or(|seat| slot == seat as u16),
        _ => false,
    }
}

/// Move a position by its key's error offset, if it has one.
fn shift(offsets: &BTreeMap<Key, (f32, f32)>, key: Key, x: &mut i16, y: &mut i16) {
    if let Some(&(dx, dy)) = offsets.get(&key) {
        *x = nudge(*x, dx);
        *y = nudge(*y, dy);
    }
}

/// A quantised position moved by `qpx` quarter pixels.
fn nudge(a: i16, qpx: f32) -> i16 {
    (a as f32 + qpx).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

/// Whether a shot is in flight rather than at the muzzle or in its
/// impact frames.
fn flying(shot: &ShotState) -> bool {
    let col = shot.state as i32;
    match shot.kind {
        ShotKind::Shell => ShellState::from_col(col) == Some(ShellState::Flying),
        ShotKind::Bullet => BulletState::from_col(col) == Some(BulletState::Flying),
        ShotKind::Plasma => PlasmaState::from_col(col) == Some(PlasmaState::Flying),
    }
}

/// A shot kind's speed, pixels per second.
fn shot_speed(kind: ShotKind) -> f32 {
    let t = tuning();
    match kind {
        ShotKind::Shell => t.shell_speed,
        ShotKind::Bullet => t.minigun_bullet_speed,
        ShotKind::Plasma => t.plasma_speed,
    }
}

/// The unit vector a wire heading points along (0 = up, clockwise).
fn heading_vector(heading: u8) -> (f32, f32) {
    let radians = dequantise_heading(heading).to_radians();
    (radians.sin(), -radians.cos())
}

/// The tanks that jumped from `from` onto `to` rather than drove there.
fn teleported(from: &Snapshot, to: &Snapshot) -> Vec<u16> {
    let span_s = ((tick_ms(to.tick) - tick_ms(from.tick)) / 1000.0).max(0.0) as f32;
    let top = tuning().tank_speed;
    from.tanks
        .iter()
        .filter_map(|a| {
            let b = to.tanks.iter().find(|b| b.id == a.id)?;
            jumped(a, b, span_s, top, &to.events).then_some(a.id)
        })
        .collect()
}

/// Whether a hull went from `a` to `b` in `span_s` seconds by some way
/// other than driving: an event says it was moved, or the distance is
/// more than twice the fastest it could have been going plus
/// `TELEPORT_SLACK_PX`. `TankEntered` says nothing of the kind - a
/// rolling-in hull reaches the inside of its gate on a drive the
/// snapshots carry tick by tick - so it is left to the distance.
fn jumped(a: &TankState, b: &TankState, span_s: f32, top_speed: f32, events: &[WireEvent]) -> bool {
    let moved = events.iter().any(|event| match *event {
        WireEvent::Teleported { slot, .. } => slot == a.id,
        WireEvent::Placed { seat, .. } => seat as u16 == a.id,
        _ => false,
    });
    if moved {
        return true;
    }
    let speed = |t: &TankState| dequantise_velocity(t.vx).hypot(dequantise_velocity(t.vy));
    let fastest = speed(a).max(speed(b)).max(top_speed);
    let steps = crate::net::wire::POSITION_STEPS_PER_PX;
    let distance = ((b.x as f32 - a.x as f32) / steps).hypot((b.y as f32 - a.y as f32) / steps);
    distance > 2.0 * fastest * span_s + TELEPORT_SLACK_PX
}

/// The shots `events` says went through a portal (`ShotTeleported`), by
/// the id `ShotState` keys them under.
fn shots_teleported(events: &[WireEvent]) -> impl Iterator<Item = u16> + '_ {
    events.iter().filter_map(|event| match *event {
        WireEvent::ShotTeleported { id, .. } => id,
        _ => None,
    })
}

/// `from`, with every position moved `alpha` of the way toward `to`.
fn blend(
from: &Snapshot, to: &Snapshot, alpha: f32) -> Snapshot {
    let mut out = from.clone();
    let span_s = ((tick_ms(to.tick) - tick_ms(from.tick)) / 1000.0).max(0.0) as f32;
    let top = tuning().tank_speed;
    for tank in &mut out.tanks {
        if let Ok(i) = to.tanks.binary_search_by_key(&tank.id, |t| t.id) {
            let next = &to.tanks[i];
            // A jump is drawn at the far end's tick, not lerped across.
            if jumped(tank, next, span_s, top, &to.events) {
                continue;
            }
            tank.x = lerp(tank.x, next.x, alpha);
            tank.y = lerp(tank.y, next.y, alpha);
        }
    }
    let through: Vec<u16> = shots_teleported(&to.events).collect();
    for shot in &mut out.shots {
        if let Ok(i) = to.shots.binary_search_by_key(&shot.id, |s| s.id) {
            // Through a portal, like a hull's jump: drawn at the far
            // end's tick, not lerped across the field.
            if through.contains(&shot.id) {
                continue;
            }
            shot.x = lerp(shot.x, to.shots[i].x, alpha);
            shot.y = lerp(shot.y, to.shots[i].y, alpha);
        }
    }
    for missile in &mut out.missiles {
        if let Some(next) = to.missiles.iter().find(|m| m.id == missile.id) {
            blend_missile(missile, next, alpha);
        }
    }
    for grenade in &mut out.grenades {
        if let Some(next) = to.grenades.iter().find(|g| g.id == grenade.id) {
            grenade.x = lerp(grenade.x, next.x, alpha);
            grenade.y = lerp(grenade.y, next.y, alpha);
            grenade.height = lerp(grenade.height, next.height, alpha);
        }
    }
    for frog in &mut out.frogs {
        if let Some(next) = to.frogs.iter().find(|f| f.side == frog.side) {
            frog.x = lerp(frog.x, next.x, alpha);
            frog.y = lerp(frog.y, next.y, alpha);
        }
    }
    out
}

/// A missile `alpha` of the way to `next`: position and height linearly,
/// both headings by the shortest arc.
fn blend_missile(missile: &mut MissileState, next: &MissileState, alpha: f32) {
    missile.x = lerp(missile.x, next.x, alpha);
    missile.y = lerp(missile.y, next.y, alpha);
    missile.height = lerp(missile.height, next.height, alpha);
    missile.facing = lerp_heading(missile.facing, next.facing, alpha);
    missile.heading = lerp_heading(missile.heading, next.heading, alpha);
}

/// `from` carried `ahead_ms` past its tick: hulls on their last velocity
/// and then dead-blended to a stop, flying shots along their heading at
/// their kind's speed, missiles along their ground heading at the speed
/// `before` -> `from` shows. Frogs and everything else hold.
fn extrapolate(from: &Snapshot, before: Option<&Snapshot>, ahead_ms: f64) -> Snapshot {
    let mut out = from.clone();
    if ahead_ms <= 0.0 {
        return out;
    }
    let hull_s = hull_travel_ms(ahead_ms) as f32 / 1000.0;
    for tank in &mut out.tanks {
        tank.x = offset(tank.x, dequantise_velocity(tank.vx) * hull_s);
        tank.y = offset(tank.y, dequantise_velocity(tank.vy) * hull_s);
    }
    let shot_s = (ahead_ms.min(SHOT_EXTRAPOLATE_MS) / 1000.0) as f32;
    for shot in out.shots.iter_mut().filter(|s| flying(s)) {
        let (dx, dy) = heading_vector(shot.heading);
        let travel = shot_speed(shot.kind) * shot_s;
        shot.x = offset(shot.x, dx * travel);
        shot.y = offset(shot.y, dy * travel);
    }
    if let Some(before) = before {
        let span_s = ((tick_ms(from.tick) - tick_ms(before.tick)) / 1000.0) as f32;
        if span_s > 0.0 {
            let steps = crate::net::wire::POSITION_STEPS_PER_PX;
            for missile in &mut out.missiles {
                let Some(prior) = before.missiles.iter().find(|m| m.id == missile.id) else { continue };
                let covered = ((missile.x as f32 - prior.x as f32) / steps).hypot((missile.y as f32 - prior.y as f32) / steps);
                let (dx, dy) = heading_vector(missile.heading);
                let travel = covered / span_s * shot_s;
                missile.x = offset(missile.x, dx * travel);
                missile.y = offset(missile.y, dy * travel);
            }
        }
    }
    out
}

/// How many milliseconds of a hull's last velocity `ahead_ms` past the
/// newest snapshot is worth: all of it up to `HULL_EXTRAPOLATE_MS`, then
/// the integral of a velocity halving every `HULL_BLEND_HALF_LIFE_MS`, so
/// the hull coasts to a stop rather than halting.
fn hull_travel_ms(ahead_ms: f64) -> f64 {
    if ahead_ms <= HULL_EXTRAPOLATE_MS {
        return ahead_ms;
    }
    let tau = HULL_BLEND_HALF_LIFE_MS / std::f64::consts::LN_2;
    let past = ahead_ms - HULL_EXTRAPOLATE_MS;
    HULL_EXTRAPOLATE_MS + tau * (1.0 - (-past / tau).exp())
}

/// Two quantised positions, `alpha` of the way from one to the other.
fn lerp(a: i16, b: i16, alpha: f32) -> i16 {
    let v = a as f32 + (b as f32 - a as f32) * alpha;
    v.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

/// Two wire headings, `alpha` of the way from one to the other the short
/// way round.
fn lerp_heading(a: u8, b: u8, alpha: f32) -> u8 {
    let turn = b.wrapping_sub(a) as i8 as f32;
    a.wrapping_add((turn * alpha).round() as i8 as u8)
}

/// A quantised position moved by `px` pixels.
fn offset(a: i16, px: f32) -> i16 {
    (a as i32 + quantise_pos(px) as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frog::Side;
    use crate::net::wire::{FrogState, POSITION_STEPS_PER_PX, quantise_heading, quantise_velocity};

    /// A snapshot at `tick`, stamped as the room would, with one hull
    /// moving right at 200 px/s: 13 quarter pixels a tick.
    fn snapshot(tick: u32) -> Snapshot {
        Snapshot {
            tick,
            server_ms: stamp(tick),
            tanks: vec![TankState {
                id: 0,
                x: 400 + (tick as i16) * 13,
                y: 800,
                vx: quantise_velocity(200.0),
                dir: 1,
                hp: 100,
                ..Default::default()
            }],
            frogs: vec![FrogState { side: Side::Player, x: 100, y: 100, hp: 100, state: 0, phase: 0, hop_x: 0, hop_y: 0 }],
            ..Default::default()
        }
    }

    /// The room's `server_ms` stamp for `tick`, on the tick schedule.
    fn stamp(tick: u32) -> u32 {
        tick_ms(tick).round() as u32
    }

    /// The local arrival time of `tick` on a perfect link with no transit.
    fn exact(tick: u32) -> i64 {
        tick_ms(tick).round() as i64
    }

    /// An event to count, at `x`.
    fn blast(x: i16) -> WireEvent {
        WireEvent::Blast { x, y: 0, chained: false, drum: crate::obstacle::Drum::Oil }
    }

    /// An interpolator fed `ticks` on a perfect link.
    fn fed(ticks: &[u32]) -> Interpolator {
        let mut interp = Interpolator::default();
        for &tick in ticks {
            interp.accept(snapshot(tick), exact(tick));
        }
        interp
    }

    /// The local time at which render time lands on tick time `t`, with
    /// the picture on its target: the clock's estimate says where the
    /// server is, and the delay is wherever the interpolator has it.
    fn at(interp: &Interpolator, t: f64) -> i64 {
        let offset = interp.clock().offset_ms().expect("a snapshot has arrived");
        (t - offset + interp.delay_ms()).round() as i64
    }

    /// Render time a frame stands at, on the tick schedule.
    fn render_of(frame: &Frame) -> f64 {
        tick_ms(frame.snapshot.tick) + frame.ahead as f64 * 1000.0
    }

    /// A deterministic pseudo-random sequence in `0..1`, so a jittered
    /// link is the same on every run.
    fn noise(seed: &mut u64) -> f64 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (*seed >> 11) as f64 / (1u64 << 53) as f64
    }

    #[test]
    fn render_time_sits_between_the_two_snapshots_that_bracket_it() {
        let mut interp = fed(&[0, 3, 6, 9]);
        // Halfway between the snapshots at tick 3 (50 ms) and 6 (100 ms).
        let frame = interp.sample(at(&interp, 75.0)).expect("a frame");
        assert_eq!(frame.snapshot.tick, 3, "the near end is the snapshot render time has passed");
        assert!(!frame.extrapolated);
        let (a, b) = (snapshot(3).tanks[0].x, snapshot(6).tanks[0].x);
        assert!(
            (frame.snapshot.tanks[0].x - (a + b) / 2).abs() <= 1,
            "half of the way between {a} and {b}, got {}",
            frame.snapshot.tanks[0].x
        );
        assert!((frame.ahead - 0.025).abs() < 0.002, "25 ms past the near end, got {}", frame.ahead);

        // A frame later, with no snapshot in between, the picture has
        // moved on: this is the whole point of the delay.
        let later = interp.sample(at(&interp, 91.0)).expect("a frame");
        assert!(later.snapshot.tanks[0].x > frame.snapshot.tanks[0].x, "a frame with no snapshot still moves");
        assert!(later.snapshot.tanks[0].x < b, "and never past the far end");
    }

    #[test]
    fn a_hulls_facing_snaps_at_its_tick_instead_of_blending() {
        let mut interp = Interpolator::default();
        interp.accept(snapshot(3), exact(3));
        let mut turned = snapshot(6);
        turned.tanks[0].dir = 2;
        interp.accept(turned, exact(6));
        // Nine tenths of the way toward the snapshot that turned.
        let frame = interp.sample(at(&interp, 95.0)).expect("a frame");
        assert_eq!(frame.snapshot.tanks[0].dir, 1, "the near end owns the facing");
        // Render time reaches the turn: it snaps, whole.
        let frame = interp.sample(at(&interp, 105.0)).expect("a frame");
        assert_eq!(frame.snapshot.tanks[0].dir, 2);
    }

    /// Past the newest snapshot a hull runs on at its last velocity for
    /// `HULL_EXTRAPOLATE_MS`, then coasts to a stop on the dead-blend -
    /// never stopping dead, never running away.
    #[test]
    fn a_gap_runs_on_the_last_velocity_and_then_dead_blends_to_a_stop() {
        let mut interp = fed(&[0, 3]);
        let newest = snapshot(3);
        let at_newest = tick_ms(3);
        // Local time is whole milliseconds, so "on the snapshot" is a
        // millisecond short of it.
        let held = interp.sample(at(&interp, at_newest - 1.0)).expect("a frame");
        assert!((held.snapshot.tanks[0].x - newest.tanks[0].x).abs() <= 1, "short of the snapshot, nothing is guessed");
        assert!(!held.extrapolated);

        // 200 px/s, in quarter pixels per millisecond.
        let per_ms = 200.0 * POSITION_STEPS_PER_PX as f64 / 1000.0;
        let x_at = |interp: &mut Interpolator, past: f64| {
            let frame = interp.sample(at(interp, at_newest + past)).expect("a frame");
            assert!(frame.extrapolated, "{past} ms past the newest snapshot is a guess");
            assert_eq!(frame.snapshot.frogs[0].x, newest.frogs[0].x, "a frog holds");
            frame.snapshot.tanks[0].x as f64 - newest.tanks[0].x as f64
        };
        let fifty = x_at(&mut interp, 50.0);
        assert!((fifty - 50.0 * per_ms).abs() <= 1.0, "full speed for the first stretch: {fifty}");
        let full = x_at(&mut interp, HULL_EXTRAPOLATE_MS);
        assert!((full - HULL_EXTRAPOLATE_MS * per_ms).abs() <= 1.0, "full speed to the blend: {full}");
        let blending = x_at(&mut interp, HULL_EXTRAPOLATE_MS + HULL_BLEND_HALF_LIFE_MS);
        let gained = blending - full;
        assert!(
            gained > 0.5 * HULL_BLEND_HALF_LIFE_MS * per_ms && gained < HULL_BLEND_HALF_LIFE_MS * per_ms,
            "one half-life into the blend the hull still moves, slower: {gained}"
        );
        let later = x_at(&mut interp, 1_000.0);
        let limit = (HULL_EXTRAPOLATE_MS + HULL_BLEND_HALF_LIFE_MS / std::f64::consts::LN_2) * per_ms;
        assert!(later <= limit + 1.0 && later > blending, "it coasts to a stop short of {limit}: {later}");
        let still = x_at(&mut interp, 2_000.0);
        assert!((still - later).abs() <= 1.0, "and stays stopped: {later} then {still}");
    }

    /// Shots in flight carry on along their heading at their kind's
    /// speed; one at the muzzle holds.
    #[test]
    fn a_flying_shot_is_extrapolated_along_its_heading() {
        let shot = |id: u16, state: ShellState, heading: f32| ShotState {
            id,
            kind: ShotKind::Shell,
            x: 2_000,
            y: 2_000,
            heading: quantise_heading(heading),
            state: state.col() as u8,
            variant: 0,
            owner: crate::net::wire::NO_SEAT,
        };
        let mut a = snapshot(0);
        a.shots = vec![shot(1, ShellState::Flying, 90.0), shot(2, ShellState::Fire0, 90.0), shot(3, ShellState::Flying, 180.0)];
        let mut b = snapshot(3);
        b.shots = a.shots.clone();
        let mut interp = Interpolator::default();
        interp.accept(a, exact(0));
        interp.accept(b, exact(3));
        interp.sample(at(&interp, tick_ms(3))).expect("a frame");
        let frame = interp.sample(at(&interp, tick_ms(3) + 40.0)).expect("a frame");
        assert!(frame.extrapolated);
        let travel = tuning().shell_speed * 0.040 * POSITION_STEPS_PER_PX;
        let right = &frame.snapshot.shots[0];
        assert!((right.x as f32 - (2_000.0 + travel)).abs() <= 2.0, "a shell heading right flew on: {right:?}");
        assert!((right.y - 2_000).abs() <= 1, "and only right: {right:?}");
        let muzzle = &frame.snapshot.shots[1];
        assert_eq!((muzzle.x, muzzle.y), (2_000, 2_000), "a shot at the muzzle holds");
        let down = &frame.snapshot.shots[2];
        assert!((down.y as f32 - (2_000.0 + travel)).abs() <= 2.0, "a shell heading down flew down: {down:?}");
        // The guess is kept short: nothing here knows the walls.
        let far = interp.sample(at(&interp, tick_ms(3) + 1_000.0)).expect("a frame");
        let cap = tuning().shell_speed * (SHOT_EXTRAPOLATE_MS / 1000.0) as f32 * POSITION_STEPS_PER_PX;
        assert!((far.snapshot.shots[0].x as f32 - (2_000.0 + cap)).abs() <= 2.0, "held at the cap: {:?}", far.snapshot.shots[0]);
    }

    /// Missiles blend by id - position, height and both headings the
    /// short way round - and carry on at their measured speed past the
    /// newest snapshot.
    #[test]
    fn missiles_are_interpolated_and_extrapolated() {
        let missile = |x: i16, height: i16, facing: u8, heading: u8| MissileState {
            id: 7,
            x,
            y: 1_000,
            height,
            facing,
            heading,
            tube: 1,
        };
        let mut a = snapshot(0);
        a.missiles = vec![missile(1_000, 40, 250, 64)];
        let mut b = snapshot(3);
        // 30 px in 50 ms: 600 px/s, climbing, the facing across the wrap.
        b.missiles = vec![missile(1_120, 80, 6, 64)];
        let mut interp = Interpolator::default();
        interp.accept(a, exact(0));
        interp.accept(b, exact(3));
        let frame = interp.sample(at(&interp, 25.0)).expect("a frame");
        let m = frame.snapshot.missiles[0];
        assert!((m.x - 1_060).abs() <= 1, "halfway along: {m:?}");
        assert_eq!(m.height, 60, "halfway up: {m:?}");
        assert_eq!(m.facing, 0, "halfway round the short way, across the wrap: {m:?}");
        assert_eq!(m.heading, 64);
        // Past the newest: on along the heading at 600 px/s.
        interp.sample(at(&interp, tick_ms(3))).expect("a frame");
        let frame = interp.sample(at(&interp, tick_ms(3) + 50.0)).expect("a frame");
        let m = frame.snapshot.missiles[0];
        assert!((m.x - (1_120 + 120)).abs() <= 2, "another 30 px on: {m:?}");
        assert_eq!(m.y, 1_000);
    }

    /// A hull that moved further than it could have driven between two
    /// snapshots - or whose far end says it was teleported - is drawn
    /// whole at the far end's tick, and that frame names it.
    #[test]
    fn a_teleport_is_snapped_at_its_tick_and_reported() {
        let mut a = snapshot(3);
        let mut b = snapshot(6);
        b.tanks[0].x = a.tanks[0].x + 2_000; // 500 px in 50 ms
        let mut c = snapshot(9);
        c.tanks[0].x = b.tanks[0].x + 39;
        let mut interp = Interpolator::default();
        interp.accept(a.clone(), exact(3));
        interp.accept(b.clone(), exact(6));
        interp.accept(c, exact(9));
        let before = interp.sample(at(&interp, 90.0)).expect("a frame");
        assert_eq!(before.snapshot.tanks[0].x, a.tanks[0].x, "held at the near end, not lerped across the map");
        assert!(before.snapped.is_empty());
        let after = interp.sample(at(&interp, 101.0)).expect("a frame");
        assert_eq!(after.snapshot.tick, 6);
        assert!((after.snapshot.tanks[0].x - b.tanks[0].x).abs() <= 1, "drawn at the far end whole");
        assert_eq!(after.snapped, vec![0], "and named, for its tread marks");
        let next = interp.sample(at(&interp, 110.0)).expect("a frame");
        assert!(next.snapped.is_empty(), "named once");

        // A short hop through a portal is a teleport by its event alone.
        a.tanks[0].id = 5;
        b = snapshot(6);
        b.tanks[0].id = 5;
        b.tanks[0].x = a.tanks[0].x + 40;
        b.events = vec![WireEvent::Teleported { slot: 5, x: 0, y: 0, to_x: b.tanks[0].x, to_y: b.tanks[0].y }];
        let mut interp = Interpolator::default();
        interp.accept(a.clone(), exact(3));
        interp.accept(b, exact(6));
        let frame = interp.sample(at(&interp, 90.0)).expect("a frame");
        assert_eq!(frame.snapshot.tanks[0].x, a.tanks[0].x, "the event says it jumped");
        let frame = interp.sample(at(&interp, 101.0)).expect("a frame");
        assert_eq!(frame.snapped, vec![5]);
    }

    /// A shot whose far end says it came through a portal holds at the
    /// near end and is drawn at the exit whole - never slid across the
    /// field between the two portals - while one with no such event is
    /// lerped as ever.
    #[test]
    fn a_shot_through_a_portal_is_drawn_as_a_jump() {
        let shot = |id: u16, x: i16| ShotState {
            id,
            kind: ShotKind::Shell,
            x,
            y: 1_000,
            heading: quantise_heading(90.0),
            state: ShellState::Flying.col() as u8,
            variant: 0,
            owner: crate::net::wire::NO_SEAT,
        };
        let mut a = snapshot(3);
        a.shots = vec![shot(7, 1_000), shot(8, 1_000)];
        let mut b = snapshot(6);
        b.shots = vec![shot(7, 4_000), shot(8, 1_080)];
        b.events = vec![WireEvent::ShotTeleported { id: Some(7), x: 1_040, y: 1_000, to_x: 3_960, to_y: 1_000 }];
        let mut interp = Interpolator::default();
        interp.accept(a, exact(3));
        interp.accept(b, exact(6));
        let before = interp.sample(at(&interp, 90.0)).expect("a frame");
        assert_eq!(before.snapshot.shots[0].x, 1_000, "held at the near end, not slid toward the exit");
        assert!(before.snapshot.shots[1].x > 1_000, "the other shot is lerped");
        let after = interp.sample(at(&interp, 101.0)).expect("a frame");
        assert_eq!(after.snapshot.tick, 6);
        assert!((after.snapshot.shots[0].x - 4_000).abs() <= 8, "drawn at the exit whole, a millisecond on: {:?}", after.snapshot.shots[0]);
    }

    #[test]
    fn the_clock_follows_the_fastest_arrivals_and_snaps_on_a_jump() {

        let mut clock = ServerClock::default();
        assert_eq!(clock.now(0), None, "nothing is known before the first reading");
        // The first reading is taken whole: server 1000 ms at local 0 ms.
        assert!(clock.observe_ms(1_000.0, 0), "the first reading is a new clock");
        assert_eq!(clock.offset_ms(), Some(1_000.0));
        assert_eq!(clock.now(500), Some(1_500.0));
        // A packet 100 ms late moves nothing: it is late, not the clock.
        assert!(!clock.observe_ms(1_100.0, 200));
        assert_eq!(clock.offset_ms(), Some(1_000.0));
        assert!((clock.lateness(1_100.0, 200) - 100.0).abs() < 1e-9);
        // A faster one than any before is the envelope at once.
        clock.observe_ms(1_220.0, 210);
        assert_eq!(clock.offset_ms(), Some(1_010.0));
        // A path that got 40 ms slower is followed once the fast readings
        // have left the window, and never below the envelope.
        let mut local = 210;
        let mut least = f64::MAX;
        for _ in 0..240 {
            local += 16;
            clock.observe_ms(local as f64 + 970.0, local);
            let offset = clock.offset_ms().unwrap();
            least = least.min(offset);
            assert!(offset >= 970.0 - 1e-9, "never below the envelope");
        }
        assert!((clock.offset_ms().unwrap() - 970.0).abs() < 1e-6, "followed down: {:?}", clock.offset_ms());
        // A stall's late burst does not drag it: three seconds of silence
        // earn a tenth of a second's fall at most.
        local += 3_000;
        for i in 0..10 {
            clock.observe_ms((local - 3_000 + i * 16) as f64 + 970.0, local);
        }
        let after = clock.offset_ms().unwrap();
        assert!(after >= 970.0 - CLOCK_FALL_CREDIT_MS * CLOCK_FALL_PER_MS - 1e-6, "a burst is late, not a new clock: {after}");
        // Another clock entirely, far ahead, is taken whole.
        assert!(clock.observe_ms(20_000.0, local));
        assert_eq!(clock.offset_ms(), Some(20_000.0 - local as f64));
    }

    #[test]
    fn a_snapshots_events_are_handed_over_once_when_render_time_reaches_it() {
        let mut interp = Interpolator::default();
        let mut with_event = snapshot(3);
        with_event.events = vec![blast(10)];
        interp.accept(snapshot(0), exact(0));
        interp.accept(with_event, exact(3));
        interp.accept(snapshot(6), exact(6));
        // Render time is still short of the snapshot that carries it.
        let frame = interp.sample(at(&interp, 20.0)).expect("a frame");
        assert!(frame.snapshot.events.is_empty(), "nothing of a tick not yet drawn");
        // It reaches it: the event goes over with the tick it belongs to.
        let frame = interp.sample(at(&interp, 60.0)).expect("a frame");
        assert_eq!(frame.snapshot.tick, 3);
        assert_eq!(frame.snapshot.events.len(), 1);
        // And never again.
        let frame = interp.sample(at(&interp, 70.0)).expect("a frame");
        assert!(frame.snapshot.events.is_empty());
    }

    #[test]
    fn a_frame_that_steps_over_a_whole_snapshot_still_hands_its_events_over() {
        let mut interp = Interpolator::default();
        interp.accept(snapshot(0), exact(0));
        for tick in [3u32, 6, 9] {
            let mut s = snapshot(tick);
            s.events = vec![blast(tick as i16)];
            interp.accept(s, exact(tick));
        }
        // One long frame, straight past ticks 3 and 6 onto 9.
        let frame = interp.sample(at(&interp, 160.0)).expect("a frame");
        assert_eq!(frame.snapshot.tick, 9);
        assert_eq!(frame.snapshot.events.len(), 3, "every snapshot passed owes its events: {:?}", frame.snapshot.events);
    }

    #[test]
    fn a_repeat_is_dropped_a_rewind_starts_over_and_a_welcome_is_the_baseline() {
        let mut interp = fed(&[0, 3, 6]);
        assert_eq!(interp.buffered(), 3);
        interp.accept(snapshot(6), 100);
        assert_eq!(interp.buffered(), 3, "the newest again is not news");
        assert_eq!(interp.newest_tick(), Some(6));
        // The round started over: the buffer holds a world that is gone.
        let mut restarted = snapshot(0);
        restarted.server_ms = 5_000;
        restarted.events = vec![WireEvent::RoundStarted {
            seed: 7,
            enemies: 4,
            mission: crate::level::Mission::Protect,
            spawn: crate::level::SpawnKind::Band,
        }];
        interp.accept(restarted, 5_000);
        assert_eq!(interp.buffered(), 1);
        let frame = interp.sample(at(&interp, 0.0)).expect("a frame");
        assert_eq!(frame.snapshot.tick, 0);
        assert_eq!(frame.snapshot.events.len(), 1, "the new round's own events are drawn");
        // A welcome's snapshot is the new baseline and is not drawn twice.
        let mut baseline = snapshot(0);
        baseline.events = vec![blast(1)];
        interp.restart(&baseline, 0);
        assert_eq!(interp.buffered(), 1);
        let frame = interp.sample(at(&interp, 0.0)).expect("a frame");
        assert!(frame.snapshot.events.is_empty(), "the welcome applied its own events");
    }

    /// A clean link pays the floor: one interval and one frame come to
    /// the knob's 33 ms, and with no lateness nothing is added.
    #[test]
    fn a_clean_link_pays_the_floor() {
        let floor = tuning().online_interpolation_delay_ms as f64;
        let mut clean = Interpolator::default();
        for tick in 0..240u32 {
            clean.accept(snapshot(tick), exact(tick) + 25);
        }
        let report = clean.report();
        assert!(report.lateness_p95_ms < 1.0, "an even link is never late: {report:?}");
        assert!((report.interval_ms - TICK_MS).abs() < 1e-9, "a snapshot a tick: {report:?}");
        assert!((report.target_ms - floor).abs() < 0.5, "a clean link pays the floor: {report:?}");
        assert_eq!(report.stalls, 0);
    }

    /// The delay covers the link's lateness percentile - once, not twice
    /// - and stays under the cap.
    #[test]
    fn the_delay_covers_the_lateness_percentile() {
        let mut dirty = Interpolator::default();
        for tick in 0..240u32 {
            let late = [0, 30, 5, 25, 10, 20][(tick % 6) as usize];
            dirty.accept(snapshot(tick), exact(tick) + late);
        }
        let report = dirty.report();
        assert!((report.lateness_p95_ms - 30.0).abs() < 1.5, "the late tail is 30 ms: {report:?}");
        let expected = TICK_MS + FRAME_MS + report.lateness_p95_ms;
        assert!((report.target_ms - expected).abs() < 0.5, "an interval, a frame and the tail: {report:?}");
        assert!(report.target_ms <= tuning().online_interpolation_delay_max_ms as f64);
    }

    /// Under jittered arrivals - late by anything up to 40 ms, in order,
    /// as TCP delivers them - the picture never runs backwards, its speed
    /// stays within the bounds, and on most frames it is exactly 1.0.
    #[test]
    fn the_render_clock_never_goes_backwards_under_jitter() {
        let mut interp = Interpolator::default();
        let mut seed = 7u64;
        let mut arrivals: Vec<(u32, i64)> = Vec::new();
        let mut last = 0i64;
        for tick in 0..900u32 {
            let arrive = (tick_ms(tick) + 30.0 + noise(&mut seed) * 40.0).round() as i64;
            last = last.max(arrive);
            arrivals.push((tick, last));
        }
        let mut next = 0;
        let mut local = 0i64;
        let mut renders: Vec<f64> = Vec::new();
        let mut steady = 0;
        let mut frames = 0;
        while local < 15_000 {
            while next < arrivals.len() && arrivals[next].1 <= local {
                interp.accept(snapshot(arrivals[next].0), arrivals[next].1);
                next += 1;
            }
            if let Some(frame) = interp.sample(local) {
                renders.push(render_of(&frame));
                let rate = interp.report().rate;
                assert!((1.0 - RATE_FAR..=1.0 + RATE_FAR).contains(&rate), "rate {rate}");
                if local > 3_000 {
                    frames += 1;
                    if rate == 1.0 {
                        steady += 1;
                    }
                }
            }
            local += 7 + (noise(&mut seed) * 10.0) as i64;
        }
        for pair in renders.windows(2) {
            assert!(pair[1] >= pair[0] - 1e-6, "render time went back from {} to {}", pair[0], pair[1]);
        }
        assert!(steady * 10 >= frames * 8, "the speed held at 1.0 on {steady} of {frames} frames");
    }

    /// One lost TCP segment: 150 ms of nothing, then ten snapshots at
    /// once. The picture rides the gap out on extrapolation, never steps
    /// back, never runs in slow motion for long, and the hull's drawn
    /// path has no jump in it.
    #[test]
    fn a_lost_segment_is_ridden_out_without_a_rewind_or_slow_motion() {
        let mut interp = Interpolator::default();
        let transit = 20i64;
        let mut arrivals: Vec<(u32, i64)> = (0..400u32).map(|t| (t, exact(t) + transit)).collect();
        // The segment carrying ticks 200.. is lost and retransmitted
        // 150 ms later, holding everything behind it.
        let release = exact(200) + transit + 150;
        for (tick, at) in arrivals.iter_mut() {
            if *tick >= 200 && *at < release {
                *at = release;
            }
        }
        let bursts = arrivals.iter().filter(|(_, at)| *at == release).count();
        assert!(bursts >= 10, "ten snapshots land at once: {bursts}");
        let mut next = 0;
        let mut renders: Vec<f64> = Vec::new();
        let mut xs: Vec<i16> = Vec::new();
        let mut slow_ms = 0i64;
        let mut longest_slow = 0i64;
        let mut extrapolated = 0;
        let mut local = 0i64;
        while local < 6_500 {
            while next < arrivals.len() && arrivals[next].1 <= local {
                interp.accept(snapshot(arrivals[next].0), arrivals[next].1);
                next += 1;
            }
            if let Some(frame) = interp.sample(local) {
                renders.push(render_of(&frame));
                xs.push(frame.snapshot.tanks[0].x);
                extrapolated += frame.extrapolated as usize;
                if interp.report().rate < 1.0 {
                    slow_ms += 8;
                    longest_slow = longest_slow.max(slow_ms);
                } else {
                    slow_ms = 0;
                }
            }
            local += 8;
        }
        assert!(extrapolated > 0, "the gap was ridden out on extrapolation");
        for pair in renders.windows(2) {
            assert!(pair[1] >= pair[0] - 1e-6, "render time went back from {} to {}", pair[0], pair[1]);
        }
        assert!(longest_slow <= 1_000, "slow motion for {longest_slow} ms");
        // 200 px/s is 6.7 quarter pixels an 8 ms frame; nothing jumps.
        for pair in xs.windows(2) {
            assert!((pair[1] - pair[0]).abs() <= 10, "the hull jumped from {} to {}", pair[0], pair[1]);
        }
        let report = interp.report();
        assert_eq!(report.stalls, 1, "one stall: {report:?}");
        let floor = tuning().online_interpolation_delay_ms as f64;
        assert!(report.target_ms < floor + 1.0, "the stall is ridden out, not paid for on every frame: {report:?}");
    }

    /// The truth turned out different from the guess - the hull stopped
    /// during a gap the picture ran through - and the correction is a
    /// glide back, not a jump.
    #[test]
    fn a_late_correction_eases_instead_of_snapping() {
        let mut interp = Interpolator::default();
        let mut arrivals: Vec<(Snapshot, i64)> = Vec::new();
        for tick in 0..=60u32 {
            arrivals.push((snapshot(tick), exact(tick)));
        }
        // The hull stopped at tick 60; the next snapshots, which say so,
        // arrive 150 ms late together.
        let stop = snapshot(60).tanks[0].x;
        for tick in 61..=120u32 {
            let mut s = snapshot(tick);
            s.tanks[0].x = stop;
            s.tanks[0].vx = 0;
            arrivals.push((s, exact(tick).max(exact(61) + 150)));
        }
        let mut next = 0;
        let mut xs: Vec<i16> = Vec::new();
        let mut local = 0i64;
        while local < 2_500 {
            while next < arrivals.len() && arrivals[next].1 <= local {
                let (s, at) = arrivals[next].clone();
                interp.accept(s, at);
                next += 1;
            }
            if let Some(frame) = interp.sample(local) {
                xs.push(frame.snapshot.tanks[0].x);
            }
            local += 8;
        }
        let report = interp.report();
        assert!(report.corrections > 0, "the guess was corrected: {report:?}");
        assert!(report.error_p95_px > 1.0, "by a visible amount: {report:?}");
        for pair in xs.windows(2) {
            assert!((pair[1] - pair[0]).abs() <= 10, "the correction jumped from {} to {}", pair[0], pair[1]);
        }
        assert_eq!(*xs.last().unwrap(), stop, "and settled on the truth");
    }

    #[test]
    fn the_measured_interval_follows_the_rooms_cadence() {
        let mut interp = Interpolator::default();
        assert_eq!(interp.interval_ms(), SNAPSHOT_INTERVAL_MS);
        // A room sending every sixth tick is a 100 ms cadence.
        for n in 1..=60u32 {
            interp.accept(snapshot(n * 6), exact(n * 6));
        }
        assert!((interp.interval_ms() - 100.0).abs() < 1e-6, "{}", interp.interval_ms());
    }

    /// A path that got 100 ms slower for good - a route change, not a
    /// stall - is followed the moment its last fast reading leaves the
    /// clock's window rather than at a fall-limited crawl after it. The
    /// picture rides the change out on a bounded run of extrapolated
    /// frames, never steps back, and ends on the floor again with every
    /// frame between two snapshots.
    #[test]
    fn a_sustained_step_in_transit_is_followed_within_the_clock_window() {
        let floor = tuning().online_interpolation_delay_ms as f64;
        let step_tick = 300u32;
        let arrive = |tick: u32| exact(tick) + if tick < step_tick { 20 } else { 120 };
        let stepped_at = arrive(step_tick);
        let mut interp = Interpolator::default();
        let (mut next, mut local) = (0u32, 0i64);
        let mut renders: Vec<f64> = Vec::new();
        let mut followed_at = None;
        let (mut extrapolated, mut extrapolated_late) = (0u32, 0u32);
        while next < 700 {
            while next < 700 && arrive(next) <= local {
                interp.accept(snapshot(next), arrive(next));
                next += 1;
            }
            if let Some(frame) = interp.sample(local) {
                renders.push(render_of(&frame));
                if local >= stepped_at && frame.extrapolated {
                    extrapolated += 1;
                    if local > stepped_at + CLOCK_WINDOW_MS + 500 {
                        extrapolated_late += 1;
                    }
                }
            }
            // The new path reads -120 ms, give or take a third of a
            // millisecond of rounding.
            let offset = interp.clock().offset_ms();
            if local >= stepped_at && followed_at.is_none() && offset.is_some_and(|o| o < -119.0) {
                followed_at = Some(local);
            }
            local += 8;
        }
        let followed = followed_at.expect("the clock followed the slower path") - stepped_at;
        assert!(followed <= CLOCK_WINDOW_MS + 100, "the clock took {followed} ms to follow a 100 ms step");
        for pair in renders.windows(2) {
            assert!(pair[1] >= pair[0] - 1e-6, "render time went back from {} to {}", pair[0], pair[1]);
        }
        // 125 frames a second. The lateness on the room's cadence widens
        // the delay within a twentieth of its window, and the playout
        // clock then slows by at most `RATE_FAR` to let the snapshots get
        // ahead of it again: about a second and a half of guessed frames,
        // and none once the clock has followed.
        assert!(extrapolated <= 150, "{extrapolated} frames extrapolated after the step");
        assert_eq!(extrapolated_late, 0, "still guessing half a second after the clock followed");
        let report = interp.report();
        assert!(report.target_ms - floor < 1.0, "the delay came back to the floor: {report:?}");
    }

    /// A wave tank reaching the inside of its gate carries a
    /// `TankEntered`, but it drove there: it is blended like any drive,
    /// not held at the near end and snapped.
    #[test]
    fn a_tank_entering_through_its_gate_is_blended_not_snapped() {
        let a = snapshot(3);
        let mut b = snapshot(6);
        b.events = vec![WireEvent::TankEntered { slot: 0 }];
        let mut interp = Interpolator::default();
        interp.accept(a.clone(), exact(3));
        interp.accept(b.clone(), exact(6));
        interp.accept(snapshot(9), exact(9));
        let frame = interp.sample(at(&interp, 75.0)).expect("a frame");
        let (from, to) = (a.tanks[0].x, b.tanks[0].x);
        assert!(
            (frame.snapshot.tanks[0].x - (from + to) / 2).abs() <= 1,
            "halfway along its drive from {from} to {to}, got {}",
            frame.snapshot.tanks[0].x
        );
        let frame = interp.sample(at(&interp, 101.0)).expect("a frame");
        assert_eq!(frame.snapshot.tick, 6);
        assert!(frame.snapped.is_empty(), "a drive is not a jump: {:?}", frame.snapped);
    }

    /// A tab that was hidden hands its whole backlog over at once: the
    /// buffer overflows and the next frame steps over everything it kept.
    /// The snapshots long past keep only what the replica needs as state -
    /// a tile's death, this seat's `Fired` - and their blasts and everyone
    /// else's muzzles are dropped rather than all drawn on one frame; the
    /// snapshots about the delay behind the room keep all of theirs.
    #[test]
    fn a_hidden_tabs_backlog_keeps_only_the_events_the_replica_needs_as_state() {
        use crate::net::wire::WeaponKind;
        let mut interp = Interpolator::default();
        interp.set_seat(Some(0));
        for tick in 0..10u32 {
            interp.accept(snapshot(tick), exact(tick));
        }
        interp.sample(exact(9)).expect("a frame");
        let backlog = 10..310u32;
        for tick in backlog.clone() {
            let mut s = snapshot(tick);
            s.events = vec![
                blast(tick as i16),
                WireEvent::Fired { slot: 0, weapon: WeaponKind::Shell, input_tick: tick },
                WireEvent::Fired { slot: 4, weapon: WeaponKind::Shell, input_tick: 0 },
            ];
            if tick % 50 == 0 {
                s.events.push(WireEvent::ObstacleDestroyed { material: crate::obstacle::Material::Brick, x: tick as i16, y: 0 });
            }
            interp.accept(s, exact(tick));
        }
        let frame = interp.sample(exact(backlog.end - 1)).expect("a frame");
        let events = &frame.snapshot.events;
        let count = |pick: &dyn Fn(&WireEvent) -> bool| events.iter().filter(|e| pick(e)).count() as u32;
        let handed = frame.snapshot.tick - backlog.start + 1;
        assert!(handed > 250, "the frame reached the end of the backlog: tick {}", frame.snapshot.tick);
        assert_eq!(count(&|e| matches!(e, WireEvent::ObstacleDestroyed { .. })), 6, "every tile death is state");
        assert_eq!(
            count(&|e| matches!(e, WireEvent::Fired { slot: 0, .. })),
            handed,
            "every one of this seat's presses is confirmed"
        );
        let blasts = count(&|e| matches!(e, WireEvent::Blast { .. }));
        let on_time = ((tuning().online_interpolation_delay_max_ms as f64 + EVENT_STALE_MS) / TICK_MS) as u32 + 2;
        assert!(blasts > 0, "the snapshots the delay would have shown keep their cosmetics");
        assert!(blasts <= on_time, "{blasts} blasts of a {handed}-snapshot backlog drawn on one frame");
        assert_eq!(count(&|e| matches!(e, WireEvent::Fired { slot: 4, .. })), blasts, "an enemy's muzzle is a cosmetic");
        assert_eq!(interp.report().stale_events, 2 * (handed - blasts) as u64);
    }

    /// A room that stalled for 700 ms and dropped the ticks it missed
    /// comes back on another clock - `server_ms - tick time` jumped - whose
    /// target is behind where render time had run on extrapolation. The
    /// picture holds there until the target catches up rather than
    /// rewinding, then runs between snapshots again.
    #[test]
    fn a_new_clock_behind_the_picture_holds_it_rather_than_rewinding() {
        let stall = 700i64;
        let mut arrivals: Vec<(Snapshot, i64)> = (0..=300u32).map(|t| (snapshot(t), exact(t) + 20)).collect();
        for tick in 301..=500u32 {
            let mut s = snapshot(tick);
            s.server_ms = stamp(tick) + stall as u32;
            arrivals.push((s, exact(tick) + stall + 20));
        }
        let mut interp = Interpolator::default();
        let (mut next, mut local) = (0usize, 0i64);
        let mut renders: Vec<f64> = Vec::new();
        let mut held_ms = 0;
        let mut tail: Vec<bool> = Vec::new();
        while next < arrivals.len() {
            while next < arrivals.len() && arrivals[next].1 <= local {
                let (s, at) = arrivals[next].clone();
                interp.accept(s, at);
                next += 1;
            }
            if let Some(frame) = interp.sample(local) {
                renders.push(render_of(&frame));
                if interp.report().rate == 0.0 {
                    held_ms += 8;
                }
                if local > exact(420) + stall {
                    tail.push(frame.extrapolated);
                }
            }
            local += 8;
        }
        // A frame's `ahead` travels as an `f32` of seconds, so the render
        // time it spells out is exact to about a ten-thousandth of a
        // millisecond.
        for pair in renders.windows(2) {
            assert!(pair[1] >= pair[0] - 1e-3, "render time went back from {} to {}", pair[0], pair[1]);
        }
        assert!(held_ms > 0, "the picture held for the new clock's target");
        assert!(held_ms <= stall + 100, "held for {held_ms} ms after a {stall} ms stall");
        assert!(!tail.is_empty() && tail.iter().all(|e| !e), "back between snapshots once the target caught up");
    }

    /// Frames every 8 ms of local time over `arrivals` (sorted by arrival):
    /// the render times drawn, the frames that moved on extrapolation -
    /// guessed, rather than held still past the newest snapshot - and the
    /// milliseconds the picture stood still.
    fn play(interp: &mut Interpolator, arrivals: &[(Snapshot, i64)], from: i64, until: i64) -> (Vec<f64>, u32, i64) {
        let (mut next, mut local) = (0usize, from);
        let (mut renders, mut extrapolated, mut held) = (Vec::new(), 0, 0);
        while local <= until {
            while next < arrivals.len() && arrivals[next].1 <= local {
                let (s, at) = arrivals[next].clone();
                interp.accept(s, at);
                next += 1;
            }
            if let Some(frame) = interp.sample(local) {
                renders.push(render_of(&frame));
                let still = interp.report().rate == 0.0;
                extrapolated += (frame.extrapolated && !still) as u32;
                if still {
                    held += 8;
                }
            }
            local += 8;
        }
        (renders, extrapolated, held)
    }

    /// **A waiting room's welcome does not set the round's clock.** The
    /// room welcomes each seat at tick 0 while it waits, then again at
    /// tick 0 when the round starts, a tenth of a second later, and ticks
    /// from there. Read as one clock, the first welcome put the room a
    /// tenth of a second ahead of every snapshot after it, and the round
    /// opened on seconds of extrapolation and a delay sized for lateness
    /// that was never there.
    #[test]
    fn a_waiting_rooms_welcome_does_not_skew_the_rounds_clock() {
        let transit = 20i64;
        let (waiting, started) = (23u32, 134u32);
        let mut interp = Interpolator::default();
        let mut lobby = snapshot(0);
        lobby.server_ms = waiting;
        interp.restart(&lobby, waiting as i64 + transit);
        let mut round = snapshot(0);
        round.server_ms = started;
        interp.restart(&round, started as i64 + transit);
        let arrivals: Vec<(Snapshot, i64)> = (1..=180u32)
            .map(|t| {
                let mut s = snapshot(t);
                s.server_ms = started + stamp(t);
                let at = s.server_ms as i64 + transit;
                (s, at)
            })
            .collect();
        let start = started as i64 + transit;
        let (renders, extrapolated, _) = play(&mut interp, &arrivals, start, start + 3_000);
        assert!(renders.windows(2).all(|w| w[1] >= w[0] - 1e-3), "render time went back");
        assert!(extrapolated <= 2, "the round opened on {extrapolated} extrapolated frames");
        let report = interp.report();
        assert!(report.target_ms < 40.0, "the delay was sized for lateness that was never there: {report:?}");
        assert!(report.delay_ms < 45.0, "the picture stood {} ms behind", report.delay_ms);
    }

    /// **A room that falls behind its schedule is waited for, not guessed
    /// past.** Owing more ticks than it catches up, a room restarts its
    /// schedule: every tick from there is stamped `dropped` later on the
    /// wall than its number says. The picture holds for that long - the
    /// room stood still - rather than reading every snapshot after as
    /// late, running on extrapolation and widening its delay for it.
    #[test]
    fn a_room_that_drops_ticks_holds_the_picture_rather_than_guessing() {
        let (transit, dropped) = (20i64, 100u32);
        let arrivals: Vec<(Snapshot, i64)> = (0..=420u32)
            .map(|t| {
                let mut s = snapshot(t);
                s.server_ms = stamp(t) + if t > 200 { dropped } else { 0 };
                let at = s.server_ms as i64 + transit;
                (s, at)
            })
            .collect();
        let mut interp = Interpolator::default();
        interp.restart(&arrivals[0].0, arrivals[0].1);
        let (renders, extrapolated, held) = play(&mut interp, &arrivals[1..], transit, exact(420) + dropped as i64 + transit);
        assert!(renders.windows(2).all(|w| w[1] >= w[0] - 1e-3), "render time went back");
        // Until the first late tick arrives nothing says the room stopped:
        // the gap's frames past the delay run on the last velocities, and
        // the two late ticks that name the new schedule take one more.
        // Past that the picture holds rather than guessing on.
        let gap_frames = ((dropped as f64 + TICK_MS - 33.3) / 8.0).ceil() as u32 + 3;
        assert!(extrapolated <= gap_frames, "{extrapolated} frames were guessed past a room that had stopped");
        assert!(held >= dropped as i64 / 2 && held <= dropped as i64 + 50, "held {held} ms for a {dropped} ms stop");
        let report = interp.report();
        assert!(report.target_ms < 40.0, "the stop widened the delay: {report:?}");
    }

    /// A stall longer than the clock's window whose backlog comes back in
    /// chunks, a round trip apart: the chunks after the first do not
    /// restart the settling, so the stall's late readings never drag the
    /// estimate back by the stall's length.
    #[test]
    fn a_backlog_in_chunks_does_not_drag_the_clock_back() {
        let mut clock = ServerClock::default();
        for local in (0..3_000i64).step_by(16) {
            clock.observe_ms(local as f64 + 1_000.0, local);
        }
        let before = clock.offset_ms().expect("an estimate");
        // Silent for three seconds, then the backlog in three chunks 80 ms
        // apart, every reading as late as the stall.
        let resumed = 6_000i64;
        for chunk in 0..3i64 {
            let at = resumed + chunk * 80;
            for i in 0..20i64 {
                let sent = 3_000 + (chunk * 20 + i) * 16;
                clock.observe_ms(sent as f64 + 1_000.0, at);
            }
        }
        let after = clock.offset_ms().expect("an estimate");
        assert!(before - after <= CLOCK_FALL_CREDIT_MS * CLOCK_FALL_PER_MS * 3.0 + 1e-6, "dragged back {} ms", before - after);
    }

    /// The same with a longer round trip: a three-second stall whose
    /// backlog comes back in eight chunks 250 ms apart, the last of them
    /// well past `CLOCK_SETTLE_MS` after the first. Every chunk's gap
    /// extends the settling, so no chunk drags the estimate back by the
    /// stall; each earns only its fall credit.
    #[test]
    fn a_backlog_in_chunks_a_long_round_trip_apart_does_not_drag_the_clock_back() {
        let mut clock = ServerClock::default();
        for local in (0..3_000i64).step_by(16) {
            clock.observe_ms(local as f64 + 1_000.0, local);
        }
        let before = clock.offset_ms().expect("an estimate");
        let (resumed, chunks, per_chunk) = (6_000i64, 8i64, 23i64);
        assert!((chunks - 1) * 250 > CLOCK_SETTLE_MS, "the backlog outlasts one settling");
        let mut least = before;
        for chunk in 0..chunks {
            let at = resumed + chunk * 250;
            for i in 0..per_chunk {
                let sent = 3_000 + (chunk * per_chunk + i) * 16;
                clock.observe_ms(sent as f64 + 1_000.0, at);
                least = least.min(clock.offset_ms().expect("an estimate"));
            }
        }
        let credit = chunks as f64 * CLOCK_FALL_CREDIT_MS * CLOCK_FALL_PER_MS;
        assert!(before - least <= credit + 1e-6, "dragged back {} ms by a stall's backlog", before - least);
    }

    /// A link that goes on arriving in bursts after a stall - a path that
    /// is simply bursty, and slower - is not settled for ever: the
    /// settling ends `CLOCK_SETTLE_MAX_MS` after the gap that started it,
    /// and the estimate follows the path.
    #[test]
    fn a_link_that_keeps_arriving_in_bursts_stops_settling() {
        let mut clock = ServerClock::default();
        for local in (0..3_000i64).step_by(16) {
            clock.observe_ms(local as f64 + 1_000.0, local);
        }
        // Three seconds of silence, then chunks every 250 ms on a path
        // 300 ms slower than before.
        let resumed = 6_000i64;
        let mut followed_at = None;
        for chunk in 0..24i64 {
            let at = resumed + chunk * 250;
            for i in 0..15i64 {
                clock.observe_ms((at - 14 + i) as f64 + 700.0, at);
            }
            let offset = clock.offset_ms().expect("an estimate");
            if at < resumed + CLOCK_SETTLE_MAX_MS {
                assert!(offset > 900.0, "still settling at {at}: {offset}");
            } else if followed_at.is_none() && (offset - 700.0).abs() < 1e-6 {
                followed_at = Some(at);
            }
        }
        let followed_at = followed_at.expect("the estimate followed the bursty path");
        assert!(followed_at <= resumed + CLOCK_SETTLE_MAX_MS + 250, "followed only at {followed_at}");
    }

    /// **A round across the hub clock's wrap reads one clock.** The
    /// room's `server_ms` is a `u32` of milliseconds since the hub
    /// started, which wraps every 49.7 days; a round that spans the wrap
    /// is played exactly as the same round far from it: render time never
    /// goes back, nothing is guessed or held, and the delay target is the
    /// same.
    #[test]
    fn a_round_across_the_hub_clocks_wrap_is_played_as_any_other() {
        let transit = 20i64;
        let run = |base: u32| {
            let arrivals: Vec<(Snapshot, i64)> = (0..=420u32)
                .map(|t| {
                    let mut s = snapshot(t);
                    s.server_ms = base.wrapping_add(stamp(t));
                    (s, exact(t) + transit)
                })
                .collect();
            let mut interp = Interpolator::default();
            interp.restart(&arrivals[0].0, arrivals[0].1);
            let (renders, extrapolated, held) = play(&mut interp, &arrivals[1..], transit, exact(420) + transit);
            (renders, extrapolated, held, interp.report())
        };
        // The clock wraps three seconds into the round.
        let base = u32::MAX - 3_000;
        assert!(base.wrapping_add(stamp(420)) < base, "the stamps cross the wrap");
        let (renders, extrapolated, held, wrapped) = run(base);
        assert!(renders.windows(2).all(|w| w[1] >= w[0] - 1e-3), "render time went back");
        assert_eq!(extrapolated, 0, "frames guessed across the wrap");
        assert_eq!(held, 0, "the picture held across the wrap");
        assert_eq!(wrapped.extrapolated_frames, 0, "{wrapped:?}");
        let (_, _, _, plain) = run(1_000);
        assert!((wrapped.target_ms - plain.target_ms).abs() < 1e-3, "the delay target moved: {wrapped:?} vs {plain:?}");
    }

    /// **A welcome after a lobby measures the link afresh.** A round on
    /// a 20 ms path, ten seconds of silence in the lobby, then the next
    /// round's welcome and snapshots on a path 60 ms slower: a clock kept
    /// from the round before would put the room 60 ms ahead of every
    /// snapshot, and the round would open on a run of extrapolated frames
    /// and a delay widened for lateness that was never there.
    #[test]
    fn a_welcome_after_a_long_lobby_measures_the_link_afresh() {
        let (fast, slow) = (20i64, 80i64);
        let round = |started: u32, transit: i64| -> Vec<(Snapshot, i64)> {
            (0..=180u32)
                .map(|t| {
                    let mut s = snapshot(t);
                    s.server_ms = started + stamp(t);
                    let at = s.server_ms as i64 + transit;
                    (s, at)
                })
                .collect()
        };
        let mut interp = Interpolator::default();
        let first = round(1_000, fast);
        interp.restart(&first[0].0, first[0].1);
        play(&mut interp, &first[1..], first[0].1, first[180].1);
        let settled = interp.clock().offset_ms().expect("a clock");
        assert!((settled + fast as f64).abs() < 1e-6, "the first round's path: {settled}");

        let second = round(1_000 + stamp(180) + 10_000, slow);
        interp.restart(&second[0].0, second[0].1);
        let (renders, extrapolated, held) = play(&mut interp, &second[1..], second[0].1, second[180].1);
        assert!(renders.windows(2).all(|w| w[1] >= w[0] - 1e-3), "render time went back");
        assert!(extrapolated <= 2, "the round opened on {extrapolated} extrapolated frames");
        assert_eq!(held, 0, "the picture held at the round's start");
        let report = interp.report();
        assert!(report.target_ms < 40.0, "the delay was sized for lateness that was never there: {report:?}");
        let offset = interp.clock().offset_ms().expect("a clock");
        assert!((offset + slow as f64).abs() < 1e-6, "the second round's path: {offset}");
    }

    /// **Lateness includes how late the room sent the tick.** A room that
    /// runs ticks 10, 11 and 12 of every twenty late sends them together
    /// with tick 13 - 50, 34 and 17 ms behind the anchor - over a link
    /// that is otherwise perfect: the clock sees every arrival on time, so
    /// only the send lateness can widen the delay to cover them. It does,
    /// and the picture waits for those ticks rather than guessing past
    /// them, never holding and never going back.
    #[test]
    fn the_delay_covers_the_ticks_a_room_sent_late() {
        let transit = 20i64;
        let sent_at = |t: u32| if (10..13).contains(&(t % 20)) { stamp(t - t % 20 + 13) } else { stamp(t) };
        let arrivals: Vec<(Snapshot, i64)> = (0..=420u32)
            .map(|t| {
                let mut s = snapshot(t);
                s.server_ms = sent_at(t);
                let at = s.server_ms as i64 + transit;
                (s, at)
            })
            .collect();
        assert_eq!(sent_at(10) - stamp(10), 50);
        let mut interp = Interpolator::default();
        interp.restart(&arrivals[0].0, arrivals[0].1);
        let (renders, extrapolated, held) = play(&mut interp, &arrivals[1..], transit, exact(420) + transit);
        assert!(renders.windows(2).all(|w| w[1] >= w[0] - 1e-3), "render time went back");
        assert_eq!(held, 0, "a late tick is not a schedule that moved");
        let report = interp.report();
        assert!(report.lateness_p95_ms >= 25.0, "the send lateness is lateness: {report:?}");
        assert!(report.target_ms >= 55.0, "and the delay covers it: {report:?}");
        assert!(extrapolated < 30, "{extrapolated} frames guessed past ticks the delay should cover");
    }

    /// A welcome whose reading is within `CLOCK_SNAP_MS` of the link's
    /// clock is the same link and keeps the clock; one further off is
    /// another server's and starts it again.
    #[test]
    fn a_welcome_keeps_the_link_clock_within_the_snap_and_resets_it_past() {
        let transit = 20i64;
        let fed = || {
            let mut interp = Interpolator::default();
            for tick in 0..120u32 {
                interp.accept(snapshot(tick), exact(tick) + transit);
            }
            interp
        };
        let mut welcome = snapshot(0);
        welcome.server_ms = stamp(120);
        let sent = stamp(120) as i64 + transit;

        let mut near = fed();
        let offset = near.clock().offset_ms().expect("a clock");
        let late = CLOCK_SNAP_MS as i64 - 100;
        near.restart(&welcome, sent + late);
        assert_eq!(near.clock().offset_ms(), Some(offset), "{late} ms off is the same link");

        let mut far = fed();
        let late = CLOCK_SNAP_MS as i64 + 100;
        far.restart(&welcome, sent + late);
        let reading = welcome.server_ms as f64 - (sent + late) as f64;
        assert_eq!(far.clock().offset_ms(), Some(reading), "{late} ms off is another clock, taken whole");
    }

    /// **A gappy link is its path, not a stall.** Snapshots come in bursts
    /// behind 120 ms gaps every 330 ms, and nothing ever outlasts the
    /// window, so no settling is ever in effect - a chain of pending ones
    /// extended from gap to gap would, once the last reading from before
    /// the first gap left the window, slow the clock's fall with no stall
    /// behind it.
    #[test]
    fn a_gappy_link_never_settles() {
        let mut clock = ServerClock::default();
        clock.set_gap_ms(STALL_INTERVALS * SNAPSHOT_INTERVAL_MS);
        let mut settled = Vec::new();
        for sent in (0..9_000i64).step_by(16) {
            let phase = sent % 330;
            let held = if phase < 120 { 120 - phase } else { 0 };
            let at = sent + 20 + held;
            clock.observe_ms(sent as f64, at);
            if clock.settling(at) {
                settled.push(at);
            }
        }
        assert!(settled.is_empty(), "settled with no stall behind it at {:?}", &settled[..settled.len().min(8)]);
    }
}
