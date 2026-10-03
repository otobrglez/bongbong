//! Where a frame's time goes, stage by stage, in a build with the dev
//! tools: the window times its steps, the particle layer, the pictures it
//! keeps and uploads, the lights, the world, the chrome and the swap, and
//! keeps each as a running average over about half a second. Read by the
//! dev server's `status.frame`, the line under the left cluster
//! (`ui_frame_stages`, which a PR preview's tuning panel turns on, on a
//! phone too) and a phone's log. It measures what the processor spends
//! issuing each stage, not what the GPU spends drawing it, which lands in
//! the swap. Without the dev tools every call is empty.

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
            });
        }
    }
}

/// Close a frame that took `_frame_ms` in all: fold its stages into the
/// averages, a stage it did not run counting as nothing.
pub fn frame_done(_frame_ms: f32) {
    #[cfg(feature = "dev-tools")]
    kept::STAGES.with(|s| {
        let mut s = s.borrow_mut();
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

/// The averages as one line: `frame 9.1 ms | sim 0.1 lights 0.4 ...`.
pub fn line() -> String {
    let all = averages();
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
