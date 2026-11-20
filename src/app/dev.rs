//! The dev tools' side of the frame loop (`dev-tools`): the dev server a
//! native window answers (`devserver.rs`), the page's C API on the web
//! (`capi.rs`) and a phone's frame times on its console. `App` calls every
//! hook on every frame, in every build; a hook does its work only where
//! its tool is built, and without the feature each is a no-op.

use super::{Args, Session};
use crate::ai::Intent;
use crate::editor::BuilderInput;
use crate::follow::CameraReport;
use crate::hud::{Hints, UiFrame};
use crate::math::Vec2;
use crate::simulation::{Game, Input};
use crate::view::{Camera, View};
use crate::Layout;
use sola_raylib::prelude::{RaylibHandle, RaylibThread, RenderTexture2D};

pub(super) struct DevHooks {
    /// The dev server; `None` under `--no-dev-server` or where its port
    /// could not be bound.
    #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
    server: Option<crate::devserver::DevServer>,
    /// What `bb_net_stats` hands the page: the online round's readings.
    #[cfg(feature = "dev-tools")]
    net_stats: crate::capi::NetStatsFeed,
    /// The frames drawn and the last press the window saw, which
    /// `bb_ui_json` reports.
    #[cfg(all(feature = "dev-tools", target_os = "emscripten"))]
    frames_drawn: u64,
    #[cfg(all(feature = "dev-tools", target_os = "emscripten"))]
    last_press: Option<crate::capi::Press>,
    /// A phone has no keyboard for the overlay cycle and its dev server is
    /// not reachable from the Mac, so the console is the one channel that
    /// says whether the frame budget holds on real hardware.
    #[cfg(all(feature = "dev-tools", any(target_os = "ios", target_os = "android")))]
    phone: FrameStats,
}

// A hook's arguments are read only where its tool is built.
#[allow(unused_variables, clippy::unused_self, clippy::too_many_arguments)]
impl DevHooks {
    /// Whether the dev tools are built: the overlay key is inert without
    /// them.
    pub const ENABLED: bool = cfg!(feature = "dev-tools");

    /// The hooks for this run: the dev server bound to `--dev-port`, else
    /// `BONGBONG_DEV_PORT`, else its default - failing to bind is a
    /// warning, not a fatal error.
    pub fn start(args: &Args) -> DevHooks {
        // Keep the C API (src/capi.rs) linked into this binary: an
        // `extern "C"` function nobody here references is fair game for
        // the linker to drop before emcc's EXPORTED_FUNCTIONS (build.rs)
        // can export it. See `capi::keep_alive`.
        #[cfg(feature = "dev-tools")]
        crate::capi::keep_alive();
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        let server = if args.no_dev_server {
            None
        } else {
            let port = args
                .dev_port
                .or_else(|| std::env::var("BONGBONG_DEV_PORT").ok()?.parse().ok())
                .unwrap_or(crate::devserver::DEFAULT_PORT);
            match crate::devserver::DevServer::start(port) {
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
        DevHooks {
            #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
            server,
            #[cfg(feature = "dev-tools")]
            net_stats: crate::capi::NetStatsFeed::default(),
            #[cfg(all(feature = "dev-tools", target_os = "emscripten"))]
            frames_drawn: 0,
            #[cfg(all(feature = "dev-tools", target_os = "emscripten"))]
            last_press: None,
            #[cfg(all(feature = "dev-tools", any(target_os = "ios", target_os = "android")))]
            phone: FrameStats::default(),
        }
    }

    /// The frame boundary, before anything is read: the dev server answers
    /// its queued requests (`DevServer::before_frame`), on the field of the
    /// live mode's map.
    pub fn before_frame(&mut self, session: &mut Session) {
        #[cfg(all(feature = "dev-tools", target_os = "emscripten"))]
        {
            self.frames_drawn += 1;
        }
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        if let Some(server) = &mut self.server {
            let (width, height) = session.field_size();
            server.before_frame(session, width, height);
        }
    }

    /// The view the dev server's `camera` tool pinned on a `field`, which
    /// outranks the map's.
    pub fn pinned_camera(&self, field: (f32, f32)) -> Option<Camera> {
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        return self.server.as_ref().and_then(|server| server.pinned_camera(field));
        #[cfg(not(all(feature = "dev-tools", not(target_os = "emscripten"))))]
        None
    }

    /// The input the dev server's `key`, `builder_touch` or `click {touch}`
    /// stood in for since the last frame, for the hints to follow.
    pub fn take_hints(&mut self) -> Option<Hints> {
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        return self.server.as_mut().and_then(|server| server.take_hints());
        #[cfg(not(all(feature = "dev-tools", not(target_os = "emscripten"))))]
        None
    }

    /// The window the chrome lays itself out in this frame.
    pub fn publish_ui(&mut self, ui: UiFrame) {
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        if let Some(server) = &mut self.server {
            server.publish_ui(ui);
        }
    }

    /// A press the window saw this frame, at `at` on the window, by a
    /// finger when `touch`.
    pub fn note_press(&mut self, at: Vec2, touch: bool) {
        #[cfg(all(feature = "dev-tools", target_os = "emscripten"))]
        {
            let count = self.last_press.map_or(1, |p| p.count + 1);
            self.last_press = Some(crate::capi::Press { count, at, touch });
        }
    }

    /// The frame's raw time, for a phone's console.
    pub fn sample_frame(&mut self, rl: &RaylibHandle, raw_dt: f32, fx: &crate::fx::Fx) {
        #[cfg(all(feature = "dev-tools", any(target_os = "ios", target_os = "android")))]
        self.phone.sample(raw_dt, rl.get_time(), rl.get_fps(), fx.live());
    }

    /// The builder's input with the fingers a dev server's
    /// `builder_touch {hold: true}` left down: they stay down across
    /// frames while the screen has none.
    pub fn hold_touches(&self, input: BuilderInput) -> BuilderInput {
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        if let Some(held) = self.server.as_ref().map(|server| server.held_touches()).filter(|held| !held.is_empty() && input.touches.is_empty()) {
            return BuilderInput { pointer: held.first().map(|t| t.pos), pressed: false, held: true, touches: held.to_vec(), ..input };
        }
        input
    }

    /// Whether the dev server holds the frames still (`step`), which
    /// freezes the round between its steps.
    pub fn lockstep(&self) -> bool {
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        return self.server.as_ref().is_some_and(|server| server.lockstep());
        #[cfg(not(all(feature = "dev-tools", not(target_os = "emscripten"))))]
        false
    }

    /// The round's input with what the dev server's `input` tool injected
    /// in the keyboard's place, for as many frames as it asked.
    pub fn shape_input(&mut self, input: Input) -> Input {
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        if let Some(server) = &mut self.server {
            return server.shape_input(input);
        }
        input
    }

    /// The intent a page's measurement script drives an online seat with
    /// this frame (`capi::bb_input`), if one holds.
    pub fn scripted_intent(&mut self) -> Option<Intent> {
        #[cfg(feature = "dev-tools")]
        return crate::capi::take_scripted_input();
        #[cfg(not(feature = "dev-tools"))]
        None
    }

    /// The online round's readings this frame, `None` in every other mode:
    /// the page reads a round's numbers only while one is played, and the
    /// frame the window leaves it they are taken down.
    pub fn net_stats(&mut self, stats: impl FnOnce() -> Option<String>) {
        #[cfg(feature = "dev-tools")]
        self.net_stats.frame(stats());
    }

    /// Run the local round's frame through the dev server
    /// (`DevServer::advance`): the `steps` the clock paid for, or a
    /// lockstep `step`'s own frames, or nothing while frozen. `false` where
    /// no dev server is attached and the loop runs the steps itself.
    pub fn advance(&mut self, game: &mut Game, input: Input, steps: u32, width: f32, height: f32, after_step: &mut dyn FnMut(&Game)) -> bool {
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        if let Some(server) = &mut self.server {
            server.advance(game, input, steps, width, height, after_step);
            return true;
        }
        false
    }

    /// The camera the frame draws through, as `report` composes it.
    pub fn publish_camera(&mut self, report: impl FnOnce() -> CameraReport) {
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        if let Some(server) = &mut self.server {
            server.publish_camera(report());
        }
    }

    /// After the frame is presented, the builder's too: a pending
    /// `screenshot` reads it from the screen here, or its client waits
    /// forever; and the page's scripts get the window as the frame laid it
    /// out (`capi::bb_ui_json`): the chrome in `ui`, the bitmap through
    /// `view`, and the last press.
    pub fn after_draw(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, scene: &RenderTexture2D, session: &Session, game: &Game, ui: &UiFrame, layout: &Layout, view: &View) {
        #[cfg(all(feature = "dev-tools", not(target_os = "emscripten")))]
        if let Some(server) = &mut self.server {
            server.after_render(rl, thread, scene, game);
        }
        #[cfg(all(feature = "dev-tools", target_os = "emscripten"))]
        crate::capi::publish_window(crate::capi::window_json(&crate::capi::WindowReport {
            session,
            ui,
            layout,
            view,
            window: (rl.get_screen_width(), rl.get_screen_height()),
            units_per_point: super::window_units_per_point(rl),
            frame: self.frames_drawn,
            press: self.last_press,
            page_overlay: super::web::overlay(),
        }));
    }
}

/// Frame-time sampling for the console on a phone: a log line every few
/// seconds.
#[cfg(all(feature = "dev-tools", any(target_os = "ios", target_os = "android")))]
#[derive(Default)]
struct FrameStats {
    sum: f32,
    max: f32,
    frames: u32,
    last_report: f64,
}

#[cfg(all(feature = "dev-tools", any(target_os = "ios", target_os = "android")))]
impl FrameStats {
    const REPORT_EVERY_SECONDS: f64 = 5.0;

    fn sample(&mut self, dt: f32, now: f64, fps: u32, live_fx: usize) {
        self.sum += dt;
        self.max = self.max.max(dt);
        self.frames += 1;
        if now - self.last_report >= Self::REPORT_EVERY_SECONDS && self.frames > 0 {
            eprintln!(
                "bongbong: frame avg {:.2} ms, max {:.2} ms, {} fps, {} fx over {} frames; {}; {}",
                self.sum / self.frames as f32 * 1000.0,
                self.max * 1000.0,
                fps,
                live_fx,
                self.frames,
                crate::frame_stages::line(),
                crate::frame_stages::draws_line()
            );
            *self = Self { last_report: now, ..Self::default() };
        }
    }
}
