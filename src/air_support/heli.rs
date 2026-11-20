//! The helicopter's sortie (docs/air-support.md): a companion to the seat
//! that called it. `step` is all of it, a pure function of the aircraft,
//! the `AirView` and the fixed step.
//!
//! It comes in from its entry edge to where its seat's tank is now
//! (`AirStage::Inbound`), holds station `air_heli_escort_px` from it and
//! follows it as it drives (`Escort`), and with a target in view takes the
//! nearest one's side of the tank, no nearer it than
//! `air_heli_standoff_px`, nose on it, firing a salvo every
//! `air_heli_salvo_seconds` (`Attack`) - held while a seat or the players'
//! bong stands within the salvo's reach of the target (`AirView::clear`). It goes home (`Egress`) when its
//! ammunition is spent, on the end screen, when its seat has no tank, or
//! when that tank is a wreck with nothing in view. No clock sends it home:
//! with rockets left and nothing to shoot at it stays with its tank.
//!
//! The flight is a velocity held to `air_heli_speed` that changes no faster
//! than `air_heli_accel`: it rides its tank's own velocity, closes on its
//! station along the curve it can still brake on, and its nose turns no
//! faster than `air_heli_turn_rate_deg`.

use super::{AirAct, AirMark, AirStage, AirView, Aircraft, turn_toward, unit};
use crate::math::Vec2;
use crate::tuning::Tuning;
use crate::Position;

/// What the helicopter keeps between steps besides `Aircraft`'s own.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeliState {
    /// Seconds since its last salvo; since it arrived, before its first.
    pub since_salvo: f32,
    /// The way home (unit): set when it turns for it, zero until then.
    pub home: Vec2,
}

/// How far from the point it is fired at a salvo's blasts reach: a
/// missile's blast past the widest of the tubes' spread
/// (`Missile::impact_offset`).
pub(super) fn salvo_reach(t: &Tuning) -> f32 {
    t.missile_blast_radius + 1.5 * t.missile_impact_spread_px
}

/// One fixed step of a helicopter (`Aircraft::step`).
pub(super) fn step(a: &mut Aircraft, view: &AirView, dt: f32, t: &Tuning, acts: &mut Vec<AirAct>) {
    a.heli.since_salvo += dt;
    let seat = match view.seat {
        Some(seat) if a.stage != AirStage::Egress && !done(a, view, t) => seat,
        _ => {
            go_home(a, dt, t);
            return;
        }
    };
    let mark = view.marks.first();
    let gap = station(a, seat, view.marks, view.field, t) - a.ground;
    fly(a, view.seat_velocity + closing(gap, dt, t), dt, t);
    let with_tank = (a.ground - seat).length() <= t.air_heli_escort_px + t.air_heli_station_slack_px;
    if a.stage == AirStage::Inbound && !with_tank {
        point(a, a.velocity, dt, t);
        return;
    }
    let stage = if mark.is_some() { AirStage::Attack } else { AirStage::Escort };
    if a.stage != stage {
        a.enter(stage);
    }
    let Some(mark) = mark else {
        if a.velocity.length() > t.air_heli_nose_speed {
            point(a, a.velocity, dt, t);
        } else {
            a.bank = 0.0;
        }
        return;
    };
    let aim = mark.at - a.ground;
    point(a, aim, dt, t);
    let on_target = unit(aim).is_none_or(|dir| a.heading.dot(dir) >= crate::math::cos(t.air_heli_aim_deg.to_radians()));
    // A salvo whose blasts would take in a seat or the players' bong is
    // held until the target is clear of them.
    if with_tank && on_target && a.ammo > 0 && a.heli.since_salvo >= t.air_heli_salvo_seconds && view.clear(mark.at, salvo_reach(t), t) {
        a.ammo -= 1;
        a.heli.since_salvo = 0.0;
        acts.push(AirAct::Salvo { from: a.drawn(), at: mark.at });
    }
}

/// Whether its sortie is over: the end screen, no tank to escort, a wreck
/// to escort with nothing in view, or its ammunition spent and
/// `air_heli_linger_seconds` gone since the last salvo.
fn done(a: &Aircraft, view: &AirView, t: &Tuning) -> bool {
    !view.live
        || view.seat.is_none()
        || (view.seat_wrecked && view.marks.is_empty())
        || (a.ammo == 0 && a.heli.since_salvo >= t.air_heli_linger_seconds)
}

/// A step of the way home: out through where it came in and on past it,
/// along the line it turned onto when it left.
fn go_home(a: &mut Aircraft, dt: f32, t: &Tuning) {
    if a.stage != AirStage::Egress {
        a.heli.home = unit(a.entry - a.ground).unwrap_or(-a.heading);
        a.enter(AirStage::Egress);
    }
    let home = a.heli.home;
    fly(a, home * t.air_heli_speed, dt, t);
    point(a, home, dt, t);
}

/// Where it wants to be while its tank stands at `seat`, always
/// `air_heli_escort_px` from it: with no mark, on the side of the tank it
/// is already on; with one, toward it - straight toward it while that
/// leaves `air_heli_standoff_px` to the mark, and otherwise as far round
/// the tank's flank, on the side it is already on, as keeps that distance
/// (right behind the tank when nowhere does). `marks` are the view's,
/// nearest first: the station is the first one's, moved out of the standoff
/// of each of the others (`clear`). Held inside the field (`inside`).
fn station(a: &Aircraft, seat: Position, marks: &[AirMark], field: (f32, f32), t: &Tuning) -> Position {
    let out = t.air_heli_escort_px;
    let from_seat = a.ground - seat;
    let side = unit(from_seat).unwrap_or(-a.heading);
    let toward = marks.first().and_then(|mark| unit(mark.at - seat).map(|dir| (dir, (mark.at - seat).length())));
    let at = match toward {
        Some((dir, far)) if out > 0.0 => {
            // The law of cosines: the angle off the line to the mark at
            // which a point `out` from the tank is the standoff from it.
            let cos = ((out * out + far * far - t.air_heli_standoff_px * t.air_heli_standoff_px) / (2.0 * out * far)).clamp(-1.0, 1.0);
            let across = Vec2::new(-dir.y, dir.x);
            let flank = if from_seat.dot(across) < 0.0 { -1.0 } else { 1.0 };
            seat + (dir * cos + across * (flank * (1.0 - cos * cos).max(0.0).sqrt())) * out
        }
        _ => seat + side * out,
    };
    inside(clear(at, marks.get(1..).unwrap_or(&[]), t.air_heli_standoff_px), field, a.height)
}

/// `at` moved out of the reach `standoff` of each of `marks` it is inside
/// of, straight away from that one, in their order: a station taken for one
/// target is not left hanging over another's tower.
fn clear(at: Position, marks: &[AirMark], standoff: f32) -> Position {
    marks.iter().fold(at, |at, mark| {
        let away = at - mark.at;
        match unit(away) {
            Some(dir) if away.length() < standoff => mark.at + dir * standoff,
            _ => at,
        }
    })
}

/// `at` held a cell inside a field of size `field`, and far enough under
/// its top edge that a helicopter drawn `height` over it is inside too.
fn inside(at: Position, field: (f32, f32), height: f32) -> Position {
    let cell = crate::OBSTACLE_GRID_SIZE;
    let top = cell + height;
    Position::new(at.x.clamp(cell, (field.0 - cell).max(cell)), at.y.clamp(top, (field.1 - cell).max(top)))
}

/// The velocity that closes the gap `to` to its station: full speed while
/// it is far, then the fastest it can still brake from at
/// `air_heli_accel`, and never past the station in one step.
fn closing(to: Vec2, dt: f32, t: &Tuning) -> Vec2 {
    let gap = to.length();
    match unit(to) {
        Some(dir) => dir * t.air_heli_speed.min((2.0 * t.air_heli_accel * gap).sqrt()).min(gap / dt),
        None => Vec2::zero(),
    }
}

/// Move one step: its velocity toward `want` held to `air_heli_speed`,
/// changing by no more than `air_heli_accel` allows, then its ground point
/// along it.
fn fly(a: &mut Aircraft, want: Vec2, dt: f32, t: &Tuning) {
    let fast = want.length();
    let want = if fast > t.air_heli_speed { want * (t.air_heli_speed / fast) } else { want };
    let change = want - a.velocity;
    let most = t.air_heli_accel * dt;
    let asked = change.length();
    a.velocity = if asked > most { a.velocity + change * (most / asked) } else { want };
    a.ground = a.ground + a.velocity * dt;
}

/// Turn its nose toward `toward` by no more than `air_heli_turn_rate_deg`
/// allows in a step; `bank` is the share of that it used.
fn point(a: &mut Aircraft, toward: Vec2, dt: f32, t: &Tuning) {
    let most = t.air_heli_turn_rate_deg.to_radians() * dt;
    let (heading, turned) = turn_toward(a.heading, toward, most);
    a.heading = heading;
    a.bank = if most > 0.0 { (turned / most).clamp(-1.0, 1.0) } else { 0.0 };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::air_support::{AirFriend, MarkKey, Sortie};

    const DT: f32 = 1.0 / 60.0;
    const FIELD: (f32, f32) = (1088.0, 544.0);

    fn view(seat: Position, velocity: Vec2, marks: &[AirMark]) -> AirView<'_> {
        AirView { seat: Some(seat), seat_velocity: velocity, seat_wrecked: false, marks, friends: &[], field: FIELD, live: true }
    }

    /// A helicopter coming in over the left edge for a seat at `seat`.
    fn heli(seat: Position, t: &Tuning) -> Aircraft {
        Aircraft::arrive(1, 0, Sortie::Heli, Position::new(-t.air_edge_margin_px, seat.y), seat, t)
    }

    fn tank(at: Position) -> AirMark {
        AirMark { key: MarkKey::Tank(1), at, velocity: Vec2::zero() }
    }

    /// `ticks` steps beside a seat standing at `seat`: every act.
    fn hold(a: &mut Aircraft, seat: Position, marks: &[AirMark], ticks: u32, t: &Tuning) -> Vec<AirAct> {
        let mut acts = Vec::new();
        for _ in 0..ticks {
            a.step(&view(seat, Vec2::zero(), marks), DT, t, &mut acts);
        }
        acts
    }

    /// A whole sortie on a script: the seat stands two seconds, drives east
    /// three and north one, and stops with two enemies in view. The
    /// aircraft before each step, after it, and what it did in it, until it
    /// is gone.
    fn sortie(t: &Tuning) -> Vec<(Aircraft, Aircraft, Vec<AirAct>)> {
        let mut seat = Position::new(200.0, 300.0);
        let mut a = heli(seat, t);
        let mut out = Vec::new();
        for tick in 0..3600 {
            let now = tick as f32 * DT;
            let velocity = match now {
                n if n < 2.0 => Vec2::zero(),
                n if n < 5.0 => Vec2::new(210.0, 0.0),
                n if n < 6.0 => Vec2::new(0.0, -210.0),
                _ => Vec2::zero(),
            };
            let marks = [tank(seat + Vec2::new(-250.0, 150.0)), tank(seat + Vec2::new(300.0, 200.0))];
            let seen = if now < 6.0 { &marks[..0] } else { &marks[..] };
            let before = a;
            let mut acts = Vec::new();
            a.step(&view(seat, velocity, seen), DT, t, &mut acts);
            seat = seat + velocity * DT;
            out.push((before, a, acts));
            if a.gone(FIELD, t) {
                return out;
            }
        }
        panic!("it never went home: {a:?}");
    }

    #[test]
    fn it_keeps_to_its_speed_its_acceleration_and_its_turn_rate() {
        let t = Tuning::default();
        let turn = t.air_heli_turn_rate_deg.to_radians() * DT;
        for (before, after, _) in sortie(&t) {
            assert!(after.velocity.length() <= t.air_heli_speed + 1e-2, "too fast: {after:?}");
            let change = (after.velocity - before.velocity).length();
            assert!(change <= t.air_heli_accel * DT + 1e-2, "swerved {change} px/s in a step: {after:?}");
            assert!((after.heading.length() - 1.0).abs() < 1e-4, "its heading is no unit: {after:?}");
            let cross = before.heading.x * after.heading.y - before.heading.y * after.heading.x;
            let turned = crate::math::atan2(cross, before.heading.dot(after.heading));
            assert!(turned.abs() <= turn + 1e-3, "its nose turned {turned} rad in a step: {after:?}");
            assert!((-1.0..=1.0).contains(&after.bank));
            assert!((after.ground - (before.ground + after.velocity * DT)).length() < 1e-3, "it moves as its velocity says");
        }
    }

    /// A salvo at a target with a seat or the players' bong inside its
    /// reach is held, and spends nothing; once the friend is clear of it
    /// the salvo goes.
    #[test]
    fn it_holds_a_salvo_that_would_reach_a_friend() {
        let t = Tuning::default();
        let seat = Position::new(200.0, 300.0);
        let target = seat + Vec2::new(300.0, 0.0);
        let marks = [tank(target)];
        let mut a = heli(seat, &t);
        let near = [AirFriend { at: target + Vec2::new(0.0, salvo_reach(&t)), reach: 16.0 }];
        let mut fired = 0;
        for _ in 0..(20.0 / DT) as u32 {
            let mut acts = Vec::new();
            a.step(&AirView { friends: &near, ..view(seat, Vec2::zero(), &marks) }, DT, &t, &mut acts);
            fired += acts.len();
        }
        assert_eq!(fired, 0, "no salvo with a friend beside the target");
        assert_eq!(a.ammo as i32, t.air_heli_salvos, "and none spent");
        let clear = [AirFriend { at: target + Vec2::new(0.0, salvo_reach(&t) + 16.0 + t.air_hold_margin_px + 2.0), reach: 16.0 }];
        let mut acts = Vec::new();
        for _ in 0..(2.0 / DT) as u32 {
            a.step(&AirView { friends: &clear, ..view(seat, Vec2::zero(), &marks) }, DT, &t, &mut acts);
        }
        assert!(!acts.is_empty(), "it fires once the friend is clear");
    }

    #[test]
    fn a_salvo_costs_one_and_it_goes_home_when_they_are_spent() {
        let t = Tuning::default();
        let flight = sortie(&t);
        let mut fired = 0;
        let mut last = None;
        for (tick, (before, after, acts)) in flight.iter().enumerate() {
            assert!(acts.len() <= 1, "one salvo a step at most");
            assert_eq!(before.ammo as usize - after.ammo as usize, acts.len(), "ammunition goes down by one a salvo");
            for act in acts {
                assert!(matches!(act, AirAct::Salvo { from, .. } if *from == after.drawn()), "{act:?}");
                assert_eq!(after.stage, AirStage::Attack);
                if let Some(last) = last {
                    assert!((tick - last) as f32 * DT >= t.air_heli_salvo_seconds - 1e-3, "salvos {last} and {tick} too close");
                }
                last = Some(tick);
                fired += 1;
            }
        }
        assert_eq!(fired, t.air_heli_salvos, "every salvo it came with");
        let (_, end, _) = flight.last().expect("it flew");
        assert_eq!((end.ammo, end.stage), (0, AirStage::Egress));
        let left = flight.iter().position(|(_, after, _)| after.stage == AirStage::Egress).expect("it left");
        let waited = (left - last.expect("it fired")) as f32 * DT;
        assert!((waited - t.air_heli_linger_seconds).abs() <= 2.0 * DT, "it left {waited} s after its last salvo");
    }

    #[test]
    fn the_same_inputs_give_the_same_bits() {
        let t = Tuning::default();
        let bits = |flight: Vec<(Aircraft, Aircraft, Vec<AirAct>)>| -> Vec<[u32; 9]> {
            flight
                .into_iter()
                .map(|(_, a, acts)| {
                    [a.ground.x, a.ground.y, a.heading.x, a.heading.y, a.velocity.x, a.velocity.y, a.bank, a.heli.since_salvo, acts.len() as f32].map(f32::to_bits)
                })
                .collect()
        };
        assert_eq!(bits(sortie(&t)), bits(sortie(&t)));
    }

    #[test]
    fn it_comes_to_its_seat_holds_station_and_stays() {
        let t = Tuning::default();
        let seat = Position::new(500.0, 300.0);
        let mut a = heli(seat, &t);
        assert_eq!(a.stage, AirStage::Inbound);
        // Ten minutes with nothing to shoot at.
        let acts = hold(&mut a, seat, &[], 36_000, &t);
        assert!(acts.is_empty(), "nothing in view, nothing fired");
        assert_eq!(a.stage, AirStage::Escort);
        assert_eq!(a.ammo as i32, t.air_heli_salvos);
        assert!(((a.ground - seat).length() - t.air_heli_escort_px).abs() < 1.0, "on station: {:?}", a.ground);
        assert!(a.velocity.length() < 1.0, "hovering: {:?}", a.velocity);
        assert!(a.ground.x < seat.x, "short of the tank on the side it came from");
    }

    #[test]
    fn it_keeps_up_with_a_tank_driving_flat_out() {
        let t = Tuning::default();
        let mut seat = Position::new(300.0, 300.0);
        let mut a = heli(seat, &t);
        hold(&mut a, seat, &[], 300, &t);
        let velocity = Vec2::new(t.tank_speed, 0.0);
        let mut acts = Vec::new();
        for tick in 0..600 {
            // A field as long as the drive.
            let long = AirView { field: (8000.0, FIELD.1), ..view(seat, velocity, &[]) };
            a.step(&long, DT, &t, &mut acts);
            // The world moves the tank after its aircraft (`Game::update`).
            seat = seat + velocity * DT;
            let away = (a.ground - seat).length();
            assert!(away <= t.air_heli_escort_px + t.air_heli_station_slack_px, "tick {tick}: {away} px from its tank");
        }
        assert!(((a.ground - seat).length() - t.air_heli_escort_px).abs() < 2.0, "back on station at speed: {:?} by {seat:?}", a.ground);
        assert!((a.velocity - velocity).length() < 2.0, "flying with the tank: {:?}", a.velocity);
        assert!(a.heading.x > 0.99, "nose along its flight: {:?}", a.heading);
    }

    #[test]
    fn it_takes_the_targets_side_of_its_tank_nose_on_it() {
        // Rockets enough to still be at it once it has settled.
        let t = Tuning { air_heli_salvos: 24, ..Tuning::default() };
        let seat = Position::new(500.0, 300.0);
        let mut a = heli(seat, &t);
        let marks = [tank(Position::new(500.0, 100.0)), tank(Position::new(800.0, 300.0))];
        // Far enough off for the whole station: straight toward it.
        let acts = hold(&mut a, seat, &marks[1..], 600, &t);
        assert_eq!(a.stage, AirStage::Attack);
        assert!((a.ground - Position::new(500.0 + t.air_heli_escort_px, 300.0)).length() < 1.0, "{:?}", a.ground);
        assert!(a.heading.x > 0.999, "nose on the target: {:?}", a.heading);
        assert!(!acts.is_empty());
        // A nearer one comes into view: every salvo is the nearest's, the
        // first mark's, and the station goes round to its side.
        let acts = hold(&mut a, seat, &marks, 600, &t);
        assert!(!acts.is_empty());
        assert!(acts.iter().all(|act| matches!(act, AirAct::Salvo { at, .. } if *at == marks[0].at)), "{acts:?}");
        assert!(a.ground.y < seat.y && a.ground.x > seat.x, "on the target's side, the flank it was on: {:?}", a.ground);
        assert!(a.heading.dot(unit(marks[0].at - a.ground).expect("apart")) > 0.999, "nose on the target: {:?}", a.heading);
    }

    #[test]
    fn its_station_is_its_escort_from_its_tank_and_its_standoff_from_the_target() {
        let t = Tuning::default();
        let seat = Position::new(500.0, 300.0);
        let (out, off) = (t.air_heli_escort_px, t.air_heli_standoff_px);
        for far in [30.0, 60.0, 120.0, 200.0, 260.0, 340.0] {
            let mark = Position::new(seat.x + far, seat.y);
            let mut a = heli(seat, &t);
            hold(&mut a, seat, &[tank(mark)], 600, &t);
            assert!(((a.ground - seat).length() - out).abs() < 1.0, "target {far} px off: {:?}", a.ground);
            let from_mark = (a.ground - mark).length();
            if far + out < off {
                assert!((from_mark - (far + out)).abs() < 1.0, "target {far} px off: right behind the tank, {from_mark} px from it");
            } else if far - out > off {
                assert!((from_mark - (far - out)).abs() < 1.0, "target {far} px off: straight toward it, {from_mark} px from it");
            } else {
                assert!((from_mark - off).abs() < 1.0, "target {far} px off: round the flank, {from_mark} px from it");
            }
        }
    }

    #[test]
    fn its_station_keeps_its_standoff_from_every_target_in_view() {
        let t = Tuning::default();
        let seat = Position::new(500.0, 300.0);
        // The nearest is due north; the station that takes its side, 120 px
        // north of the tank, is 40 px from a second one.
        let marks = [tank(Position::new(500.0, 0.0)), tank(Position::new(540.0, 180.0))];
        let mut a = heli(seat, &t);
        hold(&mut a, seat, &marks, 900, &t);
        for mark in &marks {
            let from_mark = (a.ground - mark.at).length();
            assert!(from_mark >= t.air_heli_standoff_px - 1.0, "{from_mark} px from the target at {:?}: {:?}", mark.at, a.ground);
        }
        assert!(a.ground.y < seat.y, "still on the nearest one's side: {:?}", a.ground);
    }

    #[test]
    fn it_fires_only_with_its_tank_and_nose_on_the_target() {
        let t = Tuning::default();
        let seat = Position::new(900.0, 300.0);
        let marks = [tank(Position::new(700.0, 300.0))];
        let cone = crate::math::cos(t.air_heli_aim_deg.to_radians());
        let mut a = heli(seat, &t);
        let mut fired = 0;
        for _ in 0..900 {
            let mut acts = Vec::new();
            a.step(&view(seat, Vec2::zero(), &marks), DT, &t, &mut acts);
            for _ in &acts {
                fired += 1;
                assert!((a.ground - seat).length() <= t.air_heli_escort_px + t.air_heli_station_slack_px, "fired on its way in: {:?}", a.ground);
                assert!(unit(marks[0].at - a.ground).is_none_or(|dir| a.heading.dot(dir) >= cone - 1e-4), "fired nose off the target: {:?}", a.heading);
            }
        }
        assert!(fired > 0);
        // On the end screen nothing is fired.
        let mut a = heli(seat, &t);
        hold(&mut a, seat, &marks, 120, &t);
        let mut acts = Vec::new();
        for _ in 0..600 {
            a.step(&AirView { live: false, ..view(seat, Vec2::zero(), &marks) }, DT, &t, &mut acts);
        }
        assert!(acts.is_empty());
    }

    #[test]
    fn it_goes_home_from_the_end_screen_a_lost_seat_and_a_wreck_with_nothing_in_view() {
        let t = Tuning::default();
        let seat = Position::new(500.0, 300.0);
        let marks = [tank(Position::new(700.0, 300.0))];
        let escorting = || {
            let mut a = heli(seat, &t);
            hold(&mut a, seat, &[], 300, &t);
            assert_eq!(a.stage, AirStage::Escort);
            a
        };
        let ended = AirView { live: false, ..view(seat, Vec2::zero(), &[]) };
        let lost = AirView { seat: None, ..view(seat, Vec2::zero(), &[]) };
        let wrecked = AirView { seat_wrecked: true, ..view(seat, Vec2::zero(), &[]) };
        for (name, v) in [("the end screen", ended), ("a lost seat", lost), ("a wreck", wrecked)] {
            let mut a = escorting();
            let mut acts = Vec::new();
            a.step(&v, DT, &t, &mut acts);
            assert_eq!(a.stage, AirStage::Egress, "{name}");
            let mut ticks = 0;
            while !a.gone(FIELD, &t) {
                a.step(&v, DT, &t, &mut acts);
                ticks += 1;
                assert!(ticks < 1200, "{name}: it never got off the field: {a:?}");
            }
            assert!(acts.is_empty(), "{name}");
            assert!(a.ground.x < 0.0, "{name}: out where it came in: {:?}", a.ground);
        }
        // A wreck with an enemy still in view is fought over.
        let mut a = escorting();
        let fighting = AirView { seat_wrecked: true, ..view(seat, Vec2::zero(), &marks) };
        let mut acts = Vec::new();
        for _ in 0..300 {
            a.step(&fighting, DT, &t, &mut acts);
        }
        assert_eq!(a.stage, AirStage::Attack);
        assert!(!acts.is_empty());
    }

    #[test]
    fn its_station_is_held_inside_the_field() {
        let t = Tuning::default();
        let cell = crate::OBSTACLE_GRID_SIZE;
        for seat in [Position::new(8.0, 8.0), Position::new(1080.0, 536.0), Position::new(8.0, 536.0)] {
            let mut a = heli(seat, &t);
            hold(&mut a, seat, &[], 900, &t);
            assert!(a.ground.x >= cell - 0.5 && a.ground.x <= FIELD.0 - cell + 0.5, "seat {seat:?}: {:?}", a.ground);
            assert!(a.ground.y >= cell + a.height - 0.5 && a.ground.y <= FIELD.1 - cell + 0.5, "seat {seat:?}: {:?}", a.ground);
        }
    }
}
