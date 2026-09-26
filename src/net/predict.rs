//! Your own tank, on the frame you pressed the key
//! (docs/online-coop-prd.md §4.12).
//!
//! Stage 1 draws every hull, your own included, the interpolation delay
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
//! What is predicted is the own hull, and the own shot leaving the
//! muzzle. Damage, pickups, other tanks and everyone else's shots stay
//! the server's, and stay interpolated.
//!
//! **Provisional shots** (`online_predict_shots`, on by default). A shot
//! is drawn from the predicted muzzle on the packet its press travels on,
//! gated the way the server gates it: the sandbox's seat carries the
//! server's active weapon and ammo as of the last snapshot (they are on
//! the wire), the cooldown is the weapon's own, and the ammo still to be
//! confirmed is subtracted, so a press the server would refuse draws
//! nothing. Shells and plasma fire on the press edge, the minigun while
//! the key is held - `drive_player`'s rule. A press opens a `Press`: the
//! first shot at once and the rest on their own delays (a twin barrel's
//! second shell `tank_twin_shot_delay_seconds` on, the burst's bullets
//! `minigun_bullet_delay_seconds` apart), each spawned from the sandbox's
//! pose on the frame it is due, as the server's `tick_queued_shots` would.
//! A provisional flies by dead reckoning and meets nothing: a replica runs
//! no hit test, and a hit is the server's word.
//!
//! A press is **retired against the server's own `Fired`**, oldest first,
//! since a seat's shots leave in the order the trigger was pulled and
//! come back in that order - which is what lets a provisional be matched
//! without `Fired` carrying a client tick, and so without a protocol
//! change. It happens on the frame the interpolator hands that `Fired`
//! over, which is the frame the server's shot appears in the replica, so
//! nothing flickers; a press's later shots go their own delay after, as
//! theirs appear. A press no `Fired` ever claims was one the server
//! refused and goes quietly after `PROVISIONAL_SECONDS`, counted.
//!
//! It is still a lie: a shell drawn at the present passes through tanks
//! drawn `online_interpolation_delay_ms` in the past, an error of the
//! delay times `shell_speed` - sixteen pixels at 33 ms, a quarter of a
//! hull, against a shot that answers the press on the frame it is pressed
//! instead of a round trip later. What would make it correct rather than
//! merely early is the server rewinding targets to the shooter's view
//! (§4.12, decision 9), which is not built.
//!
//! **What still costs a correction**, deliberately, and what it costs:
//!
//! - *Firing.* `weapons::apply_recoil` pushes the shooter back along the
//!   shot's axis, and the sandbox never fires - it only drives. So every
//!   shot ends one nudge, bounded by `shell_recoil_max_speed` over one
//!   snapshot interval, and settled by the next reconciliation.
//! - *A speed boost* would be worse than a correction - it changes the
//!   top speed the drive model works to, so a sandbox that did not know
//!   would fall behind every tick for the whole buff. That one is
//!   carried: `reconcile` takes the whole snapshot, flag included.
//! - *A teleport* is not eased at all. The hull is somewhere else
//!   entirely, which is past `SNAP_PX` by construction, so it is taken
//!   whole - easing across a portal would drag the picture through the
//!   scenery between the two ends.
//! - *The turret* is not on the wire at all; the replica derives it, so
//!   it follows the hull this already predicts rather than lagging it.
//!
//! **Measured** (`report`): every correction sorted into the buckets of
//! `ERROR_BUCKETS_PX`, the largest, nudges and snaps, shots drawn and
//! refused, the inputs in flight. The dev server's `status.round` reads
//! it and the rig's tests assert on it.

use std::collections::VecDeque;

use crate::PHYSICS_FIXED_DT;
use crate::ai::Intent;
use crate::math::Vec2 as Position;
use crate::net::apply;
use crate::net::wire::{Snapshot, WeaponKind};
use crate::simulation::{Game, ProvisionalKind, ProvisionalShot};
use crate::tank::ActiveWeapon;
use crate::tuning::tuning;

/// How many ticks of input to keep. A replay starts at the last tick the
/// server acknowledged, so this has to cover the worst round trip worth
/// keeping: 120 ticks is two seconds at 60 Hz, well past the point where
/// a link is unplayable for other reasons.
pub const HISTORY_TICKS: usize = 120;

/// Prediction error below this is not a correction at all.
///
/// **Set by the wire, not by taste.** A position travels as quarter
/// pixels (`wire::quantise_pos`), so the server's answer arrives rounded
/// by up to an eighth of a pixel on each axis - about 0.18 as a
/// distance - before the replay has done anything at all, and stepping
/// from a rounded start moves that a little further. Anything under half
/// a pixel is therefore the wire's rounding rather than a disagreement,
/// and nudging the hull for it would be jitter dressed up as
/// reconciliation: a correction on every single snapshot, forever, on a
/// link with nothing wrong with it.
pub const IGNORE_PX: f32 = 0.5;

/// Error past this is not nudged but snapped. A hull is 64 px wide; being
/// most of one out means the sandbox and the server disagree about
/// something structural - a wall taken on the other side, a teleport - and
/// easing across it would drag the hull through the scenery.
pub const SNAP_PX: f32 = 48.0;

/// How long a nudge takes to die away. Short enough that a correction is
/// over before the next one lands, long enough that it reads as the hull
/// settling rather than jumping.
pub const NUDGE_SECONDS: f32 = 0.1;

/// How long a press nobody confirmed is left on screen, in seconds.
///
/// A round trip plus the interpolation delay, generously: the server's
/// `Fired` for a shot it accepted cannot take longer than that to be
/// handed over, so anything still waiting here was refused - the trigger
/// was pulled on a cooldown the client had not seen, or with ammo the
/// server had already spent. Those are quiet failures by design (§4.12):
/// the shot fades rather than announcing that the client guessed wrong.
pub const PROVISIONAL_SECONDS: f32 = 0.6;

/// The id band provisional shots are drawn under.
///
/// The server hands its projectiles a per-round counter and the wire
/// carries it as a `u16`, so the top of that range is free in any round
/// that does not fire sixty thousand shots. A provisional needs an id
/// only so the replica can hold it beside the server's own; it is never
/// sent anywhere.
pub const PROVISIONAL_ID_BASE: u32 = 0xF000;

/// The edges of the correction histogram, in pixels: the wire's own
/// rounding, `IGNORE_PX`, a couple of pixels, a wheel's width, and
/// `SNAP_PX`. The sixth bucket is everything past the last.
pub const ERROR_BUCKETS_PX: [f32; 5] = [0.25, IGNORE_PX, 2.0, 8.0, SNAP_PX];

/// One shot of a press, still to leave the muzzle.
#[derive(Clone, Copy, Debug)]
struct Pending {
    /// Seconds after the press it leaves.
    due: f32,
    /// Sideways from the centreline, the twin barrel's offset.
    lateral: f32,
    /// Degrees off the barrel: a bullet's spread. Hashed, since the
    /// server rolls it and the picture only needs variety.
    aim: f32,
}

/// A shot that has left, drawn until the server's takes its place.
#[derive(Clone, Copy, Debug)]
struct Live {
    shot: ProvisionalShot,
    /// Seconds after the press it left, so it is retired that long after
    /// the press is confirmed.
    due: f32,
    /// Its drawn path has crossed a drawn hull (`mark_crossings`): the
    /// moment the lie of §4.12 was visible, counted once.
    crossed: bool,
}

/// One pull of the trigger the client drew before the server answered.
#[derive(Clone, Debug)]
struct Press {
    kind: ProvisionalKind,
    /// Seconds since the press.
    age: f32,
    /// The `Fired` for this press has arrived - its ammo is now in the
    /// sandbox, so it is no longer owed against the gate.
    accounted: bool,
    /// The age at which the interpolator handed that `Fired` over, which
    /// is when the server's shot appeared: each shot of the press is
    /// retired its own delay after it.
    confirmed_at: Option<f32>,
    pending: Vec<Pending>,
    live: Vec<Live>,
}

impl Press {
    /// The ammo the server will spend on this press.
    fn cost(&self) -> i32 {
        (self.pending.len() + self.live.len()) as i32
    }
}

/// The predictor's counters (docs/online-coop-prd.md §4.12, "Measured").
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PredictionReport {
    /// Reconciliations whose error was under `IGNORE_PX`.
    pub ignored: u32,
    /// Corrections eased off as an offset.
    pub nudges: u32,
    /// Corrections taken whole.
    pub snaps: u32,
    /// Every reconciliation's error by `ERROR_BUCKETS_PX`: at most the
    /// first edge, at most the second, ..., past the last.
    pub error_buckets: [u32; 6],
    /// The largest error seen, in pixels.
    pub max_error_px: f32,
    /// Presses drawn ahead of the server.
    pub shots_drawn: u32,
    /// Presses the server never claimed.
    pub shots_refused: u32,
    /// Provisional shots on screen right now.
    pub shots_on_screen: usize,
    /// Provisional shots whose drawn path crossed a drawn enemy hull: the
    /// frames the lie of §4.12 was on screen. The round keeps the two
    /// that follow (decision 9's measurement).
    pub crossings: u32,
    /// Crossings the server answered with a `Hit` near that point within
    /// `net::round::CROSSING_WINDOW_SECONDS`.
    pub crossings_hit: u32,
    /// Crossings it never did: a shell drawn through a hull it did not
    /// touch on the server. The rate of these against `crossings` is
    /// what decides lag compensation.
    pub crossings_missed: u32,
    /// Inputs the server had not acknowledged at the last reconciliation:
    /// the replay's length, the lead plus the round trip in ticks.
    pub in_flight: usize,
    /// The local fire gate, seconds until it opens.
    pub cooldown: f32,
    /// Times the client added a packet to widen its lead (`net::round`).
    pub lead_up: u32,
    /// Times it skipped one to narrow it.
    pub lead_down: u32,
}

/// The local seat's own tank, run ahead of the server and pulled back
/// into line by it.
pub struct Predictor {
    /// A `Game` used for nothing but `predict_seat`, and the seat's
    /// weapon, ammo and muzzle.
    sandbox: Game,
    /// Which seat this window is playing.
    seat: usize,
    /// The tick the next input will be stamped with.
    tick: u32,
    /// The inputs since the last acknowledged tick, oldest first.
    history: VecDeque<(u32, Intent)>,
    /// Where the drawn hull sits relative to the predicted one, decaying
    /// to nothing. A correction moves the *prediction* at once - the
    /// physics has to be right or the next tick compounds the error -
    /// and leaves this behind so the picture does not jump.
    offset: Position,
    /// The fire bit of the last input stepped: the server's press edge
    /// is `fire && !held_last_tick`, on the same sequence of packets.
    trigger_held: bool,
    /// Whether presses are drawn at all (`online_predict_shots`, read by
    /// the round each frame since the knob is live).
    shots_enabled: bool,
    /// Presses waiting for the server, oldest first.
    presses: VecDeque<Press>,
    next_press: u32,
    /// Seconds until the local gate lets another press out. Counted
    /// down here rather than read off the sandbox, because
    /// `predict_seat` deliberately does not fire - it only drives.
    cooldown: f32,
    report: PredictionReport,
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
            trigger_held: false,
            shots_enabled: true,
            presses: VecDeque::new(),
            next_press: 0,
            cooldown: 0.0,
            report: PredictionReport::default(),
        }
    }

    /// Whether a press draws a shot. Off, the trigger still runs the
    /// local gate down so turning it on mid-round starts in step.
    pub fn set_shots_enabled(&mut self, on: bool) {
        self.shots_enabled = on;
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
    /// One counter, not two that could drift. `intent` is the packet as
    /// sent, fire hold included, so the press edge below is the one the
    /// server will see.
    ///
    /// The trigger is pulled *after* the hull is driven, as
    /// `drive_player` does it, so the shot leaves from the pose this tick
    /// produced.
    pub fn step_at(&mut self, tick: u32, intent: Intent) {
        self.sandbox.predict_seat(self.seat, intent, PHYSICS_FIXED_DT);
        self.history.push_back((tick, intent));
        while self.history.len() > HISTORY_TICKS {
            self.history.pop_front();
        }
        self.tick = tick.wrapping_add(1);
        let pressed = intent.fire && !self.trigger_held;
        self.trigger_held = intent.fire;
        self.pull_trigger(pressed, intent.fire);
    }

    /// The local gate, and a press through it.
    ///
    /// **The gate is the client's guess at the server's**, and a good one:
    /// the weapon, its ammo and the twin barrel are the sandbox's, which
    /// the last snapshot wrote, and the cooldown is the same knob the
    /// server counts down. What it cannot know is a cooldown set by a
    /// shot this client has not heard about yet; guessing wrong there is
    /// cheap and deliberate - the shot is drawn, no `Fired` ever comes
    /// back for it, and it expires quietly.
    fn pull_trigger(&mut self, pressed: bool, held: bool) {
        if !self.shots_enabled || self.cooldown > 0.0 {
            return;
        }
        let Some((weapon, ammo, lateral)) = self.sandbox.seat_arms(self.seat) else { return };
        let t = tuning();
        let (kind, cost, cooldown, pending) = match weapon {
            ActiveWeapon::Shell | ActiveWeapon::Plasma if pressed => {
                let kind = if weapon == ActiveWeapon::Shell { ProvisionalKind::Shell } else { ProvisionalKind::Plasma };
                let cost = if lateral > 0.0 { 2 } else { 1 };
                let mut pending = vec![Pending { due: 0.0, lateral: -lateral, aim: 0.0 }];
                if lateral > 0.0 {
                    pending.push(Pending { due: t.tank_twin_shot_delay_seconds, lateral, aim: 0.0 });
                }
                (kind, cost, t.player_fire_interval, pending)
            }
            ActiveWeapon::Minigun if held => {
                let left = ammo - self.owed(ProvisionalKind::Bullet);
                if left <= 0 {
                    return;
                }
                let burst = (t.minigun_burst_size.max(1) as i32).min(left);
                let spread = t.minigun_bullet_spread_deg;
                let press = self.next_press;
                let pending = (0..burst)
                    .map(|k| Pending {
                        due: k as f32 * t.minigun_bullet_delay_seconds,
                        lateral: 0.0,
                        aim: (hash01(press, k as u32) * 2.0 - 1.0) * spread,
                    })
                    .collect();
                (ProvisionalKind::Bullet, 1, t.minigun_burst_cooldown_seconds(), pending)
            }
            // The laser's beam is a hit test, the pod's volley a seeker's,
            // the flamethrower's cone the room's (§4.12); nothing here.
            _ => return,
        };
        if ammo - self.owed(kind) < cost {
            return;
        }
        self.cooldown = cooldown;
        let mut press = Press {
            kind,
            age: 0.0,
            accounted: false,
            confirmed_at: None,
            pending,
            live: Vec::new(),
        };
        self.next_press = self.next_press.wrapping_add(1);
        // The first shot leaves on the press itself, from the pose this
        // tick produced.
        self.launch_due(&mut press);
        self.presses.push_back(press);
        self.report.shots_drawn += 1;
    }

    /// Ammo the presses still unaccounted for will spend, by kind, so a
    /// press the sandbox does not yet know about still counts against
    /// the next.
    fn owed(&self, kind: ProvisionalKind) -> i32 {
        self.presses.iter().filter(|p| !p.accounted && p.kind == kind).map(Press::cost).sum()
    }

    /// Move every pending shot whose time has come out of the muzzle,
    /// from the sandbox's pose right now - the twin's second shell and a
    /// burst's later bullets leave from where the hull is *then*, which
    /// is how `tick_queued_shots` fires them.
    fn launch_due(&self, press: &mut Press) {
        let mut i = 0;
        while i < press.pending.len() {
            let p = press.pending[i];
            if p.due > press.age {
                i += 1;
                continue;
            }
            press.pending.remove(i);
            if let Some(shot) = self.sandbox.seat_shot(self.seat, press.kind, p.aim, p.lateral) {
                press.live.push(Live { shot, due: p.due, crossed: false });
            }
        }
    }

    /// The server's `Fired` for this seat has *arrived*: the oldest press
    /// not yet accounted for is the one it names, and its ammo is now in
    /// the snapshot the sandbox just took, so it no longer counts against
    /// the gate. Retirement waits for `confirm_shot`, when the shot is
    /// drawn.
    pub fn note_fired(&mut self) {
        if let Some(press) = self.presses.iter_mut().find(|p| !p.accounted) {
            press.accounted = true;
        }
    }

    /// The interpolator handed over the server's `Fired` for this seat:
    /// the oldest press still waiting was that one, and the server's shot
    /// is on screen from this frame, so the press's own shots retire from
    /// here on, each its own delay after.
    ///
    /// With no press waiting - prediction was off for the press, or it
    /// was pulled before the sandbox existed - the room's shot still sets
    /// the local gate, measured back from `acked` by `ticks_ago`, so the
    /// next press agrees with the server's cooldown.
    pub fn confirm_shot(&mut self, weapon: WeaponKind, ticks_ago: u32) {
        match self.presses.iter_mut().find(|p| p.confirmed_at.is_none()) {
            Some(press) => press.confirmed_at = Some(press.age),
            None => {
                let t = tuning();
                let interval = match weapon {
                    WeaponKind::Shell | WeaponKind::Plasma | WeaponKind::Laser => t.player_fire_interval,
                    WeaponKind::Minigun => t.minigun_burst_cooldown_seconds(),
                    WeaponKind::Missiles => t.missile_volley_cooldown_seconds(),
                    WeaponKind::Flamethrower => 0.0,
                };
                let left = interval - ticks_ago as f32 * PHYSICS_FIXED_DT;
                self.cooldown = self.cooldown.max(left);
            }
        }
    }

    /// One rendered frame of the shots: the gate counts down, every press
    /// ages, its due shots leave, the live ones fly on by dead reckoning,
    /// confirmed ones retire as the server's appear, and a press nobody
    /// claimed within `PROVISIONAL_SECONDS` goes quietly, counted.
    pub fn advance_shots(&mut self, dt: f32) {
        self.cooldown = (self.cooldown - dt).max(0.0);
        let mut presses = std::mem::take(&mut self.presses);
        for press in &mut presses {
            press.age += dt;
            self.launch_due(press);
            for live in &mut press.live {
                let s = &mut live.shot;
                s.prev_position = s.position;
                s.position = Position::new(s.position.x + s.velocity.x * dt, s.position.y + s.velocity.y * dt);
            }
            if let Some(at) = press.confirmed_at {
                press.live.retain(|l| press.age < at + l.due);
            }
        }
        presses.retain(|p| {
            if p.confirmed_at.is_some() {
                return !(p.pending.is_empty() && p.live.is_empty());
            }
            if p.age > PROVISIONAL_SECONDS {
                self.report.shots_refused += 1;
                return false;
            }
            true
        });
        self.presses = presses;
    }

    /// The shots to draw beside the server's, with the ids they are held
    /// under in the replica.
    pub fn shots(&self) -> impl Iterator<Item = (u32, ProvisionalShot)> + '_ {
        self.presses
            .iter()
            .flat_map(|p| p.live.iter().map(|l| l.shot))
            .enumerate()
            .map(|(i, shot)| (PROVISIONAL_ID_BASE + i as u32, shot))
    }

    /// Mark every live shot whose path this frame crosses something,
    /// once each, and hand back where it did. `crosses` is the caller's
    /// test of a segment against the picture - the predictor knows the
    /// sandbox, not the drawn hulls - answering with the point to record.
    pub fn mark_crossings(&mut self, mut crosses: impl FnMut(Position, Position) -> Option<Position>) -> Vec<Position> {
        let mut points = Vec::new();
        for press in &mut self.presses {
            for live in &mut press.live {
                if live.crossed {
                    continue;
                }
                if let Some(at) = crosses(live.shot.prev_position, live.shot.position) {
                    live.crossed = true;
                    self.report.crossings += 1;
                    points.push(at);
                }
            }
        }
        points
    }

    /// How many shots are on screen that the server has not yet drawn.
    pub fn provisional_count(&self) -> usize {
        self.presses.iter().map(|p| p.live.len()).sum()
    }

    /// Presses drawn and not yet retired, for a test.
    pub fn presses_waiting(&self) -> usize {
        self.presses.len()
    }

    /// Pull the sandbox back to what the server said, then replay
    /// everything it had not seen yet.
    ///
    /// **The whole snapshot goes in, not just this seat's hull.** That is
    /// the difference between a sandbox that drifts and one that cannot:
    /// a prediction is only as good as the world it steps against, and
    /// anything the server changed that the sandbox did not hear about
    /// becomes a standing disagreement rather than a correction that
    /// settles. Every hull but this one sitting where it was at the
    /// welcome - which is what happens if only this seat is synced - is
    /// the visible version: the predicted tank drives through tanks that
    /// have moved and stops against tanks that are no longer there.
    ///
    /// So the sandbox takes `net::apply::snapshot`, the same call the
    /// replica takes, and is a projection of the server's world rather
    /// than a world of its own: other hulls, destroyed walls, a speed
    /// boost, a spent pickup, the seat's weapon and ammo, all of it,
    /// through one path that is already the tested one. What is left to
    /// predict on top is this seat's unacknowledged input.
    ///
    /// `acked` is the last input tick the server applied for this seat,
    /// and the snapshot is where that left the world. Every input after
    /// it is still in flight, so it is replayed on top - the same count
    /// of steps, the same `dt`, the same statics, which is why the answer
    /// lands rather than drifts.
    pub fn reconcile(&mut self, snapshot: &Snapshot, acked: u32) {
        let before = self.sandbox.seat_motion(self.seat).map(|(p, _, _)| p);
        apply::snapshot(&mut self.sandbox, snapshot);
        // Anything the server has already accounted for is history.
        while self.history.front().is_some_and(|(t, _)| *t <= acked) {
            self.history.pop_front();
        }
        self.report.in_flight = self.history.len();
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
        let bucket = ERROR_BUCKETS_PX.iter().position(|&edge| distance <= edge).unwrap_or(ERROR_BUCKETS_PX.len());
        self.report.error_buckets[bucket] += 1;
        self.report.max_error_px = self.report.max_error_px.max(distance);
        if distance <= IGNORE_PX {
            self.report.ignored += 1;
            return;
        }
        if distance >= SNAP_PX {
            self.offset = Position::new(0.0, 0.0);
            self.report.snaps += 1;
            return;
        }
        self.offset = Position::new(self.offset.x + error.x, self.offset.y + error.y);
        self.report.nudges += 1;
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

    /// The correction still being eased off, for a test to assert on.
    pub fn offset(&self) -> Position {
        self.offset
    }

    /// The counters, with the lead's own adjustments filled in by the
    /// round that keeps them; the round fills the crossing answers too.
    pub fn report(&self, lead_up: u32, lead_down: u32) -> PredictionReport {
        PredictionReport {
            shots_on_screen: self.provisional_count(),
            cooldown: self.cooldown,
            lead_up,
            lead_down,
            ..self.report
        }
    }
}

/// A unit hash of two counters, for a bullet's spread: the server rolls
/// one per bullet and the picture only needs the variety.
fn hash01(a: u32, b: u32) -> f32 {
    let h = a.wrapping_mul(2_654_435_761).wrapping_add(b.wrapping_mul(0x9E37_79B9)) >> 8;
    (h % 10_000) as f32 / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::MapFile;
    use crate::net::encode;
    use crate::simulation::Input;
    use crate::simulation::debug::TankPatch;
    use crate::tank::Dir;

    /// What the server would send about `game` right now, with `acked`
    /// as the input tick it has applied for seat 0. Reconciliation goes
    /// through the real wire in these tests rather than a hand-picked
    /// pose, which is the point: the sandbox takes the whole world.
    fn wire(game: &mut Game, acked: u32) -> crate::net::wire::Snapshot {
        let mut acks = [0u32; crate::net::MAX_SEATS];
        acks[0] = acked;
        encode::snapshot(game, acks)
    }

    const MAP: &str = include_str!("../../maps/default.toml");

    /// A round on the shipped map with no enemies, which is all a
    /// locomotion test needs - and no enemies means nothing else in the
    /// world for the solver to meet, so a difference is the hull's.
    fn round() -> Game {
        round_with(3, 0)
    }

    /// The same round with enemies in it, for the checks that are about
    /// what the sandbox knows of the rest of the world.
    fn round_with_enemies() -> Game {
        round_with(3, 3)
    }

    /// A round on chassis row `row` with `enemies` enemies.
    fn round_with(row: i32, enemies: usize) -> Game {
        let mut game = Game::default();
        game.seed_override = Some(0xB0B5);
        game.enemy_count_override = Some(enemies);
        game.player_row_override = Some(row);
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

    /// One tick with the trigger down, and one with it up.
    fn press() -> Intent {
        Intent { fire: true, ..Intent::default() }
    }

    fn release() -> Intent {
        Intent::default()
    }

    /// A chassis row with one barrel and one with two.
    fn single_barrel_row() -> i32 {
        let lateral = tuning().tank_barrel_lateral_offset;
        lateral.iter().position(|&l| l == 0.0).expect("a single-barrel chassis") as i32
    }

    fn twin_barrel_row() -> i32 {
        let lateral = tuning().tank_barrel_lateral_offset;
        lateral.iter().position(|&l| l > 0.0).expect("a twin-barrel chassis") as i32
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

        predictor.reconcile(&wire(&mut authority, 59), 59);

        let after = predictor.motion().expect("a hull").0;
        // Within the wire's own resolution, which is as exactly as a
        // replay can land: the server's position arrived quantised.
        let drift = ((after.x - predicted_before.x).powi(2) + (after.y - predicted_before.y).powi(2)).sqrt();
        assert!(drift < IGNORE_PX, "the replay landed {drift} px out, past the wire's rounding");
        // And so it is not treated as a correction at all.
        assert_eq!((predictor.offset().x, predictor.offset().y), (0.0, 0.0));
        let report = predictor.report(0, 0);
        assert_eq!((report.nudges, report.snaps, report.ignored), (0, 0, 1));
        assert_eq!(report.in_flight, 10, "ten inputs were still in flight");
        assert!(report.error_buckets[0] + report.error_buckets[1] == 1, "counted under the wire's rounding: {report:?}");
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
        authority.place_seat(0, nudged, rot, vel);
        let drawn_before = predictor.drawn_position().expect("a hull");
        predictor.reconcile(&wire(&mut authority, 39), 39);

        let report = predictor.report(0, 0);
        assert_eq!(report.nudges, 1, "a real difference should count");
        assert_eq!(report.snaps, 0);
        assert_eq!(report.error_buckets[3], 1, "six pixels sits in the two-to-eight bucket: {report:?}");
        assert!((report.max_error_px - 6.0).abs() < IGNORE_PX, "{report:?}");
        // The prediction took the server's answer, to the quarter pixel
        // the wire carries it in.
        let (now, _, _) = predictor.motion().expect("a hull");
        assert!((now.x - nudged.x).abs() < IGNORE_PX, "the prediction should be the server's, got {now:?}");
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

    /// **The bug this reconciliation shape exists to make impossible.**
    ///
    /// A sandbox that syncs only its own hull has every *other* tank
    /// frozen where the welcome left it, so the prediction drives
    /// through tanks that have since moved and stops against tanks that
    /// are no longer there - reported from play as the hull ignoring
    /// bounds. Taking the whole snapshot is what fixes it, and this is
    /// the property that says so: after a reconciliation, every hull in
    /// the sandbox stands where the server has it.
    #[test]
    fn the_sandbox_learns_where_every_other_tank_is() {
        let mut authority = round_with_enemies();
        let mut predictor = Predictor::new(round_with_enemies(), 0, 0);
        // The room's enemies move; the sandbox has heard nothing yet.
        for _ in 0..90 {
            authority.update(Input::default(), PHYSICS_FIXED_DT, 1088.0, 544.0);
        }
        let room: Vec<(usize, i32, i32)> =
            authority.drawable_state().tanks.iter().map(|t| (t.slot, t.x, t.y)).collect();
        assert!(room.len() > 1, "the fixture should have enemies to be wrong about");

        predictor.reconcile(&wire(&mut authority, 1), 1);

        let sandbox: Vec<(usize, i32, i32)> =
            predictor.sandbox.drawable_state().tanks.iter().map(|t| (t.slot, t.x, t.y)).collect();
        assert_eq!(sandbox, room, "the sandbox and the room disagree about where the tanks are");
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
        // The server's hull picks up a SpeedUp. The flag rides in on the
        // snapshot like everything else; nothing here names it.
        authority.give_seat_boost(0);
        predictor.reconcile(&wire(&mut authority, 0), 0);

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

    /// The point of a provisional shot: it is on screen on the tick of
    /// the press, and it leaves from where the hull is *now*.
    #[test]
    fn a_shot_is_drawn_from_the_predicted_muzzle_at_once() {
        let mut predictor = Predictor::new(round_with(single_barrel_row(), 0), 0, 0);
        for tick in 0..30u32 {
            predictor.step(script(tick));
        }
        let hull = predictor.motion().expect("a hull").0;
        predictor.step(press());
        assert_eq!(predictor.provisional_count(), 1, "the shot should be on screen at once");
        assert_eq!(predictor.report(0, 0).shots_drawn, 1);

        let (id, shell) = predictor.shots().next().expect("a shell");
        assert_eq!(id, PROVISIONAL_ID_BASE);
        assert_eq!(shell.kind, ProvisionalKind::Shell);
        let gap = ((shell.position.x - hull.x).powi(2) + (shell.position.y - hull.y).powi(2)).sqrt();
        assert!(gap > 1.0, "the shell should leave the muzzle, not the hull's centre");
        assert!(gap < 100.0, "but it is a muzzle, not a mortar: {gap}");
        // And it flies.
        let before = shell.position;
        predictor.advance_shots(PHYSICS_FIXED_DT);
        let after = predictor.shots().next().expect("a shell").1.position;
        assert_ne!((before.x, before.y), (after.x, after.y), "the shell did not move");
    }

    /// Shells fire on the press, not while held (`drive_player`), and the
    /// local gate holds even a second press to one shot per
    /// `player_fire_interval` - otherwise holding fire paints the screen
    /// with shots the server will refuse.
    #[test]
    fn a_shell_fires_on_the_edge_and_the_gate_rations_a_second_press() {
        let mut predictor = Predictor::new(round_with(single_barrel_row(), 0), 0, 0);
        for _ in 0..10 {
            predictor.step(press());
        }
        assert_eq!(predictor.provisional_count(), 1, "a held trigger drew more than one shell");
        // Released and pressed again inside the interval: nothing.
        predictor.step(release());
        predictor.step(press());
        assert_eq!(predictor.provisional_count(), 1, "the gate let a second press through early");
        // Past the interval, another is allowed - on a fresh edge.
        predictor.advance_shots(tuning().player_fire_interval + 0.001);
        predictor.step(press());
        assert_eq!(predictor.provisional_count(), 1, "a held trigger never re-arms a shell");
        predictor.step(release());
        predictor.step(press());
        assert_eq!(predictor.provisional_count(), 2);
    }

    /// **The bug this gate exists for.** A seat holding the minigun used
    /// to draw a *shell* on every press: the predictor built one without
    /// asking what the seat was armed with. Now the sandbox's weapon
    /// decides, and the minigun draws its burst - bullets, one per
    /// `minigun_bullet_delay_seconds`, while the key is held.
    #[test]
    fn the_weapon_the_sandbox_holds_decides_what_is_drawn() {
        let mut predictor = Predictor::new(round(), 0, 0);
        let patch = TankPatch { minigun_ammo: Some(40), ..Default::default() };
        predictor.sandbox.debug_set_tank(0, &patch).expect("the seat's tank");
        predictor.step(press());
        let kinds: Vec<ProvisionalKind> = predictor.shots().map(|(_, s)| s.kind).collect();
        assert_eq!(kinds, [ProvisionalKind::Bullet], "the first bullet of the burst, and no shell");
        // The rest of the burst leaves on its own clock while held.
        let burst = tuning().minigun_burst_size.max(1) as usize;
        for _ in 0..burst {
            predictor.advance_shots(tuning().minigun_bullet_delay_seconds + 0.001);
            predictor.step(press());
        }
        assert_eq!(predictor.provisional_count(), burst, "the whole burst is on screen");
        assert!(predictor.shots().all(|(_, s)| s.kind == ProvisionalKind::Bullet));
        // Its bullets fan a little, the way the server's spread does.
        let headings: Vec<f32> = predictor.shots().map(|(_, s)| s.rotation).collect();
        assert!(headings.windows(2).any(|w| w[0] != w[1]), "every bullet flew dead straight: {headings:?}");
        // Held past the burst cooldown, a second burst starts without a
        // fresh press - full-auto.
        predictor.advance_shots(tuning().minigun_burst_cooldown_seconds() + 0.001);
        predictor.step(press());
        assert_eq!(predictor.report(0, 0).shots_drawn, 2, "a held minigun keeps firing");
    }

    /// A twin-barrel chassis fires its left barrel on the press and its
    /// right `tank_twin_shot_delay_seconds` later, from where the hull is
    /// then, exactly as `tick_queued_shots` does it.
    #[test]
    fn a_twin_barrel_draws_its_second_shell_on_the_twins_delay() {
        let mut predictor = Predictor::new(round_with(twin_barrel_row(), 0), 0, 0);
        predictor.step(press());
        assert_eq!(predictor.provisional_count(), 1, "the left barrel only, on the press");
        let first = predictor.shots().next().expect("a shell").1;
        predictor.advance_shots(tuning().tank_twin_shot_delay_seconds / 2.0);
        assert_eq!(predictor.provisional_count(), 1, "not yet");
        predictor.advance_shots(tuning().tank_twin_shot_delay_seconds / 2.0 + 0.001);
        assert_eq!(predictor.provisional_count(), 2, "the right barrel, on the delay");
        let second = predictor.shots().nth(1).expect("a second shell").1;
        // Side by side across the hull, not one behind the other.
        assert!((first.rotation - second.rotation).abs() < 0.001, "the barrels fire parallel");
        let across = (first.position.x - second.position.x).abs() + (first.position.y - second.position.y).abs();
        assert!(across > 1.0, "the two shells left the same barrel");
        // One press, two shells: the gate charged two rounds for it.
        assert_eq!(predictor.report(0, 0).shots_drawn, 1);
    }

    /// The server's ammo is the sandbox's, and a press the server would
    /// refuse for want of it draws nothing - including one the sandbox
    /// has not been told about yet, whose cost is still owed.
    #[test]
    fn a_press_the_server_would_refuse_for_ammo_draws_nothing() {
        let mut predictor = Predictor::new(round_with(single_barrel_row(), 0), 0, 0);
        let patch = TankPatch { shells_ammo: Some(1), ..Default::default() };
        predictor.sandbox.debug_set_tank(0, &patch).expect("the seat's tank");
        predictor.step(press());
        assert_eq!(predictor.provisional_count(), 1, "the one shell there is");
        predictor.step(release());
        predictor.advance_shots(tuning().player_fire_interval + 0.001);
        predictor.step(press());
        assert_eq!(predictor.provisional_count(), 1, "the second press is owed the shell the first spent");
        assert_eq!(predictor.report(0, 0).shots_drawn, 1);
    }

    /// The server's word retires a press oldest first, on the frame the
    /// interpolator hands the `Fired` over - and a press's later shots
    /// their own delay after, as the server's appear.
    #[test]
    fn the_servers_fired_retires_the_oldest_press_as_its_shots_appear() {
        let mut predictor = Predictor::new(round_with(twin_barrel_row(), 0), 0, 0);
        predictor.step(press());
        predictor.step(release());
        predictor.advance_shots(tuning().player_fire_interval + 0.001);
        predictor.step(press());
        assert_eq!(predictor.presses_waiting(), 2);
        let twin = tuning().tank_twin_shot_delay_seconds;
        assert_eq!(predictor.provisional_count(), 3, "both of the first press and the first of the second");
        predictor.confirm_shot(WeaponKind::Shell, 0);
        // Confirmed, but its shots stay until the server's take over:
        // the first at once, the second after the twin's delay.
        predictor.advance_shots(0.0);
        assert_eq!(predictor.provisional_count(), 2, "the first press's first shell went at the handover");
        predictor.advance_shots(twin + 0.001);
        assert_eq!(predictor.presses_waiting(), 1, "the first press is spent");
        assert_eq!(predictor.provisional_count(), 2, "the second press has both shells out by now");
        predictor.confirm_shot(WeaponKind::Shell, 0);
        predictor.advance_shots(twin + 0.001);
        assert_eq!(predictor.presses_waiting(), 0);
        assert_eq!(predictor.report(0, 0).shots_refused, 0, "a confirmed press is not a refused one");
    }

    /// A press the server never mentions was refused - the trigger was
    /// pulled on a cooldown or an ammo count this client had not caught
    /// up with - and it goes quietly rather than hanging on screen.
    #[test]
    fn a_press_the_server_never_confirms_expires_quietly() {
        let mut predictor = Predictor::new(round_with(single_barrel_row(), 0), 0, 0);
        predictor.step(press());
        predictor.advance_shots(PROVISIONAL_SECONDS - 0.01);
        assert_eq!(predictor.provisional_count(), 1, "not yet: it is still within a round trip");
        predictor.advance_shots(0.02);
        assert_eq!(predictor.provisional_count(), 0);
        assert_eq!(predictor.report(0, 0).shots_refused, 1, "and it is counted, so a loose gate is visible");
    }

    /// A `Fired` with no press waiting is a shot this client did not
    /// predict; the room's shot still sets the local gate, measured back
    /// from how long ago the server took the input.
    #[test]
    fn a_fired_nobody_predicted_seeds_the_local_gate() {
        let mut predictor = Predictor::new(round_with(single_barrel_row(), 0), 0, 0);
        predictor.set_shots_enabled(false);
        predictor.step(press());
        assert_eq!(predictor.provisional_count(), 0, "off, nothing is drawn");
        // The server fired that press five ticks ago.
        predictor.confirm_shot(WeaponKind::Shell, 5);
        let left = predictor.report(0, 0).cooldown;
        let expected = tuning().player_fire_interval - 5.0 * PHYSICS_FIXED_DT;
        assert!((left - expected).abs() < 0.001, "the gate reads {left}, the server's is at {expected}");
        // Turned on inside that gate, a press draws nothing.
        predictor.set_shots_enabled(true);
        predictor.step(release());
        predictor.step(press());
        assert_eq!(predictor.provisional_count(), 0, "the seeded gate held the press");
    }

    /// A shot's crossing of a hull is the caller's test and the
    /// predictor's memory: asked every frame, it marks each shot once.
    #[test]
    fn a_crossing_is_marked_once_per_shot() {
        let mut predictor = Predictor::new(round_with(single_barrel_row(), 0), 0, 0);
        predictor.step(press());
        let none = predictor.mark_crossings(|_, _| None);
        assert!(none.is_empty());
        let once = predictor.mark_crossings(|_, b| Some(b));
        assert_eq!(once.len(), 1, "the shot crossed");
        let again = predictor.mark_crossings(|_, b| Some(b));
        assert!(again.is_empty(), "a shot crosses once, however often it is asked");
        assert_eq!(predictor.report(0, 0).crossings, 1);
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
        let mut authority = authority;
        authority.place_seat(0, far, rot, vel);
        predictor.reconcile(&wire(&mut authority, 29), 29);
        let report = predictor.report(0, 0);
        assert_eq!((report.nudges, report.snaps), (0, 1), "a teleport should snap, not ease");
        assert_eq!(report.error_buckets[5], 1, "and lands in the last bucket: {report:?}");
        assert_eq!((predictor.offset().x, predictor.offset().y), (0.0, 0.0));
        let now = predictor.motion().expect("a hull").0;
        assert!((now.x - far.x).abs() < 1.0, "the prediction should be where the portal put it");
        assert!((now.y - far.y).abs() < 1.0);
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
        authority.place_seat(0, Position::new(pos.x + SNAP_PX + 1.0, pos.y), rot, vel);
        predictor.reconcile(&wire(&mut authority, 19), 19);
        let report = predictor.report(0, 0);
        assert_eq!((report.nudges, report.snaps), (0, 1));
        assert_eq!((predictor.offset().x, predictor.offset().y), (0.0, 0.0), "a snap leaves nothing to ease");
    }
}
