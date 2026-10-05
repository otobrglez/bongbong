//! A training map's script and its two cells' drawing
//! (docs/training-stage.md). Headless: the data a map's `[[training.beat]]`
//! tables parse into, and the door and the flag drawn through any
//! `canvas::Canvas`. The rules that run it are `simulation/training.rs`.
//!
//! A beat is one lesson: what it starts when it begins (`BeatStart`), what
//! has to be true for it to be done (`BeatDone`), and the frog's spot and
//! lines while it runs. Beats run in order; a door cell names the beat
//! whose end opens it (`map::CellObject::Door`).

use serde::{Deserialize, Serialize};

use crate::canvas::{Canvas, Sheet};
use crate::math::{Color, Rectangle, Vec2};
use crate::pickup::PickupKind;
use crate::tank::TankKind;
use crate::{OBSTACLE_GRID_SIZE, Position};

/// A training map's script: its beats in play order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Training {
    #[serde(default)]
    pub beat: Vec<Beat>,
    /// The seat's health when the round starts, as a share of its full
    /// health (`None`: full), so a health crate has a point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_health: Option<f32>,
    /// The seat's shells when the round starts (`None`: the chassis's
    /// usual load), so the ammo crate has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_shells: Option<i32>,
}

/// One lesson of the course.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Beat {
    /// A name for the dev server and the linter; nothing keys off it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// The cell the frog stands on while this beat runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frog: Option<(i32, i32)>,
    /// The frog's lines when the beat starts, as `text` message keys.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub say: Vec<String>,
    /// The line the frog repeats when the beat has gone quiet too long.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nudge: Option<String>,
    #[serde(default, skip_serializing_if = "BeatStart::is_empty")]
    pub start: BeatStart,
    #[serde(default)]
    pub done: BeatDone,
}

/// What a beat sets going the frame it starts.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BeatStart {
    /// A shell fired at the frog from this edge of the field, along the
    /// frog's row or column: the frog is hurt by something the player
    /// cannot see yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shoot_frog: Option<Edge>,
    /// Crates air-dropped onto cells.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drop: Vec<Drop>,
    /// Enemy tanks rolled in through the map's gates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roll_in: Vec<RollInSpec>,
}

impl BeatStart {
    pub fn is_empty(&self) -> bool {
        self.shoot_frog.is_none() && self.drop.is_empty() && self.roll_in.is_empty()
    }
}

/// A side of the field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    North,
    East,
    South,
    West,
}

/// A crate dropped from the air onto `at`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Drop {
    pub kind: PickupKind,
    pub at: (i32, i32),
}

/// An enemy tank of chassis `tank` rolled in through a gate `after`
/// seconds into the beat.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RollInSpec {
    pub tank: TankKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<TrainingAi>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub after: f32,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

/// How a training tank fights.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrainingAi {
    /// It goes for the frog rather than the player.
    Dummy,
}

/// What has to hold for a beat to be done. Every condition a beat sets
/// must hold at once; a beat that sets none is done the frame it starts.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BeatDone {
    /// This many flags taken, counted over the whole round.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flags: Option<u32>,
    /// A seat took a crate of this kind since the beat began.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collect: Option<PickupKind>,
    /// Any one of these cells' tiles is gone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub destroyed: Vec<(i32, i32)>,
    /// Every one of these cells' tiles is gone: a range of drums is done
    /// once the last of them has gone up, whichever one was shot.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub destroyed_all: Vec<(i32, i32)>,
    /// The first seat's hull stands east of this column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub past_col: Option<i32>,
    /// The players' frog is at full health.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub frog_full: bool,
    /// This many enemy tanks wrecked since the beat began.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrecks: Option<u32>,
}

impl Training {
    /// The beat at `index`, 0-based.
    pub fn beat(&self, index: usize) -> Option<&Beat> {
        self.beat.get(index)
    }
}

/// The hazard band's two colours on a door's leaves: the walls sheet's
/// gold and near-black, so a door reads as machinery among iron.
const HAZARD: [Color; 2] = [Color::new(0xEE, 0xA3, 0x43, 255), Color::new(0x25, 0x25, 0x25, 255)];

/// The walls sheet's banded-iron row a door is drawn from
/// (docs/WALLS_SPEC.md, iron's third pattern).
const DOOR_ROW: i32 = 6;

/// A door cell centred on `at`: banded iron with a hazard band across its
/// middle, so a closed pen reads as a gate rather than a wall.
pub fn draw_door(c: &mut impl Canvas, at: Position) {
    let size = OBSTACLE_GRID_SIZE;
    let src = Rectangle::new(0.0, DOOR_ROW as f32 * size, size, size);
    let dest = Rectangle::new(at.x, at.y, size, size);
    c.blit(Sheet::Walls, src, dest, Vec2::new(size / 2.0, size / 2.0), 0.0, Color::WHITE);
    let (x0, y0) = ((at.x - size / 2.0) as i32, (at.y - 3.0) as i32);
    for i in 0..8 {
        c.fill_rect(x0 + i * 4, y0, 4, 6, HAZARD[(i % 2) as usize]);
    }
}

/// A flag on the cell centred on `at`: a pole and a pennant, `colour`
/// while it stands to be taken, gold once a seat has it.
pub fn draw_flag(c: &mut impl Canvas, at: Position, colour: Color, taken: bool, time: f32) {
    let (x, y) = (at.x as i32, at.y as i32);
    let pennant = if taken { HAZARD[0] } else { colour };
    c.fill_rect(x - 6, y + 8, 14, 4, Color::new(0x25, 0x25, 0x25, 90));
    c.fill_rect(x - 1, y - 14, 2, 24, Color::new(0x37, 0x37, 0x37, 255));
    // The pennant ripples a block at a time: every other column steps down
    // on the half second, never anything finer than the field's 2 px grid.
    let ripple = ((time * 4.0 + at.x * 0.1) as i32) & 1;
    for i in 0..6 {
        let drop = if i % 2 == 1 { 2 * ripple } else { 0 };
        let height = 10 - (i / 2) * 2;
        c.fill_rect(x + 1 + i * 2, y - 14 + drop, 2, height, pennant);
    }
    c.fill_rect(x - 2, y + 8, 4, 2, Color::new(0x7E, 0x7E, 0x7E, 255));
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = r#"
start_health = 0.6
start_shells = 0

[[beat]]
id = "drive"
frog = [5, 4]
say = ["frog-hello"]
done = { flags = 3 }

[[beat]]
id = "frog"
start = { shoot_frog = "east", drop = [{ kind = "frog_health", at = [31, 10] }] }
done = { frog_full = true }

[[beat]]
id = "enemy"
start = { roll_in = [{ tank = "scout", ai = "dummy", after = 1.5 }] }
done = { wrecks = 1, destroyed = [[28, 6], [28, 5]] }

[[beat]]
id = "barrels"
done = { destroyed_all = [[31, 7], [35, 6]] }
"#;

    #[test]
    fn a_script_reads_its_beats_in_order_and_writes_back_the_same() {
        let t: Training = toml::from_str(SCRIPT).expect("script parses");
        assert_eq!(t.beat.len(), 4);
        assert_eq!(t.beat[0].done.flags, Some(3));
        assert_eq!(t.beat[0].frog, Some((5, 4)));
        assert_eq!(t.beat[1].start.shoot_frog, Some(Edge::East));
        assert_eq!(t.beat[1].start.drop, vec![Drop { kind: PickupKind::FrogHealth, at: (31, 10) }]);
        assert!(t.beat[1].done.frog_full);
        let roll = t.beat[2].start.roll_in[0];
        assert_eq!((roll.tank, roll.ai, roll.after), (TankKind::Scout, Some(TrainingAi::Dummy), 1.5));
        assert_eq!(t.beat[2].done.destroyed, vec![(28, 6), (28, 5)]);
        assert_eq!(t.beat[3].done.destroyed_all, vec![(31, 7), (35, 6)]);
        let again: Training = toml::from_str(&toml::to_string(&t).expect("writes")).expect("reads back");
        assert_eq!(again, t);
    }
}
