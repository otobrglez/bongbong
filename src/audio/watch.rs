//! The sounds no event reports (docs/sound-system-prd.md sections 4.4 and
//! 4.9): BB-199 found 220 of the game's 444 interactions exist only as a
//! value that changes - an engine revving, a minigun spinning, a fire
//! burning, a hull scraping a wall. Each watcher here reads the round once a
//! frame and asks for loops (`Wish`) and one-shots (`Play`); the director
//! starts, moves and stops them.
//!
//! Watchers read what a replica has too - positions, projectile ids, the
//! tanks' replicated state - so an online round sounds like a local one;
//! where a watcher reads what a replica lacks (`Game::contacts`, which a
//! client never fills), `WATCHERS` says so.

use std::collections::{HashMap, HashSet};

use crate::bullet::Bullet;
use crate::math::Vec2;
use crate::simulation::Game;
use crate::tank::Tank;
use crate::tuning::Tuning;

use super::bind::{At, Play};
use super::cues::ids;
use super::source;

/// A loop a watcher wants playing this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Wish {
    /// Names the loop from frame to frame: a watcher's tag and its source.
    pub key: u64,
    pub cue: &'static str,
    /// `None` for a bed with no place.
    pub at: Option<Vec2>,
    pub velocity: Vec2,
    /// A level scale, 1 as the cue is mixed.
    pub gain: f32,
    pub pitch: f32,
    /// A seat played on this window: never culled, never delayed.
    pub own: bool,
}

/// One watcher, as the sound map lists it (section 4.11).
pub struct WatcherSpec {
    pub id: &'static str,
    /// What it listens to.
    pub entity: &'static str,
    /// What it reads.
    pub reads: &'static str,
    /// The cues it plays.
    pub cues: &'static [&'static str],
    /// What an online client hears of it.
    pub online: &'static str,
}

pub const WATCHERS: &[WatcherSpec] = &[
    WatcherSpec {
        id: "engine",
        entity: "tank",
        reads: "position over time (speed, acceleration); the chassis's mass and family",
        cues: &["engine.light", "engine.medium", "engine.heavy", "engine.twin", "vehicle.treads", "vehicle.ground.grass", "vehicle.ground.sand", "vehicle.ground.water", "vehicle.ground.ice"],
        online: "the same: positions replicate",
    },
    WatcherSpec { id: "minigun", entity: "tank", reads: "new bullet ids per owner slot", cues: &["weapon.minigun.loop", "weapon.minigun.spin_down"], online: "the same: bullets replicate" },
    WatcherSpec { id: "flame", entity: "flamethrower", reads: "Game::flames", cues: &["weapon.flamethrower.loop"], online: "the same: flame jets replicate" },
    WatcherSpec { id: "charge", entity: "tank", reads: "Tank::charge", cues: &["weapon.gauss_rail.charge"], online: "from the replicated charge ticks" },
    WatcherSpec { id: "tell", entity: "tank", reads: "Tank::tell turning on", cues: &["warning.windup"], online: "from the replicated tell" },
    WatcherSpec { id: "fire", entity: "fire", reads: "Game::burning_cells, burning_tiles, burning_wrecks, clustered", cues: &["world.fire"], online: "the same" },
    WatcherSpec { id: "wreck", entity: "wreck", reads: "burning wrecks, inside the fire clusters", cues: &["world.fire"], online: "the same" },
    WatcherSpec { id: "sky", entity: "weather", reads: "Game::look (rain, sand, snow)", cues: &["world.rain", "world.wind", "world.wind.soft", "world.night"], online: "the same: the sky is the map's" },
    WatcherSpec { id: "place", entity: "map", reads: "the map's theme and biome", cues: &["bed.grass", "bed.desert", "bed.valley", "bed.floodplain", "bed.karst"], online: "the same" },
    WatcherSpec { id: "slam", entity: "tank", reads: "Game::contacts impulse spikes", cues: &["vehicle.slam", "vehicle.scrape"], online: "silent: a client steps no physics (BB-214 adds Slam)" },
    WatcherSpec { id: "aircraft", entity: "aircraft", reads: "Game::aircraft", cues: &["air.heli", "air.jet"], online: "the same" },
];

/// What a watcher keeps about a tank between frames.
#[derive(Clone, Copy, Debug, Default)]
struct TankEar {
    last: Vec2,
    velocity: Vec2,
    rpm: f32,
    frame: u64,
    tell: bool,
}

#[derive(Default)]
pub struct Watchers {
    tanks: HashMap<usize, TankEar>,
    bullets_seen: HashSet<u32>,
    last_round: HashMap<usize, f64>,
    spinning: HashSet<usize>,
    impulse: HashMap<usize, f32>,
    last_slam: HashMap<usize, f64>,
}

const ENGINE: u64 = 1 << 56;
const TREADS: u64 = 2 << 56;
const GROUND: u64 = 3 << 56;
const MINIGUN: u64 = 4 << 56;
const FLAME: u64 = 5 << 56;
const FIRE: u64 = 6 << 56;
const SKY: u64 = 7 << 56;
const PLACE: u64 = 8 << 56;
const SCRAPE: u64 = 9 << 56;
const CHARGE: u64 = 10 << 56;
const AIR: u64 = 11 << 56;

impl Watchers {
    pub fn reset(&mut self) {
        *self = Watchers::default();
    }

    /// Reads the round once a frame. `own` are the owner slots this window
    /// plays; `now` is the director's clock.
    pub fn sample(&mut self, game: &Game, own: &[usize], now: f64, t: &Tuning, wishes: &mut Vec<Wish>, plays: &mut Vec<Play>) {
        self.tanks_and_treads(game, own, t, wishes, plays);
        self.minigun(game, own, now, wishes, plays);
        self.flames(game, own, wishes);
        self.fires(game, wishes);
        self.beds(game, t, wishes);
        self.slams(game, now, t, wishes, plays);
        self.aircraft(game, wishes);
    }

    fn tanks_and_treads(&mut self, game: &Game, own: &[usize], t: &Tuning, wishes: &mut Vec<Wish>, plays: &mut Vec<Play>) {
        let frame = game.frame();
        let mut live = HashSet::new();
        for tank in game.world.query::<&Tank>().iter() {
            let slot = tank.owner_slot();
            live.insert(slot);
            let ear = self.tanks.entry(slot).or_insert(TankEar { last: tank.position, frame, ..TankEar::default() });
            let steps = frame.saturating_sub(ear.frame);
            if steps > 0 {
                let dt = steps as f32 * crate::PHYSICS_FIXED_DT;
                let v = (tank.position - ear.last) * (1.0 / dt);
                // A jump - a portal, a respawn - is not a speed.
                let v = if v.length() > crate::PHYSICS_MAX_SPEED { Vec2::new(0.0, 0.0) } else { v };
                let k = 1.0 - libm::expf(-dt / 0.12);
                ear.velocity = ear.velocity + (v - ear.velocity) * k;
                let top = tank.base_speed().max(1.0);
                let target = (ear.velocity.length() / top).clamp(0.0, 1.0);
                let spool = (t.audio_engine_spool_s * tank.mass_factor()).max(0.01);
                ear.rpm += (target - ear.rpm) * (1.0 - libm::expf(-dt / spool));
                ear.last = tank.position;
                ear.frame = frame;
            }
            let tell = tank.tell.is_some();
            if tell && !ear.tell {
                plays.push(Play::new(ids::TELL_WINDUP.0, At::Slot(slot)).source(slot as u64));
            }
            ear.tell = tell;
            if tank.is_wreck() {
                continue;
            }
            let mine = own.contains(&slot);
            let family = source::engine_family(crate::tank::TankKind::from_row(tank.row).unwrap_or(crate::tank::TankKind::Assault));
            let others = if mine { 1.0 } else { super::dsp::db_to_gain(t.audio_engine_others_db) };
            let rpm = ear.rpm;
            let pitch = source::engine_pitch(tank.mass_factor()) * (t.audio_engine_idle_pitch + (t.audio_engine_top_pitch - t.audio_engine_idle_pitch) * rpm);
            let gain = (t.audio_engine_idle_gain + (t.audio_engine_drive_gain - t.audio_engine_idle_gain) * rpm) * others;
            let velocity = ear.velocity;
            wishes.push(Wish { key: ENGINE | slot as u64, cue: family.cue(), at: Some(tank.position), velocity, gain, pitch, own: mine });
            let speed = velocity.length();
            let roll = (speed / t.audio_treads_full_px_s).clamp(0.0, 1.0);
            if roll > 0.02 {
                let pitch = 0.85 + 0.3 * roll;
                wishes.push(Wish { key: TREADS | slot as u64, cue: ids::TREADS.0, at: Some(tank.position), velocity, gain: roll * others, pitch, own: mine });
                if let Some(cue) = source::surface_layer(tank.tread.surface) {
                    wishes.push(Wish { key: GROUND | slot as u64, cue, at: Some(tank.position), velocity, gain: roll * others, pitch: 1.0, own: mine });
                }
            }
        }
        self.tanks.retain(|slot, _| live.contains(slot));
    }

    fn minigun(&mut self, game: &Game, own: &[usize], now: f64, wishes: &mut Vec<Wish>, plays: &mut Vec<Play>) {
        let mut live = HashSet::new();
        for b in game.world.query::<&Bullet>().iter() {
            live.insert(b.id);
            if self.bullets_seen.insert(b.id) && !b.owner.is_tower() {
                self.last_round.insert(b.owner.slot(), now);
            }
        }
        self.bullets_seen.retain(|id| live.contains(id));
        let positions: HashMap<usize, Vec2> = game.world.query::<&Tank>().iter().map(|t| (t.owner_slot(), t.position)).collect();
        let mut firing = HashSet::new();
        for (&slot, &at) in &self.last_round {
            if now - at < 0.15
                && let Some(&pos) = positions.get(&slot)
            {
                firing.insert(slot);
                wishes.push(Wish { key: MINIGUN | slot as u64, cue: ids::MINIGUN_LOOP.0, at: Some(pos), velocity: Vec2::new(0.0, 0.0), gain: 1.0, pitch: 1.0, own: own.contains(&slot) });
            }
        }
        for &slot in self.spinning.difference(&firing) {
            plays.push(Play::new(ids::MINIGUN_SPIN_DOWN.0, At::Slot(slot)).source(slot as u64));
        }
        self.spinning = firing;
        self.last_round.retain(|_, &mut at| now - at < 2.0);
    }

    fn flames(&mut self, game: &Game, own: &[usize], wishes: &mut Vec<Wish>) {
        for jet in game.flames() {
            let slot = jet.owner.slot();
            wishes.push(Wish { key: FLAME | slot as u64, cue: ids::FLAME_LOOP.0, at: Some(jet.nozzle), velocity: Vec2::new(0.0, 0.0), gain: 1.0, pitch: 1.0, own: own.contains(&slot) });
        }
        for tank in game.world.query::<&Tank>().iter() {
            if let Some(charge) = &tank.charge {
                let slot = tank.owner_slot();
                let progress = charge.progress();
                wishes.push(Wish {
                    key: CHARGE | slot as u64,
                    cue: ids::CHARGE_GAUSS.0,
                    at: Some(tank.position),
                    velocity: Vec2::new(0.0, 0.0),
                    gain: 0.6 + 0.4 * progress,
                    pitch: 0.8 + 0.8 * progress,
                    own: own.contains(&slot),
                });
            }
        }
    }

    /// Fires, gathered into clusters of 4 x 4 cells: one loop a cluster,
    /// louder the more it burns, never one a cell.
    fn fires(&mut self, game: &Game, wishes: &mut Vec<Wish>) {
        let cluster = 4.0 * crate::OBSTACLE_GRID_SIZE;
        let mut buckets: std::collections::BTreeMap<(i32, i32), (Vec2, u32)> = std::collections::BTreeMap::new();
        let mut add = |p: Vec2| {
            let b = ((p.x / cluster).floor() as i32, (p.y / cluster).floor() as i32);
            let e = buckets.entry(b).or_insert((Vec2::new(0.0, 0.0), 0));
            e.0 += p;
            e.1 += 1;
        };
        for (p, _, _) in game.burning_cells() {
            add(p);
        }
        for (p, _) in game.burning_tiles() {
            add(p);
        }
        for (p, _) in game.burning_wrecks() {
            add(p);
        }
        for ((bx, by), (sum, n)) in buckets {
            let at = sum * (1.0 / n as f32);
            let key = FIRE | ((bx as u32 as u64) << 24) | (by as u32 as u64 & 0xFF_FFFF);
            wishes.push(Wish { key, cue: ids::FIRE_LOOP.0, at: Some(at), velocity: Vec2::new(0.0, 0.0), gain: (0.4 + 0.15 * n as f32).min(1.2), pitch: 1.0, own: false });
        }
    }

    fn beds(&mut self, game: &Game, t: &Tuning, wishes: &mut Vec<Wish>) {
        let look = game.look(t);
        if let Some(cue) = source::sky_bed(game.weather()) {
            let gain = match cue {
                "world.rain" => look.rain,
                "world.wind" => look.sand,
                "world.wind.soft" => look.snow,
                _ => 0.6,
            };
            if gain > 0.01 {
                wishes.push(Wish { key: SKY, cue, at: None, velocity: Vec2::new(0.0, 0.0), gain, pitch: 1.0, own: false });
            }
        }
        let place = source::place_bed(game.map.theme, game.map.biome);
        wishes.push(Wish { key: PLACE, cue: place, at: None, velocity: Vec2::new(0.0, 0.0), gain: 1.0, pitch: 1.0, own: false });
    }

    /// A hull slamming into something: an impulse spike, as `Fx::watch_slams`
    /// reads it; a held contact scrapes.
    fn slams(&mut self, game: &Game, now: f64, t: &Tuning, wishes: &mut Vec<Wish>, plays: &mut Vec<Play>) {
        for (slot, at, impulse, touching_tank) in game.contacts() {
            let before = self.impulse.insert(slot, impulse).unwrap_or(0.0);
            let rose = impulse >= t.ram_fx_min_impulse && impulse >= t.ram_fx_rise * before.max(1.0);
            let rested = self.last_slam.get(&slot).is_none_or(|&last| now - last >= t.ram_fx_gap_seconds as f64);
            if rose && rested && !touching_tank {
                self.last_slam.insert(slot, now);
                plays.push(Play::new(ids::SLAM.0, At::Point(at)).strength(crate::fx::slam_strength(impulse, t)).source(slot as u64));
            } else if impulse >= t.contact_fx_min_impulse * 4.0 {
                let gain = (impulse / 200.0).clamp(0.2, 1.0);
                wishes.push(Wish { key: SCRAPE | slot as u64, cue: ids::SCRAPE.0, at: Some(at), velocity: Vec2::new(0.0, 0.0), gain, pitch: 1.0, own: false });
            }
        }
    }

    fn aircraft(&mut self, game: &Game, wishes: &mut Vec<Wish>) {
        for a in game.aircraft() {
            let cue = match a.sortie {
                crate::air_support::Sortie::Heli => "air.heli",
                crate::air_support::Sortie::Jet => "air.jet",
            };
            wishes.push(Wish { key: AIR | a.id as u64, cue, at: Some(a.ground), velocity: a.velocity, gain: 1.0, pitch: 1.0, own: false });
        }
    }
}
