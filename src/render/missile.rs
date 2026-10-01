//! Drawing a seeker missile and its shadow (`missile.rs` owns the entity).

use sola_raylib::prelude::*;

use crate::math::{Color, Rectangle, Vec2};
use crate::render::shot_fx::{fade, glow, streak};
use crate::missile::Missile;
use crate::tuning::tuning;
use crate::{MISSILE_SCALE, MISSILE_TEXTURE_SIZE};

fn source_rec(missile: &Missile) -> Rectangle {
    Rectangle::new(missile.frame() as f32 * MISSILE_TEXTURE_SIZE, 0.0, MISSILE_TEXTURE_SIZE, MISSILE_TEXTURE_SIZE)
}

/// The missile's shadow on the ground under it: its own silhouette,
/// shrinking and fading as it rises (what sells the height with no camera).
pub fn draw_missile_shadow(d: &mut impl RaylibDraw, texture: &Texture2D, missile: &Missile) {
    let lift = missile.lift();
    let size = MISSILE_TEXTURE_SIZE * MISSILE_SCALE * (1.0 - 0.3 * lift);
    let dest = Rectangle::new(
        missile.position.x + tuning().shadow_dir_x * 4.0,
        missile.position.y + tuning().shadow_dir_y * 4.0,
        size,
        size,
    );
    let alpha = 255.0 * tuning().missile_shadow_opacity * (1.0 - 0.5 * lift);
    let origin = Vec2::new(size / 2.0, size / 2.0);
    d.draw_texture_pro(texture, source_rec(missile), dest, origin, missile.ground_rotation(), Color::new(0, 0, 0, alpha as u8));
}

/// The missile itself, lifted by its height and scaled up as it rises.
pub fn draw_missile(d: &mut impl RaylibDraw, texture: &Texture2D, missile: &Missile) {
    let size = MISSILE_TEXTURE_SIZE * MISSILE_SCALE * missile.draw_scale();
    let at = missile.draw_pos();
    let dest = Rectangle::new(at.x, at.y, size, size);
    let origin = Vec2::new(size / 2.0, size / 2.0);
    d.draw_texture_pro(texture, source_rec(missile), dest, origin, missile.rotation(), Color::WHITE);
}

/// The rocket motor's light (additive, drawn before the sprite): a
/// flickering white-to-orange flame jetting out of the tail and a glow
/// around it, sized with the missile as it climbs. Phased per missile so a
/// salvo does not flicker in step.
pub fn draw_missile_exhaust(d: &mut impl RaylibDraw, missile: &Missile, time: f32) {
    let strength = tuning().shot_glow_strength;
    if strength <= 0.0 {
        return;
    }
    let tail = missile.tail();
    let nose = missile.draw_pos();
    let (dx, dy) = (nose.x - tail.x, nose.y - tail.y);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.01 {
        return;
    }
    let dir = Vec2::new(dx / len, dy / len);
    let phase = (missile.frame() as f32) * 1.7 + tail.x * 0.13;
    let flicker = 0.75 + 0.25 * (time * 53.0 + phase).sin();
    let scale = missile.draw_scale();
    let flame = 10.0 * scale * flicker;
    streak(d, tail, dir, flame, 4.0 * scale, fade(Color::new(255, 250, 220, 255), strength), fade(Color::new(255, 90, 20, 60), strength));
    glow(d, tail, 9.0 * scale * flicker, fade(Color::new(255, 150, 50, 170), strength));
    crate::pyro::block_disc(&mut crate::render::pyro::Rl(d), tail, 2.0 * scale, fade(Color::new(255, 245, 210, 230), strength));
}
