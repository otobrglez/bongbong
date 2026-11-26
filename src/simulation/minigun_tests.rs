//! The minigun (docs/BULLETS_SPEC.md §8): a rotary gun whose barrels spin
//! up before the first round, fire one unbroken stream while the trigger
//! is held, and run down after; a tap fires the least burst. Its turret
//! tracks the nearest opposing tank it can see inside a cone off the
//! hull's heading (`simulation::minigun`), and a seat's rounds stagger the
//! enemy they land on.

use super::*;
use crate::map::cell_to_world;

const W: f32 = 1280.0;
const H: f32 = 720.0;

/// Seat 0 at (5,10), one enemy, the bong far out of the line of fire.
const MAP: &str = r#"
version = 1
tanks = 1
cells."2,2" = { kind = "bong" }
cells."5,10" = { kind = "start" }
"#;

/// More rounds than any case fires.
const AMMO: i32 = 250;

/// Seat 0 facing east down an open row with `AMMO` minigun rounds, the
/// enemy parked off the line with nothing to fire.
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
    game.debug_set_tank(0, &debug::TankPatch { minigun_ammo: Some(AMMO), shield_hp: Some(0.0), ..Default::default() }).expect("seat 0");
    let slot = game.first_enemy_slot();
    game.debug_set_tank(slot, &debug::TankPatch { shells_ammo: Some(0), ..Default::default() }).expect("the enemy");
    game.debug_teleport(slot, cell_to_world(30, 20), Some(180.0)).expect("the enemy is on the field");
    game
}

fn fire() -> Input {
    Input::single(Intent { fire: true, ..Intent::default() })
}

fn step(game: &mut Game, input: Input) {
    game.update(input, PHYSICS_FIXED_DT, W, H);
}

/// Rounds seat 0 has fired.
fn spent(game: &Game) -> i32 {
    AMMO - with_tank(&game.world, game.seat(0).expect("seat 0"), |t| t.minigun_ammo)
}

fn spin(game: &Game) -> f32 {
    with_tank(&game.world, game.seat(0).expect("seat 0"), |t| t.minigun_spin)
}

fn fired_now(game: &Game) -> bool {
    game.events().iter().any(|e| matches!(e, Event::Fired { slot: 0, weapon: "minigun" }))
}

/// **Barrels at rest spin up before the first round.** The pull is logged
/// at once; the round leaves `minigun_spinup_seconds` later, on the tick,
/// with the barrels at full speed.
#[test]
fn a_pull_spins_the_barrels_up_before_the_first_round() {
    let mut game = game();
    let wait = tuning().minigun_spinup_ticks(0.0);
    assert!(wait > 0, "the default gun has a spin-up");
    step(&mut game, fire());
    assert!(fired_now(&game), "the pull is the `Fired`");
    assert_eq!(spent(&game), 0);
    for tick in 1..wait {
        step(&mut game, fire());
        assert_eq!(spent(&game), 0, "no round while the barrels come up (tick {tick})");
        assert!(spin(&game) > 0.0 && spin(&game) < 1.0);
    }
    step(&mut game, fire());
    assert_eq!(spent(&game), 1, "the first round, on the spin-up's last tick");
    step(&mut game, fire());
    assert_eq!(spin(&game), 1.0, "at full speed from there");
}

/// **A held trigger is one unbroken stream**: round `n` on
/// `minigun_round_tick(n)` after the first - gaps of 1, 1, 2 ticks at 45
/// a second - straight through the seams between one burst and the next.
#[test]
fn a_held_trigger_fires_one_unbroken_stream() {
    let mut game = game();
    let t = tuning();
    let (wait, burst) = (t.minigun_spinup_ticks(0.0), t.minigun_burst_size);
    let ticks = wait + 4 * t.minigun_round_tick(burst);
    let mut pulls = 0;
    for tick in 0..=ticks {
        step(&mut game, fire());
        pulls += fired_now(&game) as u32;
        let due = if tick < wait { 0 } else { (0..).take_while(|&n| t.minigun_round_tick(n) <= tick - wait).count() };
        assert_eq!(spent(&game), due as i32, "rounds fired by tick {tick}");
    }
    assert_eq!(spent(&game) as u32, 4 * burst + 1, "45 a second: three rounds in every four ticks");
    assert_eq!(pulls, 5, "four whole bursts and the fifth's first round");
    assert_eq!(spin(&game), 1.0);
}

/// **A tap fires the least burst**, `minigun_burst_size` rounds, then the
/// barrels run down; a pull while they still turn waits only for what is
/// left of the way up.
#[test]
fn a_tap_fires_the_least_burst_and_a_turning_gun_starts_sooner() {
    let mut game = game();
    let t = tuning();
    let (wait, burst) = (t.minigun_spinup_ticks(0.0), t.minigun_burst_size);
    step(&mut game, fire());
    let run = t.minigun_burst_ticks(wait, burst);
    for _ in 0..run {
        step(&mut game, Input::default());
    }
    assert_eq!(spent(&game), burst as i32, "the whole burst, the trigger long let go");
    // Half way down, pull again.
    let down = (t.minigun_spindown_seconds / PHYSICS_FIXED_DT / 2.0) as u32;
    for _ in 0..down {
        step(&mut game, Input::default());
    }
    assert_eq!(spent(&game), burst as i32, "and not a round more");
    step(&mut game, fire());
    let turning = spin(&game);
    assert!(turning > 0.2 && turning < 0.8, "half way down: {turning}");
    let sooner = t.minigun_spinup_ticks(turning);
    assert!(sooner > 0 && sooner < wait, "{sooner} of {wait} ticks");
    for _ in 1..sooner {
        step(&mut game, Input::default());
        assert_eq!(spent(&game), burst as i32);
    }
    step(&mut game, Input::default());
    assert_eq!(spent(&game), burst as i32 + 1, "its first round, on the shorter wait");
    // Left alone, the barrels come to rest.
    let rest = t.minigun_burst_ticks(0, burst) + (t.minigun_spindown_seconds / PHYSICS_FIXED_DT) as u32 + 2;
    for _ in 0..rest {
        step(&mut game, Input::default());
    }
    assert_eq!(spin(&game), 0.0);
}

/// **The stream is tight**: every round leaves within
/// `minigun_bullet_spread_deg` of the gun's boresight, and not all down
/// one line.
#[test]
fn every_round_leaves_within_the_spread() {
    let mut game = game();
    let t = tuning();
    let seat = game.seat(0).expect("seat 0");
    let mut off: Vec<f32> = Vec::new();
    let mut seen: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    for _ in 0..t.minigun_spinup_ticks(0.0) + 60 {
        step(&mut game, fire());
        let bore = with_tank(&game.world, seat, |tank| Bullet::spawn(tank, tank.owner(), 0.0).rotation);
        for b in game.world.query::<&Bullet>().iter() {
            if seen.insert(b.id) {
                off.push(b.rotation - bore);
            }
        }
    }
    assert!(off.len() >= 25, "a second of the stream: {}", off.len());
    let spread = t.minigun_bullet_spread_deg;
    assert!(off.iter().all(|d| d.abs() <= spread + 0.01), "{off:?}");
    assert!(off.iter().any(|d| d.abs() > spread * 0.25), "and it is a stream, not a beam: {off:?}");
}

/// **A replica's gun turns for the burst the room told it of**
/// (`Tank::kick_minigun`): up to speed, held there for the burst, then
/// down to rest - with no burst of its own to run it.
#[test]
fn a_replicas_gun_turns_for_the_burst_it_is_told_of() {
    let game = game();
    let t = tuning();
    let seat = game.seat(0).expect("seat 0");
    with_tank_mut(&game.world, seat, |tank| {
        tank.kick_minigun();
        assert!(tank.minigun_burst.is_none());
        let run = t.minigun_burst_ticks(t.minigun_spinup_ticks(0.0), t.minigun_burst_size);
        for _ in 0..run {
            tank.tick_minigun_spin(PHYSICS_FIXED_DT);
        }
        assert_eq!(tank.minigun_spin, 1.0, "at speed while the burst runs");
        for _ in 0..(t.minigun_spindown_seconds / PHYSICS_FIXED_DT) as u32 + 2 {
            tank.tick_minigun_spin(PHYSICS_FIXED_DT);
        }
        assert_eq!(tank.minigun_spin, 0.0, "and at rest after it");
    });
}

/// Seat 0's stream at a brick wall three cells down its row, for `ticks`:
/// the rounds that glanced off it (`Event::Ricochet`) and the ones that hit
/// it.
fn stream_at_a_wall(game: &mut Game, ticks: u32) -> (u32, u32) {
    let (mut glanced, mut hit) = (0, 0);
    for _ in 0..ticks {
        step(game, fire());
        for e in game.events() {
            match e {
                Event::Ricochet { slot: 0, .. } => glanced += 1,
                Event::Hit { cause: HitCause::Bullet, .. } => hit += 1,
                _ => {}
            }
        }
    }
    (glanced, hit)
}

/// The brick wall `stream_at_a_wall` fires at, thick enough not to fall.
fn walled() -> Game {
    let mut map = MAP.to_string();
    for row in 7..=13 {
        for col in 9..=12 {
            map.push_str(&format!("cells.\"{col},{row}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
        }
    }
    let mut game = Game::default();
    game.enemy_count_override = Some(1);
    game.seed_override = Some(7);
    game.player_row_override = Some(0);
    game.map = MapFile::from_toml_str(&map).expect("test map parses");
    game.init(W, H);
    game.intro_timer = 0.0;
    let seat = game.seat(0).expect("seat 0");
    game.place_tank(seat, cell_to_world(5, 10), Some(90.0)).expect("seat placed");
    game.debug_set_tank(0, &debug::TankPatch { minigun_ammo: Some(AMMO), ..Default::default() }).expect("seat 0");
    let slot = game.first_enemy_slot();
    game.debug_set_tank(slot, &debug::TankPatch { shells_ammo: Some(0), ..Default::default() }).expect("the enemy");
    game.debug_teleport(slot, cell_to_world(30, 20), Some(180.0)).expect("the enemy is on the field");
    game
}

/// **A tracked stream lands**: at the default nothing glances
/// (`minigun_ricochet_chance` is 0, which rolls nothing).
#[test]
fn at_the_default_no_round_glances() {
    let mut game = walled();
    let (glanced, hit) = stream_at_a_wall(&mut game, tuning().minigun_round_tick(AMMO as u32 - 10));
    assert!(hit > 200, "the stream reached the wall: {hit}");
    assert_eq!(glanced, 0);
}

/// **With a chance set, a share of the stream glances off** what it
/// strikes (`Projectile::try_glance`): about that share.
#[cfg(feature = "dev-tools")]
#[test]
fn a_share_of_the_stream_glances_off() {
    let table = std::sync::Arc::new(crate::tuning::Tuning { minigun_ricochet_chance: 0.05, ..crate::tuning::Tuning::DEFAULT });
    crate::tuning::in_force(Some(&table), || {
        let mut game = walled();
        let (glanced, hit) = stream_at_a_wall(&mut game, tuning().minigun_round_tick(AMMO as u32 - 10));
        let share = glanced as f32 / (glanced + hit) as f32;
        assert!(glanced + hit > 200, "the stream reached the wall: {glanced} + {hit}");
        assert!(share > 0.05 * 0.4 && share < 0.05 * 2.0, "{glanced} of {} glanced", glanced + hit);
    });
}

/// **A glancing round does no harm** and turns away sharply: with every
/// round glancing, the enemy on the line takes nothing - not from the
/// glance, nor from a spent round meeting it again - and each round that
/// glanced flies off well away from its line of fire.
#[cfg(feature = "dev-tools")]
#[test]
fn a_glancing_round_does_no_harm_and_turns_away() {
    let table = std::sync::Arc::new(crate::tuning::Tuning { minigun_ricochet_chance: 1.0, ..crate::tuning::Tuning::DEFAULT });
    crate::tuning::in_force(Some(&table), || {
        let mut game = game();
        let slot = game.first_enemy_slot();
        let enemy = game.tank_entity_by_slot(slot).expect("the enemy");
        with_tank_mut(&game.world, enemy, |t| t.shield_hp = 0.0);
        let mut glances = 0;
        for _ in 0..90 {
            game.debug_teleport(slot, cell_to_world(12, 10), Some(270.0)).expect("the enemy");
            step(&mut game, fire());
            for e in game.events() {
                if let Event::Ricochet { slot: 0, heading, .. } = e {
                    glances += 1;
                    // Fired east (90): a glance turns it 100-160 degrees.
                    let off = (heading - 90.0).rem_euclid(360.0);
                    let off = off.min(360.0 - off);
                    assert!(off >= 95.0, "it turned away: heading {heading}");
                }
            }
        }
        assert!(glances > 10, "the stream reached the enemy: {glances} glances");
        assert_eq!(with_tank(&game.world, enemy, |t| t.damage), 0.0, "and none of it hurt");
    });
}

/// Seat 0 facing east as in `game`, and the enemy put at `cell` facing
/// south: shielded off, so the rounds that reach it land.
fn with_enemy_at(cell: (i32, i32)) -> (Game, usize) {
    let mut game = game();
    let slot = game.first_enemy_slot();
    game.debug_teleport(slot, cell_to_world(cell.0, cell.1), Some(180.0)).expect("the enemy");
    let enemy = game.tank_entity_by_slot(slot).expect("the enemy");
    with_tank_mut(&game.world, enemy, |t| t.shield_hp = 0.0);
    (game, slot)
}

fn seat_tank<R>(game: &Game, f: impl FnOnce(&Tank) -> R) -> R {
    with_tank(&game.world, game.seat(0).expect("seat 0"), f)
}

/// Damage seat 0's rounds have done to the tank in `slot` this tick.
fn landed_on(game: &Game, slot: usize) -> f32 {
    game.events()
        .iter()
        .filter_map(|e| match e {
            Event::Hit { target: HitTarget::Enemy { slot: s }, damage, cause: HitCause::Bullet, by: Some(0), .. } if *s == slot => Some(*damage),
            _ => None,
        })
        .sum()
}

/// **The turret swings onto a tank off the hull's line.** An enemy 23°
/// off the heading, in reach and in sight: the turret takes it, slews no
/// faster than `minigun_track_slew_deg`, settles on its bearing, locks,
/// and the stream lands on it - which a gun on the hull's line never would.
#[test]
fn the_turret_swings_onto_a_tank_off_the_line() {
    let (mut game, slot) = with_enemy_at((12, 13));
    let t = tuning();
    let seat_pos = seat_tank(&game, |tk| tk.position);
    let enemy_pos = with_tank(&game.world, game.tank_entity_by_slot(slot).unwrap(), |tk| tk.position);
    let bearing = super::minigun::wrap180(super::minigun::bearing(seat_pos, enemy_pos) - 90.0);
    assert!(bearing > 15.0 && bearing < t.minigun_track_cone_deg, "the fixture puts it {bearing}° off the line");
    step(&mut game, fire());
    let first = seat_tank(&game, |tk| (tk.minigun_target, tk.minigun_track));
    assert_eq!(first.0, Some(slot), "it takes the enemy at once");
    assert!(first.1 > 0.0 && first.1 <= t.minigun_track_slew_deg * PHYSICS_FIXED_DT + 1e-3, "one tick's slew: {}", first.1);
    let mut damage = 0.0;
    for _ in 0..60 {
        step(&mut game, fire());
        damage += landed_on(&game, slot);
    }
    // The enemy drives, so its bearing is read where it stands now.
    let seat_pos = seat_tank(&game, |tk| tk.position);
    let enemy_pos = with_tank(&game.world, game.tank_entity_by_slot(slot).unwrap(), |tk| tk.position);
    let now = super::minigun::wrap180(super::minigun::bearing(seat_pos, enemy_pos) - 90.0);
    let (track, locked) = seat_tank(&game, |tk| (tk.minigun_track, tk.minigun_locked()));
    assert!((track - now).abs() < 1.5, "settled on its bearing: {track} against {now}");
    assert!(locked, "and locked on it");
    assert!(damage > 50.0, "the stream landed: {damage}");
}

/// **Not past the cone**: an enemy 45° off a seat's heading is never
/// taken, and the turret stays on the hull's line.
#[test]
fn the_turret_does_not_swing_past_its_cone() {
    let (mut game, slot) = with_enemy_at((10, 15));
    for _ in 0..20 {
        step(&mut game, fire());
        assert_eq!(landed_on(&game, slot), 0.0);
    }
    assert_eq!(seat_tank(&game, |tk| (tk.minigun_target, tk.minigun_track)), (None, 0.0));
}

/// **Only what it can see**: an enemy in the cone behind an iron wall is
/// not taken.
#[test]
fn the_turret_does_not_track_through_a_wall() {
    let mut map = MAP.to_string();
    for row in 10..=14 {
        map.push_str(&format!("cells.\"9,{row}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
    }
    let mut game = Game::default();
    game.enemy_count_override = Some(1);
    game.seed_override = Some(7);
    game.player_row_override = Some(0);
    game.map = MapFile::from_toml_str(&map).expect("test map parses");
    game.init(W, H);
    game.intro_timer = 0.0;
    let seat = game.seat(0).expect("seat 0");
    game.place_tank(seat, cell_to_world(5, 10), Some(90.0)).expect("seat placed");
    game.debug_set_tank(0, &debug::TankPatch { minigun_ammo: Some(AMMO), ..Default::default() }).expect("seat 0");
    let slot = game.first_enemy_slot();
    game.debug_set_tank(slot, &debug::TankPatch { shells_ammo: Some(0), ..Default::default() }).expect("the enemy");
    game.debug_teleport(slot, cell_to_world(12, 12), Some(180.0)).expect("the enemy");
    for _ in 0..10 {
        step(&mut game, fire());
    }
    assert_eq!(with_tank(&game.world, seat, |tk| tk.minigun_target), None);
}

/// **An enemy's round does half a seat's**, as an enemy shell does: the
/// two ranges for one chassis, side by side.
#[test]
fn an_enemys_round_does_half_a_seats() {
    use super::weapons::Projectile;
    let game = game();
    let (seat_range, enemy_range) = seat_tank(&game, |tk| {
        let seat = Bullet::spawn(tk, Owner::Player(0), 0.0).damage_range();
        let enemy = Bullet::spawn(tk, Owner::Enemy(1), 0.0).damage_range();
        (seat, enemy)
    });
    assert_eq!((enemy_range.0 * 2.0, enemy_range.1 * 2.0), seat_range);
    assert!(seat_range.0 >= 3.0 * 0.75, "a seat's round is a real hit: {seat_range:?}");
}

/// **A seat's round staggers the enemy it lands on**: for
/// `minigun_stagger_seconds` it drives at `minigun_stagger_pace` - and a
/// seat is never staggered.
#[test]
fn a_seats_round_staggers_the_enemy_it_lands_on() {
    let (mut game, slot) = with_enemy_at((12, 10));
    let enemy = game.tank_entity_by_slot(slot).expect("the enemy");
    let mut staggered = false;
    for _ in 0..30 {
        step(&mut game, fire());
        if landed_on(&game, slot) > 0.0 {
            staggered = true;
            break;
        }
    }
    assert!(staggered, "a round landed");
    let (timer, pace) = with_tank(&game.world, enemy, |tk| (tk.stagger_timer, tk.stagger_pace()));
    assert!(timer > 0.0 && timer <= tuning().minigun_stagger_seconds);
    assert_eq!(pace, tuning().minigun_stagger_pace);
    assert_eq!(seat_tank(&game, |tk| tk.stagger_pace()), 1.0, "the seat drives on at its pace");
}
