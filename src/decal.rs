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

use sola_raylib::prelude::*;

use crate::obstacle::{Material, ObstacleTextures, Sheet, texture_for};
use crate::tuning::tuning;
use crate::{OBSTACLE_TEXTURE_SIZE, Position, RUBBLE_VARIANTS};

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
    /// position hash - no RNG draw.
    pub col: i32,
    /// Where it comes to rest, and where it is drawn once it has landed.
    pub center: Position,
    /// Where it was thrown from. Equal to `center` for anything that
    /// dropped in place, which is every tile's rubble.
    pub origin: Position,
    /// Peak height of the throw in px, 0 for anything that never flew.
    /// The game is top-down with no camera, so "height" is only a
    /// draw-time y-offset plus a shrinking shadow - the same trick
    /// `Frog`'s hop uses.
    pub arc: f32,
    /// Cosmetic seed - mirror and quarter-turn. A position hash, never a
    /// draw from the round RNG.
    pub seed: u32,
    /// Seconds since it appeared; drives only the fade-in.
    pub age: f32,
    /// Phase-3 hook: no reader today. It exists so "rubble slows a tank"
    /// can be added by changing the *consumers*, without moving the
    /// decision of where rubble lands out of the simulation.
    pub blocks: bool,
}

impl Decal {
    /// The rubble a tile of `material` leaves at `center`, or `None` for a
    /// material with no rubble row. Drops in place - a wall does not throw
    /// its own bricks anywhere.
    pub fn new(material: Material, center: Position, charred: bool) -> Option<Self> {
        let (sheet, row) = material.rubble_row(charred)?;
        let mut d = Self::at(row, center, 0);
        d.sheet = sheet;
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
        Decal { sheet: Sheet::Walls, row, col, center, origin: center, arc: 0.0, seed, age: 0.0, blocks: false }
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
/// The shadow under a piece still in the air, drawn at the point on the
/// ground it is over. Shrinks as the piece rises, which is what actually
/// sells the height in a game with no camera.
pub fn draw_decal_shadow(d: &mut impl RaylibDraw, decal: &Decal) {
    let h = decal.height();
    if h <= 0.0 {
        return;
    }
    let t = decal.flight();
    let ground = Position::new(
        decal.origin.x + (decal.center.x - decal.origin.x) * t,
        decal.origin.y + (decal.center.y - decal.origin.y) * t,
    );
    let lift = (h / tuning().debris_arc_height.max(1e-3)).clamp(0.0, 1.0);
    let r = OBSTACLE_TEXTURE_SIZE * 0.22 * (1.0 - 0.45 * lift);
    let a = (255.0 * tuning().obstacle_shadow_opacity * (1.0 - 0.4 * lift)) as u8;
    d.draw_circle_v(ground, r, Color::new(0, 0, 0, a));
}

pub fn draw_decal(d: &mut impl RaylibDraw, textures: &ObstacleTextures, decal: &Decal) {
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
        tuning().decal_opacity * fade
    } else {
        1.0
    };
    let tint = Color::new(255, 255, 255, (255.0 * opacity) as u8);
    let at = decal.draw_pos();
    let dest = Rectangle::new(at.x, at.y, size, size);
    d.draw_texture_pro(texture_for(textures, decal.sheet), src, dest, Vector2::new(size / 2.0, size / 2.0), rotation, tint);
}
