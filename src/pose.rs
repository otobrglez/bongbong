//! Render interpolation: the pose a moving thing is drawn at between two
//! simulation steps. Headless and presentation only.
//!
//! The round advances in whole steps of `PHYSICS_FIXED_DT` while the
//! display shows frames at its own rate, so a frame usually falls between
//! two steps. Every tank, projectile and bong keeps a `Smooth`: the pose
//! it had when the step began (`begin_step`, called by `Game::update`
//! before anything moves) and the pose the frame draws
//! (`Game::pose_at(alpha)`, called by the frame loop once the frame's
//! steps have run, `alpha` being the share of the next step the clock
//! already holds). The drawing reads `drawn`, never the fields.
//!
//! Nothing that plays reads any of it: not the RNG, the events, the wire,
//! a hit test or an AI snapshot. `drawn` answers the live pose unless
//! `show` ran for exactly that pose, so a round nobody posed - a
//! thumbnail, a test, the dev server's lockstep, a room's replica - draws
//! each step where it stands, and so does everything inside `update`.

use crate::math::Vec2;

/// Where something is drawn: its position, for the things that turn the
/// hull's drawn angle and the turret's in degrees, and for the things
/// that fly their height over the ground in px.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    pub position: Vec2,
    pub rotation: f32,
    pub turret: f32,
    pub lift: f32,
}

impl Pose {
    /// A pose that only has a place.
    pub fn at(position: Vec2) -> Pose {
        Pose { position, rotation: 0.0, turret: 0.0, lift: 0.0 }
    }

    /// A place and one angle.
    pub fn facing(position: Vec2, rotation: f32) -> Pose {
        Pose { position, rotation, turret: 0.0, lift: 0.0 }
    }
}

/// The angle `alpha` of the way from `from` to `to` along the shorter
/// arc, in degrees. `alpha` 1 is `to` itself, bit for bit.
pub fn lerp_angle(from: f32, to: f32, alpha: f32) -> f32 {
    if alpha >= 1.0 {
        return to;
    }
    let mut diff = (to - from) % 360.0;
    if diff > 180.0 {
        diff -= 360.0;
    } else if diff < -180.0 {
        diff += 360.0;
    }
    from + diff * alpha
}

/// The pose `alpha` of the way from `prev` to `current`: positions on the
/// straight line, angles along the shorter arc. `alpha` is clamped to
/// 0..=1, and 1 is `current` itself, bit for bit.
pub fn lerp(prev: Pose, current: Pose, alpha: f32) -> Pose {
    let alpha = if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 1.0 };
    if alpha >= 1.0 {
        return current;
    }
    Pose {
        position: Vec2::new(
            prev.position.x + (current.position.x - prev.position.x) * alpha,
            prev.position.y + (current.position.y - prev.position.y) * alpha,
        ),
        rotation: lerp_angle(prev.rotation, current.rotation, alpha),
        turret: lerp_angle(prev.turret, current.turret, alpha),
        lift: prev.lift + (current.lift - prev.lift) * alpha,
    }
}

/// The farthest a tank or a bong moves in one step and is still drawn
/// gliding there, in px: past it the move is a jump (a portal, a
/// placement) and the picture goes with it.
pub const BODY_JUMP_PX: f32 = 24.0;

/// `BODY_JUMP_PX` for a projectile, whose honest step is longer.
pub const SHOT_JUMP_PX: f32 = 96.0;

/// One thing's interpolation state. `Default` is a thing with no past: it
/// is drawn where it stands until a step begins with it in the world.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Smooth {
    /// The pose when the running step began; `None` for a thing made or
    /// moved by a jump since, which has nowhere to glide from.
    prev: Option<Pose>,
    /// What `show` answered, and the live pose it answered for.
    shown: Pose,
    of: Pose,
    /// What `show` took off the position to land it on a whole texel.
    residual: Vec2,
}

impl Smooth {
    /// A simulation step begins with the thing at `current`.
    pub fn begin_step(&mut self, current: Pose) {
        self.prev = Some(current);
        self.shown = current;
        self.of = current;
        self.residual = Vec2::new(0.0, 0.0);
    }

    /// The thing jumped (a portal, a respawn, a placement): it is drawn
    /// where it lands, with no glide from where it was.
    pub fn cut(&mut self) {
        self.prev = None;
    }

    /// Work out the pose a frame draws, `alpha` of the way from the
    /// step's start to `current`. A move longer than `jump_px` is a jump
    /// and draws at `current`. `texel` lands the position on a whole
    /// world pixel and keeps what that took off as `residual`.
    pub fn show(&mut self, current: Pose, alpha: f32, jump_px: f32, texel: bool) {
        let mut pose = match self.prev {
            Some(prev) if !jumped(prev, current, jump_px) => lerp(prev, current, alpha),
            _ => current,
        };
        self.residual = Vec2::new(0.0, 0.0);
        if texel {
            let whole = Vec2::new(pose.position.x.round(), pose.position.y.round());
            if whole.x.is_finite() && whole.y.is_finite() {
                self.residual = Vec2::new(pose.position.x - whole.x, pose.position.y - whole.y);
                pose.position = whole;
            }
        }
        self.shown = pose;
        self.of = current;
    }

    /// The pose to draw a thing whose live pose is `current`: what `show`
    /// worked out for it, or `current` itself when `show` has not run
    /// since the thing last moved.
    pub fn drawn(&self, current: Pose) -> Pose {
        if self.of == current { self.shown } else { current }
    }

    /// The drawn position less the whole texel it was landed on, zero
    /// unless `show` snapped this very pose.
    pub fn residual(&self, current: Pose) -> Vec2 {
        if self.of == current { self.residual } else { Vec2::new(0.0, 0.0) }
    }
}

fn jumped(prev: Pose, current: Pose, jump_px: f32) -> bool {
    let (dx, dy) = (current.position.x - prev.position.x, current.position.y - prev.position.y);
    !(dx * dx + dy * dy <= jump_px * jump_px)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(x: f32, y: f32, rotation: f32, turret: f32) -> Pose {
        Pose { position: Vec2::new(x, y), rotation, turret, lift: 0.0 }
    }

    #[test]
    fn a_pose_is_drawn_part_way_along_the_step() {
        let (a, b) = (pose(10.0, 20.0, 0.0, 0.0), pose(12.0, 16.0, 30.0, 60.0));
        assert_eq!(lerp(a, b, 0.0), a);
        assert_eq!(lerp(a, b, 0.5), pose(11.0, 18.0, 15.0, 30.0));
        assert_eq!(lerp(a, b, 1.0), b, "alpha 1 is the live pose itself");
        assert_eq!(lerp(a, b, 7.0), b, "and so is anything past it");
        assert_eq!(lerp(a, b, -1.0), a);
        assert_eq!(lerp(a, b, f32::NAN), b);
        let (low, high) = (Pose { lift: 10.0, ..a }, Pose { lift: 30.0, ..a });
        assert_eq!(lerp(low, high, 0.25).lift, 15.0);
    }

    #[test]
    fn an_angle_turns_along_the_shorter_arc() {
        assert_eq!(lerp_angle(350.0, 10.0, 0.5), 360.0, "through north, not the long way round");
        assert_eq!(lerp_angle(10.0, 350.0, 0.5), 0.0);
        assert_eq!(lerp_angle(0.0, 270.0, 0.5), -45.0, "a left turn goes left");
        assert_eq!(lerp_angle(-90.0, 90.0, 0.25), -45.0);
        assert_eq!(lerp_angle(350.0, 10.0, 1.0), 10.0);
        // An eased angle that wrapped past a whole turn.
        assert_eq!(lerp_angle(359.0, 1.0, 0.5), 360.0);
        assert_eq!(lerp_angle(-359.0, 1.0, 0.5), -359.0, "a whole turn apart is no turn");
    }

    #[test]
    fn a_thing_nobody_showed_is_drawn_where_it_stands() {
        let smooth = Smooth::default();
        let live = pose(5.5, 6.5, 90.0, 80.0);
        assert_eq!(smooth.drawn(live), live);
        assert_eq!(smooth.residual(live), Vec2::new(0.0, 0.0));
    }

    #[test]
    fn a_shown_thing_glides_from_where_the_step_began() {
        let mut smooth = Smooth::default();
        let (a, b) = (pose(0.0, 0.0, 0.0, 0.0), pose(2.0, 0.0, 0.0, 0.0));
        smooth.begin_step(a);
        assert_eq!(smooth.drawn(a), a);
        assert_eq!(smooth.drawn(b), b, "inside the step the live pose is the answer");
        smooth.show(b, 0.25, BODY_JUMP_PX, false);
        assert_eq!(smooth.drawn(b), pose(0.5, 0.0, 0.0, 0.0));
        // It moves on unseen: the old answer is not for this pose.
        let c = pose(4.0, 0.0, 0.0, 0.0);
        assert_eq!(smooth.drawn(c), c);
        // The next step begins: everything inside it reads the live pose.
        smooth.begin_step(b);
        assert_eq!(smooth.drawn(b), b);
    }

    #[test]
    fn a_jump_is_drawn_where_it_lands() {
        let (a, far) = (pose(0.0, 0.0, 0.0, 0.0), pose(400.0, 0.0, 180.0, 180.0));
        // A jump the simulation named.
        let mut smooth = Smooth::default();
        smooth.begin_step(a);
        smooth.cut();
        let near = pose(3.0, 0.0, 0.0, 0.0);
        smooth.show(near, 0.5, BODY_JUMP_PX, false);
        assert_eq!(smooth.drawn(near), near);
        // One it did not: too far for a step.
        let mut smooth = Smooth::default();
        smooth.begin_step(a);
        smooth.show(far, 0.5, BODY_JUMP_PX, false);
        assert_eq!(smooth.drawn(far), far);
        // Something made inside the step has no past.
        let mut smooth = Smooth::default();
        smooth.show(far, 0.5, BODY_JUMP_PX, false);
        assert_eq!(smooth.drawn(far), far);
        // A step later it glides again.
        smooth.begin_step(far);
        let on = pose(402.0, 0.0, 180.0, 180.0);
        smooth.show(on, 0.5, BODY_JUMP_PX, false);
        assert_eq!(smooth.drawn(on), pose(401.0, 0.0, 180.0, 180.0));
    }

    #[test]
    fn a_texel_anchor_keeps_what_it_dropped() {
        let mut smooth = Smooth::default();
        let (a, b) = (pose(10.0, 20.0, 0.0, 0.0), pose(11.5, 20.5, 0.0, 0.0));
        smooth.begin_step(a);
        smooth.show(b, 0.5, BODY_JUMP_PX, true);
        assert_eq!(smooth.drawn(b).position, Vec2::new(11.0, 20.0));
        assert_eq!(smooth.residual(b), Vec2::new(-0.25, 0.25));
        smooth.show(b, 1.0, BODY_JUMP_PX, false);
        assert_eq!(smooth.drawn(b), b);
        assert_eq!(smooth.residual(b), Vec2::new(0.0, 0.0));
    }
}
