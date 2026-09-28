//! One rendered frame as the metrics need it: what the player asked for,
//! what the picture held, and what happened - read the same way off an
//! online client's replica and off the local twin's round, so every
//! metric is one code path over one kind of record.

use bongbong::ai::Intent;
use bongbong::simulation::debug::Detail;
use bongbong::simulation::replica::ShotKind;
use bongbong::simulation::{Event, Game, HitTarget};
use bongbong::tank::Dir;
use serde::Serialize;

/// A hull as drawn, in field pixels.
#[derive(Clone, Copy, Debug, Serialize)]
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
#[derive(Clone, Copy, Debug, Serialize)]
pub struct ShotSample {
    pub id: u32,
    /// 0 shell, 1 bullet, 2 plasma.
    pub kind: u8,
    pub x: f32,
    pub y: f32,
    /// The firing slot, when the debug snapshot names it: a seat's index,
    /// an enemy's slot, `usize::MAX` for an enemy's shot on a replica.
    pub owner: Option<usize>,
    /// A shot this client drew on the press, ahead of the room
    /// (`net::predict::PROVISIONAL_ID_BASE` and up).
    pub provisional: bool,
    /// Still in flight (not at the muzzle, not bursting).
    pub flying: bool,
}

/// What the metrics read out of a frame's events.
#[derive(Clone, Copy, Debug, Serialize)]
pub enum EventSample {
    Fired { slot: usize },
    HitEnemy { slot: usize, x: f32, y: f32 },
    HitPlayer { player: u8, x: f32, y: f32 },
}

/// One rendered frame.
#[derive(Clone, Debug, Default, Serialize)]
pub struct FrameSample {
    /// The process clock when the frame was drawn, milliseconds.
    pub t_ms: f64,
    /// Seconds since the seat's script began.
    pub script_s: f64,
    /// The script's movement this frame (`Dir::index`), if any.
    pub move_dir: Option<u8>,
    /// The script tapped the trigger this frame.
    pub fire: bool,
    pub tanks: Vec<TankSample>,
    pub shots: Vec<ShotSample>,
    /// The events handed over on this frame.
    pub events: Vec<EventSample>,
    /// The CPU the frame's work took: `OnlineRound::frame` online, the
    /// frame's `Game::update` steps in the twin. Microseconds.
    pub cpu_us: f64,
    /// The world's tick as drawn (`Game::frame`).
    pub tick: u64,
    /// The interpolation delay in force, milliseconds (online only).
    pub interp_delay_ms: Option<f64>,
}

impl FrameSample {
    pub fn tank(&self, slot: usize) -> Option<&TankSample> {
        self.tanks.iter().find(|t| t.slot == slot)
    }

    pub fn move_dir(&self) -> Option<Dir> {
        self.move_dir.and_then(|i| Dir::ALL.get(i as usize).copied())
    }
}

/// The index of `dir` in `Dir::ALL`, the wire's and the picture's.
pub fn dir_index(dir: Dir) -> u8 {
    dir.index() as u8
}

/// The picture half of a sample: the hulls and the shots as `game` holds
/// them now.
pub fn read_picture(game: &Game, sample: &mut FrameSample) {
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
    // The drawable shots carry the id and the kind; the debug snapshot
    // carries the owner and whether it flies. Matched on kind and
    // position - the debug copy is rounded to a tenth of a pixel.
    let (w, h) = game.map.field_size();
    let debug = game.debug_snapshot(w, h, Detail::Compact);
    let mut claimed = vec![false; debug.projectiles.len()];
    sample.shots = state
        .shots
        .iter()
        .map(|s| {
            let kind = match s.kind {
                ShotKind::Shell => 0u8,
                ShotKind::Bullet => 1,
                ShotKind::Plasma => 2,
            };
            let name = ["shell", "bullet", "plasma"][kind as usize];
            let (x, y) = (s.x as f32 / 4.0, s.y as f32 / 4.0);
            let found = debug
                .projectiles
                .iter()
                .enumerate()
                .filter(|(i, p)| !claimed[*i] && p.kind == name)
                .map(|(i, p)| (i, (p.x - x).abs() + (p.y - y).abs()))
                .filter(|(_, d)| *d <= 0.6)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i);
            let (owner, flying) = match found {
                Some(i) => {
                    claimed[i] = true;
                    let p = &debug.projectiles[i];
                    (Some(p.owner), p.state == "flying")
                }
                None => (None, true),
            };
            ShotSample {
                id: s.id,
                kind,
                x,
                y,
                owner,
                provisional: s.id >= bongbong::net::predict::PROVISIONAL_ID_BASE,
                flying,
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

/// The frame's intent in sample form.
pub fn read_intent(intent: &Intent, sample: &mut FrameSample) {
    sample.move_dir = intent.move_dir.map(dir_index);
    sample.fire = intent.fire;
}
