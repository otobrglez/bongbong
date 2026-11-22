//! A round's sound rendered offline (docs/sound-system-prd.md section 4.14):
//! the director and the engine stepped beside a headless round, a sixtieth
//! of a second of mix per step, written as a WAV. `probe --audio-out` and
//! the tests use it; the variation is hashed, never drawn, so a seed renders
//! to the same file every time.

use std::sync::Arc;

use crate::math::Vec2;
use crate::simulation::Game;

use super::cues::Manifest;
use super::dsp::{Engine, RATE};
use super::space::View;
use super::{Audio, bank};

pub struct Recorder {
    audio: Audio,
    engine: Engine,
    frames: Vec<f32>,
    /// The seats it listens for: the round's own, in order.
    seats: Vec<usize>,
    /// How wide (px) a view is: the standard field's.
    half_width: f32,
}

impl Recorder {
    /// A recorder of the shipped manifest; `placeholders` lets the sounds
    /// not made yet play their stand-ins.
    pub fn new(placeholders: bool, seats: Vec<usize>) -> Recorder {
        let mut audio = Audio::new(Arc::new(Manifest::shipped()), placeholders, bank::read_disk);
        audio.warm();
        Recorder { audio, engine: Engine::new(RATE), frames: Vec::new(), seats, half_width: crate::DEFAULT_SCREEN_WIDTH as f32 * 0.5 }
    }

    /// One step's sound: call after every `Game::update`.
    pub fn step(&mut self, game: &Game, dt: f32) {
        self.audio.observe_events(game);
        let seat = self.seats.first().and_then(|&s| super::bind::tank_in(game, s).map(|t| t.position));
        let center = seat.unwrap_or(Vec2::new(self.half_width, self.half_width * 0.5));
        let views = [View { center, half_width: self.half_width, seat }];
        self.audio.observe(game, &views, &self.seats, dt, true);
        for c in self.audio.take_commands() {
            self.engine.apply(c);
        }
        let n = (dt * RATE as f32).round() as usize;
        let start = self.frames.len();
        self.frames.resize(start + 2 * n, 0.0);
        self.engine.render(&mut self.frames[start..]);
    }

    /// The mix so far, interleaved stereo.
    pub fn frames(&self) -> &[f32] {
        &self.frames
    }

    pub fn write(&self, path: &std::path::Path) -> std::io::Result<()> {
        let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
        super::dsp::wav::write(&mut file, RATE, &self.frames)
    }
}
