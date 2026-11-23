//! Every simulation event's sound decision (docs/sound-system-prd.md
//! section 4.9): an exhaustive `match` over `Event` with no wildcard, so a
//! variant added to the simulation fails to compile here until it is given
//! a cue, handed to a state watcher, or marked silent with a reason - the
//! same guard `WireEvent::from_event` keeps for the wire.
//!
//! A binding names the cue it wants by its most specific id; the manifest
//! resolves it to the nearest one it has (`Manifest::resolve`).

use crate::bong::Side;
use crate::laser::LaserVariant;
use crate::math::Vec2;
use crate::obstacle::{Drum, Material};
use crate::simulation::{Event, Game, HitCause, HitTarget, Outcome};
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
    /// Which of a sample's takes to play, counted round them; `None` lets
    /// the hash choose. A twin gun's two barrels take neighbouring ones, so
    /// the two shots never sound alike.
    pub take: Option<usize>,
    /// Added to the pan the sound's place gives it: a twin gun's barrels a
    /// little apart.
    pub pan: f32,
    /// Who hears it.
    pub hears: Hears,
}

/// Who hears a sound: anyone in earshot, or only a window that plays one
/// seat - a warning aimed at it, its own hit marker, the inside of its own
/// hull. `bind` names the seat; the director, which knows the window's
/// seats, keeps the sound or drops it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hears {
    All,
    /// The seat in this owner slot.
    Seat(usize),
}

impl Play {
    pub fn new(cue: impl Into<String>, at: At) -> Play {
        Play { cue: cue.into(), at, strength: 1.0, pitch: 1.0, after: 0.0, source: 0, take: None, pan: 0.0, hears: Hears::All }
    }

    /// Heard only by a window that plays the seat in `slot`.
    pub fn for_seat(mut self, slot: usize) -> Play {
        self.hears = Hears::Seat(slot);
        self
    }

    pub fn take(mut self, take: usize) -> Play {
        self.take = Some(take);
        self
    }

    pub fn pan(mut self, pan: f32) -> Play {
        self.pan = pan;
        self
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

/// How long the rod's falling whistle (`warning.rod`) runs before it lands.
const ROD_WHISTLE_S: f32 = 3.0;

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
        Event::DryFired { slot, .. } => one(Play::new(ids::DRY_FIRE.0, At::Slot(slot)).source(slot as u64).for_seat(slot)),
        Event::Placed { .. } => Binding::Silent("a seat placed by the dev tools or a restart"),
        Event::LaserBeam { x0, y0, leg, variant, .. } => {
            if leg == 0 {
                let pitch = LaserVariant::parse(variant).map_or(1.0, source::laser_pitch);
                one(Play::new("weapon.laser.fire", point(x0, y0)).pitch(pitch))
            } else {
                Binding::Silent("a beam's leg through a portal: its first leg sounded")
            }
        }
        Event::Shoved { .. } => Binding::Silent("the blow that shoved the tank sounds"),
        Event::Hit { target, damage, killed, x, y, cause, by } => hit(game, Landed { target, damage, killed, at: Vec2::new(x, y), cause, by }),
        Event::SonicBlast { x, y, .. } => one(Play::new("weapon.sonic_hammer.fire", point(x, y))),
        Event::TellStarted { .. } => Binding::Watched("tell"),
        Event::EmpPulse { x, y, .. } => one(Play::new("weapon.emp_burst.fire", point(x, y))),
        Event::Disabled { x, y, .. } => one(Play::new("weapon.emp_burst.disabled", point(x, y))),
        Event::TowerDisabled { x, y } => one(Play::new("tower.disabled", point(x, y))),
        Event::MissileDud { x, y } => on_water(game, x, y, Play::new("weapon.missiles.dud", point(x, y)), ids::WATER_PLOP.0, 1.0),
        Event::RailSlug { x0, y0, leg, overcharged, .. } => {
            if leg == 0 {
                let cue = if overcharged { "weapon.gauss_rail.fire.overcharged" } else { "weapon.gauss_rail.fire" };
                one(Play::new(cue, point(x0, y0)))
            } else {
                Binding::Silent("a slug's leg through a portal: its first leg sounded")
            }
        }
        Event::ChargeStarted { weapon, .. } => Binding::Watched(if weapon == ActiveWeapon::RodFromGod.name() { "rod" } else { "charge" }),
        Event::ChargeEnded { slot, weapon, end } => {
            // A rail vents its charge; a rod's reticle goes down with an
            // abort blip, and with static when it timed out.
            let rod = weapon == ActiveWeapon::RodFromGod.name();
            let cue = match (rod, end) {
                (false, ChargeEnd::Fizzled) => "weapon.gauss_rail.vent.fizzle",
                (false, ChargeEnd::Vented) => "weapon.gauss_rail.vent",
                (false, ChargeEnd::Lapsed) => "weapon.gauss_rail.vent.lapse",
                (true, ChargeEnd::Fizzled) => "weapon.rod_from_god.cancel",
                (true, ChargeEnd::Vented) => "weapon.rod_from_god.cancel.timeout",
                (true, ChargeEnd::Lapsed) => "weapon.rod_from_god.cancel.lapse",
            };
            one(Play::new(cue, At::Slot(slot)))
        }
        Event::DroneLaunched { x, y, .. } => one(Play::new("weapon.fpv_swarm.launch", point(x, y))),
        Event::DroneBurst { x, y, .. } => on_water(game, x, y, Play::new("weapon.fpv_swarm.burst", point(x, y)), ids::WATER_BLAST.0, 0.5),
        Event::DroneDowned { x, y, .. } => one(Play::new("weapon.fpv_swarm.downed", point(x, y))),
        Event::AirHit { id, aircraft, x, y } => {
            let cue = if aircraft { ids::AIR_HIT_HELI } else { ids::AIR_HIT_DRONE };
            one(Play::new(cue.0, point(x, y)).source(key(id as u64, aircraft as u64)))
        }
        Event::DroneCrashed { x, y, .. } => on_water(game, x, y, Play::new("weapon.fpv_swarm.downed.crash", point(x, y)), ids::WATER_PLOP.0, 0.7),
        Event::DroneLockLost { .. } => Binding::Watched("drone_lock"),
        Event::AirSupportCalled { .. } => one(Play::new("air.called", At::Nowhere)),
        Event::AirSupportArrived { x, y, .. } => one(Play::new("air.arrive", point(x, y))),
        Event::AirSalvo { x, y, .. } => one(Play::new("air.salvo", point(x, y))),
        Event::AirStrike { bomb, x, y, .. } => on_water(game, x, y, Play::new(if bomb { "air.bomb" } else { "air.cannon" }, point(x, y)), ids::WATER_BLAST.0, if bomb { 1.2 } else { 0.5 }),
        Event::AircraftDowned { x, y, .. } => one(Play::new("air.downed", point(x, y))),
        Event::AirSupportLeft { .. } => Binding::Watched("aircraft"),
        Event::RodCalled { cell: c, land, .. } => {
            // The designator locks at once; the fall whistles down to the
            // landing (`land` is on the round clock), its sample's length
            // before it.
            let whistle = (land - game.time - ROD_WHISTLE_S).max(0.0);
            Binding::Play(vec![Play::new("weapon.rod_from_god.call", cell(c)), Play::new(ids::ROD_INCOMING.0, cell(c)).after(whistle)])
        }
        Event::RodImpact { cell: c, .. } => {
            let at = crate::map::cell_to_world(c.0, c.1);
            on_water(game, at.x, at.y, Play::new("weapon.rod_from_god.impact", cell(c)), ids::WATER_COLUMN.0, 1.0)
        }
        Event::WellAnchored { x, y, .. } => one(Play::new("weapon.gravity_well.anchor", point(x, y))),
        Event::WellCollapsed { x, y, .. } => on_water(game, x, y, Play::new("weapon.gravity_well.collapse", point(x, y)), ids::WATER_BLAST.0, 0.8),
        Event::Swallowed { what, x, y } => one(Play::new(source::swallow_cue(what), point(x, y))),
        Event::OrbFizzled { x, y, .. } => one(Play::new("weapon.gravity_well.fizzle", point(x, y))),
        Event::Wreck { slot, x, y } => on_water(game, x, y, Play::new("explosion.wreck", point(x, y)).source(slot as u64), ids::WATER_BLAST.0, 1.0),
        Event::HullStruck { slot, surface, x, y, speed, .. } => {
            let t = tuning();
            let span = (t.audio_slam_full_speed - t.hull_struck_min_speed).max(1.0);
            let strength = 0.3 + 0.7 * ((speed - t.hull_struck_min_speed) / span).clamp(0.0, 1.0);
            one(Play::new(source::slam_cue(surface), point(x, y)).strength(strength).source(slot as u64))
        }
        Event::CrateTossed { to_x, to_y, .. } => one(Play::new("pickup.crate.land", point(to_x, to_y)).after(tuning().crate_toss_seconds)),
        Event::LooseGone { .. } => Binding::Silent("beside the drum's ObstacleDestroyed, which sounds it"),
        Event::Ram { slot, damage, .. } => one(Play::new(ids::RAM.0, At::Slot(slot)).strength((0.6 + damage / 8.0).clamp(0.6, 1.3)).source(slot as u64)),
        Event::PickupCollected { kind, x, y, .. } => one(Play::new(source::pickup_cue(kind), point(x, y))),
        Event::CrateBroken { cooked, x, y, .. } => one(Play::new(if cooked { "pickup.crate.cookoff" } else { "pickup.crate.break" }, point(x, y))),
        Event::PickupRespawned { x, y, .. } => one(Play::new("pickup.respawn", point(x, y))),
        // Keyed by the shield, so one shield turning a stream is one
        // sound at a time while two shields never hold each other back.
        Event::Deflected { slot, x, y } => one(Play::new("hit.deflect", point(x, y)).source(key(slot as u64, 0xDEF))),
        // By the shot and what it glanced off; keyed by its gun and kind,
        // so one gun's glances thin out and another gun's are their own.
        // Its whine follows the shot away (`watch.rs`'s ricochet watcher).
        Event::Ricochet { slot, shot, off, x, y, .. } => {
            let cue = format!("hit.ricochet.{}.{}", source::hit_shot(shot), struck(off));
            one(Play::new(cue, point(x, y)).source(key(slot as u64, shot as u64)))
        }
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
                one(Play::new(format!("chunk.{family}"), point(x, y)).strength((0.6 + broken.count_ones() as f32 * 0.1).min(1.2)))
            }
        }
        Event::ChunksCharred { .. } => Binding::Watched("fire"),
        Event::EmberFlew { .. } => Binding::Watched("fire"),
        Event::GrassTrampled { cell: c } => one(Play::new(ids::GRASS_TRAMPLE.0, cell(c))),
        Event::Blast { x, y, chained, drum } => {
            let cue = match drum {
                Drum::Oil => "explosion.barrel.oil",
                Drum::Fuel => "explosion.barrel.fuel",
                Drum::Lava => "explosion.lava_bomb",
            };
            let strength = if chained { crate::audio::dsp::db_to_gain(tuning().audio_chain_step_db) } else { 1.0 };
            let water = if drum == Drum::Lava { ids::WATER_STEAM.0 } else { ids::WATER_BLAST.0 };
            on_water(game, x, y, Play::new(cue, point(x, y)).strength(strength), water, strength)
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
        // The alarm is the locked seat's alone; the seat whose missile
        // locked hears a soft chirp; nobody else hears either.
        Event::MissileLocked { slot, target, .. } => match target {
            Some(target) => Binding::Play(vec![
                Play::new(ids::LOCK_ON.0, At::Slot(target)).source(target as u64).for_seat(target),
                Play::new(ids::LOCK_ACQUIRED.0, At::Slot(slot)).source(slot as u64).for_seat(slot),
            ]),
            None => Binding::Silent("a lock on the ground point: no tank to warn"),
        },
        Event::MissileBlast { x, y, .. } => on_water(game, x, y, Play::new("explosion.missile", point(x, y)), ids::WATER_BLAST.0, 0.8),
        Event::GrenadeBlast { x, y, .. } => on_water(game, x, y, Play::new("explosion.grenade", point(x, y)), ids::WATER_BLAST.0, 0.8),
        Event::CookOff { x, y } => on_water(game, x, y, Play::new("explosion.cookoff", point(x, y)), ids::WATER_BLAST.0, 0.7),
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

/// A blast, a fall or a strike on liquid water (`Depth::is_wet`: not ice)
/// throws the water up under its own sound: `play`, and `water` at
/// `strength` beside it (docs/water.md: the water takes no scorch).
fn on_water(game: &Game, x: f32, y: f32, play: Play, water: &'static str, strength: f32) -> Binding {
    let mut plays = vec![play];
    if game.water().depth_at(Vec2::new(x, y)).is_wet() {
        plays.push(Play::new(water, point(x, y)).strength(strength).source(key(x.to_bits() as u64, y.to_bits() as u64)));
    }
    Binding::Play(plays)
}

fn destroyed(material: Material) -> String {
    format!("destroy.{}.{}", source::surface_family(material), source::material_name(material))
}

/// One `Event::Hit`, as `hit` reads it.
#[derive(Clone, Copy)]
struct Landed {
    target: HitTarget,
    damage: f32,
    killed: bool,
    at: Vec2,
    cause: HitCause,
    by: Option<usize>,
}

/// What a shot struck, as the last part of a hit's or a ricochet's cue id:
/// `hull`, `bong`, `wall`, or `<family>.<material>`.
fn struck(target: HitTarget) -> String {
    match target {
        HitTarget::Player { .. } | HitTarget::Enemy { .. } => "hull".to_string(),
        HitTarget::Bong { .. } => "bong".to_string(),
        HitTarget::Obstacle { material } => format!("{}.{}", source::surface_family(material), source::material_name(material)),
        HitTarget::Wall => "wall".to_string(),
    }
}

/// The owner slot of the hull a hit landed on; `None` for anything else.
fn hull_slot(target: HitTarget) -> Option<usize> {
    match target {
        HitTarget::Player { player } => Some(player as usize),
        HitTarget::Enemy { slot } => Some(slot),
        HitTarget::Bong { .. } | HitTarget::Obstacle { .. } | HitTarget::Wall => None,
    }
}

/// A hit's sounds (docs/sound-system-prd.md section 4.4): the impact, by
/// what landed on what (`impact`); a bong's death; and the shooter's own
/// word on it (`marker`). A blast's share and the ground's burns make no
/// impact - the blast, fire, lava or slime that did it sounds, though a
/// shield that takes a blast's share whole thrums on its hull - and nor
/// does the rod's crush, past its `RodImpact`.
fn hit(game: &Game, h: Landed) -> Binding {
    let mut plays = Vec::new();
    if h.killed && matches!(h.target, HitTarget::Bong { .. }) {
        plays.push(Play::new(ids::BONG_DEATH.0, At::Point(h.at)));
    }
    if h.cause.is_shot() && h.cause != HitCause::Rod {
        plays.extend(impact(game, &h));
    } else if let (HitCause::Blast, Some(slot)) = (h.cause, hull_slot(h.target))
        && shield_took(game, slot, h.damage)
    {
        plays.push(Play::new(ids::HIT_SHIELD.0, At::Slot(slot)).source(key(slot as u64, HitCause::Blast as u64)));
    }
    plays.extend(marker(&h));
    if plays.is_empty() { Binding::Silent("the blast, fire, lava, slime or rod that did it sounds") } else { Binding::Play(plays) }
}

/// Whether the shield of the hull in `slot` took a hit of `damage` whole:
/// a shield takes all of one (`Tank::take_damage`), the one that breaks it
/// too.
fn shield_took(game: &Game, slot: usize, damage: f32) -> bool {
    damage <= 0.0
        && (tank_in(game, slot).is_some_and(|t| t.is_shielded()) || game.events().iter().any(|e| matches!(*e, Event::ShieldBroken { slot: s, .. } if s == slot)))
}

/// A shot landing: `hit.<shot>.<what>`. On a hull, pitched by the hull's
/// mass, and heard from inside it by the window playing that seat
/// (`source::hull_inside`); on a shield that took it whole, the shield's
/// thrum instead. Keyed by the gun and the cause, so one gun's stream holds
/// to its cue's cooldown and another gun's hits are their own.
fn impact(game: &Game, h: &Landed) -> Vec<Play> {
    let shot = source::hit_shot(h.cause);
    let strength = (0.6 + h.damage / 40.0).clamp(0.5, 1.3);
    let source = key(h.by.map_or(u64::MAX, |s| s as u64), h.cause as u64);
    let Some(slot) = hull_slot(h.target) else {
        return vec![Play::new(format!("hit.{shot}.{}", struck(h.target)), At::Point(h.at)).strength(strength).source(source)];
    };
    if shield_took(game, slot, h.damage) {
        return vec![Play::new(ids::HIT_SHIELD.0, At::Point(h.at)).source(source)];
    }
    let mass = tank_in(game, slot).map_or(1.0, |t| t.mass_factor());
    let pitch = source::hull_pitch(mass, tuning().audio_hull_mass_pitch);
    let mut plays = vec![Play::new(format!("hit.{shot}.hull"), At::Point(h.at)).strength(strength).pitch(pitch).source(source)];
    if let Some(inside) = source::hull_inside(h.cause) {
        plays.push(Play::new(ids::HULL_INSIDE.0, At::Slot(slot)).strength(inside * strength).pitch(pitch).source(source).for_seat(slot));
    }
    plays
}

/// The shooter's own word on a hit, heard only by the window that plays
/// it: a tick on a hull or a bong of the other side, a buzz on one of its
/// own side - itself included. A tile or the field's edge says nothing, nor
/// a tower's shot, and a blast only where it hurt.
fn marker(h: &Landed) -> Option<Play> {
    let by = h.by.filter(|&s| s != crate::shell::TOWER_SLOT)?;
    if !(h.cause.is_shot() || (h.cause == HitCause::Blast && h.damage > 0.0)) {
        return None;
    }
    // Only a seat is ever played, so its side is the players'.
    let friendly = match h.target {
        HitTarget::Player { .. } | HitTarget::Bong { side: Side::Player } => true,
        HitTarget::Enemy { .. } | HitTarget::Bong { side: Side::Enemy } => false,
        HitTarget::Obstacle { .. } | HitTarget::Wall => return None,
    };
    let cue = if friendly { ids::FRIENDLY_FIRE } else { ids::HIT_MARKER };
    Some(Play::new(cue.0, At::Nowhere).source(by as u64).for_seat(by))
}

/// A tank's shot: the cannon tiered by its calibre, a plasma bolt pitched
/// by its colour (`source::plasma_pitch`); a twin gun's shells and plasma
/// bolts each sound as their own shot (`twin_shots`); a weapon a watcher
/// drives - the minigun, the flamethrower, each missile of a volley - or
/// another event carries is not played here.
fn fired(game: &Game, slot: usize, weapon: &str) -> Binding {
    let Some(weapon) = ActiveWeapon::parse(weapon) else {
        return Binding::Silent("a weapon the sound maps do not know");
    };
    let tank = tank_in(game, slot);
    // The weapons whose twin barrel fires a second projectile
    // (`Tank::pending_shot`, `Tank::pending_plasma_shot`).
    let twin = matches!(weapon, ActiveWeapon::Shell | ActiveWeapon::Plasma) && tank.as_ref().is_some_and(|tank| tank.twin_gun());
    let rotation = tank.as_ref().map_or(0.0, |tank| tank.rotation);
    let first = match source::weapon_sound(weapon) {
        FireSound::Cue(cue) if weapon == ActiveWeapon::Plasma => {
            let pitch = tank.as_ref().map_or(1.0, |tank| source::plasma_pitch(tank.plasma_variant));
            Play::new(cue, At::Slot(slot)).pitch(pitch).source(slot as u64)
        }
        FireSound::Cue(cue) => Play::new(cue, At::Slot(slot)).source(slot as u64),
        FireSound::Watched(watcher) => return Binding::Watched(watcher),
        FireSound::Elsewhere(_) => return Binding::Silent("its own event carries the sound"),
        FireSound::Cannon => {
            let t = tuning();
            let (factor, mass) = tank.as_ref().map_or((1.0, 1.0), |tank| (t.tank_damage_factor[tank.row as usize], tank.mass_factor()));
            Play::new(source::cannon_cue(factor), At::Slot(slot)).pitch(source::engine_pitch(mass)).source(slot as u64)
        }
    };
    if twin { Binding::Play(twin_shots(first, rotation, game.frame())) } else { one(first) }
}

/// A twin gun's two shots, each heard as its own (docs/sound-system-prd.md
/// section 4.3): the first barrel's at once, the second's when its
/// projectile leaves (`tank_twin_shot_delay_seconds`), on neighbouring takes
/// so the two never sound alike, the second a shade lower, and each panned
/// toward its own barrel - the first fires from the hull's left, the second
/// from its right, so a gun aimed up or down splits them across the stereo
/// field and one aimed sideways stacks them, as two barrels do.
pub fn twin_shots(first: Play, rotation_deg: f32, frame: u64) -> Vec<Play> {
    let t = tuning();
    // The second barrel sits along (cos r, sin r) from the hull's centre
    // (`Shell::spawn`), the first opposite; the screen's x is the pan's.
    let side = libm::cosf(rotation_deg.to_radians()) * t.audio_twin_spread;
    let take = (frame % 2) as usize;
    let second = first
        .clone()
        .after(t.tank_twin_shot_delay_seconds)
        .pitch(first.pitch * (1.0 - t.audio_twin_detune))
        .take(take + 1)
        .pan(side)
        .source(first.source + 1_000);
    vec![first.take(take).pan(-side), second]
}
