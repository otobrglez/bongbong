//! The jet's sortie (docs/air-support.md): `step` is all of it, a pure
//! function of the aircraft, the `AirView` and the fixed step.
//!
//! It flies like a plane: always at `air_jet_speed`, its heading changed
//! only through `turn_toward` by `air_jet_turn_deg_per_s` at most, so its
//! path is straights and arcs of its turn circle (`turn_radius`). What it
//! does with that:
//!
//! - **A target in view** (`AirView::marks`) and a pass left: it locks one
//!   (`JetState::lock`, looked up again every step, so a target that moves
//!   is followed) and flies at it, its nose led by the target's velocity
//!   (`aim_point`). Lined up and in range it is on its run
//!   (`AirStage::Attack`): the cannon's stripe walks up to the target, then
//!   two bombs go astride where the target is at that moment. That is a
//!   pass (`Aircraft::ammo`).
//! - **Passes left**: it flies straight on past the target (`Leg::Extend`,
//!   `AirStage::Loiter`), turns back - the way that keeps it nearer the
//!   field - and runs in again, on the same target while it lives, else the
//!   nearest. Another target already under its nose it runs in on at once.
//! - **A run it cannot make** - the target inside its turn circle, where
//!   turning at it would only take it round and round - is a go-around:
//!   straight on, then back. `air_jet_go_arounds` of them and it goes home.
//! - **Nothing in view**: it flies to its seat, circles it once
//!   (`circle`) and leaves. A target that shows meanwhile is attacked.
//! - **Out of passes, the round over, no seat, or `air_jet_sortie_seconds`
//!   up**: `AirStage::Egress`, straight on off the field.
//!
//! It fires only with its ground point over the field, and never on the
//! step it arrives.

use std::f32::consts::{FRAC_PI_2, TAU};

use super::{AirAct, AirMark, AirStage, AirView, Aircraft, MarkKey, turn_toward, unit};
use crate::math::{self, Vec2};
use crate::tuning::Tuning;
use crate::Position;

/// What the jet is flying.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Leg {
    /// At its target, or with none round its seat.
    #[default]
    Run,
    /// Straight on, making room to turn back at its target.
    Extend,
}

/// What the jet keeps between steps besides `Aircraft`'s own.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JetState {
    /// Seconds flown: 0 on the step it arrives.
    age: f32,
    leg: Leg,
    /// The target it is after, while that one is in view.
    lock: Option<MarkKey>,
    /// The target `Leg::Extend` is flying on from: it turns back for that
    /// one only once the leg is flown.
    behind: Option<MarkKey>,
    /// The way it has committed to turn round to a target astern of it:
    /// -1 left, 1 right, 0 the short way.
    turn: i8,
    /// Px of `Leg::Extend` still to fly.
    extend_left: f32,
    /// How far from its target this run's cannon stripe began (px); 0
    /// before it has.
    gun_from: f32,
    /// Cannon bursts fired this run.
    bursts: u8,
    /// The way it circles its seat: -1 left, 1 right, 0 not chosen.
    orbit: i8,
    /// How far its nose has come round on that circle (radians).
    sweep: f32,
    /// Whether it has flown its circle: it flies one a sortie.
    circled: bool,
    /// Runs it has broken off.
    go_arounds: u8,
}

/// Where the jet wants its nose for a step.
struct Course {
    /// The direction to turn to; `None` flies straight on.
    want: Option<Vec2>,
    /// Whether the turn counts towards its circle round the seat.
    circling: bool,
}

impl Course {
    fn straight() -> Course {
        Course { want: None, circling: false }
    }
}

/// How far along the ring's tangent, in orbit radii, the point the jet
/// steers at on its circle lies (`circle`). Half a radius keeps its way in
/// from outside close to the straight tangent onto the ring, and never
/// asks a tighter turn of it than the ring's own.
const ORBIT_LEAD: f32 = 0.5;

/// How many orbit radii from its seat the jet is on its circle, its turn
/// counting towards the round: a quarter radius outside the ring, where
/// its nose is already most of the way from the seat to the tangent.
const ORBIT_REACH: f32 = 1.25;

/// One fixed step of a jet (`Aircraft::step`).
pub(super) fn step(a: &mut Aircraft, view: &AirView, dt: f32, t: &Tuning, acts: &mut Vec<AirAct>) {
    let arriving = a.jet.age <= 0.0;
    a.jet.age += dt;
    let speed = t.air_jet_speed;
    let max_turn = t.air_jet_turn_deg_per_s.to_radians() * dt;
    let spent = a.ammo == 0 || a.jet.age >= t.air_jet_sortie_seconds;
    let seat = view.seat.filter(|_| view.live && !spent && a.stage != AirStage::Egress);
    let mark = seat.and_then(|_| acquire(a, view));
    let course = match seat {
        Some(seat) => course(a, view, seat, mark.as_ref(), speed * dt, t),
        None => {
            if a.stage != AirStage::Egress {
                a.enter(AirStage::Egress);
            }
            Course::straight()
        }
    };
    let (heading, turned) = match course.want {
        Some(want) => turn_toward(a.heading, want, max_turn),
        None => (a.heading, 0.0),
    };
    a.heading = heading;
    a.velocity = heading * speed;
    a.ground = a.ground + a.velocity * dt;
    let share = if max_turn > 0.0 { turned / max_turn } else { 0.0 };
    let roll = if t.air_jet_bank_seconds > 0.0 { dt / t.air_jet_bank_seconds } else { 2.0 };
    a.bank = (a.bank + (share - a.bank).clamp(-roll, roll)).clamp(-1.0, 1.0);
    if course.circling {
        a.jet.sweep += turned * a.jet.orbit as f32;
        if a.jet.sweep >= TAU * t.air_jet_orbit_turns {
            a.jet.circled = true;
            a.enter(AirStage::Egress);
        }
    }
    if let Some(mark) = mark
        && !arriving
        && a.stage != AirStage::Egress
        && a.jet.leg == Leg::Run
    {
        engage(a, &mark, view.field, t, acts);
    }
}

/// The target for this step: the one it is locked on while that is in
/// view, else the first in view (the nearest its seat), which it locks. A
/// new target is a new run: one under way is over, its stripe with it.
fn acquire(a: &mut Aircraft, view: &AirView) -> Option<AirMark> {
    let locked = a.jet.lock.and_then(|key| view.marks.iter().find(|m| m.key == key));
    let mark = locked.or(view.marks.first()).copied();
    let key = mark.map(|m| m.key);
    if key != a.jet.lock && a.stage == AirStage::Attack {
        a.jet.gun_from = 0.0;
        a.jet.bursts = 0;
        a.enter(AirStage::Loiter);
    }
    a.jet.lock = key;
    mark
}

/// The radius of the jet's tightest turn (px): its speed over its turn
/// rate.
pub(super) fn turn_radius(t: &Tuning) -> f32 {
    t.air_jet_speed / t.air_jet_turn_deg_per_s.to_radians().max(1e-3)
}

/// The unit vector to the right of `heading` on the y-down screen.
fn right_of(heading: Vec2) -> Vec2 {
    Vec2::new(-heading.y, heading.x)
}

/// How far round from `heading` the direction `to` lies (radians): positive
/// to the right, as `turn_toward` counts.
fn bearing(heading: Vec2, to: Vec2) -> f32 {
    math::atan2(heading.x * to.y - heading.y * to.x, heading.dot(to))
}

/// Whether `p` is over a field of size `field`.
fn over(p: Position, field: (f32, f32)) -> bool {
    p.x >= 0.0 && p.y >= 0.0 && p.x <= field.0 && p.y <= field.1
}

/// Where the jet at `ground` points its nose for `mark`: where the target
/// will be when the bombs go, `air_jet_aim_lead` of its velocity over the
/// time left to their release, held on the field. Inside the release it is
/// the target itself.
fn aim_point(ground: Position, mark: &AirMark, field: (f32, f32), t: &Tuning) -> Position {
    let d = (mark.at - ground).length();
    let seconds = (d - t.air_jet_bomb_lead_px).max(0.0) / t.air_jet_speed.max(1.0);
    let at = mark.at + mark.velocity * (t.air_jet_aim_lead * seconds);
    Position::new(at.x.clamp(0.0, field.0.max(0.0)), at.y.clamp(0.0, field.1.max(0.0)))
}

/// Whether a target `at` from the jet (a vector) is where its bombs can
/// go: within `air_jet_bomb_cone_deg` of its nose, or right under it.
fn on_target(heading: Vec2, at: Vec2, t: &Tuning) -> bool {
    at.length() <= t.air_jet_bomb_spacing_px * 0.5 || bearing(heading, at).abs() <= t.air_jet_bomb_cone_deg.to_radians()
}

/// Which way round keeps the jet nearer the field: the side whose turn
/// circle's centre is nearer the field's middle, `tie` when neither is.
fn field_side(ground: Position, right: Vec2, radius: f32, field: (f32, f32), tie: i8) -> i8 {
    let middle = Position::new(field.0 * 0.5, field.1 * 0.5);
    let to_right = (ground + right * radius - middle).length();
    let to_left = (ground - right * radius - middle).length();
    if to_left < to_right {
        -1
    } else if to_right < to_left {
        1
    } else {
        tie
    }
}

/// The course with a seat on the field: at `mark`, or with none round the
/// seat. `flown` is the px this step covers.
fn course(a: &mut Aircraft, view: &AirView, seat: Position, mark: Option<&AirMark>, flown: f32, t: &Tuning) -> Course {
    let Some(mark) = mark else { return circle(a, seat, view.field, t) };
    a.jet.orbit = 0;
    a.jet.sweep = 0.0;
    let radius = turn_radius(t);
    let aim = aim_point(a.ground, mark, view.field, t);
    let to = aim - a.ground;
    let err = bearing(a.heading, to);
    let right = right_of(a.heading);
    // How far the jet would fly straight at the aim point after turning
    // onto it to a side: the tangent from the point to that turn circle.
    // A point inside the circle has none - turning at it only takes the
    // jet round it.
    let run_in = |side: f32| {
        let from_centre = (aim - (a.ground + right * (radius * side))).length();
        (from_centre >= radius).then(|| ((from_centre + radius) * (from_centre - radius)).sqrt())
    };
    let lined = err.abs() <= t.air_jet_gun_cone_deg.to_radians();
    if a.jet.leg == Leg::Extend {
        a.jet.extend_left -= flown;
        // The leg is cut short off the field, once either way back leaves
        // the cannon a run, and for another target already under its nose.
        let room = |side: f32| run_in(side).is_some_and(|px| px >= t.air_jet_gun_ahead_px);
        let off_field = !over(a.ground, view.field) && room(1.0) && room(-1.0);
        let another = lined && a.jet.behind != Some(mark.key);
        if a.jet.extend_left > 0.0 && !off_field && !another {
            return Course::straight();
        }
        a.jet.leg = Leg::Run;
        a.jet.behind = None;
    }
    let short = if err < 0.0 { -1 } else { 1 };
    if a.jet.turn == 0 && err.abs() > t.air_jet_astern_deg.to_radians() {
        a.jet.turn = field_side(a.ground, right, radius, view.field, short);
    } else if a.jet.turn != 0 && err.abs() <= FRAC_PI_2 {
        // Forward of the beam the short way round is the way it is turning.
        a.jet.turn = 0;
    }
    let side = if a.jet.turn != 0 { a.jet.turn } else { short } as f32;
    if !lined && run_in(side).is_none() {
        go_around(a, t);
        return Course::straight();
    }
    Course { want: Some(if a.jet.turn != 0 { right * side } else { to }), circling: false }
}

/// The course with nothing to attack: to the seat and once round it, on a
/// ring `air_jet_orbit_px` from where the seat stands now, then home. It
/// steers at the point `ORBIT_LEAD` radii along the tangent from the ring's
/// nearest point, which brings it onto the ring from either side of it and
/// holds it there; its turn counts towards the round from `ORBIT_REACH`.
fn circle(a: &mut Aircraft, seat: Position, field: (f32, f32), t: &Tuning) -> Course {
    a.jet.leg = Leg::Run;
    a.jet.behind = None;
    a.jet.turn = 0;
    a.jet.gun_from = 0.0;
    a.jet.bursts = 0;
    if a.jet.circled {
        a.enter(AirStage::Egress);
        return Course::straight();
    }
    let ring = t.air_jet_orbit_px;
    let to = seat - a.ground;
    let d = to.length();
    let Some(inward) = unit(to) else { return Course::straight() };
    let near = d <= ring * ORBIT_REACH;
    if a.jet.orbit == 0 {
        let seat_right = a.heading.x * to.y - a.heading.y * to.x >= 0.0;
        let natural = if seat_right { 1 } else { -1 };
        // By the ring it keeps the seat on the side it already is; coming
        // from afar it joins on the side nearer the field's middle.
        a.jet.orbit = if near { natural } else { field_side(seat, right_of(inward) * -1.0, ring, field, natural) };
    }
    if a.stage == AirStage::Attack || (near && a.stage == AirStage::Inbound) {
        a.enter(AirStage::Loiter);
    }
    let tangent = Vec2::new(inward.y, -inward.x) * a.jet.orbit as f32;
    Course { want: Some(tangent * (ring * ORBIT_LEAD) + inward * (d - ring)), circling: near }
}

/// Break the run off: straight on for `air_jet_extend_px` and back, or
/// home once `air_jet_go_arounds` are used up.
fn go_around(a: &mut Aircraft, t: &Tuning) {
    a.jet.go_arounds = a.jet.go_arounds.saturating_add(1);
    if a.jet.go_arounds as i32 > t.air_jet_go_arounds {
        a.enter(AirStage::Egress);
    } else {
        extend(a, t.air_jet_extend_px);
    }
}

/// Fly straight on for `px`, the run on its target over.
fn extend(a: &mut Aircraft, px: f32) {
    a.jet.leg = Leg::Extend;
    a.jet.behind = a.jet.lock;
    a.jet.extend_left = px;
    a.jet.turn = 0;
    a.jet.gun_from = 0.0;
    a.jet.bursts = 0;
    if a.stage != AirStage::Loiter {
        a.enter(AirStage::Loiter);
    }
}

/// What the jet does about `mark` from where the step left it, over the
/// field only. Lined up inside the cannon's range it is on its run
/// (`AirStage::Attack`): up to `air_jet_gun_bursts` bursts, one a step at
/// most, each `air_jet_gun_ahead_px` ahead of it on the field, spaced so
/// the last lands on the target - a stripe `air_jet_gun_len_px` long, or as
/// long as the run it had. `air_jet_bomb_lead_px` short of the target the
/// two bombs go, `air_jet_bomb_spacing_px` apart along its heading astride
/// where the target is now: a pass, and it flies on - home with none left.
fn engage(a: &mut Aircraft, mark: &AirMark, field: (f32, f32), t: &Tuning, acts: &mut Vec<AirAct>) {
    if !over(a.ground, field) {
        return;
    }
    let at = mark.at - a.ground;
    let d = at.length();
    let ahead = t.air_jet_gun_ahead_px;
    let range = ahead + t.air_jet_gun_len_px;
    let lined = bearing(a.heading, aim_point(a.ground, mark, field, t) - a.ground).abs() <= t.air_jet_gun_cone_deg.to_radians();
    let release = d <= t.air_jet_bomb_lead_px && on_target(a.heading, at, t);
    if !release && !(lined && d <= range) {
        return;
    }
    if a.stage != AirStage::Attack {
        a.enter(AirStage::Attack);
    }
    let bursts = t.air_jet_gun_bursts.clamp(1, 255) as u8;
    // A burst landing more than its own reach past the target is held.
    if lined && a.jet.bursts < bursts && d <= range && d >= ahead - t.air_jet_gun_radius_px {
        if a.jet.gun_from <= 0.0 {
            a.jet.gun_from = d;
        }
        let spacing = if bursts > 1 { (a.jet.gun_from - ahead).max(0.0) / (bursts - 1) as f32 } else { 0.0 };
        let lands = a.ground + a.heading * ahead;
        if d <= ahead + (bursts - 1 - a.jet.bursts) as f32 * spacing && over(lands, field) {
            acts.push(AirAct::Strike { at: lands, dir: a.heading, bomb: false });
            a.jet.bursts += 1;
        }
    }
    if release {
        let half = a.heading * (t.air_jet_bomb_spacing_px * 0.5);
        acts.push(AirAct::Strike { at: mark.at - half, dir: a.heading, bomb: true });
        acts.push(AirAct::Strike { at: mark.at + half, dir: a.heading, bomb: true });
        a.ammo = a.ammo.saturating_sub(1);
        if a.ammo == 0 {
            a.enter(AirStage::Egress);
        } else {
            extend(a, d + t.air_jet_extend_px);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::air_support::{Sortie, entry_point, jet_edge};

    const DT: f32 = 1.0 / 60.0;
    const FIELD: (f32, f32) = (1088.0, 544.0);
    const SEAT: Position = Position::new(96.0, 192.0);
    const MIDDLE: Position = Position::new(544.0, 272.0);

    /// A jet answering a call for `toward`, in from the left edge in line
    /// with it.
    fn jet(toward: Position, t: &Tuning) -> Aircraft {
        Aircraft::arrive(1, 0, Sortie::Jet, Position::new(-t.air_edge_margin_px, toward.y), toward, t)
    }

    /// A jet answering a call for `toward` the way the world brings one
    /// in: from the far side of it (`jet_edge`).
    fn called(toward: Position, t: &Tuning) -> Aircraft {
        let entry = entry_point(jet_edge(1, toward, FIELD), toward, FIELD, t.air_edge_margin_px);
        Aircraft::arrive(1, 0, Sortie::Jet, entry, toward, t)
    }

    fn tank(slot: usize, at: Position, velocity: Vec2) -> AirMark {
        AirMark { key: MarkKey::Tank(slot), at, velocity }
    }

    fn parked(at: Position) -> Vec<AirMark> {
        vec![tank(1, at, Vec2::zero())]
    }

    fn view(seat: Position, marks: &[AirMark]) -> AirView<'_> {
        AirView { seat: Some(seat), seat_velocity: Vec2::zero(), seat_wrecked: false, marks, field: FIELD, live: true }
    }

    fn full_load(t: &Tuning) -> u8 {
        Sortie::Jet.ammo(t)
    }

    /// One step of a sortie.
    struct Tick {
        before: Aircraft,
        after: Aircraft,
        acts: Vec<AirAct>,
    }

    impl Tick {
        /// How far its nose came round in the step (radians, right
        /// positive).
        fn turned(&self) -> f32 {
            bearing(self.before.heading, self.after.heading)
        }

        /// Where its cannon bursts (`bombs` false) or bombs landed.
        fn strikes(&self, bombs: bool) -> Vec<Position> {
            self.acts
                .iter()
                .filter_map(|act| match *act {
                    AirAct::Strike { at, bomb, .. } if bomb == bombs => Some(at),
                    _ => None,
                })
                .collect()
        }
    }

    /// Fly `a` until it is gone or `seconds` are up, `marks` saying what is
    /// in view at each step from the step's number and the jet as it is.
    fn fly(a: &mut Aircraft, seat: Position, seconds: f32, t: &Tuning, mut marks: impl FnMut(u32, &Aircraft) -> Vec<AirMark>) -> Vec<Tick> {
        let mut out = Vec::new();
        for i in 0..(seconds / DT) as u32 {
            let seen = marks(i, a);
            let before = *a;
            let mut acts = Vec::new();
            a.step(&view(seat, &seen), DT, t, &mut acts);
            out.push(Tick { before, after: *a, acts });
            if a.gone(FIELD, t) {
                break;
            }
        }
        out
    }

    fn strikes(ticks: &[Tick], bombs: bool) -> Vec<Position> {
        ticks.iter().flat_map(|tick| tick.strikes(bombs)).collect()
    }

    /// How far round `seat` the jet flew over `ticks` (radians, signed),
    /// counting the steps that began within `reach` of it.
    fn round(ticks: &[Tick], seat: Position, reach: f32) -> f32 {
        ticks
            .iter()
            .filter(|tick| (tick.before.ground - seat).length() <= reach)
            .map(|tick| bearing(tick.before.ground - seat, tick.after.ground - seat))
            .sum()
    }

    /// The middle of a pair of bombs.
    fn between(pair: &[Position]) -> Position {
        (pair[0] + pair[1]) * 0.5
    }

    /// What every step of every sortie keeps: the speed, a unit heading, a
    /// turn inside the rate, a bank inside its range rolling no faster
    /// than its row, ammunition that only goes down, and a straight line
    /// once it is on its way out.
    fn flies_like_a_plane(name: &str, ticks: &[Tick], t: &Tuning) {
        let max_turn = t.air_jet_turn_deg_per_s.to_radians() * DT;
        for (i, tick) in ticks.iter().enumerate() {
            let Tick { before, after, .. } = tick;
            assert!((after.heading.length() - 1.0).abs() < 1e-4, "{name} {i}: a heading {} long", after.heading.length());
            assert!((after.velocity - after.heading * t.air_jet_speed).length() < 1e-2, "{name} {i}: its velocity is its heading at its speed");
            assert!(((after.ground - before.ground).length() - t.air_jet_speed * DT).abs() < 1e-2, "{name} {i}: it flew its speed");
            assert!(tick.turned().abs() <= max_turn + 1e-4, "{name} {i}: turned {} of {max_turn}", tick.turned());
            assert!((-1.0..=1.0).contains(&after.bank), "{name} {i}: a bank of {}", after.bank);
            assert!((after.bank - before.bank).abs() <= DT / t.air_jet_bank_seconds + 1e-5, "{name} {i}: it rolled faster than it can");
            assert!(after.ammo <= before.ammo, "{name} {i}: ammunition came back");
            if before.stage == AirStage::Egress {
                assert_eq!(after.stage, AirStage::Egress, "{name} {i}: on its way out it stays on its way out");
                assert_eq!(after.heading, before.heading, "{name} {i}: and flies straight");
                assert!(tick.acts.is_empty(), "{name} {i}: and fires nothing");
            }
        }
    }

    /// A sortie to fly: its name, where the seat stands, and what is in
    /// view at each step.
    type Script = (&'static str, Position, Box<dyn FnMut(u32, &Aircraft) -> Vec<AirMark>>);

    fn scripts(t: &Tuning) -> Vec<Script> {
        let radius = turn_radius(t);
        let speed = t.air_jet_speed;
        vec![
            ("nothing in view", SEAT, Box::new(|_, _| Vec::new())),
            ("a parked tank", SEAT, Box::new(|_, _| parked(Position::new(288.0, 192.0)))),
            ("a parked tank in the corner", Position::new(990.0, 480.0), Box::new(|_, _| parked(Position::new(1040.0, 500.0)))),
            (
                "a tank driving up and down at a tank's best",
                SEAT,
                Box::new(|i, _| {
                    let down = (i / 90) % 2 == 0;
                    let driven = (i % 90) as f32 * DT * 210.0;
                    let y = if down { 100.0 + driven } else { 415.0 - driven };
                    vec![tank(1, Position::new(320.0, y), Vec2::new(0.0, if down { 210.0 } else { -210.0 }))]
                }),
            ),
            (
                "a tank in view from the fourth second to the eighth",
                SEAT,
                Box::new(|i, _| if (240..480).contains(&i) { parked(Position::new(300.0, 300.0)) } else { Vec::new() }),
            ),
            ("a tank always at the centre of its turn", MIDDLE, Box::new(move |_, a| parked(a.ground + right_of(a.heading) * radius))),
            ("a tank always abeam of it", MIDDLE, Box::new(move |_, a| parked(a.ground - right_of(a.heading) * (radius * 2.5)))),
            ("a tank keeping ahead of it at its own speed", MIDDLE, Box::new(move |_, a| vec![tank(1, a.ground + a.heading * 400.0, a.heading * speed)])),
            (
                "a tank jumping across the field every second",
                MIDDLE,
                Box::new(|i, _| parked(if (i / 60) % 2 == 0 { Position::new(300.0, 150.0) } else { Position::new(800.0, 400.0) })),
            ),
            (
                "two tanks changing places in the list",
                SEAT,
                Box::new(|i, _| {
                    let (one, two) = (tank(1, Position::new(288.0, 192.0), Vec2::zero()), tank(2, Position::new(200.0, 400.0), Vec2::zero()));
                    if (i / 30) % 2 == 0 { vec![one, two] } else { vec![two, one] }
                }),
            ),
        ]
    }

    #[test]
    fn it_flies_like_a_plane_whatever_it_is_shown() {
        let t = Tuning::default();
        for (name, seat, marks) in scripts(&t) {
            let mut a = jet(seat, &t);
            let ticks = fly(&mut a, seat, 90.0, &t, marks);
            assert!(ticks.len() > 60, "{name}: it flew");
            flies_like_a_plane(name, &ticks, &t);
        }
    }

    /// Whatever it is shown it is on its way out by its last second, and
    /// gone.
    #[test]
    fn it_always_leaves() {
        let t = Tuning::default();
        for (name, seat, marks) in scripts(&t) {
            let mut a = jet(seat, &t);
            let ticks = fly(&mut a, seat, 90.0, &t, marks);
            assert!(a.gone(FIELD, &t), "{name}: still here after {} steps: {a:?}", ticks.len());
            let home = ticks.iter().position(|tick| tick.after.stage == AirStage::Egress).expect("it turned for home") as f32 * DT;
            assert!(home <= t.air_jet_sortie_seconds + DT, "{name}: on its way out only after {home} s");
        }
    }

    #[test]
    fn the_same_sortie_is_the_same_bits() {
        let t = Tuning::default();
        let run = |n: usize| {
            let (_, seat, marks) = scripts(&t).swap_remove(n);
            let mut a = jet(seat, &t);
            fly(&mut a, seat, 90.0, &t, marks).into_iter().map(|tick| (tick.after, tick.acts)).collect::<Vec<_>>()
        };
        for n in 0..scripts(&t).len() {
            assert_eq!(run(n), run(n), "script {n}");
        }
    }

    /// The bank is the share of the turn rate in use, rolled into: in a
    /// held turn it has the turn's sign and comes up to the share, and it
    /// is level again on the way out.
    #[test]
    fn it_banks_the_way_it_turns() {
        let t = Tuning::default();
        let mut a = jet(SEAT, &t);
        let ticks = fly(&mut a, SEAT, 60.0, &t, |_, _| Vec::new());
        let max_turn = t.air_jet_turn_deg_per_s.to_radians() * DT;
        let held = (t.air_jet_bank_seconds * 2.0 / DT) as usize + 2;
        let mut checked = 0;
        for i in held..ticks.len() {
            let share = ticks[i].turned() / max_turn;
            // The same turn, to a hundredth of the rate, for twice the roll.
            if share.abs() > 0.2 && ticks[i - held..i].iter().all(|tick| (tick.turned() / max_turn - share).abs() < 0.01) {
                let bank = ticks[i].after.bank;
                assert_eq!(bank.signum(), share.signum(), "step {i}: banked {bank} in a turn of {share}");
                assert!((bank - share).abs() < 0.02, "step {i}: banked {bank} in a turn of {share}");
                checked += 1;
            }
        }
        assert!(checked > 60, "its circle is a held turn: {checked}");
        assert_eq!(ticks.last().expect("it flew").after.bank, 0.0, "level on its way out");
    }

    /// The run: the cannon's stripe walks up to the target and ends on it,
    /// every burst the gun's reach ahead of the jet on its line, then the
    /// two bombs astride the target; that is one pass.
    #[test]
    fn the_stripe_walks_up_to_the_target_and_the_bombs_go_astride_it() {
        let t = Tuning::default();
        let target = Position::new(700.0, 300.0);
        let mut a = jet(target, &t);
        let ticks = fly(&mut a, SEAT, 60.0, &t, |_, _| parked(target));
        let pass: Vec<&Tick> = ticks.iter().take_while(|tick| tick.before.ammo == full_load(&t)).collect();
        let mut bursts = Vec::new();
        for tick in &pass {
            for at in tick.strikes(false) {
                assert!((at - (tick.after.ground + tick.after.heading * t.air_jet_gun_ahead_px)).length() < 1e-3, "ahead of it on its line");
                bursts.push(at);
            }
        }
        assert_eq!(bursts.len(), t.air_jet_gun_bursts as usize);
        let step = t.air_jet_speed * DT;
        let short: Vec<f32> = bursts.iter().map(|at| (target - *at).length()).collect();
        assert!(short.windows(2).all(|w| w[1] < w[0]), "each nearer the target: {short:?}");
        assert!((short[0] - t.air_jet_gun_len_px).abs() < step, "the stripe is its length: {short:?}");
        assert!(short[short.len() - 1] < step, "and ends on the target: {short:?}");
        let last = pass.last().expect("a pass");
        let dropped = last.strikes(true);
        assert_eq!(dropped.len(), 2, "two bombs end the pass");
        assert!((between(&dropped) - target).length() < 1e-3, "astride the target");
        assert!(((dropped[1] - dropped[0]) - last.after.heading * t.air_jet_bomb_spacing_px).length() < 1e-3, "along its heading");
        let from = (target - last.after.ground).length();
        assert!(from <= t.air_jet_bomb_lead_px && from > t.air_jet_bomb_lead_px - step, "let go at the lead: {from}");
        assert_eq!(last.after.ammo, full_load(&t) - 1);
        assert!(pass.iter().any(|tick| tick.after.stage == AirStage::Attack));
    }

    /// With passes left it flies on, turns back and runs in on the same
    /// target; out of them it flies straight out.
    #[test]
    fn it_comes_back_while_it_has_passes_and_then_goes_home() {
        let t = Tuning::default();
        let target = Position::new(600.0, 300.0);
        let mut a = jet(target, &t);
        let ticks = fly(&mut a, SEAT, 60.0, &t, |_, _| vec![AirMark { key: MarkKey::Tower((18, 9)), at: target, velocity: Vec2::zero() }]);
        flies_like_a_plane("a tower", &ticks, &t);
        let dropped = strikes(&ticks, true);
        assert_eq!(dropped.len(), 2 * t.air_jet_passes as usize, "two bombs a pass");
        for pair in dropped.chunks(2) {
            assert!((between(pair) - target).length() < 1e-3);
        }
        let mut stages: Vec<AirStage> = ticks.iter().map(|tick| tick.after.stage).collect();
        stages.dedup();
        assert_eq!(stages, vec![AirStage::Inbound, AirStage::Attack, AirStage::Loiter, AirStage::Attack, AirStage::Egress]);
        assert_eq!(strikes(&ticks, false).len(), 2 * t.air_jet_gun_bursts as usize, "the way back has room for a whole stripe");
        let home = ticks.iter().position(|tick| tick.after.stage == AirStage::Egress).expect("it went home");
        assert_eq!(ticks[home].after.ammo, 0);
        assert!(a.gone(FIELD, &t));
    }

    /// The target is looked up every step: one that has jumped, or is
    /// driving, is flown at where it is and bombed where it is.
    #[test]
    fn a_target_that_moves_is_followed() {
        let t = Tuning::default();
        let (first, then) = (Position::new(700.0, 300.0), Position::new(820.0, 180.0));
        let mut a = jet(first, &t);
        let ticks = fly(&mut a, SEAT, 60.0, &t, |i, _| parked(if i < 60 { first } else { then }));
        let dropped = strikes(&ticks, true);
        assert_eq!(dropped.len(), 2 * t.air_jet_passes as usize);
        for pair in dropped.chunks(2) {
            assert!((between(pair) - then).length() < 1e-3, "bombed where it is, not where it was: {pair:?}");
        }
        // Driving across its nose it is led, and bombed where it is when
        // the bombs go.
        let at = |i: u32| Position::new(700.0, 80.0 + i as f32 * DT * 120.0);
        let mut a = jet(at(0), &t);
        let ticks = fly(&mut a, SEAT, 60.0, &t, |i, _| vec![tank(1, at(i), Vec2::new(0.0, 120.0))]);
        flies_like_a_plane("a tank driving across", &ticks, &t);
        let mut pairs = 0;
        for (i, tick) in ticks.iter().enumerate() {
            let dropped = tick.strikes(true);
            if !dropped.is_empty() {
                pairs += 1;
                assert!((between(&dropped) - at(i as u32)).length() < 1e-3);
            }
        }
        assert!(pairs >= 1, "it caught the tank driving across");
    }

    /// It keeps after the target it locked while that one is in view,
    /// whichever is listed first; when it is gone it takes the first
    /// listed.
    #[test]
    fn it_keeps_its_lock_and_takes_the_nearest_when_it_is_gone() {
        let t = Tuning::default();
        let (one, two) = (Position::new(700.0, 300.0), Position::new(400.0, 120.0));
        let load = full_load(&t);
        let mut a = jet(one, &t);
        let ticks = fly(&mut a, SEAT, 60.0, &t, |i, a| {
            let (first, second) = (tank(1, one, Vec2::zero()), tank(2, two, Vec2::zero()));
            if i == 0 {
                vec![first, second]
            } else if a.ammo == load {
                vec![second, first]
            } else {
                vec![second]
            }
        });
        let dropped = strikes(&ticks, true);
        assert_eq!(dropped.len(), 4);
        assert!((between(&dropped[..2]) - one).length() < 1e-3, "the first pass is on its lock");
        assert!((between(&dropped[2..]) - two).length() < 1e-3, "the second on what is left");
    }

    /// Nothing is fired on the step it arrives, and nothing with its ground
    /// point off the field.
    #[test]
    fn it_holds_its_fire_as_it_arrives_and_off_the_field() {
        let t = Tuning::default();
        // Arriving over the field, lined up inside the bombs' release.
        let target = Position::new(300.0, 300.0);
        let mut a = Aircraft::arrive(1, 0, Sortie::Jet, Position::new(220.0, 300.0), target, &t);
        let ticks = fly(&mut a, SEAT, 0.1, &t, |_, _| parked(target));
        assert!(ticks[0].acts.is_empty(), "nothing on the step it arrives");
        assert!(!ticks[1].acts.is_empty(), "and at once after it");
        // A target by the edge it comes in over: every pass is from outside.
        let target = Position::new(40.0, 300.0);
        let mut a = jet(target, &t);
        let ticks = fly(&mut a, SEAT, 60.0, &t, |_, _| parked(target));
        for tick in ticks.iter().filter(|tick| !tick.acts.is_empty()) {
            assert!(over(tick.after.ground, FIELD), "fired from {:?}", tick.after.ground);
            assert!(tick.strikes(false).into_iter().all(|at| over(at, FIELD)), "a burst landed off the field");
        }
        assert_eq!(strikes(&ticks, true).len(), 2 * t.air_jet_passes as usize, "it still made its passes");
    }

    /// With nothing to attack it flies to its seat, goes once round it on
    /// its ring, rolls out and leaves, firing nothing.
    #[test]
    fn with_nothing_in_view_it_circles_its_seat_once_and_leaves() {
        let t = Tuning::default();
        let ring = t.air_jet_orbit_px;
        for seat in [SEAT, MIDDLE, Position::new(1000.0, 500.0)] {
            let mut a = called(seat, &t);
            let ticks = fly(&mut a, seat, 60.0, &t, |_, _| Vec::new());
            assert!(ticks.iter().all(|tick| tick.acts.is_empty()), "{seat:?}");
            let circling: Vec<&Tick> = ticks.iter().filter(|tick| tick.before.stage == AirStage::Loiter).collect();
            let nose: f32 = circling.iter().map(|tick| tick.turned()).sum();
            assert!((nose.abs() - TAU).abs() < 0.05, "{seat:?}: its nose came round {nose}");
            let flown = round(&ticks, seat, ring * ORBIT_REACH);
            assert!(flown.abs() > TAU * 0.9 && flown.signum() == nose.signum(), "{seat:?}: it flew {flown} round its seat");
            // From the ring on, it holds the ring.
            let on = circling.iter().position(|tick| (tick.after.ground - seat).length() <= ring + 8.0).expect("it reached its ring");
            for tick in &circling[on..] {
                let d = (tick.after.ground - seat).length();
                assert!((d - ring).abs() <= 8.0, "{seat:?}: {d} from its seat on a ring of {ring}");
            }
            assert!((circling.len() - on) * 5 > circling.len() * 3, "{seat:?}: most of the circle is on the ring");
            assert!(a.gone(FIELD, &t), "{seat:?}");
            assert_eq!(a.ammo, full_load(&t), "{seat:?}: it took its passes home");
        }
    }

    /// A target that shows while it circles is attacked; with that one
    /// dead and a pass left it goes once round its seat and leaves.
    #[test]
    fn a_target_that_shows_is_attacked_and_then_it_circles() {
        let t = Tuning::default();
        let target = Position::new(300.0, 300.0);
        let load = full_load(&t);
        let mut a = jet(SEAT, &t);
        let ticks = fly(&mut a, SEAT, 60.0, &t, |i, a| if i >= 200 && a.ammo == load { parked(target) } else { Vec::new() });
        flies_like_a_plane("a target that shows", &ticks, &t);
        assert!(ticks[..200].iter().all(|tick| tick.acts.is_empty()));
        let dropped = strikes(&ticks, true);
        assert_eq!(dropped.len(), 2, "one pass, on the target that showed");
        assert!((between(&dropped) - target).length() < 1e-3);
        let pass = ticks.iter().position(|tick| tick.after.ammo < load).expect("the pass");
        let flown = round(&ticks[pass..], SEAT, t.air_jet_orbit_px * ORBIT_REACH);
        assert!(flown.abs() > TAU * 0.9, "then once round its seat: {flown}");
        assert!(a.jet.circled && a.gone(FIELD, &t));
    }

    /// On the end screen or with no seat it flies straight out at once and
    /// fires nothing; in the middle of its sortie, its clock sends it home.
    #[test]
    fn the_end_screen_no_seat_and_its_last_second_send_it_home() {
        let t = Tuning::default();
        let target = Position::new(300.0, 300.0);
        let marks = parked(target);
        for (live, seat) in [(false, Some(SEAT)), (true, None)] {
            let mut a = Aircraft::arrive(1, 0, Sortie::Jet, Position::new(100.0, 300.0), target, &t);
            let mut acts = Vec::new();
            a.step(&view(SEAT, &marks), DT, &t, &mut acts);
            let heading = a.heading;
            for _ in 0..600 {
                a.step(&AirView { seat, live, ..view(SEAT, &marks) }, DT, &t, &mut acts);
                assert_eq!(a.stage, AirStage::Egress);
                assert_eq!(a.heading, heading);
            }
            assert!(acts.is_empty(), "nothing fired: live {live}, seat {seat:?}");
            assert!(a.gone(FIELD, &t));
        }
        let short = Tuning { air_jet_sortie_seconds: 5.0, ..Tuning::default() };
        let target = Position::new(600.0, 300.0);
        let mut a = jet(target, &short);
        let ticks = fly(&mut a, SEAT, 60.0, &short, |_, _| parked(target));
        let home = ticks.iter().position(|tick| tick.after.stage == AirStage::Egress).expect("it went home");
        assert!((home as f32 * DT - short.air_jet_sortie_seconds).abs() <= 2.0 * DT, "at its last second: step {home}");
        assert_eq!(ticks[home].before.stage, AirStage::Loiter, "from between its passes");
        assert_eq!(a.ammo, full_load(&short) - 1, "with a pass unflown");
        assert!(a.gone(FIELD, &short));
    }

    /// A target inside its turn circle cannot be turned onto: it breaks
    /// off and flies on, and after `air_jet_go_arounds` of those it goes
    /// home with its passes.
    #[test]
    fn a_target_it_cannot_turn_onto_is_a_go_around_and_they_run_out() {
        let t = Tuning::default();
        let radius = turn_radius(&t);
        let mut a = jet(MIDDLE, &t);
        let ticks = fly(&mut a, MIDDLE, 90.0, &t, |_, a| parked(a.ground + right_of(a.heading) * radius));
        assert!(ticks.iter().all(|tick| tick.acts.is_empty()));
        assert!(ticks.iter().all(|tick| tick.after.heading == tick.before.heading), "it never turned at a target it could not reach");
        assert_eq!(a.jet.go_arounds as i32, t.air_jet_go_arounds + 1);
        assert_eq!(a.ammo, full_load(&t));
        assert!(a.gone(FIELD, &t));
    }
}
