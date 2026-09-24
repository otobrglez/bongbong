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
//! What is predicted is the own hull and the own shot leaving the
//! muzzle. Damage, pickups, other tanks and everyone else's shots stay
//! the server's, and stay interpolated.
//!
//! **A provisional shot is drawn, never simulated.** It flies by dead
//! reckoning and meets nothing: a replica runs no hit test, and whether
//! it hit is the server's word, arriving as an event like any other. It
//! is retired when the server reports this seat fired - oldest first,
//! since a seat's shots leave in the order the trigger was pulled and
//! come back in that order, which is what lets a provisional be
//! confirmed without `Fired` carrying a client tick, and so without a
//! protocol change. A shot the server never mentions was one it refused,
//! and goes quietly after `PROVISIONAL_MS`.
//!
//! **What still costs a correction**, deliberately, and what it costs:
//!
//! - *Firing.* `weapons::apply_recoil` pushes the shooter back along the
//!   shot's axis, and the sandbox never fires - it only drives. So every
//!   shot ends one nudge, bounded by `shell_recoil_max_speed` over one
//!   snapshot interval, and settled by the next reconciliation. Cheap
//!   enough to leave; predicting the recoil means predicting the shot,
//!   which is phase 5.
//! - *A speed boost* would be worse than a correction - it changes the
//!   top speed the drive model works to, so a sandbox that did not know
//!   would fall behind every tick for the whole buff. That one is
//!   carried: `reconcile` takes the flag the wire already sends.
//! - *A teleport* is not eased at all. The hull is somewhere else
//!   entirely, which is past `SNAP_PX` by construction, so it is taken
//!   whole - easing across a portal would drag the picture through the
//!   scenery between the two ends.
//! - *The turret* is not on the wire at all; the replica derives it, so
//!   it follows the hull this already predicts rather than lagging it.

use std::collections::VecDeque;

use crate::PHYSICS_FIXED_DT;
use crate::ai::Intent;
use crate::math::Vec2 as Position;
use crate::shell::{Owner, Shell};
use crate::simulation::Game;
use crate::tuning::tuning;

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

/// How long a shell nobody confirmed is left on screen.
///
/// A round trip plus an interval, generously: the server's `Fired` for a
/// shot it accepted cannot take longer than that to come back, so
/// anything still waiting here was refused - the trigger was pulled on a
/// cooldown the client had not seen, or with ammo the server had already
/// spent. Those are quiet failures by design (§4.12): the shell fades
/// rather than announcing that the client guessed wrong.
pub const PROVISIONAL_MS: i64 = 600;

/// The id band provisional shells are drawn under.
///
/// The server hands its projectiles a per-round counter and the wire
/// carries it as a `u16`, so the top of that range is free in any round
/// that does not fire sixty thousand shots. A provisional needs an id
/// only so the replica can hold it beside the server's own; it is never
/// sent anywhere.
pub const PROVISIONAL_ID_BASE: u32 = 0xF000;

/// A shot the client has drawn but the server has not confirmed.
///
/// The pose rather than a `Shell`: a `Shell` owns a `Vec` and so cannot
/// be copied into the replica, and it has to go back in every frame
/// because `net::apply` despawns everything the snapshot did not list.
/// `Shell::at` rebuilds one from these.
struct Provisional {
    position: Position,
    prev_position: Position,
    velocity: crate::math::Vec2,
    rotation: f32,
    variant: i32,
    shooter_row: i32,
    /// Local milliseconds at the trigger pull, for `PROVISIONAL_MS`.
    born_ms: i64,
}

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
    /// Shots drawn on the frame of the press, oldest first, waiting for
    /// the server to say they happened.
    shots: VecDeque<Provisional>,
    /// Seconds until the local gate will let another shot out. Counted
    /// down here rather than read off the sandbox, because
    /// `predict_seat` deliberately does not fire - it only drives.
    cooldown: f32,
    /// How many provisional shots were drawn, and how many of those the
    /// server never confirmed. A climbing refusal count means the local
    /// gate is looser than the server's.
    pub shots_drawn: u32,
    pub shots_refused: u32,
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
            shots: VecDeque::new(),
            cooldown: 0.0,
            shots_drawn: 0,
            shots_refused: 0,
        }
    }

    /// Draw a shot now, if the local gate allows one.
    ///
    /// **The gate is the client's guess at the server's.** It counts the
    /// same `player_fire_interval` down, but it cannot know about ammo
    /// the server has already spent or a cooldown set by a shot this
    /// client has not heard about yet. Guessing wrong is cheap and
    /// deliberate: the shell is drawn, no `Fired` ever comes back for it,
    /// and `expire` takes it away quietly (§4.12).
    ///
    /// `now_ms` is the local clock the expiry is measured on.
    pub fn fire(&mut self, now_ms: i64) {
        if self.cooldown > 0.0 {
            return;
        }
        // From the predicted pose, so the shell leaves the muzzle where
        // the hull is *now* rather than where the room last saw it -
        // which is the whole point of drawing it early.
        let Some(shell) = self.sandbox.seat_shell(self.seat) else { return };
        self.cooldown = tuning().player_fire_interval;
        self.shots.push_back(Provisional {
            position: shell.position,
            prev_position: shell.position,
            velocity: shell.velocity,
            rotation: shell.rotation,
            variant: shell.variant,
            shooter_row: shell.shooter_row,
            born_ms: now_ms,
        });
        self.shots_drawn += 1;
    }

    /// The server says this seat fired: the oldest shot still waiting was
    /// that one.
    ///
    /// Matched oldest-first rather than by an id, because a seat's shots
    /// leave in the order it pulled the trigger and the server reports
    /// them in that same order. That is what lets a provisional be
    /// confirmed without `Fired` carrying a client tick, and so without a
    /// protocol change.
    pub fn confirm_shot(&mut self) {
        self.shots.pop_front();
    }

    /// Drop shots the server never confirmed, and count them.
    pub fn expire(&mut self, now_ms: i64) {
        while self.shots.front().is_some_and(|p| now_ms - p.born_ms > PROVISIONAL_MS) {
            self.shots.pop_front();
            self.shots_refused += 1;
        }
    }

    /// Carry the unconfirmed shots forward by `dt` and run the local
    /// fire gate down. Dead reckoning: a provisional has no physics of
    /// its own and never hits anything - a hit is the server's word, and
    /// arrives as an event like any other.
    pub fn advance_shots(&mut self, dt: f32) {
        self.cooldown = (self.cooldown - dt).max(0.0);
        for p in &mut self.shots {
            p.prev_position = p.position;
            p.position = Position::new(p.position.x + p.velocity.x * dt, p.position.y + p.velocity.y * dt);
        }
    }

    /// The shots to draw beside the server's, with the ids they are held
    /// under in the replica.
    pub fn shots(&self) -> impl Iterator<Item = Shell> + '_ {
        let owner = Owner::Player(self.seat as u8);
        self.shots.iter().enumerate().map(move |(i, p)| {
            Shell::at(
                PROVISIONAL_ID_BASE + i as u32,
                p.position,
                p.prev_position,
                p.velocity,
                p.rotation,
                p.variant,
                p.shooter_row,
                owner,
            )
        })
    }

    /// How many shots are on screen that the server has not confirmed.
    pub fn provisional_count(&self) -> usize {
        self.shots.len()
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
        self.step_at(stamped, intent);
        stamped
    }

    /// `step`, on a tick somebody else stamped.
    ///
    /// This is the wired path: `RoomClient` owns the counter the packets
    /// carry, and the sandbox has to agree with it exactly, or the tick
    /// the server names back in `acked` would point at the wrong input.
    /// One counter, not two that could drift.
    pub fn step_at(&mut self, tick: u32, intent: Intent) {
        self.sandbox.predict_seat(self.seat, intent, PHYSICS_FIXED_DT);
        self.history.push_back((tick, intent));
        while self.history.len() > HISTORY_TICKS {
            self.history.pop_front();
        }
        self.tick = tick.wrapping_add(1);
    }

    /// Pull the sandbox back to what the server said, then replay
    /// everything it had not seen yet.
    ///
    /// `acked` is the last input tick the server applied for this seat,
    /// and the pose is where that left the hull. Every input after it is
    /// still in flight or still unacknowledged, so it is replayed on top
    /// - the same count of steps, the same `dt`, the same statics, which
    /// is why the answer lands rather than drifts.
    pub fn reconcile(&mut self, acked: u32, position: Position, rotation: f32, velocity: Position, boosted: bool) {
        let before = self.sandbox.seat_motion(self.seat).map(|(p, _, _)| p);
        self.sandbox.place_seat(self.seat, position, rotation, velocity, boosted);
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
        predictor.reconcile(59, pos, rot, vel, false);

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
        predictor.reconcile(39, nudged, rot, vel, false);

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

    /// **A boosted hull outruns a sandbox that does not know.**
    ///
    /// A SpeedUp changes the top speed the drive model works to
    /// (`Tank::speed`), so this is not a one-off correction that settles
    /// - it is a drift, every tick, for the whole buff. The wire carries
    /// the flag and the reconciliation has to pass it on; pickups
    /// themselves stay the server's.
    #[test]
    fn a_boost_the_server_reports_is_carried_into_the_sandbox() {
        let (mut authority, sandbox) = (round(), round());
        let mut predictor = Predictor::new(sandbox, 0, 0);
        // The server's hull picks up a SpeedUp.
        authority.place_seat(0, authority.seat_motion(0).expect("a hull").0, 0.0, Position::new(0.0, 0.0), true);
        let (pos, rot, vel) = authority.seat_motion(0).expect("a hull");
        predictor.reconcile(0, pos, rot, vel, true);

        // Both now run the same script; a sandbox told about the boost
        // keeps up, one that was not falls behind every tick.
        for tick in 1..60u32 {
            authority.predict_seat(0, script(tick), PHYSICS_FIXED_DT);
            predictor.step(script(tick));
        }
        let a = authority.seat_motion(0).expect("a hull").0;
        let p = predictor.motion().expect("a hull").0;
        assert_eq!((a.x, a.y), (p.x, p.y), "the boosted hull drifted away from its own prediction");
    }

    /// The point of a provisional shot: it is on screen on the frame of
    /// the press, and it leaves from where the hull is *now*.
    #[test]
    fn a_shot_is_drawn_from_the_predicted_muzzle_at_once() {
        let sandbox = round();
        let mut predictor = Predictor::new(sandbox, 0, 0);
        for tick in 0..30u32 {
            predictor.step(script(tick));
        }
        let hull = predictor.motion().expect("a hull").0;
        predictor.fire(0);
        assert_eq!(predictor.provisional_count(), 1, "the shot should be on screen at once");
        assert_eq!(predictor.shots_drawn, 1);

        let shell = predictor.shots().next().expect("a shell");
        let gap = ((shell.position.x - hull.x).powi(2) + (shell.position.y - hull.y).powi(2)).sqrt();
        assert!(gap > 1.0, "the shell should leave the muzzle, not the hull's centre");
        assert!(gap < 100.0, "but it is a muzzle, not a mortar: {gap}");
        // And it flies.
        let before = shell.position;
        predictor.advance_shots(PHYSICS_FIXED_DT);
        let after = predictor.shots().next().expect("a shell").position;
        assert_ne!((before.x, before.y), (after.x, after.y), "the shell did not move");
    }

    /// The local gate is the client's guess at the server's, and it has
    /// to hold the trigger down to one shot per `player_fire_interval` -
    /// otherwise holding fire paints the screen with shots the server
    /// will refuse.
    #[test]
    fn the_local_gate_rations_a_held_trigger() {
        let mut predictor = Predictor::new(round(), 0, 0);
        for _ in 0..10 {
            predictor.fire(0);
        }
        assert_eq!(predictor.provisional_count(), 1, "a held trigger drew more than one shot");
        // Past the interval, another is allowed.
        predictor.advance_shots(tuning().player_fire_interval + 0.001);
        predictor.fire(0);
        assert_eq!(predictor.provisional_count(), 2);
    }

    /// The server's word retires a shot; oldest first, since a seat's
    /// shots leave in the order the trigger was pulled and come back in
    /// that order.
    #[test]
    fn the_servers_fired_retires_the_oldest_shot() {
        let mut predictor = Predictor::new(round(), 0, 0);
        predictor.fire(0);
        predictor.advance_shots(tuning().player_fire_interval + 0.001);
        predictor.fire(0);
        assert_eq!(predictor.provisional_count(), 2);
        predictor.confirm_shot();
        assert_eq!(predictor.provisional_count(), 1);
        predictor.confirm_shot();
        assert_eq!(predictor.provisional_count(), 0);
        assert_eq!(predictor.shots_refused, 0, "a confirmed shot is not a refused one");
    }

    /// A shot the server never mentions was refused - the trigger was
    /// pulled on a cooldown or an ammo count this client had not caught
    /// up with - and it goes quietly rather than hanging on screen.
    #[test]
    fn a_shot_the_server_never_confirms_expires_quietly() {
        let mut predictor = Predictor::new(round(), 0, 0);
        predictor.fire(0);
        predictor.expire(PROVISIONAL_MS);
        assert_eq!(predictor.provisional_count(), 1, "not yet: it is still within a round trip");
        predictor.expire(PROVISIONAL_MS + 1);
        assert_eq!(predictor.provisional_count(), 0);
        assert_eq!(predictor.shots_refused, 1, "and it is counted, so a loose gate is visible");
    }

    /// A teleport is not a correction to ease across - the hull is
    /// somewhere else entirely - and `SNAP_PX` is what makes it a jump
    /// rather than a slide through the scenery.
    #[test]
    fn a_teleport_is_taken_whole() {
        let (authority, sandbox) = (round(), round());
        let mut predictor = Predictor::new(sandbox, 0, 0);
        for tick in 0..30u32 {
            predictor.step(script(tick));
        }
        let (pos, rot, vel) = authority.seat_motion(0).expect("a hull");
        // The other side of the field, which is what a portal does.
        let far = Position::new(pos.x + 500.0, pos.y + 300.0);
        predictor.reconcile(29, far, rot, vel, false);
        assert_eq!((predictor.nudges, predictor.snaps), (0, 1), "a teleport should snap, not ease");
        assert_eq!((predictor.offset().x, predictor.offset().y), (0.0, 0.0));
        let now = predictor.motion().expect("a hull").0;
        assert!((now.x - far.x).abs() < 1.0, "the prediction should be where the portal put it");
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
        predictor.reconcile(19, Position::new(pos.x + SNAP_PX + 1.0, pos.y), rot, vel, false);
        assert_eq!((predictor.nudges, predictor.snaps), (0, 1));
        assert_eq!((predictor.offset().x, predictor.offset().y), (0.0, 0.0), "a snap leaves nothing to ease");
    }
}
