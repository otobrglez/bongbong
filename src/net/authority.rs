//! The authority's half of a client-owned hull (docs/online-coop-prd.md
//! §4.14), shared by every room there is - the room server's
//! (`bongbong_server::room`), the rig's thread and `rig::Lockstep` - so a
//! pose is taken, refused and answered by one rule wherever the round is
//! played.
//!
//! One tick, in order: `take_pose` for every seat's intent before
//! `Game::update` (a pose puts the hull there, a packet without one
//! releases the seat to the room's own driving, a refused pose leaves the
//! hull and answers with `Placed`), then `moved_since` after it (a tick
//! that carried an owned hull further than a contact could - a portal, a
//! gate - answers with `Placed` too), and `stamp_presses` on the events
//! it sends, so a seat's `Fired` names the intent its press came on.
//!
//! **When** a room ticks is shared too (`TickClock`, docs/online-coop-prd.md
//! §4.16): tick n is due at a fixed point of wall time, whatever the ticks
//! before it took. Every client sends one intent per tick of *its* wall
//! clock, so a room whose schedule slipped with each late tick would run
//! slower than the seats feeding it, and each slip would stand in the
//! mailboxes as input waiting a tick longer. A late tick is followed by
//! the ones it held up, back to back, up to `CATCH_UP_TICKS`; a stall
//! longer than that starts the schedule again from now, the ticks it
//! cost dropped rather than run as a burst the clients would see as the
//! world lurching forward.

use std::time::{Duration, Instant};

use crate::math::Vec2 as Position;
use crate::net::MAX_SEATS;
use crate::net::events::WireEvent;
use crate::net::wire::{dir_index, quantise_pos};
use crate::simulation::{Game, SeatPose};
use crate::tank::Dir;

/// How far a tick may move a client-owned hull from the pose the client
/// gave before the client is told: the solver's own contact nudge is a
/// few pixels a tick, a portal or a gate is the width of the field.
pub const PLACED_PX: f32 = 24.0;

/// The most ticks a room runs back to back to catch up with its schedule
/// after a late one: 67 ms of stall is paid back at once, which a client
/// rides out on its interpolation buffer. Owing more starts the schedule
/// again from now (`TickClock::fire`).
pub const CATCH_UP_TICKS: u32 = 4;

/// A room's tick schedule on wall time: the next tick is due one `period`
/// after the last one was *due*, not after it ran, so the ticks keep the
/// clients' pace however long each takes. Stopped while the room is not
/// ticking (waiting, paused, ended, frozen by a dev tool), and started
/// afresh when it ticks again, so a pause is never paid back as a burst.
#[derive(Clone, Copy, Debug)]
pub struct TickClock {
    period: Duration,
    /// When the next tick is due; `None` while stopped.
    next: Option<Instant>,
}

/// How a tick started against its schedule (`TickClock::fire`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TickStart {
    /// How long after it was due the tick started.
    pub late: Duration,
    /// Ticks the schedule gave up on: past `CATCH_UP_TICKS` owed, it
    /// starts again from now rather than run them.
    pub dropped: u32,
}

impl TickClock {
    /// A stopped clock of one tick per `period`.
    pub fn new(period: Duration) -> TickClock {
        TickClock { period, next: None }
    }

    /// Run the schedule with its first tick due at `first`.
    pub fn start(&mut self, first: Instant) {
        self.next = Some(first);
    }

    /// Stop the schedule; `due` is `None` until the next `start`.
    pub fn stop(&mut self) {
        self.next = None;
    }

    pub fn running(&self) -> bool {
        self.next.is_some()
    }

    /// When the next tick is due, while the schedule runs.
    pub fn due(&self) -> Option<Instant> {
        self.next
    }

    /// The tick that was due is starting at `now`: how late it is, and
    /// the schedule moved on one period - or, when more than
    /// `CATCH_UP_TICKS` more are already owed, restarted a period from
    /// now with those dropped. A stopped clock starts on the spot.
    pub fn fire(&mut self, now: Instant) -> TickStart {
        let due = self.next.unwrap_or(now);
        let late = now.saturating_duration_since(due);
        let mut next = due + self.period;
        let mut dropped = 0;
        if now >= next {
            let owed = now.duration_since(next).as_nanos() / self.period.as_nanos().max(1) + 1;
            if owed > CATCH_UP_TICKS as u128 {
                dropped = owed.min(u32::MAX as u128) as u32;
                next = now + self.period;
            }
        }
        self.next = Some(next);
        TickStart { late, dropped }
    }
}

/// What one seat's pose did this tick.
#[derive(Clone, Debug, PartialEq)]
pub enum PoseOutcome {
    /// The hull is where the client said: this is where `moved_since`
    /// measures from.
    Applied(Position),
    /// The validator refused it (`Game::accept_seat_pose`'s reason); the
    /// hull stayed, and the event tells the client where.
    Refused(&'static str, Option<WireEvent>),
    /// The packet carried no pose: the room drives the seat again.
    Released,
}

/// Apply the pose one seat's intent carries, before the tick runs.
pub fn take_pose(game: &mut Game, seat: usize, pose: Option<SeatPose>) -> PoseOutcome {
    match pose {
        Some(pose) => match game.accept_seat_pose(seat, pose) {
            Ok(()) => PoseOutcome::Applied(pose.position),
            Err(why) => PoseOutcome::Refused(why, game.seat_pose(seat).map(|at| placed(seat, at))),
        },
        None => {
            game.release_seat(seat);
            PoseOutcome::Released
        }
    }
}

/// After the tick: a `Placed` if the tick carried the hull further than
/// `PLACED_PX` from where its pose put it.
pub fn moved_since(game: &Game, seat: usize, applied: Position) -> Option<WireEvent> {
    let at = game.seat_pose(seat)?;
    let (dx, dy) = (at.position.x - applied.x, at.position.y - applied.y);
    ((dx * dx + dy * dy).sqrt() > PLACED_PX).then(|| placed(seat, at))
}

/// Stamp every seat's `Fired` with the input tick its press came on
/// (`net::mailbox::Mailbox::press_tick`, one per seat), which the client
/// pairs the shot it drew with. `encode` stamps the ack; an owned seat's
/// tick merges several intents, so its press can be older than its ack,
/// and a room restamps the events it sends with this.
pub fn stamp_presses(events: &mut [WireEvent], presses: &[u32; MAX_SEATS]) {
    for event in events {
        if let WireEvent::Fired { slot, input_tick, .. } = event
            && let Some(&tick) = presses.get(*slot as usize)
        {
            *input_tick = tick;
        }
    }
}

/// The event that tells a client-owned seat where the room has it.
pub fn placed(seat: usize, at: SeatPose) -> WireEvent {
    WireEvent::Placed {
        seat: seat.min(u8::MAX as usize) as u8,
        x: quantise_pos(at.position.x),
        y: quantise_pos(at.position.y),
        dir: dir_index(Dir::from_rotation(at.rotation).unwrap_or(Dir::Up)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::wire::WeaponKind;

    const PERIOD: Duration = Duration::from_millis(10);

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// On time, every tick is due a period after the last was due - not
    /// after it ran - so a slow tick costs the schedule nothing.
    #[test]
    fn the_schedule_is_anchored_to_when_ticks_were_due() {
        let t0 = Instant::now();
        let mut clock = TickClock::new(PERIOD);
        assert_eq!(clock.due(), None, "a new clock is stopped");
        clock.start(t0 + PERIOD);
        assert_eq!(clock.fire(t0 + ms(10)), TickStart::default(), "on time");
        assert_eq!(clock.due(), Some(t0 + ms(20)));
        // Three milliseconds late: reported, and the next is still due
        // at 30, not 33.
        assert_eq!(clock.fire(t0 + ms(23)), TickStart { late: ms(3), dropped: 0 });
        assert_eq!(clock.due(), Some(t0 + ms(30)));
    }

    /// A stall of up to `CATCH_UP_TICKS` is paid back with ticks due at
    /// once; a longer one starts the schedule again from now.
    #[test]
    fn a_short_stall_is_caught_up_and_a_long_one_restarts_the_schedule() {
        let t0 = Instant::now();
        let mut clock = TickClock::new(PERIOD);
        clock.start(t0);
        // Due at 0, starts at 45: four more (10, 20, 30, 40) are owed.
        assert_eq!(clock.fire(t0 + ms(45)), TickStart { late: ms(45), dropped: 0 });
        let mut lates = Vec::new();
        while clock.due().unwrap() <= t0 + ms(45) {
            lates.push(clock.fire(t0 + ms(45)).late);
        }
        assert_eq!(lates, [ms(35), ms(25), ms(15), ms(5)], "caught up back to back");
        assert_eq!(clock.due(), Some(t0 + ms(50)), "and back on the original schedule");

        // Due at 50, starts at 105: five more are owed, past the cap.
        let start = clock.fire(t0 + ms(105));
        assert_eq!(start, TickStart { late: ms(55), dropped: 5 });
        assert_eq!(clock.due(), Some(t0 + ms(115)), "the schedule starts again a period from now");
    }

    #[test]
    fn a_stopped_clock_is_due_never_and_fires_on_the_spot() {
        let t0 = Instant::now();
        let mut clock = TickClock::new(PERIOD);
        clock.start(t0);
        clock.stop();
        assert!(!clock.running());
        assert_eq!(clock.due(), None);
        assert_eq!(clock.fire(t0 + ms(500)), TickStart::default(), "no schedule, so nothing is late");
        assert_eq!(clock.due(), Some(t0 + ms(510)));
    }

    /// A seat's `Fired` names its press; an enemy's, on a slot past the
    /// seats, keeps its zero.
    #[test]
    fn a_seats_fired_is_stamped_with_its_press() {
        let mut events = vec![
            WireEvent::Fired { slot: 0, weapon: WeaponKind::Shell, input_tick: 40 },
            WireEvent::Fired { slot: 1, weapon: WeaponKind::Shell, input_tick: 17 },
            WireEvent::Fired { slot: 30, weapon: WeaponKind::Shell, input_tick: 0 },
        ];
        let mut presses = [0; MAX_SEATS];
        presses[0] = 37;
        presses[1] = 17;
        stamp_presses(&mut events, &presses);
        let ticks: Vec<u32> = events
            .iter()
            .map(|e| match e {
                WireEvent::Fired { input_tick, .. } => *input_tick,
                other => panic!("not a Fired: {other:?}"),
            })
            .collect();
        assert_eq!(ticks, [37, 17, 0]);
    }
}
