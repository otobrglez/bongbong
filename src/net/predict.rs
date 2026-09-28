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
//! - *Firing*, for a predicted hull. `weapons::apply_recoil` pushes the
//!   shooter back along the shot's axis, and the sandbox never fires - it
//!   only drives. So every shot ends one nudge, bounded by
//!   `shell_recoil_max_speed` over one snapshot interval, and settled by
//!   the next reconciliation. An owned hull is never reconciled, so its
//!   sandbox kicks it at each launch (`Game::seat_recoil`), drawn or not,
//!   and the room leaves that kick to it.
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

/// How far past the point a provisional shot met something the room's
/// copy has to fly, still in flight, before the room is taken to have
/// missed: a hull's half-width, so a hit on the far face is not a miss.
pub const MISS_MARGIN_PX: f32 = 24.0;

/// How long a room shot of this seat that no press has claimed is kept
/// off the picture: long enough for the press whose `Fired` rides in the
/// same snapshot to claim it, short enough that a shot this client never
/// drew (prediction off, a gate stricter than the room's) shows promptly.
pub const UNPAIRED_HOLD_SECONDS: f32 = 0.1;

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
    /// Sandbox ticks after the press it leaves: the tick grid the room's
    /// `tick_queued_shots` fires on.
    due_ticks: u32,
    /// Sideways from the centreline, the twin barrel's offset.
    lateral: f32,
    /// Degrees off the barrel: a bullet's spread. Hashed, since the
    /// server rolls it and the picture only needs variety.
    aim: f32,
}

/// A shot that has left, drawn for its whole life on this client's
/// timeline (docs/online-coop-prd.md §4.16).
#[derive(Clone, Copy, Debug)]
struct Live {
    shot: ProvisionalShot,
    /// The room's copy of this shot, once paired (`observe_server_shots`).
    server: Option<u16>,
    /// Where it met something in the drawn world, and whether that was a
    /// tank: its impact was drawn there at once.
    local_hit: Option<(Position, bool)>,
    /// The room's copy flew on past `local_hit`: the room disagreed, and
    /// its shot is drawn instead from here on.
    show_server: bool,
    /// Its local tank hit has been scored against the room's word
    /// (`crossings_hit`/`crossings_missed`).
    scored: bool,
}

/// One pull of the trigger the client drew before the server answered.
#[derive(Clone, Debug)]
struct Press {
    kind: ProvisionalKind,
    /// The input tick the press travelled on: `Fired::input_tick` names it
    /// back.
    tick: u32,
    /// Sandbox ticks since the press: the grid its later shots leave on.
    ticks: u32,
    /// Seconds since the press, for the refusal timeout.
    age: f32,
    /// The room's `Fired` for it has arrived - its ammo is in the sandbox
    /// from now on, so it is no longer owed against the gate.
    accounted: bool,
    /// The room's `Fired` has been handed over: its shots are real.
    confirmed: bool,
    /// Its shots are drawn (`online_predict_shots` when it was pulled).
    /// An undrawn press is an owned hull's: it runs the gate and kicks the
    /// hull at each launch, and puts nothing on screen.
    drawn: bool,
    pending: Vec<Pending>,
    live: Vec<Live>,
    /// Shots of an undrawn press that have left the muzzle: owed against
    /// the gate as a drawn press's live shots are.
    undrawn: usize,
}

impl Press {
    /// The ammo the server will spend on this press.
    fn cost(&self) -> i32 {
        (self.pending.len() + self.live.len() + self.undrawn) as i32
    }
}

/// A laser beam the client drew on its press: where it left the muzzle,
/// which way, with which beam. The round finds where it stops in the
/// drawn world (`Game::present_world`) and draws it.
#[derive(Clone, Copy, Debug)]
pub struct BeamPress {
    pub start: Position,
    pub dir: crate::math::Vec2,
    pub variant: crate::laser::LaserVariant,
}

/// The room's copy of one of this seat's shots, as the frame drew it.
#[derive(Clone, Copy, Debug)]
pub struct ServerShot {
    pub id: u16,
    pub kind: ProvisionalKind,
    pub position: Position,
    pub velocity: crate::math::Vec2,
    pub flying: bool,
    /// In its impact frames: the room's copy hit something.
    pub impact: bool,
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
    /// Provisional shots this client drew hitting a tank in its own
    /// picture (decision 9's measurement, §4.16).
    pub crossings: u32,
    /// Of those, the ones whose room copy hit too.
    pub crossings_hit: u32,
    /// And the ones whose room copy flew on: the room judged the shot
    /// against somewhere else. With lag compensation this should be rare;
    /// its rate is what says whether the rewind is right.
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
    /// Laser presses whose `Fired` has not arrived: the charges owed.
    beams_owed: VecDeque<u32>,
    /// Beams drawn this tick, for the round to finish and draw.
    beams: Vec<BeamPress>,
    /// Where this frame's provisional shots met something, for the round's
    /// impact flashes.
    impacts: Vec<Position>,
    /// The room's shots of this seat first seen unpaired, and when (local
    /// seconds of `clock`): held back from the picture for a moment in
    /// case their press is about to claim them, drawn if nothing does.
    unpaired: std::collections::BTreeMap<u16, f32>,
    /// Seconds of shot-time this predictor has run, for `unpaired`.
    clock: f32,
    /// How long a press may wait for its `Fired` before it is taken as
    /// refused: `PROVISIONAL_SECONDS` at least, longer on a link whose
    /// round trip and picture delay take longer (`set_refusal_after`).
    refusal_after: f32,
    /// Seconds until the local gate lets another press out. Counted
    /// down here rather than read off the sandbox, because
    /// `predict_seat` deliberately does not fire - it only drives.
    cooldown: f32,
    /// The client owns the hull (docs/online-coop-prd.md §4.14): the
    /// sandbox's seat is the truth, a snapshot never moves it, and
    /// nothing is replayed - the room follows this hull, not the other
    /// way round.
    owned: bool,
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
            beams_owed: VecDeque::new(),
            beams: Vec::new(),
            impacts: Vec::new(),
            unpaired: std::collections::BTreeMap::new(),
            clock: 0.0,
            refusal_after: PROVISIONAL_SECONDS,
            cooldown: 0.0,
            owned: false,
            report: PredictionReport::default(),
        }
    }

    /// Whether this client owns its hull (`online_client_hull`, stage 3).
    pub fn set_owned(&mut self, owned: bool) {
        self.owned = owned;
    }

    /// The room moved the hull itself - `Placed`, a portal, a gate - and
    /// the owned hull snaps to where it says, standing still.
    pub fn place_own(&mut self, position: Position, rotation: f32) {
        self.sandbox.place_seat(self.seat, position, rotation, Position::new(0.0, 0.0));
        self.offset = Position::new(0.0, 0.0);
        self.history.clear();
    }

    /// Where the owned hull is, to put on the wire.
    pub fn pose(&self) -> Option<crate::simulation::SeatPose> {
        self.sandbox.seat_pose(self.seat)
    }

    /// Whether a press draws a shot. Off, the local gate still counts
    /// down so turning it on mid-round starts in step, and an owned hull's
    /// presses still run it and kick the hull at each launch - the room
    /// leaves that kick to the client whether the shot is drawn or not.
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
        // The room's order: timers count down, the hull drives, queued
        // shots leave, then the trigger - on the tick grid, so a skipped
        // or doubled frame cannot skew the gate against the room's.
        self.cooldown = (self.cooldown - PHYSICS_FIXED_DT).max(0.0);
        self.sandbox.predict_seat(self.seat, intent, PHYSICS_FIXED_DT);
        self.history.push_back((tick, intent));
        while self.history.len() > HISTORY_TICKS {
            self.history.pop_front();
        }
        self.tick = tick.wrapping_add(1);
        let mut presses = std::mem::take(&mut self.presses);
        for press in &mut presses {
            press.ticks += 1;
            self.launch_due(press);
        }
        self.presses = presses;
        let pressed = intent.fire && !self.trigger_held;
        self.trigger_held = intent.fire;
        self.pull_trigger(tick, pressed, intent.fire);
    }

    /// Where to draw the hull: its newest tick, as `app.rs` draws a local
    /// round's newest step. Drawing it between its last two ticks would
    /// smooth a frame that ran none or two of them, at the price of
    /// showing every input up to a tick late - a frame the local game
    /// never pays - so the owned hull is drawn exactly as the local one
    /// is, correction offset included.
    pub fn drawn_pose(&self) -> Option<(Position, f32)> {
        let (p, r, _) = self.motion()?;
        Some((Position::new(p.x + self.offset.x, p.y + self.offset.y), r))
    }

    /// A shove the room put on this hull (`Shoved`): the velocity change,
    /// applied to the sandbox's body so the next pose carries it.
    pub fn shove(&mut self, dv: crate::math::Vec2) {
        if let Some((p, r, v)) = self.sandbox.seat_motion(self.seat) {
            self.sandbox.place_seat(self.seat, p, r, Position::new(v.x + dv.x, v.y + dv.y));
        }
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
    fn pull_trigger(&mut self, tick: u32, pressed: bool, held: bool) {
        // An owned hull presses with the shots off too, for its recoil.
        let drawn = self.shots_enabled;
        if !(drawn || self.owned) || self.cooldown > 0.0 {
            return;
        }
        let Some((weapon, ammo, lateral)) = self.sandbox.seat_arms(self.seat) else { return };
        let t = tuning();
        let grid = |seconds: f32| (seconds / PHYSICS_FIXED_DT).ceil().max(1.0) as u32;
        let (kind, cost, cooldown, pending) = match weapon {
            ActiveWeapon::Shell | ActiveWeapon::Plasma if pressed => {
                let kind = if weapon == ActiveWeapon::Shell { ProvisionalKind::Shell } else { ProvisionalKind::Plasma };
                let cost = if lateral > 0.0 { 2 } else { 1 };
                let mut pending = vec![Pending { due_ticks: 0, lateral: -lateral, aim: 0.0 }];
                if lateral > 0.0 {
                    pending.push(Pending { due_ticks: grid(t.tank_twin_shot_delay_seconds), lateral, aim: 0.0 });
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
                let every = grid(t.minigun_bullet_delay_seconds);
                let pending = (0..burst)
                    .map(|k| Pending {
                        due_ticks: k as u32 * every,
                        lateral: 0.0,
                        aim: (hash01(press, k as u32) * 2.0 - 1.0) * spread,
                    })
                    .collect();
                (ProvisionalKind::Bullet, 1, t.minigun_burst_cooldown_seconds(), pending)
            }
            // The laser is full-auto while held, a charge a beam: drawn on
            // the press from the predicted muzzle, stopped by the round at
            // the first thing it meets in the drawn world.
            ActiveWeapon::Laser if held => {
                if ammo - self.beams_owed.len() as i32 <= 0 {
                    return;
                }
                if let Some((start, dir, variant)) = self.sandbox.seat_beam(self.seat) {
                    self.cooldown = t.player_fire_interval;
                    self.beams_owed.push_back(tick);
                    if drawn {
                        self.beams.push(BeamPress { start, dir, variant });
                        self.report.shots_drawn += 1;
                    }
                }
                return;
            }
            // The pod's volley is a seeker's and the flamethrower's cone
            // the room's (§4.16); nothing here.
            _ => return,
        };
        if ammo - self.owed(kind) < cost {
            return;
        }
        self.cooldown = cooldown;
        let mut press = Press {
            kind,
            tick,
            ticks: 0,
            age: 0.0,
            accounted: false,
            confirmed: false,
            drawn,
            pending,
            live: Vec::new(),
            undrawn: 0,
        };
        self.next_press = self.next_press.wrapping_add(1);
        // The first shot leaves on the press itself, from the pose this
        // tick produced.
        self.launch_due(&mut press);
        self.presses.push_back(press);
        if drawn {
            self.report.shots_drawn += 1;
        }
    }

    /// Ammo the presses still unaccounted for will spend, by kind, so a
    /// press the sandbox does not yet know about still counts against
    /// the next.
    fn owed(&self, kind: ProvisionalKind) -> i32 {
        self.presses.iter().filter(|p| !p.accounted && p.kind == kind).map(Press::cost).sum()
    }

    /// Move every pending shot whose tick has come out of the muzzle,
    /// from the sandbox's pose right now - the twin's second shell and a
    /// burst's later bullets leave from where the hull is *then*, which
    /// is how `tick_queued_shots` fires them. An owned hull is kicked back
    /// by every one, drawn or not.
    fn launch_due(&mut self, press: &mut Press) {
        let mut i = 0;
        while i < press.pending.len() {
            let p = press.pending[i];
            if p.due_ticks > press.ticks {
                i += 1;
                continue;
            }
            press.pending.remove(i);
            if let Some(shot) = self.sandbox.seat_shot(self.seat, press.kind, p.aim, p.lateral) {
                // The kick lands on the launch, as the room's does in a
                // local round. The room sends no `Shoved` for it
                // (`weapons::apply_recoil`), so this is the owned hull's
                // one kick per shot.
                if self.owned {
                    self.sandbox.seat_recoil(self.seat, press.kind, shot.velocity);
                }
                if press.drawn {
                    press.live.push(Live { shot, server: None, local_hit: None, show_server: false, scored: false });
                } else {
                    press.undrawn += 1;
                }
            }
        }
    }

    /// The room's `Fired` for this seat, fired on the input tick
    /// `input_tick`, has *arrived*: every press up to that tick has its
    /// ammo in the snapshot the sandbox just took, so none of them is owed
    /// against the gate any more. Retirement waits for `confirm_fired`.
    pub fn note_fired(&mut self, weapon: WeaponKind, input_tick: u32) {
        if weapon == WeaponKind::Laser {
            while self.beams_owed.front().is_some_and(|&t| t <= input_tick) {
                self.beams_owed.pop_front();
            }
            return;
        }
        for press in self.presses.iter_mut().filter(|p| p.tick <= input_tick) {
            press.accounted = true;
        }
    }

    /// The interpolator handed over the room's `Fired` for this seat, on
    /// input tick `input_tick`: the press that travelled on it (the last
    /// one at or before it - the room merges a tick's inputs newest-wins,
    /// so its tick can be later than the press's) is confirmed, and any
    /// earlier press still waiting was refused.
    ///
    /// With no press to confirm - prediction was off for it, or it was
    /// pulled before the sandbox existed - the room's shot still sets the
    /// local gate, measured back from `input_tick`, so the next press
    /// agrees with the room's cooldown.
    pub fn confirm_fired(&mut self, weapon: WeaponKind, input_tick: u32) {
        if weapon == WeaponKind::Laser {
            return;
        }
        let claimed = self.presses.iter().rposition(|p| !p.confirmed && p.tick <= input_tick);
        match claimed {
            Some(i) => {
                for (j, press) in self.presses.iter_mut().enumerate() {
                    if j < i && !press.confirmed {
                        // The room fired a later press first: this one it
                        // refused. Its shots go.
                        press.live.clear();
                        press.pending.clear();
                        press.undrawn = 0;
                        press.confirmed = true;
                        if press.drawn {
                            self.report.shots_refused += 1;
                        }
                    }
                }
                if let Some(press) = self.presses.get_mut(i) {
                    press.confirmed = true;
                    press.accounted = true;
                }
            }
            None => {
                let t = tuning();
                let interval = match weapon {
                    WeaponKind::Shell | WeaponKind::Plasma | WeaponKind::Laser => t.player_fire_interval,
                    WeaponKind::Minigun => t.minigun_burst_cooldown_seconds(),
                    WeaponKind::Missiles => t.missile_volley_cooldown_seconds(),
                    WeaponKind::Flamethrower => 0.0,
                };
                let ago = self.tick.wrapping_sub(input_tick) as f32 * PHYSICS_FIXED_DT;
                self.cooldown = self.cooldown.max(interval - ago);
            }
        }
    }

    /// This frame's room copies of this seat's shots: pair the new ones
    /// with the confirmed presses' shots in launch order, and let the
    /// room's outcome correct a provisional that disagrees - a hit the
    /// client did not draw snaps it to the room's impact, and a room copy
    /// that flies on past where the client drew a hit is shown instead.
    pub fn observe_server_shots(&mut self, shots: &[ServerShot]) {
        let paired: std::collections::BTreeSet<u16> =
            self.presses.iter().flat_map(|p| p.live.iter().filter_map(|l| l.server)).collect();
        let mut fresh: Vec<&ServerShot> = shots.iter().filter(|s| !paired.contains(&s.id)).collect();
        fresh.sort_by_key(|s| s.id);
        for s in fresh {
            let slot = self
                .presses
                .iter_mut()
                .filter(|p| p.confirmed)
                .flat_map(|p| p.live.iter_mut())
                .find(|l| l.server.is_none() && l.shot.kind == s.kind);
            match slot {
                Some(live) => {
                    live.server = Some(s.id);
                    self.unpaired.remove(&s.id);
                }
                None => {
                    self.unpaired.entry(s.id).or_insert(self.clock);
                }
            }
        }
        let by_id: std::collections::BTreeMap<u16, &ServerShot> = shots.iter().map(|s| (s.id, s)).collect();
        self.unpaired.retain(|id, _| by_id.contains_key(id));
        for live in self.presses.iter_mut().flat_map(|p| p.live.iter_mut()) {
            let Some(s) = live.server.and_then(|id| by_id.get(&id)) else { continue };
            match live.local_hit {
                None if s.impact && live.shot.is_flying() => {
                    // The room's shot hit something this client's did not
                    // meet: its impact is the truth.
                    live.shot.detonate_at(s.position);
                    self.impacts.push(s.position);
                }
                Some((at, tank)) if !live.show_server && s.flying => {
                    // Flown past where this client drew it stop: the room
                    // missed. Show its shot from here on.
                    let past = (s.position.x - at.x) * s.velocity.x + (s.position.y - at.y) * s.velocity.y;
                    let speed2 = s.velocity.x * s.velocity.x + s.velocity.y * s.velocity.y;
                    if speed2 > 0.0 && past / speed2.sqrt() > MISS_MARGIN_PX {
                        live.show_server = true;
                        if tank && !live.scored {
                            live.scored = true;
                            self.report.crossings_missed += 1;
                        }
                    }
                }
                Some((_, true)) if s.impact && !live.scored => {
                    live.scored = true;
                    self.report.crossings_hit += 1;
                }
                _ => {}
            }
        }
    }

    /// The room's copies of this seat's shots the picture should not
    /// show: every paired one whose provisional stands for it, and an
    /// unpaired one for `UNPAIRED_HOLD_SECONDS`, in case its press is about
    /// to claim it.
    pub fn hidden_server_shots(&self) -> Vec<u16> {
        let mut out: Vec<u16> = self
            .presses
            .iter()
            .flat_map(|p| p.live.iter())
            .filter(|l| !l.show_server)
            .filter_map(|l| l.server)
            .collect();
        out.extend(self.unpaired.iter().filter(|&(_, &at)| self.clock - at < UNPAIRED_HOLD_SECONDS).map(|(&id, _)| id));
        out
    }

    /// One rendered frame of the shots: every live one steps its own state
    /// machine, and one in flight is swept against the drawn world by
    /// `contact` (the round's `PresentWorld`): where it meets something
    /// it stops and plays its impact at once - damage stays the room's.
    /// A press nobody claimed within `PROVISIONAL_SECONDS` goes, counted.
    pub fn advance_shots(&mut self, dt: f32, mut contact: impl FnMut(ProvisionalKind, Position, Position) -> Option<(Position, bool)>) {
        self.clock += dt;
        let mut presses = std::mem::take(&mut self.presses);
        for press in &mut presses {
            press.age += dt;
            for live in &mut press.live {
                let from = live.shot.position;
                let flying = live.shot.is_flying();
                live.shot.advance(dt);
                if flying && live.local_hit.is_none()
                    && let Some((at, tank)) = contact(live.shot.kind, from, live.shot.position)
                {
                    live.shot.detonate_at(at);
                    live.local_hit = Some((at, tank));
                    self.impacts.push(at);
                    if tank {
                        self.report.crossings += 1;
                    }
                }
            }
            press.live.retain(|l| !l.shot.done && !l.show_server);
        }
        presses.retain(|p| {
            if p.confirmed {
                return !(p.pending.is_empty() && p.live.is_empty());
            }
            if p.age > self.refusal_after {
                if p.drawn {
                    self.report.shots_refused += 1;
                }
                return false;
            }
            true
        });
        self.presses = presses;
    }

    /// The shots to draw beside the room's, with the ids they are held
    /// under in the replica.
    pub fn shots(&self) -> impl Iterator<Item = (u32, ProvisionalShot)> + '_ {
        self.presses
            .iter()
            .flat_map(|p| p.live.iter().map(|l| l.shot))
            .enumerate()
            .map(|(i, shot)| (PROVISIONAL_ID_BASE + i as u32, shot))
    }

    /// How long a press may wait for the room's `Fired` on this link: the
    /// round trip plus the picture's delay plus a margin, never less than
    /// `PROVISIONAL_SECONDS` - a shot in flight is not taken away because
    /// the link is slow, only because the room did not fire it.
    pub fn set_refusal_after(&mut self, seconds: f32) {
        self.refusal_after = seconds.max(PROVISIONAL_SECONDS);
    }

    /// The beams pressed since the last call, for the round to draw.
    pub fn take_beams(&mut self) -> Vec<BeamPress> {
        std::mem::take(&mut self.beams)
    }

    /// Where provisional shots met something since the last call, for the
    /// round's impact flashes.
    pub fn take_impacts(&mut self) -> Vec<Position> {
        std::mem::take(&mut self.impacts)
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
        if self.owned {
            // The world around the hull is the room's; the hull is ours.
            // The snapshot's copy of it is where the room had it a round
            // trip ago and is put back where it was before the write.
            let keep = self.sandbox.seat_motion(self.seat);
            apply::snapshot(&mut self.sandbox, snapshot);
            if let Some((position, rotation, velocity)) = keep {
                self.sandbox.place_seat(self.seat, position, rotation, velocity);
            }
            self.history.clear();
            self.report.in_flight = 0;
            return;
        }
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

    /// No contact anywhere: a frame of flight in open air.
    fn open_air(_: ProvisionalKind, _: Position, _: Position) -> Option<(Position, bool)> {
        None
    }

    /// Frames of flight until `predictor`'s shots are all past the muzzle.
    fn fly(predictor: &mut Predictor, frames: usize) {
        for _ in 0..frames {
            predictor.advance_shots(1.0 / 60.0, open_air);
        }
    }

    /// Ticks of the sandbox with the trigger up, for the gate to count down.
    fn idle_ticks(predictor: &mut Predictor, ticks: usize) {
        for _ in 0..ticks {
            predictor.step(release());
        }
    }

    /// The point of a provisional shot: it is on screen on the tick of
    /// the press, it leaves from where the hull is *now*, and it holds at
    /// the muzzle for the shell's muzzle frames exactly as the room's copy
    /// does before it flies.
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
        assert!(!shell.is_flying(), "it starts in the muzzle frames, as the room's copy does");
        let gap = ((shell.position.x - hull.x).powi(2) + (shell.position.y - hull.y).powi(2)).sqrt();
        assert!(gap > 1.0, "the shell should leave the muzzle, not the hull's centre");
        assert!(gap < 100.0, "but it is a muzzle, not a mortar: {gap}");
        // Held at the muzzle, then flying.
        let before = shell.position;
        fly(&mut predictor, 3);
        assert_eq!(predictor.shots().next().expect("a shell").1.position, before, "still at the muzzle");
        fly(&mut predictor, 12);
        let (_, flying) = predictor.shots().next().expect("a shell");
        assert!(flying.is_flying() && flying.position != before, "past the muzzle frames it flies");
    }

    /// Shells fire on the press, not while held (`drive_player`), and the
    /// local gate - counted on the tick grid, as the room counts it -
    /// holds even a second press to one shot per `player_fire_interval`.
    #[test]
    fn a_shell_fires_on_the_edge_and_the_gate_rations_a_second_press() {
        let mut predictor = Predictor::new(round_with(single_barrel_row(), 0), 0, 0);
        for _ in 0..3 {
            predictor.step(press());
        }
        assert_eq!(predictor.provisional_count(), 1, "a held trigger drew more than one shell");
        // Released and pressed again inside the interval: nothing.
        predictor.step(release());
        predictor.step(press());
        assert_eq!(predictor.provisional_count(), 1, "the gate let a second press through early");
        // Past the interval in ticks, another is allowed - on a fresh edge.
        let ticks = (tuning().player_fire_interval / PHYSICS_FIXED_DT).ceil() as usize;
        idle_ticks(&mut predictor, ticks);
        predictor.step(press());
        assert_eq!(predictor.provisional_count(), 2);
    }

    /// **The bug this gate exists for.** A seat holding the minigun used
    /// to draw a *shell* on every press. The sandbox's weapon decides,
    /// and the minigun draws its burst - bullets on the tick grid the
    /// room fires them on, while the key is held.
    #[test]
    fn the_weapon_the_sandbox_holds_decides_what_is_drawn() {
        let mut predictor = Predictor::new(round(), 0, 0);
        let patch = TankPatch { minigun_ammo: Some(40), ..Default::default() };
        predictor.sandbox.debug_set_tank(0, &patch).expect("the seat's tank");
        predictor.step(press());
        let kinds: Vec<ProvisionalKind> = predictor.shots().map(|(_, s)| s.kind).collect();
        assert_eq!(kinds, [ProvisionalKind::Bullet], "the first bullet of the burst, and no shell");
        // The rest of the burst leaves on the room's tick grid while held.
        let burst = tuning().minigun_burst_size.max(1) as usize;
        let every = (tuning().minigun_bullet_delay_seconds / PHYSICS_FIXED_DT).ceil() as usize;
        for _ in 0..(burst - 1) * every {
            predictor.step(press());
        }
        assert_eq!(predictor.provisional_count(), burst, "the whole burst is on screen");
        assert!(predictor.shots().all(|(_, s)| s.kind == ProvisionalKind::Bullet));
        let headings: Vec<f32> = predictor.shots().map(|(_, s)| s.rotation).collect();
        assert!(headings.windows(2).any(|w| w[0] != w[1]), "every bullet flew dead straight: {headings:?}");
    }

    /// A twin-barrel chassis fires its left barrel on the press and its
    /// right on the twin's delay rounded up to the tick grid, from where
    /// the hull is then, exactly as `tick_queued_shots` does it.
    #[test]
    fn a_twin_barrel_draws_its_second_shell_on_the_twins_delay() {
        let mut predictor = Predictor::new(round_with(twin_barrel_row(), 0), 0, 0);
        predictor.step(press());
        assert_eq!(predictor.provisional_count(), 1, "the left barrel only, on the press");
        let first = predictor.shots().next().expect("a shell").1;
        let ticks = (tuning().tank_twin_shot_delay_seconds / PHYSICS_FIXED_DT).ceil() as usize;
        idle_ticks(&mut predictor, ticks - 1);
        assert_eq!(predictor.provisional_count(), 1, "not yet");
        idle_ticks(&mut predictor, 1);
        assert_eq!(predictor.provisional_count(), 2, "the right barrel, on the delay");
        let second = predictor.shots().nth(1).expect("a second shell").1;
        assert!((first.rotation - second.rotation).abs() < 0.001, "the barrels fire parallel");
        let across = (first.position.x - second.position.x).abs() + (first.position.y - second.position.y).abs();
        assert!(across > 1.0, "the two shells left the same barrel");
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
        let ticks = (tuning().player_fire_interval / PHYSICS_FIXED_DT).ceil() as usize;
        idle_ticks(&mut predictor, ticks);
        predictor.step(press());
        assert_eq!(predictor.provisional_count(), 1, "the second press is owed the shell the first spent");
        assert_eq!(predictor.report(0, 0).shots_drawn, 1);
    }

    /// **The own shot is never swapped for the room's copy** (§4.16): the
    /// press is confirmed by the `Fired` that names its input tick, the
    /// room's copy of the shot is paired with the provisional and taken
    /// off the picture, and the provisional is the one drawn for the
    /// shot's whole life.
    #[test]
    fn the_rooms_copy_is_paired_and_hidden_and_the_provisional_stays() {
        let mut predictor = Predictor::new(round_with(single_barrel_row(), 0), 0, 0);
        let tick = predictor.step(press());
        predictor.confirm_fired(WeaponKind::Shell, tick);
        let server = ServerShot {
            id: 7,
            kind: ProvisionalKind::Shell,
            position: Position::new(0.0, 0.0),
            velocity: crate::math::Vec2::new(0.0, -500.0),
            flying: false,
            impact: false,
        };
        predictor.observe_server_shots(&[server]);
        assert_eq!(predictor.hidden_server_shots(), vec![7], "the room's copy is taken off the picture");
        fly(&mut predictor, 20);
        assert_eq!(predictor.provisional_count(), 1, "the provisional flies on in its place");
        // An unclaimed shot of this seat's is held back a moment, then drawn.
        let stray = ServerShot { id: 9, ..server };
        predictor.observe_server_shots(&[server, stray]);
        assert!(predictor.hidden_server_shots().contains(&9), "held a moment for a press to claim it");
        fly(&mut predictor, 12);
        predictor.observe_server_shots(&[server, stray]);
        assert!(!predictor.hidden_server_shots().contains(&9), "nothing claimed it: drawn");
    }

    /// A shot that meets something in the drawn world stops there and
    /// plays its impact at once; if the room's copy then flies on past
    /// that point, the room missed, and its copy is shown instead.
    #[test]
    fn a_local_hit_stops_the_shot_and_a_room_miss_shows_the_rooms_copy() {
        let mut predictor = Predictor::new(round_with(single_barrel_row(), 0), 0, 0);
        let tick = predictor.step(press());
        predictor.confirm_fired(WeaponKind::Shell, tick);
        fly(&mut predictor, 12);
        let (_, shot) = predictor.shots().next().expect("a shell in flight");
        let wall = Position::new(shot.position.x + shot.velocity.x * 0.05, shot.position.y + shot.velocity.y * 0.05);
        predictor.advance_shots(1.0 / 60.0, |_, _, _| Some((wall, true)));
        let (_, stopped) = predictor.shots().next().expect("its impact frames");
        assert!(stopped.is_impact(), "stopped and playing its impact");
        assert_eq!(stopped.position, wall);
        assert_eq!(predictor.take_impacts(), vec![wall], "the impact is drawn at once");
        assert_eq!(predictor.report(0, 0).crossings, 1, "a tank hit this client drew");
        // The room's copy flies on well past it: the room missed.
        let past = Position::new(wall.x + shot.velocity.x * 0.2, wall.y + shot.velocity.y * 0.2);
        let server = ServerShot { id: 3, kind: ProvisionalKind::Shell, position: past, velocity: shot.velocity, flying: true, impact: false };
        predictor.observe_server_shots(&[server]);
        predictor.observe_server_shots(&[server]);
        assert!(!predictor.hidden_server_shots().contains(&3), "the room's copy is shown");
        assert_eq!(predictor.report(0, 0).crossings_missed, 1);
    }

    /// A press the server never claims - or one passed over by a `Fired`
    /// for a later press - was refused and goes, counted.
    #[test]
    fn a_press_the_server_never_confirms_is_refused() {
        let mut predictor = Predictor::new(round_with(single_barrel_row(), 0), 0, 0);
        predictor.step(press());
        predictor.advance_shots(PROVISIONAL_SECONDS - 0.01, open_air);
        assert_eq!(predictor.presses_waiting(), 1, "not yet: it is still within a round trip");
        predictor.advance_shots(0.02, open_air);
        assert_eq!(predictor.provisional_count(), 0);
        assert_eq!(predictor.report(0, 0).shots_refused, 1, "and it is counted, so a loose gate is visible");
        // Two presses; the room fires only the second. The gate counts
        // on the tick grid, so the first waits out the interval in ticks.
        let ticks = (tuning().player_fire_interval / PHYSICS_FIXED_DT).ceil() as usize;
        idle_ticks(&mut predictor, ticks);
        let first = predictor.step(press());
        idle_ticks(&mut predictor, ticks);
        let second = predictor.step(press());
        assert!(second > first);
        assert_eq!(predictor.presses_waiting(), 2, "two presses drawn");
        predictor.confirm_fired(WeaponKind::Shell, second);
        assert_eq!(predictor.report(0, 0).shots_refused, 2, "the first was refused");
    }

    /// A `Fired` with no press waiting is a shot this client did not
    /// predict; the room's shot still sets the local gate, measured back
    /// from the input tick it fired on.
    #[test]
    fn a_fired_nobody_predicted_seeds_the_local_gate() {
        let mut predictor = Predictor::new(round_with(single_barrel_row(), 0), 0, 0);
        predictor.set_shots_enabled(false);
        let fired = predictor.step(press());
        idle_ticks(&mut predictor, 4);
        assert_eq!(predictor.provisional_count(), 0, "off, nothing is drawn");
        predictor.confirm_fired(WeaponKind::Shell, fired);
        let left = predictor.report(0, 0).cooldown;
        let expected = tuning().player_fire_interval - 5.0 * PHYSICS_FIXED_DT;
        assert!((left - expected).abs() < 0.001, "the gate reads {left}, the server's is at {expected}");
        predictor.set_shots_enabled(true);
        predictor.step(press());
        assert_eq!(predictor.provisional_count(), 0, "the seeded gate held the press");
    }

    /// The laser is drawn on the press - a beam from the predicted muzzle
    /// - and charged against the sandbox's charges until its `Fired`.
    #[test]
    fn a_laser_is_drawn_on_the_press() {
        let mut predictor = Predictor::new(round(), 0, 0);
        let patch = TankPatch { laser_charges: Some(1), ..Default::default() };
        predictor.sandbox.debug_set_tank(0, &patch).expect("the seat's tank");
        let tick = predictor.step(press());
        let beams = predictor.take_beams();
        assert_eq!(beams.len(), 1, "a beam on the press");
        let ticks = (tuning().player_fire_interval / PHYSICS_FIXED_DT).ceil() as usize;
        for _ in 0..ticks {
            predictor.step(press());
        }
        assert!(predictor.take_beams().is_empty(), "the one charge is owed");
        predictor.note_fired(WeaponKind::Laser, tick);
        assert!(predictor.beams_owed.is_empty(), "its Fired settles it");
    }

    /// Firing from an owned hull kicks it back on the press, as the room
    /// kicks a hull in a local round - not a round trip later.
    #[test]
    fn an_owned_hull_recoils_on_the_press() {
        let mut predictor = Predictor::new(round_with(single_barrel_row(), 0), 0, 0);
        predictor.set_owned(true);
        idle_ticks(&mut predictor, 5);
        let (_, rotation, still) = predictor.motion().expect("a hull");
        assert!(still.x.abs() + still.y.abs() < 0.01, "standing still first");
        predictor.step(press());
        let (_, _, kicked) = predictor.motion().expect("a hull");
        let rad = rotation.to_radians();
        let back = -(kicked.x * rad.sin() - kicked.y * rad.cos());
        assert!(back > 0.1, "the hull moves back along its barrel after the press: {kicked:?}");
    }

    /// How fast `velocity` carries a hull facing `rotation` backwards.
    fn back_speed(velocity: crate::math::Vec2, rotation: f32) -> f32 {
        let rad = rotation.to_radians();
        -(velocity.x * rad.sin() - velocity.y * rad.cos())
    }

    /// **One kick per shot, drawn or not.** The room sends no `Shoved`
    /// for a shell, bolt or bullet (`weapons::apply_recoil`), so the owned
    /// hull's sandbox is the one place it is kicked: with the shots off a
    /// press still runs the gate and kicks the hull at each launch, drawing
    /// nothing, and with them on the drawn shots kick exactly as hard.
    #[test]
    fn an_owned_hull_recoils_once_per_shot_drawn_or_not() {
        let row = twin_barrel_row();
        let delay = (tuning().tank_twin_shot_delay_seconds / PHYSICS_FIXED_DT).ceil() as usize;
        let run = |drawn: bool| {
            let mut predictor = Predictor::new(round_with(row, 0), 0, 0);
            predictor.set_owned(true);
            predictor.set_shots_enabled(drawn);
            idle_ticks(&mut predictor, 5);
            let mut velocities = vec![predictor.motion().expect("a hull").2];
            predictor.step(press());
            velocities.push(predictor.motion().expect("a hull").2);
            for _ in 0..delay {
                predictor.step(release());
                velocities.push(predictor.motion().expect("a hull").2);
            }
            (velocities, predictor.provisional_count(), predictor.report(0, 0).shots_drawn)
        };
        let (drawn, on_screen, counted) = run(true);
        let (undrawn, off_screen, uncounted) = run(false);
        assert_eq!((on_screen, counted), (2, 1), "both barrels drawn, one press");
        assert_eq!((off_screen, uncounted), (0, 0), "nothing drawn or counted with the shots off");
        assert_eq!(drawn, undrawn, "the same kicks on the same ticks, drawn or not");

        // The press is one kick - what one `seat_recoil` puts on the same
        // standing hull - not none and not two.
        let mut reference = Predictor::new(round_with(row, 0), 0, 0);
        reference.set_owned(true);
        idle_ticks(&mut reference, 6);
        let lateral = tuning().tank_barrel_lateral_offset[row as usize];
        let shot = reference.sandbox.seat_shot(0, ProvisionalKind::Shell, 0.0, -lateral).expect("a shell");
        reference.sandbox.seat_recoil(0, ProvisionalKind::Shell, shot.velocity);
        let (_, rotation, once) = reference.motion().expect("a hull");
        assert!(back_speed(drawn[0], rotation).abs() < 0.01, "standing still first: {:?}", drawn[0]);
        assert_eq!(drawn[1], once, "one kick on the press");
        assert!(back_speed(once, rotation) > 0.1, "and it is backwards: {once:?}");
        // The second barrel kicks again on its delay, where the hull had
        // only been slowing down.
        let back: Vec<f32> = drawn.iter().map(|&v| back_speed(v, rotation)).collect();
        assert!(back[delay + 1] > back[delay], "the twin's kick on its tick: {back:?}");
        assert!(back[2..=delay].windows(2).all(|w| w[1] <= w[0]), "and none between: {back:?}");
    }

    /// With the shots off an owned hull's gate still rations ammo: a
    /// press the room would refuse kicks nothing.
    #[test]
    fn an_owned_hull_with_the_shots_off_is_not_kicked_by_a_refused_press() {
        let row = single_barrel_row();
        let ticks = (tuning().player_fire_interval / PHYSICS_FIXED_DT).ceil() as usize;
        let run = |second: Intent| {
            let mut predictor = Predictor::new(round_with(row, 0), 0, 0);
            predictor.set_owned(true);
            predictor.set_shots_enabled(false);
            let patch = TankPatch { shells_ammo: Some(1), ..Default::default() };
            predictor.sandbox.debug_set_tank(0, &patch).expect("the seat's tank");
            idle_ticks(&mut predictor, 5);
            predictor.step(press());
            let kicked = predictor.motion().expect("a hull").2;
            idle_ticks(&mut predictor, ticks);
            predictor.step(second);
            (kicked, predictor.motion().expect("a hull").2)
        };
        let (kicked, pressed) = run(press());
        let (_, idle) = run(release());
        assert!(kicked.x.abs() + kicked.y.abs() > 0.1, "the one shell there is kicks the hull: {kicked:?}");
        assert_eq!(pressed, idle, "the second press is owed the shell the first spent, and kicks nothing");
    }

    /// The owned hull is drawn at its newest tick, as a local round draws
    /// its newest step: a direction change shows on the frame whose tick
    /// made it.
    #[test]
    fn the_owned_hull_is_drawn_at_its_newest_tick() {
        let mut predictor = Predictor::new(round(), 0, 0);
        predictor.set_owned(true);
        for _ in 0..20 {
            predictor.step(Intent { move_dir: Some(Dir::Right), ..Intent::default() });
        }
        let (drawn, _) = predictor.drawn_pose().expect("a hull");
        let (newest, _, _) = predictor.motion().expect("a hull");
        assert_eq!(drawn, newest, "drawn where the newest tick put it");
        predictor.step(Intent { move_dir: Some(Dir::Down), ..Intent::default() });
        let (turned, _) = predictor.drawn_pose().expect("a hull");
        assert!(turned.y > drawn.y, "the tick that turned it down is the one drawn: {drawn:?} -> {turned:?}");
    }

    /// **Stage 3** (docs/online-coop-prd.md §4.14): a client that owns
    /// its hull is never pulled back by a snapshot - the room's copy is a
    /// round trip old and is put back where the sandbox had it - and
    /// nothing is replayed. The world around it is still the room's.
    #[test]
    fn an_owned_hull_keeps_its_place_through_a_snapshot_that_lags_it() {
        let (mut authority, sandbox) = (round_with_enemies(), round_with_enemies());
        let mut predictor = Predictor::new(sandbox, 0, 0);
        predictor.set_owned(true);
        for tick in 0..40u32 {
            predictor.step(Intent { move_dir: Some(Dir::Right), ..Intent::default() });
            let _ = tick;
        }
        let mine = predictor.motion().expect("a hull").0;
        // The room has the hull where it started and the enemies moved.
        for _ in 0..90 {
            authority.update(Input::default(), PHYSICS_FIXED_DT, 1088.0, 544.0);
        }
        let room: Vec<(usize, i32, i32)> =
            authority.drawable_state().tanks.iter().filter(|t| t.slot != 0).map(|t| (t.slot, t.x, t.y)).collect();
        predictor.reconcile(&wire(&mut authority, 39), 39);
        let after = predictor.motion().expect("a hull").0;
        assert_eq!((after.x, after.y), (mine.x, mine.y), "the snapshot moved an owned hull");
        let report = predictor.report(0, 0);
        assert_eq!((report.nudges, report.snaps, report.in_flight), (0, 0, 0), "nothing to reconcile: {report:?}");
        let sandbox: Vec<(usize, i32, i32)> =
            predictor.sandbox.drawable_state().tanks.iter().filter(|t| t.slot != 0).map(|t| (t.slot, t.x, t.y)).collect();
        assert_eq!(sandbox, room, "the rest of the world is still the room's");
        // The room moved the hull itself: the owned hull snaps to it.
        let far = Position::new(mine.x + 300.0, mine.y);
        predictor.place_own(far, Dir::Left.rotation());
        let (p, r, _) = predictor.motion().expect("a hull");
        assert_eq!((p.x, p.y, r), (far.x, far.y, Dir::Left.rotation()));
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
