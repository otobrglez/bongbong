//! One rendered frame as the metrics need it: what the players asked for,
//! what the picture held, what happened, and - online - what the client's
//! own counters read - taken the same way off an online client's replica
//! and off the local twin's round, so every metric is one code path over
//! one kind of record.

use bongbong::ai::Intent;
use bongbong::bullet::BulletState;
use bongbong::net::clock::RttReport;
use bongbong::net::interp::InterpReport;
use bongbong::net::predict::{PROVISIONAL_ID_BASE, PredictionReport};
use bongbong::plasma::PlasmaState;
use bongbong::shell::ShellState;
use bongbong::simulation::replica::ShotKind;
use bongbong::simulation::{Event, Game, HitTarget};
use bongbong::tank::Dir;
use serde::{Deserialize, Serialize};

/// The seats a run scripts: the host's and the guest's.
pub const SEATS: usize = 2;

/// A hull as drawn, in field pixels.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TankSample {
    pub slot: usize,
    pub x: f32,
    pub y: f32,
    pub row: i32,
    /// `Dir::index` of the facing.
    pub dir: u8,
    pub wreck: bool,
    pub player: bool,
}

/// A projectile as drawn.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ShotSample {
    /// The replica's id: the room's per-round counter for a room copy, and
    /// for a provisional `PROVISIONAL_ID_BASE` plus the client's own number
    /// for the shot (`Live::id` within `PROVISIONAL_ID_MASK`) - fixed for
    /// the shot's life and handed out in launch order, wrapping every 4096
    /// shots.
    pub id: u32,
    /// 0 shell, 1 bullet, 2 plasma.
    pub kind: u8,
    pub x: f32,
    pub y: f32,
    /// The seat that fired it; `None` for an enemy's.
    pub seat: Option<u8>,
    /// A shot this client drew on the press, on its own timeline
    /// (`net::predict`), rather than the room's copy of one.
    pub provisional: bool,
    /// In flight: past the muzzle frames and not yet an impact.
    pub flying: bool,
    /// In its impact frames: the shot has burst.
    pub impact: bool,
    /// Where the shot is in its own life: its state's index in the kind's
    /// state list (`ShellState::ALL`, `BulletState::ALL`,
    /// `PlasmaState::ALL`) - the muzzle frames, then flight
    /// (`flying_stage`), then the impact frames up to `final_stage`. A
    /// shot's own state machine only runs forward.
    #[serde(default)]
    pub stage: u8,
}

/// The stage a shot of `kind` (0 shell, 1 bullet, 2 plasma) flies in.
pub fn flying_stage(kind: u8) -> u8 {
    let at = match kind {
        1 => BulletState::ALL.iter().position(|&s| s == BulletState::Flying),
        2 => PlasmaState::ALL.iter().position(|&s| s == PlasmaState::Flying),
        _ => ShellState::ALL.iter().position(|&s| s == ShellState::Flying),
    };
    at.unwrap_or(0) as u8
}

/// The last stage of a shot of `kind`: its last impact frame, after which
/// it is gone.
pub fn final_stage(kind: u8) -> u8 {
    let n = match kind {
        1 => BulletState::ALL.len(),
        2 => PlasmaState::ALL.len(),
        _ => ShellState::ALL.len(),
    };
    (n - 1) as u8
}

/// What the metrics read out of a frame's events.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum EventSample {
    Fired { slot: usize },
    HitEnemy { slot: usize, x: f32, y: f32 },
    HitPlayer { player: u8, x: f32, y: f32 },
}

/// An online client's own readings on one frame: the prediction's
/// counters (cumulative), the round trip, and the interpolator - what a
/// time series of the link is read from.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct LinkSample {
    /// Corrections eased off and taken whole since the round opened.
    pub nudges: u32,
    pub snaps: u32,
    /// The largest correction so far, pixels.
    pub max_error_px: f32,
    /// Inputs the room had not acknowledged at the last reconciliation.
    pub in_flight: usize,
    /// Provisional shots on screen.
    pub shots_on_screen: usize,
    pub shots_drawn: u32,
    pub shots_refused: u32,
    pub crossings: u32,
    pub crossings_hit: u32,
    pub crossings_missed: u32,
    /// The lead's smoothed reading of the seat's mailbox depth.
    pub lead_depth: f32,
    /// The measured round trip (median of the window), once a probe has
    /// been answered.
    pub rtt_ms: Option<f64>,
    /// How far ahead of the picture the newest snapshot stands.
    pub buffer_ms: Option<f64>,
    /// The interpolation delay in force, and where it is heading.
    pub delay_ms: f64,
    pub target_ms: f64,
    pub jitter_ms: f64,
    pub interval_ms: f64,
    pub buffered: usize,
    /// This frame ran past the newest snapshot on last-known velocities.
    pub extrapolated: bool,
    /// Frames drawn on extrapolation since the round opened.
    pub extrapolated_frames: u64,
    /// Arrival lateness behind the clock's envelope, median and 95th.
    pub lateness_p50_ms: f64,
    pub lateness_p95_ms: f64,
    /// Head-of-line stalls ridden out on extrapolation so far.
    pub stalls: u64,
    /// The playout clock's speed this frame (1.0 settled).
    pub rate: f64,
    /// Corrections eased by an error offset so far, and their p95 size.
    pub corrections: u64,
    pub correction_p95_px: f64,
}

impl LinkSample {
    /// The readings off a client's accessors. `prev_extrapolated` is the
    /// last frame's cumulative count, so the flag says whether *this*
    /// frame extrapolated.
    pub fn read(
        prediction: Option<PredictionReport>,
        lead_depth: f32,
        rtt: Option<RttReport>,
        buffer_ms: Option<f64>,
        interp: &InterpReport,
        prev_extrapolated: Option<u64>,
    ) -> LinkSample {
        let p = prediction.unwrap_or_default();
        LinkSample {
            nudges: p.nudges,
            snaps: p.snaps,
            max_error_px: p.max_error_px,
            in_flight: p.in_flight,
            shots_on_screen: p.shots_on_screen,
            shots_drawn: p.shots_drawn,
            shots_refused: p.shots_refused,
            crossings: p.crossings,
            crossings_hit: p.crossings_hit,
            crossings_missed: p.crossings_missed,
            lead_depth,
            rtt_ms: rtt.map(|r| r.rtt_ms),
            buffer_ms,
            delay_ms: interp.delay_ms,
            target_ms: interp.target_ms,
            jitter_ms: interp.jitter_ms,
            interval_ms: interp.interval_ms,
            buffered: interp.buffered,
            extrapolated: prev_extrapolated.is_some_and(|prev| interp.extrapolated_frames > prev),
            extrapolated_frames: interp.extrapolated_frames,
            lateness_p50_ms: interp.lateness_p50_ms,
            lateness_p95_ms: interp.lateness_p95_ms,
            stalls: interp.stalls,
            rate: interp.rate,
            corrections: interp.corrections,
            correction_p95_px: interp.error_p95_px,
        }
    }
}

/// One rendered frame.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FrameSample {
    /// The process clock when the frame was drawn, milliseconds.
    pub t_ms: f64,
    /// Seconds since the seat's script began.
    pub script_s: f64,
    /// Each seat's scripted movement this frame (`Dir::index`), by seat:
    /// the twin records both, an online client only its own.
    pub move_dirs: [Option<u8>; SEATS],
    /// Each seat's trigger tap this frame, the same way.
    pub fires: [bool; SEATS],
    pub tanks: Vec<TankSample>,
    pub shots: Vec<ShotSample>,
    /// The events handed over on this frame.
    pub events: Vec<EventSample>,
    /// The CPU the frame's work took: `OnlineRound::frame` online, the
    /// frame's `Game::update` steps in the twin. Microseconds.
    pub cpu_us: f64,
    /// The world's tick as drawn (`Game::frame`).
    pub tick: u64,
    /// The client's own readings (online only).
    pub link: Option<LinkSample>,
}

impl FrameSample {
    pub fn tank(&self, slot: usize) -> Option<&TankSample> {
        self.tanks.iter().find(|t| t.slot == slot)
    }

    /// `seat`'s scripted movement this frame, where this record knows it.
    pub fn move_dir(&self, seat: usize) -> Option<Dir> {
        self.move_dirs.get(seat).copied().flatten().and_then(|i| Dir::ALL.get(i as usize).copied())
    }

    /// Whether `seat` tapped the trigger this frame.
    pub fn fire(&self, seat: usize) -> bool {
        self.fires.get(seat).copied().unwrap_or(false)
    }
}

/// The index of `dir` in `Dir::ALL`, the wire's and the picture's.
pub fn dir_index(dir: Dir) -> u8 {
    dir.index() as u8
}

/// Whether a drawn shot of `kind` in sheet column `state` is in flight.
fn in_flight(kind: ShotKind, state: i32) -> bool {
    match kind {
        ShotKind::Shell => ShellState::from_col(state) == Some(ShellState::Flying),
        ShotKind::Bullet => BulletState::from_col(state) == Some(BulletState::Flying),
        ShotKind::Plasma => PlasmaState::from_col(state) == Some(PlasmaState::Flying),
        // A gravity well's orb: 0 while it swells at the muzzle, 1 flying.
        ShotKind::Orb => state == 1,
    }
}

/// Where a drawn shot of `kind` in sheet column `state` is in its life
/// (`ShotSample::stage`).
fn stage(kind: ShotKind, state: i32) -> u8 {
    let at = match kind {
        ShotKind::Shell => ShellState::from_col(state).and_then(|s| ShellState::ALL.iter().position(|&a| a == s)),
        ShotKind::Bullet => BulletState::from_col(state).and_then(|s| BulletState::ALL.iter().position(|&a| a == s)),
        ShotKind::Plasma => PlasmaState::from_col(state).and_then(|s| PlasmaState::ALL.iter().position(|&a| a == s)),
        ShotKind::Orb => Some(state.clamp(0, 1) as usize),
    };
    at.unwrap_or(0) as u8
}

/// Whether a drawn shot of `kind` in sheet column `state` is in its impact
/// frames.
fn bursting(kind: ShotKind, state: i32) -> bool {
    match kind {
        ShotKind::Shell => matches!(ShellState::from_col(state), Some(ShellState::Hit0 | ShellState::Hit1 | ShellState::Hit2)),
        ShotKind::Bullet => BulletState::from_col(state) == Some(BulletState::Hit),
        ShotKind::Plasma => matches!(PlasmaState::from_col(state), Some(PlasmaState::Hit0 | PlasmaState::Hit1 | PlasmaState::Hit2)),
        // An orb anchors rather than bursts.
        ShotKind::Orb => false,
    }
}

/// The picture half of a sample: the hulls and the shots as `game` holds
/// them now. `local_seat` is the seat whose provisionals the picture
/// holds (an online client's own), `None` for the twin.
///
/// Every shot's firer is read by id, not guessed from its position: a
/// provisional is the local seat's, a room copy is a seat's when that
/// seat's own list (`Game::seat_shots`) names its id, and an enemy's
/// otherwise.
pub fn read_picture(game: &Game, local_seat: Option<u8>, sample: &mut FrameSample) {
    let state = game.drawable_state();
    sample.tanks = state
        .tanks
        .iter()
        .map(|t| TankSample {
            slot: t.slot,
            x: t.x as f32 / 4.0,
            y: t.y as f32 / 4.0,
            row: t.row,
            dir: t.dir,
            wreck: t.wreck,
            player: t.player.is_some(),
        })
        .collect();
    let mut seat_of = std::collections::BTreeMap::new();
    for seat in 0..game.players.count() as u8 {
        for s in game.seat_shots(seat) {
            seat_of.insert(s.id as u32, seat);
        }
    }
    sample.shots = state
        .shots
        .iter()
        .map(|s| {
            let provisional = s.id >= PROVISIONAL_ID_BASE;
            ShotSample {
                id: s.id,
                kind: match s.kind {
                    ShotKind::Shell => 0,
                    ShotKind::Bullet => 1,
                    ShotKind::Plasma => 2,
                    ShotKind::Orb => 3,
                },
                x: s.x as f32 / 4.0,
                y: s.y as f32 / 4.0,
                seat: if provisional { local_seat } else { seat_of.get(&s.id).copied() },
                provisional,
                flying: in_flight(s.kind, s.state),
                impact: bursting(s.kind, s.state),
                stage: stage(s.kind, s.state),
            }
        })
        .collect();
    sample.tick = game.frame();
}

/// The events the metrics read, off a slice of the simulation's.
pub fn read_events(events: &[Event], out: &mut Vec<EventSample>) {
    for e in events {
        match *e {
            Event::Fired { slot, .. } => out.push(EventSample::Fired { slot }),
            Event::Hit { target: HitTarget::Enemy { slot }, x, y, .. } => out.push(EventSample::HitEnemy { slot, x, y }),
            Event::Hit { target: HitTarget::Player { player }, x, y, .. } => out.push(EventSample::HitPlayer { player, x, y }),
            _ => {}
        }
    }
}

/// `seat`'s intent this frame in sample form.
pub fn read_intent(seat: usize, intent: &Intent, sample: &mut FrameSample) {
    if seat < SEATS {
        sample.move_dirs[seat] = intent.move_dir.map(dir_index);
        sample.fires[seat] = intent.fire;
    }
}
