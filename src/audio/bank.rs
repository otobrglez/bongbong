//! The sample bank: each manifest sample's takes, decoded on first use and
//! kept (docs/sound-system-prd.md section 4.8).
//!
//! A sample whose files are missing - planned, not yet made - plays a short
//! synthesized placeholder in a dev-tools build, coded by bus (a tick for an
//! impact, a thump for an explosion, a hum for a loop), so a hooked cue with
//! no sound is audible while play-testing; a release build plays nothing for
//! it.

use std::collections::HashMap;
use std::sync::Arc;

use super::cues::{Manifest, ROOT};
use super::dsp::{Pcm, RATE};
use super::Bus;

/// How a file is read: the window and the probe read the disk (the web's
/// preloaded file system included); a test can hand in its own.
pub type Reader = fn(&str) -> Option<Vec<u8>>;

pub fn read_disk(path: &str) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

pub struct Bank {
    manifest: Arc<Manifest>,
    takes: HashMap<String, Vec<Arc<Pcm>>>,
    placeholders: bool,
    read: Reader,
    /// The samples that played a placeholder, for the sound map and the dev
    /// tools.
    missing: Vec<String>,
}

impl Bank {
    pub fn new(manifest: Arc<Manifest>, placeholders: bool, read: Reader) -> Bank {
        Bank { manifest, takes: HashMap::new(), placeholders, read, missing: Vec::new() }
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn manifest_arc(&self) -> Arc<Manifest> {
        self.manifest.clone()
    }

    /// Swaps the manifest (the dev build's hot reload) and forgets every
    /// decoded take, so changed files are read again.
    pub fn replace(&mut self, manifest: Arc<Manifest>) {
        self.manifest = manifest;
        self.takes.clear();
        self.missing.clear();
    }

    pub fn missing(&self) -> &[String] {
        &self.missing
    }

    /// The takes of `sample`, decoding them the first time.
    pub fn takes(&mut self, sample: &str) -> &[Arc<Pcm>] {
        if !self.takes.contains_key(sample) {
            let loaded = self.load(sample);
            self.takes.insert(sample.to_string(), loaded);
        }
        &self.takes[sample]
    }

    /// Decodes every sample now, so the first shot of a round does not
    /// stall on a file.
    pub fn warm(&mut self) {
        let ids: Vec<String> = self.manifest.sample.keys().cloned().collect();
        for id in ids {
            self.takes(&id);
        }
    }

    fn load(&mut self, sample: &str) -> Vec<Arc<Pcm>> {
        let Some(def) = self.manifest.sample.get(sample) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for take in &def.takes {
            let path = format!("{ROOT}/{}", take.file);
            match (self.read)(&path).map(|bytes| super::dsp::decode::ogg(&bytes)) {
                Some(Ok(pcm)) => out.push(pcm),
                Some(Err(e)) => eprintln!("bongbong: audio: {path}: {e}"),
                None => {}
            }
        }
        if out.is_empty() {
            self.missing.push(sample.to_string());
            if self.placeholders {
                out.push(placeholder(def.bus, def.looped, sample));
            }
        }
        out
    }
}

/// A dev build's stand-in for a sample with no file: short, quiet and
/// unmistakably a placeholder, its shape by bus.
pub fn placeholder(bus: Bus, looped: bool, id: &str) -> Arc<Pcm> {
    let mut noise = Noise(id.bytes().fold(0x9E37_79B9_7F4A_7C15u64, |h, b| h.rotate_left(5) ^ b as u64));
    let rate = RATE as f32;
    let tone = |freq: f32, i: usize| libm::sinf(2.0 * std::f32::consts::PI * freq * i as f32 / rate);
    let (seconds, f): (f32, Box<dyn Fn(usize, f32, &mut Noise) -> f32>) = if looped {
        (0.5, Box::new(move |i, _, n| 0.15 * tone(70.0, i) + 0.05 * n.next()))
    } else {
        match bus {
            Bus::Weapons => (0.18, Box::new(move |_, t, n| 0.6 * n.next() * libm::expf(-t * 25.0))),
            Bus::Impacts => (0.06, Box::new(move |i, t, _| 0.5 * tone(1800.0, i) * libm::expf(-t * 80.0))),
            Bus::Explosions => (0.6, Box::new(move |i, t, n| (0.5 * n.next() + 0.4 * tone(55.0, i)) * libm::expf(-t * 6.0))),
            Bus::Warnings => (0.2, Box::new(move |i, _, _| 0.3 * tone(880.0, i))),
            Bus::Voice => (0.25, Box::new(move |i, t, _| 0.3 * tone(400.0 + 600.0 * t, i))),
            Bus::Ui => (0.03, Box::new(move |i, _, _| 0.3 * tone(1200.0, i))),
            Bus::Vehicles | Bus::World | Bus::Music => (0.3, Box::new(move |i, t, n| (0.2 * tone(110.0, i) + 0.1 * n.next()) * libm::expf(-t * 4.0))),
        }
    };
    let frames = (seconds * rate) as usize;
    let data = (0..frames)
        .map(|i| {
            let t = i as f32 / rate;
            // A 2 ms edge each end, so the placeholder never clicks.
            let edge = (i.min(frames - 1 - i) as f32 / (0.002 * rate)).min(1.0);
            (f(i, t, &mut noise) * edge * 32767.0) as i16
        })
        .collect();
    Pcm::new(RATE, 1, data)
}

struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }
}
