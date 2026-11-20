//! Headless scenario tests for the jet (docs/air-support.md "Tests"), on
//! `air_support_tests.rs`'s round.

use super::air_support_tests::*;
use super::*;
use crate::air_support::{AirStage, Aircraft, Sortie};
use crate::math::atan2;

/// One tick of a sortie: the jet after it and what happened in it.
struct Seen {
    jet: Aircraft,
    events: Vec<Event>,
}

impl Seen {
    /// Where this tick's cannon bursts (`bombs` false) or bombs landed.
    fn strikes(&self, bombs: bool) -> Vec<Position> {
        self.events
            .iter()
            .filter_map(|e| match *e {
                Event::AirStrike { bomb, x, y, .. } if bomb == bombs => Some(Position::new(x, y)),
                _ => None,
            })
            .collect()
    }

    fn arrived(&self) -> bool {
        self.events.iter().any(|e| matches!(e, Event::AirSupportArrived { .. }))
    }
}

/// The longest a jet is on the field, in ticks: its last second, and a
/// straight line out across the field from there.
fn longest() -> u32 {
    let t = tuning();
    crate::tank::ticks_of(t.air_jet_sortie_seconds + (W + H + 2.0 * t.air_edge_margin_px) / t.air_jet_speed)
}

/// Play the crate's call out: every tick the jet is on the field, from the
/// one it arrives in to the one before it is gone. `each` runs after every
/// tick of the round, the jet there or not. Panics if the jet is still
/// there after `longest`.
fn sortie(game: &mut Game, mut each: impl FnMut(&mut Game, Option<&Aircraft>)) -> Vec<Seen> {
    let mut seen = Vec::new();
    for _ in 0..delay_ticks() + 2 + longest() {
        tick(game);
        let events = game.events().to_vec();
        let jet = game.aircraft().first().copied();
        each(game, jet.as_ref());
        if events.iter().any(|e| matches!(e, Event::AirSupportLeft { .. })) {
            return seen;
        }
        if let Some(jet) = jet {
            seen.push(Seen { jet, events });
        }
    }
    panic!("the jet never left: {:?}", game.aircraft());
}

fn strikes(seen: &[Seen], bombs: bool) -> Vec<Position> {
    seen.iter().flat_map(|s| s.strikes(bombs)).collect()
}

/// The stages the sortie went through, in order.
fn stages(seen: &[Seen]) -> Vec<AirStage> {
    let mut out: Vec<AirStage> = seen.iter().map(|s| s.jet.stage).collect();
    out.dedup();
    out
}

fn at(game: &Game, e: Entity) -> Position {
    with_tank(&game.world, e, |t| t.position)
}

/// The angle from `a` round to `b` (radians, signed), the short way.
fn round_from(a: Vec2, b: Vec2) -> f32 {
    atan2(a.x * b.y - a.y * b.x, a.dot(b))
}

/// The jet runs in on the enemy inside the seat's sight box - a whole
/// stripe of its cannon, then two bombs - and wrecks a full-health tank;
/// the one outside the box and the seat are untouched. With nothing left
/// to attack it goes home with its other pass.
#[test]
fn the_jet_wrecks_what_is_in_the_box_and_spares_the_rest() {
    let mut game = round();
    let near = parked(&mut game, NEAR);
    let far = parked(&mut game, FAR);
    air_crate(&mut game, Sortie::Jet, SEAT);
    let seen = sortie(&mut game, |_, _| {});
    assert_eq!(strikes(&seen, false).len(), tuning().air_jet_gun_bursts as usize);
    let bombs = strikes(&seen, true);
    assert_eq!(bombs.len(), 2);
    assert!(((bombs[0] + bombs[1]) * 0.5).distance_to(NEAR) < 8.0, "astride the tank: {bombs:?}");
    assert!(with_tank(&game.world, near, |t| t.is_wreck()), "the tank it ran in on is wrecked");
    assert_eq!(damage(&game, far), 0.0, "outside the box is untouched");
    assert_eq!(damage(&game, seat(&game)), 0.0, "no friendly fire");
    assert_eq!(seen.last().expect("it flew").jet.ammo as i32, tuning().air_jet_passes - 1, "one pass was enough");
    assert!(game.aircraft().is_empty());
}

/// The jet follows its target: a tank that is somewhere else by the time
/// the jet is over the field is flown at where it is, and the bombs land
/// astride where it is, not where it was.
#[test]
fn the_jet_follows_a_target_that_moves() {
    let moved = Position::new(416.0, 288.0);
    let mut game = round();
    let near = parked(&mut game, NEAR);
    air_crate(&mut game, Sortie::Jet, SEAT);
    let mut jumped = false;
    let seen = sortie(&mut game, |game, jet| {
        if !jumped && jet.is_some_and(|jet| jet.stage == AirStage::Attack) {
            jumped = true;
            game.place_tank(near, moved, None).expect("placed");
        }
    });
    assert!(jumped, "it was on its run when the tank moved");
    let bombs = strikes(&seen, true);
    assert_eq!(bombs.len(), 2);
    let middle = (bombs[0] + bombs[1]) * 0.5;
    assert!(middle.distance_to(moved) < 12.0, "astride where it is: {bombs:?}");
    assert!(middle.distance_to(NEAR) > 100.0, "not where it was: {bombs:?}");
    assert!(with_tank(&game.world, near, |t| t.is_wreck()));
    let turned = seen.iter().map(|s| s.jet.bank.abs()).fold(0.0, f32::max);
    assert!(turned > 0.5, "it banked round after it: {turned}");
}

/// A tank that drives is bombed where it is when the bombs go.
#[test]
fn the_jet_bombs_a_driving_tank_where_it_is() {
    let mut game = round();
    let slot = game.debug_spawn_enemy(NEAR, Some(1), Some(crate::ai::Role::Player)).expect("spawns");
    let driver = game.tank_entity_by_slot(slot).expect("exists");
    with_tank_mut(&game.world, driver, |t| {
        t.disarm();
        t.shield_hp = 0.0;
        t.shield_timer = 0.0;
    });
    air_crate(&mut game, Sortie::Jet, SEAT);
    let mut was = Vec::new();
    let seen = sortie(&mut game, |game, jet| {
        // Patched up, so it drives the whole sortie through.
        with_tank_mut(&game.world, driver, |t| t.damage = 0.0);
        if jet.is_some() {
            was.push(at(game, driver));
        }
    });
    let mut pairs = 0;
    for (s, tank) in seen.iter().zip(&was) {
        let bombs = s.strikes(true);
        if !bombs.is_empty() {
            pairs += 1;
            assert!(((bombs[0] + bombs[1]) * 0.5).distance_to(*tank) < 12.0, "astride the tank at {tank:?}: {bombs:?}");
        }
    }
    assert!(pairs >= 1, "it caught the tank");
    let driven = was.windows(2).map(|w| w[0].distance_to(w[1])).sum::<f32>();
    assert!(driven > 64.0, "the tank drove: {driven} px");
}

/// With passes left the jet flies on past its target, turns back and
/// runs in again: two pairs of bombs on a tank that outlives the first,
/// then straight home.
#[test]
fn the_jet_comes_back_for_a_second_pass() {
    let mut game = round();
    let near = parked(&mut game, NEAR);
    air_crate(&mut game, Sortie::Jet, SEAT);
    let seen = sortie(&mut game, |game, _| with_tank_mut(&game.world, near, |t| t.damage = 0.0));
    assert_eq!(stages(&seen), vec![AirStage::Inbound, AirStage::Attack, AirStage::Loiter, AirStage::Attack, AirStage::Egress]);
    let passes = tuning().air_jet_passes as usize;
    assert_eq!(strikes(&seen, true).len(), 2 * passes, "two bombs a pass");
    let dropped: Vec<&Seen> = seen.iter().filter(|s| !s.strikes(true).is_empty()).collect();
    assert_eq!(dropped.iter().map(|s| s.jet.ammo as usize).collect::<Vec<_>>(), (0..passes).rev().collect::<Vec<_>>(), "a pass each");
    let (first, second) = (dropped[0].jet.heading, dropped[1].jet.heading);
    assert!(round_from(first, second).abs() > 1.0, "the second from another side: {first:?} then {second:?}");
    assert_eq!(damage(&game, seat(&game)), 0.0, "no friendly fire");
}

/// With no enemy in the seat's sight box the jet comes unarmed, flies to
/// its seat, goes once round it on its ring, rolls out and leaves without
/// firing: the crate is spent.
#[test]
fn with_no_enemy_in_view_the_jet_circles_its_seat_and_leaves() {
    let mut game = round();
    let far = parked(&mut game, FAR);
    air_crate(&mut game, Sortie::Jet, SEAT);
    let seen = sortie(&mut game, |_, _| {});
    assert!(seen[0].events.iter().any(|e| matches!(e, Event::AirSupportArrived { armed: false, .. })));
    assert!(seen.iter().all(|s| s.strikes(false).is_empty() && s.strikes(true).is_empty()), "it fired nothing");
    assert_eq!(stages(&seen), vec![AirStage::Inbound, AirStage::Loiter, AirStage::Egress]);
    let ring = tuning().air_jet_orbit_px;
    let circle: Vec<&Seen> = seen.iter().filter(|s| s.jet.stage == AirStage::Loiter).collect();
    // Its nose comes all the way round, and it flies all but the join
    // round its seat.
    let nose: f32 = circle.windows(2).map(|w| round_from(w[0].jet.heading, w[1].jet.heading)).sum();
    assert!((nose.abs() - std::f32::consts::TAU).abs() < 0.1, "its nose came round {nose}");
    let flown: f32 = circle.windows(2).map(|w| round_from(w[0].jet.ground - SEAT, w[1].jet.ground - SEAT)).sum();
    assert!(flown.abs() > std::f32::consts::TAU * 0.9, "it flew {flown} round its seat");
    // It comes onto the ring from outside and holds it.
    let from: Vec<f32> = circle.iter().map(|s| s.jet.ground.distance_to(SEAT)).collect();
    assert!(from.iter().all(|d| *d > ring - 8.0 && *d < ring * 1.3), "{from:?}");
    let on = from.iter().position(|d| (d - ring).abs() <= 8.0).expect("it reached its ring");
    assert!(from[on..].iter().all(|d| (d - ring).abs() <= 8.0), "on the ring it stays on it: {from:?}");
    assert!((from.len() - on) * 4 > from.len() * 3, "most of the circle is on the ring: from {on} of {}", from.len());
    assert!(circle.iter().map(|s| s.jet.bank.abs()).fold(0.0, f32::max) > 0.5, "banked into it");
    assert_eq!(seen.last().expect("it flew").jet.bank, 0.0, "and level on its way out");
    assert_eq!(damage(&game, far), 0.0);
    assert!(game.air_calls().is_empty() && game.aircraft().is_empty());
}

/// An enemy that comes into view while the jet circles is attacked: the
/// circle is broken off for a run on it.
#[test]
fn an_enemy_that_shows_while_the_jet_circles_is_attacked() {
    let shows = Position::new(288.0, 320.0);
    let mut game = round();
    let far = parked(&mut game, FAR);
    air_crate(&mut game, Sortie::Jet, SEAT);
    let mut shown = false;
    let mut before = 0;
    let seen = sortie(&mut game, |game, jet| {
        if !shown {
            before += jet.is_some() as usize;
            if jet.is_some_and(|jet| jet.stage == AirStage::Loiter && jet.stage_time > 1.0) {
                shown = true;
                game.place_tank(far, shows, None).expect("placed");
            }
        }
    });
    assert!(shown, "it was circling");
    assert!(seen[..before].iter().all(|s| s.strikes(false).is_empty() && s.strikes(true).is_empty()), "nothing fired at nothing");
    assert!(seen[before..].iter().any(|s| s.jet.stage == AirStage::Attack), "it ran in on the tank that showed");
    let bombs = strikes(&seen, true);
    assert_eq!(bombs.len(), 2);
    assert!(((bombs[0] + bombs[1]) * 0.5).distance_to(shows) < 12.0, "astride it: {bombs:?}");
    assert!(with_tank(&game.world, far, |t| t.is_wreck()));
}

/// The jet always leaves: a tank that is never where the jet comes for
/// it - across the box every time the jet gets near - cannot keep it past
/// its last second.
#[test]
fn the_jet_always_leaves() {
    let spots = [NEAR, Position::new(160.0, 400.0)];
    let mut game = round();
    let near = parked(&mut game, NEAR);
    air_crate(&mut game, Sortie::Jet, SEAT);
    let mut jumps = 0;
    let seen = sortie(&mut game, |game, jet| {
        with_tank_mut(&game.world, near, |t| t.damage = 0.0);
        let Some(jet) = jet else { return };
        let tank = at(game, near);
        if jet.ground.distance_to(tank) < 200.0 {
            let away = if tank.distance_to(spots[0]) < tank.distance_to(spots[1]) { spots[1] } else { spots[0] };
            game.place_tank(near, away, None).expect("placed");
            jumps += 1;
        }
    });
    assert!(jumps >= 2, "the tank kept out of its way: {jumps}");
    assert!(seen.len() as u32 <= longest(), "{} ticks on the field", seen.len());
    let home = seen.iter().position(|s| s.jet.stage == AirStage::Egress).expect("it turned for home");
    assert!(home as u32 <= crate::tank::ticks_of(tuning().air_jet_sortie_seconds) + 1, "for home by its last second: tick {home}");
    assert!(seen[home..].iter().all(|s| s.jet.stage == AirStage::Egress && s.jet.heading == seen[home].jet.heading), "and straight out");
    assert_eq!(damage(&game, seat(&game)), 0.0, "no friendly fire");
}

/// The jet fires only over the field: every burst and bomb comes on a tick
/// its ground point is inside it, and none on the tick it arrives -
/// wherever the seat stands, on both its passes.
#[test]
fn the_jet_fires_only_over_the_field() {
    for (seat_at, enemy_at) in [(SEAT, NEAR), (Position::new(96.0, 64.0), Position::new(200.0, 40.0)), (Position::new(990.0, 480.0), Position::new(900.0, 500.0))] {
        let mut game = round();
        let seat = seat(&game);
        game.place_tank(seat, seat_at, Some(90.0)).expect("placed");
        let enemy = parked(&mut game, enemy_at);
        air_crate(&mut game, Sortie::Jet, seat_at);
        let seen = sortie(&mut game, |game, _| with_tank_mut(&game.world, enemy, |t| t.damage = 0.0));
        for s in &seen {
            if s.strikes(false).len() + s.strikes(true).len() > 0 {
                assert!(!s.arrived(), "no strike on the tick it arrives");
                let g = s.jet.ground;
                assert!(g.x >= 0.0 && g.x <= W && g.y >= 0.0 && g.y <= H, "seat at {seat_at:?}: fired off the field at {g:?}");
            }
        }
        assert!(seen.iter().any(|s| s.jet.ground.y < 0.0 || s.jet.ground.y > H || s.jet.ground.x < 0.0 || s.jet.ground.x > W), "it was off the field at all");
        assert_eq!(strikes(&seen, true).len(), 2 * tuning().air_jet_passes as usize, "seat at {seat_at:?}: both passes made");
    }
}

/// An enemy tower is a target like an enemy tank: with only a gun tower in
/// the seat's sight box the jet comes armed and runs in on it. A bomb lands
/// either side of it, so a gun tower's 150 outlives one pass; the second
/// brings it down.
#[test]
fn the_jet_runs_in_on_an_enemy_tower_in_the_box() {
    let mut game = round_with("cells.\"9,6\" = { kind = \"gun_tower\", side = \"enemy\" }\n");
    // An enemy tank outside the box keeps the round going once the tower falls.
    parked(&mut game, FAR);
    let (standing, before) = tower_at(&game, (9, 6));
    assert!(standing);
    air_crate(&mut game, Sortie::Jet, SEAT);
    let mut after_one = None;
    let seen = sortie(&mut game, |game, jet| {
        if after_one.is_none() && jet.is_some_and(|jet| (jet.ammo as i32) < tuning().air_jet_passes) {
            after_one = Some(tower_at(game, (9, 6)));
        }
    });
    assert!(seen[0].events.iter().any(|e| matches!(e, Event::AirSupportArrived { armed: true, .. })), "armed for the tower");
    let (standing, left) = after_one.expect("a first pass");
    assert!(standing && left < before * 0.5, "one pass takes most of its health: {before} -> {left}");
    assert_eq!(strikes(&seen, true).len(), 4, "and it comes back for it");
    let (standing, after) = tower_at(&game, (9, 6));
    assert!(!standing, "the tower is down: {before} -> {after}");
}

/// The same seed and the same crate play the same jet sortie, event for
/// event and the jet bit for bit.
#[test]
fn a_jet_sortie_replays_bit_for_bit() {
    let run = || {
        let mut game = round();
        parked(&mut game, NEAR);
        parked(&mut game, Position::new(320.0, 260.0));
        air_crate(&mut game, Sortie::Jet, SEAT);
        let seen = sortie(&mut game, |_, _| {});
        (seen.iter().map(|s| s.jet).collect::<Vec<_>>(), serde_json::to_string(&seen.iter().flat_map(|s| s.events.clone()).collect::<Vec<_>>()).unwrap())
    };
    let (jets, events) = run();
    assert!(events.contains("AirStrike") || events.contains("air_strike"), "it attacked");
    assert_eq!((jets, events), run());
}
