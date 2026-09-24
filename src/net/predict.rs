//! Your own tank, on the frame you pressed the key
//! (docs/online-coop-prd.md §4.12).
//!
//! Stage 1 draws every hull, your own included, `online_interpolation_delay_ms`
//! behind the server plus half a round trip. That is fine for the eight
//! tanks you are not steering and wrong for the one you are. Stage 2 runs
//! your seat's locomotion locally and reconciles it against the server's
//! answer as snapshots arrive - rewind and replay, the Quake 3 technique,
//! which has an easy time here because locomotion is a pure function of
//! intent against a world that does not move.
//!
//! **The sandbox is a whole `Game`**, built the way `net::apply::welcome`
//! builds the replica: same map, same seed, so the walls, the obstacles
//! and the deep-water boxes are the server's own by construction. That is
//! the property the whole technique rests on - a replay that steps
//! against different statics does not converge, it drifts - and building
//! them a second way would be a second thing to keep in step. Nothing but
//! `Game::predict_seat` is ever called on it, so no RNG is drawn, nothing
//! ages, and nothing but the one hull moves.
//!
//! What is predicted is the own hull alone. Damage, pickups, other tanks
//! and every shot stay the server's, and stay interpolated; shots are
//! phase 5.

use std::collections::VecDeque;

use crate::PHYSICS_FIXED_DT;
use crate::math::Vec2 as Position;
use crate::ai::Intent;
use crate::simulation::Game;

/// How many ticks of input to keep. A replay starts at the last tick the
/// server acknowledged, so this has to cover the worst round trip worth
/// keeping: 120 ticks is two seconds at 60 Hz, well past the point where
/// a link is unplayable for other reasons.
pub const HISTORY_TICKS: usize = 120;

/// Prediction error below this is not a correction at all. A quarter
/// pixel is the wire's own resolution (`wire::pos` quantises to it), so
/// anything smaller is a number the server never actually told us.
pub const IGNORE_PX: f32 = 0.25;

/// Error past this is not nudged but snapped. A hull is 64 px wide; being
/// most of one out means the sandbox and the server disagree about
/// something structural - a wall taken on the other side, a teleport - and
/// easing across it would drag the hull through the scenery.
pub const SNAP_PX: f32 = 48.0;

/// How long a nudge takes to die away. Short enough that a correction is
/// over before the next one lands (snapshots are 50 ms apart), long
/// enough that it reads as the hull settling rather than jumping.
pub const NUDGE_SECONDS: f32 = 0.1;

/// The local seat's own tank, run ahead of the server and pulled back
/// into line by it.
pub struct Predictor {
    /// A `Game` used for nothing but `predict_seat`.
    sandbox: Game,
    /// Which seat this window is playing.
    seat: usize,
    /// The tick the next input will be stamped with.
    tick: u32,
    /// The inputs since the last acknowledged tick, oldest first.
    history: VecDeque<(u32, Intent)>,
    /// Where the drawn hull sits relative to the predicted one, decaying
    /// to nothing. A correction moves the *prediction* at once - the
    /// physics has to be right or the next tick compounds the error - and
    /// leaves this behind so the picture does not jump.
    offset: Position,
    /// How many corrections were past `IGNORE_PX`, and how many snapped:
    /// the numbers §4.12 asks to be measured before a feel pass.
    pub nudges: u32,
    pub snaps: u32,
}

impl Predictor {
    /// A predictor over `sandbox`, which must already be a round built on
    /// the server's map and seed with this seat's tank in it.
    pub fn new(sandbox: Game, seat: usize, first_tick: u32) -> Predictor {
        Predictor {
            sandbox,
            seat,
            tick: first_tick,
            history: VecDeque::with_capacity(HISTORY_TICKS),
            offset: Position::new(0.0, 0.0),
            nudges: 0,
            snaps: 0,
        }
    }

    /// The tick the next `step` will stamp.
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// Run one tick on `intent`, remember it, and return the tick it was
    /// stamped with - which is what the client puts on the wire, so the
    /// server's `acked` can name it back.
    pub fn step(&mut self, intent: Intent) -> u32 {
        let stamped = self.tick;
        self.sandbox.predict_seat(self.seat, intent, PHYSICS_FIXED_DT);
        self.history.push_back((stamped, intent));
        while self.history.len() > HISTORY_TICKS {
            self.history.pop_front();
        }
        self.tick = self.tick.wrapping_add(1);
        stamped
    }

    /// Pull the sandbox back to what the server said, then replay
    /// everything it had not seen yet.
    ///
    /// `acked` is the last input tick the server applied for this seat,
    /// and the pose is where that left the hull. Every input after it is
    /// still in flight or still unacknowledged, so it is replayed on top
    /// - the same count of steps, the same `dt`, the same statics, which
    /// is why the answer lands rather than drifts.
    pub fn reconcile(&mut self, acked: u32, position: Position, rotation: f32, velocity: Position) {
        let before = self.sandbox.seat_motion(self.seat).map(|(p, _, _)| p);
        self.sandbox.place_seat(self.seat, position, rotation, velocity);
        // Anything the server has already accounted for is history.
        while self.history.front().is_some_and(|(t, _)| *t <= acked) {
            self.history.pop_front();
        }
        let replay: Vec<Intent> = self.history.iter().map(|(_, i)| *i).collect();
        for intent in replay {
            self.sandbox.predict_seat(self.seat, intent, PHYSICS_FIXED_DT);
        }
        let Some(before) = before else { return };
        let Some((after, _, _)) = self.sandbox.seat_motion(self.seat) else { return };
        // The picture was already showing `before`; carry the difference
        // as an offset so the hull eases rather than teleports.
        let error = Position::new(before.x - after.x, before.y - after.y);
        let distance = (error.x * error.x + error.y * error.y).sqrt();
        if distance <= IGNORE_PX {
            return;
        }
        if distance >= SNAP_PX {
            self.offset = Position::new(0.0, 0.0);
            self.snaps += 1;
            return;
        }
        self.offset = Position::new(self.offset.x + error.x, self.offset.y + error.y);
        self.nudges += 1;
    }

    /// Let the visual offset die away. Called once per rendered frame
    /// with real time, not per tick: it is a property of the picture.
    pub fn decay(&mut self, dt: f32) {
        let keep = 1.0 - (dt / NUDGE_SECONDS).clamp(0.0, 1.0);
        self.offset = Position::new(self.offset.x * keep, self.offset.y * keep);
    }

    /// Where the hull actually is in the sandbox, which is what the next
    /// tick and the next reconciliation reason about.
    pub fn motion(&self) -> Option<(Position, f32, Position)> {
        self.sandbox.seat_motion(self.seat)
    }

    /// Where to draw it: the prediction plus whatever correction has not
    /// died away yet.
    pub fn drawn_position(&self) -> Option<Position> {
        let (p, _, _) = self.motion()?;
        Some(Position::new(p.x + self.offset.x, p.y + self.offset.y))
    }

    /// The correction still being eased off, for the metrics §4.12 asks
    /// for and for a test to assert on.
    pub fn offset(&self) -> Position {
        self.offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::MapFile;
    use crate::tank::Dir;

    const MAP: &str = include_str!("../../maps/default.toml");

    /// A round on the shipped map with no enemies, which is all a
    /// locomotion test needs - and no enemies means nothing else in the
    /// world for the solver to meet, so a difference is the hull's.
    fn round() -> Game {
        let mut game = Game::default();
        game.seed_override = Some(0xB0B5);
        game.enemy_count_override = Some(0);
        game.level_overrides.spawn = Some(crate::level::SpawnKind::Band);
        game.map = MapFile::from_toml_str(MAP).expect("the default map parses");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        game
    }

    /// A short script with turns in it, so the hull actually meets the
    /// solver rather than sliding down one axis.
    fn script(tick: u32) -> Intent {
        let dir = match (tick / 12) % 4 {
            0 => Dir::Up,
            1 => Dir::Right,
            2 => Dir::Down,
            _ => Dir::Left,
        };
        Intent { move_dir: Some(dir), ..Intent::default() }
    }

    /// **The property the whole technique rests on.** Two rounds built
    /// the same way, stepped on the same inputs, must agree exactly -
    /// otherwise a replay does not converge on the server's answer, it
    /// wanders away from it.
    #[test]
    fn a_sandbox_steps_a_hull_exactly_as_the_authority_does() {
        let (mut authority, sandbox) = (round(), round());
        let mut predictor = Predictor::new(sandbox, 0, 0);
        for tick in 0..180 {
            let intent = script(tick);
            authority.predict_seat(0, intent, PHYSICS_FIXED_DT);
            predictor.step(intent);
        }
        let (a, ar, av) = authority.seat_motion(0).expect("the authority's hull");
        let (p, pr, pv) = predictor.motion().expect("the predicted hull");
        assert_eq!((a.x, a.y), (p.x, p.y), "position drifted");
        assert_eq!(ar, pr, "facing drifted");
        assert_eq!((av.x, av.y), (pv.x, pv.y), "velocity drifted");
    }

    /// Reconciliation proper: the sandbox runs ahead, the server's answer
    /// arrives for an older tick, and replaying the inputs it had not
    /// seen has to land back on what the sandbox already had - because
    /// the server applied those same inputs to the same world.
    #[test]
    fn replaying_the_unacknowledged_inputs_lands_where_the_sandbox_already_was() {
        let (mut authority, sandbox) = (round(), round());
        let mut predictor = Predictor::new(sandbox, 0, 0);

        // The client is 10 ticks ahead: the server has applied 0..=59,
        // the client has predicted 0..=69.
        for tick in 0..70u32 {
            predictor.step(script(tick));
        }
        for tick in 0..60u32 {
            authority.predict_seat(0, script(tick), PHYSICS_FIXED_DT);
        }
        let predicted_before = predictor.motion().expect("a hull").0;

        let (pos, rot, vel) = authority.seat_motion(0).expect("the authority's hull");
        predictor.reconcile(59, pos, rot, vel);

        let after = predictor.motion().expect("a hull").0;
        assert_eq!((predicted_before.x, predicted_before.y), (after.x, after.y), "the replay did not land");
        // Nothing moved, so nothing to ease off and nothing to count.
        assert_eq!((predictor.offset().x, predictor.offset().y), (0.0, 0.0));
        assert_eq!((predictor.nudges, predictor.snaps), (0, 0));
    }

    /// When the server *does* disagree - it saw an input the client did
    /// not, or the other way about - the prediction moves at once and the
    /// picture is left behind to catch up, rather than the hull jumping.
    #[test]
    fn a_disagreement_moves_the_prediction_and_eases_the_picture() {
        let (mut authority, sandbox) = (round(), round());
        let mut predictor = Predictor::new(sandbox, 0, 0);
        for tick in 0..40u32 {
            predictor.step(script(tick));
            authority.predict_seat(0, script(tick), PHYSICS_FIXED_DT);
        }
        // The server's hull is a few pixels off where we had it.
        let (pos, rot, vel) = authority.seat_motion(0).expect("a hull");
        let nudged = Position::new(pos.x + 6.0, pos.y);
        let drawn_before = predictor.drawn_position().expect("a hull");
        predictor.reconcile(39, nudged, rot, vel);

        assert_eq!(predictor.nudges, 1, "a real difference should count");
        assert_eq!(predictor.snaps, 0);
        // The prediction took the server's answer...
        let (now, _, _) = predictor.motion().expect("a hull");
        assert!((now.x - nudged.x).abs() < 0.001, "the prediction should be the server's");
        // ...while the drawn hull has not moved yet.
        let drawn_after = predictor.drawn_position().expect("a hull");
        assert!(
            (drawn_after.x - drawn_before.x).abs() < 0.001,
            "the picture jumped: {} -> {}",
            drawn_before.x,
            drawn_after.x
        );
        // And the offset dies away.
        predictor.decay(NUDGE_SECONDS);
        assert!(predictor.offset().x.abs() < 0.001, "the nudge should be spent");
    }

    /// Past a hull's width the difference is structural, so it is taken
    /// whole rather than eased across - dragging the picture through a
    /// wall would look worse than the jump.
    #[test]
    fn a_difference_past_a_hull_width_snaps() {
        let (mut authority, sandbox) = (round(), round());
        let mut predictor = Predictor::new(sandbox, 0, 0);
        for tick in 0..20u32 {
            predictor.step(script(tick));
            authority.predict_seat(0, script(tick), PHYSICS_FIXED_DT);
        }
        let (pos, rot, vel) = authority.seat_motion(0).expect("a hull");
        predictor.reconcile(19, Position::new(pos.x + SNAP_PX + 1.0, pos.y), rot, vel);
        assert_eq!((predictor.nudges, predictor.snaps), (0, 1));
        assert_eq!((predictor.offset().x, predictor.offset().y), (0.0, 0.0), "a snap leaves nothing to ease");
    }
}
