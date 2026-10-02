//! How much of the world a screen shows: the rules a follow camera frames
//! a field map by, and this module is their reference
//! (docs/large-maps-follow-camera.md §3, §4 and §15). Headless and pure -
//! a screen, a seating and the sight box in, the visible world and its
//! scale out; no `Game`, no window, no RNG - and presentation only, since
//! nothing in `simulation/` sees a screen.
//!
//! - **The map decides whether there is a camera** (`MapClass`). An arena,
//!   no bigger than `ARENA_MAX_CELLS` (36 x 18 cells), is shown whole on
//!   every screen, the way `view::View` letterboxes it; anything bigger is
//!   a field map, which a camera follows the local seat across. A map's
//!   `view` key overrides the size (`MapFile::class`). The class is the
//!   map's and never the screen's, so every seat in a room plays the same
//!   kind of round.
//! - **The same area on every screen** (`frame`). A field map shows
//!   `view_area_cells` (578, the standard 34 x 17 field's area) on every
//!   screen, and the screen's shape picks only the outline, clamped
//!   between `view_aspect_min` (4:3) and `view_aspect_max` (2.4:1). It is
//!   the one rule that gives a phone, a tablet and a monitor the same
//!   amount of world - and with it the same time to see a shell coming -
//!   without bars anywhere between those two shapes; a screen past the
//!   clamp gets bars across its long axis.
//! - **Whole blocks where a pixel shows.** The field is drawn in 2 px
//!   blocks, so the art is crisp when a block covers a whole number of
//!   device pixels: a scale that is a multiple of 0.5 device pixels per
//!   world pixel. A screen under `view_fine_ppi` (360) snaps to the nearer
//!   such step, nearer by ratio, which lands the area within about 12 % of
//!   the target. On a 3x phone the steps are about 20 % apart - either
//!   neighbour would push the sight box off screen or shrink the tank
//!   under 44 pt - and a device pixel is too small for the blur to show,
//!   so a finer screen keeps the exact zoom.
//! - **The sight box is always on screen.** Enemies fire at a seat only
//!   from inside its sight box (§5), so no screen may hide it. The area's
//!   outline holds the standard +-11.5 x +-7.5 cells at every allowed
//!   shape (the 2.4:1 outline is 15.5 cells tall, the 4:3 one 27.8 wide),
//!   the snap steps outward rather than inward past it, and
//!   `Framing::room_outside` is all a camera may lead the seat by. A
//!   screen of at least 368 x 240 device pixels - the standard box at half
//!   a device pixel per world pixel, the furthest the snap zooms out -
//!   always shows it.
//! - **A local round on a big screen zooms out** (§3, §15). With no room
//!   nobody shares the round, so a monitor steps the zoom out a whole
//!   block at a time while a tank stays at least `view_local_min_tank_mm`
//!   (25 mm) wide and the view at most `view_local_max_cells` (900, which
//!   is 40 x 22.5 on a 16:9 monitor). Phones, tablets and laptops draw a
//!   tank under 25 mm already and keep the shared view; a room never
//!   steps, so a monitor that joins one shows what every other seat shows.
//! - **Play has no bar over the world**: its HUD stands in the window's
//!   corners (`hud::corners`), so a followed round is framed into the
//!   whole screen (`frame`). A view that does carry a bar above it is
//!   framed into the screen less the bar at the scale the framing picks
//!   (`frame_under_bar`), so the two fill the screen together.

use crate::tuning::{Tuning, tuning};
use crate::{OBSTACLE_GRID_SIZE, TANK_FRAME_SIZE};

/// The largest map still shown whole, in cells (columns, rows). A map
/// with no size (34 x 17) and a 36 x 18 one are arenas; a 40-wide map and
/// anything bigger - every shipped map - are field maps, which shown whole
/// would draw a phone's tank at 6.5 mm or less, under the 44 pt touch
/// floor (docs/large-maps-follow-camera.md §15). A rule about the map rather
/// than a knob: every seat in a room has to agree on it, and nothing about
/// a screen moves it.
pub const ARENA_MAX_CELLS: (f32, f32) = (36.0, 18.0);

/// A tank's width in world pixels - the 32 px frame at the 2x every tank
/// is built with (`Tank::size`) - the yardstick a framing measures a
/// screen's zoom by.
pub const TANK_PX: f32 = TANK_FRAME_SIZE * 2.0;

/// The finest zoom: a 2 px block on one device pixel, in device pixels per
/// world pixel. Neither the snap nor a local round's zoom-out goes past it.
const MIN_SCALE: f64 = 0.5;

/// One whole-block step of the zoom, in device pixels per world pixel:
/// half a device pixel per world pixel is one per 2 px block.
const BLOCK_STEP: f64 = 0.5;

const MM_PER_INCH: f32 = 25.4;

/// A comparison's allowance for rounding, relative, so a view of exactly
/// `view_local_max_cells` or a tank of exactly `view_local_min_tank_mm` is
/// within the limit rather than a rounding error past it.
const ROUNDING: f64 = 1e-9;

/// How a map is shown: whole, or followed by a camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapClass {
    /// Shown whole on every screen, letterboxed (`view::View`): the
    /// standard 34 x 17 field and anything up to `ARENA_MAX_CELLS`.
    Arena,
    /// Bigger than an arena: a camera follows the local seat, and the
    /// screen shows the world `frame` says.
    Field,
}

impl MapClass {
    /// The class a map of `cols` x `rows` cells gets by its size: an arena
    /// while it fits `ARENA_MAX_CELLS` both ways, a field map past either.
    pub fn by_size(cols: f32, rows: f32) -> MapClass {
        if cols <= ARENA_MAX_CELLS.0 && rows <= ARENA_MAX_CELLS.1 { MapClass::Arena } else { MapClass::Field }
    }

    /// Whether a camera follows the seat on a map of this class.
    pub fn follows(self) -> bool {
        self == MapClass::Field
    }
}

/// A screen as the rules see it: its size in points, how many device
/// pixels a point is, how dense those are and, when the platform says, how
/// big a point is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    /// The logical size, width by height, in points: the window in
    /// raylib's screen coordinates, a page in CSS pixels.
    pub size: (f32, f32),
    /// Device pixels per point (2 on a Retina display, 2.625 on a Pixel 9).
    pub dpr: f32,
    /// The panel's density in pixels per inch. It decides only whether the
    /// zoom snaps to whole blocks (`view_fine_ppi`).
    pub ppi: f32,
    /// Physical millimetres per point, when the platform reports the
    /// screen's size. Unknown, a tank's size on it is too, and a local
    /// round keeps the shared view.
    pub mm_per_point: Option<f32>,
}

impl Screen {
    /// A screen of `width` x `height` points at `dpr` device pixels a point
    /// and `ppi`, its physical size unknown.
    pub fn new(width: f32, height: f32, dpr: f32, ppi: f32) -> Screen {
        Screen { size: (width, height), dpr, ppi, mm_per_point: None }
    }

    /// This screen with its physical size read off its panel: `panel_px`
    /// pixels across at `ppi`. That is usually the width in points times
    /// `dpr`, but not on a desktop scaled to look like another size (a
    /// MacBook Air's 1470 points at 2x are drawn on a 2560 px panel).
    pub fn with_panel_width(self, panel_px: f32) -> Screen {
        self.with_mm_per_point(panel_px / self.ppi * MM_PER_INCH / self.size.0)
    }

    /// This screen with its physical size given as millimetres per point;
    /// anything but a positive number leaves the size unknown.
    pub fn with_mm_per_point(self, mm: f32) -> Screen {
        Screen { mm_per_point: (mm.is_finite() && mm > 0.0).then_some(mm), ..self }
    }
}

/// Who a round is framed for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seating {
    /// A round in an online room: every seat shows the same area, so no
    /// screen sees more of the field than another.
    Room,
    /// A local round, one player alone or two on one screen: nobody else
    /// shares it, so a big screen may show more (`view_local_*`).
    Local,
}

/// The box around a seat that every screen has to show, because enemies
/// fire at that seat only from inside it (docs/large-maps-follow-camera.md
/// §5): its half extents in world pixels, the seat at its centre. It is a
/// room rule, the same for every seat, so the caller hands in the room's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SightBox {
    /// Half the box's width and half its height, in world pixels.
    pub half: (f32, f32),
}

impl SightBox {
    /// A box reaching `half_width` and `half_height` world pixels either
    /// side of the seat.
    pub fn new(half_width: f32, half_height: f32) -> SightBox {
        SightBox { half: (half_width, half_height) }
    }

    /// A box reaching `half_cols` and `half_rows` 32 px cells either side
    /// of the seat: +-11.5 x +-7.5 cells is +-368 x +-240 px.
    pub fn from_cells(half_cols: f32, half_rows: f32) -> SightBox {
        SightBox::new(half_cols * OBSTACLE_GRID_SIZE, half_rows * OBSTACLE_GRID_SIZE)
    }
}

/// The numbers `frame` reads, from the tuning table's `view` group: taken
/// once a frame (`ViewRules::current`) and passed in, so a test states the
/// rules it runs and never touches the global table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewRules {
    /// `view_area_cells`: the world every screen shows, in 32 px cells.
    pub area_cells: f32,
    /// `view_aspect_min`: the narrowest outline, width over height.
    pub aspect_min: f32,
    /// `view_aspect_max`: the widest outline, width over height.
    pub aspect_max: f32,
    /// `view_fine_ppi`: the density from which the zoom stays exact.
    pub fine_ppi: f32,
    /// `view_local_min_tank_mm`: the smallest tank a local round's
    /// zoom-out draws, in millimetres.
    pub local_min_tank_mm: f32,
    /// `view_local_max_cells`: the most world a local round's zoom-out
    /// shows, in cells.
    pub local_max_cells: f32,
}

impl ViewRules {
    /// The rules in the tuning table this frame.
    pub fn current() -> ViewRules {
        ViewRules::of(&tuning())
    }

    /// The rules in `t`.
    pub fn of(t: &Tuning) -> ViewRules {
        ViewRules {
            area_cells: t.view_area_cells,
            aspect_min: t.view_aspect_min,
            aspect_max: t.view_aspect_max,
            fine_ppi: t.view_fine_ppi,
            local_min_tank_mm: t.view_local_min_tank_mm,
            local_max_cells: t.view_local_max_cells,
        }
    }
}

/// What a screen shows of a field map (`frame`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Framing {
    /// The world on screen, width by height, in world pixels: the size of
    /// the camera's view rectangle.
    pub visible: (f32, f32),
    /// Device pixels per world pixel - a multiple of 0.5 when `snapped`, so
    /// every 2 px block covers whole device pixels.
    pub scale: f32,
    /// Points per world pixel: `scale` over the screen's `dpr`.
    pub point_scale: f32,
    /// Whether the zoom was snapped to a whole-block step, which is
    /// whether the screen is under `view_fine_ppi`; otherwise `scale` is
    /// the exact same-area zoom.
    pub snapped: bool,
    /// The box the world fills, width by height in points, centred on the
    /// screen: the whole screen unless its shape was clamped.
    pub world_box: (f32, f32),
    /// The bars around `world_box`, in points: the width of each side bar
    /// and the height of the top and the bottom one. Zero unless the
    /// screen is wider than `view_aspect_max` (side bars) or narrower than
    /// `view_aspect_min` (top and bottom).
    pub bars: (f32, f32),
    /// A tank's width on screen, in points.
    pub tank_points: f32,
    /// A tank's width on screen in millimetres, when the screen's physical
    /// size is known.
    pub tank_mm: Option<f32>,
}

impl Framing {
    /// `visible` in 32 px cells.
    pub fn visible_cells(&self) -> (f32, f32) {
        (self.visible.0 / OBSTACLE_GRID_SIZE, self.visible.1 / OBSTACLE_GRID_SIZE)
    }

    /// Device pixels per 2 px block, the size every piece of the field's
    /// art is built from.
    pub fn block(&self) -> f32 {
        self.scale * 2.0
    }

    /// Whether the whole of `sight` is on screen with the camera on the
    /// seat.
    pub fn shows(&self, sight: SightBox) -> bool {
        let room = self.room(sight);
        room.0 >= 0.0 && room.1 >= 0.0
    }

    /// How far the camera may lead the seat on each axis and still show
    /// the whole of `sight`: half the view less half the box, never below
    /// zero. A look-ahead spends this and no more (§6).
    pub fn room_outside(&self, sight: SightBox) -> (f32, f32) {
        let room = self.room(sight);
        (room.0.max(0.0), room.1.max(0.0))
    }

    fn room(&self, sight: SightBox) -> (f32, f32) {
        (self.visible.0 / 2.0 - sight.half.0, self.visible.1 / 2.0 - sight.half.1)
    }
}

/// Frame a field map on `screen`: how much world it shows, at what scale,
/// and how big a tank is drawn (docs/large-maps-follow-camera.md §3, §4).
///
/// 1. Same area: the outline holding `view_area_cells`, the screen's shape
///    clamped between `view_aspect_min` and `view_aspect_max`, at the
///    largest scale that fits it on the screen. The world fills the screen
///    less bars across the long axis of a screen past the clamp.
/// 2. On a screen under `view_fine_ppi` the scale snaps to whichever
///    neighbouring whole-block step (a multiple of 0.5 device pixels per
///    world pixel, never under 0.5) is nearer by ratio - in log space -
///    the outward one on a tie, and to the outward one wherever the
///    nearer would hide `sight`.
/// 3. A `Seating::Local` round on a screen of known size steps the scale
///    down 0.5 at a time for as long as a tank stays at least
///    `view_local_min_tank_mm` wide and the world box at that scale holds
///    at most `view_local_max_cells`. A room never steps.
///
/// A degenerate screen - a minimized window's zero size, a missing scale -
/// is read as one point, or one device pixel a point, so the answer stays
/// finite.
pub fn frame(screen: Screen, seating: Seating, sight: SightBox, rules: &ViewRules) -> Framing {
    let cell = f64::from(OBSTACLE_GRID_SIZE);
    let o = Outline::of(screen, rules);
    let (box_w, box_h, dpr) = (o.box_w, o.box_h, o.dpr);
    let (half_w, half_h) = (f64::from(sight.half.0), f64::from(sight.half.1));
    let shows = |scale: f64| box_w * dpr / scale / 2.0 >= half_w && box_h * dpr / scale / 2.0 >= half_h;

    // 2. Whole blocks. `scale * scale <= lo * hi` is the scale at or below
    // the two steps' geometric mean: nearer `lo` by ratio, or as near.
    let mut scale = o.fit * dpr;
    let snapped = f64::from(screen.ppi) < f64::from(rules.fine_ppi);
    if snapped {
        let lo = ((scale * 2.0).floor() / 2.0).max(MIN_SCALE);
        let hi = ((scale * 2.0).ceil() / 2.0).max(MIN_SCALE);
        let nearer = if scale * scale <= lo * hi { lo } else { hi };
        scale = if shows(nearer) { nearer } else { lo };
    }

    // 3. A local round's zoom-out. Every step lowers the scale by a whole
    // block, and none goes under `MIN_SCALE`, so the walk ends.
    if seating == Seating::Local
        && let Some(mm_per_point) = screen.mm_per_point
    {
        let min_tank_mm = f64::from(rules.local_min_tank_mm);
        let max_area = f64::from(rules.local_max_cells) * cell * cell * (1.0 + ROUNDING);
        loop {
            let next = scale - BLOCK_STEP;
            if next < MIN_SCALE {
                break;
            }
            let points = next / dpr;
            let tank_mm = f64::from(TANK_PX) * points * f64::from(mm_per_point);
            let shown = (box_w / points) * (box_h / points);
            if tank_mm * (1.0 + ROUNDING) < min_tank_mm || shown > max_area {
                break;
            }
            scale = next;
        }
    }

    o.framing(screen, scale, snapped)
}

/// A bar across the top of the screen over a followed field map - the HUD
/// bar: `height` pixels of a bitmap as wide as the world it heads but
/// never narrower than `min_width`, so the bar's slots always have the
/// width they are laid out for. Where the view is at least that wide a
/// bitmap pixel is a world pixel; where it is narrower the bar's pixels
/// are smaller than the world's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    pub height: f32,
    pub min_width: f32,
}

impl Bar {
    /// Points per pixel of the bitmap the bar heads over `f`: the world
    /// box's width over the bitmap's, the wider of the view and
    /// `min_width`. The framing's own `point_scale` where the view is the
    /// wider.
    pub fn point_scale(&self, f: &Framing) -> f32 {
        f.world_box.0 / f.visible.0.max(self.min_width).max(1.0)
    }

    /// The bar's height on screen over `f`, in points.
    pub fn points(&self, f: &Framing) -> f32 {
        self.height.max(0.0) * self.point_scale(f)
    }
}

/// `frame` for a screen that also carries `bar` across its top: the world
/// is framed into the screen less the bar, so the bar and the world fill
/// the screen together. The bar's height on screen depends on the
/// framing - it is drawn at the world's own scale where the view is at
/// least the bar's width - so the framing is repeated on the screen less
/// the bar the last one would carry until the bar stands still. A snapped
/// scale can alternate between two neighbouring steps instead - the
/// taller bar of the inward one tipping the framing outward and back -
/// and then the outward one is kept under its own bar: it shows more
/// world, so the sight box is on screen there too. `frame_under_bar` with
/// no bar is `frame`.
pub fn frame_under_bar(screen: Screen, bar: Bar, seating: Seating, sight: SightBox, rules: &ViewRules) -> Framing {
    let height = positive(screen.size.1, 1.0);
    let below = |points: f32| Screen { size: (screen.size.0, (height - f64::from(points)).max(1.0) as f32), ..screen };
    let mut f = frame(screen, seating, sight, rules);
    if bar.height <= 0.0 {
        return f;
    }
    let mut at = bar.points(&f);
    let mut before: Option<f32> = None;
    for _ in 0..BAR_ITERATIONS {
        let next = frame(below(at), seating, sight, rules);
        let next_at = bar.points(&next);
        if (next_at - at).abs() <= at.max(1e-3) * 1e-6 {
            return next;
        }
        if before == Some(next_at) {
            let outward = if next.point_scale < f.point_scale { next } else { f };
            let under = below(bar.points(&outward));
            return Outline::of(under, rules).framing(under, f64::from(outward.scale), outward.snapped);
        }
        before = Some(at);
        at = next_at;
        f = next;
    }
    f
}

/// How many times `frame_under_bar` re-frames before it settles for the
/// last answer. An exact scale converges in two or three - the bar is a
/// few percent of the height, so each round moves the scale by a few
/// percent of the last move - and a snapped one in one or two.
const BAR_ITERATIONS: usize = 8;

/// Step 1 of `frame`: the same-area outline on a screen and the box the
/// world fills there, before any snapping.
struct Outline {
    width: f64,
    height: f64,
    dpr: f64,
    /// The exact same-area zoom, points per world pixel.
    fit: f64,
    box_w: f64,
    box_h: f64,
}

impl Outline {
    /// The clamp is written as max-then-min so knobs set the wrong way
    /// round cannot panic it.
    fn of(screen: Screen, rules: &ViewRules) -> Outline {
        let cell = f64::from(OBSTACLE_GRID_SIZE);
        let (width, height) = (positive(screen.size.0, 1.0), positive(screen.size.1, 1.0));
        let dpr = positive(screen.dpr, 1.0);
        let area = positive(rules.area_cells, 1.0) * cell * cell;
        let (aspect_min, aspect_max) = (positive(rules.aspect_min, 1.0), positive(rules.aspect_max, 1.0));
        let aspect = width / height;
        let shape = aspect.max(aspect_min).min(aspect_max);
        let (outline_w, outline_h) = ((area * shape).sqrt(), (area / shape).sqrt());
        let fit = (width / outline_w).min(height / outline_h);
        let box_w = if aspect > aspect_max { outline_w * fit } else { width };
        let box_h = if aspect < aspect_min { outline_h * fit } else { height };
        Outline { width, height, dpr, fit, box_w, box_h }
    }

    /// The world this outline's box shows at `scale` device pixels per
    /// world pixel.
    fn framing(&self, screen: Screen, scale: f64, snapped: bool) -> Framing {
        let point_scale = scale / self.dpr;
        let tank_points = f64::from(TANK_PX) * point_scale;
        Framing {
            visible: ((self.box_w / point_scale) as f32, (self.box_h / point_scale) as f32),
            scale: scale as f32,
            point_scale: point_scale as f32,
            snapped,
            world_box: (self.box_w as f32, self.box_h as f32),
            bars: (((self.width - self.box_w) / 2.0) as f32, ((self.height - self.box_h) / 2.0) as f32),
            tank_points: tank_points as f32,
            tank_mm: screen.mm_per_point.map(|mm| (tank_points * f64::from(mm)) as f32),
        }
    }
}

/// `v` if it is a positive number, else `or`.
fn positive(v: f32, or: f64) -> f64 {
    let v = f64::from(v);
    if v.is_finite() && v > 0.0 { v } else { or }
}

#[cfg(test)]
mod framing_tests {
    use super::*;

    /// The sight box the rules are built around, +-11.5 x +-7.5 cells.
    fn standard_box() -> SightBox {
        SightBox::from_cells(11.5, 7.5)
    }

    fn rules() -> ViewRules {
        ViewRules::of(&Tuning::DEFAULT)
    }

    /// One row of the device table in docs/large-maps-follow-camera.md §4:
    /// the screen in landscape, then what a room shows on it - cells across,
    /// cells down, the tank in millimetres, device pixels per block - and
    /// what a local round shows where that differs.
    struct Device {
        name: &'static str,
        points: (f32, f32),
        dpr: f32,
        ppi: f32,
        panel_px: f32,
        room: (f32, f32, f32, f32),
        local: Option<(f32, f32, f32)>,
    }

    impl Device {
        fn screen(&self) -> Screen {
            Screen::new(self.points.0, self.points.1, self.dpr, self.ppi).with_panel_width(self.panel_px)
        }

        fn frame(&self, seating: Seating) -> Framing {
            frame(self.screen(), seating, standard_box(), &rules())
        }
    }

    const DEVICES: [Device; 18] = [
        Device { name: "iPhone SE", points: (667.0, 375.0), dpr: 2.0, ppi: 326.0, panel_px: 1334.0, room: (27.8, 15.6, 7.5, 3.00), local: None },
        Device { name: "iPhone 16", points: (852.0, 393.0), dpr: 3.0, ppi: 460.0, panel_px: 2556.0, room: (35.4, 16.3, 8.0, 4.51), local: None },
        Device { name: "iPhone 17 Pro Max", points: (956.0, 440.0), dpr: 3.0, ppi: 460.0, panel_px: 2868.0, room: (35.4, 16.3, 8.9, 5.06), local: None },
        Device { name: "Pixel 9", points: (923.0, 411.0), dpr: 2.625, ppi: 422.0, panel_px: 2424.0, room: (36.0, 16.0, 8.1, 4.20), local: None },
        Device { name: "Galaxy S25", points: (780.0, 360.0), dpr: 3.0, ppi: 416.0, panel_px: 2340.0, room: (35.4, 16.3, 8.1, 4.13), local: None },
        Device { name: "Galaxy A06", points: (800.0, 360.0), dpr: 2.0, ppi: 262.0, panel_px: 1600.0, room: (33.3, 15.0, 9.3, 3.00), local: None },
        Device { name: "iPad mini", points: (1133.0, 744.0), dpr: 2.0, ppi: 326.0, panel_px: 2266.0, room: (28.3, 18.6, 12.5, 5.00), local: None },
        Device { name: "iPad Air 11\"", points: (1180.0, 820.0), dpr: 2.0, ppi: 264.0, panel_px: 2360.0, room: (29.5, 20.5, 15.4, 5.00), local: None },
        Device { name: "iPad Pro 13\"", points: (1376.0, 1032.0), dpr: 2.0, ppi: 264.0, panel_px: 2752.0, room: (28.7, 21.5, 18.5, 6.00), local: None },
        Device { name: "Galaxy Tab S9", points: (1280.0, 800.0), dpr: 2.0, ppi: 274.0, panel_px: 2560.0, room: (32.0, 20.0, 14.8, 5.00), local: None },
        Device { name: "Chromebook 11.6\"", points: (1366.0, 768.0), dpr: 1.0, ppi: 135.0, panel_px: 1366.0, room: (28.5, 16.0, 18.1, 3.00), local: None },
        Device { name: "MacBook Air 13\"", points: (1470.0, 956.0), dpr: 2.0, ppi: 224.0, panel_px: 2560.0, room: (30.6, 19.9, 19.0, 6.00), local: None },
        Device { name: "MacBook Pro 14\"", points: (1512.0, 982.0), dpr: 2.0, ppi: 254.0, panel_px: 3024.0, room: (31.5, 20.5, 19.2, 6.00), local: None },
        Device { name: "24\" 1080p", points: (1920.0, 1080.0), dpr: 1.0, ppi: 92.0, panel_px: 1920.0, room: (30.0, 16.9, 35.3, 4.00), local: Some((40.0, 22.5, 26.5)) },
        Device { name: "27\" 1440p", points: (2560.0, 1440.0), dpr: 1.0, ppi: 109.0, panel_px: 2560.0, room: (32.0, 18.0, 37.3, 5.00), local: Some((40.0, 22.5, 29.8)) },
        Device { name: "27\" 4K at 150%", points: (2560.0, 1440.0), dpr: 1.5, ppi: 163.0, panel_px: 3840.0, room: (30.0, 16.9, 39.9, 8.00), local: Some((40.0, 22.5, 29.9)) },
        Device { name: "34\" ultrawide", points: (3440.0, 1440.0), dpr: 1.0, ppi: 110.0, panel_px: 3440.0, room: (35.8, 15.0, 44.3, 6.00), local: Some((43.0, 18.0, 36.9)) },
        Device { name: "49\" 32:9", points: (5120.0, 1440.0), dpr: 1.0, ppi: 109.0, panel_px: 5120.0, room: (36.0, 15.0, 44.7, 6.00), local: Some((43.2, 18.0, 37.3)) },
    ];

    /// The doc's numbers are rounded to a tenth of a cell and of a
    /// millimetre, its blocks to a hundredth of a device pixel.
    fn assert_matches(name: &str, seating: Seating, f: &Framing, (cols, rows, mm): (f32, f32, f32)) {
        let (got_cols, got_rows) = f.visible_cells();
        let got_mm = f.tank_mm.expect("every device in the table has a known size");
        assert!(
            (got_cols - cols).abs() <= 0.1 && (got_rows - rows).abs() <= 0.1 && (got_mm - mm).abs() <= 0.1,
            "{name} ({seating:?}): {got_cols:.2} x {got_rows:.2} cells, a {got_mm:.2} mm tank; the doc says {cols} x {rows}, {mm} mm"
        );
    }

    #[test]
    fn every_device_shows_the_docs_room_view() {
        for d in &DEVICES {
            let f = d.frame(Seating::Room);
            let (cols, rows, mm, block) = d.room;
            assert_matches(d.name, Seating::Room, &f, (cols, rows, mm));
            assert!((f.block() - block).abs() <= 0.005, "{}: a {:.3} px block, the doc says {block}", d.name, f.block());
            assert_eq!(f.snapped, d.ppi < 360.0, "{}: only a screen under 360 ppi snaps", d.name);
            if f.snapped {
                assert_eq!(f.scale * 2.0, (f.scale * 2.0).round(), "{}: a snapped block is whole device pixels", d.name);
            }
        }
    }

    #[test]
    fn every_device_shows_the_docs_local_view() {
        for d in &DEVICES {
            let f = d.frame(Seating::Local);
            let (cols, rows, mm, _) = d.room;
            assert_matches(d.name, Seating::Local, &f, d.local.unwrap_or((cols, rows, mm)));
            if d.local.is_none() {
                assert_eq!(f, d.frame(Seating::Room), "{}: a tank under 25 mm keeps the shared view", d.name);
            }
        }
    }

    #[test]
    fn a_16_by_9_monitor_zooms_out_to_40_by_22_5_cells_and_no_further() {
        // Exactly the 900 cells `view_local_max_cells` allows, in whole
        // blocks; the next step out would show more.
        for name in ["24\" 1080p", "27\" 1440p", "27\" 4K at 150%"] {
            let d = DEVICES.iter().find(|d| d.name == name).unwrap();
            let f = d.frame(Seating::Local);
            assert_eq!(f.visible_cells(), (40.0, 22.5), "{name}");
            assert!(f.snapped && f.scale < d.frame(Seating::Room).scale, "{name}: whole-block steps outward");
            let next = (f.scale - 0.5) / d.dpr;
            assert!(d.points.0 / next * (d.points.1 / next) / 1024.0 > 900.0, "{name}");
        }
    }

    #[test]
    fn a_wide_screen_past_the_clamp_gets_side_bars() {
        let d = DEVICES.iter().find(|d| d.name == "49\" 32:9").unwrap();
        for seating in [Seating::Room, Seating::Local] {
            let f = d.frame(seating);
            // The box keeps 2.4:1 across the screen's full height: 3456 x
            // 1440 points, 832 points of bar either side.
            assert!((f.world_box.0 - 3456.0).abs() < 0.01 && f.world_box.1 == 1440.0, "{:?}", f.world_box);
            assert!((f.bars.0 - 832.0).abs() < 0.01 && f.bars.1 == 0.0, "{:?}", f.bars);
            assert!((f.world_box.0 / f.world_box.1 - 2.4).abs() < 1e-4);
        }
        // Every shape between 4:3 and 2.4:1 fills its screen.
        for d in DEVICES.iter().filter(|d| d.name != "49\" 32:9") {
            let f = d.frame(Seating::Room);
            assert_eq!(f.bars, (0.0, 0.0), "{}", d.name);
            assert_eq!(f.world_box, d.points, "{}", d.name);
        }
        // A square screen, narrower than 4:3, gets bars above and below
        // and shows the 4:3 outline.
        let square = frame(Screen::new(1000.0, 1000.0, 1.0, 500.0), Seating::Room, standard_box(), &rules());
        assert_eq!(square.bars.0, 0.0);
        assert!((square.bars.1 - 125.0).abs() < 0.05, "{:?}", square.bars);
        let (w, h) = square.visible;
        assert!((w / h - 4.0 / 3.0).abs() < 1e-3 && (w * h / 1024.0 - 578.0).abs() < 0.1, "{w} x {h}");
    }

    #[test]
    fn a_fine_screen_keeps_the_exact_same_area_zoom() {
        let d = DEVICES.iter().find(|d| d.name == "iPhone 16").unwrap();
        let f = d.frame(Seating::Room);
        assert!(!f.snapped);
        let (w, h) = f.visible;
        assert!((w * h / 1024.0 - 578.0).abs() < 0.01, "exactly the area: {w} x {h}");
        assert!((f.point_scale - 393.0 / h).abs() < 1e-5);
        assert!((f.scale - f.point_scale * 3.0).abs() < 1e-5);
        // 4.51 device pixels a block: neither 4 nor 5.
        assert!((f.block() - 4.51).abs() < 0.005);
        // The same screen below the threshold snaps.
        let coarse = frame(Screen { ppi: 359.0, ..d.screen() }, Seating::Room, standard_box(), &rules());
        assert!(coarse.snapped && coarse.block().fract() == 0.0, "{}", coarse.block());
    }

    #[test]
    fn the_sight_box_is_on_every_devices_screen() {
        let sight = standard_box();
        for d in &DEVICES {
            for seating in [Seating::Room, Seating::Local] {
                let f = d.frame(seating);
                assert!(f.shows(sight), "{} ({seating:?}): {:?} hides the box", d.name, f.visible_cells());
                let room = f.room_outside(sight);
                assert!(room.0 >= 0.0 && room.1 >= 0.0);
            }
        }
        // The look-ahead's room on an iPhone 16: six cells sideways and
        // almost nothing up or down (§6).
        let phone = DEVICES.iter().find(|d| d.name == "iPhone 16").unwrap().frame(Seating::Room);
        let (x, y) = phone.room_outside(sight);
        assert!((x / 32.0 - 6.2).abs() < 0.1 && y / 32.0 < 0.7, "{x} x {y}");
    }

    #[test]
    fn the_sight_box_is_on_every_screen_that_can_hold_it() {
        // Every shape from square to 4:1, at every common scale and density,
        // from a short side of 375 points up.
        let sight = standard_box();
        for height in [375.0, 600.0, 768.0, 1080.0, 1440.0, 2160.0] {
            for step in 0..=60 {
                let width = (height * (1.0 + step as f32 * 0.05)).round();
                for dpr in [1.0, 1.25, 1.5, 2.0, 2.625, 3.0] {
                    for ppi in [90.0, 220.0, 330.0, 460.0] {
                        let screen = Screen::new(width, height, dpr, ppi).with_panel_width(width * dpr);
                        for seating in [Seating::Room, Seating::Local] {
                            let f = frame(screen, seating, sight, &rules());
                            assert!(f.shows(sight), "{width} x {height} @{dpr} {ppi} ppi ({seating:?}): {:?}", f.visible_cells());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_taller_box_steps_the_iphone_se_outward() {
        let d = DEVICES.iter().find(|d| d.name == "iPhone SE").unwrap();
        // The standard box: 1.30 device pixels per world pixel snaps in to
        // 1.5, which still shows +-7.8 rows.
        let f = d.frame(Seating::Room);
        assert_eq!(f.scale, 1.5);
        // A box of +-8.5 rows does not fit at 1.5, so the snap steps out to
        // 1.0 instead and the screen shows 41.7 x 23.4 cells.
        let tall = SightBox::from_cells(11.5, 8.5);
        assert!(!f.shows(tall));
        let out = frame(d.screen(), Seating::Room, tall, &rules());
        assert_eq!(out.scale, 1.0);
        assert!(out.shows(tall));
        let (cols, rows) = out.visible_cells();
        assert!((cols - 41.7).abs() < 0.05 && (rows - 23.4).abs() < 0.05, "{cols} x {rows}");
        assert!((out.tank_points - 32.0).abs() < 1e-4);
    }

    #[test]
    fn a_room_never_zooms_out_and_an_unknown_size_never_does() {
        let d = DEVICES.iter().find(|d| d.name == "24\" 1080p").unwrap();
        let room = d.frame(Seating::Room);
        assert_eq!(room.scale, 2.0);
        let size_unknown = Screen::new(1920.0, 1080.0, 1.0, 92.0);
        let local = frame(size_unknown, Seating::Local, standard_box(), &rules());
        assert_eq!(local.scale, room.scale, "without millimetres a tank's size is unknown");
        assert_eq!(local.tank_mm, None);
        assert_eq!(local.tank_points, 128.0);
        // Turning the zoom-out's limits loose walks down to half a device
        // pixel per world pixel and stops there.
        let loose = ViewRules { local_min_tank_mm: 0.0, local_max_cells: 1e9, ..rules() };
        assert_eq!(frame(d.screen(), Seating::Local, standard_box(), &loose).scale, 0.5);
        assert_eq!(frame(d.screen(), Seating::Room, standard_box(), &loose).scale, 2.0);
    }

    #[test]
    fn a_degenerate_screen_stays_finite() {
        let screens = [
            Screen::new(0.0, 0.0, 0.0, 0.0),
            Screen::new(f32::NAN, 600.0, f32::NAN, f32::NAN).with_mm_per_point(f32::NAN),
            Screen::new(800.0, -5.0, f32::INFINITY, 100.0).with_mm_per_point(0.25),
        ];
        for screen in screens {
            for seating in [Seating::Room, Seating::Local] {
                let f = frame(screen, seating, standard_box(), &rules());
                assert!(f.visible.0.is_finite() && f.visible.1.is_finite() && f.scale.is_finite() && f.scale > 0.0, "{screen:?}: {f:?}");
            }
        }
    }

    #[test]
    fn the_rules_come_from_the_view_group() {
        let r = rules();
        assert_eq!((r.area_cells, r.aspect_min, r.aspect_max, r.fine_ppi), (578.0, 1.3333, 2.4, 360.0));
        assert_eq!((r.local_min_tank_mm, r.local_max_cells), (25.0, 900.0));
        assert!(r.aspect_min < 4.0 / 3.0, "a 4:3 screen is inside the clamp, not a rounding error outside it");
        for name in ["view_area_cells", "view_aspect_min", "view_aspect_max", "view_fine_ppi", "view_local_min_tank_mm", "view_local_max_cells"] {
            assert_eq!(Tuning::meta(name).map(|m| m.group), Some("view"), "{name}");
        }
        // The standard area is the standard field's.
        assert_eq!(r.area_cells * 32.0 * 32.0, (crate::DEFAULT_SCREEN_WIDTH * crate::DEFAULT_SCREEN_HEIGHT) as f32);
    }

    /// The HUD bar as a followed field map carries it: 32 pixels tall, laid
    /// out across the standard field's width at the least.
    fn hud_bar() -> Bar {
        Bar { height: 32.0, min_width: crate::DEFAULT_SCREEN_WIDTH as f32 }
    }

    /// A 32 px bar with no width of its own: always at the world's scale.
    fn world_bar() -> Bar {
        Bar { height: 32.0, min_width: 0.0 }
    }

    #[test]
    fn no_bar_is_the_plain_framing() {
        for d in &DEVICES {
            for seating in [Seating::Room, Seating::Local] {
                let none = Bar { height: 0.0, min_width: 1088.0 };
                assert_eq!(frame_under_bar(d.screen(), none, seating, standard_box(), &rules()), d.frame(seating), "{}", d.name);
            }
        }
    }

    #[test]
    fn the_world_and_the_bar_fill_every_screen_together() {
        // With the bar on top, the world's height and the bar's make the
        // screen's, the world's width the screen's (inside the clamp),
        // and the sight box is still on screen - for a bar at the world's
        // scale and for the HUD's, which is smaller than the world's where
        // the view is narrower than it.
        let sight = standard_box();
        let check = |name: &str, screen: Screen, seating: Seating| {
            for bar in [world_bar(), hud_bar()] {
                let f = frame_under_bar(screen, bar, seating, sight, &rules());
                let (w, h) = screen.size;
                let p = f.point_scale;
                assert!(f.shows(sight), "{name} ({seating:?}, {bar:?}): {:?} hides the box", f.visible_cells());
                let filled = f.visible.1 * p + bar.points(&f) + 2.0 * f.bars.1;
                assert!((filled - h).abs() < 0.01, "{name} ({seating:?}, {bar:?}): {} under a {} pt bar vs {h}", f.visible.1 * p, bar.points(&f));
                assert!((f.visible.0 * p + 2.0 * f.bars.0 - w).abs() < 0.01, "{name} ({seating:?}): {} at {p} vs {w}", f.visible.0);
                assert!(bar.point_scale(&f) <= p + 1e-6, "{name}: the bar is never drawn larger than the world");
                if f.snapped {
                    assert_eq!(f.scale * 2.0, (f.scale * 2.0).round(), "{name}: a snapped scale stays on whole blocks");
                }
            }
        };
        for d in &DEVICES {
            for seating in [Seating::Room, Seating::Local] {
                check(d.name, d.screen(), seating);
            }
        }
        // Every shape and density a window can take, from a phone's short
        // side up, with and without its physical size.
        for height in [375.0, 393.0, 600.0, 768.0, 900.0, 1080.0, 1440.0] {
            for step in 0..=40 {
                let width = (height * (1.0 + step as f32 * 0.05)).round();
                for dpr in [1.0, 1.5, 2.0, 3.0] {
                    for ppi in [96.0, 220.0, 460.0] {
                        let screen = Screen::new(width, height, dpr, ppi).with_panel_width(width * dpr);
                        for seating in [Seating::Room, Seating::Local] {
                            check(&format!("{width} x {height} @{dpr} {ppi} ppi"), screen, seating);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_bar_that_tips_the_snap_back_and_forth_keeps_the_outward_step() {
        // 590 x 393 points at 2x: under the bar of the outward step (0.5
        // points a world pixel) the snap goes in to 0.75, whose taller bar
        // tips it out to 0.5 again. The outward step stands, filling the
        // screen with the world and its bar.
        let screen = Screen::new(590.0, 393.0, 2.0, 220.0).with_panel_width(1180.0);
        let rules = rules();
        let at = |p: f32| frame(Screen { size: (590.0, 393.0 - 32.0 * p), ..screen }, Seating::Room, standard_box(), &rules).point_scale;
        assert_eq!((at(0.5), at(0.75)), (0.75, 0.5), "the two steps send the framing to each other");
        let f = frame_under_bar(screen, world_bar(), Seating::Room, standard_box(), &rules);
        assert_eq!(f.point_scale, 0.5);
        assert!(((f.visible.1 + 32.0) * f.point_scale - 393.0).abs() < 1e-3, "{:?}", f.visible);
        assert!(f.shows(standard_box()));
    }

    #[test]
    fn a_1080p_monitor_under_the_bar_zooms_out_to_40_cells() {
        // The local zoom-out under the bar: 40 cells across, one fewer row
        // than the bar-less 22.5 - the bar takes it. The view is wider than
        // the HUD's bar, which is then at the world's scale.
        let d = DEVICES.iter().find(|d| d.name == "24\" 1080p").unwrap();
        let f = frame_under_bar(d.screen(), hud_bar(), Seating::Local, standard_box(), &rules());
        assert_eq!(f.visible_cells(), (40.0, 21.5));
        assert_eq!(f.scale, 1.5);
        assert_eq!(hud_bar().points(&f), 48.0);
        // A room never zooms out: the shared area, 30 cells across - under
        // a bar at the world's scale, 64 points; under the HUD's, laid out
        // across 1088 bitmap pixels, 56.5, which leaves the world a few
        // more rows.
        let room = frame_under_bar(d.screen(), world_bar(), Seating::Room, standard_box(), &rules());
        assert_eq!((room.scale, room.visible), (2.0, (960.0, 508.0)));
        let room = frame_under_bar(d.screen(), hud_bar(), Seating::Room, standard_box(), &rules());
        assert_eq!(room.scale, 2.0);
        assert_eq!(room.visible.0, 960.0);
        let bar = hud_bar().points(&room);
        assert!((bar - 32.0 * 1920.0 / 1088.0).abs() < 1e-3, "{bar}");
        assert!((room.visible.1 * 2.0 + bar - 1080.0).abs() < 1e-3, "{:?}", room.visible);
    }

    #[test]
    fn the_yardstick_is_a_tank() {
        assert_eq!(crate::tank::Tank::default().size(), TANK_PX);
        assert_eq!(TANK_PX, 64.0);
    }

    #[test]
    fn the_arena_rule_stops_at_36_by_18() {
        assert_eq!(MapClass::by_size(34.0, 17.0), MapClass::Arena);
        assert_eq!(MapClass::by_size(36.0, 18.0), MapClass::Arena);
        assert_eq!(MapClass::by_size(24.0, 14.0), MapClass::Arena);
        assert_eq!(MapClass::by_size(36.5, 18.0), MapClass::Field);
        assert_eq!(MapClass::by_size(36.0, 18.5), MapClass::Field);
        assert_eq!(MapClass::by_size(37.0, 10.0), MapClass::Field);
        assert_eq!(MapClass::by_size(20.0, 19.0), MapClass::Field);
        assert_eq!(MapClass::by_size(40.0, 20.0), MapClass::Field);
        assert_eq!(MapClass::by_size(96.0, 54.0), MapClass::Field);
        assert!(MapClass::Field.follows() && !MapClass::Arena.follows());
    }
}
