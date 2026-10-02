//! `Game::render`: the whole frame with raylib. Pass 1 paints the field
//! into `scene_target` through the three `Canvas` stages of `game.rs`
//! (`paint_floor`, `paint_tiles`, `paint_standing`) with the live-round
//! layers between them - fire, glows, the locate label, projectiles,
//! blasts, airborne debris, particles - drawn on the handle directly; pass
//! 2 blits it at the field origin inside a `Camera2D` (shake, ripples,
//! flashes, banners), then the bar in window space, then the bitmap onto
//! the window through `view::present`. Reads `Game`, never mutates it.
//! `Textures` and `Effects` bundle what a frame draws from; the debug
//! overlays (`dev-tools`) draw in screen space after pass 2.

use sola_raylib::prelude::*;

use crate::text::keys;
use crate::bullet::{Bullet, BulletState};
use crate::canvas::Sheet;
use crate::decal::draw_decal;
use crate::game::PaintOptions;
use crate::hud::{
    version_line, HudModel, PlayChrome, BUILD_COLOR, HUD_STATUS_COLOR, HUD_STATUS_INSET, HUD_STATUS_TEXT_SIZE,
    HUD_VERSION_BOTTOM_INSET, HUD_VERSION_COLOR, HUD_VERSION_RIGHT_INSET,
    HUD_VERSION_TEXT_SIZE,
};
use crate::math::{Color, Rectangle};
use crate::obstacle::{draw_flying_drum, Obstacle};
use crate::render::level_select::draw_level_select;
use crate::render::lobby::{draw_lobby, draw_online_button};
use crate::pickup::PickupKind;
use crate::plasma::{Plasma, PlasmaState};
use crate::render::blast::{draw_fire_glow, draw_flame_glow, draw_fuse_glow};
use crate::pyro;
use crate::render::pyro::Rl;
use crate::render::bullet::{draw_bullet, draw_bullet_light, draw_bullet_shadow};
use crate::render::canvas::{GpuCanvas, Sheets};
use crate::render::decal::draw_decal_shadow;
use crate::render::frog::FrogVariantTextures;
use crate::render::hud::{
    draw_bar, draw_corners, draw_edge_marker, draw_leave_button, draw_leave_dialog, draw_mode_button, draw_players_button,
    draw_players_dialog, draw_restart_button, draw_result, CORNER_HUD_CLEAR,
};
use crate::render::laser::{draw_laser_beam, draw_laser_bloom, draw_laser_flares};
use crate::render::plasma::{draw_plasma, draw_plasma_shadow};
use crate::render::portal::draw_portal_glow;
use crate::missile::Missile;
use crate::render::missile::{draw_missile, draw_missile_exhaust, draw_missile_shadow};
use crate::render::shell::{draw_shell, draw_shell_light, draw_shell_shadow};
use crate::render::shot_fx::{at_nozzle, ground_light};
use crate::render::shot_shaders::ShotShaders;
use crate::render::weather::{Passes, WeatherFrame};
use crate::render::shockwave::{screen_to_ripple_uv, RippleFx};
use crate::render::tank::draw_player_label;
use crate::shell::{Shell, ShellState};
use crate::simulation::{Game, Outcome};
#[cfg(feature = "dev-tools")]
use crate::ai::Ai;
#[cfg(feature = "dev-tools")]
use crate::simulation::Overlays;
use crate::tank::Tank;
#[cfg(feature = "dev-tools")]
use crate::tank::{ActiveWeapon, Dir};
use crate::tuning::tuning;
use crate::view::View;
use crate::{Layout, Position, Rect, SHOCK_MAX};
#[cfg(feature = "dev-tools")]
use crate::math::Vec2;
#[cfg(feature = "dev-tools")]
use crate::pickup::Pickup;
#[cfg(feature = "dev-tools")]
use crate::{HUD_MARGIN, MAX_DAMAGE};

/// The sprite atlases `Game::render` draws from, bundled into one param instead
/// of four so the signature doesn't grow with every new texture.
pub struct Textures<'a> {
    pub tanks: &'a Texture2D,
    /// The tank sheet's light layer (`tank::draw_tank_glow`).
    pub tank_glow: &'a Texture2D,
    /// The weapon modules on the turrets, and their light layer.
    pub tank_modules: &'a Texture2D,
    pub tank_modules_glow: &'a Texture2D,
    pub shells: &'a Texture2D,
    pub plasma: &'a Texture2D,
    pub minigun_bullets: &'a Texture2D,
    pub tracks: &'a Texture2D,
    pub obstacles: &'a Texture2D,
    /// The props sheet (sandbags, barrels, fences) - see `obstacle::Sheet`.
    pub props: &'a Texture2D,
    /// The barrel blast animation and scorch decals - see `blast.rs`.
    pub barrel_explosion: &'a Texture2D,
    pub ground: &'a Texture2D,
    /// One `FrogVariantTextures` per `frog::FROG_VARIANT_DIRS` entry, in the
    /// same order - `render` indexes into this by `Frog::variant`.
    pub frog_variants: &'a [FrogVariantTextures],
    pub pickup_health: &'a Texture2D,
    pub pickup_ammo: &'a Texture2D,
    pub pickup_laser: &'a Texture2D,
    pub pickup_minigun: &'a Texture2D,
    pub pickup_plasma: &'a Texture2D,
    pub pickup_missiles: &'a Texture2D,
    pub pickup_speedup: &'a Texture2D,
    pub pickup_shield: &'a Texture2D,
    pub pickup_flamethrower: &'a Texture2D,
    pub pickup_frog_health: &'a Texture2D,
    /// static/missile.png - a seeker missile in flight (missile.rs).
    pub missile: &'a Texture2D,
    /// The tall-grass sheet of the round's map theme
    /// (`map::Theme::grass_texture_path`, grass.rs); `ground` above is the
    /// theme's ground tileset the same way. `app.rs` picks both per frame.
    pub grass: &'a Texture2D,
    /// static/trees_sheet.png - the two tree species (docs/TREES_SPEC.md).
    pub trees: &'a Texture2D,
    /// static/towers_sheet.png - the defence towers (docs/TOWERS_SPEC.md).
    pub towers: &'a Texture2D,
    pub pickup_tower_pack: &'a Texture2D,
    /// static/portal_sheet.png - the turning spiral (portal.rs).
    pub portal: &'a Texture2D,
    /// The round's floor shade as `app.rs` uploaded it before the frame
    /// (`render::canvas::BlockTexture`), with the stamp it was baked under.
    pub shade: Option<(u64, &'a Texture2D)>,
}

/// The game's `Sheet` lookup: what a `GpuCanvas` over these textures blits
/// from. Every sheet the field can name is here.
impl Sheets for Textures<'_> {
    fn blocks_texture(&self, stamp: u64) -> Option<&Texture2D> {
        self.shade.filter(|(held, _)| *held == stamp).map(|(_, texture)| texture)
    }

    fn texture(&self, sheet: Sheet) -> &Texture2D {
        match sheet {
            // `app.rs` picks `ground`/`grass` from the round's map theme
            // every frame, the same theme `paint_floor`/`paint_standing`
            // name here, so the payload needs no second lookup.
            Sheet::Ground(_) => self.ground,
            Sheet::Tanks => self.tanks,
            Sheet::TankGlow => self.tank_glow,
            Sheet::TankModules => self.tank_modules,
            Sheet::TankModulesGlow => self.tank_modules_glow,
            Sheet::Walls => self.obstacles,
            Sheet::Props => self.props,
            Sheet::Trees => self.trees,
            Sheet::Towers => self.towers,
            Sheet::Grass(_) => self.grass,
            Sheet::Tracks => self.tracks,
            Sheet::BarrelExplosion => self.barrel_explosion,
            Sheet::Portal => self.portal,
            Sheet::Pickup(kind) => match kind {
                PickupKind::Health => self.pickup_health,
                PickupKind::Ammo => self.pickup_ammo,
                PickupKind::Laser => self.pickup_laser,
                PickupKind::Minigun => self.pickup_minigun,
                PickupKind::Plasma => self.pickup_plasma,
                PickupKind::Missiles => self.pickup_missiles,
                PickupKind::SpeedUp => self.pickup_speedup,
                PickupKind::Shield => self.pickup_shield,
                PickupKind::Flamethrower => self.pickup_flamethrower,
                PickupKind::FrogHealth => self.pickup_frog_health,
                PickupKind::TowerPack => self.pickup_tower_pack,
            },
            Sheet::Frog { variant, clip } => self.frog_variants[variant as usize % self.frog_variants.len()].clip(clip),
        }
    }
}
/// The ripple post-effects `Game::render` drives, bundled into one param for the
/// same reason as `Textures`. See `shockwave.rs` for what each one does.
pub struct Effects<'a> {
    pub shock: &'a mut RippleFx,
    pub muzzle: &'a mut RippleFx,
    pub impact: &'a mut RippleFx,
    /// The plasma orb and flame jet shaders (`render/shot_shaders.rs`);
    /// `None` flies the baked plasma sprite and draws the flamethrower from
    /// its particles alone.
    pub shots: Option<&'a mut crate::render::shot_shaders::ShotShaders>,
    /// The weather's light map and passes (`render/weather.rs`,
    /// docs/weather.md); `None` draws every sky clear.
    pub weather: Option<&'a mut crate::render::weather::WeatherFx>,
    /// The short-lived particle layer. Owned by `main.rs`, not by `Game`
    /// (see `fx.rs`), and read-only here - `render` never mutates it, the
    /// same contract it has with `Game`.
    pub fx: &'a crate::fx::Fx,
    /// The touch scheme's feedback (`touch.rs`), drawn over the field in
    /// bitmap space when a keyboard-less device is playing; `None` draws
    /// nothing. The `bool` is whether the stick lives on the right half.
    pub touch: Option<(&'a crate::touch::TouchScheme, bool)>,
}

impl Game {
    /// The light shots throw on the ground (inside the first additive
    /// block, under every sprite): a soft pool in the shot's colour under
    /// each shell, bullet, plasma orb and missile, along the flamethrower's
    /// stream, where a laser burns, and a flash of it at every muzzle and
    /// impact - so the floor itself says where the fire is. `k` scales the
    /// lot: 1 by day, less where the weather's light map lights the ground
    /// around the shots instead (`WeatherFrame::day_light_pools`).
    fn draw_ground_light(&self, d: &mut impl RaylibDraw, k: f32) {
        if k <= 0.0 {
            return;
        }
        let ground_light = |d: &mut _, at: Position, radius: f32, color: Color, strength: f32| ground_light(d, at, radius, color, strength * k);
        let warm = Color::new(255, 140, 50, 255);
        for shell in self.world.query::<&Shell>().iter() {
            if shell.state == ShellState::Flying {
                ground_light(d, shell.position, 30.0, warm, 0.28);
            }
        }
        for bullet in self.world.query::<&Bullet>().iter() {
            if bullet.state == BulletState::Flying {
                ground_light(d, bullet.position, 12.0, Color::new(255, 200, 90, 255), 0.14);
            }
        }
        for plasma in self.world.query::<&Plasma>().iter() {
            let [_, body, _, _] = plasma.variant.orb_colors();
            match plasma.impact_progress() {
                Some(p) => ground_light(d, plasma.position, 26.0 + 30.0 * p, body, 0.35 * (1.0 - p)),
                None if plasma.state == PlasmaState::Flying => ground_light(d, plasma.position, 44.0, body, 0.4),
                None => {}
            }
        }
        for missile in self.world.query::<&Missile>().iter() {
            ground_light(d, missile.position, 26.0, warm, 0.3 * (1.0 - 0.6 * missile.lift()));
        }
        for jet in self.flames() {
            let flicker = 0.8 + 0.2 * (self.time * 29.0).sin();
            let (from, dir, reach) = jet.drawn();
            for (along, r) in [(0.35, 0.3), (0.75, 0.45)] {
                let at = Position::new(from.x + dir.x * reach * along, from.y + dir.y * reach * along);
                ground_light(d, at, reach * r, warm, 0.3 * flicker);
            }
        }
        for beam in &self.laser_beams {
            let k = (beam.timer / tuning().laser_beam_display_seconds).clamp(0.0, 1.0);
            ground_light(d, beam.end, 36.0, Color::new(255, 90, 70, 255), 0.5 * k);
        }
        for flash in &self.muzzle_flashes {
            let k = 1.0 - (flash.time / tuning().muzzle_flash_duration.max(0.01)).clamp(0.0, 1.0);
            ground_light(d, flash.center, 32.0, Color::new(255, 190, 90, 255), 0.28 * k);
        }
        for flash in &self.impact_flashes {
            let k = 1.0 - (flash.time / tuning().impact_flash_duration.max(0.01)).clamp(0.0, 1.0);
            ground_light(d, flash.center, 38.0 - 10.0 * k, warm, 0.3 * k);
        }
        if !self.towers.is_empty() || !self.ooze.is_empty() || !self.tesla_bolts.is_empty() {
            self.draw_towers_ground_light(d, k);
        }
    }
}

impl Game {
    /// Draw the whole scene for this frame: the battlefield into
    /// `scene_target`, then that plus the HUD bar into `composite` - the
    /// bitmap, `layout.window_size()` in size, the field at `layout.field`
    /// and the bar in `layout.panel` - and finally the bitmap onto the
    /// window through `view` (`view::present`), scaled and centred so the
    /// whole battlefield is on screen whatever the window is.
    ///
    /// `camera` is the follow camera's rectangle of the battlefield
    /// (`camera::FollowCamera`) on a field map, `None` for an arena, which
    /// is drawn whole. Either way pass 1 draws the whole battlefield into
    /// `scene_target`; pass 2 blits the camera's part of it into
    /// `layout.field`. Everything placed in the world in pass 2 - the
    /// ripple quads, the debug overlays - goes through a world `Camera2D`
    /// that maps battlefield pixels onto that rectangle, so the
    /// simulation's positions stay usable as they are; the banners, the
    /// dims and the dialogs go through a UI one over `layout.field`.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        scene_target: &mut RenderTexture2D,
        composite: &mut RenderTexture2D,
        view: &View,
        backdrop: Color,
        effects: &mut Effects,
        textures: &Textures,
        layout: &Layout,
        chrome: &PlayChrome,
        camera: Option<Rect>,
    ) {
        // The part of the screen the field is drawn in: the whole
        // battlefield for an arena, the camera's viewport on a field map.
        let screen_width = layout.field.w.round() as i32;
        let screen_height = layout.field.h.round() as i32;
        // The battlefield itself - `scene_target`'s size - which the
        // ripples' coordinates and the flipped source rects are taken in.
        let field_w = scene_target.texture.width as f32;
        let field_h = scene_target.texture.height as f32;
        let camera = camera.unwrap_or(Rect::new(0.0, 0.0, field_w, field_h));
        // Must be read off the handle out here: the draw closures below
        // borrow it, and the dev label needs the number.
        #[cfg(feature = "dev-tools")]
        let frame_ms = rl.get_frame_time() * 1000.0;
        let hud = HudModel::gather(self, chrome.seat);
        let t = crate::text::text();
        // The build stamp, bottom-right of the field (text width must be
        // measured on the RaylibHandle, outside the draw closure).
        let version = version_line();
        let version_w = rl.measure_text(&version, HUD_VERSION_TEXT_SIZE);

        // Precompute the centered end-of-round banner (text width must be
        // measured on the RaylibHandle, outside the draw closure).
        let banner = match self.outcome {
            Outcome::Playing => None,
            Outcome::Won => Some((t.get(keys::ROUND_WON), Color::DARKGREEN)),
            Outcome::Lost => Some((t.get(keys::ROUND_LOST), Color::MAROON)),
        };
        // Opening mission banner: solid while the round is frozen behind
        // it, then fading over INTRO_FADE_SECONDS once play starts - and
        // gone once the round is decided, whose screen it would ghost.
        let intro = {
            let alpha = if self.intro_timer > 0.0 {
                1.0
            } else {
                (self.intro_fade / crate::simulation::INTRO_FADE_SECONDS).clamp(0.0, 1.0)
            };
            (alpha > 0.0 && self.outcome == Outcome::Playing).then(|| {
                let text = t.get(crate::text::mission_banner(self.mission));
                let size = 72;
                let w = rl.measure_text(&text, size);
                (text, size, w, alpha)
            })
        };
        // Wave rounds: the `WAVE N` banner during the breather before a
        // wave - smaller than the mission banner, no dim overlay, and never
        // over the end-of-round banner. The counter itself is in the bar.
        let wave_banner = self.wave_banner().filter(|_| self.outcome == Outcome::Playing).map(|banner| {
            let text = if banner.is_final { t.get(keys::WAVE_FINAL) } else { t.fmt(keys::WAVE_BANNER, &[("n", banner.next.into())]) };
            let size = 48;
            let w = rl.measure_text(&text, size);
            (text, size, w)
        });
        let banner = banner.map(|(text, color)| {
            let title_size = 72;
            let title_w = rl.measure_text(&text, title_size);
            let seconds = self.restart_timer.ceil().max(0.0) as i32;
            let sub = t.fmt(chrome.countdown_label.unwrap_or(keys::ROUND_RESTARTING), &[("seconds", seconds.into())]);
            let sub_size = 28;
            let sub_w = rl.measure_text(&sub, sub_size);
            (text, color, title_size, title_w, sub, sub_size, sub_w)
        });
        let paused = t.get(keys::PAUSED);
        let paused_w = rl.measure_text(&paused, 72);
        // An online round's one line of chrome (its width is measured
        // here like every other string, outside the draw closures).
        let status = chrome.status.as_ref().map(|line| {
            let w = rl.measure_text(line, HUD_STATUS_TEXT_SIZE);
            (line.as_str(), w)
        });

        // Pass 1: draw the world (tracks, tanks, shells) into an offscreen
        // render texture, so a shockwave can distort the finished frame as a
        // whole-screen shader pass in pass 2. Under a sky (docs/weather.md,
        // render/weather.rs) it runs in stages that ping-pong between
        // `scene_target` and the weather's own target and always end in
        // `scene_target`: the field under the light, multiplied by the
        // light map, then what shines by itself, then the air over it all.
        let weather = match effects.weather.as_deref_mut() {
            Some(fx) => fx.begin(rl, thread, self, effects.fx, textures),
            None => None,
        };
        match weather {
            None => {
                rl.draw_texture_mode(thread, scene_target, |mut d| {
                    d.clear_background(Color::WHITE);
                    self.paint_field_lit(&mut d, textures, None, layout.corners);
                    self.paint_field_glowing(&mut d, textures, effects.shots.as_deref_mut(), effects.fx, 1.0);
                });
            }
            Some(frame) => {
                let (lit_target, mut passes) = effects.weather.as_deref_mut().and_then(|fx| fx.stages()).expect("`begin` readied the targets");
                let plan = frame.plan;
                let day_pools = frame.day_light_pools();
                // Each stage draws into the other buffer from the one before
                // it, so the last lands in `scene_target`: under light and
                // sky the first stage starts there, under one of them it
                // starts in the weather's target.
                let first_in_scene = plan.lit == plan.sky;
                {
                    let first: &mut RenderTexture2D = if first_in_scene { &mut *scene_target } else { &mut *lit_target };
                    rl.draw_texture_mode(thread, first, |mut d| {
                        d.clear_background(Color::WHITE);
                        self.paint_field_lit(&mut d, textures, plan.ground.then_some((&mut passes, &frame)), layout.corners);
                        if !plan.lit {
                            self.paint_field_glowing(&mut d, textures, effects.shots.as_deref_mut(), effects.fx, day_pools);
                        }
                    });
                }
                if plan.lit {
                    let (under, to): (&RenderTexture2D, &mut RenderTexture2D) =
                        if first_in_scene { (&*scene_target, &mut *lit_target) } else { (&*lit_target, &mut *scene_target) };
                    rl.draw_texture_mode(thread, to, |mut d| {
                        passes.draw_lit(&mut d, under, &frame);
                        self.paint_field_glowing(&mut d, textures, effects.shots.as_deref_mut(), effects.fx, day_pools);
                    });
                }
                if plan.sky {
                    // Whichever stage came before left the field in the
                    // weather's target.
                    rl.draw_texture_mode(thread, scene_target, |mut d| passes.draw_sky(&mut d, lit_target, &frame));
                }
            }
        }

        // If a shockwave is playing, push its current center/time to the shader
        // before the blit below samples through it.
        // Every live ripple goes up as one set of arrays and resolves in a
        // single blit below - see static/shockwave.fs for why N passes
        // would be both wrong and slower. Unused slots carry gain 0.
        if !self.shocks.is_empty() {
            let mut centers = [Vector2::new(0.0, 0.0); SHOCK_MAX];
            let mut times = [0.0f32; SHOCK_MAX];
            let mut gains = [0.0f32; SHOCK_MAX];
            for (i, shock) in self.shocks.iter().take(SHOCK_MAX).enumerate() {
                centers[i] = screen_to_ripple_uv(shock.center, field_w, field_h);
                times[i] = shock.time;
                gains[i] = shock.strength;
            }
            effects.shock.shader.set_shader_value_v(effects.shock.centers_loc, &centers);
            effects.shock.shader.set_shader_value_v(effects.shock.times_loc, &times);
            effects.shock.shader.set_shader_value_v(effects.shock.gains_loc, &gains);
        }

        // The camera's part of the battlefield, cut to the battlefield
        // where the camera reaches past an edge (a field map narrower than
        // the view on one axis): that band stays the clear colour. The
        // render texture is stored upside-down relative to the screen; a
        // negative source height flips it back on the way out, and the
        // rows are counted from the bottom.
        let cut_x0 = camera.x.max(0.0);
        let cut_y0 = camera.y.max(0.0);
        let cut_x1 = (camera.x + camera.w).min(field_w);
        let cut_y1 = (camera.y + camera.h).min(field_h);
        let source = Rectangle {
            x: cut_x0,
            y: field_h - cut_y1,
            width: (cut_x1 - cut_x0).max(0.0),
            height: -(cut_y1 - cut_y0).max(0.0),
        };

        // Camera shake: a short decaying wobble on the same kill-shockwave
        // trigger as `self.shock` itself (see CAMERA_SHAKE_DURATION's doc
        // comment), applied purely as an offset on this blit's destination
        // rather than a real camera - the game has no camera transform (see
        // `Shockwave::center`'s doc comment), so shifting where the already-
        // composited scene lands on screen is the cheapest way to get the
        // effect. Two out-of-phase sine waves stand in for randomness so x/y
        // don't shake in lockstep - this is a pure draw pass (see this
        // module's own doc comment), not the place to reach for an rng.
        // Muzzle/impact flash quads and the HUD deliberately aren't shifted:
        // they're either their own small on-screen quad or meant to stay put.
        // The field origin is added on top: the scene lands below the bar.
        let mut blit_offset = Vector2::new(0.0, 0.0);
        // `screen_fx_intensity` scales every whole-screen effect together;
        // folding it into the magnitude here keeps the stack cap below
        // proportional.
        let shake_magnitude = tuning().camera_shake_magnitude * tuning().screen_fx_intensity;
        for shock in &self.shocks {
            let decay = (1.0f32 - shock.time / tuning().camera_shake_duration).max(0.0);
            if decay <= 0.0 {
                continue;
            }
            let mag = shake_magnitude * shock.strength * decay;
            // Phase each one off its own position hash so several
            // overlapping shakes interfere instead of beating in lockstep
            // and doubling cleanly. Still a pure draw pass, still no rng -
            // see the comment above.
            let phase = (crate::blast::seed_for(shock.center) % 628) as f32 * 0.01;
            let t = shock.time * tuning().camera_shake_frequency + phase;
            blit_offset.x += t.sin() * mag;
            blit_offset.y += (t * 1.3 + 1.7).sin() * mag;
        }
        // Without a ceiling, three kills at once throw the composited scene
        // far enough off that the screen edge shows through as black.
        let cap = shake_magnitude * tuning().camera_shake_max_stack;
        let len = (blit_offset.x * blit_offset.x + blit_offset.y * blit_offset.y).sqrt();
        if len > cap && len > 0.0 {
            blit_offset.x *= cap / len;
            blit_offset.y *= cap / len;
        }
        // Snap the shake to whole 2px blocks. This offset moves the entire
        // composited scene, so at a fractional value every pixel in the
        // game samples between texels for the duration of the shake and
        // the whole screen shimmers - the same defect a sub-pixel particle
        // has, at the scale of everything at once. Blocks keep the art
        // crisp and make the shake read as a hard jolt rather than a
        // wobble.
        blit_offset.x = (blit_offset.x / 2.0).round() * 2.0;
        blit_offset.y = (blit_offset.y / 2.0).round() * 2.0;

        let origin = layout.field_origin();
        let blit_at = Vector2::new(
            blit_offset.x + origin.x + (cut_x0 - camera.x),
            blit_offset.y + origin.y + (cut_y0 - camera.y),
        );
        // Battlefield pixels onto the screen: whatever sits in the world.
        let world_camera = Camera2D {
            offset: origin.into(),
            target: Vector2::new(camera.x, camera.y),
            rotation: 0.0,
            zoom: 1.0,
        };
        // The field's own rectangle on screen: what covers or centres on
        // the field rather than a place in the world.
        let field_camera = Camera2D {
            offset: origin.into(),
            target: Vector2::new(0.0, 0.0),
            rotation: 0.0,
            zoom: 1.0,
        };
        // On a field map the HUD lies over the field's top-left corner, so
        // what is written along that edge moves below it.
        let top_inset = if layout.corners { CORNER_HUD_CLEAR } else { 0 };

        rl.draw_texture_mode(thread, composite, |mut d| {
            d.clear_background(Color::BLACK);

            if !self.shocks.is_empty() {
                d.draw_shader_mode(&mut effects.shock.shader, |mut sd| {
                    sd.draw_texture_rec(&*scene_target, source, blit_at, Color::WHITE);
                });
            } else {
                d.draw_texture_rec(&*scene_target, source, blit_at, Color::WHITE);
            }

            // The flash quads and the overlays are placed in the world.
            d.draw_mode2D(world_camera, |mut d, _| {
                // Layer each muzzle flash's tiny heat-haze ripple on top, one small
                // quad at a time: source and dest are the same on-screen patch (just
                // re-sampling that bit of the already-composited scene through the
                // ripple shader), so this reads as a localized wobble rather than
                // redistorting the whole frame.
                for flash in &self.muzzle_flashes {
                    let uv = screen_to_ripple_uv(flash.center, field_w, field_h);
                    effects
                        .muzzle
                        .shader
                        .set_shader_value(effects.muzzle.center_loc, uv);
                    effects
                        .muzzle
                        .shader
                        .set_shader_value(effects.muzzle.time_loc, flash.time);

                    let r = tuning().muzzle_flash_quad_radius;
                    let flash_source = Rectangle {
                        x: flash.center.x - r,
                        y: (field_h - flash.center.y) - r,
                        width: r * 2.0,
                        height: -(r * 2.0),
                    };
                    let flash_dest = Rectangle {
                        x: flash.center.x,
                        y: flash.center.y,
                        width: r * 2.0,
                        height: r * 2.0,
                    };
                    let origin = Vector2::new(r, r);

                    d.draw_shader_mode(&mut effects.muzzle.shader, |mut sd| {
                        sd.draw_texture_pro(
                            &*scene_target,
                            flash_source,
                            flash_dest,
                            origin,
                            0.0,
                            Color::WHITE,
                        );
                    });
                }

                // Same treatment for every in-flight shell-impact flash.
                for flash in &self.impact_flashes {
                    let uv = screen_to_ripple_uv(flash.center, field_w, field_h);
                    effects
                        .impact
                        .shader
                        .set_shader_value(effects.impact.center_loc, uv);
                    effects
                        .impact
                        .shader
                        .set_shader_value(effects.impact.time_loc, flash.time);

                    let r = tuning().impact_flash_quad_radius;
                    let flash_source = Rectangle {
                        x: flash.center.x - r,
                        y: (field_h - flash.center.y) - r,
                        width: r * 2.0,
                        height: -(r * 2.0),
                    };
                    let flash_dest = Rectangle {
                        x: flash.center.x,
                        y: flash.center.y,
                        width: r * 2.0,
                        height: r * 2.0,
                    };
                    let origin = Vector2::new(r, r);

                    d.draw_shader_mode(&mut effects.impact.shader, |mut sd| {
                        sd.draw_texture_pro(
                            &*scene_target,
                            flash_source,
                            flash_dest,
                            origin,
                            0.0,
                            Color::WHITE,
                        );
                    });
                }

                // Debug overlays (dev builds only): the two tank layers -
                // hitbox/collider outlines and the stats card, each on its
                // own flag - for every tank, then the dev server's other
                // layers. Drawn here (post-composite) rather than into
                // scene_target, so they're never warped by an in-flight
                // shockwave and always render crisp.
                #[cfg(feature = "dev-tools")]
                {
                    let ov = self.debug_overlays;
                    if ov.hitboxes || ov.stats {
                        for (tank, ai) in self.world.query::<(&Tank, &Ai)>().iter() {
                            draw_tank_layers(&mut d, ov, tank, Some(ai));
                        }
                        for entity in self.players().into_iter().flatten() {
                            crate::simulation::with_tank(&self.world, entity, |tank| {
                                draw_tank_layers(&mut d, ov, tank, None);
                            });
                        }
                    }
                    self.draw_debug_overlays(&mut d, field_w, field_h);
                }
            });

            // Everything from here to the bar covers or centres on the
            // field's rectangle on screen, never the bar.
            d.draw_mode2D(field_camera, |mut d, _| {
                // A kill or a barrel blast opens with a brief whole-screen
                // flash (`Game::screen_flash`, started and spaced out by
                // `Game::flash_screen`). After the ripple quads, which re-blit
                // patches of the un-flashed scene and would otherwise punch
                // darker squares through it.
                if let Some(age) = self.screen_flash {
                    let seconds = tuning().blast_screen_flash_seconds;
                    if seconds > 0.0 && age < seconds {
                        let peak = tuning().blast_screen_flash_alpha * tuning().screen_fx_intensity;
                        let a = (255.0 * peak.clamp(0.0, 1.0) * (1.0 - age / seconds)) as u8;
                        d.draw_rectangle(0, 0, screen_width, screen_height, Color::new(255, 240, 200, a));
                    }
                }

                // A frog off screen on a field map: a marker on the edge
                // of the view toward it, under every dim and dialog.
                if layout.corners {
                    let insets = crate::camera::Insets {
                        top: (CORNER_HUD_CLEAR + 18) as f32,
                        right: 28.0,
                        bottom: 30.0,
                        left: 28.0,
                    };
                    for target in crate::camera::objectives(self) {
                        if let Some(m) = crate::camera::edge_marker(camera, target.pos, insets) {
                            draw_edge_marker(&mut d, m, target, self.time);
                        }
                    }
                }

                #[cfg(feature = "dev-tools")]
                {
                    let ov = self.debug_overlays;
                    // Which preset is live, in the field's top-left corner (the
                    // player's readouts are in the bar, so this corner is free),
                    // so the I key's cycling is visible without counting layers.
                    if let Some(preset) = ov.preset_name() {
                        // Frame time and live particle count ride the same
                        // label: the FX budget has to hold on the wasm build,
                        // and without a number on screen that is an assertion
                        // nobody can check while playing.
                        let label = format!(
                            "DEV overlays: {preset} (I cycles)  |  {:.1} ms  {} fx",
                            frame_ms,
                            effects.fx.live(),
                        );
                        const LABEL_FONT_SIZE: i32 = 14;
                        let label_y = HUD_MARGIN + top_inset;
                        // Same 8px/char width estimate as `draw_tank_stats`'s
                        // panel - no font handle inside the draw closure.
                        let label_w = label.len() as i32 * 8 + 8;
                        d.draw_rectangle(
                            HUD_MARGIN - 4,
                            label_y - 2,
                            label_w,
                            LABEL_FONT_SIZE + 4,
                            Color::new(0, 0, 0, 150),
                        );
                        d.draw_text(
                            &label,
                            HUD_MARGIN,
                            label_y,
                            LABEL_FONT_SIZE,
                            Color::new(80, 200, 255, 255),
                        );
                    }
                }

                // An online round says where it stands along the field's
                // top edge: the room, the seat and how much of the
                // snapshot stream is in hand.
                if let Some((line, width)) = status {
                    d.draw_rectangle(
                        HUD_STATUS_INSET - 4,
                        HUD_STATUS_INSET + top_inset - 3,
                        width + 8,
                        HUD_STATUS_TEXT_SIZE + 6,
                        Color::new(0, 0, 0, 150),
                    );
                    d.draw_text(line, HUD_STATUS_INSET, HUD_STATUS_INSET + top_inset, HUD_STATUS_TEXT_SIZE, HUD_STATUS_COLOR);
                }

                // The build stamp along the field's bottom edge, left of
                // the corner the web page's Full screen button sits in.
                d.draw_text(
                    &version,
                    screen_width - HUD_VERSION_RIGHT_INSET - version_w,
                    screen_height - HUD_VERSION_BOTTOM_INSET - HUD_VERSION_TEXT_SIZE,
                    HUD_VERSION_TEXT_SIZE,
                    HUD_VERSION_COLOR,
                );

                // End-of-round banner over a dimming overlay. A local round
                // stacks its numbers and a level's buttons under it
                // (`hud::result_layout`); an online one counts down to the
                // room's lobby.
                if let Some((title, color, title_size, title_w, sub, sub_size, sub_w)) = &banner {
                    d.draw_rectangle(0, 0, screen_width, screen_height, Color::new(0, 0, 0, 120));
                    let cx = screen_width / 2;
                    let cy = screen_height / 2;
                    match &chrome.result {
                        Some(view) => {
                            let rows = crate::hud::result_layout(layout.field, view);
                            d.draw_text(title, cx - title_w / 2, rows.title_y as i32, *title_size, *color);
                            draw_result(&mut d, layout.field, view, sub);
                        }
                        None => {
                            d.draw_text(title, cx - title_w / 2, cy - title_size, *title_size, *color);
                            d.draw_text(sub, cx - sub_w / 2, cy + 20, *sub_size, Color::RAYWHITE);
                        }
                    }
                }

                // Mission banner: big white text over a dim overlay that both
                // fade together once the round unfreezes. A level adds its
                // number above and its title below, at half the size.
                if let Some((text, size, w, alpha)) = &intro {
                    let a = |max: f32| (max * alpha) as u8;
                    d.draw_rectangle(0, 0, screen_width, screen_height, Color::new(0, 0, 0, a(120.0)));
                    d.draw_text(text, screen_width / 2 - w / 2, screen_height / 2 - size / 2, *size, Color::new(255, 255, 255, a(255.0)));
                    if let Some(level) = &chrome.level {
                        use crate::hud::{LEVEL_NUMBER_SIZE, LEVEL_TITLE_SIZE};
                        let number = t.fmt(keys::LEVEL_NUMBER, &[("n", level.number.into()), ("count", level.count.into())]);
                        let number_w = crate::text::width(&number, LEVEL_NUMBER_SIZE);
                        let number_y = screen_height / 2 - size / 2 - 14 - LEVEL_NUMBER_SIZE;
                        let amber = Color::new(BUILD_COLOR.r, BUILD_COLOR.g, BUILD_COLOR.b, a(255.0));
                        d.draw_text(&number, screen_width / 2 - number_w / 2, number_y, LEVEL_NUMBER_SIZE, amber);
                        let title_w = crate::text::width(&level.title, LEVEL_TITLE_SIZE);
                        let title_y = screen_height / 2 + size / 2 + 14;
                        d.draw_text(&level.title, screen_width / 2 - title_w / 2, title_y, LEVEL_TITLE_SIZE, Color::new(255, 255, 255, a(255.0)));
                    }
                }
                if let Some((text, size, w)) = &wave_banner {
                    d.draw_text(text, screen_width / 2 - w / 2, screen_height / 2 - size / 2, *size, Color::RAYWHITE);
                }

                // Paused overlay draws over everything else, including the
                // end-of-round banner (its countdown is frozen too).
                if self.paused {
                    d.draw_rectangle(0, 0, screen_width, screen_height, Color::new(0, 0, 0, 120));
                    let title_size = 72;
                    d.draw_text(
                        &paused,
                        screen_width / 2 - paused_w / 2,
                        screen_height / 2 - title_size / 2,
                        title_size,
                        Color::RAYWHITE,
                    );
                }

                // The leave-round question (docs/game-editor-fusion.md
                // section 6): the same dim as PAUSED, the dialog on top.
                // The round is frozen by `main.rs` not calling `update`,
                // so nothing here is simulation state.
                if chrome.leave_dialog || chrome.players_dialog {
                    d.draw_rectangle(0, 0, screen_width, screen_height, Color::new(0, 0, 0, 120));
                }
                if chrome.leave_dialog {
                    draw_leave_dialog(&mut d, layout.field);
                } else if chrome.players_dialog {
                    draw_players_dialog(&mut d, layout.field, self.players);
                }
                // The lobby (lobby.rs) stands over the whole field, in
                // the dialogs' field space and with their dim: the round
                // behind it is the local one, frozen because nothing
                // calls `update` in this mode.
                if let Some(lobby) = &chrome.lobby {
                    draw_lobby(&mut d, layout.field, lobby, textures);
                }
                // The level select (level_select.rs), with its own dim,
                // over a round that stands still behind it.
                if let Some(levels) = &chrome.levels {
                    draw_level_select(&mut d, layout.field, levels);
                }
            });

            // The HUD, in window space, over anything the field pass might
            // have put on its edge: the bar above an arena, the corner
            // clusters over a field map.
            let level = chrome.level_button.map(|n| (n, chrome.levels.is_some()));
            if layout.corners {
                draw_corners(&mut d, layout.panel, &hud, textures, level, chrome.buttons_left(layout.panel));
            } else {
                draw_bar(&mut d, layout.panel, &hud, textures, level);
            }
            if chrome.players_button {
                draw_players_button(&mut d, layout.panel, self.players, chrome.players_dialog);
            }
            if chrome.restart_button {
                draw_restart_button(&mut d, layout.panel);
            }
            if chrome.build_button {
                draw_mode_button(&mut d, layout.panel, &t.get(keys::BUTTON_BUILD), BUILD_COLOR);
            }
            if chrome.leave_button {
                draw_leave_button(&mut d, layout.panel);
            }
            if chrome.online_button {
                draw_online_button(&mut d, layout.panel);
            }
            // The touch scheme's stick, ripples and hint: over everything,
            // in bitmap space, so they sit where the thumbs are.
            if let Some((touch, steer_right)) = effects.touch {
                touch.draw(&mut d, layout, steer_right);
            }
        });
        crate::render::view::present(rl, thread, composite, view, backdrop);
    }
}

impl Game {
    /// Every hit's flash (`fx::Flash`), inside an additive blend: the hull
    /// or the tile it landed on drawn again over itself, which doubles its
    /// light - the white-hot frames of a hit. A tile is found under the
    /// point the shot struck.
    fn draw_flashes(&self, c: &mut impl crate::canvas::Canvas, flashes: &[crate::fx::Flash]) {
        use crate::fx::Flashed;
        for flash in flashes {
            let k = flash.strength();
            if k <= 0.0 {
                continue;
            }
            let tint = Color::new(255, 255, 255, (255.0 * k) as u8);
            match flash.what {
                Flashed::Tank(slot) => {
                    for tank in self.world.query::<&crate::tank::Tank>().iter().filter(|t| t.owner_slot() == slot) {
                        crate::tank::draw_tank_hull(c, tank, self.time, tint);
                        crate::tank::draw_tank_turret(c, tank, self.time, tint);
                    }
                }
                Flashed::Tile(at) => {
                    let mut q = self.world.query::<&Obstacle>();
                    let struck = q
                        .iter()
                        .filter(|o| (o.position.x - at.x).abs() <= o.size() * 0.5 + 3.0 && (o.position.y - at.y).abs() <= o.size() * 0.5 + 3.0)
                        .min_by(|a, b| a.position.distance_to(at).total_cmp(&b.position.distance_to(at)));
                    let Some(o) = struck else { continue };
                    if o.material.is_tree() {
                        crate::obstacle::draw_tree_tinted(c, o, 0.0, self.time, tint);
                    } else if o.material.is_tower() {
                        if let Some(v) = self.tower_views().into_iter().find(|v| v.position.distance_to(o.position) < 1.0) {
                            crate::tower::draw_tower_tinted(c, v.kind, v.side, v.position, v.stage, v.heading, tint);
                        }
                    } else {
                        let fences: std::collections::HashSet<(i32, i32)> =
                            self.world.query::<&Obstacle>().iter().filter(|f| f.material == crate::obstacle::Material::Fence).map(|f| f.cell()).collect();
                        crate::obstacle::draw_obstacle_tinted(c, o, crate::obstacle::fence_axis(o, &fences), self.time, tint);
                    }
                }
            }
        }
    }

    /// The field as it stands under the light - under daylight, or under
    /// the weather's light map, which the light pass multiplies it by: the
    /// floor, the fires on it, the tiles and their glows, and everything
    /// standing on it. `ground` is the weather's ground pass, drawn in
    /// place of the bare ground tileset.
    fn paint_field_lit<D: RaylibDraw>(&self, d: &mut D, textures: &Textures, ground: Option<(&mut Passes, &WeatherFrame)>, vitals: bool) {
        // The field itself is painted through the `Canvas` trait in the
        // three stages `mapshot` also runs on a CPU canvas (`paint_floor`,
        // `paint_tiles`, `paint_standing`); between them come the layers
        // only a live round has - fire, glows, the locate label,
        // projectiles, blasts, airborne debris, particles - drawn with
        // raylib directly. A `GpuCanvas` borrows `d` for one statement,
        // so each stage makes its own. Under weather the ground pass
        // draws the ground with the sky's mark on it, and the marks lie
        // over that: tracks in the snow.
        match ground {
            Some((passes, frame)) => {
                passes.draw_ground(d, frame);
                self.paint_floor_marks(&mut GpuCanvas::new(d, textures));
            }
            None => self.paint_floor(&mut GpuCanvas::new(d, textures)),
        }

        // Burning ground cells: tongues of flame standing on each
        // (`pyro::tongues`), leaning with the wind, over the ground, under
        // the tiles beside them (a burning doorway's walls still stand
        // over the fire) and under whatever drives through them. Upper
        // cells first, so a lower cell's flames stand in front.
        let mut cells = self.burning_cells();
        if !cells.is_empty() {
            cells.sort_by(|a, b| a.0.y.total_cmp(&b.0.y).then(a.0.x.total_cmp(&b.0.x)));
            let t = tuning();
            let mut flames = Vec::new();
            for (at, left, total) in cells {
                let dying = (left / 0.6).clamp(0.0, 1.0);
                let catching = ((total - left) / 0.25).clamp(0.0, 1.0);
                let seed = crate::blast::seed_at(at, 23);
                let lean = pyro::smoke_lean(&t, at, self.time);
                let foot = Position::new(at.x, at.y + 9.0);
                pyro::tongues(&mut flames, foot, t.ground_fire_spread_px, t.ground_fire_height_px, t.ground_fire_tongues.max(0) as u32, seed, self.time, lean, dying.min(catching));
            }
            pyro::draw(&mut GpuCanvas::new(d, textures), &flames);
        }

        self.paint_tiles(&mut GpuCanvas::new(d, textures));

        // A barrel whose fuse is lit pulses (additive, so it reads as
        // light on the drum rather than a disc over it).
        d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
            // An active portal glows from below its spiral.
            if self.portals_active() {
                for &at in &self.portals {
                    draw_portal_glow(&mut bd, at, self.time);
                }
            }
            for obstacle in self.world.query::<&Obstacle>().iter() {
                if obstacle.fuse.is_some() {
                    draw_fuse_glow(&mut bd, obstacle.position, self.time);
                }
            }
            for (at, left, _total) in self.burning_cells() {
                draw_fire_glow(&mut bd, at, self.time, left);
            }
            // The flamethrower's nozzle while it fires: a hot disc at
            // the muzzle and a fainter one a third of the way out.
            for jet in self.flames() {
                let (from, dir, reach) = jet.drawn();
                draw_flame_glow(&mut bd, from, dir, reach, self.time);
            }
            // The light every burning deck, wreck and tile throws on the
            // ground round it; the flames themselves stand in
            // `paint_standing` and `paint_tiles`.
            let t = tuning();
            let bands = t.glow_bands.max(0) as u32;
            for tank in self.world.query::<&crate::tank::Tank>().iter() {
                let lean = pyro::smoke_lean(&t, tank.position, self.time);
                pyro::draw_glows(&mut Rl(&mut bd), &crate::damage_stage::flames(tank, self.time, lean), bands);
            }
            for obstacle in self.world.query::<&Obstacle>().iter().filter(|o| o.burning) {
                pyro::draw_glows(&mut Rl(&mut bd), &crate::game::tile_flames(obstacle, self.time), bands);
            }
        });

        self.paint_standing(&mut GpuCanvas::new(d, textures), PaintOptions { locate_cue: true, vitals });
    }

    /// What shines by its own light, over the lit field and so as bright
    /// at night as at noon: the locate labels, the light the shots throw,
    /// the shots, hits, flames and flares, the blasts, whatever is in the
    /// air and the particles. `day_pools` scales the daylight's glow pools
    /// on the ground (`draw_ground_light`), which the light map stands in
    /// for under a dark sky.
    fn paint_field_glowing<D: RaylibDraw>(&self, d: &mut D, textures: &Textures, mut shots: Option<&mut ShotShaders>, fx: &crate::fx::Fx, day_pools: f32) {
        // A hit lights up what it landed on: the hull or the tile drawn
        // again in light for a few frames, stepping down.
        if !fx.flashes().is_empty() {
            d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| self.draw_flashes(&mut GpuCanvas::new(&mut bd, textures), fx.flashes()));
        }
        // The towers' light (a charging coil, a mortar's ooze, a
        // tower on fire) and the globs in the air over everything that
        // stands (docs/defence-towers-prd.md section 12).
        for glob in &self.globs {
            crate::tower::draw_glob(&mut GpuCanvas::new(d, textures), glob);
        }
        d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| self.draw_towers_light(&mut bd));

        // The tanks' own lights - lamps, strips, sensor eyes, the modules'
        // lenses, a wreck's embers - under a dark sky, which multiplied
        // their paint down with everything else (`tank::draw_tank_glow`).
        // `day_pools` is 1 in daylight and falls as the sky's lights rise.
        let tank_glow = 1.0 - day_pools;
        if tank_glow > 0.0 {
            d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
                let mut c = GpuCanvas::new(&mut bd, textures);
                for tank in self.world.query::<&Tank>().iter() {
                    if self.hide_players && tank.is_player() {
                        continue;
                    }
                    crate::tank::draw_tank_glow(&mut c, tank, self.time, tank_glow);
                }
            });
        }

        // The locate cue's P1..P8 labels, over the grass, the crowd and
        // the trees - the point is to be found under all of it.
        if !self.hide_players {
            for entity in self.players().into_iter().flatten() {
                crate::simulation::with_tank(&self.world, entity, |tank| draw_player_label(d, tank, self.time));
            }
        }

        // The light the shots throw, under their sprites so each round
        // sits in its own glow: tracers, halos, the plasma's comet
        // tail, the laser's bloom (`render/shot_fx.rs`). One additive
        // block for all of it - a blend switch breaks the batch.
        let glow = tuning().shot_glow_strength > 0.0;
        if glow {
            d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
                // Light on the floor first: every shot, burn and flash
                // lights the ground around it in its own colour.
                self.draw_ground_light(&mut bd, day_pools);
                for shell in self.world.query::<&Shell>().iter() {
                    draw_shell_light(&mut bd, shell);
                }
                for bullet in self.world.query::<&Bullet>().iter() {
                    draw_bullet_light(&mut bd, bullet);
                }
                for beam in &self.laser_beams {
                    draw_laser_bloom(&mut bd, beam);
                }
            });
        }

        // A hit is the burst the particle layer's impact composes
        // (`burst.rs`), so the baked impact frames are left out.
        for shell in self.world.query::<&Shell>().iter() {
            if self.shadows_enabled && shell.state == ShellState::Flying {
                draw_shell_shadow(d, textures.shells, shell);
            }
            // Only the round in flight: its muzzle frames are the muzzle
            // flash's (`burst::muzzle`) and its impact frames the hit's.
            if shell.state == ShellState::Flying {
                draw_shell(d, textures.shells, shell);
            }
        }

        // A flying bolt is the shader orb where the shaders loaded; its
        // muzzle and impact frames stay the baked sprite.
        let orb = glow && shots.is_some();
        for plasma in self.world.query::<&Plasma>().iter() {
            let flying = plasma.state == PlasmaState::Flying;
            if self.shadows_enabled && flying {
                draw_plasma_shadow(d, textures.plasma, plasma, orb);
            }
            match shots.as_deref_mut() {
                Some(shots) if orb && flying => shots.draw_orb(d, plasma, self.time, self.map.field_size().1),
                _ if !flying => {}
                _ => draw_plasma(d, textures.plasma, plasma),
            }
        }

        for bullet in self.world.query::<&Bullet>().iter() {
            if self.shadows_enabled && bullet.state == BulletState::Flying {
                draw_bullet_shadow(d, textures.minigun_bullets, bullet);
            }
            if bullet.state != BulletState::Hit {
                draw_bullet(d, textures.minigun_bullets, bullet);
            }
        }

        for beam in &self.laser_beams {
            draw_laser_beam(d, beam);
        }
        for bolt in &self.tesla_bolts {
            crate::render::tower::draw_tesla_bolt(d, bolt);
        }
        // Every hit still playing, composed in blocks (`burst.rs`):
        // shell fireballs, bullet and ricochet sparks, plasma and tesla
        // rings, laser burns, splashes of ooze; their light after, in one
        // additive block.
        if !fx.impacts().is_empty() {
            let t = tuning();
            let hits: Vec<Vec<pyro::Shape>> = fx
                .impacts()
                .iter()
                .map(|i| crate::burst::compose(i.kind, i.pos, i.dir, i.age, i.seed, pyro::smoke_lean(&t, i.pos, self.time)))
                .collect();
            {
                let mut c = GpuCanvas::new(d, textures);
                for shapes in &hits {
                    pyro::draw(&mut c, shapes);
                }
            }
            if glow {
                let bands = t.glow_bands.max(0) as u32;
                d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
                    for shapes in &hits {
                        pyro::draw_glows(&mut Rl(&mut bd), shapes, bands);
                    }
                });
            }
        }

        // The flamethrower's jet of burning fuel, over the tanks and
        // under its own flying motes.
        if let Some(shots) = shots.as_deref_mut().filter(|_| glow) {
            for (i, jet) in self.flames().iter().enumerate() {
                let (from, dir, reach) = jet.drawn();
                shots.draw_flame(d, from, dir, reach, self.time, i as f32 * 7.31, self.map.field_size().1);
            }
        }

        // Over the shots: the flash where each leaves the barrel
        // (`burst::muzzle`, thrown down the line of the round it let
        // out) and the burn at each end of a laser. A flamethrower
        // nozzle pushes a muzzle flash every held frame and has its own
        // glow, so it gets no flash.
        if glow {
            let nozzles: Vec<Position> = self.flames().iter().map(|jet| jet.nozzle).collect();
            let flashes: Vec<Vec<pyro::Shape>> = self
                .muzzle_flashes
                .iter()
                .filter(|f| !at_nozzle(f.center, &nozzles))
                .map(|f| {
                    let near = crate::fx::shot_near(self, f.center);
                    crate::burst::muzzle(f.center, near.map(|n| n.0), f.time, crate::blast::seed_for(f.center), near.and_then(|n| n.1))
                })
                .collect();
            {
                let mut c = GpuCanvas::new(d, textures);
                for shapes in &flashes {
                    pyro::draw(&mut c, shapes);
                }
            }
            let bands = tuning().glow_bands.max(0) as u32;
            d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
                for beam in &self.laser_beams {
                    draw_laser_flares(&mut bd, beam);
                }
                for shapes in &flashes {
                    pyro::draw_glows(&mut Rl(&mut bd), shapes, bands);
                }
            });
        }

        // Blasts last, so the fire covers tanks and shots. Each is
        // composed once (`fireball.rs`, or `mushroom.rs` for a cloud),
        // its smoke leaning with the wind where it went off; the puffs
        // oldest first - a chained blast's flash lands on top of the
        // earlier fireball and reads as a second detonation - then all
        // their light in one additive block.
        if !self.blast_fx.is_empty() {
            let t = tuning();
            let blasts: Vec<Vec<pyro::Shape>> = self
                .blast_fx
                .iter()
                .map(|b| {
                    let lean = pyro::smoke_lean(&t, b.center, self.time);
                    match &b.cloud {
                        Some(cloud) => cloud.compose(b.center, b.time, lean),
                        None => crate::fireball::compose(b, lean),
                    }
                })
                .collect();
            {
                let mut c = GpuCanvas::new(d, textures);
                for shapes in &blasts {
                    pyro::draw(&mut c, shapes);
                }
            }
            let bands = t.glow_bands.max(0) as u32;
            d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
                for shapes in &blasts {
                    pyro::draw_glows(&mut Rl(&mut bd), shapes, bands);
                }
            });
        }

        // Parts still in the air, last of all: a chunk of hull thrown
        // off a wreck passes over tanks and shells, not under them.
        // Their shadows go down first so no piece is drawn over
        // another's shadow.
        if self.shadows_enabled {
            for decal in self.decals.iter().filter(|dc| !dc.landed()) {
                draw_decal_shadow(d, decal);
            }
        }
        {
            let mut c = GpuCanvas::new(d, textures);
            for decal in self.decals.iter().filter(|dc| !dc.landed()) {
                draw_decal(&mut c, decal);
            }
            // Launched fuel drums, tumbling over the lot on their way to
            // where they go off.
            for drum in &self.flying_drums {
                draw_flying_drum(&mut c, drum, self.shadows_enabled);
            }
        }

        // Seeker missiles are the highest thing in the air: over the
        // debris, shadows first so none lands on another missile.
        if self.shadows_enabled {
            for missile in self.world.query::<&Missile>().iter() {
                draw_missile_shadow(d, textures.missile, missile);
            }
        }
        if glow && self.world.query::<&Missile>().iter().next().is_some() {
            d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
                for missile in self.world.query::<&Missile>().iter() {
                    draw_missile_exhaust(&mut bd, missile, self.time);
                }
            });
        }
        for missile in self.world.query::<&Missile>().iter() {
            draw_missile(d, textures.missile, missile);
        }

        // Sparks, chips, dust and smoke over the top of everything in
        // the scene, but still inside pass 1 so an in-flight shockwave
        // warps them and the camera shake carries them along.
        crate::render::fx::draw(fx, d);
    }
}

/// One tank's dev overlay layers (`Overlays::hitboxes`, `Overlays::stats`;
/// both in the preset the I key cycles to first): the geometry is read
/// once and shared, the boxes go down first so the card stays on top.
#[cfg(feature = "dev-tools")]
fn draw_tank_layers(d: &mut impl RaylibDraw, ov: Overlays, tank: &Tank, ai: Option<&Ai>) {
    let geo = TankInspectGeometry::of(tank);
    if ov.hitboxes {
        draw_tank_boxes(d, tank, ai, &geo);
    }
    if ov.stats {
        draw_tank_stats(d, tank, ai, &geo);
    }
}

/// What both tank layers read off a tank: the facing-oriented hull rect
/// (the stats card anchors at its top-left, so the card needs it whether
/// or not the box is drawn) and the movement collider's size and corner
/// radius (drawn by the boxes, printed by the card's MOVE line).
#[cfg(feature = "dev-tools")]
struct TankInspectGeometry {
    /// The hull damage box as `(x, y, width, height)` in whole pixels.
    hull: (i32, i32, i32, i32),
    /// `Tank::move_half_extents` for the current facing.
    move_half: (f32, f32),
    /// `physics::tank_corner_radius` for those half extents.
    corner: f32,
}

#[cfg(feature = "dev-tools")]
impl TankInspectGeometry {
    fn of(tank: &Tank) -> Self {
        // Same "which axis is the long one" check as `Tank::avoidance_radius` -
        // tanks only ever face one of the four `Dir::rotation()` values, so an
        // exact match is safe here (no epsilon needed).
        let along_x = tank.rotation == Dir::Right.rotation() || tank.rotation == Dir::Left.rotation();
        let (hx, hy) = tank.hull_half_extents(along_x);
        let hull = (
            (tank.position.x - hx).round() as i32,
            (tank.position.y - hy).round() as i32,
            (hx * 2.0).round() as i32,
            (hy * 2.0).round() as i32,
        );
        let move_half = tank.move_half_extents(along_x);
        let corner = crate::physics::tank_corner_radius(move_half);
        Self { hull, move_half, corner }
    }
}

/// The `hitboxes` overlay for one tank: its hull damage box, its
/// turret+barrel damage box, and its (smaller, corner-rounded) movement
/// collider. Purely diagnostic: reads state, draws it, mutates nothing.
///
/// Three shapes, each the *real* thing physics uses, not an approximation:
/// - The lime/orange/gray box is `Tank::hull_half_extents` - the per-row
///   (TANK_HULL_BBOX_BY_ROW), facing-oriented hull silhouette backing the
///   projectile hit box (see `simulation::hits`) - not the full 32x32 sprite tile
///   (`Tank::size`/`Tank::hull_size`), which is a uniform square padded
///   well past the visible hull art on every row.
/// - The yellow box is `Tank::turret_bbox_world` - the turret+barrel
///   silhouette, hit-tested the same way. Together
///   these two boxes are exactly what a shell can hit.
/// - The sky-blue rounded box is `Tank::move_half_extents` +
///   `physics::tank_corner_radius` - the solid movement collider walls/
///   obstacles/other tanks actually block against, deliberately smaller
///   and rounder than the damage boxes (see TANK_MOVE_BBOX_FRACTION/
///   TANK_MOVE_CORNER_RADIUS in `lib.rs`). The visible gap between blue
///   and lime is the tuning surface: widen it (smaller fraction) for more
///   forgiving driving, shrink it if sprites start visibly clipping into
///   walls. The `stats` card's MOVE line prints its current world-px size
///   and corner radius for the same purpose.
#[cfg(feature = "dev-tools")]
fn draw_tank_boxes(d: &mut impl RaylibDraw, tank: &Tank, ai: Option<&Ai>, geo: &TankInspectGeometry) {
    let (x, y, width, height) = geo.hull;
    let box_color = if tank.is_wreck() {
        Color::GRAY
    } else if ai.is_some_and(Ai::is_retreating) {
        Color::ORANGE
    } else {
        Color::LIME
    };
    d.draw_rectangle_lines(x, y, width, height, box_color);

    // Turret+barrel bbox overlay (`TANK_TURRET_BBOX_BY_ROW`) - drawn in a
    // distinct color purely so it's visually comparable against the hull
    // box above; not the tank's real collider (see that table's own doc
    // comment - the barrel is deliberately excluded from hit detection
    // today, this is here to evaluate whether that should change).
    let (turret_center, turret_half) = tank.turret_bbox_world();
    let tx = (turret_center.x - turret_half.x).round() as i32;
    let ty = (turret_center.y - turret_half.y).round() as i32;
    let tw = (turret_half.x * 2.0).round() as i32;
    let th = (turret_half.y * 2.0).round() as i32;
    d.draw_rectangle_lines(tx, ty, tw, th, Color::YELLOW);

    // What a player's shot is tested against on an enemy: the hull and
    // turret boxes grown by `player_shot_hit_pad_px`
    // (`hits::Terrain::sweep`), so a screenshot shows the forgiveness.
    if ai.is_some() && !tank.is_wreck() {
        let pad = tuning().player_shot_hit_pad_px;
        if pad > 0.0 {
            for (c, h) in [tank.hull_bbox_world(), (turret_center, turret_half)] {
                d.draw_rectangle_lines(
                    (c.x - h.x - pad).round() as i32,
                    (c.y - h.y - pad).round() as i32,
                    ((h.x + pad) * 2.0).round() as i32,
                    ((h.y + pad) * 2.0).round() as i32,
                    Color::new(255, 140, 60, 150),
                );
            }
        }
    }

    // Movement collider (see the doc comment above): drawn with the exact
    // clamped corner radius the physics shape carries
    // (`physics::tank_corner_radius`), mapped onto raylib's relative
    // roundness factor (corner radius = roundness * min(w, h) / 2, so the
    // division below inverts that).
    let (mx, my) = geo.move_half;
    let move_rect = Rectangle::new(
        tank.position.x - mx,
        tank.position.y - my,
        mx * 2.0,
        my * 2.0,
    );
    d.draw_rectangle_rounded_lines(move_rect, geo.corner / mx.min(my), 8, Color::SKYBLUE);
}

/// The `stats` overlay for one tank: a small card above its hull rect -
/// ammo, the live weapon and its ammo, health, current speed and velocity,
/// the movement collider's size - and additionally (`ai: Some`, i.e. this
/// isn't a player) whether it's currently retreating to recharge and its
/// fire cooldown, pulled straight from its `Ai` - the same state `ai.rs`'s
/// `wants_retreat`/`fire_interval` act on. Anchored at the hull box's
/// top-left whether or not the `hitboxes` layer draws that box. Purely
/// diagnostic: reads state, draws it, mutates nothing.
#[cfg(feature = "dev-tools")]
fn draw_tank_stats(d: &mut impl RaylibDraw, tank: &Tank, ai: Option<&Ai>, geo: &TankInspectGeometry) {
    let (x, y, _, _) = geo.hull;
    let (mx, my) = geo.move_half;
    let corner = geo.corner;
    let speed = (tank.velocity.x * tank.velocity.x + tank.velocity.y * tank.velocity.y).sqrt();
    // What the trigger fires right now, with its own remaining ammo -
    // under the FIFO inventory (`Tank::weapon_queue`) this advances when
    // the live weapon runs dry (and a first pickup arms it directly), so
    // surface it here to watch the handover live (WPN SHELL duplicates the
    // AMMO line above; harmless, and it keeps this line self-contained).
    let (wpn_name, wpn_ammo) = match tank.active_weapon() {
        ActiveWeapon::Laser => ("LASER", tank.laser_charges),
        ActiveWeapon::Plasma => ("PLASMA", tank.plasma_ammo),
        ActiveWeapon::Minigun => ("MINIGUN", tank.minigun_ammo),
        ActiveWeapon::Missiles => ("MISSILES", tank.missile_ammo),
        ActiveWeapon::Flamethrower => ("FLAME", tank.flame_fuel_seconds()),
        ActiveWeapon::Shell => ("SHELL", tank.shells_ammo),
    };
    let mut lines = vec![
        format!("AMMO {}", tank.shells_ammo),
        format!("WPN {wpn_name} {wpn_ammo}"),
        format!(
            "HP {}/{}",
            (MAX_DAMAGE - tank.damage).max(0.0).round() as i32,
            MAX_DAMAGE as i32
        ),
        format!("SPD {speed:.0}px/s"),
        format!("VEL ({:.0},{:.0})", tank.velocity.x, tank.velocity.y),
        format!("MOVE {:.0}x{:.0} r{corner:.0}", mx * 2.0, my * 2.0),
    ];
    if let Some(ai) = ai {
        lines.push(format!(
            "RETREAT {}",
            if ai.is_retreating() { "YES" } else { "no" }
        ));
        let cooldown = ai.fire_cooldown();
        lines.push(if cooldown <= 0.0 {
            "FIRE ready".to_string()
        } else {
            format!("FIRE {cooldown:.1}s")
        });
    }

    // No font handle available inside this draw closure to measure text
    // width precisely (RaylibDraw doesn't expose it - only RaylibHandle,
    // outside the closure) - an 8px/char estimate at this font size is close
    // enough for a debug-only backing panel.
    const FONT_SIZE: i32 = 14;
    const LINE_H: i32 = 16;
    let block_w = lines.iter().map(|l| l.len()).max().unwrap_or(0) as i32 * 8 + 8;
    let block_h = lines.len() as i32 * LINE_H + 4;
    let text_y = y - block_h - 2;
    d.draw_rectangle(x, text_y - 2, block_w, block_h, Color::new(0, 0, 0, 150));
    for (i, line) in lines.iter().enumerate() {
        d.draw_text(
            line,
            x + 4,
            text_y + i as i32 * LINE_H,
            FONT_SIZE,
            Color::LIME,
        );
    }
}

#[cfg(feature = "dev-tools")]
impl Game {
    /// The debug overlay layers beyond the two tank layers
    /// (`Game::debug_overlays`, docs/dev-server-design.md; dev builds only),
    /// screen space and post-composite like them: blocked nav cells, each
    /// enemy's AI memory, projectile hit boxes, engagement targets, pickup
    /// collect radii. Each layer costs nothing while off.
    fn draw_debug_overlays(&self, d: &mut impl RaylibDraw, width: f32, height: f32) {
        let ov = self.debug_overlays;
        if ov.nav_grid {
            // The grid the enemies actually steer by this frame, prices
            // and fields included - not the bare `nav_grid`.
            let grid = self.route_grid(width, height);
            let (cols, rows, cell) = grid.dims();
            let size = cell.round() as i32;
            let goal = grid.goals().next().map(|(c, r)| Position::new((c as f32 + 0.5) * cell, (r as f32 + 0.5) * cell));
            for row in 0..rows {
                for col in 0..cols {
                    let x = (col as f32 * cell).round() as i32;
                    let y = (row as f32 * cell).round() as i32;
                    if grid.is_blocked(col, row) {
                        d.draw_rectangle(x, y, size, size, Color::new(255, 40, 40, 60));
                        d.draw_rectangle_lines(x, y, size, size, Color::new(255, 40, 40, 110));
                        continue;
                    }
                    // A priced cell: amber, deeper the dearer, capped so a
                    // ford at 8 and a lane at 4 both read as "priced".
                    let extra = grid.cost_at(col, row).saturating_sub(1);
                    if extra > 0 {
                        let alpha = (40 + extra.min(8) * 18) as u8;
                        d.draw_rectangle(x, y, size, size, Color::new(255, 170, 40, alpha));
                    }
                    // Player 1's flow: a short line from the centre toward
                    // the cell the field steps into, tipped with a dot.
                    if let Some(goal) = goal
                        && let Some((nc, nr)) = grid.flow(goal, col, row)
                    {
                        let (cx, cy) = (x as f32 + cell * 0.5, y as f32 + cell * 0.5);
                        let (dx, dy) = (nc as f32 - col as f32, nr as f32 - row as f32);
                        let (ex, ey) = (cx + dx * cell * 0.35, cy + dy * cell * 0.35);
                        d.draw_line(cx as i32, cy as i32, ex as i32, ey as i32, Color::new(80, 220, 255, 150));
                        d.draw_rectangle(ex as i32 - 1, ey as i32 - 1, 2, 2, Color::new(80, 220, 255, 220));
                    }
                }
            }
            if let Some(goal) = goal {
                d.draw_rectangle_lines((goal.x - cell * 0.5) as i32, (goal.y - cell * 0.5) as i32, size, size, Color::new(80, 220, 255, 220));
            }
        }
        if ov.pickups {
            // The square a hull's box has to touch, grown by the pad: what
            // `pickup_phase` tests (`Pickup::in_reach`), from the pickup's
            // side of it.
            let pad = tuning().pickup_collect_pad_px;
            for pickup in self.world.query::<&Pickup>().iter() {
                let half = pickup.size() * 0.5 + pad;
                let side = (half * 2.0).round() as i32;
                d.draw_rectangle_lines((pickup.position.x - half).round() as i32, (pickup.position.y - half).round() as i32, side, side, Color::GOLD);
            }
        }
        if ov.projectiles {
            let mut boxes: Vec<(Position, Vec2, f32)> = Vec::new();
            for s in self.world.query::<&Shell>().iter() {
                boxes.push((s.position, s.velocity, tuning().shell_hit_half_extent));
            }
            for p in self.world.query::<&Plasma>().iter() {
                boxes.push((p.position, p.velocity, tuning().plasma_hit_half_extent));
            }
            for b in self.world.query::<&Bullet>().iter() {
                boxes.push((b.position, b.velocity, tuning().minigun_bullet_hit_half_extent));
            }
            for (pos, vel, half) in boxes {
                let size = (half * 2.0).round().max(2.0) as i32;
                d.draw_rectangle_lines((pos.x - half).round() as i32, (pos.y - half).round() as i32, size, size, Color::MAGENTA);
                // A tenth of a second of travel.
                d.draw_line(
                    pos.x as i32,
                    pos.y as i32,
                    (pos.x + vel.x * 0.1) as i32,
                    (pos.y + vel.y * 0.1) as i32,
                    Color::new(255, 0, 255, 160),
                );
            }
        }
        if ov.ai || ov.engage {
            for (entity, tank, ai) in self.world.query::<(hecs::Entity, &Tank, &Ai)>().iter() {
                if tank.is_wreck() {
                    continue;
                }
                let (x, y) = (tank.position.x as i32, tank.position.y as i32);
                if ov.ai {
                    let s = ai.snapshot();
                    // (0, 0) is "never picked one": a tank straight into a
                    // fight has no patrol waypoint to show.
                    if s.waypoint_x != 0.0 || s.waypoint_y != 0.0 {
                        d.draw_line(x, y, s.waypoint_x as i32, s.waypoint_y as i32, Color::new(80, 200, 255, 140));
                        d.draw_circle_lines(s.waypoint_x as i32, s.waypoint_y as i32, 5.0, Color::new(80, 200, 255, 200));
                    }
                    if let Some(dir) = s.committed_dir.and_then(Dir::parse) {
                        let v = dir.vec();
                        let (ex, ey) = (tank.position.x + v.x * 40.0, tank.position.y + v.y * 40.0);
                        d.draw_line(x, y, ex as i32, ey as i32, Color::ORANGE);
                        d.draw_circle(ex as i32, ey as i32, 3.0, Color::ORANGE);
                    }
                    let label = format!(
                        "{} {}{}",
                        s.last_action.unwrap_or("-"),
                        s.committed_dir.unwrap_or("-"),
                        if s.stuck_timer > 0.0 { format!(" stuck {:.1}s", s.stuck_timer) } else { String::new() }
                    );
                    let ty = (tank.position.y + tank.hull_size() * 0.5 + 2.0) as i32;
                    d.draw_rectangle(x - 2, ty, label.len() as i32 * 7 + 4, 14, Color::new(0, 0, 0, 150));
                    d.draw_text(&label, x, ty + 1, 12, Color::new(80, 200, 255, 255));
                }
                if ov.engage && let Some(target) = self.last_engage.target(entity) {
                    let (tx, ty) = (target.x as i32, target.y as i32);
                    d.draw_line(x, y, tx, ty, Color::SKYBLUE);
                    d.draw_line(tx - 5, ty - 5, tx + 5, ty + 5, Color::SKYBLUE);
                    d.draw_line(tx - 5, ty + 5, tx + 5, ty - 5, Color::SKYBLUE);
                }
            }
        }
    }
}
