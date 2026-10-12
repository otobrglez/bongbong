//! Headless tests for free steering (docs/free-steer.md): one per promised
//! rule, the seat of a `Steer::Free` round on an open 34 x 17 field,
//! starting at cell (10, 10) facing up. Tests cannot touch the global
//! tuning table, so every number is read off the defaults.

use super::*;
use crate::ai::Intent;
use crate::tank::{free_throttle, signed_turn, wrap_degrees};
use crate::tuning::Tuning;

const W: f32 = 1088.0;
const H: f32 = 544.0;
const DT: f32 = 1.0 / 60.0;

/// A round with no enemies on an open field plus `extra` cells, its seats
/// steering `steer`.
fn round(steer: Steer, extra: &str) -> Game {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.show_intro = false;
    game.level_overrides.mission = Some(Mission::Destroy);
    game.steer = steer;
    let map = format!("version = 1\ntanks = 0\ncells.\"10,10\" = {{ kind = \"start\" }}\n{extra}");
    game.map = MapFile::from_toml_str(&map).expect("test map parses");
    game.init(W, H);
    game
}

fn seat(game: &Game) -> Entity {
    game.player().expect("a seat")
}

/// The seat's position, facing and commanded velocity.
fn pose(game: &Game) -> (Position, f32, Vec2) {
    with_tank(&game.world, seat(game), |t| (t.position, t.rotation, t.velocity))
}

/// The seat's body velocity: what it actually does.
fn body_velocity(game: &Game) -> Vec2 {
    let body = with_tank(&game.world, seat(game), |t| t.body).expect("a body");
    game.physics.velocity(body)
}

fn drive(game: &mut Game, intent: Intent, frames: usize) {
    for _ in 0..frames {
        game.update(Input::single(intent), DT, W, H);
    }
}

fn steer(game: &mut Game, heading: f32, frames: usize) {
    drive(game, Intent { heading: Some(heading), ..Intent::default() }, frames);
}

/// A row of iron along map row 7, columns 4 to 24: a wall above the seat.
fn wall_above() -> String {
    (4..=24).map(|c| format!("cells.\"{c},7\" = {{ kind = \"wall\", material = \"iron\" }}\n")).collect()
}

#[test]
fn angles_fold_and_turn_the_short_way() {
    assert_eq!(wrap_degrees(-90.0), 270.0);
    assert_eq!(wrap_degrees(450.0), 90.0);
    assert!(wrap_degrees(-1e-7) < 360.0, "a sliver under 0 never folds to 360");
    assert_eq!(signed_turn(350.0, 10.0), 20.0);
    assert_eq!(signed_turn(10.0, 350.0), -20.0);
    assert_eq!(signed_turn(0.0, 180.0), 180.0, "a reversal turns clockwise");
    assert_eq!(Dir::nearest(44.0), Dir::Up);
    assert_eq!(Dir::nearest(45.0), Dir::Right, "a tie goes clockwise");
    assert_eq!(Dir::nearest(200.0), Dir::Down);
    assert_eq!(Dir::nearest(-30.0), Dir::Up);
    assert_eq!(Dir::nearest(300.0), Dir::Left);
}

/// The throttle is full on the heading, the cosine of the turn left inside
/// the pivot angle, and nothing at or past it.
#[test]
fn the_throttle_falls_to_nothing_at_the_pivot_angle() {
    let t = Tuning::DEFAULT;
    assert_eq!(free_throttle(0.0, &t), 1.0);
    let half = free_throttle(t.tank_free_pivot_deg * 2.0 / 3.0, &t);
    assert!((half - 0.5).abs() < 1e-5, "two thirds of the way to the pivot is cos 60: {half}");
    assert_eq!(free_throttle(t.tank_free_pivot_deg, &t), 0.0);
    assert_eq!(free_throttle(170.0, &t), 0.0);
}

/// A hull's facing on the four ways is the direction's own vector; between
/// them it is the angle's.
#[test]
fn the_forward_vector_is_exact_on_the_four_ways() {
    let mut tank = Tank::default();
    for dir in Dir::ALL {
        tank.rotation = dir.rotation();
        assert_eq!(tank.forward(), dir.vec());
        assert_eq!(tank.facing(), dir);
        assert!(tank.faces_cardinal());
    }
    tank.rotation = 45.0;
    let f = tank.forward();
    assert!((f.x - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6 && (f.y + std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6, "{f:?}");
    assert!(!tank.faces_cardinal());
}

/// The hull turns toward the stick at `tank_free_turn_speed_deg`, the short
/// way round, and lands on it exactly.
#[test]
fn a_free_seat_turns_toward_the_stick_at_its_turn_rate() {
    let t = Tuning::DEFAULT;
    let mut game = round(Steer::Free, "");
    steer(&mut game, 90.0, 1);
    let (_, rotation, _) = pose(&game);
    let step = t.tank_free_turn_speed_deg * DT;
    assert!((rotation - step).abs() < 1e-3, "one step of the turn: {rotation} for {step}");
    let frames = (90.0 / step).ceil() as usize;
    steer(&mut game, 90.0, frames);
    assert_eq!(pose(&game).1, 90.0, "on the heading exactly");
    // The short way: from 90 to 315 is 135 degrees anticlockwise.
    steer(&mut game, 315.0, 1);
    assert!((pose(&game).1 - (90.0 - step)).abs() < 1e-3, "anticlockwise: {}", pose(&game).1);
}

/// On its heading the hull drives at its full speed along it, any angle at
/// all.
#[test]
fn a_free_seat_drives_any_way() {
    for heading in [45.0f32, 30.0, 200.0] {
        let mut game = round(Steer::Free, "");
        steer(&mut game, heading, 30);
        let (from, rotation, velocity) = pose(&game);
        assert_eq!(rotation, heading);
        let speed = with_tank(&game.world, seat(&game), |t| t.effective_speed());
        assert!((velocity.length() - speed).abs() < 1e-3, "full speed on its heading: {velocity:?}");
        steer(&mut game, heading, 20);
        let moved = pose(&game).0 - from;
        let course = wrap_degrees(crate::math::atan2(moved.x, -moved.y).to_degrees());
        assert!(signed_turn(course, heading).abs() < 0.5, "it goes the way it faces: {course} for {heading}");
        assert!((moved.length() - speed / 3.0).abs() < 2.0, "at its top speed: {} in a third of a second", moved.length());
    }
}

/// A turn wider than `tank_free_pivot_deg` pivots in place first: no drive
/// until the turn left is inside it.
#[test]
fn a_reversal_pivots_before_it_drives() {
    let t = Tuning::DEFAULT;
    let mut game = round(Steer::Free, "");
    steer(&mut game, 0.0, 30);
    let step = t.tank_free_turn_speed_deg * DT;
    let pivot_frames = ((180.0 - t.tank_free_pivot_deg) / step).floor() as usize;
    for frame in 0..pivot_frames {
        steer(&mut game, 180.0, 1);
        let (_, rotation, velocity) = pose(&game);
        assert_eq!(velocity, Vec2::zero(), "frame {frame}: still pivoting at {rotation}, no drive");
    }
    steer(&mut game, 180.0, 30);
    let (from, rotation, _) = pose(&game);
    assert_eq!(rotation, 180.0);
    steer(&mut game, 180.0, 10);
    assert!(pose(&game).0.y > from.y + 20.0, "it drives back down once round");
}

/// With no heading a free hull steers `move_dir`'s way, at its turn rate.
#[test]
fn a_free_seat_with_only_a_direction_steers_that_way() {
    let t = Tuning::DEFAULT;
    let mut game = round(Steer::Free, "");
    drive(&mut game, Intent { move_dir: Some(Dir::Right), ..Intent::default() }, 1);
    assert!((pose(&game).1 - t.tank_free_turn_speed_deg * DT).abs() < 1e-3);
}

/// A four-way round drives `move_dir` and never reads the heading: the
/// shipped game.
#[test]
fn a_four_way_seat_ignores_the_heading() {
    let mut game = round(Steer::Cardinal, "");
    drive(&mut game, Intent { move_dir: Some(Dir::Right), heading: Some(45.0), ..Intent::default() }, 1);
    let (_, rotation, velocity) = pose(&game);
    assert_eq!(rotation, Dir::Right.rotation(), "snapped to the key");
    assert_eq!(velocity.y, 0.0);
    drive(&mut game, Intent { heading: Some(45.0), ..Intent::default() }, 1);
    assert_eq!(pose(&game).2, Vec2::zero(), "a heading alone does not drive it");
}

/// A free seat's body is round, its radius the movement box's half-length,
/// and a four-way seat's a box.
#[test]
fn a_free_seat_stands_on_a_round_collider() {
    let game = round(Steer::Free, "");
    let (body, radius, steer) = with_tank(&game.world, seat(&game), |t| (t.body.expect("a body"), t.move_radius(), t.steer));
    assert_eq!(steer, Steer::Free);
    assert_eq!(game.physics.round_radius(body), Some(radius));
    let (hx, hy) = with_tank(&game.world, seat(&game), |t| t.move_half_extents(false));
    assert_eq!(radius, hx.max(hy));
    let game = round(Steer::Cardinal, "");
    let body = with_tank(&game.world, seat(&game), |t| t.body.expect("a body"));
    assert_eq!(game.physics.round_radius(body), None);
}

/// Head-on a free hull stops at a wall where a four-way one does: the
/// circle's radius is the box's half-length.
#[test]
fn a_free_hull_stops_head_on_where_a_four_way_one_does() {
    let mut free = round(Steer::Free, &wall_above());
    steer(&mut free, 0.0, 90);
    let mut four = round(Steer::Cardinal, &wall_above());
    drive(&mut four, Intent { move_dir: Some(Dir::Up), ..Intent::default() }, 90);
    let (a, b) = (pose(&free).0.y, pose(&four).0.y);
    assert!((a - b).abs() < 0.1, "free {a}, four-way {b}");
}

/// Driven at a wall on a slant, a free hull slides along it - no friction
/// holds it - and its tracks never push it past its top speed.
#[test]
fn a_free_seat_slides_along_a_wall_it_meets_on_a_slant() {
    let mut game = round(Steer::Free, &wall_above());
    steer(&mut game, 0.0, 90);
    let (at_wall, _, _) = pose(&game);
    let speed = with_tank(&game.world, seat(&game), |t| t.effective_speed());
    for _ in 0..60 {
        steer(&mut game, 45.0, 1);
        let v = body_velocity(&game);
        assert!(v.length() <= speed + 1.0, "never past its top speed: {v:?}");
        assert!((pose(&game).0.y - at_wall.y).abs() < 2.0, "kept against the wall: {:?}", pose(&game).0);
    }
    let slid = pose(&game).0.x - at_wall.x;
    assert!(slid > speed * 0.6, "it slid along the wall: {slid} px in a second");
}

/// A shot meets a turned hull on its true boxes, not the box around them:
/// at 45 degrees a line past the hull's corner, inside the box around it,
/// misses, and one through the middle hits. On the four ways the test is
/// the axis-aligned one, box for box.
#[test]
fn a_shot_meets_a_turned_hull_on_its_true_box() {
    let pad = Position::new(1.0, 1.0);
    let mut tank = Tank { position: Position::new(200.0, 200.0), ..Tank::default() };
    for dir in Dir::ALL {
        tank.rotation = dir.rotation();
        let (p0, p1) = (Position::new(100.0, 205.0), Position::new(300.0, 205.0));
        let (hull, turret) = (tank.hull_bbox_world(), tank.turret_bbox_world());
        let aabb = [hits::segment_hits_aabb(p0, p1, hull.0, hull.1 + pad), hits::segment_hits_aabb(p0, p1, turret.0, turret.1 + pad)];
        assert_eq!(hits::tank_entries(&tank, p0, p1, pad), aabb, "{dir:?}");
    }
    tank.rotation = 45.0;
    let (centre, around) = tank.hull_bbox_world();
    // A short stroke just inside the top-left corner of the box around the
    // turned hull, where the box reaches and the hull does not.
    let (x, y) = (centre.x - around.x + 2.0, centre.y - around.y + 2.0);
    let (p0, p1) = (Position::new(x, y), Position::new(x + 3.0, y));
    assert!(hits::segment_hits_aabb(p0, p1, centre, around + pad).is_some(), "inside the box around it");
    assert_eq!(hits::tank_entries(&tank, p0, p1, pad)[0], None, "but beside the turned hull");
    let middle = hits::tank_entries(&tank, Position::new(100.0, centre.y), Position::new(300.0, centre.y), pad);
    assert!(middle[0].is_some(), "through the middle it hits");
}

/// A parked-free enemy of the round's steering at `at`: shells only, no
/// shield, its fire timer clear.
fn enemy_at(game: &mut Game, at: Position) -> Entity {
    let slot = game.debug_spawn_enemy(at, Some(1), Some(crate::ai::Role::Player)).expect("spawns");
    let entity = game.tank_entity_by_slot(slot).expect("exists");
    with_tank_mut(&game.world, entity, |t| {
        t.disarm();
        t.shield_hp = 0.0;
        t.shield_timer = 0.0;
    });
    entity
}

/// An enemy of a free round steers freely, on a round collider; a
/// four-way round's is the box it always was.
#[test]
fn an_enemy_in_a_free_round_steers_freely() {
    let mut game = round(Steer::Free, "");
    let enemy = enemy_at(&mut game, map::cell_to_world(25, 12));
    let (steer, body, radius) = with_tank(&game.world, enemy, |t| (t.steer, t.body.expect("a body"), t.move_radius()));
    assert_eq!(steer, Steer::Free);
    assert_eq!(game.physics.round_radius(body), Some(radius));
    let mut game = round(Steer::Cardinal, "");
    let enemy = enemy_at(&mut game, map::cell_to_world(25, 12));
    let (steer, body) = with_tank(&game.world, enemy, |t| (t.steer, t.body.expect("a body")));
    assert_eq!(steer, Steer::Cardinal);
    assert_eq!(game.physics.round_radius(body), None);
}

/// A free enemy drives its route straight across open ground at an angle,
/// not in four-way steps: on its way to a seat up and to the left, its
/// facing leaves the four ways and both its coordinates fall together.
#[test]
fn a_free_enemy_drives_its_route_at_an_angle() {
    let mut game = round(Steer::Free, "");
    let enemy = enemy_at(&mut game, map::cell_to_world(26, 15));
    let from = with_tank(&game.world, enemy, |t| t.position);
    let mut turned = false;
    for _ in 0..40 {
        game.update(Input::default(), DT, W, H);
        turned |= with_tank(&game.world, enemy, |t| !t.faces_cardinal());
    }
    let moved = with_tank(&game.world, enemy, |t| t.position) - from;
    assert!(turned, "its facing left the four ways");
    assert!(moved.x < -20.0 && moved.y < -10.0, "up and to the left at once: {moved:?}");
}

/// The probe's turn rules (`simulation::debug`): a four-way hull's facing
/// is read as it is, a free hull's as the nearest way past a hysteresis
/// band; the spin chain steps by the angle turned, a reversal breaking it.
#[test]
fn the_turn_rules_read_a_free_hull_by_the_angle_it_turns() {
    use crate::simulation::debug::{spin_step, turn_heading, signed_quarter_turn};
    for held in [0.0, 90.0, 180.0, 270.0] {
        for rotation in [0.0, 90.0, 180.0, 270.0] {
            assert_eq!(turn_heading(held, rotation), rotation, "a four-way hull's own facing");
            if held != rotation {
                assert_eq!(spin_step(held, rotation), signed_quarter_turn(held, rotation), "a four-way hull's quarter turns");
            }
        }
    }
    assert_eq!(turn_heading(90.0, 44.0), 90.0, "a degree past the diagonal is held");
    assert_eq!(turn_heading(90.0, 34.0), 0.0, "past the band it is the next way");
    assert_eq!(turn_heading(0.0, 355.5), 0.0);
    assert_eq!(spin_step(350.0, 10.0), Some(20.0));
    assert_eq!(spin_step(0.0, 180.0), None, "a reversal breaks the chain");
}

/// A free round plays the same round from the same seed and the same
/// sticks, bit for bit.
#[test]
fn a_free_round_replays_bit_for_bit() {
    let run = || {
        let mut game = round(Steer::Free, &wall_above());
        let mut poses = Vec::new();
        for frame in 0..240u32 {
            let heading = (frame * 7 % 360) as f32;
            drive(&mut game, Intent { heading: Some(heading), fire: frame % 20 == 0, ..Intent::default() }, 1);
            let (p, r, _) = pose(&game);
            poses.push((p.x.to_bits(), p.y.to_bits(), r.to_bits()));
        }
        poses
    };
    assert_eq!(run(), run());
}

