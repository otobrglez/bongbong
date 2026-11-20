//! The enemies (docs/november-26-rebuild-upgrade.md stage 13): straight
//! routes on an arena, a wander that arrives, a led shot, a yield that
//! routes round, the per-weapon fire profiles, a flee with an exit and
//! crate seeking that asks whether the crate can be reached.

use super::*;
use crate::tuning::Tuning;

/// `f` with `table` in force on this thread.
fn under<R>(table: Tuning, f: impl FnOnce() -> R) -> R {
    crate::tuning::in_force(Some(&std::sync::Arc::new(table)), f)
}

fn snapshot_of(game: &Game, slot: usize) -> TankSnapshot {
    game.tank_snapshots().into_iter().find(|t| t.slot == slot).expect("tank in slot")
}

/// The cardinal heading a commanded velocity names, `None` at a stand.
fn heading_of(v: Position) -> Option<Dir> {
    if v.x.abs() < 1.0 && v.y.abs() < 1.0 {
        None
    } else if v.x.abs() > v.y.abs() {
        Some(if v.x > 0.0 { Dir::Right } else { Dir::Left })
    } else {
        Some(if v.y > 0.0 { Dir::Down } else { Dir::Up })
    }
}

/// An open arena with a scatter of iron posts between the corners: a
/// route across it has many equally cheap shapes.
fn posts_map() -> String {
    let mut map = String::from("version = 1\ntanks = 1\ncells.\"36,19\" = { kind = \"start\" }\n");
    for (c, r) in [(8, 5), (14, 9), (20, 6), (20, 13), (26, 10), (30, 15), (12, 14)] {
        map.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
    }
    map
}

/// How many times the enemy in slot 1 changed its commanded heading on
/// its way from the bottom-left of `posts_map` to attack range of the
/// seat, which drives up the right side from the bottom-right corner so
/// the diagonal to it keeps changing, under `table`.
fn turns_across(table: Tuning) -> usize {
    under(table, || {
        let mut game = game_on(&posts_map(), 1, Some(0));
        set_role_of(&mut game, 1, Role::Player);
        game.debug_teleport(1, Position::new(112.0, 624.0), Some(0.0)).expect("slot 1");
        let (mut turns, mut held) = (0, None);
        for _ in 0..(14 * 60) {
            drive(&mut game, Some(Dir::Up), 1);
            let me = snapshot_of(&game, 1);
            if me.position.distance_to(player_pos(&game)) <= tuning().enemy_attack_range {
                return turns;
            }
            if let Some(dir) = heading_of(me.commanded_velocity) {
                if held.is_some_and(|h| h != dir) {
                    turns += 1;
                }
                held = Some(dir);
            }
        }
        panic!("never reached attack range of the seat");
    })
}

#[test]
fn an_arena_enemy_crosses_a_diagonal_in_legs_not_a_staircase() {
    // It sees the seat from the far corner.
    let far_sighted = Tuning { enemy_view_range: 2000.0, ..Tuning::DEFAULT };
    let straight = turns_across(far_sighted.clone());
    let stepped = turns_across(Tuning { ai_arena_straight_routes: false, ..far_sighted });
    assert!(straight <= 2, "{straight} turns on the way across");
    assert!(stepped >= straight + 3, "straight {straight}, without it {stepped}");
}

/// The enemy in slot 1's wander waypoint.
fn waypoint_of(game: &Game) -> Position {
    let snap = game.debug_snapshot(W, H, crate::simulation::debug::Detail::Full);
    let ai = snap.tanks.iter().find(|t| t.slot == 1).and_then(|t| t.ai.as_ref()).expect("an enemy in slot 1");
    Position::new(ai.waypoint_x, ai.waypoint_y)
}

/// A lone patrol on an open field for two minutes, the seat out of its sight in
/// a corner, under `table`: how many times it took a new waypoint while
/// still more than three cells from the one it had, and how many times
/// its new waypoint lay behind the heading it held.
fn wander_legs(table: Tuning) -> (usize, usize) {
    // Slow, so a leg outlasts the patrol's clock; and never called to the
    // seat (`stragglers`), so it patrols the whole two minutes.
    under(Tuning { enemy_view_range: 100.0, enemy_speed: 60.0, enemy_speed_variance: 0.0, straggler_call_alive: 0, ..table }, || {
        let mut game = game_on("version = 1\ntanks = 1\ncells.\"1,1\" = { kind = \"start\" }\n", 1, Some(0));
        set_role_of(&mut game, 1, Role::Player);
        game.debug_teleport(1, Position::new(W / 2.0, H / 2.0), Some(0.0)).expect("slot 1");
        step(&mut game, Input::default());
        let (mut cut_short, mut doubled_back, mut waypoint) = (0, 0, waypoint_of(&game));
        for _ in 0..(120 * 60) {
            let before = snapshot_of(&game, 1);
            step(&mut game, Input::default());
            if snapshot_of(&game, 1).action.0 != Some("patrol") {
                continue;
            }
            let next = waypoint_of(&game);
            if next != waypoint {
                cut_short += usize::from(before.position.distance_to(waypoint) > 3.0 * OBSTACLE_GRID_SIZE);
                if let Some(dir) = heading_of(before.commanded_velocity) {
                    let to = next - before.position;
                    doubled_back += usize::from(to.x * dir.vec().x + to.y * dir.vec().y < 0.0);
                }
                waypoint = next;
            }
        }
        (cut_short, doubled_back)
    })
}

#[test]
fn a_patrol_drives_to_its_waypoint_and_carries_on_ahead() {
    let (cut_short, doubled_back) = wander_legs(Tuning::DEFAULT);
    let (timer_cut, timer_back) = wander_legs(Tuning { enemy_wander_patience_seconds: 0.0, enemy_wander_ahead_px: 0.0, ..Tuning::DEFAULT });
    // Its patience can run out on the longest legs at this speed.
    assert!(cut_short <= 3, "it gave up {cut_short} waypoints mid-leg");
    assert!(timer_cut >= 4 * cut_short.max(3), "on the clock alone it turned away mid-leg {timer_cut} times");
    assert!(doubled_back < timer_back, "{doubled_back} waypoints behind it, {timer_back} on distance alone");
}

/// A titan (the super tier) that cannot drive, a little over 200 px
/// below the lane a seat drives back and forth across for 40 s, under
/// `table`: how many of its shells hit the seat.
fn hits_on_a_crossing_seat(table: Tuning) -> usize {
    under(table, || {
        let mut game = game_on("version = 1\ntanks = 0\ncells.\"8,6\" = { kind = \"start\" }\n", 0, Some(0));
        step(&mut game, Input::default());
        let slot = game.debug_spawn_enemy(Position::new(W / 2.0, 400.0), Some(10), Some(Role::Player)).expect("spawns");
        let gunner = game.tank_entity_by_slot(slot).expect("exists");
        with_tank_mut(&game.world, gunner, |t| t.speed_scale = 0.0);
        let (mut hits, mut dir) = (0, Dir::Right);
        for _ in 0..(40 * 60) {
            let x = player_pos(&game).x;
            if x > W / 2.0 + 260.0 {
                dir = Dir::Left;
            } else if x < W / 2.0 - 260.0 {
                dir = Dir::Right;
            }
            // Unhurt, so the round goes on.
            let seat = game.player().expect("a seat");
            with_tank_mut(&game.world, seat, |t| t.damage = 0.0);
            drive(&mut game, Some(dir), 1);
            hits += game.events().iter().filter(|e| matches!(e, Event::Hit { target: HitTarget::Player { .. }, .. })).count();
        }
        hits
    })
}

#[test]
fn a_heavy_tier_leads_a_seat_driving_across_its_line() {
    let led = hits_on_a_crossing_seat(Tuning::DEFAULT);
    let unled = hits_on_a_crossing_seat(Tuning { enemy_lead_by_tier: [0.0; 4], ..Tuning::DEFAULT });
    assert!(led >= 3, "{led} shells found the seat");
    assert!(led >= unled + 3, "led {led}, aimed where it stood {unled}");
}

fn iron(cells: impl IntoIterator<Item = (i32, i32)>) -> String {
    cells.into_iter().map(|(c, r)| format!("cells.\"{c},{r}\" = {{ kind = \"wall\", material = \"iron\" }}\n")).collect()
}

/// What the enemy in slot 2 did over ten seconds behind the one in slot 1:
/// the frames it spent driving into that hull's back and the frames it
/// spent queued behind it.
struct Behind {
    pushing: usize,
    queued: usize,
}

/// A lane along rows 8 and 9 - one nav cell wide, the two grids standing
/// half a cell apart - closed at its west end. A seat at that end; an
/// assault 10 cells east of it on the row, lined up and just in range, so
/// it parks there and fires; a second one 6 cells behind that, out of
/// shells, backing off toward the ammo crate that lies between the first
/// and the seat. Under `table`.
fn behind_a_parked_ally(table: Tuning) -> Behind {
    let rows = 2;
    under(table, || {
        let walls = iron((4..=30).flat_map(|c| [(c, 7), (c, 8 + rows)]).chain((8..8 + rows).map(|r| (4, r))));
        let map = format!("version = 1\ntanks = 0\ncells.\"6,8\" = {{ kind = \"start\" }}\ncells.\"11,8\" = {{ kind = \"pickup\", pickup = \"ammo\" }}\n{walls}");
        let mut game = game_on(&map, 0, Some(0));
        step(&mut game, Input::default());
        for col in [16, 22] {
            game.debug_spawn_enemy(crate::map::cell_to_world(col, 8), Some(1), Some(Role::Player)).expect("spawns");
        }
        game.debug_set_tank(2, &crate::simulation::debug::TankPatch { shells_ammo: Some(0), ..Default::default() }).expect("slot 2");
        let mut seen = Behind { pushing: 0, queued: 0 };
        for _ in 0..(10 * 60) {
            // Unhurt, so the round goes on.
            let player = game.player().expect("a seat");
            with_tank_mut(&game.world, player, |t| t.damage = 0.0);
            step(&mut game, Input::default());
            let (front, back) = (snapshot_of(&game, 1), snapshot_of(&game, 2));
            // Nose to tail on the row, not passing alongside.
            let (dx, dy) = ((front.position.x - back.position.x).abs(), (front.position.y - back.position.y).abs());
            let touching = dy < OBSTACLE_GRID_SIZE * 0.5 && dx < OBSTACLE_GRID_SIZE * 1.75;
            seen.pushing += usize::from(touching && heading_of(back.commanded_velocity).is_some());
            seen.queued += usize::from(back.queued);
        }
        seen
    })
}

/// The yield as it was: the hold over, it drives on into the hull.
fn pushing_through() -> Tuning {
    Tuning { enemy_yield_reroute_seconds: 0.0, ..Tuning::DEFAULT }
}

#[test]
fn an_enemy_with_no_way_round_a_parked_ally_waits_behind_it() {
    let waiting = behind_a_parked_ally(Tuning::DEFAULT);
    assert!(waiting.pushing < 30, "it drove into the parked hull for {} frames", waiting.pushing);
    assert!(waiting.queued > 150, "queued for {} frames of 600", waiting.queued);
    let through = behind_a_parked_ally(pushing_through());
    assert_eq!(through.queued, 0);
    assert!(through.pushing > 90, "pushing through it drove into the hull for {} frames", through.pushing);
}

/// The frames, of `frames`, on which a disarmed enemy headed for a laser
/// crate ringed by iron - no route reaches it - with nothing to fight in
/// its sight. Under `table`.
fn frames_heading_for_a_walled_crate(table: Tuning, frames: usize) -> Vec<usize> {
    under(table, || {
        let mut map = String::from(
            "version = 1\ntanks = 1\ncells.\"38,20\" = { kind = \"start\" }\ncells.\"10,10\" = { kind = \"pickup\", pickup = \"laser\" }\ncells.\"30,2\" = { kind = \"bong\" }\n",
        );
        map.push_str(&iron((8..=12).flat_map(|c| (8..=12).map(move |r| (c, r))).filter(|&(c, r)| c == 8 || c == 12 || r == 8 || r == 12)));
        let mut game = game_on(&map, 1, None);
        let (slot, _) = enemy_actions(&game)[0];
        let entity = game.tank_entity_by_slot(slot).expect("the enemy exists");
        with_tank_mut(&game.world, entity, |t| t.disarm());
        game.debug_teleport(slot, crate::map::cell_to_world(4, 4), None).expect("the enemy exists");
        let mut sought = Vec::new();
        for frame in 0..frames {
            step(&mut game, Input::default());
            if enemy_actions(&game)[0].1 == Some("seek_laser") {
                sought.push(frame);
            }
        }
        assert_eq!(with_tank(&game.world, entity, |t| t.active_weapon()), ActiveWeapon::Shell, "it never got the crate");
        sought
    })
}

/// A crate an enemy has no way to is none to it: it never sets off for
/// one ringed by iron. With the rule off it heads for the crate, gives it
/// up after `enemy_crate_give_up_seconds` without a route step of headway
/// and passes it over until `enemy_crate_forget_seconds` are out (BB-98).
#[test]
fn an_enemy_heads_only_for_a_crate_it_has_a_way_to() {
    let patience = (tuning().enemy_crate_give_up_seconds * 60.0) as usize;
    let forget = (tuning().enemy_crate_forget_seconds * 60.0) as usize;
    let frames = patience + forget + 120;
    assert_eq!(frames_heading_for_a_walled_crate(Tuning::DEFAULT, frames), Vec::<usize>::new());

    // Short-sighted, so the seat never draws it off the crate.
    let blind = Tuning { enemy_crate_reachable_only: false, enemy_view_range: 200.0, ..Tuning::DEFAULT };
    let sought = frames_heading_for_a_walled_crate(blind, frames);
    assert_eq!(sought.first(), Some(&0), "it heads for the crate at once");
    let gave_up = sought.iter().position(|&f| f > patience + 10).map_or(sought.len(), |i| i);
    assert!(sought[..gave_up].windows(2).all(|w| w[1] == w[0] + 1), "it heads for the crate without a break until it gives up: {sought:?}");
    assert!(sought[gave_up - 1] >= patience - 2, "and does not give up before its patience is out: {sought:?}");
    let back = sought.get(gave_up).copied().expect("forgotten, it heads for the crate again");
    assert!(back >= patience + forget - 2, "it passes the crate over while it remembers giving up: back on frame {back}");
}
