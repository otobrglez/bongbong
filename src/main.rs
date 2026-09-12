use bongbong::tuning::tuning;
use bongbong::ai::Intent;
use bongbong::editor::{BuilderInput, CliOverrides, EditorTextures};
use bongbong::game::{Effects, Textures};
use bongbong::hud::{leave_dialog_rects, mode_button_rect, players_button_rect, players_dialog_rects};
use bongbong::mode::{Driver, Session};
use bongbong::shockwave::{RippleFx, RippleTuning};
use bongbong::simulation::{Game, Input, PlayerCount};
use bongbong::tuning;
use bongbong::tank::{Dir, TankKind};
use bongbong::touch::TouchPoint;
use bongbong::view::View;
use bongbong::{
    Layout,
};
use clap::Parser;
use sola_raylib::core::game_loop;
use sola_raylib::prelude::{KeyboardKey, RaylibHandle};

/// This frame's raw movement/fire commands for both players, from the
/// keyboard. `fire` is the raw held state - whether it actually fires
/// (edge-triggered for shells, full-auto while a laser is charged) is
/// `Game::update`'s call, not this function's; see `Input::player_intent`.
/// Player 1 is always the arrows + Space; with two players, player 2 is
/// WASD + Left Shift, otherwise idle (docs/two-players.md).
fn gather_intents(rl: &RaylibHandle, players: PlayerCount) -> (Intent, Intent) {
    let dir = |up: KeyboardKey, down: KeyboardKey, left: KeyboardKey, right: KeyboardKey| {
        if rl.is_key_down(up) {
            Some(Dir::Up)
        } else if rl.is_key_down(down) {
            Some(Dir::Down)
        } else if rl.is_key_down(left) {
            Some(Dir::Left)
        } else if rl.is_key_down(right) {
            Some(Dir::Right)
        } else {
            None
        }
    };
    let arrows = dir(KeyboardKey::KEY_UP, KeyboardKey::KEY_DOWN, KeyboardKey::KEY_LEFT, KeyboardKey::KEY_RIGHT);
    let player1 = Intent { move_dir: arrows, fire: rl.is_key_down(KeyboardKey::KEY_SPACE), ..Intent::default() };
    match players {
        PlayerCount::One => (player1, Intent::default()),
        PlayerCount::Two => {
            let wasd = dir(KeyboardKey::KEY_W, KeyboardKey::KEY_S, KeyboardKey::KEY_A, KeyboardKey::KEY_D);
            (player1, Intent { move_dir: wasd, fire: left_shift_down(rl), ..Intent::default() })
        }
    }
}

/// Whether the left Shift key - player 2's fire key - is held. Native reads
/// raylib's key. On the web emscripten's GLFW layer reports the DOM Shift
/// key as `GLFW_KEY_LEFT_SHIFT` whichever side was pressed (`libglfw.js`
/// maps keyCode 0x10 to the left key and never looks at `event.location`),
/// so Right Shift would fire player 2's tank too: the page keeps
/// `window.bbShift` (bit 1 = ShiftLeft, from `keydown`/`keyup` on
/// `event.code`) and this reads it once a frame instead.
#[cfg(target_os = "emscripten")]
fn left_shift_down(_rl: &RaylibHandle) -> bool {
    unsafe extern "C" {
        fn emscripten_run_script_int(script: *const std::os::raw::c_char) -> std::os::raw::c_int;
    }
    // SAFETY: a NUL-terminated literal, evaluated synchronously by the
    // emscripten runtime; the value is a plain int.
    let mask = unsafe { emscripten_run_script_int(c"(window.bbShift|0)".as_ptr()) };
    mask & 1 != 0
}

#[cfg(not(target_os = "emscripten"))]
fn left_shift_down(rl: &RaylibHandle) -> bool {
    rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
}

/// Command-line flags for bongbong's native binary. All optional - with none
/// given, behavior matches today's defaults exactly (random enemy count,
/// random player chassis, 1280x720 window, shadows on).
#[derive(Parser)]
#[command(name = "bongbong", about = "A pixelated tank shooter")]
struct Args {
    /// Override the number of enemies spawned this round. Takes precedence
    /// over the loaded map's own `tanks` default (see `-m`/`--map` below and
    /// `map::MapFile::tanks`); with neither given, falls back to a random
    /// count between `enemy_count_min` and `enemy_count_max`. 0 is a
    /// sandbox round: nobody to fight, and it never ends by wreck count.
    #[arg(short = 'e', long = "enemies")]
    enemies: Option<usize>,

    /// Override the map's mission (what ends the round): protect (keep the
    /// frog alive, wreck every enemy), hunt (kill the enemy frog first) or
    /// destroy (no frog, wreck every enemy). See docs/maps-to-levels.md.
    #[arg(long = "mission", value_enum)]
    mission: Option<bongbong::level::Mission>,

    /// Override the map's spawn plan: band (everyone placed at once, the
    /// default) or waves (enemies roll in through edge gates wave after
    /// wave; see --waves/--wave-size/--wave-growth/--tier-start/--tier-end).
    #[arg(long = "spawn", value_enum)]
    spawn: Option<bongbong::level::SpawnKind>,

    /// Waves plan: number of waves.
    #[arg(long = "waves")]
    waves: Option<u32>,

    /// Waves plan: tanks in the first wave.
    #[arg(long = "wave-size")]
    wave_size: Option<u32>,

    /// Waves plan: tanks added per wave.
    #[arg(long = "wave-growth")]
    wave_growth: Option<u32>,

    /// Waves plan: chassis tier of the first wave (light, medium, heavy,
    /// super).
    #[arg(long = "tier-start", value_enum)]
    tier_start: Option<bongbong::level::Tier>,

    /// Waves plan: chassis tier of the last wave.
    #[arg(long = "tier-end", value_enum)]
    tier_end: Option<bongbong::level::Tier>,

    /// Force the player's tank to a specific chassis - e.g. `--tank titan`
    /// for the twin-barrel super-heavy, without restarting until it happens
    /// to roll. Outranks every other way a chassis gets picked: the
    /// `player_tank` tuning knob, then the loaded map's own `tank` key, then
    /// (with none of the three set) a random roll each round. Persists
    /// across in-game restarts (R key).
    #[arg(long = "tank", value_enum)]
    tank: Option<TankKind>,

    /// Two-player mode: pin player 2's chassis, over the loaded map's own
    /// `tank2` key (else a random roll each round). Persists across
    /// restarts like `--tank`.
    #[arg(long = "tank2", value_enum)]
    tank2: Option<TankKind>,

    /// Start the session in single (1) or two-player (2) mode - the
    /// players button in the HUD bar switches later (docs/two-players.md).
    /// Player 1 is always the arrows + Space; two players adds player 2 on
    /// WASD + Left Shift. Kept across restarts.
    #[arg(long = "players", default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=2))]
    players: u8,

    /// The window's initial size, e.g. `--resolution 1920x1080` (default:
    /// the map's own bitmap, the field plus the HUD bar). The battlefield
    /// itself is the map's `size` and every player in a match shares it;
    /// the window only decides how large it is drawn, letterboxed so the
    /// whole field is always on screen (view.rs). Resizable afterwards;
    /// F11 toggles borderless full screen.
    #[arg(long = "resolution", value_parser = parse_resolution)]
    resolution: Option<(i32, i32)>,

    /// Start in borderless full screen on the current monitor (F11
    /// toggles it back).
    #[arg(long = "fullscreen")]
    fullscreen: bool,

    /// Development aid: the left mouse button acts as a touch point, so
    /// the touch scheme (touch.rs: joystick on one half, tap-to-fire on
    /// the other) can be tried on a desktop. Off, the mouse keeps to the
    /// bar's buttons, the dialogs and the builder.
    #[arg(long = "touch-from-mouse")]
    touch_from_mouse: bool,

    /// Disable tank/shell drop shadows (on by default). Can also be toggled
    /// at runtime with the L key - see docs/sprite-shadows-design.md.
    #[arg(long = "no-shadows")]
    no_shadows: bool,

    /// Load a saved battlefield (see docs/map-editor-design.md) instead of
    /// today's fully-random layout - border walls, the player fortress, and
    /// enemy spawns stay procedural on top of the map's terrain. Loaded (and
    /// validated) eagerly at CLI-parse time, so a missing/malformed map file
    /// fails fast with a clear error instead of silently falling back to
    /// random. With `--editor` the builder opens on this map.
    #[arg(short = 'm', long = "map", value_parser = parse_map)]
    map: Option<bongbong::map::MapFile>,

    /// Start in Build mode - the map builder - instead of playing
    /// (docs/game-editor-fusion.md). The round is set up as usual, so
    /// `PLAY` starts it on the map as edited; `--map` picks the map to
    /// edit.
    #[arg(long = "editor")]
    editor: bool,

    /// Pin the round RNG seed (decimal or 0x-hex) so the round replays
    /// identically - spawn layout, chassis/speed rolls, ground cosmetics
    /// and AI decisions all reproduce, and the R-key/auto restart replays
    /// the *same* round instead of rolling a new one. This is the repro
    /// loop for a round the probe harness flagged: paste the `seed=0x...`
    /// from its ANOMALY line here (with the same `--map`/`--enemies`) to
    /// watch that exact layout play out. See
    /// docs/gameplay-verification-design.md for what a seed does and
    /// doesn't promise (windowed runs share the probe's layout but diverge
    /// over time under variable frame dt).
    #[arg(long = "seed", value_parser = bongbong::parse_seed)]
    seed: Option<u64>,

    /// Load a tuning patch (a JSON object of `{"knob": value}` pairs - the
    /// dev panel's "Copy JSON" output, see docs/runtime-tuning-design.md)
    /// at startup, and keep watching the file: every edit saved to it is
    /// re-applied at the next frame boundary, so any text editor becomes a
    /// tuning UI on native. A malformed file at startup fails fast; a
    /// malformed edit later is reported on stderr and ignored.
    #[arg(long = "tuning")]
    tuning: Option<std::path::PathBuf>,

    /// Port for the local dev server the `bbmcp` MCP adapter drives
    /// (lockstep stepping, snapshots, screenshots, scenario setup - see
    /// docs/dev-server-design.md). Only in `--features dev-tools` native
    /// builds; a port already in use is reported and the game runs without
    /// the server. Falls back to `BONGBONG_DEV_PORT`, then 4747.
    #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
    #[arg(long = "dev-port")]
    dev_port: Option<u16>,

    /// Run without the dev server even though it is compiled in.
    #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
    #[arg(long = "no-dev-server")]
    no_dev_server: bool,
}

fn parse_map(s: &str) -> Result<bongbong::map::MapFile, String> {
    bongbong::map::MapFile::load(std::path::Path::new(s))
}

/// The battlefield a normal (non-`--editor`) round loads when `-m`/`--map`
/// wasn't given. Embedded into the binary at compile time (`include_str!`)
/// rather than read from `maps/default.toml` on disk at startup - neither
/// the wasm/web build's emscripten virtual filesystem nor a cargo-dist
/// native release archive bundles anything outside `static/` (see
/// `MapFile::from_toml_str`'s doc comment and CLAUDE.md's Web/wasm build and
/// Releases sections), so a disk read here would fail in both of this
/// project's actual distribution paths - it only ever worked when run from
/// a `cargo run` checkout with `maps/` sitting right there. `cargo watch -x
/// "run"` still picks up edits to the on-disk `maps/default.toml` live in
/// dev, since `include_str!` makes rustc treat it as a compile input and
/// trigger a rebuild.
fn default_map() -> bongbong::map::MapFile {
    let mut map = bongbong::map::MapFile::from_toml_str(include_str!("../maps/default.toml"))
        .expect("failed parsing the embedded default map");
    map.name = Some("default".to_string());
    map
}

/// Parses a `WxH` string (e.g. `1920x1080`) into a `(width, height)` pair,
/// validating both parts are positive integers.
fn parse_resolution(s: &str) -> Result<(i32, i32), String> {
    let (w, h) = s
        .split_once('x')
        .ok_or_else(|| format!("invalid resolution '{s}': expected format WxH, e.g. 1920x1080"))?;
    let width: i32 = w
        .trim()
        .parse()
        .map_err(|_| format!("invalid resolution '{s}': width '{w}' is not a valid integer"))?;
    let height: i32 = h
        .trim()
        .parse()
        .map_err(|_| format!("invalid resolution '{s}': height '{h}' is not a valid integer"))?;
    if width <= 0 || height <= 0 {
        return Err(format!(
            "invalid resolution '{s}': width and height must be positive"
        ));
    }
    Ok((width, height))
}

// raylib's PLATFORM_WEB build defaults to OpenGL ES2, and the iOS build is
// raylib on its SDL backend with the ES 2.0 renderer (tools/setup_ios.sh);
// both only accept GLSL ES 100 shaders - desktop's `#version 330` files
// won't compile there. static/web/ holds GLSL ES 100 ports of the same
// effects (see CLAUDE.md).
fn shader_path(name: &str) -> String {
    if bongbong::EMBEDDED { format!("static/web/{name}") } else { format!("static/{name}") }
}


/// Native-only tuning transport: re-apply `--tuning <file>` whenever its
/// mtime changes (polled every `POLL_FRAMES` frames - a stat, not a read),
/// so editing the JSON in any editor is a live tuning UI. Web gets the same
/// effect through the page's panel and capi.rs instead.
struct TuningWatch {
    path: std::path::PathBuf,
    last_modified: Option<std::time::SystemTime>,
    frames: u32,
}

impl TuningWatch {
    const POLL_FRAMES: u32 = 30;

    fn new(path: &std::path::Path) -> Self {
        Self {
            path: path.to_path_buf(),
            last_modified: std::fs::metadata(path).and_then(|m| m.modified()).ok(),
            frames: 0,
        }
    }

    fn poll(&mut self) {
        self.frames += 1;
        if self.frames < Self::POLL_FRAMES {
            return;
        }
        self.frames = 0;
        let Ok(modified) = std::fs::metadata(&self.path).and_then(|m| m.modified()) else {
            return;
        };
        if self.last_modified == Some(modified) {
            return;
        }
        self.last_modified = Some(modified);
        match tuning::submit_file(&self.path) {
            Ok(n) => eprintln!("[tuning] reloaded {n} knob(s) from {}", self.path.display()),
            Err(e) => eprintln!("[tuning] ignored edit: {e}"),
        }
    }
}

/// The three ripple post-effects at one field size (their UV maths is in
/// field pixels, so a field of another size needs them reloaded).
fn load_ripples(rl: &mut RaylibHandle, thread: &sola_raylib::prelude::RaylibThread, width: i32, height: i32) -> (RippleFx, RippleFx, RippleFx) {
    let shock = RippleFx::load(
        rl,
        thread,
        &shader_path("shockwave.fs"),
        width,
        height,
        RippleTuning {
            speed: tuning().shockwave_speed,
            width: tuning().shockwave_width,
            strength: tuning().shockwave_strength * tuning().screen_fx_intensity,
            duration: tuning().shockwave_duration,
        },
    );
    let muzzle = RippleFx::load(
        rl,
        thread,
        &shader_path("muzzle_flash.fs"),
        width,
        height,
        RippleTuning {
            speed: tuning().muzzle_flash_speed,
            width: tuning().muzzle_flash_width,
            strength: tuning().muzzle_flash_strength,
            duration: tuning().muzzle_flash_duration,
        },
    );
    let impact = RippleFx::load(
        rl,
        thread,
        &shader_path("impact.fs"),
        width,
        height,
        RippleTuning {
            speed: tuning().impact_flash_speed,
            width: tuning().impact_flash_width,
            strength: tuning().impact_flash_strength,
            duration: tuning().impact_flash_duration,
        },
    );
    (shock, muzzle, impact)
}

fn main() {
    #[cfg(not(target_os = "ios"))]
    run(Args::parse());
    // iOS: UIKit owns the process. SDL starts the application and calls
    // back into `ios::app_main`, which runs the game with default options
    // from inside the app bundle.
    #[cfg(target_os = "ios")]
    ios::main();
}

/// The whole game, from window to loop, for one set of options. `main`
/// fills them from the command line on desktop; the iOS entry runs them
/// with the defaults.
fn run(args: Args) {
    // Keep the `dev-tools` C API (src/capi.rs) linked into this binary: an
    // `extern "C"` function nobody here references is fair game for the
    // linker to drop before emcc's EXPORTED_FUNCTIONS (build.rs) can export
    // it. See `capi::keep_alive`.
    #[cfg(feature = "dev-tools")]
    bongbong::capi::keep_alive();

    // The battlefield is the map's (`MapFile::field_size`); the bitmap the
    // game draws is that field plus the HUD bar above it
    // (docs/hud-and-builder-layout-design.md), and the window is whatever
    // the player makes it - the bitmap is fitted into it by `view::View`.
    // The simulation, the physics, the maps and the probe only ever see
    // the field.
    let map = args.map.clone().unwrap_or_else(default_map);
    let (screen_width, screen_height) = {
        let (w, h) = map.field_size();
        (w.round() as i32, h.round() as i32)
    };
    let bitmap = Layout::for_field(screen_width as f32, screen_height as f32).window_size();
    // iOS: the window is the screen, and raylib's SDL backend sizes its
    // render target from the size InitWindow is asked for (it never reads
    // the window back), so the screen's point size has to go in here.
    #[cfg(target_os = "ios")]
    let (window_width, window_height) = ios::screen_size_points().unwrap_or(bitmap);
    #[cfg(not(target_os = "ios"))]
    let (window_width, window_height) = args.resolution.unwrap_or(bitmap);

    let mut builder = sola_raylib::init();
    builder.size(window_width, window_height).title(&format!("BongBong! v{}", env!("CARGO_PKG_VERSION")));
    // On the web the canvas box keeps the bitmap's own shape (see
    // index.astro) and raylib maps a touch against that box, so the
    // canvas must stay the bitmap's size: a resizable web window would
    // follow the tab instead. On iOS the SDL backend draws a 1x
    // framebuffer whatever the flags say, so high DPI must not be asked
    // for (the viewport would cover a corner of a 3x drawable). Native
    // windows resize freely and draw at the panel's real density.
    if !bongbong::EMBEDDED {
        builder.resizable().highdpi();
    }
    #[cfg(target_os = "ios")]
    builder.fullscreen();
    let (mut rl, thread) = builder.build();
    #[cfg(target_os = "ios")]
    ios::route_default_framebuffer(&mut rl);
    if !bongbong::EMBEDDED {
        // Half the bitmap is the smallest window that still reads.
        rl.set_window_min_size(bitmap.0 / 2, bitmap.1 / 2);
        if args.fullscreen {
            rl.toggle_borderless_windowed();
        }
    }
    // raylib closes the window on Esc by default; here Esc keeps playing
    // in the leave dialog and dismisses a builder menu, so it must never
    // reach `window_should_close`.
    rl.set_exit_key(None);

    let tanks_texture = rl
        .load_texture(&thread, "static/scifi_tanks_sheet.png")
        .expect("failed loading tanks texture");
    let shells_texture = rl
        .load_texture(&thread, "static/shells.png")
        .expect("failed loading shells texture");
    let plasma_texture = rl
        .load_texture(&thread, "static/plasma.png")
        .expect("failed loading plasma texture");
    let minigun_bullets_texture = rl
        .load_texture(&thread, "static/minigun_bullets.png")
        .expect("failed loading minigun bullets texture");
    let grass_texture = rl
        .load_texture(&thread, "static/nature_sheet.png")
        .expect("failed loading grass texture");
    let trees_texture = rl
        .load_texture(&thread, "static/trees_sheet.png")
        .expect("failed loading trees texture");
    let minigun_mount_texture = rl
        .load_texture(&thread, "static/minigun_mount.png")
        .expect("failed loading minigun mount texture");
    let damage_texture = rl
        .load_texture(&thread, "static/damage.png")
        .expect("failed loading damage texture");
    let tracks_texture = rl
        .load_texture(&thread, "static/tracks.png")
        .expect("failed loading tracks texture");
    let obstacles_texture = rl
        .load_texture(&thread, "static/walls_sheet.png")
        .expect("failed loading obstacles texture");
    let props_texture = rl
        .load_texture(&thread, "static/props_sheet.png")
        .expect("failed loading props texture");
    let barrel_explosion_texture = rl
        .load_texture(&thread, "static/barrel_explosion.png")
        .expect("failed loading barrel explosion texture");
    let ground_texture = rl
        .load_texture(&thread, "static/punyworld/punyworld-overworld-tileset.png")
        .expect("failed loading ground texture");
    // One full clip set per colour variant (see `frog::FROG_VARIANT_DIRS`) -
    // `Frog::variant` (rolled per round in `Game::init`) picks which one
    // `game.rs::render` draws from. Loaded up front like every other
    // texture, kept alive for the whole game loop.
    let frog_textures: Vec<bongbong::frog::FrogVariantTextures> = bongbong::frog::FROG_VARIANT_DIRS
        .iter()
        .map(|dir| bongbong::frog::FrogVariantTextures {
            idle: rl
                .load_texture(&thread, &format!("static/toxic_frog/{dir}/idle.png"))
                .expect("failed loading frog idle texture"),
            hurt: rl
                .load_texture(&thread, &format!("static/toxic_frog/{dir}/hurt.png"))
                .expect("failed loading frog hurt texture"),
            hop: rl
                .load_texture(&thread, &format!("static/toxic_frog/{dir}/hop.png"))
                .expect("failed loading frog hop texture"),
            attack: rl
                .load_texture(&thread, &format!("static/toxic_frog/{dir}/attack.png"))
                .expect("failed loading frog attack texture"),
            explosion: rl
                .load_texture(&thread, &format!("static/toxic_frog/{dir}/explosion.png"))
                .expect("failed loading frog explosion texture"),
        })
        .collect();
    let pickup_health_texture = rl
        .load_texture(&thread, "static/pickups/health.png")
        .expect("failed loading health pickup texture");
    let pickup_ammo_texture = rl
        .load_texture(&thread, "static/pickups/ammo.png")
        .expect("failed loading ammo pickup texture");
    let pickup_laser_texture = rl
        .load_texture(&thread, "static/pickups/laser.png")
        .expect("failed loading laser pickup texture");
    let pickup_minigun_texture = rl
        .load_texture(&thread, "static/pickups/minigun.png")
        .expect("failed loading minigun pickup texture");
    let pickup_plasma_texture = rl
        .load_texture(&thread, "static/pickups/plasma.png")
        .expect("failed loading plasma pickup texture");
    let pickup_speedup_texture = rl
        .load_texture(&thread, "static/pickups/speedup.png")
        .expect("failed loading speed-up pickup texture");
    let pickup_shield_texture = rl
        .load_texture(&thread, "static/pickups/shield.png")
        .expect("failed loading shield pickup texture");
    let pickup_flamethrower_texture = rl
        .load_texture(&thread, "static/pickups/flamethrower.png")
        .expect("failed loading flamethrower pickup texture");
    let eraser_texture = rl
        .load_texture(&thread, "static/ui/eraser.png")
        .expect("failed loading eraser texture");

    let (mut shock_fx, mut muzzle_fx, mut impact_fx) = load_ripples(&mut rl, &thread, screen_width, screen_height);
    // The short-lived particle layer lives here rather than on `Game`:
    // it is presentation only, so nothing in the simulation can see it and
    // it is free to use `rand::rng()` (see fx.rs). The web build starts at
    // a lower density - wasm is the tighter budget and a dense wave is
    // where that shows.
    let mut fx = bongbong::fx::Fx::default();
    if bongbong::EMBEDDED {
        // A literal patch of one known knob: it cannot fail, and there is
        // nothing sensible to do at startup if it somehow did.
        let _ = tuning::submit_json(r#"{"fx_density": 0.5}"#);
    }

    let mut scene_target = rl
        .load_render_texture(&thread, screen_width as u32, screen_height as u32)
        .expect("failed creating scene render texture");
    // The composited bitmap - the field plus the bar - that `view::present`
    // fits into the window. Both targets are re-created when the field
    // changes size (a dev-server `restart` on a map of another size, or
    // the builder loading one).
    let mut composite = rl
        .load_render_texture(&thread, bitmap.0 as u32, bitmap.1 as u32)
        .expect("failed creating composite render texture");
    let mut target_field = (screen_width as f32, screen_height as f32);
    let mut touch = bongbong::touch::TouchScheme::default();
    let touch_from_mouse = args.touch_from_mouse;

    if let Some(path) = &args.tuning {
        match tuning::submit_file(path) {
            Ok(n) => eprintln!("[tuning] loaded {n} knob(s) from {}", path.display()),
            Err(e) => {
                eprintln!("[tuning] {e}");
                std::process::exit(2);
            }
        }
        tuning::apply_pending();
    }
    let mut tuning_watch = args.tuning.as_deref().map(TuningWatch::new);

    let mut game = Game::default();
    game.enemy_count_override = args.enemies;
    game.level_overrides = bongbong::level::LevelOverrides {
        mission: args.mission,
        spawn: args.spawn,
        waves: args.waves,
        wave_size: args.wave_size,
        wave_growth: args.wave_growth,
        tier_start: args.tier_start,
        tier_end: args.tier_end,
    };
    game.show_intro = true;
    game.player_row_override = args.tank.map(TankKind::row);
    game.player2_row_override = args.tank2.map(TankKind::row);
    game.players = PlayerCount::from_count(args.players as usize).expect("clap limits --players to 1 or 2");
    game.shadows_enabled = !args.no_shadows;
    game.seed_override = args.seed;
    game.map = map;
    game.init(screen_width as f32, screen_height as f32);

    // The two modes (docs/game-editor-fusion.md): the round and the map
    // builder, whichever is live. `--editor` starts on the builder.
    let mut session = Session::new(game);
    session.builder.cli_overrides = CliOverrides {
        tanks: args.enemies.is_some(),
        tank: args.tank.is_some(),
        tank2: args.tank2.is_some(),
        mission: args.mission.is_some(),
        spawn: args.spawn.is_some(),
        waves: args.waves.is_some(),
        wave_size: args.wave_size.is_some(),
        wave_growth: args.wave_growth.is_some(),
        tier_start: args.tier_start.is_some(),
        tier_end: args.tier_end.is_some(),
    };
    if args.editor {
        session.driver = Driver::Build;
    }

    // The dev server is serviced at the frame boundary below, like the
    // tuning transports; failing to bind is a warning, not a fatal error.
    #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
    let mut dev: Option<bongbong::devserver::DevServer> = if args.no_dev_server {
        None
    } else {
        let port = args
            .dev_port
            .or_else(|| std::env::var("BONGBONG_DEV_PORT").ok()?.parse().ok())
            .unwrap_or(bongbong::devserver::DEFAULT_PORT);
        match bongbong::devserver::DevServer::start(port) {
            Ok(server) => {
                eprintln!("[dev] listening on 127.0.0.1:{} (bbmcp / just mcp-call)", server.port());
                Some(server)
            }
            Err(e) => {
                eprintln!("[dev] could not bind 127.0.0.1:{port}: {e} - running without the dev server");
                None
            }
        }
    };

    // A held finger is reported as a touch-point *count*, not as a press, so
    // the press edge has to be found here - one tap must produce exactly one
    // builder stroke or button press.
    let mut touch_held_last_frame = false;

    // game_loop::run drives a plain `while !window_should_close()` loop on
    // native, and hands this closure to emscripten's main loop on web - same
    // source for both, and no -sASYNCIFY=1 needed to keep the browser tab
    // responsive (see .cargo/config.toml).
    //
    // **The fps argument means something different on each side.** On native
    // it is `SetTargetFPS`, a cap. On web it picks the main loop's *driver*:
    // `emscripten_set_main_loop_arg` takes any `fps > 0` as a request for
    // `EM_TIMING_SETTIMEOUT` at `1000/fps` ms, and only `0` selects
    // `EM_TIMING_RAF` (emscripten's `libeventloop.js`, `setMainLoop`). A
    // timer is not synced to the display refresh, so a finished frame waits
    // an arbitrary slice of a refresh interval before it is shown - which
    // the player feels as lag between a tap and the tank answering it - the
    // phase drifts, which reads as judder, and browsers throttle timers
    // harder than rAF, phones most of all. Rendering 120 ticks/s to a 60 Hz
    // display also throws half the work away.
    // iOS is a plain blocking loop (SDL pumps UIKit's run loop from inside
    // event polling); the simulator ignores the GL swap interval, so a 60
    // cap is what keeps the loop from spinning - a phone's display link
    // paces it anyway.
    let target_fps = if cfg!(target_os = "emscripten") { 0 } else if cfg!(target_os = "ios") { 60 } else { 120 };
    game_loop::run(rl, thread, target_fps, move |rl, thread| {
        // Frame boundary, first: dev-server requests (state reads and
        // writes, tuning patches, an armed step or screenshot), so anything
        // they stage lands in this same frame.
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        if let Some(dev) = &mut dev {
            let (width, height) = session.field_size();
            dev.before_frame(&mut session, width, height);
        }
        // The field is the live mode's map's (a `restart` above may have
        // just swapped it); the bitmap is the field plus the bar, and the
        // view fits that bitmap into whatever the window is right now.
        let (width, height) = session.field_size();
        let layout = Layout::for_field(width, height);
        if (width, height) != target_field {
            let bitmap = layout.window_size();
            scene_target = rl
                .load_render_texture(thread, width as u32, height as u32)
                .expect("failed re-creating scene render texture");
            composite = rl
                .load_render_texture(thread, bitmap.0 as u32, bitmap.1 as u32)
                .expect("failed re-creating composite render texture");
            let (s, m, i) = load_ripples(rl, thread, width as i32, height as i32);
            shock_fx = s;
            muzzle_fx = m;
            impact_fx = i;
            target_field = (width, height);
        }
        let view = View::fit(
            {
                let (w, h) = layout.window_size();
                (w as f32, h as f32)
            },
            (rl.get_screen_width() as f32, rl.get_screen_height() as f32),
        );
        if !bongbong::EMBEDDED && rl.is_key_pressed(KeyboardKey::KEY_F11) {
            rl.toggle_borderless_windowed();
        }
        // Then land any tuning edits staged since last frame (dev panel
        // via capi.rs, the `--tuning` file watch, or the dev server)
        // before the simulation reads the table, so a frame never sees two
        // values of one knob. The ripple shaders cache their knobs as
        // uniforms, so re-upload those only when something actually changed.
        if let Some(watch) = &mut tuning_watch {
            watch.poll();
        }
        if tuning::apply_pending() {
            let t = tuning::current();
            shock_fx.set_tuning(RippleTuning {
                speed: t.shockwave_speed,
                width: t.shockwave_width,
                strength: t.shockwave_strength * t.screen_fx_intensity,
                duration: t.shockwave_duration,
            });
            muzzle_fx.set_tuning(RippleTuning {
                speed: t.muzzle_flash_speed,
                width: t.muzzle_flash_width,
                strength: t.muzzle_flash_strength,
                duration: t.muzzle_flash_duration,
            });
            impact_fx.set_tuning(RippleTuning {
                speed: t.impact_flash_speed,
                width: t.impact_flash_width,
                strength: t.impact_flash_strength,
                duration: t.impact_flash_duration,
            });
        }
        // Raw pointer state, shared by both modes. Touch is edge-detected
        // by hand because raylib reports a held finger as a point count,
        // not a press.
        let touching = rl.get_touch_point_count() > 0;
        let touch_pressed = touching && !touch_held_last_frame;
        touch_held_last_frame = touching;
        let mouse_pressed = rl.is_mouse_button_pressed(sola_raylib::prelude::MouseButton::MOUSE_BUTTON_LEFT);
        let mouse_held = rl.is_mouse_button_down(sola_raylib::prelude::MouseButton::MOUSE_BUTTON_LEFT);
        // Every pointer is read in bitmap pixels: the bar, the dialogs and
        // the builder hit-test there and never learn what the window is.
        let pointer = view.to_bitmap(if touching { rl.get_touch_position(0) } else { rl.get_mouse_position() });
        // This frame's touch points for the touch scheme, ids included so
        // a stick follows its own finger. `--touch-from-mouse` stands a
        // held left button in for one.
        let mut touch_points: Vec<TouchPoint> = (0..rl.get_touch_point_count())
            .map(|i| TouchPoint { id: rl.get_touch_point_id(i), pos: view.to_bitmap(rl.get_touch_position(i)) })
            .collect();
        if touch_from_mouse && mouse_held && touch_points.is_empty() {
            touch_points.push(TouchPoint { id: -1, pos: view.to_bitmap(rl.get_mouse_position()) });
        }
        let steer_right = tuning().touch_steer_side != 0;
        let pressed = mouse_pressed || touch_pressed;
        let held = mouse_held || touching;
        let tab = rl.is_key_pressed(KeyboardKey::KEY_TAB);
        let ctrl = rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL)
            || rl.is_key_down(KeyboardKey::KEY_LEFT_SUPER)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_SUPER);
        let dt = rl.get_frame_time();

        match session.mode() {
            Driver::Play => {
                // The two buttons and the two dialogs come first: a press
                // on any of them is never a tank order. Nothing here
                // touches the simulation - a frozen round is one whose
                // `update` is not called (see `Session::playing`).
                if session.players_dialog {
                    let rects = players_dialog_rects(layout.field);
                    let field_p = layout.to_field(pointer);
                    if pressed {
                        if rects.one.check_collision_point_rec(field_p) {
                            session.answer_players(PlayerCount::One);
                        } else if rects.two.check_collision_point_rec(field_p) {
                            session.answer_players(PlayerCount::Two);
                        } else if !rects.panel.check_collision_point_rec(field_p) {
                            session.close_players_dialog();
                        }
                    }
                    if rl.is_key_pressed(KeyboardKey::KEY_ONE) {
                        session.answer_players(PlayerCount::One);
                    } else if rl.is_key_pressed(KeyboardKey::KEY_TWO) {
                        session.answer_players(PlayerCount::Two);
                    } else if rl.is_key_pressed(KeyboardKey::KEY_ENTER) {
                        // Enter is the action, as it is "leave" in the other
                        // dialog: the point of opening this one is to switch.
                        let other = match session.game.players {
                            PlayerCount::One => PlayerCount::Two,
                            PlayerCount::Two => PlayerCount::One,
                        };
                        session.answer_players(other);
                    } else if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) || tab {
                        session.close_players_dialog();
                    }
                } else if session.dialog {
                    let rects = leave_dialog_rects(layout.field);
                    let field_p = layout.to_field(pointer);
                    if pressed {
                        if rects.leave.check_collision_point_rec(field_p) {
                            session.answer_dialog(true);
                        } else if rects.stay.check_collision_point_rec(field_p)
                            || !rects.panel.check_collision_point_rec(field_p)
                        {
                            session.answer_dialog(false);
                        }
                    }
                    if rl.is_key_pressed(KeyboardKey::KEY_ENTER) {
                        session.answer_dialog(true);
                    } else if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) || tab {
                        session.answer_dialog(false);
                    }
                } else if tab || (pressed && mode_button_rect(layout.panel).check_collision_point_rec(pointer)) {
                    session.press_build();
                } else if pressed && players_button_rect(layout.panel).check_collision_point_rec(pointer) {
                    session.press_players();
                }
            }
            Driver::Build => {
                let mut typed = String::new();
                while let Some(c) = rl.get_char_pressed() {
                    typed.push(c);
                }
                let input = BuilderInput {
                    pointer: Some(pointer),
                    pressed,
                    held,
                    right_pressed: rl.is_mouse_button_pressed(sola_raylib::prelude::MouseButton::MOUSE_BUTTON_RIGHT),
                    right_held: rl.is_mouse_button_down(sola_raylib::prelude::MouseButton::MOUSE_BUTTON_RIGHT),
                    wheel: rl.get_mouse_wheel_move(),
                    escape: rl.is_key_pressed(KeyboardKey::KEY_ESCAPE),
                    enter: rl.is_key_pressed(KeyboardKey::KEY_ENTER),
                    backspace: rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE),
                    undo: ctrl && rl.is_key_pressed(KeyboardKey::KEY_Z),
                    redo: ctrl && rl.is_key_pressed(KeyboardKey::KEY_Y),
                    typed,
                };
                if tab {
                    session.toggle();
                } else {
                    session.update_builder(&input, &layout);
                }
            }
        }

        if session.mode() == Driver::Build {
            // Nothing is steering while the builder is up, but the scheme
            // still sees the frame so a finger lifted here is not a stick
            // still held when play resumes.
            touch.update(&touch_points, &layout, steer_right, dt);
            session.builder.render(
                rl,
                thread,
                &mut composite,
                &view,
                &layout,
                &EditorTextures {
                    obstacles: &obstacles_texture,
                    props: &props_texture,
                    ground: &ground_texture,
                    grass: &grass_texture,
                    trees: &trees_texture,
                    // Palette icon: the first colour variant's idle frame -
                    // a fixed representative sprite, since the builder
                    // places a frog *cell*, not a rolled colour.
                    frog_idle: &frog_textures[0].idle,
                    pickup_health: &pickup_health_texture,
                    pickup_ammo: &pickup_ammo_texture,
                    pickup_laser: &pickup_laser_texture,
                    pickup_minigun: &pickup_minigun_texture,
                    pickup_plasma: &pickup_plasma_texture,
                    pickup_speedup: &pickup_speedup_texture,
                    pickup_shield: &pickup_shield_texture,
                    pickup_flamethrower: &pickup_flamethrower_texture,
                    eraser: &eraser_texture,
                    tanks: &tanks_texture,
                },
            );
            // The presented frame is the builder; a pending `screenshot`
            // reads it from the screen here, or the client waits forever.
            #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
            if let Some(dev) = &mut dev {
                dev.after_render(rl, thread, &scene_target, &session.game);
            }
            return;
        }

        // Gather this frame's raw input into a plain `Input` - `Game::update`
        // itself decides what to do with it (e.g. whether a wreck can move),
        // so nothing simulation-related needs to know a `RaylibHandle`
        // exists. See simulation.rs's module doc comment.
        let (mut player_intent, player2_intent) = gather_intents(rl, session.game.players);
        // A touch screen drives player 1 through the same intent the
        // keyboard does; a held key still wins the direction, a tap or a
        // key both fire. Fed every frame, dialog or not, so a lifted
        // finger is never a stick still held.
        let touch_intent = touch.update(&touch_points, &layout, steer_right, dt);
        player_intent.move_dir = player_intent.move_dir.or(touch_intent.move_dir);
        player_intent.fire = player_intent.fire || touch_intent.fire;
        let input = Input {
            player_intent,
            player2_intent,
            pause_pressed: rl.is_key_pressed(KeyboardKey::KEY_P),
            // The dev panel's "Restart round" button lands here too, as if
            // R had been pressed - the simulation never learns a browser
            // exists.
            restart_pressed: rl.is_key_pressed(KeyboardKey::KEY_R) || tuning::take_restart_request(),
            toggle_shadows_pressed: rl.is_key_pressed(KeyboardKey::KEY_L),
            // The I key is inert in a release build: overlays are dev-only.
            cycle_overlays_pressed: cfg!(feature = "dev-tools") && rl.is_key_pressed(KeyboardKey::KEY_I),
        };
        // Injected input (the dev server's `input` tool) replaces the
        // keyboard's intent for as many frames as it asked.
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        let input = match &mut dev {
            Some(dev) => dev.shape_input(input),
            None => input,
        };

        // With the dev server attached it owns the advance: real-time
        // updates normally, lockstep `step`s at the fixed timestep when
        // asked, nothing at all while frozen. While the leave dialog is up
        // nobody advances.
        if session.playing() {
            #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
            let advanced = match &mut dev {
                Some(dev) => {
                    dev.advance(&mut session.game, input, dt, width, height);
                    true
                }
                None => false,
            };
            #[cfg(not(all(feature = "dev-tools", not(target_os = "emscripten"))))]
            let advanced = false;
            if !advanced {
                session.game.update(input, dt, width, height);
            }
        }
        let game = &session.game;
        // Between the update and the draw: the particle layer reads the
        // frame's events and the world it just produced, then ages what is
        // already in flight. Deliberately not inside `Game` - see fx.rs.
        fx.observe(game, dt);
        fx.tick(dt);
        game.render(
            rl,
            thread,
            &mut scene_target,
            &mut composite,
            &view,
            &mut Effects {
                shock: &mut shock_fx,
                muzzle: &mut muzzle_fx,
                impact: &mut impact_fx,
                fx: &fx,
                touch: Some((&touch, steer_right)),
            },
            &Textures {
                tanks: &tanks_texture,
                shells: &shells_texture,
                plasma: &plasma_texture,
                minigun_bullets: &minigun_bullets_texture,
                damage: &damage_texture,
                tracks: &tracks_texture,
                obstacles: &obstacles_texture,
                props: &props_texture,
                barrel_explosion: &barrel_explosion_texture,
                ground: &ground_texture,
                frog_variants: &frog_textures,
                pickup_health: &pickup_health_texture,
                pickup_ammo: &pickup_ammo_texture,
                pickup_laser: &pickup_laser_texture,
                pickup_minigun: &pickup_minigun_texture,
                pickup_plasma: &pickup_plasma_texture,
                pickup_speedup: &pickup_speedup_texture,
                pickup_shield: &pickup_shield_texture,
                pickup_flamethrower: &pickup_flamethrower_texture,
                minigun_mount: &minigun_mount_texture,
                grass: &grass_texture,
                trees: &trees_texture,
            },
            &layout,
            &session.play_chrome(),
        );
        // A pending screenshot reads the frame just presented.
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        if let Some(dev) = &mut dev {
            dev.after_render(rl, thread, &scene_target, &session.game);
        }
    });
}

/// The iOS entry (docs/ios-native-port-prd.md): the game is raylib on its
/// SDL3 backend, and on iOS SDL has to start the application itself -
/// `SDL_RunApp` runs UIApplicationMain and calls `app_main` from the app
/// delegate once the app has launched. Everything here is plain SDL3 ABI,
/// declared by hand because sola-raylib does not bind SDL.
#[cfg(target_os = "ios")]
mod ios {
    use clap::Parser as _;
    use std::ffi::{c_char, c_int, c_void};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::OnceLock;

    #[repr(C)]
    struct SdlRect {
        x: c_int,
        y: c_int,
        w: c_int,
        h: c_int,
    }

    const SDL_INIT_VIDEO: u32 = 0x20;

    type BindFn = unsafe extern "C" fn(target: u32, id: u32);

    unsafe extern "C" {
        fn SDL_RunApp(
            argc: c_int,
            argv: *mut *mut c_char,
            main_fn: extern "C" fn(c_int, *mut *mut c_char) -> c_int,
            reserved: *mut c_void,
        ) -> c_int;
        fn SDL_Init(flags: u32) -> bool;
        fn SDL_GetPrimaryDisplay() -> u32;
        fn SDL_GetDisplayBounds(display: u32, rect: *mut SdlRect) -> bool;
        fn SDL_GetWindowProperties(window: *mut c_void) -> u32;
        fn SDL_GetNumberProperty(props: u32, name: *const c_char, default: i64) -> i64;
        /// glad's entries for glBindFramebuffer/glBindRenderbuffer inside
        /// libraylib.a: raylib was built with glad loading GL ES through
        /// SDL_GL_GetProcAddress, so every rlgl GL call goes through a
        /// table of writable pointers.
        static mut glad_glBindFramebuffer: Option<BindFn>;
        static mut glad_glBindRenderbuffer: Option<BindFn>;
    }

    static REAL_BIND_FRAMEBUFFER: OnceLock<BindFn> = OnceLock::new();
    static REAL_BIND_RENDERBUFFER: OnceLock<BindFn> = OnceLock::new();
    static SCREEN_FRAMEBUFFER: AtomicU32 = AtomicU32::new(0);
    static SCREEN_RENDERBUFFER: AtomicU32 = AtomicU32::new(0);

    unsafe extern "C" fn bind_framebuffer_routed(target: u32, id: u32) {
        let id = if id == 0 { SCREEN_FRAMEBUFFER.load(Ordering::Relaxed) } else { id };
        if let Some(real) = REAL_BIND_FRAMEBUFFER.get() {
            // SAFETY: the pointer glad loaded; same signature.
            unsafe { real(target, id) }
        }
    }

    unsafe extern "C" fn bind_renderbuffer_routed(target: u32, id: u32) {
        let id = if id == 0 { SCREEN_RENDERBUFFER.load(Ordering::Relaxed) } else { id };
        if let Some(real) = REAL_BIND_RENDERBUFFER.get() {
            // SAFETY: as above.
            unsafe { real(target, id) }
        }
    }

    /// On iOS there is no window-system framebuffer: SDL draws the window
    /// through a framebuffer object and a renderbuffer of its own, and
    /// "object 0" is nothing. rlgl binds framebuffer 0 to get back to the
    /// screen after every render-texture pass, and renderbuffer 0 after
    /// building a render texture's depth buffer - and SDL's swap presents
    /// whichever renderbuffer is bound, so after the first render texture
    /// every frame would be drawn into the void and the swap would fail
    /// with GL_INVALID_OPERATION. Route both 0s to SDL's objects by
    /// wrapping glad's pointers, once the window (and with it SDL's
    /// objects) exists.
    pub fn route_default_framebuffer(rl: &mut sola_raylib::RaylibHandle) {
        const GL_FRAMEBUFFER: u32 = 0x8D40;
        const GL_RENDERBUFFER: u32 = 0x8D41;
        // SAFETY: main thread, right after InitWindow; the glad table is
        // loaded and nothing else writes it.
        unsafe {
            let props = SDL_GetWindowProperties(rl.get_window_handle());
            let fbo = SDL_GetNumberProperty(props, c"SDL.window.uikit.opengl.framebuffer".as_ptr(), 0);
            let rbo = SDL_GetNumberProperty(props, c"SDL.window.uikit.opengl.renderbuffer".as_ptr(), 0);
            if fbo <= 0 || rbo <= 0 {
                eprintln!("bongbong: SDL reports no iOS framebuffer/renderbuffer; drawing to object 0");
                return;
            }
            SCREEN_FRAMEBUFFER.store(fbo as u32, Ordering::Relaxed);
            SCREEN_RENDERBUFFER.store(rbo as u32, Ordering::Relaxed);
            let fb_slot = &raw mut glad_glBindFramebuffer;
            if let Some(real) = (*fb_slot).take() {
                let _ = REAL_BIND_FRAMEBUFFER.set(real);
                *fb_slot = Some(bind_framebuffer_routed);
                real(GL_FRAMEBUFFER, fbo as u32);
            }
            let rb_slot = &raw mut glad_glBindRenderbuffer;
            if let Some(real) = (*rb_slot).take() {
                let _ = REAL_BIND_RENDERBUFFER.set(real);
                *rb_slot = Some(bind_renderbuffer_routed);
                real(GL_RENDERBUFFER, rbo as u32);
            }
        }
    }

    pub fn main() -> ! {
        // SAFETY: called once, on the main thread, before any other SDL use.
        let code = unsafe { SDL_RunApp(0, std::ptr::null_mut(), app_main, std::ptr::null_mut()) };
        std::process::exit(code)
    }

    extern "C" fn app_main(_argc: c_int, _argv: *mut *mut c_char) -> c_int {
        // The bundle is flat (BongBong.app/{bongbong, static/, maps/} -
        // tools/ios/bundle.sh), so with the bundle as the working directory
        // every `static/...` and `maps/...` path resolves as on desktop.
        if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|d| d.to_path_buf())) {
            if let Err(e) = std::env::set_current_dir(&dir) {
                eprintln!("bongbong: cannot enter the app bundle {}: {e}", dir.display());
            }
        }
        super::run(super::Args::parse_from(["bongbong"]));
        0
    }

    /// The screen in points, landscape, read from SDL before raylib's
    /// InitWindow: the SDL backend never re-reads the window size, so the
    /// size InitWindow is asked for is the one it renders at. `SDL_Init` is
    /// reference-counted, so raylib's own call just bumps the count.
    pub fn screen_size_points() -> Option<(i32, i32)> {
        // SAFETY: plain SDL calls on the main thread; the rect is ours.
        unsafe {
            if !SDL_Init(SDL_INIT_VIDEO) {
                return None;
            }
            let display = SDL_GetPrimaryDisplay();
            if display == 0 {
                return None;
            }
            let mut rect = SdlRect { x: 0, y: 0, w: 0, h: 0 };
            if !SDL_GetDisplayBounds(display, &mut rect) || rect.w <= 0 || rect.h <= 0 {
                return None;
            }
            // The app is landscape-only (Info.plist); this early the display
            // may still report portrait bounds, so orient by hand.
            Some((rect.w.max(rect.h), rect.w.min(rect.h)))
        }
    }
}
