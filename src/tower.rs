//! The defence towers (docs/defence-towers-prd.md): the tesla coil, the
//! machine-gun tower and bio slush. A tower is an `Obstacle` - a tile for
//! collision, the nav grid, the hit test, damage and death - and what a tile
//! does not have, its weapon, lives here in a `Tower` kept beside it in
//! `Game::towers`. The rules are `simulation/towers.rs`; this module is the
//! state and the drawing that is generic over `canvas::Canvas`.
//!
//! The art is `static/towers_sheet.png` (docs/TOWERS_SPEC.md): per kind and
//! side one row of 48 px cells - four standing stages, the ruin, and the top
//! layer, which the game rotates to the tower's heading the way it rotates a
//! tank's turret.

use hecs::Entity;

use crate::canvas::{Canvas, Sheet};
use crate::frog::Side;
use crate::math::{Color, Rectangle, Vec2};
use crate::obstacle::Material;
use crate::shell::Owner;
use crate::tuning::tuning;
use crate::{Position, TREE_TEXTURE_SIZE};

/// Which of the three towers. The variant order is the wire's
/// (`net::events::WireEvent::TowerFired`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum TowerKind {
    /// Charges while an opposing tank is in reach, then strikes it.
    Tesla,
    /// Tracks and fires minigun bullets in bursts.
    Gun,
    /// Lobs globs of ooze that coat tanks and leave puddles.
    Bio,
}

impl TowerKind {
    pub const ALL: [TowerKind; 3] = [TowerKind::Tesla, TowerKind::Gun, TowerKind::Bio];

    /// The tower a material is, if it is one.
    pub fn from_material(material: Material) -> Option<TowerKind> {
        match material {
            Material::Tesla => Some(TowerKind::Tesla),
            Material::GunTower => Some(TowerKind::Gun),
            Material::BioSlush => Some(TowerKind::Bio),
            _ => None,
        }
    }

    pub fn material(self) -> Material {
        match self {
            TowerKind::Tesla => Material::Tesla,
            TowerKind::Gun => Material::GunTower,
            TowerKind::Bio => Material::BioSlush,
        }
    }

    /// The map's and the tools' spelling.
    pub fn name(self) -> &'static str {
        match self {
            TowerKind::Tesla => "tesla",
            TowerKind::Gun => "gun_tower",
            TowerKind::Bio => "bio_slush",
        }
    }

    /// Inverse of `name`.
    pub fn parse(name: &str) -> Option<TowerKind> {
        TowerKind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// Whether the top layer turns to face a target.
    pub fn turns(self) -> bool {
        self != TowerKind::Tesla
    }

    /// How far it reaches, centre to centre.
    pub fn range(self) -> f32 {
        match self {
            TowerKind::Tesla => tuning().tesla_range,
            TowerKind::Gun => tuning().gun_tower_range,
            TowerKind::Bio => tuning().bio_range,
        }
    }

    fn index(self) -> i32 {
        match self {
            TowerKind::Tesla => 0,
            TowerKind::Gun => 1,
            TowerKind::Bio => 2,
        }
    }
}

/// One tower's weapon: everything about it that is not the tile.
#[derive(Clone, Debug)]
pub struct Tower {
    pub kind: TowerKind,
    pub side: Side,
    /// The `Obstacle` entity it stands on.
    pub entity: Entity,
    pub position: Position,
    /// The top layer's heading in degrees, 0 = up: where the gun and the
    /// mortar point. Turns at the kind's turn rate; the tesla's stays 0.
    pub heading: f32,
    /// The tank it is charging at or tracking.
    pub target: Option<Entity>,
    /// Tesla: 0..=1, a strike at 1. Bio: how far the next glob has
    /// reloaded, drawn as the glow in the mortar's mouth.
    pub charge: f32,
    /// Seconds until it may fire (the gun between bursts, the bio slush
    /// between globs) or charge again (the tesla after a strike).
    pub cooldown: f32,
    /// Gun: bullets left in the burst being fired.
    pub burst_left: u32,
    /// Gun: seconds to the next bullet of the burst.
    pub burst_timer: f32,
    /// On fire: below `tower_burn_below`, or lit by the flamethrower.
    /// Burns `tower_burn_dps` until repaired or dead, and fights at
    /// `tower_burning_fire_factor` of its rate. A plank's `Obstacle::burning`
    /// means it is burning out; a tower's does not, so it is kept here.
    pub burning: bool,
    /// Seconds of flamethrower exposure, like `Obstacle::heat`.
    pub heat: f32,
}

impl Tower {
    pub fn new(kind: TowerKind, side: Side, entity: Entity, position: Position) -> Tower {
        Tower {
            kind,
            side,
            entity,
            position,
            heading: 0.0,
            target: None,
            charge: 0.0,
            cooldown: 0.0,
            burst_left: 0,
            burst_timer: 0.0,
            burning: false,
            heat: 0.0,
        }
    }

    /// Who the tower's shots belong to.
    pub fn owner(&self, cell: u16) -> Owner {
        Owner::Tower { side: self.side, cell }
    }

    /// Whether a tank of `owner`'s is one this tower fights.
    pub fn opposes(&self, owner: Owner) -> bool {
        match self.side {
            Side::Player => !owner.is_player(),
            Side::Enemy => owner.is_player(),
        }
    }

    /// The rate multiplier fire puts on it.
    pub fn fire_factor(&self) -> f32 {
        if self.burning { tuning().tower_burning_fire_factor } else { 1.0 }
    }
}

/// A standing tower as the picture needs it (`Game::tower_views`).
#[derive(Clone, Copy, Debug)]
pub struct TowerView {
    pub kind: TowerKind,
    pub side: Side,
    pub position: Position,
    /// 0..=3, the standing looks by health.
    pub stage: i32,
    pub heading: f32,
    /// The tesla's charge, the mortar's reload: 0..=1.
    pub charge: f32,
    pub burning: bool,
}

/// A glob of ooze in the air (the bio slush's shot): a ground point moving
/// from `from` to `to` over `flight` seconds, drawn lifted by `height()`
/// over its own shadow, the flying drum's convention. No hit test in flight:
/// it lands where it was aimed.
#[derive(Clone, Debug)]
pub struct Glob {
    pub id: u32,
    pub from: Position,
    pub to: Position,
    pub age: f32,
    pub flight: f32,
    pub apex: f32,
    pub owner: Owner,
}

impl Glob {
    /// 0 at the muzzle, 1 on landing.
    pub fn progress(&self) -> f32 {
        if self.flight <= 0.0 { 1.0 } else { (self.age / self.flight).clamp(0.0, 1.0) }
    }

    pub fn ground_pos(&self) -> Position {
        let u = self.progress();
        Position::new(self.from.x + (self.to.x - self.from.x) * u, self.from.y + (self.to.y - self.from.y) * u)
    }

    /// Drawn height above the ground point: a parabola peaking at `apex`,
    /// starting from the muzzle's own few pixels.
    pub fn height(&self) -> f32 {
        let u = self.progress();
        GLOB_MUZZLE_HEIGHT * (1.0 - u) + 4.0 * self.apex * u * (1.0 - u)
    }

    pub fn landed(&self) -> bool {
        self.age >= self.flight
    }
}

/// How high the mortar's mouth sits above the ground, so a glob leaves it
/// rather than the ground under it.
pub const GLOB_MUZZLE_HEIGHT: f32 = 6.0;

/// One cell of ooze on the ground: slimes whatever drives over it until it
/// dries or burns away (`simulation/towers.rs`).
#[derive(Clone, Copy, Debug)]
pub struct OozePuddle {
    pub left: f32,
    pub total: f32,
}

impl OozePuddle {
    /// 1 when fresh, 0 when dry: what the drawing fades by.
    pub fn freshness(&self) -> f32 {
        if self.total <= 0.0 { 0.0 } else { (self.left / self.total).clamp(0.0, 1.0) }
    }
}

/// A tesla bolt in its short display window. Presentation only: the strike
/// was applied the frame it was made.
#[derive(Clone, Copy, Debug)]
pub struct TeslaBolt {
    pub start: Position,
    pub end: Position,
    pub timer: f32,
    /// Hashed from the tower's cell and the frame: which way the filaments
    /// jag, with no RNG.
    pub seed: u32,
}

impl TeslaBolt {
    pub fn new(start: Position, end: Position, seed: u32) -> TeslaBolt {
        TeslaBolt { start, end, timer: tuning().tesla_bolt_display_seconds, seed }
    }

    /// Age it by `dt`; true once it should go.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.timer -= dt;
        self.timer <= 0.0
    }

    /// 1 when fresh, fading to 0.
    pub fn alpha(&self) -> f32 {
        let total = tuning().tesla_bolt_display_seconds.max(1e-3);
        (self.timer / total).clamp(0.0, 1.0)
    }
}

/// What a dead tower leaves: its ruin art on the ground where it stood, a
/// decal that smoulders for `tower_ruin_smoke_seconds`.
#[derive(Clone, Copy, Debug)]
pub struct TowerRuin {
    pub kind: TowerKind,
    pub side: Side,
    pub position: Position,
    pub age: f32,
}

impl TowerRuin {
    pub fn smouldering(&self) -> bool {
        self.age < tuning().tower_ruin_smoke_seconds
    }
}

// --- the sheet ---------------------------------------------------------

/// Columns of towers_sheet.png: the four standing stages, then the ruin,
/// then the top layer intact and damaged, then the glow overlay (the
/// tesla's lit lens and prong tips, the mortar's loaded mouth).
pub const TOWER_STAGE_COLS: i32 = 4;
pub const TOWER_RUIN_COL: i32 = 4;
pub const TOWER_TOP_COL: i32 = 5;
pub const TOWER_TOP_DAMAGED_COL: i32 = 6;
pub const TOWER_GLOW_COL: i32 = 7;

/// A tower tile's `Obstacle::variant`: the side it fights for, so the tile
/// alone says whose it is (`side_of_variant` reads it back).
pub fn side_variant(side: Side) -> i32 {
    if side == Side::Enemy { 1 } else { 0 }
}

pub fn side_of_variant(variant: i32) -> Side {
    if variant == 1 { Side::Enemy } else { Side::Player }
}

/// The sheet row for a kind and side: kinds in `TowerKind::ALL` order, the
/// player's livery then the enemy's.
pub fn sheet_row(kind: TowerKind, side: Side) -> i32 {
    kind.index() * 2 + if side == Side::Enemy { 1 } else { 0 }
}

fn cell_rec(row: i32, col: i32) -> Rectangle {
    let s = TREE_TEXTURE_SIZE;
    Rectangle::new(col as f32 * s, row as f32 * s, s, s)
}

/// The 48 px pristine base of a kind and side - the builder's icon.
pub fn icon_source_rec(kind: TowerKind, side: Side) -> Rectangle {
    cell_rec(sheet_row(kind, side), 0)
}

/// The top layer of a kind and side, intact - the builder draws it over
/// the icon, pointing up.
pub fn icon_top_rec(kind: TowerKind, side: Side) -> Rectangle {
    cell_rec(sheet_row(kind, side), TOWER_TOP_COL)
}

/// Blit one 48 px cell centred on `at`, rotated by `rotation` degrees.
fn blit_cell(c: &mut impl Canvas, rec: Rectangle, at: Position, rotation: f32, tint: Color) {
    let s = TREE_TEXTURE_SIZE;
    c.blit(Sheet::Towers, rec, Rectangle::new(at.x, at.y, s, s), Vec2::new(s / 2.0, s / 2.0), rotation, tint);
}

/// A standing tower's shadow: its base in black at the obstacles' shadow
/// opacity, offset along the shadow direction.
pub fn draw_tower_shadow(c: &mut impl Canvas, kind: TowerKind, side: Side, at: Position) {
    let t = tuning();
    let off = t.obstacle_shadow_offset + 1.0;
    let a = (255.0 * t.obstacle_shadow_opacity) as u8;
    let at = Position::new(at.x + t.shadow_dir_x * off * 1.4, at.y + t.shadow_dir_y * off * 1.4);
    blit_cell(c, cell_rec(sheet_row(kind, side), 0), at, 0.0, Color::new(0, 0, 0, a));
}

/// A standing tower: its base at `stage` (0..=3), then the top layer turned
/// to `heading`, then - for the tesla and the mortar - the glow overlay at
/// `glow` (0..=1).
pub fn draw_tower(c: &mut impl Canvas, kind: TowerKind, side: Side, at: Position, stage: i32, heading: f32, glow: f32) {
    let row = sheet_row(kind, side);
    blit_cell(c, cell_rec(row, stage.clamp(0, TOWER_STAGE_COLS - 1)), at, 0.0, Color::WHITE);
    let top = if stage >= 2 { TOWER_TOP_DAMAGED_COL } else { TOWER_TOP_COL };
    let rotation = if kind.turns() { heading } else { 0.0 };
    blit_cell(c, cell_rec(row, top), at, rotation, Color::WHITE);
    if glow > 0.0 && kind != TowerKind::Gun {
        let a = (255.0 * glow.clamp(0.0, 1.0)) as u8;
        blit_cell(c, cell_rec(row, TOWER_GLOW_COL), at, rotation, Color::new(255, 255, 255, a));
    }
}

/// A dead tower's ruin on the ground.
pub fn draw_ruin(c: &mut impl Canvas, ruin: &TowerRuin) {
    blit_cell(c, cell_rec(sheet_row(ruin.kind, ruin.side), TOWER_RUIN_COL), ruin.position, 0.0, Color::WHITE);
}

/// Which of the four standing looks a tower's health puts it in - the
/// same rule every tile's damage stage follows.
pub fn stage_of(health: f32, max_health: f32) -> i32 {
    let frac = if max_health > 0.0 { (health / max_health).clamp(0.0, 1.0) } else { 0.0 };
    (((1.0 - frac) * TOWER_STAGE_COLS as f32) as i32).clamp(0, TOWER_STAGE_COLS - 1)
}

/// The ooze colours (docs/defence-towers-prd.md section 12, the acid lime
/// pick): deliberately off the Puny palette, like plasma.png - the one
/// thing on the field allowed to be green, which it earns by glowing.
pub const OOZE_HI: Color = Color::new(0xf4, 0xff, 0xc4, 255);
pub const OOZE_LT: Color = Color::new(0xc8, 0xff, 0x4d, 255);
pub const OOZE_MD: Color = Color::new(0x93, 0xe2, 0x3d, 255);
pub const OOZE_DK: Color = Color::new(0x52, 0xa9, 0x2f, 255);

/// A 32-bit position hash for the puddle art, the `blast::seed_at` way.
fn cell_hash(x: i32, y: i32, salt: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d) ^ (y as u32).wrapping_mul(0x1656_67b1) ^ salt.wrapping_mul(0x9e37_79b9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h
}

/// One cell of ooze on the ground, in 2 px blocks: a lumpy edge hashed from
/// the cell, a darker rim, a lit patch and bubbles that swell and pop on a
/// hashed clock. `time` is the round clock; `fade` 0..=1 thins it as it
/// dries.
pub fn draw_puddle(c: &mut impl Canvas, cell: (i32, i32), time: f32, fade: f32) {
    let size = crate::OBSTACLE_GRID_SIZE;
    // A map cell is centred on its multiple of the grid size.
    let x0 = cell.0 as f32 * size - size / 2.0;
    let y0 = cell.1 as f32 * size - size / 2.0;
    let a = (230.0 * fade.clamp(0.0, 1.0)) as u8;
    if a == 0 {
        return;
    }
    let with = |col: Color| Color::new(col.r, col.g, col.b, a);
    let blocks = (size / 2.0) as i32;
    let r = blocks as f32 / 2.0;
    for by in 0..blocks {
        for bx in 0..blocks {
            let dx = bx as f32 + 0.5 - r;
            let dy = by as f32 + 0.5 - r;
            let ang = dy.atan2(dx);
            let lobe = cell_hash(cell.0 * 7 + ((ang + std::f32::consts::PI) * 1.6) as i32, cell.1, 3) % 100;
            let edge = r * (0.78 + 0.3 * lobe as f32 / 100.0);
            let d = (dx * dx + dy * dy).sqrt();
            if d > edge {
                continue;
            }
            let col = if d > edge - 1.2 {
                OOZE_DK
            } else if dx + dy < -r * 0.3 && d < edge * 0.7 {
                OOZE_LT
            } else {
                OOZE_MD
            };
            c.fill_rect(x0 as i32 + bx * 2, y0 as i32 + by * 2, 2, 2, with(col));
        }
    }
    for k in 0..2u32 {
        let h = cell_hash(cell.0, cell.1, 11 + k);
        let phase = (time * 0.9 + (h % 1000) as f32 / 1000.0).fract();
        let bx = 3 + (h >> 10) as i32 % (blocks - 6);
        let by = 3 + (h >> 20) as i32 % (blocks - 6);
        let mut dot = |ox: i32, oy: i32| {
            c.fill_rect(x0 as i32 + (bx + ox) * 2, y0 as i32 + (by + oy) * 2, 2, 2, with(OOZE_HI));
        };
        if phase < 0.72 {
            dot(0, 0);
        } else if phase < 0.84 {
            for (ox, oy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                dot(ox, oy);
            }
        }
    }
}

/// A glob in flight: a dark blot on the ground under it, then the blob
/// lifted by its height.
pub fn draw_glob(c: &mut impl Canvas, glob: &Glob) {
    let ground = glob.ground_pos();
    let snap = |v: f32| ((v / 2.0).round() * 2.0) as i32;
    c.fill_rect(snap(ground.x) - 4, snap(ground.y) - 2, 8, 4, Color::new(0, 0, 0, 70));
    let x = snap(ground.x);
    let y = snap(ground.y - glob.height());
    c.fill_rect(x - 4, y - 2, 8, 4, OOZE_DK);
    c.fill_rect(x - 2, y - 4, 4, 8, OOZE_DK);
    c.fill_rect(x - 2, y - 2, 4, 4, OOZE_MD);
    c.fill_rect(x - 2, y - 4, 2, 2, OOZE_LT);
    c.fill_rect(x - 4, y - 2, 2, 2, OOZE_HI);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_names_its_material_and_back() {
        for kind in TowerKind::ALL {
            assert_eq!(TowerKind::from_material(kind.material()), Some(kind));
            assert!(kind.material().is_tower());
        }
        assert_eq!(TowerKind::from_material(Material::Brick), None);
    }

    #[test]
    fn each_kind_and_side_has_its_own_row() {
        let mut rows: Vec<i32> = TowerKind::ALL
            .iter()
            .flat_map(|&k| [sheet_row(k, Side::Player), sheet_row(k, Side::Enemy)])
            .collect();
        rows.sort_unstable();
        rows.dedup();
        assert_eq!(rows, (0..6).collect::<Vec<_>>());
    }

    #[test]
    fn a_glob_arcs_from_its_muzzle_to_where_it_was_aimed() {
        let mut glob = Glob {
            id: 1,
            from: Position::new(0.0, 0.0),
            to: Position::new(100.0, 0.0),
            age: 0.0,
            flight: 1.0,
            apex: 20.0,
            owner: Owner::Tower { side: Side::Player, cell: 0 },
        };
        assert_eq!(glob.height(), GLOB_MUZZLE_HEIGHT);
        glob.age = 0.5;
        assert!(glob.height() > 20.0);
        assert_eq!(glob.ground_pos(), Position::new(50.0, 0.0));
        glob.age = 1.0;
        assert!(glob.landed());
        assert_eq!(glob.height(), 0.0);
        assert_eq!(glob.ground_pos(), Position::new(100.0, 0.0));
    }

    #[test]
    fn the_stage_follows_health_and_never_reaches_the_ruin() {
        assert_eq!(stage_of(100.0, 100.0), 0);
        assert_eq!(stage_of(74.0, 100.0), 1);
        assert_eq!(stage_of(40.0, 100.0), 2);
        assert_eq!(stage_of(10.0, 100.0), 3);
        assert_eq!(stage_of(0.0, 100.0), 3);
    }
}
