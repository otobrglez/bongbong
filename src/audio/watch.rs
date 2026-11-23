//! The sounds no event reports (docs/sound-system-prd.md sections 4.4 and
//! 4.9): BB-199 found 220 of the game's 444 interactions exist only as a
//! value that changes - an engine revving, a minigun spinning, a fire
//! burning, a hull scraping a wall. Each watcher here reads the round once a
//! frame and asks for loops (`Wish`) and one-shots (`Play`); the director
//! starts, moves and stops them.
//!
//! Watchers read what a replica has too - positions, projectile ids, the
//! tanks' replicated state - so an online round sounds like a local one;
//! where a watcher reads what a replica lacks (`Game::contacts_against`,
//! which a client never fills), `WATCHERS` says so.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::bullet::Bullet;
use crate::math::Vec2;
use crate::simulation::{Game, HitCause};
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
    WatcherSpec {
        id: "scrape",
        entity: "tank",
        reads: "Game::contacts_against: a hull held against a face, and what the face is",
        cues: &["vehicle.scrape", "vehicle.scrape.water"],
        online: "silent: a client steps no physics (a slam is `Event::HullStruck`, which is sent)",
    },
    WatcherSpec { id: "aircraft", entity: "aircraft", reads: "Game::aircraft", cues: &["air.heli", "air.jet"], online: "the same" },
    WatcherSpec { id: "flyby", entity: "shell, bullet", reads: "projectiles' closest approach to a seat's own tank", cues: &["flyby.shell", "flyby.bullet"], online: "the same: projectiles replicate" },
    WatcherSpec {
        id: "ricochet",
        entity: "shell, bullet",
        reads: "Event::Ricochet's shot id, then that shot's position and velocity until it is gone",
        cues: &["hit.ricochet.whine"],
        online: "the same, from the sent Ricochet; a seat's own shot, which the replica does not hold, is flown on from where it turned",
    },
    WatcherSpec { id: "thunder", entity: "weather", reads: "weather::lightning on the round clock: a flash's rising edge", cues: &["world.thunder"], online: "the same: the clock is the round's" },
    WatcherSpec { id: "tesla", entity: "tesla tower", reads: "Game::tower_views charge", cues: &["world.tesla.charge"], online: "silent: tower charge is not on the wire" },
    WatcherSpec { id: "fuse", entity: "barrel", reads: "Game::fused_barrels", cues: &["world.fuse"], online: "the same" },
    WatcherSpec { id: "portal", entity: "portal", reads: "Game::portals", cues: &["world.portal"], online: "the same" },
    WatcherSpec { id: "volcano", entity: "volcano", reads: "Volcano::phase on the round clock", cues: &["world.volcano"], online: "the same: the clock is the round's" },
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
    /// Each projectile's nearest pass so far by a seat's own tank (px), and
    /// whether it already whooshed.
    passes: HashMap<u32, (f32, bool)>,
    /// The shots that ricocheted, by id, whose whine follows them away.
    whines: BTreeMap<u32, Whine>,
    /// The last frame's lightning, for a flash's rising edge.
    lightning: f32,
    flashes: u64,
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
const TESLA: u64 = 12 << 56;
const WHINE: u64 = 20 << 56;
const FUSE: u64 = 13 << 56;
const PORTAL: u64 = 14 << 56;
const VOLCANO: u64 = 15 << 56;

/// A key from a position, for a loop whose source has no id.
fn at_key(tag: u64, p: Vec2) -> u64 {
    tag | (((p.x.round() as i32 as u32 as u64) & 0xFF_FFFF) << 24) | ((p.y.round() as i32 as u32 as u64) & 0xFF_FFFF)
}

/// A ricocheted shot's whine: what the shot is, where and how fast it left
/// (for a shot this world does not hold - a seat's own online, drawn on its
/// client's timeline - which the whine flies on for alone), when, and
/// whether the shot was ever seen in the world since.
#[derive(Clone, Copy, Debug)]
struct Whine {
    shot: HitCause,
    from: Vec2,
    velocity: Vec2,
    since: f64,
    seen: bool,
}

impl Watchers {
    pub fn reset(&mut self) {
        *self = Watchers::default();
    }

    /// A round left `slot`'s gun now: what an online client's own minigun
    /// press says before the room's bullets arrive.
    pub fn round_fired(&mut self, slot: usize, now: f64) {
        self.last_round.insert(slot, now);
    }

    /// Reads the round once a frame. `own` are the owner slots this window
    /// plays; `now` is the director's clock.
    pub fn sample(&mut self, game: &Game, own: &[usize], now: f64, t: &Tuning, wishes: &mut Vec<Wish>, plays: &mut Vec<Play>) {
        self.tanks_and_treads(game, own, t, wishes, plays);
        self.minigun(game, own, now, wishes, plays);
        self.flames(game, own, wishes);
        self.fires(game, wishes);
        self.beds(game, t, wishes);
        self.scrapes(game, t, wishes);
        self.whines(game, now, t, wishes);
        self.aircraft(game, wishes);
        self.flybys(game, own, t, plays);
        self.thunder(game, t, plays);
        self.places(game, t, wishes);
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

    /// A hull held against a face scrapes, with what the face is
    /// (`source::scrape_cue`): steel on steel or stone, water slapping at
    /// deep water's edge, nothing against lava or the bong. Its slam into
    /// it is `Event::HullStruck`, which `bind.rs` plays.
    fn scrapes(&mut self, game: &Game, t: &Tuning, wishes: &mut Vec<Wish>) {
        for (slot, at, impulse, against) in game.contacts_against() {
            if impulse >= t.contact_fx_min_impulse * 4.0
                && impulse < t.ram_fx_min_impulse
                && let Some(cue) = source::scrape_cue(against)
            {
                let gain = (impulse / 200.0).clamp(0.2, 1.0);
                // A hull that slides off the wall into the water is another
                // loop, not the wall's playing on.
                let wet = if cue == ids::SCRAPE_WATER.0 { 1 << 32 } else { 0 };
                wishes.push(Wish { key: SCRAPE | wet | slot as u64, cue, at: Some(at), velocity: Vec2::new(0.0, 0.0), gain, pitch: 1.0, own: false });
            }
        }
    }

    /// The shot `id`, a `shot`, ricocheted at `at` and flies on along
    /// `heading` (degrees, 0 = up): its whine starts (`Event::Ricochet`,
    /// read by the director as each step's events come).
    pub fn ricocheted(&mut self, id: u32, shot: HitCause, at: Vec2, heading: f32, now: f64) {
        let t = crate::tuning::tuning();
        let speed = match shot {
            HitCause::Bullet => t.minigun_bullet_speed,
            HitCause::Plasma => t.plasma_speed,
            _ => t.shell_speed,
        };
        let a = heading.to_radians();
        let velocity = Vec2::new(libm::sinf(a), -libm::cosf(a)) * speed;
        self.whines.insert(id, Whine { shot, from: at, velocity, since: now, seen: false });
    }

    /// Each ricocheted shot's whine, following it by its id with its
    /// velocity, so it pans and bends away as the shot goes - a seat's own
    /// shell coming back past it too, which no fly-by sounds. It fades over
    /// `audio_whine_seconds` and stops when the shot is gone; a shot this
    /// world never holds is flown on from where it turned.
    fn whines(&mut self, game: &Game, now: f64, t: &Tuning, wishes: &mut Vec<Wish>) {
        if self.whines.is_empty() {
            return;
        }
        let mut live: HashMap<(u32, bool), (Vec2, Vec2)> = HashMap::new();
        for s in game.world.query::<&crate::shell::Shell>().iter().filter(|s| s.state == crate::shell::ShellState::Flying) {
            live.insert((s.id, true), (s.position, s.velocity));
        }
        for b in game.world.query::<&Bullet>().iter().filter(|b| b.state == crate::bullet::BulletState::Flying) {
            live.insert((b.id, false), (b.position, b.velocity));
        }
        let life = t.audio_whine_seconds.max(0.01) as f64;
        self.whines.retain(|&id, w| {
            let age = now - w.since;
            let here = live.get(&(id, w.shot != HitCause::Bullet)).copied();
            w.seen |= here.is_some();
            if age >= life || (w.seen && here.is_none()) {
                return false;
            }
            let (at, velocity) = here.unwrap_or((w.from + w.velocity * age as f32, w.velocity));
            let fade = (1.0 - age / life) as f32;
            let (gain, pitch) = if w.shot == HitCause::Bullet { (0.6, 1.2) } else { (1.0, 0.75) };
            wishes.push(Wish { key: WHINE | id as u64, cue: ids::RICOCHET_WHINE.0, at: Some(at), velocity, gain: gain * fade, pitch, own: false });
            true
        });
    }

    /// A shell or a round passing close by a seat's own tank whooshes once,
    /// as it starts to move away (`audio_flyby_px`); a seat's own shots
    /// never do.
    fn flybys(&mut self, game: &Game, own: &[usize], t: &Tuning, plays: &mut Vec<Play>) {
        let seats: Vec<Vec2> = game.world.query::<&Tank>().iter().filter(|tk| own.contains(&tk.owner_slot()) && !tk.is_wreck()).map(|tk| tk.position).collect();
        let reach = t.audio_flyby_px;
        let mut live = HashSet::new();
        let mut shots: Vec<(u32, Vec2, Vec2, &'static str)> = Vec::new();
        for s in game.world.query::<&crate::shell::Shell>().iter() {
            if !own.contains(&s.owner.slot()) {
                shots.push((s.id, s.position, s.velocity, ids::FLYBY_SHELL.0));
            }
        }
        for b in game.world.query::<&Bullet>().iter() {
            if !own.contains(&b.owner.slot()) {
                shots.push((b.id, b.position, b.velocity, ids::FLYBY_BULLET.0));
            }
        }
        for (id, at, velocity, cue) in shots {
            live.insert(id);
            let near = seats.iter().map(|s| s.distance_to(at)).fold(f32::INFINITY, f32::min);
            let pass = self.passes.entry(id).or_insert((f32::INFINITY, false));
            if !pass.1 && pass.0 <= reach && near > pass.0 {
                pass.1 = true;
                plays.push(Play::new(cue, At::Point(at - velocity * crate::PHYSICS_FIXED_DT)).source(id as u64));
            }
            pass.0 = pass.0.min(near);
        }
        self.passes.retain(|id, _| live.contains(id));
    }

    /// Thunder after a storm's lightning: the flash comes off the round
    /// clock (`weather::lightning`), so a replica hears the same strikes,
    /// and the clap follows it by a hashed half second to three.
    fn thunder(&mut self, game: &Game, t: &Tuning, plays: &mut Vec<Play>) {
        let look = game.look(t);
        if look.lightning <= 0.0 {
            self.lightning = 0.0;
            return;
        }
        let flash = crate::weather::lightning(game.time, t) * look.lightning;
        if flash > 0.6 && self.lightning <= 0.6 {
            self.flashes += 1;
            let h = (self.flashes.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as f32 / (1u64 << 24) as f32;
            plays.push(Play::new(ids::THUNDER.0, At::Nowhere).after(0.5 + 2.5 * h).strength(0.6 + 0.4 * look.lightning.min(1.0)).source(self.flashes));
        }
        self.lightning = flash;
    }

    /// What the field's own places hum with: a tesla tower charging, a
    /// fused barrel's fuse, a portal, a volcano working toward an eruption.
    fn places(&mut self, game: &Game, t: &Tuning, wishes: &mut Vec<Wish>) {
        let still = Vec2::new(0.0, 0.0);
        for tower in game.tower_views() {
            if tower.kind == crate::tower::TowerKind::Tesla && tower.charge > 0.05 && tower.disabled <= 0.0 {
                wishes.push(Wish { key: at_key(TESLA, tower.position), cue: ids::TESLA_CHARGE.0, at: Some(tower.position), velocity: still, gain: 0.4 + 0.6 * tower.charge, pitch: 0.8 + 0.5 * tower.charge, own: false });
            }
        }
        for at in game.fused_barrels() {
            wishes.push(Wish { key: at_key(FUSE, at), cue: ids::FUSE.0, at: Some(at), velocity: still, gain: 1.0, pitch: 1.0, own: false });
        }
        for &at in game.portals() {
            wishes.push(Wish { key: at_key(PORTAL, at), cue: ids::PORTAL_HUM.0, at: Some(at), velocity: still, gain: 1.0, pitch: 1.0, own: false });
        }
        for v in game.volcanoes() {
            let phase = v.phase(game.time, t);
            let gain = match phase.stage {
                crate::volcano::Stage::Asleep => 0.15,
                crate::volcano::Stage::Rumble => 0.3 + 0.7 * phase.progress,
                crate::volcano::Stage::Erupt => 1.0,
                crate::volcano::Stage::Cool => 0.5 * (1.0 - phase.progress),
            };
            let at = crate::map::cell_to_world(v.cell.0, v.cell.1);
            wishes.push(Wish { key: at_key(VOLCANO, at), cue: ids::VOLCANO.0, at: Some(at), velocity: still, gain, pitch: 0.9 + 0.2 * phase.heat.min(1.0), own: false });
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
