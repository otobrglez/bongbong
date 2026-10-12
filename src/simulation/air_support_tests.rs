//! Headless scenario tests for air support (docs/air-support.md "Tests"):
//! one per promised rule, on the default 34 x 17 field with the seat at
//! (96, 192) facing east, the crates and enemies placed by hand. Tests
//! cannot touch the global tuning table, so every number is read off the
//! defaults.

use super::*;
use crate::ai::Role;
use crate::air::{AirKey, AirStrike};
use crate::air_support::{AirStage, Sortie};

const W: f32 = 1088.0;
const H: f32 = 544.0;
const DT: f32 = 1.0 / 60.0;

/// The seat's pivot.
const SEAT: Position = Position::new(96.0, 192.0);

/// Six cells east of the seat: inside its sight box.
const NEAR: Position = Position::new(288.0, 192.0);

/// Across the field: outside the seat's sight box.
const FAR: Position = Position::new(1000.0, 192.0);

/// A round on an open field, nobody shielded, no banner, the seat facing
/// east carrying the rod.
fn round() -> Game {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.show_intro = false;
    game.level_overrides.mission = Some(Mission::Destroy);
    let map = "version = 1\ntanks = 0\ncells.\"3,6\" = { kind = \"start\" }\n";
    game.map = MapFile::from_toml_str(map).expect("test map parses");
    game.init(W, H);
    for tank in game.world.query_mut::<&mut Tank>() {
        tank.shield_hp = 0.0;
        tank.shield_timer = 0.0;
    }
    let seat = game.player().expect("a seat");
    game.place_tank(seat, SEAT, Some(90.0)).expect("placed");
    with_tank_mut(&game.world, seat, |t| {
        t.disarm();
        t.take_weapon(ActiveWeapon::RodFromGod);
    });
    game
}

/// A parked enemy at `at`: it cannot drive or shoot, and has no shield.
fn parked(game: &mut Game, at: Position) -> Entity {
    let slot = game.debug_spawn_enemy(at, Some(1), Some(Role::Player)).expect("spawns");
    let entity = game.tank_entity_by_slot(slot).expect("exists");
    with_tank_mut(&game.world, entity, |t| {
        t.speed_scale = 0.0;
        t.shells_ammo = 0;
        t.disarm();
        t.shield_hp = 0.0;
        t.shield_timer = 0.0;
    });
    entity
}

/// An air crate of `sortie` at `at`.
fn air_crate(game: &mut Game, sortie: Sortie, at: Position) {
    game.world.spawn((Pickup::placed(sortie.pickup(), at),));
}

/// One tick with every enemy's gun held: a parked enemy's shells come
/// back over a 15 s count, and the seat must take nothing but what air
/// support does.
fn tick(game: &mut Game) {
    for (tank, _) in game.world.query_mut::<(&mut Tank, &crate::ai::Ai)>() {
        tank.fire_cooldown = 1.0;
    }
    game.update(Input::default(), DT, W, H);
}

fn idle(game: &mut Game, ticks: u32) -> Vec<Event> {
    let mut seen = Vec::new();
    for _ in 0..ticks {
        tick(game);
        seen.extend(game.events().iter().cloned());
    }
    seen
}

/// Step until an `AirSupportLeft`, at most `cap` ticks: every event.
fn until_left(game: &mut Game, cap: u32) -> Vec<Event> {
    let mut seen = Vec::new();
    for _ in 0..cap {
        tick(game);
        let events = game.events().to_vec();
        let left = events.iter().any(|e| matches!(e, Event::AirSupportLeft { .. }));
        seen.extend(events);
        if left {
            return seen;
        }
    }
    panic!("air support never left in {cap} ticks");
}

fn delay_ticks() -> u32 {
    crate::tank::ticks_of(tuning().air_support_delay_seconds)
}

fn seat(game: &Game) -> Entity {
    game.player().expect("a seat")
}

fn damage(game: &Game, e: Entity) -> f32 {
    with_tank(&game.world, e, |t| t.damage)
}

fn count<F: Fn(&Event) -> bool>(events: &[Event], f: F) -> usize {
    events.iter().filter(|e| f(e)).count()
}

/// An enemy driving over an air crate leaves it where it is
/// (`Tank::wants_pickup`): air support answers a seat's radio.
#[test]
fn an_enemy_never_takes_an_air_crate() {
    let mut game = round();
    parked(&mut game, NEAR);
    air_crate(&mut game, Sortie::Jet, NEAR);
    let seen = idle(&mut game, 5);
    assert_eq!(count(&seen, |e| matches!(e, Event::PickupCollected { .. })), 0);
    assert!(game.air_calls().is_empty());
    assert_eq!(game.world.query::<&Pickup>().iter().count(), 1);
}

/// The crate calls a sortie and touches nothing on the tank: the special
/// it carries stays. Nothing arrives before the count runs out; the
/// aircraft arrives when it does.
#[test]
fn the_crate_calls_after_the_delay_and_keeps_the_special() {
    let mut game = round();
    parked(&mut game, NEAR);
    air_crate(&mut game, Sortie::Jet, SEAT);
    let seen = idle(&mut game, 1);
    assert_eq!(count(&seen, |e| matches!(e, Event::AirSupportCalled { arrive: Some(_), .. })), 1);
    assert_eq!(with_tank(&game.world, seat(&game), |t| t.special()), Some(ActiveWeapon::RodFromGod));
    let before = idle(&mut game, delay_ticks() - 2);
    assert_eq!(count(&before, |e| matches!(e, Event::AirSupportArrived { .. })), 0, "nothing comes early");
    assert!(game.aircraft().is_empty());
    let after = idle(&mut game, 3);
    assert_eq!(count(&after, |e| matches!(e, Event::AirSupportArrived { armed: true, .. })), 1);
    assert_eq!(game.aircraft().len(), 1);
    assert!(game.air_calls().is_empty());
}

/// The jet's run hurts the enemies inside the seat's sight box and leaves
/// the one outside it, the seat and itself untouched; a full-health tank
/// on its aim point is wrecked.
#[test]
fn the_jet_wrecks_what_is_in_the_box_and_spares_the_rest() {
    let mut game = round();
    let near = parked(&mut game, NEAR);
    let far = parked(&mut game, FAR);
    air_crate(&mut game, Sortie::Jet, SEAT);
    let seen = until_left(&mut game, delay_ticks() + 1200);
    assert_eq!(count(&seen, |e| matches!(e, Event::AirStrike { bomb: true, .. })), 2);
    assert_eq!(count(&seen, |e| matches!(e, Event::AirStrike { bomb: false, .. })), tuning().air_jet_gun_bursts as usize);
    assert!(with_tank(&game.world, near, |t| t.is_wreck()), "the tank on the aim point is wrecked");
    assert_eq!(damage(&game, far), 0.0, "outside the box is untouched");
    assert_eq!(damage(&game, seat(&game)), 0.0, "no friendly fire");
    assert!(game.aircraft().is_empty());
}

/// The heli flies in, hovers, fires its salvos at the enemy in the box and
/// leaves; the seat is untouched and the enemy outside the box too.
#[test]
fn the_heli_fires_its_salvos_and_leaves() {
    let mut game = round();
    let near = parked(&mut game, NEAR);
    let far = parked(&mut game, FAR);
    air_crate(&mut game, Sortie::Heli, SEAT);
    let mut stages = std::collections::BTreeSet::new();
    let mut seen = idle(&mut game, delay_ticks() + 1);
    for _ in 0..1800 {
        tick(&mut game);
        let events = game.events().to_vec();
        if let Some(a) = game.aircraft().first() {
            stages.insert(a.stage.code());
        }
        let left = events.iter().any(|e| matches!(e, Event::AirSupportLeft { .. }));
        seen.extend(events);
        if left {
            break;
        }
    }
    assert_eq!(stages.into_iter().collect::<Vec<_>>(), vec![AirStage::Inbound.code(), AirStage::Attack.code(), AirStage::Egress.code()]);
    assert_eq!(count(&seen, |e| matches!(e, Event::AirSalvo { .. })), tuning().air_heli_salvos as usize);
    assert!(count(&seen, |e| matches!(e, Event::AirSupportLeft { .. })) == 1, "it left");
    // Its last rockets may still be in the air when it leaves.
    idle(&mut game, 240);
    assert!(damage(&game, near) > 0.0, "the enemy in the box was hit");
    assert_eq!(damage(&game, far), 0.0, "outside the box is untouched");
    assert_eq!(damage(&game, seat(&game)), 0.0, "no friendly fire");
}

/// With no enemy in the seat's sight box the aircraft comes, passes over
/// the seat and leaves without firing: the crate is spent.
#[test]
fn with_no_enemy_in_the_box_it_fires_nothing() {
    for sortie in [Sortie::Heli, Sortie::Jet] {
        let mut game = round();
        let far = parked(&mut game, FAR);
        air_crate(&mut game, sortie, SEAT);
        let seen = until_left(&mut game, delay_ticks() + 1800);
        assert_eq!(count(&seen, |e| matches!(e, Event::AirSupportArrived { armed: false, .. })), 1, "{sortie:?}");
        assert_eq!(count(&seen, |e| matches!(e, Event::AirStrike { .. } | Event::AirSalvo { .. })), 0, "{sortie:?}");
        assert_eq!(damage(&game, far), 0.0);
        assert!(game.air_calls().is_empty());
    }
}

/// A second crate while one is counting queues a second sortie: it waits,
/// and its count starts when the first aircraft is gone.
#[test]
fn a_second_crate_queues_a_second_sortie() {
    let mut game = round();
    parked(&mut game, NEAR);
    air_crate(&mut game, Sortie::Jet, SEAT);
    idle(&mut game, 1);
    air_crate(&mut game, Sortie::Heli, SEAT);
    let seen = idle(&mut game, 1);
    assert_eq!(count(&seen, |e| matches!(e, Event::AirSupportCalled { arrive: None, sortie: Sortie::Heli, .. })), 1);
    assert_eq!(game.air_calls().len(), 2);
    until_left(&mut game, delay_ticks() + 1200);
    let next = game.air_calls().first().copied().expect("the heli's call is still there");
    assert_eq!(next.sortie, Sortie::Heli);
    let left = next.left(game.time).expect("its count started");
    assert!((left - tuning().air_support_delay_seconds).abs() < 0.05, "a full count from the first one leaving: {left}");
}

/// Bullets bring the heli down after `air_heli_hits`; anything else at
/// once. Down, it is gone and the seat's next call starts its count.
#[test]
fn the_heli_comes_down_to_bullets_and_the_next_call_counts() {
    let mut game = round();
    parked(&mut game, NEAR);
    air_crate(&mut game, Sortie::Heli, SEAT);
    idle(&mut game, 1);
    air_crate(&mut game, Sortie::Jet, SEAT);
    idle(&mut game, delay_ticks() + 30);
    let heli = game.aircraft().first().copied().expect("the heli is up");
    assert!(game.air_targets().iter().any(|a| a.key == AirKey::Aircraft(heli.id)));
    let mut f = Frame::new(DT, W, H, game.rng.take().unwrap(), Terrain::build(&game.world, W, H, &game.grass_cells, &game.water));
    for _ in 1..tuning().air_heli_hits {
        assert!(!game.strike_air(&mut f, AirKey::Aircraft(heli.id), AirStrike::Bullet));
    }
    assert!(game.strike_air(&mut f, AirKey::Aircraft(heli.id), AirStrike::Bullet));
    assert!(f.events.iter().any(|e| matches!(e, Event::AircraftDowned { by: "bullet", .. })));
    assert!(game.aircraft().is_empty());
    let next = game.air_calls().first().copied().expect("the jet's call");
    assert!(next.arrive_at.is_some(), "its count started");
    game.rng = Some(f.rng);
}

/// The jet is no air target: nothing can strike it.
#[test]
fn the_jet_is_no_air_target() {
    let mut game = round();
    parked(&mut game, NEAR);
    air_crate(&mut game, Sortie::Jet, SEAT);
    idle(&mut game, delay_ticks() + 2);
    assert_eq!(game.aircraft().len(), 1);
    assert!(game.air_targets().is_empty());
}

/// The same seed and the same crate play the same sortie, event for event.
#[test]
fn a_sortie_replays_bit_for_bit() {
    let run = || {
        let mut game = round();
        parked(&mut game, NEAR);
        parked(&mut game, Position::new(320.0, 260.0));
        air_crate(&mut game, Sortie::Heli, SEAT);
        let seen = idle(&mut game, delay_ticks() + 900);
        serde_json::to_string(&seen).unwrap()
    };
    assert_eq!(run(), run());
}
