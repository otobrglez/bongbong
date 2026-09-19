//! Drawing a minigun bullet and its shadow (`bullet.rs` owns the entity).

use sola_raylib::prelude::*;

use crate::bullet::{Bullet, BulletState};
use crate::math::{Color, Rectangle};
use crate::tuning::tuning;
use crate::{MINIGUN_BULLET_SCALE, MINIGUN_BULLET_TEXTURE_SIZE};

/// Column of this state in minigun_bullets.png.
fn state_col(state: BulletState) -> i32 {
    match state {
        BulletState::Muzzle => 0,
        BulletState::Flying => 1,
        BulletState::Hit => 2,
    }
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
