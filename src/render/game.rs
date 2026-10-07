//! `Game::render`: the whole frame with raylib, through a `view::Camera`.
//! Pass 1 paints the part of the world the camera shows into
//! `scene_target`, one texel per world pixel, through the three `Canvas`
//! stages of `game.rs` (`paint_floor`, `paint_tiles`, `paint_standing`)
//! with the live-round layers between them - fire, glows, the locate
//! label, projectiles, blasts, airborne debris, particles - drawn on the
//! handle directly, all in world pixels inside the camera's `in_target`;
//! pass 2 blits it into the bitmap at the camera's scale (shake, ripples)
//! and draws what lies in the world over it through the camera's
//! `on_field` (the ripples' quads, the debug overlays), and the bitmap
//! goes onto the window through `view::present_into` - or, for a followed
//! field map (`Camera::follows`), shifted by the camera's sub-block offset
//! (`view::present_world`). The chrome is then drawn on the window itself,
//! in UI points (`hud::UiFrame`) - the HUD's corner clusters, the banners,
//! the end screen, the dialogs, the lobby and the level select - so it
//! stands still while the world slides under it and keeps its size
//! whatever the map. Reads `Game`, never mutates it. `Textures` and
//! `Effects` bundle what a frame draws from.

use sola_raylib::prelude::*;

use crate::text::keys;
use crate::bullet::{Bullet, BulletState};
use crate::canvas::Sheet;
use crate::decal::draw_decal;
use crate::game::PaintOptions;
use crate::hud::{
    banner_size, corners, version_line, CornerShape, Corners, Fade, HudModel, PlayChrome, UiFrame, BANNER_SIZE,
    BANNER_SUB_SIZE, BUILD_COLOR, HUD_STATUS_COLOR, HUD_STATUS_TEXT_SIZE, HUD_VERSION_COLOR, HUD_VERSION_TEXT_SIZE,
    LEVEL_NUMBER_SIZE, LEVEL_TITLE_SIZE, RESULT_TITLE_SIZE, WAVE_BANNER_SIZE,
};
use crate::math::{Color, Rectangle};
use crate::obstacle::{draw_flying_drum, Obstacle};
use crate::render::level_select::draw_level_select;
use crate::render::lobby::draw_lobby;
use crate::plasma::{Plasma, PlasmaState};
use crate::render::blast::{draw_fire_glow, draw_flame_glow, draw_fuse_glow};
use crate::pyro;
use crate::render::pyro::Rl;
use crate::render::bullet::{draw_bullet, draw_bullet_light, draw_bullet_shadow};
use crate::render::canvas::{GpuCanvas, Sheets};
use crate::render::decal::draw_decal_shadow;
use crate::render::frog::FrogVariantTextures;
use crate::render::hud::{draw_banner, draw_corners, draw_leave_dialog, draw_players_dialog, draw_result, Line};
use crate::render::laser::{draw_laser_beam, draw_laser_bloom, draw_laser_flares};
use crate::render::plasma::{draw_plasma, draw_plasma_shadow};
use crate::render::portal::draw_portal_glow;
use crate::missile::Missile;
use crate::render::missile::{draw_missile, draw_missile_exhaust, draw_missile_shadow};
use crate::render::shell::{draw_shell, draw_shell_light, draw_shell_shadow};
use crate::render::shot_fx::{at_nozzle, ground_light};
use crate::render::shot_shaders::ShotShaders;
use crate::render::shockwave::RippleFx;
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
use crate::view::{culled, Camera, View};
use crate::{Layout, Position, SHOCK_MAX};
#[cfg(feature = "dev-tools")]
use crate::math::Vec2;
#[cfg(feature = "dev-tools")]
use crate::pickup::Pickup;
#[cfg(feature = "dev-tools")]
use crate::MAX_DAMAGE;

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
    /// static/crates_sheet.png - the pickups' supply crates (docs/CRATES_SPEC.md).
    pub crates: &'a Texture2D,
    /// static/pickup_glyphs.png - the pickups' symbols on their own.
    pub pickup_glyphs: &'a Texture2D,
    /// static/missile.png - a seeker missile in flight (missile.rs).
    pub missile: &'a Texture2D,
    /// The tall-grass sheet of the round's map theme
    /// (`map::Theme::grass_texture_path`, grass.rs); `ground` above is the
    /// theme's ground tileset the same way. `app.rs` picks both per frame.
    pub grass: &'a Texture2D,
    /// static/trees_sheet.png - the two tree species (docs/TREES_SPEC.md).
    pub trees: &'a Texture2D,
    /// static/target_sheet.png - the range board (docs/PROPS_SPEC.md).
    pub target: &'a Texture2D,
    /// static/towers_sheet.png - the defence towers (docs/TOWERS_SPEC.md).
    pub towers: &'a Texture2D,
    /// static/portal_sheet.png - the turning spiral (portal.rs).
    pub portal: &'a Texture2D,
    /// The round's baked images as `app.rs` uploaded them before the
    /// frame (`render::canvas::BlockTextures`, `Game::with_block_images`):
    /// the floor shade, the lava's banks and kept pictures, the cones -
    /// each with the stamp it was baked under.
    pub blocks: Vec<(u64, &'a Texture2D)>,
}

/// The game's `Sheet` lookup: what a `GpuCanvas` over these textures blits
/// from. Every sheet the field can name is here.
impl Sheets for Textures<'_> {
    fn blocks_texture(&self, stamp: u64) -> Option<&Texture2D> {
        self.blocks.iter().find(|(held, _)| *held == stamp).map(|(_, texture)| *texture)
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
            Sheet::Target => self.target,
            Sheet::Towers => self.towers,
            Sheet::Grass(_) => self.grass,
            Sheet::Tracks => self.tracks,
            Sheet::BarrelExplosion => self.barrel_explosion,
            Sheet::Portal => self.portal,
            Sheet::Crates => self.crates,
            Sheet::PickupGlyphs => self.pickup_glyphs,
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
    /// The touch scheme's feedback (`touch.rs`), drawn over everything in
    /// UI points (`hud::UiFrame`) - nothing until a touch is seen; `None`
    /// draws nothing at all. The `bool` is whether the stick lives on the
    /// right half.
    pub touch: Option<(&'a crate::touch::TouchScheme, bool)>,
    /// What the screen cannot see (`indicators.rs`): the arrows at the
    /// field's edge and the marks in the world, composed by `app.rs` while
    /// the camera shows less than the whole field; `None` draws nothing.
    pub indicators: Option<&'a crate::indicators::Picture>,
    /// The frog's speech bubble in a training round (`bubble.rs`), laid
    /// out by `app.rs` in the bitmap's pixels; `None` draws none.
    pub bubble: Option<&'a crate::bubble::Bubble>,
    /// The minimap under the right corner cluster (`minimap.rs`), drawn
    /// where the corners hold its slot (`PlayChrome::minimap`); `None`
    /// draws none.
    pub minimap: Option<crate::render::minimap::MinimapLayer<'a>>,
    /// The world an arena shows past its field in the window's margins
    /// (`render::margin`, `margin.rs`); `None` draws the margins as flat
    /// bars in the backdrop's colour.
    pub margins: Option<&'a mut crate::render::margin::MarginFx>,
    /// A couch's split screen (`follow::Split`): the second half, put on
    /// the window past the divider after the first; `None` draws one view.
    pub split: Option<SplitLayer<'a>>,
}

/// The second half of a couch's split screen, as a frame draws it
/// (`Effects::split`).
pub enum SplitLayer<'a> {
    /// The follow camera's split: the second half's own view of the world,
    /// drawn after the first through its own camera and targets.
    Follow {
        /// The second half's camera (`Camera::following` of
        /// `follow::Split`).
        camera: Camera,
        /// Its own pass-1 target and composite, the first half's sizes.
        scene: &'a mut RenderTexture2D,
        composite: &'a mut RenderTexture2D,
        /// The divider, in the followed bitmap's pixels: a point on it and
        /// the unit normal into the second half
        /// (`follow::Split::at`/`normal`).
        at: crate::math::Vec2,
        normal: crate::math::Vec2,
        /// How far apart the halves' views stand, world px: the divider
        /// fades in with it.
        apart: f32,
        /// The indicators' blocks that lie in each half's world, first
        /// half first (`indicators::Picture::world`).
        marks: [&'a [crate::indicators::Fill]; 2],
    },
    /// The establishing shot zooming into a split (`establish::ZoomSplit`):
    /// the whole field drawn once and put on the window twice, the second
    /// time through `view` past the divider.
    Zoom {
        /// The second half's zoom, as a `View` of the whole field's bitmap.
        view: View,
        /// The divider, window units: a point on it and the unit normal
        /// into the second half.
        at: crate::math::Vec2,
        normal: crate::math::Vec2,
        /// How far the zoom has drawn the divider in, and how far apart the
        /// follow views it lands on stand (world px).
        alpha: f32,
        apart: f32,
    },
}

/// The divider between a split screen's halves: the corner clusters'
/// near-black.
const DIVIDER: Color = Color::new(0x14, 0x14, 0x18, 255);

impl Game {
    /// The light shots throw on the ground (inside the first additive
    /// block, under every sprite): a soft pool in the shot's colour under
    /// each shell, bullet, plasma orb and missile, along the flamethrower's
    /// stream, where a laser burns, and a flash of it at every muzzle and
    /// impact - so the floor itself says where the fire is. `k` scales the
    /// lot: 1 by day, less where the weather's light map lights the ground
    /// around the shots instead (`WeatherFrame::day_light_pools`). A pool
    /// that cannot reach the world worth drawing (`Camera::cull`) is left
    /// out.
    fn draw_ground_light(&self, d: &mut impl RaylibDraw, k: f32, cull: Option<Rectangle>) {
        if k <= 0.0 {
            return;
        }
        let ground_light = |d: &mut _, at: Position, radius: f32, color: Color, strength: f32| {
            let reach = cull.map(|r| Rectangle::new(r.x - radius, r.y - radius, r.width + 2.0 * radius, r.height + 2.0 * radius));
            if !culled(reach, at) {
                ground_light(d, at, radius, color, strength * k);
            }
        };
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
        // A dead missile (an EMP) has no motor to light the ground.
        for missile in self.world.query::<&Missile>().iter().filter(|m| !m.is_dead()) {
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
        // A crater lights the ground by its heat, and a bomb under it as it
        // comes down (docs/volcano.md); the lava's own light on its banks
        // is `draw_lava_ground_light`'s.
        if !self.volcanoes.is_empty() {
            let t = tuning();
            for v in &self.volcanoes {
                let heat = v.phase(self.time, &t).heat;
                ground_light(d, v.centre(), 80.0 + 60.0 * heat, warm, 0.25 * heat);
            }
            for bomb in &self.lava_bombs {
                ground_light(d, bomb.ground_pos(&t), 20.0, warm, 0.25 * bomb.flight(&t));
            }
        }
    }

    /// The light the lava throws on its banks by day, `k` of it (the
    /// daylight's share, as `draw_ground_light`'s): every other cell a
    /// stepped glow, brighter in the surge. Where the frame kept the bake
    /// (`LavaPictures::ground_light`) it is its tiles, their strength in
    /// the tint's alpha, added to the colour alone - an additive blend
    /// would add that alpha to the target's too and lift the banks' shade
    /// where the scene meets the window, which the glows' faint blocks
    /// never did. Otherwise the glows block by block, additive.
    fn draw_lava_ground_light<D: RaylibDraw>(&self, d: &mut D, textures: &Textures, k: f32, cull: Option<Rectangle>) {
        if self.lava.is_empty() || k <= 0.0 {
            return;
        }
        let strength = crate::lava::GROUND_LIGHT_STRENGTH * (1.0 + self.lava_look().surge);
        let radius = crate::lava::GROUND_LIGHT_RADIUS;
        let pictures = self.lava.pictures();
        let baked = if pictures.fresh(self.time) { pictures.ground_light() } else { None }
            .and_then(|atlas| textures.blocks_texture(atlas.image.stamp).map(|texture| (atlas, texture)));
        let reach = cull.map(|r| {
            let m = radius + 16.0;
            Rectangle::new(r.x - m, r.y - m, r.width + 2.0 * m, r.height + 2.0 * m)
        });
        match baked {
            Some((atlas, texture)) => {
                use sola_raylib::ffi::{RL_FUNC_ADD, RL_ONE, RL_SRC_ALPHA, RL_ZERO};
                let share = strength * k * tuning().shot_glow_strength / crate::lava::GROUND_LIGHT_BAKED;
                let tint = Color::new(255, 255, 255, (255.0 * share.clamp(0.0, 1.0)) as u8);
                if tint.a == 0 {
                    return;
                }
                d.set_blend_factors_separate(RL_SRC_ALPHA as i32, RL_ONE as i32, RL_ZERO as i32, RL_ONE as i32, RL_FUNC_ADD as i32, RL_FUNC_ADD as i32);
                d.draw_blend_mode(BlendMode::BLEND_CUSTOM_SEPARATE, |mut bd| {
                    let b = atlas.image.block as f32;
                    for ((x, y), (sx, sy, w, h)) in atlas.tiles() {
                        if culled(reach, Position::new(x as f32 + 16.0, y as f32 + 16.0)) {
                            continue;
                        }
                        let src = Rectangle::new(sx as f32, sy as f32, w as f32, h as f32);
                        let dest = Rectangle::new(x as f32, y as f32, w as f32 * b, h as f32 * b);
                        bd.draw_texture_pro(texture, src, dest, Vector2::new(0.0, 0.0), 0.0, tint);
                    }
                });
            }
            None => d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
                for (col, row) in self.lava.cells().filter(|(c, r)| (c + r) % 2 == 0) {
                    let at = crate::map::cell_to_world(col, row);
                    if !culled(reach, at) {
                        ground_light(&mut bd, at, radius, crate::lava::GROUND_LIGHT_COLOR, strength * k);
                    }
                }
            }),
        }
    }
}

impl Game {
    /// Draw the whole scene for this frame: the part of the world `camera`
    /// shows into `scene_target` (`Camera::target_size`, a texel per world
    /// pixel), then that into `composite` - the bitmap, the field alone
    /// (`layout.field`) - and the bitmap onto the window through `view`
    /// (`view::present_into`), scaled and centred so the whole field is on
    /// screen whatever the window is - the window round it showing the
    /// world past the field (`Effects::margins`, `render::margin`) where the
    /// shapes differ. The world is drawn in world pixels through raylib
    /// cameras built from `camera` (`render::view`), so the simulation's
    /// positions are used as they are.
    ///
    /// A followed field map (`Camera::follows`) is presented shifted, so
    /// its view can move by less than a block: `composite` holds the scene
    /// target's every texel and goes onto the window's field area shifted
    /// by the camera's offset (`present_world`).
    ///
    /// The chrome is drawn on the window itself, over the presented world:
    /// the corner clusters (`hud::corners`) in UI points through a camera
    /// at `ui`'s scale, each at its `fade`, so they stand still and keep
    /// their size in points while the world slides and scales under them;
    /// the banners, the dims and the dialogs through cameras that put the
    /// bitmap's pixels where `view` puts them.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        scene_target: &mut RenderTexture2D,
        composite: &mut RenderTexture2D,
        view: &View,
        camera: &Camera,
        backdrop: Color,
        effects: &mut Effects,
        textures: &Textures,
        layout: &Layout,
        chrome: &PlayChrome,
        ui: &UiFrame,
        fade: Fade,
    ) {
        // Must be read off the handle out here: the draw closures below
        // borrow it, and the dev label needs the number.
        #[cfg(feature = "dev-tools")]
        let frame_ms = rl.get_frame_time() * 1000.0;
        let hud = HudModel::gather(self, chrome.seat);
        let t = crate::text::text();
        // The seats whose shells ride under their rings: this window's
        // own - the room's seat online, the couch's one or two locally -
        // and only where the HUD is drawn at all.
        let ammo_seats: Vec<u8> = match (chrome.hud, chrome.seat) {
            (false, _) => Vec::new(),
            (true, Some(seat)) => vec![seat],
            (true, None) => (0..self.players.count().min(2) as u8).collect(),
        };

        // What the end screen, the banners and PAUSED say; `draw_chrome`
        // sets each in the chrome's area.
        let banner = match self.outcome {
            Outcome::Playing => None,
            // A training round's win is the course done (docs/training-stage.md).
            Outcome::Won if self.map.training.is_some() => Some((t.get(keys::ROUND_TRAINED), Color::DARKGREEN)),
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
            // A training round opens on its own banner, whatever its mission.
            let words = if self.map.training.is_some() { keys::MISSION_TRAINING_BANNER } else { crate::text::mission_banner(self.mission) };
            (alpha > 0.0 && self.outcome == Outcome::Playing).then(|| (t.get(words), alpha))
        };
        // Wave rounds: the `WAVE N` banner during the breather before a
        // wave - smaller than the mission banner, no dim overlay, and never
        // over the end-of-round banner. The counter itself is in the right
        // cluster.
        let wave_banner = self.wave_banner().filter(|_| self.outcome == Outcome::Playing).map(|banner| {
            if banner.is_final { t.get(keys::WAVE_FINAL) } else { t.fmt(keys::WAVE_BANNER, &[("n", banner.next.into())]) }
        });
        // The end's beats (`EndBeats`): the finale with nothing over the
        // world, then the end screen easing in with the corners.
        let beats = self.end_beats();
        let since_end = self.since_end();
        let verdict = since_end.map_or(1.0, |s| beats.verdict(s));
        let hud_end = since_end.map_or(1.0, |s| {
            if s < beats.finale {
                let k = (s / HUD_END_FADE_SECONDS).clamp(0.0, 1.0);
                (1.0 - k) * (1.0 - k)
            } else {
                verdict
            }
        });
        let banner = banner.map(|(text, color)| {
            let seconds = self.restart_timer.min(beats.countdown).ceil().max(0.0) as i32;
            let sub = t.fmt(chrome.countdown_label.unwrap_or(keys::ROUND_RESTARTING), &[("seconds", seconds.into())]);
            (text, color, sub)
        });
        let paused = t.get(keys::PAUSED);

        // The world, through `camera` into `composite`; and on a couch's
        // split screen the second half's, through its own camera into its
        // own targets, each with the indicators' marks that lie in it, so
        // the divider cuts them as it cuts the world.
        let mut split = effects.split.take();
        let marks: [&[crate::indicators::Fill]; 2] = match &split {
            Some(SplitLayer::Follow { marks, .. }) => *marks,
            _ => [&[], &[]],
        };
        let weather = self.draw_world(rl, thread, scene_target, composite, camera, layout, effects, textures, &ammo_seats, backdrop, marks[0]);
        if let Some(SplitLayer::Follow { camera, scene, composite, .. }) = split.as_mut() {
            let camera = *camera;
            self.draw_world(rl, thread, &mut **scene, &mut **composite, &camera, layout, effects, textures, &ammo_seats, backdrop, marks[1]);
        }

        let text = ChromeText {
            t: &t,
            banner,
            verdict,
            hud_end,
            intro,
            wave_banner,
            paused,
            #[cfg(feature = "dev-tools")]
            frame_ms,
        };
        let touch = effects.touch;
        let fx_live = effects.fx.live();
        let indicators = effects.indicators;
        let bubble = effects.bubble;
        let minimap = effects.minimap.take();
        let corners = CornerShape::of(chrome, self.players.count()).map(|shape| corners(ui, &shape));
        let frame = ChromeFrame { ui, fade, corners: corners.as_ref(), units: crate::render::view::window_camera_units(rl) };

        // An arena's window margins show the world past its field
        // (`margin.rs`) where the window's shape is not the field's.
        let margin_frame = match effects.margins {
            Some(_) if camera.is_whole() => crate::margin::MarginFrame::of(view, layout),
            _ => None,
        };
        let margins = match (margin_frame, effects.margins.as_deref_mut()) {
            (Some(frame), Some(fx)) => {
                let sky = match (effects.weather.as_deref_mut(), weather.as_ref()) {
                    (Some(fx), Some(frame)) => Some((fx, frame)),
                    _ => None,
                };
                let at = layout.field_origin();
                let in_bitmap = |r: Rectangle| Rectangle::new(r.x + at.x, r.y + at.y, r.width, r.height);
                fx.draw(rl, thread, self, &frame, textures, backdrop, sky).map(|target| crate::render::view::Margins {
                    target,
                    rect: in_bitmap(frame.rect),
                    parts: frame.parts(0.0).into_iter().map(in_bitmap).collect(),
                })
            }
            _ => None,
        };
        let mut d = rl.begin_drawing(thread);
        if camera.follows() {
            crate::render::view::present_world(&mut d, composite, camera, view, layout, backdrop);
        } else {
            crate::render::view::present_into(&mut d, composite, view, backdrop, margins.as_ref());
        }
        // A split screen's second half over the first, past the divider,
        // and the divider over both.
        match &split {
            Some(SplitLayer::Follow { camera: second, composite: world, at, normal, apart, .. }) if camera.follows() => {
                crate::render::view::present_half(&mut d, world, second, view, layout, *at, *normal);
                crate::render::view::draw_divider(&mut d, view, layout, *at, *normal, *apart, DIVIDER);
            }
            Some(SplitLayer::Zoom { view: second, at, normal, alpha, apart }) if !camera.follows() => {
                crate::render::view::present_zoom_half(&mut d, composite, second, *at, *normal, backdrop);
                crate::render::view::draw_zoom_divider(&mut d, view.window, *at, *normal, second.scale, *alpha, *apart, DIVIDER);
            }
            _ => {}
        }
        let base = crate::render::view::onto_window(
            Camera2D { offset: view.offset.into(), target: Vector2::new(0.0, 0.0), rotation: 0.0, zoom: view.scale },
            frame.units,
        );
        let chrome_stage = crate::frame_stages::stage("chrome");
        self.draw_chrome(&mut d, &text, &hud, chrome, &frame, layout, camera, indicators, bubble, minimap.as_ref(), textures, touch, fx_live, base);
        drop(chrome_stage);
        let _swap = crate::frame_stages::stage("swap");
        drop(d);
    }
}

impl Game {
    /// One view of the world, through `camera`: pass 1 into `scene_target`
    /// (`Camera::target_size`, a texel per world pixel) - under the sky in
    /// the weather's stages when there is one - then pass 2 into
    /// `composite`: the scene through the kill ripple, shaken, the ripples'
    /// quads, the kill flash, the debug overlays and `marks`, the
    /// indicators' blocks that lie in the world (`indicators::Picture`'s,
    /// drawn here only for a split screen's halves). The frame's sky, for
    /// an arena's margins.
    #[allow(clippy::too_many_arguments)]
    fn draw_world(
        &self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        scene_target: &mut RenderTexture2D,
        composite: &mut RenderTexture2D,
        camera: &Camera,
        layout: &Layout,
        effects: &mut Effects,
        textures: &Textures,
        ammo_seats: &[u8],
        backdrop: Color,
        marks: &[crate::indicators::Fill],
    ) -> Option<crate::render::weather::WeatherFrame> {
        // The field area of the bitmap, which what covers the whole world
        // (a kill's flash) fills: whole pixels, rounded up so a followed
        // view of a fractional size is covered to its edge.
        let screen_width = layout.field.w.ceil() as i32;
        let screen_height = layout.field.h.ceil() as i32;
        // The scene target: the camera's view at a texel per world pixel,
        // the world rectangle worth drawing into it, and the cameras pass 1
        // and pass 2 draw the world through.
        let (target_width, target_height) = camera.target_size();
        let cull = camera.cull();
        let in_target = camera.in_target();

        // Pass 1: draw the world (tracks, tanks, shells) into an offscreen
        // render texture, so a shockwave can distort the finished frame as a
        // whole-screen shader pass in pass 2. Under a sky (docs/weather.md,
        // render/weather.rs) it runs in stages that ping-pong between
        // `scene_target` and the weather's own target and always end in
        // `scene_target`: the field under the light, multiplied by the
        // light map, then what shines by itself, then the air over it all.
        // A pass that covers a whole target - the weather's - is drawn in
        // the target's own pixels; the world is drawn through `in_target`.
        let lights_stage = crate::frame_stages::stage("lights");
        let weather = match effects.weather.as_deref_mut() {
            Some(fx) => fx.begin(rl, thread, self, effects.fx, textures, camera),
            None => None,
        };
        drop(lights_stage);
        let world_stage = crate::frame_stages::stage("world");
        match weather.as_ref() {
            None => {
                rl.draw_texture_mode(thread, scene_target, |mut d| {
                    d.clear_background(Color::WHITE);
                    d.draw_mode2D(in_target, |mut d, _| {
                        self.paint_field_lit(&mut d, textures, effects.fx, false, cull);
                        self.paint_field_glowing(&mut d, textures, effects.shots.as_deref_mut(), effects.fx, 1.0, camera, ammo_seats);
                    });
                });
            }
            Some(frame) if frame.plain => {
                // The sky without its shaders (`weather::plain`), every
                // stage straight into `scene_target`: the field (on snow
                // lying over the bare ground, under the marks), the light
                // map multiplied in by a blend mode, what shines by
                // itself, then the air as plain blocks.
                let plan = frame.plan;
                let day_pools = frame.day_light_pools();
                let light = effects.weather.as_deref().and_then(|fx| fx.light_map()).expect("`begin` readied the targets");
                let mut blocks = Vec::new();
                rl.draw_texture_mode(thread, scene_target, |mut d| {
                    d.clear_background(Color::WHITE);
                    d.draw_mode2D(in_target, |mut d, _| {
                        frame.plain_snow_cover(&mut blocks);
                        let snowed = !blocks.is_empty();
                        if snowed {
                            self.paint_ground(&mut GpuCanvas::culled(&mut d, textures, cull));
                            crate::render::weather::draw_blocks(&mut d, &blocks);
                        }
                        self.paint_field_lit(&mut d, textures, effects.fx, snowed, cull);
                        if !plan.lit {
                            self.paint_field_glowing(&mut d, textures, effects.shots.as_deref_mut(), effects.fx, day_pools, camera, ammo_seats);
                        }
                    });
                    if plan.lit {
                        crate::render::weather::multiply_light(&mut d, light, &frame);
                        d.draw_mode2D(in_target, |mut d, _| {
                            self.paint_field_glowing(&mut d, textures, effects.shots.as_deref_mut(), effects.fx, day_pools, camera, ammo_seats);
                        });
                    }
                    if plan.sky {
                        blocks.clear();
                        frame.plain_air(&mut blocks);
                        d.draw_mode2D(in_target, |mut d, _| crate::render::weather::draw_blocks(&mut d, &blocks));
                        crate::render::weather::draw_flash(&mut d, &frame);
                    }
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
                        // The ground pass draws the bare ground with the
                        // sky's mark on it in place of the ground tileset.
                        if plan.ground {
                            passes.draw_ground(&mut d, &frame);
                        }
                        d.draw_mode2D(in_target, |mut d, _| {
                            self.paint_field_lit(&mut d, textures, effects.fx, plan.ground, cull);
                            if !plan.lit {
                                self.paint_field_glowing(&mut d, textures, effects.shots.as_deref_mut(), effects.fx, day_pools, camera, ammo_seats);
                            }
                        });
                    });
                }
                if plan.lit {
                    let (under, to): (&RenderTexture2D, &mut RenderTexture2D) =
                        if first_in_scene { (&*scene_target, &mut *lit_target) } else { (&*lit_target, &mut *scene_target) };
                    rl.draw_texture_mode(thread, to, |mut d| {
                        passes.draw_lit(&mut d, under, &frame);
                        d.draw_mode2D(in_target, |mut d, _| {
                            self.paint_field_glowing(&mut d, textures, effects.shots.as_deref_mut(), effects.fx, day_pools, camera, ammo_seats);
                        });
                    });
                }
                if plan.sky {
                    // Whichever stage came before left the field in the
                    // weather's target.
                    rl.draw_texture_mode(thread, scene_target, |mut d| passes.draw_sky(&mut d, lit_target, &frame));
                }
            }
        }

        drop(world_stage);
        let post_stage = crate::frame_stages::stage("post");
        // The ripples are measured in the standard field's units from the
        // corner of the camera's part of the world the scene target holds
        // (`Camera::ripple_view`).
        for ripple in [&mut *effects.shock, &mut *effects.muzzle, &mut *effects.impact] {
            ripple.set_view(camera);
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
            let mut starts = [0.0f32; SHOCK_MAX];
            let mut signs = [1.0f32; SHOCK_MAX];
            for (i, shock) in self.shocks.iter().take(SHOCK_MAX).enumerate() {
                centers[i] = effects.shock.uv_of(shock.center);
                times[i] = shock.time;
                gains[i] = shock.strength;
                (starts[i], signs[i]) = effects.shock.start_and_sign(shock);
            }
            effects.shock.shader.set_shader_value_v(effects.shock.centers_loc, &centers);
            effects.shock.shader.set_shader_value_v(effects.shock.times_loc, &times);
            effects.shock.shader.set_shader_value_v(effects.shock.gains_loc, &gains);
            if effects.shock.starts_loc >= 0 && effects.shock.signs_loc >= 0 {
                effects.shock.shader.set_shader_value_v(effects.shock.starts_loc, &starts);
                effects.shock.shader.set_shader_value_v(effects.shock.signs_loc, &signs);
            }
        }

        // The render texture is stored upside-down relative to the screen; a
        // negative source height flips it back on the way out.
        let source = Rectangle {
            x: 0.0,
            y: 0.0,
            width: target_width as f32,
            height: -(target_height as f32),
        };

        // Camera shake (`shockwave::camera_shake`): a short decaying wobble
        // on the same kill-shockwave trigger as `self.shock` itself, applied
        // purely as an offset on this blit's destination, in world pixels at
        // the camera's scale - shifting where the already-composited scene
        // lands is the cheapest way to get the effect. A view of part of
        // the field is shaken only by what happens near it; the whole field,
        // an arena's, by everything. Muzzle/impact flash quads
        // and the HUD deliberately aren't shifted: they're either their own
        // small on-screen quad or meant to stay put. The field origin is
        // added on top: the scene lands on the field's place in the bitmap.
        // Reduced motion (`motion.rs`) has no shake, and no ripple bending
        // the whole screen either (`WorldPass::bend`).
        let still = crate::motion::reduced();
        let shake = if still {
            crate::math::Vec2::new(0.0, 0.0)
        } else {
            crate::shockwave::camera_shake(&self.shocks, (!camera.shows_whole_field()).then(|| camera.rect()), &tuning())
        };
        let blit_offset = Vector2::new(shake.x, shake.y);

        // Where pass 2 puts the world, and at what scale: the field area of
        // the bitmap at the camera's - or, for a followed view, the whole
        // of `composite` at one pixel per texel, which then holds the
        // world alone for `present_world` to scale onto the window.
        let (world_origin, world_scale) =
            if camera.follows() { (crate::math::Vec2::new(0.0, 0.0), 1.0) } else { (layout.field_origin(), camera.scale) };
        let shaken_origin = crate::math::Vec2::new(world_origin.x + blit_offset.x * world_scale, world_origin.y + blit_offset.y * world_scale);
        let in_world = |offset: crate::math::Vec2| Camera2D { offset: offset.into(), target: camera.origin.into(), rotation: 0.0, zoom: world_scale };
        let world = WorldPass {
            scene: &*scene_target,
            source,
            blit: Rectangle::new(shaken_origin.x, shaken_origin.y, target_width as f32 * world_scale, target_height as f32 * world_scale),
            on_field: in_world(world_origin),
            shaken: in_world(shaken_origin),
            area_camera: Camera2D { offset: world_origin.into(), target: Vector2::new(0.0, 0.0), rotation: 0.0, zoom: 1.0 },
            area: if camera.follows() { (target_width, target_height) } else { (screen_width, screen_height) },
            camera: *camera,
            cull,
            backdrop,
            bend: !still,
            marks,
        };
        rl.draw_texture_mode(thread, composite, |mut d| {
            d.clear_background(Color::BLACK);
            self.draw_world_layer(&mut d, &world, effects);
        });
        drop(post_stage);
        weather
    }
}

/// What pass 2 draws the world with (`Game::draw_world_layer`).
struct WorldPass<'a> {
    /// Pass 1's picture.
    scene: &'a RenderTexture2D,
    /// All of it, flipped: a render texture reads back bottom-up.
    source: Rectangle,
    /// Where it lands, with the shake.
    blit: Rectangle,
    /// What lies in the world, drawn over it; `shaken` moves with the
    /// shake as the scene does.
    on_field: Camera2D,
    shaken: Camera2D,
    /// The area the world fills, in its own pixels, for what covers all
    /// of it: the bitmap's field area, or a followed view's whole target.
    area_camera: Camera2D,
    area: (i32, i32),
    camera: Camera,
    cull: Option<Rectangle>,
    backdrop: Color,
    /// Whether the kill ripples bend the whole scene: not under reduced
    /// motion (`motion.rs`).
    bend: bool,
    /// The indicators' blocks that lie in this view's world: a split
    /// screen half's (`Effects::split`); empty otherwise, the indicators
    /// being drawn with the chrome.
    marks: &'a [crate::indicators::Fill],
}

/// Seconds the corner clusters take to step aside once the round is
/// decided, so the finale plays with nothing over the world.
const HUD_END_FADE_SECONDS: f32 = 0.4;

/// What `Game::draw_chrome` writes, gathered by `Game::render` in the
/// language on screen.
struct ChromeText<'a> {
    t: &'a crate::text::Catalogue,
    /// The end screen: its outcome, the outcome's colour and the
    /// countdown line.
    banner: Option<(String, Color, String)>,
    /// How far the end screen has eased in after the finale, 0 to 1
    /// (`EndBeats::verdict`).
    verdict: f32,
    /// The corner clusters' opacity at the round's end: gone for the
    /// finale, back with the verdict; 1 while the round plays.
    hud_end: f32,
    /// The mission banner and its opacity.
    intro: Option<(String, f32)>,
    /// The `WAVE N` banner.
    wave_banner: Option<String>,
    paused: String,
    #[cfg(feature = "dev-tools")]
    frame_ms: f32,
}

/// The window the chrome stands on: its UI frame, the corner clusters laid
/// out in it (`None` where the frame draws none), how far each has faded,
/// and the framebuffer pixels a window unit is under a camera drawn onto
/// the window (`render::view::window_camera_units`).
struct ChromeFrame<'a> {
    ui: &'a UiFrame,
    fade: Fade,
    corners: Option<&'a Corners>,
    units: f32,
}

impl Game {
    /// Pass 2's world: the scene target (through the shockwave while one
    /// plays, unless motion is reduced), the muzzle and impact ripples'
    /// quads over it, the kill flash
    /// over the whole of it, a followed view's world past the field's edge
    /// in the backdrop's colour, and the debug overlays.
    fn draw_world_layer<D: RaylibDraw>(&self, d: &mut D, w: &WorldPass, effects: &mut Effects) {
        let camera = &w.camera;
        let cull = w.cull;
        let (field_w, field_h) = camera.field;
        // A point of the world as the scene target's texel, y up the way a
        // render texture reads.
        let target_height = camera.target_size().1;
        let texel = |at: Position| Vector2::new(at.x - camera.origin.x, target_height as f32 - (at.y - camera.origin.y));

        if !self.shocks.is_empty() && w.bend {
            d.draw_shader_mode(&mut effects.shock.shader, |mut sd| {
                sd.draw_texture_pro(w.scene, w.source, w.blit, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
            });
        } else {
            d.draw_texture_pro(w.scene, w.source, w.blit, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
        }

        // The ripples' quads lie in the world: drawn through `on_field`.
        d.draw_mode2D(w.on_field, |mut d, _| {
            // Layer each muzzle flash's tiny heat-haze ripple on top, one small
            // quad at a time: source and dest are the same on-screen patch (just
            // re-sampling that bit of the already-composited scene through the
            // ripple shader), so this reads as a localized wobble rather than
            // redistorting the whole frame.
            for flash in self.muzzle_flashes.iter().filter(|f| !culled(cull, f.center)) {
                let uv = effects.muzzle.uv_of(flash.center);
                effects
                    .muzzle
                    .shader
                    .set_shader_value(effects.muzzle.center_loc, uv);
                effects
                    .muzzle
                    .shader
                    .set_shader_value(effects.muzzle.time_loc, flash.time);

                let r = tuning().muzzle_flash_quad_radius;
                let at = texel(flash.center);
                let flash_source = Rectangle {
                    x: at.x - r,
                    y: at.y - r,
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
                        w.scene,
                        flash_source,
                        flash_dest,
                        origin,
                        0.0,
                        Color::WHITE,
                    );
                });
            }

            // Same treatment for every in-flight shell-impact flash.
            for flash in self.impact_flashes.iter().filter(|f| !culled(cull, f.center)) {
                let uv = effects.impact.uv_of(flash.center);
                effects
                    .impact
                    .shader
                    .set_shader_value(effects.impact.center_loc, uv);
                effects
                    .impact
                    .shader
                    .set_shader_value(effects.impact.time_loc, flash.time);

                let r = tuning().impact_flash_quad_radius;
                let at = texel(flash.center);
                let flash_source = Rectangle {
                    x: at.x - r,
                    y: at.y - r,
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
                        w.scene,
                        flash_source,
                        flash_dest,
                        origin,
                        0.0,
                        Color::WHITE,
                    );
                });
            }
        });

        // A kill or a barrel blast opens with a brief whole-screen
        // flash (`Game::screen_flash`, started and spaced out by
        // `Game::flash_screen`), over the whole field area. After the
        // ripple quads, which re-blit patches of the un-flashed scene and
        // would otherwise punch darker squares through it.
        d.draw_mode2D(w.area_camera, |mut d, _| {
            if let Some(age) = self.screen_flash {
                // A rod's flash is stronger and longer than a drum's
                // (`Game::flash_screen_with`).
                let strength = self.screen_flash_strength.max(1.0);
                let seconds = tuning().blast_screen_flash_seconds * strength;
                if seconds > 0.0 && age < seconds {
                    let peak = (tuning().blast_screen_flash_alpha * strength).min(0.9) * tuning().screen_fx_intensity;
                    let a = (255.0 * peak.clamp(0.0, 1.0) * (1.0 - age / seconds)) as u8;
                    d.draw_rectangle(0, 0, w.area.0, w.area.1, Color::new(255, 240, 200, a));
                }
            }
        });

        // A followed view of a map shorter than the view on an axis shows
        // past the field's edge, where nothing is drawn: the backdrop's
        // colour there, the letterbox's, moving with the shake like the
        // field it borders.
        if camera.follows() {
            let r = camera.target_rect();
            let bands = [
                Rectangle::new(r.x, r.y, -r.x, r.height),
                Rectangle::new(field_w, r.y, r.x + r.width - field_w, r.height),
                Rectangle::new(r.x, r.y, r.width, -r.y),
                Rectangle::new(r.x, field_h, r.width, r.y + r.height - field_h),
            ];
            d.draw_mode2D(w.shaken, |mut d, _| {
                for band in bands.iter().filter(|b| b.width > 0.0 && b.height > 0.0) {
                    d.draw_rectangle_rec(*band, w.backdrop);
                }
            });
        }

        // Debug overlays (dev builds only): the two tank layers -
        // hitbox/collider outlines and the stats card, each on its own
        // flag - for every tank, then the dev server's other layers.
        // Drawn here, post-composite, rather than into scene_target, so
        // they're never warped by an in-flight shockwave and always
        // render crisp; they lie in the world, so `on_field` puts them
        // over what they describe.
        #[cfg(feature = "dev-tools")]
        d.draw_mode2D(w.on_field, |mut d, _| {
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
        });

        // A split screen half's off-screen marks in its own world, so they
        // end at the divider with it.
        if !w.marks.is_empty() {
            d.draw_mode2D(w.on_field, |mut d, _| {
                for f in w.marks {
                    d.draw_rectangle(f.x, f.y, f.w, f.h, f.color);
                }
            });
        }
    }

    /// What stands over the world, on the window: the off-screen
    /// indicators, through `base` (the camera that puts each bitmap pixel
    /// where `View` puts it); then everything else in UI points at the UI
    /// scale (`hud::UiFrame`), centred in the chrome's area with its dims
    /// over the whole window - the end screen, the mission and `WAVE N`
    /// banners and PAUSED under the corner clusters (the lines under the
    /// left one: an online round's status, the build stamp, the dev label)
    /// at the frame's fade, and the dialogs, the lobby and the level select
    /// over them, since nothing behind a question can be pressed; and the
    /// touch stick over everything, in UI points too.
    #[allow(clippy::too_many_arguments)]
    fn draw_chrome<D: RaylibDraw>(
        &self,
        d: &mut D,
        c: &ChromeText,
        hud: &HudModel,
        chrome: &PlayChrome,
        frame: &ChromeFrame,
        layout: &Layout,
        camera: &Camera,
        indicators: Option<&crate::indicators::Picture>,
        bubble: Option<&crate::bubble::Bubble>,
        minimap: Option<&crate::render::minimap::MinimapLayer>,
        textures: &Textures,
        touch: Option<(&crate::touch::TouchScheme, bool)>,
        fx_live: usize,
        base: Camera2D,
    ) {
        #[cfg(not(feature = "dev-tools"))]
        let _ = fx_live;
        let t = c.t;

        // What the screen cannot see (indicators.rs): its marks in the
        // world, where the frame shows the world, and its arrows and labels
        // in the bitmap's pixels - over the world layer, so no sky darkens
        // them, and under the HUD, the banners and the dialogs.
        if let Some(picture) = indicators {
            let on_field = camera.on_field(layout.field_origin());
            let world = Camera2D {
                offset: Vector2::new(base.offset.x + on_field.offset.x * base.zoom, base.offset.y + on_field.offset.y * base.zoom),
                target: on_field.target,
                rotation: 0.0,
                zoom: on_field.zoom * base.zoom,
            };
            crate::render::indicators::draw_indicators(d, picture, world, Some(base));
        }
        // The frog's bubble (bubble.rs), over the world and the arrows,
        // under the HUD.
        if let Some(bubble) = bubble {
            crate::render::bubble::draw_bubble(d, bubble, base);
        }

        // The lines under the left cluster: first what letting go of a
        // rod's reticle does, while one is up, or what the next press does
        // to an orb in flight - in its weapon's accent.
        let mut lines = Vec::new();
        if let Some(key) = chrome.prompt {
            let color = if key == crate::text::keys::HUD_WELL_ANCHOR { crate::hud::HUD_WELL_COLOR } else { crate::hud::HUD_ROD_COLOR };
            lines.push(Line { text: crate::text::text().get(key), size: HUD_STATUS_TEXT_SIZE, color });
        }
        // An online round says where it stands: the room, the seat and how
        // much of the snapshot stream is in hand.
        if let Some(status) = &chrome.status {
            lines.push(Line { text: status.clone(), size: HUD_STATUS_TEXT_SIZE, color: HUD_STATUS_COLOR });
        }
        lines.push(Line { text: version_line(), size: HUD_VERSION_TEXT_SIZE, color: HUD_VERSION_COLOR });
        // Which overlay preset is live, so the I key's cycling is visible
        // without counting layers; frame time and the live particle count
        // ride the same line - the FX budget has to hold on the wasm build,
        // and without a number on screen that is an assertion nobody can
        // check while playing.
        #[cfg(feature = "dev-tools")]
        if let Some(preset) = self.debug_overlays.preset_name() {
            lines.push(Line {
                text: format!("DEV overlays: {preset} (I cycles)  |  {:.1} ms  {} fx", c.frame_ms, fx_live),
                size: 14,
                color: Color::new(80, 200, 255, 255),
            });
        }
        // Where the frame's time goes (`frame_stages.rs`), where the
        // tuning panel asks for it.
        #[cfg(feature = "dev-tools")]
        if tuning().ui_frame_stages {
            lines.push(Line { text: crate::frame_stages::line(), size: 14, color: Color::new(80, 200, 255, 255) });
        }

        // Everything from here to the stick is in UI points: the window at
        // the UI scale, never the world's. The dims cover the whole window
        // and every banner centres on the chrome's area.
        let ui = frame.ui;
        let ui_camera = crate::render::view::onto_window(
            Camera2D { offset: Vector2::new(0.0, 0.0), target: Vector2::new(0.0, 0.0), rotation: 0.0, zoom: ui.scale },
            frame.units,
        );
        let area = ui.area;
        let (screen_w, screen_h) = (ui.screen.w.ceil() as i32, ui.screen.h.ceil() as i32);
        let cy = (area.y + area.h / 2.0).round() as i32;
        d.draw_mode2D(ui_camera, |mut d, _| {
            // End-of-round banner over a dimming overlay. A local round
            // stacks its numbers and a level's buttons under it
            // (`hud::result_layout`); an online one counts down to the
            // room's lobby.
            if let Some((title, color, sub)) = c.banner.as_ref().filter(|_| c.verdict > 0.0) {
                // Eased in after the finale: the dim deepens, the title
                // settles from a little above its row, everything fades up.
                let v = c.verdict;
                let eased = 1.0 - (1.0 - v) * (1.0 - v) * (1.0 - v);
                let fade = |col: Color| Color::new(col.r, col.g, col.b, (col.a as f32 * eased).round() as u8);
                let drop = ((1.0 - eased) * -16.0).round() as i32;
                d.draw_rectangle(0, 0, screen_w, screen_h, Color::new(0, 0, 0, (120.0 * eased).round() as u8));
                match &chrome.result {
                    Some(view) => {
                        let rows = crate::hud::result_layout(area, view);
                        draw_banner(&mut d, area, title, RESULT_TITLE_SIZE, rows.title_y as i32 + drop, fade(*color));
                        draw_result(&mut d, area, view, sub, eased);
                    }
                    None => {
                        let size = banner_size(title, RESULT_TITLE_SIZE, area);
                        draw_banner(&mut d, area, title, size, cy - size + drop, fade(*color));
                        draw_banner(&mut d, area, sub, BANNER_SUB_SIZE, cy + 20, fade(Color::RAYWHITE));
                    }
                }
            }

            // Mission banner: big white text over a dim overlay that both
            // fade together once the round unfreezes. A level adds its
            // number above and its title below, at half the size.
            if let Some((text, alpha)) = &c.intro {
                let a = |max: f32| (max * alpha) as u8;
                d.draw_rectangle(0, 0, screen_w, screen_h, Color::new(0, 0, 0, a(120.0)));
                let size = banner_size(text, BANNER_SIZE, area);
                let white = Color::new(255, 255, 255, a(255.0));
                draw_banner(&mut d, area, text, size, cy - size / 2, white);
                if let Some(level) = &chrome.level {
                    let number = t.fmt(keys::LEVEL_NUMBER, &[("n", level.number.into()), ("count", level.count.into())]);
                    let amber = Color::new(BUILD_COLOR.r, BUILD_COLOR.g, BUILD_COLOR.b, a(255.0));
                    draw_banner(&mut d, area, &number, LEVEL_NUMBER_SIZE, cy - size / 2 - 14 - LEVEL_NUMBER_SIZE, amber);
                    draw_banner(&mut d, area, &level.title, LEVEL_TITLE_SIZE, cy + size / 2 + 14, white);
                }
            }
            if let Some(text) = &c.wave_banner {
                let size = banner_size(text, WAVE_BANNER_SIZE, area);
                draw_banner(&mut d, area, text, size, cy - size / 2, Color::RAYWHITE);
            }

            // Paused overlay draws over everything else, including the
            // end-of-round banner (its countdown is frozen too).
            if self.paused {
                d.draw_rectangle(0, 0, screen_w, screen_h, Color::new(0, 0, 0, 120));
                let size = banner_size(&c.paused, BANNER_SIZE, area);
                draw_banner(&mut d, area, &c.paused, size, cy - size / 2, Color::RAYWHITE);
            }

            // The corner clusters, over the banners and their dims, which
            // leave them pressable, and at the frame's fade.
            if let Some(corners) = frame.corners {
                let fade = Fade { left: frame.fade.left * c.hud_end, right: frame.fade.right * c.hud_end };
                draw_corners(&mut d, corners, hud, chrome, self.players, textures, fade, &lines, minimap);
            }

            // The leave-round question (docs/game-editor-fusion.md
            // section 6): the same dim as PAUSED, over the corners too -
            // nothing behind a question is pressable - and the dialog on
            // top. The round is frozen by `app.rs` not calling `update`,
            // so nothing here is simulation state.
            if chrome.leave_dialog || chrome.players_dialog {
                d.draw_rectangle(0, 0, screen_w, screen_h, Color::new(0, 0, 0, 120));
            }
            if chrome.leave_dialog {
                draw_leave_dialog(&mut d, area);
            } else if chrome.players_dialog {
                draw_players_dialog(&mut d, area, self.players, ui.hints);
            }
            // The lobby (lobby.rs), with its own dim: the round behind it
            // is the local one, frozen because nothing calls `update` in
            // this mode.
            if let Some(lobby) = &chrome.lobby {
                draw_lobby(&mut d, ui, lobby, textures);
            }
            // The level select (level_select.rs), with its own dim, over
            // a round that stands still behind it.
            if let Some(levels) = &chrome.levels {
                draw_level_select(&mut d, ui, levels);
            }
            // A question about a kept map (BB-33), over everything.
            if let Some(question) = &chrome.question {
                crate::render::hud::draw_question(&mut d, ui.screen, area, question);
            }
        });

        // The touch scheme's stick, ripples and hint over everything,
        // where the thumbs are, in UI points like the HUD - the same size
        // on the glass whatever scale the world is drawn at.
        if let Some((touch, steer_right)) = touch {
            d.draw_mode2D(ui_camera, |mut d, _| touch.draw(&mut d, steer_right, ui.hints));
        }

        // The fade through black between rounds, over everything.
        if chrome.curtain > 0.0 {
            let black = Color::new(0, 0, 0, (255.0 * chrome.curtain.clamp(0.0, 1.0)).round() as u8);
            d.draw_mode2D(ui_camera, |mut d, _| d.draw_rectangle(0, 0, screen_w, screen_h, black));
        }
    }
}

impl Game {
    /// Where the tank in owner slot `slot` stands, wrecks included: where a
    /// crate's symbol flies to (`crate_fx::open`).
    fn tank_of_slot(&self, slot: usize) -> Option<Position> {
        self.world.query::<&Tank>().iter().find(|t| t.owner_slot() == slot).map(|t| t.position)
    }

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
    /// standing on it, in world pixels. `ground_drawn` says the weather's
    /// ground pass already covered the target in place of the bare ground
    /// tileset; `cull` is the world worth drawing (`Camera::cull`).
    fn paint_field_lit<D: RaylibDraw>(&self, d: &mut D, textures: &Textures, fx: &crate::fx::Fx, ground_drawn: bool, cull: Option<Rectangle>) {
        // The field itself is painted through the `Canvas` trait in the
        // three stages `mapshot` also runs on a CPU canvas (`paint_floor`,
        // `paint_tiles`, `paint_standing`); between them come the layers
        // only a live round has - fire, glows, the locate label,
        // projectiles, blasts, airborne debris, particles - drawn with
        // raylib directly. A `GpuCanvas` borrows `d` for one statement,
        // so each stage makes its own. Under weather the ground pass
        // draws the ground with the sky's mark on it, and the marks lie
        // over that: tracks in the snow.
        if ground_drawn {
            self.paint_floor_marks(&mut GpuCanvas::culled(d, textures, cull));
        } else {
            self.paint_floor(&mut GpuCanvas::culled(d, textures, cull));
        }
        // The fish under the water's surface (`fish.rs`): over its tiles,
        // under everything that burns, stands or flies over it.
        crate::render::fish::draw_fish(d, textures, self, fx.shoal(), cull);

        // The FPV drones' shadows on the ground under them, under the
        // tanks (docs/fpv-swarm.md "Drawing"); the drones themselves fly
        // over everything that stands, below.
        let drones = self.drones();
        if self.shadows_enabled {
            crate::game::paint_drone_shadows(&mut GpuCanvas::new(d, textures), &drones);
        }

        // Burning ground cells: tongues of flame standing on each
        // (`pyro::tongues`), leaning with the wind, over the ground, under
        // the tiles beside them (a burning doorway's walls still stand
        // over the fire) and under whatever drives through them. Upper
        // cells first, so a lower cell's flames stand in front.
        // A bomb's splash of lava is drawn as molten rock on the floor
        // (`lava::draw_pools`), not as flames.
        let mut cells: Vec<(Position, f32, f32)> =
            self.fires.iter().filter(|f| f.left > 0.0 && !f.lava).map(|f| (f.position(), f.left, f.total)).collect();
        cells.retain(|(at, ..)| !culled(cull, *at));
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

        self.paint_tiles(&mut GpuCanvas::culled(d, textures, cull));

        // A barrel whose fuse is lit pulses (additive, so it reads as
        // light on the drum rather than a disc over it).
        d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
            // An active portal glows from below its spiral.
            if self.portals_active() {
                for &at in self.portals.iter().filter(|at| !culled(cull, **at)) {
                    draw_portal_glow(&mut bd, at, self.time);
                }
            }
            for obstacle in self.world.query::<&Obstacle>().iter() {
                if obstacle.fuse.is_some() && !culled(cull, obstacle.position) {
                    draw_fuse_glow(&mut bd, obstacle.position, self.time);
                }
            }
            for (at, left, _total) in self.burning_cells() {
                if !culled(cull, at) {
                    draw_fire_glow(&mut bd, at, self.time, left);
                }
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
            for tank in self.world.query::<&crate::tank::Tank>().iter().filter(|tank| !culled(cull, tank.position)) {
                let lean = pyro::smoke_lean(&t, tank.position, self.time);
                pyro::draw_glows(&mut Rl(&mut bd), &crate::damage_stage::flames(tank, self.time, lean), bands);
            }
            for obstacle in self.world.query::<&Obstacle>().iter().filter(|o| o.burning && !culled(cull, o.position)) {
                pyro::draw_glows(&mut Rl(&mut bd), &crate::game::tile_flames(obstacle, self.time), bands);
            }
            for pickup in self.world.query::<&crate::pickup::Pickup>().iter().filter(|p| p.burn.is_some() && !culled(cull, p.position)) {
                pyro::draw_glows(&mut Rl(&mut bd), &crate::pickup::crate_flames(pickup, self.time), bands);
            }
        });

        self.paint_standing(&mut GpuCanvas::culled(d, textures, cull), PaintOptions { locate_cue: true });

        // The dust a sonic hammer's wave lifts and the shards of the panes
        // it shatters (docs/sonic-hammer.md): lingering, so lit with the
        // field; the arcs themselves shine in `paint_field_glowing`.
        if !self.sonic_waves.is_empty() {
            let t = tuning();
            let mut dust = Vec::new();
            for wave in &self.sonic_waves {
                dust.extend(crate::sonic::wave_dust(wave, &t, self.time));
            }
            pyro::draw(&mut GpuCanvas::new(d, textures), &dust);
        }

        // The chips a gauss rail's slug throws out of the far side of what
        // it went through (docs/gauss-rail.md "Look"): matter, so lit with
        // the field; the slug itself shines in `paint_field_glowing`.
        if !self.rail_slugs.is_empty() {
            let mut chips = Vec::new();
            for slug in &self.rail_slugs {
                chips.extend(crate::gauss::compose_slug_lit(slug));
            }
            pyro::draw(&mut GpuCanvas::new(d, textures), &chips);
        }

        // A gravity well's collapse: its dust thrown out (docs/gravity-
        // well.md "The collapse"), matter, so lit with the field.
        if self.well_fx.iter().any(|fx| fx.kind == crate::well::WellFxKind::Collapse) {
            let t = tuning();
            let mut dust = Vec::new();
            for fx in self.well_fx.iter().filter(|fx| fx.kind == crate::well::WellFxKind::Collapse && !culled(cull, fx.at)) {
                crate::well::compose_collapse_dust(&mut dust, fx.at, fx.age, pyro::smoke_lean(&t, fx.at, self.time), &t);
            }
            pyro::draw(&mut GpuCanvas::new(d, textures), &dust);
        }

        // A rod's impact - its dust rings and puffs and the debris it
        // throws - and the smoke off a fresh crater (docs/rod-from-god.md
        // "Drawing"): matter, so lit with the field; the column shines in
        // `paint_field_glowing`.
        if !self.rod_impacts.is_empty() || !self.craters.is_empty() {
            let t = tuning();
            let mut dust = Vec::new();
            for crater in self.craters.list() {
                let at = crate::map::cell_to_world(crater.cell.0, crater.cell.1);
                if !culled(cull, at) {
                    crate::rod::compose_crater_smoke(&mut dust, at, self.time - crater.at, pyro::smoke_lean(&t, at, self.time), &t);
                }
            }
            for fx in self.rod_impacts.iter().filter(|fx| !culled(cull, fx.at)) {
                crate::rod::compose_impact(&mut dust, fx, pyro::smoke_lean(&t, fx.at, self.time), &t);
            }
            pyro::draw(&mut GpuCanvas::new(d, textures), &dust);
        }

        // The FPV drones in the air over everything that stands, their
        // shadows on what is under them (docs/fpv-swarm.md "Drawing"):
        // lit with the field, their lamps in `paint_field_glowing`.
        crate::game::paint_drones(&mut GpuCanvas::new(d, textures), &drones, self.time);

        // Over everything that stands: each volcano's plume, steam while
        // it sleeps and ash from the rumble on (docs/volcano.md). It is
        // smoke, so the night darkens it with the rest.
        if !self.volcanoes.is_empty() {
            let t = tuning();
            let mut plume = Vec::new();
            for v in &self.volcanoes {
                crate::volcano::plume(v, self.time, &t, &mut plume);
            }
            pyro::draw(&mut GpuCanvas::new(d, textures), &plume);
        }
    }

    /// What shines by its own light, over the lit field and so as bright
    /// at night as at noon: the locate labels and the ammo gauges under
    /// the `ammo_seats`' rings, the light the shots throw, the shots, hits,
    /// flames and flares, the blasts, whatever is in the air and the
    /// particles, in world pixels. `day_pools` scales the daylight's glow
    /// pools on the ground (`draw_ground_light`), which the light map
    /// stands in for under a dark sky; `camera` is the view the target
    /// holds, which the shot shaders place themselves in and whose culling
    /// rectangle the many small things are tested against.
    #[allow(clippy::too_many_arguments)]
    fn paint_field_glowing<D: RaylibDraw>(
        &self,
        d: &mut D,
        textures: &Textures,
        mut shots: Option<&mut ShotShaders>,
        fx: &crate::fx::Fx,
        day_pools: f32,
        camera: &Camera,
        ammo_seats: &[u8],
    ) {
        let cull = camera.cull();
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
                    if (self.hide_players && tank.is_player()) || culled(cull, tank.position) {
                        continue;
                    }
                    crate::tank::draw_tank_glow(&mut c, tank, self.time, tank_glow);
                }
            });
        }

        // Every FPV drone's lamp - in the air and in the halos - over the
        // lit field, so a swarm reads at night; with its small light
        // (`fpv_lamp_light`) where the sky is dark.
        {
            let drones = self.drones();
            let halos: Vec<crate::fpv::HaloLook> = self
                .world
                .query::<&Tank>()
                .iter()
                .filter(|t| !(self.hide_players && t.is_player()) && !culled(cull, t.position))
                .filter_map(|t| crate::game::halo_of(t, self.time))
                .collect();
            if !drones.is_empty() || !halos.is_empty() {
                let lamps = crate::game::drone_lamps(&drones, &halos, self.time);
                pyro::draw(&mut GpuCanvas::new(d, textures), &lamps);
                let light = tuning().fpv_lamp_light * tank_glow;
                if light > 0.0 {
                    d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
                        for shape in &lamps {
                            if let pyro::Shape::Mark { pos, color, .. } = *shape {
                                ground_light(&mut bd, pos, 10.0, color, light);
                            }
                        }
                    });
                }
            }
        }

        // Lava shines by itself: under any sky that darkens the field its
        // molten blocks and the craters are drawn again over it whole, as
        // bright as by day - light the sky cannot take away.
        if tank_glow > 0.0 && (!self.lava.is_empty() || !self.volcanoes.is_empty()) {
            let look = self.lava_look();
            let t = tuning();
            let pictures = self.lava.pictures();
            let fresh = pictures.fresh(self.time);
            d.draw_blend_mode(BlendMode::BLEND_ALPHA, |mut bd| {
                let mut c = GpuCanvas::culled(&mut bd, textures, cull);
                if fresh && crate::canvas::Canvas::has_blocks(&c, pictures.glow.image.stamp) {
                    pictures.glow.draw(&mut c);
                } else {
                    for (col, row) in self.lava.cells() {
                        if !culled(cull, crate::map::cell_to_world(col, row)) {
                            crate::lava::draw_cell(&mut c, &self.lava, col, row, self.time, look, true, 1.0);
                        }
                    }
                }
                for v in &self.volcanoes {
                    if !culled(cull, v.centre()) {
                        let picture = crate::volcano::cone(&v.outlets);
                        let glow = if fresh { pictures.cone(v.cell).map(|(_, glow)| glow) } else { None };
                        crate::volcano::draw_cone_glow_with(&mut c, v.centre(), &picture, glow, &v.phase(self.time, &t), self.time, 1.0);
                    }
                }
            });
        }

        // The locate cue's P1..P8 labels, over the grass, the crowd and
        // the trees - the point is to be found under all of it.
        if !self.hide_players {
            for entity in self.players().into_iter().flatten() {
                crate::simulation::with_tank(&self.world, entity, |tank| draw_player_label(d, tank, self.time));
            }
            // What this screen's seats' triggers have left - the special
            // carried, else shells, the vitals' one readout
            // (`hud::WeaponSlot`) - as pips along the lower arc of their
            // rings: the number that matters most, where the eye already is.
            for &seat in ammo_seats {
                let Some(entity) = self.seat(seat as usize) else { continue };
                crate::simulation::with_tank(&self.world, entity, |tank| {
                    if !culled(cull, tank.position) {
                        // An offline special's pips are drawn unlit
                        // (docs/emp-burst.md).
                        let slot = crate::hud::WeaponSlot::of(tank, self.time);
                        let color = if slot.offline.is_some() { crate::hud::DIM } else { slot.color };
                        crate::tank::draw_ammo_pips(&mut GpuCanvas::new(d, textures), tank, slot.count, slot.full, color);
                    }
                });
            }
        }

        // A sonic hammer's arcs, an enemy's wind-up and the stars over a
        // stunned frog (docs/sonic-hammer.md): bright and brief, drawn
        // unlit so they read at night - a tell is a warning.
        {
            let t = tuning();
            let mut shapes = Vec::new();
            for wave in &self.sonic_waves {
                shapes.extend(crate::sonic::wave_arcs(wave, &t));
            }
            for tank in self.world.query::<&Tank>().iter().filter(|tank| !culled(cull, tank.position)) {
                // Each weapon's wind-up by its own composer.
                if let Some(w) = tank.windup().filter(|w| w.weapon == crate::tank::ActiveWeapon::SonicHammer) {
                    let dish = tank.turret_point(crate::tank_art::SONIC_MUZZLE[tank.row as usize]);
                    shapes.extend(crate::sonic::tell_arcs(dish, w.facing, w.progress, self.time, &t));
                }
            }
            for frog in self.world.query::<&crate::frog::Frog>().iter().filter(|f| !culled(cull, f.position)) {
                shapes.extend(crate::frog::stun_marks(frog, self.time));
            }
            if !shapes.is_empty() {
                pyro::draw(&mut GpuCanvas::new(d, textures), &shapes);
            }
        }

        // An EMP's rings, the sparks on every hull and tower it put out,
        // an enemy's crackle before its pulse and the lamp posts going out
        // (docs/emp-burst.md): bright and brief, drawn unlit so they read
        // at night; their glows in one additive block.
        {
            let t = tuning();
            let mut shapes = Vec::new();
            for pulse in &self.emp_pulses {
                shapes.extend(crate::emp::ring(pulse, &t));
            }
            for tank in self.world.query::<&Tank>().iter().filter(|tank| !tank.is_wreck() && !culled(cull, tank.position)) {
                let seed = crate::emp::spark_seed(tank.owner_slot());
                if tank.is_disabled() {
                    let (c, h) = tank.hull_bbox_world();
                    shapes.extend(crate::emp::sparks(c, crate::math::Vec2::new(h.x, h.y), seed, self.time, tank.disabled));
                }
                if let Some(w) = tank.windup().filter(|w| w.weapon == crate::tank::ActiveWeapon::Emp) {
                    let coil = tank.turret_point(crate::tank_art::EMP_COIL[tank.row as usize]);
                    shapes.extend(crate::emp::tell(coil, seed, w.progress, self.time));
                }
            }
            for view in self.tower_views().iter().filter(|v| v.disabled > 0.0 && !culled(cull, v.position)) {
                let half = crate::OBSTACLE_GRID_SIZE * 0.45;
                let seed = crate::blast::seed_at(view.position, 0x5C);
                shapes.extend(crate::emp::sparks(view.position, crate::math::Vec2::new(half, half), seed, self.time, view.disabled));
            }
            if self.lamps_out > 0.0 {
                let age = t.emp_lamp_seconds - self.lamps_out;
                for at in self.lamp_posts().into_iter().filter(|at| !culled(cull, *at)) {
                    shapes.extend(crate::emp::lamp_sparks(at, age));
                }
            }
            if !shapes.is_empty() {
                pyro::draw(&mut GpuCanvas::new(d, textures), &shapes);
                let bands = t.glow_bands.max(0) as u32;
                d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| pyro::draw_glows(&mut Rl(&mut bd), &shapes, bands));
            }
        }

        // A gauss rail's charge round the hull and at the module's bore,
        // its slugs - the white frame, the ion trail, the bursts where they
        // went through things - and the fizzles and vents
        // (docs/gauss-rail.md "Look"): light, drawn unlit, their glows in
        // one additive block.
        {
            let t = tuning();
            let mut shapes = Vec::new();
            for tank in self.world.query::<&Tank>().iter().filter(|tank| !tank.is_wreck() && !culled(cull, tank.position)) {
                if let Some(charge) = tank.charge.filter(|c| c.weapon == crate::tank::ActiveWeapon::GaussRail) {
                    let seed = (tank.owner_slot() as u32).wrapping_mul(0x9E37_79B9) ^ 0x6A55;
                    shapes.extend(crate::gauss::compose_charge(tank.position, crate::gauss::muzzle(tank), &charge, seed, self.time));
                }
            }
            for slug in &self.rail_slugs {
                shapes.extend(crate::gauss::compose_slug(slug, &t));
            }
            for end in &self.charge_ends {
                shapes.extend(crate::gauss::compose_end(end, pyro::smoke_lean(&t, end.at, self.time)));
            }
            if !shapes.is_empty() {
                pyro::draw(&mut GpuCanvas::new(d, textures), &shapes);
                let bands = t.glow_bands.max(0) as u32;
                d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| pyro::draw_glows(&mut Rl(&mut bd), &shapes, bands));
            }
        }

        // The gravity well (docs/gravity-well.md "Drawing"): every well
        // standing - its core, its ring, the swirl and the rim, read on the
        // zones' clock -, every orb in flight, and the snaps, collapses and
        // swallows. Light, drawn unlit, their glows in one additive block.
        if !self.orbs.is_empty() || !self.well_fx.is_empty() || self.zones.iter().any(|z| z.well().is_some()) {
            let t = tuning();
            let now = self.time + self.zone_lead;
            let mut shapes = Vec::new();
            for zone in self.zones.iter().filter(|z| z.well().is_some()) {
                if cull.is_none_or(|r| crate::math::Rectangle::new(r.x - t.well_radius_px, r.y - t.well_radius_px, r.width + 2.0 * t.well_radius_px, r.height + 2.0 * t.well_radius_px).contains(zone.centre)) {
                    crate::well::compose_well(&mut shapes, zone, now, &t);
                }
            }
            for orb in self.orbs.iter().filter(|o| !culled(cull, o.position)) {
                crate::well::compose_orb(&mut shapes, orb.position, orb.velocity, orb.age, orb.id);
            }
            for fx in self.well_fx.iter().filter(|fx| !culled(cull, fx.at)) {
                match fx.kind {
                    crate::well::WellFxKind::Snap => crate::well::compose_snap(&mut shapes, fx.at, fx.age),
                    crate::well::WellFxKind::Collapse => crate::well::compose_collapse(&mut shapes, fx.at, fx.age),
                    crate::well::WellFxKind::Swallow => crate::well::compose_swallow(&mut shapes, fx.at, fx.age),
                }
            }
            if !shapes.is_empty() {
                pyro::draw(&mut GpuCanvas::new(d, textures), &shapes);
                let bands = t.glow_bands.max(0) as u32;
                d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| pyro::draw_glows(&mut Rl(&mut bd), &shapes, bands));
            }
            // The cores over every light: holes, black whatever shines.
            let mut cores = Vec::new();
            for zone in self.zones.iter().filter(|z| z.well().is_some() && !culled(cull, z.centre)) {
                crate::well::compose_core(&mut cores, zone, now, &t);
            }
            if !cores.is_empty() {
                pyro::draw(&mut GpuCanvas::new(d, textures), &cores);
            }
        }

        // The rod from god (docs/rod-from-god.md "Drawing"): every reticle
        // up and its designator's line, every call standing - its circle,
        // its beam from the sky and its count, read on the zones' clock -
        // and the white column of an impact. Light, drawn unlit, their
        // glows in one additive block.
        {
            let t = tuning();
            let view_top = camera.rect().y;
            let mut shapes = Vec::new();
            crate::rod::compose_reticles(&mut shapes, self.world.query::<&Tank>().iter().filter(|tank| tank.reticle.is_some()), &t, self.time);
            crate::rod::compose_calls(&mut shapes, &self.zones, self.time + self.zone_lead, view_top, &t, self.time);
            for fx in &self.rod_impacts {
                crate::rod::compose_column(&mut shapes, fx, view_top, &t);
            }
            if !shapes.is_empty() {
                pyro::draw(&mut GpuCanvas::new(d, textures), &shapes);
                let bands = t.glow_bands.max(0) as u32;
                d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| pyro::draw_glows(&mut Rl(&mut bd), &shapes, bands));
            }
        }

        // The light the shots throw, under their sprites so each round
        // sits in its own glow: tracers, halos, the plasma's comet
        // tail, the laser's bloom (`render/shot_fx.rs`). One additive
        // block for all of it - a blend switch breaks the batch.
        let glow = tuning().shot_glow_strength > 0.0;
        // The lava's light on the ground (its tiles leave the target's
        // alpha alone, `draw_lava_ground_light`), then every other light
        // on the floor in one additive block.
        if glow {
            self.draw_lava_ground_light(d, textures, day_pools, cull);
        }
        if glow {
            d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
                // Light on the floor first: every shot, burn and flash
                // lights the ground around it in its own colour.
                self.draw_ground_light(&mut bd, day_pools, cull);
                for shell in self.world.query::<&Shell>().iter().filter(|s| !culled(cull, s.position)) {
                    draw_shell_light(&mut bd, shell);
                }
                for bullet in self.world.query::<&Bullet>().iter().filter(|b| !culled(cull, b.position)) {
                    draw_bullet_light(&mut bd, bullet);
                }
                for beam in &self.laser_beams {
                    draw_laser_bloom(&mut bd, beam);
                }
            });
        }

        // A hit is the burst the particle layer's impact composes
        // (`burst.rs`), so the baked impact frames are left out.
        for shell in self.world.query::<&Shell>().iter().filter(|s| !culled(cull, s.position)) {
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
        for plasma in self.world.query::<&Plasma>().iter().filter(|p| !culled(cull, p.position)) {
            let flying = plasma.state == PlasmaState::Flying;
            if self.shadows_enabled && flying {
                draw_plasma_shadow(d, textures.plasma, plasma, orb);
            }
            match shots.as_deref_mut() {
                Some(shots) if orb && flying => shots.draw_orb(d, plasma, self.time, camera),
                _ if !flying => {}
                _ => draw_plasma(d, textures.plasma, plasma),
            }
        }

        for bullet in self.world.query::<&Bullet>().iter().filter(|b| !culled(cull, b.position)) {
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
                .filter(|i| !culled(cull, i.pos))
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

        // Every crate being taken (`crate_fx::open`): what is left of it,
        // its planks and dust, its symbol rising and flying into the tank
        // that took it, the ring round that tank - over the tanks, since the
        // hull that takes a crate stands over it the frame it does.
        if !fx.opens().is_empty() {
            let t = tuning();
            let opens: Vec<(crate::fx::CrateOpen, crate::crate_fx::Open)> = fx
                .opens()
                .iter()
                .filter(|o| !culled(cull, o.at))
                .filter_map(|o| {
                    use crate::crate_fx::Opening;
                    let how = match o.slot {
                        None => Opening::Broken,
                        Some(slot) => {
                            let toward = self.tank_of_slot(slot).unwrap_or(o.at);
                            if o.spilled { Opening::Spilled { toward } } else { Opening::Taken { toward } }
                        }
                    };
                    crate::crate_fx::open(o.age, o.at, how, o.kind.ink(), &t).map(|open| (*o, open))
                })
                .collect();
            {
                let mut c = GpuCanvas::new(d, textures);
                for (o, open) in &opens {
                    if open.crate_alpha > 0.0 {
                        let tint = pyro::alpha(Color::WHITE, open.crate_alpha);
                        crate::pickup::draw_crate(&mut c, o.kind, o.at, crate::CRATE_COL_INTACT, self.shadows_enabled, tint);
                    }
                    pyro::draw(&mut c, &open.shapes);
                    if let Some(symbol) = open.symbol {
                        if let Some(at) = symbol.shadow.filter(|_| self.shadows_enabled) {
                            crate::pickup::draw_glyph(&mut c, o.kind, at, symbol.size, Color::new(0, 0, 0, 77));
                        }
                        crate::pickup::draw_glyph(&mut c, o.kind, symbol.pos, symbol.size, Color::WHITE);
                    }
                }
            }
            // The crate's flash and the symbol's blink are the sprite again
            // over itself in light, as a hit's flash is; then the glows.
            d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
                {
                    let mut c = GpuCanvas::new(&mut bd, textures);
                    for (o, open) in &opens {
                        if open.crate_flash > 0.0 {
                            let tint = pyro::alpha(Color::WHITE, open.crate_flash);
                            crate::pickup::draw_crate(&mut c, o.kind, o.at, crate::CRATE_COL_INTACT, false, tint);
                        }
                        if let Some(symbol) = open.symbol.filter(|s| s.flash > 0.0) {
                            crate::pickup::draw_glyph(&mut c, o.kind, symbol.pos, symbol.size, pyro::alpha(Color::WHITE, symbol.flash));
                        }
                    }
                }
                if glow {
                    let bands = t.glow_bands.max(0) as u32;
                    for (_, open) in &opens {
                        pyro::draw_glows(&mut Rl(&mut bd), &open.shapes, bands);
                    }
                }
            });
        }

        // The flamethrower's jet of burning fuel, over the tanks and
        // under its own flying motes.
        if let Some(shots) = shots.as_deref_mut().filter(|_| glow) {
            for (i, jet) in self.flames().iter().enumerate() {
                let (from, dir, reach) = jet.drawn();
                shots.draw_flame(d, from, dir, reach, self.time, i as f32 * 7.31, camera);
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
                .filter(|f| !at_nozzle(f.center, &nozzles) && !culled(cull, f.center))
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
        let flying = || self.decals.iter().filter(|dc| !dc.landed() && !culled(cull, dc.draw_pos()));
        if self.shadows_enabled {
            for decal in flying() {
                draw_decal_shadow(d, decal);
            }
        }
        {
            let mut c = GpuCanvas::new(d, textures);
            for decal in flying() {
                draw_decal(&mut c, decal);
            }
            // Launched fuel drums, tumbling over the lot on their way to
            // where they go off.
            for drum in &self.flying_drums {
                draw_flying_drum(&mut c, drum, self.shadows_enabled);
            }
            // The drums a gravity well holds, circling its ring lifted over
            // their shadows (docs/gravity-well.md "Held drums").
            if !self.held_drums.is_empty() {
                let t = tuning();
                for held in &self.held_drums {
                    let centre = held.centre;
                    let (ground, height) = crate::well::held_at(held, centre, self.time, &t);
                    if !culled(cull, ground) {
                        let turn = (ground - centre).y.atan2((ground - centre).x);
                        crate::obstacle::draw_held_drum(&mut c, ground, height, held.drum, turn, self.shadows_enabled);
                    }
                }
            }
        }

        // The volcanoes: where each bomb in the air will land, the bombs
        // themselves and the eruption's show over the crater, each with
        // its light (docs/volcano.md).
        if !self.volcanoes.is_empty() || !self.lava_bombs.is_empty() {
            let t = tuning();
            let mut shapes = Vec::new();
            for bomb in &self.lava_bombs {
                crate::volcano::bomb_ring(bomb, self.time, &t, &mut shapes);
            }
            for bomb in &self.lava_bombs {
                crate::volcano::bomb(bomb, self.time, &t, &mut shapes);
            }
            for v in &self.volcanoes {
                crate::volcano::eruption(v, self.time, &t, &mut shapes);
            }
            pyro::draw(&mut GpuCanvas::new(d, textures), &shapes);
            let bands = t.glow_bands.max(0) as u32;
            d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| pyro::draw_glows(&mut Rl(&mut bd), &shapes, bands));
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
        crate::render::fx::draw(fx, d, cull);
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
    // What the trigger fires right now, with its own remaining ammo: the
    // special weapon carried (`Tank::take_weapon`) until it runs dry, then
    // shells (WPN SHELL duplicates the AMMO line above; harmless, and it
    // keeps this line self-contained).
    let (wpn_name, wpn_ammo) = match tank.active_weapon() {
        ActiveWeapon::Laser => ("LASER", tank.laser_charges),
        ActiveWeapon::Plasma => ("PLASMA", tank.plasma_ammo),
        ActiveWeapon::Minigun => ("MINIGUN", tank.minigun_ammo),
        ActiveWeapon::Missiles => ("MISSILES", tank.missile_ammo),
        ActiveWeapon::Flamethrower => ("FLAME", tank.flame_fuel_seconds()),
        ActiveWeapon::Grenades => ("GRENADES", tank.grenade_ammo),
        ActiveWeapon::SonicHammer => ("SONIC", tank.sonic_ammo),
        ActiveWeapon::Emp => ("EMP", tank.emp_charges),
        ActiveWeapon::GaussRail => ("RAIL", tank.gauss_slugs),
        ActiveWeapon::FpvSwarm => ("FPV", tank.fpv_drones),
        ActiveWeapon::RodFromGod => ("ROD", tank.rods),
        ActiveWeapon::GravityWell => ("WELL", tank.wells),
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
    if tank.special_down() {
        // An EMP's outage: the electrics, and the special on its own.
        lines.push(format!("EMP OUT {:.1}s / {:.1}s", tank.disabled, tank.special_offline));
    }
    if let Some(charge) = tank.charge {
        // A charge on the trigger (a gauss rail): seconds held and stage.
        lines.push(format!("CHARGE {:.2}s {}", charge.held, charge.stage().name().to_uppercase()));
    }
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
    /// in world pixels and post-composite like them: blocked nav cells,
    /// each enemy's AI memory, projectile hit boxes, engagement targets,
    /// pickup collect radii. Each layer costs nothing while off.
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
            // Every air target's strike box, and each drone's line to its
            // aim with its lock's slot by it (docs/fpv-swarm.md "Drawing").
            for target in self.air_targets() {
                let (c, h) = target.strike_box();
                d.draw_rectangle_lines((c.x - h.x).round() as i32, (c.y - h.y).round() as i32, (h.x * 2.0).round() as i32, (h.y * 2.0).round() as i32, Color::ORANGE);
            }
            for drone in self.drones() {
                let at = drone.drawn();
                d.draw_line(at.x as i32, at.y as i32, drone.aim.x as i32, drone.aim.y as i32, Color::new(255, 160, 0, 160));
                if let Some(slot) = drone.lock.slot() {
                    d.draw_text(&slot.to_string(), drone.aim.x as i32 + 4, drone.aim.y as i32 - 12, 10, Color::ORANGE);
                }
            }
        }
        // A gravity well's reach, and round an enemy's the herd's ring the
        // pack's firing slots stand on (docs/gravity-well.md).
        if ov.engage {
            let t = tuning();
            for zone in self.zones.iter().filter(|z| z.well().is_some()) {
                let (cx, cy) = (zone.centre.x as i32, zone.centre.y as i32);
                d.draw_circle_lines(cx, cy, t.well_radius_px, Color::new(0x9A, 0x5C, 0xF0, 200));
                if !zone.owner.is_player() {
                    d.draw_circle_lines(cx, cy, t.well_ai_herd_px, Color::new(0xC9, 0x8C, 0xFF, 140));
                }
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
