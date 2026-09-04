use crate::tuning::tuning;
use rapier2d::prelude::RigidBodyHandle;
use serde::{Deserialize, Serialize};
use sola_raylib::prelude::*;
use std::collections::HashSet;

use crate::{
    OBSTACLE_GRID_SIZE,
    OBSTACLE_HULL_FRACTION,
    OBSTACLE_SCALE,
    OBSTACLE_TEXTURE_SIZE,
    PROPS_BARREL_LIT_COL,
    Position,
};

/// What a static battlefield obstacle is: one of the four wall materials
/// (walls_sheet.png / docs/WALLS_SPEC.md) or one of the three discrete props
/// (props_sheet.png / docs/PROPS_SPEC.md). Props are obstacles too - same
/// physics body, same grid cell, same hit sweep - but each has its own
/// rules (shots pass over sandbags, barrels explode, fences snap) that the
/// predicates below express, so the rest of the code asks "does this block
/// sight?" rather than matching on the variant.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Material {
    Brick,
    Iron,
    Wood,
    Glass,
    Sandbag,
    Barrel,
    Fence,
}

/// The four wall materials, in walls_sheet.png row order - `spawn_from_map`
/// rolls one cosmetic variant per material from this list for each round.
/// Props are not in it: they roll a variant per tile instead, and their
/// toughness is a scalar knob each rather than a `wall_max_health` slot.
pub const MATERIALS: [Material; 4] = [Material::Brick, Material::Iron, Material::Wood, Material::Glass];

/// Which atlas a material's rows live in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sheet {
    Walls,
    Props,
}

impl Material {
    pub fn sheet(self) -> Sheet {
        match self {
            Material::Sandbag | Material::Barrel | Material::Fence => Sheet::Props,
            _ => Sheet::Walls,
        }
    }

    /// First row in this material's own sheet (see `sheet`) its variants
    /// start at.
    fn row_base(self) -> i32 {
        match self {
            Material::Brick => 0,
            Material::Iron => 4,
            Material::Wood => 8,
            Material::Glass => 12,
            Material::Sandbag => 0,
            Material::Barrel => 3,
            Material::Fence => 5,
        }
    }

    /// Number of cosmetic variants (bond pattern / board layout / bag
    /// arrangement / drum livery / fence style) this material has -
    /// `Obstacle::variant` is rolled in `0..variants()` at spawn. A fence
    /// variant owns two rows (horizontal and vertical, see `FenceAxis`).
    pub fn variants(self) -> i32 {
        match self {
            Material::Glass => 2,
            Material::Sandbag => 3,
            Material::Barrel | Material::Fence => 2,
            _ => 4,
        }
    }

    /// HP this material absorbs before reaching its terminal state (rubble /
    /// charred / shattered / collapsed / detonated) - or, for Iron, before
    /// its cosmetic rust stage plateaus, since Iron is never destroyed. A
    /// fence is a two-state machine that ignores the damage amount
    /// (`Game::damage_obstacle`), so its health is just "two stages".
    pub fn max_health(self) -> f32 {
        match self {
            // Indexed by declaration order, which `tuning::MATERIAL_NAMES`
            // mirrors (brick, iron, wood, glass).
            Material::Brick | Material::Iron | Material::Wood | Material::Glass => tuning().wall_max_health[self as usize],
            Material::Sandbag => tuning().sandbag_max_health,
            Material::Barrel => tuning().barrel_max_health,
            Material::Fence => 2.0,
        }
    }

    /// Number of visible damage-stage columns this material ever actually
    /// draws before dying (Wood only while not burning - see
    /// `Obstacle::col`). The terminal stage (brick rubble, wood destroyed,
    /// glass shattered, a flattened sandbag, a detonated barrel, a fence
    /// reduced to stubs) is never drawn: `destroyed` fires the same frame
    /// health reaches zero, and `Game::update` removes the entity that same
    /// frame. Iron has no terminal stage at all, so all 4 of its columns
    /// are visible.
    fn visible_stages(self) -> i32 {
        match self {
            Material::Brick => 5,
            Material::Iron => 4,
            Material::Wood => 3,
            Material::Glass => 3,
            Material::Sandbag => 3,
            Material::Barrel => 3,
            Material::Fence => 2,
        }
    }

    /// A discrete prop (sandbag, barrel, fence) rather than a wall tile.
    pub fn is_prop(self) -> bool {
        self.sheet() == Sheet::Props
    }

    /// Can never be destroyed - the only material that permanently shapes
    /// the battlefield (line of fire, the linter's breach grid).
    pub fn is_permanent(self) -> bool {
        self == Material::Iron
    }

    /// Whether this tile hides what is behind it from the AI's line of
    /// sight. Sandbags are knee-high and a fence is see-through; everything
    /// else is a solid block.
    pub fn blocks_sight(self) -> bool {
        !matches!(self, Material::Sandbag | Material::Fence)
    }

    /// Odds a projectile sails over this tile instead of hitting it, rolled
    /// per projectile per tile (`Game::resolve_projectiles`). Zero means
    /// "never", and no RNG is drawn for it.
    pub fn pass_over_chance(self) -> f64 {
        match self {
            Material::Sandbag => tuning().sandbag_pass_over_chance,
            Material::Barrel => tuning().barrel_pass_over_chance,
            _ => 0.0,
        }
    }

    /// Odds a shell or bullet ricochets off this tile instead of hitting it
    /// (on top of the Iron rule in `Shell::try_ricochet`). Zero draws no RNG.
    pub fn deflect_chance(self) -> f64 {
        match self {
            Material::Barrel => tuning().barrel_deflect_chance,
            _ => 0.0,
        }
    }

    /// How long a tank has to push into this tile before it collapses
    /// (`Game::ram_props`); `None` for tiles ramming cannot flatten.
    pub fn ram_seconds(self) -> Option<f32> {
        match self {
            Material::Sandbag => Some(tuning().sandbag_ram_seconds),
            Material::Fence => Some(tuning().fence_ram_seconds),
            _ => None,
        }
    }

    /// Detonates when destroyed (`Game::apply_blast`).
    pub fn is_explosive(self) -> bool {
        self == Material::Barrel
    }
}

/// Which way a fence tile runs - decided at draw time from its fence
/// neighbours (`fence_axis`), since obstacles never rotate and the map only
/// stores the cell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FenceAxis {
    Horizontal = 0,
    Vertical = 1,
}

/// The two obstacle atlases, bundled so a draw call can pick by
/// `Material::sheet` without the caller matching on the material.
pub struct ObstacleTextures<'a> {
    pub walls: &'a Texture2D,
    pub props: &'a Texture2D,
}

/// A static battlefield obstacle: blocks tank movement like a wall (reusing
/// `physics::Physics::spawn_static`, the exact same fixed-body/cuboid-collider
/// shape), but placed inside the arena rather than around its edge, and -
/// for every material but Iron - shootable.
pub struct Obstacle {
    pub material: Material,
    /// Which cosmetic variant row of `material` this instance draws from,
    /// rolled once at spawn (see `battlefield::spawn_from_map`) - purely
    /// visual, same pattern as `Tank::shell_variant`.
    pub variant: i32,
    pub position: Position,
    pub health: f32,
    pub max_health: f32,
    /// Wood only (ignored for every other material): whether this instance
    /// catches fire when destroyed instead of breaking outright - rolled
    /// once at spawn (`wood_flammable_chance`), per docs/WALLS_SPEC.md's
    /// framing of "breaks easily" vs "catches fire" as gameplay data layered
    /// on shared art, not a separate art asset.
    pub flammable: bool,
    /// Wood only: true from the moment flammable Wood's health hits zero
    /// until it finishes charring (see `tick_burn`) - during this window
    /// `damage` is a no-op (already on fire) and `col` shows the 3-frame
    /// burn loop instead of a damage stage.
    pub burning: bool,
    /// Wood only: which of the burn loop's 3 frames (cols 4-6) is showing.
    pub burn_frame: i32,
    /// Wood only: seconds since `burn_frame` last advanced.
    pub burn_frame_timer: f32,
    /// Wood only: total seconds spent burning so far - once this passes
    /// `wood_burn_seconds`, `tick_burn` chars it out (`destroyed = true`).
    pub burn_elapsed: f32,
    /// Barrel only: seconds until this barrel detonates, armed when a
    /// neighbouring blast reached it (`Game::apply_blast`) so a chain
    /// reaction cascades visibly instead of going off all at once. While
    /// armed the barrel is inert to further damage, like burning Wood, and
    /// draws its lit-fuse column.
    pub fuse: Option<f32>,
    /// Sandbag/Fence only: seconds a tank has been pushing into this tile
    /// (`Game::ram_props`); decays while nothing pushes. Collapses at
    /// `Material::ram_seconds`.
    pub ram_timer: f32,
    /// This obstacle's rapier fixed-body collider, spawned alongside it.
    /// Unlike a tank's `body`, this is never `None` - an obstacle always has
    /// its physics body for its whole life, right up until `Game::update`
    /// removes it the same frame `destroyed` is set.
    pub body: RigidBodyHandle,
    /// Set once a destructible material's health hits zero (Brick/Glass and
    /// the props immediately, Wood either immediately or after `tick_burn`
    /// finishes charring it, a chained barrel when its fuse runs out);
    /// `Game::update` removes its physics body and despawns it that same
    /// frame, mirroring how a finished `Shell` is cleaned up. Always
    /// `false` for Iron.
    pub destroyed: bool,
}

impl Obstacle {
    /// A fresh, undamaged obstacle of `material` at full health.
    pub fn new(material: Material, variant: i32, position: Position, flammable: bool, body: RigidBodyHandle) -> Self {
        let max_health = material.max_health();
        Obstacle {
            material,
            variant,
            position,
            health: max_health,
            max_health,
            flammable,
            burning: false,
            burn_frame: 0,
            burn_frame_timer: 0.0,
            burn_elapsed: 0.0,
            fuse: None,
            ram_timer: 0.0,
            body,
            destroyed: false,
        }
    }

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

    /// The grid cell this obstacle sits in (positions are grid-aligned, see
    /// `map::cell_to_world`).
    pub fn cell(&self) -> (i32, i32) {
        (
            (self.position.x / OBSTACLE_GRID_SIZE).floor() as i32,
            (self.position.y / OBSTACLE_GRID_SIZE).floor() as i32,
        )
    }

    fn row(&self, axis: FenceAxis) -> i32 {
        match self.material {
            Material::Fence => self.material.row_base() + self.variant * 2 + axis as i32,
            _ => self.material.row_base() + self.variant,
        }
    }

    /// Sprite atlas column: the current damage/rust stage derived from
    /// `health`/`max_health`, or - for burning Wood - the 3-frame fire loop
    /// column, or - for a barrel on a fuse - the lit column. See
    /// `Material::visible_stages` for why the terminal
    /// destroyed/shattered/rubble stage never actually gets picked here.
    fn col(&self) -> i32 {
        if self.burning {
            return 4 + self.burn_frame;
        }
        if self.fuse.is_some() {
            return PROPS_BARREL_LIT_COL;
        }
        let stages = self.material.visible_stages();
        let frac = (self.health / self.max_health).clamp(0.0, 1.0);
        let stage = ((1.0 - frac) * stages as f32) as i32;
        stage.clamp(0, stages - 1)
    }

    /// Apply damage. Returns `true` exactly the frame this obstacle dies
    /// outright (health reaching zero on anything but Iron, burning-fork
    /// Wood or a fused barrel), so the caller knows precisely when to
    /// remove the physics body and spawn a hit effect - rather than
    /// re-triggering on every subsequent frame a shell happens to overlap
    /// the wreckage. Iron drains health cosmetically (plateaus its rust
    /// stage, see `Material::max_health`) but never returns `true`.
    /// Flammable Wood returns `false` the frame it ignites too - it only
    /// actually dies once `tick_burn` finishes charring it. Callers on the
    /// simulation path go through `Game::damage_obstacle`, which layers the
    /// fence and barrel rules on top of this.
    pub fn damage(&mut self, amount: f32) -> bool {
        if self.destroyed || self.burning || self.fuse.is_some() {
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
    /// loop on `wood_burn_frame_seconds`, and once `burn_elapsed` passes
    /// `wood_burn_seconds`, chars it out (`destroyed = true`) so it's
    /// removed the same instant-vanish way every other destroyed material
    /// already is.
    pub fn tick_burn(&mut self, dt: f32) {
        if !self.burning {
            return;
        }
        self.burn_frame_timer += dt;
        if self.burn_frame_timer >= tuning().wood_burn_frame_seconds {
            self.burn_frame_timer -= tuning().wood_burn_frame_seconds;
            self.burn_frame = (self.burn_frame + 1) % 3;
        }
        self.burn_elapsed += dt;
        if self.burn_elapsed >= tuning().wood_burn_seconds {
            self.destroyed = true;
        }
    }
}

/// Which way to draw a fence tile: along whichever axis it has more fence
/// neighbours, horizontal for a lone tile or a tie (a corner). `fences` is
/// the set of live fence cells, built once per frame by the renderer.
pub fn fence_axis(obstacle: &Obstacle, fences: &HashSet<(i32, i32)>) -> FenceAxis {
    let (cx, cy) = obstacle.cell();
    let horizontal = fences.contains(&(cx - 1, cy)) as i32 + fences.contains(&(cx + 1, cy)) as i32;
    let vertical = fences.contains(&(cx, cy - 1)) as i32 + fences.contains(&(cx, cy + 1)) as i32;
    if vertical > horizontal {
        FenceAxis::Vertical
    } else {
        FenceAxis::Horizontal
    }
}

/// Source rectangle (and which sheet it is in) for `material`'s pristine
/// (variant 0, undamaged) tile - used by the map editor's toolbar
/// (`editor.rs`) to draw a representative icon for each material without
/// needing a live `Obstacle` instance.
#[cfg(feature = "map-editor")]
pub fn icon_source_rec(material: Material) -> (Sheet, Rectangle) {
    (material.sheet(), source_rec(material.row_base(), 0))
}

/// Source rectangle for the obstacle at (row, col) inside either atlas -
/// both use the same 32px cells.
fn source_rec(row: i32, col: i32) -> Rectangle {
    Rectangle::new(
        col as f32 * OBSTACLE_TEXTURE_SIZE,
        row as f32 * OBSTACLE_TEXTURE_SIZE,
        OBSTACLE_TEXTURE_SIZE,
        OBSTACLE_TEXTURE_SIZE,
    )
}

fn texture_for<'a>(textures: &ObstacleTextures<'a>, material: Material) -> &'a Texture2D {
    match material.sheet() {
        Sheet::Walls => textures.walls,
        Sheet::Props => textures.props,
    }
}

/// Draw a single obstacle sprite from its atlas at its center position.
/// Obstacles never rotate (unlike tanks/shells), so this skips the
/// rotation param `draw_tank` needs; `axis` only matters for fences.
pub fn draw_obstacle(d: &mut impl RaylibDraw, textures: &ObstacleTextures, obstacle: &Obstacle, axis: FenceAxis) {
    let src = source_rec(obstacle.row(axis), obstacle.col());
    let size = obstacle.size();
    let dest = Rectangle::new(obstacle.position.x, obstacle.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);
    d.draw_texture_pro(texture_for(textures, obstacle.material), src, dest, origin, 0.0, Color::WHITE);
}

/// Draw this obstacle's drop shadow - see `tank::draw_tank_shadow` /
/// docs/sprite-shadows-design.md. Must be called before `draw_obstacle`.
pub fn draw_obstacle_shadow(d: &mut impl RaylibDraw, textures: &ObstacleTextures, obstacle: &Obstacle, axis: FenceAxis) {
    let src = source_rec(obstacle.row(axis), obstacle.col());
    let size = obstacle.size();
    let dest = Rectangle::new(
        obstacle.position.x + tuning().shadow_dir_x * tuning().obstacle_shadow_offset,
        obstacle.position.y + tuning().shadow_dir_y * tuning().obstacle_shadow_offset,
        size,
        size,
    );
    let origin = Vector2::new(size / 2.0, size / 2.0);
    let shadow = Color::new(0, 0, 0, (255.0 * tuning().obstacle_shadow_opacity) as u8);
    d.draw_texture_pro(texture_for(textures, obstacle.material), src, dest, origin, 0.0, shadow);
}
