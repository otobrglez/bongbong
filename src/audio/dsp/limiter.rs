//! The master's lookahead limiter: the mix is delayed a few milliseconds so
//! the gain can come down before a peak arrives, and it never lets a sample
//! past the ceiling. Assets are loudness-normalised (docs/sound-system-prd.md
//! section 4.12), so it seldom works; it is there for the pile-ups.

use std::collections::VecDeque;

pub struct Limiter {
    /// The delayed stereo frames, as a ring.
    delay: Vec<[f32; 2]>,
    pos: usize,
    /// The window's peaks, oldest first, each with the index it entered at:
    /// a monotonic queue, so the window's maximum is its front.
    peaks: VecDeque<(u64, f32)>,
    clock: u64,
    gain: f32,
    ceiling: f32,
    attack: f32,
    release: f32,
}

impl Limiter {
    /// A limiter at `rate` with `ceiling` as the highest sample it passes.
    pub fn new(rate: u32, ceiling: f32) -> Limiter {
        let look = (rate as f32 * 0.003) as usize + 1;
        Limiter {
            delay: vec![[0.0; 2]; look],
            pos: 0,
            peaks: VecDeque::with_capacity(look + 1),
            clock: 0,
            gain: 1.0,
            ceiling,
            attack: 1.0 - libm::expf(-1.0 / (look as f32 * 0.5)),
            release: 1.0 - libm::expf(-1.0 / (rate as f32 * 0.12)),
        }
    }

    /// Limits one stereo frame in place.
    #[inline]
    pub fn run(&mut self, frame: &mut [f32; 2]) {
        let look = self.delay.len() as u64;
        let peak = frame[0].abs().max(frame[1].abs());
        while self.peaks.back().is_some_and(|&(_, p)| p <= peak) {
            self.peaks.pop_back();
        }
        self.peaks.push_back((self.clock, peak));
        while self.peaks.front().is_some_and(|&(at, _)| at + look <= self.clock) {
            self.peaks.pop_front();
        }
        self.clock += 1;
        let window = self.peaks.front().map_or(0.0, |&(_, p)| p);
        let target = if window > self.ceiling { self.ceiling / window } else { 1.0 };
        let rate = if target < self.gain { self.attack } else { self.release };
        self.gain += (target - self.gain) * rate;
        let out = std::mem::replace(&mut self.delay[self.pos], *frame);
        self.pos = (self.pos + 1) % self.delay.len();
        let g = self.gain;
        frame[0] = (out[0] * g).clamp(-self.ceiling, self.ceiling);
        frame[1] = (out[1] * g).clamp(-self.ceiling, self.ceiling);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_passes_the_ceiling() {
        let mut l = Limiter::new(48_000, 0.9);
        let mut worst = 0.0f32;
        for i in 0..48_000 {
            let v = if (i / 100) % 2 == 0 { 3.0 } else { -2.5 };
            let mut f = [v, -v];
            l.run(&mut f);
            worst = worst.max(f[0].abs()).max(f[1].abs());
        }
        assert!(worst <= 0.9 + 1e-6, "peak {worst}");
    }

    #[test]
    fn a_quiet_signal_goes_through_untouched_but_late() {
        let mut l = Limiter::new(48_000, 0.9);
        let input: Vec<f32> = (0..2000).map(|i| 0.3 * libm::sinf(i as f32 * 0.05)).collect();
        let mut out = Vec::new();
        for &v in &input {
            let mut f = [v, v];
            l.run(&mut f);
            out.push(f[0]);
        }
        let late = l.delay.len();
        for i in 0..input.len() - late {
            assert!((out[i + late] - input[i]).abs() < 1e-6);
        }
    }
}
