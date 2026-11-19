//! Where a frame's time goes, stage by stage, in a build with the dev
//! tools: the window times its steps, the particle layer, the pictures it
//! keeps and uploads, the lights, the world, the chrome and the swap, and
//! keeps each as a running average over about half a second. Read by the
//! dev server's `status.frame_stages`, the line under the left cluster
//! (`ui_frame_stages`, which a PR preview's tuning panel turns on, on a
//! phone too) and a phone's log. It measures what the processor spends
//! issuing each stage, not what the GPU spends drawing it, which lands in
//! the swap.
//!
//! Beside the stages it keeps three more readings of the same frames:
//! what the frame drew ([`Draws`], counted where `render::canvas::GpuCanvas`
//! and `render::pyro::Rl` forward to raylib and where a blend, shader or
//! render-target scope opens), how the frames were paced over the last
//! [`WINDOW`] of them ([`Pace`]: the worst frame, the 99th percentile and
//! how many simulation steps each frame ran), and each stage's total since
//! the thread started ([`totals`], which the probe prints per tick).
//! `Game::update` times its own heavy phases as `sim.*` stages.
//!
//! Without the dev tools every call is empty and every reading is zero.

/// How many frames [`Pace`] looks back over: ten seconds at 60 a second.
pub const WINDOW: usize = 600;

/// How many step counts [`Pace::steps`] tells apart: none up to the play
/// clock's cap (`SIM_MAX_STEPS_PER_FRAME`).
pub const STEP_BINS: usize = crate::SIM_MAX_STEPS_PER_FRAME as usize + 1;

/// What one frame drew through `GpuCanvas` and `Rl`, and the state scopes
/// it opened. Chrome that calls raylib directly (the HUD, the builder's
/// bars) is not in the first four.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Draws {
    /// Sprites: `Canvas::blit`, `blocks` and `blocks_part`.
    pub blits: u32,
    /// Rectangles and gradients, which raylib draws with its shapes
    /// texture.
    pub rects: u32,
    /// Discs and rings, on the shapes texture too.
    pub shapes: u32,
    /// How often the texture of one draw differed from the draw before it
    /// (the frame's first draw counts): the least number of draw calls the
    /// counted draws split into, before any scope below splits them
    /// further.
    pub texture_switches: u32,
    /// Blend-mode scopes opened; each flushes the batch on the way in and
    /// on the way out.
    pub blend_scopes: u32,
    /// Shader scopes opened, two flushes each.
    pub shader_scopes: u32,
    /// Render-target scopes opened, two flushes each.
    pub target_scopes: u32,
}

impl Draws {
    /// The counters by name, in the order every reader lists them.
    pub fn named(&self) -> [(&'static str, u32); 7] {
        [
            ("blits", self.blits),
            ("rects", self.rects),
            ("shapes", self.shapes),
            ("texture_switches", self.texture_switches),
            ("blend_scopes", self.blend_scopes),
            ("shader_scopes", self.shader_scopes),
            ("target_scopes", self.target_scopes),
        ]
    }
}

/// How the last [`WINDOW`] frames were paced.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pace {
    /// Frames in the window so far, at most [`WINDOW`].
    pub frames: usize,
    /// The longest frame, ms.
    pub max_ms: f32,
    /// The frame time 99 in 100 frames stayed at or under, ms (nearest
    /// rank).
    pub p99_ms: f32,
    /// How many of the window's frames ran 0, 1, .. simulation steps. Only
    /// a frame the play clock paid out for is counted: not a dialog's, the
    /// builder's or a dev-server lockstep's.
    pub steps: [u32; STEP_BINS],
}

/// One stage being timed, from `stage` until it is dropped.
pub struct Stage {
    #[cfg(feature = "dev-tools")]
    name: &'static str,
    #[cfg(feature = "dev-tools")]
    start: std::time::Instant,
}

/// Time a stage of this frame until the returned guard is dropped. A stage
/// timed twice in a frame counts both.
#[must_use]
pub fn stage(_name: &'static str) -> Stage {
    Stage {
        #[cfg(feature = "dev-tools")]
        name: _name,
        #[cfg(feature = "dev-tools")]
        start: std::time::Instant::now(),
    }
}

#[cfg(feature = "dev-tools")]
mod kept {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// How much of a frame's reading goes into the average: about half a
    /// second of frames at 60 a second.
    pub const SHARE: f32 = 1.0 / 30.0;

    #[derive(Default)]
    pub struct Stages {
        /// This frame's stages so far, ms.
        pub now: Vec<(&'static str, f32)>,
        /// The running averages, ms a frame, in the order first timed;
        /// the whole frame first.
        pub average: Vec<(&'static str, f32)>,
        /// Every stage's time since the thread started, ms, in the order
        /// first timed.
        pub total: Vec<(&'static str, f64)>,
        /// What this frame has drawn so far.
        pub drawing: super::Draws,
        /// The texture of this frame's latest draw, 0 for raylib's shapes
        /// texture (no loaded texture has that id).
        pub bound: Option<u32>,
        /// What the last whole frame drew.
        pub drew: super::Draws,
        /// The steps this frame's play clock paid out, if it ran.
        pub stepping: Option<u8>,
        /// The last `WINDOW` frames, oldest first: ms and steps.
        pub window: VecDeque<(f32, Option<u8>)>,
    }

    impl Stages {
        /// Note one draw on `texture`.
        pub fn draw(&mut self, texture: u32) {
            if self.bound != Some(texture) {
                self.bound = Some(texture);
                self.drawing.texture_switches += 1;
            }
        }
    }

    thread_local! {
        pub static STAGES: RefCell<Stages> = RefCell::new(Stages::default());
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        #[cfg(feature = "dev-tools")]
        {
            let ms = self.start.elapsed().as_secs_f32() * 1000.0;
            kept::STAGES.with(|s| {
                let mut s = s.borrow_mut();
                match s.now.iter_mut().find(|(name, _)| *name == self.name) {
                    Some((_, t)) => *t += ms,
                    None => s.now.push((self.name, ms)),
                }
                match s.total.iter_mut().find(|(name, _)| *name == self.name) {
                    Some((_, t)) => *t += ms as f64,
                    None => s.total.push((self.name, ms as f64)),
                }
            });
        }
    }
}

/// Count a sprite drawn from the texture with raylib's id `_texture`.
#[inline]
pub fn blit(_texture: u32) {
    #[cfg(feature = "dev-tools")]
    kept::STAGES.with(|s| {
        let mut s = s.borrow_mut();
        s.drawing.blits += 1;
        s.draw(_texture);
    });
}

/// Count a rectangle or a gradient.
#[inline]
pub fn rect() {
    #[cfg(feature = "dev-tools")]
    kept::STAGES.with(|s| {
        let mut s = s.borrow_mut();
        s.drawing.rects += 1;
        s.draw(0);
    });
}

/// Count a disc or a ring.
#[inline]
pub fn shape() {
    #[cfg(feature = "dev-tools")]
    kept::STAGES.with(|s| {
        let mut s = s.borrow_mut();
        s.drawing.shapes += 1;
        s.draw(0);
    });
}

/// Count a blend-mode scope about to open and hand its mode back, so the
/// count sits in the call that opens it: `d.draw_blend_mode(blend(mode), ..)`.
#[inline]
pub fn blend<T>(mode: T) -> T {
    #[cfg(feature = "dev-tools")]
    kept::STAGES.with(|s| s.borrow_mut().drawing.blend_scopes += 1);
    mode
}

/// Count a shader scope about to open and hand its shader back
/// (`d.draw_shader_mode(shader(s), ..)`).
#[inline]
pub fn shader<T>(shader: T) -> T {
    #[cfg(feature = "dev-tools")]
    kept::STAGES.with(|s| s.borrow_mut().drawing.shader_scopes += 1);
    shader
}

/// Count a render-target scope about to open and hand its target back
/// (`rl.draw_texture_mode(thread, target(t), ..)`).
#[inline]
pub fn target<T>(target: T) -> T {
    #[cfg(feature = "dev-tools")]
    kept::STAGES.with(|s| s.borrow_mut().drawing.target_scopes += 1);
    target
}

/// Note the simulation steps this frame's play clock paid out.
#[inline]
pub fn steps(_steps: u32) {
    #[cfg(feature = "dev-tools")]
    kept::STAGES.with(|s| s.borrow_mut().stepping = Some(_steps.min(STEP_BINS as u32 - 1) as u8));
}

/// Close a frame that took `_frame_ms` in all: fold its stages into the
/// averages, a stage it did not run counting as nothing, keep what it
/// drew and add it to the window.
pub fn frame_done(_frame_ms: f32) {
    #[cfg(feature = "dev-tools")]
    kept::STAGES.with(|s| {
        let mut s = s.borrow_mut();
        s.drew = std::mem::take(&mut s.drawing);
        s.bound = None;
        let stepped = s.stepping.take();
        if s.window.len() == WINDOW {
            s.window.pop_front();
        }
        s.window.push_back((_frame_ms, stepped));
        let mut now = std::mem::take(&mut s.now);
        now.insert(0, ("frame", _frame_ms));
        for (name, avg) in s.average.iter_mut() {
            let reading = now.iter().find(|(n, _)| n == name).map_or(0.0, |(_, t)| *t);
            *avg += (reading - *avg) * kept::SHARE;
        }
        for (name, ms) in now {
            if !s.average.iter().any(|(n, _)| *n == name) {
                s.average.push((name, ms));
            }
        }
    });
}

/// The running averages: each stage and its ms a frame, the whole frame
/// first. Empty without the dev tools.
pub fn averages() -> Vec<(&'static str, f32)> {
    #[cfg(feature = "dev-tools")]
    {
        kept::STAGES.with(|s| s.borrow().average.clone())
    }
    #[cfg(not(feature = "dev-tools"))]
    {
        Vec::new()
    }
}

/// Every stage's total since the thread started, ms, in the order first
/// timed. Empty without the dev tools.
pub fn totals() -> Vec<(&'static str, f64)> {
    #[cfg(feature = "dev-tools")]
    {
        kept::STAGES.with(|s| s.borrow().total.clone())
    }
    #[cfg(not(feature = "dev-tools"))]
    {
        Vec::new()
    }
}

/// What the last whole frame drew. Zero without the dev tools.
pub fn draws() -> Draws {
    #[cfg(feature = "dev-tools")]
    {
        kept::STAGES.with(|s| s.borrow().drew)
    }
    #[cfg(not(feature = "dev-tools"))]
    {
        Draws::default()
    }
}

/// How the last [`WINDOW`] frames were paced. Zero without the dev tools.
pub fn pace() -> Pace {
    #[cfg(feature = "dev-tools")]
    {
        kept::STAGES.with(|s| {
            let s = s.borrow();
            let mut ms: Vec<f32> = s.window.iter().map(|(ms, _)| *ms).collect();
            ms.sort_by(f32::total_cmp);
            let mut steps = [0; STEP_BINS];
            for n in s.window.iter().filter_map(|(_, n)| *n) {
                steps[n as usize] += 1;
            }
            let rank = (ms.len() * 99).div_ceil(100).max(1) - 1;
            Pace { frames: ms.len(), max_ms: ms.last().copied().unwrap_or(0.0), p99_ms: ms.get(rank).copied().unwrap_or(0.0), steps }
        })
    }
    #[cfg(not(feature = "dev-tools"))]
    {
        Pace::default()
    }
}

/// The last frame's draws and the window's pace as one line:
/// `draws 412 blits 96 rects 3 shapes 38 tex | scopes 12 blend 2 shader 9 target | p99 17.1 max 33.4 ms | steps 0/598/2/0/0`.
pub fn draws_line() -> String {
    let (d, p) = (draws(), pace());
    let steps = p.steps.iter().map(u32::to_string).collect::<Vec<_>>().join("/");
    format!(
        "draws {} blits {} rects {} shapes {} tex | scopes {} blend {} shader {} target | p99 {:.1} max {:.1} ms | steps {steps}",
        d.blits, d.rects, d.shapes, d.texture_switches, d.blend_scopes, d.shader_scopes, d.target_scopes, p.p99_ms, p.max_ms
    )
}

/// The averages as one line: `frame 9.1 ms | sim 0.1 lights 0.4 ...`. A
/// stage inside another (`sim.nav`) is left to `averages`.
pub fn line() -> String {
    let all: Vec<_> = averages().into_iter().filter(|(name, _)| !name.contains('.')).collect();
    let mut out = String::new();
    for (i, (name, ms)) in all.iter().enumerate() {
        if i == 0 {
            out += &format!("{name} {ms:.1} ms |");
        } else {
            out += &format!(" {name} {ms:.1}");
        }
    }
    out
}

#[cfg(all(test, feature = "dev-tools"))]
mod tests {
    use super::*;

    #[test]
    fn a_frames_draws_are_kept_when_it_closes() {
        blit(7);
        blit(7);
        rect();
        shape();
        blit(9);
        let _ = blend(1);
        let _ = shader(2);
        let _ = target(3);
        let _ = target(4);
        assert_eq!(draws(), Draws::default(), "the frame is still open");
        frame_done(16.0);
        let d = draws();
        assert_eq!((d.blits, d.rects, d.shapes), (3, 1, 1));
        // 7, the shapes texture (the rectangle and the disc share it), 9.
        assert_eq!(d.texture_switches, 3);
        assert_eq!((d.blend_scopes, d.shader_scopes, d.target_scopes), (1, 1, 2));
        // The next frame starts from nothing, its first draw a switch.
        blit(9);
        frame_done(16.0);
        assert_eq!(draws(), Draws { blits: 1, texture_switches: 1, ..Draws::default() });
    }

    #[test]
    fn the_window_keeps_the_worst_frame_the_99th_and_the_steps() {
        assert_eq!(pace(), Pace::default());
        for i in 0..200 {
            steps(if i % 2 == 0 { 1 } else { 0 });
            frame_done(if i == 50 {
                90.0
            } else if i < 4 {
                30.0
            } else {
                16.0
            });
        }
        // A frame the clock did not run for.
        frame_done(16.0);
        let p = pace();
        assert_eq!(p.frames, 201);
        assert_eq!(p.max_ms, 90.0);
        // 201 frames: the 199th in order, past all but the one of 90 and
        // one of the four of 30.
        assert_eq!(p.p99_ms, 30.0);
        assert_eq!(p.steps, [100, 100, 0, 0, 0]);
        // More steps than the clock's cap land in the last bin.
        steps(9);
        frame_done(16.0);
        assert_eq!(pace().steps[STEP_BINS - 1], 1);
        // The window forgets its oldest frames.
        for _ in 0..WINDOW {
            frame_done(8.0);
        }
        let p = pace();
        assert_eq!((p.frames, p.max_ms, p.p99_ms), (WINDOW, 8.0, 8.0));
        assert_eq!(p.steps, [0; STEP_BINS]);
    }

    #[test]
    fn a_stage_adds_to_its_total_and_a_nested_one_stays_off_the_line() {
        drop(stage("sim"));
        drop(stage("sim.nav"));
        drop(stage("sim.nav"));
        frame_done(16.0);
        let names: Vec<_> = totals().into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["sim", "sim.nav"]);
        assert!(line().contains("sim") && !line().contains("sim.nav"));
        assert!(draws_line().starts_with("draws 0 blits"));
    }
}
