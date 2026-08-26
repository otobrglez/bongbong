//! The presentation layer: reads `Game`'s state (owned by `simulation.rs`)
//! and draws it. Nothing here ever mutates simulation state - `render`
//! takes `&self` - so this module can freely depend on `RaylibHandle`
//! and friends without that dependency leaking back into `simulation.rs`.
//! See `simulation.rs`'s module doc comment for the other half of this split.

use sola_raylib::prelude::*;

use crate::ai::Ai;
use crate::bullet::{Bullet, BulletState, draw_bullet, draw_bullet_shadow};
use crate::damage_stage::draw_damage;
use crate::frog::{Frog, FrogVariantTextures, draw_frog};
use crate::laser::draw_laser_beam;
use crate::obstacle::{Obstacle, draw_obstacle, draw_obstacle_shadow};
use crate::pickup::{Pickup, PickupKind, draw_pickup};
use crate::plasma::{Plasma, PlasmaState, draw_plasma, draw_plasma_shadow};
use crate::shell::{Shell, ShellState, draw_shell, draw_shell_shadow};
use crate::shockwave::{RippleFx, screen_to_ripple_uv};
use crate::simulation::{Game, Outcome};
use crate::tank::{Dir, Tank, draw_minigun_mount, draw_minigun_mount_shadow, draw_tank, draw_tank_shadow};
use crate::track::draw_track;
use crate::{
    CAMERA_SHAKE_DURATION, CAMERA_SHAKE_FREQUENCY, CAMERA_SHAKE_MAGNITUDE, HEALTH_BAR_CELL_SIZE,
    HEALTH_BAR_COLUMNS, HEALTH_BAR_HUD_SCALE, HEALTH_BAR_ICON_OFFSET,
    HEALTH_BAR_ICON_SIZE, HEALTH_BAR_OVERHEAD_FADE_SECONDS, HEALTH_BAR_OVERHEAD_GAP,
    HEALTH_BAR_VARIANTS, HUD_CRITICAL_THRESHOLD, HUD_FONT_SIZE, HUD_MARGIN, HUD_VERSION_FONT_SIZE,
    HUD_WARN_THRESHOLD, IMPACT_FLASH_QUAD_RADIUS, MAX_DAMAGE, MAX_SHELLS,
    MUZZLE_FLASH_QUAD_RADIUS,
};

/// The sprite atlases `Game::render` draws from, bundled into one param instead
/// of four so the signature doesn't grow with every new texture.
pub struct Textures<'a> {
    pub tanks: &'a Texture2D,
    pub shells: &'a Texture2D,
    pub plasma: &'a Texture2D,
    pub minigun_bullets: &'a Texture2D,
    pub damage: &'a Texture2D,
    pub tracks: &'a Texture2D,
    pub obstacles: &'a Texture2D,
    pub ground: &'a Texture2D,
    pub health_bar: &'a Texture2D,
    /// One `FrogVariantTextures` per `frog::FROG_VARIANT_DIRS` entry, in the
    /// same order - `render` indexes into this by `Frog::variant`.
    pub frog_variants: &'a [FrogVariantTextures],
    pub pickup_health: &'a Texture2D,
    pub pickup_ammo: &'a Texture2D,
    pub pickup_laser: &'a Texture2D,
    pub pickup_minigun: &'a Texture2D,
    pub pickup_plasma: &'a Texture2D,
    /// The minigun barrel-cluster overlay drawn on a tank's turret while it
    /// holds minigun ammo - see `tank::draw_minigun_mount`. One shared
    /// texture for every chassis (unlike `tanks` above), not a sheet.
    pub minigun_mount: &'a Texture2D,
}

/// The ripple post-effects `Game::render` drives, bundled into one param for the
/// same reason as `Textures`. See `shockwave.rs` for what each one does.
pub struct Effects<'a> {
    pub shock: &'a mut RippleFx,
    pub muzzle: &'a mut RippleFx,
    pub impact: &'a mut RippleFx,
}

impl Game {
    /// Draw the whole scene for this frame.
    pub fn render(
        &self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        scene_target: &mut RenderTexture2D,
        effects: &mut Effects,
        textures: &Textures,
    ) {
        let screen_width = rl.get_screen_width();
        let screen_height = rl.get_screen_height();
        let player = self.player.expect("player entity spawned in init");

        // Precompute the bottom-right version/build HUD (text width must be
        // measured on the RaylibHandle, outside the draw closure).
        let version_hud = format!("v{} {}", env!("CARGO_PKG_VERSION"), "@otobrglez");
        let version_hud_w = rl.measure_text(&version_hud, HUD_VERSION_FONT_SIZE);

        // Precompute the SHELLS/HP HUD as separate runs so each number can
        // carry its own color (hud_number_color) while the labels stay
        // neutral - raylib's draw_text is single-color per call, so the line
        // is drawn as four adjacent segments rather than one string. Widths
        // measured here for the same reason as version_hud_w above.
        let (hp, shells, laser_charges, plasma_ammo, minigun_ammo) =
            crate::simulation::with_tank(&self.world, player, |t| {
                (
                    (MAX_DAMAGE - t.damage).max(0.0).round() as i32,
                    t.shells_ammo,
                    t.laser_charges,
                    t.plasma_ammo,
                    t.minigun_ammo,
                )
            });
        let shells_color = hud_number_color(shells as f32, MAX_SHELLS as f32);
        let hp_color = hud_number_color(hp as f32, MAX_DAMAGE);
        let hud_shells_label = "SHELLS: ";
        let hud_shells_num = format!("{shells}");
        let hud_mid = "   HP: ";
        let hud_hp_num = format!("{hp}");
        let hud_shells_label_w = rl.measure_text(hud_shells_label, HUD_FONT_SIZE);
        let hud_shells_num_w = rl.measure_text(&hud_shells_num, HUD_FONT_SIZE);
        let hud_mid_w = rl.measure_text(hud_mid, HUD_FONT_SIZE);
        let hud_hp_num_w = rl.measure_text(&hud_hp_num, HUD_FONT_SIZE);
        // Only shown while charged (see Tank::laser_charges) - most rounds
        // never pick one up, so this stays out of the way otherwise.
        let hud_laser = (laser_charges > 0).then(|| {
            let label = "   LASER: ";
            let num = format!("{laser_charges}");
            let label_w = rl.measure_text(label, HUD_FONT_SIZE);
            let num_w = rl.measure_text(&num, HUD_FONT_SIZE);
            (label, num, label_w, num_w)
        });
        // Same idea as hud_laser above, for plasma ammo.
        let hud_plasma = (plasma_ammo > 0).then(|| {
            let label = "   PLASMA: ";
            let num = format!("{plasma_ammo}");
            let label_w = rl.measure_text(label, HUD_FONT_SIZE);
            let num_w = rl.measure_text(&num, HUD_FONT_SIZE);
            (label, num, label_w, num_w)
        });
        // Same idea as hud_laser above, for minigun ammo.
        let hud_minigun = (minigun_ammo > 0).then(|| {
            let label = "   MINIGUN: ";
            let num = format!("{minigun_ammo}");
            let label_w = rl.measure_text(label, HUD_FONT_SIZE);
            let num_w = rl.measure_text(&num, HUD_FONT_SIZE);
            (label, num, label_w, num_w)
        });

        // health_bar.png source rect for the player's current HP fraction -
        // see health_bar_frame's own doc comment for the frame thresholds.
        let health_bar_source = health_bar_source_rect(health_bar_frame(hp as f32 / MAX_DAMAGE));

        // Precompute the centered end-of-round banner (text width must be
        // measured on the RaylibHandle, outside the draw closure).
        let banner = match self.outcome {
            Outcome::Playing => None,
            Outcome::Won => Some(("YOU WIN", Color::DARKGREEN)),
            Outcome::Lost => Some(("YOU LOSE", Color::MAROON)),
        };
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

        // Pass 1: draw the world (tracks, tanks, shells) into an offscreen
        // render texture, so a shockwave can distort the finished frame as a
        // whole-screen shader pass in pass 2.
        rl.draw_texture_mode(thread, scene_target, |mut d| {
            d.clear_background(Color::WHITE);

            // Ground first - the floor everything else sits on. See
            // ground.rs / docs/GROUND_SPEC.md.
            crate::ground::draw(&mut d, textures.ground, &self.ground);

            // Tread marks go down first so tanks and everything else draw on top.
            for track in &self.tracks {
                draw_track(&mut d, textures.tracks, track);
            }

            for obstacle in self.world.query::<&Obstacle>().iter() {
                if self.shadows_enabled {
                    draw_obstacle_shadow(&mut d, textures.obstacles, obstacle);
                }
                draw_obstacle(&mut d, textures.obstacles, obstacle);
            }

            for pickup in self.world.query::<&Pickup>().iter() {
                let texture = match pickup.kind {
                    PickupKind::Health => textures.pickup_health,
                    PickupKind::Ammo => textures.pickup_ammo,
                    PickupKind::Laser => textures.pickup_laser,
                    PickupKind::Minigun => textures.pickup_minigun,
                    PickupKind::Plasma => textures.pickup_plasma,
                };
                draw_pickup(&mut d, texture, pickup);
            }

            crate::simulation::with_frog(
                &self.world,
                self.frog.expect("frog entity spawned in init"),
                |frog| {
                    let variant = &textures.frog_variants[frog.variant as usize];
                    draw_frog(&mut d, &variant.as_frog_textures(), frog, self.time);
                    draw_frog_health_bar(&mut d, textures.health_bar, frog);
                },
            );

            for tank in self.world.query::<&Tank>().with::<&Ai>().iter() {
                if self.shadows_enabled {
                    draw_tank_shadow(&mut d, textures.tanks, tank);
                    draw_minigun_mount_shadow(&mut d, textures.minigun_mount, tank);
                }
                draw_tank(&mut d, textures.tanks, tank);
                draw_minigun_mount(&mut d, textures.minigun_mount, tank);
                draw_damage(&mut d, textures.damage, tank, self.time);
                draw_tank_overhead_health(&mut d, textures.health_bar, tank);
            }

            crate::simulation::with_tank(&self.world, player, |tank| {
                if self.shadows_enabled {
                    draw_tank_shadow(&mut d, textures.tanks, tank);
                    draw_minigun_mount_shadow(&mut d, textures.minigun_mount, tank);
                }
                draw_tank(&mut d, textures.tanks, tank);
                draw_minigun_mount(&mut d, textures.minigun_mount, tank);
                draw_damage(&mut d, textures.damage, tank, self.time);
                draw_tank_overhead_health(&mut d, textures.health_bar, tank);
            });

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
        });

        // If a shockwave is playing, push its current center/time to the shader
        // before the blit below samples through it.
        if let Some(shock) = &self.shock {
            let uv = screen_to_ripple_uv(shock.center, screen_width as f32, screen_height as f32);
            effects
                .shock
                .shader
                .set_shader_value(effects.shock.center_loc, uv);
            effects
                .shock
                .shader
                .set_shader_value(effects.shock.time_loc, shock.time);
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
        let mut blit_offset = Vector2::new(0.0, 0.0);
        if let Some(shock) = &self.shock {
            let decay = (1.0 - shock.time / CAMERA_SHAKE_DURATION).max(0.0);
            if decay > 0.0 {
                let t = shock.time * CAMERA_SHAKE_FREQUENCY;
                blit_offset = Vector2::new(
                    t.sin() * CAMERA_SHAKE_MAGNITUDE * decay,
                    (t * 1.3 + 1.7).sin() * CAMERA_SHAKE_MAGNITUDE * decay,
                );
            }
        }

        rl.draw(thread, |mut d| {
            d.clear_background(Color::BLACK);

            if self.shock.is_some() {
                d.draw_shader_mode(&mut effects.shock.shader, |mut sd| {
                    sd.draw_texture_rec(&*scene_target, source, blit_offset, Color::WHITE);
                });
            } else {
                d.draw_texture_rec(&*scene_target, source, blit_offset, Color::WHITE);
            }

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

                let r = MUZZLE_FLASH_QUAD_RADIUS;
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

                let r = IMPACT_FLASH_QUAD_RADIUS;
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

            // Debug inspect overlay: a bounding square plus a stat readout for
            // every tank. Drawn here (screen space, post-composite) rather
            // than into scene_target, so it's never warped by an in-flight
            // shockwave and always renders crisp - tank.position is already
            // screen pixels (no camera transform), so the two spaces line up
            // 1:1 with no extra math.
            if self.inspect_enabled {
                for (tank, ai) in self.world.query::<(&Tank, &Ai)>().iter() {
                    draw_tank_inspect(&mut d, tank, Some(ai));
                }
                crate::simulation::with_tank(&self.world, player, |tank| {
                    draw_tank_inspect(&mut d, tank, None);
                });
            }

            // HUD and the end-of-round banner draw undistorted, on top of the
            // (possibly rippling) scene.
            let hud_y = HUD_MARGIN;
            let mut hud_x = HUD_MARGIN;
            d.draw_text(hud_shells_label, hud_x, hud_y, HUD_FONT_SIZE, Color::WHITE);
            hud_x += hud_shells_label_w;
            d.draw_text(&hud_shells_num, hud_x, hud_y, HUD_FONT_SIZE, shells_color);
            hud_x += hud_shells_num_w;
            d.draw_text(hud_mid, hud_x, hud_y, HUD_FONT_SIZE, Color::WHITE);
            hud_x += hud_mid_w;
            d.draw_text(&hud_hp_num, hud_x, hud_y, HUD_FONT_SIZE, hp_color);
            hud_x += hud_hp_num_w;
            if let Some((label, num, label_w, num_w)) = &hud_laser {
                d.draw_text(label, hud_x, hud_y, HUD_FONT_SIZE, Color::WHITE);
                hud_x += label_w;
                d.draw_text(num, hud_x, hud_y, HUD_FONT_SIZE, Color::new(255, 60, 160, 255));
                hud_x += num_w;
            }
            if let Some((label, num, label_w, num_w)) = &hud_plasma {
                d.draw_text(label, hud_x, hud_y, HUD_FONT_SIZE, Color::WHITE);
                hud_x += label_w;
                d.draw_text(num, hud_x, hud_y, HUD_FONT_SIZE, Color::new(60, 220, 200, 255));
                hud_x += num_w;
            }
            if let Some((label, num, label_w, num_w)) = &hud_minigun {
                d.draw_text(label, hud_x, hud_y, HUD_FONT_SIZE, Color::WHITE);
                hud_x += label_w;
                d.draw_text(num, hud_x, hud_y, HUD_FONT_SIZE, Color::new(190, 205, 215, 255));
                hud_x += num_w;
            }
            hud_x += 12;
            let health_bar_dest_h = HEALTH_BAR_ICON_SIZE.1 * HEALTH_BAR_HUD_SCALE;
            d.draw_texture_pro(
                textures.health_bar,
                health_bar_source,
                Rectangle::new(
                    hud_x as f32,
                    hud_y as f32 + (HUD_FONT_SIZE as f32 - health_bar_dest_h) / 2.0,
                    HEALTH_BAR_ICON_SIZE.0 * HEALTH_BAR_HUD_SCALE,
                    health_bar_dest_h,
                ),
                Vector2::zero(),
                0.0,
                Color::WHITE,
            );
            // Mirrors the top-left HUD's HUD_MARGIN inset, so both corners
            // sit the same distance from their edges.
            d.draw_text(
                &version_hud,
                screen_width - HUD_MARGIN - version_hud_w,
                screen_height - HUD_MARGIN - HUD_VERSION_FONT_SIZE,
                HUD_VERSION_FONT_SIZE,
                Color::WHITE,
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

            // Paused overlay draws over everything else, including the
            // end-of-round banner (its countdown is frozen too).
            if self.paused {
                d.draw_rectangle(0, 0, screen_width, screen_height, Color::new(0, 0, 0, 120));
                let title = "PAUSED";
                let title_size = 72;
                let title_w = d.measure_text(title, title_size);
                d.draw_text(
                    title,
                    screen_width / 2 - title_w / 2,
                    screen_height / 2 - title_size / 2,
                    title_size,
                    Color::RAYWHITE,
                );
            }
        });
    }
}

/// Debug inspect-mode overlay for one tank: its actual hull collider box
/// plus a second, visually distinct turret+barrel box (see
/// `Game::inspect_enabled`, toggled by the "I" key), plus a small stat
/// block - ammo, health, current speed and velocity for every tank, and
/// additionally (`ai: Some`, i.e. this isn't the player) whether it's
/// currently retreating to recharge and its fire cooldown, pulled straight
/// from its `Ai` - the same state `ai.rs`'s `wants_retreat`/`fire_interval`
/// act on. Purely diagnostic: reads state, draws it, mutates nothing.
///
/// The first (lime/orange/gray) box is `Tank::hull_half_extents` - the same
/// per-row (TANK_HULL_BBOX_BY_ROW), facing-oriented rectangle
/// `simulation::drive_tank`/`Physics::resize_collider` actually gives this
/// tank's movement collider, and (via `Tank::hit_sensor`) the shell-hit
/// sensor covering the hull - not the full 32x32 sprite tile
/// (`Tank::size`/`Tank::hull_size`), which is a uniform square padded well
/// past the visible hull art on every row. The second (yellow) box is
/// `Tank::turret_bbox_world` - the turret+barrel silhouette, backing
/// `Tank::turret_hit_sensor` the same way. Together these two boxes are
/// exactly what a shell can hit; this overlay draws the real hit-testing
/// shape, not an approximation of it.
fn draw_tank_inspect(d: &mut impl RaylibDraw, tank: &Tank, ai: Option<&Ai>) {
    // Same "which axis is the long one" check as `Tank::avoidance_radius` -
    // tanks only ever face one of the four `Dir::rotation()` values, so an
    // exact match is safe here (no epsilon needed).
    let along_x = tank.rotation == Dir::Right.rotation() || tank.rotation == Dir::Left.rotation();
    let (hx, hy) = tank.hull_half_extents(along_x);
    let x = (tank.position.x - hx).round() as i32;
    let y = (tank.position.y - hy).round() as i32;
    let width = (hx * 2.0).round() as i32;
    let height = (hy * 2.0).round() as i32;

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

    let speed = (tank.velocity.x * tank.velocity.x + tank.velocity.y * tank.velocity.y).sqrt();
    let mut lines = vec![
        format!("AMMO {}", tank.shells_ammo),
        format!(
            "HP {}/{}",
            (MAX_DAMAGE - tank.damage).max(0.0).round() as i32,
            MAX_DAMAGE as i32
        ),
        format!("SPD {speed:.0}px/s"),
        format!("VEL ({:.0},{:.0})", tank.velocity.x, tank.velocity.y),
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

/// Color for a HUD number (SHELLS or HP) given its current value and max -
/// default gray above HUD_WARN_THRESHOLD, orange between the warn and
/// critical thresholds, red below. Shared by both since they're the same
/// current/max shape, just different units.
fn hud_number_color(current: f32, max: f32) -> Color {
    let frac = if max > 0.0 { current / max } else { 0.0 };
    if frac < HUD_CRITICAL_THRESHOLD {
        Color::RED
    } else if frac < HUD_WARN_THRESHOLD {
        Color::ORANGE
    } else {
        Color::WHITE
    }
}

/// Which of health_bar.png's HEALTH_BAR_VARIANTS frames (0 = full 4/4 pips,
/// HEALTH_BAR_VARIANTS-1 = empty 0/4) represents an HP fraction. Quarter
/// thresholds so each frame's pip count matches its fraction range exactly
/// (frame 1 = "3/4 pips" covers the range where 3/4 is the closest reading);
/// only true 0 HP shows fully empty, matching how HUD_CRITICAL_THRESHOLD
/// etc. only flag real trouble rather than every routine dip.
fn health_bar_frame(frac: f32) -> i32 {
    if frac > 0.75 {
        0
    } else if frac > 0.50 {
        1
    } else if frac > 0.25 {
        2
    } else if frac > 0.0 {
        3
    } else {
        4
    }
    .min(HEALTH_BAR_VARIANTS - 1)
}

/// Source rect in health_bar.png for a given frame index (see
/// health_bar_frame) - the sheet is HEALTH_BAR_COLUMNS-wide, HEALTH_BAR_CELL_SIZE
/// per cell, with the actual icon glyph living at a fixed sub-rect
/// (HEALTH_BAR_ICON_OFFSET/HEALTH_BAR_ICON_SIZE) inside each cell.
fn health_bar_source_rect(frame: i32) -> Rectangle {
    let col = frame % HEALTH_BAR_COLUMNS;
    let row = frame / HEALTH_BAR_COLUMNS;
    Rectangle::new(
        col as f32 * HEALTH_BAR_CELL_SIZE + HEALTH_BAR_ICON_OFFSET.0,
        row as f32 * HEALTH_BAR_CELL_SIZE + HEALTH_BAR_ICON_OFFSET.1,
        HEALTH_BAR_ICON_SIZE.0,
        HEALTH_BAR_ICON_SIZE.1,
    )
}

/// Draw a tank's overhead health bar - only while `hit_flash_timer` is
/// running (see `Tank::mark_hit`) and the tank isn't a wreck (past caring
/// about its own HP). Centered under the tank's sprite, same pixel scale as
/// the HUD copy (HEALTH_BAR_HUD_SCALE), fading out over the trailing
/// HEALTH_BAR_OVERHEAD_FADE_SECONDS of its window instead of popping off.
fn draw_tank_overhead_health(d: &mut impl RaylibDraw, texture: &Texture2D, tank: &Tank) {
    if tank.is_wreck() || tank.hit_flash_timer <= 0.0 {
        return;
    }
    let frac = ((MAX_DAMAGE - tank.damage) / MAX_DAMAGE).clamp(0.0, 1.0);
    let source = health_bar_source_rect(health_bar_frame(frac));
    let w = HEALTH_BAR_ICON_SIZE.0 * HEALTH_BAR_HUD_SCALE;
    let h = HEALTH_BAR_ICON_SIZE.1 * HEALTH_BAR_HUD_SCALE;
    let dest = Rectangle::new(
        tank.position.x - w / 2.0,
        tank.position.y + tank.size() * 0.5 + HEALTH_BAR_OVERHEAD_GAP,
        w,
        h,
    );
    let alpha = if tank.hit_flash_timer > HEALTH_BAR_OVERHEAD_FADE_SECONDS {
        255
    } else {
        (255.0 * (tank.hit_flash_timer / HEALTH_BAR_OVERHEAD_FADE_SECONDS)).round() as u8
    };
    d.draw_texture_pro(
        texture,
        source,
        dest,
        Vector2::zero(),
        0.0,
        Color::new(255, 255, 255, alpha),
    );
}

/// Draw the frog's overhead health bar - same "only while `hit_flash_timer`
/// is running, fading out over the trailing HEALTH_BAR_OVERHEAD_FADE_SECONDS"
/// convention as `draw_tank_overhead_health`, just keyed off `Frog::is_dead`
/// instead of `Tank::is_wreck`.
fn draw_frog_health_bar(d: &mut impl RaylibDraw, texture: &Texture2D, frog: &Frog) {
    if frog.is_dead() || frog.hit_flash_timer <= 0.0 {
        return;
    }
    let frac = (frog.health / frog.max_health).clamp(0.0, 1.0);
    let source = health_bar_source_rect(health_bar_frame(frac));
    let w = HEALTH_BAR_ICON_SIZE.0 * HEALTH_BAR_HUD_SCALE;
    let h = HEALTH_BAR_ICON_SIZE.1 * HEALTH_BAR_HUD_SCALE;
    let dest = Rectangle::new(
        frog.position.x - w / 2.0,
        frog.position.y + frog.size() * 0.5 + HEALTH_BAR_OVERHEAD_GAP,
        w,
        h,
    );
    let alpha = if frog.hit_flash_timer > HEALTH_BAR_OVERHEAD_FADE_SECONDS {
        255
    } else {
        (255.0 * (frog.hit_flash_timer / HEALTH_BAR_OVERHEAD_FADE_SECONDS)).round() as u8
    };
    d.draw_texture_pro(
        texture,
        source,
        dest,
        Vector2::zero(),
        0.0,
        Color::new(255, 255, 255, alpha),
    );
}
