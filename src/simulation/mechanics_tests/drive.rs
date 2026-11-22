//! The drive (docs/physics-engine-design.md "Driving"): the turn that
//! fits, the buffered turn and the lane assist, weight, wrecks that
//! resist, bongs clear of hulls and the prop ram.

use super::*;
use crate::tuning::Tuning;

/// The scout, the assault (the standard weight) and the titan: the
/// lightest chassis, the one every mass is measured against, the heaviest.
const SCOUT: i32 = 0;
const ASSAULT: i32 = 1;
const TITAN: i32 = 10;

fn cell(c: i32, r: i32) -> Position {
    crate::map::cell_to_world(c, r)
}

fn iron(cells: impl IntoIterator<Item = (i32, i32)>) -> String {
    cells.into_iter().map(|(c, r)| format!("cells.\"{c},{r}\" = {{ kind = \"wall\", material = \"iron\" }}\n")).collect()
}

/// `f` with `table` in force on this thread.
fn under<R>(table: Tuning, f: impl FnOnce() -> R) -> R {
    crate::tuning::in_force(Some(&std::sync::Arc::new(table)), f)
}

/// A round of `seats` seats on a field holding only `extra`, seat 0 a
/// `row` chassis and seat 1 a `row2`, stepped once so the solver knows
/// the map's colliders.
fn round_of(seats: usize, extra: &str, row: i32, row2: i32) -> Game {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.players = PlayerCount::from_count(seats).expect("a seat count");
    game.player_row_override = Some(row);
    game.player2_row_override = Some(row2);
    game.level_overrides.mission = Some(Mission::Destroy);
    game.map = MapFile::from_toml_str(&format!("version = 1\ntanks = 0\ncells.\"1,1\" = {{ kind = \"start\" }}\n{extra}")).expect("test map parses");
    game.init(W, H);
    step(&mut game, Input::default());
    game
}

fn round(extra: &str, row: i32) -> Game {
    round_of(1, extra, row, row)
}

fn body_speed(game: &Game, tank: Entity) -> f32 {
    game.physics.velocity(with_tank(&game.world, tank, |t| t.body).expect("a body")).length()
}

/// Iron over columns 8..=16 and rows 4..=12 but for row 8 and column 12:
/// two lanes one cell wide, crossing at (12, 8).
fn crossing() -> String {
    iron((8..=16).flat_map(|c| (4..=12).map(move |r| (c, r))).filter(|&(c, r)| c != 12 && r != 8))
}

/// An assault hull standing in one of `crossing`'s lanes, 6 px past the
/// other lane's centre line on the heading `from`, turned to `turn`: how
/// far (px) its box reaches into the iron beside the lane it turned into
/// once that step is done, and how far along the lane it is 40 steps on.
fn turned_in(table: &Tuning, from: Dir, turn: Dir) -> (f32, f32) {
    under(table.clone(), || {
        let mut game = round(&crossing(), ASSAULT);
        let player = game.player().expect("player");
        let centre = cell(12, 8);
        let start = centre + from.vec() * 6.0;
        game.place_tank(player, start, Some(from.rotation())).unwrap();
        drive(&mut game, Some(turn), 1);
        let across = |p: Position| if turn.vec().x != 0.0 { (p.y - centre.y).abs() } else { (p.x - centre.x).abs() };
        let (hx, hy) = with_tank(&game.world, player, |t| t.move_half_extents(t.facing_along_x()));
        let sunk = across(player_pos(&game)) + hx.min(hy) - OBSTACLE_GRID_SIZE * 0.5;
        drive(&mut game, Some(turn), 40);
        (sunk, (player_pos(&game) - start).dot(turn.vec()))
    })
}

#[test]
fn a_turn_fits_into_a_lane_one_hull_wide_from_each_direction() {
    let fitted = Tuning::DEFAULT;
    let bare = Tuning { tank_turn_fit_px: 0.0, ..Tuning::DEFAULT };
    for (from, turn) in [(Dir::Right, Dir::Up), (Dir::Left, Dir::Down), (Dir::Down, Dir::Left), (Dir::Up, Dir::Right)] {
        let (sunk, travelled) = turned_in(&fitted, from, turn);
        assert!(sunk <= 0.6, "{from:?} then {turn:?}: the turned hull stands in the lane, {sunk} px into its wall");
        assert!(travelled > 64.0, "{from:?} then {turn:?}: and drives on down it, {travelled} px in 40 steps");
        let (sunk, _) = turned_in(&bare, from, turn);
        assert!(sunk > 3.0, "{from:?} then {turn:?}: with no fit it turns into the wall, {sunk} px deep");
    }
}

/// A corridor one cell high along row 6 (iron on rows 5 and 7, columns
/// 2..=24) with a lane one cell wide north from column 12.
fn lane_map() -> String {
    let mut cells: Vec<(i32, i32)> = vec![];
    for c in 2..=24 {
        if c != 12 {
            cells.push((c, 5));
        }
        cells.push((c, 7));
    }
    for r in 1..=4 {
        cells.push((11, r));
        cells.push((13, r));
    }
    iron(cells)
}

/// A scout driven east along `lane_map`'s corridor, the turn north
/// pressed and held from `offset` px past the lane's centre line (before
/// it, where negative): whether it ends up in the lane, and how far past
/// the line (px) it stood when its hull came round.
fn takes_the_lane(table: &Tuning, offset: f32) -> (bool, f32) {
    under(table.clone(), || {
        let mut game = round(&lane_map(), SCOUT);
        let player = game.player().expect("player");
        game.place_tank(player, cell(5, 6), Some(Dir::Right.rotation())).unwrap();
        let lane = cell(12, 6).x;
        let (mut held, mut turned_at) = (0, f32::NAN);
        while held < 90 {
            let pressed = held > 0 || player_pos(&game).x >= lane + offset;
            held += pressed as u32;
            let before = player_pos(&game).x - lane;
            drive(&mut game, Some(if pressed { Dir::Up } else { Dir::Right }), 1);
            if turned_at.is_nan() && with_tank(&game.world, player, |t| t.rotation) == Dir::Up.rotation() {
                turned_at = before;
            }
        }
        (player_pos(&game).y < cell(12, 5).y, turned_at)
    })
}

#[test]
fn a_turn_pressed_early_or_late_is_still_taken() {
    let on = Tuning::DEFAULT;
    let no_assist = Tuning { lane_assist_px: 0.0, ..Tuning::DEFAULT };
    let neither = Tuning { turn_buffer_px: 0.0, ..no_assist.clone() };
    assert!(takes_the_lane(&on, 0.0).0, "pressed on the line");
    // Early: the hull keeps its heading until the lane opens
    // (`turn_buffer_px`), where without the buffer it comes round on the
    // press and meets the wall.
    let (taken, turned_at) = takes_the_lane(&no_assist, EARLY);
    assert!(taken, "pressed {EARLY} px early");
    assert!(turned_at > EARLY + 2.0, "the turn was held: the hull came round at {turned_at} px");
    let (taken, turned_at) = takes_the_lane(&neither, EARLY);
    assert!(!taken, "with no buffer an early turn meets the wall");
    assert!(turned_at <= EARLY + 4.0, "and it came round on the press, at {turned_at} px");
    // Late: the tracks bring the hull back to the lane (`lane_assist_px`).
    assert!(takes_the_lane(&on, LATE).0, "pressed {LATE} px late");
    assert!(!takes_the_lane(&no_assist, LATE).0, "with no assist a late turn slides past");
    // The two together reach further either way than either alone.
    assert!(takes_the_lane(&on, EARLY - 4.0).0 && !takes_the_lane(&no_assist, EARLY - 4.0).0);
}

/// How early and how late (px from the lane's centre line) the turns
/// above are pressed.
const EARLY: f32 = -20.0;
const LATE: f32 = 8.0;

/// An assault hull driven north from open ground at a lane one cell wide
/// (column 12, rows 1..=5), `off` px east of the lane's centre line: how
/// far north of the lane's mouth it is two seconds on (negative: short).
fn enters_the_mouth(table: &Tuning, off: f32) -> f32 {
    under(table.clone(), || {
        let mut game = round(&iron((1..=5).flat_map(|r| [(11, r), (13, r)])), ASSAULT);
        let player = game.player().expect("player");
        game.place_tank(player, cell(12, 9) + Position::new(off, 0.0), Some(Dir::Up.rotation())).unwrap();
        drive(&mut game, Some(Dir::Up), 120);
        cell(12, 5).y + OBSTACLE_GRID_SIZE * 0.5 - player_pos(&game).y
    })
}

#[test]
fn a_hull_a_little_off_a_lanes_centre_is_guided_in() {
    let on = Tuning::DEFAULT;
    let bare = Tuning { lane_assist_px: 0.0, ..Tuning::DEFAULT };
    for off in [-8.0, 8.0] {
        let reached = enters_the_mouth(&on, off);
        assert!(reached > 64.0, "{off} px off the line: guided into the lane, {reached} px up it");
        let reached = enters_the_mouth(&bare, off);
        assert!(reached < 0.0, "{off} px off the line with no assist: stopped on the lane's corner, {reached} px");
    }
    // Further off than the assist reaches, the hull is left where it drives.
    let far = on.lane_assist_px + 12.0;
    assert!(enters_the_mouth(&on, far) < 0.0, "{far} px off the line is not the lane the player meant");
}

/// A `row` chassis on open ground driven east for two seconds and let go:
/// the steps it took to reach 95% of its top speed, and the steps and px
/// it took to stop.
fn sprint(table: &Tuning, row: i32) -> (usize, usize, f32) {
    under(table.clone(), || {
        let mut game = round("", row);
        let player = game.player().expect("player");
        game.place_tank(player, cell(5, 10), Some(Dir::Right.rotation())).unwrap();
        let mut speeds = vec![];
        for _ in 0..120 {
            drive(&mut game, Some(Dir::Right), 1);
            speeds.push(body_speed(&game, player));
        }
        let top = *speeds.last().expect("steps");
        let gathered = speeds.iter().position(|&s| s >= top * 0.95).expect("reaches its speed");
        let let_go = player_pos(&game);
        let mut stopped = 0;
        while body_speed(&game, player) > 0.0 && stopped < 240 {
            drive(&mut game, None, 1);
            stopped += 1;
        }
        (gathered, stopped, (player_pos(&game) - let_go).length())
    })
}

#[test]
fn a_heavy_chassis_gathers_speed_and_stops_slower_than_a_light_one() {
    let table = Tuning::DEFAULT;
    let (scout, assault, titan) = (sprint(&table, SCOUT), sprint(&table, ASSAULT), sprint(&table, TITAN));
    assert!(scout.0 < assault.0 && assault.0 < titan.0, "steps to top speed: scout {}, assault {}, titan {}", scout.0, assault.0, titan.0);
    assert!(scout.1 < assault.1 && assault.1 < titan.1, "steps to stop: scout {}, assault {}, titan {}", scout.1, assault.1, titan.1);
    // The standard chassis is the unit: weight leaves it exactly as it was.
    let flat = Tuning { tank_drive_mass_exponent: 0.0, tank_brake_mass_exponent: 0.0, ..Tuning::DEFAULT };
    assert_eq!(sprint(&flat, ASSAULT), assault);
    // Weight narrows what mass alone does: with one force for every
    // chassis a scout is quicker still and a titan slower.
    let (light, heavy) = (sprint(&flat, SCOUT), sprint(&flat, TITAN));
    assert!(light.0 < scout.0 && heavy.0 > titan.0, "steps to top speed with no weight in the drive: scout {}, titan {}", light.0, heavy.0);
    assert!(light.1 < scout.1 && heavy.1 > titan.1, "steps to stop with no weight in the brake: scout {}, titan {}", light.1, heavy.1);
}

/// Seat 0, a `pusher` chassis, driven for two seconds into seat 1, a
/// `target` chassis facing north that nobody drives - from the south,
/// along its tracks, or from the west, across them - parked or wrecked
/// first: how far (px) the target went in the second second.
fn pushed(table: &Tuning, pusher: i32, target: i32, across: bool, wreck: bool) -> f32 {
    under(table.clone(), || {
        let mut game = round_of(2, "", pusher, target);
        let (a, b) = (game.seats[0].expect("seat 0"), game.seats[1].expect("seat 1"));
        let at = cell(10, 16);
        game.place_tank(b, at, Some(0.0)).unwrap();
        if wreck {
            game.debug_kill(1).unwrap();
        }
        let (from, dir) = if across { (Position::new(at.x - 70.0, at.y), Dir::Right) } else { (Position::new(at.x, at.y + 70.0), Dir::Up) };
        game.place_tank(a, from, Some(dir.rotation())).unwrap();
        let mut marks = vec![];
        for f in 0..120 {
            // A ram wears both hulls: mended, so the push is all that is read.
            for seat in [a, b] {
                with_tank_mut(&game.world, seat, |t| {
                    if !t.is_wreck() {
                        t.damage = 0.0;
                    }
                });
            }
            drive(&mut game, Some(dir), 1);
            if f == 59 || f == 119 {
                marks.push(with_tank(&game.world, b, |t| t.position));
            }
        }
        (marks[1] - marks[0]).length()
    })
}

#[test]
fn a_heavy_hull_shoves_a_light_one_and_a_parked_hull_holds_its_ground() {
    let on = Tuning::DEFAULT;
    for across in [false, true] {
        let (heavy, even, light) = (pushed(&on, TITAN, SCOUT, across, false), pushed(&on, ASSAULT, ASSAULT, across, false), pushed(&on, SCOUT, TITAN, across, false));
        assert!(heavy > 2.0 * even && even > 1.5 * light, "across {across}: a titan shoves a scout {heavy} px, an assault its like {even} px, a scout a titan {light} px");
        assert!(even < 30.0 && light < 12.0, "across {across}: a parked hull only creeps before its like ({even} px) and hardly moves before a lighter one ({light} px)");
    }
    // With no hold a parked hull is pushed across its tracks at the pusher's own pace.
    let loose = Tuning { tank_park_hold_force: 0.0, ..Tuning::DEFAULT };
    assert!(pushed(&loose, ASSAULT, ASSAULT, true, false) > 150.0);
    // With no weight in the drive a scout pushes a parked titan along as fast as a titan does.
    let flat = Tuning { tank_drive_mass_exponent: 0.0, tank_brake_mass_exponent: 0.0, tank_park_hold_force: 0.0, ..Tuning::DEFAULT };
    let (by_scout, by_titan) = (pushed(&flat, SCOUT, TITAN, false, false), pushed(&flat, TITAN, TITAN, false, false));
    assert!((by_scout - by_titan).abs() < 2.0, "one drive force for all: {by_scout} px against {by_titan} px");
}

#[test]
fn a_wreck_resists_a_push_in_proportion_to_its_mass() {
    let on = Tuning::DEFAULT;
    for across in [false, true] {
        let (scout, assault, titan) = (pushed(&on, ASSAULT, SCOUT, across, true), pushed(&on, ASSAULT, ASSAULT, across, true), pushed(&on, ASSAULT, TITAN, across, true));
        assert!(scout > 1.2 * assault && assault > 2.0 * titan, "across {across}: an assault hull shoves a scout's wreck {scout} px, its like's {assault} px, a titan's {titan} px");
        assert!(assault < 100.0, "across {across}: a wreck is dead weight, shoved {assault} px in a second by its like");
    }
    // With no damping a wreck is pushed at the pusher's own pace.
    let loose = Tuning { wreck_linear_damping: 0.0, ..Tuning::DEFAULT };
    assert!(pushed(&loose, ASSAULT, ASSAULT, true, true) > 150.0);
}

#[test]
fn a_bong_is_not_left_overlapping_a_hull() {
    let mut game = round_of(2, "", ASSAULT, TITAN);
    let hull = game.seats[1].expect("seat 1");
    let bong = cell(12, 10);
    let away = Vec2::new(1.0, 0.0);
    let hop = |game: &Game| {
        let terrain = Terrain::build(&game.world, W, H, &[], &game.water);
        let mut rng = RoundRng::seed_from_u64(3);
        (bong_hop_target(&mut rng, bong, away, 96.0, &terrain, W, H).expect("a landing"), terrain)
    };
    // Where the hop lands with nothing in its way.
    game.place_tank(hull, cell(30, 3), Some(0.0)).unwrap();
    let (open, _) = hop(&game);
    // A hull parked on that spot: the hop lands clear of it. A wreck too.
    game.place_tank(hull, open, Some(0.0)).unwrap();
    for wrecked in [false, true] {
        if wrecked {
            game.debug_kill(1).unwrap();
            step(&mut game, Input::default());
        }
        let at = with_tank(&game.world, hull, |t| t.position);
        let (hx, hy) = with_tank(&game.world, hull, |t| t.move_half_extents(t.facing_along_x()));
        let (landing, terrain) = hop(&game);
        let clear = (landing.x - at.x).abs() >= hx + BONG_COLLIDER_HALF_EXTENT.0 || (landing.y - at.y).abs() >= hy + BONG_COLLIDER_HALF_EXTENT.1;
        assert!(clear, "wrecked {wrecked}: landed at {landing:?} on the hull at {at:?}");
        assert!(!terrain.bong_fits(at), "wrecked {wrecked}: the hull's own spot is no landing");
        // A well's pull: up to the hull and no further, and free to leave.
        let beside = Position::new(at.x - hx - BONG_COLLIDER_HALF_EXTENT.0 - 6.0, at.y);
        let into = beside + Vec2::new(4.0, 0.0);
        assert!(terrain.bong_slides(beside + Vec2::new(-4.0, 0.0), beside), "wrecked {wrecked}: slid up to the hull");
        assert!(!terrain.bong_slides(beside, into), "wrecked {wrecked}: never into it");
        assert!(terrain.bong_slides(into, beside), "wrecked {wrecked}: and out again, if it is ever there");
    }
}

/// A line of sandbags along row 8, columns 10..=14.
fn sandbags() -> String {
    (10..=14).map(|c| format!("cells.\"{c},8\" = {{ kind = \"sandbag\" }}\n")).collect()
}

fn sandbags_standing(game: &Game) -> usize {
    game.world.query::<&Obstacle>().iter().filter(|o| o.material == Material::Sandbag && !o.destroyed).count()
}

fn longest_ram(game: &Game) -> f32 {
    game.world.query::<&Obstacle>().iter().map(|o| o.ram_timer).fold(0.0, f32::max)
}

#[test]
fn the_prop_ram_reads_the_bodys_velocity_toward_the_tile() {
    // Where a hull's flank rests against the sandbags' north face: found
    // by driving one into them.
    let mut game = round(&sandbags(), ASSAULT);
    let player = game.player().expect("player");
    game.place_tank(player, cell(12, 6), Some(Dir::Down.rotation())).unwrap();
    // A sandbag cushions the last few px (`soft_cushion_decel`), so the
    // hull settles against it a few steps later than against a wall.
    drive(&mut game, Some(Dir::Down), 24);
    assert!(longest_ram(&game) > 0.0, "driven into, a sandbag's clock runs");
    let (hx, hy) = with_tank(&game.world, player, |t| t.move_half_extents(false));
    let face = player_pos(&game).y + hy;
    drive(&mut game, Some(Dir::Down), 30);
    assert!(sandbags_standing(&game) < 5, "and it goes over");

    // Grazing: driven east along the face, flank against it. The body goes
    // along the sandbags, not into them, and no clock runs up (a step's
    // worth where the hull's corner catches a sandbag's edge, and it
    // decays) - where every touch counts (`prop_ram_min_speed` 0) the same
    // drive wears them.
    let graze = |table: Tuning| {
        under(table, || {
            let mut game = round(&sandbags(), ASSAULT);
            let player = game.player().expect("player");
            game.place_tank(player, Position::new(cell(10, 8).x, face - hx + 0.3), Some(Dir::Right.rotation())).unwrap();
            // The first steps settle the hull against the face.
            drive(&mut game, Some(Dir::Right), 4);
            let mut longest = 0.0_f32;
            for _ in 0..30 {
                drive(&mut game, Some(Dir::Right), 1);
                longest = longest.max(longest_ram(&game));
            }
            (longest, sandbags_standing(&game))
        })
    };
    let (longest, standing) = graze(Tuning::DEFAULT);
    assert!(longest < 0.05 && standing == 5, "a hull grazing along a sandbag does not ram it: {longest} s, {standing} standing");
    let (longest, standing) = graze(Tuning { prop_ram_min_speed: 0.0, ..Tuning::DEFAULT });
    assert!(longest > 0.08 || standing < 5,"where every touch counts the same graze wears them: {longest} s, {standing} standing");

    // Skidding: no drive commanded, the body thrown south into the line.
    let mut game = round(&sandbags(), ASSAULT);
    let player = game.player().expect("player");
    game.place_tank(player, cell(12, 6), Some(Dir::Down.rotation())).unwrap();
    let body = with_tank(&game.world, player, |t| t.body).expect("a body");
    for _ in 0..60 {
        game.physics.set_velocity(body, Position::new(0.0, 150.0));
        step(&mut game, Input::default());
        assert_eq!(with_tank(&game.world, player, |t| t.velocity), Vec2::zero(), "nothing is commanded");
    }
    assert!(sandbags_standing(&game) < 5, "a hull thrown into a sandbag takes it down with no drive commanded");
}

#[test]
fn a_seats_pose_may_stand_a_turn_fit_further_on_the_tick_it_turns() {
    let mut game = round("", ASSAULT);
    let player = game.player().expect("player");
    let at = cell(10, 10);
    game.place_tank(player, at, Some(Dir::Up.rotation())).unwrap();
    let top = with_tank(&game.world, player, |t| t.effective_speed());
    let own = top * PHYSICS_FIXED_DT * POSE_REACH_TICKS + POSE_REACH_SLACK_PX;
    let fit = Tuning::DEFAULT.tank_turn_fit_px;
    let pose = |by: Vec2, facing: Dir| SeatPose { position: at + by, rotation: facing.rotation(), velocity: Vec2::zero() };
    // Still facing along y: the chassis's own reach and no further.
    assert!(game.accept_seat_pose(0, pose(Vec2::new(own + 1.0, 0.0), Dir::Up), 1).is_err(), "no turn, no fit");
    // Turned across: the fit's reach on top, each way.
    assert_eq!(game.accept_seat_pose(0, pose(Vec2::new(own + fit - 1.0, 0.0), Dir::Right), 1), Ok(()));
    game.place_tank(player, at, Some(Dir::Up.rotation())).unwrap();
    assert!(game.accept_seat_pose(0, pose(Vec2::new(own + fit * 1.5, 0.0), Dir::Right), 1).is_err(), "and no further than the fit");
}
