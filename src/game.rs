//! The field as a picture, painted over `canvas::Canvas`: the three stages
//! any canvas can run - `paint_floor` (ground, edge shade, tracks,
//! scorches, landed rubble, oil, portals), `paint_tiles` (walls and props
//! with their shadows and caps), `paint_standing` (pickups, the y-sorted
//! tanks/frogs/grass walk, trees) - and `paint_field`, all three. Reads
//! `Game`'s state (owned by `simulation/`) and never mutates it. The GPU
//! runs these stages through `render::canvas::GpuCanvas` inside
//! `Game::render` (`render/game.rs`, the raylib half of this module); the
//! map thumbnail runs them on `canvas::CpuCanvas` with no window at all.

use crate::blast::draw_scorch;
use crate::canvas::Canvas;
use crate::damage_stage::draw_damage;
use crate::decal::draw_decal;
use crate::frog::{draw_frog, draw_frog_ring};
use crate::math::Color;
use crate::obstacle::{draw_obstacle, draw_obstacle_cap, draw_obstacle_shadow, draw_oil_cell, draw_tree, draw_tree_shadow, fence_axis, tree_lean, Material, Obstacle};
use crate::pickup::{draw_pickup, Pickup};
use crate::portal::draw_portal;
use crate::simulation::Game;
use crate::tank::{
    draw_enemy_ring, draw_minigun_mount, draw_minigun_mount_shadow, draw_player_locate, draw_player_ring, draw_tank, draw_tank_shadow,
    draw_tank_shield, Tank,
};
use crate::track::draw_track;
use hecs::Entity;
use std::collections::HashSet;

/// Which ring and readout a tank draws with. The three cases used to be
/// three near-identical loops in `render`; they collapsed into one when the
/// tanks had to be sorted against the grass.
#[derive(Clone, Copy, PartialEq)]
enum TankRole {
    Player,
    /// The second human tank of a two-player round: its own blue ring.
    Player2,
    Enemy,
    /// A wave tank still rolling in: partly off-screen by construction, and
    /// with no health ring or damage overlay until it arrives.
    RollIn,
}

/// One thing standing on the battlefield, for `render`'s back-to-front
/// pass. Trees are deliberately not in here - see that pass.
enum Standing<'a> {
    Tank(&'a Tank, TankRole),
    Frog(Entity),
}

/// A tank and everything drawn on it, in the order the layers stack.
fn draw_one_tank(c: &mut impl Canvas, tank: &Tank, role: TankRole, time: f32, shadows: bool, locate_cue: bool) {
    match role {
        TankRole::Player | TankRole::Player2 => {
            // The locate ripple under the marker so the steady ring stays
            // legible over the swelling one.
            if locate_cue {
                draw_player_locate(c, tank, time, time);
            }
            draw_player_ring(c, tank, time);
        }
        TankRole::Enemy => draw_enemy_ring(c, tank, time),
        TankRole::RollIn => {}
    }
    draw_tank_shield(c, tank, time);
    if shadows {
        draw_tank_shadow(c, tank);
        draw_minigun_mount_shadow(c, tank);
    }
    draw_tank(c, tank);
    draw_minigun_mount(c, tank);
    if role != TankRole::RollIn {
        draw_damage(c, tank, time);
    }
}

/// What `Game::paint_standing` draws besides the field itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaintOptions {
    /// The round-start locate ripple under each player's ring
    /// (`tank::draw_player_locate`). The game draws it; a map thumbnail
    /// (`mapshot`) leaves it out - a two-second cue is not the map. The
    /// cue's other half, the `P1`/`P2` label, is text and stays in `render`.
    pub locate_cue: bool,
}

impl Game {
    /// The floor of the field: the ground tileset and its edge shade (a
    /// flat white sheet under `plain_canvas`), tread marks, burn marks,
    /// landed rubble and unlit oil pools - everything lying flat under
    /// whatever stands. First of the three `Canvas` stages `render` and
    /// `mapshot` share; `paint_field` runs all three.
    pub fn paint_floor(&self, c: &mut impl Canvas) {
        // Ground first - the floor everything else sits on. See
        // ground.rs / docs/GROUND_SPEC.md. `plain_canvas` keeps the
        // white clear instead.
        if !self.plain_canvas {
            let (width, height) = self.map.field_size();
            crate::ground::draw(c, &self.ground, self.map.theme, self.time);
            crate::ground::draw_edge_shade(c, width.round() as i32, height.round() as i32);
        }

        // Tread marks go down first so tanks and everything else draw on top.
        for track in &self.tracks {
            draw_track(c, track);
        }

        // Burn marks under everything that stands, so a barrel that
        // survived a neighbour's blast sits on the mark it left.
        for scorch in &self.scorches {
            draw_scorch(c, scorch);
        }

        // Rubble from tiles that died this round: above the burn marks
        // (a barrel that took a wall with it scorched the ground first)
        // but under everything that still stands, so a wall built over
        // old rubble still reads as solid.
        for decal in self.decals.iter().filter(|dc| dc.landed()) {
            draw_decal(c, decal);
        }
        // Unlit oil trails: puddles on the ground, under everything.
        for &(col, row) in &self.oil_cells {
            draw_oil_cell(c, crate::map::cell_to_world(col, row));
        }
        // Portals last on the floor: over tracks and scorches, under
        // everything that stands. Only an active network draws at all.
        if self.portals_active() {
            for &at in &self.portals {
                draw_portal(c, at, self.time, Color::WHITE);
            }
        }
    }

    /// The tiles: every wall and prop with its shadow and edge cap. Trees
    /// are not tiles here - their canopies belong over the tanks, so they
    /// close `paint_standing` instead.
    pub fn paint_tiles(&self, c: &mut impl Canvas) {
        let fences: HashSet<(i32, i32)> = self
            .world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| o.material == Material::Fence)
            .map(|o| o.cell())
            .collect();
        for obstacle in self.world.query::<&Obstacle>().iter().filter(|o| !o.material.is_tree()) {
            let axis = fence_axis(obstacle, &fences);
            if self.shadows_enabled {
                draw_obstacle_shadow(c, obstacle, axis);
            }
            draw_obstacle(c, obstacle, axis, self.time);
            // Lighting along whichever faces face open ground, so a run
            // of tiles reads as one structure rather than as a grid.
            draw_obstacle_cap(c, obstacle);
        }
    }

    /// Everything standing on the ground: pickups, then tanks, frogs and
    /// grass drawn back to front by where they *meet* the ground, then the
    /// trees over the lot.
    ///
    /// Grass has to be interleaved rather than drawn on top of the lot: a
    /// tuft rooted behind a tank should be hidden by it, and drawing all
    /// grass last is exactly what made a tank look buried in grass that
    /// grows well past it. `Game::grass` is already sorted by root (see
    /// `Game::init`), so this is a merge walk, not a sort of several
    /// hundred sprites.
    ///
    /// Trees are the deliberate exception and still come after all of
    /// this: a crown is *above* tank height, so it overhangs a hull
    /// whichever side of the trunk that hull is on. You drive under a tree
    /// and through grass. Sorted back to front among themselves: their
    /// 48px sprites overlap on a 32px grid, and without an order the
    /// crowns of a grove pop in and out of each other as the query
    /// iterates.
    pub fn paint_standing(&self, c: &mut impl Canvas, opts: PaintOptions) {
        for pickup in self.world.query::<&Pickup>().iter() {
            draw_pickup(c, pickup);
        }

        let player = self.player.expect("player entity spawned in init");
        let rollins: HashSet<Entity> = {
            let mut q = self.world.query::<(Entity, &crate::simulation::RollIn)>();
            let set = q.iter().map(|(e, _)| e).collect();
            set
        };
        let mut tank_query = self.world.query::<(Entity, &Tank)>();
        let mut standing: Vec<(f32, Standing)> = tank_query
            .iter()
            .map(|(entity, tank)| {
                let role = if entity == player {
                    TankRole::Player
                } else if Some(entity) == self.player2 {
                    TankRole::Player2
                } else if rollins.contains(&entity) {
                    TankRole::RollIn
                } else {
                    TankRole::Enemy
                };
                (tank.position.y, Standing::Tank(tank, role))
            })
            .filter(|(_, item)| {
                !(self.hide_players && matches!(item, Standing::Tank(_, TankRole::Player | TankRole::Player2)))
            })
            .collect();
        for frog_entity in [self.frog, self.enemy_frog].into_iter().flatten() {
            let y = crate::simulation::with_frog(&self.world, frog_entity, |frog| frog.position.y);
            standing.push((y, Standing::Frog(frog_entity)));
        }
        standing.sort_by(|a, b| a.0.total_cmp(&b.0));

        let mut next_tuft = 0usize;
        let grass_up_to = |c: &mut _, upto: f32, from: usize| {
            let mut i = from;
            while i < self.grass.len() && self.grass[i].base.y <= upto {
                crate::grass::draw_tuft(c, &self.grass[i], self.map.theme, self.time);
                i += 1;
            }
            i
        };
        for (key, item) in &standing {
            next_tuft = grass_up_to(c, *key, next_tuft);
            match item {
                Standing::Tank(tank, role) => draw_one_tank(c, tank, *role, self.time, self.shadows_enabled, opts.locate_cue),
                Standing::Frog(entity) => {
                    crate::simulation::with_frog(&self.world, *entity, |frog| {
                        draw_frog_ring(c, frog, self.time);
                        draw_frog(c, frog, self.time);
                    });
                }
            }
        }
        grass_up_to(c, f32::INFINITY, next_tuft);

        let movers: Vec<crate::Position> = self
            .world
            .query::<&Tank>()
            .iter()
            .filter(|t| !t.is_dead())
            .map(|t| t.position)
            .collect();
        let mut tree_query = self.world.query::<&Obstacle>();
        let mut trees: Vec<&Obstacle> = tree_query.iter().filter(|o| o.material.is_tree()).collect();
        trees.sort_by(|a, b| a.position.y.total_cmp(&b.position.y));
        for tree in &trees {
            let lean = tree_lean(tree, &movers);
            if self.shadows_enabled {
                draw_tree_shadow(c, tree, lean, self.time);
            }
            draw_tree(c, tree, lean, self.time);
        }
    }

    /// The static field as a fresh round shows it: `paint_floor`,
    /// `paint_tiles`, `paint_standing`, in that order, onto a canvas the
    /// caller has cleared. What `mapshot` renders (docs/mapshot-prd.md);
    /// `render` runs the same three stages with its live-round layers in
    /// between.
    pub fn paint_field(&self, c: &mut impl Canvas, opts: PaintOptions) {
        self.paint_floor(c);
        self.paint_tiles(c);
        self.paint_standing(c, opts);
    }
}
