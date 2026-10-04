//! Short-lived particles: sparks, chips, dust, smoke and embers.
//!
//! This is the presentation half of the destruction work, and it lives
//! outside `Game` on purpose. `main.rs` owns the `Fx`, feeds it from the
//! event log between `Game::update` and `Game::render`, and passes it in
//! to be drawn. Nothing in `simulation/` can see it, no snapshot carries
//! it, and `render` takes `&Game`, so there is no path from a particle
//! back into a simulation value - which is exactly why this module may
//! use `rand::rng()` freely while `decal.rs` may not. Anything whose
//! *position matters* to gameplay later (where rubble lands) belongs in
//! `decal.rs` instead; this module is only for things that vanish.
//!
//! Everything draws as a raylib primitive rather than a sprite. At this
//! scale a chip is one or two design pixels, so an atlas would buy detail
//! nobody can see, and keeping the additive kinds in one contiguous block
//! matters far more for the web build than the shape of each speck does.

use std::collections::{HashMap, HashSet};

use rand::RngExt;
use crate::math::{Color, Vec2};

use crate::bullet::Bullet;
use crate::obstacle::{Drum, Material};
use crate::plasma::{Plasma, PlasmaState, PlasmaVariant};
use crate::shell::{Shell, ShellState};
use crate::simulation::{Event, Game, HitTarget};
use crate::tuning::tuning;
use crate::Position;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParticleKind {
    /// A hot fleck thrown off an impact: additive, bright, barely falls.
    Spark,
    /// A dark chip of whatever was hit: falls, bounces, tumbles.
    Chip,
    /// A ground-hugging puff that expands and fades fast.
    Dust,
    /// A column that rises, expands and lingers.
    Smoke,
    /// A mote off a fire: rises, flickers, additive.
    Ember,
    /// A droplet thrown up by a hull wading (docs/water.md): arcs up and
    /// falls back, gone the moment it lands - water does not bounce.
    Spray,
    /// A puff of a seeker missile's smoke trail: hangs in the air where
    /// the missile left it, swells a little and pales away.
    Trail,
}

/// Which weapon a hit came from - each has its own burst.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ImpactKind {
    Shell,
    Bullet,
    Plasma(PlasmaVariant),
    /// A laser's burn; `true` for the blue beam.
    Laser(bool),
    /// A tesla coil's bolt landing.
    Tesla,
    /// A bio slush glob bursting on the ground.
    Ooze,
    /// A shot glancing off iron or a barrel and flying on.
    Ricochet,
    /// A shot turned away by a rainbow shield.
    Deflect,
    /// Dust knocked off a tile a shot hit and did not break, in the
    /// material's colours (`pyro::dust_of`).
    Dust(Material),
    /// A tile coming down: a cloud of its dust rolling out from where it
    /// stood.
    Collapse(Material),
    /// A tile that burnt out falling in: a cloud of ash.
    Ash,
}

impl ImpactKind {
    /// How long this kind of hit plays, from the `shot_fx` knobs.
    pub fn seconds(self) -> f32 {
        let t = tuning();
        match self {
            ImpactKind::Shell => t.shell_hit_seconds,
            ImpactKind::Bullet => t.bullet_hit_seconds,
            ImpactKind::Plasma(_) => t.plasma_hit_seconds,
            ImpactKind::Laser(_) => t.laser_hit_seconds,
            ImpactKind::Tesla => t.tesla_hit_seconds,
            ImpactKind::Ooze => t.ooze_hit_seconds,
            ImpactKind::Ricochet | ImpactKind::Deflect => t.bullet_hit_seconds * 0.7,
            ImpactKind::Dust(_) => t.tile_dust_seconds,
            ImpactKind::Collapse(_) | ImpactKind::Ash => t.tile_collapse_seconds,
        }
    }
}

/// One hit playing out where a shot landed: kept here, on the particle
/// layer's own clock, rather than read off the projectile, so it runs
/// smoothly for as long as it likes - past the projectile's own short
/// impact frames, and on a replica whose projectiles' timers never run.
pub struct Impact {
    pub(crate) pos: Position,
    /// The way the shot was travelling (a unit vector). Only the drawing
    /// reads it (`burst::compose`).
    #[cfg_attr(not(feature = "render"), allow(dead_code))]
    pub(crate) dir: Vec2,
    pub(crate) kind: ImpactKind,
    pub(crate) age: f32,
    /// Per-hit variety for the burst's hashed parts: a hash of where it
    /// landed and of how many hits came before it. Only the drawing reads
    /// it.
    #[cfg_attr(not(feature = "render"), allow(dead_code))]
    pub(crate) seed: u32,
}

impl Impact {
    /// 0 at the hit to 1 as it ends.
    pub fn progress(&self) -> f32 {
        (self.age / self.kind.seconds().max(0.01)).clamp(0.0, 1.0)
    }
}

/// What a hit lit up: a hull, by its owner slot, or the tile a shot struck
/// without breaking, by where it struck.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Flashed {
    Tank(usize),
    Tile(Position),
}

/// A hit's flash: the hull or tile it landed on drawn again in light for a
/// few frames (`render/game.rs`), stepping down as it goes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Flash {
    pub(crate) what: Flashed,
    pub(crate) age: f32,
}

impl Flash {
    /// How bright it is now, in three steps: full, two thirds, one third,
    /// then gone at `hit_flash_seconds`.
    pub fn strength(&self) -> f32 {
        let life = tuning().hit_flash_seconds;
        if life <= 0.0 || self.age >= life {
            return 0.0;
        }
        match (self.age / life * 3.0) as i32 {
            0 => 1.0,
            1 => 0.66,
            _ => 0.33,
        }
    }
}

/// A crate being taken, playing out where it stood (`crate_fx::open`):
/// started by `Event::PickupCollected`, on this layer's clock, so it runs on
/// a replica as it does in a local round - the crate itself is gone from
/// the world the frame it is taken.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CrateOpen {
    pub(crate) kind: crate::pickup::PickupKind,
    pub(crate) at: Position,
    /// The owner slot of the tank that took it, whose hull the symbol
    /// flies into; `None` for a crate a blast or fire broke.
    pub(crate) slot: Option<usize>,
    /// It was a broken crate's contents lying loose: no crate left to split.
    pub(crate) spilled: bool,
    pub(crate) age: f32,
}

pub struct Particle {
    pub(crate) pos: Position,
    /// Only the drawing reads it outside this module: a fast spark is
    /// drawn with a block of motion blur behind it.
    #[cfg_attr(not(feature = "render"), allow(dead_code))]
    pub(crate) vel: Vec2,
    /// Fake height above the ground plane. The game is top-down with no
    /// camera, so this is only a draw-time y-offset - the same trick
    /// `decal::Decal`'s arc uses.
    pub(crate) z: f32,
    vz: f32,
    pub(crate) age: f32,
    pub(crate) life: f32,
    pub(crate) size: f32,
    /// The colour it was thrown with; only the drawing reads it.
    #[cfg_attr(not(feature = "render"), allow(dead_code))]
    pub(crate) tint: Color,
    pub(crate) kind: ParticleKind,
}

/// Every short-lived effect the presentation layer owns.
#[derive(Default)]
pub struct Fx {
    particles: Vec<Particle>,
    /// The last `Game::frame()` `observe` consumed. The dev server can
    /// render many times without advancing (`pause`, `step`), and
    /// `Game::events` still holds the previously advanced frame's
    /// contents - without this guard a frozen frame re-emits its bursts
    /// on every single render.
    last_frame: u64,
    /// Fractional carry for rate-based emitters. A tile giving off 22
    /// embers a second at 120fps owes 0.18 of an ember per frame, so each
    /// source keeps its own remainder, keyed by a hash of its position.
    accum: HashMap<u32, f32>,
    /// Owner slots of the hulls that were wading last frame, so the frame
    /// one wades in gets a splash rather than only the running spray.
    wading: HashSet<usize>,
    /// Where each missile in the air (by its key from `Game::missiles`)
    /// last laid a trail puff: the trail is laid by distance flown, so it
    /// stays a continuous line at any speed and frame rate.
    trail_last: HashMap<u32, Position>,
    /// Hits playing out, oldest first (`Impact`).
    impacts: Vec<Impact>,
    /// Crates being taken, oldest first (`CrateOpen`).
    opens: Vec<CrateOpen>,
    /// Ids of the shots already seen in their impact frames, so each hit
    /// is started once.
    impacts_seen: HashSet<u32>,
    /// The round clock at the last `observe`: the wind the smoke drifts
    /// on (`pyro::smoke_lean`) is a function of it.
    clock: f32,
    /// Hits started so far: salts each hit's seed, so a burst of minigun
    /// rounds into one spot does not repeat one picture.
    impacts_started: u32,
    /// The hulls and tiles a hit has just lit up.
    flashes: Vec<Flash>,
    /// The cells of the tiles burning at the last `observe`: a tile that
    /// dies in one of them burnt out, and falls in as ash.
    burning: HashSet<(i32, i32)>,
}

impl Fx {
    pub fn live(&self) -> usize {
        self.particles.len()
    }

    /// Every hit still playing, oldest first, for the renderer.
    #[cfg(feature = "render")]
    pub(crate) fn impacts(&self) -> &[Impact] {
        &self.impacts
    }

    fn start_open(&mut self, open: CrateOpen) {
        if self.opens.len() >= MAX_OPENS {
            self.opens.remove(0);
        }
        self.opens.push(open);
    }

    /// Every crate still being opened, oldest first, for the renderer.
    pub fn opens(&self) -> &[CrateOpen] {
        &self.opens
    }

    /// Every hull and tile still flashing from a hit, for the renderer.
    pub fn flashes(&self) -> &[Flash] {
        &self.flashes
    }

    /// Light up what a hit landed on; a second hit on it starts its flash
    /// over rather than stacking another.
    fn flash(&mut self, what: Flashed) {
        if tuning().hit_flash_seconds <= 0.0 {
            return;
        }
        match self.flashes.iter_mut().find(|f| f.what == what) {
            Some(f) => f.age = 0.0,
            None => self.flashes.push(Flash { what, age: 0.0 }),
        }
    }

    fn start_impact(&mut self, pos: Position, velocity: Vec2, kind: ImpactKind) {
        let len = velocity.length();
        let dir = if len > 0.01 { velocity * (1.0 / len) } else { Vec2::new(0.0, -1.0) };
        if self.impacts.len() >= MAX_IMPACTS {
            self.impacts.remove(0);
        }
        self.impacts_started = self.impacts_started.wrapping_add(1);
        let seed = crate::blast::seed_at(pos, 31).wrapping_add(self.impacts_started.wrapping_mul(0x9E37_79B9));
        self.impacts.push(Impact { pos, dir, kind, age: 0.0, seed });
    }

    /// Every live particle, oldest first, for `render::fx::draw`.
    #[cfg(feature = "render")]
    pub(crate) fn particles(&self) -> &[Particle] {
        &self.particles
    }

    /// Drop everything. Called when a new round starts - the `Fx` outlives
    /// `Game::init`, which knows nothing about it, so without this the
    /// previous round's smoke hangs over the new one's opening frame.
    pub fn clear(&mut self) {
        self.particles.clear();
        self.accum.clear();
        self.wading.clear();
        self.trail_last.clear();
        self.impacts.clear();
        self.impacts_seen.clear();
        self.opens.clear();
        self.flashes.clear();
        self.burning.clear();
    }

    fn push(&mut self, p: Particle) {
        let cap = tuning().fx_max_particles.max(0) as usize;
        if cap == 0 {
            return;
        }
        if self.particles.len() >= cap {
            // Oldest first: a burst that overflows the cap should eat into
            // the smoke still hanging around from a minute ago, not
            // cannibalise itself half way through.
            let excess = self.particles.len() + 1 - cap;
            self.particles.drain(..excess);
        }
        self.particles.push(p);
    }

    /// Scale a requested count by `fx_density`, which the web build lowers.
    fn count(&self, n: i32) -> i32 {
        ((n as f32) * tuning().fx_density).round() as i32
    }

    // ---- emitters ------------------------------------------------------

    /// One mote of the flamethrower's stream: a droplet of burning fuel
    /// thrown from the nozzle down the stream at a speed that carries it to
    /// the reach within its life, in the fire ramp - white-hot near the
    /// nozzle, ember-red further out. The stream is a jet of liquid, so the
    /// motes hug its line and fan out only as it blooms: the spread grows
    /// with the square of how far along they start. `along` is where on
    /// the stream it starts (0 at the nozzle, 1 at the reach): motes seeded
    /// along the length keep the stream full from the first frame, instead
    /// of one that visibly "arrives".
    fn flame_mote(&mut self, jet: &crate::simulation::FlameJet, along: f32) {
        let mut rng = rand::rng();
        let (from, dir, reach) = jet.drawn();
        let half = tuning().flame_half_angle_deg.to_radians() * (0.15 + 0.55 * along * along);
        let a = dir.y.atan2(dir.x) + rng.random_range(-half..half);
        // Reach the end of the cone in about a third of a second.
        let speed = (reach / 0.3) * rng.random_range(0.6..1.1);
        let start = reach * along;
        // Seeded across part of the stream's width at its start point,
        // not only on the centre line, so the body is filled rather than a
        // dotted line.
        let half_w = start * half.tan() * 0.5;
        let side = rng.random_range(-half_w..=half_w.max(0.01));
        let pos = Position::new(from.x + dir.x * start - dir.y * side, from.y + dir.y * start + dir.x * side);
        let life = ((reach - start) / speed).max(0.05) * rng.random_range(0.8..1.2);
        let tint = if along < 0.15 { WHITE_T } else if along < 0.55 { FIRE_T } else { EMBER_T };
        self.push(Particle {
            pos,
            vel: Vec2::new(a.cos() * speed, a.sin() * speed),
            z: 0.0,
            vz: 0.0,
            age: 0.0,
            life,
            // Two blocks through the body of the stream, one at the
            // white-hot root where it is narrowest.
            size: if along < 0.15 { FX_GRID } else { FX_GRID * 2.0 },
            tint,
            kind: ParticleKind::Ember,
        });
    }

    fn burst(&mut self, at: Position, kind: ParticleKind, n: i32, speed: f32, tints: &[Color]) {
        let mut rng = rand::rng();
        for _ in 0..n.max(0) {
            let a = rng.random_range(0.0..std::f32::consts::TAU);
            let s = speed * rng.random_range(0.35..1.0);
            // Sizes are whole blocks (see FX_GRID), not arbitrary radii:
            // a spark is one design pixel, a smoke puff starts at two.
            let (life, size, vz) = match kind {
                ParticleKind::Spark => (tuning().spark_lifetime, FX_GRID, 0.0),
                ParticleKind::Chip => (tuning().chip_lifetime, FX_GRID, -rng.random_range(60.0..160.0)),
                ParticleKind::Dust => (tuning().dust_lifetime, FX_GRID * 2.0, 0.0),
                ParticleKind::Smoke => (tuning().smoke_lifetime, FX_GRID * 2.0, 0.0),
                ParticleKind::Ember => (tuning().ember_lifetime, FX_GRID, 0.0),
                ParticleKind::Spray => (tuning().chip_lifetime, FX_GRID, -rng.random_range(50.0..130.0)),
                ParticleKind::Trail => (tuning().missile_trail_seconds, FX_GRID * 2.0, 0.0),
            };
            self.push(Particle {
                pos: at,
                vel: Vec2::new(a.cos() * s, a.sin() * s),
                z: 0.0,
                vz,
                age: 0.0,
                life: life * rng.random_range(0.6..1.3),
                size,
                tint: tints[rng.random_range(0..tints.len())],
                kind,
            });
        }
    }

    /// Like `burst`, but thrown forward: every particle leaves along `dir`
    /// (a unit vector) turned by up to `half` radians either way - a muzzle
    /// spitting sparks down the barrel's line, a laser's burn splashing
    /// back toward the gun.
    #[allow(clippy::too_many_arguments)]
    fn cone_burst(&mut self, at: Position, dir: Vec2, half: f32, kind: ParticleKind, n: i32, speed: f32, tints: &[Color]) {
        if tuning().fx_max_particles <= 0 || n <= 0 {
            return;
        }
        self.burst(at, kind, n, speed, tints);
        let mut rng = rand::rng();
        let base = dir.y.atan2(dir.x);
        // `push` appends and evicts from the front, so the burst is always
        // the last `n` particles.
        let from = self.particles.len().saturating_sub(n as usize);
        for p in &mut self.particles[from..] {
            let s = p.vel.length();
            let a = base + rng.random_range(-half..=half);
            p.vel = Vec2::new(a.cos() * s, a.sin() * s);
        }
    }

    /// A glint off a flying shot: one block of light in the shot's own
    /// colour, loosed backwards and sideways off its path so a bolt sheds a
    /// glittering wake.
    fn glint(&mut self, at: Position, back: Vec2, kind: ParticleKind, tint: Color) {
        let mut rng = rand::rng();
        let side = Vec2::new(-back.y, back.x) * rng.random_range(-30.0..30.0);
        self.push(Particle {
            pos: at,
            vel: back * rng.random_range(20.0..60.0) + side,
            z: 0.0,
            vz: 0.0,
            age: 0.0,
            life: tuning().spark_lifetime * rng.random_range(0.8..1.6),
            size: FX_GRID,
            tint,
            kind,
        });
    }

    /// One puff of a missile's smoke trail at `at`, drifting a touch so
    /// the line frays as it thins.
    fn trail_puff(&mut self, at: Position) {
        let mut rng = rand::rng();
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let s = rng.random_range(0.0..6.0);
        let tints = [WHITE_T, STONE_LT];
        self.push(Particle {
            pos: at,
            vel: Vec2::new(a.cos() * s, a.sin() * s),
            z: 0.0,
            vz: 0.0,
            age: 0.0,
            life: tuning().missile_trail_seconds * rng.random_range(0.75..1.25),
            size: FX_GRID * 2.0,
            tint: tints[rng.random_range(0..tints.len())],
            kind: ParticleKind::Trail,
        });
    }

    /// What a destroyed tile throws off, by material. The colours are the
    /// same families the rubble rows are drawn from, so the burst and the
    /// debris it leaves read as the same substance.
    fn tile_death(&mut self, material: Material, at: Position) {
        if self.burning.contains(&crate::map::world_to_cell(at)) {
            self.start_impact(at, Vec2::zero(), ImpactKind::Ash);
        } else if crate::pyro::dust_of(material).is_some() {
            self.start_impact(at, Vec2::zero(), ImpactKind::Collapse(material));
        }
        let n = self.count(tuning().tile_burst_particles);
        match material {
            Material::Brick | Material::Iron | Material::Volcano | Material::Door => {
                self.burst(at, ParticleKind::Chip, n, 90.0, &[STONE_LT, STONE_MD, STONE_DK]);
                self.burst(at, ParticleKind::Dust, self.count(6), 40.0, &[DUST_T]);
            }
            Material::Glass | Material::Lamp => {
                // Glass throws further, lighter and brighter than masonry.
                self.burst(at, ParticleKind::Chip, self.count(tuning().tile_burst_particles + 6), 150.0, &[GLASS_L, GLASS_M, WHITE_T]);
                self.burst(at, ParticleKind::Spark, self.count(4), 120.0, &[WHITE_T]);
            }
            Material::Wood | Material::Fence => {
                self.burst(at, ParticleKind::Chip, n, 100.0, &[WOOD_L, WOOD_M, WOOD_D]);
                self.burst(at, ParticleKind::Ember, self.count(4), 50.0, &[EMBER_T, FIRE_T]);
            }
            Material::Sandbag => {
                self.burst(at, ParticleKind::Dust, self.count(tuning().tile_burst_particles + 4), 55.0, &[SAND_L, SAND_M]);
            }
            Material::Barrel => {
                self.burst(at, ParticleKind::Spark, self.count(10), 170.0, &[FIRE_T, EMBER_T]);
                self.burst(at, ParticleKind::Smoke, self.count(5), 30.0, &[SMOKE_T]);
            }
            Material::Tree | Material::Pine => {
                // A tree coming down is mostly leaves - slow, drifting, and
                // far more of them than a wall throws chips - over a much
                // smaller spray of the timber underneath.
                self.burst(at, ParticleKind::Dust, self.count(tuning().tile_burst_particles + 8), 45.0, &[LEAF_L, LEAF_M, LEAF_D]);
                self.burst(at, ParticleKind::Chip, self.count(5), 95.0, &[WOOD_M, WOOD_D]);
            }
            // A tower comes apart as armour plate, sparks and smoke; its
            // own death (discharge, cook-off, spill) adds the rest.
            Material::Tesla | Material::GunTower | Material::BioSlush => {
                self.burst(at, ParticleKind::Chip, self.count(tuning().tile_burst_particles + 6), 120.0, &[STONE_LT, STONE_MD, STONE_DK]);
                self.burst(at, ParticleKind::Spark, self.count(10), 160.0, &[FIRE_T, EMBER_T, WHITE_T]);
                self.burst(at, ParticleKind::Smoke, self.count(6), 30.0, &[SMOKE_T]);
            }
        }
    }

    /// A shot that chipped a tile without killing it: a small spray of the
    /// material, scaled well under `tile_death`'s burst so a wall being
    /// worn down still reads as less than a wall coming apart.
    fn tile_chip(&mut self, material: Material, at: Position, dir: Option<Vec2>) {
        if crate::pyro::dust_of(material).is_some() {
            self.start_impact(at, dir.unwrap_or(Vec2::zero()), ImpactKind::Dust(material));
        }
        let n = self.count(tuning().tile_chip_particles);
        match material {
            Material::Glass => {
                self.burst(at, ParticleKind::Chip, n, 120.0, &[GLASS_L, GLASS_M, WHITE_T]);
                self.burst(at, ParticleKind::Spark, self.count(1), 90.0, &[WHITE_T]);
            }
            Material::Iron => {
                // Steel does not chip - it throws sparks.
                self.burst(at, ParticleKind::Spark, n, 140.0, &[FIRE_T, WHITE_T]);
            }
            Material::Wood | Material::Fence => {
                self.burst(at, ParticleKind::Chip, n, 80.0, &[WOOD_L, WOOD_M, WOOD_D]);
            }
            Material::Sandbag => {
                self.burst(at, ParticleKind::Dust, n, 45.0, &[SAND_L, SAND_M]);
            }
            Material::Tree | Material::Pine => {
                self.burst(at, ParticleKind::Dust, n, 55.0, &[LEAF_L, LEAF_M, LEAF_D]);
            }
            _ => {
                self.burst(at, ParticleKind::Chip, n, 85.0, &[STONE_LT, STONE_MD, STONE_DK]);
                self.burst(at, ParticleKind::Dust, self.count(2), 35.0, &[DUST_T]);
            }
        }
    }

    fn wreck(&mut self, at: Position) {
        self.burst(at, ParticleKind::Spark, self.count(tuning().wreck_burst_particles), 200.0, &[FIRE_T, EMBER_T, WHITE_T]);
        self.burst(at, ParticleKind::Chip, self.count(10), 120.0, &[STONE_DK, STONE_MD]);
        self.burst(at, ParticleKind::Smoke, self.count(8), 40.0, &[SMOKE_T]);
    }

    fn blast(&mut self, at: Position, chained: bool, drum: Drum) {
        // A cascade fires one of these per link, so a chained pop is
        // deliberately smaller - otherwise a barrel row saturates the cap
        // and the later blasts evict the earlier ones' particles.
        let scale = if chained { 0.5 } else { 1.0 };
        match drum {
            Drum::Oil => {
                self.burst(at, ParticleKind::Spark, self.count((18.0 * scale) as i32), 210.0, &[FIRE_T, EMBER_T]);
                self.burst(at, ParticleKind::Smoke, self.count((7.0 * scale) as i32), 35.0, &[SOOT_T]);
            }
            // A lava bomb: a spray of molten drops and grey ash.
            Drum::Lava => {
                self.burst(at, ParticleKind::Ember, self.count((16.0 * scale) as i32), 150.0, &[FIRE_T, EMBER_T]);
                self.burst(at, ParticleKind::Smoke, self.count((6.0 * scale) as i32), 30.0, &[SMOKE_T]);
            }
            // Fuel: whiter, faster, and hardly any smoke - it burns clean.
            Drum::Fuel => {
                self.burst(at, ParticleKind::Spark, self.count((22.0 * scale) as i32), 260.0, &[WHITE_T, FIRE_T]);
                self.burst(at, ParticleKind::Smoke, self.count((3.0 * scale) as i32), 40.0, &[SMOKE_T]);
            }
        }
    }

    // ---- the two halves of a frame -------------------------------------

    /// Read this frame's events and world state and emit from both. The
    /// event half is a no-op when `observe_events` already saw this
    /// simulation frame.
    pub fn observe(&mut self, game: &Game, dt: f32) {
        self.observe_events(game);
        self.sample_world(game, dt);
    }

    /// The bursts of the simulation frame `game` is at, once per frame:
    /// `Game::events` holds one `update`'s output, so a rendered frame that
    /// runs two steps calls this after each or the first step's bursts are
    /// gone before the draw. A frame already consumed emits nothing.
    pub fn observe_events(&mut self, game: &Game) {
        if game.frame() != self.last_frame {
            // A restart rewinds the counter, which is also the moment the
            // old round's particles should go.
            if game.frame() < self.last_frame {
                self.clear();
            }
            self.last_frame = game.frame();
            for e in game.events() {
                match *e {
                    Event::RoundStarted { .. } => self.clear(),
                    Event::PickupCollected { slot, kind, x, y, spilled } => {
                        self.start_open(CrateOpen { kind, at: Position::new(x, y), slot: Some(slot), spilled, age: 0.0 });
                    }
                    Event::CrateBroken { kind, x, y, .. } => {
                        let at = Position::new(x, y);
                        self.start_open(CrateOpen { kind, at, slot: None, spilled: false, age: 0.0 });
                        if let Some(ramp) = crate::pyro::dust_of(Material::Wood) {
                            let tints = [ramp[1], ramp[2]];
                            self.burst(at, ParticleKind::Dust, self.count(6), 50.0, &tints);
                        }
                    }
                    Event::ObstacleDestroyed { material, x, y } => self.tile_death(material, Position::new(x, y)),
                    // A hit the tile *survived*. Without this, a wall only
                    // ever throws anything on the shot that finishes it,
                    // and every shot before that lands silently.
                    Event::Hit { target: HitTarget::Obstacle { material }, killed: false, x, y, .. } => {
                        let at = Position::new(x, y);
                        self.flash(Flashed::Tile(at));
                        self.tile_chip(material, at, shot_heading_near(game, at));
                    }
                    Event::Wreck { x, y, .. } => {
                        self.wreck(Position::new(x, y));
                        self.splash_if_wet(game, Position::new(x, y), 14);
                    }
                    Event::Blast { x, y, chained, drum } => {
                        self.blast(Position::new(x, y), chained, drum);
                        self.splash_if_wet(game, Position::new(x, y), 18);
                    }
                    // A fuel drum leaving the ground: a spit of sparks and
                    // a puff where it stood.
                    Event::DrumLaunched { x, y, .. } => {
                        self.burst(Position::new(x, y), ParticleKind::Spark, self.count(8), 90.0, &[WHITE_T, FIRE_T]);
                        self.burst(Position::new(x, y), ParticleKind::Smoke, self.count(3), 20.0, &[SMOKE_T]);
                    }
                    // A tank through a portal: a blue flare where it
                    // vanished, a softer one plus settling motes where it
                    // appeared, so the eye is led from one to the other.
                    Event::Teleported { x, y, to_x, to_y, .. } => {
                        self.burst(Position::new(x, y), ParticleKind::Spark, self.count(14), 140.0, &[PORTAL_T, PORTAL_LT_T, WHITE_T]);
                        self.burst(Position::new(to_x, to_y), ParticleKind::Spark, self.count(10), 90.0, &[PORTAL_LT_T, WHITE_T]);
                        self.burst(Position::new(to_x, to_y), ParticleKind::Ember, self.count(6), 40.0, &[PORTAL_T]);
                    }
                    // A lava bomb thrown from the crater: a spit of molten
                    // drops and a puff of ash at the rim (docs/volcano.md).
                    Event::LavaBombLaunched { x, y, .. } => {
                        self.burst(Position::new(x, y), ParticleKind::Ember, self.count(6), 70.0, &[FIRE_T, EMBER_T]);
                        self.burst(Position::new(x, y), ParticleKind::Smoke, self.count(2), 18.0, &[SOOT_T]);
                    }
                    // A lantern set down catches with a few sparks; a
                    // broken one goes out in a spray of glass.
                    Event::LanternSet { x, y, .. } => {
                        self.burst(Position::new(x, y), ParticleKind::Spark, self.count(4), 30.0, &[FIRE_T, WHITE_T]);
                    }
                    Event::LanternBroken { x, y } => {
                        self.burst(Position::new(x, y), ParticleKind::Chip, self.count(8), 110.0, &[GLASS_L, GLASS_M, WHITE_T]);
                        self.burst(Position::new(x, y), ParticleKind::Spark, self.count(3), 60.0, &[FIRE_T]);
                    }
                    // Something the flamethrower lit or collapsed.
                    Event::Ignited { x, y, .. } => {
                        self.burst(Position::new(x, y), ParticleKind::Spark, self.count(6), 70.0, &[FIRE_T, WHITE_T]);
                    }
                    // Ground catching: a flare of embers as the fire takes.
                    Event::FireStarted { x, y, .. } => {
                        self.burst(Position::new(x, y), ParticleKind::Ember, self.count(5), 40.0, &[FIRE_T, EMBER_T]);
                    }
                    // A frog health pack collected: a lift of green
                    // sparks off the frog itself, which is wherever it
                    // happens to be standing and not where the pack was.
                    Event::FrogHealed { x, y, .. } => {
                        self.burst(Position::new(x, y), ParticleKind::Spark, self.count(10), 60.0, &[HEAL_T, WHITE_T]);
                        self.burst(Position::new(x, y), ParticleKind::Ember, self.count(4), 26.0, &[HEAL_T]);
                    }
                    // A seeker missile coming down: a tight fireball's
                    // worth of sparks and a puff, a volley's four in a row.
                    Event::MissileBlast { x, y, .. } => {
                        self.burst(Position::new(x, y), ParticleKind::Spark, self.count(10), 150.0, &[FIRE_T, EMBER_T, WHITE_T]);
                        self.burst(Position::new(x, y), ParticleKind::Smoke, self.count(4), 30.0, &[SMOKE_T]);
                        self.splash_if_wet(game, Position::new(x, y), 8);
                    }
                    Event::CookOff { x, y } => {
                        self.burst(Position::new(x, y), ParticleKind::Spark, self.count(8), 120.0, &[FIRE_T, EMBER_T]);
                    }
                    // A shot turned away by a rainbow shield: a small, cold
                    // scatter at the point of contact. Deliberately slight -
                    // this fires on every deflected shot, and the impact
                    // flash the hit loop already pushes is doing most of the
                    // work.
                    Event::Deflected { x, y, .. } => {
                        self.start_impact(Position::new(x, y), Vec2::zero(), ImpactKind::Deflect);
                        self.burst(Position::new(x, y), ParticleKind::Spark, self.count(4), 90.0, &[SHIELD_T, WHITE_T]);
                    }
                    // A shot glancing off iron or a barrel: a few hot
                    // sparks at the point of contact, as slight as the
                    // shield's, since the impact flash already marks it.
                    Event::Ricochet { x, y, .. } => {
                        self.start_impact(Position::new(x, y), Vec2::zero(), ImpactKind::Ricochet);
                        self.burst(Position::new(x, y), ParticleKind::Spark, self.count(3), 80.0, &[WHITE_T, FIRE_T]);
                    }
                    // The shield itself giving way: a full ring of sparks
                    // off the hull plus a little smoke, so the moment reads
                    // as the shield going rather than another hit landing.
                    // The camera shake rides `SHOCK_SHIELD_BREAK`, pushed in
                    // `simulation/`.
                    Event::ShieldBroken { x, y, .. } => {
                        self.burst(Position::new(x, y), ParticleKind::Spark, self.count(16), 150.0, &[SHIELD_T, WHITE_T]);
                        self.burst(Position::new(x, y), ParticleKind::Smoke, self.count(5), 34.0, &[SMOKE_T]);
                    }
                    // A shot landing on a hull, a frog or the border: a
                    // shower of hot sparks, a couple of dark flecks of
                    // armour and a wisp of smoke. Tiles are `tile_chip`'s.
                    Event::Hit { target: target @ (HitTarget::Player { .. } | HitTarget::Enemy { .. } | HitTarget::Frog { .. }), x, y, .. } => {
                        let at = Position::new(x, y);
                        match target {
                            HitTarget::Player { player } => self.flash(Flashed::Tank(player as usize)),
                            HitTarget::Enemy { slot } => self.flash(Flashed::Tank(slot)),
                            _ => {}
                        }
                        self.burst(at, ParticleKind::Spark, self.count(tuning().shot_hit_sparks), 190.0, &[WHITE_T, FIRE_T, EMBER_T]);
                        self.burst(at, ParticleKind::Chip, self.count(2), 90.0, &[STONE_DK, STONE_MD]);
                        self.burst(at, ParticleKind::Smoke, self.count(1), 18.0, &[SMOKE_T]);
                    }
                    Event::Hit { target: HitTarget::Wall, x, y, .. } => {
                        let at = Position::new(x, y);
                        self.burst(at, ParticleKind::Spark, self.count(tuning().shot_hit_sparks / 2), 150.0, &[WHITE_T, FIRE_T]);
                        self.burst(at, ParticleKind::Dust, self.count(2), 35.0, &[DUST_T]);
                    }
                    // A tesla bolt landing: a crackle of violet sparks off
                    // the hull it struck and a spit off the terminal.
                    Event::TeslaStrike { x0, y0, x1, y1, chained } => {
                        self.start_impact(Position::new(x1, y1), Vec2::new(x1 - x0, y1 - y0), ImpactKind::Tesla);
                        let n = if chained { 6 } else { 10 };
                        self.burst(Position::new(x1, y1), ParticleKind::Spark, self.count(n), 150.0, &[TESLA_T, TESLA_DEEP_T, WHITE_T]);
                        self.burst(Position::new(x0, y0), ParticleKind::Spark, self.count(3), 70.0, &[TESLA_T, WHITE_T]);
                    }
                    // A mortar's lob: a spit of ooze out of the mouth. A gun
                    // tower's burst is `muzzle_sparks`', off its flashes.
                    Event::TowerFired { kind: "bio_slush", x, y, heading } => {
                        let r = heading.to_radians();
                        let dir = Vec2::new(r.sin(), -r.cos());
                        self.cone_burst(Position::new(x, y), dir, 0.5, ParticleKind::Spray, self.count(5), 90.0, &OOZE_TINTS);
                    }
                    Event::GlobSplashed { x, y } => {
                        self.start_impact(Position::new(x, y), Vec2::zero(), ImpactKind::Ooze);
                        self.burst(Position::new(x, y), ParticleKind::Spray, self.count(14), 110.0, &OOZE_TINTS);
                        self.splash_if_wet(game, Position::new(x, y), 8);
                    }
                    Event::Slimed { slot } => {
                        if let Some(&(_, at)) = game.slimed().iter().find(|s| s.0 == slot) {
                            self.burst(at, ParticleKind::Spray, self.count(6), 60.0, &OOZE_TINTS);
                        }
                    }
                    // Ooze coming off in the water: a splash with a lime
                    // fleck or two in it.
                    Event::SlimeWashed { slot } => {
                        if let Some(&(_, at, _)) = game.wading().iter().find(|w| w.0 == slot) {
                            self.burst(at, ParticleKind::Spray, self.count(10), 80.0, &[WATER_L, WATER_M, WHITE_T]);
                            self.burst(at, ParticleKind::Spray, self.count(4), 60.0, &OOZE_TINTS);
                        }
                    }
                    // A tower pack at work: the rainbow the pack is painted
                    // in, lifting off the tower it mended.
                    Event::TowerRepaired { x, y, .. } => {
                        self.burst(Position::new(x, y), ParticleKind::Spark, self.count(18), 110.0, &RAINBOW_TINTS);
                        self.burst(Position::new(x, y), ParticleKind::Ember, self.count(6), 36.0, &RAINBOW_TINTS);
                    }
                    // A laser's burn: sparks in the beam's colour splashing
                    // back off whatever stopped it.
                    Event::LaserBeam { x0, y0, x1, y1, variant, .. } => {
                        let (dx, dy) = (x0 - x1, y0 - y1);
                        let len = (dx * dx + dy * dy).sqrt();
                        if len > 0.5 {
                            let tint = if variant == "blue" { LASER_BLUE_T } else { LASER_RED_T };
                            let back = Vec2::new(dx / len, dy / len);
                            self.start_impact(Position::new(x1, y1), back * -1.0, ImpactKind::Laser(variant == "blue"));
                            self.cone_burst(Position::new(x1, y1), back, 1.2, ParticleKind::Spark, self.count(tuning().shot_hit_sparks), 170.0, &[tint, WHITE_T]);
                        }
                    }
                    _ => {}
                }
            }
            self.muzzle_sparks(game);
        }
    }

    /// Every muzzle flash the frame just lit: sparks spat down the line of
    /// the shot that left it, and a wisp of gun smoke. The flash itself
    /// carries no heading, so the shot is looked up - the nearest round
    /// still at the barrel - and a flash with none near (a missile launch)
    /// throws its sparks all round. A flamethrower nozzle lights one every
    /// held frame and has its own stream, so it is left out.
    fn muzzle_sparks(&mut self, game: &Game) {
        let n = self.count(tuning().muzzle_sparks);
        if n <= 0 {
            return;
        }
        let fresh: Vec<Position> = game.muzzle_flashes.iter().filter(|f| f.time == 0.0).map(|f| f.center).collect();
        for at in fresh {
            if game.flames().iter().any(|jet| jet.nozzle.distance_to(at) < 6.0) {
                continue;
            }
            match shot_heading_near(game, at) {
                Some(dir) => self.cone_burst(at, dir, 0.45, ParticleKind::Spark, n, 220.0, &[WHITE_T, FIRE_T]),
                None => self.burst(at, ParticleKind::Spark, n, 120.0, &[WHITE_T, FIRE_T]),
            }
            self.burst(at, ParticleKind::Smoke, self.count(1), 14.0, &[SMOKE_T]);
        }
    }

    /// Start a hit for every shell, bullet and plasma bolt that has just
    /// reached its impact frames. Watched from the world rather than the
    /// event log, so a replica - which sees shots, not `Hit` events for
    /// each - starts the same bursts; keyed by projectile id so a hit
    /// starts once however many frames its impact state is seen.
    fn watch_impacts(&mut self, game: &Game) {
        let mut hits: Vec<(u32, Position, Vec2, ImpactKind)> = Vec::new();
        let mut live: HashSet<u32> = HashSet::new();
        for s in game.world.query::<&Shell>().iter() {
            live.insert(s.id);
            if matches!(s.state, ShellState::Hit0 | ShellState::Hit1 | ShellState::Hit2) {
                hits.push((s.id, s.position, s.velocity, ImpactKind::Shell));
            }
        }
        for b in game.world.query::<&Bullet>().iter() {
            live.insert(b.id);
            if b.state == crate::bullet::BulletState::Hit {
                hits.push((b.id, b.position, b.velocity, ImpactKind::Bullet));
            }
        }
        for p in game.world.query::<&Plasma>().iter() {
            live.insert(p.id);
            if p.impact_progress().is_some() {
                hits.push((p.id, p.position, p.velocity, ImpactKind::Plasma(p.variant)));
            }
        }
        for (id, pos, vel, kind) in hits {
            if self.impacts_seen.insert(id) {
                self.start_impact(pos, vel, kind);
            }
        }
        self.impacts_seen.retain(|id| live.contains(id));
    }

    /// Continuous emitters. These are *states*, not events - a tile is
    /// burning for a second and a half, not at one instant - so they are
    /// sampled from the world every render frame and rate-limited through
    /// `accum`, which keeps the output smooth at any frame rate.
    /// A blast or a death on water throws water instead of leaving a
    /// scorch (docs/water.md).
    fn splash_if_wet(&mut self, game: &Game, at: Position, n: i32) {
        if game.water().depth_at(at).is_wet() {
            self.burst(at, ParticleKind::Spray, self.count(n), 130.0, &[WATER_L, WATER_M, WHITE_T]);
        }
    }

    fn sample_world(&mut self, game: &Game, dt: f32) {
        self.clock = game.time;
        self.watch_impacts(game);
        // Spray off a wading hull (docs/water.md): a splash the frame it
        // wades in, then droplets at a rate that follows its speed.
        let spray = tuning().water_spray_rate;
        let mut wading_now = HashSet::new();
        for (slot, pos, speed) in game.wading() {
            wading_now.insert(slot);
            if !self.wading.contains(&slot) {
                self.burst(pos, ParticleKind::Spray, self.count(10), 80.0, &[WATER_L, WATER_M, WHITE_T]);
            }
            if spray > 0.0 && speed > 15.0 {
                let rate = spray * (speed / 120.0).clamp(0.2, 1.5) * tuning().fx_density;
                if self.due(0x5A7E_0000 ^ slot as u32, rate, dt) {
                    self.burst(pos, ParticleKind::Spray, 1, 50.0, &[WATER_L, WATER_M]);
                }
            }
        }
        self.wading = wading_now;

        let (ember_rate, smoke_rate) = (tuning().wood_ember_rate, tuning().wood_smoke_rate);
        self.burning = game.burning_tiles().iter().map(|&(pos, _)| crate::map::world_to_cell(pos)).collect();
        for (pos, _elapsed) in game.burning_tiles() {
            let key = crate::blast::seed_at(pos, 1);
            if self.due(key, ember_rate * tuning().fx_density, dt) {
                self.burst(pos, ParticleKind::Ember, 1, 22.0, &[EMBER_T, FIRE_T]);
            }
            if self.due(key ^ 0x9e37, smoke_rate * tuning().fx_density, dt) {
                self.burst(pos, ParticleKind::Smoke, 1, 12.0, &[SMOKE_T]);
            }
        }
        // Burning ground - a pool or a lit trail - throws what a burning
        // plank does, dying down over its last moments.
        for (pos, left, _total) in game.burning_cells() {
            let key = crate::blast::seed_at(pos, 6);
            let dying = (left / 0.6).clamp(0.2, 1.0);
            if self.due(key, ember_rate * dying * tuning().fx_density, dt) {
                self.burst(pos, ParticleKind::Ember, 1, 24.0, &[EMBER_T, FIRE_T]);
            }
            if self.due(key ^ 0x2b7e, smoke_rate * dying * tuning().fx_density, dt) {
                self.burst(pos, ParticleKind::Smoke, 1, 12.0, &[SOOT_T]);
            }
        }
        // Lava throws motes: single sparks off its surface that rise and
        // cool down the fire ramp, more in the surge; an erupting crater
        // throws a stream of them (docs/volcano.md).
        if !game.lava().is_empty() || !game.volcanoes().is_empty() {
            let t = tuning();
            let surge = game.lava_look().surge;
            let rate = t.lava_mote_rate * (1.0 + 2.0 * surge) * t.fx_density;
            if rate > 0.0 {
                for (col, row) in game.lava().cells() {
                    let pos = crate::map::cell_to_world(col, row);
                    if self.due(crate::blast::seed_at(pos, 12), rate, dt) {
                        self.burst(pos, ParticleKind::Ember, 1, 18.0, &[EMBER_T, FIRE_T]);
                    }
                }
            }
            for v in game.volcanoes() {
                let phase = v.phase(game.time, &t);
                let key = crate::blast::seed_at(v.centre(), 13);
                if self.due(key, t.lava_mote_rate * 8.0 * phase.heat * t.fx_density, dt) {
                    self.burst(v.centre(), ParticleKind::Ember, 1, 30.0 + 60.0 * phase.heat, &[EMBER_T, FIRE_T, WHITE_T]);
                }
            }
        }
        // Towers: a damaged one leaks smoke (and sparks, at its last
        // stage), a burning one throws what a burning plank does, a ruin
        // smoulders for `tower_ruin_smoke_seconds`, dying down as it goes.
        for view in game.tower_views() {
            let key = crate::blast::seed_at(view.position, 8);
            let top = Position::new(view.position.x, view.position.y - 6.0);
            let smoke = match view.stage {
                0 | 1 => 0.0,
                2 => 0.4,
                _ => 1.0,
            } + if view.burning { 1.0 } else { 0.0 };
            if smoke > 0.0 && self.due(key, smoke_rate * smoke * tuning().fx_density, dt) {
                self.burst(top, ParticleKind::Smoke, 1, 12.0, &[SMOKE_T]);
            }
            let sparks = if view.stage >= 3 { 0.5 } else { 0.0 } + if view.burning { 1.0 } else { 0.0 };
            if sparks > 0.0 && self.due(key ^ 0x7a7a, ember_rate * sparks * tuning().fx_density, dt) {
                let tints: &[Color] = if view.burning { &[EMBER_T, FIRE_T] } else { &[WHITE_T, FIRE_T] };
                self.burst(top, if view.burning { ParticleKind::Ember } else { ParticleKind::Spark }, 1, 30.0, tints);
            }
        }
        let ruin_seconds = tuning().tower_ruin_smoke_seconds.max(0.01);
        for ruin in game.tower_ruins().iter().filter(|r| r.smouldering()) {
            let key = crate::blast::seed_at(ruin.position, 9);
            let dying = (1.0 - ruin.age / ruin_seconds).clamp(0.2, 1.0);
            if self.due(key, smoke_rate * dying * tuning().fx_density, dt) {
                self.burst(ruin.position, ParticleKind::Smoke, 1, 12.0, &[SMOKE_T]);
            }
        }
        // A coated hull drips ooze.
        for (slot, pos) in game.slimed() {
            if self.due(0x0DE0_0000 ^ slot as u32, 4.0 * tuning().fx_density, dt) {
                self.burst(pos, ParticleKind::Spray, 1, 30.0, &OOZE_TINTS);
            }
        }
        // A drum with its fuse lit spits sparks from the bung: the tell
        // that it is about to go, next to the lit lid and the rocking.
        let spark_rate = tuning().barrel_fuse_spark_rate;
        if spark_rate > 0.0 {
            for pos in game.fused_barrels() {
                let key = crate::blast::seed_at(pos, 7);
                if self.due(key, spark_rate * tuning().fx_density, dt) {
                    let bung = Position::new(pos.x + 4.0, pos.y - 8.0);
                    self.burst(bung, ParticleKind::Spark, 1, 45.0, &[FIRE_T, WHITE_T]);
                }
            }
        }
        // The flamethrower's stream. Rate-limited per nozzle like every
        // other emitter, but a stream at 90 motes a second owes more than
        // one per frame, so it drains its accumulator rather than taking
        // one; capped so a hitch cannot dump a second's worth at once.
        let stream_rate = tuning().flame_particle_rate * tuning().fx_density;
        if stream_rate > 0.0 {
            for (i, jet) in game.flames().iter().enumerate() {
                let key = 0xF1A3_0000 ^ i as u32;
                let mut n = 0;
                while n < 16 && self.due(key, stream_rate, dt) {
                    let along = rand::rng().random_range(0.0..0.8);
                    self.flame_mote(jet, along);
                    n += 1;
                }
                let (from, dir, reach) = jet.drawn();
                // Sparks spat off the tip, arcing on past the reach.
                if self.due(key ^ 0x3c3c, stream_rate * 0.06, dt) {
                    let tip = Position::new(from.x + dir.x * reach * 0.8, from.y + dir.y * reach * 0.8);
                    self.cone_burst(tip, dir, 0.6, ParticleKind::Spark, 1, 160.0, &[WHITE_T, FIRE_T]);
                }
                // Smoke off the far end, where the fire has burnt out.
                if self.due(key ^ 0x5a5a, stream_rate * 0.12, dt) {
                    let end = Position::new(from.x + dir.x * reach, from.y + dir.y * reach);
                    self.burst(end, ParticleKind::Smoke, 1, 16.0, &[SMOKE_T]);
                }
            }
        }
        // Flying shots shed light: a plasma bolt glitters in its own
        // colour, a shell throws the odd ember off its tracer.
        let glint = tuning().shot_trail_glint_rate * tuning().fx_density;
        if glint > 0.0 {
            let mut lit: Vec<(u32, Position, Vec2, ParticleKind, Color)> = Vec::new();
            for plasma in game.world.query::<&Plasma>().iter() {
                if plasma.state == PlasmaState::Flying {
                    let tint = match plasma.variant {
                        PlasmaVariant::Teal => PLASMA_TEAL_T,
                        PlasmaVariant::Purple => PLASMA_PURPLE_T,
                    };
                    lit.push((0x9A5A_0000 ^ plasma.id, plasma.position, back_of(plasma.velocity), ParticleKind::Spark, tint));
                }
            }
            for shell in game.world.query::<&Shell>().iter() {
                if shell.state == ShellState::Flying {
                    lit.push((0x5E11_0000 ^ shell.id, shell.position, back_of(shell.velocity), ParticleKind::Ember, FIRE_T));
                }
            }
            for (key, at, back, kind, tint) in lit {
                let rate = if kind == ParticleKind::Ember { glint * 0.5 } else { glint };
                if self.due(key, rate, dt) {
                    self.glint(at, back, kind, tint);
                }
            }
        }
        // Seeker missiles lay a smoke trail through the air: a puff every
        // `missile_trail_spacing` px of flight, filled in along the whole
        // segment since the last one so a fast missile draws a line, not
        // dots. A spark off the motor now and then.
        let spacing = tuning().missile_trail_spacing;
        let mut flying = HashSet::new();
        for (id, tail, _lift) in game.missiles() {
            flying.insert(id);
            if spacing <= 0.0 {
                continue;
            }
            let from = *self.trail_last.entry(id).or_insert(tail);
            let d = from.distance_to(tail);
            let steps = ((d / spacing) as usize).min(64);
            for k in 1..=steps {
                let at = from + (tail - from) * (k as f32 * spacing / d);
                self.trail_puff(at);
            }
            if steps > 0 {
                self.trail_last.insert(id, from + (tail - from) * (steps as f32 * spacing / d));
            }
            if self.due(0x3155_0000 ^ id, 8.0 * tuning().fx_density, dt) {
                self.burst(tail, ParticleKind::Spark, 1, 40.0, &[FIRE_T, EMBER_T]);
            }
        }
        self.trail_last.retain(|id, _| flying.contains(id));
        // A hull with afterburn on it burns like a wreck does, until the
        // timer runs out.
        let (flame_rate, wsmoke_rate) = (tuning().wreck_flame_rate, tuning().wreck_smoke_rate);
        for (pos, _left) in game.burning_tanks() {
            let key = crate::blast::seed_at(pos, 8);
            if self.due(key, flame_rate * tuning().fx_density, dt) {
                self.burst(pos, ParticleKind::Ember, 1, 30.0, &[FIRE_T, EMBER_T]);
            }
            if self.due(key ^ 0x7c3d, wsmoke_rate * 0.6 * tuning().fx_density, dt) {
                self.burst(pos, ParticleKind::Smoke, 1, 14.0, &[SMOKE_T]);
            }
        }
        for (pos, _left) in game.burning_wrecks() {
            let key = crate::blast::seed_at(pos, 2);
            if self.due(key, flame_rate * tuning().fx_density, dt) {
                self.burst(pos, ParticleKind::Ember, 1, 26.0, &[FIRE_T, EMBER_T]);
            }
            if self.due(key ^ 0x51ed, wsmoke_rate * tuning().fx_density, dt) {
                self.burst(pos, ParticleKind::Smoke, 1, 14.0, &[SOOT_T]);
            }
        }
        // A damaged hull smokes from its engine deck
        // (`damage_stage::smoke`): a wisp at the damaged tier, a column as
        // it gets worse, black smoke and embers once the deck burns.
        let hull_rate = tuning().hull_smoke_rate * tuning().fx_density;
        if hull_rate > 0.0 {
            let mut hulls: Vec<(usize, Position, f32, bool)> = Vec::new();
            for tank in game.world.query::<&crate::tank::Tank>().iter() {
                if let Some(thick) = crate::damage_stage::smoke(tank) {
                    hulls.push((tank.owner_slot(), crate::damage_stage::engine_deck(tank), thick, crate::damage_stage::fire(tank) > 0.0));
                }
            }
            for (slot, at, thick, burning) in hulls {
                let key = 0xDA3A_0000 ^ slot as u32;
                if self.due(key, hull_rate * (0.15 + 0.85 * thick), dt) {
                    let tint = if burning { SOOT_T } else { SMOKE_T };
                    self.burst(at, ParticleKind::Smoke, 1, 8.0, &[tint]);
                }
                if burning && self.due(key ^ 0x3e3e, flame_rate * 0.3 * tuning().fx_density, dt) {
                    self.burst(at, ParticleKind::Ember, 1, 22.0, &[FIRE_T, EMBER_T]);
                }
            }
        }
        // Flecks kicked up by a hull crossing tall grass - leaf on the
        // grass theme, straw on the desert's dry scrub. The only emitter
        // whose source is not something on fire or in contact -
        // `grass_disturbed` reports the cells a *moving* tank is in, so a
        // parked one rustles nothing.
        let rustle = tuning().grass_rustle_rate;
        if rustle > 0.0 {
            let flecks: &[Color] = match game.map.theme {
                crate::map::Theme::Grass => &[LEAF_L, LEAF_M, LEAF_D],
                crate::map::Theme::Desert => &[STRAW_L, STRAW_M, STRAW_D],
            };
            for pos in game.grass_disturbed() {
                if self.due(crate::blast::seed_at(pos, 3), rustle * tuning().fx_density, dt) {
                    self.burst(pos, ParticleKind::Dust, 1, 34.0, flecks);
                }
            }
        }
        // Impacts and scrapes. `max_impulse` is the solver's own measure of
        // how hard the contact is, so a gentle nudge against a wall stays
        // silent and a real slam throws sparks - the threshold is what
        // separates the two rather than any guess about speed.
        let (floor, spark_at) = (tuning().contact_fx_min_impulse, tuning().contact_fx_spark_impulse);
        for (at, impulse, on_tank) in game.contacts() {
            if impulse < floor {
                continue;
            }
            let key = crate::blast::seed_at(at, 4);
            let hard = impulse >= spark_at;
            // Rate scales with how hard it is, so a scrape trickles and a
            // slam sprays.
            let rate = tuning().contact_fx_rate * (impulse / spark_at).clamp(0.3, 2.5);
            if !self.due(key, rate * tuning().fx_density, dt) {
                continue;
            }
            if hard && on_tank {
                // Steel on steel.
                self.burst(at, ParticleKind::Spark, 2, 90.0, &[FIRE_T, WHITE_T]);
            } else if hard {
                self.burst(at, ParticleKind::Spark, 1, 70.0, &[FIRE_T, WHITE_T]);
                self.burst(at, ParticleKind::Dust, 1, 30.0, &[DUST_T]);
            } else {
                self.burst(at, ParticleKind::Dust, 1, 25.0, &[DUST_T]);
            }
        }

        // Grinding a prop under the tracks used to be completely silent.
        for (pos, _t) in game.ramming_tiles() {
            let key = crate::blast::seed_at(pos, 3);
            if self.due(key, tuning().ram_dust_rate * tuning().fx_density, dt) {
                self.burst(pos, ParticleKind::Dust, 1, 30.0, &[DUST_T, SAND_M]);
            }
        }
    }

    /// Rate limiter: true once per `1/rate` seconds for this source.
    fn due(&mut self, key: u32, rate: f32, dt: f32) -> bool {
        if rate <= 0.0 {
            return false;
        }
        let acc = self.accum.entry(key).or_insert(0.0);
        *acc += rate * dt;
        if *acc >= 1.0 {
            *acc -= 1.0;
            return true;
        }
        false
    }

    pub fn tick(&mut self, dt: f32) {
        self.impacts.retain_mut(|i| {
            i.age += dt;
            i.age < i.kind.seconds()
        });
        let open = crate::crate_fx::open_seconds(&tuning());
        self.opens.retain_mut(|o| {
            o.age += dt;
            o.age < open
        });
        let flash = tuning().hit_flash_seconds;
        self.flashes.retain_mut(|f| {
            f.age += dt;
            f.age < flash
        });
        let t = tuning();
        let (gravity, drag, bounce) = (t.debris_gravity, t.debris_air_drag, t.debris_bounce);
        let (rise, growth) = (t.smoke_rise_speed, t.smoke_growth);
        let clock = self.clock;
        self.particles.retain_mut(|p| {
            p.age += dt;
            if p.age >= p.life {
                return false;
            }
            match p.kind {
                ParticleKind::Chip => {
                    p.vz += gravity * dt;
                    p.z -= p.vz * dt;
                    if p.z <= 0.0 {
                        p.z = 0.0;
                        p.vz = -p.vz * bounce;
                        p.vel.x *= 0.55;
                        p.vel.y *= 0.55;
                    }
                }
                // Smoke and embers climb and drift down-wind as they go,
                // as far as the grass under them leans.
                ParticleKind::Smoke | ParticleKind::Ember => {
                    p.z += rise * dt;
                    p.size += growth * dt;
                    p.pos.x += crate::pyro::smoke_lean(&t, p.pos, clock) * rise * dt;
                }
                // Swells to about three blocks over its life, and drifts
                // with the wind where it hangs.
                ParticleKind::Trail => {
                    p.size += FX_GRID * 2.0 * dt / p.life.max(0.05);
                    p.pos.x += crate::pyro::smoke_lean(&t, p.pos, clock) * 10.0 * dt;
                }
                // Dust hugs the ground and blows along it.
                ParticleKind::Dust => p.pos.x += crate::pyro::smoke_lean(&t, p.pos, clock) * 16.0 * dt,
                ParticleKind::Spray => {
                    p.vz += gravity * dt;
                    p.z -= p.vz * dt;
                    // Landed: a droplet is gone the moment it meets the
                    // water again.
                    if p.z <= 0.0 && p.vz > 0.0 {
                        return false;
                    }
                }
                _ => {}
            }
            // Exponential, so the slowdown is frame-rate independent - the
            // same shape `drive_tank`'s deceleration uses.
            let k = (-drag * dt).exp();
            p.vel.x *= k;
            p.vel.y *= k;
            p.pos.x += p.vel.x * dt;
            p.pos.y += p.vel.y * dt;
            true
        });
        // The accumulators are keyed by source position, and sources go
        // away (a tile finishes burning). Drop stale ones rather than
        // growing the map for the whole round.
        if self.accum.len() > 256 {
            self.accum.clear();
        }
    }
}

/// The unit vector pointing back along `velocity`, or up for a shot at
/// rest.
fn back_of(velocity: Vec2) -> Vec2 {
    let len = velocity.length();
    if len > 0.01 { velocity * (-1.0 / len) } else { Vec2::new(0.0, 1.0) }
}

/// The heading of the shell, bullet or plasma bolt nearest `at` within
/// 28 px - the round a muzzle flash at `at` just let out.
pub(crate) fn shot_heading_near(game: &Game, at: Position) -> Option<Vec2> {
    shot_near(game, at).map(|(dir, _)| dir)
}

/// The round nearest `at` within 28 px - the one a muzzle flash at `at`
/// just let out - as its heading and, for a plasma bolt, its variant (so
/// its flash burns in its own colour).
pub(crate) fn shot_near(game: &Game, at: Position) -> Option<(Vec2, Option<PlasmaVariant>)> {
    let mut best: Option<(f32, Vec2, Option<PlasmaVariant>)> = None;
    let mut consider = |pos: Position, vel: Vec2, plasma: Option<PlasmaVariant>| {
        let d = pos.distance_to(at);
        let len = vel.length();
        if d < 28.0 && len > 0.01 && best.is_none_or(|(b, _, _)| d < b) {
            best = Some((d, vel * (1.0 / len), plasma));
        }
    };
    for s in game.world.query::<&Shell>().iter() {
        consider(s.position, s.velocity, None);
    }
    for b in game.world.query::<&Bullet>().iter() {
        consider(b.position, b.velocity, None);
    }
    for p in game.world.query::<&Plasma>().iter() {
        consider(p.position, p.velocity, Some(p.variant));
    }
    best.map(|(_, dir, plasma)| (dir, plasma))
}

/// Every sprite in the game lands on a 2-screen-pixel block (tanks draw a
/// 32px tile at scale 2, walls bake the same chunkiness into the art - see
/// CLAUDE.md), so particles have to as well or they read as a different,
/// smoother game drawn on top of this one. Three things do that here:
/// positions snap to the block grid, sizes are whole numbers of blocks,
/// and colour steps through a short ramp instead of fading an alpha
/// channel. A pixel-art flame is drawn, not blended.
pub(crate) const FX_GRID: f32 = 2.0;

/// Hits kept playing at once; a minigun burst into a wall is the case
/// that reaches it, and the oldest go first.
const MAX_IMPACTS: usize = 48;
/// The most crates being opened at once; the oldest goes first.
const MAX_OPENS: usize = 16;

// Particle tints. Deliberately literals rather than a palette import:
// these are light, not surface, and several are drawn additively where a
// snapped surface colour would read as muddy.
pub(crate) const STONE_LT: Color = Color::new(0xC1, 0xC1, 0xC1, 255);
pub(crate) const STONE_MD: Color = Color::new(0x9E, 0x9E, 0x96, 255);
const STONE_DK: Color = Color::new(0x7E, 0x7E, 0x7E, 255);
const WOOD_L: Color = Color::new(0xDE, 0x99, 0x43, 255);
const WOOD_M: Color = Color::new(0x99, 0x65, 0x24, 255);
const WOOD_D: Color = Color::new(0x50, 0x33, 0x0B, 255);
const SAND_L: Color = Color::new(0xC9, 0xB2, 0x66, 255);
const SAND_M: Color = Color::new(0xB7, 0xA2, 0x48, 255);
const GLASS_L: Color = Color::new(0x27, 0xD8, 0xC5, 255);
const GLASS_M: Color = Color::new(0x04, 0xA0, 0xB4, 255);
const DUST_T: Color = crate::pyro::DUST[3];
const SMOKE_T: Color = crate::pyro::SMOKE[2];
/// Black smoke off burning oil, a burning deck, a wreck: the smoke ramp a
/// step darker (`render/fx.rs` reads the tint).
pub(crate) const SOOT_T: Color = crate::pyro::SMOKE[1];

/// The light colours of the shots themselves, matched to their drawn
/// glows (`render/laser.rs`, `render/plasma.rs`).
const LASER_RED_T: Color = Color::new(0xFF, 0x50, 0x46, 255);
const LASER_BLUE_T: Color = Color::new(0x50, 0x9A, 0xFF, 255);
const PLASMA_TEAL_T: Color = Color::new(0x28, 0xDC, 0xC8, 255);
const PLASMA_PURPLE_T: Color = Color::new(0xB0, 0x6A, 0xF0, 255);
pub(crate) const EMBER_T: Color = Color::new(0xE4, 0x42, 0x19, 255);
pub(crate) const FIRE_T: Color = Color::new(0xEE, 0xA3, 0x43, 255);
pub(crate) const WHITE_T: Color = Color::new(0xFF, 0xFF, 0xFF, 255);
// The ground tileset's own water tones (`ground::WATER_FLOW_MARK` and the
// flat lake), so spray is the water it came out of.
const WATER_L: Color = Color::new(0x1D, 0xCC, 0xCB, 255);
const WATER_M: Color = Color::new(0x04, 0xA0, 0xB4, 255);
// Foliage. The one place particles are allowed green - it is the same
// exemption trees_sheet.png/nature_sheet.png take (`just check-sheets`):
// manufactured objects are never green, vegetation is.
const LEAF_L: Color = Color::new(0x7C, 0x98, 0x3C, 255);
const LEAF_M: Color = Color::new(0x5F, 0x91, 0x4B, 255);
const LEAF_D: Color = Color::new(0x1C, 0x4C, 0x33, 255);
// Dry grass (the desert theme). What a hull kicks out of a desert tuft is
// straw, not leaf: the SAND_* steps gen_grass.py builds a blade's tip, lit
// side and body from, so the flecks are the tuft's own colours leaving it.
const STRAW_L: Color = Color::new(0xD2, 0xBA, 0x6B, 255);
const STRAW_M: Color = Color::new(0xB7, 0xA2, 0x48, 255);
const STRAW_D: Color = Color::new(0x67, 0x51, 0x2A, 255);
// The frog health pack's burst. Green under the same exemption - the frog
// is the one living thing on the field, and `hud::FROG_COLOR` is what the
// bar already reads it in.
const HEAL_T: Color = Color::new(0x78, 0xDC, 0x5A, 255);
/// Rainbow-shield violet, matching `hud::SHIELD_COLOR` so a deflection and
/// the HUD gauge read as the same mechanic. Off the Puny Palette on purpose,
/// like the shield ring itself.
const SHIELD_T: Color = Color::new(0xAA, 0x78, 0xFF, 255);
/// The tesla bolt's violets (`render::tower`'s strands), off the palette
/// like the shield's.
const TESLA_T: Color = Color::new(0xCA, 0xA6, 0xFF, 255);
const TESLA_DEEP_T: Color = Color::new(0x9A, 0x66, 0xFF, 255);
/// The bio slush's ooze, the sheet's acid lime (`tower::OOZE_*`).
const OOZE_TINTS: [Color; 3] = [crate::tower::OOZE_HI, crate::tower::OOZE_LT, crate::tower::OOZE_MD];
/// The tower pack's rainbow, the colours its icon sweeps through.
const RAINBOW_TINTS: [Color; 6] = [
    Color::new(0xFF, 0x50, 0x46, 255),
    Color::new(0xFF, 0xA0, 0x3C, 255),
    Color::new(0xFF, 0xE6, 0x5A, 255),
    HEAL_T,
    PORTAL_T,
    SHIELD_T,
];
/// A portal's blues - the P1 team ramp, deliberately off-palette like the
/// portal sheet itself (docs/PALETTE.md).
const PORTAL_T: Color = Color::new(0x4D, 0x9B, 0xE6, 255);
const PORTAL_LT_T: Color = Color::new(0x8F, 0xD3, 0xFF, 255);

#[cfg(test)]
mod fx_tests {
    use super::*;

    fn spark(life: f32) -> Particle {
        Particle {
            pos: Position::new(0.0, 0.0),
            vel: Vec2::new(0.0, 0.0),
            z: 0.0,
            vz: 0.0,
            age: 0.0,
            life,
            size: 1.0,
            tint: WHITE_T,
            kind: ParticleKind::Spark,
        }
    }

    #[test]
    fn particles_expire_and_the_cap_holds() {
        let mut fx = Fx::default();
        let cap = tuning().fx_max_particles as usize;
        for _ in 0..cap + 50 {
            fx.push(spark(1.0));
        }
        assert_eq!(fx.live(), cap, "the cap holds under a flood");
        fx.tick(2.0);
        assert_eq!(fx.live(), 0, "and everything past its lifetime is dropped");
    }

    #[test]
    fn the_cap_evicts_the_oldest_first() {
        let mut fx = Fx::default();
        let cap = tuning().fx_max_particles as usize;
        // Fill with short-lived, then flood with long-lived: if eviction
        // took the newest, the survivors would be the short-lived ones.
        for _ in 0..cap {
            fx.push(spark(0.1));
        }
        for _ in 0..cap {
            fx.push(spark(9.0));
        }
        assert_eq!(fx.live(), cap);
        fx.tick(0.5);
        assert_eq!(fx.live(), cap, "the short-lived ones were the ones evicted");
    }

    #[test]
    fn a_chip_falls_bounces_and_settles() {
        let mut fx = Fx::default();
        let mut p = spark(9.0);
        p.kind = ParticleKind::Chip;
        p.vz = -120.0; // thrown upward
        fx.push(p);
        let mut peak: f32 = 0.0;
        for _ in 0..240 {
            fx.tick(1.0 / 60.0);
            if let Some(c) = fx.particles.first() {
                peak = peak.max(c.z);
            }
        }
        assert!(peak > 0.0, "it left the ground");
        let c = fx.particles.first().expect("still alive");
        assert!(c.z >= 0.0, "it never sinks below the ground");
        assert!(c.z < peak * 0.5, "and has come back down rather than floating");
    }

    #[test]
    fn the_rate_limiter_matches_its_rate() {
        let mut fx = Fx::default();
        // 20 per second over one second, stepped at 120fps.
        let hits = (0..120).filter(|_| fx.due(1, 20.0, 1.0 / 120.0)).count();
        assert!((19..=21).contains(&hits), "20/sec produced {hits} in a second");
        assert!(!fx.due(2, 0.0, 1.0), "a zero rate never fires");
    }
}
