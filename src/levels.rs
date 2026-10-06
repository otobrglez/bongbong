//! The levels (docs/levels.md): the maps `levels.toml` lists, played in
//! order. A win opens the next level, a loss plays the same one again,
//! and a win on the last comes back round to the first. Headless like
//! `lobby.rs`: `mode::Session` holds a [`Campaign`] and asks it which map
//! a level is, and `app.rs` carries the progress to and from the
//! platform's store - a file on a desktop or a phone, the page's
//! `localStorage` on the web.
//!
//! **A level is a map by name.** Whether the round on the field is a
//! level is read off its map's name (`Campaign::position`), so a level
//! loaded into the builder and played is still that level, and any other
//! map - `-m`, the builder's Load list, a Save As under a new name - is
//! free play, which restarts on its own as it always has.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::map::MapFile;

/// The level list, compiled in (see its header for the rules).
pub const LEVELS_TOML: &str = include_str!("../levels.toml");

/// One entry of `levels.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Level {
    /// The map, by its file stem under `maps/` - one of `SHIPPED_MAPS`.
    pub map: String,
    /// The English title, as the opening banner draws it.
    pub title: String,
    /// A level a player may pass over (docs/training-stage.md): while it
    /// is the furthest reached, the level after it is open too.
    #[serde(default)]
    pub skippable: bool,
}

impl Level {
    /// The title in the language on screen: the catalogue's
    /// `level-<map>` where it has one, else the English one written in
    /// `levels.toml`, folded for the font either way.
    pub fn title(&self) -> String {
        let t = crate::text::text();
        t.message(&format!("level-{}", self.map), &[]).unwrap_or_else(|| crate::text::fold(&self.title).into_owned())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LevelsFile {
    #[serde(default)]
    level: Vec<Level>,
}

/// The levels in play order: never empty, every map named once and by a
/// name a file could have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Levels(Vec<Level>);

impl Levels {
    /// Parse a `levels.toml`.
    pub fn parse(text: &str) -> Result<Levels, String> {
        let file: LevelsFile = toml::from_str(text).map_err(|e| e.to_string())?;
        if file.level.is_empty() {
            return Err("no [[level]] entries".into());
        }
        for (i, level) in file.level.iter().enumerate() {
            let name_ok = !level.map.is_empty() && level.map.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !name_ok {
                return Err(format!("level {}: {:?} is not a map name", i + 1, level.map));
            }
            if level.title.trim().is_empty() {
                return Err(format!("level {} ({}) has no title", i + 1, level.map));
            }
            if file.level[..i].iter().any(|l| l.map == level.map) {
                return Err(format!("level {}: {} is listed twice", i + 1, level.map));
            }
        }
        Ok(Levels(file.level))
    }

    /// The compiled-in list. A broken `levels.toml` is a bug in the
    /// repo, which `level_tests` catches before it ships.
    pub fn shipped() -> Levels {
        Levels::parse(LEVELS_TOML).unwrap_or_else(|e| panic!("levels.toml: {e}"))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, i: usize) -> Option<&Level> {
        self.0.get(i)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Level> {
        self.0.iter()
    }

    /// Which level `map` (a map's name) is, if any.
    pub fn position(&self, map: &str) -> Option<usize> {
        self.0.iter().position(|l| l.map == map)
    }

    /// The number the first level is shown by: 0 where it is skippable
    /// (the training stage, docs/training-stage.md), so the first real
    /// level is still level 1; else 1.
    pub fn first_number(&self) -> usize {
        if self.0.first().is_some_and(|l| l.skippable) { 0 } else { 1 }
    }

    /// The number level `i` (an index) is shown by: on the banner, the
    /// level button, its tile, `--level` and the dev server.
    pub fn number(&self, i: usize) -> usize {
        i + self.first_number()
    }

    /// The highest number shown, the banner's count.
    pub fn last_number(&self) -> usize {
        self.number(self.len() - 1)
    }

    /// A level as the command line names it (`--level`): its number as
    /// shown (`number`), or its map's name.
    pub fn find(&self, spec: &str) -> Option<usize> {
        let spec = spec.trim();
        match spec.parse::<usize>() {
            Ok(n) => (self.first_number()..=self.last_number()).contains(&n).then(|| n - self.first_number()),
            Err(_) => self.position(spec),
        }
    }
}

/// Where the player is in the levels for this session: the list, the
/// furthest level reached, and the builder's edits.
#[derive(Clone, Debug)]
pub struct Campaign {
    pub levels: Levels,
    /// The furthest level reached, as an index: the first until a win
    /// opens another. Only a win moves it, and only forward.
    reached: usize,
    /// The levels as last edited in the builder, by map name, for the
    /// rest of the session: a level played again is played as edited
    /// (`remember_edit`).
    edits: BTreeMap<String, MapFile>,
    /// A win has moved `reached` and the store has not been told.
    unsaved: bool,
}

impl Campaign {
    /// The campaign on `levels`, with the progress the store kept: the
    /// map name of the furthest level reached. A name the list no longer
    /// has starts from the first level.
    pub fn new(levels: Levels, saved: Option<&str>) -> Campaign {
        let reached = saved.and_then(|name| levels.position(name)).unwrap_or(0);
        Campaign { levels, reached, edits: BTreeMap::new(), unsaved: false }
    }

    /// The furthest level reached - the one a new session opens on.
    pub fn reached(&self) -> usize {
        self.reached
    }

    /// The furthest level open to play: the furthest reached, or the one
    /// after it while that one is skippable. Winning an open level moves
    /// `reached` past it (`won`), so a skipped level stays open behind.
    pub fn open_to(&self) -> usize {
        let skip = self.levels.get(self.reached).is_some_and(|l| l.skippable);
        if skip { (self.reached + 1).min(self.levels.len() - 1) } else { self.reached }
    }

    /// Which level the map called `name` is, if any.
    pub fn position(&self, name: Option<&str>) -> Option<usize> {
        self.levels.position(name?)
    }

    /// The map of level `i`, named after it: the builder's last edit this
    /// session, else the map file (`map::open_map` - a desktop build's
    /// `maps/<map>.toml`, else the shipped copy).
    pub fn map(&self, i: usize) -> Result<MapFile, String> {
        let level = self.levels.get(i).ok_or_else(|| format!("there is no level {}", i + 1))?;
        if let Some(edit) = self.edits.get(&level.map) {
            return Ok(edit.clone());
        }
        crate::map::open_map(&level.map)
    }

    /// Keep `map` as its level's map for the rest of the session, when
    /// its name makes it one - what `PLAY` does with the builder's canvas.
    pub fn remember_edit(&mut self, map: &MapFile) {
        if let Some(name) = map.name.as_deref().filter(|name| self.levels.position(name).is_some()) {
            self.edits.insert(name.to_string(), map.clone());
        }
    }

    /// The level after `i`: after the last, the first again - level 1,
    /// not the training stage before it.
    pub fn next(&self, i: usize) -> usize {
        match (i + 1) % self.levels.len() {
            // Round from the last level to level 1, past the training stage.
            0 if self.levels.first_number() == 0 && self.levels.len() > 1 => 1,
            next => next,
        }
    }

    /// Whether `i` is the last level.
    pub fn is_last(&self, i: usize) -> bool {
        i + 1 == self.levels.len()
    }

    /// Level `i` was won: the next one is reached, if it is further than
    /// any before. The last level opens nothing further.
    pub fn won(&mut self, i: usize) {
        let opened = (i + 1).min(self.levels.len() - 1);
        if opened > self.reached {
            self.reached = opened;
            self.unsaved = true;
        }
    }

    /// The progress to hand the store, once per change: the map name of
    /// the furthest level reached.
    pub fn take_unsaved(&mut self) -> Option<String> {
        if !std::mem::take(&mut self.unsaved) {
            return None;
        }
        self.levels.get(self.reached).map(|l| l.map.clone())
    }
}

/// What the progress store holds.
#[derive(Deserialize, serde::Serialize)]
struct ProgressFile {
    /// The map name of the furthest level reached.
    level: String,
}

/// The progress as the store writes it.
pub fn progress_text(level: &str) -> String {
    toml::to_string(&ProgressFile { level: level.to_string() }).expect("a string field serialises")
}

/// The level a store's text names, or `None` for text that is not
/// progress - an empty store, a file from somewhere else.
pub fn parse_progress(text: &str) -> Option<String> {
    let file: ProgressFile = toml::from_str(text).ok()?;
    let level = file.level.trim();
    (!level.is_empty()).then(|| level.to_string())
}

/// Read the progress file at `path`: `None` when there is none yet or it
/// is not progress.
pub fn read_progress(path: &Path) -> Option<String> {
    parse_progress(&std::fs::read_to_string(path).ok()?)
}

/// Write the progress file at `path`, making its directory.
pub fn write_progress(path: &Path, level: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    std::fs::write(path, progress_text(level)).map_err(|e| format!("writing {}: {e}", path.display()))
}

/// Where a desktop or an iPhone keeps the progress file: `data_dir`'s
/// `progress.toml`. `BONGBONG_PROGRESS` names a file outright. `None`
/// where the environment names no data directory. Android's directory
/// comes from its activity (`app::android::data_dir`) and the web keeps
/// its progress in the page's `localStorage`.
pub fn progress_path() -> Option<PathBuf> {
    if let Some(file) = std::env::var_os("BONGBONG_PROGRESS").filter(|f| !f.is_empty()) {
        return Some(PathBuf::from(file));
    }
    Some(data_dir()?.join("progress.toml"))
}

/// The game's own folder in the platform's per-user data directory -
/// `%APPDATA%` on Windows, `~/Library/Application Support` on Apple's
/// platforms (on iOS `HOME` is the app's own container), `$XDG_DATA_HOME`
/// or `~/.local/share` elsewhere - then `bongbong`. Where the progress file
/// lives, and the macOS app's saved maps (`app::macos`). `None` where the
/// environment names no such directory.
pub fn data_dir() -> Option<PathBuf> {
    let home = || std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    let dir = if cfg!(windows) {
        std::env::var_os("APPDATA").filter(|d| !d.is_empty()).map(PathBuf::from)
    } else if cfg!(target_vendor = "apple") {
        home().map(|h| h.join("Library").join("Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|d| d.is_absolute())
            .or_else(|| home().map(|h| h.join(".local").join("share")))
    }?;
    Some(dir.join("bongbong"))
}

#[cfg(test)]
mod level_tests {
    /// A skippable level leaves the next one open while it is the
    /// furthest reached, and winning that next one moves past both.
    #[test]
    fn a_skippable_level_leaves_the_next_one_open() {
        let levels = Levels::parse("[[level]]\nmap = \"a\"\ntitle = \"A\"\nskippable = true\n\n[[level]]\nmap = \"b\"\ntitle = \"B\"\n\n[[level]]\nmap = \"c\"\ntitle = \"C\"\n").expect("parses");
        let mut c = Campaign::new(levels, None);
        assert_eq!((c.reached(), c.open_to()), (0, 1), "the level after the skippable one is open");
        c.won(1);
        assert_eq!((c.reached(), c.open_to()), (2, 2), "winning it passes over the skippable level");
        assert_eq!(c.next(2), 1, "the last level leads round to level 1, not back to training");
    }

    /// Boot Camp is level 0 and Lotus Lagoon open beside it from the start.
    #[test]
    fn boot_camp_opens_the_list_and_can_be_skipped() {
        let levels = Levels::shipped();
        let c = Campaign::new(levels, None);
        assert_eq!(c.levels.get(0).map(|l| l.map.as_str()), Some("boot-camp"));
        assert_eq!((c.reached(), c.open_to()), (0, 1));
    }

    use super::*;

    /// Every level names a shipped map that opens, so the web build and
    /// the phones carry every level (maplint's `SUPPORTED_MAPS` holds
    /// them to lint clean).
    #[test]
    fn every_level_is_a_shipped_map() {
        let levels = Levels::shipped();
        assert!(levels.len() >= 2, "a campaign of one has nowhere to go");
        for level in levels.iter() {
            assert!(
                crate::map::SHIPPED_MAPS.iter().any(|(name, _)| *name == level.map),
                "{} is a level but not a shipped map",
                level.map
            );
            let map = crate::map::open_map(&level.map).unwrap_or_else(|e| panic!("{}: {e}", level.map));
            assert_eq!(map.name.as_deref(), Some(level.map.as_str()), "a level's map carries its name");
        }
    }

    #[test]
    fn a_broken_list_is_refused_by_name() {
        assert!(Levels::parse("").is_err(), "empty");
        let twice = "[[level]]\nmap = \"a\"\ntitle = \"A\"\n[[level]]\nmap = \"a\"\ntitle = \"B\"\n";
        assert!(Levels::parse(twice).unwrap_err().contains("twice"));
        assert!(Levels::parse("[[level]]\nmap = \"../x\"\ntitle = \"X\"\n").unwrap_err().contains("not a map name"));
        assert!(Levels::parse("[[level]]\nmap = \"x\"\ntitle = \" \"\n").unwrap_err().contains("no title"));
        assert!(Levels::parse("[[level]]\nmap = \"x\"\ntitle = \"X\"\nwaves = 3\n").is_err(), "an unknown key is a typo");
    }

    #[test]
    fn a_level_is_found_by_number_or_by_name() {
        let levels = Levels::parse("[[level]]\nmap = \"a\"\ntitle = \"A\"\n[[level]]\nmap = \"b\"\ntitle = \"B\"\n").unwrap();
        assert_eq!(levels.find("1"), Some(0));
        assert_eq!(levels.find(" 2 "), Some(1));
        assert_eq!(levels.find("3"), None);
        assert_eq!(levels.find("0"), None);
        assert_eq!(levels.find("b"), Some(1));
        assert_eq!(levels.find("c"), None);
    }

    /// A skippable first level is level 0, so the first real level is
    /// still level 1, and `--level` takes the numbers as shown.
    #[test]
    fn a_skippable_first_level_is_level_zero() {
        let levels = Levels::parse("[[level]]\nmap = \"a\"\ntitle = \"A\"\nskippable = true\n[[level]]\nmap = \"b\"\ntitle = \"B\"\n").unwrap();
        assert_eq!((levels.number(0), levels.number(1), levels.last_number()), (0, 1, 1));
        assert_eq!((levels.find("0"), levels.find("1"), levels.find("2")), (Some(0), Some(1), None));
        let shipped = Levels::shipped();
        assert_eq!((shipped.number(0), shipped.get(0).map(|l| l.map.as_str())), (0, Some("boot-camp")));
    }

    fn three() -> Levels {
        Levels::parse(
            "[[level]]\nmap = \"lotus-lagoon\"\ntitle = \"A\"\n[[level]]\nmap = \"carnival\"\ntitle = \"B\"\n[[level]]\nmap = \"scrapyard\"\ntitle = \"C\"\n",
        )
        .unwrap()
    }

    /// Only a win moves the progress, only forward, and the store hears
    /// of each move once.
    #[test]
    fn a_win_opens_the_next_level_once() {
        let mut c = Campaign::new(three(), None);
        assert_eq!((c.reached(), c.take_unsaved()), (0, None));
        c.won(0);
        assert_eq!(c.reached(), 1);
        assert_eq!(c.take_unsaved().as_deref(), Some("carnival"));
        assert_eq!(c.take_unsaved(), None, "told once");
        c.won(0);
        assert_eq!((c.reached(), c.take_unsaved()), (1, None), "winning an earlier level again moves nothing");
        c.won(2);
        assert_eq!((c.reached(), c.take_unsaved().as_deref()), (2, Some("scrapyard")), "the last opens only itself");
        assert_eq!((c.next(2), c.is_last(2), c.is_last(1)), (0, true, false), "and the one after it is the first");
    }

    #[test]
    fn saved_progress_is_read_by_name() {
        assert_eq!(Campaign::new(three(), Some("scrapyard")).reached(), 2);
        assert_eq!(Campaign::new(three(), Some("no-such-level")).reached(), 0, "a level the list lost starts over");
        assert_eq!(parse_progress(&progress_text("carnival")).as_deref(), Some("carnival"));
        assert_eq!(parse_progress(""), None);
        assert_eq!(parse_progress("level = \"\""), None);
        assert_eq!(parse_progress("not toml at all ["), None);
    }

    /// A level edited in the builder is that level for the rest of the
    /// session; a map under another name is nobody's edit.
    #[test]
    fn an_edit_is_the_level_from_then_on() {
        let mut c = Campaign::new(three(), None);
        let shipped = c.map(1).expect("carnival opens");
        assert_eq!(shipped.name.as_deref(), Some("carnival"));
        let mut edited = shipped.clone();
        edited.tanks = Some(1);
        c.remember_edit(&edited);
        assert_eq!(c.map(1).unwrap(), edited);
        let mut other = edited.clone();
        other.name = Some("my-carnival".into());
        other.tanks = Some(2);
        c.remember_edit(&other);
        assert_eq!(c.map(1).unwrap(), edited, "a map under another name is free play");
        assert!(c.map(3).is_err());
    }

    #[test]
    fn progress_round_trips_through_a_file() {
        let dir = std::env::temp_dir().join(format!("bongbong-progress-{}", std::process::id()));
        let path = dir.join("nested").join("progress.toml");
        assert_eq!(read_progress(&path), None, "no file yet");
        write_progress(&path, "hedge-maze").expect("writes");
        assert_eq!(read_progress(&path).as_deref(), Some("hedge-maze"));
        write_progress(&path, "carnival").expect("overwrites");
        assert_eq!(read_progress(&path).as_deref(), Some("carnival"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
