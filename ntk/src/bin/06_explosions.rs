//! Demo 06 - explosions, on the real bongbong engine.
//!
//! A white field. Build mode: the game's own map builder with the drum tools
//! - click to place a barrel, click it again (or right-click) to remove it.
//! P (or Tab) plays: the round is initialised from the map and a click on a
//! barrel sets it off. Everything after that is the engine as shipped: the
//! fireball sheet, the particle layer, the scorch and rubble decals, the
//! shockwave shader, camera shake, fuses chaining to the next barrel, a fuel
//! drum launching, an oil drum's burning pool.
//!
//! What the game crate needed for this (four small hooks, nothing else):
//! `Game::plain_canvas` (skip the ground tileset), `Game::hide_players` (the
//! engine always spawns a player tank; we keep it parked, immortal and
//! undrawn), `Game::barrels` and `Game::debug_detonate` (the click).
//!
//! Talking points:
//! - The simulation has no raylib in it: `Game::update` takes an `Input`
//!   and a dt. Rendering is a separate pass that reads state.
//! - The builder edits a `MapFile`, never live entities; `play` re-inits
//!   the round from it. R restarts from the same map.
//! - Effects are layered: sprite animation, particles (owned by the app,
//!   not the simulation), a full-screen shader pass, camera shake.

use bongbong::editor::{BuilderFrame, BuilderInput, EditorTextures, Tool};
use bongbong::fx::Fx;
use bongbong::render::game::{Effects, Textures};
use bongbong::hud::{Fade, PlayChrome, UiFrame};
use bongbong::level::{LevelOverrides, Mission};
use bongbong::map::{CellObject, MapFile};
use bongbong::mode::{Driver, Session};
use bongbong::obstacle::Drum;
use bongbong::render::shockwave::{RippleFx, RippleTuning};
use bongbong::simulation::debug::TankPatch;
use bongbong::simulation::{Game, Input};
use bongbong::tuning::tuning;
use bongbong::view::{Camera, View};
use bongbong::{Layout, Position, HUD_BAR_HEIGHT};
use raylib::core::game_loop;
use raylib::prelude::*;

/// The field in 32 px cells: 1088 x 448 px - the game's own width, so the
/// builder's bar fits, and roughly the other demos' height.
const COLS: f32 = 34.0;
const ROWS: f32 = 14.0;
/// Where the (hidden) player tank is parked: a corner cell.
const PARK: (i32, i32) = (1, 1);

fn main() {
    let mut map = MapFile::new();
    map.size = Some((COLS, ROWS));
    map.tanks = Some(0); // no enemies
    map.mission.kind = Mission::Destroy; // no frog either
    map.set_cell(PARK.0, PARK.1, CellObject::Start);
    let (width, height) = map.field_size();
    let (w, h) = (width as i32, height as i32);

    let (mut rl, thread) = init()
        .size(w, h + HUD_BAR_HEIGHT)
        .title("NTK 06 - Explosions")
        .build();
    rl.set_exit_key(None);

    // Everything the renderer and the builder can draw - loaded once.
    let load = |rl: &mut RaylibHandle, path: &str| rl.load_texture(&thread, path).unwrap_or_else(|_| panic!("{path}"));
    let tanks = load(&mut rl, "static/scifi_tanks_sheet.png");
    let shells = load(&mut rl, "static/shells.png");
    let plasma = load(&mut rl, "static/plasma.png");
    let minigun_bullets = load(&mut rl, "static/minigun_bullets.png");
    let grass = load(&mut rl, "static/nature_sheet.png");
    let trees = load(&mut rl, "static/trees_sheet.png");
    let towers = load(&mut rl, "static/towers_sheet.png");
    let crates = load(&mut rl, "static/crates_sheet.png");
    let pickup_glyphs = load(&mut rl, "static/pickup_glyphs.png");
    let portal = load(&mut rl, "static/portal_sheet.png");
    let tank_glow = load(&mut rl, "static/scifi_tanks_glow.png");
    let tank_modules = load(&mut rl, "static/tank_modules.png");
    let tank_modules_glow = load(&mut rl, "static/tank_modules_glow.png");
    let missile = load(&mut rl, "static/missile.png");
    let tracks = load(&mut rl, "static/tracks.png");
    let obstacles = load(&mut rl, "static/walls_sheet.png");
    let props = load(&mut rl, "static/props_sheet.png");
    let barrel_explosion = load(&mut rl, "static/barrel_explosion.png");
    let ground = load(&mut rl, "static/punyworld/punyworld-overworld-tileset.png");
    let frog_idle = load(&mut rl, &format!("static/toxic_frog/{}/idle.png", bongbong::frog::FROG_VARIANT_DIRS[0]));
    let eraser = load(&mut rl, "static/ui/eraser.png");

    // The three full-screen ripple shaders the renderer resolves in pass 2.
    let ripple = |rl: &mut RaylibHandle, file: &str, speed: f32, width_: f32, strength: f32, duration: f32| {
        RippleFx::load(rl, &thread, &format!("static/{file}"), RippleTuning { speed, width: width_, strength, duration })
    };
    let (mut shock, mut muzzle, mut impact) = {
        let t = tuning();
        (
            ripple(&mut rl, "shockwave.fs", t.shockwave_speed, t.shockwave_width, t.shockwave_strength * t.screen_fx_intensity, t.shockwave_duration),
            ripple(&mut rl, "muzzle_flash.fs", t.muzzle_flash_speed, t.muzzle_flash_width, t.muzzle_flash_strength, t.muzzle_flash_duration),
            ripple(&mut rl, "impact.fs", t.impact_flash_speed, t.impact_flash_width, t.impact_flash_strength, t.impact_flash_duration),
        )
    };
    let mut fx = Fx::default();

    // Pass 1 target (the field) and two composites, each the bare field:
    // the builder's canvas, whose bar is drawn on the window over its own
    // strip, and play mode's, which the demo's chrome draws no HUD over.
    let mut scene = rl.load_render_texture(&thread, w as u32, h as u32).expect("scene target");
    let mut composite_build = rl.load_render_texture(&thread, w as u32, h as u32).expect("composite");
    // Where the builder draws a zoomed canvas: made when first needed.
    let mut builder_scene = bongbong::editor::render::BuilderScene::default();
    let mut composite_play = rl.load_render_texture(&thread, w as u32, h as u32).expect("composite");
    let layout_play = Layout::bare(width, height);

    let mut game = Game::default();
    game.map = map;
    game.enemy_count_override = Some(0);
    game.level_overrides = LevelOverrides { mission: Some(Mission::Destroy), ..Default::default() };
    game.show_intro = false;
    game.shadows_enabled = false;
    game.plain_canvas = true;
    game.hide_players = true;
    game.init(width, height);

    let mut session = Session::new(game);
    session.driver = Driver::Build;
    session.builder.select_tool(Tool::Drum(Drum::Oil));
    session.builder.plain_canvas = true;

    game_loop::run(rl, thread, 120, move |rl, thread| {
        if rl.is_key_pressed(KeyboardKey::KEY_Q) {
            rl.request_quit();
        }
        let window = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);
        let mouse: bongbong::math::Vec2 = rl.get_mouse_position().into();
        let pressed = rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT);
        let dt = rl.get_frame_time();

        // P or Tab flips modes. Leaving a live round normally asks first;
        // the demo answers "leave" on the spot.
        if rl.is_key_pressed(KeyboardKey::KEY_P) || rl.is_key_pressed(KeyboardKey::KEY_TAB) {
            match session.mode() {
                Driver::Build => {
                    session.play();
                }
                Driver::Play | Driver::Lobby | Driver::Online => {
                    session.press_build();
                    if session.dialog {
                        session.answer_dialog(true);
                    }
                }
            }
        }

        // The builder on the window, its bar along the top and the canvas
        // under it; play's bare field fitted to the window. The demo never
        // opens a room, so the two online drivers cannot come up here; they
        // take the round's own view all the same.
        let map = session.builder.map();
        let build_frame = BuilderFrame::new(UiFrame::plain(window), map.field_size(), map.class(), None);
        let view = match session.mode() {
            Driver::Build => build_frame.view,
            Driver::Play | Driver::Lobby | Driver::Online => View::fit((w as f32, h as f32), window),
        };
        let pointer = view.to_bitmap(mouse);

        let textures = Textures {
            tanks: &tanks,
            tank_glow: &tank_glow,
            tank_modules: &tank_modules,
            tank_modules_glow: &tank_modules_glow,
            shells: &shells,
            plasma: &plasma,
            minigun_bullets: &minigun_bullets,
            missile: &missile,
            grass: &grass,
            trees: &trees,
            towers: &towers,
            crates: &crates,
            pickup_glyphs: &pickup_glyphs,
            portal: &portal,
            tracks: &tracks,
            obstacles: &obstacles,
            props: &props,
            barrel_explosion: &barrel_explosion,
            ground: &ground,
            frog_variants: &[],
            // The demo paints a plain white canvas, which draws no ground
            // and so no floor shade.
            blocks: Vec::new(),
        };

        match session.mode() {
            // BUILD: the game's builder, driven by the same input struct the
            // game fills. 1/2/3 pick the drum tools without the bar.
            Driver::Build => {
                let key = |a: KeyboardKey, b: KeyboardKey| rl.is_key_pressed(a) || rl.is_key_pressed(b);
                if key(KeyboardKey::KEY_ONE, KeyboardKey::KEY_KP_1) {
                    session.builder.select_tool(Tool::Drum(Drum::Oil));
                }
                if key(KeyboardKey::KEY_TWO, KeyboardKey::KEY_KP_2) {
                    session.builder.select_tool(Tool::Drum(Drum::Fuel));
                }
                if key(KeyboardKey::KEY_THREE, KeyboardKey::KEY_KP_3) {
                    session.builder.select_tool(Tool::Prop(bongbong::obstacle::Material::Barrel));
                }
                let ctrl = rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL) || rl.is_key_down(KeyboardKey::KEY_LEFT_SUPER);
                let input = BuilderInput {
                    pointer: Some(mouse),
                    pressed,
                    held: pressed || rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT),
                    right_pressed: rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT),
                    right_held: rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT)
                        || rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_RIGHT),
                    wheel: rl.get_mouse_wheel_move(),
                    escape: rl.is_key_pressed(KeyboardKey::KEY_ESCAPE),
                    enter: rl.is_key_pressed(KeyboardKey::KEY_ENTER),
                    backspace: rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE),
                    undo: ctrl && rl.is_key_pressed(KeyboardKey::KEY_Z),
                    redo: ctrl && rl.is_key_pressed(KeyboardKey::KEY_Y),
                    typed: String::new(),
                    ..BuilderInput::default()
                };
                session.update_builder(&input, &build_frame);
                if session.mode() == Driver::Build {
                    session.builder.render(
                        rl,
                        thread,
                        &mut composite_build,
                        &mut builder_scene,
                        &build_frame,
                        bongbong::math::Color::WHITE,
                        &EditorTextures {
                            obstacles: &obstacles,
                            props: &props,
                            ground: &ground,
                            grass: &grass,
                            trees: &trees,
                            towers: &towers,
                            crates: &crates,
                            pickup_glyphs: &pickup_glyphs,
                            portal: &portal,
                            frog_idle: &frog_idle,
                            eraser: &eraser,
                            tanks: &tanks,
                            shade: None,
                            minimap: None,
                            thumbnails: None,
                        },
                    );
                    return;
                }
            }
            // PLAY: a click on a barrel sets it off; R restarts the round.
            Driver::Play | Driver::Lobby | Driver::Online => {
                if pressed {
                    let field_pos: Position = layout_play.to_field(pointer);
                    let _ = session.game.debug_detonate(field_pos);
                }
                // The parked player: no input, pinned in its corner, never
                // hurt - it is only here because the engine wants one.
                let park = bongbong::map::cell_to_world(PARK.0, PARK.1);
                let _ = session.game.debug_teleport(0, park, Some(0.0));
                let _ = session.game.debug_set_tank(0, &TankPatch { damage: Some(0.0), ..Default::default() });
                let input = Input { restart_pressed: rl.is_key_pressed(KeyboardKey::KEY_R), ..Default::default() };
                if session.playing() {
                    session.game.update(input, dt, width, height);
                }
            }
        }

        let game = &session.game;
        fx.observe(game, dt);
        fx.tick(dt);
        game.render(
            rl,
            thread,
            &mut scene,
            &mut composite_play,
            &view,
            &Camera::whole((width, height)),
            bongbong::math::Color::WHITE,
            &mut Effects { shock: &mut shock, muzzle: &mut muzzle, impact: &mut impact, shots: None, weather: None, fx: &fx, touch: None, indicators: None, minimap: None, margins: None, split: None },
            &textures,
            &layout_play,
            &PlayChrome::default(),
            &UiFrame::plain(window),
            Fade::default(),
        );
    });
}
