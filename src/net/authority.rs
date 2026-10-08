//! The authority's half of a client-owned hull (docs/online-coop-prd.md
//! §4.14), shared by every room there is - the room server's
//! (`bongbong_server::room`), the rig's thread and `rig::Lockstep` - so a
//! pose is taken, refused and answered by one rule wherever the round is
//! played.
//!
//! One tick, in order: `take_pose` for every seat's intent before
//! `Game::update` (a pose puts the hull there, if it is no further from
//! the room's copy than the driving its mailbox read covers -
//! `Mailbox::pose_reach_ticks`, a whole stall for the burst that ends
//! one -, a packet without one releases the seat to the room's own
//! driving, a refused pose leaves the hull and answers with `Placed`),
//! then `moved_since` after it (a tick
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
/// `reach_ticks` is how much of the client's driving the mailbox read
/// that delivered it covers (`net::mailbox::Mailbox::pose_reach_ticks`),
/// which is how far from the room's copy of the hull the validator lets
/// the pose stand (`Game::accept_seat_pose`).
pub fn take_pose(game: &mut Game, seat: usize, pose: Option<SeatPose>, reach_ticks: u32) -> PoseOutcome {
    match pose {
        Some(pose) => match game.accept_seat_pose(seat, pose, reach_ticks) {
            Ok(()) => PoseOutcome::Applied(pose.position),
            Err(why) => PoseOutcome::Refused(why, game.seat_pose(seat).map(|at| placed(seat, at))),
        },
        None => {
            game.release_seat(seat);
            PoseOutcome::Released
        }
    }
}

/// Hand the round one seat's hold report for this tick
/// (`net::mailbox::Mailbox::hold_ticks`, docs/gauss-rail.md "The hold
/// report"), before the tick runs: a charge-and-hold trigger counts the
/// client's ticks, within `tank::CHARGE_HOLD_SPARE_TICKS` of its own.
pub fn take_hold(game: &mut Game, seat: usize, hold: Option<u32>) {
    game.set_seat_hold(seat, hold);
}

/// Hand the round one seat's reticle report for this tick
/// (`net::mailbox::Mailbox::reticle`, docs/rod-from-god.md "The reticle
/// report"), before the tick runs: a rod's reticle stands on the cell the
/// client has it on, held to the reticle's range of the room's hull.
pub fn take_reticle(game: &mut Game, seat: usize, reticle: u16) {
    let cols = crate::net::encode::field_cols(game);
    game.set_seat_reticle(seat, crate::net::encode::reticle_from_code(cols, reticle));
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
    use crate::PHYSICS_FIXED_DT;
    use crate::map::MapFile;
    use crate::net::mailbox::{Mailbox, REACH_SPARE_TICKS};
    use crate::net::wire::{IntentMsg, WeaponKind};
    use crate::simulation::{Input, POSE_REACH_SLACK_PX, POSE_REACH_TICKS, PlayerCount};

    const PERIOD: Duration = Duration::from_millis(10);

    /// A one-seat round on an open field with no enemy and no intro.
    fn open_round() -> Game {
        let mut game = Game::default();
        game.enemy_count_override = Some(0);
        game.seed_override = Some(7);
        game.player_row_override = Some(0);
        game.players = PlayerCount::ONE;
        game.show_intro = false;
        game.map = MapFile::from_toml_str(
            r#"
version = 1
tanks = 1
cells."2,2" = { kind = "frog" }
cells."8,8" = { kind = "start" }
"#,
        )
        .expect("the map parses");
        let (width, height) = game.map.field_size();
        game.init(width, height);
        game
    }

    /// One tick of a room's seat 0, the room server's order: read the
    /// mailbox, put its pose through `take_pose` with the reach the read
    /// vouches for, then run the update.
    fn room_tick(game: &mut Game, mailbox: &Mailbox, now: Instant) -> PoseOutcome {
        let read = mailbox.read(now);
        let outcome = take_pose(game, 0, read.and_then(|m| m.pose()), mailbox.pose_reach_ticks());
        let (width, height) = game.map.field_size();
        game.update(Input::single(read.map(|m| m.intent()).unwrap_or_default()), PHYSICS_FIXED_DT, width, height);
        outcome
    }

    /// **The burst that ends a stall is taken; a jump is not**
    /// (docs/online-coop-prd.md §4.14, §4.16). An owned read takes every
    /// intent at or before its play point, so after a stall one pose
    /// carries the client's whole drive through it, and the room believes it as far as the
    /// read covers. A single tick's packet still vouches for one tick, so
    /// a hull that claims to have crossed three cells in it is refused,
    /// stays where it was and is told so; and no read vouches for more
    /// than the room's own ticks since the last pose, whatever ticks the
    /// client stamps.
    #[test]
    fn an_owned_burst_after_a_stall_is_taken_and_a_jump_is_not() {
        let mut game = open_round();
        let mailbox = Mailbox::new();
        let now = Instant::now();
        let start = game.seat_pose(0).expect("the seat");
        let speed = game.tank_snapshots()[0].top_speed;
        let step = speed * PHYSICS_FIXED_DT;
        let y = start.position.y;
        let pose_at = |tick: u32, x: f32, vx: f32| {
            IntentMsg { tick, ..IntentMsg::default() }.with_pose(SeatPose {
                position: Position::new(x, y),
                rotation: Dir::Right.rotation(),
                velocity: Position::new(vx, 0.0),
            })
        };
        // Standing still, one packet a tick: every pose taken.
        for tick in 0..5 {
            mailbox.post(pose_at(tick, start.position.x, 0.0), now);
            let outcome = room_tick(&mut game, &mailbox, now);
            assert!(matches!(outcome, PoseOutcome::Applied(_)), "tick {tick}: {outcome:?}");
        }
        // The link stalls ten ticks while the client drives off at top
        // speed; the room holds the still hull where it was.
        let stall = 10u32;
        for _ in 0..stall {
            let outcome = room_tick(&mut game, &mailbox, now);
            assert!(matches!(outcome, PoseOutcome::Applied(_)), "a still guess is taken: {outcome:?}");
        }
        // Then the ten packets arrive at once.
        for n in 1..=stall {
            mailbox.post(pose_at(4 + n, start.position.x + step * n as f32, speed), now);
        }
        let travelled = step * stall as f32;
        let floor = step * POSE_REACH_TICKS + POSE_REACH_SLACK_PX;
        assert!(travelled > floor, "the fixture drives past the validator's floor: {travelled} px vs {floor} px");
        let outcome = room_tick(&mut game, &mailbox, now);
        assert!(matches!(outcome, PoseOutcome::Applied(_)), "the stall's burst was refused: {outcome:?}");
        let here = game.seat_pose(0).expect("the seat");
        assert!(
            (here.position.x - (start.position.x + travelled)).abs() < 1.0,
            "the room's hull is not where the client drove it: {here:?}, {travelled} px on from {start:?}"
        );

        // One packet, one tick on, three cells further: refused.
        let jump = pose_at(4 + stall + 1, here.position.x + 96.0, speed);
        mailbox.post(jump, now);
        let outcome = room_tick(&mut game, &mailbox, now);
        assert!(
            matches!(outcome, PoseOutcome::Refused("further than the hull could have gone", Some(WireEvent::Placed { seat: 0, .. }))),
            "a one-tick jump was taken: {outcome:?}"
        );
        // The tick drove the seat itself from where it was, so it coasts
        // a step at most - nowhere near the pose.
        let stayed = game.seat_pose(0).expect("the seat");
        assert!(stayed.position.x - here.position.x <= step + 0.5, "a refused pose moved the hull: {stayed:?}");

        // A packet stamped a hundred ticks on claims a hundred ticks of
        // driving; the room believes it for its own ticks since the last
        // pose, and a pose that far off is refused like the jump.
        let claimed = 100.0;
        mailbox.post(pose_at(4 + stall + 100, stayed.position.x + step * claimed, speed), now);
        let _ = room_tick(&mut game, &mailbox, now);
        let outcome = room_tick(&mut game, &mailbox, now);
        assert!(mailbox.pose_reach_ticks() <= 2 + REACH_SPARE_TICKS, "reach {}", mailbox.pose_reach_ticks());
        assert!(matches!(outcome, PoseOutcome::Refused(..)), "a claimed stall was believed: {outcome:?}");
    }

    /// **A room that stood still believes the driving its stall cost.**
    /// A room held up 200 ms finds its client twelve ticks on, and the
    /// read after the stall - `BUFFER_MAX` or more behind, so its play
    /// point starts again - carries all of that driving in one pose. The
    /// room ran one read since the last pose, but the wall clock ran
    /// twelve ticks, and the pose is believed for those.
    #[test]
    fn an_owned_pose_after_a_room_stall_is_taken() {
        let mut game = open_round();
        let mailbox = Mailbox::new();
        let dt = Duration::from_secs_f32(PHYSICS_FIXED_DT);
        let mut now = Instant::now();
        let start = game.seat_pose(0).expect("the seat");
        let speed = game.tank_snapshots()[0].top_speed;
        let step = speed * PHYSICS_FIXED_DT;
        let x_at = |tick: u32| start.position.x + step * tick as f32;
        let pose_at = |tick: u32| {
            IntentMsg { tick, ..IntentMsg::default() }.with_pose(SeatPose {
                position: Position::new(x_at(tick), start.position.y),
                rotation: Dir::Right.rotation(),
                velocity: Position::new(speed, 0.0),
            })
        };
        // Driving right at top speed, a packet and a tick a tick.
        let mut tick = 0u32;
        for _ in 0..10 {
            mailbox.post(pose_at(tick), now);
            let outcome = room_tick(&mut game, &mailbox, now);
            assert!(matches!(outcome, PoseOutcome::Applied(_)), "tick {tick}: {outcome:?}");
            tick += 1;
            now += dt;
        }
        // The room stalls 200 ms while the packets keep coming.
        let stall = 12u32;
        for _ in 0..stall {
            mailbox.post(pose_at(tick), now);
            tick += 1;
            now += dt;
        }
        let floor = step * POSE_REACH_TICKS + POSE_REACH_SLACK_PX;
        let driven = step * stall as f32;
        assert!(driven > floor, "the fixture drives past the validator's floor: {driven} px vs {floor} px");
        // Then it ticks again: the pose after the stall is taken.
        for _ in 0..4 {
            mailbox.post(pose_at(tick), now);
            let outcome = room_tick(&mut game, &mailbox, now);
            assert!(matches!(outcome, PoseOutcome::Applied(_)), "a pose after the stall was refused at tick {tick}: {outcome:?}");
            tick += 1;
            now += dt;
        }
        let here = game.seat_pose(0).expect("the seat");
        let acked = mailbox.acked_tick();
        assert!(
            (here.position.x - x_at(acked)).abs() < 1.0,
            "the room's hull is not where the client drove it: {here:?}, tick {acked} at {} px",
            x_at(acked)
        );
    }

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
