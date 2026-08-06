use bongbong::game::{Effects, Game, Textures};
use bongbong::shockwave::{RippleFx, RippleTuning};
use bongbong::{
    IMPACT_FLASH_DURATION, IMPACT_FLASH_SPEED, IMPACT_FLASH_STRENGTH, IMPACT_FLASH_WIDTH,
    MUZZLE_FLASH_DURATION, MUZZLE_FLASH_SPEED, MUZZLE_FLASH_STRENGTH, MUZZLE_FLASH_WIDTH,
    SHOCKWAVE_DURATION, SHOCKWAVE_SPEED, SHOCKWAVE_STRENGTH, SHOCKWAVE_WIDTH,
};
use sola_raylib::core::game_loop;

static DEFAULT_SCREEN_WIDTH: i32 = 1280;
static DEFAULT_SCREEN_HEIGHT: i32 = 720;

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
    let (mut rl, thread) = sola_raylib::init()
        .size(DEFAULT_SCREEN_WIDTH, DEFAULT_SCREEN_HEIGHT)
        .title("bongbong")
        .build();

    let tanks_texture = rl
        .load_texture(&thread, "static/tanks.png")
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

    let mut shock_fx = RippleFx::load(
        &mut rl,
        &thread,
        &shader_path("shockwave.fs"),
        DEFAULT_SCREEN_WIDTH,
        DEFAULT_SCREEN_HEIGHT,
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
        DEFAULT_SCREEN_WIDTH,
        DEFAULT_SCREEN_HEIGHT,
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
        DEFAULT_SCREEN_WIDTH,
        DEFAULT_SCREEN_HEIGHT,
        RippleTuning {
            speed: IMPACT_FLASH_SPEED,
            width: IMPACT_FLASH_WIDTH,
            strength: IMPACT_FLASH_STRENGTH,
            duration: IMPACT_FLASH_DURATION,
        },
    );
    let mut scene_target = rl
        .load_render_texture(
            &thread,
            DEFAULT_SCREEN_WIDTH as u32,
            DEFAULT_SCREEN_HEIGHT as u32,
        )
        .expect("failed creating scene render texture");

    let mut game = Game::default();
    game.init(&rl);

    // game_loop::run drives a plain `while !window_should_close()` loop on
    // native, and hands this closure to emscripten's main loop on web - same
    // source for both, and no -sASYNCIFY=1 needed to keep the browser tab
    // responsive (see .cargo/config.toml).
    game_loop::run(rl, thread, 60, move |rl, thread| {
        game.update(rl);
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
            },
        );
    });
}
