//! Spent cases (docs/BULLETS_SPEC.md §9): the brass a minigun throws out
//! of its port, one a round. A case leaves sideways off the barrel's line
//! and a little back, tumbles on a low arc over its own shadow, bounces
//! once and lies where it stopped for `casing_linger_seconds`, then
//! dissolves block by block.
//!
//! Presentation only, owned by `fx.rs`, which throws one for every bullet
//! it first sees at its gun (`Fx::watch_casings`): it uses `rand::rng()`,
//! nothing that plays reads it and no snapshot carries it. Whole 2 px
//! blocks in the brass of the palette, as `pieces.rs` draws a wall's.

use rand::RngExt;

use crate::canvas::Canvas;
use crate::math::{Color, Vec2};
use crate::pyro::{bayer, block_of, BLOCK};
use crate::tuning::tuning;
use crate::Position;

/// A case in the air: its lit half and its shaded one (GOLD_BRIGHT,
/// WOOD_AMBER).
const FLYING: [Color; 2] = [Color::new(0xEE, 0xA3, 0x43, 255), Color::new(0xB5, 0x7A, 0x28, 255)];

/// A case lying spent, a step down so it sits in the ground rather than
/// on it (WOOD_AMBER, WOOD_DK).
const LYING: [Color; 2] = [Color::new(0xB5, 0x7A, 0x28, 255), Color::new(0x99, 0x65, 0x24, 255)];

/// How high over the ground the port is, px.
const PORT_HEIGHT: f32 = 8.0;

/// The last seconds of its lie a case takes to dissolve.
const DISSOLVE_SECONDS: f32 = 1.0;

/// One case, flying or lying.
#[derive(Clone, Copy, Debug)]
pub struct Casing {
    pos: Position,
    vel: Vec2,
    /// Height over the ground, px; up is positive.
    z: f32,
    vz: f32,
    bounces: u8,
    age: f32,
    /// Seconds it has lain still.
    rest: f32,
    /// Lying across the screen rather than up it: rolled as it is thrown,
    /// and what its tumble ends on.
    across: bool,
}

impl Casing {
    fn lying(&self) -> bool {
        self.z <= 0.0 && self.vz == 0.0
    }
}

/// Every spent case in the round.
#[derive(Default)]
pub struct Casings {
    list: Vec<Casing>,
}

impl Casings {
    pub fn clear(&mut self) {
        self.list.clear();
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// One case out of the port at `port`, of a gun firing along `barrel`
    /// (a unit vector): thrown to the barrel's right for `side` 1, its left
    /// for -1, at about `casing_eject_speed`.
    pub fn eject(&mut self, port: Position, barrel: Vec2, side: f32) {
        let t = tuning();
        let cap = t.fx_max_casings.max(0) as usize;
        if cap == 0 {
            return;
        }
        let mut rng = rand::rng();
        // Off the barrel's line and a little back, fanned.
        let out = Vec2::new(-barrel.y, barrel.x) * side - barrel * 0.35;
        let angle = crate::math::atan2(out.y, out.x) + rng.random_range(-0.4..0.4);
        let speed = t.casing_eject_speed * rng.random_range(0.6..1.3);
        let case = Casing {
            pos: port,
            vel: Vec2::new(crate::math::cos(angle) * speed, crate::math::sin(angle) * speed),
            z: PORT_HEIGHT,
            vz: rng.random_range(70.0..140.0),
            bounces: 0,
            age: 0.0,
            rest: 0.0,
            across: rng.random_bool(0.5),
        };
        if self.list.len() >= cap {
            // The oldest lying case makes room; failing that, the oldest
            // of all.
            let at = self.list.iter().position(Casing::lying).unwrap_or(0);
            self.list.remove(at);
        }
        self.list.push(case);
    }

    pub fn tick(&mut self, dt: f32) {
        let t = tuning();
        let (gravity, drag, bounce) = (t.debris_gravity, t.debris_air_drag, t.debris_bounce);
        let linger = t.casing_linger_seconds;
        self.list.retain_mut(|c| {
            c.age += dt;
            if c.lying() {
                c.rest += dt;
                return c.rest < linger;
            }
            c.vz -= gravity * dt;
            c.z += c.vz * dt;
            if c.z <= 0.0 {
                c.z = 0.0;
                if c.vz < -60.0 && c.bounces < 2 {
                    c.vz = -c.vz * bounce;
                    c.vel = c.vel * 0.5;
                    c.bounces += 1;
                } else {
                    c.vz = 0.0;
                    c.vel = Vec2::zero();
                }
            }
            let k = crate::math::exp(-drag * dt);
            c.vel = c.vel * k;
            c.pos.x += c.vel.x * dt;
            c.pos.y += c.vel.y * dt;
            true
        });
    }

    /// Draw the cases lying on the ground (`airborne` false: on the floor,
    /// under everything that stands) or the ones in the air over their
    /// shadows (`airborne` true: over the tiles and the tanks). A case is
    /// two 2 px blocks end to end; in the air it turns end over end a
    /// quarter at a time, and over its last second on the ground it
    /// dissolves on the field's Bayer grid.
    pub fn draw(&self, c: &mut impl Canvas, airborne: bool) {
        let linger = tuning().casing_linger_seconds;
        let b = BLOCK as i32;
        for case in self.list.iter().filter(|p| p.lying() != airborne) {
            if c.culls(case.pos) {
                continue;
            }
            let (x, y) = (crate::pyro::snap(case.pos.x), crate::pyro::snap(case.pos.y));
            if airborne {
                let across = case.across ^ ((case.age * 16.0) as i32 % 2 == 1);
                let (dx, dy) = if across { (b, 0) } else { (0, b) };
                let shadow = Color::new(0x25, 0x25, 0x25, 60);
                c.fill_rect(x, y, b + dx, b + dy, shadow);
                let lift = crate::pyro::snap(case.z);
                c.fill_rect(x, y - lift, b, b, FLYING[0]);
                c.fill_rect(x + dx, y - lift + dy, b, b, FLYING[1]);
            } else {
                // How far gone it is, 0 until its last second.
                let gone = (1.0 - (linger - case.rest) / DISSOLVE_SECONDS).clamp(0.0, 1.0);
                let (dx, dy) = if case.across { (b, 0) } else { (0, b) };
                for (k, (bx, by)) in [(x, y), (x + dx, y + dy)].into_iter().enumerate() {
                    let (gx, gy) = block_of(bx as f32, by as f32);
                    if bayer(gx, gy) >= gone {
                        c.fill_rect(bx, by, b, b, LYING[k]);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    /// A case leaves to the side it is thrown to, lands clear of the port,
    /// lies there, and is gone after its linger.
    #[test]
    fn a_case_flies_out_lands_lies_and_goes() {
        let mut cases = Casings::default();
        let port = Position::new(100.0, 100.0);
        // Firing up the screen: the barrel's right is +x.
        for _ in 0..20 {
            cases.eject(port, Vec2::new(0.0, -1.0), 1.0);
        }
        assert_eq!(cases.len(), 20);
        assert!(cases.list.iter().all(|c| !c.lying()), "every case starts in the air");
        for _ in 0..90 {
            cases.tick(DT);
        }
        assert!(cases.list.iter().all(Casing::lying), "a second and a half in, every case lies still");
        for case in &cases.list {
            assert!(case.pos.x > port.x + 4.0, "thrown to the barrel's right: {case:?}");
            assert!(case.pos.distance_to(port) < 96.0, "and not far: {case:?}");
        }
        for _ in 0..(60.0 * tuning().casing_linger_seconds) as i32 + 2 {
            cases.tick(DT);
        }
        assert!(cases.is_empty(), "nothing is left lying");
    }

    /// The left side throws them the other way.
    #[test]
    fn the_side_picks_which_way_a_case_leaves() {
        let mut cases = Casings::default();
        let port = Position::new(100.0, 100.0);
        for _ in 0..20 {
            cases.eject(port, Vec2::new(0.0, -1.0), -1.0);
        }
        for _ in 0..90 {
            cases.tick(DT);
        }
        assert!(cases.list.iter().all(|c| c.pos.x < port.x - 4.0));
    }

    /// At the cap the oldest lying case makes room for a new one.
    #[test]
    fn cases_keep_to_their_cap() {
        let mut cases = Casings::default();
        let cap = tuning().fx_max_casings as usize;
        for _ in 0..cap + 25 {
            cases.eject(Position::new(0.0, 0.0), Vec2::new(1.0, 0.0), 1.0);
        }
        assert_eq!(cases.len(), cap);
    }
}
