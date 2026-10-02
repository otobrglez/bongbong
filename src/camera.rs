//! The follow camera for field maps - maps larger than an arena
//! (`MapFile::follows`, docs/large-maps-follow-camera.md). Presentation
//! only: nothing in `simulation/` sees it, and a local round and a room's
//! replica go through the same code.
//!
//! Two halves. `viewport` decides how much world a screen shows - the
//! same area on every device (`camera_view_area_cells`), the window's
//! shape deciding only the outline, clamped between `ASPECT_MIN` and
//! `ASPECT_MAX` - and at what scale, snapped to whole art blocks on
//! displays coarse enough for an uneven block to show. `FollowCamera`
//! decides where that window sits: on the tank, inside a small dead zone,
//! leading in the direction it faces by no more than the room outside the
//! sight box, eased by a critically damped spring and clamped to the
//! field. It hands back whole field pixels, so the art never lands between
//! texels; `render::game` crops `scene_target` to that rectangle.

use crate::math::Vec2;
use crate::simulation::{with_frog, with_tank, Game};
use crate::tuning::{tuning, Tuning};
use crate::{Rect, OBSTACLE_GRID_SIZE, TANK_SPRITE_SIZE};

/// How much of its half-width a view that cannot hold the sight box (a
/// zoomed phone) may lead the tank by.
const LEAD_SHARE_ZOOMED: f32 = 0.3;

/// The narrowest and widest a field map's view gets. Between them every
/// screen fills edge to edge; past them (a 32:9 monitor) the view keeps
/// the widest shape and the window letterboxes the rest.
pub const ASPECT_MIN: f32 = 4.0 / 3.0;
pub const ASPECT_MAX: f32 = 2.4;

/// The smallest view the screens drawn over the field are laid out for -
/// the dialogs, the lobby, the level select and the end screen
/// (`hud::result_layout`, `lobby.rs`): the smallest shipped field. The
/// phone zoom never shows less.
pub const UI_MIN_VIEW: (f32, f32) = (768.0, 384.0);

/// How much world a screen shows and how big: `size` in field pixels,
/// whole, and `scale` in window (logical) pixels per field pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub size: (f32, f32),
    pub scale: f32,
}

/// The viewport for a window of `window` logical pixels on a display of
/// `dpr` device pixels per logical pixel, under the live tuning.
pub fn viewport(window: (f32, f32), dpr: f32) -> Viewport {
    viewport_with(window, dpr, &tuning())
}

/// `viewport` under an explicit table, for the tests.
pub fn viewport_with(window: (f32, f32), dpr: f32, t: &Tuning) -> Viewport {
    let (ww, wh) = (window.0.max(1.0), window.1.max(1.0));
    let dpr = if dpr > 0.0 { dpr } else { 1.0 };
    let aspect = (ww / wh).clamp(ASPECT_MIN, ASPECT_MAX);
    let area = t.camera_view_area_cells * OBSTACLE_GRID_SIZE * OBSTACLE_GRID_SIZE;
    let (box_w, box_h) = ((area * aspect).sqrt(), (area / aspect).sqrt());
    // A small screen zooms in until a tank is `camera_min_tank_points`
    // across, so a phone shows less than the same area rather than tanks
    // a few millimetres long - never less than `UI_MIN_VIEW`.
    let same_area = (ww / box_w).min(wh / box_h);
    let ui_cap = (ww / UI_MIN_VIEW.0).min(wh / UI_MIN_VIEW.1);
    let floor = (t.camera_min_tank_points / TANK_SPRITE_SIZE).min(ui_cap);
    let floored = floor > same_area;
    let exact = same_area.max(floor);
    let sight = (t.camera_sight_x_cells * OBSTACLE_GRID_SIZE, t.camera_sight_y_cells * OBSTACLE_GRID_SIZE);
    let shown = |scale: f32| -> (f32, f32) {
        let (w, h) = (ww / scale, wh / scale);
        (w.min(h * ASPECT_MAX), h.min(w / ASPECT_MIN))
    };
    let scale = if dpr < t.camera_snap_below_dpr {
        // Each 2 px art block a whole number of device pixels: the device
        // scale a multiple of a half. The nearer step, unless stepping in
        // would hide part of the sight box - then the step out. A zoomed
        // phone takes the step in, short of the UI's smallest view.
        let p = exact * dpr;
        let lo = ((p * 2.0).floor() / 2.0).max(0.5);
        let hi = ((p * 2.0).ceil() / 2.0).max(0.5);
        let mut pick = if (p / lo).ln().abs() <= (hi / p).ln().abs() { lo } else { hi };
        let (w, h) = shown(pick / dpr);
        if floored {
            pick = if hi / dpr <= ui_cap { hi } else { lo };
        } else if w / 2.0 < sight.0 || h / 2.0 < sight.1 {
            pick = lo;
        }
        pick / dpr
    } else {
        exact
    };
    let (w, h) = shown(scale);
    Viewport { size: (w.ceil(), h.ceil()), scale }
}

/// What the camera follows this frame: a point on the field, the way it
/// faces (a unit cardinal vector, `None` for a pair it frames between)
/// and whether it is moving.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Subject {
    pub pos: Vec2,
    pub facing: Option<Vec2>,
    pub moving: bool,
}

/// The camera's subject in `game`: the seat this window plays in a room
/// (`local_seat`), else player 1, or the midpoint of both tanks on a
/// two-player couch while both stand. A wreck is still followed - the
/// round goes on around it.
pub fn subject(game: &Game, local_seat: Option<u8>) -> Option<Subject> {
    let of = |index: usize| -> Option<(Vec2, f32, bool, bool)> {
        let entity = game.seat(index)?;
        Some(with_tank(&game.world, entity, |t| (t.position, t.rotation, t.velocity.length() > 0.0, t.is_wreck())))
    };
    if let Some(seat) = local_seat {
        if let Some((pos, rot, moving, _)) = of(seat as usize) {
            return Some(Subject { pos, facing: Some(cardinal(rot)), moving });
        }
    }
    let first = of(0);
    if game.players.count() == 2 {
        if let (Some(a), Some(b)) = (first, of(1)) {
            if !a.3 && !b.3 {
                return Some(Subject { pos: (a.0 + b.0) * 0.5, facing: None, moving: a.2 || b.2 });
            }
            let live = if a.3 { b } else { a };
            return Some(Subject { pos: live.0, facing: Some(cardinal(live.1)), moving: live.2 });
        }
    }
    first.map(|(pos, rot, moving, _)| Subject { pos, facing: Some(cardinal(rot)), moving })
}

/// A facing in degrees (0 up, clockwise) as the nearest cardinal unit
/// vector, y down.
fn cardinal(degrees: f32) -> Vec2 {
    let quarter = ((degrees / 90.0).round() as i32).rem_euclid(4);
    match quarter {
        0 => Vec2::new(0.0, -1.0),
        1 => Vec2::new(1.0, 0.0),
        2 => Vec2::new(0.0, 1.0),
        _ => Vec2::new(-1.0, 0.0),
    }
}

/// The follow camera's state between frames. `Default` is unplaced: the
/// first frame with a subject cuts straight to it.
#[derive(Clone, Copy, Debug, Default)]
pub struct FollowCamera {
    center: Vec2,
    velocity: Vec2,
    base: Vec2,
    lead: Vec2,
    lead_velocity: Vec2,
    placed: bool,
}

impl FollowCamera {
    /// Forget where it was: the next frame cuts to its subject.
    pub fn reset(&mut self) {
        self.placed = false;
    }

    /// One rendered frame, `dt` seconds after the last: the visible part
    /// of a `field`-sized battlefield for a viewport of `view` field
    /// pixels, in whole field pixels. On an axis where the view is larger
    /// than the field the rectangle is centred on it, reaching past both
    /// edges.
    pub fn frame(&mut self, subject: Option<Subject>, dt: f32, field: (f32, f32), view: (f32, f32)) -> Rect {
        self.frame_with(subject, dt, field, view, &tuning())
    }

    /// `frame` under an explicit table, for the tests.
    pub fn frame_with(&mut self, subject: Option<Subject>, dt: f32, field: (f32, f32), view: (f32, f32), t: &Tuning) -> Rect {
        let cell = OBSTACLE_GRID_SIZE;
        if let Some(s) = subject {
            // A jump further than a screen - a restart, a portal, a seat
            // coming back through a gate - is a cut, never a pan.
            if !self.placed || s.pos.distance_to(self.base) > view.0.max(view.1) {
                self.base = s.pos;
                self.lead = Vec2::zero();
                self.lead_velocity = Vec2::zero();
                self.velocity = Vec2::zero();
                self.center = clamp_center(s.pos, field, view);
                self.placed = true;
            }
            let dz = t.camera_dead_zone_cells * cell;
            self.base.x = self.base.x.clamp(s.pos.x - dz, s.pos.x + dz);
            self.base.y = self.base.y.clamp(s.pos.y - dz, s.pos.y + dz);
            // The lead spends only the room outside the sight box, so the
            // box stays on screen behind the tank as well as ahead of it.
            // A view zoomed in past the sight box (a phone) still leads,
            // by a share of itself.
            let spare = |half: f32, sight: f32| {
                if half >= sight + cell / 4.0 { half - sight - cell / 4.0 } else { half * LEAD_SHARE_ZOOMED }
            };
            let spare_x = spare(view.0 / 2.0, t.camera_sight_x_cells * cell);
            let spare_y = spare(view.1 / 2.0, t.camera_sight_y_cells * cell);
            let reach = t.camera_look_ahead_cells * cell * if s.moving { 1.0 } else { 0.35 };
            let want = match s.facing {
                Some(f) => Vec2::new(f.x * reach.min(spare_x), f.y * reach.min(spare_y)),
                None => Vec2::zero(),
            };
            let ease = t.camera_look_ease_seconds;
            (self.lead.x, self.lead_velocity.x) = smooth_damp(self.lead.x, want.x, self.lead_velocity.x, ease, dt);
            (self.lead.y, self.lead_velocity.y) = smooth_damp(self.lead.y, want.y, self.lead_velocity.y, ease, dt);
            let target = clamp_center(self.base + self.lead, field, view);
            let follow = t.camera_follow_seconds;
            (self.center.x, self.velocity.x) = smooth_damp(self.center.x, target.x, self.velocity.x, follow, dt);
            (self.center.y, self.velocity.y) = smooth_damp(self.center.y, target.y, self.velocity.y, follow, dt);
        } else if !self.placed {
            self.center = Vec2::new(field.0 / 2.0, field.1 / 2.0);
        }
        let c = clamp_center(self.center, field, view);
        Rect::new((c.x - view.0 / 2.0).round(), (c.y - view.1 / 2.0).round(), view.0, view.1)
    }
}

/// Which objective an edge marker points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Objective {
    /// The players' frog, the one a Protect round is lost with.
    Frog,
    /// The other side's frog, the one a Hunt round is won with.
    EnemyFrog,
}

/// An objective somewhere on the field: where it is, which it is, and
/// whether it is being hurt right now (the marker blinks).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub pos: Vec2,
    pub kind: Objective,
    pub hurt: bool,
}

/// The round's live frogs, for the edge markers: the players' and, in a
/// round that has one, the other side's.
pub fn objectives(game: &Game) -> Vec<Target> {
    let frog = |entity, kind| {
        with_frog(&game.world, entity, |f| (!f.is_dead()).then_some(Target { pos: f.position, kind, hurt: f.hurt_timer > 0.0 }))
    };
    let mut out = Vec::new();
    out.extend(game.frog.and_then(|e| frog(e, Objective::Frog)));
    out.extend(game.enemy_frog.and_then(|e| frog(e, Objective::EnemyFrog)));
    out
}

/// How far in from each edge of the view a marker stands, in field pixels:
/// clear of the corner HUD at the top and of the thumbs at the bottom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Insets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

/// A marker at the edge of the view for something off it: where it
/// stands in the view (view pixels from its top-left), the unit direction
/// toward the target, and how far away the target is in cells.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeMarker {
    pub at: Vec2,
    pub dir: Vec2,
    pub cells: f32,
}

/// The marker for `target` when it is off `camera` (the view's rectangle
/// of the field), `None` while it is on screen: the point where the line
/// from the middle of the view's inset rectangle toward the target leaves
/// that rectangle, so the marker sits on the edge nearest the way to go.
pub fn edge_marker(camera: Rect, target: Vec2, insets: Insets) -> Option<EdgeMarker> {
    const ON_SCREEN_MARGIN: f32 = 8.0;
    let inside = target.x >= camera.x - ON_SCREEN_MARGIN
        && target.x <= camera.x + camera.w + ON_SCREEN_MARGIN
        && target.y >= camera.y - ON_SCREEN_MARGIN
        && target.y <= camera.y + camera.h + ON_SCREEN_MARGIN;
    if inside {
        return None;
    }
    let (x0, x1) = (camera.x + insets.left, camera.x + camera.w - insets.right);
    let (y0, y1) = (camera.y + insets.top, camera.y + camera.h - insets.bottom);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let centre = Vec2::new((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let d = target - centre;
    let len = d.length();
    if len <= 0.0 {
        return None;
    }
    let tx = if d.x.abs() > 1e-6 { ((x1 - x0) / 2.0) / d.x.abs() } else { f32::INFINITY };
    let ty = if d.y.abs() > 1e-6 { ((y1 - y0) / 2.0) / d.y.abs() } else { f32::INFINITY };
    let edge = centre + d * tx.min(ty).min(1.0);
    Some(EdgeMarker {
        at: Vec2::new(edge.x - camera.x, edge.y - camera.y),
        dir: d * (1.0 / len),
        cells: len / OBSTACLE_GRID_SIZE,
    })
}

/// The nearest centre that keeps the view inside the field on each axis,
/// or the field's middle on an axis the view is larger than.
fn clamp_center(c: Vec2, field: (f32, f32), view: (f32, f32)) -> Vec2 {
    let axis = |c: f32, field: f32, view: f32| {
        if view >= field { field / 2.0 } else { c.clamp(view / 2.0, field - view / 2.0) }
    };
    Vec2::new(axis(c.x, field.0, view.0), axis(c.y, field.1, view.1))
}

/// A critically damped spring toward `target` (Game Programming Gems 4's
/// smooth damp): no overshoot, frame-rate independent, `smooth` seconds
/// to roughly close the gap. Returns the new value and velocity.
fn smooth_damp(current: f32, target: f32, velocity: f32, smooth: f32, dt: f32) -> (f32, f32) {
    if smooth <= 0.0 || dt <= 0.0 {
        return if smooth <= 0.0 { (target, 0.0) } else { (current, velocity) };
    }
    let omega = 2.0 / smooth;
    let x = omega * dt;
    let decay = 1.0 / (1.0 + x + 0.48 * x * x + 0.235 * x * x * x);
    let change = current - target;
    let temp = (velocity + omega * change) * dt;
    ((target + (change + temp) * decay), (velocity - omega * temp) * decay)
}

#[cfg(test)]
mod camera_tests {
    use super::*;

    fn t() -> Tuning {
        Tuning::DEFAULT
    }

    /// The same world area on a tablet and a monitor, within the
    /// whole-block snap's step, and the sight box on every one of them.
    #[test]
    fn every_large_screen_shows_about_the_same_area_and_the_sight_box() {
        let t = t();
        let target = t.camera_view_area_cells * 32.0 * 32.0;
        for (window, dpr) in [((1180.0, 820.0), 2.0), ((1024.0, 768.0), 2.0), ((1920.0, 1080.0), 1.0), ((3440.0, 1440.0), 1.0), ((1440.0, 900.0), 2.0)] {
            let v = viewport_with(window, dpr, &t);
            let area = v.size.0 * v.size.1;
            assert!((area / target - 1.0).abs() < 0.3, "{window:?}: {:?} is {area} of {target}", v.size);
            assert!(v.size.0 / 2.0 >= t.camera_sight_x_cells * 32.0 - 1.0, "{window:?}: {:?}", v.size);
            assert!(v.size.1 / 2.0 >= t.camera_sight_y_cells * 32.0 - 1.0, "{window:?}: {:?}", v.size);
            assert!(v.size.0 * v.scale >= window.0 - 1.0 || v.size.0 / v.size.1 >= ASPECT_MAX - 0.05, "fills the width");
        }
    }

    /// A phone zooms in until a tank is `camera_min_tank_points` across,
    /// and never shows less than the UI's smallest view; a large screen
    /// never zooms for it.
    #[test]
    fn a_phone_zooms_in_to_a_readable_tank() {
        let t = t();
        let floor = t.camera_min_tank_points / TANK_SPRITE_SIZE;
        // An iPhone 16 in landscape, in points.
        let phone = viewport_with((852.0, 393.0), 3.0, &t);
        assert!(phone.scale >= floor - 1e-4, "{phone:?}");
        assert!(phone.size.0 >= UI_MIN_VIEW.0 && phone.size.1 >= UI_MIN_VIEW.1, "{phone:?}");
        assert!(phone.size.0 * phone.size.1 < t.camera_view_area_cells * 1024.0, "less than the same area");
        // An iPhone SE: the UI's smallest view wins over the tank floor.
        let se = viewport_with((667.0, 375.0), 2.0, &t);
        assert!(se.size.0 >= UI_MIN_VIEW.0 - 1.0 && se.size.1 >= UI_MIN_VIEW.1 - 1.0, "{se:?}");
        let off = Tuning { camera_min_tank_points: 0.0, ..Tuning::DEFAULT };
        for (window, dpr) in [((1180.0, 820.0), 2.0), ((1920.0, 1080.0), 1.0)] {
            assert_eq!(viewport_with(window, dpr, &t), viewport_with(window, dpr, &off), "{window:?}");
        }
    }

    /// A coarse display snaps to whole blocks; a 3x phone keeps the exact
    /// zoom.
    #[test]
    fn coarse_displays_snap_to_whole_blocks() {
        let t = t();
        let desk = viewport_with((1920.0, 1080.0), 1.0, &t);
        assert_eq!((desk.scale * 2.0).fract(), 0.0, "1x: {}", desk.scale);
        let phone = viewport_with((852.0, 393.0), 3.0, &t);
        let exact = viewport_with((852.0, 393.0), 3.0, &Tuning { camera_snap_below_dpr: 0.0, ..Tuning::DEFAULT });
        assert_eq!(phone, exact);
    }

    /// A 32:9 monitor keeps the widest allowed shape and letterboxes.
    #[test]
    fn a_super_ultrawide_is_clamped() {
        let v = viewport_with((5120.0, 1440.0), 1.0, &t());
        assert!(v.size.0 / v.size.1 <= ASPECT_MAX + 0.01, "{:?}", v.size);
        assert!(v.size.0 * v.scale < 5120.0 - 100.0);
    }

    fn still(pos: Vec2) -> Subject {
        Subject { pos, facing: Some(Vec2::new(1.0, 0.0)), moving: false }
    }

    /// The first frame cuts to the tank; then the camera settles on it
    /// plus its lead, and a jump across the map is a cut again.
    #[test]
    fn it_cuts_then_follows_then_cuts_on_a_jump() {
        let t = t();
        let (field, view) = ((3072.0, 1728.0), (1100.0, 520.0));
        let mut cam = FollowCamera::default();
        let r = cam.frame_with(Some(still(Vec2::new(1500.0, 800.0))), 1.0 / 60.0, field, view, &t);
        assert!((r.x + r.w / 2.0 - 1500.0).abs() <= 1.0 && (r.y + r.h / 2.0 - 800.0).abs() <= 1.0, "{r:?}");
        for _ in 0..240 {
            cam.frame_with(Some(still(Vec2::new(1500.0, 800.0))), 1.0 / 60.0, field, view, &t);
        }
        let r = cam.frame_with(Some(still(Vec2::new(1500.0, 800.0))), 1.0 / 60.0, field, view, &t);
        let lead = r.x + r.w / 2.0 - 1500.0;
        assert!(lead > 10.0 && lead <= t.camera_look_ahead_cells * 32.0 * 0.35 + 1.0, "a still tank leads a little: {lead}");
        let r = cam.frame_with(Some(still(Vec2::new(300.0, 300.0))), 1.0 / 60.0, field, view, &t);
        assert_eq!((r.x, r.y), (0.0, 40.0), "cut, clamped to the field's corner: {r:?}");
    }

    /// An objective on screen has no marker; one off it has a marker on
    /// the inset edge toward it, pointing at it.
    #[test]
    fn an_off_screen_objective_is_marked_on_the_edge_toward_it() {
        let cam = Rect::new(1000.0, 500.0, 800.0, 400.0);
        let insets = Insets { top: 70.0, right: 24.0, bottom: 40.0, left: 24.0 };
        assert!(edge_marker(cam, Vec2::new(1400.0, 700.0), insets).is_none());
        let east = edge_marker(cam, Vec2::new(3000.0, 735.0), insets).expect("off to the right");
        assert_eq!(east.at.x, 800.0 - 24.0);
        assert!(east.dir.x > 0.99, "{east:?}");
        let north = edge_marker(cam, Vec2::new(1415.0, 0.0), insets).expect("above");
        assert_eq!(north.at.y, 70.0);
        assert!(north.dir.y < -0.99);
        let corner = edge_marker(cam, Vec2::new(0.0, 2000.0), insets).expect("down-left");
        assert!(corner.at.x >= 24.0 - 1e-3 && corner.at.y <= 400.0 - 40.0 + 1e-3, "{corner:?}");
        assert!((east.cells - (3000.0 - 1400.0) / 32.0).abs() < 1.0);
    }

    /// The view never shows past the field, and an axis the view is wider
    /// than is centred.
    #[test]
    fn it_stays_inside_the_field_and_centres_a_short_axis() {
        let t = t();
        let mut cam = FollowCamera::default();
        let r = cam.frame_with(Some(still(Vec2::new(3000.0, 50.0))), 1.0 / 60.0, (3072.0, 400.0), (1100.0, 520.0), &t);
        assert_eq!(r.x + r.w, 3072.0);
        assert_eq!(r.y, -60.0, "centred: {r:?}");
    }

    /// The lead never spends the sight box: behind a tank driving right
    /// the view still shows the box's half-width.
    #[test]
    fn the_lead_keeps_the_sight_box_on_screen() {
        let t = t();
        let (field, view) = ((6000.0, 3000.0), (1133.0, 522.0));
        let mut cam = FollowCamera::default();
        let s = Subject { pos: Vec2::new(3000.0, 1500.0), facing: Some(Vec2::new(0.0, -1.0)), moving: true };
        let mut r = cam.frame_with(Some(s), 1.0 / 60.0, field, view, &t);
        for _ in 0..600 {
            r = cam.frame_with(Some(s), 1.0 / 60.0, field, view, &t);
        }
        let below = r.y + r.h - s.pos.y;
        assert!(below >= t.camera_sight_y_cells * 32.0 - 1.0, "{below}");
    }
}
