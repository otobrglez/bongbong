//! The raylib half of the weather (docs/weather.md; `weather.rs` is the
//! headless half): the light map and the three passes a weathered frame
//! runs around pass 1 of `Game::render`.
//!
//! - **The light map** (`light` target, the size of the scene target - the
//!   camera's view at a texel per world pixel): cleared to the look's
//!   ambient, then every light of `weather::lights` added in, each a fan of
//!   triangles out to where its rays stop, bright at the source and falling
//!   off with distance - so walls cast shadows. Stored halved, so a pixel
//!   can be lit up to twice daylight.
//! - **The ground pass** (`weather_ground.fs`): the bare ground tileset,
//!   painted into the `ground` target first, drawn onto the field with
//!   snow, ice, wet earth, puddles and rings on it, before any tread mark,
//!   tile or tank - so tanks leave tracks in the snow and walls stay dry.
//! - **The light pass** (`weather_light.fs`): the field as it stands under
//!   daylight, multiplied by the light map on the 2 px block grid, before
//!   the shots, flares and fireballs are drawn - those shine by their own
//!   light and stay hot in the dark.
//! - **The sky pass** (`weather_sky.fs`): fog, blowing sand, rain, snow,
//!   heat haze and the white of lightning, over the finished field.
//!
//! The passes ping-pong between `scene_target` and this module's `lit`
//! target and always end in `scene_target`, so pass 2, the ripple
//! re-blits and the dev server's screenshots read the weathered field
//! with no change of their own. Every target is the camera's view
//! (`view::Camera`) and every pass works in world pixels, so the snow, the
//! rain and the light's bands stay on the world's block grid and the sun's
//! gradient spans the field, while the vignette frames the view. Every
//! uniform is set per frame from the round, the camera and the tuning
//! table; nothing is kept between frames but the targets and the cell
//! mask.

use sola_raylib::prelude::*;
use std::sync::Arc;

use crate::fx::Fx;
use crate::ground::Depth;
use crate::map::{cell_to_world, CellObject};
use crate::math::{Color, Rectangle, Vec2};
use crate::render::canvas::GpuCanvas;
use crate::render::game::Textures;
use crate::simulation::Game;
use crate::tuning::{tuning, Tuning};
use crate::view::Camera;
use crate::weather::{lightning, lights_in, Light, Look, Plan, Rgb, Shape};
use crate::{MAX_SEATS, OBSTACLE_GRID_SIZE};

/// The shaders' clock wraps after this many seconds. Their noise hashes
/// the clock through positions (a raindrop's fall is `time * speed`
/// pixels), and a float that has counted an hour of rain at 600 px/s no
/// longer holds a pixel; half an hour keeps every position exact to a
/// quarter pixel. A wrap is a jump in the pattern once a half hour.
const CLOCK_WRAP_SECONDS: f32 = 1800.0;

/// Where a light's falloff is sampled along each ray, as fractions of its
/// radius: denser near the source, where the curve bends most, so the
/// straight-edged triangles between them draw a smooth pool.
const RINGS: [f32; 5] = [0.0, 0.16, 0.38, 0.66, 1.0];

struct LightLocs {
    field_size: i32,
    view_origin: i32,
    view_size: i32,
    sun: i32,
    ambient: i32,
    bands: i32,
    dither: i32,
    darkness: i32,
    vignette: i32,
    light_map: i32,
}

struct GroundLocs {
    view_origin: i32,
    view_size: i32,
    cells: i32,
    time: i32,
    rain: i32,
    splash_rate: i32,
    snow: i32,
    frozen: i32,
    cell_mask: i32,
}

struct SkyLocs {
    view_origin: i32,
    view_size: i32,
    time: i32,
    lit: i32,
    ambient: i32,
    rain: i32,
    rain_speed: i32,
    rain_slant: i32,
    fog: i32,
    fog_drift: i32,
    sand: i32,
    sand_wind: i32,
    snow: i32,
    haze: i32,
    haze_amp: i32,
    haze_speed: i32,
    flash: i32,
    clear_radius: i32,
    seats: i32,
    seat_count: i32,
    gust_on: i32,
    gust_since: i32,
    gust_dir: i32,
    gust_front: i32,
    gust_len: i32,
    light_map: i32,
}

/// The three compiled passes and their uniform locations.
struct PassShaders {
    light: Shader,
    light_locs: LightLocs,
    ground: Shader,
    ground_locs: GroundLocs,
    sky: Shader,
    sky_locs: SkyLocs,
}

/// The render targets, each the scene target's size, re-created when the
/// camera's view changes size.
struct Targets {
    size: (i32, i32),
    light: RenderTexture2D,
    ground: RenderTexture2D,
    lit: RenderTexture2D,
}

/// What the ground pass knows about each map cell - its water and its
/// road - as a texture one texel per cell, re-uploaded only when it
/// changes (a new round, another map).
struct Mask {
    cols: i32,
    rows: i32,
    bytes: Vec<u8>,
    texture: Texture2D,
}

/// The weather's GPU state for the whole session. `app.rs` owns it and
/// hands it to `Game::render` through `Effects::weather`; `None` there (a
/// driver that could not compile the passes) draws every sky clear.
pub struct WeatherFx {
    shaders: PassShaders,
    targets: Option<Targets>,
    mask: Option<Mask>,
    /// Set by the first target or mask that could not be made: said once
    /// on stderr, not once a frame.
    warned: bool,
}

/// One frame's weather, resolved by `WeatherFx::begin` before pass 1: the
/// stages it runs and every value their uniforms take.
pub struct WeatherFrame {
    pub plan: Plan,
    look: Look,
    /// The light the light map was cleared to: the look's ambient, lifted
    /// toward daylight while lightning strikes.
    ambient: Rgb,
    /// The round clock, wrapped (`CLOCK_WRAP_SECONDS`).
    time: f32,
    /// Lightning, 0 to 1.
    flash: f32,
    /// The view the frame is drawn through; `size` is its targets' size.
    camera: Camera,
    size: (i32, i32),
    cells: (i32, i32),
    seats: [Vector2; MAX_SEATS],
    seat_count: usize,
    /// The sandstorm gust crossing the field: seconds since its front left
    /// and the way it blows (`weather::gust_on_field`).
    gust: Option<(f32, Vector2)>,
    /// The rules froze the round's water (`WaterLayout::is_frozen`).
    frozen: bool,
    tuning: Arc<Tuning>,
}

impl WeatherFrame {
    /// How much of the daylight's glow pools the shots should still throw
    /// on the ground (`Game::draw_ground_light`): none where the light map
    /// already lights the ground around every shot, all of it under a sky
    /// whose lights barely show.
    pub fn day_light_pools(&self) -> f32 {
        if self.plan.lit { (1.0 - self.look.lights).clamp(0.0, 1.0) } else { 1.0 }
    }
}

fn compile(rl: &mut RaylibHandle, thread: &RaylibThread, name: &str, source: &str) -> Result<Shader, String> {
    rl.load_shader_from_memory(thread, None, Some(source)).map_err(|e| format!("{name}: {e}"))
}

/// A colour of the light map: `rgb` halved, as the target stores it.
fn stored(rgb: Rgb, alpha: u8) -> Color {
    let c = |v: f32| (v * 0.5 * 255.0).round().clamp(0.0, 255.0) as u8;
    Color::new(c(rgb[0]), c(rgb[1]), c(rgb[2]), alpha)
}

/// Where a pass's target lies in the world: the world point at its
/// top-left corner and its size, a texel per world pixel.
fn set_view(s: &mut Shader, origin_loc: i32, size_loc: i32, frame: &WeatherFrame) {
    s.set_shader_value(origin_loc, Vector2::new(frame.camera.origin.x, frame.camera.origin.y));
    s.set_shader_value(size_loc, Vector2::new(frame.size.0 as f32, frame.size.1 as f32));
}

/// A render texture read the right way up: the whole of it, flipped (raylib
/// stores a target bottom-up).
fn whole(size: (i32, i32)) -> Rectangle {
    Rectangle::new(0.0, 0.0, size.0 as f32, -(size.1 as f32))
}

impl WeatherFx {
    /// Compile the three passes (the GLSL ES 100 twins in `static/web/` on
    /// the embedded builds; every source is in the binary). A failure is
    /// returned rather than fatal: the game then draws every sky clear.
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Result<Self, String> {
        let (light_src, ground_src, sky_src) = if crate::EMBEDDED {
            (
                include_str!("../../static/web/weather_light.fs"),
                include_str!("../../static/web/weather_ground.fs"),
                include_str!("../../static/web/weather_sky.fs"),
            )
        } else {
            (include_str!("../../static/weather_light.fs"), include_str!("../../static/weather_ground.fs"), include_str!("../../static/weather_sky.fs"))
        };
        let light = compile(rl, thread, "weather_light.fs", light_src)?;
        let ground = compile(rl, thread, "weather_ground.fs", ground_src)?;
        let sky = compile(rl, thread, "weather_sky.fs", sky_src)?;
        let light_locs = LightLocs {
            field_size: light.get_shader_location("fieldSize"),
            view_origin: light.get_shader_location("viewOrigin"),
            view_size: light.get_shader_location("viewSize"),
            sun: light.get_shader_location("sun"),
            ambient: light.get_shader_location("ambient"),
            bands: light.get_shader_location("bands"),
            dither: light.get_shader_location("dither"),
            darkness: light.get_shader_location("darkness"),
            vignette: light.get_shader_location("vignette"),
            light_map: light.get_shader_location("lightMap"),
        };
        let ground_locs = GroundLocs {
            view_origin: ground.get_shader_location("viewOrigin"),
            view_size: ground.get_shader_location("viewSize"),
            cells: ground.get_shader_location("cells"),
            time: ground.get_shader_location("time"),
            rain: ground.get_shader_location("rain"),
            splash_rate: ground.get_shader_location("splashRate"),
            snow: ground.get_shader_location("snow"),
            frozen: ground.get_shader_location("frozen"),
            cell_mask: ground.get_shader_location("cellMask"),
        };
        let sky_locs = SkyLocs {
            view_origin: sky.get_shader_location("viewOrigin"),
            view_size: sky.get_shader_location("viewSize"),
            time: sky.get_shader_location("time"),
            lit: sky.get_shader_location("lit"),
            ambient: sky.get_shader_location("ambient"),
            rain: sky.get_shader_location("rain"),
            rain_speed: sky.get_shader_location("rainSpeed"),
            rain_slant: sky.get_shader_location("rainSlant"),
            fog: sky.get_shader_location("fog"),
            fog_drift: sky.get_shader_location("fogDrift"),
            sand: sky.get_shader_location("sand"),
            sand_wind: sky.get_shader_location("sandWind"),
            snow: sky.get_shader_location("snow"),
            haze: sky.get_shader_location("haze"),
            haze_amp: sky.get_shader_location("hazeAmp"),
            haze_speed: sky.get_shader_location("hazeSpeed"),
            flash: sky.get_shader_location("flash"),
            clear_radius: sky.get_shader_location("clearRadius"),
            // Resolved as "name[0]", which every driver accepts for an
            // array uniform (see `RippleFx::load`).
            seats: sky.get_shader_location("seats[0]"),
            seat_count: sky.get_shader_location("seatCount"),
            gust_on: sky.get_shader_location("gustOn"),
            gust_since: sky.get_shader_location("gustSince"),
            gust_dir: sky.get_shader_location("gustDir"),
            gust_front: sky.get_shader_location("gustFront"),
            gust_len: sky.get_shader_location("gustLen"),
            light_map: sky.get_shader_location("lightMap"),
        };
        Ok(WeatherFx { shaders: PassShaders { light, light_locs, ground, ground_locs, sky, sky_locs }, targets: None, mask: None, warned: false })
    }

    /// Resolve this frame's weather for `game` and draw what has to exist
    /// before pass 1: the light map (the round's lights and the particle
    /// layer's hits) and the bare ground for the ground pass, both over
    /// `camera`'s view. `None` is a clear frame - the sky in force draws
    /// nothing, or its targets could not be made - and pass 1 runs as it
    /// would with no weather at all.
    pub fn begin(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, game: &Game, fx: &Fx, textures: &Textures, camera: &Camera) -> Option<WeatherFrame> {
        let t = tuning();
        let look = Look::of(game.weather(), &t);
        let plan = look.plan()?;
        let (w, h) = game.map.field_size();
        let size = camera.target_size();
        let cells = match self.ensure_targets(rl, thread, size).and_then(|()| if plan.ground { self.ensure_mask(rl, thread, game) } else { Ok((1, 1)) }) {
            Ok(cells) => cells,
            Err(e) => {
                if !self.warned {
                    eprintln!("[weather] {e}; drawing the sky clear");
                    self.warned = true;
                }
                return None;
            }
        };
        // A strike is a whole-screen flash, so the reduced-flash knob that
        // calms the kill flash and the shake calms it too.
        let flash = if look.lightning > 0.0 { (lightning(game.time, &t) * look.lightning * t.screen_fx_intensity).clamp(0.0, 1.5) } else { 0.0 };
        let ambient = look.ambient.map(|a| a + (1.05 - a).max(0.0) * (flash * 0.9).min(1.0));
        let targets = self.targets.as_mut().expect("ensure_targets made them");
        if plan.lit {
            let all = lights_in(game, fx.impacts(), &look, &t, camera.part());
            draw_light_map(rl, thread, &mut targets.light, ambient, &all, camera);
        }
        if plan.ground {
            rl.draw_texture_mode(thread, &mut targets.ground, |mut d| {
                d.clear_background(Color::WHITE);
                d.draw_mode2D(camera.in_target(), |mut d, _| game.paint_ground(&mut GpuCanvas::culled(&mut d, textures, camera.cull())));
            });
        }
        let mut seats = [Vector2::new(0.0, 0.0); MAX_SEATS];
        let mut seat_count = 0;
        for entity in game.players().into_iter().flatten() {
            let at = crate::simulation::with_tank(&game.world, entity, |tank| tank.position);
            if seat_count < MAX_SEATS {
                seats[seat_count] = Vector2::new(at.x, at.y);
                seat_count += 1;
            }
        }
        // The gust the rules push the hulls with, drawn where it blows: its
        // age travels rather than its start, so the wrapped clock never
        // splits a front in two.
        let gust = (t.weather_rules && game.weather() == crate::map::Weather::Sandstorm)
            .then(|| crate::weather::gust_on_field(game.time, w, h, &t))
            .flatten()
            .map(|g| (game.time - g.start, Vector2::new(g.dir.x, g.dir.y)));
        Some(WeatherFrame {
            plan,
            look,
            ambient,
            time: game.time.rem_euclid(CLOCK_WRAP_SECONDS),
            flash,
            camera: *camera,
            size,
            cells,
            seats,
            seat_count,
            gust,
            frozen: game.water().is_frozen(),
            tuning: t,
        })
    }

    /// The target the passes ping-pong through and the passes themselves,
    /// borrowed apart so one can be drawn into while the others are read.
    /// `None` before the first `begin` that ran.
    pub fn stages(&mut self) -> Option<(&mut RenderTexture2D, Passes<'_>)> {
        let targets = self.targets.as_mut()?;
        let Targets { light, ground, lit, .. } = targets;
        let mask = self.mask.as_ref().map(|m| &m.texture);
        Some((lit, Passes { shaders: &mut self.shaders, light, ground, mask }))
    }

    fn ensure_targets(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, size: (i32, i32)) -> Result<(), String> {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return Ok(());
        }
        let make = |rl: &mut RaylibHandle| rl.load_render_texture(thread, size.0 as u32, size.1 as u32).map_err(|e| format!("weather target: {e}"));
        let light = make(rl)?;
        let ground = make(rl)?;
        let lit = make(rl)?;
        self.targets = Some(Targets { size, light, ground, lit });
        Ok(())
    }

    /// The cell mask for `game`'s map, uploaded when it changed; the
    /// mask's size in cells.
    fn ensure_mask(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, game: &Game) -> Result<(i32, i32), String> {
        let (cols, rows, bytes) = mask_bytes(game);
        match &mut self.mask {
            Some(mask) if mask.cols == cols && mask.rows == rows => {
                if mask.bytes != bytes {
                    mask.texture.update_texture(&bytes).map_err(|e| format!("cell mask: {e}"))?;
                    mask.bytes = bytes;
                }
            }
            _ => {
                let image = Image::gen_image_color(cols, rows, Color::BLACK);
                let mut texture = rl.load_texture_from_image(thread, &image).map_err(|e| format!("cell mask: {e}"))?;
                texture.update_texture(&bytes).map_err(|e| format!("cell mask: {e}"))?;
                self.mask = Some(Mask { cols, rows, bytes, texture });
            }
        }
        Ok((cols, rows))
    }
}

/// The ground pass's per-cell facts about `game`'s map, RGBA8 row by row:
/// red the water's depth (0 dry, 128 a ford, 255 deep), green road. The
/// grid is the ground layer's: cell `(c, r)` centred on `(c, r) * 32`, one
/// more column and row than the field is wide and tall.
fn mask_bytes(game: &Game) -> (i32, i32, Vec<u8>) {
    let (w, h) = game.map.field_size();
    let cols = (w / OBSTACLE_GRID_SIZE).ceil() as i32 + 1;
    let rows = (h / OBSTACLE_GRID_SIZE).ceil() as i32 + 1;
    let mut bytes = vec![0u8; (cols * rows * 4).max(0) as usize];
    for r in 0..rows {
        for c in 0..cols {
            let i = ((r * cols + c) * 4) as usize;
            bytes[i] = match game.water.depth_at(cell_to_world(c, r)) {
                Depth::Dry => 0,
                Depth::Shallow => 128,
                Depth::Deep | Depth::Ice => 255,
            };
            bytes[i + 3] = 255;
        }
    }
    for (c, r, cell) in game.map.iter_cells() {
        if matches!(cell, CellObject::Road) && (0..cols).contains(&c) && (0..rows).contains(&r) {
            bytes[((r * cols + c) * 4 + 1) as usize] = 255;
        }
    }
    (cols, rows, bytes)
}

/// Draw the light map over `camera`'s view: `ambient` everywhere and every
/// light added over it, in world pixels.
fn draw_light_map(rl: &mut RaylibHandle, thread: &RaylibThread, target: &mut RenderTexture2D, ambient: Rgb, lights: &[Light], camera: &Camera) {
    rl.draw_texture_mode(thread, target, |mut d| {
        d.clear_background(stored(ambient, 255));
        d.draw_mode2D(camera.in_target(), |mut d, _| {
            d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
                for light in lights {
                    draw_light(&mut bd, light);
                }
            });
        });
    });
}

/// One light as a fan of triangles: `RINGS` vertices down every ray, each
/// carrying the light's colour at that distance, and nothing past where
/// the ray stopped - which is the shadow. Straight into raylib's batch
/// with a colour per vertex, like `shot_fx`'s streaks; call inside the
/// additive block. A point light's last ray joins its first; a cone's
/// edges fade out on their own (`Light::ray`).
fn draw_light(_d: &mut impl RaylibDraw, light: &Light) {
    let n = light.reach.len();
    if n < 2 || light.radius < 1.0 {
        return;
    }
    let cone = matches!(light.shape, Shape::Cone { .. });
    let falloff = |dist: f32| {
        let u = (1.0 - dist / light.radius).clamp(0.0, 1.0);
        if cone { u.powf(1.3) } else { u * u }
    };
    let rays: Vec<(Vec2, f32)> = (0..n).map(|i| light.ray(i, n)).collect();
    let vertex = |ray: usize, ring: usize| -> (Vec2, Color) {
        let (dir, weight) = rays[ray];
        let dist = (RINGS[ring] * light.radius).min(light.reach[ray]);
        let k = weight * falloff(dist);
        (light.at + dir * dist, stored([light.color[0] * k, light.color[1] * k, light.color[2] * k], 255))
    };
    let spans = if cone { n - 1 } else { n };
    // SAFETY: plain rlgl immediate-mode calls between the begin and end of
    // a drawing pass (the caller holds a draw handle); they only append
    // vertices to raylib's current batch, which flushes itself when full.
    unsafe {
        sola_raylib::ffi::rlBegin(0x0004); // RL_TRIANGLES
        for i in 0..spans {
            let j = (i + 1) % n;
            for ring in 0..RINGS.len() - 1 {
                let a0 = vertex(i, ring);
                let a1 = vertex(i, ring + 1);
                let b0 = vertex(j, ring);
                let b1 = vertex(j, ring + 1);
                push_triangle(a0, a1, b1);
                push_triangle(a0, b1, b0);
            }
        }
        sola_raylib::ffi::rlEnd();
    }
}

/// Three vertices of one triangle, counter-clockwise on screen as raylib
/// wants them, into the open `rlBegin` batch.
///
/// # Safety
/// Only between `rlBegin(RL_TRIANGLES)` and `rlEnd` inside a drawing pass.
unsafe fn push_triangle(a: (Vec2, Color), b: (Vec2, Color), c: (Vec2, Color)) {
    let cross = (b.0.x - a.0.x) * (c.0.y - a.0.y) - (b.0.y - a.0.y) * (c.0.x - a.0.x);
    let (b, c) = if cross > 0.0 { (c, b) } else { (b, c) };
    for (p, col) in [a, b, c] {
        // SAFETY: the caller's contract - inside an open rlgl batch.
        unsafe {
            sola_raylib::ffi::rlColor4ub(col.r, col.g, col.b, col.a);
            sola_raylib::ffi::rlVertex2f(p.x, p.y);
        }
    }
}

/// The three passes, with the targets they read. Borrowed out of
/// `WeatherFx` by `stages`.
pub struct Passes<'a> {
    shaders: &'a mut PassShaders,
    light: &'a RenderTexture2D,
    ground: &'a RenderTexture2D,
    mask: Option<&'a Texture2D>,
}

impl Passes<'_> {
    /// The bare ground with the sky's mark on it (`weather_ground.fs`),
    /// over the whole target: what pass 1 draws in place of the ground
    /// tileset. Drawn in the target's own pixels, outside the world's
    /// camera.
    pub fn draw_ground<D: RaylibDraw>(&mut self, d: &mut D, frame: &WeatherFrame) {
        let Some(mask) = self.mask else {
            return;
        };
        let t = &frame.tuning;
        let l = &self.shaders.ground_locs;
        let s = &mut self.shaders.ground;
        set_view(s, l.view_origin, l.view_size, frame);
        s.set_shader_value(l.cells, Vector2::new(frame.cells.0 as f32, frame.cells.1 as f32));
        s.set_shader_value(l.time, frame.time);
        s.set_shader_value(l.rain, frame.look.rain);
        s.set_shader_value(l.splash_rate, t.rain_splash_rate);
        s.set_shader_value(l.snow, (frame.look.snow * t.snow_cover).clamp(0.0, 1.0));
        s.set_shader_value(l.frozen, if frame.frozen { 1.0f32 } else { 0.0 });
        let (raw, mask_loc, mask_tex) = (*s.as_ref(), l.cell_mask, *mask.as_ref());
        let source = self.ground;
        d.draw_shader_mode(s, |mut sd| {
            // SAFETY: a copy of the bound shader's handle and a live
            // texture; raylib binds the sampler for this batch (it has to
            // be set inside the shader mode, which resets the extra units).
            unsafe { sola_raylib::ffi::SetShaderValueTexture(raw, mask_loc, mask_tex) };
            sd.draw_texture_rec(source, whole(frame.size), Vector2::new(0.0, 0.0), Color::WHITE);
        });
    }

    /// `source` - the field as it stands under daylight - multiplied by
    /// the light map (`weather_light.fs`), over the whole target, in the
    /// target's own pixels.
    pub fn draw_lit<D: RaylibDraw>(&mut self, d: &mut D, source: &RenderTexture2D, frame: &WeatherFrame) {
        let t = &frame.tuning;
        let l = &self.shaders.light_locs;
        let s = &mut self.shaders.light;
        let darkness = {
            let lum = 0.299 * frame.ambient[0] + 0.587 * frame.ambient[1] + 0.114 * frame.ambient[2];
            (1.0 - lum).clamp(0.0, 1.0)
        };
        s.set_shader_value(l.field_size, Vector2::new(frame.camera.field.0, frame.camera.field.1));
        set_view(s, l.view_origin, l.view_size, frame);
        s.set_shader_value(l.ambient, frame.ambient);
        s.set_shader_value(l.bands, t.light_bands.max(0) as f32);
        s.set_shader_value(l.dither, if t.light_dither { 1.0f32 } else { 0.0 });
        s.set_shader_value(l.darkness, darkness);
        s.set_shader_value(l.vignette, frame.look.vignette.clamp(0.0, 1.0));
        s.set_shader_value(l.sun, [0.3 * frame.look.sun, 0.15 * frame.look.sun, 0.03 * frame.look.sun]);
        let (raw, light_loc, light_tex) = (*s.as_ref(), l.light_map, *self.light.as_ref());
        d.draw_shader_mode(s, |mut sd| {
            // SAFETY: as in `draw_ground`.
            unsafe { sola_raylib::ffi::SetShaderValueTexture(raw, light_loc, light_tex) };
            sd.draw_texture_rec(source, whole(frame.size), Vector2::new(0.0, 0.0), Color::WHITE);
        });
    }

    /// `source` - the finished field - with the air over it
    /// (`weather_sky.fs`), over the whole target, in the target's own
    /// pixels.
    pub fn draw_sky<D: RaylibDraw>(&mut self, d: &mut D, source: &RenderTexture2D, frame: &WeatherFrame) {
        let t = &frame.tuning;
        let l = &self.shaders.sky_locs;
        let s = &mut self.shaders.sky;
        set_view(s, l.view_origin, l.view_size, frame);
        s.set_shader_value(l.time, frame.time);
        s.set_shader_value(l.lit, if frame.plan.lit { 1.0f32 } else { 0.0 });
        s.set_shader_value(l.ambient, frame.ambient);
        s.set_shader_value(l.rain, frame.look.rain);
        s.set_shader_value(l.rain_speed, t.rain_speed_px);
        s.set_shader_value(l.rain_slant, t.rain_slant);
        s.set_shader_value(l.fog, frame.look.fog);
        s.set_shader_value(l.fog_drift, t.fog_drift_speed);
        s.set_shader_value(l.sand, frame.look.sand);
        s.set_shader_value(l.sand_wind, t.sand_wind_speed);
        s.set_shader_value(l.snow, frame.look.snow);
        s.set_shader_value(l.haze, frame.look.haze);
        s.set_shader_value(l.haze_amp, t.haze_amplitude_px);
        s.set_shader_value(l.haze_speed, t.haze_speed);
        s.set_shader_value(l.flash, frame.flash * 0.85);
        s.set_shader_value(l.clear_radius, t.weather_clear_radius_px);
        s.set_shader_value_v(l.seats, &frame.seats);
        s.set_shader_value(l.seat_count, frame.seat_count as f32);
        let (since, dir) = frame.gust.unwrap_or((0.0, Vector2::new(1.0, 0.0)));
        s.set_shader_value(l.gust_on, if frame.gust.is_some() { 1.0f32 } else { 0.0 });
        s.set_shader_value(l.gust_since, since);
        s.set_shader_value(l.gust_dir, dir);
        s.set_shader_value(l.gust_front, t.sand_gust_front_speed.max(1.0));
        s.set_shader_value(l.gust_len, t.sand_gust_seconds.max(0.05));
        let (raw, light_loc, light_tex) = (*s.as_ref(), l.light_map, *self.light.as_ref());
        d.draw_shader_mode(s, |mut sd| {
            // SAFETY: as in `draw_ground`.
            unsafe { sola_raylib::ffi::SetShaderValueTexture(raw, light_loc, light_tex) };
            sd.draw_texture_rec(source, whole(frame.size), Vector2::new(0.0, 0.0), Color::WHITE);
        });
    }
}
