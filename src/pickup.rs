//! Health and ammo pickups: small, inert collectibles that respawn near the
//! battlefield's corners (see `battlefield::sample_corner_position`,
//! `simulation::respawn_pickup`). Unlike `Obstacle`/`Frog` there's no
//! physics body - nothing should ever collide with a pickup, it's picked up
//! by proximity alone (`simulation::collect_pickups`), so it's just a
//! position and a kind, checked against every living tank each frame.

use serde::{Deserialize, Serialize};
use sola_raylib::prelude::*;

use crate::{PICKUP_SCALE, PICKUP_TEXTURE_SIZE, Position};

/// Which effect a pickup has when collected - see `simulation::collect_pickups`.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PickupKind {
    Health,
    Ammo,
}

pub struct Pickup {
    pub kind: PickupKind,
    pub position: Position,
}

impl Pickup {
    /// Side length of this pickup's sprite on screen - drawn 1:1
    /// (PICKUP_SCALE), same "native res reads fine, no need to match the
    /// tanks' chunky look" reasoning as obstacles.
    pub fn size(&self) -> f32 {
        PICKUP_TEXTURE_SIZE * PICKUP_SCALE
    }
}

/// Draw one pickup, centered on its position. `texture` is whichever of
/// `game::Textures::pickup_health`/`pickup_ammo` matches `pickup.kind` - the
/// caller picks (see `Game::render`), since each kind is its own standalone
/// 32x32 image rather than rows in one shared sheet.
pub fn draw_pickup(d: &mut impl RaylibDraw, texture: &Texture2D, pickup: &Pickup) {
    let size = pickup.size();
    let src = Rectangle::new(0.0, 0.0, PICKUP_TEXTURE_SIZE, PICKUP_TEXTURE_SIZE);
    let dest = Rectangle::new(pickup.position.x, pickup.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);
    d.draw_texture_pro(texture, src, dest, origin, 0.0, Color::WHITE);
}
