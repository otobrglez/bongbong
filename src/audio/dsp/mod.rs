//! The audio engine (docs/sound-system-prd.md section 4.1): it plays what the
//! director asks for and nothing else. Pure Rust with no raylib, so the same
//! code mixes the window's sound on every platform, renders a seeded round's
//! audio headless (`probe --audio-out`) and runs in the tests.
//!
//! The director speaks to it only through [`Command`]s - start a voice,
//! update its gain, pan, pitch and high-cut, stop it, set a bus - and the
//! engine renders interleaved stereo `f32` frames. It is `Send`, so a driver
//! may own it on the audio thread and hand it commands from the main one.
//!
//! Transcendental math goes through `libm`, as the simulation's does, so a
//! render is the same on every platform and a pinned render hash holds.

pub mod decode;
pub mod engine;
pub mod filter;
pub mod limiter;
pub mod reverb;
pub mod wav;

pub use decode::Pcm;
pub use engine::{Command, Engine, VoiceId, VoiceParams};

/// The rate the engine mixes at, in frames a second. Every sample file is
/// made at it, and the driver opens its stream at it.
pub const RATE: u32 = 48_000;

/// Decibels to a linear gain.
pub fn db_to_gain(db: f32) -> f32 {
    libm::powf(10.0, db / 20.0)
}

/// A linear gain to decibels, with silence as -120.
pub fn gain_to_db(gain: f32) -> f32 {
    if gain <= 1e-6 { -120.0 } else { 20.0 * libm::log10f(gain) }
}
