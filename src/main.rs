use bongbong::game::Game;

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

    let mut game = Game::default();
    game.init(&rl);

    while !rl.window_should_close() {
        game.update(&rl);
        game.render(
            &mut rl,
            &thread,
            &tanks_texture,
            &shells_texture,
            &damage_texture,
        );
    }
}
