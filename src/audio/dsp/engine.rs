//! The mixer: voices that play samples, buses they play into, and the master
//! (docs/sound-system-prd.md section 4.1, D1).
//!
//! A voice reads its sample at a rate (its pitch times the sample's rate over
//! the engine's), through a per-channel high-cut, into a stereo pair at its
//! gain and pan, after an optional delay; a loop wraps, a one-shot ends. Every
//! parameter moves smoothly - a change set by a command glides over about ten
//! milliseconds - so a distance, an occlusion or an engine's revs never click.
//! Buses carry a gain and a duck. Each voice also sends into the room
//! (`reverb.rs`), at its own share; the room returns into the mix ahead of
//! the master, which carries the player's volume, a mono switch, the
//! watchdog (D5) and a lookahead limiter.

use std::sync::Arc;

use super::Pcm;
use super::filter::{LowPass, OPEN_HZ};
use super::limiter::Limiter;
use super::reverb::{Reverb, Room};

/// A voice's name, chosen by the director, unique while the voice lives.
pub type VoiceId = u32;

/// How many buses the engine mixes (`audio::Bus`).
pub const BUSES: usize = 9;

/// Frames mixed between parameter updates.
const BLOCK: usize = 64;

/// What a voice is set to. `gain` is linear; `pan` runs -1 (left) to 1
/// (right); `pitch` multiplies the playback rate; `cutoff` is the high-cut
/// in Hz; `send` is the share of what it plays that goes to the room.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceParams {
    pub gain: f32,
    pub pan: f32,
    pub pitch: f32,
    pub cutoff: f32,
    pub send: f32,
}

impl VoiceParams {
    pub const NEUTRAL: VoiceParams = VoiceParams { gain: 1.0, pan: 0.0, pitch: 1.0, cutoff: 20_000.0, send: 0.0 };
}

/// What the director asks of the engine.
#[derive(Clone, Debug)]
pub enum Command {
    /// Starts a voice: `delay` seconds from now, `offset` seconds into the
    /// sample, fading in over `fade_in` seconds.
    Start { id: VoiceId, pcm: Arc<Pcm>, bus: usize, looped: bool, params: VoiceParams, delay: f32, offset: f32, fade_in: f32 },
    /// Moves a voice's parameters; they glide there.
    Update { id: VoiceId, params: VoiceParams },
    /// Fades a voice out over `fade` seconds, then drops it.
    Stop { id: VoiceId, fade: f32 },
    /// Holds every voice on a bus where it stands, or lets it go on.
    Pause { bus: usize, paused: bool },
    /// A bus's level, linear.
    Bus { bus: usize, gain: f32 },
    /// Drops a bus to `gain` at once and lets it back up over `release`
    /// seconds; the deepest duck in force wins.
    Duck { bus: usize, gain: f32, release: f32 },
    /// The whole mix's level, linear, and whether it plays in mono.
    Master { gain: f32, mono: bool },
    /// The room every voice sends into.
    Room(Room),
    /// The director is alive (D5): without one for a quarter second the
    /// engine fades itself out.
    Beat,
}

struct Voice {
    id: VoiceId,
    pcm: Arc<Pcm>,
    bus: usize,
    looped: bool,
    /// Where it reads, in the sample's frames.
    pos: f64,
    /// The rate it reads at, in the sample's frames per engine frame.
    step: f64,
    /// Frames left before it sounds.
    delay: u32,
    target: VoiceParams,
    gain_l: f32,
    gain_r: f32,
    cutoff: f32,
    send: f32,
    filters: [LowPass; 2],
    /// The fade envelope 0..1 and how far it moves a frame.
    fade: f32,
    fade_step: f32,
    stopping: bool,
    done: bool,
}

struct BusState {
    gain: f32,
    target: f32,
    duck: f32,
    duck_release: f32,
    paused: bool,
}

/// What the engine reports to the dev tools.
#[derive(Clone, Copy, Debug, Default)]
pub struct EngineStats {
    pub voices: usize,
    pub peak: f32,
}

pub struct Engine {
    rate: u32,
    voices: Vec<Voice>,
    buses: [BusState; BUSES],
    master: f32,
    master_target: f32,
    mono: bool,
    watchdog: f32,
    since_beat: u32,
    limiter: Limiter,
    block: Vec<[f32; 2]>,
    /// What the voices send to the room this block.
    sends: Vec<[f32; 2]>,
    reverb: Reverb,
    peak: f32,
}

impl Engine {
    pub fn new(rate: u32) -> Engine {
        Engine {
            rate,
            voices: Vec::with_capacity(128),
            buses: std::array::from_fn(|_| BusState { gain: 1.0, target: 1.0, duck: 1.0, duck_release: 0.3, paused: false }),
            master: 1.0,
            master_target: 1.0,
            mono: false,
            watchdog: 1.0,
            since_beat: 0,
            limiter: Limiter::new(rate, super::db_to_gain(-1.0)),
            block: vec![[0.0; 2]; BLOCK],
            sends: vec![[0.0; 2]; BLOCK],
            reverb: Reverb::new(rate),
            peak: 0.0,
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn stats(&self) -> EngineStats {
        EngineStats { voices: self.voices.len(), peak: self.peak }
    }

    pub fn apply(&mut self, command: Command) {
        match command {
            Command::Start { id, pcm, bus, looped, params, delay, offset, fade_in } => {
                self.voices.retain(|v| v.id != id);
                let (l, r) = pan_gains(params.pan, params.gain);
                let fade_frames = (fade_in * self.rate as f32).max(1.0);
                let frames = pcm.frames().max(1) as f64;
                let start = offset as f64 * pcm.rate as f64;
                let pos = if looped { start.rem_euclid(frames) } else { start.min(frames) };
                let mut filters = [LowPass::default(); 2];
                for f in &mut filters {
                    f.tune(params.cutoff, self.rate as f32);
                }
                self.voices.push(Voice {
                    id,
                    step: step_of(&pcm, params.pitch, self.rate),
                    pcm,
                    bus: bus.min(BUSES - 1),
                    looped,
                    pos,
                    delay: (delay.max(0.0) * self.rate as f32) as u32,
                    target: params,
                    gain_l: l,
                    gain_r: r,
                    cutoff: params.cutoff,
                    send: params.send,
                    filters,
                    fade: if fade_in > 0.0 { 0.0 } else { 1.0 },
                    fade_step: 1.0 / fade_frames,
                    stopping: false,
                    done: false,
                });
            }
            Command::Update { id, params } => {
                if let Some(v) = self.voices.iter_mut().find(|v| v.id == id) {
                    v.target = params;
                }
            }
            Command::Stop { id, fade } => {
                if let Some(v) = self.voices.iter_mut().find(|v| v.id == id) {
                    v.stopping = true;
                    v.fade_step = 1.0 / (fade * self.rate as f32).max(1.0);
                }
            }
            Command::Pause { bus, paused } => {
                if let Some(b) = self.buses.get_mut(bus) {
                    b.paused = paused;
                }
            }
            Command::Bus { bus, gain } => {
                if let Some(b) = self.buses.get_mut(bus) {
                    b.target = gain.max(0.0);
                }
            }
            Command::Duck { bus, gain, release } => {
                if let Some(b) = self.buses.get_mut(bus)
                    && gain < b.duck
                {
                    b.duck = gain.max(0.0);
                    b.duck_release = release.max(0.01);
                }
            }
            Command::Master { gain, mono } => {
                self.master_target = gain.max(0.0);
                self.mono = mono;
            }
            Command::Room(room) => self.reverb.set(room),
            Command::Beat => self.since_beat = 0,
        }
    }

    /// Mixes `out.len() / 2` interleaved stereo frames.
    pub fn render(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let mut done = 0;
        let mut peak = 0.0f32;
        while done < frames {
            let n = (frames - done).min(BLOCK);
            self.render_block(n);
            for i in 0..n {
                let [l, r] = self.block[i];
                out[2 * (done + i)] = l;
                out[2 * (done + i) + 1] = r;
                peak = peak.max(l.abs()).max(r.abs());
            }
            done += n;
        }
        self.peak = peak;
        self.voices.retain(|v| !v.done);
    }

    fn render_block(&mut self, n: usize) {
        let rate = self.rate as f32;
        // Parameters glide with a 10 ms time constant, step by block.
        let glide = 1.0 - libm::expf(-(n as f32) / (rate * 0.010));
        for f in &mut self.block[..n] {
            *f = [0.0, 0.0];
        }
        for f in &mut self.sends[..n] {
            *f = [0.0, 0.0];
        }
        // The watchdog (D5): no beat for a quarter second and the mix fades.
        self.since_beat = self.since_beat.saturating_add(n as u32);
        let alive = if self.since_beat > self.rate / 4 { 0.0 } else { 1.0 };
        self.watchdog += (alive - self.watchdog) * (1.0 - libm::expf(-(n as f32) / (rate * 0.1)));
        let mut bus_from = [0.0f32; BUSES];
        let mut bus_to = [0.0f32; BUSES];
        for (i, b) in self.buses.iter_mut().enumerate() {
            bus_from[i] = b.gain * b.duck;
            b.gain += (b.target - b.gain) * glide;
            b.duck += (1.0 - b.duck) * (1.0 - libm::expf(-(n as f32) / (rate * b.duck_release)));
            bus_to[i] = b.gain * b.duck;
        }
        let inv_n = 1.0 / n as f32;
        for v in &mut self.voices {
            if self.buses[v.bus].paused && !v.stopping {
                continue;
            }
            let (tl, tr) = pan_gains(v.target.pan, v.target.gain);
            let (l0, r0) = (v.gain_l, v.gain_r);
            v.gain_l += (tl - v.gain_l) * glide;
            v.gain_r += (tr - v.gain_r) * glide;
            let (l1, r1) = (v.gain_l, v.gain_r);
            v.cutoff += (v.target.cutoff - v.cutoff) * glide;
            let s0 = v.send;
            v.send += (v.target.send - v.send) * glide;
            let s1 = v.send;
            let sending = s0 > 0.0 || s1 > 0.0;
            let filtered = v.cutoff < OPEN_HZ;
            if filtered {
                for f in &mut v.filters {
                    f.tune(v.cutoff, rate);
                }
            }
            v.step = step_of(&v.pcm, v.target.pitch, self.rate);
            let (b0, b1) = (bus_from[v.bus], bus_to[v.bus]);
            let frames = v.pcm.frames() as f64;
            for (i, (out, send)) in self.block[..n].iter_mut().zip(self.sends[..n].iter_mut()).enumerate() {
                if v.delay > 0 {
                    v.delay -= 1;
                    continue;
                }
                if v.done {
                    break;
                }
                let t = i as f32 * inv_n;
                let bus = b0 + (b1 - b0) * t;
                let gl = (l0 + (l1 - l0) * t) * bus * v.fade;
                let gr = (r0 + (r1 - r0) * t) * bus * v.fade;
                let (mut sl, mut sr) = read(&v.pcm, v.pos, v.looped, frames);
                if filtered {
                    sl = v.filters[0].run(sl);
                    sr = v.filters[1].run(sr);
                }
                out[0] += sl * gl;
                out[1] += sr * gr;
                if sending {
                    let s = s0 + (s1 - s0) * t;
                    send[0] += sl * gl * s;
                    send[1] += sr * gr * s;
                }
                if v.stopping {
                    v.fade -= v.fade_step;
                    if v.fade <= 0.0 {
                        v.done = true;
                    }
                } else if v.fade < 1.0 {
                    v.fade = (v.fade + v.fade_step).min(1.0);
                }
                v.pos += v.step;
                if v.pos >= frames {
                    if v.looped {
                        v.pos -= frames;
                    } else {
                        v.done = true;
                    }
                }
            }
        }
        if !self.reverb.idle() {
            self.reverb.run(&self.sends[..n], &mut self.block[..n], glide);
        }
        let m0 = self.master;
        self.master += (self.master_target - self.master) * glide;
        let m1 = self.master;
        let dog = self.watchdog;
        for (i, f) in self.block[..n].iter_mut().enumerate() {
            let g = (m0 + (m1 - m0) * i as f32 * inv_n) * dog;
            let mut frame = [f[0] * g, f[1] * g];
            if self.mono {
                let m = 0.5 * (frame[0] + frame[1]);
                frame = [m, m];
            }
            self.limiter.run(&mut frame);
            *f = frame;
        }
    }
}

/// How many sample frames a voice moves per engine frame.
fn step_of(pcm: &Pcm, pitch: f32, rate: u32) -> f64 {
    pitch.clamp(0.05, 8.0) as f64 * pcm.rate as f64 / rate as f64
}

/// Equal-power pan: the left and right gains for `pan` in -1..1.
fn pan_gains(pan: f32, gain: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    // Scaled so the centre is unity on each side, not -3 dB.
    let k = std::f32::consts::SQRT_2 * gain;
    (libm::cosf(angle) * k, libm::sinf(angle) * k)
}

/// The sample at `pos`, linearly between its two frames; a loop reads round
/// its end.
#[inline]
fn read(pcm: &Pcm, pos: f64, looped: bool, frames: f64) -> (f32, f32) {
    let i = pos as isize;
    let t = (pos - i as f64) as f32;
    let next = if looped && (i + 1) as f64 >= frames { 0 } else { i + 1 };
    let (a0, a1) = pcm.frame(i);
    let (b0, b1) = pcm.frame(next);
    (a0 + (b0 - a0) * t, a1 + (b1 - a1) * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, seconds: f32) -> Arc<Pcm> {
        let n = (seconds * 48_000.0) as usize;
        let data = (0..n).map(|i| (libm::sinf(2.0 * std::f32::consts::PI * freq * i as f32 / 48_000.0) * 16000.0) as i16).collect();
        Pcm::new(48_000, 1, data)
    }

    fn render(engine: &mut Engine, seconds: f32) -> Vec<f32> {
        let mut out = vec![0.0; (seconds * 48_000.0) as usize * 2];
        engine.apply(Command::Beat);
        engine.render(&mut out);
        out
    }

    fn start(id: VoiceId, pcm: Arc<Pcm>, params: VoiceParams, looped: bool) -> Command {
        Command::Start { id, pcm, bus: 1, looped, params, delay: 0.0, offset: 0.0, fade_in: 0.0 }
    }

    #[test]
    fn a_one_shot_plays_then_ends() {
        let mut e = Engine::new(48_000);
        e.apply(start(1, sine(440.0, 0.1), VoiceParams::NEUTRAL, false));
        let out = render(&mut e, 0.2);
        let first = out[..4800 * 2].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let after = out[6000 * 2..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(first > 0.3, "it sounds: {first}");
        assert!(after < 1e-3, "and stops: {after}");
        assert_eq!(e.stats().voices, 0);
    }

    #[test]
    fn a_loop_keeps_playing_until_stopped() {
        let mut e = Engine::new(48_000);
        e.apply(start(1, sine(220.0, 0.05), VoiceParams::NEUTRAL, true));
        let out = render(&mut e, 0.2);
        assert!(out[out.len() - 200..].iter().any(|v| v.abs() > 0.1));
        e.apply(Command::Stop { id: 1, fade: 0.01 });
        render(&mut e, 0.05);
        assert_eq!(e.stats().voices, 0);
    }

    #[test]
    fn pan_moves_the_sound_to_one_side() {
        let mut e = Engine::new(48_000);
        e.apply(start(1, sine(440.0, 0.5), VoiceParams { pan: -1.0, ..VoiceParams::NEUTRAL }, false));
        let out = render(&mut e, 0.2);
        let left = out.iter().step_by(2).fold(0.0f32, |m, v| m.max(v.abs()));
        let right = out.iter().skip(1).step_by(2).fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(left > 0.3 && right < 1e-3, "left {left} right {right}");
    }

    #[test]
    fn a_delayed_voice_waits() {
        let mut e = Engine::new(48_000);
        e.apply(Command::Start { id: 1, pcm: sine(440.0, 0.5), bus: 1, looped: false, params: VoiceParams::NEUTRAL, delay: 0.1, offset: 0.0, fade_in: 0.0 });
        let out = render(&mut e, 0.2);
        let before = out[..4000 * 2].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let after = out[6000 * 2..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(before < 1e-6 && after > 0.3, "before {before} after {after}");
    }

    #[test]
    fn the_watchdog_silences_a_director_that_stopped_beating() {
        let mut e = Engine::new(48_000);
        e.apply(start(1, sine(220.0, 0.05), VoiceParams::NEUTRAL, true));
        let mut out = vec![0.0; 48_000 * 2];
        e.render(&mut out);
        let tail = out[out.len() - 2000..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(tail < 1e-3, "a second without a beat is silent: {tail}");
    }

    #[test]
    fn a_render_is_the_same_every_time() {
        let make = || {
            let mut e = Engine::new(48_000);
            e.apply(start(1, sine(330.0, 0.3), VoiceParams { pitch: 1.07, cutoff: 2000.0, pan: 0.3, gain: 0.8, send: 0.2 }, false));
            render(&mut e, 0.3)
        };
        assert_eq!(make(), make());
    }
}
