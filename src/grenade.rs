//! Grenades (`pickup::PickupKind::Grenades`, `Tank::grenade_ammo`): the
//! launcher's drum lobs a canister up into the air along the gun line. In the
//! air it flies over everything - walls, props, tanks - stopped only by
//! the field's edge; it comes down, hops and rolls, bouncing off walls,
//! tiles and the field's edge, and off hulls, which hand it their own
//! motion, so a tank driving into one shoves it along and knocks it away.
//! It flashes faster and faster across the fuse, and after
//! `grenade_fuse_seconds` it goes off in a blast with a shockwave
//! (`Game::resolve_grenades`).
//!
//! Like every other projectile it has no physics body: `Grenade::roll` is
//! its whole motion - the arc, then on the ground drag, the hulls and a
//! sweep against the tiles and walls - over plain boxes the world half
//! (`simulation/grenades.rs`) gathers, so a grenade never moves a tank and
//! a round without one steps exactly as before. No RNG.
//!
//! The drawing is generic over `canvas::Canvas`, in the effects language
//! (docs/effects.md): whole 2 px blocks in the palette's steps - a steel
//! canister with a black cap, a red band and a brass lever, the tank
//! modules' materials - shaded from the light's side, lifted by its height
//! over a shadow that stays on the ground, tumbling in the air and rolling
//! on its side.

use crate::canvas::Canvas;
use crate::math::{Color, Vec2};
use crate::pyro::{self, BLOCK};
use crate::shell::Owner;
use crate::tuning::{tuning, Tuning};
use crate::Position;


/// One grenade on the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grenade {
    /// Per-round id from `Game::spawn_pending`'s one projectile counter,
    /// the key it travels under (`net::wire::GrenadeState`).
    pub id: u32,
    /// Whose launcher threw it: the side its blast hurts.
    pub owner: Owner,
    /// The point on the ground under it.
    pub position: Position,
    /// Velocity over the ground (px/s).
    pub velocity: Vec2,
    /// Height above the ground (px): above 0 it is in the air, passing
    /// over everything but the field's edge.
    pub height: f32,
    /// Vertical speed (px/s, up positive).
    pub climb: f32,
    /// Seconds left before the blast.
    pub fuse: f32,
    /// Radians the ball has turned since launch (distance over radius),
    /// along `heading`: where its lamp is drawn. Presentation only.
    pub roll: f32,
    /// Unit direction it last rolled in. Presentation only.
    pub heading: Vec2,
    /// The well whose ring it circles (docs/gravity-well.md): while set it
    /// does not roll - `well::orbit_at` places it - and its fuse burns on.
    pub orbit: Option<crate::well::GrenadeOrbit>,
}

/// What a grenade rolls among this tick: the boxes it bounces off.
#[derive(Default)]
pub struct Surroundings {
    /// Tiles, as (centre, half-extents): bounced off on the ground and
    /// flown over in the air.
    pub solids: Vec<(Position, Position)>,
    /// The field's four walls, as (centre, half-extents): what stops a
    /// grenade in the air too.
    pub edges: Vec<(Position, Position)>,
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
    /// `grenade_launch_speed` over the ground, plus `carry` - the launching
    /// hull's share of its own velocity - lobbed up at
    /// `grenade_launch_climb` from `grenade_launch_height`, with a full
    /// fuse.
    pub fn launch(muzzle: Position, dir: Vec2, carry: Vec2, owner: Owner) -> Self {
        let t = tuning();
        let velocity = clamp_speed(dir * t.grenade_launch_speed + carry, t.grenade_max_speed);
        Grenade {
            id: 0,
            owner,
            position: muzzle,
            velocity,
            height: t.grenade_launch_height,
            climb: t.grenade_launch_climb,
            fuse: t.grenade_fuse_seconds,
            roll: 0.0,
            heading: dir,
            orbit: None,
        }
    }

    /// Whether it is in the air: flying over everything but the field's
    /// edge.
    pub fn airborne(&self) -> bool {
        self.height > 0.0 || self.climb > 0.0
    }

    /// Where the ball is drawn: its ground point lifted by its height.
    pub fn draw_pos(&self) -> Position {
        Position::new(self.position.x, self.position.y - self.height)
    }

    /// Whether the fuse has run out: the blast is due.
    pub fn spent(&self) -> bool {
        self.fuse <= 0.0
    }

    /// One tick, `dt` seconds: the fuse burns. In the air it falls under
    /// `grenade_gravity` and flies on, swept against the field's edges
    /// alone; landing, it hops back up at `grenade_ground_bounce` of its
    /// fall while that is faster than `grenade_hop_min_speed`, and keeps
    /// `grenade_landing_keep` of its speed over the ground. On the ground
    /// the ground's drag slows it (to rest under `grenade_stop_speed`), any
    /// hull it overlaps pushes it out and hands it its own motion, then it
    /// moves with a sweep against the tiles and edges, reflecting off the
    /// face it strikes. No RNG; walks the boxes in the order given.
    pub fn roll(&mut self, dt: f32, around: &Surroundings, ground: Ground) {
        self.roll_pulled(dt, around, ground, Vec2::zero());
    }

    /// `roll` with a gravity well's `pull` (px/s², docs/gravity-well.md)
    /// on its ground motion: in the air, and on the ground past the drag's
    /// stop, so one lying still is drawn in too. A zero pull is `roll`.
    pub fn roll_pulled(&mut self, dt: f32, around: &Surroundings, ground: Ground, pull: Vec2) {
        let t = tuning();
        self.fuse -= dt;
        let r = t.grenade_radius;
        let pulled = pull.x != 0.0 || pull.y != 0.0;
        if pulled && self.airborne() {
            self.velocity = self.velocity + pull * dt;
        }
        if self.airborne() {
            self.climb -= t.grenade_gravity * dt;
            self.height += self.climb * dt;
            if self.height <= 0.0 {
                self.height = 0.0;
                let fall = -self.climb;
                self.climb = if fall > t.grenade_hop_min_speed { fall * t.grenade_ground_bounce } else { 0.0 };
                self.velocity = self.velocity * t.grenade_landing_keep;
            }
            let from = self.position;
            self.sweep(dt, &around.edges, r, t.grenade_wall_restitution);
            let moved = self.position - from;
            let distance = moved.length();
            if distance > 0.01 {
                self.heading = moved * (1.0 / distance);
                self.roll += distance / r;
            }
            return;
        }
        let factor = match ground {
            Ground::Dry => 1.0,
            Ground::Water => t.grenade_water_drag_factor,
            Ground::Ice => t.grenade_ice_drag_factor,
        };
        let keep = (1.0 - t.grenade_roll_drag * factor * dt).max(0.0);
        self.velocity = self.velocity * keep;
        if pulled {
            self.velocity = self.velocity + pull * dt;
        } else if self.velocity.length() < t.grenade_stop_speed {
            self.velocity = Vec2::zero();
        }
        for &(center, half, moving) in &around.hulls {
            self.off_hull(center, half + Position::new(r, r), moving, &t);
        }
        self.velocity = clamp_speed(self.velocity, t.grenade_max_speed);
        let from = self.position;
        let solids: Vec<(Position, Position)> = around.solids.iter().chain(&around.edges).copied().collect();
        self.sweep(dt, &solids, r, t.grenade_wall_restitution);
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

    /// Whether this grenade is flashing now.
    pub fn lamp_lit(&self) -> bool {
        let t = tuning();
        Grenade::lamp_lit_at(t.grenade_fuse_seconds - self.fuse, t.grenade_fuse_seconds, &t)
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

/// The can's steel, dark to light: the palette's stone steps, the tank
/// modules' steel.
const STEEL: [Color; 8] = [
    Color::new(0x37, 0x37, 0x37, 255),
    Color::new(0x5A, 0x5A, 0x5A, 255),
    Color::new(0x7E, 0x7E, 0x7E, 255),
    Color::new(0x8E, 0x8E, 0x8E, 255),
    Color::new(0x9E, 0x9E, 0x96, 255),
    Color::new(0xB0, 0xB0, 0xB0, 255),
    Color::new(0xC1, 0xC1, 0xC1, 255),
    Color::new(0xDA, 0xDA, 0xDA, 255),
];

/// The cap: black to the stone's dark steps.
const CAP: [Color; 4] = [
    Color::new(0x25, 0x25, 0x25, 255),
    Color::new(0x37, 0x37, 0x37, 255),
    Color::new(0x5A, 0x5A, 0x5A, 255),
    Color::new(0x7E, 0x7E, 0x7E, 255),
];

/// The band, dark: the palette's reds, the missile pod's warheads.
const BAND: [Color; 4] = [
    Color::new(0x4A, 0x22, 0x21, 255),
    Color::new(0x81, 0x2F, 0x27, 255),
    Color::new(0x9C, 0x35, 0x27, 255),
    Color::new(0xE4, 0x42, 0x19, 255),
];

/// The band on its flash: red burning up to white.
const BAND_LIT: [Color; 4] = [
    Color::new(0xE4, 0x42, 0x19, 255),
    Color::new(0xFF, 0x42, 0x1A, 255),
    Color::new(0xFF, 0xE2, 0xA0, 255),
    Color::new(0xFF, 0xFF, 0xFF, 255),
];

/// The lever's brass (WOOD_DK, WOOD_AMBER, GOLD_MD, GOLD_BRIGHT).
const BRASS: [Color; 4] = [
    Color::new(0x99, 0x65, 0x24, 255),
    Color::new(0xB5, 0x7A, 0x28, 255),
    Color::new(0xDC, 0x9C, 0x4A, 255),
    Color::new(0xEE, 0xA3, 0x43, 255),
];

/// The canister's long axis, as a unit vector: on the ground it lies across
/// the way it rolls; in the air it tumbles end over end, turning with the
/// distance it has flown.
fn axis(grenade: &Grenade) -> Vec2 {
    let across = Vec2::new(-grenade.heading.y, grenade.heading.x);
    if !grenade.airborne() {
        return across;
    }
    let turn = grenade.roll * 0.35;
    let (s, c) = turn.sin_cos();
    Vec2::new(across.x * c - across.y * s, across.x * s + across.y * c)
}

/// The blocks of a capsule `length` long and `width` wide round `center`
/// along `along` (unit), with each block's place in it: how far along the
/// axis (-1 at the base, 1 at the cap) and across it (-1..1).
fn capsule(center: Position, along: Vec2, length: f32, width: f32, mut block: impl FnMut(i32, i32, f32, f32)) {
    let half_l = length * 0.5;
    let half_w = width * 0.5;
    let reach = half_l + BLOCK;
    let (bx0, by0) = pyro::block_of(center.x - reach, center.y - reach);
    let (bx1, by1) = pyro::block_of(center.x + reach, center.y + reach);
    let across = Vec2::new(-along.y, along.x);
    for by in by0..=by1 {
        for bx in bx0..=bx1 {
            let p = Position::new((bx as f32 + 0.5) * BLOCK - center.x, (by as f32 + 0.5) * BLOCK - center.y);
            let u = p.x * along.x + p.y * along.y;
            let v = p.x * across.x + p.y * across.y;
            let core = (half_l - half_w).max(0.0);
            let du = (u.abs() - core).max(0.0);
            if du * du + v * v <= half_w * half_w {
                block(bx, by, u / half_l, v / half_w);
            }
        }
    }
}

/// The step of `ramp` at `t`, dithered across the field's Bayer pattern.
fn step(ramp: &[Color], t: f32, bx: i32, by: i32) -> Color {
    pyro::step_dithered(ramp, t, bx, by)
}

/// A grenade: a steel canister with a black cap, a red band and a brass
/// lever. Its shadow stays on the ground, smaller and fainter the higher it
/// is; the can is lifted by its height and drawn a little bigger near the
/// top, lit from the upper left like a cylinder, the lever turning round
/// it as it rolls, and the band burning white on every flash.
pub fn draw_grenade(c: &mut impl Canvas, grenade: &Grenade) {
    let t = tuning();
    let lift = (grenade.height / t.grenade_draw_lift_px.max(1.0)).clamp(0.0, 1.0);
    let scale = 1.0 + (t.grenade_apex_draw_scale - 1.0) * lift;
    let length = 2.0 * t.grenade_radius * scale;
    let width = 1.2 * t.grenade_radius * scale;
    let along = axis(grenade);
    let g = grenade.position;
    let drop = 3.0 + grenade.height * 0.5;
    let shadow = Position::new(g.x + t.shadow_dir_x * drop, g.y + t.shadow_dir_y * drop);
    let shade = Color::new(0, 0, 0, (80.0 * (1.0 - 0.5 * lift)) as u8);
    let shrink = 1.0 - 0.35 * lift;
    capsule(shadow, along, length * shrink / scale, width * shrink / scale, |bx, by, _, _| {
        c.fill_rect(bx * 2, by * 2, 2, 2, shade);
    });
    let at = grenade.draw_pos();
    // Which side of the can faces the light (from the upper left).
    let across = Vec2::new(-along.y, along.x);
    let lit_side = if across.x * 0.6 + across.y * 0.8 > 0.0 { -1.0 } else { 1.0 };
    let lit = grenade.lamp_lit();
    // The lever rides round the can as it rolls; on the far side it hides.
    let lever = grenade.roll.cos();
    capsule(at, along, length, width, |bx, by, u, v| {
        let light = (0.55 + 0.4 * v * lit_side - 0.25 * v * v).clamp(0.0, 1.0);
        let color = if u > 0.72 {
            step(&CAP, light * 0.7, bx, by)
        } else if u.abs() < 0.2 {
            if lit { step(&BAND_LIT, light + 0.2, bx, by) } else { step(&BAND, light, bx, by) }
        } else if lever > -0.2 && (v - lever * 0.6).abs() < 0.28 && u > -0.55 && u < 0.62 {
            step(&BRASS, light + 0.1, bx, by)
        } else if u < -0.86 {
            step(&STEEL[..4], light, bx, by)
        } else {
            step(&STEEL, light * 1.1, bx, by)
        };
        c.fill_rect(bx * 2, by * 2, 2, 2, color);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ball(at: Position, v: Vec2) -> Grenade {
        Grenade {
            id: 1,
            owner: Owner::Player(0),
            position: at,
            velocity: v,
            height: 0.0,
            climb: 0.0,
            fuse: 6.0,
            roll: 0.0,
            heading: Vec2::new(0.0, -1.0),
            orbit: None,
        }
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
        let around = Surroundings { solids: vec![wall], ..Default::default() };
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
        let around = Surroundings { hulls: vec![hull], ..Default::default() };
        let mut g = ball(Position::new(100.0 + 14.0 + 3.0, 100.0), Vec2::zero());
        g.roll(DT, &around, Ground::Dry);
        assert!(g.position.x >= 100.0 + 14.0 + tuning().grenade_radius, "pushed out of the hull: {:?}", g.position);
        assert!(g.velocity.x > 120.0, "knocked away faster than the hull drives: {:?}", g.velocity);
    }

    #[test]
    fn a_still_hull_is_a_wall() {
        let hull = (Position::new(100.0, 100.0), Position::new(14.0, 14.0), Vec2::zero());
        let around = Surroundings { hulls: vec![hull], ..Default::default() };
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
    fn it_is_drawn_on_the_block_grid() {
        let mut c = crate::canvas::CpuCanvas::blank(40, 40);
        let g = ball(Position::new(20.0, 20.0), Vec2::zero());
        draw_grenade(&mut c, &g);
        let drawn: Vec<(usize, usize)> =
            (0..40).flat_map(|y| (0..40).map(move |x| (x, y))).filter(|&(x, y)| c.pixel(x, y) != Color::WHITE).collect();
        assert!(!drawn.is_empty());
        assert!(drawn.iter().all(|&(x, y)| x % 2 == 0 || drawn.contains(&(x - 1, y))), "whole blocks");
        let reach = 2 * tuning().grenade_radius as i32;
        assert!(drawn.iter().all(|&(x, y)| (x as i32 - 20).abs() <= reach && (y as i32 - 20).abs() <= reach));
    }

    #[test]
    fn launched_it_flies_over_a_wall_lands_and_hops_to_the_ground() {
        let wall = (Position::new(160.0, 100.0), Position::new(16.0, 16.0));
        let edge = (Position::new(1000.0, 100.0), Position::new(16.0, 400.0));
        let around = Surroundings { solids: vec![wall], edges: vec![edge], ..Default::default() };
        let mut g = Grenade::launch(Position::new(100.0, 100.0), Vec2::new(1.0, 0.0), Vec2::zero(), Owner::Player(0));
        let mut peak: f32 = 0.0;
        let mut landings = 0;
        let mut was_up = true;
        for _ in 0..180 {
            g.roll(DT, &around, Ground::Dry);
            peak = peak.max(g.height);
            if was_up && !g.airborne() {
                landings += 1;
            }
            was_up = g.airborne();
        }
        assert!(peak > 16.0, "it went up: {peak}");
        assert!(g.position.x > 176.0 + tuning().grenade_radius, "flew over the wall: {:?}", g.position);
        assert!(!g.airborne() && landings == 1, "came down to roll, once settled: {landings}");
    }

    #[test]
    fn in_the_air_the_field_edge_still_stops_it() {
        let edge = (Position::new(200.0, 100.0), Position::new(16.0, 400.0));
        let around = Surroundings { edges: vec![edge], ..Default::default() };
        let mut g = Grenade::launch(Position::new(150.0, 100.0), Vec2::new(1.0, 0.0), Vec2::zero(), Owner::Player(0));
        for _ in 0..30 {
            g.roll(DT, &around, Ground::Dry);
            assert!(g.position.x <= 200.0 - 16.0 - tuning().grenade_radius + 0.1, "{:?}", g.position);
        }
    }
}
