//! What is off screen (docs/large-maps-follow-camera.md section 7): the
//! arrows one seat's screen shows at its edge for the enemies, teammates,
//! frogs and wave gates it cannot see, the lane warning on an enemy lined
//! up to fire on that seat, the arc on its own tank toward whatever just
//! hit it, and the hollow marker an enemy leaves where it slipped out of
//! sight.
//!
//! Presentation only, the `hud.rs` pattern: plain values gathered from the
//! round once a rendered frame, which the renderer draws and nothing else
//! reads. It reads a `Game` - a local round or an online replica, the same
//! code, since a replica carries every tank, shot and frog - and never
//! writes it. Unlike `HudModel` it remembers across frames (when an enemy
//! lined up, when it last fired, where a hidden one was last seen, which
//! gates are flashing), all of it timed on the round clock (`Game::time`),
//! so the indicators stand still while the round does. No RNG, and every
//! walk goes in owner-slot order, so the same round gives the same arrows.
//!
//! Four layers, each testable on its own:
//!
//! - `Awareness::observe_events` reads a simulation frame's events into
//!   `Note`s: who fired and whether it was lined up on this seat, a hit on
//!   this seat and the way it came, a tank through its gate. Call it after
//!   every step, as `Fx::observe_events` is called - a rendered frame that
//!   runs two steps would lose the first one's events otherwise; a frame
//!   already read is skipped.
//! - `Scene::of` reads the round into plain values: every tank, the frogs
//!   and the sky's sight.
//! - `Awareness::frame` applies every rule to a `Scene` on a `ViewFrame`
//!   and lays the result out on the screen's edge as `Indicators`.
//! - `picture` turns one screen's `Indicators` - a seat's, or both seats'
//!   of a couch round on one screen - into the blocks and labels
//!   `render::indicators` paints (`Picture`), in the game's own pixel art.
//!
//! `Awareness::gather` is the first three for one seat. `ScreenAwareness`
//! is all four for a screen, the memory `app.rs` keeps: it feeds every
//! local seat's `Awareness` the events of each step and composes the
//! screen's `Picture` once a frame, with the view built from the frame's
//! camera (`ViewFrame::of_camera`) and the sizes the rows give in points
//! turned into the bitmap's pixels (`in_points`).
//!
//! **The rules.**
//!
//! - *Placement.* An arrow sits where the line from the tank's point on
//!   the screen to its target meets the inset rectangle
//!   (`ViewFrame::inset`). One that lands in a keep-out rectangle (a HUD
//!   cluster, a thumb's rest, the minimap) slides along the rectangle's
//!   edge, round a corner if it must, to the nearest point clear of all of
//!   them (`cast`, `slide`).
//! - *Distance* is size and opacity: full up to `indicator_near_screens`
//!   away, `indicator_far_scale` and `indicator_far_alpha` at
//!   `indicator_far_screens`, where a screen is the view's extent along
//!   the arrow (`screens`, `distance_look`).
//! - *Clustering and priority.* Enemy arrows within `indicator_cluster_pt`
//!   of each other merge into one with a count. At most
//!   `indicator_max_arrows` show, filled by lane threats, teammates, frogs,
//!   flashing gates and then the nearest enemies; the enemies left over
//!   fold into one count per screen edge. Teammates and frogs are never
//!   merged and never left out, past the cap if they must be.
//! - *The lane warning.* An enemy lined up on this seat by the AI's own
//!   fire rule (`lined_up`: the seat on the row or column it faces, within
//!   `enemy_fire_align_px`, inside its attack range) with its line of sight
//!   clear is settling its aim, which is how `ai::act_attack` starts the
//!   settle: the warning runs from that frame, its `settle` reaching 1
//!   after `enemy_aim_settle`, and flashes the frame the enemy fires down
//!   the lane. A replica carries no `Ai`, so the settle is read from what
//!   is drawn - the hull turned down the lane - rather than from the AI's
//!   timer. Only this seat's screen shows it: another seat's `Awareness`
//!   measures the lane against that seat's own tank. An enemy fires at a
//!   seat only from inside the seat's sight box (`ai::in_sight_box`), and
//!   the box is always on screen, so an enemy an arrow points at fires on
//!   nothing yet: its warning is the heads-up that it is lined up down the
//!   lane and closing in to fire.
//! - *Concealment.* An enemy in tall grass gets no arrow unless it fired
//!   within `indicator_reveal_fire_seconds` or stands within
//!   `indicator_reveal_px` of the seat (`concealed`); nor, at night or in
//!   fog, does one further than the sky lets an enemy see
//!   (`Game::enemy_sight`). An enemy that slips out of sight leaves a
//!   last-seen marker where it was last seen, which never moves - it
//!   would give the hidden tank away - and fades over
//!   `indicator_last_seen_seconds`.
//! - *The hit arc* points from this seat's tank back the way the last hit
//!   came, for `indicator_hit_arc_seconds`: up the line a shot flew, at a
//!   beam's or a tesla bolt's source, a flame's nozzle, a rammer, a biting
//!   frog or a missile's launcher. No event names a shot's shooter, so the
//!   shot is looked up where the hit landed - the hit test leaves it there
//!   in its impact frames - and its line read back.
//! - *Gates* flash for `indicator_gate_flash_seconds` from the frame a
//!   tank starts rolling in through one - the first of a wave on the frame
//!   the wave is called (`WaveStarted`, which names no gate), a seat coming
//!   back ahead of the wave's own tanks - and again when it comes through
//!   (`TankEntered`). A teammate in a gate lane, a wreck the wave is
//!   bringing back, shows there; one still lying where it fell has no
//!   arrow.

use std::collections::{BTreeMap, BTreeSet};

use hecs::Entity;

use crate::ai::axis_offsets;
use crate::bullet::Bullet;
use crate::frog::{Frog, Side};
use crate::math::{Color, Rectangle, Vec2};
use crate::plasma::Plasma;
use crate::shell::{Owner, Shell};
use crate::simulation::{Event, Game, HitTarget, RollIn, with_tank};
use crate::tank::{Dir, Tank};
use crate::tuning::{Tuning, tuning};
use crate::view::Camera;
use crate::{OBSTACLE_GRID_SIZE, Position};

/// Flashes this close together (world pixels, three cells) are one gate:
/// the edge of the field, where a tank driving in first shows on a
/// replica, and the lane's inside point, where it comes through.
const GATE_SAME_PX: f32 = 3.0 * OBSTACLE_GRID_SIZE;

/// How close (world pixels) a beam's end, a bolt's or a shot must be to a
/// hit to be what landed it: the wire's quarter pixels with room to spare.
const HIT_MATCH_PX: f32 = 2.0;

/// A round clock this far behind the last one read (seconds) is a new
/// round, not a replica's snapshot landing a little behind its own easing.
const ROUND_RESTART_SECONDS: f32 = 0.5;

// ---- the view -------------------------------------------------------------

/// One edge of the inset rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Edge {
    Top,
    Right,
    Bottom,
    Left,
}

impl Edge {
    /// All four, in `index` order: clockwise from the top.
    pub const ALL: [Edge; 4] = [Edge::Top, Edge::Right, Edge::Bottom, Edge::Left];

    /// Position in `ALL`, for per-edge tables such as `Indicators::folded`.
    pub fn index(self) -> usize {
        self as usize
    }
}

/// One screen's view of the world as plain values: which part of the
/// field it shows, at what size, and where on the screen an arrow may
/// sit. The camera fills it in; a test writes it by hand.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewFrame {
    /// The world rectangle the screen shows, in world pixels.
    pub world: Rectangle,
    /// Screen points per world pixel.
    pub scale: f32,
    /// Where `world`'s top-left corner lands on the screen, in points.
    pub origin: Vec2,
    /// The rectangle the arrows sit on, in screen points: the screen inset
    /// `indicator_inset_pt` inside the safe area (`inset_of`).
    pub inset: Rectangle,
    /// Screen rectangles no arrow may sit in, in points: the HUD clusters,
    /// the thumbs' rests, the minimap where there is one.
    pub keep_out: Vec<Rectangle>,
    /// The part of the screen this view does not show - the other half of
    /// a couch's split screen (`follow::Split`) - which it treats as off
    /// screen, its arrows sitting on the divider; `None` for a view of the
    /// whole screen.
    pub beyond: Option<Beyond>,
}

/// The other half of a split screen, as one view of it sees it: everything
/// past a line, in the screen's points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beyond {
    /// A point on the divider ...
    pub at: Vec2,
    /// ... and the unit normal into the half the view does not show.
    pub normal: Vec2,
    /// How far inside the divider an arrow sits: the `indicator_inset_pt`
    /// the arrows keep from the screen's edge.
    pub inset: f32,
}

impl Beyond {
    /// How far past the divider `p` is, in points; negative on this side.
    fn past(&self, p: Vec2) -> f32 {
        (p.x - self.at.x) * self.normal.x + (p.y - self.at.y) * self.normal.y
    }

    /// The inset rectangle's edge that faces the way the divider does: what
    /// an arrow on the divider counts as for the edge counts.
    fn edge(&self) -> Edge {
        let n = self.normal;
        if n.x.abs() >= n.y.abs() {
            if n.x > 0.0 { Edge::Right } else { Edge::Left }
        } else if n.y > 0.0 {
            Edge::Bottom
        } else {
            Edge::Top
        }
    }

    /// `at`, an arrow's place on the inset rectangle, kept to this side of
    /// the divider: one that sits past the line `inset` inside it moves to
    /// where the line from `origin` toward `to` crosses it, or, for a line
    /// that never does, straight back onto it.
    fn clip(&self, origin: Vec2, to: Vec2, at: Vec2, edge: Edge) -> (Vec2, Edge) {
        let limit = -self.inset;
        if self.past(at) <= limit {
            return (at, edge);
        }
        let d = to - origin;
        let along = d.x * self.normal.x + d.y * self.normal.y;
        let from = self.past(origin);
        let hit = if along > 1e-6 && from < limit { Some(origin + d * ((limit - from) / along)) } else { None };
        let on = hit.unwrap_or_else(|| {
            let back = self.past(at) - limit;
            Vec2::new(at.x - self.normal.x * back, at.y - self.normal.y * back)
        });
        (on, self.edge())
    }
}

impl ViewFrame {
    /// `world` drawn at `scale` points per pixel with its corner at
    /// `origin`, its arrows on `safe` - the screen's safe area - inset by
    /// `indicator_inset_pt`, and nothing kept out yet.
    pub fn new(world: Rectangle, scale: f32, origin: Vec2, safe: Rectangle, t: &Tuning) -> ViewFrame {
        ViewFrame { world, scale, origin, inset: ViewFrame::inset_of(safe, t), keep_out: Vec::new(), beyond: None }
    }

    /// This view as one half of a split screen: the screen past the line
    /// through `at` square to the unit `normal` (in the view's points) is
    /// the other half's, off screen for this one, and its arrows keep
    /// `indicator_inset_pt` inside the line as they do inside the screen's
    /// edge.
    pub fn split_at(self, at: Vec2, normal: Vec2, t: &Tuning) -> ViewFrame {
        ViewFrame { beyond: Some(Beyond { at, normal, inset: t.indicator_inset_pt.max(0.0) }), ..self }
    }

    /// What `camera` draws onto `field`, the field area of the bitmap: the
    /// world rectangle it shows at its scale, the field area's corner where
    /// the view's lands (`Camera::on_field`), and the arrows on the part of
    /// the field area the view covers. Laid out in the bitmap's pixels -
    /// `t` holds the sizes in them (`in_points`); the caller adds the HUD's
    /// corner clusters and the thumbs' rests as keep-outs.
    pub fn of_camera(camera: &Camera, field: Rectangle, t: &Tuning) -> ViewFrame {
        let dest = camera.dest();
        let (x0, y0) = (field.x.max(field.x + dest.x), field.y.max(field.y + dest.y));
        let x1 = (field.x + field.width).min(field.x + dest.x + dest.width);
        let y1 = (field.y + field.height).min(field.y + dest.y + dest.height);
        let shown = Rectangle::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0));
        ViewFrame::new(camera.rect(), camera.scale, Vec2::new(field.x, field.y), shown, t)
    }

    /// `safe` shrunk by `indicator_inset_pt` on every side, never past its
    /// own middle.
    pub fn inset_of(safe: Rectangle, t: &Tuning) -> Rectangle {
        let dx = t.indicator_inset_pt.min(safe.width * 0.5).max(0.0);
        let dy = t.indicator_inset_pt.min(safe.height * 0.5).max(0.0);
        Rectangle::new(safe.x + dx, safe.y + dy, safe.width - 2.0 * dx, safe.height - 2.0 * dy)
    }

    /// A world position on the screen, in points.
    pub fn to_screen(&self, p: Position) -> Vec2 {
        Vec2::new(self.origin.x + (p.x - self.world.x) * self.scale, self.origin.y + (p.y - self.world.y) * self.scale)
    }

    /// Whether the screen shows the world position `p`: inside the view,
    /// and on a split screen on this view's side of the divider.
    pub fn shows(&self, p: Position) -> bool {
        self.world.contains(p) && self.beyond.is_none_or(|b| b.past(self.to_screen(p)) <= 0.0)
    }

    /// The middle of what the screen shows, in world pixels.
    pub fn centre(&self) -> Position {
        Position::new(self.world.x + self.world.width * 0.5, self.world.y + self.world.height * 0.5)
    }
}

/// A touch screen's points to the millimetre: an iOS point, a CSS pixel on
/// a phone's browser and an Android dp are each about a 160th of an inch,
/// near enough to keep an arrow out from under a thumb.
pub const POINTS_PER_MM: f32 = 160.0 / 25.4;

/// Where a touch screen's thumbs rest, which no arrow sits under: a square
/// pad `indicator_thumb_pad_mm` wide whose middle is `indicator_thumb_in_mm`
/// in from each side of `screen` and `indicator_thumb_up_mm` up from its
/// bottom edge, left then right. `screen` is the whole screen, the glass
/// the thumbs lie on, in the units the arrows are laid out in, `points` of
/// them to a point (`POINTS_PER_MM`) - a letterboxed bitmap's pixels reach
/// past the bitmap, so a pad lands under the thumb rather than on the
/// bitmap's corner.
pub fn thumb_rests(screen: Rectangle, points: f32, t: &Tuning) -> [Rectangle; 2] {
    let per_mm = points * POINTS_PER_MM;
    let side = t.indicator_thumb_pad_mm * per_mm;
    let inward = t.indicator_thumb_in_mm * per_mm;
    let middle_y = screen.y + screen.height - t.indicator_thumb_up_mm * per_mm;
    let pad = |middle_x: f32| Rectangle::new(middle_x - side * 0.5, middle_y - side * 0.5, side, side);
    [pad(screen.x + inward), pad(screen.x + screen.width - inward)]
}

// ---- the round as plain values --------------------------------------------

/// The round as the indicators read it, once a rendered frame: plain
/// values, so every rule runs without a `Game` (`Scene::of` reads one).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    /// The round clock (`Game::time`): every duration is measured on it.
    pub time: f32,
    /// This seat's tank, `None` before the round has one.
    pub seat: Option<SeatView>,
    /// Every other tank, in owner-slot order.
    pub tanks: Vec<TankView>,
    /// The frogs, the player's first.
    pub frogs: Vec<FrogView>,
    /// How far an enemy can be seen under the round's sky
    /// (`Game::enemy_sight`) where the sky shortens it - night, a storm,
    /// fog - and `None` under one that hides nothing.
    pub sight: Option<f32>,
    /// Every volcano that is rumbling or erupting: its crater and whether
    /// it is erupting yet.
    pub volcanoes: Vec<(Position, bool)>,
}

/// This seat's own tank.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeatView {
    /// Its owner slot.
    pub slot: usize,
    pub pos: Position,
    pub wreck: bool,
    /// Driving in through a gate: the gate's point, `None` on the field.
    pub gate: Option<Position>,
}

impl SeatView {
    /// Standing on the field: neither a wreck nor in a gate lane.
    pub fn active(&self) -> bool {
        !self.wreck && self.gate.is_none()
    }
}

/// Another tank, a teammate's or an enemy's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TankView {
    /// Its owner slot.
    pub slot: usize,
    /// The seat it is, `None` for an enemy.
    pub seat: Option<u8>,
    pub pos: Position,
    pub wreck: bool,
    /// Driving in through a gate (`waves::RollIn`, or off the field on a
    /// replica): the gate's point, `None` on the field.
    pub gate: Option<Position>,
    /// Standing in a tall-grass cell (`grass::conceals`).
    pub in_grass: bool,
    /// Lined up to fire on this seat with its line of sight clear
    /// (`lined_up` and `PresentWorld::line_of_sight`): its aim is settling.
    /// Only ever set on an enemy.
    pub lane: bool,
}

impl TankView {
    /// An enemy standing on the field.
    pub fn is_enemy(&self) -> bool {
        self.seat.is_none() && !self.wreck && self.gate.is_none()
    }

    /// A teammate on the field or driving back in through a gate.
    pub fn is_teammate(&self) -> bool {
        self.seat.is_some() && !self.wreck
    }
}

/// A frog: the player's, or the enemy's a Hunt round is after.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrogView {
    pub side: Side,
    pub pos: Position,
    pub alive: bool,
}

/// One thing a simulation frame did that the indicators remember, read
/// from its events by `Awareness::observe_events`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Note {
    /// A new round: everything remembered goes.
    RoundStarted,
    /// The tank in `slot` fired; `at_seat` when it was lined up on this
    /// seat as it did (`lined_up`), which flashes its warning.
    Fired { slot: usize, at_seat: bool },
    /// This seat was hit from `from`: a unit vector from its tank toward
    /// where the hit came from.
    Hit { from: Vec2 },
    /// The tank in `slot` came through its gate at `at`.
    Entered { slot: usize, at: Position },
}

// ---- what the screen shows -------------------------------------------------

/// Where an indicator sits on the inset rectangle and which way it points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgePoint {
    /// In screen points.
    pub at: Vec2,
    pub edge: Edge,
    /// Unit vector from `at` toward the target on the screen (y down).
    pub dir: Vec2,
}

/// An enemy lined up on this seat's row or column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneWarning {
    /// How far its aim has settled: 0 the frame it lined up, 1 once
    /// `enemy_aim_settle` has passed and it may fire; 0 once it has left
    /// the lane and only its shot's flash is left.
    pub settle: f32,
    /// 1 the frame it fired down the lane, falling to 0 over
    /// `indicator_fire_flash_seconds`; 0 before it has.
    pub flash: f32,
}

/// What an arrow points at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArrowKind {
    /// One enemy, or `count` whose arrows landed together; `warning` when
    /// the one it was placed for is lined up on this seat.
    Enemy { count: u32, warning: Option<LaneWarning> },
    /// Another seat: its number (the `P2` label and the ring colour,
    /// `tank::team_color`), and whether it is driving back in through a
    /// gate.
    Teammate { seat: u8, entering: bool },
    /// A frog: the player's (Protect), or the enemy's (Hunt's objective).
    Frog { side: Side },
    /// A wave gate a tank is rolling in through; `flash` as in `GateFlash`.
    Gate { flash: f32 },
    /// A volcano that is rumbling or erupting (docs/volcano.md): the
    /// warning that bombs are coming, `erupting` once they are.
    Volcano { erupting: bool },
}

/// One arrow at the edge of the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arrow {
    pub kind: ArrowKind,
    pub place: EdgePoint,
    /// What it points at, in world pixels: the nearest of a merged arrow's
    /// enemies.
    pub target: Position,
    /// Size factor, 1 down to `indicator_far_scale`.
    pub scale: f32,
    /// Opacity, 1 down to `indicator_far_alpha`.
    pub alpha: f32,
    /// How far the target is, in cells, from the point the arrows are cast
    /// from - the seat's tank, or the screen's middle while the screen does
    /// not show it: the frog's label.
    pub cells: f32,
}

/// The enemies past `indicator_max_arrows` on one edge, as one count.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EdgeCount {
    /// How many enemies.
    pub count: u32,
    /// Whether one of them is lined up on this seat.
    pub warning: bool,
    /// Where the first of them - the most urgent, else the nearest - would
    /// have sat, in screen points.
    pub at: Vec2,
}

/// The spot an enemy was last seen at before it slipped out of sight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LastSeen {
    /// In world pixels. It never moves.
    pub at: Position,
    /// The same spot on the screen, in points.
    pub screen: Vec2,
    /// For a spot off the screen, where its hollow arrow sits.
    pub edge: Option<EdgePoint>,
    /// 1 when it was left, falling to 0 over `indicator_last_seen_seconds`.
    pub fade: f32,
}

/// The arc on this seat's own tank toward whatever just hit it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitArc {
    /// Unit vector from the tank toward where the hit came from (y down).
    pub dir: Vec2,
    /// 1 on the hit, falling to 0 over `indicator_hit_arc_seconds`.
    pub fade: f32,
    /// Where the tank stands this frame, in world pixels: the arc's centre.
    pub at: Position,
}

/// A wave gate flashing because a tank is rolling in through it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GateFlash {
    /// In world pixels.
    pub at: Position,
    /// The same spot on the screen, in points.
    pub screen: Vec2,
    /// Whether the screen shows it; one that does not also has an arrow
    /// while there is room for it.
    pub on_screen: bool,
    /// 1 when a tank last started in or came through, falling to 0 over
    /// `indicator_gate_flash_seconds`.
    pub flash: f32,
}

/// Everything one seat's screen shows of what it cannot see, for one
/// frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Indicators {
    /// In priority order: lane threats, teammates, frogs, gates, then the
    /// other enemies by distance.
    pub arrows: Vec<Arrow>,
    /// The enemies that did not fit, one count per edge (`Edge::index`).
    pub folded: [EdgeCount; 4],
    /// Newest first, at most `indicator_max_arrows`.
    pub last_seen: Vec<LastSeen>,
    pub hit_arc: Option<HitArc>,
    /// Every gate flashing, on the screen or off it.
    pub gates: Vec<GateFlash>,
    /// Every enemy in sight, on the screen or off it, in owner-slot order
    /// (world pixels): what a minimap draws, under the same grass and sky
    /// as the arrows.
    pub in_sight: Vec<Position>,
}

// ---- the rules, pure ----------------------------------------------------------

/// Where the line from `from` toward `to` (screen points) leaves `inset`,
/// and through which edge; a `from` outside `inset` is first brought to
/// its nearest point inside. `None` when the two points are one.
pub fn cast(inset: Rectangle, from: Vec2, to: Vec2) -> Option<(Vec2, Edge)> {
    let (l, t) = (inset.x, inset.y);
    let (r, b) = ((l + inset.width).max(l), (t + inset.height).max(t));
    let from = Vec2::new(from.x.clamp(l, r), from.y.clamp(t, b));
    let d = to - from;
    if d.length_sqr() < 1e-8 {
        return None;
    }
    let tx = if d.x > 0.0 {
        (r - from.x) / d.x
    } else if d.x < 0.0 {
        (l - from.x) / d.x
    } else {
        f32::INFINITY
    };
    let ty = if d.y > 0.0 {
        (b - from.y) / d.y
    } else if d.y < 0.0 {
        (t - from.y) / d.y
    } else {
        f32::INFINITY
    };
    let (s, edge) = if tx < ty {
        (tx, if d.x > 0.0 { Edge::Right } else { Edge::Left })
    } else {
        (ty, if d.y > 0.0 { Edge::Bottom } else { Edge::Top })
    };
    let p = from + d * s;
    Some((Vec2::new(p.x.clamp(l, r), p.y.clamp(t, b)), edge))
}

/// `at` on `inset`'s `edge`, moved along the rectangle's outline - round a
/// corner if it must - to the nearest point at least `margin` clear of
/// every rectangle in `keep_out`. A point already clear stays; one with no
/// clear point anywhere stays too.
pub fn slide(inset: Rectangle, at: Vec2, edge: Edge, keep_out: &[Rectangle], margin: f32) -> (Vec2, Edge) {
    let ring = Ring::of(inset);
    if ring.len() <= 0.0 {
        return (at, edge);
    }
    let blocked = ring.blocked(keep_out, margin);
    let s0 = ring.param(at, edge);
    if !ring.is_blocked(s0, &blocked) {
        return (at, edge);
    }
    match ring.nearest_free(s0, &blocked) {
        Some(s) => ring.point(s),
        None => (at, edge),
    }
}

/// The inset rectangle's outline as one loop, measured clockwise from the
/// top-left corner: the top edge, the right, the bottom, the left.
struct Ring {
    l: f32,
    t: f32,
    w: f32,
    h: f32,
}

impl Ring {
    fn of(r: Rectangle) -> Ring {
        Ring { l: r.x, t: r.y, w: r.width.max(0.0), h: r.height.max(0.0) }
    }

    fn len(&self) -> f32 {
        2.0 * (self.w + self.h)
    }

    /// How far round the loop `p`, on `edge`, lies.
    fn param(&self, p: Vec2, edge: Edge) -> f32 {
        let (w, h) = (self.w, self.h);
        match edge {
            Edge::Top => p.x - self.l,
            Edge::Right => w + (p.y - self.t),
            Edge::Bottom => w + h + (self.l + w - p.x),
            Edge::Left => 2.0 * w + h + (self.t + h - p.y),
        }
    }

    /// The point `s` round the loop, and the edge it is on.
    fn point(&self, s: f32) -> (Vec2, Edge) {
        let (w, h) = (self.w, self.h);
        let (r, b) = (self.l + w, self.t + h);
        let s = s.rem_euclid(self.len());
        if s < w {
            (Vec2::new(self.l + s, self.t), Edge::Top)
        } else if s < w + h {
            (Vec2::new(r, self.t + (s - w)), Edge::Right)
        } else if s < 2.0 * w + h {
            (Vec2::new(r - (s - w - h), b), Edge::Bottom)
        } else {
            (Vec2::new(self.l, b - (s - 2.0 * w - h)), Edge::Left)
        }
    }

    /// The stretches of the loop inside any of `rects` grown by `margin`,
    /// merged and in order; one that runs over the loop's start ends past
    /// `len`.
    fn blocked(&self, rects: &[Rectangle], margin: f32) -> Vec<(f32, f32)> {
        let (w, h) = (self.w, self.h);
        let (l, t, r, b) = (self.l, self.t, self.l + w, self.t + h);
        let mut spans: Vec<(f32, f32)> = Vec::new();
        for k in rects {
            let (x0, x1) = (k.x - margin, k.x + k.width + margin);
            let (y0, y1) = (k.y - margin, k.y + k.height + margin);
            let (ax, bx) = (x0.max(l), x1.min(r));
            let (ay, by) = (y0.max(t), y1.min(b));
            if ax <= bx {
                if (y0..=y1).contains(&t) {
                    spans.push((ax - l, bx - l));
                }
                if (y0..=y1).contains(&b) {
                    spans.push((w + h + r - bx, w + h + r - ax));
                }
            }
            if ay <= by {
                if (x0..=x1).contains(&r) {
                    spans.push((w + ay - t, w + by - t));
                }
                if (x0..=x1).contains(&l) {
                    spans.push((2.0 * w + h + b - by, 2.0 * w + h + b - ay));
                }
            }
        }
        spans.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let mut merged: Vec<(f32, f32)> = Vec::new();
        for (a, b) in spans {
            match merged.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        // A stretch running off the loop's end into one at its start - a
        // keep-out over the top-left corner - is one stretch.
        let len = self.len();
        if merged.len() > 1 && merged[0].0 <= 0.0 && merged[merged.len() - 1].1 >= len {
            let first = merged.remove(0);
            let last = merged.len() - 1;
            merged[last].1 = len + first.1;
        }
        merged
    }

    /// Whether `s` lies strictly inside one of `blocked`.
    fn is_blocked(&self, s: f32, blocked: &[(f32, f32)]) -> bool {
        let len = self.len();
        blocked.iter().any(|&(a, b)| (a < s && s < b) || (a < s + len && s + len < b))
    }

    /// The point round the loop nearest `s0` that no stretch of `blocked`
    /// covers, the earlier of two as near; `None` when they cover it all.
    fn nearest_free(&self, s0: f32, blocked: &[(f32, f32)]) -> Option<f32> {
        let len = self.len();
        if blocked.iter().any(|&(a, b)| b - a >= len) {
            return None;
        }
        let mut best: Option<(f32, f32)> = None;
        for &(a, b) in blocked {
            for c in [a.rem_euclid(len), b.rem_euclid(len)] {
                if self.is_blocked(c, blocked) {
                    continue;
                }
                let d = (c - s0).rem_euclid(len);
                let d = d.min(len - d);
                if best.is_none_or(|(bd, bc)| d < bd || (d == bd && c < bc)) {
                    best = Some((d, c));
                }
            }
        }
        best.map(|(_, c)| c)
    }
}

/// How many screens `to` is from `from` (world pixels), a screen being the
/// extent of `world` - the view's rectangle - along the line between them.
pub fn screens(world: Rectangle, from: Position, to: Position) -> f32 {
    let d = to - from;
    let len = d.length();
    if len <= 0.0 {
        return 0.0;
    }
    let (ux, uy) = ((d.x / len).abs(), (d.y / len).abs());
    let along_x = if ux > 0.0 { world.width / ux } else { f32::INFINITY };
    let along_y = if uy > 0.0 { world.height / uy } else { f32::INFINITY };
    let extent = along_x.min(along_y);
    if extent > 0.0 { len / extent } else { f32::INFINITY }
}

/// An arrow's size factor and opacity at `screens` away: full up to
/// `indicator_near_screens`, `indicator_far_scale` and
/// `indicator_far_alpha` from `indicator_far_screens` on, in a straight
/// line between.
pub fn distance_look(screens: f32, t: &Tuning) -> (f32, f32) {
    let span = t.indicator_far_screens - t.indicator_near_screens;
    let f = if span > 0.0 {
        ((screens - t.indicator_near_screens) / span).clamp(0.0, 1.0)
    } else if screens > t.indicator_near_screens {
        1.0
    } else {
        0.0
    };
    (1.0 + (t.indicator_far_scale - 1.0) * f, 1.0 + (t.indicator_far_alpha - 1.0) * f)
}

/// Whether an enemy stays hidden in tall grass: it stands in a grass cell
/// (`in_grass`, the cell rule `grass::conceals` the AI hides a player by)
/// and has neither fired within `indicator_reveal_fire_seconds`
/// (`since_fired`, seconds) nor come within `indicator_reveal_px` of the
/// seat (`distance`). No drawing hides a tank in grass - the tufts cover
/// at most part of a hull (`grass.rs`) - so this is the one place the
/// rule lives; a sprite that hides one should ask it too.
pub fn concealed(in_grass: bool, since_fired: Option<f32>, distance: f32, t: &Tuning) -> bool {
    in_grass && !since_fired.is_some_and(|s| s <= t.indicator_reveal_fire_seconds) && distance > t.indicator_reveal_px
}

/// Whether an enemy at `enemy` facing `facing` is lined up on a seat at
/// `seat` by the AI's aim (`ai::act_attack`): the seat ahead on the axis
/// it faces, within `enemy_fire_align_px` of that line and within its
/// attack range - `enemy_attack_range`, never past `sight`, what the sky
/// lets it see (`Game::enemy_sight`). The seat's sight box is left out on
/// purpose: the AI fires only from inside it, but the box is always on
/// screen, so every enemy an arrow points at stands outside it, and the
/// warning is the heads-up that one is lined up and closing in. The line
/// of sight is the caller's to ask.
pub fn lined_up(enemy: Position, facing: Dir, seat: Position, sight: f32, t: &Tuning) -> bool {
    let dir = Dir::toward(enemy, seat);
    let (off_axis, forward) = axis_offsets(enemy, seat, dir);
    facing == dir && forward > 0.0 && off_axis <= t.enemy_fire_align_px && enemy.distance_to(seat) <= t.enemy_attack_range.min(sight)
}

/// The cardinal direction a hull at `rotation` degrees faces, to the
/// nearest quarter turn (a replica's heading travels quantised).
pub fn facing_of(rotation: f32) -> Dir {
    match ((rotation / 90.0).round() as i32).rem_euclid(4) {
        0 => Dir::Up,
        1 => Dir::Right,
        2 => Dir::Down,
        _ => Dir::Left,
    }
}

/// 1 at age 0, falling to 0 at `total` seconds; 0 when `total` is.
fn fade(age: f32, total: f32) -> f32 {
    if total <= 0.0 { 0.0 } else { (1.0 - age.max(0.0) / total).clamp(0.0, 1.0) }
}

/// `v` scaled to length 1, `None` when it has none.
fn unit(v: Vec2) -> Option<Vec2> {
    let len = v.length();
    (len > 1e-4).then(|| v / len)
}

// ---- the memory --------------------------------------------------------------

/// A spot an enemy was last seen at.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Mark {
    slot: usize,
    at: Position,
    since: f32,
}

/// The last hit on this seat.
#[derive(Clone, Copy, Debug, PartialEq)]
struct HitMark {
    from: Vec2,
    since: f32,
}

/// A gate flashing.
#[derive(Clone, Copy, Debug, PartialEq)]
struct GateMark {
    at: Position,
    since: f32,
}

/// One seat's off-screen awareness across frames: the memory the rules
/// need and the calls that feed it. One per seat on a screen - a couch
/// round split in two keeps one for each half.
#[derive(Clone, Debug, Default)]
pub struct Awareness {
    /// The seat this memory is for; another forgets everything.
    seat: Option<u8>,
    /// The seed of the round it was read from (`Game::round_seed`);
    /// another round forgets everything.
    round: Option<u64>,
    /// The simulation frame whose events were read last.
    last_frame: Option<u64>,
    /// The round clock at the last call.
    clock: f32,
    /// When each tank last fired.
    fired: BTreeMap<usize, f32>,
    /// When each enemy lined up on this seat, while it stays lined up.
    lined: BTreeMap<usize, f32>,
    /// When each enemy last fired while lined up on this seat.
    flashed: BTreeMap<usize, f32>,
    /// Where each enemy in sight on the last frame stood.
    seen: BTreeMap<usize, Position>,
    /// Where enemies slipped out of sight, oldest first.
    last_seen: Vec<Mark>,
    hit: Option<HitMark>,
    gates: Vec<GateMark>,
    /// The tanks driving in through a gate on the last frame.
    entering: BTreeSet<usize>,
}

impl Awareness {
    pub fn new() -> Awareness {
        Awareness::default()
    }

    /// This frame's indicators for `seat` on `view`: the events not yet
    /// read, then the round, then every rule. What the renderer calls once
    /// a frame, after `observe_events` has run after every step.
    pub fn gather(&mut self, game: &Game, seat: u8, view: &ViewFrame) -> Indicators {
        self.observe_events(game, seat);
        let scene = Scene::of(game, seat);
        self.frame(&scene, view, &tuning())
    }

    /// Read the events of the simulation frame `game` is at, once: what
    /// fired, a hit on `seat` and the way it came, a tank through its gate.
    /// A frame already read is skipped. Another round - a new seed, or a
    /// frame counter that went back, which is a restart on a pinned seed -
    /// forgets everything first, as does a replica taking the screen from
    /// the local round.
    pub fn observe_events(&mut self, game: &Game, seat: u8) {
        self.for_seat(seat);
        let round = Some(game.round_seed());
        if self.round != round {
            self.forget();
            self.round = round;
        }
        let frame = game.frame();
        match self.last_frame {
            Some(last) if last == frame => return,
            Some(last) if frame < last => self.forget(),
            _ => {}
        }
        self.last_frame = Some(frame);
        if game.events().is_empty() {
            return;
        }
        let t = tuning();
        let sight = game.enemy_sight();
        let first_enemy = game.first_enemy_slot();
        let me_slot = seat as usize;
        let me = game.seat(me_slot).map(|e| with_tank(&game.world, e, |tk| (e, tk.position, tk.hull_bbox_world(), tk.is_wreck())));
        let tank_at = |slot: usize| -> Option<(Position, Dir)> {
            game.world.query::<&Tank>().iter().find(|tk| tk.owner_slot() == slot).map(|tk| (tk.position, facing_of(tk.rotation)))
        };
        let mut notes = Vec::new();
        for event in game.events() {
            let note = match *event {
                Event::RoundStarted { .. } => Some(Note::RoundStarted),
                Event::Fired { slot, .. } => {
                    let at_seat = slot >= first_enemy
                        && me.is_some_and(|(_, pos, _, wreck)| {
                            !wreck && tank_at(slot).is_some_and(|(at, facing)| lined_up(at, facing, pos, sight, &t))
                        });
                    Some(Note::Fired { slot, at_seat })
                }
                Event::Hit { target: HitTarget::Player { player }, x, y, .. } if player == seat => me
                    .and_then(|(entity, pos, hull, _)| hit_from(game, entity, pos, hull, Position::new(x, y)))
                    .map(|from| Note::Hit { from }),
                Event::Ram { slot, other_slot, .. } if slot == me_slot || other_slot == me_slot => {
                    let other = if slot == me_slot { other_slot } else { slot };
                    me.zip(tank_at(other)).and_then(|((_, pos, _, _), (at, _))| unit(at - pos)).map(|from| Note::Hit { from })
                }
                Event::FrogBite { side, slot, .. } if slot == me_slot => {
                    let frog = [game.frog, game.enemy_frog]
                        .into_iter()
                        .flatten()
                        .filter_map(|e| game.world.get::<&Frog>(e).ok().map(|f| (f.side, f.position)))
                        .find(|&(s, _)| s == side);
                    me.zip(frog).and_then(|((_, pos, _, _), (_, at))| unit(at - pos)).map(|from| Note::Hit { from })
                }
                // A missile bursts with no `Hit`: an enemy's that came down
                // on this seat points back at its launcher, or at the burst
                // where the launcher is gone.
                Event::MissileBlast { slot, x, y } if slot >= first_enemy => me.and_then(|(_, pos, hull, _)| {
                    let blast = Position::new(x, y);
                    let reach = t.missile_blast_radius + hull.1.x.max(hull.1.y);
                    if blast.distance_to(pos) > reach {
                        return None;
                    }
                    let source = tank_at(slot).map_or(blast, |(at, _)| at);
                    unit(source - pos).map(|from| Note::Hit { from })
                }),
                Event::TankEntered { slot } => tank_at(slot).map(|(at, _)| Note::Entered { slot, at }),
                _ => None,
            };
            notes.extend(note);
        }
        for note in notes {
            self.note(game.time, note);
        }
    }

    /// Remember one thing that happened at round time `time`.
    pub fn note(&mut self, time: f32, note: Note) {
        self.tick(time);
        match note {
            Note::RoundStarted => self.forget(),
            Note::Fired { slot, at_seat } => {
                self.fired.insert(slot, time);
                if at_seat {
                    self.flashed.insert(slot, time);
                }
            }
            Note::Hit { from } => self.hit = Some(HitMark { from, since: time }),
            Note::Entered { at, .. } => self.flash_gate(at, time),
        }
    }

    /// Every rule applied to `scene` on `view`: what this frame's screen
    /// shows of everything it cannot see.
    pub fn frame(&mut self, scene: &Scene, view: &ViewFrame, t: &Tuning) -> Indicators {
        self.tick(scene.time);
        let now = scene.time;
        // Arrows are cast from the seat's tank while the screen shows it,
        // and from the middle of the screen while it does not - a wreck
        // spectating, a seat in a gate lane.
        let anchor = match scene.seat {
            Some(s) if s.active() && view.shows(s.pos) => s.pos,
            _ => view.centre(),
        };
        let seat_active = scene.seat.is_some_and(|s| s.active());

        // A tank first seen driving in flashes its gate.
        let mut entering = BTreeSet::new();
        let rolling = scene.seat.iter().filter_map(|s| s.gate.map(|g| (s.slot, g)));
        let rolling = rolling.chain(scene.tanks.iter().filter_map(|tv| tv.gate.map(|g| (tv.slot, g))));
        for (slot, gate) in rolling.collect::<Vec<_>>() {
            if !self.entering.contains(&slot) {
                self.flash_gate(gate, now);
            }
            entering.insert(slot);
        }
        self.entering = entering;

        // Each enemy on the field: its lane, and whether it is in sight.
        let mut live = BTreeSet::new();
        let mut visible: Vec<&TankView> = Vec::new();
        for tv in scene.tanks.iter().filter(|tv| tv.is_enemy()) {
            live.insert(tv.slot);
            if tv.lane && seat_active {
                self.lined.entry(tv.slot).or_insert(now);
            } else {
                self.lined.remove(&tv.slot);
            }
            let distance = anchor.distance_to(tv.pos);
            let since_fired = self.fired.get(&tv.slot).map(|&at| now - at);
            let hidden = concealed(tv.in_grass, since_fired, distance, t) || scene.sight.is_some_and(|s| distance > s);
            if hidden {
                if let Some(at) = self.seen.remove(&tv.slot) {
                    self.last_seen.retain(|m| m.slot != tv.slot);
                    self.last_seen.push(Mark { slot: tv.slot, at, since: now });
                }
            } else {
                self.seen.insert(tv.slot, tv.pos);
                self.last_seen.retain(|m| m.slot != tv.slot);
                visible.push(tv);
            }
        }
        // A wreck or a tank gone hides nothing: forget it, marker and all.
        self.lined.retain(|slot, _| live.contains(slot));
        self.seen.retain(|slot, _| live.contains(slot));
        self.flashed.retain(|slot, at| live.contains(slot) && now - *at < t.indicator_fire_flash_seconds);
        self.fired.retain(|_, at| now - *at <= t.indicator_reveal_fire_seconds);
        self.last_seen.retain(|m| live.contains(&m.slot) && now - m.since < t.indicator_last_seen_seconds);
        self.hit = self.hit.filter(|h| now - h.since < t.indicator_hit_arc_seconds);
        self.gates.retain(|g| now - g.since < t.indicator_gate_flash_seconds);

        let origin = view.to_screen(anchor);
        let margin = t.indicator_arrow_pt * 0.5;
        let arrow = |kind: ArrowKind, target: Position| -> Option<Arrow> {
            let place = place(view, origin, target, margin)?;
            let (scale, alpha) = distance_look(screens(view.world, anchor, target), t);
            let cells = anchor.distance_to(target) / OBSTACLE_GRID_SIZE;
            Some(Arrow { kind, place, target, scale, alpha, cells })
        };

        // The enemies off the screen: lane threats first, the most urgent
        // leading, then the rest nearest first; each joins the first arrow
        // already within `indicator_cluster_pt` of its own.
        let in_sight = visible.iter().map(|tv| tv.pos).collect();
        let mut threats: Vec<(LaneWarning, f32, &TankView)> = Vec::new();
        let mut plain: Vec<(f32, &TankView)> = Vec::new();
        for tv in visible.into_iter().filter(|tv| !view.shows(tv.pos)) {
            let distance = anchor.distance_to(tv.pos);
            match self.warning(tv.slot, now, t) {
                Some(w) => threats.push((w, distance, tv)),
                None => plain.push((distance, tv)),
            }
        }
        threats.sort_by(|a, b| {
            b.0.flash.total_cmp(&a.0.flash).then(b.0.settle.total_cmp(&a.0.settle)).then(a.1.total_cmp(&b.1)).then(a.2.slot.cmp(&b.2.slot))
        });
        plain.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.slot.cmp(&b.1.slot)));
        let ordered = threats.into_iter().map(|(w, _, tv)| (Some(w), tv)).chain(plain.into_iter().map(|(_, tv)| (None, tv)));
        let mut enemies: Vec<Arrow> = Vec::new();
        for (warning, tv) in ordered {
            let Some(a) = arrow(ArrowKind::Enemy { count: 1, warning }, tv.pos) else { continue };
            let near = enemies.iter_mut().find(|e| e.place.at.distance_to(a.place.at) <= t.indicator_cluster_pt);
            match near {
                Some(Arrow { kind: ArrowKind::Enemy { count, .. }, .. }) => *count += 1,
                _ => enemies.push(a),
            }
        }
        let threat_count = enemies.iter().take_while(|a| matches!(a.kind, ArrowKind::Enemy { warning: Some(_), .. })).count();

        // Teammates and frogs: never merged, never left out.
        let mut kept: Vec<Arrow> = Vec::new();
        for tv in scene.tanks.iter().filter(|tv| tv.is_teammate() && !view.shows(tv.pos)) {
            let seat = tv.seat.expect("a teammate has a seat");
            kept.extend(arrow(ArrowKind::Teammate { seat, entering: tv.gate.is_some() }, tv.pos));
        }
        for frog in scene.frogs.iter().filter(|f| f.alive && !view.shows(f.pos)) {
            kept.extend(arrow(ArrowKind::Frog { side: frog.side }, frog.pos));
        }
        // A volcano waking off the screen: never merged, never left out -
        // its bombs are on their way.
        for &(at, erupting) in scene.volcanoes.iter().filter(|(at, _)| !view.shows(*at)) {
            kept.extend(arrow(ArrowKind::Volcano { erupting }, at));
        }

        // Gates off the screen, the most recent first.
        let mut gate_marks: Vec<&GateMark> = self.gates.iter().filter(|g| !view.shows(g.at)).collect();
        gate_marks.sort_by(|a, b| b.since.total_cmp(&a.since));
        let gate_arrows = gate_marks
            .into_iter()
            .filter_map(|g| arrow(ArrowKind::Gate { flash: fade(now - g.since, t.indicator_gate_flash_seconds) }, g.at));

        // The cap, by priority: the lane threats take it first, the
        // teammates and the frogs show whatever is left of it - past it if
        // they must - and the room after them goes to the gates, then the
        // nearest enemies. The enemies past it fold into a count per edge.
        let mut folded = [EdgeCount::default(); 4];
        let mut fold = |a: &Arrow| {
            if let ArrowKind::Enemy { count, warning } = a.kind {
                let edge = &mut folded[a.place.edge.index()];
                if edge.count == 0 {
                    edge.at = a.place.at;
                }
                edge.count += count;
                edge.warning |= warning.is_some();
            }
        };
        let threats_shown = threat_count.min(t.indicator_max_arrows);
        let room = t.indicator_max_arrows.saturating_sub(threats_shown + kept.len());
        let mut arrows: Vec<Arrow> = enemies[..threats_shown].to_vec();
        arrows.extend(kept);
        enemies[threats_shown..threat_count].iter().for_each(&mut fold);
        let rest = gate_arrows.chain(enemies[threat_count..].iter().copied());
        for (i, a) in rest.enumerate() {
            if i < room {
                arrows.push(a);
            } else {
                fold(&a);
            }
        }

        let last_seen = self
            .last_seen
            .iter()
            .rev()
            .take(t.indicator_max_arrows)
            .map(|m| LastSeen {
                at: m.at,
                screen: view.to_screen(m.at),
                edge: if view.shows(m.at) { None } else { place(view, origin, m.at, margin) },
                fade: fade(now - m.since, t.indicator_last_seen_seconds),
            })
            .collect();
        let gates = self
            .gates
            .iter()
            .map(|g| GateFlash {
                at: g.at,
                screen: view.to_screen(g.at),
                on_screen: view.shows(g.at),
                flash: fade(now - g.since, t.indicator_gate_flash_seconds),
            })
            .collect();
        let hit_arc = self.hit.zip(scene.seat).map(|(h, s)| HitArc { dir: h.from, fade: fade(now - h.since, t.indicator_hit_arc_seconds), at: s.pos });
        Indicators { arrows, folded, last_seen, hit_arc, gates, in_sight }
    }

    /// The warning on the enemy in `slot`: while it stays lined up on this
    /// seat, and while the flash of a shot it fired down the lane lasts.
    fn warning(&self, slot: usize, now: f32, t: &Tuning) -> Option<LaneWarning> {
        let settle = self.lined.get(&slot).map(|&since| {
            if t.enemy_aim_settle > 0.0 { ((now - since) / t.enemy_aim_settle).clamp(0.0, 1.0) } else { 1.0 }
        });
        let flash = self.flashed.get(&slot).map_or(0.0, |&at| fade(now - at, t.indicator_fire_flash_seconds));
        (settle.is_some() || flash > 0.0).then(|| LaneWarning { settle: settle.unwrap_or(0.0), flash })
    }

    /// Flash the gate at `at`, or the one already flashing beside it.
    fn flash_gate(&mut self, at: Position, time: f32) {
        match self.gates.iter_mut().find(|g| g.at.distance_to(at) <= GATE_SAME_PX) {
            Some(g) => g.since = time,
            None => self.gates.push(GateMark { at, since: time }),
        }
    }

    /// Move the clock to `time`; one far behind it is a new round.
    fn tick(&mut self, time: f32) {
        if time + ROUND_RESTART_SECONDS < self.clock {
            self.forget();
        }
        self.clock = time;
    }

    /// Start over for `seat` if this memory was another's.
    fn for_seat(&mut self, seat: u8) {
        if self.seat != Some(seat) {
            *self = Awareness { seat: Some(seat), ..Awareness::default() };
        }
    }

    /// Forget the round: everything but whose memory this is, which round
    /// it reads and how far it has read.
    fn forget(&mut self) {
        let (seat, round, last_frame, clock) = (self.seat, self.round, self.last_frame, self.clock);
        *self = Awareness { seat, round, last_frame, clock, ..Awareness::default() };
    }
}

/// One screen's awareness: an `Awareness` for each local seat it shows -
/// the one seat of a phone or a room, or both seats of a couch round on
/// one screen - in the order `picture` reads them, the first one's arrows
/// the screen's. A seat list that changes gives a slot whose seat changed
/// a fresh memory (`Awareness::observe_events`), and a new round forgets
/// on its own, so the caller only says which seats are on the screen.
#[derive(Clone, Debug, Default)]
pub struct ScreenAwareness {
    seats: Vec<Awareness>,
    /// The indicators `picture` composed last, one per seat: which enemies
    /// are in sight and which gates flash, for the minimap (`shown`).
    shown: Vec<Indicators>,
}

impl ScreenAwareness {
    /// Read the events of the frame `game` is at for each of `seats`:
    /// after every step of a local round, and once a frame on a replica.
    pub fn observe_events(&mut self, game: &Game, seats: &[u8]) {
        self.seats.resize_with(seats.len(), Awareness::new);
        for (memory, &seat) in self.seats.iter_mut().zip(seats) {
            memory.observe_events(game, seat);
        }
    }

    /// This frame's picture of what `seats` cannot see on `view`, its
    /// pulses on the round clock: what `Awareness::gather` does per seat,
    /// then `picture`, every size in points `points` of `view`'s units each
    /// (`in_points`) - a bitmap shown smaller than its own pixels draws its
    /// arrows larger, so they stay the size the rows give on the glass.
    pub fn picture(&mut self, game: &Game, seats: &[u8], view: &ViewFrame, points: f32) -> Picture {
        self.seats.resize_with(seats.len(), Awareness::new);
        let t = in_points(&tuning(), points);
        self.shown.clear();
        for (memory, &seat) in self.seats.iter_mut().zip(seats) {
            memory.observe_events(game, seat);
            self.shown.push(memory.frame(&Scene::of(game, seat), view, &t));
        }
        picture(&self.shown, view, game.time, &t, label_font(points))
    }

    /// A couch's split screen: each of `seats` on its own half, `views`
    /// in the same order (`ViewFrame::split_at`), each half's picture its
    /// own seat's alone - its arrows, counts and marks - as `picture`
    /// composes one seat's. The indicators of both are `shown`.
    pub fn pictures(&mut self, game: &Game, seats: &[u8], views: &[ViewFrame], points: f32) -> Vec<Picture> {
        self.seats.resize_with(seats.len(), Awareness::new);
        let t = in_points(&tuning(), points);
        self.shown.clear();
        for ((memory, &seat), view) in self.seats.iter_mut().zip(seats).zip(views) {
            memory.observe_events(game, seat);
            self.shown.push(memory.frame(&Scene::of(game, seat), view, &t));
        }
        let font = label_font(points);
        self.shown.iter().zip(views).map(|(ind, view)| picture(std::slice::from_ref(ind), view, game.time, &t, font)).collect()
    }

    /// The indicators the last `picture` composed, one per seat on the
    /// screen: what the minimap reads its enemies and flashing gates from
    /// (`minimap::Marks`), under the same concealment as the arrows.
    pub fn shown(&self) -> &[Indicators] {
        &self.shown
    }
}

/// `t` with the indicators' sizes in points - the inset, the arrow, the
/// cluster - turned into a view laid out `points` units to the point: the
/// bitmap's pixels, at however many of them make a point at the UI scale
/// the rest of the chrome is drawn at (`hud::UiFrame`).
pub fn in_points(t: &Tuning, points: f32) -> Tuning {
    let points = if points.is_finite() && points > 0.0 { points } else { 1.0 };
    Tuning {
        indicator_inset_pt: t.indicator_inset_pt * points,
        indicator_arrow_pt: t.indicator_arrow_pt * points,
        indicator_cluster_pt: t.indicator_cluster_pt * points,
        ..*t
    }
}

/// The size the labels are set in on a view `points` units to the point:
/// the default font at the smallest whole multiple of its own 10 px that
/// is `LABEL_MIN_PT` on the glass, so its letters stay whole blocks.
pub fn label_font(points: f32) -> i32 {
    let points = if points.is_finite() && points > 0.0 { points } else { 1.0 };
    let base = crate::hud::HUD_LABEL_SIZE;
    base * ((LABEL_MIN_PT * points / base as f32).ceil() as i32).max(1)
}

/// Where an arrow from `origin` (screen points) toward `target` (world
/// pixels) sits on `view`'s inset rectangle, slid clear of its keep-out
/// rectangles by `margin`.
fn place(view: &ViewFrame, origin: Vec2, target: Position, margin: f32) -> Option<EdgePoint> {
    let to = view.to_screen(target);
    let (hit, edge) = cast(view.inset, origin, to)?;
    let (at, edge) = slide(view.inset, hit, edge, &view.keep_out, margin);
    // On a split screen the divider is this view's edge too.
    let (at, edge) = match view.beyond {
        Some(beyond) => beyond.clip(origin, to, at, edge),
        None => (at, edge),
    };
    let dir = unit(to - at).or_else(|| unit(to - origin))?;
    Some(EdgePoint { at, edge, dir })
}

// ---- reading the round -------------------------------------------------------

impl Scene {
    /// The round `game` - a local round or a replica - as seat `seat`
    /// reads it this frame.
    pub fn of(game: &Game, seat: u8) -> Scene {
        let t = tuning();
        let field = game.map.field_size();
        let sight = game.enemy_sight();
        let seat_entity = game.seat(seat as usize);
        // A tank in a gate lane: the `RollIn` a local round gives it, or -
        // on a replica, which has none - a hull off the field.
        let gate_of = |entity: Entity, pos: Position| -> Option<Position> {
            match game.world.get::<&RollIn>(entity) {
                Ok(roll) => Some(roll.to),
                Err(_) => off_field(pos, field).then(|| onto_field(pos, field)),
            }
        };
        let mut me = None;
        let mut tanks: Vec<(TankView, Dir)> = Vec::new();
        for (entity, tank) in game.world.query::<(Entity, &Tank)>().iter() {
            let gate = gate_of(entity, tank.position);
            if Some(entity) == seat_entity {
                me = Some(SeatView { slot: tank.owner_slot(), pos: tank.position, wreck: tank.is_wreck(), gate });
                continue;
            }
            let view = TankView {
                slot: tank.owner_slot(),
                seat: game.player_index(entity),
                pos: tank.position,
                wreck: tank.is_wreck(),
                gate,
                in_grass: crate::grass::conceals(&game.grass_cells, tank.position),
                lane: false,
            };
            tanks.push((view, facing_of(tank.rotation)));
        }
        tanks.sort_by_key(|(tv, _)| tv.slot);
        // The lane: the AI's fire rule first, then its line of sight, the
        // world built for that only when an enemy is lined up at all.
        if let Some(me) = me.filter(SeatView::active) {
            let mut world = None;
            for (tv, facing) in tanks.iter_mut().filter(|(tv, _)| tv.is_enemy()) {
                if lined_up(tv.pos, *facing, me.pos, sight, &t) {
                    let world = world.get_or_insert_with(|| game.present_world());
                    tv.lane = world.line_of_sight(tv.pos, me.pos);
                }
            }
        }
        let frogs = [game.frog, game.enemy_frog]
            .into_iter()
            .flatten()
            .filter_map(|e| game.world.get::<&Frog>(e).ok().map(|f| FrogView { side: f.side, pos: f.position, alive: !f.is_dead() }))
            .collect();
        let shortened = crate::weather::sight_factor(game.weather, &t) < 1.0;
        let volcanoes = game
            .volcanoes()
            .iter()
            .filter_map(|v| {
                let phase = v.phase(game.time, &t);
                phase.is_warning().then(|| (v.centre(), phase.stage == crate::volcano::Stage::Erupt))
            })
            .collect();
        Scene {
            time: game.time,
            seat: me,
            tanks: tanks.into_iter().map(|(tv, _)| tv).collect(),
            frogs,
            sight: shortened.then_some(sight),
            volcanoes,
        }
    }
}

/// Whether `p` lies off the field of size `field`.
fn off_field(p: Position, field: (f32, f32)) -> bool {
    p.x < 0.0 || p.y < 0.0 || p.x > field.0 || p.y > field.1
}

/// `p` brought onto the field, half a cell in from its edge: the mouth of
/// the gate lane a tank off the field is driving in through.
fn onto_field(p: Position, field: (f32, f32)) -> Position {
    let half = OBSTACLE_GRID_SIZE * 0.5;
    Position::new(p.x.clamp(half, (field.0 - half).max(half)), p.y.clamp(half, (field.1 - half).max(half)))
}

/// Which way the hit on the seat whose tank is `me` (entity `entity`,
/// hull box `hull`), landing at `at`, came from - a unit vector from the
/// tank. A beam or a tesla bolt that stopped there points back at its
/// source; a shot standing there - the hit test leaves it in its impact
/// frames where it struck - back up the line it flew; a flame's contact,
/// which lands on the hull's centre, at the nearest nozzle of another's
/// jet; anything else at the face of the hull it landed on, since every
/// tank fires along a row or a column.
fn hit_from(game: &Game, entity: Entity, me: Position, hull: (Position, Position), at: Position) -> Option<Vec2> {
    for event in game.events() {
        if let Event::LaserBeam { x0, y0, x1, y1, .. } | Event::TeslaStrike { x0, y0, x1, y1, .. } = *event
            && Position::new(x1, y1).distance_to(at) <= HIT_MATCH_PX
        {
            return unit(Position::new(x0 - x1, y0 - y1));
        }
    }
    if let Some(v) = shot_velocity_at(game, at) {
        return unit(-v);
    }
    if at.distance_to(me) <= HIT_MATCH_PX {
        let seat = game.player_index(entity).map(Owner::Player);
        let nozzle = game
            .flames()
            .iter()
            .filter(|jet| Some(jet.owner) != seat)
            .map(|jet| jet.nozzle)
            .min_by(|a, b| a.distance_to(me).total_cmp(&b.distance_to(me)));
        return nozzle.and_then(|n| unit(n - me));
    }
    let (centre, half) = hull;
    let d = at - centre;
    let (nx, ny) = (d.x / half.x.max(1.0), d.y / half.y.max(1.0));
    if nx.abs() < 1e-3 && ny.abs() < 1e-3 {
        return None;
    }
    Some(if nx.abs() >= ny.abs() { Vec2::new(nx.signum(), 0.0) } else { Vec2::new(0.0, ny.signum()) })
}

/// The velocity of the shot standing nearest `at`, within
/// `HIT_MATCH_PX`.
fn shot_velocity_at(game: &Game, at: Position) -> Option<Vec2> {
    let mut best: Option<(f32, Vec2)> = None;
    let mut consider = |pos: Position, vel: Vec2| {
        let d = pos.distance_to(at);
        if d <= HIT_MATCH_PX && vel.length() > 0.01 && best.is_none_or(|(b, _)| d < b) {
            best = Some((d, vel));
        }
    };
    for s in game.world.query::<&Shell>().iter() {
        consider(s.position, s.velocity);
    }
    for b in game.world.query::<&Bullet>().iter() {
        consider(b.position, b.velocity);
    }
    for p in game.world.query::<&Plasma>().iter() {
        consider(p.position, p.velocity);
    }
    best.map(|(_, v)| v)
}

// ---- the picture ---------------------------------------------------------------

/// The colours the marks are drawn in (docs/PALETTE.md): palette steps for
/// the hostile red of the enemies, their rings and the hit arc (RED_BRIGHT,
/// RED_DEEP), the gates' amber (GOLD_BRIGHT) and the near-black (BLACK)
/// every mark is rimmed in, so a mark reads on grass, sand, snow and a
/// night field alike. The two identities are off the palette on purpose,
/// as their rings are, since an identity has to be loud against the
/// terrain: a teammate's arrow is its seat's ring colour
/// (`tank::team_color`) and the frog's the green of the HUD's FROG gauge,
/// which the grass's own greens would swallow. White is a flash. The
/// minimap's marks (`minimap::picture`) wear the same colours.
pub(crate) const HOSTILE: Color = Color::new(0xFF, 0x42, 0x1A, 255);
const HOSTILE_DEEP: Color = Color::new(0x9C, 0x35, 0x27, 255);
pub(crate) const FROG_GREEN: Color = Color::new(120, 220, 90, 255);
pub(crate) const GATE_AMBER: Color = Color::new(0xEE, 0xA3, 0x43, 255);
pub(crate) const RIM: Color = Color::new(0x25, 0x25, 0x25, 255);
pub(crate) const WHITE: Color = Color::new(0xFF, 0xFF, 0xFF, 255);

/// A block: the 2 px every mark is built from, on the screen as in the
/// world (`pyro::BLOCK`), so an arrow is as chunky as the sprites under it.
const B: i32 = 2;

/// A diagonal arrow's square side, in its cardinal length: about the same
/// area of blocks, so turning does not change how much an arrow weighs.
const CORNER_SIDE: f32 = 0.85;

/// A lined-up enemy's ring: a block wide round its arrow's middle, pulsing
/// between these radii in arrow lengths - motion the eye's edge catches
/// where it would miss a change of hue - and the white disc of its shot's
/// flash the larger of them.
const RING_SMALL: f32 = 0.8;
const RING_LARGE: f32 = 1.05;

/// The hollow arrow of a last-seen spot off the screen, in full arrows.
const HOLLOW_SCALE: f32 = 0.8;

/// The last-seen mark in the world: four corners of a square this many
/// blocks from its middle to its sides, each corner's arms this long.
const MARK_HALF_BLOCKS: i32 = 7;
const MARK_ARM_BLOCKS: i32 = 3;

/// A gate the screen shows: a frame round the cell it is in.
const GATE_HALF_BLOCKS: i32 = 8;

/// The hit arc's thickness, in blocks.
const ARC_BLOCKS: i32 = 2;

/// The fraction of its life the hit arc opens with in white, the flash a
/// hit is.
const ARC_WHITE: f32 = 0.15;

/// How far a mark fades before a label stops fading with it: a count or a
/// seat's number is read, not glanced at.
const LABEL_ALPHA_MIN: f32 = 0.75;

/// The smallest a label's letters are on the glass, in points
/// (`label_font`): the lab's size for the numbers at the edge.
const LABEL_MIN_PT: f32 = 8.0;

/// A rectangle of whole blocks in one colour, in the space of the list it
/// is in (`Picture`): world pixels or the screen's points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fill {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub color: Color,
}

/// A number or a seat's name by a mark, in the screen's points, on a dark
/// plate so it reads on any ground under any sky.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub text: String,
    /// The text's top-left corner.
    pub x: i32,
    pub y: i32,
    /// The default font's size (`label_font`).
    pub size: i32,
    pub color: Color,
    /// The plate behind it, a block wider than the text all round.
    pub plate: Fill,
}

/// One screen's indicators as what to paint (`picture`): blocks in the
/// world, blocks on the screen and labels, each list in the order it is
/// painted. Plain values, painted by `render::indicators`; drawn over the
/// field after the scene, like the HUD, so no sky darkens them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Picture {
    /// In world pixels, drawn through the camera: the last-seen marks and
    /// the flashing gates the screen shows, the hit arcs round the seats'
    /// tanks.
    pub world: Vec<Fill>,
    /// In the screen's points: the arrows with the rings of the lined-up
    /// enemies round them, and the hollow arrows of last-seen spots off the
    /// screen.
    pub screen: Vec<Fill>,
    /// In the screen's points, over everything else: the merged arrows'
    /// counts, the teammates' seats, the frogs' distances and the counts of
    /// the enemies folded into each edge.
    pub labels: Vec<Label>,
}

impl Picture {
    /// Whether there is nothing to paint.
    pub fn is_empty(&self) -> bool {
        self.world.is_empty() && self.screen.is_empty() && self.labels.is_empty()
    }
}

/// What one screen shows of what its seats cannot see, as blocks and labels
/// to paint. `seats` holds one `Indicators` per seat the screen is for: one,
/// or two on a couch's screen, where the first's arrows, counts, last-seen
/// marks and gates are the screen's and every other seat adds what is its
/// own - its hit arc, the teammates no arrow points at yet and its lane
/// warnings, which the arrow already pointing at that enemy takes on
/// (`shared_arrows`). `view` is the screen the indicators were gathered on,
/// whose safe area the labels keep inside, `t` the table in its units
/// (`in_points`) and `font` the size the labels are set in (`label_font`);
/// `time` is the round clock the pulses and blinks run on, so they stand
/// still with the round.
///
/// - *Arrows* are notched arrowheads of whole blocks (`arrow_cells`) rimmed
///   in near-black, their tips on their edge points so they lie inside the
///   inset rectangle, sized and faded as the model says: an enemy's in the
///   hostile red, a teammate's in its ring colour with its seat (`P2`), a
///   frog's in green with its distance in cells - the enemy frog Hunt is
///   after rimmed in red -, a gate's in amber, blinking while it flashes,
///   as a teammate's arrow does while it drives back in through a gate.
///   The most urgent is painted last, on top.
/// - *A lane warning* puts a rimmed ring round its arrow that swells and
///   shrinks `indicator_pulse_hz` times a second, dark red while the aim
///   settles and bright red once it has, the arrow swelling with it by up
///   to `indicator_pulse_swell` as the aim settles; the shot down the lane
///   turns the arrow white on a white disc for the first half of its
///   flash, then the disc fades.
/// - *Counts*: a merged arrow carries its count, and each edge's folded
///   enemies a `+N` where the first of them would have sat, its plate
///   pulsing red when one of them is lined up.
/// - *In the world*: corner marks where an enemy was last seen, fading, a
///   hollow arrow at the edge for a spot off the screen; a frame blinking
///   round a gate the screen shows; and on each seat's tank an arc of
///   `indicator_hit_arc_degrees` toward the last hit, white as it lands and
///   then red, fading.
pub fn picture(seats: &[Indicators], view: &ViewFrame, time: f32, t: &Tuning, font: i32) -> Picture {
    let mut out = Picture::default();
    let Some((first, others)) = seats.split_first() else { return out };
    let throb = pulse(time, t.indicator_pulse_hz);
    let blink = blink_on(time, t.indicator_gate_blink_hz);
    let grow = t.indicator_inset_pt.max(0.0);
    let bounds = Rectangle::new(view.inset.x - grow, view.inset.y - grow, view.inset.width + 2.0 * grow, view.inset.height + 2.0 * grow);

    // In the world: where enemies were last seen, the gates the screen
    // shows, the hit arcs.
    for spot in &first.last_seen {
        match spot.edge {
            None => rimmed(&mut out.world, spot.at, &bracket_cells(MARK_HALF_BLOCKS, MARK_ARM_BLOCKS), HOSTILE, RIM, spot.fade),
            Some(edge) => {
                let len = t.indicator_arrow_pt * HOLLOW_SCALE;
                hollow(&mut out.screen, behind(edge.at, edge.dir, len * 0.5), &arrow_cells(edge.dir, len), HOSTILE, spot.fade);
            }
        }
    }
    if blink {
        for gate in first.gates.iter().filter(|g| g.on_screen && g.flash > 0.0) {
            rimmed(&mut out.world, gate.at, &frame_cells(GATE_HALF_BLOCKS), GATE_AMBER, RIM, 0.4 + 0.6 * gate.flash);
        }
    }
    for arc in seats.iter().filter_map(|s| s.hit_arc) {
        let half = (t.indicator_hit_arc_degrees * 0.5).to_radians();
        let body = if arc.fade > 1.0 - ARC_WHITE { WHITE } else { HOSTILE };
        rimmed(&mut out.world, arc.at, &arc_cells(arc.dir, t.indicator_hit_arc_px, ARC_BLOCKS, half), body, RIM, arc.fade);
    }

    // On the screen: the arrows, the least urgent first so the most urgent
    // is painted on top, then their labels.
    let arrows = shared_arrows(first, others);
    for arrow in arrows.iter().rev() {
        let mut len = t.indicator_arrow_pt * arrow.scale;
        let mut alpha = arrow.alpha;
        let (body, rim) = match arrow.kind {
            ArrowKind::Enemy { warning, .. } => {
                if let Some(w) = warning {
                    len *= 1.0 + t.indicator_pulse_swell * w.settle * throb;
                    let middle = behind(arrow.place.at, arrow.place.dir, len * 0.5);
                    let radius = len * (RING_SMALL + (RING_LARGE - RING_SMALL) * throb);
                    let ring = if w.settle >= 0.5 { HOSTILE } else { HOSTILE_DEEP };
                    rimmed(&mut out.screen, middle, &ring_cells(radius), ring, RIM, alpha);
                    if w.flash > 0.0 {
                        put(&mut out.screen, block_at(middle), &disc_cells(len * RING_LARGE), crate::pyro::alpha(WHITE, w.flash));
                    }
                }
                let flashed = warning.is_some_and(|w| w.flash >= 0.5);
                (if flashed { WHITE } else { HOSTILE }, RIM)
            }
            ArrowKind::Teammate { seat, entering } => {
                if entering && !blink {
                    continue;
                }
                (crate::tank::team_color(seat), RIM)
            }
            ArrowKind::Frog { side: Side::Player } => (FROG_GREEN, RIM),
            ArrowKind::Frog { side: Side::Enemy } => (FROG_GREEN, HOSTILE),
            ArrowKind::Gate { flash } => {
                if !blink {
                    continue;
                }
                alpha *= 0.4 + 0.6 * flash;
                (GATE_AMBER, RIM)
            }
            // Pulsing on the lane warning's beat, gold while it rumbles and
            // red-hot once it throws.
            ArrowKind::Volcano { erupting } => {
                len *= 1.0 + t.indicator_pulse_swell * throb;
                (if erupting { HOSTILE } else { GATE_AMBER }, if throb >= 0.5 { crate::pyro::FIRE[6] } else { RIM })
            }
        };
        let middle = behind(arrow.place.at, arrow.place.dir, len * 0.5);
        rimmed(&mut out.screen, middle, &arrow_cells(arrow.place.dir, len), body, rim, alpha);
    }
    for arrow in &arrows {
        let reach = t.indicator_arrow_pt * arrow.scale + B as f32;
        let (text, color) = match arrow.kind {
            ArrowKind::Enemy { count, .. } if count > 1 => (count.to_string(), WHITE),
            ArrowKind::Teammate { seat, .. } => {
                let name = crate::text::text().fmt(crate::text::keys::SEAT_LABEL, &[("n", (u32::from(seat) + 1).into())]);
                (name, crate::tank::team_color(seat))
            }
            ArrowKind::Frog { .. } => ((arrow.cells.round() as i64).to_string(), FROG_GREEN),
            _ => continue,
        };
        let alpha = arrow.alpha.max(LABEL_ALPHA_MIN);
        let size = label_size(&text, font);
        let corner = label_corner(arrow.place.at, arrow.place.dir, reach, size, bounds);
        out.labels.push(label(text, font, corner, color, RIM, alpha));
    }
    for count in first.folded.iter().filter(|c| c.count > 0) {
        let text = format!("+{}", count.count);
        let size = label_size(&text, font);
        let middle = (count.at.x - size.0 * 0.5, count.at.y - size.1 * 0.5);
        let corner = inside(middle, size, bounds);
        let plate = if count.warning && throb >= 0.5 { HOSTILE_DEEP } else { RIM };
        out.labels.push(label(text, font, corner, WHITE, plate, 1.0));
    }
    out
}

/// The arrows of a screen several seats share: the first seat's, then from
/// every other seat each teammate no arrow points at yet, and each of its
/// lane threats - taken on by the enemy arrow already pointing at that
/// enemy, which keeps the more urgent warning of the two, or added where no
/// arrow does.
fn shared_arrows(first: &Indicators, others: &[Indicators]) -> Vec<Arrow> {
    let mut arrows = first.arrows.clone();
    for other in others {
        for arrow in &other.arrows {
            match arrow.kind {
                ArrowKind::Teammate { seat, .. } => {
                    let shown = arrows.iter().any(|a| matches!(a.kind, ArrowKind::Teammate { seat: s, .. } if s == seat));
                    if !shown {
                        arrows.push(*arrow);
                    }
                }
                ArrowKind::Enemy { warning: Some(w), .. } => {
                    let same = arrows.iter_mut().find(|a| matches!(a.kind, ArrowKind::Enemy { .. }) && a.target == arrow.target);
                    match same {
                        Some(Arrow { kind: ArrowKind::Enemy { warning, .. }, .. }) => {
                            let urgency = |w: &LaneWarning| (w.flash, w.settle);
                            if warning.is_none_or(|old| urgency(&w) > urgency(&old)) {
                                *warning = Some(w);
                            }
                        }
                        _ => arrows.push(*arrow),
                    }
                }
                _ => {}
            }
        }
    }
    arrows
}

/// The blocks of an arrow `len` points long pointing along `dir`, turned to
/// the nearest of the eight compass headings: along an axis a notched
/// triangle, its tip forward (`triangle_cells`), and on a diagonal a
/// notched corner, its tip the corner (`corner_cells`) - the shapes a few
/// blocks can draw that still point, where a dart turned any finer reads as
/// a smudge, and that do not crawl while the direction drifts. As (column,
/// row) offsets from the block the arrow is centred on, row by row. Empty
/// for a direction that is none.
pub fn arrow_cells(dir: Vec2, len: f32) -> Vec<(i32, i32)> {
    let Some(dir) = unit(dir) else { return Vec::new() };
    let n = ((len / B as f32).round() as i32).max(2);
    let octant = ((crate::trig::atan2(dir.y, dir.x) / std::f32::consts::FRAC_PI_4).round() as i32).rem_euclid(8);
    let shape = if octant % 2 == 0 { triangle_cells(n) } else { corner_cells(n) };
    let mut cells: Vec<(i32, i32)> = shape.into_iter().map(|c| quarter_turns(c, octant / 2)).collect();
    cells.sort_by_key(|&(i, j)| (j, i));
    cells
}

/// An arrow pointing right, `n` blocks from its back to its tip: each
/// column a block shorter at either end than the one two behind it, and a
/// notch a quarter of its length deep cut into the middle of its back.
fn triangle_cells(n: i32) -> Vec<(i32, i32)> {
    let depth = if n >= 5 { (n / 4).max(1) } else { 0 };
    let back = n / 2;
    let mut cells = Vec::new();
    for c in 0..n {
        let h = (n - 1 - c) / 2;
        for j in -h..=h {
            if !(c < depth && j.abs() < depth - c) {
                cells.push((c - back, j));
            }
        }
    }
    cells
}

/// An arrow pointing down and right, `n` blocks long as a triangle would
/// be: the corner of a square `CORNER_SIDE` of that a side - its blocks on
/// and past the diagonal, the tip the square's corner - with a notch a
/// third of its side deep cut into the middle of the diagonal, its back.
fn corner_cells(n: i32) -> Vec<(i32, i32)> {
    let m = ((n as f32 * CORNER_SIDE).round() as i32).max(2);
    let depth = if m >= 4 { ((m - 1) / 3).max(1) } else { 0 };
    let back = m / 2;
    let mut cells = Vec::new();
    for j in 0..m {
        for i in 0..m {
            let d = i + j - (m - 1);
            if d >= 0 && !(d < depth && (i - j).abs() <= depth - d) {
                cells.push((i - back, j - back));
            }
        }
    }
    cells
}

/// `cell` turned `quarters` quarter turns clockwise on the screen (y down)
/// round the block it is counted from.
fn quarter_turns(cell: (i32, i32), quarters: i32) -> (i32, i32) {
    let (mut i, mut j) = cell;
    for _ in 0..quarters.rem_euclid(4) {
        (i, j) = (-j, i);
    }
    (i, j)
}

/// The blocks round `cells` - touching one along a side or at a corner -
/// that are not in it: an outline a block wide, row by row.
pub fn rim_cells(cells: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let inside: BTreeSet<(i32, i32)> = cells.iter().copied().collect();
    let mut rim = BTreeSet::new();
    for &(i, j) in cells {
        for dj in -1..=1 {
            for di in -1..=1 {
                if !inside.contains(&(i + di, j + dj)) {
                    rim.insert((j + dj, i + di));
                }
            }
        }
    }
    rim.into_iter().map(|(j, i)| (i, j)).collect()
}

/// The blocks of a disc of `radius` round the middle of the block it is
/// centred on.
fn disc_cells(radius: f32) -> Vec<(i32, i32)> {
    let n = (radius / B as f32).ceil() as i32;
    let mut cells = Vec::new();
    for j in -n..=n {
        for i in -n..=n {
            let (x, y) = ((i * B) as f32, (j * B) as f32);
            if x * x + y * y <= radius * radius {
                cells.push((i, j));
            }
        }
    }
    cells
}

/// The blocks of a ring a block wide just inside `radius`, round the middle
/// of the block it is centred on.
fn ring_cells(radius: f32) -> Vec<(i32, i32)> {
    let inner: BTreeSet<(i32, i32)> = disc_cells(radius - B as f32).into_iter().collect();
    disc_cells(radius).into_iter().filter(|c| !inner.contains(c)).collect()
}

/// The blocks of a band `thickness` blocks deep from `radius` out, round
/// the middle of the block it is centred on, within `half_angle` radians
/// either side of `dir`: the hit arc. Empty for a direction that is none.
pub fn arc_cells(dir: Vec2, radius: f32, thickness: i32, half_angle: f32) -> Vec<(i32, i32)> {
    let Some(dir) = unit(dir) else { return Vec::new() };
    let outer = radius + (thickness * B) as f32;
    let reach = crate::trig::cos(half_angle.clamp(0.0, std::f32::consts::PI));
    let n = (outer / B as f32).ceil() as i32 + 1;
    let mut cells = Vec::new();
    for j in -n..=n {
        for i in -n..=n {
            let p = Vec2::new((i * B) as f32, (j * B) as f32);
            let r = p.length();
            if r >= radius && r < outer && p.dot(dir) >= r * reach {
                cells.push((i, j));
            }
        }
    }
    cells
}

/// The four corners of a square `half` blocks from its middle block to its
/// sides, each an L whose arms run `arm` blocks along the sides: the mark
/// where an enemy was last seen.
pub fn bracket_cells(half: i32, arm: i32) -> Vec<(i32, i32)> {
    let mut cells = BTreeSet::new();
    for (sx, sy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
        for k in 0..arm.max(1) {
            cells.insert((sy * half, sx * (half - k)));
            cells.insert((sy * (half - k), sx * half));
        }
    }
    cells.into_iter().map(|(j, i)| (i, j)).collect()
}

/// The blocks of a square frame `half` blocks from its middle block to its
/// sides.
fn frame_cells(half: i32) -> Vec<(i32, i32)> {
    let mut cells = Vec::new();
    for j in -half..=half {
        for i in -half..=half {
            if i.abs() == half || j.abs() == half {
                cells.push((i, j));
            }
        }
    }
    cells
}

/// The point `by` back from `at` against `dir`: the middle of an arrow
/// `2 * by` long whose tip is at `at`, as every arrow's is on the inset
/// rectangle, so the arrow lies inside it.
fn behind(at: Vec2, dir: Vec2, by: f32) -> Vec2 {
    at - unit(dir).unwrap_or(Vec2::zero()) * by
}

/// The block (column, row) the point `p` lies in: every shape is centred
/// on one, so its blocks lie on the 2 px grid.
fn block_at(p: Vec2) -> (i32, i32) {
    ((p.x / B as f32).floor() as i32, (p.y / B as f32).floor() as i32)
}

/// Paint `cells` round the block `centre` in `color`, a run of blocks along
/// a row as one fill.
fn put(out: &mut Vec<Fill>, centre: (i32, i32), cells: &[(i32, i32)], color: Color) {
    if color.a == 0 {
        return;
    }
    let mut rows: Vec<(i32, i32)> = cells.iter().map(|&(i, j)| (j, i)).collect();
    rows.sort_unstable();
    rows.dedup();
    let mut k = 0;
    while k < rows.len() {
        let (j, first) = rows[k];
        let mut last = first;
        while k + 1 < rows.len() && rows[k + 1] == (j, last + 1) {
            k += 1;
            last += 1;
        }
        out.push(Fill { x: (centre.0 + first) * B, y: (centre.1 + j) * B, w: (last - first + 1) * B, h: B, color });
        k += 1;
    }
}

/// `cells` round the block `at` lies in, in `body` with a rim of `rim`,
/// both at `alpha` (stepped to eighths, `pyro::alpha`).
fn rimmed(out: &mut Vec<Fill>, at: Vec2, cells: &[(i32, i32)], body: Color, rim: Color, alpha: f32) {
    let centre = block_at(at);
    put(out, centre, &rim_cells(cells), crate::pyro::alpha(rim, alpha));
    put(out, centre, cells, crate::pyro::alpha(body, alpha));
}

/// `cells` round the block `at` lies in, hollow: an outline round them in
/// `color` and a near-black rim round that, the blocks themselves left
/// open on the ground.
fn hollow(out: &mut Vec<Fill>, at: Vec2, cells: &[(i32, i32)], color: Color, alpha: f32) {
    let centre = block_at(at);
    let line = rim_cells(cells);
    let solid: Vec<(i32, i32)> = cells.iter().chain(&line).copied().collect();
    put(out, centre, &rim_cells(&solid), crate::pyro::alpha(RIM, alpha));
    put(out, centre, &line, crate::pyro::alpha(color, alpha));
}

/// The size of the plate a label's `text` set in `font` sits on: the text
/// a block in from its edges all round.
fn label_size(text: &str, font: i32) -> (f32, f32) {
    ((crate::text::width(text, font) + 2 * B) as f32, (font + 2 * B) as f32)
}

/// The top-left corner of a plate `size` points across by an arrow whose
/// tip is at `at`, pointing along `dir` and reaching `reach` back from its
/// tip: behind the arrow, toward the middle of the screen and a block clear
/// of it, kept inside `bounds` and on the block grid.
pub fn label_corner(at: Vec2, dir: Vec2, reach: f32, size: (f32, f32), bounds: Rectangle) -> (i32, i32) {
    let dir = unit(dir).unwrap_or(Vec2::zero());
    let extent = dir.x.abs() * size.0 * 0.5 + dir.y.abs() * size.1 * 0.5;
    let middle = at - dir * (reach + B as f32 + extent);
    inside((middle.x - size.0 * 0.5, middle.y - size.1 * 0.5), size, bounds)
}

/// The corner `corner` of a box `size` across moved inside `bounds` - to
/// its top-left where the box is the larger - and onto the block grid.
fn inside(corner: (f32, f32), size: (f32, f32), bounds: Rectangle) -> (i32, i32) {
    let x = corner.0.min(bounds.x + bounds.width - size.0).max(bounds.x);
    let y = corner.1.min(bounds.y + bounds.height - size.1).max(bounds.y);
    let snap = |v: f32| (v / B as f32).round() as i32 * B;
    (snap(x), snap(y))
}

/// A label with its plate's top-left at `corner`: `text` set in `font`, in
/// `color` over a `plate`-coloured plate, at `alpha`.
fn label(text: String, font: i32, corner: (i32, i32), color: Color, plate: Color, alpha: f32) -> Label {
    let (w, h) = label_size(&text, font);
    Label {
        x: corner.0 + B,
        y: corner.1 + B,
        size: font,
        color: crate::pyro::alpha(color, alpha),
        plate: Fill { x: corner.0, y: corner.1, w: w as i32, h: h as i32, color: crate::pyro::alpha(plate, alpha * 0.8) },
        text,
    }
}

/// A pulse at round time `time`: 0 to 1 and back `hz` times a second.
fn pulse(time: f32, hz: f32) -> f32 {
    let phase = (time.max(0.0) * hz).rem_euclid(1.0);
    0.5 + 0.5 * crate::trig::sin(std::f32::consts::TAU * phase)
}

/// Whether a blink `hz` times a second is in its lit half at round time
/// `time`.
fn blink_on(time: f32, hz: f32) -> bool {
    (time.max(0.0) * hz).rem_euclid(1.0) < 0.5
}

#[cfg(test)]
mod indicator_tests {
    use super::*;
    use crate::PHYSICS_FIXED_DT;
    use crate::level::Mission;
    use crate::map::{MapFile, Weather, cell_to_world};
    use crate::simulation::debug::TankPatch;
    use crate::simulation::{Input, PlayerCount};

    // ---- plain inputs ----

    /// The seat's tank, in the middle of `screen`.
    const SEAT: Position = Position::new(200.0, 150.0);

    /// A 400 x 300 point screen showing the world from its origin at one
    /// point a pixel, its arrows on the rectangle 10 pt in: (10, 10) to
    /// (390, 290).
    fn screen() -> ViewFrame {
        let r = Rectangle::new(0.0, 0.0, 400.0, 300.0);
        ViewFrame::new(r, 1.0, Vec2::zero(), r, &Tuning::DEFAULT)
    }

    fn enemy(slot: usize, x: f32, y: f32) -> TankView {
        TankView { slot, seat: None, pos: Position::new(x, y), wreck: false, gate: None, in_grass: false, lane: false }
    }

    fn scene(time: f32, tanks: Vec<TankView>) -> Scene {
        Scene { time, seat: Some(SeatView { slot: 0, pos: SEAT, wreck: false, gate: None }), tanks, ..Scene::default() }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    /// The warning on the first enemy arrow; `None` with no enemy arrow.
    fn warning(ind: &Indicators) -> Option<Option<LaneWarning>> {
        ind.arrows.iter().find_map(|a| match a.kind {
            ArrowKind::Enemy { warning, .. } => Some(warning),
            _ => None,
        })
    }

    fn enemy_arrows(ind: &Indicators) -> usize {
        ind.arrows.iter().filter(|a| matches!(a.kind, ArrowKind::Enemy { .. })).count()
    }

    fn kinds(ind: &Indicators) -> Vec<ArrowKind> {
        ind.arrows.iter().map(|a| a.kind).collect()
    }

    #[test]
    fn a_waking_volcano_off_the_screen_has_an_arrow_whatever_the_cap() {
        let mut t = Tuning::DEFAULT;
        t.indicator_max_arrows = 1;
        let mut s = scene(1.0, vec![enemy(5, 900.0, 150.0), enemy(6, 950.0, 400.0)]);
        s.volcanoes = vec![(Position::new(200.0, -500.0), false)];
        let ind = Awareness::new().frame(&s, &screen(), &t);
        assert!(kinds(&ind).contains(&ArrowKind::Volcano { erupting: false }), "{:?}", kinds(&ind));
        // On the screen, none: the volcano is in sight.
        s.volcanoes = vec![(Position::new(200.0, 100.0), true)];
        let ind = Awareness::new().frame(&s, &screen(), &t);
        assert!(!kinds(&ind).iter().any(|k| matches!(k, ArrowKind::Volcano { .. })));
    }

    /// The arrow stops where the line from the tank leaves the inset
    /// rectangle, on the edge it leaves through.
    #[test]
    fn an_arrow_sits_where_the_line_to_its_target_leaves_the_inset() {
        let inset = screen().inset;
        let from = Vec2::new(200.0, 150.0);
        assert_eq!(cast(inset, from, Vec2::new(1000.0, 150.0)), Some((Vec2::new(390.0, 150.0), Edge::Right)));
        assert_eq!(cast(inset, from, Vec2::new(200.0, -500.0)), Some((Vec2::new(200.0, 10.0), Edge::Top)));
        let (p, edge) = cast(inset, from, Vec2::new(-200.0, 450.0)).expect("a line");
        assert_eq!(edge, Edge::Bottom);
        assert!(close(p.x, 200.0 - 400.0 * 140.0 / 300.0) && close(p.y, 290.0), "{p:?}");
        assert_eq!(cast(inset, from, from), None, "no line to a point on the tank");
        // A tank in the inset's margin casts from the inset's edge.
        assert_eq!(cast(inset, Vec2::new(2.0, 150.0), Vec2::new(1000.0, 150.0)), Some((Vec2::new(390.0, 150.0), Edge::Right)));
    }

    /// An arrow that lands in a keep-out slides along the edge to the
    /// nearest point half an arrow clear of it, round a corner when the
    /// keep-out covers one, and stays put when nothing is clear.
    #[test]
    fn an_arrow_in_a_keep_out_slides_to_the_nearest_clear_point() {
        let inset = screen().inset;
        let cluster = Rectangle::new(300.0, 0.0, 100.0, 60.0);
        assert_eq!(slide(inset, Vec2::new(350.0, 10.0), Edge::Top, &[cluster], 8.0), (Vec2::new(292.0, 10.0), Edge::Top));
        assert_eq!(slide(inset, Vec2::new(390.0, 40.0), Edge::Right, &[cluster], 8.0), (Vec2::new(390.0, 68.0), Edge::Right));
        assert_eq!(slide(inset, Vec2::new(200.0, 10.0), Edge::Top, &[cluster], 8.0), (Vec2::new(200.0, 10.0), Edge::Top));
        let corner = Rectangle::new(0.0, 0.0, 80.0, 80.0);
        assert_eq!(slide(inset, Vec2::new(20.0, 10.0), Edge::Top, &[corner], 8.0), (Vec2::new(88.0, 10.0), Edge::Top));
        assert_eq!(slide(inset, Vec2::new(10.0, 30.0), Edge::Left, &[corner], 8.0), (Vec2::new(10.0, 88.0), Edge::Left));
        let everywhere = Rectangle::new(-50.0, -50.0, 500.0, 400.0);
        assert_eq!(slide(inset, Vec2::new(390.0, 150.0), Edge::Right, &[everywhere], 8.0), (Vec2::new(390.0, 150.0), Edge::Right));
        // Through a whole frame: a thumb's rest over the bottom edge.
        let mut view = screen();
        view.keep_out.push(Rectangle::new(150.0, 260.0, 100.0, 40.0));
        let ind = Awareness::new().frame(&scene(0.0, vec![enemy(5, 200.0, 900.0)]), &view, &Tuning::DEFAULT);
        let a = ind.arrows[0].place;
        assert_eq!((a.edge, a.at.y), (Edge::Bottom, 290.0));
        assert!(close(a.at.x, 142.0) || close(a.at.x, 258.0), "half an arrow clear of the pad: {a:?}");
        assert!(a.dir.y > 0.9, "still pointing down at the enemy: {a:?}");
    }

    /// One half of a couch's split screen treats the other half as off
    /// screen: what stands past the divider gets an arrow on it, the inset
    /// inside, where the line from the tank crosses it; what stands on this
    /// side shows as ever, and what lies past the screen's own edge on this
    /// side keeps its arrow there.
    #[test]
    fn a_split_screens_half_puts_arrows_on_the_divider_for_what_is_past_it() {
        let t = Tuning::DEFAULT;
        // The divider straight down the screen at x = 300, the other half
        // on its right.
        let view = screen().split_at(Vec2::new(300.0, 150.0), Vec2::new(1.0, 0.0), &t);
        assert!(view.shows(Position::new(100.0, 150.0)) && view.shows(Position::new(299.0, 20.0)));
        assert!(!view.shows(Position::new(350.0, 150.0)), "past the divider is the other half's");
        let ind = Awareness::new().frame(&scene(0.0, vec![enemy(5, 350.0, 150.0), enemy(6, 100.0, 150.0)]), &view, &t);
        assert_eq!(ind.arrows.len(), 1, "the enemy on this side shows; the one past the divider gets the arrow");
        let a = ind.arrows[0].place;
        assert_eq!((a.edge, a.at), (Edge::Right, Vec2::new(290.0, 150.0)));
        assert!(a.dir.x > 0.99, "{a:?}");
        // A teammate over there: its arrow too, never merged.
        let mate = TankView { seat: Some(1), ..enemy(1, 360.0, 160.0) };
        let ind = Awareness::new().frame(&scene(0.0, vec![mate]), &view, &t);
        assert_eq!(kinds(&ind), vec![ArrowKind::Teammate { seat: 1, entering: false }]);
        assert!(ind.arrows[0].place.at.x <= 290.0 + 1e-3);
        // Far off past the top edge, on this side of the line: the screen's
        // own edge, as on a whole screen.
        let ind = Awareness::new().frame(&scene(0.0, vec![enemy(5, 230.0, -500.0)]), &view, &t);
        assert_eq!(ind.arrows[0].place.edge, Edge::Top);
        // A slanted divider: the arrow sits where the line from the tank
        // crosses it, the inset inside, on the edge it faces.
        let n = Vec2::new(0.6, 0.8);
        let view = screen().split_at(Vec2::new(250.0, 200.0), n, &t);
        let ind = Awareness::new().frame(&scene(0.0, vec![enemy(5, 390.0, 290.0)]), &view, &t);
        let a = ind.arrows[0].place;
        let past = (a.at.x - 250.0) * n.x + (a.at.y - 200.0) * n.y;
        assert!(close(past, -t.indicator_inset_pt), "{a:?}: {past}");
        assert!(((a.at.x - SEAT.x) * 140.0 - (a.at.y - SEAT.y) * 190.0).abs() < 1e-2, "on the line to the enemy: {a:?}");
        assert_eq!(a.edge, Edge::Bottom);
    }

    /// Size and opacity are full up to a screen away and least from four
    /// on; a screen is the view's extent along the arrow.
    #[test]
    fn distance_reads_as_size_and_opacity() {
        let t = Tuning::DEFAULT;
        assert_eq!(distance_look(0.5, &t), (1.0, 1.0));
        assert_eq!(distance_look(1.0, &t), (1.0, 1.0));
        let (s, a) = distance_look(2.5, &t);
        assert!(close(s, 0.8) && close(a, 0.775), "{s} {a}");
        let (s, a) = distance_look(4.0, &t);
        assert!(close(s, 0.6) && close(a, 0.55), "{s} {a}");
        assert_eq!(distance_look(9.0, &t), distance_look(4.0, &t), "no smaller past four screens");
        let world = Rectangle::new(0.0, 0.0, 400.0, 300.0);
        assert!(close(screens(world, SEAT, Position::new(600.0, 150.0)), 1.0), "a width sideways");
        assert!(close(screens(world, SEAT, Position::new(200.0, 450.0)), 1.0), "a height up or down");
        assert!(close(screens(world, SEAT, Position::new(600.0, 450.0)), 1.0), "the diagonal's chord");
        assert!(close(screens(world, SEAT, Position::new(1000.0, 150.0)), 2.0));
        let ind = Awareness::new().frame(&scene(0.0, vec![enemy(5, 1800.0, 150.0)]), &screen(), &t);
        let arrow = ind.arrows[0];
        assert!(close(arrow.scale, 0.6) && close(arrow.alpha, 0.55), "{arrow:?}");
        assert!(close(arrow.cells, 1600.0 / OBSTACLE_GRID_SIZE), "{arrow:?}");
    }

    /// Enemy arrows that land within `indicator_cluster_pt` of each other
    /// are one arrow with a count, placed for the nearest of them.
    #[test]
    fn enemy_arrows_that_land_together_merge_with_a_count() {
        let tanks = vec![enemy(5, 1000.0, 160.0), enemy(6, 1000.0, 150.0), enemy(7, 1000.0, -400.0)];
        let ind = Awareness::new().frame(&scene(0.0, tanks), &screen(), &Tuning::DEFAULT);
        assert_eq!(kinds(&ind), vec![ArrowKind::Enemy { count: 2, warning: None }, ArrowKind::Enemy { count: 1, warning: None }]);
        assert_eq!(ind.arrows[0].target, Position::new(1000.0, 150.0), "the nearest leads");
        assert_eq!(ind.arrows[0].place.at, Vec2::new(390.0, 150.0));
    }

    /// A teammate's and a frog's arrows never merge with an enemy's, and
    /// the cap never leaves them out: what is left of it goes to the
    /// enemies, and the rest fold into their edges' counts.
    #[test]
    fn teammates_and_frogs_never_merge_and_always_show() {
        let mut tanks = vec![enemy(5, 1000.0, 150.0), enemy(6, 200.0, -600.0), enemy(7, -600.0, 150.0)];
        tanks.push(TankView { seat: Some(1), ..enemy(1, 1000.0, 152.0) });
        let mut s = scene(0.0, tanks);
        s.frogs.push(FrogView { side: Side::Player, pos: Position::new(200.0, 900.0), alive: true });
        s.frogs.push(FrogView { side: Side::Enemy, pos: Position::new(900.0, 900.0), alive: false });
        let ind = Awareness::new().frame(&s, &screen(), &Tuning::DEFAULT);
        assert_eq!(
            kinds(&ind),
            vec![
                ArrowKind::Teammate { seat: 1, entering: false },
                ArrowKind::Frog { side: Side::Player },
                ArrowKind::Enemy { count: 1, warning: None },
                ArrowKind::Enemy { count: 1, warning: None },
                ArrowKind::Enemy { count: 1, warning: None },
            ],
            "the teammate beside an enemy stays its own arrow; a dead frog has none"
        );
        let beside = ind.arrows.iter().find(|a| a.target == Position::new(1000.0, 150.0)).expect("the enemy beside it");
        assert!(close(beside.place.at.distance_to(ind.arrows[0].place.at), 0.475), "half a point apart, still two arrows");
        let mut t = Tuning::DEFAULT;
        t.indicator_max_arrows = 2;
        let ind = Awareness::new().frame(&s, &screen(), &t);
        assert_eq!(kinds(&ind), vec![ArrowKind::Teammate { seat: 1, entering: false }, ArrowKind::Frog { side: Side::Player }]);
        let folded: Vec<u32> = ind.folded.iter().map(|e| e.count).collect();
        assert_eq!(folded, vec![1, 1, 0, 1], "one each on the top, right and left edges");
        t.indicator_max_arrows = 1;
        assert_eq!(Awareness::new().frame(&s, &screen(), &t).arrows.len(), 2, "both kept past a cap of one");
    }

    /// The lane threats take the cap first; teammates and frogs still show
    /// past it, and a threat that does not fit is counted with its warning.
    #[test]
    fn lane_threats_lead_the_cap_and_teammates_still_show() {
        let threat = |slot: usize, y: f32| TankView { lane: true, ..enemy(slot, 450.0, y) };
        let mut tanks = vec![threat(5, 150.0), threat(6, 30.0), threat(7, 270.0), enemy(8, -300.0, 150.0)];
        tanks.push(TankView { seat: Some(1), ..enemy(1, 200.0, -600.0) });
        let mut s = scene(0.0, tanks);
        s.frogs.push(FrogView { side: Side::Enemy, pos: Position::new(200.0, 900.0), alive: true });
        let mut t = Tuning::DEFAULT;
        t.indicator_max_arrows = 2;
        let ind = Awareness::new().frame(&s, &screen(), &t);
        let settling = Some(LaneWarning { settle: 0.0, flash: 0.0 });
        assert_eq!(
            kinds(&ind),
            vec![
                ArrowKind::Enemy { count: 1, warning: settling },
                ArrowKind::Enemy { count: 1, warning: settling },
                ArrowKind::Teammate { seat: 1, entering: false },
                ArrowKind::Frog { side: Side::Enemy },
            ]
        );
        assert_eq!(ind.arrows[0].target, Position::new(450.0, 150.0), "the nearest threat leads");
        assert_eq!(ind.folded[Edge::Right.index()], EdgeCount { count: 1, warning: true, at: ind.folded[Edge::Right.index()].at });
        assert_eq!(ind.folded[Edge::Left.index()].count, 1, "the plain enemy has no room at all");
        assert!(!ind.folded[Edge::Left.index()].warning);
    }

    /// Eight arrows at most, by priority: a lane threat first, however far,
    /// then the nearest enemies; the rest are counted on their edges.
    #[test]
    fn the_cap_fills_by_priority_and_folds_the_rest_per_edge() {
        let mut tanks: Vec<TankView> = (0..9).map(|i| enemy(10 + i, 450.0, 150.0 + (i as f32 - 4.0) * 40.0)).collect();
        tanks.extend((0..3).map(|i| enemy(20 + i, -100.0, 150.0 + (i as f32 - 1.0) * 80.0)));
        let ind = Awareness::new().frame(&scene(0.0, tanks.clone()), &screen(), &Tuning::DEFAULT);
        assert_eq!(ind.arrows.len(), 8);
        assert!(ind.arrows.iter().all(|a| a.kind == ArrowKind::Enemy { count: 1, warning: None } && a.place.edge == Edge::Right));
        let folded: Vec<u32> = ind.folded.iter().map(|e| e.count).collect();
        assert_eq!(folded, vec![0, 1, 0, 3], "the farthest of the east nine and all three to the west");
        assert_eq!(ind.folded[Edge::Left.index()].at, Vec2::new(10.0, 150.0), "the count sits where its nearest would have");
        tanks[10].lane = true;
        let ind = Awareness::new().frame(&scene(0.0, tanks), &screen(), &Tuning::DEFAULT);
        assert_eq!(ind.arrows.len(), 8);
        assert_eq!(ind.arrows[0].kind, ArrowKind::Enemy { count: 1, warning: Some(LaneWarning { settle: 0.0, flash: 0.0 }) });
        assert_eq!(ind.arrows[0].target, Position::new(-100.0, 150.0));
        let folded: Vec<u32> = ind.folded.iter().map(|e| e.count).collect();
        assert_eq!(folded, vec![0, 2, 0, 2]);
        assert!(!ind.folded.iter().any(|e| e.warning));
    }

    /// The lane is the AI's fire rule: the seat ahead on the axis the
    /// enemy faces, within `enemy_fire_align_px` of it, inside the attack
    /// range and what the sky lets it see.
    #[test]
    fn lined_up_is_the_ai_fire_rule() {
        let t = Tuning::DEFAULT;
        let far = 10_000.0;
        assert!(lined_up(Position::new(450.0, 160.0), Dir::Left, SEAT, far, &t));
        assert!(lined_up(Position::new(205.0, 400.0), Dir::Up, SEAT, far, &t));
        assert!(!lined_up(Position::new(450.0, 160.0), Dir::Up, SEAT, far, &t), "facing off the lane");
        assert!(!lined_up(Position::new(450.0, 160.0), Dir::Right, SEAT, far, &t), "facing away");
        assert!(!lined_up(Position::new(450.0, 180.0), Dir::Left, SEAT, far, &t), "30 px off the row");
        assert!(!lined_up(Position::new(560.0, 150.0), Dir::Left, SEAT, far, &t), "past the attack range");
        assert!(!lined_up(Position::new(450.0, 160.0), Dir::Left, SEAT, 200.0, &t), "past what the sky lets it see");
        assert_eq!(facing_of(270.0), Dir::Left);
        assert_eq!(facing_of(-90.0), Dir::Left);
        assert_eq!(facing_of(358.6), Dir::Up, "a quantised heading reads as its quarter turn");
    }

    /// The warning runs from the frame the enemy lines up, settles over
    /// `enemy_aim_settle`, flashes when it fires down the lane and outlives
    /// the lane only as long as the flash.
    #[test]
    fn a_lane_threat_warns_while_its_aim_settles_and_flashes_on_its_shot() {
        let t = Tuning::DEFAULT;
        let at = |time: f32, lane: bool| scene(time, vec![TankView { lane, ..enemy(5, 450.0, 150.0) }]);
        let mut aw = Awareness::new();
        assert_eq!(warning(&aw.frame(&at(0.0, true), &screen(), &t)), Some(Some(LaneWarning { settle: 0.0, flash: 0.0 })));
        assert_eq!(warning(&aw.frame(&at(0.125, true), &screen(), &t)), Some(Some(LaneWarning { settle: 0.5, flash: 0.0 })));
        assert_eq!(warning(&aw.frame(&at(0.3, true), &screen(), &t)), Some(Some(LaneWarning { settle: 1.0, flash: 0.0 })));
        aw.note(0.3, Note::Fired { slot: 5, at_seat: true });
        assert_eq!(warning(&aw.frame(&at(0.3, true), &screen(), &t)), Some(Some(LaneWarning { settle: 1.0, flash: 1.0 })));
        let w = warning(&aw.frame(&at(0.45, true), &screen(), &t)).flatten().expect("warned");
        assert!(close(w.flash, 0.5) && w.settle == 1.0, "{w:?}");
        let w = warning(&aw.frame(&at(0.5, false), &screen(), &t)).flatten().expect("the flash outlives the lane");
        assert!(w.settle == 0.0 && close(w.flash, 1.0 / 3.0), "{w:?}");
        assert_eq!(warning(&aw.frame(&at(0.7, false), &screen(), &t)), Some(None));
        aw.note(0.8, Note::Fired { slot: 5, at_seat: false });
        assert_eq!(warning(&aw.frame(&at(0.8, false), &screen(), &t)), Some(None), "a shot off the lane flashes nothing");
        // A seat that is a wreck is lined up on by nobody.
        let mut wrecked = at(1.0, true);
        wrecked.seat = Some(SeatView { slot: 0, pos: SEAT, wreck: true, gate: None });
        assert_eq!(warning(&aw.frame(&wrecked, &screen(), &t)), Some(None));
    }

    /// A tank in tall grass is hidden unless it fired in the last
    /// `indicator_reveal_fire_seconds` or is within `indicator_reveal_px`.
    #[test]
    fn a_concealed_enemy_gets_no_arrow_unless_it_fired_or_came_close() {
        let t = Tuning::DEFAULT;
        assert!(concealed(true, None, 500.0, &t));
        assert!(!concealed(false, None, 500.0, &t), "out of the grass");
        assert!(!concealed(true, Some(1.0), 500.0, &t), "fired a second ago");
        assert!(concealed(true, Some(2.0), 500.0, &t), "fired two seconds ago");
        assert!(!concealed(true, None, 50.0, &t), "within two cells");
        // Slot 5 hides off the screen; slot 6 stands on it in the open.
        let in_view = Position::new(300.0, 100.0);
        let at = |time: f32| scene(time, vec![TankView { in_grass: true, ..enemy(5, 450.0, 150.0) }, enemy(6, in_view.x, in_view.y)]);
        let mut aw = Awareness::new();
        let ind = aw.frame(&at(0.0), &screen(), &t);
        assert!(ind.arrows.is_empty());
        assert_eq!(ind.in_sight, vec![in_view], "the minimap's enemies, the screen's own among them");
        aw.note(1.0, Note::Fired { slot: 5, at_seat: false });
        let ind = aw.frame(&at(1.0), &screen(), &t);
        assert_eq!(enemy_arrows(&ind), 1, "firing gives it away");
        assert_eq!(ind.in_sight, vec![Position::new(450.0, 150.0), in_view]);
        assert_eq!(enemy_arrows(&aw.frame(&at(2.4), &screen(), &t)), 1);
        let ind = aw.frame(&at(2.6), &screen(), &t);
        assert_eq!(enemy_arrows(&ind), 0, "hidden again");
        assert_eq!(ind.in_sight, vec![in_view]);
        assert_eq!(ind.last_seen.iter().map(|m| m.at).collect::<Vec<_>>(), vec![Position::new(450.0, 150.0)]);
        // Within two cells it is seen in the grass: a screen this small
        // still has it off the edge.
        let tiny = ViewFrame::new(Rectangle::new(180.0, 130.0, 40.0, 40.0), 1.0, Vec2::zero(), Rectangle::new(0.0, 0.0, 40.0, 40.0), &t);
        let near = scene(3.0, vec![TankView { in_grass: true, ..enemy(5, 250.0, 150.0) }]);
        assert_eq!(enemy_arrows(&Awareness::new().frame(&near, &tiny, &t)), 1);
    }

    /// An enemy that slips out of sight leaves a marker where it was last
    /// seen, which stays there whatever the hidden tank does, fades, and
    /// goes when the tank is seen again or is wrecked.
    #[test]
    fn an_enemy_that_slips_out_of_sight_leaves_a_marker_that_never_moves() {
        let t = Tuning::DEFAULT;
        let at = |time: f32, x: f32, y: f32, in_grass: bool| scene(time, vec![TankView { in_grass, ..enemy(5, x, y) }]);
        let seen = Position::new(600.0, 150.0);
        let mut aw = Awareness::new();
        let ind = aw.frame(&at(0.0, seen.x, seen.y, false), &screen(), &t);
        assert!(ind.last_seen.is_empty() && enemy_arrows(&ind) == 1);
        let ind = aw.frame(&at(0.1, 640.0, 150.0, true), &screen(), &t);
        assert_eq!(enemy_arrows(&ind), 0);
        assert_eq!(ind.last_seen.len(), 1);
        let mark = ind.last_seen[0];
        assert_eq!((mark.at, mark.fade), (seen, 1.0));
        assert_eq!(mark.edge.map(|e| e.edge), Some(Edge::Right), "off the screen: a hollow arrow at the edge");
        let ind = aw.frame(&at(1.0, 700.0, 220.0, true), &screen(), &t);
        assert_eq!(ind.last_seen.iter().map(|m| m.at).collect::<Vec<_>>(), vec![seen], "it never follows the tank");
        assert!(close(ind.last_seen[0].fade, 1.0 - 0.9 / 4.0), "{:?}", ind.last_seen);
        assert_eq!(ind.last_seen[0].edge, mark.edge);
        assert!(aw.frame(&at(4.2, 700.0, 220.0, true), &screen(), &t).last_seen.is_empty(), "faded out");
        // Seen again, the marker goes at once.
        let mut aw = Awareness::new();
        aw.frame(&at(0.0, seen.x, seen.y, false), &screen(), &t);
        aw.frame(&at(0.1, 640.0, 150.0, true), &screen(), &t);
        let ind = aw.frame(&at(0.5, 680.0, 150.0, false), &screen(), &t);
        assert!(ind.last_seen.is_empty() && enemy_arrows(&ind) == 1);
        // A wreck hides nothing: its marker goes with it.
        aw.frame(&at(0.6, 700.0, 150.0, true), &screen(), &t);
        let wreck = scene(0.7, vec![TankView { wreck: true, ..enemy(5, 700.0, 150.0) }]);
        assert!(aw.frame(&wreck, &screen(), &t).last_seen.is_empty());
    }

    /// At night or in fog an enemy further from the seat than the sky lets
    /// one see gets no arrow, and one that leaves that reach is last seen
    /// where it did.
    #[test]
    fn the_sky_hides_an_enemy_past_its_sight() {
        let t = Tuning::DEFAULT;
        let mut night = scene(0.0, vec![enemy(5, 650.0, 150.0)]);
        night.sight = Some(480.0);
        let mut aw = Awareness::new();
        assert_eq!(enemy_arrows(&aw.frame(&night, &screen(), &t)), 1, "450 px away, inside the 480 it sees");
        night.time = 0.5;
        night.tanks[0].pos = Position::new(800.0, 150.0);
        let ind = aw.frame(&night, &screen(), &t);
        assert_eq!(enemy_arrows(&ind), 0, "600 px away in the dark");
        assert_eq!(ind.last_seen.iter().map(|m| m.at).collect::<Vec<_>>(), vec![Position::new(650.0, 150.0)]);
        let clear = scene(0.0, vec![enemy(5, 2200.0, 150.0)]);
        assert_eq!(enemy_arrows(&Awareness::new().frame(&clear, &screen(), &t)), 1, "a clear sky hides nothing");
    }

    /// A hit holds the arc toward its source for `indicator_hit_arc_seconds`;
    /// a newer hit takes its place.
    #[test]
    fn a_hit_holds_an_arc_toward_its_source() {
        let t = Tuning::DEFAULT;
        let mut aw = Awareness::new();
        aw.note(1.0, Note::Hit { from: Vec2::new(1.0, 0.0) });
        let held = HitArc { dir: Vec2::new(1.0, 0.0), fade: 1.0, at: SEAT };
        assert_eq!(aw.frame(&scene(1.0, vec![]), &screen(), &t).hit_arc, Some(held), "round the seat's tank");
        let arc = aw.frame(&scene(1.4, vec![]), &screen(), &t).hit_arc.expect("held");
        assert!(close(arc.fade, 0.5), "{arc:?}");
        aw.note(1.5, Note::Hit { from: Vec2::new(0.0, -1.0) });
        let arc = aw.frame(&scene(1.5, vec![]), &screen(), &t).hit_arc.expect("the newer hit");
        assert_eq!((arc.dir, arc.fade), (Vec2::new(0.0, -1.0), 1.0));
        let gone = Scene { seat: None, ..scene(1.6, vec![]) };
        assert_eq!(aw.frame(&gone, &screen(), &t).hit_arc, None, "no tank, nothing to draw it round");
        assert_eq!(aw.frame(&scene(2.4, vec![]), &screen(), &t).hit_arc, None);
    }

    /// A gate flashes from the frame a tank is first seen rolling in
    /// through it, and again when it comes through; the tank in the lane
    /// is no enemy arrow yet.
    #[test]
    fn a_gate_flashes_while_a_tank_rolls_in_and_after_it_comes_through() {
        let t = Tuning::DEFAULT;
        let gate = Position::new(1000.0, 150.0);
        let rolling = |time: f32| scene(time, vec![TankView { gate: Some(gate), ..enemy(9, 1040.0, 150.0) }]);
        let mut aw = Awareness::new();
        let ind = aw.frame(&rolling(0.0), &screen(), &t);
        assert_eq!(ind.gates, vec![GateFlash { at: gate, screen: Vec2::new(1000.0, 150.0), on_screen: false, flash: 1.0 }]);
        assert_eq!(kinds(&ind), vec![ArrowKind::Gate { flash: 1.0 }]);
        let ind = aw.frame(&rolling(1.0), &screen(), &t);
        assert!(close(ind.gates[0].flash, 2.0 / 3.0), "no fresh flash while the same tank drives on: {:?}", ind.gates);
        aw.note(2.0, Note::Entered { slot: 9, at: Position::new(990.0, 150.0) });
        let ind = aw.frame(&scene(2.0, vec![enemy(9, 990.0, 150.0)]), &screen(), &t);
        assert_eq!(ind.gates.len(), 1, "the lane's mouth and its inside are one gate");
        assert_eq!((ind.gates[0].at, ind.gates[0].flash), (gate, 1.0));
        assert_eq!(kinds(&ind), vec![ArrowKind::Gate { flash: 1.0 }, ArrowKind::Enemy { count: 1, warning: None }]);
        assert!(aw.frame(&scene(5.1, vec![enemy(9, 990.0, 150.0)]), &screen(), &t).gates.is_empty());
        // A gate the screen shows flashes on it, with no arrow.
        let near = scene(0.0, vec![TankView { gate: Some(Position::new(300.0, 150.0)), ..enemy(9, 340.0, 150.0) }]);
        let ind = Awareness::new().frame(&near, &screen(), &t);
        assert!(ind.gates[0].on_screen && ind.arrows.is_empty());
    }

    /// A teammate driving back in through a gate shows there, with its
    /// seat; a wrecked one waiting on the field has no arrow.
    #[test]
    fn a_teammate_driving_back_in_shows_at_its_gate() {
        let t = Tuning::DEFAULT;
        let back = TankView { seat: Some(2), gate: Some(Position::new(16.0, 150.0)), ..enemy(2, -50.0, 150.0) };
        let down = TankView { seat: Some(3), wreck: true, ..enemy(3, 1000.0, 150.0) };
        let ind = Awareness::new().frame(&scene(0.0, vec![back, down]), &screen(), &t);
        let teammates: Vec<(ArrowKind, Edge)> =
            ind.arrows.iter().filter(|a| matches!(a.kind, ArrowKind::Teammate { .. })).map(|a| (a.kind, a.place.edge)).collect();
        assert_eq!(teammates, vec![(ArrowKind::Teammate { seat: 2, entering: true }, Edge::Left)]);
        assert_eq!(ind.gates.len(), 1, "its gate flashes too");
    }

    /// A new round forgets the last one's memory, whether it is announced
    /// or only seen in the clock going back.
    #[test]
    fn a_new_round_forgets_everything() {
        let t = Tuning::DEFAULT;
        let at = |time: f32, in_grass: bool| scene(time, vec![TankView { in_grass, ..enemy(5, 600.0, 150.0) }]);
        let marked = || {
            let mut aw = Awareness::new();
            aw.frame(&at(10.0, false), &screen(), &t);
            aw.note(10.0, Note::Hit { from: Vec2::new(1.0, 0.0) });
            assert_eq!(aw.frame(&at(10.1, true), &screen(), &t).last_seen.len(), 1);
            aw
        };
        let mut aw = marked();
        aw.note(10.2, Note::RoundStarted);
        let ind = aw.frame(&at(10.2, true), &screen(), &t);
        assert!(ind.last_seen.is_empty() && ind.hit_arc.is_none());
        let mut aw = marked();
        let ind = aw.frame(&at(0.0, true), &screen(), &t);
        assert!(ind.last_seen.is_empty() && ind.hit_arc.is_none());
    }

    /// The knobs are the `indicators` group's rows.
    #[test]
    fn the_knobs_are_rows_in_the_indicators_group() {
        let rows: Vec<_> = Tuning::SCHEMA.iter().filter(|m| m.name.starts_with("indicator_")).collect();
        assert_eq!(rows.len(), 22);
        assert!(rows.iter().all(|m| m.group == "indicators"), "{rows:?}");
        assert!(Tuning::SCHEMA.iter().filter(|m| m.group == "indicators").all(|m| m.name.starts_with("indicator_")));
    }

    // ---- through a round ----

    /// An open 34 x 17 field: the start on row 8 near the west edge, and a
    /// meadow of tall grass on the east side, columns 24 to 28, rows 6 to
    /// 10.
    fn open_map() -> String {
        let mut map = String::from("version = 1\ncells.\"4,8\" = { kind = \"start\" }\n");
        for c in 24..=28 {
            for r in 6..=10 {
                map.push_str(&format!("cells.\"{c},{r}\" = {{ kind = \"tall_grass\" }}\n"));
            }
        }
        map
    }

    fn round(map: &str, enemies: usize, players: PlayerCount) -> Game {
        let mut game = Game::default();
        game.enemy_count_override = Some(enemies);
        game.seed_override = Some(7);
        game.players = players;
        game.level_overrides.mission = Some(Mission::Destroy);
        game.map = MapFile::from_toml_str(map).expect("test map parses");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        game
    }

    fn step(game: &mut Game) {
        let (w, h) = game.map.field_size();
        game.update(Input::default(), PHYSICS_FIXED_DT, w, h);
    }

    /// A screen of `w` x `h` points showing the world around `p` at one
    /// point a pixel.
    fn view_sized(p: Position, w: f32, h: f32) -> ViewFrame {
        let world = Rectangle::new(p.x - w * 0.5, p.y - h * 0.5, w, h);
        ViewFrame::new(world, 1.0, Vec2::zero(), Rectangle::new(0.0, 0.0, w, h), &Tuning::DEFAULT)
    }

    fn view_around(p: Position) -> ViewFrame {
        view_sized(p, 400.0, 300.0)
    }

    #[test]
    fn through_the_round_an_enemy_past_the_view_gets_an_arrow_on_the_right_edge() {
        let mut game = round(&open_map(), 1, PlayerCount::ONE);
        let me = cell_to_world(4, 8);
        game.debug_teleport(0, me, Some(90.0)).expect("the seat");
        let there = cell_to_world(20, 8);
        game.debug_teleport(1, there, Some(0.0)).expect("the enemy in slot 1");
        let view = view_around(me);
        let ind = Awareness::new().gather(&game, 0, &view);
        assert_eq!(ind.arrows.len(), 1, "{:?}", ind.arrows);
        let arrow = ind.arrows[0];
        assert_eq!(arrow.kind, ArrowKind::Enemy { count: 1, warning: None });
        assert_eq!(arrow.target, there);
        assert_eq!(arrow.place.edge, Edge::Right);
        assert_eq!(arrow.place.at, Vec2::new(view.inset.x + view.inset.width, view.to_screen(me).y));
        assert!(arrow.place.dir.x > 0.999, "{arrow:?}");
    }

    #[test]
    fn through_the_round_a_concealed_enemy_gets_no_arrow_and_its_marker_stays_put() {
        let mut game = round(&open_map(), 1, PlayerCount::ONE);
        let me = cell_to_world(4, 8);
        game.debug_teleport(0, me, Some(90.0)).expect("the seat");
        let seen = cell_to_world(20, 8);
        game.debug_teleport(1, seen, Some(0.0)).expect("the enemy in slot 1");
        let view = view_around(me);
        let mut aw = Awareness::new();
        assert_eq!(enemy_arrows(&aw.gather(&game, 0, &view)), 1, "in the open it shows");
        game.debug_teleport(1, cell_to_world(26, 8), Some(0.0)).expect("into the grass");
        let ind = aw.gather(&game, 0, &view);
        assert_eq!(enemy_arrows(&ind), 0, "in the grass it does not");
        assert_eq!(ind.last_seen.iter().map(|m| m.at).collect::<Vec<_>>(), vec![seen]);
        let first = ind.last_seen[0].fade;
        // The round runs and the tank moves about in the grass: the marker
        // stays where it was seen.
        for i in 0..60 {
            step(&mut game);
            game.debug_teleport(1, cell_to_world(25 + i % 3, 7 + i % 4), Some(0.0)).expect("about the meadow");
            let ind = aw.gather(&game, 0, &view);
            assert_eq!(enemy_arrows(&ind), 0, "frame {i}");
            assert_eq!(ind.last_seen.iter().map(|m| m.at).collect::<Vec<_>>(), vec![seen], "frame {i}");
        }
        let fade = aw.gather(&game, 0, &view).last_seen[0].fade;
        assert!(fade < first && fade > 0.0, "fading on the round clock: {first} then {fade}");
    }

    /// Another round on the screen - here one on another seed, at the
    /// same frame - is not the one the marker was left in.
    #[test]
    fn through_the_round_another_round_on_the_screen_forgets_the_last() {
        let me = cell_to_world(4, 8);
        let view = view_around(me);
        let hidden_round = |seed: u64| {
            let mut game = round(&open_map(), 1, PlayerCount::ONE);
            game.seed_override = Some(seed);
            let (w, h) = game.map.field_size();
            game.init(w, h);
            game.debug_teleport(0, me, Some(90.0)).expect("the seat");
            game
        };
        let mut game = hidden_round(7);
        let mut aw = Awareness::new();
        game.debug_teleport(1, cell_to_world(20, 8), Some(0.0)).expect("in the open");
        aw.gather(&game, 0, &view);
        game.debug_teleport(1, cell_to_world(26, 8), Some(0.0)).expect("into the grass");
        assert_eq!(aw.gather(&game, 0, &view).last_seen.len(), 1);
        let mut other = hidden_round(8);
        other.debug_teleport(1, cell_to_world(26, 8), Some(0.0)).expect("in the grass");
        assert_eq!(other.frame(), game.frame());
        assert!(aw.gather(&other, 0, &view).last_seen.is_empty());
    }

    #[test]
    fn through_the_round_the_lane_warning_shows_only_to_the_seat_it_is_lined_up_on() {
        let (p0, p1) = (cell_to_world(4, 8), cell_to_world(4, 3));
        let lined_up_round = |map: &str| {
            let mut game = round(map, 1, PlayerCount::TWO);
            game.debug_teleport(0, p0, Some(90.0)).expect("seat 0");
            game.debug_teleport(1, p1, Some(90.0)).expect("seat 1");
            // The round's one enemy, slot 2, facing west down seat 0's row
            // eight cells off: inside the attack range, off both screens.
            game.debug_teleport(2, cell_to_world(12, 8), Some(270.0)).expect("the enemy in slot 2");
            game
        };
        let (v0, v1) = (view_sized(p0, 200.0, 160.0), view_sized(p1, 200.0, 160.0));
        let game = lined_up_round(&open_map());
        let mut seat0 = Awareness::new();
        let mut seat1 = Awareness::new();
        assert_eq!(warning(&seat0.gather(&game, 0, &v0)), Some(Some(LaneWarning { settle: 0.0, flash: 0.0 })));
        assert_eq!(warning(&seat1.gather(&game, 1, &v1)), Some(None), "seat 1 is not on its row");
        // A wall across the row hides the seat from it: no warning.
        let walled = format!("{}cells.\"8,8\" = {{ kind = \"wall\", material = \"brick\" }}\n", open_map());
        let game = lined_up_round(&walled);
        assert_eq!(warning(&Awareness::new().gather(&game, 0, &v0)), Some(None));
    }

    #[test]
    fn through_the_round_a_shell_that_lands_points_the_arc_back_up_its_lane() {
        let mut game = round(&open_map(), 1, PlayerCount::ONE);
        let me = cell_to_world(4, 8);
        game.debug_teleport(0, me, Some(90.0)).expect("the seat");
        game.debug_set_tank(0, &TankPatch { shield_hp: Some(0.0), ..TankPatch::default() }).expect("no spawn shield");
        // Seven cells east on the same row, facing west: in range, lined
        // up, just off a 400 pt screen.
        game.debug_teleport(1, cell_to_world(11, 8), Some(270.0)).expect("the enemy in slot 1");
        let view = view_around(me);
        let mut aw = Awareness::new();
        let mut flashed = false;
        let mut arc = None;
        for _ in 0..600 {
            step(&mut game);
            aw.observe_events(&game, 0);
            let fired = game.events().iter().any(|e| matches!(e, Event::Fired { slot: 1, .. }));
            let hit = game.events().iter().any(|e| matches!(e, Event::Hit { target: HitTarget::Player { player: 0 }, .. }));
            let ind = aw.gather(&game, 0, &view);
            if fired {
                let w = warning(&ind).flatten().expect("lined up on the seat as it fired");
                flashed |= w.flash == 1.0 && w.settle == 1.0;
            }
            if hit {
                arc = ind.hit_arc;
                break;
            }
        }
        assert!(flashed, "the shot flashed the warning");
        let arc = arc.expect("a shell landed on the seat");
        assert!(arc.dir.x > 0.95, "back east up the row it flew: {arc:?}");
        assert_eq!(arc.fade, 1.0);
    }

    #[test]
    fn through_the_round_a_tank_in_a_gate_lane_flashes_its_gate_and_is_no_enemy_yet() {
        let mut game = round(&open_map(), 2, PlayerCount::ONE);
        let me = cell_to_world(4, 8);
        game.debug_teleport(0, me, Some(90.0)).expect("the seat");
        // Slot 1 as a replica draws a wave tank driving in: off the field's
        // east edge, with no `RollIn`.
        let (w, _) = game.map.field_size();
        game.debug_teleport(1, Position::new(w + 40.0, me.y), Some(270.0)).expect("slot 1");
        // Slot 2 as a local round carries one: a `RollIn` to its gate.
        let inside = cell_to_world(31, 2);
        game.debug_teleport(2, cell_to_world(32, 2), Some(270.0)).expect("slot 2");
        let entity = game.tank_entity_by_slot(2).expect("slot 2's tank");
        game.world.insert_one(entity, RollIn { to: inside }).expect("a live entity");
        let ind = Awareness::new().gather(&game, 0, &view_around(me));
        assert_eq!(enemy_arrows(&ind), 0, "{:?}", ind.arrows);
        let gates: Vec<Position> = ind.gates.iter().map(|g| g.at).collect();
        assert_eq!(gates, vec![Position::new(w - OBSTACLE_GRID_SIZE * 0.5, me.y), inside]);
        assert!(ind.gates.iter().all(|g| g.flash == 1.0 && !g.on_screen));
        assert_eq!(ind.arrows.iter().filter(|a| matches!(a.kind, ArrowKind::Gate { .. })).count(), 2);
    }

    #[test]
    fn the_scene_reads_the_sky_and_the_grass() {
        let mut game = round(&open_map(), 1, PlayerCount::ONE);
        game.debug_teleport(1, cell_to_world(26, 8), Some(0.0)).expect("into the grass");
        let scene = Scene::of(&game, 0);
        assert_eq!(scene.sight, None, "a clear sky");
        assert_eq!(scene.tanks.iter().map(|tv| (tv.slot, tv.in_grass)).collect::<Vec<_>>(), vec![(1, true)]);
        assert_eq!(scene.seat.map(|s| s.slot), Some(0));
        game.weather = Weather::Night;
        let t = Tuning::DEFAULT;
        assert_eq!(Scene::of(&game, 0).sight, Some(t.enemy_view_range * t.night_sight_factor));
    }
}

#[cfg(test)]
mod picture_tests {
    use super::*;

    /// A 400 x 300 point screen showing the world from its origin at one
    /// point a pixel, its arrows on the rectangle 10 pt in: (10, 10) to
    /// (390, 290).
    fn screen() -> ViewFrame {
        let r = Rectangle::new(0.0, 0.0, 400.0, 300.0);
        ViewFrame::new(r, 1.0, Vec2::zero(), r, &Tuning::DEFAULT)
    }

    const RIGHT: Vec2 = Vec2::new(1.0, 0.0);
    const LEFT: Vec2 = Vec2::new(-1.0, 0.0);
    const UP: Vec2 = Vec2::new(0.0, -1.0);

    /// An arrow of `kind` at full size whose tip is at `at`, pointing along
    /// `dir`.
    fn arrow(kind: ArrowKind, at: Vec2, edge: Edge, dir: Vec2) -> Arrow {
        Arrow { kind, place: EdgePoint { at, edge, dir }, target: at + dir * 200.0, scale: 1.0, alpha: 1.0, cells: 12.4 }
    }

    fn enemy(at: Vec2, edge: Edge, dir: Vec2) -> Arrow {
        arrow(ArrowKind::Enemy { count: 1, warning: None }, at, edge, dir)
    }

    fn shown(arrows: Vec<Arrow>) -> Indicators {
        Indicators { arrows, ..Indicators::default() }
    }

    /// The picture of `seats` on `screen()` at round time `time`, labels at
    /// the font's own size.
    fn draw(seats: &[Indicators], time: f32) -> Picture {
        picture(seats, &screen(), time, &Tuning::DEFAULT, 10)
    }

    fn rgb(c: Color) -> (u8, u8, u8) {
        (c.r, c.g, c.b)
    }

    /// The fills of `fills` in `color`, whatever their opacity.
    fn in_color(fills: &[Fill], color: Color) -> Vec<Fill> {
        fills.iter().copied().filter(|f| rgb(f.color) == rgb(color)).collect()
    }

    /// The box round `fills`: left, top, right, bottom.
    fn extent(fills: &[Fill]) -> (i32, i32, i32, i32) {
        let l = fills.iter().map(|f| f.x).min().expect("something drawn");
        let t = fills.iter().map(|f| f.y).min().expect("something drawn");
        let r = fills.iter().map(|f| f.x + f.w).max().expect("something drawn");
        let b = fills.iter().map(|f| f.y + f.h).max().expect("something drawn");
        (l, t, r, b)
    }

    /// Every 2 px block `fills` cover, as block coordinates.
    fn covered(fills: &[Fill]) -> BTreeSet<(i32, i32)> {
        let mut set = BTreeSet::new();
        for f in fills {
            for y in (f.y..f.y + f.h).step_by(B as usize) {
                for x in (f.x..f.x + f.w).step_by(B as usize) {
                    set.insert((x.div_euclid(B), y.div_euclid(B)));
                }
            }
        }
        set
    }

    /// `cell` mirrored across the axis of the compass heading `octant`.
    fn mirror(cell: (i32, i32), octant: i32) -> (i32, i32) {
        let (i, j) = cell;
        match octant.rem_euclid(4) {
            0 => (i, -j),
            1 => (j, i),
            2 => (-i, j),
            _ => (-j, -i),
        }
    }

    /// Each of the eight headings draws an arrowhead pointing that way: one
    /// tip block on its axis, furthest along it, the shape mirrored about
    /// the axis, and `len` points long along an axis.
    #[test]
    fn an_arrow_points_its_way_at_every_compass_heading() {
        for octant in 0..8 {
            let a = octant as f32 * std::f32::consts::FRAC_PI_4;
            let dir = Vec2::new(a.cos(), a.sin());
            let cells = arrow_cells(dir, 16.0);
            let along = |c: &(i32, i32)| c.0 as f32 * dir.x + c.1 as f32 * dir.y;
            let reach = cells.iter().map(along).fold(f32::MIN, f32::max);
            let tips: Vec<&(i32, i32)> = cells.iter().filter(|c| (along(c) - reach).abs() < 1e-3).collect();
            assert_eq!(tips.len(), 1, "octant {octant}: one tip, {cells:?}");
            let tip = *tips[0];
            assert!((tip.1 as f32 * dir.x - tip.0 as f32 * dir.y).abs() < 1e-3, "octant {octant}: the tip on the axis");
            let set: BTreeSet<(i32, i32)> = cells.iter().copied().collect();
            assert!(cells.iter().all(|&c| set.contains(&mirror(c, octant))), "octant {octant}: mirrored about its axis");
            if octant % 2 == 0 {
                let span = cells.iter().map(along).fold(f32::MIN, f32::max) - cells.iter().map(along).fold(f32::MAX, f32::min);
                assert_eq!(span.round() as i32 + 1, 8, "octant {octant}: 16 points is eight blocks");
            }
        }
        assert!(arrow_cells(Vec2::zero(), 16.0).is_empty(), "no direction, no arrow");
        assert!(arrow_cells(RIGHT, 9.6).len() < arrow_cells(RIGHT, 16.0).len(), "a far arrow is smaller");
    }

    /// An arrow keeps its blocks until its direction crosses half a step
    /// to the next heading, so it does not crawl while its target drifts.
    #[test]
    fn an_arrow_holds_its_shape_until_its_direction_crosses_half_a_step() {
        let at = |deg: f32| arrow_cells(Vec2::new(deg.to_radians().cos(), deg.to_radians().sin()), 16.0);
        assert_eq!(at(20.0), at(0.0));
        assert_eq!(at(-20.0), at(0.0));
        assert_eq!(at(25.0), at(45.0));
        assert_ne!(at(0.0), at(45.0));
    }

    /// A rim surrounds a shape a block wide and never covers it.
    #[test]
    fn a_rim_surrounds_a_shape_and_never_covers_it() {
        let cells = arrow_cells(Vec2::new(1.0, 1.0), 16.0);
        let shape: BTreeSet<(i32, i32)> = cells.iter().copied().collect();
        let rim: BTreeSet<(i32, i32)> = rim_cells(&cells).into_iter().collect();
        assert!(shape.is_disjoint(&rim));
        let touches = |c: &(i32, i32)| (-1..=1).any(|dj| (-1..=1).any(|di| shape.contains(&(c.0 + di, c.1 + dj))));
        assert!(rim.iter().all(touches), "every rim block touches the shape");
        for &(i, j) in &cells {
            for dj in -1..=1 {
                for di in -1..=1 {
                    let c = (i + di, j + dj);
                    assert!(shape.contains(&c) || rim.contains(&c), "{c:?} round {:?} is shape or rim", (i, j));
                }
            }
        }
    }

    /// A row of blocks is one fill, on the 2 px grid round its centre block.
    #[test]
    fn a_row_of_blocks_is_painted_as_one_fill() {
        let mut out = Vec::new();
        put(&mut out, (10, 20), &[(2, 0), (0, 0), (1, 0), (5, 0), (0, 1)], WHITE);
        assert_eq!(
            out,
            vec![
                Fill { x: 20, y: 40, w: 6, h: 2, color: WHITE },
                Fill { x: 30, y: 40, w: 2, h: 2, color: WHITE },
                Fill { x: 20, y: 42, w: 2, h: 2, color: WHITE },
            ]
        );
        let mut none = Vec::new();
        put(&mut none, (0, 0), &[(0, 0)], Color::new(255, 255, 255, 0));
        assert!(none.is_empty(), "nothing at no opacity");
    }

    /// An arrow's tip is on its edge point, so the arrow - rim and all -
    /// lies inside the screen on every edge.
    #[test]
    fn an_arrows_tip_sits_on_its_edge_point_inside_the_screen() {
        for (at, edge, dir) in [
            (Vec2::new(390.0, 150.0), Edge::Right, RIGHT),
            (Vec2::new(10.0, 150.0), Edge::Left, LEFT),
            (Vec2::new(200.0, 10.0), Edge::Top, UP),
            (Vec2::new(200.0, 290.0), Edge::Bottom, Vec2::new(0.0, 1.0)),
            (Vec2::new(390.0, 290.0), Edge::Bottom, Vec2::new(0.7071, 0.7071)),
        ] {
            let p = draw(&[shown(vec![enemy(at, edge, dir)])], 0.0);
            let (l, t, r, b) = extent(&p.screen);
            assert!(l >= 0 && t >= 0 && r <= 400 && b <= 300, "{edge:?}: inside the screen, {:?}", (l, t, r, b));
            let body = extent(&in_color(&p.screen, HOSTILE));
            let front = match edge {
                Edge::Right => body.2 as f32 - at.x,
                Edge::Left => at.x - body.0 as f32,
                Edge::Top => at.y - body.1 as f32,
                Edge::Bottom => body.3 as f32 - at.y,
            };
            assert!(front.abs() <= 2.0 * B as f32, "{edge:?}: the tip on the edge point, {front} off");
        }
    }

    /// Each kind of arrow wears its own colour inside a near-black rim; the
    /// enemy frog's rim is red.
    #[test]
    fn every_kind_of_arrow_wears_its_colour() {
        let at = Vec2::new(390.0, 150.0);
        let body = |kind: ArrowKind| {
            let p = draw(&[shown(vec![arrow(kind, at, Edge::Right, RIGHT)])], 0.0);
            let colors: BTreeSet<(u8, u8, u8)> = p.screen.iter().map(|f| rgb(f.color)).collect();
            colors
        };
        let set = |cs: &[Color]| cs.iter().map(|&c| rgb(c)).collect::<BTreeSet<_>>();
        assert_eq!(body(ArrowKind::Enemy { count: 1, warning: None }), set(&[HOSTILE, RIM]));
        assert_eq!(body(ArrowKind::Teammate { seat: 1, entering: false }), set(&[crate::tank::team_color(1), RIM]));
        assert_eq!(body(ArrowKind::Frog { side: Side::Player }), set(&[FROG_GREEN, RIM]));
        assert_eq!(body(ArrowKind::Frog { side: Side::Enemy }), set(&[FROG_GREEN, HOSTILE]));
        assert_eq!(body(ArrowKind::Gate { flash: 1.0 }), set(&[GATE_AMBER, RIM]));
    }

    /// A lined-up enemy's arrow gets a ring that pulses, dark red while
    /// its aim settles and bright once it has; the shot down the lane turns
    /// the arrow white on a white disc, which outlives the white arrow.
    #[test]
    fn a_lane_warning_rings_its_arrow_and_its_shot_flashes_white() {
        let at = Vec2::new(390.0, 150.0);
        let warned = |settle: f32, flash: f32| {
            let kind = ArrowKind::Enemy { count: 1, warning: Some(LaneWarning { settle, flash }) };
            shown(vec![arrow(kind, at, Edge::Right, RIGHT)])
        };
        let hz = Tuning::DEFAULT.indicator_pulse_hz;
        let (peak, trough) = (0.25 / hz, 0.75 / hz);
        let plain = draw(&[shown(vec![enemy(at, Edge::Right, RIGHT)])], peak);
        let settling = draw(&[warned(0.0, 0.0)], peak);
        assert!(!in_color(&settling.screen, HOSTILE_DEEP).is_empty(), "a dark ring while the aim settles");
        let plain_box = extent(&plain.screen);
        let ring_box = extent(&settling.screen);
        assert!(ring_box.1 < plain_box.1 && ring_box.3 > plain_box.3, "the ring reaches round the arrow");
        let settled = draw(&[warned(1.0, 0.0)], peak);
        assert!(in_color(&settled.screen, HOSTILE_DEEP).is_empty(), "bright once it has settled");
        let low = draw(&[warned(1.0, 0.0)], trough);
        assert!(extent(&settled.screen).3 - extent(&settled.screen).1 > extent(&low.screen).3 - extent(&low.screen).1, "the pulse swells it");
        // The arrow is painted last: white the frame it fires, on a white
        // disc, then red again on the disc as it fades.
        let fired = draw(&[warned(1.0, 1.0)], peak);
        assert_eq!(rgb(fired.screen.last().expect("an arrow").color), rgb(WHITE));
        assert!(in_color(&fired.screen, WHITE).iter().all(|f| f.color.a == 255), "a full white flash");
        let fading = draw(&[warned(1.0, 0.4)], peak);
        assert_eq!(rgb(fading.screen.last().expect("an arrow").color), rgb(HOSTILE));
        let disc = in_color(&fading.screen, WHITE);
        assert!(!disc.is_empty() && disc.iter().all(|f| f.color.a == 95), "the disc at three eighths: {disc:?}");
    }

    /// A merged arrow carries its count, a teammate its seat and a frog its
    /// distance, each behind its arrow toward the middle of the screen; a
    /// lone enemy and a gate carry nothing.
    #[test]
    fn labels_sit_behind_their_arrows_inside_the_screen() {
        let at = Vec2::new(390.0, 150.0);
        let labels = |kind: ArrowKind| draw(&[shown(vec![arrow(kind, at, Edge::Right, RIGHT)])], 0.0).labels;
        let merged = labels(ArrowKind::Enemy { count: 3, warning: None });
        assert_eq!(merged.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), vec!["3"]);
        let plate = merged[0].plate;
        let arrow_left = 390 - Tuning::DEFAULT.indicator_arrow_pt as i32 - B;
        assert!(plate.x + plate.w <= arrow_left, "behind the arrow: {plate:?}");
        assert!(plate.x >= 0 && plate.y >= 0 && plate.y + plate.h <= 300, "inside the screen");
        assert_eq!((merged[0].x, merged[0].y), (plate.x + B, plate.y + B), "the text a block in");
        let mate = labels(ArrowKind::Teammate { seat: 1, entering: false });
        assert_eq!(mate[0].text, "P2");
        assert_eq!(rgb(mate[0].color), rgb(crate::tank::team_color(1)));
        let frog = labels(ArrowKind::Frog { side: Side::Player });
        assert_eq!((frog[0].text.as_str(), rgb(frog[0].color)), ("12", rgb(FROG_GREEN)), "12.4 cells away");
        assert!(labels(ArrowKind::Enemy { count: 1, warning: None }).is_empty());
        assert!(labels(ArrowKind::Gate { flash: 1.0 }).is_empty());
        // A label by an arrow in a corner stays on the screen.
        let corner = draw(&[shown(vec![arrow(ArrowKind::Enemy { count: 4, warning: None }, Vec2::new(390.0, 290.0), Edge::Bottom, Vec2::new(0.7071, 0.7071))])], 0.0);
        let p = corner.labels[0].plate;
        assert!(p.x >= 0 && p.y >= 0 && p.x + p.w <= 400 && p.y + p.h <= 300, "{p:?}");
    }

    /// The enemies folded into an edge read `+N` where the first of them
    /// would have sat, the plate pulsing red while one of them is lined up.
    #[test]
    fn folded_counts_read_plus_n_where_the_first_would_have_sat() {
        let mut ind = Indicators::default();
        ind.folded[Edge::Right.index()] = EdgeCount { count: 5, warning: false, at: Vec2::new(390.0, 150.0) };
        let p = draw(std::slice::from_ref(&ind), 0.0);
        assert_eq!(p.labels.len(), 1);
        let label = &p.labels[0];
        assert_eq!(label.text, "+5");
        let plate = label.plate;
        assert!(plate.x + plate.w <= 400 && plate.x <= 390 && plate.x + plate.w >= 390, "on its edge point, on the screen: {plate:?}");
        assert!(plate.y <= 150 && plate.y + plate.h >= 150);
        assert_eq!(rgb(plate.color), rgb(RIM));
        ind.folded[Edge::Right.index()].warning = true;
        let hz = Tuning::DEFAULT.indicator_pulse_hz;
        assert_eq!(rgb(draw(std::slice::from_ref(&ind), 0.25 / hz).labels[0].plate.color), rgb(HOSTILE_DEEP), "red on the pulse");
        assert_eq!(rgb(draw(std::slice::from_ref(&ind), 0.75 / hz).labels[0].plate.color), rgb(RIM), "dark between");
    }

    /// A flashing gate's arrow and frame, and a teammate driving back in,
    /// blink; a teammate on the field does not.
    #[test]
    fn gates_and_teammates_driving_back_in_blink() {
        let hz = Tuning::DEFAULT.indicator_gate_blink_hz;
        let (on, off) = (0.1 / hz, 0.6 / hz);
        let gate = arrow(ArrowKind::Gate { flash: 0.8 }, Vec2::new(390.0, 150.0), Edge::Right, RIGHT);
        let back = arrow(ArrowKind::Teammate { seat: 2, entering: true }, Vec2::new(10.0, 150.0), Edge::Left, LEFT);
        let still = arrow(ArrowKind::Teammate { seat: 1, entering: false }, Vec2::new(200.0, 10.0), Edge::Top, UP);
        let mut ind = shown(vec![gate, back, still]);
        ind.gates.push(GateFlash { at: Vec2::new(100.0, 100.0), screen: Vec2::new(100.0, 100.0), on_screen: true, flash: 0.8 });
        let lit = draw(std::slice::from_ref(&ind), on);
        assert!(!in_color(&lit.screen, GATE_AMBER).is_empty() && !in_color(&lit.world, GATE_AMBER).is_empty());
        assert!(!in_color(&lit.screen, crate::tank::team_color(2)).is_empty());
        let dark = draw(std::slice::from_ref(&ind), off);
        assert!(in_color(&dark.screen, GATE_AMBER).is_empty() && dark.world.is_empty());
        assert!(in_color(&dark.screen, crate::tank::team_color(2)).is_empty(), "driving back in: blinks");
        assert!(!in_color(&dark.screen, crate::tank::team_color(1)).is_empty(), "on the field: steady");
        assert_eq!(dark.labels.len(), 2, "the seats' names stay up while their arrows blink");
    }

    /// The hit arc lies round the tank on the side the hit came from, white
    /// as it lands and then red, fading.
    #[test]
    fn the_hit_arc_faces_the_hit_and_opens_white() {
        let at = Vec2::new(100.0, 100.0);
        let arc = |fade: f32| {
            let ind = Indicators { hit_arc: Some(HitArc { dir: UP, fade, at }), ..Indicators::default() };
            draw(&[ind], 0.0).world
        };
        let fresh = arc(1.0);
        let body = in_color(&fresh, WHITE);
        assert!(!body.is_empty() && in_color(&fresh, HOSTILE).is_empty(), "white as it lands");
        let (l, t, r, b) = extent(&body);
        let radius = Tuning::DEFAULT.indicator_hit_arc_px;
        assert!(b as f32 <= at.y - radius * 0.5 && t as f32 >= at.y - radius - 3.0 * B as f32, "above the tank: {:?}", (l, t, r, b));
        assert!(((l + r) as f32 * 0.5 - at.x).abs() <= B as f32, "centred on the way the hit came");
        let later = arc(0.5);
        let red = in_color(&later, HOSTILE);
        assert!(!red.is_empty() && red.iter().all(|f| f.color.a == 127), "red, half faded: {red:?}");
    }

    /// An enemy last seen on the screen leaves a mark in the world, one
    /// last seen off it a hollow arrow at the edge.
    #[test]
    fn a_last_seen_spot_is_a_mark_in_the_world_or_a_hollow_arrow_at_the_edge() {
        let spot = |edge: Option<EdgePoint>| {
            let at = Vec2::new(120.0, 80.0);
            Indicators { last_seen: vec![LastSeen { at, screen: at, edge, fade: 0.5 }], ..Indicators::default() }
        };
        let mark = draw(&[spot(None)], 0.0);
        assert!(mark.screen.is_empty());
        let (l, t, r, b) = extent(&mark.world);
        assert!(l >= 100 && r <= 140 && t >= 60 && b <= 100, "round the spot: {:?}", (l, t, r, b));
        assert!(mark.world.iter().all(|f| f.color.a == 127), "faded by half");
        let edge = EdgePoint { at: Vec2::new(10.0, 80.0), edge: Edge::Left, dir: LEFT };
        let hollow = draw(&[spot(Some(edge))], 0.0);
        assert!(hollow.world.is_empty());
        let red = covered(&in_color(&hollow.screen, HOSTILE));
        let len = Tuning::DEFAULT.indicator_arrow_pt * HOLLOW_SCALE;
        let (bx, by) = block_at(behind(edge.at, LEFT, len * 0.5));
        let inside: Vec<(i32, i32)> = arrow_cells(LEFT, len).iter().map(|&(i, j)| (bx + i, by + j)).collect();
        assert!(!red.is_empty() && inside.iter().all(|c| !red.contains(c)), "hollow: the arrow's own blocks left open");
    }

    /// A screen two seats share shows the first seat's arrows, every
    /// seat's hit arc, the teammates no arrow points at yet, and each seat's
    /// lane warnings - on the arrow already pointing at that enemy, the
    /// more urgent kept, or as an arrow of their own.
    #[test]
    fn a_shared_screen_keeps_the_first_seats_arrows_and_adds_each_seats_own() {
        let warn = |settle: f32, flash: f32| Some(LaneWarning { settle, flash });
        let at = |y: f32| Vec2::new(390.0, y);
        let mut first = shown(vec![
            arrow(ArrowKind::Teammate { seat: 1, entering: false }, Vec2::new(10.0, 150.0), Edge::Left, LEFT),
            arrow(ArrowKind::Enemy { count: 1, warning: None }, at(100.0), Edge::Right, RIGHT),
            arrow(ArrowKind::Enemy { count: 1, warning: warn(1.0, 0.0) }, at(200.0), Edge::Right, RIGHT),
        ]);
        first.hit_arc = Some(HitArc { dir: UP, fade: 1.0, at: Vec2::new(100.0, 100.0) });
        let mut second = shown(vec![
            arrow(ArrowKind::Teammate { seat: 0, entering: false }, Vec2::new(200.0, 10.0), Edge::Top, UP),
            arrow(ArrowKind::Enemy { count: 1, warning: warn(0.5, 0.0) }, at(101.0), Edge::Right, RIGHT),
            arrow(ArrowKind::Enemy { count: 1, warning: warn(0.2, 0.0) }, at(201.0), Edge::Right, RIGHT),
            arrow(ArrowKind::Enemy { count: 1, warning: warn(0.0, 0.0) }, at(250.0), Edge::Right, RIGHT),
            arrow(ArrowKind::Enemy { count: 1, warning: None }, at(30.0), Edge::Right, RIGHT),
        ]);
        // The second seat sees the first's two enemies from elsewhere: the
        // same targets, other edge points.
        second.arrows[1].target = first.arrows[1].target;
        second.arrows[2].target = first.arrows[2].target;
        second.hit_arc = Some(HitArc { dir: RIGHT, fade: 1.0, at: Vec2::new(300.0, 200.0) });
        let arrows = shared_arrows(&first, std::slice::from_ref(&second));
        let kinds: Vec<ArrowKind> = arrows.iter().map(|a| a.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ArrowKind::Teammate { seat: 1, entering: false },
                ArrowKind::Enemy { count: 1, warning: warn(0.5, 0.0) },
                ArrowKind::Enemy { count: 1, warning: warn(1.0, 0.0) },
                ArrowKind::Teammate { seat: 0, entering: false },
                ArrowKind::Enemy { count: 1, warning: warn(0.0, 0.0) },
            ],
            "the second seat's warning on the first's arrow, the more urgent kept, its own threat added, its plain enemy not"
        );
        assert_eq!(arrows[1].place, first.arrows[1].place, "the first seat's arrow, where it put it");
        let p = draw(&[first, second], 0.0);
        let arcs = covered(&in_color(&p.world, WHITE));
        assert!(arcs.iter().any(|&(x, _)| x < 75) && arcs.iter().any(|&(x, _)| x > 150), "both seats' arcs");
    }

    /// The sizes the rows give in points become the bitmap's pixels at the
    /// scale the window shows it, and the labels a whole multiple of the
    /// font's own size at least eight points tall.
    #[test]
    fn sizes_in_points_follow_the_scale_the_bitmap_is_shown_at() {
        let t = Tuning::DEFAULT;
        let big = in_points(&t, 2.0);
        assert_eq!((big.indicator_inset_pt, big.indicator_arrow_pt, big.indicator_cluster_pt), (20.0, 32.0, 44.0));
        assert_eq!(big.indicator_far_scale, t.indicator_far_scale, "everything else as it was");
        assert_eq!(in_points(&t, f32::NAN), t);
        assert_eq!(in_points(&t, 0.0), t);
        assert_eq!(label_font(1.0), 10, "the bitmap at its own size: the font's own");
        assert_eq!(label_font(0.8), 10, "a bitmap shown larger");
        assert_eq!(label_font(1.28), 20, "a field map's bitmap in a smaller window");
        assert_eq!(label_font(1.87), 20, "a phone");
        assert_eq!(label_font(3.0), 30);
        assert_eq!(label_font(f32::INFINITY), 10);
    }

    /// The view a camera draws onto the field area of the bitmap: the
    /// world rectangle it shows, at its scale, from the field area's
    /// corner, the arrows 10 pt inside it.
    #[test]
    fn a_cameras_view_frame_is_the_field_area_it_draws_onto() {
        let t = Tuning::DEFAULT;
        let field = Rectangle::new(0.0, 32.0, 1088.0, 544.0);
        let camera = Camera::zoomed((1088.0, 544.0), Vec2::new(544.0, 272.0), 2.0);
        let view = ViewFrame::of_camera(&camera, field, &t);
        assert_eq!((view.world, view.scale, view.origin), (Rectangle::new(272.0, 136.0, 544.0, 272.0), 2.0, Vec2::new(0.0, 32.0)));
        assert_eq!(view.inset, Rectangle::new(10.0, 42.0, 1068.0, 524.0));
        assert_eq!(view.to_screen(Vec2::new(272.0, 136.0)), Vec2::new(0.0, 32.0));
        assert_eq!(view.to_screen(Vec2::new(544.0, 272.0)), Vec2::new(544.0, 304.0), "the view's middle the field area's");
        assert!(view.keep_out.is_empty(), "the caller adds the HUD's clusters and the thumbs");
        // A view whose texels overrun the field area keeps its arrows on it.
        let third = Camera::zoomed((1088.0, 544.0), Vec2::new(544.0, 272.0), 3.0);
        assert_eq!(ViewFrame::of_camera(&third, field, &t).inset, view.inset);
        let whole = ViewFrame::of_camera(&Camera::whole((1088.0, 544.0)), field, &t);
        assert_eq!((whole.world, whole.scale), (Rectangle::new(0.0, 0.0, 1088.0, 544.0), 1.0));
    }

    /// The thumbs rest on pads in from the screen's bottom corners, which
    /// an arrow slides off.
    #[test]
    fn thumb_rests_sit_in_from_the_bottom_corners_and_push_arrows_along() {
        let t = Tuning::DEFAULT;
        let glass = Rectangle::new(0.0, 0.0, 852.0, 393.0);
        let [left, right] = thumb_rests(glass, 1.0, &t);
        let mm = POINTS_PER_MM;
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert!(close(left.width, 12.0 * mm) && close(left.height, 12.0 * mm));
        assert!(close(left.x + left.width * 0.5, 22.0 * mm) && close(right.x + right.width * 0.5, 852.0 - 22.0 * mm));
        assert!(close(left.y + left.height * 0.5, 393.0 - 18.0 * mm) && close(right.y, left.y));
        let [twice, _] = thumb_rests(glass, 2.0, &t);
        assert!(close(twice.width, 2.0 * left.width), "in the arrows' units, two to the point");
        // A bitmap letterboxed 120 pt in from the screen's left edge: the
        // left pad lies over its inset's left side, and an arrow cast there
        // slides clear of it.
        let safe = Rectangle::new(120.0, 0.0, 612.0, 393.0);
        let mut view = ViewFrame::new(Rectangle::new(0.0, 0.0, 612.0, 393.0), 1.0, Vec2::new(120.0, 0.0), safe, &t);
        let inset_left = view.inset.x;
        assert!(left.x < inset_left && inset_left < left.x + left.width, "the pad reaches the inset");
        // The seat in the middle of the bitmap, an enemy far to its left on
        // the line from the seat through the pad's middle on the inset's
        // left side (the world is the screen 120 pt to the left).
        let seat = Position::new(306.0, 196.0);
        let (side_x, pad_y, far_x) = (inset_left - 120.0, left.y + left.height * 0.5, -400.0);
        let lost = Position::new(far_x, seat.y + (pad_y - seat.y) * (seat.x - far_x) / (seat.x - side_x));
        let scene = Scene {
            seat: Some(SeatView { slot: 0, pos: seat, wreck: false, gate: None }),
            tanks: vec![TankView { slot: 5, seat: None, pos: lost, wreck: false, gate: None, in_grass: false, lane: false }],
            ..Scene::default()
        };
        let free = Awareness::new().frame(&scene, &view, &t);
        let y = free.arrows[0].place.at.y;
        assert!(y > left.y && y < left.y + left.height, "without the pads the arrow lands on the thumb: {y}");
        view.keep_out.extend(thumb_rests(glass, 1.0, &t));
        let slid = Awareness::new().frame(&scene, &view, &t).arrows[0].place;
        let margin = t.indicator_arrow_pt * 0.5;
        assert_eq!(slid.edge, Edge::Left);
        assert!(slid.at.y <= left.y - margin + 1e-3 || slid.at.y >= left.y + left.height + margin - 1e-3, "slid clear: {slid:?}");
    }

    /// The picture is a function of what it is given: the same indicators
    /// at the same time draw the same blocks, and nothing draws nothing.
    #[test]
    fn the_same_indicators_draw_the_same_picture() {
        let mut ind = shown(vec![
            arrow(ArrowKind::Enemy { count: 2, warning: Some(LaneWarning { settle: 0.6, flash: 0.0 }) }, Vec2::new(390.0, 150.0), Edge::Right, RIGHT),
            arrow(ArrowKind::Frog { side: Side::Player }, Vec2::new(10.0, 290.0), Edge::Left, Vec2::new(-0.7071, 0.7071)),
        ]);
        ind.hit_arc = Some(HitArc { dir: LEFT, fade: 0.7, at: Vec2::new(200.0, 150.0) });
        assert_eq!(draw(std::slice::from_ref(&ind), 1.3), draw(std::slice::from_ref(&ind), 1.3));
        assert!(draw(&[], 0.0).is_empty());
        assert!(draw(&[Indicators::default()], 0.0).is_empty());
    }
}
