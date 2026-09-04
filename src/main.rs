use bongbong::tuning::tuning;
use bongbong::ai::Intent;
use bongbong::game::{Effects, Textures};
use bongbong::shockwave::{RippleFx, RippleTuning};
use bongbong::simulation::{Game, Input};
use bongbong::tuning;
use bongbong::tank::{Dir, TankKind};
use bongbong::{
    DEFAULT_SCREEN_HEIGHT,
    DEFAULT_SCREEN_WIDTH,
};
use clap::Parser;
use sola_raylib::core::game_loop;
use sola_raylib::prelude::KeyboardKey;

/// Command-line flags for bongbong's native binary. All optional - with none
/// given, behavior matches today's defaults exactly (random enemy count,
/// random player chassis, 1280x720 window, shadows on).
#[derive(Parser)]
#[command(name = "bongbong", about = "A pixelated tank shooter")]
struct Args {
    /// Override the number of enemies spawned this round. Takes precedence
    /// over the loaded map's own `tanks` default (see `-m`/`--map` below and
    /// `map::MapFile::tanks`); with neither given, falls back to a random
    /// count between ENEMY_COUNT_MIN and ENEMY_COUNT_MAX (see lib.rs).
    #[arg(short = 'e', long = "enemies")]
    enemies: Option<usize>,

    /// Force the player's tank to a specific chassis - e.g. `--tank titan`
    /// for the twin-barrel super-heavy, without restarting until it happens
    /// to roll. Outranks every other way a chassis gets picked: the
    /// `player_tank` tuning knob, then the loaded map's own `tank` key, then
    /// (with none of the three set) a random roll each round. Persists
    /// across in-game restarts (R key).
    #[arg(long = "tank", value_enum)]
    tank: Option<TankKind>,

    /// Override the window size, e.g. `--resolution 1920x1080` (default:
    /// 1280x720).
    #[arg(long = "resolution", value_parser = parse_resolution)]
    resolution: Option<(i32, i32)>,

    /// Disable tank/shell drop shadows (on by default). Can also be toggled
    /// at runtime with the L key - see docs/sprite-shadows-design.md.
    #[arg(long = "no-shadows")]
    no_shadows: bool,

    /// Load a saved battlefield (see docs/map-editor-design.md) instead of
    /// today's fully-random layout - border walls, the player fortress, and
    /// enemy spawns stay procedural on top of the map's terrain. Loaded (and
    /// validated) eagerly at CLI-parse time, so a missing/malformed map file
    /// fails fast with a clear error instead of silently falling back to
    /// random. With `--editor` (map-editor builds only), this instead
    /// pre-loads the named map into the editor's canvas.
    #[arg(short = 'm', long = "map", value_parser = parse_map)]
    map: Option<bongbong::map::MapFile>,

    /// Open the dev-only battlefield map editor instead of starting a round
    /// - see docs/map-editor-design.md. Combine with `--map` to edit an
    /// existing map rather than starting from a blank canvas. Only exists in
    /// builds compiled with `--features map-editor`; never present in a
    /// release build.
    #[cfg(feature = "map-editor")]
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
    bongbong::map::MapFile::from_toml_str(include_str!("../maps/default.toml"))
        .expect("failed parsing the embedded default map")
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

// raylib's PLATFORM_WEB build defaults to OpenGL ES2, which only accepts
// GLSL ES 100 shaders - desktop's `#version 330` files won't compile there.
// static/web/ holds GLSL ES 100 ports of the same effects (see CLAUDE.md).
#[cfg(target_os = "emscripten")]
fn shader_path(name: &str) -> String {
    format!("static/web/{name}")
}
#[cfg(not(target_os = "emscripten"))]
fn shader_path(name: &str) -> String {
    format!("static/{name}")
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

fn main() {
    // Keep the `dev-tools` C API (src/capi.rs) linked into this binary: an
    // `extern "C"` function nobody here references is fair game for the
    // linker to drop before emcc's EXPORTED_FUNCTIONS (build.rs) can export
    // it. See `capi::keep_alive`.
    #[cfg(feature = "dev-tools")]
    bongbong::capi::keep_alive();

    let args = Args::parse();
    let (screen_width, screen_height) = args
        .resolution
        .unwrap_or((DEFAULT_SCREEN_WIDTH, DEFAULT_SCREEN_HEIGHT));

    let (mut rl, thread) = sola_raylib::init()
        .size(screen_width, screen_height)
        .title("BongBong!")
        .build();

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
    let ground_texture = rl
        .load_texture(&thread, "static/punyworld/punyworld-overworld-tileset.png")
        .expect("failed loading ground texture");
    let health_bar_texture = rl
        .load_texture(&thread, "static/health_bar.png")
        .expect("failed loading health bar texture");
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
    #[cfg(feature = "map-editor")]
    let eraser_texture = rl
        .load_texture(&thread, "static/ui/eraser.png")
        .expect("failed loading eraser texture");

    // `--editor`: skip `Game`/`simulation::Input` entirely and drive
    // `MapEditor`'s own update/render loop instead - same window, same
    // already-loaded textures, different top-level driver. See
    // docs/map-editor-design.md's "Entering the editor" section. Only
    // compiled into `map-editor`-feature builds.
    #[cfg(feature = "map-editor")]
    if args.editor {
        let mut editor = bongbong::editor::MapEditor::new(args.map, screen_width as f32, screen_height as f32);
        game_loop::run(rl, thread, 60, move |rl, thread| {
            let (width, height) = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);
            if let bongbong::editor::EditorAction::Close = editor.update(rl, width, height) {
                rl.request_quit();
            }
            editor.render(
                rl,
                thread,
                width,
                height,
                &bongbong::editor::EditorTextures {
                    obstacles: &obstacles_texture,
                    ground: &ground_texture,
                    // Editor palette icon: just the first colour variant's
                    // idle frame - a fixed representative sprite, since the
                    // editor places a frog *cell*, not a rolled colour (that
                    // roll only happens per-round, in `Game::init`).
                    frog_idle: &frog_textures[0].idle,
                    pickup_health: &pickup_health_texture,
                    pickup_ammo: &pickup_ammo_texture,
                    pickup_laser: &pickup_laser_texture,
                    pickup_minigun: &pickup_minigun_texture,
                    pickup_plasma: &pickup_plasma_texture,
                    pickup_speedup: &pickup_speedup_texture,
                    pickup_shield: &pickup_shield_texture,
                    eraser: &eraser_texture,
                    tanks: &tanks_texture,
                },
            );
        });
        return;
    }

    let mut shock_fx = RippleFx::load(
        &mut rl,
        &thread,
        &shader_path("shockwave.fs"),
        screen_width,
        screen_height,
        RippleTuning {
            speed: tuning().shockwave_speed,
            width: tuning().shockwave_width,
            strength: tuning().shockwave_strength,
            duration: tuning().shockwave_duration,
        },
    );
    let mut muzzle_fx = RippleFx::load(
        &mut rl,
        &thread,
        &shader_path("muzzle_flash.fs"),
        screen_width,
        screen_height,
        RippleTuning {
            speed: tuning().muzzle_flash_speed,
            width: tuning().muzzle_flash_width,
            strength: tuning().muzzle_flash_strength,
            duration: tuning().muzzle_flash_duration,
        },
    );
    let mut impact_fx = RippleFx::load(
        &mut rl,
        &thread,
        &shader_path("impact.fs"),
        screen_width,
        screen_height,
        RippleTuning {
            speed: tuning().impact_flash_speed,
            width: tuning().impact_flash_width,
            strength: tuning().impact_flash_strength,
            duration: tuning().impact_flash_duration,
        },
    );
    let mut scene_target = rl
        .load_render_texture(&thread, screen_width as u32, screen_height as u32)
        .expect("failed creating scene render texture");

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
    game.player_row_override = args.tank.map(TankKind::row);
    game.shadows_enabled = !args.no_shadows;
    game.seed_override = args.seed;
    game.map = args.map.unwrap_or_else(default_map);
    game.init(screen_width as f32, screen_height as f32);

    // game_loop::run drives a plain `while !window_should_close()` loop on
    // native, and hands this closure to emscripten's main loop on web - same
    // source for both, and no -sASYNCIFY=1 needed to keep the browser tab
    // responsive (see .cargo/config.toml).
    game_loop::run(rl, thread, 120, move |rl, thread| {
        // Frame boundary: land any tuning edits staged since last frame
        // (dev panel via capi.rs, or the `--tuning` file watch) before the
        // simulation reads the table, so a frame never sees two values of
        // one knob. The ripple shaders cache their knobs as uniforms, so
        // re-upload those only when something actually changed.
        if let Some(watch) = &mut tuning_watch {
            watch.poll();
        }
        if tuning::apply_pending() {
            let t = tuning::current();
            shock_fx.set_tuning(RippleTuning {
                speed: t.shockwave_speed,
                width: t.shockwave_width,
                strength: t.shockwave_strength,
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
        // Gather this frame's raw input into a plain `Input` - `Game::update`
        // itself decides what to do with it (e.g. whether a wreck can move),
        // so nothing simulation-related needs to know a `RaylibHandle`
        // exists. See simulation.rs's module doc comment.
        let mut player_intent = Intent::default();
        if rl.is_key_down(KeyboardKey::KEY_UP) {
            player_intent.move_dir = Some(Dir::Up);
        } else if rl.is_key_down(KeyboardKey::KEY_DOWN) {
            player_intent.move_dir = Some(Dir::Down);
        } else if rl.is_key_down(KeyboardKey::KEY_LEFT) {
            player_intent.move_dir = Some(Dir::Left);
        } else if rl.is_key_down(KeyboardKey::KEY_RIGHT) {
            player_intent.move_dir = Some(Dir::Right);
        }
        // Raw held-state - whether this actually fires (edge-triggered for
        // shells, full-auto while a laser is charged) is `Game::update`'s
        // call, not this closure's; see `Input::player_intent`'s doc comment.
        player_intent.fire = rl.is_key_down(KeyboardKey::KEY_SPACE);
        let input = Input {
            player_intent,
            pause_pressed: rl.is_key_pressed(KeyboardKey::KEY_P),
            // The dev panel's "Restart round" button lands here too, as if
            // R had been pressed - the simulation never learns a browser
            // exists.
            restart_pressed: rl.is_key_pressed(KeyboardKey::KEY_R) || tuning::take_restart_request(),
            toggle_shadows_pressed: rl.is_key_pressed(KeyboardKey::KEY_L),
            toggle_inspect_pressed: rl.is_key_pressed(KeyboardKey::KEY_I),
        };
        let dt = rl.get_frame_time();
        let (width, height) = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);

        game.update(input, dt, width, height);
        game.render(
            rl,
            thread,
            &mut scene_target,
            &mut Effects {
                shock: &mut shock_fx,
                muzzle: &mut muzzle_fx,
                impact: &mut impact_fx,
            },
            &Textures {
                tanks: &tanks_texture,
                shells: &shells_texture,
                plasma: &plasma_texture,
                minigun_bullets: &minigun_bullets_texture,
                damage: &damage_texture,
                tracks: &tracks_texture,
                obstacles: &obstacles_texture,
                ground: &ground_texture,
                health_bar: &health_bar_texture,
                frog_variants: &frog_textures,
                pickup_health: &pickup_health_texture,
                pickup_ammo: &pickup_ammo_texture,
                pickup_laser: &pickup_laser_texture,
                pickup_minigun: &pickup_minigun_texture,
                pickup_plasma: &pickup_plasma_texture,
                pickup_speedup: &pickup_speedup_texture,
                pickup_shield: &pickup_shield_texture,
                minigun_mount: &minigun_mount_texture,
            },
        );
    });
}
