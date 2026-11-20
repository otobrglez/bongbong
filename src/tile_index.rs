//! The tiles by where they stand, for the painters: `TileIndex` lists a
//! round's `Obstacle`s in the order a walk of the world meets them and
//! files each under the `marks::BLOCK_PX` block it stands in, with every
//! fence's cell beside them, so a painter over part of the field
//! (`Game::with_obstacles_in`) reads the blocks its view crosses and draws
//! what it finds in the walk's order - the order tiles lie over one another
//! in. It is a copy of the world and nothing that plays reads it: a new
//! round starts with none (`Game::init`), a window holds it to the world
//! once a frame (`Game::refresh_pictures`, tile by tile), and every reader
//! first checks that the world still holds the same ids in the same rows
//! (`stands`) and builds it again if not.

use crate::marks::{block_of, blocks_over};
use crate::math::Rectangle;
use crate::obstacle::{Material, Obstacle};
use crate::Position;
use hecs::{Entity, World};
use std::collections::{HashMap, HashSet};

/// One tile as the index filed it.
#[derive(Clone, Copy, PartialEq)]
struct Entry {
    entity: Entity,
    position: Position,
    material: Material,
}

impl Entry {
    fn of(entity: Entity, obstacle: &Obstacle) -> Entry {
        Entry { entity, position: obstacle.position, material: obstacle.material }
    }
}

#[derive(Default)]
pub struct TileIndex {
    /// Every tile, in the order `World::query::<&Obstacle>` yields them.
    entries: Vec<Entry>,
    /// Each block's tiles as indices into `entries`, rising.
    blocks: HashMap<(i32, i32), Vec<u32>>,
    /// The cell of every fence (`obstacle::fence_axis` reads a fence's
    /// neighbours, which may stand outside any view).
    fences: HashSet<(i32, i32)>,
    /// The rows of the world the index was built from (`rows`).
    rows: Vec<u32>,
    /// Which build this is, of every index's: 0 before the first.
    built: u64,
}

/// How many times any index has been built.
static BUILDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Each table of `world` that holds obstacles, as its length then the ids
/// in its rows: what a walk of the obstacles visits, by id.
fn rows(world: &World) -> impl Iterator<Item = u32> + '_ {
    world
        .archetypes()
        .filter(|table| table.has::<Obstacle>() && !table.is_empty())
        .flat_map(|table| std::iter::once(table.len()).chain(table.ids().iter().copied()))
}

impl TileIndex {
    /// Whether `world` holds the entities the index was built from, in the
    /// same rows. A tile moved or changed in place, or one despawned and
    /// another spawned into its row under its id, passes; `matches` is
    /// the test that catches those.
    pub fn stands(&self, world: &World) -> bool {
        let mut kept = self.rows.iter();
        rows(world).all(|id| kept.next() == Some(&id)) && kept.next().is_none()
    }

    /// Whether the index is `world`'s tiles exactly: each entity where it
    /// stands, of the material it is, in the walk's order.
    pub fn matches(&self, world: &World) -> bool {
        let mut kept = self.entries.iter();
        world.query::<(Entity, &Obstacle)>().iter().all(|(entity, obstacle)| kept.next() == Some(&Entry::of(entity, obstacle))) && kept.next().is_none()
    }

    /// Build the index from `world`, in the room it has.
    pub fn rebuild(&mut self, world: &World) {
        self.entries.clear();
        self.fences.clear();
        for numbers in self.blocks.values_mut() {
            numbers.clear();
        }
        for (entity, obstacle) in world.query::<(Entity, &Obstacle)>().iter() {
            let entry = Entry::of(entity, obstacle);
            self.blocks.entry(block_of(entry.position)).or_default().push(self.entries.len() as u32);
            if entry.material == Material::Fence {
                self.fences.insert(obstacle.cell());
            }
            self.entries.push(entry);
        }
        self.blocks.retain(|_, numbers| !numbers.is_empty());
        self.rows.clear();
        self.rows.extend(rows(world));
        self.built = BUILDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    }

    /// Which build this is: a number no other index and no other build of
    /// this one has, so a list gathered from it can be told from any other.
    pub fn built(&self) -> u64 {
        self.built
    }

    /// The cell of every fence standing.
    pub fn fences(&self) -> &HashSet<(i32, i32)> {
        &self.fences
    }

    /// The tiles standing inside `rect` (`Rectangle::contains` on each
    /// one's position, as `view::culled` tests it) into `out`, in the walk's
    /// order.
    pub fn gather(&self, rect: Rectangle, out: &mut Vec<Entity>) {
        out.clear();
        let mut picked: Vec<u32> =
            blocks_over(&self.blocks, rect).flatten().copied().filter(|&i| rect.contains(self.entries[i as usize].position)).collect();
        picked.sort_unstable();
        out.extend(picked.into_iter().map(|i| self.entries[i as usize].entity));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::MapFile;
    use crate::simulation::Game;

    /// A field of a few screens with a tile on every third cell or so,
    /// fences and trees among them, hashed so it is the same on every run.
    fn dense() -> Game {
        let mut map = String::from("version = 1\nsize = [90, 60]\ntanks = 0\ncells.\"2,2\" = { kind = \"start\" }\n");
        for c in 4..88 {
            for r in 4..58 {
                let h = (c as u32).wrapping_mul(73_856_093) ^ (r as u32).wrapping_mul(19_349_663);
                if h % 3 != 0 {
                    continue;
                }
                let kind = [
                    "kind = \"wall\", material = \"brick\"",
                    "kind = \"wall\", material = \"iron\"",
                    "kind = \"wall\", material = \"wood\"",
                    "kind = \"fence\"",
                    "kind = \"tree\"",
                    "kind = \"barrel\"",
                    "kind = \"sandbag\"",
                ][(h / 3 % 7) as usize];
                map.push_str(&format!("cells.\"{c},{r}\" = {{ {kind} }}\n"));
            }
        }
        let mut game = Game::default();
        game.enemy_count_override = Some(0);
        game.seed_override = Some(7);
        game.map = MapFile::from_toml_str(&map).expect("the map parses");
        let (width, height) = game.map.field_size();
        game.init(width, height);
        game
    }

    fn views() -> Vec<Rectangle> {
        vec![
            Rectangle::new(0.0, 0.0, 1280.0, 720.0),
            Rectangle::new(800.0, 480.0, 1280.0, 720.0),
            Rectangle::new(1023.5, 767.25, 640.0, 360.0),
            Rectangle::new(256.0, 256.0, 512.0, 256.0),
            Rectangle::new(-5000.0, -5000.0, 100.0, 100.0),
            Rectangle::new(-1.0e7, -1.0e7, 2.0e7, 2.0e7),
        ]
    }

    /// The tiles `Game::with_obstacles_in` hands a painter, against the
    /// walk of the world it stands in for: the same tiles in the same
    /// order; and `Game::obstacles_in`, the ones of them it is asked for.
    fn assert_as_a_walk(game: &Game, what: &str) {
        let key = |o: &Obstacle| (o.position.x.to_bits(), o.position.y.to_bits(), o.material, o.variant);
        let wood = |o: &Obstacle| o.material == Material::Wood;
        // Twice over: the second pass of a view is handed the first's list.
        for cull in views().into_iter().chain(views()).map(Some).chain([None]) {
            let walked: Vec<_> = game.world.query::<&Obstacle>().iter().filter(|o| !crate::view::culled(cull, o.position)).map(key).collect();
            let shown: Vec<_> = game.with_obstacles_in(cull, |shown| shown.iter().map(|o| key(o)).collect());
            assert_eq!(shown.len(), walked.len(), "{what}: {cull:?}");
            assert!(shown == walked, "{what}: {cull:?}: the same tiles in the walk's order");
            let walked: Vec<_> = game.world.query::<&Obstacle>().iter().filter(|o| wood(o) && !crate::view::culled(cull, o.position)).map(key).collect();
            let held: Vec<_> = game.obstacles_in(cull, wood).iter().map(|o| key(o)).collect();
            assert!(!held.is_empty() || walked.is_empty());
            assert!(held == walked, "{what}: {cull:?}: the wooden ones of them, in the walk's order");
        }
        let fences: HashSet<(i32, i32)> = game.world.query::<&Obstacle>().iter().filter(|o| o.material == Material::Fence).map(|o| o.cell()).collect();
        assert!(!fences.is_empty() && *game.tile_index().fences() == fences, "{what}: every fence's cell");
    }

    #[test]
    fn a_view_draws_the_tiles_a_walk_would_in_its_order() {
        let game = dense();
        assert!(game.world.query::<&Obstacle>().iter().count() > 1000, "a field worth indexing");
        assert_as_a_walk(&game, "a fresh round");
        let first = views()[0];
        let count = |cull| game.with_obstacles_in(cull, |shown| shown.len());
        assert!(count(Some(first)) > 100 && count(Some(first)) < count(None) / 2, "a view is part of the field");
    }

    #[test]
    fn the_index_follows_the_world_as_tiles_go_and_come() {
        let mut game = dense();
        assert_as_a_walk(&game, "before");
        // Tiles despawned out of the middle of their tables: the rows
        // behind them close up, so the walk's order changes.
        let doomed: Vec<Entity> = game.world.query::<(Entity, &Obstacle)>().iter().map(|(e, _)| e).step_by(7).collect();
        for entity in doomed {
            game.world.despawn(entity).expect("it stands");
        }
        assert!(!game.tile_index.borrow().stands(&game.world), "the rows moved");
        assert_as_a_walk(&game, "after the despawns");
        // A tile somewhere else with the rows as they were - as one
        // despawned and another spawned under its id in its row leaves
        // them: only the frame's own check sees it.
        drop(game.tile_index());
        if let Some(tile) = game.world.query::<&mut Obstacle>().iter().last() {
            tile.position = Position::new(tile.position.x + 640.0, tile.position.y);
        }
        assert!(game.tile_index.borrow().stands(&game.world) && !game.tile_index.borrow().matches(&game.world));
        game.refresh_pictures(&[]);
        assert_as_a_walk(&game, "after a tile moved in its row");
    }
}
