//! What holds the sound system together (docs/sound-system-prd.md section
//! 4.9): the manifest parses; every cue the code names - directly, through
//! an enumeration's exhaustive map or through a watcher - resolves in it;
//! every approved sample has its files; the walls agree with the sonic
//! hammer; nothing the simulation or the wire runs names the audio; and a
//! round's events and state turn into the commands they should.

use std::sync::Arc;

use super::bind::{self, Binding, Hears};
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
use crate::simulation::{Event, Game, HitCause, HitTarget, Input};
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

// ---- Shots and hits: who hears what (the BB-102 gap pass).

/// The plays `event` binds to, through the director, for a window that
/// plays the seats in `own`: the cues it started.
fn heard(game: &Game, event: &Event, own: &[usize]) -> Vec<String> {
    let mut a = audio();
    a.observe(game, &view_on(game), own, 0.0, true);
    a.take_commands();
    let before = a.status().recent.len();
    if let Binding::Play(plays) = bind::bind(event, game) {
        a.pending.extend(plays);
    }
    a.observe(game, &view_on(game), own, 0.0, true);
    a.status().recent[before..].to_vec()
}

fn plays(game: &Game, event: &Event) -> Vec<bind::Play> {
    match bind::bind(event, game) {
        Binding::Play(plays) => plays,
        _ => Vec::new(),
    }
}

fn resolved(cue: &str) -> String {
    manifest().resolve(cue).map(|(k, _)| k.to_string()).unwrap_or_default()
}

fn shot(target: HitTarget, damage: f32, cause: HitCause, by: Option<usize>) -> Event {
    Event::Hit { target, damage, killed: false, x: 200.0, y: 200.0, cause, by }
}

/// A missile's lock is the alarm of the seat it locked on and nobody
/// else's; the seat whose missile locked hears a soft chirp instead.
#[test]
fn a_missile_lock_warns_only_its_target_and_chirps_for_its_shooter() {
    let game = game_on(OPEN);
    let on_me = Event::MissileLocked { slot: 7, target: Some(0), x: 0.0, y: 0.0 };
    assert!(heard(&game, &on_me, &[0]).iter().any(|c| c == ids::LOCK_ON.0), "the locked seat is warned");
    assert!(heard(&game, &on_me, &[]).is_empty(), "a window not playing it hears nothing");
    let mine = Event::MissileLocked { slot: 0, target: Some(7), x: 0.0, y: 0.0 };
    let cues = heard(&game, &mine, &[0]);
    assert!(cues.iter().any(|c| c == ids::LOCK_ACQUIRED.0) && !cues.iter().any(|c| c == ids::LOCK_ON.0), "its own lock chirps: {cues:?}");
    assert!(matches!(bind::bind(&Event::MissileLocked { slot: 0, target: None, x: 0.0, y: 0.0 }, &game), Binding::Silent(_)));
}

/// The shooter's own word on a hit: a tick on the other side, a buzz on
/// its own; another seat's or an enemy's hits say nothing to this window.
#[test]
fn a_hit_ticks_for_its_shooter_and_buzzes_on_its_own_side() {
    let game = game_on(OPEN);
    let on_enemy = shot(HitTarget::Enemy { slot: 7 }, 10.0, HitCause::Shell, Some(0));
    assert!(heard(&game, &on_enemy, &[0]).iter().any(|c| c == ids::HIT_MARKER.0));
    assert!(!heard(&game, &on_enemy, &[1]).iter().any(|c| c == ids::HIT_MARKER.0), "only the shooter's window ticks");
    let on_mate = shot(HitTarget::Player { player: 1 }, 10.0, HitCause::Shell, Some(0));
    assert!(heard(&game, &on_mate, &[0]).iter().any(|c| c == ids::FRIENDLY_FIRE.0));
    let on_bong = shot(HitTarget::Bong { side: crate::bong::Side::Player }, 10.0, HitCause::Bullet, Some(0));
    assert!(plays(&game, &on_bong).iter().any(|p| p.cue == ids::FRIENDLY_FIRE.0 && p.hears == Hears::Seat(0)));
    let on_wall = shot(HitTarget::Obstacle { material: Material::Brick }, 10.0, HitCause::Shell, Some(0));
    assert!(!plays(&game, &on_wall).iter().any(|p| p.cue == ids::HIT_MARKER.0), "a tile takes no marker");
    let by_enemy = shot(HitTarget::Player { player: 0 }, 10.0, HitCause::Shell, Some(7));
    assert!(!heard(&game, &by_enemy, &[0]).iter().any(|c| c == ids::FRIENDLY_FIRE.0 || c == ids::HIT_MARKER.0));
}

/// A hit on a seat's own hull rings inside it for its player alone, and a
/// heavier hull rings lower.
#[test]
fn a_hit_on_a_seats_hull_rings_inside_it_and_a_heavy_hull_rings_lower() {
    let game = game_on(OPEN);
    let on_me = shot(HitTarget::Player { player: 0 }, 12.0, HitCause::Shell, Some(7));
    let p = plays(&game, &on_me);
    assert!(p.iter().any(|p| p.cue == "hit.shell.hull"));
    assert!(p.iter().any(|p| p.cue == ids::HULL_INSIDE.0 && p.hears == Hears::Seat(0)), "{p:?}");
    assert!(!heard(&game, &on_me, &[1]).iter().any(|c| c == ids::HULL_INSIDE.0), "another window hears only the outside");
    let laser = shot(HitTarget::Player { player: 0 }, 12.0, HitCause::Laser, Some(7));
    assert!(!plays(&game, &laser).iter().any(|p| p.cue == ids::HULL_INSIDE.0), "a beam does not ring the hull");
    let exp = crate::tuning::Tuning::DEFAULT.audio_hull_mass_pitch;
    assert!(source::hull_pitch(1.89, exp) < 1.0 && source::hull_pitch(0.77, exp) > 1.0);
}

/// A shield that takes a hit whole thrums; the same hit on bare armour
/// clangs.
#[test]
fn a_shield_that_takes_a_hit_thrums_rather_than_clangs() {
    let game = game_on(OPEN);
    let laser = shot(HitTarget::Player { player: 0 }, 0.0, HitCause::Laser, Some(7));
    assert!(plays(&game, &laser).iter().all(|p| p.cue != ids::HIT_SHIELD.0), "no shield, no thrum");
    for tank in game.world.query::<&mut crate::tank::Tank>().iter() {
        tank.shield_hp = 50.0;
    }
    let p = plays(&game, &laser);
    assert!(p.iter().any(|p| p.cue == ids::HIT_SHIELD.0) && p.iter().all(|p| !p.cue.ends_with(".hull")), "{p:?}");
    let blast = shot(HitTarget::Player { player: 0 }, 0.0, HitCause::Blast, None);
    assert!(plays(&game, &blast).iter().any(|p| p.cue == ids::HIT_SHIELD.0 && p.at == bind::At::Slot(0)), "a blast's share on a shield thrums on the hull");
}

/// A bong killed cries out, whatever killed it; the rod's crush adds no
/// impact of its own; a sonic wave booms on a hull; a round clipping a
/// drone that flies on ticks.
#[test]
fn deaths_crushes_waves_and_clipped_drones_have_their_own_sounds() {
    let game = game_on(OPEN);
    let bong = Event::Hit { target: HitTarget::Bong { side: crate::bong::Side::Enemy }, damage: 30.0, killed: true, x: 0.0, y: 0.0, cause: HitCause::Blast, by: None };
    assert!(plays(&game, &bong).iter().any(|p| p.cue == ids::BONG_DEATH.0));
    let rod = shot(HitTarget::Enemy { slot: 7 }, 100.0, HitCause::Rod, None);
    assert!(matches!(bind::bind(&rod, &game), Binding::Silent(_)), "the rod's impact sounds, not its every hull");
    let wave = shot(HitTarget::Enemy { slot: 7 }, 8.0, HitCause::Sonic, Some(0));
    assert!(plays(&game, &wave).iter().any(|p| resolved(&p.cue) == "hit.sonic.hull"));
    let well = shot(HitTarget::Enemy { slot: 7 }, 8.0, HitCause::Well, Some(0));
    assert!(plays(&game, &well).iter().any(|p| resolved(&p.cue) == "hit.well"));
    let drone = Event::AirHit { id: 3, aircraft: false, x: 0.0, y: 0.0 };
    assert!(plays(&game, &drone).iter().any(|p| p.cue == ids::AIR_HIT_DRONE.0));
}

/// A round on each material plays that material's sound, not the stone
/// one; a ricochet plays its shot off its surface; and the cooldowns key on
/// the gun, so one gun's stream thins while two guns stay their own.
#[test]
fn rounds_and_ricochets_sound_their_shot_and_surface() {
    let game = game_on(OPEN);
    for (material, cue) in [(Material::Wood, "hit.bullet.wood"), (Material::Glass, "hit.bullet.glass"), (Material::Sandbag, "hit.bullet.earth"), (Material::Pine, "hit.bullet.foliage"), (Material::Tyres, "hit.bullet.rubber"), (Material::BioSlush, "hit.bullet.slime")] {
        let p = plays(&game, &shot(HitTarget::Obstacle { material }, 2.0, HitCause::Bullet, Some(0)));
        assert_eq!(resolved(&p[0].cue), cue, "{material:?}");
    }
    assert_eq!(resolved(&plays(&game, &shot(HitTarget::Obstacle { material: Material::BioSlush }, 9.0, HitCause::Shell, Some(0)))[0].cue), "hit.shell.slime");
    let ricochet = |slot: usize, shot: HitCause, off: HitTarget| Event::Ricochet { slot, id: 1, shot, off, x: 0.0, y: 0.0, heading: 0.0 };
    let cue_of = |e: &Event| resolved(&plays(&game, e)[0].cue);
    assert_eq!(cue_of(&ricochet(0, HitCause::Shell, HitTarget::Obstacle { material: Material::Iron })), "hit.ricochet.shell");
    assert_eq!(cue_of(&ricochet(0, HitCause::Shell, HitTarget::Obstacle { material: Material::Volcano })), "hit.ricochet.shell.masonry");
    assert_eq!(cue_of(&ricochet(0, HitCause::Shell, HitTarget::Obstacle { material: Material::Barrel })), "hit.ricochet.shell.metal.barrel");
    assert_eq!(cue_of(&ricochet(0, HitCause::Bullet, HitTarget::Enemy { slot: 7 })), "hit.ricochet.bullet");
    assert_eq!(cue_of(&ricochet(0, HitCause::Bullet, HitTarget::Obstacle { material: Material::Wood })), "hit.ricochet.bullet.wood");
    let source = |e: &Event| plays(&game, e)[0].source;
    let iron = HitTarget::Obstacle { material: Material::Iron };
    assert_eq!(source(&ricochet(0, HitCause::Bullet, iron)), source(&Event::Ricochet { slot: 0, id: 9, shot: HitCause::Bullet, off: iron, x: 50.0, y: 9.0, heading: 0.0 }), "one gun's glances share a key");
    assert_ne!(source(&ricochet(0, HitCause::Bullet, iron)), source(&ricochet(7, HitCause::Bullet, iron)), "two guns' do not");
    let on = |by: usize, x: f32| Event::Hit { target: HitTarget::Wall, damage: 0.0, killed: false, x, y: 0.0, cause: HitCause::Bullet, by: Some(by) };
    assert_eq!(source(&on(0, 10.0)), source(&on(0, 90.0)), "one gun's stream shares a key wherever it lands");
    assert_ne!(source(&on(0, 10.0)), source(&on(7, 10.0)));
    let chunks = |broken: u16| Event::ChunksBroken { material: Material::Brick, variant: 0, x: 0.0, y: 0.0, broken, dx: 0.0, dy: 0.0, collapsed: false };
    assert!((plays(&game, &chunks(0b1000_0000))[0].strength - 0.7).abs() < 1e-5, "one chunk is one chunk, whichever bit it is");
}

/// A ricochet's whine follows its shot by id and lets go when the shot is
/// gone; one this world never held flies on from where it turned and fades
/// out over `audio_whine_seconds`.
#[test]
fn a_ricochets_whine_follows_the_shot_away() {
    let game = game_on(OPEN);
    let mut a = audio();
    a.observe(&game, &view_on(&game), &[0], 0.0, true);
    a.take_commands();
    a.watch.ricocheted(4242, HitCause::Shell, Vec2::new(300.0, 300.0), 90.0, a.time);
    a.observe(&game, &view_on(&game), &[0], 0.05, true);
    let first = a.take_commands();
    assert!(started(&first).iter().any(|&(_, bus, looped)| bus == Bus::Impacts.index() && looped), "the whine starts: {first:?}");
    assert!(a.status().sounds.iter().any(|s| s.cue == ids::RICOCHET_WHINE.0));
    let life = crate::tuning::tuning().audio_whine_seconds;
    for _ in 0..((life / 0.05) as usize + 4) {
        a.observe(&game, &view_on(&game), &[0], 0.05, true);
    }
    assert!(!a.status().sounds.iter().any(|s| s.cue == ids::RICOCHET_WHINE.0), "and fades out with its time");
}

/// A seat's press with too little ammo for its shot - a twin barrel's two
/// - clicks for its player, once a press; online the click is the press's,
/// and the room's copy of it is skipped.
#[test]
fn a_dry_press_clicks_for_its_seat() {
    let mut game = game_on(OPEN);
    game.intro_timer = 0.0;
    for tank in game.world.query::<&mut crate::tank::Tank>().iter() {
        tank.shells_ammo = 1;
    }
    let mut a = audio();
    a.observe_events(&game);
    a.observe(&game, &view_on(&game), &[0], 0.0, true);
    a.take_commands();
    let fire = Input::single(Intent { fire: true, ..Intent::default() });
    step(&mut game, &mut a, fire);
    assert!(game.events().iter().any(|e| matches!(e, Event::DryFired { slot: 0, .. })), "the twin's one shell is not its two: {:?}", game.events());
    assert!(a.status().recent.iter().any(|c| c == ids::DRY_FIRE.0));
    let clicks = |a: &Audio| a.status().recent.iter().filter(|c| *c == ids::DRY_FIRE.0).count();
    let held = clicks(&a);
    for _ in 0..30 {
        step(&mut game, &mut a, fire);
    }
    assert_eq!(clicks(&a), held, "a held trigger clicks once");

    let mut online = audio();
    online.set_predicted_seat(Some(0));
    online.observe(&game, &view_on(&game), &[0], 0.0, true);
    online.own_press(&game, 0, crate::net::round::PressSound::Dry);
    online.observe(&game, &view_on(&game), &[0], 0.0, true);
    assert_eq!(clicks(&online), 1, "the press clicks at once");
    step(&mut game, &mut online, Input::default());
    step(&mut game, &mut online, fire);
    assert_eq!(clicks(&online), 1, "the room's copy is skipped");
}

/// What a scrape sounds like follows the face: steel on a wall or a hull,
/// water at deep water's edge, nothing at lava's or the bong's.
#[test]
fn a_scrape_follows_the_face() {
    use crate::obstacle::Surface as Face;
    use crate::simulation::Against;
    let m = manifest();
    assert_eq!(source::scrape_cue(Against::DeepWater), Some(ids::SCRAPE_WATER.0));
    assert_eq!(source::scrape_cue(Against::Face(Face::Plain)), None);
    for face in Face::ALL.into_iter().filter(|&f| f != Face::Plain) {
        assert_eq!(source::scrape_cue(Against::Face(face)), Some(ids::SCRAPE.0));
    }
    assert_eq!(source::scrape_cue(Against::Hull), Some(ids::SCRAPE.0));
    for against in [Against::Hull, Against::DeepWater] {
        resolves(&m, source::scrape_cue(against).unwrap());
    }
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
