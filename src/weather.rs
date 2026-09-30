//! The sky over the battlefield (docs/weather.md), its headless half: the
//! sky a round is fought under (`in_force`, `random_sky`), what it does to
//! the rules (`sight_factor`, `grip_factor`, `freezes`, `gusts`/`gust_at` -
//! the one part of this module the simulation reads), what each
//! `map::Weather` looks like (`Look`), which of the renderer's stages a
//! frame needs (`Plan`), when lightning strikes (`lightning`), and every
//! light the round throws into the dark (`lights`), with the walls' shadows
//! cast by a raycast over the tile grid (`Occluders`). `render/weather.rs`
//! is the raylib half: the light map, the ground and the sky passes.
//!
//! Nothing here writes to a `Game` or draws the round's RNG: the sky is
//! the map's key (or the override knob's), a `random` one a hash of the
//! round's seed, settled once by `Game::init`, and every rule and every
//! picture is a function of it, the knobs, the round clock and hashed
//! positions. So a seeded replay is the same replay
//! under its sky, a clear sky plays exactly as a round with no weather,
//! a room's replicas draw and predict under the room's sky, a paused frame
//! and a re-render draw the same picture, and a dev-server lockstep
//! freezes the rain with the round.

use crate::blast::{seed_for, BlastKind};
use crate::bullet::{Bullet, BulletState};
use crate::frog::Side;
use crate::fx::{Impact, ImpactKind};
use crate::laser::LaserVariant;
use crate::map::Weather;
use crate::math::{Color, Vec2};
use crate::missile::Missile;
use crate::obstacle::Obstacle;
use crate::pickup::{Pickup, PickupKind};
use crate::plasma::{Plasma, PlasmaState, PlasmaVariant};
use crate::shell::{Shell, ShellState};
use crate::simulation::Game;
use crate::tank::Tank;
use crate::tower::TowerKind;
use crate::tuning::Tuning;
use crate::{OBSTACLE_GRID_SIZE, Position};

/// A colour as the light map adds it up: linear channels, 1.0 the
/// brightness of daylight, allowed above it.
pub type Rgb = [f32; 3];

/// The night's colour at `night_ambient` 1: moonlight is blue, and a dark
/// field reads as night rather than as a dimmed day because of it.
const NIGHT_TINT: Rgb = [0.75, 0.9, 1.45];

/// A storm's gloom, at `night_ambient` 1 - greyer than the night's.
const STORM_TINT: Rgb = [0.9, 0.97, 1.2];

/// How far past a cone's half angle its soft edge runs, as a factor of
/// that angle.
const CONE_EDGE: f32 = 1.3;

/// A tesla tower's violet, its coil and its bolts (`render/tower.rs`'s
/// `BOLT_GLOW`).
const TESLA_LIGHT: Rgb = [0.69, 0.47, 1.0];

/// The bio slush's lime ooze (`tower::OOZE_LT`).
const OOZE_LIGHT: Rgb = [0.78, 1.0, 0.3];

/// A weather as numbers: the light the field is lit by and how much of
/// each layer the sky carries. `Look::of` resolves one from its weather
/// and the `weather` tuning group; the renderer reads nothing else.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// The light every pixel of the field is multiplied by before any lamp
    /// adds to it: `[1, 1, 1]` is daylight, a red channel over the blue
    /// warms the picture (dusk), and a channel above 1 brightens it past
    /// daylight (heat haze).
    pub ambient: Rgb,
    /// How strongly the round's lights show, 0 to 1: full at night,
    /// faint under a grey day sky, none in the sun.
    pub lights: f32,
    /// The low sun of dusk: warm light rising toward the field's west edge.
    pub sun: f32,
    /// How much the field darkens toward its edges under this sky.
    pub vignette: f32,
    /// How hard it rains, 0 to 1 (more than 1 with `rain_density`).
    pub rain: f32,
    /// How bright a lightning strike is, 0 for a sky without them.
    pub lightning: f32,
    /// How thick the fog is.
    pub fog: f32,
    /// How thick the blowing sand is.
    pub sand: f32,
    /// How hard it snows; the ground is covered in proportion.
    pub snow: f32,
    /// How strongly the air shimmers.
    pub haze: f32,
}

impl Look {
    /// A clear day: nothing to draw.
    pub const CLEAR: Look = Look {
        ambient: [1.0, 1.0, 1.0],
        lights: 0.0,
        sun: 0.0,
        vignette: 0.0,
        rain: 0.0,
        lightning: 0.0,
        fog: 0.0,
        sand: 0.0,
        snow: 0.0,
        haze: 0.0,
    };

    /// `weather` as designed, then scaled by the `weather` group: the
    /// per-layer amounts, `night_ambient`, and `weather_strength`, which
    /// eases the light toward daylight and thins every layer together.
    pub fn of(weather: Weather, t: &Tuning) -> Look {
        let tint = |tint: Rgb, k: f32| [tint[0] * k, tint[1] * k, tint[2] * k];
        let designed = match weather {
            Weather::Clear => Look::CLEAR,
            Weather::Night => Look { ambient: tint(NIGHT_TINT, t.night_ambient), lights: 1.0, vignette: 0.35, ..Look::CLEAR },
            Weather::Dusk => Look { ambient: [0.8, 0.62, 0.54], lights: 0.6, sun: 1.0, vignette: 0.2, ..Look::CLEAR },
            Weather::Rain => Look { ambient: [0.76, 0.8, 0.9], lights: 0.4, vignette: 0.15, rain: 0.7, ..Look::CLEAR },
            Weather::Storm => Look {
                ambient: tint(STORM_TINT, (t.night_ambient * 2.1).min(1.0)),
                lights: 1.0,
                vignette: 0.35,
                rain: 1.0,
                lightning: 1.0,
                ..Look::CLEAR
            },
            Weather::Fog => Look { ambient: [0.93, 0.95, 0.99], lights: 0.25, fog: 0.85, ..Look::CLEAR },
            Weather::Sandstorm => Look { ambient: [1.0, 0.87, 0.7], lights: 0.35, vignette: 0.25, sand: 0.9, ..Look::CLEAR },
            Weather::Snow => Look { ambient: [0.98, 1.0, 1.06], lights: 0.15, vignette: 0.05, snow: 0.85, ..Look::CLEAR },
            Weather::HeatHaze => Look { ambient: [1.08, 1.0, 0.86], haze: 1.0, ..Look::CLEAR },
            // Not a sky of its own: `in_force` puts one of `SKIES` in its
            // place before a look is asked for.
            Weather::Random => Look::CLEAR,
        };
        let s = t.weather_strength.clamp(0.0, 1.0);
        Look {
            ambient: designed.ambient.map(|a| 1.0 + (a - 1.0) * s),
            lights: designed.lights * s,
            sun: designed.sun * s,
            vignette: designed.vignette * t.weather_vignette * s,
            rain: designed.rain * t.rain_density * s,
            lightning: designed.lightning * t.lightning_strength * s,
            fog: designed.fog * t.fog_density * s,
            sand: designed.sand * t.sand_density * s,
            snow: designed.snow * t.snow_density * s,
            haze: designed.haze * s,
        }
    }

    /// What the renderer has to run for this look; `None` is a clear
    /// frame, drawn exactly as a round without weather.
    pub fn plan(&self) -> Option<Plan> {
        const EPS: f32 = 0.004;
        let lit = self.ambient.iter().any(|a| (a - 1.0).abs() > EPS) || self.lights > EPS || self.sun > EPS || self.lightning > EPS || self.vignette > EPS;
        let ground = self.rain > EPS || self.snow > EPS;
        let sky = self.rain > EPS || self.fog > EPS || self.sand > EPS || self.snow > EPS || self.haze > EPS || self.lightning > EPS;
        (lit || ground || sky).then_some(Plan { lit, ground, sky })
    }

    /// How dark the ambient is, 0 in daylight to 1 in pitch black: what the
    /// light pass desaturates the unlit field by.
    pub fn darkness(&self) -> f32 {
        let lum = 0.299 * self.ambient[0] + 0.587 * self.ambient[1] + 0.114 * self.ambient[2];
        (1.0 - lum).clamp(0.0, 1.0)
    }
}

/// Which of the renderer's weather stages a frame runs (docs/weather.md):
/// the light pass multiplies the field by the light map, the ground pass
/// puts snow, ice, puddles and ripples on the floor under everything that
/// stands, the sky pass draws the air over everything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub lit: bool,
    pub ground: bool,
    pub sky: bool,
}

impl Game {
    /// The sky this round is fought and drawn under, settled by `init`
    /// (`in_force`): never `Random`.
    pub fn weather(&self) -> Weather {
        self.weather
    }
}

/// The sky a round of `map` seeded `seed` is fought under: the
/// `weather_override` knob's weather when it names one - unless
/// `map_only`, a room's round, which is its map's alone - else the map's,
/// and in place of `Random` the seed's `random_sky`. Never `Random`
/// itself. `Game::init` asks it once per round.
pub fn in_force(map: Weather, seed: u64, map_only: bool, t: &Tuning) -> Weather {
    let knob = if map_only { None } else { knob(t) };
    match knob.unwrap_or(map) {
        Weather::Random => random_sky(seed),
        sky => sky,
    }
}

/// The weather the `weather_override` knob names, `Random` included;
/// `None` while it follows each map (-1).
pub fn knob(t: &Tuning) -> Option<Weather> {
    usize::try_from(t.weather_override).ok().and_then(|i| Weather::ALL.get(i).copied())
}

/// The sky a `random` weather is in the round seeded `seed`: one of
/// `Weather::SKIES`, every one as likely. A hash of the seed rather than a
/// draw from the round's RNG, so the round's stream is untouched - a
/// seeded replay, the probe fixtures and a room's round are the same
/// under it - and the same seed always brings the same sky: `--seed`, a
/// `restart {seed}` and a room's `Welcome`, whose seed every replica is
/// initialised on. An unpinned round's seed is drawn fresh, so is its sky.
pub fn random_sky(seed: u64) -> Weather {
    let h = mix64(seed.wrapping_add(0x5EA7_4E12).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    Weather::SKIES[(h % Weather::SKIES.len() as u64) as usize]
}

/// How far an enemy sees under `sky`, as a multiple of
/// `enemy_view_range` (docs/weather.md "The rules"): shorter at night, in
/// a storm and in fog, 1 under every other sky and with `weather_rules`
/// off. `Game::enemy_sight` is the range in force.
pub fn sight_factor(sky: Weather, t: &Tuning) -> f32 {
    if !t.weather_rules {
        return 1.0;
    }
    match sky {
        Weather::Night | Weather::Storm => t.night_sight_factor,
        Weather::Fog => t.fog_sight_factor,
        _ => 1.0,
    }
}

/// The fraction of its grip a hull keeps on the wet ground of a rainy
/// sky (rain, storm); 1 on a dry one.
pub fn grip_factor(sky: Weather, t: &Tuning) -> f32 {
    if t.weather_rules && matches!(sky, Weather::Rain | Weather::Storm) { t.rain_grip_factor } else { 1.0 }
}

/// Whether a round under `sky` has its water frozen over: a snowy sky's,
/// read once by `Game::init` (`ground::WaterLayout::freeze`).
pub fn freezes(sky: Weather, t: &Tuning) -> bool {
    t.weather_rules && sky == Weather::Snow
}

/// One of a sandstorm's gusts (`gusts`): the round time its front leaves
/// the field's corner nearest the wind, and the way it blows (a unit
/// vector, east swung by up to `sand_gust_spread_deg`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gust {
    pub start: f32,
    pub dir: Vec2,
}

impl Gust {
    /// How hard this gust blows at `pos` at round time `time`, 0 to 1. Its
    /// front crosses the field along `dir` at `sand_gust_front_speed`;
    /// behind it the wind rises fast and dies away over
    /// `sand_gust_seconds`. The sky shader draws the same band.
    pub fn strength_at(&self, pos: Position, time: f32, t: &Tuning) -> f32 {
        let along = pos.x * self.dir.x + pos.y * self.dir.y;
        let age = (time - self.start - along / t.sand_gust_front_speed.max(1.0)) / t.sand_gust_seconds.max(0.05);
        if !(0.0..1.0).contains(&age) {
            return 0.0;
        }
        smoothstep(0.0, 0.2, age) * (1.0 - smoothstep(0.35, 1.0, age))
    }
}

/// The gusts that may be blowing at round time `time`: one in most
/// `sand_gust_gap_seconds` windows, somewhere in its first half, none in
/// the round's first window; the windows either side of this one are
/// included because a front takes a while to cross. A pure function of
/// the clock like `lightning`, so the room, every replica and every
/// sandbox blow alike.
pub fn gusts(time: f32, t: &Tuning) -> impl Iterator<Item = Gust> {
    let gap = t.sand_gust_gap_seconds.max(1.0);
    let window = (time / gap).floor() as i64;
    let spread = t.sand_gust_spread_deg.to_radians();
    (window - 1..=window + 1).filter_map(move |w| {
        if w < 1 || unit_hash(w, 0x6b) < 0.2 {
            return None;
        }
        let start = (w as f32 + 0.1 + 0.4 * unit_hash(w, 0x6c)) * gap;
        let swing = (unit_hash(w, 0x6d) * 2.0 - 1.0) * spread;
        Some(Gust { start, dir: Vec2::new(swing.cos(), swing.sin()) })
    })
}

/// The wind at `pos` at round time `time` under `sky` (px/s): a
/// sandstorm's gust while one passes there, none otherwise. What
/// `Footing` carries a hull by.
pub fn gust_at(sky: Weather, pos: Position, time: f32, t: &Tuning) -> Vec2 {
    let mut wind = Vec2::new(0.0, 0.0);
    if !t.weather_rules || sky != Weather::Sandstorm || t.sand_gust_speed <= 0.0 {
        return wind;
    }
    for gust in gusts(time, t) {
        let k = gust.strength_at(pos, time, t) * t.sand_gust_speed;
        wind = Vec2::new(wind.x + gust.dir.x * k, wind.y + gust.dir.y * k);
    }
    wind
}

/// The gust whose band is over a field of `width` x `height` at round
/// time `time`, if any - what the sky shader draws.
pub fn gust_on_field(time: f32, width: f32, height: f32, t: &Tuning) -> Option<Gust> {
    let front = t.sand_gust_front_speed.max(1.0);
    let seconds = t.sand_gust_seconds.max(0.05);
    gusts(time, t)
        .filter(|g| {
            let corners = [(0.0, 0.0), (width, 0.0), (0.0, height), (width, height)].map(|(x, y)| x * g.dir.x + y * g.dir.y);
            let near = corners.iter().copied().fold(f32::INFINITY, f32::min);
            let far = corners.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            time - g.start - far / front < seconds && time - g.start - near / front > 0.0
        })
        .last()
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let k = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    k * k * (3.0 - 2.0 * k)
}

impl Game {
    /// How far this round's enemies see (px): `enemy_view_range` under
    /// the sky's `sight_factor`. Every sighting test an enemy makes reads
    /// it - the shared alert, the chase and attack tiers, the engagement
    /// ring, a hunter's snipe.
    pub fn enemy_sight(&self) -> f32 {
        let t = crate::tuning::tuning();
        t.enemy_view_range * sight_factor(self.weather, &t)
    }
}

/// The `weather` query parameter of a page URL, if it names a weather -
/// the web build's `--weather`.
pub fn weather_from_url(url: &str) -> Option<Weather> {
    let query = url.split('#').next()?.split_once('?')?.1;
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| *name == "weather")
        .and_then(|(_, value)| Weather::parse(value.trim()))
}

/// SplitMix64's mixing step: every bit of `h` stirred into every bit of
/// the answer. No RNG, the same on every machine.
fn mix64(mut h: u64) -> u64 {
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 31)
}

/// A hash of `n` to 0..1, for the lightning schedule.
fn unit_hash(n: i64, salt: u64) -> f32 {
    let h = mix64((n as u64).wrapping_add(salt).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// How bright lightning is lighting the field at round time `time`, 0 to
/// 1: a strike in most `lightning_gap_seconds` windows, somewhere in its
/// middle, as a sharp flash and a second flicker a sixth of a second
/// later. A pure function of the clock, so a replica strikes when the
/// room's round would and a frozen round holds its flash.
pub fn lightning(time: f32, t: &Tuning) -> f32 {
    let gap = t.lightning_gap_seconds.max(1.0);
    let window = (time / gap).floor() as i64;
    let mut flash = 0.0f32;
    for w in [window - 1, window] {
        if w < 0 || unit_hash(w, 0x51) < 0.15 {
            continue;
        }
        let strike = (w as f32 + 0.15 + 0.7 * unit_hash(w, 0x9d)) * gap;
        let dt = time - strike;
        if !(0.0..1.0).contains(&dt) {
            continue;
        }
        flash = flash.max((-dt * 20.0).exp());
        if dt >= 0.16 {
            flash = flash.max(0.75 * (-(dt - 0.16) * 7.0).exp());
        }
    }
    flash.clamp(0.0, 1.0)
}

/// The tiles that stop light (`Material::blocks_light`), as a grid of the
/// map's cells - cell `(c, r)` centred on `(c, r) * OBSTACLE_GRID_SIZE`
/// like every tile - built fresh each frame, since a wall shot away lets
/// the light through at once.
#[derive(Clone, Debug, Default)]
pub struct Occluders {
    cols: i32,
    rows: i32,
    solid: Vec<bool>,
}

impl Occluders {
    pub fn of(game: &Game) -> Occluders {
        let (w, h) = game.map.field_size();
        let cols = (w / OBSTACLE_GRID_SIZE).ceil() as i32 + 1;
        let rows = (h / OBSTACLE_GRID_SIZE).ceil() as i32 + 1;
        let mut occ = Occluders { cols, rows, solid: vec![false; (cols * rows).max(0) as usize] };
        for obstacle in game.world.query::<&Obstacle>().iter() {
            if obstacle.material.blocks_light() {
                let (c, r) = obstacle.cell();
                occ.set(c, r);
            }
        }
        occ
    }

    fn set(&mut self, c: i32, r: i32) {
        if (0..self.cols).contains(&c) && (0..self.rows).contains(&r) {
            self.solid[(r * self.cols + c) as usize] = true;
        }
    }

    fn solid(&self, c: i32, r: i32) -> bool {
        (0..self.cols).contains(&c) && (0..self.rows).contains(&r) && self.solid[(r * self.cols + c) as usize]
    }

    /// How far (px) a ray from `from` along the unit vector `dir` travels
    /// before it enters a light-stopping cell, plus `bleed` into it, at
    /// most `radius`. The cell the ray starts in never stops it: a fire on
    /// a burning wall lights its neighbours. An Amanatides-Woo walk of the
    /// grid, one step per cell crossed.
    pub fn reach(&self, from: Position, dir: Vec2, radius: f32, bleed: f32) -> f32 {
        let cell = OBSTACLE_GRID_SIZE;
        // Grid space, where the integers are cell edges.
        let gx = (from.x + cell * 0.5) / cell;
        let gy = (from.y + cell * 0.5) / cell;
        let (mut cx, mut cy) = (gx.floor() as i32, gy.floor() as i32);
        let start = (cx, cy);
        let axis = |d: f32, g: f32, c: i32| -> (i32, f32, f32) {
            if d > 1e-6 {
                (1, (c as f32 + 1.0 - g) * cell / d, cell / d)
            } else if d < -1e-6 {
                (-1, (g - c as f32) * cell / -d, cell / -d)
            } else {
                (0, f32::INFINITY, f32::INFINITY)
            }
        };
        let (step_x, mut next_x, delta_x) = axis(dir.x, gx, cx);
        let (step_y, mut next_y, delta_y) = axis(dir.y, gy, cy);
        let mut t = 0.0;
        loop {
            if (cx, cy) != start && self.solid(cx, cy) {
                return (t + bleed).min(radius);
            }
            if next_x < next_y {
                t = next_x;
                next_x += delta_x;
                cx += step_x;
            } else {
                t = next_y;
                next_y += delta_y;
                cy += step_y;
            }
            if t >= radius || !t.is_finite() {
                return radius;
            }
        }
    }
}

/// Which way a light shines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// Every way at once: a fire, a flash, a glow.
    Point,
    /// A beam along `dir` (a unit vector) with a soft edge from
    /// `half_angle` (radians) out to `CONE_EDGE` times it: a headlight.
    Cone { dir: Vec2, half_angle: f32 },
}

/// One light in the round: where it is, how far it reaches, its colour at
/// the source (linear, intensity folded in) and, per ray, how far it gets
/// before a wall stops it.
#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    pub at: Position,
    pub radius: f32,
    pub color: Rgb,
    pub shape: Shape,
    /// How far each ray reaches, in ray order (`ray`), each at most
    /// `radius`. Filled by `cast`.
    pub reach: Vec<f32>,
    /// Whether walls stop this light (`cast` leaves every ray at `radius`
    /// otherwise): off for the small, quick lights, which would pay a
    /// raycast for a shadow nobody could see.
    pub shadows: bool,
}

impl Light {
    pub fn point(at: Position, radius: f32, color: Rgb) -> Light {
        Light { at, radius, color, shape: Shape::Point, reach: Vec::new(), shadows: true }
    }

    pub fn cone(at: Position, dir: Vec2, half_angle: f32, radius: f32, color: Rgb) -> Light {
        Light { at, radius, color, shape: Shape::Cone { dir, half_angle }, reach: Vec::new(), shadows: true }
    }

    /// The same light, walls or no walls.
    pub fn unshadowed(mut self) -> Light {
        self.shadows = false;
        self
    }

    /// How many rays the light is drawn with: enough that the gap between
    /// two ray ends is a few pixels at full reach.
    pub fn ray_count(&self) -> usize {
        match self.shape {
            Shape::Point => ((std::f32::consts::TAU * self.radius / 7.0).ceil() as usize).clamp(16, 160),
            Shape::Cone { half_angle, .. } => ((2.0 * half_angle * CONE_EDGE * self.radius / 6.0).ceil() as usize + 1).clamp(6, 72),
        }
    }

    /// Ray `i` of `n`: its direction and its share of the light, 1 inside
    /// a cone and falling to 0 across its soft edge (always 1 for a point).
    pub fn ray(&self, i: usize, n: usize) -> (Vec2, f32) {
        match self.shape {
            Shape::Point => {
                let a = std::f32::consts::TAU * i as f32 / n.max(1) as f32;
                (Vec2::new(a.cos(), a.sin()), 1.0)
            }
            Shape::Cone { dir, half_angle } => {
                let spread = half_angle * CONE_EDGE;
                let u = if n > 1 { i as f32 / (n - 1) as f32 } else { 0.5 };
                let off = -spread + 2.0 * spread * u;
                let base = dir.y.atan2(dir.x);
                let a = base + off;
                let edge = ((spread - off.abs()) / (spread - half_angle * 0.7).max(1e-4)).clamp(0.0, 1.0);
                (Vec2::new(a.cos(), a.sin()), edge * edge * (3.0 - 2.0 * edge))
            }
        }
    }

    /// Fill `reach`: every ray against the walls, or every ray at full
    /// radius without them.
    pub fn cast(mut self, occluders: Option<&Occluders>, bleed: f32) -> Light {
        let n = self.ray_count();
        self.reach = (0..n)
            .map(|i| match occluders.filter(|_| self.shadows) {
                Some(occ) => occ.reach(self.at, self.ray(i, n).0, self.radius, bleed),
                None => self.radius,
            })
            .collect();
        self
    }
}

fn rgb(c: Color) -> Rgb {
    [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0]
}

fn scale(c: Rgb, k: f32) -> Rgb {
    c.map(|v| v * k.max(0.0))
}

/// `a` eased toward `b` by `k`.
fn mix(a: Rgb, b: Rgb, k: f32) -> Rgb {
    [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k]
}

/// A fire's unsteady brightness, 0.64 to 1, hashed per source so two
/// fires never flicker in step.
fn flicker(time: f32, at: Position) -> f32 {
    let seed = (seed_for(at) % 1000) as f32 * 0.37;
    0.82 + 0.18 * (time * 23.0 + seed).sin() * (time * 7.3 + seed * 1.7).sin()
}

/// The colour a hull's headlight throws.
fn headlight_color(player: bool) -> Rgb {
    if player { [1.0, 0.95, 0.82] } else { [1.0, 0.68, 0.42] }
}

/// The colour a pickup glows in, keyed to its icon.
fn pickup_color(kind: PickupKind) -> Rgb {
    match kind {
        PickupKind::Health => [1.0, 0.36, 0.36],
        PickupKind::Ammo => [1.0, 0.86, 0.36],
        PickupKind::Laser => [1.0, 0.32, 0.3],
        PickupKind::Minigun => [1.0, 0.7, 0.3],
        PickupKind::Plasma => [0.4, 0.95, 1.0],
        PickupKind::Missiles => [1.0, 0.6, 0.3],
        PickupKind::SpeedUp => [0.85, 1.0, 0.4],
        PickupKind::Shield => [0.45, 0.7, 1.0],
        PickupKind::Flamethrower => [1.0, 0.55, 0.2],
        PickupKind::FrogHealth => [0.45, 1.0, 0.55],
        PickupKind::TowerPack => [0.8, 0.7, 1.0],
    }
}

fn plasma_color(variant: PlasmaVariant) -> Rgb {
    match variant {
        PlasmaVariant::Teal => [0.3, 0.95, 1.0],
        PlasmaVariant::Purple => [0.75, 0.45, 1.0],
    }
}

fn laser_color(variant: LaserVariant) -> Rgb {
    match variant {
        LaserVariant::Red => [1.0, 0.35, 0.3],
        LaserVariant::Blue => [0.45, 0.62, 1.0],
    }
}

/// The unit vector a sprite facing `rotation` degrees points along (0 up,
/// 90 right).
fn heading(rotation: f32) -> Vec2 {
    let r = rotation.to_radians();
    Vec2::new(r.sin(), -r.cos())
}

/// Every light the round throws this frame under `look`, each already
/// cast against the walls: headlights and hull glows, the frogs and the
/// pickups, portals, fires, fuses and burning wrecks, blasts, every shot,
/// flash and beam in flight, and the flash of every hit the particle layer
/// is playing (`impacts`: `fx.rs` is the window's, not the round's). Empty
/// under a sky whose lights do not show.
pub fn lights(game: &Game, impacts: &[Impact], look: &Look, t: &Tuning) -> Vec<Light> {
    let k = look.lights;
    if k <= 0.0 {
        return Vec::new();
    }
    let time = game.time;
    let mut out: Vec<Light> = Vec::new();

    // Tanks: a beam ahead of every live hull, a lamp at its nose and a
    // glow in its seat's colour; a wreck burns until it is a dead hulk.
    for (entity, tank) in game.world.query::<(hecs::Entity, &Tank)>().iter() {
        let player = game.is_player(entity);
        if player && game.hide_players {
            continue;
        }
        if tank.is_wreck() {
            if !tank.is_dead() {
                let left = 1.0 - (tank.wreck_timer / t.wreck_burn_seconds.max(0.01)).clamp(0.0, 1.0);
                let heat = scale([1.0, 0.5, 0.18], k * t.fire_light_strength * left.sqrt() * flicker(time, tank.position));
                out.push(Light::point(tank.position, t.fire_light_radius_px * 0.6, heat));
            }
            continue;
        }
        let dir = heading(tank.visual_rotation);
        let nose = tank.position + dir * (tank.hull_size() * 0.4);
        let beam = headlight_color(player);
        let length = t.headlight_length_px * if player { 1.0 } else { 0.8 };
        if length > 1.0 && t.headlight_strength > 0.0 {
            out.push(Light::cone(nose, dir, t.headlight_half_angle_deg.to_radians(), length, scale(beam, k * t.headlight_strength)));
            out.push(Light::point(nose, 12.0, scale(beam, k * t.headlight_strength * 0.8)).unshadowed());
        }
        if t.hull_glow_radius_px > 1.0 && t.hull_glow_strength > 0.0 {
            let glow = match tank.player_index() {
                Some(seat) => mix(rgb(crate::tank::team_color(seat)), [1.0, 1.0, 1.0], 0.45),
                None => [0.9, 0.55, 0.38],
            };
            let strength = if player { 1.0 } else { 0.7 };
            out.push(Light::point(tank.position, t.hull_glow_radius_px, scale(glow, k * t.hull_glow_strength * strength)).unshadowed());
        }
    }
    for (at, left) in game.burning_tanks() {
        let heat = scale([1.0, 0.52, 0.2], k * t.fire_light_strength * (left / t.flame_afterburn_seconds.max(0.01)).clamp(0.3, 1.0) * flicker(time, at));
        out.push(Light::point(at, t.fire_light_radius_px * 0.5, heat).unshadowed());
    }

    // The frogs and the pickups, faintly, so a lamp can find them.
    if t.pickup_glow_strength > 0.0 {
        for frog_entity in [game.frog, game.enemy_frog].into_iter().flatten() {
            let (at, color, dead) = crate::simulation::with_frog(&game.world, frog_entity, |frog| {
                let color = match frog.side {
                    Side::Player => [0.45, 1.0, 0.55],
                    Side::Enemy => [1.0, 0.45, 0.55],
                };
                (frog.position, color, frog.is_dead())
            });
            if !dead {
                out.push(Light::point(at, 38.0, scale(color, k * t.pickup_glow_strength)).unshadowed());
            }
        }
        for pickup in game.world.query::<&Pickup>().iter() {
            out.push(Light::point(pickup.position, 26.0, scale(pickup_color(pickup.kind), k * t.pickup_glow_strength)).unshadowed());
        }
    }

    // Portals glow from below their spiral.
    if game.portals_active() {
        for (i, &at) in game.portals.iter().enumerate() {
            let pulse = 0.85 + 0.15 * (time * 2.2 + i as f32 * 1.7).sin();
            out.push(Light::point(at, 112.0, scale([0.45, 0.6, 1.25], k * 0.9 * pulse)));
        }
    }

    // Fire: burning ground, burning tiles, lit fuses, drums in the air.
    let fire = |at: Position, radius: f32, strength: f32| {
        Light::point(at, radius, scale([1.0, 0.56, 0.22], k * t.fire_light_strength * strength * flicker(time, at)))
    };
    for (at, left, total) in game.burning_cells() {
        let fade = (left / total.max(0.01)).clamp(0.0, 1.0).sqrt().max(0.25);
        out.push(fire(at, t.fire_light_radius_px, 1.1 * fade));
    }
    for obstacle in game.world.query::<&Obstacle>().iter() {
        if obstacle.burning {
            out.push(fire(obstacle.position, t.fire_light_radius_px * 0.9, 1.0));
        } else if obstacle.fuse.is_some() {
            let pulse = 0.55 + 0.45 * (time * 12.0).sin().abs();
            out.push(Light::point(obstacle.position, 72.0, scale([1.0, 0.4, 0.2], k * t.fire_light_strength * pulse)));
        }
    }
    for drum in &game.flying_drums {
        out.push(fire(drum.ground_pos(), 56.0, 0.8).unshadowed());
    }

    // The towers: a tesla's coil as it charges and every bolt along its
    // path, a mortar's ooze and the globs it lobs, a tower on fire.
    for view in game.tower_views() {
        match view.kind {
            TowerKind::Tesla if view.charge > 0.05 => {
                let charge = view.charge * view.charge;
                out.push(Light::point(view.position, 56.0 + 56.0 * charge, scale(TESLA_LIGHT, k * 1.1 * charge)));
            }
            TowerKind::Bio => out.push(Light::point(view.position, 44.0, scale(OOZE_LIGHT, k * 0.35)).unshadowed()),
            _ => {}
        }
        if view.burning {
            out.push(fire(view.position, t.fire_light_radius_px * 0.9, 1.0));
        }
    }
    for bolt in &game.tesla_bolts {
        let strength = k * t.shot_light_strength * bolt.alpha();
        for along in [0.0, 0.5, 1.0] {
            let at = bolt.start + (bolt.end - bolt.start) * along;
            out.push(Light::point(at, 70.0, scale(TESLA_LIGHT, strength * (1.2 - 0.4 * (along - 0.5f32).abs()))));
        }
    }
    for glob in &game.globs {
        out.push(Light::point(glob.ground_pos(), 30.0, scale(OOZE_LIGHT, k * 0.45)).unshadowed());
    }
    for (&(c, r), puddle) in &game.ooze {
        out.push(Light::point(crate::map::cell_to_world(c, r), 30.0, scale(OOZE_LIGHT, k * 0.25 * puddle.freshness())).unshadowed());
    }

    // Blasts light the field in the frame they go off and fade with their
    // glow; a mushroom cloud's flash reaches half as far again.
    let s = k * t.shot_light_strength;
    let blast_life = t.blast_glow_seconds.max(0.05) * 2.0;
    for blast in &game.blast_fx {
        let life = 1.0 - blast.time / blast_life;
        if life <= 0.0 {
            continue;
        }
        let cloud = if blast.cloud.is_some() { 1.5 } else { 1.0 };
        let radius = t.blast_light_radius_px * blast.scale * cloud * (0.75 + 0.45 * (1.0 - life));
        let color = match blast.kind {
            BlastKind::Oil => [1.0, 0.62, 0.3],
            BlastKind::Fuel => [1.0, 0.82, 0.55],
        };
        // Where the fireball is drawn: the centre plus its cause's lean.
        let at = Position::new(blast.center.x + blast.offset.x, blast.center.y + blast.offset.y);
        out.push(Light::point(at, radius, scale(color, s * 2.2 * life.powf(1.6) * cloud.sqrt())));
    }

    // Shots in flight, and the flashes where they leave and land.
    for shell in game.world.query::<&Shell>().iter() {
        if shell.state == ShellState::Flying {
            out.push(Light::point(shell.position, 52.0, scale([1.0, 0.62, 0.28], s * 0.75)));
        }
    }
    for bullet in game.world.query::<&Bullet>().iter() {
        if bullet.state == BulletState::Flying {
            out.push(Light::point(bullet.position, 22.0, scale([1.0, 0.8, 0.42], s * 0.45)).unshadowed());
        }
    }
    for plasma in game.world.query::<&Plasma>().iter() {
        let color = plasma_color(plasma.variant);
        match plasma.impact_progress() {
            Some(p) => out.push(Light::point(plasma.position, 60.0 + 40.0 * p, scale(color, s * 1.2 * (1.0 - p)))),
            None if plasma.state == PlasmaState::Flying => out.push(Light::point(plasma.position, 76.0, scale(color, s))),
            None => {}
        }
    }
    for missile in game.world.query::<&Missile>().iter() {
        out.push(Light::point(missile.position, 48.0, scale([1.0, 0.6, 0.3], s * 0.85 * (1.0 - 0.5 * missile.lift()))).unshadowed());
    }
    for jet in game.flames() {
        for along in [0.25, 0.55, 0.85] {
            let at = jet.origin + jet.dir * (jet.reach * along);
            out.push(fire(at, jet.reach * 0.45 + 24.0, 0.9).unshadowed());
        }
    }
    for beam in &game.laser_beams {
        let life = (beam.timer / t.laser_beam_display_seconds.max(0.01)).clamp(0.0, 1.0);
        let color = laser_color(beam.variant);
        let span = beam.end - beam.start;
        let steps = (span.length() / 48.0).ceil().max(1.0) as usize;
        for i in 0..steps {
            let at = beam.start + span * ((i as f32 + 0.5) / steps as f32);
            out.push(Light::point(at, 40.0, scale(color, s * 0.6 * life)).unshadowed());
        }
        out.push(Light::point(beam.end, 80.0, scale(color, s * 1.2 * life)));
    }
    for flash in &game.muzzle_flashes {
        let life = 1.0 - (flash.time / t.muzzle_flash_duration.max(0.01)).clamp(0.0, 1.0);
        if life > 0.0 {
            out.push(Light::point(flash.center, 90.0 * (0.6 + 0.4 * life), scale([1.0, 0.82, 0.55], s * 1.8 * life)));
        }
    }
    for flash in &game.impact_flashes {
        let life = 1.0 - (flash.time / t.impact_flash_duration.max(0.01)).clamp(0.0, 1.0);
        if life > 0.0 {
            out.push(Light::point(flash.center, 100.0, scale([1.0, 0.66, 0.35], s * 1.6 * life)));
        }
    }

    for impact in impacts {
        let life = (1.0 - impact.progress()).powf(1.5);
        if life <= 0.0 {
            continue;
        }
        out.push(match impact.kind {
            ImpactKind::Shell => Light::point(impact.pos, 110.0, scale([1.0, 0.66, 0.35], s * 1.8 * life)),
            ImpactKind::Bullet => Light::point(impact.pos, 36.0, scale([1.0, 0.85, 0.5], s * 0.8 * life)).unshadowed(),
            ImpactKind::Plasma(variant) => Light::point(impact.pos, 96.0, scale(plasma_color(variant), s * 1.5 * life)),
            ImpactKind::Laser(blue) => {
                let variant = if blue { LaserVariant::Blue } else { LaserVariant::Red };
                Light::point(impact.pos, 70.0, scale(laser_color(variant), s * 1.4 * life))
            }
            ImpactKind::Tesla => Light::point(impact.pos, 90.0, scale(TESLA_LIGHT, s * 1.5 * life)),
            ImpactKind::Ooze => Light::point(impact.pos, 44.0, scale(OOZE_LIGHT, s * 0.6 * life)).unshadowed(),
            ImpactKind::Ricochet => Light::point(impact.pos, 30.0, scale([1.0, 0.85, 0.5], s * 0.6 * life)).unshadowed(),
            ImpactKind::Deflect => Light::point(impact.pos, 30.0, scale([0.67, 0.47, 1.0], s * 0.6 * life)).unshadowed(),
            // Dust throws no light.
            ImpactKind::Dust(_) | ImpactKind::Collapse(_) | ImpactKind::Ash => continue,
        });
    }

    let occluders = t.light_shadows.then(|| Occluders::of(game));
    out.into_iter().map(|light| light.cast(occluders.as_ref(), t.light_wall_bleed_px)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{cell_to_world, MapFile};
    use crate::simulation::{Input, PlayerCount};

    const W: f32 = 1088.0;
    const H: f32 = 544.0;

    fn round(map: &str) -> Game {
        let mut game = Game::default();
        game.enemy_count_override = Some(0);
        game.seed_override = Some(0xB0B5);
        game.player_row_override = Some(0);
        game.players = PlayerCount::ONE;
        game.map = MapFile::from_toml_str(map).expect("test map parses");
        game.init(W, H);
        game.update(Input::default(), crate::PHYSICS_FIXED_DT, W, H);
        game
    }

    #[test]
    fn clear_draws_nothing_and_every_other_sky_draws_something() {
        let t = Tuning::DEFAULT;
        assert_eq!(Look::of(Weather::Clear, &t).plan(), None);
        for w in Weather::SKIES.into_iter().filter(|w| *w != Weather::Clear) {
            assert!(Look::of(w, &t).plan().is_some(), "{w:?} draws nothing");
        }
        let night = Look::of(Weather::Night, &t).plan().unwrap();
        assert_eq!(night, Plan { lit: true, ground: false, sky: false }, "night is light alone");
        let rain = Look::of(Weather::Rain, &t).plan().unwrap();
        assert!(rain.lit && rain.ground && rain.sky, "rain wets the ground and fills the air");
        let snow = Look::of(Weather::Snow, &t).plan().unwrap();
        assert!(snow.ground && snow.sky, "snow covers the ground and falls");
        let haze = Look::of(Weather::HeatHaze, &t).plan().unwrap();
        assert!(!haze.ground && haze.sky, "haze is the air alone");
    }

    #[test]
    fn weather_strength_zero_is_a_clear_day_and_the_knobs_scale_their_layer() {
        let mut t = Tuning::DEFAULT;
        t.weather_strength = 0.0;
        for w in Weather::SKIES {
            assert_eq!(Look::of(w, &t).plan(), None, "{w:?} at strength 0");
        }
        let mut t = Tuning::DEFAULT;
        let base = Look::of(Weather::Rain, &t).rain;
        t.rain_density = 2.0;
        assert!((Look::of(Weather::Rain, &t).rain - 2.0 * base).abs() < 1e-6);
        let mut t = Tuning::DEFAULT;
        t.night_ambient = 0.5;
        let brighter = Look::of(Weather::Night, &t);
        assert!(brighter.ambient[2] > Look::of(Weather::Night, &Tuning::DEFAULT).ambient[2]);
        assert!(Look::of(Weather::Night, &Tuning::DEFAULT).darkness() > 0.7, "night is dark");
        assert!(Look::of(Weather::HeatHaze, &Tuning::DEFAULT).darkness() < 0.05, "heat haze is bright");
    }

    #[test]
    fn the_override_knob_outranks_the_map_and_minus_one_follows_it() {
        let mut t = Tuning::DEFAULT;
        assert_eq!(in_force(Weather::Fog, 7, false, &t), Weather::Fog);
        t.weather_override = Weather::Night.index() as i32;
        assert_eq!(in_force(Weather::Fog, 7, false, &t), Weather::Night);
        assert_eq!(in_force(Weather::Random, 7, false, &t), Weather::Night, "a named override outranks a random map");
        t.weather_override = Weather::Clear.index() as i32;
        assert_eq!(in_force(Weather::Snow, 7, false, &t), Weather::Clear, "the override can clear a map's sky");
        t.weather_override = Weather::Random.index() as i32;
        assert_eq!(in_force(Weather::Fog, 7, false, &t), random_sky(7), "a random override rolls over every map");
        // A room's round is its map's: the knob is the window's own.
        t.weather_override = Weather::Night.index() as i32;
        assert_eq!(in_force(Weather::Fog, 7, true, &t), Weather::Fog);
        assert_eq!(in_force(Weather::Random, 7, true, &t), random_sky(7));
        let mut table = Tuning::DEFAULT;
        assert!(table.set("weather_override", Weather::Random.index() as f64).is_ok(), "the knob's range reaches random");
        assert!(table.set("weather_override", Weather::ALL.len() as f64).is_err(), "and ends there");
    }

    #[test]
    fn a_random_sky_is_the_seeds_and_every_sky_comes_up() {
        let t = Tuning::DEFAULT;
        let mut counts = [0u32; Weather::SKIES.len()];
        for seed in 0..9000u64 {
            let sky = in_force(Weather::Random, seed, false, &t);
            assert_ne!(sky, Weather::Random, "random always resolves to a sky");
            assert_eq!(sky, random_sky(seed), "the same seed, the same sky");
            counts[sky.index()] += 1;
        }
        // 1000 of each on average; a sky under 850 or over 1150 is a
        // biased pick, not chance (about five standard deviations).
        for (sky, n) in Weather::SKIES.iter().zip(counts) {
            assert!((850..=1150).contains(&n), "{} came up {n} times in 9000 seeds", sky.name());
        }
        // Neighbouring seeds - a sweep of base + i, like the probe's - are
        // not all one sky.
        let run: Vec<Weather> = (0..8u64).map(random_sky).collect();
        assert!(run.windows(2).any(|w| w[0] != w[1]), "{run:?}");
        // A map's own sky is not touched by the seed.
        assert!((0..50u64).all(|seed| in_force(Weather::Dusk, seed, false, &t) == Weather::Dusk));
    }

    #[test]
    fn a_random_map_draws_the_same_sky_for_the_same_seed() {
        use crate::{DEFAULT_SCREEN_HEIGHT, DEFAULT_SCREEN_WIDTH};
        let round = |seed: u64| {
            let mut game = Game::default();
            game.map = crate::map::open_map("default").unwrap();
            game.map.weather = Weather::Random;
            game.seed_override = Some(seed);
            game.enemy_count_override = Some(0);
            game.init(DEFAULT_SCREEN_WIDTH as f32, DEFAULT_SCREEN_HEIGHT as f32);
            game
        };
        let a = round(0xB0B5);
        assert_eq!(a.weather(), random_sky(0xB0B5));
        assert_eq!(round(0xB0B5).weather(), a.weather(), "a replay is drawn under the sky it was fought under");
        let skies: std::collections::BTreeSet<Weather> = (1..=40u64).map(|seed| round(seed).weather()).collect();
        assert!(skies.len() >= 5, "forty seeds bring several skies: {skies:?}");
    }

    #[test]
    fn gusts_come_now_and_then_and_blow_where_their_band_is() {
        let t = Tuning::DEFAULT;
        let gap = t.sand_gust_gap_seconds;
        // One window's worth of gusts, each counted once by its start.
        let mut starts: Vec<f32> = (0..400).flat_map(|w| gusts(w as f32 * gap + 0.5 * gap, &t).map(|g| g.start)).collect();
        starts.sort_by(f32::total_cmp);
        starts.dedup();
        assert!(starts.iter().all(|s| *s >= gap), "the round's first window is calm: {:?}", &starts[..3]);
        let per_window = starts.len() as f32 / 400.0;
        assert!((0.7..0.9).contains(&per_window), "most windows gust: {per_window:.2} per window");
        for gust in gusts(20.0 * gap, &t) {
            let swing = gust.dir.y.atan2(gust.dir.x).abs().to_degrees();
            assert!(swing <= t.sand_gust_spread_deg + 1e-3, "{gust:?}");
            assert!((gust.dir.x * gust.dir.x + gust.dir.y * gust.dir.y - 1.0).abs() < 1e-5);
        }
        // Follow one gust across a point: it rises, peaks at full strength
        // and dies away, and only while its band is over the field.
        let gust = gusts(starts[0] + 0.01, &t).find(|g| g.start == starts[0]).expect("the first gust");
        let at = Position::new(400.0, 200.0);
        let mut peak = 0.0f32;
        for i in 0..600 {
            let time = gust.start + i as f32 * 0.01;
            let k = gust.strength_at(at, time, &t);
            assert!((0.0..=1.0).contains(&k));
            peak = peak.max(k);
            let wind = gust_at(Weather::Sandstorm, at, time, &t);
            if k > 0.0 {
                assert!(wind.x > 0.0, "downwind is east: {wind:?} at {time}");
                assert!(gust_on_field(time, 1088.0, 544.0, &t).is_some(), "a band over the field at {time}");
            }
        }
        assert!(peak > 0.99, "a gust blows at full strength for a moment: {peak}");
        assert!(gust_on_field(starts[0] - 3.0, 1088.0, 544.0, &t).is_none(), "calm before it");
        // No other sky blows.
        for sky in Weather::SKIES.into_iter().filter(|s| *s != Weather::Sandstorm) {
            let wind = gust_at(sky, at, gust.start + 1.0, &t);
            assert!(wind.x == 0.0 && wind.y == 0.0, "{sky:?}");
        }
    }

    #[test]
    fn the_page_url_names_a_weather() {
        assert_eq!(weather_from_url("https://bongbong.io/?weather=night"), Some(Weather::Night));
        assert_eq!(weather_from_url("http://localhost:4321/?rooms=ws://x&weather=heat_haze#top"), Some(Weather::HeatHaze));
        assert_eq!(weather_from_url("https://bongbong.io/?weather=hail"), None, "an unknown name is ignored");
        assert_eq!(weather_from_url("https://bongbong.io/j/AK7QX"), None);
    }

    #[test]
    fn lightning_is_a_pure_function_of_the_clock_and_strikes_now_and_then() {
        let t = Tuning::DEFAULT;
        let samples: Vec<f32> = (0..6000).map(|i| lightning(i as f32 / 60.0, &t)).collect();
        assert!(samples.iter().all(|v| (0.0..=1.0).contains(v)));
        assert_eq!(lightning(12.34, &t), lightning(12.34, &t), "the same instant flashes the same");
        // Count the strikes in 100 seconds: a strike is a rise past half.
        let strikes = samples.windows(2).filter(|w| w[0] < 0.5 && w[1] >= 0.5).count();
        let expected = 100.0 / t.lightning_gap_seconds;
        assert!(strikes as f32 >= expected * 0.5 && strikes as f32 <= expected * 2.5, "{strikes} strikes in 100 s");
        assert!(samples.iter().filter(|v| **v > 0.05).count() < samples.len() / 4, "the sky is dark most of the time");
    }

    #[test]
    fn a_wall_stops_a_ray_and_the_cell_a_light_stands_in_does_not() {
        let game = round(
            r#"
version = 1
cells."2,2" = { kind = "frog" }
cells."20,12" = { kind = "start" }
cells."10,5" = { kind = "wall", material = "brick" }
cells."10,8" = { kind = "wall", material = "glass" }
cells."14,5" = { kind = "wall", material = "wood" }
"#,
        );
        let occ = Occluders::of(&game);
        let from = cell_to_world(6, 5);
        // East into the brick at column 10: its west face is at 10*32-16.
        let hit = occ.reach(from, Vec2::new(1.0, 0.0), 400.0, 0.0);
        assert!((hit - (10.0 * 32.0 - 16.0 - from.x)).abs() < 0.5, "{hit}");
        assert!((occ.reach(from, Vec2::new(1.0, 0.0), 400.0, 10.0) - (hit + 10.0)).abs() < 0.5, "bleed carries into the wall");
        // West is open all the way out.
        assert_eq!(occ.reach(from, Vec2::new(-1.0, 0.0), 150.0, 0.0), 150.0);
        // Glass lets light through.
        let glass_row = cell_to_world(6, 8);
        assert_eq!(occ.reach(glass_row, Vec2::new(1.0, 0.0), 300.0, 0.0), 300.0, "glass is not a shadow");
        // A light inside a wall cell (a burning wood wall) is not stopped
        // by its own cell.
        assert_eq!(occ.reach(cell_to_world(14, 5), Vec2::new(0.0, 1.0), 120.0, 0.0), 120.0);
        // A diagonal walk still finds the wall.
        let diag = Vec2::new(1.0, 1.0) * std::f32::consts::FRAC_1_SQRT_2;
        let near = occ.reach(cell_to_world(8, 3), diag, 400.0, 0.0);
        assert!(near < 100.0, "the diagonal runs into the brick at (10, 5): {near}");
    }

    #[test]
    fn a_cone_is_full_in_the_middle_and_fades_across_its_edge() {
        let light = Light::cone(Position::new(0.0, 0.0), Vec2::new(0.0, -1.0), 0.4, 200.0, [1.0; 3]);
        let n = light.ray_count();
        assert!(n >= 6);
        let (mid, w_mid) = light.ray(n / 2, n);
        assert!(mid.y < -0.99 && w_mid > 0.99, "the middle ray points ahead at full strength");
        let (_, w_edge) = light.ray(0, n);
        assert!(w_edge < 0.01, "the outermost ray has faded out");
        let point = Light::point(Position::new(0.0, 0.0), 100.0, [1.0; 3]);
        let n = point.ray_count();
        assert!((0..n).all(|i| point.ray(i, n).1 == 1.0));
    }

    #[test]
    fn a_hull_throws_a_beam_ahead_and_a_wall_cuts_it_short() {
        let open = round("version = 1\ncells.\"2,2\" = { kind = \"frog\" }\ncells.\"10,8\" = { kind = \"start\" }\n");
        let t = Tuning::DEFAULT;
        let look = Look::of(Weather::Night, &t);
        let lights = lights(&open, &[], &look, &t);
        let beam = lights.iter().find(|l| matches!(l.shape, Shape::Cone { .. })).expect("a headlight");
        assert!(beam.reach.iter().all(|r| (*r - beam.radius).abs() < 1e-3), "nothing in the way");
        // The same hull with a wall right in front of it: the start faces
        // up, so put the wall two cells above.
        let walled = round("version = 1\ncells.\"2,2\" = { kind = \"frog\" }\ncells.\"10,8\" = { kind = \"start\" }\ncells.\"10,6\" = { kind = \"wall\", material = \"iron\" }\n");
        let lights = super::lights(&walled, &[], &look, &t);
        let beam = lights.iter().find(|l| matches!(l.shape, Shape::Cone { .. })).expect("a headlight");
        let n = beam.reach.len();
        assert!(beam.reach[n / 2] < beam.radius * 0.5, "the wall stops the middle of the beam: {:?}", beam.reach);
        // Daylight throws no light at all, and nothing here draws RNG or
        // moves the round.
        let frame = walled.frame();
        assert!(super::lights(&walled, &[], &Look::of(Weather::Clear, &t), &t).is_empty());
        assert_eq!(walled.frame(), frame);
    }

    /// The passes' desktop sources and their GLSL ES 100 twins in
    /// `static/web/` are ported by hand; every uniform the renderer sets
    /// has to be declared, with the same type, in both.
    #[test]
    fn the_web_twins_declare_the_desktop_uniforms() {
        fn uniforms(source: &str) -> Vec<String> {
            let mut out: Vec<String> = source
                .lines()
                .filter_map(|line| line.trim().strip_prefix("uniform "))
                .map(|rest| rest.split(';').next().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" "))
                .collect();
            out.sort();
            out
        }
        let pairs = [
            (include_str!("../static/weather_light.fs"), include_str!("../static/web/weather_light.fs")),
            (include_str!("../static/weather_ground.fs"), include_str!("../static/web/weather_ground.fs")),
            (include_str!("../static/weather_sky.fs"), include_str!("../static/web/weather_sky.fs")),
        ];
        for (desktop, web) in pairs {
            assert!(desktop.starts_with("#version 330") && web.starts_with("#version 100"));
            assert_eq!(uniforms(desktop), uniforms(web));
            assert!(web.contains("gl_FragColor") && !web.contains("finalColor ="), "the twin writes gl_FragColor");
        }
    }

    #[test]
    fn every_light_is_finite_and_inside_its_radius_on_a_shipped_map() {
        let t = Tuning::DEFAULT;
        let mut game = Game::default();
        game.seed_override = Some(0xB0B5);
        game.enemy_count_override = Some(4);
        game.map = MapFile::from_toml_str(include_str!("../maps/default.toml")).unwrap();
        let (w, h) = game.map.field_size();
        game.init(w, h);
        for _ in 0..120 {
            game.update(Input::default(), crate::PHYSICS_FIXED_DT, w, h);
        }
        for weather in Weather::SKIES {
            let look = Look::of(weather, &t);
            for light in lights(&game, &[], &look, &t) {
                assert!(light.at.x.is_finite() && light.at.y.is_finite() && light.radius > 0.0, "{light:?}");
                assert!(light.color.iter().all(|c| c.is_finite() && *c >= 0.0), "{light:?}");
                assert_eq!(light.reach.len(), light.ray_count());
                assert!(light.reach.iter().all(|r| *r >= 0.0 && *r <= light.radius + 1e-3), "{light:?}");
            }
        }
    }
}
