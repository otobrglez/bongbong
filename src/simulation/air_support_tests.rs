//! Headless scenario tests for air support (docs/air-support.md "Tests"):
//! one per promised rule, on the default 34 x 17 field with the seat at
//! (96, 192) facing east, the crates and enemies placed by hand. Tests
//! cannot touch the global tuning table, so every number is read off the
//! defaults. Here: the call, the queue, the shoot-down and the round set
//! up that `air_heli_tests.rs` and `air_jet_tests.rs` share.

use super::*;
use crate::ai::Role;
use crate::air::{AirKey, AirStrike};
use crate::air_support::Sortie;

pub(super) const W: f32 = 1088.0;
pub(super) const H: f32 = 544.0;
pub(super) const DT: f32 = 1.0 / 60.0;

/// The seat's pivot.
pub(super) const SEAT: Position = Position::new(96.0, 192.0);

/// Six cells east of the seat: inside its sight box.
pub(super) const NEAR: Position = Position::new(288.0, 192.0);

/// Across the field: outside the seat's sight box.
pub(super) const FAR: Position = Position::new(1000.0, 192.0);

/// A round on an open field, nobody shielded, no banner, the seat facing
/// east carrying the rod.
pub(super) fn round() -> Game {
    round_with("")
}

/// `round` on a map with `extra` cells too.
pub(super) fn round_with(extra: &str) -> Game {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.show_intro = false;
    game.level_overrides.mission = Some(Mission::Destroy);
    let map = format!("version = 1\ntanks = 0\ncells.\"3,6\" = {{ kind = \"start\" }}\n{extra}");
    game.map = MapFile::from_toml_str(&map).expect("test map parses");
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
pub(super) fn parked(game: &mut Game, at: Position) -> Entity {
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
pub(super) fn air_crate(game: &mut Game, sortie: Sortie, at: Position) {
    game.world.spawn((Pickup::placed(sortie.pickup(), at),));
}

/// One tick with every enemy's gun held, the towers' too: a parked
/// enemy's shells come back over a 15 s count, and the seat must take
/// nothing but what air support does.
pub(super) fn tick(game: &mut Game) {
    for (tank, _) in game.world.query_mut::<(&mut Tank, &crate::ai::Ai)>() {
        tank.fire_cooldown = 1.0;
    }
    for tower in game.towers.values_mut() {
        tower.cooldown = 1.0;
        tower.charge = 0.0;
        tower.burst_left = 0;
    }
    game.update(Input::default(), DT, W, H);
}

pub(super) fn idle(game: &mut Game, ticks: u32) -> Vec<Event> {
    let mut seen = Vec::new();
    for _ in 0..ticks {
        tick(game);
        seen.extend(game.events().iter().cloned());
    }
    seen
}

/// Step until an `AirSupportLeft`, at most `cap` ticks: every event.
pub(super) fn until_left(game: &mut Game, cap: u32) -> Vec<Event> {
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

pub(super) fn delay_ticks() -> u32 {
    crate::tank::ticks_of(tuning().air_support_delay_seconds)
}

pub(super) fn seat(game: &Game) -> Entity {
    game.player().expect("a seat")
}

pub(super) fn damage(game: &Game, e: Entity) -> f32 {
    with_tank(&game.world, e, |t| t.damage)
}

pub(super) fn count<F: Fn(&Event) -> bool>(events: &[Event], f: F) -> usize {
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

/// The tower standing on map cell `cell`: whether it still stands and its
/// health.
pub(super) fn tower_at(game: &Game, cell: (i32, i32)) -> (bool, f32) {
    // A dead tower leaves `Game::towers` for its ruin.
    let Some(tower) = game.towers.get(&cell) else { return (false, 0.0) };
    let o = game.world.get::<&crate::obstacle::Obstacle>(tower.entity).expect("its tile");
    (!o.destroyed, o.health)
}

/// The seat's own towers are never a target: with only a player tower in
/// the box the aircraft comes unarmed and fires nothing.
#[test]
fn a_players_own_tower_is_no_target() {
    let mut game = round_with("cells.\"9,6\" = { kind = \"tesla\" }\n");
    // An enemy tank outside the box keeps the round going once the tower falls.
    parked(&mut game, FAR);
    air_crate(&mut game, Sortie::Jet, SEAT);
    let seen = until_left(&mut game, delay_ticks() + 1200);
    assert_eq!(count(&seen, |e| matches!(e, Event::AirSupportArrived { armed: false, .. })), 1);
    assert_eq!(count(&seen, |e| matches!(e, Event::AirStrike { .. })), 0);
}

/// An aircraft in the air is on the field's picture (`Game::paint_field`),
/// which is a headless round's screenshot: its sprite is drawn where it
/// flies.
#[test]
fn the_field_painter_draws_what_flies() {
    use crate::canvas::{CpuCanvas, Pixels, Sheet};
    use crate::math::Color;
    let cell = crate::air_support::AIRCRAFT_CELL as usize;
    let mut game = round();
    parked(&mut game, FAR);
    air_crate(&mut game, Sortie::Heli, SEAT);
    idle(&mut game, delay_ticks() + 240);
    let heli = game.aircraft().first().copied().expect("the heli is up");
    let paint = |sheet: bool| {
        let mut canvas = CpuCanvas::blank(W as usize, H as usize);
        if sheet {
            canvas.insert_sheet(Sheet::Aircraft, Pixels::filled(4 * cell, cell, Color::BLACK));
        }
        game.paint_field(&mut canvas, crate::game::PaintOptions { locate_cue: false });
        canvas
    };
    let (with, without) = (paint(true), paint(false));
    let at = heli.drawn();
    assert_eq!(with.pixel(at.x as usize, at.y as usize), Color::BLACK, "the heli at {at:?}");
    assert_ne!(without.pixel(at.x as usize, at.y as usize), Color::BLACK);
    assert_ne!(with.hash(), without.hash());
}
