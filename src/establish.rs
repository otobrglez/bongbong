//! The establishing shot (docs/large-maps-follow-camera.md section 6): a
//! field map's round opens on the whole map for about a second - the frog,
//! the gates, the lie of the land - then zooms fast down to the tank's
//! follow view, or cuts to it under reduced motion (`motion.rs`).
//!
//! It is tied to the round's opening, where the mission banner holds the
//! round still (`Game::intro_timer`), so it never costs a player control
//! or a sight of the fight: the shot fits inside the banner, and the frame
//! a steer or a shot ends the banner ends the shot with it. A round with no
//! banner - the dev server's `restart {intro: false}`, a room's round - has
//! no shot, and an arena, shown whole already, never plays one.
//!
//! Presentation only and headless, the `follow.rs` pattern: it reads the
//! round and never writes it, and `app.rs` draws what it says. Two layers,
//! each testable on its own: `Establish::update` is the timeline, on the
//! banner's own clock, so a lockstep `step` plays it frame for frame; and
//! `Mapping`/`between` are the zoom, the world's place on the window moved
//! from the whole map's to the follow view's about the one world point both
//! put at the same place, so the picture zooms straight into it. A couch
//! pair whose round opens apart zooms into the split screen (`zoom_split`):
//! each half zooms into its own follow view, the divider drawn in as the
//! two pictures part.

use crate::math::{Rectangle, Vec2};
use crate::tuning::{tuning, Tuning};
use crate::view::{Camera, View};

/// The largest side, in texels, of the whole-field target the shot is
/// drawn through - a texel per world pixel, the arenas' path. A field
/// past it - 128 cells a side; every shipped map, the 96 x 54 study map
/// included, is well inside - opens on the follow view instead.
pub const MAX_TEXELS: f32 = 4096.0;

/// The `camera` group's establishing-shot rows, read once a frame
/// (`EstablishRules::current`) and passed in, so a test states the rules it
/// runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EstablishRules {
    /// `camera_establish_hold_seconds`: how long the whole map shows; 0
    /// plays no shot.
    pub hold_seconds: f32,
    /// `camera_establish_zoom_seconds`: how long the zoom down to the tank
    /// takes.
    pub zoom_seconds: f32,
}

impl EstablishRules {
    /// The rules in the tuning table this frame.
    pub fn current() -> EstablishRules {
        EstablishRules::of(&tuning())
    }

    /// The rules in `t`.
    pub fn of(t: &Tuning) -> EstablishRules {
        EstablishRules { hold_seconds: t.camera_establish_hold_seconds, zoom_seconds: t.camera_establish_zoom_seconds }
    }

    /// Whether a shot plays at all.
    pub fn plays(&self) -> bool {
        self.hold_seconds > 0.0
    }
}

/// The most texels of whole field a phone or a tablet (`crate::EMBEDDED`)
/// draws the shot through: its targets - two, five under a sky, each with
/// a depth buffer, about 8 bytes a texel - stay under about 170 MB, which
/// longwater (80 x 45 cells, 3.7 million texels) fits and a larger field
/// would multiply on a device with a phone's memory.
pub const EMBEDDED_MAX_AREA: f32 = 2048.0 * 2048.0;

/// Whether a field of `field` world pixels fits the shot's whole-field
/// target on a build that is (`embedded`) or is not a phone's or a
/// tablet's: `MAX_TEXELS` a side, and on a phone or a tablet
/// `EMBEDDED_MAX_AREA` in all.
pub fn fits(field: (f32, f32), embedded: bool) -> bool {
    let (w, h) = (field.0.ceil(), field.1.ceil());
    w <= MAX_TEXELS && h <= MAX_TEXELS && (!embedded || w * h <= EMBEDDED_MAX_AREA)
}

/// Whether a screen plays the shot for a round on a field of `field` world
/// pixels: a local round's (`local`), on a followed field map whose view
/// shows part of it (`shows_part`), small enough for the whole-field
/// target on this build (`fits`), with the rows asking for a shot.
pub fn wanted(local: bool, shows_part: bool, field: (f32, f32), rules: &EstablishRules) -> bool {
    local && shows_part && fits(field, crate::EMBEDDED) && rules.plays()
}

/// What the screen shows this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// The whole map.
    Whole,
    /// The zoom from the whole map down to the follow view, eased, from 0
    /// (the whole map) to 1 (the follow view).
    Zoom(f32),
    /// The follow view: the shot is over, was cut short, or never played.
    Follow,
}

impl Phase {
    /// Whether the screen shows the shot rather than the follow view.
    pub fn showing(self) -> bool {
        self != Phase::Follow
    }

    /// The spelling `status.camera.establishing` uses.
    pub fn name(self) -> &'static str {
        match self {
            Phase::Whole => "whole",
            Phase::Zoom(_) => "zoom",
            Phase::Follow => "follow",
        }
    }

    /// How far the zoom has come, 0 on the whole map and 1 on the follow
    /// view.
    pub fn progress(self) -> f32 {
        match self {
            Phase::Whole => 0.0,
            Phase::Zoom(q) => q,
            Phase::Follow => 1.0,
        }
    }
}

/// Where the shot stands `elapsed` seconds into an opening banner of
/// `total` seconds: the whole map for `camera_establish_hold_seconds`, the
/// zoom for `camera_establish_zoom_seconds`, then the follow view - the
/// hold shortened first, then the zoom, so the shot ends no later than the
/// banner does. Under reduced motion the whole map cuts straight to the
/// follow view where the zoom would start.
pub fn phase_at(elapsed: f32, total: f32, rules: &EstablishRules, reduced: bool) -> Phase {
    if !rules.plays() || !(total > 0.0) {
        return Phase::Follow;
    }
    let zoom = rules.zoom_seconds.max(0.0).min(total);
    let hold = rules.hold_seconds.min(total - zoom).max(0.0);
    let elapsed = if elapsed.is_finite() { elapsed.max(0.0) } else { 0.0 };
    if elapsed < hold {
        Phase::Whole
    } else if reduced || elapsed >= hold + zoom {
        Phase::Follow
    } else {
        Phase::Zoom(ease((elapsed - hold) / zoom))
    }
}

/// `p` from 0 to 1 eased in and out (smoothstep): the zoom leaves the
/// whole map gently and settles onto the follow view the same way.
fn ease(p: f32) -> f32 {
    let p = p.clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

/// The shot's memory from one frame to the next: which round it read and
/// the opening it plays over.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Establish {
    /// The round it read last: its seed and the simulation frame it stood
    /// at.
    round: Option<(u64, u64)>,
    /// The opening banner's seconds when the round was first read, while
    /// the shot still has to play out; `None` once it is over, and for a
    /// round that opened with no banner or on no shot.
    total: Option<f32>,
    /// What the last frame showed.
    last: Option<Phase>,
}

impl Establish {
    /// This frame's phase for the round of seed `seed` standing at
    /// simulation frame `frame`, its opening banner holding the round still
    /// for `intro_left` more seconds (`Game::intro_timer`), on a screen that
    /// plays a shot when `wanted` says so (`wanted`). A new round - another
    /// seed, a frame counter that went back, or a round at its frame 0 -
    /// arms the shot when it opens behind a banner, and the shot is over
    /// for that round the frame the banner is: run out, or ended by a seat
    /// steering or firing.
    pub fn update(&mut self, seed: u64, frame: u64, intro_left: f32, wanted: bool, rules: &EstablishRules, reduced: bool) -> Phase {
        let new_round = match self.round {
            Some((s, f)) => s != seed || frame < f || frame == 0,
            None => true,
        };
        self.round = Some((seed, frame));
        if new_round {
            self.total = (wanted && intro_left > 0.0 && rules.plays()).then_some(intro_left);
        }
        let phase = match self.total {
            Some(total) if wanted && intro_left > 0.0 => phase_at(total - intro_left, total, rules, reduced),
            _ => Phase::Follow,
        };
        if phase == Phase::Follow {
            self.total = None;
        }
        self.last = Some(phase);
        phase
    }

    /// Whether the last frame showed the shot: what the next frame's render
    /// targets are made for (`app.rs`'s `Presentation`).
    pub fn showing(&self) -> bool {
        self.last.is_some_and(Phase::showing)
    }
}

/// How the world lands on the window: a world point `p` at
/// `origin + p * scale`, in the window's units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mapping {
    pub scale: f32,
    pub origin: Vec2,
}

impl Mapping {
    /// The mapping a frame presents: `camera`'s view in the field area of a
    /// bitmap at `field_origin`, the bitmap on the window through `view`.
    pub fn of(camera: &Camera, field_origin: Vec2, view: &View) -> Mapping {
        let corner = camera.rect();
        let scale = camera.scale * view.scale;
        let origin = Vec2::new(
            view.offset.x + (field_origin.x - corner.x * camera.scale) * view.scale,
            view.offset.y + (field_origin.y - corner.y * camera.scale) * view.scale,
        );
        Mapping { scale, origin }
    }

    /// Where a world point lands on the window.
    pub fn to_window(&self, world: Vec2) -> Vec2 {
        Vec2::new(self.origin.x + world.x * self.scale, self.origin.y + world.y * self.scale)
    }

    /// The world rectangle a window of `window` shows through it.
    pub fn shows(&self, window: (f32, f32)) -> Rectangle {
        let s = if self.scale > 0.0 { self.scale } else { 1.0 };
        Rectangle::new(-self.origin.x / s, -self.origin.y / s, window.0 / s, window.1 / s)
    }

    /// The `View` that puts a bitmap of the whole field of `field` world
    /// pixels - a pixel a world pixel, `Camera::whole` - on a window of
    /// `window` this way.
    pub fn view(&self, field: (f32, f32), window: (f32, f32)) -> View {
        View { bitmap: field, window, scale: self.scale, offset: self.origin }
    }
}

/// The zoom `q` of the way from `from` to `to` (0 is `from`, 1 is `to`):
/// the scale moving at a steady ratio, about the world point both mappings
/// put at the same place on the window, which stays there - a straight
/// zoom into it, nothing sliding sideways. Mappings of one scale slide
/// from one to the other instead.
pub fn between(from: Mapping, to: Mapping, q: f32) -> Mapping {
    let q = if q.is_finite() { q.clamp(0.0, 1.0) } else { 0.0 };
    if q <= 0.0 {
        return from;
    }
    if q >= 1.0 {
        return to;
    }
    let (s0, s1) = (from.scale, to.scale);
    if !(s0 > 0.0 && s1 > 0.0) || (s1 - s0).abs() <= 1e-6 * s0.max(s1) {
        let origin = Vec2::new(from.origin.x + (to.origin.x - from.origin.x) * q, from.origin.y + (to.origin.y - from.origin.y) * q);
        return Mapping { scale: s0 + (s1 - s0) * q, origin };
    }
    let scale = s0 * (s1 / s0).powf(q);
    // The fixed point: from.origin + f * s0 == to.origin + f * s1.
    let f = Vec2::new((to.origin.x - from.origin.x) / (s0 - s1), (to.origin.y - from.origin.y) / (s0 - s1));
    let at = from.to_window(f);
    Mapping { scale, origin: Vec2::new(at.x - f.x * scale, at.y - f.y * scale) }
}

/// A couch's split screen (`follow::Split`) as the zoom opens it: the
/// halves' two mappings this frame, the divider between them on the window
/// and how far it is drawn in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomSplit {
    /// A point on the divider, window units ...
    pub at: Vec2,
    /// ... and the unit normal into the second half.
    pub normal: Vec2,
    /// How far the divider is drawn in, 0 to 1: the zoom's progress, so it
    /// comes in from nothing on the whole map, where the two halves are one
    /// picture.
    pub alpha: f32,
}

/// The split `q` of the way through the zoom (`Phase::Zoom`), the first
/// half's mapping `first` and the second's `second` this frame (each
/// `between` the whole map's and its own follow view's), the couch's seats
/// at `seats` (world px). The divider is the perpendicular bisector of the
/// two seats as the two mappings put them on the window - the follow
/// split's own rule, so it lands on the follow split's line as the zoom
/// does, and each seat stands on its own side of it all the way down.
pub fn zoom_split(first: Mapping, second: Mapping, seats: (Vec2, Vec2), q: f32) -> ZoomSplit {
    let (a, b) = (first.to_window(seats.0), second.to_window(seats.1));
    let d = b - a;
    let len = d.length();
    let normal = if len > 1e-4 { d * (1.0 / len) } else { Vec2::new(1.0, 0.0) };
    let q = if q.is_finite() { q.clamp(0.0, 1.0) } else { 0.0 };
    ZoomSplit { at: (a + b) * 0.5, normal, alpha: q }
}

#[cfg(test)]
mod establish_tests {
    use super::*;

    fn rules() -> EstablishRules {
        EstablishRules::of(&Tuning::DEFAULT)
    }

    const DT: f32 = 1.0 / 60.0;

    /// A round's frames from its opening: the banner counting down from
    /// `banner` seconds, ended early at frame `cut` when one is given.
    fn play(banner: f32, frames: usize, cut: Option<usize>, wanted: bool, reduced: bool) -> Vec<Phase> {
        let mut e = Establish::default();
        let mut left = banner;
        (1..=frames as u64)
            .map(|frame| {
                left = if cut.is_some_and(|c| frame as usize >= c) { 0.0 } else { (left - DT).max(0.0) };
                e.update(7, frame, left, wanted, &rules(), reduced)
            })
            .collect()
    }

    #[test]
    fn the_rows_come_from_the_camera_group() {
        for name in ["camera_establish_hold_seconds", "camera_establish_zoom_seconds"] {
            assert_eq!(Tuning::meta(name).map(|m| m.group), Some("camera"), "{name}");
        }
        let r = rules();
        assert!((0.8..=1.2).contains(&r.hold_seconds), "about a second: {r:?}");
        assert!(r.zoom_seconds > 0.0 && r.hold_seconds + r.zoom_seconds < Tuning::DEFAULT.mission_banner_seconds, "{r:?}");
    }

    #[test]
    fn a_round_with_an_intro_opens_on_the_whole_map_then_zooms_to_the_tank() {
        let r = rules();
        let frames = play(Tuning::DEFAULT.mission_banner_seconds, 150, None, true, false);
        let whole = frames.iter().take_while(|p| **p == Phase::Whole).count();
        assert!((whole as f32 * DT - r.hold_seconds).abs() <= 2.0 * DT, "the whole map for the hold: {whole} frames");
        let zoom: Vec<f32> = frames[whole..].iter().map_while(|p| match p {
            Phase::Zoom(q) => Some(*q),
            _ => None,
        }).collect();
        assert!((zoom.len() as f32 * DT - r.zoom_seconds).abs() <= 2.0 * DT, "the zoom for its seconds: {}", zoom.len());
        assert!(zoom.windows(2).all(|w| w[1] > w[0]), "the zoom only moves on: {zoom:?}");
        assert!(zoom[0] < 0.05 && *zoom.last().unwrap() > 0.95, "{zoom:?}");
        // Then the follow view, for good, while the banner still holds the
        // round still - the shot never outlasts it.
        let rest = &frames[whole + zoom.len()..];
        assert!(!rest.is_empty() && rest.iter().all(|p| *p == Phase::Follow));
        assert!((whole + zoom.len()) as f32 * DT < Tuning::DEFAULT.mission_banner_seconds);
    }

    #[test]
    fn a_round_with_no_intro_has_no_shot() {
        let frames = play(0.0, 60, None, true, false);
        assert!(frames.iter().all(|p| *p == Phase::Follow), "{frames:?}");
        // Nor where the screen does not play one: an arena, a room, a map
        // past the target's size.
        let frames = play(2.0, 60, None, false, false);
        assert!(frames.iter().all(|p| *p == Phase::Follow), "{frames:?}");
        let r = rules();
        assert!(wanted(true, true, (2560.0, 1440.0), &r), "longwater");
        assert!(wanted(true, true, (3072.0, 1728.0), &r), "the 96 x 54 study map");
        assert!(!wanted(false, true, (2560.0, 1440.0), &r), "a room's round");
        assert!(!wanted(true, false, (1088.0, 544.0), &r), "a view that shows the whole field");
        assert!(!wanted(true, true, (8000.0, 2000.0), &r), "past the whole-field target");
        assert!(!wanted(true, true, (2560.0, 1440.0), &EstablishRules { hold_seconds: 0.0, ..r }), "turned off");
        // A phone or a tablet keeps its targets to a phone's memory:
        // longwater plays the shot there, the study map opens on the
        // follow view.
        assert!(fits((2560.0, 1440.0), true), "longwater on a phone");
        assert!(!fits((3072.0, 1728.0), true), "the study map on a phone");
        assert!(fits((3072.0, 1728.0), false), "the study map on a desktop");
        assert!(!fits((4100.0, 100.0), false), "past a side");
    }

    #[test]
    fn a_steer_or_a_shot_cuts_the_shot_short() {
        // The banner ended by a seat moving: the follow view that frame,
        // and the shot does not come back with the next frames.
        for cut in [10, 70, 85] {
            let frames = play(2.0, 140, Some(cut), true, false);
            assert!(frames[..cut - 1].iter().all(|p| p.showing()), "cut {cut}");
            assert!(frames[cut - 1..].iter().all(|p| *p == Phase::Follow), "cut {cut}: {:?}", &frames[cut - 1..cut + 2]);
        }
        // The shot is over for the round: a banner that somehow came back
        // without a new round shows nothing.
        let mut e = Establish::default();
        assert_eq!(e.update(3, 1, 2.0, true, &rules(), false), Phase::Whole);
        assert_eq!(e.update(3, 2, 0.0, true, &rules(), false), Phase::Follow);
        assert_eq!(e.update(3, 3, 1.5, true, &rules(), false), Phase::Follow);
        // A new round - another seed, or the frame counter gone back - plays
        // it again.
        assert_eq!(e.update(4, 4, 2.0, true, &rules(), false), Phase::Whole);
        e.update(4, 5, 0.0, true, &rules(), false);
        assert_eq!(e.update(4, 1, 2.0, true, &rules(), false), Phase::Whole, "a restart on a pinned seed");
    }

    #[test]
    fn reduced_motion_cuts_instead_of_zooming() {
        let r = rules();
        let frames = play(2.0, 150, None, true, true);
        let whole = frames.iter().take_while(|p| **p == Phase::Whole).count();
        assert!((whole as f32 * DT - r.hold_seconds).abs() <= 2.0 * DT, "the whole map holds as long: {whole}");
        assert!(frames[whole..].iter().all(|p| *p == Phase::Follow), "then a cut, no zoom");
    }

    #[test]
    fn a_short_banner_shortens_the_hold_first() {
        let r = EstablishRules { hold_seconds: 1.0, zoom_seconds: 0.5 };
        assert_eq!(phase_at(0.2, 1.0, &r, false), Phase::Whole);
        assert!(matches!(phase_at(0.6, 1.0, &r, false), Phase::Zoom(_)), "the zoom ends with the banner");
        assert_eq!(phase_at(1.0, 1.0, &r, false), Phase::Follow);
        // Shorter than the zoom: no hold, and the zoom fills the banner.
        assert!(matches!(phase_at(0.0, 0.3, &r, false), Phase::Zoom(q) if q == 0.0));
        assert_eq!(phase_at(0.0, 0.0, &r, false), Phase::Follow);
    }

    /// A couch pair whose round opens apart: each half zooms from the
    /// whole map into its own follow view, the two one picture at the
    /// start, the divider between them always the bisector of where the
    /// two halves show their seats - each seat on its own side - and,
    /// once the zoom lands, the follow split's own line on the window.
    #[test]
    fn a_couch_pair_apart_zooms_into_the_split_screen() {
        use crate::follow::{Follow, FollowRules, SeatTank, Seats, Stage};
        use crate::framing::SightBox;
        let field = (2560.0, 1440.0);
        let window = (1600.0, 900.0);
        let visible = (1066.0, 600.0);
        let stage = Stage { visible, field, sight: SightBox::from_cells(11.5, 7.5) };
        let tank = |seat: usize, x: f32, y: f32| SeatTank {
            seat,
            position: Vec2::new(x, y),
            facing: Vec2::new(1.0, 0.0),
            velocity: Vec2::new(0.0, 0.0),
            top_speed: 210.0,
            live: true,
        };
        let seats = Seats { local: vec![0, 1], tanks: vec![tank(0, 300.0, 300.0), tank(1, 2200.0, 1100.0)] };
        let shot = Follow::default().update(&seats, 1.0 / 60.0, &stage, &FollowRules::of(&Tuning::DEFAULT));
        let split = shot.split.expect("too far apart for one view");
        let view = View::fill(visible, window);
        let first_camera = Camera::following(field, shot.corner, visible, 1.0, 1.5);
        let second_camera = Camera::following(field, split.corner, visible, 1.0, 1.5);
        let whole = Mapping::of(&Camera::whole(field), Vec2::new(0.0, 0.0), &View::fit(field, window));
        let first = Mapping::of(&first_camera, Vec2::new(0.0, 0.0), &view);
        let second = Mapping::of(&second_camera, Vec2::new(0.0, 0.0), &view);
        let (a, b) = (seats.tanks[0].position, seats.tanks[1].position);
        // On the whole map the two halves are one picture, and no line.
        let open = zoom_split(between(whole, first, 0.0), between(whole, second, 0.0), (a, b), 0.0);
        assert_eq!(between(whole, first, 0.0), between(whole, second, 0.0));
        assert_eq!(open.alpha, 0.0);
        let side = |z: &ZoomSplit, p: Vec2| (p.x - z.at.x) * z.normal.x + (p.y - z.at.y) * z.normal.y;
        for k in 0..=20 {
            let q = k as f32 / 20.0;
            let (m1, m2) = (between(whole, first, q), between(whole, second, q));
            let z = zoom_split(m1, m2, (a, b), q);
            assert!(side(&z, m1.to_window(a)) < 0.0 && side(&z, m2.to_window(b)) > 0.0, "{q}: {z:?}");
            assert!((z.alpha - q).abs() < 1e-6);
        }
        // Landed: the follow split's line, put on the window.
        let landed = zoom_split(first, second, (a, b), 1.0);
        let at = view.to_window(split.at);
        assert!((landed.at - at).length() < 1.0, "{landed:?} vs {at:?}");
        assert!((landed.normal - split.normal).length() < 1e-2, "{landed:?} vs {split:?}");
        assert_eq!(landed.alpha, 1.0);
    }

    #[test]
    fn the_zoom_runs_from_the_whole_map_to_the_follow_view_about_one_point() {
        // The whole of longwater fitted to 1600 x 900, then the follow view
        // at 1.5 window units per world pixel round a tank at (1216, 1071).
        let field = (2560.0, 1440.0);
        let window = (1600.0, 900.0);
        let whole = Mapping::of(&Camera::whole(field), Vec2::new(0.0, 0.0), &View::fit(field, window));
        assert_eq!(whole.scale, 0.625);
        let follow_camera = Camera::following(field, Vec2::new(683.0, 771.0), (1066.7, 600.0), 1.0, 1.5);
        let follow = Mapping::of(&follow_camera, Vec2::new(0.0, 0.0), &View::fill((1066.7, 600.0), window));
        let corner = follow.to_window(Vec2::new(follow_camera.rect().x, follow_camera.rect().y));
        assert!(corner.x.abs() < 0.01 && corner.y.abs() < 0.01, "the follow view's corner at the window's: {corner:?}");
        assert_eq!(between(whole, follow, 0.0), whole);
        assert_eq!(between(whole, follow, 1.0), follow);
        // Every step in between is a zoom about one world point, which
        // stays where both ends put it, and the scale only grows.
        let f = Vec2::new((follow.origin.x - whole.origin.x) / (whole.scale - follow.scale), (follow.origin.y - whole.origin.y) / (whole.scale - follow.scale));
        let fixed = whole.to_window(f);
        let mut last = whole.scale;
        for k in 1..20 {
            let m = between(whole, follow, k as f32 / 20.0);
            assert!(m.scale > last, "{k}: {m:?}");
            last = m.scale;
            let at = m.to_window(f);
            assert!((at.x - fixed.x).abs() < 0.01 && (at.y - fixed.y).abs() < 0.01, "{k}: {at:?} vs {fixed:?}");
        }
        // The rectangle a mapping shows, and the view that presents it.
        let shown = whole.shows(window);
        assert!((shown.width - 2560.0).abs() < 0.01 && (shown.height - 1440.0).abs() < 0.01, "{shown:?}");
        let v = whole.view(field, window);
        assert_eq!(v.to_window(Vec2::new(100.0, 50.0)), whole.to_window(Vec2::new(100.0, 50.0)));
        // One scale at both ends: a slide.
        let a = Mapping { scale: 1.5, origin: Vec2::new(0.0, 0.0) };
        let b = Mapping { scale: 1.5, origin: Vec2::new(-300.0, 60.0) };
        assert_eq!(between(a, b, 0.5), Mapping { scale: 1.5, origin: Vec2::new(-150.0, 30.0) });
    }
}
