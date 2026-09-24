//! Seeker missiles (`pickup::PickupKind::Missiles`, `Tank::missile_ammo`):
//! a four-tube pod on the turret fires a volley, one missile per tube a
//! beat apart (`Tank::missile_volley`), and each missile flies in three
//! stages:
//!
//! 1. **Climb** - up out of its tube along the launcher's heading, fanned
//!    a few degrees per tube, rising to `missile_apex_height`.
//! 2. **Seek** - a short hang at the apex while `Game::guide_missiles`
//!    locks it onto the nearest opposing tank within `missile_seek_range`,
//!    or, with none in range, onto the ground point the launcher was aimed
//!    at.
//! 3. **Chase** - it accelerates after the target with a limited (and
//!    growing) turn rate, following it while it moves, and comes down over
//!    the last `missile_dive_distance`. Inside `missile_commit_distance` it
//!    stops tracking and **dives** on the last spot it saw, bursting there
//!    in a small blast (`Game::resolve_missiles`).
//!
//! A missile is in the air the whole way, so it has no hit test at all:
//! walls, props, trees and tanks under it are flown over, and only the
//! blast at the end touches anything. Hiding behind a wall does not help;
//! moving when the dive commits does.
//!
//! The simulation works on the missile's *ground point* (`position`) and a
//! separate `height`; the presentation lifts the sprite by the height and
//! leaves its shadow on the ground, the convention `FlyingDrum` and the
//! thrown decals already use. No RNG anywhere in flight.

use crate::tuning::tuning;
use sola_raylib::prelude::*;

use crate::shell::Owner;
use crate::{MISSILE_FRAMES, MISSILE_SCALE, MISSILE_TEXTURE_SIZE, Position};

/// Where a missile is in its flight - see this module's doc comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissileStage {
    Climb,
    Seek,
    Chase,
    /// Committed: flying straight down onto `Missile::aim`, no longer
    /// following anything.
    Dive,
}

impl MissileStage {
    /// Lower-case name for tooling/JSON.
    pub fn name(self) -> &'static str {
        match self {
            MissileStage::Climb => "climb",
            MissileStage::Seek => "seek",
            MissileStage::Chase => "chase",
            MissileStage::Dive => "dive",
        }
    }
}

/// One seeker missile in the air.
pub struct Missile {
    pub stage: MissileStage,
    /// The point on the ground under the missile.
    pub position: Position,
    /// Height above `position` (px): 0 on the ground, `missile_apex_height`
    /// at the top of the climb.
    pub height: f32,
    /// Unit heading over the ground.
    pub dir: Vector2,
    /// Ground speed (px/s).
    pub speed: f32,
    /// Seconds since launch.
    pub age: f32,
    /// Seconds spent in the current stage.
    pub stage_time: f32,
    pub owner: Owner,
    /// The tank it is locked onto (set by `Game::guide_missiles`), if any.
    pub target: Option<hecs::Entity>,
    /// Where it is heading: the target's latest position while tracking,
    /// the frozen dive point once committed, the launcher's aim point
    /// until a lock is made.
    pub aim: Position,
    /// Set by `Game::guide_missiles` once the seek has picked (or failed
    /// to pick) a target; the chase starts only after that.
    pub locked: bool,
    /// Set the step the missile reaches `aim` on its dive; the frame's
    /// `resolve_missiles` bursts it and removes it.
    pub arrived: bool,
    /// Which tube it left (0 = leftmost): the fan offset, and a salt for
    /// the cosmetic flame flicker so a volley does not flicker in step.
    pub tube: u8,
}

impl Missile {
    /// A missile leaving tube `tube` at `origin` (the tube mouth), heading
    /// `dir` (unit, already fanned), aiming by default at `fallback_aim` -
    /// the ground point it dives on if the seek finds nothing.
    pub fn spawn(origin: Position, dir: Vector2, owner: Owner, tube: u8, fallback_aim: Position) -> Missile {
        Missile {
            stage: MissileStage::Climb,
            position: origin,
            height: 0.0,
            dir,
            speed: tuning().missile_climb_speed,
            age: 0.0,
            stage_time: 0.0,
            owner,
            target: None,
            aim: fallback_aim,
            locked: false,
            arrived: false,
            tube,
        }
    }

    /// The seek stage has had its hang time and wants a target: what
    /// `Game::guide_missiles` looks for.
    pub fn wants_lock(&self) -> bool {
        self.stage == MissileStage::Seek && !self.locked
    }

    /// Still following a target, so its position should refresh `aim`.
    pub fn tracking(&self) -> bool {
        self.target.is_some() && matches!(self.stage, MissileStage::Seek | MissileStage::Chase)
    }

    fn enter(&mut self, stage: MissileStage) {
        self.stage = stage;
        self.stage_time = 0.0;
    }

    /// Ground distance to `aim`.
    pub fn distance_to_aim(&self) -> f32 {
        self.position.distance_to(self.aim)
    }

    /// Advance one fixed step. Pure geometry against `aim`; locking and
    /// tracking are `Game::guide_missiles`' job between frames.
    pub fn advance(&mut self, dt: f32) {
        if self.arrived {
            return;
        }
        let t = tuning();
        self.age += dt;
        self.stage_time += dt;
        match self.stage {
            MissileStage::Climb => {
                let k = (self.stage_time / t.missile_climb_seconds.max(1e-3)).clamp(0.0, 1.0);
                // Ease out: it leaves the tube fast and rounds over at the top.
                self.height = t.missile_apex_height * (1.0 - (1.0 - k) * (1.0 - k));
                self.speed = t.missile_climb_speed;
                self.step(dt);
                if k >= 1.0 {
                    self.enter(MissileStage::Seek);
                }
            }
            MissileStage::Seek => {
                // Hanging at the top, nosing round toward whatever it is
                // about to go after.
                self.height = t.missile_apex_height;
                self.speed = (self.speed - t.missile_accel * 0.5 * dt).max(t.missile_climb_speed * 0.4);
                if self.locked {
                    self.turn_toward_aim(t.missile_turn_rate_deg, dt);
                }
                self.step(dt);
                if self.locked && self.stage_time >= t.missile_acquire_seconds {
                    self.enter(MissileStage::Chase);
                }
            }
            MissileStage::Chase => {
                self.speed = (self.speed + t.missile_accel * dt).min(t.missile_speed);
                let rate = t.missile_turn_rate_deg + t.missile_turn_rate_growth_deg * self.stage_time;
                self.turn_toward_aim(rate, dt);
                if self.distance_to_aim() <= t.missile_commit_distance {
                    self.commit(None);
                } else if self.age >= t.missile_max_flight_seconds {
                    let ahead = self.position + self.dir * t.missile_commit_distance;
                    self.commit(Some(ahead));
                }
                self.fly_down(dt);
            }
            MissileStage::Dive => {
                self.speed = (self.speed + t.missile_accel * dt).min(t.missile_speed);
                self.fly_down(dt);
            }
        }
    }

    /// Stop tracking and dive on `at`, or on the current aim.
    fn commit(&mut self, at: Option<Position>) {
        if let Some(at) = at {
            self.aim = at;
        }
        self.target = None;
        self.enter(MissileStage::Dive);
    }

    /// Chase and dive: move toward the aim, lose height over the last
    /// `missile_dive_distance`, and arrive when this step would reach it.
    /// The dive points straight at its spot, so it cannot orbit it.
    fn fly_down(&mut self, dt: f32) {
        let t = tuning();
        if self.stage == MissileStage::Dive {
            let to = self.aim - self.position;
            let len = to.length();
            if len > 1e-3 {
                self.dir = to / len;
            }
        }
        let dist = self.distance_to_aim();
        if self.stage == MissileStage::Dive && dist <= self.speed * dt {
            self.position = self.aim;
            self.height = 0.0;
            self.arrived = true;
            return;
        }
        self.step(dt);
        let dist = self.distance_to_aim();
        let fall = (dist / t.missile_dive_distance.max(1.0)).clamp(0.0, 1.0);
        // Never climbs back up: a target running away does not lift it.
        self.height = self.height.min(t.missile_apex_height * fall);
    }

    fn step(&mut self, dt: f32) {
        self.position = self.position + self.dir * (self.speed * dt);
    }

    /// Rotate `dir` toward `aim` by at most `rate_deg` degrees per second.
    fn turn_toward_aim(&mut self, rate_deg: f32, dt: f32) {
        let to = self.aim - self.position;
        if to.length() < 1e-3 {
            return;
        }
        let want = to.y.atan2(to.x);
        let have = self.dir.y.atan2(self.dir.x);
        let mut delta = want - have;
        while delta > std::f32::consts::PI {
            delta -= std::f32::consts::TAU;
        }
        while delta < -std::f32::consts::PI {
            delta += std::f32::consts::TAU;
        }
        let max = rate_deg.to_radians() * dt;
        let turned = have + delta.clamp(-max, max);
        self.dir = Vector2::new(turned.cos(), turned.sin());
    }

    /// Facing in degrees, the game's convention (0 = up).
    pub fn rotation(&self) -> f32 {
        self.dir.x.atan2(-self.dir.y).to_degrees()
    }

    /// 0 on the ground .. 1 at the apex.
    pub fn lift(&self) -> f32 {
        (self.height / tuning().missile_apex_height.max(1.0)).clamp(0.0, 1.0)
    }

    /// Where the sprite is drawn: the ground point raised by the height.
    pub fn draw_pos(&self) -> Position {
        Position::new(self.position.x, self.position.y - self.height)
    }

    /// The exhaust end of the sprite at its drawn position - where the
    /// smoke trail comes out.
    pub fn tail(&self) -> Position {
        let back = MISSILE_TEXTURE_SIZE * MISSILE_SCALE * self.draw_scale() * 0.4;
        self.draw_pos() - self.dir * back
    }

    /// Nearer the camera at the top of the climb, so drawn bigger.
    pub fn draw_scale(&self) -> f32 {
        1.0 + (tuning().missile_apex_draw_scale - 1.0) * self.lift()
    }
}

/// The exhaust flicker frame: cycled from age, offset per tube.
fn frame(missile: &Missile) -> i32 {
    ((missile.age * 18.0) as i32 + missile.tube as i32) % MISSILE_FRAMES
}

fn source_rec(missile: &Missile) -> Rectangle {
    Rectangle::new(frame(missile) as f32 * MISSILE_TEXTURE_SIZE, 0.0, MISSILE_TEXTURE_SIZE, MISSILE_TEXTURE_SIZE)
}

/// The missile's shadow on the ground under it: its own silhouette,
/// shrinking and fading as it rises (what sells the height with no camera).
pub fn draw_missile_shadow(d: &mut impl RaylibDraw, texture: &Texture2D, missile: &Missile) {
    let lift = missile.lift();
    let size = MISSILE_TEXTURE_SIZE * MISSILE_SCALE * (1.0 - 0.3 * lift);
    let dest = Rectangle::new(
        missile.position.x + tuning().shadow_dir_x * 4.0,
        missile.position.y + tuning().shadow_dir_y * 4.0,
        size,
        size,
    );
    let alpha = 255.0 * tuning().missile_shadow_opacity * (1.0 - 0.5 * lift);
    let origin = Vector2::new(size / 2.0, size / 2.0);
    d.draw_texture_pro(texture, source_rec(missile), dest, origin, missile.rotation(), Color::new(0, 0, 0, alpha as u8));
}

/// The missile itself, lifted by its height and scaled up as it rises.
pub fn draw_missile(d: &mut impl RaylibDraw, texture: &Texture2D, missile: &Missile) {
    let size = MISSILE_TEXTURE_SIZE * MISSILE_SCALE * missile.draw_scale();
    let at = missile.draw_pos();
    let dest = Rectangle::new(at.x, at.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);
    d.draw_texture_pro(texture, source_rec(missile), dest, origin, missile.rotation(), Color::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fly_until_arrived(m: &mut Missile, max_steps: usize) -> usize {
        for i in 0..max_steps {
            if m.wants_lock() {
                m.locked = true;
            }
            m.advance(crate::PHYSICS_FIXED_DT);
            if m.arrived {
                return i;
            }
        }
        panic!("never arrived: {:?} at {:?} aim {:?}", m.stage, m.position, m.aim);
    }

    #[test]
    fn a_missile_climbs_seeks_chases_and_lands_on_its_aim() {
        let aim = Position::new(300.0, 100.0);
        let mut m = Missile::spawn(Position::new(100.0, 100.0), Vector2::new(0.0, -1.0), Owner::Player(0), 0, aim);
        let mut peak: f32 = 0.0;
        let mut stages = vec![m.stage];
        for _ in 0..2000 {
            if m.wants_lock() {
                m.locked = true;
            }
            m.advance(crate::PHYSICS_FIXED_DT);
            peak = peak.max(m.height);
            if stages.last() != Some(&m.stage) {
                stages.push(m.stage);
            }
            if m.arrived {
                break;
            }
        }
        assert!(m.arrived);
        assert_eq!(stages, vec![MissileStage::Climb, MissileStage::Seek, MissileStage::Chase, MissileStage::Dive]);
        assert!((peak - tuning().missile_apex_height).abs() < 1.0, "reached the apex: {peak}");
        assert_eq!(m.height, 0.0);
        assert!(m.position.distance_to(aim) < 1e-3);
    }

    #[test]
    fn a_target_directly_behind_the_launch_is_still_reached() {
        // Launched up, target straight down: the chase has to turn all
        // the way round without circling forever.
        let aim = Position::new(100.0, 400.0);
        let mut m = Missile::spawn(Position::new(100.0, 300.0), Vector2::new(0.0, -1.0), Owner::Enemy(3), 2, aim);
        fly_until_arrived(&mut m, 5000);
        assert!(m.position.distance_to(aim) < 1e-3);
    }

    #[test]
    fn a_missile_waits_at_the_apex_until_it_is_locked() {
        let mut m = Missile::spawn(Position::new(0.0, 0.0), Vector2::new(1.0, 0.0), Owner::Player(0), 0, Position::new(500.0, 0.0));
        for _ in 0..600 {
            m.advance(crate::PHYSICS_FIXED_DT);
        }
        assert_eq!(m.stage, MissileStage::Seek, "no lock, no chase");
        assert!(!m.arrived);
    }
}
