//! The presentation layer: reads `Game`'s state (owned by `simulation.rs`)
//! and draws it. Nothing here ever mutates simulation state - `render`
//! takes `&self` - so this module can freely depend on `RaylibHandle`
//! and friends without that dependency leaking back into `simulation.rs`.
//! See `simulation.rs`'s module doc comment for the other half of this split.

use crate::tuning::tuning;
use sola_raylib::prelude::*;

use crate::ai::Ai;
use crate::bullet::{Bullet, BulletState, draw_bullet, draw_bullet_shadow};
use crate::damage_stage::draw_damage;
use crate::frog::{Frog, FrogVariantTextures, draw_frog, draw_frog_ring};
use crate::laser::draw_laser_beam;
use crate::obstacle::{Obstacle, draw_obstacle, draw_obstacle_shadow};
use crate::pickup::{Pickup, PickupKind, draw_pickup};
use crate::plasma::{Plasma, PlasmaState, draw_plasma, draw_plasma_shadow};
use crate::shell::{Shell, ShellState, draw_shell, draw_shell_shadow};
use crate::shockwave::{RippleFx, screen_to_ripple_uv};
use crate::simulation::{Game, Outcome};
#[cfg(feature = "dev-tools")]
use crate::simulation::Overlays;
#[cfg(feature = "dev-tools")]
use crate::tank::Dir;
use crate::tank::{
    ActiveWeapon, Tank, draw_minigun_mount, draw_minigun_mount_shadow, draw_player_ring, draw_tank, draw_tank_shadow,
    draw_tank_shield,
};
use crate::track::draw_track;
use crate::{
    HEALTH_BAR_CELL_SIZE,
    HEALTH_BAR_COLUMNS,
    HEALTH_BAR_HUD_SCALE,
    HEALTH_BAR_ICON_OFFSET,
    HEALTH_BAR_ICON_SIZE,
    HEALTH_BAR_OVERHEAD_GAP,
    HEALTH_BAR_VARIANTS,
    HUD_FONT_SIZE,
    HUD_MARGIN,
    HUD_VERSION_FONT_SIZE,
    MAX_DAMAGE,
};

/// Accent colors for the HUD's special-weapon ammo counts - one per
/// `ActiveWeapon` special, shared between each weapon's count number
/// (always) and its label (only while that weapon is the live one - see
/// the HUD block in `render`). Presentation-only, so they live here rather
/// than in `lib.rs`'s gameplay tuning.
const HUD_LASER_COLOR: Color = Color::new(255, 60, 160, 255);
const HUD_PLASMA_COLOR: Color = Color::new(60, 220, 200, 255);
const HUD_MINIGUN_COLOR: Color = Color::new(190, 205, 215, 255);

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
    pub pickup_speedup: &'a Texture2D,
    pub pickup_shield: &'a Texture2D,
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
        let (hp, shells, laser_charges, plasma_ammo, minigun_ammo, active_weapon) =
            crate::simulation::with_tank(&self.world, player, |t| {
                (
                    (MAX_DAMAGE - t.damage).max(0.0).round() as i32,
                    t.shells_ammo,
                    t.laser_charges,
                    t.plasma_ammo,
                    t.minigun_ammo,
                    t.active_weapon(),
                )
            });
        let shells_color = hud_number_color(shells as f32, tuning().max_shells as f32);
        let hp_color = hud_number_color(hp as f32, MAX_DAMAGE);
        // The live weapon's label carries a ">" marker (and, for a special,
        // its own accent color instead of neutral white) - with the FIFO
        // weapon queue (see `Tank::weapon_queue`), several stocked weapons
        // can show at once and the counts alone no longer say which one
        // the trigger actually fires.
        let hud_shells_label = if active_weapon == ActiveWeapon::Shell {
            ">SHELLS: "
        } else {
            "SHELLS: "
        };
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
            let active = active_weapon == ActiveWeapon::Laser;
            let label = if active { "   >LASER: " } else { "   LASER: " };
            let num = format!("{laser_charges}");
            let label_w = rl.measure_text(label, HUD_FONT_SIZE);
            let num_w = rl.measure_text(&num, HUD_FONT_SIZE);
            let label_color = if active { HUD_LASER_COLOR } else { Color::WHITE };
            (label, num, label_w, num_w, label_color)
        });
        // Same idea as hud_laser above, for plasma ammo.
        let hud_plasma = (plasma_ammo > 0).then(|| {
            let active = active_weapon == ActiveWeapon::Plasma;
            let label = if active { "   >PLASMA: " } else { "   PLASMA: " };
            let num = format!("{plasma_ammo}");
            let label_w = rl.measure_text(label, HUD_FONT_SIZE);
            let num_w = rl.measure_text(&num, HUD_FONT_SIZE);
            let label_color = if active { HUD_PLASMA_COLOR } else { Color::WHITE };
            (label, num, label_w, num_w, label_color)
        });
        // Same idea as hud_laser above, for minigun ammo.
        let hud_minigun = (minigun_ammo > 0).then(|| {
            let active = active_weapon == ActiveWeapon::Minigun;
            let label = if active { "   >MINIGUN: " } else { "   MINIGUN: " };
            let num = format!("{minigun_ammo}");
            let label_w = rl.measure_text(label, HUD_FONT_SIZE);
            let num_w = rl.measure_text(&num, HUD_FONT_SIZE);
            let label_color = if active { HUD_MINIGUN_COLOR } else { Color::WHITE };
            (label, num, label_w, num_w, label_color)
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
                    PickupKind::SpeedUp => textures.pickup_speedup,
                    PickupKind::Shield => textures.pickup_shield,
                };
                draw_pickup(&mut d, texture, pickup);
            }

            for frog_entity in [self.frog, self.enemy_frog].into_iter().flatten() {
                crate::simulation::with_frog(&self.world, frog_entity, |frog| {
                    let variant = &textures.frog_variants[frog.variant as usize];
                    draw_frog_ring(&mut d, frog, self.time);
                    draw_frog(&mut d, &variant.as_frog_textures(), frog, self.time);
                    draw_frog_health_bar(&mut d, textures.health_bar, frog);
                });
            }

            for tank in self.world.query::<&Tank>().with::<&Ai>().iter() {
                draw_tank_shield(&mut d, tank, self.time);
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
                draw_player_ring(&mut d, tank, self.time);
                draw_tank_shield(&mut d, tank, self.time);
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
            let decay = (1.0 - shock.time / tuning().camera_shake_duration).max(0.0);
            if decay > 0.0 {
                let t = shock.time * tuning().camera_shake_frequency;
                blit_offset = Vector2::new(
                    t.sin() * tuning().camera_shake_magnitude * decay,
                    (t * 1.3 + 1.7).sin() * tuning().camera_shake_magnitude * decay,
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

            // Debug overlays (dev builds only): the inspect layer's
            // hitbox/collider outlines plus a stat readout for every tank,
            // then the dev server's other layers. Drawn here (screen space,
            // post-composite) rather than into scene_target, so they're
            // never warped by an in-flight shockwave and always render
            // crisp - tank.position is already screen pixels (no camera
            // transform), so the two spaces line up 1:1 with no extra math.
            #[cfg(feature = "dev-tools")]
            {
                if self.debug_overlays.inspect {
                    for (tank, ai) in self.world.query::<(&Tank, &Ai)>().iter() {
                        draw_tank_inspect(&mut d, tank, Some(ai));
                    }
                    crate::simulation::with_tank(&self.world, player, |tank| {
                        draw_tank_inspect(&mut d, tank, None);
                    });
                }
                self.draw_debug_overlays(&mut d, screen_width as f32, screen_height as f32);
                // Which preset is live, one line under the top-left HUD row,
                // so the I key's cycling is visible without counting layers.
                if self.debug_overlays.any() {
                    let preset = if self.debug_overlays == Overlays::INSPECT {
                        "inspect"
                    } else if self.debug_overlays == Overlays::ALL {
                        "all"
                    } else {
                        "custom"
                    };
                    let label = format!("DEV overlays: {preset} (I cycles)");
                    const LABEL_FONT_SIZE: i32 = if HUD_FONT_SIZE / 2 < 14 { HUD_FONT_SIZE / 2 } else { 14 };
                    let label_y = HUD_MARGIN + HUD_FONT_SIZE + 6;
                    // Same 8px/char width estimate as `draw_tank_inspect`'s
                    // stat panel - no font handle inside the draw closure.
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
            if let Some((label, num, label_w, num_w, label_color)) = &hud_laser {
                d.draw_text(label, hud_x, hud_y, HUD_FONT_SIZE, *label_color);
                hud_x += label_w;
                d.draw_text(num, hud_x, hud_y, HUD_FONT_SIZE, HUD_LASER_COLOR);
                hud_x += num_w;
            }
            if let Some((label, num, label_w, num_w, label_color)) = &hud_plasma {
                d.draw_text(label, hud_x, hud_y, HUD_FONT_SIZE, *label_color);
                hud_x += label_w;
                d.draw_text(num, hud_x, hud_y, HUD_FONT_SIZE, HUD_PLASMA_COLOR);
                hud_x += num_w;
            }
            if let Some((label, num, label_w, num_w, label_color)) = &hud_minigun {
                d.draw_text(label, hud_x, hud_y, HUD_FONT_SIZE, *label_color);
                hud_x += label_w;
                d.draw_text(num, hud_x, hud_y, HUD_FONT_SIZE, HUD_MINIGUN_COLOR);
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

            // Mission banner: big white text over a dim overlay that both
            // fade together once the round unfreezes.
            if let Some((text, size, w, alpha)) = intro {
                let a = |max: f32| (max * alpha) as u8;
                d.draw_rectangle(0, 0, screen_width, screen_height, Color::new(0, 0, 0, a(120.0)));
                d.draw_text(text, screen_width / 2 - w / 2, screen_height / 2 - size / 2, size, Color::new(255, 255, 255, a(255.0)));
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

/// The inspect overlay for one tank (dev builds only - `Overlays::inspect`,
/// part of the presets the I key cycles): its hull damage box, its
/// turret+barrel damage box, and its (smaller, corner-rounded) movement
/// collider, plus a small stat block - ammo, health, current speed and velocity for every
/// tank, and additionally (`ai: Some`, i.e. this isn't the player) whether
/// it's currently retreating to recharge and its fire cooldown, pulled
/// straight from its `Ai` - the same state `ai.rs`'s
/// `wants_retreat`/`fire_interval` act on. Purely diagnostic: reads state,
/// draws it, mutates nothing.
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
///   walls. The stat block's MOVE line prints its current world-px size
///   and corner radius for the same purpose.
#[cfg(feature = "dev-tools")]
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

    // Movement collider (see the doc comment above): drawn with the exact
    // clamped corner radius the physics shape carries
    // (`physics::tank_corner_radius`), mapped onto raylib's relative
    // roundness factor (corner radius = roundness * min(w, h) / 2, so the
    // division below inverts that).
    let (mx, my) = tank.move_half_extents(along_x);
    let corner = crate::physics::tank_corner_radius((mx, my));
    let move_rect = Rectangle::new(
        tank.position.x - mx,
        tank.position.y - my,
        mx * 2.0,
        my * 2.0,
    );
    d.draw_rectangle_rounded_lines(move_rect, corner / mx.min(my), 8, Color::SKYBLUE);

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
    /// The debug overlay layers beyond inspect (`Game::debug_overlays`,
    /// docs/dev-server-design.md; dev builds only), screen space and
    /// post-composite like the inspect block: blocked nav cells, each
    /// enemy's AI memory, projectile hit boxes, engagement targets, pickup
    /// collect radii. Each layer costs nothing while off.
    fn draw_debug_overlays(&self, d: &mut impl RaylibDraw, width: f32, height: f32) {
        let ov = self.debug_overlays;
        if ov.nav_grid {
            let grid = self.nav_grid(width, height);
            let (cols, rows, cell) = grid.dims();
            let size = cell.round() as i32;
            for row in 0..rows {
                for col in 0..cols {
                    if grid.is_blocked(col, row) {
                        let x = (col as f32 * cell).round() as i32;
                        let y = (row as f32 * cell).round() as i32;
                        d.draw_rectangle(x, y, size, size, Color::new(255, 40, 40, 60));
                        d.draw_rectangle_lines(x, y, size, size, Color::new(255, 40, 40, 110));
                    }
                }
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

/// Color for a HUD number (SHELLS or HP) given its current value and max -
/// default gray above HUD_WARN_THRESHOLD, orange between the warn and
/// critical thresholds, red below. Shared by both since they're the same
/// current/max shape, just different units.
fn hud_number_color(current: f32, max: f32) -> Color {
    let frac = if max > 0.0 { current / max } else { 0.0 };
    if frac < tuning().hud_critical_threshold {
        Color::RED
    } else if frac < tuning().hud_warn_threshold {
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
    let alpha = if tank.hit_flash_timer > tuning().health_bar_overhead_fade_seconds {
        255
    } else {
        (255.0 * (tank.hit_flash_timer / tuning().health_bar_overhead_fade_seconds)).round() as u8
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
    let alpha = if frog.hit_flash_timer > tuning().health_bar_overhead_fade_seconds {
        255
    } else {
        (255.0 * (frog.hit_flash_timer / tuning().health_bar_overhead_fade_seconds)).round() as u8
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
