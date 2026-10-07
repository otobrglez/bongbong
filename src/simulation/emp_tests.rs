//! Headless scenario tests for the EMP burst (docs/emp-burst.md "Tests"):
//! one per promised rule, on the default 34 x 17 field with the seat at
//! cell (3, 6) facing east and enemies placed by hand. Tests cannot touch
//! the global tuning table, so every number is read off the defaults.

use super::*;
use crate::ai::Role;
use crate::missile::Missile;
use crate::tuning::Tuning;

const W: f32 = 1088.0;
const H: f32 = 544.0;
const DT: f32 = 1.0 / 60.0;

/// The seat's pivot: cell (3, 6).
const SEAT: Position = Position::new(96.0, 192.0);

/// A round on an open field plus `extra` cells, `players` seats (the second
/// at cell (3, 12)), nobody shielded, no banner, the first seat at `SEAT`
/// facing east with a crate's worth of pulses.
fn round_with(extra: &str, players: usize) -> Game {
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.show_intro = false;
    game.level_overrides.mission = Some(Mission::Destroy);
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
        t.emp_charges = tuning().emp_charges_per_pickup;
    });
    game
}

fn round(extra: &str) -> Game {
    round_with(extra, 1)
}

/// A parked enemy of chassis `row` at `at`: it cannot drive or shoot.
fn parked(game: &mut Game, at: Position) -> Entity {
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

/// The seat's trigger for one frame; the frame's events.
fn step(game: &mut Game, fire: bool) -> Vec<Event> {
    let mut input = Input::default();
    input.seats[0].fire = fire;
    game.update(input, DT, W, H);
    game.events().to_vec()
}

/// Press the trigger for one frame, then `frames` idle ones; every event.
fn pulse(game: &mut Game, frames: usize) -> Vec<Event> {
    let mut seen = step(game, true);
    for _ in 0..frames {
        seen.extend(step(game, false));
    }
    seen
}

fn tank<R>(game: &Game, entity: Entity, f: impl FnOnce(&Tank) -> R) -> R {
    with_tank(&game.world, entity, f)
}

fn slot_of(game: &Game, entity: Entity) -> usize {
    tank(game, entity, |t| t.owner_slot())
}

#[test]
fn an_emp_crate_arms_the_burst_and_replaces_the_special_carried() {
    let t = Tuning::DEFAULT;
    let mut tank = Tank::default();
    tank.take_weapon(ActiveWeapon::Laser);
    tank.take_weapon(ActiveWeapon::Emp);
    assert_eq!((tank.special(), tank.laser_charges, tank.emp_charges), (Some(ActiveWeapon::Emp), 0, t.emp_charges_per_pickup));
    tank.emp_charges = 1;
    tank.take_weapon(ActiveWeapon::Emp);
    assert_eq!(tank.emp_charges, t.emp_charges_per_pickup, "a second crate refills to a crate's worth");
    assert_eq!(ActiveWeapon::Emp.full_load(), t.emp_charges_per_pickup);
    assert_eq!(crate::pickup::PickupKind::Emp.weapon(), Some(ActiveWeapon::Emp));
}

/// One pulse per press: `Fired` then `EmpPulse` in one tick, a charge
/// spent, the shooter's own special offline for `emp_disable_seconds` -
/// its trigger firing shells meanwhile - and the next press after it
/// pulsing again.
#[test]
fn the_emp_fires_on_the_press_and_takes_its_own_special_offline() {
    let mut game = round("");
    let seat = game.player().unwrap();
    let events = step(&mut game, true);
    let fired = events.iter().position(|e| matches!(e, Event::Fired { slot: 0, weapon: "emp_burst" })).expect("fired");
    let pulsed = events.iter().position(|e| matches!(e, Event::EmpPulse { slot: 0, .. })).expect("pulsed");
    assert!(fired < pulsed, "Fired first, in the same tick");
    let t = Tuning::DEFAULT;
    tank(&game, seat, |tk| {
        assert_eq!(tk.emp_charges, t.emp_charges_per_pickup - 1);
        assert!(tk.special_offline > 0.0 && tk.active_weapon() == ActiveWeapon::Shell, "offline: the trigger fires shells");
        assert_eq!(tk.special(), Some(ActiveWeapon::Emp), "it still carries the EMP");
        assert!(!tk.is_disabled(), "the shooter's own electrics stay on");
    });
    // A press within the offline fires a shell.
    for _ in 0..40 {
        step(&mut game, false);
    }
    let events = step(&mut game, true);
    assert!(events.iter().any(|e| matches!(e, Event::Fired { slot: 0, weapon: "shell" })), "{events:?}");
    // After it, the next press pulses.
    for _ in 0..((t.emp_disable_seconds / DT) as usize + 10) {
        step(&mut game, false);
    }
    assert!(tank(&game, seat, |tk| tk.active_weapon()) == ActiveWeapon::Emp);
    let events = step(&mut game, true);
    assert!(events.iter().any(|e| matches!(e, Event::EmpPulse { slot: 0, .. })));
}

/// The ring reaches a near hull before a far one, by the nearest point of
/// its box, and nothing past its reach.
#[test]
fn the_ring_reaches_far_things_later_and_stops_at_its_reach() {
    let mut game = round("");
    let t = Tuning::DEFAULT;
    let near = parked(&mut game, Position::new(SEAT.x + 60.0, SEAT.y));
    let far = parked(&mut game, Position::new(SEAT.x + 150.0, SEAT.y));
    // Straight below, its box's nearest point a little past the reach
    // whichever way the hull turns.
    let half = tank(&game, far, |tk| tk.hull_bbox_world().1.x.max(tk.hull_bbox_world().1.y));
    let past = parked(&mut game, Position::new(SEAT.x, SEAT.y + t.emp_radius_px + half + 12.0));
    let (near_slot, far_slot, past_slot) = (slot_of(&game, near), slot_of(&game, far), slot_of(&game, past));
    let mut at: BTreeMap<usize, usize> = BTreeMap::new();
    for frame in 0..30 {
        for e in step(&mut game, frame == 0) {
            if let Event::Disabled { slot, .. } = e {
                at.entry(slot).or_insert(frame);
            }
        }
    }
    let (n, f) = (at[&near_slot], at[&far_slot]);
    assert!(n < f, "the near hull first: {at:?}");
    assert!(!at.contains_key(&past_slot), "nothing past the reach: {at:?}");
    assert!(tank(&game, far, |tk| tk.is_disabled()) && !tank(&game, past, |tk| tk.is_disabled()));
}

/// An EMP is not sound: iron between the shooter and an enemy stops
/// nothing.
#[test]
fn walls_do_not_stop_the_pulse() {
    let mut game = round("cells.\"5,6\" = { kind = \"wall\", material = \"iron\" }\ncells.\"5,5\" = { kind = \"wall\", material = \"iron\" }\ncells.\"5,7\" = { kind = \"wall\", material = \"iron\" }\n");
    let behind = parked(&mut game, Position::new(SEAT.x + 128.0, SEAT.y));
    pulse(&mut game, 20);
    assert!(tank(&game, behind, |tk| tk.is_disabled()));
}

/// A struck enemy coasts on the intent it had - the trigger released, its
/// brain off, its clocks frozen - and reboots when the outage ends, its
/// stuck clock cleared.
#[test]
fn a_struck_enemy_coasts_on_its_last_intent_and_reboots() {
    let mut game = round("");
    let enemy = parked(&mut game, Position::new(SEAT.x + 120.0, SEAT.y + 40.0));
    with_tank_mut(&game.world, enemy, |tk| {
        tk.speed_scale = 1.0;
        tk.shells_ammo = 10;
    });
    // A few frames to think and move.
    for _ in 0..10 {
        step(&mut game, false);
    }
    // The intent it had when the ring reached it.
    let mut last = None;
    for i in 0..30 {
        let struck = step(&mut game, i == 0).iter().any(|e| matches!(e, Event::Disabled { .. }));
        if struck {
            last = Some(game.world.get::<&Ai>(enemy).unwrap().last_intent());
            break;
        }
    }
    let last = last.expect("struck");
    for _ in 0..30 {
        step(&mut game, false);
    }
    let ai = game.world.get::<&Ai>(enemy).unwrap().snapshot();
    assert!(ai.down, "its brain is off");
    let intent_now = game.world.get::<&Ai>(enemy).unwrap().last_intent();
    assert_eq!((intent_now.move_dir, intent_now.face), (last.move_dir, last.face), "it coasts on its last intent");
    let fire_timer = ai.fire_timer;
    let mut fired = false;
    for _ in 0..60 {
        fired |= step(&mut game, false).iter().any(|e| matches!(e, Event::Fired { slot, .. } if *slot == slot_of(&game, enemy)));
    }
    assert!(!fired, "the trigger released");
    assert_eq!(game.world.get::<&Ai>(enemy).unwrap().snapshot().fire_timer, fire_timer, "its clocks frozen");
    for _ in 0..((tuning().emp_disable_seconds / DT) as usize) {
        step(&mut game, false);
    }
    let ai = game.world.get::<&Ai>(enemy).unwrap().snapshot();
    assert!(!ai.down, "rebooted");
    assert!(ai.stuck_timer < 0.5, "no stuck evidence from the coast: {}", ai.stuck_timer);
}

#[test]
fn a_live_shield_pops_with_shield_broken() {
    let mut game = round("");
    let enemy = parked(&mut game, Position::new(SEAT.x + 80.0, SEAT.y));
    with_tank_mut(&game.world, enemy, |tk| tk.raise_shield());
    let events = pulse(&mut game, 20);
    assert!(events.iter().any(|e| matches!(e, Event::ShieldBroken { slot, .. } if *slot == slot_of(&game, enemy))));
    assert!(!tank(&game, enemy, |tk| tk.is_shielded()));
}

/// A wind-up of any weapon lapses when the ring reaches it: a hammer's
/// tell goes off nowhere.
#[test]
fn an_enemys_tell_lapses_when_it_is_struck() {
    let mut game = round("");
    let enemy = parked(&mut game, Position::new(SEAT.x + 80.0, SEAT.y));
    with_tank_mut(&game.world, enemy, |tk| {
        tk.sonic_ammo = 3;
        tk.tell = Some(crate::tank::Tell { weapon: ActiveWeapon::SonicHammer, left: 0.4, total: 0.55, facing: Dir::Left });
    });
    let events = pulse(&mut game, 60);
    assert!(!events.iter().any(|e| matches!(e, Event::SonicBlast { slot, .. } if *slot != 0)), "no blast");
    assert!(tank(&game, enemy, |tk| tk.tell.is_none()));
}

/// What a special had in hand stops short, its unfired rounds kept; a
/// twin gun's second shell still leaves.
#[test]
fn a_burst_and_a_volley_stop_short_and_keep_their_rounds() {
    let mut tk = Tank::default();
    tk.minigun_ammo = 30;
    tk.minigun_burst = Some(crate::tank::MinigunBurst { bullets_remaining: 4, timer: 0.05, aim_offset: 0.0 });
    tk.missile_volley = Some(crate::tank::MissileVolley { missiles_remaining: 3, timer: 0.1, next_tube: 1 });
    tk.pending_shot = Some(crate::tank::PendingShot { timer: 0.1, aim_offset: 0.0, lateral_offset: 4.0 });
    tk.flame_held = true;
    assert!(tk.disable(3.0), "newly disabled");
    assert!(tk.minigun_burst.is_none() && tk.missile_volley.is_none() && !tk.flame_held);
    assert_eq!(tk.minigun_ammo, 30, "the rounds kept");
    assert!(tk.pending_shot.is_some(), "shells are not electric");
    assert!(!tk.disable(2.0) && tk.disabled == 3.0, "a second strike never shortens it");
}

/// A seat the ring reaches - a teammate's pulse here - keeps its stick and
/// its shells: its special is offline, its press fires a shell.
#[test]
fn a_struck_seat_drives_and_fires_shells_with_its_special_offline() {
    let mut game = round_with("", 2);
    let second = game.seat(1).unwrap();
    game.place_tank(second, Position::new(SEAT.x, SEAT.y + 100.0), Some(90.0)).unwrap();
    with_tank_mut(&game.world, second, |tk| tk.take_weapon(ActiveWeapon::Laser));
    pulse(&mut game, 20);
    tank(&game, second, |tk| {
        assert!(tk.is_disabled() && tk.special() == Some(ActiveWeapon::Laser) && tk.active_weapon() == ActiveWeapon::Shell);
    });
    let at = tank(&game, second, |tk| tk.position);
    let mut fired_shell = false;
    for i in 0..30 {
        let mut input = Input::default();
        input.seats[1].move_dir = Some(Dir::Right);
        input.seats[1].fire = i == 0;
        game.update(input, DT, W, H);
        fired_shell |= game.events().iter().any(|e| matches!(e, Event::Fired { slot: 1, weapon: "shell" }));
    }
    assert!(fired_shell, "its press fires a shell");
    assert!(tank(&game, second, |tk| tk.position.x) > at.x + 20.0, "it drives");
}

/// A tower in the ring goes offline - a tesla's charge lost, no strike -
/// for `emp_tower_seconds`, then fights again.
#[test]
fn towers_in_the_ring_go_offline_and_come_back() {
    let mut game = round("cells.\"6,6\" = { kind = \"tesla\", side = \"enemy\" }\n");
    let seat = game.player().unwrap();
    with_tank_mut(&game.world, seat, |tk| tk.damage = 0.0);
    // Let it charge a while, then pulse.
    for _ in 0..20 {
        step(&mut game, false);
    }
    assert!(game.towers.get(&(6, 6)).unwrap().charge > 0.0, "charging at the seat");
    pulse(&mut game, 10);
    let tower = game.towers.get(&(6, 6)).cloned().unwrap();
    assert!(tower.disabled > 0.0 && tower.charge == 0.0, "offline, its charge lost");
    let t = Tuning::DEFAULT;
    let offline_frames = ((t.emp_tower_seconds - 0.3) / DT) as usize;
    let mut struck = false;
    for _ in 0..offline_frames {
        struck |= step(&mut game, false).iter().any(|e| matches!(e, Event::TeslaStrike { .. }));
        with_tank_mut(&game.world, seat, |tk| tk.damage = 0.0);
    }
    assert!(!struck, "no strike while offline");
    let mut struck = false;
    for _ in 0..((t.tesla_charge_seconds + 1.0) / DT) as usize {
        struck |= step(&mut game, false).iter().any(|e| matches!(e, Event::TeslaStrike { .. }));
        with_tank_mut(&game.world, seat, |tk| tk.damage = 0.0);
    }
    assert!(struck, "it strikes again once back");
}

/// A missile in the air in the ring dies and comes down a dud: no blast.
#[test]
fn a_missile_in_the_ring_falls_dead_and_lands_a_dud() {
    let mut game = round("");
    let mut m = Missile::spawn(Position::new(SEAT.x + 100.0, SEAT.y), Vec2::new(1.0, 0.0), Owner::Enemy(9), 0, Position::new(600.0, 192.0));
    m.height = 30.0;
    m.set_id(900);
    game.world.spawn((m,));
    let mut far = Missile::spawn(Position::new(900.0, 400.0), Vec2::new(1.0, 0.0), Owner::Enemy(9), 1, Position::new(1000.0, 400.0));
    far.height = 30.0;
    far.set_id(901);
    game.world.spawn((far,));
    let events = pulse(&mut game, 120);
    assert!(events.iter().any(|e| matches!(e, Event::MissileDud { .. })), "a dud lands");
    let dud = events.iter().find_map(|e| match e {
        Event::MissileDud { x, y } => Some(Position::new(*x, *y)),
        _ => None,
    });
    assert!(!events.iter().any(|e| matches!(e, Event::MissileBlast { x, .. } if dud.is_some_and(|d| (d.x - x).abs() < 1.0))), "no blast where it fell");
    assert!(game.world.query::<&Missile>().iter().all(|m| m.id != 900), "gone");
    assert!(
        game.world.query::<&Missile>().iter().any(|m| m.id == 901 && !m.is_dead()) || events.iter().any(|e| matches!(e, Event::MissileBlast { .. })),
        "the one outside the ring flies on"
    );
}

/// At night the pulse puts every lamp post on the map out for
/// `emp_lamp_seconds` - a seat beside one is no longer lit for the enemies
/// - and they light again after.
#[test]
fn at_night_the_lamp_posts_go_out_and_come_back() {
    let mut game = round("weather = \"night\"\ncells.\"20,12\" = { kind = \"lamp\" }\n");
    assert!(game.is_night());
    let by_lamp = Position::new(20.0 * 32.0, 12.0 * 32.0 + 40.0);
    let t = Tuning::DEFAULT;
    assert!(game.is_lit(by_lamp, &t));
    pulse(&mut game, 1);
    assert!(game.lamps_out() > 0.0 && game.lit_lamp_posts().is_empty());
    assert!(!game.is_lit(by_lamp, &t), "dark while they are out");
    assert!(game.sight_on(by_lamp) < t.enemy_view_range);
    for _ in 0..((t.emp_lamp_seconds / DT) as usize + 5) {
        step(&mut game, false);
    }
    assert_eq!(game.lamps_out(), 0.0);
    assert!(game.is_lit(by_lamp, &t), "lit again");
}

#[test]
fn by_day_the_lamp_posts_are_left_alone() {
    let mut game = round("cells.\"20,12\" = { kind = \"lamp\" }\n");
    pulse(&mut game, 5);
    assert_eq!(game.lamps_out(), 0.0);
    assert_eq!(game.lit_lamp_posts().len(), 1);
}

/// The shooter is the eye of the storm: its lights and its shield stay.
#[test]
fn the_shooter_keeps_its_lights_and_its_shield() {
    let mut game = round("");
    let seat = game.player().unwrap();
    with_tank_mut(&game.world, seat, |tk| tk.raise_shield());
    pulse(&mut game, 20);
    tank(&game, seat, |tk| assert!(tk.is_shielded() && !tk.is_disabled()));
}

/// The pulse is blind: a teammate in the ring is disabled, and an enemy's
/// pulse disables its fellow enemies.
#[test]
fn the_pulse_disables_a_teammate_and_fellow_enemies() {
    let mut game = round_with("", 2);
    let second = game.seat(1).unwrap();
    game.place_tank(second, Position::new(SEAT.x + 40.0, SEAT.y + 90.0), Some(0.0)).unwrap();
    pulse(&mut game, 20);
    assert!(tank(&game, second, |tk| tk.is_disabled()), "a teammate");
    // An enemy's own pulse, fired by hand, disables the enemy beside it.
    let mut game = round("");
    let a = parked(&mut game, Position::new(600.0, 300.0));
    let b = parked(&mut game, Position::new(680.0, 300.0));
    with_tank_mut(&game.world, a, |tk| {
        tk.emp_charges = 1;
        tk.tell = Some(crate::tank::Tell { weapon: ActiveWeapon::Emp, left: DT * 0.5, total: 0.5, facing: Dir::Up });
    });
    for _ in 0..20 {
        step(&mut game, false);
    }
    assert!(tank(&game, b, |tk| tk.is_disabled()), "a fellow enemy");
    assert!(!tank(&game, a, |tk| tk.is_disabled()), "never the shooter");
}

#[test]
fn a_wreck_is_not_struck() {
    let mut game = round("");
    let wreck = parked(&mut game, Position::new(SEAT.x + 80.0, SEAT.y));
    with_tank_mut(&game.world, wreck, |tk| tk.damage = crate::MAX_DAMAGE);
    step(&mut game, false);
    let slot = slot_of(&game, wreck);
    let events = pulse(&mut game, 20);
    assert!(!events.iter().any(|e| matches!(e, Event::Disabled { slot: s, .. } if *s == slot)));
}

/// On the end screen the ring finishes its picture and strikes nothing.
#[test]
fn the_ring_finishes_on_the_end_screen_and_strikes_nothing() {
    let mut game = round("");
    let enemy = parked(&mut game, Position::new(SEAT.x + 150.0, SEAT.y));
    step(&mut game, true);
    // The seat goes down on the next frame and the round is lost.
    game.debug_kill(0).expect("the seat");
    step(&mut game, false);
    assert_ne!(game.outcome(), Outcome::Playing, "the round is lost");
    let mut drawn = 0;
    for _ in 0..40 {
        step(&mut game, false);
        drawn += usize::from(!game.emp_pulses.is_empty());
    }
    assert!(drawn > 5, "the ring runs on on the end screen");
    assert!(!tank(&game, enemy, |tk| tk.is_disabled()), "striking nothing");
    assert!(game.emp_pulses.is_empty(), "and out");
}

/// A round with pulses in it is a pure function of its inputs.
#[test]
fn a_round_with_the_emp_replays_bit_for_bit() {
    let run = || {
        let mut game = round("weather = \"night\"\ncells.\"20,12\" = { kind = \"lamp\" }\ncells.\"8,8\" = { kind = \"tesla\", side = \"enemy\" }\n");
        let _ = parked(&mut game, Position::new(200.0, 200.0));
        let _ = parked(&mut game, Position::new(240.0, 260.0));
        let mut trace = Vec::new();
        for i in 0..400 {
            step(&mut game, i % 90 == 0);
            trace.push(format!("{:?}", game.drawable_state()));
        }
        trace
    };
    assert_eq!(run(), run());
}

/// The hashed spawn swap hands the EMP out by its share and draws nothing
/// from the round's RNG.
#[test]
fn the_spawn_swap_hands_out_the_emp_by_its_share() {
    let at = Position::new(400.0, 200.0);
    let mut t = Tuning::DEFAULT;
    let mut laser = Tank::default();
    laser.take_weapon(ActiveWeapon::Laser);
    sonic::swap_spawn_special_with(&t, &mut laser, 5, at);
    assert_eq!(laser.special(), Some(ActiveWeapon::Laser), "nothing at the default share");
    t.enemy_special_weapon_emp_share = 1.0;
    sonic::swap_spawn_special_with(&t, &mut laser, 5, at);
    assert_eq!(laser.special(), Some(ActiveWeapon::Emp));
    let mut shells = Tank::default();
    sonic::swap_spawn_special_with(&t, &mut shells, 5, at);
    assert_eq!(shells.special(), None, "a tank that drew none keeps none");
}

/// An enemy takes a weapon crate only on shells - and a special offline is
/// still carried, so it does not trade it away.
#[test]
fn an_enemy_takes_a_crate_only_on_shells_and_not_while_its_special_is_offline() {
    let mut tk = Tank { owner: Owner::Enemy(4), ..Tank::default() };
    assert!(tk.wants_pickup(crate::pickup::PickupKind::Emp));
    tk.take_weapon(ActiveWeapon::Plasma);
    tk.disable(3.0);
    assert_eq!(tk.active_weapon(), ActiveWeapon::Shell);
    assert!(!tk.wants_pickup(crate::pickup::PickupKind::Emp), "it still carries its plasma");
    assert!(!tk.wants_pickup(crate::pickup::PickupKind::Laser));
}

/// An EMP-armed enemy of the player role at `at`, parked.
fn emp_enemy(game: &mut Game, at: Position) -> Entity {
    let entity = parked(game, at);
    with_tank_mut(&game.world, entity, |tk| tk.emp_charges = 3);
    entity
}

/// The arm the enemy's special rule took on its last think.
fn arm(game: &Game, entity: Entity) -> Option<&'static str> {
    game.world.get::<&Ai>(entity).expect("an enemy").snapshot().special
}

/// An enemy with the EMP pulses a seat in its ring - a bare one by day too,
/// the condition being a seat in reach - after its crackle, the tank held,
/// the seat it is used on recorded.
#[test]
fn an_emp_enemy_pulses_a_seat_in_its_ring_after_its_crackle() {
    let mut game = round("");
    let seat = game.player().unwrap();
    with_tank_mut(&game.world, seat, |tk| tk.emp_charges = 0);
    let enemy = emp_enemy(&mut game, Position::new(SEAT.x + 110.0, SEAT.y));
    let slot = slot_of(&game, enemy);
    let (mut told, mut pulsed, mut aimed) = (None, None, None);
    for frame in 0..300 {
        for e in step(&mut game, false) {
            match e {
                Event::TellStarted { slot: s, weapon: "emp_burst" } if s == slot && told.is_none() => told = Some(frame),
                Event::EmpPulse { slot: s, .. } if s == slot && pulsed.is_none() => pulsed = Some(frame),
                _ => {}
            }
        }
        if told.is_some() && aimed.is_none() {
            aimed = Some(game.world.get::<&Ai>(enemy).unwrap().shot_at_seat());
        }
    }
    let (told, pulsed) = (told.expect("a crackle"), pulsed.expect("a pulse"));
    let ticks = (tuning().emp_tell_seconds / DT).round() as i32;
    assert!(((pulsed - told) as i32 - ticks).abs() <= 1, "told {told}, pulsed {pulsed}");
    assert_eq!(aimed, Some(Some(0)), "used on the seat");
    assert!(tank(&game, seat, |tk| tk.is_disabled()) || tank(&game, seat, |tk| tk.disabled == 0.0), "the seat was in its ring");
}

/// An ally in its ring holds the pulse (with the commander off, the
/// default), and so does one of its own side's towers.
#[test]
fn the_emp_holds_for_an_ally_and_for_its_own_tower() {
    for extra in ["", "cells.\"8,7\" = { kind = \"tesla\", side = \"enemy\" }\n"] {
        let mut game = round(extra);
        let seat = game.player().unwrap();
        with_tank_mut(&game.world, seat, |tk| tk.emp_charges = 0);
        let enemy = emp_enemy(&mut game, Position::new(SEAT.x + 120.0, SEAT.y + 32.0));
        if extra.is_empty() {
            let _ally = parked(&mut game, Position::new(SEAT.x + 120.0, SEAT.y + 120.0));
        }
        let slot = slot_of(&game, enemy);
        let mut told = false;
        for _ in 0..240 {
            told |= step(&mut game, false).iter().any(|e| matches!(e, Event::TellStarted { slot: s, .. } if *s == slot));
        }
        assert!(!told, "held ({extra:?})");
    }
}

/// A training dummy never pulses.
#[test]
fn a_training_dummy_never_pulses() {
    let mut game = round("");
    let enemy = emp_enemy(&mut game, Position::new(SEAT.x + 110.0, SEAT.y));
    game.world.get::<&mut Ai>(enemy).unwrap().frog_only = true;
    let mut told = false;
    for _ in 0..240 {
        told |= step(&mut game, false).iter().any(|e| matches!(e, Event::TellStarted { .. }));
    }
    assert!(!told);
}

/// By day a closer leaves a bare seat alone - it pulses only what comes
/// into its ring - and at night it goes looking for it.
#[test]
fn the_closer_leaves_a_bare_seat_by_day_and_closes_in_at_night() {
    for (sky, want) in [("", false), ("weather = \"night\"\n", true)] {
        let mut game = round(sky);
        let seat = game.player().unwrap();
        with_tank_mut(&game.world, seat, |tk| tk.emp_charges = 0);
        let enemy = emp_enemy(&mut game, Position::new(SEAT.x + 300.0, SEAT.y));
        with_tank_mut(&game.world, enemy, |tk| tk.speed_scale = 1.0);
        let mut closing = false;
        for _ in 0..120 {
            step(&mut game, false);
            closing |= matches!(arm(&game, enemy), Some("approach" | "close"));
        }
        assert_eq!(closing, want, "sky {sky:?}");
    }
}

/// An enemy keeps out of the ring of a seat carrying an armed EMP: one
/// inside backs out past the edge and does not come back while it is
/// armed.
#[test]
fn enemies_keep_out_of_an_armed_seats_ring() {
    let mut game = round("");
    let t = Tuning::DEFAULT;
    let seat = game.player().unwrap();
    let enemy = parked(&mut game, Position::new(SEAT.x + 150.0, SEAT.y + 64.0));
    with_tank_mut(&game.world, enemy, |tk| {
        tk.speed_scale = 1.0;
        tk.shells_ammo = 10;
    });
    let berth = t.emp_radius_px + t.emp_ai_berth_px;
    let mut out_at = None;
    let mut back_in = 0;
    let mut was_out = false;
    for frame in 0..600 {
        step(&mut game, false);
        let d = tank(&game, enemy, |tk| tk.position).distance_to(tank(&game, seat, |tk| tk.position));
        if out_at.is_none() && d > berth {
            out_at = Some(frame);
        }
        // A crossing back in after it was out.
        if was_out && d < berth {
            back_in += 1;
        }
        was_out = d >= berth;
        if out_at.is_some() {
            assert!(d > t.emp_radius_px + 16.0, "never back in its reach: {d} at frame {frame}");
        }
    }
    assert!(out_at.is_some(), "it backed out");
    assert!(back_in <= 1, "it does not hunt the edge: {back_in} crossings back in");
}

/// A seat's EMP offline is no danger: the pack may close in.
#[test]
fn a_seats_emp_is_a_danger_only_while_armed() {
    let game = round("");
    let seat = game.player().unwrap();
    let dangers = |game: &Game| {
        let seats: Vec<emp::EmpSeat> =
            game.players().into_iter().flatten().enumerate().map(|(i, e)| with_tank(&game.world, e, |tk| emp::EmpSeat::of(i as u8, tk, true, false))).collect();
        game.emp_dangers(&seats)
    };
    assert_eq!(dangers(&game).len(), 1, "armed");
    with_tank_mut(&game.world, seat, |tk| tk.special_offline = 2.0);
    assert!(dangers(&game).is_empty(), "offline");
}

/// A disabled enemy spots nobody for the shared alert.
#[test]
fn a_disabled_enemy_spots_nobody() {
    let mut game = round("");
    let enemy = parked(&mut game, Position::new(SEAT.x + 200.0, SEAT.y));
    with_tank_mut(&game.world, enemy, |tk| {
        tk.disable(5.0);
    });
    game.alert_position = None;
    game.alert_timer = 0.0;
    for _ in 0..10 {
        step(&mut game, false);
    }
    assert!(game.alert_position.is_none(), "no alert from a blind tank");
}

/// A disabled enemy fires nothing, so the off-screen warning gives it no
/// lane: lined up on the seat it is read as one, struck it is not.
#[test]
fn a_disabled_enemy_has_no_lane_warning() {
    let mut game = round("");
    let seat = game.player().unwrap();
    with_tank_mut(&game.world, seat, |tk| tk.emp_charges = 0);
    let enemy = parked(&mut game, Position::new(SEAT.x + 250.0, SEAT.y));
    game.place_tank(enemy, Position::new(SEAT.x + 250.0, SEAT.y), Some(270.0)).unwrap();
    let lane = |game: &Game| crate::indicators::Scene::of(game, 0).tanks.iter().any(|t| t.lane);
    assert!(lane(&game), "lined up, a lane");
    with_tank_mut(&game.world, enemy, |tk| {
        tk.disable(2.0);
    });
    assert!(!lane(&game), "disabled, none");
}

/// The HUD's weapon slot reads the outage: `WPN OFFLINE` flickering
/// with the count, then the special back.
#[test]
fn the_weapon_slot_flickers_offline_while_the_special_is_down() {
    let mut tk = Tank::default();
    tk.take_weapon(ActiveWeapon::Laser);
    let slot = crate::hud::WeaponSlot::of(&tk, 0.0);
    assert_eq!((slot.offline, slot.weapon), (None, ActiveWeapon::Laser));
    tk.disable(2.0);
    let half = 0.5 / tuning().emp_hud_flicker_hz;
    let (a, b) = (crate::hud::WeaponSlot::of(&tk, 0.01), crate::hud::WeaponSlot::of(&tk, half + 0.01));
    assert_eq!((a.offline, b.offline), (Some(true), Some(false)), "it flickers");
    assert_eq!(a.weapon, ActiveWeapon::Laser, "the special it carries");
    let (top, bottom) = crate::hud::offline_lines("WPN OFFLINE");
    assert_eq!((top, bottom), ("WPN", Some("OFFLINE")));
}

/// The hammer's trouble reads the outage: a seat its knock would slide
/// into an enemy tower's reach is "trouble" while the tower fights, and
/// only "breaker" while an EMP has it offline.
#[test]
fn an_offline_tower_is_no_trouble_for_the_hammer() {
    let seat_at = Position::new(300.0, 192.0);
    let reach = tuning().tesla_range;
    let tower = (((seat_at.x + reach + 40.0) / 32.0).round() as i32, 6);
    for (offline, want) in [(false, "trouble"), (true, "breaker")] {
        let mut game = round(&format!("cells.\"{},{}\" = {{ kind = \"tesla\", side = \"enemy\" }}\n", tower.0, tower.1));
        let seat = game.player().unwrap();
        game.place_tank(seat, seat_at, Some(270.0)).unwrap();
        with_tank_mut(&game.world, seat, |tk| tk.emp_charges = 0);
        let enemy = parked(&mut game, Position::new(seat_at.x - 90.0, 192.0));
        with_tank_mut(&game.world, enemy, |tk| tk.sonic_ammo = 3);
        if offline {
            game.towers.get_mut(&tower).unwrap().disable(10.0);
        }
        step(&mut game, false);
        step(&mut game, false);
        assert_eq!(arm(&game, enemy), Some(want), "offline {offline}");
    }
}

/// A player tower offline is no detour for the enemies: its reach leaves
/// the route grid's surcharge while it is out.
#[test]
fn an_offline_player_tower_is_no_detour() {
    let mut game = round("cells.\"12,8\" = { kind = \"tesla\", side = \"player\" }\n");
    assert!(!game.player_tower_reach(W, H).is_empty(), "online, its reach is priced");
    game.towers.get_mut(&(12, 8)).unwrap().disable(5.0);
    assert!(game.player_tower_reach(W, H).is_empty(), "offline, it is not");
}

/// A disabled enemy's turret sags `emp_droop_deg` to its hashed side over
/// `emp_droop_seconds` and comes back after; a seat's never does.
#[test]
fn a_disabled_enemys_turret_sags_and_comes_back() {
    let mut game = round("");
    let enemy = parked(&mut game, Position::new(SEAT.x + 100.0, SEAT.y));
    let seat = game.player().unwrap();
    pulse(&mut game, 45);
    let t = Tuning::DEFAULT;
    let side = crate::emp::droop_side(slot_of(&game, enemy));
    assert!((tank(&game, enemy, |tk| tk.droop) - t.emp_droop_deg * side).abs() < 0.01, "sagged: {}", tank(&game, enemy, |tk| tk.droop));
    assert_eq!(tank(&game, seat, |tk| tk.droop), 0.0);
    for _ in 0..((t.emp_disable_seconds / DT) as usize + 30) {
        step(&mut game, false);
    }
    assert_eq!(tank(&game, enemy, |tk| tk.droop), 0.0, "back on its aim");
}

/// With the commander off, an EMP enemy holding its pulse for an ally is a
/// danger that ally keeps out of: the ally backs out of the ring, and the
/// pulse goes off with nobody of its own side in it.
#[test]
fn an_ally_backs_out_of_a_held_pulse_and_then_it_goes_off() {
    let mut game = round("");
    let seat = game.player().unwrap();
    with_tank_mut(&game.world, seat, |tk| tk.emp_charges = 0);
    let clearer = emp_enemy(&mut game, Position::new(SEAT.x + 110.0, SEAT.y));
    let ally = parked(&mut game, Position::new(SEAT.x + 110.0, SEAT.y + 90.0));
    with_tank_mut(&game.world, ally, |tk| {
        tk.speed_scale = 1.0;
        tk.shells_ammo = 10;
    });
    let slot = slot_of(&game, clearer);
    let ally_slot = slot_of(&game, ally);
    let mut pulsed = None;
    for frame in 0..600 {
        let events = step(&mut game, false);
        if events.iter().any(|e| matches!(e, Event::EmpPulse { slot: s, .. } if *s == slot)) {
            pulsed = Some(frame);
            break;
        }
    }
    let at = pulsed.expect("the pulse goes off once the ring is clear");
    let gap = tank(&game, ally, |tk| tk.position).distance_to(tank(&game, clearer, |tk| tk.position));
    assert!(gap > tuning().emp_radius_px, "the ally stood clear when it went off: {gap} at frame {at}");
    for _ in 0..20 {
        assert!(!step(&mut game, false).iter().any(|e| matches!(e, Event::Disabled { slot: s, .. } if *s == ally_slot)), "and it is not struck");
    }
}
