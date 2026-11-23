//! Sound (docs/sound-system-prd.md). A presentation layer, as `fx.rs` is: it
//! reads the round each step and each frame - the replica online - and never
//! writes it, and it draws no round RNG, so seeds, replays, the probe and
//! the room server are untouched (CLAUDE.md's seeded-replay rule).
//!
//! [`Audio`] is the director. After every simulation step it reads the
//! step's events (`observe_events`, deduped on `Game::frame` the way `Fx`
//! is) and turns each into the sounds `bind.rs` gives it; once a frame
//! (`observe`) it lets the state watchers (`watch.rs`) ask for the loops no
//! event reports - engines, tracks, the minigun, fires, the weather - places
//! every sound for the listeners (`space.rs`), decides what may play (each
//! cue's polyphony and cooldown, the HDR window, the voice budget), and
//! writes [`dsp::Command`]s for the engine. The engine (`dsp/`) mixes them;
//! a driver - `render/audio.rs` in the window, a WAV writer offline - plays
//! the mix.
//!
//! Variation (which take, how much pitch and level jitter) is hashed from
//! the cue, the frame and the source, never drawn, so a seeded round renders
//! to the same audio every time.

pub mod bank;
pub mod bind;
pub mod cues;
pub mod dsp;
pub mod offline;
pub mod source;
pub mod space;
pub mod watch;

#[cfg(test)]
mod audio_tests;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::math::Vec2;
use crate::simulation::{Event, Game};
use crate::tank::Tank;
use crate::tuning::{Tuning, tuning};

use bind::{At, Binding, Play};
use cues::{Manifest, Steal};
use dsp::reverb::Room;
use dsp::{Command, VoiceId, VoiceParams, db_to_gain, gain_to_db};
use space::{Heard, Space, View};
use watch::{Watchers, Wish};

/// The buses sound is mixed on (section 4.5). Index with `index`; the
/// tuning rows `audio_bus_db` follow this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Bus {
    Music,
    Weapons,
    Impacts,
    Explosions,
    Vehicles,
    World,
    Warnings,
    Voice,
    Ui,
}

impl Bus {
    pub const ALL: [Bus; 9] = [Bus::Music, Bus::Weapons, Bus::Impacts, Bus::Explosions, Bus::Vehicles, Bus::World, Bus::Warnings, Bus::Voice, Bus::Ui];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        crate::tuning::BUS_NAMES[self.index()]
    }

    /// Whether it is the round's: held under a dialog or a pause, where the
    /// music, the voice and the UI carry on.
    pub fn in_round(self) -> bool {
        !matches!(self, Bus::Music | Bus::Voice | Bus::Ui)
    }
}

/// How far a sound carries (section 4.3); the distances are the
/// `audio_class_far_px` rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Tiny,
    Small,
    Medium,
    Large,
    Huge,
}

impl Class {
    pub fn index(self) -> usize {
        self as usize
    }
}

/// What a cue is, for the mix and the sound map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    OneShot,
    Loop,
    ModulatedLoop,
    Stinger,
    Ui,
    Voice,
}

impl Kind {
    pub fn looped(self) -> bool {
        matches!(self, Kind::Loop | Kind::ModulatedLoop)
    }
}

/// Which sound gives way when too many play; a warning never does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Medium,
    High,
    Critical,
    Warning,
}

/// The player's own settings (section 4.5), kept with the progress.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    pub master: f32,
    pub music: f32,
    pub effects: f32,
    pub voice: f32,
    pub muted: bool,
    pub mono: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { master: 1.0, music: 0.8, effects: 1.0, voice: 1.0, muted: false, mono: false }
    }
}

/// One sound the director has started: a one-shot or a watcher's loop, as
/// one engine voice per layer.
#[derive(Clone, Debug)]
struct Instance {
    cue: String,
    voices: Vec<(VoiceId, usize)>,
    bus: Bus,
    priority: Priority,
    /// dB at the loudest listener.
    loudness: f32,
    /// When it ends on the director's clock; never for a loop.
    ends: f64,
    /// A loop's watcher key.
    key: Option<u64>,
    seen: bool,
    own: bool,
}

impl Instance {
    fn rank(&self) -> (bool, Priority, f32) {
        (self.own, self.priority, self.loudness)
    }

    fn pinned(&self) -> bool {
        self.own || self.priority == Priority::Warning
    }
}

/// What the dev tools read (`audio_status`).
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub voices: usize,
    pub sounds: Vec<SoundStatus>,
    pub hdr_top_db: f32,
    pub recent: Vec<String>,
    pub missing: Vec<String>,
    pub settings: Settings,
}

#[derive(Clone, Debug, Serialize)]
pub struct SoundStatus {
    pub cue: String,
    pub bus: Bus,
    pub loudness_db: f32,
    pub looped: bool,
    pub own: bool,
}

pub struct Audio {
    bank: bank::Bank,
    space: Space,
    watch: Watchers,
    playing: Vec<Instance>,
    pending: Vec<Play>,
    cooldown: HashMap<(String, u64), f64>,
    commands: Vec<Command>,
    next_voice: VoiceId,
    hdr_top: f32,
    time: f64,
    last_frame: u64,
    seed: Option<u64>,
    settings: Settings,
    sent_bus: [f32; 9],
    sent_master: (f32, bool),
    sent_room: Option<Room>,
    paused: bool,
    variation: u64,
    recent: VecDeque<String>,
    /// The owner slot whose shots this window draws at the press online
    /// (`OnlineRound::draws_own_shots`): its own presses play at once
    /// (`own_press`) and the room's late copies of them are skipped.
    predicted: Option<usize>,
}

/// How many recent cues the dev tools see.
const RECENT: usize = 32;

impl Audio {
    pub fn new(manifest: Arc<Manifest>, placeholders: bool, read: bank::Reader) -> Audio {
        Audio {
            bank: bank::Bank::new(manifest, placeholders, read),
            space: Space::default(),
            watch: Watchers::default(),
            playing: Vec::new(),
            pending: Vec::new(),
            cooldown: HashMap::new(),
            commands: Vec::new(),
            next_voice: 1,
            hdr_top: -120.0,
            time: 0.0,
            last_frame: 0,
            seed: None,
            settings: Settings::default(),
            sent_bus: [f32::NAN; 9],
            sent_master: (f32::NAN, false),
            sent_room: None,
            paused: false,
            variation: 0,
            recent: VecDeque::new(),
            predicted: None,
        }
    }

    /// The shipped manifest, its files read from disk; a dev-tools build
    /// plays placeholders for the sounds not made yet.
    pub fn shipped() -> Audio {
        Audio::new(Arc::new(Manifest::shipped()), cfg!(feature = "dev-tools"), bank::read_disk)
    }

    pub fn manifest(&self) -> &Manifest {
        self.bank.manifest()
    }

    /// Swaps the manifest (the dev build's hot reload); what plays carries on.
    pub fn reload(&mut self, manifest: Manifest) {
        self.bank.replace(Arc::new(manifest));
    }

    /// Decodes every sample now rather than on its first play.
    pub fn warm(&mut self) {
        self.bank.warm();
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    pub fn set_settings(&mut self, settings: Settings) {
        self.settings = settings;
    }

    /// A UI sound, with no place.
    pub fn ui(&mut self, cue: &str) {
        self.pending.push(Play::new(cue, At::Nowhere));
    }

    /// The bong putting a line up (section 4.7): its babble where it
    /// stands, no words, pitched by its kind. The app calls it when
    /// `bubble::BongVoice` shows a new line.
    pub fn babble(&mut self, at: Vec2, kind: crate::bong::BongKind) {
        self.pending.push(Play::new(cues::ids::BONG_BABBLE.0, At::Point(at)).pitch(source::bong_pitch(kind)));
    }

    /// The commands the frame wrote, for the driver.
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands)
    }

    /// Reads one simulation step's events. Call after every `update`, as
    /// the app does `Fx::observe_events`; a step already read is skipped.
    pub fn observe_events(&mut self, game: &Game) {
        let (frame, seed) = (game.frame(), game.round_seed());
        if self.seed == Some(seed) && frame == self.last_frame {
            return;
        }
        if self.seed != Some(seed) || frame < self.last_frame {
            self.new_round();
        }
        self.seed = Some(seed);
        self.last_frame = frame;
        for event in game.events() {
            if matches!(event, Event::RoundStarted { .. } | Event::ObstacleDestroyed { .. } | Event::ChunksBroken { collapsed: true, .. }) {
                self.space.walls_changed();
            }
            if self.room_copy_of_own_press(event) {
                continue;
            }
            if let Event::Ricochet { id, shot, x, y, heading, .. } = *event {
                self.watch.ricocheted(id, shot, Vec2::new(x, y), heading, self.time);
            }
            if let Binding::Play(plays) = bind::bind(event, game) {
                self.pending.extend(plays);
            }
        }
    }

    /// Online, the seat whose shots this window draws at the press, or
    /// `None` (a local round, or a client that waits for the room).
    pub fn set_predicted_seat(&mut self, slot: Option<usize>) {
        self.predicted = slot;
    }

    /// Whether `event` is the room's late copy of a press this window
    /// already sounded (docs/sound-system-prd.md section 4.10): the own
    /// seat's cannon, plasma, well anchor, charge end or dry click. The
    /// shows of the weapons drawn on the press never reach the replica
    /// (`apply.rs`).
    fn room_copy_of_own_press(&self, event: &Event) -> bool {
        let Some(own) = self.predicted else {
            return false;
        };
        match *event {
            Event::Fired { slot, weapon } => slot == own && matches!(weapon, "shell" | "plasma" | "minigun"),
            Event::WellAnchored { slot, .. } | Event::ChargeEnded { slot, .. } | Event::DryFired { slot, .. } => slot == own,
            _ => false,
        }
    }

    /// This client's own press online, sounded at once from its tank in
    /// `slot` rather than a round trip later.
    pub fn own_press(&mut self, game: &Game, slot: usize, press: crate::net::round::PressSound) {
        use crate::net::round::PressSound;
        let cue = match press {
            PressSound::Shell | PressSound::Plasma => {
                let weapon = if press == PressSound::Shell { "shell" } else { "plasma" };
                if let Binding::Play(plays) = bind::bind(&Event::Fired { slot, weapon }, game) {
                    self.pending.extend(plays);
                }
                return;
            }
            PressSound::Bullet => {
                self.watch.round_fired(slot, self.time);
                return;
            }
            PressSound::Beam => "weapon.laser.fire",
            PressSound::Sonic => "weapon.sonic_hammer.fire",
            PressSound::Emp => "weapon.emp_burst.fire",
            PressSound::Rail => "weapon.gauss_rail.fire",
            PressSound::ChargeEnd => "weapon.gauss_rail.vent",
            PressSound::Drone => "weapon.fpv_swarm.launch",
            PressSound::Rod => {
                let whistle = (tuning().rod_countdown_seconds - 3.0).max(0.0);
                self.pending.push(Play::new(cues::ids::ROD_INCOMING.0, At::Slot(slot)).after(whistle));
                "weapon.rod_from_god.call"
            }
            PressSound::Orb => "weapon.gravity_well.fire",
            PressSound::Anchor => "weapon.gravity_well.anchor",
            PressSound::Dry => {
                if let Binding::Play(plays) = bind::bind(&Event::DryFired { slot, weapon: "shell" }, game) {
                    self.pending.extend(plays);
                }
                return;
            }
        };
        self.pending.push(Play::new(cue, At::Slot(slot)).source(slot as u64));
    }

    /// A new round: every loop ends, the watchers forget, the walls are
    /// read again.
    fn new_round(&mut self) {
        for inst in self.playing.drain(..) {
            for (id, _) in inst.voices {
                self.commands.push(Command::Stop { id, fade: 0.2 });
            }
        }
        self.watch.reset();
        self.cooldown.clear();
        self.space.walls_changed();
        self.hdr_top = -120.0;
    }

    /// The frame's work: the watchers, the loops, the sounds the steps
    /// asked for, the levels. `views` are the window's views; `own` the
    /// owner slots this window plays; `dt` the frame's real time; `running`
    /// whether the round moves (not under a dialog, a pause or the builder).
    pub fn observe(&mut self, game: &Game, views: &[View], own: &[usize], dt: f32, running: bool) {
        let t = tuning();
        let dt = dt.max(0.0);
        self.time += dt as f64;
        self.space.set_views(views, dt, &t);
        self.space.refresh(game);
        if running == self.paused {
            self.paused = !running;
            for bus in Bus::ALL.into_iter().filter(|b| b.in_round()) {
                self.commands.push(Command::Pause { bus: bus.index(), paused: self.paused });
            }
        }
        self.hdr_top = (self.hdr_top - t.audio_hdr_release_db_s * dt).max(-120.0);
        let now = self.time;
        self.playing.retain(|i| i.key.is_some() || i.ends > now);
        if running {
            let mut wishes = Vec::new();
            let mut plays = Vec::new();
            self.watch.sample(game, own, self.time, &t, &mut wishes, &mut plays);
            self.pending.extend(plays);
            self.loops(&wishes, &t);
        }
        for play in std::mem::take(&mut self.pending) {
            self.start(game, play, own, &t);
        }
        self.levels(&t);
        self.room(game, &t);
        self.commands.push(Command::Beat);
    }

    /// The room the round's sounds ring in (section 4.3): an arena's
    /// shorter, a field's longer and more open; sent when it changes.
    fn room(&mut self, game: &Game, t: &Tuning) {
        let seconds = match game.map.class() {
            crate::framing::MapClass::Arena => t.audio_reverb_arena_s,
            crate::framing::MapClass::Field => t.audio_reverb_field_s,
        };
        let room = Room { seconds, damp_hz: t.audio_reverb_damp_hz, gain: db_to_gain(t.audio_reverb_db) };
        if self.sent_room != Some(room) {
            self.sent_room = Some(room);
            self.commands.push(Command::Room(room));
        }
    }

    fn hash(&mut self, cue: &str, source: u64) -> u64 {
        self.variation = self.variation.wrapping_add(1);
        let mut h = self.last_frame ^ source.rotate_left(17) ^ self.variation.rotate_left(41);
        for b in cue.bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01B3);
        }
        splitmix(h)
    }

    fn tank_at(&self, game: &Game, slot: usize) -> Option<Vec2> {
        game.world.query::<&Tank>().iter().find(|t| t.owner_slot() == slot).map(|t| t.position)
    }

    /// Starts one one-shot, if the mix lets it: one for a seat this window
    /// does not play (`Hears::Seat`) never does.
    fn start(&mut self, game: &Game, play: Play, own: &[usize], t: &Tuning) {
        if let bind::Hears::Seat(seat) = play.hears
            && !own.contains(&seat)
        {
            return;
        }
        let Some((id, cue)) = self.bank.manifest().resolve(&play.cue).map(|(k, c)| (k.to_string(), c.clone())) else {
            return;
        };
        let at = match play.at {
            At::Point(p) => Some(p),
            At::Slot(slot) => match self.tank_at(game, slot) {
                Some(p) => Some(p),
                None => return,
            },
            At::Nowhere => None,
        };
        let mine = matches!(play.at, At::Slot(s) if own.contains(&s)) || matches!(play.hears, bind::Hears::Seat(_));
        let pinned = mine || cue.priority == Priority::Warning;
        let mut heard = match at {
            Some(p) => self.space.hear(p, Vec2::new(0.0, 0.0), cue.class, t),
            None => Heard::CENTRE,
        };
        if pinned {
            heard.delay = 0.0;
            heard.gain = heard.gain.max(0.7);
        }
        let h = self.hash(&id, play.source);
        let (pitch_j, gain_j) = if cue.jitter { jitter(h, t) } else { (1.0, 1.0) };
        let gain = heard.gain * play.strength * db_to_gain(cue.gain_db) * gain_j;
        if gain < 1e-4 {
            return;
        }
        let loudness = gain_to_db(gain);
        if cue.cooldown_ms > 0 {
            let k = (id.clone(), play.source);
            if self.cooldown.get(&k).is_some_and(|&last| self.time - last < cue.cooldown_ms as f64 / 1000.0) {
                return;
            }
            self.cooldown.insert(k, self.time);
        }
        if !pinned && loudness < self.hdr_top - t.audio_hdr_window_db {
            return;
        }
        if !self.make_room(&id, &cue, loudness, pinned, false, t) {
            return;
        }
        let mut voices = Vec::new();
        let mut ends = self.time;
        for (li, layer) in cue.layers.iter().enumerate() {
            let w = layer.weight(heard.distance);
            if w <= 1e-3 {
                continue;
            }
            let takes = self.bank.takes(&layer.sample);
            if takes.is_empty() {
                continue;
            }
            let pcm = takes[play.take.unwrap_or(h as usize) % takes.len()].clone();
            let pitch = play.pitch * layer.pitch * heard.doppler * pitch_j;
            let pan = (heard.pan + play.pan).clamp(-1.0, 1.0);
            let params = VoiceParams { gain: gain * w * db_to_gain(layer.gain_db), pan, pitch, cutoff: heard.cutoff, send: heard.send };
            let vid = self.voice_id();
            let delay = heard.delay + play.after;
            ends = ends.max(self.time + delay as f64 + (pcm.seconds() / pitch.max(0.05)) as f64);
            self.commands.push(Command::Start { id: vid, pcm, bus: cue.bus.index(), looped: false, params, delay, offset: 0.0, fade_in: 0.0 });
            voices.push((vid, li));
        }
        if voices.is_empty() {
            return;
        }
        self.duck(cue.bus, gain, t);
        self.hdr_top = self.hdr_top.max(loudness);
        self.note(&id);
        self.playing.push(Instance { cue: id, voices, bus: cue.bus, priority: cue.priority, loudness, ends, key: None, seen: true, own: mine });
    }

    /// The watchers' loops: start the new ones, move the playing ones, let
    /// go of the ones no longer wished for or out of earshot.
    fn loops(&mut self, wishes: &[Wish], t: &Tuning) {
        for inst in &mut self.playing {
            inst.seen = false;
        }
        let mut placed: Vec<(&Wish, String, cues::Cue, Heard, f32)> = wishes
            .iter()
            .filter_map(|w| {
                let (id, cue) = self.bank.manifest().resolve(w.cue)?;
                let mut heard = match w.at {
                    Some(p) => self.space.hear(p, w.velocity, cue.class, t),
                    None => Heard::CENTRE,
                };
                if w.own {
                    heard.gain = heard.gain.max(0.8);
                }
                let gain = heard.gain * w.gain * db_to_gain(cue.gain_db);
                Some((w, id.to_string(), cue.clone(), heard, gain))
            })
            .collect();
        placed.sort_by(|a, b| b.4.total_cmp(&a.4));
        for (wish, id, cue, heard, gain) in placed {
            let loudness = gain_to_db(gain);
            let audible = gain >= 1e-4 && (wish.own || loudness >= self.hdr_top - t.audio_hdr_window_db);
            let pitch = wish.pitch * heard.doppler;
            if let Some(pos) = self.playing.iter().position(|i| i.key == Some(wish.key)) {
                if !audible {
                    let inst = self.playing.remove(pos);
                    for (vid, _) in inst.voices {
                        self.commands.push(Command::Stop { id: vid, fade: 0.25 });
                    }
                    continue;
                }
                let inst = &mut self.playing[pos];
                inst.seen = true;
                inst.loudness = loudness;
                for &(vid, li) in &inst.voices {
                    let layer = &cue.layers[li.min(cue.layers.len() - 1)];
                    let w = layer.weight(heard.distance);
                    let params = VoiceParams { gain: gain * w * db_to_gain(layer.gain_db), pan: heard.pan, pitch: pitch * layer.pitch, cutoff: heard.cutoff, send: heard.send };
                    self.commands.push(Command::Update { id: vid, params });
                }
                continue;
            }
            if !audible || !self.make_room(&id, &cue, loudness, wish.own, true, t) {
                continue;
            }
            let h = self.hash(&id, wish.key);
            let mut voices = Vec::new();
            for (li, layer) in cue.layers.iter().enumerate() {
                let takes = self.bank.takes(&layer.sample);
                if takes.is_empty() {
                    continue;
                }
                let pcm = takes[(h as usize) % takes.len()].clone();
                let w = layer.weight(heard.distance);
                let params = VoiceParams { gain: gain * w * db_to_gain(layer.gain_db), pan: heard.pan, pitch: pitch * layer.pitch, cutoff: heard.cutoff, send: heard.send };
                let offset = (h >> 16) as f32 / (1u64 << 48) as f32 * pcm.seconds();
                let vid = self.voice_id();
                self.commands.push(Command::Start { id: vid, pcm, bus: cue.bus.index(), looped: true, params, delay: 0.0, offset, fade_in: 0.15 });
                voices.push((vid, li));
            }
            if voices.is_empty() {
                continue;
            }
            self.note(&id);
            self.playing.push(Instance { cue: id, voices, bus: cue.bus, priority: cue.priority, loudness, ends: f64::INFINITY, key: Some(wish.key), seen: true, own: wish.own });
        }
        let mut gone = Vec::new();
        self.playing.retain(|i| {
            let keep = i.key.is_none() || i.seen;
            if !keep {
                gone.extend(i.voices.iter().map(|&(v, _)| v));
            }
            keep
        });
        for id in gone {
            self.commands.push(Command::Stop { id, fade: 0.25 });
        }
    }

    /// Makes room for a new sound of cue `id` at `loudness`: under the cue's
    /// polyphony (stealing as the cue says) and the voice budget (stealing
    /// the lowest-ranked sound below it). False if it may not play.
    fn make_room(&mut self, id: &str, cue: &cues::Cue, loudness: f32, pinned: bool, looped: bool, t: &Tuning) -> bool {
        let same: Vec<usize> = (0..self.playing.len()).filter(|&i| self.playing[i].cue == id && self.playing[i].key.is_some() == looped).collect();
        if same.len() >= cue.polyphony.max(1) as usize {
            let victim = match cue.steal {
                Steal::None => return false,
                Steal::Oldest => same.first().copied(),
                Steal::Quietest => same.iter().copied().min_by(|&a, &b| self.playing[a].loudness.total_cmp(&self.playing[b].loudness)),
            };
            match victim {
                Some(v) if cue.steal == Steal::Oldest || self.playing[v].loudness <= loudness || pinned => self.stop(v),
                _ => return false,
            }
        }
        let new_rank = (pinned, cue.priority, loudness);
        while self.playing.iter().map(|i| i.voices.len()).sum::<usize>() + cue.layers.len() > t.audio_voices {
            let victim = (0..self.playing.len())
                .filter(|&i| !self.playing[i].pinned())
                .min_by(|&a, &b| self.playing[a].rank().partial_cmp(&self.playing[b].rank()).unwrap_or(std::cmp::Ordering::Equal));
            match victim {
                Some(v) if self.playing[v].rank() < new_rank => self.stop(v),
                _ => return false,
            }
        }
        true
    }

    fn stop(&mut self, index: usize) {
        let inst = self.playing.remove(index);
        for (id, _) in inst.voices {
            self.commands.push(Command::Stop { id, fade: 0.05 });
        }
    }

    /// A loud sound ducks the beds under it (section 4.5).
    fn duck(&mut self, bus: Bus, gain: f32, t: &Tuning) {
        let (targets, depth): (&[Bus], f32) = match bus {
            Bus::Explosions => (&[Bus::Vehicles, Bus::World], t.audio_explosion_duck_db * gain.min(1.0)),
            Bus::Voice => (&[Bus::Music, Bus::World], -6.0),
            Bus::Warnings => (&[Bus::Music], -4.0),
            _ => return,
        };
        for b in targets {
            self.commands.push(Command::Duck { bus: b.index(), gain: db_to_gain(depth), release: t.audio_duck_release_s });
        }
    }

    /// The buses' and the master's levels, sent when they change.
    fn levels(&mut self, t: &Tuning) {
        for bus in Bus::ALL {
            let i = bus.index();
            let setting = match bus {
                Bus::Music => self.settings.music,
                Bus::Voice => self.settings.voice,
                _ => self.settings.effects,
            };
            let gain = db_to_gain(t.audio_bus_db[i]) * setting;
            if !(gain - self.sent_bus[i]).abs().le(&1e-5) {
                self.sent_bus[i] = gain;
                self.commands.push(Command::Bus { bus: i, gain });
            }
        }
        let master = if self.settings.muted { 0.0 } else { db_to_gain(t.audio_master_db) * self.settings.master };
        let mono = self.settings.mono;
        if !(master - self.sent_master.0).abs().le(&1e-5) || mono != self.sent_master.1 {
            self.sent_master = (master, mono);
            self.commands.push(Command::Master { gain: master, mono });
        }
    }

    fn voice_id(&mut self) -> VoiceId {
        let id = self.next_voice;
        self.next_voice = self.next_voice.wrapping_add(1).max(1);
        id
    }

    fn note(&mut self, cue: &str) {
        if self.recent.len() == RECENT {
            self.recent.pop_front();
        }
        self.recent.push_back(cue.to_string());
    }

    pub fn status(&self) -> Status {
        Status {
            voices: self.playing.iter().map(|i| i.voices.len()).sum(),
            sounds: self
                .playing
                .iter()
                .map(|i| SoundStatus { cue: i.cue.clone(), bus: i.bus, loudness_db: i.loudness, looped: i.key.is_some(), own: i.own })
                .collect(),
            hdr_top_db: self.hdr_top,
            recent: self.recent.iter().cloned().collect(),
            missing: self.bank.missing().to_vec(),
            settings: self.settings,
        }
    }
}

/// A take's pitch and level jitter, from a hash.
fn jitter(h: u64, t: &Tuning) -> (f32, f32) {
    let a = ((h >> 8) & 0xFFFF) as f32 / 65535.0 * 2.0 - 1.0;
    let b = ((h >> 24) & 0xFFFF) as f32 / 65535.0 * 2.0 - 1.0;
    (1.0 + a * t.audio_pitch_jitter, db_to_gain(b * t.audio_gain_jitter_db))
}

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
