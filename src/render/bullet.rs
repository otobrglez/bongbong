//! Drawing a minigun bullet and its shadow (`bullet.rs` owns the entity).

use sola_raylib::prelude::*;

use crate::bullet::{Bullet, BulletState};
use crate::math::{Color, Rectangle};
use crate::render::shot_fx::{fade, glow, heading, streak};
use crate::tuning::tuning;
use crate::{MINIGUN_BULLET_SCALE, MINIGUN_BULLET_TEXTURE_SIZE};

/// Column of this state in minigun_bullets.png.
fn state_col(state: BulletState) -> i32 {
    state.col()
}

/// Source rectangle for a bullet frame (state column) in minigun_bullets.png.
fn source_rec(col: i32) -> Rectangle {
    Rectangle::new(
        col as f32 * MINIGUN_BULLET_TEXTURE_SIZE,
        0.0,
        MINIGUN_BULLET_TEXTURE_SIZE,
        MINIGUN_BULLET_TEXTURE_SIZE,
    )
}

/// Draw a bullet using its current state's frame, centered and rotated to
/// face travel.
pub fn draw_bullet(d: &mut impl RaylibDraw, texture: &Texture2D, bullet: &Bullet) {
    let src = source_rec(state_col(bullet.state));
    let size = MINIGUN_BULLET_TEXTURE_SIZE * MINIGUN_BULLET_SCALE;

    let dest = Rectangle::new(bullet.position.x, bullet.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);

    d.draw_texture_pro(texture, src, dest, origin, bullet.rotation, Color::WHITE);
}

/// Draw this bullet's drop shadow - same convention as `draw_shell_shadow`.
/// Caller (`Game::render`) only calls this while `bullet.state ==
/// BulletState::Flying`.
pub fn draw_bullet_shadow(d: &mut impl RaylibDraw, texture: &Texture2D, bullet: &Bullet) {
    let src = source_rec(state_col(bullet.state));
    let size = MINIGUN_BULLET_TEXTURE_SIZE * MINIGUN_BULLET_SCALE;

    let dest = Rectangle::new(
        bullet.position.x + tuning().shadow_dir_x * bullet.shadow_offset,
        bullet.position.y + tuning().shadow_dir_y * bullet.shadow_offset,
        size,
        size,
    );
    let origin = Vector2::new(size / 2.0, size / 2.0);
    let shadow = Color::new(0, 0, 0, (255.0 * tuning().minigun_bullet_shadow_opacity) as u8);

    d.draw_texture_pro(texture, src, dest, origin, bullet.rotation, shadow);
}

/// A flying bullet's tracer (additive, before the sprites): a thin
/// yellow-white streak cooling to orange behind it and a pinpoint glow on
/// the round, so a burst draws bright dashed lines across the field.
pub fn draw_bullet_light(d: &mut impl RaylibDraw, bullet: &Bullet) {
    let strength = tuning().shot_glow_strength;
    if bullet.state != BulletState::Flying || strength <= 0.0 {
        return;
    }
    let dir = heading(bullet.rotation);
    let length = tuning().bullet_tracer_length;
    streak(d, bullet.position, dir, length, 3.0, fade(Color::new(255, 150, 40, 170), strength), Color::new(200, 50, 10, 0));
    streak(d, bullet.position, dir, length * 0.8, 1.5, fade(Color::new(255, 255, 200, 255), strength), fade(Color::new(255, 140, 30, 110), strength));
    glow(d, bullet.position, 5.0, fade(Color::new(255, 220, 120, 200), strength));
}
