//! The presentation layer: reads `Game`'s state (owned by `simulation.rs`)
//! and draws it. Nothing here ever mutates simulation state - `render`
//! takes `&self` - so this module can freely depend on `RaylibHandle`
//! and friends without that dependency leaking back into `simulation.rs`.
//! See `simulation.rs`'s module doc comment for the other half of this split.

use sola_raylib::prelude::*;

use crate::ai::Ai;
use crate::damage_stage::draw_damage;
use crate::obstacle::{Obstacle, draw_obstacle, draw_obstacle_shadow};
use crate::shell::{Shell, ShellState, draw_shell, draw_shell_shadow};
use crate::shockwave::{RippleFx, screen_to_ripple_uv};
use crate::simulation::{Game, Outcome};
use crate::tank::{Tank, draw_tank, draw_tank_shadow};
use crate::track::draw_track;
use crate::{
    HUD_CRITICAL_THRESHOLD, HUD_WARN_THRESHOLD, IMPACT_FLASH_QUAD_RADIUS, MAX_DAMAGE, MAX_SHELLS,
    MUZZLE_FLASH_QUAD_RADIUS,
};

/// The sprite atlases `Game::render` draws from, bundled into one param instead
/// of four so the signature doesn't grow with every new texture.
pub struct Textures<'a> {
    pub tanks: &'a Texture2D,
    pub shells: &'a Texture2D,
    pub damage: &'a Texture2D,
    pub tracks: &'a Texture2D,
    pub obstacles: &'a Texture2D,
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
        let version_hud = format!(
            "v{} ({}) {}",
            env!("CARGO_PKG_VERSION"),
            env!("BONGBONG_GIT_COMMIT"),
            "@otobrglez"
        );
        let version_hud_w = rl.measure_text(&version_hud, 18);

        // Precompute the SHELLS/HP HUD as separate runs so each number can
        // carry its own color (hud_number_color) while the labels stay
        // neutral - raylib's draw_text is single-color per call, so the line
        // is drawn as four adjacent segments rather than one string. Widths
        // measured here for the same reason as version_hud_w above.
        let (hp, shells) = crate::simulation::with_tank(&self.world, player, |t| {
            (
                (MAX_DAMAGE - t.damage).max(0.0).round() as i32,
                t.shells_ammo,
            )
        });
        let shells_color = hud_number_color(shells as f32, MAX_SHELLS as f32);
        let hp_color = hud_number_color(hp as f32, MAX_DAMAGE);
        let hud_shells_label = "SHELLS: ";
        let hud_shells_num = format!("{shells}");
        let hud_mid = "   HP: ";
        let hud_hp_num = format!("{hp}");
        let hud_shells_label_w = rl.measure_text(hud_shells_label, 24);
        let hud_shells_num_w = rl.measure_text(&hud_shells_num, 24);
        let hud_mid_w = rl.measure_text(hud_mid, 24);

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
            d.clear_background(Color::RAYWHITE);

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

            for tank in self.world.query::<&Tank>().with::<&Ai>().iter() {
                if self.shadows_enabled {
                    draw_tank_shadow(&mut d, textures.tanks, tank);
                }
                draw_tank(&mut d, textures.tanks, tank);
                draw_damage(&mut d, textures.damage, tank, self.time);
            }

            crate::simulation::with_tank(&self.world, player, |tank| {
                if self.shadows_enabled {
                    draw_tank_shadow(&mut d, textures.tanks, tank);
                }
                draw_tank(&mut d, textures.tanks, tank);
                draw_damage(&mut d, textures.damage, tank, self.time);
            });

            for shell in self.world.query::<&Shell>().iter() {
                if self.shadows_enabled && shell.state == ShellState::Flying {
                    draw_shell_shadow(&mut d, textures.shells, shell);
                }
                draw_shell(&mut d, textures.shells, shell);
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

        rl.draw(thread, |mut d| {
            d.clear_background(Color::BLACK);

            if self.shock.is_some() {
                d.draw_shader_mode(&mut effects.shock.shader, |mut sd| {
                    sd.draw_texture_rec(
                        &*scene_target,
                        source,
                        Vector2::new(0.0, 0.0),
                        Color::WHITE,
                    );
                });
            } else {
                d.draw_texture_rec(&*scene_target, source, Vector2::new(0.0, 0.0), Color::WHITE);
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
            let hud_y = screen_height - 50;
            let mut hud_x = 50;
            d.draw_text(hud_shells_label, hud_x, hud_y, 24, Color::DARKGRAY);
            hud_x += hud_shells_label_w;
            d.draw_text(&hud_shells_num, hud_x, hud_y, 24, shells_color);
            hud_x += hud_shells_num_w;
            d.draw_text(hud_mid, hud_x, hud_y, 24, Color::DARKGRAY);
            hud_x += hud_mid_w;
            d.draw_text(&hud_hp_num, hud_x, hud_y, 24, hp_color);
            d.draw_text(
                &version_hud,
                screen_width - 120 - version_hud_w,
                screen_height - 50,
                24,
                Color::DARKGRAY,
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

/// Debug inspect-mode overlay for one tank: a bounding square (see
/// `Game::inspect_enabled`, toggled by the "I" key) plus a small stat block -
/// ammo, health, current speed and velocity for every tank, and additionally
/// (`ai: Some`, i.e. this isn't the player) whether it's currently retreating
/// to recharge and its fire cooldown, pulled straight from its `Ai` - the
/// same state `ai.rs`'s `wants_retreat`/`fire_interval` act on. Purely
/// diagnostic: reads state, draws it, mutates nothing.
fn draw_tank_inspect(d: &mut impl RaylibDraw, tank: &Tank, ai: Option<&Ai>) {
    let half = tank.size() * 0.5;
    let x = (tank.position.x - half).round() as i32;
    let y = (tank.position.y - half).round() as i32;
    let side = tank.size().round() as i32;

    let box_color = if tank.is_wreck() {
        Color::GRAY
    } else if ai.is_some_and(Ai::is_retreating) {
        Color::ORANGE
    } else {
        Color::LIME
    };
    d.draw_rectangle_lines(x, y, side, side, box_color);

    let speed = (tank.velocity.x * tank.velocity.x + tank.velocity.y * tank.velocity.y).sqrt();
    let mut lines = vec![
        format!("AMMO {}/{}", tank.shells_ammo, MAX_SHELLS),
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
        Color::DARKGRAY
    }
}
