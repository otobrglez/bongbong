use crate::tuning::tuning;
use rapier2d::prelude::RigidBodyHandle;
use serde::{Deserialize, Serialize};
use sola_raylib::prelude::*;
use std::collections::HashSet;

use crate::{
    EDGE_CAP_ROW_BASE,
    OBSTACLE_GRID_SIZE,
    RUBBLE_ROW_TREE,
    RUBBLE_ROW_TREE_CHARRED,
    RUBBLE_ROW_BARREL,
    RUBBLE_ROW_BRICK,
    RUBBLE_ROW_FENCE,
    RUBBLE_ROW_GLASS,
    RUBBLE_ROW_SANDBAG,
    RUBBLE_ROW_WOOD,
    RUBBLE_ROW_WOOD_CHARRED,
    OBSTACLE_HULL_FRACTION,
    OBSTACLE_SCALE,
    OBSTACLE_TEXTURE_SIZE,
    PROPS_BARREL_LIT_COL,
    PROPS_OIL_ROW,
    PROPS_OIL_VARIANTS,
    Position,
    TREE_BURN_COL,
    TREE_ROW_BROADLEAF,
    TREE_SHIMMER_FRAMES,
    TREE_STAGES,
    TREE_ROW_CONIFER,
    TREE_TEXTURE_SIZE,
    TREE_VARIANTS,
};

/// What a static battlefield obstacle is: one of the four wall materials
/// (walls_sheet.png / docs/WALLS_SPEC.md), one of the three discrete props
/// (props_sheet.png / docs/PROPS_SPEC.md), or one of the two tree species
/// (trees_sheet.png / docs/TREES_SPEC.md). All of them are obstacles - same
/// physics body, same grid cell, same hit sweep - but each has its own
/// rules (shots pass over sandbags, barrels explode, fences snap, trees
/// burn and can be flattened) that the predicates below express, so the
/// rest of the code asks "does this block sight?" rather than matching on
/// the variant.
///
/// **Order is load-bearing at the top of the list only.** `max_health`
/// indexes `tuning().wall_max_health`, a `[f32; 4]`, by `self as usize`, so
/// the four wall materials must stay the first four; everything after them
/// has a scalar knob of its own and is safe to append to.
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
    /// A broad, bushy deciduous crown.
    Tree,
    /// A conifer - spikier and darker, so the two read apart by silhouette
    /// before colour.
    Pine,
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
    Trees,
}

impl Sheet {
    /// Source cell size in this atlas. Walls and props share the obstacle
    /// grid's own 32px; trees are drawn from 48px cells so a canopy can
    /// overhang the cell its trunk stands in (see `TREE_TEXTURE_SIZE`).
    pub fn cell(self) -> f32 {
        match self {
            Sheet::Trees => TREE_TEXTURE_SIZE,
            _ => OBSTACLE_TEXTURE_SIZE,
        }
    }
}

impl Material {
    pub fn sheet(self) -> Sheet {
        match self {
            Material::Sandbag | Material::Barrel | Material::Fence => Sheet::Props,
            Material::Tree | Material::Pine => Sheet::Trees,
            _ => Sheet::Walls,
        }
    }

    /// First row in this material's own sheet (see `sheet`) its variants
    /// start at.
    pub(crate) fn row_base(self) -> i32 {
        match self {
            Material::Brick => 0,
            Material::Iron => 4,
            Material::Wood => 8,
            Material::Glass => 12,
            Material::Sandbag => 0,
            Material::Barrel => 3,
            Material::Fence => 5,
            Material::Tree => TREE_ROW_BROADLEAF,
            Material::Pine => TREE_ROW_CONIFER,
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
            Material::Tree | Material::Pine => TREE_VARIANTS,
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
            Material::Tree => tuning().tree_max_health,
            Material::Pine => tuning().pine_max_health,
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
            Material::Tree | Material::Pine => 3,
        }
    }

    /// Which atlas and row of rubble this material leaves on the ground
    /// when it dies, if any (`decal::Decal`, `RUBBLE_ROW_*`). `charred`
    /// picks a burnt-out variant over the intact one where there is one.
    ///
    /// `None` only for Iron, which never dies. Everything on the walls and
    /// props sheets leaves its rubble on the *walls* sheet (see
    /// `RUBBLE_ROW_SANDBAG`); trees are the exception, because leaf litter
    /// is green and the walls sheet is under the no-green guard.
    pub fn rubble_row(self, charred: bool) -> Option<(Sheet, i32)> {
        match self {
            Material::Brick => Some((Sheet::Walls, RUBBLE_ROW_BRICK)),
            Material::Wood => Some((
                Sheet::Walls,
                if charred { RUBBLE_ROW_WOOD_CHARRED } else { RUBBLE_ROW_WOOD },
            )),
            Material::Glass => Some((Sheet::Walls, RUBBLE_ROW_GLASS)),
            Material::Sandbag => Some((Sheet::Walls, RUBBLE_ROW_SANDBAG)),
            Material::Barrel => Some((Sheet::Walls, RUBBLE_ROW_BARREL)),
            Material::Fence => Some((Sheet::Walls, RUBBLE_ROW_FENCE)),
            Material::Tree | Material::Pine => Some((
                Sheet::Trees,
                if charred { RUBBLE_ROW_TREE_CHARRED } else { RUBBLE_ROW_TREE },
            )),
            Material::Iron => None,
        }
    }

    /// A discrete prop (sandbag, barrel, fence) rather than a wall tile.
    pub fn is_prop(self) -> bool {
        self.sheet() == Sheet::Props
    }

    /// Vegetation: bigger than its cell, never part of a wall run, and the
    /// one solid thing a map places that is *supposed* to be green.
    pub fn is_tree(self) -> bool {
        self.sheet() == Sheet::Trees
    }

    /// One of the four wall materials - the only ones that autotile into
    /// runs, so the only ones with an edge cap and a `MATERIALS` slot.
    pub fn is_wall(self) -> bool {
        self.sheet() == Sheet::Walls
    }

    /// Odds an instance of this material is the kind that catches fire when
    /// it dies rather than breaking outright, rolled once per tile at spawn
    /// (`Obstacle::flammable`). Zero draws no RNG, so a map with neither
    /// wood nor trees replays exactly as before either existed.
    pub fn flammable_chance(self) -> f64 {
        match self {
            Material::Wood => tuning().wood_flammable_chance,
            Material::Tree | Material::Pine => tuning().tree_flammable_chance,
            _ => 0.0,
        }
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
            Material::Tree | Material::Pine => Some(tuning().tree_ram_seconds),
            _ => None,
        }
    }

    /// Detonates when destroyed (`Game::apply_blast`).
    pub fn is_explosive(self) -> bool {
        self == Material::Barrel
    }
}

/// The two kinds of oil barrel, which are also its two liveries on
/// `props_sheet.png` (`Obstacle::variant` for a barrel *is* its drum):
/// the red drum with a band is oil and leaves a burning pool, the grey
/// drum with the hazard rim is fuel and goes off harder, launching when a
/// neighbouring blast sets it off. A map may pin one (`kind = "barrel",
/// drum = "oil"`) or leave the roll to spawn.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Drum {
    Oil = 0,
    Fuel = 1,
}

impl Drum {
    pub const ALL: [Drum; 2] = [Drum::Oil, Drum::Fuel];

    pub fn from_variant(variant: i32) -> Drum {
        if variant == 1 { Drum::Fuel } else { Drum::Oil }
    }

    pub fn name(self) -> &'static str {
        match self {
            Drum::Oil => "oil",
            Drum::Fuel => "fuel",
        }
    }

    /// How this kind's fuse compares to `barrel_fuse_seconds`: oil
    /// smoulders, fuel cracks first.
    pub fn fuse_factor(self) -> f32 {
        match self {
            Drum::Oil => tuning().oil_fuse_factor,
            Drum::Fuel => tuning().fuel_fuse_factor,
        }
    }
}

/// A barrel's lit fuse: how long is left, how long it was, and what lit
/// it. `from` is the blast or fire that reached it - `None` never happens
/// today, but a fuse the *cause* of which is unknown is what the field
/// would mean - and is what a chained blast leans away from and a fuel
/// drum launches away from.
#[derive(Clone, Copy, Debug)]
pub struct Fuse {
    pub left: f32,
    pub total: f32,
    pub from: Option<Position>,
}

impl Fuse {
    /// How long this fuse has been burning relative to the shortest one a
    /// blast hands out, 0 at the centre of the blast that lit it to 1 at
    /// its edge. A chained blast grows with it: a drum that smouldered
    /// goes up bigger than one that went at once.
    pub fn smoulder(&self) -> f32 {
        let shortest = tuning().barrel_fuse_seconds * 0.5;
        let longest = tuning().barrel_fuse_seconds * 2.5;
        if longest <= shortest {
            return 0.0;
        }
        ((self.total - shortest) / (longest - shortest)).clamp(0.0, 1.0)
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

/// The three obstacle atlases, bundled so a draw call can pick by
/// `Material::sheet` without the caller matching on the material.
pub struct ObstacleTextures<'a> {
    pub walls: &'a Texture2D,
    pub props: &'a Texture2D,
    pub trees: &'a Texture2D,
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
    /// Wood and trees (always false elsewhere): whether this instance
    /// catches fire when destroyed instead of breaking outright - rolled
    /// once at spawn from `Material::flammable_chance`, per
    /// docs/WALLS_SPEC.md's framing of "breaks easily" vs "catches fire" as
    /// gameplay data layered on shared art, not a separate art asset.
    pub flammable: bool,
    /// True from the moment a flammable tile's health hits zero until it
    /// finishes charring (see `tick_burn`) - during this window `damage`
    /// is a no-op (already on fire) and `col` shows the 3-frame burn loop
    /// instead of a damage stage.
    pub burning: bool,
    /// Which of the burn loop's 3 frames (cols 4-6) is showing.
    pub burn_frame: i32,
    /// Seconds since `burn_frame` last advanced.
    pub burn_frame_timer: f32,
    /// Total seconds spent burning so far - once this passes
    /// `wood_burn_seconds`, `tick_burn` chars it out (`destroyed = true`).
    /// Trees share wood's burn timing: it is one fire, and splitting the
    /// knob would only be worth it if they were meant to burn differently.
    pub burn_elapsed: f32,
    /// Barrel only: the lit fuse, armed when a neighbouring blast or a
    /// burning cell reached it (`Game::apply_blast`, `tick_fires`) so a
    /// chain reaction cascades visibly instead of going off all at once.
    /// While armed the barrel is inert to further damage, like burning
    /// Wood, and draws its lit-fuse column.
    pub fuse: Option<Fuse>,
    /// Seconds of flame exposure (`Game::resolve_flames`), fading at
    /// `flame_heat_decay` when the stream is elsewhere. What catches, and
    /// when, is read off this: wood and drums at `flame_ignite_seconds`,
    /// a sandbag at `flame_sandbag_seconds`, a fence at
    /// `flame_fence_seconds`.
    pub heat: f32,
    /// Walls only: which faces (N E S W as bits 0..3) a blast has hit,
    /// drawn blackened over the edge cap so a corridor a cascade ran down
    /// carries the mark. Never cleared - soot does not wash off mid-round.
    pub scorched: u8,
    /// Sandbag/Fence only: seconds a tank has been pushing into this tile
    /// (`Game::ram_props`); decays while nothing pushes. Collapses at
    /// `Material::ram_seconds`.
    pub ram_timer: f32,
    /// Cached 8-bit neighbour mask, for the edge-cap overlay. Wall layouts
    /// only ever lose tiles, never gain them, so this is filled once at
    /// spawn and refreshed when something is destroyed - never rebuilt per
    /// frame the way `fence_axis` does it.
    pub edge_mask: u8,
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
            heat: 0.0,
            scorched: 0,
            ram_timer: 0.0,
            edge_mask: 0,
            body,
            destroyed: false,
        }
    }

    /// Which kind of drum a barrel is; `None` for anything else.
    pub fn drum(&self) -> Option<Drum> {
        (self.material == Material::Barrel).then(|| Drum::from_variant(self.variant))
    }

    /// Sideways draw offset (px) of a drum whose fuse is lit: it rocks at
    /// 12 Hz, a whole 2px block either way, phased by its position hash so
    /// a cluster does not rock in unison. Zero for anything unlit.
    pub fn fuse_rock(&self, time: f32) -> f32 {
        if self.fuse.is_none() {
            return 0.0;
        }
        let amp = tuning().barrel_fuse_rock_px;
        if amp <= 0.0 {
            return 0.0;
        }
        let phase = (crate::blast::seed_at(self.position, 5) % 100) as f32 / 100.0 * std::f32::consts::TAU;
        let wave = (time * 12.0 * std::f32::consts::TAU + phase).sin();
        ((wave * amp) / 2.0).round() * 2.0
    }

    /// Side length of the *cell* this obstacle occupies - its collider,
    /// its nav-grid footprint, its place in the map. One grid cell for
    /// everything, trees included.
    pub fn size(&self) -> f32 {
        OBSTACLE_TEXTURE_SIZE * OBSTACLE_SCALE
    }

    /// Side length of the drawn sprite, which is `size()` for everything
    /// but a tree: a tree's 48px cell overhangs its 32px footprint by 8px
    /// on each side, so canopies interlock instead of tiling. Never use
    /// this for physics or grid maths - that is what `size()` is for.
    pub fn sprite_size(&self) -> f32 {
        self.material.sheet().cell() * OBSTACLE_SCALE
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
        self.damage_stage()
    }

    /// Which of `Material::visible_stages` this obstacle's health puts it
    /// in. Split out of `col` because trees lay their stages out
    /// differently (see `tree_col`) but derive them the same way.
    pub(crate) fn damage_stage(&self) -> i32 {
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
    /// A flammable tile returns `false` the frame it ignites too - it only
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
        if self.flammable {
            self.burning = true;
            return false;
        }
        self.destroyed = true;
        true
    }

    /// Advance a burning tile's fire - a no-op for anything not alight.
    /// Cosmetic only (see docs/WALLS_SPEC.md's fire
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
pub fn icon_source_rec(material: Material) -> (Sheet, Rectangle) {
    let sheet = material.sheet();
    (sheet, source_rec(sheet, material.row_base(), 0))
}

/// Source rectangle for the cell at (row, col) inside `sheet` - the atlases
/// differ only in cell size (`Sheet::cell`).
fn source_rec(sheet: Sheet, row: i32, col: i32) -> Rectangle {
    let cell = sheet.cell();
    Rectangle::new(col as f32 * cell, row as f32 * cell, cell, cell)
}

/// Source rectangle for a drum kind's pristine cell, for the builder's
/// icons and canvas (a pinned drum is drawn as what it will be).
pub fn drum_source_rec(drum: Drum) -> Rectangle {
    source_rec(Sheet::Props, Material::Barrel.row_base() + drum as i32, 0)
}

/// Source rectangle for an oil-trail ground cell (`PROPS_OIL_ROW`), the
/// variant picked from the cell's position hash.
pub fn oil_source_rec(center: Position) -> Rectangle {
    let variant = (crate::blast::seed_at(center, 8) % PROPS_OIL_VARIANTS as u32) as i32;
    source_rec(Sheet::Props, PROPS_OIL_ROW, variant)
}

/// An unlit oil-trail cell: a dark puddle on the ground, drawn under
/// everything that stands. Not an obstacle, so it never goes through
/// `draw_obstacle`.
pub fn draw_oil_cell(d: &mut impl RaylibDraw, textures: &ObstacleTextures, center: Position) {
    let src = oil_source_rec(center);
    let size = OBSTACLE_TEXTURE_SIZE * OBSTACLE_SCALE;
    let dest = Rectangle::new(center.x, center.y, size, size);
    d.draw_texture_pro(textures.props, src, dest, Vector2::new(size / 2.0, size / 2.0), 0.0, Color::WHITE);
}

/// A launched fuel drum in the air (`simulation::FlyingDrum`): the intact
/// drum sprite tumbling in quarter-turns along its arc, over a shadow
/// that shrinks as it rises - the same trick a thrown decal uses.
pub fn draw_flying_drum(
    d: &mut impl RaylibDraw,
    textures: &ObstacleTextures,
    drum: &crate::simulation::FlyingDrum,
    shadows: bool,
) {
    let size = OBSTACLE_TEXTURE_SIZE * OBSTACLE_SCALE;
    if shadows {
        let ground = drum.ground_pos();
        let lift = (drum.height() / (tuning().debris_arc_height * 1.4).max(1e-3)).clamp(0.0, 1.0);
        let r = size * 0.3 * (1.0 - 0.45 * lift);
        let a = (255.0 * tuning().obstacle_shadow_opacity * (1.0 - 0.4 * lift)) as u8;
        d.draw_circle_v(ground, r, Color::new(0, 0, 0, a));
    }
    let src = source_rec(Sheet::Props, Material::Barrel.row_base() + drum.variant, 0);
    let at = drum.draw_pos();
    let dest = Rectangle::new(at.x, at.y, size, size);
    let rotation = ((drum.flight() * 6.0) as i32 % 4) as f32 * 90.0;
    d.draw_texture_pro(textures.props, src, dest, Vector2::new(size / 2.0, size / 2.0), rotation, Color::WHITE);
}

pub(crate) fn texture_for<'a>(textures: &ObstacleTextures<'a>, sheet: Sheet) -> &'a Texture2D {
    match sheet {
        Sheet::Walls => textures.walls,
        Sheet::Props => textures.props,
        Sheet::Trees => textures.trees,
    }
}

/// Draw a single obstacle sprite from its atlas at its center position.
/// Obstacles never rotate (unlike tanks/shells), so this skips the
/// rotation param `draw_tank` needs; `axis` only matters for fences.
pub fn draw_obstacle(d: &mut impl RaylibDraw, textures: &ObstacleTextures, obstacle: &Obstacle, axis: FenceAxis, time: f32) {
    let sheet = obstacle.material.sheet();
    let src = source_rec(sheet, obstacle.row(axis), obstacle.col());
    let size = obstacle.sprite_size();
    let dest = Rectangle::new(obstacle.position.x + obstacle.fuse_rock(time), obstacle.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);
    d.draw_texture_pro(texture_for(textures, sheet), src, dest, origin, 0.0, Color::WHITE);
}

/// Which of a cell's 16 neighbour combinations to draw a cap for.
///
/// The Rust side computes a full **8-bit** mask (N NE E SE S SW W NW, with
/// a diagonal only counting when both its adjacent orthogonals are set -
/// the standard blob rule) and looks it up here. Today every entry is a
/// 4-bit value in `0..=15`, i.e. the diagonals are ignored, because at
/// 16x16 design pixels the only thing the extra 31 blob tiles buy is an
/// inner-corner notch one or two design pixels wide - and these maps are
/// hand-authored rectangles and L-corners where diagonal-only adjacency
/// barely occurs.
///
/// Keeping the 8-bit mask and this table is what makes that decision
/// cheap to revisit: widening to the full 47-tile set means adding columns
/// to the sheet and changing these values, with no other code moving.
static BLOB_TILE: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut m = 0usize;
    while m < 256 {
        // bit3=N bit2=E bit1=S bit0=W, the same order ground.rs's
        // ROAD_EDGE uses, so the two autotilers read alike.
        let n = (m & 0b0000_0001) != 0;
        let e = (m & 0b0000_0100) != 0;
        let s = (m & 0b0001_0000) != 0;
        let w = (m & 0b0100_0000) != 0;
        table[m] = ((n as u8) << 3) | ((e as u8) << 2) | ((s as u8) << 1) | (w as u8);
        m += 1;
    }
    table
};

/// The 8-bit neighbour mask for `cell` against a set of solid wall cells.
/// Bit order N NE E SE S SW W NW; a diagonal is only set when both of its
/// adjacent orthogonals are, which is what collapses 256 combinations to
/// the blob set's 47.
pub fn neighbour_mask(cell: (i32, i32), cells: &HashSet<(i32, i32)>) -> u8 {
    let (c, r) = cell;
    let at = |dc: i32, dr: i32| cells.contains(&(c + dc, r + dr));
    let (n, e, s, w) = (at(0, -1), at(1, 0), at(0, 1), at(-1, 0));
    let mut mask = 0u8;
    if n { mask |= 0b0000_0001 }
    if n && e && at(1, -1) { mask |= 0b0000_0010 }
    if e { mask |= 0b0000_0100 }
    if s && e && at(1, 1) { mask |= 0b0000_1000 }
    if s { mask |= 0b0001_0000 }
    if s && w && at(-1, 1) { mask |= 0b0010_0000 }
    if w { mask |= 0b0100_0000 }
    if n && w && at(-1, -1) { mask |= 0b1000_0000 }
    mask
}

/// Draw the edge-cap overlay for a wall tile: the lighting along whichever
/// faces are exposed to open ground. Composites over the tile already
/// drawn, so it works for every damage stage and variant. Only walls get
/// one - a sandbag, a fence or a tree is a discrete object, not part of a
/// run.
pub fn draw_obstacle_cap(d: &mut impl RaylibDraw, textures: &ObstacleTextures, obstacle: &Obstacle) {
    if !obstacle.material.is_wall() {
        return;
    }
    let Some(row_offset) = MATERIALS.iter().position(|m| *m == obstacle.material) else {
        return;
    };
    let col = BLOB_TILE[obstacle.edge_mask as usize] as i32;
    let src = source_rec(Sheet::Walls, EDGE_CAP_ROW_BASE + row_offset as i32, col);
    let size = obstacle.size();
    let dest = Rectangle::new(obstacle.position.x, obstacle.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);
    d.draw_texture_pro(textures.walls, src, dest, origin, 0.0, Color::WHITE);
    draw_scorched_faces(d, obstacle);
}

/// Soot on the faces a blast hit (`Obstacle::scorched`): a dark band two
/// blocks deep along each marked face, drawn over the cap. A band rather
/// than a sheet row because it composites with every material, stage and
/// variant the same way the cap itself does.
fn draw_scorched_faces(d: &mut impl RaylibDraw, obstacle: &Obstacle) {
    if obstacle.scorched == 0 {
        return;
    }
    let size = obstacle.size();
    let (left, top) = (obstacle.position.x - size / 2.0, obstacle.position.y - size / 2.0);
    let band = FX_BLOCK * 2.0;
    let soot = Color::new(20, 20, 20, 190);
    let faces = obstacle.scorched;
    if faces & SCORCH_N != 0 {
        d.draw_rectangle(left as i32, top as i32, size as i32, band as i32, soot);
    }
    if faces & SCORCH_E != 0 {
        d.draw_rectangle((left + size - band) as i32, top as i32, band as i32, size as i32, soot);
    }
    if faces & SCORCH_S != 0 {
        d.draw_rectangle(left as i32, (top + size - band) as i32, size as i32, band as i32, soot);
    }
    if faces & SCORCH_W != 0 {
        d.draw_rectangle(left as i32, top as i32, band as i32, size as i32, soot);
    }
}

/// `Obstacle::scorched` bits, one per face.
pub const SCORCH_N: u8 = 1;
pub const SCORCH_E: u8 = 2;
pub const SCORCH_S: u8 = 4;
pub const SCORCH_W: u8 = 8;

/// Which face of the tile at `tile` looks toward `from`: the one a blast
/// there blackens.
pub fn face_toward(tile: Position, from: Position) -> u8 {
    let dx = from.x - tile.x;
    let dy = from.y - tile.y;
    if dx.abs() >= dy.abs() {
        if dx >= 0.0 { SCORCH_E } else { SCORCH_W }
    } else if dy >= 0.0 {
        SCORCH_S
    } else {
        SCORCH_N
    }
}

/// How far this tree's crown is leaning right now, in world px, and which
/// way. Positive is to the right; `tree_blit` turns it into the bend.
///
/// **Only ever non-zero while something is pushing the tree over.** A tree
/// does not sway in the wind here: an ambient bend was built and rejected -
/// what works on a blade of grass reads as wrong on a crown, because a
/// trunk is stiff and the eye knows it. A tree's idle life is the dapple
/// frames on the sheet instead (`tree_col`), where the light moves and the
/// geometry does not.
///
/// What is left is the lean under a ram. `ram_timer` is already exactly
/// "how long something has been leaning on this", so the bend grows with
/// its square and the tree is well over by the time it goes - the
/// alternative, a tree standing bolt upright until it vanishes, is what
/// made ramming one read as nothing happening. Direction comes from the
/// nearest live tank, which while `ram_timer` is running is the one doing
/// the pushing (nothing else can accumulate it).
pub fn tree_lean(obstacle: &Obstacle, movers: &[Position]) -> f32 {
    let t = tuning();
    let seed = crate::blast::seed_at(obstacle.position, 41);
    let mut lean = 0.0;

    let limit = obstacle.material.ram_seconds().unwrap_or(0.0);
    if obstacle.ram_timer > 0.0 && limit > 0.0 {
        let push = (obstacle.ram_timer / limit).clamp(0.0, 1.0);
        if let Some(m) = movers
            .iter()
            .min_by(|a, b| a.distance_to(obstacle.position).total_cmp(&b.distance_to(obstacle.position)))
        {
            let dx = obstacle.position.x - m.x;
            let dy = obstacle.position.y - m.y;
            let d = (dx * dx + dy * dy).sqrt();
            // A tank pushing straight along the trunk's axis has no
            // sideways component to give, and a tree shoved from directly
            // below still has to visibly give way - so fall back to a side
            // picked from the tree's own hash, which holds still for the
            // whole push.
            let away = if d > 0.001 && dx.abs() > 0.5 {
                dx / d
            } else if seed & 1 == 0 {
                1.0
            } else {
                -1.0
            };
            lean += away * t.tree_lean_px * push * push;
        }
    }
    lean
}

/// Which sheet column a tree draws from right now - **the whole of a
/// tree's idle animation.**
///
/// Every damage stage exists in `TREE_SHIMMER_FRAMES` copies that differ
/// only in where a few patches of canopy step one rung up the foliage
/// ramp, so cycling them is light moving through the leaves with nothing
/// moving. `tree_dapple_seconds` sets the rate and the tree's own position
/// hash sets its phase, so a wood shimmers out of step with itself rather
/// than blinking as one.
pub fn tree_col(obstacle: &Obstacle, time: f32) -> i32 {
    if obstacle.burning {
        return TREE_BURN_COL + obstacle.burn_frame;
    }
    let seed = crate::blast::seed_at(obstacle.position, 41);
    let step = (time / tuning().tree_dapple_seconds.max(0.05)) as i64;
    let frame = (step + (seed % TREE_SHIMMER_FRAMES as u32) as i64).rem_euclid(TREE_SHIMMER_FRAMES as i64);
    frame as i32 * TREE_STAGES + obstacle.damage_stage()
}

/// Draw a tree. Split from `draw_obstacle` because a tree has its own
/// column layout (`tree_col`) and because it is the one thing on the
/// battlefield that can bend.
pub fn draw_tree(d: &mut impl RaylibDraw, textures: &ObstacleTextures, obstacle: &Obstacle, lean: f32, time: f32) {
    let sheet = obstacle.material.sheet();
    let src = source_rec(sheet, obstacle.row(FenceAxis::Horizontal), tree_col(obstacle, time));
    let size = obstacle.sprite_size();
    tree_blit(d, texture_for(textures, sheet), src, obstacle.position, size, lean, Color::WHITE);
}

/// The same lean applied to the drop shadow, so a bending crown does not
/// slide out of its own shadow. Must be called before `draw_tree`.
pub fn draw_tree_shadow(d: &mut impl RaylibDraw, textures: &ObstacleTextures, obstacle: &Obstacle, lean: f32, time: f32) {
    let sheet = obstacle.material.sheet();
    let src = source_rec(sheet, obstacle.row(FenceAxis::Horizontal), tree_col(obstacle, time));
    let size = obstacle.sprite_size();
    let at = Position::new(
        obstacle.position.x + tuning().shadow_dir_x * tuning().obstacle_shadow_offset,
        obstacle.position.y + tuning().shadow_dir_y * tuning().obstacle_shadow_offset,
    );
    let shadow = Color::new(0, 0, 0, (255.0 * tuning().obstacle_shadow_opacity) as u8);
    tree_blit(d, texture_for(textures, sheet), src, at, size, lean, shadow);
}

/// How many horizontal slices a leaning tree is drawn in.
///
/// Eight over 48px is a 6px band - fine enough that the bend reads as a
/// curve rather than as two halves sliding, coarse enough that a wood
/// costs eight blits a tree instead of one.
const TREE_SWAY_BANDS: i32 = 8;

/// One leaning blit, as a stack of horizontal bands each shifted sideways
/// by a whole 2px block. `lean` is zero for a tree nothing is pushing, and
/// then this is a single ordinary quad.
///
/// **Not a rotation.** Rotating the sprite about its base is what grass
/// does and what this was first; on a crown it resamples the interior
/// every frame, and measured against a static capture the whole inside of
/// every canopy churned, not just its outline. Bands translate rigidly
/// instead, so nothing inside a band can crawl, and snapping the shift to
/// `2.0` keeps every pixel on the same block grid the rest of the game
/// draws on (the rule `fx.rs` and the camera shake already follow).
///
/// The shift ramps as the *square* of the height up the sprite, so the
/// trunk holds still and the crown is what gives way.
fn tree_blit(
    d: &mut impl RaylibDraw,
    texture: &Texture2D,
    src: Rectangle,
    center: Position,
    size: f32,
    lean: f32,
    tint: Color,
) {
    let band = size / TREE_SWAY_BANDS as f32;
    let (left, top) = (center.x - size / 2.0, center.y - size / 2.0);
    let shift = |i: i32| {
        let up = 1.0 - (i as f32 * band + band * 0.5) / size;
        (lean * up * up / FX_BLOCK).round() * FX_BLOCK
    };
    // Neighbouring bands usually land on the same block, so a lean that
    // steps 4-2-0 down the sprite costs three blits, not eight. Runs are
    // emitted as one quad each; a still tree is a single quad again.
    let mut start = 0;
    while start < TREE_SWAY_BANDS {
        let dx = shift(start);
        let mut end = start + 1;
        while end < TREE_SWAY_BANDS && shift(end) == dx {
            end += 1;
        }
        let (y, h) = (start as f32 * band, (end - start) as f32 * band);
        d.draw_texture_pro(
            texture,
            Rectangle::new(src.x, src.y + y, src.width, h),
            Rectangle::new(left + dx, top + y, size, h),
            Vector2::zero(),
            0.0,
            tint,
        );
        start = end;
    }
}

/// The screen-pixel block every sprite in the game lands on - see
/// `fx::FX_GRID`, which snaps particles to the same one.
const FX_BLOCK: f32 = 2.0;

/// Draw this obstacle's drop shadow - see `tank::draw_tank_shadow` /
/// docs/sprite-shadows-design.md. Must be called before `draw_obstacle`.
pub fn draw_obstacle_shadow(d: &mut impl RaylibDraw, textures: &ObstacleTextures, obstacle: &Obstacle, axis: FenceAxis) {
    let sheet = obstacle.material.sheet();
    let src = source_rec(sheet, obstacle.row(axis), obstacle.col());
    let size = obstacle.sprite_size();
    let dest = Rectangle::new(
        obstacle.position.x + tuning().shadow_dir_x * tuning().obstacle_shadow_offset,
        obstacle.position.y + tuning().shadow_dir_y * tuning().obstacle_shadow_offset,
        size,
        size,
    );
    let origin = Vector2::new(size / 2.0, size / 2.0);
    let shadow = Color::new(0, 0, 0, (255.0 * tuning().obstacle_shadow_opacity) as u8);
    d.draw_texture_pro(texture_for(textures, sheet), src, dest, origin, 0.0, shadow);
}
