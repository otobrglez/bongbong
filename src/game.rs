//! The field as a picture, painted over `canvas::Canvas`: the three stages
//! any canvas can run - `paint_floor` (ground, floor shade, tracks,
//! scorches, landed rubble, oil, portals), `paint_tiles` (walls and props
//! with their shadows and caps), `paint_standing` (pickups, the y-sorted
//! tanks/frogs/grass walk, trees) - and `paint_field`, all three. Reads
//! `Game`'s state (owned by `simulation/`) and never mutates it. The GPU
//! runs these stages through `render::canvas::GpuCanvas` inside
//! `Game::render` (`render/game.rs`, the raylib half of this module); the
//! map thumbnail runs them on `canvas::CpuCanvas` with no window at all.
//! On a canvas over part of the field (`Canvas::cull`) the stages leave
//! out what lies wholly outside it.

use crate::blast::draw_scorch;
use crate::canvas::Canvas;
use crate::decal::draw_decal;
use crate::frog::{draw_frog, draw_frog_ring};
use crate::math::Color;
use crate::obstacle::{draw_obstacle, draw_obstacle_cap, draw_obstacle_shadow, draw_oil_cell, draw_tree, draw_tree_shadow, fence_axis, tree_lean, Material, Obstacle};
use crate::pickup::{draw_pickup, Pickup};
use crate::portal::draw_portal;
use crate::simulation::Game;
use crate::tank::{
    draw_enemy_ring, draw_player_locate, draw_player_ring, draw_tank, draw_tank_heat_shield, draw_tank_shadow, draw_tank_shield, Tank,
};
use crate::track::draw_track;
use crate::view::culled;
use hecs::Entity;
use std::collections::HashSet;

/// Which ring and readout a tank draws with. The three cases used to be
/// three near-identical loops in `render`; they collapsed into one when the
/// tanks had to be sorted against the grass.
#[derive(Clone, Copy, PartialEq)]
enum TankRole {
    /// Any seat's tank: one ring, drawn in that seat's own team colour.
    Player,
    Enemy,
    /// A tank still rolling in - a wave's, or a seat driving back in
    /// (`simulation/waves.rs`): partly off-screen by construction, and
    /// with no health ring or damage overlay until it arrives.
    RollIn,
}

/// One thing standing on the battlefield, for `render`'s back-to-front
/// pass. Trees are deliberately not in here - see that pass.
enum Standing<'a> {
    Tank(&'a Tank, TankRole),
    Frog(Entity),
    Tower(crate::tower::TowerView),
    /// A lamp post: its lantern stands above its foot, so a tank that
    /// drives behind one is drawn behind it.
    Lamp(crate::Position),
}

/// A tank and everything drawn on it, in the order the layers stack.
fn draw_one_tank(c: &mut impl Canvas, tank: &Tank, role: TankRole, time: f32, shadows: bool, locate_cue: bool) {
    match role {
        TankRole::Player => {
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
    draw_tank_heat_shield(c, tank, time);
    if shadows {
        draw_tank_shadow(c, tank, time);
    }
    // The sheet's damage tiers carry the wear; a burning deck or wreck
    // adds its flames over everything on the tank, leaning with the wind
    // (`damage_stage.rs`). Their light is the glowing pass's.
    draw_tank(c, tank, time);
    crate::tank::draw_tank_slime(c, tank, time);
    if role != TankRole::RollIn {
        let lean = crate::pyro::smoke_lean(&crate::tuning::tuning(), tank.position, time);
        crate::pyro::draw(c, &crate::damage_stage::flames(tank, time, lean));
    }
}

/// The flames on a burning tile (`Obstacle::burning`: timber and trees
/// charring out), in the effects language (`pyro::tongues`): catching over
/// its first moments and dying down over the end of `wood_burn_seconds`,
/// leaning with the wind; a tree burns in its crown. The last shape is
/// their light, which the glowing pass draws.
pub fn tile_flames(obstacle: &Obstacle, time: f32) -> Vec<crate::pyro::Shape> {
    let t = crate::tuning::tuning();
    let mut out = Vec::new();
    if !obstacle.burning {
        return out;
    }
    let catching = (obstacle.burn_elapsed / 0.3).clamp(0.0, 1.0);
    let left = t.wood_burn_seconds - obstacle.burn_elapsed;
    let dying = (left / (t.wood_burn_seconds * 0.3).max(0.01)).clamp(0.0, 1.0);
    let strength = catching.min(dying).max(0.35);
    let at = obstacle.position;
    let lean = crate::pyro::smoke_lean(&t, at, time);
    let seed = crate::blast::seed_at(at, 29);
    // Tall enough to rise clear of the tile, over its own glowing char.
    let (foot, spread, height, count) = if obstacle.material.is_tree() {
        (crate::Position::new(at.x, at.y - 4.0), 24.0, 30.0, 3)
    } else {
        (crate::Position::new(at.x, at.y + 6.0), 22.0, 32.0, 3)
    };
    crate::pyro::tongues(&mut out, foot, spread, height, count, seed, time, lean, strength);
    out.push(crate::pyro::Shape::Glow {
        pos: crate::Position::new(foot.x, foot.y - height * 0.4),
        radius: 34.0 * (0.5 + 0.5 * strength),
        color: crate::pyro::alpha(crate::pyro::FIRE[5], 0.22 * strength),
    });
    out
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
    /// Bring the pictures the round keeps between frames - the lava's,
    /// the bombs' pools', the craters' molten rock (`lava::LavaPictures`)
    /// - up to this frame, for `views`, the world rectangles the window
    /// shows (empty: the whole field). A window calls it before it uploads
    /// the frame's textures (`with_block_images`); a painter draws from the
    /// pictures only on the frame they were brought up to.
    pub fn refresh_pictures(&self, views: &[crate::math::Rectangle]) {
        if self.lava.is_empty() && self.volcanoes.is_empty() && !self.fires.iter().any(|f| f.lava) {
            return;
        }
        let t = crate::tuning::tuning();
        let pools: Vec<((i32, i32), f32)> =
            self.fires.iter().filter(|f| f.lava).map(|f| (f.cell, crate::lava::pool_heat(f.left, f.total))).collect();
        let cones: Vec<_> = self.volcanoes.iter().map(|v| (v.cell, crate::volcano::cone(&v.outlets), v.phase(self.time, &t))).collect();
        self.lava.refresh_pictures(&crate::lava::PictureFrame {
            time: self.time,
            look: self.lava_look(),
            views,
            pools: &pools,
            cones: &cones,
            bands: t.glow_bands.max(0) as u32,
        });
    }

    /// Every baked image the round draws from, in an order that holds
    /// while the round does, for the window's GPU copies: the floor shade,
    /// the lava's banks, the kept pictures, then each cone's.
    pub fn with_block_images<R>(&self, f: impl FnOnce(&[&crate::canvas::BlockImage]) -> R) -> R {
        let pictures = self.lava.pictures();
        let cones: Vec<_> = self.volcanoes.iter().map(|v| crate::volcano::cone(&v.outlets)).collect();
        let mut images = vec![self.ground.shade()];
        if !self.lava.is_empty() {
            images.push(self.lava.banks());
        }
        images.extend(pictures.images());
        for cone in &cones {
            let parts = cone.images();
            images.extend([&parts.body, &parts.shadow, &parts.skirt]);
        }
        f(&images)
    }

    /// The floor of the field: the ground tileset and its baked shade (a
    /// flat white sheet under `plain_canvas`), tread marks, burn marks,
    /// landed rubble and unlit oil pools - everything lying flat under
    /// whatever stands. First of the three `Canvas` stages `render` and
    /// `mapshot` share; `paint_field` runs all three. It is `paint_ground`
    /// then `paint_floor_marks`, the seam the weather's ground pass
    /// (`render/weather.rs`) draws its snow and puddles in.
    pub fn paint_floor(&self, c: &mut impl Canvas) {
        self.paint_ground(c);
        self.paint_floor_marks(c);
    }

    /// The ground tileset alone - the floor everything else sits on (see
    /// ground.rs / docs/GROUND_SPEC.md). `plain_canvas` keeps the white
    /// clear instead.
    pub fn paint_ground(&self, c: &mut impl Canvas) {
        if !self.plain_canvas {
            crate::ground::draw(c, &self.ground, self.map.theme, self.time);
        }
    }

    /// Everything lying flat on the ground: its baked shade (the walls'
    /// and the edge's, `ground::bake_shade`), then the marks on it. Each
    /// mark the canvas culls (`Canvas::culls`) is left out.
    pub fn paint_floor_marks(&self, c: &mut impl Canvas) {
        if !self.plain_canvas {
            crate::ground::draw_shade(c, &self.ground);
        }
        // The ground the lava toasts and the cinders round each cone's
        // foot (docs/volcano.md), under every mark.
        if !self.lava.is_empty() {
            c.blocks(self.lava.banks());
        }
        for v in &self.volcanoes {
            if !c.culls(v.centre()) {
                crate::volcano::draw_skirt(c, v.centre(), &crate::volcano::cone(&v.outlets));
            }
        }

        // Tread marks go down first so tanks and everything else draw on top.
        for track in &self.tracks {
            if !c.culls(track.position) {
                draw_track(c, track);
            }
        }

        // Burn marks under everything that stands, so a barrel that
        // survived a neighbour's blast sits on the mark it left.
        for scorch in &self.scorches {
            if !c.culls(scorch.center) {
                draw_scorch(c, scorch);
            }
        }

        // Rubble from tiles that died this round: above the burn marks
        // (a barrel that took a wall with it scorched the ground first)
        // but under everything that still stands, so a wall built over
        // old rubble still reads as solid.
        for decal in self.decals.iter().filter(|dc| dc.landed()) {
            if !c.culls(decal.center) {
                draw_decal(c, decal);
            }
        }
        // The lava over the marks it runs across: block by block from each
        // cell's neighbours, its bands running downstream, its crust
        // floes drifting with them; then the splashes the bombs threw. A
        // window draws the lava and the pools from the pictures it kept
        // for this frame (`refresh_pictures`), a tile a quad.
        let pictures = self.lava.pictures();
        let fresh = pictures.fresh(self.time);
        if !self.lava.is_empty() {
            let look = self.lava_look();
            let kept = fresh && c.has_blocks(pictures.lava.image.stamp);
            if kept {
                pictures.lava.draw(c);
            }
            for (col, row) in self.lava.cells() {
                if !c.culls(crate::map::cell_to_world(col, row)) {
                    if !kept {
                        crate::lava::draw_cell(c, &self.lava, col, row, self.time, look, false, 1.0);
                    }
                    crate::lava::draw_floes(c, &self.lava, col, row, self.time, look);
                }
            }
        }
        if fresh && c.has_blocks(pictures.pools().image.stamp) {
            pictures.pools().draw(c);
        } else {
            let pools: Vec<((i32, i32), f32)> = self
                .fires
                .iter()
                .filter(|f| f.lava && !c.culls(f.position()))
                .map(|f| (f.cell, crate::lava::pool_heat(f.left, f.total)))
                .collect();
            crate::lava::draw_pools(c, &pools, self.time);
        }
        drop(pictures);
        // Unlit oil trails: puddles on the ground, under everything.
        for &(col, row) in &self.oil_cells {
            let at = crate::map::cell_to_world(col, row);
            if !c.culls(at) {
                draw_oil_cell(c, at);
            }
        }
        // Portals last on the floor: over tracks and scorches, under
        // everything that stands. Only an active network draws at all.
        if self.portals_active() {
            for &at in &self.portals {
                if !c.culls(at) {
                    draw_portal(c, at, self.time, Color::WHITE);
                }
            }
        }
        // The lanterns the seats set down, on the ground under the tanks.
        for lantern in &self.lanterns {
            if !c.culls(lantern.position) {
                crate::lamp::draw_lantern(c, lantern, self.time);
            }
        }
        // What dead towers left, then the ooze over it: a burst vat's spill
        // covers its own ruin (docs/defence-towers-prd.md section 12).
        for ruin in &self.tower_ruins {
            crate::tower::draw_ruin(c, ruin);
        }
        crate::tower::draw_ooze(c, &self.ooze, self.time);
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
        // Towers stand in `paint_standing`; their shadows fall here, under
        // the tiles and everything that drives past.
        let towers = self.tower_views();
        if self.shadows_enabled {
            for view in &towers {
                if !c.culls(view.position) {
                    crate::tower::draw_tower_shadow(c, view.kind, view.side, view.position);
                }
            }
        }
        // A volcano's cone is one picture over all of its tiles, its shadow
        // first (docs/volcano.md).
        let t = crate::tuning::tuning();
        let pictures = self.lava.pictures();
        let fresh = pictures.fresh(self.time);
        for v in &self.volcanoes {
            if c.culls(v.centre()) {
                continue;
            }
            let picture = crate::volcano::cone(&v.outlets);
            if self.shadows_enabled {
                crate::volcano::draw_cone_shadow(c, v.centre(), &picture, (t.shadow_dir_x, t.shadow_dir_y));
            }
            let molten = if fresh { pictures.cone(v.cell).map(|(body, _)| body) } else { None };
            crate::volcano::draw_cone_with(c, v.centre(), &picture, molten, &v.phase(self.time, &t), self.time);
        }
        drop(pictures);
        for obstacle in self
            .world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| !o.material.is_tree() && !o.material.is_tower() && !o.material.is_drawn())
        {
            if c.culls(obstacle.position) {
                continue;
            }
            let axis = fence_axis(obstacle, &fences);
            if self.shadows_enabled {
                draw_obstacle_shadow(c, obstacle, axis);
            }
            draw_obstacle(c, obstacle, axis, self.time);
            // Lighting along whichever faces face open ground, so a run
            // of tiles reads as one structure rather than as a grid.
            draw_obstacle_cap(c, obstacle);
        }
        // Burning timber: flames standing on each plank, over the tiles
        // round it so a burning door in a wall is not cut off by the wall
        // beside it.
        for obstacle in self.world.query::<&Obstacle>().iter().filter(|o| o.burning && !o.material.is_tree()) {
            if !c.culls(obstacle.position) {
                crate::pyro::draw(c, &tile_flames(obstacle, self.time));
            }
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
        // What the canvas culls (`Canvas::cull`) stands out of the walk.
        let cull = c.cull();
        for pickup in self.world.query::<&Pickup>().iter() {
            if !culled(cull, pickup.position) {
                draw_pickup(c, pickup, self.time, self.shadows_enabled);
            }
        }

        let rollins: HashSet<Entity> = {
            let mut q = self.world.query::<(Entity, &crate::simulation::RollIn)>();
            let set = q.iter().map(|(e, _)| e).collect();
            set
        };
        let mut tank_query = self.world.query::<(Entity, &Tank)>();
        let mut standing: Vec<(f32, Standing)> = tank_query
            .iter()
            .map(|(entity, tank)| {
                // The lane first: a tank on its way in through a gate is
                // off the field, seat or not, and draws nothing on it.
                let role = if rollins.contains(&entity) {
                    TankRole::RollIn
                } else if self.is_player(entity) {
                    TankRole::Player
                } else {
                    TankRole::Enemy
                };
                (tank.position.y, Standing::Tank(tank, role))
            })
            .filter(|(_, item)| {
                !(self.hide_players && matches!(item, Standing::Tank(_, TankRole::Player)))
            })
            .filter(|(_, item)| !matches!(item, Standing::Tank(tank, _) if culled(cull, tank.position)))
            .collect();
        for frog_entity in [self.frog, self.enemy_frog].into_iter().flatten() {
            let at = crate::simulation::with_frog(&self.world, frog_entity, |frog| frog.position);
            if !culled(cull, at) {
                standing.push((at.y, Standing::Frog(frog_entity)));
            }
        }
        // A tower rises north of its base, so a tank that drives behind
        // one is drawn behind it.
        for view in self.tower_views() {
            if !culled(cull, view.position) {
                standing.push((view.position.y, Standing::Tower(view)));
            }
        }
        for at in self.lamp_posts() {
            if !culled(cull, at) {
                standing.push((at.y, Standing::Lamp(at)));
            }
        }
        standing.sort_by(|a, b| a.0.total_cmp(&b.0));

        let mut next_tuft = 0usize;
        let grass_up_to = |c: &mut _, upto: f32, from: usize| {
            let mut i = from;
            while i < self.grass.len() && self.grass[i].base.y <= upto {
                if !culled(cull, self.grass[i].base) {
                    crate::grass::draw_tuft(c, &self.grass[i], self.map.theme, self.time);
                }
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
                Standing::Tower(view) => {
                    // The tesla's lens lights with its charge; the mortar's
                    // mouth glows as the next glob comes up.
                    let glow = match view.kind {
                        crate::tower::TowerKind::Bio => view.charge * view.charge,
                        _ => view.charge,
                    };
                    crate::tower::draw_tower(c, view.kind, view.side, view.position, view.stage, view.heading, glow);
                }
                Standing::Lamp(at) => {
                    let t = crate::tuning::tuning();
                    crate::lamp::draw_post(c, *at, self.time, (t.shadow_dir_x, t.shadow_dir_y), self.shadows_enabled);
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
        let mut trees: Vec<&Obstacle> = tree_query.iter().filter(|o| o.material.is_tree() && !culled(cull, o.position)).collect();
        trees.sort_by(|a, b| a.position.y.total_cmp(&b.position.y));
        for tree in &trees {
            let lean = tree_lean(tree, &movers);
            if self.shadows_enabled {
                draw_tree_shadow(c, tree, lean, self.time);
            }
            draw_tree(c, tree, lean, self.time);
            if tree.burning {
                crate::pyro::draw(c, &tile_flames(tree, self.time));
            }
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
