//! Headless scenario tests for the helicopter (docs/air-support.md "Tests"), on
//! `air_support_tests.rs`'s round.

use super::air_support_tests::*;
use super::*;
use crate::ai::{Intent, in_sight_box};
use crate::air_support::{AirStage, Aircraft, Sortie};
use crate::tank::Dir;

/// Below the seat's sight box wherever it drives along its row: an enemy
/// here keeps the round going and is never in view.
const ASIDE: Position = Position::new(64.0, 500.0);

/// The helicopter in the air.
fn heli(game: &Game) -> Aircraft {
    game.aircraft().first().copied().expect("the heli is up")
}

fn seat_at(game: &Game) -> Position {
    with_tank(&game.world, seat(game), |t| t.position)
}

/// How far the helicopter's ground point is from its seat's tank.
fn apart(game: &Game) -> f32 {
    heli(game).ground.distance_to(seat_at(game))
}

/// How far from its tank the helicopter still counts as with it.
fn with_tank_px() -> f32 {
    tuning().air_heli_escort_px + tuning().air_heli_station_slack_px
}

/// Take the crate at the seat and step until the helicopter has come to
/// its tank: every event on the way.
fn call(game: &mut Game) -> Vec<Event> {
    air_crate(game, Sortie::Heli, SEAT);
    let mut seen = idle(game, delay_ticks() + 1);
    for _ in 0..600 {
        if game.aircraft().first().is_some_and(|a| a.stage != AirStage::Inbound) {
            return seen;
        }
        seen.extend(idle(game, 1));
    }
    panic!("the heli never came to its tank: {:?}", game.aircraft());
}

/// `ticks` ticks with the seat driving `dir`, every enemy's gun held as in
/// `tick`: every event, and the furthest the helicopter got from its tank.
fn drive(game: &mut Game, dir: Dir, ticks: u32) -> (Vec<Event>, f32) {
    let mut seen = Vec::new();
    let mut furthest = 0.0_f32;
    for _ in 0..ticks {
        for (tank, _) in game.world.query_mut::<(&mut Tank, &crate::ai::Ai)>() {
            tank.fire_cooldown = 1.0;
        }
        game.update(Input::single(Intent { move_dir: Some(dir), ..Intent::default() }), DT, W, H);
        seen.extend(game.events().iter().cloned());
        furthest = furthest.max(apart(game));
    }
    (seen, furthest)
}

fn salvos(events: &[Event]) -> usize {
    count(events, |e| matches!(e, Event::AirSalvo { .. }))
}

/// The heli comes in to where its tank is and then goes with it: the seat
/// drives off from a stand across the field and turns down it, and the
/// heli is never further from it than its station and the slack. With
/// nothing in view it fires nothing and keeps every salvo.
#[test]
fn the_heli_comes_to_its_tank_and_goes_where_it_drives() {
    let mut game = round();
    parked(&mut game, ASIDE);
    let mut seen = call(&mut game);
    assert!(apart(&game) <= with_tank_px(), "it came to its tank: {} px off", apart(&game));
    let (east, furthest_east) = drive(&mut game, Dir::Right, 200);
    let (south, furthest_south) = drive(&mut game, Dir::Down, 60);
    seen.extend(east);
    seen.extend(south);
    let driven = seat_at(&game);
    assert!(driven.x > SEAT.x + 500.0 && driven.y > SEAT.y + 100.0, "the seat drove: {driven:?}");
    assert!(furthest_east.max(furthest_south) <= with_tank_px(), "it fell {furthest_east} and {furthest_south} px behind");
    // The tank stopped, it settles on its station beside it.
    seen.extend(idle(&mut game, 180));
    let a = heli(&game);
    assert_eq!(a.stage, AirStage::Escort);
    assert!((apart(&game) - tuning().air_heli_escort_px).abs() < 4.0, "on station: {} px off", apart(&game));
    assert!(a.velocity.length() < 4.0, "hovering: {:?}", a.velocity);
    assert_eq!(salvos(&seen), 0);
    assert_eq!(a.ammo as i32, tuning().air_heli_salvos);
}

/// The heli fires every salvo it came with at the enemy in the seat's
/// sight box, one at a time and `air_heli_salvo_seconds` apart, and then
/// leaves; the seat is untouched and the enemy outside the box too.
#[test]
fn the_heli_fires_its_salvos_and_leaves() {
    let mut game = round();
    let near = parked(&mut game, NEAR);
    let far = parked(&mut game, FAR);
    air_crate(&mut game, Sortie::Heli, SEAT);
    let mut stages = Vec::new();
    let mut seen = idle(&mut game, delay_ticks() + 1);
    let mut fired_at = Vec::new();
    let mut hit = false;
    for now in 0..2400 {
        tick(&mut game);
        // Patched up between salvos, so it has a target for every one.
        hit |= damage(&game, near) > 0.0;
        with_tank_mut(&game.world, near, |t| t.damage = 0.0);
        let events = game.events().to_vec();
        if let Some(a) = game.aircraft().first() {
            if stages.last() != Some(&a.stage) {
                stages.push(a.stage);
            }
            if salvos(&events) > 0 {
                assert_eq!(salvos(&events), 1, "one salvo a tick");
                assert_eq!(a.ammo as usize, tuning().air_heli_salvos as usize - fired_at.len() - 1, "a salvo costs one");
                assert!(a.ground.distance_to(SEAT) <= with_tank_px(), "fired from beside its tank: {:?}", a.ground);
                fired_at.push(now);
            }
        }
        let left = events.iter().any(|e| matches!(e, Event::AirSupportLeft { .. }));
        seen.extend(events);
        if left {
            break;
        }
    }
    assert_eq!(stages, vec![AirStage::Inbound, AirStage::Attack, AirStage::Egress]);
    assert_eq!(salvos(&seen), tuning().air_heli_salvos as usize);
    assert!(fired_at.windows(2).all(|w| (w[1] - w[0]) as f32 * DT >= tuning().air_heli_salvo_seconds - 1e-3), "{fired_at:?}");
    for e in &seen {
        if let Event::AirSalvo { x, y, .. } = e {
            assert!(in_sight_box(SEAT, Position::new(*x, *y)), "a salvo at ({x}, {y}), off the seat's screen");
        }
    }
    assert_eq!(count(&seen, |e| matches!(e, Event::AirSupportLeft { .. })), 1, "it left");
    assert!(game.aircraft().is_empty());
    // Its last rockets may still be in the air when it leaves.
    idle(&mut game, 240);
    assert!(hit || damage(&game, near) > 0.0, "the enemy in the box was hit");
    assert_eq!(damage(&game, far), 0.0, "outside the box is untouched");
    assert_eq!(damage(&game, seat(&game)), 0.0, "no friendly fire");
}

/// With no enemy in the seat's sight box the heli fires nothing and does
/// not leave: a minute on it is still beside its tank with every salvo.
/// An enemy that comes into view then is attacked.
#[test]
fn with_no_enemy_in_view_the_heli_stays_until_one_comes() {
    let mut game = round();
    let far = parked(&mut game, FAR);
    let mut seen = call(&mut game);
    assert_eq!(count(&seen, |e| matches!(e, Event::AirSupportArrived { armed: false, .. })), 1);
    seen.extend(idle(&mut game, 3600));
    assert_eq!(salvos(&seen), 0);
    assert_eq!(count(&seen, |e| matches!(e, Event::AirSupportLeft { .. })), 0, "no clock sends it home");
    let a = heli(&game);
    assert_eq!(a.stage, AirStage::Escort);
    assert_eq!(a.ammo as i32, tuning().air_heli_salvos);
    assert!(apart(&game) <= with_tank_px());
    assert_eq!(damage(&game, far), 0.0);
    let near = parked(&mut game, NEAR);
    let mut hit = false;
    let mut later = Vec::new();
    for _ in 0..600 {
        later.extend(idle(&mut game, 1));
        hit |= with_tank(&game.world, near, |t| t.damage > 0.0);
        if hit {
            break;
        }
    }
    assert!(hit, "the enemy that came into view was hit");
    assert!(salvos(&later) >= 1);
    assert_eq!(heli(&game).stage, AirStage::Attack);
    assert_eq!(damage(&game, far), 0.0);
}

/// The heli puts its salvos into an enemy tower when it is the only target
/// in the box: its rockets bring a tesla down from outside the tesla's
/// reach (the towers' guns are held, a tesla's arc at the air is not), and
/// with nothing left in view it is back to escorting.
#[test]
fn the_heli_brings_an_enemy_tower_down() {
    let mut game = round_with("cells.\"9,6\" = { kind = \"tesla\", side = \"enemy\" }\n");
    // An enemy tank outside the box keeps the round going once the tower falls.
    parked(&mut game, FAR);
    let (standing, before) = tower_at(&game, (9, 6));
    assert!(standing);
    let mut seen = call(&mut game);
    assert_eq!(count(&seen, |e| matches!(e, Event::AirSupportArrived { armed: true, .. })), 1, "armed for the tower");
    for _ in 0..1500 {
        seen.extend(idle(&mut game, 1));
        if !tower_at(&game, (9, 6)).0 {
            break;
        }
    }
    let (standing, after) = tower_at(&game, (9, 6));
    assert!(!standing, "the tesla is down: {before} -> {after}");
    assert_eq!(count(&seen, |e| matches!(e, Event::AircraftDowned { .. })), 0, "it kept out of the tesla's reach");
    assert!(salvos(&seen) >= 1 && salvos(&seen) <= tuning().air_heli_salvos as usize);
    // The rockets still in the air land, and nothing more is fired.
    let after_it = idle(&mut game, 240);
    assert_eq!(salvos(&after_it), 0);
    if let Some(a) = game.aircraft().first() {
        assert_eq!(a.stage, AirStage::Escort, "rockets left and nothing in view: it stays");
        assert!(a.ammo > 0);
    }
}

/// A seat wrecked with nothing in view sends its heli home, the round
/// still on for the other seat.
#[test]
fn a_wrecked_seat_with_nothing_in_view_sends_the_heli_home() {
    let mut game = round();
    game.players = PlayerCount::TWO;
    game.init(W, H);
    for tank in game.world.query_mut::<&mut Tank>() {
        tank.shield_hp = 0.0;
        tank.shield_timer = 0.0;
    }
    let first = seat(&game);
    game.place_tank(first, SEAT, Some(90.0)).expect("placed");
    let far = parked(&mut game, FAR);
    call(&mut game);
    assert_eq!(heli(&game).stage, AirStage::Escort);
    game.debug_kill(0).expect("the seat is wrecked");
    let seen = until_left(&mut game, 600);
    assert!(with_tank(&game.world, first, |t| t.is_wreck()));
    assert_eq!(game.outcome, Outcome::Playing, "the other seat plays on");
    assert_eq!(salvos(&seen), 0);
    assert!(game.aircraft().is_empty());
    assert_eq!(damage(&game, far), 0.0);
}

/// The end screen sends the heli home whatever it has left and whatever is
/// in view: the only seat wrecked, the round is lost, and from the next
/// tick it is on its way out and fires nothing.
#[test]
fn the_end_screen_sends_the_heli_home() {
    let mut game = round();
    parked(&mut game, NEAR);
    call(&mut game);
    assert_eq!(heli(&game).stage, AirStage::Attack);
    game.debug_kill(0).expect("the seat is wrecked");
    for _ in 0..600 {
        if game.outcome != Outcome::Playing {
            break;
        }
        tick(&mut game);
    }
    assert_eq!(game.outcome, Outcome::Lost);
    let a = heli(&game);
    assert_eq!(a.stage, AirStage::Attack, "an enemy in view: only the end of the round sends it home");
    assert!(a.ammo > 0, "it had rockets left");
    tick(&mut game);
    assert_eq!(heli(&game).stage, AirStage::Egress);
    let seen = until_left(&mut game, 600);
    assert_eq!(salvos(&seen), 0);
    assert!(game.aircraft().is_empty());
}
