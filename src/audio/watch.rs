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
use crate::ground::Depth;
use crate::math::Vec2;
use crate::simulation::{Game, HitCause};
use crate::tank::Tank;
use crate::tuning::Tuning;
use crate::wear::{Roll, Sky, Surface};

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
        cues: &["engine.light", "engine.medium", "engine.heavy", "engine.twin", "vehicle.treads"],
        online: "the same: positions replicate",
    },
    WatcherSpec {
        id: "minigun",
        entity: "tank",
        reads: "new bullet ids per owner slot; Tank::minigun_spin leaving rest",
        cues: &["weapon.minigun.loop", "weapon.minigun.spin_down", "weapon.minigun.spin_up"],
        online: "the same: bullets replicate, and a replica spins its barrels on the room's Fired",
    },
    WatcherSpec {
        id: "flame",
        entity: "flamethrower",
        reads: "Game::flames, and each slot's jet starting and stopping against Tank::flame_fuel",
        cues: &["weapon.flamethrower.loop", "weapon.flamethrower.start", "weapon.flamethrower.stop", "weapon.flamethrower.stop.dry"],
        online: "the same: flame jets and the fuel replicate",
    },
    WatcherSpec {
        id: "charge",
        entity: "tank",
        reads: "Tank::charge of a rail: its progress, its stage (Charge::stage) and, for a seat's own, Charge::vent_in",
        cues: &["weapon.gauss_rail.charge", "weapon.gauss_rail.ready", "weapon.gauss_rail.charge.overcharged", "weapon.gauss_rail.vent.alarm"],
        online: "from the replicated charge ticks",
    },
    WatcherSpec {
        id: "rod",
        entity: "tank",
        reads: "Tank::charge of a rod, and a seat's own Tank::reticle stepping a cell",
        cues: &["weapon.rod_from_god.uplink", "weapon.rod_from_god.reticle"],
        online: "from the replicated charge ticks and reticle",
    },
    WatcherSpec {
        id: "rail_lane",
        entity: "tank",
        reads: "Game::rail_lanes holding a seat's own tank, and the charging tank's Charge::progress",
        cues: &["warning.lock.rail"],
        online: "the same: the charge replicates",
    },
    WatcherSpec {
        id: "drone_lock",
        entity: "fpv drone",
        reads: "Drone::lock on a seat's own tank, and the nearest such drone's distance",
        cues: &["warning.lock.drone"],
        online: "the same: the lock replicates (DroneState::lock)",
    },
    WatcherSpec {
        id: "kill",
        entity: "tank",
        reads: "RoundStats::by_seat rising for a seat this window plays",
        cues: &["ui.kill"],
        online: "silent: a replica keeps no wreck tally",
    },
    WatcherSpec {
        id: "missile",
        entity: "missile",
        reads: "new missile ids, and each missile's position over time",
        cues: &["weapon.missiles.fire", "weapon.missiles.motor"],
        online: "the same: missiles replicate",
    },
    WatcherSpec { id: "dive", entity: "fpv drone", reads: "DroneStage::Dive and its stage clock", cues: &["weapon.fpv_swarm.dive"], online: "the same: the stage replicates" },
    WatcherSpec { id: "grenade_fuse", entity: "grenade", reads: "Grenade::lamp_lit turning on", cues: &["weapon.grenades.beep"], online: "the same: the fuse replicates" },
    WatcherSpec {
        id: "special",
        entity: "tank",
        reads: "for a seat's own: Tank::special_down, Tank::special, shells_ammo, missile_tubes_empty",
        cues: &[
            "weapon.offline",
            "weapon.online",
            "weapon.spent",
            "weapon.shell.reload",
            "weapon.shell.reload.full",
            "weapon.missiles.reload",
            "weapon.missiles.reload.ready",
        ],
        online: "the same but the missile pod's reload, which a replica does not run",
    },
    WatcherSpec {
        id: "near_miss",
        entity: "rail slug, laser beam",
        reads: "Event::RailSlug and Event::LaserBeam segments against a seat's own tank, less the step's hits of their kind",
        cues: &["flyby.rail", "flyby.laser"],
        online: "the same: both events are sent",
    },
    WatcherSpec { id: "tell", entity: "tank", reads: "Tank::tell turning on", cues: &["warning.windup"], online: "from the replicated tell" },
    WatcherSpec { id: "fire", entity: "fire", reads: "Game::burning_cells, burning_tiles, burning_wrecks, clustered", cues: &["world.fire"], online: "the same" },
    WatcherSpec { id: "wreck", entity: "wreck", reads: "burning wrecks, inside the fire clusters", cues: &["world.fire"], online: "the same" },
    WatcherSpec { id: "sky", entity: "weather", reads: "Game::look (rain, sand, snow)", cues: &["world.rain", "world.wind", "world.wind.soft", "world.night"], online: "the same: the sky is the map's" },
    WatcherSpec {
        id: "ground",
        entity: "tank",
        reads: "Tank::tread (surface, roll, wet), the water, lava and heat under it, the rubble and brush, Game::skidding, Game::drift_at, the sky and theme; the burn and boost timers ending in water",
        cues: &[
            "vehicle.ground.grass",
            "vehicle.ground.sand",
            "vehicle.ground.water",
            "vehicle.ground.ice",
            "vehicle.ground.lava",
            "vehicle.ground.mud",
            "vehicle.ground.snow",
            "vehicle.ground.dust",
            "vehicle.ground.puddle",
            "vehicle.ground.rubble",
            "vehicle.ground.brush",
            "vehicle.pivot",
            "vehicle.slip",
            "vehicle.slip.ice",
            "vehicle.slip.mud",
            "vehicle.coast.ice",
            "vehicle.skid",
            "vehicle.skid.grass",
            "vehicle.skid.sand",
            "vehicle.skid.ice",
            "vehicle.skid.water",
            "vehicle.skid.snow",
            "vehicle.skid.mud",
            "vehicle.splash",
            "vehicle.splash.out",
            "vehicle.drip",
            "world.lava.sizzle",
            "world.fire.douse",
            "pickup.boost.fizzle",
        ],
        online: "the same: positions, the tread, the rubble and the burn, boost and skid replicate",
    },
    WatcherSpec {
        id: "water",
        entity: "water",
        reads: "Game::water (depth, current, ice) gathered into clusters near the ears; Game::look's rain; burning wrecks in a ford or in rain",
        cues: &["world.water.lap", "world.water.stream", "world.water.ice", "world.rain.water", "world.fire.hiss"],
        online: "the same: the water is the map's",
    },
    WatcherSpec {
        id: "landing",
        entity: "grenade, bong",
        reads: "each grenade's height touching down, and what it lands on; a bong's hop ending (Bong::hop_timer) in water",
        cues: &["weapon.grenades.bounce", "weapon.grenades.bounce.water", "weapon.grenades.bounce.ice", "bong.land.water"],
        online: "the same: a grenade's height and the bong's hop replicate",
    },
    WatcherSpec { id: "place", entity: "map", reads: "the map's theme and biome", cues: &["bed.grass", "bed.desert", "bed.valley", "bed.floodplain", "bed.karst"], online: "the same" },
    WatcherSpec {
        id: "scrape",
        entity: "tank",
        reads: "Game::contacts_against: a hull held against a face, and what the face is",
        cues: &["vehicle.scrape", "vehicle.scrape.water"],
        online: "silent: a client steps no physics (a slam is `Event::HullStruck`, which is sent)",
    },
    WatcherSpec { id: "aircraft", entity: "aircraft", reads: "Game::aircraft", cues: &["air.heli", "air.jet"], online: "the same" },
    WatcherSpec { id: "flyby", entity: "shell, bullet, plasma bolt, missile", reads: "projectiles' closest approach to a seat's own tank", cues: &["flyby.shell", "flyby.bullet", "flyby.plasma", "flyby.missile"], online: "the same: projectiles replicate" },
    WatcherSpec {
        id: "ricochet",
        entity: "shell, bullet",
        reads: "Event::Ricochet's shot id, then that shot's position and velocity until it is gone",
        cues: &["hit.ricochet.whine"],
        online: "the same, from the sent Ricochet; a seat's own shot, which the replica does not hold, is flown on from where it turned",
    },
    WatcherSpec { id: "thunder", entity: "weather", reads: "weather::lightning on the round clock: a flash's rising edge", cues: &["world.thunder"], online: "the same: the clock is the round's" },
    WatcherSpec { id: "tesla", entity: "tesla tower", reads: "Game::tower_views charge", cues: &["world.tesla.charge"], online: "the same: the charge travels (`Snapshot::towers`)" },
    WatcherSpec { id: "fuse", entity: "barrel", reads: "Game::fused_barrels", cues: &["world.fuse"], online: "the same" },
    WatcherSpec { id: "portal", entity: "portal", reads: "Game::portals", cues: &["world.portal"], online: "the same" },
    WatcherSpec { id: "volcano", entity: "volcano", reads: "Volcano::phase on the round clock", cues: &["world.volcano"], online: "the same: the clock is the round's" },
    WatcherSpec {
        id: "crows",
        entity: "crow",
        reads: "the director's own fauna::Fauna (audio::life): a flock's first bird lifting off, its first flier's age, a flock all down; calls hashed on the round clock",
        cues: &["world.crow.flush", "world.crow.caw.far", "world.crow.caw"],
        online: "the same: the director steps its creatures on the replica, as the window draws them",
    },
    WatcherSpec {
        id: "ducks",
        entity: "mallard",
        reads: "the director's own fauna::Fauna (audio::life): a group running to take off, a duck's fright from a hull, a duck landing, Duck::dabbling; quacks hashed on the round clock; none on ice",
        cues: &["world.duck.takeoff", "world.duck.quack.wary", "world.duck.land", "world.duck.quack", "world.duck.dabble"],
        online: "the same: the director steps its creatures on the replica, as the window draws them",
    },
    WatcherSpec {
        id: "fireflies",
        entity: "firefly",
        reads: "the director's own fauna::Fauna (audio::life): the fireflies gathered into swarms, fireflies::shown and each one's glow",
        cues: &["world.firefly"],
        online: "the same: the sky and the creatures are the replica's",
    },
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
    /// What the ground watcher keeps about each hull (`fn grounds`).
    footing: HashMap<usize, GroundEar>,
    /// Where the listeners are this frame (`set_ears`): what the water
    /// beds are picked by.
    ears: Vec<Vec2>,
    /// The map's water gathered for its sounds (`fn waters`).
    water: Option<WaterMap>,
    /// Each grenade's fall, for its touchdowns (`fn landings`).
    grenades: HashMap<u32, GrenadeEar>,
    /// Whether each bong was mid-hop last frame, by its entity.
    hops: HashMap<u64, bool>,
    bullets_seen: HashSet<u32>,
    last_round: HashMap<usize, f64>,
    spinning: HashSet<usize>,
    /// Each tank's charge as last read (`ChargeEar`), by owner slot.
    charge_ears: HashMap<usize, ChargeEar>,
    /// What each tank's weapons were doing last frame (`ArmsEar`), by owner
    /// slot.
    arm_ears: HashMap<usize, ArmsEar>,
    /// When each warning that beeps on a schedule - a seat in a rail's
    /// lane, a seat's rail about to vent - beeps next, on the director's
    /// clock, by its key.
    beeps: HashMap<u64, f64>,
    /// Each grenade's fuse lamp last frame, by id.
    lamps: HashMap<u32, bool>,
    /// Each missile and each diving drone, followed by id.
    tracks: HashMap<u32, Track>,
    /// The wrecks each seat has dealt (`RoundStats::by_seat`), once read.
    kills: Option<[u32; crate::MAX_SEATS]>,
    /// The beams the steps since the last frame drew (`Watchers::beam`).
    beams: Vec<Beam>,
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
const PUDDLE: u64 = 30 << 56;
const LAVA_SIZZLE: u64 = 31 << 56;
const DRIP: u64 = 32 << 56;
const TURN: u64 = 33 << 56;
const SKID: u64 = 34 << 56;
const RUBBLE: u64 = 35 << 56;
const BRUSH: u64 = 36 << 56;
const WATER: u64 = 37 << 56;
const HISS: u64 = 38 << 56;
const RAIN_WATER: u64 = 39 << 56;
const MINIGUN: u64 = 4 << 56;
const FLAME: u64 = 5 << 56;
const LOCK_DRONE: u64 = 40 << 56;
const DIVE: u64 = 41 << 56;
const MOTOR: u64 = 42 << 56;
const OVERCHARGE: u64 = 43 << 56;
const UPLINK: u64 = 44 << 56;
const OFFLINE: u64 = 45 << 56;
const LANE_BEEP: u64 = 46 << 56;
const VENT_BEEP: u64 = 47 << 56;
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

/// A key from a hull's slot and the cue it plays: a hull whose ground
/// changes under it lets one loop go and starts the next, where a key of
/// its slot alone would keep the first cue playing.
fn cue_key(tag: u64, cue: &str, slot: usize) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for b in cue.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01B3);
    }
    tag | ((h & 0xFF_FFFF) << 24) | (slot as u64 & 0xFF_FFFF)
}

/// What the ground watcher keeps about a hull between frames.
#[derive(Clone, Copy, Debug, Default)]
struct GroundEar {
    /// In liquid water, burning, and under a speed boost, last frame.
    wading: bool,
    burning: bool,
    boosted: bool,
    /// When it last climbed out of a ford: its drip tail's start.
    left_water: Option<f64>,
    /// The pivot, slip or coast it last made and when that lets go
    /// (`audio_turn_hold_s`).
    turn: Option<(&'static str, f64)>,
    /// Its speed last frame and when that was, for a coast's slowing.
    speed: f32,
    at: f64,
}

/// A grenade's fall between frames: its last height, the highest it rose
/// since it last touched down, and whether it is coming down.
#[derive(Clone, Copy, Debug, Default)]
struct GrenadeEar {
    height: f32,
    peak: f32,
    falling: bool,
}

/// One stretch of the map's water with a sound of its own.
#[derive(Clone, Copy, Debug)]
struct WaterBed {
    key: u64,
    cue: &'static str,
    at: Vec2,
    gain: f32,
}

/// The map's water gathered for its sounds into clusters of
/// `WATER_CLUSTER` cells, the way fires are: the beds, and every cluster's
/// liquid cells, which the rain on the water is weighed by.
struct WaterMap {
    /// The round time it was gathered at.
    at: f32,
    beds: Vec<WaterBed>,
    pools: Vec<(Vec2, u32)>,
}

/// A side of a water cluster, in the water grid's cells.
const WATER_CLUSTER: i32 = 4;

/// How often (round seconds) the water is gathered again: a rod's crater
/// fills with rain mid-round (`WaterLayout::fill`).
const WATER_REGATHER_S: f32 = 5.0;

/// The liquid cells in earshot at which rain on the water plays at the
/// rain's own level.
const RAIN_WATER_FULL_CELLS: f32 = 24.0;

impl WaterMap {
    /// Every water cell of the map, in row-major order: a lake's or a
    /// ford's cell beside dry ground laps, a stream's cell (one the current
    /// runs down) babbles, a frozen cell beside dry ground hushes and
    /// creaks; each kind is summed per cluster.
    fn gather(game: &Game) -> WaterMap {
        let water = game.water();
        let mut buckets: BTreeMap<(i32, i32, u8), (Vec2, u32)> = BTreeMap::new();
        let mut pools: BTreeMap<(i32, i32), (Vec2, u32)> = BTreeMap::new();
        if !water.is_empty() {
            let tile = crate::GROUND_WORLD_TILE;
            let (w, h) = game.map.field_size();
            let (cols, rows) = ((w / tile).ceil() as i32 + 1, (h / tile).ceil() as i32 + 1);
            for r in 0..rows {
                for c in 0..cols {
                    let depth = water.depth_of_cell(c, r);
                    if depth == Depth::Dry {
                        continue;
                    }
                    let at = Vec2::new(c as f32 * tile, r as f32 * tile);
                    let cluster = (c.div_euclid(WATER_CLUSTER), r.div_euclid(WATER_CLUSTER));
                    if depth.is_wet() {
                        let pool = pools.entry(cluster).or_insert((Vec2::new(0.0, 0.0), 0));
                        pool.0 += at;
                        pool.1 += 1;
                    }
                    let shore = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dc, dr)| water.depth_of_cell(c + dc, r + dr) == Depth::Dry);
                    let kind = match depth {
                        Depth::Shallow if water.pushes_south(at) => 1,
                        Depth::Shallow | Depth::Deep if shore => 0,
                        Depth::Ice if shore => 2,
                        Depth::Dry | Depth::Shallow | Depth::Deep | Depth::Ice => continue,
                    };
                    let bucket = buckets.entry((cluster.0, cluster.1, kind)).or_insert((Vec2::new(0.0, 0.0), 0));
                    bucket.0 += at;
                    bucket.1 += 1;
                }
            }
        }
        let beds = buckets
            .into_iter()
            .map(|((bx, by, kind), (sum, n))| {
                let (cue, gain) = match kind {
                    0 => (ids::WATER_LAP.0, (0.3 + 0.08 * n as f32).min(1.0)),
                    1 => (ids::WATER_STREAM.0, (0.4 + 0.08 * n as f32).min(1.0)),
                    _ => (ids::WATER_ICE.0, (0.2 + 0.04 * n as f32).min(0.6)),
                };
                let key = WATER | ((kind as u64) << 48) | (((bx as u32 as u64) & 0xFF_FFFF) << 24) | ((by as u32 as u64) & 0xFF_FFFF);
                WaterBed { key, cue, at: sum * (1.0 / n as f32), gain }
            })
            .collect();
        let pools = pools.into_values().map(|(sum, n)| (sum * (1.0 / n as f32), n)).collect();
        WaterMap { at: game.time, beds, pools }
    }
}

/// Whether a burning wreck at `at` hisses rather than crackles: it stands
/// in a ford, or the rain falls on it.
fn wreck_hisses(game: &Game, at: Vec2) -> bool {
    game.water().depth_at(at).is_wet() || game.wear_look().sky == Sky::Rain
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
        self.grounds(game, own, now, t, wishes, plays);
        self.waters(game, t, wishes);
        self.landings(game, t, plays);
        self.minigun(game, own, now, wishes, plays);
        self.flames(game, own, wishes);
        self.charges(game, own, now, t, wishes, plays);
        self.arms(game, own, now, t, wishes, plays);
        self.warnings(game, own, now, t, wishes, plays);
        self.ordnance(game, t, wishes, plays);
        self.near_misses(game, own, t, plays);
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
                // A jump - a portal, a respawn - is not a speed, and the
                // ground's own drift under a live hull - a current, a gust
                // - is not its tracks'.
                let v = if v.length() > crate::PHYSICS_MAX_SPEED {
                    Vec2::new(0.0, 0.0)
                } else if tank.is_wreck() {
                    v
                } else {
                    v - game.drift_at(tank.position)
                };
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
            }
        }
        self.tanks.retain(|slot, _| live.contains(slot));
    }

    /// Where the listeners are this frame: what the water beds are picked
    /// by (`fn waters`).
    pub fn set_ears(&mut self, ears: impl Iterator<Item = Vec2>) {
        self.ears.clear();
        self.ears.extend(ears);
    }

    /// What the ground under each hull sounds like (docs/sound-system-prd.md
    /// section 4.4), from the speed `tanks_and_treads` measured: the
    /// surface's layer as the sky and theme have it (`source::ground_layer`)
    /// and a road's puddles in the rain; rubble crunching and brush
    /// rustling under the tracks; a pivot's grind, a slip's screech or
    /// slush and a coast's hiss on ice (`Tread::roll`), each held a moment
    /// (`audio_turn_hold_s`); a hull knocked off its tracks skidding by
    /// what it slides on; a splash wading into a ford, the water pouring
    /// off climbing out and the drips after it (`Tread::wet`); a fire it
    /// carried in hissing out, a speed boost the water takes fizzling; and
    /// lava's sizzle on a hull in it, faint on a hot bank, none under a
    /// heat shield. Everything it reads replicates, so a replica sounds the
    /// same.
    fn grounds(&mut self, game: &Game, own: &[usize], now: f64, t: &Tuning, wishes: &mut Vec<Wish>, plays: &mut Vec<Play>) {
        let look = game.wear_look();
        let water = game.water();
        let skids: HashMap<usize, Vec2> = game.skidding().into_iter().map(|(slot, at, _)| (slot, at)).collect();
        let others = super::dsp::db_to_gain(t.audio_engine_others_db);
        let mut live = HashSet::new();
        for tank in game.world.query::<&Tank>().iter() {
            let slot = tank.owner_slot();
            let Some(&ear) = self.tanks.get(&slot) else { continue };
            live.insert(slot);
            let at = tank.position;
            let alive = !tank.is_wreck();
            let depth = water.depth_at(at);
            let wading = alive && depth.is_wet();
            let burning = alive && tank.burn_timer > 0.0;
            let boosted = alive && tank.speed_boost_timer > 0.0;
            let velocity = ear.velocity;
            let speed = velocity.length();
            let foot = self.footing.entry(slot).or_insert(GroundEar { wading, burning, boosted, speed, at: now, ..GroundEar::default() });
            let mine = own.contains(&slot);
            let level = if mine { 1.0 } else { others };
            let mass = tank.mass_factor();

            if wading && !foot.wading {
                let pace = (speed / t.audio_splash_full_px_s).clamp(0.0, 1.0);
                let strength = (0.35 + 0.65 * pace) * mass.sqrt().clamp(0.8, 1.3);
                plays.push(Play::new(ids::SPLASH.0, At::Slot(slot)).strength(strength).pitch(source::engine_pitch(mass)).source(slot as u64));
                foot.left_water = None;
            } else if alive && !wading && foot.wading {
                let pace = (speed / t.audio_splash_full_px_s).clamp(0.0, 1.0);
                plays.push(Play::new(ids::SPLASH_OUT.0, At::Slot(slot)).strength(0.6 + 0.4 * pace).pitch(source::engine_pitch(mass)).source(slot as u64));
                foot.left_water = Some(now);
            }
            if wading && foot.burning && !burning {
                plays.push(Play::new(ids::DOUSE.0, At::Slot(slot)).source(slot as u64));
            }
            if wading && foot.boosted && !boosted {
                plays.push(Play::new(ids::BOOST_FIZZLE.0, At::Slot(slot)).pitch(1.3).source(slot as u64));
            }
            (foot.wading, foot.burning, foot.boosted) = (wading, burning, boosted);
            let span = (now - foot.at) as f32;
            let slowing = if span > 0.0 { (foot.speed - speed) / span } else { 0.0 };
            if span > 0.0 {
                (foot.speed, foot.at) = (speed, now);
            }
            if !alive {
                continue;
            }

            let tread = tank.tread;
            let in_lava = depth == Depth::Dry && game.lava().depth_at(at) != Depth::Dry;
            let roll = (speed / t.audio_treads_full_px_s).clamp(0.0, 1.0);
            let cell = crate::map::world_to_cell(at);
            let mut wish = |key: u64, cue: &'static str, gain: f32, pitch: f32| {
                wishes.push(Wish { key, cue, at: Some(at), velocity, gain: gain * level, pitch, own: mine });
            };
            if roll > 0.02 {
                if let Some(cue) = source::ground_layer(tread.surface, look, tread.wet, in_lava) {
                    wish(cue_key(GROUND, cue, slot), cue, roll, 1.0);
                }
                if look.sky == Sky::Rain && tread.surface == Surface::Road && !in_lava {
                    wish(PUDDLE | slot as u64, ids::PUDDLE.0, roll, 1.0);
                }
                let rubble = match game.rubble.level(cell) {
                    crate::chunks::RubbleLevel::Clear => 0.0,
                    crate::chunks::RubbleLevel::Light => 0.6,
                    crate::chunks::RubbleLevel::Heavy => 1.0,
                };
                if rubble > 0.0 {
                    wish(RUBBLE | slot as u64, ids::RUBBLE.0, roll * rubble, source::engine_pitch(mass));
                }
                if game.brush.contains_key(&cell) {
                    let standing = if game.grass_trampled.contains(&cell) { 0.35 } else { 1.0 };
                    wish(BRUSH | slot as u64, ids::BRUSH.0, roll * standing, 1.0);
                }
            }

            let heat = if in_lava { 1.0 } else { game.heat_at(at) };
            if heat > 0.25 && tank.takes_heat() {
                wish(LAVA_SIZZLE | slot as u64, ids::LAVA_SIZZLE.0, if in_lava { 1.0 } else { 0.3 * heat }, 1.0);
            }

            if let Some(left) = foot.left_water {
                let fade = 1.0 - (now - left) as f32 / t.audio_drip_seconds.max(0.1);
                if fade <= 0.0 || tread.wet <= 0.05 {
                    foot.left_water = None;
                } else {
                    wish(DRIP | slot as u64, ids::DRIP.0, tread.wet * fade, 1.0);
                }
            }

            let skid = skids.get(&slot).copied();
            let muddy = crate::wear::muddy(look, tread.surface, tread.wet);
            let slippery_start = tread.speeding_up && (tread.surface == Surface::Ice || look.sky == Sky::Rain);
            let turn = if skid.is_some() || tread.surface.is_water() || in_lava {
                None
            } else {
                match tread.roll {
                    Roll::Pivoting if slippery_start => Some(source::slip_cue(tread.surface, muddy)),
                    Roll::Pivoting => Some(ids::PIVOT.0),
                    Roll::Sliding => Some(source::slip_cue(tread.surface, muddy)),
                    Roll::Rolling if tread.surface == Surface::Ice && speed > 20.0 && slowing > t.audio_coast_decel_px_s2 => Some(ids::COAST_ICE.0),
                    Roll::Rolling => None,
                }
            };
            if let Some(cue) = turn {
                foot.turn = Some((cue, now + t.audio_turn_hold_s as f64));
            }
            let held = foot.turn;
            match held {
                Some((cue, until)) if now <= until => wish(cue_key(TURN, cue, slot), cue, 0.5 + 0.5 * roll, source::engine_pitch(mass)),
                Some(_) => foot.turn = None,
                None => {}
            }

            if let Some(skid_at) = skid
                && speed > 20.0
            {
                let lava = game.water().depth_at(skid_at) == Depth::Dry && game.lava().depth_at(skid_at) != Depth::Dry;
                let cue = source::skid_cue(game.surface_at(skid_at), look, tread.wet, lava);
                wish(cue_key(SKID, cue, slot), cue, (speed / t.audio_skid_full_px_s).clamp(0.2, 1.0), 1.0);
            }
        }
        self.footing.retain(|slot, _| live.contains(slot));
    }

    /// The water's own sounds (docs/sound-system-prd.md section 4.4): the
    /// beds `WaterMap` gathers - a shore lapping, a stream babbling, a
    /// frozen shore's hush and creak - the nearest `audio_water_beds` in
    /// earshot of the ears; rain on the water in earshot, weighed by how
    /// much of it there is; and a burning wreck in a ford or in the rain
    /// hissing in place of the fire's crackle (`wreck_hisses`).
    fn waters(&mut self, game: &Game, t: &Tuning, wishes: &mut Vec<Wish>) {
        let stale = self.water.as_ref().is_none_or(|w| game.time < w.at || game.time - w.at > WATER_REGATHER_S);
        if stale {
            self.water = Some(WaterMap::gather(game));
        }
        let still = Vec2::new(0.0, 0.0);
        if let Some(map) = &self.water
            && !self.ears.is_empty()
        {
            let near = |p: Vec2| self.ears.iter().map(|e| e.distance_to(p)).fold(f32::INFINITY, f32::min);
            let reach = t.audio_class_far_px[super::Class::Small.index()];
            let mut beds: Vec<(f32, &WaterBed)> = map.beds.iter().map(|b| (near(b.at), b)).filter(|(d, _)| *d < reach).collect();
            beds.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.key.cmp(&b.1.key)));
            for (_, bed) in beds.into_iter().take(t.audio_water_beds) {
                wishes.push(Wish { key: bed.key, cue: bed.cue, at: Some(bed.at), velocity: still, gain: bed.gain, pitch: 1.0, own: false });
            }
            let rain = game.look(t).rain;
            if rain > 0.01 {
                let earshot = t.audio_class_far_px[super::Class::Medium.index()];
                let cells: u32 = map.pools.iter().filter(|(p, _)| near(*p) < earshot).map(|(_, n)| n).sum();
                let gain = rain * (cells as f32 / RAIN_WATER_FULL_CELLS).min(1.0);
                if gain > 0.01 {
                    wishes.push(Wish { key: RAIN_WATER, cue: ids::RAIN_WATER.0, at: None, velocity: still, gain, pitch: 1.3, own: false });
                }
            }
        }
        for tank in game.world.query::<&Tank>().iter() {
            if tank.is_wreck() && !tank.is_dead() && wreck_hisses(game, tank.position) {
                let key = HISS | tank.owner_slot() as u64;
                wishes.push(Wish { key, cue: ids::FIRE_HISS.0, at: Some(tank.position), velocity: still, gain: 0.9, pitch: 0.6, own: false });
            }
        }
    }

    /// Things coming down: a grenade touching down - a metal clunk on dry
    /// ground, smaller each hop, a plop in water, a skitter on ice - found
    /// from its height, which falls and turns (`GrenadeEar`); and a bong's
    /// hop ending in water, a comic plop.
    pub(super) fn landings(&mut self, game: &Game, t: &Tuning, plays: &mut Vec<Play>) {
        // The height a launched grenade tops out at: its first landing's
        // full strength.
        let full = (t.grenade_launch_climb * t.grenade_launch_climb / (2.0 * t.grenade_gravity.max(1.0))).max(1.0);
        let water = game.water();
        let mut live = HashSet::new();
        for g in game.world.query::<&crate::grenade::Grenade>().iter() {
            live.insert(g.id);
            let h = g.height;
            let ear = self.grenades.entry(g.id).or_insert(GrenadeEar { height: h, peak: h, falling: false });
            if h < ear.height - 0.05 {
                ear.falling = true;
            }
            // Down on the ground, or already up again off a hop the frame
            // never saw it touch.
            if ear.falling && (h <= 0.0 || h > ear.height + 0.05) {
                let strength = libm::sqrtf(ear.peak / full).clamp(0.15, 1.0);
                let (cue, pitch) = match water.depth_at(g.position) {
                    Depth::Shallow | Depth::Deep => (ids::GRENADE_BOUNCE_WATER.0, 1.4),
                    Depth::Ice => (ids::GRENADE_BOUNCE_ICE.0, 1.3),
                    Depth::Dry => (ids::GRENADE_BOUNCE.0, 1.0 + 0.25 * (1.0 - strength)),
                };
                plays.push(Play::new(cue, At::Point(g.position)).strength(strength).pitch(pitch).source(g.id as u64));
                ear.falling = false;
                ear.peak = h;
            }
            ear.peak = ear.peak.max(h);
            ear.height = h;
        }
        self.grenades.retain(|id, _| live.contains(id));

        let mut bongs = HashSet::new();
        for (entity, bong) in game.world.query::<(hecs::Entity, &crate::bong::Bong)>().iter() {
            let key = entity.to_bits().get();
            bongs.insert(key);
            let hopping = bong.hop_timer > 0.0 && !bong.is_dead();
            let was = self.hops.insert(key, hopping).unwrap_or(hopping);
            if was && !hopping && !bong.is_dead() && water.depth_at(bong.position).is_wet() {
                plays.push(Play::new(ids::BONG_SPLASH.0, At::Point(bong.position)).pitch(source::bong_pitch(bong.kind)).source(key));
            }
        }
        self.hops.retain(|key, _| bongs.contains(key));
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
        // A wreck in a ford or in the rain hisses instead (`fn waters`).
        for (p, _) in game.burning_wrecks() {
            if !wreck_hisses(game, p) {
                add(p);
            }
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

    /// A shell, a round, a plasma bolt or a missile passing close by a
    /// seat's own tank whooshes once, as it starts to move away
    /// (`audio_flyby_px`), the louder the closer it came
    /// (`audio_flyby_edge_strength` at the edge). A seat's own shots never
    /// do, nor a shot first seen already that close - one leaving a tank
    /// beside the seat, or the seat's own on a replica, which names no
    /// shot's owner.
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
        for p in game.world.query::<&crate::plasma::Plasma>().iter() {
            if !own.contains(&p.owner.slot()) {
                shots.push((p.id, p.position, p.velocity, ids::FLYBY_PLASMA.0));
            }
        }
        // A missile's velocity is measured (`ordnance`): a replica's has
        // none of its own.
        for m in game.world.query::<&crate::missile::Missile>().iter() {
            if !own.contains(&m.owner.slot()) && !m.is_dead() {
                let velocity = self.tracks.get(&m.id).map_or(Vec2::new(0.0, 0.0), |track| track.velocity);
                shots.push((m.id, m.position, velocity, ids::FLYBY_MISSILE.0));
            }
        }
        for (id, at, velocity, cue) in shots {
            live.insert(id);
            let near = seats.iter().map(|s| s.distance_to(at)).fold(f32::INFINITY, f32::min);
            let pass = self.passes.entry(id).or_insert((near, near <= reach));
            if !pass.1 && pass.0 <= reach && near > pass.0 {
                pass.1 = true;
                let strength = flyby_strength(pass.0, t);
                plays.push(Play::new(cue, At::Point(at - velocity * crate::PHYSICS_FIXED_DT)).strength(strength).source(id as u64));
            }
            pass.0 = pass.0.min(near);
        }
        self.passes.retain(|id, _| live.contains(id));
    }

    /// The charge-and-hold weapons while their trigger is held
    /// (`Tank::charge`). A rail whines, rising as it fills; pings once full
    /// (`Charge::stage`); crackles once overcharged; and, a seat's own,
    /// beeps to be let go in its last `audio_vent_warn_s` before it vents
    /// (`Charge::vent_in`). A rod holds its uplink's carrier tone while its
    /// reticle is up, and a seat's own ticks for each cell its reticle
    /// steps.
    fn charges(&mut self, game: &Game, own: &[usize], now: f64, t: &Tuning, wishes: &mut Vec<Wish>, plays: &mut Vec<Play>) {
        use crate::tank::{ActiveWeapon, ChargeStage};
        let still = Vec2::new(0.0, 0.0);
        let mut live = HashSet::new();
        for tank in game.world.query::<&Tank>().iter() {
            let Some(charge) = tank.charge.filter(|_| !tank.is_wreck()) else {
                continue;
            };
            let slot = tank.owner_slot();
            let mine = own.contains(&slot);
            live.insert(slot);
            let reticle = tank.reticle.map(|r| r.cell).filter(|_| charge.weapon == ActiveWeapon::RodFromGod);
            let now_ear = ChargeEar { weapon: charge.weapon, stage: charge.stage(), reticle };
            let was = self.charge_ears.insert(slot, now_ear).filter(|was| was.weapon == charge.weapon);
            if charge.weapon == ActiveWeapon::RodFromGod {
                wishes.push(Wish { key: UPLINK | slot as u64, cue: ids::ROD_UPLINK.0, at: Some(tank.position), velocity: still, gain: 0.8, pitch: 1.0, own: mine });
                if mine && reticle.is_some() && was.is_some_and(|was| was.reticle.is_some() && was.reticle != reticle) {
                    plays.push(Play::new(ids::ROD_RETICLE.0, At::Slot(slot)).source(slot as u64));
                }
                continue;
            }
            let progress = charge.progress();
            wishes.push(Wish { key: CHARGE | slot as u64, cue: ids::CHARGE_GAUSS.0, at: Some(tank.position), velocity: still, gain: 0.6 + 0.4 * progress, pitch: 0.8 + 0.8 * progress, own: mine });
            if now_ear.stage == ChargeStage::Full && was.is_some_and(|was| was.stage == ChargeStage::Charging) {
                plays.push(Play::new(ids::GAUSS_READY.0, At::Slot(slot)).source(slot as u64));
            }
            if now_ear.stage == ChargeStage::Overcharged {
                wishes.push(Wish { key: OVERCHARGE | slot as u64, cue: ids::GAUSS_OVERCHARGED.0, at: Some(tank.position), velocity: still, gain: 1.0, pitch: 1.0, own: mine });
            }
            let key = VENT_BEEP | slot as u64;
            let vent = charge.vent_in();
            if mine && vent > 0.0 && vent <= t.audio_vent_warn_s {
                if due(&mut self.beeps, key, now, t.audio_vent_beep_s) {
                    plays.push(Play::new(ids::VENT_ALARM.0, At::Slot(slot)).source(slot as u64));
                }
            } else {
                self.beeps.remove(&key);
            }
        }
        self.charge_ears.retain(|slot, _| live.contains(slot));
    }

    /// What a weapon does between its shots. Any tank's: a minigun's
    /// barrels spinning up from rest; a flamethrower lighting as its jet
    /// starts and going out as it stops - its valve shut, or sputtering
    /// dry. A seat's own: static while its special is offline
    /// (`Tank::special_down`) and a chime when it is back; an empty clunk
    /// when its special is spent, then the cannon re-arming
    /// (`audio_rearm_s`); the magazine racking a shell as it gets back to a
    /// shot's cost and a firmer clunk once full; the missile pod refilling
    /// tube by tube and a tick once it is full.
    fn arms(&mut self, game: &Game, own: &[usize], now: f64, t: &Tuning, wishes: &mut Vec<Wish>, plays: &mut Vec<Play>) {
        use crate::tank::ActiveWeapon;
        let jets: HashSet<usize> = game.flames().iter().map(|jet| jet.owner.slot()).collect();
        let mut live = HashSet::new();
        for tank in game.world.query::<&Tank>().iter() {
            if tank.is_wreck() {
                continue;
            }
            let slot = tank.owner_slot();
            let mine = own.contains(&slot);
            live.insert(slot);
            let down = tank.special_down();
            let was = self.arm_ears.get(&slot).copied();
            let ear = ArmsEar {
                spinning: tank.minigun_spin > 0.0 || tank.minigun_burst.is_some() || tank.minigun_show > 0.0,
                jetting: jets.contains(&slot),
                down,
                down_at: if down { now } else { was.map_or(f64::NEG_INFINITY, |was| was.down_at) },
                special: tank.special(),
                shells: tank.shells_ammo,
                tubes: tank.missile_tubes_empty,
            };
            self.arm_ears.insert(slot, ear);
            if mine && down {
                wishes.push(Wish { key: OFFLINE | slot as u64, cue: ids::OFFLINE.0, at: Some(tank.position), velocity: Vec2::new(0.0, 0.0), gain: 0.7, pitch: 1.0, own: true });
            }
            let Some(was) = was else {
                continue;
            };
            let (at, source) = (At::Slot(slot), slot as u64);
            if ear.spinning && !was.spinning {
                plays.push(Play::new(ids::MINIGUN_SPIN_UP.0, at).source(source));
            }
            if ear.jetting && !was.jetting {
                plays.push(Play::new(ids::FLAME_START.0, at).source(source));
            } else if was.jetting && !ear.jetting {
                let dry = tank.flame_fuel <= 0.0 && !down;
                plays.push(Play::new(if dry { ids::FLAME_DRY.0 } else { ids::FLAME_STOP.0 }, at).source(source));
            }
            if !mine {
                continue;
            }
            // Back: a special that fires again, offline a moment ago - not
            // one a crate just brought.
            if ear.usable() && !was.usable() && now - was.down_at < BACK_ONLINE_S {
                plays.push(Play::new(ids::ONLINE.0, at).source(source));
            }
            // Spent: gone while it fired, not hidden by an outage (a
            // replica's wire names no special while it is offline).
            if was.usable() && ear.special.is_none() && !down {
                plays.push(Play::new(ids::SPENT.0, at).source(source));
                plays.push(Play::new(ids::SHELL_RELOAD.0, at).after(t.audio_rearm_s).source(source));
            }
            let cost = if tank.twin_gun() { 2 } else { 1 };
            if ear.shells == was.shells + 1 && tank.active_weapon() == ActiveWeapon::Shell {
                if ear.shells >= t.max_shells {
                    plays.push(Play::new(ids::SHELL_FULL.0, at).pitch(0.8).source(source));
                } else if was.shells < cost && ear.shells >= cost {
                    plays.push(Play::new(ids::SHELL_RELOAD.0, at).source(source));
                }
            }
            // The pod refills between volleys (`Tank::tick_missile_pod`);
            // a volley's next salvo empties it again at once.
            if ear.tubes < was.tubes && tank.missile_volley.is_none() && tank.missile_ammo > 0 {
                for i in 0..(was.tubes - ear.tubes) {
                    plays.push(Play::new(ids::MISSILE_RELOAD.0, at).pitch(1.15).after(0.08 * i as f32).source(source));
                }
                if ear.tubes == 0 {
                    plays.push(Play::new(ids::MISSILE_READY.0, at).after(0.1).source(source));
                }
            }
        }
        self.arm_ears.retain(|slot, _| live.contains(slot));
    }

    /// The warnings meant for a seat this window plays. Lock beeps while it
    /// stands in another tank's charging rail lane (`Game::rail_lanes`),
    /// quicker as that charge fills (`audio_lane_beep_slow_s` to
    /// `audio_lane_beep_fast_s`); a thin tone while a drone's lock is on it,
    /// rising as the nearest such drone closes in
    /// (`audio_drone_lock_far_px`) and gone with the lock; and a sting when
    /// it wrecks an enemy (`RoundStats::by_seat`, the count
    /// `feedback.rs`'s hold on a kill reads).
    fn warnings(&mut self, game: &Game, own: &[usize], now: f64, t: &Tuning, wishes: &mut Vec<Wish>, plays: &mut Vec<Play>) {
        let seats: Vec<(usize, Vec2)> =
            game.world.query::<&Tank>().iter().filter(|tk| own.contains(&tk.owner_slot()) && !tk.is_wreck()).map(|tk| (tk.owner_slot(), tk.position)).collect();
        let lanes = if seats.is_empty() || !game.any_rail_charging() { Vec::new() } else { game.rail_lanes() };
        let fill: HashMap<usize, f32> =
            if lanes.is_empty() { HashMap::new() } else { game.world.query::<&Tank>().iter().filter_map(|tk| tk.charge.map(|c| (tk.owner_slot(), c.progress()))).collect() };
        for &(slot, at) in &seats {
            let key = LANE_BEEP | slot as u64;
            let threat = lanes.iter().filter(|l| l.slot != slot && l.depth(at) >= 0.0).map(|l| fill.get(&l.slot).copied().unwrap_or(0.0)).reduce(f32::max);
            let Some(p) = threat else {
                self.beeps.remove(&key);
                continue;
            };
            let every = t.audio_lane_beep_slow_s + (t.audio_lane_beep_fast_s - t.audio_lane_beep_slow_s) * p;
            if due(&mut self.beeps, key, now, every) {
                plays.push(Play::new(ids::LOCK_RAIL.0, At::Slot(slot)).pitch(1.0 + 0.25 * p).source(slot as u64));
            }
        }
        let drones = if seats.is_empty() { Vec::new() } else { game.drones() };
        for &(slot, at) in &seats {
            let near = drones.iter().filter(|d| d.in_air() && d.lock.slot() == Some(slot)).map(|d| d.ground.distance_to(at)).fold(f32::INFINITY, f32::min);
            if near.is_finite() {
                let close = 1.0 - (near / t.audio_drone_lock_far_px.max(1.0)).clamp(0.0, 1.0);
                wishes.push(Wish { key: LOCK_DRONE | slot as u64, cue: ids::LOCK_DRONE.0, at: Some(at), velocity: Vec2::new(0.0, 0.0), gain: 0.6 + 0.4 * close, pitch: 0.8 + 0.8 * close, own: true });
            }
        }
        let by_seat = game.round_stats().by_seat;
        if let Some(before) = self.kills {
            for &slot in own {
                if by_seat.get(slot).zip(before.get(slot)).is_some_and(|(now, before)| now > before) {
                    plays.push(Play::new(ids::KILL.0, At::Nowhere).source(slot as u64));
                }
            }
        }
        self.kills = Some(by_seat);
    }

    /// What flies on its own. Each missile is heard leaving its tube - one
    /// launch a missile, so a volley is heard tube by tube at its own
    /// spacing - and its motor while it flies, pitched by its measured
    /// speed (`audio_missile_motor_px_s`) and gone when an EMP kills it.
    /// Each drone's dive rises to a scream (`audio_dive_rise_s`). Each
    /// grenade's fuse beeps with its lamp, quicker as it nears going off
    /// (`Grenade::lamp_lit_at`).
    fn ordnance(&mut self, game: &Game, t: &Tuning, wishes: &mut Vec<Wish>, plays: &mut Vec<Play>) {
        let frame = game.frame();
        let mut live = HashSet::new();
        for m in game.world.query::<&crate::missile::Missile>().iter() {
            live.insert(m.id);
            let track = match self.tracks.get_mut(&m.id) {
                Some(track) => {
                    track.follow(m.position, frame);
                    *track
                }
                None => {
                    plays.push(Play::new(ids::MISSILE_LAUNCH.0, At::Point(m.position)).source(m.id as u64));
                    *self.tracks.entry(m.id).or_insert(Track::new(m.position, frame))
                }
            };
            if m.is_dead() {
                continue;
            }
            let pitch = 0.85 + 0.35 * (track.velocity.length() / t.audio_missile_motor_px_s.max(1.0)).clamp(0.0, 1.0);
            wishes.push(Wish { key: MOTOR | m.id as u64, cue: ids::MISSILE_MOTOR.0, at: Some(m.position), velocity: track.velocity, gain: 0.8, pitch, own: false });
        }
        for d in game.world.query::<&crate::fpv::Drone>().iter() {
            if d.stage != crate::fpv::DroneStage::Dive || d.landed {
                continue;
            }
            live.insert(d.id);
            let track = self.tracks.entry(d.id).or_insert(Track::new(d.ground, frame));
            track.follow(d.ground, frame);
            let rise = (d.stage_time / t.audio_dive_rise_s.max(0.01)).clamp(0.0, 1.0);
            wishes.push(Wish { key: DIVE | d.id as u64, cue: ids::FPV_DIVE.0, at: Some(d.ground), velocity: track.velocity, gain: 0.6 + 0.4 * rise, pitch: 1.0 + 0.5 * rise, own: false });
        }
        self.tracks.retain(|id, _| live.contains(id));
        let mut lamps = HashSet::new();
        for g in game.world.query::<&crate::grenade::Grenade>().iter() {
            lamps.insert(g.id);
            let lit = crate::grenade::Grenade::lamp_lit_at(t.grenade_fuse_seconds - g.fuse, t.grenade_fuse_seconds, t);
            if self.lamps.insert(g.id, lit).is_some_and(|was| lit && !was) {
                plays.push(Play::new(ids::GRENADE_BEEP.0, At::Point(g.position)).pitch(1.5).source(g.id as u64));
            }
        }
        self.lamps.retain(|id, _| lamps.contains(id));
    }

    /// A rail slug or a laser beam passing close by a seat's own tank
    /// without striking it cracks or zaps past, heard where it came
    /// nearest - within `audio_flyby_px`, the louder the closer, as a
    /// fly-by is; the seat that fired it hears none of its own.
    fn near_misses(&mut self, game: &Game, own: &[usize], t: &Tuning, plays: &mut Vec<Play>) {
        if self.beams.is_empty() {
            return;
        }
        let seats: Vec<(usize, Vec2)> =
            game.world.query::<&Tank>().iter().filter(|tk| own.contains(&tk.owner_slot()) && !tk.is_wreck()).map(|tk| (tk.owner_slot(), tk.position)).collect();
        for (i, beam) in std::mem::take(&mut self.beams).into_iter().enumerate() {
            for &(slot, at) in &seats {
                if beam.shooter == Some(slot) || beam.struck.contains(&slot) {
                    continue;
                }
                let (near, d) = nearest_on(beam.from, beam.to, at);
                if d <= t.audio_flyby_px {
                    plays.push(Play::new(beam.cue, At::Point(near)).strength(flyby_strength(d, t)).source(((i as u64) << 8) | slot as u64));
                }
            }
        }
    }

    /// A beam a step drew (`Beam::of`), for the next frame's near misses.
    pub fn beam(&mut self, beam: Beam) {
        self.beams.push(beam);
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

/// What the charge watcher keeps about a tank's charge between frames.
#[derive(Clone, Copy, Debug)]
struct ChargeEar {
    weapon: crate::tank::ActiveWeapon,
    stage: crate::tank::ChargeStage,
    /// A rod's reticle's cell.
    reticle: Option<(i32, i32)>,
}

/// What the weapon watcher keeps about a tank between frames.
#[derive(Clone, Copy, Debug)]
struct ArmsEar {
    /// Its minigun running: the barrels turning, or a burst on.
    spinning: bool,
    /// Its flamethrower's jet on.
    jetting: bool,
    /// Its special offline (`Tank::special_down`).
    down: bool,
    /// When it was last offline, on the director's clock.
    down_at: f64,
    special: Option<crate::tank::ActiveWeapon>,
    shells: i32,
    tubes: u8,
}

impl ArmsEar {
    /// Carrying a special that fires.
    fn usable(&self) -> bool {
        self.special.is_some() && !self.down
    }
}

/// How soon after an outage (s) a special that fires again is heard back
/// online, rather than as one a crate brought: a replica's wire names the
/// special again a snapshot after its outage runs out.
const BACK_ONLINE_S: f64 = 1.0;

/// Faster than this (px/s) between two frames, a followed projectile
/// jumped (a portal) rather than flew.
const JUMP_PX_S: f32 = 2000.0;

/// A projectile followed from frame to frame by where it is, so its
/// velocity is measured: a replica's as well as a round's.
#[derive(Clone, Copy, Debug)]
struct Track {
    at: Vec2,
    frame: u64,
    velocity: Vec2,
}

impl Track {
    fn new(at: Vec2, frame: u64) -> Track {
        Track { at, frame, velocity: Vec2::new(0.0, 0.0) }
    }

    /// On to `at` on `frame`: the velocity over the steps between.
    fn follow(&mut self, at: Vec2, frame: u64) {
        let steps = frame.saturating_sub(self.frame);
        if steps == 0 {
            return;
        }
        let v = (at - self.at) * (1.0 / (steps as f32 * crate::PHYSICS_FIXED_DT));
        self.velocity = if v.length() > JUMP_PX_S { Vec2::new(0.0, 0.0) } else { v };
        self.at = at;
        self.frame = frame;
    }
}

/// A beam a step drew, for the near misses (`Watchers::near_misses`).
#[derive(Clone, Debug, PartialEq)]
pub struct Beam {
    pub from: Vec2,
    pub to: Vec2,
    /// The owner slot that fired it, where the event names it: a rail
    /// slug's always, a laser's when a seat fired it.
    pub shooter: Option<usize>,
    /// What it sounds like going by.
    pub cue: &'static str,
    /// The seats its step's hits of its kind struck.
    pub struck: Vec<usize>,
}

impl Beam {
    /// The beam `event` drew, if it is one - a rail slug's leg or a laser
    /// beam's - with the seats `events`, its step's, have its kind
    /// striking.
    pub fn of(event: &crate::simulation::Event, events: &[crate::simulation::Event]) -> Option<Beam> {
        use crate::simulation::{Event, HitCause, HitTarget};
        let (from, to, shooter, cue, kind) = match *event {
            Event::RailSlug { slot, x0, y0, x1, y1, .. } => (Vec2::new(x0, y0), Vec2::new(x1, y1), Some(slot), ids::FLYBY_RAIL.0, HitCause::Rail),
            Event::LaserBeam { x0, y0, x1, y1, seat, .. } => {
                (Vec2::new(x0, y0), Vec2::new(x1, y1), (seat != crate::net::wire::NO_SEAT).then_some(seat as usize), ids::FLYBY_LASER.0, HitCause::Laser)
            }
            _ => return None,
        };
        let struck = events
            .iter()
            .filter_map(|e| match *e {
                Event::Hit { target: HitTarget::Player { player }, cause, .. } if cause == kind => Some(player as usize),
                _ => None,
            })
            .collect();
        Some(Beam { from, to, shooter, cue, struck })
    }
}

/// Whether the warning keyed `key` beeps now, every `every` seconds from
/// its first: a beep moves its next on.
fn due(beeps: &mut HashMap<u64, f64>, key: u64, now: f64, every: f32) -> bool {
    let next = beeps.entry(key).or_insert(now);
    if now < *next {
        return false;
    }
    *next = now + every.max(0.02) as f64;
    true
}

/// A fly-by's or a near miss's strength for a pass `d` px from the seat's
/// tank: full straight over it, `audio_flyby_edge_strength` at
/// `audio_flyby_px`.
fn flyby_strength(d: f32, t: &Tuning) -> f32 {
    let close = 1.0 - (d / t.audio_flyby_px.max(1.0)).clamp(0.0, 1.0);
    t.audio_flyby_edge_strength + (1.0 - t.audio_flyby_edge_strength) * close
}

/// The point of the segment `a..b` nearest `p`, and how far it is from `p`.
fn nearest_on(a: Vec2, b: Vec2, p: Vec2) -> (Vec2, f32) {
    let ab = b - a;
    let ap = p - a;
    let len2 = ab.x * ab.x + ab.y * ab.y;
    let s = if len2 > 1e-6 { ((ap.x * ab.x + ap.y * ab.y) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let q = a + ab * s;
    (q, q.distance_to(p))
}
