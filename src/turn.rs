//! The crisp turn: a sprite drawn at an angle that is not a quarter turn,
//! kept on its own design-pixel grid. Headless and presentation only -
//! the arithmetic `render::turn` hands to `static/turn.vs`/`turn.fs`.
//!
//! A sprite at rest is whole blocks: one design pixel of its sheet, drawn
//! `scale` screen pixels wide. Turned by the GPU's nearest sampling each
//! screen pixel picks its own texel, so the blocks break into single
//! pixels and every edge grows half-size steps. Here the screen is cut
//! into blocks the size a design pixel is drawn, laid from the sprite's
//! pivot - where its own pixels lie at rest - and each block shows the one
//! design pixel under its centre once the turn is undone (`Turned::pixel`),
//! smoothed by Scale2x's corner rule in the shader. Every colour drawn is
//! one the cell holds.
//!
//! A quarter turn never comes here (`is_quarter`): it is drawn by the
//! plain blit, texel for texel what it always was.

use crate::canvas::Sheet;
use crate::math::{Rectangle, Vec2};

/// How many texels of `sheet` make one design pixel, for the sheets whose
/// sprites are turned on their grid; `None` for a sheet that turns the
/// plain way. The tank sheets hold one texel to the design pixel and are
/// drawn at twice the size; the towers are baked at two texels to the
/// design pixel and drawn one to one.
pub fn block(sheet: Sheet) -> Option<f32> {
    match sheet {
        Sheet::Tanks | Sheet::TankGlow | Sheet::TankModules | Sheet::TankModulesGlow => Some(1.0),
        Sheet::Towers => Some(2.0),
        _ => None,
    }
}

/// How far past its cell a turned sprite's quad reaches, design pixels.
pub const MARGIN: f32 = 2.0;

/// Whether `degrees` is a whole number of quarter turns, which the plain
/// blit draws exactly.
pub fn is_quarter(degrees: f32) -> bool {
    degrees % 90.0 == 0.0
}

/// One sprite turned on its grid: the quad to draw and what the shader
/// needs to fill it. Lengths are design pixels unless they say otherwise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turned {
    /// The quad's corners in raylib's order (top-left, bottom-left,
    /// bottom-right, top-right of the unturned sprite): where each stands
    /// in the world, and how far it is from the pivot along the screen's
    /// axes.
    pub corners: [(Vec2, Vec2); 4],
    /// The source cell's corner on its sheet.
    pub cell_corner: Vec2,
    /// The source cell's size.
    pub cell: Vec2,
    /// The pivot, from the cell's corner.
    pub pivot: Vec2,
    /// The turn, clockwise on the y-down screen.
    pub radians: f32,
}

impl Turned {
    /// The quad for a blit of `src` into `dest` about `origin` at
    /// `degrees` (`Canvas::blit`'s arguments) from a sheet of `block`
    /// texels to the design pixel. `None` for what the grid cannot hold -
    /// a mirrored source, a sprite stretched more one way than the other,
    /// an empty one - which the plain blit draws.
    pub fn of(src: Rectangle, dest: Rectangle, origin: Vec2, degrees: f32, block: f32) -> Option<Turned> {
        if !(block > 0.0 && src.width > 0.0 && src.height > 0.0 && dest.width > 0.0 && dest.height > 0.0 && degrees.is_finite()) {
            return None;
        }
        let cell = Vec2::new(src.width / block, src.height / block);
        // Screen pixels to the design pixel.
        let size = dest.width / cell.x;
        if (dest.height / cell.y - size).abs() > 1e-3 * size {
            return None;
        }
        let radians = degrees.to_radians();
        let (sin, cos) = crate::trig::sin_cos(radians);
        // Past the cell on every side: a block whose centre is inside the
        // turned cell reaches out of it, and a sheared pixel lands up to a
        // block and a half from where the true turn puts it.
        let reach = MARGIN * size;
        let (left, top) = (-origin.x - reach, -origin.y - reach);
        let (right, bottom) = (-origin.x + dest.width + reach, -origin.y + dest.height + reach);
        let corner = |x: f32, y: f32| {
            let turned = Vec2::new(x * cos - y * sin, x * sin + y * cos);
            (Vec2::new(dest.x + turned.x, dest.y + turned.y), Vec2::new(turned.x / size, turned.y / size))
        };
        Some(Turned {
            corners: [corner(left, top), corner(left, bottom), corner(right, bottom), corner(right, top)],
            cell_corner: Vec2::new(src.x / block, src.y / block),
            cell,
            pivot: Vec2::new(origin.x / size, origin.y / size),
            radians,
        })
    }

    /// The point of the cell the block holding `at` shows, `at` being a
    /// screen point's distance from the pivot: the block's centre with the
    /// turn undone. Its whole part is the design pixel. The shader's own
    /// arithmetic, kept here for the tests.
    pub fn pixel(&self, at: Vec2) -> Vec2 {
        let lead = Vec2::new(self.pivot.x.fract(), self.pivot.y.fract());
        let block = Vec2::new((at.x + lead.x).floor() + 0.5 - lead.x, (at.y + lead.y).floor() + 0.5 - lead.y);
        let (sin, cos) = crate::trig::sin_cos(self.radians);
        Vec2::new(self.pivot.x + cos * block.x + sin * block.y, self.pivot.y + cos * block.y - sin * block.x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tank(degrees: f32) -> Turned {
        // A 40 px tank cell drawn 80 px about its centre.
        Turned::of(Rectangle::new(80.0, 120.0, 40.0, 40.0), Rectangle::new(300.5, 200.25, 80.0, 80.0), Vec2::new(40.0, 40.0), degrees, 1.0).unwrap()
    }

    #[test]
    fn only_whole_quarter_turns_are_quarters() {
        for q in [0.0, 90.0, -90.0, 180.0, 270.0, 360.0, -0.0] {
            assert!(is_quarter(q), "{q}");
        }
        for a in [12.0, 84.0, 89.999, 0.001, 45.0, 150.0] {
            assert!(!is_quarter(a), "{a}");
        }
    }

    #[test]
    fn an_unturned_block_shows_the_pixel_the_plain_blit_does() {
        let t = tank(0.0);
        assert_eq!(t.cell, Vec2::new(40.0, 40.0));
        assert_eq!(t.pivot, Vec2::new(20.0, 20.0));
        assert_eq!(t.cell_corner, Vec2::new(80.0, 120.0));
        // Every screen pixel of the 80 px sprite: the plain blit shows the
        // texel under the pixel's centre.
        for y in 0..80 {
            for x in 0..80 {
                let at = Vec2::new((x as f32 + 0.5 - 40.0) / 2.0, (y as f32 + 0.5 - 40.0) / 2.0);
                let p = t.pixel(at);
                assert_eq!((p.x.floor(), p.y.floor()), ((x / 2) as f32, (y / 2) as f32));
                // And from its centre, as far from a neighbour as can be.
                assert_eq!((p.x.fract(), p.y.fract()), (0.5, 0.5));
            }
        }
    }

    #[test]
    fn a_block_is_one_design_pixel_at_any_angle() {
        for degrees in [12.0, 24.0, 45.0, 84.0, 150.0, -33.0] {
            let t = tank(degrees);
            // The four screen pixels of a block show one point.
            let block = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)].map(|(x, y)| t.pixel(Vec2::new(3.0 + x, -7.0 + y)));
            assert!(block.iter().all(|p| *p == block[0]), "{degrees}: {block:?}");
            // The pivot's own block stays within a pixel of the pivot.
            let p = t.pixel(Vec2::new(0.25, 0.25));
            assert!((p.x - 20.0).abs() < 1.0 && (p.y - 20.0).abs() < 1.0, "{degrees}: {p:?}");
        }
        // A quarter turn clockwise: the pixel right of the pivot on screen
        // is the one above it in the cell.
        let p = tank(90.0).pixel(Vec2::new(5.5, 0.5));
        assert!((p.x - 20.5).abs() < 1e-4 && (p.y - 14.5).abs() < 1e-4, "{p:?}");
    }

    #[test]
    fn a_pivot_between_pixels_keeps_the_resting_grid() {
        // A 5 px cell about its centre, 2.5 px in: at rest the blocks'
        // edges fall half a design pixel either side of the pivot.
        let t = Turned::of(Rectangle::new(0.0, 0.0, 5.0, 5.0), Rectangle::new(10.0, 10.0, 10.0, 10.0), Vec2::new(5.0, 5.0), 0.0, 1.0).unwrap();
        for x in 0..10 {
            let p = t.pixel(Vec2::new((x as f32 + 0.5 - 5.0) / 2.0, 0.0));
            assert_eq!(p.x.floor(), (x / 2) as f32, "{x}");
        }
    }

    #[test]
    fn the_quad_is_the_turned_sprite_and_its_margin() {
        let t = tank(0.0);
        assert_eq!(t.corners[0], (Vec2::new(300.5 - 44.0, 200.25 - 44.0), Vec2::new(-22.0, -22.0)));
        assert_eq!(t.corners[2], (Vec2::new(300.5 + 44.0, 200.25 + 44.0), Vec2::new(22.0, 22.0)));
        // Turned, each corner is as far from the pivot as before, and what
        // it carries is its place in design pixels.
        let t = tank(30.0);
        for (world, design) in t.corners {
            let (dx, dy) = (world.x - 300.5, world.y - 200.25);
            assert!(((dx * dx + dy * dy).sqrt() - 44.0 * std::f32::consts::SQRT_2).abs() < 1e-3);
            assert!((design.x - dx / 2.0).abs() < 1e-5 && (design.y - dy / 2.0).abs() < 1e-5);
        }
    }

    #[test]
    fn a_baked_sheet_counts_its_design_pixels_in_blocks_of_texels() {
        // A 48 texel tower cell at two texels to the design pixel, drawn
        // one to one.
        let t = Turned::of(Rectangle::new(240.0, 48.0, 48.0, 48.0), Rectangle::new(100.0, 100.0, 48.0, 48.0), Vec2::new(24.0, 24.0), 20.0, 2.0).unwrap();
        assert_eq!((t.cell, t.pivot, t.cell_corner), (Vec2::new(24.0, 24.0), Vec2::new(12.0, 12.0), Vec2::new(120.0, 24.0)));
    }

    #[test]
    fn what_the_grid_cannot_hold_is_left_to_the_plain_blit() {
        let dest = Rectangle::new(0.0, 0.0, 80.0, 80.0);
        let origin = Vec2::new(40.0, 40.0);
        // Mirrored.
        assert_eq!(Turned::of(Rectangle::new(0.0, 0.0, -40.0, 40.0), dest, origin, 10.0, 1.0), None);
        // Stretched one way.
        assert_eq!(Turned::of(Rectangle::new(0.0, 0.0, 40.0, 40.0), Rectangle::new(0.0, 0.0, 80.0, 60.0), origin, 10.0, 1.0), None);
        assert_eq!(Turned::of(Rectangle::new(0.0, 0.0, 40.0, 40.0), dest, origin, f32::NAN, 1.0), None);
    }

    #[test]
    fn only_the_sheets_on_a_whole_grid_turn_on_it() {
        assert_eq!(block(Sheet::Tanks), Some(1.0));
        assert_eq!(block(Sheet::Towers), Some(2.0));
        assert_eq!(block(Sheet::Walls), None);
        assert_eq!(block(Sheet::Aircraft), None);
    }
}
