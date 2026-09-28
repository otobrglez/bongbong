//! Lag compensation and shoves on owned hulls (docs/online-coop-prd.md
//! §4.16): a seat's shots and beams are judged against the enemies and
//! frogs its client was drawing (`Game::set_seat_view`, the hit-box
//! history), and the velocity changes the room puts on a client-owned
//! hull travel as `Event::Shoved`.
//!
//! The scenarios are scripted rather than played: the enemy is placed by
//! hand before every tick, so "where the enemy was six ticks ago" is a
//! number the test knows, and its AI's own drift inside one tick is a few
//! pixels against margins of tens.

use super::*;
use crate::map::cell_to_world;

const W: f32 = 1280.0;
const H: f32 = 720.0;

/// Seat 0 at (5,10) facing east down an open row, one enemy, the frog far
/// out of every line of fire, a barrel south-west of the start and inside
/// its blast radius for the blast case.
const MAP: &str = r#"
version = 1
tanks = 1
cells."2,2" = { kind = "frog" }
cells."5,10" = { kind = "start" }
cells."4,12" = { kind = "barrel", drum = "oil" }
"#;

/// The barrel's cell.
const BARREL: (i32, i32) = (4, 12);

/// Ticks the round runs before a rewound shot, so the seat's view lies
/// inside the history.
const WARMUP_TICKS: usize = 20;

/// How far behind the room's tick the seat's client draws, in the tests
/// that rewind.
const BEHIND: u64 = 6;

/// How far the enemy moves across the line of fire each tick once it
/// moves: fast enough that six ticks carry its whole box off the line.
const ENEMY_STEP_PX: f32 = 12.0;

fn game() -> Game {
    let mut game = Game::default();
    game.enemy_count_override = Some(1);
    game.seed_override = Some(7);
    game.player_row_override = Some(0);
    game.map = MapFile::from_toml_str(MAP).expect("test map parses");
    game.init(W, H);
    game.intro_timer = 0.0;
    let seat = game.seat(0).expect("seat 0");
    game.place_tank(seat, cell_to_world(5, 10), Some(90.0)).expect("seat placed");
    game.debug_set_tank(0, &debug::TankPatch { shield_hp: Some(0.0), ..Default::default() }).expect("seat 0");
    // The enemy holds its fire: nothing but the seat's shot is in the air.
    let slot = enemy_slot(&game);
    game.debug_set_tank(slot, &debug::TankPatch { shells_ammo: Some(0), shield_hp: Some(0.0), ..Default::default() })
        .expect("the enemy");
    game
}

fn enemy_slot(game: &Game) -> usize {
    game.first_enemy_slot()
}

/// The centre of the row the seat fires along.
fn line_y() -> f32 {
    cell_to_world(5, 10).y
}

/// Where the enemy stands when it is on the line: ten cells east.
fn enemy_home() -> Position {
    Position::new(cell_to_world(15, 10).x, line_y())
}

fn place_enemy(game: &mut Game, at: Position) {
    let slot = enemy_slot(game);
    game.debug_teleport(slot, at, Some(180.0)).expect("the enemy is on the field");
}

fn fire() -> Input {
    Input::single(Intent { fire: true, ..Intent::default() })
}

fn step(game: &mut Game, input: Input) {
    game.update(input, PHYSICS_FIXED_DT, W, H);
}

/// Tell the room the seat's client is drawing `BEHIND` ticks before the
/// update about to run - which is what `view_tick` says on a link that
/// far behind.
fn view_behind(game: &mut Game, behind: u64) {
    let next = game.frame() + 1;
    game.set_seat_view(0, (next - behind) as u32, 0);
}

fn enemy_hit(game: &Game) -> Option<Position> {
    let slot = enemy_slot(game);
    game.events().iter().find_map(|e| match e {
        Event::Hit { target: HitTarget::Enemy { slot: s }, x, y, .. } if *s == slot => Some(Position::new(*x, *y)),
        _ => None,
    })
}

fn enemy_boxes(game: &Game) -> [(Position, Position); 2] {
    let entity = game.tank_entity_by_slot(enemy_slot(game)).expect("the enemy");
    with_tank(&game.world, entity, |t| [t.hull_bbox_world(), t.turret_bbox_world()])
}

/// The seat's shell, once it has left the muzzle.
fn flying_shell_x(game: &Game) -> Option<f32> {
    game.world
        .query::<&Shell>()
        .iter()
        .find(|s| s.owner == Owner::Player(0) && s.state == ShellState::Flying)
        .map(|s| s.position.x)
}

/// What a shot fired down the line reports for this tick's enemy boxes:
/// whether the line - grown by a player's shell half-extent and pad -
/// crosses them.
fn line_crosses(boxes: &[(Position, Position); 2]) -> bool {
    let reach = tuning().shell_hit_half_extent + tuning().player_shot_hit_pad_px;
    boxes.iter().any(|(c, h)| (c.y - line_y()).abs() < h.y + reach)
}

struct ShotOutcome {
    /// Where the enemy was hit, if it was.
    hit: Option<Position>,
    /// Whether the enemy's boxes at the tick the shell reached it still
    /// crossed the line of fire - false means a hit was on the past.
    crossed_now: bool,
}

/// Fire one shell east down the line at the enemy, which starts to move
/// south across it once the shell is `BEHIND` ticks of flight short of
/// its box - so the shell reaches the box's column with the enemy
/// `BEHIND` ticks' travel off the line, and the box `BEHIND` ticks ago
/// still on it. `view` is whether the seat's client draws that far back.
fn shell_across(view: bool) -> ShotOutcome {
    let mut game = game();
    let home = enemy_home();
    let step_px = tuning().shell_speed * PHYSICS_FIXED_DT;
    let reach = tuning().shell_hit_half_extent + tuning().player_shot_hit_pad_px;
    for _ in 0..WARMUP_TICKS {
        place_enemy(&mut game, home);
        step(&mut game, Input::default());
    }
    let mut enemy_y = home.y;
    let mut moving = false;
    let mut pressed = false;
    for _ in 0..240 {
        if !moving
            && let Some(x) = flying_shell_x(&game)
        {
            let [(hull_c, hull_h), _] = enemy_boxes(&game);
            let face = hull_c.x - hull_h.x - reach;
            moving = face - x <= BEHIND as f32 * step_px;
        }
        if moving {
            enemy_y += ENEMY_STEP_PX;
        }
        place_enemy(&mut game, Position::new(home.x, enemy_y));
        if view {
            view_behind(&mut game, BEHIND);
        }
        step(&mut game, if pressed { Input::default() } else { fire() });
        pressed = true;
        if let Some(hit) = enemy_hit(&game) {
            return ShotOutcome { hit: Some(hit), crossed_now: line_crosses(&enemy_boxes(&game)) };
        }
        if moving && flying_shell_x(&game).is_none_or(|x| x > home.x + 64.0) {
            return ShotOutcome { hit: None, crossed_now: line_crosses(&enemy_boxes(&game)) };
        }
    }
    panic!("the shell never reached the enemy's column");
}

#[test]
fn a_shot_from_a_seat_drawing_the_past_hits_where_the_enemy_was() {
    let with_view = shell_across(true);
    let hit = with_view.hit.expect("the rewound sweep meets the enemy where the client drew it");
    assert!(!with_view.crossed_now, "the enemy's present box is off the line: this hit is on the past");
    assert!((hit.y - line_y()).abs() < 1.0, "the impact is on the line of fire: {hit:?}");
    assert!(hit.x < enemy_home().x, "the shell struck the near face of the rewound box: {hit:?}");
}

#[test]
fn the_same_shot_with_no_view_misses_the_moving_enemy() {
    let outcome = shell_across(false);
    assert_eq!(outcome.hit, None, "judged in the present, the shell passes behind the enemy");
}

/// Hold the enemy on the line, then jump it off just before the seat
/// fires its laser; the beam is judged with the seat drawing `behind`
/// ticks back (0: no view at all).
fn laser_after_the_enemy_left(behind: u64) -> (Option<Position>, f32) {
    let mut game = game();
    game.debug_set_tank(0, &debug::TankPatch { laser_charges: Some(1), ..Default::default() }).expect("seat 0");
    let home = enemy_home();
    for _ in 0..WARMUP_TICKS {
        place_enemy(&mut game, home);
        step(&mut game, Input::default());
    }
    place_enemy(&mut game, Position::new(home.x, home.y + 96.0));
    if behind > 0 {
        view_behind(&mut game, behind);
    }
    step(&mut game, fire());
    let beam_end = game
        .events()
        .iter()
        .find_map(|e| match e {
            Event::LaserBeam { x1, seat: 0, .. } => Some(*x1),
            _ => None,
        })
        .expect("the seat fired its laser");
    assert!(!line_crosses(&enemy_boxes(&game)), "the enemy stands off the line when the beam goes out");
    (enemy_hit(&game), beam_end)
}

#[test]
fn a_laser_from_a_seat_drawing_the_past_hits_where_the_enemy_was() {
    let (hit, beam_end) = laser_after_the_enemy_left(BEHIND);
    let hit = hit.expect("the beam meets the enemy the client drew");
    assert!((hit.y - line_y()).abs() < 1.0);
    assert!(beam_end < enemy_home().x, "the beam stops at the rewound box: {beam_end}");
}

#[test]
fn the_same_laser_with_no_view_goes_past_the_enemy() {
    let (hit, beam_end) = laser_after_the_enemy_left(0);
    assert_eq!(hit, None);
    assert!(beam_end > enemy_home().x + 64.0, "the beam runs on to the far wall: {beam_end}");
}

#[test]
fn a_shot_a_shield_turns_back_is_judged_in_the_present() {
    let mut game = game();
    let slot = enemy_slot(&game);
    game.debug_set_tank(slot, &debug::TankPatch { shield_hp: Some(500.0), ..Default::default() }).expect("the enemy");
    let home = enemy_home();
    for _ in 0..WARMUP_TICKS {
        place_enemy(&mut game, home);
        step(&mut game, Input::default());
    }
    let mut pressed = false;
    for _ in 0..240 {
        place_enemy(&mut game, home);
        view_behind(&mut game, BEHIND);
        step(&mut game, if pressed { Input::default() } else { fire() });
        if !pressed {
            let rewinds: Vec<u8> = game.world.query::<(&Shell, &Rewind)>().iter().map(|(_, r)| r.0).collect();
            assert_eq!(rewinds, vec![BEHIND as u8], "the seat's shell carries its seat's rewind");
        }
        pressed = true;
        if game.events().iter().any(|e| matches!(e, Event::Deflected { .. })) {
            let turned: Vec<(Owner, u8)> =
                game.world.query::<(&Shell, &Rewind)>().iter().map(|(s, r)| (s.owner, r.0)).collect();
            assert_eq!(turned, vec![(Owner::Enemy(slot), 0)], "the shield's shell, judged in the present");
            return;
        }
    }
    panic!("the shield never turned the shell");
}

#[test]
fn a_view_further_back_than_the_ring_is_judged_at_its_oldest_tick() {
    // `seat_rewind` measures from the frame in hand, which inside an
    // update is that update's; here it is the last one run.
    let mut game = game();
    for _ in 0..30 {
        step(&mut game, Input::default());
    }
    let now = game.frame();
    game.set_seat_view(0, 1, 0);
    assert_eq!(game.seat_rewind(Owner::Player(0)), REWIND_MAX_TICKS);
    // A view of the present or ahead of it rewinds nothing.
    game.set_seat_view(0, now as u32, 0);
    assert_eq!(game.seat_rewind(Owner::Player(0)), 0);
    game.set_seat_view(0, now as u32 + 3, 0);
    assert_eq!(game.seat_rewind(Owner::Player(0)), 0);
    // The fraction rounds to the nearest whole tick.
    game.set_seat_view(0, (now - 6) as u32, 100);
    assert_eq!(game.seat_rewind(Owner::Player(0)), 6, "5.61 ticks behind");
    game.set_seat_view(0, (now - 6) as u32, 200);
    assert_eq!(game.seat_rewind(Owner::Player(0)), 5, "5.22 ticks behind");
    // An enemy's shot and a seat with no view are judged in the present.
    assert_eq!(game.seat_rewind(Owner::Enemy(enemy_slot(&game))), 0);
    assert_eq!(game.seat_rewind(Owner::Player(1)), 0);
}

#[test]
fn the_history_keeps_the_last_ticks_and_forgets_older_ones() {
    let mut game = game();
    let start = game.frame();
    for _ in 0..40 {
        step(&mut game, Input::default());
    }
    let now = game.frame();
    assert_eq!(now, start + 40);
    for back in 0..REWIND_MAX_TICKS as u64 {
        assert!(game.hit_history.at(now - back).is_some(), "tick {} is kept", now - back);
    }
    assert!(game.hit_history.at(now - REWIND_MAX_TICKS as u64).is_none(), "the ring holds fifteen ticks");
    let entry = game.hit_history.at(now).expect("the newest tick");
    assert_eq!(entry.tanks.len(), 1, "one enemy, and no seat");
    assert_eq!(entry.frogs.len(), 1, "the player's frog");
    game.init(W, H);
    assert!(game.hit_history.at(now).is_none(), "a new round starts with no past");
}

// --- Shoves on owned hulls ---

fn shoves(game: &Game) -> Vec<(usize, Vec2)> {
    game.events()
        .iter()
        .filter_map(|e| match e {
            Event::Shoved { seat, vx, vy } => Some((*seat, Vec2::new(*vx, *vy))),
            _ => None,
        })
        .collect()
}

/// The client says its hull is where the room already has it, standing
/// still: the seat is owned for the coming update.
fn own_seat(game: &mut Game) {
    let pose = game.seat_pose(0).expect("seat 0 has a pose");
    let pose = SeatPose { velocity: Vec2::new(0.0, 0.0), ..pose };
    game.accept_seat_pose(0, pose).expect("the room takes the pose");
}

/// An enemy shell a few pixels west of the seat's hull, flying east into
/// it, so the coming update's sweep lands it.
fn shell_into_seat(game: &mut Game) {
    let seat = game.seat(0).expect("seat 0");
    let (hull_c, hull_h) = with_tank(&game.world, seat, |t| t.hull_bbox_world());
    let at = Position::new(hull_c.x - hull_h.x - 4.0, hull_c.y);
    let velocity = Vec2::new(tuning().shell_speed, 0.0);
    let owner = Owner::Enemy(enemy_slot(game));
    game.world.spawn((Shell::at(9_999, at, at, velocity, 90.0, 0, 0, owner),));
}

fn park_enemy_far(game: &mut Game) {
    place_enemy(game, Position::new(cell_to_world(30, 3).x, cell_to_world(30, 3).y));
}

fn seat_hit(game: &Game) -> bool {
    game.events().iter().any(|e| matches!(e, Event::Hit { target: HitTarget::Player { player: 0 }, killed: false, .. }))
}

#[test]
fn an_owned_seat_hit_by_a_shell_is_told_the_knockback() {
    let mut game = game();
    park_enemy_far(&mut game);
    step(&mut game, Input::default());
    own_seat(&mut game);
    shell_into_seat(&mut game);
    step(&mut game, Input::default());
    assert!(seat_hit(&game), "the shell landed on the seat");
    let shoves = shoves(&game);
    assert_eq!(shoves.len(), 1, "one shove: {shoves:?}");
    let (seat, dv) = shoves[0];
    assert_eq!(seat, 0);
    let speed = tuning().shell_impact_knockback_speed;
    assert!((dv.x - speed).abs() < 1e-3 && dv.y.abs() < 1e-3, "east at the knockback speed: {dv:?}");
}

#[test]
fn a_seat_nobody_owns_is_shoved_in_the_world_and_told_nothing() {
    let mut game = game();
    park_enemy_far(&mut game);
    step(&mut game, Input::default());
    shell_into_seat(&mut game);
    step(&mut game, Input::default());
    assert!(seat_hit(&game), "the shell landed on the seat");
    assert!(shoves(&game).is_empty());
}

#[test]
fn an_owned_seat_is_told_its_recoil() {
    let mut game = game();
    park_enemy_far(&mut game);
    step(&mut game, Input::default());
    own_seat(&mut game);
    step(&mut game, fire());
    assert!(game.events().iter().any(|e| matches!(e, Event::Fired { slot: 0, .. })), "the seat fired");
    let shoves = shoves(&game);
    assert_eq!(shoves.len(), 1, "one kick: {shoves:?}");
    let (seat, dv) = shoves[0];
    assert_eq!(seat, 0);
    assert!(dv.x < 0.0 && dv.y.abs() < 1e-3, "back along the barrel, which points east: {dv:?}");
    assert!(-dv.x <= tuning().shell_recoil_max_speed + 1e-3);
}

#[test]
fn an_owned_seat_beside_a_blast_is_told_the_shove() {
    let mut game = game();
    park_enemy_far(&mut game);
    step(&mut game, Input::default());
    game.debug_detonate(cell_to_world(BARREL.0, BARREL.1)).expect("the barrel");
    own_seat(&mut game);
    step(&mut game, Input::default());
    assert!(game.events().iter().any(|e| matches!(e, Event::Blast { .. })), "the barrel went off");
    let shoves = shoves(&game);
    assert_eq!(shoves.len(), 1, "one shove: {shoves:?}");
    let (seat, dv) = shoves[0];
    assert_eq!(seat, 0);
    // The barrel is south-west of the seat: the shove is north-east.
    assert!(dv.x > 0.0 && dv.y < 0.0, "away from the barrel: {dv:?}");
}

#[test]
fn a_local_round_emits_no_shoves() {
    // Three enemies in the open and a seat that drives and fires: hits,
    // knockback, recoil and rams all happen, and none of it is anybody's
    // to be told about.
    let map = r#"
version = 1
tanks = 3
cells."2,2" = { kind = "frog" }
cells."5,10" = { kind = "start" }
"#;
    let mut game = Game::default();
    game.enemy_count_override = Some(3);
    game.seed_override = Some(11);
    game.map = MapFile::from_toml_str(map).expect("test map parses");
    game.init(W, H);
    let mut fired = 0;
    for frame in 0..1200u32 {
        let intent = Intent {
            move_dir: Some(if (frame / 90) % 2 == 0 { Dir::Right } else { Dir::Left }),
            fire: frame % 20 == 0,
            ..Intent::default()
        };
        step(&mut game, Input::single(intent));
        fired += game.events().iter().filter(|e| matches!(e, Event::Fired { .. })).count();
        assert!(shoves(&game).is_empty(), "frame {frame}: {:?}", shoves(&game));
    }
    assert!(fired > 10, "the round saw real fire: {fired}");
}
