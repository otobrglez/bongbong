//! Shots through portals (docs/teleporting.md, "Shots"). A shell, bullet
//! or plasma bolt whose path this frame passes within
//! `portal_shot_radius` of an active portal's anchor goes in at the point
//! of its path nearest the anchor and comes out of another portal at the
//! same offset from that one's anchor, on the same heading at the same
//! speed. The exit is one draw from the round's RNG, uniform among the
//! other portals, as a tank's is - and only where a shot goes in, so a
//! round in which no shot meets a portal draws exactly what it would on a
//! map with none. A laser beam is bent the same way, leg by leg, for the
//! reach it has left (`Game::resolve_lasers`).
//!
//! A shot is judged up to the point it goes in: whatever it meets before
//! the anchor's nearest point stops it there, and nothing past it on that
//! side does. It comes out a hair past the exit's nearest point, heading
//! away from that anchor. A shot that starts inside a portal's swirl - one
//! that just came out of it, or one fired by a tank standing on it - is
//! *leaving* that portal and cannot go into it until it has left the
//! swirl (`ShotPortals::leaving`), the way a tank on its portal cooldown
//! drives off the portal it stands on. `portal_shot_max_passes` bounds the
//! passes of one shot or beam: two portals lined up on a shot's heading
//! would otherwise hand it back and forth for ever. Missiles fly over
//! everything and are left alone.
//!
//! The geometry (`shot_entry`, `exit_point`, `inside`) is pure, so the
//! client's drawing (`simulation::present`, `net::predict`) stops a shot
//! it draws in the present where the room puts it through.

use rand::RngExt;

use crate::Position;
use crate::math::Vec2;
use crate::tuning::tuning;

use super::weapons::Projectile;
use super::{Event, Frame, Game};

/// How far past the exit's nearest point a shot comes out, along its
/// heading: enough that rounding can never put it a hair short of that
/// point.
const EXIT_CLEARANCE_PX: f32 = 0.05;

/// A projectile's dealings with portals: a component beside it (the
/// `Rewind` pattern), attached only to a shot fired inside a portal's
/// swirl or on its first pass, so a shot that never meets a portal spawns
/// and flies exactly as it would on a map with none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ShotPortals {
    /// Portals it has passed through (`portal_shot_max_passes`).
    pub passes: u8,
    /// The portal whose swirl it is still inside - the one it came out of,
    /// or the one it was fired inside - which it cannot go into until it
    /// has left the swirl.
    pub leaving: Option<usize>,
}

/// Where a shot flying `p0..p1` goes into a portal: the first of
/// `portals` but `leaving` whose anchor the segment passes within `radius`
/// of, at the point of the segment nearest that anchor - the portal's
/// index and that point. Only a shot still closing on an anchor goes in
/// (the nearest point is past `p0`), so one that came out of a portal -
/// which leaves heading away from it - is not taken back. Ties go to the
/// lower index.
pub fn shot_entry(p0: Position, p1: Position, portals: &[Position], radius: f32, leaving: Option<usize>) -> Option<(usize, Position)> {
    let d = p1 - p0;
    let len = d.length();
    if len <= f32::EPSILON {
        return None;
    }
    let u = Vec2::new(d.x / len, d.y / len);
    let mut best: Option<(f32, usize)> = None;
    for (i, &anchor) in portals.iter().enumerate() {
        if leaving == Some(i) {
            continue;
        }
        let s = (anchor - p0).dot(u);
        if s <= 0.0 || s > len {
            continue;
        }
        let nearest = p0 + u * s;
        if nearest.distance_to(anchor) > radius {
            continue;
        }
        if best.is_none_or(|(b, _)| s < b) {
            best = Some((s, i));
        }
    }
    best.map(|(s, i)| (i, p0 + u * s))
}

/// The portal whose swirl `p` stands inside (within `radius` of its
/// anchor), if any: the first in order.
pub fn inside(p: Position, portals: &[Position], radius: f32) -> Option<usize> {
    portals.iter().position(|&anchor| p.distance_to(anchor) <= radius)
}

/// Where a shot heading along `dir` (a unit vector) that went into the
/// portal at `entrance` at `at` comes out of the portal at `exit`: the same
/// offset from that anchor, a hair along its heading.
pub fn exit_point(entrance: Position, exit: Position, at: Position, dir: Vec2) -> Position {
    exit + (at - entrance) + dir * EXIT_CLEARANCE_PX
}

impl Game {
    /// The portals shots pass through: the active network while
    /// `portal_shots` is on, else none.
    pub(crate) fn shot_portals(&self) -> &[Position] {
        if tuning().portal_shots { self.active_portals() } else { &[] }
    }

    /// The portal a shot that went into `entrance` comes out of: one draw
    /// from the round's RNG, uniform among the others. Only called with an
    /// active network, so there is always another.
    pub(super) fn draw_shot_exit(&self, f: &mut Frame, entrance: usize) -> usize {
        let k = f.rng.random_range(0..self.portals.len() - 1);
        if k >= entrance { k + 1 } else { k }
    }

    /// A shot just spawned at `at` inside a portal's swirl is leaving that
    /// portal: the component that says so, or `None` (nearly always) for
    /// one fired in the open.
    pub(super) fn fired_inside_portal(&self, at: Position) -> Option<ShotPortals> {
        let leaving = inside(at, self.shot_portals(), tuning().portal_shot_radius)?;
        Some(ShotPortals { passes: 0, leaving: Some(leaving) })
    }

    /// Put projectile `entity`, which went into portal `entrance` at `at`
    /// flying at `velocity`, out of another portal (`draw_shot_exit`):
    /// moved there with its heading, speed, owner and everything else
    /// kept, its passes counted, leaving the exit, and
    /// `Event::ShotTeleported` logged.
    pub(super) fn shot_through<P: Projectile>(&mut self, f: &mut Frame, entity: hecs::Entity, entrance: usize, at: Position, velocity: Vec2) {
        let speed = velocity.length();
        if speed <= f32::EPSILON {
            return;
        }
        let dir = Vec2::new(velocity.x / speed, velocity.y / speed);
        let exit = self.draw_shot_exit(f, entrance);
        let to = exit_point(self.portals[entrance], self.portals[exit], at, dir);
        let id = {
            let mut q = self.world.query_one::<&mut P>(entity);
            let Ok(p) = q.get() else { return };
            p.set_position(to);
            p.begin_frame();
            p.id()
        };
        let passes = self.world.get::<&ShotPortals>(entity).map_or(0, |s| s.passes);
        self.world.insert_one(entity, ShotPortals { passes: passes.saturating_add(1), leaving: Some(exit) }).ok();
        f.events.push(Event::ShotTeleported { id: Some(id), x: at.x, y: at.y, to_x: to.x, to_y: to.y });
    }

    /// A shot at `position` that is leaving a portal and has now left its
    /// swirl may go into it again.
    pub(super) fn note_left_portal(&mut self, entity: hecs::Entity, position: Position) {
        let Ok(mut state) = self.world.get::<&mut ShotPortals>(entity) else { return };
        let Some(i) = state.leaving else { return };
        if self.portals.get(i).is_none_or(|&anchor| position.distance_to(anchor) > tuning().portal_shot_radius) {
            state.leaving = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORTALS: [Position; 2] = [Position::new(320.0, 352.0), Position::new(960.0, 352.0)];

    #[test]
    fn a_shot_goes_in_at_the_point_of_its_path_nearest_the_anchor() {
        let (i, at) = shot_entry(Position::new(300.0, 362.0), Position::new(330.0, 362.0), &PORTALS, 28.0, None).expect("passes 10 px from A");
        assert_eq!(i, 0);
        assert!((at.x - 320.0).abs() < 1e-3 && (at.y - 362.0).abs() < 1e-3, "{at:?}");
    }

    #[test]
    fn a_shot_that_has_not_reached_the_nearest_point_or_passes_wide_stays_out() {
        assert!(shot_entry(Position::new(280.0, 352.0), Position::new(310.0, 352.0), &PORTALS, 28.0, None).is_none(), "short of the anchor");
        assert!(shot_entry(Position::new(300.0, 400.0), Position::new(340.0, 400.0), &PORTALS, 28.0, None).is_none(), "48 px wide");
        assert!(shot_entry(Position::new(330.0, 352.0), Position::new(360.0, 352.0), &PORTALS, 28.0, None).is_none(), "already past it");
    }

    #[test]
    fn a_shot_leaving_a_portal_is_not_taken_by_it() {
        let (p0, p1) = (Position::new(310.0, 352.0), Position::new(330.0, 352.0));
        assert!(shot_entry(p0, p1, &PORTALS, 28.0, None).is_some(), "inside the swirl and closing on the anchor: in");
        assert!(shot_entry(p0, p1, &PORTALS, 28.0, Some(0)).is_none(), "unless it is leaving that portal");
        assert_eq!(inside(p0, &PORTALS, 28.0), Some(0));
        assert_eq!(inside(Position::new(640.0, 352.0), &PORTALS, 28.0), None);
    }

    #[test]
    fn a_shot_comes_out_heading_away_and_is_not_taken_back() {
        let dir = Vec2::new(1.0, 0.0);
        let at = Position::new(320.0, 362.0);
        let out = exit_point(PORTALS[0], PORTALS[1], at, dir);
        assert!((out.y - 362.0).abs() < 1e-3 && out.x > 960.0, "{out:?}");
        assert!(shot_entry(out, out + dir * 12.0, &PORTALS, 28.0, None).is_none(), "the exit does not take it back");
        // Lined up, the next portal along does take it.
        let back = Vec2::new(-1.0, 0.0);
        let out = exit_point(PORTALS[0], PORTALS[1], at, back);
        let (i, _) = shot_entry(out, out + back * 700.0, &PORTALS, 28.0, Some(1)).expect("A lies ahead");
        assert_eq!(i, 0);
    }
}
