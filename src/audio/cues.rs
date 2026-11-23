//! The cue manifest, `audio/cues.toml` (docs/sound-system-prd.md section 4.8):
//! every sound the game plays and how it plays, embedded in the binary the
//! way the language catalogues are, so the headless build, the tests and the
//! web all read the same file.
//!
//! Two levels. A **sample** is a set of sound files - the takes - and the
//! recipe that made them (prompt, model, parameters, provenance). A **cue**
//! is a behaviour: which bus, how far it carries, how many may play at once,
//! and which samples it layers by distance. Generation works on samples;
//! mixing works on cues; many cues share samples.
//!
//! Cue ids are dotted, general to specific: `hit.shell.metal.iron`. A cue
//! the manifest lacks falls back to its parent (`hit.shell.metal`, then
//! `hit.shell`), so a new material plays its family's sound at once and the
//! sound map shows the specific one as wanted.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{Bus, Class, Kind, Priority};

/// The manifest as shipped.
pub const SHIPPED: &str = include_str!("../../audio/cues.toml");

/// Where the sample files live, relative to the working directory.
pub const ROOT: &str = "static/audio";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default)]
    pub style: Style,
    #[serde(default)]
    pub sample: BTreeMap<String, Sample>,
    #[serde(default)]
    pub cue: BTreeMap<String, Cue>,
}

/// The house style every prompt is built on (docs/SOUND_SPEC.md).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Style {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub prefix: String,
}

/// Where a sample stands in the pipeline (section 4.12).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Wanted, with a recipe, and no take yet.
    #[default]
    Planned,
    /// Takes made and waiting for a pick.
    Review,
    /// Picked, processed and shipped.
    Approved,
    /// A stand-in file until the real one is made.
    Placeholder,
    /// Heard and turned down: to be made again.
    Rejected,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    #[serde(default)]
    pub status: Status,
    pub bus: Bus,
    #[serde(default = "one")]
    pub channels: u16,
    /// The prompt after the style prefix.
    pub prompt: String,
    pub duration_seconds: f32,
    #[serde(default = "default_influence")]
    pub prompt_influence: f32,
    #[serde(rename = "loop", default)]
    pub looped: bool,
    #[serde(default = "default_takes")]
    pub takes_wanted: u8,
    /// The model it was or is to be made with, when not the style's.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default, rename = "take")]
    pub takes: Vec<Take>,
}

/// One shipped file and where it came from.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Take {
    /// Under `static/audio/`.
    pub file: String,
    #[serde(default)]
    pub flow: Option<String>,
    #[serde(default)]
    pub generation: Option<String>,
    #[serde(default)]
    pub made: Option<String>,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub rms_db: Option<f32>,
    #[serde(default)]
    pub peak_dbtp: Option<f32>,
    #[serde(default)]
    pub seconds: Option<f32>,
}

/// How a cue steals a voice when it is at its polyphony.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Steal {
    /// The quietest copy gives way.
    #[default]
    Quietest,
    /// The oldest copy gives way.
    Oldest,
    /// The new one is not played.
    None,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Cue {
    pub kind: Kind,
    pub bus: Bus,
    pub priority: Priority,
    pub class: Class,
    #[serde(default = "default_polyphony")]
    pub polyphony: u8,
    #[serde(default)]
    pub cooldown_ms: u32,
    #[serde(default)]
    pub steal: Steal,
    #[serde(default)]
    pub gain_db: f32,
    /// Whether takes vary in pitch and level (`audio_pitch_jitter`).
    #[serde(default = "yes")]
    pub jitter: bool,
    #[serde(default)]
    pub layers: Vec<Layer>,
    /// What the cue is, for the sound map.
    #[serde(default)]
    pub note: String,
}

/// One sample a cue plays, over a band of distance; neighbouring bands
/// crossfade.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub sample: String,
    #[serde(default)]
    pub from_px: f32,
    #[serde(default = "far_away")]
    pub until_px: f32,
    #[serde(default)]
    pub gain_db: f32,
    /// A pitch scale on the sample, 1 as made: one sample serving a
    /// smaller sound than it was made for (a round on wood playing the
    /// shell's splinter higher).
    #[serde(default = "unity")]
    pub pitch: f32,
}

/// How wide a band's edge fades (px).
pub const LAYER_FADE_PX: f32 = 200.0;

impl Layer {
    /// How much of this layer plays at `distance`, 0..1.
    pub fn weight(&self, distance: f32) -> f32 {
        let rise = if self.from_px <= 0.0 { 1.0 } else { ((distance - self.from_px) / LAYER_FADE_PX + 1.0).clamp(0.0, 1.0) };
        let fall = if self.until_px >= far_away() { 1.0 } else { ((self.until_px - distance) / LAYER_FADE_PX).clamp(0.0, 1.0) };
        rise * fall
    }
}

fn one() -> u16 {
    1
}
fn yes() -> bool {
    true
}
fn default_influence() -> f32 {
    0.45
}
fn default_takes() -> u8 {
    4
}
fn default_polyphony() -> u8 {
    4
}
fn unity() -> f32 {
    1.0
}
fn far_away() -> f32 {
    1.0e9
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let m: Manifest = toml::from_str(text).map_err(|e| e.to_string())?;
        for (id, cue) in &m.cue {
            for layer in &cue.layers {
                if !m.sample.contains_key(&layer.sample) {
                    return Err(format!("cue {id} plays sample {} the manifest lacks", layer.sample));
                }
            }
        }
        Ok(m)
    }

    /// The shipped manifest.
    pub fn shipped() -> Manifest {
        Manifest::parse(SHIPPED).expect("audio/cues.toml parses (audio_tests::the_manifest_parses)")
    }

    /// The cue `id` names, or the nearest ancestor that exists: `a.b.c`,
    /// then `a.b`, then `a`.
    pub fn resolve<'a>(&'a self, id: &str) -> Option<(&'a str, &'a Cue)> {
        let mut key = id;
        loop {
            if let Some((k, cue)) = self.cue.get_key_value(key) {
                return Some((k.as_str(), cue));
            }
            key = &key[..key.rfind('.')?];
        }
    }
}

/// A cue's id, as code names it: `cues::ids::CANNON_FIRE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CueId(pub &'static str);

/// The cue ids code names directly, with `ALL` for the tests that hold the
/// code and the manifest to each other. Ids built from data names
/// (`engine.<family>`, `hit.<shot>.<family>.<material>`) are spelled where
/// they are built, by the exhaustive maps in `source.rs`.
macro_rules! cue_ids {
    ($($name:ident = $id:literal;)*) => {
        pub mod ids {
            use super::CueId;
            $(pub const $name: CueId = CueId($id);)*
            pub const ALL: &[CueId] = &[$($name,)*];
        }
    };
}

cue_ids! {
    UI_CLICK = "ui.click";
    UI_REFUSE = "ui.refuse";
    ROUND_START = "round.start";
    ROUND_WON = "round.won";
    ROUND_LOST = "round.lost";
    WAVE_START = "round.wave";
    HIT_MARKER = "ui.hit_marker";
    FRIENDLY_FIRE = "ui.friendly_fire";
    LOCK_ACQUIRED = "warning.lock.acquired";
    DRY_FIRE = "weapon.dry_fire";
    HIT_SHIELD = "hit.shield";
    HULL_INSIDE = "hit.hull_inside";
    BONG_DEATH = "bong.death";
    RICOCHET_WHINE = "hit.ricochet.whine";
    SCRAPE_WATER = "vehicle.scrape.water";
    AIR_HIT_DRONE = "hit.air.drone";
    AIR_HIT_HELI = "hit.air.heli";
    FLYBY_SHELL = "flyby.shell";
    FLYBY_BULLET = "flyby.bullet";
    FLYBY_MISSILE = "flyby.missile";
    TREADS = "vehicle.treads";
    SLAM = "vehicle.slam";
    SCRAPE = "vehicle.scrape";
    RAM = "vehicle.ram";
    SPLASH = "vehicle.splash";
    SPLASH_OUT = "vehicle.splash.out";
    DRIP = "vehicle.drip";
    PUDDLE = "vehicle.ground.puddle";
    RUBBLE = "vehicle.ground.rubble";
    BRUSH = "vehicle.ground.brush";
    PIVOT = "vehicle.pivot";
    COAST_ICE = "vehicle.coast.ice";
    LAVA_SIZZLE = "world.lava.sizzle";
    DOUSE = "world.fire.douse";
    FIRE_HISS = "world.fire.hiss";
    BOOST_FIZZLE = "pickup.boost.fizzle";
    WATER_BLAST = "explosion.water";
    WATER_STEAM = "explosion.water.steam";
    WATER_PLOP = "explosion.water.plop";
    WATER_COLUMN = "explosion.water.column";
    GRENADE_BOUNCE = "weapon.grenades.bounce";
    GRENADE_BOUNCE_WATER = "weapon.grenades.bounce.water";
    GRENADE_BOUNCE_ICE = "weapon.grenades.bounce.ice";
    BONG_SPLASH = "bong.land.water";
    WATER_LAP = "world.water.lap";
    WATER_STREAM = "world.water.stream";
    WATER_ICE = "world.water.ice";
    RAIN_WATER = "world.rain.water";
    GRASS_TRAMPLE = "world.grass.trample";
    MINIGUN_LOOP = "weapon.minigun.loop";
    MINIGUN_SPIN_DOWN = "weapon.minigun.spin_down";
    FLAME_LOOP = "weapon.flamethrower.loop";
    CHARGE_GAUSS = "weapon.gauss_rail.charge";
    GAUSS_READY = "weapon.gauss_rail.ready";
    GAUSS_OVERCHARGED = "weapon.gauss_rail.charge.overcharged";
    VENT_ALARM = "weapon.gauss_rail.vent.alarm";
    ROD_UPLINK = "weapon.rod_from_god.uplink";
    ROD_RETICLE = "weapon.rod_from_god.reticle";
    LOCK_RAIL = "warning.lock.rail";
    LOCK_DRONE = "warning.lock.drone";
    FPV_DIVE = "weapon.fpv_swarm.dive";
    KILL = "ui.kill";
    GRENADE_BEEP = "weapon.grenades.beep";
    MISSILE_LAUNCH = "weapon.missiles.fire";
    MISSILE_MOTOR = "weapon.missiles.motor";
    MISSILE_RELOAD = "weapon.missiles.reload";
    MISSILE_READY = "weapon.missiles.reload.ready";
    MINIGUN_SPIN_UP = "weapon.minigun.spin_up";
    FLAME_START = "weapon.flamethrower.start";
    FLAME_STOP = "weapon.flamethrower.stop";
    FLAME_DRY = "weapon.flamethrower.stop.dry";
    OFFLINE = "weapon.offline";
    ONLINE = "weapon.online";
    SPENT = "weapon.spent";
    SHELL_RELOAD = "weapon.shell.reload";
    SHELL_FULL = "weapon.shell.reload.full";
    FLYBY_PLASMA = "flyby.plasma";
    FLYBY_RAIL = "flyby.rail";
    FLYBY_LASER = "flyby.laser";
    TELL_WINDUP = "warning.windup";
    LOCK_ON = "warning.lock";
    ROD_INCOMING = "warning.rod";
    TESLA_CHARGE = "world.tesla.charge";
    FIRE_LOOP = "world.fire";
    FUSE = "world.fuse";
    RAIN = "world.rain";
    WIND = "world.wind";
    THUNDER = "world.thunder";
    VOLCANO = "world.volcano";
    PORTAL_HUM = "world.portal";
    BONG_BABBLE = "voice.bong";
}
