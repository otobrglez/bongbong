//! The follow camera (docs/large-maps-follow-camera.md §6): where the view
//! of a field map stands, frame by frame. Presentation only and headless,
//! the `indicators.rs` pattern: it reads a round - a local one or an online
//! replica, the same code - and never writes it, draws no RNG, and nothing
//! in `simulation/` sees it. Its answer is the view's corner in world
//! pixels, which `view::Camera::following` snaps to the block grid, the
//! rest of it shifting the picture when the frame is presented.
//!
//! Two layers, each testable on its own:
//!
//! - `Seats::of` reads the round into plain values: every seat's tank -
//!   where it is, the way its hull faces, how fast it can go, whether it is
//!   in the fight - and which seats this screen plays.
//! - `Follow::update` turns those into the view, one frame at a time, on
//!   the time the round advanced that frame (a frame that ran no step
//!   moves nothing, so the view and the tanks move together):
//!   - **Who.** The screen's own seat. Two seats on one screen (couch
//!     play) share one view on their midpoint while both sight boxes fit in
//!     it; apart, the view follows seat 0 and says so (`ShotKind::Apart`) -
//!     the split screen is later work. A seat that is a wreck or on its way
//!     back through a gate keeps the view for
//!     `camera_spectate_delay_seconds`, then the nearest live teammate has
//!     it until the seat is back in the fight.
//!   - **Dead zone.** The view's anchor stays put while the seat moves
//!     within `camera_dead_zone_px` of it on an axis, so four-way
//!     corrections and slides along a wall do not wobble the view; past
//!     that the seat drags it.
//!   - **Look-ahead.** The view leads the seat the way its hull faces
//!     (`Tank::rotation`, which snaps on a turn), by
//!     `camera_lead_at_rest` of the room the sight box leaves on that axis
//!     at rest - a tank fires where it faces - and all of it at the tank's
//!     top speed. It moves at a steady pace that swings it across its room
//!     in `camera_lead_ease_seconds`. A reversal holds for
//!     `camera_lead_reverse_hold_seconds` before the lead turns round; a
//!     turn to the side starts it swinging at once.
//!   - **Spring.** The view chases the anchor plus the lead on a
//!     critically damped spring (`camera_spring_seconds`, Unity's
//!     `SmoothDamp` smoothing time), integrated exactly, so it moves the
//!     same at any frame rate, with the anchor's velocity fed forward as
//!     the goal's, so a steady drive does not trail.
//!   - **The sight box stays on screen.** Enemies fire at a seat only from
//!     inside its sight box, so the view's centre never strays further from
//!     a seat it shows than the room outside the box
//!     (`Framing::room_outside`), whatever the lead and the spring say.
//!   - **The field.** The view never shows past the field on an axis the
//!     map is longer than the view, and slows into its edge over
//!     `camera_edge_ease_px` rather than running into it; on an axis the
//!     map is shorter, the map is centred.
//!   - **Cuts, not pans**, when the followed seat goes through a portal
//!     (`Event::Teleported`) or comes in through a gate
//!     (`Event::TankEntered`), when a round starts, when the view hands
//!     over to a seat more than a screen away, and when the seat moved
//!     further in a frame than it could drive - a pan across half the map
//!     is disorienting at this pace.

use crate::framing::{Framing, Seating, SightBox};
use crate::math::{Rectangle, Vec2};
use crate::simulation::{Event, Game};
use crate::tuning::{tuning, Tuning};
use crate::view::{Camera, View};
use crate::{Layout, MAX_SEATS};

/// The `camera` group's numbers, read once a frame (`FollowRules::current`)
/// and passed in, so a test states the rules it runs and never touches the
/// global table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FollowRules {
    /// `camera_dead_zone_px`: how far the seat moves inside the view on an
    /// axis before it drags the view along.
    pub dead_zone_px: f32,
    /// `camera_lead_at_rest`: the look-ahead at rest, as a fraction of the
    /// room it may spend.
    pub lead_at_rest: f32,
    /// `camera_lead_ease_seconds`: how long the look-ahead takes to settle.
    pub lead_ease_seconds: f32,
    /// `camera_lead_reverse_hold_seconds`: how long a reversal holds before
    /// the look-ahead flips.
    pub reverse_hold_seconds: f32,
    /// `camera_spring_seconds`: the spring's smoothing time.
    pub spring_seconds: f32,
    /// `camera_spectate_delay_seconds`: how long a screen stays on its own
    /// seat's wreck before it follows a teammate.
    pub spectate_delay_seconds: f32,
    /// `camera_edge_ease_px`: how far from the field's edge the view's
    /// goal starts easing into it.
    pub edge_ease_px: f32,
}

impl FollowRules {
    /// The rules in the tuning table this frame.
    pub fn current() -> FollowRules {
        FollowRules::of(&tuning())
    }

    /// The rules in `t`.
    pub fn of(t: &Tuning) -> FollowRules {
        FollowRules {
            dead_zone_px: t.camera_dead_zone_px,
            lead_at_rest: t.camera_lead_at_rest,
            lead_ease_seconds: t.camera_lead_ease_seconds,
            reverse_hold_seconds: t.camera_lead_reverse_hold_seconds,
            spring_seconds: t.camera_spring_seconds,
            spectate_delay_seconds: t.camera_spectate_delay_seconds,
            edge_ease_px: t.camera_edge_ease_px,
        }
    }
}

/// One seat's tank as the camera reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeatTank {
    pub seat: usize,
    /// The hull's centre, world px.
    pub position: Vec2,
    /// The way the hull faces as a unit vector on an axis: `Tank::rotation`,
    /// which snaps on a turn, not the eased `visual_rotation`.
    pub facing: Vec2,
    /// The body's velocity, px/s - what the view feeds forward when it has
    /// no motion of its own to measure (a seat it was not following).
    pub velocity: Vec2,
    /// The chassis's undamaged, unboosted top speed, px/s
    /// (`Tank::base_speed`): the speed the look-ahead reaches its full
    /// length at.
    pub top_speed: f32,
    /// In the fight: not a wreck, and not on its way back through a gate.
    pub live: bool,
}

/// The round's seats as the camera reads them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seats {
    /// The seats this screen plays: one, or the two of a couch round.
    pub local: Vec<usize>,
    /// Every seat's tank, in seat order.
    pub tanks: Vec<SeatTank>,
}

impl Seats {
    /// Read `game`'s seats, `local` the ones this screen plays (a seat the
    /// round does not hold is left out).
    pub fn of(game: &Game, local: &[usize]) -> Seats {
        let on_field = game.seats_on_field();
        let tanks = (0..game.players.count())
            .filter_map(|seat| {
                let entity = game.seat(seat)?;
                let (position, rotation, velocity) = game.seat_motion(seat)?;
                let (wreck, top_speed) = crate::simulation::with_tank(&game.world, entity, |t| (t.is_wreck(), t.base_speed()));
                Some(SeatTank {
                    seat,
                    position,
                    facing: cardinal(rotation),
                    velocity,
                    top_speed,
                    live: !wreck && on_field[seat].is_some(),
                })
            })
            .collect();
        Seats { local: local.iter().copied().filter(|&s| s < game.players.count()).collect(), tanks }
    }

    fn tank(&self, seat: usize) -> Option<SeatTank> {
        self.tanks.iter().find(|t| t.seat == seat).copied()
    }
}

/// A hull rotation in degrees (0 up, 90 right) as a unit vector on the
/// nearest axis.
pub fn cardinal(rotation: f32) -> Vec2 {
    let quarter = if rotation.is_finite() { ((rotation / 90.0).round() as i64).rem_euclid(4) } else { 0 };
    match quarter {
        0 => Vec2::new(0.0, -1.0),
        1 => Vec2::new(1.0, 0.0),
        2 => Vec2::new(0.0, 1.0),
        _ => Vec2::new(-1.0, 0.0),
    }
}

/// The view's geometry this frame: how much world it shows, the field's
/// size, and the sight box it keeps on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stage {
    /// The world the view shows, world px (`Framing::visible`).
    pub visible: (f32, f32),
    /// The field's size, world px.
    pub field: (f32, f32),
    pub sight: SightBox,
}

impl Stage {
    /// How far the view's centre may stray from a seat it shows on each
    /// axis and still hold the seat's whole sight box: half the view less
    /// half the box, never below zero (`Framing::room_outside`'s rule).
    pub fn room(&self) -> (f32, f32) {
        ((self.visible.0 / 2.0 - self.sight.half.0).max(0.0), (self.visible.1 / 2.0 - self.sight.half.1).max(0.0))
    }
}

/// What the view is on this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotKind {
    /// The screen's own seat.
    Seat,
    /// The two seats of a couch round, on their midpoint: both sight boxes
    /// fit in the view.
    Shared,
    /// The two seats of a couch round, too far apart for both boxes: the
    /// view is seat 0's and the other seat may be off it.
    Apart,
    /// A live teammate, while the screen's own seat waits as a wreck or
    /// drives back in through a gate.
    Spectating,
    /// No seat to follow - an online round before its welcome: the view
    /// holds where it is.
    Nobody,
}

impl ShotKind {
    /// The spelling `status.camera` uses.
    pub fn name(self) -> &'static str {
        match self {
            ShotKind::Seat => "seat",
            ShotKind::Shared => "shared",
            ShotKind::Apart => "apart",
            ShotKind::Spectating => "spectating",
            ShotKind::Nobody => "nobody",
        }
    }
}

/// Where the view stands this frame, and why.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    /// The view's top-left corner, world px, exact: `Camera::following`
    /// snaps it to the block grid and presents the rest.
    pub corner: Vec2,
    /// The view's centre, world px.
    pub center: Vec2,
    pub kind: ShotKind,
    /// The seat the view follows: seat 0 of a shared pair, `None` for
    /// nobody.
    pub seat: Option<usize>,
    /// Where the seats whose sight boxes the view keeps on screen stand:
    /// the followed seat, and the second seat of a shared pair.
    pub keeps: [Option<Vec2>; 2],
    /// The look-ahead in force, world px from the anchor.
    pub lead: Vec2,
    /// Whether the view cut to this frame rather than moving to it.
    pub cut: bool,
}

impl Shot {
    /// Whether every sight box this view keeps is in `view` (the world
    /// rectangle the frame shows), as far as the field reaches - a box
    /// round a seat by the boundary walls hangs past the field, where
    /// there is nothing to see. `slack` world pixels of rounding are
    /// allowed on every edge.
    pub fn boxes_in(&self, view: Rectangle, field: (f32, f32), sight: SightBox, slack: f32) -> bool {
        self.keeps.iter().flatten().all(|p| {
            let x0 = (p.x - sight.half.0).max(0.0);
            let y0 = (p.y - sight.half.1).max(0.0);
            let x1 = (p.x + sight.half.0).min(field.0);
            let y1 = (p.y + sight.half.1).min(field.1);
            x0 >= view.x - slack && y0 >= view.y - slack && x1 <= view.x + view.width + slack && y1 <= view.y + view.height + slack
        })
    }
}

/// Two couch seats that went apart share a view again only once they are
/// this much closer than the boxes need, so a pair standing at the edge of
/// fitting does not swing the view back and forth.
const SHARE_AGAIN_PX: f32 = 32.0;

/// A seat that moved further in one frame than this past what it could
/// drive in the time went somewhere rather than drove there: the view cuts.
const JUMP_PX: f32 = 64.0;

/// What the view frames this frame.
#[derive(Clone, Copy, Debug)]
enum Target {
    One(SeatTank, ShotKind),
    Shared(SeatTank, SeatTank),
}

impl Target {
    /// Who the view is on, which decides whether a frame continues the last
    /// one's motion: the seat, or the pair.
    fn key(self) -> (usize, Option<usize>) {
        match self {
            Target::One(t, _) => (t.seat, None),
            Target::Shared(a, b) => (a.seat, Some(b.seat)),
        }
    }
}

/// The follow camera's state from one frame to the next.
#[derive(Clone, Debug, Default)]
pub struct Follow {
    /// The view's centre and its velocity: the spring. `None` before the
    /// first frame, which cuts.
    center: Option<Vec2>,
    velocity: Vec2,
    /// The dead zone's anchor.
    base: Vec2,
    /// The look-ahead in force, and the axis direction it leads along.
    lead: Vec2,
    lead_dir: Vec2,
    /// How long the hull has faced against `lead_dir`.
    reversed_for: f32,
    /// Who the view was on last frame, and where the anchor stood.
    last: Option<((usize, Option<usize>), Vec2)>,
    /// How long the screen's own seat has been out of the fight.
    out_for: f32,
    /// Couch: whether the pair was too far apart last frame.
    apart: bool,
    /// Cuts the round's events asked for: seats (a bit each) that went
    /// through a portal or came in through a gate, or everything, for a
    /// round that started.
    cut_seats: u32,
    cut_all: bool,
    /// The last simulation frame whose events were read.
    seen_frame: Option<u64>,
}

impl Follow {
    /// Read the events of the simulation frame `game` stands at, once per
    /// frame: call it after every step, as `Fx::observe_events` is called,
    /// since a rendered frame that runs two steps would lose the first
    /// one's events otherwise. A round that starts over, or a frame counter
    /// that goes back, cuts.
    pub fn observe_events(&mut self, game: &Game) {
        self.note(game.frame(), game.events(), game.players.count());
    }

    fn note(&mut self, frame: u64, events: &[Event], seats: usize) {
        if self.seen_frame == Some(frame) {
            return;
        }
        if self.seen_frame.is_some_and(|seen| frame < seen) {
            self.cut_all = true;
        }
        self.seen_frame = Some(frame);
        for event in events {
            match *event {
                Event::RoundStarted { .. } => self.cut_all = true,
                Event::Teleported { slot, .. } | Event::TankEntered { slot } if slot < seats.min(MAX_SEATS) => {
                    self.cut_seats |= 1 << slot;
                }
                _ => {}
            }
        }
    }

    /// Cut on the next frame: the round on screen changed (a room's replica
    /// for the local round or back, another map).
    pub fn cut(&mut self) {
        self.cut_all = true;
    }

    /// Move the view on by `dt` seconds of the round - zero on a frame
    /// that ran no step, so the view stands still with the round.
    pub fn update(&mut self, seats: &Seats, dt: f32, stage: &Stage, rules: &FollowRules) -> Shot {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        let room = stage.room();
        let Some(target) = self.choose(seats, dt, stage, rules) else {
            return self.hold(stage);
        };

        // The anchor, and how it moves.
        let (anchor, facing, body_velocity, top_speed, keeps, kind, seat) = match target {
            Target::One(t, kind) => (t.position, Some(t.facing), t.velocity, t.top_speed, [Some(t.position), None], kind, t.seat),
            Target::Shared(a, b) => (
                (a.position + b.position) * 0.5,
                None,
                (a.velocity + b.velocity) * 0.5,
                a.top_speed.min(b.top_speed),
                [Some(a.position), Some(b.position)],
                ShotKind::Shared,
                a.seat,
            ),
        };
        let key = target.key();
        let same = self.last.is_some_and(|(k, _)| k == key);
        let moved = self.last.map_or(Vec2::new(0.0, 0.0), |(_, at)| anchor - at);
        let jumped = same && moved.length() > top_speed.max(0.0) * 2.0 * dt + JUMP_PX;
        let far = !same
            && self
                .center
                .is_some_and(|c| (anchor.x - c.x).abs() > stage.visible.0 || (anchor.y - c.y).abs() > stage.visible.1);
        let seat_bit = |s: usize| s < 32 && self.cut_seats & (1 << s) != 0;
        let portal = match target {
            Target::One(t, _) => seat_bit(t.seat),
            Target::Shared(a, b) => seat_bit(a.seat) || seat_bit(b.seat),
        };
        let cut = self.center.is_none() || self.cut_all || portal || jumped || far;
        self.cut_all = false;
        self.cut_seats = 0;
        self.last = Some((key, anchor));

        // How the anchor moves: what it did since the last frame, when that
        // was a drive and not a jump - the body's velocity reports motion
        // for a hull jammed against a wall - else what the body says.
        let velocity = if same && !cut && dt > 0.0 { moved * (1.0 / dt) } else { body_velocity };
        let speed = if top_speed > 0.0 { (velocity.length() / top_speed).clamp(0.0, 1.0) } else { 0.0 };

        let dead_zone = rules.dead_zone_px.max(0.0);
        let cap = ((room.0 - dead_zone).max(0.0), (room.1 - dead_zone).max(0.0));
        let goal_velocity;
        if cut {
            self.base = anchor;
            self.lead_dir = facing.unwrap_or(Vec2::new(0.0, 0.0));
            self.reversed_for = 0.0;
            self.lead = lead_for(self.lead_dir, speed, cap, rules);
            goal_velocity = velocity;
            self.center = Some(anchor + self.lead);
            self.velocity = goal_velocity;
        } else {
            // Dead zone: the anchor drags the base once it is past the
            // zone's edge, and only then is the anchor's velocity the
            // goal's.
            let (base_x, drag_x) = dead_zone_axis(self.base.x, anchor.x, velocity.x, dead_zone);
            let (base_y, drag_y) = dead_zone_axis(self.base.y, anchor.y, velocity.y, dead_zone);
            self.base = Vec2::new(base_x, base_y);
            goal_velocity = Vec2::new(if drag_x { velocity.x } else { 0.0 }, if drag_y { velocity.y } else { 0.0 });

            // Look-ahead: which way, then how far, eased.
            match facing {
                Some(f) if self.lead_dir == Vec2::new(0.0, 0.0) => self.lead_dir = f,
                Some(f) if f == self.lead_dir => self.reversed_for = 0.0,
                Some(f) if f == self.lead_dir * -1.0 => {
                    self.reversed_for += dt;
                    if self.reversed_for >= rules.reverse_hold_seconds {
                        self.lead_dir = f;
                        self.reversed_for = 0.0;
                    }
                }
                Some(f) => {
                    self.lead_dir = f;
                    self.reversed_for = 0.0;
                }
                None => {
                    self.lead_dir = Vec2::new(0.0, 0.0);
                    self.reversed_for = 0.0;
                }
            }
            let want = lead_for(self.lead_dir, speed, cap, rules);
            self.lead = Vec2::new(
                ease_toward(self.lead.x, want.x, cap.0, dt, rules.lead_ease_seconds),
                ease_toward(self.lead.y, want.y, cap.1, dt, rules.lead_ease_seconds),
            );

            // The spring, on each axis, toward a goal eased into the field
            // over its last `camera_edge_ease_px`, so the view slows into
            // the field's edge rather than running into it.
            let center = self.center.unwrap_or(anchor);
            let goal = self.base + self.lead;
            let (gx, kx) = ease_into_field(goal.x, stage.visible.0, stage.field.0, rules.edge_ease_px);
            let (gy, ky) = ease_into_field(goal.y, stage.visible.1, stage.field.1, rules.edge_ease_px);
            let (x, vx) = spring(center.x, self.velocity.x, gx, goal_velocity.x * kx, dt, rules.spring_seconds);
            let (y, vy) = spring(center.y, self.velocity.y, gy, goal_velocity.y * ky, dt, rules.spring_seconds);
            self.center = Some(Vec2::new(x, y));
            self.velocity = Vec2::new(vx, vy);
        }

        // The sight boxes, then the field.
        let mut center = self.center.unwrap_or(anchor);
        let (cx, held_x) = keep_boxes(center.x, keeps.iter().flatten().map(|p| p.x), room.0);
        let (cy, held_y) = keep_boxes(center.y, keeps.iter().flatten().map(|p| p.y), room.1);
        center = Vec2::new(cx, cy);
        if held_x {
            self.velocity.x = goal_velocity.x;
        }
        if held_y {
            self.velocity.y = goal_velocity.y;
        }
        let (fx, edge_x) = keep_in_field(center.x, stage.visible.0, stage.field.0);
        let (fy, edge_y) = keep_in_field(center.y, stage.visible.1, stage.field.1);
        center = Vec2::new(fx, fy);
        if edge_x {
            self.velocity.x = 0.0;
        }
        if edge_y {
            self.velocity.y = 0.0;
        }
        self.center = Some(center);

        Shot {
            corner: Vec2::new(center.x - stage.visible.0 / 2.0, center.y - stage.visible.1 / 2.0),
            center,
            kind,
            seat: Some(seat),
            keeps,
            lead: self.lead,
            cut,
        }
    }

    /// Who the view is on this frame.
    fn choose(&mut self, seats: &Seats, dt: f32, stage: &Stage, rules: &FollowRules) -> Option<Target> {
        let local: Vec<SeatTank> = seats.local.iter().filter_map(|&s| seats.tank(s)).collect();
        let first = *local.first()?;
        if let [a, b, ..] = local[..] {
            self.out_for = 0.0;
            return Some(match (a.live, b.live) {
                (true, true) => {
                    let slack = if self.apart { SHARE_AGAIN_PX } else { 0.0 };
                    let gap = b.position - a.position;
                    let fits = |gap: f32, visible: f32, half: f32| gap.abs() + 2.0 * half + slack <= visible;
                    if fits(gap.x, stage.visible.0, stage.sight.half.0) && fits(gap.y, stage.visible.1, stage.sight.half.1) {
                        self.apart = false;
                        Target::Shared(a, b)
                    } else {
                        self.apart = true;
                        Target::One(a, ShotKind::Apart)
                    }
                }
                (false, true) => Target::One(b, ShotKind::Seat),
                _ => Target::One(a, ShotKind::Seat),
            });
        }
        self.apart = false;
        if first.live {
            self.out_for = 0.0;
            return Some(Target::One(first, ShotKind::Seat));
        }
        self.out_for += dt;
        if self.out_for >= rules.spectate_delay_seconds {
            let mate = seats
                .tanks
                .iter()
                .filter(|t| t.live && !seats.local.contains(&t.seat))
                .min_by(|a, b| {
                    let (da, db) = (a.position.distance_to(first.position), b.position.distance_to(first.position));
                    da.total_cmp(&db).then(a.seat.cmp(&b.seat))
                });
            if let Some(mate) = mate {
                return Some(Target::One(*mate, ShotKind::Spectating));
            }
        }
        Some(Target::One(first, ShotKind::Seat))
    }

    /// No seat to follow: the view stays where it was, or on the field's
    /// middle, inside the field.
    fn hold(&mut self, stage: &Stage) -> Shot {
        let middle = Vec2::new(stage.field.0 / 2.0, stage.field.1 / 2.0);
        let at = self.center.unwrap_or(middle);
        let center = Vec2::new(keep_in_field(at.x, stage.visible.0, stage.field.0).0, keep_in_field(at.y, stage.visible.1, stage.field.1).0);
        self.center = Some(center);
        self.velocity = Vec2::new(0.0, 0.0);
        self.last = None;
        Shot {
            corner: Vec2::new(center.x - stage.visible.0 / 2.0, center.y - stage.visible.1 / 2.0),
            center,
            kind: ShotKind::Nobody,
            seat: None,
            keeps: [None, None],
            lead: Vec2::new(0.0, 0.0),
            cut: false,
        }
    }
}

/// The look-ahead along `dir` at `speed` (0 at rest to 1 at top speed):
/// `lead_at_rest` of the room `cap` leaves on that axis at rest, all of it
/// at top speed.
fn lead_for(dir: Vec2, speed: f32, cap: (f32, f32), rules: &FollowRules) -> Vec2 {
    let rest = rules.lead_at_rest.clamp(0.0, 1.0);
    let share = rest + (1.0 - rest) * speed.clamp(0.0, 1.0);
    Vec2::new(dir.x * share * cap.0, dir.y * share * cap.1)
}

/// One axis of the look-ahead: `lead` moved toward `want` at a steady
/// pace, the one that swings it from one side of its room `cap` to the
/// other in `seconds` - so a reversal takes about that long and a smaller
/// change less - and kept inside the room. No time to ease over is the
/// wanted lead itself.
fn ease_toward(lead: f32, want: f32, cap: f32, dt: f32, seconds: f32) -> f32 {
    let moved = if seconds > 0.0 {
        let step = 2.0 * cap * dt / seconds;
        lead + (want - lead).clamp(-step, step)
    } else {
        want
    };
    moved.clamp(-cap, cap)
}

/// One axis of the dead zone: the base moved to within `zone` of the
/// anchor, and whether the anchor is dragging it - at the zone's edge and
/// moving outward (with no zone at all, whenever it moves).
fn dead_zone_axis(base: f32, anchor: f32, velocity: f32, zone: f32) -> (f32, bool) {
    let base = base.clamp(anchor - zone, anchor + zone);
    let off = anchor - base;
    let dragging = if zone <= 0.0 { true } else { off.abs() >= zone - 1e-3 && off * velocity > 0.0 };
    (base, dragging)
}

/// One axis of a critically damped spring of smoothing time `smooth`
/// (`2 / smooth` its angular frequency, Unity's `SmoothDamp`) carrying `x`
/// moving at `v` toward `goal`, which is taken to have moved at `goal_v`
/// through the step - so it stood `goal_v * dt` back when the step began.
/// Integrated exactly: two steps of `dt` land where one of `2 dt` does,
/// and a goal moving steadily is followed with no lag at all. No time
/// moves nothing; no smoothing is the goal itself.
pub fn spring(x: f32, v: f32, goal: f32, goal_v: f32, dt: f32, smooth: f32) -> (f32, f32) {
    if dt <= 0.0 {
        return (x, v);
    }
    if smooth <= 0.0 {
        return (goal, goal_v);
    }
    let w = 2.0 / smooth;
    let e = x - (goal - goal_v * dt);
    let de = v - goal_v;
    let j = de + w * e;
    let decay = (-w * dt).exp();
    (goal + (e + j * dt) * decay, goal_v + (de - w * j * dt) * decay)
}

/// One axis of the sight boxes: `center` moved into reach of every seat at
/// `seats` - no further than `room` from any - and whether it had to move.
/// Seats too far apart for one centre (which `Follow::choose` never frames
/// together) keep the first one's reach.
fn keep_boxes(center: f32, seats: impl Iterator<Item = f32> + Clone, room: f32) -> (f32, bool) {
    let lo = seats.clone().map(|p| p - room).fold(f32::NEG_INFINITY, f32::max);
    let hi = seats.clone().map(|p| p + room).fold(f32::INFINITY, f32::min);
    let (lo, hi) = if lo <= hi {
        (lo, hi)
    } else {
        let first = seats.clone().next().unwrap_or(center);
        (first - room, first + room)
    };
    if !lo.is_finite() || !hi.is_finite() {
        return (center, false);
    }
    let kept = center.clamp(lo, hi);
    (kept, kept != center)
}

/// One axis of the view's goal near the field's edge: past `ease` pixels
/// from where the view would meet the edge the goal is itself; inside them
/// it is eased into the edge, never reaching past it, and moves the
/// slower the further it went - which is the factor returned, to scale
/// the goal's velocity by. On an axis the view is the longer, the field's
/// middle, standing still. No ease is a plain clamp, the velocity kept
/// only inside it.
fn ease_into_field(goal: f32, visible: f32, field: f32, ease: f32) -> (f32, f32) {
    if visible >= field {
        return (field / 2.0, 0.0);
    }
    let (lo, hi) = (visible / 2.0, field - visible / 2.0);
    let ease = ease.min((hi - lo) / 2.0);
    if ease <= 0.0 {
        let kept = goal.clamp(lo, hi);
        return (kept, if kept == goal { 1.0 } else { 0.0 });
    }
    // Inside the band the distance to the edge `d` becomes
    // `ease * e^((d - ease) / ease)`: the same value and slope where the
    // band starts, and nothing left past the edge.
    let soften = |d: f32| {
        if d >= ease {
            (d, 1.0)
        } else {
            let k = ((d - ease) / ease).exp();
            (ease * k, k)
        }
    };
    let (below, k_lo) = soften(goal - lo);
    let (above, k_hi) = soften(hi - goal);
    if k_lo < 1.0 {
        (lo + below, k_lo)
    } else if k_hi < 1.0 {
        (hi - above, k_hi)
    } else {
        (goal, 1.0)
    }
}

/// One axis of the field: `center` moved so the view of `visible` stays
/// inside a field of `field`, or onto the field's middle where the view is
/// the longer; and whether it had to move.
fn keep_in_field(center: f32, visible: f32, field: f32) -> (f32, bool) {
    let kept = if visible >= field { field / 2.0 } else { center.clamp(visible / 2.0, field - visible / 2.0) };
    (kept, kept != center)
}

/// How the window drew the world this frame (`status.camera`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraMode {
    /// The whole field, as every arena shows it.
    Whole,
    /// A field map, followed.
    Follow,
    /// A view the dev server's `camera` tool pinned.
    Pinned,
    /// The builder's own camera over its canvas (`editor::camera`).
    Build,
}

impl CameraMode {
    /// The spelling `status.camera.view` uses.
    pub fn name(self) -> &'static str {
        match self {
            CameraMode::Whole => "whole",
            CameraMode::Follow => "follow",
            CameraMode::Pinned => "pinned",
            CameraMode::Build => "build",
        }
    }
}

/// One frame's camera, as the window drew it: the view, the bitmap it
/// landed in and how that met the window, and for a followed field map the
/// framing and the follow behind it. The window hands it to the dev
/// server each frame, which reports it and hit-tests a `click` on the same
/// bitmap a finger lands on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraReport {
    pub mode: CameraMode,
    pub camera: Camera,
    /// The bitmap: the bar and the field area.
    pub layout: Layout,
    pub view: View,
    pub follow: Option<FollowReport>,
}

/// A followed field map's half of `CameraReport`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FollowReport {
    pub framing: Framing,
    pub seating: Seating,
    pub sight: SightBox,
    pub shot: Shot,
}

#[cfg(test)]
mod follow_tests {
    use super::*;

    fn rules() -> FollowRules {
        FollowRules::of(&Tuning::DEFAULT)
    }

    /// The 1080p monitor's local view under the bar: 40 x 21.5 cells, on
    /// grand-campaign's 48 x 24 field, with the standard sight box.
    fn stage() -> Stage {
        Stage { visible: (1280.0, 688.0), field: (1536.0, 768.0), sight: SightBox::from_cells(11.5, 7.5) }
    }

    /// The same view on a field far bigger than it, so the field's edge
    /// never holds the view.
    fn open_stage() -> Stage {
        Stage { field: (4000.0, 2000.0), ..stage() }
    }

    fn tank(seat: usize, x: f32, y: f32) -> SeatTank {
        SeatTank {
            seat,
            position: Vec2::new(x, y),
            facing: Vec2::new(1.0, 0.0),
            velocity: Vec2::new(0.0, 0.0),
            top_speed: 210.0,
            live: true,
        }
    }

    fn one(t: SeatTank) -> Seats {
        Seats { local: vec![t.seat], tanks: vec![t] }
    }

    const DT: f32 = 1.0 / 60.0;

    #[test]
    fn the_rules_come_from_the_camera_group() {
        let r = rules();
        assert_eq!(r.dead_zone_px, 12.8, "about 0.4 cell");
        assert_eq!((r.lead_at_rest, r.lead_ease_seconds, r.spring_seconds), (0.35, 0.5, 0.16));
        for name in [
            "camera_dead_zone_px",
            "camera_lead_at_rest",
            "camera_lead_ease_seconds",
            "camera_lead_reverse_hold_seconds",
            "camera_spring_seconds",
            "camera_spectate_delay_seconds",
            "camera_edge_ease_px",
        ] {
            assert_eq!(Tuning::meta(name).map(|m| m.group), Some("camera"), "{name}");
        }
    }

    #[test]
    fn a_rotation_snaps_to_the_nearest_axis() {
        assert_eq!(cardinal(0.0), Vec2::new(0.0, -1.0));
        assert_eq!(cardinal(90.0), Vec2::new(1.0, 0.0));
        assert_eq!(cardinal(180.0), Vec2::new(0.0, 1.0));
        assert_eq!(cardinal(270.0), Vec2::new(-1.0, 0.0));
        assert_eq!(cardinal(-90.0), Vec2::new(-1.0, 0.0));
        assert_eq!(cardinal(359.0), Vec2::new(0.0, -1.0));
        assert_eq!(cardinal(f32::NAN), Vec2::new(0.0, -1.0));
    }

    #[test]
    fn the_first_frame_cuts_onto_the_seat_with_the_lead_at_rest() {
        let mut f = Follow::default();
        let s = stage();
        let shot = f.update(&one(tank(0, 700.0, 400.0)), DT, &s, &rules());
        assert!(shot.cut);
        assert_eq!(shot.kind, ShotKind::Seat);
        // Facing right at rest: 35 % of the room right of the box, less the
        // dead zone.
        let cap = s.room().0 - rules().dead_zone_px;
        assert!((shot.lead.x - 0.35 * cap).abs() < 1e-3 && shot.lead.y == 0.0, "{:?}", shot.lead);
        assert!((shot.center.x - (700.0 + 0.35 * cap)).abs() < 1e-3 && shot.center.y == 400.0, "{shot:?}");
        assert_eq!(shot.corner, Vec2::new(shot.center.x - 640.0, 400.0 - 344.0));
    }

    #[test]
    fn the_dead_zone_holds_the_view_until_the_seat_leaves_it() {
        let mut f = Follow::default();
        let s = open_stage();
        let r = FollowRules { lead_at_rest: 0.0, ..rules() };
        let mut t = tank(0, 700.0, 400.0);
        t.facing = Vec2::new(0.0, -1.0);
        let start = f.update(&one(t), DT, &s, &r).center;
        // Wobbling inside the zone on the cross axis moves nothing.
        for (i, dx) in [4.0, -6.0, 9.0, -12.0, 3.0].iter().enumerate() {
            t.position.x = 700.0 + dx;
            let shot = f.update(&one(t), DT, &s, &r);
            assert_eq!(shot.center.x, start.x, "frame {i}: {shot:?}");
        }
        // Past the zone, the view follows: it ends up the zone's width
        // behind the seat.
        let mut shot = f.update(&one(t), DT, &s, &r);
        for _ in 0..240 {
            t.position.x += 1.0;
            shot = f.update(&one(t), DT, &s, &r);
        }
        assert!((t.position.x - shot.center.x - r.dead_zone_px).abs() < 0.01, "{} vs {}", t.position.x, shot.center.x);
        // When the seat stops, the view coasts a little past where it
        // stood and settles back, the zone's width behind it again.
        let mut furthest: f32 = 0.0;
        for _ in 0..120 {
            shot = f.update(&one(t), DT, &s, &r);
            furthest = furthest.max(shot.center.x - (t.position.x - r.dead_zone_px));
        }
        assert!(furthest > 0.0 && furthest < 2.0, "{furthest}");
        assert!((t.position.x - shot.center.x - r.dead_zone_px).abs() < 0.01, "{} vs {}", t.position.x, shot.center.x);
    }

    #[test]
    fn the_look_ahead_grows_with_speed_and_never_spends_more_than_the_room() {
        let s = stage();
        let r = rules();
        let cap = (s.room().0 - r.dead_zone_px, s.room().1 - r.dead_zone_px);
        for (facing, axis) in [(Vec2::new(1.0, 0.0), 0), (Vec2::new(0.0, 1.0), 1), (Vec2::new(-1.0, 0.0), 0), (Vec2::new(0.0, -1.0), 1)] {
            let mut f = Follow::default();
            let mut t = tank(0, 768.0, 384.0);
            t.facing = facing;
            let mut max_lead: f32 = 0.0;
            for _ in 0..300 {
                // Full speed along the facing, wherever the field lets it.
                t.position = t.position + facing * (210.0 * DT);
                t.position = Vec2::new(t.position.x.clamp(100.0, 1436.0), t.position.y.clamp(100.0, 668.0));
                let shot = f.update(&one(t), DT, &s, &r);
                let lead = if axis == 0 { shot.lead.x.abs() } else { shot.lead.y.abs() };
                max_lead = max_lead.max(lead);
                let room = s.room();
                assert!(lead <= [cap.0, cap.1][axis] + 1e-3, "{facing:?}: lead {lead} past the cap");
                // Whatever the field does, the view keeps the seat's box.
                let rect = Rectangle::new(shot.corner.x, shot.corner.y, s.visible.0, s.visible.1);
                assert!(shot.boxes_in(rect, s.field, s.sight, 1e-3), "{facing:?}: {shot:?} room {room:?}");
            }
            assert!(max_lead > 0.9 * [cap.0, cap.1][axis], "{facing:?}: the lead reached {max_lead} of {cap:?}");
        }
    }

    #[test]
    fn the_sight_box_stays_in_view_whatever_the_spring_does() {
        // A spring so slow it would trail far behind: the box still holds.
        let r = FollowRules { spring_seconds: 2.0, ..rules() };
        for s in [stage(), open_stage()] {
            let mut f = Follow::default();
            let mut t = tank(0, 200.0, 384.0);
            for frame in 0..400 {
                t.position.x = (t.position.x + 420.0 * DT).min(1400.0);
                let shot = f.update(&one(t), DT, &s, &r);
                let rect = Rectangle::new(shot.corner.x, shot.corner.y, s.visible.0, s.visible.1);
                assert!(shot.boxes_in(rect, s.field, s.sight, 1e-3), "frame {frame}: {shot:?}");
                if s.field.0 > 2000.0 && t.position.x > s.visible.0 {
                    // Away from the field's edge the centre itself stays in
                    // reach of the seat.
                    assert!((shot.center.x - t.position.x).abs() <= s.room().0 + 1e-3, "frame {frame}: {shot:?}");
                }
            }
        }
    }

    #[test]
    fn the_spring_settles_without_overshoot_and_at_any_frame_rate() {
        // From rest toward a still goal: monotone, there in about a second.
        let (mut x, mut v) = (0.0f32, 0.0f32);
        let mut last = x;
        for _ in 0..60 {
            (x, v) = spring(x, v, 100.0, 0.0, DT, 0.16);
            assert!(x >= last - 1e-4 && x <= 100.0 + 1e-3, "{x}");
            last = x;
        }
        assert!((x - 100.0).abs() < 0.01, "{x}");
        // Two half steps land where one whole step does, for a goal moving
        // steadily: the integration is exact.
        let one = spring(10.0, 50.0, 200.0, 120.0, 0.1, 0.16);
        let half = spring(10.0, 50.0, 200.0 - 120.0 * 0.05, 120.0, 0.05, 0.16);
        let two = spring(half.0, half.1, 200.0, 120.0, 0.05, 0.16);
        assert!((one.0 - two.0).abs() < 1e-3 && (one.1 - two.1).abs() < 1e-2, "{one:?} vs {two:?}");
        // A goal moving steadily is followed with no lag once caught.
        let (mut x, mut v) = (0.0f32, 0.0f32);
        let mut goal = 0.0;
        for _ in 0..240 {
            goal += 210.0 * DT;
            (x, v) = spring(x, v, goal, 210.0, DT, 0.16);
        }
        assert!((x - goal).abs() < 0.01 && (v - 210.0).abs() < 0.01, "{x} {v} vs {goal}");
        // No time moves nothing; no smoothing is the goal.
        assert_eq!(spring(5.0, 1.0, 50.0, 0.0, 0.0, 0.16), (5.0, 1.0));
        assert_eq!(spring(5.0, 1.0, 50.0, 3.0, DT, 0.0), (50.0, 3.0));
    }

    #[test]
    fn a_drive_is_followed_at_any_frame_rate() {
        // The same drive at 60, 30 and 144 frames a second lands the view
        // in the same place.
        let s = stage();
        let r = rules();
        let ends: Vec<Vec2> = [60.0f32, 30.0, 144.0]
            .iter()
            .map(|hz| {
                let mut f = Follow::default();
                let mut t = tank(0, 300.0, 384.0);
                let dt = 1.0 / hz;
                let mut shot = f.update(&one(t), dt, &s, &r);
                let frames = (1.5 * hz).round() as usize;
                for _ in 0..frames {
                    t.position.x += 150.0 * dt;
                    shot = f.update(&one(t), dt, &s, &r);
                }
                shot.center
            })
            .collect();
        for end in &ends[1..] {
            assert!((end.x - ends[0].x).abs() < 1.0 && (end.y - ends[0].y).abs() < 1e-3, "{ends:?}");
        }
    }

    #[test]
    fn a_reversal_holds_before_the_look_ahead_flips_and_a_turn_does_not() {
        let s = stage();
        let r = rules();
        let mut f = Follow::default();
        let mut t = tank(0, 768.0, 384.0);
        f.update(&one(t), DT, &s, &r);
        for _ in 0..60 {
            f.update(&one(t), DT, &s, &r);
        }
        // Reverse: for the hold the lead keeps its side.
        t.facing = Vec2::new(-1.0, 0.0);
        let held = (r.reverse_hold_seconds / DT) as usize - 1;
        for _ in 0..held {
            let shot = f.update(&one(t), DT, &s, &r);
            assert!(shot.lead.x > 0.0, "{shot:?}");
        }
        for _ in 0..60 {
            f.update(&one(t), DT, &s, &r);
        }
        assert!(f.update(&one(t), DT, &s, &r).lead.x < 0.0, "flipped after the hold");
        // A quarter turn swings it at once.
        t.facing = Vec2::new(0.0, 1.0);
        let shot = f.update(&one(t), DT, &s, &r);
        assert!(shot.lead.y > 0.0, "{shot:?}");
    }

    #[test]
    fn a_reversal_at_speed_swings_the_view_at_a_steady_pace() {
        // Full speed right, then full speed left: the look-ahead crosses
        // its room no faster than one side to the other in the ease time,
        // and the view never jumps a frame.
        let s = open_stage();
        let r = rules();
        let cap = s.room().0 - r.dead_zone_px;
        let mut f = Follow::default();
        let mut t = tank(0, 1500.0, 1000.0);
        t.velocity = Vec2::new(210.0, 0.0);
        let mut last = f.update(&one(t), DT, &s, &r);
        for _ in 0..120 {
            t.position.x += 210.0 * DT;
            last = f.update(&one(t), DT, &s, &r);
        }
        assert!(last.lead.x > 0.9 * cap, "{last:?}");
        t.facing = Vec2::new(-1.0, 0.0);
        t.velocity = Vec2::new(-210.0, 0.0);
        let mut swing = 0;
        for _ in 0..120 {
            t.position.x -= 210.0 * DT;
            let shot = f.update(&one(t), DT, &s, &r);
            assert!((shot.lead.x - last.lead.x).abs() <= 2.0 * cap * DT / r.lead_ease_seconds + 1e-3, "{last:?} -> {shot:?}");
            assert!((shot.center.x - last.center.x).abs() <= 210.0 * DT + 2.0 * cap * DT / r.lead_ease_seconds + 1.0, "{last:?} -> {shot:?}");
            if shot.lead.x < last.lead.x {
                swing += 1;
            }
            last = shot;
        }
        assert!(last.lead.x < -0.9 * cap, "{last:?}");
        let seconds = swing as f32 * DT;
        assert!(seconds >= 0.9 * r.lead_ease_seconds && seconds <= 1.1 * r.lead_ease_seconds, "a full swing in {seconds} s");
    }

    #[test]
    fn the_view_stays_inside_the_field_and_centres_a_short_axis() {
        let r = rules();
        let s = stage();
        let mut f = Follow::default();
        let corner = f.update(&one(tank(0, 40.0, 40.0)), DT, &s, &r).corner;
        assert_eq!(corner, Vec2::new(0.0, 0.0), "the top-left corner");
        let mut f = Follow::default();
        let shot = f.update(&one(tank(0, 1500.0, 740.0)), DT, &s, &r);
        assert_eq!(shot.corner, Vec2::new(1536.0 - 1280.0, 768.0 - 688.0), "the bottom-right corner");
        // Hedge Maze's 40 x 20 is shorter than the view's 21.5 rows: centred.
        let short = Stage { field: (1280.0, 640.0), ..s };
        let mut f = Follow::default();
        let shot = f.update(&one(tank(0, 200.0, 100.0)), DT, &short, &r);
        assert_eq!(shot.corner, Vec2::new(0.0, (640.0 - 688.0) / 2.0));
    }

    #[test]
    fn the_view_slows_into_the_fields_edge_rather_than_stopping_dead() {
        // Full speed toward the right wall: the view's step shrinks frame
        // by frame to nothing at the edge, which it never passes.
        let s = stage();
        let r = rules();
        let edge = s.field.0 - s.visible.0 / 2.0;
        let mut f = Follow::default();
        let mut t = tank(0, 700.0, 384.0);
        t.velocity = Vec2::new(210.0, 0.0);
        let mut last = f.update(&one(t), DT, &s, &r).center.x;
        let mut last_step = 0.0f32;
        let mut biggest_change = 0.0f32;
        for _ in 0..360 {
            t.position.x = (t.position.x + 210.0 * DT).min(1500.0);
            let x = f.update(&one(t), DT, &s, &r).center.x;
            assert!(x <= edge + 1e-3, "{x} past {edge}");
            let step = x - last;
            biggest_change = biggest_change.max((step - last_step).abs());
            (last, last_step) = (x, step);
        }
        assert!((last - edge).abs() < 0.5, "the view comes to rest at the edge: {last} vs {edge}");
        assert!(biggest_change < 1.0, "the step changed by {biggest_change} px in a frame");
        // Without the ease the same drive stops dead at the edge.
        let hard = FollowRules { edge_ease_px: 0.0, ..r };
        let mut f = Follow::default();
        let mut t = tank(0, 700.0, 384.0);
        let mut last = f.update(&one(t), DT, &s, &hard).center.x;
        let mut steps = Vec::new();
        for _ in 0..360 {
            t.position.x = (t.position.x + 210.0 * DT).min(1500.0);
            let x = f.update(&one(t), DT, &s, &hard).center.x;
            steps.push(x - last);
            last = x;
        }
        let jolt = steps.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(jolt > 2.0, "{jolt}");
    }

    #[test]
    fn a_portal_a_gate_and_a_new_round_cut() {
        let s = open_stage();
        let r = rules();
        let mut f = Follow::default();
        let mut t = tank(0, 400.0, 384.0);
        f.note(1, &[], 1);
        assert!(f.update(&one(t), DT, &s, &r).cut, "the first frame");
        for frame in 2..20 {
            f.note(frame, &[], 1);
            t.position.x += 3.0;
            assert!(!f.update(&one(t), DT, &s, &r).cut, "driving is followed");
        }
        // Through a portal: the seat lands 900 px away and the view is there.
        t.position.x += 900.0;
        f.note(20, &[Event::Teleported { slot: 0, x: 457.0, y: 384.0, to_x: t.position.x, to_y: 384.0 }], 1);
        let shot = f.update(&one(t), DT, &s, &r);
        assert!(shot.cut && (shot.center.x - t.position.x).abs() <= s.room().0, "{shot:?}");
        // An enemy's portal or gate is nobody's cut.
        f.note(21, &[Event::Teleported { slot: 1, x: 0.0, y: 0.0, to_x: 1.0, to_y: 1.0 }, Event::TankEntered { slot: 1 }], 1);
        assert!(!f.update(&one(t), DT, &s, &r).cut);
        // The seat back through its gate, and a round started.
        f.note(22, &[Event::TankEntered { slot: 0 }], 1);
        assert!(f.update(&one(t), DT, &s, &r).cut);
        f.note(23, &[], 1);
        assert!(!f.update(&one(t), DT, &s, &r).cut);
        f.note(0, &[], 1);
        assert!(f.update(&one(t), DT, &s, &r).cut, "a frame counter that went back is a new round");
        // A frame already read is not read twice.
        f.note(1, &[Event::TankEntered { slot: 0 }], 1);
        f.update(&one(t), DT, &s, &r);
        f.note(1, &[Event::TankEntered { slot: 0 }], 1);
        assert!(!f.update(&one(t), DT, &s, &r).cut);
        // Moving further than a tank can drive in the time is a cut too.
        t.position.y += 200.0;
        assert!(f.update(&one(t), DT, &s, &r).cut);
    }

    #[test]
    fn a_wrecked_seat_hands_the_view_to_the_nearest_teammate_and_takes_it_back() {
        let s = stage();
        let r = rules();
        let mut f = Follow::default();
        let mut me = tank(0, 300.0, 300.0);
        let near = tank(2, 600.0, 300.0);
        let far = tank(1, 1400.0, 700.0);
        let seats = |me: SeatTank| Seats { local: vec![0], tanks: vec![me, far, near] };
        assert_eq!(f.update(&seats(me), DT, &s, &r).seat, Some(0));
        me.live = false;
        let delay = (r.spectate_delay_seconds / DT).ceil() as usize - 1;
        for _ in 0..delay {
            let shot = f.update(&seats(me), DT, &s, &r);
            assert_eq!((shot.kind, shot.seat), (ShotKind::Seat, Some(0)), "the view stays on the wreck a while");
        }
        f.update(&seats(me), DT, &s, &r);
        let shot = f.update(&seats(me), DT, &s, &r);
        assert_eq!((shot.kind, shot.seat), (ShotKind::Spectating, Some(2)), "the nearest live teammate");
        assert!(!shot.cut, "a teammate within a screen is panned to");
        // Back through the gate: the seat has the view again, cut.
        me.live = true;
        me.position = Vec2::new(1500.0, 40.0);
        f.note(5, &[Event::TankEntered { slot: 0 }], 3);
        let shot = f.update(&seats(me), DT, &s, &r);
        assert_eq!((shot.kind, shot.seat, shot.cut), (ShotKind::Seat, Some(0), true));
        // Alone, a wreck keeps the view.
        let mut f = Follow::default();
        let mut alone = tank(0, 300.0, 300.0);
        alone.live = false;
        for _ in 0..200 {
            assert_eq!(f.update(&one(alone), DT, &s, &r).seat, Some(0));
        }
    }

    #[test]
    fn a_couch_pair_shares_the_view_while_both_boxes_fit_and_seat_0_has_it_when_not() {
        let s = stage();
        let r = rules();
        let mut f = Follow::default();
        let a = tank(0, 500.0, 384.0);
        let mut b = tank(1, 800.0, 384.0);
        let pair = |a: SeatTank, b: SeatTank| Seats { local: vec![0, 1], tanks: vec![a, b] };
        let shot = f.update(&pair(a, b), DT, &s, &r);
        assert_eq!(shot.kind, ShotKind::Shared);
        assert_eq!(shot.center, Vec2::new(650.0, 384.0), "the midpoint, no look-ahead");
        let rect = |shot: &Shot| Rectangle::new(shot.corner.x, shot.corner.y, s.visible.0, s.visible.1);
        assert!(shot.boxes_in(rect(&shot), s.field, s.sight, 1e-3));
        // Further apart than two boxes fit: seat 0's view, reported.
        b.position.x = 500.0 + (1280.0 - 2.0 * 368.0) + 10.0;
        let shot = f.update(&pair(a, b), DT, &s, &r);
        assert_eq!((shot.kind, shot.seat), (ShotKind::Apart, Some(0)));
        // Back to just fitting is not enough to share again ...
        b.position.x = 500.0 + (1280.0 - 2.0 * 368.0) - 10.0;
        assert_eq!(f.update(&pair(a, b), DT, &s, &r).kind, ShotKind::Apart);
        // ... a cell closer is.
        b.position.x -= 32.0;
        assert_eq!(f.update(&pair(a, b), DT, &s, &r).kind, ShotKind::Shared);
        // One of the pair down: the other's own view.
        b.live = false;
        let shot = f.update(&pair(a, b), DT, &s, &r);
        assert_eq!((shot.kind, shot.seat), (ShotKind::Seat, Some(0)));
    }

    #[test]
    fn no_seat_holds_the_view_in_the_field() {
        let s = stage();
        let mut f = Follow::default();
        let shot = f.update(&Seats::default(), DT, &s, &rules());
        assert_eq!((shot.kind, shot.seat, shot.cut), (ShotKind::Nobody, None, false));
        assert_eq!(shot.center, Vec2::new(768.0, 384.0));
    }

    #[test]
    fn a_rounds_seats_are_read_from_the_game() {
        let mut game = Game::default();
        game.map = crate::map::MapFile::from_toml_str("version = 1\nsize = [48, 24]\n").unwrap();
        game.enemy_count_override = Some(0);
        game.players = crate::simulation::PlayerCount::TWO;
        let (w, h) = game.map.field_size();
        game.init(w, h);
        let seats = Seats::of(&game, &[0, 1, 5]);
        assert_eq!(seats.local, vec![0, 1], "a seat the round does not hold is left out");
        assert_eq!(seats.tanks.len(), 2);
        for t in &seats.tanks {
            let (position, rotation, _) = game.seat_motion(t.seat).unwrap();
            assert_eq!(t.position, position);
            assert_eq!(t.facing, cardinal(rotation));
            assert!(t.live && t.top_speed > 0.0);
        }
    }
}
