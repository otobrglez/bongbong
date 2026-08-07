use bongbong::ai::Intent;
use bongbong::game::{Effects, Textures};
use bongbong::shockwave::{RippleFx, RippleTuning};
use bongbong::simulation::{Game, Input};
use bongbong::tank::Dir;
use bongbong::{
    IMPACT_FLASH_DURATION, IMPACT_FLASH_SPEED, IMPACT_FLASH_STRENGTH, IMPACT_FLASH_WIDTH,
    MUZZLE_FLASH_DURATION, MUZZLE_FLASH_SPEED, MUZZLE_FLASH_STRENGTH, MUZZLE_FLASH_WIDTH,
    SHOCKWAVE_DURATION, SHOCKWAVE_SPEED, SHOCKWAVE_STRENGTH, SHOCKWAVE_WIDTH,
};
use clap::Parser;
use sola_raylib::core::game_loop;
use sola_raylib::prelude::KeyboardKey;

static DEFAULT_SCREEN_WIDTH: i32 = 1280;
static DEFAULT_SCREEN_HEIGHT: i32 = 720;

/// Command-line flags for bongbong's native binary. All optional - with none
/// given, behavior matches today's defaults exactly (random enemy count,
/// 1280x720 window, shadows on).
#[derive(Parser)]
#[command(name = "bongbong", about = "A pixelated tank shooter")]
struct Args {
    /// Override the number of enemies spawned this round (default: random
    /// between ENEMY_COUNT_MIN and ENEMY_COUNT_MAX, see lib.rs).
    #[arg(short = 'e', long = "enemies")]
    enemies: Option<usize>,

    /// Override the window size, e.g. `--resolution 1920x1080` (default:
    /// 1280x720).
    #[arg(long = "resolution", value_parser = parse_resolution)]
    resolution: Option<(i32, i32)>,

    /// Disable tank/shell drop shadows (on by default). Can also be toggled
    /// at runtime with the L key - see docs/sprite-shadows-design.md.
    #[arg(long = "no-shadows")]
    no_shadows: bool,
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

fn main() {
    let args = Args::parse();
    let (screen_width, screen_height) = args
        .resolution
        .unwrap_or((DEFAULT_SCREEN_WIDTH, DEFAULT_SCREEN_HEIGHT));

    let (mut rl, thread) = sola_raylib::init()
        .size(screen_width, screen_height)
        .title("bongbong")
        .build();

    let tanks_texture = rl
        .load_texture(&thread, "static/tanks_candy.png")
        .expect("failed loading tanks texture");
    let shells_texture = rl
        .load_texture(&thread, "static/shells.png")
        .expect("failed loading shells texture");
    let damage_texture = rl
        .load_texture(&thread, "static/damage.png")
        .expect("failed loading damage texture");
    let tracks_texture = rl
        .load_texture(&thread, "static/tracks.png")
        .expect("failed loading tracks texture");
    let obstacles_texture = rl
        .load_texture(&thread, "static/obstacles.png")
        .expect("failed loading obstacles texture");

    let mut shock_fx = RippleFx::load(
        &mut rl,
        &thread,
        &shader_path("shockwave.fs"),
        screen_width,
        screen_height,
        RippleTuning {
            speed: SHOCKWAVE_SPEED,
            width: SHOCKWAVE_WIDTH,
            strength: SHOCKWAVE_STRENGTH,
            duration: SHOCKWAVE_DURATION,
        },
    );
    let mut muzzle_fx = RippleFx::load(
        &mut rl,
        &thread,
        &shader_path("muzzle_flash.fs"),
        screen_width,
        screen_height,
        RippleTuning {
            speed: MUZZLE_FLASH_SPEED,
            width: MUZZLE_FLASH_WIDTH,
            strength: MUZZLE_FLASH_STRENGTH,
            duration: MUZZLE_FLASH_DURATION,
        },
    );
    let mut impact_fx = RippleFx::load(
        &mut rl,
        &thread,
        &shader_path("impact.fs"),
        screen_width,
        screen_height,
        RippleTuning {
            speed: IMPACT_FLASH_SPEED,
            width: IMPACT_FLASH_WIDTH,
            strength: IMPACT_FLASH_STRENGTH,
            duration: IMPACT_FLASH_DURATION,
        },
    );
    let mut scene_target = rl
        .load_render_texture(&thread, screen_width as u32, screen_height as u32)
        .expect("failed creating scene render texture");

    let mut game = Game::default();
    game.enemy_count_override = args.enemies;
    game.shadows_enabled = !args.no_shadows;
    game.init(screen_width as f32, screen_height as f32);

    // game_loop::run drives a plain `while !window_should_close()` loop on
    // native, and hands this closure to emscripten's main loop on web - same
    // source for both, and no -sASYNCIFY=1 needed to keep the browser tab
    // responsive (see .cargo/config.toml).
    game_loop::run(rl, thread, 60, move |rl, thread| {
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
        player_intent.fire = rl.is_key_pressed(KeyboardKey::KEY_SPACE);
        let input = Input {
            player_intent,
            pause_pressed: rl.is_key_pressed(KeyboardKey::KEY_P),
            restart_pressed: rl.is_key_pressed(KeyboardKey::KEY_R),
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
                damage: &damage_texture,
                tracks: &tracks_texture,
                obstacles: &obstacles_texture,
            },
        );
    });
}
