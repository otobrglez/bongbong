//! Headless scenario tests for the flamethrower (docs/flamethrower-prd.md
//! section 11): one per promised rule. The weapon draws no RNG, so most
//! rules are exact rather than statistical; the two that depend on a
//! spawn roll (a wood tile's `flammable`, a drum's launch chance) sweep
//! seeds. Tests cannot touch the global tuning table, so every timing is
//! read off the defaults.

use super::debug::TankPatch;
use super::hits::Terrain;
use super::*;
use crate::ai::Intent;
use crate::map::cell_to_world;
use crate::obstacle::{Material, SCORCH_S};
use crate::tank::Dir;

const W: f32 = 1280.0;
const H: f32 = 720.0;
const DT: f32 = 1.0 / 60.0;

/// Where the player stands in every test: cell (20,15), facing up, so
/// "ahead" is smaller y. The muzzle sits `tank_muzzle_forward_offset`
/// times the sprite scale above the hull centre.
const PLAYER_CELL: (i32, i32) = (20, 15);
/// The one enemy every map needs is boxed in iron in the far corner, out
/// of sight and reach of everything the tests do.
const ENEMY_CELL: (i32, i32) = (37, 20);

fn map_with(extra: &str) -> String {
    let mut text = String::from(
        r#"
version = 1
tanks = 1
cells."2,2" = { kind = "frog" }
cells."20,18" = { kind = "start" }
"#,
    );
    for (c, r) in [(36, 19), (37, 19), (38, 19), (36, 20), (38, 20), (36, 21), (37, 21), (38, 21)] {
        text.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
    }
    text.push_str(extra);
    text
}

/// A round on `map` with the player parked at `PLAYER_CELL` facing up
/// and holding `fuel` seconds of flamethrower fuel.
fn armed_game(map: &str, seed: u64, fuel: f32) -> Game {
    let mut game = Game::default();
    game.enemy_count_override = Some(1);
    game.seed_override = Some(seed);
    game.player_row_override = Some(0);
    game.map = MapFile::from_toml_str(map).expect("test map parses");
    game.init(W, H);
    game.debug_teleport(1, cell_to_world(ENEMY_CELL.0, ENEMY_CELL.1), Some(0.0)).expect("enemy in slot 1");
    game.debug_teleport(0, cell_to_world(PLAYER_CELL.0, PLAYER_CELL.1), Some(0.0)).expect("player in slot 0");
    if fuel > 0.0 {
        game.debug_set_tank(0, &TankPatch { flame_fuel: Some(fuel), ..TankPatch::default() }).unwrap();
    }
    game
}

fn step(game: &mut Game, input: Input) {
    game.update(input, DT, W, H);
}

fn fire() -> Input {
    Input { player_intent: Intent { fire: true, ..Intent::default() }, ..Input::default() }
}

fn drive(dir: Dir) -> Input {
    Input { player_intent: Intent { move_dir: Some(dir), ..Intent::default() }, ..Input::default() }
}

/// Hold the trigger for `frames`, collecting every event.
fn hold(game: &mut Game, frames: usize) -> Vec<Event> {
    let mut events = Vec::new();
    for _ in 0..frames {
        step(game, fire());
        events.extend(game.events().iter().cloned());
    }
    events
}

fn idle(game: &mut Game, frames: usize) -> Vec<Event> {
    let mut events = Vec::new();
    for _ in 0..frames {
        step(game, Input::default());
        events.extend(game.events().iter().cloned());
    }
    events
}

fn snapshot(game: &Game, slot: usize) -> TankSnapshot {
    game.tank_snapshots().into_iter().find(|t| t.slot == slot).expect("tank in slot")
}

fn player_weapon(game: &Game) -> ActiveWeapon {
    with_tank(&game.world, game.player.unwrap(), |t| t.active_weapon())
}

fn obstacle_at(game: &Game, cell: (i32, i32)) -> Option<(Material, f32, bool, bool, u8, bool)> {
    let pos = cell_to_world(cell.0, cell.1);
    game.world
        .query::<&Obstacle>()
        .iter()
        .find(|o| o.position.distance_to(pos) < 1.0)
        .map(|o| (o.material, o.health, o.burning, o.fuse.is_some(), o.scorched, o.destroyed))
}

fn ignited(events: &[Event], what: &str) -> usize {
    events.iter().filter(|e| matches!(e, Event::Ignited { what: w, .. } if *w == what)).count()
}

/// The frame a `material` tile reported its destruction, holding fire.
fn destroyed_under_fire(game: &mut Game, material: Material, max_frames: usize) -> Option<usize> {
    for frame in 1..=max_frames {
        step(game, fire());
        if game.events().iter().any(|e| matches!(e, Event::ObstacleDestroyed { material: m, .. } if *m == material)) {
            return Some(frame);
        }
    }
    None
}

// 1. A pickup grants fuel and queues the weapon; holding fire drains it
//    at one second per second; the slot empties at zero.
#[test]
fn a_fuel_tank_arms_the_flamethrower_and_a_held_trigger_drains_it() {
    let map = map_with("cells.\"20,15\" = { kind = \"pickup\", pickup = \"flamethrower\" }\n");
    let mut game = armed_game(&map, 3, 0.0);
    assert_eq!(player_weapon(&game), ActiveWeapon::Shell);
    step(&mut game, Input::default());
    assert!(game.events().iter().any(|e| matches!(e, Event::PickupCollected { slot: 0, kind: PickupKind::Flamethrower })));
    let fuel = tuning().flame_fuel_per_pickup;
    assert!((snapshot(&game, 0).flame_fuel - fuel).abs() < 1e-4, "the pickup grants the whole tank");
    assert_eq!(player_weapon(&game), ActiveWeapon::Flamethrower, "a first pickup arms it at once");

    let events = hold(&mut game, 60);
    let after = snapshot(&game, 0).flame_fuel;
    assert!((fuel - after - 1.0).abs() < 0.02, "one second of holding burns one second of fuel: {after}");
    assert_eq!(events.iter().filter(|e| matches!(e, Event::Fired { weapon: "flamethrower", .. })).count(), 1, "one Fired per hold, not per frame");
    assert!(!game.flames().is_empty(), "a jet is live while the trigger is held");

    idle(&mut game, 1);
    assert!(game.flames().is_empty(), "no jet the frame after release");
    let events = hold(&mut game, 60 * fuel.ceil() as usize + 5);
    assert_eq!(events.iter().filter(|e| matches!(e, Event::Fired { weapon: "flamethrower", .. })).count(), 1, "the second hold is one more Fired");
    assert_eq!(snapshot(&game, 0).flame_fuel, 0.0);
    assert_eq!(player_weapon(&game), ActiveWeapon::Shell, "dry: the queue moves on");
    assert!(game.flames().is_empty(), "nothing fires on an empty tank");
}

// Enemies never carry it: the fuel tank is not collected by one.
#[test]
fn an_enemy_drives_over_the_fuel_tank_without_collecting_it() {
    let map = map_with("cells.\"30,15\" = { kind = \"pickup\", pickup = \"flamethrower\" }\n");
    let mut game = armed_game(&map, 3, 0.0);
    game.debug_teleport(1, cell_to_world(30, 15), Some(0.0)).unwrap();
    step(&mut game, Input::default());
    assert_eq!(game.world.query::<&Pickup>().iter().count(), 1, "the pickup is still on the field");
    assert_eq!(snapshot(&game, 1).flame_fuel, 0.0);
    assert!(!game.events().iter().any(|e| matches!(e, Event::PickupCollected { .. })));
}

// 2. An enemy directly ahead inside the range takes damage; one behind,
//    or beside the cone, takes none; one past a wall takes none.
#[test]
fn the_cone_reaches_what_is_ahead_and_nothing_else() {
    let run = |extra: &str, enemy_cell: (i32, i32)| -> (f32, Vec<Event>) {
        let map = map_with(extra);
        let mut game = armed_game(&map, 5, 6.0);
        game.debug_teleport(1, cell_to_world(enemy_cell.0, enemy_cell.1), Some(180.0)).unwrap();
        let events = hold(&mut game, 12);
        (snapshot(&game, 1).damage, events)
    };
    let dps = tuning().flame_damage_per_second;
    let (ahead, events) = run("", (20, 13));
    assert!(ahead > 0.0 && ahead <= dps * 12.0 * DT + 0.01, "ahead takes the rate: {ahead}");
    assert_eq!(
        events.iter().filter(|e| matches!(e, Event::Hit { target: HitTarget::Enemy { slot: 1 }, damage, .. } if *damage == 0.0)).count(),
        1,
        "contact is logged once per hold"
    );
    let (behind, _) = run("", (20, 17));
    assert_eq!(behind, 0.0, "behind the nozzle");
    let (beside, _) = run("", (17, 13));
    assert_eq!(beside, 0.0, "beside the cone");
    let (far, _) = run("", (20, 9));
    assert_eq!(far, 0.0, "past the range");
    let (walled, _) = run("cells.\"20,12\" = { kind = \"wall\", material = \"iron\" }\n", (20, 10));
    assert_eq!(walled, 0.0, "past a wall");
}

// 3. A touched enemy keeps burning for `flame_afterburn_seconds` after
//    the stream moves away, and afterburn can kill.
#[test]
fn afterburn_keeps_a_touched_tank_burning_and_can_finish_it() {
    let map = map_with("");
    let mut game = armed_game(&map, 5, 6.0);
    game.debug_teleport(1, cell_to_world(20, 13), Some(180.0)).unwrap();
    hold(&mut game, 6);
    let t = tuning();
    let burn = snapshot(&game, 1).burn_timer;
    assert!((burn - t.flame_afterburn_seconds).abs() < 2.0 * DT + 1e-4, "contact sets the timer: {burn}");
    let touched = snapshot(&game, 1).damage;
    assert!(game.burning_tanks().len() == 1, "the burning hull is reported for the particles");
    // Move the enemy well out of the cone and watch the afterburn run.
    game.debug_teleport(1, cell_to_world(30, 15), Some(0.0)).unwrap();
    let half = (t.flame_afterburn_seconds / 2.0 / DT) as usize;
    idle(&mut game, half);
    let midway = snapshot(&game, 1).damage;
    assert!(midway > touched, "still taking damage after the stream left");
    idle(&mut game, half + 10);
    let done = snapshot(&game, 1).damage;
    assert!((done - touched - t.flame_afterburn_dps * t.flame_afterburn_seconds).abs() < 0.5, "afterburn dealt its rate for its duration: {}", done - touched);
    assert_eq!(snapshot(&game, 1).burn_timer, 0.0);
    idle(&mut game, 30);
    assert_eq!(snapshot(&game, 1).damage, done, "and stopped");

    // A tank on its last sliver dies of the afterburn, not the stream.
    let mut game = armed_game(&map, 5, 6.0);
    game.debug_teleport(1, cell_to_world(20, 13), Some(180.0)).unwrap();
    game.debug_set_tank(1, &TankPatch { damage: Some(MAX_DAMAGE - 0.5), ..TankPatch::default() }).unwrap();
    hold(&mut game, 1);
    assert!(!snapshot(&game, 1).is_wreck, "one frame of stream is under half a point");
    game.debug_teleport(1, cell_to_world(30, 15), Some(0.0)).unwrap();
    let events = idle(&mut game, 30);
    assert!(snapshot(&game, 1).is_wreck, "the afterburn finished it");
    assert!(events.iter().any(|e| matches!(e, Event::Wreck { slot: 1, .. })), "and it exploded like any kill");
}

// 4. Holding on a cell for the ignition time lights it; a quick sweep
//    does not; the lit cell blocks the nav grid and burns out on schedule.
#[test]
fn ground_lights_under_a_held_stream_but_not_a_sweep() {
    let map = map_with("");
    let t = tuning();
    let ignite_frames = (t.flame_ignite_seconds / DT).ceil() as usize;

    let mut game = armed_game(&map, 5, 6.0);
    let sweep = hold(&mut game, ignite_frames / 2);
    let cooled = idle(&mut game, 60);
    assert_eq!(ignited(&sweep, "ground") + ignited(&cooled, "ground"), 0, "a quick sweep lights nothing");
    assert!(game.burning_cells().is_empty());
    let ahead = cell_to_world(20, 13);
    assert_eq!(game.cell_heat((20, 13)), 0.0, "and the heat has faded");

    let events = hold(&mut game, ignite_frames + 2);
    assert!(ignited(&events, "ground") >= 1, "a held stream lights the ground");
    assert!(events.iter().any(|e| matches!(e, Event::FireStarted { pool: true, .. })));
    let cells = game.burning_cells();
    assert!(!cells.is_empty());
    assert!(cells.iter().all(|(p, _, total)| p.y < ahead.y + 1.0 && *total == t.flame_ground_seconds), "lit ahead of the nozzle, for the ground time");
    assert!(cells.iter().any(|(p, _, _)| p.distance_to(ahead) < 1.0), "the cell the stream washes over is among them");
    assert!(!game.nav_grid(W, H).usable(ahead), "a burning cell is a wall to the AI");

    idle(&mut game, (t.flame_ground_seconds / DT) as usize + 5);
    assert!(game.burning_cells().is_empty(), "burnt out on schedule");
    assert!(game.nav_grid(W, H).usable(ahead), "and open again");
}

// 5. Grass in a lit cell is gone and the cell no longer conceals.
#[test]
fn grass_under_the_stream_burns_away_and_stops_concealing() {
    let map = map_with("cells.\"20,13\" = { kind = \"tall_grass\" }\ncells.\"16,13\" = { kind = \"tall_grass\" }\n");
    let mut game = armed_game(&map, 5, 6.0);
    let cell = cell_to_world(20, 13);
    assert!(Terrain::build(&game.world, W, H, &game.grass_cells).conceals(cell), "grass conceals before the fire");
    let tufts = game.grass.len();
    assert!(tufts > 0);

    let ignite_frames = (tuning().flame_ignite_seconds / DT).ceil() as usize;
    hold(&mut game, ignite_frames + 2);
    assert!(!Terrain::build(&game.world, W, H, &game.grass_cells).conceals(cell), "a burning cell no longer conceals");
    assert!(!game.grass_cells.iter().any(|c| c.distance_to(cell) < 1.0));
    assert_eq!(game.grass.len(), tufts, "the tufts stay, as stubs");
    let burnt = game.grass.iter().filter(|g| g.burnt).count();
    assert!(burnt > 0 && burnt < tufts, "the lit cell's tufts charred, the other cell's did not: {burnt}/{tufts}");
    idle(&mut game, 200);
    assert!(!Terrain::build(&game.world, W, H, &game.grass_cells).conceals(cell), "burnt grass stays gone");
}

// 6. A wood tile rolled non-flammable still ignites under the stream.
#[test]
fn a_wood_tile_ignites_under_the_stream_whatever_it_rolled() {
    let map = map_with("cells.\"20,12\" = { kind = \"wall\", material = \"wood\" }\n");
    let ignite_frames = (tuning().flame_ignite_seconds / DT).ceil() as usize;
    let mut flammable_rolls = 0;
    for seed in 1..=12u64 {
        let mut game = armed_game(&map, seed, 6.0);
        let rolled = game.world.query::<&Obstacle>().iter().find(|o| o.material == Material::Wood).map(|o| o.flammable).unwrap();
        flammable_rolls += rolled as usize;
        let events = hold(&mut game, ignite_frames + 2);
        let (_, health, burning, _, _, _) = obstacle_at(&game, (20, 12)).expect("the plank is still there, alight");
        assert!(burning && health == 0.0, "seed {seed}: the plank is burning");
        assert_eq!(ignited(&events, "wood"), 1, "seed {seed}");
    }
    assert!(flammable_rolls < 12, "some of the twelve planks rolled non-flammable");
}

// 7. A sandbag collapses after its seconds, a fence after fewer.
#[test]
fn a_sandbag_and_a_fence_collapse_under_sustained_heat() {
    let t = tuning();
    let sandbag_frames = (t.flame_sandbag_seconds / DT).round() as usize;
    let fence_frames = (t.flame_fence_seconds / DT).round() as usize;

    let mut game = armed_game(&map_with("cells.\"20,12\" = { kind = \"sandbag\" }\n"), 5, 10.0);
    let sandbag = destroyed_under_fire(&mut game, Material::Sandbag, sandbag_frames + 30).expect("the sandbag collapses");
    assert!(sandbag >= sandbag_frames && sandbag <= sandbag_frames + 3, "at its exposure: frame {sandbag}");
    assert!(game.events().iter().any(|e| matches!(e, Event::Ignited { what: "sandbag", .. })));

    let mut game = armed_game(&map_with("cells.\"20,12\" = { kind = \"fence\" }\n"), 5, 10.0);
    let fence = destroyed_under_fire(&mut game, Material::Fence, fence_frames + 30).expect("the fence snaps");
    assert!(fence >= fence_frames && fence <= fence_frames + 3, "at its exposure: frame {fence}");
    assert!(fence < sandbag, "a fence goes before a bag of sand");
}

// 8. A drum in the cone gets a fuse; a fuel drum launches away from the
//    shooter.
#[test]
fn a_drum_in_the_cone_is_fused_and_a_fuel_drum_launches_away() {
    let ignite_frames = (tuning().flame_ignite_seconds / DT).ceil() as usize;
    let mut game = armed_game(&map_with("cells.\"20,12\" = { kind = \"barrel\", drum = \"oil\" }\n"), 5, 6.0);
    let events = hold(&mut game, ignite_frames + 2);
    assert_eq!(ignited(&events, "drum"), 1);
    let (_, _, _, fused, _, _) = obstacle_at(&game, (20, 12)).expect("the drum is still there, fused");
    assert!(fused);
    assert_eq!(game.fused_barrels().len(), 1);
    let events = idle(&mut game, 60 * 12);
    assert!(events.iter().any(|e| matches!(e, Event::Blast { chained: true, drum: Drum::Oil, .. })), "and pops in place");

    let mut launched = 0;
    for seed in 1..=10u64 {
        let mut game = armed_game(&map_with("cells.\"20,12\" = { kind = \"barrel\", drum = \"fuel\" }\n"), seed, 6.0);
        hold(&mut game, ignite_frames + 2);
        let events = idle(&mut game, 60 * 8);
        if let Some(Event::DrumLaunched { y, to_y, .. }) = events.iter().find(|e| matches!(e, Event::DrumLaunched { .. })) {
            launched += 1;
            assert!(*to_y < *y, "seed {seed}: launched away from the shooter, who stands below");
        }
    }
    assert!(launched > 0, "a fuel drum lit by the stream launches");
}

// 9. An iron tile facing the muzzle is sooted and undamaged.
#[test]
fn iron_facing_the_nozzle_is_sooted_and_nothing_more() {
    let mut game = armed_game(&map_with("cells.\"20,12\" = { kind = \"wall\", material = \"iron\" }\n"), 5, 6.0);
    let (_, before, _, _, soot_before, _) = obstacle_at(&game, (20, 12)).unwrap();
    assert_eq!(soot_before, 0);
    let events = hold(&mut game, 120);
    let (_, after, burning, fused, soot, destroyed) = obstacle_at(&game, (20, 12)).unwrap();
    assert_eq!(soot & SCORCH_S, SCORCH_S, "the face toward the shooter is sooted");
    assert_eq!(after, before);
    assert!(!burning && !fused && !destroyed);
    assert!(events.iter().all(|e| !matches!(e, Event::Ignited { what: "drum" | "wood" | "tree" | "sandbag" | "fence", .. } | Event::ObstacleDestroyed { .. })));
    // The ground in front of the wall burns; the tile's own cell does not.
    assert!(!game.burning_cells().is_empty(), "the ground short of the wall lights");
    let tile = cell_to_world(20, 12);
    assert!(game.burning_cells().iter().all(|(p, _, _)| p.distance_to(tile) > 1.0), "the tile's own cell never lights as ground");
}

// 10. The shooter takes no damage from its stream and does take damage
//     from driving into the strip it lit.
#[test]
fn the_shooter_is_safe_from_its_stream_but_not_from_the_fire_it_leaves() {
    let mut game = armed_game(&map_with(""), 5, 10.0);
    let ignite_frames = (tuning().flame_ignite_seconds / DT).ceil() as usize;
    hold(&mut game, ignite_frames + 6);
    assert_eq!(snapshot(&game, 0).damage, 0.0, "the stream never hurts its shooter");
    assert!(!game.burning_cells().is_empty());
    for _ in 0..50 {
        step(&mut game, drive(Dir::Up));
    }
    assert!(snapshot(&game, 0).damage > 0.0, "driving into the strip costs health");
}

// 11. A round that uses the flamethrower replays bit-identical: nothing
//     it does draws RNG.
#[test]
fn a_round_with_the_flamethrower_replays_bit_identical() {
    let map = map_with(
        r#"
cells."20,12" = { kind = "wall", material = "wood" }
cells."19,13" = { kind = "tall_grass" }
cells."21,13" = { kind = "barrel", drum = "oil" }
cells."22,14" = { kind = "sandbag" }
"#,
    );
    let run = |seed: u64| {
        let mut game = armed_game(&map, seed, 12.0);
        let mut trace = Vec::new();
        for frame in 0..420 {
            let input = match frame % 60 {
                0..=29 => fire(),
                30..=39 => drive(Dir::Right),
                _ => drive(Dir::Up),
            };
            step(&mut game, input);
            trace.push((
                game.tank_snapshots().iter().map(|t| (t.position.x.to_bits(), t.position.y.to_bits(), t.damage.to_bits(), t.burn_timer.to_bits())).collect::<Vec<_>>(),
                game.burning_cells().len(),
                game.events().len(),
            ));
        }
        trace
    };
    assert!(run(9) == run(9));
}
