//! Oil prints (docs/ground-memory.md "Oil prints"): a hull that drives
//! through an unlit oil cell carries oil on its treads and prints it on
//! the next `oil_print_cells` cells its centre enters. A print is an
//! oil-trail cell like any the map paints - it joins `Game::oil_cells`, so
//! the flamethrower, a drum's blast, a rod and a burning neighbour light
//! it, the fire runs along the prints on the trail's clock
//! (`oil_trail_cells_per_second`, `oil_trail_burn_seconds`) and
//! `fire_burnt_out` spends it. `Game::oil_prints` only says which of the
//! oil cells are prints and how each is drawn.
//!
//! The room lays them; a replica learns each from `Event::OilPrinted` and
//! a joiner from `Welcome::oil_prints`, so a client burns where the room
//! burns. No RNG: the walk goes seats in index order, then enemies by
//! owner slot, and a print is a pure function of the hull's step.

use super::*;
use crate::ground::Depth;
use crate::map::{cell_to_world, world_to_cell};

/// One oil print: an oil-trail cell a hull laid, with how it is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OilPrint {
    /// The way the hull was going when it printed the cell.
    pub dir: Dir,
    /// How much oil was left on the treads, in eighths of a full load: 8
    /// for the first cell out of the pool, fainter after.
    pub fade: u8,
}

impl Game {
    /// Every hull on the field that is carrying oil prints the cell its
    /// centre has just entered; one that has just entered an oil pool
    /// takes on a full load. Water and lava wash the load off. Off
    /// (`oil_print_cells` 0) the phase touches nothing.
    pub(super) fn print_oil(&mut self, f: &mut Frame) {
        let load = self.oil_print_cells;
        if load <= 0 {
            return;
        }
        let load = load.min(u8::MAX as i32) as u8;
        let mut order: Vec<Entity> = self.seats_on_field().into_iter().flatten().collect();
        let mut enemies: Vec<(usize, Entity)> =
            self.world.query::<(Entity, &Tank)>().with::<&Ai>().without::<&RollIn>().iter().map(|(e, t)| (t.owner_slot(), e)).collect();
        enemies.sort_unstable_by_key(|&(slot, _)| slot);
        order.extend(enemies.into_iter().map(|(_, e)| e));
        for entity in order {
            let (cell, from, carried, pos) = {
                let mut q = self.world.query_one::<&mut Tank>(entity);
                let Ok(tank) = q.get() else { continue };
                if tank.is_wreck() {
                    tank.oil_load = 0;
                    tank.oil_cell = None;
                    continue;
                }
                let cell = world_to_cell(tank.position);
                let from = tank.oil_cell.replace(cell);
                if from == Some(cell) {
                    continue;
                }
                (cell, from, tank.oil_load, tank.position)
            };
            let wet = self.water.depth_at(pos) != Depth::Dry || self.lava.depth_at(pos) != Depth::Dry;
            let left = if wet {
                0
            } else if self.is_oil_pool(cell) {
                load
            } else if carried == 0 {
                0
            } else if let Some(from) = from {
                self.print_step(f, from, cell, carried, load)
            } else {
                carried
            };
            if left != carried {
                let mut q = self.world.query_one::<&mut Tank>(entity);
                if let Ok(tank) = q.get() {
                    tank.oil_load = left;
                }
            }
        }
    }

    /// A hull carrying `carried` cells of oil stepped from `from` into
    /// `to`: print what the step covers and return the load left. A jump
    /// (a portal, a placement) prints nothing across the gap and keeps the
    /// load; a diagonal step prints the orthogonal cell between first, so
    /// the fire's four-way spread still runs along the prints.
    fn print_step(&mut self, f: &mut Frame, from: (i32, i32), to: (i32, i32), carried: u8, load: u8) -> u8 {
        let (dc, dr) = (to.0 - from.0, to.1 - from.1);
        if dc.abs() > 1 || dr.abs() > 1 {
            return carried;
        }
        let mut steps: Vec<((i32, i32), Dir)> = Vec::with_capacity(2);
        if dc != 0 && dr != 0 {
            steps.push(((from.0 + dc, from.1), Dir::of_step(dc, 0).unwrap_or(Dir::Right)));
            steps.push((to, Dir::of_step(0, dr).unwrap_or(Dir::Down)));
        } else if let Some(dir) = Dir::of_step(dc, dr) {
            steps.push((to, dir));
        }
        let mut left = carried;
        for (cell, dir) in steps {
            if left == 0 {
                break;
            }
            self.lay_oil_print(f, cell, OilPrint { dir, fade: print_fade(left, load) });
            left -= 1;
        }
        left
    }

    /// An unlit oil cell the map painted: in `oil_cells`, not a print and
    /// not burning. Prints never load a hull, so oil cannot spread without
    /// end.
    fn is_oil_pool(&self, cell: (i32, i32)) -> bool {
        self.oil_cells.contains(&cell) && !self.oil_prints.contains_key(&cell) && !self.fires.iter().any(|fire| fire.cell == cell)
    }

    /// Lay `print` on `cell` unless the cell cannot take one: already oil,
    /// burning, off the field, wet, or under a standing tile. A print laid
    /// beside a fire whose spread is spent catches at once, as a trail cell
    /// beside a burning one would have.
    fn lay_oil_print(&mut self, f: &mut Frame, cell: (i32, i32), print: OilPrint) {
        if self.oil_cells.contains(&cell) || self.fires.iter().any(|fire| fire.cell == cell) {
            return;
        }
        let pos = cell_to_world(cell.0, cell.1);
        if pos.x < 0.0 || pos.y < 0.0 || pos.x > f.width || pos.y > f.height {
            return;
        }
        if self.water.depth_at(pos) != Depth::Dry || self.lava.depth_at(pos) != Depth::Dry {
            return;
        }
        if self.world.query::<&Obstacle>().iter().any(|o| !o.destroyed && o.cell() == cell) {
            return;
        }
        self.put_oil_print(cell, print);
        f.events.push(Event::OilPrinted { cell, dir: print.dir.name(), fade: print.fade });
        let catches = props::neighbours(cell)
            .iter()
            .any(|n| self.fires.iter().any(|fire| fire.cell == *n && fire.left > 0.0 && fire.spread_at.is_none()));
        if catches {
            self.light_cell(f, cell, tuning().oil_trail_burn_seconds, false);
        }
    }

    /// Make `cell` an oil print: one of the oil cells, drawn as `print`.
    /// The one place a print is added, by the round and by a replica
    /// (`net::apply`).
    pub(crate) fn put_oil_print(&mut self, cell: (i32, i32), print: OilPrint) {
        self.oil_cells.insert(cell);
        self.oil_prints.insert(cell, print);
    }
}

/// The fade of a print laid with `left` of a full `load` still on the
/// treads, in eighths rounded up: 8 for the first cell out of the pool,
/// never below 1.
fn print_fade(left: u8, load: u8) -> u8 {
    let load = load.max(1) as u32;
    ((8 * left as u32).div_ceil(load)).clamp(1, 8) as u8
}

#[cfg(test)]
mod tests {
    use super::print_fade;

    #[test]
    fn a_load_fades_in_eighths_from_full() {
        assert_eq!([4, 3, 2, 1].map(|left| print_fade(left, 4)), [8, 6, 4, 2]);
        assert_eq!(print_fade(1, 16), 1, "never below one eighth");
        assert_eq!(print_fade(1, 1), 8);
    }
}
