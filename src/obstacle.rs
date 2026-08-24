use rapier2d::prelude::RigidBodyHandle;
use serde::{Deserialize, Serialize};
use sola_raylib::prelude::*;

use crate::{
    OBSTACLE_BRICK_MAX_HEALTH, OBSTACLE_GLASS_MAX_HEALTH, OBSTACLE_HULL_FRACTION,
    OBSTACLE_IRON_MAX_HEALTH, OBSTACLE_SCALE, OBSTACLE_SHADOW_OFFSET, OBSTACLE_SHADOW_OPACITY,
    OBSTACLE_TEXTURE_SIZE, OBSTACLE_WOOD_BURN_FRAME_SECONDS, OBSTACLE_WOOD_BURN_SECONDS,
    OBSTACLE_WOOD_MAX_HEALTH, Position, SHADOW_DIR_X, SHADOW_DIR_Y,
};

/// Which of the four materials a static battlefield wall is built from - see
/// walls_sheet.png / docs/WALLS_SPEC.md for the sheet layout each one draws
/// from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Material {
    Brick,
    Iron,
    Wood,
    Glass,
}

/// Every material, in walls_sheet.png row order - `Game::init` rolls a
/// uniform pick from this for each obstacle spawn.
pub const MATERIALS: [Material; 4] = [Material::Brick, Material::Iron, Material::Wood, Material::Glass];

impl Material {
    /// First row in walls_sheet.png this material's variants start at.
    fn row_base(self) -> i32 {
        match self {
            Material::Brick => 0,
            Material::Iron => 4,
            Material::Wood => 8,
            Material::Glass => 12,
        }
    }

    /// Number of cosmetic variant rows (bond pattern / board layout / surface
    /// treatment) this material has - `Obstacle::variant` is rolled in
    /// `0..variants()` at spawn.
    pub fn variants(self) -> i32 {
        match self {
            Material::Glass => 2,
            _ => 4,
        }
    }

    /// HP this material absorbs before reaching its terminal state (rubble /
    /// charred / shattered) - or, for Iron, before its cosmetic rust stage
    /// plateaus, since Iron is never actually destroyed. Ordered fragile to
    /// tough: glass snaps almost immediately, wood breaks easily, brick
    /// holds longer, iron the longest of all on top of being permanent.
    pub fn max_health(self) -> f32 {
        match self {
            Material::Glass => OBSTACLE_GLASS_MAX_HEALTH,
            Material::Wood => OBSTACLE_WOOD_MAX_HEALTH,
            Material::Brick => OBSTACLE_BRICK_MAX_HEALTH,
            Material::Iron => OBSTACLE_IRON_MAX_HEALTH,
        }
    }

    /// Number of visible damage-stage columns this material ever actually
    /// draws before dying (Wood only while not burning - see
    /// `Obstacle::state`). The terminal stage (brick rubble, wood destroyed,
    /// glass shattered) is never drawn: `destroyed` fires the same frame
    /// health reaches zero, and `Game::update` removes the entity that same
    /// frame - the same "vanishes the instant it dies" behaviour the old
    /// Crate always had. Iron has no terminal stage at all, so all 4 of its
    /// columns are visible.
    fn visible_stages(self) -> i32 {
        match self {
            Material::Brick => 5,
            Material::Iron => 4,
            Material::Wood => 3,
            Material::Glass => 3,
        }
    }
}

/// A static battlefield obstacle: blocks tank movement like a wall (reusing
/// `physics::Physics::spawn_static`, the exact same fixed-body/cuboid-collider
/// shape), but placed inside the arena rather than around its edge, and -
/// for every material but Iron - shootable.
pub struct Obstacle {
    pub material: Material,
    /// Which cosmetic variant row of `material` this instance draws from,
    /// rolled once at spawn (see `Game::init`) - purely visual, same pattern
    /// as `Tank::shell_variant`.
    pub variant: i32,
    pub position: Position,
    pub health: f32,
    pub max_health: f32,
    /// Wood only (ignored for every other material): whether this instance
    /// catches fire when destroyed instead of breaking outright - rolled
    /// once at spawn (`OBSTACLE_WOOD_FLAMMABLE_CHANCE`), per docs/WALLS_SPEC.md's
    /// framing of "breaks easily" vs "catches fire" as gameplay data layered
    /// on shared art, not a separate art asset.
    pub flammable: bool,
    /// Wood only: true from the moment flammable Wood's health hits zero
    /// until it finishes charring (see `tick_burn`) - during this window
    /// `damage` is a no-op (already on fire) and `state` shows the 3-frame
    /// burn loop instead of a damage stage.
    pub burning: bool,
    /// Wood only: which of the burn loop's 3 frames (cols 4-6) is showing.
    pub burn_frame: i32,
    /// Wood only: seconds since `burn_frame` last advanced.
    pub burn_frame_timer: f32,
    /// Wood only: total seconds spent burning so far - once this passes
    /// `OBSTACLE_WOOD_BURN_SECONDS`, `tick_burn` chars it out (`destroyed = true`).
    pub burn_elapsed: f32,
    /// This obstacle's rapier fixed-body collider, spawned alongside it (see
    /// `Game::spawn_obstacles`). Unlike a tank's `body`, this is never
    /// `None` - an obstacle always has its physics body for its whole life,
    /// right up until `Game::update` removes it the same frame `destroyed`
    /// is set.
    pub body: RigidBodyHandle,
    /// Set once a destructible material's health hits zero (Brick/Glass
    /// immediately, Wood either immediately or after `tick_burn` finishes
    /// charring it - see `damage`/`tick_burn`); `Game::update` removes its
    /// physics body and drops it from `Game::obstacles` that same frame,
    /// mirroring how a finished `Shell` is cleaned up via `Shell::done`.
    /// Always `false` for Iron.
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
        self.material.row_base() + self.variant
    }

    /// Sprite atlas column: the current damage/rust stage derived from
    /// `health`/`max_health`, or - for burning Wood - the 3-frame fire loop
    /// column. See `Material::visible_stages` for why the terminal
    /// destroyed/shattered/rubble stage never actually gets picked here.
    fn col(&self) -> i32 {
        if self.burning {
            return 4 + self.burn_frame;
        }
        let stages = self.material.visible_stages();
        let frac = (self.health / self.max_health).clamp(0.0, 1.0);
        let stage = ((1.0 - frac) * stages as f32) as i32;
        stage.clamp(0, stages - 1)
    }

    /// Apply shell damage. Returns `true` exactly the frame this obstacle
    /// dies outright (Brick/Glass reaching zero health, or non-flammable
    /// Wood reaching zero health), so `Game::update` knows precisely when to
    /// remove the physics body and spawn a hit effect - rather than
    /// re-triggering on every subsequent frame a shell happens to overlap
    /// the wreckage. Iron drains health cosmetically (plateaus its rust
    /// stage, see `Material::max_health`) but never returns `true`.
    /// Flammable Wood returns `false` the frame it ignites too - it only
    /// actually dies once `tick_burn` finishes charring it.
    pub fn damage(&mut self, amount: f32) -> bool {
        if self.destroyed || self.burning {
            return false;
        }
        self.health = (self.health - amount).max(0.0);
        if self.health > 0.0 {
            return false;
        }
        if self.material == Material::Iron {
            return false;
        }
        if self.material == Material::Wood && self.flammable {
            self.burning = true;
            return false;
        }
        self.destroyed = true;
        true
    }

    /// Advance flammable Wood's burn loop - a no-op for every other
    /// material/state. Cosmetic only (see docs/WALLS_SPEC.md's fire
    /// section): cycles `burn_frame` through the sheet's 3-frame flicker
    /// loop on `OBSTACLE_WOOD_BURN_FRAME_SECONDS`, and once `burn_elapsed`
    /// passes `OBSTACLE_WOOD_BURN_SECONDS`, chars it out (`destroyed = true`)
    /// so it's removed the same instant-vanish way every other destroyed
    /// material already is.
    pub fn tick_burn(&mut self, dt: f32) {
        if !self.burning {
            return;
        }
        self.burn_frame_timer += dt;
        if self.burn_frame_timer >= OBSTACLE_WOOD_BURN_FRAME_SECONDS {
            self.burn_frame_timer -= OBSTACLE_WOOD_BURN_FRAME_SECONDS;
            self.burn_frame = (self.burn_frame + 1) % 3;
        }
        self.burn_elapsed += dt;
        if self.burn_elapsed >= OBSTACLE_WOOD_BURN_SECONDS {
            self.destroyed = true;
        }
    }
}

/// Source rectangle for `material`'s pristine (variant 0, undamaged) tile -
/// used by the map editor's toolbar (`editor.rs`) to draw a representative
/// icon for each wall material without needing a live `Obstacle` instance.
#[cfg(feature = "map-editor")]
pub fn icon_source_rec(material: Material) -> Rectangle {
    source_rec(material.row_base(), 0)
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
