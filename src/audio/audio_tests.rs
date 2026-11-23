//! What holds the sound system together (docs/sound-system-prd.md section
//! 4.9): the manifest parses; every cue the code names - directly, through
//! an enumeration's exhaustive map or through a watcher - resolves in it;
//! every approved sample has its files; the walls agree with the sonic
//! hammer; nothing the simulation or the wire runs names the audio; and a
//! round's events and state turn into the commands they should.

use std::sync::Arc;

use super::bind::{self, Binding};
use super::cues::{Manifest, Status, ids};
use super::dsp::Command;
use super::source::{self, FireSound};
use super::space::View;
use super::watch::WATCHERS;
use super::{Audio, Bus};
use crate::ai::Intent;
use crate::map::{MapFile, Theme, Weather};
use crate::math::Vec2;
use crate::obstacle::Material;
use crate::pickup::PickupKind;
use crate::simulation::{Game, Input};
use crate::tank::{ActiveWeapon, TankKind};
use crate::tower::TowerKind;
use crate::wear::Surface;

fn manifest() -> Manifest {
    Manifest::shipped()
}

fn resolves(m: &Manifest, cue: &str) {
    assert!(m.resolve(cue).is_some(), "cue {cue} resolves in audio/cues.toml, or one of its parents does");
}

#[test]
fn the_manifest_parses() {
    let m = manifest();
    assert!(!m.cue.is_empty() && !m.sample.is_empty());
}

#[test]
fn every_cue_the_code_names_resolves() {
    let m = manifest();
    for id in ids::ALL {
        resolves(&m, id.0);
    }
}

#[test]
fn every_enumeration_maps_to_a_cue_in_the_manifest() {
    let m = manifest();
    for kind in TankKind::ALL {
        resolves(&m, source::engine_family(kind).cue());
    }
    for weapon in ActiveWeapon::ALL {
        match source::weapon_sound(weapon) {
            FireSound::Cue(cue) => resolves(&m, cue),
            FireSound::Cannon => {
                for factor in [0.75, 1.0, 1.3, 1.6] {
                    resolves(&m, source::cannon_cue(factor));
                }
            }
            FireSound::Watched(watcher) => assert!(WATCHERS.iter().any(|w| w.id == watcher), "{weapon:?} is played by a watcher that exists"),
            FireSound::Elsewhere(_) => {}
        }
    }
    for material in Material::ALL {
        let family = source::surface_family(material);
        let name = source::material_name(material);
        resolves(&m, &format!("hit.shell.{family}.{name}"));
        resolves(&m, &format!("destroy.{family}.{name}"));
    }
    for kind in PickupKind::ALL {
        resolves(&m, source::pickup_cue(kind));
    }
    for kind in TowerKind::ALL {
        resolves(&m, source::tower_cue(kind));
    }
    for sky in Weather::ALL {
        if let Some(cue) = source::sky_bed(sky) {
            resolves(&m, cue);
        }
    }
    for theme in Theme::ALL {
        resolves(&m, source::place_bed(theme, None));
        for biome in crate::biome::Biome::ALL {
            resolves(&m, source::place_bed(theme, Some(biome)));
        }
    }
    for surface in [Surface::Grass, Surface::Road, Surface::Sand, Surface::Ford, Surface::Deep, Surface::Ice] {
        if let Some(cue) = source::surface_layer(surface) {
            resolves(&m, cue);
        }
    }
    for what in crate::well::Swallow::ALL {
        resolves(&m, source::swallow_cue(what));
    }
    for kind in crate::bong::BongKind::ALL {
        assert!(source::bong_pitch(kind) > 0.0);
    }
}

#[test]
fn every_watcher_cue_resolves() {
    let m = manifest();
    for w in WATCHERS {
        for cue in w.cues {
            resolves(&m, cue);
        }
    }
}

#[test]
fn every_list_of_variants_is_whole() {
    // Each list is what an exhaustive map is walked over; a variant missing
    // from it would never be checked.
    let mut materials: Vec<&str> = Material::ALL.iter().map(|&m| source::material_name(m)).collect();
    materials.sort_unstable();
    materials.dedup();
    assert_eq!(materials.len(), Material::ALL.len());
    for weapon in ActiveWeapon::ALL {
        assert_eq!(ActiveWeapon::parse(weapon.name()), Some(weapon));
    }
}

#[test]
fn sound_transmission_agrees_with_blocks_sound() {
    for m in Material::ALL {
        let loss = m.sound_transmission();
        assert!(loss <= 0.0);
        assert_eq!(m.blocks_sound(), loss <= -6.0, "{m:?}: blocks_sound and a loss of at least 6 dB go together");
    }
}

#[test]
fn every_cue_plays_samples_on_its_own_bus_kind() {
    let m = manifest();
    for (id, cue) in &m.cue {
        assert!(!cue.layers.is_empty(), "{id} plays something");
        for layer in &cue.layers {
            let sample = &m.sample[&layer.sample];
            if cue.kind.looped() {
                assert!(sample.looped, "{id} loops, so its sample {} is made as a seamless loop", layer.sample);
            }
        }
    }
}

#[test]
fn every_approved_sample_has_its_files() {
    let m = manifest();
    for (id, sample) in &m.sample {
        if sample.status != Status::Approved {
            continue;
        }
        assert!(!sample.takes.is_empty(), "{id} is approved with no take");
        for take in &sample.takes {
            let path = format!("{}/{}/{}", env!("CARGO_MANIFEST_DIR"), super::cues::ROOT, take.file);
            let bytes = std::fs::read(&path).unwrap_or_else(|_| panic!("{id}: {path} exists"));
            let pcm = super::dsp::decode::ogg(&bytes).unwrap_or_else(|e| panic!("{id}: {path} decodes: {e}"));
            assert_eq!(pcm.rate, super::dsp::RATE, "{id}: {path} is at the engine's rate");
            assert!(pcm.seconds() > 0.05 && pcm.seconds() < 31.0, "{id}: {path} is {} s", pcm.seconds());
            assert!(pcm.peak() <= 1.0, "{id}: {path} peaks at {}", pcm.peak());
        }
    }
}

#[test]
fn the_simulation_and_the_wire_never_name_the_audio() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    let mut stack = vec![root.join("simulation"), root.join("net")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("readable") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("readable");
            if source.contains("crate::audio") || source.contains("use crate::audio") {
                offenders.push(path);
            }
        }
    }
    assert!(offenders.is_empty(), "these files name the audio: {offenders:?}");
}

// ---- A round's events and state, through the director.

const W: f32 = 1280.0;
const H: f32 = 720.0;

fn game_on(map: &str) -> Game {
    let mut game = Game::default();
    game.enemy_count_override = Some(0);
    game.seed_override = Some(7);
    game.player_row_override = Some(1);
    game.map = MapFile::from_toml_str(map).expect("test map parses");
    game.init(W, H);
    game
}

const OPEN: &str = r#"
version = 1
tanks = 0
size = [24, 14]
cells."2,2" = { kind = "bong" }
cells."6,6" = { kind = "start" }
"#;

fn audio() -> Audio {
    Audio::new(Arc::new(Manifest::shipped()), true, |_| None)
}

fn view_on(game: &Game) -> Vec<View> {
    let seat = bind::tank_in(game, 0).map(|t| t.position);
    vec![View { center: seat.unwrap_or(Vec2::new(400.0, 300.0)), half_width: 544.0, seat }]
}

fn started(commands: &[Command]) -> Vec<(u32, usize, bool)> {
    commands.iter().filter_map(|c| if let Command::Start { id, bus, looped, .. } = c { Some((*id, *bus, *looped)) } else { None }).collect()
}

fn step(game: &mut Game, audio: &mut Audio, input: Input) -> Vec<Command> {
    game.update(input, crate::PHYSICS_FIXED_DT, W, H);
    audio.observe_events(game);
    let views = view_on(game);
    audio.observe(game, &views, &[0], crate::PHYSICS_FIXED_DT, true);
    audio.take_commands()
}

#[test]
fn a_shot_starts_the_cannon_once() {
    let mut game = game_on(OPEN);
    let mut a = audio();
    a.observe_events(&game);
    a.observe(&game, &view_on(&game), &[0], 0.0, true);
    a.take_commands();
    let fire = Input::single(Intent { fire: true, ..Intent::default() });
    let mut commands = step(&mut game, &mut a, fire);
    for _ in 0..3 {
        commands.extend(step(&mut game, &mut a, Input::default()));
    }
    let weapons: Vec<_> = started(&commands).into_iter().filter(|&(_, bus, looped)| bus == Bus::Weapons.index() && !looped).collect();
    assert!(!weapons.is_empty(), "the shot sounds: {commands:?}");
    assert!(a.status().recent.iter().any(|c| c == "weapon.shell.fire"), "it is the cannon: {:?}", a.status().recent);
    // Reading the same step twice plays nothing new.
    a.observe_events(&game);
    a.observe(&game, &view_on(&game), &[0], 0.0, true);
    assert!(started(&a.take_commands()).iter().all(|&(_, bus, _)| bus != Bus::Weapons.index()));
}

/// A twin gun's two shots are two sounds: the second when its projectile
/// leaves, on the other take, a shade lower, the two panned to their own
/// barrels across a gun aimed up and stacked on one aimed sideways.
#[test]
fn a_twin_guns_two_shots_are_two_sounds() {
    let t = crate::tuning::tuning();
    let first = bind::Play::new("weapon.shell.fire", bind::At::Slot(0)).pitch(1.0).source(0);
    let up = bind::twin_shots(first.clone(), 0.0, 10);
    assert_eq!(up.len(), 2);
    let (a, b) = (&up[0], &up[1]);
    assert_eq!((a.after, b.after), (0.0, t.tank_twin_shot_delay_seconds));
    assert_ne!(a.take.map(|k| k % 2), b.take.map(|k| k % 2), "neighbouring takes");
    assert!(b.pitch < a.pitch && a.source != b.source);
    assert!(a.pan < 0.0 && b.pan > 0.0 && (b.pan - a.pan - 2.0 * t.audio_twin_spread).abs() < 1e-4, "split across: {} {}", a.pan, b.pan);
    let sideways = bind::twin_shots(first, 90.0, 10);
    assert!(sideways.iter().all(|p| p.pan.abs() < 1e-4), "stacked when the barrels lie along the view");

    // Through the director: a twin chassis's shell starts two cannon
    // voices, the second the barrel gap later and to the right of the first.
    let mut game = game_on(OPEN);
    assert!(bind::tank_in(&game, 0).is_some_and(|tank| tank.twin_gun()), "row 1 is a twin chassis");
    let mut a = audio();
    a.observe_events(&game);
    a.observe(&game, &view_on(&game), &[0], 0.0, true);
    a.take_commands();
    let commands = step(&mut game, &mut a, Input::single(Intent { fire: true, ..Intent::default() }));
    let shots: Vec<(f32, f32)> = commands
        .iter()
        .filter_map(|c| match c {
            Command::Start { bus, looped: false, params, delay, .. } if *bus == Bus::Weapons.index() => Some((*delay, params.pan)),
            _ => None,
        })
        .collect();
    let early = shots.iter().filter(|(d, _)| *d < t.tank_twin_shot_delay_seconds * 0.5).count();
    let late: Vec<_> = shots.iter().filter(|(d, _)| (*d - t.tank_twin_shot_delay_seconds).abs() < 1e-3).collect();
    assert!(early >= 1 && !late.is_empty(), "two shots, the second a barrel gap later: {shots:?}");
    let first_pan = shots.iter().find(|(d, _)| *d < 1e-3).map(|(_, p)| *p).unwrap_or(0.0);
    assert!(late.iter().all(|(_, p)| *p > first_pan), "the second barrel to the right of the first: {shots:?}");
}

#[test]
fn a_seat_has_an_engine_that_revs_with_speed() {
    let mut game = game_on(OPEN);
    let mut a = audio();
    a.observe_events(&game);
    let commands = {
        let views = view_on(&game);
        a.observe(&game, &views, &[0], 1.0 / 60.0, true);
        a.take_commands()
    };
    assert!(started(&commands).iter().any(|&(_, bus, looped)| bus == Bus::Vehicles.index() && looped), "an engine loop starts: {commands:?}");
    let pitch_of = |cmds: &[Command]| {
        cmds.iter()
            .filter_map(|c| match c {
                Command::Update { params, .. } => Some(params.pitch),
                _ => None,
            })
            .fold(0.0f32, f32::max)
    };
    let idle = pitch_of(&step(&mut game, &mut a, Input::default()));
    let drive = Input::single(Intent { move_dir: Some(crate::tank::Dir::Right), ..Intent::default() });
    let mut last = Vec::new();
    for _ in 0..60 {
        last = step(&mut game, &mut a, drive);
    }
    assert!(pitch_of(&last) > idle, "driving revs the engine: idle {idle}, driving {}", pitch_of(&last));
}

#[test]
fn every_event_a_round_logs_has_a_decision() {
    // Not a compile check - that is the exhaustive match - but a guard that
    // binding a real round's events never panics and names resolvable cues.
    let m = manifest();
    let mut game = game_on(OPEN);
    let fire = Input::single(Intent { fire: true, ..Intent::default() });
    for i in 0..120 {
        game.update(if i % 20 == 0 { fire } else { Input::default() }, crate::PHYSICS_FIXED_DT, W, H);
        for e in game.events() {
            if let Binding::Play(plays) = bind::bind(e, &game) {
                for p in plays {
                    resolves(&m, &p.cue);
                }
            }
        }
    }
}

#[test]
fn a_cue_at_its_polyphony_steals_rather_than_piling_up() {
    let mut game = game_on(OPEN);
    let mut a = audio();
    a.observe_events(&game);
    a.observe(&game, &view_on(&game), &[0], 0.0, true);
    a.take_commands();
    let at = bind::tank_in(&game, 0).map(|t| t.position).unwrap();
    for i in 0..20 {
        a.pending.push(bind::Play::new("explosion.wreck", bind::At::Point(at)).source(i));
    }
    a.observe(&game, &view_on(&game), &[0], 0.0, true);
    let live = a.status().sounds.iter().filter(|s| s.cue == "explosion.wreck").count();
    let poly = Manifest::shipped().cue["explosion.wreck"].polyphony as usize;
    assert!(live <= poly, "{live} wrecks play at once, polyphony {poly}");
    // The silent step keeps the game honest about time.
    step(&mut game, &mut a, Input::default());
}

// ---- Weapon feedback and warnings, through the watchers.

/// One frame of the watchers for the seats `own` at `now` on the
/// director's clock: the loops they wish for and the sounds they ask for.
fn watch(w: &mut super::watch::Watchers, game: &Game, own: &[usize], now: f64) -> (Vec<super::watch::Wish>, Vec<bind::Play>) {
    let t = crate::tuning::tuning();
    let (mut wishes, mut plays) = (Vec::new(), Vec::new());
    w.sample(game, own, now, &t, &mut wishes, &mut plays);
    (wishes, plays)
}

fn tank_mut(game: &mut Game, slot: usize, f: impl FnOnce(&mut crate::tank::Tank)) {
    let tank = game.world.query_mut::<&mut crate::tank::Tank>().into_iter().find(|t| t.owner_slot() == slot).expect("a tank in the slot");
    f(tank);
}

/// An enemy parked at `at` facing `rotation`, unarmed, unshielded: its slot.
fn parked_enemy(game: &mut Game, at: Vec2, rotation: f32) -> usize {
    let slot = game.debug_spawn_enemy(at, Some(1), Some(crate::ai::Role::Player)).expect("spawns");
    let entity = game.tank_entity_by_slot(slot).expect("exists");
    game.place_tank(entity, at, Some(rotation)).expect("placed");
    tank_mut(game, slot, |t| {
        t.speed_scale = 0.0;
        t.disarm();
        t.shells_ammo = 0;
        t.shield_hp = 0.0;
        t.shield_timer = 0.0;
    });
    slot
}

fn count(plays: &[bind::Play], cue: super::cues::CueId) -> usize {
    plays.iter().filter(|p| p.cue == cue.0).count()
}

/// A seat standing in another tank's charging rail lane hears lock beeps,
/// quicker as the charge fills; out of the lane, or as a seat another
/// window plays, it hears none.
#[test]
fn a_seat_in_a_charging_rails_lane_hears_lock_beeps_quicker_as_it_fills() {
    use crate::tank::{ActiveWeapon, Charge};
    let mut game = game_on(OPEN);
    let seat = bind::tank_in(&game, 0).map(|t| t.position).expect("a seat");
    let rail = parked_enemy(&mut game, seat + Vec2::new(256.0, 0.0), 270.0);
    let beeps = |game: &mut Game, held: f32, own: &[usize]| {
        tank_mut(game, rail, |t| t.charge = Some(Charge::new(ActiveWeapon::GaussRail, held)));
        let mut w = super::watch::Watchers::default();
        (0..60)
            .map(|i| {
                let plays = watch(&mut w, game, own, i as f64 / 60.0).1;
                plays.iter().filter(|p| p.cue == ids::LOCK_RAIL.0 && p.at == bind::At::Slot(0)).count()
            })
            .sum::<usize>()
    };
    let slow = beeps(&mut game, 0.1, &[0]);
    let fast = beeps(&mut game, 10.0, &[0]);
    assert!(slow >= 2 && fast > 2 * slow, "quicker as it fills: {slow} a second at the start, {fast} at full");
    assert_eq!(beeps(&mut game, 1.0, &[]), 0, "a seat this window does not play hears nothing");
    let entity = game.tank_entity_by_slot(0).expect("the seat");
    game.place_tank(entity, seat + Vec2::new(0.0, 128.0), None).expect("placed");
    assert_eq!(beeps(&mut game, 1.0, &[0]), 0, "out of the lane, no beeps");
}

/// A drone locked on a seat is a thin tone for that seat, higher and
/// louder the nearer it comes, gone once the lock is.
#[test]
fn a_drones_lock_on_a_seat_is_a_tone_rising_as_it_nears_and_gone_with_the_lock() {
    use crate::fpv::{Drone, DroneLock, DroneStage};
    let mut game = game_on(OPEN);
    let seat = bind::tank_in(&game, 0).map(|t| t.position).expect("a seat");
    let lock = DroneLock::Tank { entity: game.tank_entity_by_slot(0).expect("the seat"), slot: 0 };
    let mut drone = Drone::launch(seat + Vec2::new(400.0, 0.0), Vec2::new(-1.0, 0.0), 0, crate::shell::Owner::Enemy(9), lock, seat);
    drone.id = 900;
    drone.stage = DroneStage::Cruise;
    let entity = game.world.spawn((drone,));
    let tone = |game: &Game, own: &[usize]| {
        let mut w = super::watch::Watchers::default();
        watch(&mut w, game, own, 0.0).0.into_iter().find(|wish| wish.cue == ids::LOCK_DRONE.0)
    };
    let far = tone(&game, &[0]).expect("a tone while the lock holds");
    assert!(far.own && tone(&game, &[]).is_none(), "the locked seat's alone");
    game.world.get::<&mut Drone>(entity).expect("the drone").ground = seat + Vec2::new(60.0, 0.0);
    let near = tone(&game, &[0]).expect("still locked");
    assert!(near.pitch > far.pitch && near.gain > far.gain, "it rises as the drone nears: {far:?} then {near:?}");
    game.world.get::<&mut Drone>(entity).expect("the drone").lock = DroneLock::None;
    assert!(tone(&game, &[0]).is_none(), "the lock lost, the tone stops");
}

/// A seat that wrecks an enemy hears a sting; a seat another window plays
/// does not.
#[test]
fn a_seats_kill_is_confirmed_with_a_sting() {
    let mut game = game_on(OPEN);
    let seat = bind::tank_in(&game, 0).map(|t| t.position).expect("a seat");
    let entity = game.tank_entity_by_slot(0).expect("the seat");
    game.place_tank(entity, seat, Some(90.0)).expect("placed");
    let victim = parked_enemy(&mut game, seat + Vec2::new(128.0, 0.0), 270.0);
    tank_mut(&mut game, victim, |t| t.damage = crate::MAX_DAMAGE - 1.0);
    let (mut mine, mut theirs) = (super::watch::Watchers::default(), super::watch::Watchers::default());
    let (mut stings, mut others) = (0, 0);
    let fire = Input::single(Intent { fire: true, ..Intent::default() });
    for i in 0..90 {
        game.update(if i == 0 { fire } else { Input::default() }, crate::PHYSICS_FIXED_DT, W, H);
        stings += count(&watch(&mut mine, &game, &[0], i as f64 / 60.0).1, ids::KILL);
        others += count(&watch(&mut theirs, &game, &[], i as f64 / 60.0).1, ids::KILL);
    }
    assert_eq!(game.round_stats().by_seat[0], 1, "the seat wrecked it");
    assert_eq!((stings, others), (1, 0), "one sting, for the seat's own window");
}

/// A volley is heard tube by tube, each missile leaving on its own frame,
/// and each flies with its motor.
#[test]
fn a_missile_volley_is_heard_tube_by_tube_each_with_its_motor() {
    let mut game = game_on(OPEN);
    tank_mut(&mut game, 0, |t| {
        t.disarm();
        t.missile_ammo = 4;
    });
    let mut w = super::watch::Watchers::default();
    watch(&mut w, &game, &[0], 0.0);
    let fire = Input::single(Intent { fire: true, ..Intent::default() });
    let (mut launches, mut motors) = (Vec::new(), 0);
    for i in 0..30 {
        game.update(if i == 0 { fire } else { Input::default() }, crate::PHYSICS_FIXED_DT, W, H);
        let (wishes, plays) = watch(&mut w, &game, &[0], (i + 1) as f64 / 60.0);
        launches.extend(std::iter::repeat_n(i, count(&plays, ids::MISSILE_LAUNCH)));
        motors = motors.max(wishes.iter().filter(|wish| wish.cue == ids::MISSILE_MOTOR.0).count());
    }
    assert_eq!(launches.len(), 4, "one launch a tube: {launches:?}");
    assert!(launches.windows(2).all(|p| p[1] > p[0]), "each on its own frame, at the volley's spacing: {launches:?}");
    assert_eq!(motors, 4, "four motors in the air");
}

/// A rail's charge pings once as it reaches full, crackles once
/// overcharged, and beeps its own seat to let go in its last moments.
#[test]
fn a_rails_charge_pings_when_full_crackles_overcharged_and_beeps_its_seat_before_it_vents() {
    let t = crate::tuning::tuning();
    let mut game = game_on(OPEN);
    tank_mut(&mut game, 0, |tk| {
        tk.disarm();
        tk.gauss_slugs = 2;
    });
    let mut w = super::watch::Watchers::default();
    watch(&mut w, &game, &[0], 0.0);
    let hold = Input::single(Intent { fire: true, ..Intent::default() });
    let (mut ready, mut alarm, mut crackle) = (Vec::new(), Vec::new(), None);
    let vent = ((t.gauss_charge_seconds + t.gauss_hold_seconds) * 60.0) as usize;
    for i in 0..vent + 20 {
        game.update(hold, crate::PHYSICS_FIXED_DT, W, H);
        let (wishes, plays) = watch(&mut w, &game, &[0], (i + 1) as f64 / 60.0);
        ready.extend(std::iter::repeat_n(i, count(&plays, ids::GAUSS_READY)));
        alarm.extend(std::iter::repeat_n(i, count(&plays, ids::VENT_ALARM)));
        if crackle.is_none() && wishes.iter().any(|wish| wish.cue == ids::GAUSS_OVERCHARGED.0) {
            crackle = Some(i);
        }
    }
    let full = (t.gauss_charge_seconds * 60.0) as usize;
    assert!(ready.len() == 1 && ready[0].abs_diff(full) <= 2, "one ping at full, frame {full}: {ready:?}");
    assert!(crackle.is_some_and(|c| c > ready[0]), "the crackle after full: {crackle:?}");
    let warn = vent - (t.audio_vent_warn_s * 60.0) as usize;
    assert!(alarm.len() >= 4 && alarm.iter().all(|&i| i + 2 >= warn && i <= vent + 1), "beeps in its last moments, frames {warn}..{vent}: {alarm:?}");
}

/// A rod's reticle is its uplink's tone and a tick a cell, not the rail's
/// whine; put down without a call it aborts rather than venting.
#[test]
fn a_rods_reticle_hums_its_uplink_ticks_its_cells_and_aborts_when_put_down() {
    use crate::tank::{ActiveWeapon, Charge, ChargeEnd};
    let mut game = game_on(OPEN);
    let cue_of = |game: &Game, weapon: ActiveWeapon, end: ChargeEnd| match bind::bind(&crate::simulation::Event::ChargeEnded { slot: 0, weapon: weapon.name(), end }, game) {
        Binding::Play(plays) => plays[0].cue.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(cue_of(&game, ActiveWeapon::RodFromGod, ChargeEnd::Fizzled), "weapon.rod_from_god.cancel");
    assert_eq!(cue_of(&game, ActiveWeapon::RodFromGod, ChargeEnd::Vented), "weapon.rod_from_god.cancel.timeout");
    assert_eq!(cue_of(&game, ActiveWeapon::GaussRail, ChargeEnd::Vented), "weapon.gauss_rail.vent");
    tank_mut(&mut game, 0, |t| {
        t.charge = Some(Charge::new(ActiveWeapon::RodFromGod, 0.5));
        t.reticle = Some(crate::rod::Reticle::new((6, 3)));
    });
    let mut w = super::watch::Watchers::default();
    let (wishes, _) = watch(&mut w, &game, &[0], 0.0);
    assert!(wishes.iter().any(|wish| wish.cue == ids::ROD_UPLINK.0) && !wishes.iter().any(|wish| wish.cue == ids::CHARGE_GAUSS.0), "{wishes:?}");
    tank_mut(&mut game, 0, |t| t.reticle = Some(crate::rod::Reticle::new((7, 3))));
    assert_eq!(count(&watch(&mut w, &game, &[0], 0.1).1, ids::ROD_RETICLE), 1, "a tick for the cell it stepped");
    assert_eq!(count(&watch(&mut w, &game, &[0], 0.2).1, ids::ROD_RETICLE), 0, "none while it rests");
}

/// A grenade's fuse beeps with its lamp, far quicker in its last second
/// than its first.
#[test]
fn a_grenades_fuse_beeps_with_its_lamp_quicker_as_it_nears_going_off() {
    use crate::grenade::Grenade;
    let t = crate::tuning::tuning();
    let mut game = game_on(OPEN);
    let mut grenade = Grenade::launch(Vec2::new(300.0, 300.0), Vec2::new(0.0, -1.0), Vec2::new(0.0, 0.0), crate::shell::Owner::Enemy(9));
    grenade.id = 901;
    let entity = game.world.spawn((grenade,));
    let mut w = super::watch::Watchers::default();
    let frames = (t.grenade_fuse_seconds * 60.0) as usize;
    let mut beeps = Vec::new();
    for i in 0..frames {
        game.world.get::<&mut Grenade>(entity).expect("the grenade").fuse = t.grenade_fuse_seconds - i as f32 / 60.0;
        beeps.extend(std::iter::repeat_n(i, count(&watch(&mut w, &game, &[0], i as f64 / 60.0).1, ids::GRENADE_BEEP)));
    }
    let first = beeps.iter().filter(|&&i| i < 60).count();
    let last = beeps.iter().filter(|&&i| i >= frames - 60).count();
    assert!(first >= 1 && last > 2 * first, "{first} beeps in its first second, {last} in its last");
}

/// A seat's minigun spins up from rest at the press; its flamethrower
/// lights, snaps shut when let go and sputters when it runs dry.
#[test]
fn a_minigun_spins_up_from_rest_and_a_flamethrower_lights_shuts_and_runs_dry() {
    let mut game = game_on(OPEN);
    tank_mut(&mut game, 0, |t| {
        t.disarm();
        t.minigun_ammo = 40;
    });
    let mut w = super::watch::Watchers::default();
    watch(&mut w, &game, &[0], 0.0);
    let fire = Input::single(Intent { fire: true, ..Intent::default() });
    let mut spin_ups = Vec::new();
    for i in 0..20 {
        game.update(if i == 0 { fire } else { Input::default() }, crate::PHYSICS_FIXED_DT, W, H);
        spin_ups.extend(std::iter::repeat_n(i, count(&watch(&mut w, &game, &[0], (i + 1) as f64 / 60.0).1, ids::MINIGUN_SPIN_UP)));
    }
    assert_eq!(spin_ups, vec![0], "one spin-up, at the press");

    let mut game = game_on(OPEN);
    tank_mut(&mut game, 0, |t| {
        t.disarm();
        t.flame_fuel = 0.5;
    });
    let mut w = super::watch::Watchers::default();
    watch(&mut w, &game, &[0], 0.0);
    let mut heard = Vec::new();
    for i in 0..60 {
        // Held for ten frames, let go for five, then held till it is dry.
        let held = !(10..15).contains(&i);
        game.update(if held { fire } else { Input::default() }, crate::PHYSICS_FIXED_DT, W, H);
        let plays = watch(&mut w, &game, &[0], (i + 1) as f64 / 60.0).1;
        for cue in [ids::FLAME_START, ids::FLAME_STOP, ids::FLAME_DRY] {
            heard.extend(std::iter::repeat_n(cue.0, count(&plays, cue)));
        }
    }
    assert_eq!(heard, vec![ids::FLAME_START.0, ids::FLAME_STOP.0, ids::FLAME_START.0, ids::FLAME_DRY.0], "lit, shut, lit, dry");
}

/// A seat's special going offline is static, coming back a chime; spent,
/// an empty clunk and the cannon re-arming; its magazine racks a shell as
/// it gets back to a shot's cost and clunks once it is full.
#[test]
fn a_seats_special_and_magazine_are_heard_going_out_and_coming_back() {
    let t = crate::tuning::tuning();
    let mut game = game_on(OPEN);
    let cost = if bind::tank_in(&game, 0).is_some_and(|tank| tank.twin_gun()) { 2 } else { 1 };
    tank_mut(&mut game, 0, |tk| {
        tk.disarm();
        tk.laser_charges = 2;
        tk.disabled = 2.0;
    });
    let mut w = super::watch::Watchers::default();
    let (wishes, _) = watch(&mut w, &game, &[0], 0.0);
    assert!(wishes.iter().any(|wish| wish.cue == ids::OFFLINE.0 && wish.own), "static while it is out");
    tank_mut(&mut game, 0, |tk| tk.disabled = 0.0);
    assert_eq!(count(&watch(&mut w, &game, &[0], 0.1).1, ids::ONLINE), 1, "a chime when it is back");
    tank_mut(&mut game, 0, |tk| tk.laser_charges = 0);
    let plays = watch(&mut w, &game, &[0], 0.2).1;
    assert_eq!(count(&plays, ids::SPENT), 1, "an empty clunk: {plays:?}");
    assert!(plays.iter().any(|p| p.cue == ids::SHELL_RELOAD.0 && (p.after - t.audio_rearm_s).abs() < 1e-6), "then the cannon re-arms: {plays:?}");
    tank_mut(&mut game, 0, |tk| tk.shells_ammo = cost - 1);
    watch(&mut w, &game, &[0], 0.3);
    tank_mut(&mut game, 0, |tk| tk.shells_ammo = cost);
    assert_eq!(count(&watch(&mut w, &game, &[0], 0.4).1, ids::SHELL_RELOAD), 1, "a shell racked: back to a shot's cost");
    tank_mut(&mut game, 0, |tk| tk.shells_ammo = t.max_shells - 1);
    watch(&mut w, &game, &[0], 0.5);
    tank_mut(&mut game, 0, |tk| tk.shells_ammo = t.max_shells);
    assert_eq!(count(&watch(&mut w, &game, &[0], 0.6).1, ids::SHELL_FULL), 1, "a firmer clunk when full");
    // Another window's seat hears none of it.
    let mut theirs = super::watch::Watchers::default();
    tank_mut(&mut game, 0, |tk| tk.laser_charges = 1);
    watch(&mut theirs, &game, &[], 0.0);
    tank_mut(&mut game, 0, |tk| tk.laser_charges = 0);
    assert_eq!(count(&watch(&mut theirs, &game, &[], 0.1).1, ids::SPENT), 0);
}

/// A rail slug passing close by a seat cracks past, louder the closer;
/// not one that went through it, nor its own, nor one far off.
#[test]
fn a_rail_passing_close_by_a_seat_cracks_past_unless_it_struck_or_was_its_own() {
    use super::watch::{Beam, Watchers};
    let game = game_on(OPEN);
    let seat = bind::tank_in(&game, 0).map(|t| t.position).expect("a seat");
    let past = |shooter: Option<usize>, struck: Vec<usize>, off: f32| {
        let mut w = Watchers::default();
        w.beam(Beam { from: seat + Vec2::new(-300.0, off), to: seat + Vec2::new(300.0, off), shooter, cue: ids::FLYBY_RAIL.0, struck });
        watch(&mut w, &game, &[0], 0.0).1.into_iter().filter(|p| p.cue == ids::FLYBY_RAIL.0).collect::<Vec<_>>()
    };
    let close = past(Some(5), vec![], 8.0);
    let farther = past(Some(5), vec![], 36.0);
    assert!(close.len() == 1 && farther.len() == 1 && farther[0].strength < close[0].strength, "{close:?} {farther:?}");
    assert!(past(Some(5), vec![], 200.0).is_empty(), "far off");
    assert!(past(Some(5), vec![0], 8.0).is_empty(), "it went through the seat");
    assert!(past(Some(0), vec![], 8.0).is_empty(), "the seat's own");
    let slug = crate::simulation::Event::RailSlug {
        slot: 5,
        seat: crate::net::wire::NO_SEAT,
        leg: 0,
        x0: 0.0,
        y0: 0.0,
        x1: 100.0,
        y1: 0.0,
        portal: false,
        overcharged: false,
        pierced: Vec::new(),
    };
    let beam = Beam::of(&slug, &[]).expect("a slug is a beam");
    assert_eq!((beam.shooter, beam.cue, beam.struck.len()), (Some(5), ids::FLYBY_RAIL.0, 0));
}

/// Each weapon crate arms with its family's sound, and each calibre's
/// cannon is its own.
#[test]
fn each_weapon_crate_and_each_calibre_has_its_own_sound() {
    let m = manifest();
    let cue = |id: &str| m.resolve(id).map(|(k, _)| k.to_string()).expect("resolves");
    assert_eq!(cue(source::pickup_cue(PickupKind::Laser)), "pickup.weapon.energy");
    assert_eq!(cue(source::pickup_cue(PickupKind::Flamethrower)), "pickup.weapon.fuel");
    assert_eq!(cue(source::pickup_cue(PickupKind::GravityWell)), "pickup.weapon.field");
    assert_eq!(cue(source::pickup_cue(PickupKind::RodFromGod)), "pickup.weapon.uplink");
    assert_eq!(cue(source::pickup_cue(PickupKind::Minigun)), "pickup.weapon");
    let tiers: Vec<String> = [0.75, 1.0, 1.3, 1.6].iter().map(|&f| cue(source::cannon_cue(f))).collect();
    assert_eq!(tiers, ["weapon.shell.fire.light", "weapon.shell.fire", "weapon.shell.fire.heavy", "weapon.shell.fire.super"]);
}

#[test]
fn an_online_press_sounds_once_at_the_press_and_never_again_from_the_room() {
    let mut game = game_on(OPEN);
    let mut a = audio();
    a.observe_events(&game);
    a.observe(&game, &view_on(&game), &[0], 0.0, true);
    a.take_commands();
    // The client drew its own shot at the press and sounds it there.
    a.set_predicted_seat(Some(0));
    a.own_press(&game, 0, crate::net::round::PressSound::Shell);
    a.observe(&game, &view_on(&game), &[0], 0.0, true);
    let at_press = started(&a.take_commands()).into_iter().filter(|&(_, bus, _)| bus == Bus::Weapons.index()).count();
    assert!(at_press >= 1, "the press sounds at once");
    // The room's `Fired` for that seat arrives later: it is not played again.
    let fire = Input::single(Intent { fire: true, ..Intent::default() });
    let mut later = step(&mut game, &mut a, fire);
    for _ in 0..3 {
        later.extend(step(&mut game, &mut a, Input::default()));
    }
    let again = started(&later).into_iter().filter(|&(_, bus, looped)| bus == Bus::Weapons.index() && !looped).count();
    assert_eq!(again, 0, "the room's copy of the shot is skipped: {later:?}");
}
