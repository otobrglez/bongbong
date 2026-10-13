//! What a destroyed tile leaves on the ground.
//!
//! A `Decal` is the wall equivalent of `blast::Scorch`: it is created by
//! the simulation the frame a tile dies, persists for the rest of the
//! round (`Game::init` clears them, `DECAL_MAX` caps them, oldest first),
//! and is drawn under everything that stands. Like `Scorch` it draws no
//! RNG - every cosmetic choice comes from `blast::seed_at`, a hash of the
//! tile's own position - so a purely visual field can never shift a
//! seeded replay.
//!
//! The art is the dedicated rubble block at the bottom of
//! `walls_sheet.png` (`RUBBLE_ROW_*`), plus the two tree rows on
//! `trees_sheet.png` - leaf litter is green, and the walls sheet is under
//! the no-green guard, so a decal carries the atlas its row is in. One row
//! per kind of leftover,
//! `RUBBLE_VARIANTS` columns each, coverage ramping from a few scattered
//! chips to a dense pile. The variant is picked per tile from the position
//! hash, and `draw_decal` then mirrors and quarter-turns it, so one
//! material has 8 x 8 apparent forms and a levelled wall does not read as
//! a grid of clones. Only Iron has no rubble row - it never dies.

use crate::math::{Color, Rectangle, Vec2};

use crate::canvas::Canvas;
use crate::obstacle::{Material, Sheet};
use crate::tuning::tuning;
use crate::{Position, RUBBLE_VARIANTS};

/// One leftover on the ground, oldest first in `Game::decals`.
pub struct Decal {
    /// Which atlas `row` is in. Everything but tree litter is on the walls
    /// sheet, which is why `at` defaults to it.
    pub sheet: Sheet,
    /// The rubble row, frozen at death so a later change to
    /// `Material::rubble_row` cannot repaint rubble already on the ground.
    /// Also the only thing drawing needs, which is why no material is
    /// stored: a blown-off tank part has no `Material` at all.
    pub row: i32,
    /// Which of the row's `RUBBLE_VARIANTS` this tile left, from the
    /// position hash - no RNG draw. A dry tree's litter is the same
    /// variant `TREE_DRY_COL` along (`props::tile_rubble`).
    pub col: i32,
    /// Where it comes to rest, and where it is drawn once it has landed.
    pub center: Position,
    /// Where it was thrown from. Equal to `center` for anything that
    /// dropped in place, which is every tile's rubble.
    pub origin: Position,
    /// Peak height of the throw in px, 0 for anything that never flew.
    /// The game looks straight down with no perspective, so "height" is
    /// only a draw-time y-offset plus a shrinking shadow - the same trick
    /// `Bong`'s hop uses.
    pub arc: f32,
    /// Cosmetic seed - mirror and quarter-turn. A position hash, never a
    /// draw from the round RNG.
    pub seed: u32,
    /// Seconds since it appeared; drives the fade-in and, for a `hot`
    /// one, how far it has smouldered. The rubble a tank drives over is
    /// the simulation's own per-cell count (`Game::rubble`), not this
    /// picture.
    pub age: f32,
    /// Fire finished the tile: its charred rubble smoulders for
    /// `ruin_smoulder_seconds` of its `age` (`smoulder.rs`).
    pub hot: bool,
    /// A burnt tree's stump (`Decal::stump`): drawn standing, at full
    /// strength and charred, never mirrored or turned.
    pub upright: bool,
}

impl Decal {
    /// The rubble a tile of `material` leaves at `center`, or `None` for a
    /// material with no rubble row. Drops in place - a wall does not throw
    /// its own bricks anywhere.
    pub fn new(material: Material, center: Position, charred: bool) -> Option<Self> {
        let (sheet, row) = material.rubble_row(charred)?;
        let mut d = Self::at(row, center, 0);
        d.sheet = sheet;
        d.hot = charred;
        Some(d)
    }

    /// The stump a tree of `material` and `variant` that burnt down
    /// leaves standing at `center`: the never-felled `TREE_STUMP_COL` of
    /// its own row, `dry` on a desert map, charred and smouldering like
    /// its litter. `None` for anything but a tree.
    pub fn stump(material: Material, variant: i32, center: Position, dry: bool) -> Option<Self> {
        if !material.is_tree() {
            return None;
        }
        let mut d = Self::at(material.row_base() + variant, center, 13);
        d.sheet = Sheet::Trees;
        d.col = crate::TREE_STUMP_COL + if dry { crate::TREE_DRY_COL } else { 0 };
        d.hot = true;
        d.upright = true;
        Some(d)
    }

    /// A piece thrown from `from` and landing at `to`: same record, but it
    /// arcs there over `debris_flight_seconds` before settling. `salt`
    /// separates the several pieces one explosion throws from the same
    /// point, so each picks its own cell, spin and arc height - all from
    /// the position hash, so a wreck full of parts still draws no RNG.
    pub fn thrown(row: i32, from: Position, to: Position, salt: u32) -> Self {
        let mut d = Self::at(row, to, salt);
        d.origin = from;
        // Vary the arc a little per piece so a burst does not read as one
        // rigid fan.
        d.arc = tuning().debris_arc_height * (0.6 + 0.8 * (((d.seed >> 8) % 100) as f32 / 100.0));
        d
    }

    fn at(row: i32, center: Position, salt: u32) -> Self {
        let seed = crate::blast::seed_at(center, 7 + salt * 3);
        // Salted separately from the mirror/rotation seed so two pieces
        // that happen to share a variant still differ in orientation.
        let col = (crate::blast::seed_at(center, 11 + salt * 3) % RUBBLE_VARIANTS as u32) as i32;
        Decal { sheet: Sheet::Walls, row, col, center, origin: center, arc: 0.0, seed, age: 0.0, hot: false, upright: false }
    }

    /// Pick a landed piece up and throw it again, to `to`: it takes off
    /// from where it lay with a fresh arc, and lands where the caller
    /// (the simulation) says - a blast rearranging the rubble around it.
    pub fn rethrow(&mut self, to: Position) {
        self.origin = self.center;
        self.center = to;
        self.age = 0.0;
        self.arc = tuning().debris_arc_height * (0.5 + 0.6 * (((self.seed >> 4) % 100) as f32 / 100.0));
    }

    /// How far through its throw it is, 1.0 once it has settled.
    pub fn flight(&self) -> f32 {
        if self.arc <= 0.0 {
            return 1.0;
        }
        (self.age / tuning().debris_flight_seconds.max(1e-3)).clamp(0.0, 1.0)
    }

    pub fn landed(&self) -> bool {
        self.flight() >= 1.0
    }

    /// Height above the ground right now - drives the draw offset and the
    /// shadow, and is zero at both ends of the throw.
    pub fn height(&self) -> f32 {
        let t = self.flight();
        self.arc * 4.0 * t * (1.0 - t)
    }

    /// Where to draw it: along the ground from `origin` to `center`, lifted
    /// by `height`.
    pub fn draw_pos(&self) -> Position {
        let t = self.flight();
        Position::new(
            self.origin.x + (self.center.x - self.origin.x) * t,
            self.origin.y + (self.center.y - self.origin.y) * t - self.height(),
        )
    }
}

/// Draw one settled decal. Mirrored and quarter-turned by its seed so a
/// levelled wall doesn't read as a row of clones; quarter-turns keep the
/// pixels square, exactly like `blast::draw_scorch`.
pub fn draw_decal(c: &mut impl Canvas, decal: &Decal) {
    draw_decal_kept(c, decal, 1.0);
}

/// `draw_decal` with `keep` (0 to 1) of its strength, in eighths: what
/// snow lying over it leaves showing (docs/aftermath.md).
pub fn draw_decal_kept(c: &mut impl Canvas, decal: &Decal, keep: f32) {
    let keep = (keep.clamp(0.0, 1.0) * 8.0).round() / 8.0;
    if keep <= 0.0 {
        return;
    }
    if decal.upright {
        draw_stump(c, decal, keep);
        return;
    }
    let cell = decal.sheet.cell();
    let flip = if decal.seed & 1 != 0 { -1.0 } else { 1.0 };
    let src = Rectangle::new(decal.col as f32 * cell, decal.row as f32 * cell, cell * flip, cell);
    let size = cell * crate::OBSTACLE_SCALE;
    let rotation = ((decal.seed >> 1) % 4) as f32 * 90.0;
    // A piece still in the air is drawn solid: it is a lump of debris in
    // flight, not a mark on the ground, and the fade-in only makes sense
    // for something settling.
    let opacity = if decal.landed() {
        let fade = (decal.age / tuning().decal_fade_in_seconds.max(1e-3)).clamp(0.0, 1.0);
        tuning().decal_opacity * fade * keep
    } else {
        1.0
    };
    let tint = Color::new(255, 255, 255, (255.0 * opacity) as u8);
    let at = decal.draw_pos();
    let dest = Rectangle::new(at.x, at.y, size, size);
    c.blit(decal.sheet, src, dest, Vec2::new(size / 2.0, size / 2.0), rotation, tint);
}

/// A burnt tree's stump: its sheet cell standing as drawn, at full
/// strength once faded in, darkened to char.
fn draw_stump(c: &mut impl Canvas, decal: &Decal, keep: f32) {
    let cell = decal.sheet.cell();
    let src = Rectangle::new(decal.col as f32 * cell, decal.row as f32 * cell, cell, cell);
    let size = cell * crate::OBSTACLE_SCALE;
    let fade = (decal.age / tuning().decal_fade_in_seconds.max(1e-3)).clamp(0.0, 1.0);
    let tint = Color::new(STUMP_CHAR.r, STUMP_CHAR.g, STUMP_CHAR.b, (255.0 * fade * keep) as u8);
    let at = decal.center;
    c.blit(decal.sheet, src, Rectangle::new(at.x, at.y, size, size), Vec2::new(size / 2.0, size / 2.0), 0.0, tint);
}

/// What a stump's wood is multiplied by once fire has had it: its browns
/// pulled down toward the `CHAR` ramp.
const STUMP_CHAR: Color = Color::new(0x6A, 0x58, 0x4C, 0xFF);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burnt_tree_leaves_a_standing_stump_and_nothing_else_does() {
        let at = Position::new(64.0, 64.0);
        let stump = Decal::stump(Material::Pine, 2, at, false).expect("a tree leaves a stump");
        assert!(stump.upright && stump.hot);
        assert_eq!(stump.sheet, Sheet::Trees);
        assert_eq!((stump.row, stump.col), (Material::Pine.row_base() + 2, crate::TREE_STUMP_COL));
        assert_eq!(Decal::stump(Material::Pine, 2, at, true).expect("a dry one").col, crate::TREE_STUMP_COL + crate::TREE_DRY_COL);
        assert!(Decal::stump(Material::Brick, 0, at, false).is_none());
        assert!(Decal::new(Material::Wood, at, true).expect("charred planks").hot, "charred rubble smoulders");
        assert!(!Decal::new(Material::Wood, at, false).expect("broken planks").hot, "broken rubble does not");
    }
}
