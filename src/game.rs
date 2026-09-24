//! The presentation layer: reads `Game`'s state (owned by `simulation.rs`)
//! and draws it. Nothing here ever mutates simulation state - `render`
//! takes `&self` - so this module can freely depend on `RaylibHandle`
//! and friends without that dependency leaking back into `simulation.rs`.
//! See `simulation.rs`'s module doc comment for the other half of this split.

use crate::tuning::tuning;
use sola_raylib::prelude::*;

use crate::bullet::{Bullet, BulletState, draw_bullet, draw_bullet_shadow};
use crate::damage_stage::draw_damage;
use crate::canvas::{Canvas, GpuCanvas, Sheet, Sheets};
use crate::portal::{draw_portal, draw_portal_glow};
use crate::frog::{FrogVariantTextures, draw_frog, draw_frog_ring};
use crate::laser::draw_laser_beam;
use crate::blast::{draw_blast, draw_blast_glow, draw_burning_hull_glow, draw_fire_glow, draw_flame_glow, draw_fuse_glow, draw_ground_fire, draw_scorch};
use crate::decal::{draw_decal, draw_decal_shadow};
use crate::obstacle::{draw_flying_drum, draw_obstacle_cap, draw_oil_cell, draw_tree, draw_tree_shadow, tree_lean, Material, Obstacle, draw_obstacle, draw_obstacle_shadow, fence_axis};
#[cfg(feature = "dev-tools")]
use crate::ai::Ai;
use hecs::Entity;
use std::collections::HashSet;
use crate::pickup::{Pickup, PickupKind, draw_pickup};
use crate::missile::{Missile, draw_missile, draw_missile_shadow};
use crate::plasma::{Plasma, PlasmaState, draw_plasma, draw_plasma_shadow};
use crate::shell::{Shell, ShellState, draw_shell, draw_shell_shadow};
use crate::hud::{
    draw_bar, draw_leave_dialog, draw_mode_button, draw_players_button, draw_players_dialog, draw_restart_button, version_line, HudModel,
    PlayChrome, BUILD_COLOR, HUD_VERSION_BOTTOM_INSET, HUD_VERSION_COLOR, HUD_VERSION_RIGHT_INSET, HUD_VERSION_TEXT_SIZE,
};
use crate::shockwave::{RippleFx, screen_to_ripple_uv};
use crate::view::View;
use crate::simulation::{Game, Outcome};
#[cfg(feature = "dev-tools")]
use crate::simulation::Overlays;
#[cfg(feature = "dev-tools")]
use crate::tank::Dir;
#[cfg(feature = "dev-tools")]
use crate::tank::ActiveWeapon;
use crate::tank::{
    Tank, draw_enemy_ring, draw_minigun_mount, draw_minigun_mount_shadow, draw_missile_pod, draw_missile_pod_shadow,
    draw_player_label, draw_player_locate,
    draw_player_ring, draw_tank, draw_tank_shadow, draw_tank_shield,
};
use crate::track::draw_track;
use crate::{Layout, SHOCK_MAX};
#[cfg(feature = "dev-tools")]
use crate::{HUD_MARGIN, MAX_DAMAGE};

/// The sprite atlases `Game::render` draws from, bundled into one param instead
/// of four so the signature doesn't grow with every new texture.
pub struct Textures<'a> {
    pub tanks: &'a Texture2D,
    pub shells: &'a Texture2D,
    pub plasma: &'a Texture2D,
    pub minigun_bullets: &'a Texture2D,
    /// static/missile.png - a seeker missile in flight (missile.rs).
    pub missile: &'a Texture2D,
    pub damage: &'a Texture2D,
    pub tracks: &'a Texture2D,
    pub obstacles: &'a Texture2D,
    /// The props sheet (sandbags, barrels, fences) - see `obstacle::Sheet`.
    pub props: &'a Texture2D,
    /// The barrel blast animation and scorch decals - see `blast.rs`.
    pub barrel_explosion: &'a Texture2D,
    pub ground: &'a Texture2D,
    /// One `FrogVariantTextures` per `frog::FROG_VARIANT_DIRS` entry, in the
    /// same order - `render` indexes into this by `Frog::variant`.
    pub frog_variants: &'a [FrogVariantTextures],
    pub pickup_health: &'a Texture2D,
    pub pickup_ammo: &'a Texture2D,
    pub pickup_laser: &'a Texture2D,
    pub pickup_minigun: &'a Texture2D,
    pub pickup_plasma: &'a Texture2D,
    pub pickup_missiles: &'a Texture2D,
    pub pickup_speedup: &'a Texture2D,
    pub pickup_shield: &'a Texture2D,
    pub pickup_flamethrower: &'a Texture2D,
    pub pickup_frog_health: &'a Texture2D,
    /// The minigun barrel-cluster overlay drawn on a tank's turret while it
    /// holds minigun ammo - see `tank::draw_minigun_mount`. One shared
    /// texture for every chassis (unlike `tanks` above), not a sheet.
    pub minigun_mount: &'a Texture2D,
    /// The seeker-missile pod on a turret while the tank holds missiles -
    /// see `tank::draw_missile_pod`. One texture for every chassis.
    pub missile_pod: &'a Texture2D,
    /// The tall-grass sheet of the round's map theme
    /// (`map::Theme::grass_texture_path`, grass.rs); `ground` above is the
    /// theme's ground tileset the same way. `app.rs` picks both per frame.
    pub grass: &'a Texture2D,
    /// static/trees_sheet.png - the two tree species (docs/TREES_SPEC.md).
    pub trees: &'a Texture2D,
    /// static/portal_sheet.png - the turning spiral (portal.rs).
    pub portal: &'a Texture2D,
}

/// The game's `Sheet` lookup: what a `GpuCanvas` over these textures blits
/// from. Every sheet the field can name is here.
impl Sheets for Textures<'_> {
    fn texture(&self, sheet: Sheet) -> &Texture2D {
        match sheet {
            // `app.rs` picks `ground`/`grass` from the round's map theme
            // every frame, the same theme `paint_floor`/`paint_standing`
            // name here, so the payload needs no second lookup.
            Sheet::Ground(_) => self.ground,
            Sheet::Tanks => self.tanks,
            Sheet::Walls => self.obstacles,
            Sheet::Props => self.props,
            Sheet::Trees => self.trees,
            Sheet::Grass(_) => self.grass,
            Sheet::Damage => self.damage,
            Sheet::MinigunMount => self.minigun_mount,
            Sheet::MissilePod => self.missile_pod,
            Sheet::Tracks => self.tracks,
            Sheet::BarrelExplosion => self.barrel_explosion,
            Sheet::Portal => self.portal,
            Sheet::Pickup(kind) => match kind {
                PickupKind::Health => self.pickup_health,
                PickupKind::Ammo => self.pickup_ammo,
                PickupKind::Laser => self.pickup_laser,
                PickupKind::Minigun => self.pickup_minigun,
                PickupKind::Plasma => self.pickup_plasma,
                PickupKind::Missiles => self.pickup_missiles,
                PickupKind::SpeedUp => self.pickup_speedup,
                PickupKind::Shield => self.pickup_shield,
                PickupKind::Flamethrower => self.pickup_flamethrower,
                PickupKind::FrogHealth => self.pickup_frog_health,
            },
            Sheet::Frog { variant, clip } => self.frog_variants[variant as usize % self.frog_variants.len()].clip(clip),
        }
    }
}

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
        draw_missile_pod_shadow(c, tank);
    }
    draw_tank(c, tank);
    draw_minigun_mount(c, tank);
    draw_missile_pod(c, tank);
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

/// The ripple post-effects `Game::render` drives, bundled into one param for the
/// same reason as `Textures`. See `shockwave.rs` for what each one does.
pub struct Effects<'a> {
    pub shock: &'a mut RippleFx,
    pub muzzle: &'a mut RippleFx,
    pub impact: &'a mut RippleFx,
    /// The short-lived particle layer. Owned by `main.rs`, not by `Game`
    /// (see `fx.rs`), and read-only here - `render` never mutates it, the
    /// same contract it has with `Game`.
    pub fx: &'a crate::fx::Fx,
    /// The touch scheme's feedback (`touch.rs`), drawn over the field in
    /// bitmap space when a keyboard-less device is playing; `None` draws
    /// nothing. The `bool` is whether the stick lives on the right half.
    pub touch: Option<(&'a crate::touch::TouchScheme, bool)>,
}

impl Game {
    /// Draw the whole scene for this frame: the battlefield into
    /// `scene_target`, then that plus the HUD bar into `composite` - the
    /// bitmap, `layout.window_size()` in size, the field at `layout.field`
    /// and the bar in `layout.panel` - and finally the bitmap onto the
    /// window through `view` (`view::present`), scaled and centred so the
    /// whole battlefield is on screen whatever the window is. Everything
    /// field-relative in the second pass goes through a `Camera2D` whose
    /// offset is the field origin, so the simulation's pixel positions
    /// stay usable as they are.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        scene_target: &mut RenderTexture2D,
        composite: &mut RenderTexture2D,
        view: &View,
        backdrop: Color,
        effects: &mut Effects,
        textures: &Textures,
        layout: &Layout,
        chrome: &PlayChrome,
    ) {
        let screen_width = layout.field.w.round() as i32;
        let screen_height = layout.field.h.round() as i32;
        // Must be read off the handle out here: the draw closures below
        // borrow it, and the dev label needs the number.
        #[cfg(feature = "dev-tools")]
        let frame_ms = rl.get_frame_time() * 1000.0;
        let player = self.player.expect("player entity spawned in init");

        let hud = HudModel::gather(self);
        // The build stamp, bottom-right of the field (text width must be
        // measured on the RaylibHandle, outside the draw closure).
        let version = version_line();
        let version_w = rl.measure_text(&version, HUD_VERSION_TEXT_SIZE);

        // Precompute the centered end-of-round banner (text width must be
        // measured on the RaylibHandle, outside the draw closure).
        let banner = match self.outcome {
            Outcome::Playing => None,
            Outcome::Won => Some(("YOU WIN", Color::DARKGREEN)),
            Outcome::Lost => Some(("YOU LOSE", Color::MAROON)),
        };
        // Opening mission banner: solid while the round is frozen behind
        // it, then fading over INTRO_FADE_SECONDS once play starts.
        let intro = {
            let alpha = if self.intro_timer > 0.0 {
                1.0
            } else {
                (self.intro_fade / crate::simulation::INTRO_FADE_SECONDS).clamp(0.0, 1.0)
            };
            (alpha > 0.0).then(|| {
                let text = self.mission.banner();
                let size = 72;
                let w = rl.measure_text(text, size);
                (text, size, w, alpha)
            })
        };
        // Wave rounds: the `WAVE N` banner during the breather before a
        // wave - smaller than the mission banner, no dim overlay, and never
        // over the end-of-round banner. The counter itself is in the bar.
        let wave_banner = self.wave_banner().filter(|_| self.outcome == Outcome::Playing).map(|text| {
            let size = 48;
            let w = rl.measure_text(&text, size);
            (text, size, w)
        });
        let banner = banner.map(|(text, color)| {
            let title_size = 72;
            let title_w = rl.measure_text(text, title_size);
            let sub = format!(
                "Restarting in {}...",
                self.restart_timer.ceil().max(0.0) as i32
            );
            let sub_size = 28;
            let sub_w = rl.measure_text(&sub, sub_size);
            (text, color, title_size, title_w, sub, sub_size, sub_w)
        });
        let paused_w = rl.measure_text("PAUSED", 72);

        // Pass 1: draw the world (tracks, tanks, shells) into an offscreen
        // render texture, so a shockwave can distort the finished frame as a
        // whole-screen shader pass in pass 2.
        rl.draw_texture_mode(thread, scene_target, |mut d| {
            d.clear_background(Color::WHITE);

            // The field itself is painted through the `Canvas` trait in the
            // three stages `mapshot` also runs on a CPU canvas (`paint_floor`,
            // `paint_tiles`, `paint_standing`); between them come the layers
            // only a live round has - fire, glows, the locate label,
            // projectiles, blasts, airborne debris, particles - drawn with
            // raylib directly. A `GpuCanvas` borrows `d` for one statement,
            // so each stage makes its own.
            self.paint_floor(&mut GpuCanvas::new(&mut d, textures));

            // Burning ground cells: the flames over the ground, under the
            // tiles beside them (a burning doorway's walls still stand
            // over the fire) and under whatever drives through them.
            for (at, left, total) in self.burning_cells() {
                draw_ground_fire(&mut d, textures.barrel_explosion, at, self.time, left, total);
            }

            self.paint_tiles(&mut GpuCanvas::new(&mut d, textures));

            // A barrel whose fuse is lit pulses (additive, so it reads as
            // light on the drum rather than a disc over it).
            d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
                // An active portal glows from below its spiral.
                if self.portals_active() {
                    for &at in &self.portals {
                        draw_portal_glow(&mut bd, at, self.time);
                    }
                }
                for obstacle in self.world.query::<&Obstacle>().iter() {
                    if obstacle.fuse.is_some() {
                        draw_fuse_glow(&mut bd, obstacle.position, self.time);
                    }
                }
                for (at, left, _total) in self.burning_cells() {
                    draw_fire_glow(&mut bd, at, self.time, left);
                }
                // The flamethrower's nozzle while it fires: a hot disc at
                // the muzzle and a fainter one a third of the way out.
                for jet in self.flames() {
                    draw_flame_glow(&mut bd, jet.origin, jet.dir, jet.reach, self.time);
                }
                // A hull with afterburn on it glows under its embers, so
                // the state reads between particles too.
                for (at, left) in self.burning_tanks() {
                    draw_burning_hull_glow(&mut bd, at, self.time, left);
                }
            });

            self.paint_standing(&mut GpuCanvas::new(&mut d, textures), PaintOptions { locate_cue: true });

            // The locate cue's P1/P2 labels, over the grass, the crowd and
            // the trees - the point is to be found under all of it.
            if !self.hide_players {
                for entity in [Some(player), self.player2].into_iter().flatten() {
                    crate::simulation::with_tank(&self.world, entity, |tank| draw_player_label(&mut d, tank, self.time));
                }
            }

            for shell in self.world.query::<&Shell>().iter() {
                if self.shadows_enabled && shell.state == ShellState::Flying {
                    draw_shell_shadow(&mut d, textures.shells, shell);
                }
                draw_shell(&mut d, textures.shells, shell);
            }

            for plasma in self.world.query::<&Plasma>().iter() {
                if self.shadows_enabled && plasma.state == PlasmaState::Flying {
                    draw_plasma_shadow(&mut d, textures.plasma, plasma);
                }
                draw_plasma(&mut d, textures.plasma, plasma);
            }

            for bullet in self.world.query::<&Bullet>().iter() {
                if self.shadows_enabled && bullet.state == BulletState::Flying {
                    draw_bullet_shadow(&mut d, textures.minigun_bullets, bullet);
                }
                draw_bullet(&mut d, textures.minigun_bullets, bullet);
            }

            for beam in &self.laser_beams {
                draw_laser_beam(&mut d, beam);
            }

            // Barrel blasts last, so the fireball covers tanks and shots:
            // the additive bloom first, then the sprite frames oldest
            // first (a chained blast's flash lands on top of the earlier
            // fireball and reads as a second detonation).
            d.draw_blend_mode(BlendMode::BLEND_ADDITIVE, |mut bd| {
                for blast in &self.blast_fx {
                    draw_blast_glow(&mut bd, blast);
                }
            });
            for blast in &self.blast_fx {
                match &blast.cloud {
                    Some(cloud) => crate::mushroom::draw(&mut GpuCanvas::new(&mut d, textures), cloud, blast.center, blast.time),
                    None => draw_blast(&mut d, textures.barrel_explosion, blast),
                }
            }

            // Parts still in the air, last of all: a chunk of hull thrown
            // off a wreck passes over tanks and shells, not under them.
            // Their shadows go down first so no piece is drawn over
            // another's shadow.
            if self.shadows_enabled {
                for decal in self.decals.iter().filter(|dc| !dc.landed()) {
                    draw_decal_shadow(&mut d, decal);
                }
            }
            {
                let mut c = GpuCanvas::new(&mut d, textures);
                for decal in self.decals.iter().filter(|dc| !dc.landed()) {
                    draw_decal(&mut c, decal);
                }
                // Launched fuel drums, tumbling over the lot on their way to
                // where they go off.
                for drum in &self.flying_drums {
                    draw_flying_drum(&mut c, drum, self.shadows_enabled);
                }
            }

            // Seeker missiles are the highest thing in the air: over the
            // debris, shadows first so none lands on another missile.
            if self.shadows_enabled {
                for missile in self.world.query::<&Missile>().iter() {
                    draw_missile_shadow(&mut d, textures.missile, missile);
                }
            }
            for missile in self.world.query::<&Missile>().iter() {
                draw_missile(&mut d, textures.missile, missile);
            }

            // Sparks, chips, dust and smoke over the top of everything in
            // the scene, but still inside pass 1 so an in-flight shockwave
            // warps them and the camera shake carries them along.
            effects.fx.draw(&mut d);
        });

        // If a shockwave is playing, push its current center/time to the shader
        // before the blit below samples through it.
        // Every live ripple goes up as one set of arrays and resolves in a
        // single blit below - see static/shockwave.fs for why N passes
        // would be both wrong and slower. Unused slots carry gain 0.
        if !self.shocks.is_empty() {
            let mut centers = [Vector2::new(0.0, 0.0); SHOCK_MAX];
            let mut times = [0.0f32; SHOCK_MAX];
            let mut gains = [0.0f32; SHOCK_MAX];
            for (i, shock) in self.shocks.iter().take(SHOCK_MAX).enumerate() {
                centers[i] = screen_to_ripple_uv(shock.center, screen_width as f32, screen_height as f32);
                times[i] = shock.time;
                gains[i] = shock.strength;
            }
            effects.shock.shader.set_shader_value_v(effects.shock.centers_loc, &centers);
            effects.shock.shader.set_shader_value_v(effects.shock.times_loc, &times);
            effects.shock.shader.set_shader_value_v(effects.shock.gains_loc, &gains);
        }

        // The render texture is stored upside-down relative to the screen; a
        // negative source height flips it back on the way out.
        let source = Rectangle {
            x: 0.0,
            y: 0.0,
            width: screen_width as f32,
            height: -(screen_height as f32),
        };

        // Camera shake: a short decaying wobble on the same kill-shockwave
        // trigger as `self.shock` itself (see CAMERA_SHAKE_DURATION's doc
        // comment), applied purely as an offset on this blit's destination
        // rather than a real camera - the game has no camera transform (see
        // `Shockwave::center`'s doc comment), so shifting where the already-
        // composited scene lands on screen is the cheapest way to get the
        // effect. Two out-of-phase sine waves stand in for randomness so x/y
        // don't shake in lockstep - this is a pure draw pass (see this
        // module's own doc comment), not the place to reach for an rng.
        // Muzzle/impact flash quads and the HUD deliberately aren't shifted:
        // they're either their own small on-screen quad or meant to stay put.
        // The field origin is added on top: the scene lands below the bar.
        let mut blit_offset = Vector2::new(0.0, 0.0);
        // `screen_fx_intensity` scales every whole-screen effect together;
        // folding it into the magnitude here keeps the stack cap below
        // proportional.
        let shake_magnitude = tuning().camera_shake_magnitude * tuning().screen_fx_intensity;
        for shock in &self.shocks {
            let decay = (1.0f32 - shock.time / tuning().camera_shake_duration).max(0.0);
            if decay <= 0.0 {
                continue;
            }
            let mag = shake_magnitude * shock.strength * decay;
            // Phase each one off its own position hash so several
            // overlapping shakes interfere instead of beating in lockstep
            // and doubling cleanly. Still a pure draw pass, still no rng -
            // see the comment above.
            let phase = (crate::blast::seed_for(shock.center) % 628) as f32 * 0.01;
            let t = shock.time * tuning().camera_shake_frequency + phase;
            blit_offset.x += t.sin() * mag;
            blit_offset.y += (t * 1.3 + 1.7).sin() * mag;
        }
        // Without a ceiling, three kills at once throw the composited scene
        // far enough off that the screen edge shows through as black.
        let cap = shake_magnitude * tuning().camera_shake_max_stack;
        let len = (blit_offset.x * blit_offset.x + blit_offset.y * blit_offset.y).sqrt();
        if len > cap && len > 0.0 {
            blit_offset.x *= cap / len;
            blit_offset.y *= cap / len;
        }
        // Snap the shake to whole 2px blocks. This offset moves the entire
        // composited scene, so at a fractional value every pixel in the
        // game samples between texels for the duration of the shake and
        // the whole screen shimmers - the same defect a sub-pixel particle
        // has, at the scale of everything at once. Blocks keep the art
        // crisp and make the shake read as a hard jolt rather than a
        // wobble.
        blit_offset.x = (blit_offset.x / 2.0).round() * 2.0;
        blit_offset.y = (blit_offset.y / 2.0).round() * 2.0;

        let origin = layout.field_origin();
        let blit_at = Vector2::new(blit_offset.x + origin.x, blit_offset.y + origin.y);
        let field_camera = Camera2D {
            offset: origin,
            target: Vector2::new(0.0, 0.0),
            rotation: 0.0,
            zoom: 1.0,
        };

        rl.draw_texture_mode(thread, composite, |mut d| {
            d.clear_background(Color::BLACK);

            if !self.shocks.is_empty() {
                d.draw_shader_mode(&mut effects.shock.shader, |mut sd| {
                    sd.draw_texture_rec(&*scene_target, source, blit_at, Color::WHITE);
                });
            } else {
                d.draw_texture_rec(&*scene_target, source, blit_at, Color::WHITE);
            }

            // Everything from here to the bar is field-relative: the flash
            // quads, the overlays, the banners and the dims all centre on
            // and cover the field, never the bar.
            d.draw_mode2D(field_camera, |mut d, _| {
                // Layer each muzzle flash's tiny heat-haze ripple on top, one small
                // quad at a time: source and dest are the same on-screen patch (just
                // re-sampling that bit of the already-composited scene through the
                // ripple shader), so this reads as a localized wobble rather than
                // redistorting the whole frame.
                for flash in &self.muzzle_flashes {
                    let uv =
                        screen_to_ripple_uv(flash.center, screen_width as f32, screen_height as f32);
                    effects
                        .muzzle
                        .shader
                        .set_shader_value(effects.muzzle.center_loc, uv);
                    effects
                        .muzzle
                        .shader
                        .set_shader_value(effects.muzzle.time_loc, flash.time);

                    let r = tuning().muzzle_flash_quad_radius;
                    let flash_source = Rectangle {
                        x: flash.center.x - r,
                        y: (screen_height as f32 - flash.center.y) - r,
                        width: r * 2.0,
                        height: -(r * 2.0),
                    };
                    let flash_dest = Rectangle {
                        x: flash.center.x,
                        y: flash.center.y,
                        width: r * 2.0,
                        height: r * 2.0,
                    };
                    let origin = Vector2::new(r, r);

                    d.draw_shader_mode(&mut effects.muzzle.shader, |mut sd| {
                        sd.draw_texture_pro(
                            &*scene_target,
                            flash_source,
                            flash_dest,
                            origin,
                            0.0,
                            Color::WHITE,
                        );
                    });
                }

                // Same treatment for every in-flight shell-impact flash.
                for flash in &self.impact_flashes {
                    let uv =
                        screen_to_ripple_uv(flash.center, screen_width as f32, screen_height as f32);
                    effects
                        .impact
                        .shader
                        .set_shader_value(effects.impact.center_loc, uv);
                    effects
                        .impact
                        .shader
                        .set_shader_value(effects.impact.time_loc, flash.time);

                    let r = tuning().impact_flash_quad_radius;
                    let flash_source = Rectangle {
                        x: flash.center.x - r,
                        y: (screen_height as f32 - flash.center.y) - r,
                        width: r * 2.0,
                        height: -(r * 2.0),
                    };
                    let flash_dest = Rectangle {
                        x: flash.center.x,
                        y: flash.center.y,
                        width: r * 2.0,
                        height: r * 2.0,
                    };
                    let origin = Vector2::new(r, r);

                    d.draw_shader_mode(&mut effects.impact.shader, |mut sd| {
                        sd.draw_texture_pro(
                            &*scene_target,
                            flash_source,
                            flash_dest,
                            origin,
                            0.0,
                            Color::WHITE,
                        );
                    });
                }

                // A kill or a barrel blast opens with a brief whole-screen
                // flash (`Game::screen_flash`, started and spaced out by
                // `Game::flash_screen`). After the ripple quads, which re-blit
                // patches of the un-flashed scene and would otherwise punch
                // darker squares through it.
                if let Some(age) = self.screen_flash {
                    let seconds = tuning().blast_screen_flash_seconds;
                    if seconds > 0.0 && age < seconds {
                        let peak = tuning().blast_screen_flash_alpha * tuning().screen_fx_intensity;
                        let a = (255.0 * peak.clamp(0.0, 1.0) * (1.0 - age / seconds)) as u8;
                        d.draw_rectangle(0, 0, screen_width, screen_height, Color::new(255, 240, 200, a));
                    }
                }

                // Debug overlays (dev builds only): the two tank layers -
                // hitbox/collider outlines and the stats card, each on its
                // own flag - for every tank, then the dev server's other
                // layers. Drawn here (screen space, post-composite) rather
                // than into scene_target, so they're never warped by an
                // in-flight shockwave and always render crisp - tank.position
                // is already screen pixels (no camera transform), so the two
                // spaces line up 1:1 with no extra math.
                #[cfg(feature = "dev-tools")]
                {
                    let ov = self.debug_overlays;
                    if ov.hitboxes || ov.stats {
                        for (tank, ai) in self.world.query::<(&Tank, &Ai)>().iter() {
                            draw_tank_layers(&mut d, ov, tank, Some(ai));
                        }
                        for entity in [Some(player), self.player2].into_iter().flatten() {
                            crate::simulation::with_tank(&self.world, entity, |tank| {
                                draw_tank_layers(&mut d, ov, tank, None);
                            });
                        }
                    }
                    self.draw_debug_overlays(&mut d, screen_width as f32, screen_height as f32);
                    // Which preset is live, in the field's top-left corner (the
                    // player's readouts are in the bar, so this corner is free),
                    // so the I key's cycling is visible without counting layers.
                    if let Some(preset) = ov.preset_name() {
                        // Frame time and live particle count ride the same
                        // label: the FX budget has to hold on the wasm build,
                        // and without a number on screen that is an assertion
                        // nobody can check while playing.
                        let label = format!(
                            "DEV overlays: {preset} (I cycles)  |  {:.1} ms  {} fx",
                            frame_ms,
                            effects.fx.live(),
                        );
                        const LABEL_FONT_SIZE: i32 = 14;
                        let label_y = HUD_MARGIN;
                        // Same 8px/char width estimate as `draw_tank_stats`'s
                        // panel - no font handle inside the draw closure.
                        let label_w = label.len() as i32 * 8 + 8;
                        d.draw_rectangle(
                            HUD_MARGIN - 4,
                            label_y - 2,
                            label_w,
                            LABEL_FONT_SIZE + 4,
                            Color::new(0, 0, 0, 150),
                        );
                        d.draw_text(
                            &label,
                            HUD_MARGIN,
                            label_y,
                            LABEL_FONT_SIZE,
                            Color::new(80, 200, 255, 255),
                        );
                    }
                }

                // The build stamp along the field's bottom edge, left of
                // the corner the web page's Full screen button sits in.
                d.draw_text(
                    &version,
                    screen_width - HUD_VERSION_RIGHT_INSET - version_w,
                    screen_height - HUD_VERSION_BOTTOM_INSET - HUD_VERSION_TEXT_SIZE,
                    HUD_VERSION_TEXT_SIZE,
                    HUD_VERSION_COLOR,
                );

                // End-of-round banner over a dimming overlay.
                if let Some((title, color, title_size, title_w, sub, sub_size, sub_w)) = &banner {
                    d.draw_rectangle(0, 0, screen_width, screen_height, Color::new(0, 0, 0, 120));
                    let cx = screen_width / 2;
                    let cy = screen_height / 2;
                    d.draw_text(
                        title,
                        cx - title_w / 2,
                        cy - title_size,
                        *title_size,
                        *color,
                    );
                    d.draw_text(sub, cx - sub_w / 2, cy + 20, *sub_size, Color::RAYWHITE);
                }

                // Mission banner: big white text over a dim overlay that both
                // fade together once the round unfreezes.
                if let Some((text, size, w, alpha)) = intro {
                    let a = |max: f32| (max * alpha) as u8;
                    d.draw_rectangle(0, 0, screen_width, screen_height, Color::new(0, 0, 0, a(120.0)));
                    d.draw_text(text, screen_width / 2 - w / 2, screen_height / 2 - size / 2, size, Color::new(255, 255, 255, a(255.0)));
                }
                if let Some((text, size, w)) = &wave_banner {
                    d.draw_text(text, screen_width / 2 - w / 2, screen_height / 2 - size / 2, *size, Color::RAYWHITE);
                }

                // Paused overlay draws over everything else, including the
                // end-of-round banner (its countdown is frozen too).
                if self.paused {
                    d.draw_rectangle(0, 0, screen_width, screen_height, Color::new(0, 0, 0, 120));
                    let title_size = 72;
                    d.draw_text(
                        "PAUSED",
                        screen_width / 2 - paused_w / 2,
                        screen_height / 2 - title_size / 2,
                        title_size,
                        Color::RAYWHITE,
                    );
                }

                // The leave-round question (docs/game-editor-fusion.md
                // section 6): the same dim as PAUSED, the dialog on top.
                // The round is frozen by `main.rs` not calling `update`,
                // so nothing here is simulation state.
                if chrome.leave_dialog || chrome.players_dialog {
                    d.draw_rectangle(0, 0, screen_width, screen_height, Color::new(0, 0, 0, 120));
                }
                if chrome.leave_dialog {
                    draw_leave_dialog(&mut d, layout.field);
                } else if chrome.players_dialog {
                    draw_players_dialog(&mut d, layout.field, self.players);
                }
            });

            // The HUD bar, in window space, over anything the field pass
            // might have put on its edge.
            draw_bar(&mut d, layout.panel, &hud, textures);
            if chrome.players_button {
                draw_players_button(&mut d, layout.panel, self.players, chrome.players_dialog);
            }
            if chrome.restart_button {
                draw_restart_button(&mut d, layout.panel);
            }
            if chrome.build_button {
                draw_mode_button(&mut d, layout.panel, "BUILD", BUILD_COLOR);
            }
            // The touch scheme's stick, ripples and hint: over everything,
            // in bitmap space, so they sit where the thumbs are.
            if let Some((touch, steer_right)) = effects.touch {
                touch.draw(&mut d, layout, steer_right);
            }
        });
        crate::view::present(rl, thread, composite, view, backdrop);
    }
}

/// One tank's dev overlay layers (`Overlays::hitboxes`, `Overlays::stats`;
/// both in the preset the I key cycles to first): the geometry is read
/// once and shared, the boxes go down first so the card stays on top.
#[cfg(feature = "dev-tools")]
fn draw_tank_layers(d: &mut impl RaylibDraw, ov: Overlays, tank: &Tank, ai: Option<&Ai>) {
    let geo = TankInspectGeometry::of(tank);
    if ov.hitboxes {
        draw_tank_boxes(d, tank, ai, &geo);
    }
    if ov.stats {
        draw_tank_stats(d, tank, ai, &geo);
    }
}

/// What both tank layers read off a tank: the facing-oriented hull rect
/// (the stats card anchors at its top-left, so the card needs it whether
/// or not the box is drawn) and the movement collider's size and corner
/// radius (drawn by the boxes, printed by the card's MOVE line).
#[cfg(feature = "dev-tools")]
struct TankInspectGeometry {
    /// The hull damage box as `(x, y, width, height)` in whole pixels.
    hull: (i32, i32, i32, i32),
    /// `Tank::move_half_extents` for the current facing.
    move_half: (f32, f32),
    /// `physics::tank_corner_radius` for those half extents.
    corner: f32,
}

#[cfg(feature = "dev-tools")]
impl TankInspectGeometry {
    fn of(tank: &Tank) -> Self {
        // Same "which axis is the long one" check as `Tank::avoidance_radius` -
        // tanks only ever face one of the four `Dir::rotation()` values, so an
        // exact match is safe here (no epsilon needed).
        let along_x = tank.rotation == Dir::Right.rotation() || tank.rotation == Dir::Left.rotation();
        let (hx, hy) = tank.hull_half_extents(along_x);
        let hull = (
            (tank.position.x - hx).round() as i32,
            (tank.position.y - hy).round() as i32,
            (hx * 2.0).round() as i32,
            (hy * 2.0).round() as i32,
        );
        let move_half = tank.move_half_extents(along_x);
        let corner = crate::physics::tank_corner_radius(move_half);
        Self { hull, move_half, corner }
    }
}

/// The `hitboxes` overlay for one tank: its hull damage box, its
/// turret+barrel damage box, and its (smaller, corner-rounded) movement
/// collider. Purely diagnostic: reads state, draws it, mutates nothing.
///
/// Three shapes, each the *real* thing physics uses, not an approximation:
/// - The lime/orange/gray box is `Tank::hull_half_extents` - the per-row
///   (TANK_HULL_BBOX_BY_ROW), facing-oriented hull silhouette backing the
///   projectile hit box (see `simulation::hits`) - not the full 32x32 sprite tile
///   (`Tank::size`/`Tank::hull_size`), which is a uniform square padded
///   well past the visible hull art on every row.
/// - The yellow box is `Tank::turret_bbox_world` - the turret+barrel
///   silhouette, hit-tested the same way. Together
///   these two boxes are exactly what a shell can hit.
/// - The sky-blue rounded box is `Tank::move_half_extents` +
///   `physics::tank_corner_radius` - the solid movement collider walls/
///   obstacles/other tanks actually block against, deliberately smaller
///   and rounder than the damage boxes (see TANK_MOVE_BBOX_FRACTION/
///   TANK_MOVE_CORNER_RADIUS in `lib.rs`). The visible gap between blue
///   and lime is the tuning surface: widen it (smaller fraction) for more
///   forgiving driving, shrink it if sprites start visibly clipping into
///   walls. The `stats` card's MOVE line prints its current world-px size
///   and corner radius for the same purpose.
#[cfg(feature = "dev-tools")]
fn draw_tank_boxes(d: &mut impl RaylibDraw, tank: &Tank, ai: Option<&Ai>, geo: &TankInspectGeometry) {
    let (x, y, width, height) = geo.hull;
    let box_color = if tank.is_wreck() {
        Color::GRAY
    } else if ai.is_some_and(Ai::is_retreating) {
        Color::ORANGE
    } else {
        Color::LIME
    };
    d.draw_rectangle_lines(x, y, width, height, box_color);

    // Turret+barrel bbox overlay (`TANK_TURRET_BBOX_BY_ROW`) - drawn in a
    // distinct color purely so it's visually comparable against the hull
    // box above; not the tank's real collider (see that table's own doc
    // comment - the barrel is deliberately excluded from hit detection
    // today, this is here to evaluate whether that should change).
    let (turret_center, turret_half) = tank.turret_bbox_world();
    let tx = (turret_center.x - turret_half.x).round() as i32;
    let ty = (turret_center.y - turret_half.y).round() as i32;
    let tw = (turret_half.x * 2.0).round() as i32;
    let th = (turret_half.y * 2.0).round() as i32;
    d.draw_rectangle_lines(tx, ty, tw, th, Color::YELLOW);

    // Movement collider (see the doc comment above): drawn with the exact
    // clamped corner radius the physics shape carries
    // (`physics::tank_corner_radius`), mapped onto raylib's relative
    // roundness factor (corner radius = roundness * min(w, h) / 2, so the
    // division below inverts that).
    let (mx, my) = geo.move_half;
    let move_rect = Rectangle::new(
        tank.position.x - mx,
        tank.position.y - my,
        mx * 2.0,
        my * 2.0,
    );
    d.draw_rectangle_rounded_lines(move_rect, geo.corner / mx.min(my), 8, Color::SKYBLUE);
}

/// The `stats` overlay for one tank: a small card above its hull rect -
/// ammo, the live weapon and its ammo, health, current speed and velocity,
/// the movement collider's size - and additionally (`ai: Some`, i.e. this
/// isn't a player) whether it's currently retreating to recharge and its
/// fire cooldown, pulled straight from its `Ai` - the same state `ai.rs`'s
/// `wants_retreat`/`fire_interval` act on. Anchored at the hull box's
/// top-left whether or not the `hitboxes` layer draws that box. Purely
/// diagnostic: reads state, draws it, mutates nothing.
#[cfg(feature = "dev-tools")]
fn draw_tank_stats(d: &mut impl RaylibDraw, tank: &Tank, ai: Option<&Ai>, geo: &TankInspectGeometry) {
    let (x, y, _, _) = geo.hull;
    let (mx, my) = geo.move_half;
    let corner = geo.corner;
    let speed = (tank.velocity.x * tank.velocity.x + tank.velocity.y * tank.velocity.y).sqrt();
    // What the trigger fires right now, with its own remaining ammo -
    // under the FIFO inventory (`Tank::weapon_queue`) this advances when
    // the live weapon runs dry (and a first pickup arms it directly), so
    // surface it here to watch the handover live (WPN SHELL duplicates the
    // AMMO line above; harmless, and it keeps this line self-contained).
    let (wpn_name, wpn_ammo) = match tank.active_weapon() {
        ActiveWeapon::Laser => ("LASER", tank.laser_charges),
        ActiveWeapon::Plasma => ("PLASMA", tank.plasma_ammo),
        ActiveWeapon::Minigun => ("MINIGUN", tank.minigun_ammo),
        ActiveWeapon::Missiles => ("MISSILES", tank.missile_ammo),
        ActiveWeapon::Flamethrower => ("FLAME", tank.flame_fuel_seconds()),
        ActiveWeapon::Shell => ("SHELL", tank.shells_ammo),
    };
    let mut lines = vec![
        format!("AMMO {}", tank.shells_ammo),
        format!("WPN {wpn_name} {wpn_ammo}"),
        format!(
            "HP {}/{}",
            (MAX_DAMAGE - tank.damage).max(0.0).round() as i32,
            MAX_DAMAGE as i32
        ),
        format!("SPD {speed:.0}px/s"),
        format!("VEL ({:.0},{:.0})", tank.velocity.x, tank.velocity.y),
        format!("MOVE {:.0}x{:.0} r{corner:.0}", mx * 2.0, my * 2.0),
    ];
    if let Some(ai) = ai {
        lines.push(format!(
            "RETREAT {}",
            if ai.is_retreating() { "YES" } else { "no" }
        ));
        let cooldown = ai.fire_cooldown();
        lines.push(if cooldown <= 0.0 {
            "FIRE ready".to_string()
        } else {
            format!("FIRE {cooldown:.1}s")
        });
    }

    // No font handle available inside this draw closure to measure text
    // width precisely (RaylibDraw doesn't expose it - only RaylibHandle,
    // outside the closure) - an 8px/char estimate at this font size is close
    // enough for a debug-only backing panel.
    const FONT_SIZE: i32 = 14;
    const LINE_H: i32 = 16;
    let block_w = lines.iter().map(|l| l.len()).max().unwrap_or(0) as i32 * 8 + 8;
    let block_h = lines.len() as i32 * LINE_H + 4;
    let text_y = y - block_h - 2;
    d.draw_rectangle(x, text_y - 2, block_w, block_h, Color::new(0, 0, 0, 150));
    for (i, line) in lines.iter().enumerate() {
        d.draw_text(
            line,
            x + 4,
            text_y + i as i32 * LINE_H,
            FONT_SIZE,
            Color::LIME,
        );
    }
}

#[cfg(feature = "dev-tools")]
impl Game {
    /// The debug overlay layers beyond the two tank layers
    /// (`Game::debug_overlays`, docs/dev-server-design.md; dev builds only),
    /// screen space and post-composite like them: blocked nav cells, each
    /// enemy's AI memory, projectile hit boxes, engagement targets, pickup
    /// collect radii. Each layer costs nothing while off.
    fn draw_debug_overlays(&self, d: &mut impl RaylibDraw, width: f32, height: f32) {
        let ov = self.debug_overlays;
        if ov.nav_grid {
            // The grid the enemies actually steer by this frame, prices
            // and fields included - not the bare `nav_grid`.
            let grid = self.route_grid(width, height);
            let (cols, rows, cell) = grid.dims();
            let size = cell.round() as i32;
            let goal = grid.goals().next().map(|(c, r)| Vector2::new((c as f32 + 0.5) * cell, (r as f32 + 0.5) * cell));
            for row in 0..rows {
                for col in 0..cols {
                    let x = (col as f32 * cell).round() as i32;
                    let y = (row as f32 * cell).round() as i32;
                    if grid.is_blocked(col, row) {
                        d.draw_rectangle(x, y, size, size, Color::new(255, 40, 40, 60));
                        d.draw_rectangle_lines(x, y, size, size, Color::new(255, 40, 40, 110));
                        continue;
                    }
                    // A priced cell: amber, deeper the dearer, capped so a
                    // ford at 8 and a lane at 4 both read as "priced".
                    let extra = grid.cost_at(col, row).saturating_sub(1);
                    if extra > 0 {
                        let alpha = (40 + extra.min(8) * 18) as u8;
                        d.draw_rectangle(x, y, size, size, Color::new(255, 170, 40, alpha));
                    }
                    // Player 1's flow: a short line from the centre toward
                    // the cell the field steps into, tipped with a dot.
                    if let Some(goal) = goal
                        && let Some((nc, nr)) = grid.flow(goal, col, row)
                    {
                        let (cx, cy) = (x as f32 + cell * 0.5, y as f32 + cell * 0.5);
                        let (dx, dy) = (nc as f32 - col as f32, nr as f32 - row as f32);
                        let (ex, ey) = (cx + dx * cell * 0.35, cy + dy * cell * 0.35);
                        d.draw_line(cx as i32, cy as i32, ex as i32, ey as i32, Color::new(80, 220, 255, 150));
                        d.draw_rectangle(ex as i32 - 1, ey as i32 - 1, 2, 2, Color::new(80, 220, 255, 220));
                    }
                }
            }
            if let Some(goal) = goal {
                d.draw_rectangle_lines((goal.x - cell * 0.5) as i32, (goal.y - cell * 0.5) as i32, size, size, Color::new(80, 220, 255, 220));
            }
        }
        if ov.pickups {
            let radius = tuning().pickup_collect_radius;
            for pickup in self.world.query::<&Pickup>().iter() {
                d.draw_circle_lines(pickup.position.x as i32, pickup.position.y as i32, radius, Color::GOLD);
            }
        }
        if ov.projectiles {
            let mut boxes: Vec<(Vector2, Vector2, f32)> = Vec::new();
            for s in self.world.query::<&Shell>().iter() {
                boxes.push((s.position, s.velocity, tuning().shell_hit_half_extent));
            }
            for p in self.world.query::<&Plasma>().iter() {
                boxes.push((p.position, p.velocity, tuning().plasma_hit_half_extent));
            }
            for b in self.world.query::<&Bullet>().iter() {
                boxes.push((b.position, b.velocity, tuning().minigun_bullet_hit_half_extent));
            }
            // A missile has no hit box: its aim point, and the line to it.
            for m in self.world.query::<&Missile>().iter() {
                d.draw_line(m.position.x as i32, m.position.y as i32, m.aim.x as i32, m.aim.y as i32, Color::new(255, 120, 0, 160));
                d.draw_circle_lines(m.aim.x as i32, m.aim.y as i32, tuning().missile_blast_radius, Color::ORANGE);
            }
            for (pos, vel, half) in boxes {
                let size = (half * 2.0).round().max(2.0) as i32;
                d.draw_rectangle_lines((pos.x - half).round() as i32, (pos.y - half).round() as i32, size, size, Color::MAGENTA);
                // A tenth of a second of travel.
                d.draw_line(
                    pos.x as i32,
                    pos.y as i32,
                    (pos.x + vel.x * 0.1) as i32,
                    (pos.y + vel.y * 0.1) as i32,
                    Color::new(255, 0, 255, 160),
                );
            }
        }
        if ov.ai || ov.engage {
            for (entity, tank, ai) in self.world.query::<(hecs::Entity, &Tank, &Ai)>().iter() {
                if tank.is_wreck() {
                    continue;
                }
                let (x, y) = (tank.position.x as i32, tank.position.y as i32);
                if ov.ai {
                    let s = ai.snapshot();
                    // (0, 0) is "never picked one": a tank straight into a
                    // fight has no patrol waypoint to show.
                    if s.waypoint_x != 0.0 || s.waypoint_y != 0.0 {
                        d.draw_line(x, y, s.waypoint_x as i32, s.waypoint_y as i32, Color::new(80, 200, 255, 140));
                        d.draw_circle_lines(s.waypoint_x as i32, s.waypoint_y as i32, 5.0, Color::new(80, 200, 255, 200));
                    }
                    if let Some(dir) = s.committed_dir.and_then(Dir::parse) {
                        let v = dir.vec();
                        let (ex, ey) = (tank.position.x + v.x * 40.0, tank.position.y + v.y * 40.0);
                        d.draw_line(x, y, ex as i32, ey as i32, Color::ORANGE);
                        d.draw_circle(ex as i32, ey as i32, 3.0, Color::ORANGE);
                    }
                    let label = format!(
                        "{} {}{}",
                        s.last_action.unwrap_or("-"),
                        s.committed_dir.unwrap_or("-"),
                        if s.stuck_timer > 0.0 { format!(" stuck {:.1}s", s.stuck_timer) } else { String::new() }
                    );
                    let ty = (tank.position.y + tank.hull_size() * 0.5 + 2.0) as i32;
                    d.draw_rectangle(x - 2, ty, label.len() as i32 * 7 + 4, 14, Color::new(0, 0, 0, 150));
                    d.draw_text(&label, x, ty + 1, 12, Color::new(80, 200, 255, 255));
                }
                if ov.engage && let Some(target) = self.last_engage.target(entity) {
                    let (tx, ty) = (target.x as i32, target.y as i32);
                    d.draw_line(x, y, tx, ty, Color::SKYBLUE);
                    d.draw_line(tx - 5, ty - 5, tx + 5, ty + 5, Color::SKYBLUE);
                    d.draw_line(tx - 5, ty + 5, tx + 5, ty - 5, Color::SKYBLUE);
                }
            }
        }
    }
}
