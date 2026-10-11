//! The nav grid (`pathfind::Grid`): the occupancy grid the AI routes by,
//! built the same way by the map linter (`Game::nav_grid`), and the
//! routing grid every frame hands the enemies, priced and with a flow
//! field per shared target (`Game::route_grid`).
//!
//! **Kept across frames** (docs/large-maps-follow-camera.md section 12):
//! building the occupancy and labelling its components is most of what a
//! routing grid costs on a field map, and both change only when the
//! terrain does - a tile comes or goes, a frog hops, a fire starts or
//! burns out. So the round keeps them (`NavCache`, brought up to date by
//! `Game::refresh_nav` at the top of every frame) in two layers:
//!
//! - the **layer**: the field's boundary, every tile at its seam-closed
//!   extent and the deep water. A tile's own extent depends on its
//!   neighbours, so the layer is rebuilt whole, and only when the world's
//!   tiles are no longer the ones it was built from - read every frame in
//!   the world's own order, position and material, about a microsecond on
//!   the study map - or the field's size or a knob the grid reads has
//!   changed;
//! - the **base**: the layer with the frogs and the burning cells blocked
//!   (stamped on a copy every frame, a handful of shapes), the portals
//!   found on it, the fords priced and the components labelled. It is
//!   rebuilt only when that occupancy comes out different from the one it
//!   was built on.
//!
//! A frame's routing grid then starts from a copy of the base and adds
//! what only the frame decides: the surcharges and the fields
//! (`Game::route_grid_on`). Blocking a cell only ever adds and everything
//! after it - the portals' footprints, the prices, the labels - is a
//! function of the occupancy, the portals' anchors and their knobs, so the
//! base is the grid `nav_grid` builds from scratch, labelled, cell for
//! cell; the tests hold every frame of several rounds to that. What only
//! `Game::init` sets - the water and the portals - empties the cache there.

use std::collections::HashSet;

use crate::battlefield;
use crate::frog::Frog;
use crate::obstacle::{Material, Obstacle};
use crate::pathfind::Grid;
use crate::tank::{Dir, Tank};
use crate::tuning::tuning;
use crate::{FROG_COLLIDER_HALF_EXTENT, OBSTACLE_GRID_SIZE, PATHFIND_CELL_SIZE, Position};

use super::{Ai, Game, with_frog, with_tank};

/// What the layer is built under besides the tiles: the field's size and
/// the knobs the grid reads, as bits so the comparison is exact.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Setting {
    width: u32,
    height: u32,
    portal_radius: u32,
    portal_hop_cost: u32,
    ford_cost: i32,
    lava_cost: i32,
    crater_cost: i32,
    rubble_costs: (i32, i32),
    rubble_levels: (i32, i32),
}

impl Setting {
    fn now(width: f32, height: f32) -> Self {
        let t = tuning();
        Self {
            width: width.to_bits(),
            height: height.to_bits(),
            portal_radius: t.portal_trigger_radius.to_bits(),
            portal_hop_cost: t.portal_hop_cost.to_bits(),
            ford_cost: t.water_ford_path_cost,
            lava_cost: t.lava_ford_path_cost,
            crater_cost: t.rod_crater_path_cost,
            rubble_costs: (t.rubble_light_path_cost, t.rubble_heavy_path_cost),
            rubble_levels: (t.rubble_light, t.rubble_heavy),
        }
    }
}

/// One tile as the layer reads it: where it stands and what it is - the
/// material decides whether it closes a seam and whether its footprint is
/// a tree's.
#[derive(Clone, Copy, PartialEq, Debug)]
struct TileKey {
    x: u32,
    y: u32,
    material: Material,
}

impl TileKey {
    fn of(o: &Obstacle) -> Self {
        Self { x: o.position.x.to_bits(), y: o.position.y.to_bits(), material: o.material }
    }
}

/// The nav grid the round keeps across frames - see the module doc.
/// `Game::init` empties it; `Game::refresh_nav` fills it. A `Grid` keeps
/// its fields' searches in `RefCell`s, so a round that keeps one is `Send`
/// but not `Sync`: a room or the rig moves its round onto its own thread,
/// and nothing shares one between threads.
#[derive(Default)]
pub(crate) struct NavCache {
    /// What `layer` was built under, `None` until it is.
    setting: Option<Setting>,
    /// The tiles `layer` was built from, in the world's order.
    tiles: Vec<TileKey>,
    /// The boundary, the tiles and the deep water blocked.
    layer: Option<Grid>,
    /// `layer` with the frogs and the burning cells blocked, its portals,
    /// its prices and its labels: `nav_grid` labelled, as of the last
    /// `refresh_nav`.
    base: Option<Grid>,
    /// Tests: how many times each was built, so a test can see both kept
    /// and rebuilt.
    #[cfg(test)]
    built: (u32, u32),
}

impl NavCache {
    /// The base as of the last `Game::refresh_nav`.
    pub(crate) fn base(&self) -> &Grid {
        self.base.as_ref().expect("refresh_nav runs before the base is read")
    }

    /// The layer as of the last `Game::refresh_nav`: the boundary, the
    /// tiles and the deep water, with no frog in it - what a frog walks
    /// by, since the base blocks the frog's own cells.
    pub(crate) fn layer(&self) -> &Grid {
        self.layer.as_ref().expect("refresh_nav runs before the layer is read")
    }

    /// Forget everything kept: the next `Game::refresh_nav` builds both
    /// layers anew.
    pub(crate) fn clear(&mut self) {
        *self = NavCache {
            #[cfg(test)]
            built: self.built,
            ..NavCache::default()
        };
    }
}

impl Game {
    /// The obstacle-occupancy grid the AI routes by this frame (see
    /// `pathfind::Grid`), built from scratch from the current terrain and
    /// built exactly the same way by the map linter, so the two can't
    /// drift. The margin is the worst-case tank in the roster, so no route
    /// is too narrow for a titan. The frog is included: it is a solid
    /// static body that blocks movement exactly like a tile and can move.
    /// The battlefield's own boundary needs no entry here - `Grid::build`
    /// insets its outer edge by the same margin. A frame's routing grid
    /// starts from this grid kept across frames (`refresh_nav`) rather
    /// than from a fresh one.
    ///
    /// Tiles are taken at the *seam-closed* extent
    /// (`battlefield::tile_half_extent`, the same one `hits::Terrain` and
    /// the physics colliders use), reduced to the larger axis because
    /// `Grid::build` carries one scalar per obstacle. A tile's plain
    /// `hull_size() * 0.5` is 12 px against a run's real 16 px: walls
    /// modelled *smaller* than the solver sees them, the direction of
    /// error `maplint::check_planner_physics` exists to catch.
    pub(crate) fn nav_grid(&self, width: f32, height: f32) -> Grid {
        let mut grid = Self::nav_open(width, height);
        for (center, half_extent) in self.nav_tile_shapes().into_iter().chain(self.nav_moving_shapes()) {
            grid.block(center, half_extent);
        }
        self.nav_finish(grid)
    }

    /// The field with only its boundary blocked, at the nav grid's pitch
    /// and margin.
    fn nav_open(width: f32, height: f32) -> Grid {
        Grid::open(width, height, PATHFIND_CELL_SIZE, battlefield::max_tank_clearance_half_extent())
    }

    /// What the layer blocks: every tile at its seam-closed extent and
    /// every deep water and deep lava cell (`nav_grid`).
    fn nav_tile_shapes(&self) -> Vec<(Position, f32)> {
        // Trees are left out of the seam-close set for the same reason
        // `hits::Terrain::build` leaves them out: they never close a seam,
        // here or in physics, so they must not close anybody else's.
        let seam_cells: HashSet<(i32, i32)> = self
            .world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| !o.material.is_tree())
            .map(|o| battlefield::pos_to_cell(o.position))
            .collect();
        self.world
            .query::<&Obstacle>()
            .iter()
            .map(|o| {
                let (gx, gy) = battlefield::pos_to_cell(o.position);
                let half = battlefield::tile_half_extent(o.material, &seam_cells, gx, gy, o.hull_size() * 0.5);
                (o.position, half.x.max(half.y))
            })
            // Deep water is a wall (docs/water.md): the same cells the
            // static colliders stand on, at a cell's half-extent.
            .chain(self.water.deep_cells().map(|p| (p, OBSTACLE_GRID_SIZE * 0.5)))
            // So is a lava lake's deep middle (docs/volcano.md).
            .chain(self.lava.deep_cells().map(|p| (p, OBSTACLE_GRID_SIZE * 0.5)))
            .collect()
    }

    /// What comes, goes and moves between frames: the frogs and the
    /// burning cells (`nav_grid`).
    fn nav_moving_shapes(&self) -> impl Iterator<Item = (Position, f32)> + '_ {
        let frogs: Vec<(Position, f32)> = self
            .world
            .query::<&Frog>()
            .iter()
            .map(|fr| (fr.position, FROG_COLLIDER_HALF_EXTENT.0.max(FROG_COLLIDER_HALF_EXTENT.1)))
            .collect();
        // A burning cell is a wall for as long as it burns: the AI routes
        // around a pool rather than through it. A tiny half-extent, so
        // only the margin decides how wide the detour is.
        frogs.into_iter().chain(self.fires.iter().map(|fire| (fire.position(), 1.0)))
    }

    /// An occupancy finished into a nav grid: an active portal network
    /// joined as one hub the planner may route through
    /// (`Grid::with_portals`; with fewer than two portals the grid is
    /// exactly the plain one), and the fords priced.
    fn nav_finish(&self, grid: Grid) -> Grid {
        let t = tuning();
        let mut grid = grid.with_portals(self.active_portals(), t.portal_trigger_radius, t.portal_hop_cost);
        // A ford is open but dear: the router wades only when the dry way
        // round costs more.
        grid.weigh(self.water.shallow_cells(), t.water_ford_path_cost.max(1) as u32);
        // A lava ford burns: the router crosses one only when the way round
        // is far longer (docs/volcano.md).
        grid.weigh(self.lava.ford_cells(), t.lava_ford_path_cost.max(1) as u32);
        // A rod's dry crater is a pit (docs/rod-from-god.md): dear too, a
        // filled one priced above as the ford it is. Making a crater empties
        // the kept grid, so the base is priced again.
        if !self.craters.is_empty() {
            let dry: Vec<Position> = self
                .craters
                .cells()
                .filter(|&(c, r)| self.water.depth_of_cell(c, r) == crate::ground::Depth::Dry)
                .map(|(c, r)| crate::map::cell_to_world(c, r))
                .collect();
            grid.weigh(dry.into_iter(), t.rod_crater_path_cost.max(1) as u32);
        }
        // Rubble is open but slow (`chunks::Rubble`): dearer the deeper it
        // lies. A change of level empties the kept grid
        // (`Game::lay_rubble`), so the base is priced again.
        for level in [crate::chunks::RubbleLevel::Light, crate::chunks::RubbleLevel::Heavy] {
            let cost = if level == crate::chunks::RubbleLevel::Light { t.rubble_light_path_cost } else { t.rubble_heavy_path_cost };
            let cells: Vec<Position> =
                self.rubble.cells().filter(|&(c, _)| self.rubble.level(c) == level).map(|(c, _)| crate::map::cell_to_world(c.0, c.1)).collect();
            if !cells.is_empty() {
                grid.weigh(cells.into_iter(), cost.max(1) as u32);
            }
        }
        grid
    }

    /// Bring the nav grid the round keeps (`NavCache`) up to date with the
    /// terrain, at the top of a frame: the layer rebuilt when the world's
    /// tiles, the field's size or a knob it reads have changed since it was
    /// built, the base when the layer with the frogs and the burning cells
    /// on it no longer blocks the cells the base was built on. Afterwards
    /// `self.nav.base()` is `nav_grid` labelled. No RNG.
    pub(super) fn refresh_nav(&mut self, width: f32, height: f32) {
        let setting = Setting::now(width, height);
        if self.nav.setting != Some(setting) {
            self.nav.clear();
            self.nav.setting = Some(setting);
        }
        if self.nav.layer.is_none() || !self.nav_tiles_unchanged() {
            let mut layer = Self::nav_open(width, height);
            for (center, half_extent) in self.nav_tile_shapes() {
                layer.block(center, half_extent);
            }
            self.nav.tiles = self.world.query::<&Obstacle>().iter().map(TileKey::of).collect();
            self.nav.layer = Some(layer);
            #[cfg(test)]
            {
                self.nav.built.0 += 1;
            }
        }
        let mut occupancy = self.nav.layer.as_ref().expect("built above").without_fields();
        for (center, half_extent) in self.nav_moving_shapes() {
            occupancy.block(center, half_extent);
        }
        if !self.nav.base.as_ref().is_some_and(|base| base.same_occupancy(&occupancy)) {
            let mut base = self.nav_finish(occupancy);
            base.label();
            self.nav.base = Some(base);
            #[cfg(test)]
            {
                self.nav.built.1 += 1;
            }
        }
    }

    /// Whether the world's tiles are the ones the layer was built from,
    /// in the same order: a tile that died, one that was added and one
    /// that moved all fail it.
    fn nav_tiles_unchanged(&self) -> bool {
        let mut kept = self.nav.tiles.iter();
        let same = self.world.query::<&Obstacle>().iter().all(|o| kept.next().is_some_and(|&k| k == TileKey::of(o)));
        same && kept.next().is_none()
    }

    /// The frame's routing grid, built from scratch: `nav_grid` labelled
    /// and `route_grid_on` it. `Game::update` starts from the grid the
    /// round keeps instead (`refresh_nav`), which is this grid cell for
    /// cell; tools and tests reading a frame's routing ask this one.
    pub(crate) fn route_grid(&self, width: f32, height: f32) -> Grid {
        let mut base = self.nav_grid(width, height);
        base.label();
        self.route_grid_on(base, width, height)
    }

    /// The frame's routing grid on `grid`, `nav_grid` labelled for O(1)
    /// reachability: priced with the tactical surcharges, and carrying
    /// one flow field per target the pack shares - every live player and,
    /// while it lives, the player's frog (a hunter's quarry). Enemies then
    /// route toward those by reading the field, which is worked out only
    /// as far as the frame reads it (`pathfind`), every answer the whole
    /// field's; only a target of their own (an engagement slot, a wander
    /// waypoint, a pickup) still costs a search.
    ///
    /// The lane surcharge walks the cells in front of each live player's
    /// barrel (`Tank::rotation` is the axis a shot flies along) out to
    /// `route_lane_cells`, stopping at the first blocked cell - a shell
    /// flies further, but no tank can stand there anyway. The crowd
    /// surcharge is the cell each live enemy stands in. A field reads the
    /// prices as it is worked out, so every one of them is applied here,
    /// before the first field is added, and nowhere else. No RNG: a
    /// surcharge is a pure function of positions, and the field's ties
    /// break on cell index.
    pub(super) fn route_grid_on(&self, mut grid: Grid, width: f32, height: f32) -> Grid {
        let t = tuning();
        let players: Vec<(Position, f32)> = self
            .seats_on_field()
            .into_iter()
            .flatten()
            .filter_map(|e| with_tank(&self.world, e, |tank| (!tank.is_wreck()).then_some((tank.position, tank.rotation))))
            .collect();
        if t.route_lane_cost > 0 {
            let mut lane = Vec::new();
            for &(pos, rotation) in &players {
                let Some(dir) = Dir::from_rotation(rotation) else {
                    continue;
                };
                let step = dir.vec();
                let mut at = pos;
                for _ in 0..t.route_lane_cells {
                    if grid.blocked_ahead(at, step) {
                        break;
                    }
                    at = Position::new(at.x + step.x * PATHFIND_CELL_SIZE, at.y + step.y * PATHFIND_CELL_SIZE);
                    lane.push(at);
                }
            }
            grid.surcharge(lane.into_iter(), t.route_lane_cost as u32);
        }
        if t.route_crowd_cost > 0 {
            let standing: Vec<Position> =
                self.world.query::<(&Tank, &Ai)>().iter().filter(|(tank, _)| !tank.is_wreck()).map(|(tank, _)| tank.position).collect();
            grid.surcharge(standing.into_iter(), t.route_crowd_cost as u32);
        }
        // The player's towers and the ooze on the ground (docs/defence-
        // towers-prd.md section 9): cells an enemy would rather go round.
        if !self.towers.is_empty() {
            grid.surcharge(self.player_tower_reach(width, height).into_iter(), t.route_tower_cost as u32);
        }
        if !self.ooze.is_empty() {
            grid.surcharge(self.ooze_route_cells().into_iter(), t.bio_puddle_path_cost as u32);
        }
        // The seats' dangers (an armed EMP, docs/emp-burst.md "AI"): cells
        // an enemy would rather go round than cross.
        if t.enemy_danger_route_cost > 0 && self.any_emp() {
            grid.surcharge(self.danger_route_cells(&grid).into_iter(), t.enemy_danger_route_cost as u32);
        }
        // A rod's call (docs/rod-from-god.md "Reacting to a call"): its
        // circle is dear for the countdown, so the fields go round it.
        if !self.zones.is_empty() {
            for (at, cost) in self.zone_route_cells(&grid) {
                grid.surcharge(std::iter::once(at), cost);
            }
        }
        for &(pos, _) in &players {
            grid.add_field(pos);
        }
        if let Some(frog) = self.frog.and_then(|e| with_frog(&self.world, e, |fr| (!fr.is_dead()).then_some(fr.position))) {
            grid.add_field(frog);
        }
        #[cfg(test)]
        if self.whole_fields {
            grid.settle_fields();
        }
        grid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::Intent;
    use crate::map::MapFile;
    use crate::simulation::{Event, Input, PlayerCount};

    const DT: f32 = 1.0 / 60.0;

    /// A fresh round of `map` on `seed` for `seats` seats.
    fn round(map: &str, seed: u64, seats: usize) -> Game {
        let mut game = Game::default();
        game.seed_override = Some(seed);
        game.players = PlayerCount::from_count(seats).expect("a seat count");
        game.map = MapFile::from_toml_str(map).expect("the map parses");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        game
    }

    /// The props playground's seat turned west, firing down row 11 at the
    /// barrel cluster every half second: drums go up, the oil burns, tiles
    /// die and the enemies come at the seat and past the frog.
    fn shooting(frame: u32) -> Input {
        let mut intent = Intent::default();
        if frame <= 2 {
            intent.move_dir = Some(Dir::Left);
        } else {
            intent.fire = frame % 30 == 0;
        }
        Input::single(intent)
    }

    /// Every frame of three rounds - the props playground with its seat
    /// shooting the drums down, the portals fixture and longwater's field
    /// with its lake - the nav grid the round keeps is the grid `nav_grid`
    /// builds from scratch that frame, labelled, cell for cell. The base is
    /// kept on most frames, and on the playground both layers are rebuilt
    /// where the terrain changed: the layer as tiles die, the base on its
    /// own as well, as fires start and burn out.
    #[test]
    fn the_kept_nav_grid_is_the_one_built_from_scratch_every_frame() {
        let runs = [
            ("props", include_str!("../../maps/test/props.toml"), true),
            ("portals", include_str!("../../maps/test/portals.toml"), false),
            ("longwater", include_str!("../../maps/longwater.toml"), false),
        ];
        for (name, map, shoot) in runs {
            let mut game = round(map, 0xB0B5, 1);
            let (w, h) = game.map.field_size();
            let (mut tiles_died, mut most_fires, mut burnt_out) = (0, 0, false);
            const FRAMES: u32 = 900;
            for frame in 1..=FRAMES {
                game.refresh_nav(w, h);
                let mut scratch = game.nav_grid(w, h);
                scratch.label();
                assert!(game.nav.base().same_as(&scratch), "{name}, frame {frame}: the kept grid is not the one built from scratch");
                let fires = game.fires.len();
                game.update(if shoot { shooting(frame) } else { Input::default() }, DT, w, h);
                tiles_died += game.events().iter().filter(|e| matches!(e, Event::ObstacleDestroyed { .. })).count();
                most_fires = most_fires.max(game.fires.len());
                burnt_out |= game.fires.len() < fires;
            }
            let (layers, bases) = game.nav.built;
            assert!(bases < FRAMES / 2, "{name}: the base was kept on most frames ({bases} built in {FRAMES})");
            if shoot {
                assert!(tiles_died > 0 && layers > 1, "{name}: tiles died ({tiles_died}) and the layer was rebuilt ({layers} built)");
                assert!(most_fires > 0 && burnt_out, "{name}: fires started ({most_fires} at most) and burnt out");
                assert!(bases > layers, "{name}: the base was rebuilt on its own ({bases} against {layers} layers)");
            }
        }
    }

    /// A round routed on the kept grid plays exactly as one routed on a
    /// grid built from scratch every frame (`Game::scratch_nav`), tank for
    /// tank and bit for bit: the props playground with its seat shooting
    /// the drums down, and two seats on longwater with its first wave
    /// rolling in.
    #[test]
    fn a_round_routed_on_the_kept_grid_plays_as_one_routed_from_scratch() {
        let runs = [("props", include_str!("../../maps/test/props.toml"), true, 1), ("longwater", include_str!("../../maps/longwater.toml"), false, 2)];
        for (name, map, shoot, seats) in runs {
            let run = |scratch: bool| {
                let mut game = round(map, 0xB0B5, seats);
                game.scratch_nav = scratch;
                let (w, h) = game.map.field_size();
                let mut samples = Vec::new();
                for frame in 1..=900u32 {
                    game.update(if shoot { shooting(frame) } else { Input::default() }, DT, w, h);
                    if frame % 30 == 0 {
                        let tanks: Vec<(usize, u32, u32, u32, u32)> = game
                            .tank_snapshots()
                            .iter()
                            .map(|t| (t.slot, t.position.x.to_bits(), t.position.y.to_bits(), t.rotation.to_bits(), t.damage.to_bits()))
                            .collect();
                        samples.push(tanks);
                    }
                }
                samples
            };
            let (kept, scratch) = (run(false), run(true));
            assert!(kept.iter().any(|tanks| tanks.len() > seats + 1), "{name}: enemies came onto the field");
            for (i, (a, b)) in kept.iter().zip(&scratch).enumerate() {
                assert_eq!(a, b, "{name}, sample {i}: a tank went another way on the kept grid");
            }
        }
    }
}
