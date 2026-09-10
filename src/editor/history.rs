//! The builder's edit history: every stroke, settings change, load and
//! reset is one `EditStep` on an `UndoStack`, so a mis-tap on a phone is
//! one undo away and a thirty-cell drag comes back in one press. Pure
//! data over `MapFile` - no drawing, no input - which is what lets the dev
//! server's `builder_undo`/`builder_redo` and the bar's buttons share it
//! and the tests below run headlessly.

use crate::level::{Mission, SpawnKind, Tier};
use crate::map::{CellObject, MapFile};
use crate::tank::TankKind;

/// How many steps the stack keeps. Past this the oldest step is dropped;
/// a map has a few hundred cells, so this is plenty for a session.
pub const UNDO_DEPTH: usize = 200;

/// The map-level keys the MAP menu edits, as one copyable value so a
/// settings change can be stored, compared and reverted like a cell edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MapSettings {
    pub tanks: Option<u32>,
    pub tank: Option<TankKind>,
    pub mission: Mission,
    pub spawn: SpawnKind,
    pub waves: Option<u32>,
    pub size: Option<u32>,
    pub growth: Option<u32>,
    pub tier_start: Option<Tier>,
    pub tier_end: Option<Tier>,
}

impl MapSettings {
    pub fn of(map: &MapFile) -> Self {
        MapSettings {
            tanks: map.tanks,
            tank: map.tank,
            mission: map.mission.kind,
            spawn: map.spawn.kind,
            waves: map.spawn.waves,
            size: map.spawn.size,
            growth: map.spawn.growth,
            tier_start: map.spawn.tier_start,
            tier_end: map.spawn.tier_end,
        }
    }

    pub fn write_to(&self, map: &mut MapFile) {
        map.tanks = self.tanks;
        map.tank = self.tank;
        map.mission.kind = self.mission;
        map.spawn.kind = self.spawn;
        map.spawn.waves = self.waves;
        map.spawn.size = self.size;
        map.spawn.growth = self.growth;
        map.spawn.tier_start = self.tier_start;
        map.spawn.tier_end = self.tier_end;
    }

    /// The names of the fields that differ between two settings, in field
    /// order - what a diff reports and what the settings panel highlights.
    pub fn changed_fields(&self, other: &MapSettings) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.tanks != other.tanks {
            out.push("tanks");
        }
        if self.tank != other.tank {
            out.push("tank");
        }
        if self.mission != other.mission {
            out.push("mission");
        }
        if self.spawn != other.spawn {
            out.push("spawn");
        }
        if self.waves != other.waves {
            out.push("waves");
        }
        if self.size != other.size {
            out.push("wave_size");
        }
        if self.growth != other.growth {
            out.push("wave_growth");
        }
        if self.tier_start != other.tier_start {
            out.push("tier_start");
        }
        if self.tier_end != other.tier_end {
            out.push("tier_end");
        }
        out
    }
}

/// One cell's before/after inside a stroke.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellChange {
    pub col: i32,
    pub row: i32,
    pub before: Option<CellObject>,
    pub after: Option<CellObject>,
}

/// One undoable step.
#[derive(Clone, Debug, PartialEq)]
pub enum EditStep {
    /// A press-drag-release on the field: every cell it changed, in the
    /// order it changed them (a singleton move records the old cell's
    /// clear and the new cell's placement as two changes).
    Cells(Vec<CellChange>),
    /// One or more MAP menu fields.
    Settings { before: MapSettings, after: MapSettings },
    /// The whole map replaced: a load or a reset.
    Map { before: Box<MapFile>, after: Box<MapFile> },
}

impl EditStep {
    /// Apply the step forwards (`redo`) or backwards (`undo`).
    fn apply(&self, map: &mut MapFile, forward: bool) {
        match self {
            EditStep::Cells(changes) => {
                // Backwards walks the changes in reverse so a cell touched
                // twice in one stroke lands on its original value.
                let apply_one = |map: &mut MapFile, c: &CellChange| {
                    let value = if forward { c.after } else { c.before };
                    match value {
                        Some(obj) => map.set_cell(c.col, c.row, obj),
                        None => map.clear_cell(c.col, c.row),
                    }
                };
                if forward {
                    changes.iter().for_each(|c| apply_one(map, c));
                } else {
                    changes.iter().rev().for_each(|c| apply_one(map, c));
                }
            }
            EditStep::Settings { before, after } => {
                if forward { after } else { before }.write_to(map);
            }
            EditStep::Map { before, after } => {
                let source = if forward { after } else { before };
                let name = map.name.take();
                *map = (**source).clone();
                map.name = name;
            }
        }
    }

    /// The cells this step touches, for tooling replies.
    pub fn cells(&self) -> Vec<CellChange> {
        match self {
            EditStep::Cells(changes) => changes.clone(),
            _ => Vec::new(),
        }
    }
}

/// The undo and redo stacks. `push` records a step that has *already*
/// been applied to the map; `undo`/`redo` apply steps and move them
/// between the two sides.
#[derive(Clone, Debug, Default)]
pub struct UndoStack {
    undo: Vec<EditStep>,
    redo: Vec<EditStep>,
}

impl UndoStack {
    /// Record an applied step. Drops the redo branch, and the oldest undo
    /// step past `UNDO_DEPTH`. An empty cell step is not recorded at all.
    pub fn push(&mut self, step: EditStep) {
        if matches!(&step, EditStep::Cells(c) if c.is_empty()) {
            return;
        }
        self.redo.clear();
        self.undo.push(step);
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
    }

    /// Revert the newest step; `Some` with that step when there was one.
    pub fn undo(&mut self, map: &mut MapFile) -> Option<EditStep> {
        let step = self.undo.pop()?;
        step.apply(map, false);
        self.redo.push(step.clone());
        Some(step)
    }

    /// Re-apply the newest undone step; `Some` with it when there was one.
    pub fn redo(&mut self, map: &mut MapFile) -> Option<EditStep> {
        let step = self.redo.pop()?;
        step.apply(map, true);
        self.undo.push(step.clone());
        Some(step)
    }

    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

/// What differs between the builder's map and its baseline.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct MapDiff {
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    pub settings: Vec<&'static str>,
}

impl MapDiff {
    pub fn between(baseline: &MapFile, current: &MapFile) -> MapDiff {
        let mut diff = MapDiff::default();
        for (col, row, obj) in current.iter_cells() {
            match baseline.cell(col, row) {
                None => diff.added += 1,
                Some(b) if b != obj => diff.changed += 1,
                Some(_) => {}
            }
        }
        for (col, row, _) in baseline.iter_cells() {
            if current.cell(col, row).is_none() {
                diff.removed += 1;
            }
        }
        diff.settings = MapSettings::of(baseline).changed_fields(&MapSettings::of(current));
        diff
    }

    pub fn is_empty(&self) -> bool {
        self.added == 0 && self.removed == 0 && self.changed == 0 && self.settings.is_empty()
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use crate::obstacle::Material;

    fn brick() -> CellObject {
        CellObject::Wall { material: Material::Brick }
    }

    #[test]
    fn a_cell_step_undoes_and_redoes_as_one() {
        let mut map = MapFile::new();
        let mut stack = UndoStack::default();
        let changes: Vec<CellChange> =
            (0..30).map(|i| CellChange { col: i, row: 3, before: None, after: Some(brick()) }).collect();
        for c in &changes {
            map.set_cell(c.col, c.row, c.after.unwrap());
        }
        stack.push(EditStep::Cells(changes));
        assert_eq!(map.cells.len(), 30);
        assert!(stack.undo(&mut map).is_some());
        assert_eq!(map.cells.len(), 0);
        assert_eq!((stack.undo_depth(), stack.redo_depth()), (0, 1));
        assert!(stack.redo(&mut map).is_some());
        assert_eq!(map.cells.len(), 30);
        assert!(stack.redo(&mut map).is_none());
    }

    #[test]
    fn a_new_edit_drops_the_redo_branch_and_the_depth_is_capped() {
        let mut map = MapFile::new();
        let mut stack = UndoStack::default();
        for i in 0..(UNDO_DEPTH + 20) as i32 {
            map.set_cell(i, 0, brick());
            stack.push(EditStep::Cells(vec![CellChange { col: i, row: 0, before: None, after: Some(brick()) }]));
        }
        assert_eq!(stack.undo_depth(), UNDO_DEPTH);
        stack.undo(&mut map);
        stack.undo(&mut map);
        assert_eq!(stack.redo_depth(), 2);
        map.set_cell(0, 5, brick());
        stack.push(EditStep::Cells(vec![CellChange { col: 0, row: 5, before: None, after: Some(brick()) }]));
        assert_eq!(stack.redo_depth(), 0);
        // An empty stroke is not a step.
        stack.push(EditStep::Cells(Vec::new()));
        assert_eq!(stack.undo_depth(), UNDO_DEPTH - 2 + 1);
    }

    #[test]
    fn a_cell_touched_twice_in_one_stroke_returns_to_its_original() {
        let mut map = MapFile::new();
        map.set_cell(4, 4, CellObject::Road);
        let mut stack = UndoStack::default();
        map.clear_cell(4, 4);
        map.set_cell(4, 4, brick());
        stack.push(EditStep::Cells(vec![
            CellChange { col: 4, row: 4, before: Some(CellObject::Road), after: None },
            CellChange { col: 4, row: 4, before: None, after: Some(brick()) },
        ]));
        stack.undo(&mut map);
        assert_eq!(map.cell(4, 4), Some(&CellObject::Road));
    }

    #[test]
    fn settings_steps_and_diffs_name_the_fields_that_changed() {
        let mut map = MapFile::new();
        let baseline = map.clone();
        let before = MapSettings::of(&map);
        let mut after = before;
        after.tanks = Some(7);
        after.mission = Mission::Hunt;
        after.write_to(&mut map);
        let mut stack = UndoStack::default();
        stack.push(EditStep::Settings { before, after });
        map.set_cell(1, 1, brick());
        let diff = MapDiff::between(&baseline, &map);
        assert_eq!(diff.added, 1);
        assert_eq!(diff.settings, vec!["tanks", "mission"]);
        stack.undo(&mut map);
        assert_eq!(map.tanks, None);
        assert_eq!(map.mission.kind, Mission::Protect);
    }

    #[test]
    fn a_map_step_keeps_the_display_name() {
        let mut map = MapFile::new();
        map.name = Some("arena".into());
        let before = map.clone();
        let mut loaded = MapFile::new();
        loaded.set_cell(2, 2, brick());
        let mut stack = UndoStack::default();
        map.cells = loaded.cells.clone();
        stack.push(EditStep::Map { before: Box::new(before), after: Box::new(loaded) });
        stack.undo(&mut map);
        assert_eq!(map.cells.len(), 0);
        assert_eq!(map.name.as_deref(), Some("arena"));
        stack.redo(&mut map);
        assert_eq!(map.cells.len(), 1);
    }
}
