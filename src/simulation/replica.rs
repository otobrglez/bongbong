//! The surface the online co-op code reads a round through and writes a
//! client replica through (`net::encode`, `net::apply`;
//! docs/online-coop-prd.md §4.2, §4.5): crate-private accessors over the
//! fields `Game` keeps to itself, and `DrawableState`, the definition of
//! "the same picture" - what a snapshot has to carry for a replica to
//! draw what the server draws, at the resolution it travels at. Nothing
//! here runs inside `update`, draws RNG or reaches a renderer.
//! `DrawableState` and its accessor are public so a bin can compare two
//! rounds the way the round-trip tests do.

use hecs::Entity;

use crate::ai::Ai;
use crate::bullet::Bullet;
use crate::frog::{Frog, Side};
use crate::math::Vec2;
use crate::obstacle::{Material, Obstacle};
use crate::physics::Physics;
use crate::pickup::{Pickup, PickupKind};
use crate::plasma::{Plasma, PlasmaVariant};
use crate::shell::Shell;
use crate::tank::{ActiveWeapon, Tank};
use crate::{FROG_ATTACK_SECONDS, FROG_EXPLOSION_FPS, FROG_EXPLOSION_FRAMES, FROG_HOP_SECONDS, FROG_HURT_SECONDS, Position};

use super::{Game, Outcome};

/// Which projectile a `DrawableShot` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ShotKind {
    Shell,
    Bullet,
    Plasma,
}

/// One hull as the picture needs it: the slot that keys it, its chassis,
/// where it stands on the quarter-pixel grid, which way it faces, its
/// health in whole points, the ring and overlay states, and the weapon the
/// HUD names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawableTank {
    pub slot: usize,
    /// `Some(index)` for a human player's tank.
    pub player: Option<u8>,
    pub row: i32,
    /// Quarter pixels.
    pub x: i32,
    /// Quarter pixels.
    pub y: i32,
    /// `Dir::index` of the facing.
    pub dir: u8,
    /// `Tank::hull_points`.
    pub hp: u8,
    pub wreck: bool,
    /// `Tank::shield_points`.
    pub shield: u8,
    pub boost: bool,
    pub burning: bool,
    /// The hit window is running.
    pub hit: bool,
    pub flame: bool,
    pub weapon: ActiveWeapon,
    /// Rounds left for `weapon`, saturated at 255.
    pub ammo: u8,
}

/// One projectile in flight or in its muzzle/impact frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawableShot {
    pub id: u32,
    pub kind: ShotKind,
    /// Quarter pixels.
    pub x: i32,
    /// Quarter pixels.
    pub y: i32,
    /// The facing in 256 steps around the turn.
    pub heading: u8,
    /// The sheet column of the choreography state.
    pub state: i32,
    /// The sprite variant (`Shell::variant`, a `PlasmaVariant`'s index,
    /// 0 for a bullet).
    pub variant: i32,
}

/// One frog: where it is, its health, which clips are playing and how far
/// the top-priority one has come.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawableFrog {
    pub side: Side,
    /// Quarter pixels.
    pub x: i32,
    /// Quarter pixels.
    pub y: i32,
    /// `Frog::health_points`.
    pub hp: u8,
    pub dead: bool,
    pub hopping: bool,
    pub hurt: bool,
    pub biting: bool,
    /// `Frog::clip_phase`.
    pub phase: u8,
    /// The hop's landing spot, quarter pixels; zero unless hopping.
    pub hop_x: i32,
    pub hop_y: i32,
}

/// One solid tile as it stands now: the fields a hit, a fire, a fuse, a
/// blast or a ram change, over its fixed material and variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawableTile {
    pub cell: (i32, i32),
    pub material: Material,
    pub variant: i32,
    /// Health in whole points.
    pub hp: u8,
    pub burning: bool,
    pub fused: bool,
    pub scorched: u8,
    /// `Obstacle::lean_strength`.
    pub lean: u8,
}

/// The whole picture at one tick, every family sorted by its key so two
/// states compare with `==`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawableState {
    pub tanks: Vec<DrawableTank>,
    pub shots: Vec<DrawableShot>,
    pub frogs: Vec<DrawableFrog>,
    /// Every pickup on the field: its cell and kind.
    pub pickups: Vec<((i32, i32), PickupKind)>,
    pub tiles: Vec<DrawableTile>,
    /// Every burning ground cell with its time left in tenths.
    pub fires: Vec<((i32, i32), u8)>,
    /// The wave (0 under the band plan), live enemies, tanks still to
    /// roll in, the intro banner's and the end screen's time left in
    /// tenths, and the outcome.
    pub wave: u32,
    pub alive: usize,
    pub pending: usize,
    pub intro_tenths: u8,
    pub restart_tenths: u8,
    pub outcome: Outcome,
}

/// Pixels to quarter pixels, the picture's grid.
pub fn quarter_px(v: f32) -> i32 {
    (v * 4.0).round() as i32
}

/// Seconds to tenths, saturated at 255.
pub fn tenths(seconds: f32) -> u8 {
    (seconds * 10.0).round().clamp(0.0, 255.0) as u8
}

/// Degrees to one of 256 steps around the turn.
pub fn heading_step(degrees: f32) -> u8 {
    if !degrees.is_finite() {
        return 0;
    }
    ((degrees.rem_euclid(360.0) / 360.0 * 256.0).round() as u32 % 256) as u8
}

impl Tank {
    /// Shield charge in whole points: 0 without a live shield, at least 1
    /// with one (so a shield down to its last fraction still reads as up),
    /// at most 255.
    pub fn shield_points(&self) -> u8 {
        if !self.is_shielded() {
            return 0;
        }
        self.shield_hp.round().clamp(1.0, 255.0) as u8
    }

    /// Rounds left for the live weapon (`active_weapon`), saturated at 255.
    pub fn active_ammo(&self) -> u8 {
        self.weapon_ammo(self.active_weapon()).clamp(0, 255) as u8
    }
}

impl Frog {
    /// How long the clip on show has run over its own length, 0..=255:
    /// the death sequence, else the hop, else the bite, else the hurt
    /// flicker (the priority `anim` draws by); 0 while idling.
    pub fn clip_phase(&self) -> u8 {
        let fraction = if let Some(elapsed) = self.death_elapsed {
            let length = FROG_EXPLOSION_FRAMES as f32 / FROG_EXPLOSION_FPS;
            (elapsed / length).min(1.0)
        } else if self.hop_timer > 0.0 {
            1.0 - self.hop_timer / FROG_HOP_SECONDS
        } else if self.attack_timer > 0.0 {
            1.0 - self.attack_timer / FROG_ATTACK_SECONDS
        } else if self.hurt_timer > 0.0 {
            1.0 - self.hurt_timer / FROG_HURT_SECONDS
        } else {
            0.0
        };
        (fraction.clamp(0.0, 1.0) * 255.0).round() as u8
    }

    /// Put the clip on show `phase` of the way through, the inverse of
    /// `clip_phase` for the flags a snapshot carries: each clip that is
    /// on gets its timer, the top-priority one from `phase` and the others
    /// at their full length (the wire says they run, not how far).
    pub fn set_clips(&mut self, dead: bool, hopping: bool, biting: bool, hurt: bool, phase: u8) {
        let fraction = phase as f32 / 255.0;
        let length = FROG_EXPLOSION_FRAMES as f32 / FROG_EXPLOSION_FPS;
        self.death_elapsed = dead.then_some(fraction * length);
        let mut top = !dead;
        let mut timer = |on: bool, full: f32| -> f32 {
            if !on {
                return 0.0;
            }
            // The clip on show keeps a hair of time left at phase 255,
            // so it still reads as running.
            let value = if top { ((1.0 - fraction) * full).max(f32::EPSILON) } else { full };
            top = false;
            value
        };
        self.hop_timer = timer(hopping, FROG_HOP_SECONDS);
        self.attack_timer = timer(biting, FROG_ATTACK_SECONDS);
        self.hurt_timer = timer(hurt, FROG_HURT_SECONDS);
    }
}

impl Obstacle {
    /// How far a ram has pushed this prop or tree toward collapse, 0 for
    /// none then 1..=3 thirds of `Material::ram_seconds` (rounded, never
    /// below 1 while anything pushes) - the lean the picture shows.
    pub fn lean_strength(&self) -> u8 {
        let Some(limit) = self.material.ram_seconds() else { return 0 };
        if self.ram_timer <= 0.0 || limit <= 0.0 {
            return 0;
        }
        ((self.ram_timer / limit).clamp(0.0, 1.0) * 3.0).round().clamp(1.0, 3.0) as u8
    }

    /// Inverse of `lean_strength`: the ram timer that reads as `strength`.
    pub fn set_lean_strength(&mut self, strength: u8) {
        let limit = self.material.ram_seconds().unwrap_or(0.0);
        self.ram_timer = if strength == 0 { 0.0 } else { strength.min(3) as f32 / 3.0 * limit };
    }
}

impl Game {
    /// The rapier world, for the replica's body bookkeeping.
    pub(crate) fn physics_mut(&mut self) -> &mut Physics {
        &mut self.physics
    }

    /// Seconds left on the end screen's automatic restart - the number
    /// the banner counts down - and zero while the round is playing.
    ///
    /// Public because a room server is another crate and this is what it
    /// steers the end screen by: it ticks the round through the countdown
    /// like any other frame and ends the round on the tick before the one
    /// where `update` would call `init` and seat everyone in a round
    /// nobody asked for (docs/online-coop-prd.md §4.7).
    pub fn restart_countdown(&self) -> f32 {
        self.restart_timer
    }

    /// What the body does (`Physics::velocity`), or for a tank without a
    /// body - one rolling in - its kinematic `Tank::velocity`.
    pub(crate) fn body_velocity(&self, tank: &Tank) -> Vec2 {
        tank.body.map(|b| self.physics.velocity(b)).unwrap_or(tank.velocity)
    }

    /// The round's pickup slots in map order, the index the snapshot's
    /// pickup bitmask is over.
    pub(crate) fn pickup_slots(&self) -> &[(Position, PickupKind)] {
        &self.map_pickup_slots
    }

    /// Take the `Ai` off every enemy, which is what makes a `Game` a
    /// replica: no `.with::<&Ai>()` query sees them and nothing would think
    /// for them if `update` ever ran.
    pub(crate) fn strip_ai(&mut self) {
        let minds: Vec<Entity> = self.world.query::<(Entity, &Ai)>().iter().map(|(e, _)| e).collect();
        for entity in minds {
            self.world.remove_one::<Ai>(entity).ok();
        }
    }

    /// The picture at this tick. Every family sorted by its key; positions
    /// on the quarter-pixel grid, health in whole points, timers in
    /// tenths - the resolution the wire carries, so an authoritative round
    /// and its replica compare equal exactly when a client draws the same
    /// frame.
    pub fn drawable_state(&self) -> DrawableState {
        let mut tanks: Vec<DrawableTank> = self
            .world
            .query::<(Entity, &Tank)>()
            .iter()
            .map(|(entity, t)| DrawableTank {
                slot: t.owner_slot(),
                player: self.player_index(entity),
                row: t.row,
                x: quarter_px(t.position.x),
                y: quarter_px(t.position.y),
                dir: crate::tank::Dir::from_rotation(t.rotation).map_or(0, |d| d.index() as u8),
                hp: t.hull_points(),
                wreck: t.is_wreck(),
                shield: t.shield_points(),
                boost: t.speed_boost_timer > 0.0,
                burning: t.burn_timer > 0.0,
                hit: t.hit_flash_timer > 0.0,
                flame: t.flame_held,
                weapon: t.active_weapon(),
                ammo: t.active_ammo(),
            })
            .collect();
        tanks.sort_by_key(|t| t.slot);

        let mut shots: Vec<DrawableShot> = Vec::new();
        for s in self.world.query::<&Shell>().iter() {
            shots.push(DrawableShot {
                id: s.id,
                kind: ShotKind::Shell,
                x: quarter_px(s.position.x),
                y: quarter_px(s.position.y),
                heading: heading_step(s.rotation),
                state: s.state.col(),
                variant: s.variant,
            });
        }
        for b in self.world.query::<&Bullet>().iter() {
            shots.push(DrawableShot {
                id: b.id,
                kind: ShotKind::Bullet,
                x: quarter_px(b.position.x),
                y: quarter_px(b.position.y),
                heading: heading_step(b.rotation),
                state: b.state.col(),
                variant: 0,
            });
        }
        for p in self.world.query::<&Plasma>().iter() {
            shots.push(DrawableShot {
                id: p.id,
                kind: ShotKind::Plasma,
                x: quarter_px(p.position.x),
                y: quarter_px(p.position.y),
                heading: heading_step(p.rotation),
                state: p.state.col(),
                variant: plasma_variant_index(p.variant),
            });
        }
        shots.sort_by_key(|s| s.id);

        let mut frogs: Vec<DrawableFrog> = self
            .world
            .query::<&Frog>()
            .iter()
            .map(|f| {
                let hopping = f.hop_timer > 0.0;
                DrawableFrog {
                    side: f.side,
                    x: quarter_px(f.position.x),
                    y: quarter_px(f.position.y),
                    hp: f.health_points(),
                    dead: f.is_dead(),
                    hopping,
                    hurt: f.hurt_timer > 0.0,
                    biting: f.attack_timer > 0.0,
                    phase: f.clip_phase(),
                    hop_x: if hopping { quarter_px(f.hop_end.x) } else { 0 },
                    hop_y: if hopping { quarter_px(f.hop_end.y) } else { 0 },
                }
            })
            .collect();
        frogs.sort_by_key(|f| f.side == Side::Enemy);

        let mut pickups: Vec<((i32, i32), PickupKind)> = self
            .world
            .query::<&Pickup>()
            .iter()
            .map(|p| (crate::map::world_to_cell(p.position), p.kind))
            .collect();
        pickups.sort();

        let mut tiles: Vec<DrawableTile> = self
            .world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| !o.destroyed)
            .map(|o| DrawableTile {
                cell: o.cell(),
                material: o.material,
                variant: o.variant,
                hp: o.health.round().clamp(0.0, 255.0) as u8,
                burning: o.burning,
                fused: o.fuse.is_some(),
                scorched: o.scorched,
                lean: o.lean_strength(),
            })
            .collect();
        tiles.sort_by_key(|t| (t.cell.1, t.cell.0));

        let mut fires: Vec<((i32, i32), u8)> = self.fires.iter().map(|f| (f.cell, tenths(f.left))).collect();
        fires.sort();

        let wave = self.wave_status();
        DrawableState {
            tanks,
            shots,
            frogs,
            pickups,
            tiles,
            fires,
            wave: wave.map_or(0, |w| w.index),
            alive: wave.map_or(0, |w| w.alive),
            pending: wave.map_or(0, |w| w.pending),
            intro_tenths: tenths(self.intro_timer),
            restart_tenths: tenths(self.restart_timer),
            outcome: self.outcome,
        }
    }
}

/// A `PlasmaVariant` as the number it travels as (0 teal, 1 purple).
pub(crate) fn plasma_variant_index(variant: PlasmaVariant) -> i32 {
    match variant {
        PlasmaVariant::Teal => 0,
        PlasmaVariant::Purple => 1,
    }
}

/// Inverse of `plasma_variant_index`; anything else reads as teal.
pub(crate) fn plasma_variant_from_index(index: i32) -> PlasmaVariant {
    if index == 1 { PlasmaVariant::Purple } else { PlasmaVariant::Teal }
}
