//! The mushroom hunt's rules (docs/mushroom-hunt-prd.md): the map's
//! mushrooms laid out at `init`, and the phase that hands each to the first
//! seat whose hull reaches it. A `forage` round is won the frame the last
//! one goes (`check_round_end`); in any other mission they are taken all
//! the same and count for nothing. `mushroom.rs` is the data and the
//! drawing.
//!
//! No RNG: the map places them, the seats are walked in index order, so
//! two seats on one mushroom in one frame give it to the lower seat. A map
//! without mushrooms runs none of this and replays as before.

use super::*;
use crate::mushroom::{Mushroom, MUSHROOM_MAX};

impl Game {
    /// Lay the map's mushrooms out, none taken: from `init`, after the
    /// map. Cells past `MUSHROOM_MAX` are left out in `iter_cells` order.
    pub(super) fn init_mushrooms(&mut self) {
        self.mushrooms = self
            .map
            .iter_cells()
            .filter(|(_, _, o)| matches!(o, CellObject::Mushroom))
            .take(MUSHROOM_MAX)
            .map(|(col, row, _)| Mushroom::new(col, row))
            .collect();
    }

    /// Hand each mushroom still out to the first live seat on the field
    /// whose hull box reaches it, seats in index order.
    pub(super) fn mushroom_phase(&mut self, f: &mut Frame) {
        if self.mushrooms.iter().all(|m| m.taken) {
            return;
        }
        let pad = tuning().mushroom_collect_pad_px;
        let hulls: Vec<(usize, Position, Position)> = self
            .seats_on_field()
            .into_iter()
            .enumerate()
            .filter_map(|(seat, e)| e.map(|e| (seat, e)))
            .filter(|&(_, e)| !with_tank(&self.world, e, |t| t.is_wreck()))
            .map(|(seat, e)| {
                let (center, half) = with_tank(&self.world, e, |t| t.hull_bbox_world());
                (seat, center, half)
            })
            .collect();
        let mut left = self.mushrooms_left();
        for m in self.mushrooms.iter_mut().filter(|m| !m.taken) {
            if let Some(&(seat, _, _)) = hulls.iter().find(|&&(_, center, half)| m.in_reach(center, half, pad)) {
                m.take(self.time);
                left -= 1;
                f.events.push(Event::MushroomTaken { seat: seat as u8, col: m.col as i16, row: m.row as i16, left: left as u16 });
            }
        }
    }

    /// The round's mushrooms, taken or not, in map order.
    pub fn mushrooms(&self) -> &[Mushroom] {
        &self.mushrooms
    }

    /// How many mushrooms are still out.
    pub fn mushrooms_left(&self) -> usize {
        self.mushrooms.iter().filter(|m| !m.taken).count()
    }

    /// Every mushroom of a map that has any is taken: a `forage` round's
    /// win.
    pub(super) fn all_mushrooms_taken(&self) -> bool {
        !self.mushrooms.is_empty() && self.mushrooms_left() == 0
    }

    /// The mushrooms as one bit each, set once taken: what a snapshot
    /// carries (`net::wire::Snapshot::mushrooms`).
    pub fn mushrooms_taken_mask(&self) -> u64 {
        self.mushrooms.iter().enumerate().filter(|(_, m)| m.taken).fold(0, |mask, (i, _)| mask | (1 << i))
    }

    /// Set every mushroom's `taken` from a snapshot's mask (a replica): a
    /// mushroom the mask newly takes pops from the replica's clock now.
    pub fn set_mushrooms_taken(&mut self, mask: u64) {
        let now = self.time;
        for (i, m) in self.mushrooms.iter_mut().enumerate() {
            match (m.taken, mask & (1 << i) != 0) {
                (false, true) => m.take(now),
                (true, false) => *m = crate::mushroom::Mushroom::new(m.col, m.row),
                _ => {}
            }
        }
    }
}
