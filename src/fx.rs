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

use std::collections::HashMap;

use rand::RngExt;
use sola_raylib::prelude::*;

use crate::obstacle::Material;
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
}

impl ParticleKind {
    fn additive(self) -> bool {
        matches!(self, ParticleKind::Spark | ParticleKind::Ember)
    }
}

pub struct Particle {
    pos: Position,
    vel: Vector2,
    /// Fake height above the ground plane. The game is top-down with no
    /// camera, so this is only a draw-time y-offset - the same trick
    /// `decal::Decal`'s arc uses.
    z: f32,
    vz: f32,
    age: f32,
    life: f32,
    size: f32,
    tint: Color,
    kind: ParticleKind,
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
}

impl Fx {
    pub fn live(&self) -> usize {
        self.particles.len()
    }

    /// Drop everything. Called when a new round starts - the `Fx` outlives
    /// `Game::init`, which knows nothing about it, so without this the
    /// previous round's smoke hangs over the new one's opening frame.
    pub fn clear(&mut self) {
        self.particles.clear();
        self.accum.clear();
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
            };
            self.push(Particle {
                pos: at,
                vel: Vector2::new(a.cos() * s, a.sin() * s),
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

    /// What a destroyed tile throws off, by material. The colours are the
    /// same families the rubble rows are drawn from, so the burst and the
    /// debris it leaves read as the same substance.
    fn tile_death(&mut self, material: Material, at: Position) {
        let n = self.count(tuning().tile_burst_particles);
        match material {
            Material::Brick | Material::Iron => {
                self.burst(at, ParticleKind::Chip, n, 90.0, &[STONE_LT, STONE_MD, STONE_DK]);
                self.burst(at, ParticleKind::Dust, self.count(6), 40.0, &[DUST_T]);
            }
            Material::Glass => {
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
        }
    }

    /// A shot that chipped a tile without killing it: a small spray of the
    /// material, scaled well under `tile_death`'s burst so a wall being
    /// worn down still reads as less than a wall coming apart.
    fn tile_chip(&mut self, material: Material, at: Position) {
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

    fn blast(&mut self, at: Position, chained: bool) {
        // A cascade fires one of these per link, so a chained pop is
        // deliberately smaller - otherwise a barrel row saturates the cap
        // and the later blasts evict the earlier ones' particles.
        let scale = if chained { 0.5 } else { 1.0 };
        self.burst(at, ParticleKind::Spark, self.count((18.0 * scale) as i32), 210.0, &[FIRE_T, EMBER_T]);
        self.burst(at, ParticleKind::Smoke, self.count((7.0 * scale) as i32), 35.0, &[SMOKE_T]);
    }

    // ---- the two halves of a frame -------------------------------------

    /// Read this frame's events and world state and emit from both.
    pub fn observe(&mut self, game: &Game, dt: f32) {
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
                    Event::ObstacleDestroyed { material, x, y } => self.tile_death(material, Position::new(x, y)),
                    // A hit the tile *survived*. Without this, a wall only
                    // ever throws anything on the shot that finishes it,
                    // and every shot before that lands silently.
                    Event::Hit { target: HitTarget::Obstacle { material }, killed: false, x, y, .. } => {
                        self.tile_chip(material, Position::new(x, y))
                    }
                    Event::Wreck { x, y, .. } => self.wreck(Position::new(x, y)),
                    Event::Blast { x, y, chained } => self.blast(Position::new(x, y), chained),
                    Event::CookOff { x, y } => {
                        self.burst(Position::new(x, y), ParticleKind::Spark, self.count(8), 120.0, &[FIRE_T, EMBER_T]);
                    }
                    _ => {}
                }
            }
        }
        self.sample_world(game, dt);
    }

    /// Continuous emitters. These are *states*, not events - a tile is
    /// burning for a second and a half, not at one instant - so they are
    /// sampled from the world every render frame and rate-limited through
    /// `accum`, which keeps the output smooth at any frame rate.
    fn sample_world(&mut self, game: &Game, dt: f32) {
        let (ember_rate, smoke_rate) = (tuning().wood_ember_rate, tuning().wood_smoke_rate);
        for (pos, _elapsed) in game.burning_tiles() {
            let key = crate::blast::seed_at(pos, 1);
            if self.due(key, ember_rate * tuning().fx_density, dt) {
                self.burst(pos, ParticleKind::Ember, 1, 22.0, &[EMBER_T, FIRE_T]);
            }
            if self.due(key ^ 0x9e37, smoke_rate * tuning().fx_density, dt) {
                self.burst(pos, ParticleKind::Smoke, 1, 12.0, &[SMOKE_T]);
            }
        }
        let (flame_rate, wsmoke_rate) = (tuning().wreck_flame_rate, tuning().wreck_smoke_rate);
        for (pos, _left) in game.burning_wrecks() {
            let key = crate::blast::seed_at(pos, 2);
            if self.due(key, flame_rate * tuning().fx_density, dt) {
                self.burst(pos, ParticleKind::Ember, 1, 26.0, &[FIRE_T, EMBER_T]);
            }
            if self.due(key ^ 0x51ed, wsmoke_rate * tuning().fx_density, dt) {
                self.burst(pos, ParticleKind::Smoke, 1, 14.0, &[SMOKE_T]);
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
        let t = tuning();
        let (gravity, drag, bounce) = (t.debris_gravity, t.debris_air_drag, t.debris_bounce);
        let (rise, growth) = (t.smoke_rise_speed, t.smoke_growth);
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
                ParticleKind::Smoke | ParticleKind::Ember => {
                    p.z += rise * dt;
                    p.size += growth * dt;
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

    /// Draw every particle. Non-additive kinds first in one run, then all
    /// the additive ones inside a single blend-mode block: a blend switch
    /// breaks raylib's batch, so interleaving them would cost one batch
    /// per particle instead of two for the whole layer.
    pub fn draw(&self, d: &mut impl RaylibDraw) {
        for p in self.particles.iter().filter(|p| !p.kind.additive()) {
            draw_particle(d, p);
        }
        if self.particles.iter().any(|p| p.kind.additive()) {
            d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
                for p in self.particles.iter().filter(|p| p.kind.additive()) {
                    draw_particle(&mut bd, p);
                }
            });
        }
    }
}

/// Every sprite in the game lands on a 2-screen-pixel block (tanks draw a
/// 32px tile at scale 2, walls bake the same chunkiness into the art - see
/// CLAUDE.md), so particles have to as well or they read as a different,
/// smoother game drawn on top of this one. Three things do that here:
/// positions snap to the block grid, sizes are whole numbers of blocks,
/// and colour steps through a short ramp instead of fading an alpha
/// channel. A pixel-art flame is drawn, not blended.
const FX_GRID: f32 = 2.0;

fn snap(v: f32) -> i32 {
    ((v / FX_GRID).floor() * FX_GRID) as i32
}

/// Fire cools as it ages: white-hot, then yellow, orange, deep red. Four
/// steps is enough to read as a flame and few enough to stay obviously
/// hand-picked rather than interpolated.
const FIRE_RAMP: [Color; 4] = [WHITE_T, FIRE_T, EMBER_T, DEEP_T];
/// Smoke lightens and thins as it rises and cools.
const SMOKE_RAMP: [Color; 3] = [SMOKE_DK, SMOKE_MD, SMOKE_LT];

fn ramp_pick(ramp: &[Color], t: f32) -> Color {
    let i = ((t * ramp.len() as f32) as usize).min(ramp.len() - 1);
    ramp[i]
}

fn draw_particle(d: &mut impl RaylibDraw, p: &Particle) {
    let t = (p.age / p.life).clamp(0.0, 1.0);
    let base = match p.kind {
        ParticleKind::Spark | ParticleKind::Ember => ramp_pick(&FIRE_RAMP, t),
        ParticleKind::Smoke => ramp_pick(&SMOKE_RAMP, t),
        // A chip or a dust mote keeps the colour of whatever it came off.
        _ => p.tint,
    };
    // Stepped, not smooth: four levels of transparency read as a pixel-art
    // dissolve, where a continuous fade reads as a soft airbrushed blob.
    let levels = 4.0;
    let fade = ((1.0 - t) * levels).ceil() / levels;
    let opacity = match p.kind {
        ParticleKind::Smoke => fade * tuning().smoke_opacity,
        // Fire holds full brightness and dies by stepping down the ramp
        // rather than by dimming.
        ParticleKind::Spark | ParticleKind::Ember => if t < 0.85 { 1.0 } else { 0.5 },
        _ => fade,
    };
    if opacity <= 0.0 {
        return;
    }
    let c = Color::new(base.r, base.g, base.b, (255.0 * opacity) as u8);
    // `z` is a straight y-offset - the game is top-down with no camera, so
    // height is just "further up the screen".
    let blocks = (p.size / FX_GRID).round().max(1.0);
    let side = (blocks * FX_GRID) as i32;
    let x = snap(p.pos.x - blocks * FX_GRID / 2.0);
    let y = snap(p.pos.y - p.z - blocks * FX_GRID / 2.0);
    d.draw_rectangle(x, y, side, side, c);
}

// Particle tints. Deliberately literals rather than a palette import:
// these are light, not surface, and several are drawn additively where a
// snapped surface colour would read as muddy.
const STONE_LT: Color = Color::new(0xC1, 0xC1, 0xC1, 255);
const STONE_MD: Color = Color::new(0x9E, 0x9E, 0x96, 255);
const STONE_DK: Color = Color::new(0x7E, 0x7E, 0x7E, 255);
const WOOD_L: Color = Color::new(0xDE, 0x99, 0x43, 255);
const WOOD_M: Color = Color::new(0x99, 0x65, 0x24, 255);
const WOOD_D: Color = Color::new(0x50, 0x33, 0x0B, 255);
const SAND_L: Color = Color::new(0xC9, 0xB2, 0x66, 255);
const SAND_M: Color = Color::new(0xB7, 0xA2, 0x48, 255);
const GLASS_L: Color = Color::new(0x27, 0xD8, 0xC5, 255);
const GLASS_M: Color = Color::new(0x04, 0xA0, 0xB4, 255);
const DUST_T: Color = Color::new(0xD8, 0xBF, 0x8E, 255);
const SMOKE_T: Color = Color::new(0x55, 0x52, 0x4E, 255);
const SMOKE_DK: Color = Color::new(0x37, 0x37, 0x37, 255);
const SMOKE_MD: Color = Color::new(0x55, 0x52, 0x4E, 255);
const SMOKE_LT: Color = Color::new(0x7E, 0x7E, 0x7E, 255);
const DEEP_T: Color = Color::new(0x81, 0x2F, 0x27, 255);
const EMBER_T: Color = Color::new(0xE4, 0x42, 0x19, 255);
const FIRE_T: Color = Color::new(0xEE, 0xA3, 0x43, 255);
const WHITE_T: Color = Color::new(0xFF, 0xFF, 0xFF, 255);
// Foliage. The one place particles are allowed green - it is the same
// exemption trees_sheet.png/nature_sheet.png take (`just check-sheets`):
// manufactured objects are never green, vegetation is.
const LEAF_L: Color = Color::new(0x7C, 0x98, 0x3C, 255);
const LEAF_M: Color = Color::new(0x5F, 0x91, 0x4B, 255);
const LEAF_D: Color = Color::new(0x1C, 0x4C, 0x33, 255);

#[cfg(test)]
mod fx_tests {
    use super::*;

    fn spark(life: f32) -> Particle {
        Particle {
            pos: Position::new(0.0, 0.0),
            vel: Vector2::new(0.0, 0.0),
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
