//! The canvas as a round stands it (docs/game-editor-fusion.md §5): the
//! map's tiles, trees, grass, woods and actors dressed in the looks of the
//! round BUILD paused (`RoundLook`) and drawn by the round's own painters
//! (`game::paint_tile_run`, `game::paint_tree`, `grass::draw_tuft`,
//! `tank::draw_tank`, `bong::draw_bong`), so a wall run, a grove, a meadow
//! or the start reads the same in Build as it did in Play a moment before.
//! Worked out whole when the canvas opens or its map is edited whole, and
//! patched cell by cell through a stroke (`Still::patch`), never per frame;
//! headless like the rest of the edit model: the builder's painter
//! (`render.rs`) runs its stages on a `GpuCanvas`.
//!
//! What a round adds to its map and the canvas does not - damage, wear,
//! fires, the enemies, anything moving on its own - stays out: the canvas
//! is the map. What it keeps of the round is cosmetic: each wall
//! material's variant, each prop's and tree's (a tile placed since BUILD
//! draws one hashed from its cell), the ground's seed (`MapEditor::dress`),
//! the seats' chassis and the sky.

use std::collections::{BTreeSet, HashSet};

use rapier2d::prelude::RigidBodyHandle;

use crate::bong::{Bong, Side};
use crate::canvas::Canvas;
use crate::grass::GrassTuft;
use super::CellChange;
use crate::map::{cell_to_world, world_to_cell, CellObject, MapFile, Theme};
use crate::obstacle::{Looks, Material, Obstacle};
use crate::pickup::PickupKind;
use crate::tank::Tank;
use crate::tower::TowerKind;
use crate::woods::Woods;
use crate::Position;

/// What the builder keeps of the round BUILD paused to draw its canvas the
/// way that round showed it (`MapEditor::dress`): the tiles' looks, each
/// couch seat's chassis and the seed that picked its sky. Kept after BACK and PLAY until the next
/// BUILD brings a newer round's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoundLook {
    pub tiles: Looks,
    /// Each couch seat's chassis row in the round, P1 then P2: what a start
    /// draws where the map leaves its seat's chassis to the game.
    pub chassis: [Option<i32>; 2],
    /// The round's seed, which picked its sky from its map's
    /// (`MapEditor::map_sky`).
    pub seed: u64,
}

/// Something standing on the canvas that the grass is sorted against: drawn
/// back to front by where it meets the ground, as `Game::paint_standing`
/// draws a round's.
pub enum Stand {
    /// A start: the seat's tank as a round sets it down there.
    Seat(Box<Tank>),
    Bong(Box<Bong>),
    Tower { kind: TowerKind, side: Side, at: Position, heading: f32 },
    Lamp(Position),
}

impl Stand {
    /// Where it stands, its cell's middle: what the walk is sorted by,
    /// back to front.
    fn at(&self) -> Position {
        match self {
            Stand::Seat(tank) => tank.position,
            Stand::Bong(bong) => bong.position,
            Stand::Tower { at, .. } | Stand::Lamp(at) => *at,
        }
    }
}

/// The most changes `Still::patch` takes in cell by cell, as
/// `CellIndex::update` does: past it the picture is worked out again whole.
const PATCH_MOST: usize = 64;

/// How far round a changed cell the grass can read it, in cells across and
/// down: a tuft keeps off the tiles two cells along its row, in the row
/// below and in the two rows above that its art spans (`grass::keep_off`),
/// and its art at the largest `grass_scale` reaches under three cells to
/// either side of its root.
const GRASS_READS: (i32, i32) = (4, 2);

/// The canvas's picture of the map at rest. See the module docs.
#[derive(Default)]
pub struct Still {
    /// The walls and props (`game::is_tile`), in the map's cell order - the
    /// order a round spawns, so shadows and caps overlap as they do there.
    tiles: Vec<Obstacle>,
    fences: HashSet<(i32, i32)>,
    /// The trees, back to front (`game::sort_trees`).
    trees: Vec<Obstacle>,
    /// The grass cells' scatters and the bushes, each beside the cell it
    /// grows on, sorted by where it is rooted.
    tufts: Vec<((i32, i32), GrassTuft)>,
    /// The woods' undergrowth, sorted by where each is rooted.
    undergrowth: Vec<GrassTuft>,
    woods: Woods,
    /// An edit taken in since the woods were worked out may have changed
    /// them (`patch`): their crowns, floor and undergrowth stand as they
    /// were until the picture is worked out again.
    stale_woods: bool,
    theme: Theme,
    /// The starts, the bongs, the towers and the lamp posts, back to front.
    standing: Vec<Stand>,
    pickups: Vec<(Position, PickupKind)>,
    oil: Vec<Position>,
    doors: Vec<Position>,
    flags: Vec<Position>,
    /// The walls' cells, whose edges join (`obstacle::neighbour_mask`).
    walls: HashSet<(i32, i32)>,
    /// The cells a tuft keeps off (`grass::keep_off`) besides the cones:
    /// what a round spawns an obstacle on, trees left out.
    solid: HashSet<(i32, i32)>,
    /// The volcanoes' cells in the map's order, the cells their cones
    /// cover (`battlefield::volcano_cone_cells`) and every cell a cone
    /// could cover (`volcano::footprint`).
    volcanoes: Vec<(i32, i32)>,
    cones: HashSet<(i32, i32)>,
    footprint: HashSet<(i32, i32)>,
    /// What stops the canvas's lights: its light-stopping tiles and the
    /// cones.
    occluders: crate::weather::Occluders,
}

/// The chassis row a start draws for `seat` (0 or 1): the map's own where
/// it names one, else the paused round's, else the first.
fn seat_row(map: &MapFile, look: &RoundLook, seat: usize) -> i32 {
    let named = if seat == 0 { map.tank } else { map.tank2 };
    named.map(crate::tank::TankKind::row).or(look.chassis[seat]).unwrap_or(0).clamp(0, crate::TANK_SHELL_VARIANT_BY_ROW.len() as i32 - 1)
}

/// A cell's place in the map's order: row by row, left to right.
fn order((col, row): (i32, i32)) -> (i32, i32) {
    (row, col)
}

impl Still {
    /// The picture of `map`, whose cells are `cells` in its own order
    /// (`CellIndex::cells`), dressed in `look`.
    pub fn build(map: &MapFile, cells: &[(i32, i32, CellObject)], look: &RoundLook) -> Still {
        let t = crate::tuning::tuning();
        let volcanoes: Vec<(i32, i32)> = cells.iter().filter(|(_, _, o)| *o == CellObject::Volcano).map(|&(c, r, _)| (c, r)).collect();
        let mut still = Still {
            woods: Woods::build(map),
            theme: map.theme,
            cones: crate::battlefield::volcano_cone_cells(map).into_iter().collect(),
            footprint: volcanoes.iter().flat_map(|&(c, r)| crate::volcano::footprint(c, r)).collect(),
            volcanoes,
            ..Still::default()
        };
        for &(col, row, obj) in cells {
            still.mark((col, row), Some(obj));
        }
        let blockers = cells.iter().filter(|(_, _, o)| o.material().is_some_and(Material::blocks_light)).map(|&(c, r, _)| (c, r));
        still.occluders = crate::weather::Occluders::over(map.field_size(), blockers.chain(still.cones.iter().copied()));
        for &(col, row, obj) in cells {
            still.place(map, look, (col, row), obj);
            still.grow(&t, (col, row), obj);
        }
        still.undergrowth = still.woods.undergrowth();
        still.undergrowth.sort_by(|a, b| a.base.y.total_cmp(&b.base.y));
        still.sort();
        still
    }

    /// Take in the cells `changes` touched as `map` holds them now, rather
    /// than working the picture out again: each changed cell is placed
    /// again, the walls round it joined again and the grass that can reach
    /// it (`GRASS_READS`) grown again - a few cells a frame of a stroke, not
    /// the map. The woods stand as they were, and an edit within their
    /// reach (`Woods::reaches`) or of a tree leaves them stale
    /// (`stale_woods`) for the caller to work the whole out again when the
    /// stroke is done. False, the picture untouched, for more than
    /// `PATCH_MOST` changes or one on a volcano's footprint (the cone it
    /// draws reads the cells under it): work it out again whole.
    pub fn patch(&mut self, map: &MapFile, look: &RoundLook, changes: &[CellChange]) -> bool {
        let volcanic = |c: &CellChange| {
            self.footprint.contains(&(c.col, c.row)) || c.before == Some(CellObject::Volcano) || c.after == Some(CellObject::Volcano)
        };
        if changes.len() > PATCH_MOST || changes.iter().any(volcanic) {
            return false;
        }
        let t = crate::tuning::tuning();
        let tree = |o: Option<CellObject>| o.and_then(|o| o.material()).is_some_and(Material::is_tree);
        self.stale_woods |= changes.iter().any(|c| tree(c.before) || tree(c.after) || self.woods.reaches((c.col, c.row)));
        // Each changed cell once, in the map's order, as the map holds it.
        let changed: BTreeSet<(i32, i32)> = changes.iter().map(|c| order((c.col, c.row))).collect();
        let now: Vec<((i32, i32), Option<CellObject>)> = changed.iter().map(|&(row, col)| ((col, row), map.cell(col, row).copied())).collect();
        for &(cell, obj) in &now {
            self.mark(cell, obj);
            let blocks = obj.and_then(|o| o.material()).is_some_and(Material::blocks_light) || self.cones.contains(&cell);
            self.occluders.put(cell.0, cell.1, blocks);
        }
        // What stood there comes off and what stands there now goes on,
        let gone = |cell: (i32, i32)| changed.contains(&order(cell));
        self.fences.retain(|&cell| !gone(cell));
        self.trees.retain(|o| !gone(o.cell()));
        self.standing.retain(|s| !gone(world_to_cell(s.at())));
        self.pickups.retain(|&(at, _)| !gone(world_to_cell(at)));
        for list in [&mut self.oil, &mut self.doors, &mut self.flags] {
            list.retain(|&at| !gone(world_to_cell(at)));
        }
        for &(cell, obj) in &now {
            let tiled = obj.is_some_and(|obj| self.place(map, look, cell, obj));
            if !tiled && let Ok(i) = self.tile_at(cell) {
                self.tiles.remove(i);
            }
        }
        // the walls round it join it again,
        for &((col, row), _) in &now {
            for n in (row - 1..=row + 1).flat_map(|r| (col - 1..=col + 1).map(move |c| (c, r))) {
                if self.walls.contains(&n)
                    && let Ok(i) = self.tile_at(n)
                {
                    self.tiles[i].edge_mask = crate::obstacle::neighbour_mask(n, &self.walls);
                }
            }
        }
        // and the grass that can reach it grows again.
        let (dc, dr) = GRASS_READS;
        let reach: BTreeSet<(i32, i32)> =
            now.iter().flat_map(|&((col, row), _)| (row - dr..=row + dr).flat_map(move |r| (col - dc..=col + dc).map(move |c| (c, r)))).collect();
        self.tufts.retain(|(cell, _)| !reach.contains(cell));
        for &cell in &reach {
            if let Some(obj) = map.cell(cell.0, cell.1).copied() {
                self.grow(&t, cell, obj);
            }
        }
        self.sort();
        true
    }

    /// An edit taken in since the woods were worked out may have changed
    /// them (`patch`).
    pub fn stale_woods(&self) -> bool {
        self.stale_woods
    }

    /// The volcanoes' cells, in the map's order.
    pub fn volcanoes(&self) -> &[(i32, i32)] {
        &self.volcanoes
    }

    /// What stops the canvas's lights (`weather::cast_lights`).
    pub fn occluders(&self) -> &crate::weather::Occluders {
        &self.occluders
    }

    /// Note what `cell` holds in the sets the tiles' edges and the grass
    /// read.
    fn mark(&mut self, cell: (i32, i32), obj: Option<CellObject>) {
        let material = obj.and_then(|o| o.material());
        let put = |set: &mut HashSet<(i32, i32)>, on: bool| {
            if on {
                set.insert(cell);
            } else {
                set.remove(&cell);
            }
        };
        put(&mut self.walls, material.is_some_and(Material::is_wall));
        put(&mut self.solid, material.is_some_and(|m| !m.is_tree()));
    }

    /// Where the tile on `cell` stands in `tiles`, or where it would.
    fn tile_at(&self, cell: (i32, i32)) -> Result<usize, usize> {
        self.tiles.binary_search_by(|o| order(o.cell()).cmp(&order(cell)))
    }

    /// Put what `obj` draws on `cell` into the picture, its grass aside
    /// (`grow`); the lists drawn back to front are put in order after
    /// (`sort`). Whether it drew a tile, which takes the place of any
    /// the cell had.
    fn place(&mut self, map: &MapFile, look: &RoundLook, cell: (i32, i32), obj: CellObject) -> bool {
        let at = cell_to_world(cell.0, cell.1);
        let tile = match obj {
            CellObject::Wall { material } => {
                let mut tile = Obstacle::new(material, look.tiles.wall(material), at, false, RigidBodyHandle::invalid());
                tile.edge_mask = crate::obstacle::neighbour_mask(cell, &self.walls);
                tile
            }
            CellObject::Tesla { .. } | CellObject::GunTower { .. } | CellObject::BioSlush { .. } => {
                let (kind, side) = obj.tower().expect("tower cells name a tower");
                let (w, h) = map.field_size();
                let heading = crate::tower::opening_heading(kind, at, Position::new(w * 0.5, h * 0.5), 0.0);
                self.standing.push(Stand::Tower { kind, side, at, heading });
                return false;
            }
            CellObject::Lamp => {
                self.standing.push(Stand::Lamp(at));
                return false;
            }
            CellObject::Door { .. } => {
                self.doors.push(at);
                return false;
            }
            CellObject::Flag => {
                self.flags.push(at);
                return false;
            }
            CellObject::Oil => {
                self.oil.push(at);
                return false;
            }
            CellObject::Pickup { pickup } => {
                self.pickups.push((at, pickup));
                return false;
            }
            CellObject::Start | CellObject::Start2 => {
                let seat = usize::from(obj == CellObject::Start2);
                let row = seat_row(map, look, seat);
                self.standing.push(Stand::Seat(Box::new(Tank {
                    row,
                    shell_variant: crate::TANK_SHELL_VARIANT_BY_ROW[row as usize],
                    position: at,
                    owner: crate::shell::Owner::Player(seat as u8),
                    ..Tank::default()
                })));
                return false;
            }
            CellObject::Bong | CellObject::EnemyBong => {
                let side = if obj == CellObject::Bong { Side::Player } else { Side::Enemy };
                self.standing.push(Stand::Bong(Box::new(Bong::at_rest(side, at, map.bong_kind(side), RigidBodyHandle::invalid()))));
                return false;
            }
            CellObject::TallGrass => return false,
            _ if obj.bush().is_some() => return false,
            _ => {
                // A prop or a tree: its own look, a pinned drum's livery.
                let Some(material) = obj.material().filter(|m| m.is_tree() || !m.is_drawn()) else { return false };
                let variant = obj.drum().map_or_else(|| look.tiles.tile(material, cell), |d| d as i32);
                let mut tile = Obstacle::new(material, variant, at, false, RigidBodyHandle::invalid());
                tile.dry = material.is_dry(map.theme);
                if material.is_tree() {
                    tile.crown = self.woods.crown(cell);
                    self.trees.push(tile);
                    return false;
                }
                if material == Material::Fence {
                    self.fences.insert(cell);
                }
                tile
            }
        };
        match self.tile_at(cell) {
            Ok(i) => self.tiles[i] = tile,
            Err(i) => self.tiles.insert(i, tile),
        }
        true
    }

    /// Put the tufts `obj` grows on `cell` into the picture: a grass cell's
    /// scatter, kept off the tiles and cones round it, or a bush.
    fn grow(&mut self, t: &crate::tuning::Tuning, cell: (i32, i32), obj: CellObject) {
        let at = cell_to_world(cell.0, cell.1);
        if obj == CellObject::TallGrass {
            let (solid, cones) = (&self.solid, &self.cones);
            let tufts = crate::grass::tufts_for_cell(t, at, |n| solid.contains(&n) || cones.contains(&n));
            self.tufts.extend(tufts.into_iter().map(|tuft| (cell, tuft)));
        } else if let Some(bush) = obj.bush() {
            self.tufts.push((cell, crate::grass::bush_tuft(at, bush)));
        }
    }

    /// Put the lists drawn back to front in order: the trees
    /// (`game::sort_trees`), the tufts by root, what stands by its foot
    /// and the rest in the map's order.
    fn sort(&mut self) {
        crate::game::sort_trees(&mut self.trees);
        let by_cell = |a: &Position, b: &Position| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x));
        self.tufts.sort_by(|a, b| by_cell(&a.1.base, &b.1.base));
        self.standing.sort_by(|a, b| by_cell(&a.at(), &b.at()));
        self.pickups.sort_by(|a, b| by_cell(&a.0, &b.0));
        for list in [&mut self.oil, &mut self.doors, &mut self.flags] {
            list.sort_by(by_cell);
        }
    }

    /// The woods' floor under the trees (`Woods::floor`), and the unlit oil
    /// trails: what lies flat under the tiles.
    pub fn paint_floor(&self, c: &mut impl Canvas) {
        if !self.woods.is_empty() {
            c.blocks(self.woods.floor());
        }
        for &at in &self.oil {
            if !c.culls(at) {
                crate::obstacle::draw_oil_cell(c, at);
            }
        }
    }

    /// The forest floor's picture, for the window's GPU copy.
    pub fn woods_floor(&self) -> Option<&crate::canvas::BlockImage> {
        (!self.woods.is_empty()).then(|| self.woods.floor())
    }

    /// The tiles as `Game::paint_tiles` draws a round's: the towers'
    /// shadows, then every wall and prop with its shadow and cap, then the
    /// training's doors and flags.
    pub fn paint_tiles(&self, c: &mut impl Canvas, time: f32) {
        for stand in &self.standing {
            if let Stand::Tower { kind, side, at, .. } = stand
                && !c.culls(*at)
            {
                crate::tower::draw_tower_shadow(c, *kind, *side, *at);
            }
        }
        crate::game::paint_tile_run(c, self.tiles.iter(), &self.fences, true, time);
        for &at in &self.doors {
            if !c.culls(at) {
                crate::training::draw_door(c, at);
            }
        }
        for &at in &self.flags {
            if !c.culls(at) {
                crate::training::draw_flag(c, at, crate::tank::team_color(0), false, time);
            }
        }
    }

    /// Everything standing, as `Game::paint_standing` draws a round's: the
    /// crates, then the seats, bongs, towers and lamp posts merged back to
    /// front with the grass, then the trees over the lot.
    pub fn paint_standing(&self, c: &mut impl Canvas, time: f32) {
        let t = crate::tuning::tuning();
        let cull = c.cull();
        for &(at, kind) in &self.pickups {
            if !crate::view::culled(cull, at) {
                crate::pickup::draw_crate(c, kind, at, crate::crate_fx::glint_col(time, at, &t), true, crate::math::Color::WHITE);
            }
        }
        let theme = self.theme;
        // The grass and the undergrowth, merged back to front.
        let (mut grass, mut under) = (0, 0);
        let mut grass_up_to = |c: &mut _, upto: f32| loop {
            let g = self.tufts.get(grass).map(|(_, tuft)| tuft).filter(|tuft| tuft.base.y <= upto);
            let u = self.undergrowth.get(under).filter(|tuft| tuft.base.y <= upto);
            let tuft = match (g, u) {
                (Some(g), Some(u)) if u.base.y < g.base.y => {
                    under += 1;
                    u
                }
                (Some(g), _) => {
                    grass += 1;
                    g
                }
                (None, Some(u)) => {
                    under += 1;
                    u
                }
                (None, None) => break,
            };
            if !crate::view::culled(cull, tuft.base) {
                crate::grass::draw_tuft(c, tuft, theme, time);
            }
        };
        for stand in &self.standing {
            grass_up_to(c, stand.at().y);
            match stand {
                Stand::Seat(tank) if !crate::view::culled(cull, tank.position) => {
                    crate::tank::draw_player_ring(c, tank, time);
                    crate::tank::draw_tank_shadow(c, tank, time);
                    crate::tank::draw_tank(c, tank, time);
                }
                Stand::Bong(bong) if !crate::view::culled(cull, bong.position) => {
                    crate::bong::draw_bong_ring(c, bong, time);
                    crate::bong::draw_bong(c, bong, time);
                }
                Stand::Tower { kind, side, at, heading } if !crate::view::culled(cull, *at) => {
                    crate::tower::draw_tower(c, *kind, *side, *at, 0, *heading, 0.0);
                }
                Stand::Lamp(at) if !crate::view::culled(cull, *at) => {
                    crate::lamp::draw_post(c, *at, time, (t.shadow_dir_x, t.shadow_dir_y), true, true);
                }
                _ => {}
            }
        }
        grass_up_to(c, f32::INFINITY);
        for tree in self.trees.iter().filter(|tree| !crate::view::culled(cull, tree.position)) {
            crate::game::paint_tree(c, tree, 0.0, 1.0, true, time);
        }
    }

    /// The lights what stands on the canvas throws at a sky whose lights
    /// show `k` (`Look::lights`), as a round's at rest throws them
    /// (`weather::lights_in`): each start's tank as its seat's, the bongs,
    /// the crates, the lamp posts and the towers, uncast.
    pub fn lights(&self, out: &mut Vec<crate::weather::Light>, k: f32, t: &crate::tuning::Tuning, time: f32) {
        use crate::weather::{bong_light, lamp_light, pickup_light, tank_lights, tower_lights};
        for stand in &self.standing {
            match stand {
                Stand::Seat(tank) => tank_lights(out, tank, true, k, t, time),
                Stand::Bong(bong) if t.pickup_glow_strength > 0.0 => out.push(bong_light(bong.position, bong.side, k, t)),
                Stand::Bong(_) => {}
                &Stand::Tower { kind, side, at, heading } => {
                    let view = crate::tower::TowerView { kind, side, position: at, stage: 0, heading, charge: 0.0, burning: false, disabled: 0.0, droop: 0.0 };
                    tower_lights(out, &view, k, t, time);
                }
                Stand::Lamp(at) => out.push(lamp_light(*at, 1.0, k, t, time)),
            }
        }
        if t.pickup_glow_strength > 0.0 {
            for &(at, kind) in &self.pickups {
                out.push(pickup_light(at, kind, k, t));
            }
        }
    }

    /// The starts' tanks, P1's first where both stand: what their labels
    /// are drawn over.
    pub fn seats(&self) -> impl Iterator<Item = &Tank> {
        self.standing.iter().filter_map(|s| match s {
            Stand::Seat(tank) => Some(&**tank),
            _ => None,
        })
    }
}

#[cfg(test)]
mod still_tests {
    use super::*;

    /// A round of `name`'s shipped map, seeded, with no enemies.
    fn round(name: &str) -> (MapFile, crate::simulation::Game) {
        let map = MapFile::load(std::path::Path::new(&format!("maps/{name}.toml"))).expect("a shipped map");
        let mut game = crate::simulation::Game::default();
        game.seed_override = Some(7);
        game.enemy_count_override = Some(0);
        game.map = map.clone();
        let (w, h) = map.field_size();
        game.init(w, h);
        (map, game)
    }

    /// The canvas draws what a round of its map draws, dressed in that
    /// round's looks: every wall and prop in the same cell with the same
    /// material, variant and edge mask, every tree with the same variant
    /// and crown, every tuft rooted alike - on shipped maps with walls,
    /// props, woods, grass and bushes.
    #[test]
    fn the_still_picture_is_the_rounds() {
        for name in ["default", "jungle-temple", "boot-camp", "hedge-maze"] {
            let (map, game) = round(name);
            let look = RoundLook { tiles: game.looks().clone(), ..RoundLook::default() };
            let still = Still::build(&map, crate::editor::CellIndex::of(&map).cells(), &look);
            let key = |o: &Obstacle| (o.cell(), format!("{:?}", o.material), o.variant, o.edge_mask, o.dry, (o.crown.dx, o.crown.dy, o.crown.mirror));
            let mut q = game.world.query::<&Obstacle>();
            let obstacles: Vec<&Obstacle> = q.iter().collect();
            let mut tiles: Vec<_> = obstacles.iter().filter(|o| crate::game::is_tile(o)).map(|o| key(o)).collect();
            let mut trees: Vec<_> = obstacles.iter().filter(|o| o.material.is_tree()).map(|o| key(o)).collect();
            let (mut ours, mut our_trees): (Vec<_>, Vec<_>) = (still.tiles.iter().map(key).collect(), still.trees.iter().map(key).collect());
            for list in [&mut tiles, &mut trees, &mut ours, &mut our_trees] {
                list.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
            }
            assert_eq!(ours, tiles, "{name}: the tiles");
            assert_eq!(our_trees, trees, "{name}: the trees");
            let tuft = |t: &GrassTuft| (t.base.x.to_bits(), t.base.y.to_bits(), t.row, t.col, t.bush);
            let mut round_tufts: Vec<_> = game.grass.iter().map(tuft).collect();
            let mut our_tufts: Vec<_> = still.tufts.iter().map(|(_, t)| t).chain(&still.undergrowth).map(tuft).collect();
            round_tufts.sort_unstable_by_key(|t| (t.0, t.1, t.2, t.3));
            our_tufts.sort_unstable_by_key(|t| (t.0, t.1, t.2, t.3));
            assert_eq!(our_tufts, round_tufts, "{name}: the grass");
        }
    }

    /// What `still` draws, as text to compare two pictures by: every list
    /// in the order it is drawn, and the sets the patch reads. Without
    /// `woods` the trees are listed by cell with no crowns and the
    /// undergrowth is left out: what a stroke near a wood leaves stale.
    fn drawn(still: &Still, woods: bool) -> Vec<String> {
        let tile = |o: &Obstacle| format!("{:?} {:?} {} {} {}", o.cell(), o.material, o.variant, o.edge_mask, o.dry);
        let tuft = |t: &GrassTuft| format!("{:?} {} {} {} {:?} {:?}", t.base, t.row, t.col, t.seed, t.lean, t.bush);
        let sorted = |set: &HashSet<(i32, i32)>| {
            let mut cells: Vec<_> = set.iter().copied().collect();
            cells.sort_unstable();
            format!("{cells:?}")
        };
        let trees = if woods {
            still.trees.iter().map(|o| format!("{} {:?}", tile(o), o.crown)).collect()
        } else {
            let mut trees: Vec<_> = still.trees.iter().map(tile).collect();
            trees.sort();
            trees
        };
        let standing = still.standing.iter().map(|s| match s {
            Stand::Seat(tank) => format!("seat {} {:?} {:?}", tank.row, tank.owner, tank.position),
            Stand::Bong(bong) => format!("bong {:?} {:?} {:?}", bong.side, bong.kind, bong.position),
            Stand::Tower { kind, side, at, heading } => format!("tower {kind:?} {side:?} {at:?} {heading}"),
            Stand::Lamp(at) => format!("lamp {at:?}"),
        });
        let mut out = vec![
            still.tiles.iter().map(tile).collect::<Vec<_>>().join("; "),
            trees.join("; "),
            still.tufts.iter().map(|(cell, t)| format!("{cell:?} {}", tuft(t))).collect::<Vec<_>>().join("; "),
            standing.collect::<Vec<_>>().join("; "),
            format!("{:?} {:?} {:?} {:?}", still.pickups, still.oil, still.doors, still.flags),
            sorted(&still.fences),
            sorted(&still.walls),
            sorted(&still.solid),
            format!("{:?}", still.occluders),
        ];
        if woods {
            out.push(still.undergrowth.iter().map(tuft).collect::<Vec<_>>().join("; "));
        }
        out
    }

    /// Through a stroke the picture is patched cell by cell (`Still::patch`)
    /// to what one worked out whole draws - the walls joined, the grass
    /// kept off the new tiles, props, a bush, a start moved, crates, oil, a
    /// lamp post - and the woods, left stale by a stroke in their reach,
    /// are worked out again when it is done.
    #[test]
    fn a_stroke_patches_the_picture_to_the_one_worked_out_whole() {
        use crate::editor::{MapEditor, Tool};
        let mut map = MapFile::new();
        map.size = Some((40.0, 26.0));
        for row in 2..12 {
            for col in 2..12 {
                map.set_cell(col, row, CellObject::TallGrass);
            }
        }
        for col in 14..22 {
            map.set_cell(col, 4, CellObject::Wall { material: Material::Brick });
        }
        for row in 15..23 {
            for col in 3..13 {
                if (col + row) % 4 != 0 {
                    map.set_cell(col, row, CellObject::Tree);
                }
            }
        }
        map.set_cell(30, 3, CellObject::Start);
        map.set_cell(25, 10, CellObject::Pickup { pickup: PickupKind::Health });
        let whole = |ed: &MapEditor| Still::build(ed.map(), crate::editor::CellIndex::of(ed.map()).cells(), ed.look());
        let mut ed = MapEditor::new(map);
        let _ = ed.still();
        let strokes: Vec<(Tool, Vec<(i32, i32)>)> = vec![
            (Tool::Wall(Material::Brick), vec![(5, 5), (6, 5), (7, 5), (7, 6), (7, 7)]),
            (Tool::Wall(Material::Brick), vec![(16, 5), (16, 6), (17, 6)]),
            (Tool::Eraser, vec![(15, 4), (6, 5)]),
            (Tool::Prop(Material::Sandbag), vec![(9, 9), (10, 9)]),
            (Tool::Prop(Material::Fence), vec![(2, 9), (3, 9), (4, 9)]),
            (Tool::Bush(crate::grass::Bush::Berry), vec![(3, 3)]),
            (Tool::Start, vec![(20, 10)]),
            (Tool::Pickup(PickupKind::Health), vec![(26, 10)]),
            (Tool::OilTrail, vec![(28, 12), (29, 12)]),
            (Tool::Prop(Material::Lamp), vec![(32, 18)]),
            (Tool::Prop(Material::Tree), vec![(14, 16), (15, 16)]),
            (Tool::Wall(Material::Iron), vec![(10, 13), (11, 13), (12, 14)]),
            (Tool::Eraser, vec![(6, 18), (7, 18)]),
        ];
        for (tool, path) in strokes {
            ed.select_tool(tool);
            ed.begin_stroke(path[0], false);
            for (i, &cell) in path.iter().enumerate() {
                if i > 0 {
                    ed.stroke_to(cell);
                }
                assert!(ed.still.get().is_some(), "{tool:?} at {cell:?}: the picture is patched, not worked out again");
                let woods = !ed.still().stale_woods();
                assert_eq!(drawn(ed.still(), woods), drawn(&whole(&ed), woods), "{tool:?} at {cell:?}");
            }
            ed.finish_stroke();
            assert!(!ed.still().stale_woods());
            assert_eq!(drawn(ed.still(), true), drawn(&whole(&ed), true), "{tool:?}, the stroke done");
        }
    }

    /// A start draws the chassis its map names for its seat, else the one
    /// the paused round's seat had, else the first; a prop placed since
    /// BUILD draws a variant hashed from its cell, the same edit to edit.
    #[test]
    fn a_start_is_the_maps_chassis_else_the_rounds_and_a_new_prop_keeps_its_look() {
        let mut map = MapFile::new();
        map.size = Some((10.0, 6.0));
        map.set_cell(2, 2, CellObject::Start);
        map.set_cell(5, 2, CellObject::Start2);
        map.set_cell(7, 3, CellObject::Sandbag);
        let look = RoundLook { chassis: [Some(3), Some(5)], ..RoundLook::default() };
        let rows = |map: &MapFile| -> Vec<i32> {
            Still::build(map, crate::editor::CellIndex::of(map).cells(), &look).seats().map(|t| t.row).collect()
        };
        assert_eq!(rows(&map), vec![3, 5], "the round's");
        map.tank = crate::tank::TankKind::from_row(8);
        assert_eq!(rows(&map), vec![8, 5], "the map's own for P1");
        let still = Still::build(&map, crate::editor::CellIndex::of(&map).cells(), &look);
        let bag = still.tiles.iter().find(|o| o.cell() == (7, 3)).expect("the sandbag");
        assert_eq!(bag.variant, look.tiles.tile(Material::Sandbag, (7, 3)));
        assert!((0..Material::Sandbag.variants()).contains(&bag.variant));
    }

    /// How long the still picture of every map in `maps/` takes to work
    /// out, the slowest first. Prints; run with `--ignored --nocapture`
    /// (and `--release` for a phone's order).
    #[test]
    #[ignore]
    fn shipped_maps_still_timing() {
        let mut rows = Vec::new();
        for entry in std::fs::read_dir("maps").expect("maps/") {
            let path = entry.expect("an entry").path();
            if path.extension().is_none_or(|e| e != "toml") {
                continue;
            }
            let Ok(map) = MapFile::load(&path) else { continue };
            let index = crate::editor::CellIndex::of(&map);
            let t = std::time::Instant::now();
            let still = Still::build(&map, index.cells(), &RoundLook::default());
            let all = t.elapsed().as_secs_f64() * 1e3;
            let t = std::time::Instant::now();
            std::hint::black_box(Woods::build(&map));
            let woods = t.elapsed().as_secs_f64() * 1e3;
            rows.push((all, woods, map.cells.len(), still.tufts.len(), path.display().to_string()));
        }
        rows.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (all, woods, cells, tufts, name) in rows.iter().take(12) {
            eprintln!("{all:6.2} ms (woods {woods:5.2})  {cells:6} cells {tufts:6} tufts  {name}");
        }
    }
}
