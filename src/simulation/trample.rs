//! Trampled cover (docs/ground-memory.md "Trampled cover"): a tall-grass,
//! bush or reed cell that hulls have driven into `grass_trample_passes`
//! times hides nobody for the rest of the round. A small set kept here in
//! the round - not the cosmetic wear grid, which is capped and each
//! client's own - so the AI's sight, the off-screen arrows and a replica
//! all read one answer through `Game::cover_cells`. No RNG, ordered maps
//! only; off (`trample_passes` 0) touches nothing.

use hecs::Entity;

use super::{Event, Frame, Game};
use crate::tank::Tank;

impl Game {
    /// Count every hull's centre entering a grass cell, and trample a
    /// cell on its `trample_passes`th pass: `GrassTrampled`, out of the
    /// cover for the round. Live hulls only - seats in index order, then
    /// enemies - from the poses `sync_tanks_and_ram` just left. The first
    /// cell a hull is seen in is where it appeared, not where it drove, so
    /// it counts nothing; a parked hull counts its cell once.
    pub(super) fn trample_grass(&mut self, f: &mut Frame) {
        if self.trample_passes == 0 || self.grass_cells.is_empty() {
            return;
        }
        let mut hulls: Vec<(usize, Entity)> =
            self.world.query::<(Entity, &Tank)>().iter().filter(|(_, t)| !t.is_wreck()).map(|(e, t)| (t.owner_slot(), e)).collect();
        hulls.sort_unstable_by_key(|&(slot, _)| slot);
        for (_, entity) in hulls {
            let entered = {
                let mut q = self.world.query_one::<&mut Tank>(entity);
                let Ok(tank) = q.get() else { continue };
                let cell = crate::grass::hull_cell(tank.trample_cell, tank.position);
                let entered = tank.trample_cell.is_some_and(|last| last != cell).then_some(cell);
                tank.trample_cell = Some(cell);
                entered
            };
            let Some(cell) = entered else { continue };
            if self.grass_trampled.contains(&cell) || !self.is_grass_cell(cell) {
                continue;
            }
            let passes = self.grass_passes.entry(cell).or_insert(0);
            *passes += 1;
            if *passes >= self.trample_passes {
                self.grass_passes.remove(&cell);
                self.grass_trampled.insert(cell);
                f.events.push(Event::GrassTrampled { cell });
            }
        }
    }

    /// Hold every tuft rooted in a trampled cell flat (`crush` 1), on the
    /// round and on a replica alike (`tick_grass`). Burnt tufts are left as
    /// the fire left them.
    pub(super) fn hold_trampled_grass(&mut self) {
        if self.grass_trampled.is_empty() {
            return;
        }
        for tuft in self.grass.iter_mut().filter(|g| !g.burnt) {
            if self.grass_trampled.contains(&crate::map::world_to_cell(tuft.base)) {
                tuft.crush = 1.0;
            }
        }
    }

    /// Is map cell `cell` one of the map's grass (or bush, or reed) cells?
    fn is_grass_cell(&self, cell: (i32, i32)) -> bool {
        self.grass_cells.iter().any(|&c| crate::map::world_to_cell(c) == cell)
    }
}
