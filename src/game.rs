//! The field as a picture, painted over `canvas::Canvas`: the three stages
//! any canvas can run - `paint_floor` (ground, floor shade, tracks,
//! scorches, landed rubble, oil, portals), `paint_tiles` (walls and props
//! with their shadows and caps), `paint_standing` (pickups, the y-sorted
//! tanks/bongs/grass walk, trees) - and `paint_field`, all three. Reads
//! `Game`'s state (owned by `simulation/`) and never mutates it. The GPU
//! runs these stages through `render::canvas::GpuCanvas` inside
//! `Game::render` (`render/game.rs`, the raylib half of this module); the
//! map thumbnail runs them on `canvas::CpuCanvas` with no window at all.
//! On a canvas over part of the field (`Canvas::cull`) the stages leave
//! out what lies wholly outside it.

use crate::blast::draw_scorch_kept;
use crate::canvas::Canvas;
use crate::decal::draw_decal_kept;
use crate::bong::{draw_bong, draw_bong_ring};
use crate::math::Color;
use crate::obstacle::{draw_obstacle, draw_obstacle_cap, draw_obstacle_shadow, draw_oil_cell, draw_oil_print, draw_tree, draw_tree_shadow, fence_axis, tree_lean, Material, Obstacle, TileLook};
use crate::pickup::{draw_pickup, Pickup};
use crate::portal::draw_portal;
use crate::simulation::Game;
use crate::tank::{
    draw_enemy_ring, draw_player_locate, draw_player_ring, draw_tank, draw_tank_heat_shield, draw_tank_shadow, draw_tank_shield, Tank,
};
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
    Bong(Entity),
    Tower(crate::tower::TowerView),
    /// A lamp post: its lantern stands above its foot, so a tank that
    /// drives behind one is drawn behind it.
    Lamp(crate::Position),
    /// A grenade on the ground, in the grass like anything else: only the
    /// tufts in front of it cover it, so its flash still shows in a field.
    /// One in the air is over everything that stands.
    Grenade(&'a crate::grenade::Grenade),
    /// A burning ground cell's flames (its centre, the seconds it has
    /// left and its whole burn), standing on their foot: whatever is
    /// further up the field is behind them, whatever drives through nearer
    /// is in front.
    Fire(crate::Position, f32, f32),
}

/// How far below its cell's centre a ground fire's flames stand, px: the
/// foot they rise from, and their place among what stands.
pub const GROUND_FIRE_FOOT: f32 = 9.0;

/// The tongues of flame standing on the burning ground cell at `at`
/// (`pyro::tongues`), leaning with the wind, rising as the cell catches
/// and sinking over the last of the `left` seconds of its `total` burn.
pub fn ground_fire(out: &mut Vec<crate::pyro::Shape>, at: crate::Position, left: f32, total: f32, time: f32, t: &crate::tuning::Tuning) {
    let dying = (left / 0.6).clamp(0.0, 1.0);
    let catching = ((total - left) / 0.25).clamp(0.0, 1.0);
    let seed = crate::blast::seed_at(at, 23);
    let lean = crate::pyro::smoke_lean(t, at, time);
    let foot = crate::Position::new(at.x, at.y + GROUND_FIRE_FOOT);
    crate::pyro::tongues(out, foot, t.ground_fire_spread_px, t.ground_fire_height_px, t.ground_fire_tongues.max(0) as u32, seed, time, lean, dying.min(catching));
}

/// The colour a hit's silhouette is drawn in over what `what` names, if
/// one of `flashes` is on it: white, in the flash's step of
/// `hit_flash_alpha`.
fn flash_ink(flashes: &[crate::fx::Flash], what: crate::fx::Flashed, t: &crate::tuning::Tuning) -> Option<Color> {
    let k = flashes.iter().filter(|f| f.what() == what).map(|f| f.strength()).fold(0.0, f32::max);
    (k > 0.0).then(|| crate::pyro::alpha(Color::WHITE, k * t.hit_flash_alpha)).filter(|c| c.a > 0)
}

/// The lists the stages fill on every frame, kept between frames so that a
/// frame builds them in the room the last one left: a stage takes a list
/// out (`scratch`), fills it and puts it back (`keep`).
#[derive(Default)]
struct Scratch {
    picked: Vec<Entity>,
    picked_for: Option<(u64, [u32; 4])>,
    shown: Vec<&'static Obstacle>,
    rollins: HashSet<Entity>,
    standing: Vec<(f32, Standing<'static>)>,
    trees: Vec<&'static Obstacle>,
    movers: Vec<crate::Position>,
    seats: Vec<(crate::Position, crate::Position)>,
}

thread_local! {
    static SCRATCH: std::cell::RefCell<Scratch> = std::cell::RefCell::new(Scratch::default());
}

/// Take one of the thread's kept lists out, as it was put back.
fn scratch<T: Default>(list: impl FnOnce(&mut Scratch) -> &mut T) -> T {
    SCRATCH.with(|s| std::mem::take(list(&mut s.borrow_mut())))
}

/// Put a list back for the next frame.
fn keep<T>(list: impl FnOnce(&mut Scratch) -> &mut T, value: T) {
    SCRATCH.with(|s| *list(&mut s.borrow_mut()) = value);
}

/// `list` emptied, as a list of another type of the same layout - one
/// holding borrows of another frame's - in the room it already has.
fn emptied<T, U>(mut list: Vec<T>) -> Vec<U> {
    list.clear();
    list.into_iter().map(|_| unreachable!("the list is empty")).collect()
}

/// What a wreck's aftermath puts on its hull (docs/aftermath.md): its
/// remain while it is one (`Game::wreck_hot`) and how much snow lies on a
/// cold deck (`Game::hulk_snow`).
#[derive(Clone, Copy, Default)]
struct HulkLook {
    hot: Option<crate::smoulder::Hot>,
    snow: f32,
}

/// A tank and everything drawn on it, in the order the layers stack.
fn draw_one_tank(c: &mut impl Canvas, tank: &Tank, role: TankRole, time: f32, shadows: bool, locate_cue: bool, hulk: HulkLook, t: &crate::tuning::Tuning) {
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
    // A cold hulk's rust and the snow on its deck, under anything still
    // burning on it (docs/aftermath.md).
    if hulk.hot.is_some() {
        crate::pyro::draw(c, &crate::damage_stage::weathering(tank, hulk.snow, t));
    }
    // An EMP's lights out: the lamps and strips drawn dark (docs/emp-burst.md).
    crate::tank::draw_tank_dark(c, tank, time);
    crate::tank::draw_tank_slime(c, tank, time);
    if role != TankRole::RollIn {
        let lean = crate::pyro::smoke_lean(t, tank.drawn().position, time);
        crate::pyro::draw(c, &crate::damage_stage::flames(tank, time, lean));
        // A smouldering hulk's glowing breaches and flare-ups; their light
        // is the glowing pass's.
        if let Some(hot) = hulk.hot.as_ref() {
            crate::pyro::draw(c, &crate::damage_stage::smoulder(tank, hot, time, t));
        }
    }
    // The FPV halo round the hull, over it: its shadows and drones; the
    // lamps are the glowing pass's (`halo_of`).
    if let Some(look) = halo_of(tank, time) {
        let (halo_shadows, pic) = crate::fpv::compose_halo(&look, t);
        if shadows {
            crate::pyro::draw(c, &halo_shadows);
        }
        crate::pyro::draw(c, &pic.body);
    }
}

/// Seconds an EMP takes to drop a halo to the ground (`halo_of`).
const HALO_SETTLE_SECONDS: f32 = 0.25;

/// How `tank`'s FPV halo is drawn at `time` (docs/fpv-swarm.md "Drawing"):
/// the drones it holds, less the ones a client is drawing off it on its
/// own press (`Tank::fpv_lifting`); settled on the ground while an EMP has
/// it, tumbling down and dark on a wreck. `None` with none to draw.
pub fn halo_of(tank: &Tank, time: f32) -> Option<crate::fpv::HaloLook> {
    let t = crate::tuning::tuning();
    let drones = (tank.fpv_drones - tank.fpv_lifting as i32).max(0) as usize;
    // A wreck's halo lies beside it until its fire is out.
    if drones == 0 || (tank.is_wreck() && tank.wreck_timer >= t.wreck_burn_seconds) {
        return None;
    }
    // An EMP drops the halo over `HALO_SETTLE_SECONDS` from its start.
    let settle = if tank.is_disabled() { ((t.emp_disable_seconds - tank.disabled) / HALO_SETTLE_SECONDS).clamp(0.0, 1.0) } else { 0.0 };
    Some(crate::fpv::HaloLook {
        centre: tank.drawn().position,
        sprite_size: tank.sprite_size(),
        drones,
        halo: crate::fpv::Halo::of(&t),
        time,
        lamp: crate::fpv::lamp_color(tank.owner()),
        settle,
        wreck_age: tank.is_wreck().then_some(tank.wreck_timer),
        seed: tank.owner_slot() as u32,
    })
}

/// The FPV drones' shadows on the ground under them (docs/fpv-swarm.md
/// "Drawing"), drawn after the floor and under the tanks. `drones` in id
/// order.
pub fn paint_drone_shadows(c: &mut impl Canvas, drones: &[crate::fpv::Drone]) {
    let t = crate::tuning::tuning();
    let dir = crate::math::Vec2::new(t.shadow_dir_x, t.shadow_dir_y);
    for d in drones {
        crate::pyro::draw(c, &crate::fpv::compose_shadow(d.ground, d.height, t.fpv_shadow_opacity, dir));
    }
}

/// The FPV drones in the air (docs/fpv-swarm.md "Drawing"), over
/// everything standing - trees too: each drone's body; its lamp is the
/// glowing pass's (`drone_lamps`). `drones` in id order.
pub fn paint_drones(c: &mut impl Canvas, drones: &[crate::fpv::Drone], time: f32) {
    let t = crate::tuning::tuning();
    for d in drones {
        crate::pyro::draw(c, &crate::fpv::compose_drone(&crate::fpv::look_of(d, time, &t)).body);
    }
}

/// The lamps of every FPV drone - in the air and in every halo - as
/// blocks for the glowing pass, so they read at night.
pub fn drone_lamps(drones: &[crate::fpv::Drone], halos: &[crate::fpv::HaloLook], time: f32) -> Vec<crate::pyro::Shape> {
    let t = crate::tuning::tuning();
    let mut out = Vec::new();
    for d in drones {
        out.extend(crate::fpv::compose_drone(&crate::fpv::look_of(d, time, &t)).lamps);
    }
    for look in halos {
        out.extend(crate::fpv::compose_halo(look, &t).1.lamps);
    }
    out
}

/// The flames on a burning tile (`Obstacle::burning`: timber and trees
/// charring out), in the effects language (`pyro::tongues`): catching over
/// its first moments and dying down over the end of its burn
/// (`Material::burn_seconds`, on the clock it is drawn by, `burn_shown`),
/// leaning with the wind; a tree burns in its crown. The last shape is
/// their light, which the glowing pass draws.
pub fn tile_flames(obstacle: &Obstacle, time: f32) -> Vec<crate::pyro::Shape> {
    let t = crate::tuning::tuning();
    let mut out = Vec::new();
    if !obstacle.burning {
        return out;
    }
    let burn = obstacle.burn_seconds();
    let catching = (obstacle.burn_shown / 0.3).clamp(0.0, 1.0);
    let left = burn - obstacle.burn_shown;
    let dying = (left / (burn * 0.3).max(0.01)).clamp(0.0, 1.0);
    let strength = catching.min(dying).max(0.35);
    let at = obstacle.position;
    let lean = crate::pyro::smoke_lean(&t, at, time);
    let seed = crate::blast::seed_at(at, 29);
    // Tall enough to rise clear of the tile, over its own glowing char.
    // A plank wall burns where its fire has got to (`Obstacle::chunk_alight`):
    // the tongues stand over those chunks and widen as the front walks.
    let alight: Vec<crate::Position> = match obstacle.chunks {
        Some(c) if obstacle.material == crate::obstacle::Material::Wood => (0..crate::chunks::CHUNKS)
            .filter(|&i| c.stands(i) && obstacle.chunk_alight(i))
            .map(|i| crate::chunks::chunk_center(at, i))
            .collect(),
        _ => Vec::new(),
    };
    let (foot, spread, height, count) = if obstacle.material.is_tree() {
        (crate::Position::new(at.x, at.y - 4.0), 24.0, 30.0, 3)
    } else if !alight.is_empty() {
        let n = alight.len() as f32;
        let mid = crate::Position::new(alight.iter().map(|p| p.x).sum::<f32>() / n, alight.iter().map(|p| p.y).sum::<f32>() / n);
        let wide = alight.iter().map(|p| (p.x - mid.x).abs()).fold(0.0f32, f32::max);
        (crate::Position::new(mid.x, mid.y + 4.0), (8.0 + wide * 2.0).min(26.0), 20.0 + 1.5 * n, if n > 6.0 { 3 } else { 2 })
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

/// Whether `obstacle` is drawn as a tile (`paint_tile_run`): a wall or a
/// prop - not a tree, whose crown stands over the tanks, nor a tower, a
/// lamp, a volcano or a door, which their own painters draw.
pub fn is_tile(obstacle: &Obstacle) -> bool {
    !obstacle.material.is_tree() && !obstacle.material.is_tower() && !obstacle.material.is_drawn()
}

/// Every tile of `tiles` (`is_tile`), in the order given, as a round draws
/// one: its drop shadow while `shadows`, the tile at its variant, then
/// the lighting along whichever of its faces face open ground, so a run
/// of tiles reads as one structure rather than as a grid. `fences` is
/// every fence's cell, which way each runs (`fence_axis`); `look` is how
/// the biome dresses them (`TileLook`). The round's tiles
/// (`Game::paint_tiles`) and the builder's canvas (`editor::still`) are
/// drawn by this one loop, under the knobs `t`. Each tile the canvas culls
/// is left out.
pub fn paint_tile_run<'a>(
    c: &mut impl Canvas,
    tiles: impl Iterator<Item = &'a Obstacle>,
    fences: &HashSet<(i32, i32)>,
    shadows: bool,
    time: f32,
    t: &crate::tuning::Tuning,
    look: TileLook,
) {
    for obstacle in tiles {
        if c.culls(obstacle.position) {
            continue;
        }
        let axis = fence_axis(obstacle, fences);
        if shadows {
            draw_obstacle_shadow(c, obstacle, axis, t, look);
        }
        draw_obstacle(c, obstacle, axis, time, look);
        draw_obstacle_cap(c, obstacle, look);
    }
}

/// `trees` back to front by where each crown is drawn (`woods.rs` moves it
/// off its cell), cell order breaking ties: their 48 px sprites overlap on
/// a 32 px grid, and without an order the crowns of a grove pop in and out
/// of each other.
pub fn sort_trees<T: std::borrow::Borrow<Obstacle>>(trees: &mut [T]) {
    trees.sort_by(|a, b| {
        let (a, b) = (a.borrow(), b.borrow());
        (a.position.y + a.crown.dy).total_cmp(&(b.position.y + b.crown.dy)).then(a.position.x.total_cmp(&b.position.x))
    });
}

/// One tree as a round stands it: its shadow while `shadows`, its crown
/// leaning `lean` and drawn `alpha` opaque (a canopy cut away over a
/// player's hull is see-through), and its flames while it burns. The
/// round's woods (`Game::paint_standing`) and the builder's
/// (`editor::still`) are drawn by this.
pub fn paint_tree(c: &mut impl Canvas, tree: &Obstacle, lean: f32, alpha: f32, shadows: bool, time: f32, t: &crate::tuning::Tuning) {
    if shadows {
        draw_tree_shadow(c, tree, lean, time, alpha, t);
    }
    if alpha < 1.0 {
        let alpha = (255.0 * alpha.clamp(0.0, 1.0)) as u8;
        crate::obstacle::draw_tree_tinted(c, tree, lean, time, crate::math::Color::new(255, 255, 255, alpha), t);
    } else {
        draw_tree(c, tree, lean, time, t);
    }
    if tree.burning {
        crate::pyro::draw(c, &tile_flames(tree, time));
    }
}

/// What the aftermath puts in the air over the field this frame
/// (`Game::aftermath_air`, docs/aftermath.md).
#[derive(Default)]
pub struct AftermathAir {
    /// The columns' and the banks' shadows, for the floor.
    pub shadows: Vec<crate::pyro::Shape>,
    /// The columns, the haze's banks and the ash, back to front.
    pub smoke: Vec<crate::pyro::Shape>,
    /// What the haze grades the field toward (`smoulder::haze_grade`), over
    /// this rectangle of it.
    pub grade: Option<(crate::math::Rectangle, Color)>,
}

impl AftermathAir {
    /// The air laid down over everything that stands: the smoke, then the
    /// haze's grade over all of it.
    pub fn paint_over(&self, c: &mut impl Canvas) {
        crate::pyro::draw(c, &self.smoke);
        if let Some((r, color)) = self.grade {
            crate::pyro::Blocks::fill_rect(c, r.x.floor() as i32, r.y.floor() as i32, r.width.ceil() as i32 + 2, r.height.ceil() as i32 + 2, color);
        }
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
    /// Bring the pictures the round keeps between frames - the ground's
    /// worn cells (`wear::Pictures`), the lava's, the bombs' pools', the
    /// craters' molten rock (`lava::LavaPictures`) - up to this frame, for
    /// `views`, the world rectangles the window shows (empty: the whole
    /// field). A window calls it before it uploads the frame's textures
    /// (`with_block_images`); a painter draws from the pictures only on the
    /// frame they were brought up to.
    pub fn refresh_pictures(&self, views: &[crate::math::Rectangle]) {
        let tiles_as_they_stand = self.tile_index.borrow().matches(&self.world);
        if !tiles_as_they_stand {
            self.tile_index.borrow_mut().rebuild(&self.world);
        }
        let surface = |p: crate::Position| self.surface_at(p);
        // Under a biome the marks take the colours of its materials.
        let material = |bx: i32, by: i32| self.ground.floor_material(bx, by);
        let floor = self.ground.biome().map(|biome| crate::wear::BiomeFloor { biome, at: &material });
        self.wear.refresh_pictures(
            &crate::wear::PictureFrame { time: self.time, look: self.wear_look(), views, surface: &surface, floor },
            &crate::tuning::tuning(),
        );
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

    /// The tiles by where they stand (`tile_index.rs`), built again here
    /// if the world's rows are not the ones it was built from.
    pub(crate) fn tile_index(&self) -> std::cell::Ref<'_, crate::tile_index::TileIndex> {
        let stands = self.tile_index.borrow().stands(&self.world);
        if !stands {
            self.tile_index.borrow_mut().rebuild(&self.world);
        }
        self.tile_index.borrow()
    }

    /// The entities of the obstacles standing inside `rect`, in the order
    /// a walk of the world meets them (`TileIndex::gather`), with the key
    /// they were gathered under: once for a rectangle and a build of the
    /// index, and handed again to the frame's other painters of the same
    /// view. Give both back with `keep_picked`; while they are out another
    /// painter gathers its own.
    fn picked_in(&self, rect: crate::math::Rectangle) -> (Vec<Entity>, (u64, [u32; 4])) {
        let index = self.tile_index();
        let key = (index.built(), [rect.x.to_bits(), rect.y.to_bits(), rect.width.to_bits(), rect.height.to_bits()]);
        let mut picked = scratch(|s| &mut s.picked);
        if scratch(|s| &mut s.picked_for) != Some(key) {
            index.gather(rect, &mut picked);
        }
        (picked, key)
    }

    /// Put `picked_in`'s list back for the next painter.
    fn keep_picked(&self, picked: Vec<Entity>, key: (u64, [u32; 4])) {
        keep(|s| &mut s.picked, picked);
        keep(|s| &mut s.picked_for, Some(key));
    }

    /// Hand `paint` the obstacles a painter culling to `cull` draws
    /// (`view::culled` on each one's position), in the order a walk of the
    /// world meets them - the order tiles lie over one another in. With no
    /// rectangle that is the walk; with one they are found through the
    /// tile index, so a view of a large field reads what stands in it
    /// rather than the field.
    pub fn with_obstacles_in<R>(&self, cull: Option<crate::math::Rectangle>, paint: impl FnOnce(&[&Obstacle]) -> R) -> R {
        let mut query = self.world.query::<&Obstacle>();
        let mut shown: Vec<&Obstacle> = emptied(scratch(|s| &mut s.shown));
        let view;
        match cull {
            None => shown.extend(query.iter()),
            Some(rect) => {
                let (picked, key) = self.picked_in(rect);
                view = query.view();
                shown.extend(picked.iter().filter_map(|&entity| view.get(entity)));
                self.keep_picked(picked, key);
            }
        }
        let out = paint(&shown);
        keep(|s| &mut s.shown, emptied(shown));
        out
    }

    /// The few of those obstacles `keep` answers true for, each held
    /// (`hecs::Ref`) for a painter that cannot draw inside
    /// `with_obstacles_in`.
    pub fn obstacles_in(&self, cull: Option<crate::math::Rectangle>, keep: impl Fn(&Obstacle) -> bool) -> Vec<hecs::Ref<'_, Obstacle>> {
        let held = |entity: Entity| self.world.get::<&Obstacle>(entity).ok();
        let mut query = self.world.query::<(Entity, &Obstacle)>();
        let Some(rect) = cull else {
            return query.iter().filter(|(_, o)| keep(o)).filter_map(|(entity, _)| held(entity)).collect();
        };
        let (picked, key) = self.picked_in(rect);
        let view = query.view();
        let out = picked.iter().filter(|&&entity| view.get(entity).is_some_and(|(_, o)| keep(o))).filter_map(|&entity| held(entity)).collect();
        self.keep_picked(picked, key);
        out
    }

    /// Every baked image the round draws from, in an order that holds
    /// while the round does, for the window's GPU copies: the floor shade,
    /// the ground's worn cells, the biome's materials, the forest floor,
    /// the lava's banks, the kept pictures, then each cone's.
    pub fn with_block_images<R>(&self, f: impl FnOnce(&[&crate::canvas::BlockImage]) -> R) -> R {
        let pictures = self.lava.pictures();
        let wear = self.wear.pictures();
        let cones: Vec<_> = self.volcanoes.iter().map(|v| crate::volcano::cone(&v.outlets)).collect();
        let mut images = vec![self.ground.shade(), &wear.atlas().image];
        if !self.plain_canvas {
            images.extend(self.ground.materials());
        }
        if !self.woods.is_empty() {
            images.push(self.woods.floor());
        }
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
        let t = crate::tuning::tuning();
        if !self.plain_canvas {
            crate::ground::draw_shade(c, &self.ground);
        }
        // The forest floor under the woods (`woods.rs`).
        if !self.woods.is_empty() {
            c.blocks(self.woods.floor());
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

        // The ground's tread marks (docs/ground-memory.md), baked a cell a
        // tile for the cells in view (`refresh_pictures`): first, so every
        // burn mark, crater and piece of rubble lies over them.
        self.wear.pictures().atlas().draw(c);

        // Burn marks under everything that stands, so a barrel that
        // survived a neighbour's blast sits on the mark it left.
        for scorch in self.scorches.shown(c.cull()) {
            if !c.culls(scorch.center) {
                // Under snow a cold mark goes under it (docs/aftermath.md).
                let cold = if scorch.ember { self.cold_for(crate::smoulder::Kind::Bed, scorch.age, &t) } else { scorch.age };
                draw_scorch_kept(c, scorch, self.snow_keep(cold, &t));
            }
        }

        // The rods' craters (docs/rod-from-god.md): over the burn mark the
        // impact left, under the rubble it threw; filled with water under a
        // wet sky, iced under snow.
        if !self.craters.is_empty() {
            let rain = crate::weather::fills_craters(self.weather, &t);
            for crater in self.craters.list() {
                let at = crate::map::cell_to_world(crater.cell.0, crater.cell.1);
                if c.culls(at) {
                    continue;
                }
                let water = match self.water.depth_of_cell(crater.cell.0, crater.cell.1) {
                    crate::ground::Depth::Shallow | crate::ground::Depth::Deep => crate::rod::CraterWater::Filled,
                    crate::ground::Depth::Ice => crate::rod::CraterWater::Frozen,
                    crate::ground::Depth::Dry => crate::rod::CraterWater::Dry,
                };
                crate::rod::draw_crater(c, crater, self.time - crater.at, water, if rain { 1.0 } else { 0.0 }, self.time);
            }
        }

        // Rubble from tiles that died this round: above the burn marks
        // (a barrel that took a wall with it scorched the ground first)
        // but under everything that still stands, so a wall built over
        // old rubble still reads as solid.
        for decal in self.decals.shown(c.cull()).filter(|dc| dc.landed()) {
            if !c.culls(decal.center) {
                let kind = if decal.upright { crate::smoulder::Kind::Stump } else { crate::smoulder::Kind::Ruin };
                let cold = if decal.hot { self.cold_for(kind, decal.age, &t) } else { decal.age };
                draw_decal_kept(c, decal, self.snow_keep(cold, &t), self.tile_look());
            }
        }
        // What the fight left smouldering on the ground, and the snow over
        // what has gone cold (docs/aftermath.md).
        self.paint_aftermath_floor(c);
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
        // Unlit oil trails: puddles on the ground, under everything; a
        // cell a hull printed is drawn as its print.
        for &(col, row) in &self.oil_cells {
            let at = crate::map::cell_to_world(col, row);
            if !c.culls(at) {
                match self.oil_prints.get(&(col, row)) {
                    Some(print) => draw_oil_print(c, at, print.dir, print.fade),
                    None => draw_oil_cell(c, at),
                }
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
        // Last on the floor, under everything that stands in it: the reach
        // of every blast a seat owns that is about to go off. Never culled
        // by its centre: the ring reaches far past it.
        for ring in self.blast_rings() {
            crate::blast::draw_blast_ring(c, &ring, &t);
        }
    }

    /// How the round's biome dresses its walls, fences and their rubble
    /// (`TileLook`): read off the ground it is drawn over, never by
    /// anything that plays.
    pub fn tile_look(&self) -> TileLook {
        TileLook::of(self.ground.biome())
    }

    /// The tiles: every wall and prop with its shadow and edge cap. Trees
    /// are not tiles here - their canopies belong over the tanks, so they
    /// close `paint_standing` instead.
    pub fn paint_tiles(&self, c: &mut impl Canvas) {
        // Every fence's cell: a fence joins its neighbours, in view or not.
        let index = self.tile_index();
        let fences = index.fences();
        let cull = c.cull();
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
        // What stands in the canvas's part of the field, in the world's
        // order.
        self.with_obstacles_in(cull, |shown| {
            paint_tile_run(c, shown.iter().copied().filter(|o| is_tile(o)), fences, self.shadows_enabled, self.time, &t, self.tile_look());
            // Training doors and flags (docs/training-stage.md): drawn by
            // their own code, doors with the tiles, flags standing on the
            // ground beside them.
            for obstacle in shown.iter().filter(|o| o.material == crate::obstacle::Material::Door) {
                if !c.culls(obstacle.position) {
                    crate::training::draw_door(c, obstacle.position);
                }
            }
            for flag in self.training_flags() {
                if !c.culls(flag.at) {
                    crate::training::draw_flag(c, flag.at, crate::tank::team_color(0), flag.taken, self.time);
                }
            }
            // A range board's own fire (`target.rs`): the soot creeping in
            // under the stream, the embers pulsing over its char once it
            // burns - under the flames that stand on it.
            let ignite = t.flame_ignite_seconds;
            for obstacle in shown.iter().filter(|o| o.material == Material::Target) {
                if !c.culls(obstacle.position) {
                    crate::pyro::draw(c, &crate::target::fire_shapes(obstacle, ignite));
                }
            }
            // Burning timber: flames standing on each plank, over the tiles
            // round it so a burning door in a wall is not cut off by the wall
            // beside it.
            for obstacle in shown.iter().filter(|o| o.burning && !o.material.is_tree()) {
                if !c.culls(obstacle.position) {
                    crate::pyro::draw(c, &tile_flames(obstacle, self.time));
                }
            }
        });
    }

    /// Everything standing on the ground: pickups, then tanks, bongs and
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
        self.paint_standing_hit(c, opts, &[]);
    }

    /// `paint_standing` with the hits of the moment on it (`fx::Flash`,
    /// which the particle layer keeps): a hull or a bong a shot just
    /// landed on is covered by its own silhouette in white, straight
    /// after it is drawn - at its place in the walk, so under the grass in
    /// front of it and under every crown, and as bright on a dark chassis
    /// as on a pale one.
    pub fn paint_standing_hit(&self, c: &mut impl Canvas, opts: PaintOptions, flashes: &[crate::fx::Flash]) {
        // What the canvas culls (`Canvas::cull`) stands out of the walk.
        let cull = c.cull();
        let t = crate::tuning::tuning();
        for pickup in self.world.query::<&Pickup>().iter() {
            if !culled(cull, pickup.at()) {
                draw_pickup(c, pickup, self.time, self.shadows_enabled, &t);
            }
        }

        let mut rollins = scratch(|s| &mut s.rollins);
        rollins.clear();
        rollins.extend(self.world.query::<(Entity, &crate::simulation::RollIn)>().iter().map(|(e, _)| e));
        let mut tank_query = self.world.query::<(Entity, &Tank)>();
        let tanks = tank_query
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
            .filter(|(_, item)| !matches!(item, Standing::Tank(tank, _) if culled(cull, tank.position)));
        let mut standing: Vec<(f32, Standing)> = emptied(scratch(|s| &mut s.standing));
        standing.extend(tanks);
        for bong_entity in [self.bong, self.enemy_bong].into_iter().flatten() {
            let at = crate::simulation::with_bong(&self.world, bong_entity, |bong| bong.position);
            if !culled(cull, at) {
                standing.push((at.y, Standing::Bong(bong_entity)));
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
        let mut grenade_query = self.world.query::<&crate::grenade::Grenade>();
        for grenade in grenade_query.iter() {
            if !culled(cull, grenade.position) {
                let key = if grenade.airborne() { f32::MAX } else { grenade.position.y };
                standing.push((key, Standing::Grenade(grenade)));
            }
        }
        // Burning ground: each cell's flames at their foot. A bomb's splash
        // of lava is molten rock on the floor (`lava::draw_pools`), not
        // flames.
        for fire in self.fires.iter().filter(|f| f.left > 0.0 && !f.lava) {
            let at = fire.position();
            if !culled(cull, at) {
                standing.push((at.y + GROUND_FIRE_FOOT, Standing::Fire(at, fire.left, fire.total)));
            }
        }
        // Stable, so things on one line keep the order they were listed in.
        standing.sort_by(|a, b| a.0.total_cmp(&b.0));

        let mut flames = Vec::new();
        // The tufts are sorted by root, so the ones a canvas over part of
        // the field can show are one run of them: from the first rooted at
        // its top edge to the last above its bottom one.
        let (mut next_tuft, last_tuft) = match cull {
            Some(r) => (self.grass.partition_point(|g| g.base.y < r.y), self.grass.partition_point(|g| g.base.y < r.y + r.height)),
            None => (0, self.grass.len()),
        };
        let grass_up_to = |c: &mut _, upto: f32, from: usize| {
            let mut i = from;
            while i < last_tuft && self.grass[i].base.y <= upto {
                if !culled(cull, self.grass[i].base) {
                    crate::grass::draw_tuft(c, &self.grass[i], self.map.theme, self.time, &t);
                }
                i += 1;
            }
            i
        };
        for (key, item) in &standing {
            next_tuft = grass_up_to(c, *key, next_tuft);
            match item {
                Standing::Tank(tank, role) => {
                    let hulk = HulkLook { hot: self.wreck_hot(tank), snow: self.hulk_snow(tank, &t) };
                    draw_one_tank(c, tank, *role, self.time, self.shadows_enabled, opts.locate_cue, hulk, &t);
                    if *role != TankRole::RollIn
                        && let Some(ink) = flash_ink(flashes, crate::fx::Flashed::Tank(tank.owner_slot()), &t)
                    {
                        crate::tank::draw_tank_silhouette(c, tank, self.time, ink);
                    }
                }
                Standing::Bong(entity) => {
                    crate::simulation::with_bong(&self.world, *entity, |bong| {
                        draw_bong_ring(c, bong, self.time);
                        draw_bong(c, bong, self.time);
                        if let Some(ink) = flash_ink(flashes, crate::fx::Flashed::Bong(bong.side), &t) {
                            crate::bong::draw_bong_silhouette(c, bong, self.time, ink);
                        }
                    });
                }
                Standing::Tower(view) => {
                    // The tesla's lens lights with its charge; the mortar's
                    // mouth glows as the next glob comes up.
                    // Offline (an EMP), both are dark and the top sags.
                    let glow = match view.kind {
                        _ if view.disabled > 0.0 => 0.0,
                        crate::tower::TowerKind::Bio => view.charge * view.charge,
                        _ => view.charge,
                    };
                    crate::tower::draw_tower(c, view.kind, view.side, view.position, view.stage, view.heading + view.droop, glow);
                }
                Standing::Lamp(at) => {
                    let lit = crate::emp::lamp_lit(*at, self.lamps_out);
                    crate::lamp::draw_post(c, *at, self.time, (t.shadow_dir_x, t.shadow_dir_y), self.shadows_enabled, lit);
                }
                Standing::Grenade(grenade) => crate::grenade::draw_grenade(c, grenade),
                Standing::Fire(at, left, total) => {
                    flames.clear();
                    ground_fire(&mut flames, *at, *left, *total, self.time, &t);
                    crate::pyro::draw(c, &flames);
                }
            }
        }
        grass_up_to(c, f32::INFINITY, next_tuft);
        keep(|s| &mut s.standing, emptied(standing));
        keep(|s| &mut s.rollins, rollins);

        let mut movers = scratch(|s| &mut s.movers);
        movers.clear();
        movers.extend(self.world.query::<&Tank>().iter().filter(|t| !t.is_dead()).map(|t| t.position));
        self.with_obstacles_in(cull, |shown| {
            let mut trees: Vec<&Obstacle> = emptied(scratch(|s| &mut s.trees));
            trees.extend(shown.iter().copied().filter(|o| o.material.is_tree() && !culled(cull, o.position)));
            sort_trees(&mut trees);
            // The players' hulls, for the canopy cutaway: the crowns hiding
            // much of one are drawn see-through, so a player never loses their
            // own tank under a wood. An enemy under a canopy stays hidden.
            let mut seats = scratch(|s| &mut s.seats);
            seats.clear();
            seats.extend(
                self.world
                    .query::<(Entity, &Tank)>()
                    .iter()
                    .filter(|&(e, tank)| !tank.is_dead() && !self.hide_players && self.is_player(e))
                    .map(|(_, tank)| tank.hull_bbox_world()),
            );
            let cutaway = crate::obstacle::canopy_cutaway(&trees, &seats, t.canopy_cutaway_cover);
            for (tree, &see_through) in trees.iter().zip(&cutaway) {
                // A sonic wave passing sways the crown away from its pivot;
                // a hull's hit on the trunk shakes it.
                let lean = tree_lean(tree, &movers, &t) + crate::sonic::tree_push(tree.position, &self.sonic_waves, &t) + self.tree_shake_lean(tree.cell(), &t);
                let alpha = if see_through { t.canopy_cutaway_alpha } else { 1.0 };
                paint_tree(c, tree, lean, alpha, self.shadows_enabled, self.time, &t);
            }
            keep(|s| &mut s.trees, emptied(trees));
            keep(|s| &mut s.seats, seats);
        });
        keep(|s| &mut s.movers, movers);
    }

    /// What of a mark that went cold `cold` seconds ago still shows through
    /// the snow (`smoulder::snow_cover`): all of it under any other sky, and
    /// all of it while it is still hot.
    fn snow_keep(&self, cold: f32, t: &crate::tuning::Tuning) -> f32 {
        if self.weather != crate::map::Weather::Snow {
            return 1.0;
        }
        1.0 - crate::smoulder::snow_cover(cold, t)
    }

    /// Seconds since a remain of `kind` `age` seconds old went cold
    /// (`smoulder::spans`), negative while it is still hot.
    fn cold_for(&self, kind: crate::smoulder::Kind, age: f32, t: &crate::tuning::Tuning) -> f32 {
        let (burn, smoulder, wisps) = crate::smoulder::spans(kind, crate::smoulder::douse(self.weather, t), t);
        age - burn - smoulder - wisps
    }

    /// The aftermath on the floor (docs/aftermath.md): under a snowy sky the
    /// wet ring each hot wreck melts round itself (the snow over the cold
    /// marks is their own fade, `snow_keep`); then the embers glinting in
    /// every hot bed, heap of charred rubble, stump and ruin, and their
    /// flare-ups. Nothing on a field nothing has burnt on.
    fn paint_aftermath_floor(&self, c: &mut impl Canvas) {
        let t = crate::tuning::tuning();
        let hots = self.smouldering();
        if self.weather == crate::map::Weather::Snow {
            let mut melt = Vec::new();
            for hot in hots.iter().filter(|h| h.kind == crate::smoulder::Kind::Wreck && !c.culls(h.at)) {
                crate::smoulder::melt(hot, &t, &mut melt);
            }
            crate::pyro::draw(c, &melt);
        }
        let mut glints = Vec::new();
        for hot in hots.iter().filter(|h| h.kind != crate::smoulder::Kind::Wreck && !c.culls(h.at)) {
            let radius = match hot.kind {
                crate::smoulder::Kind::Bed => 18.0,
                crate::smoulder::Kind::Tower => 16.0,
                crate::smoulder::Kind::Ruin => 13.0,
                crate::smoulder::Kind::Crater => 10.0,
                _ => 6.0,
            };
            crate::smoulder::glints(hot, self.time, radius, &t, &mut glints);
            crate::smoulder::flare(hot, self.time, radius, &t, &mut glints);
        }
        crate::pyro::draw(c, &glints);
    }

    /// How much snow lies on `tank`'s deck (`smoulder::snow_cover`): under a
    /// snowy sky, from the moment its wreck goes cold; 0 otherwise.
    fn hulk_snow(&self, tank: &Tank, t: &crate::tuning::Tuning) -> f32 {
        if self.weather != crate::map::Weather::Snow || !tank.is_wreck() {
            return 0.0;
        }
        let (burn, smoulder, wisps) = crate::smoulder::spans(crate::smoulder::Kind::Wreck, crate::smoulder::douse(self.weather, t), t);
        crate::smoulder::snow_cover(tank.wreck_age - burn - smoulder - wisps, t)
    }

    /// What the aftermath puts in the air over `view` this frame
    /// (docs/aftermath.md), composed once for the painter to lay down in
    /// two places: the smoke's shadows on the floor, everything else over
    /// what stands. The columns are the hottest `smoulder_max_columns` in
    /// view; the haze's banks thin round every live seat. `None` is the
    /// whole field.
    pub fn aftermath_air(&self, view: Option<crate::math::Rectangle>) -> AftermathAir {
        let t = crate::tuning::tuning();
        let mut air = AftermathAir::default();
        let (w, h) = self.map.field_size();
        let view = view.unwrap_or(crate::math::Rectangle::new(0.0, 0.0, w, h));
        let inside = |p: crate::Position| {
            let m = 160.0;
            p.x >= view.x - m && p.x <= view.x + view.width + m && p.y >= view.y - m && p.y <= view.y + view.height + 2.0 * m
        };
        let mut hots: Vec<(f32, crate::smoulder::Hot)> = self.smouldering().into_iter().filter(|h| inside(h.at)).map(|h| (h.load(&t), h)).collect();
        hots.sort_by(|a, b| b.0.total_cmp(&a.0));
        let sunlit = crate::smoulder::sunlit(self.weather);
        for (_, hot) in hots.iter().take(t.smoulder_max_columns.max(0) as usize) {
            crate::smoulder::column(hot, self.time, self.weather, &t, &mut air.smoke, sunlit.then_some(&mut air.shadows));
        }
        let haze = self.battle_haze;
        if haze > 0.0 {
            let seats: Vec<crate::Position> = self
                .world
                .query::<(Entity, &Tank)>()
                .iter()
                .filter(|&(e, tank)| self.is_player(e) && !tank.is_wreck())
                .map(|(_, tank)| tank.position)
                .collect();
            let pale = matches!(self.weather, crate::map::Weather::Night | crate::map::Weather::Storm) || self.night_fallen;
            crate::smoulder::haze_banks(view, &seats, self.time, haze, pale, &t, &mut air.smoke);
            crate::smoulder::ash(view, self.time, haze, &t, &mut air.smoke);
            air.grade = crate::smoulder::haze_grade(haze, &t).map(|color| (view, color));
        }
        air
    }

    /// The shadows of what flies - the FPV drones (docs/fpv-swarm.md
    /// "Drawing") and air support's aircraft (docs/air-support.md
    /// "Drawing") - on the ground under them: after the floor, under the
    /// tiles and the tanks. Nothing while shadows are off.
    pub fn paint_air_shadows(&self, c: &mut impl Canvas) {
        if !self.shadows_enabled {
            return;
        }
        paint_drone_shadows(c, &self.drones());
        crate::air_support::paint_shadows(c, &self.aircraft(), self.time);
    }

    /// What flies, over everything that stands: the FPV drones' bodies
    /// (their lamps are the glowing pass's), then air support's aircraft
    /// over them - it flies highest.
    pub fn paint_air(&self, c: &mut impl Canvas) {
        paint_drones(c, &self.drones(), self.time);
        crate::air_support::paint_aircraft(c, &self.aircraft(), self.time);
    }

    /// The field on a canvas the caller has cleared: `paint_floor`,
    /// `paint_air_shadows`, `paint_tiles`, `paint_standing`, `paint_air`, in
    /// that order. What `mapshot` renders (docs/mapshot-prd.md) - a fresh
    /// round, where nothing flies yet - and a headless round's
    /// `screenshot`; `render` runs the same stages with its live-round
    /// layers in between.
    pub fn paint_field(&self, c: &mut impl Canvas, opts: PaintOptions) {
        self.paint_floor(c);
        self.paint_air_shadows(c);
        self.paint_tiles(c);
        self.paint_standing(c, opts);
        self.paint_air(c);
    }
}
