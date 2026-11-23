//! The room (docs/sound-system-prd.md section 4.3): a small feedback delay
//! network every placed voice sends into, the send rising with distance so
//! a far sound is heard more by its room than by itself.
//!
//! Four delay lines, 32 to 52 ms at 48 kHz (scaled to the engine's rate),
//! mixed back into one another through a Householder matrix - every line
//! feeds every other, so the tail is dense - each with a one-pole high-cut
//! in its loop, so the room darkens as it decays, and a gain that makes the
//! tail fall 60 dB in `seconds`. Pure `f32` arithmetic through `libm`, like
//! the rest of the engine: the same sends render the same tail everywhere.

/// The lines' lengths at 48 kHz, mutually prime so their echoes never line
/// up into a flutter.
const LENGTHS_48K: [usize; 4] = [1559, 1877, 2129, 2477];

/// What the room is set to: how long its tail rings (to -60 dB), where its
/// high-cut sits, and the level it returns at, linear.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    pub seconds: f32,
    pub damp_hz: f32,
    pub gain: f32,
}

impl Room {
    /// No room: nothing returns.
    pub const DRY: Room = Room { seconds: 0.5, damp_hz: 4000.0, gain: 0.0 };
}

pub struct Reverb {
    rate: f32,
    lines: [Vec<f32>; 4],
    at: [usize; 4],
    feedback: [f32; 4],
    damp: f32,
    lows: [f32; 4],
    gain: f32,
    gain_target: f32,
}

impl Reverb {
    pub fn new(rate: u32) -> Reverb {
        let scale = rate as f32 / 48_000.0;
        let lines = LENGTHS_48K.map(|n| vec![0.0f32; ((n as f32 * scale) as usize).max(1)]);
        let mut reverb = Reverb { rate: rate as f32, lines, at: [0; 4], feedback: [0.0; 4], damp: 1.0, lows: [0.0; 4], gain: 0.0, gain_target: 0.0 };
        reverb.set(Room::DRY);
        reverb
    }

    pub fn set(&mut self, room: Room) {
        let seconds = room.seconds.max(0.05);
        for (i, line) in self.lines.iter().enumerate() {
            // Each pass round a line of `n` frames loses its share of 60 dB.
            self.feedback[i] = libm::powf(10.0, -3.0 * line.len() as f32 / (self.rate * seconds));
        }
        let hz = room.damp_hz.clamp(100.0, self.rate * 0.45);
        self.damp = 1.0 - libm::expf(-2.0 * std::f32::consts::PI * hz / self.rate);
        self.gain_target = room.gain.max(0.0);
    }

    /// Whether anything is ringing or about to: a room at no level with
    /// nothing sent costs nothing.
    pub fn idle(&self) -> bool {
        self.gain == 0.0 && self.gain_target == 0.0
    }

    /// Runs `input` (the voices' sends, stereo) through the room and adds
    /// what returns to `out`; `glide` moves the return level as the
    /// engine's parameters move.
    pub fn run(&mut self, input: &[[f32; 2]], out: &mut [[f32; 2]], glide: f32) {
        let g0 = self.gain;
        self.gain += (self.gain_target - self.gain) * glide;
        if self.gain.abs() < 1e-6 && self.gain_target == 0.0 {
            self.gain = 0.0;
        }
        let g1 = self.gain;
        let inv_n = 1.0 / input.len().max(1) as f32;
        for (i, (x, y)) in input.iter().zip(out.iter_mut()).enumerate() {
            let mono = 0.5 * (x[0] + x[1]);
            let taps: [f32; 4] = std::array::from_fn(|k| self.lines[k][self.at[k]]);
            // Householder: every line hears every other, energy kept.
            let half = 0.5 * (taps[0] + taps[1] + taps[2] + taps[3]);
            for (k, tap) in taps.iter().enumerate() {
                let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
                let back = (tap - half) * self.feedback[k] + sign * mono;
                // The loop's high-cut, flushed near zero so a dying tail
                // never sinks into denormals.
                let mut low = self.lows[k] + (back - self.lows[k]) * self.damp;
                if low.abs() < 1e-18 {
                    low = 0.0;
                }
                self.lows[k] = low;
                self.lines[k][self.at[k]] = low;
                self.at[k] = (self.at[k] + 1) % self.lines[k].len();
            }
            let g = g0 + (g1 - g0) * i as f32 * inv_n;
            y[0] += (taps[0] - taps[2]) * 0.5 * g;
            y[1] += (taps[1] - taps[3]) * 0.5 * g;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn energy(frames: &[[f32; 2]]) -> f32 {
        frames.iter().map(|f| f[0] * f[0] + f[1] * f[1]).sum()
    }

    /// A click into the room comes back as a tail that dies away, longer
    /// in a longer room, and a dry room returns nothing.
    #[test]
    fn a_click_rings_and_dies_away() {
        let tail = |seconds: f32| {
            let mut r = Reverb::new(48_000);
            r.set(Room { seconds, damp_hz: 6000.0, gain: 1.0 });
            r.gain = 1.0;
            let mut input = vec![[0.0f32; 2]; 48_000];
            input[0] = [1.0, 1.0];
            let mut out = vec![[0.0f32; 2]; 48_000];
            r.run(&input, &mut out, 1.0);
            out
        };
        let short = tail(0.5);
        let long = tail(1.5);
        let (early, late) = (energy(&short[..12_000]), energy(&short[36_000..]));
        assert!(early > 0.0 && late < early * 1e-4, "the tail dies: {early} then {late}");
        assert!(energy(&long[24_000..]) > energy(&short[24_000..]), "a longer room rings longer");
        assert!(short.iter().all(|f| f[0].is_finite() && f[1].is_finite()));

        let mut dry = Reverb::new(48_000);
        let mut out = vec![[0.0f32; 2]; 4800];
        let mut input = vec![[0.0f32; 2]; 4800];
        input[0] = [1.0, 1.0];
        dry.run(&input, &mut out, 1.0);
        assert_eq!(energy(&out), 0.0);
        assert!(dry.idle());
    }
}
