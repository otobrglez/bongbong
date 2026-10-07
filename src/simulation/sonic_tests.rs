//! Headless scenario tests for the sonic hammer (docs/sonic-hammer.md
//! "Tests"): one per promised rule, on the default 34 x 17 field with the
//! seat at cell (3, 6) facing east and parked enemies placed by hand. Tests
//! cannot touch the global tuning table, so every number is read off the
//! defaults.

use super::*;
use crate::ai::Intent;
use crate::sonic::falloff;
use crate::ai::Role;
use crate::tuning::Tuning;

const W: f32 = 1088.0;
const H: f32 = 544.0;
const DT: f32 = 1.0 / 60.0;

/// The seat's pivot: cell (3, 6).
const SEAT: Position = Position::new(96.0, 192.0);

/// A round on an open field plus `extra` cells, `players` seats (the second
/// at cell (3, 12)), nobody shielded, no banner.
fn round_with(extra: &str, players: usize, mission: Mission) -> Game {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.show_intro = false;
    game.level_overrides.mission = Some(mission);
    game.players = PlayerCount::from_count(players).expect("a seat count");
    let map = format!("version = 1\ntanks = 0\ncells.\"3,6\" = {{ kind = \"start\" }}\ncells.\"3,12\" = {{ kind = \"start2\" }}\n{extra}");
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
        t.sonic_ammo = tuning().sonic_ammo_per_pickup;
    });
    game
}

fn round(extra: &str) -> Game {
    round_with(extra, 1, Mission::Destroy)
}

/// A parked enemy of chassis `row` at `at`: it cannot drive or shoot.
fn parked_row(game: &mut Game, at: Position, row: i32) -> Entity {
    let slot = game.debug_spawn_enemy(at, Some(row), None).expect("spawns");
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

fn step(game: &mut Game, fire: bool) -> Vec<Event> {
    let mut input = Input::default();
    input.seats[0].fire = fire;
    game.update(input, DT, W, H);
    game.events().to_vec()
}

/// Press the trigger for one frame, then `frames` idle ones; every event.
fn blast(game: &mut Game, frames: usize) -> Vec<Event> {
    let mut seen = step(game, true);
    for _ in 0..frames {
        seen.extend(step(game, false));
    }
    seen
}

fn pos(game: &Game, entity: Entity) -> Position {
    with_tank(&game.world, entity, |t| t.position)
}

fn damage(game: &Game, entity: Entity) -> f32 {
    with_tank(&game.world, entity, |t| t.damage)
}

fn moved(game: &Game, entity: Entity, from: Position) -> f32 {
    pos(game, entity).distance_to(from)
}

#[test]
fn a_sonic_crate_arms_the_hammer_and_replaces_the_special_carried() {
    let t = tuning();
    let mut tank = Tank::default();
    tank.take_weapon(ActiveWeapon::Minigun);
    tank.take_weapon(ActiveWeapon::SonicHammer);
    assert_eq!((tank.minigun_ammo, tank.sonic_ammo), (0, t.sonic_ammo_per_pickup));
    assert_eq!(tank.active_weapon(), ActiveWeapon::SonicHammer);
    assert_eq!(tank.special(), Some(ActiveWeapon::SonicHammer));
    tank.sonic_ammo = 2;
    tank.take_weapon(ActiveWeapon::SonicHammer);
    assert_eq!(tank.sonic_ammo, t.sonic_ammo_per_pickup, "a second crate refills to one crate's worth");
    let mut enemy = Tank { owner: Owner::Enemy(3), ..Tank::default() };
    assert!(enemy.wants_pickup(PickupKind::SonicHammer), "an enemy on shells takes one");
    enemy.take_weapon(ActiveWeapon::Laser);
    assert!(!enemy.wants_pickup(PickupKind::SonicHammer), "and one carrying a special does not");
}

#[test]
fn the_hammer_fires_on_the_press_and_spends_one_blast() {
    let mut game = round("");
    let events = step(&mut game, true);
    let fired = events.iter().position(|e| matches!(e, Event::Fired { slot: 0, weapon: "sonic_hammer" }));
    let blasted = events.iter().position(|e| matches!(e, Event::SonicBlast { slot: 0, dir: "right", .. }));
    assert!(fired.is_some() && blasted.is_some() && fired < blasted, "Fired then SonicBlast: {events:?}");
    let seat = game.player().unwrap();
    let left = |game: &Game| with_tank(&game.world, seat, |t| t.sonic_ammo);
    assert_eq!(left(&game), tuning().sonic_ammo_per_pickup - 1);
    // Held, it fires no more.
    for _ in 0..120 {
        assert!(!step(&mut game, true).iter().any(|e| matches!(e, Event::SonicBlast { .. })));
    }
    assert_eq!(left(&game), tuning().sonic_ammo_per_pickup - 1);
    step(&mut game, false);
    assert!(step(&mut game, true).iter().any(|e| matches!(e, Event::SonicBlast { .. })), "a fresh press past the reload");
}

#[test]
fn the_cone_shoves_an_enemy_ahead_and_not_one_beside() {
    let mut game = round("");
    let (ahead_at, beside_at) = (Position::new(196.0, 192.0), Position::new(96.0, 300.0));
    let ahead = parked_row(&mut game, ahead_at, 1);
    let beside = parked_row(&mut game, beside_at, 1);
    blast(&mut game, 60);
    assert!(pos(&game, ahead).x > ahead_at.x + 30.0, "ahead is shoved away: {:?}", pos(&game, ahead));
    assert!(moved(&game, beside, beside_at) < 1.0, "beside is not");
}

#[test]
fn the_wave_reaches_far_things_later() {
    let mut game = round("");
    let near = parked_row(&mut game, Position::new(160.0, 192.0), 1);
    let far = parked_row(&mut game, Position::new(240.0, 192.0), 1);
    let (mut near_at, mut far_at) = (None, None);
    step(&mut game, true);
    for frame in 1..40 {
        if near_at.is_none() && with_tank(&game.world, near, |t| t.skid > 0.0) {
            near_at = Some(frame);
        }
        if far_at.is_none() && with_tank(&game.world, far, |t| t.skid > 0.0) {
            far_at = Some(frame);
        }
        step(&mut game, false);
    }
    let (near_at, far_at) = (near_at.expect("the near one is reached"), far_at.expect("the far one is reached"));
    assert!(far_at > near_at, "the front reaches the far one later: {near_at} vs {far_at}");
}

#[test]
fn the_shove_falls_off_with_distance_and_a_heavy_chassis_barely_slides() {
    let slide = |at: f32, row: i32| {
        let mut game = round("");
        let from = Position::new(at, 192.0);
        let e = parked_row(&mut game, from, row);
        blast(&mut game, 90);
        moved(&game, e, from)
    };
    let (near, far) = (slide(150.0, 1), slide(220.0, 1));
    assert!(near > far + 10.0, "near {near} vs far {far}");
    let (scout, titan) = (slide(170.0, 0), slide(170.0, 10));
    assert!(titan * 4.0 < scout, "a titan barely slides: titan {titan} vs scout {scout}");
}

#[test]
fn a_skidding_tank_cannot_drive_and_slides_as_far_whichever_way_it_faces() {
    let slide = |facing: f32, drive: bool| {
        let mut game = round_with("", 2, Mission::Destroy);
        let mate = game.seat(1).expect("seat 2");
        let from = Position::new(176.0, 192.0);
        game.place_tank(mate, from, Some(facing)).unwrap();
        let mut input = Input::default();
        input.seats[0].fire = true;
        game.update(input, DT, W, H);
        // Where the knock found it, and how far it went while skidding;
        // the stick is pushed from the knock on.
        let mut knocked: Option<Position> = None;
        let mut in_skid = Vec2::zero();
        for _ in 0..90 {
            let before = pos(&game, mate);
            let mut input = Input::default();
            input.seats[1].move_dir = (drive && knocked.is_some()).then_some(Dir::Up);
            game.update(input, DT, W, H);
            if with_tank(&game.world, mate, |t| t.skid > 0.0) {
                let at = *knocked.get_or_insert(before);
                in_skid = pos(&game, mate) - at;
            }
        }
        assert!(knocked.is_some());
        (in_skid, pos(&game, mate) - from)
    };
    let (head_on, _) = slide(90.0, false);
    let (broadside, _) = slide(0.0, false);
    assert!(head_on.x > 30.0, "{head_on:?}");
    assert!((head_on.x - broadside.x).abs() < 3.0, "head-on {head_on:?} vs broadside {broadside:?}");
    // Driving up through the skid: it slides on as if the stick were
    // idle, and drives off up only once the skid is over.
    let (driving, after) = slide(0.0, true);
    assert!((driving.x - broadside.x).abs() < 3.0 && driving.y.abs() < 1.0, "the stick does not steer a skid: {driving:?}");
    assert!(after.y < -20.0, "and drives once it is over: {after:?}");
}

#[test]
fn walls_and_towers_shadow_the_cone_and_glass_shatters_as_it_does() {
    for (wall, breaks) in [("{ kind = \"wall\", material = \"iron\" }", false), ("{ kind = \"wall\", material = \"brick\" }", false), ("{ kind = \"tesla\", side = \"enemy\" }", false), ("{ kind = \"wall\", material = \"glass\" }", true)] {
        let mut game = round(&format!("cells.\"5,6\" = {wall}\n"));
        let behind_at = Position::new(240.0, 192.0);
        let beside_at = Position::new(200.0, 140.0);
        let behind = parked_row(&mut game, behind_at, 1);
        let beside = parked_row(&mut game, beside_at, 1);
        let events = blast(&mut game, 60);
        assert!(moved(&game, behind, behind_at) < 1.0, "{wall}: what stands behind is shadowed");
        assert!(moved(&game, beside, beside_at) > 10.0, "{wall}: beside the shadow is reached");
        let gone = events.iter().any(|e| matches!(e, Event::ObstacleDestroyed { .. }));
        assert_eq!(gone, breaks, "{wall}: only glass breaks");
    }
}

#[test]
fn a_lantern_is_broken_and_a_lamp_post_left_standing() {
    let mut game = round("cells.\"5,7\" = { kind = \"lamp\" }\n");
    game.lanterns.push(crate::lamp::Lantern { id: 1, position: Position::new(170.0, 192.0), seat: 0, lit_at: 0.0 });
    let events = blast(&mut game, 40);
    assert!(events.iter().any(|e| matches!(e, Event::LanternBroken { .. })), "the lantern breaks");
    assert!(game.lanterns.is_empty());
    assert!(!events.iter().any(|e| matches!(e, Event::ObstacleDestroyed { .. })), "the lamp post stands");
}

#[test]
fn grass_the_wave_crosses_stops_concealing_for_a_while() {
    let mut game = round("cells.\"6,6\" = { kind = \"tall_grass\" }\ncells.\"7,6\" = { kind = \"tall_grass\" }\n");
    let cell = Position::new(192.0, 192.0);
    assert!(crate::grass::conceals(&game.cover_cells(), cell));
    blast(&mut game, 30);
    assert!(!crate::grass::conceals(&game.cover_cells(), cell), "flattened, it hides nobody");
    assert!(game.grass.iter().filter(|t| crate::map::world_to_cell(t.base) == (6, 6)).all(|t| t.crush >= 1.0), "and lies flat");
    for _ in 0..((tuning().sonic_grass_flat_seconds / DT) as usize + 10) {
        step(&mut game, false);
    }
    assert!(crate::grass::conceals(&game.cover_cells(), cell), "it hides again");
}

#[test]
fn a_drum_is_thrown_onto_the_tank_behind_it() {
    let mut game = round("cells.\"5,6\" = { kind = \"barrel\", drum = \"oil\" }\n");
    let enemy = parked_row(&mut game, Position::new(256.0, 192.0), 1);
    let events = blast(&mut game, 90);
    let launched = events.iter().find_map(|e| match *e {
        Event::DrumLaunched { to_x, to_y, drum, .. } => Some((Position::new(to_x, to_y), drum)),
        _ => None,
    });
    assert_eq!(launched, Some((Position::new(256.0, 192.0), crate::obstacle::Drum::Oil)), "{events:?}");
    assert!(events.iter().any(|e| matches!(e, Event::Blast { chained: true, .. })), "it goes off where it lands");
    assert!(damage(&game, enemy) > tuning().sonic_damage, "and hurts the tank it landed on");
}

#[test]
fn a_drum_with_no_tank_behind_it_is_thrown_along_the_wave() {
    let mut game = round("cells.\"5,6\" = { kind = \"barrel\", drum = \"fuel\" }\n");
    let events = blast(&mut game, 10);
    let to = events.iter().find_map(|e| match *e {
        Event::DrumLaunched { to_x, to_y, .. } => Some(Position::new(to_x, to_y)),
        _ => None,
    });
    let cells = (tuning().sonic_drum_throw_cells * falloff(&tuning(), 64.0)).round();
    assert_eq!(to, Some(Position::new(160.0 + cells * 32.0, 192.0)));
}

#[test]
fn a_frog_the_wave_reaches_is_stunned_and_neither_hops_nor_bites() {
    let mut game = round_with("cells.\"6,6\" = { kind = \"frog\" }\n", 1, Mission::Protect);
    let frog = game.frog.expect("a frog");
    blast(&mut game, 20);
    let (stunned, hop, bite, health) =
        with_frog(&game.world, frog, |f| (f.is_stunned(), f.can_hop(), f.can_attack(), f.health));
    assert!(stunned && !hop && !bite);
    assert_eq!(health, with_frog(&game.world, frog, |f| f.max_health), "no harm done");
    for _ in 0..((tuning().sonic_frog_stun_seconds / DT) as usize + 2) {
        step(&mut game, false);
    }
    assert!(!with_frog(&game.world, frog, |f| f.is_stunned()));
}

#[test]
fn the_hammer_hurts_only_the_other_side_and_shoves_everyone() {
    let mut game = round_with("", 2, Mission::Destroy);
    let mate = game.seat(1).unwrap();
    let mate_at = Position::new(196.0, 192.0);
    game.place_tank(mate, mate_at, Some(90.0)).unwrap();
    let enemy_at = Position::new(196.0, 236.0);
    let enemy = parked_row(&mut game, enemy_at, 1);
    let events = blast(&mut game, 60);
    assert_eq!(damage(&game, mate), 0.0, "a teammate is not hurt");
    assert!(moved(&game, mate, mate_at) > 10.0, "but is shoved");
    let hurt = damage(&game, enemy);
    assert!(hurt > 0.0 && hurt <= tuning().sonic_damage, "{hurt}");
    let hit = events.iter().find_map(|e| match *e {
        Event::Hit { target: HitTarget::Enemy { .. }, cause, .. } => Some(cause),
        _ => None,
    });
    assert_eq!(hit, Some(HitCause::Sonic), "a sonic hit says so");
}

#[test]
fn a_shield_soaks_the_damage_not_the_shove() {
    let mut game = round("");
    let at = Position::new(196.0, 192.0);
    let enemy = parked_row(&mut game, at, 1);
    with_tank_mut(&game.world, enemy, |t| t.raise_shield());
    let shield = with_tank(&game.world, enemy, |t| t.shield_hp);
    blast(&mut game, 60);
    assert_eq!(damage(&game, enemy), 0.0);
    assert!(with_tank(&game.world, enemy, |t| t.shield_hp) < shield, "the shield took it");
    assert!(moved(&game, enemy, at) > 10.0, "and the hull still slid");
}

#[test]
fn a_grenade_in_the_cone_is_pushed_away() {
    let mut game = round("");
    let mut g = crate::grenade::Grenade::launch(Position::new(170.0, 192.0), Vec2::new(0.0, -1.0), Vec2::zero(), Owner::Player(0));
    g.velocity = Vec2::zero();
    g.height = 0.0;
    g.climb = 0.0;
    game.world.spawn((g,));
    blast(&mut game, 8);
    let vx = game.world.query::<&crate::grenade::Grenade>().iter().next().expect("the grenade").velocity.x;
    assert!(vx > 50.0, "{vx}");
}

#[test]
fn a_wreck_is_left_alone() {
    let mut game = round("");
    let at = Position::new(196.0, 192.0);
    let enemy = parked_row(&mut game, at, 1);
    with_tank_mut(&game.world, enemy, |t| t.damage = crate::MAX_DAMAGE);
    step(&mut game, false);
    let at = pos(&game, enemy);
    blast(&mut game, 60);
    assert!(moved(&game, enemy, at) < 0.5);
}

/// The pose validator allows a client-owned hull a knock's speed while it
/// may still be skidding, and no longer.
#[test]
fn an_owned_hull_is_allowed_its_knock() {
    let mut game = round_with("", 2, Mission::Destroy);
    let mate = game.seat(1).unwrap();
    let from = Position::new(300.0, 300.0);
    game.place_tank(mate, from, Some(90.0)).unwrap();
    let pose = SeatPose { position: Position::new(from.x + 25.0, from.y), rotation: 90.0, velocity: Vec2::new(400.0, 0.0) };
    assert!(game.accept_seat_pose(1, pose, 4).is_err(), "past the chassis's reach");
    game.seat_knock[1] = (400.0, game.frame + 10);
    game.place_tank(mate, from, Some(90.0)).unwrap();
    assert!(game.accept_seat_pose(1, pose, 4).is_ok(), "within a knock's");
}

#[test]
fn a_round_with_the_hammer_replays_bit_for_bit() {
    let play = || {
        let mut game = round("cells.\"6,6\" = { kind = \"tall_grass\" }\ncells.\"9,5\" = { kind = \"barrel\", drum = \"fuel\" }\n");
        parked_row(&mut game, Position::new(220.0, 200.0), 2);
        parked_row(&mut game, Position::new(320.0, 170.0), 7);
        let mut trail = Vec::new();
        for frame in 0..400 {
            step(&mut game, frame % 90 == 0);
            trail.extend(game.tank_snapshots().iter().map(|t| (t.position.x.to_bits(), t.position.y.to_bits(), t.damage.to_bits())));
        }
        trail
    };
    assert_eq!(play(), play());
}

#[test]
fn the_spawn_swap_is_off_at_zero() {
    let at = Position::new(400.0, 200.0);
    let mut t = Tuning::DEFAULT;
    let mut laser = Tank::default();
    laser.take_weapon(ActiveWeapon::Laser);
    let mut shells = Tank::default();
    sonic::swap_spawn_special_with(&t, &mut laser, 5, at);
    sonic::swap_spawn_special_with(&t, &mut shells, 5, at);
    assert_eq!((laser.special(), shells.special()), (Some(ActiveWeapon::Laser), None), "at the default share nothing changes");
    t.enemy_special_weapon_sonic_share = 1.0;
    sonic::swap_spawn_special_with(&t, &mut laser, 5, at);
    sonic::swap_spawn_special_with(&t, &mut shells, 5, at);
    assert_eq!(laser.special(), Some(ActiveWeapon::SonicHammer), "a drawn special is swapped");
    assert_eq!(shells.special(), None, "a tank that drew none keeps none");
}

/// An enemy carrying the hammer winds up before it goes off: the tell
/// first, the blast `sonic_tell_seconds` later, the tank holding its aim
/// the whole time.
#[test]
fn an_enemys_tell_runs_before_its_blast() {
    let mut game = round("");
    let at = Position::new(196.0, 192.0);
    let enemy = parked_row(&mut game, at, 1);
    with_tank_mut(&game.world, enemy, |t| t.sonic_ammo = 3);
    let slot = with_tank(&game.world, enemy, |t| t.owner_slot());
    let (mut told, mut blasted) = (None, None);
    let mut facing_during = Vec::new();
    for frame in 0..300 {
        for e in step(&mut game, false) {
            match e {
                Event::TellStarted { slot: s, weapon: "sonic_hammer" } if s == slot && told.is_none() => told = Some(frame),
                Event::SonicBlast { slot: s, .. } if s == slot && blasted.is_none() => blasted = Some(frame),
                _ => {}
            }
        }
        if told.is_some() && blasted.is_none() {
            facing_during.push(with_tank(&game.world, enemy, |t| (t.rotation, t.tell.is_some())));
        }
    }
    let (told, blasted) = (told.expect("a tell"), blasted.expect("a blast"));
    let ticks = (tuning().sonic_tell_seconds / DT).round() as i32;
    assert!(((blasted - told) as i32 - ticks).abs() <= 1, "told {told}, blasted {blasted}");
    assert!(facing_during.iter().all(|&(r, _)| r == facing_during[0].0), "it holds its aim: {facing_during:?}");
    assert_eq!(facing_during[0].0, 270.0, "facing the seat");
}

#[test]
fn a_wrecked_tanks_tell_never_goes_off() {
    let mut game = round("");
    let enemy = parked_row(&mut game, Position::new(196.0, 192.0), 1);
    with_tank_mut(&game.world, enemy, |t| {
        t.sonic_ammo = 3;
        t.tell = Some(crate::tank::Tell { weapon: ActiveWeapon::SonicHammer, left: 0.2, total: 0.5, facing: Dir::Left });
        t.damage = crate::MAX_DAMAGE;
    });
    for _ in 0..60 {
        assert!(!step(&mut game, false).iter().any(|e| matches!(e, Event::SonicBlast { slot, .. } if *slot != 0)));
    }
    assert!(with_tank(&game.world, enemy, |t| t.tell.is_none()));
}

/// The command layer's order is never the last word on a tank in a tell:
/// its hold is applied after the commander's.
#[test]
fn a_tell_holds_under_c2() {
    let mut commander = command::Commander::default();
    commander.push_order(4, crate::simulation::comms::Order::Nudge { dir: Dir::Up });
    let tell = crate::tank::Tell { weapon: ActiveWeapon::SonicHammer, left: 0.3, total: 0.55, facing: Dir::Left };
    let intent = Intent { move_dir: Some(Dir::Right), fire: true, ..Intent::default() };
    let free = commanded_intent(&commander, 4, intent, None);
    assert_eq!(free.move_dir, Some(Dir::Up), "the commander's nudge reaches a tank with no tell");
    let held = commanded_intent(&commander, 4, intent, Some(tell));
    assert_eq!((held.move_dir, held.face, held.fire), (None, Some(Dir::Left), false));
}

/// A parked enemy of the medium chassis at `at`.
fn parked(game: &mut Game, at: Position) -> Entity {
    parked_row(game, at, 1)
}

/// A hammer-armed enemy of the player role at `at`, parked.
fn hammer_enemy(game: &mut Game, at: Position) -> Entity {
    let slot = game.debug_spawn_enemy(at, Some(1), Some(Role::Player)).expect("spawns");
    let entity = game.tank_entity_by_slot(slot).expect("exists");
    with_tank_mut(&game.world, entity, |t| {
        t.speed_scale = 0.0;
        t.disarm();
        t.shells_ammo = 0;
        t.sonic_ammo = 3;
        t.shield_hp = 0.0;
        t.shield_timer = 0.0;
    });
    entity
}

/// The arm the enemy's special rule took on its last think.
fn arm(game: &Game, entity: Entity) -> Option<&'static str> {
    game.world.get::<&Ai>(entity).expect("an enemy").snapshot().special
}

/// The rule reads the knock's slide: a seat the shove would carry into an
/// enemy tower's reach is shoved there ("trouble"); with no tower behind
/// it, the same seat in the tank's face is only broken off ("breaker").
#[test]
fn the_rule_reads_where_the_knock_would_slide_the_seat() {
    let seat_at = Position::new(300.0, 192.0);
    let reach = tuning().tesla_range;
    let tower = (((seat_at.x + reach + 40.0) / 32.0).round() as i32, 6);
    for (with_tower, want) in [(true, "trouble"), (false, "breaker")] {
        let extra = if with_tower { format!("cells.\"{},{}\" = {{ kind = \"tesla\", side = \"enemy\" }}\n", tower.0, tower.1) } else { String::new() };
        let mut game = round(&extra);
        let seat = game.player().unwrap();
        game.place_tank(seat, seat_at, Some(270.0)).unwrap();
        let enemy = hammer_enemy(&mut game, Position::new(seat_at.x - 90.0, 192.0));
        step(&mut game, false);
        step(&mut game, false);
        assert_eq!(arm(&game, enemy), Some(want), "tower {with_tower}");
    }
}

/// Of two hammer tanks fighting the same seat, only the nearer closes in.
#[test]
fn only_the_nearest_hammer_tank_brawls() {
    let mut game = round("");
    let near = hammer_enemy(&mut game, Position::new(96.0, 192.0 + 200.0));
    let far = hammer_enemy(&mut game, Position::new(96.0 + 260.0, 192.0));
    let (mut near_approached, mut far_approached) = (false, false);
    for _ in 0..30 {
        step(&mut game, false);
        near_approached |= arm(&game, near) == Some("approach");
        far_approached |= arm(&game, far) == Some("approach");
    }
    assert!(near_approached, "the nearer closes in");
    assert!(!far_approached, "the other fights by the tree");
}

/// A skid is not being stuck: the stuck and breach clocks stand still
/// while a knocked enemy slides.
#[test]
fn a_shoved_enemy_is_not_counted_stuck() {
    let mut game = round("");
    let enemy = parked(&mut game, Position::new(196.0, 192.0));
    step(&mut game, true);
    let mut checked = 0;
    for _ in 0..90 {
        // A think that saw the skid: the knock lands after the frame's
        // thinking, and the frame's timers run before it, so neither the
        // frame it lands on nor the one it runs out on is one.
        let before = game.world.get::<&Ai>(enemy).unwrap().snapshot();
        let skidding = with_tank(&game.world, enemy, |t| t.skid > DT);
        step(&mut game, false);
        if skidding {
            let after = game.world.get::<&Ai>(enemy).unwrap().snapshot();
            assert!(after.stuck_timer <= before.stuck_timer, "{} -> {}", before.stuck_timer, after.stuck_timer);
            assert!(after.wall_ahead_timer <= before.wall_ahead_timer);
            checked += 1;
        }
    }
    assert!(checked > 10, "it skidded for {checked} frames");
}

/// How far `entity` slides from the frame after the knock to the end of
/// its skid, and the speed the knock left it at.
fn skid_run(game: &mut Game, entity: Entity) -> (f32, f32) {
    step(game, true);
    let mut start: Option<(Position, f32)> = None;
    for _ in 0..200 {
        step(game, false);
        let (skid, speed, at) = with_tank(&game.world, entity, |t| (t.skid, t.skid_speed, t.position));
        if skid > 0.0 && start.is_none() {
            start = Some((at, speed));
        }
        if skid == 0.0 && start.is_some() {
            break;
        }
    }
    let (from, speed) = start.expect("a skid");
    (pos(game, entity).distance_to(from), speed)
}

/// The rule's prediction of a slide (`sonic::slide`) is the skid the
/// simulation runs, within the frame it starts on.
#[test]
fn the_predicted_slide_is_the_skid() {
    let mut game = round("");
    let enemy = parked(&mut game, Position::new(190.0, 192.0));
    let (slid, speed) = skid_run(&mut game, enemy);
    let predicted = crate::sonic::slide(&tuning(), speed, 1.0);
    assert!((slid + speed * DT - predicted).abs() < 3.0, "slid {slid} + a frame, predicted {predicted}");
}

/// Under snow a lake is ice, and a knock on it slides as far as the grip
/// floor lets it: `1 / sonic_skid_grip_floor` of the dry slide.
#[test]
fn ice_lengthens_the_skid() {
    let mut lake = String::from("weather = \"snow\"\n");
    for c in 5..=14 {
        for r in 3..=10 {
            lake.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"water\" }}\n"));
        }
    }
    let at = Position::new(224.0, 192.0);
    let mut dry = round("");
    let e = parked(&mut dry, at);
    let (dry_slide, dry_speed) = skid_run(&mut dry, e);
    let mut ice = round(&lake);
    let e = parked(&mut ice, at);
    let (ice_slide, ice_speed) = skid_run(&mut ice, e);
    assert!((dry_speed - ice_speed).abs() < 1.0, "the same knock: {dry_speed} vs {ice_speed}");
    let ratio = ice_slide / dry_slide;
    let want = 1.0 / tuning().sonic_skid_grip_floor;
    assert!((ratio - want).abs() < 0.3, "ice {ice_slide} vs dry {dry_slide}: {ratio}, want {want}");
}

#[test]
fn a_fused_drum_is_thrown_too() {
    let mut game = round("cells.\"5,6\" = { kind = \"barrel\", drum = \"oil\" }\n");
    let drum = game.world.query::<(Entity, &Obstacle)>().iter().find(|(_, o)| o.material.is_explosive()).map(|(e, _)| e).expect("the drum");
    game.world.get::<&mut Obstacle>(drum).unwrap().fuse = Some(crate::obstacle::Fuse { left: 3.0, total: 3.0, from: None });
    parked(&mut game, Position::new(256.0, 192.0));
    let events = blast(&mut game, 30);
    assert!(events.iter().any(|e| matches!(e, Event::DrumLaunched { .. })), "{events:?}");
}

/// The wave moves no crate - a crate on the ground is not a body - and no
/// shell in flight.
#[test]
fn a_crate_in_the_cone_is_left_alone() {
    let mut game = round("");
    let at = game.debug_spawn_pickup(PickupKind::Health, Position::new(192.0, 192.0)).expect("a crate");
    blast(&mut game, 40);
    assert!(game.world.query::<&crate::pickup::Pickup>().iter().any(|p| p.position == at), "the crate stands where it was");
}

/// A tank shoved onto a portal goes through, and the jump ends its skid.
#[test]
fn a_tank_shoved_into_a_portal_goes_through_and_stops_skidding() {
    let mut game = round("cells.\"8,6\" = { kind = \"portal\" }\ncells.\"28,13\" = { kind = \"portal\" }\n");
    let enemy = parked(&mut game, Position::new(176.0, 192.0));
    let slot = with_tank(&game.world, enemy, |t| t.owner_slot());
    let events = blast(&mut game, 60);
    let jumped = events.iter().position(|e| matches!(e, Event::Teleported { slot: s, .. } if *s == slot));
    assert!(jumped.is_some(), "it went through: {events:?}");
    assert!(pos(&game, enemy).distance_to(crate::map::cell_to_world(28, 13)) < 96.0, "and came out by the other portal");
    assert_eq!(with_tank(&game.world, enemy, |t| t.skid), 0.0);
}

/// A wave still running when the round ends runs its picture out on the
/// end screen and moves nobody.
#[test]
fn the_wave_finishes_on_the_end_screen_and_moves_nothing() {
    let mut game = round_with("cells.\"30,14\" = { kind = \"frog\" }\n", 2, Mission::Protect);
    let mate = game.seat(1).unwrap();
    let mate_at = Position::new(240.0, 192.0);
    game.place_tank(mate, mate_at, Some(0.0)).unwrap();
    step(&mut game, true);
    let frog = game.frog.expect("a frog");
    with_frog_mut(&game.world, frog, |f| f.damage(1.0e6));
    step(&mut game, false);
    assert_ne!(game.outcome(), crate::simulation::Outcome::Playing, "the round is lost");
    let mate_at = pos(&game, mate);
    let mut drawn = 0;
    for _ in 0..60 {
        step(&mut game, false);
        drawn += usize::from(!game.sonic_waves.is_empty());
    }
    assert!(drawn > 5, "the wave runs on on the end screen");
    assert!(game.sonic_waves.is_empty(), "and out");
    assert!(moved(&game, mate, mate_at) < 0.5, "moving nobody");
}
