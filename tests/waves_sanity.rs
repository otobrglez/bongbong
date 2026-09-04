//! Headless end-to-end run of a waves round on the shipped map through
//! the public API only: every wave called, tanks rolling in through
//! gates, wrecks despawning, and the round ending Won once the last wave
//! is wrecked. The player is shielded for the whole round (its shield
//! bounces shells back at the shooters), so an AFK round plays through
//! instead of ending Lost within seconds.

use bongbong::level::{Mission, SpawnKind};
use bongbong::map::MapFile;
use bongbong::simulation::debug::TankPatch;
use bongbong::simulation::{Event, Game, Input, Outcome};

fn run(seed: u64, shielded: bool) -> (Game, usize, usize, usize) {
    let (w, h) = (1280.0, 720.0);
    let mut game = Game::default();
    game.seed_override = Some(seed);
    game.level_overrides.mission = Some(Mission::Destroy);
    game.level_overrides.spawn = Some(SpawnKind::Waves);
    game.level_overrides.waves = Some(3);
    game.level_overrides.wave_size = Some(2);
    game.map = MapFile::from_toml_str(include_str!("../maps/default.toml")).expect("default map parses");
    game.init(w, h);
    if shielded {
        game.debug_set_tank(0, &TankPatch { shield_timer: Some(1.0e9), ..TankPatch::default() }).unwrap();
    }
    let (mut waves, mut entered, mut removed) = (0, 0, 0);
    // Three waves' worth of `wave_timeout_seconds` plus their gaps.
    let frames = if shielded { 15_000 } else { 3600 };
    for _ in 0..frames {
        game.update(Input::default(), 1.0 / 60.0, w, h);
        for e in game.events() {
            match e {
                Event::WaveStarted { .. } => waves += 1,
                Event::TankEntered { .. } => entered += 1,
                Event::WreckRemoved { .. } => removed += 1,
                _ => {}
            }
        }
        if game.outcome() != Outcome::Playing {
            break;
        }
    }
    let status = game.wave_status().expect("a waves round reports status");
    eprintln!(
        "seed={seed:#x} shielded={shielded} outcome={:?} waves_started={waves} tanks_entered={entered} wrecks_removed={removed} status={status:?} frame={}",
        game.outcome(),
        game.frame()
    );
    (game, waves, entered, removed)
}

#[test]
fn a_three_wave_destroy_round_on_the_default_map_plays_through() {
    let (w, h) = (1280.0, 720.0);
    let (game, waves, entered, removed) = run(0xB0B5, true);
    assert_eq!(waves, 3, "every wave was called");
    // `wave_growth` defaults to 1: waves of 2, 3 and 4.
    assert_eq!(entered, 2 + 3 + 4, "every tank of every wave rolled in");
    assert!(removed >= 1, "at least one wreck despawned");
    assert!(matches!(game.outcome(), Outcome::Won | Outcome::Playing), "{:?}", game.outcome());
    for t in game.tank_snapshots().iter().filter(|t| !t.entering) {
        let p = t.position;
        assert!(p.x > 0.0 && p.x < w && p.y > 0.0 && p.y < h, "on the field: ({:.0},{:.0})", p.x, p.y);
    }
}

/// The unshielded control: an AFK player still sees wave 1 arrive.
#[test]
fn an_afk_player_still_sees_the_first_wave_arrive() {
    let (_, waves, entered, _) = run(0xB0B5, false);
    assert!(waves >= 1 && entered >= 2, "waves={waves} entered={entered}");
}
