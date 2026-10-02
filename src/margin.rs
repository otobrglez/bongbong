//! Arenas draw their margins (docs/large-maps-follow-camera.md §10, §13
//! item 7). An arena - a map shown whole (`framing::MapClass::Arena`) - is
//! fitted into the window with its shape kept (`view::View`), and where the
//! window's shape differs from the field's, the rest of the window shows
//! the world past the field's boundary rather than flat bars: the round's
//! own ground carried on out of the field (`ground::GroundGrid::beyond` -
//! the sand drifting on, water painted to the edge running out of the
//! picture, a road ending at the boundary) under the field's edge shade
//! carried on and deepened to a dark plateau (`ground::bake_margin_shade`),
//! so it reads as off the playfield. That is the platforms' own advice -
//! Google Play's Level Up guidelines and Apple's both ask a game not to
//! letterbox - and with the HUD in the window's corners, bars would frame
//! every arena on a phone or a tablet.
//!
//! Nothing stands on it and nothing plays there: the simulation, the
//! field's rect on the window, `View`'s mapping and every pointer are what
//! they were, and the margins are drawn behind the field
//! (`render::margin`), under the round's sky the way the field is. Only an
//! arena in play, online or behind the lobby draws them; the builder keeps
//! its plain canvas fill, and a followed field map shows the world it
//! follows.
//!
//! Headless: `MarginFrame::of` is the world the window shows round the
//! field, and `Margin` the ground and the shade made for it, kept until the
//! round's floor changes or a window needs the ground to reach further.

use crate::ground::{bake_margin_shade, GroundGrid, MarginShade};
use crate::map::Theme;
use crate::math::{Rectangle, Vec2};
use crate::tuning::Tuning;
use crate::view::View;
use crate::{Layout, GROUND_WORLD_TILE};

/// The most cells the ground is carried past the field's own grid: a
/// window further than this past its arena on a side shows the shade's
/// plateau over the backdrop out there.
pub const MAX_CELLS: usize = 96;

/// The cells the ground's reach is rounded up to, so a window resized by a
/// little keeps the ground it has.
const CELLS_STEP: usize = 4;

/// The world a window shows round an arena's field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarginFrame {
    /// The world rectangle the whole window shows, px, rounded out to the
    /// 2 px block grid: the field and the margins round it.
    pub rect: Rectangle,
    /// How many cells past the field's own ground grid the ground has to
    /// reach to cover `rect`, rounded up to `CELLS_STEP`, at most
    /// `MAX_CELLS`.
    pub cells: usize,
}

impl MarginFrame {
    /// The world `view` shows round the field `layout` puts in its bitmap,
    /// a bitmap pixel to the world pixel - an arena's - or `None` where the
    /// bitmap fills the window and there is no margin to draw.
    pub fn of(view: &View, layout: &Layout) -> Option<MarginFrame> {
        if view.is_identity() {
            return None;
        }
        let corner = layout.field_origin();
        let world = |window: Vec2| {
            let at = view.to_bitmap(window);
            Vec2::new(at.x - corner.x, at.y - corner.y)
        };
        let (near, far) = (world(Vec2::new(0.0, 0.0)), world(Vec2::new(view.window.0, view.window.1)));
        let block = crate::pyro::BLOCK;
        let (x0, y0) = ((near.x / block).floor() * block, (near.y / block).floor() * block);
        let (x1, y1) = ((far.x / block).ceil() * block, (far.y / block).ceil() * block);
        let (w, h) = (layout.field.w, layout.field.h);
        if x0 >= 0.0 && y0 >= 0.0 && x1 <= w && y1 <= h {
            return None;
        }
        // The field's own grid reaches half a cell past it on every side:
        // its last cell is centred on the field's width rounded up to
        // whole cells.
        let tile = GROUND_WORLD_TILE;
        let half = tile / 2.0;
        let (right, bottom) = ((w / tile).ceil() * tile + half, (h / tile).ceil() * tile + half);
        let past = (-half - x0).max(-half - y0).max(x1 - right).max(y1 - bottom).max(0.0);
        let cells = ((past / tile).ceil() as usize).max(1).div_ceil(CELLS_STEP) * CELLS_STEP;
        Some(MarginFrame { rect: Rectangle::new(x0, y0, x1 - x0, y1 - y0), cells: cells.min(MAX_CELLS) })
    }
}

/// The ground and the shade an arena's margins are drawn from, made for
/// one round's floor (`fits`).
pub struct Margin {
    /// The round's floor carried `cells` cells further on every side.
    pub ground: GroundGrid,
    pub shade: MarginShade,
    pub theme: Theme,
    /// What it was made from: the floor's build (`GroundGrid::stamp`) and
    /// the reach.
    floor: u64,
    cells: usize,
}

impl Margin {
    /// The margins of a round whose floor is `floor`, its field `field` px
    /// in the theme `theme`, the ground carried `cells` cells past it.
    pub fn new(floor: &GroundGrid, field: (f32, f32), theme: Theme, cells: usize, t: &Tuning) -> Margin {
        Margin { ground: floor.beyond(cells), shade: bake_margin_shade(field.0, field.1, theme, t), theme, floor: floor.stamp(), cells }
    }

    /// Whether this was made for the floor `floor` and reaches `cells`
    /// cells or more: a new round's floor is another, and a window that
    /// needs no more ground than this reaches keeps it.
    pub fn fits(&self, floor: &GroundGrid, cells: usize) -> bool {
        self.floor == floor.stamp() && self.cells >= cells
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIELD: (f32, f32) = (1088.0, 544.0);

    fn frame(window: (f32, f32)) -> Option<MarginFrame> {
        let layout = Layout::bare(FIELD.0, FIELD.1);
        MarginFrame::of(&View::fit(FIELD, window), &layout)
    }

    #[test]
    fn a_window_the_fields_shape_has_no_margin() {
        assert_eq!(frame(FIELD), None);
        assert_eq!(frame((2176.0, 1088.0)), None, "twice the field, no bars");
    }

    /// The window shows the field where `View` puts it, and the world
    /// round it to the window's edges: the rect is the window in world
    /// pixels, rounded out to whole blocks, and the ground reaches it.
    #[test]
    fn the_margins_are_the_window_past_the_field_in_world_pixels() {
        for (window, sides) in [((1088.0, 576.0), false), ((1180.0, 820.0), false), ((852.0, 393.0), true), ((1024.0, 768.0), false), ((1920.0, 1080.0), false)] {
            let f = frame(window).unwrap_or_else(|| panic!("{window:?} has margins"));
            let view = View::fit(FIELD, window);
            let r = f.rect;
            for v in [r.x, r.y, r.width, r.height] {
                assert_eq!(v % 2.0, 0.0, "{window:?}: on the block grid: {r:?}");
            }
            let near = view.to_bitmap(Vec2::new(0.0, 0.0));
            let far = view.to_bitmap(Vec2::new(window.0, window.1));
            assert!(r.x <= near.x && r.y <= near.y && r.x + r.width >= far.x && r.y + r.height >= far.y, "{window:?}: {r:?} covers the window");
            assert!(r.x > near.x - 2.0 && r.y > near.y - 2.0 && r.x + r.width < far.x + 2.0 && r.y + r.height < far.y + 2.0, "{window:?}: and no more");
            // The bars are on the long axis the window has to spare.
            assert_eq!(r.x < 0.0, sides, "{window:?}: {r:?}");
            assert_eq!(r.y < 0.0, !sides, "{window:?}: {r:?}");
            // The ground's reach: the field's grid ends half a cell past
            // it, and `cells` more cover the rest.
            let reach = (f.cells as f32 + 0.5) * GROUND_WORLD_TILE;
            assert!(-r.x <= reach && -r.y <= reach && r.x + r.width <= FIELD.0 + reach && r.y + r.height <= FIELD.1 + reach, "{window:?}: {f:?}");
            assert_eq!(f.cells % CELLS_STEP, 0);
        }
        // An iPad's 115 points of margin over and under a 1088 x 544 arena
        // are 106 world pixels: four cells reach them.
        assert_eq!(frame((1180.0, 820.0)).map(|f| f.cells), Some(4));
    }

    /// A window far past its arena - a tall desktop window - asks for no
    /// more than `MAX_CELLS` of ground.
    #[test]
    fn the_grounds_reach_is_capped() {
        let f = frame((300.0, 3000.0)).expect("margins");
        assert_eq!(f.cells, MAX_CELLS);
    }

    #[test]
    fn a_margin_fits_its_round_until_a_window_needs_more_ground() {
        let t = Tuning::DEFAULT;
        let water = [crate::Position::new(5.0 * GROUND_WORLD_TILE, 0.0)];
        let floor = crate::ground::build(FIELD.0, FIELD.1, 7, &[], &water, &[], crate::ground::Look { theme: Theme::Grass, edge_shade: true });
        let margin = Margin::new(&floor, FIELD, Theme::Grass, 8, &t);
        assert!(margin.fits(&floor, 4) && margin.fits(&floor, 8) && !margin.fits(&floor, 12));
        assert_eq!(margin.ground.origin(), (-8, -8));
        let next = crate::ground::build(FIELD.0, FIELD.1, 7, &[], &water, &[], crate::ground::Look { theme: Theme::Grass, edge_shade: true });
        assert!(!margin.fits(&next, 4), "a new round's floor is another");
    }
}
