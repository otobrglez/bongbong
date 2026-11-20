//! The minigun (docs/BULLETS_SPEC.md §8): a rotary gun whose barrels spin
//! up before the first round, fire one unbroken stream while the trigger
//! is held, and run down after; a tap fires the least burst.

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

/// **A held trigger is one unbroken stream**: a round every
/// `minigun_round_ticks` from the first, straight through the seams
/// between one burst and the next.
#[test]
fn a_held_trigger_fires_one_unbroken_stream() {
    let mut game = game();
    let t = tuning();
    let (wait, every, burst) = (t.minigun_spinup_ticks(0.0), t.minigun_round_ticks(), t.minigun_burst_size);
    let ticks = wait + 4 * burst * every;
    let mut pulls = 0;
    for tick in 0..=ticks {
        step(&mut game, fire());
        pulls += fired_now(&game) as u32;
        let due = if tick < wait { 0 } else { (tick - wait) / every + 1 };
        assert_eq!(spent(&game), due as i32, "rounds fired by tick {tick}");
    }
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

/// **Not every round lands**: about `minigun_ricochet_chance` of them
/// glance off what they strike instead (`Projectile::try_glance`).
#[test]
fn a_share_of_the_stream_glances_off() {
    let mut game = walled();
    let (glanced, hit) = stream_at_a_wall(&mut game, (AMMO as u32 - 10) * tuning().minigun_round_ticks());
    let share = glanced as f32 / (glanced + hit) as f32;
    let chance = tuning().minigun_ricochet_chance;
    assert!(glanced + hit > 200, "the stream reached the wall: {glanced} + {hit}");
    assert!(share > chance * 0.4 && share < chance * 2.0, "{glanced} of {} glanced, against {chance}", glanced + hit);
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
