//! The player's own maps, kept between sessions on every platform
//! (BB-33): a shipped map the builder changed is kept as its *modified
//! copy*, `<name>-modd`, beside the original rather than over it, and is
//! what the Load list, the levels and the builder open in its place until
//! it is reverted; a map the player named themselves is kept under that
//! name. Headless: the bytes go through a `Backend` - a directory on a
//! desktop, an iPhone and Android (`Dir`), the page's `localStorage` on
//! the web (`app.rs`) - and everything here works on a `Store` over one,
//! so the tests run on `Memory`.
//!
//! `--map-modding false` keeps it switched off for a run (`app::run` sets
//! no store): the builder then saves under `map::maps_dir()` as it always
//! has, a file of a shipped map's name standing in for it, which is how
//! the shipped maps themselves are authored in a checkout.
//!
//! A modified copy names the original it was made from on its first line,
//! `# modified from <revision>` - the FNV-1a of the shipped text
//! (`map::fnv1a`), a TOML comment the map's parser skips - so a new build
//! whose original differs can ask whether to switch to it (`stale`).

use std::path::PathBuf;
use std::sync::OnceLock;

use crate::map::{self, MapFile, SHIPPED_MAPS};

/// What a modified copy's key adds to its original's name.
pub const MODIFIED_SUFFIX: &str = "-modd";

/// How a modified copy's first line starts; the original's revision
/// follows as 16 hex digits.
const BASE_PREFIX: &str = "# modified from ";

/// Where the kept maps' text lives, by key - a map's name, or a shipped
/// map's name and `MODIFIED_SUFFIX`.
pub trait Backend: Send + Sync {
    /// The text kept under `key`, if any.
    fn read(&self, key: &str) -> Option<String>;
    /// Whether anything is kept under `key` - `read` without the text.
    fn contains(&self, key: &str) -> bool {
        self.read(key).is_some()
    }
    fn write(&self, key: &str, text: &str) -> Result<(), String>;
    /// Forget `key`; forgetting what is not there is no error.
    fn remove(&self, key: &str) -> Result<(), String>;
    /// Every key kept, in any order.
    fn keys(&self) -> Vec<String>;
}

/// A directory of `<key>.toml` files: every platform but the web.
pub struct Dir(pub PathBuf);

impl Dir {
    fn path(&self, key: &str) -> PathBuf {
        self.0.join(format!("{key}.toml"))
    }
}

impl Backend for Dir {
    fn read(&self, key: &str) -> Option<String> {
        std::fs::read_to_string(self.path(key)).ok()
    }

    fn contains(&self, key: &str) -> bool {
        self.path(key).is_file()
    }

    fn write(&self, key: &str, text: &str) -> Result<(), String> {
        std::fs::create_dir_all(&self.0).map_err(|e| format!("creating {}: {e}", self.0.display()))?;
        let path = self.path(key);
        std::fs::write(&path, text).map_err(|e| format!("writing {}: {e}", path.display()))
    }

    fn remove(&self, key: &str) -> Result<(), String> {
        let path = self.path(key);
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("removing {}: {e}", path.display())),
            _ => Ok(()),
        }
    }

    fn keys(&self) -> Vec<String> {
        std::fs::read_dir(&self.0)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|s| s.to_str()) == Some("toml"))
            .filter_map(|path| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .collect()
    }
}

/// Text in memory: the tests' backend.
#[derive(Default)]
pub struct Memory(std::sync::Mutex<std::collections::BTreeMap<String, String>>);

impl Backend for Memory {
    fn read(&self, key: &str) -> Option<String> {
        self.0.lock().ok()?.get(key).cloned()
    }

    fn write(&self, key: &str, text: &str) -> Result<(), String> {
        self.0.lock().map_err(|_| "the store is poisoned".to_string())?.insert(key.to_string(), text.to_string());
        Ok(())
    }

    fn remove(&self, key: &str) -> Result<(), String> {
        self.0.lock().map_err(|_| "the store is poisoned".to_string())?.remove(key);
        Ok(())
    }

    fn keys(&self) -> Vec<String> {
        self.0.lock().map(|kept| kept.keys().cloned().collect()).unwrap_or_default()
    }
}

/// The run's store, set once at startup (`enable`); none while map
/// modding is off.
static STORE: OnceLock<Box<dyn Backend>> = OnceLock::new();

/// Keep the player's maps in `backend` for the rest of the process. Set
/// once, at startup, before anything lists, opens or saves a map; a later
/// call is ignored. Tests never call it - they make a `Store` of their own.
pub fn enable(backend: Box<dyn Backend>) {
    let _ = STORE.set(backend);
}

/// The run's store, while map modding is on.
pub fn store() -> Option<Store<'static>> {
    STORE.get().map(|backend| Store(backend.as_ref()))
}

/// Whether map modding is on this run.
pub fn enabled() -> bool {
    STORE.get().is_some()
}

/// The shipped text of the map called `name`, if it is one.
pub fn original(name: &str) -> Option<&'static str> {
    SHIPPED_MAPS.iter().find(|(n, _)| *n == name).map(|(_, text)| *text)
}

/// The key `name`'s modified copy is kept under.
pub fn modified_key(name: &str) -> String {
    format!("{name}{MODIFIED_SUFFIX}")
}

/// The revision a modified copy's first line names, if it names one.
fn base_of(text: &str) -> Option<u64> {
    let line = text.lines().next()?.strip_prefix(BASE_PREFIX)?;
    u64::from_str_radix(line.trim(), 16).ok()
}

/// What `Store::save` did with a map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Saved {
    /// Kept as the shipped map's modified copy.
    Modified,
    /// The shipped map as it ships: no copy is kept (one there is gone).
    Original,
    /// Kept under its own name.
    Own,
}

/// The player's maps in one backend.
#[derive(Clone, Copy)]
pub struct Store<'a>(pub &'a dyn Backend);

impl Store<'_> {
    /// Whether the shipped map `name` has a modified copy.
    pub fn is_modified(&self, name: &str) -> bool {
        original(name).is_some() && self.0.contains(&modified_key(name))
    }

    /// The text `map::map_source` reads the map called `name` from, while
    /// modding is on: a shipped map's modified copy, else a map kept under
    /// its own name - never one under a shipped map's, which only its
    /// modified copy stands in for -, else the shipped text. `None` for a
    /// name none of them has.
    pub fn source(&self, name: &str) -> Option<std::borrow::Cow<'static, str>> {
        use std::borrow::Cow;
        match original(name) {
            Some(text) => Some(self.0.read(&modified_key(name)).map(Cow::Owned).unwrap_or(Cow::Borrowed(text))),
            None => self.0.read(name).map(Cow::Owned),
        }
    }

    /// The names of the maps kept under their own names, sorted: what the
    /// Load list offers beside the shipped ones.
    pub fn own_maps(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .0
            .keys()
            .into_iter()
            .filter(|key| !key.ends_with(MODIFIED_SUFFIX) && original(key).is_none())
            .collect();
        names.sort();
        names
    }

    /// Keep `map` as the map called `name`: a shipped map as its modified
    /// copy, naming the original it was made from - or, where it is the
    /// original again, with no copy at all -, any other under `name`.
    pub fn save(&self, name: &str, map: &MapFile) -> Result<Saved, String> {
        let text = map.to_toml_string()?;
        let Some(shipped) = original(name) else {
            self.0.write(name, &text)?;
            return Ok(Saved::Own);
        };
        let as_shipped = MapFile::from_toml_str(shipped).map_err(|e| format!("the shipped map {name}: {e}"))?;
        if map.revision() == as_shipped.revision() {
            self.0.remove(&modified_key(name))?;
            return Ok(Saved::Original);
        }
        let base = map::fnv1a(shipped.as_bytes());
        self.0.write(&modified_key(name), &format!("{BASE_PREFIX}{base:016x}\n{text}"))?;
        Ok(Saved::Modified)
    }

    /// Forget the shipped map `name`'s modified copy, so the original is
    /// what opens again.
    pub fn revert(&self, name: &str) -> Result<(), String> {
        self.0.remove(&modified_key(name))
    }

    /// Whether the shipped map `name` has a modified copy made from
    /// another original than the one this build ships - a copy that names
    /// none counts as made from another.
    pub fn stale(&self, name: &str) -> bool {
        let Some(shipped) = original(name) else { return false };
        self.0
            .read(&modified_key(name))
            .is_some_and(|text| base_of(&text) != Some(map::fnv1a(shipped.as_bytes())))
    }

    /// Keep `name`'s modified copy as it is, as made from the original
    /// this build ships: the player chose their copy over a changed
    /// original, so `stale` asks no more.
    pub fn keep(&self, name: &str) -> Result<(), String> {
        let (Some(shipped), Some(text)) = (original(name), self.0.read(&modified_key(name))) else {
            return Ok(());
        };
        let body = if base_of(&text).is_some() { text.split_once('\n').map_or("", |(_, rest)| rest) } else { text.as_str() };
        let base = map::fnv1a(shipped.as_bytes());
        self.0.write(&modified_key(name), &format!("{BASE_PREFIX}{base:016x}\n{body}"))
    }
}

/// A question about a kept map, asked over the round or the builder
/// (`mode::Session::question`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Question {
    /// FILE > REVERT TO ORIGINAL: forget the modified copy of `name`?
    Revert { name: String },
    /// The original `name`'s modified copy was made from is not the one
    /// this build ships: switch to it and forget the copy?
    OriginalChanged { name: String },
}

impl Question {
    /// The map it is about.
    pub fn name(&self) -> &str {
        match self {
            Question::Revert { name } | Question::OriginalChanged { name } => name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped(name: &str) -> MapFile {
        let mut map = MapFile::from_toml_str(original(name).expect("shipped")).expect("parses");
        map.name = Some(name.to_string());
        map
    }

    /// An edit of `map`: one more wall in the top-left corner's cell, or
    /// one fewer where it had one.
    fn edited(mut map: MapFile) -> MapFile {
        if map.cell(1, 1).is_some() {
            map.cells.remove("1,1");
        } else {
            map.set_cell(1, 1, map::CellObject::Wall { material: crate::obstacle::Material::Iron });
        }
        map
    }

    /// A shipped map changed is kept as its modified copy beside the
    /// original, is what opens in its place, and goes on a revert.
    #[test]
    fn a_shipped_map_is_kept_as_its_modified_copy() {
        let memory = Memory::default();
        let store = Store(&memory);
        assert!(!store.is_modified("default"));
        assert_eq!(store.save("default", &edited(shipped("default"))), Ok(Saved::Modified));
        assert!(memory.contains("default-modd") && !memory.contains("default"), "kept beside the original: {:?}", memory.keys());
        assert!(store.is_modified("default"));
        let source = store.source("default").expect("opens");
        let opened = MapFile::from_toml_str(&source).expect("the copy parses");
        assert_eq!(opened.revision(), edited(shipped("default")).revision());
        assert!(store.own_maps().is_empty(), "a copy is no map of its own");
        store.revert("default").expect("reverts");
        assert!(!store.is_modified("default"));
        assert_eq!(store.source("default").as_deref(), original("default"));
    }

    /// A shipped map saved as it ships keeps no copy, and takes away one
    /// that was there - undoing every edit is a revert.
    #[test]
    fn the_original_again_keeps_no_copy() {
        let memory = Memory::default();
        let store = Store(&memory);
        store.save("portals", &edited(shipped("portals"))).expect("saves");
        assert!(store.is_modified("portals"));
        assert_eq!(store.save("portals", &shipped("portals")), Ok(Saved::Original));
        assert!(!store.is_modified("portals"));
        assert!(memory.keys().is_empty());
    }

    /// A map of the player's own name is kept under it and listed; a
    /// file under a shipped map's own name stands in for nothing.
    #[test]
    fn own_maps_are_kept_under_their_names() {
        let memory = Memory::default();
        let store = Store(&memory);
        assert_eq!(store.save("my-fort", &shipped("towers")), Ok(Saved::Own));
        memory.write("default", "not a map").expect("writes");
        assert_eq!(store.own_maps(), vec!["my-fort".to_string()]);
        assert!(store.source("my-fort").is_some());
        assert_eq!(store.source("default").as_deref(), original("default"), "only a modified copy stands in for a shipped map");
        assert_eq!(store.source("nowhere"), None);
    }

    /// A copy made from the original this build ships is not stale; one
    /// made from another (or naming none) is, until the player keeps it.
    #[test]
    fn a_copy_of_another_original_is_stale_until_kept() {
        let memory = Memory::default();
        let store = Store(&memory);
        store.save("default", &edited(shipped("default"))).expect("saves");
        assert!(!store.stale("default"));
        let text = memory.read("default-modd").expect("kept");
        let body = text.split_once('\n').expect("a header").1.to_string();
        memory.write("default-modd", &format!("{BASE_PREFIX}0123456789abcdef\n{body}")).expect("writes");
        assert!(store.stale("default"));
        store.keep("default").expect("keeps");
        assert!(!store.stale("default"));
        assert_eq!(memory.read("default-modd").map(|t| t.split_once('\n').map(|(_, b)| b.to_string())), Some(Some(body.clone())));
        memory.write("default-modd", &body).expect("writes");
        assert!(store.stale("default"), "a copy naming no original is stale");
        store.keep("default").expect("keeps");
        assert!(!store.stale("default"));
        assert!(MapFile::from_toml_str(&memory.read("default-modd").expect("kept")).is_ok(), "the header is a comment");
    }

    /// The directory backend round-trips text and forgets what is gone.
    #[test]
    fn the_directory_backend_round_trips() {
        let dir = std::env::temp_dir().join(format!("bongbong-mapstore-{}", std::process::id()));
        let backend = Dir(dir.clone());
        assert!(backend.keys().is_empty() && !backend.contains("a"));
        backend.write("a-modd", "x = 1\n").expect("writes");
        assert!(backend.contains("a-modd"));
        assert_eq!(backend.read("a-modd").as_deref(), Some("x = 1\n"));
        assert_eq!(backend.keys(), vec!["a-modd".to_string()]);
        backend.remove("a-modd").expect("removes");
        backend.remove("a-modd").expect("removing what is gone is no error");
        assert!(!backend.contains("a-modd"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
