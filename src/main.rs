use bongbong::game::Game;
use bongbong::shockwave::RippleFx;
use bongbong::{
    IMPACT_FLASH_DURATION, IMPACT_FLASH_SPEED, IMPACT_FLASH_STRENGTH, IMPACT_FLASH_WIDTH,
    MUZZLE_FLASH_DURATION, MUZZLE_FLASH_SPEED, MUZZLE_FLASH_STRENGTH, MUZZLE_FLASH_WIDTH,
    SHOCKWAVE_DURATION, SHOCKWAVE_SPEED, SHOCKWAVE_STRENGTH, SHOCKWAVE_WIDTH,
};

static DEFAULT_SCREEN_WIDTH: i32 = 1280;
static DEFAULT_SCREEN_HEIGHT: i32 = 720;

fn main() {
    let (mut rl, thread) = sola_raylib::init()
        .size(DEFAULT_SCREEN_WIDTH, DEFAULT_SCREEN_HEIGHT)
        .title("bongbong")
        .build();
    rl.set_target_fps(60);

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
        "static/shockwave.fs",
        DEFAULT_SCREEN_WIDTH,
        DEFAULT_SCREEN_HEIGHT,
        SHOCKWAVE_SPEED,
        SHOCKWAVE_WIDTH,
        SHOCKWAVE_STRENGTH,
        SHOCKWAVE_DURATION,
    );
    let mut muzzle_fx = RippleFx::load(
        &mut rl,
        &thread,
        "static/muzzle_flash.fs",
        DEFAULT_SCREEN_WIDTH,
        DEFAULT_SCREEN_HEIGHT,
        MUZZLE_FLASH_SPEED,
        MUZZLE_FLASH_WIDTH,
        MUZZLE_FLASH_STRENGTH,
        MUZZLE_FLASH_DURATION,
    );
    let mut impact_fx = RippleFx::load(
        &mut rl,
        &thread,
        "static/impact.fs",
        DEFAULT_SCREEN_WIDTH,
        DEFAULT_SCREEN_HEIGHT,
        IMPACT_FLASH_SPEED,
        IMPACT_FLASH_WIDTH,
        IMPACT_FLASH_STRENGTH,
        IMPACT_FLASH_DURATION,
    );
    let mut scene_target = rl
        .load_render_texture(&thread, DEFAULT_SCREEN_WIDTH as u32, DEFAULT_SCREEN_HEIGHT as u32)
        .expect("failed creating scene render texture");

    let mut game = Game::default();
    game.init(&rl);

    while !rl.window_should_close() {
        game.update(&rl);
        game.render(
            &mut rl,
            &thread,
            &mut scene_target,
            &mut shock_fx,
            &mut muzzle_fx,
            &mut impact_fx,
            &tanks_texture,
            &shells_texture,
            &damage_texture,
            &tracks_texture,
        );
    }
}
