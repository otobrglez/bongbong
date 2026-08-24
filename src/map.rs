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

use crate::obstacle::Material;
use crate::pickup::PickupKind;
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
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CellObject {
    Wall { material: Material },
    Road,
    Frog,
    Pickup { pickup: PickupKind },
}

/// A saved battlefield layout. Keys are `"<col>,<row>"` grid-cell strings
/// (TOML tables require string keys) - only occupied cells are stored, so a
/// mostly-empty map stays a small file.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct MapFile {
    pub version: u32,
    pub cells: HashMap<String, CellObject>,
}

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
        MapFile { version: CURRENT_VERSION, cells: HashMap::new() }
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading map {}: {e}", path.display()))?;
        Self::from_toml_str(&text).map_err(|e| format!("parsing map {}: {e}", path.display()))
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
        let text = toml::to_string_pretty(self).map_err(|e| format!("serializing map: {e}"))?;
        std::fs::write(path, text).map_err(|e| format!("writing {}: {e}", path.display()))
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

    /// Every placed cell as `(col, row, &CellObject)` - silently skips a
    /// malformed key rather than panicking, defensive against a hand-edited
    /// file (this module's own writer never produces one).
    pub fn iter_cells(&self) -> impl Iterator<Item = (i32, i32, &CellObject)> {
        self.cells
            .iter()
            .filter_map(|(key, obj)| parse_cell_key(key).map(|(col, row)| (col, row, obj)))
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
}

/// Directory saved maps live under, relative to the process's working
/// directory - same "path relative to CWD, not the binary" convention every
/// other asset path in this project already follows (see CLAUDE.md's
/// Releases section).
pub fn maps_dir() -> PathBuf {
    PathBuf::from("maps")
}

/// Every `.toml` file under `maps_dir()`, by file stem, sorted - used by the
/// editor's Load panel. An unreadable/missing directory just yields an
/// empty list rather than an error (nothing to load yet is a normal state,
/// not a failure).
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
