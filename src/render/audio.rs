//! The raylib half of `audio`: the one `AudioStream` the engine's mix plays
//! through (docs/sound-system-prd.md D2).
//!
//! The engine lives behind a lock the device's callback takes: raylib calls
//! `fill` from miniaudio's audio thread natively and from the page's audio
//! callback on the web, and it applies the commands the main thread queued,
//! then mixes. The main thread only ever queues (`AudioOut::send`), so a
//! frame that stalls never starves the device; the engine's watchdog (D5)
//! fades it out if the queue goes quiet. Nothing here holds the engine's
//! lock while it calls into raylib, so the two locks - ours and raylib's
//! mixer's - are never taken in the opposite order.
//!
//! sola-raylib's `set_audio_stream_callback` drops the stream it is handed
//! and gives the callback a read-only buffer, and `RaylibAudio` closes the
//! device when any clone of it drops: the stream is set up through raylib's
//! own calls, and the device handle is kept for the life of the program.

use std::ffi::{c_uint, c_void};
use std::sync::Mutex;

use sola_raylib::core::audio::RaylibAudio;
use sola_raylib::ffi;

use crate::audio::dsp::engine::EngineStats;
use crate::audio::dsp::{Command, Engine, RATE};

static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);
static QUEUE: Mutex<Vec<Command>> = Mutex::new(Vec::new());

/// raylib's stream callback: `frames` stereo `f32` frames to fill.
unsafe extern "C" fn fill(buffer: *mut c_void, frames: c_uint) {
    // SAFETY: raylib hands the stream's own buffer, `frames` frames in its
    // format: two `f32` channels, as `AudioOut::open` loaded it.
    let out = unsafe { std::slice::from_raw_parts_mut(buffer as *mut f32, frames as usize * 2) };
    // A queue the main thread holds this instant waits for the next call.
    let commands = QUEUE.try_lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    if let Ok(mut engine) = ENGINE.lock() {
        if let Some(engine) = engine.as_mut() {
            for c in commands {
                engine.apply(c);
            }
            engine.render(out);
            return;
        }
    }
    out.fill(0.0);
}

/// The open device and its stream.
pub struct AudioOut {
    _stream: ffi::AudioStream,
}

impl AudioOut {
    /// Opens the default device; `None` where there is none (a CI box, a
    /// device refused), and the game plays silent.
    pub fn open() -> Option<AudioOut> {
        let audio = RaylibAudio::init_audio_device().ok()?;
        if !audio.is_audio_device_ready() {
            return None;
        }
        // Kept for the program's life: dropping it closes the device.
        let audio: &'static RaylibAudio = Box::leak(Box::new(audio));
        *ENGINE.lock().ok()? = Some(Engine::new(RATE));
        let stream = audio.new_audio_stream(RATE, 32, 2);
        if !stream.is_audio_stream_valid() {
            return None;
        }
        // SAFETY: the stream stays loaded for the program's life, and the
        // device it belongs to is never closed (`audio` is leaked above).
        let raw = unsafe { stream.inner() };
        unsafe {
            ffi::SetAudioStreamCallback(raw, Some(fill));
            ffi::PlayAudioStream(raw);
        }
        Some(AudioOut { _stream: raw })
    }

    /// Queues the frame's commands for the next callback.
    pub fn send(&self, commands: Vec<Command>) {
        if commands.is_empty() {
            return;
        }
        if let Ok(mut q) = QUEUE.lock() {
            q.extend(commands);
        }
    }

    pub fn stats(&self) -> EngineStats {
        ENGINE.lock().ok().and_then(|e| e.as_ref().map(Engine::stats)).unwrap_or_default()
    }
}
