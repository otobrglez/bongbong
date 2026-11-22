//! Every simulation event's sound decision (docs/sound-system-prd.md
//! section 4.9): an exhaustive `match` over `Event` with no wildcard, so a
//! variant added to the simulation fails to compile here until it is given
//! a cue, handed to a state watcher, or marked silent with a reason - the
//! same guard `WireEvent::from_event` keeps for the wire.
//!
//! A binding names the cue it wants by its most specific id; the manifest
//! resolves it to the nearest one it has (`Manifest::resolve`).

use crate::math::Vec2;
use crate::obstacle::{Drum, Material};
use crate::simulation::{Event, Game, HitTarget, Outcome};
use crate::tank::{ActiveWeapon, ChargeEnd, Tank};
use crate::tower::TowerKind;
use crate::tuning::tuning;

use super::cues::ids;
use super::source::{self, FireSound};

/// Where a sound is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum At {
    Point(Vec2),
    /// On the tank in this owner slot, wherever it is when the sound starts.
    Slot(usize),
    /// Nowhere in particular: the UI, a stinger, an announcement.
    Nowhere,
}

/// One sound an event asks for.
#[derive(Clone, Debug, PartialEq)]
pub struct Play {
    pub cue: String,
    pub at: At,
    /// A level scale, 1 as the cue is mixed.
    pub strength: f32,
    /// A pitch scale on top of the cue's own.
    pub pitch: f32,
    /// Extra seconds before it starts (a twin barrel's second crack).
    pub after: f32,
    /// What made it, for the cue's cooldown and for telling its copies apart.
    pub source: u64,
}

impl Play {
    pub fn new(cue: impl Into<String>, at: At) -> Play {
        Play { cue: cue.into(), at, strength: 1.0, pitch: 1.0, after: 0.0, source: 0 }
    }

    pub fn strength(mut self, s: f32) -> Play {
        self.strength = s;
        self
    }

    pub fn pitch(mut self, p: f32) -> Play {
        self.pitch = p;
        self
    }

    pub fn after(mut self, seconds: f32) -> Play {
        self.after = seconds;
        self
    }

    pub fn source(mut self, source: u64) -> Play {
        self.source = source;
        self
    }
}

/// What an event sounds like.
#[derive(Clone, Debug, PartialEq)]
pub enum Binding {
    Play(Vec<Play>),
    /// A state watcher (`watch.rs`) plays it, from what the event changed.
    Watched(&'static str),
    /// No sound, and why.
    Silent(&'static str),
}

fn one(play: Play) -> Binding {
    Binding::Play(vec![play])
}

fn point(x: f32, y: f32) -> At {
    At::Point(Vec2::new(x, y))
}

fn cell(cell: (i32, i32)) -> At {
    At::Point(crate::map::cell_to_world(cell.0, cell.1))
}

fn key(a: u64, b: u64) -> u64 {
    a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b
}

/// The tank in owner slot `slot`, if it stands.
pub fn tank_in<'g>(game: &'g Game, slot: usize) -> Option<hecs::Ref<'g, Tank>> {
    let entity = game.world.query::<(hecs::Entity, &Tank)>().iter().find(|(_, t)| t.owner_slot() == slot).map(|(e, _)| e)?;
    game.world.get::<&Tank>(entity).ok()
}

pub fn bind(event: &Event, game: &Game) -> Binding {
    match *event {
        Event::RoundStarted { .. } => one(Play::new(ids::ROUND_START.0, At::Nowhere)),
        Event::RoundEnded { outcome } => match outcome {
            Outcome::Won => one(Play::new(ids::ROUND_WON.0, At::Nowhere)),
            Outcome::Lost => one(Play::new(ids::ROUND_LOST.0, At::Nowhere)),
            Outcome::Playing => Binding::Silent("a round that goes on"),
        },
        Event::Fired { slot, weapon } => fired(game, slot, weapon),
        Event::Placed { .. } => Binding::Silent("a seat placed by the dev tools or a restart"),
        Event::LaserBeam { x0, y0, leg, .. } => {
            if leg == 0 {
                one(Play::new("weapon.laser.fire", point(x0, y0)))
            } else {
                Binding::Silent("a beam's leg through a portal: its first leg sounded")
            }
        }
        Event::Shoved { .. } => Binding::Silent("the blow that shoved the tank sounds"),
        Event::Hit { cause, .. } if !cause.is_shot() => Binding::Silent("the blast, fire, lava or slime that did it sounds"),
        Event::Hit { target, damage, killed: _, x, y, cause } => {
            let shot = source::hit_shot(cause);
            let what = match target {
                HitTarget::Player { .. } | HitTarget::Enemy { .. } => "hull".to_string(),
                HitTarget::Bong { .. } => "bong".to_string(),
                HitTarget::Obstacle { material } => format!("{}.{}", source::surface_family(material), source::material_name(material)),
                HitTarget::Wall => "wall".to_string(),
            };
            let strength = (0.6 + damage / 40.0).clamp(0.5, 1.3);
            one(Play::new(format!("hit.{shot}.{what}"), point(x, y)).strength(strength).source(key(x.to_bits() as u64, y.to_bits() as u64)))
        }
        Event::SonicBlast { x, y, .. } => one(Play::new("weapon.sonic_hammer.fire", point(x, y))),
        Event::TellStarted { .. } => Binding::Watched("tell"),
        Event::EmpPulse { x, y, .. } => one(Play::new("weapon.emp_burst.fire", point(x, y))),
        Event::Disabled { x, y, .. } => one(Play::new("weapon.emp_burst.disabled", point(x, y))),
        Event::TowerDisabled { x, y } => one(Play::new("tower.disabled", point(x, y))),
        Event::MissileDud { x, y } => one(Play::new("weapon.missiles.dud", point(x, y))),
        Event::RailSlug { x0, y0, leg, overcharged, .. } => {
            if leg == 0 {
                let cue = if overcharged { "weapon.gauss_rail.fire.overcharged" } else { "weapon.gauss_rail.fire" };
                one(Play::new(cue, point(x0, y0)))
            } else {
                Binding::Silent("a slug's leg through a portal: its first leg sounded")
            }
        }
        Event::ChargeStarted { .. } => Binding::Watched("charge"),
        Event::ChargeEnded { slot, end, .. } => {
            let cue = match end {
                ChargeEnd::Fizzled => "weapon.gauss_rail.vent.fizzle",
                ChargeEnd::Vented => "weapon.gauss_rail.vent",
                ChargeEnd::Lapsed => "weapon.gauss_rail.vent.lapse",
            };
            one(Play::new(cue, At::Slot(slot)))
        }
        Event::DroneLaunched { x, y, .. } => one(Play::new("weapon.fpv_swarm.launch", point(x, y))),
        Event::DroneBurst { x, y, .. } => one(Play::new("weapon.fpv_swarm.burst", point(x, y))),
        Event::DroneDowned { x, y, .. } => one(Play::new("weapon.fpv_swarm.downed", point(x, y))),
        Event::DroneCrashed { x, y, .. } => one(Play::new("weapon.fpv_swarm.downed.crash", point(x, y))),
        Event::DroneLockLost { .. } => Binding::Silent("the drone flies on; its lock tone stops with the watcher"),
        Event::AirSupportCalled { .. } => one(Play::new("air.called", At::Nowhere)),
        Event::AirSupportArrived { x, y, .. } => one(Play::new("air.arrive", point(x, y))),
        Event::AirSalvo { x, y, .. } => one(Play::new("air.salvo", point(x, y))),
        Event::AirStrike { bomb, x, y, .. } => one(Play::new(if bomb { "air.bomb" } else { "air.cannon" }, point(x, y))),
        Event::AircraftDowned { x, y, .. } => one(Play::new("air.downed", point(x, y))),
        Event::AirSupportLeft { .. } => Binding::Watched("aircraft"),
        Event::RodCalled { cell: c, .. } => one(Play::new("weapon.rod_from_god.call", cell(c))),
        Event::RodImpact { cell: c, .. } => one(Play::new("weapon.rod_from_god.impact", cell(c))),
        Event::WellAnchored { x, y, .. } => one(Play::new("weapon.gravity_well.anchor", point(x, y))),
        Event::WellCollapsed { x, y, .. } => one(Play::new("weapon.gravity_well.collapse", point(x, y))),
        Event::Swallowed { what, x, y } => one(Play::new(source::swallow_cue(what), point(x, y))),
        Event::OrbFizzled { x, y, .. } => one(Play::new("weapon.gravity_well.fizzle", point(x, y))),
        Event::Wreck { slot, x, y } => one(Play::new("explosion.wreck", point(x, y)).source(slot as u64)),
        Event::Ram { slot, damage, .. } => one(Play::new(ids::RAM.0, At::Slot(slot)).strength((0.6 + damage / 8.0).clamp(0.6, 1.3)).source(slot as u64)),
        Event::PickupCollected { kind, x, y, .. } => one(Play::new(source::pickup_cue(kind), point(x, y))),
        Event::CrateBroken { cooked, x, y, .. } => one(Play::new(if cooked { "pickup.crate.cookoff" } else { "pickup.crate.break" }, point(x, y))),
        Event::PickupRespawned { x, y, .. } => one(Play::new("pickup.respawn", point(x, y))),
        Event::Deflected { x, y, .. } => one(Play::new("hit.deflect", point(x, y))),
        Event::Ricochet { x, y, .. } => one(Play::new("hit.ricochet", point(x, y))),
        Event::ShieldBroken { x, y, .. } => one(Play::new("hit.shield_break", point(x, y))),
        Event::ShellsCollided { x, y } => one(Play::new("hit.shells_collide", point(x, y))),
        Event::BongBite { slot, .. } => one(Play::new("bong.bite", At::Slot(slot))),
        Event::BongHealed { x, y, .. } => one(Play::new("bong.heal", point(x, y))),
        Event::BongTookPack { x, y, .. } => one(Play::new("bong.pack", point(x, y))),
        Event::WaveStarted { .. } => one(Play::new(ids::WAVE_START.0, At::Nowhere)),
        Event::TankEntered { .. } => Binding::Watched("engine"),
        Event::Rerolled { .. } => Binding::Silent("an AI respawn reroll, unseen"),
        Event::StragglersCalled { .. } => Binding::Silent("the director calling stragglers, unseen"),
        Event::WreckRemoved { .. } => Binding::Watched("wreck"),
        Event::ObstacleDestroyed { material, x, y } => one(Play::new(destroyed(material), point(x, y)).source(key(x.to_bits() as u64, y.to_bits() as u64))),
        Event::ChunksBroken { material, x, y, broken, collapsed, .. } => {
            let family = source::surface_family(material);
            if collapsed {
                one(Play::new(format!("destroy.{family}.collapse"), point(x, y)))
            } else {
                one(Play::new(format!("chunk.{family}"), point(x, y)).strength((0.6 + broken as f32 * 0.1).min(1.2)))
            }
        }
        Event::ChunksCharred { .. } => Binding::Watched("fire"),
        Event::EmberFlew { .. } => Binding::Watched("fire"),
        Event::GrassTrampled { .. } => Binding::Watched("engine"),
        Event::Blast { x, y, chained, drum } => {
            let cue = match drum {
                Drum::Oil => "explosion.barrel.oil",
                Drum::Fuel => "explosion.barrel.fuel",
                Drum::Lava => "explosion.lava_bomb",
            };
            let strength = if chained { crate::audio::dsp::db_to_gain(tuning().audio_chain_step_db) } else { 1.0 };
            one(Play::new(cue, point(x, y)).strength(strength))
        }
        Event::DrumLaunched { x, y, .. } => one(Play::new("explosion.drum_launch", point(x, y))),
        Event::LanternSet { x, y, .. } => one(Play::new("world.lantern.set", point(x, y))),
        Event::LanternBroken { x, y } => one(Play::new("world.lantern.break", point(x, y))),
        Event::LavaBombLaunched { x, y, .. } => one(Play::new("world.volcano.bomb", point(x, y))),
        Event::Teleported { to_x, to_y, .. } => one(Play::new("world.portal.teleport", point(to_x, to_y))),
        Event::ShotTeleported { to_x, to_y, .. } => one(Play::new("world.portal.shot", point(to_x, to_y))),
        Event::FireStarted { x, y, .. } => one(Play::new("world.fire.ignite", point(x, y)).source(key(x.to_bits() as u64, y.to_bits() as u64))),
        Event::Ignited { x, y, .. } => one(Play::new("world.fire.ignite", point(x, y)).source(key(x.to_bits() as u64, y.to_bits() as u64))),
        Event::OilPrinted { .. } => Binding::Silent("tread marks in oil, seen not heard"),
        Event::MissileLocked { target, .. } => match target {
            Some(target) => one(Play::new(ids::LOCK_ON.0, At::Slot(target)).source(target as u64)),
            None => Binding::Silent("a lock on nothing"),
        },
        Event::MissileBlast { x, y, .. } => one(Play::new("explosion.missile", point(x, y))),
        Event::GrenadeBlast { x, y, .. } => one(Play::new("explosion.grenade", point(x, y))),
        Event::CookOff { x, y } => one(Play::new("explosion.cookoff", point(x, y))),
        Event::TeslaStrike { x0, y0, x1, y1, chained } => {
            let mid = point(0.5 * (x0 + x1), 0.5 * (y0 + y1));
            let strength = if chained { 0.7 } else { 1.0 };
            one(Play::new(source::tower_cue(TowerKind::Tesla), mid).strength(strength))
        }
        Event::TowerFired { kind, x, y, .. } => match kind {
            "bio_slush" => one(Play::new(source::tower_cue(TowerKind::Bio), point(x, y))),
            _ => one(Play::new(source::tower_cue(TowerKind::Gun), point(x, y))),
        },
        Event::GlobSplashed { x, y } => one(Play::new("tower.bio.splash", point(x, y))),
        Event::Slimed { slot } => one(Play::new("tower.bio.slimed", At::Slot(slot))),
        Event::SlimeWashed { slot } => one(Play::new("tower.bio.washed", At::Slot(slot))),
        Event::TowerRepaired { x, y, .. } => one(Play::new("tower.repaired", point(x, y))),
        Event::BeatDone { .. } => one(Play::new("ui.beat", At::Nowhere)),
        Event::DoorOpened { x, y, .. } => one(Play::new("world.door", point(x, y))),
        Event::FlagTaken { x, y, .. } => one(Play::new("world.flag", point(x, y))),
        Event::BongRevived { x, y } => one(Play::new("bong.revive", point(x, y))),
        Event::PhysicsQuarantine { .. } => Binding::Silent("a physics guard, not a moment"),
        Event::AiAction { .. }
        | Event::EngageSlot { .. }
        | Event::StuckEscape { .. }
        | Event::Breach { .. }
        | Event::Retreat { .. }
        | Event::Alert { .. }
        | Event::Retarget { .. } => Binding::Silent("an AI trace, recorded only while tracing"),
    }
}

fn destroyed(material: Material) -> String {
    format!("destroy.{}.{}", source::surface_family(material), source::material_name(material))
}

/// A tank's shot: the cannon tiered by its calibre, with a twin barrel's
/// second crack a beat later; a weapon a watcher drives or another event
/// carries is not played here.
fn fired(game: &Game, slot: usize, weapon: &str) -> Binding {
    let Some(weapon) = ActiveWeapon::parse(weapon) else {
        return Binding::Silent("a weapon the sound maps do not know");
    };
    match source::weapon_sound(weapon) {
        FireSound::Cue(cue) => one(Play::new(cue, At::Slot(slot)).source(slot as u64)),
        FireSound::Watched(watcher) => Binding::Watched(watcher),
        FireSound::Elsewhere(_) => Binding::Silent("its own event carries the sound"),
        FireSound::Cannon => {
            let t = tuning();
            let (factor, mass, twin) = match tank_in(game, slot) {
                Some(tank) => (t.tank_damage_factor[tank.row as usize], tank.mass_factor(), tank.twin_gun()),
                None => (1.0, 1.0, false),
            };
            let cue = source::cannon_cue(factor);
            let pitch = source::engine_pitch(mass);
            let first = Play::new(cue, At::Slot(slot)).pitch(pitch).source(slot as u64);
            if twin {
                let second = first.clone().after(t.tank_twin_shot_delay_seconds).strength(0.85).source(slot as u64 + 1_000);
                Binding::Play(vec![first, second])
            } else {
                one(first)
            }
        }
    }
}
