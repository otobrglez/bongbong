use rapier2d::prelude::RigidBodyHandle;
use sola_raylib::prelude::*;

use crate::{
    OBSTACLE_CRATE_ROW, OBSTACLE_CRATE_STAGES, OBSTACLE_HULL_FRACTION, OBSTACLE_ROCK_ROW,
    OBSTACLE_SCALE, OBSTACLE_SHADOW_OFFSET, OBSTACLE_SHADOW_OPACITY, OBSTACLE_TEXTURE_SIZE,
    Position, SHADOW_DIR_X, SHADOW_DIR_Y,
};

/// What kind of static battlefield obstacle this is - see obstacles.png.
#[derive(Clone, Copy, PartialEq)]
pub enum ObstacleKind {
    /// Indestructible terrain - blocks tanks and shells alike, forever.
    Rock,
    /// Destructible: takes shell damage (see `damage`) and breaks once its
    /// health reaches zero.
    Crate,
}

/// A static battlefield obstacle: blocks tank movement like a wall (reusing
/// `physics::Physics::spawn_static`, the exact same fixed-body/cuboid-collider
/// shape), but placed inside the arena rather than around its edge, and -
/// for a `Crate` - shootable.
pub struct Obstacle {
    pub kind: ObstacleKind,
    pub position: Position,
    pub health: f32,
    pub max_health: f32,
    /// This obstacle's rapier fixed-body collider, spawned alongside it (see
    /// `Game::spawn_obstacles`). Unlike a tank's `body`, this is never
    /// `None` - an obstacle always has its physics body for its whole life,
    /// right up until `Game::update` removes it the same frame `destroyed`
    /// is set.
    pub body: RigidBodyHandle,
    /// Set once a `Crate`'s health hits zero (see `damage`); `Game::update`
    /// removes its physics body and drops it from `Game::obstacles` that
    /// same frame, mirroring how a finished `Shell` is cleaned up via
    /// `Shell::done`. Always `false` for a `Rock`.
    pub destroyed: bool,
}

impl Obstacle {
    /// Side length of this obstacle on screen (square sprite), matching
    /// `Tank::size`.
    pub fn size(&self) -> f32 {
        OBSTACLE_TEXTURE_SIZE * OBSTACLE_SCALE
    }

    /// Collision footprint side length - see OBSTACLE_HULL_FRACTION, same
    /// reasoning as `Tank::hull_size`.
    pub fn hull_size(&self) -> f32 {
        self.size() * OBSTACLE_HULL_FRACTION
    }

    fn row(&self) -> i32 {
        match self.kind {
            ObstacleKind::Rock => OBSTACLE_ROCK_ROW,
            ObstacleKind::Crate => OBSTACLE_CRATE_ROW,
        }
    }

    /// Sprite atlas column: always 0 for a Rock (one static frame); a Crate
    /// steps through OBSTACLE_CRATE_STAGES frames as it takes damage, from
    /// pristine at full health down to the last frame at the edge of
    /// breaking - the same current/max-health-driven frame pick
    /// `damage_stage.rs` uses for a tank, just without the animation.
    fn col(&self) -> i32 {
        match self.kind {
            ObstacleKind::Rock => 0,
            ObstacleKind::Crate => {
                let frac = (self.health / self.max_health).clamp(0.0, 1.0);
                let stage = ((1.0 - frac) * OBSTACLE_CRATE_STAGES as f32) as i32;
                stage.clamp(0, OBSTACLE_CRATE_STAGES - 1)
            }
        }
    }

    /// Apply shell damage. No-op (and always `false`) for an indestructible
    /// `Rock`, or an already-`destroyed` `Crate`. Returns `true` exactly the
    /// frame health first reaches zero, so `Game::update` knows precisely
    /// when to remove the physics body and spawn a hit effect, rather than
    /// re-triggering on every subsequent frame a shell happens to overlap
    /// the wreckage.
    pub fn damage(&mut self, amount: f32) -> bool {
        if self.kind != ObstacleKind::Crate || self.destroyed {
            return false;
        }
        self.health = (self.health - amount).max(0.0);
        if self.health <= 0.0 {
            self.destroyed = true;
            return true;
        }
        false
    }
}

/// Source rectangle for the obstacle at (row, col) inside the atlas.
fn source_rec(row: i32, col: i32) -> Rectangle {
    Rectangle::new(
        col as f32 * OBSTACLE_TEXTURE_SIZE,
        row as f32 * OBSTACLE_TEXTURE_SIZE,
        OBSTACLE_TEXTURE_SIZE,
        OBSTACLE_TEXTURE_SIZE,
    )
}

/// Draw a single obstacle sprite from the atlas at its center position.
/// Obstacles never rotate (unlike tanks/shells), so this skips the
/// rotation param `draw_tank` needs.
pub fn draw_obstacle(d: &mut impl RaylibDraw, texture: &Texture2D, obstacle: &Obstacle) {
    let src = source_rec(obstacle.row(), obstacle.col());
    let size = obstacle.size();
    let dest = Rectangle::new(obstacle.position.x, obstacle.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);
    d.draw_texture_pro(texture, src, dest, origin, 0.0, Color::WHITE);
}

/// Draw this obstacle's drop shadow - see `tank::draw_tank_shadow` /
/// docs/sprite-shadows-design.md. Must be called before `draw_obstacle`.
pub fn draw_obstacle_shadow(d: &mut impl RaylibDraw, texture: &Texture2D, obstacle: &Obstacle) {
    let src = source_rec(obstacle.row(), obstacle.col());
    let size = obstacle.size();
    let dest = Rectangle::new(
        obstacle.position.x + SHADOW_DIR_X * OBSTACLE_SHADOW_OFFSET,
        obstacle.position.y + SHADOW_DIR_Y * OBSTACLE_SHADOW_OFFSET,
        size,
        size,
    );
    let origin = Vector2::new(size / 2.0, size / 2.0);
    let shadow = Color::new(0, 0, 0, (255.0 * OBSTACLE_SHADOW_OPACITY) as u8);
    d.draw_texture_pro(texture, src, dest, origin, 0.0, shadow);
}
