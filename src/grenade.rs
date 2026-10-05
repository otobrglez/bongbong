//! Grenades (`pickup::PickupKind::Grenades`, `Tank::grenade_ammo`): the
//! launcher's drum lobs a steel ball along the gun line, and it rolls -
//! bouncing off walls, tiles and the field's edge, and off hulls, which
//! hand it their own motion, so a tank driving into one shoves it along
//! and knocks it away. Its lamp blinks faster and faster across the fuse,
//! and after `grenade_fuse_seconds` it goes off in a blast with a
//! shockwave (`Game::resolve_grenades`).
//!
//! Like every other projectile it has no physics body: `Grenade::roll` is
//! its whole motion - drag, the hulls, then a sweep against the tiles and
//! walls - over plain boxes the world half (`simulation/grenades.rs`)
//! gathers, so a grenade never moves a tank and a round without one steps
//! exactly as before. No RNG.
//!
//! The drawing is generic over `canvas::Canvas`, in the effects language
//! (docs/effects.md): whole 2 px blocks in palette steps, the ball shaded
//! from the light's side, the lamp turning with the roll.

use crate::canvas::Canvas;
use crate::math::{Color, Vec2};
use crate::pickup::PickupKind;
use crate::pyro::{self, BLOCK};
use crate::shell::Owner;
use crate::tuning::{tuning, Tuning};
use crate::Position;

/// The ball's shades, dark to light: gunmetal off the palette's greys.
const BODY: [Color; 4] = [
    Color::new(0x25, 0x25, 0x2E, 255),
    Color::new(0x3E, 0x41, 0x4C, 255),
    Color::new(0x60, 0x65, 0x71, 255),
    Color::new(0x9C, 0xA3, 0xAD, 255),
];

/// The lamp while it is dark.
const LAMP_OFF: Color = Color::new(0x4A, 0x22, 0x21, 255);

/// One grenade on the ground.
pub struct Grenade {
    /// Per-round id from `Game::spawn_pending`'s one projectile counter,
    /// the key it travels under (`net::wire::GrenadeState`).
    pub id: u32,
    /// Whose launcher threw it: the side its blast hurts.
    pub owner: Owner,
    pub position: Position,
    pub velocity: Vec2,
    /// Seconds left before the blast.
    pub fuse: f32,
    /// Radians the ball has turned since launch (distance over radius),
    /// along `heading`: where its lamp is drawn. Presentation only.
    pub roll: f32,
    /// Unit direction it last rolled in. Presentation only.
    pub heading: Vec2,
}

/// What a grenade rolls among this tick: the boxes it bounces off.
#[derive(Default)]
pub struct Surroundings {
    /// Tiles and the field's walls, as (centre, half-extents).
    pub solids: Vec<(Position, Position)>,
    /// Hulls, as (centre, half-extents, velocity): a live tank's moving,
    /// a wreck's still.
    pub hulls: Vec<(Position, Position, Vec2)>,
}

/// What a grenade rolls over: the drag it feels, as a multiple of the
/// ground's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ground {
    Dry,
    Water,
    Ice,
}

impl Grenade {
    /// A grenade leaving the barrel at `muzzle` along `dir` (unit) at
    /// `grenade_launch_speed`, plus `carry` - the launching hull's share
    /// of its own velocity - with a full fuse.
    pub fn launch(muzzle: Position, dir: Vec2, carry: Vec2, owner: Owner) -> Self {
        let t = tuning();
        let velocity = clamp_speed(dir * t.grenade_launch_speed + carry, t.grenade_max_speed);
        Grenade { id: 0, owner, position: muzzle, velocity, fuse: t.grenade_fuse_seconds, roll: 0.0, heading: dir }
    }

    /// Whether the fuse has run out: the blast is due.
    pub fn spent(&self) -> bool {
        self.fuse <= 0.0
    }

    /// One tick of rolling, `dt` seconds: the fuse burns, the ground's
    /// drag slows it (to rest under `grenade_stop_speed`), any hull it
    /// overlaps pushes it out and hands it its own motion, then it moves
    /// with a sweep against `around.solids`, reflecting off the face it
    /// strikes. No RNG; walks the boxes in the order given.
    pub fn roll(&mut self, dt: f32, around: &Surroundings, ground: Ground) {
        let t = tuning();
        self.fuse -= dt;
        let factor = match ground {
            Ground::Dry => 1.0,
            Ground::Water => t.grenade_water_drag_factor,
            Ground::Ice => t.grenade_ice_drag_factor,
        };
        let keep = (1.0 - t.grenade_roll_drag * factor * dt).max(0.0);
        self.velocity = self.velocity * keep;
        if self.velocity.length() < t.grenade_stop_speed {
            self.velocity = Vec2::zero();
        }
        let r = t.grenade_radius;
        for &(center, half, moving) in &around.hulls {
            self.off_hull(center, half + Position::new(r, r), moving, &t);
        }
        self.velocity = clamp_speed(self.velocity, t.grenade_max_speed);
        let from = self.position;
        self.sweep(dt, &around.solids, r, t.grenade_wall_restitution);
        let moved = self.position - from;
        let distance = moved.length();
        if distance > 0.01 {
            self.heading = moved * (1.0 / distance);
            self.roll += distance / r;
        }
    }

    /// Out of a hull's box (grown by the radius) along the shallower axis,
    /// with the speed into the hull - relative to the hull's own - turned
    /// back by `grenade_tank_restitution`.
    fn off_hull(&mut self, center: Position, half: Position, moving: Vec2, t: &Tuning) {
        let d = self.position - center;
        let (px, py) = (half.x - d.x.abs(), half.y - d.y.abs());
        if px <= 0.0 || py <= 0.0 {
            return;
        }
        let normal = if px < py {
            self.position.x += px * sign(d.x);
            Vec2::new(sign(d.x), 0.0)
        } else {
            self.position.y += py * sign(d.y);
            Vec2::new(0.0, sign(d.y))
        };
        let relative = self.velocity - moving;
        let into = relative.x * normal.x + relative.y * normal.y;
        if into < 0.0 {
            self.velocity = self.velocity - normal * ((1.0 + t.grenade_tank_restitution) * into);
        }
    }

    /// Move `velocity * dt`, stopping at the first box (grown by `r`) the
    /// way crosses and reflecting off its face with `restitution`, at most
    /// a few times a tick. Starting inside a box - pushed into a wall by a
    /// hull - puts it out on the nearer face first.
    fn sweep(&mut self, dt: f32, solids: &[(Position, Position)], r: f32, restitution: f32) {
        let grow = Position::new(r, r);
        for &(center, half) in solids {
            let half = half + grow;
            let d = self.position - center;
            let (px, py) = (half.x - d.x.abs(), half.y - d.y.abs());
            if px > 0.0 && py > 0.0 {
                if px < py {
                    self.position.x += px * sign(d.x);
                    if self.velocity.x * sign(d.x) < 0.0 {
                        self.velocity.x = -self.velocity.x * restitution;
                    }
                } else {
                    self.position.y += py * sign(d.y);
                    if self.velocity.y * sign(d.y) < 0.0 {
                        self.velocity.y = -self.velocity.y * restitution;
                    }
                }
            }
        }
        let mut left = dt;
        for _ in 0..4 {
            if left <= 0.0 || self.velocity.length() <= 0.0 {
                break;
            }
            let to = self.position + self.velocity * left;
            let hit = solids
                .iter()
                .filter_map(|&(center, half)| entry(self.position, to, center, half + grow).map(|time| (time, center, half + grow)))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            let Some((time, center, half)) = hit else {
                self.position = to;
                break;
            };
            let at = self.position + (to - self.position) * time;
            // The face struck is the axis the grenade was further outside on.
            let dx = (self.position.x - center.x).abs() - half.x;
            let dy = (self.position.y - center.y).abs() - half.y;
            self.position = at;
            if dx > dy {
                self.position.x = center.x + (half.x + 0.01) * sign(self.position.x - center.x);
                self.velocity.x = -self.velocity.x * restitution;
                self.velocity.y *= restitution.max(0.85);
            } else {
                self.position.y = center.y + (half.y + 0.01) * sign(self.position.y - center.y);
                self.velocity.y = -self.velocity.y * restitution;
                self.velocity.x *= restitution.max(0.85);
            }
            left *= 1.0 - time;
        }
    }

    /// Whether the lamp is lit `elapsed` seconds into a fuse of `fuse`
    /// seconds: a blink whose rate rises steadily from
    /// `grenade_blink_hz_start` to `grenade_blink_hz_end` across it, lit
    /// for the first half of every cycle.
    pub fn lamp_lit_at(elapsed: f32, fuse: f32, t: &Tuning) -> bool {
        let (f0, f1) = (t.grenade_blink_hz_start, t.grenade_blink_hz_end);
        let e = elapsed.clamp(0.0, fuse.max(0.001));
        let cycles = f0 * e + (f1 - f0) * e * e / (2.0 * fuse.max(0.001));
        cycles.fract() < 0.5
    }

    /// Whether this grenade's lamp is lit now.
    pub fn lamp_lit(&self) -> bool {
        let t = tuning();
        Grenade::lamp_lit_at(t.grenade_fuse_seconds - self.fuse, t.grenade_fuse_seconds, &t)
    }

    /// Where its lamp is drawn: on the ball's face, turned by the roll
    /// along its heading, so a rolling grenade's lamp swings over it.
    pub fn lamp_point(&self) -> Position {
        let r = tuning().grenade_radius;
        let swing = self.roll.sin() * r * 0.55;
        Position::new(self.position.x + self.heading.x * swing, self.position.y + self.heading.y * swing - 1.0)
    }
}

fn sign(v: f32) -> f32 {
    if v < 0.0 { -1.0 } else { 1.0 }
}

fn clamp_speed(v: Vec2, max: f32) -> Vec2 {
    let len = v.length();
    if len > max && len > 0.0 { v * (max / len) } else { v }
}

/// The entry time (0..1] of the segment `p0..p1` into the box, if it
/// enters it this tick from outside.
fn entry(p0: Position, p1: Position, center: Position, half: Position) -> Option<f32> {
    let d = p1 - p0;
    let mut t_enter = f32::NEG_INFINITY;
    let mut t_exit = f32::INFINITY;
    for (p, dv, lo, hi) in [(p0.x, d.x, center.x - half.x, center.x + half.x), (p0.y, d.y, center.y - half.y, center.y + half.y)] {
        if dv.abs() < f32::EPSILON {
            if p <= lo || p >= hi {
                return None;
            }
        } else {
            let (mut a, mut b) = ((lo - p) / dv, (hi - p) / dv);
            if a > b {
                std::mem::swap(&mut a, &mut b);
            }
            t_enter = t_enter.max(a);
            t_exit = t_exit.min(b);
        }
    }
    (t_enter >= 0.0 && t_enter <= 1.0 && t_enter < t_exit).then_some(t_enter)
}

/// A grenade on the ground: its shadow, the ball shaded from the light's
/// side, a band round it, the lamp - lit in the crate's ink, or dark.
pub fn draw_grenade(c: &mut impl Canvas, grenade: &Grenade) {
    let t = tuning();
    let r = t.grenade_radius;
    let p = grenade.position;
    let shadow = Position::new(p.x + t.shadow_dir_x * 2.0, p.y + t.shadow_dir_y * 2.0);
    pyro::block_disc(c, shadow, r, Color::new(0, 0, 0, 80));
    pyro::block_disc(c, p, r, BODY[0]);
    pyro::block_disc(c, Position::new(p.x - BLOCK * 0.5, p.y - BLOCK * 0.5), r - BLOCK * 0.75, BODY[1]);
    pyro::block_disc(c, Position::new(p.x - BLOCK, p.y - BLOCK), r - BLOCK * 1.75, BODY[2]);
    let (hx, hy) = (pyro::snap(p.x - r * 0.45), pyro::snap(p.y - r * 0.55));
    c.fill_rect(hx - 2, hy - 2, 2, 2, BODY[3]);
    let lamp = grenade.lamp_point();
    let (lx, ly) = (pyro::snap(lamp.x), pyro::snap(lamp.y));
    let [_, base, light] = PickupKind::Grenades.ink();
    if grenade.lamp_lit() {
        c.fill_rect(lx - 2, ly - 2, 4, 2, base);
        c.fill_rect(lx - 2, ly - 2, 2, 2, light);
    } else {
        c.fill_rect(lx - 2, ly - 2, 4, 2, LAMP_OFF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ball(at: Position, v: Vec2) -> Grenade {
        Grenade { id: 1, owner: Owner::Player(0), position: at, velocity: v, fuse: 6.0, roll: 0.0, heading: Vec2::new(0.0, -1.0) }
    }

    const DT: f32 = crate::PHYSICS_FIXED_DT;

    #[test]
    fn a_rolling_grenade_slows_to_rest() {
        let mut g = ball(Position::new(100.0, 100.0), Vec2::new(200.0, 0.0));
        for _ in 0..600 {
            g.roll(DT, &Surroundings::default(), Ground::Dry);
        }
        assert_eq!(g.velocity, Vec2::zero(), "came to rest");
        assert!(g.position.x > 200.0, "it rolled a way first: {:?}", g.position);
        assert!(g.roll > 0.0);
    }

    #[test]
    fn it_bounces_off_a_wall_and_comes_back() {
        let wall = (Position::new(200.0, 100.0), Position::new(16.0, 16.0));
        let around = Surroundings { solids: vec![wall], hulls: vec![] };
        let mut g = ball(Position::new(150.0, 100.0), Vec2::new(300.0, 0.0));
        let mut furthest: f32 = 0.0;
        for _ in 0..60 {
            g.roll(DT, &around, Ground::Dry);
            furthest = furthest.max(g.position.x);
        }
        assert!(furthest <= 200.0 - 16.0 - tuning().grenade_radius + 0.1, "never inside the wall: {furthest}");
        assert!(g.velocity.x < 0.0 && g.position.x < 150.0, "bounced back: {:?} {:?}", g.position, g.velocity);
    }

    #[test]
    fn a_hull_driving_into_it_knocks_it_away() {
        let hull = (Position::new(100.0, 100.0), Position::new(14.0, 14.0), Vec2::new(120.0, 0.0));
        let around = Surroundings { solids: vec![], hulls: vec![hull] };
        let mut g = ball(Position::new(100.0 + 14.0 + 3.0, 100.0), Vec2::zero());
        g.roll(DT, &around, Ground::Dry);
        assert!(g.position.x >= 100.0 + 14.0 + tuning().grenade_radius, "pushed out of the hull: {:?}", g.position);
        assert!(g.velocity.x > 120.0, "knocked away faster than the hull drives: {:?}", g.velocity);
    }

    #[test]
    fn a_still_hull_is_a_wall() {
        let hull = (Position::new(100.0, 100.0), Position::new(14.0, 14.0), Vec2::zero());
        let around = Surroundings { solids: vec![], hulls: vec![hull] };
        let mut g = ball(Position::new(100.0, 100.0 - 14.0 - 4.0), Vec2::new(0.0, 100.0));
        g.roll(DT, &around, Ground::Dry);
        assert!(g.velocity.y < 0.0, "bounced off the parked hull: {:?}", g.velocity);
    }

    #[test]
    fn water_slows_it_and_ice_hardly_does() {
        let run = |ground| {
            let mut g = ball(Position::new(0.0, 0.0), Vec2::new(200.0, 0.0));
            for _ in 0..30 {
                g.roll(DT, &Surroundings::default(), ground);
            }
            g.position.x
        };
        let (dry, water, ice) = (run(Ground::Dry), run(Ground::Water), run(Ground::Ice));
        assert!(water < dry && dry < ice, "{water} < {dry} < {ice}");
    }

    #[test]
    fn the_fuse_burns_down_and_the_blink_quickens() {
        let t = Tuning::DEFAULT;
        let mut g = ball(Position::new(0.0, 0.0), Vec2::zero());
        let ticks = (t.grenade_fuse_seconds / DT).ceil() as usize;
        for _ in 0..ticks {
            assert!(!g.spent());
            g.roll(DT, &Surroundings::default(), Ground::Dry);
        }
        assert!(g.spent());
        // Count lamp changes in the first and the last second.
        let flips = |from: f32| {
            let mut n = 0;
            let mut was = Grenade::lamp_lit_at(from, t.grenade_fuse_seconds, &t);
            for i in 1..=600 {
                let now = Grenade::lamp_lit_at(from + i as f32 / 600.0, t.grenade_fuse_seconds, &t);
                n += (now != was) as u32;
                was = now;
            }
            n
        };
        assert!(flips(t.grenade_fuse_seconds - 1.0) > 3 * flips(0.0), "it blinks faster near the end");
    }

    #[test]
    fn it_is_drawn_on_the_block_grid_with_its_lamp() {
        let mut c = crate::canvas::CpuCanvas::blank(40, 40);
        let g = ball(Position::new(20.0, 20.0), Vec2::zero());
        draw_grenade(&mut c, &g);
        let drawn: Vec<(usize, usize)> =
            (0..40).flat_map(|y| (0..40).map(move |x| (x, y))).filter(|&(x, y)| c.pixel(x, y) != Color::WHITE).collect();
        assert!(!drawn.is_empty());
        assert!(drawn.iter().all(|&(x, y)| x % 2 == 0 || drawn.contains(&(x - 1, y))), "whole blocks");
        assert!(drawn.iter().all(|&(x, y)| (x as i32 - 20).abs() <= 12 && (y as i32 - 20).abs() <= 12));
    }
}
