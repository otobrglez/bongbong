//! On-disk battlefield map format (see docs/map-editor-design.md): which
//! object, if any, sits at each grid cell of a hand-authored or
//! editor-saved battlefield. Deliberately small - a map only overrides the
//! *interior static terrain* (walls, road, the frog, pickup spawn slots).
//! Everything else (border walls, the player fortress, enemy spawns) stays
//! exactly as procedural as it is without a map; `battlefield::spawn_from_map`
//! is the module that actually spawns a map's cells into a round, called
//! from `simulation::Game::init` when `Game::map` is `Some`.
//!
//! Cell coordinates are grid indices, not pixels - `cell_to_world`/
//! `world_to_cell` are the one place that conversion happens, so the editor
//! (`editor.rs`) and the game (`battlefield.rs`) never duplicate it.
//! Coordinates land on exact multiples of `OBSTACLE_GRID_SIZE`, matching
//! how `battlefield.rs`/`obstacle.rs` already place every other static
//! tile.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::framing::MapClass;
use crate::level::{MissionConfig, SpawnConfig};
use crate::frog::Side;
use crate::obstacle::{Drum, Material};
use crate::tower::TowerKind;
use crate::pickup::PickupKind;
use crate::tank::TankKind;
use crate::{OBSTACLE_GRID_SIZE, Position};

/// Current on-disk schema version - bump only on an incompatible format
/// change, and read defensively (reject an unknown future version rather
/// than guessing at it) if that ever happens.
pub const CURRENT_VERSION: u32 = 1;

/// What one grid cell holds. `material`/`pickup` are only ever present
/// alongside their matching `kind` - `#[serde(tag = "kind")]` makes that a
/// property of the TOML shape itself (e.g. `kind = "wall"` with no
/// `material` key fails to parse) rather than something callers have to
/// double-check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CellObject {
    Wall { material: Material },
    Road,
    /// Water: a ground cell painted like road. Its shape comes from the
    /// cells around it (`ground::Layout`): a line of single cells is a
    /// river, a block two or more wide a lake. The rules follow the shape
    /// (docs/water.md): open lake water is deep - a wall to hulls, nothing
    /// to shots - and every other water cell is a ford that slows a hull,
    /// loosens its grip and, in a north/south stream, carries it
    /// downstream; fire never takes on it, frogs hop toward it, and the
    /// AI's router prices a ford and walls off the deep. Not an
    /// `Obstacle`: deep water is static colliders spawned by `Game::init`.
    Water,
    Frog,
    /// Player 1's start - singleton like `Frog`. A map without one spawns
    /// player 1 at the nearest free cell to the centre.
    Start,
    /// Player 2's start in a two-player round - singleton like `Start`.
    /// Ignored in a single-player round; a map without one places player
    /// 2 beside player 1 (`Game::init`).
    #[serde(rename = "start2")]
    Start2,
    Pickup { pickup: PickupKind },
    /// The enemy side's frog (Hunt mission) - singleton like `Frog`.
    /// Ignored by missions without an enemy frog.
    #[serde(rename = "enemy_frog")]
    EnemyFrog,
    /// A wave roll-in gate: must sit on a nav-grid edge cell. A map with
    /// any gate cells uses only those; otherwise gates are scanned from the
    /// wall layout each wave.
    Gate,
    /// The three destructible props (docs/sandbags-barrels-fences.md): each
    /// spawns an `Obstacle` of the matching `Material`, variant rolled per
    /// tile at spawn.
    Sandbag,
    /// An oil barrel. `drum` pins its kind (`drum = "oil"` leaves a
    /// burning pool, `drum = "fuel"` goes off harder and launches when
    /// chained - `obstacle::Drum`); absent, the kind is rolled per tile at
    /// spawn like the other props' variants, and a file without the key
    /// reads exactly as it did before the key existed.
    Barrel {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        drum: Option<Drum>,
    },
    Fence,
    /// A range board (docs/range-target-prd.md): a bullseye on a stand,
    /// a solid wooden prop placed to be shot at - it wears through four
    /// stages and splinters, or burns when fire reaches it.
    Target,
    /// An oil trail: a ground cell that is *not* solid and has no nav
    /// effect until a blast or a burning neighbour lights it, after which
    /// the fire runs along it cell by cell (`oil_trail_cells_per_second`),
    /// hurting whatever drives over it and setting off any drum it reaches.
    /// The author's fuse (docs/barrel-explosion-variety.md section D).
    Oil,
    /// The two tree species (docs/TREES_SPEC.md). Solid like a prop, but
    /// drawn from 48px cells so the canopy overhangs the cell it stands
    /// in; both burn, and a tank can flatten one by driving at it.
    Tree,
    Pine,
    /// Tall grass: cover a tank can sit in. Deliberately **not** solid and
    /// deliberately not an `Obstacle` - `Game::nav_grid` feeds every
    /// obstacle into pathfinding with no material filter, so anything that
    /// is one is impassable to the AI and a wall to `maplint`. Grass you
    /// drive through has to be its own light entity (see `grass.rs`).
    #[serde(rename = "tall_grass")]
    TallGrass,
    /// A teleport portal (docs/teleporting.md): the *anchor* cell of a
    /// ~3x3-cell spiral tanks drive into to be moved to another portal.
    /// Multi-instance, deliberately **not** solid and not an `Obstacle`
    /// (the same reasoning as `TallGrass`): the art spills over the
    /// neighbouring cells, which stay paintable. A map with fewer than two
    /// portals has an inert network - nothing teleports and the round
    /// draws none of them.
    Portal,
    /// The defence towers (docs/defence-towers-prd.md): a solid tile that
    /// fights for `side` - the player's when the key is absent, so a file
    /// without it reads the way the builder's plain tool writes it.
    Tesla {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        side: Option<Side>,
    },
    #[serde(rename = "gun_tower")]
    GunTower {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        side: Option<Side>,
    },
    #[serde(rename = "bio_slush")]
    BioSlush {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        side: Option<Side>,
    },
    /// Lava (docs/volcano.md): a ground cell painted like water, its shape
    /// read the same way (`ground::Layout`) - a line of single cells is a
    /// stream, a block two or more wide a lake. A stream is a burning
    /// ford: a hull crosses it at a wade and burns the whole way unless it
    /// carries the heat shield; a lake's open middle is deep, a wall to
    /// hulls and nothing to shots. It flows away from the volcano it leaves
    /// (`lava::LavaLayout`) and radiates heat onto its banks. Not an
    /// `Obstacle`, like water.
    Lava,
    /// A volcano (docs/volcano.md): the crater's cell, the centre of a
    /// cone `volcano::FOOTPRINT` cells across (`volcano::footprint`), each
    /// of whose cells is a permanent `Material::Volcano` tile - it stops
    /// shots, hulls and the nav grid like iron. It sleeps, rumbles, erupts
    /// and cools on a clock of its own and throws lava bombs while it
    /// erupts. Multi-instance, each on its own clock.
    Volcano,
    /// A lamp post (docs/volcano.md): a solid tile that lights the ground
    /// round it at night (`weather::lights_in`) - and shows whoever stands
    /// in its light to the enemy at full range (`Game::sight_on`). One shot
    /// puts it out for good.
    Lamp,
    /// A training door (docs/training-stage.md): a solid, permanent tile
    /// that closes a pen until beat `beat` (1-based, in the map's
    /// `[[training.beat]]` order) is done, when the round takes every door
    /// of that beat away. Multi-instance: a door is a run of these cells.
    Door { beat: u8 },
    /// A training flag (docs/training-stage.md): not solid, taken by the
    /// first seat whose hull reaches it and counted by a beat's `flags`.
    Flag,
}

impl CellObject {
    /// The obstacle material this cell spawns, if it spawns a solid tile.
    pub fn material(&self) -> Option<Material> {
        match self {
            CellObject::Wall { material } => Some(*material),
            CellObject::Sandbag => Some(Material::Sandbag),
            CellObject::Barrel { .. } => Some(Material::Barrel),
            CellObject::Fence => Some(Material::Fence),
            CellObject::Target => Some(Material::Target),
            CellObject::Tree => Some(Material::Tree),
            CellObject::Pine => Some(Material::Pine),
            CellObject::Tesla { .. } => Some(Material::Tesla),
            CellObject::GunTower { .. } => Some(Material::GunTower),
            CellObject::BioSlush { .. } => Some(Material::BioSlush),
            CellObject::Volcano => Some(Material::Volcano),
            CellObject::Lamp => Some(Material::Lamp),
            CellObject::Door { .. } => Some(Material::Door),
            _ => None,
        }
    }

    /// The tower a cell places and the side it fights for, if it is a
    /// tower: the player's unless the cell says `side = "enemy"`.
    pub fn tower(&self) -> Option<(TowerKind, Side)> {
        let (kind, side) = match *self {
            CellObject::Tesla { side } => (TowerKind::Tesla, side),
            CellObject::GunTower { side } => (TowerKind::Gun, side),
            CellObject::BioSlush { side } => (TowerKind::Bio, side),
            _ => return None,
        };
        Some((kind, side.unwrap_or(Side::Player)))
    }

    /// The cell that places a `kind` tower fighting for `side`. The
    /// player's side is written without the key, the way a hand-authored
    /// file leaves it out.
    pub fn for_tower(kind: TowerKind, side: Side) -> CellObject {
        let side = (side == Side::Enemy).then_some(Side::Enemy);
        match kind {
            TowerKind::Tesla => CellObject::Tesla { side },
            TowerKind::Gun => CellObject::GunTower { side },
            TowerKind::Bio => CellObject::BioSlush { side },
        }
    }

    /// The drum kind a barrel cell pins, if it is a barrel and pins one.
    pub fn drum(&self) -> Option<Drum> {
        match self {
            CellObject::Barrel { drum } => *drum,
            _ => None,
        }
    }

    /// Spawns a solid tile a tank can't stand on (a wall or a prop).
    pub fn is_solid(&self) -> bool {
        self.material().is_some()
    }

    /// The cell that places a standalone solid of `material` - a prop or a
    /// tree (`None` for wall materials, which are `Wall { material }`).
    pub fn prop(material: Material) -> Option<CellObject> {
        match material {
            Material::Sandbag => Some(CellObject::Sandbag),
            Material::Barrel => Some(CellObject::Barrel { drum: None }),
            Material::Fence => Some(CellObject::Fence),
            Material::Target => Some(CellObject::Target),
            Material::Tree => Some(CellObject::Tree),
            Material::Pine => Some(CellObject::Pine),
            Material::Lamp => Some(CellObject::Lamp),
            _ => None,
        }
    }
}

/// The battlefield's look: which retint of the ground tileset and which
/// tall-grass sheet a round draws with (TOML: a top-level `theme =
/// "desert"`). Purely presentational - the simulation, the nav grid and
/// the linter never read it - so two maps that differ only in theme play
/// identically. Absent means `Grass`, so every older file parses
/// unchanged. Both sheets of every theme ship in every build
/// (`ground_texture_path`/`grass_texture_path` name them), and `app.rs`
/// picks the pair by the live map each frame, so the builder can switch a
/// map's theme and see it at once. New themes (ice is the obvious next
/// one) are one variant plus one retint curve and one grass species set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Grass,
    Desert,
}

impl Theme {
    /// Every theme, in the order the builder's THEME row cycles them.
    pub const ALL: [Theme; 2] = [Theme::Grass, Theme::Desert];

    /// The TOML spelling, also the dev server's and the builder's.
    pub fn name(self) -> &'static str {
        match self {
            Theme::Grass => "grass",
            Theme::Desert => "desert",
        }
    }

    pub fn parse(s: &str) -> Option<Theme> {
        Theme::ALL.iter().copied().find(|t| t.name() == s)
    }

    /// The retinted Puny World tileset `ground.rs` blits from
    /// (`tools/retint_ground.py`, one file per theme).
    pub fn ground_texture_path(self) -> &'static str {
        match self {
            Theme::Grass => "static/punyworld/punyworld-overworld-tileset.png",
            Theme::Desert => "static/punyworld/punyworld-overworld-tileset-desert.png",
        }
    }

    /// The tall-grass sheet (`tools/spritegen/gen_grass.py`, one per theme).
    pub fn grass_texture_path(self) -> &'static str {
        match self {
            Theme::Grass => "static/nature_sheet.png",
            Theme::Desert => "static/nature_sheet_desert.png",
        }
    }

    /// Whether `ground::build` drifts the pack's sand tiles over the open
    /// floor. Only where the retint makes them a near tone of the fill
    /// (the desert's hardpan): on the grass retint they are a khaki that
    /// reads as dirt patches, which the object-driven road placement
    /// deliberately replaced (docs/GROUND_SPEC.md §5).
    pub fn drifts(self) -> bool {
        matches!(self, Theme::Desert)
    }

    /// The tint a tall-grass cell's flecks and the builder's grass icon
    /// use for the floor under the tufts: the theme's fill tone.
    pub fn floor_color(self) -> (u8, u8, u8) {
        match self {
            Theme::Grass => (0x61, 0x95, 0x41),
            Theme::Desert => (0xCC, 0xB3, 0x85),
        }
    }
}

fn is_default_theme(t: &Theme) -> bool {
    *t == Theme::Grass
}

/// The sky over the battlefield (TOML: the top-level `weather` key - one
/// of a map's `Skies` -, docs/weather.md): drawn, and part of the
/// rules - shorter enemy sight at night and in fog, less grip in the
/// rain, the water frozen in the snow, gusts in a sandstorm
/// (`weather::sight_factor` and its neighbours). `Game::init` settles the
/// round's sky once; a clear one plays exactly as a map without the key.
/// What each one looks like is `weather::Look::of`; `Random` is a sky
/// picked by the round's seed (`weather::random_sky`), on a map the set
/// of every sky (`Skies::ALL`); the `weather_override` knob (`--weather`, the
/// web page's `?weather=`) puts one sky over every local round without
/// editing any map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Weather {
    #[default]
    Clear,
    Night,
    Dusk,
    Rain,
    /// A thunderstorm: heavy rain at night, with lightning.
    Storm,
    Fog,
    Sandstorm,
    Snow,
    HeatHaze,
    /// One of `SKIES`, picked afresh by every round's seed: the same seed
    /// always brings the same sky, and every replica of a room draws the
    /// room's (`weather::in_force`).
    Random,
}

impl Weather {
    /// Every sky a round can be drawn under - everything but `Random`, in
    /// `ALL`'s order. What `Random` picks from.
    pub const SKIES: [Weather; 9] = [
        Weather::Clear,
        Weather::Night,
        Weather::Dusk,
        Weather::Rain,
        Weather::Storm,
        Weather::Fog,
        Weather::Sandstorm,
        Weather::Snow,
        Weather::HeatHaze,
    ];

    /// Every weather; a weather's index here is its `weather_override`
    /// value.
    pub const ALL: [Weather; 10] = [
        Weather::Clear,
        Weather::Night,
        Weather::Dusk,
        Weather::Rain,
        Weather::Storm,
        Weather::Fog,
        Weather::Sandstorm,
        Weather::Snow,
        Weather::HeatHaze,
        Weather::Random,
    ];

    /// The TOML spelling, also the dev server's, the command line's and
    /// the builder's.
    pub fn name(self) -> &'static str {
        match self {
            Weather::Clear => "clear",
            Weather::Night => "night",
            Weather::Dusk => "dusk",
            Weather::Rain => "rain",
            Weather::Storm => "storm",
            Weather::Fog => "fog",
            Weather::Sandstorm => "sandstorm",
            Weather::Snow => "snow",
            Weather::HeatHaze => "heat_haze",
            Weather::Random => "random",
        }
    }

    pub fn parse(s: &str) -> Option<Weather> {
        Weather::ALL.iter().copied().find(|w| w.name() == s)
    }

    /// This weather's position in `ALL`, the `weather_override` value that
    /// forces it.
    pub fn index(self) -> usize {
        Weather::ALL.iter().position(|w| *w == self).expect("every weather is in ALL")
    }
}

/// The skies a map may be fought under (TOML: the top-level `weather`
/// key, the MAP panel's WEATHER checklist; docs/weather.md): every round
/// takes one of them, picked by its seed (`weather::pick_sky`). None at
/// all is a clear sky, as is `clear` alone, and is not written back. One
/// sky is written as its name (`weather = "night"`), every sky as
/// `"random"`, any other set as a list in `Weather::SKIES` order
/// (`weather = ["night", "rain"]`), so a map with one sky or none
/// re-saves byte for byte - and keeps its clear stamp's `revision`.
/// `clear` may be one of several (`["clear", "rain"]`: a dry round or a
/// wet one).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Skies(u16);

impl Skies {
    /// No sky named: a clear one.
    pub const CLEAR: Skies = Skies(0);

    /// Every sky: what `random` means.
    pub const ALL: Skies = Skies((1 << Weather::SKIES.len()) - 1);

    /// The set from its bits over `Weather::SKIES`, `clear` alone folded
    /// into none, so the two are one value.
    const fn from_bits(bits: u16) -> Skies {
        Skies(if bits == 1 { 0 } else { bits & Skies::ALL.0 })
    }

    fn bit(sky: Weather) -> Option<u16> {
        Weather::SKIES.iter().position(|w| *w == sky).map(|i| 1 << i)
    }

    /// One weather as a set: `random` is every sky.
    pub fn of(weather: Weather) -> Skies {
        match Skies::bit(weather) {
            Some(bit) => Skies::from_bits(bit),
            None => Skies::ALL,
        }
    }

    /// Whether `sky` is one a round may take: `clear` while none is
    /// named. Never `random`.
    pub fn contains(self, sky: Weather) -> bool {
        match Skies::bit(sky) {
            Some(1) => self.0 == 0 || self.0 & 1 != 0,
            Some(bit) => self.0 & bit != 0,
            None => false,
        }
    }

    /// The set with `sky` in or out of it - the WEATHER checklist's tap.
    /// Taking `clear` off a set holding nothing else leaves it clear;
    /// `random` names no single sky and changes nothing.
    pub fn toggled(self, sky: Weather) -> Skies {
        let Some(bit) = Skies::bit(sky) else { return self };
        Skies::from_bits(if self.contains(sky) { self.0 & !bit } else { self.0 | bit })
    }

    /// The skies a round may take, in `Weather::SKIES` order: `clear`
    /// alone for none.
    pub fn skies(self) -> Vec<Weather> {
        if self.0 == 0 {
            return vec![Weather::Clear];
        }
        Weather::SKIES.iter().enumerate().filter(|(i, _)| self.0 & (1 << i) != 0).map(|(_, w)| *w).collect()
    }

    /// The one weather this set is, where one name says it: a sky,
    /// `clear` for none, `random` for every sky.
    pub fn single(self) -> Option<Weather> {
        if self == Skies::ALL {
            return Some(Weather::Random);
        }
        match self.skies().as_slice() {
            [sky] => Some(*sky),
            _ => None,
        }
    }

    pub fn is_clear(self) -> bool {
        self.0 == 0
    }

    /// The names as the map file, the dev server and the tools spell
    /// them: one name where `single` has one, else the list.
    pub fn names(self) -> Vec<&'static str> {
        match self.single() {
            Some(w) => vec![w.name()],
            None => self.skies().into_iter().map(Weather::name).collect(),
        }
    }

    /// The set a list of names spells - `random` adds every sky, an
    /// empty list is clear; `Err` names the first word that is no
    /// weather.
    pub fn parse<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<Skies, &'a str> {
        let mut bits = 0;
        for name in names {
            let weather = Weather::parse(name).ok_or(name)?;
            bits |= Skies::bit(weather).unwrap_or(Skies::ALL.0);
        }
        Ok(Skies::from_bits(bits))
    }
}

impl From<Weather> for Skies {
    fn from(weather: Weather) -> Skies {
        Skies::of(weather)
    }
}

impl Serialize for Skies {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.single() {
            Some(w) => s.serialize_str(w.name()),
            None => self.names().serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for Skies {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Skies, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Spelling {
            One(String),
            Many(Vec<String>),
        }
        let names = match Spelling::deserialize(d)? {
            Spelling::One(name) => vec![name],
            Spelling::Many(names) => names,
        };
        Skies::parse(names.iter().map(String::as_str)).map_err(|bad| {
            let known: Vec<&str> = Weather::ALL.iter().map(|w| w.name()).collect();
            serde::de::Error::custom(format!("unknown weather `{bad}`, expected one of {}", known.join(", ")))
        })
    }
}

fn is_default_weather(w: &Skies) -> bool {
    w.is_clear()
}

/// How the map asks to be shown (TOML: a top-level `view = "whole"` or
/// `view = "follow"`; docs/large-maps-follow-camera.md §1): whole on every
/// screen like an arena, or followed by a camera like a field map, over
/// what its size says (`framing::MapClass::by_size`). Absent means by
/// size, which is not written back, so every older file parses and
/// re-saves unchanged. `MapFile::class` is the answer with the key applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MapView {
    /// Shown whole, however big the map is.
    Whole,
    /// Followed by a camera, however small the map is.
    Follow,
}

impl MapView {
    /// Both choices, whole first.
    pub const ALL: [MapView; 2] = [MapView::Whole, MapView::Follow];

    /// The TOML spelling.
    pub fn name(self) -> &'static str {
        match self {
            MapView::Whole => "whole",
            MapView::Follow => "follow",
        }
    }

    pub fn parse(s: &str) -> Option<MapView> {
        MapView::ALL.iter().copied().find(|v| v.name() == s)
    }

    /// The class this choice puts a map in.
    pub fn class(self) -> MapClass {
        match self {
            MapView::Whole => MapClass::Arena,
            MapView::Follow => MapClass::Field,
        }
    }
}

/// A saved battlefield layout. Keys are `"<col>,<row>"` grid-cell strings
/// (TOML tables require string keys) - only occupied cells are stored, so a
/// mostly-empty map stays a small file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MapFile {
    pub version: u32,
    #[serde(default)]
    pub cells: HashMap<String, CellObject>,
    /// Default number of enemy tanks to spawn on this map, unless overridden
    /// at runtime by `-e`/`--enemies` (see `main.rs`). `Some(0)` is a
    /// sandbox: no enemies, and the round never ends by wreck count. `None` (the default -
    /// absent from a map's TOML, `#[serde(default)]` so older map files
    /// still parse) means "no map-level default", in which case `Game::init`
    /// falls back to its usual random `ENEMY_COUNT_MIN..=ENEMY_COUNT_MAX`
    /// roll, same as today.
    #[serde(default)]
    pub tanks: Option<u32>,
    /// The chassis the player spawns in on this map (TOML: a top-level
    /// `tank = "titan"`, spelled exactly like `--tank`'s own values - see
    /// `tank::TankKind`). `None` (the default - absent from a map's TOML,
    /// `#[serde(default)]` so older map files still parse) means "no
    /// map-level preference", leaving the player's chassis to the
    /// `player_tank` tuning knob or, failing that, `Game::init`'s random
    /// roll. `--tank` on the command line outranks this.
    #[serde(default)]
    pub tank: Option<TankKind>,
    /// Player 2's chassis in a two-player round (TOML: a top-level
    /// `tank2 = "scout"`), the same way `tank` names player 1's. `None`
    /// means a random roll; `--tank2` outranks it.
    #[serde(default)]
    pub tank2: Option<TankKind>,
    /// The look (TOML: a top-level `theme = "grass"|"desert"`, the MAP
    /// panel's THEME row). Absent means grass, and grass is not written
    /// back, so older files re-save unchanged.
    #[serde(default, skip_serializing_if = "is_default_theme")]
    pub theme: Theme,
    /// The skies a round may take (TOML: a top-level `weather = "night"`
    /// or `weather = ["night", "rain"]`, the MAP panel's WEATHER
    /// checklist). Absent means clear, and clear is not written back.
    #[serde(default, skip_serializing_if = "is_default_weather")]
    pub weather: Skies,
    /// How the map is shown (TOML: a top-level `view = "whole"|"follow"`).
    /// `None`, the key absent, leaves it to the map's size (`class`) and is
    /// not written back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<MapView>,
    /// When night falls (TOML: a top-level `nightfall = 90.0`, seconds of
    /// the round clock, docs/volcano.md): the round opens under the map's
    /// sky and darkens into night over the `nightfall_seconds` before it
    /// (`weather::nightfall_mix`), its rules turning to the night's at
    /// that moment (`Game::fall_night`). `None`, the key absent, keeps the
    /// map's sky the whole round and is not written back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nightfall: Option<f32>,
    /// The `[mission]` table - what ends the round (docs/maps-to-levels.md).
    /// Absent means Protect.
    #[serde(default)]
    pub mission: MissionConfig,
    /// The `[spawn]` table - how enemies arrive. Absent means the band
    /// plan (`tanks` / `--enemies` / a random roll).
    #[serde(default)]
    pub spawn: SpawnConfig,
    /// The battlefield's size in cells, `size = [cols, rows]` (TOML: a
    /// top-level array; rows may be fractional, the shipped 40 x 22.5 is
    /// `[40, 22.5]`). This is the *field* - the world the simulation runs
    /// in, the walls' inner faces, the ground - and every player in a
    /// match shares it; the window and the screen only decide how large it
    /// is drawn (`view::View`). `None` means `DEFAULT_SCREEN_WIDTH` x
    /// `DEFAULT_SCREEN_HEIGHT`, so older files parse unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<(f32, f32)>,
    /// The clear check's stamp (TOML: a `[cleared]` table the builder's
    /// SAVE writes): the revision of this map its author won from the
    /// builder's plain PLAY, and the par. It counts only while it names the
    /// map's own revision (`cleared_par`) - a map edited since, by hand or
    /// in the builder, is a new revision nobody has won. Absent, and not
    /// written back, until a revision is cleared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleared: Option<Cleared>,
    /// A training map's script (docs/training-stage.md): its beats in
    /// order, written last as `[[training.beat]]` tables. Absent on every
    /// other map, which runs none of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub training: Option<crate::training::Training>,
    /// Where this map came from, for display only: the file stem when
    /// `load` read it, `"default"` for the embedded map, `None` for text
    /// handed over directly (the dev server's inline `map_toml`). Never
    /// written to disk.
    #[serde(skip)]
    pub name: Option<String>,
}

/// The most cells a map spans on either side: what an online round can
/// carry. Positions travel as quarter pixels in an `i16`, which saturates
/// at 8191 px (`net::wire::POSITION_MAX_PX`) - 250 cells and the tank
/// length a wave tank starts beyond the edge - and a cell index as a
/// `u16`, which holds 250 x 250 cells.
pub const MAX_SIDE_CELLS: f32 = 250.0;

/// The fewest cells a map spans on either side: room for a tank to stand
/// and wander a cell clear of the border on each side. Narrower, the
/// enemies' patrol had no ground to pick a point in and the round
/// panicked. The builder keeps a larger floor of its own
/// (`editor::MIN_MAP_CELLS`); this is what a hand-written or sent map is
/// held to.
pub const MIN_SIDE_CELLS: f32 = 8.0;

/// The most cells a map drawn whole (`view = "whole"`) may span on either
/// side: its round draws the field into a target a texel per world pixel,
/// and 4096 texels a side is what every GPU the game runs on holds - the
/// establishing shot's bound too (`establish::MAX_TEXELS`). A larger map
/// asking to be seen whole is followed instead.
pub const WHOLE_MAX_CELLS: f32 = 128.0;

fn cell_key(col: i32, row: i32) -> String {
    format!("{col},{row}")
}

fn parse_cell_key(key: &str) -> Option<(i32, i32)> {
    let (c, r) = key.split_once(',')?;
    Some((c.trim().parse().ok()?, r.trim().parse().ok()?))
}

/// Grid cell (col, row) -> world-space center position - the same "position
/// is an exact multiple of the grid" convention `sample_structure_positions`/
/// `spawn_player_fortress` already place every wall tile on.
pub fn cell_to_world(col: i32, row: i32) -> Position {
    Position::new(col as f32 * OBSTACLE_GRID_SIZE, row as f32 * OBSTACLE_GRID_SIZE)
}

/// World-space position -> nearest grid cell - the inverse of
/// `cell_to_world`, used by the editor to turn a mouse position into the
/// cell it should place/erase.
pub fn world_to_cell(pos: Position) -> (i32, i32) {
    (
        (pos.x / OBSTACLE_GRID_SIZE).round() as i32,
        (pos.y / OBSTACLE_GRID_SIZE).round() as i32,
    )
}

impl MapFile {
    pub fn new() -> Self {
        MapFile {
            version: CURRENT_VERSION,
            cells: HashMap::new(),
            tanks: None,
            tank: None,
            tank2: None,
            theme: Theme::default(),
            weather: Skies::CLEAR,
            view: None,
            nightfall: None,
            mission: MissionConfig::default(),
            spawn: SpawnConfig::default(),
            size: None,
            cleared: None,
            training: None,
            name: None,
        }
    }

    /// The map's revision (docs/large-maps-patterns.md, "Clear check
    /// before sharing"): a 64-bit FNV-1a hash of its TOML in one canonical
    /// form - every table's keys sorted, the clear stamp left out, since
    /// it names a revision and cannot be part of one, and the display name
    /// with it, which is never written. The same map is the same revision
    /// in every build and on every platform, and any edit to a cell or a
    /// setting is another one.
    pub fn revision(&self) -> u64 {
        let text = toml::Value::try_from(self)
            .ok()
            .and_then(|mut value| {
                if let Some(table) = value.as_table_mut() {
                    table.remove("cleared");
                }
                toml::to_string(&sorted_keys(value)).ok()
            })
            .unwrap_or_default();
        fnv1a(text.as_bytes())
    }

    /// The par this map's stamp holds for it, in seconds: `None` without a
    /// stamp or with one for another revision.
    pub fn cleared_par(&self) -> Option<f64> {
        let stamp = self.cleared.as_ref()?;
        (stamp.revision == revision_text(self.revision())).then_some(stamp.par)
    }

    /// Whether a room may be given this map, by the clear check: one of
    /// `SHIPPED_MAPS` as it ships - what the lobby's own HOST sends by
    /// name - or a map whose stamp says it was won as it stands
    /// (`cleared_par`).
    ///
    /// A training map never is: its script plays one seat.
    pub fn hostable(&self) -> bool {
        self.training.is_none() && (self.cleared_par().is_some() || shipped_revisions().contains(&self.revision()))
    }
}

/// The clear check's stamp in a map file (`MapFile::cleared`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cleared {
    /// The revision that was won (`MapFile::revision`), as 16 hex digits.
    pub revision: String,
    /// The par: the round clock when it was won, in seconds to a tenth.
    pub par: f64,
}

impl Cleared {
    /// The stamp for `revision` won in `par` seconds.
    pub fn new(revision: u64, par: f64) -> Cleared {
        Cleared { revision: revision_text(revision), par }
    }
}

/// `value` with every table's keys in sorted order: the canonical form a
/// revision hashes, whichever map the toml crate keeps tables in - sorted
/// by default, in insertion order under its `preserve_order` feature, which
/// would hand the cells over in a `HashMap`'s order, another each run.
fn sorted_keys(value: toml::Value) -> toml::Value {
    match value {
        toml::Value::Table(table) => {
            let mut entries: Vec<(String, toml::Value)> = table.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            toml::Value::Table(entries.into_iter().map(|(key, value)| (key, sorted_keys(value))).collect())
        }
        toml::Value::Array(items) => toml::Value::Array(items.into_iter().map(sorted_keys).collect()),
        other => other,
    }
}

/// The 64-bit FNV-1a hash of `bytes`: a map's revision is this of its
/// canonical TOML, and the builder's thumbnails are kept by this of the
/// text a map is read from.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// A revision as a stamp spells it: 16 lowercase hex digits.
pub fn revision_text(revision: u64) -> String {
    format!("{revision:016x}")
}

/// The revisions of `SHIPPED_MAPS` as they ship, worked out once.
fn shipped_revisions() -> &'static [u64] {
    static REVISIONS: std::sync::OnceLock<Vec<u64>> = std::sync::OnceLock::new();
    REVISIONS.get_or_init(|| SHIPPED_MAPS.iter().filter_map(|(_, text)| MapFile::from_toml_str(text).ok()).map(|map| map.revision()).collect())
}

impl MapFile {

    /// The battlefield size in world pixels this map asks for: `size`
    /// times the cell, or the default field when the map names none.
    pub fn field_size(&self) -> (f32, f32) {
        match self.size {
            Some((cols, rows)) if cols > 0.0 && rows > 0.0 => (cols * OBSTACLE_GRID_SIZE, rows * OBSTACLE_GRID_SIZE),
            _ => (crate::DEFAULT_SCREEN_WIDTH as f32, crate::DEFAULT_SCREEN_HEIGHT as f32),
        }
    }

    /// Whether the map is shown whole or followed by a camera
    /// (docs/large-maps-follow-camera.md §1, §15): its `view` key when it
    /// has one - `whole` only while the map fits `WHOLE_MAX_CELLS` both
    /// ways - else its size, an arena up to 36 x 18 cells and a field map
    /// past that (`framing::MapClass::by_size`).
    pub fn class(&self) -> MapClass {
        let (width, height) = self.field_size();
        let (cols, rows) = (width / OBSTACLE_GRID_SIZE, height / OBSTACLE_GRID_SIZE);
        match self.view {
            Some(MapView::Whole) if cols > WHOLE_MAX_CELLS || rows > WHOLE_MAX_CELLS => MapClass::Field,
            Some(view) => view.class(),
            None => MapClass::by_size(cols, rows),
        }
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading map {}: {e}", path.display()))?;
        let mut map = Self::from_toml_str(&text).map_err(|e| format!("parsing map {}: {e}", path.display()))?;
        map.name = path.file_stem().map(|s| s.to_string_lossy().into_owned());
        Ok(map)
    }

    /// Parse already-in-memory TOML text rather than reading it from a path -
    /// `load` itself uses this, and it's also what lets `main.rs` embed
    /// `maps/default.toml` into the binary at compile time (`include_str!`)
    /// instead of reading it from disk at startup. That matters because
    /// this project's only two distribution paths - the wasm/web build's
    /// emscripten virtual filesystem and cargo-dist's native release
    /// archives - both currently bundle only `static/` (see CLAUDE.md's Web
    /// / wasm build and Releases sections), not `maps/`; a disk read for
    /// the game's own default battlefield would 404/panic in either build.
    /// The map editor's own Load panel (native, dev-only) still reads
    /// `maps/*.toml` from disk via `load` - only the game's built-in
    /// fallback needed to stop depending on that.
    pub fn from_toml_str(text: &str) -> Result<Self, String> {
        let map = Self::from_toml_str_any_size(text)?;
        if let Some((cols, rows)) = map.size {
            if !(cols.is_finite() && rows.is_finite() && cols <= MAX_SIDE_CELLS && rows <= MAX_SIDE_CELLS) {
                return Err(format!("size = [{cols}, {rows}] is past the largest map, {MAX_SIDE_CELLS} cells a side"));
            }
            if cols < MIN_SIDE_CELLS || rows < MIN_SIDE_CELLS {
                return Err(format!("size = [{cols}, {rows}] is under the smallest map, {MIN_SIDE_CELLS} cells a side"));
            }
        }
        Ok(map)
    }

    /// The map in `text`, its version checked and its `size` taken as
    /// written: what a builder stamp is read with (`editor::select`), whose
    /// `size` is its own extent rather than a field's, so a playable map's
    /// bounds do not hold it. Everything that plays a map reads it through
    /// `from_toml_str`.
    pub fn from_toml_str_any_size(text: &str) -> Result<Self, String> {
        let map: MapFile = toml::from_str(text).map_err(|e| format!("{e}"))?;
        if map.version > CURRENT_VERSION {
            return Err(format!(
                "map is version {}, newer than this build supports ({CURRENT_VERSION})",
                map.version
            ));
        }
        Ok(map)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("creating {}: {e}", parent.display()))?;
        }
        let text = self.to_toml_string()?;
        std::fs::write(path, text).map_err(|e| format!("writing {}: {e}", path.display()))
    }

    /// The map as TOML text, in the shape `save` writes (one table per
    /// cell) - what `from_toml_str` parses back.
    pub fn to_toml_string(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| format!("serializing map: {e}"))
    }

    pub fn cell(&self, col: i32, row: i32) -> Option<&CellObject> {
        self.cells.get(&cell_key(col, row))
    }

    pub fn set_cell(&mut self, col: i32, row: i32, object: CellObject) {
        self.cells.insert(cell_key(col, row), object);
    }

    pub fn clear_cell(&mut self, col: i32, row: i32) {
        self.cells.remove(&cell_key(col, row));
    }

    /// Every placed cell as `(col, row, &CellObject)`, in a fixed
    /// row-then-column order - silently skips a malformed key rather than
    /// panicking, defensive against a hand-edited file (this module's own
    /// writer never produces one).
    ///
    /// The sort is load-bearing for seeded-round determinism, not
    /// cosmetics: `cells` is a `HashMap`, whose iteration order varies per
    /// process, and `battlefield::spawn_from_map` both consumes RNG per
    /// Wood tile and spawns hecs entities while walking this iterator - so
    /// unordered iteration would make the RNG stream and the entity
    /// creation order (hence every later `world.query` iteration order)
    /// differ run-to-run even under a fixed seed (see
    /// docs/gameplay-verification-design.md §1.3). Sorting the parsed
    /// `(row, col)` tuples here covers every consumer at once; sorting the
    /// raw string keys instead would be a trap (`"10,2" < "2,3"`
    /// lexicographically), which is also why `cells` isn't simply a
    /// `BTreeMap`. Maps are a few hundred cells, so the collect+sort cost
    /// is irrelevant next to what callers do with the result.
    pub fn iter_cells(&self) -> impl Iterator<Item = (i32, i32, &CellObject)> {
        let mut cells: Vec<(i32, i32, &CellObject)> = self
            .cells
            .iter()
            .filter_map(|(key, obj)| parse_cell_key(key).map(|(col, row)| (col, row, obj)))
            .collect();
        cells.sort_by_key(|&(col, row, _)| (row, col));
        cells.into_iter()
    }

    /// The map's one frog cell, if it placed one - see "Frog: singleton
    /// enforcement" in docs/map-editor-design.md; nothing in `MapFile`
    /// itself enforces the singleton (that's the editor's job when placing
    /// one), this just finds whichever one is there.
    pub fn frog_cell(&self) -> Option<(i32, i32)> {
        self.iter_cells()
            .find(|(_, _, obj)| matches!(obj, CellObject::Frog))
            .map(|(col, row, _)| (col, row))
    }

    /// The map's one player-start cell, if it placed one - same singleton
    /// convention as `frog_cell` (enforced by the editor when placing one,
    /// not by this type). `Game::init` reads this directly (rather than
    /// waiting on `battlefield::spawn_from_map`'s output) since the player
    /// is spawned before map terrain is; when a map places no start cell,
    /// `Game::init` falls back to `nearest_free_cell` around the
    /// battlefield's center instead.
    pub fn start_cell(&self) -> Option<(i32, i32)> {
        self.iter_cells()
            .find(|(_, _, obj)| matches!(obj, CellObject::Start))
            .map(|(col, row, _)| (col, row))
    }

    /// The map's one player-2 start cell, if it placed one - same
    /// singleton convention as `start_cell`. Read only in a two-player
    /// round.
    pub fn start2_cell(&self) -> Option<(i32, i32)> {
        self.iter_cells()
            .find(|(_, _, obj)| matches!(obj, CellObject::Start2))
            .map(|(col, row, _)| (col, row))
    }

    /// The map's one enemy-frog cell (Hunt mission), if it placed one -
    /// same singleton convention as `frog_cell`.
    pub fn enemy_frog_cell(&self) -> Option<(i32, i32)> {
        self.iter_cells()
            .find(|(_, _, obj)| matches!(obj, CellObject::EnemyFrog))
            .map(|(col, row, _)| (col, row))
    }

    /// Every explicit wave gate cell, in `iter_cells` order.
    pub fn gate_cells(&self) -> Vec<(i32, i32)> {
        self.iter_cells()
            .filter(|(_, _, obj)| matches!(obj, CellObject::Gate))
            .map(|(col, row, _)| (col, row))
            .collect()
    }

    /// Every portal anchor cell, in `iter_cells` order - the order
    /// `Game::portals` keeps, so a portal's index is stable across the
    /// map, the round and the dev server.
    pub fn portal_cells(&self) -> Vec<(i32, i32)> {
        self.iter_cells()
            .filter(|(_, _, obj)| matches!(obj, CellObject::Portal))
            .map(|(col, row, _)| (col, row))
            .collect()
    }

    /// Every volcano's crater cell, in `iter_cells` order - the order
    /// `Game::volcanoes` keeps.
    pub fn volcano_cells(&self) -> Vec<(i32, i32)> {
        self.iter_cells()
            .filter(|(_, _, obj)| matches!(obj, CellObject::Volcano))
            .map(|(col, row, _)| (col, row))
            .collect()
    }

    /// Every cell a volcano's cone stands on (`volcano::footprint`): solid
    /// ground a hull can neither stand on nor spawn in, though only the
    /// crater is a cell of the map.
    pub fn volcano_footprint(&self) -> std::collections::BTreeSet<(i32, i32)> {
        self.volcano_cells().into_iter().flat_map(|(c, r)| crate::volcano::footprint(c, r)).collect()
    }

    /// True when `(col, row)` holds a solid tile or stands under a
    /// volcano's cone.
    pub fn solid_at(&self, col: i32, row: i32) -> bool {
        self.cell(col, row).is_some_and(CellObject::is_solid)
            || self.volcano_cells().iter().any(|&(c, r)| crate::volcano::in_footprint(c, r, col, row))
    }

    /// Cap on how far `nearest_free_cell` will spiral out looking for an
    /// unwalled cell - 64 cells (2048px at `OBSTACLE_GRID_SIZE`) comfortably
    /// covers the default 1280x720 battlefield (40x22.5 cells) from any
    /// starting point, so this only matters as a bound against a
    /// pathological future map, not something normal play ever brushes up
    /// against.
    pub(crate) const NEAREST_FREE_CELL_MAX_RADIUS: i32 = 64;

    /// `(col, row)` if it holds nothing solid, else the nearest cell to it
    /// (by expanding ring, closest first) that doesn't - only walls and
    /// props block a tank spawn; road/pickup/frog cells are fine to spawn
    /// on top of. Used as
    /// the fallback player-start position when a map places no `Start` cell
    /// (`Game::init`), so the player never spawns wedged inside a wall a
    /// hand-authored map happened to place at/near the exact center.
    /// Gives up and returns the original cell unchanged past
    /// `NEAREST_FREE_CELL_MAX_RADIUS` rings - an occasional wall-embedded
    /// spawn on a pathological map beats an unbounded search.
    pub fn nearest_free_cell(&self, col: i32, row: i32) -> (i32, i32) {
        let volcano = self.volcano_footprint();
        let is_wall = |c: i32, r: i32| self.cell(c, r).is_some_and(CellObject::is_solid) || volcano.contains(&(c, r));
        if !is_wall(col, row) {
            return (col, row);
        }
        for radius in 1..=Self::NEAREST_FREE_CELL_MAX_RADIUS {
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    // Only the ring at exactly this radius - smaller
                    // radii were already checked on an earlier iteration.
                    if dx.abs().max(dy.abs()) != radius {
                        continue;
                    }
                    let (c, r) = (col + dx, row + dy);
                    if !is_wall(c, r) {
                        return (c, r);
                    }
                }
            }
        }
        (col, row)
    }
}

/// Directory saved maps live under: `maps`, relative to the process's
/// working directory - the "path relative to CWD, not the binary"
/// convention every asset path in this project follows (see CLAUDE.md's
/// Releases section) - unless `set_maps_dir` named another, which the
/// macOS app does (`app::macos`), since a signed bundle must not be
/// written to.
pub fn maps_dir() -> PathBuf {
    MAPS_DIR.get().cloned().unwrap_or_else(|| PathBuf::from("maps"))
}

/// The directory `maps_dir` answers in place of `maps`.
static MAPS_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Keep saved maps under `dir` for the rest of the process; set once, at
/// startup, before anything lists or saves a map. A later call is ignored.
pub fn set_maps_dir(dir: PathBuf) {
    let _ = MAPS_DIR.set(dir);
}

/// The maps compiled into the binary, by name: the default battlefields,
/// the two mission fixtures, the portal and tower maps, the big field
/// (`longwater`, free play several screens across, the one the follow
/// camera was built for), then the hand-authored levels (each file's
/// header says how it plays). They are what the web build can offer its
/// Load list, since nothing outside `static/` ships in the wasm, what the
/// online lobby's map stepper walks, in this order, and what the room
/// server can open; they stand in on native for a checkout without a
/// `maps/` directory.
pub const SHIPPED_MAPS: &[(&str, &str)] = &[
    ("default", include_str!("../maps/default.toml")),
    ("default-desert", include_str!("../maps/default-desert.toml")),
    ("hunt-basic", include_str!("../maps/missions/hunt-basic.toml")),
    ("waves-basic", include_str!("../maps/missions/waves-basic.toml")),
    ("portals", include_str!("../maps/portals.toml")),
    ("towers", include_str!("../maps/towers.toml")),
    ("longwater", include_str!("../maps/longwater.toml")),
    ("lotus-lagoon", include_str!("../maps/lotus-lagoon.toml")),
    ("vulkan", include_str!("../maps/vulkan.toml")),
    ("hedge-maze", include_str!("../maps/hedge-maze.toml")),
    ("oasis-bazaar", include_str!("../maps/oasis-bazaar.toml")),
    ("castle-moat", include_str!("../maps/castle-moat.toml")),
    ("archipelago", include_str!("../maps/archipelago.toml")),
    ("black-gold", include_str!("../maps/black-gold.toml")),
    ("harbor-lights", include_str!("../maps/harbor-lights.toml")),
    ("carnival", include_str!("../maps/carnival.toml")),
    ("jungle-temple", include_str!("../maps/jungle-temple.toml")),
    ("serpent-river", include_str!("../maps/serpent-river.toml")),
    ("no-mans-land", include_str!("../maps/no-mans-land.toml")),
    ("glasshouses", include_str!("../maps/glasshouses.toml")),
    ("scrapyard", include_str!("../maps/scrapyard.toml")),
    ("grand-campaign", include_str!("../maps/grand-campaign.toml")),
    ("boot-camp", include_str!("../maps/boot-camp.toml")),
];

/// The names of the `SHIPPED_MAPS` a room may play, in their order: every
/// one but a training map (`MapFile::hostable`), which plays one seat.
/// What the lobby's map stepper walks.
pub fn hostable_maps() -> &'static [&'static str] {
    static HOSTABLE: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    HOSTABLE.get_or_init(|| {
        SHIPPED_MAPS
            .iter()
            .filter(|(_, text)| MapFile::from_toml_str(text).is_ok_and(|map| map.training.is_none()))
            .map(|(name, _)| *name)
            .collect()
    })
}

/// Whether this build keeps the maps the builder saves: everywhere while
/// map modding is on (`mapstore`, the player's own store on every
/// platform); with it off, native yes - under `maps_dir()` - and the web,
/// iOS and Android no (their edits live in memory for the session -
/// docs/game-editor-fusion.md; an app bundle is read-only).
pub fn saving_available() -> bool {
    crate::mapstore::enabled() || !crate::EMBEDDED
}

/// One map the builder's Load list can offer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MapEntry {
    pub name: String,
    /// Kept by the player: a file under `maps_dir()` (map modding off) or a
    /// map of their own in the store (`mapstore`); otherwise one of
    /// `SHIPPED_MAPS`.
    pub on_disk: bool,
    /// A shipped map the player has a modified copy of, which is what
    /// opens under its name (`mapstore`).
    pub modified: bool,
}

/// Every map the builder can load, sorted by name. With map modding on,
/// every shipped map - each marked where the player modified it - and the
/// player's own maps. Off, the files under `maps_dir()` on native, plus the
/// shipped maps not shadowed by a file of the same name; the web build
/// lists only the shipped ones.
pub fn available_maps() -> Vec<MapEntry> {
    let mut entries: Vec<MapEntry> = match crate::mapstore::store() {
        Some(store) => store.own_maps().into_iter().map(|name| MapEntry { name, on_disk: true, modified: false }).collect(),
        None if saving_available() => list_maps().into_iter().map(|name| MapEntry { name, on_disk: true, modified: false }).collect(),
        None => Vec::new(),
    };
    let store = crate::mapstore::store();
    for (name, _) in SHIPPED_MAPS {
        if !entries.iter().any(|e| e.name == *name) {
            let modified = store.is_some_and(|store| store.is_modified(name));
            entries.push(MapEntry { name: (*name).to_string(), on_disk: false, modified });
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

/// Open a map by its Load-list name (`map_source`). The result carries
/// `name` for display.
pub fn open_map(name: &str) -> Result<MapFile, String> {
    let text = map_source(name)?;
    let mut map = MapFile::from_toml_str(&text).map_err(|e| format!("parsing map {name}: {e}"))?;
    map.name = Some(name.to_string());
    Ok(map)
}

/// The text `open_map` reads a map by its Load-list name from. With map
/// modding on, the player's store (`mapstore::Store::source`: a shipped
/// map's modified copy, else their own map, else the shipped text). Off,
/// the file under `maps_dir()` when there is one (native), else the
/// shipped map's.
pub fn map_source(name: &str) -> Result<std::borrow::Cow<'static, str>, String> {
    if let Some(store) = crate::mapstore::store() {
        return store.source(name).ok_or_else(|| format!("no map named {name:?}"));
    }
    let path = maps_dir().join(format!("{name}.toml"));
    if saving_available() && path.is_file() {
        return std::fs::read_to_string(&path).map(std::borrow::Cow::Owned).map_err(|e| format!("reading map {}: {e}", path.display()));
    }
    SHIPPED_MAPS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, text)| std::borrow::Cow::Borrowed(*text))
        .ok_or_else(|| format!("no map named {name:?}"))
}

/// Every `.toml` file under `maps_dir()`, by file stem, sorted - the
/// on-disk half of `available_maps`. An unreadable/missing directory just
/// yields an empty list rather than an error (nothing to load yet is a
/// normal state, not a failure).
pub fn list_maps() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(maps_dir())
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("toml"))
        .filter_map(|entry| entry.path().file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod toml_tests {
    use super::*;

    #[test]
    fn shipped_maps_parse_and_open_by_name() {
        for (name, _) in SHIPPED_MAPS {
            let map = open_map(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(map.name.as_deref(), Some(*name));
            assert!(!map.cells.is_empty());
        }
        assert!(open_map("no-such-map").is_err());
        let entries = available_maps();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"default"));
        assert!(names.windows(2).all(|w| w[0] < w[1]), "sorted and unique");
    }

    #[test]
    fn toml_string_round_trips_the_default_map() {
        let map = MapFile::from_toml_str(include_str!("../maps/default.toml")).unwrap();
        let back = MapFile::from_toml_str(&map.to_toml_string().unwrap()).unwrap();
        assert_eq!(back.version, map.version);
        assert_eq!(back.tanks, map.tanks);
        assert_eq!(back.cells.len(), map.cells.len());
        for (key, cell) in &map.cells {
            assert!(back.cells.get(key) == Some(cell), "cell {key} changed");
        }
        assert_eq!(back.name, None, "name is not part of the file");
        assert_eq!(back.mission, map.mission, "the mission table changed");
        assert_eq!(back.spawn, map.spawn, "the spawn table changed");
    }

    #[test]
    fn theme_round_trips_and_defaults_to_grass() {
        let map = MapFile::from_toml_str("version = 1\n").unwrap();
        assert_eq!(map.theme, Theme::Grass);
        assert!(!map.to_toml_string().unwrap().contains("theme"), "the default is not written back");
        let map = MapFile::from_toml_str("version = 1\ntheme = \"desert\"\ncells.\"1,1\" = { kind = \"road\" }\n").unwrap();
        assert_eq!(map.theme, Theme::Desert);
        let text = map.to_toml_string().unwrap();
        assert!(text.contains("theme = \"desert\""), "{text}");
        assert_eq!(MapFile::from_toml_str(&text).unwrap().theme, Theme::Desert);
        assert!(MapFile::from_toml_str("version = 1\ntheme = \"lava\"\n").is_err(), "an unknown theme is a parse error");
        for t in Theme::ALL {
            assert_eq!(Theme::parse(t.name()), Some(t));
        }
        let desert = open_map("default-desert").unwrap();
        assert_eq!(desert.theme, Theme::Desert);
    }

    #[test]
    fn weather_round_trips_and_defaults_to_clear() {
        let map = MapFile::from_toml_str("version = 1\n").unwrap();
        assert_eq!(map.weather, Skies::CLEAR);
        assert!(!map.to_toml_string().unwrap().contains("weather"), "the default is not written back");
        let map = MapFile::from_toml_str("version = 1\nweather = \"heat_haze\"\ncells.\"1,1\" = { kind = \"road\" }\n").unwrap();
        assert_eq!(map.weather, Weather::HeatHaze.into());
        let text = map.to_toml_string().unwrap();
        assert!(text.contains("weather = \"heat_haze\""), "{text}");
        assert_eq!(MapFile::from_toml_str(&text).unwrap().weather, Weather::HeatHaze.into());
        assert!(MapFile::from_toml_str("version = 1\nweather = \"hail\"\n").is_err(), "an unknown weather is a parse error");
        assert!(MapFile::from_toml_str("version = 1\nweather = [\"rain\", \"hail\"]\n").is_err(), "in a list too");
        for (i, w) in Weather::ALL.into_iter().enumerate() {
            assert_eq!(Weather::parse(w.name()), Some(w));
            assert_eq!(w.index(), i);
            // The serde spelling is the name the builder and the tools use,
            // and one sky is written back as it was read.
            let text = format!("version = 1\nweather = \"{}\"\n", w.name());
            let map = MapFile::from_toml_str(&text).unwrap();
            assert_eq!(map.weather, Skies::of(w), "{}", w.name());
            assert_eq!(map.weather.single(), Some(w), "{}", w.name());
            if w != Weather::Clear {
                assert!(map.to_toml_string().unwrap().contains(&format!("weather = \"{}\"", w.name())), "{}", w.name());
            }
        }
        // `SKIES` is `ALL` without `Random`, in the same order, so a sky's
        // override index is the same in both.
        assert_eq!(Weather::ALL.iter().filter(|w| **w != Weather::Random).copied().collect::<Vec<_>>(), Weather::SKIES);
        let mut random = MapFile::new();
        random.weather = Weather::Random.into();
        assert_eq!(random.weather, Skies::ALL);
        let text = random.to_toml_string().unwrap();
        assert!(text.contains("weather = \"random\""), "{text}");
        assert_eq!(MapFile::from_toml_str(&text).unwrap().weather, Skies::ALL, "a random sky stays random on disk");
    }

    /// A map's `weather` may name several skies: a list, read in any
    /// order and with repeats, written back once each in `SKIES` order -
    /// and a list of every sky is `random`, of one sky its name, of none
    /// clear, so every spelling has one canonical form.
    #[test]
    fn a_list_of_skies_round_trips() {
        let map = MapFile::from_toml_str("version = 1\nweather = [\"snow\", \"night\", \"snow\"]\n").unwrap();
        assert_eq!(map.weather.skies(), vec![Weather::Night, Weather::Snow]);
        assert_eq!(map.weather.single(), None);
        let text = map.to_toml_string().unwrap();
        let at = |name: &str| text.find(&format!("\"{name}\"")).unwrap_or_else(|| panic!("{text}"));
        assert!(text.contains("weather = [") && at("night") < at("snow") && text.matches("snow").count() == 1, "{text}");
        assert_eq!(MapFile::from_toml_str(&text).unwrap().weather, map.weather);
        let one = MapFile::from_toml_str("version = 1\nweather = [\"fog\"]\n").unwrap();
        assert!(one.to_toml_string().unwrap().contains("weather = \"fog\""));
        for none in ["weather = []", "weather = [\"clear\"]", "weather = \"clear\""] {
            let map = MapFile::from_toml_str(&format!("version = 1\n{none}\n")).unwrap();
            assert_eq!(map.weather, Skies::CLEAR, "{none}");
            assert!(!map.to_toml_string().unwrap().contains("weather"), "{none} is not written back");
        }
        let every: Vec<String> = Weather::SKIES.iter().map(|w| format!("\"{}\"", w.name())).collect();
        let all = MapFile::from_toml_str(&format!("version = 1\nweather = [{}]\n", every.join(", "))).unwrap();
        assert_eq!(all.weather, Skies::ALL);
        assert!(all.to_toml_string().unwrap().contains("weather = \"random\""));
        // Clear may be one of several: a dry round or a wet one.
        let either = MapFile::from_toml_str("version = 1\nweather = [\"rain\", \"clear\"]\n").unwrap();
        assert_eq!(either.weather.skies(), vec![Weather::Clear, Weather::Rain]);
        let text = either.to_toml_string().unwrap();
        assert_eq!(MapFile::from_toml_str(&text).unwrap().weather, either.weather, "{text}");
    }

    /// The checklist's tap: in and out again, `clear` shown while nothing
    /// is named and taken out only from among others.
    #[test]
    fn toggling_skies() {
        let s = Skies::CLEAR;
        assert!(s.contains(Weather::Clear) && !s.contains(Weather::Random));
        assert_eq!(s.toggled(Weather::Clear), Skies::CLEAR, "clear alone stays clear");
        let s = s.toggled(Weather::Night);
        assert_eq!(s, Weather::Night.into());
        assert!(!s.contains(Weather::Clear));
        let s = s.toggled(Weather::Clear);
        assert_eq!(s.skies(), vec![Weather::Clear, Weather::Night]);
        assert_eq!(s.toggled(Weather::Night), Skies::CLEAR, "clear alone again is none");
        assert_eq!(s.toggled(Weather::Random), s, "random names no one sky");
        assert_eq!(Skies::parse(["random", "night"]), Ok(Skies::ALL));
        assert_eq!(Skies::parse(["night", "hail"]), Err("hail"));
    }

    #[test]
    fn view_round_trips_and_defaults_to_the_maps_size() {
        let map = MapFile::from_toml_str("version = 1\n").unwrap();
        assert_eq!(map.view, None);
        assert!(!map.to_toml_string().unwrap().contains("view"), "by size is not written back");
        for v in MapView::ALL {
            let text = format!("version = 1\nview = \"{}\"\ncells.\"1,1\" = {{ kind = \"road\" }}\n", v.name());
            let map = MapFile::from_toml_str(&text).unwrap();
            assert_eq!(map.view, Some(v), "{}", v.name());
            let back = map.to_toml_string().unwrap();
            assert!(back.contains(&format!("view = \"{}\"", v.name())), "{back}");
            assert_eq!(MapFile::from_toml_str(&back).unwrap().view, Some(v));
            assert_eq!(MapView::parse(v.name()), Some(v));
        }
        assert!(MapFile::from_toml_str("version = 1\nview = \"zoom\"\n").is_err(), "an unknown view is a parse error");
        assert_eq!(MapView::parse("auto"), None, "by size has no spelling: it is the key left out");
    }

    #[test]
    fn a_maps_view_key_overrides_its_size() {
        let class = |lines: &str| MapFile::from_toml_str(&format!("version = 1\n{lines}")).unwrap().class();
        assert_eq!(class(""), MapClass::Arena, "no size is the standard 34 x 17");
        assert_eq!(class("size = [36, 18]\n"), MapClass::Arena);
        assert_eq!(class("size = [36, 18.5]\n"), MapClass::Field);
        assert_eq!(class("size = [40, 20]\n"), MapClass::Field);
        assert_eq!(class("size = [48, 24]\nview = \"whole\"\n"), MapClass::Arena);
        assert_eq!(class("size = [34, 17]\nview = \"follow\"\n"), MapClass::Field);
        assert_eq!(class("view = \"follow\"\n"), MapClass::Field);
        assert_eq!(class("size = [36, 18]\nview = \"whole\"\n"), MapClass::Arena);
        let mut map = MapFile::new();
        assert_eq!(map.class(), MapClass::Arena);
        map.size = Some((96.0, 54.0));
        assert_eq!(map.class(), MapClass::Field);
        map.view = Some(MapView::Whole);
        assert_eq!(map.class(), MapClass::Arena);
        // Past what a target drawn whole holds, `whole` is followed.
        assert_eq!(class("size = [128, 128]\nview = \"whole\"\n"), MapClass::Arena);
        assert_eq!(class("size = [129, 40]\nview = \"whole\"\n"), MapClass::Field);
        assert_eq!(class("size = [250, 250]\nview = \"whole\"\n"), MapClass::Field);
    }

    #[test]
    fn every_shipped_map_is_a_field_map() {
        // Every level and free-play map is past 36 x 18 - the smallest are
        // 48 x 24 - so the follow camera frames each of them; an arena is a
        // map of your own, or a fixture under maps/test/.
        for (name, text) in SHIPPED_MAPS {
            assert_eq!(MapFile::from_toml_str(text).unwrap().class(), MapClass::Field, "{name}");
        }
        let big = open_map("longwater").unwrap();
        assert_eq!((big.size, big.class()), (Some((112.0, 63.0)), MapClass::Field));
        let study = MapFile::load(Path::new("maps/study/frontier.toml")).unwrap();
        assert_eq!((study.size, study.class()), (Some((96.0, 54.0)), MapClass::Field));
    }

    #[test]
    fn missing_level_tables_mean_protect_and_band() {
        let map = MapFile::from_toml_str("version = 1\n").unwrap();
        assert_eq!(map.mission, MissionConfig::default());
        assert_eq!(map.spawn, SpawnConfig::default());
    }

    #[test]
    fn level_tables_and_new_cell_kinds_round_trip() {
        use crate::level::{Mission, SpawnKind, Tier};
        // Dotted keys, the form the fixtures use: a `[mission]`/`[spawn]`
        // table header would swallow every `cells.` line after it.
        let text = r#"
version = 1
tanks = 6
mission.kind = "hunt"
spawn.kind = "waves"
spawn.waves = 4
spawn.size = 2
spawn.tier_start = "light"
spawn.tier_end = "heavy"
tank2 = "titan"
cells."3,5" = { kind = "enemy_frog" }
cells."4,5" = { kind = "start2" }
cells."0,11" = { kind = "gate" }
cells."39,11" = { kind = "gate" }
cells."30,5" = { kind = "portal" }
cells."10,5" = { kind = "portal" }
"#;
        let map = MapFile::from_toml_str(text).unwrap();
        assert_eq!(map.tank2, Some(TankKind::Titan));
        assert_eq!(map.start2_cell(), Some((4, 5)));
        assert_eq!(map.start_cell(), None);
        assert!(map.cell(4, 5).is_some_and(|c| !c.is_solid()));
        // An older file without the key still parses, with no preference.
        assert_eq!(MapFile::from_toml_str("version = 1\n").unwrap().tank2, None);
        assert_eq!(map.mission.kind, Mission::Hunt);
        assert_eq!(map.spawn.kind, SpawnKind::Waves);
        assert_eq!((map.spawn.waves, map.spawn.size, map.spawn.growth), (Some(4), Some(2), None));
        assert_eq!((map.spawn.tier_start, map.spawn.tier_end), (Some(Tier::Light), Some(Tier::Heavy)));
        assert_eq!(map.enemy_frog_cell(), Some((3, 5)));
        assert_eq!(map.gate_cells(), vec![(0, 11), (39, 11)]);
        // Portals: multi-instance anchors in `iter_cells` order, never solid,
        // so a spawn fallback can stand on one.
        assert_eq!(map.portal_cells(), vec![(10, 5), (30, 5)]);
        assert!(map.cell(10, 5).is_some_and(|c| !c.is_solid() && c.material().is_none()));
        assert_eq!(map.nearest_free_cell(10, 5), (10, 5));
        let back = MapFile::from_toml_str(&map.to_toml_string().unwrap()).unwrap();
        assert_eq!(back.mission, map.mission);
        assert_eq!(back.spawn, map.spawn);
        assert_eq!(back.enemy_frog_cell(), map.enemy_frog_cell());
        assert_eq!(back.gate_cells(), map.gate_cells());
        assert_eq!(back.portal_cells(), map.portal_cells());
        assert_eq!(back.start2_cell(), Some((4, 5)));
        assert_eq!(back.tank2, Some(TankKind::Titan));
    }

    #[test]
    fn a_maps_size_is_its_field_and_round_trips() {
        let text = "version = 1\nsize = [40, 22.5]\ncells.\"3,3\" = { kind = \"start\" }\n";
        let map = MapFile::from_toml_str(text).unwrap();
        assert_eq!(map.size, Some((40.0, 22.5)));
        assert_eq!(map.field_size(), (1280.0, 720.0));
        let back = MapFile::from_toml_str(&map.to_toml_string().unwrap()).unwrap();
        assert_eq!(back.size, map.size, "size did not survive the TOML round trip");
        // No size means the standard field, and a size never serialises as an
        // empty key.
        let bare = MapFile::from_toml_str("version = 1\n").unwrap();
        assert_eq!(bare.size, None);
        assert_eq!(bare.field_size(), (crate::DEFAULT_SCREEN_WIDTH as f32, crate::DEFAULT_SCREEN_HEIGHT as f32));
        assert!(!bare.to_toml_string().unwrap().contains("size"));
        let ints = MapFile::from_toml_str("version = 1\nsize = [30, 15]\n").unwrap();
        assert_eq!(ints.field_size(), (960.0, 480.0));
    }

    /// The largest map is one an online round can carry: its far corner,
    /// and a wave tank waiting a tank length past it, are positions the
    /// wire holds, and its last cell an index the wire holds.
    #[test]
    fn a_map_is_no_larger_than_the_wire_carries() {
        let largest = MapFile::from_toml_str("version = 1\nsize = [250, 250]\n").unwrap();
        let (w, h) = largest.field_size();
        let past = w.max(h) + 2.0 * OBSTACLE_GRID_SIZE;
        assert!(past <= crate::net::wire::POSITION_MAX_PX, "{past} px does not fit the wire");
        let last = (MAX_SIDE_CELLS as u32) * (MAX_SIDE_CELLS as u32);
        assert!(last <= u16::MAX as u32, "{last} cells do not fit a u16 index");
        for size in ["[251, 20]", "[40, 250.5]", "[nan, 20]", "[40, inf]"] {
            let err = MapFile::from_toml_str(&format!("version = 1\nsize = {size}\n")).unwrap_err();
            assert!(err.contains("largest map"), "{size}: {err}");
        }
    }

    /// A map has room for a tank to stand and wander on either axis: a
    /// sliver of a map is refused by name rather than played, and every
    /// map shipped or under `maps/` is at least that large.
    #[test]
    fn a_map_is_no_smaller_than_a_round_can_play_on() {
        for size in ["[1.5, 20]", "[40, 7.5]", "[0, 0]", "[-3, 20]"] {
            let err = MapFile::from_toml_str(&format!("version = 1\nsize = {size}\n")).unwrap_err();
            assert!(err.contains("smallest map"), "{size}: {err}");
        }
        assert!(MapFile::from_toml_str("version = 1\nsize = [8, 8]\n").is_ok());
        // A builder stamp's size is its own extent, read without the bound.
        assert_eq!(MapFile::from_toml_str_any_size("version = 1\nsize = [6, 5]\n").unwrap().size, Some((6.0, 5.0)));
        for (name, text) in SHIPPED_MAPS {
            let (w, h) = MapFile::from_toml_str(text).unwrap().field_size();
            assert!(w / OBSTACLE_GRID_SIZE >= MIN_SIDE_CELLS && h / OBSTACLE_GRID_SIZE >= MIN_SIDE_CELLS, "{name}");
        }
    }

    #[test]
    fn prop_cells_round_trip_and_count_as_solid() {
        let text = r#"
version = 1
cells."4,4" = { kind = "sandbag" }
cells."5,4" = { kind = "barrel" }
cells."6,4" = { kind = "fence" }
cells."7,4" = { kind = "road" }
cells."8,4" = { kind = "water" }
"#;
        let map = MapFile::from_toml_str(text).unwrap();
        assert_eq!(map.cell(4, 4).and_then(|c| c.material()), Some(Material::Sandbag));
        assert_eq!(map.cell(5, 4).and_then(|c| c.material()), Some(Material::Barrel));
        assert_eq!(map.cell(6, 4).and_then(|c| c.material()), Some(Material::Fence));
        assert!(map.cell(7, 4).is_some_and(|c| !c.is_solid()), "road is not solid");
        assert_ne!(map.nearest_free_cell(5, 4), (5, 4), "a barrel cell blocks a spawn");
        assert_eq!(map.nearest_free_cell(7, 4), (7, 4));
        assert_eq!(map.cell(8, 4), Some(&CellObject::Water));
        assert!(!CellObject::Water.is_solid(), "water is ground, like road");
        assert_eq!(map.nearest_free_cell(8, 4), (8, 4), "a tank can stand in water");
        let back = MapFile::from_toml_str(&map.to_toml_string().unwrap()).unwrap();
        for (col, row, cell) in map.iter_cells() {
            assert!(back.cell(col, row) == Some(cell), "cell {col},{row} changed");
        }
        for m in [Material::Sandbag, Material::Barrel, Material::Fence] {
            assert_eq!(CellObject::prop(m).and_then(|c| c.material()), Some(m));
        }
        assert!(CellObject::prop(Material::Brick).is_none());
    }

    #[test]
    fn load_names_the_map_after_its_file() {
        let map = MapFile::load(Path::new("maps/test/choke.toml")).unwrap();
        assert_eq!(map.name.as_deref(), Some("choke"));
        assert_eq!(map.tanks, Some(4));
    }

    /// A revision is the map and nothing else: the order its cells went in,
    /// its display name and its stamp are no part of it, the TOML it
    /// writes reads back as the same revision, and any edit to a cell or a
    /// setting is another.
    #[test]
    fn a_revision_is_the_map_and_nothing_else() {
        let mut a = MapFile::new();
        a.set_cell(3, 4, CellObject::Start);
        a.set_cell(10, 2, CellObject::Wall { material: Material::Brick });
        a.set_cell(2, 10, CellObject::Frog);
        let mut b = MapFile::new();
        b.set_cell(2, 10, CellObject::Frog);
        b.set_cell(10, 2, CellObject::Wall { material: Material::Brick });
        b.set_cell(3, 4, CellObject::Start);
        assert_eq!(a.revision(), b.revision(), "the order the cells went in");
        b.name = Some("mine".into());
        b.cleared = Some(Cleared::new(7, 12.5));
        assert_eq!(a.revision(), b.revision(), "the name and the stamp");
        let back = MapFile::from_toml_str(&a.to_toml_string().unwrap()).unwrap();
        assert_eq!(back.revision(), a.revision(), "read back from its TOML");
        let mut edited = a.clone();
        edited.set_cell(5, 5, CellObject::Gate);
        assert_ne!(edited.revision(), a.revision(), "a cell placed");
        let mut edited = a.clone();
        edited.set_cell(10, 2, CellObject::Wall { material: Material::Iron });
        assert_ne!(edited.revision(), a.revision(), "a cell changed");
        let mut edited = a.clone();
        edited.tanks = Some(3);
        assert_ne!(edited.revision(), a.revision(), "a setting");
        let mut edited = a.clone();
        edited.size = Some((40.0, 22.5));
        assert_ne!(edited.revision(), a.revision(), "the size");
        assert_eq!(revision_text(a.revision()).len(), 16);
    }

    /// The clear check's stamp counts for the revision it names and no
    /// other, rides the TOML both ways, and is what lets a map of one's own
    /// be hosted; a shipped map as it ships needs none, and an edited one
    /// is a map of one's own.
    #[test]
    fn a_stamp_counts_for_its_own_revision_only() {
        let mut map = MapFile::new();
        map.set_cell(3, 8, CellObject::Start);
        assert_eq!(map.cleared_par(), None);
        assert!(!map.hostable(), "nobody has won it");
        map.cleared = Some(Cleared::new(map.revision(), 83.4));
        assert_eq!(map.cleared_par(), Some(83.4));
        assert!(map.hostable());
        let text = map.to_toml_string().unwrap();
        assert!(text.contains("[cleared]") && text.contains("par = 83.4"), "{text}");
        let back = MapFile::from_toml_str(&text).unwrap();
        assert_eq!(back.cleared_par(), Some(83.4), "read back");
        let mut edited = back.clone();
        edited.set_cell(6, 6, CellObject::Gate);
        assert_eq!(edited.cleared_par(), None, "an edit since: a revision nobody has won");
        assert!(!edited.hostable());
        let plain = MapFile::new();
        assert!(!plain.to_toml_string().unwrap().contains("cleared"), "no stamp is written until there is one");
        for name in hostable_maps() {
            let text = SHIPPED_MAPS.iter().find(|(n, _)| n == name).map(|(_, text)| *text).unwrap();
            let shipped = MapFile::from_toml_str(text).unwrap();
            assert!(shipped.hostable(), "{name} as it ships");
            let mut changed = shipped.clone();
            changed.set_cell(1, 1, CellObject::Gate);
            assert!(!changed.hostable(), "{name} edited");
        }
        let training = SHIPPED_MAPS.iter().filter(|(_, text)| MapFile::from_toml_str(text).unwrap().training.is_some());
        for (name, text) in training {
            assert!(!MapFile::from_toml_str(text).unwrap().hostable(), "{name} is a training map, never a room's");
            assert!(!hostable_maps().contains(name));
        }
    }

    /// What a revision costs on the 96 x 54 study map - the builder works
    /// one out after each edit it shows the clear check of:
    /// `cargo test --lib a_revision_of_the_study_map_timing -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn a_revision_of_the_study_map_timing() {
        let map = MapFile::load(Path::new("maps/study/frontier.toml")).unwrap();
        let runs = 20;
        let start = std::time::Instant::now();
        let mut last = 0;
        for _ in 0..runs {
            last = std::hint::black_box(&map).revision();
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / runs as f64;
        println!("frontier: {} cells, revision {} in {ms:.2} ms", map.cells.len(), revision_text(last));
    }
}
