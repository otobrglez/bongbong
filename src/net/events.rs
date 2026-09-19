//! `simulation::Event` on the wire (docs/online-coop-prd.md §4.4).
//!
//! The simulation's `Event` carries `&'static str` names and `f32`
//! positions and only serialises; `WireEvent` is its owned mirror that
//! goes both ways, with positions in quarter pixels and every name as a
//! small enum. `WireEvent::from_event` is the one classifier: an
//! exhaustive `match` with no wildcard, so a variant added to `Event` fails
//! to compile here until it is either mirrored or put on the not-sent
//! list. The AI's trace (`AiAction`, `EngageSlot`, `StuckEscape`, `Breach`,
//! `Retreat`, `Alert`, `Retarget`) and `PhysicsQuarantine` (logged
//! server-side) never travel.
//!
//! `to_event` gives the replica the `Event` its presentation layer already
//! reads (`fx.rs` diffs `game.events()`), the quantised positions
//! dequantised.

use serde::{Deserialize, Serialize};

use crate::frog::Side;
use crate::level::{Mission, SpawnKind, Tier};
use crate::net::wire::{RoundOutcome, WeaponKind, dequantise_heading, dequantise_pos, quantise_pos};
use crate::obstacle::{Drum, Material};
use crate::pickup::PickupKind;
use crate::simulation::{Event, HitTarget};

/// The serde tags (`Event`'s `event` field) of the variants
/// `WireEvent::from_event` never sends.
pub const NOT_SENT: [&str; 8] = [
    "physics_quarantine",
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
}

impl IgnitedWhat {
    /// Every kind, in wire order.
    pub const ALL: [IgnitedWhat; 7] = [
        IgnitedWhat::Ground,
        IgnitedWhat::Oil,
        IgnitedWhat::Wood,
        IgnitedWhat::Tree,
        IgnitedWhat::Drum,
        IgnitedWhat::Sandbag,
        IgnitedWhat::Fence,
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
    Fired { slot: u16, weapon: WeaponKind },
    Hit { target: WireHitTarget, damage: f32, killed: bool, x: i16, y: i16 },
    Wreck { slot: u16, x: i16, y: i16 },
    Ram { slot: u16, other_slot: u16, damage: f32 },
    PickupCollected { slot: u16, kind: PickupKind },
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
    Teleported { slot: u16, x: i16, y: i16, to_x: i16, to_y: i16 },
    FireStarted { x: i16, y: i16, pool: bool },
    Ignited { x: i16, y: i16, what: IgnitedWhat },
    CookOff { x: i16, y: i16 },
    /// `slot`'s shell bounced off iron or a barrel at (`x`, `y`) and flies
    /// on along `heading` (`wire::quantise_heading`). Reserved: the
    /// simulation does not emit a ricochet yet (its bounces are silent,
    /// docs/online-coop-prd.md §3), so `to_event` has nothing to map this
    /// to and returns `None`.
    Ricochet { slot: u16, x: i16, y: i16, heading: u8 },
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
                WireEvent::Fired { slot: slot_u16(slot), weapon: kind }
            }
            Event::Hit { target, damage, killed, x, y } => {
                WireEvent::Hit { target: target.into(), damage, killed, x: q(x), y: q(y) }
            }
            Event::Wreck { slot, x, y } => WireEvent::Wreck { slot: slot_u16(slot), x: q(x), y: q(y) },
            Event::Ram { slot, other_slot, damage } => {
                WireEvent::Ram { slot: slot_u16(slot), other_slot: slot_u16(other_slot), damage }
            }
            Event::PickupCollected { slot, kind } => WireEvent::PickupCollected { slot: slot_u16(slot), kind },
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
            Event::DrumLaunched { x, y, to_x, to_y } => {
                WireEvent::DrumLaunched { x: q(x), y: q(y), to_x: q(to_x), to_y: q(to_y) }
            }
            Event::Teleported { slot, x, y, to_x, to_y } => {
                WireEvent::Teleported { slot: slot_u16(slot), x: q(x), y: q(y), to_x: q(to_x), to_y: q(to_y) }
            }
            Event::FireStarted { x, y, pool } => WireEvent::FireStarted { x: q(x), y: q(y), pool },
            Event::Ignited { x, y, what } => {
                let kind = IgnitedWhat::parse(what).unwrap_or_else(|| {
                    debug_assert!(false, "unknown ignition target {what:?} in Event::Ignited");
                    IgnitedWhat::Ground
                });
                WireEvent::Ignited { x: q(x), y: q(y), what: kind }
            }
            Event::CookOff { x, y } => WireEvent::CookOff { x: q(x), y: q(y) },
            // Never sent: logged on the server.
            Event::PhysicsQuarantine { .. } => return None,
            // Never sent: the AI's trace.
            Event::AiAction { .. }
            | Event::EngageSlot { .. }
            | Event::StuckEscape { .. }
            | Event::Breach { .. }
            | Event::Retreat { .. }
            | Event::Alert { .. }
            | Event::Retarget { .. } => return None,
        })
    }

    /// The simulation `Event` the replica hands its presentation layer;
    /// `None` for `Ricochet`, which the simulation has no counterpart for.
    pub fn to_event(&self) -> Option<Event> {
        let d = dequantise_pos;
        Some(match *self {
            WireEvent::RoundStarted { seed, enemies, mission, spawn } => {
                Event::RoundStarted { seed, enemies: enemies as usize, mission, spawn }
            }
            WireEvent::RoundEnded { outcome } => Event::RoundEnded { outcome: outcome.into() },
            WireEvent::Fired { slot, weapon } => Event::Fired { slot: slot as usize, weapon: weapon.name() },
            WireEvent::Hit { target, damage, killed, x, y } => {
                Event::Hit { target: target.into(), damage, killed, x: d(x), y: d(y) }
            }
            WireEvent::Wreck { slot, x, y } => Event::Wreck { slot: slot as usize, x: d(x), y: d(y) },
            WireEvent::Ram { slot, other_slot, damage } => {
                Event::Ram { slot: slot as usize, other_slot: other_slot as usize, damage }
            }
            WireEvent::PickupCollected { slot, kind } => Event::PickupCollected { slot: slot as usize, kind },
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
            WireEvent::DrumLaunched { x, y, to_x, to_y } => {
                Event::DrumLaunched { x: d(x), y: d(y), to_x: d(to_x), to_y: d(to_y) }
            }
            WireEvent::Teleported { slot, x, y, to_x, to_y } => {
                Event::Teleported { slot: slot as usize, x: d(x), y: d(y), to_x: d(to_x), to_y: d(to_y) }
            }
            WireEvent::FireStarted { x, y, pool } => Event::FireStarted { x: d(x), y: d(y), pool },
            WireEvent::Ignited { x, y, what } => Event::Ignited { x: d(x), y: d(y), what: what.name() },
            WireEvent::CookOff { x, y } => Event::CookOff { x: d(x), y: d(y) },
            WireEvent::Ricochet { .. } => return None,
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
            Event::PickupCollected { slot: 1, kind: PickupKind::FrogHealth },
            Event::PickupRespawned { kind: PickupKind::Shield, x: 32.0, y: 32.0 },
            Event::Deflected { slot: 2, x: 1.5, y: -2.25 },
            Event::ShieldBroken { slot: 2, x: 8.0, y: 9.0 },
            Event::ShellsCollided { x: 500.0, y: 250.0 },
            Event::FrogBite { side: Side::Enemy, slot: 4, damage: 30.0, killed: true },
            Event::FrogHealed { side: Side::Player, slot: 0, amount: 40.0, x: 544.0, y: 272.0 },
            Event::WaveStarted { wave: 3, size: 6, tier: Tier::Heavy },
            Event::TankEntered { slot: 9 },
            Event::WreckRemoved { slot: 9 },
            Event::ObstacleDestroyed { material: Material::Pine, x: 96.0, y: 96.0 },
            Event::Blast { x: 128.0, y: 160.0, chained: true, drum: Drum::Fuel },
            Event::DrumLaunched { x: 128.0, y: 160.0, to_x: 256.0, to_y: 160.0 },
            Event::Teleported { slot: 1, x: 64.0, y: 64.0, to_x: 960.0, to_y: 480.0 },
            Event::FireStarted { x: 48.0, y: 48.0, pool: true },
            Event::Ignited { x: 48.0, y: 80.0, what: "sandbag" },
            Event::CookOff { x: 64.0, y: 96.75 },
            Event::PhysicsQuarantine { bodies: 1, colliders: 2 },
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
        assert_eq!(seen.len(), 31, "one sample per Event variant");
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
    fn ricochet_is_wire_only() {
        let wire = WireEvent::Ricochet { slot: 1, x: 4, y: 8, heading: 64 };
        assert!(wire.to_event().is_none());
        assert_eq!(wire.ricochet_heading(), Some(90.0));
        assert!(Event::try_from(&wire).is_err());
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
