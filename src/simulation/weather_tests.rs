//! Headless scenario tests for the weather's rules (docs/weather.md "The
//! rules"): one per promise - enemy sight at night and in fog, grip in the
//! rain, the frozen lake in the snow, a sandstorm's gusts. Every rule is a
//! pure function of the sky, the knobs and the round clock, so each test
//! is exact: the same drive or the same sighting under a clear sky and
//! under the weather. Tests cannot touch the global tuning table, so every
//! number is read off the defaults.

use super::*;
use crate::map::cell_to_world;
use crate::tank::Dir;

const W: f32 = 1280.0;
const H: f32 = 720.0;

/// An empty field under `weather` with the player at (5, 11) and `extra`
/// cells; a Destroy round with no enemies, which never ends by itself.
fn round(weather: &str, extra: &str) -> Game {
    let mut map = format!("version = 1\ntanks = 0\ncells.\"5,11\" = {{ kind = \"start\" }}\n{extra}");
    if !weather.is_empty() {
        map.push_str(&format!("weather = \"{weather}\"\n"));
    }
    let mut game = Game::default();
    game.seed_override = Some(7);
    game.player_row_override = Some(1);
    game.level_overrides.mission = Some(Mission::Destroy);
    game.map = MapFile::from_toml_str(&map).expect("test map parses");
    game.init(W, H);
    game
}

/// A 16 x 11 lake over columns 10..=25 and rows 6..=16: open water in the
/// middle, fords round its rim.
fn lake() -> String {
    let mut cells = String::new();
    for c in 10..=25 {
        for r in 6..=16 {
            cells.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"water\" }}\n"));
        }
    }
    cells
}

fn player_pos(game: &Game) -> Position {
    game.tank_snapshots().iter().find(|t| t.is_player).expect("player").position
}

fn teleport_player(game: &mut Game, pos: Position) {
    let player = game.player().expect("player");
    let body = with_tank(&game.world, player, |t| t.body).expect("body");
    game.physics.set_position(body, pos);
    with_tank_mut(&game.world, player, |t| t.position = pos);
}

fn drive(game: &mut Game, dir: Option<Dir>, frames: usize) {
    for _ in 0..frames {
        let mut input = Input::default();
        input.seats[0].move_dir = dir;
        game.update(input, PHYSICS_FIXED_DT, W, H);
    }
}

/// Whether one frame with an enemy `distance` px east of the player, in
/// the open, raises the group's alert on it.
fn seen_at(weather: &str, distance: f32) -> bool {
    let mut game = round(weather, "");
    let at = player_pos(&game);
    game.debug_spawn_enemy(Position::new(at.x + distance, at.y), Some(1), None).expect("an enemy spawns");
    drive(&mut game, None, 1);
    game.alert_position.is_some()
}

#[test]
fn night_and_fog_shorten_how_far_an_enemy_sees() {
    let t = tuning();
    assert_eq!(round("", "").enemy_sight(), t.enemy_view_range, "a clear sky is the knob itself");
    assert_eq!(round("night", "").enemy_sight(), t.enemy_view_range * t.night_sight_factor);
    assert_eq!(round("storm", "").enemy_sight(), t.enemy_view_range * t.night_sight_factor, "a storm is a night sky");
    assert_eq!(round("fog", "").enemy_sight(), t.enemy_view_range * t.fog_sight_factor);
    assert_eq!(round("rain", "").enemy_sight(), t.enemy_view_range, "rain hides nobody");

    // Between the night's reach and the day's: seen only by daylight.
    let far = t.enemy_view_range * (t.night_sight_factor + 1.0) * 0.5;
    assert!(seen_at("", far), "a clear day sees {far:.0} px");
    assert!(!seen_at("night", far), "the night hides a tank {far:.0} px off");
    assert!(!seen_at("fog", far));
    // Between the fog's reach and the night's: the night sees, fog hides.
    let near = t.enemy_view_range * (t.night_sight_factor + t.fog_sight_factor) * 0.5;
    assert!(seen_at("night", near), "the night still sees {near:.0} px");
    assert!(!seen_at("fog", near), "fog hides a tank {near:.0} px off");
}

#[test]
fn the_rain_loosens_every_hulls_grip() {
    // Up to speed eastward, then a hard turn north: how far the old
    // momentum carries the hull east before the tracks scrub it off.
    let drift = |weather: &str| {
        let mut game = round(weather, "");
        drive(&mut game, Some(Dir::Right), 60);
        let turn = player_pos(&game);
        drive(&mut game, Some(Dir::Up), 30);
        player_pos(&game).x - turn.x
    };
    let (dry, wet) = (drift(""), drift("rain"));
    assert!(wet > dry * 1.3, "the rain carries a turn further: dry {dry:.1} px, wet {wet:.1} px");
    assert!((drift("storm") - wet).abs() < 0.01, "a storm's ground is as wet");
    assert!((drift("fog") - dry).abs() < 0.01, "fog leaves the ground dry");
}

#[test]
fn snow_freezes_the_lake_into_ice_a_hull_drives_across() {
    let clear = round("", &lake());
    let snow = round("snow", &lake());
    assert!(!clear.water().is_frozen() && snow.water().is_frozen());
    let middle = cell_to_world(17, 11);
    assert_eq!(clear.water().depth_at(middle), crate::ground::Depth::Deep);
    assert_eq!(snow.water().depth_at(middle), crate::ground::Depth::Ice);
    assert_eq!(snow.water().deep_cells().count(), 0, "no deep water is left for a collider or the nav grid");

    // The router goes straight over the ice and round the open lake.
    let (from, to) = (cell_to_world(7, 11), cell_to_world(28, 11));
    let around = clear.nav_path_cells(from, to, W, H).expect("the lake can be walked round");
    let across = snow.nav_path_cells(from, to, W, H).expect("the ice is open");
    assert!(across < around, "over the ice {across} cells, round the lake {around}");

    // And a hull does: the open water stops it at the shore, the ice
    // takes it all the way over.
    let reach = |mut game: Game| {
        teleport_player(&mut game, cell_to_world(7, 11));
        drive(&mut game, Some(Dir::Right), 240);
        player_pos(&game).x
    };
    assert!(reach(round("", &lake())) < cell_to_world(14, 11).x, "the lake stops a hull on its rim");
    assert!(reach(round("snow", &lake())) > cell_to_world(26, 11).x, "the ice carries it over");
}

#[test]
fn snow_mid_round_ices_the_lake_and_a_thaw_waits_for_the_next_round() {
    let mut game = round("", &lake());
    drive(&mut game, None, 30);
    let (from, to) = (cell_to_world(7, 11), cell_to_world(28, 11));
    let around = game.nav_path_cells(from, to, W, H).expect("the lake can be walked round");
    game.change_weather(crate::map::Weather::Snow.into());
    assert_eq!(game.weather(), crate::map::Weather::Snow);
    assert_eq!(game.frame(), 30, "the round runs on");
    assert!(game.water().is_frozen() && game.water().deep_cells().count() == 0);
    let across = game.nav_path_cells(from, to, W, H).expect("the ice is open");
    assert!(across < around, "over the ice {across} cells, round the lake {around}");
    // The open water's bodies went with it: a hull drives over.
    teleport_player(&mut game, from);
    drive(&mut game, Some(Dir::Right), 240);
    assert!(player_pos(&game).x > cell_to_world(26, 11).x, "the ice carries it over");
    // Rain falls on the ice, which stays ice until the next round.
    game.change_weather(crate::map::Weather::Rain.into());
    assert_eq!(game.weather(), crate::map::Weather::Rain);
    assert!(game.water().is_frozen(), "no thaw under a hull mid-round");
    game.init(W, H);
    assert!(!game.water().is_frozen(), "the next round settles its water from its own sky");
}

#[test]
fn ice_slides_where_the_ground_would_stop_a_hull() {
    // The same drive and the same release, on the ice and on dry ground
    // under the same snowy sky: how far the hull coasts once let go.
    let coast = |cell: (i32, i32)| {
        let mut game = round("snow", &lake());
        teleport_player(&mut game, cell_to_world(cell.0, cell.1));
        drive(&mut game, Some(Dir::Right), 45);
        let released = player_pos(&game);
        drive(&mut game, None, 120);
        player_pos(&game).x - released.x
    };
    let (ice, dry) = (coast((11, 11)), coast((11, 20)));
    assert!(ice > dry * 3.0 && ice > 8.0, "a hull slides on the ice: {ice:.1} px, on dry ground {dry:.1} px");

    // Ice takes tread marks, as the ground does, and wets nothing.
    let mut game = round("snow", &lake());
    teleport_player(&mut game, cell_to_world(12, 11));
    drive(&mut game, Some(Dir::Right), 60);
    let on_ice = |p: &Position| p.x > cell_to_world(10, 11).x && p.x < cell_to_world(25, 11).x && (p.y - cell_to_world(12, 11).y).abs() < 20.0;
    assert!(game.wear().blocks().any(|(p, _)| on_ice(&p)), "the ice takes the hull's marks");
    assert!(game.wear().blocks().all(|(_, b)| b.wet() == 0.0), "and none of them is wet");
    assert!(game.wading().is_empty(), "nothing sprays on the ice");
}

#[test]
fn a_sandstorms_gusts_carry_an_idle_hull_downwind() {
    // Half a minute standing still: gusts come in most windows after the
    // first, and each one carries the hull east while it passes.
    let drift = |weather: &str| {
        let mut game = round(weather, "");
        let start = player_pos(&game);
        let mut furthest = Position::new(0.0, 0.0);
        for _ in 0..1800 {
            drive(&mut game, None, 1);
            let p = player_pos(&game);
            if p.distance_to(start) > furthest.distance_to(Position::new(0.0, 0.0)) {
                furthest = Position::new(p.x - start.x, p.y - start.y);
            }
        }
        (furthest, player_pos(&game).x - start.x)
    };
    let (still, _) = drift("");
    assert!(still.x.abs() < 0.5 && still.y.abs() < 0.5, "a clear day holds still: {still:?}");
    let (gusted, east) = drift("sandstorm");
    assert!(gusted.x > 12.0, "a gust carried the hull east: {gusted:?}");
    assert!(gusted.x > gusted.y.abs() * 2.0, "downwind is east, give or take the swing: {gusted:?}");
    assert!(east > 12.0, "and it stays where the wind left it: {east:.1} px");
}

#[test]
fn with_the_rules_off_a_sky_is_only_drawn() {
    let mut t = crate::tuning::Tuning::DEFAULT;
    t.weather_rules = false;
    for sky in crate::map::Weather::SKIES {
        assert_eq!(crate::weather::sight_factor(sky, &t), 1.0);
        assert_eq!(crate::weather::grip_factor(sky, &t), 1.0);
        assert!(!crate::weather::freezes(sky, &t));
        for i in 0..600 {
            let v = crate::weather::gust_at(sky, Position::new(300.0, 200.0), i as f32 * 0.1, &t);
            assert!(v.x == 0.0 && v.y == 0.0, "{sky:?} blew at {i}");
        }
    }
}
