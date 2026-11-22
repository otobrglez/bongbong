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

// ---- Water, mud and ground (the `ground`, `water` and `landing` watchers).

/// Every ground sound the maps can ask for resolves - the surface's layer
/// under every sky and theme, wet or dry, in lava or out; a slip and a
/// skid on every surface - and the sky and theme change what the tracks say.
#[test]
fn every_ground_sound_resolves() {
    use crate::wear::{Look, Sky};
    let m = manifest();
    let surfaces = [Surface::Grass, Surface::Road, Surface::Sand, Surface::Ford, Surface::Deep, Surface::Ice];
    for theme in Theme::ALL {
        for sky in [Sky::Dry, Sky::Rain, Sky::Snow] {
            let look = Look { theme, sky };
            for surface in surfaces {
                for wet in [0.0, 1.0] {
                    for lava in [false, true] {
                        if let Some(cue) = source::ground_layer(surface, look, wet, lava) {
                            resolves(&m, cue);
                        }
                        resolves(&m, source::skid_cue(surface, look, wet, lava));
                    }
                    resolves(&m, source::slip_cue(surface, crate::wear::muddy(look, surface, wet)));
                }
            }
        }
    }
    let rain = Look { theme: Theme::Grass, sky: Sky::Rain };
    assert_eq!(source::ground_layer(Surface::Grass, rain, 0.0, false), Some("vehicle.ground.mud"));
    assert_eq!(source::ground_layer(Surface::Sand, rain, 0.0, false), Some("vehicle.ground.sand"), "rain leaves sand sand");
    assert_eq!(source::ground_layer(Surface::Road, Look::default(), 0.5, false), Some("vehicle.ground.mud"), "tracks wet from a ford");
    assert_eq!(source::ground_layer(Surface::Grass, Look { theme: Theme::Desert, sky: Sky::Dry }, 0.0, false), Some("vehicle.ground.dust"));
    assert_eq!(source::ground_layer(Surface::Road, Look { theme: Theme::Grass, sky: Sky::Snow }, 0.0, false), Some("vehicle.ground.snow"));
    assert_eq!(source::ground_layer(Surface::Deep, Look::default(), 0.0, true), Some("vehicle.ground.lava"));
    assert_eq!(source::slip_cue(Surface::Ice, false), "vehicle.slip.ice");
    assert_eq!(source::skid_cue(Surface::Grass, Look::default(), 0.0, false), "vehicle.skid.grass");
}

/// `OPEN` with `extra` cells (and keys) added.
fn open_with(extra: &str) -> String {
    format!("{OPEN}{extra}")
}

/// A row of water or lava cells at `row`, columns `from..=to`.
fn strip(kind: &str, row: i32, from: i32, to: i32) -> String {
    (from..=to).map(|c| format!("cells.\"{c},{row}\" = {{ kind = \"{kind}\" }}\n")).collect()
}

/// A column of water cells at `col`, rows `from..=to`: a stream.
fn stream(col: i32, from: i32, to: i32) -> String {
    (from..=to).map(|r| format!("cells.\"{col},{r}\" = {{ kind = \"water\" }}\n")).collect()
}

/// The first field point in the box (step 4 px) that `wet` holds for.
fn find(from: Vec2, to: Vec2, wet: impl Fn(Vec2) -> bool) -> Option<Vec2> {
    let mut y = from.y;
    while y <= to.y {
        let mut x = from.x;
        while x <= to.x {
            let p = Vec2::new(x, y);
            if wet(p) {
                return Some(p);
            }
            x += 4.0;
        }
        y += 4.0;
    }
    None
}

fn primed(game: &Game) -> Audio {
    let mut a = audio();
    a.observe_events(game);
    a.observe(game, &view_on(game), &[0], 0.0, true);
    a.take_commands();
    a
}

fn heard(a: &Audio, cue: &str) -> bool {
    a.status().recent.iter().any(|c| c == cue)
}

fn playing(a: &Audio, cue: &str) -> bool {
    a.status().sounds.iter().any(|s| s.cue == cue)
}

fn steps(game: &mut Game, a: &mut Audio, input: Input, n: usize) {
    for _ in 0..n {
        step(game, a, input);
    }
}

/// Wading into a ford splashes; climbing out pours and leaves the runs
/// dripping.
#[test]
fn a_hull_splashes_into_a_ford_and_drips_coming_out() {
    let mut game = game_on(&open_with(&strip("water", 6, 9, 12)));
    let east = find(Vec2::new(380.0, 180.0), Vec2::new(440.0, 240.0), |p| game.water().depth_at(p).is_wet()).map(|p| {
        let mut p = p;
        while game.water().depth_at(Vec2::new(p.x + 4.0, p.y)).is_wet() {
            p.x += 4.0;
        }
        p
    });
    let east = east.expect("the ford has a wet point");
    let mut a = primed(&game);
    steps(&mut game, &mut a, Input::default(), 2);
    game.debug_teleport(0, east, Some(90.0)).expect("into the ford");
    steps(&mut game, &mut a, Input::default(), 4);
    assert!(heard(&a, "vehicle.splash"), "the splash going in: {:?}", a.status().recent);
    let right = Input::single(Intent { move_dir: Some(crate::tank::Dir::Right), ..Intent::default() });
    let mut out = false;
    for _ in 0..120 {
        step(&mut game, &mut a, right);
        if heard(&a, "vehicle.splash.out") {
            out = true;
            break;
        }
    }
    assert!(out, "the water pours off climbing out: {:?}", a.status().recent);
    steps(&mut game, &mut a, Input::default(), 3);
    assert!(playing(&a, "vehicle.drip"), "and the runs drip: {:?}", a.status().sounds);
}

/// A hull parked in a stream drifts with the current, and its tracks are
/// silent: the current is not their speed (`Game::drift_at`).
#[test]
fn a_hull_parked_in_a_current_runs_no_tracks() {
    let mut game = game_on(&open_with(&stream(10, 0, 13)));
    let at = find(Vec2::new(300.0, 120.0), Vec2::new(380.0, 260.0), |p| game.water().depth_at(p).is_wet() && game.water().pushes_south(p)).expect("a stream cell");
    let mut a = primed(&game);
    game.debug_teleport(0, at, None).expect("into the stream");
    steps(&mut game, &mut a, Input::default(), 90);
    let now = bind::tank_in(&game, 0).map(|t| t.position).expect("the seat");
    assert!(now.y - at.y > 10.0, "the current carries it: {at:?} to {now:?}");
    assert!(game.drift_at(now).y > 0.0, "the drift is the current's");
    assert!(!playing(&a, "vehicle.treads"), "no tracks for a drift: {:?}", a.status().sounds);
    assert!(playing(&a, "world.water.stream"), "the stream babbles beside it: {:?}", a.status().sounds);
}

/// A hull in lava sizzles.
#[test]
fn a_hull_in_lava_sizzles() {
    let mut game = game_on(&open_with(&strip("lava", 6, 9, 12)));
    let at = find(Vec2::new(300.0, 180.0), Vec2::new(420.0, 240.0), |p| game.lava().depth_at(p) != crate::ground::Depth::Dry).expect("a lava point");
    let mut a = primed(&game);
    game.debug_teleport(0, at, None).expect("into the lava");
    steps(&mut game, &mut a, Input::default(), 6);
    assert!(playing(&a, "world.lava.sizzle"), "{:?}", a.status().sounds);
}

/// A lake's shore laps near the ears; frozen under snow it creaks instead.
#[test]
fn a_lake_laps_and_a_frozen_one_creaks() {
    let lake: String = (3..=6).map(|r| strip("water", r, 9, 12)).collect();
    for (sky, cue) in [("", "world.water.lap"), ("weather = \"snow\"\n", "world.water.ice")] {
        let mut game = game_on(&format!("{sky}{OPEN}{lake}"));
        let mut a = primed(&game);
        steps(&mut game, &mut a, Input::default(), 3);
        assert!(playing(&a, cue), "{sky:?}: {:?}", a.status().sounds);
    }
}

/// A blast on open water throws it up; a lava bomb hisses in it; on dry
/// ground or ice nothing is added.
#[test]
fn a_blast_on_water_adds_the_water() {
    let lake: String = (3..=6).map(|r| strip("water", r, 9, 12)).collect();
    let game = game_on(&open_with(&lake));
    let wet = find(Vec2::new(280.0, 80.0), Vec2::new(440.0, 240.0), |p| game.water().depth_at(p).is_wet()).expect("a wet point");
    let cues = |e: crate::simulation::Event, game: &Game| match bind::bind(&e, game) {
        Binding::Play(plays) => plays.into_iter().map(|p| p.cue).collect::<Vec<_>>(),
        other => panic!("{other:?}"),
    };
    let blast = crate::simulation::Event::GrenadeBlast { slot: 0, x: wet.x, y: wet.y };
    assert!(cues(blast, &game).contains(&"explosion.water".to_string()));
    let bomb = crate::simulation::Event::Blast { x: wet.x, y: wet.y, chained: false, drum: crate::obstacle::Drum::Lava };
    assert!(cues(bomb, &game).contains(&"explosion.water.steam".to_string()));
    let dry = crate::simulation::Event::GrenadeBlast { slot: 0, x: 100.0, y: 400.0 };
    assert_eq!(cues(dry, &game), vec!["explosion.grenade".to_string()]);
    let frozen = game_on(&format!("weather = \"snow\"\n{OPEN}{lake}"));
    assert_eq!(cues(crate::simulation::Event::GrenadeBlast { slot: 0, x: wet.x, y: wet.y }, &frozen), vec!["explosion.grenade".to_string()], "ice takes no splash");
}

/// A grenade clunks each time it touches down, the second hop softer.
#[test]
fn a_grenade_clunks_as_it_lands() {
    let mut game = game_on(OPEN);
    let mut a = primed(&game);
    let seat = bind::tank_in(&game, 0).map(|t| t.position).expect("the seat");
    let mut g = crate::grenade::Grenade::launch(seat + Vec2::new(0.0, 60.0), Vec2::new(0.0, 1.0), Vec2::new(0.0, 0.0), crate::shell::Owner::Player(0));
    g.id = 4242;
    game.world.spawn((g,));
    let mut strengths = Vec::new();
    for _ in 0..90 {
        let mut plays = Vec::new();
        game.update(Input::default(), crate::PHYSICS_FIXED_DT, W, H);
        a.watch.landings(&game, &crate::tuning::tuning(), &mut plays);
        strengths.extend(plays.iter().filter(|p| p.cue == "weapon.grenades.bounce").map(|p| p.strength));
    }
    assert!(!strengths.is_empty(), "it clunks down");
    assert!(strengths.windows(2).all(|w| w[1] < w[0]), "each hop softer: {strengths:?}");
}

/// The bong's hop landing in water plops.
#[test]
fn the_bong_plops_into_water() {
    let mut game = game_on(&open_with(&strip("water", 2, 4, 8)));
    let wet = find(Vec2::new(130.0, 60.0), Vec2::new(280.0, 110.0), |p| game.water().depth_at(p).is_wet()).expect("a wet point");
    let mut a = primed(&game);
    {
        let mut q = game.world.query::<&mut crate::bong::Bong>();
        let bong = q.iter().next().expect("the bong");
        bong.start_hop(wet);
    }
    steps(&mut game, &mut a, Input::default(), 60);
    assert!(heard(&a, "bong.land.water"), "{:?}", a.status().recent);
}

/// A burning hull driven into a ford has its fire put out with a hiss.
#[test]
fn the_water_hisses_a_burning_hull_out() {
    let mut game = game_on(&open_with(&strip("water", 6, 9, 12)));
    let wet = find(Vec2::new(300.0, 180.0), Vec2::new(420.0, 240.0), |p| game.water().depth_at(p).is_wet()).expect("a wet point");
    let mut a = primed(&game);
    for tank in game.world.query::<&mut crate::tank::Tank>().iter() {
        if tank.owner_slot() == 0 {
            tank.burn_timer = 5.0;
        }
    }
    steps(&mut game, &mut a, Input::default(), 2);
    game.debug_teleport(0, wet, None).expect("into the ford");
    steps(&mut game, &mut a, Input::default(), 4);
    assert!(heard(&a, "world.fire.douse"), "{:?}", a.status().recent);
}

/// A wreck burning in the rain hisses rather than crackles.
#[test]
fn a_wreck_in_the_rain_hisses() {
    let mut game = game_on(&format!("weather = \"rain\"\n{OPEN}"));
    let mut a = primed(&game);
    game.debug_kill(0).expect("the seat wrecks");
    steps(&mut game, &mut a, Input::default(), 4);
    assert!(playing(&a, "world.fire.hiss"), "{:?}", a.status().sounds);
    assert!(!playing(&a, "world.fire"), "no crackle under the rain: {:?}", a.status().sounds);
}

/// Turning from a standstill pivots the tracks, and the pivot is heard.
#[test]
fn a_pivot_grinds() {
    let mut game = game_on(OPEN);
    let mut a = primed(&game);
    let right = Input::single(Intent { move_dir: Some(crate::tank::Dir::Right), ..Intent::default() });
    steps(&mut game, &mut a, right, 12);
    assert!(heard(&a, "vehicle.pivot"), "{:?}", a.status().recent);
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
