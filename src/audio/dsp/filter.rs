//! The per-voice high-cut: a state-variable low-pass in the trapezoidal form
//! (Simper's), which stays stable while its cutoff moves - occlusion and
//! distance move it every block.

use std::f32::consts::PI;

/// The cutoff above which a voice skips its filter: nothing a speaker plays
/// is cut there.
pub const OPEN_HZ: f32 = 18_000.0;

/// One channel's low-pass.
#[derive(Clone, Copy, Debug, Default)]
pub struct LowPass {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
}

impl LowPass {
    /// Sets the cutoff (Hz) at `rate` frames a second, Butterworth Q.
    pub fn tune(&mut self, cutoff: f32, rate: f32) {
        let fc = cutoff.clamp(20.0, rate * 0.45);
        let g = libm::tanf(PI * fc / rate);
        let k = std::f32::consts::SQRT_2;
        self.a1 = 1.0 / (1.0 + g * (g + k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    #[inline]
    pub fn run(&mut self, v0: f32) -> f32 {
        let v3 = v0 - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        v2
    }

    /// Forgets what it was filtering, for a voice that is reused.
    pub fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, rate: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| libm::sinf(2.0 * PI * freq * i as f32 / rate)).collect()
    }

    fn level(signal: &[f32]) -> f32 {
        signal.iter().skip(signal.len() / 2).fold(0.0f32, |m, v| m.max(v.abs()))
    }

    #[test]
    fn a_low_pass_keeps_the_lows_and_cuts_the_highs() {
        let rate = 48_000.0;
        let mut f = LowPass::default();
        f.tune(1000.0, rate);
        let low: Vec<f32> = tone(100.0, rate, 4800).into_iter().map(|v| f.run(v)).collect();
        f.reset();
        let high: Vec<f32> = tone(10_000.0, rate, 4800).into_iter().map(|v| f.run(v)).collect();
        assert!(level(&low) > 0.9, "100 Hz passes a 1 kHz low-pass: {}", level(&low));
        assert!(level(&high) < 0.05, "10 kHz is cut by a 1 kHz low-pass: {}", level(&high));
    }
}
