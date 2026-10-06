//! `simulation::Event` on the wire (docs/online-coop-prd.md §4.4).
//!
//! The simulation's `Event` carries `&'static str` names and `f32`
//! positions and only serialises; `WireEvent` is its owned mirror that
//! goes both ways, with positions in quarter pixels and every name as a
//! small enum. `WireEvent::from_event` is the one classifier: an
//! exhaustive `match` with no wildcard, so a variant added to `Event` fails
//! to compile here until it is either mirrored or put on the not-sent
//! list. The AI's trace (`AiAction`, `EngageSlot`, `StuckEscape`, `Breach`,
//! `Retreat`, `Alert`, `Retarget`), `MissileLocked` (which tank a seeker
//! picked - a decision, not a picture; its `MissileBlast` does travel),
//! `Rerolled` (a straggler taken off out of every seat's sight - the
//! replica sees the hull leave and come back through its gate by itself),
//! `PhysicsQuarantine` (logged server-side) and a training round's
//! `BeatDone`, `DoorOpened`, `FlagTaken` and `FrogRevived` (a training map
//! is never hosted) never travel.
//!
//! `to_event` gives the replica the `Event` its presentation layer already
//! reads (`fx.rs` diffs `game.events()`), the quantised positions
//! dequantised.

use serde::{Deserialize, Serialize};

use crate::frog::Side;
use crate::laser::LaserVariant;
use crate::level::{Mission, SpawnKind, Tier};
use crate::net::wire::{RoundOutcome, WeaponKind, dequantise_heading, dequantise_pos, dir_from_index, dir_index, quantise_heading, quantise_pos};
use crate::tank::Dir;
use crate::obstacle::{Drum, Material};
use crate::pickup::PickupKind;
use crate::tower::TowerKind;
use crate::simulation::{Event, HitTarget};

/// The serde tags (`Event`'s `event` field) of the variants
/// `WireEvent::from_event` never sends.
pub const NOT_SENT: [&str; 13] = [
    "physics_quarantine",
    "beat_done",
    "door_opened",
    "flag_taken",
    "frog_revived",
    "rerolled",
    "ai_action",
    "engage_slot",
    "stuck_escape",
    "breach",
    "retreat",
    "alert",
    "retarget",
];

/// What the flamethrower lit (`Event::Ignited`'s `what`), in wire order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IgnitedWhat {
    Ground,
    Oil,
    Wood,
    Tree,
    Drum,
    Sandbag,
    Fence,
    /// A defence tower caught fire (docs/defence-towers-prd.md section 7).
    Tower,
}

impl IgnitedWhat {
    /// Every kind, in wire order.
    pub const ALL: [IgnitedWhat; 8] = [
        IgnitedWhat::Ground,
        IgnitedWhat::Oil,
        IgnitedWhat::Wood,
        IgnitedWhat::Tree,
        IgnitedWhat::Drum,
        IgnitedWhat::Sandbag,
        IgnitedWhat::Fence,
        IgnitedWhat::Tower,
    ];

    /// The name `flame.rs` puts in the event.
    pub fn name(self) -> &'static str {
        match self {
            IgnitedWhat::Ground => "ground",
            IgnitedWhat::Oil => "oil",
            IgnitedWhat::Wood => "wood",
            IgnitedWhat::Tree => "tree",
            IgnitedWhat::Drum => "drum",
            IgnitedWhat::Sandbag => "sandbag",
            IgnitedWhat::Fence => "fence",
            IgnitedWhat::Tower => "tower",
        }
    }

    /// Inverse of `name`; `None` for anything else.
    pub fn parse(name: &str) -> Option<IgnitedWhat> {
        IgnitedWhat::ALL.into_iter().find(|w| w.name() == name)
    }
}

/// `HitTarget` on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireHitTarget {
    Player { player: u8 },
    Enemy { slot: u16 },
    Frog { side: Side },
    Obstacle { material: Material },
    Wall,
}

impl From<HitTarget> for WireHitTarget {
    fn from(t: HitTarget) -> Self {
        match t {
            HitTarget::Player { player } => WireHitTarget::Player { player },
            HitTarget::Enemy { slot } => WireHitTarget::Enemy { slot: slot_u16(slot) },
            HitTarget::Frog { side } => WireHitTarget::Frog { side },
            HitTarget::Obstacle { material } => WireHitTarget::Obstacle { material },
            HitTarget::Wall => WireHitTarget::Wall,
        }
    }
}

impl From<WireHitTarget> for HitTarget {
    fn from(t: WireHitTarget) -> Self {
        match t {
            WireHitTarget::Player { player } => HitTarget::Player { player },
            WireHitTarget::Enemy { slot } => HitTarget::Enemy { slot: slot as usize },
            WireHitTarget::Frog { side } => HitTarget::Frog { side },
            WireHitTarget::Obstacle { material } => HitTarget::Obstacle { material },
            WireHitTarget::Wall => HitTarget::Wall,
        }
    }
}

/// One round event as the client receives it. Positions are quarter
/// pixels (`wire::quantise_pos`); damage amounts stay `f32` because they
/// are exact in the simulation and events are rare. Slots are `u16`
/// because a wave round's owner-slot counter is monotonic.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WireEvent {
    RoundStarted { seed: u64, enemies: u16, mission: Mission, spawn: SpawnKind },
    RoundEnded { outcome: RoundOutcome },
    /// A trigger pull; `input_tick` is the seat's input tick the room
    /// applied on the tick it fired (0 for an enemy), so the client pairs
    /// it with the press it drew (`encode::wire_events_acked`).
    Fired { slot: u16, weapon: WeaponKind, input_tick: u32 },
    Hit { target: WireHitTarget, damage: f32, killed: bool, x: i16, y: i16 },
    Wreck { slot: u16, x: i16, y: i16 },
    Ram { slot: u16, other_slot: u16, damage: f32 },
    PickupCollected { slot: u16, kind: PickupKind, x: i16, y: i16, spilled: bool },
    CrateBroken { kind: PickupKind, x: i16, y: i16, cooked: bool },
    PickupRespawned { kind: PickupKind, x: i16, y: i16 },
    Deflected { slot: u16, x: i16, y: i16 },
    ShieldBroken { slot: u16, x: i16, y: i16 },
    ShellsCollided { x: i16, y: i16 },
    FrogBite { side: Side, slot: u16, damage: f32, killed: bool },
    FrogHealed { side: Side, slot: u16, amount: f32, x: i16, y: i16 },
    WaveStarted { wave: u16, size: u16, tier: Tier },
    TankEntered { slot: u16 },
    WreckRemoved { slot: u16 },
    ObstacleDestroyed { material: Material, x: i16, y: i16 },
    Blast { x: i16, y: i16, chained: bool, drum: Drum },
    DrumLaunched { x: i16, y: i16, to_x: i16, to_y: i16 },
    /// A laser's beam, muzzle to where it stopped (`LaserVariant::index`),
    /// and the seat that fired it (`wire::NO_SEAT` for an enemy): the
    /// shooter draws its own beam on the press and skips this one. A beam
    /// bent by portals is one a leg, in order (`Event::LaserBeam`'s `leg`
    /// and `portal`): the shooter skips the first, which is the one it drew.
    LaserBeam { x0: i16, y0: i16, x1: i16, y1: i16, variant: u8, seat: u8, leg: u8, portal: bool },
    /// A shove the room put on a client-owned hull - knockback, a blast, a
    /// ram, a missile launch's recoil - that the owner applies to its own
    /// body, since the room places that hull wherever the owner says
    /// (docs/online-coop-prd.md §4.16). The recoil of a shell, bolt or
    /// bullet is not sent: the owner kicks its own hull at each launch.
    /// A velocity change, `quantise_velocity`.
    Shoved { seat: u8, vx: i8, vy: i8 },
    /// The room moved a client-owned hull itself; the client snaps to it
    /// (`dir_index`).
    Placed { seat: u8, x: i16, y: i16, dir: u8 },
    Teleported { slot: u16, x: i16, y: i16, to_x: i16, to_y: i16 },
    /// A shot through a portal; `Event::ShotTeleported`. `id` is the shot's
    /// as `ShotState::id` keys it, `None` for a beam: the interpolator
    /// draws the shot's jump as a jump rather than a slide across the
    /// field.
    ShotTeleported { id: Option<u16>, x: i16, y: i16, to_x: i16, to_y: i16 },
    FireStarted { x: i16, y: i16, pool: bool },
    Ignited { x: i16, y: i16, what: IgnitedWhat },
    CookOff { x: i16, y: i16 },
    /// `slot`'s seeker missile came down and burst at (`x`, `y`);
    /// `Event::MissileBlast`. `MissileLocked` does not travel - see
    /// the not-sent list.
    MissileBlast { slot: u16, x: i16, y: i16 },
    /// `slot`'s shot bounced off iron or a barrel at (`x`, `y`) and flies
    /// on along `heading` (`wire::quantise_heading`); `Event::Ricochet`.
    Ricochet { slot: u16, x: i16, y: i16, heading: u8 },
    /// A tesla coil's bolt; `Event::TeslaStrike` (docs/defence-towers-prd.md).
    TeslaStrike { x0: i16, y0: i16, x1: i16, y1: i16, chained: bool },
    /// A tower fired along `heading` (`wire::quantise_heading`);
    /// `Event::TowerFired`.
    TowerFired { kind: TowerKind, x: i16, y: i16, heading: u8 },
    GlobSplashed { x: i16, y: i16 },
    Slimed { slot: u16 },
    SlimeWashed { slot: u16 },
    TowerRepaired { side: Side, x: i16, y: i16 },
    /// A volcano's lava bomb thrown from its crater toward where it will
    /// burst; `Event::LavaBombLaunched` (docs/volcano.md).
    LavaBombLaunched { x: i16, y: i16, to_x: i16, to_y: i16 },
    /// A seat set a lantern down; `Event::LanternSet`.
    LanternSet { seat: u8, x: i16, y: i16 },
    /// A blast broke a lantern; `Event::LanternBroken`.
    LanternBroken { x: i16, y: i16 },
    /// `slot`'s grenade went off at (`x`, `y`); `Event::GrenadeBlast`.
    GrenadeBlast { slot: u16, x: i16, y: i16 },
    /// A seat took the mushroom on a cell; `Event::MushroomTaken`
    /// (docs/mushroom-hunt-prd.md).
    MushroomTaken { seat: u8, col: i16, row: i16, left: u16 },
}

fn slot_u16(slot: usize) -> u16 {
    slot.min(u16::MAX as usize) as u16
}

fn count_u16(n: u32) -> u16 {
    n.min(u16::MAX as u32) as u16
}

impl WireEvent {
    /// The wire form of `event`, or `None` for a variant that never
    /// travels (`NOT_SENT`). A name the vocabulary does not know is a
    /// simulation change this module has not caught up with: it trips a
    /// debug assertion and falls back to the first kind.
    pub fn from_event(event: &Event) -> Option<WireEvent> {
        let q = quantise_pos;
        Some(match *event {
            Event::RoundStarted { seed, enemies, mission, spawn } => {
                WireEvent::RoundStarted { seed, enemies: slot_u16(enemies), mission, spawn }
            }
            Event::RoundEnded { outcome } => WireEvent::RoundEnded { outcome: outcome.into() },
            Event::Fired { slot, weapon } => {
                let kind = WeaponKind::parse(weapon).unwrap_or_else(|| {
                    debug_assert!(false, "unknown weapon name {weapon:?} in Event::Fired");
                    WeaponKind::Shell
                });
                WireEvent::Fired { slot: slot_u16(slot), weapon: kind, input_tick: 0 }
            }
            Event::Hit { target, damage, killed, x, y } => {
                WireEvent::Hit { target: target.into(), damage, killed, x: q(x), y: q(y) }
            }
            Event::Wreck { slot, x, y } => WireEvent::Wreck { slot: slot_u16(slot), x: q(x), y: q(y) },
            Event::Ram { slot, other_slot, damage } => {
                WireEvent::Ram { slot: slot_u16(slot), other_slot: slot_u16(other_slot), damage }
            }
            Event::PickupCollected { slot, kind, x, y, spilled } => WireEvent::PickupCollected { slot: slot_u16(slot), kind, x: q(x), y: q(y), spilled },
            Event::CrateBroken { kind, x, y, cooked } => WireEvent::CrateBroken { kind, x: q(x), y: q(y), cooked },
            Event::PickupRespawned { kind, x, y } => WireEvent::PickupRespawned { kind, x: q(x), y: q(y) },
            Event::Deflected { slot, x, y } => WireEvent::Deflected { slot: slot_u16(slot), x: q(x), y: q(y) },
            Event::ShieldBroken { slot, x, y } => {
                WireEvent::ShieldBroken { slot: slot_u16(slot), x: q(x), y: q(y) }
            }
            Event::ShellsCollided { x, y } => WireEvent::ShellsCollided { x: q(x), y: q(y) },
            Event::FrogBite { side, slot, damage, killed } => {
                WireEvent::FrogBite { side, slot: slot_u16(slot), damage, killed }
            }
            Event::FrogHealed { side, slot, amount, x, y } => {
                WireEvent::FrogHealed { side, slot: slot_u16(slot), amount, x: q(x), y: q(y) }
            }
            Event::WaveStarted { wave, size, tier } => {
                WireEvent::WaveStarted { wave: count_u16(wave), size: count_u16(size), tier }
            }
            Event::TankEntered { slot } => WireEvent::TankEntered { slot: slot_u16(slot) },
            Event::WreckRemoved { slot } => WireEvent::WreckRemoved { slot: slot_u16(slot) },
            Event::ObstacleDestroyed { material, x, y } => {
                WireEvent::ObstacleDestroyed { material, x: q(x), y: q(y) }
            }
            Event::Blast { x, y, chained, drum } => WireEvent::Blast { x: q(x), y: q(y), chained, drum },
            Event::LavaBombLaunched { x, y, to_x, to_y } => {
                WireEvent::LavaBombLaunched { x: q(x), y: q(y), to_x: q(to_x), to_y: q(to_y) }
            }
            Event::LanternSet { seat, x, y } => WireEvent::LanternSet { seat, x: q(x), y: q(y) },
            Event::LanternBroken { x, y } => WireEvent::LanternBroken { x: q(x), y: q(y) },
            Event::GrenadeBlast { slot, x, y } => WireEvent::GrenadeBlast { slot: slot_u16(slot), x: q(x), y: q(y) },
            Event::DrumLaunched { x, y, to_x, to_y } => {
                WireEvent::DrumLaunched { x: q(x), y: q(y), to_x: q(to_x), to_y: q(to_y) }
            }
            Event::Placed { seat, x, y, rotation } => WireEvent::Placed {
                seat: seat.min(u8::MAX as usize) as u8,
                x: q(x),
                y: q(y),
                dir: dir_index(Dir::from_rotation(rotation).unwrap_or(Dir::Up)),
            },
            Event::LaserBeam { x0, y0, x1, y1, variant, seat, leg, portal } => {
                let variant = LaserVariant::parse(variant).unwrap_or_else(|| {
                    debug_assert!(false, "unknown laser variant {variant:?} in Event::LaserBeam");
                    LaserVariant::Red
                });
                WireEvent::LaserBeam { x0: q(x0), y0: q(y0), x1: q(x1), y1: q(y1), variant: variant.index(), seat, leg, portal }
            }
            Event::Shoved { seat, vx, vy } => WireEvent::Shoved {
                seat: seat.min(u8::MAX as usize) as u8,
                vx: crate::net::wire::quantise_velocity(vx),
                vy: crate::net::wire::quantise_velocity(vy),
            },
            Event::Teleported { slot, x, y, to_x, to_y } => {
                WireEvent::Teleported { slot: slot_u16(slot), x: q(x), y: q(y), to_x: q(to_x), to_y: q(to_y) }
            }
            Event::ShotTeleported { id, x, y, to_x, to_y } => WireEvent::ShotTeleported {
                id: id.map(crate::net::encode::shot_wire_id),
                x: q(x),
                y: q(y),
                to_x: q(to_x),
                to_y: q(to_y),
            },
            Event::FireStarted { x, y, pool } => WireEvent::FireStarted { x: q(x), y: q(y), pool },
            Event::Ignited { x, y, what } => {
                let kind = IgnitedWhat::parse(what).unwrap_or_else(|| {
                    debug_assert!(false, "unknown ignition target {what:?} in Event::Ignited");
                    IgnitedWhat::Ground
                });
                WireEvent::Ignited { x: q(x), y: q(y), what: kind }
            }
            Event::CookOff { x, y } => WireEvent::CookOff { x: q(x), y: q(y) },
            Event::MissileBlast { slot, x, y } => {
                WireEvent::MissileBlast { slot: slot_u16(slot), x: q(x), y: q(y) }
            }
            Event::Ricochet { slot, x, y, heading } => {
                WireEvent::Ricochet { slot: slot_u16(slot), x: q(x), y: q(y), heading: quantise_heading(heading) }
            }
            Event::TeslaStrike { x0, y0, x1, y1, chained } => {
                WireEvent::TeslaStrike { x0: q(x0), y0: q(y0), x1: q(x1), y1: q(y1), chained }
            }
            Event::TowerFired { kind, x, y, heading } => {
                let kind = TowerKind::parse(kind).unwrap_or_else(|| {
                    debug_assert!(false, "unknown tower name {kind:?} in Event::TowerFired");
                    TowerKind::Gun
                });
                WireEvent::TowerFired { kind, x: q(x), y: q(y), heading: quantise_heading(heading) }
            }
            Event::GlobSplashed { x, y } => WireEvent::GlobSplashed { x: q(x), y: q(y) },
            Event::Slimed { slot } => WireEvent::Slimed { slot: slot_u16(slot) },
            Event::SlimeWashed { slot } => WireEvent::SlimeWashed { slot: slot_u16(slot) },
            Event::TowerRepaired { side, x, y } => WireEvent::TowerRepaired { side, x: q(x), y: q(y) },
            Event::MushroomTaken { seat, col, row, left } => WireEvent::MushroomTaken { seat, col, row, left },
            // Never sent: logged on the server.
            Event::PhysicsQuarantine { .. } => return None,
            // Never sent: a training map is never hosted
            // (`MapFile::hostable`), so no room runs its script.
            Event::BeatDone { .. } | Event::DoorOpened { .. } | Event::FlagTaken { .. } | Event::FrogRevived { .. } => return None,
            // Never sent: the hull's leaving and its roll-in through the
            // new gate are in the snapshots.
            Event::Rerolled { .. } => return None,
            // Never sent: the AI's trace.
            Event::AiAction { .. }
            | Event::EngageSlot { .. }
            | Event::StuckEscape { .. }
            | Event::Breach { .. }
            | Event::Retreat { .. }
            | Event::Alert { .. }
            | Event::Retarget { .. } => return None,
            // Never sent: which tank a missile picked is the server's
            // decision, not a picture. The burst it ends in travels, and
            // that is the only part `fx.rs` reads.
            Event::MissileLocked { .. } => return None,
        })
    }

    /// The simulation `Event` the replica hands its presentation layer.
    /// Every variant has one; the `Option` is the shape `TryFrom` and the
    /// round-trip tests read it through.
    pub fn to_event(&self) -> Option<Event> {
        let d = dequantise_pos;
        Some(match *self {
            WireEvent::RoundStarted { seed, enemies, mission, spawn } => {
                Event::RoundStarted { seed, enemies: enemies as usize, mission, spawn }
            }
            WireEvent::RoundEnded { outcome } => Event::RoundEnded { outcome: outcome.into() },
            WireEvent::Fired { slot, weapon, .. } => Event::Fired { slot: slot as usize, weapon: weapon.name() },
            WireEvent::Hit { target, damage, killed, x, y } => {
                Event::Hit { target: target.into(), damage, killed, x: d(x), y: d(y) }
            }
            WireEvent::Wreck { slot, x, y } => Event::Wreck { slot: slot as usize, x: d(x), y: d(y) },
            WireEvent::Ram { slot, other_slot, damage } => {
                Event::Ram { slot: slot as usize, other_slot: other_slot as usize, damage }
            }
            WireEvent::PickupCollected { slot, kind, x, y, spilled } => Event::PickupCollected { slot: slot as usize, kind, x: d(x), y: d(y), spilled },
            WireEvent::CrateBroken { kind, x, y, cooked } => Event::CrateBroken { kind, x: d(x), y: d(y), cooked },
            WireEvent::PickupRespawned { kind, x, y } => Event::PickupRespawned { kind, x: d(x), y: d(y) },
            WireEvent::Deflected { slot, x, y } => Event::Deflected { slot: slot as usize, x: d(x), y: d(y) },
            WireEvent::ShieldBroken { slot, x, y } => Event::ShieldBroken { slot: slot as usize, x: d(x), y: d(y) },
            WireEvent::ShellsCollided { x, y } => Event::ShellsCollided { x: d(x), y: d(y) },
            WireEvent::FrogBite { side, slot, damage, killed } => {
                Event::FrogBite { side, slot: slot as usize, damage, killed }
            }
            WireEvent::FrogHealed { side, slot, amount, x, y } => {
                Event::FrogHealed { side, slot: slot as usize, amount, x: d(x), y: d(y) }
            }
            WireEvent::WaveStarted { wave, size, tier } => {
                Event::WaveStarted { wave: wave as u32, size: size as u32, tier }
            }
            WireEvent::TankEntered { slot } => Event::TankEntered { slot: slot as usize },
            WireEvent::WreckRemoved { slot } => Event::WreckRemoved { slot: slot as usize },
            WireEvent::ObstacleDestroyed { material, x, y } => Event::ObstacleDestroyed { material, x: d(x), y: d(y) },
            WireEvent::Blast { x, y, chained, drum } => Event::Blast { x: d(x), y: d(y), chained, drum },
            WireEvent::LavaBombLaunched { x, y, to_x, to_y } => {
                Event::LavaBombLaunched { x: d(x), y: d(y), to_x: d(to_x), to_y: d(to_y) }
            }
            WireEvent::LanternSet { seat, x, y } => Event::LanternSet { seat, x: d(x), y: d(y) },
            WireEvent::MushroomTaken { seat, col, row, left } => Event::MushroomTaken { seat, col, row, left },
            WireEvent::LanternBroken { x, y } => Event::LanternBroken { x: d(x), y: d(y) },
            WireEvent::GrenadeBlast { slot, x, y } => Event::GrenadeBlast { slot: slot as usize, x: d(x), y: d(y) },
            WireEvent::DrumLaunched { x, y, to_x, to_y } => {
                Event::DrumLaunched { x: d(x), y: d(y), to_x: d(to_x), to_y: d(to_y) }
            }
            WireEvent::Placed { seat, x, y, dir } => Event::Placed {
                seat: seat as usize,
                x: d(x),
                y: d(y),
                rotation: dir_from_index(dir).unwrap_or(Dir::Up).rotation(),
            },
            WireEvent::Shoved { seat, vx, vy } => Event::Shoved {
                seat: seat as usize,
                vx: crate::net::wire::dequantise_velocity(vx),
                vy: crate::net::wire::dequantise_velocity(vy),
            },
            WireEvent::LaserBeam { x0, y0, x1, y1, variant, seat, leg, portal } => Event::LaserBeam {
                seat,
                leg,
                portal,
                x0: d(x0),
                y0: d(y0),
                x1: d(x1),
                y1: d(y1),
                variant: LaserVariant::ALL.get(variant as usize).copied().unwrap_or(LaserVariant::Red).name(),
            },
            WireEvent::Teleported { slot, x, y, to_x, to_y } => {
                Event::Teleported { slot: slot as usize, x: d(x), y: d(y), to_x: d(to_x), to_y: d(to_y) }
            }
            WireEvent::ShotTeleported { id, x, y, to_x, to_y } => {
                Event::ShotTeleported { id: id.map(u32::from), x: d(x), y: d(y), to_x: d(to_x), to_y: d(to_y) }
            }
            WireEvent::FireStarted { x, y, pool } => Event::FireStarted { x: d(x), y: d(y), pool },
            WireEvent::Ignited { x, y, what } => Event::Ignited { x: d(x), y: d(y), what: what.name() },
            WireEvent::CookOff { x, y } => Event::CookOff { x: d(x), y: d(y) },
            WireEvent::MissileBlast { slot, x, y } => {
                Event::MissileBlast { slot: slot as usize, x: d(x), y: d(y) }
            }
            WireEvent::Ricochet { slot, x, y, heading } => {
                Event::Ricochet { slot: slot as usize, x: d(x), y: d(y), heading: dequantise_heading(heading) }
            }
            WireEvent::TeslaStrike { x0, y0, x1, y1, chained } => {
                Event::TeslaStrike { x0: d(x0), y0: d(y0), x1: d(x1), y1: d(y1), chained }
            }
            WireEvent::TowerFired { kind, x, y, heading } => {
                Event::TowerFired { kind: kind.name(), x: d(x), y: d(y), heading: dequantise_heading(heading) }
            }
            WireEvent::GlobSplashed { x, y } => Event::GlobSplashed { x: d(x), y: d(y) },
            WireEvent::Slimed { slot } => Event::Slimed { slot: slot as usize },
            WireEvent::SlimeWashed { slot } => Event::SlimeWashed { slot: slot as usize },
            WireEvent::TowerRepaired { side, x, y } => Event::TowerRepaired { side, x: d(x), y: d(y) },
        })
    }

    /// The heading a `Ricochet` carries, in degrees; `None` for any other
    /// variant.
    pub fn ricochet_heading(&self) -> Option<f32> {
        match *self {
            WireEvent::Ricochet { heading, .. } => Some(dequantise_heading(heading)),
            _ => None,
        }
    }
}

impl TryFrom<&Event> for WireEvent {
    /// The variant's serde tag, for a log line.
    type Error = &'static str;

    fn try_from(event: &Event) -> Result<Self, Self::Error> {
        WireEvent::from_event(event).ok_or("event is on the not-sent list")
    }
}

impl TryFrom<&WireEvent> for Event {
    /// The variant's serde tag, for a log line.
    type Error = &'static str;

    fn try_from(event: &WireEvent) -> Result<Self, Self::Error> {
        event.to_event().ok_or("no simulation counterpart")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::Outcome;

    /// One sample of every `Event` variant, positions on the quarter-pixel
    /// grid so the wire round trip is exact.
    fn every_event() -> Vec<Event> {
        vec![
            Event::RoundStarted { seed: 7, enemies: 12, mission: Mission::Hunt, spawn: SpawnKind::Waves },
            Event::RoundEnded { outcome: Outcome::Won },
            Event::Fired { slot: 300, weapon: "plasma" },
            Event::Hit {
                target: HitTarget::Obstacle { material: Material::Glass },
                damage: 12.5,
                killed: false,
                x: 100.25,
                y: 200.5,
            },
            Event::Wreck { slot: 3, x: 64.0, y: 96.75 },
            Event::Ram { slot: 0, other_slot: 5, damage: 3.25 },
            Event::PickupCollected { slot: 1, kind: PickupKind::FrogHealth, x: 48.0, y: 80.0, spilled: false },
            Event::CrateBroken { kind: PickupKind::Ammo, x: 112.0, y: 80.0, cooked: true },
            Event::PickupRespawned { kind: PickupKind::Shield, x: 32.0, y: 32.0 },
            Event::Deflected { slot: 2, x: 1.5, y: -2.25 },
            Event::ShieldBroken { slot: 2, x: 8.0, y: 9.0 },
            Event::ShellsCollided { x: 500.0, y: 250.0 },
            Event::FrogBite { side: Side::Enemy, slot: 4, damage: 30.0, killed: true },
            Event::FrogHealed { side: Side::Player, slot: 0, amount: 40.0, x: 544.0, y: 272.0 },
            Event::WaveStarted { wave: 3, size: 6, tier: Tier::Heavy },
            Event::TankEntered { slot: 9 },
            Event::Rerolled { slot: 9, x: 96.0, y: 64.0 },
            Event::WreckRemoved { slot: 9 },
            Event::ObstacleDestroyed { material: Material::Pine, x: 96.0, y: 96.0 },
            Event::Blast { x: 128.0, y: 160.0, chained: true, drum: Drum::Fuel },
            Event::DrumLaunched { x: 128.0, y: 160.0, to_x: 256.0, to_y: 160.0 },
            Event::LavaBombLaunched { x: 640.0, y: 352.0, to_x: 800.0, to_y: 416.0 },
            Event::LanternSet { seat: 1, x: 200.0, y: 96.0 },
            Event::MushroomTaken { seat: 1, col: 12, row: 7, left: 3 },
            Event::LanternBroken { x: 200.0, y: 96.0 },
            Event::GrenadeBlast { slot: 0, x: 200.0, y: 96.0 },
            Event::LaserBeam { x0: 100.0, y0: 200.0, x1: 100.0, y1: 32.0, variant: "blue", seat: 1, leg: 1, portal: true },
            Event::Shoved { seat: 0, vx: 120.0, vy: -40.0 },
            Event::Placed { seat: 2, x: 320.0, y: 160.0, rotation: 90.0 },
            Event::Teleported { slot: 1, x: 64.0, y: 64.0, to_x: 960.0, to_y: 480.0 },
            Event::ShotTeleported { id: Some(41), x: 320.0, y: 362.25, to_x: 960.0, to_y: 362.25 },

            Event::FireStarted { x: 48.0, y: 48.0, pool: true },
            Event::Ignited { x: 48.0, y: 80.0, what: "sandbag" },
            Event::CookOff { x: 64.0, y: 96.75 },
            Event::Ricochet { slot: 2, x: 320.0, y: 64.0, heading: 90.0 },
            Event::TeslaStrike { x0: 160.0, y0: 80.0, x1: 240.5, y1: 96.25, chained: true },
            Event::TowerFired { kind: "bio_slush", x: 160.0, y: 67.0, heading: 90.0 },
            Event::GlobSplashed { x: 300.0, y: 180.0 },
            Event::Slimed { slot: 4 },
            Event::SlimeWashed { slot: 4 },
            Event::TowerRepaired { side: Side::Enemy, x: 176.0, y: 80.0 },
            Event::PhysicsQuarantine { bodies: 1, colliders: 2 },
            Event::BeatDone { beat: 2 },
            Event::DoorOpened { beat: 1, x: 320.0, y: 176.0 },
            Event::FlagTaken { seat: 0, x: 256.0, y: 80.0 },
            Event::FrogRevived { x: 1088.0, y: 208.0 },
            Event::AiAction { slot: 5, from: None, to: Some("attack") },
            Event::EngageSlot { slot: 5, from: Some(1), to: None },
            Event::StuckEscape { slot: 5, escapes: 2 },
            Event::Breach { slot: 5, dir: Some("up") },
            Event::Retreat { slot: 5, on: true },
            Event::Alert { on: false, x: 1.0, y: 2.0 },
            Event::Retarget { slot: 5, player: 1 },
        ]
    }

    fn tag(event: &Event) -> String {
        serde_json::to_value(event).unwrap()["event"].as_str().unwrap().to_string()
    }

    #[test]
    fn every_variant_is_sent_or_on_the_not_sent_list() {
        let events = every_event();
        let mut seen = std::collections::BTreeSet::new();
        for event in &events {
            let tag = tag(event);
            assert!(seen.insert(tag.clone()), "duplicate sample {tag}");
            let sent = WireEvent::from_event(event).is_some();
            let listed = NOT_SENT.contains(&tag.as_str());
            assert!(sent != listed, "{tag}: sent={sent} listed={listed}");
        }
        assert_eq!(seen.len(), 53, "one sample per Event variant");
        for name in NOT_SENT {
            assert!(seen.contains(name), "NOT_SENT names an unknown variant {name}");
        }
    }

    #[test]
    fn sent_variants_survive_the_wire_and_come_back_as_the_same_event() {
        for event in every_event() {
            let Some(wire) = WireEvent::from_event(&event) else { continue };
            let bytes = postcard::to_stdvec(&wire).unwrap();
            let back: WireEvent = postcard::from_bytes(&bytes).unwrap();
            assert_eq!(back, wire);
            let sim = back.to_event().expect("a sent event has a simulation form");
            assert_eq!(serde_json::to_value(&sim).unwrap(), serde_json::to_value(&event).unwrap());
        }
    }

    #[test]
    fn ricochet_carries_its_heading_both_ways() {
        let wire = WireEvent::Ricochet { slot: 1, x: 4, y: 8, heading: 64 };
        assert_eq!(wire.ricochet_heading(), Some(90.0));
        let Some(Event::Ricochet { slot, x, y, heading }) = wire.to_event() else { panic!("a ricochet has a simulation form") };
        assert_eq!((slot, x, y, heading), (1, 1.0, 2.0, 90.0));
        let back = WireEvent::from_event(&Event::Ricochet { slot: 1, x: 1.0, y: 2.0, heading: 90.4 }).unwrap();
        assert_eq!(back, wire, "the heading lands on its nearest step");
        assert!(WireEvent::try_from(&Event::Retreat { slot: 1, on: true }).is_err());
    }

    #[test]
    fn name_vocabularies_round_trip() {
        for what in IgnitedWhat::ALL {
            assert_eq!(IgnitedWhat::parse(what.name()), Some(what));
        }
        assert_eq!(IgnitedWhat::parse("lava"), None);
        for weapon in WeaponKind::ALL {
            let Some(WireEvent::Fired { weapon: back, .. }) =
                WireEvent::from_event(&Event::Fired { slot: 0, weapon: weapon.name() })
            else {
                panic!("Fired is sent")
            };
            assert_eq!(back, weapon);
        }
    }

    #[test]
    fn wide_slots_saturate_rather_than_wrap() {
        let wire = WireEvent::from_event(&Event::TankEntered { slot: 70_000 }).unwrap();
        assert_eq!(wire, WireEvent::TankEntered { slot: u16::MAX });
    }
}
