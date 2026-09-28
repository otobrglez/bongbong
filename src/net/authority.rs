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
//! gate - answers with `Placed` too).

use crate::math::Vec2 as Position;
use crate::net::events::WireEvent;
use crate::net::wire::{dir_index, quantise_pos};
use crate::simulation::{Game, SeatPose};
use crate::tank::Dir;

/// How far a tick may move a client-owned hull from the pose the client
/// gave before the client is told: the solver's own contact nudge is a
/// few pixels a tick, a portal or a gate is the width of the field.
pub const PLACED_PX: f32 = 24.0;

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

/// The event that tells a client-owned seat where the room has it.
pub fn placed(seat: usize, at: SeatPose) -> WireEvent {
    WireEvent::Placed {
        seat: seat.min(u8::MAX as usize) as u8,
        x: quantise_pos(at.position.x),
        y: quantise_pos(at.position.y),
        dir: dir_index(Dir::from_rotation(at.rotation).unwrap_or(Dir::Up)),
    }
}
